//! gp serve's runner (docs/loop.md, "The runner"; issue #43): the loop
//! starts its own workers, so no agent controls it at the top. Every few
//! seconds, while the home's `loop.toml` has the runner on and the loop
//! isn't halted, it reads the open steps (crate::steps, as `gp loop steps`
//! gives them) and starts a worker on each open step of the agent's that
//! no worker has had in the last AGAIN_AFTER_MS (a step still open after
//! its worker ended gets another, up to MAX_RUNS), that touches nothing a
//! live worker's step touches, up to the owner's `max_workers`.
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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

/// How often the runner checks the loop's health for trouble that lasts.
const HEALTH_EVERY: Duration = Duration::from_secs(300);

/// What a worker needs besides its step.
#[derive(Clone)]
pub(crate) struct Setup {
    pub home: PathBuf,
    pub games: PathBuf,
    /// The repo its worktree is made from.
    pub repo: PathBuf,
    /// gameprinter's checkout, which its gp mcp reads.
    pub tools: PathBuf,
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

/// How many yeses run now, from claimed to their ask's result written.
static YESES_RUNNING: AtomicUsize = AtomicUsize::new(0);
/// The asks whose yes this server runs now: one left `running` that isn't
/// here died with the server that ran it ([`gp_mcp::asks::end_dead_runs`]).
static YES_IDS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
/// The clip keeping's gates run now ([`keep_clips`]).
static KEEPING: AtomicBool = AtomicBool::new(false);

/// What the runner has at work now: its workers, the yeses it runs and
/// whether the clip keeping runs. serve/reload.rs moves onto a new gp only
/// once all are none.
pub(crate) fn at_work() -> (usize, usize, bool) {
    (
        RUNNING.lock().unwrap_or_else(|e| e.into_inner()).len(),
        YESES_RUNNING.load(Ordering::SeqCst),
        KEEPING.load(Ordering::SeqCst),
    )
}

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
/// How long after a step's worker ended, with the step still open, a new
/// worker takes it up again: the row came back to the loop (a failed card
/// set it to next, a worker stopped short) and nothing else would move it.
pub(crate) const AGAIN_AFTER_MS: u64 = 15 * 60 * 1000;

/// The most workers one step gets: past these the step waits for its row
/// to change (a new note makes a new step id) or for the person.
pub(crate) const MAX_RUNS: u64 = 3;

/// The runs a worker's record counts (records before the count had one).
fn runs(w: &Value) -> u64 {
    w["runs"].as_u64().unwrap_or(1)
}

pub(crate) fn plan(
    open: &[Value],
    workers: &[Value],
    live: &BTreeSet<String>,
    max: usize,
    now_ms: u64,
) -> Vec<Value> {
    let is_live = |w: &&Value| {
        w["step"]["id"].as_str().is_some_and(|id| live.contains(id)) || loop_state::holds(w)
    };
    // A step an ended worker had waits a while, then gets another, up to
    // MAX_RUNS: its row is still open, so the work isn't done.
    let had: BTreeSet<&str> = workers
        .iter()
        .filter(|w| {
            let ended = w["last_ms"]
                .as_u64()
                .or_else(|| w["started_ms"].as_u64())
                .unwrap_or(0);
            is_live(w) || runs(w) >= MAX_RUNS || now_ms.saturating_sub(ended) < AGAIN_AFTER_MS
        })
        .filter_map(|w| w["step"]["id"].as_str())
        .collect();
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
        // Only quest-log items: the engine's own agent steps are hints for
        // an agent at the keyboard (one, "add the clips", imports into the
        // games checkout itself), not work the runner starts unasked.
        if s["who"] != json!("agent")
            || s["journey"] != json!("quest log")
            || had.contains(id)
            || !id_ok(id)
        {
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
    let says = if let Some(w) = super::reload::waiting_words() {
        w
    } else if !s.runner.on {
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
        "kept": kept_says(&ctx.paths.home),
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
    // The build folders of workers that ended while no gp serve ran.
    {
        let home = ctx.paths.home.clone();
        std::thread::spawn(move || {
            for p in crate::builds::ended_loop_builds(&home, &running_ids()) {
                let _ = fs::remove_dir_all(p);
            }
        });
    }
    std::thread::spawn(move || {
        // The loop's health, checked every few minutes so a problem that
        // lasts files itself as a row (crate::trouble) with no one running
        // gp loop health. The first check is a minute in, after the first
        // heartbeats.
        let mut health_at = Instant::now() + Duration::from_secs(60);
        loop {
            if Instant::now() >= health_at {
                health_at = Instant::now() + HEALTH_EVERY;
                let _ = crate::loop_cmd::health_in(&ctx.paths, Some(&ctx.games));
            }
            // The heartbeat the runner check reads (gp_mcp::steps::no_runner).
            let agent = matches!(clip_agent::Settings::current(&ctx).mode, Mode::Command(_));
            loop_state::beat(&ctx.paths.home, agent);
            // A yes whose run died (gp serve stopped mid-gate) fails, so its
            // row goes back to next instead of waiting for a hand.
            gp_mcp::asks::end_dead_runs(&ctx.paths.home, |id| {
                YES_IDS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains(id)
            });
            settle_card_rows(&ctx.paths.home);
            // The person's yeses run even while gp serve moves onto a newly
            // published gp (serve/reload.rs): the move waits for the
            // workers' steps, which can take half an hour, and a yes must
            // not wait that long (trouble-card-the-person-said-yes-over-a-
            // minute). The move waits for the yeses too (still_at_work).
            if loop_state::settings(&ctx.paths.home).runner.on {
                run_yeses(&ctx);
            }
            // Anything else new waits for the move; the new build takes it up.
            if !super::reload::draining() {
                if loop_state::settings(&ctx.paths.home).runner.on
                    && crate::wrong::halt(&ctx.paths.home).is_none()
                {
                    keep_clips(&ctx);
                }
                look(&ctx);
            }
            std::thread::sleep(LOOK);
        }
    });
}

/// The person's yeses, run by the runner itself: each yes on an ask not
/// yet used runs that exact call once (`gp --json COMMAND ARGV`), in a
/// thread of its own, and its ask records running, then landed, passed
/// or failed with its last lines, which the page's card shows. No agent
/// is needed between the YES and the run.
fn run_yeses(ctx: &Arc<Ctx>) {
    let home = ctx.paths.home.clone();
    let now = gp_mcp::asks::now_ms();
    for a in gp_mcp::asks::all(&home) {
        if !gp_mcp::asks::is_yes(&a, now) {
            continue;
        }
        let id = a["id"].as_str().unwrap_or("").to_owned();
        // Counted before it is taken, so the move onto a new gp (reload.rs,
        // which marks itself sealed and then counts the yeses) either sees
        // it or is seen here: none is cut off by the exec.
        YESES_RUNNING.fetch_add(1, Ordering::SeqCst);
        if super::reload::sealed() {
            YESES_RUNNING.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        let Some(a) = gp_mcp::asks::claim_for_runner(&home, &id) else {
            YESES_RUNNING.fetch_sub(1, Ordering::SeqCst);
            continue;
        };
        let gp = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gp"));
        let games = ctx.games.clone();
        // The person's own gameprinter checkout: the one the build tree is
        // a worktree of (its git's common folder's parent), on its branch.
        let tools = Command::new("git")
            .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .current_dir(&ctx.repo)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                PathBuf::from(String::from_utf8_lossy(&o.stdout).trim())
                    .parent()
                    .map(Path::to_path_buf)
            })
            .unwrap_or_else(|| ctx.repo.clone());
        let home = home.clone();
        YES_IDS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone());
        std::thread::spawn(move || {
            // Counted until its result is written, whatever happens.
            struct Ran(String);
            impl Drop for Ran {
                fn drop(&mut self) {
                    YESES_RUNNING.fetch_sub(1, Ordering::SeqCst);
                    YES_IDS
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&self.0);
                }
            }
            let _ran = Ran(id.clone());
            // Gates all start at once (owner, yeses-land-together): they
            // hold gate.lock in turn and the one holding it takes the
            // waiting ones of its kind into its batch (gate_queue), so
            // several yeses land in one run of the checks. Any other yes
            // runs one at a time.
            let _one = (!is_gate(a["command"].as_str().unwrap_or("")))
                .then(|| ONE.lock().unwrap_or_else(|e| e.into_inner()));
            let mut args: Vec<String> = vec!["--json".into()];
            args.extend(
                a["command"]
                    .as_str()
                    .unwrap_or("")
                    .split(' ')
                    .map(str::to_owned),
            );
            args.extend(
                a["argv"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(str::to_owned)),
            );
            // The tool's own commands (gp tool gate) act on gameprinter's
            // checkout, found from the current folder; the rest on the games.
            let tool = a["command"]
                .as_str()
                .is_some_and(|c| c.starts_with("tool "));
            let dir = if tool { &tools } else { &games };
            if tool && !args.iter().any(|x| x == "--onto") {
                args.push("--onto".into());
                args.push("main".into());
            }
            let out = Command::new(&gp)
                .args(&args)
                .current_dir(dir)
                .env("GP_HOME", &home)
                .env("GP_GAMES_DIR", &games)
                .env("GP_NO_HINTS", "1")
                .stdin(Stdio::null())
                .output();
            let (state, lines) = match out {
                Ok(o) => {
                    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
                    let mut lines: Vec<String> = v["lines"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|l| l.as_str().map(str::to_owned))
                        .collect();
                    if lines.is_empty() {
                        lines = String::from_utf8_lossy(&o.stderr)
                            .lines()
                            .chain(String::from_utf8_lossy(&o.stdout).lines())
                            .filter(|l| !l.trim().is_empty())
                            .map(str::to_owned)
                            .collect();
                    }
                    let keep = lines.len().saturating_sub(6);
                    let mut lines = lines.split_off(keep);
                    lines.push(format!("(ran in {})", dir.display()));
                    let state = if !o.status.success() || v["verdict"] == json!("FAIL") {
                        "failed"
                    } else if v["landed"].is_string() {
                        "landed"
                    } else {
                        "passed"
                    };
                    (state, lines)
                }
                Err(e) => ("failed", vec![format!("it couldn't start: {e}")]),
            };
            let _ = gp_mcp::asks::set_ran(&home, &id, state, &lines);
        });
    }
}

