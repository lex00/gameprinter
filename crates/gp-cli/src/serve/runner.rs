//! gp serve's runner (docs/loop.md, "The runner"; issue #43): the loop
//! starts its own workers, so no agent controls it at the top. Every few
//! seconds, while the home's `loop.toml` has the runner on and the loop
//! isn't halted, it reads the open steps (crate::steps, as `gp loop steps`
//! gives them) and starts a worker on each open step of the agent's that
//! no worker has had, that touches nothing a live worker's step touches,
//! up to the owner's `max_workers`.
//!
//! A worker is the clip agent's command (`gp serve --clip-agent CMD`, else
//! `$GP_CLIP_AGENT`; serve/clip_agent.rs), started in a git worktree of its
//! own (`$GP_HOME/work/loop/trees/ID`, branch `loop/ID`, from main), with a
//! brief built from the step on stdin (and `gp agent brief`'s, when the
//! step is about a clip) and `GP_STEP` set to its step's id, so its gp_run
//! calls cite that step and no other. A `claude` command gets gp's MCP
//! server with `GP_STEP` set and its tools scoped to the worktree.
//!
//! Its briefs, logs and MCP setup are in `$GP_HOME/work/loop/files/ID/`.
//! Its record is `$GP_HOME/work/loop/workers/ID.json` (gp_mcp::steps): the
//! step, its state (`working`, `attempting`, `waiting for a yes`,
//! `retrying`, `done`, `stopped`), why, its worktree, log, command and last
//! line. A worker that ends with an error is started again with the error
//! in its brief, up to [`TRIES`] tries. Its results go back through the
//! gates, and the engine recomputes the steps from the files after each
//! landing, so the runner's next look sees them. A halt (crate::wrong)
//! stops new starts; workers already running finish their step.
//!
//! | Endpoint | What |
//! |---|---|
//! | `POST /api/loop/workers/{id}/stop` | Stops one worker and what it started |

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use serde_json::{Value, json};

use super::clip_agent::{self, Mode};
use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};
use gp_mcp::steps as loop_state;

pub(super) fn routes() -> Vec<Route> {
    vec![Route {
        path: "/api/loop/workers/{id}/stop",
        about: "POST: stops one of the loop runner's workers",
        handler: stop,
    }]
}

/// How many times a worker that ends with an error is started.
pub(crate) const TRIES: u32 = 2;
/// How often the runner looks at the steps.
const LOOK: Duration = Duration::from_secs(5);

/// What a worker needs besides its step.
#[derive(Clone)]
pub(crate) struct Setup {
    pub home: PathBuf,
    pub games: PathBuf,
    /// The repo its worktree is made from.
    pub repo: PathBuf,
    pub gp: PathBuf,
    pub argv: Vec<String>,
    pub program: PathBuf,
    pub timeout: Duration,
    pub tries: u32,
    /// How long before a try after one that failed.
    pub again_after: Duration,
}

/// The live workers: a step id and its stop flag.
static RUNNING: Mutex<BTreeMap<String, Arc<AtomicBool>>> = Mutex::new(BTreeMap::new());

fn running_ids() -> BTreeSet<String> {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect()
}

/// A worker's own files: its briefs, logs and MCP setup.
fn files_dir(home: &Path, id: &str) -> PathBuf {
    loop_state::loop_dir(home).join("files").join(id)
}

fn file(home: &Path, id: &str) -> PathBuf {
    loop_state::workers_dir(home).join(format!("{id}.json"))
}

fn write(home: &Path, v: &Value) {
    let id = v["step"]["id"].as_str().unwrap_or("");
    if id.is_empty() {
        return;
    }
    let f = file(home, id);
    let _ = fs::create_dir_all(loop_state::workers_dir(home));
    let tmp = f.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &f);
    }
}

