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
    /// The games whose quest-log items the loop works on; none means every
    /// game's.
    pub games: Vec<String>,
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

/// Whether a gp started this one, at any remove: gp serve's children, a
/// gate's checks, gp_run's commands. The runner check holds the person's
/// and an agent's own calls to it, not the loop's own machinery. Read
/// from the process table (`ps`) once.
pub fn inside() -> bool {
    static INSIDE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INSIDE.get_or_init(|| {
        #[cfg(unix)]
        {
            let Ok(o) = Command::new("ps")
                .args(["-A", "-o", "pid=", "-o", "ppid=", "-o", "comm="])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
            else {
                return false;
            };
            let t = String::from_utf8_lossy(&o.stdout);
            let mut up = std::collections::HashMap::new();
            for l in t.lines() {
                let mut w = l.split_whitespace();
                let (Some(pid), Some(ppid)) = (w.next(), w.next()) else {
                    continue;
                };
                let comm = w.collect::<Vec<_>>().join(" ");
                if let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) {
                    up.insert(pid, (ppid, comm));
                }
            }
            let mut at = std::os::unix::process::parent_id();
            for _ in 0..64 {
                let Some((ppid, comm)) = up.get(&at) else {
                    break;
                };
                if comm == "gp" || comm.ends_with("/gp") {
                    return true;
                }
                if *ppid == at || *ppid == 0 {
                    break;
                }
                at = *ppid;
            }
            false
        }
        #[cfg(not(unix))]
        false
    })
}

/// The runner's heartbeat: gp serve's runner writes it every look.
pub fn beat_file(home: &Path) -> PathBuf {
    loop_dir(home).join("runner.json")
}

/// How old a heartbeat may be and its runner still count as going.
pub const BEAT_FOR_MS: u64 = 20_000;

/// Writes the runner's heartbeat: its process, the time, and whether a
/// worker command is set.
pub fn beat(home: &Path, agent: bool) {
    let f = beat_file(home);
    let _ = std::fs::create_dir_all(loop_dir(home));
    let v = serde_json::json!({
        "pid": std::process::id(),
        "at_ms": crate::asks::now_ms(),
        "agent": agent,
    });
    let tmp = f.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec(&v).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &f);
    }
}

/// Whether the runner check holds for calls on `home`: not in a gp a gp
/// started, not in a worker (`GP_STEP`), not on CI, not on a home in the
/// temp folder (tests).
fn checked(home: &Path) -> bool {
    if std::env::var_os(STEP_ENV).is_some_and(|v| !v.is_empty())
        || std::env::var_os("CI").is_some_and(|v| !v.is_empty())
    {
        return false;
    }
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let h = canon(home);
    let tmp = canon(&std::env::temp_dir());
    !(h.starts_with(&tmp) || h.starts_with("/tmp") || h.starts_with("/private/tmp"))
}

/// Why the loop's runner isn't going for `home`, in words; None when it
/// is, or when the check doesn't hold here ([`checked`]). It goes when a
/// gp serve for the home beat in the last [`BEAT_FOR_MS`], the runner is
/// on, and a worker command is set.
pub fn why_no_runner(home: &Path) -> Option<String> {
    if !checked(home) {
        return None;
    }
    let beat: Option<Value> = std::fs::read(beat_file(home))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let now = crate::asks::now_ms();
    let fresh = beat.as_ref().is_some_and(|b| {
        b["pid"].as_u64().is_some_and(crate::proc::alive)
            && now.saturating_sub(b["at_ms"].as_u64().unwrap_or(0)) <= BEAT_FOR_MS
    });
    if !fresh {
        return Some("No gp serve is running the loop's runner for this home.".into());
    }
    if !settings(home).runner.on {
        return Some("gp serve is running, but its runner is off.".into());
    }
    if !beat
        .as_ref()
        .is_some_and(|b| b["agent"] == serde_json::json!(true))
    {
        return Some(
            "The runner is on, but no worker command is set, so it starts nothing.".into(),
        );
    }
    None
}

/// [`why_no_runner`], unless a gp started this process ([`inside`]).
pub fn no_runner_here(home: &Path) -> Option<String> {
    let why = why_no_runner(home)?;
    (!inside()).then_some(why)
}

/// The error a call outside the loop gets while no runner goes.
pub fn no_runner_says(why: &str) -> String {
    let bar = "=".repeat(72);
    format!(
        "{bar}\nNO RUNNER: THE LOOP ISN'T RUNNING. Nothing moves on its own.\n{why}\nStart it: gp serve (it runs the runner), gp loop set --runner on, and a worker\ncommand (gp serve --clip-agent CMD, or $GP_CLIP_AGENT).\nA write from outside the loop is refused until then; reads still run.\n{bar}"
    )
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
            games: vec!["tapper".into()],
        };
        save(&home, &s).unwrap();
        assert_eq!(settings(&home), s);
    }

    #[test]
    fn a_home_in_the_temp_folder_is_not_held_to_the_runner() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(why_no_runner(t.path()), None);
    }

    #[test]
    fn the_error_says_why_and_how_to_start_it() {
        let s = no_runner_says("gp serve is running, but its runner is off.");
        assert!(
            s.contains("NO RUNNER")
                && s.contains("its runner is off")
                && s.contains("gp loop set --runner on")
        );
    }
}
