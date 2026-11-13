//! A game's sprite edits (docs/sprite-editing.md): each edit an agent's
//! `assets_edit` told the game's run of, with its status read from the
//! copy now, and its preview pictures; and the run's inputs, where a
//! person's accept or reject of an edit, and a point-and-tell request, go.
//!
//! Ported from game-printer's serve/edits.rs, capture_edits.rs and the
//! input route of serve/capture.rs (wave 8). What gameprinter's layout
//! changed: a run is a game, and the folder its agent's MCP tools write
//! (`assets_edit` and `requests` with `run` = the game's folder name) is
//! `$GP_HOME/capture/GAME/`, as gp-mcp names it. A game with no such
//! folder has no edits yet. The run's other inputs (answers, pins, notes
//! to a capture run's loop) have no capture run to go to; the mechanic
//! page's talk is where notes go.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/edits` | The game's sprite edits: item, text, and data with status, previews, shots and checks |
//! | `GET /api/capture/{run}/edits/{copy}/{seq}/{which}` | An edit's preview PNG: `which` is edit or context |
//! | `GET`, `POST /api/capture/{run}/input` | The run's inputs; POST a `request` (point and tell) or an `accept` or `reject` of a sprite edit |

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use gp_capture::{Input, read_inputs};
use serde_json::{Value, json};

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/edits",
            about: "The run's sprite edits (asset_edit inputs): item, text, and data with status, previews, shots and checks",
            handler: edits_route,
        },
        Route {
            path: "/api/capture/{run}/edits/{copy}/{seq}/{which}",
            about: "An edit's preview PNG: which is edit (before, after, changed) or context (the recorded frames)",
            handler: preview,
        },
        Route {
            path: "/api/capture/{run}/input",
            about: "The run's inputs; POST a point-and-tell request, or accept or reject a sprite edit",
            handler: input_route,
        },
    ]
}

/// The folder a game's agent tools write: `$GP_HOME/capture/GAME/`, the
/// run gp-mcp's `assets_edit` and `requests` name by the game's folder.
pub(crate) fn agent_dir(ctx: &Ctx, params: &Params) -> Result<PathBuf, Fail> {
    let run = super::game::run_dir(ctx, params)?;
    gp_capture::run_dir(&ctx.paths.home, &run.name).map_err(|e| Fail::bad_request(e.to_string()))
}

fn edits_route(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let dir = agent_dir(ctx, params)?;
    let list = if dir.join("run.json").is_file() {
        edits(&dir)?
    } else {
        Vec::new()
    };
    Ok(Response::json(
        &json!({"run": params["run"], "edits": list}),
    ))
}

fn preview(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let dir = agent_dir(ctx, params)?;
    let which = params.get("which").map(String::as_str).unwrap_or("");
    if !matches!(which, "edit" | "context") {
        return Err(Fail::not_found("a preview is edit or context"));
    }
    let item = format!(
        "{}:{}",
        params.get("copy").map(String::as_str).unwrap_or(""),
        params.get("seq").map(String::as_str).unwrap_or("")
    );
    let file = preview_file(&dir, &item, which).map_err(|e| Fail::not_found(e.to_string()))?;
    match std::fs::read(&file) {
        Ok(body) => Ok(Response::bytes("image/png", body)),
        Err(_) => Err(Fail::not_found("the preview file is gone")),
    }
}

