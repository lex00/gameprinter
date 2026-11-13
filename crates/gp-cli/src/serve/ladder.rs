//! The game view's Scenarios section (docs/inspector.md, "Scenarios"):
//! the scenario ladder of `gp interp scenarios` on a run, scored, refined
//! on one scenario or worked through in auto mode, each as a job that
//! writes its state as it goes, so the page polls one state with a
//! sentence saying what is happening and what comes next.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/scenarios?recording=NAME` | The state: `idle`, `running` (with `progress`), `done` (the `ladder`: every scenario's state, result, scores and history, the `next` one and the first failing), `stuck` or `failed`; and `says` |
//! | `POST /api/capture/{run}/scenarios` | `{recording, at?, tier?, bar?}`: scores the ladder from the original's frame `at`, or from a quiet frame found in the input |
//! | `POST /api/capture/{run}/scenarios/refine` | `{recording, scenario, at?, bar?}`: the refinement loop on the run's working copy narrowed to one scenario; its rounds stream into the state |
//! | `POST /api/capture/{run}/scenarios/auto` | `{recording, at?, bar?}`: works through the open scenarios on its own |
//! | `POST /api/capture/{run}/scenarios/stop` | Says whether a job is running; gameprinter's scenario loop has no stop flag, so it ends after its rounds |
//!
//! Ported from game-printer's serve/scenarios.rs (wave 8); the routes for
//! a clip played in free play and a clip's input are super::scenarios.
//! What gameprinter's layout changed: a run is a game; its recordings are
//! super::lockstep::recordings; the state, cache and history are in the
//! run's `$GP_HOME/work/serve/GAME/scenarios/`; the played scenarios are
//! the run's free play clips; the refine and auto jobs write the run's
//! working copy (`model/` beside it, made from the game's folder when
//! there is none, as the background work makes it); there is no stop flag
//! or run recorder (crate::scenario_refine).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

use super::game::{Run, is_name, rom_of, run_dir};
use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};
use crate::scenario::{self as sc, Bench};
use crate::scenario_refine::RefineArgs;
use gp_interp::scenario::{Bar, Scenario};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/scenarios/refine",
            about: "POST {recording, scenario, at?, bar?}: the refinement loop on one scenario of the run's working copy, its rounds streamed into the scenarios state",
            handler: refine,
        },
        Route {
            path: "/api/capture/{run}/scenarios/auto",
            about: "POST {recording, at?, bar?}: works through the open scenarios on its own, refining each until it passes",
            handler: auto,
        },
        Route {
            path: "/api/capture/{run}/scenarios/stop",
            about: "POST: whether a scenario job is running on the run (it ends after its rounds: no stop flag in gameprinter)",
            handler: stop,
        },
        Route {
            path: "/api/capture/{run}/scenarios",
            about: "GET ?recording=NAME: the scenario ladder's state (idle, running with progress, done with every scenario's state and scores, and the next one); POST {recording, at?, tier?, bar?} scores it",
            handler: score,
        },
    ]
}

/// The runs a job is running on now.
static RUNNING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn folder(run: &Run) -> PathBuf {
    run.dir.join("scenarios")
}

fn state_file(run: &Run, recording: &str) -> PathBuf {
    folder(run).join(format!("{recording}.json"))
}

fn write_state(file: &Path, v: &Value) {
    if let Some(p) = file.parent() {
        let _ = fs::create_dir_all(p);
    }
    // Written whole and moved over, so a poll never reads half of it.
    let tmp = file.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_vec(v).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, file);
    }
}

fn recording(v: &Value) -> Result<String, Fail> {
    let n = v["recording"]
        .as_str()
        .ok_or_else(|| Fail::bad_request("give the recording the scenarios start from"))?;
    if !is_name(n) {
        return Err(Fail::not_found("no such recording"));
    }
    Ok(n.to_owned())
}

fn bar(v: &Value) -> Result<Bar, Fail> {
    Bar::parse(v["bar"].as_str().unwrap_or("exact")).map_err(Fail::bad_request)
}

/// The run's played scenarios: the clips of the free play sessions on it,
/// with their save points' logs found under `$GP_HOME`.
fn played_of(ctx: &Ctx, run: &Run) -> Vec<Scenario> {
    let mut out = super::freeplay::clips_for_run(&ctx.paths.home, &run.name);
    sc::resolve_logs(&mut out, &ctx.paths.home);
    out
}

