//! GitHub's checks of main, for `gp loop health`: the newest finished run
//! of each workflow on main, read with `gh` in the checkout gp was built
//! from, and a PROBLEM for each that failed, naming its step and its first
//! FAIL line. Until this, check-web failed at mechanic-page on three pushes
//! (7a2abdff, b8d68308, aa015082, 2026-11-25) before anyone looked: `just
//! check` runs no browser check and nothing in the loop read GitHub.
//!
//! The games checkout's workflows are read the same way, in that checkout
//! ([`Repo`]), and a run of either repo queued on main for over
//! [`QUEUED_LONG_MS`] is a PROBLEM naming the job that waits and, for a
//! self-hosted runner, how to start it (health-reads-the-games-repo-s-checks:
//! from 2026-11-19 to 2026-11-28 the games repo's self-hosted runner was
//! off, every push's run sat queued, and no line said so).
//!
//! The read goes to GitHub at most every [`EVERY_MS`] and is kept in
//! `$GP_HOME/work/loop/github-checks.json` (the games repo's in
//! `github-checks-games.json`); gp serve's runner reads it in
//! the background so its heartbeat never waits on the network, and `gp
//! loop health` by hand reads it first when it is older. A home in the
//! temp folder (a test's) never asks GitHub. Where `gh` can't say (not
//! installed, not logged in, no network), health prints an UNREAD line and
//! counts no problem.

use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How old the kept read may be before GitHub is asked again: the runner's
/// health check runs every 5 minutes.
pub(crate) const EVERY_MS: u64 = 5 * 60 * 1000;

/// How long one `gh` call may take.
const GH_LIMIT: Duration = Duration::from_secs(30);

/// The most of a failed run's log read for its FAIL line.
const LOG_MAX: usize = 8 << 20;

/// A conclusion that is a failure of the check (a cancelled or skipped run
/// says nothing about main, so the run before it stands).
fn failed(c: &str) -> bool {
    matches!(c, "failure" | "timed_out" | "startup_failure")
}

fn settled(c: &str) -> bool {
    c == "success" || failed(c)
}

/// How long a run on main may sit queued before it is a problem: a check
/// takes minutes, so an hour is a runner that isn't there.
pub(crate) const QUEUED_LONG_MS: u64 = 60 * 60 * 1000;

/// A repo whose checks health reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Repo {
    /// gameprinter's own, the checkout gp was built from.
    Gameprinter,
    /// The games checkout's.
    Games,
}

impl Repo {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Repo::Gameprinter => "gameprinter",
            Repo::Games => "games",
        }
    }

    /// The part of the kept files' names after `github-` that is the
    /// repo's: none for gameprinter's (the names from before the games
    /// repo was read), `games-` for the games repo's.
    fn part(self) -> &'static str {
        match self {
            Repo::Gameprinter => "",
            Repo::Games => "games-",
        }
    }
}

/// The repos to read, each with its checkout: gameprinter's when gp was
/// built from a checkout, and the games checkout (`games`, else
/// crate::roms_cmd::games_dir's) when it is in git and isn't that one.
pub(crate) fn repos(games: Option<&Path>) -> Vec<(Repo, PathBuf)> {
    let in_git = |d: &Path| d.ancestors().any(|a| a.join(".git").exists());
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let mut out = Vec::new();
    if let Some(r) = crate::repo::built().filter(|r| r.join(".git").exists()) {
        out.push((Repo::Gameprinter, r));
    }
    if let Some(g) = crate::roms_cmd::games_dir(games).filter(|g| in_git(g))
        && !out.iter().any(|(_, r)| canon(r) == canon(&g))
    {
        out.push((Repo::Games, g));
    }
    out
}

/// The kept read's file of `repo`.
pub(crate) fn file(home: &Path, repo: Repo) -> PathBuf {
    let name = match repo {
        Repo::Gameprinter => "github-checks.json".to_owned(),
        Repo::Games => format!("github-checks-{}.json", repo.name()),
    };
    gp_mcp::steps::loop_dir(home).join(name)
}

/// One workflow's standing on main: its newest settled run, and how many
/// settled runs in a row up to it failed (0 when it passed).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Standing {
    pub repo: Repo,
    pub workflow: String,
    pub id: u64,
    pub sha: String,
    pub url: String,
    pub failed_in_a_row: usize,
    /// The commit of a run of the workflow still going (queued or in
    /// progress) newer than this one, when there is one.
    pub running: Option<String>,
}

/// Each workflow's standing from `gh run list --json` (newest first) of
/// `repo`, in the order their newest settled runs come.
pub(crate) fn standings(runs: &Value, repo: Repo) -> Vec<Standing> {
    let mut out: Vec<Standing> = Vec::new();
    let mut done: Vec<String> = Vec::new();
    let mut going: Vec<(String, String)> = Vec::new();
    for r in runs.as_array().into_iter().flatten() {
        let w = r["workflowName"].as_str().unwrap_or("").to_owned();
        let c = r["conclusion"].as_str().unwrap_or("");
        if !w.is_empty()
            && r["status"] != json!("completed")
            && !out.iter().any(|s| s.workflow == w)
            && !going.iter().any(|g| g.0 == w)
        {
            let sha = r["headSha"].as_str().unwrap_or("");
            going.push((w.clone(), sha.to_owned()));
        }
        if w.is_empty() || r["status"] != json!("completed") || !settled(c) || done.contains(&w) {
            continue;
        }
        match out.iter_mut().find(|s| s.workflow == w) {
            Some(s) if failed(c) => s.failed_in_a_row += 1,
            Some(_) => done.push(w),
            None => {
                if !failed(c) {
                    done.push(w.clone());
                }
                let running = going.iter().find(|g| g.0 == w).map(|g| g.1.clone());
                out.push(Standing {
                    repo,
                    workflow: w,
                    id: r["databaseId"].as_u64().unwrap_or(0),
                    sha: r["headSha"].as_str().unwrap_or("").to_owned(),
                    url: r["url"].as_str().unwrap_or("").to_owned(),
                    failed_in_a_row: usize::from(failed(c)),
                    running,
                });
            }
        }
    }
    out
}

