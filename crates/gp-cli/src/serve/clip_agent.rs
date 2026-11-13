//! The clip agent (game-printer's serve/clip_agent.rs; docs/agents.md,
//! "The clip agent"): an agent `gp serve` starts on one clip, working in a
//! git worktree of its own and posting to the run's work record
//! (`serve/work.rs`) as it goes, so the REFINE tab shows each step.
//!
//! It starts from `POST /api/capture/{run}/clips/{clip}/agent` (the
//! queue's "let the agent fix it"), from "fix this clip" on a clip no
//! numbers fix on the model as it stands, and by itself, the hand-off:
//! while the run's queue is on, its stuck clips (a refine round wrote
//! nothing for them on this model) are grouped by the measure that parts
//! first, with the slot left out ("entities.barrel.x" for every barrel),
//! and each group goes to one agent, up to the run's agents at once (2 by
//! default, 0 for none). A group with an agent, or whose clips an approved
//! change not on main yet covers, isn't handed off, nor one handed off
//! already on the model as it stands. A handed-off agent's attempt that
//! makes its clip better and no active clip worse is kept by itself.
//!
//! Its worktree is `$GP_HOME/work/agents/<run>/<clip>` on branch
//! `agent/<run>/<clip>`, made from main's HEAD (a reused one with nothing
//! of its own is moved up to main's HEAD), with its own `CARGO_TARGET_DIR`
//! in it, cleared with `cargo clean` when the agent stops. Its copy of the
//! run's model is `$GP_HOME/work/agents/<run>/<clip>.model/`, made from the
//! working copy, with that copy kept beside it as `<clip>.model.base/`.
//!
//! The command is off unless asked for: `gp serve --clip-agent COMMAND`,
//! else `GP_CLIP_AGENT` (`GP_CLIP_AGENT=claude`, or
//! `GP_CLIP_AGENT="claude --model sonnet"`); without either, or with
//! `off`, nothing starts, and the page says how to turn it on.
//! game-printer's fell back to the request agent and then to `claude`;
//! gameprinter has no request agent, and starting Claude unasked costs the
//! owner money, so it is off by default. A `claude` command runs in print
//! mode with file tools scoped to the worktree and the model copy and a
//! short list of shell commands. Any other command gets the prompt on
//! stdin and `GP_CLIP_RUN`, `GP_CLIP_ID`, `GP_CLIP_WORKTREE`,
//! `GP_CLIP_MODEL`, `GP_CLIP_PROMPT` and `GP_GP`. It stops when the clip
//! matches, at its limits (`GP_CLIP_AGENT_TIMEOUT` seconds, default 3600,
//! and `GP_CLIP_AGENT_ATTEMPTS`, default 6), or when stopped.
//!
//! | Endpoint | What |
//! |---|---|
//! | `POST /api/capture/{run}/clips/{clip}/agent` | Starts the clip agent on the clip: `started`, and `why` when it didn't |
//! | `POST /api/capture/{run}/agent/stop` | Stops the run's agents, or `{clip}`'s alone |
//!
//! Each agent's state is the run's `agent/clip-agents/<clip>.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::game::{Run, run_dir, run_named};
use super::work::{self, Entry};
use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/clips/{clip}/agent",
            about: "POST: starts the clip agent on a clip, in its own worktree; up to the run's agents at once. Off unless gp serve has --clip-agent or GP_CLIP_AGENT",
            handler: start,
        },
        Route {
            path: "/api/capture/{run}/agent/stop",
            about: "POST: stops the run's clip agents, or {clip}'s alone",
            handler: stop,
        },
    ]
}

const DEFAULT_TIMEOUT_SECS: u64 = 3600;
const DEFAULT_ATTEMPTS: u64 = 6;

/// How the agent is started: not at all, or a command and its arguments.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Mode {
    Off,
    Command(Vec<String>),
}

/// `gp serve --clip-agent`'s value, set when the server starts.
static FLAG: OnceLock<Option<String>> = OnceLock::new();

pub(super) fn set_flag(clip: Option<String>) {
    let _ = FLAG.set(clip);
}

/// The command from the flag, else the variable; off without either.
pub(crate) fn setting(flag: Option<&str>, env: Option<&str>) -> Mode {
    let text = flag
        .filter(|s| !s.trim().is_empty())
        .or(env.filter(|s| !s.trim().is_empty()));
    match text.map(str::trim) {
        None => Mode::Off,
        Some(t) if t.eq_ignore_ascii_case("off") => Mode::Off,
        Some(t) => Mode::Command(t.split_whitespace().map(str::to_owned).collect()),
    }
}

/// What the page says while the agent is off.
pub(crate) const OFF: &str = "The clip agent is off, so nothing starts Claude unasked. To turn it on, start gp serve with GP_CLIP_AGENT=claude (or --clip-agent claude; GP_CLIP_AGENT=\"claude --model sonnet\" picks the model).";

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

/// A program on PATH, or a path that exists.
fn find_program(program: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let p = Path::new(program);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_owned());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(program))
        .find(|f| f.is_file())
}

fn is_claude(program: &str) -> bool {
    Path::new(program)
        .file_name()
        .is_some_and(|n| n == "claude")
}

/// Everything a clip agent needs besides the clip.
#[derive(Clone)]
pub(crate) struct Settings {
    pub mode: Mode,
    pub timeout: Duration,
    pub attempts: u64,
    pub paths: gp_evidence::Paths,
    pub repo: PathBuf,
    pub games: PathBuf,
    pub gp: PathBuf,
}

impl Settings {
    pub fn current(ctx: &Ctx) -> Self {
        Self {
            mode: setting(
                FLAG.get().cloned().flatten().as_deref(),
                std::env::var("GP_CLIP_AGENT").ok().as_deref(),
            ),
            timeout: Duration::from_secs(env_u64("GP_CLIP_AGENT_TIMEOUT", DEFAULT_TIMEOUT_SECS)),
            attempts: env_u64("GP_CLIP_AGENT_ATTEMPTS", DEFAULT_ATTEMPTS),
            paths: ctx.paths.clone(),
            repo: work::agent_repo(ctx),
            games: ctx.games.clone(),
            gp: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gp")),
        }
    }
}

