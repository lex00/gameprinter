//! Where the page's pointer sends what the person says about a box on
//! the screen (point and tell): `POST /api/capture/{run}/input` with kind
//! `request` becomes the person's turn in the talk of the mechanic open on
//! the game's page, with a `point` field, which the agent reads with
//! `gp mechanic show` and answers with `gp mechanic say`.
//!
//! Until 2026-11-13 this route also kept a capture run's inputs
//! (`$GP_HOME/capture/GAME/inputs.ndjson`): requests, the agent's sprite
//! edits and a person's accept or reject of them. Nothing made a capture
//! run any more, so that store was removed; the owner approved it on
//! 2026-11-13. Other kinds (answers, pins, notes from the old editor's
//! live panel) are refused in words, as they were.
//!
//! | Endpoint | What |
//! |---|---|
//! | `POST /api/capture/{run}/input` | A point-and-tell `request`: `text`, and in `data` the recording, frame, box, label, choice and what is under it |

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![Route {
        path: "/api/capture/{run}/input",
        about: "POST a point-and-tell request: it becomes the person's turn in the talk of the mechanic open on the game's page",
        handler: input_route,
    }]
}

/// What the page sends: the fields of an input the route reads.
#[derive(Deserialize)]
struct Input {
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    data: Option<Value>,
}

fn input_route(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    super::game::run_dir(ctx, params)?;
    super::post_only(req)?;
    let input: Input = serde_json::from_slice(&req.body)
        .map_err(|e| Fail::bad_request(format!("not an input: {e}")))?;
    if input.kind == "request" {
        return point_turn(params, &input);
    }
    Err(Fail {
        status: 422,
        message: format!(
            "a {} input is for a capture run's loop, which a game in gameprinter has none of: the mechanic page's talk takes notes, and the background work's questions take answers",
            input.kind
        ),
    })
}

/// A point-and-tell request: the person's turn in the open mechanic's
/// talk, with a `point` (the frame, the recording, the box and what is
/// under it, the choice), which the agent reads with `gp mechanic show`
/// (capture-run.md, step 3). The reply keeps the shape the pointer reads
/// (`input.item` is `request:N`, N the turn's number).
fn point_turn(params: &Params, input: &Input) -> Result<Response, Fail> {
    let game = params.get("run").map(String::as_str).unwrap_or("");
    let s = super::pages::made(game).ok_or_else(|| {
        Fail::conflict(
            "open the game's page first, so what you point at goes to the mechanic open there",
        )
    })?;
    let d = input.data.clone().unwrap_or(Value::Null);
    let mut text = input.text.clone().unwrap_or_default().trim().to_owned();
    let choice = d["choice"].as_str().unwrap_or("");
    if text.is_empty() {
        text = choice.replace('_', " ");
    }
    if text.is_empty() {
        return Err(Fail::bad_request("say what is wrong there"));
    }
    let things: Vec<Value> = d["point"]["things"]
        .as_array()
        .map(|a| a.iter().take(12).cloned().collect())
        .unwrap_or_default();
    let point = json!({
        "recording": d["recording"],
        "frame": d["frame"],
        "rect": d["rect"],
        "label": d["label"],
        "choice": d["choice"],
        "things": things,
        "model_frame": d["point"]["model_frame"],
    });
    let (n, mechanic) = s
        .point_turn(&text, point)
        .map_err(|e| Fail::conflict(format!("{e:#}: open a mechanic on CLIPS first")))?;
    Ok(Response::json(&json!({
        "run": game,
        "mechanic": mechanic,
        "turn": n,
        "input": {"kind": "request", "item": format!("request:{n}"), "text": text},
    })))
}
