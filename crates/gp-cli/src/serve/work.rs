//! The work record (game-printer's serve/work.rs; docs/agents.md, "Working
//! on a clip in the loop"): what Claude does on a run's clips, written as
//! it happens so the REFINE tab's queue and the clip's card show it.
//! Anyone can write to it: the driver (the coordinator in its own lane
//! worktrees) and the clip agent `gp serve` starts on a clip
//! (`serve/clip_agent.rs`). Each entry says who wrote it.
//!
//! An entry is a note ("reading the barrel's roll"), an attempt (a change
//! tried in a worktree, with the clip's score before and after, the other
//! active clips better and worse, the curve and a trial preview), an
//! approval (the person pressed keep), or a done (the work on the clip
//! stopped). The record is the run's `agent/work.jsonl`, one entry a line,
//! appended; an attempt's scores, curve, model and preview are in
//! `agent/<n>/`. An approved attempt is also written to
//! `$GP_HOME/work/agents/approved.json` for the driver, which merges its
//! branch into main and restarts the server.
//!
//! What gameprinter's layout changed:
//!
//! - The run's folder is `$GP_HOME/work/serve/GAME/`, so the record is
//!   `$GP_HOME/work/serve/GAME/agent/work.jsonl`.
//! - The clips scored are the run's active clips (the background work's
//!   pool: played ones and the corpus's, less the skipped), scored as the
//!   background work scores them (`gp clip score`'s scorer). A score is
//!   frames together in thousandths, higher better, where game-printer's
//!   was a distance, lower better.
//! - The game's folder is in the games checkout, a repo of its own, and
//!   the code is gameprinter's, so an agent changes the model in a copy of
//!   the run's working copy (`--model-from DIR`; the clip agent's is
//!   `$GP_HOME/work/agents/RUN/CLIP.model/`, with the copy it started from
//!   beside it as `CLIP.model.base/`) and the code in its worktree. The
//!   model scored is the run's working copy with the copy's changes merged
//!   in file by file (`git merge-file`, base the copy it started from), so
//!   the numbers the queue tuned stay.
//! - The trial preview is built with the worktree's
//!   `scripts/web/build-preview.sh`, gameprinter's preview of the model.
//! - The person's tells (game-printer's serve/tell.rs) aren't here: the
//!   mechanic page's talk (`gp mechanic say`) is where a person tells an
//!   agent about a clip.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/agent` | The record: who is working on what, the notes and the attempts |
//! | `GET /api/capture/{run}/agent/attempts/{n}` | One attempt in full: each clip's score before and after |
//! | `GET /api/capture/{run}/agent/attempts/{n}/curve` | The attempt's curves, before and after |
//! | `GET /api/capture/{run}/agent/attempts/{n}/preview/{*file}` | A file of the attempt's trial preview |
//! | `POST /api/capture/{run}/agent/approve` | `{attempt}`: keep this change |
//!
//! `gp agent note`, `attempt`, `done`, `approve`, `state` and `score`
//! write and read the same record (`crate::agent_cmd`).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::game::{Run, run_dir};
use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/agent/approve",
            about: "POST {attempt}: keep an attempt that made its clip better and no active clip worse; written to $GP_HOME/work/agents/approved.json for the driver to merge",
            handler: approve,
        },
        Route {
            path: "/api/capture/{run}/agent/attempts/{n}/curve",
            about: "An attempt's curves: the first wrong measure's original and rebuilt values, before and after the change",
            handler: curve,
        },
        Route {
            path: "/api/capture/{run}/agent/attempts/{n}/preview/{*file}",
            about: "A file of an attempt's trial preview: the game rebuilt from the attempt's model and code",
            handler: preview,
        },
        Route {
            path: "/api/capture/{run}/agent/attempts/{n}",
            about: "One attempt in full: what it changed, each active clip's score before and after",
            handler: attempt,
        },
        Route {
            path: "/api/capture/{run}/agent",
            about: "The run's work record: who is working on which clip, what on, the notes and the attempts",
            handler: state,
        },
    ]
}

/// The approved file's version.
const APPROVED_SCHEMA: u32 = 1;

/// Who wrote an entry: `driver` or `clip agent`.
pub(crate) fn parse_by(by: &str) -> anyhow::Result<&'static str> {
    match by.trim().to_ascii_lowercase().trim_start_matches("the ") {
        "driver" => Ok("driver"),
        "clip agent" | "clip-agent" | "agent" => Ok("clip agent"),
        other => bail!("--by is `driver` or `clip agent`, not {other:?}"),
    }
}

/// One line of `agent/work.jsonl`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// `note`, `attempt`, `approved`, `done` or `tell` (the person's note
    /// on a clip, `serve/tell.rs`).
    pub kind: String,
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// What it's working on, in plain words: "the barrel (x parts at frame 9)".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// The attempt an `attempt` or `approved` entry is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u64>,
    /// A `tell`'s number (`serve/tell.rs`), counted from 1 in the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// The tell a `note` answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<u64>,
    /// A `tell`'s frame of the clip, and of the original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_frame: Option<u64>,
    /// Where on the screen a `tell` points, in game pixels: a spot `[x, y]`
    /// or a region `[x, y, w, h]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<[u32; 4]>,
    /// A `tell`'s clip score when it was told, as the background work
    /// kept it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Value>,
    pub when_ms: u64,
}

