//! GitHub's checks of main, for `gp loop health`: the newest finished run
//! of each workflow on main, read with `gh` in the checkout gp was built
//! from, and a PROBLEM for each that failed, naming its step and its first
//! FAIL line. Until this, check-web failed at mechanic-page on three pushes
//! (7a2abdff, b8d68308, aa015082, 2026-11-25) before anyone looked: `just
//! check` runs no browser check and nothing in the loop read GitHub.
//!
//! The read goes to GitHub at most every [`EVERY_MS`] and is kept in
//! `$GP_HOME/work/loop/github-checks.json`; gp serve's runner reads it in
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

/// The kept read's file.
pub(crate) fn file(home: &Path) -> PathBuf {
    gp_mcp::steps::loop_dir(home).join("github-checks.json")
}

/// One workflow's standing on main: its newest settled run, and how many
/// settled runs in a row up to it failed (0 when it passed).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Standing {
    pub workflow: String,
    pub id: u64,
    pub sha: String,
    pub url: String,
    pub failed_in_a_row: usize,
    /// The commit of a run of the workflow still going (queued or in
    /// progress) newer than this one, when there is one.
    pub running: Option<String>,
}

/// Each workflow's standing from `gh run list --json` (newest first), in
/// the order their newest settled runs come.
pub(crate) fn standings(runs: &Value) -> Vec<Standing> {
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

/// The line of a failed run's log (`gh run view ID --log-failed`, each line
/// `JOB<tab>STEP<tab>TIME TEXT`) that says why: the first that starts with
/// FAIL, else the first error GitHub marked (`##[error]`), with the job,
/// step, time and terminal colours taken off.
pub(crate) fn why_line(log: &str) -> Option<String> {
    let texts: Vec<String> = log.lines().map(log_text).collect();
    let fail = texts.iter().find(|t| t.starts_with("FAIL"));
    let err = || {
        texts.iter().find_map(|t| {
            t.strip_prefix("##[error]")
                .map(str::trim)
                .map(str::to_owned)
        })
    };
    fail.cloned()
        .or_else(err)
        .filter(|t| !t.is_empty())
        .map(|t| crate::next::cut(&t, 200))
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
pub(crate) fn line(
    s: &Standing,
    steps: &[String],
    why: Option<&str>,
) -> Result<String, (String, Vec<String>)> {
    let at = &s.sha[..s.sha.len().min(8)];
    if s.failed_in_a_row == 0 {
        return Ok(format!("GitHub's {} passes on main at {at}", s.workflow));
    }
    let step = if steps.is_empty() {
        String::new()
    } else {
        format!(" at {}", steps.join(", "))
    };
    let words = format!(
        "GitHub's {} fails on main{step}: its last {} finished run(s) failed, the newest on {at}. `just check` runs no such check, so fix it as a tools row and land it through the tool gate; `gh run view {} --log-failed` in the gameprinter checkout prints the whole failure.",
        s.workflow, s.failed_in_a_row, s.id
    );
    let mut under = Vec::new();
    if let Some(w) = why {
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
        "GitHub's {} failed on main at {at}, and a newer run{on} is still going: health waits for it to end",
        s.workflow
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

/// Reads GitHub now: the read as kept in [`file`].
pub(crate) fn read_now(repo: &Path, now: u64) -> Value {
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
            "databaseId,workflowName,headSha,status,conclusion,url",
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
    for s in standings(&runs) {
        let (steps, why) = if s.failed_in_a_row > 0 {
            let id = s.id.to_string();
            let steps = gh_out(repo, &["run", "view", &id, "--json", "jobs"])
                .ok()
                .and_then(|o| serde_json::from_str::<Value>(&o).ok())
                .map(|j| failed_steps(&j))
                .unwrap_or_default();
            let why = gh_out(repo, &["run", "view", &id, "--log-failed"])
                .ok()
                .and_then(|l| why_line(&l));
            (steps, why)
        } else {
            (Vec::new(), None)
        };
        // A failure with a newer run going is held back until that run ends.
        lines.push(match (waiting_line(&s), line(&s, &steps, why.as_deref())) {
            (_, Ok(l)) => json!({"ok": l}),
            (Some(w), Err((p, _))) => json!({"waiting": w, "held": p}),
            (None, Err((p, under))) => json!({"problem": p, "under": under}),
        });
    }
    json!({"read_ms": now, "lines": lines})
}

fn kept(home: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(file(home)).ok()?).ok()
}

fn fresh(home: &Path, now: u64) -> bool {
    kept(home)
        .and_then(|v| v["read_ms"].as_u64())
        .is_some_and(|t| now.saturating_sub(t) < EVERY_MS)
}

fn keep(home: &Path, v: &Value) {
    let f = file(home);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = f.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &f);
    }
}

/// Reads GitHub when the kept read is older than [`EVERY_MS`] (`gp loop
/// health` by hand, before its checks).
pub(crate) fn refresh(home: &Path, repo: &Path) {
    let now = gp_mcp::asks::now_ms();
    if crate::trouble::live_home(home) && !fresh(home, now) {
        keep(home, &read_now(repo, now));
    }
}

static READING: AtomicBool = AtomicBool::new(false);

/// [`refresh`] on a thread of its own, one at a time (gp serve's runner,
/// whose heartbeat mustn't wait on GitHub).
pub(crate) fn refresh_behind(home: &Path, repo: &Path) {
    let now = gp_mcp::asks::now_ms();
    if !crate::trouble::live_home(home) || fresh(home, now) || READING.swap(true, Ordering::SeqCst)
    {
        return;
    }
    let (home, repo) = (home.to_path_buf(), repo.to_path_buf());
    std::thread::spawn(move || {
        keep(&home, &read_now(&repo, gp_mcp::asks::now_ms()));
        READING.store(false, Ordering::SeqCst);
    });
}

/// The failures health holds back while a newer run goes (their PROBLEM
/// words, for crate::trouble to keep their rows open), or None when the
/// kept read can't say: there is none yet, or gh couldn't read GitHub.
pub(crate) fn held(home: &Path) -> Option<Vec<String>> {
    let v = kept(home)?;
    if v["unread"].is_string() {
        return None;
    }
    let held = v["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["held"].as_str())
        .map(str::to_owned)
        .collect();
    Some(held)
}

/// Health's lines from the kept read: Ok lines, Err problems with the
/// lines that go under each, and an UNREAD line when gh couldn't say.
pub(crate) fn health_lines(home: &Path, now: u64) -> Vec<Result<String, (String, Vec<String>)>> {
    let Some(v) = kept(home) else {
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
            "UNREAD   GitHub's checks of main ({read}): {why}"
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
        let s = standings(&runs);
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
        let why = why_line(log);
        assert_eq!(why.as_deref(), Some("FAIL timed out: the game's page"));
        let Err((words, under)) = line(&s[0], &steps, why.as_deref()) else {
            panic!("a failing check is a problem");
        };
        assert!(words.starts_with("GitHub's check-web fails on main at mechanic-page (job web): its last 3 finished run(s) failed, the newest on aa015082."), "{words}");
        assert_eq!(under[0], "           FAIL timed out: the game's page");
        assert_eq!(
            line(&s[1], &[], None),
            Ok("GitHub's check passes on main at aa015082".to_owned())
        );
        // The next push failing the same way is the same trouble row.
        let later = Standing {
            id: 12,
            sha: "0c0ffee12345".into(),
            failed_in_a_row: 4,
            ..s[0].clone()
        };
        let Err((again, _)) = line(&later, &steps, None) else {
            panic!()
        };
        assert_eq!(crate::trouble::key(&words), crate::trouble::key(&again));
        // With no FAIL line, GitHub's own error says why.
        assert_eq!(
            why_line(
                "j\ts\t2026-11-25T10:01:01.1234567Z ##[error]Process completed with exit code 1."
            )
            .as_deref(),
            Some("Process completed with exit code 1.")
        );
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
        let s = standings(&runs);
        assert_eq!(s[0].running.as_deref(), Some("b32f4590ffff"));
        assert_eq!(s[1].running, None, "its run going is older");
        let w = waiting_line(&s[0]).unwrap();
        assert!(w.starts_with("GitHub's check-web failed on main at aa015082"));
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
        refresh(home.path(), repo.path());
        assert!(!file(home.path()).exists());
        keep(
            home.path(),
            &json!({"read_ms": 0, "lines": [
            {"ok": "GitHub's check passes on main at aa015082"},
            {"problem": "GitHub's check-web fails on main", "under": ["           FAIL x"]},
            {"waiting": "red", "held": "red"}]}),
        );
        let l = health_lines(home.path(), 120_000);
        assert_eq!(
            l[0],
            Ok("OK       GitHub's check passes on main at aa015082 (read 2 min ago)".to_owned())
        );
        assert_eq!(
            l[1],
            Err((
                "GitHub's check-web fails on main".to_owned(),
                vec!["           FAIL x".to_owned()]
            ))
        );
        assert_eq!(l[2], Ok("WAITING  red (read 2 min ago)".to_owned()));
        assert_eq!(held(home.path()), Some(vec!["red".to_owned()]));
        keep(
            home.path(),
            &json!({"read_ms": 0, "unread": "gh auth status failed: not logged in"}),
        );
        assert_eq!(held(home.path()), None, "gh couldn't say");
        assert_eq!(
            health_lines(home.path(), 1000),
            vec![Ok("UNREAD   GitHub's checks of main (read under a minute ago): gh auth status failed: not logged in".to_owned())]
        );
    }
}
