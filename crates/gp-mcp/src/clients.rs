//! The agents that connected to gp mcp, kept in `$GP_HOME/agents.json`:
//! each client's name and version from its `initialize`, the gp serving
//! it, and when. gp is meant to be used through an agent, so a person
//! running gp by hand while none has connected this week is told how to
//! connect one: `gp self check`'s Agent line, a stderr line at most once a
//! day ([`nudge_due`]), and a line on gp serve's page ([`no_agent`]).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The file under `$GP_HOME`.
pub const FILE: &str = "agents.json";
/// When the last stderr line was printed, under `$GP_HOME`.
const NUDGED: &str = "agents-nudged";
/// How long a connection counts as "an agent is connected".
pub const WEEK_MS: u64 = 7 * 24 * 3600 * 1000;
/// How often the stderr line comes at most.
pub const NUDGE_EVERY_MS: u64 = 24 * 3600 * 1000;
/// What a person running gp by hand is told while no agent connected this
/// week.
pub const LINE: &str = "gp works through your agent, and none has connected to gp mcp this week; run gp mcp setup to connect one";

fn file(home: &Path) -> PathBuf {
    home.join(FILE)
}

/// Every client that connected, newest first: name, version, gp, at_ms,
/// and worker (a loop worker's session).
pub fn seen(home: &Path) -> Vec<Value> {
    let v: Value = std::fs::read_to_string(file(home))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let mut out: Vec<Value> = v["clients"]
        .as_object()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default();
    out.sort_by_key(|c| std::cmp::Reverse(c["at_ms"].as_u64().unwrap_or(0)));
    out
}

/// Records a client from its initialize params (as JSON), served by gp
/// `gp` at `now_ms`: one entry per client name, the newest kept.
pub fn record(home: &Path, start: &Value, gp: &str, now_ms: u64) -> Result<(), String> {
    let info = &start["clientInfo"];
    let name = info["name"]
        .as_str()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or("an MCP client");
    let mut clients = serde_json::Map::new();
    for c in seen(home) {
        if let Some(n) = c["name"].as_str() {
            clients.insert(n.to_owned(), c.clone());
        }
    }
    let worker = std::env::var_os(crate::steps::STEP_ENV).is_some_and(|v| !v.is_empty());
    clients.insert(
        name.to_owned(),
        json!({
            "name": name,
            "version": info["version"].as_str().unwrap_or(""),
            "gp": gp,
            "at_ms": now_ms,
            "worker": worker,
        }),
    );
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let tmp = home.join(format!(".{FILE}.{}", std::process::id()));
    let all = json!({ "clients": clients });
    let text = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, file(home)).map_err(|e| e.to_string())
}

/// The newest client that connected within `within_ms` of `now_ms`.
pub fn recent(home: &Path, now_ms: u64, within_ms: u64) -> Option<Value> {
    seen(home)
        .into_iter()
        .next()
        .filter(|c| now_ms.saturating_sub(c["at_ms"].as_u64().unwrap_or(0)) <= within_ms)
}

/// [`LINE`] while no agent connected in the last week, else None.
pub fn no_agent(home: &Path, now_ms: u64) -> Option<&'static str> {
    recent(home, now_ms, WEEK_MS).is_none().then_some(LINE)
}

/// Whether the stderr line is due at `now_ms`: no agent this week and
/// none printed in the last day. When it is, the time is kept, so the
/// next one comes a day later.
pub fn nudge_due(home: &Path, now_ms: u64) -> bool {
    if no_agent(home, now_ms).is_none() {
        return false;
    }
    let stamp = home.join(NUDGED);
    let last = std::fs::read_to_string(&stamp)
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok());
    if last.is_some_and(|l| now_ms.saturating_sub(l) < NUDGE_EVERY_MS) {
        return false;
    }
    std::fs::write(&stamp, now_ms.to_string()).is_ok()
}

/// How long ago `at_ms` was at `now_ms`, in words.
pub fn ago(at_ms: u64, now_ms: u64) -> String {
    let s = now_ms.saturating_sub(at_ms) / 1000;
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_that_connects_is_kept_and_the_line_stops() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let now = 1_000 * WEEK_MS;
        assert_eq!(no_agent(home, now), Some(LINE));
        // The line comes once a day at most.
        assert!(nudge_due(home, now));
        assert!(!nudge_due(home, now + 1000));
        assert!(nudge_due(home, now + NUDGE_EVERY_MS));
        let start = json!({"clientInfo": {"name": "claude-code", "version": "2.1.0"}});
        record(home, &start, "0.1.2", now).unwrap();
        let codex = json!({"clientInfo": {"name": "codex"}});
        record(home, &codex, "0.1.2", now - 5).unwrap();
        let s = seen(home);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0]["name"], "claude-code");
        assert_eq!(s[0]["version"], "2.1.0");
        assert_eq!(s[0]["gp"], "0.1.2");
        assert_eq!(no_agent(home, now), None);
        assert!(!nudge_due(home, now + 2 * NUDGE_EVERY_MS));
        // A week later it says so again.
        assert_eq!(no_agent(home, now + WEEK_MS + 1), Some(LINE));
        // A client with no name is kept too.
        record(home, &json!({}), "0.1.2", now + 1).unwrap();
        assert_eq!(seen(home)[0]["name"], "an MCP client");
        assert_eq!(ago(now - 3 * 86_400_000, now), "3 days ago");
        assert!(!LINE.contains('\n'));
    }
}
