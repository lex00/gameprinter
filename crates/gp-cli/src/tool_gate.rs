//! `gp tool gate`: a patch to the tool repo (`gameprinter`), held to the
//! tool's own checks and a Donkey Kong canary. A worktree in the gate's
//! home is put at the branch tip and the game gate's no-op scored through
//! the tip's gp, then the patch is applied and built, `just check` and the
//! tests run, and the no-op scored through the patched gp: no row may get
//! worse. What passes lands on the branch
//! and is published at $GP_HOME/bin/gp. Keeps the shape of `gate.rs` and
//! shares its apply, hook, tree and landing code.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::clip_score::rom_of;
use crate::gate::{self, Failed, Patch, Row, Rows, git};
use crate::publish;

/// The canary game's folder in the games repo.
const CANARY: &str = "donkey-kong";
/// The profile the gate builds and publishes.
const PROFILE: &str = "fastdev";
const TIP_TRIES: usize = 5;
/// How many scored builds stay under `<gate home>/tool/canary/`, at about
/// 1.1 GB each.
const KEEP_BUILDS: usize = 2;
/// The version of the kept canary scores, in their files' names
/// (`base<N>-<key>.json`, [`canary_key`]) and in the key. Bumped when what
/// a score is made with changes, so a score made the old way is never read:
/// version 2 was the first made with the tip's own kit, in a fixed folder
/// and environment; version 3 is keyed by everything a score depends on.
const BASE_FORMAT: u32 = 3;
/// What a canary score keeps of the gate's environment. The rest goes, since
/// a variable such as GP_BUILD, GP_DK_MODULE, GP_ASSETS or GP_TOOL_SRC could
/// change a score; `scored` sets GP_HOME, GP_ROM_DIR and GP_TOOL_SRC itself.
const SCORE_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "RUSTUP_HOME",
    "CARGO_HOME",
    "GP_JOBS",
];

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
    /// Its worktree, target folder and canary runs are kept there.
    #[arg(long)]
    pub home: Option<PathBuf>,
    /// The games checkout the canary reads (default: the folder `games`
    /// beside this repo, else $GP_GAMES_DIR).
    #[arg(long)]
    pub games: Option<PathBuf>,
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
    canary_dir: PathBuf,
    /// The person's $GP_HOME: where builds are published.
    home: PathBuf,
    games_repo: PathBuf,
    games_tree: PathBuf,
    paths: Paths,
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
/// failure, say). The canary's module builds set their own (module.rs).
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
                fs::metadata(d.join(t)).is_ok_and(|m| {
                    use std::os::unix::fs::PermissionsExt as _;
                    m.is_file() && m.permissions().mode() & 0o111 != 0
                })
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
    use std::os::unix::process::CommandExt as _;
    if let Some(d) = log.parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let (out, err) = (log.with_extension("out"), log.with_extension("err"));
    let file = |p: &Path| fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()));
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(file(&out)?)
        .stderr(file(&err)?)
        .process_group(0)
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
/// too, for the other: 40 s or more a gate, and a gp that changed under the
/// canary. Apart, each stays built.
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

