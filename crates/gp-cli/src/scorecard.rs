//! `gp scorecard`: one row per game showing what capture has found so far
//! (docs/scorecard.md). Static columns (asset map, model, mechanic catalog)
//! are computed from the files; columns that need a run (fidelity, propose,
//! slot readiness) are read from the metrics database, and say "needs ROM"
//! or "no run yet" when nothing was recorded. `--record` writes every number
//! as a `scorecard.<template>.<column>` metric, and `gp metrics report`
//! draws the table with a trend per column.
//!
//! Ported from game-printer. What gameprinter's layout changed: a game is
//! also found in the games checkout (as every wave-9 command finds one), and
//! `--module` gives the game's module for the mechanics column, since
//! gameprinter's hooks live in each game's module rather than in gp. The
//! recorder is the process's (crate::runrec).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, CommandFactory};
use serde::{Deserialize, Serialize};
use serde_json::json;

use gp_assets::AssetMap;
use gp_evidence::Paths;
use gp_metrics::MetricsDb;
#[cfg(test)]
use gp_metrics::RunRecorder;
use gp_model::Model;

use crate::Outcome;

/// The prefix of the metrics `--record` writes.
pub const PREFIX: &str = "scorecard";

#[derive(Args)]
pub struct ScorecardArgs {
    /// Templates to show: a name under templates/ or a directory holding
    /// model.toml. Default: every template.
    pub templates: Vec<String>,
    /// Record every number as a `scorecard.<template>.<column>` metric, and
    /// keep the table for `gp metrics report`.
    #[arg(long)]
    pub record: bool,
    /// Recording IDs: run `gp model propose` on them for each template shown
    /// and use the result (it names the facts that differ) instead of the
    /// latest recorded one. Needs the store's recordings and, for a
    /// commercial game, its ROM; a template that can't run it keeps the
    /// recorded numbers.
    #[arg(long = "recording", num_args = 1..)]
    pub recordings: Vec<String>,
    /// The folder holding the templates.
    #[arg(long, default_value = "templates")]
    pub dir: PathBuf,
    /// The game's module (.wasm), for the mechanics column: gameprinter's
    /// hooks live in each game's module. Without it, gp's built-in hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The lifted assets the module runs with (with --module); default the
    /// model source's under $GP_HOME/work/assets.
    #[arg(long)]
    pub assets: Option<PathBuf>,
}

/// One number, recorded under `scorecard.<template>.<key>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub key: String,
    pub value: f64,
    pub unit: String,
}

/// One column of the table: text for a person, and where it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub group: String,
    pub label: String,
    pub text: String,
    /// `ok`, `fail`, `needs-rom`, `no-run`, `not-built` or `n/a`.
    pub state: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Detail {
    /// Hooks that are game-specific code with no general mechanic yet.
    pub debt: Vec<String>,
    /// Facts whose measured value differs from the model's.
    pub differing: Vec<String>,
    /// Presentation paths by kind name.
    pub kinds: BTreeMap<String, usize>,
    pub slots: Vec<String>,
    pub notes: Vec<String>,
    /// The top items `gp gaps` found, by place (docs/gaps.md).
    #[serde(default)]
    pub gaps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub template: String,
    pub title: String,
    pub tier: String,
    pub summaries: Vec<Summary>,
    pub cells: Vec<Cell>,
    pub detail: Detail,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub rows: Vec<Row>,
}

/// The latest value of every metric a column can come from.
pub struct Recorded {
    latest: BTreeMap<String, f64>,
}

impl Recorded {
    const PREFIXES: [&'static str; 9] = [
        "chunks.",
        "gaps.",
        "sound.",
        "template.",
        "propose.",
        "validate.",
        "track.",
        "derive.",
        "model.",
    ];

    pub fn load(db: &MetricsDb) -> Result<Self> {
        let mut latest = BTreeMap::new();
        for name in db.names()? {
            if Self::PREFIXES.iter().any(|p| name.starts_with(p))
                && let Some(row) = db.latest(&name)?
            {
                latest.insert(name, row.value);
            }
        }
        Ok(Self { latest })
    }

    #[cfg(test)]
    pub fn from_pairs(pairs: &[(&str, f64)]) -> Self {
        Self {
            latest: pairs.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect(),
        }
    }

    fn get(&self, name: &str) -> Option<f64> {
        self.latest.get(name).copied()
    }

    fn names(&self) -> impl Iterator<Item = &String> {
        self.latest.keys()
    }
}

struct Ctx<'a> {
    recorded: &'a Recorded,
    home: &'a Path,
    recordings: &'a [String],
    paths: Option<&'a Paths>,
    has_derive: bool,
    module: Option<&'a Path>,
    assets: Option<&'a Path>,
}

pub(crate) fn run(args: &ScorecardArgs, paths: &Paths) -> Result<Outcome> {
    let db = MetricsDb::open(&paths.metrics_db())?;
    let recorded = Recorded::load(&db)?;
    let ctx = Ctx {
        recorded: &recorded,
        home: &paths.home,
        recordings: &args.recordings,
        paths: Some(paths),
        has_derive: derive_exists(),
        module: args.module.as_deref(),
        assets: args.assets.as_deref(),
    };
    let dirs = template_dirs(&args.dir, &args.templates)?;
    let mut card = Card::default();
    for dir in &dirs {
        card.rows
            .push(row(dir, &ctx).with_context(|| format!("scorecard for {}", dir.display()))?);
    }
    if args.record {
        for r in &card.rows {
            for c in &r.cells {
                crate::runrec::metric(
                    format!("{PREFIX}.{}.{}", r.template, c.key),
                    c.value,
                    c.unit.clone(),
                );
            }
        }
        save_snapshot(paths, &card)?;
    }
    Ok(Outcome::ok(json!({ "rows": card.rows }), text(&card)))
}

/// Whether `gp model derive` is a command of this build. Looked up in the
/// command tree, so the column starts working the day the command lands.
fn derive_exists() -> bool {
    crate::Cli::command()
        .find_subcommand("model")
        .is_some_and(|m| m.find_subcommand("derive").is_some())
}

