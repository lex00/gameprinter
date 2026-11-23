//! `gp tool gate`: a patch to the tool repo (`gameprinter`), held to the
//! tool's own checks. A worktree in the gate's home is put at the branch
//! tip, the patch is applied and built, and `just check` and the tests run
//! at once. `just check` is the game check: Moss Hop, built from its source
//! with no ROM, held to its baselines, in the interpreter, in the page's
//! players and in web parity, all through the patched gp. The gate reads no
//! ROM and no games checkout. What passes lands on the branch and is
//! published at $GP_HOME/bin/gp. Keeps the shape of `gate.rs` and shares its
//! apply, hook, tree and landing code.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::gate::{self, Failed, Patch, git};
use crate::publish;

/// The profile the gate builds and publishes.
const PROFILE: &str = "fastdev";
const TIP_TRIES: usize = 5;

#[derive(Args)]
pub struct ToolGateArgs {
    /// The patch: `git diff` output, or a one-commit `git format-patch`
    /// mail, which keeps its own message and author.
    #[arg(long)]
    pub patch: PathBuf,
    /// The commit message, for a patch that isn't a format-patch mail.
    #[arg(long, short)]
    pub message: Option<String>,
    /// The branch to land on (default: the one checked out here).
    #[arg(long)]
    pub onto: Option<String>,
    /// The gate's home (default: $GP_GATE_HOME, else ~/.gameprinter/gate).
    /// Its worktree and target folders are kept there.
    #[arg(long)]
    pub home: Option<PathBuf>,
    /// Say PASS or FAIL and land nothing.
    #[arg(long)]
    pub dry_run: bool,
}

struct Cx {
    repo: PathBuf,
    onto: String,
    /// The gate's worktree of the tool, under the gate's home.
    tree: PathBuf,
    /// One target folder for every try, `<home>/tool/target`.
    target: PathBuf,
    /// The person's $GP_HOME: where builds are published.
    home: PathBuf,
    steps: Steps,
    /// The programs `just check` runs that must be on PATH ([`TOOLS`]).
    tools: &'static [&'static str],
}

/// What `just check` needs on PATH, looked for before anything is built so
/// a missing one fails the gate in a second, not at minute ten: just, the
/// node tools of the page checks and the editor, python3, git, and cc65's
/// ca65 and ld65 for Moss Hop. sccache is optional (scripts/cargo/locked.py
/// runs without it), so it isn't here.
const TOOLS: &[&str] = &[
    "just", "node", "npm", "npx", "python3", "git", "ca65", "ld65",
];

/// A line on stderr, never a panic: a dry run whose caller ended has a
/// closed stderr, where `eprintln!` panics (a lane that panicked there
/// left the gate holding tool.lock with no step logged, 2026-11-22).
fn progress(s: impl AsRef<str>) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "tool gate: {}", s.as_ref());
}

/// How long each of the gate's steps took: a `step NAME: S s` line on
/// stderr and a JSON line in `<gate home>/tool/steps.jsonl`, with the
/// patch's hash as the run and the seconds since the gate started.
struct Steps {
    file: PathBuf,
    run: String,
    t0: std::time::Instant,
}

impl Steps {
    fn new(file: PathBuf, run: &str) -> Self {
        Steps {
            file,
            run: run.to_owned(),
            t0: std::time::Instant::now(),
        }
    }

    /// Runs `f` as the step `step`; `ok` says whether what it gave is a pass.
    fn time<T>(&self, step: &str, f: impl FnOnce() -> T, ok: impl Fn(&T) -> bool) -> T {
        let t = std::time::Instant::now();
        let v = f();
        self.record(step, t.elapsed().as_secs_f64(), ok(&v));
        v
    }

    fn record(&self, step: &str, secs: f64, ok: bool) {
        progress(format!(
            "step {step}: {secs:.1} s{}",
            if ok { "" } else { " (failed)" }
        ));
        self.write(json!({
            "at": crate::open::now_rfc3339(),
            "run": self.run,
            "step": step,
            "secs": (secs * 10.0).round() / 10.0,
            "ok": ok,
            "since_start": (self.t0.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        }));
    }

    /// A line about the run that isn't a timing, such as the files a check
    /// changed.
    fn note(&self, what: &str) {
        progress(format!("step note: {what}"));
        self.write(json!({"at": crate::open::now_rfc3339(), "run": self.run, "note": what}));
    }

    fn write(&self, line: Value) {
        use std::io::Write as _;
        if let Some(d) = self.file.parent() {
            let _ = fs::create_dir_all(d);
        }
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
        {
            // One write, so lines from lanes and test runs at once don't mix.
            let _ = f.write_all(format!("{line}\n").as_bytes());
        }
    }

    /// The `check-step: NAME S s` lines `just check` printed
    /// (scripts/check/step.sh), each recorded as the step `check/NAME`.
    fn check_lines(&self, out: &str) {
        for (name, secs, ok) in check_steps(out) {
            self.record(&format!("check/{name}"), secs, ok);
        }
    }
}

/// `(name, seconds, passed)` of each `check-step: NAME S s` line in `out`;
/// a failed step's line ends in `(failed)`.
fn check_steps(out: &str) -> Vec<(String, f64, bool)> {
    out.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("check-step: ")?;
            let mut w = rest.split_whitespace();
            let name = w.next()?.to_owned();
            let secs = w.next()?.parse().ok()?;
            (w.next()? == "s").then_some(())?;
            Some((name, secs, w.next() != Some("(failed)")))
        })
        .collect()
}

/// `cargo`, through the machine's build lock when it's installed.
fn cargo(c: &Cx) -> Command {
    let locked = c.home.join("bin/locked/cargo");
    let program = match std::env::var_os("GP_CARGO") {
        Some(p) => PathBuf::from(p),
        None if locked.is_file() => locked,
        None => PathBuf::from("cargo"),
    };
    let mut cmd = Command::new(program);
    gate_env(&mut cmd);
    // The gate's two targets (the build's and the tests') and every tree
    // share compiled crates through sccache when this machine has it, so a
    // gate builds only what the patch changed.
    if std::env::var_os("RUSTC_WRAPPER").is_none() && which("sccache") {
        cmd.env("RUSTC_WRAPPER", "sccache");
        if std::env::var_os("SCCACHE_DIR").is_none() {
            cmd.env("SCCACHE_DIR", c.home.join("work/sccache"));
        }
    }
    cmd
}

/// Whether `name` is a program on PATH.
fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// The gate's own build settings for a child that builds: the profile's
/// `incremental = true` (Cargo.toml, fastdev) stands, so a caller's
/// CARGO_INCREMENTAL=0, or a CARGO_PROFILE_FASTDEV_INCREMENTAL, doesn't
/// reach it and make every build in the kept target a whole one.
/// GP_GATE_NO_INCREMENTAL=1 keeps the caller's (after an incremental-cache
/// failure, say).
///
/// A loop worker's gate runs with the worker's GP_STEP and GP_WORKER set;
/// the tests' own servers would cite that step and be refused, so they
/// never reach a child.
fn gate_env(cmd: &mut Command) -> &mut Command {
    cmd.env_remove("GP_STEP").env_remove("GP_WORKER");
    if std::env::var_os("GP_GATE_NO_INCREMENTAL").is_some_and(|v| v == "1") {
        return cmd;
    }
    cmd.env_remove("CARGO_INCREMENTAL")
        .env_remove("CARGO_PROFILE_FASTDEV_INCREMENTAL")
}

/// `Err` carrying the command's last lines.
fn run_step(cmd: &mut Command, what: &str) -> std::result::Result<(), String> {
    run_out(cmd, what).map(|_| ())
}

/// The command's output, stdout then stderr, or `Err` with its last lines.
fn run_out(cmd: &mut Command, what: &str) -> std::result::Result<String, String> {
    let o = cmd.output().map_err(|e| format!("running {what}: {e}"))?;
    let all = format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    if o.status.success() {
        return Ok(all);
    }
    Err(tail_text(what, &all))
}

/// What a failed command says: its last 25 lines, after the tests cargo
/// says failed.
fn tail_text(what: &str, all: &str) -> String {
    let tail: Vec<&str> = all
        .lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(25)
        .collect();
    let names = failed_tests(all);
    let named = if names.is_empty() {
        String::new()
    } else {
        format!(" ({})", names.join(", "))
    };
    format!(
        "{what} failed{named}:\n{}",
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    )
}

/// The tests cargo's output says failed, by their paths, so a FAIL line
/// names them.
pub(crate) fn failed_tests(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| l.strip_prefix("test ")?.strip_suffix(" ... FAILED"))
        .map(str::to_owned)
        .collect()
}

fn path_with_locked(c: &Cx) -> Result<std::ffi::OsString> {
    let locked = c.home.join("bin/locked");
    let path = std::env::var_os("PATH").unwrap_or_default();
    Ok(if locked.is_dir() {
        std::env::join_paths(std::iter::once(locked).chain(std::env::split_paths(&path)))?
    } else {
        path
    })
}

/// The tools of [`Cx::tools`] missing from the PATH the checks run with.
fn missing_tools(c: &Cx) -> Vec<&'static str> {
    let path = path_with_locked(c).unwrap_or_default();
    c.tools
        .iter()
        .copied()
        .filter(|t| {
            !std::env::split_paths(&path).any(|d| {
                gp_mcp::proc::is_executable(&d.join(t))
                    || (cfg!(windows) && d.join(format!("{t}.exe")).is_file())
            })
        })
        .collect()
}

/// `cargo fmt --all --check` in the tree: the first line of `just check`,
/// run before the build too, since it takes a second and needs no build.
/// `just check` still runs it, so this only refuses earlier.
fn fmt(c: &Cx) -> std::result::Result<(), String> {
    let what = "cargo fmt --all --check";
    let o = cargo(c)
        .args(["fmt", "--all", "--check"])
        .current_dir(&c.tree)
        .output()
        .map_err(|e| format!("running {what}: {e}"))?;
    if o.status.success() {
        return Ok(());
    }
    let all = format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    Err(fmt_why(&c.tree, &all, &tail_text(what, &all)))
}

