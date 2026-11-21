//! The next step at the end of every gp MCP tool reply (docs/mcp.md): its
//! id, who does it and what it says, so an agent always knows the step to
//! do and cite next, and two agents on the same home see the same one.
//!
//! A worker the runner started (`GP_STEP`) is told its own step: it does
//! that one and nothing else. Any other caller gets `next`'s answer for
//! the game or copy the call named (the one opened last when it named
//! none); the tool `next` gives its own answer's step and runs nothing.

use serde_json::{Value, json};

use crate::{Config, Reply, run_gp, steps};

/// What a worker is told: its step is the next one.
pub const WORKER_SAYS: &str =
    "This step is yours: do it and nothing else, then stop with a report.";

/// `next`'s step as the reply carries it: its id, who and says.
fn of_answer(v: &Value) -> Option<Value> {
    let n = &v["next"];
    let id = n["id"].as_str().filter(|s| !s.is_empty())?;
    Some(json!({"id": id, "who": n["who"], "says": n["says"]}))
}

fn no_step() -> Value {
    json!({"id": null, "why": "next gave no step"})
}

/// A name a gp argument may take: letters, digits, `-`, `_` and one `/`
/// (a copy's theme/NAME).
fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'/')
        && !s.contains("..")
}

/// The next step for a call to `name` with `args`, from `worker` (the
/// `GP_STEP` this server runs with), the reply itself (`next`) or `ask`
/// (`gp --json progress --next ARGS`).
pub fn decide(
    name: &str,
    args: &Value,
    reply: &Value,
    worker: Option<&str>,
    ask: impl FnOnce(&[&str]) -> Result<Value, String>,
) -> Value {
    if let Some(step) = worker.filter(|s| !s.trim().is_empty()) {
        return json!({"id": step, "who": "agent", "says": WORKER_SAYS});
    }
    if name == "next" {
        return of_answer(reply).unwrap_or_else(no_step);
    }
    let mut a: Vec<&str> = vec!["progress", "--next"];
    if let Some(c) = args["copy"].as_str().filter(|c| name_ok(c)) {
        a.extend(["--copy", c]);
    } else if let Some(g) = args["game"]
        .as_str()
        .or_else(|| args["args"]["game"].as_str())
        .filter(|g| name_ok(g) && !g.contains('/'))
    {
        a.extend(["--template", g]);
    }
    match ask(&a) {
        Ok(v) => of_answer(&v).unwrap_or_else(no_step),
        Err(e) => json!({"id": null, "why": crate::activity::cut(&e, 200)}),
    }
}

/// Puts the next step in `reply` as `next_step`.
pub fn attach(config: &Config, name: &str, args: &Value, reply: &mut Reply) {
    if !reply.json.is_object() {
        return;
    }
    let worker = std::env::var(steps::STEP_ENV).ok();
    let step = decide(name, args, &reply.json, worker.as_deref(), |a| {
        let os: Vec<&std::ffi::OsStr> = a.iter().map(std::ffi::OsStr::new).collect();
        let (code, v) = run_gp(config, &os)?;
        if code != 0 {
            return Err(v["error"]
                .as_str()
                .map_or_else(|| v.to_string(), str::to_owned));
        }
        Ok(v)
    });
    reply.json["next_step"] = step;
}

/// The reply's last line of text: the next step's id, who and what.
pub fn line(reply: &Value) -> Option<String> {
    let s = &reply["next_step"];
    let id = s["id"].as_str()?;
    let who = s["who"].as_str().unwrap_or("?");
    let says = s["says"].as_str().unwrap_or("");
    Some(format!("NEXT STEP {id} ({who}): {says}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_is_told_its_own_step_and_nothing_runs() {
        let s = decide(
            "gp_run",
            &json!({}),
            &json!({}),
            Some("s0123456789ab"),
            |_| panic!("asked next"),
        );
        assert_eq!(s["id"], "s0123456789ab");
        assert_eq!(s["who"], "agent");
    }

    #[test]
    fn next_gives_its_own_step() {
        let r = json!({"next": {"id": "sabc", "who": "person", "says": "Play it."}});
        let s = decide("next", &json!({}), &r, None, |_| panic!("asked next"));
        assert_eq!(
            s,
            json!({"id": "sabc", "who": "person", "says": "Play it."})
        );
    }

    #[test]
    fn another_tool_asks_next_for_the_game_it_named() {
        let mut asked = Vec::new();
        let s = decide(
            "gp_run",
            &json!({"command": "corpus score", "args": {"game": "rooftop"}}),
            &json!({}),
            None,
            |a| {
                asked = a.iter().map(|s| s.to_string()).collect();
                Ok(json!({"next": {"id": "s1", "who": "tools", "says": "Score it."}}))
            },
        );
        assert_eq!(asked, ["progress", "--next", "--template", "rooftop"]);
        assert_eq!(s["id"], "s1");
        let mut asked = Vec::new();
        decide(
            "surface_describe",
            &json!({"copy": "theme/moss"}),
            &json!({}),
            None,
            |a| {
                asked = a.iter().map(|s| s.to_string()).collect();
                Ok(json!({}))
            },
        );
        assert_eq!(asked, ["progress", "--next", "--copy", "theme/moss"]);
    }

    #[test]
    fn a_failed_next_says_why_and_the_text_ends_with_the_step() {
        let s = decide("validate", &json!({}), &json!({}), None, |_| {
            Err("no game".into())
        });
        assert!(s["id"].is_null() && s["why"] == "no game");
        assert_eq!(line(&json!({"next_step": s})), None);
        let r = json!({"next_step": {"id": "s1", "who": "agent", "says": "Fix it."}});
        assert_eq!(line(&r).unwrap(), "NEXT STEP s1 (agent): Fix it.");
    }
}
