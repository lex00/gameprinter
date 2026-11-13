//! `gp bench scratch`: the benchmark from scratch
//! (docs/plan-extraction-process.md, "From scratch, four stages").
//!
//! The start is what the plan allows: the ROM, the game's package with its
//! corpus (template.toml, corpus.toml and inputs/), and a model that holds
//! only the header the schema needs (`[model]`'s schema, name, title,
//! source, view and frame rate). No asset slots, entities, states,
//! transitions, rules or counters, no evidence, and no state mapping:
//! those are the facts the tools must find. Each stage runs the tools on
//! what the stage before left, and what each tool produced is applied to a
//! copy and graded against the real model and mapping.
//!
//! Ported from game-printer, whose stages ran its capture-run loop on
//! baselines. Each stage is gameprinter's pieces now:
//!
//! | Stage | game-printer | Here |
//! |---|---|---|
//! | 0. Static harvest | The bench file's `static_command` | The same; `gp model derive` and any source or labels file refused |
//! | 1. Clip generation | `gp capture start`, `grid --mode batch`, `generate`, `background`, `gp clips matrix` | `gp capture dense` on the stage's package |
//! | 2. Trace harvest | `gp model propose` on the capture run's recordings | `gp model propose` on recordings of the corpus clips, made by `gp clip log` and `gp record` in the bench's home |
//! | 3. Refinement | Rounds of `gp solve --check` and `gp model refine --lockstep` on each baseline | Rounds of `gp solve --check` and `gp model refine --lockstep` on the corpus |
//!
//! Exact play is the corpus scored by `gp corpus score` (frames together,
//! clips exact) instead of the baselines through `gp interp compare`; the
//! system order and tile kinds are graded by behaviour on the same scores
//! (two models play the same when every clip's model curve is the same).
//! A tool that fails is recorded with the last line of its error, and the
//! run goes on. Facts are every leaf of the real model (but its evidence
//! and documentation) and of its state mapping; one is right when the
//! produced model or mapping holds the same value at the same path. The
//! target comes first: minutes of run time per second of the corpus
//! played exactly.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_evidence::{Hash, Paths};
use serde::Deserialize;
use serde_json::{Value, json};
use toml::Value as Toml;

use crate::bench_clips::{Head, Tools, copy_dir, s, score_rows};
use crate::bench_grade::{Grader, elements, facts_of, grade_dir, read_toml, round3};

/// The JSON shape of `gp bench scratch`. A change to it bumps this.
pub const SCRATCH_SCHEMA: &str = "gp bench scratch 3";

#[derive(Args, Clone)]
pub struct ScratchArgs {
    /// The game (its bench file is bench/GAME.toml in the gameprinter
    /// repo), or a bench file.
    pub game: String,
    /// The ROM, instead of the one the package accepts in $GP_ROM_DIR.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// The directory the bench file's paths are relative to (default: the
    /// games checkout, $GP_GAMES_DIR or ../games beside the repo).
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// The bench's own home, made afresh (default:
    /// $GP_HOME/work/bench/GAME-scratch).
    #[arg(long)]
    pub work: Option<PathBuf>,
    /// A `gp bench clips` report.json, shown beside stage 1: clip
    /// generation on the hand-written model and mapping, not from scratch.
    #[arg(long)]
    pub clips: Option<PathBuf>,
    /// Run every stage a second time with scrambled copies of the answer
    /// key where a tool could find them; each stage's output must come out
    /// byte for byte the same.
    #[arg(long)]
    pub scramble: bool,
    /// Check the grader first: a null tool (the start) scores no fact, an
    /// oracle tool (the real model and mapping) scores every fact and plays
    /// every frame exactly, and a wrong tool (the real model with numbers
    /// changed) has each change graded wrong.
    #[arg(long = "self-test")]
    pub self_test: bool,
}

#[derive(Deserialize)]
struct BenchFile {
    bench: Head,
    scratch: ScratchSpec,
}

#[derive(Deserialize)]
struct ScratchSpec {
    rounds: u32,
    /// Stage 0's command, with `{model}` for the model folder, `{rom}`,
    /// `{asset_map}` and `{manifest}`. Its `--json` output's `facts` with a
    /// `path`, a `value` and a `status` of `agrees` or `new` are written
    /// into the model. Empty: no stage 0 tool.
    #[serde(default)]
    static_command: Vec<String>,
    /// The asset map, under the root; default: the package's
    /// `recipe.asset_map`.
    #[serde(default)]
    asset_map: Option<PathBuf>,
    /// How many corpus clips stage 2 records for `gp model propose`.
    #[serde(default = "recordings")]
    recordings: usize,
    /// game-printer's template manifest; the package's template.toml now.
    #[serde(default)]
    #[allow(dead_code)]
    manifest: Option<PathBuf>,
}

fn recordings() -> usize {
    4
}

/// The start: the real model's header only, as much as the schema needs
/// (schema, name, title, source, view and frame rate). An asset slot needs a role, which the asset map doesn't
/// give, so none is in the start; the lift reads the asset map itself.
pub(crate) fn start_model(real: &Toml) -> Result<String> {
    let mut out = toml::map::Map::new();
    let mut head = toml::map::Map::new();
    for k in ["schema", "name", "title", "source", "view", "frame_rate"] {
        if let Some(v) = real.get("model").and_then(|m| m.get(k)) {
            head.insert(k.to_owned(), v.clone());
        }
    }
    out.insert("model".into(), Toml::Table(head));
    Ok(toml::to_string(&Toml::Table(out))?)
}

/// Refuses a stage 0 command that reads the source or labels: stage 0
/// reads only the ROM and the asset map.
fn check_static(cmd: &[String], forbidden: &[PathBuf]) -> Result<()> {
    if cmd.len() >= 2 && cmd[0] == "model" && cmd[1] == "derive" {
        bail!(
            "stage 0 reads only the ROM and the asset map: `gp model derive` reads the assembly source the evidence cites (crates/gp-derive/src/asm.rs)"
        );
    }
    for a in cmd {
        if a == "--with-source" {
            bail!(
                "stage 0 reads only the ROM and the asset map: it may not run with --with-source"
            );
        }
        let p = Path::new(a);
        let source_like = matches!(
            p.extension().and_then(|x| x.to_str()),
            Some("s" | "asm" | "inc" | "labels" | "dbg")
        ) || p
            .file_name()
            .is_some_and(|n| n.to_string_lossy().contains("labels"))
            || forbidden.iter().any(|f| f.ends_with(p) || p.ends_with(f));
        if source_like {
            bail!("stage 0 reads only the ROM and the asset map: {a} is a source or labels file");
        }
    }
    Ok(())
}

fn set_path(doc: &mut Toml, path: &str, val: Toml) {
    let segs: Vec<&str> = path.split('.').collect();
    let mut v = doc;
    for (i, seg) in segs.iter().enumerate() {
        let Some(t) = v.as_table_mut() else { return };
        if i + 1 == segs.len() {
            t.insert((*seg).to_owned(), val);
            return;
        }
        v = t
            .entry((*seg).to_owned())
            .or_insert_with(|| Toml::Table(toml::map::Map::new()));
    }
}

/// A measured value as a model writes it: an integer as one, else text.
fn written(measured: &str) -> Toml {
    match measured.parse::<i64>() {
        Ok(i) => Toml::Integer(i),
        Err(_) => Toml::String(measured.to_owned()),
    }
}

