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
//! | `$GP_HOME/work/loop/replay/runs/ROW/STAMP-N/` | One trial: its scratch home, brief, log and patch (its tree and its worker's build are removed) |
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

/// Whether archived row `r` is a trouble row (crate::trouble): its
/// measure is a gp loop health line read from today's home and GitHub,
/// which a trial can't put back as it was, so its worker finds the trouble
/// cleared and rightly changes nothing. Five of the eleven no-patch trials
/// of baseline-01403e91 were such rows.
pub(crate) fn is_trouble(r: &Value) -> bool {
    r["trouble"].is_object()
        || r["mechanic"]
            .as_str()
            .is_some_and(|m| m.starts_with("trouble-"))
}
/// The first commit on main of the repo at `at` whose message names row
/// `game`/`row`, as the tool's commits do: "(row GAME/ROW)".
fn named_commit(at: &Path, game: &str, row: &str) -> Option<String> {
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
}

/// Why a trial can't do archived row `r` of `game` again from `base` (in
/// `at`, the checkout of its repo; `commit`, its landed commit, and
/// `files`, what that changed), or None when it can. Of the 11 trials of
/// replay baseline-01403e91 that made no patch, 6 were rows like these:
///
/// - a row with no target whose landing put a picture into a theme: an
///   image model made it and the person's look on its card judged it, so
///   a worker can't make it again and no gate can say it passes
///   (theme-tip-money, theme-font-at-sign-and-credits);
/// - a row whose note names a commit of its repo, other than its own,
///   that its base lacks: it closed on a later fix (get-ready-screen's
///   note: "fixed by e5e06ef", drink-far-end's fix; the runner's wake
///   takes `waits_on` off a row whose wait ended, so the note is what is
///   left of it);
/// - a row that waited on another (`waits_on`) whose landing its base
///   lacks, or that never landed (a card still to come). A row waited on
///   in the other repo is there at today's tip, as a trial runs it.
fn left_out(
    home: &Path,
    game: &str,
    r: &Value,
    at: &Path,
    commit: &str,
    base: &str,
    files: &[String],
) -> Option<String> {
    let repo = if r["repo"] == json!("tools") {
        "tools"
    } else {
        "games"
    };
    let picture = |f: &String| f.contains("/themes/") && f.to_lowercase().ends_with(".png");
    if repo == "games" && r["target"].is_null() && files.iter().any(picture) {
        return Some("its landing put a picture into a theme, judged by a look on its card".into());
    }
    let in_base = |c: &str| git(at, &["merge-base", "--is-ancestor", c, base]).is_ok();
    let hex = |w: &&str| {
        (7..=40).contains(&w.len())
            && w.chars().all(|c| c.is_ascii_hexdigit())
            && w.chars().any(|c| c.is_ascii_digit())
            && w.chars().any(|c| c.is_ascii_alphabetic())
    };
    let note = r["note"].as_str().unwrap_or("");
    let words = note.split(|c: char| !c.is_ascii_alphanumeric());
    for w in words.filter(hex) {
        let Ok(c) = git(
            at,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{w}^{{commit}}"),
            ],
        ) else {
            continue;
        };
        if c != commit && !in_base(&c) {
            return Some(format!("it closed on {w}, a commit its base lacks"));
        }
    }
    let w = r["waits_on"].as_str()?;
    let Ok((g, row)) = crate::mechanic::waits::parse(game, w) else {
        return Some(format!("it waited on {w}, which names no row"));
    };
    let waited = crate::mechanic::archive::read_archive(home, &g)
        .into_iter()
        .rev()
        .find(|x| x["mechanic"] == json!(row));
    let waited_repo = match &waited {
        Some(x) if x["repo"] == json!("tools") => "tools",
        _ => "games",
    };
    if waited_repo != repo {
        return None;
    }
    let landed = waited
        .as_ref()
        .and_then(|x| x["landed"]["commit"].as_str().map(str::to_owned))
        .or_else(|| named_commit(at, &g, &row));
    let Some(c) = landed else {
        return Some(format!("it waited on {g}/{row}, which never landed"));
    };
    (!in_base(&c)).then(|| format!("it waited on {g}/{row}, whose landing {c} its base lacks"))
}

