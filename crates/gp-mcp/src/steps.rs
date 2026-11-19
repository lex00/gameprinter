//! The loop's state as gp_run and gp serve read it (docs/loop.md, issue
//! #43): the home's loop settings (`$GP_HOME/loop.toml`), the steps the
//! engine has open now (`gp loop steps`, crates/gp-cli/src/steps.rs), and
//! the steps the runner's live workers hold.
//!
//! With `steps_required` on, gp_run runs a call that writes, lands or
//! publishes only as a step the loop gave: the call cites the step's id
//! (`step`) and the step runs the call's command (policy::decide_step).
//! Reads stay free. A worker the runner started has `GP_STEP` set, and its
//! calls may cite no other step, with the setting on or off.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Config;

/// The variable a worker's gp is started with: the step it works on.
pub const STEP_ENV: &str = "GP_STEP";

/// `$GP_HOME/loop.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// gp_run runs a write, a landing or a publish only as an open step.
    pub steps_required: bool,
    pub runner: Runner,
}

/// gp serve's runner (crates/gp-cli/src/serve/runner.rs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Runner {
    /// It starts workers. Off by default: each is an agent, which costs.
    pub on: bool,
    /// How many workers at once, at most.
    pub max_workers: u32,
    /// The owner's note on spending, shown on the page beside the runner.
    pub spending: String,
}

impl Default for Runner {
    fn default() -> Self {
        Runner {
            on: false,
            max_workers: 2,
            spending: String::new(),
        }
    }
}

pub fn settings_file(home: &Path) -> PathBuf {
    home.join("loop.toml")
}

/// The home's settings; the defaults (everything off) without a file or
/// with one that doesn't read.
pub fn settings(home: &Path) -> Settings {
    std::fs::read_to_string(settings_file(home))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(home: &Path, s: &Settings) -> Result<(), String> {
    let text = format!(
        "# The loop's settings (docs/loop.md). Change them with gp loop set.\n{}",
        toml::to_string(s).map_err(|e| e.to_string())?
    );
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    std::fs::write(settings_file(home), text).map_err(|e| e.to_string())
}

/// The loop's own folder in the home.
pub fn loop_dir(home: &Path) -> PathBuf {
    home.join("work").join("loop")
}

/// The runner's workers' records, one `ID.json` each.
pub fn workers_dir(home: &Path) -> PathBuf {
    loop_dir(home).join("workers")
}

/// A worker's states that hold its step.
pub const LIVE: &[&str] = &["working", "attempting", "waiting for a yes", "retrying"];

/// The records of the runner's workers, oldest first.
pub fn workers(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(workers_dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
        .collect();
    out.sort_by_key(|v| v["started_ms"].as_u64().unwrap_or(0));
    out
}

/// Whether a worker's record holds its step: a live state, and its
/// process still running.
pub fn holds(w: &Value) -> bool {
    LIVE.contains(&w["state"].as_str().unwrap_or(""))
        && w["pid"].as_u64().is_some_and(crate::proc::alive)
}

/// The steps the runner's live workers hold: a step stays theirs while
/// they work, though the facts it was decided from change as they do.
pub fn held(home: &Path) -> Vec<Value> {
    workers(home)
        .into_iter()
        .filter(holds)
        .map(|w| w["step"].clone())
        .filter(|s| s["id"].is_string())
        .collect()
}

/// The steps open now, as `gp loop steps` gives them (`open`, `halted`),
/// with the home and games checkout of this server.
pub fn current(config: &Config) -> Result<Value, String> {
    let mut cmd = Command::new(&config.gp);
    cmd.args(["--json", "loop", "steps"])
        .env(crate::CALLER_ENV, "mcp")
        .env("GP_NO_HINTS", "1")
        .env("GP_HOME", &config.home)
        .stdin(Stdio::null());
    if let Some(g) = &config.games {
        cmd.env("GP_GAMES_DIR", g);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("running {}: {e}", config.gp.display()))?;
    if !out.status.success() {
        return Err(format!(
            "gp loop steps failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("gp loop steps printed no JSON ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_are_off_without_a_file_and_keep_what_is_saved() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        assert_eq!(settings(&home), Settings::default());
        assert!(!settings(&home).steps_required && !settings(&home).runner.on);
        let s = Settings {
            steps_required: true,
            runner: Runner {
                on: false,
                max_workers: 3,
                spending: "about 5 dollars a day".into(),
            },
        };
        save(&home, &s).unwrap();
        assert_eq!(settings(&home), s);
    }
}
