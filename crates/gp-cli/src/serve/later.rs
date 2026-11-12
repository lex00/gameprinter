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
        "/api/capture/{run}/lockstep",
        "the lockstep view on a run's recordings isn't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/moments",
        "the moments view isn't in gameprinter's gp serve yet; gp interp moments gives them",
    ),
    (
        "/api/capture/{run}/refine",
        "the refine view isn't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/refine/stop",
        "the refine view isn't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/scenarios",
        "the scenario ladder view isn't in gameprinter's gp serve yet; gp interp scenarios runs it",
    ),
    (
        "/api/capture/{run}/start-state",
        "a start state from a run's recording isn't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/point",
        "point and tell isn't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/requests",
        "point and tell's requests aren't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/edits",
        "the sprite edits list isn't in gameprinter's gp serve yet",
    ),
    (
        "/api/capture/{run}/inputs/{name}",
        "a run's recordings' input logs aren't in gameprinter's gp serve yet: a game has no capture run's recordings",
    ),
    (
        "/api/capture/{run}/input",
        "a capture run's inputs (answers, pins, notes) aren't in gameprinter's gp serve: a game has no capture run",
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