fn id_ok(id: &str) -> bool {
    id.len() == 13 && id.starts_with('s') && id[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// The steps to start now: the agent's open steps, oldest rule first,
/// that no worker has had (whatever became of it: a step done or stopped
/// isn't started again while the facts it was decided from hold) and that
/// touch nothing a live worker's step touches, up to `max` live at once.
pub(crate) fn plan(
    open: &[Value],
    workers: &[Value],
    live: &BTreeSet<String>,
    max: usize,
) -> Vec<Value> {
    let had: BTreeSet<&str> = workers
        .iter()
        .filter_map(|w| w["step"]["id"].as_str())
        .collect();
    let is_live = |w: &&Value| {
        w["step"]["id"].as_str().is_some_and(|id| live.contains(id)) || loop_state::holds(w)
    };
    let mut taken: BTreeSet<String> = workers
        .iter()
        .filter(is_live)
        .flat_map(|w| w["step"]["touches"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    let mut n = workers.iter().filter(is_live).count();
    let mut out = Vec::new();
    for s in open {
        if n >= max {
            break;
        }
        let Some(id) = s["id"].as_str() else { continue };
        if s["who"] != json!("agent") || had.contains(id) || !id_ok(id) {
            continue;
        }
        let touches: Vec<String> = s["touches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect();
        if touches.iter().any(|t| taken.contains(t)) {
            continue;
        }
        taken.extend(touches);
        n += 1;
        out.push(s.clone());
    }
    out
}

/// The runner in words, for the page.
pub(crate) fn status(ctx: &Ctx) -> Value {
    let s = loop_state::settings(&ctx.paths.home);
    let mode = clip_agent::Settings::current(ctx).mode;
    let live = loop_state::workers(&ctx.paths.home)
        .iter()
        .filter(|w| loop_state::holds(w))
        .count();
    let says = if !s.runner.on {
        "The runner is off: nothing starts a worker. gp loop set --runner on turns it on."
            .to_owned()
    } else if mode == Mode::Off {
        format!(
            "The runner is on, but no agent command is set, so nothing starts. {}",
            clip_agent::OFF
        )
    } else if crate::wrong::halt(&ctx.paths.home).is_some() {
        format!(
            "The runner is on and the loop is halted: no new worker starts. {live} still at work."
        )
    } else {
        format!(
            "The runner is on: {live} of at most {} workers at work.",
            s.runner.max_workers
        )
    };
    json!({
        "on": s.runner.on,
        "max_workers": s.runner.max_workers,
        "spending": s.runner.spending,
        "agent": matches!(mode, Mode::Command(_)),
        "says": says,
    })
}

/// Starts the runner's look, once per server.
pub(crate) fn start(ctx: Arc<Ctx>) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    // A record left live by a server that stopped: its process ended
    // unwatched, or runs on with no one watching it.
    for mut w in loop_state::workers(&ctx.paths.home) {
        if loop_state::LIVE.contains(&w["state"].as_str().unwrap_or(""))
            && !w["pid"].as_u64().is_some_and(gp_mcp::proc::alive)
        {
            w["state"] = json!("stopped");
            w["why"] = json!("it ended while gp serve wasn't running");
            write(&ctx.paths.home, &w);
        }
    }
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(LOOK);
            look(&ctx);
        }
    });
}

/// One look: start what the plan says, when the runner is on, an agent
/// command is set and the loop isn't halted.
fn look(ctx: &Ctx) {
    let home = &ctx.paths.home;
    let s = loop_state::settings(home);
    if !s.runner.on || crate::wrong::halt(home).is_some() {
        return;
    }
    let cs = clip_agent::Settings::current(ctx);
    let Mode::Command(argv) = cs.mode.clone() else {
        return;
    };
    let Some(program) = argv.first().and_then(|p| clip_agent::find_program(p)) else {
        return;
    };
    let v = crate::loop_cmd::steps_json(&ctx.paths, Some(&ctx.games));
    let open = v["open"].as_array().cloned().unwrap_or_default();
    let todo = plan(
        &open,
        &loop_state::workers(home),
        &running_ids(),
        s.runner.max_workers as usize,
    );
    let setup = Setup {
        home: home.clone(),
        games: ctx.games.clone(),
        repo: cs.repo.clone(),
        gp: cs.gp.clone(),
        argv,
        program,
        timeout: cs.timeout,
        tries: TRIES,
        again_after: Duration::from_secs(10),
    };
    for step in todo {
        spawn(setup.clone(), step);
    }
}

/// Starts a worker on `step` in a thread of its own.
pub(crate) fn spawn(setup: Setup, step: Value) -> std::thread::JoinHandle<()> {
    let id = step["id"].as_str().unwrap_or("").to_owned();
    let flag = Arc::new(AtomicBool::new(false));
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), flag.clone());
    let mut rec = json!({
        "step": step,
        "state": "working",
        "why": "",
        "worker": format!("loop-{id}"),
        "started_ms": gp_mcp::asks::now_ms(),
        "tries": 0,
    });
    write(&setup.home, &rec);
    std::thread::spawn(move || {
        if let Err(e) = work(&setup, &flag, &mut rec) {
            rec["state"] = json!("stopped");
            rec["why"] = json!(format!("it couldn't start: {e:#}"));
        }
        rec["pid"] = Value::Null;
        write(&setup.home, &rec);
        RUNNING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    })
}

