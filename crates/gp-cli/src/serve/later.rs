//! The editor's routes that gameprinter's `gp serve` doesn't serve yet,
//! each answering in words what it was and what stands in for it, so the
//! page says so rather than failing quietly. Each moves to a module of its
//! own as it is ported from game-printer's serve/.

use super::game::run_dir;
use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

/// Each route and why it isn't here yet.
const LATER: &[(&str, &str)] = &[
    (
        "/api/capture/{run}/grid",
        "the grid (game-printer's serve/grid.rs) isn't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/grid/in-flight",
        "the grid's clips in flight aren't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/grid/mode",
        "the grid's mode isn't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/clips/{clip}/tell",
        "telling an agent about a clip isn't in gameprinter's gp serve yet; gp mechanic say is the agent's side of the mechanic page's talk",
    ),
    (
        "/api/capture/{run}/refine",
        "the game view's refine panel (game-printer's serve/refine.rs) isn't in gameprinter's gp serve: the background work on the REFINE tab runs the same loop (gp model refine --lockstep) on a clip",
    ),
    (
        "/api/capture/{run}/refine/stop",
        "the game view's refine panel isn't in gameprinter's gp serve: the REFINE tab's background work has the pause",
    ),
];

pub(super) fn routes() -> Vec<Route> {
    LATER
        .iter()
        .map(|&(path, about)| Route {
            path,
            about,
            handler: later,
        })
        .collect()
}

fn later(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    run_dir(ctx, params)?;
    let why = ctx
        .routes
        .iter()
        .find(|r| super::matches(r.path, &req.path).is_some())
        .map_or("not here yet", |r| r.about);
    Err(Fail {
        status: 501,
        message: why.to_owned(),
    })
}
