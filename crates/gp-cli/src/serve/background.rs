//! The background work (game-printer's serve/background.rs and
//! serve/workers.rs; docs/show.md, "The background work"): a queue on each
//! run that scores its active clips on the game's model as it stands,
//! several at once, while the person plays.
//!
//! The active clips (the pool) are the clips played in free play and the
//! game's corpus clips, less those marked skipped; the played ones first,
//! starred before kept. A clip is scored by `gp clip score`'s scorer
//! ([`crate::clip_score::score_one`]): the original and the game's module
//! in lockstep from power-on through the clip's start, its measures
//! compared frame by frame. A score is kept with the model it was made on
//! (the hash of model.toml, mapping.toml and the module), and a clip is
//! scored again when that changes.
//!
//! As game-printer's, the queue is paused whenever the server starts,
//! whatever was saved, until "start" in the page. The number of jobs at
//! once is the run's `workers` (8 by default, at most the machine's cores
//! less one); each worker opens the model once and keeps it while the
//! model is unchanged, and rests after a job (`GP_BACKGROUND_PAUSE_MS`, 2
//! seconds).
//!
//! The state is `$GP_HOME/work/serve/GAME/background/state.json`, each
//! clip's score files under `background/scores/`. Its `live` says whether
//! the queue is on and in which server's process, for `gp progress --next`.
//!
//! Once every active clip is scored, the corpus clip that parts earliest
//! gets a refine round: gameprinter's refinement loop (`gp model refine
//! GAME --lockstep --clip ID`) on the run's working copy of the game's
//! folder (`$GP_HOME/work/serve/GAME/model/`, made on the first round),
//! one round at a time. Its progress lines are the job's live card; the
//! values it writes are the round's changes, after which every clip is
//! scored again on the changed model; a round that writes nothing leaves
//! the clip stuck on that model with the loop's questions (the copy's
//! questions.toml), answered from the page through `gp model questions`.
//! The rounds are `background/history.jsonl`. Clips played in free play are
//! refined once `gp corpus import` brings them into the corpus.
//!
//! A round runs `gp model refine --progress-json`, whose progress events
//! (the window, the passes' sizes, each trial, each value kept) are folded
//! into the job's bars ([`fold_progress`]) and sent on the run's event
//! stream as `refine.progress`, so the REFINE tab's live card moves as the
//! round goes. Clips put in the queue by hand ("add to the queue") are
//! scored and refined first, in order. A clip a round leaves stuck goes to
//! the clip agent (`serve/clip_agent.rs`) when one is turned on.
//!
//! The grid of mechanics by contexts over these clips and scores is
//! serve/grid.rs. What game-printer's queue also did and this doesn't yet:
//! the grid's own clips, rescoring by what a change touches, the ledger and its verify, and the
//! curves. Their routes answer in words.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/background` | The state: on or paused, the jobs running now, the jobs just done, each active clip's score, the questions and `says` |
//! | `POST /api/capture/{run}/background/start`, `.../stop` | On (with `{clip}`, that clip first), or paused |
//! | `GET`, `POST /api/capture/{run}/background/workers` | `{workers, max}`; POST `{workers}` |
//! | `GET`, `POST /api/capture/{run}/background/agents` | `{agents, max}`; POST `{agents}`: how many clip agents the stuck clips go to at once |
//! | `GET`, `POST /api/capture/{run}/background/queue` | `{queued}`; POST `{clip}` adds a clip to the queue, `{clip, remove: true}` takes it out |
//! | `POST /api/capture/{run}/background/rescore` | Scores every clip again |
//! | `GET /api/capture/{run}/background/history` | The rounds: none, since no refine runs yet |
//! | `GET /api/capture/{run}/fidelity` | The fidelity line, behaviour alone: the mean of the active clips' frames together. Coverage and the picture aren't measured here yet |

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::game::{Run, read_json, run_dir, run_named, write_json};
use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/background",
            about: "The background work's state: on or paused, the jobs now and just done, each clip's score",
            handler: state,
        },
        Route {
            path: "/api/capture/{run}/background/start",
            about: "Turns the background work on; with {clip}, that clip first",
            handler: start,
        },
        Route {
            path: "/api/capture/{run}/background/stop",
            about: "Pauses the background work after the jobs running now",
            handler: stop,
        },
        Route {
            path: "/api/capture/{run}/background/workers",
            about: "How many jobs at once: {workers, max}; POST {workers}",
            handler: workers,
        },
        Route {
            path: "/api/capture/{run}/background/agents",
            about: "How many clip agents at once: {agents, max}; POST {agents}",
            handler: agents,
        },
        Route {
            path: "/api/capture/{run}/background/rescore",
            about: "Scores every clip again",
            handler: rescore,
        },
        Route {
            path: "/api/capture/{run}/background/history",
            about: "The refine rounds, oldest first: each round's changes, the clips it fixed, or the question it left",
            handler: history,
        },
        Route {
            path: "/api/capture/{run}/background/answer",
            about: "Answers one of the loop's questions: {question, choice}",
            handler: answer,
        },
        Route {
            path: "/api/capture/{run}/background/verify",
            about: "Replays the run's ledger: not here yet",
            handler: verify,
        },
        Route {
            path: "/api/capture/{run}/background/queue",
            about: "The clips put in the queue by hand, worked first in order: {queued}; POST {clip} adds one (with remove: true, takes it out)",
            handler: queue,
        },
        Route {
            path: "/api/capture/{run}/fidelity",
            about: "The fidelity line: behaviour alone, from the active clips' scores",
            handler: fidelity,
        },
        Route {
            path: "/api/capture/{run}/background/curve",
            about: "One clip's curves for the drawer's chart: not here yet",
            handler: curve,
        },
    ]
}

