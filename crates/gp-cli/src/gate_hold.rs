//! Who holds a gate home's locks, and the watch that makes an orphaned gate
//! run let go of one (docs/gate.md, "The locks").
//!
//! The two gates land in different repos, so each has its own lock
//! ([`Lane`]): the model gate `gate.lock`, the tool gate `tool.lock`. A
//! model gate and a tool gate run at the same time; two of one kind run in
//! turn.
//!
//! The gate that takes a lock writes its holder file beside it: its
//! pid, its parent's, when it took the lock, what it is and the steps file
//! it logs to. While it holds the lock a thread watches it:
//!
//! - a dry run whose caller is gone (its parent pid changed: the worker or
//!   the MCP server that started it ended) stops, since no one reads its
//!   verdict;
//! - a run that logs no step to its steps file for [`IDLE`] (30 minutes;
//!   `GP_GATE_IDLE_S` sets it in seconds) stops.
//!
//! Stopping sets a flag the gate's lanes read ([`stopping`]), which ends
//! their process groups, so the gate ends with a FAIL that says why; if it
//! hasn't ended [`GRACE`] later, the process exits, which frees the lock.
//! A landing run the runner started outlives its server on purpose, so the
//! caller rule is for dry runs only. `gp loop health` reads the holder file
//! ([`health_line`]) and says PROBLEM naming the pid when the holder looks
//! stuck, or when the lock is held by a gate that names no holder (an
//! older gp).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

/// How long a gate may log no step before it gives up the lock.
/// Fifteen minutes: it was thirty, and a hung gate held tool.lock for two
/// hours on 2026-11-22 with seven runs behind it. The longest honest gap
/// (a cold build on a busy machine) was nine and a half.
pub(crate) const IDLE: Duration = Duration::from_secs(15 * 60);
/// How long a gate that was told to stop has to end on its own.
const GRACE: Duration = Duration::from_secs(30);
/// How long a gate with no steps file (the model gate) may hold the lock
/// before gp loop health calls it stuck.
const HELD_LONG: Duration = Duration::from_secs(30 * 60);

static STOP: AtomicBool = AtomicBool::new(false);
static WHY: Mutex<String> = Mutex::new(String::new());
/// The steps file of the gate this process is, once it holds its lock.
static STEPS: Mutex<Option<PathBuf>> = Mutex::new(None);
/// When this gate last wrote a beat, so a beat is written at most every
/// [`BEAT_EVERY_MS`].
static BEAT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const BEAT_EVERY_MS: u64 = 15_000;

