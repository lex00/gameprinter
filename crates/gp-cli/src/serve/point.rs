//! Pause, point and tell in the workspace: what is under a box of the
//! screen, in words.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/point` | `?recording=NAME&frame=N&rect=x,y,w,h`: what's under the box in both games, named in plain words ([`super::pointing`]) |
//!
//! What the person says about the box is posted as an input
//! (`POST /api/capture/{run}/input` with kind `request`, super::edits) and
//! becomes their turn in the open mechanic's talk. The routes that listed
//! requests and asked for one to be explained (`GET requests`,
//! `POST requests/{n}/explain`) read a capture run's store, which nothing
//! made, and went on 2026-11-13.
//!
//! Ported from game-printer's serve/point.rs (wave 8); a run is a game.

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![Route {
        path: "/api/capture/{run}/point",
        about: "`?recording=NAME&frame=N&rect=x,y,w,h`: what's under a box of the screen in both games (entities, level cells, HUD, sprites), named in plain words",
        handler: point,
    }]
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
