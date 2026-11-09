//! `gp corpus score`: the corpus of a game (its `clips.toml` and
//! `corpus.toml`) scored on a model, each clip's curve checked against the
//! set's.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Outcome;
use crate::clip_score::{Open, rom_of};
use crate::clipset::{self as cs, ClipSet};
use crate::hosted::Source;

/// The manifest beside a clip set.
pub(crate) const FILE: &str = "corpus.toml";
/// The score record's own format.
pub(crate) const RECORD_FORMAT: &str = "0.1.0";
/// The record's predicate type.
pub(crate) const PREDICATE: &str = "urn:game-printer:score-record:0";

#[derive(Subcommand)]
pub enum CorpusCmd {
    /// Score a game's corpus on a model: every clip made again from the
    /// ROM, played through the model and checked against its curve. Prints
    /// the clips exact and the fidelity and, with --json, the score record.
    Score {
        /// The game's package folder (games/donkey-kong), with its
        /// clips.toml and corpus.toml.
        game: PathBuf,
        /// The run's name: its starts and caches are kept under $GP_HOME.
        #[arg(long)]
        run: String,
        /// The game's built module. Needed for a model with hooks.
        #[arg(long)]
        module: Option<PathBuf>,
        /// The model to score; by default the game's own.
        #[arg(long)]
        model: Option<PathBuf>,
        /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE,
        /// lifted there first if missing).
        #[arg(long)]
        assets: Option<PathBuf>,
        /// The ROM; by default the one in $GP_ROM_DIR the package names.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Score only these clips, by ID. Repeatable.
        #[arg(long)]
        only: Vec<String>,
    },
}

pub(crate) fn run(cmd: &CorpusCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        CorpusCmd::Score {
            game,
            run,
            module,
            model,
            assets,
            rom,
            only,
        } => score(
            paths,
            game,
            run,
            &Open {
                module: module.as_deref(),
                model: model.as_deref(),
                assets: assets.as_deref(),
                rom: rom.as_deref(),
            },
            only,
        ),
    }
}

/// A run's folder under `$GP_HOME/capture`, its name checked.
fn run_dir(home: &Path, run: &str) -> Result<PathBuf> {
    if run.is_empty()
        || run.len() > 80
        || run == "cache"
        || !run
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("a run name is letters, digits, - and _, and not `cache` (got {run:?})");
    }
    Ok(home.join("capture").join(run))
}

/// The manifest's corpus name and version.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorpusFile {
    pub corpus: Head,
    pub target: Target,
    #[serde(default)]
    pub needs: Needs,
    /// Each clip's recipe hash at this version, by ID: what `bump`
    /// compares the clip set with.
    #[serde(default)]
    pub recipes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changelog: Vec<Change>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Head {
    /// The clip set format this manifest is part of, [`cs::FORMAT`].
    pub format: String,
    pub name: String,
    /// Semver of the corpus's content.
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// The content hash over the clips' recipes ([`cs::content_hash`]).
    pub hash: String,
    /// The BLAKE3 of `clips.toml` itself: a change here with the same
    /// `hash` is metadata only.
    #[serde(default)]
    pub metadata: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
}

/// The game a corpus is for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    /// The template or source ID.
    #[serde(default)]
    pub game: String,
    /// The base ROM's headerless SHA-256, as `sources/<platform>.toml`
    /// registers it.
    pub rom: String,
    /// For a hack: the patch's SHA-256.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub patch: String,
}

/// What a corpus needs of the tools that make its clips again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Needs {
    /// gp's version and the corpus generator's, as the clip set says.
    #[serde(default)]
    pub generator: String,
    /// The state mapping's `schema` the curves were measured with.
    #[serde(default)]
    pub mapping: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Change {
    pub version: String,
    pub date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub says: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
}