/// A yes's command that is a gate, which batches with the gates waiting
/// beside it: `model gate` and `tool gate`.
fn is_gate(command: &str) -> bool {
    matches!(command, "model gate" | "tool gate")
}

/// The quest-log row a card is for: its step's (`row:GAME/ROW`), read from
/// the step's worker record, since the step itself is gone once its worker
/// ended. None for a card raised outside the loop.
pub(crate) fn ask_row(home: &Path, a: &Value) -> Option<(String, String)> {
    let id = a["step"].as_str().filter(|id| id_ok(id))?;
    let w: Value = serde_json::from_slice(&fs::read(file(home, id)).ok()?).ok()?;
    row_of(&w["step"])
}

/// How long after a card ended its row still follows it: cards that ended
/// before rows followed them were settled by hand.
const CARD_SETTLES_FOR_MS: u64 = 24 * 60 * 60 * 1000;

/// What a card asks to do, in a few words: its command's message up to
/// `Expect:` (a gate's), without the game's name in front, else its sentence.
fn card_what(a: &Value) -> String {
    let argv: Vec<&str> = a["argv"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let msg = argv
        .iter()
        .position(|x| *x == "--message" || *x == "-m")
        .and_then(|i| argv.get(i + 1))
        .copied()
        .unwrap_or("");
    let game = a["game"].as_str().unwrap_or("");
    let mut what = msg.split(" Expect:").next().unwrap_or("").trim();
    if !game.is_empty() {
        what = what
            .strip_prefix(&format!("{game}:"))
            .map_or(what, str::trim);
    }
    let what = if what.is_empty() {
        a["says"].as_str().unwrap_or("").trim()
    } else {
        what
    };
    crate::next::cut(what.trim_end_matches('.'), 300)
}

/// Once a card is gone its quest-log row follows it (owner,
/// page-one-design): landed, the row is done with what landed; failed, it is
/// next with why, so the loop takes it up again; the person's NO, or a card
/// that ran out with no answer, leaves the row asking them one plain
/// question, which they answer in YOUR TURN. A row an agent works on waits
/// until it doesn't; a row already done moves only when its note still
/// talks of the card. Each card moves its row once (`row_settled`), and only
/// the newest card of a row does.
fn settle_card_rows(home: &Path) {
    let now = gp_mcp::asks::now_ms();
    // Only cards made lately can still move a row, or be a row's newest.
    let recent = gp_mcp::asks::OPEN_FOR_MS + CARD_SETTLES_FOR_MS;
    let all: Vec<(Value, Option<(String, String)>)> = gp_mcp::asks::all(home)
        .into_iter()
        .filter(|a| now.saturating_sub(a["made_ms"].as_u64().unwrap_or(0)) < recent)
        .map(|a| {
            let row = ask_row(home, &a);
            (a, row)
        })
        .collect();
    for (a, row_of_a) in &all {
        if a["row_settled"].as_bool() == Some(true) {
            continue;
        }
        let id = a["id"].as_str().unwrap_or("");
        let ran = a["ran"]["state"].as_str();
        let (kind, ended) = match (ran, a["state"].as_str()) {
            (Some("landed" | "passed" | "failed"), _) => {
                (ran.unwrap_or(""), a["ran"]["done_ms"].as_u64().unwrap_or(0))
            }
            (None, Some("no")) => ("no", a["answered_ms"].as_u64().unwrap_or(0)),
            (None, Some("open")) if !gp_mcp::asks::is_open(a, now) => (
                "ran out",
                a["made_ms"].as_u64().unwrap_or(0) + gp_mcp::asks::OPEN_FOR_MS,
            ),
            _ => continue,
        };
        if now.saturating_sub(ended) > CARD_SETTLES_FOR_MS {
            continue;
        }
        let Some((game, row)) = row_of_a else {
            let _ = gp_mcp::asks::mark(home, id, "row_settled", json!(true));
            continue;
        };
        let made = a["made_ms"].as_u64().unwrap_or(0);
        let newer = all.iter().any(|(b, rb)| {
            b["made_ms"].as_u64().unwrap_or(0) > made
                && rb.as_ref() == Some(&(game.clone(), row.clone()))
        });
        let moved = if newer {
            Ok(true)
        } else {
            settle_row_by_card(home, game, row, kind, ended, &card_what(a), a)
        };
        if let Ok(true) = moved {
            let _ = gp_mcp::asks::mark(home, id, "row_settled", json!(true));
            // Its row has it now (or a newer card of the row does): a failed
            // card is the loop's again, not a problem for gp loop health.
            let f = home.join("mechanics").join(game).join("queue.json");
            if crate::mechanic::session::read_queue(&f)
                .iter()
                .any(|r| r["mechanic"] == json!(row))
            {
                let _ = gp_mcp::asks::mark(home, id, "row_took_it", json!(true));
            }
        }
    }
}

/// Row `row` of `game` after its card ended as `kind` at `ended` (ms).
/// Ok(false) when it waits (an agent works on it), Ok(true) once it's
/// settled. A row changed after the card ended (set by hand, or by its
/// worker) is left as it is: the card is older than what the row says
/// (owner, page-status-matches-log).
fn settle_row_by_card(
    home: &Path,
    game: &str,
    row: &str,
    kind: &str,
    ended: u64,
    what: &str,
    a: &Value,
) -> anyhow::Result<bool> {
    if !crate::mechanic::session::name_ok(game) {
        return Ok(true);
    }
    let f = home.join("mechanics").join(game).join("queue.json");
    let mut q = crate::mechanic::session::read_queue(&f);
    let Some(r) = q.iter_mut().find(|r| r["mechanic"] == json!(row)) else {
        return Ok(true);
    };
    if r["status"] == json!("agent working") {
        return Ok(false);
    }
    if r["changed_ms"].as_u64().is_some_and(|c| c > ended) {
        return Ok(true);
    }
    let note = r["note"].as_str().unwrap_or("");
    let of_card = note.to_lowercase().contains("card") || note.contains("YES");
    if r["status"] == json!("done") && !of_card {
        return Ok(true);
    }
    let why = a["ran"]["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .rfind(|l| !l.trim().is_empty() && !l.starts_with("(ran in "))
        .unwrap_or("no reason given");
    let (status, note) = match kind {
        "landed" => ("done", format!("Landed on your YES: {what}.")),
        "passed" => ("done", format!("Done on your YES: {what}.")),
        "failed" => (
            "next",
            format!(
                "Its card ran on your YES and failed: {}. The loop takes it up again.",
                crate::next::cut(why.trim_end_matches('.'), 300)
            ),
        ),
        "no" => (
            "asking you",
            format!("You said NO to its card ({what}). What should change?"),
        ),
        _ => (
            "asking you",
            format!("Its card ran out unanswered ({what}). Should it land?"),
        ),
    };
    // A row with a target closes on its measure, not on the landing: it
    // waits in line until the scoreboard measures it again
    // (crate::scoreboard::close_rows).
    let (status, note) = match r["target"].as_str() {
        Some(t) if status == "done" => (
            "in line",
            format!(
                "{note} It closes itself when its target is met ({t}); the scoreboard measures it again."
            ),
        ),
        _ => (status, note),
    };
    let Some(o) = r.as_object_mut() else {
        return Ok(true);
    };
    if status == "in line" && o.contains_key("target") {
        o.insert("awaiting_measure".into(), json!(gp_mcp::asks::now_ms()));
    }
    if o.get("by") == Some(&json!("person")) && !o.contains_key("asked") {
        let said = o.get("note").cloned().unwrap_or(json!(""));
        o.insert("asked".into(), said);
    }
    o.insert("status".into(), json!(status));
    o.insert("note".into(), json!(note));
    o.remove("worker");
    if status == "done" {
        o.remove("clip");
    }
    crate::mechanic::session::write_queue(&f, &q)?;
    Ok(true)
}

/// One gate the runner runs at a time (a yes, or the clips it keeps): two
/// gates at once would share the gate's own tree.
static ONE: Mutex<()> = Mutex::new(());
/// How often the runner looks for clips the corpus hasn't.
const KEEP_EVERY: Duration = Duration::from_secs(120);

/// What the runner last took into a game's corpus on its own.
fn kept_file(home: &Path, game: &str) -> PathBuf {
    loop_state::loop_dir(home)
        .join("kept")
        .join(format!("{game}.json"))
}

/// The games whose clips the runner keeps: each game of the games
/// checkout with a corpus and mechanic sessions in the home.
fn keeping(home: &Path, games: &Path) -> Vec<String> {
    let Ok(dirs) = fs::read_dir(home.join("mechanics")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = dirs
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|g| games.join(g).join("corpus.toml").is_file())
        .collect();
    out.sort();
    out
}

/// The clips every recording and every worker makes go into the corpus
/// with no card (owner, gate-keeps-clips): every [`KEEP_EVERY`], for each
/// game with mechanic sessions, a dry `gp corpus import`, and when it
/// would add clips, `gp model gate GAME --import`, which lands them only
/// if no clip scores worse. It never changes or removes a clip already in
/// the corpus (no --replaces: that asks). A set of clips that failed isn't
/// tried again until the sessions give another.
///
/// A clip enters only when it plays together with the original (owner,
/// gate-refuses-failing-clips): the gate fails one that doesn't with
/// `refused`, and the runner holds it out (`--removes` keeps a new clip
/// out of the import), puts why on its session's quest-log row
/// ([`hold_rows`]) and gates the rest again at once. Held clips are tried
/// again once the game's folder has a new commit. What it did is in
/// `work/loop/kept/GAME.json` (with `held` and `held_at`), and the
/// runner's status says it.
fn keep_clips(ctx: &Arc<Ctx>) {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < KEEP_EVERY) || KEEPING.load(Ordering::SeqCst) {
            return;
        }
        *last = Some(Instant::now());
    }
    KEEPING.store(true, Ordering::SeqCst);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let home = &ctx.paths.home;
        for game in keeping(home, &ctx.games) {
            let dir = ctx.games.join(&game);
            let f = kept_file(home, &game);
            let before: Value = fs::read(&f)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(Value::Null);
            // The game's last commit: clips held out are tried again once
            // it moves (the model may play them now).
            let at = crate::gate::git(&ctx.games, &["log", "-1", "--format=%H", "--", &game])
                .unwrap_or_default();
            let mut held: Vec<Value> = if before["held_at"] == json!(at) {
                before["held"].as_array().cloned().unwrap_or_default()
            } else {
                Vec::new()
            };
            // At most a few rounds: each holds out what the gate refused.
            for _ in 0..3 {
                let remove: Vec<String> = held
                    .iter()
                    .filter_map(|h| h["id"].as_str().map(str::to_owned))
                    .collect();
                let Ok(o) = crate::corpus_import::run(
                    &crate::corpus_import::Args {
                        game: &game,
                        template: Some(&dir),
                        from: Some(home),
                        write: false,
                        says: None,
                        date: None,
                        remove: &remove,
                        replace: &[],
                        sessions: &[],
                    },
                    &ctx.paths,
                ) else {
                    break;
                };
                let added: Vec<String> = o.json["added"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect();
                if added.is_empty() {
                    break;
                }
                if before["state"] == json!("failed") && before["added"] == json!(added) {
                    break;
                }
                let (state, lines, refused) = gate_import(&ctx, &dir, &remove);
                let fresh: Vec<Value> = refused
                    .into_iter()
                    .filter(|r| {
                        r["id"]
                            .as_str()
                            .is_some_and(|i| added.iter().any(|a| a == i))
                    })
                    .collect();
                let keep = lines.len().saturating_sub(6);
                let mut rec = json!({
                    "game": game,
                    "at_ms": gp_mcp::asks::now_ms(),
                    "added": added,
                    "state": state,
                    "lines": lines[keep..],
                    "held": held,
                    "held_at": at,
                });
                if !fresh.is_empty() {
                    let _ = hold_rows(home, &game, &fresh);
                    held.extend(fresh);
                    rec["held"] = json!(held);
                    rec["state"] = json!("held");
                    let _ = fs::create_dir_all(f.parent().unwrap_or(home));
                    let _ = fs::write(&f, serde_json::to_vec_pretty(&rec).unwrap_or_default());
                    continue;
                }
                let _ = fs::create_dir_all(f.parent().unwrap_or(home));
                let _ = fs::write(&f, serde_json::to_vec_pretty(&rec).unwrap_or_default());
                break;
            }
        }
        KEEPING.store(false, Ordering::SeqCst);
    });
}