/// Workers by default, as game-printer's.
const DEFAULT_WORKERS: usize = 8;
const DEFAULT_AGENTS: usize = 2;
const MAX_AGENTS: usize = 8;
/// The jobs just done that the state keeps.
const DONE: usize = 12;

/// The most workers: the machine's cores less one.
pub(crate) fn max_workers() -> usize {
    std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .saturating_sub(1)
        .max(1)
}

fn pause() -> Duration {
    let ms = std::env::var("GP_BACKGROUND_PAUSE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    Duration::from_millis(ms)
}

/// A job running now.
#[derive(Clone)]
struct Flight {
    /// `score` or `refine`.
    kind: &'static str,
    /// What a refine round says now, from its progress lines.
    says: String,
    /// A refine round's bars as its progress events move them
    /// ([`fold_progress`]): the pass, the impact pass's trials, the fine
    /// pass's, the best split so far against the start's, the value tried.
    bars: Value,
    clip: String,
    name: String,
    model: String,
    k: usize,
    n: usize,
    since: Instant,
}

/// A run's queue as it stands in memory.
struct Live {
    enabled: bool,
    workers: usize,
    agents: usize,
    /// The clip asked for first ("fix this clip").
    focus: Option<String>,
    flight: BTreeMap<usize, Flight>,
    /// Worker threads started.
    threads: usize,
    error: Option<String>,
}

static LIVE: Mutex<BTreeMap<String, Live>> = Mutex::new(BTreeMap::new());
/// Held while a run's state file is read and replaced.
static FILE: Mutex<()> = Mutex::new(());
/// Each run's events as they happen, newest last, for the page's event
/// stream: at most [`EVENTS_KEPT`], each with its `seq`.
static EVENTS: Mutex<BTreeMap<String, Vec<Value>>> = Mutex::new(BTreeMap::new());
/// Wakes the streams when an event comes.
static EVENT_BELL: std::sync::Condvar = std::sync::Condvar::new();
const EVENTS_KEPT: usize = 2000;

/// Records an event of the run's background work in the capture event
/// format the page reads (docs/formats/capture-events.md): `score.done`,
/// `refine.start`, `refine.progress`, `refine.round`, `queue.on`,
/// `queue.off`.
pub(crate) fn emit(run: &str, kind: &str, data: Value) {
    let mut all = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    let list = all.entry(run.to_owned()).or_default();
    let seq = list.last().and_then(|e| e["seq"].as_u64()).unwrap_or(0) + 1;
    list.push(json!({"v": 1, "seq": seq, "at_ms": now_ms(), "round": 0,
                     "actor": "pipeline", "kind": kind, "stage": "background", "data": data}));
    let over = list.len().saturating_sub(EVENTS_KEPT);
    list.drain(..over);
    EVENT_BELL.notify_all();
}

/// The run's events after `seq`.
pub(crate) fn events_after(run: &str, after: u64) -> Vec<Value> {
    EVENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(run)
        .map(|l| {
            l.iter()
                .filter(|e| e["seq"].as_u64().unwrap_or(0) > after)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Waits up to `wait` for an event after `seq`, then gives those there are.
pub(crate) fn wait_events(run: &str, after: u64, wait: Duration) -> Vec<Value> {
    let g = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    let has = |g: &BTreeMap<String, Vec<Value>>| {
        g.get(run)
            .and_then(|l| l.last())
            .is_some_and(|e| e["seq"].as_u64().unwrap_or(0) > after)
    };
    if has(&g) {
        drop(g);
    } else {
        drop(EVENT_BELL.wait_timeout_while(g, wait, |g| !has(g)));
    }
    events_after(run, after)
}

/// The server's context, for workers started from a handler.
static CTX: OnceLock<Arc<Ctx>> = OnceLock::new();
/// Keeps the server's context for the workers. Every run's queue starts
/// paused.
pub(crate) fn start_dispatcher(ctx: Arc<Ctx>) {
    let _ = CTX.set(ctx.clone());
    super::clip_agent::start_hand_off(ctx);
}

fn state_file(run: &Run) -> PathBuf {
    run.dir.join("background").join("state.json")
}

/// The saved state: `{schema, workers, agents, scores, first}`.
fn saved(run: &Run) -> Value {
    let v = read_json(&state_file(run));
    if v.is_object() {
        v
    } else {
        json!({"schema": 1, "scores": {}, "first": {}})
    }
}

/// Changes the saved state under the lock.
fn update_saved(run: &Run, f: impl FnOnce(&mut Value)) -> Result<Value, Fail> {
    let _g = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let mut v = saved(run);
    f(&mut v);
    write_json(&state_file(run), &v)?;
    Ok(v)
}

fn with_live<T>(run: &Run, f: impl FnOnce(&mut Live) -> T) -> T {
    let mut all = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let l = all.entry(run.name.clone()).or_insert_with(|| {
        let v = saved(run);
        Live {
            enabled: false,
            workers: v["workers"]
                .as_u64()
                .map_or(DEFAULT_WORKERS, |w| w as usize)
                .clamp(1, max_workers()),
            agents: v["agents"]
                .as_u64()
                .map_or(DEFAULT_AGENTS, |a| a as usize)
                .min(MAX_AGENTS),
            focus: None,
            flight: BTreeMap::new(),
            threads: 0,
            error: None,
        }
    });
    f(l)
}

/// The server's context, for the clip agents' threads.
pub(crate) fn server_ctx() -> Option<Arc<Ctx>> {
    CTX.get().cloned()
}

/// The working copy's questions, for the clip agent's prompt.
pub(crate) fn open_questions(run: &Run) -> Vec<Value> {
    questions(run)
}

/// The saved state, for the clip agents (`serve/clip_agent.rs`).
pub(crate) fn saved_state(run: &Run) -> Value {
    saved(run)
}

/// The clips a job has now, for the grid (serve/grid.rs).
pub(crate) fn working_on(run: &Run) -> std::collections::BTreeSet<String> {
    with_live(run, |l| l.flight.values().map(|f| f.clip.clone()).collect())
}

/// The queue as the server knows it, for the loop's answer
/// (crate::next): on or paused, the clips a job has now, and the error
/// that stopped it.
pub(crate) fn live(run: &Run) -> (bool, Vec<String>, Option<String>) {
    with_live(run, |l| {
        (
            l.enabled,
            l.flight.values().map(|f| f.name.clone()).collect(),
            l.error.clone(),
        )
    })
}

/// Keeps whether the queue is on in the state file, with this server's
/// process, so `gp progress --next` and the MCP tool `next` know it
/// without asking the server.
fn record_live(run: &Run, on: bool) {
    let _ = update_saved(run, |v| {
        v["live"] = json!({"on": on, "pid": std::process::id(), "at_ms": now_ms()});
    });
}

/// Whether the run's queue is on, and how many clip agents it hands its
/// stuck clips to at once.
pub(crate) fn queue_on(run: &Run) -> (bool, usize) {
    with_live(run, |l| (l.enabled, l.agents))
}

/// Changes the saved state, for the clip agents.
pub(crate) fn update_state(run: &Run, f: impl FnOnce(&mut Value)) -> Result<Value, Fail> {
    update_saved(run, f)
}

/// The clips put in the queue by hand, in order.
fn queued(v: &Value) -> Vec<String> {
    v["queued"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_str().map(str::to_owned))
        .collect()
}

/// One active clip: its ID, its name, and whether it is starred.
#[derive(Clone)]
pub(crate) struct Active {
    pub id: String,
    pub name: String,
    pub starred: bool,
}

/// The active clips in the queue's order: the played ones first (starred
/// before kept), then the corpus's in its order; skipped ones left out.
pub(crate) fn active_now(ctx: &Ctx, run: &Run) -> Vec<Active> {
    let home = &ctx.paths.home;
    let marks = super::freeplay::marks_for_run(home, &run.name);
    let mut played: Vec<Active> = super::freeplay::clips_for_run(home, &run.name)
        .into_iter()
        .map(|c| {
            let m = marks
                .get(&c.id)
                .map_or(super::freeplay::PLAYED_MARK, String::as_str);
            (c, m.to_owned())
        })
        .filter(|(_, m)| m != "skipped")
        .map(|(c, m)| Active {
            id: c.id,
            name: c.name,
            starred: m == "starred",
        })
        .collect();
    played.sort_by_key(|a| !a.starred);
    let gmarks = super::generated::marks(run);
    let made = super::generated::made(run)
        .into_iter()
        .filter(|c| gmarks.get(&c.id).map(String::as_str) != Some("skipped"))
        .map(|c| Active {
            starred: gmarks.get(&c.id).map(String::as_str) == Some("starred"),
            id: c.id,
            name: c.name,
        });
    played.extend(made);
    played
}

/// What a score depends on: the model, its mapping and the module.
pub(crate) fn model_key_of(ctx: &Ctx, run: &Run, dir: &std::path::Path) -> String {
    let mut h = blake3::Hasher::new();
    for f in [gp_model::MODEL_FILE, gp_interp::mapping::MAPPING_FILE] {
        h.update(&std::fs::read(dir.join(f)).unwrap_or_default());
    }
    if let Some(m) = run.rebuild(ctx).module {
        h.update(&std::fs::read(m).unwrap_or_default());
    }
    h.finalize().to_hex()[..12].to_owned()
}

pub(crate) fn model_key(ctx: &Ctx, run: &Run) -> String {
    let mut h = blake3::Hasher::new();
    for f in [gp_model::MODEL_FILE, gp_interp::mapping::MAPPING_FILE] {
        h.update(&std::fs::read(run.model_dir().join(f)).unwrap_or_default());
    }
    if let Some(m) = run.rebuild(ctx).module {
        h.update(&std::fs::read(m).unwrap_or_default());
    }
    h.finalize().to_hex()[..12].to_owned()
}

/// A worker's next job.
enum Job {
    Score(Active, usize, usize),
    Refine(Active),
}

/// The next job a worker takes: the focus first, then the first active
/// clip not scored on `model` and not running; once every one is scored,
/// a refine round on the corpus clip that parts earliest, one at a time,
/// unless it is stuck on this model.
fn next(ctx: &Ctx, run: &Run, live: &Live, model: &str) -> Option<Job> {
    // A refine round writes the working copy as it goes: nothing is scored
    // on a model half written.
    if live.flight.values().any(|f| f.kind == "refine") {
        return None;
    }
    let active = active_now(ctx, run);
    let v = saved(run);
    let scores = &v["scores"];
    let running: Vec<&str> = live.flight.values().map(|f| f.clip.as_str()).collect();
    let n = active.len();
    let todo = |a: &Active| {
        !running.contains(&a.id.as_str()) && scores[&a.id]["model"].as_str() != Some(model)
    };
    if let Some(f) = &live.focus
        && let Some((k, a)) = active.iter().enumerate().find(|(_, a)| &a.id == f)
        && todo(a)
    {
        return Some(Job::Score(a.clone(), k + 1, n));
    }
    // The clips put in the queue by hand, in their order, then the rest.
    let asked = queued(&v);
    for q in &asked {
        if let Some((k, a)) = active.iter().enumerate().find(|(_, a)| &a.id == q)
            && todo(a)
        {
            return Some(Job::Score(a.clone(), k + 1, n));
        }
    }
    if let Some((k, a)) = active.iter().enumerate().find(|(_, a)| todo(a)) {
        return Some(Job::Score(a.clone(), k + 1, n));
    }
    if !running.is_empty() {
        return None;
    }
    let corpus: Vec<String> = super::generated::made(run)
        .into_iter()
        .map(|c| c.id)
        .collect();
    let can = |a: &Active| {
        let s = &scores[&a.id];
        corpus.contains(&a.id)
            && s["model"].as_str() == Some(model)
            && s["exact"] == json!(false)
            && s["error"].is_null()
            && v["stuck"][&a.id].as_str() != Some(model)
    };
    if let Some(f) = &live.focus
        && let Some(a) = active.iter().find(|a| &a.id == f && can(a))
    {
        return Some(Job::Refine(a.clone()));
    }
    for q in &asked {
        if let Some(a) = active.iter().find(|a| &a.id == q && can(a)) {
            return Some(Job::Refine(a.clone()));
        }
    }
    active
        .iter()
        .filter(|a| can(a))
        .min_by_key(|a| {
            let s = &scores[&a.id];
            (
                s["drift"].as_u64().unwrap_or(u64::MAX),
                s["score"].as_u64().unwrap_or(0),
            )
        })
        .map(|a| Job::Refine(a.clone()))
}

/// Starts workers up to the run's number.
fn ensure_workers(run: &Run) {
    let Some(ctx) = CTX.get() else { return };
    let want = with_live(run, |l| {
        let from = l.threads;
        l.threads = l.threads.max(l.workers);
        from..l.threads
    });
    for slot in want {
        let (ctx, name) = (ctx.clone(), run.name.clone());
        std::thread::spawn(move || worker(&ctx, &name, slot));
    }
}

/// One worker of a run's queue: takes the next job while the queue is on
/// and its slot is under the run's workers.
fn worker(ctx: &Ctx, name: &str, slot: usize) {
    let mut opened: Option<(String, crate::clip_score::Ctx)> = None;
    loop {
        let Ok(run) = run_named(ctx, name) else {
            return;
        };
        let model = model_key(ctx, &run);
        let job = with_live(&run, |l| {
            if !l.enabled || slot >= l.workers {
                return None;
            }
            let job = next(ctx, &run, l, &model)?;
            let (kind, a, k, n) = match &job {
                Job::Score(a, k, n) => ("score", a, *k, *n),
                Job::Refine(a) => ("refine", a, 0, 0),
            };
            l.flight.insert(
                slot,
                Flight {
                    kind,
                    says: String::new(),
                    bars: json!({}),
                    clip: a.id.clone(),
                    name: a.name.clone(),
                    model: model.clone(),
                    k,
                    n,
                    since: Instant::now(),
                },
            );
            Some(job)
        });
        let a = match job {
            None => {
                if with_live(&run, |l| !l.enabled || slot >= l.workers) {
                    opened = None;
                }
                std::thread::sleep(Duration::from_millis(700));
                continue;
            }
            Some(Job::Refine(a)) => {
                refine(ctx, &run, slot, &a, &model);
                with_live(&run, |l| {
                    l.flight.remove(&slot);
                    if l.focus.as_deref() == Some(a.id.as_str()) {
                        l.focus = None;
                    }
                });
                std::thread::sleep(pause());
                continue;
            }
            Some(Job::Score(a, _, _)) => a,
        };
        if opened.as_ref().is_none_or(|(k, _)| k != &model) {
            let rb = run.rebuild(ctx);
            let mdir = run.model_dir();
            let how = crate::clip_score::Open {
                module: rb.module.as_deref(),
                model: Some(&mdir),
                assets: None,
                rom: None,
            };
            match crate::clip_score::Ctx::open(&ctx.paths, &run.game, &how) {
                Ok(c) => opened = Some((model.clone(), c)),
                Err(e) => {
                    with_live(&run, |l| {
                        l.error = Some(format!("couldn't open the model: {e:#}"));
                        l.enabled = false;
                        l.flight.remove(&slot);
                    });
                    record_live(&run, false);
                    continue;
                }
            }
        }
        let sc = &opened.as_ref().expect("opened").1;
        let score = score_clip(ctx, &run, sc, &a, &model);
        emit(
            &run.name,
            "score.done",
            json!({"clip": a.id, "name": a.name, "score": score["score"], "exact": score["exact"],
                   "says": score["says"], "error": score["error"]}),
        );
        let _ = update_saved(&run, |v| {
            let before = v["scores"][&a.id]["score"].as_u64();
            if v["first"].get(&a.id).is_none() && score["error"].is_null() {
                v["first"][&a.id] = score["exact"].clone();
            }
            let mut s = score.clone();
            s["fixed"] = json!(v["first"][&a.id] == json!(false) && s["exact"] == json!(true));
            v["scores"][&a.id] = s;
            push_done(
                v,
                json!({
                    "kind": "score",
                    "clip": a.id,
                    "name": a.name,
                    "before": before,
                    "after": score["score"],
                    "exact": score["exact"],
                    "error": score["error"],
                    "says": score["says"],
                }),
            );
        });
        with_live(&run, |l| {
            l.flight.remove(&slot);
            if l.focus.as_deref() == Some(a.id.as_str()) && score["exact"] == json!(true) {
                l.focus = None;
            }
        });
        std::thread::sleep(pause());
    }
}

/// Adds a job just done to the state's `done`, keeping the last few.
pub(crate) fn push_done(v: &mut Value, mut d: Value) {
    let seq = v["seq"].as_u64().unwrap_or(0) + 1;
    v["seq"] = json!(seq);
    d["seq"] = json!(seq);
    let mut done = v["done"].as_array().cloned().unwrap_or_default();
    done.push(d);
    let over = done.len().saturating_sub(DONE);
    done.drain(..over);
    v["done"] = json!(done);
}

fn history_file(run: &Run) -> PathBuf {
    run.dir.join("background").join("history.jsonl")
}

/// The rounds so far, oldest first.
pub(crate) fn rounds(run: &Run) -> Vec<Value> {
    std::fs::read_to_string(history_file(run))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A refine round on one clip: gameprinter's refinement loop, `gp model
/// refine GAME --lockstep --clip ID`, run on the working copy by this gp,
/// holding the corpus's other clips as it does. Its progress lines are the
/// job's `target` while it runs; the values it writes are the round's
/// changes, and a round that writes none leaves the clip stuck on this
/// model, with the questions the loop asked in the copy's questions.toml.
fn refine(ctx: &Ctx, run: &Run, slot: usize, a: &Active, model: &str) {
    use std::io::BufRead as _;
    emit(
        &run.name,
        "refine.start",
        json!({"clip": a.id, "name": a.name}),
    );
    let said = |line: &str| {
        if let Some(p) = line
            .strip_prefix("progress: ")
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
        {
            let bars = with_live(run, |l| {
                l.flight.get_mut(&slot).map(|f| {
                    fold_progress(&mut f.bars, &p);
                    f.bars.clone()
                })
            });
            if let Some(mut b) = bars {
                b["clip"] = json!(a.id);
                b["name"] = json!(a.name);
                b["slot"] = json!(slot);
                emit(&run.name, "refine.progress", b);
            }
            return;
        }
        let line = line
            .strip_prefix(&format!("{}: ", a.id))
            .unwrap_or(line)
            .to_owned();
        emit(
            &run.name,
            "refine.progress",
            json!({"clip": a.id, "says": line}),
        );
        with_live(run, |l| {
            if let Some(f) = l.flight.get_mut(&slot) {
                f.says = line;
            }
        });
    };
    let outcome = (|| -> Result<Value, String> {
        let copy = run.ensure_copy().map_err(|e| e.to_string())?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut cmd = std::process::Command::new(exe);
        cmd.args(["--json", "model", "refine"])
            .arg(&run.game)
            .arg("--lockstep")
            .arg("--progress-json")
            .arg("--model")
            .arg(&copy)
            .args(["--clip", &a.id])
            .args(["--run", &format!("serve-{}", run.name)]);
        if let Some(m) = run.rebuild(ctx).module {
            cmd.arg("--module").arg(m);
        }
        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut tail = Vec::new();
        if let Some(err) = child.stderr.take() {
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                said(&line);
                tail.push(line);
            }
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(&text).map_err(|_| {
            let t = tail.join("\n");
            if t.contains("unrecognized subcommand") {
                "this gp has no `gp model refine` yet: it comes with the refine port".to_owned()
            } else {
                t.lines().rev().take(3).collect::<Vec<_>>().join("; ")
            }
        })
    })();
    let n = rounds(run)
        .iter()
        .filter(|r| r["round"].is_number())
        .count()
        + 1;
    let round = match &outcome {
        Ok(j) => {
            let changes: Vec<Value> = j["written"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|w| json!({"param": w["param"], "old": w["old"], "new": w["new"]}))
                .collect();
            let fixed = j["exact"]["after"].as_i64().unwrap_or(0)
                - j["exact"]["before"].as_i64().unwrap_or(0);
            let stuck = changes.is_empty();
            let says = if stuck {
                format!(
                    "Round {n}: no numbers make {} match; {} questions asked.",
                    a.name, j["questions"]
                )
            } else {
                format!(
                    "Round {n}: {}; {} clips more exact.",
                    changes
                        .iter()
                        .map(|c| format!(
                            "{} {} -> {}",
                            c["param"].as_str().unwrap_or(""),
                            c["old"].as_str().unwrap_or(""),
                            c["new"].as_str().unwrap_or("")
                        ))
                        .collect::<Vec<_>>()
                        .join(", "),
                    fixed.max(0)
                )
            };
            json!({"round": n, "when_ms": now_ms(), "clip": a.id, "clip_name": a.name,
                   "changes": changes, "fixed": fixed.max(0), "worse": [], "stuck": stuck,
                   "passes": !stuck, "says": says})
        }
        Err(e) => json!({"when_ms": now_ms(), "clip": a.id, "clip_name": a.name, "stuck": true,
                         "says": format!("Couldn't refine {}: {e}", a.name)}),
    };
    if let Some(d) = history_file(run).parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(history_file(run))
    {
        use std::io::Write as _;
        let _ = writeln!(f, "{round}");
    }
    emit(&run.name, "refine.round", round.clone());
    let _ = update_saved(run, |v| {
        if round["stuck"] == json!(true) {
            v["stuck"][&a.id] = json!(model);
        }
        if let Ok(j) = &outcome {
            for q in j["clips"][0]["questions"].as_array().into_iter().flatten() {
                if let Some(id) = q["id"].as_str().or(q.as_str()) {
                    v["asked_by"][id] = json!(a.id);
                }
            }
        }
        push_done(
            v,
            json!({"kind": "refine", "clip": a.id, "name": a.name,
                   "after": v["scores"][&a.id]["score"], "exact": false,
                   "error": outcome.as_ref().err(), "says": round["says"]}),
        );
    });
}

/// A refine round's progress event (`gp model refine --progress-json`)
/// folded into its bars, in the shape the live card reads
/// (game-printer's editor/src/backfold.ts, `BgJob`): `pass`, `impact_k`
/// of `impact_n`, `fine_k` on `fine_knobs`, `base` and `best` (the
/// window's splits at the start and the fewest a trial reached), `kept`
/// (values written), `trying` and `target`.
pub(crate) fn fold_progress(b: &mut Value, p: &Value) {
    if !b.is_object() {
        *b = json!({});
    }
    let n = |v: &Value, k: &str| v[k].as_u64().unwrap_or(0);
    if let Some(w) = p.get("window") {
        b["window"] = w.clone();
        if let Some(r) = p["round"].as_u64() {
            b["round"] = json!(r);
        }
    } else if let Some(t) = p.get("target") {
        b["target"] = json!({"says": t["says"]});
    } else if let Some(pass) = p.get("pass") {
        let base = pass["base"].clone();
        match pass["name"].as_str() {
            Some("impact") => {
                b["pass"] = json!("impact");
                b["impact_n"] = json!(n(pass, "trials"));
                b["impact_k"] = json!(0);
                b["fine_k"] = json!(0);
                b["fine_knobs"] = json!(0);
                b["fine_on"] = json!(0);
                b["fine_params"] = json!([]);
                b["kept"] = b["kept"].as_u64().map_or(json!(0), |k| json!(k));
                b["base"] = base.clone();
                b["best"] = base;
            }
            Some("fine") => {
                b["pass"] = json!("fine");
                b["fine_knobs"] = json!(n(pass, "knobs"));
                b["fine_k"] = json!(0);
                b["fine_on"] = json!(0);
                b["fine_params"] = json!([]);
                if b["base"].is_null() {
                    b["base"] = base;
                }
            }
            _ => {}
        }
    } else if let Some(t) = p.get("trying") {
        if t["pass"] == "fine" {
            b["fine_k"] = json!(n(b, "fine_k") + 1);
            // Which of the fine pass's numbers it is on: one bar step each.
            let mut seen: Vec<Value> = b["fine_params"].as_array().cloned().unwrap_or_default();
            if !seen.contains(&t["param"]) {
                seen.push(t["param"].clone());
            }
            b["fine_on"] = json!(seen.len());
            b["fine_params"] = json!(seen);
        } else {
            b["impact_k"] = json!(n(b, "impact_k") + 1);
        }
        if let Some(s) = t["split"].as_u64()
            && b["best"].as_u64().is_none_or(|best| s < best)
        {
            b["best"] = json!(s);
        }
        b["trying"] = json!({"param": t["param"], "value": t["value"], "words": t["words"],
                             "pass": t["pass"], "split": t["split"], "curve": t["curve"]});
    } else if let Some(i) = p.get("impact") {
        b["impact"] = json!({"says": i["says"]});
    } else if p.get("round").is_some_and(Value::is_object) {
        b["kept"] = json!(n(b, "kept") + 1);
    }
}

/// The working copy's questions as the page reads them: each with its
/// clip (the one whose round asked it, when known), its choices (the
/// candidates) and its answer.
fn questions(run: &Run) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(run.copy_dir().join("questions.toml")) else {
        return Vec::new();
    };
    let Ok(t) = toml::from_str::<toml::Table>(&text) else {
        return Vec::new();
    };
    let v = saved(run);
    let show = |c: &toml::Value| {
        ["says", "label", "value", "text"]
            .iter()
            .find_map(|k| {
                c.get(*k)
                    .map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_owned))
            })
            .unwrap_or_else(|| c.to_string())
    };
    t.get("question")
        .and_then(|q| q.as_array())
        .into_iter()
        .flatten()
        .map(|q| {
            let s = |k: &str| q.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned();
            let id = s("id");
            let choices: Vec<String> = q
                .get("candidate")
                .and_then(|c| c.as_array())
                .map(|c| c.iter().map(show).collect())
                .unwrap_or_default();
            let answer = q
                .get("answer")
                .and_then(|x| x.as_integer())
                .and_then(|i| choices.get(i as usize).cloned());
            let clip = v["asked_by"][&id].as_str().unwrap_or("").to_owned();
            json!({"id": id, "clip": clip, "clip_name": clip, "ask": s("text"),
                   "choices": choices, "detail": s("settle"), "model": "",
                   "answer": answer, "kind": s("kind"), "path": s("path")})
        })
        .collect()
}

/// Scores one clip and says how it stands, as the strip reads a score.
fn score_clip(ctx: &Ctx, run: &Run, sc: &crate::clip_score::Ctx, a: &Active, model: &str) -> Value {
    score_clip_in(
        ctx,
        run,
        sc,
        &a.id,
        model,
        &run.dir.join("background").join("scores"),
    )
}

/// Scores one clip on an opened model, its files under `root` (an
/// attempt's scores go to the attempt's folder), and says how it stands.
pub(crate) fn score_clip_in(
    ctx: &Ctx,
    run: &Run,
    sc: &crate::clip_score::Ctx,
    id: &str,
    model: &str,
    root: &std::path::Path,
) -> Value {
    let failed = |e: String| {
        json!({"model": model, "score": 0, "permille": 0, "exact": false,
               "error": e, "says": format!("can't score it: {e}")})
    };
    let (log, at, _, _) = match super::scenarios::clip_input(ctx, run, id) {
        Ok(x) => x,
        Err(f) => return failed(f.message),
    };
    let out = root.join(crate::clipset::file_key(id));
    let j = match crate::clip_score::score_one(sc, id, &log, at, &out) {
        Ok(s) => s.json,
        Err(e) => return failed(format!("{e:#}")),
    };
    let t = j["straight"]["together"].as_u64().unwrap_or(0);
    let n = j["straight"]["frames"].as_u64().unwrap_or(0);
    let permille = (t * 1000).checked_div(n).unwrap_or(0);
    let exact = n > 0 && t == n;
    let first = &j["first"];
    let drift = first["frame"].as_u64();
    let drift_path = first["paths"][0].as_str().map(str::to_owned);
    let lasting = &j["first_lasting"]["paths"][0];
    let says = match (drift, &drift_path) {
        _ if n == 0 => "no frame compared".to_owned(),
        _ if exact => format!("together on all {n} frames"),
        (Some(f), Some(p)) => {
            let vals = if lasting["path"].as_str() == Some(p.as_str()) {
                format!(
                    " (original {}, rebuilt {})",
                    lasting["original"].as_str().unwrap_or("?"),
                    lasting["model"].as_str().unwrap_or("?")
                )
            } else {
                String::new()
            };
            format!("together on {t} of {n} frames; {p} wrong from frame {f}{vals}")
        }
        _ => format!("together on {t} of {n} frames"),
    };
    let tracks: Vec<Value> = match (drift, &drift_path) {
        (Some(f), Some(p)) if !exact => vec![json!({
            "track": p.split('.').next().unwrap_or(p),
            "frame": f,
            "path": p,
            "says": format!("wrong from frame {f} ({p})"),
        })],
        _ => Vec::new(),
    };
    json!({
        "model": model,
        "score": permille,
        "permille": permille,
        "exact": exact,
        "drift": if exact { Value::Null } else { json!(drift) },
        "drift_path": drift_path,
        "says": says,
        "tracks": tracks,
        "error": Value::Null,
    })
}

/// The state as the page reads it, for `gp capture background`.
pub(crate) fn state_view(ctx: &Ctx, run: &Run) -> Value {
    view(ctx, run)
}

/// The state as the page reads it.
fn view(ctx: &Ctx, run: &Run) -> Value {
    ensure_workers(run);
    let v = saved(run);
    let model = model_key(ctx, run);
    let active = active_now(ctx, run);
    let ids: Vec<&str> = active.iter().map(|a| a.id.as_str()).collect();
    let mut scores = serde_json::Map::new();
    for id in &ids {
        if let Some(s) = v["scores"].get(*id) {
            scores.insert((*id).to_owned(), s.clone());
        }
    }
    let scored = scores.len();
    let exact = scores
        .values()
        .filter(|s| s["exact"] == json!(true))
        .count();
    let on_model = scores
        .values()
        .filter(|s| s["model"].as_str() == Some(model.as_str()))
        .count();
    let stored = v["scores"].as_object().map_or(0, serde_json::Map::len);
    let (enabled, workers, agents, focus, flight, error) = with_live(run, |l| {
        (
            l.enabled,
            l.workers,
            l.agents,
            l.focus.clone(),
            l.flight.clone(),
            l.error.clone(),
        )
    });
    let jobs: Vec<Value> = flight
        .iter()
        .map(|(slot, f)| {
            let mut j = json!({"kind": f.kind, "clip": f.clip, "name": f.name, "model": f.model,
                   "slot": slot, "k": f.k, "n": f.n, "round": rounds(run).len() + 1,
                   "secs": f.since.elapsed().as_secs(),
                   "target": if f.says.is_empty() { Value::Null } else { json!({"says": f.says}) }});
            for (k, v) in f.bars.as_object().into_iter().flatten() {
                if k == "round" || k == "window" {
                    continue;
                }
                if k == "target" && v["says"].is_null() {
                    continue;
                }
                j[k] = v.clone();
            }
            j
        })
        .collect();
    let would = ids.len() - on_model;
    let says = if !enabled {
        if would == 0 {
            format!(
                "Paused. All {} active clips are scored on the model as it stands.",
                ids.len()
            )
        } else {
            format!("Paused. A start would score {would} clips.")
        }
    } else if !jobs.is_empty() {
        format!("Scoring {} of {} clips at once.", jobs.len(), ids.len())
    } else if would == 0 {
        format!(
            "Every active clip is scored on the model as it stands: {exact} of {} match.",
            ids.len()
        )
    } else {
        "Waiting for a worker.".into()
    };
    json!({
        "run": run.name,
        "enabled": enabled,
        "working": !jobs.is_empty(),
        "recording": "corpus",
        "scores": scores,
        "questions": questions(run),
        "round": rounds(run).iter().filter(|r| r["round"].is_number()).count(),
        "rounds": rounds(run).len(),
        "job": jobs.first().cloned(),
        "jobs": jobs,
        "done": v["done"].as_array().cloned().unwrap_or_default(),
        "says": says,
        "error": error,
        "focus": focus,
        "workers": workers,
        "max_workers": max_workers(),
        "agents": agents,
        "max_agents": MAX_AGENTS,
        "active": ids,
        "names": active.iter().map(|a| (a.id.clone(), json!(a.name))).collect::<serde_json::Map<_, _>>(),
        "totals": {"active": ids.len(), "scored": scored, "exact": exact, "stored": stored},
        "start_would": {"would_score": would, "pool": ids.len()},
        "stuck": v["stuck"].clone(),
        "clip_agents": super::clip_agent::live_json(run),
        "agents_idle": super::clip_agent::idle_reason(ctx, run),
        "queued": queued(&v),
        "holds": [],
        "model": model,
    })
}

fn state(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    Ok(Response::json(&view(ctx, &run)))
}

fn start(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let run = run_dir(ctx, params)?;
    let v: Value = if req.body.is_empty() {
        json!({})
    } else {
        super::body(req)?
    };
    let clip = v["clip"].as_str().map(str::to_owned);
    with_live(&run, |l| {
        l.enabled = true;
        l.error = None;
        if clip.is_some() {
            l.focus.clone_from(&clip);
        }
    });
    record_live(&run, true);
    emit(&run.name, "queue.on", json!({"clip": clip}));
    // "Fix this clip" on a clip no numbers fix on this model: the clip
    // agent takes it (game-printer's, docs/agents.md, "The clip agent").
    let mut out = view(ctx, &run);
    if let Some(c) = &clip
        && out["stuck"][c].as_str() == out["model"].as_str()
    {
        out["agent"] = super::clip_agent::start_agent(ctx, &run, c)
            .unwrap_or_else(|e| json!({"started": false, "why": format!("{e:#}")}));
    }
    Ok(Response::json(&out))
}

fn stop(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let run = run_dir(ctx, params)?;
    with_live(&run, |l| l.enabled = false);
    record_live(&run, false);
    emit(&run.name, "queue.off", json!({}));
    Ok(Response::json(&view(ctx, &run)))
}

fn workers(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    if req.method == "POST" {
        let v: Value = super::body(req)?;
        let w = v["workers"]
            .as_u64()
            .ok_or_else(|| Fail::bad_request("give {workers}"))? as usize;
        let w = w.clamp(1, max_workers());
        with_live(&run, |l| l.workers = w);
        update_saved(&run, |v| v["workers"] = json!(w))?;
        ensure_workers(&run);
    }
    let w = with_live(&run, |l| l.workers);
    Ok(Response::json(&json!({"workers": w, "max": max_workers()})))
}

fn agents(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    if req.method == "POST" {
        let v: Value = super::body(req)?;
        let a = v["agents"]
            .as_u64()
            .ok_or_else(|| Fail::bad_request("give {agents}"))? as usize;
        let a = a.min(MAX_AGENTS);
        with_live(&run, |l| l.agents = a);
        update_saved(&run, |v| v["agents"] = json!(a))?;
    }
    let a = with_live(&run, |l| l.agents);
    Ok(Response::json(&json!({"agents": a, "max": MAX_AGENTS})))
}

/// The clips put in the queue by hand ("add to the queue" on the page):
/// scored and refined before the rest, in the order they were added.
fn queue(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    if req.method == "POST" {
        let v: Value = super::body(req)?;
        let clip = v["clip"]
            .as_str()
            .ok_or_else(|| Fail::bad_request("give {clip}"))?
            .to_owned();
        let remove = v["remove"] == json!(true);
        if !remove && !active_now(ctx, &run).iter().any(|a| a.id == clip) {
            return Err(Fail::not_found(format!(
                "{clip} isn't one of the run's active clips (played, or the corpus's, and not skipped)"
            )));
        }
        update_saved(&run, |s| {
            let mut q = queued(s);
            q.retain(|c| c != &clip);
            if !remove {
                q.push(clip.clone());
            }
            s["queued"] = json!(q);
        })?;
        emit(
            &run.name,
            if remove {
                "queue.removed"
            } else {
                "queue.added"
            },
            json!({"clip": clip}),
        );
    }
    let v = saved(&run);
    Ok(Response::json(&json!({"queued": queued(&v)})))
}

fn rescore(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let run = run_dir(ctx, params)?;
    update_saved(&run, |v| {
        if let Some(m) = v["scores"].as_object_mut() {
            for s in m.values_mut() {
                s["model"] = json!("");
            }
        }
    })?;
    Ok(Response::json(&view(ctx, &run)))
}

fn history(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    Ok(Response::json(&json!({"rounds": rounds(&run)})))
}

fn answer(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let run = run_dir(ctx, params)?;
    let v: Value = super::body(req)?;
    let (id, choice) = (
        v["question"].as_str().unwrap_or(""),
        v["choice"].as_str().unwrap_or(""),
    );
    let q = questions(&run)
        .into_iter()
        .find(|q| q["id"] == json!(id))
        .ok_or_else(|| Fail::not_found(format!("no question {id}")))?;
    let k = q["choices"]
        .as_array()
        .and_then(|c| c.iter().position(|x| x == &json!(choice)))
        .ok_or_else(|| {
            Fail::bad_request(format!("{choice:?} isn't one of the question's choices"))
        })?;
    let exe = std::env::current_exe()?;
    let out = std::process::Command::new(exe)
        .args(["model", "questions"])
        .arg(run.copy_dir())
        .arg("--answer")
        .arg(format!("{id}={}", k + 1))
        .output()?;
    if !out.status.success() {
        return Err(Fail::conflict(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(history_file(&run))
        .and_then(|mut f| {
            use std::io::Write as _;
            writeln!(
                f,
                "{}",
                json!({"when_ms": now_ms(), "clip": q["clip"], "answer": id, "choice": choice,
                       "says": format!("Answered {id}: {choice}.")})
            )
        });
    Ok(Response::json(&view(ctx, &run)))
}

fn verify(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    run_dir(ctx, params)?;
    Err(Fail::conflict(
        "verifying the ledger isn't in gameprinter's gp serve yet; gp corpus score scores every clip again from the ROM",
    ))
}

fn curve(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    run_dir(ctx, params)?;
    Err(Fail::not_found(
        "a clip's curves aren't in gameprinter's gp serve yet",
    ))
}

/// The fidelity line from the scores alone: behaviour is the mean of the
/// scored active clips' frames together. game-printer's fidelity also
/// weighed coverage and the picture (its serve/fidelity.rs), which aren't
/// measured here yet, so they stay at nothing and the line says so.
fn fidelity(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let v = saved(&run);
    let active = active_now(ctx, &run);
    let scored: Vec<&Value> = active
        .iter()
        .filter_map(|a| v["scores"].get(&a.id))
        .filter(|s| s["error"].is_null())
        .collect();
    let n = scored.len() as u64;
    let exact = scored.iter().filter(|s| s["exact"] == json!(true)).count();
    let ppm = scored
        .iter()
        .map(|s| s["permille"].as_u64().unwrap_or(0) * 1000)
        .sum::<u64>()
        .checked_div(n)
        .unwrap_or(0);
    let score = ppm / 10_000;
    let says = format!(
        "Behaviour {score} / 100 from {n} of {} clips scored, {exact} exact. Coverage and the picture aren't measured in gameprinter's gp serve yet.",
        active.len()
    );
    Ok(Response::json(&json!({
        "version": "serve-behaviour-0",
        "score": score,
        "behaviour": {"score": score, "ppm": ppm, "clips": n, "exact": exact},
        "coverage": {"score": 0, "ppm": 0},
        "picture": Value::Null,
        "confidence": {"level": "low", "says": "behaviour alone, from the clips scored so far"},
        "holding": [],
        "says": says,
        "trend": "",
        "history": [],
    })))
}