/// The worker's worktree, made from main the first time.
fn worktree(setup: &Setup, id: &str) -> anyhow::Result<(PathBuf, String)> {
    let repo = &setup.repo;
    if !repo.join(".git").exists() {
        bail!(
            "{} isn't a git checkout, so there's nothing to make a worktree of (GP_AGENT_REPO names one)",
            repo.display()
        );
    }
    let path = loop_state::loop_dir(&setup.home).join("trees").join(id);
    let branch = format!("loop/{id}");
    if path.join(".git").exists() {
        return Ok((path, branch));
    }
    fs::create_dir_all(path.parent().unwrap_or(&path))?;
    let p = path.to_string_lossy().into_owned();
    let has_branch = super::work::git_ok(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok();
    if has_branch {
        super::work::git_ok(repo, &["worktree", "add", "-q", &p, &branch])?;
    } else {
        super::work::git_ok(repo, &["worktree", "add", "-q", "-b", &branch, &p, "main"])?;
    }
    Ok((path, branch))
}

/// The worker's brief: the step, the rules, and the clip's brief when the
/// step is about a clip.
pub(crate) fn brief(setup: &Setup, step: &Value, wt: &Path, branch: &str, before: &str) -> String {
    let id = step["id"].as_str().unwrap_or("?");
    let list = |k: &str| {
        step[k]
            .as_array()
            .map(|c| {
                c.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "none".into())
    };
    let gp = setup.gp.display();
    let mut b = vec![
        "You are a worker in Game Printer's loop (docs/loop.md, docs/workers.md). The loop gave you one step. Do that step and nothing else, then stop.".to_owned(),
        String::new(),
        format!("Step {id}, for {}: {}", step["subject"].as_str().unwrap_or("the home"), step["says"].as_str().unwrap_or("")),
        format!("Its command: {}", step["how"]["cli"].as_str().unwrap_or("none given")),
        format!("The gp commands it runs: {}", list("commands")),
        format!("It touches: {}", list("touches")),
        String::new(),
        format!("Your worktree is {} (branch {branch}). Start each shell there with: eval \"$({gp} worker env loop-{id} --shell)\" (add --rust when you change Rust).", wt.display()),
        format!("Every gp_run call that writes, lands or publishes is this step: gp_run cites {id} for you and refuses any other step."),
        "A change lands only through a gate: gp model gate for a game, gp tool gate for the tool. A landing waits for the person's yes on their page; ask and wait, never work around it.".to_owned(),
        "When the step is done, or you can't do it, stop with a short report: what you did, what you checked, and what is left.".to_owned(),
    ];
    if !before.is_empty() {
        b.push(String::new());
        b.push(before.to_owned());
    }
    if let Some((run, clip)) = clip_of(step) {
        let out = Command::new(&setup.gp)
            .args([
                "agent",
                "brief",
                "--run",
                &run,
                "--clip",
                &clip,
                "--no-curve",
            ])
            .env("GP_HOME", &setup.home)
            .env("GP_GAMES_DIR", &setup.games)
            .env("GP_NO_HINTS", "1")
            .stdin(Stdio::null())
            .output();
        b.push(String::new());
        match out {
            Ok(o) if o.status.success() => {
                b.push(format!("gp agent brief --run {run} --clip {clip}:"));
                b.push(String::from_utf8_lossy(&o.stdout).trim().to_owned());
            }
            _ => b.push(format!(
                "The step is about clip {clip}: start with gp agent brief --run {run} --clip {clip}."
            )),
        }
    }
    b.join("\n") + "\n"
}

/// The run and clip of a step whose command starts with `gp agent brief
/// --run RUN --clip CLIP`.
fn clip_of(step: &Value) -> Option<(String, String)> {
    let cli = step["how"]["cli"].as_str()?;
    let w: Vec<&str> = cli
        .split_whitespace()
        .map(|w| w.trim_end_matches(','))
        .collect();
    let at = w.windows(3).position(|x| x == ["gp", "agent", "brief"])?;
    let after = &w[at + 3..];
    let val = |flag: &str| {
        after
            .iter()
            .position(|x| *x == flag)
            .and_then(|i| after.get(i + 1))
            .map(|s| s.to_string())
    };
    Some((val("--run")?, val("--clip")?))
}

/// The arguments a `claude` worker gets after its own: print mode, gp's
/// MCP server with its step, and its tools scoped to its worktree.
fn claude_args(setup: &Setup, wt: &Path, mcp: &Path) -> Vec<String> {
    let w = wt.display();
    let g = setup.gp.display();
    let mut a: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--permission-mode",
        "dontAsk",
        "--strict-mcp-config",
        "--mcp-config",
    ]
    .map(String::from)
    .to_vec();
    a.push(mcp.display().to_string());
    a.push("--allowedTools".into());
    for t in [
        "Read".to_owned(),
        "Glob".to_owned(),
        "Grep".to_owned(),
        format!("Edit(/{w}/**)"),
        format!("Write(/{w}/**)"),
        "mcp__gp__next".to_owned(),
        "mcp__gp__gp_commands".to_owned(),
        "mcp__gp__gp_run".to_owned(),
        format!("Bash({g} worker env:*)"),
        "Bash(cargo build:*)".to_owned(),
        "Bash(cargo test:*)".to_owned(),
        "Bash(cargo fmt:*)".to_owned(),
        "Bash(cargo clippy:*)".to_owned(),
        "Bash(git status:*)".to_owned(),
        "Bash(git diff:*)".to_owned(),
        "Bash(git log:*)".to_owned(),
        "Bash(git add:*)".to_owned(),
        "Bash(git commit:*)".to_owned(),
    ] {
        a.push(t);
    }
    a.push("--disallowedTools".into());
    for t in [
        "Bash(rm:*)",
        "Bash(git push:*)",
        "Bash(git clean:*)",
        "Bash(git commit --no-verify:*)",
        "Bash(git reset --hard:*)",
        "Bash(git stash:*)",
        "WebFetch",
        "WebSearch",
    ] {
        a.push(t.to_owned());
    }
    a
}

/// The worker's state while its process runs: waiting for a yes when an
/// open ask is its step's, attempting while its `gp agent attempt` runs,
/// else working.
fn state_now(home: &Path, id: &str) -> &'static str {
    let mine = |v: &Value| v["step"] == json!(id);
    if gp_mcp::asks::pending(home)
        .iter()
        .any(|a| mine(a) && a["state"] == json!("open"))
    {
        "waiting for a yes"
    } else if gp_mcp::activity::running(home)
        .iter()
        .any(|r| mine(r) && r["command"] == json!("agent attempt"))
    {
        "attempting"
    } else {
        "working"
    }
}