/// `agent/clip-agents/<clip>.json`: one agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Status {
    pub clip: String,
    /// `starting`, `working`, `matched`, `stopped`, `limit`, `failed` or
    /// `finished`.
    pub state: String,
    #[serde(default)]
    pub why: String,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    /// Its copy of the run's model.
    #[serde(default)]
    pub model: Option<String>,
    pub log: Option<String>,
    pub command: Option<String>,
    pub started_ms: u64,
    #[serde(default)]
    pub from_attempt: u64,
    #[serde(default)]
    pub slot: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default)]
    pub auto: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<u64>,
}

fn status_folder(run: &Run) -> PathBuf {
    work::folder(run).join("clip-agents")
}

fn status_file(run: &Run, clip: &str) -> PathBuf {
    status_folder(run).join(format!("{}.json", safe(clip)))
}

fn read_statuses(run: &Run) -> Vec<Status> {
    let mut out: Vec<Status> = fs::read_dir(status_folder(run))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|e| fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect();
    out.sort_by_key(|s| s.started_ms);
    out
}

fn write_status(run: &Run, s: &Status) {
    let f = status_file(run, &s.clip);
    let _ = fs::create_dir_all(status_folder(run));
    let tmp = f.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_vec_pretty(s).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &f);
    }
    super::background::emit(
        &run.name,
        "agent.status",
        one_json(run, s.clone(), &work::entries(run)),
    );
}

/// One agent working now.
#[derive(Clone)]
struct Running {
    clip: String,
    clips: Vec<String>,
    group: Option<String>,
    slot: u32,
    flag: Arc<AtomicBool>,
}

static RUNNING: Mutex<BTreeMap<String, Vec<Running>>> = Mutex::new(BTreeMap::new());

fn running_on(run: &str) -> Vec<Running> {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(run)
        .cloned()
        .unwrap_or_default()
}

fn is_running(run: &str, clip: &str) -> bool {
    running_on(run).iter().any(|r| r.clip == clip)
}

fn its_attempts(run: &Run, s: &Status) -> Vec<work::Attempt> {
    work::attempts(run)
        .into_iter()
        .filter(|a| a.by == "clip agent" && a.clip == s.clip && a.n >= s.from_attempt)
        .collect()
}

/// The attempts recorded to the end: scored and previewed, their entry
/// written.
fn finished(entries: &[Entry]) -> BTreeSet<u64> {
    entries
        .iter()
        .filter(|e| e.kind == "attempt")
        .filter_map(|e| e.attempt)
        .collect()
}

/// The status line: "Agent: reading the game's roll code", "Agent: trying
/// 'the barrel rolls off the edge' (attempt 2)".
pub(crate) fn says(s: &Status, entries: &[Entry], atts: &[work::Attempt]) -> String {
    let mine: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.by == "clip agent" && e.when_ms >= s.started_ms)
        .filter(|e| e.clip.as_ref().is_none_or(|c| *c == s.clip))
        .collect();
    let k = atts.len();
    match s.state.as_str() {
        "starting" => "Agent: starting in its own copy of the repo".into(),
        "working" => {
            if let Some(a) = atts.last().filter(|a| !finished(entries).contains(&a.n)) {
                return format!("Agent: trying '{}' (attempt {k})", a.title);
            }
            match mine
                .iter()
                .rev()
                .find(|e| e.kind == "note" || e.kind == "attempt")
            {
                Some(e) if e.kind == "note" => format!("Agent: {}", e.text.trim_end_matches('.')),
                Some(_) => format!("Agent: attempt {k} scored; working on the next"),
                None => "Agent: reading the clip and the game's code".into(),
            }
        }
        "matched" if !s.kept.is_empty() => {
            format!("Agent: the clip matches after attempt {k}, kept for the coordinator to merge.")
        }
        "matched" => {
            format!("Agent: the clip matches after attempt {k}. Keep it to have it merged.")
        }
        "limit" => format!("Agent: stopped at its limit ({}).", s.why),
        "stopped" => "Agent: stopped.".into(),
        "failed" => format!("Agent: it couldn't go on: {}", s.why),
        _ if k == 0 => format!("Agent: finished without an attempt. {}", s.why),
        _ => format!(
            "Agent: finished after {k} attempts without a match. {}",
            s.why
        ),
    }
}

/// The work record's `clip_agent`: the latest started.
pub(crate) fn status_json(run: &Run) -> Value {
    match read_statuses(run).pop() {
        Some(s) => one_json(run, s, &work::entries(run)),
        None => Value::Null,
    }
}

fn one_json(run: &Run, mut s: Status, entries: &[Entry]) -> Value {
    // One that died with the server isn't working.
    if matches!(s.state.as_str(), "starting" | "working") && !is_running(&run.name, &s.clip) {
        s.state = "stopped".into();
        s.why = "the server stopped".into();
    }
    let atts = its_attempts(run, &s);
    let says = says(&s, entries, &atts);
    let mut v = json!(s);
    v["row"] = json!(row_says(&s, &says));
    v["says"] = json!(says);
    v["attempts"] = json!(atts.len());
    v
}

/// The agents working on the run now, by their number.
pub(crate) fn live_json(run: &Run) -> Value {
    let live = running_on(&run.name);
    if live.is_empty() {
        return json!([]);
    }
    let entries = work::entries(run);
    let mut out: Vec<(u32, Value)> = read_statuses(run)
        .into_iter()
        .filter(|s| live.iter().any(|r| r.clip == s.clip))
        .map(|s| (s.slot, one_json(run, s, &entries)))
        .collect();
    out.sort_by_key(|(slot, _)| *slot);
    Value::from(out.into_iter().map(|(_, v)| v).collect::<Vec<_>>())
}