fn template_dirs(base: &Path, names: &[String]) -> Result<Vec<PathBuf>> {
    if names.is_empty() {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(base)
            .with_context(|| format!("reading {}", base.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join("model.toml").is_file())
            .collect();
        dirs.sort();
        return Ok(dirs);
    }
    names
        .iter()
        .map(|n| {
            let direct = PathBuf::from(n);
            let named = base.join(n);
            let game = crate::roms_cmd::games_dir(None).map(|g| g.join(n));
            [Some(direct), Some(named), game]
                .into_iter()
                .flatten()
                .find(|p| p.join("model.toml").is_file())
                .with_context(|| {
                    format!(
                        "no template `{n}` (looked for {n}/model.toml and {}/{n}/model.toml)",
                        base.display()
                    )
                })
        })
        .collect()
}

fn kebab(v: &impl Serialize) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The repository root above a template: the nearest ancestor with an
/// `assets/` folder.
fn repo_root(dir: &Path) -> Option<PathBuf> {
    let abs = std::fs::canonicalize(dir).ok()?;
    abs.ancestors()
        .find(|a| a.join("assets").is_dir() && a.join("templates").is_dir())
        .map(Path::to_path_buf)
}

struct Builder {
    summaries: Vec<Summary>,
    cells: Vec<Cell>,
    detail: Detail,
}

impl Builder {
    fn sum(
        &mut self,
        group: &str,
        label: &str,
        text: impl Into<String>,
        state: &str,
        source: &str,
    ) {
        self.summaries.push(Summary {
            group: group.into(),
            label: label.into(),
            text: text.into(),
            state: state.into(),
            source: source.into(),
        });
    }

    fn num(&mut self, key: impl Into<String>, value: f64, unit: &str) {
        self.cells.push(Cell {
            key: key.into(),
            value,
            unit: unit.into(),
        });
    }
}

fn row(dir: &Path, ctx: &Ctx) -> Result<Row> {
    let id = std::fs::canonicalize(dir)
        .unwrap_or_else(|_| dir.to_path_buf())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let model = crate::mechanics::load_model(dir)?;
    let manifest: Option<toml::Value> = std::fs::read_to_string(dir.join("template.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok());
    let tier = manifest
        .as_ref()
        .and_then(|m| m.get("template")?.get("tier")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "rom".into());
    let title = manifest
        .as_ref()
        .and_then(|m| m.get("template")?.get("title")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| model.model.name.clone());
    // What a missing run means: a game from a commercial ROM can't run
    // without it; an open game just hasn't been run.
    let missing = if tier == "open" {
        "no-run"
    } else {
        "needs-rom"
    };
    let mut b = Builder {
        summaries: Vec::new(),
        cells: Vec::new(),
        detail: Detail::default(),
    };
    let mut ids = vec![id.as_str()];
    if model.model.name != id {
        ids.push(model.model.name.as_str());
    }

    assets(dir, &id, &model, ctx, missing, &mut b);
    let catalog = mechanics(dir, &model, ctx, &mut b);
    facts(dir, &model, &ids, ctx, missing, &mut b);
    fidelity(&ids, ctx, missing, &mut b);
    gaps(&ids, ctx, missing, &mut b);
    chunks(&ids, ctx, missing, &mut b);
    sprites(&ids, ctx, missing, &mut b);
    sound(&ids, ctx, missing, &mut b);
    if catalog.is_none() {
        b.detail
            .notes
            .push("the mechanic catalog could not be built".into());
    }
    Ok(Row {
        template: id,
        title,
        tier,
        summaries: b.summaries,
        cells: b.cells,
        detail: b.detail,
    })
}

fn kb(n: u64) -> String {
    if n >= 10_000 {
        format!("{}k", n / 1024)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1024.0)
    } else {
        format!("{n}B")
    }
}

fn assets(dir: &Path, id: &str, model: &Model, ctx: &Ctx, missing: &str, b: &mut Builder) {
    let g = "assets";
    let source = model
        .assets
        .values()
        .find_map(|s| s.from.as_ref().map(|f| f.source.clone()));
    let root = repo_root(dir);
    // game-printer kept asset maps in the repo's assets/; gameprinter's
    // games name theirs in template.toml's recipe.
    let map_text = source
        .as_ref()
        .zip(root.as_ref())
        .and_then(|(s, r)| std::fs::read_to_string(r.join("assets").join(format!("{s}.toml"))).ok())
        .or_else(|| std::fs::read_to_string(crate::level_kit::manifest(dir).1?).ok());
    // An arcade map names graphics regions, not PRG and CHR.
    if let Some(t) = map_text
        .as_deref()
        .filter(|t| gp_assets::arcade::is_arcade(t))
    {
        match gp_assets::arcade::ArcadeMap::parse(t) {
            Ok(m) => {
                let mut regions: BTreeMap<&str, u32> = BTreeMap::new();
                for a in &m.assets {
                    *regions.entry(a.region.as_str()).or_default() += a.count;
                }
                for (r, n) in &regions {
                    b.num(format!("assets.{r}_pictures"), f64::from(*n), "count");
                }
                b.num("assets.count", m.assets.len() as f64, "count");
                let text: Vec<String> = regions.iter().map(|(r, n)| format!("{r} {n}")).collect();
                b.sum(
                    g,
                    "PRG/CHR",
                    text.join(" "),
                    "ok",
                    "arcade asset map (gp-assets)",
                );
                b.sum(
                    g,
                    "assets",
                    m.assets.len().to_string(),
                    "ok",
                    "arcade asset map (gp-assets)",
                );
            }
            Err(e) => {
                for l in ["PRG/CHR", "assets"] {
                    b.sum(
                        g,
                        l,
                        format!("bad asset map: {e}"),
                        "fail",
                        "arcade asset map (gp-assets)",
                    );
                }
            }
        }
    }
    let map = map_text
        .as_deref()
        .filter(|t| !gp_assets::arcade::is_arcade(t))
        .and_then(|t| AssetMap::parse(t).ok());
    match &map {
        Some(map) => {
            let (mut prg, mut chr) = (0u64, 0u64);
            for s in map.spans() {
                match s.region {
                    gp_assets::Region::Prg => prg += u64::from(s.len),
                    gp_assets::Region::Chr => chr += u64::from(s.len),
                }
            }
            b.num("assets.prg_bytes", prg as f64, "bytes");
            b.num("assets.chr_bytes", chr as f64, "bytes");
            b.num("assets.count", map.assets.len() as f64, "count");
            let known = root
                .as_ref()
                .and_then(|r| gp_evidence::registry::Registry::load_dir(&r.join("sources")).ok())
                .and_then(|reg| reg.sources.into_iter().find(|k| k.id == map.source));
            let part = |name: &str, n: u64, total: Option<u64>| match total {
                Some(t) if t > 0 => format!("{name} {}%", n * 100 / t),
                _ => format!("{name} {}", kb(n)),
            };
            b.sum(
                g,
                "PRG/CHR",
                format!(
                    "{} {}",
                    part("PRG", prg, known.as_ref().and_then(|k| k.prg_rom_bytes)),
                    part("CHR", chr, known.as_ref().and_then(|k| k.chr_rom_bytes))
                ),
                "ok",
                "asset map (gp-assets)",
            );
            b.sum(
                g,
                "assets",
                map.assets.len().to_string(),
                "ok",
                "asset map (gp-assets)",
            );
        }
        None if map_text
            .as_deref()
            .is_some_and(gp_assets::arcade::is_arcade) => {}
        None => {
            for l in ["PRG/CHR", "assets"] {
                b.sum(g, l, "no asset map", "n/a", "asset map (gp-assets)");
            }
        }
    }

    // Slots by kind, and how many an asset map entry fills.
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut filled = 0usize;
    for (name, slot) in &model.assets {
        let kind = kebab(&slot.kind);
        *by_kind.entry(kind.clone()).or_default() += 1;
        let ok = slot
            .from
            .as_ref()
            .is_some_and(|f| map.as_ref().is_some_and(|m| m.asset(&f.asset).is_some()));
        filled += usize::from(ok);
        b.detail.slots.push(format!(
            "{name}: {kind}, {}",
            if ok { "filled" } else { "empty" }
        ));
    }
    b.num("assets.slots", model.assets.len() as f64, "count");
    b.num("assets.slots_filled", filled as f64, "count");
    for (k, n) in &by_kind {
        b.num(format!("assets.slots.{k}"), *n as f64, "count");
    }
    let ready = ctx.recorded.get(&format!("template.{id}.slots_ready"));
    if let Some(r) = ready {
        b.num("assets.slots_ready", r, "count");
    }
    b.sum(
        g,
        "slots",
        format!(
            "{filled}/{}, {}",
            model.assets.len(),
            ready.map_or_else(
                || if missing == "no-run" {
                    "none ready yet".into()
                } else {
                    "needs ROM".into()
                },
                |r| format!("{r:.0} ready")
            )
        ),
        if ready.is_some() { "ok" } else { missing },
        "model slots; gp template check (template.<id>.slots_ready)",
    );

    // Stub, original or replaced, from slots.toml where one was written.
    let slots_file = [
        ctx.home.join("work/assets").join(id).join("slots.toml"),
        ctx.home.join("work/templates").join(id).join("slots.toml"),
    ]
    .into_iter()
    .find(|p| p.is_file());
    let state: Option<BTreeMap<String, usize>> = slots_file
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|v| {
            let slots = v.get("slots")?.as_table()?.clone();
            let mut c: BTreeMap<String, usize> = BTreeMap::new();
            for s in slots.values() {
                *c.entry(s.get("status")?.as_str()?.to_owned()).or_default() += 1;
            }
            Some(c)
        });
    match state {
        Some(c) => {
            for (k, n) in &c {
                b.num(format!("assets.art.{k}"), *n as f64, "count");
            }
            let t: Vec<String> = c.iter().map(|(k, n)| format!("{n} {k}")).collect();
            b.sum(g, "art", t.join(", "), "ok", "slots.toml (docs/stubs.md)");
        }
        None => b.sum(
            g,
            "art",
            "no slots.toml",
            "no-run",
            "slots.toml (docs/stubs.md)",
        ),
    }
}

/// The names of the presentation kinds a model path belongs to: the part
/// after the owner (`entities.player.sprite.tiles` -> `sprite`), or for an
/// asset slot its kind. New kinds the catalog classifies as presentation
/// show up here without a change.
fn presentation_kind(path: &str, model: &Model) -> String {
    let segs: Vec<String> = path
        .split('.')
        .map(|s| s.split('[').next().unwrap_or(s).to_owned())
        .collect();
    match segs.first().map(String::as_str) {
        Some("assets") => segs
            .get(1)
            .and_then(|n| model.assets.get(n))
            .map(|s| kebab(&s.kind))
            .unwrap_or_else(|| "assets".into()),
        Some("entities" | "levels" | "cameras" | "rules" | "counters") if segs.len() > 3 => {
            // `entities.x.present.<kind>.*` names its kind one level down.
            if segs[2] == "present" && segs.len() > 4 {
                segs[3].clone()
            } else {
                segs[2].clone()
            }
        }
        _ => segs.get(1).or(segs.first()).cloned().unwrap_or_default(),
    }
}

fn mechanics(dir: &Path, model: &Model, ctx: &Ctx, b: &mut Builder) -> Option<()> {
    let exts = match (ctx.module, ctx.paths) {
        (Some(m), Some(paths)) => {
            match crate::hosted::Source::open_dir(dir, ctx.assets, paths, Some(m)) {
                Ok((s, _)) => s.exts,
                Err(e) => {
                    b.detail
                        .notes
                        .push(format!("the module could not be opened: {e:#}"));
                    return None;
                }
            }
        }
        _ => std::sync::Arc::new(gp_exts::registry(&model.model.name).0),
    };
    let cat = gp_interp::mechanics::catalog(model, &exts);
    let hooks: BTreeSet<&str> = cat
        .mechanics
        .iter()
        .filter_map(|m| m.hook.as_deref())
        .collect();
    let components = cat.mechanics.iter().filter(|m| m.hook.is_none()).count();
    let nhooks = hooks.len() + cat.opaque.len();
    let src = "gp model mechanics (gp_interp::mechanics, Ext::describe)";
    b.num("mechanics.used", cat.mechanics.len() as f64, "count");
    b.num("mechanics.components", components as f64, "count");
    b.num("mechanics.hooks", nhooks as f64, "count");
    b.num("mechanics.debt", cat.debt.len() as f64, "count");
    b.num("mechanics.problems", cat.problems.len() as f64, "count");
    for (class, n) in &cat.counts {
        b.num(format!("mechanics.paths.{class}"), *n as f64, "paths");
    }
    b.sum(
        "mechanics",
        "mechanics",
        format!("{} ({components} components)", cat.mechanics.len()),
        "ok",
        src,
    );
    b.sum("mechanics", "hooks", nhooks.to_string(), "ok", src);
    b.sum(
        "mechanics",
        "debt",
        cat.debt.len().to_string(),
        if cat.debt.is_empty() { "ok" } else { "warn" },
        src,
    );
    b.sum(
        "mechanics",
        "paths m/p/c/pr",
        ["mechanic", "parameter", "content", "presentation"]
            .iter()
            .map(|c| cat.counts.get(c).copied().unwrap_or(0).to_string())
            .collect::<Vec<_>>()
            .join("/"),
        "ok",
        src,
    );
    for d in &cat.debt {
        b.detail
            .debt
            .push(format!("{} ({}): {}", d.hook, d.record, d.reason));
    }
    // Presentation kinds, by kind name.
    for p in &cat.paths {
        if p.class.name() == "presentation" {
            *b.detail
                .kinds
                .entry(presentation_kind(&p.path, model))
                .or_default() += 1;
        }
    }
    let kinds = b.detail.kinds.clone();
    b.num("present.kinds", kinds.len() as f64, "count");
    for (k, n) in &kinds {
        b.num(format!("present.kind.{k}"), *n as f64, "paths");
    }
    b.sum(
        "presentation",
        "present kinds",
        if kinds.is_empty() {
            "none".to_owned()
        } else {
            kinds
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        },
        "ok",
        "model paths the catalog classes as presentation",
    );
    Some(())
}

fn facts(dir: &Path, model: &Model, ids: &[&str], ctx: &Ctx, missing: &str, b: &mut Builder) {
    let g = "facts";
    let by = gp_model::validate::evidence_for(model);
    let mut by_source: BTreeMap<String, usize> = BTreeMap::new();
    for (_, key) in &by {
        let source = key
            .as_ref()
            .and_then(|k| model.evidence.get(k))
            .map_or("none", |e| e.source.name());
        *by_source.entry(source.to_owned()).or_default() += 1;
    }
    let none = by_source.get("none").copied().unwrap_or(0);
    b.num("facts.total", by.len() as f64, "facts");
    b.num("facts.with_evidence", (by.len() - none) as f64, "facts");
    for (s, n) in &by_source {
        b.num(format!("facts.source.{s}"), *n as f64, "facts");
    }
    let src = "model.toml and its evidence table (gp model check)";
    b.sum(g, "facts", by.len().to_string(), "ok", src);
    let mix: Vec<String> = by_source
        .iter()
        .filter(|(s, _)| s.as_str() != "none")
        .map(|(s, n)| format!("{n} {s}"))
        .collect();
    b.sum(
        g,
        "by source",
        format!(
            "{}{}",
            if mix.is_empty() {
                "-".to_owned()
            } else {
                mix.join(", ")
            },
            if none > 0 {
                format!(", {none} without evidence")
            } else {
                String::new()
            }
        ),
        if none > 0 { "warn" } else { "ok" },
        src,
    );

    // Measured by gp model propose: a run now, or the latest recorded one.
    let ran = if ctx.recordings.is_empty() {
        None
    } else {
        ctx.paths
            .and_then(|p| run_propose(dir, ctx.recordings, p, b))
    };
    let counts = ran.or_else(|| {
        ids.iter().find_map(|i| {
            Some((
                ctx.recorded.get(&format!("propose.{i}.compared"))?,
                ctx.recorded.get(&format!("propose.{i}.agree"))?,
            ))
        })
    });
    let psrc = "gp model propose (propose.<template>.*)";
    match counts {
        Some((compared, agree)) => {
            b.num("facts.compared", compared, "count");
            b.num("facts.agree", agree, "count");
            b.num("facts.differ", compared - agree, "count");
            b.sum(
                g,
                "measured",
                format!(
                    "{agree:.0} agree, {:.0} differ of {compared:.0}",
                    compared - agree
                ),
                if compared > agree { "warn" } else { "ok" },
                psrc,
            );
        }
        None => b.sum(g, "measured", state_text(missing), missing, psrc),
    }

    // Derived by gp model derive, once that command is on this build.
    if ctx.has_derive {
        let get = |m: &str| {
            ids.iter()
                .find_map(|i| ctx.recorded.get(&format!("derive.{i}.{m}")))
        };
        match (get("derivable_share"), get("agree_share")) {
            (Some(share), agree) => {
                b.num("facts.derivable_share", share, "share");
                let (text, state) = match agree {
                    Some(a) => {
                        b.num("facts.derived_agree_share", a, "share");
                        let state = if a < 1.0 { "warn" } else { "ok" };
                        (
                            format!("{:.0}% derivable, {:.1}% agree", share * 100.0, a * 100.0),
                            state,
                        )
                    }
                    None => (format!("{:.0}% derivable", share * 100.0), "ok"),
                };
                b.sum(
                    g,
                    "derived",
                    text,
                    state,
                    "gp model derive (derive.<template>.derivable_share, .agree_share)",
                );
            }
            (None, _) => b.sum(
                g,
                "derived",
                state_text("no-run"),
                "no-run",
                "gp model derive (derive.<template>.derivable_share, .agree_share)",
            ),
        }
    } else {
        b.sum(g, "derived", "not built", "not-built", "gp model derive");
    }
}

/// Runs `gp model propose` in process, returning (compared, agree) and naming
/// the facts that differ. gameprinter's propose records no metrics, so
/// nothing of its lands in this run's.
fn run_propose(
    dir: &Path,
    recordings: &[String],
    paths: &Paths,
    b: &mut Builder,
) -> Option<(f64, f64)> {
    match crate::refine::recording::propose(dir, recordings, "1..", false, paths) {
        Ok(out) => {
            for p in out.json["proposals"].as_array().into_iter().flatten() {
                if p["agrees"] == json!(false) {
                    b.detail.differing.push(format!(
                        "{}: measured {} {}, model has {}",
                        p["path"].as_str().unwrap_or("?"),
                        p["measured"].as_str().unwrap_or("?"),
                        p["unit"].as_str().unwrap_or(""),
                        p["stated"].as_str().unwrap_or("?")
                    ));
                }
            }
            let a = &out.json["accuracy"];
            Some((a["compared"].as_f64()?, a["agree"].as_f64()?))
        }
        Err(e) => {
            b.detail.notes.push(format!("propose could not run: {e:#}"));
            None
        }
    }
}

fn state_text(state: &str) -> &'static str {
    match state {
        "needs-rom" => "needs ROM",
        "no-run" => "no run yet",
        "not-built" => "not built",
        _ => "n/a",
    }
}