/// `gp model gate GAME --import`, holding out the clips `remove` names:
/// its state (landed, passed or failed), its lines, and the clips it
/// refused for not playing together (`refused`, `{id, why}`).
fn gate_import(
    ctx: &Ctx,
    dir: &Path,
    remove: &[String],
) -> (&'static str, Vec<String>, Vec<Value>) {
    let home = &ctx.paths.home;
    let gp = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gp"));
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let mut cmd = Command::new(&gp);
    cmd.args(["--json", "model", "gate"])
        .arg(dir)
        .arg("--import");
    for id in remove {
        cmd.arg("--removes").arg(id);
    }
    let out = cmd
        .current_dir(&ctx.games)
        .env("GP_HOME", home)
        .env("GP_GAMES_DIR", &ctx.games)
        .env("GP_NO_HINTS", "1")
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(o) => {
            let v: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
            let lines: Vec<String> = v["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| l.as_str().map(str::to_owned))
                .chain(
                    v["lines"]
                        .is_null()
                        .then(|| String::from_utf8_lossy(&o.stderr).trim().to_owned())
                        .filter(|s| !s.is_empty()),
                )
                .collect();
            let state = if !o.status.success() || v["verdict"] == json!("FAIL") {
                "failed"
            } else if v["landed"].is_string() {
                "landed"
            } else {
                "passed"
            };
            let refused = v["refused"].as_array().cloned().unwrap_or_default();
            (state, lines, refused)
        }
        Err(e) => (
            "failed",
            vec![format!("it couldn't start: {e}")],
            Vec::new(),
        ),
    }
}

