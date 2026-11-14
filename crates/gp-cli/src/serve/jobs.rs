//! The page's work buttons: a fixed table of actions, each one gp command
//! run through the same code as the agent's `gp_run` (gp_mcp::tools), so a
//! press gets what the agent's call gets and nothing of its own: the
//! policy row (a command that needs the person's yes waits for the yes
//! card), the plain line in the agent's log, the "running" record the
//! status line reads, and the change counter. Each call is marked as the
//! person's (`Started::Page`): its log line reads "You checked the game's
//! tests", a change it makes refreshes the page's views quietly instead of
//! saying the agent changed the game, and an action that only reads counts
//! no change at all.
//!
//! The page names an action, never a command: only the actions below run,
//! with the arguments written here. Each runs on a thread of its own, one
//! at a time per game and action, and its record is kept so the page (and
//! the agent, by reading the same file) sees how it ended.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/do/{which}` | The action's last record: `{state: never \| running \| done \| failed \| waits, ...}` |
//! | `POST /api/capture/{run}/do/{which}` | Starts it; 409 while it runs |
//!
//! | File | What |
//! |---|---|
//! | `$GP_HOME/work/serve/GAME/jobs/WHICH.json` | `{action, command, state, started_ms, ended_ms?, exit?, json?, error?, ask_id?}` |

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::game::{Run, run_named};
use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

/// One thing a page button may start.
pub(crate) struct Action {
    /// Its name in the route.
    pub name: &'static str,
    /// The gp command, as gp_run takes it.
    pub command: &'static str,
    /// Its arguments for a game, by name, as gp_commands gives them.
    pub args: fn(&str) -> Value,
    /// Seconds before gp_run stops it.
    pub timeout_s: u64,
    /// It changes nothing of the person's (what it writes is its own
    /// scratch), so it counts no change.
    pub reads_only: bool,
}

/// Every action the page can start. Nothing else runs from these routes.
pub(crate) const ACTIONS: &[Action] = &[
    // CHECK THE TESTS: every clip of the game's tests made again from the
    // person's copy of the game.
    Action {
        name: "verify",
        command: "corpus verify",
        args: |g| json!({"dir": g, "run": g}),
        timeout_s: 1800,
        reads_only: true,
    },
    // LET THE TOOLS TRY: the tools play each quest item themselves.
    Action {
        name: "triage",
        command: "mechanic triage",
        args: |g| json!({"game": g, "budget": 20}),
        timeout_s: 1800,
        reads_only: false,
    },
];

pub(super) fn routes() -> Vec<Route> {
    vec![Route {
        path: "/api/capture/{run}/do/{which}",
        about: "One of the page's work buttons (verify, triage): GET its last record, POST starts it through gp_run",
        handler: handle,
    }]
}

/// The actions running in this server now, by (game, action).
static BUSY: Mutex<BTreeSet<(String, &'static str)>> = Mutex::new(BTreeSet::new());

fn record_file(run: &Run, action: &str) -> PathBuf {
    run.dir.join("jobs").join(format!("{action}.json"))
}

fn now_ms() -> u64 {
    gp_mcp::asks::now_ms()
}

fn write(run: &Run, action: &str, v: &Value) {
    let f = record_file(run, action);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, v.to_string());
}

/// The action's last record. One left "running" by a server that has
/// stopped is "failed", in words.
pub(crate) fn read(run: &Run, action: &'static str) -> Value {
    let v: Value = std::fs::read(record_file(run, action))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({"action": action, "state": "never"}));
    let busy = BUSY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&(run.name.clone(), action));
    if v["state"] == json!("running") && !busy {
        let mut v = v;
        v["state"] = json!("failed");
        v["error"] = json!("the page's server stopped while it ran");
        return v;
    }
    v
}

/// The agent's MCP config for this server: the same gp, home and games.
fn config(ctx: &Ctx) -> Result<gp_mcp::Config, Fail> {
    let work = ctx.paths.home.join("work").join("mcp");
    std::fs::create_dir_all(&work)?;
    Ok(gp_mcp::Config {
        gp: std::env::current_exe()?,
        repo: ctx.repo.clone(),
        work,
        home: ctx.paths.home.clone(),
        rom: None,
        assets: None,
        games: Some(ctx.games.clone()),
    })
}