/// Puts the pictures the repo at `repo` keeps in Git LFS into the trial's
/// `tree`, cloned with their pointers: the tree takes the repo's own LFS
/// store, as a worktree of it shares it, and checks them out from there,
/// with nothing fetched. Without it a theme row's worker read pointers
/// (gp theme look check read 109 PNGs it couldn't open, replay
/// baseline-01403e91). With git-lfs's filter set in the tree, a real
/// picture is cleaned back to its pointer, so it is no change of the
/// worker's, and a picture the worker makes is stored where the gate's
/// dry run on the repo finds it. Nothing to do where git-lfs isn't
/// installed.
fn lfs_pictures(repo: &Path, tree: &Path) -> Result<()> {
    if git(tree, &["lfs", "version"]).is_err() {
        return Ok(());
    }
    let common = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let store = Path::new(&common).join("lfs");
    git(tree, &["config", "lfs.storage", &store.to_string_lossy()])?;
    git(tree, &["lfs", "install", "--local"])?;
    git(tree, &["lfs", "checkout"])?;
    Ok(())
}

/// Every archived row of the home with a landed commit that `tools` (for a
/// tools row) or `games` still has, as a task: its game, row, repo, kind,
/// commit, the commit before it (the base a worker starts from) and the
/// row a worker gets. A trouble row is left out (is_trouble).
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
            if is_trouble(&r) {
                continue;
            }
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
            let Some(commit) = r["landed"]["commit"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| named_commit(at, &game, row))
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
            if left_out(home, &game, &r, at, &full, &base, &files).is_some() {
                continue;
            }
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
        // Pictures kept outside git are cloned as pointers, and checked
        // out from the repo's own LFS store just after (lfs_pictures).
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
        lfs_pictures(repo, &tree).context("checking out the tree's LFS pictures")?;
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
    // Whatever happened, the trial's tree and branch go, and its worker's
    // cargo target (3 to 4 GB a trial: crate::builds::ended_replay_builds)
    // and module builds, and its record no longer holds its step: its pid
    // is this process, which goes on to the next trial.
    let _ = fs::remove_dir_all(&tree);
    let _ = fs::remove_dir_all(
        crate::builds::target_root(&scratch).join(crate::worker::branch_folder(&branch)),
    );
    let _ = fs::remove_dir_all(scratch.join("work").join("target"));
    let _ = fs::remove_dir_all(scratch.join("work").join("module-build"));
    let record = gp_mcp::steps::workers_dir(&scratch).join(format!("{id}.json"));
    if let Some(mut rec) = fs::read(&record)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        rec["state"] = json!("done");
        rec["ended_ms"] = json!(gp_mcp::asks::now_ms());
        let _ = fs::write(&record, rec.to_string());
    }
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

    /// A trouble row's measure is a health line of today's home and
    /// GitHub, which a trial can't put back, so a pick over an archive that
    /// holds one leaves it out of the set; docs/worker-evals.md says so.
    #[test]
    fn a_pick_leaves_trouble_rows_out_of_the_set() {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let g = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        g(&["init", "-q", "-b", "main"]);
        g(&["commit", "-q", "--allow-empty", "-m", "first"]);
        g(&["commit", "-q", "--allow-empty", "-m", "second"]);
        let landed = git(&repo, &["rev-parse", "HEAD"]).unwrap();
        let home = t.path().join("home");
        fs::create_dir_all(home.join("mechanics").join("g")).unwrap();
        let row = |name: &str, closed: u64| {
            json!({"mechanic": name, "status": "done", "repo": "tools",
                "landed": {"commit": landed}, "closed_ms": closed})
        };
        let mut keyed = row("checkout-health", 3);
        keyed["trouble"] = json!({"kind": "health", "key": "checkout health"});
        fs::write(
            crate::mechanic::archive::archive_file(&home, "g"),
            serde_json::to_string(&[
                row("tree-count", 1),
                row("trouble-card-failed-in-the-last-hour-0e911a", 2),
                keyed,
            ])
            .unwrap(),
        )
        .unwrap();
        let set = pick(candidates(&home, Some(&repo), None), 10);
        let ids: Vec<&str> = set.iter().filter_map(|c| c["id"].as_str()).collect();
        assert_eq!(ids, ["g/tree-count"]);
        let doc = fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/worker-evals.md"
        ))
        .unwrap();
        assert!(
            doc.contains("A trouble row is left out"),
            "docs/worker-evals.md says the set leaves trouble rows out"
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
                "echo \"$GP_HOME $GP_STEP $(pwd)\" > {}\n\
                 mkdir -p \"$GP_HOME/work/target/replay-$GP_STEP/fastdev\"\n\
                 echo built > \"$GP_HOME/work/target/replay-$GP_STEP/fastdev/gp\"\n\
                 mkdir -p \"$GP_HOME/work/module-build/g/target\"\n\
                 echo built > \"$GP_HOME/work/module-build/g/target/g.wasm\"\n\
                 cat > /dev/null\n",
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
        assert!(
            !run_dir.join("home").join("work").join("target").exists(),
            "the worker's build is removed"
        );
        assert!(
            !run_dir
                .join("home")
                .join("work")
                .join("module-build")
                .exists(),
            "its module builds are removed"
        );
        // Its record names this process, which runs on: it says done.
        let recs = gp_mcp::steps::workers(&run_dir.join("home"));
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["state"], "done");
        assert!(!gp_mcp::steps::holds(&recs[0]));
        assert!(run_dir.join("home").join("mechanics").is_dir());
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

    /// The first replay of the whole set, `--trials 2 --label
    /// baseline-01403e91` on the build at 01403e91 (2026-11-27), is in
    /// docs/worker-evals.md: its row of the before and after table has
    /// `gp loop replay --report`'s numbers in place of "no replay has been
    /// run yet", and one paragraph says what stopped the 11 trials that
    /// left no patch, naming each one's trial folder (RUN/1795753219343-N
    /// under replay/runs). It fails listing what the doc is missing.
    #[test]
    fn the_first_baseline_is_in_the_doc() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let doc = std::fs::read_to_string(root.join("docs/worker-evals.md")).unwrap();
        let mut missing = Vec::new();
        if doc.contains("no replay has been run yet") {
            missing.push("the table still says no replay has been run yet".to_string());
        }
        let row = doc.lines().find(|l| {
            l.starts_with('|')
                && l.split('|')
                    .any(|c| c.trim().trim_matches('`') == "baseline-01403e91")
        });
        match row {
            None => missing.push("the table has no row labelled baseline-01403e91".into()),
            Some(row) => {
                // Change | Label | Tasks | Trials | Pass rate | Pass every
                // trial | Turns | Cost | Refusals
                let cells: Vec<&str> = row.split('|').map(str::trim).collect();
                let want = [
                    (3, "30"),
                    (4, "60"),
                    (5, "45.0%"),
                    (6, "36.7%"),
                    (7, "65.87"),
                    (8, "2.28"),
                    (9, "3.25"),
                ];
                for (i, w) in want {
                    if !cells.get(i).is_some_and(|c| c.contains(w)) {
                        missing.push(format!(
                            "the baseline row's column {i} is {:?}, wanted {w}",
                            cells.get(i).copied().unwrap_or("")
                        ));
                    }
                }
            }
        }
        // The 11 trials that ended with no patch, by their run folder's
        // name after the game and the trial's stamp.
        let no_patch = [
            ("trouble-the-home-s-disk-has-gb-free-under-gb-gp-8546ab", 1),
            ("trouble-github-s-check-web-fails-on-main-at-run-0e911a", 1),
            ("trouble-github-s-check-web-fails-on-main-at-run-0e911a", 2),
            ("get-ready-screen", 1),
            ("get-ready-screen", 2),
            ("theme-trophies-at-the-dancers", 1),
            ("theme-trophies-at-the-dancers", 2),
            ("trouble-the-home-s-disk-has-gb-free-under-gb-gp-7e400a", 1),
            ("trouble-the-home-s-disk-has-gb-free-under-gb-gp-7e400a", 2),
            ("theme-tip-money", 2),
            ("theme-font-at-sign-and-credits", 1),
        ];
        let logs: Vec<String> = no_patch
            .iter()
            .map(|(run, n)| format!("-{run}/1795753219343-{n}"))
            .collect();
        let best = doc
            .split("\n\n")
            .map(|p| logs.iter().filter(|l| p.contains(l.as_str())).count())
            .max()
            .unwrap_or(0);
        if best < logs.len() {
            let unnamed: Vec<&str> = logs
                .iter()
                .filter(|l| !doc.contains(l.as_str()))
                .map(|l| l.trim_start_matches('-'))
                .collect();
            missing.push(format!(
                "no paragraph names all 11 no-patch trials' logs (the best names {best}); \
                 not named anywhere: {}",
                unnamed.join(", ")
            ));
        }
        assert!(
            missing.is_empty(),
            "docs/worker-evals.md is missing the first baseline:\n{}",
            missing.join("\n")
        );
    }
    /// Runs git in `repo` as a test's committer; panics when it fails.
    fn git_in(repo: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .stdin(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// Row replay-theme-rows-get-what-they-need: a model trial's
    /// tree has the game's pictures as main has them, never their Git LFS
    /// pointers. The trials of replay baseline-01403e91 cloned the tree
    /// with GIT_LFS_SKIP_SMUDGE, and gp theme look check read 109 PNGs it
    /// couldn't open. The real picture is no change of the worker's: a
    /// worker that touches nothing still changed nothing. Skipped where
    /// git-lfs isn't installed.
    #[test]
    fn a_model_trial_tree_has_the_real_pictures_main_has() {
        let lfs = Command::new("git")
            .args(["lfs", "version"])
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success());
        if !lfs {
            eprintln!("git-lfs isn't installed: skipped");
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("games");
        let pics = repo.join("g").join("themes").join("t").join("theme");
        fs::create_dir_all(&pics).unwrap();
        git_in(&repo, &["init", "-q", "-b", "main"]);
        git_in(&repo, &["lfs", "install", "--local"]);
        fs::write(
            repo.join(".gitattributes"),
            "*.png filter=lfs diff=lfs merge=lfs -text\n",
        )
        .unwrap();
        let png: Vec<u8> = b"\x89PNG\r\n\x1a\n"
            .iter()
            .copied()
            .chain((0..200u8).map(|i| i.wrapping_mul(37)))
            .collect();
        fs::write(pics.join("pic.png"), &png).unwrap();
        fs::write(repo.join("g").join("model.toml"), "a = 1\n").unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "the theme"]);
        // Main's own blob is the pointer: the picture is in LFS.
        let blob = git(&repo, &["show", "HEAD:g/themes/t/theme/pic.png"]).unwrap();
        assert!(blob.starts_with("version https://git-lfs"), "{blob}");
        let base = git(&repo, &["rev-parse", "HEAD"]).unwrap();
        let tools = t.path().join("tools");
        fs::create_dir_all(&tools).unwrap();
        git_in(&tools, &["init", "-q", "-b", "main"]);
        let seen = t.path().join("seen.png");
        let sh = t.path().join("agent.sh");
        fs::write(
            &sh,
            format!(
                "cp \"$(pwd)/g/themes/t/theme/pic.png\" {}\ncat > /dev/null\n",
                seen.display()
            ),
        )
        .unwrap();
        let rig = Rig {
            home: t.path().join("home"),
            tools,
            games: repo.clone(),
            gp: PathBuf::from("/nonexistent/gp"),
            argv: vec!["sh".into(), sh.display().to_string()],
            program: PathBuf::from("/bin/sh"),
            timeout: Duration::from_secs(60),
        };
        let task = json!({"id": "g/r", "game": "g", "row": "r", "repo": "games", "kind": "model",
            "base": base, "task": {"mechanic": "r", "status": "next", "note": "do it"}});
        let r = trial(&rig, &task, "test", 1, "7").unwrap();
        let got = fs::read(&seen).unwrap();
        assert!(
            got == png,
            "the tree's pic.png is not main's picture: {}",
            String::from_utf8_lossy(&got[..got.len().min(80)])
        );
        assert_eq!(r["why"], "the worker changed nothing");
        assert_eq!(r["patch_bytes"], 0);
        assert_eq!(git(&repo, &["branch", "--list", "replay/*"]).unwrap(), "");
    }

    /// Row replay-theme-rows-get-what-they-need: the set leaves out
    /// what a trial can't do again from its base. A row with no target
    /// whose landing put a theme's picture in (judged by the person's look
    /// on its card, from an image model's picture: theme-tip-money,
    /// theme-font-at-sign-and-credits), a row whose note names a later fix
    /// it closed on (as get-ready-screen closed on drink-far-end's), and a
    /// row that waited on another whose landing its base lacks or that
    /// never landed (a card still to come). A row that waited on a landing
    /// its base holds, and whose note names that and its own commit, is
    /// kept.
    #[test]
    fn the_set_leaves_out_a_picture_card_and_a_row_that_waited_past_its_base() {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("games");
        let game = repo.join("g");
        fs::create_dir_all(game.join("themes").join("t").join("theme")).unwrap();
        git_in(&repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join("README"), "games\n").unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "first"]);
        let mut n = 0;
        let mut commit = |files: &[&str]| {
            n += 1;
            for f in files {
                fs::write(game.join(f), format!("{n}\n")).unwrap();
            }
            git_in(&repo, &["add", "."]);
            git_in(&repo, &["commit", "-qm", &format!("change {n}")]);
            git(&repo, &["rev-parse", "HEAD"]).unwrap()
        };
        let c1 = commit(&["model.toml"]);
        let c2 = commit(&["themes/t/theme/tip.png", "themes/t/theme/art.txt"]);
        let c3 = commit(&["model.toml"]);
        let c4 = commit(&["model.toml"]);
        let c5 = commit(&["model.toml"]);
        let c6 = commit(&["model.toml"]);
        let home = t.path().join("home");
        fs::create_dir_all(home.join("mechanics").join("g")).unwrap();
        let row = |m: &str, c: Option<&String>, closed: u64| {
            let mut r = json!({"mechanic": m, "status": "done", "closed_ms": closed,
                "note": "Landed on your YES."});
            if let Some(c) = c {
                r["landed"] = json!({"card": format!("card-{m}"), "commit": c});
            }
            r
        };
        let mut picture = row("theme-pic", Some(&c2), 2);
        picture["measure_needed"] =
            json!("a theme picture has no scoreboard line: it is done when its card lands");
        let mut past = row("fixed-later", Some(&c3), 3);
        past["target"] = json!("level 1 sprites at least 10 of 10");
        past["waits_on"] = json!("g/later-fix");
        // A commit's short name as a note gives it, with a digit and a
        // letter in it (the note's all-digit words are no commit's).
        let short = |c: &str| -> String {
            let n = (7..=c.len())
                .find(|&n| {
                    let s = &c[..n];
                    s.chars().any(|x| x.is_ascii_digit())
                        && s.chars().any(|x| x.is_ascii_alphabetic())
                })
                .unwrap_or(c.len());
            c[..n].to_owned()
        };
        let mut held = row("waited-in-base", Some(&c5), 5);
        held["waits_on"] = json!("plain-fix");
        // A note that names a commit its base holds, and its own commit.
        held["note"] = json!(format!(
            "Its wait is over: plain-fix landed ({}). Landed as {}.",
            short(&c1),
            short(&c5)
        ));
        let mut card = row("waited-on-a-card", Some(&c6), 6);
        card["waits_on"] = json!("g/never-landed");
        // Its wait gone, its note names the later fix it closed on.
        let mut closed = row("closed-on-a-later-fix", Some(&c1), 8);
        closed["note"] = json!(format!(
            "Its split at frame 419 was later-fix's, fixed by {}.",
            short(&c4)
        ));
        let archive = vec![
            row("plain-fix", Some(&c1), 1),
            picture,
            past,
            row("later-fix", Some(&c4), 4),
            held,
            card,
            row("never-landed", None, 7),
            closed,
        ];
        fs::write(
            crate::mechanic::archive::archive_file(&home, "g"),
            serde_json::to_string_pretty(&archive).unwrap(),
        )
        .unwrap();
        let got: BTreeSet<String> = candidates(&home, None, Some(&repo))
            .iter()
            .filter_map(|c| c["row"].as_str().map(str::to_owned))
            .collect();
        let want: BTreeSet<String> = ["plain-fix", "later-fix", "waited-in-base"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(got, want);
    }
}