fn input_route(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let dir = agent_dir(ctx, params)?;
    if req.method == "GET" || req.method == "HEAD" {
        let inputs = if dir.is_dir() {
            read_inputs(&dir).map_err(|e| anyhow!("{e}"))?
        } else {
            Vec::new()
        };
        return Ok(Response::json(&json!({
            "run": params["run"], "applied": 0, "inputs": inputs,
        })));
    }
    super::post_only(req)?;
    let input: Input = serde_json::from_slice(&req.body)
        .map_err(|e| Fail::bad_request(format!("not an input: {e}")))?;
    std::fs::create_dir_all(&dir)?;
    // A point-and-tell request is numbered and becomes an event at once
    // (docs/point-and-tell.md); a reply is an agent's.
    if input.kind == "request" {
        let mut out = super::point::post(&dir, input)?;
        out["run"] = json!(params["run"]);
        return Ok(Response::json(&out));
    }
    if !matches!(input.kind.as_str(), "accept" | "reject") {
        return Err(Fail {
            status: 422,
            message: format!(
                "a {} input is for a capture run's loop, which a game in gameprinter has none of: the mechanic page's talk takes notes, and the background work's questions take answers",
                input.kind
            ),
        });
    }
    let n = gp_capture::append_input(&dir, &input).map_err(|e| Fail {
        status: 422,
        message: e.to_string(),
    })?;
    let mut out = json!({"run": params["run"], "inputs": n, "input": input});
    // A decision on a sprite edit is applied now, so the panel shows it.
    match apply_decision(&dir, &input) {
        Ok(d) => out["decision"] = d,
        Err(e) => out["decision_error"] = json!(e.to_string()),
    }
    Ok(Response::json(&out))
}

/// The latest `asset_edit` input for each item, oldest item first.
pub(crate) fn announcements(dir: &Path) -> Result<Vec<Input>> {
    let mut out: Vec<Input> = Vec::new();
    for i in read_inputs(dir).map_err(|e| anyhow!("{e}"))? {
        if i.kind != "asset_edit" {
            continue;
        }
        match out.iter_mut().find(|o| o.item == i.item) {
            Some(o) => *o = i,
            None => out.push(i),
        }
    }
    Ok(out)
}

fn path_of(v: &Value) -> Option<PathBuf> {
    v.as_str().map(PathBuf::from)
}

/// The MCP server's configuration an announcement names, with this `gp` as
/// the binary its checks run.
fn server_config(data: &Value) -> Result<gp_mcp::Config> {
    let s = &data["server"];
    Ok(gp_mcp::Config {
        gp: std::env::current_exe().context("finding gp")?,
        repo: path_of(&s["repo"]).ok_or_else(|| anyhow!("the edit doesn't name its repo"))?,
        work: path_of(&s["work"])
            .ok_or_else(|| anyhow!("the edit doesn't name its working copies"))?,
        home: path_of(&s["home"]).ok_or_else(|| anyhow!("the edit doesn't name its home"))?,
        rom: path_of(&s["rom"]),
        assets: path_of(&s["assets"]),
    })
}

fn find(dir: &Path, item: &str) -> Result<Input> {
    announcements(dir)?
        .into_iter()
        .find(|a| a.item.as_deref() == Some(item))
        .ok_or_else(|| {
            anyhow!("no edit {item} in this run (an agent's assets_edit with run names one)")
        })
}

/// Applies an `accept` or `reject` input to the edit it names, now.
/// Returns what `assets_edit` returned, or the reason it couldn't.
pub(crate) fn apply_decision(dir: &Path, input: &Input) -> Result<Value> {
    let accept = match input.kind.as_str() {
        "accept" => true,
        "reject" => false,
        k => bail!("{k} isn't a decision"),
    };
    let item = input
        .item
        .as_deref()
        .ok_or_else(|| anyhow!("a decision names an item"))?;
    if let Some(n) = item.strip_prefix("sound:") {
        // A sound edit is kept or undone by the agent that reads the
        // decision (gp-mcp's sound_edit with undo); nothing to apply here.
        return Ok(json!({
            "status": if accept { "accepted" } else { "rejected" },
            "edit": n,
        }));
    }
    let (copy, seq) = gp_mcp::edit_log::parse_item(item).map_err(anyhow::Error::msg)?;
    let ann = find(dir, item)?;
    let data = ann.data.clone().unwrap_or(Value::Null);
    let config = server_config(&data)?;
    let _lock = gp_mcp::CopyLock::take(copy);
    let reply = gp_mcp::asset_edit::decide(
        &config,
        copy,
        seq,
        accept,
        match input.actor {
            gp_capture::Actor::Agent => "agent",
            _ => "person",
        },
        gp_mcp::asset_edit::Tell::Quiet,
    )
    .map_err(anyhow::Error::msg)?;
    let mut j = reply.json;
    // The base64 previews are for an agent; the panel has the files.
    if let Some(o) = j.as_object_mut() {
        o.remove("preview");
    }
    if reply.is_error {
        bail!(
            "{}",
            j["error"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| j["problems"].to_string())
        );
    }
    Ok(j)
}