/// The failed steps of a run from `gh run view ID --json jobs`, as `STEP
/// (job JOB)`; a failed job with no failed step is `job JOB`.
pub(crate) fn failed_steps(jobs: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for j in jobs["jobs"].as_array().into_iter().flatten() {
        if !failed(j["conclusion"].as_str().unwrap_or("")) {
            continue;
        }
        let job = j["name"].as_str().unwrap_or("");
        let steps: Vec<&str> = j["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| failed(s["conclusion"].as_str().unwrap_or("")))
            .filter_map(|s| s["name"].as_str())
            .collect();
        if steps.is_empty() {
            out.push(format!("job {job}"));
        }
        out.extend(steps.into_iter().map(|s| format!("{s} (job {job})")));
    }
    out
}

/// A run on main that hasn't ended and was made over [`QUEUED_LONG_MS`]
/// ago: the oldest such run of its workflow.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Long {
    pub repo: Repo,
    pub workflow: String,
    pub id: u64,
    pub sha: String,
    pub url: String,
    /// Whether the run itself is still queued (no job of it started).
    pub queued: bool,
    /// How long ago it was made.
    pub waited_ms: u64,
}

fn waits(status: &str) -> bool {
    matches!(status, "queued" | "waiting" | "pending" | "requested")
}

/// The runs of `gh run list --json` (newest first, with `createdAt`) that
/// were made over [`QUEUED_LONG_MS`] before `now` and haven't ended, the
/// oldest of each workflow, in the order of their workflows' newest runs.
pub(crate) fn long_runs(runs: &Value, repo: Repo, now: u64) -> Vec<Long> {
    let mut out: Vec<Long> = Vec::new();
    for r in runs.as_array().into_iter().flatten() {
        let w = r["workflowName"].as_str().unwrap_or("");
        let status = r["status"].as_str().unwrap_or("");
        let Some(made) = r["createdAt"]
            .as_str()
            .and_then(crate::store::rfc3339_seconds)
        else {
            continue;
        };
        let waited = now.saturating_sub((made * 1000.0) as u64);
        if w.is_empty() || status == "completed" || waited <= QUEUED_LONG_MS {
            continue;
        }
        let long = Long {
            repo,
            workflow: w.to_owned(),
            id: r["databaseId"].as_u64().unwrap_or(0),
            sha: r["headSha"].as_str().unwrap_or("").to_owned(),
            url: r["url"].as_str().unwrap_or("").to_owned(),
            queued: waits(status),
            waited_ms: waited,
        };
        // Newest first: a later one of the same workflow is older.
        match out.iter_mut().find(|l| l.workflow == w) {
            Some(l) => *l = long,
            None => out.push(long),
        }
    }
    out
}