pub(crate) fn read_manifest(text: &str, what: &str) -> Result<CorpusFile> {
    let c: CorpusFile = toml::from_str(text).with_context(|| format!("{what} isn't a corpus"))?;
    if !cs::readable(&c.corpus.format) {
        bail!(
            "{what} is clip set format {}, this gp reads {}",
            c.corpus.format,
            cs::FORMAT
        );
    }
    if !valid_name(&c.corpus.name) {
        bail!(
            "{what}: corpus.name {:?} isn't lowercase letters, digits, - and _",
            c.corpus.name
        );
    }
    Version::parse(&c.corpus.version)
        .with_context(|| format!("{what}: corpus.version {:?}", c.corpus.version))?;
    Ok(c)
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version(u64, u64, u64);

impl Version {
    pub(crate) fn parse(s: &str) -> Result<Version> {
        let p: Vec<&str> = s.split('.').collect();
        let n = |x: &str| x.parse::<u64>().ok();
        match p.as_slice() {
            [a, b, c] => match (n(a), n(b), n(c)) {
                (Some(a), Some(b), Some(c)) => Ok(Version(a, b, c)),
                _ => bail!("{s} isn't MAJOR.MINOR.PATCH"),
            },
            _ => bail!("{s} isn't MAJOR.MINOR.PATCH"),
        }
    }

    pub(crate) fn next(self, step: Step) -> Version {
        let Version(a, b, c) = self;
        match step {
            Step::None => self,
            Step::Patch => Version(a, b, c + 1),
            Step::Minor => Version(a, b + 1, 0),
            Step::Major => Version(a + 1, 0, 0),
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The version step a change makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    None,
    Patch,
    Minor,
    Major,
}

/// A corpus as read from its folder.
struct Read {
    set: ClipSet,
    recipes: BTreeMap<String, String>,
    hash: String,
    metadata: String,
    manifest: Option<CorpusFile>,
}

fn load(dir: &Path) -> Result<Read> {
    let file = dir.join(cs::FILE);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let set = cs::parse(&text, &file)?;
    let reader = |p: &str| -> Result<String> {
        fs::read_to_string(dir.join(p)).with_context(|| format!("reading {p}"))
    };
    let mut recipes = BTreeMap::new();
    for r in &set.clips {
        let h = cs::recipe_hash(r, &reader).with_context(|| format!("clip {}'s recipe", r.id))?;
        recipes.insert(r.id.clone(), h);
    }
    let manifest = match fs::read_to_string(dir.join(FILE)) {
        Ok(t) => Some(read_manifest(&t, &format!("{}/{FILE}", dir.display()))?),
        Err(_) => None,
    };
    Ok(Read {
        hash: cs::content_hash(&recipes),
        metadata: gp_evidence::Hash::of(text.as_bytes()).to_string(),
        set,
        recipes,
        manifest,
    })
}

#[derive(Default)]
pub(crate) struct Diff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Each changed clip with what changed in it, when both recipes are
    /// at hand (`curve`, `coverage`, `presses`, `start`, `frames`), else `recipe`.
    pub changed: Vec<(String, Vec<&'static str>)>,
    /// The ROM, patch or mapping differ: the clips are for another game,
    /// or their curves were measured another way.
    pub target: Vec<String>,
    /// Anything else differs: names, reasons, parts, guards, the
    /// generator, the manifest's title.
    pub metadata: Vec<String>,
}

impl Diff {
    pub(crate) fn step(&self) -> Step {
        if !self.removed.is_empty() || !self.changed.is_empty() || !self.target.is_empty() {
            Step::Major
        } else if !self.added.is_empty() {
            Step::Minor
        } else if !self.metadata.is_empty() {
            Step::Patch
        } else {
            Step::None
        }
    }
}

fn diff_recorded(m: &CorpusFile, now: &Read) -> Diff {
    let mut d = Diff::default();
    for (id, h) in &now.recipes {
        match m.recipes.get(id) {
            None => d.added.push(id.clone()),
            Some(x) if x != h => d.changed.push((id.clone(), vec!["recipe"])),
            _ => {}
        }
    }
    for id in m.recipes.keys() {
        if !now.recipes.contains_key(id) {
            d.removed.push(id.clone());
        }
    }
    let h = &now.set.clipset;
    if !h.rom.is_empty() && h.rom != m.target.rom {
        d.target
            .push(format!("rom {} -> {}", or_none(&m.target.rom), h.rom));
    }
    if !h.mapping.is_empty() && h.mapping != m.needs.mapping {
        d.target.push(format!(
            "mapping {} -> {}",
            or_none(&m.needs.mapping),
            h.mapping
        ));
    }
    if !h.generator.is_empty() && h.generator != m.needs.generator {
        d.metadata.push(format!(
            "generator {} -> {}",
            or_none(&m.needs.generator),
            h.generator
        ));
    }
    if now.metadata != m.corpus.metadata
        && d.added.is_empty()
        && d.removed.is_empty()
        && d.changed.is_empty()
    {
        d.metadata
            .push("clips.toml's names, reasons, parts or guards".into());
    }
    d
}

fn or_none(s: &str) -> &str {
    if s.is_empty() { "none" } else { s }
}

fn short(h: &str) -> &str {
    &h[..h.len().min(12)]
}

fn checked(dir: &Path) -> Result<(Read, CorpusFile, Vec<String>)> {
    let now = load(dir)?;
    let Some(m) = now.manifest.clone() else {
        bail!("{} has no {FILE}; gp corpus init makes one", dir.display());
    };
    let mut problems = Vec::new();
    if now.hash != m.corpus.hash {
        let d = diff_recorded(&m, &now);
        problems.push(format!(
            "{}'s clips aren't version {}'s (hash {}, recorded {}); gp corpus bump suggests {}",
            dir.display(),
            m.corpus.version,
            short(&now.hash),
            short(&m.corpus.hash),
            Version::parse(&m.corpus.version)?.next(d.step())
        ));
    }
    if !now.set.clipset.rom.is_empty() && now.set.clipset.rom != m.target.rom {
        problems.push(format!(
            "clips.toml is for ROM {}, corpus.toml for {}",
            now.set.clipset.rom, m.target.rom
        ));
    }
    Ok((now, m, problems))
}

/// The gp build: the hash of this binary, or `GP_BUILD` when set.
fn build() -> String {
    if let Ok(b) = std::env::var("GP_BUILD") {
        return b;
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| fs::read(p).ok())
        .map(|b| gp_evidence::Hash::of(&b).to_string())
        .unwrap_or_else(|| format!("gp {}", env!("CARGO_PKG_VERSION")))
}

/// The hash of a model's text as the record names it: its canonical tree's,
/// else the text's, so a reformatted model gets the same record.
fn model_key(text: &str) -> String {
    gp_model::Model::parse(text)
        .ok()
        .and_then(|m| toml::to_string(&m.to_tree()).ok())
        .map_or_else(
            || gp_evidence::Hash::of(text.as_bytes()).to_string(),
            |t| gp_evidence::Hash::of(t.as_bytes()).to_string(),
        )
}

/// The model's lifted assets: `--assets`, else `$GP_HOME/work/assets/SOURCE`,
/// lifted there from the game's asset map and the ROM if not there yet.
fn assets_of(
    paths: &Paths,
    game: &Path,
    mdir: &Path,
    given: Option<&Path>,
    rom: &[u8],
) -> Result<PathBuf> {
    if let Some(a) = given {
        return Ok(a.to_owned());
    }
    let file = mdir.join(gp_model::MODEL_FILE);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let model = gp_model::Model::parse(&text).with_context(|| file.display().to_string())?;
    let Some(source) = &model.model.source else {
        bail!("the model names no source, so give its lifted assets with --assets");
    };
    let out = paths.home.join("work").join("assets").join(source);
    if !out.join(gp_assets::MAP_FILE).is_file() {
        let pkg: toml::Table = fs::read_to_string(game.join("package.toml"))
            .with_context(|| format!("reading {}/package.toml", game.display()))?
            .parse()
            .with_context(|| game.display().to_string())?;
        let entries = pkg
            .get("rom")
            .and_then(|r| r.get("entry"))
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();
        let sha = crate::roms::sha256_of(rom)?;
        let entry = entries
            .iter()
            .find(|e| e.get("sha256").and_then(|v| v.as_str()) == Some(sha.as_str()))
            .context("the package names no asset map for this ROM")?;
        let map_rel = entry
            .get("asset_map")
            .and_then(|v| v.as_str())
            .context("the package's ROM entry has no asset_map")?;
        let map_path = game.join(map_rel);
        let map_text = fs::read_to_string(&map_path)
            .with_context(|| format!("reading {}", map_path.display()))?;
        let map = gp_assets::AssetMap::parse(&map_text)
            .with_context(|| map_path.display().to_string())?;
        let (files, _) = gp_assets::lift(rom, &map, &map_text)?;
        for f in &files {
            let p = out.join(&f.name);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d)?;
            }
            fs::write(&p, &f.bytes)?;
        }
    }
    Ok(out)
}

fn score(paths: &Paths, game: &Path, run: &str, how: &Open, only: &[String]) -> Result<Outcome> {
    let (now, m, mut problems) = checked(game)?;
    let only: Option<BTreeSet<String>> = (!only.is_empty()).then(|| only.iter().cloned().collect());
    let rom = rom_of(paths, game, how.rom)?;
    let mdir = how.model.unwrap_or(game).to_owned();
    let assets = assets_of(paths, game, &mdir, how.assets, &rom)?;
    let (source, _) = Source::open_dir(&mdir, Some(&assets), paths, how.module)?;
    let dir = run_dir(&paths.home, run)?;
    fs::create_dir_all(&dir)?;
    let g = cs::regen_with(
        &cs::Work {
            paths,
            run_dir: &dir,
            run,
            mdir: &mdir,
            assets: &assets,
            rom,
            source: Arc::new(source),
        },
        game,
        only.as_ref(),
    )?;
    if !m.needs.mapping.is_empty() && g.mapping != m.needs.mapping {
        problems.push(format!(
            "the model's state mapping is schema {}, the corpus's curves were measured with {}",
            g.mapping, m.needs.mapping
        ));
    }
    if g.differ > 0 || g.failed > 0 || g.measured > 0 {
        problems.push(format!(
            "{} clips differ from the corpus's curve, {} measured again, {} couldn't be made",
            g.differ, g.measured, g.failed
        ));
    }
    let b = &g.bench;
    let model_hash = model_key(&g.model_text);
    // The contract's bars for the corpus's clips, when it sets them.
    let bars = fs::read_to_string(b.dir.join(gp_contract::CONTRACT_FILE))
        .ok()
        .and_then(|t| gp_contract::Contract::parse(&t).ok())
        .and_then(|c| c.corpus);
    let bar_together = bars
        .as_ref()
        .map(|c| c.together_permille())
        .transpose()
        .map_err(anyhow::Error::msg)?
        .flatten();
    let bar_longest = bars
        .as_ref()
        .map(|c| c.longest_frames())
        .transpose()
        .map_err(anyhow::Error::msg)?
        .flatten();
    let mut clips = Vec::new();
    for r in &g.set.clips {
        if only.as_ref().is_some_and(|o| !o.contains(&r.id)) {
            continue;
        }
        let row = match g.outcomes.get(&r.id) {
            Some(Ok(o)) => {
                // Frames on which every measure agrees, of those measured.
                let all = (o.together_frames * 1000)
                    .checked_div(o.measured)
                    .unwrap_or(0);
                let together_ok = bar_together.map(|t| all >= t);
                // A long run is held to the longest stretch only when it
                // has that much play: a game over or a clear scene has
                // none.
                let longest_ok = match (&o.stretches, bar_longest) {
                    (Some(st), Some(t)) if st.play >= t => Some(st.longest_any() >= t),
                    _ => None,
                };
                let mut row = json!({"id": r.id, "recipe": now.recipes[&r.id],
                    "curve": crate::scenario::curve_hash(o, false),
                    "model_curve": crate::scenario::curve_hash(o, true),
                    "exact": o.exact, "permille": o.headline.together_permille, "score": o.score,
                    "frames": o.measured, "frames_together": o.together_frames,
                    "first": o.first.as_ref().map(|f| json!({"frame": f.frame, "original_frame": f.original_frame, "path": f.path})),
                    "moved": o.moved, "start": o.original_from,
                    "pairing": o.pairing, "out_of_scope": o.out_of_scope});
                if o.stopped {
                    row["stopped"] = json!(true);
                }
                if let Some(st) = &o.stretches {
                    row["stretches"] = json!({
                        "frames": st.frames, "together": st.together, "play": st.play,
                        "split_events": st.events,
                        "whole": st.whole.len(), "open_end": st.open_end,
                        "longest": st.longest(), "longest_any": st.longest_any(),
                        "median": st.median(), "p10": st.percentile(10),
                        "shortest": st.whole.first().copied().unwrap_or(0),
                        "at_least_5s": st.at_least(5), "at_least_10s": st.at_least(10),
                        "at_least_30s": st.at_least(30),
                    });
                }
                // The screen measure, for the gate's picture lane.
                if let Some(sc) = &o.screen {
                    row["screen"] = json!({
                        "sprites": {"same": sc.sprites.same, "total": sc.sprites.total},
                        "background": {"same": sc.background.same, "total": sc.background.total},
                    });
                }
                if together_ok.is_some() || longest_ok.is_some() {
                    row["bars"] = json!({"together": together_ok, "longest_stretch": longest_ok});
                }
                row
            }
            Some(Err(e)) => json!({"id": r.id, "recipe": now.recipes[&r.id], "error": e}),
            None => json!({"id": r.id, "recipe": now.recipes[&r.id], "error": "not made"}),
        };
        clips.push(row);
    }
    // The totals, from the rows: those scored now and those kept alike.
    let mut bar_lines = Vec::new();
    let mut pair_lines: Vec<String> = Vec::new();
    let (mut met, mut held) = (0usize, 0usize);
    let (mut exact, mut errors, mut permille_sum) = (0usize, 0usize, 0u64);
    for row in &clips {
        let id = row["id"].as_str().unwrap_or("");
        if row.get("error").is_some_and(|e| !e.is_null()) {
            errors += 1;
            continue;
        }
        if row["exact"] == true {
            exact += 1;
        }
        permille_sum += row["permille"].as_u64().unwrap_or(0);
        let together_ok = row["bars"]["together"].as_bool();
        let longest_ok = row["bars"]["longest_stretch"].as_bool();
        if together_ok.is_some() || longest_ok.is_some() {
            held += 1;
            if together_ok != Some(false) && longest_ok != Some(false) {
                met += 1;
            } else {
                let (t, f) = (
                    row["frames_together"].as_u64().unwrap_or(0),
                    row["frames"].as_u64().unwrap_or(0),
                );
                let all = (t * 1000).checked_div(f).unwrap_or(0);
                let mut why = vec![format!(
                    "{t}/{f} frames together ({}.{}%)",
                    all / 10,
                    all % 10
                )];
                if let Some(l) = row["stretches"]["longest_any"].as_u64() {
                    why.push(format!(
                        "longest stretch {} s",
                        gp_interp::lockstep::seconds(l)
                    ));
                }
                bar_lines.push(format!("  below the bars: {id} ({})", why.join(", ")));
            }
        }
        // A clip whose frames don't pair one to one from its start, or
        // that runs past the template's scope, says so.
        let mut said = Vec::new();
        if let Ok(p) =
            serde_json::from_value::<gp_interp::lockstep::Pairing>(row["pairing"].clone())
            && !p.lag.is_empty()
        {
            said.push(p.says());
        }
        if let Some(x) = row["out_of_scope"].as_object() {
            said.push(format!(
                "out of scope from the original's frame {} (the clip's {}), {} frames left out",
                x.get("original_frame").and_then(Value::as_u64).unwrap_or(0),
                x.get("frame").and_then(Value::as_u64).unwrap_or(0),
                x.get("frames").and_then(Value::as_u64).unwrap_or(0)
            ));
        }
        if !said.is_empty() {
            pair_lines.push(format!("  {id}: {}", said.join("; ")));
        }
    }
    let n = clips.len();
    // Every clip counts: one that can't be scored counts as 0.
    let fidelity = if n == 0 { 0 } else { permille_sum / n as u64 };
    let model_name = gp_model::Model::parse(&g.model_text)
        .map(|x| x.model.name)
        .unwrap_or_default();
    let contract = fs::read(b.dir.join(gp_contract::CONTRACT_FILE)).unwrap_or_default();
    let h = |x: &[u8]| gp_evidence::Hash::of(x).to_string();
    let record = json!({
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [
            {"name": format!("corpus {} {}", m.corpus.name, m.corpus.version), "digest": {"blake3": m.corpus.hash}},
            {"name": format!("model {model_name}"), "digest": {"blake3": model_hash}},
        ],
        "predicateType": PREDICATE,
        "predicate": {
            "record_format": RECORD_FORMAT,
            "corpus": {"name": m.corpus.name, "version": m.corpus.version, "hash": m.corpus.hash},
            "rom": {"sha256": m.target.rom, "patch": if m.target.patch.is_empty() { Value::Null } else { json!(m.target.patch) }},
            "model": {"hash": model_hash, "short": crate::scenario::model_hash(&g.model_text), "name": model_name},
            "mapping": {"hash": h(b.mapping_text.as_bytes()), "schema": g.mapping},
            "contract": h(&contract),
            "profile": Value::Null,
            "runtime": {"name": "gp-interp", "scenario": gp_interp::scenario::SCENARIO_VERSION},
            "gp": {"version": env!("CARGO_PKG_VERSION"), "build": build()},
            "results": {"clips": n, "exact": exact, "errors": errors, "fidelity_permille": fidelity,
                "bars": if held > 0 { json!({"held": held, "met": met}) } else { Value::Null }},
            "clips": clips,
        },
    });
    let mut text = vec![format!(
        "On corpus {} {} ({}): {exact} of {n} clips exact, fidelity {}.{}%, {errors} couldn't be scored. Model {}.",
        m.corpus.name,
        m.corpus.version,
        short(&m.corpus.hash),
        fidelity / 10,
        fidelity % 10,
        short(&model_hash),
    )];
    if held > 0 {
        text.push(format!(
            "The contract's corpus bars{}{}: {met} of {held} clips meet them.",
            bar_together
                .map(|t| format!(", {}.{}% together", t / 10, t % 10))
                .unwrap_or_default(),
            bar_longest
                .map(|f| format!(
                    ", a longest stretch of {} s on a long run",
                    gp_interp::lockstep::seconds(f)
                ))
                .unwrap_or_default(),
        ));
        text.extend(bar_lines);
    }
    let offsets: BTreeSet<i64> = clips
        .iter()
        .filter_map(|c| c["pairing"]["offset"].as_i64())
        .collect();
    if !offsets.is_empty() {
        text.push(format!(
            "Pairing: model frame n is the original's n+k at each clip's start, k {}; {} clips skip a lag frame or leave the template's scope{}",
            offsets
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            pair_lines.len(),
            if pair_lines.is_empty() { "." } else { ":" }
        ));
        text.extend(pair_lines);
    }

    for p in &problems {
        text.push(format!("problem: {p}"));
    }
    let v = json!({"ok": problems.is_empty(), "record": record, "problems": problems});
    let mut out = Outcome::ok(v, text.join("\n"));
    if !problems.is_empty() {
        out.exit = 1;
    }
    Ok(out)
}
