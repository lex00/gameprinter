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
    /// The games whose game work is held (the owner's "game work stops
    /// while the tooling is fixed"; `all` holds every game's): the runner
    /// starts no worker on their game rows, the finder makes no rows of
    /// their gaps and no trouble is filed for their rows, while their tool
    /// rows go on. `gp loop set --hold` sets it, `--go-on` lifts it.
    pub hold: Vec<String>,
}

impl Settings {
    /// Whether `game`'s game work is held ([`Settings::hold`]).
    pub fn held(&self, game: &str) -> bool {
        self.hold.iter().any(|g| g == "all" || g == game)
    }
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
    /// The model every `claude` worker runs on (`--model`), so two people
    /// on one ROM get the same work; empty leaves it to the person's own.
    pub model: String,
    /// Its effort (`--effort`: low, medium, high, xhigh, max); empty leaves
    /// it to the person's own.
    pub effort: String,
    /// A change to gp itself lands once its gate passes, with no card
    /// waiting for a YES: the card is a notice. On by default (the owner,
    /// 2026-11-22: card waits were 38% of a tools row's time, and half
    /// the cards were answered in under a minute). A game's change still
    /// waits for its card, since only a person judges a look.
    pub land_tools: bool,
    /// The most turns one run of a `claude` worker takes (`--max-turns`);
    /// 0 for no cap. A run past it is cut off and counts as a failed try.
    pub max_turns: u32,
    /// The most one run of a `claude` worker spends, in dollars
    /// (`--max-budget-usd`); 0 for no cap.
    pub max_spend_usd: f64,
    /// The runner starts a worker on a quest-log row only when the row
    /// has a target or says what would measure it (`measure_needed`). On
    /// by default; off starts every open row, as before 2026-11-26.
    pub measure_required: bool,
    /// How many failed tries of one step's workers make it a trouble row
    /// of the loop's own, after which the runner starts no worker on it
    /// until its row is set next again (`gp loop set --fail-tries`); 0
    /// for no limit.
    pub fail_tries: u32,
    /// The most all the workers' runs of a day spend, in dollars, counted
    /// from the runs ledger: past it the runner starts no worker that day
    /// (`gp loop set --day-spend`); 0 for no cap.
    pub day_spend_usd: f64,
    /// The machine's 1-minute load over which the runner starts no new
    /// worker (`gp loop set --max-load`); 0 for the machine's cores.
    pub max_load: f64,
}

/// How many failed tries of a step make it a trouble row when loop.toml
/// names none.
pub const WORKER_FAIL_TRIES: u32 = 3;

/// The turn cap when loop.toml names none: above the largest run in the
/// logs up to 2026-11-26 (264 turns).
pub const WORKER_MAX_TURNS: u32 = 300;
/// The spend cap of one run, in dollars, when loop.toml names none.
pub const WORKER_MAX_SPEND_USD: f64 = 30.0;

/// The model a worker runs on when loop.toml names none.
pub const WORKER_MODEL: &str = "opus";
/// Its effort when loop.toml names none.
pub const WORKER_EFFORT: &str = "high";

impl Default for Runner {
    fn default() -> Self {
        Runner {
            on: false,
            max_workers: 2,
            land_tools: true,
            spending: String::new(),
            model: WORKER_MODEL.into(),
            effort: WORKER_EFFORT.into(),
            max_turns: WORKER_MAX_TURNS,
            max_spend_usd: WORKER_MAX_SPEND_USD,
            measure_required: true,
            fail_tries: WORKER_FAIL_TRIES,
            day_spend_usd: 0.0,
            max_load: 0.0,
        }
    }
}

pub fn settings_file(home: &Path) -> PathBuf {
    home.join("loop.toml")
}

/// The home's settings; the defaults (everything off) without a file or
/// with one that doesn't read. A new home (its folder missing or empty)
/// is given its file with `steps_required` on ([`new_home_settings`]):
/// no agent is at work on it yet whose writes the setting would refuse.
pub fn settings(home: &Path) -> Settings {
    let f = settings_file(home);
    if let Ok(t) = std::fs::read_to_string(&f) {
        return toml::from_str(&t).unwrap_or_default();
    }
    if !cfg!(test) && !in_temp(home) && is_new_home(home) {
        let s = new_home_settings();
        if save(home, &s).is_ok() {
            return s;
        }
    }
    Settings::default()
}