/// The bench on the run: its working copy (made when `copy` and there is
/// none) or the model it plays, its ROM and the recording's input.
fn bench(ctx: &Ctx, run: &Run, name: &str, copy: bool) -> Result<Bench, Fail> {
    let mdir = if copy {
        run.ensure_copy()?
    } else {
        run.model_dir()
    };
    let log = super::lockstep::input_of(ctx, run, name)?;
    let rom = rom_of(ctx, run)?;
    let text = String::from_utf8(log).map_err(|_| Fail::bad_request("the input log isn't text"))?;
    let rb = run.rebuild(ctx);
    let assets = rb.assets_for(&ctx.paths, &rom)?;
    let (source, assets) =
        crate::hosted::Source::open_dir(&mdir, Some(&assets), &ctx.paths, rb.module.as_deref())?;
    Bench::new(
        &mdir,
        &assets,
        rom.to_vec(),
        text,
        name,
        &folder(run).join("cache"),
        std::sync::Arc::new(source),
    )
    .map_err(Fail::from)
}

fn idle(name: &str) -> Value {
    json!({"recording": name, "state": "idle",
        "says": "Not run yet. Next: run the scenarios from a quiet moment the page finds in the recording."})
}

/// Starts a job unless one is running on the run; the job writes `file`.
fn spawn(
    run: &str,
    file: PathBuf,
    first: Value,
    job: impl FnOnce(&mut dyn FnMut(&Value)) -> anyhow::Result<Value> + Send + 'static,
) -> Result<Response, Fail> {
    {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if running.contains(run) {
            return Ok(Response::json(
                &json!({"run": run, "started": false, "why": "a scenarios job is running on this run"}),
            ));
        }
        running.insert(run.to_owned());
    }
    write_state(&file, &first);
    let run2 = run.to_owned();
    std::thread::spawn(move || {
        let mut state = first;
        let r = {
            let mut put = |p: &Value| {
                if let Some(o) = p.as_object() {
                    for (k, v) in o {
                        state[k] = v.clone();
                    }
                }
                write_state(&file, &state);
            };
            job(&mut put)
        };
        match r {
            Ok(v) => {
                if let Some(o) = v.as_object() {
                    for (k, v) in o {
                        state[k] = v.clone();
                    }
                }
            }
            Err(e) => {
                state["state"] = json!("failed");
                state["error"] = json!(format!("{e:#}"));
                state["says"] = json!(format!("It failed: {e:#}"));
            }
        }
        state["progress"] = Value::Null;
        write_state(&file, &state);
        RUNNING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&run2);
    });
    Ok(Response::json(&json!({"run": run, "started": true})))
}

fn running(run: &str) -> bool {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(run)
}

fn score(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    if req.method == "GET" || req.method == "HEAD" {
        let name = req
            .param("recording")
            .ok_or_else(|| Fail::bad_request("name a recording with ?recording="))?;
        if !is_name(name) {
            return Err(Fail::not_found("no such recording"));
        }
        let mut v = fs::read(state_file(&run, name))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .unwrap_or_else(|| idle(name));
        // A job that died with the server is no longer running.
        if v["state"] == "running" && !running(&run.name) {
            v["state"] = json!("stopped");
            v["says"] = json!("Stopped when the server did. Next: run it again.");
        }
        return Ok(Response::json(&v));
    }
    post_only(req)?;
    let v: Value = super::body(req)?;
    let name = recording(&v)?;
    let at = v["at"].as_u64();
    let tier = v["tier"].as_u64().and_then(|t| u8::try_from(t).ok());
    let bar = bar(&v)?;
    let b = bench(ctx, &run, &name, false)?;
    let played = played_of(ctx, &run);
    let first = json!({"recording": name, "job": "score", "state": "running",
        "says": "Finding a quiet moment to start from.", "progress": null});
    spawn(&run.name, state_file(&run, &name), first, move |put| {
        let (model, text) = b.model()?;
        let started = b.started(&model, &text, at)?;
        let list = sc::scenarios(&model, &b.hosted.exts, &played, tier);
        let results = sc::score(&b, &model, &text, &started, &list, true, &mut |k, n, s| {
            put(
                &json!({"progress": {"k": k + 1, "n": n, "scenario": s.id, "name": s.name},
                "says": format!("Testing: {} ({} of {n})", s.name, k + 1)}),
            );
        })?;
        let history = sc::record_history(&b, &started, &text, "scored", &results);
        let ladder = sc::ladder_json(
            &model,
            &started,
            &list,
            &results,
            bar,
            &[],
            &history,
            &b.input,
        );
        Ok(json!({"state": "done", "says": ladder["says"], "ladder": ladder}))
    })
}