/// Validate and tracking results from recorded metrics.
fn fidelity(ids: &[&str], ctx: &Ctx, missing: &str, b: &mut Builder) {
    let g = "fidelity";
    let r = ctx.recorded;
    // validate.<id>.checks.passed is the interpreter; validate.<id>.<rt>.checks.passed another runtime.
    let mut runtimes: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    for id in ids {
        let head = format!("validate.{id}.");
        for name in r.names() {
            let Some(rest) = name.strip_prefix(&head) else {
                continue;
            };
            let Some(mid) = rest.strip_suffix("checks.passed") else {
                continue;
            };
            let rt = mid.trim_end_matches('.');
            let base = format!("{head}{mid}");
            let get = |k: &str| r.get(&format!("{base}checks.{k}")).unwrap_or(0.0);
            runtimes.insert(
                if rt.is_empty() {
                    "interp".into()
                } else {
                    rt.to_owned()
                },
                (get("passed"), get("failed"), get("unexercised")),
            );
        }
    }
    let src = "gp validate (validate.<template>[.<runtime>].checks.*)";
    if runtimes.is_empty() {
        b.sum(g, "validate", state_text(missing), missing, src);
    } else {
        let mut parts = Vec::new();
        let mut state = "ok";
        for (rt, (p, f, u)) in &runtimes {
            b.num(format!("fidelity.{rt}.passed"), *p, "count");
            b.num(format!("fidelity.{rt}.failed"), *f, "count");
            b.num(format!("fidelity.{rt}.unexercised"), *u, "count");
            let verdict = if *f > 0.0 {
                state = "fail";
                "fail"
            } else if *p > 0.0 {
                "pass"
            } else {
                "unexercised"
            };
            parts.push(format!("{rt} {verdict} {p:.0}/{f:.0}/{u:.0}"));
        }
        b.sum(g, "validate", parts.join(", "), state, src);
    }

    // track.<id>.<recording>.{precision,recall}: the mean over recordings.
    let tsrc = "gp track (track.<template>.<recording>.precision/recall)";
    let mean = |what: &str| {
        let vals: Vec<f64> = ids
            .iter()
            .flat_map(|id| {
                let head = format!("track.{id}.");
                r.names()
                    .filter(|n| n.starts_with(&head) && n.ends_with(&format!(".{what}")))
                    .filter_map(|n| r.get(n))
                    .collect::<Vec<_>>()
            })
            .collect();
        (!vals.is_empty()).then(|| vals.iter().sum::<f64>() / vals.len() as f64)
    };
    match (mean("precision"), mean("recall")) {
        (Some(p), Some(rc)) => {
            b.num("fidelity.track.precision", p, "ratio");
            b.num("fidelity.track.recall", rc, "ratio");
            b.sum(g, "track P/R", format!("{p:.2}/{rc:.2}"), "ok", tsrc);
        }
        _ => b.sum(g, "track P/R", state_text(missing), missing, tsrc),
    }
}