/// The jobs of a run that wait for a runner, from `gh api
/// repos/{owner}/{repo}/actions/runs/ID/jobs` (whose jobs have their
/// runner's `labels`, which `gh run view --json jobs` hasn't): each job's
/// name and labels.
pub(crate) fn waiting_jobs(jobs: &Value) -> Vec<(String, Vec<String>)> {
    jobs["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|j| waits(j["status"].as_str().unwrap_or("")))
        .map(|j| {
            let labels = j["labels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            (j["name"].as_str().unwrap_or("?").to_owned(), labels)
        })
        .collect()
}

/// The runner folder a home may hold for the games repo's self-hosted
/// runner (its run.sh starts it).
pub(crate) fn runner_dir(home: &Path) -> PathBuf {
    home.join("runner-games")
}

/// A long run as health's PROBLEM (its words and the lines under it), or
/// None when it is going with no job waiting (a long check, not a missing
/// runner). `jobs` are its [`waiting_jobs`]. The words name no commit or
/// run, which go under them, so a runner that stays off is one trouble row
/// that doesn't move from push to push.
pub(crate) fn long_line(
    l: &Long,
    jobs: &[(String, Vec<String>)],
    runner: Option<&Path>,
) -> Option<(String, Vec<String>)> {
    if jobs.is_empty() && !l.queued {
        return None;
    }
    let hours = (l.waited_ms / 3_600_000).max(1);
    let what = if jobs.is_empty() {
        "no job of it has started".to_owned()
    } else {
        jobs.iter()
            .map(|(name, labels)| {
                if labels.is_empty() {
                    format!("its job {name} waits for a runner")
                } else {
                    format!(
                        "its job {name} waits for a runner labelled {}",
                        labels.join(", ")
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    let self_hosted = jobs
        .iter()
        .any(|(_, labels)| labels.iter().any(|x| x == "self-hosted"));
    let how = if self_hosted {
        let at = match runner {
            Some(d) => format!(" ({})", d.display()),
            None => String::new(),
        };
        format!(
            "Only a self-hosted runner takes it, and none has: the runner is likely off. Start it on the machine it was set up on with ./run.sh in the runner's folder{at}, and the queued runs go."
        )
    } else {
        format!(
            "GitHub's runners haven't taken it: GitHub's status page may say why; `gh run view {}` in the {} checkout shows it.",
            l.id,
            l.repo.name()
        )
    };
    let words = format!(
        "GitHub's {} of {} has a run on main queued for over {hours} hour(s): {what}. {how}",
        l.workflow,
        l.repo.name()
    );
    let mut under = vec![format!(
        "           the oldest such run is on {}",
        &l.sha[..l.sha.len().min(8)]
    )];
    if !l.url.is_empty() {
        under.push(format!("           {}", l.url));
    }
    Some((words, under))
}

/// The lines of a failed run's log (`gh run view ID --log-failed`, each
/// line `JOB<tab>STEP<tab>TIME TEXT`) that say why, with the job, step,
/// time and terminal colours taken off: the first that starts with FAIL;
/// `just check`'s failed steps (`check-step: NAME S s (failed)`, from
/// scripts/check/step.sh) and its failed test runs (`tests: NAME failed`);
/// the tests cargo failed (`test PATH ... FAILED`, at most three) and the
/// first panic with the line after it. With none of those, the first error
/// GitHub marked (`##[error]`). Until this only FAIL and `##[error]` were
/// read, and `just check` prints no FAIL: on 1c2cc41a (2026-11-27) health
/// kept "Process completed with exit code 1." and nothing else.
pub(crate) fn why_lines(log: &str) -> Vec<String> {
    let texts: Vec<String> = log.lines().map(log_text).collect();
    let mut out: Vec<String> = Vec::new();
    fn add(out: &mut Vec<String>, t: &str) {
        let t = crate::next::cut(t.trim(), 200);
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    if let Some(f) = texts.iter().find(|t| t.starts_with("FAIL")) {
        add(&mut out, f);
    }
    for t in &texts {
        if t.starts_with("check-step: ") && t.ends_with("(failed)") {
            add(&mut out, t);
        }
    }
    for t in &texts {
        if t.starts_with("tests: ") && t.contains(" failed") {
            add(&mut out, t);
        }
    }
    for t in texts
        .iter()
        .filter(|t| t.starts_with("test ") && t.ends_with("... FAILED"))
        .take(3)
    {
        add(&mut out, t);
    }
    if let Some(i) = texts.iter().position(|t| t.contains("' panicked at ")) {
        add(&mut out, &texts[i]);
        if let Some(next) = texts.get(i + 1) {
            add(&mut out, next);
        }
    }
    if out.is_empty()
        && let Some(e) = texts.iter().find_map(|t| t.strip_prefix("##[error]"))
    {
        add(&mut out, e);
    }
    out
}

/// Where health keeps the failed steps' log of `workflow`'s newest failed
/// run on main: `$GP_HOME/work/loop/github-failed-NAME.log` for
/// gameprinter's (`github-failed-games-NAME.log` for the games repo's),
/// NAME the workflow's name with each other character made `-`. A loop
/// worker's shell runs no `gh`, but it can read the home's work folders.
pub(crate) fn log_file(home: &Path, repo: Repo, workflow: &str) -> PathBuf {
    let name: String = workflow
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    gp_mcp::steps::loop_dir(home).join(format!("github-failed-{}{name}.log", repo.part()))
}

fn log_text(line: &str) -> String {
    let text = line.rsplit('\t').next().unwrap_or(line);
    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // A colour: ESC [ ... letter.
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        clean.push(c);
    }
    let t = clean.trim_start_matches('\u{feff}').trim();
    // GitHub's time at the start: 2026-11-25T10:00:00.1234567Z.
    match t.split_once(' ') {
        Some((time, rest)) if time.len() >= 20 && time.ends_with('Z') && time.contains('T') => {
            rest.trim().to_owned()
        }
        _ => t.to_owned(),
    }
}

/// A standing as health's line: Ok for a pass, Err (the problem's words,
/// and the lines under it) for a failure. The words leave out what changes
/// from push to push only as numbers (the commit, the run, the count),
/// which crate::trouble's key turns into `#`, so a check that stays red
/// adds to one row.
/// `log` is where its failed steps' log was kept, when it was: the words
/// name it after the part [`crate::trouble::key`] keeps (its first 240
/// characters), so the row stays the one it was.
pub(crate) fn line(
    s: &Standing,
    steps: &[String],
    why: &[String],
    log: Option<&Path>,
) -> Result<String, (String, Vec<String>)> {
    let at = &s.sha[..s.sha.len().min(8)];
    let repo = s.repo.name();
    if s.failed_in_a_row == 0 {
        return Ok(format!(
            "GitHub's {} of {repo} passes on main at {at}",
            s.workflow
        ));
    }
    let step = if steps.is_empty() {
        String::new()
    } else {
        format!(" at {}", steps.join(", "))
    };
    let kept = match log {
        Some(p) => format!(
            " Its failed steps' log, read by health, is at {} (a worker's shell runs no gh; read it there).",
            p.display()
        ),
        None => String::new(),
    };
    let fix = match s.repo {
        Repo::Gameprinter => {
            "`just check` runs no such check, so fix it as a tools row and land it through the tool gate"
        }
        Repo::Games => "Fix it as a games row and land it through the model gate",
    };
    let words = format!(
        "GitHub's {} of {repo} fails on main{step}: its last {} finished run(s) failed, the newest on {at}. {fix}; `gh run view {} --log-failed` in the {repo} checkout prints the whole failure.{kept}",
        s.workflow, s.failed_in_a_row, s.id
    );
    let mut under = Vec::new();
    for w in why {
        under.push(format!("           {w}"));
    }
    if !s.url.is_empty() {
        under.push(format!("           {}", s.url));
    }
    Err((words, under))
}

/// A failing standing with a newer run of its workflow still going, as
/// health's WAITING line: the failure is held back, not a PROBLEM, so
/// nothing new is filed and its trouble row stays as it is until that run
/// ends (2026-11-26: health filed check-web's failure on aa015082 and a
/// worker took it up while b32f4590's run, its fix, was still going).
pub(crate) fn waiting_line(s: &Standing) -> Option<String> {
    if s.failed_in_a_row == 0 {
        return None;
    }
    let going = s.running.as_deref()?;
    let at = &s.sha[..s.sha.len().min(8)];
    let on = if going.is_empty() {
        String::new()
    } else {
        format!(" on {}", &going[..going.len().min(8)])
    };
    Some(format!(
        "GitHub's {} of {} failed on main at {at}, and a newer run{on} is still going: health waits for it to end",
        s.workflow,
        s.repo.name()
    ))
}

/// The `gh` to run: $GP_GH, else gh on the PATH, else Homebrew's (gp serve
/// started from launchd has a short PATH).
fn gh() -> String {
    if let Ok(g) = std::env::var("GP_GH")
        && !g.is_empty()
    {
        return g;
    }
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("gh").is_file()))
        .unwrap_or(false);
    if !on_path {
        for g in ["/opt/homebrew/bin/gh", "/usr/local/bin/gh"] {
            if Path::new(g).is_file() {
                return g.to_owned();
            }
        }
    }
    "gh".to_owned()
}

/// One `gh` call in `repo`, stopped after [`GH_LIMIT`]: its stdout, or why
/// it gave none.
fn gh_out(repo: &Path, args: &[&str]) -> Result<String, String> {
    let mut child = Command::new(gh())
        .args(args)
        .current_dir(repo)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run gh: {e}"))?;
    let drain = |p: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(mut p) = p {
                let _ = p.by_ref().take(LOG_MAX as u64).read_to_end(&mut b);
                // Read the rest so gh never blocks on a full pipe.
                let _ = std::io::copy(&mut p, &mut std::io::sink());
            }
            b
        })
    };
    let out = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < GH_LIMIT => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "gh {} took over {} s",
                    args.join(" "),
                    GH_LIMIT.as_secs()
                ));
            }
            Err(e) => return Err(format!("couldn't wait for gh: {e}")),
        }
    };
    let out = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let err = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    if status.success() {
        Ok(out)
    } else {
        let e = err
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        Err(format!(
            "gh {} failed: {}",
            args[..2.min(args.len())].join(" "),
            crate::next::cut(e, 160)
        ))
    }
}