fn steps_line(line: &Value) {
    use std::io::Write as _;
    let file = STEPS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(file) = file
        && let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(file)
    {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

/// Logs a step of the gate this process is: what it was, how long it took
/// and whether it passed, with numbers of its own (`more`: clips scored,
/// what was reused). The watch reads the file's time, so a gate that logs
/// keeps its lock and one that stops logging gives it up after [`IDLE`];
/// gp loop health reads the same lines.
pub(crate) fn step(name: &str, secs: f64, ok: bool, more: Value) {
    let mut line = json!({
        "at": crate::open::now_rfc3339(),
        "run": std::process::id(),
        "step": name,
        "secs": (secs * 10.0).round() / 10.0,
        "ok": ok,
    });
    if let (Some(l), Some(m)) = (line.as_object_mut(), more.as_object()) {
        for (k, v) in m {
            l.insert(k.clone(), v.clone());
        }
    }
    steps_line(&line);
}

/// Runs `f` as the step `name`, logged with its time.
pub(crate) fn timed<T, E>(
    name: &str,
    f: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let t = std::time::Instant::now();
    let r = f();
    step(name, t.elapsed().as_secs_f64(), r.is_ok(), json!({}));
    r
}

/// A sign of work inside a long step (a clip scored): at most one line
/// every [`BEAT_EVERY_MS`]. A step that hangs writes none, and the watch
/// then ends the gate; one that only runs long keeps its lock.
pub(crate) fn beat() {
    let now = gp_mcp::asks::now_ms();
    let last = BEAT_MS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < BEAT_EVERY_MS
        || BEAT_MS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    if STEPS.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        steps_line(
            &json!({"at": crate::open::now_rfc3339(), "run": std::process::id(), "beat": true}),
        );
    }
}

/// Whether this gate was told to stop by its watch.
pub(crate) fn stopping() -> bool {
    STOP.load(Ordering::SeqCst)
}

/// Why this gate was told to stop, once it was.
pub(crate) fn why() -> String {
    WHY.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The idle limit: `GP_GATE_IDLE_S` seconds, else [`IDLE`].
fn idle() -> Duration {
    std::env::var("GP_GATE_IDLE_S")
        .ok()
        .and_then(|s| s.parse().ok())
        .map_or(IDLE, Duration::from_secs)
}

/// Which gate a lock is for: each kind has its own lock, holder file and
/// queue in the gate's home.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    /// gp model gate: lands in the games checkout.
    Model,
    /// gp tool gate: lands in gameprinter and publishes gp.
    Tool,
}

impl Lane {
    pub(crate) const ALL: [Lane; 2] = [Lane::Model, Lane::Tool];

    fn lock_name(self) -> &'static str {
        match self {
            Lane::Model => "gate.lock",
            Lane::Tool => "tool.lock",
        }
    }

    fn holder_name(self) -> &'static str {
        match self {
            Lane::Model => "gate.holder.json",
            Lane::Tool => "tool.holder.json",
        }
    }

    /// The folder whose `queue/` holds this kind's tickets
    /// (crate::gate_queue): the home itself for the model gate, its
    /// `tool/` for the tool gate.
    pub(crate) fn queue_home(self, home: &Path) -> PathBuf {
        match self {
            Lane::Model => home.to_owned(),
            Lane::Tool => home.join("tool"),
        }
    }
}

/// The lock file of `lane` in `home`.
pub(crate) fn lock_file(home: &Path, lane: Lane) -> PathBuf {
    home.join(lane.lock_name())
}

/// `lane`'s lock in `home`, opened without emptying it.
pub(crate) fn open_lock(home: &Path, lane: Lane) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_file(home, lane))
}

pub(crate) fn holder_file(home: &Path, lane: Lane) -> PathBuf {
    home.join(lane.holder_name())
}

/// A tool gate of a gp from before the two locks holds `gate.lock` and
/// works in the same `tool/` folders: a tool gate that holds `tool.lock`
/// waits for it to end. Its holder file names it (`what` a tool gate).
/// The lock it returns is held while this gate runs, or `None` when no
/// such gate holds `gate.lock`.
pub(crate) fn wait_for_an_older_tool_gate(home: &Path) -> Option<fs::File> {
    let h = older_tool_gate(home)?;
    let lock = open_lock(home, Lane::Model).ok()?;
    say(&format!(
        "tool gate: an older gp's tool gate (pid {}) holds gate.lock: waiting for it",
        h["pid"]
    ));
    lock.lock().ok()?;
    Some(lock)
}

/// The holder of `gate.lock` when it is a tool gate (an older gp's).
fn older_tool_gate(home: &Path) -> Option<Value> {
    live_holder(home, Lane::Model).filter(|h| {
        h["what"]
            .as_str()
            .is_some_and(|w| w.starts_with("tool gate"))
    })
}

/// Whether a tool gate of an older gp runs now, holding `gate.lock`.
pub(crate) fn older_tool_gate_runs(home: &Path) -> bool {
    older_tool_gate(home).is_some()
}