/// The quest-log row a corpus clip's ID names: its session,
/// `drink-far-end` of `tapper.drink-far-end.001`.
fn row_of_clip(id: &str) -> Option<&str> {
    let (rest, _n) = id.rsplit_once('.')?;
    Some(rest.rsplit_once('.').map_or(rest, |(_, s)| s))
}

/// Each clip held out of the corpus waits on its session's quest-log row
/// with why: the row's `open` gets a line for it (one a clip), and a row
/// marked done goes back to next with that line as its note, since its
/// clip doesn't play together yet. A row with no entry gets one.
fn hold_rows(home: &Path, game: &str, held: &[Value]) -> anyhow::Result<()> {
    if !crate::mechanic::session::name_ok(game) {
        bail!("not a game name: {game}");
    }
    let f = home.join("mechanics").join(game).join("queue.json");
    let mut q = crate::mechanic::session::read_queue(&f);
    for h in held {
        let (Some(id), Some(why)) = (h["id"].as_str(), h["why"].as_str()) else {
            continue;
        };
        let Some(row) = row_of_clip(id) else {
            continue;
        };
        let line = format!(
            "{id} is held out of the corpus: {why}. It enters once the model plays it together with the original"
        );
        let at = match q.iter().position(|r| r["mechanic"] == json!(row)) {
            Some(i) => i,
            None => {
                q.push(json!({"mechanic": row, "status": "next", "by": "loop"}));
                q.len() - 1
            }
        };
        let Some(o) = q[at].as_object_mut() else {
            continue;
        };
        let mut open: Vec<Value> = o
            .get("open")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        open.retain(|l| {
            !l.as_str()
                .is_some_and(|s| s.starts_with(&format!("{id} is held out")))
        });
        open.push(json!(line));
        o.insert("open".into(), json!(open));
        let status = o.get("status").and_then(Value::as_str).unwrap_or("");
        if status == "done" || status.is_empty() {
            o.insert("status".into(), json!("next"));
            o.insert("note".into(), json!(crate::next::cut(&line, 600)));
            o.remove("clip");
        } else if o.get("note").is_none() {
            o.insert("note".into(), json!(crate::next::cut(&line, 600)));
        }
        o.insert("changed_ms".into(), json!(gp_mcp::asks::now_ms()));
    }
    crate::mechanic::session::write_queue(&f, &q)
}