/// Reads `which`'s checks on GitHub now, `gh` run in its checkout `repo`:
/// the read as kept in [`file`], with each failed run's log kept at
/// [`log_file`].
pub(crate) fn read_now(home: &Path, which: Repo, repo: &Path, now: u64) -> Value {
    let runs = gh_out(
        repo,
        &[
            "run",
            "list",
            "--branch",
            "main",
            "--limit",
            "60",
            "--json",
            "databaseId,workflowName,headSha,status,conclusion,url,createdAt",
        ],
    )
    .and_then(|o| {
        serde_json::from_str::<Value>(&o).map_err(|e| format!("gh run list said no list: {e}"))
    });
    let runs = match runs {
        Ok(r) => r,
        Err(why) => return json!({"read_ms": now, "unread": why}),
    };
    let mut lines = Vec::new();
    for s in standings(&runs, which) {
        let (steps, why, log) = if s.failed_in_a_row > 0 {
            let id = s.id.to_string();
            let steps = gh_out(repo, &["run", "view", &id, "--json", "jobs"])
                .ok()
                .and_then(|o| serde_json::from_str::<Value>(&o).ok())
                .map(|j| failed_steps(&j))
                .unwrap_or_default();
            let text = gh_out(repo, &["run", "view", &id, "--log-failed"]).ok();
            let why = text.as_deref().map(why_lines).unwrap_or_default();
            let log = text.and_then(|t| keep_log(home, &s, &t));
            (steps, why, log)
        } else {
            (Vec::new(), Vec::new(), None)
        };
        // A failure with a newer run going is held back until that run ends.
        lines.push(
            match (waiting_line(&s), line(&s, &steps, &why, log.as_deref())) {
                (_, Ok(l)) => json!({"ok": l}),
                (Some(w), Err((p, _))) => json!({"waiting": w, "held": p}),
                (None, Err((p, under))) => json!({"problem": p, "under": under}),
            },
        );
    }
    // A run sitting queued: the jobs waiting, with their runners' labels.
    let runner = Some(runner_dir(home)).filter(|d| d.join("run.sh").is_file());
    for l in long_runs(&runs, which, now) {
        let jobs = gh_out(
            repo,
            &[
                "api",
                &format!("repos/{{owner}}/{{repo}}/actions/runs/{}/jobs", l.id),
            ],
        )
        .ok()
        .and_then(|o| serde_json::from_str::<Value>(&o).ok())
        .map(|j| waiting_jobs(&j))
        .unwrap_or_default();
        if let Some((p, under)) = long_line(&l, &jobs, runner.as_deref()) {
            lines.push(json!({"problem": p, "under": under}));
        }
    }
    json!({"read_ms": now, "lines": lines})
}