/// "Agent 1: the barrel's place across (3 clips). Reading the clip and the
/// game's code."
pub(crate) fn row_says(s: &Status, says: &str) -> String {
    let about = s.about.clone().unwrap_or_else(|| s.clip.clone());
    let n = s.clips.len().max(1);
    let clips = if n == 1 {
        "1 clip".to_owned()
    } else {
        format!("{n} clips")
    };
    let rest = says.strip_prefix("Agent: ").unwrap_or(says);
    let mut c = rest.chars();
    let mut rest: String = c
        .next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default();
    if !rest.is_empty() && !rest.ends_with('.') {
        rest.push('.');
    }
    format!("Agent {}: {about} ({clips}). {rest}", s.slot.max(1))
        .trim_end()
        .to_owned()
}

/// The progress rows, one for each agent working now.
pub(crate) fn working_rows(run: &Run) -> Vec<work::Working> {
    live_json(run)
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| work::Working {
            by: format!("clip agent {}", v["slot"].as_u64().unwrap_or(1).max(1)),
            clip: v["clip"].as_str().map(str::to_owned),
            on: v["about"].as_str().map(str::to_owned),
            last: v["says"].as_str().map(str::to_owned),
            when_ms: v["started_ms"].as_u64().unwrap_or(0),
            says: v["row"].as_str().unwrap_or("").to_owned(),
        })
        .collect()
}

/// A name a git branch and a folder can carry.
fn safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('.')
        .to_owned()
}

/// The clip's worktree, made from main's HEAD the first time.
fn worktree(settings: &Settings, run: &str, clip: &str) -> anyhow::Result<(PathBuf, String)> {
    let repo = &settings.repo;
    if !repo.join(".git").exists() {
        bail!(
            "{} isn't a git checkout, so there's nothing to make a worktree of (GP_AGENT_REPO names one)",
            repo.display()
        );
    }
    let path = agents_dir(&settings.paths.home, run).join(safe(clip));
    let branch = format!("agent/{}/{}", safe(run), safe(clip));
    if path.join(".git").exists() {
        refresh(&path);
        return Ok((path, branch));
    }
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let p = path.to_string_lossy().into_owned();
    let has_branch = work::git_ok(
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
        work::git_ok(repo, &["worktree", "add", "-q", &p, &branch])?;
        refresh(&path);
    } else {
        work::git_ok(repo, &["worktree", "add", "-q", "-b", &branch, &p, "main"])?;
    }
    Ok((path, branch))
}

fn agents_dir(home: &Path, run: &str) -> PathBuf {
    home.join("work").join("agents").join(safe(run))
}

/// Brings a reused worktree up to main's HEAD when that is a fast-forward
/// and the worktree has nothing of its own.
fn refresh(wt: &Path) {
    let clean = work::git_ok(wt, &["status", "--porcelain"]).is_ok_and(|s| s.is_empty());
    let behind = work::git_ok(wt, &["merge-base", "--is-ancestor", "HEAD", "main"]).is_ok();
    if clean && behind {
        let _ = work::git_ok(wt, &["merge", "-q", "--ff-only", "main"]);
    }
}

/// The agent's copy of the run's model and the copy it started from: made
/// from the working copy the first time, kept after.
fn model_copy(home: &Path, run: &Run, clip: &str) -> anyhow::Result<PathBuf> {
    let dir = agents_dir(home, &run.name).join(format!("{}.model", safe(clip)));
    if !dir.join(gp_model::MODEL_FILE).is_file() {
        let from = run.model_dir();
        super::game::copy_tree(&from, &dir)?;
        let base = dir.with_file_name(format!("{}.model.base", safe(clip)));
        if !base.exists() {
            super::game::copy_tree(&from, &base)?;
        }
    }
    Ok(dir)
}

