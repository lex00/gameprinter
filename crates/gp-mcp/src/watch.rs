//! The agent in a watch session (docs/watch.md): start one, follow its
//! events, watch addresses and hook instructions on its recipes, ask for a
//! window's instructions, search again, point at the screen, and read what
//! the person's page shows. The session's events have one writer, a `gp
//! watch` process: the tools that add to them run one (in the background
//! for a search, to the end for a rerun). Every call is appended to the
//! session's `inputs.ndjson` as a `tool` input from the agent, so the
//! person sees it in the page's feed.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gp_events::watch::WatchInput;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Config, Reply};

/// The most events `watch_events` returns at once.
pub const PAGE: usize = 500;
/// The longest `watch_events` waits for new events.
const MAX_WAIT: Duration = Duration::from_secs(30);

fn session_dir(config: &Config, name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || name.len() > 80
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "a session name is letters, digits, - and _ (got {name:?})"
        ));
    }
    Ok(config.home.join("watch").join(name))
}

fn existing(config: &Config, name: &str) -> Result<PathBuf, String> {
    let d = session_dir(config, name)?;
    if !d.join("session.json").is_file() {
        return Err(format!("there's no watch session `{name}`"));
    }
    Ok(d)
}

/// Appends an input to a session, in one write.
fn append(dir: &Path, input: &WatchInput) -> Result<(), String> {
    use std::io::Write as _;
    input.check()?;
    let mut line = serde_json::to_string(input).map_err(|e| e.to_string())?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("inputs.ndjson"))
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| e.to_string())
}

/// Logs a watch tool's call in its session as the agent's `tool` input.
pub fn log_call(config: &Config, tool: &str, args: &Value, reply: &Reply) {
    let Some(name) = args.get("name").and_then(Value::as_str) else {
        return;
    };
    let Ok(dir) = existing(config, name) else {
        return;
    };
    let result = gp_capture::summarize(&reply.json, crate::capture::SUMMARY_CHARS);
    let _ = append(
        &dir,
        &WatchInput {
            v: 1,
            actor: "agent".into(),
            kind: "tool".into(),
            data: json!({
                "tool": tool,
                "args": gp_capture::summarize(args, crate::capture::SUMMARY_CHARS),
                "result": if reply.is_error { format!("refused: {result}") } else { result },
            }),
        },
    );
}

/// Runs `gp` in the background (a session's writer), waiting until `ready`
/// holds or it exits. A `gp` that exits with an error is the call's error.
fn spawn(config: &Config, args: &[String], ready: impl Fn() -> bool) -> Result<(), String> {
    let mut child = std::process::Command::new(&config.gp)
        .args(args)
        .env("GP_STORE", config.store())
        .env("GP_HOME", &config.home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("running {}: {e}", config.gp.display()))?;
    let t0 = Instant::now();
    loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            if st.success() {
                return Ok(());
            }
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                use std::io::Read as _;
                let _ = e.read_to_string(&mut err);
            }
            return Err(err.trim().to_owned());
        }
        if ready() || t0.elapsed() > Duration::from_secs(20) {
            // Reap it when it ends, so it leaves no zombie.
            std::thread::spawn(move || child.wait());
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartArgs {
    /// The session's name: letters, digits, - and _.
    pub name: String,
    /// The iNES ROM's path.
    pub rom: String,
    /// Candidates the search runs, at most.
    #[serde(default)]
    pub budget: Option<usize>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// `unrun`, `unread` or `stops` (gp explore --aim).
    #[serde(default)]
    pub aim: Option<String>,
    /// Routines to aim at, by the harvest's names.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Seeded runs of random presses among stage 0's starts.
    #[serde(default)]
    pub start_runs: Option<usize>,
    /// Frames each start runs.
    #[serde(default)]
    pub start_frames: Option<usize>,
    /// Replace a session of this name.
    #[serde(default)]
    pub replace: bool,
}

/// `watch_start`: `gp watch start` in the background.
pub fn start(config: &Config, a: StartArgs) -> Result<Reply, String> {
    let dir = session_dir(config, &a.name)?;
    let mut args = vec![
        "watch".to_owned(),
        "start".into(),
        a.name.clone(),
        a.rom.clone(),
    ];
    let mut opt = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            args.push(k.to_owned());
            args.push(v);
        }
    };
    opt("--budget", a.budget.map(|v| v.to_string()));
    opt("--seed", a.seed.map(|v| v.to_string()));
    opt("--aim", a.aim.clone());
    opt("--start-runs", a.start_runs.map(|v| v.to_string()));
    opt("--start-frames", a.start_frames.map(|v| v.to_string()));
    for t in &a.targets {
        args.push("--target".into());
        args.push(t.clone());
    }
    if a.replace {
        args.push("--replace".into());
    }
    let events = dir.join("events.ndjson");
    spawn(config, &args, || events.is_file())?;
    Ok(Reply::ok(json!({
        "name": a.name, "started": true, "dir": dir,
        "follow": "watch_events with after set to the last seq read",
    })))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventsArgs {
    pub name: String,
    /// The last seq already read; events after it are returned.
    #[serde(default)]
    pub after: u64,
    /// Only these kinds (`bank`, `stop.reached`, `clip.kept`).
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Wait up to this many milliseconds (at most 30000) for an event after
    /// `after`, while the session's writer is writing.
    #[serde(default)]
    pub wait_ms: u64,
}