/// What `gp gaps` found: the counts from its metrics
/// (`gaps.<template>.<place>.count`), and the top items from the reports
/// it writes under `$GP_HOME/reports/gaps/<template>/`.
fn gaps(ids: &[&str], ctx: &Ctx, missing: &str, b: &mut Builder) {
    let g = "gaps";
    let r = ctx.recorded;
    let Some(id) = ids
        .iter()
        .find(|id| r.names().any(|n| n.starts_with(&format!("gaps.{id}."))))
    else {
        for label in ["changes", "interactive", "drawn", "detection quality"] {
            b.sum(
                g,
                label,
                state_text(missing),
                missing,
                "gp gaps (gaps.<template>.<place>.count)",
            );
        }
        return;
    };
    let get = |place: &str| r.get(&format!("gaps.{id}.{place}.count"));
    let places = ["ram", "oam", "background", "sound", "code"];
    let mut parts = Vec::new();
    for p in places {
        if let Some(v) = get(p) {
            b.num(format!("gaps.{p}.count"), v, "count");
            parts.push(format!("{p} {v:.0}"));
        }
    }
    let src = "gp gaps changes (gaps.<template>.<place>.count)";
    if parts.is_empty() {
        b.sum(g, "changes", state_text(missing), missing, src);
    } else {
        b.sum(g, "changes", parts.join(", "), "ok", src);
    }
    let isrc = "gp gaps interactive (gaps.<template>.interactive.count)";
    match get("interactive") {
        Some(v) => {
            b.num("gaps.interactive.count", v, "count");
            b.sum(g, "interactive", format!("{v:.0} kinds"), "ok", isrc);
        }
        None => b.sum(g, "interactive", state_text(missing), missing, isrc),
    }
    let dsrc = "gp gaps drawn (gaps.<template>.drawn.count)";
    match get("drawn") {
        Some(v) => {
            b.num("gaps.drawn.count", v, "count");
            let kinds = r.get(&format!("gaps.{id}.drawn.kinds")).unwrap_or(0.0);
            b.sum(
                g,
                "drawn",
                format!("{v:.0} of {kinds:.0} kinds off"),
                if v > 0.0 { "warn" } else { "ok" },
                dsrc,
            );
        }
        None => b.sum(g, "drawn", state_text(missing), missing, dsrc),
    }
    // Detection quality: each detector's precision and recall against the
    // game's labelled reference set (gaps.<template>.<detector>.precision
    // and .recall).
    let qsrc =
        "gp gaps all against crates/gp-gaps/truth (gaps.<template>.<detector>.precision, .recall)";
    let mut q = Vec::new();
    for d in ["sound", "effects", "interactive", "handlers", "ram"] {
        let p = r.get(&format!("gaps.{id}.{d}.precision"));
        let rc = r.get(&format!("gaps.{id}.{d}.recall"));
        if let (Some(p), Some(rc)) = (p, rc) {
            b.num(format!("gaps.{d}.precision"), p, "ratio");
            b.num(format!("gaps.{d}.recall"), rc, "ratio");
            q.push(format!("{d} {p:.2}/{rc:.2}"));
        }
    }
    if q.is_empty() {
        b.sum(g, "detection quality", "no reference set", "n/a", qsrc);
    } else {
        if let Some(n) = r.get(&format!("gaps.{id}.effects.named")) {
            q.push(format!("named {n:.2}"));
        }
        b.sum(g, "detection quality", q.join(", "), "ok", qsrc);
    }
    b.detail.gaps = gap_items(&ctx.home.join("reports/gaps").join(id), 5);
}