/// One tool's run: whether it ran, and its first line of error with the
/// work folder's path taken out, so the report is the same anywhere.
fn ran<T>(r: &Result<T>, work: &Path) -> Value {
    match r {
        Ok(_) => json!({"ok": true}),
        Err(e) => {
            let msg = format!("{e:#}").replace(&work.display().to_string(), "<work>");
            let line = msg
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .to_owned();
            json!({"ok": false, "error": line})
        }
    }
}

/// The grader's own scoring: the corpus of the real package (the answer
/// key's folder, so its hooks build), scored on a model folder with `gp
/// corpus score --model`. Run by the bench itself, never by a stage tool.
pub(crate) struct Scorer {
    tools: Tools,
    game: PathBuf,
    /// Every clip's frames, from the real model's score.
    pub(crate) frames: u64,
    /// The frames the real model plays exactly: the oracle's bar, since a
    /// hand-written model needn't play every frame of its corpus.
    pub(crate) frames_real: u64,
    /// The original's frames a second.
    pub(crate) fps: f64,
}

impl Scorer {
    fn score(&self, dir: &Path) -> Result<Value> {
        anyhow::ensure!(
            dir.join(gp_interp::mapping::MAPPING_FILE).exists(),
            "no state mapping: nothing compares the two games"
        );
        crate::bench_clips::with_hooks(&self.game, dir)?;
        let (v, _) = self.tools.json_exit(
            &[
                "corpus",
                "score",
                &s(&self.game),
                "--run",
                "grade",
                "--model",
                &s(dir),
            ],
            true,
        )?;
        Ok(v)
    }

    /// The original's frames on which every mapped value is equal, over the
    /// corpus, and the clips played exactly.
    pub(crate) fn exact(&self, dir: &Path) -> Result<(u64, Vec<String>)> {
        let v = self.score(dir)?;
        let rows = score_rows(&v);
        let frames = rows
            .values()
            .filter_map(|r| r["frames_together"].as_u64())
            .sum();
        let exact = rows
            .iter()
            .filter(|(_, r)| r["exact"] == true)
            .map(|(id, _)| id.clone())
            .collect();
        Ok((frames, exact))
    }

    /// Whether two models play every clip the same, frame for frame (the
    /// same model curve), and the clips played.
    pub(crate) fn same_play(&self, a: &Path, b: &Path) -> Result<(bool, usize)> {
        let (x, y) = (score_rows(&self.score(a)?), score_rows(&self.score(b)?));
        let curve = |r: &Value| r["model_curve"].as_str().map(str::to_owned);
        let same = x.len() == y.len()
            && x.iter()
                .all(|(id, r)| curve(r).is_some() && y.get(id).map(curve) == Some(curve(r)));
        Ok((same, x.len()))
    }
}

/// The path of the model's system order among the facts.
const ORDER_FACT: &str = "model.systems";

/// What a tile kind stops, by the core's meaning of its name: `stop` for
/// the kinds that stop movement, `top` for a top-only one, `none` else.
fn kind_stops(kind: &str) -> &'static str {
    match kind {
        "solid" | "ice" | "coin_block" | "item_block" | "brick" => "stop",
        "top" => "top",
        _ => "none",
    }
}

/// A tile kind's behaviour in a model, with its name left out: what it
/// stops, and what a head hit and a pickup on it do (whether an event
/// fires, the speed after, and what the cell becomes, by what that stops).
fn kind_behaviour(model: &Toml, kind: &str) -> String {
    let mut parts = vec![format!("stops {}", kind_stops(kind))];
    let entities = model.get("entities").and_then(Toml::as_table);
    for (name, e) in entities.into_iter().flatten() {
        let jump = e.get("jump");
        let hits = jump
            .and_then(|j| j.get("head"))
            .and_then(|h| h.get("hits"))
            .and_then(Toml::as_array);
        for h in hits.into_iter().flatten() {
            if h.get("tile").and_then(Toml::as_str) == Some(kind) {
                parts.push(format!(
                    "{name} head hit: event {}, speed {}, becomes {}",
                    h.get("event").is_some(),
                    h.get("speed").map(Toml::to_string).unwrap_or_default(),
                    h.get("becomes")
                        .and_then(Toml::as_str)
                        .map(kind_stops)
                        .unwrap_or("same"),
                ));
            }
        }
        if let Some(p) = jump.and_then(|j| j.get("pickup"))
            && p.get("tile").and_then(Toml::as_str) == Some(kind)
        {
            parts.push(format!(
                "{name} pickup: event {}, becomes {}",
                p.get("event").is_some(),
                kind_stops(p.get("becomes").and_then(Toml::as_str).unwrap_or("empty")),
            ));
        }
    }
    parts.join("; ")
}

/// Each level's tile kinds by tile value.
fn tile_kinds(model: &Toml) -> BTreeMap<(String, String), String> {
    let mut out = BTreeMap::new();
    let levels = model.get("levels").and_then(Toml::as_table);
    for (l, v) in levels.into_iter().flatten() {
        let kinds = v.get("tile_kinds").and_then(Toml::as_table);
        for (value, k) in kinds.into_iter().flatten() {
            if let Some(k) = k.as_str() {
                out.insert((l.clone(), value.clone()), k.to_owned());
            }
        }
    }
    out
}

/// A fact whose value names a tile kind.
fn is_kind_fact(path: &str) -> bool {
    let leaf = path.rsplit('.').next().unwrap_or("");
    path.contains(".tile_kinds.")
        || ((path.contains(".jump.head.hits[") || path.contains(".jump.pickup."))
            && (leaf == "tile" || leaf == "becomes"))
}