fn parent() -> u32 {
    #[cfg(unix)]
    {
        std::os::unix::process::parent_id()
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// When `p` was last written, in ms since the epoch.
fn modified_ms(p: &Path) -> Option<u64> {
    let t = fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// The holder's last sign of work: its steps file's last write, or when it
/// took the lock.
fn last_ms(h: &Value) -> u64 {
    let since = h["since_ms"].as_u64().unwrap_or(0);
    h["steps"]
        .as_str()
        .and_then(|s| modified_ms(Path::new(s)))
        .map_or(since, |m| m.max(since))
}

/// Why the holder `h` should stop now, if it should: `parent_now` is its
/// parent pid as it stands.
fn why_stop(h: &Value, now_ms: u64, parent_now: u32, idle: Duration) -> Option<String> {
    let ppid = h["ppid"].as_u64().unwrap_or(0);
    if h["dry_run"] == json!(true) && ppid != 0 && u64::from(parent_now) != ppid {
        return Some(format!(
            "its caller (pid {ppid}) is gone, and no one reads a dry run's verdict"
        ));
    }
    if h["steps"].is_string() {
        let quiet = now_ms.saturating_sub(last_ms(h));
        if quiet >= idle.as_millis() as u64 {
            return Some(format!("it logged no step for {} min", quiet / 60_000));
        }
    }
    None
}

/// This gate's hold on the lock: its holder file, removed when dropped.
pub(crate) struct Hold {
    file: PathBuf,
    /// Set when the hold ends, which ends its watch.
    done: Arc<AtomicBool>,
}

impl Hold {
    /// Writes the holder file for the gate that just took `home`'s lock of
    /// `lane` and starts its watch. `what` names the gate ("tool gate dry
    /// run"), `steps` is the file it logs its steps to, and `turn` how it
    /// came to hold the lock (crate::gate_queue::take_turn): its lock line
    /// says how long it waited, how many were ahead of it when it came and
    /// how often it gave the lock back to one of them.
    pub(crate) fn take(
        home: &Path,
        lane: Lane,
        what: &str,
        steps: Option<&Path>,
        dry_run: bool,
        turn: &crate::gate_queue::Turn,
    ) -> Self {
        let h = json!({
            "pid": std::process::id(),
            "ppid": parent(),
            "since_ms": gp_mcp::asks::now_ms(),
            "what": what,
            "steps": steps.map(|p| p.display().to_string()),
            "dry_run": dry_run,
        });
        if let Some(s) = steps {
            *STEPS.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.to_owned());
            let mut more = turn.json();
            more["what"] = json!(what);
            step("lock", 0.0, true, more);
        }
        let file = holder_file(home, lane);
        let tmp = file.with_extension("tmp");
        if fs::write(&tmp, h.to_string()).is_ok() {
            let _ = fs::rename(&tmp, &file);
        }
        let done = Arc::new(AtomicBool::new(false));
        let (label, watched, ended) = (what.to_owned(), file.clone(), done.clone());
        let lock = lane.lock_name();
        std::thread::spawn(move || watch(&h, &label, lock, &watched, &ended));
        Hold { file, done }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        let mine = fs::read(&self.file)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .is_some_and(|h| h["pid"].as_u64() == Some(u64::from(std::process::id())));
        if mine {
            let _ = fs::remove_file(&self.file);
        }
    }
}

/// The watch: every few seconds, whether the gate should stop; when it
/// should, the flag, a note in its steps file, and after [`GRACE`] the
/// process's end.
fn watch(h: &Value, what: &str, lock: &str, file: &Path, done: &AtomicBool) {
    let idle = idle();
    loop {
        std::thread::sleep(Duration::from_secs(5));
        if done.load(Ordering::SeqCst) {
            return;
        }
        let Some(why) = why_stop(h, gp_mcp::asks::now_ms(), parent(), idle) else {
            continue;
        };
        let says = format!("{what} {} gives up {lock}: {why}", std::process::id());
        // The flag and the exit timer come before any writing: an orphaned
        // dry run's stderr is closed (eprintln! panics there) or full (a
        // write blocks), and either kept this watch from stopping the gate.
        *WHY.lock().unwrap_or_else(|e| e.into_inner()) = says.clone();
        STOP.store(true, Ordering::SeqCst);
        let gone = file.to_owned();
        std::thread::spawn(move || {
            std::thread::sleep(GRACE);
            let _ = fs::remove_file(&gone);
            std::process::exit(2);
        });
        note(h, &says);
        say(&format!("gate: {says}"));
        return;
    }
}

/// A note in the holder's steps file, when it has one.
fn note(h: &Value, says: &str) {
    use std::io::Write as _;
    if let Some(s) = h["steps"].as_str()
        && let Ok(mut f) = fs::OpenOptions::new().append(true).open(s)
    {
        let line = json!({"at": crate::open::now_rfc3339(), "note": says});
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

/// A line on stderr that can't panic.
fn say(line: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// The live holder of `home`'s lock of `lane`, from its holder file.
fn live_holder(home: &Path, lane: Lane) -> Option<Value> {
    let h: Value = serde_json::from_slice(&fs::read(holder_file(home, lane)).ok()?).ok()?;
    let pid = h["pid"].as_u64()?;
    gp_mcp::proc::alive_since(pid, h["since_ms"].as_u64().unwrap_or(0)).then_some(h)
}

/// Whether some process holds `home`'s lock of `lane` now.
pub(crate) fn lock_held(home: &Path, lane: Lane) -> bool {
    if !lock_file(home, lane).is_file() {
        return false;
    }
    match open_lock(home, lane) {
        Ok(f) => f.try_lock().is_err(),
        Err(_) => false,
    }
}

/// gp loop health's line on the gate home's lock of `lane`: `Ok(line)`
/// or `Err(problem)`, or `None` with no gate home.
pub(crate) fn health_line(home: &Path, lane: Lane, now_ms: u64) -> Option<Result<String, String>> {
    if !home.is_dir() {
        return None;
    }
    let lock = lock_file(home, lane).display().to_string();
    let Some(h) = live_holder(home, lane) else {
        return Some(if lock_held(home, lane) {
            Err(format!(
                "{lock} is held by a gate run that names no holder (an older gp): find it with lsof {lock}, and kill it if it has logged no step for {} min.",
                IDLE.as_secs() / 60
            ))
        } else {
            Ok(format!("{lock} is free"))
        });
    };
    let pid = h["pid"].as_u64().unwrap_or(0);
    let what = h["what"].as_str().unwrap_or("gate");
    let held = now_ms.saturating_sub(h["since_ms"].as_u64().unwrap_or(0)) / 60_000;
    let quiet = now_ms.saturating_sub(last_ms(&h)) / 60_000;
    let stuck = if h["steps"].is_string() {
        quiet >= IDLE.as_secs() / 60
    } else {
        held >= HELD_LONG.as_secs() / 60
    };
    let line = format!(
        "{lock} is held by pid {pid} ({what}) for {held} min, its last step {quiet} min ago"
    );
    Some(if stuck {
        Err(format!(
            "{line}: every other {} waits behind it. It gives the lock up by itself; if it doesn't, kill {pid}.",
            match lane {
                Lane::Model => "model gate",
                Lane::Tool => "tool gate",
            }
        ))
    } else {
        Ok(line)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dry run whose caller is gone stops; a landing run doesn't. A run
    /// that logged no step for the idle limit stops; one that did, or one
    /// with no steps file, doesn't.
    #[test]
    fn an_orphaned_or_idle_gate_is_told_to_stop() {
        let t = tempfile::tempdir().unwrap();
        let steps = t.path().join("steps.jsonl");
        fs::write(&steps, "{}\n").unwrap();
        let now = modified_ms(&steps).unwrap();
        let idle = Duration::from_secs(60);
        let h = |dry: bool, steps: Option<&Path>, since: u64| {
            json!({"pid": 1, "ppid": 500, "since_ms": since, "what": "tool gate",
                   "steps": steps.map(|p| p.display().to_string()), "dry_run": dry})
        };
        let dry = h(true, Some(&steps), now);
        assert_eq!(why_stop(&dry, now, 500, idle), None);
        assert!(
            why_stop(&dry, now, 1, idle)
                .unwrap()
                .contains("caller (pid 500) is gone")
        );
        let land = h(false, Some(&steps), now);
        assert_eq!(why_stop(&land, now, 1, idle), None);
        // No step for two minutes, the limit one.
        let later = now + 120_000;
        assert_eq!(
            why_stop(&land, later, 500, idle).as_deref(),
            Some("it logged no step for 2 min")
        );
        assert_eq!(why_stop(&land, now + 30_000, 500, idle), None);
        // The model gate logs no steps: only its caller is watched.
        let model = h(false, None, now);
        assert_eq!(why_stop(&model, later, 500, idle), None);
    }

    /// The holder file names this process while it holds the lock and goes
    /// when its hold does; health names the holder's pid, and calls it a
    /// problem once it logged no step for the idle limit.
    #[test]
    fn health_names_the_pid_holding_the_lock() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let lock = open_lock(home, Lane::Tool).unwrap();
        let now = gp_mcp::asks::now_ms();
        assert!(
            health_line(home, Lane::Tool, now)
                .unwrap()
                .unwrap()
                .ends_with("tool.lock is free")
        );
        lock.lock().unwrap();
        let steps = home.join("steps.jsonl");
        fs::write(&steps, "{}\n").unwrap();
        let hold = Hold::take(
            home,
            Lane::Tool,
            "tool gate dry run",
            Some(&steps),
            false,
            &Default::default(),
        );
        let me = std::process::id();
        let line = health_line(home, Lane::Tool, now).unwrap().unwrap();
        assert!(
            line.contains(&format!(
                "tool.lock is held by pid {me} (tool gate dry run)"
            )),
            "{line}"
        );
        // The model gate's lock is another lock: free while this is held.
        assert!(
            health_line(home, Lane::Model, now)
                .unwrap()
                .unwrap()
                .ends_with("gate.lock is free")
        );
        let later = now + IDLE.as_millis() as u64 + 60_000;
        let problem = health_line(home, Lane::Tool, later).unwrap().unwrap_err();
        assert!(problem.contains(&format!("kill {me}")), "{problem}");
        assert!(problem.contains("every other tool gate"), "{problem}");
        drop(hold);
        assert!(!holder_file(home, Lane::Tool).exists());
    }

    /// A tool gate and a model gate run at the same time: each holds its
    /// own lock, so neither waits for the other, while a second gate of
    /// the same kind does wait.
    #[test]
    fn a_tool_gate_and_a_model_gate_hold_different_locks() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let model = open_lock(home, Lane::Model).unwrap();
        model.lock().unwrap();
        let _m = Hold::take(
            home,
            Lane::Model,
            "model gate",
            None,
            false,
            &Default::default(),
        );
        let tool = open_lock(home, Lane::Tool).unwrap();
        assert!(
            tool.try_lock().is_ok(),
            "a model gate holds back no tool gate"
        );
        let _t = Hold::take(
            home,
            Lane::Tool,
            "tool gate",
            None,
            false,
            &Default::default(),
        );
        assert!(lock_held(home, Lane::Model) && lock_held(home, Lane::Tool));
        assert!(
            open_lock(home, Lane::Model).unwrap().try_lock().is_err(),
            "a second model gate waits"
        );
        assert!(
            open_lock(home, Lane::Tool).unwrap().try_lock().is_err(),
            "a second tool gate waits"
        );
        // Each holder file names its own gate.
        let what = |lane| live_holder(home, lane).unwrap()["what"].clone();
        assert_eq!(
            (what(Lane::Model), what(Lane::Tool)),
            (json!("model gate"), json!("tool gate"))
        );
        // A model gate holding gate.lock is no older tool gate to wait for.
        assert!(wait_for_an_older_tool_gate(home).is_none());
        // Each kind's tickets go in a queue of its own.
        assert_ne!(Lane::Model.queue_home(home), Lane::Tool.queue_home(home));
    }
}
