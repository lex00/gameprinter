//! `gp bench clips`: Phase 1 of docs/plan-extraction-process.md. It runs
//! the ROM-only clip tools on a game in a home of its own and grades what
//! they find against the game's answer key, which only the grader reads.
//!
//! Ported from game-printer, whose stages were its capture-run loop. Each
//! is gameprinter's piece now:
//!
//! | game-printer's stage | Here, in the bench's home |
//! |---|---|
//! | The run: `gp capture start`, `grid --mode batch`, `generate --start`, `background` | The corpus: `gp corpus score GAME --run real`, every clip made again from the ROM and scored on the hand-written model |
//! | The code matrix: `gp clips matrix --labels rom`, once per seed | `gp corpus reduce GAME` (the routines and branch ways the corpus runs, the untaken ways and the fewest clips that run them all) and `gp capture dense GAME --seed S` once per seed (clips the code chooses) |
//! | The grid: `gp capture grid --code`, then `--drive N` | The dense clips of each seed stand for the driven clips |
//! | A wrong model: the same run on the template with one fact changed | The corpus scored on the hand-written model with the bench file's one change (`--reuse` of the real score) |
//! | Mutants: `gp capture prune --mutants` | `gp model mutants`' mutants (made here by the same code), each scored on the corpus by `gp corpus score --model M --reuse`: a clip the real model played exactly stops at its first split, as prune's fast way did |
//!
//! The grade keeps the plan's measures table. Rows game-printer graded
//! against Moss Hop's source ground truth (elements found, unreachable
//! directions told apart, inputs found) say they need one; Donkey Kong and
//! Tapper have no source. The report is the same for the same files, ROM
//! and build; its hash leaves out the wall times.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use gp_evidence::{Hash, Paths};
use serde::Deserialize;
use serde_json::{Value, json};
use toml::Value as Toml;

use crate::Outcome;
use crate::bench_grade::{Grader, jaccard, round3};
use crate::mutants::Mutant;

/// The JSON shape of `gp bench clips`. A change to it bumps this.
pub const CLIPS_SCHEMA: &str = "gp bench clips 2";

#[derive(Subcommand)]
pub enum BenchCmd {
    /// Grade the ROM-only clip tools on a game whose answer key is known
    /// (docs/benchmarks.md). The tools run in a home of their own (--work)
    /// on the hand-written model and mapping, over the game's corpus. Only
    /// the grader reads the answer key. The report holds the measures of
    /// docs/plan-extraction-process.md and the mutants each kind of wrong
    /// model leaves uncaught. Its hash leaves out the times, so a rerun
    /// gives the same hash.
    Clips(ClipsArgs),
    /// Run every stage from scratch and grade it (docs/benchmarks.md). The
    /// start holds only the header the schema needs. Each stage's tools
    /// run on what the stage before left, and their output is graded
    /// against the real model and mapping. The report leads with the
    /// minutes of run time per second of gameplay played exactly. Stage 0
    /// refuses a tool that reads the source or the labels. Each stage's
    /// output has a hash of its own, for telling which stage a change
    /// moved.
    Scratch(crate::bench_scratch::ScratchArgs),
    /// Print the measures tables of the last `gp bench scratch` and `gp
    /// bench clips` runs of a game, from their report.json files. Status
    /// reports quote this table (docs/plan-extraction-process.md, "Guards").
    /// Exits 2 when neither report exists.
    Report(ReportArgs),
}

#[derive(Args, Clone)]
pub struct ReportArgs {
    /// The game whose reports to read.
    pub game: String,
    /// The `gp bench clips` report (default:
    /// $GP_HOME/work/bench/GAME/report.json).
    #[arg(long)]
    pub clips: Option<PathBuf>,
    /// The `gp bench scratch` report (default:
    /// $GP_HOME/work/bench/GAME-scratch/report.json).
    #[arg(long)]
    pub scratch: Option<PathBuf>,
}