/// The tile kinds graded by behaviour (docs/benchmarks.md): each kind the
/// model gives a tile value is matched to the real kind on the same tile
/// value when the two behave the same in their models, names aside. A kind
/// fact (a tile value's kind, or a head hit's or pickup's tile or what it
/// becomes) whose name differs from the real one is right when the
/// matching takes the stated name to the real one. The kinds named exactly
/// as in the real model are counted apart, as readings. Changes `grade`
/// in place; returns the row, or null without tile kinds.
fn grade_kinds(dir: &Path, real: &Toml, grade: &mut Value) -> Value {
    let Some(have) = read_toml(&dir.join(gp_model::MODEL_FILE)) else {
        return Value::Null;
    };
    let (theirs, ours) = (tile_kinds(&have), tile_kinds(real));
    if theirs.is_empty() {
        return Value::Null;
    }
    // Names the same in both models stand for themselves.
    let mut matched: BTreeMap<String, String> = BTreeMap::new();
    let (mut same, mut differ) = (Vec::new(), Vec::new());
    for (at, kb) in &theirs {
        let Some(kr) = ours.get(at) else { continue };
        if kind_behaviour(&have, kb) == kind_behaviour(real, kr) {
            matched.insert(kb.clone(), kr.clone());
            same.push(json!({"tile": format!("{}.{}", at.0, at.1), "stated": kb, "real": kr}));
        } else {
            differ.push(
                json!({"tile": format!("{}.{}", at.0, at.1), "stated": kb, "real": kr,
                "stated_does": kind_behaviour(&have, kb), "real_does": kind_behaviour(real, kr)}),
            );
        }
    }
    let real_facts = facts_of(real, None);
    let have_facts = facts_of(&have, None);
    let f = &mut grade["facts"];
    let wrong: Vec<String> = f["wrong_paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(str::to_owned))
        .collect();
    let mut moved = 0i64;
    for p in &wrong {
        if !is_kind_fact(p) {
            continue;
        }
        let (Some(h), Some(r)) = (
            have_facts.get(p).and_then(Toml::as_str),
            real_facts.get(p).and_then(Toml::as_str),
        ) else {
            continue;
        };
        let to = matched.get(h).map(String::as_str).unwrap_or(h);
        if to == r {
            if let Some(w) = f["wrong_paths"].as_array_mut() {
                w.retain(|x| x != p.as_str());
            }
            if let Some(rp) = f["right_paths"].as_array_mut() {
                rp.push(json!(p));
            }
            moved += 1;
        }
    }
    for (k, d) in [("wrong", -moved), ("right", moved)] {
        let n = f[k].as_i64().unwrap_or(0) + d;
        f[k] = json!(n.max(0));
    }
    let named_as_real = theirs
        .iter()
        .filter(|(at, kb)| ours.get(*at) == Some(*kb))
        .count();
    json!({"by": "behaviour", "matched": same, "differ": differ, "regraded_right": moved,
        "names": {"tile_values": theirs.len(), "named_as_real": named_as_real}})
}

/// A copy of a model folder with the given system order, in `to`.
fn with_order(from: &Path, order: &Toml, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    let mut doc: Toml = toml::from_str(&fs::read_to_string(from.join(gp_model::MODEL_FILE))?)?;
    set_path(&mut doc, ORDER_FACT, order.clone());
    fs::write(to.join(gp_model::MODEL_FILE), toml::to_string(&doc)?)?;
    let map = from.join(gp_interp::mapping::MAPPING_FILE);
    if map.exists() {
        fs::copy(&map, to.join(gp_interp::mapping::MAPPING_FILE))?;
    }
    Ok(())
}

/// The system order graded by behaviour (docs/benchmarks.md): a stated
/// order that differs from the real one is right when the rebuild with it
/// plays every corpus clip frame for frame as the rebuild with the real order,
/// every other fact the same; wrong when one frame differs. A model that
/// can't be rebuilt keeps the exact-match grade, and the row says why.
/// Changes `grade` in place; returns the row, or null when the order isn't
/// stated or is the real one.
fn grade_order(
    dir: &Path,
    real_order: &Toml,
    play: &Scorer,
    scratch: &Path,
    grade: &mut Value,
) -> Value {
    let stated = read_toml(&dir.join(gp_model::MODEL_FILE))
        .and_then(|m| m.get("model")?.get("systems").cloned());
    let Some(stated) = stated else {
        return Value::Null;
    };
    if &stated == real_order {
        return Value::Null;
    }
    let checked = (|| {
        let other = scratch.join("real-order");
        let _ = fs::remove_dir_all(&other);
        with_order(dir, real_order, &other)?;
        play.same_play(dir, &other)
    })();
    let (same, played) = match checked {
        Ok(x) => x,
        Err(e) => {
            return json!({"by": "exact match", "why": format!("the stated model can't be rebuilt yet: {}", format!("{e:#}").lines().next().unwrap_or(""))});
        }
    };
    if same {
        let f = &mut grade["facts"];
        if let Some(w) = f["wrong_paths"].as_array_mut() {
            w.retain(|p| p != ORDER_FACT);
        }
        if let Some(r) = f["right_paths"].as_array_mut() {
            r.push(json!(ORDER_FACT));
        }
        for (k, d) in [("wrong", -1i64), ("right", 1)] {
            let n = f[k].as_i64().unwrap_or(0) + d;
            f[k] = json!(n.max(0));
        }
    }
    json!({"by": "behaviour", "clips": played, "same_play": same,
        "grade": if same { "right" } else { "wrong" }})
}

/// A stage's folder from the one before: its model and mapping, and the
/// package's corpus beside them (template.toml, corpus.toml, inputs/), so
/// the stage's tools take it as the game.
fn copy_stage(from: &Path, to: &Path) -> Result<()> {
    copy_dir(from, to)
}

/// The sealed folder a pass's tools run in: the ROM, the asset map, and
/// the package's corpus (template.toml, corpus.toml, inputs/), and nothing
/// else: no model, no mapping, no hooks.
struct Sealed {
    dir: PathBuf,
    rom: PathBuf,
    asset_map: PathBuf,
    game: PathBuf,
}

fn seal(pass: &Path, rom: &Path, asset_map: Option<&Path>, package: &Path) -> Result<Sealed> {
    let dir = pass.join("sealed");
    fs::create_dir_all(dir.join("assets"))?;
    let rom_to = dir.join(rom.file_name().context("the ROM's name")?);
    fs::copy(rom, &rom_to)?;
    let map_to = match asset_map {
        Some(m) => {
            let to = dir
                .join("assets")
                .join(m.file_name().context("the asset map's name")?);
            fs::copy(m, &to)?;
            to
        }
        None => dir.join("assets"),
    };
    let game = dir.join("game");
    fs::create_dir_all(&game)?;
    for f in ["template.toml", "corpus.toml", "roms.toml", "package.toml"] {
        if package.join(f).is_file() {
            fs::copy(package.join(f), game.join(f))?;
        }
    }
    if package.join("inputs").is_dir() {
        copy_dir(&package.join("inputs"), &game.join("inputs"))?;
    }
    Ok(Sealed {
        dir,
        rom: rom_to,
        asset_map: map_to,
        game,
    })
}

/// A sandbox profile that lets a tool read everything but the games
/// checkout, the answer key and `$GP_HOME/work`, except its own pass folder
/// and the gp binary.
fn sandbox_profile(pass: &Path, denied: &[PathBuf], gp: &Path) -> Result<Option<PathBuf>> {
    if !cfg!(target_os = "macos") || !Path::new("/usr/bin/sandbox-exec").exists() {
        return Ok(None);
    }
    let q = |p: &Path| {
        format!(
            "\"{}\"",
            p.display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        )
    };
    // Only a file's contents are denied, not its metadata, so a tool can
    // still canonicalize its own pass folder under a denied $GP_HOME/work.
    let mut text = String::from("(version 1)\n(allow default)\n");
    for d in denied {
        if let Ok(c) = fs::canonicalize(d) {
            text.push_str(&format!("(deny file-read-data (subpath {}))\n", q(&c)));
        }
    }
    let pass = fs::canonicalize(pass)?;
    text.push_str(&format!("(allow file-read-data (subpath {}))\n", q(&pass)));
    text.push_str(&format!(
        "(allow file-read-data (literal {}))\n",
        q(&fs::canonicalize(gp)?)
    ));
    let file = pass.join("sandbox.sb");
    fs::write(&file, text)?;
    Ok(Some(file))
}

/// Scrambled copies of the answer key's files, where a tool that looked
/// for them from its folder would find them: the same paths under the
/// sealed folder, each file's lines in reverse.
fn plant_decoys(sealed: &Path, root: &Path, files: &[PathBuf]) -> Result<usize> {
    let mut n = 0;
    for f in files {
        let Ok(rel) = f.strip_prefix(root) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(f) else {
            continue;
        };
        let to = sealed.join(rel);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        let mut lines: Vec<&str> = text.lines().collect();
        lines.reverse();
        fs::write(&to, lines.join("\n"))?;
        n += 1;
    }
    Ok(n)
}

/// What one pass of the stages produced: each stage's folder, its tools'
/// rows and the hash of its output (files and rows, the pass folder's path
/// taken out).
struct Pass {
    stages: Vec<(PathBuf, Vec<Value>)>,
    extra: Value,
    hashes: Vec<String>,
    times: BTreeMap<&'static str, f64>,
    sealed: bool,
}

fn stage_hash(dir: &Path, rows: &[Value], pass: &Path) -> String {
    let mut bytes = Vec::new();
    for f in [gp_model::MODEL_FILE, gp_interp::mapping::MAPPING_FILE] {
        bytes.extend(fs::read(dir.join(f)).unwrap_or_default());
        bytes.push(0);
    }
    let rows = serde_json::to_string(rows)
        .unwrap_or_default()
        .replace(&pass.display().to_string(), "<work>");
    bytes.extend(rows.as_bytes());
    Hash::of(&bytes).to_string()
}

/// Runs the four stages' tools in `pass`, sealed, from the start model.
/// What every pass reads: the bench file, the ROM, the package, the start
/// and the grader (for the files the tools must not read).
struct Inputs<'a> {
    spec: &'a BenchFile,
    root: &'a Path,
    rom: &'a Path,
    package: &'a Path,
    asset_map: Option<&'a Path>,
    start_text: &'a str,
    grader: &'a Grader,
}

