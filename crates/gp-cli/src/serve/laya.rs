//! Laya on gp serve ([`crate::laya`]): the decision server the tools ask
//! which buttons to hold, started here when a tool needs it and stopped
//! with gp serve. One already answering at 127.0.0.1:7720 is used as it is.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/laya` | Whether Laya answers, who started it, the install it runs from, or how to install it |
//! | `POST /api/laya/stop` | Stops the Laya gp serve started; one another process started is left alone |
//! | `POST /api/laya/start` | Starts Laya from the install when none answers and waits until it does (a tool's ask: `gp mechanic demo`, `gp mechanic say --judge`, the triage's demo step) |

use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/laya",
            about: "Whether Laya (the decision server) answers at 127.0.0.1:7720, who started it, and its install",
            handler: status,
        },
        Route {
            path: "/api/laya/start",
            about: "Starts Laya from its install when none answers, and waits until it does",
            handler: start,
        },
        Route {
            path: "/api/laya/stop",
            about: "Stops the Laya gp serve started (one another process started is left alone)",
            handler: stop,
        },
    ]
}

fn status(ctx: &Ctx, _req: &Request, _p: &Params) -> Result<Response, Fail> {
    Ok(Response::json(&crate::laya::status(&ctx.paths.home)))
}

fn start(ctx: &Ctx, req: &Request, _p: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let logs = ctx.paths.home.join("work").join("serve");
    match crate::laya::ensure(&ctx.paths.home, &ctx.repo, &logs) {
        Ok(v) => Ok(Response::json(&v)),
        Err(e) => Err(Fail::conflict(format!("{e:#}"))),
    }
}

fn stop(ctx: &Ctx, req: &Request, _p: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let stopped = crate::laya::stop();
    let mut v = crate::laya::status(&ctx.paths.home);
    v["stopped"] = serde_json::json!(stopped);
    Ok(Response::json(&v))
}