#[derive(Args, Clone)]
pub struct ClipsArgs {
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
    /// $GP_HOME/work/bench/GAME). A folder that isn't a bench's is refused.
    #[arg(long)]
    pub work: Option<PathBuf>,
    /// Run only the first N mutants (by name), for a quick look; the
    /// report says so.
    #[arg(long)]
    pub mutants: Option<usize>,
}

pub(crate) fn run(cmd: &BenchCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        BenchCmd::Clips(a) => {
            let v = clips(paths, a)?;
            Ok(Outcome::ok(v.clone(), text_of(&v)))
        }
        BenchCmd::Scratch(a) => {
            let v = crate::bench_scratch::scratch(paths, a)?;
            Ok(Outcome::ok(v.clone(), crate::bench_scratch::text_of(&v)))
        }
        BenchCmd::Report(a) => Ok(report(paths, a)),
    }
}

/// `gp bench report`: the last reports' tables.
pub(crate) fn report(paths: &Paths, a: &ReportArgs) -> Outcome {
    let bench = paths.home.join("work").join("bench");
    let read = |p: PathBuf| -> Option<(PathBuf, Value)> {
        let v = serde_json::from_str(&fs::read_to_string(&p).ok()?).ok()?;
        Some((p, v))
    };
    let scratch = read(a.scratch.clone().unwrap_or_else(|| {
        bench
            .join(format!("{}-scratch", a.game))
            .join("report.json")
    }));
    let clips = read(
        a.clips
            .clone()
            .unwrap_or_else(|| bench.join(&a.game).join("report.json")),
    );
    let mut text = Vec::new();
    if let Some((p, v)) = &scratch {
        text.push(format!("From {}", p.display()));
        text.push(crate::bench_scratch::text_of(v));
    }
    if let Some((p, v)) = &clips {
        text.push(format!("From {}", p.display()));
        text.push(text_of(v));
    }
    let found = scratch.is_some() || clips.is_some();
    if !found {
        text.push(format!(
            "no bench report for {} under {}; run gp bench scratch or gp bench clips",
            a.game,
            bench.display()
        ));
    }
    Outcome {
        json: json!({"scratch": scratch.map(|x| x.1), "clips": clips.map(|x| x.1)}),
        text: text.join("\n\n"),
        exit: if found { 0 } else { 2 },
    }
}

// The bench file.

#[derive(Deserialize)]
struct BenchFile {
    bench: Head,
    wrong_model: WrongModel,
}

/// The bench file's `[bench]`. game-printer's `baselines`, `search_rows`,
/// `generate_steps`, `background_steps` and `drive` fed its capture run;
/// they are read and not used.
#[derive(Deserialize, Clone)]
pub(crate) struct Head {
    /// The game's package folder, under the root.
    pub(crate) template: PathBuf,
    /// The ROM, under the root; default: the one the package accepts.
    #[serde(default)]
    pub(crate) rom: Option<PathBuf>,
    pub(crate) seeds: Vec<u64>,
    /// `gp capture dense`'s press sequences a seed.
    #[serde(default = "dense_budget")]
    pub(crate) dense_budget: u64,
    /// `gp capture dense`'s clips to choose.
    #[serde(default = "dense_keep")]
    pub(crate) dense_keep: u64,
    #[serde(default)]
    #[allow(dead_code)]
    baselines: Option<PathBuf>,
    #[serde(default)]
    #[allow(dead_code)]
    search_rows: Option<usize>,
    #[serde(default)]
    #[allow(dead_code)]
    generate_steps: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    background_steps: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    drive: Option<u64>,
}

impl Head {
    /// The game's package folder. `gp bench scratch` takes only its corpus
    /// (template.toml, corpus.toml, inputs/) into the sealed folder, never
    /// its model, mapping or hooks.
    pub(crate) fn package(&self, root: &Path) -> PathBuf {
        root.join(&self.template)
    }
}

fn dense_budget() -> u64 {
    400
}