fn run_pass(pass: &Path, inp: &Inputs<'_>, decoys: bool) -> Result<Pass> {
    let Inputs {
        spec,
        root,
        rom,
        package,
        asset_map,
        start_text,
        grader,
    } = *inp;
    let mut times: BTreeMap<&'static str, f64> = BTreeMap::new();
    let mut clock = Instant::now();
    let mut lap = |name: &'static str, clock: &mut Instant| {
        times.insert(name, round3(clock.elapsed().as_secs_f64()));
        *clock = Instant::now();
    };
    fs::create_dir_all(pass)?;
    let sealed = seal(pass, rom, asset_map, package)?;
    if decoys {
        plant_decoys(&sealed.dir, root, grader.files())?;
    }
    let gp = std::env::current_exe().context("finding the gp binary")?;
    let gp_home = std::env::var_os("GP_HOME").map(PathBuf::from);
    let mut denied = vec![root.to_owned()];
    denied.extend(grader.files().iter().cloned());
    if let Some(h) = &gp_home {
        denied.push(h.join("work"));
    }
    let sandbox = sandbox_profile(pass, &denied, &gp)?;
    let tools = Tools {
        gp,
        home: pass.join("home"),
        root: sealed.dir.clone(),
        sandbox: sandbox.clone(),
        // The sealed folder stands for the games checkout, so a tool that
        // reads every package's roms.toml finds the sealed copy's only.
        env: vec![
            ("GP_ASSETS".into(), sealed.dir.join("assets")),
            ("GP_GAMES_DIR".into(), sealed.dir.clone()),
        ],
    };
    crate::bench_clips::home_with_rom(&tools.home, &sealed.rom)?;
    let start = pass.join("start");
    copy_dir(&sealed.game, &start)?;
    fs::write(start.join(gp_model::MODEL_FILE), start_text)?;
    let mut stages = Vec::new();

    // Stage 0: static harvest.
    let s0 = pass.join("stage0");
    copy_stage(&start, &s0)?;
    let mut s0_tools = vec![json!({"tool": "gp model derive --from code", "ok": false,
        "excluded": "a source reader: it reads the assembly source the evidence cites (crates/gp-derive/src/asm.rs), not the ROM"})];
    let mut open = json!("no stage 0 tool lists open facts; every fact the start lacks is open");
    if !spec.scratch.static_command.is_empty() {
        let model_arg = s(&s0);
        let rom_arg = s(&sealed.rom);
        let map_arg = s(&sealed.asset_map);
        let manifest_arg = s(&sealed.game.join("template.toml"));
        let argv: Vec<&str> = spec
            .scratch
            .static_command
            .iter()
            .map(|x| match x.as_str() {
                "{model}" => model_arg.as_str(),
                "{rom}" => rom_arg.as_str(),
                "{asset_map}" => map_arg.as_str(),
                "{manifest}" => manifest_arg.as_str(),
                _ => x.as_str(),
            })
            .collect();
        let r = tools.json(&argv);
        let mut row = ran(&r, pass);
        row["tool"] = json!(format!("gp {}", spec.scratch.static_command.join(" ")));
        if let Ok(out) = &r {
            let mut doc = read_toml(&s0.join(gp_model::MODEL_FILE))
                .unwrap_or(Toml::Table(Default::default()));
            let mut n = 0;
            for f in out["facts"].as_array().into_iter().flatten() {
                if matches!(f["status"].as_str(), Some("agrees" | "new"))
                    && let (Some(p), Some(v)) = (f["path"].as_str(), f.get("value"))
                    && let Ok(t) = toml::Value::try_from(v.clone())
                {
                    set_path(&mut doc, p, t);
                    n += 1;
                }
            }
            fs::write(s0.join(gp_model::MODEL_FILE), toml::to_string(&doc)?)?;
            row["facts_written"] = json!(n);
            if let Some(o) = out.get("open") {
                open = o.clone();
            }
        }
        s0_tools.push(row);
    }
    stages.push((s0.clone(), s0_tools));
    lap("static_harvest", &mut clock);

    // Stage 1: clip generation.
    let s1 = pass.join("stage1");
    copy_stage(&s0, &s1)?;
    let seed = spec.bench.seeds.first().copied().unwrap_or(1).to_string();
    let dense_dir = pass.join("dense");
    let dense = tools.json(&[
        "capture",
        "dense",
        &s(&s1),
        "--seed",
        &seed,
        "--budget",
        &spec.bench.dense_budget.to_string(),
        "--keep",
        &spec.bench.dense_keep.to_string(),
        "--into",
        &s(&dense_dir),
    ]);
    let mut s1_tools = vec![{
        let mut r = ran(&dense, pass);
        r["tool"] = json!("gp capture dense");
        r
    }];
    let recipes = dense
        .as_ref()
        .ok()
        .and_then(|d| d["clips"].as_array().cloned())
        .unwrap_or_default();
    let recipe_frames: u64 = recipes.iter().filter_map(|r| r["frames"].as_u64()).sum();
    s1_tools.push(json!({"tool": "gp corpus reduce", "ok": false,
        "excluded": "it measures the corpus the package holds; the clips it would keep write no fact"}));
    stages.push((s1.clone(), s1_tools));
    lap("clip_generation", &mut clock);

    // Stage 2: trace harvest, on recordings of the first corpus clips.
    let s2 = pass.join("stage2");
    copy_stage(&s1, &s2)?;
    let ids: Vec<String> = crate::clipset::read_game(&sealed.game)
        .map(|set| set.clips.into_iter().map(|c| c.id).collect())
        .unwrap_or_default();
    let logs = pass.join("logs");
    fs::create_dir_all(&logs)?;
    let mut recorded = Vec::new();
    let mut record_errors = BTreeSet::new();
    for id in ids.iter().take(spec.scratch.recordings) {
        let log = logs.join(format!("{id}.log"));
        let r = tools
            .gp(&["clip", "log", &s(&sealed.game), id, "--out", &s(&log)])
            .and_then(|_| {
                tools.json(&[
                    "record",
                    "--rom",
                    &s(&sealed.rom),
                    "--input",
                    &s(&log),
                    "--name",
                    id,
                ])
            });
        match r {
            Ok(v) => match v["id"].as_str() {
                Some(rid) => recorded.push(rid.to_owned()),
                None => {
                    record_errors.insert("gp record --json gave no id".to_owned());
                }
            },
            Err(e) => {
                if let Some(line) = ran(&Err::<(), _>(e), pass)["error"].as_str() {
                    record_errors.insert(line.to_owned());
                }
            }
        }
    }
    let mut args: Vec<String> = ["model", "propose", &s(&s2), "--no-questions", "--recording"]
        .iter()
        .map(|x| (*x).to_owned())
        .collect();
    args.extend(recorded.iter().cloned());
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let propose = if recorded.is_empty() {
        Err(anyhow::anyhow!("no recording of a corpus clip was made"))
    } else {
        tools.json(&argv)
    };
    let mut row = ran(&propose, pass);
    row["tool"] = json!("gp model propose");
    row["recordings"] = json!(recorded.len());
    row["recording_errors"] = json!(record_errors);
    let mut written_n = 0;
    if let Ok(p) = &propose {
        let mut doc =
            read_toml(&s2.join(gp_model::MODEL_FILE)).unwrap_or(Toml::Table(Default::default()));
        for x in p["proposals"].as_array().into_iter().flatten() {
            if x["settled"] == true
                && let (Some(path), Some(m)) = (x["path"].as_str(), x["measured"].as_str())
            {
                set_path(&mut doc, path, written(m));
                written_n += 1;
            }
        }
        fs::write(s2.join(gp_model::MODEL_FILE), toml::to_string(&doc)?)?;
    }
    row["facts_written"] = json!(written_n);
    stages.push((s2.clone(), vec![row, json!({"tool": "gp element inputs", "ok": false,
        "excluded": "it slices a scored clip's first wrong measure, which needs a state mapping; it writes no fact"})]));
    lap("trace_harvest", &mut clock);

    // Stage 3: refinement on the corpus.
    let s3 = pass.join("stage3");
    copy_stage(&s2, &s3)?;
    let mut rounds = Vec::new();
    for r in 1..=spec.scratch.rounds {
        let before = fs::read_to_string(s3.join(gp_model::MODEL_FILE)).unwrap_or_default();
        let solved = tools.json(&["solve", &s(&s3), "--run", "scratch", "--check"]);
        let mut fits = BTreeSet::new();
        let f = tools.gp_exit(&["model", "refine", &s(&s3), "--lockstep"], true);
        if let Some(e) = ran(&f, pass)["error"].as_str() {
            fits.insert(e.to_owned());
        }
        let after = fs::read_to_string(s3.join(gp_model::MODEL_FILE)).unwrap_or_default();
        rounds.push(json!({"round": r, "solve": ran(&solved, pass), "fit_errors": fits, "changed": after != before}));
        if after == before {
            break;
        }
    }
    stages.push((
        s3.clone(),
        vec![json!({"tool": "gp solve --check, gp model refine --lockstep", "rounds": rounds})],
    ));
    lap("clip_refinement", &mut clock);

    let hashes = stages
        .iter()
        .map(|(d, rows)| stage_hash(d, rows, pass))
        .collect();
    Ok(Pass {
        stages,
        extra: json!({"open": open, "recipes": recipes.len(), "recipe_frames": recipe_frames,
            "recordings": recorded.len()}),
        hashes,
        times,
        sealed: sandbox.is_some(),
    })
}