/// A clip's score in an attempt, as the card shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Side {
    pub permille: u64,
    pub score: u64,
    pub exact: bool,
    /// The clip's first wrong frame.
    pub first_wrong: Option<u64>,
    pub first_path: Option<String>,
    /// That frame in words.
    pub says: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Side {
    /// From a score as the background work keeps it.
    pub fn of(s: &Value) -> Self {
        Self {
            permille: s["permille"].as_u64().unwrap_or(0),
            score: s["score"].as_u64().unwrap_or(0),
            exact: s["exact"] == json!(true),
            first_wrong: s["drift"].as_u64(),
            first_path: s["drift_path"].as_str().map(str::to_owned),
            says: s["says"].as_str().unwrap_or("").to_owned(),
            error: s["error"].as_str().map(str::to_owned),
        }
    }
}

/// One attempt, `agent/<n>/attempt.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Attempt {
    pub n: u64,
    pub by: String,
    pub clip: String,
    #[serde(default)]
    pub clip_name: String,
    pub title: String,
    #[serde(default)]
    pub says: String,
    pub before: Option<Side>,
    pub after: Option<Side>,
    /// The other active clips that got better and worse, by ID.
    #[serde(default)]
    pub better: Vec<String>,
    #[serde(default)]
    pub worse: Vec<String>,
    #[serde(default)]
    pub others: usize,
    /// The trial preview's folder under the run, `agent/<n>/preview`.
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_error: Option<String>,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    #[serde(default)]
    pub dirty: bool,
    #[serde(default)]
    pub shape: bool,
    /// How the model scored was made, in words.
    #[serde(default)]
    pub model_says: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sync: String,
    #[serde(default)]
    pub squash: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squash_ref: Option<String>,
    #[serde(default)]
    pub approved: bool,
    pub when_ms: u64,
}

impl Attempt {
    /// Better on its clip and no active clip worse: what keep needs.
    pub fn keepable(&self) -> bool {
        let (Some(b), Some(a)) = (&self.before, &self.after) else {
            return false;
        };
        rank_side(a) < rank_side(b) && self.worse.is_empty()
    }
}

/// Lower is better: one that can't be scored last, then by frames apart.
fn rank(s: &Value) -> (u8, u64) {
    if !s["error"].is_null() {
        (1, u64::MAX)
    } else if s["exact"] == json!(true) {
        (0, 0)
    } else {
        (0, 1001 - s["permille"].as_u64().unwrap_or(0).min(1000))
    }
}

fn rank_side(s: &Side) -> (u8, u64) {
    if s.error.is_some() {
        (1, u64::MAX)
    } else if s.exact {
        (0, 0)
    } else {
        (0, 1001 - s.permille.min(1000))
    }
}

/// Held while a record or the approved file is written by this process.
static WRITE: Mutex<()> = Mutex::new(());

