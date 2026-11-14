//! The person's yes on the page: the asks `gp_run` left when it refused a
//! command for want of a yes (gp_mcp::asks), and the page's YES and NO.
//! The page shows them from the status line's answer (`waits_for_yes`);
//! these routes list them and write the answer.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/asks` | The asks still open, and the yeses not yet used, newest first |
//! | `POST /api/asks/{id}` | `{answer: "yes" \| "no"}`: the person's answer to an open ask |

use serde_json::{Value, json};

use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};
use gp_mcp::asks;

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/asks",
            about: "What the agent waits for the person's yes on: the asks still open",
            handler: list,
        },
        Route {
            path: "/api/asks/{id}",
            about: "POST {answer: yes|no}: the person's answer to an ask",
            handler: answer,
        },
    ]
}

fn list(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    Ok(Response::json(
        &json!({"asks": asks::pending(&ctx.paths.home)}),
    ))
}

fn answer(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let id = params.get("id").map(String::as_str).unwrap_or("");
    let v: Value = super::body(req)?;
    let a = match v["answer"].as_str() {
        Some("yes") => asks::Answer::Yes,
        Some("no") => asks::Answer::No,
        _ => return Err(Fail::bad_request("answer is yes or no")),
    };
    let done = asks::answer(&ctx.paths.home, id, a).map_err(Fail::conflict)?;
    Ok(Response::json(&done))
}