/// The grader's self-tests (the plan's null, oracle and wrong tools).
fn total_facts(real: &BTreeMap<String, Toml>) -> u64 {
    real.len() as u64
}

fn self_test(
    work: &Path,
    start_text: &str,
    grader: &Grader,
    play: &Scorer,
    real_facts: &BTreeMap<String, Toml>,
    real_elements: &BTreeSet<String>,
) -> Result<Value> {
    let (real, mapping) = grader.answer()?;
    // Null: the start alone.
    let null = work.join("selftest-null");
    fs::create_dir_all(&null)?;
    fs::write(null.join(gp_model::MODEL_FILE), start_text)?;
    let start_right = grade_dir(&null, real_facts, real_elements)["facts"]["right"]
        .as_u64()
        .unwrap_or(0);
    // Oracle: the real model and mapping.
    let oracle = work.join("selftest-oracle");
    fs::create_dir_all(&oracle)?;
    fs::write(oracle.join(gp_model::MODEL_FILE), toml::to_string(&real)?)?;
    if let Some(m) = &mapping {
        fs::write(
            oracle.join(gp_interp::mapping::MAPPING_FILE),
            toml::to_string(m)?,
        )?;
    }
    let og = grade_dir(&oracle, real_facts, real_elements);
    // Wrong: every integer of the real model one more.
    let mut changed = 0;
    let mut wrong_model = real.clone();
    bump_ints(&mut wrong_model, &mut changed);
    let wrong = work.join("selftest-wrong");
    fs::create_dir_all(&wrong)?;
    fs::write(
        wrong.join(gp_model::MODEL_FILE),
        toml::to_string(&wrong_model)?,
    )?;
    if let Some(m) = &mapping {
        fs::write(
            wrong.join(gp_interp::mapping::MAPPING_FILE),
            toml::to_string(m)?,
        )?;
    }
    let wg = grade_dir(&wrong, real_facts, real_elements);
    let total = real_facts.len() as u64;
    let n = |g: &Value, k: &str| g["facts"][k].as_u64().unwrap_or(0);
    let oracle_frames = play.exact(&oracle).map(|(f, _)| f).ok();
    // The bar: the frames the hand-written model plays exactly (game-
    // printer's Moss Hop model played every baseline frame).
    let frames = play.frames_real;
    // The order: each swap of two neighbouring systems in the real model,
    // graded by behaviour, must agree with the independent check against
    // the original (as many corpus frames exact as the real order plays).
    let mut order = json!({"swaps": 0, "right": 0, "wrong": 0, "disagree": [], "pass": true});
    if let Some(Toml::Array(sys)) = real.get("model").and_then(|m| m.get("systems")).cloned() {
        let real_order = Toml::Array(sys.clone());
        let (mut right, mut wrong, mut disagree) = (0, 0, Vec::new());
        for i in 0..sys.len().saturating_sub(1) {
            let mut sw = sys.clone();
            sw.swap(i, i + 1);
            let dir = work.join("selftest-order").join(format!("swap{i}"));
            let _ = fs::remove_dir_all(&dir);
            with_order(&oracle, &Toml::Array(sw), &dir)?;
            let mut g = json!({"facts": {"right": 0, "wrong": 1, "right_paths": [], "wrong_paths": [ORDER_FACT]}});
            let row = grade_order(
                &dir,
                &real_order,
                play,
                &work.join("selftest-order").join(format!("check{i}")),
                &mut g,
            );
            let graded_right = row["grade"] == "right";
            let exact = play.exact(&dir).map(|(f, _)| f).ok() == Some(frames);
            if graded_right {
                right += 1
            } else {
                wrong += 1
            }
            if row["by"] != "behaviour" || graded_right != exact {
                disagree.push(json!({"swap": i, "graded": row, "exact_against_original": exact}));
            }
        }
        order = json!({"swaps": right + wrong, "right": right, "wrong": wrong,
            "disagree": disagree, "pass": disagree.is_empty() && right + wrong > 0});
    }
    // Tile kinds: the real model with its kinds renamed (each to another
    // with the same core meaning) grades every kind fact right; with one
    // kind's head hit changed, that kind's tile value grades wrong.
    fn rename_kinds(v: &mut Toml, path: &str, f: &dyn Fn(&str) -> &'static str) {
        match v {
            Toml::Table(t) => {
                for (k, x) in t.iter_mut() {
                    rename_kinds(x, &format!("{path}.{k}"), f);
                }
            }
            Toml::Array(a) => {
                for (i, x) in a.iter_mut().enumerate() {
                    rename_kinds(x, &format!("{path}[{i}]"), f);
                }
            }
            Toml::String(s) if is_kind_fact(path.trim_start_matches('.')) => {
                *s = f(s).to_owned();
            }
            _ => {}
        }
    }
    // Each kind to another with the same core meaning; the rest unchanged.
    fn static_rename(k: &str) -> &'static str {
        match k {
            "coin_block" => "item_block",
            "item_block" => "brick",
            "brick" => "coin_block",
            "coin" => "water",
            "water" => "coin",
            "solid" => "solid",
            "top" => "top",
            "ice" => "ice",
            "ladder" => "ladder",
            "hazard" => "hazard",
            _ => "empty",
        }
    }
    let kinds = if tile_kinds(&real).is_empty() {
        json!({"pass": true, "skipped": "the real model has no tile kinds"})
    } else {
        let mut renamed = real.clone();
        rename_kinds(&mut renamed, "", &static_rename);
        let dir = work.join("selftest-kinds-renamed");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(gp_model::MODEL_FILE), toml::to_string(&renamed)?)?;
        if let Some(m) = &mapping {
            fs::write(
                dir.join(gp_interp::mapping::MAPPING_FILE),
                toml::to_string(m)?,
            )?;
        }
        let mut g = grade_dir(&dir, real_facts, real_elements);
        let row = grade_kinds(&dir, &real, &mut g);
        let renamed_ok = g["facts"]["wrong"].as_u64() == Some(0)
            && g["facts"]["right"].as_u64() == Some(total_facts(real_facts));
        // One kind's head hit made to leave the cell empty instead.
        let mut changed = real.clone();
        let mut target = None;
        if let Some(es) = changed.get_mut("entities").and_then(Toml::as_table_mut) {
            for (_, e) in es.iter_mut() {
                let hits = e
                    .get_mut("jump")
                    .and_then(|j| j.get_mut("head"))
                    .and_then(|h| h.get_mut("hits"))
                    .and_then(Toml::as_array_mut);
                for h in hits.into_iter().flatten() {
                    if target.is_none()
                        && let Some(t) = h.as_table_mut()
                    {
                        target = t.get("tile").and_then(Toml::as_str).map(str::to_owned);
                        let now = t.get("becomes").and_then(Toml::as_str).map(kind_stops);
                        let to = if now == Some("none") {
                            "solid"
                        } else {
                            "empty"
                        };
                        t.insert("becomes".into(), Toml::String(to.into()));
                    }
                }
            }
        }
        let changed_ok = match &target {
            None => true,
            Some(t) => {
                rename_kinds(&mut changed, "", &static_rename);
                let dir = work.join("selftest-kinds-changed");
                fs::create_dir_all(&dir)?;
                fs::write(dir.join(gp_model::MODEL_FILE), toml::to_string(&changed)?)?;
                if let Some(m) = &mapping {
                    fs::write(
                        dir.join(gp_interp::mapping::MAPPING_FILE),
                        toml::to_string(m)?,
                    )?;
                }
                let mut g = grade_dir(&dir, real_facts, real_elements);
                let r = grade_kinds(&dir, &real, &mut g);
                let wrong: BTreeSet<String> = g["facts"]["wrong_paths"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect();
                let values: Vec<String> = tile_kinds(&real)
                    .into_iter()
                    .filter(|(_, k)| k == t)
                    .map(|((l, v), _)| format!("levels.{l}.tile_kinds.{v}"))
                    .collect();
                !values.is_empty()
                    && values.iter().all(|p| wrong.contains(p))
                    && r["differ"].as_array().is_some_and(|d| !d.is_empty())
            }
        };
        json!({"renamed": {"row": row, "pass": renamed_ok}, "changed_head_hit": {"kind": target, "pass": changed_ok},
            "pass": renamed_ok && changed_ok})
    };
    let checks = json!({
        "kinds": kinds,
        "order": order,
        "null": {"right_by_tools": 0, "right_in_start": start_right, "pass": true},
        "oracle": {"right": n(&og, "right"), "of": total, "frames_exact": oracle_frames, "frames": frames, "corpus_frames": play.frames,
            "pass": n(&og, "right") == total && og["elements"]["found"] == og["elements"]["real"] && oracle_frames == Some(frames)},
        "wrong": {"changed": changed, "wrong": n(&wg, "wrong"), "pass": n(&wg, "wrong") == changed && n(&wg, "right") == total - changed},
    });
    let pass = ["null", "oracle", "wrong", "order", "kinds"]
        .iter()
        .all(|k| checks[*k]["pass"] == true);
    anyhow::ensure!(pass, "the grader's self-test failed: {checks}");
    Ok(checks)
}