/// The run's `agent/` folder.
pub(crate) fn folder(run: &Run) -> PathBuf {
    run.dir.join("agent")
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Appends one entry, one short line written at once, so a CLI and the
/// server can both append; the page hears it as an `agent.<kind>` event.
pub(crate) fn append(run: &Run, mut e: Entry) -> anyhow::Result<Entry> {
    {
        let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
        if e.when_ms == 0 {
            e.when_ms = now_ms();
        }
        if e.kind == "tell" && e.id.is_none() {
            e.id = Some(entries(run).iter().filter_map(|x| x.id).max().unwrap_or(0) + 1);
        }
        fs::create_dir_all(folder(run))?;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(folder(run).join("work.jsonl"))?;
        f.write_all(format!("{}\n", serde_json::to_string(&e)?).as_bytes())?;
    }
    super::background::emit(&run.name, &format!("agent.{}", e.kind), json!(e));
    Ok(e)
}

pub(crate) fn entries(run: &Run) -> Vec<Entry> {
    fs::read_to_string(folder(run).join("work.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn attempt_dir(run: &Run, n: u64) -> PathBuf {
    folder(run).join(n.to_string())
}

pub(crate) fn read_attempt(run: &Run, n: u64) -> Option<Attempt> {
    fs::read(attempt_dir(run, n).join("attempt.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

pub(crate) fn write_attempt(run: &Run, a: &Attempt) -> anyhow::Result<()> {
    let d = attempt_dir(run, a.n);
    fs::create_dir_all(&d)?;
    let tmp = d.join(".attempt.json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(a)?)?;
    fs::rename(&tmp, d.join("attempt.json"))?;
    let mut v = json!(a);
    v["line"] = json!(attempt_line(a));
    v["keepable"] = json!(a.keepable());
    super::background::emit(&run.name, "agent.attempt", v);
    Ok(())
}

/// The run's attempts, by number.
pub(crate) fn attempts(run: &Run) -> Vec<Attempt> {
    let mut out: Vec<Attempt> = fs::read_dir(folder(run))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
        .filter_map(|n| read_attempt(run, n))
        .collect();
    out.sort_by_key(|a| a.n);
    out
}

/// The next attempt's number, its folder made so no other writer takes it.
fn claim(run: &Run) -> anyhow::Result<u64> {
    fs::create_dir_all(folder(run))?;
    let mut n = attempts(run).last().map_or(0, |a| a.n) + 1;
    loop {
        match fs::create_dir(attempt_dir(run, n)) {
            Ok(()) => return Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e.into()),
        }
    }
}

/// A percent from thousandths.
fn pct(permille: u64) -> u64 {
    (permille + 5) / 10
}

/// One attempt in a line: "Attempt 2: the barrel rolls off the edge.
/// Clip: 62% -> 81%; others: 3 better, none worse."
pub(crate) fn attempt_line(a: &Attempt) -> String {
    let mut s = format!("Attempt {}: {}.", a.n, a.title.trim_end_matches('.'));
    match (&a.before, &a.after) {
        (Some(b), Some(x)) => {
            let side = |s: &Side| {
                if s.error.is_some() {
                    "can't start".to_owned()
                } else if s.exact {
                    "matches".to_owned()
                } else {
                    format!("{}%", pct(s.permille))
                }
            };
            s.push_str(&format!(" Clip: {} -> {}", side(b), side(x)));
            if a.others > 0 {
                let n = |k: usize| {
                    if k == 0 {
                        "none".to_owned()
                    } else {
                        k.to_string()
                    }
                };
                s.push_str(&format!(
                    "; others: {} better, {} worse",
                    n(a.better.len()),
                    n(a.worse.len())
                ));
            }
            s.push('.');
        }
        _ if a.model_says.starts_with("It failed") => {
            s.push(' ');
            s.push_str(&a.model_says);
        }
        _ if a.model_says.starts_with("Not scored") || a.when_ms == 0 => s.push_str(" Not scored."),
        _ => s.push_str(" Building it and scoring the clips with the change now."),
    }
    s
}

/// What one writer is doing now, from its entries since it last said done.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Working {
    pub by: String,
    pub clip: Option<String>,
    pub on: Option<String>,
    pub last: Option<String>,
    pub when_ms: u64,
    /// "Claude is working on: the barrel (x parts at frame 9). Last: found
    /// the cause in the roll routine."
    pub says: String,
}

pub(crate) fn working(es: &[Entry], atts: &[Attempt]) -> Vec<Working> {
    let mut by: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for e in es {
        let v = by.entry(e.by.as_str()).or_default();
        if e.kind == "done" {
            v.clear();
        } else if e.kind != "approved" {
            v.push(e);
        }
    }
    by.into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(who, v)| {
            let on = v.iter().rev().find_map(|e| e.on.clone());
            let clip = v.iter().rev().find_map(|e| e.clip.clone());
            let last = v.iter().rev().find_map(|e| match e.kind.as_str() {
                "note" if !e.text.is_empty() => Some(e.text.clone()),
                "attempt" => e
                    .attempt
                    .and_then(|n| atts.iter().find(|a| a.n == n))
                    .map(attempt_line)
                    .or_else(|| Some(e.text.clone())),
                _ => None,
            });
            let when_ms = v.last().map_or(0, |e| e.when_ms);
            let mut says = match &on {
                Some(on) => format!("Claude is working on: {}.", on.trim_end_matches('.')),
                None => "Claude is working on this.".to_owned(),
            };
            if let Some(l) = &last {
                says.push_str(&format!(" Last: {}", l.trim_end_matches('.')));
                says.push('.');
            }
            Working {
                by: who.to_owned(),
                clip,
                on,
                last,
                when_ms,
                says,
            }
        })
        .collect()
}

/// The record as the page reads it.
pub(crate) fn state_json(run: &Run) -> Value {
    let es = entries(run);
    let atts = attempts(run);
    let lines: Vec<Value> = atts
        .iter()
        .map(|a| {
            let mut v = json!(a);
            v["line"] = json!(attempt_line(a));
            v["keepable"] = json!(a.keepable());
            v
        })
        .collect();
    let notes: Vec<&Entry> = es.iter().rev().take(200).collect();
    // The clip agents working now each have a row of their own.
    let rows = super::clip_agent::working_rows(run);
    let mut who = working(&es, &atts);
    if !rows.is_empty() {
        who.retain(|w| w.by != "clip agent");
        who.extend(rows);
    }
    json!({
        "run": run.name,
        "working": who,
        "attempts": lines,
        "entries": notes.into_iter().rev().collect::<Vec<_>>(),
        "clip_agent": super::clip_agent::status_json(run),
        "clip_agents": super::clip_agent::live_json(run),
        "fix_ready": fix_ready_json(run),
    })
}

// ---------------------------------------------------------------- scoring

/// The active clips scored on a model folder with this build.
pub(crate) struct Scored {
    pub model: String,
    pub scores: BTreeMap<String, Value>,
    /// The focus clip's first wrong measure, frame by frame.
    pub curve: Value,
}

/// Scores every active clip of `run` on `model` (the run's model as it
/// stands when none), each clip's files under `root`, on the helper
/// threads. `reuse` scores already had on the same model are taken as
/// they are, but for the focus clip.
pub(crate) fn score_clips(
    ctx: &Ctx,
    run: &Run,
    model: Option<&Path>,
    root: &Path,
    focus: Option<&str>,
    reuse: &Value,
) -> anyhow::Result<Scored> {
    let dir = model.map_or_else(|| run.model_dir(), Path::to_owned);
    let mh = super::background::model_key_of(ctx, run, &dir);
    let clips = super::background::active_now(ctx, run);
    let mut scores = BTreeMap::new();
    let mut todo = Vec::new();
    for c in &clips {
        match reuse.get(&c.id) {
            Some(s) if s["model"].as_str() == Some(mh.as_str()) && focus != Some(&c.id) => {
                scores.insert(c.id.clone(), s.clone());
            }
            _ => todo.push(c.id.clone()),
        }
    }
    let rb = run.rebuild(ctx);
    let open = || {
        crate::clip_score::Ctx::open(
            &ctx.paths,
            &run.game,
            &crate::clip_score::Open {
                module: rb.module.as_deref(),
                model: Some(&dir),
                assets: None,
                rom: None,
            },
        )
    };
    // Opened once here so a model that can't open says why at once.
    open().context("opening the model to score")?;
    let each = crate::pool::map(
        &todo,
        || open().ok(),
        |sc, _, id| match sc {
            Some(sc) => super::background::score_clip_in(ctx, run, sc, id, &mh, root),
            None => json!({"model": mh, "score": 0, "permille": 0, "exact": false,
                           "error": "couldn't open the model", "says": "can't score it"}),
        },
        &mut |_| {},
    );
    for (id, s) in todo.iter().zip(each) {
        scores.insert(id.clone(), s);
    }
    let curve = focus
        .and_then(|f| scores.get(f))
        .map(|s| {
            json!({"clip": focus, "model": mh, "drift": s["drift"], "drift_path": s["drift_path"],
                   "tracks": s["tracks"], "says": s["says"]})
        })
        .unwrap_or(Value::Null);
    Ok(Scored {
        model: mh,
        scores,
        curve,
    })
}

/// `gp agent score`'s JSON: what an attempt reads from a worktree's build.
pub(crate) fn scored_json(s: &Scored) -> Value {
    json!({"model": s.model, "scores": s.scores, "curve": s.curve})
}

fn scored_from_json(v: &Value) -> anyhow::Result<Scored> {
    Ok(Scored {
        model: v["model"].as_str().unwrap_or_default().to_owned(),
        scores: serde_json::from_value(v["scores"].clone())
            .context("the worktree's gp agent score gave no scores")?,
        curve: v["curve"].clone(),
    })
}

// ------------------------------------------------------------- git

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// `git`, with its error when it fails.
pub(crate) fn git_ok(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !o.status.success() {
        bail!(
            "git {} failed: {}{}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim(),
            String::from_utf8_lossy(&o.stdout).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(r) = p.strip_prefix(root) {
                out.push(r.to_owned());
            }
        }
    }
    out.sort();
    out
}

/// Where an attempt's `CHANGE.md` goes in its commit.
pub(crate) const CHANGES_DIR: &str = "changes";

/// Moves the worktree's `CHANGE.md` to `changes/<run>-<n>.md`.
fn file_change_note(w: &Path, run: &str, n: u64) -> anyhow::Result<()> {
    let from = w.join("CHANGE.md");
    if !from.is_file() {
        return Ok(());
    }
    let dir = w.join(CHANGES_DIR);
    fs::create_dir_all(&dir)?;
    let safe: String = run
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    fs::rename(&from, dir.join(format!("{safe}-{n}.md")))?;
    Ok(())
}

/// Merges main into the worktree's branch when main has moved. A conflict
/// is undone and reported as the error, naming the files.
fn sync_main(w: &Path) -> anyhow::Result<String> {
    let main = git_ok(w, &["rev-parse", "--short", "main"])?;
    if git(w, &["merge-base", "--is-ancestor", "main", "HEAD"]).is_some() {
        return Ok(format!("The branch already has main's latest ({main})."));
    }
    let m = Command::new("git")
        .arg("-C")
        .arg(w)
        .args(["merge", "--no-edit", "-q", "main"])
        .stdin(Stdio::null())
        .output()?;
    if m.status.success() {
        return Ok(format!("Main ({main}) merged into the branch first."));
    }
    let files = git(w, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
    let _ = git(w, &["merge", "--abort"]);
    let files: Vec<&str> = files.lines().collect();
    if files.is_empty() {
        bail!(
            "merging main ({main}) into the branch failed: {}",
            String::from_utf8_lossy(&m.stderr).trim()
        );
    }
    bail!(
        "main ({main}) conflicts with the branch in {}; not scored, since the tree would be stale. Run `git merge main` in the worktree, fix the conflicts, commit, and record the attempt again",
        files.join(", ")
    )
}

/// The branch's tree as one commit on main's HEAD, kept by a ref
/// (`attempts/<run>/<clip>/<n>`). None when the branch changes nothing.
fn squash_on_main(
    w: &Path,
    run: &str,
    clip: &str,
    n: u64,
    msg: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let tree = git_ok(w, &["rev-parse", "HEAD^{tree}"])?;
    if tree == git_ok(w, &["rev-parse", "main^{tree}"])? {
        return Ok(None);
    }
    let main = git_ok(w, &["rev-parse", "main"])?;
    let c = git_ok(w, &["commit-tree", &tree, "-p", &main, "-m", msg])?;
    let safe = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let name = format!("attempts/{}/{}/{n}", safe(run), safe(clip));
    git_ok(w, &["update-ref", &format!("refs/heads/{name}"), &c])?;
    Ok(Some((c, name)))
}

/// The model an attempt is scored on: the run's working copy with the
/// changes in `theirs` (against `base`, the copy it started from) merged
/// in file by file, so the numbers the queue tuned stay. A file whose
/// changes conflict is taken from `theirs` as it is, and said so.
fn attempt_model(
    run: &Run,
    theirs: &Path,
    base: Option<&Path>,
    out: &Path,
) -> anyhow::Result<String> {
    let mine_dir = run.model_dir();
    if !theirs.join(gp_model::MODEL_FILE).is_file() {
        bail!("{} has no {}", theirs.display(), gp_model::MODEL_FILE);
    }
    fs::create_dir_all(out)?;
    for f in files_under(&mine_dir) {
        let to = out.join(&f);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(mine_dir.join(&f), &to)?;
    }
    let mut taken = Vec::new();
    let mut merged = 0;
    let mut changed = 0;
    for f in files_under(theirs) {
        let other = fs::read(theirs.join(&f))?;
        let base_b = base
            .and_then(|b| fs::read(b.join(&f)).ok())
            .or_else(|| fs::read(run.game.join(&f)).ok());
        if base_b.as_deref() == Some(other.as_slice()) {
            continue;
        }
        changed += 1;
        let to = out.join(&f);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        let mine = fs::read(&to).ok();
        match (mine, base_b) {
            (Some(mine), Some(b)) if mine != b => {
                let tmp = out.join(".merge");
                fs::create_dir_all(&tmp)?;
                let (x, y, z) = (tmp.join("mine"), tmp.join("base"), tmp.join("theirs"));
                fs::write(&x, &mine)?;
                fs::write(&y, &b)?;
                fs::write(&z, &other)?;
                let o = Command::new("git")
                    .args(["merge-file", "-p"])
                    .args([&x, &y, &z])
                    .output()?;
                if o.status.success() {
                    fs::write(&to, &o.stdout)?;
                    merged += 1;
                } else {
                    fs::write(&to, &other)?;
                    taken.push(f.display().to_string());
                }
            }
            _ => fs::write(&to, &other)?,
        }
    }
    let mut says = format!(
        "The run's model with the {changed} files changed in {} merged in",
        theirs.display()
    );
    if merged > 0 {
        says.push_str(&format!(" ({merged} merged with the tuned numbers)"));
    }
    if !taken.is_empty() {
        says.push_str(&format!(
            "; {} taken as they are, their tuned numbers set aside (they conflicted)",
            taken.join(", ")
        ));
    }
    says.push('.');
    Ok(says)
}

// ------------------------------------------------------------- attempts

/// What `gp agent attempt` was given.
pub(crate) struct AttemptArgs<'a> {
    pub run: &'a str,
    pub clip: &'a str,
    pub by: &'static str,
    pub title: &'a str,
    pub says: &'a str,
    pub worktree: Option<&'a Path>,
    /// A model folder scored as it is.
    pub model: Option<&'a Path>,
    /// A changed copy of the run's model, merged into its working copy.
    pub model_from: Option<&'a Path>,
    /// A gp to score with instead of building the worktree's.
    pub gp: Option<&'a Path>,
    pub target: Option<&'a Path>,
    pub preview: bool,
    pub shape: bool,
    pub commit: bool,
}

fn tail(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(15)..].join("\n")
}

/// Held while a worktree's gp or trial preview builds, so agents build one
/// at a time: `$GP_HOME/work/agents/build.lock`. One that finds it held
/// says so on its clip and waits its turn.
fn build_turn(home: &Path, run: &Run, a: &AttemptArgs) -> Option<fs::File> {
    let p = home.join("work").join("agents").join("build.lock");
    fs::create_dir_all(p.parent()?).ok()?;
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&p)
        .ok()?;
    match f.try_lock() {
        Ok(()) => return Some(f),
        Err(fs::TryLockError::WouldBlock) => {}
        Err(_) => return None,
    }
    let _ = append(
        run,
        Entry {
            kind: "note".into(),
            by: a.by.to_owned(),
            clip: Some(a.clip.to_owned()),
            text: "waiting for another agent's build to finish".into(),
            ..Entry::default()
        },
    );
    f.lock().ok()?;
    Some(f)
}

/// The worktree's gp, built in its own target folder.
pub(crate) fn build_gp(worktree: &Path, target: &Path, log: &Path) -> anyhow::Result<PathBuf> {
    let o = Command::new("nice")
        .args([
            "-n", "10", "cargo", "build", "-q", "--locked", "-p", "gp-cli", "--bin", "gp",
        ])
        .env("CARGO_TARGET_DIR", target)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .output()
        .context("running cargo build in the worktree")?;
    let _ = fs::write(log, [&o.stdout[..], &o.stderr[..]].concat());
    if !o.status.success() {
        bail!("the worktree's gp didn't build:\n{}", tail(&o.stderr));
    }
    Ok(target.join("debug").join("gp"))
}

/// Builds the trial preview with the repo's `scripts/web/build-preview.sh`.
fn build_preview(
    script_repo: &Path,
    target: Option<&Path>,
    out: &Path,
    model: &Path,
    assets: &Path,
    log: &Path,
) -> anyhow::Result<()> {
    let script = std::env::var_os("GP_AGENT_PREVIEW_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(|| script_repo.join("scripts/web/build-preview.sh"));
    let mut cmd = Command::new("nice");
    cmd.args(["-n", "10", "bash"])
        .arg(&script)
        .arg(out)
        .arg(model)
        .arg(assets)
        .current_dir(script_repo)
        .stdin(Stdio::null());
    if let Some(t) = target
        && std::env::var_os("GP_AGENT_PREVIEW_SCRIPT").is_none()
    {
        cmd.env("CARGO_TARGET_DIR", t);
    }
    let o = cmd.output().context("running build-preview.sh")?;
    let _ = fs::write(log, [&o.stdout[..], &o.stderr[..]].concat());
    if !o.status.success() {
        bail!("the trial preview didn't build:\n{}", tail(&o.stderr));
    }
    Ok(())
}

/// Runs an attempt: commits and squashes the worktree's change, builds its
/// gp, scores the clip and every active clip before and after, builds the
/// trial preview, and records it.
pub(crate) fn run_attempt(ctx: &Ctx, a: &AttemptArgs) -> anyhow::Result<Attempt> {
    let run = super::game::run_named(ctx, a.run).map_err(|f| anyhow::anyhow!(f.message))?;
    let clip = super::background::active_now(ctx, &run)
        .into_iter()
        .find(|c| c.id == a.clip)
        .with_context(|| format!("no active clip {} in run {}", a.clip, a.run))?;
    let n = claim(&run)?;
    let adir = attempt_dir(&run, n);
    let mut att = Attempt {
        n,
        by: a.by.to_owned(),
        clip: a.clip.to_owned(),
        clip_name: clip.name.clone(),
        title: a.title.trim().to_owned(),
        says: a.says.trim().to_owned(),
        shape: a.shape,
        when_ms: now_ms(),
        ..Attempt::default()
    };
    // Shown at once, scores to come.
    write_attempt(&run, &att)?;
    let r = (|| -> anyhow::Result<()> {
        if let Some(w) = a.worktree {
            let w = w
                .canonicalize()
                .with_context(|| format!("no worktree {}", w.display()))?;
            att.worktree = Some(w.display().to_string());
            if a.commit {
                file_change_note(&w, a.run, n)?;
                if git(&w, &["status", "--porcelain"]).is_some_and(|s| !s.is_empty()) {
                    let msg = format!("Attempt {n} on {}: {}", a.clip, att.title);
                    git_ok(&w, &["add", "-A"])?;
                    git_ok(&w, &["commit", "-q", "-m", &msg]).context(
                        "the worktree's changes couldn't be committed (the pre-commit hook may have refused them)",
                    )?;
                }
                match sync_main(&w) {
                    Ok(says) => att.sync = says,
                    Err(e) => {
                        att.sync = format!("{e:#}");
                        return Err(e);
                    }
                }
            }
            att.branch = git(&w, &["rev-parse", "--abbrev-ref", "HEAD"]);
            att.commit = git(&w, &["rev-parse", "HEAD"]);
            if a.commit {
                let msg = format!("Attempt {n} on {}: {}\n\n{}", a.clip, att.title, att.says);
                if let Some((c, name)) = squash_on_main(&w, a.run, a.clip, n, msg.trim())? {
                    att.commit = Some(c);
                    att.squash = true;
                    att.squash_ref = Some(name);
                }
            }
            att.dirty = git(&w, &["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
        }
        let model_dir = match (a.model, a.model_from) {
            (Some(m), _) => {
                att.model_says = format!("The model folder {}.", m.display());
                m.to_owned()
            }
            (None, Some(from)) => {
                let out = adir.join("model");
                // The copy it started from, beside it (`DIR.base`), else
                // the game's own folder.
                let base = from
                    .file_name()
                    .map(|n| from.with_file_name(format!("{}.base", n.to_string_lossy())))
                    .filter(|b| b.is_dir());
                att.model_says = attempt_model(&run, from, base.as_deref(), &out)?;
                out
            }
            (None, None) if a.worktree.is_some() => {
                att.model_says = "The run's model as it stands, with the worktree's code.".into();
                run.model_dir()
            }
            (None, None) => {
                att.model_says = "Not scored: no worktree or model given.".into();
                write_attempt(&run, &att)?;
                return Ok(());
            }
        };
        write_attempt(&run, &att)?;
        // Before: the model as it stands, with this build; the queue's
        // scores on the same model are taken as they are.
        let q = super::background::saved_state(&run);
        let before = score_clips(
            ctx,
            &run,
            None,
            &adir.join("before"),
            Some(a.clip),
            &q["scores"],
        )?;
        let target = a
            .target
            .map(Path::to_owned)
            .or_else(|| a.worktree.map(|w| w.join("target")));
        let gp = match (a.gp, a.worktree) {
            (Some(g), _) => Some(g.to_owned()),
            (None, Some(w)) => {
                let _turn = build_turn(&ctx.paths.home, &run, a);
                Some(build_gp(
                    w,
                    target.as_deref().unwrap_or(&w.join("target")),
                    &adir.join("build.log"),
                )?)
            }
            (None, None) => None,
        };
        let after = match &gp {
            Some(g) => {
                let o = Command::new("nice")
                    .arg("-n")
                    .arg("10")
                    .arg(g)
                    .args(["--json", "agent", "score", "--run", a.run, "--clip", a.clip])
                    .arg("--games")
                    .arg(&ctx.games)
                    .env("GP_SERVE_MODULES", ctx.modules_env())
                    .arg("--model")
                    .arg(&model_dir)
                    .arg("--out")
                    .arg(adir.join("after"))
                    .stdin(Stdio::null())
                    .output()
                    .with_context(|| format!("running {}", g.display()))?;
                if !o.status.success() {
                    let err = String::from_utf8_lossy(&o.stderr);
                    if err.contains("unrecognized subcommand") {
                        bail!(
                            "the worktree's gp has no `gp agent score`: merge main into the worktree's branch first"
                        );
                    }
                    bail!(
                        "the worktree's gp couldn't score the clips:\n{}",
                        tail(&o.stderr)
                    );
                }
                let v: Value = serde_json::from_slice(&o.stdout)
                    .context("the worktree's gp agent score didn't print JSON")?;
                scored_from_json(v.get("result").unwrap_or(&v))?
            }
            None => score_clips(
                ctx,
                &run,
                Some(&model_dir),
                &adir.join("after"),
                Some(a.clip),
                &Value::Null,
            )?,
        };
        att.before = before.scores.get(a.clip).map(Side::of);
        att.after = after.scores.get(a.clip).map(Side::of);
        for (id, b) in &before.scores {
            if id == a.clip {
                continue;
            }
            let Some(x) = after.scores.get(id) else {
                continue;
            };
            att.others += 1;
            if rank(x) < rank(b) {
                att.better.push(id.clone());
            } else if rank(x) > rank(b) {
                att.worse.push(id.clone());
            }
        }
        fs::write(
            adir.join("curve.json"),
            serde_json::to_vec(&json!({"before": before.curve, "after": after.curve}))?,
        )?;
        fs::write(
            adir.join("scores.json"),
            serde_json::to_vec(&json!({"before": before.scores, "after": after.scores}))?,
        )?;
        write_attempt(&run, &att)?;
        if a.preview {
            let r = (|| -> anyhow::Result<()> {
                if run.platform() == crate::roms::Platform::Arcade {
                    bail!(
                        "an arcade game's preview of the model isn't built here yet; play the attempt's model in the mechanic page"
                    );
                }
                let rom = super::game::rom_of(ctx, &run).map_err(|f| anyhow::anyhow!(f.message))?;
                let assets =
                    crate::corpus_score::assets_of(&ctx.paths, &run.game, &run.game, None, &rom)?;
                let script_repo = a.worktree.map_or_else(|| ctx.repo.clone(), Path::to_owned);
                let _turn = build_turn(&ctx.paths.home, &run, a);
                build_preview(
                    &script_repo,
                    target.as_deref(),
                    &adir.join("preview"),
                    &model_dir,
                    &assets,
                    &adir.join("preview.log"),
                )
            })();
            match r {
                Ok(()) => att.preview = Some(format!("agent/{n}/preview")),
                Err(e) => att.preview_error = Some(format!("{e:#}")),
            }
        }
        Ok(())
    })();
    if let Err(e) = &r {
        att.model_says = format!("It failed: {e:#}");
    }
    write_attempt(&run, &att)?;
    append(
        &run,
        Entry {
            kind: "attempt".into(),
            by: a.by.to_owned(),
            clip: Some(a.clip.to_owned()),
            text: att.title.clone(),
            attempt: Some(n),
            ..Entry::default()
        },
    )?;
    r.map(|()| att)
}

// ------------------------------------------------------------- on main

/// `$GP_HOME/work/agents/approved.json`: the attempts kept, for the driver.
pub(crate) fn approved_file(home: &Path) -> PathBuf {
    home.join("work").join("agents").join("approved.json")
}

pub(crate) fn approved_json(home: &Path) -> Value {
    fs::read(approved_file(home))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

/// The agent repo: `GP_AGENT_REPO`, else the gameprinter checkout.
pub(crate) fn agent_repo(ctx: &Ctx) -> PathBuf {
    std::env::var_os("GP_AGENT_REPO").map_or_else(|| ctx.repo.clone(), PathBuf::from)
}

/// Whether a commit or ref is on main in `repo`: an ancestor of main, or
/// a commit whose patch main already holds (`git cherry`).
pub(crate) fn commit_on_main_in(repo: &Path, r: &str) -> bool {
    let Some(main) = git(repo, &["rev-parse", "--verify", "-q", "main^{commit}"]) else {
        return false;
    };
    let Some(c) = git(
        repo,
        &["rev-parse", "--verify", "-q", &format!("{r}^{{commit}}")],
    ) else {
        return false;
    };
    static SEEN: Mutex<BTreeMap<(String, String), bool>> = Mutex::new(BTreeMap::new());
    let key = (main.clone(), c.clone());
    if let Some(v) = SEEN.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return *v;
    }
    let on = git(repo, &["merge-base", "--is-ancestor", &c, &main]).is_some()
        || git(repo, &["cherry", &main, &c, &format!("{c}^")]).is_some_and(|out| {
            out.lines()
                .any(|l| l.strip_prefix("- ").is_some_and(|h| h.trim() == c))
        });
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() > 4096 {
        seen.clear();
    }
    seen.insert(key, on);
    on
}

/// Whether an approval's change is on main, read from git, never stored.
pub(crate) fn entry_on_main_in(repo: &Path, e: &Value) -> bool {
    ["squash_ref", "commit"]
        .iter()
        .filter_map(|k| e[*k].as_str())
        .any(|c| commit_on_main_in(repo, c))
}

/// The fixes ready to merge: each approved attempt not on main, newest per
/// clip.
pub(crate) fn fix_ready_json(run: &Run) -> Value {
    let repo = std::env::var_os("GP_AGENT_REPO").map_or_else(super::repo_root, PathBuf::from);
    let mut by_clip: BTreeMap<String, Value> = BTreeMap::new();
    for a in attempts(run).iter().filter(|a| a.approved) {
        if [a.squash_ref.as_deref(), a.commit.as_deref()]
            .into_iter()
            .flatten()
            .any(|c| commit_on_main_in(&repo, c))
        {
            continue;
        }
        let exact = a.after.as_ref().is_some_and(|s| s.exact);
        let says = format!(
            "A fix is ready: attempt {} makes this clip {}. It's waiting to be merged.",
            a.n,
            if exact { "exact" } else { "better" }
        );
        by_clip.insert(
            a.clip.clone(),
            json!({"clip": a.clip, "attempt": a.n, "title": a.title, "exact": exact, "says": says}),
        );
    }
    let fixes: Vec<Value> = by_clip.into_values().collect();
    let mut n: Vec<u64> = fixes.iter().filter_map(|f| f["attempt"].as_u64()).collect();
    n.sort_unstable();
    n.dedup();
    json!({"count": n.len(), "fixes": fixes})
}

/// A keep by the rule alone, for a clip agent the queue handed a group of
/// stuck clips (`serve/clip_agent.rs`).
pub(crate) struct Auto<'a> {
    pub clips: &'a [String],
    pub group: Option<&'a str>,
}

/// Keeps an attempt, by a person (`auto` none) or by the rule: marks it
/// approved and adds it to the approved file.
pub(crate) fn approve_attempt_as(
    home: &Path,
    run: &Run,
    n: u64,
    auto: Option<Auto<'_>>,
) -> anyhow::Result<Attempt> {
    let mut a = read_attempt(run, n).with_context(|| format!("no attempt {n}"))?;
    if !a.keepable() {
        bail!(
            "attempt {n} can't be kept: keep is for an attempt that makes its clip better and no active clip worse"
        );
    }
    if a.approved {
        return Ok(a);
    }
    a.approved = true;
    write_attempt(run, &a)?;
    {
        let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
        let file = approved_file(home);
        let mut v: Value = fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({"schema": APPROVED_SCHEMA, "approved": []}));
        if !v["approved"].is_array() {
            v["approved"] = json!([]);
        }
        v["schema"] = json!(APPROVED_SCHEMA);
        if let Some(list) = v["approved"].as_array_mut() {
            list.push(json!({
                "run": run.name,
                "clip": a.clip,
                "clip_name": a.clip_name,
                "attempt": a.n,
                "title": a.title,
                "says": a.says,
                "by": a.by,
                "branch": a.branch,
                "commit": a.commit,
                "squash": a.squash,
                "squash_ref": a.squash_ref,
                "worktree": a.worktree,
                "dirty": a.dirty,
                "shape": a.shape,
                "user_ok": auto.is_none(),
                "auto": auto.is_some(),
                "clips": auto.as_ref().map_or_else(|| vec![a.clip.clone()], |x| x.clips.to_vec()),
                "group": auto.as_ref().and_then(|x| x.group),
                "approved_ms": now_ms(),
            }));
        }
        if let Some(p) = file.parent() {
            fs::create_dir_all(p)?;
        }
        let tmp = file.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&v)?)?;
        fs::rename(&tmp, &file)?;
    }
    append(
        run,
        Entry {
            kind: "approved".into(),
            by: if auto.is_some() { "clip agent" } else { "you" }.into(),
            clip: Some(a.clip.clone()),
            text: a.title.clone(),
            attempt: Some(n),
            ..Entry::default()
        },
    )?;
    Ok(a)
}

// ------------------------------------------------------------- handlers

fn state(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    Ok(Response::json(&state_json(&run)))
}

fn number(params: &Params) -> Result<u64, Fail> {
    params
        .get("n")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Fail::bad_request("an attempt is a number"))
}

