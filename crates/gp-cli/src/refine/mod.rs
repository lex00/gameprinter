//! `gp model refine GAME --lockstep`: a model refined against the original,
//! clip by clip, from the game's corpus; and `gp model questions`, the
//! questions it asks.
//!
//! First the clips are checked against the ROM: each is played on the
//! original by the corpus scorer ([`clipset::regen_with`]) with the game's
//! own model, and its curve, the hash of the original's mapped measures on
//! every frame it pairs with the model's, is compared with the one recorded
//! for it, in the clip set or in a score given with `--against`. A clip whose curve differs, or that can't be made, is
//! reported and not used. A corpus plan records no curves of its own, so
//! without `--against` its clips are used unchecked, and said so.
//!
//! Then each clip the model doesn't play exactly gets the lockstep loop
//! ([`lockstep`]), started from the original's state at the clip's start.
//! A value it keeps must also leave every other clip used no worse: none
//! with fewer frames together or an earlier first split, run the same way.
//!
//! Ported from game-printer's refine_lockstep.rs and refine.rs. Its
//! `gp model propose` and the recording-driven `gp model refine`, which
//! measure from sprite tracks of store recordings (gp-track, gp-extract's
//! feel and propose), aren't here; neither are the solvers.

pub(crate) mod lockstep;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use gp_interp::lockstep::Lockstep;
use gp_interp::mapping::{MAPPING_FILE, Mapping};
use gp_model::{MODEL_FILE, Model};
use gp_runtime::InputFrame;
use gp_sim::Fx;
use serde_json::{Value, json};

use crate::Outcome;
use crate::clip::cache::Saved;
use crate::clip_score::{Ctx, Open};
use crate::clipset;
use crate::hosted::Source;
use gp_extract::apply::{Fact, apply, at};
use gp_extract::questions::{QUESTIONS_FILE, Question, Questions};

#[derive(Args)]
pub struct RefineArgs {
    /// The game's package folder (games/donkey-kong): its corpus is what
    /// the model is refined on.
    pub game: PathBuf,
    /// Refine from the lockstep with the original, clip by clip. The only
    /// way this gp refines.
    #[arg(long, required = true)]
    pub lockstep: bool,
    /// The working copy whose model.toml and questions.toml are rewritten:
    /// a copy of the game's folder outside any git checkout (default: GAME
    /// itself, when it isn't in one).
    #[arg(long)]
    pub model: Option<PathBuf>,
    /// Refine on these clips only, by ID; the others aren't held either.
    /// Repeatable. Default: every clip of the corpus.
    #[arg(long = "clip", value_name = "ID")]
    pub clips: Vec<String>,
    /// A score whose rows record each clip's curve (`gp corpus score`,
    /// `gp game score` or a canary's JSON): each clip is checked against
    /// it before it is used.
    #[arg(long)]
    pub against: Option<PathBuf>,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE,
    /// lifted there first if missing).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// The ROM; by default the one in $GP_ROM_DIR the package names.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// A window's length: frames (300) or seconds (5s).
    #[arg(long, default_value_t = lockstep::DEFAULT_WINDOW, value_parser = lockstep::parse_window)]
    pub window: u64,
    /// The most rounds on each clip.
    #[arg(long, default_value_t = lockstep::DEFAULT_ROUNDS)]
    pub rounds: u32,
    /// One of the clip's own moments, KIND:N (jump:1), with a single
    /// --clip: windows in its frames, and only its mechanic's numbers.
    #[arg(long, value_parser = lockstep::parse_moment)]
    pub moment: Option<(String, usize)>,
    /// Only this mechanic's numbers (gp_interp's catalog name). Repeatable.
    #[arg(long = "mechanic", value_name = "NAME")]
    pub mechanics: Vec<String>,
    /// Drift thresholds, FIELD=DISTANCE,...: a window ends where a measure
    /// drifts past its threshold.
    #[arg(long, value_parser = lockstep::parse_drift)]
    pub drift: Option<Vec<(String, Fx)>>,
    /// The run the clips are made in, under $GP_HOME/work/refine/.
    #[arg(long, default_value = "refine")]
    pub run: String,
    /// Each of the loop's progress events (the window, the passes' sizes,
    /// each trial, each kept value) as one JSON line on standard error,
    /// `progress: {"clip": ID, ...}`, besides the lines in words: what
    /// `gp serve`'s live card reads to move its bars.
    #[arg(long)]
    pub progress_json: bool,
}