/// A fmt failure whose first line names the files rustfmt would change
/// (the `Diff in FILE at line N:` lines of its whole output `all`, as paths
/// in the tree), then its last lines `tail`, so one dry run says all a
/// worker must format.
fn fmt_why(tree: &Path, all: &str, tail: &str) -> String {
    let real = fs::canonicalize(tree).unwrap_or_else(|_| tree.to_owned());
    let mut files: Vec<String> = Vec::new();
    for l in all.lines() {
        let Some(rest) = l.trim().strip_prefix("Diff in ") else {
            continue;
        };
        // `Diff in FILE at line N:`, or the older `Diff in FILE:N:`.
        let f = match rest.find(" at line ") {
            Some(i) => &rest[..i],
            None => rest
                .trim_end_matches(':')
                .trim_end_matches(char::is_numeric),
        };
        let p = Path::new(f.trim_end_matches(':'));
        let shown = match p.strip_prefix(tree).or_else(|_| p.strip_prefix(&real)) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => p.display().to_string(),
        };
        if !files.contains(&shown) {
            files.push(shown);
        }
    }
    if files.is_empty() {
        return tail.to_owned();
    }
    format!(
        "cargo fmt would change {}: run cargo fmt --all in your worktree and make the patch again\n{tail}",
        files.join(", ")
    )
}

/// Where the gate keeps the trees a dry run passed: `<gate home>/tool/passed`,
/// one file per tree, named by its git tree hash.
fn passed_dir(c: &Cx) -> PathBuf {
    c.target.with_file_name("passed")
}

/// How long a dry run's pass stands for the same tree: past it the gate
/// checks again (a new toolchain or node on the machine, say).
const PASS_KEEPS: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// The dry run that passed `tree` within [`PASS_KEEPS`], as its record.
fn passed_before(c: &Cx, tree: &str) -> Option<Value> {
    let f = passed_dir(c).join(tree);
    let age = fs::metadata(&f).ok()?.modified().ok()?.elapsed().ok()?;
    if age > PASS_KEEPS {
        return None;
    }
    serde_json::from_str(&fs::read_to_string(&f).ok()?).ok()
}

/// A hash's first 12 digits, as the gate's lines show it.
fn short(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

/// Keeps a dry run's pass of `tree`, so the gate after the person's yes
/// lands the same tree without checking it again.
fn keep_pass(c: &Cx, tree: &str, tip: &str, commit: &str) {
    let d = passed_dir(c);
    if fs::create_dir_all(&d).is_err() {
        return;
    }
    let rec = json!({
        "at": crate::open::now_rfc3339(),
        "patch": c.steps.run,
        "tip": tip,
        "commit": commit,
    });
    let tmp = d.join(format!("{tree}.{}.new", std::process::id()));
    if fs::write(&tmp, rec.to_string()).is_ok() {
        let _ = fs::rename(&tmp, d.join(tree));
    }
}

/// Builds gp in the gate's tree, in the gate's profile.
fn build(c: &Cx) -> std::result::Result<(), String> {
    progress("building gp");
    run_step(
        cargo(c)
            .args(["build", "-q", "--profile", PROFILE, "-p", "gp-cli"])
            .current_dir(&c.tree)
            .env("CARGO_TARGET_DIR", &c.target),
        "cargo build",
    )
}

fn built_gp(c: &Cx) -> PathBuf {
    c.target.join(PROFILE).join("gp")
}

/// The page's two players built into the tree's target/play-web, where the
/// published gp (built from the tree) serves them from: `just check` builds
/// them into the gate's own target, which that gp never reads, so without
/// this the screen and the mechanic page can't start.
fn players(c: &Cx, lines: &mut Vec<String>) -> Result<()> {
    let script = c.tree.join("scripts/web/build-play.sh");
    if !script.is_file() {
        return Ok(());
    }
    let out = c.tree.join("target/play-web");
    for arcade in [false, true] {
        let mut cmd = Command::new(&script);
        if arcade {
            cmd.arg("--arcade");
        }
        cmd.arg(&out)
            .current_dir(&c.tree)
            .env("CARGO_TARGET_DIR", &c.target);
        gate_env(&mut cmd);
        let o = cmd.output().context("running build-play.sh")?;
        if !o.status.success() {
            bail!(
                "the published gp's players didn't build (build-play.sh{}): {}",
                if arcade { " --arcade" } else { "" },
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
    }
    lines.push(format!("the page's players built in {}", out.display()));
    Ok(())
}

/// Runs `cmd` in a process group of its own, its stdout and stderr in
/// `<log>.out` and `<log>.err` (files, not pipes: a grandchild that
/// outlives a stopped lane can't hold a pipe open), and stops the whole
/// group when `stop` is set. `Ok((stdout, stderr))` when it exits 0, else
/// `Err` with its last lines.
fn run_killable(
    cmd: &mut Command,
    what: &str,
    stop: &AtomicBool,
    log: &Path,
) -> std::result::Result<(String, String), String> {
    if let Some(d) = log.parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let (out, err) = (log.with_extension("out"), log.with_extension("err"));
    let file = |p: &Path| fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()));
    let mut child = gp_mcp::proc::own_group(cmd)
        .stdin(Stdio::null())
        .stdout(file(&out)?)
        .stderr(file(&err)?)
        .spawn()
        .map_err(|e| format!("running {what}: {e}"))?;
    let pid = child.id();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {}
            Err(e) => return Err(format!("waiting for {what}: {e}")),
        }
        if stop.load(Ordering::SeqCst) || crate::gate_hold::stopping() {
            kill_group(pid, &mut child);
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let read = |p: &Path| String::from_utf8_lossy(&fs::read(p).unwrap_or_default()).into_owned();
    let (stdout, stderr) = (read(&out), read(&err));
    match status {
        None if crate::gate_hold::stopping() => {
            Err(format!("{what} stopped: {}", crate::gate_hold::why()))
        }
        None => Err(format!("{what} stopped: another lane failed")),
        Some(st) if st.success() => Ok((stdout, stderr)),
        Some(_) => Err(tail_text(what, &format!("{stdout}\n{stderr}"))),
    }
}

/// Stops a lane's process group: TERM, then KILL after 3 s. std has no
/// killpg and gp-cli no libc, so through kill(1).
fn kill_group(pid: u32, child: &mut std::process::Child) {
    if cfg!(not(unix)) {
        gp_mcp::proc::stop_group(pid, std::time::Duration::ZERO);
        let _ = child.wait();
        return;
    }
    let signal = |sig: &str| {
        let _ = Command::new("kill")
            .args([sig, "--", &format!("-{pid}")])
            .stderr(Stdio::null())
            .status();
    };
    signal("-TERM");
    let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < end {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // What's left of the group, the leader too.
    signal("-KILL");
    let _ = child.wait();
}

/// A lane: its name, its phase in a FAIL line, and what it runs. It gives
/// the lines it adds to the gate's output, or its failure.
type LaneFn<'a> =
    Box<dyn FnOnce(&AtomicBool) -> std::result::Result<Vec<String>, String> + Send + 'a>;

/// Runs the lanes at once, each on a thread. The first to fail sets the
/// stop flag, which stops the others' processes, and its failure is the
/// gate's. Every lane must pass for `Ok`: the same conjunction as running
/// them one after another, so the order of failures changes only which
/// failure is named when two lanes would fail.
fn run_lanes(
    c: &Cx,
    lanes: Vec<(&'static str, &'static str, LaneFn<'_>)>,
) -> std::result::Result<Vec<String>, Failed> {
    let stop = AtomicBool::new(false);
    let (tx, rx) = std::sync::mpsc::channel();
    let t0 = std::time::Instant::now();
    let n = lanes.len();
    let mut first: Option<Failed> = None;
    let mut lines = Vec::new();
    std::thread::scope(|s| {
        for (name, phase, f) in lanes {
            let (tx, stop) = (tx.clone(), &stop);
            s.spawn(move || {
                // A lane that panics fails as itself: its result still
                // comes, so the loop below never waits for it.
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(stop)))
                    .unwrap_or_else(|p| Err(format!("lane {name} panicked: {}", panic_text(&*p))));
                let _ = tx.send((name, phase, r));
            });
        }
        // Only the lanes hold a sender now: if one ends without sending,
        // recv ends instead of waiting for ever.
        drop(tx);
        for _ in 0..n {
            let Ok((name, phase, r)) = rx.recv() else {
                break;
            };
            c.steps.record(
                &format!("lane {name}"),
                t0.elapsed().as_secs_f64(),
                r.is_ok(),
            );
            match r {
                Ok(l) => lines.extend(l),
                Err(why) if first.is_none() => {
                    if n > 1 {
                        progress(format!("{name} failed: stopping the other lanes"));
                    }
                    stop.store(true, Ordering::SeqCst);
                    first = Some(Failed { why, phase });
                }
                Err(_) => {}
            }
        }
    });
    c.steps
        .record("lanes", t0.elapsed().as_secs_f64(), first.is_none());
    match first {
        Some(f) => Err(f),
        None => Ok(lines),
    }
}

/// What a panic said, from its payload.
fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".into())
}

/// Where a lane's output is kept: `<steps folder>/logs/NAME`.
fn lane_log(c: &Cx, name: &str) -> PathBuf {
    c.steps
        .file
        .with_file_name("logs")
        .join(name.replace(['/', ' '], "_"))
}

/// What the gate says when this machine can't build a module from the
/// kit (`why`, crate::self_check::why_no_module_build): no check here
/// builds one, and gp self check skips its own, so a change that breaks
/// the kit's module build passes here unseen (0.1.3's build script
/// panicked on the kit's manifests, and no worker's shell ever ran that
/// step; docs/release.md). The release
/// job's `gp self check --as-installed --strict` builds one and fails
/// without it.
fn module_build_note(why: Option<String>) -> Option<String> {
    why.map(|why| {
        format!(
            "not checked here: a module built from the kit (gp self check skips it on this machine: {why}); the release job's gp self check --strict builds one"
        )
    })
}