fn bump_ints(v: &mut Toml, n: &mut u64) {
    match v {
        Toml::Table(t) => {
            for (k, x) in t.iter_mut() {
                if k == "evidence" || k == "schema" {
                    continue;
                }
                bump_ints(x, n);
            }
        }
        Toml::Array(a) if a.iter().any(Toml::is_table) => {
            a.iter_mut().for_each(|x| bump_ints(x, n))
        }
        Toml::Integer(i) => {
            *i += 1;
            *n += 1;
        }
        _ => {}
    }
}

/// The empty start: no entities, levels, rules, counters, cameras or
/// assets, and no number but the header's.
fn check_empty_start(text: &str) -> Result<()> {
    let v: Toml = toml::from_str(text)?;
    let keys: Vec<&String> = v.as_table().map(|t| t.keys().collect()).unwrap_or_default();
    anyhow::ensure!(
        keys.iter().all(|k| *k == "model"),
        "the start holds more than its header: {keys:?}"
    );
    anyhow::ensure!(elements(&v).is_empty(), "the start names elements");
    Ok(())
}

pub fn scratch(paths: &Paths, a: &ScratchArgs) -> Result<Value> {
    let root = crate::bench_clips::root_of(a.root.as_deref())?;
    let (spec_file, game) = crate::bench_clips::bench_file(&a.game)?;
    let spec: BenchFile = toml::from_str(
        &fs::read_to_string(&spec_file)
            .with_context(|| format!("reading {}", spec_file.display()))?,
    )
    .with_context(|| spec_file.display().to_string())?;
    let grader = Grader::load(&root, &spec_file)?;
    check_static(&spec.scratch.static_command, grader.files())?;
    let package = spec.bench.package(&root);
    let rom = crate::bench_clips::rom_of(a.rom.as_deref(), &spec.bench, &root, &package, paths)?;
    let rom_bytes = fs::read(&rom).with_context(|| format!("reading {}", rom.display()))?;
    let arcade =
        crate::original::Platform::of_file(&rom_bytes) == crate::original::Platform::Arcade;
    let asset_map = match &spec.scratch.asset_map {
        Some(m) => Some(root.join(m)),
        None => read_toml(&package.join("template.toml"))
            .and_then(|t| {
                t.get("recipe")?
                    .get("asset_map")?
                    .as_str()
                    .map(str::to_owned)
            })
            .map(|m| package.join(m))
            .filter(|p| p.is_file()),
    };
    let work = a.work.clone().unwrap_or_else(|| {
        paths
            .home
            .join("work")
            .join("bench")
            .join(format!("{game}-scratch"))
    });
    let work = crate::bench_clips::fresh_work(&work, "gp bench scratch")?;

    // The answer key, read by the grader only.
    let (real, real_mapping) = grader.answer()?;
    let real_facts = facts_of(&real, real_mapping.as_ref());
    let real_elements = elements(&real);
    // The grader's scoring: the real package's corpus, in a home of its
    // own, scored on the real model for every clip's frames.
    let grade_tools = Tools {
        gp: std::env::current_exe().context("finding the gp binary")?,
        home: work.join("grade-home"),
        root: work.clone(),
        sandbox: None,
        env: vec![("GP_GAMES_DIR".into(), root.clone())],
    };
    crate::bench_clips::home_with_rom(&grade_tools.home, &rom)?;
    let mut play = Scorer {
        tools: grade_tools,
        game: grader.model_dir().to_owned(),
        frames: 0,
        frames_real: 0,
        fps: if arcade { 30.0 } else { 60.0988 },
    };
    let real_score = play.score(grader.model_dir())?;
    let real_rows = score_rows(&real_score);
    play.frames = real_rows
        .values()
        .filter_map(|r| r["frames"].as_u64())
        .sum();
    play.frames_real = real_rows
        .values()
        .filter_map(|r| r["frames_together"].as_u64())
        .sum();
    let start_text = start_model(&real)?;
    check_empty_start(&start_text)?;

    let inputs = Inputs {
        spec: &spec,
        root: &root,
        rom: &rom,
        package: &package,
        asset_map: asset_map.as_deref(),
        start_text: &start_text,
        grader: &grader,
    };
    let pass = run_pass(&work.join("pass"), &inputs, false)?;
    let scramble = if a.scramble {
        let p2 = run_pass(&work.join("scrambled"), &inputs, true)?;
        let same: Vec<bool> = pass
            .hashes
            .iter()
            .zip(&p2.hashes)
            .map(|(x, y)| x == y)
            .collect();
        anyhow::ensure!(
            same.iter().all(|x| *x),
            "the scramble test failed: a stage's output changed with scrambled source, labels and answer key ({same:?})"
        );
        json!({"identical": true, "stages": same.len()})
    } else {
        Value::Null
    };
    let self_test = if a.self_test {
        Some(self_test(
            &work,
            &start_text,
            &grader,
            &play,
            &real_facts,
            &real_elements,
        )?)
    } else {
        None
    };

    // The grade, after the tools ran.
    let start_grade = grade_dir(
        &work.join("pass").join("start"),
        &real_facts,
        &real_elements,
    );
    let real_order = real_facts.get(ORDER_FACT).cloned();
    let mut order_rows = Vec::new();
    let grades: Vec<Value> = pass
        .stages
        .iter()
        .enumerate()
        .map(|(i, (d, _))| {
            let mut g = grade_dir(d, &real_facts, &real_elements);
            if let Some(real) = &real_order {
                let row = grade_order(
                    d,
                    real,
                    &play,
                    &work.join("order-check").join(format!("stage{i}")),
                    &mut g,
                );
                g["order"] = row.clone();
                order_rows.push(row);
            }
            if let Ok((real_model, _)) = grader.answer() {
                g["kinds"] = grade_kinds(d, &real_model, &mut g);
            }
            g
        })
        .collect();
    let start_right: BTreeSet<String> = start_grade["facts"]["right_paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_owned))
        .collect();
    let by_tools = |g: &Value| -> u64 {
        g["facts"]["right_paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|p| !start_right.contains(*p))
            .count() as u64
    };
    let total = real_facts.len() as u64;
    let x = &pass.extra;
    let frames = play.frames;
    let exact = play.exact(&pass.stages[3].0);
    let (frames_exact, exact_names) = exact
        .as_ref()
        .map(|(f, n)| (*f, n.clone()))
        .unwrap_or_default();
    let tools_of = |i: usize| json!(pass.stages[i].1);
    let mut v = json!({
        "schema": SCRATCH_SCHEMA,
        "bench": spec_file.display().to_string(),
        "guards": {"sealed": pass.sealed, "sealed_how": if pass.sealed { "an empty folder with the ROM, the asset map and the package's corpus (template.toml, corpus.toml, inputs/), under sandbox-exec denying the games checkout, the answer key and $GP_HOME/work" } else { "an empty folder with the ROM, the asset map and the package's corpus; no sandbox on this platform" },
            "scramble": scramble, "self_test": self_test, "empty_start": true},
        "start": {"holds": "the header the schema needs: schema, name, title, source, view and frame rate", "grade": start_grade,
            "right_from_start_alone": start_right},
        "stages": [
            {"stage": "0. Static harvest", "tools": tools_of(0), "grade": grades[0], "open": x["open"],
             "by_tools": {"right": by_tools(&grades[0]), "of": total, "share": round3(by_tools(&grades[0]) as f64 / total.max(1) as f64)}},
            {"stage": "1. Clip generation", "tools": tools_of(1),
             "with_hand_model": a.clips.as_ref().and_then(|f| fs::read_to_string(f).ok())
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .map(|c| json!({"measures": c["measures"], "result": c["result_blake3"]})),
             "recipes": x["recipes"], "recipe_frames": x["recipe_frames"],
             "facts_settled_by_recipes": "none credited: no tool ties a recipe's trace to a model fact",
             "grade": grades[1]},
            {"stage": "2. Trace harvest", "tools": tools_of(2), "grade": grades[2],
             "by_tools": {"right": by_tools(&grades[2]), "of": total}},
            {"stage": "3. Refinement", "rounds": pass.stages[3].1[0]["rounds"], "grade": grades[3],
             "exact": ran(&exact, &work), "clips_exact": exact_names, "clips": score_rows(&real_score).len(),
             "by_tools": {"right": by_tools(&grades[3]), "of": total}},
        ],
        "exact": {"frames": frames, "frames_exact": frames_exact,
            "seconds": round3(frames as f64 / play.fps), "seconds_exact": round3(frames_exact as f64 / play.fps)},
        "facts_right": {"by_tools": by_tools(&grades[3]), "at_end": grades[3]["facts"]["right"], "wrong": grades[3]["facts"]["wrong"],
            "from_start": start_right.len(), "of": total},
        "stage_blake3": pass.hashes,
    });
    let hash = Hash::of(serde_json::to_string(&v)?.as_bytes()).to_string();
    v["result_blake3"] = json!(hash);
    let mut times = pass.times.clone();
    times.insert("builds", 0.0);
    let total_s: f64 = [
        "static_harvest",
        "clip_generation",
        "trace_harvest",
        "clip_refinement",
    ]
    .iter()
    .filter_map(|k| times.get(k))
    .sum();
    times.insert("total", round3(total_s));
    let exact_s = frames_exact as f64 / play.fps;
    v["target"] = json!({
        "minutes_per_exact_second": if exact_s > 0.0 { json!(round3(total_s / 60.0 / exact_s)) } else { Value::Null },
        "goal": 1.0,
        "builds": "none: the run builds nothing, gp was built before it",
    });
    v["seconds"] = json!(times);
    v["work"] = json!(work.display().to_string());
    fs::write(
        work.join("report.json"),
        serde_json::to_string_pretty(&v)? + "\n",
    )?;
    Ok(v)
}

pub(crate) fn text_of(v: &Value) -> String {
    let st = &v["stages"];
    let sec = &v["seconds"];
    let x = &v["exact"];
    let min = |k: &str| sec[k].as_f64().unwrap_or(0.0) / 60.0;
    let g = |i: usize| &st[i]["grade"];
    let facts = |g: &Value| {
        format!(
            "{} right, {} wrong, {} missing of {}",
            g["facts"]["right"], g["facts"]["wrong"], g["facts"]["missing"], g["facts"]["total"]
        )
    };
    let els = |g: &Value| {
        format!(
            "{} of {} (named {}, precision {})",
            g["elements"]["found"],
            g["elements"]["real"],
            g["elements"]["named"],
            g["elements"]["precision"]
                .as_f64()
                .map_or("none named".into(), |p| p.to_string())
        )
    };
    let tools = |i: usize| -> String {
        st[i]["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                let name = t["tool"].as_str().unwrap_or("");
                match (t["excluded"].as_str(), t["error"].as_str(), t["ok"] == true) {
                    (Some(why), _, _) => format!("{name}: excluded, {why}"),
                    (_, Some(e), _) => format!("{name}: failed, {e}"),
                    (_, _, true) => format!("{name}: ran"),
                    _ => format!("{name}: didn't run"),
                }
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    let mut out = vec![
        format!("{}: from scratch", v["bench"].as_str().unwrap_or("")),
        String::new(),
        "| The target | Value |".into(),
        "|---|---|".into(),
        format!(
            "| Minutes per exact second (goal 1 or less) | {} |",
            match v["target"]["minutes_per_exact_second"].as_f64() {
                Some(m) => format!("{m:.1}"),
                None => "none: no second plays exactly".into(),
            }
        ),
        format!(
            "| Wall time | {:.1} min: static harvest {:.1}, clip generation {:.1}, trace harvest {:.1}, refinement {:.1}, builds 0 |",
            min("total"),
            min("static_harvest"),
            min("clip_generation"),
            min("trace_harvest"),
            min("clip_refinement")
        ),
        format!(
            "| Exact seconds | {} of {} s ({} of {} frames) |",
            x["seconds_exact"], x["seconds"], x["frames_exact"], x["frames"]
        ),
        format!(
            "| Facts right | {} of {} by tools; {} were in the start (its header) |",
            v["facts_right"]["by_tools"], v["facts_right"]["of"], v["facts_right"]["from_start"]
        ),
        {
            let g = &v["guards"];
            let st = &g["self_test"];
            format!(
                "| Guards | sealed: {}; scramble: {}; grader self-test: {} |",
                if g["sealed"] == true {
                    "yes, under sandbox-exec"
                } else {
                    "folder only, no sandbox here"
                },
                if g["scramble"]["identical"] == true {
                    "every stage's output the same"
                } else {
                    "not run"
                },
                if st.is_null() {
                    "not run".to_owned()
                } else {
                    format!(
                        "null {} right, oracle {} of {} right and {} of {} frames exact, wrong {} of {} changes wrong",
                        st["null"]["right_by_tools"],
                        st["oracle"]["right"],
                        st["oracle"]["of"],
                        st["oracle"]["frames_exact"],
                        st["oracle"]["frames"],
                        st["wrong"]["wrong"],
                        st["wrong"]["changed"]
                    )
                }
            )
        },
        String::new(),
        "| Stage | Measure | Value |".into(),
        "|---|---|---|".into(),
        format!("| Start | Facts | {} |", facts(&v["start"]["grade"])),
        format!("| 0. Static harvest | Tools | {} |", tools(0)),
        format!("| 0. Static harvest | Elements found | {} |", els(g(0))),
        format!("| 0. Static harvest | Facts | {} |", facts(g(0))),
        format!(
            "| 0. Static harvest | Built statically by tools | {} of {} ({}) |",
            st[0]["by_tools"]["right"], st[0]["by_tools"]["of"], st[0]["by_tools"]["share"]
        ),
        format!(
            "| 0. Static harvest | Open facts | {} |",
            st[0]["open"]
                .as_str()
                .map_or_else(|| st[0]["open"].to_string(), str::to_owned)
        ),
        format!("| 1. Clip generation | Tools | {} |", tools(1)),
        format!(
            "| 1. Clip generation | Recipes and frames | {} recipes, {} frames; {} |",
            st[1]["recipes"],
            st[1]["recipe_frames"],
            st[1]["facts_settled_by_recipes"].as_str().unwrap_or("")
        ),
        format!("| 2. Trace harvest | Tools | {} |", tools(2)),
        format!("| 2. Trace harvest | Facts | {} |", facts(g(2))),
        format!(
            "| 3. Refinement | Rounds | {} |",
            st[3]["rounds"].as_array().map_or(0, Vec::len)
        ),
        format!(
            "| 3. Refinement | Clips exact | {}; {} |",
            // game-printer's reports counted its 8 baselines.
            match st[3]["clips"].as_u64() {
                Some(n) => format!(
                    "{} of {n} corpus clips",
                    st[3]["clips_exact"].as_array().map_or(0, Vec::len)
                ),
                None => format!(
                    "{} of 8 baselines",
                    st[3]["baselines_exact"].as_array().map_or(0, Vec::len)
                ),
            },
            st[3]["exact"]["error"].as_str().unwrap_or("compared")
        ),
        format!("| 3. Refinement | Facts | {} |", facts(g(3))),
        String::new(),
    ];
    for m in st[1]["with_hand_model"]["measures"]
        .as_array()
        .into_iter()
        .flatten()
    {
        out.push(format!(
            "| 1. Clip generation, on the hand-written model and mapping (gp bench clips) | {} | {} |",
            m["measure"].as_str().unwrap_or(""),
            m["value"].as_str().unwrap_or("")
        ));
    }
    for (i, h) in v["stage_blake3"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        out.push(format!("stage {i}: {}", h.as_str().unwrap_or("")));
    }
    out.push(format!(
        "result {}; work in {}",
        v["result_blake3"].as_str().unwrap_or(""),
        v["work"].as_str().unwrap_or("")
    ));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_0_refuses_a_source_reader_or_a_labels_file() {
        let none: Vec<PathBuf> = Vec::new();
        let cmd = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert!(
            check_static(
                &cmd(&["model", "derive", "{model}", "--from", "code"]),
                &none
            )
            .is_err()
        );
        assert!(
            check_static(
                &cmd(&[
                    "model",
                    "static",
                    "{model}",
                    "--labels",
                    "games/mosshop/labels.toml"
                ]),
                &none
            )
            .is_err()
        );
        assert!(
            check_static(
                &cmd(&["model", "static", "{model}", "games/mosshop/mosshop.s"]),
                &none
            )
            .is_err()
        );
        assert!(check_static(&cmd(&["capture", "dense", "r", "--with-source"]), &none).is_err());
        assert!(check_static(&cmd(&["model", "static", "{model}"]), &none).is_ok());
    }
}