/// Writes a failed run's log to [`log_file`], its first line naming the
/// run, the commit and its address: the file, or None when it couldn't be
/// written.
fn keep_log(home: &Path, s: &Standing, log: &str) -> Option<PathBuf> {
    let f = log_file(home, s.repo, &s.workflow);
    std::fs::create_dir_all(f.parent()?).ok()?;
    let head = format!(
        "GitHub's {} of {} on main at {}, run {} ({}): gh run view {} --log-failed\n",
        s.workflow,
        s.repo.name(),
        s.sha,
        s.id,
        s.url,
        s.id
    );
    let tmp = f.with_extension("log.tmp");
    std::fs::write(&tmp, format!("{head}{log}")).ok()?;
    std::fs::rename(&tmp, &f).ok()?;
    Some(f)
}

const REPOS: [Repo; 2] = [Repo::Gameprinter, Repo::Games];

fn kept(home: &Path, repo: Repo) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(file(home, repo)).ok()?).ok()
}

fn fresh(home: &Path, repo: Repo, now: u64) -> bool {
    kept(home, repo)
        .and_then(|v| v["read_ms"].as_u64())
        .is_some_and(|t| now.saturating_sub(t) < EVERY_MS)
}

fn keep(home: &Path, repo: Repo, v: &Value) {
    let f = file(home, repo);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = f.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &f);
    }
}

/// Reads GitHub for each of `repos` (from [`repos`]) whose kept read is
/// older than [`EVERY_MS`] (`gp loop health` by hand, before its checks).
pub(crate) fn refresh(home: &Path, repos: &[(Repo, PathBuf)]) {
    let now = gp_mcp::asks::now_ms();
    if !crate::trouble::live_home(home) {
        return;
    }
    for (which, dir) in repos {
        if !fresh(home, *which, now) {
            keep(home, *which, &read_now(home, *which, dir, now));
        }
    }
}

static READING: AtomicBool = AtomicBool::new(false);

/// [`refresh`] on a thread of its own, one at a time (gp serve's runner,
/// whose heartbeat mustn't wait on GitHub).
pub(crate) fn refresh_behind(home: &Path, repos: Vec<(Repo, PathBuf)>) {
    let now = gp_mcp::asks::now_ms();
    let stale: Vec<(Repo, PathBuf)> = repos
        .into_iter()
        .filter(|(which, _)| !fresh(home, *which, now))
        .collect();
    if !crate::trouble::live_home(home) || stale.is_empty() || READING.swap(true, Ordering::SeqCst)
    {
        return;
    }
    let home = home.to_path_buf();
    std::thread::spawn(move || {
        for (which, dir) in stale {
            keep(
                &home,
                which,
                &read_now(&home, which, &dir, gp_mcp::asks::now_ms()),
            );
        }
        READING.store(false, Ordering::SeqCst);
    });
}

