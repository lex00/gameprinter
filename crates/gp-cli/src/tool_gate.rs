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

fn progress(s: impl AsRef<str>) {
    eprintln!("tool gate: {}", s.as_ref());
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
    cmd
}

/// The gate's own build settings for a child that builds: the profile's
/// `incremental = true` (Cargo.toml, fastdev) stands, so a caller's
/// CARGO_INCREMENTAL=0, or a CARGO_PROFILE_FASTDEV_INCREMENTAL, doesn't
/// reach it and make every build in the kept target a whole one.
/// GP_GATE_NO_INCREMENTAL=1 keeps the caller's (after an incremental-cache
/// failure, say).
fn gate_env(cmd: &mut Command) -> &mut Command {
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
    run_step(
        cargo(c)
            .args(["fmt", "--all", "--check"])
            .current_dir(&c.tree),
        "cargo fmt --all --check",
    )
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
        if stop.load(Ordering::SeqCst) {
            kill_group(pid, &mut child);
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let read = |p: &Path| String::from_utf8_lossy(&fs::read(p).unwrap_or_default()).into_owned();
    let (stdout, stderr) = (read(&out), read(&err));
    match status {
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
                let r = f(stop);
                let _ = tx.send((name, phase, r));
            });
        }
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

/// Where a lane's output is kept: `<steps folder>/logs/NAME`.
fn lane_log(c: &Cx, name: &str) -> PathBuf {
    c.steps
        .file
        .with_file_name("logs")
        .join(name.replace(['/', ' '], "_"))
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
            .env("GP_SLOW_TESTS", "1");
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
    let runs = test_runs(&slow_tests(&c.tree));
    progress(format!(
        "cargo test: {} slow tests each in a run of its own, beside the rest",
        runs.len() - 1
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
            .filter_map(|h| h.join().expect("a test run's thread").err())
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
fn trial(
    c: &Cx,
    a: &ToolGateArgs,
    patch: &Patch,
    tip: &str,
    lines: &mut Vec<String>,
) -> Result<std::result::Result<(String, bool), Failed>> {
    macro_rules! refuse {
        ($why:expr, $phase:expr) => {
            return Ok(Err(Failed {
                why: $why,
                phase: $phase,
            }))
        };
    }
    let tree = &c.tree;
    let merged = match c.steps.time(
        "apply",
        || gate::apply_how(tree, &patch.diff, true),
        Result::is_ok,
    ) {
        Ok(how) => how,
        Err(why) => refuse!(why, "patch"),
    };
    if let Some(how) = &merged {
        lines.push(how.clone());
    }
    let files = git(tree, &["diff", "--cached", "--name-only", tip])?;
    if files.is_empty() {
        refuse!("the patch changes nothing".into(), "patch");
    }
    if let Some(f) = files
        .lines()
        .find(|f| f.starts_with(".githooks/") || f.starts_with("scripts/romguard/"))
    {
        refuse!(
            format!("the patch changes {f}, the ROM-blocking hook, which only the person edits"),
            "patch"
        );
    }
    let gate_gp = std::env::current_exe()?;
    if let Err(why) = c
        .steps
        .time("hook", || gate::hook(tree, &gate_gp), Result::is_ok)
    {
        refuse!(format!("the ROM-blocking hook refuses it: {why}"), "hook");
    }
    let msg = a
        .message
        .clone()
        .unwrap_or_else(|| "A patch, by gp tool gate".into());
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
        if let Err(e) = c.steps.time(
            "commit",
            || gate::commit(tree, &msg, patch, &gate_gp),
            Result::is_ok,
        ) {
            refuse!(format!("{e:#}"), "commit");
        }
        return Ok(Ok((git(tree, &["rev-parse", "HEAD"])?, true)));
    }
    if let Err(why) = c.steps.time("fmt", || fmt(c), Result::is_ok) {
        refuse!(why, "fmt");
    }
    if let Err(why) = c.steps.time("build", || build(c), Result::is_ok) {
        refuse!(why, "build");
    }
    // A merge that took the tip's CLI.md: written again by the merged gp.
    if merged
        .as_deref()
        .is_some_and(|m| m.contains("CLI.md written again"))
    {
        match gate::regenerate(tree, &built_gp(c)) {
            Ok(true) => lines.push("CLI.md written again by the merged gp".into()),
            Ok(false) => {}
            Err(e) => refuse!(format!("{e:#}"), "build"),
        }
    }
    let lanes: Vec<(&'static str, &'static str, LaneFn<'_>)> = vec![
        ("check", "just check", Box::new(|stop| check_lane(c, stop))),
        ("tests", "tests", Box::new(|stop| tests_lane(c, stop))),
    ];
    match run_lanes(c, lanes) {
        Ok(l) => lines.extend(l),
        Err(f) => return Ok(Err(f)),
    }
    if let Err(e) = c.steps.time(
        "commit",
        || gate::commit(tree, &msg, patch, &gate_gp),
        Result::is_ok,
    ) {
        refuse!(format!("{e:#}"), "commit");
    }
    Ok(Ok((git(tree, &["rev-parse", "HEAD"])?, false)))
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
    let lock = fs::File::create(home.join("gate.lock"))?;
    lock.lock()?;
    let tool = home.join("tool");
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
        tree: tool.join("tree"),
        target: tool.join("target"),
        home: paths.home.clone(),
        steps: Steps::new(tool.join("steps.jsonl"), &patch_hash),
        tools: TOOLS,
    };
    let o = tries(&c, a, &patch);
    c.steps.record(
        "gate",
        c.steps.t0.elapsed().as_secs_f64(),
        o.as_ref().is_ok_and(|o| o.exit == 0),
    );
    o
}

/// The gate's tries on the branch tip, each from a fresh tree, until one
/// lands, fails, or the tip has moved [`TIP_TRIES`] times.
fn tries(c: &Cx, a: &ToolGateArgs, patch: &Patch) -> Result<Outcome> {
    let mut lines = Vec::new();
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
    for _ in 0..TIP_TRIES {
        let tip = git(&c.repo, &["rev-parse", &c.onto])?;
        c.steps.time(
            "checkout",
            || gate::checkout(&c.repo, &c.tree, &tip),
            Result::is_ok,
        )?;
        let tried = c.steps.time(
            "try",
            || trial(c, a, patch, &tip, &mut lines),
            |r| matches!(r, Ok(Ok(_))),
        )?;
        match tried {
            Err(f) => {
                failed = Some(f);
                gate::clean(&c.tree);
                break;
            }
            Ok((commit, words_only)) => {
                if a.dry_run {
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
                                    &mut lines,
                                )
                            },
                            Result::is_ok,
                        )?;
                        return Ok(Outcome::ok(
                            json!({"verdict": "PASS", "commit": commit, "tip": tip, "lines": lines}),
                            lines.join("\n"),
                        ));
                    }
                    Err((why, moved)) if moved => {
                        lines.push(format!("the tip moved: {why}; again"));
                    }
                    Err((why, _)) => bail!("couldn't land {commit}: {why}"),
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

fn fail(f: Failed, mut lines: Vec<String>) -> Outcome {
    lines.push(format!("FAIL ({}): {}", f.phase, f.why));
    Outcome {
        json: json!({"verdict": "FAIL", "phase": f.phase, "why": f.why, "lines": lines}),
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
    /// profile with the slow tests on (GP_SLOW_TESTS=1): built once, then
    /// the slow test in a run of its own beside the rest, which skips it.
    /// A failing test there fails the gate.
    #[test]
    fn the_gates_test_run_has_every_test_and_a_failure_stops_it() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let patch = patch_of(&c, "tool.txt", "the tool, said again\n");
        let o = dry_run(&c, &patch);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        let seen = fs::read_to_string(tests_target(&c).join("cargo-test-seen")).unwrap();
        let mut runs: Vec<&str> = seen.lines().collect();
        runs[1..].sort_unstable();
        assert_eq!(
            runs,
            [
                "test -q --workspace --profile fastdev --no-run | GP_SLOW_TESTS=1",
                "test -q --workspace --profile fastdev -- --skip a_slow_one | GP_SLOW_TESTS=1",
                "test -q --workspace --profile fastdev -- a_slow_one | GP_SLOW_TESTS=1",
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
            "tests/a_slow_one",
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