/// What gp_run answered, as the record keeps it.
pub(crate) fn ended(reply: &gp_mcp::Reply) -> Value {
    let j = &reply.json;
    if let Some(id) = j["ask_id"].as_str() {
        return json!({"state": "waits", "ask_id": id,
            "error": j["refused"].as_str().unwrap_or("it waits for your yes")});
    }
    if let Some(why) = j["refused"].as_str().or(j["error"].as_str()) {
        return json!({"state": "failed", "error": why});
    }
    if j.get("timed_out").is_some() {
        return json!({"state": "failed", "error": "it ran past its time limit and was stopped"});
    }
    // The command's own JSON, from the file gp_run saved when it was long.
    let out = match j.get("json") {
        Some(v) => v.clone(),
        None => j["cut"]["stdout_saved"]
            .as_str()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null),
    };
    let exit = j["exit"].as_i64().unwrap_or(2);
    let error = (exit >= 2).then(|| {
        out["error"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| gp_mcp::activity::reason(j["stderr"].as_str().unwrap_or("")))
            .unwrap_or_else(|| format!("it ended with code {exit}"))
    });
    json!({"state": if exit >= 2 { "failed" } else { "done" }, "exit": exit, "json": out, "error": error})
}

fn handle(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_named(ctx, params.get("run").map(String::as_str).unwrap_or(""))?;
    let which = params.get("which").map(String::as_str).unwrap_or("");
    let Some(action) = ACTIONS.iter().find(|a| a.name == which) else {
        return Err(Fail::not_found(format!(
            "no action {which}: the page can start {}",
            ACTIONS
                .iter()
                .map(|a| a.name)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    };
    match req.method.as_str() {
        "GET" | "HEAD" => Ok(Response::json(&read(&run, action.name))),
        "POST" => start(ctx, run, action),
        _ => Err(Fail {
            status: 405,
            message: "GET or POST".into(),
        }),
    }
}

fn start(ctx: &Ctx, run: Run, action: &'static Action) -> Result<Response, Fail> {
    let key = (run.name.clone(), action.name);
    if !BUSY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.clone())
    {
        return Err(Fail::conflict(format!(
            "{} is running already",
            action.name
        )));
    }
    let config = match config(ctx) {
        Ok(c) => c,
        Err(e) => {
            BUSY.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
            return Err(e);
        }
    };
    let started_ms = now_ms();
    let base = json!({"action": action.name, "command": action.command, "started_ms": started_ms});
    let mut running = base.clone();
    running["state"] = json!("running");
    write(&run, action.name, &running);
    let run = Arc::new(run);
    let r = run.clone();
    std::thread::spawn(move || {
        let args = json!({"command": action.command, "args": (action.args)(&r.name),
                          "timeout_s": action.timeout_s});
        let started = gp_mcp::tools::Started::Page {
            reads_only: action.reads_only,
        };
        let mut v = match gp_mcp::tools::call_by(&config, "gp_run", args, started) {
            Some(reply) => ended(&reply),
            None => json!({"state": "failed", "error": "gp_run isn't a tool of this build"}),
        };
        for (k, x) in base.as_object().into_iter().flatten() {
            v[k] = x.clone();
        }
        v["ended_ms"] = json!(now_ms());
        write(&r, action.name, &v);
        BUSY.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
    });
    Ok(Response::json(&running))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(j: Value) -> gp_mcp::Reply {
        gp_mcp::Reply::ok(j)
    }

    #[test]
    fn the_table_is_fixed_and_each_command_runs_from_a_tool_without_a_yes() {
        let names: Vec<_> = ACTIONS.iter().map(|a| a.name).collect();
        assert_eq!(names, ["verify", "triage"]);
        for a in ACTIONS {
            let row = gp_mcp::policy::row(a.command)
                .unwrap_or_else(|| panic!("{} has no policy row", a.command));
            let args = (a.args)("g");
            let given = |n: &str| args.get(n).is_some_and(|v| v != &json!(false));
            assert!(
                matches!(
                    gp_mcp::policy::decide(row, &given, &[]),
                    gp_mcp::policy::Decision::Run { .. }
                ),
                "{} would need a yes or be refused",
                a.command
            );
        }
    }

    #[test]
    fn a_reply_is_kept_in_words() {
        let done = ended(&reply(json!({"exit": 0, "json": {"ok": true}})));
        assert_eq!(done["state"], "done");
        assert_eq!(done["json"]["ok"], true);
        let check = ended(&reply(json!({"exit": 1, "json": {"ok": false}})));
        assert_eq!(check["state"], "done");
        assert_eq!(check["exit"], 1);
        let failed = ended(&reply(
            json!({"exit": 2, "stderr": "error: no ROM for donkey-kong in /roms"}),
        ));
        assert_eq!(failed["state"], "failed");
        assert!(
            failed["error"].as_str().unwrap().contains("no ROM"),
            "{failed}"
        );
        let waits = ended(&reply(json!({"ask_id": "a1", "refused": "needs a yes"})));
        assert_eq!(waits["state"], "waits");
        let refused = ended(&reply(json!({"refused": "Laya isn't answering"})));
        assert_eq!(refused["state"], "failed");
    }
}
