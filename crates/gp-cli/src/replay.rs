//! Scoring the workers (issue 53): a replay set made from archived rows,
//! and `gp loop replay`, which runs a worker on one of them again and
//! scores what it makes with the gate's dry run. Nothing lands.
//!
//! The corpus, the gates and the bench score the games. Nothing scored
//! the workers: a change to the brief, the tools a worker is given or the
//! model it runs on was tried in production and judged by feel. Each
//! archived row with a landed commit is a real task with a known good
//! outcome and a gate that says pass or fail, so a set of them, run more
//! than once on each side of a change, gives a pass rate, turns, cost and
//! refusals to compare (docs/worker-evals.md).
//!
//! | File | What |
//! |---|---|
//! | `$GP_HOME/work/loop/replay/set.json` | The set: each task's game, row, repo, kind, landed commit, base commit and the row as a worker first got it |
//! | `$GP_HOME/work/loop/replay/runs/ROW/STAMP-N/` | One trial: its scratch home, brief, log and patch (its tree is removed) |
//! | `$GP_HOME/work/loop/replay/results.jsonl` | One line per trial: the label, the row, pass or why not, turns, cost, refusals |
//!
//! A trial's worker has a scratch home, not the person's: its cards go
//! there, where no runner answers them, and its page is marked served so
//! no call is run on the worker's own word. Its tree is a clone of one
//! branch made at the task's base commit, so nothing that landed after
//! the base is in it: the first real trial, on 2026-11-26, was a worktree
//! of the person's checkout, and its worker found the fix on main with
//! git log and in the checkout with Grep, and changed nothing. The gate's
//! dry run is the real gate's, onto that branch, which is deleted after.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::serve::runner::{self, Run, Setup};

/// The replay's folder in the home.
pub(crate) fn dir(home: &Path) -> PathBuf {
    gp_mcp::steps::loop_dir(home).join("replay")
}

pub(crate) fn set_file(home: &Path) -> PathBuf {
    dir(home).join("set.json")
}

pub(crate) fn results_file(home: &Path) -> PathBuf {
    dir(home).join("results.jsonl")
}

/// The kinds of task a set is spread over.
pub(crate) const KINDS: &[&str] = &["tool", "page", "model"];

/// The kind of an archived row's task: a game's row is a model fix; a
/// tools row is a page fix when its commit changed a file of the page
/// (`files`, the commit's), else a tool fix.
pub(crate) fn kind_of(repo: &str, files: &[String]) -> &'static str {
    if repo != "tools" {
        return "model";
    }
    let page = |f: &String| {
        f.starts_with("editor/")
            || f.starts_with("scripts/web/")
            || f.starts_with("site/")
            || f.ends_with(".js")
            || f.ends_with(".css")
            || f.ends_with(".html")
    };
    if files.iter().any(page) {
        "page"
    } else {
        "tool"
    }
}

/// The words a row's session was started with (`prompt` in its
/// session.toml): what the row was first filed for, which the archive's
/// own fields no longer say once the row has been worked.
pub(crate) fn first_words(home: &Path, game: &str, row: &str) -> Option<String> {
    let f = home
        .join("mechanics")
        .join(game)
        .join(row)
        .join("session.toml");
    let t: toml::Value = toml::from_str(&fs::read_to_string(f).ok()?).ok()?;
    t.get("prompt")?
        .as_str()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
}