/// The clip's presses as runs of buttons by frame: "frames 0-39: nothing;
/// 40-45: coin; ...", in the platform's buttons.
fn presses_words(ctx: &Ctx, run: &Run, clip: &str) -> String {
    let Ok((log, at, _, _)) = super::scenarios::clip_input(ctx, run, clip) else {
        return "its presses can't be read".into();
    };
    let screen = super::game::run_screen(run);
    let names: Vec<String> = screen["buttons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b.as_str().map(str::to_owned))
        .collect();
    let text = log.to_text();
    let mut body: Vec<&str> = text
        .lines()
        .filter(|l| {
            l.split_whitespace()
                .next()
                .is_some_and(|w| w.parse::<u64>().is_ok())
        })
        .collect();
    // The arcade log's lines are runs already ("40 C......"); the NES's too.
    let mut out = Vec::new();
    let mut frame = 0u64;
    let mut runs: Vec<(u64, u64, String)> = Vec::new();
    for l in body.drain(..) {
        let mut it = l.split_whitespace();
        let n: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let cols = it.next().unwrap_or("");
        let held: Vec<&str> = cols
            .chars()
            .enumerate()
            .filter(|(_, c)| *c != '.')
            .map(|(i, _)| names.get(i).map_or("?", String::as_str))
            .collect();
        let held = if held.is_empty() {
            "nothing".to_owned()
        } else {
            held.join("+")
        };
        runs.push((frame, frame + n.saturating_sub(1), held));
        frame += n;
    }
    for (a, b, h) in runs {
        if b < at as u64 {
            continue;
        }
        let a = a.max(at as u64) - at as u64;
        let b = b - at as u64;
        out.push(format!("{a}-{b}: {h}"));
        if out.len() >= 40 {
            out.push("...".into());
            break;
        }
    }
    out.join("; ")
}

/// The prompt: the task, the clips and the question, the rules, and how
/// to post.
#[allow(clippy::too_many_arguments)]
fn prompt(
    ctx: &Ctx,
    run: &Run,
    clip: &super::background::Active,
    wt: &Path,
    model: &Path,
    settings: &Settings,
    status: &Status,
) -> String {
    let mut o = String::new();
    let mut line = |s: String| {
        o.push_str(&s);
        o.push('\n');
    };
    let v = super::background::saved_state(run);
    let gp = settings.gp.display();
    let games = settings.games.display();
    let r = &run.name;
    let platform = run.platform().name();
    line("A person is watching a clip of a game rebuilt from a model, looping on top of the original, and the two part. Numbers alone don't fix it: how a mechanic works must change. You make that change, in steps the person sees on the clip.".into());
    line(String::new());
    line(format!(
        "Run: {r} (the game {}, {platform})",
        run.game.display()
    ));
    line(format!("Clip: {} ({})", clip.id, clip.name));
    line(format!(
        "Its presses, by the clip's frame: {}",
        presses_words(ctx, run, &clip.id)
    ));
    let s = &v["scores"][&clip.id];
    if !s.is_null() {
        line(format!(
            "Its score: {}",
            s["says"].as_str().unwrap_or("not scored yet")
        ));
    }
    if status.clips.len() > 1 {
        line(String::new());
        line(format!(
            "This clip is one of {} stuck on the same thing{}. One general change should fix them all: each attempt scores every one of them, and every active clip.",
            status.clips.len(),
            status
                .question
                .as_deref()
                .map_or(String::new(), |t| format!(": {t}"))
        ));
        for c in &status.clips {
            line(format!(
                "- {c}: {}",
                v["scores"][c]["says"].as_str().unwrap_or("")
            ));
        }
    }
    for q in super::background::open_questions(run)
        .iter()
        .filter(|q| q["clip"] == json!(clip.id) && q["answer"].is_null())
    {
        line(String::new());
        line(format!(
            "The loop is stuck on a question: {}",
            q["ask"].as_str().unwrap_or("")
        ));
    }
    line(String::new());
    line(format!(
        "Your worktree: {}, a git worktree of gameprinter on its own branch, with CARGO_TARGET_DIR={}/target. Edit code only there. Your copy of the run's model: {}. Edit the model (model.toml, mapping.toml, the module's source) only there. The gp to call is {gp}; the games are in {games}. AGENTS.md and docs/ in the worktree say how the repo works; game-printer's docs/agents.md is where this loop comes from.",
        wt.display(),
        wt.display(),
        model.display()
    ));
    let mut mine = vec![clip.id.clone()];
    mine.extend(status.clips.iter().filter(|c| **c != clip.id).cloned());
    let told = super::tell::brief(run, &mine);
    if !told.is_empty() {
        line(String::new());
        line("The person has told you this about the clip (reply to each, below):".into());
        for t in told {
            line(format!("- {t}"));
        }
    }
    line(String::new());
    line("How to work:".into());
    line(format!(
        "1. Post what you are doing as you go, in plain words the person reads on the clip: `{gp} agent note --games {games} --run {r} --clip {} --by \"clip agent\" --on \"<what you're on, in a few words>\" \"<what you're doing>\"`.",
        clip.id
    ));
    line(format!(
        "2. Start with `{gp} agent brief --games {games} --run {r} --clip {} --json`: the clip's first wrong frame and values, the mechanics that own them, what the original's code read there and the next commands. Find the cause in the original's code first (`{gp} clip score`, `{gp} element inputs`, `{gp} code listing`), then make one change at a time: a mechanic, an option, the mapping or the model. A fix is a general mechanic or option that any game can turn on, never a game's special case. Describe the change in a `CHANGE.md` at the worktree root (`features:`, `log:` lines, `breaking:` only for a change that breaks old files).",
        clip.id
    ));
    line(format!(
        "3. Record each change: `{gp} agent attempt --games {games} --run {r} --clip {} --by \"clip agent\" --worktree {} --model-from {} --commit --title \"<the change in a few words>\" --says \"<one plain sentence: what changed and why>\"`. Add `--shape` when it changes how a mechanic works. It commits, builds the worktree's gp, scores this clip and every active clip, builds a trial preview and prints the score before and after. If main moved, it merges main in first; on a conflict it scores nothing, and you run `git merge main`, fix it, commit and record again. Keep the attempt only if the clip gets better and no other clip worse; otherwise undo it with a new commit and try the next idea.",
        clip.id,
        wt.display(),
        model.display()
    ));
    line(if status.auto {
        format!(
            "4. Stop when the clip matches, or after {} attempts, with `{gp} agent done --games {games} --run {r} --clip {} --by \"clip agent\" \"<why>\"`. An attempt that makes the clip better and no active clip worse is kept by itself, for the coordinator to merge; build on it.",
            settings.attempts, clip.id
        )
    } else {
        format!(
            "4. Stop when the clip matches, or after {} attempts, with `{gp} agent done --games {games} --run {r} --clip {} --by \"clip agent\" \"<why>\"`. The person keeps a change by pressing keep; the driver merges it.",
            settings.attempts, clip.id
        )
    });
    line(format!(
        "5. Between steps, read {}/{} if it exists: the person may have told you something about a clip. Reply to each with `{gp} agent note --games {games} --run {r} --clip <clip> --by \"clip agent\" --reply-to <number> \"<what you found or did>\"`.",
        wt.display(),
        super::tell::INBOX
    ));
    line(String::new());
    line(format!(
        "The shell runs only these, one command at a time (no `cd ... &&`, no variables before the command): `{gp} agent`, `{gp} model`, `{gp} clip`, `{gp} element`, `{gp} code`, `{gp} interp`, `cargo build`/`test`/`check`/`fmt`/`clippy`, `git status`/`diff`/`log`/`show`/`add`/`commit`/`revert`/`merge`, and `ls`, `cat`, `head`, `tail`, `wc`, `sed -n`, `grep`. Your working folder is already the worktree. Use Read, Grep and Glob for files, and Edit or Write for changes in the worktree and the model copy. A refused command is only that command: go on with the allowed ones."
    ));
    line("Rules: never rm, never push, never `--no-verify`, never commit ROMs, recordings, lifted assets, disassemblies or frames of a commercial game. Write to the person about what they see on the screen, not code or memory.".into());
    o
}

/// The clips an agent works on, and why.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Group {
    pub key: Option<String>,
    pub clips: Vec<String>,
    pub about: String,
    pub question: Option<String>,
    pub auto: bool,
}

/// A measure with its slot left out: every barrel's place across is one
/// group ("entities.barrel3.x" and "entities.barrel.x" alike).
pub(crate) fn group_key(path: &str) -> String {
    path.split('.')
        .map(|p| p.trim_end_matches(|c: char| c.is_ascii_digit()))
        .collect::<Vec<_>>()
        .join(".")
}