/// An edit's status now, from its copy; None when the copy can't be read.
pub(crate) fn status_now(data: &Value, item: &str) -> Option<Value> {
    let (copy, seq) = gp_mcp::edit_log::parse_item(item).ok()?;
    let config = server_config(data).ok()?;
    let dir = config.existing_copy(copy).ok()?;
    let e = gp_mcp::asset_edit::status_in(&dir, seq).ok()??;
    Some(json!({ "status": e.status.name(), "decided_by": e.decided_by }))
}

/// Every edit in the run, as the live panel draws it: the latest
/// announcement's data with the status read from the copy now.
pub(crate) fn edits(dir: &Path) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for a in announcements(dir)? {
        let item = a.item.clone().unwrap_or_default();
        let mut data = a.data.clone().unwrap_or(Value::Null);
        if let Some(now) = status_now(&data, &item) {
            data["status"] = now["status"].clone();
            data["decided_by"] = now["decided_by"].clone();
        }
        if let Some(o) = data.as_object_mut() {
            o.remove("server");
        }
        out.push(json!({ "item": item, "text": a.text, "data": data }));
    }
    Ok(out)
}

/// A preview file of an edit (`edit` or `context`), by the path its
/// announcement gives.
pub(crate) fn preview_file(dir: &Path, item: &str, which: &str) -> Result<PathBuf> {
    let ann = find(dir, item)?;
    let data = ann.data.unwrap_or(Value::Null);
    let p = path_of(&data["previews"][which])
        .ok_or_else(|| anyhow!("edit {item} has no {which} preview"))?;
    // Only a file in the copy's previews folder.
    let config = server_config(&data)?;
    let (copy, _) = gp_mcp::edit_log::parse_item(item).map_err(anyhow::Error::msg)?;
    let previews = config
        .copy_dir(copy)
        .map_err(anyhow::Error::msg)?
        .join(gp_mcp::asset_edit::PREVIEW_DIR);
    if p.parent() != Some(previews.as_path()) {
        bail!("edit {item}'s preview isn't in its copy's previews folder");
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: &str, item: &str, data: Option<Value>) -> Input {
        let actor = if kind == "asset_edit" {
            gp_capture::Actor::Agent
        } else {
            gp_capture::Actor::Person
        };
        let mut i = Input::new(actor, kind);
        i.item = Some(item.into());
        i.data = data;
        i
    }

    #[test]
    fn the_latest_announcement_of_each_edit_counts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("run.json"), "{}").unwrap();
        for i in [
            input("asset_edit", "c:1", Some(json!({ "status": "proposed" }))),
            input("asset_edit", "c:2", Some(json!({ "status": "placed" }))),
            input("accept", "c:1", None),
            input("asset_edit", "c:1", Some(json!({ "status": "accepted" }))),
        ] {
            gp_capture::append_input(dir.path(), &i).unwrap();
        }
        let a = announcements(dir.path()).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].item.as_deref(), Some("c:1"));
        assert_eq!(a[0].data.as_ref().unwrap()["status"], "accepted");
        let e = edits(dir.path()).unwrap();
        assert_eq!(
            e[1]["data"]["status"], "placed",
            "no copy to read: as announced"
        );
        let err = apply_decision(dir.path(), &input("accept", "c:9", None)).unwrap_err();
        assert!(err.to_string().contains("no edit c:9"), "{err}");
    }
}