fn attempt(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let n = number(params)?;
    let a = read_attempt(&run, n).ok_or_else(|| Fail::not_found(format!("no attempt {n}")))?;
    let mut v = json!(a);
    v["line"] = json!(attempt_line(&a));
    v["keepable"] = json!(a.keepable());
    v["scores"] = fs::read(attempt_dir(&run, n).join("scores.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    Ok(Response::json(&v))
}

fn curve(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let n = number(params)?;
    let v: Value = fs::read(attempt_dir(&run, n).join("curve.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| Fail::not_found(format!("attempt {n} has no curve yet")))?;
    let mut after = v["after"].clone();
    if after.is_null() {
        return Err(Fail::not_found(format!("attempt {n} has no curve")));
    }
    after["before"] = v["before"].clone();
    Ok(Response::json(&after))
}

fn preview(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let n = number(params)?;
    let rel = params.get("file").map(String::as_str).unwrap_or("");
    let mut file = attempt_dir(&run, n).join("preview");
    for part in rel.split('/') {
        if !super::game::is_name(part) {
            return Err(Fail::not_found("no such file"));
        }
        file.push(part);
    }
    let kind = match Path::new(rel).extension().and_then(|x| x.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    };
    match fs::read(&file) {
        Ok(body) => Ok(Response::bytes(kind, body)),
        Err(_) => Err(Fail::not_found("no such file in the attempt's preview")),
    }
}

fn approve(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let run = run_dir(ctx, params)?;
    let v: Value = super::body(req)?;
    let n = v["attempt"]
        .as_u64()
        .ok_or_else(|| Fail::bad_request("give the attempt's number"))?;
    approve_attempt_as(&ctx.paths.home, &run, n, None)
        .map_err(|e| Fail::bad_request(format!("{e:#}")))?;
    Ok(Response::json(&state_json(&run)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(permille: u64, exact: bool) -> Side {
        Side {
            permille,
            score: permille,
            exact,
            first_wrong: None,
            first_path: None,
            says: String::new(),
            error: None,
        }
    }

    #[test]
    fn keep_needs_better_and_none_worse() {
        let mut a = Attempt {
            n: 2,
            title: "the barrel rolls off the edge".into(),
            before: Some(side(620, false)),
            after: Some(side(810, false)),
            others: 5,
            better: vec!["a".into(), "b".into(), "c".into()],
            when_ms: 1,
            ..Attempt::default()
        };
        assert!(a.keepable());
        assert_eq!(
            attempt_line(&a),
            "Attempt 2: the barrel rolls off the edge. Clip: 62% -> 81%; others: 3 better, none worse."
        );
        a.worse.push("d".into());
        assert!(!a.keepable());
        a.worse.clear();
        a.after = Some(side(1000, true));
        assert!(a.keepable());
        assert!(attempt_line(&a).contains("62% -> matches"));
        a.after = Some(side(600, false));
        assert!(!a.keepable());
    }

    #[test]
    fn the_one_working_reads_as_a_sentence() {
        let e = |kind: &str, text: &str, on: Option<&str>| Entry {
            kind: kind.into(),
            by: "driver".into(),
            clip: Some("c".into()),
            text: text.into(),
            on: on.map(str::to_owned),
            ..Entry::default()
        };
        let w = working(
            &[
                e(
                    "note",
                    "reading the roll routine",
                    Some("the barrel (x parts at frame 9)"),
                ),
                e("note", "found the cause in the roll routine", None),
            ],
            &[],
        );
        assert_eq!(
            w[0].says,
            "Claude is working on: the barrel (x parts at frame 9). Last: found the cause in the roll routine."
        );
        let w = working(&[e("note", "x", None), e("done", "", None)], &[]);
        assert!(w.is_empty());
    }

    #[test]
    fn who_writes_is_one_of_two() {
        assert_eq!(parse_by("the driver").unwrap(), "driver");
        assert_eq!(parse_by("clip-agent").unwrap(), "clip agent");
        assert!(parse_by("someone").is_err());
    }
}