/// The run's stuck clips (no numbers fixed them on the model as it
/// stands) by the measure that parts first, leaving out `covered`.
pub(crate) fn stuck_groups(ctx: &Ctx, run: &Run, covered: &BTreeSet<String>) -> Vec<Group> {
    let v = super::background::saved_state(run);
    let model = super::background::model_key(ctx, run);
    let active = super::background::active_now(ctx, run);
    let mut out: Vec<Group> = Vec::new();
    for a in &active {
        if v["stuck"][&a.id].as_str() != Some(model.as_str()) || covered.contains(&a.id) {
            continue;
        }
        let s = &v["scores"][&a.id];
        let path = s["drift_path"].as_str().unwrap_or("");
        let key = if path.is_empty() {
            format!("clip:{}", a.id)
        } else {
            group_key(path)
        };
        match out
            .iter_mut()
            .find(|g| g.key.as_deref() == Some(key.as_str()))
        {
            Some(g) => g.clips.push(a.id.clone()),
            None => out.push(Group {
                key: Some(key.clone()),
                clips: vec![a.id.clone()],
                about: if path.is_empty() {
                    a.name.clone()
                } else {
                    format!("{} ({})", group_key(path), a.name)
                },
                question: s["says"].as_str().map(str::to_owned),
                auto: false,
            }),
        }
    }
    out
}

/// The clips an approved change not on main yet covers.
pub(crate) fn covered(ctx: &Ctx, run: &Run) -> BTreeSet<String> {
    let v = work::approved_json(&ctx.paths.home);
    let repo = work::agent_repo(ctx);
    let mut out = BTreeSet::new();
    for e in v["approved"].as_array().into_iter().flatten() {
        if e["run"] != json!(run.name) || work::entry_on_main_in(&repo, e) {
            continue;
        }
        out.extend(e["clip"].as_str().map(str::to_owned));
        for c in e["clips"].as_array().into_iter().flatten() {
            out.extend(c.as_str().map(str::to_owned));
        }
    }
    out
}

/// Starts the clip agent on a clip, with the group of the clips stuck on
/// the same thing when it is stuck.
pub(crate) fn start_agent(ctx: &Ctx, run: &Run, clip: &str) -> anyhow::Result<Value> {
    let group = stuck_groups(ctx, run, &BTreeSet::new())
        .into_iter()
        .find(|g| g.clips.iter().any(|c| c == clip))
        .map(|mut g| {
            g.clips.retain(|c| c != clip);
            g.clips.insert(0, clip.to_owned());
            g
        })
        .unwrap_or_else(|| Group {
            clips: vec![clip.to_owned()],
            ..Group::default()
        });
    start_group(ctx, run, group)
}

fn agents_limit(run: &Run) -> usize {
    super::background::queue_on(run).1
}

/// Starts an agent on a group of clips, unless one works on any of them or
/// on the group, or the run has its agents at once (at least 1).
pub(crate) fn start_group(ctx: &Ctx, run: &Run, mut group: Group) -> anyhow::Result<Value> {
    let settings = Settings::current(ctx);
    let clip_id = group
        .clips
        .first()
        .cloned()
        .context("a group with no clip")?;
    let limit = agents_limit(run).max(1);
    if let Some(why) = refusal(&running_on(&run.name), &group, limit) {
        return Ok(json!({"started": false, "why": why}));
    }
    let Mode::Command(argv) = settings.mode.clone() else {
        return Ok(json!({"started": false, "why": OFF}));
    };
    let program = argv.first().cloned().unwrap_or_default();
    let Some(found) = find_program(&program) else {
        return Ok(
            json!({"started": false, "why": format!("No clip agent is set up: `{program}` isn't installed. Install the Claude Code command line tool (`claude`), or give GP_CLIP_AGENT another command.")}),
        );
    };
    let clip = super::background::active_now(ctx, run)
        .into_iter()
        .find(|c| c.id == clip_id)
        .with_context(|| format!("no active clip {clip_id}"))?;
    if group.about.is_empty() {
        group.about = clip.name.clone();
    }
    let flag = Arc::new(AtomicBool::new(false));
    let slot = {
        let mut r = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        let live = r.entry(run.name.clone()).or_default();
        if let Some(why) = refusal(live, &group, limit) {
            return Ok(json!({"started": false, "why": why}));
        }
        let slot = (1..)
            .find(|k| !live.iter().any(|x| x.slot == *k))
            .unwrap_or(1);
        live.push(Running {
            clip: clip_id.clone(),
            clips: group.clips.clone(),
            group: group.key.clone(),
            slot,
            flag: flag.clone(),
        });
        slot
    };
    let mut status = Status {
        clip: clip_id.clone(),
        state: "starting".into(),
        started_ms: work::now_ms(),
        from_attempt: work::attempts(run).last().map_or(0, |a| a.n) + 1,
        slot,
        clips: group.clips.clone(),
        group: group.key.clone(),
        about: Some(group.about.clone()),
        question: group.question.clone(),
        auto: group.auto,
        ..Status::default()
    };
    write_status(run, &status);
    let _ = work::append(
        run,
        Entry {
            kind: "note".into(),
            by: "clip agent".into(),
            clip: Some(clip_id.clone()),
            text: "starting in its own copy of the repo".into(),
            on: Some(format!("{} ({})", clip.name, clip_id)),
            ..Entry::default()
        },
    );
    let Some(ctx2) = super::background::server_ctx() else {
        bail!("the server's context isn't set");
    };
    let name = run.name.clone();
    std::thread::spawn(move || {
        let Ok(run) = run_named(&ctx2, &name) else {
            return;
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_agent(
                &ctx2,
                &settings,
                &run,
                &clip,
                &found,
                &argv,
                &flag,
                &mut status,
            )
        }));
        let failed = match r {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(format!("{e:#}")),
            Err(_) => Some("it panicked".into()),
        };
        if let Some(why) = failed {
            status.state = "failed".into();
            status.why = why;
        }
        // Off the running list first, so the status written says what it
        // ended as.
        if let Some(v) = RUNNING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&name)
        {
            v.retain(|x| x.clip != status.clip);
        }
        write_status(&run, &status);
        if let Some(wt) = status.worktree.as_deref() {
            clear_build(Path::new(wt));
        }
        let _ = work::append(
            &run,
            Entry {
                kind: "done".into(),
                by: "clip agent".into(),
                clip: Some(status.clip.clone()),
                text: format!("{}: {}", status.state, status.why),
                ..Entry::default()
            },
        );
    });
    Ok(json!({"started": true, "slot": slot}))
}

