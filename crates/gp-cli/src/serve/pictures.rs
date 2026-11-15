//! The person's answer to a picture the agent proposes for their theme
//! (crate::theme_picture, issue #4). The page shows the oldest open
//! proposal as a card from the status line's answer (`waits_for_yes`,
//! `what: "picture"`), and THEMES lists them all; these routes give the
//! pictures and write the answer.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/pictures` | `{open, recent}`: the open proposals, oldest first, and the last 20 answered |
//! | `GET /api/pictures/{id}/{which}` | The proposal's `old.png` or `new.png` |
//! | `POST /api/pictures/{id}` | `{answer: "accept" \| "reject" \| "undo", words?}`: ACCEPT, REJECT with what to change, or PUT THE OLD ONE BACK |

use serde_json::{Value, json};

use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};
use crate::theme_picture::{self, Answer};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/pictures",
            about: "The pictures the agent proposes for the person's theme: open ones and the last answered",
            handler: list,
        },
        Route {
            path: "/api/pictures/{id}/{which}",
            about: "A proposal's old.png or new.png",
            handler: picture,
        },
        Route {
            path: "/api/pictures/{id}",
            about: "POST {answer: accept|reject|undo, words?}: the person's answer to a proposed picture",
            handler: answer,
        },
    ]
}

fn list(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    let home = &ctx.paths.home;
    let recent: Vec<Value> = theme_picture::all(home)
        .into_iter()
        .filter(|v| v["state"] != json!("open"))
        .take(20)
        .collect();
    Ok(Response::json(&json!({
        "open": theme_picture::queue(home),
        "recent": recent,
    })))
}

fn picture(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let id = params.get("id").map(String::as_str).unwrap_or("");
    let which = params.get("which").map(String::as_str).unwrap_or("");
    let which = which.strip_suffix(".png").unwrap_or(which);
    let bytes = theme_picture::png(&ctx.paths.home, id, which)
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
    let done = theme_picture::decide(&ctx.paths.home, id, a, v["words"].as_str())
        .map_err(Fail::conflict)?;
    Ok(Response::json(&done))
}
