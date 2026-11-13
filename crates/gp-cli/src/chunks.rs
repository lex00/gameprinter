//! `gp chunks`: a template's level chunk catalog (docs/chunks.md). It reads
//! the model's levels from their lifted assets, mines them in two layers
//! (the format's object vocabulary and patterns in the decoded grid), says
//! what each chunk does, and writes the catalog (docs/formats/chunks.md)
//! under `$GP_HOME/work/chunks/<template>/`, since it holds the game's
//! content. Detect and catalog only.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use serde_json::json;

use gp_chunks::format::{self, Catalog, Layer, Settings};
use gp_evidence::Paths;

use crate::Outcome;

#[derive(Args)]
pub struct ChunksArgs {
    /// A template directory holding model.toml, a template id under
    /// templates/ or a game in the games checkout.
    pub template: Option<PathBuf>,
    /// The model's lifted assets (default `$GP_HOME/work/assets/<source>`).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// Only these model levels (repeat for more).
    #[arg(long = "level", value_name = "NAME")]
    pub levels: Vec<String>,
    /// The interactive element catalog whose cell kinds to match (default
    /// `$GP_HOME/reports/gaps/<template>/interactive.json`, when it exists).
    #[arg(long)]
    pub interactive: Option<PathBuf>,
    /// Occurrences a mined pattern, group or compound needs.
    #[arg(long, default_value_t = 2)]
    pub min_count: usize,
    /// Window patterns kept per size.
    #[arg(long, default_value_t = 200)]
    pub max_patterns: usize,
    /// Chunk kinds per layer given a reachability test (0: none).
    #[arg(long, default_value_t = 6)]
    pub reach_tests: usize,
    /// Chunks printed per layer.
    #[arg(long, default_value_t = 8)]
    pub top: usize,
    /// Where to write the catalog, instead of
    /// `$GP_HOME/work/chunks/<template>/chunks.json`.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// The game's built module. Needed for the reachability tests of a
    /// model with hooks.
    #[arg(long, value_name = "FILE")]
    pub module: Option<PathBuf>,
    /// Print the catalog format's JSON Schema and exit.
    #[arg(long)]
    pub schema: bool,
}

pub(crate) fn run(args: &ChunksArgs, paths: &Paths) -> Result<Outcome> {
    if args.schema {
        let text = format::schema();
        return Ok(Outcome::ok(
            serde_json::from_str(&text).context("the schema")?,
            text.trim_end().to_owned(),
        ));
    }
    let Some(dir) = &args.template else {
        bail!("name a template directory, or pass --schema");
    };
    let dir = &crate::level_kit::template_dir(dir)?;
    let model = crate::mechanics::load_model(dir)?;
    let id = dir
        .canonicalize()
        .unwrap_or_else(|_| dir.clone())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| model.model.name.clone());
    let assets = match &args.assets {
        Some(a) => a.clone(),
        None => {
            let Some(source) = &model.model.source else {
                bail!("the model names no source, so give its lifted assets with --assets");
            };
            paths.home.join("work/assets").join(source)
        }
    };
    let loaded = gp_chunks::source::load(&model, &assets, &args.levels).map_err(|e| {
        anyhow::anyhow!(
            "{e}\n(lift the model's assets with gp assets lift or gp template build, or name them with --assets)"
        )
    })?;
    if loaded.grids.is_empty() {
        bail!(
            "no level of the model could be read: {}",
            loaded.notes.join("; ")
        );
    }
    let interactive_path = args.interactive.clone().or_else(|| {
        let p = paths
            .home
            .join("reports/gaps")
            .join(&id)
            .join("interactive.json");
        p.exists().then_some(p)
    });
    let interactive = match &interactive_path {
        Some(p) => {
            let text =
                std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            let v: serde_json::Value =
                serde_json::from_str(&text).with_context(|| p.display().to_string())?;
            gp_chunks::interactive_kinds(&v)
        }
        None => Vec::new(),
    };
    // The reachability tests step the model through the game's module
    // (`--module`) or the built-in interpreter, as gp interp reach does.
    let src = crate::hosted::Source::open_dir(dir, Some(&assets), paths, args.module.as_deref())
        .map(|(s, _)| s);
    let opts = gp_chunks::Options {
        min_count: args.min_count.max(1),
        max_patterns: args.max_patterns,
        reach_tests: args.reach_tests,
        ..gp_chunks::Options::default()
    };
    let native = gp_interp::variant::NativeSim(&gp_sim::Exts::new());
    let sim: &dyn gp_interp::variant::Sim = match &src {
        Ok(s) => s,
        Err(_) => &native,
    };
    let tester = gp_chunks::demand::Reach::new(&loaded.spec, sim);
    let mined = gp_chunks::mine(
        &loaded.grids,
        &opts,
        &interactive,
        (args.reach_tests > 0).then_some(&tester as &dyn gp_chunks::Tester),
    );
    let mut notes = loaded.notes.clone();
    notes.extend(mined.notes.iter().cloned());
    if tester.envelope.is_none() {
        notes.push(format!(
            "the player's jump couldn't be measured ({}), so no reachability test ran",
            tester.why_not.as_deref().unwrap_or("no level has a player")
        ));
    }
    let catalog = Catalog {
        format: format::FORMAT.into(),
        version: format::VERSION.into(),
        template: id.clone(),
        model: model.model.name.clone(),
        source: model.model.source.clone(),
        settings: Settings {
            min_count: opts.min_count,
            windows: opts.windows.clone(),
            slices: opts.slices,
            max_patterns: opts.max_patterns,
            max_group: opts.max_group,
            reach_tests: opts.reach_tests,
        },
        envelope: tester.envelope.map(|e| format::Envelope {
            rise: e.rise,
            airtime: e.airtime,
            walk_distance: e.walk_distance,
            run_distance: e.run_distance,
        }),
        interactive: interactive_path.as_ref().map(|p| p.display().to_string()),
        levels: mined.levels,
        layers: mined.layers,
        controls: mined.controls,
        chunks: mined.chunks,
        notes,
    };
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| paths.home.join("work/chunks").join(&id).join("chunks.json"));
    refuse_repo(&out)?;
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p).with_context(|| format!("creating {}", p.display()))?;
    }
    let text = serde_json::to_string_pretty(&catalog)? + "\n";
    std::fs::write(&out, &text).with_context(|| format!("writing {}", out.display()))?;

    let l = catalog.layers;
    let kinds = l.objects + l.groups + l.gaps + l.patterns + l.compounds;
    let mean = |f: fn(&format::Coverage) -> f64| {
        let n = catalog.levels.len().max(1) as f64;
        catalog.levels.iter().map(|lv| f(&lv.coverage)).sum::<f64>() / n
    };
    let cov_any = mean(|c| c.any);
    let text = report(&catalog, args.top, &out);
    Ok(Outcome::ok(
        json!({
            "ok": true,
            "template": id,
            "catalog": out,
            "levels": catalog.levels.len(),
            "layers": catalog.layers,
            "kinds": kinds,
            "coverage": {
                "any": cov_any,
                "objects": mean(|c| c.objects),
                "groups": mean(|c| c.groups),
                "patterns": mean(|c| c.patterns),
            },
            "envelope": catalog.envelope,
            "notes": catalog.notes,
        }),
        text,
    ))
}