/// Why an agent can't start on `group` with `live` working, if it can't.
fn refusal(live: &[Running], group: &Group, limit: usize) -> Option<String> {
    let first = group.clips.first().map(String::as_str).unwrap_or("");
    if live.iter().any(|r| r.clip == first) {
        return Some("The clip agent is already working on this clip.".into());
    }
    if let Some(r) = live.iter().find(|r| {
        (group.key.is_some() && r.group == group.key)
            || group.clips.iter().any(|c| r.clips.contains(c))
    }) {
        return Some(format!(
            "Agent {} is already working on this clip's group.",
            r.slot
        ));
    }
    if live.len() >= limit {
        return Some(if limit == 1 {
            "The clip agent is working on another clip of this run; stop it first.".into()
        } else {
            format!(
                "{limit} clip agents are working on other clips of this run, its most at once; stop one first."
            )
        });
    }
    None
}

/// Each run's last failed hand-off, in words, until one starts.
static HAND_OFF_ERROR: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

/// Why no clip agent works on the run's stuck clips, in one line; none
/// when none is stuck or an agent works.
pub(crate) fn idle_reason(ctx: &Ctx, run: &Run) -> Value {
    let groups = stuck_groups(ctx, run, &BTreeSet::new());
    let stuck: usize = groups.iter().map(|g| g.clips.len()).sum();
    if stuck == 0 || !running_on(&run.name).is_empty() {
        return Value::Null;
    }
    let clips = if stuck == 1 {
        "1 stuck clip".to_owned()
    } else {
        format!("{stuck} stuck clips")
    };
    let (enabled, agents) = super::background::queue_on(run);
    let cov = covered(ctx, run);
    let v = super::background::saved_state(run);
    let model = super::background::model_key(ctx, run);
    let left: Vec<&Group> = groups
        .iter()
        .filter(|g| g.clips.iter().any(|c| !cov.contains(c)))
        .collect();
    let handed = left
        .iter()
        .filter(|g| v["handed"][g.key.as_deref().unwrap_or("")].as_str() == Some(model.as_str()))
        .count();
    let why = if let Mode::Off = Settings::current(ctx).mode {
        format!("No clip agent, with {clips}: {OFF}")
    } else if agents == 0 {
        format!("No clip agent: agents are set to 0, with {clips}.")
    } else if let Some(e) = HAND_OFF_ERROR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&run.name)
    {
        format!("No clip agent: the last hand-off failed: {e}")
    } else if !enabled {
        format!("No clip agent: the background work is paused, with {clips}.")
    } else if left.is_empty() {
        format!(
            "No clip agent: every one of the {clips} is covered by an approved fix not on main yet."
        )
    } else if handed >= left.len() {
        "No clip agent: the stuck groups went to an agent on this model already; a model change hands them off again.".to_owned()
    } else {
        return Value::Null;
    };
    json!(why)
}

/// The hand-off, once a second for every run whose queue is on.
pub(crate) fn start_hand_off(ctx: Arc<Ctx>) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if !matches!(Settings::current(&ctx).mode, Mode::Command(_)) {
                continue;
            }
            for name in super::game::all_runs(&ctx) {
                if let Ok(run) = run_named(&ctx, &name) {
                    hand_off(&ctx, &run);
                }
            }
        }
    });
}

/// While the queue is on, each group of stuck clips with no agent, not
/// covered by an approved change and not handed off on this model goes to
/// an agent, up to the run's agents at once.
fn hand_off(ctx: &Ctx, run: &Run) {
    let (enabled, n) = super::background::queue_on(run);
    if !enabled || n == 0 || running_on(&run.name).len() >= n {
        return;
    }
    let model = super::background::model_key(ctx, run);
    let v = super::background::saved_state(run);
    for mut g in stuck_groups(ctx, run, &covered(ctx, run)) {
        if running_on(&run.name).len() >= n {
            break;
        }
        let key = g.key.clone().unwrap_or_default();
        if v["handed"][&key].as_str() == Some(model.as_str())
            || refusal(&running_on(&run.name), &g, usize::MAX).is_some()
        {
            continue;
        }
        g.auto = true;
        let first = g.clips[0].clone();
        match start_group(ctx, run, g) {
            Ok(r) if r["started"] == true => {
                let _ = super::background::update_state(run, |s| {
                    s["handed"][&key] = json!(model);
                });
                HAND_OFF_ERROR
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&run.name);
            }
            Ok(r) => {
                if let Some(why) = r["why"].as_str() {
                    HAND_OFF_ERROR
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(run.name.clone(), format!("{first}: {why}"));
                }
                break;
            }
            Err(e) => {
                HAND_OFF_ERROR
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(run.name.clone(), format!("{first}: {e:#}"));
            }
        }
    }
}

