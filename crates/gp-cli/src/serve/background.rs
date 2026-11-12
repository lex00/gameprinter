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
//! clip's score files under `background/scores/`.
//!
//! What game-printer's queue also did and this doesn't yet: refine rounds
//! (fitting numbers on the clip that parts earliest), the questions they
//! ask, the clip agents, the grid, rescoring by what a change touches, the
//! ledger and its verify, and the curves. The routes for them answer in
//! words.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/background` | The state: on or paused, the jobs running now, the jobs just done, each active clip's score, the questions and `says` |
//! | `POST /api/capture/{run}/background/start`, `.../stop` | On (with `{clip}`, that clip first), or paused |
//! | `GET`, `POST /api/capture/{run}/background/workers` | `{workers, max}`; POST `{workers}` |
//! | `GET`, `POST /api/capture/{run}/background/agents` | `{agents, max}`; POST `{agents}`; kept, though no clip agent runs yet |
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
            about: "The refine rounds: none yet, since refining isn't here yet",
            handler: history,
        },
        Route {
            path: "/api/capture/{run}/background/answer",
            about: "Answers a question; there are none until refining comes",
            handler: answer,
        },
        Route {
            path: "/api/capture/{run}/background/verify",
            about: "Replays the run's ledger: not here yet",
            handler: verify,
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
fn max_workers() -> usize {
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
/// The server's context, for workers started from a handler.
static CTX: OnceLock<Arc<Ctx>> = OnceLock::new();
/// Keeps the server's context for the workers. Every run's queue starts
/// paused.
pub(crate) fn start_dispatcher(ctx: Arc<Ctx>) {
    let _ = CTX.set(ctx);
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
fn model_key(ctx: &Ctx, run: &Run) -> String {
    let mut h = blake3::Hasher::new();
    for f in [gp_model::MODEL_FILE, gp_interp::mapping::MAPPING_FILE] {
        h.update(&std::fs::read(run.game.join(f)).unwrap_or_default());
    }
    if let Some(m) = run.rebuild(ctx).module {
        h.update(&std::fs::read(m).unwrap_or_default());
    }
    h.finalize().to_hex()[..12].to_owned()
}

/// The next clip a worker takes: the focus first, then the first active
/// clip not scored on `model` and not running.
fn next(ctx: &Ctx, run: &Run, live: &Live, model: &str) -> Option<(Active, usize, usize)> {
    let active = active_now(ctx, run);
    let scores = saved(run)["scores"].clone();
    let running: Vec<&str> = live.flight.values().map(|f| f.clip.as_str()).collect();
    let n = active.len();
    let todo = |a: &Active| {
        !running.contains(&a.id.as_str()) && scores[&a.id]["model"].as_str() != Some(model)
    };
    if let Some(f) = &live.focus
        && let Some((k, a)) = active.iter().enumerate().find(|(_, a)| &a.id == f)
        && todo(a)
    {
        return Some((a.clone(), k + 1, n));
    }
    active
        .iter()
        .enumerate()
        .find(|(_, a)| todo(a))
        .map(|(k, a)| (a.clone(), k + 1, n))
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

/// One worker of a run's queue: takes the next clip while the queue is on
/// and its slot is under the run's workers, scores it and keeps the score.
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
            let (a, k, n) = next(ctx, &run, l, &model)?;
            l.flight.insert(
                slot,
                Flight {
                    clip: a.id.clone(),
                    name: a.name.clone(),
                    model: model.clone(),
                    k,
                    n,
                    since: Instant::now(),
                },
            );
            Some(a)
        });
        let Some(a) = job else {
            if with_live(&run, |l| !l.enabled || slot >= l.workers) {
                opened = None;
            }
            std::thread::sleep(Duration::from_millis(700));
            continue;
        };
        if opened.as_ref().is_none_or(|(k, _)| k != &model) {
            let rb = run.rebuild(ctx);
            let how = crate::clip_score::Open {
                module: rb.module.as_deref(),
                model: None,
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
                    continue;
                }
            }
        }
        let sc = &opened.as_ref().expect("opened").1;
        let score = score_clip(ctx, &run, sc, &a, &model);
        let _ = update_saved(&run, |v| {
            let before = v["scores"][&a.id]["score"].as_u64();
            if v["first"].get(&a.id).is_none() && score["error"].is_null() {
                v["first"][&a.id] = score["exact"].clone();
            }
            let mut s = score.clone();
            s["fixed"] = json!(v["first"][&a.id] == json!(false) && s["exact"] == json!(true));
            v["scores"][&a.id] = s;
            let mut done = v["done"].as_array().cloned().unwrap_or_default();
            let seq = v["seq"].as_u64().unwrap_or(0) + 1;
            v["seq"] = json!(seq);
            done.push(json!({
                "seq": seq,
                "kind": "score",
                "clip": a.id,
                "name": a.name,
                "before": before,
                "after": score["score"],
                "exact": score["exact"],
                "error": score["error"],
                "says": score["says"],
            }));
            let over = done.len().saturating_sub(DONE);
            done.drain(..over);
            v["done"] = json!(done);
        });
        with_live(&run, |l| {
            l.flight.remove(&slot);
            if l.focus.as_deref() == Some(a.id.as_str()) {
                l.focus = None;
            }
        });
        std::thread::sleep(pause());
    }
}

/// Scores one clip and says how it stands, as the strip reads a score.
fn score_clip(ctx: &Ctx, run: &Run, sc: &crate::clip_score::Ctx, a: &Active, model: &str) -> Value {
    let failed = |e: String| {
        json!({"model": model, "score": 0, "permille": 0, "exact": false,
               "error": e, "says": format!("can't score it: {e}")})
    };
    let (log, at, _, _) = match super::scenarios::clip_input(ctx, run, &a.id) {
        Ok(x) => x,
        Err(f) => return failed(f.message),
    };
    let out = run
        .dir
        .join("background")
        .join("scores")
        .join(crate::clipset::file_key(&a.id));
    let j = match crate::clip_score::score_one(sc, &a.id, &log, at, &out) {
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
            json!({"kind": "score", "clip": f.clip, "name": f.name, "model": f.model,
                   "slot": slot, "k": f.k, "n": f.n, "secs": f.since.elapsed().as_secs()})
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
        "questions": [],
        "round": 0,
        "rounds": 0,
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
        "totals": {"active": ids.len(), "scored": scored, "exact": exact, "stored": stored},
        "start_would": {"would_score": would, "pool": ids.len()},
        "stuck": {},
        "clip_agents": [],
        "agents_idle": "No clip agent runs in gameprinter's gp serve yet: the background work scores clips and doesn't fix numbers yet.",
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
            l.focus = clip;
        }
    });
    Ok(Response::json(&view(ctx, &run)))
}

fn stop(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let run = run_dir(ctx, params)?;
    with_live(&run, |l| l.enabled = false);
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
    run_dir(ctx, params)?;
    Ok(Response::json(&json!({"rounds": []})))
}

fn answer(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    run_dir(ctx, params)?;
    Err(Fail::not_found(
        "no such question: the background work asks none until refining comes to gameprinter's gp serve",
    ))
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
