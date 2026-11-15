//! The person's answer to a sound change the agent proposes for their
//! copy (gp_mcp::sound_proposal, issue #4), as serve/pictures.rs is for a
//! picture. The page shows the oldest open proposal as a card from the
//! status line's answer (`waits_for_yes`, `what: "sound"`); these routes
//! give its two clips and write the answer.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/sounds` | `{open, recent}`: the open proposals, oldest first, and the last 20 answered |
//! | `GET /api/sounds/{id}/{which}` | The proposal's `old.wav` (the song now) or `new.wav` (with the change) |
//! | `POST /api/sounds/{id}` | `{answer: "accept" \| "reject" \| "undo", words?}`: ACCEPT, REJECT with what to change, or PUT THE OLD ONE BACK |

use gp_mcp::sound_proposal::{self as store, Answer};
use serde_json::{Value, json};

use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/sounds",
            about: "The sound changes the agent proposes for the person's copy: open ones and the last answered",
            handler: list,
        },
        Route {
            path: "/api/sounds/{id}/{which}",
            about: "A proposal's old.wav (the song now) or new.wav (with the change)",
            handler: clip,
        },
        Route {
            path: "/api/sounds/{id}",
            about: "POST {answer: accept|reject|undo, words?}: the person's answer to a proposed sound change",
            handler: answer,
        },
    ]
}

fn list(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    let home = &ctx.paths.home;
    let recent: Vec<Value> = store::all(home)
        .into_iter()
        .filter(|v| v["state"] != json!("open"))
        .take(20)
        .collect();
    Ok(Response::json(&json!({
        "open": store::queue(home),
        "recent": recent,
    })))
}

fn clip(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let id = params.get("id").map(String::as_str).unwrap_or("");
    let which = params.get("which").map(String::as_str).unwrap_or("");
    let which = which.strip_suffix(".wav").unwrap_or(which);
    let bytes = store::wav(&ctx.paths.home, id, which)
        .ok_or_else(|| Fail::not_found(format!("no sound {which} for proposal {id}")))?;
    Ok(Response::bytes("audio/wav", bytes))
}

fn answer(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let id = params.get("id").map(String::as_str).unwrap_or("");
    let v: Value = super::body(req)?;
    let a = match v["answer"].as_str() {
        Some("accept") => Answer::Accept,
        Some("reject") => Answer::Reject,
        Some("undo") => Answer::Undo,
        _ => return Err(Fail::bad_request("answer is accept, reject or undo")),
    };
    let done =
        store::decide(&ctx.paths.home, id, a, v["words"].as_str()).map_err(Fail::conflict)?;
    Ok(Response::json(&done))
}
