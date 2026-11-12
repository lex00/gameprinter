//! The `requests` tool (docs/point-and-tell.md): what the person pointed at
//! in the workspace and said should happen there, with every reply so far.
//! The agent reads them, has one explained (`gp request explain`, the same
//! helper the workspace calls), and answers with its own reply, which can
//! carry a proposal made with `assets_edit` or `sound_edit` for the person
//! to accept or reject beside the request.

use gp_capture::{Actor, Input};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Config, Reply};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestsArgs {
    /// The capture run or play session the person is working in.
    pub run: String,
    /// Only requests with this status: `open`, `explained`, `proposed`,
    /// `needs_mechanic`, `waiting`, `accepted` or `rejected`.
    #[serde(default)]
    pub status: Option<String>,
    /// Explain this request now (`gp request explain`): what the original
    /// does in its frames and region, what the model does instead, and the
    /// model piece that would cover it, posted as a reply.
    #[serde(default)]
    pub explain: Option<u64>,
    /// Answer a request yourself.
    #[serde(default)]
    pub reply: Option<ReplyArgs>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplyArgs {
    /// The request's number.
    pub id: u64,
    /// What to tell the person, in plain words: game pictures and names,
    /// never RAM or code.
    pub text: String,
    /// `explained`, `proposed` (with `proposal`) or `needs_mechanic` (no
    /// tool can make the change yet; say in `would_build` what would).
    #[serde(default = "explained")]
    pub status: String,
    /// The proposal's item: an assets_edit made with place false
    /// (`<copy>:<seq>`) or a sound_edit (`sound:<seq>`). The person accepts
    /// or rejects it under the request.
    #[serde(default)]
    pub proposal: Option<String>,
    /// For `needs_mechanic`: the general model piece that would cover it.
    #[serde(default)]
    pub would_build: Option<Value>,
}

fn explained() -> String {
    "explained".into()
}

pub fn requests(config: &Config, a: RequestsArgs) -> Result<Reply, String> {
    let dir = gp_capture::run_dir(&config.home, &a.run).map_err(|e| e.to_string())?;
    if !dir.join("run.json").is_file() {
        return Err(format!("there's no capture run `{}`", a.run));
    }
    let mut did = json!(null);
    if let Some(n) = a.explain {
        let out = std::process::Command::new(&config.gp)
            .args(["--json", "request", "explain", &a.run, &n.to_string()])
            .env("GP_HOME", &config.home)
            .current_dir(&config.repo)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("running {}: {e}", config.gp.display()))?;
        if !out.status.success() {
            return Err(format!(
                "gp request explain exited {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        did = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    }
    if let Some(r) = a.reply {
        if !gp_capture::requests::STATUSES.contains(&r.status.as_str()) {
            return Err(format!(
                "status is one of {}",
                gp_capture::requests::STATUSES.join(", ")
            ));
        }
        if r.status == "proposed" && r.proposal.is_none() {
            return Err("a proposed reply names its proposal (<copy>:<seq> or sound:<seq>)".into());
        }
        let mut i = Input::new(Actor::Agent, "reply");
        i.item = Some(format!("request:{}", r.id));
        i.text = Some(r.text.clone());
        i.data = Some(json!({
            "status": r.status,
            "lines": [r.text],
            "proposal": r.proposal.as_ref().map(|p| json!({"item": p, "kind": if p.starts_with("sound:") { "sound_edit" } else { "asset_edit" }})),
            "would_build": r.would_build,
        }));
        let posted = gp_capture::requests::post(&dir, i).map_err(|e| e.to_string())?;
        did = json!({"replied": posted.input, "event": posted.event});
    }
    let mut all = gp_capture::requests::list(&dir).map_err(|e| e.to_string())?;
    if let Some(s) = &a.status {
        all.retain(|r| &r.status == s);
    }
    Ok(Reply::ok(json!({
        "run": a.run,
        "requests": all,
        "did": did,
        "watched": gp_capture::writer_busy(&dir),
    })))
}