/// The last line of a log with words in it.
fn last_line(log: &Path) -> Option<String> {
    let t = fs::read_to_string(log).ok()?;
    let tail = t.lines().rev().find(|l| !l.trim().is_empty())?;
    Some(crate::next::cut(tail, 160))
}

/// Runs the worker's tries until one ends well, it is stopped, or the
/// tries or the time are up.
fn work(setup: &Setup, flag: &AtomicBool, rec: &mut Value) -> anyhow::Result<()> {
    let step = rec["step"].clone();
    let id = step["id"].as_str().unwrap_or("").to_owned();
    let (wt, branch) = worktree(setup, &id)?;
    rec["worktree"] = json!(wt);
    rec["branch"] = json!(branch);
    let dir = files_dir(&setup.home, &id);
    fs::create_dir_all(&dir)?;
    let mcp = dir.join("mcp.json");
    let mut env = json!({"GP_HOME": setup.home, "GP_GAMES_DIR": setup.games});
    env[loop_state::STEP_ENV] = json!(id);
    fs::write(
        &mcp,
        serde_json::to_vec_pretty(&json!({"mcpServers": {"gp": {
            "command": setup.gp, "args": ["mcp"], "env": env,
        }}}))?,
    )?;
    let t0 = Instant::now();
    let mut before = String::new();
    for n in 1..=setup.tries.max(1) {
        rec["tries"] = json!(n);
        let brief_path = dir.join(format!("brief.{n}.txt"));
        let log_path = dir.join(format!("{n}.log"));
        fs::write(&brief_path, brief(setup, &step, &wt, &branch, &before))?;
        let log = File::create(&log_path)?;
        let mut cmd = Command::new(&setup.program);
        let mut shown = vec![setup.program.display().to_string()];
        cmd.args(&setup.argv[1..]);
        shown.extend(setup.argv[1..].iter().cloned());
        if clip_agent::is_claude(setup.argv.first().map(String::as_str).unwrap_or("")) {
            let extra = claude_args(setup, &wt, &mcp);
            cmd.args(&extra);
            shown.extend(extra);
        }
        rec["command"] = json!(clip_agent::command_line(&shown));
        rec["log"] = json!(log_path);
        rec["brief"] = json!(brief_path);
        cmd.env("GP_HOME", &setup.home)
            .env("GP_GAMES_DIR", &setup.games)
            .env(loop_state::STEP_ENV, &id)
            .env("GP_WORKER", format!("loop-{id}"))
            .env("GP_LOOP_BRIEF", &brief_path)
            .env("GP_LOOP_WORKTREE", &wt)
            .env("GP_GP", &setup.gp)
            .env("GP_NO_WINDOW", "1")
            .current_dir(&wt)
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .stdin(Stdio::piped());
        gp_mcp::proc::own_group(&mut cmd);
        let mut child = cmd.spawn().context("starting the worker")?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&fs::read(&brief_path).unwrap_or_default());
        }
        rec["pid"] = json!(child.id());
        rec["state"] = json!("working");
        write(&setup.home, rec);
        let end = |child: &mut std::process::Child| {
            gp_mcp::proc::stop_group(child.id(), Duration::from_secs(2));
            let _ = child.kill();
            let _ = child.wait();
        };
        let code = loop {
            if let Some(s) = child.try_wait()? {
                break s.code();
            }
            if flag.load(Ordering::Relaxed) {
                end(&mut child);
                rec["state"] = json!("stopped");
                rec["why"] = json!("you stopped it");
                rec["last"] = json!(last_line(&log_path));
                return Ok(());
            }
            if t0.elapsed() >= setup.timeout {
                end(&mut child);
                rec["state"] = json!("stopped");
                rec["why"] = json!(format!(
                    "its time ran out ({} minutes)",
                    setup.timeout.as_secs() / 60
                ));
                return Ok(());
            }
            let now = state_now(&setup.home, &id);
            let last = last_line(&log_path);
            if rec["state"] != json!(now) || rec["last"] != json!(last) {
                rec["state"] = json!(now);
                rec["last"] = json!(last);
                write(&setup.home, rec);
            }
            std::thread::sleep(Duration::from_millis(250));
        };
        rec["last"] = json!(last_line(&log_path));
        if code == Some(0) {
            rec["state"] = json!("done");
            rec["why"] = json!("it ended its step");
            return Ok(());
        }
        let said = match code {
            Some(c) => format!("exit {c}"),
            None => "a signal".to_owned(),
        };
        if n >= setup.tries.max(1) {
            rec["state"] = json!("stopped");
            rec["why"] = json!(format!("it failed {n} times; the last ended with {said}"));
            return Ok(());
        }
        rec["state"] = json!("retrying");
        rec["why"] = json!(format!("try {n} ended with {said}"));
        write(&setup.home, rec);
        before = format!(
            "Your try {n} at this step ended with {said}. Its last line: {}. Its log is {}.",
            last_line(&log_path).unwrap_or_else(|| "none".into()),
            log_path.display()
        );
        let wait_until = Instant::now() + setup.again_after;
        while Instant::now() < wait_until {
            if flag.load(Ordering::Relaxed) {
                rec["state"] = json!("stopped");
                rec["why"] = json!("you stopped it");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}

/// Stops worker `id`: its flag when this server runs it, else its process
/// group when its record names one still running.
pub(crate) fn stop_worker(home: &Path, id: &str) -> Result<Value, String> {
    if !id_ok(id) {
        return Err(format!("a step's id is s and 12 hex digits (got {id:?})"));
    }
    if let Some(f) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
        f.store(true, Ordering::Relaxed);
        return Ok(json!({"stopping": id}));
    }
    let mut w: Value = fs::read(file(home, id))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| format!("no worker on step {id}"))?;
    if let Some(pid) = w["pid"].as_u64().filter(|p| gp_mcp::proc::alive(*p)) {
        gp_mcp::proc::stop_group(pid as u32, Duration::from_secs(2));
    }
    if loop_state::LIVE.contains(&w["state"].as_str().unwrap_or("")) {
        w["state"] = json!("stopped");
        w["why"] = json!("you stopped it");
        w["pid"] = Value::Null;
        write(home, &w);
    }
    Ok(json!({"stopped": id}))
}