/// The arguments added after a `claude` command's own: print mode, file
/// tools in the worktree and the model copy only, and a short list of
/// shell commands.
pub(crate) fn claude_args(wt: &Path, model: &Path, gp: &Path, run_dir: &Path) -> Vec<String> {
    let w = wt.display();
    let m = model.display();
    let g = gp.display();
    let mut a: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--permission-mode",
        "dontAsk",
        "--strict-mcp-config",
        "--tools",
        "Read,Edit,Write,Glob,Grep,Bash",
        "--add-dir",
    ]
    .map(String::from)
    .to_vec();
    a.push(model.display().to_string());
    a.push(run_dir.display().to_string());
    a.push("--allowedTools".into());
    for t in [
        "Read".to_owned(),
        "Glob".to_owned(),
        "Grep".to_owned(),
        format!("Edit(/{w}/**)"),
        format!("Edit(/{m}/**)"),
        format!("Bash({g} agent:*)"),
        format!("Bash({g} model:*)"),
        format!("Bash({g} clip:*)"),
        format!("Bash({g} element:*)"),
        format!("Bash({g} code:*)"),
        format!("Bash({g} interp:*)"),
        "Bash(cargo build:*)".to_owned(),
        "Bash(cargo test:*)".to_owned(),
        "Bash(cargo check:*)".to_owned(),
        "Bash(cargo fmt:*)".to_owned(),
        "Bash(cargo clippy:*)".to_owned(),
        "Bash(git status:*)".to_owned(),
        "Bash(git diff:*)".to_owned(),
        "Bash(git log:*)".to_owned(),
        "Bash(git show:*)".to_owned(),
        "Bash(git add:*)".to_owned(),
        "Bash(git commit:*)".to_owned(),
        "Bash(git revert:*)".to_owned(),
        "Bash(git merge:*)".to_owned(),
        "Bash(ls:*)".to_owned(),
        "Bash(grep:*)".to_owned(),
        "Bash(cat:*)".to_owned(),
        "Bash(head:*)".to_owned(),
        "Bash(tail:*)".to_owned(),
        "Bash(wc:*)".to_owned(),
        "Bash(sed -n:*)".to_owned(),
        "Bash(pwd)".to_owned(),
    ] {
        a.push(t);
    }
    a.push("--disallowedTools".into());
    for t in [
        "Bash(rm:*)",
        "Bash(git push:*)",
        "Bash(git clean:*)",
        "Bash(git commit --no-verify:*)",
        "Bash(git commit -n:*)",
        "Bash(git reset --hard:*)",
        "Bash(git stash:*)",
        "Bash(find:*)",
        "WebFetch",
        "WebSearch",
    ] {
        a.push(t.to_owned());
    }
    a
}