/// The lane `just check`, from the tree, in the gate's profile, told
/// (GP_GATE_RUNS_TESTS=1) to leave its own test run to the tests lane.
/// GP_GATE_RUN names this gate's process: `just check` honours
/// GP_GATE_RUNS_TESTS only while that process lives, so the variable left
/// in someone's shell doesn't skip their tests.
fn check_lane(c: &Cx, stop: &AtomicBool) -> std::result::Result<Vec<String>, String> {
    let path = path_with_locked(c).map_err(|e| e.to_string())?;
    progress("just check");
    let since = std::time::SystemTime::now();
    let r = run_killable(
        gate_env(&mut Command::new("just"))
            .arg("check")
            .current_dir(&c.tree)
            .env("CARGO_TARGET_DIR", &c.target)
            .env("GP_PROFILE", PROFILE)
            .env("GP_GATE_RUNS_TESTS", "1")
            .env("GP_GATE_RUN", std::process::id().to_string())
            .env("PATH", &path),
        "just check",
        stop,
        &lane_log(c, "just-check"),
    );
    match &r {
        Ok((out, err)) => c.steps.check_lines(&format!("{out}\n{err}")),
        Err(why) => c.steps.check_lines(why),
    }
    note_changes(c, "just check", since);
    r.map(|_| Vec::new())
}

/// The tests' own target folder, `<gate home>/tool/target-tests`. A
/// `--workspace` build resolves some shared dependencies with more features
/// than `-p gp-cli` does (syn's extra-traits, fold and visit, through the
/// workspace's dev-dependencies), which changes thiserror's build, so in one
/// target folder each rebuilt tetanes-core and everything above it, gp-cli
/// too, for the other: 40 s or more a gate, and a gp that changed under
/// `just check`. Apart, each stays built.
fn tests_target(c: &Cx) -> PathBuf {
    c.target.with_file_name("target-tests")
}

/// The lane of the tests, on their own: the patch can't remove them by
/// changing the justfile. The workspace's tests in the gate's profile with
/// the slow ones on (GP_SLOW_TESTS=1), built once, then each slow test in a
/// run of its own beside a run of the rest ([`test_runs`]), in their own
/// target folder ([`tests_target`]).
fn tests_lane(c: &Cx, stop: &AtomicBool) -> std::result::Result<Vec<String>, String> {
    progress("cargo test");
    let since = std::time::SystemTime::now();
    let target = tests_target(c);
    let test = |extra: &[String]| {
        let mut cmd = cargo(c);
        cmd.args(["test", "-q", "--workspace", "--profile", PROFILE])
            .args(extra)
            .current_dir(&c.tree)
            .env("CARGO_TARGET_DIR", &target)
            .env_remove("GP_SLOW_TESTS");
        cmd
    };
    c.steps.time(
        "test compile",
        || {
            run_killable(
                &mut test(&["--no-run".into()]),
                "cargo test",
                stop,
                &lane_log(c, "test-compile"),
            )
        },
        Result::is_ok,
    )?;
    // The slow tests (GP_SLOW_TESTS) are CI's: the gate runs the rest, so a
    // change lands in minutes.
    let runs = test_runs(&[]);
    progress(format!(
        "cargo test: the {} slow tests are left to CI",
        slow_tests(&c.tree).len()
    ));
    let test = &test;
    let fails: Vec<String> = std::thread::scope(|s| {
        let runs: Vec<_> = runs
            .iter()
            .map(|(label, args)| {
                s.spawn(move || {
                    let args: Vec<String> = std::iter::once("--".to_owned())
                        .chain(args.iter().cloned())
                        .collect();
                    c.steps.time(
                        &format!("tests/{label}"),
                        || {
                            run_killable(
                                &mut test(&args),
                                &format!("cargo test ({label})"),
                                stop,
                                &lane_log(c, &format!("test-{label}")),
                            )
                        },
                        Result::is_ok,
                    )
                })
            })
            .collect();
        runs.into_iter()
            .filter_map(|h| match h.join() {
                Ok(r) => r.err(),
                Err(p) => Some(format!("a test run panicked: {}", panic_text(&*p))),
            })
            .collect()
    });
    note_changes(c, "the tests", since);
    if fails.is_empty() {
        Ok(Vec::new())
    } else {
        Err(fails.join("\n"))
    }
}

/// Prints, as a `step note`, the files of the tree a step changed: what
/// `git diff` sees against the index, and the files under the parts cargo
/// watches (crates/, third_party/, .cargo/, Cargo.toml, Cargo.lock,
/// rust-toolchain.toml) written since `since`. A file a check writes there
/// makes cargo build gp again. Only a note: it refuses nothing.
fn note_changes(c: &Cx, step: &str, since: std::time::SystemTime) {
    let files = changed_since(&c.tree, since);
    if files.is_empty() {
        return;
    }
    let shown: Vec<&str> = files.iter().take(20).map(String::as_str).collect();
    c.steps.note(&format!(
        "files changed during {step} ({}): {}",
        files.len(),
        shown.join(", ")
    ));
}

fn changed_since(tree: &Path, since: std::time::SystemTime) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    if let Ok(d) = git(tree, &["diff", "--name-only"]) {
        out.extend(d.lines().map(|l| format!("{l} (differs from the index)")));
    }
    let newer = |p: &Path| {
        fs::symlink_metadata(p)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t > since)
    };
    let mut dirs = Vec::new();
    for top in ["crates", "third_party", ".cargo"] {
        dirs.push(tree.join(top));
    }
    for f in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"] {
        if newer(&tree.join(f)) {
            out.insert(f.to_owned());
        }
    }
    while let Some(d) = dirs.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                let n = e.file_name();
                if n != "target" && n != "node_modules" && n != ".git" {
                    dirs.push(p);
                }
            } else if newer(&p)
                && let Ok(rel) = p.strip_prefix(tree)
            {
                out.insert(rel.to_string_lossy().into_owned());
            }
        }
    }
    out.into_iter().collect()
}

/// The tests that start with the slow-test switch
/// (`if gp_arcade::skip_slow_test()`), by function name, from the tree's
/// crates: the names `just test` finds the same way.
fn slow_tests(tree: &Path) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    let mut dirs = vec![tree.join("crates")];
    while let Some(d) = dirs.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs")
                && let Ok(text) = fs::read_to_string(&p)
            {
                names.extend(slow_in(&text));
            }
        }
    }
    names.into_iter().collect()
}

/// The functions in `text` that have a line starting with
/// `if ...skip_slow_test()`: each is the nearest `fn NAME(` above it.
fn slow_in(text: &str) -> Vec<String> {
    let mut f: Option<&str> = None;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(i) = line.find("fn ") {
            let rest = &line[i + 3..];
            if let Some(j) = rest.find('(') {
                let n = &rest[..j];
                if !n.is_empty()
                    && n.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                {
                    f = Some(n);
                }
            }
        }
        let t = line.trim_start();
        if t.starts_with("if ")
            && t.contains("skip_slow_test()")
            && let Some(n) = f
        {
            out.push(n.to_owned());
        }
    }
    out
}

/// The gate's test runs, each `(label, what follows --)`, all with the slow
/// tests on: one per slow test, filtered to its name, and one of the rest,
/// skipping those names. A filter and a `--skip` match names the same way
/// (a name containing the text), so together the runs are every test, and
/// a slow test `slow_tests` misses runs with the rest.
fn test_runs(slow: &[String]) -> Vec<(String, Vec<String>)> {
    let rest = slow
        .iter()
        .flat_map(|n| ["--skip".to_owned(), n.clone()])
        .collect();
    std::iter::once(("rest".to_owned(), rest))
        .chain(slow.iter().map(|n| (n.clone(), vec![n.clone()])))
        .collect()
}

/// The tree's patch, or why it was refused: the commit a passing try made.
/// After the build, `just check` and the tests run at once as lanes
/// ([`run_lanes`]); the first failure stops the other.
/// One patch of a tool gate's batch: its own, or a waiting tool gate's it
/// took in (`gate_queue`, with the ticket's name).
struct Item {
    patch: Patch,
    message: String,
    shown: String,
    ticket: Option<String>,
}

/// Applies `item` on the tree and commits it: the patch's own checks
/// (it applies, changes something, not the hook) and the hook.
fn apply_one(
    c: &Cx,
    item: &Item,
    lines: &mut Vec<String>,
    merged_cli: &mut bool,
) -> Result<std::result::Result<(), Failed>> {
    let tree = &c.tree;
    let fail = |why: String, phase| Ok(Err(Failed { why, phase }));
    let merged = match c.steps.time(
        "apply",
        || gate::apply_how(tree, &item.patch.diff, true),
        Result::is_ok,
    ) {
        Ok(how) => how,
        Err(why) => return fail(why, "patch"),
    };
    if let Some(how) = &merged {
        *merged_cli |= how.contains("CLI.md written again");
        lines.push(how.clone());
    }
    let files = git(tree, &["diff", "--cached", "--name-only", "HEAD"])?;
    if files.is_empty() {
        return fail("the patch changes nothing".into(), "patch");
    }
    if let Some(f) = files
        .lines()
        .find(|f| f.starts_with(".githooks/") || f.starts_with("scripts/romguard/"))
    {
        return fail(
            format!("the patch changes {f}, the ROM-blocking hook, which only the person edits"),
            "patch",
        );
    }
    let gate_gp = std::env::current_exe()?;
    if let Err(why) = c
        .steps
        .time("hook", || gate::hook(tree, &gate_gp), Result::is_ok)
    {
        return fail(format!("the ROM-blocking hook refuses it: {why}"), "hook");
    }
    if let Err(e) = c.steps.time(
        "commit",
        || gate::commit(tree, &item.message, &item.patch, &gate_gp),
        Result::is_ok,
    ) {
        return fail(format!("{e:#}"), "commit");
    }
    Ok(Ok(()))
}

