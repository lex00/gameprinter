//! The loop's answer for the page's status line (crate::next, the same as
//! `gp progress --next` and the MCP tool `next`), with what this server
//! knows of the background work and what the page saw the person do.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/next` | The answer for the game whose page was opened last |
//! | `GET /api/next/{run}` | The answer for a game. `?signal=idle\|drops\|hops\|away`, `&row=ROW` and `&shown=N` (how many times that clue was shown) ask for a clue |

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};
use crate::next::{Background, PageSignal};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/next",
            about: "Where the port of the game opened last stands, and the next step",
            handler: last,
        },
        Route {
            path: "/api/next/{run}",
            about: "Where the port of a game stands, the next step and who does it, and a clue when the page asks: ?signal=idle|drops|hops|away&row=ROW&shown=N",
            handler: of_game,
        },
    ]
}

fn last(ctx: &Ctx, req: &Request, _params: &Params) -> Result<Response, Fail> {
    answer(ctx, req, None)
}

fn of_game(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let game = params.get("run").map(String::as_str).unwrap_or("");
    super::game::run_named(ctx, game)?;
    answer(ctx, req, Some(game))
}

fn answer(ctx: &Ctx, req: &Request, game: Option<&str>) -> Result<Response, Fail> {
    let live = game
        .and_then(|g| super::game::run_named(ctx, g).ok())
        .map(|run| {
            let (on, jobs, error) = super::background::live(&run);
            Background {
                on,
                jobs,
                error,
                page_open: true,
            }
        });
    let mut f = crate::next::gather(&ctx.paths, Some(&ctx.games), game, live)?;
    f.page = req
        .param("signal")
        .filter(|s| ["idle", "drops", "hops", "away"].contains(s))
        .map(|s| PageSignal {
            signal: s.to_owned(),
            row: req.param("row").map(str::to_owned),
            shown: req.param("shown").and_then(|n| n.parse().ok()).unwrap_or(0),
        });
    Ok(Response::json(&crate::next::json_of(&crate::next::answer(
        &f,
    ))))
}