/// What `gp chunks` found (docs/chunks.md): the chunk kinds by layer and
/// the mean share of each level's structure inside a known chunk, from its
/// metrics (`chunks.<template>.*`).
fn chunks(ids: &[&str], ctx: &Ctx, missing: &str, b: &mut Builder) {
    let r = ctx.recorded;
    let src = "gp chunks (chunks.<template>.kinds, .coverage)";
    let Some(id) = ids
        .iter()
        .find(|id| r.get(&format!("chunks.{id}.kinds")).is_some())
    else {
        b.sum("chunks", "chunks", state_text(missing), missing, src);
        return;
    };
    let get = |k: &str| r.get(&format!("chunks.{id}.{k}")).unwrap_or(0.0);
    for k in [
        "kinds",
        "objects",
        "groups",
        "gaps",
        "patterns",
        "compounds",
    ] {
        b.num(format!("chunks.{k}"), get(k), "count");
    }
    for k in [
        "coverage",
        "coverage.objects",
        "coverage.groups",
        "coverage.patterns",
    ] {
        b.num(format!("chunks.{k}"), get(k), "ratio");
    }
    b.sum(
        "chunks",
        "chunks",
        format!(
            "{:.0} kinds (objects {:.0}, groups {:.0}, gaps {:.0}, patterns {:.0}, compounds {:.0}); coverage {:.0}%",
            get("kinds"),
            get("objects"),
            get("groups"),
            get("gaps"),
            get("patterns"),
            get("compounds"),
            get("coverage") * 100.0
        ),
        "ok",
        src,
    );
}

/// Sprite capture's coverage from the latest `gp sprites capture`
/// (sprites.<template>.frames, .states_covered, .states, .unobserved).
fn sprites(ids: &[&str], ctx: &Ctx, missing: &str, b: &mut Builder) {
    let src = "gp sprites capture (sprites.<template>.frames, .states_covered, .unobserved)";
    let r = ctx.recorded;
    let Some(id) = ids
        .iter()
        .find(|id| r.get(&format!("sprites.{id}.frames")).is_some())
    else {
        b.sum("present", "sprites", state_text(missing), missing, src);
        return;
    };
    let get = |k: &str| r.get(&format!("sprites.{id}.{k}")).unwrap_or(0.0);
    let (frames, covered, states, unobserved) = (
        get("frames"),
        get("states_covered"),
        get("states"),
        get("unobserved"),
    );
    b.num("sprites.frames", frames, "count");
    b.num("sprites.states_covered", covered, "count");
    b.num("sprites.states", states, "count");
    b.num("sprites.unobserved", unobserved, "count");
    b.sum(
        "present",
        "sprites",
        format!("{frames:.0} frames, states {covered:.0}/{states:.0}, {unobserved:.0} unseen"),
        if covered < states { "partial" } else { "ok" },
        src,
    );
}

