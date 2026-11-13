//! Pause, point and tell in the workspace (docs/point-and-tell.md).
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/point` | `?recording=NAME&frame=N&rect=x,y,w,h`: what's under the box in both games, named in plain words ([`super::pointing`]) |
//! | `GET /api/capture/{run}/requests` | The run's requests with their replies and status, and which are being explained |
//! | `POST /api/capture/{run}/requests/{n}/explain` | game-printer started `gp request explain RUN N` here; that command isn't in gameprinter yet, so this says so |
//!
//! A request itself is posted as an input (`POST /api/capture/{run}/input`
//! with kind `request`, super::edits), which [`post`] numbers and writes as
//! an event.
//!
//! Ported from game-printer's serve/point.rs (wave 8). What gameprinter's
//! layout changed: a run is a game, and its requests are kept in the folder
//! the agent's gp-mcp `requests` tool reads and answers them in,
//! `$GP_HOME/capture/GAME/` (super::edits::agent_dir). `gp request
//! explain` and the local agent that took an explained request
//! (serve/agent.rs) are not ported: an agent with gp mcp's `requests` tool
//! answers a request, and its reply shows under it.

use std::path::Path;
use std::sync::Mutex;

use serde_json::json;

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) const EXPLAIN: &str = "/api/capture/{run}/requests/{n}/explain";

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/point",
            about: "`?recording=NAME&frame=N&rect=x,y,w,h`: what's under a box of the screen in both games (entities, level cells, HUD, sprites), named in plain words",
            handler: point,
        },
        Route {
            path: "/api/capture/{run}/requests",
            about: "A run's point-and-tell requests with their replies and status",
            handler: requests,
        },
        Route {
            path: EXPLAIN,
            about: "POST: explain request n in the background (gp request explain, not in gameprinter yet: says so)",
            handler: explain,
        },
    ]
}

/// Held while a request or reply is numbered and written, so two don't
/// get one number.
static POST: Mutex<()> = Mutex::new(());

/// Numbers and appends a request from the workspace and writes its event.
pub(super) fn post(dir: &Path, input: gp_capture::Input) -> Result<serde_json::Value, Fail> {
    let _g = POST.lock().unwrap_or_else(|e| e.into_inner());
    let posted = gp_capture::requests::post(dir, input).map_err(|e| Fail {
        status: 422,
        message: e.to_string(),
    })?;
    Ok(json!({"inputs": posted.inputs, "input": posted.input, "event": posted.event}))
}

fn point(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = super::game::run_dir(ctx, params)?;
    let frame_text = req.param("frame").unwrap_or("");
    let frame: u64 = frame_text.parse().map_err(|_| {
        Fail::bad_request(format!("frame must be a whole number, not {frame_text:?}"))
    })?;
    let rect = super::pointing::parse_rect(
        req.param("rect")
            .ok_or_else(|| Fail::bad_request("give the rectangle: ?rect=x,y,w,h"))?,
    )
    .map_err(Fail::bad_request)?;
    let name = match req.param("recording") {
        Some(n) => n.to_owned(),
        None => super::lockstep::recordings(ctx, &run)
            .first()
            .and_then(|r| r["name"].as_str().map(str::to_owned))
            .ok_or_else(|| Fail::bad_request("the run has no recordings"))?,
    };
    let game = super::pointing::RunGame::of_run(ctx, &run, &name)?;
    let out = super::pointing::resolve(&ctx.paths, &game, &name, frame, rect)
        .map_err(|e| Fail::bad_request(format!("{e:#}")))?;
    Ok(Response::json(&out))
}

fn requests(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let dir = super::edits::agent_dir(ctx, params)?;
    let run = params.get("run").map(String::as_str).unwrap_or("");
    let all = if dir.is_dir() {
        gp_capture::requests::list(&dir).map_err(|e| anyhow::anyhow!("{e}"))?
    } else {
        Vec::new()
    };
    Ok(Response::json(
        &json!({"run": run, "requests": all, "explaining": [], "agent": []}),
    ))
}

fn explain(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    super::post_only(req)?;
    super::edits::agent_dir(ctx, params)?;
    Err(Fail {
        status: 501,
        message: "gp request explain isn't in gameprinter yet: an agent with gp mcp's requests tool (run = the game's folder) answers a request, and its reply shows under it".into(),
    })
}