/// One try of a batch on `tip`: each patch is applied and committed in
/// turn, then the batch is built and checked once. In a batch of more than
/// one, a patch that doesn't apply (or the hook refuses) is left out into
/// `dropped` and the rest go on; alone, it fails the try.
///
/// A gate that lands checks a tree a dry run passed ([`passed_before`]) no
/// further: it builds it, for publishing, and lands it. The tree is the
/// tip with the patches applied, so the same patch on a tip that moved is
/// another tree, checked again. A dry run always checks, and keeps its pass
/// ([`keep_pass`]).
fn trial(
    c: &Cx,
    items: &[Item],
    tip: &str,
    dry_run: bool,
    lines: &mut Vec<String>,
    dropped: &mut Vec<(usize, Failed)>,
) -> Result<std::result::Result<Tried, Failed>> {
    macro_rules! refuse {
        ($why:expr, $phase:expr) => {
            return Ok(Err(Failed {
                why: $why,
                phase: $phase,
            }))
        };
    }
    let tree = &c.tree;
    let mut merged_cli = false;
    for (k, item) in items.iter().enumerate() {
        if let Err(f) = apply_one(c, item, lines, &mut merged_cli)? {
            if items.len() == 1 {
                return Ok(Err(f));
            }
            lines.push(format!(
                "{} is left out of the batch: {}",
                item.shown, f.why
            ));
            gate::clean(tree);
            dropped.push((k, f));
        }
    }
    if dropped.len() == items.len() {
        refuse!("no patch of the batch applies".into(), "patch");
    }
    let files = git(tree, &["diff", "--name-only", tip, "HEAD"])?;
    let applied = git(tree, &["rev-parse", "HEAD^{tree}"])?;
    let tried = |commit: String, words_only: bool| Tried {
        commit,
        words_only,
        tree: applied.clone(),
    };
    let gate_gp = std::env::current_exe()?;
    // Words alone (Markdown, the site's pages): the prose lint, then land.
    // Nothing is built, so nothing is tested twice and nothing is published.
    if docs_only(&files) {
        lines.push("words only: the prose lint, no build and no checks".into());
        let mut lint = Command::new("just");
        lint.current_dir(tree)
            .args(["--no-deps", "site-prose"])
            .env("PATH", path_with_locked(c)?);
        if let Err(why) = c.steps.time(
            "prose",
            || run_step(gate_env(&mut lint), "just site-prose"),
            Result::is_ok,
        ) {
            refuse!(why, "prose");
        }
        return Ok(Ok(tried(git(tree, &["rev-parse", "HEAD"])?, true)));
    }
    let before = if dry_run {
        None
    } else {
        passed_before(c, &applied)
    };
    if let Some(rec) = &before {
        let msg = format!(
            "a dry run passed this same tree ({}) at {} (patch {}, on {}): built for publishing, not checked again",
            short(&applied),
            rec["at"].as_str().unwrap_or("?"),
            rec["patch"].as_str().unwrap_or("?"),
            short(rec["tip"].as_str().unwrap_or("?")),
        );
        c.steps.note(&msg);
        lines.push(msg);
    } else if let Err(why) = c.steps.time("fmt", || fmt(c), Result::is_ok) {
        refuse!(why, "fmt");
    }
    if let Err(why) = c.steps.time("build", || build(c), Result::is_ok) {
        refuse!(why, "build");
    }
    // A merge that took the tip's CLI.md: written again by the merged gp,
    // into the last commit.
    if merged_cli {
        match gate::regenerate(tree, &built_gp(c)) {
            Ok(true) => {
                let mut amend = Command::new("git");
                amend
                    .arg("-C")
                    .arg(tree)
                    .args(["commit", "-q", "--amend", "--no-edit"])
                    .env("GP", &gate_gp);
                if let Err(why) = run_step(&mut amend, "git commit --amend") {
                    refuse!(why, "commit");
                }
                lines.push("CLI.md written again by the merged gp".into());
            }
            Ok(false) => {}
            Err(e) => refuse!(format!("{e:#}"), "build"),
        }
    }
    if before.is_none() {
        let lanes: Vec<(&'static str, &'static str, LaneFn<'_>)> = vec![
            ("check", "just check", Box::new(|stop| check_lane(c, stop))),
            ("tests", "tests", Box::new(|stop| tests_lane(c, stop))),
        ];
        match run_lanes(c, lanes) {
            Ok(l) => lines.extend(l),
            Err(f) => return Ok(Err(f)),
        }
    }
    if let Some(msg) = module_build_note(crate::self_check::why_no_module_build()) {
        c.steps.note(&msg);
        lines.push(msg);
    }
    // The editor the landing publishes beside the build (crate::publish):
    // `just check` doesn't build it, and this tree's own is kept between
    // gates, so it is built here when editor/ changed since.
    match c.steps.time("editor", || editor(c), Result::is_ok) {
        Ok(Some(l)) => lines.push(l),
        Ok(None) => {}
        Err(why) => refuse!(why, "editor"),
    }
    Ok(Ok(tried(git(tree, &["rev-parse", "HEAD"])?, false)))
}

/// The gate's tree's editor/dist built from its editor/ at HEAD
/// ([`publish::ensure_editor`]): npm ci and the vite build, as the
/// justfile's `editor` recipe, only when editor/'s sources or lock differ
/// from the ones the kept dist was built from.
fn editor(c: &Cx) -> std::result::Result<Option<String>, String> {
    if !c.tree.join("editor/package.json").is_file() {
        return Ok(None);
    }
    let source = git(&c.tree, &["rev-parse", "HEAD:editor"]).map_err(|e| format!("{e:#}"))?;
    let path = path_with_locked(c).map_err(|e| format!("{e:#}"))?;
    publish::ensure_editor(&c.tree, &source, |dir| {
        progress("building the editor");
        let npm = |args: &[&str]| {
            let mut cmd = Command::new("npm");
            cmd.args(args).current_dir(dir).env("PATH", &path);
            gate_env(&mut cmd);
            cmd
        };
        run_step(
            &mut npm(&["ci", "--silent", "--no-audit", "--no-fund"]),
            "npm ci in editor/",
        )?;
        run_step(
            &mut npm(&["run", "--silent", "build"]),
            "npm run build in editor/",
        )
    })
}

/// A try that passed: the commit to land, whether it was words alone, and
/// the tree the patches made on the tip (before any CLI.md written again).
struct Tried {
    commit: String,
    words_only: bool,
    tree: String,
}

/// A patch of words alone: every file it changes is Markdown or a page of
/// the docs site. CLI.md is generated from gp's commands, so a change to it
/// is not words alone.
fn docs_only(files: &str) -> bool {
    files.lines().all(|f| {
        f != "CLI.md"
            && (f.ends_with(".md")
                || f.starts_with("site/src/content/")
                || f.starts_with("site/public/"))
    })
}

pub(crate) fn run(a: &ToolGateArgs, paths: &Paths) -> Result<Outcome> {
    let cwd = std::env::current_dir()?;
    let repo = PathBuf::from(git(&cwd, &["rev-parse", "--show-toplevel"])?);
    let onto = gate::onto_branch(a.onto.as_deref(), &repo, false)?;
    let home = gate::gate_home(a.home.as_deref())?;
    // The tool gate's own lock and queue: a model gate lands in the games
    // checkout, holds gate.lock and runs beside this one (gate_hold::Lane).
    let lane = crate::gate_hold::Lane::Tool;
    let lock = crate::gate_hold::open_lock(&home, lane)?;
    // A gate that lands takes a ticket first, so the tool gate holding the
    // lock can take its patch into its batch (gate_queue), as model gates do.
    let ticket = if a.dry_run {
        None
    } else {
        let common = git(
            &repo,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let req = crate::gate_queue::Request {
            patch: PathBuf::new(),
            shown: a.patch.display().to_string(),
            message: a.message.clone(),
            repo: PathBuf::from(common),
            game: String::new(),
            onto: onto.clone(),
            run: TOOL_RUN.into(),
            removes: Vec::new(),
            replaces: Vec::new(),
        };
        Some(crate::gate_queue::Ticket::enter(
            &lane.queue_home(&home),
            req,
            &a.patch,
        )?)
    };
    let waited = lock.try_lock().is_err();
    if waited {
        lock.lock()?;
    }
    if let Some(o) = ticket.as_ref().and_then(|t| t.result()) {
        return Ok(o);
    }
    // A tool gate of an older gp holds gate.lock and works in tool/ too.
    let _older = crate::gate_hold::wait_for_an_older_tool_gate(&home);
    let tool = home.join("tool");
    // Who holds the lock, and the watch that gives it up when this run's
    // caller is gone (a dry run) or it logs no step for a while (gate_hold).
    let _hold = crate::gate_hold::Hold::take(
        &home,
        lane,
        if a.dry_run {
            "tool gate dry run"
        } else {
            "tool gate"
        },
        Some(&tool.join("steps.jsonl")),
        a.dry_run,
    );
    fs::create_dir_all(&tool)?;
    let patch_bytes = fs::read(&a.patch).with_context(|| a.patch.display().to_string())?;
    let patch_hash = blake3::hash(&patch_bytes).to_hex()[..12].to_owned();
    let patch = match gate::read_patch(&a.patch, &tool)? {
        Ok(p) => p,
        Err(why) => {
            return Ok(fail(
                Failed {
                    why,
                    phase: "patch",
                },
                Vec::new(),
            ));
        }
    };
    let c = Cx {
        repo,
        onto,
        tree: tool.join(TREE),
        target: tool.join("target"),
        home: paths.home.clone(),
        steps: Steps::new(tool.join("steps.jsonl"), &patch_hash),
        tools: TOOLS,
    };
    // The yeses the runner starts together reach the lock within a moment of
    // each other: a gate that found it free waits that moment before it
    // takes the waiting ones in.
    if ticket.is_some() && !waited {
        std::thread::sleep(BATCH_SETTLE);
    }
    let mut items = vec![Item {
        patch,
        message: own_message(a),
        shown: a.patch.display().to_string(),
        ticket: None,
    }];
    if let Some(t) = &ticket {
        for (k, (name, r)) in t.claim().into_iter().enumerate() {
            match gate::read_patch(&r.patch, &tool.join(BATCH).join((k + 1).to_string()))? {
                Ok(p) => items.push(Item {
                    patch: p,
                    message: r
                        .message
                        .unwrap_or_else(|| "A patch, by gp tool gate".into()),
                    shown: r.shown,
                    ticket: Some(name),
                }),
                Err(why) => t.answer(
                    &name,
                    &fail(
                        Failed {
                            why,
                            phase: "patch",
                        },
                        Vec::new(),
                    ),
                ),
            }
        }
    }
    let o = batch(&c, a, ticket.as_ref(), items);
    c.steps.record(
        "gate",
        c.steps.t0.elapsed().as_secs_f64(),
        o.as_ref().is_ok_and(|o| o.exit == 0),
    );
    o
}

/// The tool gate's tree, under the gate home's `tool/`: one per lock. A
/// tool gate of a gp from before the two locks holds `gate.lock`, not
/// `tool.lock`, and works in `tool/tree`, so it could start while this
/// gate runs and share its tree, each git stopping on the other's
/// index.lock; this gate's tree is its own (issue 45).
pub(crate) const TREE: &str = "tree-tool-lock";
/// Where a batch's patches are read to, apart from an older gp's
/// `tool/batch` for the same reason.
const BATCH: &str = "batch-tool-lock";

/// The run name of a tool gate's ticket: tool gates batch only together.
const TOOL_RUN: &str = "tool gate";
/// How long a gate that found the lock free waits for the gates started
/// with it to take their tickets.
const BATCH_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);