/// What `gp sound capture` found (docs/sound.md): songs and effects
/// captured over every capture of the template, and how well the notes
/// played back agree with the register logs they came from (metrics
/// `sound.<template>.<capture>.songs`, `.sfx`, `.f1`, `.sfx_f1`).
fn sound(ids: &[&str], ctx: &Ctx, missing: &str, b: &mut Builder) {
    let g = "sound";
    let r = ctx.recorded;
    let src = "gp sound capture (sound.<template>.<capture>.*)";
    let Some(id) = ids
        .iter()
        .find(|id| r.names().any(|n| n.starts_with(&format!("sound.{id}."))))
    else {
        b.sum(g, "captured", state_text(missing), missing, src);
        b.sum(g, "agreement", state_text(missing), missing, src);
        return;
    };
    let prefix = format!("sound.{id}.");
    let mut songs = 0.0;
    let mut sfx = 0.0;
    let mut f1 = Vec::new();
    let mut sfx_f1 = Vec::new();
    for n in r
        .names()
        .filter(|n| n.starts_with(&prefix) && !n.contains(".decode."))
    {
        let v = r.get(n).unwrap_or(0.0);
        if n.ends_with(".songs") {
            songs += v;
        } else if n.ends_with(".sfx") {
            sfx += v;
        } else if n.ends_with(".sfx_f1") {
            sfx_f1.push(v);
        } else if n.ends_with(".f1") {
            f1.push(v);
        }
    }
    b.num("sound.songs", songs, "count");
    b.num("sound.sfx", sfx, "count");
    b.sum(
        g,
        "captured",
        format!("{songs:.0} songs, {sfx:.0} effects"),
        "ok",
        src,
    );
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    if f1.is_empty() {
        b.sum(g, "agreement", state_text(missing), missing, src);
    } else {
        let m = mean(&f1);
        b.num("sound.f1", m, "ratio");
        let mut t = format!("notes F1 {m:.3}");
        if !sfx_f1.is_empty() {
            let e = mean(&sfx_f1);
            b.num("sound.sfx_f1", e, "ratio");
            t.push_str(&format!(", effects {e:.3}"));
        }
        b.sum(
            g,
            "agreement",
            t,
            if m >= 0.95 { "ok" } else { "warn" },
            src,
        );
    }
}

/// The top `n` items of each place in the reports `gp gaps` wrote.
fn gap_items(dir: &Path, n: usize) -> Vec<String> {
    let read = |name: &str| -> Option<serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{name}.json"))).ok()?).ok()
    };
    let mut out = Vec::new();
    let s = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .map(|x| x.to_string().trim_matches('"').to_owned())
            .unwrap_or_default()
    };
    if let Some(c) = read("changes") {
        for (place, key, what) in [
            ("ram", "name", "play_changes"),
            ("oam", "name", "tracks"),
            ("background", "before", "count"),
            ("sound", "name", "count"),
            ("code", "name", "frames"),
        ] {
            for i in c
                .get(place)
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .take(n)
            {
                let extra = if place == "background" {
                    format!(" -> {}", s(i, "after"))
                } else if place == "sound" {
                    format!(" (suggests {})", s(i, "suggests"))
                } else {
                    String::new()
                };
                out.push(format!(
                    "{place}: {}{extra}, {} {}",
                    s(i, key),
                    s(i, what),
                    what.replace('_', " ")
                ));
            }
        }
    }
    if let Some(c) = read("interactive") {
        for k in c
            .get("kinds")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .take(n)
        {
            let t = k.get("trigger").cloned().unwrap_or_default();
            out.push(format!(
                "interactive: {} x{}, trigger {} by {}",
                s(k, "kind"),
                s(k, "count"),
                s(&t, "relation"),
                s(&t, "routine")
            ));
        }
    }
    if let Some(c) = read("drawn") {
        for k in c
            .get("kinds")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .take(n)
        {
            let sys = k
                .get("systematic")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if sys.is_empty() {
                continue;
            }
            let sys: Vec<String> = sys
                .iter()
                .map(|x| x.as_str().unwrap_or("").to_owned())
                .collect();
            out.push(format!("drawn: {} {}", s(k, "kind"), sys.join(" ")));
        }
    }
    out
}

/// The table as text: a row per game.
fn text(card: &Card) -> String {
    let Some(first) = card.rows.first() else {
        return "no templates".into();
    };
    // Every row has the same columns in the same order.
    let heads: Vec<&str> = first.summaries.iter().map(|s| s.label.as_str()).collect();
    let mut widths: Vec<usize> = heads.iter().map(|h| h.len()).collect();
    let name_w = card
        .rows
        .iter()
        .map(|r| r.template.len())
        .max()
        .unwrap_or(4)
        .max(4);
    for r in &card.rows {
        for (i, s) in r.summaries.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(s.text.len());
            }
        }
    }
    let mut t = String::new();
    let _ = write!(t, "{:<name_w$}", "game");
    for (h, w) in heads.iter().zip(&widths) {
        let _ = write!(t, "  {h:<w$}");
    }
    t.push('\n');
    for r in &card.rows {
        let _ = write!(t, "{:<name_w$}", r.template);
        for (s, w) in r.summaries.iter().zip(&widths) {
            let _ = write!(t, "  {:<w$}", s.text);
        }
        t.push('\n');
    }
    for r in &card.rows {
        for d in &r.detail.debt {
            let _ = writeln!(t, "{}: debt: {d}", r.template);
        }
        for d in &r.detail.differing {
            let _ = writeln!(t, "{}: differs: {d}", r.template);
        }
        for d in &r.detail.notes {
            let _ = writeln!(t, "{}: note: {d}", r.template);
        }
    }
    t.lines().map(str::trim_end).collect::<Vec<_>>().join("\n")
}

const SNAPSHOT: &str = "scorecard.json";

fn save_snapshot(paths: &Paths, card: &Card) -> Result<()> {
    let dir = paths.reports_dir();
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(SNAPSHOT);
    std::fs::write(&file, serde_json::to_string_pretty(card)?)
        .with_context(|| format!("writing {}", file.display()))
}

/// The Scorecard section of `gp metrics report`: computed from the templates
/// in the current directory when there are any, else the table `--record`
/// last saved.
pub(crate) fn report_section(db: &MetricsDb, paths: &Paths) -> String {
    let card = Recorded::load(db)
        .ok()
        .and_then(|recorded| {
            let ctx = Ctx {
                recorded: &recorded,
                home: &paths.home,
                recordings: &[],
                paths: None,
                has_derive: derive_exists(),
                module: None,
                assets: None,
            };
            let dirs = template_dirs(Path::new("templates"), &[]).ok()?;
            let rows: Result<Vec<Row>> = dirs.iter().map(|d| row(d, &ctx)).collect();
            rows.ok().map(|rows| Card { rows })
        })
        .or_else(|| {
            let t = std::fs::read_to_string(paths.reports_dir().join(SNAPSHOT)).ok()?;
            serde_json::from_str(&t).ok()
        });
    match card {
        Some(card) => html(db, &card),
        None => "<h2>Scorecard</h2><p>No scorecard yet. Run <code>just scorecard</code> from the repository.</p>".into(),
    }
}

use gp_metrics::report::{esc, sparkline};