/// The row a replayed worker gets, made from archived row `r`: next, with
/// the words the row was first filed with (`asked`, the person's own;
/// else `prompt`, its session's first words; else `what`) and never its
/// last note or its plain sentence, which say what landed. Its target and
/// what would measure it are kept: they are the finish line, not the
/// answer.
pub(crate) fn task_row(r: &Value, prompt: Option<&str>) -> Value {
    let first = [r["asked"].as_str(), prompt, r["what"].as_str()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("");
    let mut row = json!({
        "mechanic": r["mechanic"],
        "status": "next",
        "note": first,
        "repo": r["repo"],
    });
    for k in ["target", "measure_needed", "line"] {
        if !r[k].is_null() {
            row[k] = r[k].clone();
        }
    }
    if crate::steps::no_measure(&row) {
        row["measure_needed"] = json!("the gate's dry run passes on the change");
    }
    row
}

/// Picks `n` tasks of `candidates` spread across their kinds: the newest
/// closed of each kind in turn, until `n` are picked or none is left.
pub(crate) fn pick(mut candidates: Vec<Value>, n: usize) -> Vec<Value> {
    candidates.sort_by_key(|c| std::cmp::Reverse(c["closed_ms"].as_u64().unwrap_or(0)));
    let mut by: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for c in candidates {
        by.entry(c["kind"].as_str().unwrap_or("tool").to_owned())
            .or_default()
            .push(c);
    }
    for v in by.values_mut() {
        v.reverse();
    }
    let mut out = Vec::new();
    while out.len() < n && by.values().any(|v| !v.is_empty()) {
        for k in KINDS {
            if out.len() >= n {
                break;
            }
            if let Some(c) = by.get_mut(*k).and_then(Vec::pop) {
                out.push(c);
            }
        }
        // A kind this file doesn't name is still a task.
        for (k, v) in by.iter_mut() {
            if out.len() < n
                && !KINDS.contains(&k.as_str())
                && let Some(c) = v.pop()
            {
                out.push(c);
            }
        }
    }
    out
}

use crate::serve::work::git_ok as git;

/// Every archived row of the home with a landed commit that `tools` (for a
/// tools row) or `games` still has, as a task: its game, row, repo, kind,
/// commit, the commit before it (the base a worker starts from) and the
/// row a worker gets.
pub(crate) fn candidates(home: &Path, tools: Option<&Path>, games: Option<&Path>) -> Vec<Value> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(home.join("mechanics")) else {
        return out;
    };
    let mut names: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|g| crate::mechanic::session::name_ok(g))
        .collect();
    names.sort();
    for game in names {
        for r in crate::mechanic::archive::read_archive(home, &game) {
            let Some(row) = r["mechanic"].as_str() else {
                continue;
            };
            let repo = if r["repo"] == json!("tools") {
                "tools"
            } else {
                "games"
            };
            let Some(at) = (if repo == "tools" { tools } else { games }) else {
                continue;
            };
            // The commit its card landed, as the archive keeps it; a tools
            // row has none there (on 2026-11-26 all 57 kept were game
            // rows'), so it is the first commit on main whose message names
            // the row, as the tool's commits do: "(row GAME/ROW)".
            let named = || {
                git(
                    at,
                    &[
                        "log",
                        "refs/heads/main",
                        "--reverse",
                        "--format=%H",
                        "-F",
                        "--grep",
                        &format!("(row {game}/{row})"),
                    ],
                )
                .ok()
                .and_then(|o| o.lines().next().map(str::to_owned))
            };
            let Some(commit) = r["landed"]["commit"]
                .as_str()
                .map(str::to_owned)
                .or_else(named)
            else {
                continue;
            };
            let commit = commit.as_str();
            let Ok(base) = git(
                at,
                &["rev-parse", "--verify", "--quiet", &format!("{commit}^")],
            ) else {
                continue;
            };
            let Ok(full) = git(at, &["rev-parse", "--verify", "--quiet", commit]) else {
                continue;
            };
            let files: Vec<String> = git(at, &["show", "--name-only", "--format=", &full])
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect();
            out.push(json!({
                "id": format!("{game}/{row}"),
                "game": game,
                "row": row,
                "repo": repo,
                "kind": kind_of(repo, &files),
                "commit": full,
                "base": base,
                "closed_ms": r["closed_ms"],
                "task": task_row(&r, first_words(home, &game, row).as_deref()),
            }));
        }
    }
    out
}

