//! The person's answer to a sprite edit the agent proposes with
//! `assets_edit` (gp_mcp::sprite_proposal, issue #4), on an NES copy's
//! tiles or an arcade copy's board sprites, as serve/pictures.rs is for a
//! theme's picture and serve/sounds.rs for a sound change. The page shows
//! the oldest open proposal as a card from the status line's answer
//! (`waits_for_yes`, `what: "sprite"`); these routes give its pictures and
//! write the answer into the copy's art history, decided by the person.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/sprites` | `{open, recent}`: the open proposals of every copy, oldest first, and the last 20 the person answered here |
//! | `GET /api/sprites/{id}/{which}` | The proposal's `edit.png` (before and after, close up) or `context.png` (the recorded frames before and after) |
//! | `POST /api/sprites/{id}` | `{answer: "accept" \| "reject" \| "undo", words?}`: ACCEPT, REJECT with what to change, or PUT THE OLD ONE BACK |

use gp_mcp::sprite_proposal::{self as store, Answer};
use serde_json::{Value, json};

use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/sprites",
            about: "The sprite edits the agent proposes for the person's copies: open ones and the last answered",
            handler: list,
        },
        Route {
            path: "/api/sprites/{id}/{which}",
            about: "A proposal's edit.png (before and after, close up) or context.png (the recorded frames)",
            handler: picture,
        },
        Route {
            path: "/api/sprites/{id}",
            about: "POST {answer: accept|reject|undo, words?}: the person's answer to a proposed sprite edit",
            handler: answer,
        },
    ]
}

fn list(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    let home = &ctx.paths.home;
    let recent: Vec<Value> = store::answered(home).into_iter().take(20).collect();
    Ok(Response::json(&json!({
        "open": store::queue(home),
        "recent": recent,
    })))
}

fn picture(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let id = params.get("id").map(String::as_str).unwrap_or("");
    let which = params.get("which").map(String::as_str).unwrap_or("");
    let which = which.strip_suffix(".png").unwrap_or(which);
    let bytes = store::png(&ctx.paths.home, id, which)
        .ok_or_else(|| Fail::not_found(format!("no picture {which} for proposal {id}")))?;
    Ok(Response::bytes("image/png", bytes))
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
    let config = super::jobs::config(ctx)?;
    let done = store::decide(&config, id, a, v["words"].as_str()).map_err(Fail::conflict)?;
    Ok(Response::json(&done))
}