/// The section's HTML: the table, then a drill-down per game with a trend
/// for every recorded number.
pub fn html(db: &MetricsDb, card: &Card) -> String {
    let mut h = String::from(
        "<h2>Scorecard</h2>\n<style>\n\
         .sc td.ok{} .sc td.warn{color:var(--fail)} .sc td.fail{color:var(--fail);font-weight:600}\n\
         .sc td.needs-rom,.sc td.no-run,.sc td.not-built,.sc td.n\\/a{color:var(--muted)}\n\
         .sc details{margin:8px 0} .sc summary{cursor:pointer;font-weight:600}\n\
         .sc ul{margin:4px 0 8px 18px;padding:0}\n\
         </style>\n<p>One row per game, from the files and the recorded runs. Grey cells have no run behind them yet (docs/scorecard.md).</p>\n",
    );
    let Some(first) = card.rows.first() else {
        h.push_str("<p>No templates.</p>");
        return h;
    };
    h.push_str("<div class=\"wrap sc\"><table><thead><tr><th>game</th>");
    for s in &first.summaries {
        let _ = write!(h, "<th title=\"{}\">{}</th>", esc(&s.group), esc(&s.label));
    }
    h.push_str("</tr></thead><tbody>");
    for r in &card.rows {
        let _ = write!(h, "<tr><td><code>{}</code></td>", esc(&r.template));
        for s in &r.summaries {
            let _ = write!(
                h,
                "<td class=\"{}\" title=\"{}\">{}</td>",
                esc(&s.state),
                esc(&s.source),
                esc(&s.text)
            );
        }
        h.push_str("</tr>");
    }
    h.push_str("</tbody></table></div>\n<div class=\"sc\">");
    for r in &card.rows {
        let _ = write!(
            h,
            "<details><summary>{} ({})</summary>",
            esc(&r.title),
            esc(&r.template)
        );
        h.push_str("<div class=\"wrap\"><table><thead><tr><th>Column</th><th>Value</th><th>Trend</th><th>Source</th></tr></thead><tbody>");
        for s in &r.summaries {
            let _ = write!(
                h,
                "<tr><td>{}: {}</td><td class=\"{}\">{}</td><td></td><td>{}</td></tr>",
                esc(&s.group),
                esc(&s.label),
                esc(&s.state),
                esc(&s.text),
                esc(&s.source)
            );
        }
        for c in &r.cells {
            let name = format!("{PREFIX}.{}.{}", r.template, c.key);
            let trend = db
                .history(&name, 30)
                .map(|hist| sparkline(&hist))
                .unwrap_or_default();
            let _ = write!(
                h,
                "<tr><td><code>{}</code></td><td class=\"num\">{} {}</td><td>{}</td><td>recorded by <code>gp scorecard --record</code></td></tr>",
                esc(&c.key),
                gp_metrics::report::fmt_value(c.value),
                esc(&c.unit),
                trend
            );
        }
        h.push_str("</tbody></table></div>");
        let list = |h: &mut String, title: &str, items: &[String]| {
            if items.is_empty() {
                return;
            }
            let _ = write!(h, "<p>{}</p><ul>", esc(title));
            for i in items {
                let _ = write!(h, "<li>{}</li>", esc(i));
            }
            h.push_str("</ul>");
        };
        list(
            &mut h,
            "Hooks that are still game-specific code (debt):",
            &r.detail.debt,
        );
        list(
            &mut h,
            "Facts whose measured value differs from the model's:",
            &r.detail.differing,
        );
        list(&mut h, "Slots:", &r.detail.slots);
        list(&mut h, "Notes:", &r.detail.notes);
        list(
            &mut h,
            "Gaps: what recordings show that the model doesn't account for (gp gaps, docs/gaps.md):",
            &r.detail.gaps,
        );
        let kinds: Vec<String> = r
            .detail
            .kinds
            .iter()
            .map(|(k, n)| format!("{k}: {n} paths"))
            .collect();
        list(&mut h, "Presentation kinds in the model:", &kinds);
        h.push_str("</details>");
    }
    h.push_str("</div>\n");
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_metrics::run::{RunStatus, Tool};

    fn mosshop() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop")
    }

    fn ctx<'a>(rec: &'a Recorded, home: &'a Path) -> Ctx<'a> {
        Ctx {
            recorded: rec,
            home,
            recordings: &[],
            paths: None,
            has_derive: false,
            module: None,
            assets: None,
        }
    }

    fn summary<'a>(r: &'a Row, label: &str) -> &'a Summary {
        r.summaries.iter().find(|s| s.label == label).unwrap()
    }

    #[test]
    fn static_columns_for_moss_hop() {
        let home = tempfile::tempdir().unwrap();
        let rec = Recorded::from_pairs(&[]);
        let r = row(&mosshop(), &ctx(&rec, home.path())).unwrap();
        assert_eq!(r.template, "mosshop");
        let cell = |k: &str| r.cells.iter().find(|c| c.key == k).map(|c| c.value);
        assert_eq!(
            cell("assets.count"),
            Some(6.0),
            "five lifted and the sprite-set capture writes"
        );
        assert_eq!(cell("assets.slots"), Some(6.0), "the five and the font");
        assert_eq!(cell("assets.slots_filled"), Some(6.0));
        assert_eq!(
            cell("assets.prg_bytes").unwrap() + cell("assets.chr_bytes").unwrap(),
            8766.0
        );
        assert!(cell("mechanics.used").unwrap() >= 9.0);
        assert_eq!(cell("mechanics.debt"), Some(0.0));
        assert!(cell("facts.total").unwrap() > 50.0);
        assert!(r.detail.kinds.contains_key("sprite"));
        assert!(r.detail.kinds.contains_key("palettes"));
    }

    #[test]
    fn unrecorded_columns_are_honest() {
        let home = tempfile::tempdir().unwrap();
        let rec = Recorded::from_pairs(&[]);
        let r = row(&mosshop(), &ctx(&rec, home.path())).unwrap();
        let v = summary(&r, "validate");
        assert_eq!(
            (v.text.as_str(), v.state.as_str()),
            ("no run yet", "no-run")
        );
        assert_eq!(summary(&r, "derived").text, "not built");
        assert_eq!(summary(&r, "measured").text, "no run yet");
        assert_eq!(summary(&r, "art").text, "no slots.toml");
    }

    #[test]
    fn sound_metrics_fill_the_sound_columns() {
        let home = tempfile::tempdir().unwrap();
        let rec = Recorded::from_pairs(&[
            ("sound.mosshop.one.songs", 2.0),
            ("sound.mosshop.one.sfx", 3.0),
            ("sound.mosshop.one.f1", 1.0),
            ("sound.mosshop.one.sfx_f1", 0.9),
            ("sound.mosshop.decode.theme.f1", 0.2),
        ]);
        let r = row(&mosshop(), &ctx(&rec, home.path())).unwrap();
        assert_eq!(summary(&r, "captured").text, "2 songs, 3 effects");
        assert_eq!(
            summary(&r, "agreement").text,
            "notes F1 1.000, effects 0.900"
        );
        let r = row(&mosshop(), &ctx(&Recorded::from_pairs(&[]), home.path())).unwrap();
        assert_eq!(summary(&r, "captured").state, "no-run");
    }

    #[test]
    fn gaps_metrics_and_reports_fill_the_gaps_columns() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("reports/gaps/mosshop");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("changes.json"),
            r#"{"ram":[{"name":"Timer","play_changes":40}],"sound":[{"name":"Sfx_Ping","count":4,"suggests":"coins +1 (100%)"}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("drawn.json"),
            r#"{"kinds":[{"kind":"walker","systematic":["dy -9"]},{"kind":"hero","systematic":[]}]}"#,
        )
        .unwrap();
        let rec = Recorded::from_pairs(&[
            ("gaps.mosshop.ram.count", 12.0),
            ("gaps.mosshop.sound.count", 1.0),
            ("gaps.mosshop.interactive.count", 3.0),
            ("gaps.mosshop.drawn.count", 1.0),
            ("gaps.mosshop.drawn.kinds", 2.0),
            ("gaps.mosshop.sound.precision", 1.0),
            ("gaps.mosshop.sound.recall", 0.5),
        ]);
        let r = row(&mosshop(), &ctx(&rec, home.path())).unwrap();
        assert_eq!(summary(&r, "detection quality").text, "sound 1.00/0.50");
        assert_eq!(summary(&r, "changes").text, "ram 12, sound 1");
        assert_eq!(summary(&r, "interactive").text, "3 kinds");
        assert_eq!(summary(&r, "drawn").text, "1 of 2 kinds off");
        assert!(
            r.cells
                .iter()
                .any(|c| c.key == "gaps.ram.count" && c.value == 12.0)
        );
        assert_eq!(
            r.detail.gaps,
            vec![
                "ram: Timer, 40 play changes".to_owned(),
                "sound: Sfx_Ping (suggests coins +1 (100%)), 4 count".to_owned(),
                "drawn: walker dy -9".to_owned(),
            ]
        );
        // A game nobody ran gp gaps on says so.
        let r = row(&mosshop(), &ctx(&Recorded::from_pairs(&[]), home.path())).unwrap();
        assert_eq!(summary(&r, "changes").state, "no-run");
    }

    #[test]
    fn chunk_metrics_fill_the_chunks_column() {
        let home = tempfile::tempdir().unwrap();
        let rec = Recorded::from_pairs(&[
            ("chunks.mosshop.kinds", 32.0),
            ("chunks.mosshop.gaps", 1.0),
            ("chunks.mosshop.patterns", 31.0),
            ("chunks.mosshop.coverage", 1.0),
        ]);
        let r = row(&mosshop(), &ctx(&rec, home.path())).unwrap();
        assert_eq!(
            summary(&r, "chunks").text,
            "32 kinds (objects 0, groups 0, gaps 1, patterns 31, compounds 0); coverage 100%"
        );
        assert!(
            r.cells
                .iter()
                .any(|c| c.key == "chunks.coverage" && c.value == 1.0)
        );
        let r = row(&mosshop(), &ctx(&Recorded::from_pairs(&[]), home.path())).unwrap();
        assert_eq!(summary(&r, "chunks").state, "no-run");
    }

    #[test]
    fn recorded_metrics_fill_the_run_columns() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("work/assets/mosshop")).unwrap();
        std::fs::write(
            home.path().join("work/assets/mosshop/slots.toml"),
            "format = \"0.1.0\"\nmodel = \"mosshop\"\n[slots.tiles]\nstatus = \"stub\"\n[slots.palettes]\nstatus = \"stub\"\n[slots.level]\nstatus = \"original\"\n",
        )
        .unwrap();
        let rec = Recorded::from_pairs(&[
            ("validate.mosshop.checks.passed", 20.0),
            ("validate.mosshop.checks.failed", 0.0),
            ("validate.mosshop.checks.unexercised", 1.0),
            ("validate.mosshop.bevy.checks.passed", 3.0),
            ("validate.mosshop.bevy.checks.failed", 2.0),
            ("track.mosshop.a.precision", 1.0),
            ("track.mosshop.a.recall", 0.5),
            ("track.mosshop.b.precision", 0.5),
            ("track.mosshop.b.recall", 0.5),
            ("propose.mosshop.compared", 10.0),
            ("propose.mosshop.agree", 8.0),
            ("template.mosshop.slots_ready", 5.0),
            ("sprites.mosshop.frames", 2.0),
            ("sprites.mosshop.states_covered", 2.0),
            ("sprites.mosshop.states", 2.0),
            ("sprites.mosshop.unobserved", 0.0),
        ]);
        let r = row(&mosshop(), &ctx(&rec, home.path())).unwrap();
        assert_eq!(
            summary(&r, "sprites").text,
            "2 frames, states 2/2, 0 unseen"
        );
        let v = summary(&r, "validate");
        assert_eq!(v.text, "bevy fail 3/2/0, interp pass 20/0/1");
        assert_eq!(v.state, "fail");
        assert_eq!(summary(&r, "track P/R").text, "0.75/0.50");
        assert_eq!(summary(&r, "measured").text, "8 agree, 2 differ of 10");
        assert_eq!(summary(&r, "art").text, "1 original, 2 stub");
        assert!(summary(&r, "slots").text.ends_with("5 ready"));
        let cell = |k: &str| r.cells.iter().find(|c| c.key == k).map(|c| c.value);
        assert_eq!(cell("facts.differ"), Some(2.0));
        assert_eq!(cell("fidelity.bevy.failed"), Some(2.0));
    }

    #[test]
    fn a_commercial_game_without_runs_needs_its_rom() {
        let home = tempfile::tempdir().unwrap();
        let rec = Recorded::from_pairs(&[]);
        // game-printer used mm2-usa, which gameprinter cut; Donkey Kong in
        // the games checkout is a commercial game too. Skips without it.
        let Some(dir) = crate::roms_cmd::games_dir(None)
            .map(|g| g.join("donkey-kong"))
            .filter(|d| d.join("model.toml").is_file())
        else {
            return;
        };
        let r = row(&dir, &ctx(&rec, home.path())).unwrap();
        assert_eq!(summary(&r, "validate").text, "needs ROM");
        assert_eq!(summary(&r, "sprites").state, "needs-rom");
    }

    #[test]
    fn report_section_has_table_trends_and_drill_down() {
        let tmp = tempfile::tempdir().unwrap();
        let db = MetricsDb::open(&tmp.path().join("m.sqlite")).unwrap();
        for v in [2.0, 3.0, 5.0] {
            let mut rec = RunRecorder::start(
                "scorecard",
                vec![],
                Tool {
                    name: "gp".into(),
                    version: "0".into(),
                },
            );
            rec.metric("scorecard.mosshop.facts.agree", v, "count");
            db.record_run(&rec.finish(RunStatus::Ok), Path::new("x"))
                .unwrap();
        }
        let rec = Recorded::from_pairs(&[]);
        let mut r = row(&mosshop(), &ctx(&rec, tmp.path())).unwrap();
        r.cells.push(Cell {
            key: "facts.agree".into(),
            value: 5.0,
            unit: "count".into(),
        });
        r.detail.debt.push("hook_a (rec): why".into());
        r.detail
            .differing
            .push("entities.player.jump.speed: measured 4, model has 5".into());
        let page = html(&db, &Card { rows: vec![r] });
        assert!(page.contains("<h2>Scorecard</h2>"));
        assert!(page.contains("<polyline"));
        assert!(page.contains("hook_a (rec): why"));
        assert!(page.contains("entities.player.jump.speed"));
        assert!(page.contains("no run yet"));
        assert!(!page.contains("http://") && !page.contains("https://"));
    }

    #[test]
    fn text_has_a_row_per_game() {
        let home = tempfile::tempdir().unwrap();
        let rec = Recorded::from_pairs(&[]);
        let r = row(&mosshop(), &ctx(&rec, home.path())).unwrap();
        let t = text(&Card { rows: vec![r] });
        assert_eq!(t.lines().filter(|l| l.starts_with("mosshop")).count(), 1);
        assert!(t.lines().next().unwrap().starts_with("game"));
    }
}
