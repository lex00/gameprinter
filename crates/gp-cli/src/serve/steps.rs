//! The loop's steps on the page (docs/loop.md): the open steps of the
//! home, each with its id and words, and the person's WRONG on one, which
//! halts the loop (crate::wrong).
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/steps` | `{open, halted, workers, settings, runner}`: the open steps as `gp loop steps` gives them, the halt, the runner's workers, the home's loop settings and the runner in words |
//! | `POST /api/steps/{id}/wrong` | `{why}`: marks open step `id` wrong; the loop halts until a landed change names the objection |

use serde_json::{Value, json};

use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/steps",
            about: "The loop's open steps, its halt and its settings",
            handler: list,
        },
        Route {
            path: "/api/steps/{id}/wrong",
            about: "POST {why}: the person marks an open step wrong, which halts the loop",
            handler: wrong,
        },
    ]
}

fn list(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    let mut v = crate::loop_cmd::steps_json(&ctx.paths, Some(&ctx.games));
    v["settings"] = json!(gp_mcp::steps::settings(&ctx.paths.home));
    v["runner"] = super::runner::status(ctx);
    Ok(Response::json(&v))
}

fn wrong(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let id = params.get("id").map(String::as_str).unwrap_or("");
    let v: Value = super::body(req)?;
    let why = v["why"].as_str().unwrap_or("");
    if why.trim().is_empty() {
        return Err(Fail::bad_request("why: say what is wrong with the step"));
    }
    let done = crate::wrong::mark(&ctx.paths, Some(&ctx.games), id, why)
        .map_err(|e| Fail::conflict(e.to_string()))?;
    Ok(Response::json(&done))
}