fn dense_keep() -> u64 {
    15
}

#[derive(Deserialize)]
struct WrongModel {
    says: String,
    set: BTreeMap<String, Toml>,
}

/// The bench file and the game's name: GAME is `bench/GAME.toml` in the
/// gameprinter repo (here, else the build's), or a bench file's path.
pub(crate) fn bench_file(game: &str) -> Result<(PathBuf, String)> {
    if game.ends_with(".toml") {
        let f = fs::canonicalize(game).with_context(|| format!("reading {game}"))?;
        let g = f
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Ok((f, g));
    }
    let here = PathBuf::from("bench").join(format!("{game}.toml"));
    let f = if here.is_file() {
        here
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../bench")
            .join(format!("{game}.toml"))
    };
    Ok((f, game.to_owned()))
}

/// The root a bench file's paths are under: `--root`, else the games
/// checkout.
pub(crate) fn root_of(given: Option<&Path>) -> Result<PathBuf> {
    let r = match given {
        Some(r) => r.to_owned(),
        None => std::env::var_os("GP_GAMES_DIR").map_or_else(
            || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games"),
            PathBuf::from,
        ),
    };
    fs::canonicalize(&r).with_context(|| format!("the bench's root {}", r.display()))
}

/// The ROM: given, the bench file's, or the one the package accepts.
pub(crate) fn rom_of(
    given: Option<&Path>,
    head: &Head,
    root: &Path,
    game: &Path,
    paths: &Paths,
) -> Result<PathBuf> {
    if let Some(r) = given {
        return Ok(fs::canonicalize(r)?);
    }
    if let Some(r) = &head.rom {
        return Ok(root.join(r));
    }
    let t: Toml = toml::from_str(
        &fs::read_to_string(game.join("template.toml"))
            .with_context(|| format!("reading {}", game.join("template.toml").display()))?,
    )?;
    crate::model_static::template_rom(game, &t, paths)
}

/// A fresh bench folder: one that is a bench's is cleared, any other
/// refused.
pub(crate) fn fresh_work(work: &Path, what: &str) -> Result<PathBuf> {
    let marker = work.join("bench.marker");
    if work.exists() {
        if !marker.exists() {
            bail!(
                "{} exists and isn't a bench's folder; give another --work",
                work.display()
            );
        }
        fs::remove_dir_all(work).with_context(|| format!("clearing {}", work.display()))?;
    }
    fs::create_dir_all(work)?;
    fs::write(&marker, format!("{what}\n"))?;
    Ok(fs::canonicalize(work)?)
}

// The tools, run in the bench's home.

#[derive(Clone)]
pub(crate) struct Tools {
    pub(crate) gp: PathBuf,
    pub(crate) home: PathBuf,
    /// The folder each tool runs in.
    pub(crate) root: PathBuf,
    /// A macOS sandbox profile each tool runs under (`sandbox-exec -f`).
    pub(crate) sandbox: Option<PathBuf>,
    /// More environment for each tool.
    pub(crate) env: Vec<(String, PathBuf)>,
}

impl Tools {
    /// Runs `gp ARGS` in the bench's home and returns its stdout.
    pub(crate) fn gp(&self, args: &[&str]) -> Result<String> {
        self.gp_exit(args, false).map(|(o, _)| o)
    }