pub(crate) fn read_questions(dir: &Path) -> Result<Questions> {
    match fs::read_to_string(dir.join(QUESTIONS_FILE)) {
        Ok(t) => Questions::parse(&t)
            .map_err(anyhow::Error::msg)
            .with_context(|| dir.join(QUESTIONS_FILE).display().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Questions::default()),
        Err(e) => Err(e).with_context(|| dir.join(QUESTIONS_FILE).display().to_string()),
    }
}

fn read_model(dir: &Path) -> Result<(String, Model)> {
    let file = dir.join(MODEL_FILE);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let model = Model::parse(&text).with_context(|| file.display().to_string())?;
    Ok((text, model))
}

/// A model value as the questions' candidates write it.
fn value_text(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The refs an answer writes into the fact's evidence: the question, then
/// the sources the chosen candidate was read at. A candidate without refs
/// that keeps the model's value keeps the refs of the evidence entry that
/// covers the fact, so an answer never drops the source it was read from.
fn answer_refs(model: &Model, q: &Question, chosen: usize) -> Vec<gp_model::model::Ref> {
    let mut refs = vec![at(format!("{QUESTIONS_FILE} {}", q.id))];
    let c = &q.candidates[chosen];
    let mut cited = c.refs.clone();
    if cited.is_empty() {
        let tree = model.to_tree();
        let same = gp_model::facts(&tree)
            .into_iter()
            .find(|(f, _)| gp_model::path::display(f) == q.path)
            .is_some_and(|(_, v)| value_text(v) == c.value);
        if same {
            cited = gp_model::validate::evidence_for(model)
                .into_iter()
                .find(|(f, _)| gp_model::path::display(f) == q.path)
                .and_then(|(_, key)| key)
                .and_then(|k| model.evidence.get(&k))
                .map(|e| e.refs.clone())
                .unwrap_or_default();
        }
    }
    for r in cited {
        if !refs.contains(&r) {
            refs.push(r);
        }
    }
    refs
}

/// `gp model questions DIR`: lists the working copy's questions, and with
/// `answers` (`ID=N`, N the candidate's number from 1) writes each chosen
/// answer into the model as a stated fact. A shape question's answer
/// approves a general mechanic for someone to build, and writes nothing.
pub(crate) fn questions(dir: &Path, answers: &[String]) -> Result<Outcome> {
    let mut qs = read_questions(dir)?;
    let (_, model) = read_model(dir)?;
    let mut facts = Vec::new();
    for a in answers {
        let (id, n) = a
            .rsplit_once('=')
            .with_context(|| format!("`{a}` isn't ID=N"))?;
        let n: usize = n
            .parse()
            .with_context(|| format!("`{a}`: N isn't a number"))?;
        let q = qs
            .questions
            .iter_mut()
            .find(|q| q.id == id)
            .with_context(|| format!("there's no question `{id}`"))?;
        if n == 0 || n > q.candidates.len() {
            bail!("`{id}` has candidates 1 to {}", q.candidates.len());
        }
        q.answer = Some(n - 1);
        if !matches!(q.kind.as_str(), "which-entity" | "shape" | "worse") {
            facts.push(Fact {
                path: q.path.clone(),
                value: q.candidates[n - 1].value.clone(),
                source: "stated",
                confidence: 100,
                note: format!(
                    "Answered by the person: {} ({})",
                    q.text,
                    q.candidates[n - 1].from
                ),
                refs: answer_refs(&model, q, n - 1),
            });
        }
    }
    if !facts.is_empty() {
        let (text, _) = read_model(dir)?;
        let new = apply(&text, &facts).map_err(anyhow::Error::msg)?;
        fs::write(dir.join(MODEL_FILE), new)?;
    }
    if !answers.is_empty() {
        let file = dir.join(QUESTIONS_FILE);
        fs::write(&file, qs.to_toml()).with_context(|| format!("writing {}", file.display()))?;
    }
    let mut t = String::new();
    for q in &qs.questions {
        let _ = writeln!(
            t,
            "{} {} [{}]\n  {}\n  settle: {}",
            if q.answer.is_some() {
                "answered"
            } else {
                "open    "
            },
            q.id,
            q.kind,
            q.text,
            q.settle
        );
        for (i, c) in q.candidates.iter().enumerate() {
            let mark = if q.answer == Some(i) { "*" } else { " " };
            let _ = writeln!(t, "   {mark}{}. {}  ({})", i + 1, c.value, c.from);
        }
    }
    if qs.questions.is_empty() {
        t.push_str("no questions");
    }
    let open = qs.open().count();
    Ok(Outcome::ok(
        json!({ "questions": qs.questions, "open": open, "applied": facts.len() }),
        t.trim_end().to_owned(),
    ))
}

/// The file the last `gp model refine` run on a game is kept in, for the
/// mechanic page's refinement view: $GP_HOME/work/refine/GAME/last.json.
fn last_file(paths: &Paths, game: &str) -> PathBuf {
    paths
        .home
        .join("work")
        .join("refine")
        .join(game)
        .join("last.json")
}

/// The last run on `game` (its package folder's name), as the page reads
/// it: `{"run": null}` when there is none.
pub(crate) fn last_run(paths: &Paths, game: &str) -> Value {
    let run = crate::mechanic::name_ok(game)
        .then(|| fs::read(last_file(paths, game)).ok())
        .flatten()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .unwrap_or(Value::Null);
    json!({"game": game, "run": run})
}

/// Refuses a working copy inside a git checkout: the loop rewrites its
/// model.toml, and a game's folder in the games repo changes only through
/// the gate.
fn refuse_checkout(dir: &Path) -> Result<()> {
    let full = dir
        .canonicalize()
        .with_context(|| format!("no folder {}", dir.display()))?;
    if let Some(top) = full.ancestors().find(|p| p.join(".git").exists()) {
        bail!(
            "{} is inside the git checkout {}; gp model refine rewrites its model.toml, so give it a copy outside any checkout with --model (cp -R {} /some/where)",
            dir.display(),
            top.display(),
            dir.display()
        );
    }
    Ok(())
}

/// The curve each clip is recorded with in a score's JSON: a canary's
/// `clips`, or a score record's `record.predicate.clips`.
fn recorded_curves(file: &Path) -> Result<BTreeMap<String, String>> {
    let v: Value = serde_json::from_slice(
        &fs::read(file).with_context(|| format!("reading {}", file.display()))?,
    )
    .with_context(|| format!("{} isn't JSON", file.display()))?;
    let rows = v["clips"]
        .as_array()
        .or_else(|| v["record"]["predicate"]["clips"].as_array())
        .or_else(|| v["json"]["record"]["predicate"]["clips"].as_array())
        .with_context(|| format!("{} has no rows of clips", file.display()))?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let id = r["id"].as_str()?;
            let c = r["curve"].as_str().filter(|c| !c.is_empty())?;
            Some((id.to_owned(), c.to_owned()))
        })
        .collect())
}

