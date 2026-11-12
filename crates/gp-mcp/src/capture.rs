//! The agent in a capture run (docs/capture-live.md): every tool takes an
//! optional `run`, and a call that names one is appended to the run's
//! inputs as a `tool` input (the tool, its arguments and its result, each
//! cut to a line), which the run's writer echoes into the events as the
//! agent's. `capture_events` reads a run's events after a sequence number,
//! as the workspace does, and `capture_input` sends an answer, a pin or a
//! note as the agent. The format is `gp-capture`'s
//! (docs/formats/capture-events.md).

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use gp_capture::{Actor, Input};

use crate::{Config, Reply};

/// How long a tool's arguments or result may be in the log.
pub const SUMMARY_CHARS: usize = 200;

/// The most events `capture_events` returns at once.
pub const PAGE: usize = 500;

/// The `run` argument every tool takes, added to each tool's schema.
pub fn run_property() -> Value {
    json!({
        "type": ["string", "null"],
        "description": "A capture run (gp capture, docs/capture-live.md) to log this call in, so the person watching the run sees it beside the pipeline's stages.",
    })
}

/// Adds the optional `run` to a tool's schema.
pub fn with_run(mut schema: Value) -> Value {
    if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut)
        && !props.contains_key("run")
    {
        props.insert("run".into(), run_property());
    }
    schema
}

/// Takes `run` out of a tool's arguments, so the tool's own argument type
/// never sees it.
pub fn split_run(args: &mut Value) -> Option<String> {
    args.as_object_mut()?
        .remove("run")
        .and_then(|v| v.as_str().map(str::to_owned))
}

/// Appends a tool call to a run's inputs as the agent's. Returns why it
/// couldn't be logged, if it couldn't.
pub fn log_call(
    config: &Config,
    run: &str,
    tool: &str,
    args: &Value,
    reply: &Reply,
) -> Option<String> {
    let dir = match gp_capture::run_dir(&config.home, run) {
        Ok(d) => d,
        Err(e) => return Some(e.to_string()),
    };
    let mut input = Input::new(Actor::Agent, "tool");
    input.tool = Some(tool.to_owned());
    input.args = Some(gp_capture::summarize(args, SUMMARY_CHARS));
    let result = gp_capture::summarize(&reply.json, SUMMARY_CHARS);
    input.result = Some(if reply.is_error {
        format!("refused: {result}")
    } else {
        result
    });
    gp_capture::append_input(&dir, &input)
        .err()
        .map(|e| e.to_string())
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventsArgs {
    /// The capture run.
    pub run: String,
    /// The last sequence number already read; events after it are returned.
    #[serde(default)]
    pub after: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputArgs {
    /// The capture run.
    pub run: String,
    /// `answer` (question and choice: only what the person chose), `pin` or
    /// `unpin` (path: a question, model path or gap the person said won't
    /// be modelled) or `note` (text for the person).
    pub kind: String,
    #[serde(default)]
    pub question: Option<String>,
    /// The candidate chosen, from 1.
    #[serde(default)]
    pub choice: Option<u32>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

/// The run's events after `after`, with what's still open: questions asked
/// and not answered, and how many inputs wait for the next round.
pub fn events(config: &Config, a: EventsArgs) -> Result<Reply, String> {
    let dir = gp_capture::run_dir(&config.home, &a.run).map_err(|e| e.to_string())?;
    if !dir.join("run.json").is_file() {
        return Err(format!("there's no capture run `{}`", a.run));
    }
    let all = gp_capture::events_after(&dir, 0, usize::MAX).map_err(|e| e.to_string())?;
    let mut open: Vec<Value> = Vec::new();
    for e in &all {
        match e["kind"].as_str() {
            Some("question") => open.push(e["data"].clone()),
            Some("input")
                if e["data"]["status"] == "applied"
                    && matches!(e["data"]["input"]["kind"].as_str(), Some("answer" | "pin")) =>
            {
                let i = &e["data"]["input"];
                let key = i["question"].as_str().or(i["path"].as_str()).unwrap_or("");
                open.retain(|q| q["id"] != key && q["path"] != key);
            }
            _ => {}
        }
    }
    let page: Vec<Value> = all
        .iter()
        .filter(|e| e["seq"].as_u64().is_some_and(|s| s > a.after))
        .take(PAGE)
        .cloned()
        .collect();
    let applied = std::fs::read(dir.join("run.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|r| r["inputs_applied"].as_u64())
        .unwrap_or(0) as usize;
    let inputs = gp_capture::read_inputs(&dir).map_err(|e| e.to_string())?;
    Ok(Reply::ok(json!({
        "run": a.run,
        "events": page,
        "last_seq": all.last().and_then(|e| e["seq"].as_u64()).unwrap_or(0),
        "open_questions": open,
        "pending_inputs": inputs.len().saturating_sub(applied),
        "watched": gp_capture::writer_busy(&dir),
    })))
}

/// Sends an input to a run as the agent.
pub fn input(config: &Config, a: InputArgs) -> Result<Reply, String> {
    let dir = gp_capture::run_dir(&config.home, &a.run).map_err(|e| e.to_string())?;
    if a.kind == "tool" {
        return Err("tool inputs are written for every call that names a run".into());
    }
    let mut i = Input::new(Actor::Agent, &a.kind);
    i.question = a.question;
    i.choice = a.choice;
    i.path = a.path;
    i.text = a.text;
    let n = gp_capture::append_input(&dir, &i).map_err(|e| e.to_string())?;
    Ok(Reply::ok(json!({
        "run": a.run,
        "inputs": n,
        "watched": gp_capture::writer_busy(&dir),
        "input": i,
    })))
}

/// A run's newest event since `after` as a line, for a progress message.
pub fn progress_line(config: &Config, run: &str, after: u64) -> Option<(u64, String)> {
    let dir = gp_capture::run_dir(&config.home, run).ok()?;
    let evs = gp_capture::events_after(&dir, after, usize::MAX).ok()?;
    let e = evs.iter().rev().find(|e| {
        matches!(
            e["kind"].as_str(),
            Some("stage.end" | "round.end" | "run.state")
        )
    })?;
    let d = &e["data"];
    let text = match e["kind"].as_str()? {
        "stage.end" => format!(
            "round {}: {} {}",
            e["round"],
            e["stage"].as_str().unwrap_or(""),
            d["status"].as_str().unwrap_or("")
        ),
        "round.end" => format!(
            "round {} ended: {}",
            e["round"],
            d["summary"].as_str().unwrap_or("")
        ),
        _ => format!("run {}", d["state"].as_str().unwrap_or("")),
    };
    Some((evs.last()?["seq"].as_u64()?, text))
}

/// The last sequence number of a run, 0 when it has none or isn't there.
pub fn last_seq(config: &Config, run: &str) -> u64 {
    gp_capture::run_dir(&config.home, run)
        .ok()
        .and_then(|d| gp_capture::last_seq(&d).ok())
        .unwrap_or(0)
}