    /// [`Self::gp`], and with `allow_1` an exit of 1 (a stage that failed,
    /// which is a finding, not a broken bench) is returned as `true`.
    pub(crate) fn gp_exit(&self, args: &[&str], allow_1: bool) -> Result<(String, bool)> {
        let mut cmd = match &self.sandbox {
            Some(profile) => {
                let mut c = Command::new("/usr/bin/sandbox-exec");
                c.arg("-f").arg(profile).arg(&self.gp);
                c
            }
            None => Command::new(&self.gp),
        };
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        // The benches measure the ROM-only tools, with no LLM or agent
        // (docs/plan-extraction-process.md).
        cmd.env("GP_REQUEST_AGENT", "off")
            .env("GP_CLIP_AGENT", "off")
            .env_remove("GP_WORKER");
        let out = cmd
            .args(args)
            .current_dir(&self.root)
            .env("GP_HOME", &self.home)
            .env("GP_STORE", self.home.join("store"))
            .env("GP_ROM_DIR", self.home.join("roms"))
            .output()
            .with_context(|| format!("running gp {}", args.join(" ")))?;
        if allow_1 && out.status.code() == Some(1) {
            return Ok((String::from_utf8_lossy(&out.stdout).into_owned(), true));
        }
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<&str> = err.lines().rev().take(8).collect();
            bail!(
                "gp {} failed ({}):\n{}",
                args.join(" "),
                out.status,
                tail.into_iter().rev().collect::<Vec<_>>().join("\n")
            );
        }
        Ok((String::from_utf8_lossy(&out.stdout).into_owned(), false))
    }

    pub(crate) fn json(&self, args: &[&str]) -> Result<Value> {
        self.json_exit(args, false).map(|(v, _)| v)
    }

    /// [`Self::json`], an exit of 1 allowed when `allow_1`.
    pub(crate) fn json_exit(&self, args: &[&str], allow_1: bool) -> Result<(Value, bool)> {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let (text, failed) = self.gp_exit(&all, allow_1)?;
        let v = serde_json::from_str(&text)
            .with_context(|| format!("gp {}: not JSON", args.join(" ")))?;
        Ok((v, failed))
    }
}

pub(crate) fn s(p: &Path) -> String {
    p.display().to_string()
}

/// Copies a package's files (not a `target/` build folder).
pub(crate) fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    let mut entries: Vec<_> = fs::read_dir(from)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let dest = to.join(e.file_name());
        if p.is_dir() {
            if e.file_name() != "target" {
                copy_dir(&p, &dest)?;
            }
        } else {
            fs::copy(&p, &dest)?;
        }
    }
    Ok(())
}

/// A bench's home with the ROM in its ROM folder, so the tools find it by
/// its hash.
pub(crate) fn home_with_rom(home: &Path, rom: &Path) -> Result<()> {
    let roms = home.join("roms");
    fs::create_dir_all(&roms)?;
    fs::copy(
        rom,
        roms.join(rom.file_name().context("the ROM's file name")?),
    )?;
    Ok(())
}

/// Each clip's row of a `gp corpus score --json`, by ID.
pub(crate) fn score_rows(v: &Value) -> BTreeMap<String, Value> {
    // `gp corpus score --json` keeps them in its score record.
    let rows = if v["clips"].is_array() {
        &v["clips"]
    } else {
        &v["record"]["predicate"]["clips"]
    };
    rows.as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r["id"].as_str()?.to_owned(), r.clone())))
        .collect()
}

/// The clips a score is worse on than the real model's: one that fails
/// where it passed, fails more (a higher score or fewer frames together)
/// or can't be made.
pub(crate) fn worse(real: &BTreeMap<String, Value>, other: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for (id, r) in score_rows(other) {
        let Some(b) = real.get(&id) else { continue };
        let err = |x: &Value| x.get("error").is_some_and(|e| !e.is_null());
        let is_worse = if err(&r) {
            !err(b)
        } else if err(b) {
            false
        } else {
            (b["exact"] == true && r["exact"] != true)
                || r["score"].as_u64().unwrap_or(0) > b["score"].as_u64().unwrap_or(0)
                || r["permille"].as_u64().unwrap_or(0) < b["permille"].as_u64().unwrap_or(0)
        };
        if is_worse {
            out.push(id);
        }
    }
    out
}