/// One clip as every run of it needs it.
struct ClipRun {
    id: String,
    /// Its input from power-on.
    inputs: Vec<InputFrame>,
    /// The original's frame it starts at.
    start: u64,
    resume: Option<(Saved, gp_interp::lockstep::Prefix)>,
    draws: Option<gp_sim::DrawScript>,
}

impl ClipRun {
    fn make(ctx: &Ctx, game: &Path, recipe: &clipset::Recipe, mapping: &Mapping) -> Result<Self> {
        let (log, start) = clipset::recipe_log(game, recipe)?.with_context(|| {
            format!(
                "clip {} starts from a recording or its presses file is missing",
                recipe.id
            )
        })?;
        let inputs = crate::run::frames(&log);
        let resume = crate::clip_score::resume_of(ctx, &log, &inputs, start)?;
        let from = start as u64;
        let draws = if mapping.score_draws {
            Some(match &resume {
                Some((saved, p)) => crate::lockstep::record_draws_on(
                    saved.nes(ctx.rom(), gp_evidence::manifest::PowerOn::default())?,
                    mapping,
                    &log.frames[p.inputs..],
                    from,
                )?,
                None => crate::lockstep::record_draws(ctx.rom(), mapping, &log.frames, from)?,
            })
        } else {
            None
        };
        Ok(Self {
            id: recipe.id.clone(),
            inputs,
            start: from,
            resume,
            draws,
        })
    }
}

/// How a clip's whole run in resync mode stands: frames together, and the
/// original's frame of its first split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Standing {
    together: usize,
    frames: usize,
    first: Option<u64>,
}

impl Standing {
    fn of(ls: &Lockstep) -> Self {
        Self {
            together: ls.frames.len() - ls.splits.len(),
            frames: ls.frames.len(),
            first: ls.splits.first().map(|s| s.original_frame),
        }
    }

    fn exact(self) -> bool {
        self.first.is_none()
    }