/// The catalog holds a game's content, so it never goes in a git work tree.
fn refuse_repo(out: &Path) -> Result<()> {
    let mut p = out.parent().map(Path::to_path_buf);
    while let Some(d) = p {
        if d.join(".git").exists() {
            bail!(
                "{} is inside the git work tree {}; a chunk catalog holds the game's levels, so write it under $GP_HOME",
                out.display(),
                d.display()
            );
        }
        p = d.parent().map(Path::to_path_buf);
    }
    Ok(())
}

fn pct(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

/// A line saying what a chunk does.
fn summary(c: &gp_chunks::Chunk) -> String {
    let f = &c.function;
    let mut parts = Vec::new();
    let classes: Vec<String> = f
        .classes
        .iter()
        .filter(|(k, _)| k.as_str() != "empty")
        .map(|(k, v)| format!("{k} {v}"))
        .collect();
    if !classes.is_empty() {
        parts.push(classes.join(" "));
    }
    if !f.tile_kinds.is_empty() {
        parts.push(
            f.tile_kinds
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if !f.interactive.is_empty() {
        let mut ks: Vec<String> = f.interactive.keys().take(3).cloned().collect();
        if f.interactive.len() > 3 {
            ks.push(format!("{} more", f.interactive.len() - 3));
        }
        parts.push(format!("interactive: {}", ks.join(", ")));
    }
    if let Some(e) = f.enemies.first() {
        parts.push(format!("{} near {}", e.entity, pct(e.share)));
    }
    if let Some(d) = &f.demand {
        parts.push(format!(
            "needs {}{}{}{}",
            d.needs,
            if d.height_px > 0 {
                format!(", wall {} px", d.height_px)
            } else {
                String::new()
            },
            if d.gap_px > 0 {
                format!(", gap {} px", d.gap_px)
            } else {
                String::new()
            },
            if d.stands_on_top { ", stood on" } else { "" }
        ));
    }
    let p = &f.placement;
    parts.push(format!("ground {} air {}", p.ground, p.air));
    parts.join("; ")
}

fn report(c: &Catalog, top: usize, out: &Path) -> String {
    let l = c.layers;
    let mut t = format!(
        "{}: {} layouts, {} object kinds, {} groups, {} gaps, {} patterns, {} compounds\n",
        c.template,
        c.levels.len(),
        l.objects,
        l.groups,
        l.gaps,
        l.patterns,
        l.compounds
    );
    if let Some(e) = c.envelope {
        t.push_str(&format!(
            "jump: rises {} px, {} frames, carries {} px walking{}\n",
            e.rise,
            e.airtime,
            e.walk_distance,
            e.run_distance
                .map_or(String::new(), |d| format!(", {d} running"))
        ));
    }
    t.push_str("coverage of each layout's structure (objects, groups, patterns, any):\n");
    for lv in &c.levels {
        let v = lv.coverage;
        t.push_str(&format!(
            "  {:<24} {:>5} cells  {:>4} {:>4} {:>4} {:>4}\n",
            lv.name,
            lv.cells,
            pct(v.objects),
            pct(v.groups),
            pct(v.patterns),
            pct(v.any)
        ));
    }
    for layer in [
        Layer::Objects,
        Layer::Groups,
        Layer::Gaps,
        Layer::Compounds,
        Layer::Patterns,
    ] {
        let list: Vec<&gp_chunks::Chunk> = c
            .chunks
            .iter()
            .filter(|k| k.layer == layer)
            .take(top)
            .collect();
        if list.is_empty() {
            continue;
        }
        t.push_str(&format!("{layer:?}:\n"));
        for k in list {
            t.push_str(&format!(
                "  {} ({} times in {} layouts, {}x{}): {}\n",
                k.name,
                k.count,
                k.by_level.len(),
                k.size[0],
                k.size[1],
                summary(k)
            ));
        }
    }
    for n in &c.notes {
        t.push_str(&format!("note: {n}\n"));
    }
    t.push_str(&format!("catalog: {}", out.display()));
    t
}