/// A model folder (model.toml and the real mapping) holding `text`, for
/// `--model`.
/// `--model`, with the package's hook crate beside it: `gp corpus score
/// --model` builds a model's hooks from the folder it is given
/// (crate::module::built_for), keyed by their code, so every mutant shares
/// one build.
fn model_dir(dir: &Path, text: &str, mapping: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    with_hooks(mapping.parent().unwrap_or(Path::new(".")), dir)?;
    fs::write(dir.join(gp_model::MODEL_FILE), text)?;
    if mapping.is_file() {
        fs::copy(mapping, dir.join(gp_interp::mapping::MAPPING_FILE))?;
    }
    Ok(())
}

/// The package's hook crate (ext/ and its Cargo.lock) copied into `dir`
/// when `dir` has none, so a model folder given with `--model` builds the
/// hooks its model names.
pub(crate) fn with_hooks(package: &Path, dir: &Path) -> Result<()> {
    if package.join("ext").is_dir() && !dir.join("ext").exists() {
        copy_dir(&package.join("ext"), &dir.join("ext"))?;
        for f in ["Cargo.lock", "package.toml", "template.toml"] {
            if package.join(f).is_file() && !dir.join(f).exists() {
                fs::copy(package.join(f), dir.join(f))?;
            }
        }
    }
    Ok(())
}