/// What the canary's kit would read that a commit of the index doesn't
/// hold: the files that differ from the index, and the files under crates/
/// and third_party/ git doesn't track and doesn't ignore.
fn uncommitted(tree: &Path) -> std::result::Result<Vec<String>, String> {
    let e = |e: anyhow::Error| format!("{e:#}");
    let changed = git(tree, &["diff", "--name-only", "-z"]).map_err(e)?;
    let others = git(
        tree,
        &[
            "ls-files",
            "-z",
            "--others",
            "--exclude-standard",
            "--",
            "crates",
            "third_party",
        ],
    )
    .map_err(e)?;
    Ok(changed
        .split('\0')
        .chain(others.split('\0'))
        .filter(|f| !f.is_empty())
        .map(str::to_owned)
        .collect())
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

/// The arguments of a canary score after `--home`'s value: in the kept
/// score's key, so a gate that scores another way never reads an old score.
const SCORE_ARGS: &[&str] = &["--json", "model", "gate", "--score-only", "--run", "canary"];

/// The rows of the corpus scored by `gp`: `gp model gate --score-only`
/// with a home of its own (`label`), since a home's caches are the build's.
/// gp finds its module kit in the tree it was built in, so `GP_TOOL_SRC`
/// names the gate's tree: the caller scores each side while the tree holds
/// that side's sources. It runs in its home with [`SCORE_ENV`] alone, so
/// the two sides differ only in the gp and the kit.
fn scored(c: &Cx, gp: &Path, label: &str) -> std::result::Result<Value, String> {
    scored_until(c, gp, label, &AtomicBool::new(false))
}

/// `cmd` with the gate's environment cleared but [`SCORE_ENV`].
fn kept_env(cmd: &mut Command) -> &mut Command {
    cmd.env_clear();
    for k in SCORE_ENV {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    cmd
}

/// `gp` as a canary score runs it: in `home`, with [`SCORE_ENV`] alone and
/// GP_HOME, GP_ROM_DIR and GP_TOOL_SRC set by the gate.
fn score_cmd(c: &Cx, gp: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(gp);
    kept_env(&mut cmd)
        .current_dir(home)
        .env("GP_HOME", home)
        .env("GP_ROM_DIR", &c.paths.roms)
        .env("GP_TOOL_SRC", &c.tree);
    cmd
}

/// The home a score is made in: `<name>-<the score's key, 12 hex>`. Named
/// by the key, a home is never met by a score of another tree, so the kit
/// it holds is always the tree's that the key describes.
fn score_label(name: &str, parts: &KeyParts) -> String {
    format!("{name}-{}", &canary_key(parts)[..12])
}

/// The module kit the score in `home` reads, made with `gp game kit` into
/// `home/work/module-kit/<the gp's hash>`, where that gp looks for it
/// (module.rs, `own_kit`). Made before the score starts, while nothing else
/// runs in the tree, so a file a check writes and puts back while the canary
/// runs can't end up in it. A home named by its key ([`score_label`]) that
/// already has one keeps it: the same key is the same tree.
fn make_kit(c: &Cx, gp: &Path, parts: &KeyParts, home: &Path) -> std::result::Result<(), String> {
    let dir = home.join("work").join("module-kit").join(&parts.gp[..16]);
    if dir.join("Cargo.lock").is_file() {
        return Ok(());
    }
    let kits = home.join("work").join("module-kit");
    fs::create_dir_all(&kits).map_err(|e| format!("{}: {e}", kits.display()))?;
    let tmp = dir.with_extension(format!("tmp{}", std::process::id()));
    run_step(
        score_cmd(c, gp, home).args(["game", "kit", "-o"]).arg(&tmp),
        "gp game kit (the canary's module kit)",
    )?;
    fs::rename(&tmp, &dir).map_err(|e| format!("moving the kit to {}: {e}", dir.display()))
}

/// [`scored`], stopped when `stop` is set.
fn scored_until(
    c: &Cx,
    gp: &Path,
    label: &str,
    stop: &AtomicBool,
) -> std::result::Result<Value, String> {
    let game = c.games_tree.join(CANARY);
    let home = c.canary_dir.join(label);
    fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let mut cmd = score_cmd(c, gp, &home);
    cmd.args(&SCORE_ARGS[..3])
        .arg(&game)
        .args(&SCORE_ARGS[3..])
        .arg("--home")
        .arg(&home);
    let r = run_killable(
        &mut cmd,
        &format!("{} ({label})", gp.display()),
        stop,
        &lane_log(c, &format!("canary-{label}")),
    );
    mark_used(&home);
    prune_builds(&c.canary_dir);
    let (out, _) = r?;
    serde_json::from_str(&out).map_err(|e| format!("gp gave no JSON: {e}"))
}

/// Stamps a build's home as just used: scoring a build again reuses its
/// home, which needn't change the folder's own time.
fn mark_used(home: &Path) {
    if let Ok(f) = fs::File::open(home) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// Removes all but the newest [`KEEP_BUILDS`] scored builds: the `tip-*` and
/// `patch-*` folders. The `base-*.json` scores are small and stay, so a
/// second patch on the same tip still doesn't score the tip again.
fn prune_builds(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let mut builds: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name();
            let n = n.to_str()?;
            (n.starts_with("tip-") || n.starts_with("patch-")).then_some(())?;
            e.file_type().ok()?.is_dir().then_some(())?;
            Some((e.metadata().ok()?.modified().ok()?, e.path()))
        })
        .collect();
    builds.sort();
    for (_, p) in builds.iter().rev().skip(KEEP_BUILDS) {
        match fs::remove_dir_all(p) {
            Ok(()) => progress(format!("canary: removed old build {}", p.display())),
            Err(e) => progress(format!("canary: couldn't remove {}: {e}", p.display())),
        }
    }
}

fn rows_of(v: &Value) -> Rows {
    v["clips"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| (c["id"].as_str().unwrap_or("").to_owned(), Row::of(c)))
        .collect()
}

/// What a canary score depends on, each part as text, for the key its
/// kept score is filed under ([`canary_key`]).
#[derive(Clone)]
struct KeyParts {
    /// blake3 of the gp file that scored.
    gp: String,
    /// The tool tree it scored against ([`tree_state`]).
    tree: String,
    /// The games tree ([`games_state`]).
    games: String,
    /// blake3 of the canary's ROM.
    rom: String,
    /// `rustc -vV` in the tree, the toolchain its module is built with, run
    /// with [`SCORE_ENV`] alone, and the cargo settings in CARGO_HOME.
    toolchain: String,
    /// The environment the score kept ([`SCORE_ENV`], and GP_ROM_DIR).
    env: Vec<(String, String)>,
    /// How it was scored ([`SCORE_ARGS`] and the canary's name).
    args: String,
}

/// The key a canary score is kept under: the first 24 hex digits of blake3
/// over [`BASE_FORMAT`] and each part, each followed by a 0 byte.
fn canary_key(p: &KeyParts) -> String {
    let mut h = blake3::Hasher::new();
    let mut part = |s: &str| {
        h.update(s.as_bytes());
        h.update(&[0]);
    };
    part(&BASE_FORMAT.to_string());
    for s in [&p.gp, &p.tree, &p.games, &p.rom, &p.toolchain] {
        part(s);
    }
    for (k, v) in &p.env {
        part(k);
        part(v);
    }
    part(&p.args);
    h.finalize().to_hex()[..24].to_owned()
}

fn file_hash(p: &Path) -> std::result::Result<String, String> {
    let mut h = blake3::Hasher::new();
    let mut f = fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    std::io::copy(&mut f, &mut h).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(h.finalize().to_hex().to_string())
}

/// The tool tree as a canary score reads it: the tree of its index (`git
/// write-tree`), every file that differs from the index with its bytes,
/// and the files under crates/ and third_party/ git doesn't track, ignored
/// ones too, with their bytes, since the module kit copies those folders
/// as they are on disk (module.rs, `walk`). target/, node_modules/ and
/// .DS_Store, which the kit leaves out, are left out.
fn tree_state(tree: &Path) -> std::result::Result<String, String> {
    let e = |e: anyhow::Error| format!("{e:#}");
    let mut h = blake3::Hasher::new();
    h.update(git(tree, &["write-tree"]).map_err(e)?.as_bytes());
    let changed = git(tree, &["diff", "--name-only", "-z"]).map_err(e)?;
    let others = git(
        tree,
        &["ls-files", "-z", "--others", "--", "crates", "third_party"],
    )
    .map_err(e)?;
    let ignored = git(
        tree,
        &[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--",
            "crates",
            "third_party",
        ],
    )
    .map_err(e)?;
    // As `walk` leaves them out: node_modules and .DS_Store anywhere, but a
    // `target` folder only as a crate's build output (crates/NAME/target, or
    // one beside a Cargo.toml), since a source folder can be named target.
    let left_out = |f: &str| {
        let parts: Vec<&str> = f.split('/').collect();
        parts.iter().enumerate().any(|(i, c)| match *c {
            "node_modules" | ".DS_Store" => true,
            "target" => {
                let dir = parts[..i].join("/");
                i == 0
                    || (i == 2 && parts[0] == "crates")
                    || tree.join(&dir).join("Cargo.toml").is_file()
            }
            _ => false,
        })
    };
    let mut files: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for list in [&changed, &others, &ignored] {
        files.extend(list.split('\0').filter(|f| !f.is_empty() && !left_out(f)));
    }
    for f in files {
        h.update(b"\0");
        h.update(f.as_bytes());
        h.update(b"\0");
        // A file the index has and the tree lost reads as empty: its name
        // still counts.
        h.update(&fs::read(tree.join(f)).unwrap_or_default());
    }
    Ok(h.finalize().to_hex().to_string())
}

/// The games tree as a score reads it: its HEAD's tree, what `git status`
/// says differs, and the files that differ from HEAD and those git doesn't
/// track (ignored ones too), with their bytes.
fn games_state(games: &Path) -> std::result::Result<String, String> {
    let e = |e: anyhow::Error| format!("{e:#}");
    let head = git(games, &["rev-parse", "HEAD^{tree}"]).map_err(e)?;
    let status = git(games, &["status", "--porcelain", "--ignored"]).map_err(e)?;
    // Without --exclude-standard: the untracked files, ignored ones too.
    let others = git(games, &["ls-files", "-z", "--others"]).map_err(e)?;
    let changed = git(games, &["diff", "HEAD", "--name-only", "-z"]).map_err(e)?;
    let mut h = blake3::Hasher::new();
    for f in changed
        .split('\0')
        .chain(others.split('\0'))
        .filter(|f| !f.is_empty())
    {
        h.update(f.as_bytes());
        h.update(b"\0");
        h.update(&fs::read(games.join(f)).unwrap_or_default());
        h.update(b"\0");
    }
    Ok(format!("{head}\n{status}\n{}", h.finalize().to_hex()))
}

/// The cargo settings of `cargo_home` (its config.toml, or the older
/// config), which can set flags for the module's build: each file's name
/// and bytes.
fn cargo_config(cargo_home: &Path) -> String {
    let mut out = String::new();
    for f in ["config.toml", "config"] {
        if let Ok(b) = fs::read(cargo_home.join(f)) {
            out.push_str(&format!("\n{f}:\n{}", String::from_utf8_lossy(&b)));
        }
    }
    out
}

/// The key parts of a score through `gp` against the trees as they are.
fn key_parts(c: &Cx, gp: &Path) -> std::result::Result<KeyParts, String> {
    let rom = rom_of(&c.paths, &c.games_tree.join(CANARY), None).map_err(|e| format!("{e:#}"))?;
    // In the score's environment, so a caller's RUSTUP_TOOLCHAIN, which the
    // score doesn't see, doesn't name the toolchain.
    let toolchain = kept_env(&mut Command::new("rustc"))
        .arg("-vV")
        .current_dir(&c.tree)
        .output()
        .map_err(|e| format!("rustc -vV: {e}"))?;
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cargo")))
        .unwrap_or_default();
    let mut env: Vec<(String, String)> = SCORE_ENV
        .iter()
        .map(|k| {
            let v = std::env::var_os(k).unwrap_or_default();
            ((*k).to_owned(), v.to_string_lossy().into_owned())
        })
        .collect();
    env.push((
        "GP_ROM_DIR".into(),
        c.paths.roms.to_string_lossy().into_owned(),
    ));
    Ok(KeyParts {
        gp: file_hash(gp)?,
        tree: tree_state(&c.tree)?,
        games: games_state(&c.games_tree)?,
        rom: blake3::hash(&rom).to_hex().to_string(),
        toolchain: format!(
            "{}{}{}",
            String::from_utf8_lossy(&toolchain.stdout),
            String::from_utf8_lossy(&toolchain.stderr),
            cargo_config(&cargo_home)
        ),
        env,
        args: format!("{} {CANARY}", SCORE_ARGS.join(" ")),
    })
}

/// The kept score's file for a key.
fn base_file(c: &Cx, key: &str) -> PathBuf {
    c.canary_dir.join(format!("base{BASE_FORMAT}-{key}.json"))
}

/// Writes a kept score through a new file renamed into place, so a reader
/// never sees half of one.
fn keep_score(file: &Path, v: &Value) -> std::result::Result<(), String> {
    if let Some(d) = file.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, serde_json::to_vec(v).unwrap_or_default()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, file).map_err(|e| e.to_string())
}