    /// Why `self` is worse than `was`, if it is.
    fn worse_than(self, was: Standing) -> Option<String> {
        if self.together < was.together {
            return Some(format!(
                "{} of {} frames together, was {}",
                self.together, self.frames, was.together
            ));
        }
        match (self.first, was.first) {
            (Some(n), None) => Some(format!("splits at the original's frame {n}, was exact")),
            (Some(n), Some(w)) if n < w => {
                Some(format!("first splits at the original's frame {n}, was {w}"))
            }
            _ => None,
        }
    }
}

/// Everything the clips' whole runs share.
struct Bench<'a> {
    base: &'a Source,
    rom: &'a [u8],
    mapping: Mapping,
    tolerance: Vec<Fx>,
}

impl Bench<'_> {
    /// The clip's whole run in resync mode on `model`, from its start.
    fn whole(&self, c: &ClipRun, model: &Model) -> Result<Standing> {
        use gp_interp::lockstep::{self, Mode, Options};
        let mut interp = self.base.edited(model)?.open()?;
        if let Some(mut d) = c.draws.clone() {
            d.base = c.start;
            interp = interp.with_draws(d)?;
        }
        let spec = interp.spec().clone();
        let opts = Options {
            mode: Mode::Resync,
            tolerance: self.tolerance.clone(),
            from: Some(c.start),
            ..Options::default()
        };
        let ls = match &c.resume {
            Some((saved, at)) => {
                let mut o = crate::original::OriginalRuntime::from_saved(self.rom, saved)?;
                lockstep::lockstep_resumed(
                    &mut o,
                    &mut interp,
                    &spec,
                    &self.mapping,
                    &c.inputs,
                    &opts,
                    None,
                    *at,
                )
            }
            None => {
                let mut o = crate::original::OriginalRuntime::of_rom(self.rom)?;
                lockstep::lockstep(&mut o, &mut interp, &spec, &self.mapping, &c.inputs, &opts)
            }
        }
        .map_err(anyhow::Error::msg)?;
        Ok(Standing::of(&ls))
    }

    /// Every clip's whole run on `model`, on the helper threads.
    fn standings(&self, clips: &[&ClipRun], model: &Model) -> Vec<Result<Standing>> {
        crate::pool::map(clips, || (), |_, _, c| self.whole(c, model), &mut |_| {})
    }
}

/// A round's progress on standard error, a line for each window, impact
/// pass and kept value, so a long run shows where it is.
fn progress(clip: &str, p: &Value, as_json: bool) {
    if as_json {
        let mut e = p.clone();
        if let Some(o) = e.as_object_mut() {
            o.insert("clip".into(), json!(clip));
        }
        eprintln!("progress: {e}");
    }
    let line = if let Some(w) = p.get("window") {
        format!(
            "window: the original's frames {} to {}",
            w["original_from"], w["original_to"]
        )
    } else if let Some(t) = p["target"]["says"].as_str() {
        t.to_owned()
    } else if let Some(t) = p["impact"]["says"].as_str() {
        t.to_owned()
    } else if let Some(r) = p.get("round") {
        format!(
            "round {}: {} {} -> {}",
            r["round"],
            r["param"].as_str().unwrap_or(""),
            r["old"].as_str().unwrap_or(""),
            r["new"].as_str().unwrap_or("")
        )
    } else {
        return;
    };
    eprintln!("{clip}: {line}");
}