pub fn clips(paths: &Paths, a: &ClipsArgs) -> Result<Value> {
    let t0 = Instant::now();
    let mut times: BTreeMap<&'static str, f64> = BTreeMap::new();
    let mut clock = Instant::now();
    let mut lap = |name: &'static str, clock: &mut Instant| {
        times.insert(name, round3(clock.elapsed().as_secs_f64()));
        *clock = Instant::now();
    };
    let root = root_of(a.root.as_deref())?;
    let (spec_file, game) = bench_file(&a.game)?;
    let spec: BenchFile = toml::from_str(
        &fs::read_to_string(&spec_file)
            .with_context(|| format!("reading {}", spec_file.display()))?,
    )
    .with_context(|| spec_file.display().to_string())?;
    let grader = Grader::load(&root, &spec_file)?;
    let template = spec.bench.package(&root);
    let rom = rom_of(a.rom.as_deref(), &spec.bench, &root, &template, paths)?;
    let rom_bytes = fs::read(&rom).with_context(|| format!("reading the ROM {}", rom.display()))?;
    let sha = crate::roms::sha256_of(&rom_bytes)?;
    let real_text = fs::read_to_string(template.join(gp_model::MODEL_FILE))?;
    let real: Toml = toml::from_str(&real_text)?;
    let (answer, _) = grader.answer()?;

    // The bench's home, made afresh, with a copy of the package: no tool
    // writes in the games checkout.
    let work = fresh_work(
        &a.work
            .clone()
            .unwrap_or_else(|| paths.home.join("work").join("bench").join(&game)),
        "gp bench clips",
    )?;
    let g = work.join("game");
    copy_dir(&template, &g)?;
    let tools = Tools {
        gp: std::env::current_exe().context("finding the gp binary")?,
        home: work.join("home"),
        root: work.clone(),
        sandbox: None,
        env: vec![("GP_GAMES_DIR".into(), root.clone())],
    };
    home_with_rom(&tools.home, &rom)?;
    let gs = s(&g);

    // The corpus on the real model.
    let (real_score, real_failed) =
        tools.json_exit(&["corpus", "score", &gs, "--run", "real"], true)?;
    let real_file = work.join("real.json");
    fs::write(&real_file, serde_json::to_string_pretty(&real_score)?)?;
    let real_rows = score_rows(&real_score);
    lap("run", &mut clock);

    // The code the corpus runs, and the dense clips of each seed.
    let reduce = tools.json(&["corpus", "reduce", &gs])?;
    let mut dense = Vec::new();
    for &seed in &spec.bench.seeds {
        let into = work.join(format!("dense-{seed}"));
        let d = tools.json(&[
            "capture",
            "dense",
            &gs,
            "--seed",
            &seed.to_string(),
            "--budget",
            &spec.bench.dense_budget.to_string(),
            "--keep",
            &spec.bench.dense_keep.to_string(),
            "--into",
            &s(&into),
        ])?;
        dense.push(d);
    }
    lap("matrix", &mut clock);

    // The wrong model.
    let wrong = Mutant {
        name: "wrong-model".into(),
        says: spec.wrong_model.says.clone(),
        set: spec.wrong_model.set.clone(),
        ..Mutant::default()
    };
    let mapping = template.join(gp_interp::mapping::MAPPING_FILE);
    let wrong_dir = work.join("wrong-model");
    model_dir(
        &wrong_dir,
        &crate::mutants::mutate(&real_text, &wrong)?,
        &mapping,
    )?;
    let reuse = s(&real_file);
    let score_on = |dir: &Path, run: &str| {
        tools.json_exit(
            &[
                "corpus",
                "score",
                &gs,
                "--run",
                run,
                "--model",
                &s(dir),
                "--reuse",
                &reuse,
            ],
            true,
        )
    };
    let (wrong_score, wrong_failed) = score_on(&wrong_dir, "real")?;
    let wrong_worse = worse(&real_rows, &wrong_score);
    lap("wrong_model", &mut clock);

    // Mutants of the real model, scored on the corpus.
    let made_all = crate::mutants::make(&real);
    let about: BTreeMap<String, (&'static str, String, String)> = made_all
        .iter()
        .map(|m| {
            (
                m.mutant.name.clone(),
                (m.kind, m.path.clone(), m.what.clone()),
            )
        })
        .collect();
    let mut mutants: Vec<Mutant> = made_all.into_iter().map(|m| m.mutant).collect();
    let made = mutants.len();
    let limited = a.mutants.is_some_and(|n| n < made);
    if let Some(n) = a.mutants {
        mutants.truncate(n);
    }
    fs::write(
        work.join("mutants.toml"),
        crate::mutants::file_text(&mutants)?,
    )?;
    let mut by_kind: BTreeMap<&str, (u64, u64, u64)> = crate::mutants::KINDS
        .iter()
        .map(|(k, _)| (*k, (0, 0, 0)))
        .collect();
    let mut uncaught = Vec::new();
    let mut invalid = Vec::new();
    let mut catches: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mdir = work.join("mutant");
    for m in &mutants {
        let Some((kind, path, what)) = about.get(&m.name) else {
            continue;
        };
        let e = by_kind.entry(kind).or_default();
        let scored = crate::mutants::mutate(&real_text, m)
            .and_then(|text| model_dir(&mdir, &text, &mapping))
            .and_then(|()| score_on(&mdir, "real"));
        let v = match scored {
            Ok((v, _)) => v,
            Err(err) => {
                e.2 += 1;
                invalid.push(json!({"mutant": m.name, "part": path, "change": what,
                    "error": format!("{err:#}").lines().last().unwrap_or("").to_owned()}));
                continue;
            }
        };
        e.1 += 1;
        let w = worse(&real_rows, &v);
        if w.is_empty() {
            uncaught.push(json!({
                "mutant": m.name, "kind": kind, "part": path, "change": what,
                "lives_in": grader.lives_in(path, &answer),
            }));
        } else {
            e.0 += 1;
            for id in w {
                catches.entry(id).or_default().push(m.name.clone());
            }
        }
    }
    lap("mutants", &mut clock);
    let run_clips = real_rows.len();
    let (caught, tried): (u64, u64) = by_kind
        .values()
        .fold((0, 0), |(c, t), x| (c + x.0, t + x.1));

    // The grade.
    let p = &reduce["plan"];
    let ways = reduce["untaken"].as_array().map_or(0, Vec::len);
    let counted_untaken = reduce["untaken"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|u| u["counted"] == true)
        .count();
    let recipes = |d: &Value| -> BTreeSet<String> {
        d["clips"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| format!("{} {}", c["start"], c["presses"]))
            .collect()
    };
    let seeds_jaccard = dense
        .get(1)
        .map(|d| round3(jaccard(&recipes(&dense[0]), &recipes(d))));
    let dense_line: Vec<String> = dense
        .iter()
        .zip(&spec.bench.seeds)
        .map(|(d, s)| {
            let k = &d["kept"]["search"];
            format!(
                "seed {s}: {} clips, {} frames, {} branch ways (the corpus: {})",
                k["clips"], k["frames"], k["branches"], d["pool"]["branches"]
            )
        })
        .collect();
    let exact = real_rows.values().filter(|r| r["exact"] == true).count();
    let no_key = "not graded: it needs a ground truth written from the game's source (game-printer graded Moss Hop's); this game has none";
    let hand_inputs = [
        "the state mapping (mapping.toml), written by a person",
        "the model the clips are scored on (model.toml), written by a person",
        "the corpus clips' presses (corpus.toml, inputs/), played by a person",
    ];
    let measures = json!([
        {"measure": "Elements found from the ROM", "value": no_key},
        {"measure": "Branch directions found",
         "value": format!("{} branch ways untaken (one way or neither) in the routines the corpus runs, {} of them in level code; covering what it takes needs {} clips by stage, {} not by stage",
            ways, counted_untaken, p["cover_by_stage"], p["cover_not_by_stage"])},
        {"measure": "Unreachable told apart", "value": no_key},
        {"measure": "Inputs found", "value": no_key},
        {"measure": "Coverage filled", "value": dense_line.join("; ")},
        {"measure": "Clips needed",
         "value": format!("{} of the corpus's {} clips ({} of {} own frames), by gp corpus reduce",
            p["kept"], p["clips"], p["kept_frames"], p["all_frames"])},
        {"measure": "Wrong models caught",
         "value": format!("{caught} of {tried} mutants, by the corpus's {run_clips} clips{}", if limited { " (a limited run)" } else { "" }),
         "detail": "Per kind below. A mutant is caught when a clip scores worse on it than on the hand-written model; dropping a mapping field isn't a mutant the existing code can make (it edits model.toml only)"},
        {"measure": "Stable targets",
         "value": format!("dense clips Jaccard {} across seeds; the wrong model ({}) {} on {} clips",
            seeds_jaccard.map_or("-".into(), |x| x.to_string()), spec.wrong_model.says,
            if wrong_worse.is_empty() { "isn't caught" } else { "is caught" }, wrong_worse.len())},
        {"measure": "Hand or LLM steps",
         "value": format!("0 of the bench's {} steps; {} inputs written by hand", 5, hand_inputs.len()),
         "detail": hand_inputs},
    ]);
    let mut v = json!({
        "schema": CLIPS_SCHEMA,
        "bench": spec_file.display().to_string(),
        "rom_sha256": sha,
        "measures": measures,
        "corpus": {"clips": run_clips, "exact": exact, "failed": real_failed},
        "reduce": reduce["plan"],
        "dense": dense.iter().map(|d| json!({"seed": d["seed"], "pool": d["pool"], "kept": d["kept"]})).collect::<Vec<_>>(),
        "seeds": spec.bench.seeds,
        "stability": {
            "seeds": {"dense_jaccard": seeds_jaccard},
            "wrong_model": {"says": spec.wrong_model.says, "worse_on": wrong_worse, "failed": wrong_failed},
        },
        "mutants": {
            "kinds": crate::mutants::KINDS.iter().map(|(k, says)| {
                let (c, t, bad) = by_kind.get(k).copied().unwrap_or_default();
                json!({"kind": k, "says": says, "caught": c, "scored": t, "invalid": bad})
            }).collect::<Vec<_>>(),
            "made": made, "caught": caught, "scored": tried, "run_clips": run_clips, "limited": limited,
            "uncaught": uncaught, "invalid": invalid, "catches_by_clip": catches,
        },
    });
    let hash = Hash::of(serde_json::to_string(&v)?.as_bytes()).to_string();
    v["result_blake3"] = json!(hash);
    times.insert("total", round3(t0.elapsed().as_secs_f64()));
    v["seconds"] = json!(times);
    v["work"] = json!(work.display().to_string());
    fs::write(
        work.join("report.json"),
        serde_json::to_string_pretty(&v)? + "\n",
    )?;
    Ok(v)
}