#[cfg(test)]
thread_local! {
    static VERIFY_IN_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// GP_GATE_CANARY_VERIFY=1: score the tip even when its score is kept, and
/// fail unless the two agree.
fn verify_mode() -> bool {
    #[cfg(test)]
    if VERIFY_IN_TEST.with(std::cell::Cell::get) {
        return true;
    }
    std::env::var_os("GP_GATE_CANARY_VERIFY").is_some_and(|v| v == "1")
}

/// The tip's canary score, or `None` with a line saying why when there's no
/// ROM. Called while the tree is at the tip, before the patch is applied:
/// the tip's gp reads its kit from the tree, and scored after the patch it
/// would run the patched gp-sim and gp-interp inside Donkey Kong's module.
/// Kept under the key of everything it depends on ([`key_parts`]), so a
/// second patch on the same tip doesn't pay for it again, nor a gate on a
/// tip the gate landed (the landing keeps its patched score under the key
/// this computes).
fn tip_score(
    c: &Cx,
    tip: &str,
    tip_gp: &Path,
    lines: &mut Vec<String>,
) -> std::result::Result<Option<Value>, String> {
    let game = c.games_tree.join(CANARY);
    if rom_of(&c.paths, &game, None).is_err() {
        lines.push(format!(
            "canary skipped: no ROM for {CANARY} in {}",
            c.paths.roms.display()
        ));
        return Ok(None);
    }
    let parts = key_parts(c, tip_gp)?;
    let key = canary_key(&parts);
    let file = base_file(c, &key);
    let kept = fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(|v| !rows_of(v).is_empty());
    if let Some(v) = &kept
        && !verify_mode()
    {
        lines.push(format!(
            "canary: the tip's score is kept ({})",
            file.display()
        ));
        return Ok(Some(v.clone()));
    }
    progress(format!("canary: scoring {CANARY} through the tip's gp"));
    let label = score_label(&format!("tip-{}", &tip[..12]), &parts);
    make_kit(c, tip_gp, &parts, &c.canary_dir.join(&label))?;
    let v = scored(c, tip_gp, &label)?;
    if let Some(k) = kept {
        let (a, b) = (rows_of(&k), rows_of(&v));
        let same = a.len() == b.len()
            && a.iter().zip(&b).all(|((i, _), (j, _))| i == j)
            && gate::worse(&a, &b).is_none()
            && gate::worse(&b, &a).is_none();
        if !same {
            return Err(format!(
                "GP_GATE_CANARY_VERIFY: the tip scored now differs from its kept score {}: the key misses something a score depends on",
                file.display()
            ));
        }
        lines.push("canary: the kept score of the tip agrees with a new one".into());
        return Ok(Some(v));
    }
    keep_score(&file, &v)?;
    Ok(Some(v))
}

/// The patched gp's rows held to the tip's (`base`): a FAIL, or the line
/// saying they held.
fn compare(base: &Value, new: &Value) -> std::result::Result<String, String> {
    let (base_rows, new_rows) = (rows_of(base), rows_of(new));
    if base_rows.is_empty() {
        return Err("the tip's canary score has no rows".into());
    }
    for (id, _) in &base_rows {
        if !new_rows.iter().any(|(i, _)| i == id) {
            return Err(format!("{id}: no row through the patched gp"));
        }
    }
    if let Some(why) = gate::worse(&base_rows, &new_rows) {
        return Err(why);
    }
    Ok(format!(
        "canary: {} clips of {CANARY} held through the patched gp",
        base_rows.len()
    ))
}

/// The canary: the patched gp's score in the home `label` against the
/// tip's (`base`), the score, or why it failed. `stop` stops it when
/// another lane failed.
fn canary(
    c: &Cx,
    base: &Value,
    gp: &Path,
    label: &str,
    stop: &AtomicBool,
) -> std::result::Result<(Value, String), String> {
    progress(format!("canary: scoring {CANARY} through the patched gp"));
    let new = scored_until(c, gp, label, stop)
        .map_err(|why| format!("the patched gp can't score {CANARY}: {why}"))?;
    let line = compare(base, &new)?;
    Ok((new, line))
}

/// A copy of the gate's build, `<canary dir>/bins/gp-<blake3, 16 hex>`, to
/// score: the build in the target folder can be replaced while the checks
/// run, and the gate publishes only bytes it scored. Returns the copy and
/// the full hash. The four newest copies are kept.
fn snapshot(c: &Cx) -> std::result::Result<(PathBuf, String), String> {
    let gp = built_gp(c);
    let hash = file_hash(&gp)?;
    let dir = c.canary_dir.join("bins");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(format!("gp-{}", &hash[..16]));
    if !dest.is_file() || file_hash(&dest).ok().as_deref() != Some(hash.as_str()) {
        // A new file renamed into place (publish.rs has why).
        let tmp = dir.join(format!("gp-{}.tmp{}", &hash[..16], std::process::id()));
        fs::copy(&gp, &tmp).map_err(|e| format!("copying {}: {e}", gp.display()))?;
        fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
        if file_hash(&dest)? != hash {
            return Err(format!("{} changed while it was copied", gp.display()));
        }
    }
    mark_used(&dest);
    if let Ok(rd) = fs::read_dir(&dir) {
        let mut bins: Vec<(std::time::SystemTime, PathBuf)> = rd
            .flatten()
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        bins.sort();
        for (_, p) in bins.iter().rev().skip(4) {
            let _ = fs::remove_file(p);
        }
    }
    Ok((dest, hash))
}

/// `<gate home>/tool/published.json`: each commit the gate published, with
/// the blake3 of the build. A published tip build is scored as the tip only
/// when it is listed here with its bytes.
fn published_file(c: &Cx) -> PathBuf {
    c.steps.file.with_file_name("published.json")
}

fn published_by_gate(c: &Cx, commit: &str, file: &Path) -> bool {
    let Ok(b) = fs::read(published_file(c)) else {
        return false;
    };
    let Ok(v) = serde_json::from_slice::<Value>(&b) else {
        return false;
    };
    v[commit]
        .as_str()
        .is_some_and(|h| file_hash(file).ok().as_deref() == Some(h))
}

fn note_published(c: &Cx, commit: &str, file: &Path) -> std::result::Result<(), String> {
    let f = published_file(c);
    let mut v: Value = fs::read(&f)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    v[commit] = json!(file_hash(file)?);
    keep_score(&f, &v)
}

fn games_dir(a: &ToolGateArgs, repo: &Path) -> Result<PathBuf> {
    if let Some(g) = &a.games {
        return fs::canonicalize(g).with_context(|| g.display().to_string());
    }
    if let Some(g) = std::env::var_os("GP_GAMES_DIR") {
        return Ok(PathBuf::from(g));
    }
    // Beside this repo's main checkout, where a worktree's own parent isn't.
    let common = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let g = Path::new(&common)
        .parent()
        .and_then(Path::parent)
        .map(|p| p.join("games"));
    match g {
        Some(g) if g.join(CANARY).is_dir() => Ok(g),
        _ => bail!(
            "no games checkout beside this repo for the canary; pass --games or set GP_GAMES_DIR"
        ),
    }
}

/// A patched gp's canary score, kept for the next gate once its commit
/// lands: the scored copy of the build, its hash, the key's parts and the
/// score.
struct Scored {
    gp: PathBuf,
    hash: String,
    /// The home it was scored in, whose module kit it read.
    home: PathBuf,
    parts: KeyParts,
    value: Value,
}

/// The commit a passing try made, and its canary score (`None`: no canary).
struct Tried {
    commit: String,
    scored: Option<Scored>,
}

/// The tree's patch, or why it was refused. `base` is the tip's canary
/// score (`None`: no canary), made before this applies the patch. After the
/// build, `just check`, the tests and the patched canary run at once as
/// lanes ([`run_lanes`]); the first failure stops the rest.
#[allow(clippy::too_many_arguments)]
fn trial(
    c: &Cx,
    a: &ToolGateArgs,
    patch: &Patch,
    tip: &str,
    base: Option<&Value>,
    patch_hash: &str,
    lines: &mut Vec<String>,
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
    // The canary scores a copy of this build, against the tree as it is
    // now: the key of its score is taken here, and its module kit made
    // here, before the lanes start, from the same quiet tree. What the kit
    // reads must be what lands: the index, so nothing may differ from it.
    let scoring = match base {
        None => None,
        Some(_) => {
            let loose = uncommitted(tree).map_err(|e| anyhow::anyhow!(e))?;
            if !loose.is_empty() {
                refuse!(
                    format!(
                        "the tree differs from what would be committed, so the canary would score something else: {}",
                        loose.join(", ")
                    ),
                    "tree"
                );
            }
            let made = c.steps.time(
                "canary kit",
                || {
                    let (gp, hash) = snapshot(c)?;
                    let parts = key_parts(c, &gp)?;
                    let label = score_label(&format!("patch-{patch_hash}"), &parts);
                    make_kit(c, &gp, &parts, &c.canary_dir.join(&label))?;
                    Ok((gp, hash, parts, label))
                },
                Result::is_ok,
            );
            match made {
                Ok(s) => Some(s),
                Err(why) => refuse!(why, "canary"),
            }
        }
    };
    let lanes_from = std::time::SystemTime::now();
    let slot: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);
    let mut lanes: Vec<(&'static str, &'static str, LaneFn<'_>)> = vec![
        ("check", "just check", Box::new(|stop| check_lane(c, stop))),
        ("tests", "tests", Box::new(|stop| tests_lane(c, stop))),
    ];
    if let (Some(base), Some((gp, _, _, label))) = (base, &scoring) {
        let slot = &slot;
        lanes.push((
            "canary",
            "canary",
            Box::new(move |stop| {
                let (v, line) = canary(c, base, gp, label, stop)?;
                *slot.lock().expect("the canary's slot") = Some(v);
                Ok(vec![line])
            }),
        ));
    }
    match run_lanes(c, lanes) {
        Ok(l) => lines.extend(l),
        Err(f) => return Ok(Err(f)),
    }
    // What lands and is published must be what the canary scored: the
    // tree and the build's bytes. A check that changed the tree is refused:
    // the kit was made before it, and the commit holds the index, so no
    // score could stand for what lands. A check that built gp again from
    // the same tree means the canary scores the new build, now.
    let scored = match (base, scoring) {
        (Some(base), Some((gp, hash, parts, label))) => {
            let tree_now = tree_state(tree).map_err(|e| anyhow::anyhow!(e))?;
            if tree_now != parts.tree {
                let files = changed_since(tree, lanes_from);
                let named = if files.is_empty() {
                    "a file the canary's kit or the commit reads".to_owned()
                } else {
                    files.join(", ")
                };
                refuse!(
                    format!(
                        "a check changed the tree after the canary's kit was made, so what would land isn't what was scored: {named}"
                    ),
                    "tree"
                );
            }
            let now = file_hash(&built_gp(c)).map_err(|e| anyhow::anyhow!(e))?;
            if now == hash {
                let value = slot
                    .into_inner()
                    .expect("the canary's slot")
                    .context("the canary lane gave no score")?;
                Some(Scored {
                    gp,
                    hash,
                    home: c.canary_dir.join(label),
                    parts,
                    value,
                })
            } else {
                lines.push("canary: gp changed during the checks, scored again".to_owned());
                progress(lines.last().expect("just pushed"));
                let again = snapshot(c).and_then(|(gp, hash)| {
                    let parts = key_parts(c, &gp)?;
                    let label = score_label(&format!("patch-{patch_hash}"), &parts);
                    let home = c.canary_dir.join(&label);
                    let (value, line) = c.steps.time(
                        "canary (again)",
                        || {
                            make_kit(c, &gp, &parts, &home)?;
                            canary(c, base, &gp, &label, &AtomicBool::new(false))
                        },
                        Result::is_ok,
                    )?;
                    Ok((
                        Scored {
                            gp,
                            hash,
                            home,
                            parts,
                            value,
                        },
                        line,
                    ))
                });
                match again {
                    Ok((s, line)) => {
                        lines.push(line);
                        Some(s)
                    }
                    Err(why) => refuse!(why, "canary"),
                }
            }
        }
        _ => None,
    };
    let msg = a
        .message
        .clone()
        .unwrap_or_else(|| "A patch, by gp tool gate".into());
    if let Err(e) = c.steps.time(
        "commit",
        || gate::commit(tree, &msg, patch, &gate_gp),
        Result::is_ok,
    ) {
        refuse!(format!("{e:#}"), "commit");
    }
    Ok(Ok(Tried {
        commit: git(tree, &["rev-parse", "HEAD"])?,
        scored,
    }))
}