pub(crate) fn read_set(home: &Path) -> Vec<Value> {
    fs::read(set_file(home))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_set(home: &Path, set: &[Value]) -> Result<()> {
    fs::create_dir_all(dir(home))?;
    fs::write(set_file(home), serde_json::to_string_pretty(set)?)?;
    Ok(())
}

/// The task of the set that `name` names: its `GAME/ROW`, or its row's
/// name when one task alone has it.
pub(crate) fn find<'a>(set: &'a [Value], name: &str) -> Result<&'a Value> {
    if let Some(t) = set.iter().find(|t| t["id"] == json!(name)) {
        return Ok(t);
    }
    let by_row: Vec<&Value> = set.iter().filter(|t| t["row"] == json!(name)).collect();
    match by_row.as_slice() {
        [t] => Ok(t),
        [] => bail!(
            "no task {name} in the replay set ({} tasks; gp loop replay --pick N makes it, and with no row it lists them)",
            set.len()
        ),
        many => bail!(
            "{} tasks have a row named {name}: say which as GAME/ROW ({})",
            many.len(),
            many.iter()
                .filter_map(|t| t["id"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The step a replayed worker is given for `task`, as the quest log's
/// steps are made (crate::steps::row_steps), with an id of its own.
pub(crate) fn step_of(task: &Value, stamp: &str) -> Value {
    let (game, row) = (
        task["game"].as_str().unwrap_or(""),
        task["row"].as_str().unwrap_or(""),
    );
    let tools = task["repo"] == json!("tools");
    let mut cli = crate::steps::ROW_CLI
        .replace("GAME", game)
        .replace("ROW", row);
    if tools {
        cli.push_str(crate::steps::TOOL_ROW_CLI);
    }
    let mut commands = crate::steps::commands_of(&cli);
    commands.push("*".into());
    let note = task["task"]["note"].as_str().unwrap_or("");
    let key = json!({"journey": "replay", "game": game, "row": row, "stamp": stamp});
    let h = blake3::hash(key.to_string().as_bytes()).to_hex();
    json!({
        "id": format!("s{}", &h[..12]),
        "journey": "quest log",
        "subject": game,
        "who": "agent",
        "says": if note.is_empty() {
            format!("Work on {row}.")
        } else {
            format!("Work on {row}: {}", crate::next::cut(note, 200))
        },
        "commands": commands,
        "touches": [format!("row:{game}/{row}")],
        "how": {"cli": cli},
        "repo": if tools { "tools" } else { "games" },
        "target": task["task"]["target"],
        "measure_needed": task["task"]["measure_needed"],
        "no_measure": false,
    })
}

/// What a replay needs beside the task.
pub(crate) struct Rig {
    /// The person's home: the set and the results are there.
    pub home: PathBuf,
    /// The gameprinter checkout and the games checkout.
    pub tools: PathBuf,
    pub games: PathBuf,
    /// The gp that serves the worker and runs the gate's dry run.
    pub gp: PathBuf,
    /// The agent's command, its program found.
    pub argv: Vec<String>,
    pub program: PathBuf,
    pub timeout: Duration,
}

/// Waits for `child` up to `timeout`; past it the child is killed. Its
/// exit code, None when it was killed or ended by a signal.
fn wait(child: &mut std::process::Child, timeout: Duration) -> Option<i32> {
    let t = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.code(),
            Ok(None) if t.elapsed() < timeout => std::thread::sleep(Duration::from_millis(500)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// One trial of `task`: a worker with today's brief and tools on the
/// task's base commit, in a scratch home, then the gate's dry run on what
/// it made. Gives the trial's result line; nothing lands, and the trial's
/// worktree and branch are removed.
pub(crate) fn trial(rig: &Rig, task: &Value, label: &str, n: u32, stamp: &str) -> Result<Value> {
    let (game, row) = (
        task["game"].as_str().context("the task names no game")?,
        task["row"].as_str().context("the task names no row")?,
    );
    let base = task["base"].as_str().context("the task names no base")?;
    let tools_row = task["repo"] == json!("tools");
    let repo = if tools_row { &rig.tools } else { &rig.games };
    let step = step_of(task, &format!("{stamp}-{n}"));
    let id = step["id"].as_str().unwrap_or("").to_owned();
    let run_dir = dir(&rig.home)
        .join("runs")
        .join(format!("{game}-{row}"))
        .join(format!("{stamp}-{n}"));
    let (scratch, tree) = (run_dir.join("home"), run_dir.join("tree"));
    fs::create_dir_all(scratch.join("mechanics").join(game))?;
    // The scratch home: the row as a worker first got it, a page marked
    // served by this process (so a call that needs the person's yes waits
    // on a card no one answers, and is never run on the worker's word),
    // and the worker's record, which makes its step citable.
    fs::write(
        scratch.join("mechanics").join(game).join("queue.json"),
        serde_json::to_string_pretty(&[task["task"].clone()])?,
    )?;
    gp_mcp::asks::mark_page_served(&scratch, 0)?;
    fs::create_dir_all(gp_mcp::steps::workers_dir(&scratch))?;
    fs::write(
        gp_mcp::steps::workers_dir(&scratch).join(format!("{id}.json")),
        json!({"step": step, "state": "working", "worker": format!("replay-{id}"),
            "pid": std::process::id(), "started_ms": gp_mcp::asks::now_ms()})
        .to_string(),
    )?;
    // The tree: a clone of one branch made at the base, sharing the repo's
    // objects, so it has the past and no branch or commit from after it.
    let branch = format!("replay/{id}");
    git(repo, &["branch", "-f", &branch, base])?;
    let cloned = Command::new("git")
        .args(["clone", "-q", "--shared", "--single-branch", "--no-tags"])
        .args(["--branch", &branch])
        .arg(repo)
        .arg(&tree)
        // Pictures kept outside git stay pointers: nothing is fetched.
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .stdin(Stdio::null())
        .output()
        .context("cloning the trial's tree")?;
    if !cloned.status.success() {
        let _ = git(repo, &["branch", "-D", &branch]);
        bail!(
            "git clone of {branch}: {}",
            String::from_utf8_lossy(&cloned.stderr).trim()
        );
    }
    let ran = (|| -> Result<Value> {
        let games = if tools_row { &rig.games } else { &tree };
        // The checkout the row is about is the tree, in what the worker
        // may read too: the person's own is at today's tip, with the fix.
        let setup = Setup {
            home: scratch.clone(),
            games: if tools_row {
                rig.games.clone()
            } else {
                tree.clone()
            },
            repo: tree.clone(),
            tools: if tools_row {
                tree.clone()
            } else {
                rig.tools.clone()
            },
            gp: rig.gp.clone(),
            argv: rig.argv.clone(),
            program: rig.program.clone(),
            timeout: rig.timeout,
            tries: 1,
            again_after: Duration::from_secs(0),
        };
        let mcp = run_dir.join("mcp.json");
        let mut args = vec![
            json!("mcp"),
            json!("--repo"),
            json!(rig.tools),
            json!("--games"),
            json!(rig.games),
        ];
        if !tools_row {
            args.extend([json!("--worker-games"), json!(tree)]);
        }
        let mut env = json!({"GP_HOME": scratch, "GP_GAMES_DIR": games});
        env[gp_mcp::steps::STEP_ENV] = json!(id);
        fs::write(
            &mcp,
            serde_json::to_vec_pretty(&json!({"mcpServers": {"gp": {
                "command": rig.gp, "args": args, "env": env,
            }}}))?,
        )?;
        let brief_path = run_dir.join("brief.txt");
        fs::write(
            &brief_path,
            runner::brief(&setup, &step, &tree, "detached", &Run::default()),
        )?;
        let log_path = run_dir.join("1.log");
        let log = fs::File::create(&log_path)?;
        let claude =
            crate::serve::clip_agent::is_claude(rig.argv.first().map(String::as_str).unwrap_or(""));
        let mut argv: Vec<String> = rig.argv[1..].to_vec();
        if claude {
            argv.extend(runner::claude_args(&setup, &tree, &mcp));
        }
        let mut cmd = Command::new(&rig.program);
        cmd.args(&argv);
        runner::worker_env(&mut cmd);
        cmd.env("GP_HOME", &scratch)
            .env("GP_GAMES_DIR", games)
            .env(gp_mcp::steps::STEP_ENV, &id)
            .env("GP_WORKER", format!("replay-{id}"))
            .env("GP_LOOP_BRIEF", &brief_path)
            .env("GP_LOOP_WORKTREE", &tree)
            .env("GP_GP", &rig.gp)
            .env("GP_NO_WINDOW", "1")
            .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
            .current_dir(&tree)
            .stdin(Stdio::from(fs::File::open(&brief_path)?))
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        let mut child = cmd.spawn().context("starting the worker")?;
        let code = wait(&mut child, rig.timeout);
        let result = crate::worker_runs::result_line(&log_path);
        let ending = if claude {
            crate::worker_runs::ending(result.as_ref())
        } else {
            crate::worker_runs::Ending::Finished
        };
        let summary = result
            .as_ref()
            .map(crate::worker_runs::summary)
            .unwrap_or(Value::Null);
        // What it made: everything in its tree against the base, its own
        // commits and what it left uncommitted, but for the patch files a
        // worker writes in its tree to hand to the gate.
        git(&tree, &["add", "-A", "--", ".", ":(exclude)*.patch"])?;
        let patch = Command::new("git")
            .arg("-C")
            .arg(&tree)
            .args(["diff", "--cached", "--binary", base])
            .output()?
            .stdout;
        let patch_path = run_dir.join("made.patch");
        fs::write(&patch_path, &patch)?;
        let (pass, why) = if patch.is_empty() {
            (false, "the worker changed nothing".to_owned())
        } else {
            let mut gate = Command::new(&rig.gp);
            gate.arg("--json");
            if tools_row {
                gate.args(["tool", "gate"]);
            } else {
                gate.args(["model", "gate", game]);
            }
            gate.arg("--patch")
                .arg(&patch_path)
                .args([
                    "-m",
                    &format!("A replay of {game}/{row}"),
                    "--onto",
                    &branch,
                ])
                .arg("--dry-run")
                .current_dir(repo)
                .env("GP_HOME", &rig.home)
                .env("GP_GAMES_DIR", &rig.games)
                .env("GP_NO_HINTS", "1")
                .env_remove(gp_mcp::steps::STEP_ENV)
                .stdin(Stdio::null());
            let o = gate.output().context("running the gate's dry run")?;
            let said: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
            let why = said["why"]
                .as_str()
                .map(|w| format!("{}: {w}", said["phase"].as_str().unwrap_or("gate")))
                .unwrap_or_else(|| {
                    if o.status.success() {
                        "the gate's dry run passed".to_owned()
                    } else {
                        crate::next::cut(String::from_utf8_lossy(&o.stderr).trim(), 300)
                    }
                });
            (o.status.success(), why)
        };
        Ok(json!({
            "at_ms": gp_mcp::asks::now_ms(),
            "label": label,
            "task": task["id"],
            "kind": task["kind"],
            "trial": n,
            "pass": pass,
            "why": why,
            "exit": code,
            "ended": ending.says().unwrap_or_else(|| "its result says success".to_owned()),
            "result": summary,
            "patch_bytes": patch.len(),
            "dir": run_dir,
        }))
    })();
    // Whatever happened, the trial's tree and branch go.
    let _ = fs::remove_dir_all(&tree);
    let _ = git(repo, &["branch", "-D", &branch]);
    let line = ran?;
    crate::worker_runs::append(&results_file(&rig.home), &line);
    Ok(line)
}

/// The score of result lines `results` (all of one label): how many tasks
/// and trials, the pass rate over trials, the share of tasks that passed
/// every trial, and the mean turns, cost and refusals of the trials that
/// have a result line.
pub(crate) fn score(results: &[Value]) -> Value {
    let mut by_task: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for r in results {
        let e = by_task
            .entry(r["task"].as_str().unwrap_or("?"))
            .or_default();
        e.0 += 1;
        if r["pass"] == json!(true) {
            e.1 += 1;
        }
    }
    let trials = results.len();
    let passed: usize = by_task.values().map(|(_, p)| p).sum();
    let every = by_task.values().filter(|(n, p)| n == p).count();
    let with: Vec<&Value> = results.iter().filter(|r| r["result"].is_object()).collect();
    let mean = |k: &str| {
        (!with.is_empty()).then(|| {
            let s: f64 = with.iter().filter_map(|r| r["result"][k].as_f64()).sum();
            (s / with.len() as f64 * 100.0).round() / 100.0
        })
    };
    let rate =
        |a: usize, b: usize| (b > 0).then(|| (a as f64 / b as f64 * 1000.0).round() / 1000.0);
    json!({
        "tasks": by_task.len(),
        "trials": trials,
        "passed": passed,
        "pass_rate": rate(passed, trials),
        "tasks_passing_every_trial": every,
        "every_trial_rate": rate(every, by_task.len()),
        "turns": mean("turns"),
        "cost_usd": mean("cost_usd"),
        "denials": mean("denials"),
    })
}

/// The scores of every label in `results`, in the order each label first
/// appears, with a line a person reads for each and the table's row for
/// docs/worker-evals.md.
pub(crate) fn report(results: &[Value]) -> Value {
    let mut labels: Vec<&str> = Vec::new();
    for r in results {
        let l = r["label"].as_str().unwrap_or("?");
        if !labels.contains(&l) {
            labels.push(l);
        }
    }
    let pct = |v: &Value| match v.as_f64() {
        Some(x) => format!("{:.1}%", x * 100.0),
        None => "none".to_owned(),
    };
    let num = |v: &Value| match v.as_f64() {
        Some(x) => format!("{x}"),
        None => "none".to_owned(),
    };
    let mut scores = Vec::new();
    let mut lines = vec![
        "| label | tasks | trials | pass rate | pass every trial | turns | cost | refusals |"
            .to_owned(),
        "|---|---|---|---|---|---|---|---|".to_owned(),
    ];
    for l in labels {
        let of: Vec<Value> = results
            .iter()
            .filter(|r| r["label"].as_str().unwrap_or("?") == l)
            .cloned()
            .collect();
        let mut s = score(&of);
        s["label"] = json!(l);
        lines.push(format!(
            "| {l} | {} | {} | {} | {} | {} | {} | {} |",
            s["tasks"],
            s["trials"],
            pct(&s["pass_rate"]),
            pct(&s["every_trial_rate"]),
            num(&s["turns"]),
            match s["cost_usd"].as_f64() {
                Some(c) => format!("${c:.2}"),
                None => "none".to_owned(),
            },
            num(&s["denials"]),
        ));
        scores.push(s);
    }
    if scores.is_empty() {
        lines = vec!["No replay has run: gp loop replay ROW --label NAME runs one.".to_owned()];
    }
    json!({"scores": scores, "lines": lines})
}

/// What `gp loop replay` was asked.
pub(crate) struct Ask<'a> {
    pub row: Option<&'a str>,
    pub pick: Option<usize>,
    pub all: bool,
    pub trials: u32,
    pub label: &'a str,
    pub report: bool,
    pub agent: Option<&'a str>,
}

/// `gp loop replay`: `--pick N` makes the set, `--report` scores the
/// results, a row (or `--all`) runs its trials, and with none of them the
/// set is listed.
pub(crate) fn run(ask: &Ask, paths: &gp_evidence::Paths) -> Result<crate::Outcome> {
    let home = &paths.home;
    // A worker's shell runs gp loop commands as they are; this one starts
    // paid workers, so it is the person's alone.
    if std::env::var_os(gp_mcp::steps::STEP_ENV).is_some_and(|v| !v.is_empty()) {
        bail!("gp loop replay starts workers, so only the person runs it: not a loop worker's");
    }
    let tools = crate::repo::built()
        .filter(|r| r.join(".git").exists())
        .map(|r| runner::tools_checkout(&r));
    let games = crate::roms_cmd::games_dir(None);
    if ask.report {
        let v = report(&crate::worker_runs::lines_of(&results_file(home)));
        let text = v["lines"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(crate::Outcome::ok(v, text));
    }
    if let Some(n) = ask.pick {
        let set = pick(candidates(home, tools.as_deref(), games.as_deref()), n);
        write_set(home, &set)?;
        let mut by: BTreeMap<&str, usize> = BTreeMap::new();
        for t in &set {
            *by.entry(t["kind"].as_str().unwrap_or("?")).or_default() += 1;
        }
        let text = format!(
            "{}The replay set has {} task(s) ({}), in {}",
            if games.is_none() {
                "No games checkout was found (give $GP_GAMES_DIR), so no game's row is in the set. "
            } else {
                ""
            },
            set.len(),
            by.iter()
                .map(|(k, n)| format!("{n} {k}"))
                .collect::<Vec<_>>()
                .join(", "),
            set_file(home).display()
        );
        return Ok(crate::Outcome::ok(json!({"set": set}), text));
    }
    let set = read_set(home);
    let todo: Vec<&Value> = match (ask.row, ask.all) {
        (Some(name), _) => vec![find(&set, name)?],
        (None, true) => set.iter().collect(),
        (None, false) => {
            let mut lines = vec![format!(
                "{} task(s) in the replay set (gp loop replay ROW runs one, --all every one, --pick N makes the set):",
                set.len()
            )];
            for t in &set {
                lines.push(format!(
                    "  {} ({}, from {})",
                    t["id"].as_str().unwrap_or("?"),
                    t["kind"].as_str().unwrap_or("?"),
                    &t["base"].as_str().unwrap_or("?")
                        [..t["base"].as_str().map_or(1, |b| b.len().min(9))]
                ));
            }
            return Ok(crate::Outcome::ok(json!({"set": set}), lines.join("\n")));
        }
    };
    let agent = ask
        .agent
        .map(str::to_owned)
        .or_else(|| std::env::var("GP_CLIP_AGENT").ok())
        .unwrap_or_else(|| "claude".to_owned());
    let argv: Vec<String> = agent.split_whitespace().map(str::to_owned).collect();
    let program = argv
        .first()
        .and_then(|p| crate::serve::clip_agent::find_program(p))
        .with_context(|| format!("the agent command {agent:?} names no program on PATH"))?;
    let rig = Rig {
        home: home.clone(),
        tools: tools.context("no gameprinter checkout: gp was not built from one here")?,
        games: games.context("no games checkout: give $GP_GAMES_DIR")?,
        gp: std::env::current_exe()?,
        argv,
        program,
        timeout: Duration::from_secs(3600),
    };
    let stamp = gp_mcp::asks::now_ms().to_string();
    let mut done = Vec::new();
    let mut lines = Vec::new();
    for t in todo {
        for n in 1..=ask.trials.max(1) {
            let r = match trial(&rig, t, ask.label, n, &stamp) {
                Ok(r) => r,
                Err(e) => json!({"task": t["id"], "trial": n, "label": ask.label,
                    "pass": false, "why": format!("the trial couldn't run: {e:#}")}),
            };
            lines.push(format!(
                "{} trial {n}: {} ({})",
                t["id"].as_str().unwrap_or("?"),
                if r["pass"] == json!(true) {
                    "PASS"
                } else {
                    "FAIL"
                },
                r["why"].as_str().unwrap_or("")
            ));
            done.push(r);
        }
    }
    let ran: BTreeSet<&str> = done.iter().filter_map(|r| r["task"].as_str()).collect();
    let s = score(&done);
    lines.push(format!(
        "{} task(s), {} trial(s) as {:?}: {} passed; gp loop replay --report scores every label",
        ran.len(),
        done.len(),
        ask.label,
        s["passed"]
    ));
    Ok(crate::Outcome::ok(
        json!({"results": done, "score": s}),
        lines.join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue 53: a replayed row is the row as it was first asked, never
    /// with the note that says what landed.
    #[test]
    fn a_replayed_row_has_what_was_asked_and_not_what_landed() {
        let r = json!({
            "mechanic": "tree-count", "status": "done", "repo": "tools",
            "note": "Landed: the count is read from the folder.",
            "asked": "health should count the trees left",
            "target": "trees sprites at least 3 of 4",
            "landed": {"commit": "abc"}, "closed_ms": 5, "worker": "loop-x",
        });
        let t = task_row(&r, Some("the session's first words"));
        assert_eq!(t["status"], "next");
        assert_eq!(t["note"], "health should count the trees left");
        assert_eq!(t["target"], "trees sprites at least 3 of 4");
        assert!(t.get("landed").is_none() && t.get("worker").is_none());
        assert!(!t.to_string().contains("Landed:"));
        // A row with no measure of its own is measured by the dry run, so
        // its worker starts.
        let bare = task_row(
            &json!({"mechanic": "r", "status": "done", "note": "done it", "plain": "It landed."}),
            None,
        );
        assert_eq!(bare["note"], "");
        assert!(
            bare.get("plain").is_none(),
            "the plain sentence says how it ended"
        );
        let filed = task_row(
            &json!({"mechanic": "r", "note": "done it"}),
            Some(" fix the count "),
        );
        assert_eq!(filed["note"], "fix the count");
        assert!(!crate::steps::no_measure(&bare));
        let step = step_of(
            &json!({"game": "g", "row": "tree-count", "repo": "tools", "task": t}),
            "1-1",
        );
        assert_eq!(step["touches"], json!(["row:g/tree-count"]));
        assert_eq!(
            step["says"],
            "Work on tree-count: health should count the trees left"
        );
        assert!(
            step["commands"]
                .as_array()
                .unwrap()
                .contains(&json!("tool gate"))
        );
        let other = step_of(
            &json!({"game": "g", "row": "tree-count", "repo": "tools", "task": {}}),
            "1-2",
        );
        assert_ne!(step["id"], other["id"], "each trial has a step of its own");
    }

    /// Issue 53: the set is spread across kinds, newest first of each.
    #[test]
    fn the_set_is_picked_across_kinds() {
        let c = |id: &str, kind: &str, closed: u64| json!({"id": id, "kind": kind, "closed_ms": closed});
        let all = vec![
            c("t1", "tool", 1),
            c("t2", "tool", 2),
            c("t3", "tool", 3),
            c("t4", "tool", 4),
            c("p1", "page", 1),
            c("m1", "model", 1),
            c("m2", "model", 2),
        ];
        let ids = |v: &[Value]| -> Vec<String> {
            v.iter()
                .map(|x| x["id"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(ids(&pick(all.clone(), 5)), ["t4", "p1", "m2", "t3", "m1"]);
        assert_eq!(pick(all.clone(), 50).len(), 7, "no more than there are");
        assert_eq!(pick(all, 0).len(), 0);
        let f = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(kind_of("games", &f(&["g/model.toml"])), "model");
        assert_eq!(
            kind_of("tools", &f(&["crates/gp-cli/src/trees.rs"])),
            "tool"
        );
        assert_eq!(
            kind_of("tools", &f(&["scripts/web/public/check-page.mjs"])),
            "page"
        );
        assert_eq!(
            kind_of("tools", &f(&["crates/gp-cli/src/mechanic/screen.js"])),
            "page"
        );
    }

    /// Issue 53: the report gives the pass rate, the share of tasks that
    /// pass every trial, turns, cost and refusals for each label.
    #[test]
    fn a_label_is_scored_by_pass_rate_and_cost() {
        let r = |label: &str, task: &str, pass: bool, turns: u64, cost: f64| {
            json!({"label": label, "task": task, "pass": pass,
                "result": {"turns": turns, "cost_usd": cost, "denials": 2}})
        };
        let results = vec![
            r("before", "g/a", true, 10, 1.0),
            r("before", "g/a", false, 30, 3.0),
            r("before", "g/b", true, 20, 2.0),
            r("before", "g/b", true, 20, 2.0),
            json!({"label": "before", "task": "g/c", "pass": false, "result": null}),
            r("after", "g/a", true, 10, 1.0),
        ];
        let before: Vec<Value> = results[..5].to_vec();
        let s = score(&before);
        assert_eq!(s["tasks"], 3);
        assert_eq!(s["trials"], 5);
        assert_eq!(s["passed"], 3);
        assert_eq!(s["pass_rate"], 0.6);
        assert_eq!(s["tasks_passing_every_trial"], 1);
        assert_eq!(s["every_trial_rate"], 0.333);
        assert_eq!(s["turns"], 20.0);
        assert_eq!(s["cost_usd"], 2.0);
        assert_eq!(s["denials"], 2.0);
        let rep = report(&results);
        let lines: Vec<&str> = rep["lines"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            lines[2],
            "| before | 3 | 5 | 60.0% | 33.3% | 20 | $2.00 | 2 |"
        );
        assert_eq!(
            lines[3],
            "| after | 1 | 1 | 100.0% | 100.0% | 10 | $1.00 | 2 |"
        );
        assert!(score(&[])["pass_rate"].is_null());
        assert!(
            report(&[])["lines"][0]
                .as_str()
                .unwrap()
                .starts_with("No replay has run")
        );
    }

    /// The set's tasks are found by GAME/ROW, or by a row's name when one
    /// task alone has it.
    #[test]
    fn a_task_is_named_by_its_row() {
        let set = vec![
            json!({"id": "g/a", "game": "g", "row": "a"}),
            json!({"id": "h/a", "game": "h", "row": "a"}),
            json!({"id": "g/b", "game": "g", "row": "b"}),
        ];
        assert_eq!(find(&set, "g/a").unwrap()["game"], "g");
        assert_eq!(find(&set, "b").unwrap()["id"], "g/b");
        assert!(
            find(&set, "a")
                .unwrap_err()
                .to_string()
                .contains("g/a, h/a")
        );
        assert!(
            find(&set, "none")
                .unwrap_err()
                .to_string()
                .contains("--pick N")
        );
    }

    /// A trial runs a worker on the task's base commit in a scratch home
    /// and leaves no tree or branch behind; a worker that changes nothing
    /// fails it. The stand-in worker is a shell script, never an agent,
    /// and no gate runs for a trial with no change.
    #[test]
    fn a_trial_leaves_no_tree_or_branch_and_a_worker_that_changes_nothing_fails() {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let g = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&repo)
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
        };
        g(&["init", "-q", "-b", "main"]);
        fs::write(repo.join("a.txt"), "a\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-qm", "first"]);
        let base = git(&repo, &["rev-parse", "HEAD"]).unwrap();
        let seen = t.path().join("seen");
        let sh = t.path().join("agent.sh");
        fs::write(
            &sh,
            format!(
                "echo \"$GP_HOME $GP_STEP $(pwd)\" > {}\ncat > /dev/null\n",
                seen.display()
            ),
        )
        .unwrap();
        let rig = Rig {
            home: t.path().join("home"),
            tools: repo.clone(),
            games: repo.clone(),
            gp: PathBuf::from("/nonexistent/gp"),
            argv: vec!["sh".into(), sh.display().to_string()],
            program: PathBuf::from("/bin/sh"),
            timeout: Duration::from_secs(60),
        };
        let task = json!({"id": "g/r", "game": "g", "row": "r", "repo": "tools", "kind": "tool",
            "base": base, "task": {"mechanic": "r", "status": "next", "note": "do it"}});
        let r = trial(&rig, &task, "test", 1, "7").unwrap();
        assert_eq!(r["pass"], false);
        assert_eq!(r["why"], "the worker changed nothing");
        assert_eq!(r["label"], "test");
        // The worker ran in the trial's tree on the base, with the scratch
        // home and a step of its own.
        let said = fs::read_to_string(&seen).unwrap();
        let run_dir = PathBuf::from(r["dir"].as_str().unwrap());
        assert!(
            said.starts_with(&format!("{} s", run_dir.join("home").display())),
            "{said}"
        );
        assert!(said.trim_end().ends_with("/tree"), "{said}");
        assert!(!run_dir.join("tree").exists(), "the tree is removed");
        assert!(run_dir.join("brief.txt").is_file() && run_dir.join("1.log").is_file());
        assert_eq!(
            git(&repo, &["worktree", "list"]).unwrap().lines().count(),
            1
        );
        assert_eq!(git(&repo, &["branch", "--list", "replay/*"]).unwrap(), "");
        // The person's home has the result line and nothing of the row.
        let lines = crate::worker_runs::lines_of(&results_file(&rig.home));
        assert_eq!(lines.len(), 1);
        assert!(!rig.home.join("mechanics").exists());
    }
}