pub(crate) fn text_of(v: &Value) -> String {
    let mut out = vec![
        format!(
            "{}: graded against the answer key",
            v["bench"].as_str().unwrap_or("")
        ),
        String::new(),
    ];
    out.push("| Measure | Value |".into());
    out.push("|---|---|".into());
    for m in v["measures"].as_array().into_iter().flatten() {
        out.push(format!(
            "| {} | {} |",
            m["measure"].as_str().unwrap_or(""),
            m["value"].as_str().unwrap_or("")
        ));
    }
    out.push(String::new());
    out.push("| Mutant kind | Caught | Scored | Invalid |".into());
    out.push("|---|---|---|---|".into());
    for k in v["mutants"]["kinds"].as_array().into_iter().flatten() {
        out.push(format!(
            "| {} ({}) | {} | {} | {} |",
            k["kind"].as_str().unwrap_or(""),
            k["says"].as_str().unwrap_or(""),
            k["caught"],
            k["scored"],
            k["invalid"]
        ));
    }
    let unc = v["mutants"]["uncaught"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !unc.is_empty() {
        out.push(String::new());
        out.push(format!("Uncaught ({}):", unc.len()));
        for u in &unc {
            out.push(format!(
                "  {} {}: {}; in {}",
                u["mutant"].as_str().unwrap_or(""),
                u["part"].as_str().unwrap_or(""),
                u["change"].as_str().unwrap_or(""),
                u["lives_in"].as_str().unwrap_or("")
            ));
        }
    }
    out.push(String::new());
    out.push(format!(
        "result {}; wall time {} s; work in {}",
        v["result_blake3"].as_str().unwrap_or(""),
        v["seconds"]["total"],
        v["work"].as_str().unwrap_or("")
    ));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_is_worse_when_it_fails_more_or_cant_be_made() {
        let real = score_rows(&json!({"clips": [
            {"id": "a", "exact": true, "score": 0, "permille": 1000},
            {"id": "b", "exact": false, "score": 3, "permille": 900},
            {"id": "c", "exact": true, "score": 0, "permille": 1000},
        ]}));
        let other = json!({"clips": [
            {"id": "a", "exact": false, "score": 1, "permille": 990},
            {"id": "b", "exact": false, "score": 3, "permille": 900},
            {"id": "c", "error": "not made"},
        ]});
        assert_eq!(worse(&real, &other), ["a", "c"]);
        assert!(worse(&real, &json!({"clips": []})).is_empty());
    }

    /// `gp bench report` prints the measures of both reports, and exits 2
    /// with neither.
    #[test]
    fn report_reads_both_reports_or_exits_2() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(Some(dir.path().to_path_buf()), None, None).unwrap();
        let a = ReportArgs {
            game: "made-up".into(),
            clips: None,
            scratch: None,
        };
        assert_eq!(report(&paths, &a).exit, 2);
        let bench = dir.path().join("work/bench/made-up");
        fs::create_dir_all(&bench).unwrap();
        fs::write(
            bench.join("report.json"),
            json!({"bench": "bench/made-up.toml", "measures": [{"measure": "Clips needed", "value": "3 of 9"}],
                "mutants": {"kinds": [], "uncaught": []}, "result_blake3": "x", "seconds": {"total": 1}, "work": "w"})
                .to_string(),
        )
        .unwrap();
        let o = report(&paths, &a);
        assert_eq!(o.exit, 0);
        assert!(o.text.contains("| Clips needed | 3 of 9 |"), "{}", o.text);
    }
}