/// What a new home starts with: `steps_required` on, the runner off.
pub fn new_home_settings() -> Settings {
    Settings {
        steps_required: true,
        ..Settings::default()
    }
}

/// Whether `home` is new: its folder is missing or holds nothing.
pub fn is_new_home(home: &Path) -> bool {
    match std::fs::read_dir(home) {
        Ok(mut d) => d.next().is_none(),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

pub fn save(home: &Path, s: &Settings) -> Result<(), String> {
    let text = format!(
        "# The loop's settings (docs/loop.md). Change them with gp loop set.\n{}",
        toml::to_string(s).map_err(|e| e.to_string())?
    );
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    std::fs::write(settings_file(home), text).map_err(|e| e.to_string())
}

/// The commands of this process's ancestors, nearest first, from the
/// process table (`ps`), read once.
fn ancestors() -> &'static [String] {
    static UP: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    UP.get_or_init(|| {
        #[cfg(unix)]
        {
            let Ok(o) = Command::new("ps")
                .args(["-A", "-o", "pid=", "-o", "ppid=", "-o", "comm="])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
            else {
                return Vec::new();
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
            let mut out = Vec::new();
            let mut at = std::os::unix::process::parent_id();
            for _ in 0..64 {
                let Some((ppid, comm)) = up.get(&at) else {
                    break;
                };
                out.push(comm.clone());
                if *ppid == at || *ppid == 0 {
                    break;
                }
                at = *ppid;
            }
            out
        }
        #[cfg(not(unix))]
        Vec::new()
    })
}

/// Whether a process name is a gp: `gp`, or a published build's own copy
/// `gp-HASH` ($GP_HOME/bin/gp-4d3b7d33b42b, what a worker's gp mcp runs),
/// so gp_run's commands from a worker aren't taken for its shell's.
fn is_gp(comm: &str) -> bool {
    let name = comm.rsplit('/').next().unwrap_or(comm);
    name == "gp"
        || name
            .strip_prefix("gp-")
            .is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Whether gp's home is a test's, in the temp folder: the loop's guards
/// don't hold for it (a worker running gp-cli's tests from its shell).
pub fn test_home() -> bool {
    std::env::var_os("GP_HOME").is_some_and(|h| in_temp(Path::new(&h)))
}

/// Whether a gp started this one, at any remove: gp serve's children, a
/// gate's checks, gp_run's commands. The runner check holds the person's
/// and an agent's own calls to it, not the loop's own machinery.
pub fn inside() -> bool {
    ancestors().iter().any(|c| is_gp(c))
}

/// A worker's own write from its shell, not through gp_run: refused, so
/// the step and the person's yes hold for it (a gate run from a shell asks
/// no one). None when this gp may go on.
pub fn worker_shell_write() -> Option<String> {
    let step = std::env::var(STEP_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    if test_home() {
        return None;
    }
    // A gp between this one and the worker's agent started it (gp_run's
    // commands, a script a gp command runs): the loop's own machinery.
    let up = ancestors();
    let agent = up
        .iter()
        .position(|c| c == "claude" || c.ends_with("/claude"))
        .unwrap_or(up.len());
    let by_gp = up[..agent].iter().any(|c| is_gp(c));
    (!by_gp).then(|| format!(
        "this is a loop worker's shell (step {step}): a gp command that writes, lands or publishes runs only through the gp_run tool, where the step and the person's yes hold. Run it with gp_run."
    ))
}

/// The runner's heartbeats: each gp serve's runner writes its own,
/// `runner-PID.json`, every look, so two servers on one home (two ports)
/// don't write over each other.
pub fn beat_file(home: &Path) -> PathBuf {
    loop_dir(home).join(format!("runner-{}.json", std::process::id()))
}

/// Every server's heartbeat on this home.
fn beats(home: &Path) -> Vec<Value> {
    std::fs::read_dir(loop_dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("runner") && n.ends_with(".json")
        })
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect()
}

/// How old a heartbeat may be and its runner still count as going.
pub const BEAT_FOR_MS: u64 = 20_000;

/// Writes the runner's heartbeat: its process, the time, and whether a
/// worker command is set.
pub fn beat(home: &Path, agent: bool) {
    let f = beat_file(home);
    let _ = std::fs::create_dir_all(loop_dir(home));
    static CLEARED: std::sync::Once = std::sync::Once::new();
    CLEARED.call_once(|| clear_ended_beats(home, crate::asks::now_ms(), crate::proc::alive_since));
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

/// How long an ended server's heartbeat is kept: gp serve's start reads
/// the last hour's to tell a restart by hand (gp-cli trouble.rs).
pub const ENDED_BEAT_KEPT_MS: u64 = 3_600_000;

/// Removes the heartbeats of servers that ended over
/// [`ENDED_BEAT_KEPT_MS`] ago: their process isn't running, or its pid
/// was given to a newer one (`alive_since(pid, at_ms)` false), which
/// `gp loop health` would otherwise one day read as a stuck runner.
pub fn clear_ended_beats(home: &Path, now: u64, alive_since: impl Fn(u64, u64) -> bool) {
    for e in std::fs::read_dir(loop_dir(home))
        .into_iter()
        .flatten()
        .flatten()
    {
        let n = e.file_name().to_string_lossy().into_owned();
        if !(n.starts_with("runner") && n.ends_with(".json")) {
            continue;
        }
        let Some(b) = std::fs::read(e.path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            continue;
        };
        let pid = b["pid"].as_u64().unwrap_or(0);
        let at = b["at_ms"].as_u64().unwrap_or(0);
        if pid == u64::from(std::process::id()) {
            continue;
        }
        if now.saturating_sub(at) > ENDED_BEAT_KEPT_MS && !alive_since(pid, at) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// `p` with its symlinks resolved, though it doesn't exist yet: its
/// nearest existing folder resolved, the rest as given. A test's home
/// (`TMP/home`, made by the first gp that writes it) must read as in the
/// temp folder, which on macOS resolves to /private/var/folders.
fn resolved(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = p;
    loop {
        if let Ok(c) = std::fs::canonicalize(at) {
            return rest.iter().rev().fold(c, |c, n| c.join(n));
        }
        match (at.parent(), at.file_name()) {
            (Some(up), Some(n)) => {
                rest.push(n.to_os_string());
                at = up;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Whether `home` is in the temp folder (a test's home).
fn in_temp(home: &Path) -> bool {
    let h = resolved(home);
    let tmp = resolved(&std::env::temp_dir());
    h.starts_with(&tmp) || h.starts_with("/tmp") || h.starts_with("/private/tmp")
}

/// Whether the runner check holds for calls on `home`: not in a gp a gp
/// started, not in a worker (`GP_STEP`), not on CI, not on a home in the
/// temp folder (tests).
fn checked(home: &Path) -> bool {
    // This crate's own tests run gp_run on homes of their own.
    if cfg!(test) {
        return false;
    }
    if std::env::var_os(STEP_ENV).is_some_and(|v| !v.is_empty())
        || std::env::var_os("CI").is_some_and(|v| !v.is_empty())
    {
        return false;
    }
    !in_temp(home)
}

/// Why the loop's runner isn't going for `home`, in words; None when it
/// is, or when the check doesn't hold here ([`checked`]). It goes when a
/// gp serve for the home beat in the last [`BEAT_FOR_MS`], the runner is
/// on, and a worker command is set.
pub fn why_no_runner(home: &Path) -> Option<String> {
    if !checked(home) {
        return None;
    }
    let now = crate::asks::now_ms();
    let fresh: Vec<Value> = beats(home)
        .into_iter()
        .filter(|b| {
            b["pid"].as_u64().is_some_and(crate::proc::alive)
                && now.saturating_sub(b["at_ms"].as_u64().unwrap_or(0)) <= BEAT_FOR_MS
        })
        .collect();
    if fresh.is_empty() {
        return Some("No gp serve is running the loop's runner for this home.".into());
    }
    if !settings(home).runner.on {
        return Some("gp serve is running, but its runner is off.".into());
    }
    if !fresh.iter().any(|b| b["agent"] == serde_json::json!(true)) {
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
    fn an_ended_servers_heartbeat_is_cleared_after_the_hour() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let dir = loop_dir(home);
        std::fs::create_dir_all(&dir).unwrap();
        let now = 10 * ENDED_BEAT_KEPT_MS;
        let put = |name: &str, pid: u64, at: u64| {
            std::fs::write(
                dir.join(name),
                serde_json::json!({"pid": pid, "at_ms": at, "agent": true}).to_string(),
            )
            .unwrap();
        };
        // 1 runs and beat just now; 2 ended long ago (or its pid went to
        // a newer process); 3 ended within the hour; 4 runs, an old beat.
        put("runner-1.json", 1, now - 1000);
        put("runner-2.json", 2, now - 2 * ENDED_BEAT_KEPT_MS);
        put("runner-3.json", 3, now - ENDED_BEAT_KEPT_MS / 2);
        put("runner-4.json", 4, now - 2 * ENDED_BEAT_KEPT_MS);
        put("runner.json", 5, now - 2 * ENDED_BEAT_KEPT_MS);
        clear_ended_beats(home, now, |pid, _| pid == 1 || pid == 4);
        let left = |n: &str| dir.join(n).exists();
        assert!(left("runner-1.json"));
        assert!(!left("runner-2.json"));
        assert!(left("runner-3.json"));
        assert!(left("runner-4.json"));
        assert!(!left("runner.json"));
    }

    #[test]
    fn a_new_home_is_a_missing_or_empty_folder_and_starts_with_steps_required() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        assert!(is_new_home(&home));
        std::fs::create_dir_all(&home).unwrap();
        assert!(is_new_home(&home));
        std::fs::create_dir_all(home.join("work")).unwrap();
        assert!(!is_new_home(&home));
        let s = new_home_settings();
        assert!(s.steps_required && !s.runner.on);
        // A home with its file keeps what it says.
        save(&home, &Settings::default()).unwrap();
        assert!(!settings(&home).steps_required);
    }

    #[test]
    fn a_home_not_made_yet_in_the_temp_folder_is_a_test_home() {
        // gp-cli's code_reading tests give gp TMP/home before anything
        // makes it; on macOS the temp folder resolves to /private/var.
        let t = tempfile::tempdir().unwrap();
        assert!(in_temp(&t.path().join("home")));
        assert!(in_temp(&t.path().join("home").join("work").join("loop")));
        assert!(in_temp(t.path()));
        assert!(!in_temp(Path::new("/Users/someone/.gameprinter")));
    }

    #[test]
    fn a_published_builds_copy_is_a_gp() {
        assert!(is_gp("gp") && is_gp("/Users/x/.gameprinter/bin/gp"));
        assert!(is_gp("/Users/x/.gameprinter/bin/gp-4d3b7d33b42b"));
        assert!(!is_gp("gp-") && !is_gp("/usr/bin/gpg") && !is_gp("gp-mcp-helper"));
    }

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
                model: "sonnet".into(),
                effort: "medium".into(),
                land_tools: true,
                max_turns: 120,
                max_spend_usd: 12.5,
                measure_required: false,
                fail_tries: 4,
                day_spend_usd: 20.0,
                max_load: 12.0,
            },
            games: vec!["rooftop".into()],
            hold: vec!["hilltop".into()],
        };
        save(&home, &s).unwrap();
        assert_eq!(settings(&home), s);
        assert!(s.held("hilltop") && !s.held("rooftop"));
        let all = Settings {
            hold: vec!["all".into()],
            ..Settings::default()
        };
        assert!(all.held("rooftop") && !Settings::default().held("rooftop"));
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