/// Gates `items` (this gate's own first) as one batch. When it lands, each
/// patch it took in is answered with the landing, and one it left out is
/// given back to gate itself; when it fails, each is given back and this
/// gate's own patch is gated alone, so one bad patch never costs the rest.
fn batch(
    c: &Cx,
    a: &ToolGateArgs,
    ticket: Option<&crate::gate_queue::Ticket>,
    items: Vec<Item>,
) -> Result<Outcome> {
    let alone = |items: &[Item]| tries_of(c, a.dry_run, &items[..1]).map(|(o, _)| o);
    let Some(t) = ticket.filter(|_| items.len() > 1) else {
        return alone(&items);
    };
    let (o, dropped) = match tries_of(c, a.dry_run, &items) {
        Ok(r) => r,
        Err(e) => {
            for i in &items[1..] {
                t.release(i.ticket.as_deref().unwrap_or(""));
            }
            return Err(e);
        }
    };
    let left_out = |k: usize| dropped.iter().any(|(d, _)| *d == k);
    for (k, i) in items.iter().enumerate().skip(1) {
        let name = i.ticket.as_deref().unwrap_or("");
        if o.exit == 0 && !left_out(k) {
            t.answer(name, &o);
        } else {
            t.release(name);
        }
    }
    if o.exit == 0 && !left_out(0) {
        return Ok(o);
    }
    let mut again = alone(&items)?;
    let why = if o.exit == 0 {
        "it was left out of the batch that landed".to_owned()
    } else {
        format!(
            "the batch of {} failed ({}), so each patch is gated alone",
            items.len(),
            o.json["why"].as_str().unwrap_or("no reason given")
        )
    };
    if let Some(l) = again.json["lines"].as_array_mut() {
        l.insert(0, json!(why));
    }
    again.text = format!("{why}\n{}", again.text);
    Ok(again)
}

/// The gate's tries of its one patch.
#[cfg(test)]
fn tries(c: &Cx, a: &ToolGateArgs, patch: &Patch) -> Result<Outcome> {
    let item = Item {
        patch: Patch {
            diff: patch.diff.clone(),
            message: patch.message.clone(),
            author: patch.author.clone(),
            date: patch.date.clone(),
        },
        message: own_message(a),
        shown: a.patch.display().to_string(),
        ticket: None,
    };
    Ok(tries_of(c, a.dry_run, std::slice::from_ref(&item))?.0)
}

fn own_message(a: &ToolGateArgs) -> String {
    a.message
        .clone()
        .unwrap_or_else(|| "A patch, by gp tool gate".into())
}

/// The gate's tries of a batch on the branch tip, each from a fresh tree,
/// until one lands, fails, or the tip has moved [`TIP_TRIES`] times: the
/// outcome, and the patches the landing try left out (by their place in
/// `items`).
fn tries_of(c: &Cx, dry_run: bool, items: &[Item]) -> Result<(Outcome, Vec<(usize, Failed)>)> {
    let mut dropped = Vec::new();
    let o = tries_in(c, dry_run, items, &mut dropped)?;
    Ok((o, dropped))
}