/// `watch_events`: the session's events after a seq, waiting for new ones
/// when asked.
pub fn events(config: &Config, a: EventsArgs) -> Result<Reply, String> {
    let dir = existing(config, &a.name)?;
    let until = Instant::now() + Duration::from_millis(a.wait_ms).min(MAX_WAIT);
    loop {
        let writing = gp_capture::writer_busy(&dir);
        let all = gp_capture::events_after(&dir, a.after, usize::MAX).map_err(|e| e.to_string())?;
        let last = all
            .last()
            .and_then(|e| e["seq"].as_u64())
            .unwrap_or(a.after);
        let page: Vec<Value> = all
            .into_iter()
            .filter(|e| {
                a.kinds.is_empty()
                    || e["kind"]
                        .as_str()
                        .is_some_and(|k| a.kinds.iter().any(|x| x == k))
            })
            .take(PAGE)
            .collect();
        if last > a.after || !writing || Instant::now() >= until {
            // A full page stops at its last event; otherwise every event
            // after `after` was looked at.
            let next = if page.len() == PAGE {
                page.last().and_then(|e| e["seq"].as_u64()).unwrap_or(last)
            } else {
                last
            };
            return Ok(Reply::ok(json!({
                "name": a.name, "events": page, "last_seq": next, "writing": writing,
            })));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchArgs {
    pub name: String,
    /// Addresses whose writes to report: `$F0`, `0x6F0`.
    #[serde(default)]
    pub addrs: Vec<String>,
    /// Instructions to hook: `01:8000`, `C27D`, `sub_C1FB`.
    #[serde(default)]
    pub at: Vec<String>,
}

/// `watch_watch` and `watch_hook`: `gp watch rerun` on the session.
pub fn rerun(config: &Config, a: WatchArgs) -> Result<Reply, String> {
    existing(config, &a.name)?;
    if a.addrs.is_empty() && a.at.is_empty() {
        return Err("name an address to watch or an instruction to hook".into());
    }
    let mut args: Vec<std::ffi::OsString> = vec!["watch".into(), "rerun".into(), (&a.name).into()];
    for x in &a.addrs {
        args.push("--watch".into());
        args.push(x.into());
    }
    for x in &a.at {
        args.push("--hook".into());
        args.push(x.into());
    }
    let refs: Vec<&std::ffi::OsStr> = args.iter().map(|s| s.as_os_str()).collect();
    let (code, v) = crate::run_gp(config, &refs)?;
    let mut v = v.get("data").cloned().unwrap_or(v);
    v["follow"] = json!("watch_events with kinds watch, hook and rerun.end");
    Ok(if code == 0 {
        Reply::ok(v)
    } else {
        Reply::error(v.to_string())
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailArgs {
    pub name: String,
    /// The recipe's hash, as an event's `run` gives it.
    pub recipe: String,
    /// Frames `A..B` (without B) or `A..=B`.
    pub frames: String,
    /// The most trace lines returned (default 200).
    #[serde(default)]
    pub max_lines: Option<usize>,
}

/// `watch_detail`: L3 for a window, as `gp reach --detail` gives it.
pub fn detail(config: &Config, a: DetailArgs) -> Result<Reply, String> {
    existing(config, &a.name)?;
    let args = [
        "watch", "detail", &a.name, "--recipe", &a.recipe, "--frames", &a.frames,
    ];
    let refs: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    let (code, v) = crate::run_gp(config, &refs)?;
    let mut v = v.get("data").cloned().unwrap_or(v);
    if code != 0 {
        return Ok(Reply::error(v.to_string()));
    }
    let trace = v["trace"].as_str().unwrap_or("").to_owned();
    let lines: Vec<&str> = trace.lines().collect();
    let max = a.max_lines.unwrap_or(200);
    v["lines"] = json!(lines.len());
    if lines.len() > max {
        v["trace"] = json!(lines[..max].join("\n") + "\n");
        v["cut"] = json!(true);
    }
    v["trace_hash"] = json!(gp_evidence::Hash::of(trace.as_bytes()).to_string());
    Ok(Reply::ok(v))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExploreArgs {
    pub name: String,
    #[serde(default)]
    pub budget: Option<usize>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// `stops` (the default), `unrun` or `unread`.
    #[serde(default)]
    pub aim: Option<String>,
    #[serde(default)]
    pub targets: Vec<String>,
}

/// `watch_explore`: `gp watch explore` in the background.
pub fn explore(config: &Config, a: ExploreArgs) -> Result<Reply, String> {
    let dir = existing(config, &a.name)?;
    if gp_capture::writer_busy(&dir) {
        return Err(format!(
            "session {} is being written; wait for its watch.end",
            a.name
        ));
    }
    let mut args = vec!["watch".to_owned(), "explore".into(), a.name.clone()];
    if let Some(b) = a.budget {
        args.extend(["--budget".into(), b.to_string()]);
    }
    if let Some(s) = a.seed {
        args.extend(["--seed".into(), s.to_string()]);
    }
    if let Some(m) = &a.aim {
        args.extend(["--aim".into(), m.clone()]);
    }
    for t in &a.targets {
        args.extend(["--target".into(), t.clone()]);
    }
    let before = gp_capture::last_seq(&dir).unwrap_or(0);
    spawn(config, &args, || {
        gp_capture::last_seq(&dir).unwrap_or(0) > before
    })?;
    Ok(Reply::ok(json!({
        "name": a.name, "started": true, "after": before,
        "follow": "watch_events from after; the search ends with watch.end",
    })))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PointArgs {
    pub name: String,
    /// The recipe's hash.
    pub recipe: String,
    pub frame: u64,
    /// The box in picture pixels: x, y, width, height.
    pub rect: [u16; 4],
}

/// `watch_point`: the sprites under a box and the routine that wrote each.
pub fn point(config: &Config, a: PointArgs) -> Result<Reply, String> {
    let dir = existing(config, &a.name)?;
    let rect = format!("{},{},{},{}", a.rect[0], a.rect[1], a.rect[2], a.rect[3]);
    let frame = a.frame.to_string();
    let args = [
        "watch", "point", &a.name, "--recipe", &a.recipe, "--frame", &frame, "--rect", &rect,
    ];
    let refs: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    let (code, v) = crate::run_gp(config, &refs)?;
    let v = v.get("data").cloned().unwrap_or(v);
    if code != 0 {
        return Ok(Reply::error(v.to_string()));
    }
    append(
        &dir,
        &WatchInput {
            v: 1,
            actor: "agent".into(),
            kind: "point".into(),
            data: json!({"recipe": a.recipe, "frame": a.frame, "rect": a.rect, "answer": v}),
        },
    )?;
    Ok(Reply::ok(v))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewArgs {
    pub name: String,
}

/// `view_now`: what the person's page shows now, and the last point.
pub fn view_now(config: &Config, a: ViewArgs) -> Result<Reply, String> {
    let dir = existing(config, &a.name)?;
    let text = std::fs::read_to_string(dir.join("inputs.ndjson")).unwrap_or_default();
    let inputs: Vec<WatchInput> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let last = |kind: &str| inputs.iter().rev().find(|i| i.kind == kind).cloned();
    Ok(Reply::ok(json!({
        "name": a.name,
        "view": last("view").map(|i| i.data),
        "point": last("point").map(|i| i.data),
        "inputs": inputs.len(),
    })))
}