/// The game folders in `games` whose package.toml has hooks, so a module:
/// what a page open builds first.
fn hooked_games(games: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(games) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|d| {
            fs::read_to_string(d.join("package.toml"))
                .ok()
                .and_then(|t| t.parse::<toml::Table>().ok())
                .is_some_and(|t| t.contains_key("hooks"))
        })
        .collect();
    out.sort();
    out
}

/// After a landing, the hooked games' modules built with the gp just
/// published, so the first page open after it doesn't wait for them. The
/// new gp's module kit is the one its canary made (`<its score home>/work/
/// module-kit/<the gp's hash>`, same bytes, same key), copied into the
/// person's home first, so the builds don't read the gate's tree, which the
/// next gate moves. The builds run detached, one after another, with their
/// output in `$GP_HOME/work/warm-modules.log`; the gate doesn't wait, and a
/// failed one only means the page builds it on open, as before.
fn warm_modules(c: &Cx, commit: &str, scored: Option<&Scored>, lines: &mut Vec<String>) {
    use std::os::unix::process::CommandExt as _;
    let gp = c.home.join("bin").join(format!("gp-{}", &commit[..12]));
    let games = hooked_games(&c.games_repo);
    if games.is_empty() || !gp.is_file() {
        return;
    }
    let work = c.home.join("work");
    if let Some(s) = scored {
        let key = &s.hash[..16];
        let to = work.join("module-kit").join(key);
        // The kit the canary read, in the home it scored in.
        let from = Some(s.home.join("work").join("module-kit").join(key))
            .filter(|k| k.join("Cargo.lock").is_file());
        if let Some(from) = from
            && !to.join("Cargo.lock").is_file()
        {
            let tmp = to.with_extension(format!("tmp{}", std::process::id()));
            let copied = fs::create_dir_all(work.join("module-kit")).is_ok()
                && Command::new("cp")
                    .arg("-R")
                    .arg(&from)
                    .arg(&tmp)
                    .status()
                    .is_ok_and(|s| s.success());
            // Another gp may have made it meanwhile; theirs is the same.
            if !copied || (fs::rename(&tmp, &to).is_err() && !to.join("Cargo.lock").is_file()) {
                lines.push(format!("couldn't copy the module kit to {}", to.display()));
            }
        }
    }
    let log = work.join("warm-modules.log");
    let Ok(out) = fs::create_dir_all(&work).and_then(|()| fs::File::create(&log)) else {
        return;
    };
    let Ok(err) = out.try_clone() else { return };
    let started = Command::new("sh")
        .arg("-c")
        .arg(r#"gp=$1; shift; for d; do "$gp" game build "$d"; done"#)
        .arg("sh")
        .arg(&gp)
        .args(&games)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn();
    match started {
        Ok(_) => lines.push(format!(
            "building the modules of {} hooked games with the new gp, beside ({})",
            games.len(),
            log.display()
        )),
        Err(e) => lines.push(format!("couldn't start the module builds: {e}")),
    }
}

/// After a landing: the published build is the scored one, byte for byte,
/// or the gate says so and keeps nothing. Then it is noted as the gate's
/// in published.json, and the canary score is kept as the score of the
/// landed commit's build, under the key the next gate computes for its tip.
fn keep_landed(c: &Cx, commit: &str, scored: Option<&Scored>, lines: &mut Vec<String>) {
    let published = c.home.join("bin").join(format!("gp-{}", &commit[..12]));
    let hash = match file_hash(&published) {
        Ok(h) => h,
        Err(e) => {
            lines.push(format!("couldn't read the published build: {e}"));
            return;
        }
    };
    if let Some(s) = scored
        && s.hash != hash
    {
        lines.push(format!(
            "the published {} isn't the build the canary scored; its score isn't kept",
            published.display()
        ));
        return;
    }
    if let Err(e) = note_published(c, commit, &published) {
        lines.push(format!("couldn't note the published build: {e}"));
    }
    if let Some(s) = scored {
        let parts = KeyParts {
            gp: hash,
            ..s.parts.clone()
        };
        if let Err(e) = keep_score(&base_file(c, &canary_key(&parts)), &s.value) {
            lines.push(format!("couldn't keep the canary score: {e}"));
        }
    }
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
    let games_repo = games_dir(a, &repo)?;
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
        canary_dir: tool.join("canary"),
        home: paths.home.clone(),
        games_repo,
        games_tree: tool.join("games-tree"),
        paths: paths.clone(),
        steps: Steps::new(tool.join("steps.jsonl"), &patch_hash),
        tools: TOOLS,
    };
    let games_head = git(&c.games_repo, &["rev-parse", "HEAD"])?;
    gate::checkout(&c.games_repo, &c.games_tree, &games_head)?;
    let o = tries(&c, a, &patch, &patch_hash);
    c.steps.record(
        "gate",
        c.steps.t0.elapsed().as_secs_f64(),
        o.as_ref().is_ok_and(|o| o.exit == 0),
    );
    o
}