fn tries_in(
    c: &Cx,
    dry_run: bool,
    items: &[Item],
    dropped: &mut Vec<(usize, Failed)>,
) -> Result<Outcome> {
    let mut lines = Vec::new();
    if items.len() > 1 {
        lines.push(format!(
            "a batch of {}: {}",
            items.len(),
            items
                .iter()
                .map(|i| i.shown.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut failed = None;
    let missing = missing_tools(c);
    if !missing.is_empty() {
        let f = Failed {
            why: format!(
                "just check needs {} on PATH, and this machine has none",
                missing.join(", ")
            ),
            phase: "preflight",
        };
        return Ok(fail(f, lines));
    }
    // Before the tree is touched: a lock in it, or a checkout to land in
    // with edits to the same files, keeps the person's yes.
    let files: Vec<String> = items
        .iter()
        .flat_map(|i| gate::patch_files(&c.repo, &i.patch.diff))
        .collect();
    if let Err(n) = gate::can_land(&c.repo, &c.tree, &c.onto, &files, !dry_run) {
        let f = Failed {
            why: n.why,
            phase: n.phase,
        };
        return Ok(fail(f, lines));
    }
    for _ in 0..TIP_TRIES {
        let tip = git(&c.repo, &["rev-parse", &c.onto])?;
        let out = c.steps.time(
            "checkout",
            || gate::checkout(&c.repo, &c.tree, &tip),
            Result::is_ok,
        );
        if let Err(e) = out {
            let f = Failed {
                why: format!(
                    "the gate's tree {} couldn't be checked out: {e:#}",
                    c.tree.display()
                ),
                phase: "checkout",
            };
            return Ok(fail(f, lines));
        }
        dropped.clear();
        let tried = c.steps.time(
            "try",
            || trial(c, items, &tip, dry_run, &mut lines, dropped),
            |r| matches!(r, Ok(Ok(_))),
        )?;
        match tried {
            Err(f) => {
                failed = Some(f);
                gate::clean(&c.tree);
                break;
            }
            Ok(Tried {
                commit,
                words_only,
                tree,
            }) => {
                if dry_run {
                    if !words_only {
                        keep_pass(c, &tree, &tip, &commit);
                        lines.push(format!(
                            "this pass is kept: the gate after the yes lands this same tree without checking it again, while {} stays at {}",
                            c.onto,
                            short(&tip)
                        ));
                    }
                    lines.push("--dry-run: PASS, nothing lands".into());
                    return Ok(Outcome::ok(
                        json!({"verdict": "PASS", "commit": commit, "tip": tip, "lines": lines}),
                        lines.join("\n"),
                    ));
                }
                let landed = c.steps.time(
                    "land",
                    || gate::land(&c.repo, &c.onto, &tip, &commit),
                    Result::is_ok,
                );
                match landed {
                    Ok(()) => {
                        lines.push(format!("PASS: landed {commit} on {}", c.onto));
                        // A landing that names an objection to one of the
                        // loop's steps clears it (crate::wrong).
                        lines.extend(crate::wrong::clear_landed(&c.home, &c.repo, &commit));
                        if words_only {
                            lines.push(
                                "words only: nothing built, so gp isn't published again".into(),
                            );
                            return Ok(Outcome::ok(
                                json!({"verdict": "PASS", "commit": commit, "tip": tip, "lines": lines}),
                                lines.join("\n"),
                            ));
                        }
                        c.steps.time(
                            "publish",
                            || {
                                publish::publish(
                                    &c.home,
                                    &built_gp(c),
                                    &commit,
                                    &c.repo,
                                    &c.tree,
                                    &mut lines,
                                )
                            },
                            Result::is_ok,
                        )?;
                        c.steps
                            .time("players", || players(c, &mut lines), Result::is_ok)?;
                        return Ok(Outcome::ok(
                            json!({"verdict": "PASS", "commit": commit, "tip": tip, "lines": lines}),
                            lines.join("\n"),
                        ));
                    }
                    Err((why, moved)) if moved => {
                        lines.push(format!("the tip moved: {why}; again"));
                    }
                    Err((why, _)) => {
                        let f = Failed {
                            why: format!("couldn't land {commit}: {why}"),
                            phase: "land",
                        };
                        return Ok(fail(f, lines));
                    }
                }
            }
        }
    }
    let f = failed.unwrap_or(Failed {
        why: format!("the tip moved {TIP_TRIES} times"),
        phase: "tip",
    });
    Ok(fail(f, lines))
}

/// The gate's FAIL. One whose cause is outside the patch (a phase of
/// [`gate::NOT_THE_PATCH`]) says so in `not_the_patch`, and the runner
/// keeps the yes and runs it again.
fn fail(f: Failed, mut lines: Vec<String>) -> Outcome {
    let outside = gate::NOT_THE_PATCH.contains(&f.phase);
    lines.push(format!("FAIL ({}): {}", f.phase, f.why));
    Outcome {
        json: json!({"verdict": "FAIL", "phase": f.phase, "why": f.why,
            "not_the_patch": outside, "lines": lines}),
        text: lines.join("\n"),
        exit: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn set_time(p: &Path, secs: u64) {
        fs::File::open(p)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    fn cx(root: &Path) -> Cx {
        Cx {
            repo: root.join("repo"),
            onto: "main".into(),
            tree: root.join("tree"),
            target: root.join("target"),
            home: root.join("home"),
            steps: Steps::new(root.join("steps.jsonl"), "test"),
            tools: &["just", "git"],
        }
    }

    /// A FAIL line names the tests cargo says failed.
    #[test]
    fn a_patch_of_words_alone_lands_after_the_prose_lint_with_no_build() {
        let d = tempfile::tempdir().unwrap();
        let c = gate_rig(d.path());
        let patch = patch_of(&c, "README.md", "the tool, said in other words\n");
        let out = gate(&c, &patch, false);
        assert!(out.text.contains("words only"), "{}", out.text);
        assert!(out.text.contains("PASS: landed"), "{}", out.text);
        assert!(!out.text.contains("just check"), "{}", out.text);
    }

    #[test]
    fn words_only_is_markdown_and_the_sites_pages_never_cli_md_or_code() {
        assert!(docs_only(
            "README.md\nsite/src/content/docs/themes.md\ndocs/gate.md"
        ));
        assert!(docs_only("site/public/logo.svg"));
        assert!(!docs_only("CLI.md"));
        assert!(!docs_only("docs/gate.md\ncrates/gp-cli/src/main.rs"));
        assert!(!docs_only("site/src/components/Landing.astro"));
        assert!(!docs_only("justfile"));
    }

    #[test]
    fn the_failed_tests_are_read_from_cargos_output() {
        let out = "running 3 tests\ntest a::b ... ok\ntest serve::agent::tests::slow ... FAILED\ntest c ... FAILED\n\nfailures:\n";
        assert_eq!(failed_tests(out), ["serve::agent::tests::slow", "c"]);
        let why = run_step(
            Command::new("sh").args(["-c", "printf 'test x::y ... FAILED\\n'; exit 101"]),
            "cargo test",
        )
        .unwrap_err();
        assert!(why.starts_with("cargo test failed (x::y):"), "{why}");
    }

    fn git_in(dir: &Path, args: &[&str]) {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {o:?}");
    }

    /// A lane that panics (as `eprintln!` does on an orphaned dry run's
    /// closed stderr) fails the gate in its own name; before, run_lanes
    /// waited for its result for ever, holding tool.lock.
    #[test]
    fn a_lane_that_panics_fails_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        let c = cx(dir.path());
        type R = std::result::Result<Vec<String>, String>;
        let panics: LaneFn<'_> =
            Box::new(|_: &AtomicBool| -> R { panic!("failed printing to stderr") });
        let passes: LaneFn<'_> =
            Box::new(|_: &AtomicBool| -> R { Ok(vec!["the tests passed".into()]) });
        let lanes = vec![("check", "check", panics), ("tests", "tests", passes)];
        let f = run_lanes(&c, lanes).unwrap_err();
        assert_eq!(f.phase, "check");
        assert!(
            f.why
                .contains("lane check panicked: failed printing to stderr"),
            "{}",
            f.why
        );
    }

    /// On a machine that can't build a
    /// module from the kit, the gate says it didn't check one, and why;
    /// on one that can, it says nothing.
    #[test]
    fn a_gate_that_cant_build_a_module_says_so() {
        let why = "no toolchain (rustup not on PATH)".to_owned();
        let note = module_build_note(Some(why)).unwrap();
        assert!(
            note.starts_with("not checked here: a module built from the kit"),
            "{note}"
        );
        assert!(note.contains("no toolchain (rustup not on PATH)"), "{note}");
        assert!(note.contains("gp self check --strict"), "{note}");
        assert_eq!(module_build_note(None), None);
    }

    fn script(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
        gp_mcp::proc::make_executable(path).unwrap();
    }

    /// A tool repo and a gate around it, with stand-ins for cargo and just,
    /// and no ROM folder and no games checkout anywhere: the gate needs
    /// neither. The cargo stand-in's `build` writes a gp that prints the
    /// tree it was built in; its `test` adds a line of its arguments and
    /// GP_SLOW_TESTS to `<target>/cargo-test-seen` and fails when the
    /// tree's tool.txt says `a failing test`, as a failing test would. The
    /// repo has one slow test, `a_slow_one`. The just stand-in leaves
    /// GP_GATE_RUNS_TESTS and GP_GATE_RUN in `<target>/just-seen`, and the
    /// gp it was handed (`target/fastdev/gp`, the patched build) in
    /// `<times>/just-gp`.
    fn gate_rig(root: &Path) -> Cx {
        let c = cx(root);
        fs::create_dir_all(&c.repo).unwrap();
        git_in(&c.repo, &["init", "-q", "-b", "main"]);
        git_in(&c.repo, &["config", "user.name", "gate test"]);
        git_in(&c.repo, &["config", "user.email", "gate@test"]);
        fs::write(c.repo.join("README.md"), "the tool\n").unwrap();
        fs::write(c.repo.join("tool.txt"), "the tool\n").unwrap();
        fs::create_dir_all(c.repo.join("crates/gp-x/tests")).unwrap();
        fs::write(
            c.repo.join("crates/gp-x/tests/slow.rs"),
            "#[test]\nfn a_slow_one() {\n    if gp_arcade::skip_slow_test() {\n        return;\n    }\n}\n",
        )
        .unwrap();
        script(&c.repo.join(".githooks/pre-commit"), "#!/bin/sh\nexit 0\n");
        git_in(&c.repo, &["add", "-A"]);
        git_in(&c.repo, &["commit", "-q", "-m", "the tip"]);

        let (ctl, times) = (root.join("ctl"), root.join("times"));
        fs::create_dir_all(&ctl).unwrap();
        fs::create_dir_all(&times).unwrap();
        // `ctl/meet`: each lane, once started, waits until both have
        // started (just check, a cargo test run), so the lanes end only if
        // they ran at once. One that waits 30 s without the other leaves
        // NAME.alone and goes on.
        let meet = |me: &str| {
            format!(
                "if [ -f '{ctl}/meet' ]; then n=0; while [ ! -f '{times}/just.start' ] || [ ! -f '{times}/test.start' ]; do n=$((n+1)); if [ $n -gt 300 ]; then : > '{times}/{me}.alone'; break; fi; sleep 0.1; done; fi",
                ctl = ctl.display(),
                times = times.display()
            )
        };
        let locked = c.home.join("bin/locked");
        script(
            &locked.join("cargo"),
            &format!(
                r#"#!/bin/sh
if [ "$1" = test ]; then
  mkdir -p "$CARGO_TARGET_DIR"
  echo "$* | GP_SLOW_TESTS=${{GP_SLOW_TESTS:-}}" >> "$CARGO_TARGET_DIR/cargo-test-seen"
  case " $* " in *" --no-run "*) exit 0 ;; esac
  echo $$ > '{times}/test.pid'
  date +%s > '{times}/test.start'
  {meet_test}
  [ -f '{ctl}/test-sleep' ] && sleep "$(cat '{ctl}/test-sleep')"
  if grep -q 'a failing test' tool.txt; then echo 'test fails ... FAILED'; exit 101; fi
  date +%s > '{times}/test.end'
  exit 0
fi
if [ "$1" = fmt ] && grep -q 'bad format' tool.txt; then echo 'Diff in src/lib.rs at line 1:'; exit 1; fi
[ "$1" = build ] || exit 0
mkdir -p "$CARGO_TARGET_DIR/fastdev"
out="$CARGO_TARGET_DIR/fastdev/gp"
printf '#!/bin/sh\necho built in %s\n' "$(pwd -P)" > "$out.new"
chmod +x "$out.new"
mv "$out.new" "$out"
"#,
                ctl = ctl.display(),
                times = times.display(),
                meet_test = meet("test")
            ),
        );
        // just check: `ctl/just-fails` fails it after 0.2 s, `ctl/just-sleep`
        // holds it.
        script(
            &locked.join("just"),
            &format!(
                r#"#!/bin/sh
printf 'GP_GATE_RUNS_TESTS=%s\nGP_GATE_RUN=%s\n' "${{GP_GATE_RUNS_TESTS:-}}" "${{GP_GATE_RUN:-}}" > "$CARGO_TARGET_DIR/just-seen"
"$CARGO_TARGET_DIR/fastdev/gp" > '{times}/just-gp'
date +%s > '{times}/just.start'
{meet_just}
if [ -f '{ctl}/just-fails' ]; then sleep 0.2; echo 'check-step: clippy 0.2 s (failed)'; exit 1; fi
[ -f '{ctl}/just-sleep' ] && sleep "$(cat '{ctl}/just-sleep')"
echo 'check-step: fmt 0.4 s'
echo 'check-step: baseline-mosshop 1.5 s'
date +%s > '{times}/just.end'
"#,
                ctl = ctl.display(),
                times = times.display(),
                meet_just = meet("just")
            ),
        );
        c
    }

    /// Turns on one of the rig's switches (see [`gate_rig`]).
    fn switch(c: &Cx, name: &str, value: &str) {
        fs::write(c.repo.parent().unwrap().join("ctl").join(name), value).unwrap();
    }

    fn stamp(c: &Cx, name: &str) -> Option<String> {
        fs::read_to_string(c.repo.parent().unwrap().join("times").join(name)).ok()
    }

    /// A patch to `file` in the rig's repo, written as `git diff` writes it.
    fn patch_of(c: &Cx, file: &str, text: &str) -> Patch {
        fs::write(c.repo.join(file), text).unwrap();
        let o = Command::new("git")
            .arg("-C")
            .arg(&c.repo)
            .arg("diff")
            .output()
            .unwrap();
        git_in(&c.repo, &["checkout", "--", "."]);
        let f = c.repo.parent().unwrap().join("fix.patch");
        fs::write(&f, &o.stdout).unwrap();
        Patch {
            diff: f,
            message: Some("a patch".into()),
            author: None,
            date: None,
        }
    }

    fn gate(c: &Cx, patch: &Patch, dry_run: bool) -> Outcome {
        let a = ToolGateArgs {
            patch: patch.diff.clone(),
            message: None,
            onto: Some("main".into()),
            home: None,
            dry_run,
        };
        tries(c, &a, patch).unwrap()
    }

    fn dry_run(c: &Cx, patch: &Patch) -> Outcome {
        gate(c, patch, true)
    }

    /// Before the gate touches its tree it checks it can land: git's
    /// index.lock in the tree, or uncommitted edits to the patch's files in
    /// the checkout it lands in, is a FAIL that says it isn't the patch
    /// (`not_the_patch`, so the runner keeps the yes); edits to other files
    /// don't stop it, a dry run doesn't land so edits don't stop it either,
    /// and a failed check is the patch's (issue 45).
    #[test]
    fn a_gate_that_cant_land_says_it_isnt_the_patch() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!(o.json["not_the_patch"], Value::Null);

        let lock = PathBuf::from(
            git(
                &c.tree,
                &[
                    "rev-parse",
                    "--path-format=absolute",
                    "--git-path",
                    "index.lock",
                ],
            )
            .unwrap(),
        );
        fs::write(&lock, "").unwrap();
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "FAIL", "{}", o.text);
        assert_eq!(o.json["phase"], "ready");
        assert_eq!(o.json["not_the_patch"], true);
        assert!(o.text.contains("index.lock"), "{}", o.text);
        fs::remove_file(&lock).unwrap();

        // The person's checkout has an edit to tool.txt, not committed.
        fs::write(c.repo.join("tool.txt"), "the person's own edit\n").unwrap();
        let o = gate(&c, &patch, false);
        assert_eq!(o.json["verdict"], "FAIL", "{}", o.text);
        assert_eq!(o.json["phase"], "ready");
        assert_eq!(o.json["not_the_patch"], true);
        assert!(o.text.contains("tool.txt"), "{}", o.text);
        assert_eq!(
            o.json["why"].as_str().unwrap().matches("tool.txt").count(),
            1
        );
        assert_eq!(dry_run(&c, &patch).json["verdict"], "PASS");
        git_in(&c.repo, &["checkout", "--", "tool.txt"]);

        // An edit to another file doesn't stop the landing.
        fs::write(c.repo.join("README.md"), "the person's notes\n").unwrap();
        let o = gate(&c, &patch, false);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        git_in(&c.repo, &["checkout", "--", "README.md"]);

        let failing = patch_of(&c, "tool.txt", "a failing test\n");
        let o = dry_run(&c, &failing);
        assert_eq!(o.json["verdict"], "FAIL", "{}", o.text);
        assert_eq!(o.json["not_the_patch"], false);
    }

    /// The gate's game check is `just check`'s, through the patched gp: the
    /// gp `just check` runs is the one built from the patched tree. With no
    /// ROM folder and no games checkout the gate passes, and it reads and
    /// makes nothing of a game's: no score homes, no module builds.
    #[test]
    fn the_gate_checks_through_the_patched_gp_with_no_rom_and_no_games() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        let tree = fs::canonicalize(&c.tree).unwrap();
        assert_eq!(
            stamp(&c, "just-gp").unwrap().trim(),
            format!("built in {}", tree.display())
        );
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "ctl",
                "fix.patch",
                "home",
                "logs",
                "passed",
                "repo",
                "steps.jsonl",
                "target",
                "target-tests",
                "times",
                "tree"
            ]
        );
        assert!(!c.home.join("work").exists(), "no module builds started");
    }

    /// The gate's own test run is the whole workspace in the gate's
    /// profile with the slow tests off (they are CI's): built once, then
    /// run once. A failing test there fails the gate.
    #[test]
    fn the_gates_test_run_has_every_test_and_a_failure_stops_it() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        let seen = fs::read_to_string(tests_target(&c).join("cargo-test-seen")).unwrap();
        let runs: Vec<&str> = seen.lines().collect();
        assert_eq!(
            runs,
            [
                "test -q --workspace --profile fastdev --no-run | GP_SLOW_TESTS=",
                "test -q --workspace --profile fastdev -- | GP_SLOW_TESTS=",
            ],
            "{seen}"
        );
        assert_eq!(
            fs::read_to_string(c.target.join("just-seen")).unwrap(),
            format!("GP_GATE_RUNS_TESTS=1\nGP_GATE_RUN={}\n", std::process::id()),
            "just check leaves the tests to this gate's run, which it names"
        );

        let patch = patch_of(&c, "tool.txt", "a failing test\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "FAIL", "{}", o.text);
        assert_eq!(o.json["phase"], "tests", "{}", o.text);
        assert!(
            o.text.contains("cargo test (rest) failed (fails)"),
            "{}",
            o.text
        );
    }

    /// Each step of a gate run leaves a line in steps.jsonl with its
    /// seconds, and `just check`'s own step lines are kept as `check/NAME`.
    #[test]
    fn a_gate_run_leaves_a_line_per_step() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        let text = fs::read_to_string(dir.path().join("steps.jsonl")).unwrap();
        let steps: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for want in [
            "checkout",
            "apply",
            "hook",
            "fmt",
            "build",
            "lane check",
            "check/fmt",
            "check/baseline-mosshop",
            "test compile",
            "tests/rest",
            "lane tests",
            "lanes",
            "commit",
            "try",
        ] {
            let s = steps
                .iter()
                .find(|s| s["step"] == want)
                .unwrap_or_else(|| panic!("no {want} in {text}"));
            assert_eq!(s["ok"], true, "{want}");
            assert!(s["secs"].as_f64().is_some() && s["since_start"].as_f64().is_some());
        }
        for gone in ["tip build", "tip publish", "lane canary"] {
            assert!(!steps.iter().any(|s| s["step"] == gone), "{gone} in {text}");
        }
        let fmt = steps.iter().find(|s| s["step"] == "check/fmt").unwrap();
        assert_eq!(fmt["secs"], 0.4);
        assert_eq!(
            check_steps(
                "x\ncheck-step: clippy 25.0 s\n  check-step: play-web 3 s (failed)\ncheck-step: bad\n"
            ),
            [
                ("clippy".to_owned(), 25.0, true),
                ("play-web".to_owned(), 3.0, false)
            ]
        );
    }

    /// A patch cargo fmt refuses fails at "fmt" before anything is built
    /// or tested; a tool missing from PATH fails before the tip is touched.
    #[test]
    fn fmt_and_the_tools_fail_first() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let patch = patch_of(&c, "tool.txt", "bad format\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["phase"], "fmt", "{}", o.text);
        assert!(
            o.json["why"]
                .as_str()
                .unwrap()
                .starts_with("cargo fmt would change src/lib.rs: run cargo fmt --all"),
            "the first line names the files: {}",
            o.text
        );
        assert!(o.text.contains("Diff in src/lib.rs"), "{}", o.text);
        assert!(!tests_target(&c).join("cargo-test-seen").exists());
        let steps = fs::read_to_string(dir.path().join("steps.jsonl")).unwrap();
        assert!(
            !steps.contains(r#""step":"build""#),
            "the patch wasn't built: {steps}"
        );

        let mut c = c;
        c.tools = &["just", "no-such-tool-here"];
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["phase"], "preflight", "{}", o.text);
        assert!(
            o.text.contains("needs no-such-tool-here on PATH"),
            "{}",
            o.text
        );
    }

    /// rustfmt's two ways of naming a file, absolute in the tree or not,
    /// each named once, as a path in the tree.
    #[test]
    fn a_fmt_failure_names_each_file_once_in_the_tree() {
        let all = "Diff in /t/tree/crates/a/src/lib.rs at line 3:\n-x\n+y\n\
            Diff in /t/tree/crates/a/src/lib.rs at line 9:\n\
            Diff in /t/tree/crates/b/src/main.rs:12:\n\
            Diff in other.rs at line 1:\n";
        let why = fmt_why(Path::new("/t/tree"), all, "the tail");
        assert_eq!(
            why,
            "cargo fmt would change crates/a/src/lib.rs, crates/b/src/main.rs, other.rs: run cargo fmt --all in your worktree and make the patch again\nthe tail"
        );
        assert_eq!(
            fmt_why(Path::new("/t"), "error: no rustfmt", "the tail"),
            "the tail"
        );
    }

    /// The gate after a yes lands a tree its dry run passed without running
    /// just check and the tests again, and says so; the same patch on a tip
    /// that moved is another tree, and is checked.
    #[test]
    fn a_yes_after_a_passing_dry_run_lands_without_checking_again() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let lanes = || {
            fs::read_to_string(dir.path().join("steps.jsonl"))
                .unwrap()
                .lines()
                .filter(|l| l.contains(r#""step":"lane check""#))
                .count()
        };
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(o.text.contains("this pass is kept"), "{}", o.text);
        assert_eq!(lanes(), 1);

        let o = gate(&c, &patch, false);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(o.text.contains("not checked again"), "{}", o.text);
        assert_eq!(lanes(), 1, "just check and the tests ran once");
        let commit = o.json["commit"].as_str().unwrap();
        assert_eq!(git(&c.repo, &["rev-parse", "main"]).unwrap(), commit);
        assert!(
            c.home
                .join("bin")
                .join(format!("gp-{}", &commit[..12]))
                .is_file(),
            "the build is published"
        );

        let patch = patch_of(&c, "tool.txt", "the tool, said a third way\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!(lanes(), 2);
        fs::write(c.repo.join("README.md"), "the tool, moved on\n").unwrap();
        git_in(&c.repo, &["commit", "-q", "-am", "the tip moves"]);
        let o = gate(&c, &patch, false);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(!o.text.contains("not checked again"), "{}", o.text);
        assert_eq!(lanes(), 3, "the moved tip is checked again");
    }

    /// A caller's CARGO_INCREMENTAL doesn't reach the gate's builds.
    #[test]
    fn the_gates_builds_drop_a_callers_incremental_switch() {
        let mut cmd = Command::new("cargo");
        cmd.env("CARGO_INCREMENTAL", "0");
        gate_env(&mut cmd);
        let envs: Vec<_> = cmd.get_envs().collect();
        for k in ["CARGO_INCREMENTAL", "CARGO_PROFILE_FASTDEV_INCREMENTAL"] {
            assert!(
                envs.contains(&(std::ffi::OsStr::new(k), None)),
                "{k}: {envs:?}"
            );
        }
        let c = cx(Path::new("/nowhere"));
        let envs: Vec<_> = cargo(&c)
            .get_envs()
            .map(|(k, v)| (k.to_owned(), v.map(ToOwned::to_owned)))
            .collect();
        assert!(
            envs.contains(&("CARGO_INCREMENTAL".into(), None)),
            "{envs:?}"
        );
    }

    /// Files a step writes under the parts cargo watches are named, and
    /// older files and target/ are not.
    #[test]
    fn the_files_a_check_changed_are_named() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path();
        git_in(t, &["init", "-q"]);
        fs::create_dir_all(t.join("crates/a/src")).unwrap();
        fs::create_dir_all(t.join("crates/a/target")).unwrap();
        fs::write(t.join("crates/a/src/lib.rs"), "old").unwrap();
        set_time(&t.join("crates/a/src/lib.rs"), 1000);
        let since = SystemTime::now() - Duration::from_secs(5);
        fs::write(t.join("crates/a/src/new.rs"), "x").unwrap();
        fs::write(t.join("crates/a/target/out"), "x").unwrap();
        fs::write(t.join("Cargo.lock"), "x").unwrap();
        assert_eq!(
            changed_since(t, since),
            ["Cargo.lock", "crates/a/src/new.rs"]
        );
    }

    /// just check and the tests run at once: each lane waits for the other
    /// to start (`ctl/meet`), so run one after the other the first would
    /// wait alone and say so. No clock bound: a loaded machine makes them
    /// slower, not apart.
    #[test]
    fn lanes_run_at_the_same_time() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        switch(&c, "meet", "");
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        for n in ["just.start", "test.start", "just.end", "test.end"] {
            assert!(stamp(&c, n).is_some(), "{n}: the lane didn't run through");
        }
        for n in ["just", "test"] {
            assert!(
                stamp(&c, &format!("{n}.alone")).is_none(),
                "{n} waited 30 s and the other lane didn't start: they ran one after the other"
            );
        }
    }

    /// A lane that fails stops the other at once: just check fails after
    /// 0.2 s while a test run would take 30.
    #[test]
    fn the_first_failing_lane_stops_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        switch(&c, "just-fails", "");
        switch(&c, "test-sleep", "30");
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "FAIL", "{}", o.text);
        assert_eq!(o.json["phase"], "just check", "{}", o.text);
        // Not stopped, a test run would have slept its 30 s and written
        // test.end before the gate ended; no clock bound is needed.
        assert!(stamp(&c, "test.end").is_none(), "the tests were stopped");
        let pid = stamp(&c, "test.pid").unwrap();
        let alive = Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap()
            .success();
        assert!(!alive, "the test run {pid} is gone");
    }

    /// A failing test stops just check.
    #[test]
    fn a_failing_test_stops_check_early() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        switch(&c, "just-sleep", "30");
        let patch = patch_of(&c, "tool.txt", "a failing test\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["phase"], "tests", "{}", o.text);
        // Not stopped, just check would have slept its 30 s and written
        // just.end before the gate ended.
        assert!(stamp(&c, "just.end").is_none(), "just check was stopped");
    }

    /// A landing moves the branch to the commit the try made and publishes
    /// that build as the gp of the commit; a dry run lands and publishes
    /// nothing.
    #[test]
    fn a_landing_lands_and_publishes_the_patched_build() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let tip = git(&c.repo, &["rev-parse", "main"]).unwrap();
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!(git(&c.repo, &["rev-parse", "main"]).unwrap(), tip);
        assert!(
            !c.home.join("bin/gp").exists(),
            "a dry run publishes nothing"
        );

        let o = gate(&c, &patch, false);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        let commit = o.json["commit"].as_str().unwrap();
        assert_eq!(git(&c.repo, &["rev-parse", "main"]).unwrap(), commit);
        let published = c.home.join("bin").join(format!("gp-{}", &commit[..12]));
        assert_eq!(
            fs::read(&published).unwrap(),
            fs::read(built_gp(&c)).unwrap()
        );
        assert!(
            o.text.contains(&format!("PASS: landed {commit} on main")),
            "{}",
            o.text
        );
    }

    /// A landing publishes the page's files beside the build, and a dry
    /// run after it, which leaves its patch's page in the gate's tree,
    /// leaves the published page as the landing left it: what gp serve
    /// reads (crate::repo::page_files) changes only when a landing
    /// publishes.
    #[test]
    fn a_dry_run_leaves_the_published_page_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        fs::create_dir_all(c.repo.join("mechanic")).unwrap();
        fs::write(c.repo.join("mechanic/screen.js"), "the page\n").unwrap();
        git_in(&c.repo, &["add", "-A"]);
        git_in(&c.repo, &["commit", "-q", "-m", "the page"]);

        let landed = patch_of(&c, "mechanic/screen.js", "the landed page\n");
        let o = gate(&c, &landed, false);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        let commit = o.json["commit"].as_str().unwrap();
        let build = c.home.join("bin").join(format!("gp-{}", &commit[..12]));
        let pages = crate::repo::pages_beside(&build);
        let pages = pages.expect("the page published with the build");
        let js = pages.join("mechanic/screen.js");
        let served = || fs::read_to_string(&js).unwrap();
        assert_eq!(served(), "the landed page\n");

        let tried = patch_of(&c, "mechanic/screen.js", "a page that may never land\n");
        let o = dry_run(&c, &tried);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!(crate::repo::pages_beside(&build), Some(pages.clone()));
        assert_eq!(
            served(),
            "the landed page\n",
            "a dry run leaves the published page alone"
        );
    }

    /// A patch of the rig as a batch item, its diff kept under `name`.
    fn item_of(c: &Cx, file: &str, text: &str, name: &str) -> Item {
        let p = patch_of(c, file, text);
        let kept = c.repo.parent().unwrap().join(name);
        fs::rename(&p.diff, &kept).unwrap();
        Item {
            patch: Patch {
                diff: kept,
                message: None,
                ..p
            },
            message: format!("{name} by the batch"),
            shown: name.into(),
            ticket: None,
        }
    }

    /// A batch lands each patch as a commit of its own after one build
    /// and one run of the checks; a patch that doesn't apply beside the
    /// others is left out and the rest land.
    #[test]
    fn a_batch_lands_its_patches_in_one_run_and_leaves_out_one_that_doesnt_apply() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let tip = git(&c.repo, &["rev-parse", "main"]).unwrap();
        let items = [
            item_of(&c, "tool.txt", "the tool, said again\n", "a.patch"),
            item_of(&c, "README.md", "the tool, read me\n", "b.patch"),
            item_of(&c, "tool.txt", "the tool, said otherwise\n", "c.patch"),
        ];
        let (o, dropped) = tries_of(&c, false, &items).unwrap();
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!(dropped.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [2]);
        assert!(o.text.contains("c.patch is left out"), "{}", o.text);
        let log = git(&c.repo, &["log", "--format=%s", &format!("{tip}..main")]).unwrap();
        assert_eq!(log, "b.patch by the batch\na.patch by the batch");
        let tests = fs::read_to_string(tests_target(&c).join("cargo-test-seen")).unwrap();
        let builds = tests.lines().filter(|l| l.contains("--no-run")).count();
        assert_eq!(builds, 1, "one build of the tests for the batch: {tests}");
    }

    /// A batch that lands answers the gate it took in with its landing; one
    /// that fails gives the other gate's patch back to gate itself and
    /// lands this gate's own alone.
    #[test]
    fn a_failed_batch_gives_the_others_back_and_lands_its_own_alone() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let queue_home = dir.path().join("gate-home");
        fs::create_dir_all(&queue_home).unwrap();
        let req = |shown: &str| crate::gate_queue::Request {
            patch: PathBuf::new(),
            shown: shown.into(),
            message: None,
            repo: c.repo.join(".git"),
            game: String::new(),
            onto: "main".into(),
            run: TOOL_RUN.into(),
            removes: Vec::new(),
            replaces: Vec::new(),
        };
        let a = |p: &Item| ToolGateArgs {
            patch: p.patch.diff.clone(),
            message: Some(p.message.clone()),
            onto: Some("main".into()),
            home: None,
            dry_run: false,
        };
        let round = |own: Item, other: Item| {
            let me =
                crate::gate_queue::Ticket::enter(&queue_home, req("own"), &own.patch.diff).unwrap();
            let them =
                crate::gate_queue::Ticket::enter(&queue_home, req("other"), &other.patch.diff)
                    .unwrap();
            let (name, _) = me.claim().pop().unwrap();
            let args = a(&own);
            let other = Item {
                ticket: Some(name),
                ..other
            };
            let o = batch(&c, &args, Some(&me), vec![own, other]).unwrap();
            (o, them.result())
        };

        let tip = git(&c.repo, &["rev-parse", "main"]).unwrap();
        let (o, theirs) = round(
            item_of(&c, "tool.txt", "the tool, said again\n", "a.patch"),
            item_of(&c, "README.md", "the tool, read me\n", "b.patch"),
        );
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!(theirs.unwrap().json["commit"], o.json["commit"]);
        let log = git(&c.repo, &["log", "--format=%s", &format!("{tip}..main")]).unwrap();
        assert_eq!(log.lines().count(), 2, "{log}");

        let tip = git(&c.repo, &["rev-parse", "main"]).unwrap();
        let (o, theirs) = round(
            item_of(&c, "README.md", "the tool, read me again\n", "c.patch"),
            item_of(&c, "tool.txt", "a failing test\n", "d.patch"),
        );
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(o.text.contains("the batch of 2 failed"), "{}", o.text);
        assert!(theirs.is_none(), "the other gate gates its patch itself");
        let log = git(&c.repo, &["log", "--format=%s", &format!("{tip}..main")]).unwrap();
        assert_eq!(log, "c.patch by the batch");
    }

    /// A slow test is the `fn` above its switch line; a mention of the
    /// switch in a comment is not one, and the runs are the rest, skipping
    /// each slow name, and one run per slow name.
    #[test]
    fn the_slow_tests_are_read_from_their_switch_lines() {
        let text = "/// Start with `if gp_arcade::skip_slow_test() { return; }`.\n\
            pub fn skip_slow_test() -> bool {\n    true\n}\n\
            mod tests {\n    #[test]\n    fn long_one() {\n        if gp_arcade::skip_slow_test() {\n            return;\n        }\n    }\n\
            #[test]\n    fn quick_one() {}\n}\n\
            #[test]\nfn other_long1() {\n    if skip_slow_test() {\n        return;\n    }\n}\n";
        assert_eq!(slow_in(text), ["long_one", "other_long1"]);
        let runs = test_runs(&["a".into(), "b".into()]);
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            runs,
            [
                ("rest".to_owned(), args(&["--skip", "a", "--skip", "b"])),
                ("a".to_owned(), args(&["a"])),
                ("b".to_owned(), args(&["b"])),
            ]
        );
        assert_eq!(test_runs(&[]), [("rest".to_owned(), vec![])]);
    }
}
