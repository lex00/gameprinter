//! The person's yes on the page: the asks `gp_run` left when it refused a
//! command for want of a yes (gp_mcp::asks), and the page's YES and NO.
//! The page shows them from the status line's answer (`waits_for_yes`);
//! these routes list them and write the answer.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/asks` | The asks still open, and the yeses not yet used, newest first |
//! | `POST /api/asks/{id}` | `{answer: "yes" \| "no", words?, duplicate_of?, keep_going?}`: the person's answer to an open ask, with what they wrote beside it: kept on the ask, written to its row's talk as their turn, and put in the row's note when the runner settles the row by the card (a NO with words sends the row to next with them). A NO with `duplicate_of` (another open card of the game, the card's DUPLICATE OF) parks the row in line waiting on that card's row, instead of sending it back to make the same card again. A NO with `keep_going` (a gate card's LOOKS GOOD, KEEP GOING) lands nothing and parks the row in line marked iterating: no worker gates it until the person presses LAND IT on the row |

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
            about: "POST {answer: yes|no, words?}: the person's answer to an ask, and what they wrote beside it",
            handler: answer,
        },
    ]
}

fn list(ctx: &Ctx, _req: &Request, _params: &Params) -> Result<Response, Fail> {
    let home = &ctx.paths.home;
    // Each card with the quest-log row it is for (`GAME/ROW`), from its
    // step's worker record, so the page shows a row's card beside the row
    // after its step has closed.
    let shown: Vec<Value> = asks::shown(home)
        .into_iter()
        .map(|mut a| {
            if let Some((g, r)) = super::runner::ask_row(home, &a) {
                a["row"] = json!(format!("{g}/{r}"));
            }
            a
        })
        .collect();
    Ok(Response::json(&json!({"asks": shown})))
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
    let words = v["words"].as_str().unwrap_or("").trim();
    // LOOKS GOOD, KEEP GOING on a gate's card: a NO that lands nothing and
    // marks the row iterating when the runner settles it
    // (runner::settle_row_by_card).
    let keep_going = v["keep_going"].as_bool() == Some(true);
    if keep_going && a != asks::Answer::No {
        return Err(Fail::bad_request("keep_going goes with a NO"));
    }
    let home = &ctx.paths.home;
    let dup = duplicate_of(home, id, a, &v)?;
    let mut done = asks::answer_with(home, id, a, words).map_err(Fail::conflict)?;
    if let Some(d) = &dup {
        asks::mark(home, id, "duplicate_of", json!(d)).map_err(Fail::conflict)?;
        done["duplicate_of"] = json!(d);
    }
    if keep_going {
        asks::mark(home, id, "keep_going", json!(true)).map_err(Fail::conflict)?;
        done["keep_going"] = json!(true);
    }
    // The words go to the card's row as the person's turn, so its worker
    // reads them with the row; the row's status is the runner's to set
    // when it settles the card (runner::settle_card_rows).
    let mut out = done.clone();
    let said_words = done["words"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| keep_going.then(String::new));
    if let Some(w) = said_words
        && let Some((game, row)) = super::runner::ask_row(&ctx.paths.home, &done)
    {
        let said = match a {
            _ if keep_going => "LOOKS GOOD, KEEP GOING",
            asks::Answer::Yes => "YES",
            asks::Answer::No => "NO",
        };
        let w = if w.is_empty() {
            "nothing lands; it stays iterating until LAND IT on the row".to_owned()
        } else {
            w
        };
        let text = match &dup {
            Some(d) => format!(
                "NO to the card ({}), a duplicate of card {d}: {w}",
                super::runner::card_what(&done)
            ),
            None => format!(
                "{said} to the card ({}): {w}",
                super::runner::card_what(&done)
            ),
        };
        let turn = crate::mechanic::session::Session::open_or_make(&ctx.paths, &game, &row)
            .and_then(|s| s.turn_keeping_status(json!({"by": "player", "text": text, "card": id})));
        out["row"] = json!(format!("{game}/{row}"));
        if let Err(e) = turn {
            out["talk_error"] = json!(format!("{e:#}"));
        }
    }
    Ok(Response::json(&out))
}

/// The card a NO names as the one ask `id` duplicates (`duplicate_of`):
/// another ask on file, of the same game, still open or holding a yes.
/// Refused with a YES, or when it names no such card.
fn duplicate_of(
    home: &std::path::Path,
    id: &str,
    a: asks::Answer,
    v: &Value,
) -> Result<Option<String>, Fail> {
    let Some(other) = v["duplicate_of"]
        .as_str()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    else {
        return Ok(None);
    };
    if a != asks::Answer::No {
        return Err(Fail::bad_request("duplicate_of goes with a NO"));
    }
    if other == id {
        return Err(Fail::bad_request("a card isn't a duplicate of itself"));
    }
    let Some(mine) = asks::read(home, id) else {
        let why = format!("there's no ask {id}");
        return Err(Fail::conflict(why));
    };
    let Some(b) = asks::read(home, other) else {
        let why = format!("duplicate_of: there's no card {other}");
        return Err(Fail::bad_request(why));
    };
    let now = asks::now_ms();
    let state = b["state"].as_str().unwrap_or("?");
    if !(asks::is_open(&b, now) || asks::is_yes(&b, now) || state == "running") {
        let why = format!("duplicate_of: card {other} isn't open any more ({state})");
        return Err(Fail::bad_request(why));
    }
    if b["game"] != mine["game"] {
        let why = format!("duplicate_of: card {other} is for another game");
        return Err(Fail::bad_request(why));
    }
    Ok(Some(other.to_owned()))
}
