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
//! | `POST /api/capture/{run}/input` | A point-and-tell `request`: `text`, and in `data` the recording, frame, box, label, choice and what is under it; with `data.ask` `explain` it is the person's "what is this?", and the turn's `point.explain` is the `gp element explain` call that answers it |
//!
//! A question ("what is this?", issue #10) is the same turn with
//! `point.ask` set to `explain` and `point.explain` the command line the
//! agent runs to answer it, made here from what the server knows: the
//! game's folder, the clip (a corpus clip by its ID; a free play clip or
//! a store recording has none yet, and `point.explain_missing` says so),
//! the frame from power-on, the box and which game's picture.

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
    let run = super::game::run_dir(ctx, params)?;
    super::post_only(req)?;
    let input: Input = serde_json::from_slice(&req.body)
        .map_err(|e| Fail::bad_request(format!("not an input: {e}")))?;
    if input.kind == "request" {
        return point_turn(params, &input, &run);
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
fn point_turn(params: &Params, input: &Input, run: &super::game::Run) -> Result<Response, Fail> {
    let game = params.get("run").map(String::as_str).unwrap_or("");
    let s = super::pages::made(game).ok_or_else(|| {
        Fail::conflict(
            "open the game's page first, so what you point at goes to the mechanic open there",
        )
    })?;
    let d = input.data.clone().unwrap_or(Value::Null);
    let mut text = input.text.clone().unwrap_or_default().trim().to_owned();
    let choice = d["choice"].as_str().unwrap_or("");
    let asks = d["ask"] == "explain";
    if text.is_empty() {
        text = choice.replace('_', " ");
    }
    if text.is_empty() && asks {
        text = "what is this?".to_owned();
    }
    if text.is_empty() {
        return Err(Fail::bad_request("say what is wrong there"));
    }
    let things: Vec<Value> = d["point"]["things"]
        .as_array()
        .map(|a| a.iter().take(12).cloned().collect())
        .unwrap_or_default();
    let mut point = json!({
        "recording": d["recording"],
        "frame": d["frame"],
        "rect": d["rect"],
        "label": d["label"],
        "choice": d["choice"],
        "things": things,
        "model_frame": d["point"]["model_frame"],
    });
    if asks {
        point["ask"] = json!("explain");
        match explain_call(run, &d) {
            Ok(c) => point["explain"] = json!(c),
            Err(why) => point["explain_missing"] = json!(why),
        }
    }
    let (n, mechanic) = s
        .point_turn(&text, point.clone())
        .map_err(|e| Fail::conflict(format!("{e:#}: open a mechanic on CLIPS first")))?;
    Ok(Response::json(&json!({
        "run": game,
        "mechanic": mechanic,
        "turn": n,
        "explain": point.get("explain"),
        "input": {"kind": "request", "item": format!("request:{n}"), "text": text},
    })))
}

/// The `gp element explain` call that answers a "what is this?" turn:
/// the game's folder, the recording as a corpus clip, the frame from
/// power-on, the box and the side. The words why there is none when the
/// recording is no corpus clip or a part is missing.
pub(crate) fn explain_call(run: &super::game::Run, d: &Value) -> Result<String, String> {
    let rec = d["recording"].as_str().unwrap_or("");
    if rec.is_empty() {
        return Err("the turn names no recording".into());
    }
    if crate::clip_score::corpus_clip(&run.game, rec).is_err() {
        return Err(format!(
            "{rec} isn't a clip of the game's corpus; gp element explain reads corpus clips only so far"
        ));
    }
    let frame = d["frame"].as_u64().ok_or("the turn has no frame")?;
    let r = &d["rect"];
    let n = |k: &str| r[k].as_i64().ok_or_else(|| format!("the box has no {k}"));
    let (x, y, w, h) = (n("x")?, n("y")?, n("w")?, n("h")?);
    let side = if d["side"] == "model" {
        "model"
    } else {
        "original"
    };
    Ok(format!(
        "gp element explain --run {} --clip {rec} --log-frame {frame} --box {x},{y},{w},{h} --side {side}",
        run.game.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A question about a corpus clip gets the call that answers it; one
    /// about a recording that is no corpus clip says why there is none.
    #[test]
    fn a_question_names_the_call_that_answers_it() {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("g");
        std::fs::create_dir_all(game.join("inputs")).unwrap();
        std::fs::write(
            game.join("corpus.toml"),
            "schema = 1\n\n[[clip]]\nid = \"g.walk.001\"\nname = \"walk\"\nstart = 0\npresses = \"inputs/g.walk.001.presses\"\nframes = 2\n",
        )
        .unwrap();
        let run = super::super::game::Run {
            name: "g".into(),
            game: game.clone(),
            dir: d.path().join("serve"),
        };
        let q = json!({"recording": "g.walk.001", "frame": 120, "rect": {"x": 10, "y": 20, "w": 16, "h": 16}, "side": "model"});
        match explain_call(&run, &q) {
            Ok(c) => assert_eq!(
                c,
                format!(
                    "gp element explain --run {} --clip g.walk.001 --log-frame 120 --box 10,20,16,16 --side model",
                    game.display()
                )
            ),
            // The made-up corpus may not read as the clip set's format
            // wants; then the refusal must name the recording.
            Err(why) => assert!(why.contains("g.walk.001"), "{why}"),
        }
        let other =
            json!({"recording": "free-1", "frame": 5, "rect": {"x": 0, "y": 0, "w": 1, "h": 1}});
        let why = explain_call(&run, &other).unwrap_err();
        assert!(why.contains("free-1") && why.contains("corpus"), "{why}");
        assert!(explain_call(&run, &json!({})).is_err());
    }
}