/// The failures health holds back while a newer run goes (their PROBLEM
/// words, for crate::trouble to keep their rows open), of every repo read,
/// or None when the kept reads can't say: there is none yet, or gh couldn't
/// read GitHub for one of them.
pub(crate) fn held(home: &Path) -> Option<Vec<String>> {
    let mut held = Vec::new();
    let mut any = false;
    for v in REPOS.iter().filter_map(|r| kept(home, *r)) {
        if v["unread"].is_string() {
            return None;
        }
        any = true;
        held.extend(
            v["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| l["held"].as_str())
                .map(str::to_owned),
        );
    }
    any.then_some(held)
}

/// Health's lines from the kept reads, gameprinter's then the games
/// repo's: Ok lines, Err problems with the lines that go under each, and an
/// UNREAD line for a repo gh couldn't say of.
pub(crate) fn health_lines(home: &Path, now: u64) -> Vec<Result<String, (String, Vec<String>)>> {
    REPOS
        .iter()
        .flat_map(|r| repo_lines(home, *r, now))
        .collect()
}

fn repo_lines(home: &Path, repo: Repo, now: u64) -> Vec<Result<String, (String, Vec<String>)>> {
    let Some(v) = kept(home, repo) else {
        return Vec::new();
    };
    let ago = now.saturating_sub(v["read_ms"].as_u64().unwrap_or(0)) / 60_000;
    let read = if ago == 0 {
        "read under a minute ago".to_owned()
    } else {
        format!("read {ago} min ago")
    };
    if let Some(why) = v["unread"].as_str() {
        return vec![Ok(format!(
            "UNREAD   GitHub's checks of the {} repo's main ({read}): {why}",
            repo.name()
        ))];
    }
    let mut out = Vec::new();
    for l in v["lines"].as_array().into_iter().flatten() {
        if let Some(ok) = l["ok"].as_str() {
            out.push(Ok(format!("OK       {ok} ({read})")));
        } else if let Some(w) = l["waiting"].as_str() {
            out.push(Ok(format!("WAITING  {w} ({read})")));
        } else if let Some(p) = l["problem"].as_str() {
            let under = l["under"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            out.push(Err((p.to_owned(), under)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(w: &str, id: u64, sha: &str, status: &str, c: &str) -> Value {
        json!({"workflowName": w, "databaseId": id, "headSha": sha, "status": status,
               "conclusion": c, "url": format!("https://github.com/o/r/actions/runs/{id}")})
    }

    #[test]
    fn a_check_failing_on_main_is_a_problem_naming_its_step_and_fail_line() {
        // gh run list, newest first: check-web failed on its last three
        // finished runs (one cancelled between them, one still running),
        // check passed, site failed once before passing.
        let runs = json!([
            run("check-web", 9, "", "in_progress", ""),
            run("check-web", 8, "aa015082aaaa", "completed", "failure"),
            run("check", 7, "aa015082aaaa", "completed", "success"),
            run("check-web", 6, "b8d68308bbbb", "completed", "cancelled"),
            run("check-web", 5, "b8d68308bbbb", "completed", "failure"),
            run("site", 4, "7405143dcccc", "completed", "success"),
            run("check-web", 3, "7a2abdffdddd", "completed", "failure"),
            run("site", 2, "7a2abdffdddd", "completed", "failure"),
            run("check-web", 1, "f5f1e226eeee", "completed", "success"),
            run("check-web", 0, "f5f1e226eeee", "completed", "failure"),
        ]);
        let s = standings(&runs, Repo::Gameprinter);
        assert_eq!(
            s.iter()
                .map(|s| (s.workflow.as_str(), s.id, s.failed_in_a_row))
                .collect::<Vec<_>>(),
            vec![("check-web", 8, 3), ("check", 7, 0), ("site", 4, 0)]
        );
        assert_eq!(s[0].running.as_deref(), Some(""));
        let jobs = json!({"jobs": [
            {"name": "build", "conclusion": "success", "steps": []},
            {"name": "web", "conclusion": "failure", "steps": [
                {"name": "players", "conclusion": "success"},
                {"name": "mechanic-page", "conclusion": "failure"},
                {"name": "page", "conclusion": "skipped"}]}]});
        let steps = failed_steps(&jobs);
        assert_eq!(steps, vec!["mechanic-page (job web)"]);
        let log = "web\tmechanic-page\t2026-11-25T10:00:00.1234567Z ok the root\n\
                   web\tmechanic-page\t2026-11-25T10:01:00.1234567Z \u{1b}[31mFAIL timed out: the game's page\u{1b}[0m\n\
                   web\tmechanic-page\t2026-11-25T10:01:01.1234567Z ##[error]Process completed with exit code 1.\n";
        let why = why_lines(log);
        assert_eq!(why, vec!["FAIL timed out: the game's page"]);
        let Err((words, under)) = line(&s[0], &steps, &why, None) else {
            panic!("a failing check is a problem");
        };
        assert!(words.starts_with("GitHub's check-web of gameprinter fails on main at mechanic-page (job web): its last 3 finished run(s) failed, the newest on aa015082."), "{words}");
        assert_eq!(under[0], "           FAIL timed out: the game's page");
        assert_eq!(
            line(&s[1], &[], &[], None),
            Ok("GitHub's check of gameprinter passes on main at aa015082".to_owned())
        );
        // The next push failing the same way is the same trouble row.
        let later = Standing {
            id: 12,
            sha: "0c0ffee12345".into(),
            failed_in_a_row: 4,
            ..s[0].clone()
        };
        let Err((again, _)) = line(&later, &steps, &[], None) else {
            panic!()
        };
        assert_eq!(crate::trouble::key(&words), crate::trouble::key(&again));
        // With no FAIL line, GitHub's own error says why.
        assert_eq!(
            why_lines(
                "j\ts\t2026-11-25T10:01:01.1234567Z ##[error]Process completed with exit code 1."
            ),
            vec!["Process completed with exit code 1."]
        );
    }

    /// `just check` prints no FAIL line: its failed step, its failed test
    /// run, the tests cargo failed and the first panic are what say why, and
    /// GitHub's "exit code 1" isn't kept beside them (2026-11-27: health kept
    /// only that for 1c2cc41a). The failed steps' log is kept where a
    /// worker can read it, named after the part of the words the trouble
    /// row's key keeps, so the row stays the one it was.
    #[test]
    fn a_just_check_failure_names_its_step_and_tests_and_its_log_is_kept() {
        let at = "check\tRun just check\t2026-11-27T05:00:00.1234567Z ";
        let log = [
            "check-step: fmt 1.2 s",
            "running 3 tests",
            "test loop_train::tests::a_train_lands_two ... ok",
            "test loop_train::tests::a_train_lands_one ... FAILED",
            "test runner_limits::tests::a_run_past_its_limit_is_cut ... FAILED",
            "---- loop_train::tests::a_train_lands_one stdout ----",
            "thread 'loop_train::tests::a_train_lands_one' panicked at crates/gp-cli/src/loop_train.rs:900:9:",
            "assertion `left == right` failed: the tree",
            "tests: rest failed; its output:",
            "check-step: test 812.0 s (failed)",
            "##[error]Process completed with exit code 1.",
        ]
        .iter()
        .map(|l| format!("{at}{l}\n"))
        .collect::<String>();
        assert_eq!(
            why_lines(&log),
            vec![
                "check-step: test 812.0 s (failed)",
                "tests: rest failed; its output:",
                "test loop_train::tests::a_train_lands_one ... FAILED",
                "test runner_limits::tests::a_run_past_its_limit_is_cut ... FAILED",
                "thread 'loop_train::tests::a_train_lands_one' panicked at crates/gp-cli/src/loop_train.rs:900:9:",
                "assertion `left == right` failed: the tree",
            ]
        );

        let home = tempfile::tempdir().unwrap();
        let s = Standing {
            repo: Repo::Gameprinter,
            workflow: "check".into(),
            id: 46113570452,
            sha: "1c2cc41ab7d1".into(),
            url: "https://github.com/o/r/actions/runs/46113570452".into(),
            failed_in_a_row: 1,
            running: None,
        };
        let f = keep_log(home.path(), &s, &log).unwrap();
        assert_eq!(f, log_file(home.path(), Repo::Gameprinter, "check"));
        assert!(
            f.ends_with("work/loop/github-failed-check.log"),
            "{}",
            f.display()
        );
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.starts_with("GitHub's check of gameprinter on main at 1c2cc41ab7d1, run 46113570452 (https://github.com/o/r/actions/runs/46113570452): gh run view 46113570452 --log-failed\n"), "{text}");
        assert!(text.ends_with(&log));
        assert_eq!(
            log_file(home.path(), Repo::Gameprinter, "chant release")
                .file_name()
                .unwrap(),
            "github-failed-chant-release.log"
        );
        assert_eq!(
            log_file(home.path(), Repo::Games, "check")
                .file_name()
                .unwrap(),
            "github-failed-games-check.log"
        );

        let steps = vec!["Run just check (job check)".to_owned()];
        let why = why_lines(&log);
        let Err((words, under)) = line(&s, &steps, &why, Some(&f)) else {
            panic!("a failing check is a problem")
        };
        assert!(
            words.ends_with(&format!(
                "prints the whole failure. Its failed steps' log, read by health, is at {} (a worker's shell runs no gh; read it there).",
                f.display()
            )),
            "{words}"
        );
        assert_eq!(under.len(), 7, "{under:?}");
        assert_eq!(under[0], "           check-step: test 812.0 s (failed)");
        assert_eq!(
            under[6],
            "           https://github.com/o/r/actions/runs/46113570452"
        );
        let Err((unkept, _)) = line(&s, &steps, &[], None) else {
            panic!()
        };
        assert_eq!(crate::trouble::key(&words), crate::trouble::key(&unkept));
    }

    /// A failure with a newer run of its workflow still going is held back
    /// as a WAITING line (2026-11-26: check-web's fix b32f4590 was still
    /// running while health filed aa015082's failure); a run still going
    /// that is older than the settled one holds nothing.
    #[test]
    fn a_failure_with_a_newer_run_going_waits_for_it() {
        let runs = json!([
            run("check-web", 3, "b32f4590ffff", "in_progress", ""),
            run("check-web", 2, "aa015082aaaa", "completed", "failure"),
            run("site", 1, "aa015082aaaa", "completed", "failure"),
            run("site", 0, "0c0ffee12345", "queued", ""),
        ]);
        let s = standings(&runs, Repo::Gameprinter);
        assert_eq!(s[0].running.as_deref(), Some("b32f4590ffff"));
        assert_eq!(s[1].running, None, "its run going is older");
        let w = waiting_line(&s[0]).unwrap();
        assert!(w.starts_with("GitHub's check-web of gameprinter failed on main at aa015082"));
        assert!(w.contains("newer run on b32f4590 is still going"));
        assert_eq!(waiting_line(&s[1]), None);
        let ended = Standing {
            running: None,
            ..s[0].clone()
        };
        assert_eq!(waiting_line(&ended), None);
    }

    #[test]
    fn health_reads_the_kept_read_and_a_test_home_never_asks_github() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        assert!(health_lines(home.path(), 0).is_empty());
        assert_eq!(held(home.path()), None, "nothing read yet");
        // A home in the temp folder: no gh is run and nothing is kept.
        let both = [
            (Repo::Gameprinter, repo.path().to_path_buf()),
            (Repo::Games, repo.path().to_path_buf()),
        ];
        refresh(home.path(), &both);
        assert!(!file(home.path(), Repo::Gameprinter).exists());
        assert!(!file(home.path(), Repo::Games).exists());
        assert!(
            file(home.path(), Repo::Games).ends_with("work/loop/github-checks-games.json"),
            "the games repo's read is kept apart"
        );
        keep(
            home.path(),
            Repo::Gameprinter,
            &json!({"read_ms": 0, "lines": [
            {"ok": "GitHub's check of gameprinter passes on main at aa015082"},
            {"problem": "GitHub's check-web of gameprinter fails on main", "under": ["           FAIL x"]},
            {"waiting": "red", "held": "red"}]}),
        );
        let l = health_lines(home.path(), 120_000);
        assert_eq!(
            l[0],
            Ok("OK       GitHub's check of gameprinter passes on main at aa015082 (read 2 min ago)".to_owned())
        );
        assert_eq!(
            l[1],
            Err((
                "GitHub's check-web of gameprinter fails on main".to_owned(),
                vec!["           FAIL x".to_owned()]
            ))
        );
        assert_eq!(l[2], Ok("WAITING  red (read 2 min ago)".to_owned()));
        assert_eq!(held(home.path()), Some(vec!["red".to_owned()]));
        // The games repo's read comes after gameprinter's, and its held
        // failures join gameprinter's.
        keep(
            home.path(),
            Repo::Games,
            &json!({"read_ms": 60_000, "lines": [
            {"ok": "GitHub's check of games passes on main at 0c0ffee1"},
            {"waiting": "games red", "held": "games red"}]}),
        );
        let l = health_lines(home.path(), 120_000);
        assert_eq!(l.len(), 5);
        assert_eq!(
            l[3],
            Ok(
                "OK       GitHub's check of games passes on main at 0c0ffee1 (read 1 min ago)"
                    .to_owned()
            )
        );
        assert_eq!(
            held(home.path()),
            Some(vec!["red".to_owned(), "games red".to_owned()])
        );
        keep(
            home.path(),
            Repo::Games,
            &json!({"read_ms": 0, "unread": "gh auth status failed: not logged in"}),
        );
        assert_eq!(held(home.path()), None, "gh couldn't say of the games repo");
        assert_eq!(
            health_lines(home.path(), 1000)[3],
            Ok("UNREAD   GitHub's checks of the games repo's main (read under a minute ago): gh auth status failed: not logged in".to_owned())
        );
    }

    /// The games repo's check runs a job on a self-hosted runner that was
    /// off for nine days (health-reads-the-games-repo-s-checks): a run
    /// queued on main for over an hour is a PROBLEM naming the job and its
    /// labels, saying the runner is likely off and how to start it, and
    /// its words stay the same from push to push. A run going with no job
    /// waiting, or queued under an hour, is none.
    #[test]
    fn a_run_queued_for_over_an_hour_names_the_job_and_its_runner() {
        let at = |w: &str, id: u64, sha: &str, status: &str, made: &str| {
            let mut r = run(w, id, sha, status, "");
            r["createdAt"] = json!(made);
            r
        };
        // 2026-11-28T12:00:00Z.
        let now = (crate::store::rfc3339_seconds("2026-11-28T12:00:00Z").unwrap() * 1000.0) as u64;
        let runs = json!([
            at("check", 7, "bbbb2222bbbb", "queued", "2026-11-28T11:50:00Z"),
            at("check", 6, "aaaa1111aaaa", "queued", "2026-11-27T09:00:00Z"),
            at(
                "site",
                5,
                "aaaa1111aaaa",
                "in_progress",
                "2026-11-27T09:00:00Z"
            ),
            at(
                "check",
                4,
                "99990000cccc",
                "completed",
                "2026-11-19T08:00:00Z"
            ),
        ]);
        let long = long_runs(&runs, Repo::Games, now);
        assert_eq!(
            long.iter()
                .map(|l| (l.workflow.as_str(), l.id, l.queued))
                .collect::<Vec<_>>(),
            vec![("check", 6, true), ("site", 5, false)],
            "the oldest of each workflow, the one ten minutes old passed over"
        );
        assert_eq!(long[0].waited_ms, 27 * 3_600_000);

        // gh api .../actions/runs/6/jobs: one job ran, the other waits.
        let jobs = json!({"jobs": [
            {"name": "build", "status": "completed", "labels": ["ubuntu-latest"]},
            {"name": "rom", "status": "queued", "labels": ["self-hosted", "rom"]}]});
        let waiting = waiting_jobs(&jobs);
        assert_eq!(
            waiting,
            vec![(
                "rom".to_owned(),
                vec!["self-hosted".to_owned(), "rom".to_owned()]
            )]
        );
        let home = tempfile::tempdir().unwrap();
        let runner = runner_dir(home.path());
        let (words, under) = long_line(&long[0], &waiting, Some(&runner)).unwrap();
        assert_eq!(
            words,
            format!(
                "GitHub's check of games has a run on main queued for over 27 hour(s): its job rom waits for a runner labelled self-hosted, rom. Only a self-hosted runner takes it, and none has: the runner is likely off. Start it on the machine it was set up on with ./run.sh in the runner's folder ({}), and the queued runs go.",
                runner.display()
            )
        );
        assert_eq!(
            under,
            vec![
                "           the oldest such run is on aaaa1111".to_owned(),
                "           https://github.com/o/r/actions/runs/6".to_owned(),
            ]
        );
        assert!(runner.ends_with("runner-games"));
        // A later read, a later push and longer waited: the same trouble
        // row, and the words name no commit for the row to move to.
        let later = Long {
            id: 9,
            sha: "cccc3333cccc".into(),
            waited_ms: 40 * 3_600_000,
            ..long[0].clone()
        };
        let (again, _) = long_line(&later, &waiting, Some(&runner)).unwrap();
        assert_eq!(crate::trouble::key(&words), crate::trouble::key(&again));
        assert!(!words.contains("aaaa1111") && !again.contains("cccc3333"));
        // No runner folder in the home: still how to start it.
        let (bare, _) = long_line(&long[0], &waiting, None).unwrap();
        assert!(
            bare.contains("with ./run.sh in the runner's folder, and"),
            "{bare}"
        );
        // A job waiting on GitHub's own runners says so.
        let hosted = vec![("build".to_owned(), vec!["ubuntu-latest".to_owned()])];
        let (h, _) = long_line(&long[0], &hosted, None).unwrap();
        assert!(h.contains("GitHub's runners haven't taken it"), "{h}");
        assert!(h.contains("in the games checkout"), "{h}");
        // A queued run whose jobs couldn't be read is still a problem.
        let (none, _) = long_line(&long[0], &[], None).unwrap();
        assert!(none.contains(": no job of it has started."), "{none}");
        // A run going whose jobs all run is a long check, not a problem.
        assert_eq!(long_line(&long[1], &[], None), None);

        // A failure in the games repo is a PROBLEM naming that repo, fixed
        // through the model gate, read in the games checkout.
        let red = json!([run("check", 3, "dddd4444dddd", "completed", "failure")]);
        let s = standings(&red, Repo::Games);
        let Err((words, _)) = line(&s[0], &["rom (job rom)".to_owned()], &[], None) else {
            panic!("a failing check is a problem")
        };
        assert!(
            words.starts_with("GitHub's check of games fails on main at rom (job rom):"),
            "{words}"
        );
        assert!(words.contains("Fix it as a games row and land it through the model gate"));
        assert!(words.contains("--log-failed` in the games checkout"));
        assert_eq!(
            line(
                &Standing {
                    failed_in_a_row: 0,
                    ..s[0].clone()
                },
                &[],
                &[],
                None
            ),
            Ok("GitHub's check of games passes on main at dddd4444".to_owned())
        );
    }
}