/// `gp model refine GAME --lockstep`.
pub(crate) fn run(a: &RefineArgs, paths: &Paths) -> Result<Outcome> {
    let game = fs::canonicalize(&a.game).with_context(|| a.game.display().to_string())?;
    let dir = a.model.clone().unwrap_or_else(|| game.clone());
    refuse_checkout(&dir)?;
    let dir = fs::canonicalize(&dir)?;
    if a.moment.is_some() && a.clips.len() != 1 {
        bail!("--moment names a moment of one clip: give exactly one --clip");
    }
    let rom = crate::clip_score::rom_of(paths, &game, a.rom.as_deref())?;
    let assets = crate::corpus_score::assets_of(paths, &game, &dir, a.assets.as_deref(), &rom)?;
    let (source, _) = Source::open_dir(&dir, Some(&assets), paths, a.module.as_deref())?;
    let source = Arc::new(source);
    let mapping_file = dir.join(MAPPING_FILE);
    let mapping = Mapping::parse(
        &fs::read_to_string(&mapping_file)
            .with_context(|| format!("reading {}", mapping_file.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", mapping_file.display()))?;
    let set = clipset::read_game(&game)?;
    for id in &a.clips {
        if !set.clips.iter().any(|r| &r.id == id) {
            bail!("{} has no clip {id}", game.display());
        }
    }
    let recipes: Vec<&clipset::Recipe> = set
        .clips
        .iter()
        .filter(|r| a.clips.is_empty() || a.clips.contains(&r.id))
        .collect();
    let mut lines: Vec<String> = Vec::new();

    // The clips checked against the ROM: each played on the original, its
    // curve against the one recorded for it.
    let against = match &a.against {
        Some(f) => recorded_curves(f)?,
        None => BTreeMap::new(),
    };
    let recorded = |r: &clipset::Recipe| -> Option<String> {
        (!r.curve.is_empty())
            .then(|| r.curve.clone())
            .or_else(|| against.get(&r.id).cloned())
    };
    let mut checks: BTreeMap<String, Value> = BTreeMap::new();
    let mut usable: Vec<&clipset::Recipe> = Vec::new();
    if recipes.iter().any(|r| recorded(r).is_some()) {
        let run_dir = paths.home.join("work").join("refine").join(&a.run);
        fs::create_dir_all(&run_dir)?;
        // Played against the game's own model, as the recorded curves were:
        // a curve holds the original's measures on the frames paired with
        // the model's, so a working copy whose numbers are off would move
        // it. With the same model, only the clip on the original can.
        let (own, _) = Source::open_dir(&game, Some(&assets), paths, a.module.as_deref())?;
        let work = clipset::Work {
            paths,
            run_dir: &run_dir,
            run: &a.run,
            mdir: &game,
            assets: &assets,
            rom: rom.clone(),
            source: Arc::new(own),
        };
        let only = recipes.iter().map(|r| r.id.clone()).collect();
        let g = clipset::regen_with(&work, &game, Some(&only), &clipset::Scoring::default())?;
        for r in &recipes {
            let now = g.curves.get(&r.id);
            let failed = match g.outcomes.get(&r.id) {
                Some(Err(e)) => Some(e.clone()),
                None => Some("it couldn't be made from the ROM".into()),
                Some(Ok(_)) => None,
            };
            let (status, used) = match (failed, recorded(r), now) {
                (Some(e), _, _) => (json!({"status": "failed", "why": e}), false),
                (None, None, _) => (json!({"status": "unchecked"}), true),
                (None, Some(want), Some(c)) if *c == want => {
                    (json!({"status": "checked", "curve": c}), true)
                }
                (None, Some(want), c) => (
                    json!({"status": "differs", "recorded": want, "now": c}),
                    false,
                ),
            };
            if !used {
                lines.push(format!(
                    "{}: not used: {}",
                    r.id,
                    match status["status"].as_str() {
                        Some("failed") => format!(
                            "it can't be played on the original: {}",
                            status["why"].as_str().unwrap_or("")
                        ),
                        _ => "on the original it doesn't give the curve recorded for it".into(),
                    }
                ));
            } else {
                usable.push(r);
            }
            checks.insert(r.id.clone(), status);
        }
    } else {
        for r in &recipes {
            checks.insert(r.id.clone(), json!({"status": "unchecked"}));
            usable.push(r);
        }
    }
    let count = |s: &str| checks.values().filter(|v| v["status"] == s).count();
    lines.push(format!(
        "clips against the ROM: {} checked, {} unchecked (no curve recorded for them), {} differ, {} can't be played",
        count("checked"),
        count("unchecked"),
        count("differs"),
        count("failed")
    ));

    // Each usable clip's runs, and how it stands on the model now.
    let ctx = Ctx::open(
        paths,
        &game,
        &Open {
            module: a.module.as_deref(),
            model: Some(&dir),
            assets: Some(&assets),
            rom: a.rom.as_deref(),
        },
    )?;
    // Made on the helper threads: a clip new to the start cache plays its
    // lockstep from power-on to its start once.
    let clips: Vec<ClipRun> = crate::pool::map(
        &usable,
        || (),
        |_, _, r| ClipRun::make(&ctx, &game, r, &mapping),
        &mut |_| {},
    )
    .into_iter()
    .collect::<Result<_>>()?;
    let (_, model0) = read_model(&dir)?;
    let (tolerance, _) = crate::lockstep::tolerances(&dir, &model0, &mapping);
    let bench = Bench {
        base: &source,
        rom: &rom,
        mapping: mapping.clone(),
        tolerance,
    };
    let all: Vec<&ClipRun> = clips.iter().collect();
    // The standings of the model on disk, made again when it changes.
    let held: std::cell::RefCell<(String, Vec<Standing>)> =
        std::cell::RefCell::new((String::new(), Vec::new()));
    let standings_now = || -> Result<Vec<Standing>> {
        let (text, m) = read_model(&dir)?;
        let mut h = held.borrow_mut();
        if h.0 != text || h.1.len() != all.len() {
            let s = bench
                .standings(&all, &m)
                .into_iter()
                .zip(&all)
                .map(|(r, c)| r.with_context(|| format!("clip {}", c.id)))
                .collect::<Result<Vec<_>>>()?;
            *h = (text, s);
        }
        Ok(h.1.clone())
    };
    let before = standings_now()?;
    let exact_before = before.iter().filter(|s| s.exact()).count();
    lines.push(format!(
        "on the model as it was: {exact_before} of {} clips exact in resync from their starts",
        clips.len()
    ));

    let text_before = read_model(&dir)?.0;
    let mut runs: Vec<Value> = Vec::new();
    for (i, c) in clips.iter().enumerate() {
        let now = standings_now()?;
        if now[i].exact() {
            continue;
        }
        // No other clip used may get worse.
        let accept = |m: &Model| -> Result<Option<String>> {
            let now = standings_now()?;
            let others: Vec<&ClipRun> = all
                .iter()
                .enumerate()
                .filter(|(k, _)| *k != i)
                .map(|(_, c)| *c)
                .collect();
            let was: Vec<Standing> = now
                .iter()
                .enumerate()
                .filter(|(k, _)| *k != i)
                .map(|(_, s)| *s)
                .collect();
            for ((c, r), w) in others.iter().zip(bench.standings(&others, m)).zip(was) {
                let s = r.with_context(|| format!("clip {}", c.id))?;
                if let Some(why) = s.worse_than(w) {
                    return Ok(Some(format!("{}: {why}", c.id)));
                }
            }
            Ok(None)
        };
        let scope = lockstep::Scope {
            start_at: Some(c.start),
            mechanics: a.mechanics.clone(),
            accept: Some(&accept),
            moment: a.moment.clone(),
            until: None,
            drift: a.drift.clone().unwrap_or_default(),
        };
        let evidence = format!("clip {}", c.id);
        let out = lockstep::run_scoped(
            &lockstep::LockstepArgs {
                dir: &dir,
                rom: &rom,
                inputs: &c.inputs,
                source: &evidence,
                model: &source,
                resume: c.resume.as_ref().map(|(s, p)| (s, *p)),
                draws: c.draws.clone(),
                rounds: a.rounds,
                window: a.window,
                from: Some(c.start),
            },
            &scope,
            &mut |p| progress(&c.id, p, a.progress_json),
        )?;
        lines.push(format!("{}:", c.id));
        lines.extend(out.text.lines().map(|l| format!("  {l}")));
        let mut j = out.json;
        j["clip"] = json!(c.id);
        runs.push(j);
    }
    let after = standings_now()?;
    let exact_after = after.iter().filter(|s| s.exact()).count();
    let text_after = read_model(&dir)?.0;
    let changed = text_after != text_before;
    let written: Vec<Value> = runs
        .iter()
        .flat_map(|run| {
            run["rounds"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| json!({"param": r["param"], "old": r["old"], "new": r["new"], "clip": run["clip"]}))
        })
        .collect();
    let asked: usize = runs
        .iter()
        .map(|r| r["questions"].as_array().map_or(0, Vec::len))
        .sum();
    if runs.is_empty() {
        lines.push("every clip used is exact: nothing to refine, the model is unchanged".into());
    } else {
        lines.push(format!(
            "now {exact_after} of {} clips exact; {} values written, {asked} questions in {}",
            clips.len(),
            written.len(),
            dir.join(QUESTIONS_FILE).display()
        ));
    }
    let j = json!({
        "game": game,
        "model": dir,
        "checks": checks,
        "exact": {"before": exact_before, "after": exact_after, "clips": clips.len()},
        "changed": changed,
        "written": written,
        "questions": asked,
        "clips": runs,
    });
    if let Some(name) = game.file_name().and_then(|n| n.to_str()) {
        let f = last_file(paths, name);
        if let Some(d) = f.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(&f, serde_json::to_vec(&j)?)?;
    }
    Ok(Outcome {
        json: j,
        text: lines.join("\n"),
        exit: u8::from(asked > 0),
    })
}