/// What the runner last took into each game's corpus, in words.
fn kept_says(home: &Path) -> Vec<String> {
    let Ok(dir) = fs::read_dir(loop_state::loop_dir(home).join("kept")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = dir
        .flatten()
        .filter_map(|e| serde_json::from_slice::<Value>(&fs::read(e.path()).ok()?).ok())
        .map(|v| {
            let ids: Vec<&str> = v["added"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let why = v["lines"]
                .as_array()
                .and_then(|l| l.last())
                .and_then(Value::as_str)
                .unwrap_or("");
            let game = v["game"].as_str().unwrap_or("");
            let held: Vec<&str> = v["held"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|h| h["id"].as_str())
                .collect();
            let held_says = if held.is_empty() {
                String::new()
            } else {
                format!(
                    " Held out, since {} play together with the original yet (why is on {} quest-log row{}): {}.",
                    if held.len() == 1 { "it doesn't" } else { "they don't" },
                    if held.len() == 1 { "its" } else { "their" },
                    if held.len() == 1 { "" } else { "s" },
                    held.join(", ")
                )
            };
            let said = match v["state"].as_str().unwrap_or("") {
                "held" => format!("{game}: took no clip into the corpus."),
                "landed" => format!(
                    "{game}: took {} clip{} into the corpus on its own: {}.",
                    ids.len(),
                    if ids.len() == 1 { "" } else { "s" },
                    ids.join(", ")
                ),
                "failed" => format!(
                    "{game}: couldn't take {} into the corpus: {why}",
                    ids.join(", ")
                ),
                _ => format!("{game}: gated {}: {why}", ids.join(", ")),
            };
            format!("{said}{held_says}")
        })
        .collect();
    out.sort();
    out
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
        gp_mcp::asks::now_ms(),
    );
    let setup = Setup {
        home: home.clone(),
        games: ctx.games.clone(),
        repo: cs.repo.clone(),
        tools: ctx.repo.clone(),
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
    let before = fs::read(file(&setup.home, &id))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let mut rec = json!({
        "step": step,
        "state": "working",
        "why": "",
        "worker": format!("loop-{id}"),
        "started_ms": gp_mcp::asks::now_ms(),
        "tries": 0,
        "runs": before.as_ref().map_or(0, runs) + 1,
    });
    write(&setup.home, &rec);
    std::thread::spawn(move || {
        if let Err(e) = work(&setup, &flag, &mut rec) {
            rec["state"] = json!("stopped");
            rec["why"] = json!(format!("it couldn't start: {e:#}"));
        }
        rec["pid"] = Value::Null;
        // Its build folder: nothing needs it once the worker ended (its
        // card's gate builds in the gate's own tree from the patch).
        let freed = crate::builds::remove_loop_build(&setup.home, &id);
        if freed > 0 {
            rec["build_removed"] = json!(freed);
        }
        write(&setup.home, &rec);
        report_on_row(&setup, &rec);
        RUNNING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    })
}

/// A quest-log item's worker that ended: its report goes into the item's
/// log, so the person reads on the page what it did and what is left.
fn report_on_row(setup: &Setup, rec: &Value) {
    let Some((game, row)) = row_of(&rec["step"]) else {
        return;
    };
    let id = rec["step"]["id"].as_str().unwrap_or("");
    let dir = files_dir(&setup.home, id);
    let newest = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .max_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    let report = newest
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| {
            t.lines().rev().find_map(|l| {
                let v: Value = serde_json::from_str(l).ok()?;
                if v["type"] == json!("result") {
                    v["result"].as_str().map(str::to_owned)
                } else {
                    None
                }
            })
        });
    let state = rec["state"].as_str().unwrap_or("?");
    let why = rec["why"].as_str().unwrap_or("no report");
    let said = report.clone().unwrap_or_else(|| why.to_owned());
    let words = match &report {
        Some(r) => format!(
            "Worker loop-{id} ended ({state}): {}",
            crate::next::cut(r, 1500)
        ),
        None => format!("Worker loop-{id} ended ({state}): {said}"),
    };
    let _ = settle_row(&setup.home, &game, &row, &said);
    let _ = Command::new(&setup.gp)
        .args(["mechanic", "log", &game, &row, &words])
        .env("GP_HOME", &setup.home)
        .env("GP_GAMES_DIR", &setup.games)
        .env("GP_NO_HINTS", "1")
        .env_remove(loop_state::STEP_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// A row its worker left marked `agent working` when it ended: no agent is
/// on it any more, so it takes its status from the worker's report:
/// `asking you` with the report's questions when it asks the person
/// something, else `next` with what the report says is left. A row the
/// worker moved itself (done, asking you, ...) stays as it is.
fn settle_row(home: &Path, game: &str, row: &str, report: &str) -> anyhow::Result<()> {
    if !crate::mechanic::session::name_ok(game) {
        bail!("not a game name: {game}");
    }
    let f = home.join("mechanics").join(game).join("queue.json");
    let mut q = crate::mechanic::session::read_queue(&f);
    let Some(r) = q
        .iter_mut()
        .find(|r| r["mechanic"] == json!(row) && r["status"] == json!("agent working"))
    else {
        return Ok(());
    };
    let (status, note) = match asks_of(report) {
        Some(a) => ("asking you", a),
        None => ("next", left_of(report)),
    };
    let Some(o) = r.as_object_mut() else {
        return Ok(());
    };
    // The person's own words stay theirs (as gp mechanic queue keeps them).
    if o.get("by") == Some(&json!("person")) && !o.contains_key("asked") {
        let said = o.get("note").cloned().unwrap_or(json!(""));
        o.insert("asked".into(), said);
    }
    o.insert("status".into(), json!(status));
    o.insert("note".into(), json!(crate::next::cut(&note, 600)));
    o.remove("worker");
    crate::mechanic::session::write_queue(&f, &q)
}

/// The report's lines that ask the person something (a `?` ending a
/// sentence), or None when it asks nothing.
fn asks_of(report: &str) -> Option<String> {
    let asks: Vec<&str> = report
        .lines()
        .map(str::trim)
        .filter(|l| {
            l.char_indices().any(|(i, c)| {
                c == '?'
                    && l[i + 1..]
                        .chars()
                        .next()
                        .is_none_or(|n| n.is_whitespace() || "\"')*_".contains(n))
            })
        })
        .collect();
    (!asks.is_empty()).then(|| asks.join(" "))
}

/// What the report says is left: from its line naming what is left (or
/// still open) to its end, else its last paragraph.
fn left_of(report: &str) -> String {
    let lines: Vec<&str> = report.lines().collect();
    let named = lines.iter().position(|l| {
        let l = l.to_lowercase();
        [
            "left:",
            "is left",
            "'s left",
            "still open",
            "remaining:",
            "left to do",
        ]
        .iter()
        .any(|k| l.contains(k))
    });
    let part: Vec<&str> = match named {
        Some(i) => lines[i..].to_vec(),
        None => {
            let body: Vec<&str> = report.trim_end().lines().collect();
            let start = body
                .iter()
                .rposition(|l| l.trim().is_empty())
                .map_or(0, |i| i + 1);
            body[start..].to_vec()
        }
    };
    part.iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The worker's worktree, made from main the first time.
fn worktree(setup: &Setup, id: &str, games: bool) -> anyhow::Result<(PathBuf, String)> {
    // A game's step works in the games checkout (in_games); any other
    // step in the repo the agent works on.
    let repo = if games { &setup.games } else { &setup.repo };
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
        format!("Your worktree is {} (branch {branch}).", wt.display()),
        format!("Run gp commands that write, land or publish with the gp_run tool only: from your shell they are refused. Reads may run in Bash as {gp} COMMAND ARGS, one command a call: no eval, no &&, no pipes. Before cargo in the worktree, run {gp} worker env loop-{id} --rust and set what it prints."),
        format!("Every gp_run call that writes, lands or publishes is this step: gp_run cites {id} for you and refuses any other step."),
        "A change lands only through a gate: gp model gate for a game, gp tool gate for the tool. A landing waits for the person's yes on their page; ask and wait, never work around it.".to_owned(),
        "The step is the whole item: do what it needs now, as far as a card for the person's yes. If what is left is a landing, make the patch, dry-run it and raise the card in this step; never leave it to another step. Don't answer the person with an explanation in place of the work: when they say they don't understand, do the next thing and tell them in one line what is on their page.".to_owned(),
        "When the step is done, or you can't do it, stop with a short report: what you did, what you checked, and what is left.".to_owned(),
    ];
    if let Some((game, row)) = row_of(step) {
        b.push(String::new());
        b.extend([
            if step["repo"] == json!("tools") {
                format!("The step is quest-log item {row} of {game}, and its work is in gameprinter itself: your worktree is of the gameprinter checkout. Land the change with gp tool gate --patch FILE -m MESSAGE.")
            } else {
                format!("The step is quest-log item {row} of {game}. Your worktree is of the games checkout: make the change there.")
            },
            format!("First: {gp} mechanic show {game} {row} (the item, its notes and talk; before the row has a session it prints the row's note). Your first gp_run mechanic busy or mechanic log makes the row's session; mechanic start serves a page and isn't a worker's."),
            format!("At every step say where you are on the person's page: {gp} mechanic busy {game} {row} \"WHAT YOU ARE DOING. Next: WHAT COMES NEXT\"; put what you found with {gp} mechanic log {game} {row} \"...\"."),
            format!("When you ask the person to judge something they can see, give the row the clip that shows it: gp_run mechanic queue with game {game}, mechanic {row}, status \"asking you\", the question as note, clip N (or ROW/N, or a corpus clip's id), frames A-B and look THEME (the theme it is about, or game). The row and its card then have LOOK AT IT, which sets it up on their screens, the original beside the model. When they say yes to a look in the game's own pictures, lock it: gp_run mechanic approve with game {game} and mechanic {row} (the clip must be in the corpus), and the model gate then fails a change that draws it further from the original."),
            format!("Land a change with git diff > a patch file in your worktree, then gp_run model gate with game {game}, patch FILE, message \"{game}: WHAT CHANGES. Expect: WHAT THE PERSON SEES. Checked: HOW\" and dry_run first; then the same without dry_run puts a card on the person's page and the loop runs it on their YES. Don't wait for it: say in your report that the card is up, and stop. Never run a gate from your shell, and never land onto your own branch."),
            format!("A row with a target (the TARGET line of mechanic show, a measure on the scoreboard) closes itself when gp serve's scoreboard measures it there; you don't set it done, and --status done is refused before. Without one, give it one as you start: gp_run mechanic queue with game {game}, mechanic {row}, target \"LINE sprites at least N of M\" (a line of {game}/scoreboard.toml), or measure_needed \"WHAT WOULD MEASURE IT\" when no line can."),
            format!("When an item with no target is done: {gp} mechanic queue {game} {row} --status done --note \"WHAT LANDED\". A row with a target whose change is on a card: --status \"in line\"; the YES lands it and the measure closes it. When it needs the person: --status \"asking you\" with the question in --note."),
            format!("A problem you find that isn't this item's becomes a row of its own, with no card: gp_run mechanic propose with game {game}, mechanic NEW-ROW, what \"WHAT IS WRONG\", from \"WHERE YOU SAW IT\" and its test in the same call (goal \"RAM TEST\", do, stop), which goes in triage.toml so the triage tries it first. Don't fix it in this step."),
        ]);
    }
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

/// The game and row of a quest-log item's step (it touches `row:GAME/ROW`).
fn row_of(step: &Value) -> Option<(String, String)> {
    step["touches"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|t| t.strip_prefix("row:"))
        .and_then(|r| r.split_once('/'))
        .map(|(g, r)| (g.to_owned(), r.to_owned()))
}

/// Whether a step's work is in the games checkout: a quest-log item of a
/// game's (not the tools'), or the engine's step on a game (it touches
/// `game:NAME`, such as its corpus import). Its worker gets a worktree of
/// the games checkout, and its gp writes go there.
fn in_games(step: &Value) -> bool {
    if step["repo"] == json!("tools") {
        return false;
    }
    row_of(step).is_some()
        || step["touches"].as_array().is_some_and(|t| {
            t.iter()
                .any(|t| t.as_str().is_some_and(|t| t.starts_with("game:")))
        })
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
    // The home's themes, which a worker brings into the games checkout
    // (they are read there, never changed in place).
    let themes = setup.home.join("work").join("themes");
    if themes.is_dir() {
        a.push("--add-dir".into());
        a.push(themes.display().to_string());
    }
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
        // gp itself, one command a call: the item's record, the checks and
        // the gates, which hold the writes to the loop's rules.
        format!("Bash({g}:*)"),
        "Bash(gp:*)".to_owned(),
        "Bash(cargo build:*)".to_owned(),
        "Bash(cargo test:*)".to_owned(),
        "Bash(cargo fmt:*)".to_owned(),
        "Bash(cargo clippy:*)".to_owned(),
        // Moving files in its worktree (a theme brought into the games
        // checkout): copies and folders, no deletes.
        "Bash(cp:*)".to_owned(),
        "Bash(mkdir:*)".to_owned(),
        // Drawing a theme's pictures with a script, and taking a picture out
        // of the worktree through git.
        "Bash(python3:*)".to_owned(),
        "Bash(git rm:*)".to_owned(),
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
    let in_games = in_games(&step);
    let (wt, branch) = worktree(setup, &id, in_games)?;
    rec["worktree"] = json!(wt);
    rec["branch"] = json!(branch);
    // A games step's gp writes (corpus import's clips, a probe's saves) go
    // to its own worktree, never into the person's checkout, which a
    // landing may be using; they reach it only through a gate, and the
    // gates still run on the person's checkout, the one they land on.
    let games = if in_games { &wt } else { &setup.games };
    let dir = files_dir(&setup.home, &id);
    fs::create_dir_all(&dir)?;
    let mcp = dir.join("mcp.json");
    let mut env = json!({"GP_HOME": setup.home, "GP_GAMES_DIR": games});
    env[loop_state::STEP_ENV] = json!(id);
    // gp mcp reads gameprinter's own checkout (templates/), which a games
    // worktree isn't: name it, and the games checkout.
    let mut args = vec![
        json!("mcp"),
        json!("--repo"),
        json!(setup.tools),
        json!("--games"),
        json!(setup.games),
    ];
    if in_games {
        args.extend([json!("--worker-games"), json!(wt)]);
    }
    fs::write(
        &mcp,
        serde_json::to_vec_pretty(&json!({"mcpServers": {"gp": {
            "command": setup.gp, "args": args, "env": env,
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
            .env("GP_GAMES_DIR", games)
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
                // When the worker last acted: its log's newest line.
                if rec["last"] != json!(last) {
                    rec["last_ms"] = json!(gp_mcp::asks::now_ms());
                }
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
        json!({"id": id, "journey": "quest log", "who": who, "subject": "g", "says": "Do it.", "commands": ["corpus import"],
            "touches": touches, "how": {"cli": "gp corpus import g --write"}})
    }

    #[test]
    fn the_plan_leaves_the_engines_own_steps_to_an_agent_at_the_keyboard() {
        let mut engine = s("s000000000009", "agent", &["game:a"]);
        engine["journey"] = json!("porting a game");
        let open = [engine, s("s000000000008", "agent", &["row:a/r"])];
        let got = plan(&open, &[], &BTreeSet::new(), 5, 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["id"], json!("s000000000008"));
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
            ids(plan(&open, &[], &none, 2, 0)),
            ["s000000000001", "s000000000003"]
        );
        // A step a worker just had is not started again; a live worker's
        // touches and count hold.
        let now = 100 * AGAIN_AFTER_MS;
        let workers = [
            json!({"step": s("s000000000001", "agent", &["game:a"]), "state": "done", "last_ms": now - 1}),
            json!({"step": s("s000000000009", "agent", &["game:b"]), "state": "working"}),
        ];
        let live: BTreeSet<String> = ["s000000000009".to_owned()].into();
        assert_eq!(ids(plan(&open, &workers, &live, 2, now)), ["s000000000004"]);
        assert!(plan(&open, &workers, &live, 1, now).is_empty());
    }

    #[test]
    fn a_step_still_open_after_its_worker_ended_gets_another_up_to_the_most_runs() {
        let open = [s("s000000000001", "agent", &["row:a/r"])];
        let now = 100 * AGAIN_AFTER_MS;
        let ended = |runs: u64, ago: u64| {
            [
                json!({"step": s("s000000000001", "agent", &["row:a/r"]), "state": "done",
                "last_ms": now - ago, "runs": runs}),
            ]
        };
        let none = BTreeSet::new();
        // Ended a while ago, its row still open: taken up again.
        assert_eq!(
            plan(&open, &ended(1, AGAIN_AFTER_MS), &none, 2, now).len(),
            1
        );
        // Ended just now: it waits.
        assert!(plan(&open, &ended(1, 1000), &none, 2, now).is_empty());
        // At the most runs: it waits for its row to change.
        assert!(plan(&open, &ended(MAX_RUNS, AGAIN_AFTER_MS), &none, 2, now).is_empty());
        // A record from before the count is one run.
        let old = [
            json!({"step": s("s000000000001", "agent", &["row:a/r"]), "state": "stopped",
            "started_ms": now - AGAIN_AFTER_MS}),
        ];
        assert_eq!(plan(&open, &old, &none, 2, now).len(), 1);
    }

    #[test]
    fn a_row_left_agent_working_takes_its_status_from_the_report() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let f = home.join("mechanics/g/queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let q = json!([
            {"mechanic": "a", "status": "agent working", "worker": "loop-x", "note": "go", "by": "person"},
            {"mechanic": "b", "status": "agent working", "worker": "loop-y"},
            {"mechanic": "c", "status": "done", "note": "landed"},
        ]);
        fs::write(&f, q.to_string()).unwrap();
        let row = |m: &str| -> Value {
            crate::mechanic::session::read_queue(&f)
                .into_iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
        };
        settle_row(
            home,
            "g",
            "a",
            "I made the clip.\nShould I model the initials screen? Or leave it?\n",
        )
        .unwrap();
        let a = row("a");
        assert_eq!(a["status"], "asking you");
        assert_eq!(
            a["note"],
            "Should I model the initials screen? Or leave it?"
        );
        assert_eq!(a["asked"], "go");
        assert!(a.get("worker").is_none());
        settle_row(
            home,
            "g",
            "b",
            "Did the fix (see https://x/?a=1).\n\nLeft: the card for the gate.\nThen done.",
        )
        .unwrap();
        let b = row("b");
        assert_eq!(b["status"], "next");
        assert_eq!(b["note"], "Left: the card for the gate. Then done.");
        settle_row(home, "g", "c", "Anything left?").unwrap();
        assert_eq!(row("c")["status"], "done");
        // With no line naming what is left, its last paragraph.
        assert_eq!(left_of("Did it.\n\nThe card is up."), "The card is up.");
    }

    #[test]
    fn a_row_follows_its_card_once_the_card_is_gone() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let f = home.join("mechanics/g/queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(
            &f,
            json!([
                {"mechanic": "a", "status": "asking you", "note": "Its card is on your page: press YES", "clip": {"n": 1}},
                {"mechanic": "b", "status": "asking you", "note": "The card waits for your YES"},
                {"mechanic": "c", "status": "done", "note": "Done long ago"},
                {"mechanic": "d", "status": "agent working", "note": "card up"},
                // Set by hand after its card failed (tapper's move); set
                // aside after its NO (double-drink-end).
                {"mechanic": "e", "status": "done", "note": "Landed by hand after the first card failed",
                 "changed_ms": gp_mcp::asks::now_ms() - 100},
                {"mechanic": "f", "status": "in line", "note": "Its card was set aside",
                 "changed_ms": gp_mcp::asks::now_ms() - 100},
            ])
            .to_string(),
        )
        .unwrap();
        let now = gp_mcp::asks::now_ms();
        let asks = gp_mcp::asks::dir(home);
        fs::create_dir_all(&asks).unwrap();
        fs::create_dir_all(loop_state::workers_dir(home)).unwrap();
        let card = |n: u64, row: &str, extra: Value| {
            let step = format!("s{n:012x}");
            fs::write(
                file(home, &step),
                json!({"step": {"id": step, "touches": [format!("row:g/{row}")]}}).to_string(),
            )
            .unwrap();
            let mut a = json!({"id": format!("ask-{n}"), "made_ms": now - 1000 + n, "step": step, "game": "g",
                "command": "model gate", "argv": ["g", "--message", "g: the glass froths. Expect: froth"]});
            for (k, v) in extra.as_object().unwrap() {
                a[k] = v.clone();
            }
            fs::write(asks.join(format!("ask-{n}.json")), a.to_string()).unwrap();
        };
        card(
            1,
            "a",
            json!({"state": "used", "ran": {"state": "landed", "done_ms": now}}),
        );
        card(2, "b", json!({"state": "no", "answered_ms": now}));
        card(
            3,
            "c",
            json!({"state": "used", "ran": {"state": "failed", "done_ms": now, "lines": ["worse"]}}),
        );
        card(
            4,
            "d",
            json!({"state": "used", "ran": {"state": "landed", "done_ms": now}}),
        );
        card(
            5,
            "e",
            json!({"state": "used", "ran": {"state": "failed", "done_ms": now - 500, "lines": ["local changes"]}}),
        );
        card(6, "f", json!({"state": "no", "answered_ms": now - 500}));
        settle_card_rows(home);
        let row = |m: &str| -> Value {
            crate::mechanic::session::read_queue(&f)
                .into_iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
        };
        assert_eq!(row("a")["status"], "done");
        assert_eq!(row("a")["note"], "Landed on your YES: the glass froths.");
        assert!(row("a").get("clip").is_none());
        assert_eq!(row("b")["status"], "asking you");
        assert_eq!(
            row("b")["note"],
            "You said NO to its card (the glass froths). What should change?"
        );
        // A done row whose note no longer talks of a card stays done.
        assert_eq!(row("c")["status"], "done");
        // A row changed after its card ended keeps what it says.
        assert_eq!(row("e")["status"], "done");
        assert_eq!(row("f")["status"], "in line");
        assert_eq!(
            gp_mcp::asks::read(home, "ask-5").unwrap()["row_settled"],
            true
        );
        // A failed card its row took is no longer gp loop health's problem.
        assert_eq!(
            gp_mcp::asks::read(home, "ask-5").unwrap()["row_took_it"],
            true
        );
        assert_eq!(
            gp_mcp::asks::read(home, "ask-3").unwrap()["row_took_it"],
            true
        );
        // The rows the cards moved carry when they changed.
        assert!(row("a")["changed_ms"].as_u64().unwrap() >= now);
        // A row an agent works on waits, and its card isn't settled yet.
        assert_eq!(row("d")["status"], "agent working");
        assert_eq!(
            gp_mcp::asks::read(home, "ask-4")
                .unwrap()
                .get("row_settled"),
            None
        );
        assert_eq!(
            gp_mcp::asks::read(home, "ask-1").unwrap()["row_settled"],
            true
        );
        // Once settled, a row moved since stays as it is.
        let mut q = crate::mechanic::session::read_queue(&f);
        q[0]["status"] = json!("next");
        crate::mechanic::session::write_queue(&f, &q).unwrap();
        settle_card_rows(home);
        assert_eq!(row("a")["status"], "next");
        // The page's cards name their row.
        assert_eq!(
            ask_row(home, &gp_mcp::asks::read(home, "ask-2").unwrap()),
            Some(("g".into(), "b".into()))
        );
    }

    /// A clip the gate held out waits on its session's row with why: a
    /// done row goes back to next with it as its note, a row asking the
    /// person keeps its note and gets the line in `open` (once a clip), a
    /// session with no row gets one; the runner's status says it.
    #[test]
    fn a_held_clip_waits_on_its_row_with_why() {
        assert_eq!(
            row_of_clip("tapper.drink-far-end.001"),
            Some("drink-far-end")
        );
        assert_eq!(row_of_clip("hop.walk.002"), Some("walk"));
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let f = home.join("mechanics").join("g").join("queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let q = vec![
            json!({"mechanic": "drink", "status": "done", "note": "Landed it"}),
            json!({"mechanic": "tip", "status": "asking you", "note": "Is the tip right?"}),
        ];
        crate::mechanic::session::write_queue(&f, &q).unwrap();
        let held = vec![
            json!({"id": "g.drink.001", "why": "1/400 frames together straight through"}),
            json!({"id": "g.tip.002", "why": "8/10 frames together straight through"}),
            json!({"id": "g.bonus.001", "why": "can't be scored: no presses"}),
        ];
        hold_rows(home, "g", &held).unwrap();
        hold_rows(home, "g", &held[..1]).unwrap();
        let q = crate::mechanic::session::read_queue(&f);
        let row = |m: &str| {
            q.iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
                .clone()
        };
        let drink = row("drink");
        assert_eq!(drink["status"], "next");
        assert!(
            drink["note"]
                .as_str()
                .unwrap()
                .starts_with("g.drink.001 is held out of the corpus: 1/400 frames together")
        );
        assert_eq!(drink["open"].as_array().unwrap().len(), 1);
        let tip = row("tip");
        assert_eq!(tip["status"], "asking you");
        assert_eq!(tip["note"], "Is the tip right?");
        assert!(
            tip["open"][0]
                .as_str()
                .unwrap()
                .starts_with("g.tip.002 is held out")
        );
        assert_eq!(row("bonus")["status"], "next");
        let kept = kept_file(home, "g");
        fs::create_dir_all(kept.parent().unwrap()).unwrap();
        fs::write(
            &kept,
            serde_json::to_vec(
                &json!({"game": "g", "added": ["g.walk.001"], "state": "landed",
                "lines": [], "held": held[..1]}),
            )
            .unwrap(),
        )
        .unwrap();
        let says = kept_says(home);
        assert_eq!(
            says,
            [
                "g: took 1 clip into the corpus on its own: g.walk.001. Held out, since it doesn't play together with the original yet (why is on its quest-log row): g.drink.001."
            ]
        );
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
        // The games checkout, where a game's step gets its worktree.
        let games = t.join("games");
        fs::create_dir_all(&games).unwrap();
        git_in(&games, &["init", "-q", "-b", "main"]);
        fs::write(games.join("g.txt"), "g\n").unwrap();
        git_in(&games, &["add", "."]);
        git_in(&games, &["commit", "-qm", "first"]);
        Setup {
            home: t.join("home"),
            games: t.join("games"),
            tools: repo.clone(),
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
                "echo \"$GP_STEP $GP_WORKER $(git rev-parse --abbrev-ref HEAD)\" > {o}\necho \"games $GP_GAMES_DIR\" >> {o}\ncat >> {o}\necho all done\n",
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
        // A game's step works in a worktree of the games checkout, and its
        // gp and gp mcp write there, not in the person's checkout; its
        // gates still run on the person's.
        let wt = loop_state::loop_dir(&setup.home).join("trees").join(id);
        assert!(wt.join("g.txt").is_file(), "a games worktree");
        assert!(
            seen.contains(&format!("games {}\n", wt.display())),
            "{seen}"
        );
        let mcp = fs::read_to_string(files_dir(&setup.home, id).join("mcp.json")).unwrap();
        let mcp: Value = serde_json::from_str(&mcp).unwrap();
        let gp = &mcp["mcpServers"]["gp"];
        assert_eq!(gp["env"]["GP_GAMES_DIR"], json!(wt));
        let args = gp["args"].as_array().unwrap();
        let after = |flag: &str| args.iter().position(|a| a == flag).map(|i| &args[i + 1]);
        assert_eq!(after("--games"), Some(&json!(setup.games)));
        assert_eq!(after("--worker-games"), Some(&json!(wt)));
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