/// A refine's progress into the state: what it tries, and each round.
fn stream(p: &Value, id: &str, rounds: &mut Vec<Value>, put: &mut dyn FnMut(&Value)) {
    if let Some(t) = p.get("trying") {
        put(
            &json!({"refine": {"scenario": id, "rounds": rounds, "trying": t},
            "says": format!("Trying {} = {} ({}).", t["param"].as_str().unwrap_or(""), t["value"].as_str().unwrap_or(""), gp_interp::scenario::short(t["mechanic"].as_str().unwrap_or("")))}),
        );
    } else if let Some(r) = p.get("round").filter(|r| r.is_object()) {
        rounds.push(r.clone());
        put(
            &json!({"refine": {"scenario": id, "rounds": rounds, "trying": null},
            "says": format!("Round {}: {} {} -> {}.", r["round"], r["param"].as_str().unwrap_or(""), r["old"].as_str().unwrap_or(""), r["new"].as_str().unwrap_or(""))}),
        );
    } else if let Some(w) = p.get("working") {
        put(&json!({"auto": w, "says": w["says"]}));
    }
}

fn first_state(run: &Run, name: &str, job: &str, says: String) -> (PathBuf, Value) {
    let file = state_file(run, name);
    let mut first = fs::read(&file)
        .ok()
        .and_then(|x| serde_json::from_slice::<Value>(&x).ok())
        .unwrap_or_else(|| idle(name));
    first["job"] = json!(job);
    first["state"] = json!("running");
    first["says"] = json!(says);
    (file, first)
}

fn refine(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    post_only(req)?;
    let v: Value = super::body(req)?;
    let name = recording(&v)?;
    let id = v["scenario"]
        .as_str()
        .ok_or_else(|| Fail::bad_request("give the scenario to refine on"))?
        .to_owned();
    let at = v["at"].as_u64();
    let bar = bar(&v)?;
    let b = bench(ctx, &run, &name, true)?;
    let played = played_of(ctx, &run);
    let (file, mut first) = first_state(&run, &name, "refine", format!("Refining on {id}."));
    first["refine"] = json!({"scenario": id, "rounds": [], "trying": null});
    spawn(&run.name, file, first, move |put| {
        let args = RefineArgs {
            at,
            tier: None,
            played: &played,
            bar,
            pool_only: false,
        };
        let mut rounds: Vec<Value> = Vec::new();
        let r = crate::scenario_refine::refine_one(&b, &args, &id, &mut |p| {
            stream(p, &id, &mut rounds, put);
        })?;
        Ok(
            json!({"state": if r["passes"] == true { "done" } else { "stuck" }, "says": r["says"],
            "refine": {"scenario": id, "rounds": rounds, "trying": null, "result": r},
            "ladder": r["ladder"]}),
        )
    })
}

fn auto(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    post_only(req)?;
    let v: Value = super::body(req)?;
    let name = recording(&v)?;
    let at = v["at"].as_u64();
    let bar = bar(&v)?;
    let b = bench(ctx, &run, &name, true)?;
    let played = played_of(ctx, &run);
    let (file, first) = first_state(
        &run,
        &name,
        "auto",
        "Auto: finding the next open sequence.".into(),
    );
    spawn(&run.name, file, first, move |put| {
        let args = RefineArgs {
            at,
            tier: None,
            played: &played,
            bar,
            pool_only: false,
        };
        let mut rounds: Vec<Value> = Vec::new();
        let mut current = String::new();
        let r = crate::scenario_refine::auto(&b, &args, &mut |p| {
            if let Some(w) = p.get("working") {
                current = w["scenario"].as_str().unwrap_or("").to_owned();
                rounds.clear();
            }
            stream(p, &current, &mut rounds, put);
        })?;
        Ok(
            json!({"state": r["state"], "says": r["says"], "auto": {"steps": r["steps"], "question": r["question"]},
            "ladder": r["ladder"]}),
        )
    })
}

fn stop(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    post_only(req)?;
    let busy = running(&run.name);
    Ok(Response::json(&json!({
        "run": run.name,
        "stopping": false,
        "running": busy,
        "why": "gameprinter's scenario loop has no stop flag: a job ends after its rounds",
    })))
}