fn stop(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let id = params.get("id").map(String::as_str).unwrap_or("");
    let v = stop_worker(&ctx.paths.home, id).map_err(Fail::conflict)?;
    Ok(Response::json(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, who: &str, touches: &[&str]) -> Value {
        json!({"id": id, "who": who, "subject": "g", "says": "Do it.", "commands": ["corpus import"],
            "touches": touches, "how": {"cli": "gp corpus import g --write"}})
    }

    #[test]
    fn the_plan_starts_the_agents_open_steps_that_touch_nothing_taken_up_to_the_limit() {
        let open = [
            s("s000000000001", "agent", &["game:a"]),
            s("s000000000002", "person", &["page"]),
            s("s000000000003", "agent", &["game:b"]),
            s("s000000000004", "agent", &["game:c"]),
            s("s000000000005", "agent", &["game:d"]),
        ];
        let ids = |v: Vec<Value>| -> Vec<String> {
            v.iter()
                .map(|x| x["id"].as_str().unwrap().to_owned())
                .collect()
        };
        let none = BTreeSet::new();
        assert_eq!(
            ids(plan(&open, &[], &none, 2)),
            ["s000000000001", "s000000000003"]
        );
        // A step a worker had is not started again; a live worker's touches
        // and count hold.
        let workers = [
            json!({"step": s("s000000000001", "agent", &["game:a"]), "state": "done"}),
            json!({"step": s("s000000000009", "agent", &["game:b"]), "state": "working"}),
        ];
        let live: BTreeSet<String> = ["s000000000009".to_owned()].into();
        assert_eq!(ids(plan(&open, &workers, &live, 2)), ["s000000000004"]);
        assert!(plan(&open, &workers, &live, 1).is_empty());
    }

    #[test]
    fn a_clip_step_names_its_run_and_clip() {
        let st = json!({"how": {"cli": "gp agent brief --run g --clip g.jump.001, then gp model gate g --patch FILE -m MSG"}});
        assert_eq!(clip_of(&st), Some(("g".into(), "g.jump.001".into())));
        assert_eq!(clip_of(&s("s000000000001", "agent", &[])), None);
    }

    fn git_in(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A temp home, games folder and repo with main, and a stand-in agent
    /// command: `sh SCRIPT`. Never Claude.
    fn rig(t: &Path, script: &str) -> Setup {
        let repo = t.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git_in(&repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join("a.txt"), "a\n").unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "first"]);
        let sh = t.join("agent.sh");
        fs::write(&sh, script).unwrap();
        fs::create_dir_all(t.join("games")).unwrap();
        Setup {
            home: t.join("home"),
            games: t.join("games"),
            repo,
            gp: PathBuf::from("/nonexistent/gp"),
            argv: vec!["sh".into(), sh.display().to_string()],
            program: PathBuf::from("/bin/sh"),
            timeout: Duration::from_secs(60),
            tries: 2,
            again_after: Duration::from_millis(50),
        }
    }

    fn record(home: &Path, id: &str) -> Value {
        serde_json::from_slice(&fs::read(file(home, id)).unwrap()).unwrap()
    }

    #[test]
    fn a_worker_gets_its_step_brief_and_worktree_and_ends_done() {
        let t = tempfile::tempdir().unwrap();
        let out = t.path().join("seen");
        let setup = rig(
            t.path(),
            &format!(
                "echo \"$GP_STEP $GP_WORKER $(git rev-parse --abbrev-ref HEAD)\" > {o}\ncat >> {o}\necho all done\n",
                o = out.display()
            ),
        );
        let id = "s0000000000a1";
        spawn(setup.clone(), s(id, "agent", &["game:g"]))
            .join()
            .unwrap();
        let seen = fs::read_to_string(&out).unwrap();
        assert!(
            seen.starts_with(&format!("{id} loop-{id} loop/{id}\n")),
            "{seen}"
        );
        assert!(
            seen.contains(&format!("Step {id}, for g: Do it.")),
            "{seen}"
        );
        assert!(seen.contains("gp_run cites"), "{seen}");
        let r = record(&setup.home, id);
        assert_eq!(r["state"], "done");
        assert_eq!(r["tries"], 1);
        assert_eq!(r["last"], "all done");
        assert!(r["pid"].is_null());
        assert!(!loop_state::holds(&r));
    }

    #[test]
    fn a_failing_worker_is_tried_again_then_stopped() {
        let t = tempfile::tempdir().unwrap();
        let setup = rig(t.path(), "echo nope\nexit 3\n");
        let id = "s0000000000a2";
        spawn(setup.clone(), s(id, "agent", &["game:g"]))
            .join()
            .unwrap();
        let r = record(&setup.home, id);
        assert_eq!(r["state"], "stopped");
        assert_eq!(r["tries"], 2);
        assert!(r["why"].as_str().unwrap().contains("failed 2 times"), "{r}");
        // The second try's brief says how the first ended.
        let b2 = fs::read_to_string(files_dir(&setup.home, id).join("brief.2.txt")).unwrap();
        assert!(
            b2.contains("Your try 1 at this step ended with exit 3"),
            "{b2}"
        );
    }

    #[test]
    fn stop_ends_a_working_worker() {
        let t = tempfile::tempdir().unwrap();
        let setup = rig(t.path(), "echo working\nsleep 30\n");
        let id = "s0000000000a3";
        let h = spawn(setup.clone(), s(id, "agent", &["game:g"]));
        let t0 = Instant::now();
        while record(&setup.home, id)["pid"].is_null() && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(loop_state::holds(&record(&setup.home, id)), "it runs");
        stop_worker(&setup.home, id).unwrap();
        h.join().unwrap();
        assert!(t0.elapsed() < Duration::from_secs(20));
        let r = record(&setup.home, id);
        assert_eq!(r["state"], "stopped");
        assert_eq!(r["why"], "you stopped it");
    }
}