/// The gate's tries on the branch tip, each from a fresh tree, until one
/// lands, fails, or the tip has moved [`TIP_TRIES`] times. The tip's gp is
/// built and its canary scored while the tree is at the tip, and only then
/// is the patch applied.
fn tries(c: &Cx, a: &ToolGateArgs, patch: &Patch, patch_hash: &str) -> Result<Outcome> {
    let paths = &c.paths;
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
        let published = paths.home.join("bin").join(format!("gp-{}", &tip[..12]));
        let tip_gp = if published.is_file() && published_by_gate(c, &tip, &published) {
            lines.push(format!("tip {}: using its published build", &tip[..12]));
            published
        } else {
            if let Err(why) = c.steps.time("tip build", || build(c), Result::is_ok) {
                bail!("the tip doesn't build: {why}");
            }
            if published.is_file() {
                // A `just publish` from someone's checkout: built from what
                // that checkout held, which needn't be the tip. Left alone.
                lines.push(format!(
                    "tip {}: its published build isn't one the gate published, so the gate scores its own",
                    &tip[..12]
                ));
                snapshot(c).map_err(|e| anyhow::anyhow!(e))?.0
            } else {
                c.steps.time(
                    "tip publish",
                    || publish::publish(&c.home, &built_gp(c), &tip, &c.repo, &mut lines),
                    Result::is_ok,
                )?;
                note_published(c, &tip, &published).map_err(|e| anyhow::anyhow!(e))?;
                published
            }
        };
        let base = match c.steps.time(
            "canary (tip)",
            || tip_score(c, &tip, &tip_gp, &mut lines),
            Result::is_ok,
        ) {
            Ok(b) => b,
            Err(e) => bail!("the tip's canary score: {e}"),
        };
        let tried = c.steps.time(
            "try",
            || trial(c, a, patch, &tip, base.as_ref(), patch_hash, &mut lines),
            |r| matches!(r, Ok(Ok(_))),
        )?;
        match tried {
            Err(f) => {
                failed = Some(f);
                gate::clean(&c.tree);
                break;
            }
            Ok(Tried { commit, scored }) => {
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
                        c.steps.time(
                            "publish",
                            || {
                                // The scored copy, when the canary ran.
                                let file = scored
                                    .as_ref()
                                    .map_or_else(|| built_gp(c), |s| s.gp.clone());
                                publish::publish(&c.home, &file, &commit, &c.repo, &mut lines)
                            },
                            Result::is_ok,
                        )?;
                        keep_landed(c, &commit, scored.as_ref(), &mut lines);
                        warm_modules(c, &commit, scored.as_ref(), &mut lines);
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
            canary_dir: root.join("canary"),
            home: root.join("home"),
            games_repo: root.join("games"),
            games_tree: root.join("games-tree"),
            paths: Paths {
                home: root.join("home"),
                store: root.join("store"),
                roms: root.join("roms"),
            },
            steps: Steps::new(root.join("steps.jsonl"), "test"),
            tools: &["just", "git"],
        }
    }

    /// A FAIL line names the tests cargo says failed.
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

    /// Scoring a build leaves it and the next newest, whatever the score
    /// says, and touches nothing but older builds. `true` is the gp: it
    /// prints no JSON, so the score itself is an error.
    #[test]
    fn scoring_a_build_keeps_only_the_two_newest() {
        let dir = tempfile::tempdir().unwrap();
        let c = cx(dir.path());
        fs::create_dir(&c.canary_dir).unwrap();
        for (i, name) in ["tip-rescored", "patch-b", "patch-c", "patch-d"]
            .iter()
            .enumerate()
        {
            let home = c.canary_dir.join(name);
            fs::create_dir(&home).unwrap();
            fs::write(home.join("cache"), b"x").unwrap();
            set_time(&home, 1000 + i as u64);
        }
        let base = c.canary_dir.join("base-a-b.json");
        fs::write(&base, b"{}").unwrap();
        let other = c.canary_dir.join("other");
        fs::create_dir(&other).unwrap();
        set_time(&other, 1);
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let link = c.canary_dir.join("tip-link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        assert!(scored(&c, Path::new("true"), "tip-rescored").is_err());

        let has = |n: &str| c.canary_dir.join(n).exists();
        assert!(has("tip-rescored"), "the build just scored is the newest");
        assert!(has("patch-d"), "the next newest stays");
        assert!(!has("patch-c"), "an older build goes");
        assert!(!has("patch-b"), "an older build goes");
        assert!(base.is_file(), "a kept score isn't a build");
        assert!(other.is_dir(), "only tip-* and patch-* folders are builds");
        assert!(outside.is_dir(), "a link isn't followed");
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
        use std::os::unix::fs::PermissionsExt as _;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A tool repo and a gate around it, with stand-ins for cargo, just and
    /// gp. The repo's `crates/gp-sim/step` stands for the simulation in the
    /// module kit and `Cargo.lock` for the kit's lock. The cargo stand-in
    /// "builds" a gp that, like the real one, holds the path of the tree it
    /// was built in and finds its kit there unless GP_TOOL_SRC names
    /// another. Like the real one it scores with the kit in its home's
    /// work/module-kit/ when one is there, else copies one from the tree
    /// first; `game kit -o DIR` makes one in DIR. Its bytes don't depend on
    /// the kit's files, so two trees that differ only there build the same
    /// gp. That gp scores the one clip 10 of 10 frames together when
    /// the kit's step is `right` and 5 of 10 when not, refuses a kit whose
    /// lock isn't the one it was built with (as the module's `--locked`
    /// build did), and leaves what it read, its folder and its environment
    /// in its home. The cargo stand-in's `test` adds a line of its
    /// arguments and GP_SLOW_TESTS to `<target>/cargo-test-seen` and fails
    /// when the tree's README.md says `a failing test`, as a failing test
    /// would. The repo has one slow test, `a_slow_one`. The just stand-in
    /// leaves GP_GATE_RUNS_TESTS and GP_GATE_RUN in `<target>/just-seen`.
    fn gate_rig(root: &Path) -> Cx {
        let c = cx(root);
        fs::create_dir_all(&c.repo).unwrap();
        git_in(&c.repo, &["init", "-q", "-b", "main"]);
        git_in(&c.repo, &["config", "user.name", "gate test"]);
        git_in(&c.repo, &["config", "user.email", "gate@test"]);
        fs::create_dir_all(c.repo.join("crates/gp-sim")).unwrap();
        fs::write(c.repo.join("crates/gp-sim/step"), "right\n").unwrap();
        fs::write(c.repo.join("Cargo.lock"), "lock-a\n").unwrap();
        fs::write(c.repo.join("README.md"), "the tool\n").unwrap();
        fs::create_dir_all(c.repo.join("crates/gp-x/tests")).unwrap();
        fs::write(
            c.repo.join("crates/gp-x/tests/slow.rs"),
            "#[test]\nfn a_slow_one() {\n    if gp_arcade::skip_slow_test() {\n        return;\n    }\n}\n",
        )
        .unwrap();
        script(&c.repo.join(".githooks/pre-commit"), "#!/bin/sh\nexit 0\n");
        git_in(&c.repo, &["add", "-A"]);
        git_in(&c.repo, &["commit", "-q", "-m", "the tip"]);

        let rom = {
            let mut r = b"NES\x1a\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
            r.extend(vec![7u8; 16384 + 8192]);
            r
        };
        fs::create_dir_all(&c.paths.roms).unwrap();
        fs::write(c.paths.roms.join("dk.nes"), &rom).unwrap();
        let sha = crate::roms::Rom::of_bytes(rom).unwrap().sha256;
        let game = c.games_tree.join(CANARY);
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("package.toml"),
            format!(
                "[package]\nname = \"dk\"\n\n[[rom.entry]]\nsha256 = \"{sha}\"\nrole = \"scored\"\n"
            ),
        )
        .unwrap();
        git_in(&c.games_tree, &["init", "-q", "-b", "main"]);
        git_in(&c.games_tree, &["add", "-A"]);
        git_in(
            &c.games_tree,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "games",
            ],
        );

        let gp_body = root.join("gp-body.sh");
        let (ctl, times) = (root.join("ctl"), root.join("times"));
        fs::create_dir_all(&ctl).unwrap();
        fs::create_dir_all(&times).unwrap();
        fs::write(
            &gp_body,
            format!(
                r#"src="${{GP_TOOL_SRC:-$BUILT_IN}}"
ctl='{ctl}'
times='{times}'
if [ "$1 $2" = "game kit" ]; then
  mkdir -p "$4"; cp "$src/crates/gp-sim/step" "$src/Cargo.lock" "$4/"; echo "$4" >> "$times/kits"; exit 0
fi
home="$9"
mkdir -p "$home"
if [ "$1" = game ]; then echo "$*" >> "$times/warm"; exit 0; fi
side=tip; case "$home" in */patch-*) side=patch ;; esac
echo "$side" >> "$times/gp-runs"
if [ "$side" = patch ]; then
  echo $$ > "$times/gp.pid"
  date +%s > "$times/gp.start"
  if [ -f "$ctl/gp-fails" ]; then echo "error: the kit doesn't match" >&2; exit 1; fi
  [ -f "$ctl/gp-sleep" ] && sleep "$(cat "$ctl/gp-sleep")"
fi
kit=""
for k in "$home"/work/module-kit/*; do [ -f "$k/Cargo.lock" ] && kit="$k"; done
if [ -z "$kit" ]; then
  kit="$home/work/module-kit/own"; mkdir -p "$kit"; cp "$src/crates/gp-sim/step" "$src/Cargo.lock" "$kit/"
fi
cat "$kit/step" > "$home/kit-seen"
pwd -P > "$home/cwd-seen"
env > "$home/env-seen"
if [ "$(cat "$kit/Cargo.lock")" != "$BUILT_LOCK" ]; then
  echo "error: cannot update the lock file because --locked was passed" >&2
  exit 1
fi
if [ "$(cat "$kit/step")" = right ]; then t=10; else t=5; fi
[ "$side" = patch ] && date +%s > "$times/gp.end"
printf '{{"clips":[{{"id":"c1","frames":10,"frames_together":%s}}]}}\n' "$t"
"#,
                ctl = ctl.display(),
                times = times.display()
            ),
        )
        .unwrap();
        let locked = c.home.join("bin/locked");
        script(
            &locked.join("cargo"),
            &format!(
                r#"#!/bin/sh
if [ "$1" = test ]; then
  mkdir -p "$CARGO_TARGET_DIR"
  echo "$* | GP_SLOW_TESTS=${{GP_SLOW_TESTS:-}}" >> "$CARGO_TARGET_DIR/cargo-test-seen"
  case " $* " in *" --no-run "*) exit 0 ;; esac
  date +%s > '{times}/test.start'
  [ -f '{ctl}/test-sleep' ] && sleep "$(cat '{ctl}/test-sleep')"
  if grep -q 'a failing test' README.md; then echo 'test fails ... FAILED'; exit 101; fi
  exit 0
fi
if [ "$1" = fmt ] && grep -q 'bad format' README.md; then echo 'Diff in src/lib.rs at line 1:'; exit 1; fi
[ "$1" = build ] || exit 0
mkdir -p "$CARGO_TARGET_DIR/fastdev"
out="$CARGO_TARGET_DIR/fastdev/gp"
{{ printf '#!/bin/sh\nBUILT_IN=%s\nBUILT_LOCK=%s\n' "$(pwd -P)" "$(cat Cargo.lock)"; cat '{}'; }} > "$out.new"
chmod +x "$out.new"
mv "$out.new" "$out"
"#,
                gp_body.display(),
                ctl = ctl.display(),
                times = times.display()
            ),
        );
        // just check: `ctl/just-fails` fails it after 0.2 s, `ctl/just-sleep`
        // holds it, `ctl/just-rebuilds` builds gp again with other bytes,
        // as a check step that dirtied gp-cli did.
        script(
            &locked.join("just"),
            &format!(
                r#"#!/bin/sh
printf 'GP_GATE_RUNS_TESTS=%s\nGP_GATE_RUN=%s\n' "${{GP_GATE_RUNS_TESTS:-}}" "${{GP_GATE_RUN:-}}" > "$CARGO_TARGET_DIR/just-seen"
date +%s > '{times}/just.start'
[ -f '{ctl}/just-writes' ] && echo 'fn x() {{}}' > crates/gp-sim/gen.rs
if [ -f '{ctl}/just-flips' ]; then echo wrong > crates/gp-sim/step; sleep 2; echo right > crates/gp-sim/step; fi
if [ -f '{ctl}/just-fails' ]; then sleep 0.2; echo 'check-step: clippy 0.2 s (failed)'; exit 1; fi
[ -f '{ctl}/just-sleep' ] && sleep "$(cat '{ctl}/just-sleep')"
if [ -f '{ctl}/just-rebuilds' ]; then
  gp="$CARGO_TARGET_DIR/fastdev/gp"
  {{ cat "$gp"; echo '# built again'; }} > "$gp.new"; chmod +x "$gp.new"; mv "$gp.new" "$gp"
fi
echo 'check-step: fmt 0.4 s'
date +%s > '{times}/just.end'
"#,
                ctl = ctl.display(),
                times = times.display()
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

    /// The seconds of the step `name` in the rig's steps.jsonl, the last
    /// time it ran.
    fn step_secs(c: &Cx, name: &str) -> f64 {
        let text = fs::read_to_string(c.repo.parent().unwrap().join("steps.jsonl")).unwrap();
        text.lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .rfind(|v| v["step"] == name)
            .and_then(|v| v["secs"].as_f64())
            .unwrap_or_else(|| panic!("no step {name}: {text}"))
    }

    /// A patch to `file` in the rig's repo, written as `git diff` writes it.
    fn patch_of(c: &Cx, file: &str, text: &str) -> (Patch, String) {
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
        let hash = blake3::hash(&o.stdout).to_hex()[..12].to_owned();
        (
            Patch {
                diff: f,
                message: Some("a patch".into()),
                author: None,
                date: None,
            },
            hash,
        )
    }

    fn dry_run(c: &Cx, patch: &Patch, hash: &str) -> Outcome {
        let a = ToolGateArgs {
            patch: patch.diff.clone(),
            message: None,
            onto: Some("main".into()),
            home: None,
            games: None,
            dry_run: true,
        };
        tries(c, &a, patch, hash).unwrap()
    }

    fn seen(c: &Cx, side: &str, what: &str) -> String {
        let rd = fs::read_dir(&c.canary_dir).unwrap();
        let home = rd
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_name().unwrap().to_string_lossy().starts_with(side))
            .unwrap_or_else(|| panic!("no {side} home"));
        fs::read_to_string(home.join(what)).unwrap()
    }

    /// The tip's gp is scored with the tip's kit and the patched gp with the
    /// patched kit, so a patch that makes the simulation step wrongly scores
    /// worse than the tip and fails at the canary. Scored after the patch
    /// was applied, the tip read the wrong step too and the patch passed.
    #[test]
    fn each_side_of_the_canary_is_scored_with_its_own_kit() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let (patch, hash) = patch_of(&c, "crates/gp-sim/step", "wrong\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "FAIL", "{}", o.text);
        assert_eq!(o.json["phase"], "canary", "{}", o.text);
        assert!(
            o.text
                .contains("c1: was 100% (10/10 frames together), now 5/10"),
            "{}",
            o.text
        );
        assert_eq!(seen(&c, "tip-", "kit-seen"), "right\n");
        assert_eq!(seen(&c, "patch-", "kit-seen"), "wrong\n");
        assert_eq!(
            fs::read_to_string(c.tree.join("crates/gp-sim/step")).unwrap(),
            "right\n",
            "a refused patch leaves the tree at the tip"
        );
    }

    /// A patch that changes the kit's Cargo.lock: the tip's gp scores with
    /// the tip's lock, so its module still builds, and the patch passes.
    /// Scored after the patch was applied, the tip's gp met the patched lock
    /// and the gate stopped.
    #[test]
    fn a_patch_to_the_lock_leaves_the_tips_score_alone() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let (patch, hash) = patch_of(&c, "Cargo.lock", "lock-b\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(
            o.text.contains("canary: 1 clips of donkey-kong held"),
            "{}",
            o.text
        );
    }

    /// Each side is scored in its own home with the kept environment alone:
    /// nothing else of the gate's environment reaches it, and GP_HOME,
    /// GP_ROM_DIR and GP_TOOL_SRC are the gate's. A second patch on the same
    /// tip reads the tip's kept score.
    #[test]
    fn a_canary_score_runs_in_its_home_with_the_kept_environment() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        for side in ["tip-", "patch-"] {
            let cwd = seen(&c, side, "cwd-seen");
            assert!(
                cwd.contains(&format!(
                    "canary/{side}{}-",
                    if side == "tip-" {
                        o.json["tip"].as_str().unwrap()[..12].to_owned()
                    } else {
                        hash.clone()
                    }
                )),
                "{side} ran in {cwd}"
            );
            let env = seen(&c, side, "env-seen");
            let mut names = Vec::new();
            for line in env.lines() {
                let Some((k, v)) = line.split_once('=') else {
                    continue;
                };
                names.push(k.to_owned());
                match k {
                    "GP_TOOL_SRC" => assert_eq!(v, c.tree.to_str().unwrap()),
                    "GP_ROM_DIR" => assert_eq!(v, c.paths.roms.to_str().unwrap()),
                    "GP_HOME" => assert!(v.contains("/canary/"), "{v}"),
                    // What the shell sets of itself.
                    "PWD" | "OLDPWD" | "SHLVL" | "_" => {}
                    k => assert!(SCORE_ENV.contains(&k), "{side} got {k}"),
                }
            }
            for k in ["GP_TOOL_SRC", "GP_ROM_DIR", "GP_HOME"] {
                assert!(names.iter().any(|n| n == k), "{side} has no {k}");
            }
        }
        let tip = o.json["tip"].as_str().unwrap();
        let kept = fs::read_dir(&c.canary_dir).unwrap().flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&format!("base{BASE_FORMAT}-"))
        });
        assert!(kept, "the tip's score is kept");
        // The tip's gp gone: a second patch on this tip uses the kept score.
        let tip_homes = || -> Vec<PathBuf> {
            fs::read_dir(&c.canary_dir)
                .unwrap()
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(&format!("tip-{}-", &tip[..12]))
                })
                .map(|e| e.path())
                .collect()
        };
        for h in tip_homes() {
            fs::remove_dir_all(h).unwrap();
        }
        let (patch, hash) = patch_of(&c, "README.md", "the tool, once more\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(tip_homes().is_empty());
    }

    /// The gate's own test run is the whole workspace in the gate's
    /// profile with the slow tests on (GP_SLOW_TESTS=1): built once, then
    /// the slow test in a run of its own beside the rest, which skips it.
    /// A failing test there fails the gate.
    #[test]
    fn the_gates_test_run_has_every_test_and_a_failure_stops_it() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
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

        let (patch, hash) = patch_of(&c, "README.md", "a failing test\n");
        let o = dry_run(&c, &patch, &hash);
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
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        let text = fs::read_to_string(dir.path().join("steps.jsonl")).unwrap();
        let steps: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for want in [
            "checkout",
            "tip build",
            "canary (tip)",
            "apply",
            "hook",
            "build",
            "lane check",
            "check/fmt",
            "test compile",
            "tests/rest",
            "tests/a_slow_one",
            "lane canary",
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
    /// or tested; a tool missing from PATH fails before the tip is scored.
    #[test]
    fn fmt_and_the_tools_fail_first() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let (patch, hash) = patch_of(&c, "README.md", "bad format\n");
        let o = dry_run(&c, &patch, &hash);
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
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
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

    fn landing(c: &Cx, patch: &Patch, hash: &str) -> Outcome {
        let a = ToolGateArgs {
            patch: patch.diff.clone(),
            message: None,
            onto: Some("main".into()),
            home: None,
            games: None,
            dry_run: false,
        };
        tries(c, &a, patch, hash).unwrap()
    }

    fn runs_of(c: &Cx, side: &str) -> usize {
        stamp(c, "gp-runs")
            .unwrap_or_default()
            .lines()
            .filter(|l| *l == side)
            .count()
    }

    /// just check, the tests and the patched canary start together: each
    /// takes 2 s, and the three take about 2 s, not 6.
    #[test]
    fn lanes_run_at_the_same_time() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        for s in ["just-sleep", "test-sleep", "gp-sleep"] {
            switch(&c, s, "3");
        }
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        let lanes = step_secs(&c, "lanes");
        // One after another they take 9 s; at once, 3 and the machine's
        // load.
        assert!(lanes < 7.5, "the lanes took {lanes} s");
        for n in ["just.start", "test.start", "gp.start"] {
            assert!(stamp(&c, n).is_some(), "{n}: the lane didn't run");
        }
    }

    /// A lane that fails stops the others at once: just check fails after
    /// 0.2 s while the canary would take 10.
    #[test]
    fn the_first_failing_lane_stops_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        switch(&c, "just-fails", "");
        switch(&c, "gp-sleep", "30");
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "FAIL", "{}", o.text);
        assert_eq!(o.json["phase"], "just check", "{}", o.text);
        assert!(step_secs(&c, "lanes") < 15.0, "the canary would take 30 s");
        assert!(stamp(&c, "gp.end").is_none(), "the canary was stopped");
        let pid = stamp(&c, "gp.pid").unwrap();
        let alive = Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap()
            .success();
        assert!(!alive, "the canary's gp {pid} is gone");
    }

    /// A canary that fails stops just check.
    #[test]
    fn a_failing_canary_stops_check_early() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        switch(&c, "gp-fails", "");
        switch(&c, "just-sleep", "30");
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["phase"], "canary", "{}", o.text);
        assert!(o.text.contains("the kit doesn't match"), "{}", o.text);
        assert!(step_secs(&c, "lanes") < 15.0, "just check would take 30 s");
        assert!(stamp(&c, "just.end").is_none(), "just check was stopped");
    }

    /// gp built again during the check: the canary scored the first build,
    /// so it scores the build that will be published, again.
    #[test]
    fn the_published_gp_is_the_one_scored() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        switch(&c, "just-rebuilds", "");
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(
            o.text
                .contains("gp changed during the checks, scored again"),
            "{}",
            o.text
        );
        assert_eq!(runs_of(&c, "patch"), 2);
    }

    /// What the newest score home of `side` holds in `what`.
    fn newest_seen(c: &Cx, side: &str, what: &str) -> String {
        let home = fs::read_dir(&c.canary_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(side))
            .max_by_key(|e| e.metadata().unwrap().modified().unwrap())
            .unwrap_or_else(|| panic!("no {side} home"))
            .path();
        fs::read_to_string(home.join(what)).unwrap()
    }

    /// The same patch again after the tip moved in kit-only code: gp's
    /// bytes are the same, but the patched canary reads the new tree's kit,
    /// not the kit an earlier score of the patch left in its home.
    #[test]
    fn a_score_never_reads_an_older_trees_kit() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!(newest_seen(&c, "patch-", "kit-seen"), "right\n");
        // The tip moves: only the kit's code changes, so gp is the same.
        fs::write(c.repo.join("crates/gp-sim/step"), "wrong\n").unwrap();
        git_in(&c.repo, &["commit", "-qam", "kit-only change"]);
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!(
            newest_seen(&c, "patch-", "kit-seen"),
            "wrong\n",
            "the patched canary read the old tree's kit: {}",
            o.text
        );
    }

    /// A check that leaves a file in the tree: what would land isn't what
    /// the canary scored, so the gate refuses and names the file.
    #[test]
    fn a_check_that_changes_the_tree_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        switch(&c, "just-writes", "");
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "FAIL", "{}", o.text);
        assert_eq!(o.json["phase"], "tree", "{}", o.text);
        assert!(o.text.contains("crates/gp-sim/gen.rs"), "{}", o.text);
        assert_eq!(runs_of(&c, "patch"), 1, "nothing is scored again");
    }

    /// The canary's kit is made before the lanes start: a check that
    /// writes a kit file and puts it back while the canary runs doesn't
    /// reach the score.
    #[test]
    fn the_kit_is_made_before_the_checks_run() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        switch(&c, "just-flips", "");
        switch(&c, "gp-sleep", "1");
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!(newest_seen(&c, "patch-", "kit-seen"), "right\n");
    }

    /// A landing keeps the patched gp's score as the score of the commit it
    /// landed, so the next gate on that tip doesn't score the tip; and it
    /// records what it published, so that build is trusted as the tip's.
    #[test]
    fn the_tip_score_is_kept_for_the_next_tip() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = landing(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert_eq!((runs_of(&c, "tip"), runs_of(&c, "patch")), (1, 1));
        let (patch, hash) = patch_of(&c, "README.md", "the tool, once more\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(o.text.contains("using its published build"), "{}", o.text);
        assert!(
            o.text.contains("canary: the tip's score is kept"),
            "{}",
            o.text
        );
        assert_eq!(
            (runs_of(&c, "tip"), runs_of(&c, "patch")),
            (1, 2),
            "the second gate scored only its patch"
        );
        // Verify mode scores the tip anyway and finds the kept score right.
        let (patch, hash) = patch_of(&c, "README.md", "the tool, a third time\n");
        let o = tries_verifying(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(
            o.text.contains("canary: the kept score of the tip agrees"),
            "{}",
            o.text
        );
        assert_eq!(runs_of(&c, "tip"), 2);
    }

    /// A landing starts the hooked games' module builds with the new gp,
    /// and only those games'.
    #[test]
    fn a_landing_builds_the_hooked_games_modules() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        for (g, hooks) in [("dk", true), ("plain", false)] {
            let d = c.games_repo.join(g);
            fs::create_dir_all(&d).unwrap();
            let extra = if hooks {
                "\n[hooks]\nstep = \"ext\"\n"
            } else {
                ""
            };
            fs::write(
                d.join("package.toml"),
                format!("[package]\nname = \"{g}\"\n{extra}"),
            )
            .unwrap();
        }
        assert_eq!(hooked_games(&c.games_repo), [c.games_repo.join("dk")]);
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = landing(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(
            o.text.contains("building the modules of 1 hooked games"),
            "{}",
            o.text
        );
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while stamp(&c, "warm").is_none() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100));
        }
        let warm = stamp(&c, "warm").unwrap();
        assert_eq!(
            warm.trim(),
            format!("game build {}", c.games_repo.join("dk").display())
        );
    }

    fn tries_verifying(c: &Cx, patch: &Patch, hash: &str) -> Outcome {
        let a = ToolGateArgs {
            patch: patch.diff.clone(),
            message: None,
            onto: Some("main".into()),
            home: None,
            games: None,
            dry_run: true,
        };
        VERIFY_IN_TEST.with(|v| v.set(true));
        let o = tries(c, &a, patch, hash).unwrap();
        VERIFY_IN_TEST.with(|v| v.set(false));
        o
    }

    /// A gp in the published folder that the gate didn't publish (a `just
    /// publish` from someone's checkout) isn't scored as the tip: the gate
    /// builds the tip itself, and leaves that file alone.
    #[test]
    fn a_tip_build_the_gate_didnt_publish_is_built_again() {
        let dir = tempfile::tempdir().unwrap();
        let c = gate_rig(dir.path());
        let tip = git(&c.repo, &["rev-parse", "main"]).unwrap();
        let hand = c.home.join("bin").join(format!("gp-{}", &tip[..12]));
        script(&hand, "#!/bin/sh\nexit 7\n");
        let (patch, hash) = patch_of(&c, "README.md", "the tool, said again\n");
        let o = dry_run(&c, &patch, &hash);
        assert_eq!(o.json["verdict"], "PASS", "{}", o.text);
        assert!(
            o.text.contains("isn't one the gate published"),
            "{}",
            o.text
        );
        assert_eq!(fs::read_to_string(&hand).unwrap(), "#!/bin/sh\nexit 7\n");
    }

    /// The kept score's key changes with each thing a score depends on.
    #[test]
    fn the_canary_key_covers_each_part() {
        let parts = KeyParts {
            gp: "g".into(),
            tree: "t".into(),
            games: "m".into(),
            rom: "r".into(),
            toolchain: "rustc 1".into(),
            env: vec![("PATH".into(), "/bin".into())],
            args: "--json model gate".into(),
        };
        let k = canary_key(&parts);
        assert_eq!(k, canary_key(&parts.clone()));
        assert_eq!(k.len(), 24);
        let changed: Vec<KeyParts> = vec![
            KeyParts {
                gp: "g2".into(),
                ..parts.clone()
            },
            KeyParts {
                tree: "t2".into(),
                ..parts.clone()
            },
            KeyParts {
                games: "m2".into(),
                ..parts.clone()
            },
            KeyParts {
                rom: "r2".into(),
                ..parts.clone()
            },
            KeyParts {
                toolchain: "rustc 2".into(),
                ..parts.clone()
            },
            KeyParts {
                env: vec![("PATH".into(), "/usr/bin".into())],
                ..parts.clone()
            },
            KeyParts {
                args: "--json model gate x".into(),
                ..parts.clone()
            },
        ];
        for p in changed {
            assert_ne!(canary_key(&p), k);
        }
        // Parts don't run together: "ab" + "c" isn't "a" + "bc".
        let a = KeyParts {
            gp: "ab".into(),
            tree: "c".into(),
            ..parts.clone()
        };
        let b = KeyParts {
            gp: "a".into(),
            tree: "bc".into(),
            ..parts.clone()
        };
        assert_ne!(canary_key(&a), canary_key(&b));
    }

    /// The tree's state is its index's tree with what differs from it: a
    /// file changed, added or ignored under crates/ is in it, target/ isn't.
    #[test]
    fn the_tree_state_sees_changed_and_ignored_files() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path();
        git_in(t, &["init", "-q"]);
        fs::create_dir_all(t.join("crates/a/target")).unwrap();
        fs::write(t.join("crates/a/lib.rs"), "1").unwrap();
        fs::write(t.join(".gitignore"), "*.gen\n").unwrap();
        git_in(t, &["add", "-A"]);
        let s0 = tree_state(t).unwrap();
        fs::write(t.join("crates/a/target/out"), "x").unwrap();
        assert_eq!(tree_state(t).unwrap(), s0, "target/ isn't the tree's");
        fs::write(t.join("crates/a/x.gen"), "x").unwrap();
        let s1 = tree_state(t).unwrap();
        assert_ne!(s1, s0, "an ignored file under crates/ counts");
        fs::write(t.join("crates/a/x.gen"), "y").unwrap();
        assert_ne!(tree_state(t).unwrap(), s1, "and its bytes");
        fs::write(t.join("crates/a/lib.rs"), "2").unwrap();
        assert_ne!(
            tree_state(t).unwrap(),
            s1,
            "a change not in the index counts"
        );
    }

    /// A source folder named target inside a crate is copied into the kit
    /// (module.rs, `walk`), so it is in the tree's state; a crate's own
    /// target/ isn't.
    #[test]
    fn a_source_folder_named_target_is_in_the_tree_state() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path();
        git_in(t, &["init", "-q"]);
        fs::create_dir_all(t.join("crates/a/src/target")).unwrap();
        fs::create_dir_all(t.join("crates/a/target")).unwrap();
        fs::write(t.join("crates/a/Cargo.toml"), "[package]\n").unwrap();
        git_in(t, &["add", "-A"]);
        let s0 = tree_state(t).unwrap();
        fs::write(t.join("crates/a/target/out"), "x").unwrap();
        assert_eq!(tree_state(t).unwrap(), s0, "a crate's build output");
        fs::write(t.join("crates/a/src/target/mod.rs"), "x").unwrap();
        assert_ne!(tree_state(t).unwrap(), s0, "source under src/target");
    }

    /// The games tree's state holds the bytes of the files git ignores
    /// there, not only their names.
    #[test]
    fn the_games_state_sees_an_ignored_files_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path();
        git_in(g, &["init", "-q"]);
        fs::write(g.join(".gitignore"), "*.gen\n").unwrap();
        git_in(g, &["add", "-A"]);
        git_in(
            g,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "g",
            ],
        );
        fs::write(g.join("x.gen"), "1").unwrap();
        let s1 = games_state(g).unwrap();
        fs::write(g.join("x.gen"), "2").unwrap();
        assert_ne!(games_state(g).unwrap(), s1);
    }

    /// `rustc -vV` for the key runs with the score's environment alone, as
    /// the score does: a variable of the gate's (cargo test sets
    /// CARGO_MANIFEST_DIR) doesn't reach it. The cargo settings in
    /// CARGO_HOME are in the key.
    #[test]
    fn the_toolchain_part_is_read_as_the_score_reads_it() {
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        let o = kept_env(&mut Command::new("env")).output().unwrap();
        let env = String::from_utf8_lossy(&o.stdout);
        assert!(!env.contains("CARGO_MANIFEST_DIR="), "{env}");
        let dir = tempfile::tempdir().unwrap();
        let none = cargo_config(dir.path());
        fs::write(
            dir.path().join("config.toml"),
            "[build]\nrustflags = [\"-g\"]\n",
        )
        .unwrap();
        assert_ne!(cargo_config(dir.path()), none);
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
