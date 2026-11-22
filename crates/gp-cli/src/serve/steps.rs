//! The loop's steps on the page (docs/loop.md): the open steps of the
//! home, each with its id and words, and the person's WRONG on one, which
//! halts the loop (crate::wrong).
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/steps` | `{open, halted, workers, settings, runner}`: the open steps as `gp loop steps` gives them, the halt, the runner's workers, the home's loop settings and the runner in words |
//! | `POST /api/steps/{id}/wrong` | `{why}`: marks open step `id` wrong; the loop halts until a landed change names the objection |
//! | `GET /api/scoreboard/{game}` | `{game, lines, held_out, error, done_ms}`: the game's scoreboard as kept (crate::scoreboard), each line with its owner's lock (`locked`) and its confirm row, or `lines: []` when the game has none; `done_ms`, when each row finished in the last day |

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
        Route {
            path: "/api/scoreboard/{game}",
            about: "The game's scoreboard: a line for each part worked on, with its state, background, sprites and approved looks, and how each moved",
            handler: scoreboard,
        },
    ]
}

fn scoreboard(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let game = params.get("game").map(String::as_str).unwrap_or("");
    if !crate::next::games_in(&ctx.games).iter().any(|g| g == game) {
        return Err(Fail::not_found(format!(
            "no game {game} in the games checkout"
        )));
    }
    let mut v = crate::scoreboard::board(&ctx.paths.home, &ctx.games, game)
        .unwrap_or_else(|| json!({"game": game, "lines": [], "error": null}));
    let since = gp_mcp::asks::now_ms().saturating_sub(DAY_MS);
    v["done_ms"] = json!(done_since(&ctx.paths.home, game, since));
    Ok(Response::json(&v))
}

/// A day, in ms: the page counts the rows done since its own midnight
/// out of the last day's.
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// When each of the game's rows finished, newest first, since `since` (ms
/// since the epoch): the quest log's done rows by their last change, and
/// the archive's by when they closed.
fn done_since(home: &std::path::Path, game: &str, since: u64) -> Vec<u64> {
    let file = home.join("mechanics").join(game).join("queue.json");
    let q = crate::mechanic::session::read_queue(&file);
    let open = q
        .iter()
        .filter(|r| r["status"] == json!("done"))
        .filter_map(|r| r["changed_ms"].as_u64());
    let archived = crate::mechanic::archive::read_archive(home, game);
    let closed = archived.iter().filter_map(|r| r["closed_ms"].as_u64());
    let mut all: Vec<u64> = open.chain(closed).filter(|t| *t >= since).collect();
    all.sort_unstable();
    all.reverse();
    all
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