/// Clears a finished agent's worktree build, keeping the worktree and its
/// branch.
fn clear_build(wt: &Path) {
    let target = wt.join("target");
    if !target.is_dir() {
        return;
    }
    let _ = Command::new("cargo")
        .arg("clean")
        .env("CARGO_TARGET_DIR", &target)
        .current_dir(wt)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// A command line as one string, each part quoted when it needs it.
fn command_line(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| {
            if p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@".contains(c))
            {
                p.clone()
            } else {
                format!("'{}'", p.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[allow(clippy::too_many_arguments)]
fn run_agent(
    ctx: &Ctx,
    settings: &Settings,
    run: &Run,
    clip: &super::background::Active,
    found: &Path,
    argv: &[String],
    flag: &AtomicBool,
    status: &mut Status,
) -> anyhow::Result<()> {
    let (wt, branch) = worktree(settings, &run.name, &clip.id)?;
    status.worktree = Some(wt.display().to_string());
    status.branch = Some(branch);
    let model = model_copy(&settings.paths.home, run, &clip.id)?;
    status.model = Some(model.display().to_string());
    let stamp = status.started_ms;
    let name = safe(&clip.id);
    let folder = work::folder(run);
    fs::create_dir_all(&folder)?;
    let prompt_path = folder.join(format!("clip-agent-{stamp}-{name}.prompt.txt"));
    let log_path = folder.join(format!("clip-agent-{stamp}-{name}.log"));
    fs::write(
        &prompt_path,
        prompt(ctx, run, clip, &wt, &model, settings, status),
    )?;
    let log = File::create(&log_path)?;
    status.log = Some(log_path.display().to_string());
    let program = argv.first().map(String::as_str).unwrap_or("");
    let mut cmd = Command::new("nice");
    cmd.args(["-n", "10"]).arg(found).args(&argv[1..]);
    let mut shown = vec![found.display().to_string()];
    shown.extend(argv[1..].iter().cloned());
    if is_claude(program) {
        let extra = claude_args(&wt, &model, &settings.gp, &run.dir);
        cmd.args(&extra);
        shown.extend(extra);
    }
    status.command = Some(command_line(&shown));
    {
        let mut l = log.try_clone()?;
        let _ = writeln!(l, "$ {}\n", command_line(&shown));
    }
    cmd.env("GP_HOME", &settings.paths.home)
        .env("GP_GAMES_DIR", &settings.games)
        .env("GP_SERVE_MODULES", ctx.modules_env())
        .env("GP_CLIP_RUN", &run.name)
        .env("GP_CLIP_ID", &clip.id)
        .env("GP_CLIP_WORKTREE", &wt)
        .env("GP_CLIP_MODEL", &model)
        .env("GP_CLIP_PROMPT", &prompt_path)
        .env("GP_GP", &settings.gp)
        .env("GP_NO_WINDOW", "1")
        .env("CARGO_TARGET_DIR", wt.join("target"))
        .current_dir(&wt)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .stdin(Stdio::piped());
    // Its own process group, so stop ends what it started too.
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().context("starting the clip agent")?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&fs::read(&prompt_path).unwrap_or_default());
    }
    status.state = "working".into();
    write_status(run, status);
    let t0 = Instant::now();
    // The agent and every process it started (a build, an attempt): its
    // own process group, by its own PID.
    let end = |child: &mut std::process::Child| {
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{}", child.id())])
            .status();
        let _ = child.kill();
        let _ = child.wait();
    };
    loop {
        let exited = child.try_wait()?.map(|s| s.code());
        keep_passing(settings, run, status);
        if let Some(code) = exited {
            let atts = its_attempts(run, status);
            if atts
                .last()
                .and_then(|a| a.after.as_ref())
                .is_some_and(|s| s.exact)
            {
                status.state = "matched".into();
            } else {
                status.state = "finished".into();
                status.why = match code {
                    Some(0) => last_words(status.log.as_deref())
                        .map(|w| format!("It said: {w}"))
                        .unwrap_or_default(),
                    Some(c) => {
                        format!("It stopped with an error (exit {c}); its output is in the log.")
                    }
                    None => "It was stopped by a signal.".into(),
                };
            }
            return Ok(());
        }
        if flag.load(Ordering::Relaxed) {
            end(&mut child);
            status.state = "stopped".into();
            status.why = "you stopped it".into();
            return Ok(());
        }
        let atts = its_attempts(run, status);
        let fin = finished(&work::entries(run));
        let done: Vec<&work::Attempt> = atts.iter().filter(|a| fin.contains(&a.n)).collect();
        if done
            .last()
            .and_then(|a| a.after.as_ref())
            .is_some_and(|s| s.exact)
        {
            end(&mut child);
            status.state = "matched".into();
            return Ok(());
        }
        if done.len() as u64 >= settings.attempts {
            end(&mut child);
            status.state = "limit".into();
            status.why = format!("{} attempts", settings.attempts);
            return Ok(());
        }
        if t0.elapsed() >= settings.timeout {
            end(&mut child);
            status.state = "limit".into();
            status.why = format!("{} minutes", settings.timeout.as_secs() / 60);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A handed-off agent's finished attempts that make its clip better and
/// no active clip worse, kept: written to the approved file with `auto`.
fn keep_passing(settings: &Settings, run: &Run, status: &mut Status) {
    if !status.auto {
        return;
    }
    let fin = finished(&work::entries(run));
    for a in its_attempts(run, status) {
        if !fin.contains(&a.n) || a.approved || !a.keepable() || status.kept.contains(&a.n) {
            continue;
        }
        let auto = work::Auto {
            clips: &status.clips,
            group: status.group.as_deref(),
        };
        if work::approve_attempt_as(&settings.paths.home, run, a.n, Some(auto)).is_ok() {
            status.kept.push(a.n);
            write_status(run, status);
        }
    }
}

/// The agent's final message, from a `claude` log in stream-json: its
/// `result` line, cut to a sentence or two for the card.
fn last_words(log: Option<&str>) -> Option<String> {
    let text = fs::read_to_string(log?).ok()?;
    let r = text.lines().rev().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        (v["type"] == "result").then(|| v["result"].as_str().map(str::to_owned))?
    })?;
    let flat = r.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(if flat.len() > 400 {
        let mut end = 400;
        while !flat.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &flat[..end])
    } else {
        flat
    })
}

fn start(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let run = run_dir(ctx, params)?;
    let clip = params.get("clip").cloned().unwrap_or_default();
    let r = start_agent(ctx, &run, &clip).map_err(|e| Fail::bad_request(format!("{e:#}")))?;
    let mut v = work::state_json(&run);
    v["started"] = r["started"].clone();
    v["why"] = r["why"].clone();
    Ok(Response::json(&v))
}

fn stop(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let run = run_dir(ctx, params)?;
    let only = serde_json::from_slice::<Value>(&req.body)
        .ok()
        .and_then(|b| b["clip"].as_str().map(str::to_owned));
    let mine = |c: &str| only.as_deref().is_none_or(|o| o == c);
    if let Some(v) = RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&run.name)
    {
        for r in v.iter().filter(|r| mine(&r.clip)) {
            r.flag.store(true, Ordering::Relaxed);
        }
    }
    Ok(Response::json(&work::state_json(&run)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_off_unless_asked_for() {
        let cmd = |v: &[&str]| Mode::Command(v.iter().map(|s| (*s).to_owned()).collect());
        assert_eq!(setting(None, None), Mode::Off);
        assert_eq!(setting(Some(""), Some("  ")), Mode::Off);
        assert_eq!(setting(None, Some("off")), Mode::Off);
        assert_eq!(setting(None, Some("claude")), cmd(&["claude"]));
        assert_eq!(
            setting(Some("claude --model sonnet"), Some("other")),
            cmd(&["claude", "--model", "sonnet"])
        );
    }

    #[test]
    fn its_status_reads_as_a_sentence() {
        let s = Status {
            clip: "c".into(),
            state: "working".into(),
            started_ms: 10,
            slot: 2,
            about: Some("entities.barrel.x".into()),
            clips: vec!["c".into(), "d".into(), "e".into()],
            ..Status::default()
        };
        let note = Entry {
            kind: "note".into(),
            by: "clip agent".into(),
            clip: Some("c".into()),
            text: "reading the barrel's roll".into(),
            when_ms: 11,
            ..Entry::default()
        };
        let says = says(&s, &[note], &[]);
        assert_eq!(says, "Agent: reading the barrel's roll");
        assert_eq!(
            row_says(&s, &says),
            "Agent 2: entities.barrel.x (3 clips). Reading the barrel's roll."
        );
    }

    #[test]
    fn a_group_leaves_the_slot_out() {
        assert_eq!(group_key("entities.barrel3.x"), "entities.barrel.x");
        assert_eq!(group_key("entities.mario.vy"), "entities.mario.vy");
    }

    #[test]
    fn one_agent_a_group_up_to_the_limit() {
        let r = |clip: &str, slot| Running {
            clip: clip.into(),
            clips: vec![clip.into()],
            group: Some(format!("g{clip}")),
            slot,
            flag: Arc::new(AtomicBool::new(false)),
        };
        let g = |clips: &[&str]| Group {
            key: Some("k".into()),
            clips: clips.iter().map(|c| (*c).to_owned()).collect(),
            ..Group::default()
        };
        assert!(refusal(&[], &g(&["a"]), 1).is_none());
        assert!(
            refusal(&[r("a", 1)], &g(&["a"]), 2)
                .unwrap()
                .contains("already")
        );
        assert!(
            refusal(&[r("b", 1)], &g(&["a", "b"]), 2)
                .unwrap()
                .contains("group")
        );
        assert!(
            refusal(&[r("b", 1), r("c", 2)], &g(&["a"]), 2)
                .unwrap()
                .contains("2 clip agents")
        );
    }

    #[test]
    fn claude_gets_its_tools_scoped() {
        let a = claude_args(
            Path::new("/w"),
            Path::new("/m"),
            Path::new("/gp"),
            Path::new("/r"),
        );
        assert!(a.contains(&"Edit(//w/**)".to_owned()));
        assert!(a.contains(&"Edit(//m/**)".to_owned()));
        assert!(a.contains(&"Bash(rm:*)".to_owned()));
        assert!(a.contains(&"Bash(git stash:*)".to_owned()));
    }
}
