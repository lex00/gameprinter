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
#[cfg(not(unix))]
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

/// How often a live worker's state (its cards, its run records, its log's
/// last line) is read again.
const LOOK_AT_STATE: Duration = Duration::from_secs(5);
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

/// A live worker as this server watches it.
#[derive(Default)]
pub(crate) struct Live {
    /// Set to stop it.
    stop: AtomicBool,
    /// Its process runs on its own (detached, its exit written to a file;
    /// [`launch`]), so a move onto a new gp leaves it running and the new
    /// server takes it up ([`adopt`]). Unset while this server does
    /// something for it in process: its worktree, between tries, its
    /// report. Changed only with [`RUNNING`] held, which the move holds
    /// from its last look to its exec.
    out: AtomicBool,
}

/// The live workers: a step id and how it is watched.
static RUNNING: Mutex<BTreeMap<String, Arc<Live>>> = Mutex::new(BTreeMap::new());

/// How many yeses run now, from claimed to their ask's result written.
static YESES_RUNNING: AtomicUsize = AtomicUsize::new(0);
/// The asks whose yes this server runs now: one left `running` that isn't
/// here died with the server that ran it ([`gp_mcp::asks::end_dead_runs`]).
static YES_IDS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
/// The clip keeping's gates run now ([`keep_clips`]).
static KEEPING: AtomicBool = AtomicBool::new(false);

/// What the runner has at work in this process now: its workers between
/// processes (a worker whose process runs on its own isn't counted: it
/// keeps running across a move and the new server takes it up), the yeses
/// it runs and whether the clip keeping runs. serve/reload.rs moves onto a
/// new gp only once all are none.
pub(crate) fn at_work() -> (usize, usize, bool) {
    (
        in_process(&RUNNING.lock().unwrap_or_else(|e| e.into_inner())),
        YESES_RUNNING.load(Ordering::SeqCst),
        KEEPING.load(Ordering::SeqCst),
    )
}

fn in_process(running: &BTreeMap<String, Arc<Live>>) -> usize {
    running
        .values()
        .filter(|l| !l.out.load(Ordering::SeqCst))
        .count()
}

/// The live workers held, for a move onto a new gp: while the guard is
/// held no worker goes from running on its own to work in this process,
/// so a move that saw none in process execs with none. With how many are
/// in process now.
pub(crate) fn hold_workers() -> (
    std::sync::MutexGuard<'static, BTreeMap<String, Arc<Live>>>,
    usize,
) {
    let g = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let n = in_process(&g);
    (g, n)
}

/// Sets whether worker `live`'s process runs on its own, with the live
/// workers held.
fn set_out(live: &Live, out: bool) {
    let _g = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    live.out.store(out, Ordering::SeqCst);
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
            // A worker the person stopped parked its row in line: once
            // they set it next again its step starts at once.
            let by_person = w["stopped_by"] == json!("person");
            is_live(w)
                || runs(w) >= MAX_RUNS
                || (!by_person && now_ms.saturating_sub(ended) < AGAIN_AFTER_MS)
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
    let mut out: Vec<(usize, Value)> = Vec::new();
    // A row that claims a whole game is looked at after the rest: it holds
    // every line of its game, so it starts only once the narrower rows have
    // had the free slots (crate::steps::claim_of). The steps start in the
    // order they are open.
    let (narrow, whole): (Vec<_>, Vec<_>) = open
        .iter()
        .enumerate()
        .partition(|(_, s)| !crate::steps::claims_game(s));
    for (i, s) in narrow.into_iter().chain(whole) {
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
        // A game's row claims its scoreboard line, and runs beside the
        // game's rows of other lines; one that names the game's model,
        // mapping or hooks claims the whole game (crate::steps::clashes).
        if touches.iter().any(|t| crate::steps::clashes(&taken, t)) {
            continue;
        }
        taken.extend(touches);
        n += 1;
        out.push((i, s.clone()));
    }
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, s)| s).collect()
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
    // A record left live by the server before (this process before its
    // move onto a new gp, or a server stopped): one whose process runs on
    // its own, or ended with its exit written, is taken up and watched
    // here; one whose process ended unwatched is stopped.
    let setup = setup_of(&ctx);
    for mut w in loop_state::workers(&ctx.paths.home) {
        if !loop_state::LIVE.contains(&w["state"].as_str().unwrap_or("")) {
            continue;
        }
        if let Some(setup) = setup.clone().filter(|_| Launched::of(&w).is_some()) {
            adopt(setup, w);
            continue;
        }
        if !w["pid"].as_u64().is_some_and(gp_mcp::proc::alive) {
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
            // The score caches still in use, the gate's among them, lose
            // their files older than a day every hour; else they grow
            // with no limit (loop-clean-gate-score-cache: 47 GB).
            let gate = crate::gate::gate_home(None).ok();
            loop {
                crate::builds::clear_old_caches(&home, gate.as_deref());
                std::thread::sleep(Duration::from_secs(60 * 60));
            }
        });
    }
    std::thread::spawn(move || {
        // The loop's health, checked every few minutes so a problem that
        // lasts files itself as a row (crate::trouble) with no one running
        // gp loop health. The first check is a minute in, after the first
        // heartbeats.
        let mut health_at = Instant::now() + Duration::from_secs(60);
        let mut archive_at = Instant::now() + Duration::from_secs(60);
        loop {
            if Instant::now() >= health_at {
                health_at = Instant::now() + HEALTH_EVERY;
                let _ = crate::loop_cmd::health_in(&ctx.paths, Some(&ctx.games));
            }
            // The heartbeat the runner check reads (gp_mcp::steps::no_runner).
            let agent = matches!(clip_agent::Settings::current(&ctx).mode, Mode::Command(_));
            loop_state::beat(&ctx.paths.home, agent);
            // A yes whose run a stopped gp serve cut off is a yes again (or
            // ends as the run's own output says), so it runs again instead
            // of waiting for a hand; cut off too often, it fails and its
            // row goes back to next.
            gp_mcp::asks::end_dead_runs(&ctx.paths.home, |id| {
                YES_IDS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains(id)
            });
            settle_card_rows(&ctx.paths.home);
            // A row whose look approved.toml has saved, by any landing, is
            // done, so its CONFIRM isn't offered again (confirmed-row-closes).
            settle_saved_rows(&ctx.paths.home, &ctx.games, gp_mcp::asks::now_ms());
            // Rows long finished leave the quest log for its archive, so the
            // page shows the work going on.
            if Instant::now() >= archive_at {
                archive_at = Instant::now() + ARCHIVE_EVERY;
                // Rows parked on another row go on once it lands (owner,
                // waits-wake-on-landing), before that row is archived.
                let now = gp_mcp::asks::now_ms();
                // A row asking the person about a card that is gone goes on
                // (trouble-rows-dont-ask-the-person-open-questions).
                settle_card_asks(&ctx.paths.home, now);
                wake_waiting(&ctx.paths.home, Some(ctx.games.as_path()), now);
                archive_finished(&ctx.paths.home, gp_mcp::asks::now_ms());
            }
            // The person's yeses run even while gp serve moves onto a newly
            // published gp (serve/reload.rs): the move waits for the
            // workers' steps, which can take half an hour, and a yes must
            // not wait that long (trouble-card-the-person-said-yes-over-a-
            // minute). The move waits for the yeses too (still_at_work).
            if loop_state::settings(&ctx.paths.home).runner.on {
                run_yeses(&ctx);
                run_confirms(&ctx);
            }
            // Clip keeping waits for the move; the new build takes it up.
            if !super::reload::draining()
                && loop_state::settings(&ctx.paths.home).runner.on
                && crate::wrong::halt(&ctx.paths.home).is_none()
            {
                keep_clips(&ctx);
            }
            // Workers start whether or not a move waits: a worker's process
            // runs on its own, on the newest gp, and the new build takes it
            // up (adopt). Before, nothing started from the moment a gp was
            // published until every running yes had ended, and with a tool
            // landing every few minutes and a model gate of ten, the open
            // rows had no worker for most of an hour.
            look(&ctx);
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
    // A worker's card for a change to gp itself is answered by the gate:
    // its landing run checks the patch, and a patch that fails lands
    // nothing (Runner::land_tools).
    if gp_mcp::steps::settings(&home).runner.land_tools {
        for a in gp_mcp::asks::all(&home) {
            if gp_mcp::asks::is_open(&a, now)
                && a["command"] == json!("tool gate")
                && a["step"].as_str().is_some_and(|s| !s.is_empty())
                && let Some(id) = a["id"].as_str()
                && gp_mcp::asks::answer(&home, id, gp_mcp::asks::Answer::Yes).is_ok()
            {
                let _ = gp_mcp::asks::mark(&home, id, "answered_by", json!("the gate"));
            }
        }
    }
    for a in gp_mcp::asks::all(&home) {
        if !gp_mcp::asks::is_yes(&a, now) {
            continue;
        }
        let id = a["id"].as_str().unwrap_or("").to_owned();
        // A gate that failed for a cause outside its patch kept its yes,
        // and runs again a minute later (gp_mcp::asks::keep_yes).
        if gp_mcp::asks::kept_waits(&a, now) {
            continue;
        }
        // A gate's patch made on top of cards not landed (`--after`) runs
        // after them: held, its yes kept alive, while one waits; failed
        // with why once one was refused.
        if let Some(why) = hold_for_after(&home, &a, now) {
            if let Err(e) = why
                && gp_mcp::asks::claim_for_runner(&home, &id).is_some()
            {
                let _ = gp_mcp::asks::set_ran(&home, &id, "failed", &[e]);
            }
            continue;
        }
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
        let gp = yes_gp(&home, &a)
            .unwrap_or_else(|| std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gp")));
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
            // Gates all start at once (owner, yeses-land-together): those
            // of a kind hold their lock in turn (model gates gate.lock,
            // tool gates tool.lock, so a model gate and a tool gate run at
            // once: gate_hold::Lane) and the one holding it takes the
            // waiting ones of its kind into its batch (gate_queue), so
            // several yeses land in one run of the checks. Any other yes
            // runs one at a time.
            // A corpus import that writes runs as `gp model gate --import`:
            // in a tree of its own, landing on main through the gate, never
            // in the person's games checkout.
            let call = match import_as_gate(&a, &games) {
                Some(Ok(call)) => call,
                Some(Err(why)) => {
                    let _ = gp_mcp::asks::set_ran(&home, &id, "failed", &[why]);
                    return;
                }
                None => {
                    let mut call: Vec<String> = a["command"]
                        .as_str()
                        .unwrap_or("")
                        .split(' ')
                        .map(str::to_owned)
                        .collect();
                    call.extend(
                        a["argv"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|x| x.as_str().map(str::to_owned)),
                    );
                    call
                }
            };
            let _one = (!is_gate(&call[..call.len().min(2)].join(" ")))
                .then(|| ONE.lock().unwrap_or_else(|e| e.into_inner()));
            let mut args: Vec<String> = vec!["--json".into()];
            args.extend(call);
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
            // Its output goes to files beside the ask, not pipes, so a run
            // that outlives this server still ends with its words kept, and
            // the next server reads them (gp_mcp::asks::end_dead_runs).
            let (out_f, err_f) = gp_mcp::asks::run_files(&home, &id);
            let child = File::create(&out_f).and_then(|o| {
                let e = File::create(&err_f)?;
                Command::new(&gp)
                    .args(&args)
                    .current_dir(dir)
                    .env("GP_HOME", &home)
                    .env("GP_GAMES_DIR", &games)
                    .env("GP_NO_HINTS", "1")
                    .stdin(Stdio::null())
                    .stdout(o)
                    .stderr(e)
                    .spawn()
            });
            let (state, lines, outside) = match child {
                Ok(mut c) => {
                    let _ = gp_mcp::asks::set_child(&home, &id, c.id());
                    let ok = c.wait().map(|s| s.success()).unwrap_or(false);
                    let out = fs::read(&out_f).unwrap_or_default();
                    let err = fs::read(&err_f).unwrap_or_default();
                    let (state, mut lines) = gp_mcp::asks::outcome(Some(ok), &out, &err);
                    lines.push(format!("(ran in {})", dir.display()));
                    (state, lines, gp_mcp::asks::not_the_patch(&out))
                }
                Err(e) => ("failed", vec![format!("it couldn't start: {e}")], None),
            };
            // A gate that couldn't land for a cause outside its patch keeps
            // the person's yes and runs again; only a failed check, or a
            // patch that doesn't apply, uses it.
            let mut lines = lines;
            if state == "failed"
                && let Some(why) = outside
            {
                if gp_mcp::asks::keep_yes(&home, &id, &why) == Ok(true) {
                    let _ = fs::remove_file(&out_f);
                    let _ = fs::remove_file(&err_f);
                    return;
                }
                lines.push(format!(
                    "{} (it couldn't land {} times, a minute apart)",
                    why.trim_end_matches('.'),
                    gp_mcp::asks::KEPT_TRIES + 1
                ));
            }
            let _ = gp_mcp::asks::set_ran(&home, &id, state, &lines);
            let _ = fs::remove_file(&out_f);
            let _ = fs::remove_file(&err_f);
        });
    }
}

/// A yes to `gp corpus import GAME --write ...` (a --replace or --remove
/// asks) as the call the runner makes instead: `model gate GAMES/GAME
/// --import` with the same clips (`--replaces`, `--removes`), sessions
/// and changelog line, so the import is made on main's tip in the gate's
/// own tree and lands through the gate, where run as asked it wrote the
/// person's games checkout outside any gate (ask 1795591778060-0d6074b2).
/// None for any other ask, and for an import that doesn't write; Err for
/// one with what the gate can't take (`--from`, `--date`).
fn import_as_gate(a: &Value, games: &Path) -> Option<Result<Vec<String>, String>> {
    if a["command"] != json!("corpus import") {
        return None;
    }
    let argv: Vec<&str> = a["argv"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !argv.contains(&"--write") {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    let mut folder: Option<PathBuf> = None;
    let mut game: Option<&str> = None;
    let mut it = argv.iter();
    while let Some(&x) = it.next() {
        let (flag, inline) = match x.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (x, None),
        };
        if flag == "--write" {
            continue;
        }
        if !flag.starts_with("--") {
            game = Some(x);
            continue;
        }
        let Some(v) = inline.or_else(|| it.next().copied()) else {
            return Some(Err(format!("gp corpus import's {flag} has no value")));
        };
        match flag {
            "--replace" => out.extend(["--replaces".into(), v.to_owned()]),
            "--remove" => out.extend(["--removes".into(), v.to_owned()]),
            "--session" | "--says" => out.extend([flag.to_owned(), v.to_owned()]),
            "--template" => folder = Some(PathBuf::from(v)),
            _ => {
                return Some(Err(format!(
                    "gp corpus import's {flag} has no place in gp model gate --import, which lands the import on main: ask again without it"
                )));
            }
        }
    }
    let Some(dir) = folder.or_else(|| game.map(|g| games.join(g))) else {
        return Some(Err("gp corpus import names no game".into()));
    };
    let mut call = vec![
        "model".to_owned(),
        "gate".to_owned(),
        dir.to_string_lossy().into_owned(),
        "--import".to_owned(),
    ];
    call.extend(out);
    Some(Ok(call))
}

/// The values of `flag` in an ask's arguments.
fn arg_values(a: &Value, flag: &str) -> Vec<String> {
    let argv: Vec<&str> = a["argv"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    argv.windows(2)
        .filter(|w| w[0] == flag)
        .map(|w| w[1].to_owned())
        .collect()
}

/// Whether a yes to a model gate with `--after` waits: Some(Ok) while a
/// card it stands on hasn't landed (its yes is kept alive, `after_held_ms`,
/// and the card says why in `after_waits`), Some(Err(why)) once one was
/// refused, None when it runs now.
fn hold_for_after(home: &Path, a: &Value, now: u64) -> Option<Result<(), String>> {
    if a["command"] != json!("model gate") {
        return None;
    }
    let after = arg_values(a, "--after");
    if after.is_empty() {
        return None;
    }
    let id = a["id"].as_str().unwrap_or("");
    match crate::gate::stands_on(home, &after, now) {
        crate::gate::StandsOn::Landed(_) => None,
        crate::gate::StandsOn::Refused(why) => Some(Err(why)),
        crate::gate::StandsOn::Waiting(why) => {
            let _ = gp_mcp::asks::mark(home, id, "after_held_ms", json!(now));
            if a["after_waits"] != json!(why) {
                let _ = gp_mcp::asks::mark(home, id, "after_waits", json!(why));
            }
            Some(Ok(()))
        }
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

/// For tests: card `ask-N`, made at `made_ms` by step `sN` (its worker
/// record touching `row:GAME/ROW`), with `extra`'s keys (its state, ran).
#[cfg(test)]
pub(crate) fn test_card(home: &Path, n: u64, made_ms: u64, game: &str, row: &str, extra: Value) {
    let step = format!("s{n:012x}");
    fs::create_dir_all(loop_state::workers_dir(home)).unwrap();
    fs::write(
        file(home, &step),
        json!({"step": {"id": step, "touches": [format!("row:{game}/{row}")]}}).to_string(),
    )
    .unwrap();
    let mut a = json!({"id": format!("ask-{n}"), "made_ms": made_ms, "step": step, "game": game,
        "command": "tool gate", "argv": ["--patch", "fix.patch", "-m", "The import keeps the arcade buttons. Expect: x"]});
    for (k, v) in extra.as_object().unwrap() {
        a[k] = v.clone();
    }
    let dir = gp_mcp::asks::dir(home);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("ask-{n}.json")), a.to_string()).unwrap();
}

/// How a card stands for its row: "in play" while it waits for the
/// person's answer, holds their yes or runs on it; else how it ended:
/// landed, passed, failed, no (the person's NO) or ran out.
pub(crate) fn card_stands(a: &Value, now: u64) -> &'static str {
    match a["ran"]["state"].as_str() {
        Some("landed") => "landed",
        Some("passed") => "passed",
        Some("failed") => "failed",
        Some(_) => "in play",
        None if gp_mcp::asks::is_open(a, now) || gp_mcp::asks::is_yes(a, now) => "in play",
        None if a["state"] == json!("no") => "no",
        None => "ran out",
    }
}

/// The newest card of row `row` of `game` and how it stands
/// ([`card_stands`]), or None when the row has no card on file.
pub(crate) fn row_card(
    home: &Path,
    game: &str,
    row: &str,
    now: u64,
) -> Option<(Value, &'static str)> {
    gp_mcp::asks::all(home)
        .into_iter()
        .find(|a| ask_row(home, a).is_some_and(|(g, r)| g == game && r == row))
        .map(|a| {
            let s = card_stands(&a, now);
            (a, s)
        })
}

/// Whether row `r` asks the person about a card: asking you, with no clip
/// to watch, and a note that talks of a card (press YES, the card is up).
/// Such a row has nothing to ask once its card is gone.
fn asks_of_card(r: &Value) -> bool {
    let note = r["note"].as_str().unwrap_or("");
    r["status"] == json!("asking you")
        && r.get("clip").is_none()
        && (note.to_lowercase().contains("card") || note.contains("YES"))
}

/// How long a row asking the person about a card, with no card of it on
/// file, stays as it is before the loop takes it up again.
const CARD_ASK_WITHOUT_CARD_MS: u64 = 10 * 60 * 1000;

/// Each row asking the person about a card ([`asks_of_card`]) whose card is
/// gone follows its newest card as settle_card_rows has it (landed: done;
/// failed, NO, ran out: next), even when the row changed after the card
/// ended; one with no card on file for ten minutes goes to next, so the
/// loop takes it up again. On 2026-11-23 the fix card of
/// trouble-the-page-showed-the-person-an-error-78288a landed while its
/// worker still ran, the worker then set the row asking you ("a card to
/// land a fix"), and YOUR TURN asked the owner "Its card is no longer on
/// your page. What should happen with it?". Returns the rows it moved.
fn settle_card_asks(home: &Path, now: u64) -> Vec<(String, String)> {
    let mut moved = Vec::new();
    let held: BTreeSet<String> = loop_state::workers(home)
        .iter()
        .filter(|w| loop_state::holds(w))
        .flat_map(|w| w["step"]["touches"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    let Ok(dir) = fs::read_dir(home.join("mechanics")) else {
        return moved;
    };
    let mut games: Vec<String> = dir
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|g| crate::mechanic::session::name_ok(g))
        .collect();
    games.sort();
    for game in games {
        let f = home.join("mechanics").join(&game).join("queue.json");
        let rows: Vec<String> = crate::mechanic::session::read_queue(&f)
            .iter()
            .filter(|r| asks_of_card(r))
            .filter_map(|r| r["mechanic"].as_str().map(str::to_owned))
            .filter(|m| !held.contains(&format!("row:{game}/{m}")))
            .collect();
        for row in rows {
            let settled = match row_card(home, &game, &row, now) {
                Some((_, "in play")) => continue,
                Some((a, kind)) => {
                    let ended = match kind {
                        "landed" | "passed" | "failed" => a["ran"]["done_ms"].as_u64(),
                        "no" => a["answered_ms"].as_u64(),
                        _ => a["made_ms"].as_u64().map(|m| m + gp_mcp::asks::OPEN_FOR_MS),
                    }
                    .unwrap_or(0)
                    .min(now);
                    settle_row_by_card(home, &game, &row, kind, ended, &card_what(&a), &a)
                }
                None => no_card_left(&f, &row, now),
            };
            if let Ok(true) = settled {
                moved.push((game.clone(), row));
            }
        }
    }
    moved
}

/// Row `row` of queue `f`, asking about a card there is none of: once it
/// has stood so for [`CARD_ASK_WITHOUT_CARD_MS`], it goes to next.
fn no_card_left(f: &Path, row: &str, now: u64) -> anyhow::Result<bool> {
    let mut q = crate::mechanic::session::read_queue(f);
    let Some(r) = q.iter_mut().find(|r| r["mechanic"] == json!(row)) else {
        return Ok(false);
    };
    let since = r["changed_ms"].as_u64().unwrap_or(0);
    if now.saturating_sub(since) < CARD_ASK_WITHOUT_CARD_MS {
        return Ok(false);
    }
    let Some(o) = r.as_object_mut() else {
        return Ok(false);
    };
    o.insert("status".into(), json!("next"));
    o.insert(
        "note".into(),
        json!("It asked you about a card that was never on your page. The loop takes it up again."),
    );
    o.remove("worker");
    crate::mechanic::session::write_queue(f, &q)?;
    Ok(true)
}

/// How long after a card ended its row still follows it: cards that ended
/// before rows followed them were settled by hand.
const CARD_SETTLES_FOR_MS: u64 = 24 * 60 * 60 * 1000;

/// What a card asks to do, in a few words: its command's message up to
/// `Expect:` (a gate's), without the game's name in front, else its sentence.
pub(crate) fn card_what(a: &Value) -> String {
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
/// that ran out with no answer, sends it to next too, with their words. A
/// row an agent works on waits
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
/// (owner, page-status-matches-log), unless it asks the person about the
/// card ([`asks_of_card`]).
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
    // LOOKS GOOD, KEEP GOING (a NO with keep_going): the row is iterating
    // from now, so its worker's gates are refused (gp_mcp::iterating) even
    // while one still works on it.
    let keep_going = kind == "no" && a["keep_going"] == json!(true);
    if keep_going && r.get("iterating").is_none() {
        r["iterating"] = json!(KEPT_GOING);
        if r["status"] == json!("agent working") {
            crate::mechanic::session::write_queue(&f, &q)?;
            return Ok(false);
        }
    }
    if r["status"] == json!("agent working") {
        return Ok(false);
    }
    // A row asking the person about its card once the card is gone has
    // nothing left to ask, so it follows the card however late it was set
    // (settle_card_asks). The person's LOOKS GOOD, KEEP GOING parks it
    // however it was changed since: it is their newest word on it.
    if r["changed_ms"].as_u64().is_some_and(|c| c > ended) && !asks_of_card(r) && !keep_going {
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
    // What the person wrote beside their answer, kept in the note (it is in
    // the row's talk too, POST /api/asks/ID).
    let words = a["words"].as_str().map(str::trim).filter(|w| !w.is_empty());
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
        // No card is left on the page for these, so the row doesn't wait on
        // the person: the loop takes it up again (ask-only-about-looks).
        "no" => (
            "next",
            format!(
                "You said NO to its card ({what}). The loop takes it up again; say on the row what should change."
            ),
        ),
        _ => (
            "next",
            format!(
                "Its card ran out unanswered ({what}). The loop takes it up again and raises a new card."
            ),
        ),
    };
    // A NO with words sends the row back with them, for its worker to make
    // the fix again; a YES with words lands and keeps them on the row.
    let note = match (kind, words) {
        ("no", Some(w)) => format!(
            "You said NO to its card ({what}) and wrote: \"{w}\". The loop takes it up again and makes the fix you asked for."
        ),
        (_, Some(w)) => format!("{note} Beside your YES you wrote: \"{w}\"."),
        (_, None) => note,
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
    // A NO that names the card it duplicates (DUPLICATE OF on the card,
    // `duplicate_of` on POST /api/asks/ID) parks the row on that card's row
    // instead of sending it back to make the same card again (2026-11-22:
    // three cards for one row).
    let dup = match kind {
        "no" => duplicate_of(home, a, game, row),
        _ => None,
    };
    let (status, note) = match &dup {
        Some(d) => ("in line", d.note(what, words)),
        None if keep_going => ("in line", kept_going_note(what, words)),
        None => (status, note),
    };
    let Some(o) = r.as_object_mut() else {
        return Ok(true);
    };
    if keep_going && dup.is_none() {
        // Parked from now (words before don't wake it; words after do, and
        // its worker may go on with anything but a gate).
        o.insert("parked_ms".into(), json!(gp_mcp::asks::now_ms()));
        o.insert("parked".into(), json!(KEPT_GOING));
        o.remove("waits_on");
        o.remove("waits_since");
        o.insert(
            "plain".into(),
            json!("You said it looks good: nothing landed, and it waits for your LAND IT."),
        );
    }
    if let Some(d) = &dup {
        // Parked from now: words the person wrote before don't wake it.
        o.insert("parked_ms".into(), json!(gp_mcp::asks::now_ms()));
        match &d.on {
            Some(on) => {
                o.insert("waits_on".into(), json!(on));
                o.insert("waits_since".into(), json!(gp_mcp::asks::now_ms()));
                o.remove("parked");
            }
            None => {
                let why = format!("its card duplicated the card {}", d.other);
                o.insert("parked".into(), json!(why));
            }
        }
    }
    if status == "in line" && o.contains_key("target") && dup.is_none() && !keep_going {
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

/// Why a row is iterating once the person pressed LOOKS GOOD, KEEP GOING on
/// its card (its `iterating` and `parked`).
const KEPT_GOING: &str = "you said LOOKS GOOD, KEEP GOING on its card";

/// The row's note after LOOKS GOOD, KEEP GOING on its card (`what`), with
/// the person's words.
fn kept_going_note(what: &str, words: Option<&str>) -> String {
    let mut n = format!(
        "You said LOOKS GOOD, KEEP GOING to its card ({what}): nothing landed. The work stays where it is, and no worker gates it until you press LAND IT on this row; then it is gated as one patch, on one card."
    );
    if let Some(w) = words {
        n.push_str(&format!(" You wrote: \"{w}\"."));
    }
    n
}

/// A card the person refused as a duplicate of another: the other card's
/// id and what it does, and the row it is for (`GAME/ROW`) when that is
/// another row, which the refused card's row then waits on.
struct Duplicate {
    other: String,
    other_what: String,
    on: Option<String>,
}

impl Duplicate {
    /// The refused card's row's note.
    fn note(&self, what: &str, words: Option<&str>) -> String {
        let mut n = match &self.on {
            Some(on) => format!(
                "You said NO to its card ({what}): it duplicates the card for {on} ({}). It waits on {on}, and the loop takes it up again once that card is used, refused or gone.",
                self.other_what
            ),
            None => format!(
                "You said NO to its card ({what}): it duplicates the card {} ({}). It waits in line; set it next to take it up again.",
                self.other, self.other_what
            ),
        };
        if let Some(w) = words {
            n.push_str(&format!(" You wrote: \"{w}\"."));
        }
        n
    }
}

/// The card ask `a` (of row `row` of `game`) was refused as a duplicate
/// of, when its NO named one (`duplicate_of`) that is still on file.
fn duplicate_of(home: &Path, a: &Value, game: &str, row: &str) -> Option<Duplicate> {
    let other = a["duplicate_of"].as_str().filter(|d| !d.is_empty())?;
    let b = gp_mcp::asks::read(home, other)?;
    let on = ask_row(home, &b)
        .filter(|(g, r)| !(g == game && r == row))
        .map(|(g, r)| format!("{g}/{r}"));
    Some(Duplicate {
        other: other.to_owned(),
        other_what: card_what(&b),
        on,
    })
}

/// How often the runner archives finished rows, and wakes the rows whose
/// wait is over.
const ARCHIVE_EVERY: Duration = Duration::from_secs(60);

/// Moves each row parked in line on another row to next once that row's
/// card landed, or it is done or archived (crate::mechanic::waits), with
/// the landing named; a row with its own card still open or unsettled
/// waits for it.
fn wake_waiting(home: &Path, games: Option<&Path>, now: u64) -> Vec<(String, String)> {
    let recent = gp_mcp::asks::OPEN_FOR_MS + CARD_SETTLES_FOR_MS;
    let mut cards = crate::mechanic::waits::Cards::default();
    let mut newest: BTreeMap<(String, String), u64> = BTreeMap::new();
    for a in gp_mcp::asks::all(home)
        .into_iter()
        .filter(|a| now.saturating_sub(a["made_ms"].as_u64().unwrap_or(0)) < recent)
    {
        let Some(row) = ask_row(home, &a) else {
            continue;
        };
        if a["row_settled"] != json!(true) {
            cards.busy.insert(row.clone());
        }
        let made = a["made_ms"].as_u64().unwrap_or(0);
        if a["ran"]["state"] == json!("landed") && newest.get(&row).is_none_or(|m| made > *m) {
            newest.insert(row.clone(), made);
            let commit = crate::mechanic::archive::commit_of(&a["ran"]["lines"]);
            let done = &a["ran"]["done_ms"];
            let l = json!({"what": card_what(&a), "commit": commit, "done_ms": done});
            cards.landed.insert(row, l);
        }
    }
    crate::mechanic::waits::wake(home, games, &cards)
}

/// Moves each game's rows done for an hour, with no live worker and no
/// card still open or unsettled, from its quest log to its archive
/// (crate::mechanic::archive), with the commit the row's newest landed
/// card names.
fn archive_finished(home: &Path, now: u64) {
    use crate::mechanic::archive::{Beside, commit_of, sweep};
    let recent = gp_mcp::asks::OPEN_FOR_MS + CARD_SETTLES_FOR_MS;
    let cards: Vec<(Value, (String, String))> = gp_mcp::asks::all(home)
        .into_iter()
        .filter(|a| now.saturating_sub(a["made_ms"].as_u64().unwrap_or(0)) < recent)
        .filter_map(|a| {
            let row = ask_row(home, &a)?;
            Some((a, row))
        })
        .collect();
    let held: BTreeSet<String> = loop_state::workers(home)
        .iter()
        .filter(|w| loop_state::holds(w))
        .flat_map(|w| w["step"]["touches"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    let Ok(dir) = fs::read_dir(home.join("mechanics")) else {
        return;
    };
    for e in dir.flatten() {
        let game = e.file_name().to_string_lossy().into_owned();
        if !crate::mechanic::session::name_ok(&game) || !e.path().join("queue.json").is_file() {
            continue;
        }
        let beside = |row: &str| {
            let mine = || {
                cards
                    .iter()
                    .filter(|(_, (g, r))| *g == game && r == row)
                    .map(|(a, _)| a)
            };
            let landed = mine()
                .filter(|a| a["ran"]["state"] == json!("landed"))
                .max_by_key(|a| a["made_ms"].as_u64().unwrap_or(0))
                .map(|a| {
                    json!({
                        "card": a["id"],
                        "what": card_what(a),
                        "commit": commit_of(&a["ran"]["lines"]),
                    })
                });
            Beside {
                worker: held.contains(&format!("row:{game}/{row}")),
                card: mine().any(|a| a["row_settled"] != json!(true)),
                landed,
            }
        };
        let _ = sweep(home, &game, now, beside);
    }
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
    gate_said(out)
}

/// A `gp --json model gate` run's state (landed, passed or failed), its
/// lines, and the clips it refused (`refused`).
fn gate_said(
    out: std::io::Result<std::process::Output>,
) -> (&'static str, Vec<String>, Vec<Value>) {
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

/// The rows whose confirmed look this server locks now (`GAME/ROW`).
static LOCKING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
/// How long a confirmed look whose clip isn't in the corpus yet waits
/// before it is tried again: the clip keeping takes clips in that often.
const LOCK_AGAIN_MS: u64 = 120_000;
/// How many tries a confirmed look gets before its row goes back to next.
const LOCK_TRIES: u64 = 30;

/// How locking one confirmed look ended.
#[derive(Debug, PartialEq)]
enum Locked {
    /// Landed (or the same lock was there): the clip, its frames, how many
    /// frames are locked, and whether a commit landed.
    Yes {
        clip: String,
        frames: Value,
        n: u64,
        landed: bool,
    },
    /// Its clip isn't in the corpus yet: tried again later.
    Wait(String),
    /// It can't be locked: why.
    No(String),
}

/// The owner's yeses to looks, locked by the runner itself (owner,
/// yes-on-a-look-locks-it): each quest-log row a yes marked `confirmed`
/// (crate::approved::confirm: CONFIRM on the page, or a typed plain yes)
/// gets `gp mechanic approve` on a worktree of the games tip, and the
/// approved.toml it writes lands through `gp model gate`, no card (the
/// yes was the owner's) and no worker. The row goes to done with the
/// frames it locked (`locked`); a clip not in the corpus yet waits for the
/// clip keeping and is tried again; any other failure sends the row to
/// next with why.
fn run_confirms(ctx: &Arc<Ctx>) {
    let now = gp_mcp::asks::now_ms();
    for game in crate::next::games_in(&ctx.games) {
        let Ok(qf) = crate::mechanic::session::queue_file(&ctx.paths, &game) else {
            continue;
        };
        for r in crate::mechanic::session::read_queue(&qf) {
            let c = &r["confirmed"];
            let Some(row) = r["mechanic"].as_str().filter(|_| c.is_object()) else {
                continue;
            };
            if c["again_ms"].as_u64().is_some_and(|t| t > now) {
                continue;
            }
            let key = format!("{game}/{row}");
            if !LOCKING
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key.clone())
            {
                continue;
            }
            // Counted as a yes, so a move onto a new gp waits for it.
            YESES_RUNNING.fetch_add(1, Ordering::SeqCst);
            if super::reload::sealed() {
                YESES_RUNNING.fetch_sub(1, Ordering::SeqCst);
                LOCKING
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&key);
                return;
            }
            let (ctx, game, row, qf) = (ctx.clone(), game.clone(), row.to_owned(), qf.clone());
            std::thread::spawn(move || {
                struct Ran(String);
                impl Drop for Ran {
                    fn drop(&mut self) {
                        YESES_RUNNING.fetch_sub(1, Ordering::SeqCst);
                        LOCKING
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&self.0);
                    }
                }
                let _ran = Ran(key);
                let out = lock_look(&ctx, &game, &row);
                let _ = settle_lock(&qf, &row, &out, gp_mcp::asks::now_ms());
            });
        }
    }
}

/// Each game's open rows whose look its approved.toml in the games checkout
/// has saved go done with what was saved (crate::approved::settle_saved):
/// a worker's model gate card that saved a look closes its row as the
/// runner's own lock does. Returns the rows closed, `GAME/ROW`.
fn settle_saved_rows(home: &Path, games: &Path, now: u64) -> Vec<String> {
    let mut out = Vec::new();
    for game in crate::next::games_in(games) {
        if !crate::mechanic::session::name_ok(&game) {
            continue;
        }
        let looks = crate::approved::saved_looks(Some(games), &game);
        let qf = home.join("mechanics").join(&game).join("queue.json");
        let closed = crate::approved::settle_saved(&qf, &game, &looks, now);
        out.extend(
            closed
                .unwrap_or_default()
                .into_iter()
                .map(|r| format!("{game}/{r}")),
        );
    }
    out
}

/// How many times a lock is measured on a new tip when the tip moved under
/// it, before it waits and is tried again later.
const LOCK_TIP_TRIES: usize = 3;

/// `gp mechanic approve GAME ROW` on a worktree of the games tip, and its
/// approved.toml landed through the model gate. A tip that moved while it
/// measured (another landing, the gate's [`crate::gate::STALE_LOCK`]) has
/// it measured again on the new tip: on 2026-11-23 a lock measured on the
/// build before games 620441d landed on top of it, and every later gate
/// failed on it.
fn lock_look(ctx: &Ctx, game: &str, row: &str) -> Locked {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = Locked::No("not tried".to_owned());
    for _ in 0..LOCK_TIP_TRIES {
        out = lock_look_once(ctx, game, row);
        match &out {
            Locked::No(why) if why.contains(crate::gate::STALE_LOCK) => {}
            _ => return out,
        }
    }
    match out {
        Locked::No(why) => Locked::Wait(why),
        o => o,
    }
}

/// One try of [`lock_look`].
fn lock_look_once(ctx: &Ctx, game: &str, row: &str) -> Locked {
    let home = &ctx.paths.home;
    let gp = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gp"));
    let locks = loop_state::loop_dir(home).join("locks");
    let wt = locks.join(format!("{game}-{row}"));
    let patch = locks.join(format!("{game}-{row}.patch"));
    let games = &ctx.games;
    let w = wt.to_string_lossy().into_owned();
    let clear = || {
        let _ = crate::gate::git(games, &["worktree", "remove", "--force", &w]);
        let _ = fs::remove_dir_all(&wt);
        let _ = crate::gate::git(games, &["worktree", "prune"]);
    };
    clear();
    let _ = fs::create_dir_all(&locks);
    if let Err(e) = crate::gate::git(games, &["worktree", "add", "--detach", &w, "HEAD"]) {
        return Locked::No(format!("no worktree of the games tip: {e:#}"));
    }
    let out = approve_and_gate(ctx, &gp, game, row, &wt, &patch);
    clear();
    let _ = fs::remove_file(&patch);
    out
}

/// [`lock_look`]'s work in the worktree `wt`: the approve, its diff as
/// `patch`, and the gate.
fn approve_and_gate(
    ctx: &Ctx,
    gp: &Path,
    game: &str,
    row: &str,
    wt: &Path,
    patch: &Path,
) -> Locked {
    let (home, games) = (&ctx.paths.home, &ctx.games);
    let at = crate::gate::git(wt, &["rev-parse", "HEAD"]).unwrap_or_default();
    let o = Command::new(gp)
        .args(["--json", "mechanic", "approve", game, row, "--games"])
        .arg(wt)
        .current_dir(wt)
        .env("GP_HOME", home)
        .env("GP_GAMES_DIR", wt)
        .env("GP_NO_HINTS", "1")
        .stdin(Stdio::null())
        .output();
    let o = match o {
        Ok(o) => o,
        Err(e) => return Locked::No(format!("gp mechanic approve couldn't start: {e}")),
    };
    if !o.status.success() {
        let why = String::from_utf8_lossy(&o.stderr);
        let why = why.trim().trim_start_matches("gp: ").to_owned();
        return if why.contains("import it first") || why.contains("the corpus has no clip") {
            Locked::Wait(why)
        } else {
            Locked::No(why)
        };
    }
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
    let clip = v["clip"].as_str().unwrap_or("").to_owned();
    let frames = v["frames"].clone();
    let n = v["frames_locked"].as_u64().unwrap_or(0);
    // Only the game's folder: git-lfs writes its hooks (post-checkout and
    // the like) into the new worktree's .githooks, which the model gate
    // refuses as outside the game.
    let folder = crate::labels::game_folder(wt, game)
        .and_then(|d| {
            d.strip_prefix(wt)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| game.to_owned());
    let _ = crate::gate::git(wt, &["add", "-A", "--", &folder]);
    let diff = match crate::gate::git(wt, &["diff", "--cached", "--binary", "--", &folder]) {
        Ok(d) => d,
        Err(e) => return Locked::No(format!("{e:#}")),
    };
    if diff.trim().is_empty() {
        return Locked::Yes {
            clip,
            frames,
            n,
            landed: false,
        };
    }
    // The game changed on the tip while it was measured: measure it again
    // there (lock_look) rather than land a lock of the build before.
    let tip = crate::gate::git(games, &["rev-parse", "HEAD"]).unwrap_or_default();
    let at = at.trim();
    if !at.is_empty() && tip.trim() != at {
        let range = format!("{at}..{}", tip.trim());
        let moved = crate::gate::git(games, &["diff", "--name-only", &range, "--", &folder]);
        if moved.is_ok_and(|m| !m.trim().is_empty()) {
            return Locked::No(format!(
                "{}: {game} changed on the games tip while it was measured",
                crate::gate::STALE_LOCK
            ));
        }
    }
    if let Err(e) = fs::write(patch, format!("{diff}\n")) {
        return Locked::No(format!("the patch couldn't be written: {e}"));
    }
    let [a, b] = [0, 1].map(|i| frames[i].as_u64().unwrap_or(0));
    let msg = format!(
        "{game}: lock the look the owner said yes to on {row} ({clip} frames {a}-{b}, {n} frames). Expect: the model gate fails a change that draws one of those frames further from the original. Checked: gp mechanic approve on the games tip, by gp serve's runner on the owner's yes"
    );
    let dir = crate::labels::game_folder(games, game).unwrap_or_else(|| games.join(game));
    let gated = Command::new(gp)
        .args(["--json", "model", "gate"])
        .arg(&dir)
        .arg("--patch")
        .arg(patch)
        .arg("-m")
        .arg(&msg)
        .current_dir(games)
        .env("GP_HOME", home)
        .env("GP_GAMES_DIR", games)
        .env("GP_NO_HINTS", "1")
        .stdin(Stdio::null())
        .output();
    match gate_said(gated) {
        ("landed", ..) => Locked::Yes {
            clip,
            frames,
            n,
            landed: true,
        },
        (_, lines, _) => Locked::No(format!(
            "the model gate didn't land it: {}",
            lines.last().map_or("no reason given", String::as_str)
        )),
    }
}

/// Row `row` of the quest log `qf` after its confirmed look was locked as
/// `out`. A row whose yes was taken back (no `confirmed`) is left as it is.
fn settle_lock(qf: &Path, row: &str, out: &Locked, now: u64) -> anyhow::Result<()> {
    use crate::mechanic::session::{read_queue, write_queue};
    let mut q = read_queue(qf);
    let Some(o) = q
        .iter_mut()
        .find(|r| r["mechanic"] == json!(row) && r["confirmed"].is_object())
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };
    let tries = o["confirmed"]["tries"].as_u64().unwrap_or(0) + 1;
    let failed = |why: &str| {
        format!(
            "You said it looks right, but it couldn't be saved: {}. The loop tries again.",
            crate::next::cut(why.trim_end_matches('.'), 400)
        )
    };
    match out {
        Locked::Yes {
            clip,
            frames,
            n,
            landed,
        } => {
            let [a, b] = [0, 1].map(|i| frames[i].as_u64().unwrap_or(0));
            let how = if *landed {
                "Saved as right"
            } else {
                "Already saved as right"
            };
            let note = format!("{how}: {row} ({clip} frames {a} to {b}).");
            let target = o.get("target").and_then(Value::as_str).map(str::to_owned);
            let (status, note) = match target {
                Some(t) => {
                    o.insert("awaiting_measure".into(), json!(now));
                    (
                        "in line",
                        format!("{note} It closes itself when its target is met ({t})."),
                    )
                }
                None => ("done", note),
            };
            o.insert("status".into(), json!(status));
            o.insert("note".into(), json!(note));
            o.insert(
                "locked".into(),
                json!({"clip": clip, "frames": frames, "frames_locked": n, "at_ms": now}),
            );
            o.remove("confirmed");
            o.remove("clip");
        }
        Locked::Wait(why) if tries < LOCK_TRIES => {
            let c = &mut o["confirmed"];
            c["tries"] = json!(tries);
            c["again_ms"] = json!(now + LOCK_AGAIN_MS);
            c["why"] = json!(why);
            let why = crate::next::cut(why.trim_end_matches('.'), 300);
            let note = format!(
                "You said it looks right. It is saved once the loop has taken its clip in, which it does every few minutes ({why})."
            );
            o.insert("note".into(), json!(note));
        }
        Locked::Wait(why) | Locked::No(why) => {
            o.insert("status".into(), json!("next"));
            o.insert("note".into(), json!(failed(why)));
            o.remove("confirmed");
        }
    }
    o.insert("changed_ms".into(), json!(now));
    write_queue(qf, &q)
}

/// The quest-log row a corpus clip's ID names: its session,
/// `drink-far-end` of `rooftop.drink-far-end.001`.
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
    let Some(setup) = setup_of(ctx) else {
        return;
    };
    // A game's port run ends once its rows are done and no worker is on it:
    // its next worker starts on the newest gp (crate::port_run).
    crate::port_run::end_finished(home, &loop_state::workers(home));
    let v = crate::loop_cmd::steps_json(&ctx.paths, Some(&ctx.games));
    let open = v["open"].as_array().cloned().unwrap_or_default();
    let todo = plan(
        &open,
        &loop_state::workers(home),
        &running_ids(),
        s.runner.max_workers as usize,
        gp_mcp::asks::now_ms(),
    );
    // Fewer workers than allowed and no row for another: the loop is idle,
    // and gp serve's finder (crate::scoreboard::find_work) makes rows of the
    // gaps on the scoreboard's lines that aren't full. The trouble filer's
    // checks run at once, so a problem that would make a row makes it now.
    let live = loop_state::workers(home)
        .iter()
        .filter(|w| loop_state::holds(w))
        .count();
    if todo.is_empty() && live < s.runner.max_workers as usize {
        let was = idle();
        IDLE_MS.store(gp_mcp::asks::now_ms(), Ordering::SeqCst);
        if !was {
            let _ = crate::loop_cmd::health_in(&ctx.paths, Some(&ctx.games));
        }
    }
    for step in todo {
        spawn(setup.clone(), step);
    }
}

/// When the runner last looked and found fewer workers at work than
/// allowed and no step to start.
static IDLE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Whether the runner is idle now: its last look, within the last half
/// minute, found room for a worker and nothing to start.
pub(crate) fn idle() -> bool {
    let at = IDLE_MS.load(Ordering::SeqCst);
    at > 0 && gp_mcp::asks::now_ms().saturating_sub(at) < 30_000
}

/// What a worker is started with, when an agent command is set and found.
fn setup_of(ctx: &Ctx) -> Option<Setup> {
    let cs = clip_agent::Settings::current(ctx);
    let Mode::Command(argv) = cs.mode.clone() else {
        return None;
    };
    let program = argv.first().and_then(|p| clip_agent::find_program(p))?;
    Some(Setup {
        home: ctx.paths.home.clone(),
        games: ctx.games.clone(),
        repo: cs.repo.clone(),
        tools: ctx.repo.clone(),
        gp: cs.gp.clone(),
        argv,
        program,
        timeout: cs.timeout,
        tries: TRIES,
        again_after: Duration::from_secs(10),
    })
}

/// Starts a worker on `step` in a thread of its own.
pub(crate) fn spawn(setup: Setup, step: Value) -> std::thread::JoinHandle<()> {
    let id = step["id"].as_str().unwrap_or("").to_owned();
    let before = fs::read(file(&setup.home, &id))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let rec = json!({
        "step": step,
        "state": "working",
        "why": "",
        "worker": format!("loop-{id}"),
        "started_ms": gp_mcp::asks::now_ms(),
        "tries": 0,
        "runs": before.as_ref().map_or(0, runs) + 1,
    });
    write(&setup.home, &rec);
    watched(setup, rec, false)
}

/// Takes up a worker the server before left running on its own (or that
/// ended with its exit written since): it is watched here as if started
/// here, its tries going on from the one its process is.
pub(crate) fn adopt(setup: Setup, rec: Value) -> std::thread::JoinHandle<()> {
    watched(setup, rec, true)
}

fn watched(setup: Setup, mut rec: Value, resume: bool) -> std::thread::JoinHandle<()> {
    let id = rec["step"]["id"].as_str().unwrap_or("").to_owned();
    let live = Arc::new(Live::default());
    // An adopted worker's process runs on its own from the start: a move
    // right after this one needn't wait for it.
    live.out.store(resume, Ordering::SeqCst);
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), live.clone());
    std::thread::spawn(move || {
        if let Err(e) = work(&setup, &live, &mut rec, resume) {
            set_out(&live, false);
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
        // A worker the person stopped leaves its row parked in line, so no
        // worker takes it up again until they set it next (before its record
        // ends, so no look finds the step open with no worker).
        if rec["stopped_by"] == json!("person") {
            park_stopped_row(&setup.home, &rec);
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
    let last: Option<Value> = newest
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| {
            t.lines().rev().find_map(|l| {
                let v: Value = serde_json::from_str(l).ok()?;
                (v["type"] == json!("result")).then_some(v)
            })
        });
    // The worker's structured result (--json-schema), which code reads; the
    // report's prose is for the person's log.
    let result = last
        .as_ref()
        .map(|v| v["structured_output"].clone())
        .filter(|r| r["outcome"].is_string());
    let report = last
        .as_ref()
        .and_then(|v| v["result"].as_str().map(str::to_owned))
        .filter(|r| !r.trim().is_empty())
        .or_else(|| result.as_ref().map(note_of));
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
    let _ = settle_row(&setup.home, &game, &row, &said, result.as_ref());
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

/// The structured result a worker ends with (claude's --json-schema): the
/// runner reads it, not the report's prose, for the row's note, so the
/// same work gives the same note and the same next step.
pub(crate) const WORKER_RESULT_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["outcome","did","left"],"properties":{"outcome":{"type":"string","enum":["done","on a card","asking you","left","blocked"],"description":"done: nothing is left (a row with no target you set done yourself); on a card: a gate card waits for the person's YES; asking you: the row asks the person about a look with its clip; left: work is left for the next worker; blocked: the step can't go on"},"did":{"type":"string","description":"One or two sentences: what this step did and how it was checked"},"left":{"type":"string","description":"What is left for the next worker, or why it is blocked; empty when done"},"target_met":{"type":["boolean","null"],"description":"Whether the row's target measured as met, null when it has none"},"measures":{"type":"array","items":{"type":"string"},"description":"Measures taken, as LINE: VALUE"},"next_rows":{"type":"array","items":{"type":"string"},"description":"Rows this step proposed (gp mechanic propose), by name"}}}"#;

/// A row's note from a worker's structured result: what is left, or for a
/// row with nothing left what was done.
fn note_of(result: &Value) -> String {
    let s = |k: &str| result[k].as_str().map(str::trim).unwrap_or("").to_owned();
    let (did, left) = (s("did"), s("left"));
    let mut note = match result["outcome"].as_str().unwrap_or("") {
        "done" | "on a card" | "asking you" => did,
        "blocked" => format!("Blocked: {left}"),
        _ if left.is_empty() => did,
        _ => format!("Left: {left}"),
    };
    let rows: Vec<&str> = result["next_rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !rows.is_empty() {
        note.push_str(&format!(" Proposed: {}.", rows.join(", ")));
    }
    note
}

/// A row its worker left marked `agent working` when it ended: no agent is
/// on it any more, so it goes to `next`, with the note its worker's
/// structured result makes when it gave one, else from its report: what
/// the report says is left, and the report's questions with them for the
/// next worker to decide (only a look, with its clip on the row, asks the
/// person, and a worker sets that itself). No row is closed on a note's or
/// a report's wording, nor on a result's outcome: a row closes by its
/// target's measure, or by its worker setting it done (`gp mechanic queue
/// --status done`), so two runs of the same item end the same way whatever
/// words their workers chose. Any other row the worker moved itself (done,
/// asking you, next ...) stays as it is.
fn settle_row(
    home: &Path,
    game: &str,
    row: &str,
    report: &str,
    result: Option<&Value>,
) -> anyhow::Result<()> {
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
    // Only a look asks the owner, and a look comes with the clip the worker
    // put on the row itself (gp mechanic queue --clip): a question left in
    // a report is the next worker's to decide (owner, ask-only-about-looks).
    let note = match (result, asks_of(report)) {
        (Some(res), _) => note_of(res),
        (None, Some(a)) => format!(
            "The last worker asked: {a} It isn't the person's to answer: the next worker picks, says why in the note, and goes on. {}",
            left_of(report)
        ),
        (None, None) => left_of(report),
    };
    let status = "next";
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

/// Words naming what is left (or still open).
const LEFT_WORDS: &[&str] = &[
    "left:",
    "is left",
    "'s left",
    "still open",
    "remaining:",
    "left to do",
];

fn names_left(l: &str) -> bool {
    let l = l.to_lowercase();
    LEFT_WORDS.iter().any(|k| l.contains(k))
}

/// What the report says is left: from its line naming what is left (or
/// still open) to its end, else its last paragraph.
fn left_of(report: &str) -> String {
    let lines: Vec<&str> = report.lines().collect();
    let named = lines.iter().position(|l| names_left(l));
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

/// The worker's worktree, made from main the first time, with whether its
/// branch was made from main just now (so its pending patches go on it).
fn worktree(setup: &Setup, id: &str, games: bool) -> anyhow::Result<(PathBuf, String, bool)> {
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
        return Ok((path, branch, false));
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
    Ok((path, branch, !has_branch))
}

/// Starts the build folder `gp worker env` gives the worker in `wt` (on
/// `branch`) as a clone of the tool gate's (crate::warm::start).
fn warm_start(setup: &Setup, wt: &Path, branch: &str) -> Value {
    let Ok(gate) = crate::gate::gate_home(None) else {
        return json!({"warm": false, "why": "no gate home"});
    };
    let wt = fs::canonicalize(wt).unwrap_or_else(|_| wt.to_owned());
    let dst = crate::worker::target_of(&crate::builds::target_root(&setup.home), Some(branch), &wt);
    crate::warm::start(&gate, &dst, &wt)
}

/// One of a game's pending patches as a worker's tree took it: its card's
/// id, row, what it changes, and why it was left out when it didn't apply.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Pending {
    pub id: String,
    pub row: Option<String>,
    pub what: String,
    pub skipped: Option<String>,
}

/// Whether a model gate card is still pending: open, holding a yes, or
/// running.
fn is_pending(a: &Value, now: u64) -> bool {
    gp_mcp::asks::is_open(a, now) || gp_mcp::asks::is_yes(a, now) || a["state"] == json!("running")
}

/// `game`'s pending patches: its model gate cards open, holding a yes or
/// running, in the order they were raised.
pub(crate) fn pending_cards(home: &Path, game: &str, now: u64) -> Vec<Value> {
    let mut v: Vec<Value> = gp_mcp::asks::all(home)
        .into_iter()
        .filter(|a| {
            a["command"] == json!("model gate")
                && a["game"].as_str() == Some(game)
                && is_pending(a, now)
        })
        .collect();
    v.sort_by_key(|a| a["made_ms"].as_u64().unwrap_or(0));
    v
}

/// A card's patch: the file its `--patch` names, else the copy its look
/// kept (crate::mechanic::ask_look) once the worker's tree that wrote it
/// is gone.
fn card_patch(home: &Path, game: &str, a: &Value) -> Result<Vec<u8>, String> {
    let Some(p) = arg_values(a, "--patch").pop() else {
        return Err("its card names no --patch".into());
    };
    let id = a["id"].as_str().unwrap_or("");
    let kept = home
        .join("work/mechanic-preview")
        .join(game)
        .join("asks")
        .join(id)
        .join("ask.patch");
    fs::read(&p)
        .or_else(|_| fs::read(&kept))
        .map_err(|e| format!("its patch {p} can't be read: {e}"))
}

/// `git` in `dir` with the loop's own name on its commits, Err with git's
/// words.
fn git_in(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "gp loop")
        .env("GIT_AUTHOR_EMAIL", "loop@game-printer.invalid")
        .env("GIT_COMMITTER_NAME", "gp loop")
        .env("GIT_COMMITTER_EMAIL", "loop@game-printer.invalid")
        .stdin(Stdio::null())
        .output()?;
    if !o.status.success() {
        bail!(
            "git {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// The game's pending patches put on the worker's tree `wt` (its branch
/// just made from main), oldest raised first, each one a commit of its
/// own (git's plumbing: no hooks, nothing leaves the branch). A patch
/// that doesn't apply on what is below it is left out, with why. The
/// patches are kept in the worker's folder `dir`. Returns each card as it
/// was taken and the commit the worker's own change starts from.
fn stack_pending(
    home: &Path,
    wt: &Path,
    dir: &Path,
    game: &str,
    now: u64,
) -> anyhow::Result<(Vec<Pending>, String)> {
    let mut out = Vec::new();
    for a in pending_cards(home, game, now) {
        let id = a["id"].as_str().unwrap_or("").to_owned();
        let row = ask_row(home, &a).map(|(_, r)| r);
        let what = card_what(&a);
        let mut p = Pending {
            id: id.clone(),
            row,
            what,
            skipped: None,
        };
        let applied = card_patch(home, game, &a).and_then(|text| {
            let f = dir.join(format!("pending-{id}.patch"));
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            fs::write(&f, &text).map_err(|e| e.to_string())?;
            let f = f.to_string_lossy().into_owned();
            git_in(wt, &["apply", "--index", "--whitespace=nowarn", &f]).map_err(|e| {
                let e = format!("{e:#}");
                let e = e.lines().find(|l| l.contains("error")).unwrap_or(&e);
                format!(
                    "it doesn't apply on main with the cards before it ({})",
                    e.trim()
                )
            })?;
            let tree = git_in(wt, &["write-tree"]).map_err(|e| format!("{e:#}"))?;
            let msg = format!("pending card {id}: {}", p.what);
            let c = git_in(wt, &["commit-tree", &tree, "-p", "HEAD", "-m", &msg])
                .map_err(|e| format!("{e:#}"))?;
            git_in(wt, &["update-ref", "HEAD", &c]).map_err(|e| format!("{e:#}"))?;
            Ok(())
        });
        if let Err(why) = applied {
            // Nothing of it stays: the index and files as the last commit.
            let _ = git_in(wt, &["reset", "-q", "--hard", "HEAD"]);
            p.skipped = Some(why);
        }
        out.push(p);
    }
    let base = git_in(wt, &["rev-parse", "HEAD"])?;
    Ok((out, base))
}

/// A row with a target whose gap the pending patches on its tree `wt`
/// already close (its line measured there meets it): set in line, waiting
/// on the row of the newest such card (the gates land them in the order
/// they were raised, so that one lands last), with a note, and the why
/// for the worker's record. None when the row has no target, no patch
/// went on the tree, the measure can't be made, or it isn't met.
fn closed_by_pending(
    setup: &Setup,
    wt: &Path,
    dir: &Path,
    game: &str,
    row: &str,
    pending: &[Pending],
) -> Option<String> {
    let on: Vec<&Pending> = pending.iter().filter(|p| p.skipped.is_none()).collect();
    let last = on.last()?;
    let qf = setup.home.join("mechanics").join(game).join("queue.json");
    let target = crate::mechanic::session::read_queue(&qf)
        .into_iter()
        .find(|r| r["mechanic"] == json!(row))?["target"]
        .as_str()?
        .to_owned();
    let (met, reading) =
        crate::scoreboard::met_in(&setup.home, dir, wt, game, &target, &setup.gp).ok()?;
    if !met {
        return None;
    }
    let cards: Vec<String> = on.iter().map(|p| format!("card {}", p.id)).collect();
    let other = on
        .iter()
        .rev()
        .find(|p| p.row.as_deref().is_some_and(|r| r != row));
    let wait = other.and_then(|p| p.row.clone());
    let note = match &wait {
        Some(r) => format!(
            "Its gap is closed by the patches still on cards ({}): its target ({target}) measures {reading} on main with them, so it isn't worked again. It waits on {r}, whose card {} lands last of them, and closes on its measure once they land.",
            cards.join(", "),
            last.id
        ),
        None => format!(
            "Its own card {} closes its gap: its target ({target}) measures {reading} on main with it, so it isn't worked again. It closes on its measure once the card lands.",
            last.id
        ),
    };
    let mut q = crate::mechanic::session::read_queue(&qf);
    let r = q.iter_mut().find(|r| r["mechanic"] == json!(row))?;
    let o = r.as_object_mut()?;
    let now = gp_mcp::asks::now_ms();
    o.insert("status".into(), json!("in line"));
    o.insert("note".into(), json!(note));
    o.remove("worker");
    if let Some(w) = &wait {
        o.insert("waits_on".into(), json!(format!("{game}/{w}")));
        o.insert("waits_since".into(), json!(now));
    }
    o.insert("changed_ms".into(), json!(now));
    crate::mechanic::session::write_queue(&qf, &q).ok()?;
    Some(format!(
        "its row's target is met on main with the pending patches ({reading}): no worker needed"
    ))
}

/// The brief's words on the pending patches the worker's tree has.
fn pending_words(game: &str, pending: &[Pending], base: &str) -> String {
    let (on, off): (Vec<&Pending>, Vec<&Pending>) =
        pending.iter().partition(|p| p.skipped.is_none());
    if pending.is_empty() {
        return String::new();
    }
    let name = |p: &Pending| match &p.row {
        Some(r) => format!("card {} (row {r}): {}", p.id, p.what),
        None => format!("card {}: {}", p.id, p.what),
    };
    let mut b = Vec::new();
    if !on.is_empty() {
        let ids: Vec<&str> = on.iter().map(|p| p.id.as_str()).collect();
        b.push(format!(
            "Your worktree is main plus {game}'s pending patches, the model gate cards not landed yet, applied in the order they were raised, each a commit on your branch: {}. Build on these, don't remake them: a gap one of them closes is closed. Make your patch with git diff {base} (main plus them; a diff from main would carry them again), and name the cards it stands on to the gate: gp_run model gate with after [{}]. The gate lands it after them, or refuses it if one of them is refused. A dry run is on main without them, so it may say a patch that needs them doesn't apply: raise the card anyway.",
            on.iter().map(|p| name(p)).collect::<Vec<_>>().join("; "),
            ids.join(", ")
        ));
    }
    for p in off {
        b.push(format!(
            "Left out of your worktree: {}, since {}.",
            name(p),
            p.skipped.as_deref().unwrap_or("it didn't apply")
        ));
    }
    b.join("\n")
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
        format!("Run gp commands that write, land or publish with the MCP tool gp_run only: from your shell they are refused. Reads may run in Bash as {gp} COMMAND ARGS, one command a call: no eval, no &&, no pipes. Before cargo in the worktree, run {gp} worker env loop-{id} --rust and set what it prints."),
        format!("Every gp_run call that writes, lands or publishes is this step: gp_run cites {id} for you and refuses any other step."),
        "A change lands only through a gate: gp model gate for a game, gp tool gate for the tool. A landing waits for the person's yes on their page; ask and wait, never work around it.".to_owned(),
        "The step is the whole item: do what it needs now, as far as a card for the person's yes. If what is left is a landing, make the patch, dry-run it and raise the card in this step; never leave it to another step. Don't answer the person with an explanation in place of the work: when they say they don't understand, do the next thing and tell them in one line what is on their page.".to_owned(),
        "When the step is done, or you can't do it, stop with a short report: what you did, what you checked, and what is left.".to_owned(),
        "Your last answer is a structured result (its schema is given to you): outcome (done, on a card, asking you, left or blocked), did, left, and target_met, measures and next_rows where you have them. The runner reads it, not your prose, to make the row's note for the next worker; it closes no row (a row closes by its measure or your --status done).".to_owned(),
    ];
    if let Some((game, row)) = row_of(step) {
        b.push(String::new());
        b.extend([
            if step["repo"] == json!("tools") {
                format!("The step is quest-log item {row} of {game}, and its work is in gameprinter itself: your worktree is of the gameprinter checkout. Land the change with gp tool gate --patch FILE -m MESSAGE. Check it once: build gp, run only the tests you added or changed (cargo test -p CRATE NAME), then git diff > a patch file and gp_run tool gate with patch FILE, message and dry_run. Don't run the whole test suite, just check or clippy yourself: the dry run runs them all, and its FAIL line names what failed (for fmt, the files to format). Its pass is kept, so the gate after the person's YES lands that same tree without checking it again while main hasn't moved.")
            } else {
                format!("The step is quest-log item {row} of {game}. Your worktree is of the games checkout: make the change there. A row about {game}'s own look (no theme named) is the port's fidelity work, not a theme: the model draws the original's pictures from the person's own ROM as it plays (gp needs their ROM file to run it), and the games repo holds no ROM bytes and no art copied from it, so making the model's screen match the original frame for frame is what an emulator does. A theme is new art of its own under {game}/themes/NAME, and only a row that names one is theme work.")
            },
            format!("First: {gp} mechanic show {game} {row} (the item, its notes and talk; before the row has a session it prints the row's note). Your first gp_run mechanic busy or mechanic log makes the row's session; mechanic start serves a page and isn't a worker's."),
            format!("At every step say where you are on the person's page: {gp} mechanic busy {game} {row} \"WHAT YOU ARE DOING. Next: WHAT COMES NEXT\"; put what you found with {gp} mechanic log {game} {row} \"...\"."),
            format!("When you ask the person to judge something they can see, give the row the clip that shows it: gp_run mechanic queue with game {game}, mechanic {row}, status \"asking you\", the question as note, clip N (or ROW/N, or a corpus clip's id), frames A-B and look THEME (the theme it is about, or game). The row and its card then have LOOK AT IT, which sets it up on their screens, the original beside the model, and a look in the game's own pictures (look game) has CONFIRM: their CONFIRM, or a plain yes typed, is locked by gp serve's runner with no worker. When they say yes to such a look in other words, lock it: gp_run mechanic approve with game {game} and mechanic {row} (the clip must be in the corpus), and the model gate then fails a change that draws it further from the original."),
            format!("Land a change with git diff > a patch file in your worktree, then gp_run model gate with game {game}, patch FILE, message \"{game}: WHAT CHANGES. Expect: WHAT THE PERSON SEES. Checked: HOW\", clip N (or ROW/N, or a corpus clip's id), frames A-B and look THEME (or game) for the clip that shows the change, and dry_run first; then the same without dry_run puts a card with LOOK AT IT on the person's page and the loop runs it on their YES. Don't wait for it: say in your report that the card is up, and stop. Never run a gate from your shell, and never land onto your own branch."),
            format!("A row with a target (the TARGET line of mechanic show, a measure on the scoreboard) closes itself when gp serve's scoreboard measures it there; you don't set it done, and --status done is refused before. Without one, give it one as you start: gp_run mechanic queue with game {game}, mechanic {row}, target \"LINE sprites at least N of M\" (a line of {game}/scoreboard.toml), or measure_needed \"WHAT WOULD MEASURE IT\" when no line can."),
            format!("When an item with no target is done: {gp} mechanic queue {game} {row} --status done --note \"WHAT LANDED\". A row with a target whose change is on a card: --status \"in line\"; the YES lands it and the measure closes it. A row that can't go on until another row lands: --status \"in line\" --waits-on ROW (GAME/ROW for another game's), never a note alone; the runner moves it to next with the landing named once that row lands or is done. Only a look asks the person: --status \"asking you\" with a clip and frames for them to watch (above), or a card for their YES. The person has handed you every other choice (the tools, the routing, how to do the work): pick, say why in the note, and go on; a question in your report doesn't reach them."),
            format!("A problem you find that isn't this item's becomes a row of its own, with no card: gp_run mechanic propose with game {game}, mechanic NEW-ROW, what \"WHAT IS WRONG\", plain \"ONE SENTENCE A PERSON CAN READ\", from \"WHERE YOU SAW IT\" and its test in the same call (goal \"RAM TEST\", do, stop), which goes in triage.toml so the triage tries it first. Don't fix it in this step."),
            format!("The person's page shows each row's plain sentence on its card in place of your note: when you set a row's status, give it plain \"ONE SENTENCE A PERSON CAN READ\" too (gp_run mechanic queue with game {game}, mechanic {row} and plain: what it is about or where it stands, with no addresses, row names or ids)."),
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
    // Every worker the same, whoever's machine it runs on: the model and
    // effort loop.toml names (the agent command's own, when it gives them),
    // and none of the person's own settings, hooks, plugins or CLAUDE.md
    // (user settings are left out; auto-memory is off by its variable, set
    // where the worker starts). --bare would also do it but reads only
    // ANTHROPIC_API_KEY, which a person signed in by subscription hasn't.
    let r = loop_state::settings(&setup.home).runner;
    let own = |f: &str| setup.argv.iter().any(|x| x == f);
    if !r.model.is_empty() && !own("--model") {
        a.extend(["--model".to_owned(), r.model]);
    }
    if !r.effort.is_empty() && !own("--effort") {
        a.extend(["--effort".to_owned(), r.effort]);
    }
    if !own("--setting-sources") && !own("--bare") {
        a.extend(["--setting-sources".to_owned(), "project,local".to_owned()]);
    }
    // It ends with a structured result, which makes its row's note.
    a.push("--json-schema".into());
    a.push(WORKER_RESULT_SCHEMA.into());
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
    // The log's tail only: a worker's log grows to megabytes, and this is
    // read for every live worker on every look.
    use std::io::{Read as _, Seek as _, SeekFrom};
    const TAIL: u64 = 64 * 1024;
    let mut f = fs::File::open(log).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut b = Vec::new();
    f.read_to_end(&mut b).ok()?;
    let t = String::from_utf8_lossy(&b);
    let tail = t.lines().rev().find(|l| !l.trim().is_empty())?;
    Some(crate::next::cut(tail, 160))
}

/// Runs the worker's tries until one ends well, it is stopped, or the
/// tries or the time are up. With `resume`, the try its record names is
/// already running (or ended) on its own: it is watched, not started.
fn work(setup: &Setup, live: &Live, rec: &mut Value, resume: bool) -> anyhow::Result<()> {
    let step = rec["step"].clone();
    let id = step["id"].as_str().unwrap_or("").to_owned();
    // A worker on a game's port starts on the gp its port run started on,
    // not on one a tool landing published since (crate::port_run).
    let pinned;
    let setup = match port_gp(setup, rec) {
        Some(gp) => {
            pinned = Setup {
                gp,
                ..setup.clone()
            };
            &pinned
        }
        None => setup,
    };
    let in_games = in_games(&step);
    let (wt, branch, fresh) = worktree(setup, &id, in_games)?;
    rec["worktree"] = json!(wt);
    rec["branch"] = json!(branch);
    let dir = files_dir(&setup.home, &id);
    fs::create_dir_all(&dir)?;
    // A tools worker's build folder starts as a clone of the tool gate's
    // warm build, so its first cargo build compiles only what it changes
    // (crate::warm); where it can't, it starts empty and says why.
    if fresh && !in_games && wt.join("crates").join("gp-cli").is_dir() {
        rec["warm"] = warm_start(setup, &wt, &branch);
        write(&setup.home, rec);
    }
    // A game row's worker starts from main plus the game's pending
    // patches, so it builds on the fixes waiting on a card or in the
    // gate's queue instead of making them again; a row whose target they
    // already meet waits for them, with no worker.
    if fresh
        && in_games
        && let Some((game, row)) = row_of(&step)
    {
        match stack_pending(&setup.home, &wt, &dir, &game, gp_mcp::asks::now_ms()) {
            Ok((pending, base)) => {
                rec["pending"] = json!(pending);
                rec["base"] = json!(base);
                write(&setup.home, rec);
                if let Some(why) = closed_by_pending(setup, &wt, &dir, &game, &row, &pending) {
                    rec["state"] = json!("done");
                    rec["why"] = json!(why);
                    return Ok(());
                }
            }
            Err(e) => rec["pending_error"] = json!(format!("{e:#}")),
        }
    }
    let pending: Vec<Pending> = serde_json::from_value(rec["pending"].clone()).unwrap_or_default();
    let base = rec["base"].as_str().unwrap_or("main").to_owned();
    let row_game = row_of(&step).map(|(g, _)| g).unwrap_or_default();
    let on_pending = pending_words(&row_game, &pending, &base);
    // A games step's gp writes (corpus import's clips, a probe's saves) go
    // to its own worktree, never into the person's checkout, which a
    // landing may be using; they reach it only through a gate, and the
    // gates still run on the person's checkout, the one they land on.
    let games = if in_games { &wt } else { &setup.games };
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
    // Its time runs from its start, across a move onto a new gp.
    let started_ms = rec["started_ms"]
        .as_u64()
        .unwrap_or_else(gp_mcp::asks::now_ms);
    let timed_out =
        || gp_mcp::asks::now_ms().saturating_sub(started_ms) >= setup.timeout.as_millis() as u64;
    let flag = &live.stop;
    let mut before = String::new();
    let first = if resume {
        rec["tries"].as_u64().unwrap_or(1).max(1) as u32
    } else {
        1
    };
    for n in first..=setup.tries.max(first) {
        let log_path = dir.join(format!("{n}.log"));
        let adopted = resume && n == first;
        let p = if adopted {
            Launched::of(rec).context("the worker's record names no process")?
        } else {
            rec["tries"] = json!(n);
            let brief_path = dir.join(format!("brief.{n}.txt"));
            let before = [on_pending.as_str(), before.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            fs::write(&brief_path, brief(setup, &step, &wt, &branch, &before))?;
            let log = File::create(&log_path)?;
            let mut argv: Vec<String> = setup.argv[1..].to_vec();
            let claude =
                clip_agent::is_claude(setup.argv.first().map(String::as_str).unwrap_or(""));
            if claude {
                argv.extend(claude_args(setup, &wt, &mcp));
            }
            let mut shown = vec![setup.program.display().to_string()];
            shown.extend(argv.iter().cloned());
            rec["command"] = json!(clip_agent::command_line(&shown));
            rec["log"] = json!(log_path);
            rec["brief"] = json!(brief_path);
            let mut cmd = launcher(&setup.program, &argv, &brief_path, &dir, n);
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
                .stderr(Stdio::from(log));
            if claude {
                // The person's own memory stays out of the worker.
                cmd.env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");
            }
            gp_mcp::proc::own_group(&mut cmd);
            let p = launch(cmd, &brief_path, &dir, n).context("starting the worker")?;
            rec["pid"] = json!(p.pid);
            rec["group"] = json!(p.group);
            rec["exit_file"] = json!(p.exit);
            rec["state"] = json!("working");
            write(&setup.home, rec);
            p
        };
        // From here its process runs on its own until it ends.
        set_out(live, true);
        let end = |p: &Launched| {
            gp_mcp::proc::stop_group(p.group, Duration::from_secs(2));
            p.reap();
        };
        let mut looked_alive = Instant::now();
        let mut looked_at_state = Instant::now() - LOOK_AT_STATE;
        let code = loop {
            let ended = p.ended(looked_alive.elapsed() >= Duration::from_secs(2));
            if looked_alive.elapsed() >= Duration::from_secs(2) {
                looked_alive = Instant::now();
            }
            if let Some(code) = ended {
                set_out(live, false);
                p.reap();
                // Stopped from outside this server (gp loop, another
                // server): its record says so, and a stop isn't a failed try
                // to start again.
                if code != Some(0) && stopped_on_file(&setup.home, &id) {
                    stopped_by_person(rec);
                    rec["last"] = json!(last_line(&log_path));
                    return Ok(());
                }
                break code;
            }
            if flag.load(Ordering::Relaxed) || stopped_on_file(&setup.home, &id) {
                set_out(live, false);
                end(&p);
                stopped_by_person(rec);
                rec["last"] = json!(last_line(&log_path));
                return Ok(());
            }
            if timed_out() {
                set_out(live, false);
                end(&p);
                rec["state"] = json!("stopped");
                rec["why"] = json!(format!(
                    "its time ran out ({} minutes)",
                    setup.timeout.as_secs() / 60
                ));
                return Ok(());
            }
            // Every card, every run record and the log's tail: read every
            // few seconds, not every 250 ms (with six workers that was a
            // scan of every card file 24 times a second).
            if looked_at_state.elapsed() < LOOK_AT_STATE {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            looked_at_state = Instant::now();
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
        // Ended by a signal: someone stopped it (a kill of its group, from
        // a shell or another tool). That is a stop, not a failed try: its
        // row is parked and no second worker starts on it.
        if code.is_none() {
            stopped_by_person(rec);
            return Ok(());
        }
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
            if flag.load(Ordering::Relaxed) || stopped_on_file(&setup.home, &id) {
                stopped_by_person(rec);
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}

/// The gp a worker on a game's port runs (crate::port_run), written in its
/// record: the one its record already names (a later try, or a worker
/// taken up after a move onto a new gp), else its port run's. None for a
/// worker on no game's port (a tools row runs on the newest gp).
fn port_gp(setup: &Setup, rec: &mut Value) -> Option<PathBuf> {
    let game = crate::port_run::game_of(&rec["step"])?;
    if let Some(gp) = rec["gp"]
        .as_str()
        .map(PathBuf::from)
        .filter(|g| g.is_file())
    {
        return Some(gp);
    }
    let gp = crate::port_run::gp_for(&setup.home, &game, &setup.gp, gp_mcp::asks::now_ms());
    rec["gp"] = json!(gp);
    rec["port_run"] = json!(game);
    Some(gp)
}

/// The gp a yes runs on: the one the worker that raised its card ran, for a
/// card of a game's port run ([`port_gp`]), so the call runs on the gp that
/// made it; None for any other (the tool's own commands run on the newest).
///
/// Now always None: a YES runs on the newest gp, whatever gp the worker
/// that raised its card ran (the owner, 2026-11-22: 'why is the worker gate
/// on the old one lol, fix that'). A card raised that morning was landed
/// by that morning's gate, without the day's fixes to the gate.
fn yes_gp(home: &Path, a: &Value) -> Option<PathBuf> {
    let _ = (home, a);
    None
}

/// Stops worker `id`: its flag when this server runs it, else its process
/// group when its record names one still running.
pub(crate) fn stop_worker(home: &Path, id: &str) -> Result<Value, String> {
    if !id_ok(id) {
        return Err(format!("a step's id is s and 12 hex digits (got {id:?})"));
    }
    if let Some(f) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
        f.stop.store(true, Ordering::Relaxed);
        return Ok(json!({"stopping": id}));
    }
    let mut w: Value = fs::read(file(home, id))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| format!("no worker on step {id}"))?;
    // Its process group: the launcher's, which outlives the launcher.
    let group = w["pid"]
        .as_u64()
        .filter(|p| gp_mcp::proc::alive(*p))
        .map(|pid| w["group"].as_u64().unwrap_or(pid));
    // The stop goes on its record before its process ends, so whichever
    // server watches it reads a stop, not a failed try to start again; and
    // its row is parked first, so no look finds the step open with its
    // worker ended.
    if loop_state::LIVE.contains(&w["state"].as_str().unwrap_or("")) {
        stopped_by_person(&mut w);
        w["pid"] = Value::Null;
        park_stopped_row(home, &w);
        write(home, &w);
    }
    if let Some(group) = group {
        gp_mcp::proc::stop_group(group as u32, Duration::from_secs(2));
    }
    Ok(json!({"stopped": id}))
}

/// Marks a worker's record stopped by the person.
fn stopped_by_person(rec: &mut Value) {
    rec["state"] = json!("stopped");
    rec["why"] = json!("you stopped it");
    rec["stopped_by"] = json!("person");
    rec["stopped_ms"] = json!(gp_mcp::asks::now_ms());
}

/// Whether worker `id`'s record on file says the person stopped it
/// ([`stop_worker`] from outside the server watching it).
fn stopped_on_file(home: &Path, id: &str) -> bool {
    let w: Value = fs::read(file(home, id))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    w["state"] == json!("stopped") && w["stopped_by"] == json!("person")
}

/// The row of a worker the person stopped: parked in line, its note kept
/// after why, so no worker takes it up (crate::steps gives a parked row no
/// step for words written before the park) until it is set next again,
/// which isn't a hand fix (crate::mechanic::is_parked), or the person
/// writes on it. A done row stays done.
fn park_stopped_row(home: &Path, rec: &Value) {
    let Some((game, row)) = row_of(&rec["step"]) else {
        return;
    };
    if !crate::mechanic::session::name_ok(&game) {
        return;
    }
    let f = home.join("mechanics").join(&game).join("queue.json");
    let mut q = crate::mechanic::session::read_queue(&f);
    let Some(r) = q
        .iter_mut()
        .find(|r| r["mechanic"] == json!(row) && r["status"] != json!("done"))
    else {
        return;
    };
    let id = rec["step"]["id"].as_str().unwrap_or("");
    let why = format!("you stopped its worker (loop-{id}); set it next to take it up again");
    if r["parked"] == json!(why.as_str()) && r["status"] == json!("in line") {
        return;
    }
    crate::mechanic::park_row(r, &why);
    let _ = crate::mechanic::session::write_queue(&f, &q);
}

/// A worker's process as started by [`launch`]. On unix it runs on its
/// own: a short `sh` (the process group's leader, which this server waits
/// for at once) starts it in the background with the brief on its stdin
/// and writes its pid, and when it ends its exit code goes to a file. It
/// is no child of gp serve, so it keeps running across gp serve's move
/// onto a new build (exec), and any server reads its end from the file.
pub(crate) struct Launched {
    /// The process that runs the agent and writes its exit.
    pub pid: u32,
    /// Its process group, which [`gp_mcp::proc::stop_group`] stops.
    pub group: u32,
    /// Where its exit code is written when it ends (none off unix).
    pub exit: Option<PathBuf>,
}

/// The children this server waits for itself (off unix).
#[cfg(not(unix))]
static CHILDREN: Mutex<BTreeMap<u32, std::process::Child>> = Mutex::new(BTreeMap::new());

/// The launcher's script: the agent in the background with the brief on
/// its stdin, its exit code written when it ends, its pid written now.
#[cfg(unix)]
const LAUNCH_SH: &str = r#"e=$1; p=$2; b=$3; shift 3
( "$@" < "$b"; c=$?; echo "$c" > "$e.tmp"; mv "$e.tmp" "$e" ) &
echo "$!" > "$p.tmp"; mv "$p.tmp" "$p"
"#;

fn exit_file(dir: &Path, n: u32) -> PathBuf {
    dir.join(format!("{n}.exit"))
}

fn pid_file(dir: &Path, n: u32) -> PathBuf {
    dir.join(format!("{n}.pid"))
}

/// The command that starts try `n` of `program ARGV` with the brief.
fn launcher(program: &Path, argv: &[String], brief: &Path, dir: &Path, n: u32) -> Command {
    #[cfg(unix)]
    {
        let mut c = Command::new("/bin/sh");
        c.arg("-c")
            .arg(LAUNCH_SH)
            .arg("sh")
            .arg(exit_file(dir, n))
            .arg(pid_file(dir, n))
            .arg(brief)
            .arg(program)
            .args(argv)
            .stdin(Stdio::null());
        c
    }
    #[cfg(not(unix))]
    {
        let _ = (brief, dir, n);
        let mut c = Command::new(program);
        c.args(argv).stdin(Stdio::piped());
        c
    }
}

/// Starts the [`launcher`]'s command.
fn launch(mut cmd: Command, brief: &Path, dir: &Path, n: u32) -> anyhow::Result<Launched> {
    #[cfg(unix)]
    {
        let _ = brief;
        let (exit, pidf) = (exit_file(dir, n), pid_file(dir, n));
        let _ = fs::remove_file(&exit);
        let _ = fs::remove_file(&pidf);
        let mut sh = cmd.spawn()?;
        let group = sh.id();
        let s = sh.wait()?;
        let pid = fs::read_to_string(&pidf)
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
            .with_context(|| format!("the launcher wrote no pid (it ended with {s})"))?;
        Ok(Launched {
            pid,
            group,
            exit: Some(exit),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, n);
        let mut child = cmd.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&fs::read(brief).unwrap_or_default());
        }
        let pid = child.id();
        CHILDREN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(pid, child);
        Ok(Launched {
            pid,
            group: pid,
            exit: None,
        })
    }
}

impl Launched {
    /// The process a worker's record names, when it can be taken up: one
    /// started detached (its exit file named), still running or ended with
    /// its exit written. It asks the process table first and the file
    /// after: the process writes its exit before it ends, so one found
    /// ended has its file there, while the file looked for first could be
    /// written, and the process end, between the two looks, and a worker
    /// ending just then would read as neither (it couldn't start: the
    /// worker's record names no process).
    pub(crate) fn of(rec: &Value) -> Option<Self> {
        let pid = rec["pid"].as_u64()? as u32;
        let exit = PathBuf::from(rec["exit_file"].as_str()?);
        let group = rec["group"].as_u64().map_or(pid, |g| g as u32);
        (gp_mcp::proc::alive(u64::from(pid)) || exit.is_file()).then_some(Self {
            pid,
            group,
            exit: Some(exit),
        })
    }

    fn written(&self) -> Option<Option<i32>> {
        let t = fs::read_to_string(self.exit.as_ref()?).ok()?;
        Some(t.trim().parse::<i32>().ok())
    }

    /// Its exit code once it ended (None inside for a signal or an end
    /// with nothing written), else None. `look` also asks the process
    /// table, for an end that wrote nothing (its group stopped).
    fn ended(&self, look: bool) -> Option<Option<i32>> {
        #[cfg(not(unix))]
        if self.exit.is_none() {
            let _ = look;
            let mut m = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
            let c = m.get_mut(&self.pid)?;
            return match c.try_wait() {
                Ok(Some(s)) => Some(s.code()),
                Ok(None) => None,
                Err(_) => Some(None),
            };
        }
        if let Some(c) = self.written() {
            return Some(c);
        }
        if look && !gp_mcp::proc::alive(u64::from(self.pid)) {
            return Some(self.written().flatten());
        }
        None
    }

    /// Off unix, waits for the child it ended; on unix nothing waits (the
    /// launcher was waited for, and its background child isn't ours).
    fn reap(&self) {
        #[cfg(not(unix))]
        if let Some(mut c) = CHILDREN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.pid)
        {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
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

    /// A yes to a corpus import that writes runs as the model gate's
    /// import, with the same clips, sessions and line, never in the games
    /// checkout; one that only plans runs as asked.
    #[test]
    fn a_yes_to_a_corpus_import_that_writes_runs_through_the_model_gate() {
        let games = Path::new("/games");
        let ask = |argv: Value| json!({"command": "corpus import", "argv": argv});
        let call = import_as_gate(
            &ask(json!([
                "--write",
                "--says",
                "the arcade buttons",
                "--replace",
                "g.a.001",
                "--remove=g.b.*",
                "--session",
                "a",
                "g"
            ])),
            games,
        );
        assert_eq!(
            call,
            Some(Ok([
                "model",
                "gate",
                "/games/g",
                "--import",
                "--says",
                "the arcade buttons",
                "--replaces",
                "g.a.001",
                "--removes",
                "g.b.*",
                "--session",
                "a"
            ]
            .map(str::to_owned)
            .to_vec()))
        );
        let t = import_as_gate(&ask(json!(["--template", "/x/g", "--write", "g"])), games);
        assert_eq!(
            t,
            Some(Ok(["model", "gate", "/x/g", "--import"]
                .map(str::to_owned)
                .to_vec()))
        );
        assert_eq!(
            import_as_gate(&ask(json!(["--replace", "g.a.001", "g"])), games),
            None
        );
        let from = import_as_gate(&ask(json!(["--write", "--from", "/h", "g"])), games);
        assert!(matches!(from, Some(Err(w)) if w.contains("--from")));
        let gate = json!({"command": "model gate", "argv": ["g", "--write"]});
        assert_eq!(import_as_gate(&gate, games), None);
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
    fn one_worker_at_a_time_changes_a_games_model() {
        // A game's rows hold the game too; tool rows hold only their row.
        let mut tool = s("s000000000003", "agent", &["row:a/t"]);
        tool["repo"] = json!("tools");
        let open = [
            s("s000000000001", "agent", &["row:a/r", "game:a"]),
            s("s000000000002", "agent", &["row:a/q", "game:a"]),
            tool,
            s("s000000000004", "agent", &["row:b/r", "game:b"]),
        ];
        let got = plan(&open, &[], &BTreeSet::new(), 5, 0);
        let ids: Vec<&str> = got.iter().filter_map(|x| x["id"].as_str()).collect();
        assert_eq!(ids, ["s000000000001", "s000000000003", "s000000000004"]);
        // While the first row's worker is live, the second waits for it.
        let workers = [json!({"step": open[0].clone(), "state": "working"})];
        let live: BTreeSet<String> = ["s000000000001".to_owned()].into();
        let got = plan(&open[1..], &workers, &live, 5, 0);
        assert!(got.iter().all(|x| x["id"] != json!("s000000000002")));
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
            None,
        )
        .unwrap();
        let a = row("a");
        assert_eq!(a["status"], "next");
        assert!(
            a["note"].as_str().unwrap().starts_with(
                "The last worker asked: Should I model the initials screen? Or leave it? It isn't the person's"
            ),
            "{a}"
        );
        // A report ending in the tool-log question leaves its row at next.
        let tq = home.join("mechanics/g/queue.json");
        let mut q = crate::mechanic::session::read_queue(&tq);
        q.push(json!({"mechanic": "t", "status": "agent working"}));
        crate::mechanic::session::write_queue(&tq, &q).unwrap();
        settle_row(
            home,
            "g",
            "t",
            "Did the patch.\nShould this row move to the tool's quest log?",
            None,
        )
        .unwrap();
        assert_eq!(row("t")["status"], "next");
        assert_eq!(a["asked"], "go");
        assert!(a.get("worker").is_none());
        settle_row(
            home,
            "g",
            "b",
            "Did the fix (see https://x/?a=1).\n\nLeft: the card for the gate.\nThen done.",
            None,
        )
        .unwrap();
        let b = row("b");
        assert_eq!(b["status"], "next");
        assert_eq!(b["note"], "Left: the card for the gate. Then done.");
        settle_row(home, "g", "c", "Anything left?", None).unwrap();
        assert_eq!(row("c")["status"], "done");
        // A row its worker left at next stays at next whatever its note or
        // the report says: wording closes nothing.
        let mut q = crate::mechanic::session::read_queue(&f);
        q.push(json!({"mechanic": "d", "status": "next", "note": "Nothing new to land: ad344acc has it."}));
        q.push(json!({"mechanic": "e", "status": "next", "note": "Landed 1f2e."}));
        q.push(json!({"mechanic": "g", "status": "next", "note": "Gone from the tip."}));
        q.push(json!({"mechanic": "i", "status": "next", "note": "Landed it.", "target": "level 1 sprites at least 1"}));
        crate::mechanic::session::write_queue(&f, &q).unwrap();
        for m in ["d", "e", "g", "i"] {
            settle_row(home, "g", m, "Done. Landed.", None).unwrap();
            assert_eq!(row(m)["status"], "next", "{m}");
        }
        // With no line naming what is left, its last paragraph.
        assert_eq!(left_of("Did it.\n\nThe card is up."), "The card is up.");
    }

    #[test]
    fn a_structured_result_makes_its_rows_note_and_not_the_prose() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let f = home.join("mechanics/g/queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let q = json!([
            {"mechanic": "a", "status": "agent working", "worker": "loop-x"},
            {"mechanic": "b", "status": "agent working", "worker": "loop-y"},
        ]);
        fs::write(&f, q.to_string()).unwrap();
        let row = |m: &str| -> Value {
            crate::mechanic::session::read_queue(&f)
                .into_iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
        };
        // Prose with a question and a left line: the result wins.
        let left = json!({"outcome": "left", "did": "Made the clip.", "left": "the gate card.",
            "next_rows": ["tile-flicker"]});
        settle_row(
            home,
            "g",
            "a",
            "Should I land it?\nLeft: nothing",
            Some(&left),
        )
        .unwrap();
        assert_eq!(row("a")["status"], "next");
        assert_eq!(
            row("a")["note"],
            "Left: the gate card. Proposed: tile-flicker."
        );
        // An outcome of done closes nothing: the row closes by its measure
        // or its worker's --status done.
        let done = json!({"outcome": "done", "did": "Landed 1f2e.", "left": ""});
        settle_row(home, "g", "b", "Left: lots?", Some(&done)).unwrap();
        assert_eq!(row("b")["status"], "next");
        assert_eq!(row("b")["note"], "Landed 1f2e.");
        let blocked = json!({"outcome": "blocked", "did": "", "left": "no ROM."});
        assert_eq!(note_of(&blocked), "Blocked: no ROM.");
        // The schema the worker is given parses and names its fields.
        let s: Value = serde_json::from_str(WORKER_RESULT_SCHEMA).unwrap();
        assert_eq!(s["required"], json!(["outcome", "did", "left"]));
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
                // Set by hand after its card failed; set
                // aside after its NO (double-drink-end).
                {"mechanic": "e", "status": "done", "note": "Landed by hand after the first card failed",
                 "changed_ms": gp_mcp::asks::now_ms() - 100},
                {"mechanic": "f", "status": "in line", "note": "Its card was set aside",
                 "changed_ms": gp_mcp::asks::now_ms() - 100},
                {"mechanic": "h", "status": "asking you", "note": "Its card is on your page: press YES"},
                {"mechanic": "i", "status": "asking you", "note": "Its card is on your page: press YES"},
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
        // A NO and a YES with the person's words beside them
        // (card-answer-takes-words).
        card(
            7,
            "h",
            json!({"state": "no", "answered_ms": now, "words": "the foam is too tall"}),
        );
        card(
            8,
            "i",
            json!({"state": "used", "words": "keep the rim", "ran": {"state": "landed", "done_ms": now}}),
        );
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
        assert_eq!(row("b")["status"], "next");
        assert_eq!(
            row("b")["note"],
            "You said NO to its card (the glass froths). The loop takes it up again; say on the row what should change."
        );
        assert_eq!(row("h")["status"], "next");
        assert_eq!(
            row("h")["note"],
            "You said NO to its card (the glass froths) and wrote: \"the foam is too tall\". The loop takes it up again and makes the fix you asked for."
        );
        assert_eq!(row("i")["status"], "done");
        assert_eq!(
            row("i")["note"],
            "Landed on your YES: the glass froths. Beside your YES you wrote: \"keep the rim\"."
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

    /// The owner's case (2026-11-23, "i don't understand either card open
    /// right now"): a trouble row's fix card landed while its worker still
    /// ran, the worker then set the row asking you about the card, and the
    /// page asked "What should happen with it?". A row asking about a card
    /// that is gone follows it, however late it was set: landed, it closes
    /// with no ask; refused, it goes to next. One whose card is still on the
    /// page waits; one with no card at all goes to next after ten minutes.
    #[test]
    fn a_row_asking_about_a_card_that_is_gone_follows_it() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let f = home.join("mechanics/made-up/queue.json");
        let now = gp_mcp::asks::now_ms();
        let ask = |m: &str| json!({"mechanic": m, "status": "asking you", "note": "On your page: a card to land a fix to the clip import."});
        let mut never = ask("never");
        never["changed_ms"] = json!(now - CARD_ASK_WITHOUT_CARD_MS - 1);
        crate::mechanic::session::write_queue(
            &f,
            &[
                ask("trouble-landed"),
                ask("trouble-no"),
                ask("trouble-open"),
                ask("just-asked"),
                never.clone(),
                json!({"mechanic": "looks", "status": "asking you", "note": "Does the card table look right?", "clip": {"n": 1}}),
            ],
        )
        .unwrap();
        // write_queue stamps the rows now; the one with no card stood long.
        let mut q = crate::mechanic::session::read_queue(&f);
        q[4] = never;
        fs::write(&f, serde_json::to_string(&q).unwrap()).unwrap();
        let landed = json!({"state": "used", "row_settled": true,
            "ran": {"state": "landed", "done_ms": now - 60_000, "lines": ["PASS: landed a697061b on main"]}});
        test_card(home, 1, now - 120_000, "made-up", "trouble-landed", landed);
        test_card(
            home,
            2,
            now - 120_000,
            "made-up",
            "trouble-no",
            json!({"state": "no", "answered_ms": now - 60_000, "row_settled": true}),
        );
        test_card(
            home,
            3,
            now - 1000,
            "made-up",
            "trouble-open",
            json!({"state": "open"}),
        );
        assert_eq!(
            card_stands(&gp_mcp::asks::read(home, "ask-3").unwrap(), now),
            "in play"
        );
        // Settled before the row asked (it changed later): left as it was.
        settle_card_rows(home);
        let row = |m: &str| -> Value {
            crate::mechanic::session::read_queue(&f)
                .into_iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
        };
        assert_eq!(row("trouble-landed")["status"], "asking you");
        let moved = settle_card_asks(home, now);
        assert_eq!(
            moved,
            [
                ("made-up".to_owned(), "trouble-landed".to_owned()),
                ("made-up".to_owned(), "trouble-no".to_owned()),
                ("made-up".to_owned(), "never".to_owned()),
            ]
        );
        let l = row("trouble-landed");
        assert_eq!(l["status"], "done");
        assert_eq!(
            l["note"],
            "Landed on your YES: The import keeps the arcade buttons."
        );
        assert_eq!(row("trouble-no")["status"], "next");
        assert_eq!(row("trouble-open")["status"], "asking you");
        assert_eq!(row("just-asked")["status"], "asking you");
        assert_eq!(row("never")["status"], "next");
        assert!(
            row("never")["note"]
                .as_str()
                .unwrap()
                .contains("takes it up again")
        );
        // A row asking about a look it has a clip for stays the person's.
        assert_eq!(row("looks")["status"], "asking you");
        // Nothing left to move.
        assert!(settle_card_asks(home, now).is_empty());
    }

    /// The owner's case (2026-11-23): a YES meant "this is what I want for
    /// this round" and began a 30-minute landing. LOOKS GOOD, KEEP GOING on a
    /// gate's card lands nothing: the row is parked in line marked
    /// iterating, so its worker's gates are refused (gp_mcp::iterating),
    /// even when its worker was still on it as the person answered.
    #[test]
    fn looks_good_keep_going_parks_the_row_iterating_and_lands_nothing() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let f = home.join("mechanics/g/queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(
            &f,
            json!([
                {"mechanic": "tip-money", "status": "in line", "note": "Its card waits for your YES", "target": "level 1 sprites at least 1 of 2"},
                {"mechanic": "clear-mugs", "status": "agent working", "note": "Its card waits for your YES"},
            ])
            .to_string(),
        )
        .unwrap();
        let now = gp_mcp::asks::now_ms();
        let kept =
            json!({"state": "no", "answered_ms": now, "keep_going": true, "words": "nice tips"});
        test_card(home, 1, now - 1000, "g", "tip-money", kept);
        test_card(
            home,
            2,
            now - 999,
            "g",
            "clear-mugs",
            json!({"state": "no", "answered_ms": now, "keep_going": true}),
        );
        settle_card_rows(home);
        let row = |m: &str| -> Value {
            crate::mechanic::session::read_queue(&f)
                .into_iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
        };
        let r = row("tip-money");
        assert_eq!(r["status"], "in line");
        assert_eq!(r["iterating"], KEPT_GOING);
        assert!(crate::mechanic::is_parked(&r));
        assert!(r.get("awaiting_measure").is_none());
        let note = r["note"].as_str().unwrap();
        assert!(
            note.contains("nothing landed") && note.contains("LAND IT"),
            "{note}"
        );
        assert!(note.contains("You wrote: \"nice tips\""), "{note}");
        // Its worker still on it: marked at once, parked once it stops.
        let w = row("clear-mugs");
        assert_eq!(w["status"], "agent working");
        assert_eq!(w["iterating"], KEPT_GOING);
        let mut q = crate::mechanic::session::read_queue(&f);
        q[1]["status"] = json!("next");
        crate::mechanic::session::write_queue(&f, &q).unwrap();
        settle_card_rows(home);
        let w = row("clear-mugs");
        assert_eq!(w["status"], "in line");
        assert!(crate::mechanic::is_parked(&w));
    }

    /// The owner's case (2026-11-22): refusing a duplicate card sent its row
    /// to next, and a worker made the same card again, three times. A NO
    /// naming the card it duplicates parks the row on that card's row with
    /// both cards named; it wakes when that card ends; a NO naming another
    /// card of its own row parks it with no wait; a plain NO is as before.
    #[test]
    fn a_no_as_a_duplicate_parks_the_row_on_the_other_cards_row() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let f = home.join("mechanics/g/queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(
            &f,
            json!([
                {"mechanic": "dup", "status": "in line", "note": "Its card waits for your YES"},
                {"mechanic": "fix", "status": "in line", "note": "Its card waits for your YES"},
                {"mechanic": "twice", "status": "in line", "note": "Its card waits for your YES"},
                {"mechanic": "plain", "status": "in line", "note": "Its card waits for your YES"},
            ])
            .to_string(),
        )
        .unwrap();
        let now = gp_mcp::asks::now_ms();
        let asks = gp_mcp::asks::dir(home);
        fs::create_dir_all(&asks).unwrap();
        fs::create_dir_all(loop_state::workers_dir(home)).unwrap();
        let card = |n: u64, row: &str, msg: &str, extra: Value| {
            let step = format!("s{n:012x}");
            fs::write(
                file(home, &step),
                json!({"step": {"id": step, "touches": [format!("row:g/{row}")]}}).to_string(),
            )
            .unwrap();
            let mut a = json!({"id": format!("ask-{n}"), "made_ms": now - 1000 + n, "step": step, "game": "g",
                "command": "model gate", "argv": ["g", "--message", msg]});
            for (k, v) in extra.as_object().unwrap() {
                a[k] = v.clone();
            }
            fs::write(asks.join(format!("ask-{n}.json")), a.to_string()).unwrap();
        };
        let fix = "g: the bubbles go a frame late. Expect: x";
        card(1, "fix", fix, json!({"state": "open"}));
        card(
            2,
            "dup",
            "g: the bubbles a frame later. Expect: x",
            json!({"state": "no", "answered_ms": now, "duplicate_of": "ask-1", "words": "same as the other"}),
        );
        card(3, "twice", "g: one.", json!({"state": "open"}));
        card(
            4,
            "twice",
            "g: one again. Expect: x",
            json!({"state": "no", "answered_ms": now, "duplicate_of": "ask-3"}),
        );
        let no = json!({"state": "no", "answered_ms": now});
        card(5, "plain", "g: foam.", no);
        settle_card_rows(home);
        let row = |m: &str| -> Value {
            crate::mechanic::session::read_queue(&f)
                .into_iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
        };
        let d = row("dup");
        assert_eq!(d["status"], "in line");
        assert_eq!(d["waits_on"], "g/fix");
        assert!(crate::mechanic::is_parked(&d));
        assert_eq!(
            d["note"],
            "You said NO to its card (the bubbles a frame later): it duplicates the card for g/fix (the bubbles go a frame late). It waits on g/fix, and the loop takes it up again once that card is used, refused or gone. You wrote: \"same as the other\"."
        );
        let tw = row("twice");
        assert_eq!(tw["status"], "in line");
        assert!(tw.get("waits_on").is_none());
        assert!(crate::mechanic::is_parked(&tw));
        let twn = tw["note"].as_str().unwrap();
        assert!(twn.contains("duplicates the card ask-3 (one)"), "{tw}");
        assert_eq!(row("plain")["status"], "next");
        // While fix's card is open, dup waits; once it lands, dup wakes.
        assert!(wake_waiting(home, None, now).is_empty());
        card(
            1,
            "fix",
            fix,
            json!({"state": "used", "row_settled": true,
                "ran": {"state": "landed", "done_ms": now + 60_000, "lines": ["PASS: landed 0a1b2c3d on main"]}}),
        );
        assert_eq!(
            wake_waiting(home, None, now + 60_000),
            vec![("g".to_owned(), "dup".to_owned())]
        );
        assert_eq!(row("dup")["status"], "next");
    }

    /// The runner archives a row done for an hour with the commit its
    /// landed card names; a row with a live worker, one whose card isn't
    /// settled yet and one done lately stay on the page.
    #[test]
    fn a_finished_row_goes_to_the_archive_with_its_landed_commit() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let now = gp_mcp::asks::now_ms();
        let old = now - 2 * 60 * 60 * 1000;
        let f = home.join("mechanics/g/queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(
            &f,
            json!([
                {"mechanic": "a", "status": "done", "note": "Landed on your YES: the glass froths.", "changed_ms": old},
                {"mechanic": "b", "status": "done", "changed_ms": old},
                {"mechanic": "c", "status": "done", "changed_ms": old},
                {"mechanic": "d", "status": "done", "changed_ms": now - 1000},
                {"mechanic": "e", "status": "next", "changed_ms": old},
            ])
            .to_string(),
        )
        .unwrap();
        let asks = gp_mcp::asks::dir(home);
        fs::create_dir_all(&asks).unwrap();
        fs::create_dir_all(loop_state::workers_dir(home)).unwrap();
        let step = |n: u64, row: &str, extra: Value| {
            let id = format!("s{n:012x}");
            let mut w =
                json!({"step": {"id": id, "touches": [format!("row:g/{row}")]}, "state": "done"});
            for (k, v) in extra.as_object().unwrap() {
                w[k] = v.clone();
            }
            fs::write(file(home, &id), w.to_string()).unwrap();
            id
        };
        let card = |n: u64, row: &str, extra: Value| {
            let id = step(n, row, json!({}));
            let mut a = json!({"id": format!("ask-{n}"), "made_ms": now - 3 * 60 * 60 * 1000 + n, "step": id, "game": "g",
                "command": "tool gate", "argv": ["--message", "the glass froths. Expect: froth"]});
            for (k, v) in extra.as_object().unwrap() {
                a[k] = v.clone();
            }
            fs::write(asks.join(format!("ask-{n}.json")), a.to_string()).unwrap();
        };
        card(
            1,
            "a",
            json!({"state": "used", "row_settled": true,
                "ran": {"state": "landed", "done_ms": old, "lines": ["PASS: landed 0123abcd on main"]}}),
        );
        card(2, "c", json!({"state": "yes"}));
        step(
            3,
            "b",
            json!({"state": "working", "pid": std::process::id()}),
        );
        archive_finished(home, now);
        let left: Vec<String> = crate::mechanic::session::read_queue(&f)
            .iter()
            .filter_map(|r| r["mechanic"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(left, vec!["b", "c", "d", "e"]);
        let a = crate::mechanic::archive::read_archive(home, "g");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0]["mechanic"], "a");
        assert_eq!(a[0]["landed"]["commit"], "0123abcd");
        assert_eq!(a[0]["landed"]["card"], "ask-1");
        assert_eq!(a[0]["landed"]["what"], "the glass froths");
        assert_eq!(a[0]["closed_ms"], json!(old));
    }

    /// The owner's case (waits-wake-on-landing): a row parked in line on a
    /// row whose card then landed goes to next with the landing's commit
    /// and what landed, while a row with its own card up stays parked.
    #[test]
    fn a_row_waiting_on_a_landed_row_goes_to_next() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let now = gp_mcp::asks::now_ms();
        let f = home.join("mechanics/g/queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(
            &f,
            json!([
                {"mechanic": "plugin", "status": "in line", "target": "x own at least 1"},
                {"mechanic": "site", "status": "in line", "note": "the plugin first", "waits_on": "g/plugin", "waits_since": now - 60_000},
                {"mechanic": "mine", "status": "in line", "note": "Waits for plugin to land; card up"},
            ])
            .to_string(),
        )
        .unwrap();
        let asks = gp_mcp::asks::dir(home);
        fs::create_dir_all(&asks).unwrap();
        fs::create_dir_all(loop_state::workers_dir(home)).unwrap();
        let card = |n: u64, row: &str, a: Value| {
            let id = format!("s{n:012x}");
            let w =
                json!({"step": {"id": id, "touches": [format!("row:g/{row}")]}, "state": "done"});
            fs::write(file(home, &id), w.to_string()).unwrap();
            let mut base = json!({"id": format!("ask-{n}"), "made_ms": now - 30_000, "step": id, "game": "g",
                "command": "tool gate", "argv": ["--message", "the plugin. Expect: it installs"]});
            for (k, v) in a.as_object().unwrap() {
                base[k] = v.clone();
            }
            fs::write(asks.join(format!("ask-{n}.json")), base.to_string()).unwrap();
        };
        card(
            1,
            "plugin",
            json!({"state": "used", "row_settled": true,
                "ran": {"state": "landed", "done_ms": now - 10_000, "lines": ["PASS: landed f28c0973 on main"]}}),
        );
        card(2, "mine", json!({"state": "open"}));
        assert_eq!(
            wake_waiting(home, None, now),
            vec![("g".to_owned(), "site".to_owned())]
        );
        let q = crate::mechanic::session::read_queue(&f);
        let row = |n: &str| {
            q.iter()
                .find(|r| r["mechanic"] == json!(n))
                .unwrap()
                .clone()
        };
        let site = row("site");
        assert_eq!(site["status"], "next");
        let note = site["note"].as_str().unwrap();
        assert!(
            note.contains("plugin landed (f28c0973): the plugin"),
            "{note}"
        );
        assert!(site.get("waits_on").is_none());
        assert_eq!(row("mine")["status"], "in line");
        assert_eq!(row("plugin")["status"], "in line");
    }

    /// A clip the gate held out waits on its session's row with why: a
    /// done row goes back to next with it as its note, a row asking the
    /// person keeps its note and gets the line in `open` (once a clip), a
    /// session with no row gets one; the runner's status says it.
    #[test]
    fn a_held_clip_waits_on_its_row_with_why() {
        assert_eq!(
            row_of_clip("rooftop.drink-far-end.001"),
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
    fn a_confirmed_look_locks_with_no_worker_or_waits_for_its_clip() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("mechanics").join("g").join("queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let clip = json!({"id": "g.l1.001", "from": 10, "to": 12, "look": "game"});
        let q = vec![
            json!({"mechanic": "l1", "status": "asking you", "note": "Does level 1 look right?", "clip": clip}),
            json!({"mechanic": "demo", "status": "asking you", "note": "And the demo?", "clip": {"n": 1, "look": "game"}}),
            json!({"mechanic": "theme", "status": "asking you", "note": "The mug?", "clip": {"n": 1, "look": "last-call"}}),
        ];
        crate::mechanic::session::write_queue(&f, &q).unwrap();
        let row = |m: &str| {
            crate::mechanic::session::read_queue(&f)
                .into_iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
        };
        // The yes: in line, confirmed, the question kept as asked.
        let confirm = |row: &str, said: &str, typed: bool| {
            crate::approved::confirm(&f, "g", &[], row, said, typed)
        };
        confirm("l1", "CONFIRM", false).unwrap();
        // A typed yes: its turn has set the row agent working first.
        let mut q = crate::mechanic::session::read_queue(&f);
        q[1]["status"] = json!("agent working");
        crate::mechanic::session::write_queue(&f, &q).unwrap();
        assert!(confirm("demo", "yes", false).is_err());
        confirm("demo", "yes", true).unwrap();
        assert!(confirm("theme", "yes", true).is_err());
        let l1 = row("l1");
        assert_eq!(l1["status"], "in line");
        assert_eq!(l1["confirmed"]["said"], "CONFIRM");
        assert_eq!(l1["asked"], "Does level 1 look right?");
        // Locked: done with the frames it locked, its clip and yes gone.
        let yes = Locked::Yes {
            clip: "g.l1.001".into(),
            frames: json!([10, 12]),
            n: 3,
            landed: true,
        };
        settle_lock(&f, "l1", &yes, 1000).unwrap();
        let l1 = row("l1");
        assert_eq!(l1["status"], "done");
        assert_eq!(l1["locked"]["frames"], json!([10, 12]));
        assert!(l1.get("confirmed").is_none() && l1.get("clip").is_none());
        assert!(
            l1["note"]
                .as_str()
                .unwrap()
                .starts_with("Saved as right: l1 (g.l1.001 frames 10 to 12).")
        );
        // A clip not in the corpus yet: tried again after a while.
        let wait = Locked::Wait("clip 1 of demo isn't in the corpus".into());
        settle_lock(&f, "demo", &wait, 1000).unwrap();
        let demo = row("demo");
        assert_eq!(demo["status"], "in line");
        assert_eq!(demo["confirmed"]["again_ms"], json!(1000 + LOCK_AGAIN_MS));
        assert_eq!(demo["confirmed"]["tries"], 1);
        // Any other failure: next, with why, for the loop.
        settle_lock(&f, "demo", &Locked::No("the gate failed".into()), 2000).unwrap();
        let demo = row("demo");
        assert_eq!(demo["status"], "next");
        assert!(demo.get("confirmed").is_none());
        assert!(demo["note"].as_str().unwrap().contains("the gate failed"));
        // A row whose yes is gone is left as it is.
        settle_lock(&f, "theme", &yes, 3000).unwrap();
        assert_eq!(row("theme")["status"], "asking you");
    }

    #[test]
    fn a_row_whose_look_a_card_saved_closes_and_a_second_yes_does_nothing() {
        let t = tempfile::tempdir().unwrap();
        let (home, games) = (t.path().join("home"), t.path().join("games"));
        let f = home.join("mechanics").join("g").join("queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let dir = games.join("g");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("roms.toml"), "").unwrap();
        fs::write(dir.join(gp_model::MODEL_FILE), "# a model\n").unwrap();
        let clip =
            |from: u64, to: u64| json!({"id": "g.l4.001", "from": from, "to": to, "look": "game"});
        let q = vec![
            // Confirmed, its look then saved by a worker's model gate card.
            json!({"mechanic": "confirm-level-4", "status": "in line", "clip": clip(0, 99),
                "confirmed": {"at_ms": 1, "said": "CONFIRM"}, "note": "You said yes to this look"}),
            // Asking again about a look saved already.
            json!({"mechanic": "again", "status": "asking you", "clip": clip(10, 20)}),
            // Asking about frames the saved look doesn't hold.
            json!({"mechanic": "later", "status": "asking you", "clip": clip(50, 150)}),
            // A worker at it is left to them.
            json!({"mechanic": "busy", "status": "agent working", "clip": clip(0, 99)}),
            // With a target: in line for its measure.
            json!({"mechanic": "aimed", "status": "in line", "clip": clip(0, 99),
                "target": "level 4 sprites at least 99 of 100"}),
        ];
        crate::mechanic::session::write_queue(&f, &q).unwrap();
        let look = |row: &str, to: u64| crate::approved::Look {
            row: row.into(),
            clip: "g.l4.001".into(),
            frames: [0, to],
            look: "game".into(),
            approved: "2026-11-21".into(),
            apart: (0..=to).map(|i| [i, 0, 0]).collect(),
        };
        let looks = ["confirm-level-4", "again", "later", "busy", "aimed"].map(|r| look(r, 99));
        fs::write(dir.join("approved.toml"), crate::approved::text(&looks)).unwrap();
        let row = |m: &str| {
            crate::mechanic::session::read_queue(&f)
                .into_iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
        };
        // The second yes, before the runner looks: nothing locks.
        let r = crate::approved::confirm(&f, "g", &looks, "again", "yes", false).unwrap();
        assert_eq!(r["status"], "done");
        assert!(r.get("confirmed").is_none() && r.get("clip").is_none());
        let note = r["note"].as_str().unwrap();
        assert!(note.starts_with("Already saved as right"), "{note}");
        let closed = settle_saved_rows(&home, &games, 5000);
        assert_eq!(closed, ["g/confirm-level-4", "g/aimed"]);
        let c = row("confirm-level-4");
        assert_eq!(c["status"], "done");
        assert!(c.get("confirmed").is_none() && c.get("clip").is_none());
        assert_eq!(c["locked"]["frames"], json!([0, 99]));
        assert_eq!(c["locked"]["saved"], "2026-11-21");
        let note = c["note"].as_str().unwrap();
        assert!(
            note.starts_with("Saved as right: g.l4.001 frames 0-99 (100 frames, saved 2026-11-21)"),
            "{note}"
        );
        assert_eq!(row("later")["status"], "asking you");
        assert_eq!(row("busy")["status"], "agent working");
        let aimed = row("aimed");
        assert_eq!(aimed["status"], "in line");
        assert_eq!(aimed["awaiting_measure"], 5000);
        // Settled once: the next look closes nothing.
        assert!(settle_saved_rows(&home, &games, 6000).is_empty());
        // The runner's lock arriving after leaves the closed row as it is.
        let yes = Locked::Yes {
            clip: "g.l4.001".into(),
            frames: json!([0, 99]),
            n: 100,
            landed: false,
        };
        let was = row("confirm-level-4");
        settle_lock(&f, "confirm-level-4", &yes, 7000).unwrap();
        assert_eq!(row("confirm-level-4"), was);
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

    /// The flag's value in a worker's arguments.
    fn flag(a: &[String], f: &str) -> Option<String> {
        a.iter()
            .position(|x| x == f)
            .and_then(|i| a.get(i + 1).cloned())
    }

    #[test]
    fn a_worker_runs_on_the_loops_model_not_the_persons_setup() {
        let t = tempfile::tempdir().unwrap();
        let mut setup = rig(t.path(), "true\n");
        let (wt, mcp) = (t.path().join("wt"), t.path().join("mcp.json"));
        // No loop.toml: the pinned defaults, and no user settings.
        let a = claude_args(&setup, &wt, &mcp);
        assert_eq!(
            flag(&a, "--model").as_deref(),
            Some(loop_state::WORKER_MODEL)
        );
        assert_eq!(
            flag(&a, "--effort").as_deref(),
            Some(loop_state::WORKER_EFFORT)
        );
        assert_eq!(
            flag(&a, "--setting-sources").as_deref(),
            Some("project,local")
        );
        // It ends with the structured result its row's note is made from.
        assert_eq!(
            flag(&a, "--json-schema").as_deref(),
            Some(WORKER_RESULT_SCHEMA)
        );
        // loop.toml's choice, and empty leaves it to the person's own.
        let mut s = loop_state::settings(&setup.home);
        s.runner.model = "sonnet".into();
        s.runner.effort = String::new();
        loop_state::save(&setup.home, &s).unwrap();
        let a = claude_args(&setup, &wt, &mcp);
        assert_eq!(flag(&a, "--model").as_deref(), Some("sonnet"));
        assert_eq!(flag(&a, "--effort"), None);
        // An agent command that gives its own model keeps it, given once.
        setup.argv = vec!["claude".into(), "--model".into(), "haiku".into()];
        let a = claude_args(&setup, &wt, &mcp);
        assert_eq!(flag(&a, "--model"), None);
    }

    #[test]
    fn a_worker_in_a_port_run_starts_on_the_newest_gp_after_a_tool_landing() {
        let t = tempfile::tempdir().unwrap();
        let out = t.path().join("seen");
        let mut setup = rig(
            t.path(),
            &format!("echo \"$GP_STEP $GP_GP\" >> {}\n", out.display()),
        );
        let bin = setup.home.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let (old, new) = (bin.join("gp-aaaaaaaaaaaa"), bin.join("gp-bbbbbbbbbbbb"));
        fs::write(&old, "").unwrap();
        fs::write(&new, "").unwrap();
        let (old, new) = (
            fs::canonicalize(old).unwrap(),
            fs::canonicalize(new).unwrap(),
        );
        let row = |id: &str, row: &str, repo: &str| {
            let mut st = s(id, "agent", &[&format!("row:g/{row}"), "game:g"]);
            st["repo"] = json!(repo);
            st
        };
        setup.gp = old.clone();
        spawn(setup.clone(), row("s0000000000c1", "a", "games"))
            .join()
            .unwrap();
        // A tool landing published another gp, and gp serve moved onto it.
        setup.gp = new.clone();
        spawn(setup.clone(), row("s0000000000c2", "b", "games"))
            .join()
            .unwrap();
        // A tools row changes gp itself: it runs on the newest.
        let mut tool = s("s0000000000c3", "agent", &["row:g/c"]);
        tool["repo"] = json!("tools");
        spawn(setup.clone(), tool).join().unwrap();
        let seen = fs::read_to_string(&out).unwrap();
        assert!(
            seen.contains(&format!("s0000000000c1 {}\n", old.display())),
            "{seen}"
        );
        assert!(
            seen.contains(&format!("s0000000000c2 {}\n", new.display())),
            "{seen}"
        );
        assert!(
            seen.contains(&format!("s0000000000c3 {}\n", new.display())),
            "{seen}"
        );
        let mcp: Value = serde_json::from_slice(
            &fs::read(files_dir(&setup.home, "s0000000000c2").join("mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(mcp["mcpServers"]["gp"]["command"], json!(new));
        // The YES on a card one of them raised runs on the gp that made it;
        // a tool gate's on the newest.
        let card = |command: &str| json!({"step": "s0000000000c2", "command": command});
        assert_eq!(yes_gp(&setup.home, &card("model gate")), None);
        assert_eq!(yes_gp(&setup.home, &card("tool gate")), None);
        // Moved on, the game's next worker starts on the newest.
        assert!(crate::port_run::end(&setup.home, "g"));
        spawn(setup.clone(), row("s0000000000c4", "d", "games"))
            .join()
            .unwrap();
        let seen = fs::read_to_string(&out).unwrap();
        assert!(
            seen.contains(&format!("s0000000000c4 {}\n", new.display())),
            "{seen}"
        );
    }

    /// A worker reaches the loop through the MCP tools a person's agent
    /// has: every gp command its brief names is one gp has, with its
    /// flags, and every tool it names is among the worker's allowed ones.
    #[test]
    fn a_workers_brief_names_commands_gp_has_and_tools_it_may_call() {
        let t = tempfile::tempdir().unwrap();
        let mut setup = rig(t.path(), "true\n");
        // As a worker's brief says it on a home where gp is on the path.
        setup.gp = PathBuf::from("gp");
        let (wt, mcp) = (t.path().join("wt"), t.path().join("mcp.json"));
        let allowed = claude_args(&setup, &wt, &mcp);
        let tools = crate::mcp_keeps_up::tool_names();
        for repo in ["tools", "games"] {
            let cli = format!(
                "{}{}",
                crate::steps::ROW_CLI
                    .replace("GAME", "g")
                    .replace("ROW", "r"),
                crate::steps::TOOL_ROW_CLI
            );
            let step = json!({
                "id": "s0000000000b1", "subject": "g", "says": "Work on r.",
                "how": {"cli": cli}, "commands": crate::steps::commands_of(&cli),
                "touches": ["row:g/r"], "repo": repo,
            });
            let b = brief(&setup, &step, &wt, "loop/s0000000000b1", "");
            assert_eq!(
                b.contains("run only the tests you added or changed"),
                repo == "tools",
                "a tools row's worker checks once and leaves the rest to the dry run:\n{b}"
            );
            assert_eq!(
                crate::mcp_keeps_up::unknown_commands(&b),
                Vec::<String>::new(),
                "the brief (serve/runner.rs brief) names gp commands gp hasn't:\n{b}"
            );
            for n in b
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|w| w.contains('_') && tools.contains(w))
            {
                assert!(
                    allowed.contains(&format!("mcp__gp__{n}")),
                    "the brief names {n}, which the worker isn't allowed (claude_args)"
                );
            }
        }
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
        assert_eq!(r["stopped_by"], "person");
    }

    /// The owner's case (2026-11-22): a worker stopped from outside the
    /// server watching it was started again at once as try 2, and once its
    /// row's note changed, as a new step. A stop isn't a failed try: the
    /// worker ends stopped after try 1, its row is parked in line with its
    /// note kept, and its step waits for the row to go next again, then
    /// starts at once.
    #[test]
    fn a_stopped_worker_parks_its_row_and_is_not_tried_again() {
        let t = tempfile::tempdir().unwrap();
        let setup = rig(t.path(), "echo working\nsleep 30\n");
        let f = setup.home.join("mechanics/g/queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(
            &f,
            json!([
                {"mechanic": "r", "status": "agent working", "note": "the foam is too tall"},
            ])
            .to_string(),
        )
        .unwrap();
        let id = "s0000000000a9";
        let mut step = s(id, "agent", &["row:g/r"]);
        step["repo"] = json!("tools");
        let h = spawn(setup.clone(), step.clone());
        let t0 = Instant::now();
        while record(&setup.home, id)["last"] != json!("working")
            && t0.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        // As gp loop's stop does it from another process: the record marked
        // and the process group ended, with this server unaware.
        let mut w = record(&setup.home, id);
        let group = w["group"].as_u64().or(w["pid"].as_u64()).unwrap();
        stopped_by_person(&mut w);
        write(&setup.home, &w);
        gp_mcp::proc::stop_group(group as u32, Duration::from_secs(2));
        h.join().unwrap();
        let r = record(&setup.home, id);
        assert_eq!(r["state"], "stopped");
        assert_eq!(r["tries"], 1, "a stop isn't a failed try");
        assert!(!files_dir(&setup.home, id).join("brief.2.txt").exists());
        let row = crate::mechanic::session::read_queue(&f)[0].clone();
        assert_eq!(row["status"], "in line");
        assert!(crate::mechanic::is_parked(&row), "{row}");
        let note = row["note"].as_str().unwrap();
        assert!(
            note.starts_with("Parked: you stopped its worker (loop-s0000000000a9)"),
            "{note}"
        );
        assert!(note.ends_with("the foam is too tall"), "{note}");
        assert!(row.get("worker").is_none());
        // Set next again (its step the same), it starts at once, though its
        // worker ended moments ago; a worker that ended on its own waits.
        let now = gp_mcp::asks::now_ms();
        let workers = vec![record(&setup.home, id)];
        let none = BTreeSet::new();
        let open = std::slice::from_ref(&step);
        assert_eq!(plan(open, &workers, &none, 3, now).len(), 1);
        let mut ended = workers[0].clone();
        ended["stopped_by"] = Value::Null;
        ended["last_ms"] = json!(now);
        assert!(plan(open, &[ended], &none, 3, now).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_running_worker_is_not_in_the_servers_hands_and_a_new_server_takes_it_up() {
        let t = tempfile::tempdir().unwrap();
        let go = t.path().join("go");
        let setup = rig(
            t.path(),
            &format!(
                "while [ ! -e {g} ]; do sleep 0.05; done\necho fin\n",
                g = go.display()
            ),
        );
        let id = "s0000000000a4";
        let h = spawn(setup.clone(), s(id, "agent", &["game:g"]));
        let out = || {
            RUNNING
                .lock()
                .unwrap()
                .get(id)
                .is_some_and(|l| l.out.load(Ordering::SeqCst))
        };
        let t0 = Instant::now();
        while !out() && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        // Its process runs on its own: a move onto a new gp needn't wait.
        assert!(out(), "running on its own");
        let r = record(&setup.home, id);
        assert!(loop_state::holds(&r), "{r}");
        assert!(r["exit_file"].is_string() && r["group"].is_u64(), "{r}");
        fs::write(&go, "").unwrap();
        h.join().unwrap();
        assert_eq!(record(&setup.home, id)["state"], "done");

        // A worker the server before left running: the new one watches it
        // to its end.
        let id = "s0000000000a5";
        let dir = files_dir(&setup.home, id);
        fs::create_dir_all(&dir).unwrap();
        let brief = dir.join("brief.1.txt");
        fs::write(&brief, "the brief\n").unwrap();
        let log = File::create(dir.join("1.log")).unwrap();
        let _ = fs::remove_file(&go);
        let script = t.path().join("left.sh");
        fs::write(
            &script,
            format!(
                "while [ ! -e {g} ]; do sleep 0.05; done\ncat\necho left running\n",
                g = go.display()
            ),
        )
        .unwrap();
        let mut cmd = launcher(
            Path::new("/bin/sh"),
            &[script.display().to_string()],
            &brief,
            &dir,
            1,
        );
        cmd.stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log));
        gp_mcp::proc::own_group(&mut cmd);
        let p = launch(cmd, &brief, &dir, 1).unwrap();
        let rec = json!({"step": s(id, "agent", &["game:g"]), "state": "working", "tries": 1, "runs": 1,
            "started_ms": gp_mcp::asks::now_ms(), "pid": p.pid, "group": p.group, "exit_file": p.exit});
        write(&setup.home, &rec);
        assert!(Launched::of(&rec).is_some());
        let h = adopt(setup.clone(), rec);
        fs::write(&go, "").unwrap();
        h.join().unwrap();
        let r = record(&setup.home, id);
        assert_eq!(r["state"], "done", "{r}");
        assert_eq!(r["tries"], 1);
        assert_eq!(r["last"], "left running");
        let said = fs::read_to_string(dir.join("1.log")).unwrap();
        assert!(said.contains("the brief"), "its brief on stdin: {said}");

        // One that ended with an error while no server watched: its next
        // try starts here.
        let id = "s0000000000a6";
        let dir = files_dir(&setup.home, id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("1.log"), "nope\n").unwrap();
        fs::write(exit_file(&dir, 1), "3\n").unwrap();
        let rec = json!({"step": s(id, "agent", &["game:g"]), "state": "working", "tries": 1, "runs": 1,
            "started_ms": gp_mcp::asks::now_ms(), "pid": 999_999_999u64, "exit_file": exit_file(&dir, 1)});
        assert!(Launched::of(&rec).is_some());
        adopt(setup.clone(), rec).join().unwrap();
        let r = record(&setup.home, id);
        assert_eq!(r["state"], "done", "{r}");
        assert_eq!(r["tries"], 2);
        let b2 = fs::read_to_string(dir.join("brief.2.txt")).unwrap();
        assert!(
            b2.contains("Your try 1 at this step ended with exit 3"),
            "{b2}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_worker_ending_while_it_is_looked_at_can_still_be_taken_up() {
        // Looked at again and again while it ends, its record names a
        // process every time: running, or ended with its exit written.
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("w");
        fs::create_dir_all(&dir).unwrap();
        let brief = dir.join("brief.txt");
        fs::write(&brief, "").unwrap();
        let script = t.path().join("short.sh");
        fs::write(&script, "sleep 0.05\n").unwrap();
        for n in 1..=8 {
            let mut cmd = launcher(
                Path::new("/bin/sh"),
                &[script.display().to_string()],
                &brief,
                &dir,
                n,
            );
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
            gp_mcp::proc::own_group(&mut cmd);
            let p = launch(cmd, &brief, &dir, n).unwrap();
            let rec = json!({"pid": p.pid, "group": p.group, "exit_file": p.exit});
            let t0 = Instant::now();
            loop {
                assert!(Launched::of(&rec).is_some(), "try {n}: {rec}");
                if !gp_mcp::proc::alive(u64::from(p.pid)) {
                    break;
                }
                assert!(t0.elapsed() < Duration::from_secs(10), "try {n} ran on");
            }
        }
    }

    /// Game g's scoreboard lines as row_steps reads them: level 1 on
    /// clip g.l1.001, level 2 on g.l2.001.
    fn g_lines() -> Vec<(String, String)> {
        vec![
            ("level 1".into(), "g.l1.001".into()),
            ("level 2".into(), "g.l2.001".into()),
        ]
    }

    /// A row of game g's quest log (`r`, its mechanic `row`) as a step,
    /// its claim made as row_steps makes it.
    fn row_step_of(id: &str, row: &str, mut r: Value) -> Value {
        r["mechanic"] = json!(row);
        let mut touches = vec![format!("row:g/{row}")];
        touches.extend(crate::steps::claim_of("g", &r, &g_lines()));
        let t: Vec<&str> = touches.iter().map(String::as_str).collect();
        s(id, "agent", &t)
    }

    /// A row of game g's quest log with target `target`, or with a note
    /// that names model.toml, so it claims the whole game.
    fn row_step(id: &str, row: &str, target: Option<&str>) -> Value {
        match target {
            Some(t) => row_step_of(id, row, json!({"target": t})),
            None => row_step_of(id, row, json!({"note": "the bar's speed in model.toml"})),
        }
    }

    /// What each kind of row claims besides itself: the narrowest thing
    /// its work needs.
    #[test]
    fn a_row_claims_its_line_the_game_only_for_the_games_files_else_itself() {
        let claim = |r: Value| crate::steps::claim_of("g", &r, &g_lines());
        let line = |l: &str| vec![format!("line:g/{l}")];
        let table: [(Value, Vec<String>); 9] = [
            // A target names its line, whatever else the row says.
            (
                json!({"target": "Level 2 sprites at least 10", "note": "model.toml"}),
                line("level 2"),
            ),
            // Filed with a corpus clip a line measures: that line.
            (
                json!({"clip": {"id": "g.l1.001", "from": 3, "to": 9}}),
                line("level 1"),
            ),
            // Found on a line (gp picture --propose --line) with no target.
            (json!({"line": "Level 2"}), line("level 2")),
            // A clip no line measures: that clip.
            (
                json!({"clip": {"id": "g.other.001"}}),
                vec!["clip:g/g.other.001".into()],
            ),
            // The game's own files: the whole game.
            (
                json!({"note": "the timer in model.toml is a frame short"}),
                vec!["game:g".into()],
            ),
            (
                json!({"from": "mapping.toml:12 sprite.slot"}),
                vec!["game:g".into()],
            ),
            (
                json!({"note": "a hook in ext/ draws it"}),
                vec!["game:g".into()],
            ),
            // The owner's words with no clip, target or file: only the row.
            (
                json!({"note": "the fourth bar's background is wrong"}),
                vec![],
            ),
            (json!({}), vec![]),
        ];
        for (r, want) in table {
            assert_eq!(claim(r.clone()), want, "{r}");
        }
        // What clashes with what: the same claim, and the game with its
        // lines; rows and other games' claims don't.
        let taken =
            |t: &[&str]| -> BTreeSet<String> { t.iter().map(|x| (*x).to_owned()).collect() };
        let clashes = [
            (&["game:g"][..], "line:g/level 1", true),
            (&["line:g/level 1"][..], "game:g", true),
            (&["line:g/level 1"][..], "line:g/level 1", true),
            (&["line:g/level 1"][..], "line:g/level 2", false),
            (&["game:h"][..], "line:g/level 1", false),
            (&["game:g"][..], "row:g/x", false),
            (&["game:g"][..], "clip:g/g.other.001", true),
            (&["clip:g/g.other.001"][..], "game:g", true),
            (&["clip:g/g.other.001"][..], "clip:g/g.other.001", true),
            (&["clip:g/g.other.001"][..], "line:g/level 1", false),
            (&["row:g/x"][..], "row:g/x", true),
            (&["row:g/x"][..], "row:g/y", false),
        ];
        for (t, c, want) in clashes {
            assert_eq!(crate::steps::clashes(&taken(t), c), want, "{t:?} {c}");
        }
    }

    /// Rows with no target and no file of the game's run side by side; a
    /// whole-game row waits while the narrower rows fill the slots, and the
    /// page says what it waits for.
    #[test]
    fn a_whole_game_row_starts_after_the_rows_that_claim_less() {
        let ids = |v: Vec<Value>| -> Vec<String> {
            v.iter()
                .map(|x| x["id"].as_str().unwrap().to_owned())
                .collect()
        };
        let none = BTreeSet::new();
        let whole = row_step("s000000000021", "w", None);
        let words = row_step_of("s000000000022", "x", json!({"note": "the owner's words"}));
        let words2 = row_step_of("s000000000023", "y", json!({}));
        let l1 = row_step_of("s000000000024", "z", json!({"clip": {"id": "g.l1.001"}}));
        assert_eq!(words["touches"].as_array().unwrap().len(), 1);
        let open = [whole.clone(), words.clone(), words2.clone(), l1.clone()];
        // Three slots: the three rows that claim less, in their order.
        assert_eq!(
            ids(plan(&open, &[], &none, 3, 0)),
            ["s000000000022", "s000000000023", "s000000000024"]
        );
        // A slot to spare: the whole-game row can't run beside the line's.
        assert_eq!(
            ids(plan(&open, &[], &none, 5, 0)),
            ["s000000000022", "s000000000023", "s000000000024"]
        );
        // With no line row it runs beside the rows that claim only
        // themselves, after them in the plan but in its place in the list.
        let open = [whole.clone(), words.clone(), words2.clone()];
        assert_eq!(
            ids(plan(&open, &[], &none, 5, 0)),
            ["s000000000021", "s000000000022", "s000000000023"]
        );
        assert_eq!(
            ids(plan(&open, &[], &none, 2, 0)),
            ["s000000000022", "s000000000023"]
        );
        // The page's words for a row that waits for a live one.
        let held = [whole.clone()];
        assert_eq!(
            crate::steps::waits_for(&l1, &held).as_deref(),
            Some("Waits for w, which holds the game.")
        );
        assert_eq!(
            crate::steps::waits_for(&whole, std::slice::from_ref(&l1)).as_deref(),
            Some("Waits for z, which holds the line level 1.")
        );
        assert_eq!(crate::steps::waits_for(&words, &held), None);
        assert_eq!(crate::steps::waits_for(&whole, &held), None);
    }

    /// Two picture rows of one cause, one with a target on level 1 and seen
    /// in level 2's clip too, the other with a target on level 2, went to
    /// two workers who each brought the same fix (docs/loop.md). A row claims the line of every clip it was seen in, so the second
    /// waits for the first; a clip no line measures is claimed by its id.
    #[test]
    fn rows_seen_in_one_clip_wait_for_each_other() {
        let ids = |v: Vec<Value>| -> Vec<String> {
            v.iter()
                .map(|x| x["id"].as_str().unwrap().to_owned())
                .collect()
        };
        let e3 = row_step_of(
            "s000000000031",
            "tiles-74-e3",
            json!({"target": "level 1 sprites at least 1350 of 1400",
                   "seen_in": ["g.l1.001", "g.l2.001"]}),
        );
        assert_eq!(
            e3["touches"],
            json!(["row:g/tiles-74-e3", "line:g/level 1", "line:g/level 2"])
        );
        let e4 = row_step_of(
            "s000000000032",
            "tiles-74-e4",
            json!({"target": "level 2 sprites at least 1314 of 1464",
                   "seen_in": ["g.l2.001"]}),
        );
        let other = row_step_of(
            "s000000000033",
            "elsewhere",
            json!({"seen_in": ["g.demo.001"]}),
        );
        assert_eq!(other["touches"][1], json!("clip:g/g.demo.001"));
        let same_clip = row_step_of(
            "s000000000034",
            "elsewhere-too",
            json!({"clip": {"id": "g.demo.001", "from": 3, "to": 9}}),
        );
        let none = BTreeSet::new();
        let open = [e3.clone(), e4.clone(), other.clone(), same_clip.clone()];
        assert_eq!(
            ids(plan(&open, &[], &none, 5, 0)),
            ["s000000000031", "s000000000033"]
        );
        let held = [e3.clone(), other.clone()];
        assert_eq!(
            crate::steps::waits_for(&e4, &held).as_deref(),
            Some("Waits for tiles-74-e3, which holds the line level 2.")
        );
        assert_eq!(
            crate::steps::waits_for(&same_clip, &held).as_deref(),
            Some("Waits for elsewhere, which holds the clip g.demo.001.")
        );
        // A whole-game row waits for a row that holds only a clip.
        let whole = row_step("s000000000035", "w", None);
        assert_eq!(
            ids(plan(&[other.clone(), whole], &[], &none, 5, 0)),
            ["s000000000033"]
        );
    }

    #[test]
    fn a_rows_target_claims_its_line_and_rows_of_other_lines_run_beside_it() {
        let ids = |v: Vec<Value>| -> Vec<String> {
            v.iter()
                .map(|x| x["id"].as_str().unwrap().to_owned())
                .collect()
        };
        let l1 = row_step(
            "s000000000011",
            "a",
            Some("level 1 sprites at least 330 of 336"),
        );
        assert_eq!(l1["touches"][1], json!("line:g/level 1"));
        let l1b = row_step(
            "s000000000012",
            "b",
            Some("Level 1 background at least 300"),
        );
        let l2 = row_step("s000000000013", "c", Some("level 2 sprites at least 10"));
        let whole = row_step("s000000000014", "d", None);
        assert_eq!(whole["touches"][1], json!("game:g"));
        let none = BTreeSet::new();
        // Rows of two lines of one game run side by side; a second row of
        // the same line waits, and so does a row that claims the whole
        // game.
        let open = [l1.clone(), l1b.clone(), l2.clone(), whole.clone()];
        assert_eq!(
            ids(plan(&open, &[], &none, 5, 0)),
            ["s000000000011", "s000000000013"]
        );
        // A whole-game row first in the list still waits for the lines'.
        let open = [whole.clone(), l1.clone(), l2.clone()];
        assert_eq!(
            ids(plan(&open, &[], &none, 5, 0)),
            ["s000000000011", "s000000000013"]
        );
        // A live worker on a line holds that line and the whole game, not
        // the other lines; max_workers still caps them.
        let workers = [json!({"step": l1.clone(), "state": "working"})];
        let live: BTreeSet<String> = ["s000000000011".to_owned()].into();
        let open = [l1b.clone(), whole.clone(), l2.clone()];
        assert_eq!(ids(plan(&open, &workers, &live, 5, 0)), ["s000000000013"]);
        assert!(plan(&open, &workers, &live, 1, 0).is_empty());
        // A row of another game's line is free of game g's claims.
        let other = s("s000000000015", "agent", &["row:h/x", "game:h"]);
        let open = [other];
        assert_eq!(ids(plan(&open, &workers, &live, 5, 0)), ["s000000000015"]);
    }

    /// A model gate card of game `game` in `home`, made `made` ms after a
    /// minute ago (so their order is theirs), for the
    /// step `step` of row `row` (its worker's record written), with its
    /// patch `patch`.
    #[allow(clippy::too_many_arguments)]
    fn card(
        home: &Path,
        id: &str,
        game: &str,
        state: &str,
        made: u64,
        step: &str,
        row: &str,
        patch: &Path,
    ) {
        let now = gp_mcp::asks::now_ms();
        let a = json!({"id": id, "command": "model gate", "game": game, "state": state,
            "made_ms": now - 60_000 + made, "answered_ms": now, "step": step,
            "argv": ["--patch", patch, "--message", format!("{game}: fix {row}. Expect: x"), game]});
        let d = gp_mcp::asks::dir(home);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(format!("{id}.json")), a.to_string()).unwrap();
        write(
            home,
            &json!({"step": {"id": step, "touches": [format!("row:{game}/{row}")]}, "state": "done"}),
        );
    }

    #[test]
    fn a_game_rows_worker_starts_from_main_with_the_games_pending_patches() {
        let t = tempfile::tempdir().unwrap();
        let out = t.path().join("seen");
        let setup = rig(
            t.path(),
            &format!(
                "cat > {o}\necho '--- log' >> {o}\ngit log --format=%s >> {o}\necho '--- files' >> {o}\ncat g.txt h.txt >> {o}\n",
                o = out.display()
            ),
        );
        let p = |name: &str, text: &str| {
            let f = t.path().join(name);
            fs::write(&f, text).unwrap();
            f
        };
        let g1 = p(
            "g1.patch",
            "diff --git a/g.txt b/g.txt\n--- a/g.txt\n+++ b/g.txt\n@@ -1 +1 @@\n-g\n+g1\n",
        );
        let h = p(
            "h.patch",
            "diff --git a/h.txt b/h.txt\nnew file mode 100644\n--- /dev/null\n+++ b/h.txt\n@@ -0,0 +1 @@\n+h\n",
        );
        let bad = p(
            "bad.patch",
            "diff --git a/g.txt b/g.txt\n--- a/g.txt\n+++ b/g.txt\n@@ -1 +1 @@\n-x\n+y\n",
        );
        let home = &setup.home;
        // Raised out of order: the tree takes them oldest first.
        card(
            home,
            "2-bbb",
            "g",
            "yes",
            2000,
            "s0000000000b2",
            "slide",
            &h,
        );
        card(
            home,
            "1-aaa",
            "g",
            "open",
            1000,
            "s0000000000b1",
            "gap",
            &g1,
        );
        card(
            home,
            "3-ccc",
            "g",
            "open",
            3000,
            "s0000000000b3",
            "late",
            &bad,
        );
        // Not pending, or another game's: left out.
        card(
            home,
            "4-ddd",
            "g",
            "no",
            4000,
            "s0000000000b4",
            "nope",
            &bad,
        );
        card(
            home,
            "5-eee",
            "other",
            "open",
            5000,
            "s0000000000b5",
            "x",
            &bad,
        );
        let id = "s0000000000b9";
        let mut st = row_step(id, "r", None);
        st["repo"] = json!("games");
        spawn(setup.clone(), st).join().unwrap();
        let seen = fs::read_to_string(&out).unwrap();
        let (brief, rest) = seen.split_once("--- log\n").unwrap();
        let (log, files) = rest.split_once("--- files\n").unwrap();
        // Each pending patch is a commit on the worker's branch, oldest
        // raised first, above main's; the one that doesn't apply isn't.
        let log: Vec<&str> = log.lines().collect();
        assert_eq!(log.len(), 3, "{log:?}");
        assert!(
            log[0].starts_with("pending card 2-bbb: fix slide"),
            "{log:?}"
        );
        assert!(log[1].starts_with("pending card 1-aaa: fix gap"), "{log:?}");
        assert_eq!(log[2], "first");
        assert_eq!(files, "g1\nh\n");
        // The brief lists them, says to build on them, how to make the
        // patch above them and which cards to name to the gate.
        let r = record(home, id);
        let base = r["base"].as_str().unwrap();
        assert!(brief.contains("card 1-aaa (row gap): fix gap"), "{brief}");
        assert!(
            brief.contains("card 2-bbb (row slide): fix slide"),
            "{brief}"
        );
        assert!(
            brief.contains("Build on these, don't remake them"),
            "{brief}"
        );
        assert!(brief.contains(&format!("git diff {base}")), "{brief}");
        assert!(brief.contains("after [1-aaa, 2-bbb]"), "{brief}");
        assert!(
            brief.contains("Left out of your worktree: card 3-ccc (row late)"),
            "{brief}"
        );
        assert!(
            !brief.contains("4-ddd") && !brief.contains("5-eee"),
            "{brief}"
        );
        assert_eq!(r["state"], "done", "{r}");
        let took: Vec<Pending> = serde_json::from_value(r["pending"].clone()).unwrap();
        assert_eq!(took.len(), 3);
        assert!(
            took[2]
                .skipped
                .as_deref()
                .unwrap()
                .contains("doesn't apply")
        );
        // Its patches are kept beside its brief.
        assert!(files_dir(home, id).join("pending-1-aaa.patch").is_file());
    }

    #[test]
    fn a_yes_to_a_patch_made_on_top_waits_for_the_cards_under_it() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let f = t.path().join("x.patch");
        let now = gp_mcp::asks::now_ms();
        card(
            home,
            "1-base",
            "g",
            "open",
            1000,
            "s0000000000d1",
            "gap",
            &f,
        );
        card(
            home,
            "2-top",
            "g",
            "yes",
            2000,
            "s0000000000d2",
            "slide",
            &f,
        );
        let d = gp_mcp::asks::dir(home).join("2-top.json");
        let mut top: Value = serde_json::from_str(&fs::read_to_string(&d).unwrap()).unwrap();
        top["argv"] = json!(["--patch", f, "--after", "1-base", "g"]);
        // Answered long ago: a yes the runner holds stays alive.
        top["answered_ms"] = json!(now - 2 * gp_mcp::asks::YES_FOR_MS);
        fs::write(&d, top.to_string()).unwrap();
        let read = |id: &str| gp_mcp::asks::read(home, id).unwrap();
        let set = |id: &str, k: &str, v: Value| gp_mcp::asks::mark(home, id, k, v).unwrap();
        // The base waits for the person: the yes is held, kept alive and
        // says why.
        assert_eq!(hold_for_after(home, &read("2-top"), now), Some(Ok(())));
        let held = read("2-top");
        assert_eq!(held["after_held_ms"], json!(now));
        assert!(gp_mcp::asks::is_yes(&held, now + 1000));
        assert!(held["after_waits"].as_str().unwrap().contains("1-base"));
        // The base runs on its yes: still held.
        set("1-base", "state", json!("running"));
        assert_eq!(hold_for_after(home, &read("2-top"), now), Some(Ok(())));
        // The base landed: the yes runs now.
        set("1-base", "state", json!("used"));
        set("1-base", "ran", json!({"state": "landed"}));
        assert_eq!(hold_for_after(home, &read("2-top"), now), None);
        // The base failed, or the person said NO to it: the yes fails, why
        // named, and the gate itself refuses the patch.
        set("1-base", "ran", json!({"state": "failed"}));
        let why = hold_for_after(home, &read("2-top"), now)
            .unwrap()
            .unwrap_err();
        assert!(why.contains("1-base") && why.contains("failed"), "{why}");
        set("1-base", "ran", Value::Null);
        set("1-base", "state", json!("no"));
        let why = hold_for_after(home, &read("2-top"), now)
            .unwrap()
            .unwrap_err();
        assert!(why.contains("said NO"), "{why}");
        match crate::gate::stands_on(home, &["1-base".into()], now) {
            crate::gate::StandsOn::Refused(w) => assert!(w.contains("1-base"), "{w}"),
            o => panic!("{o:?}"),
        }
        // A card gone holds nothing back; a yes with no --after runs.
        assert!(matches!(
            crate::gate::stands_on(home, &["9-gone".into()], now),
            crate::gate::StandsOn::Landed(g) if g.len() == 1
        ));
        assert_eq!(hold_for_after(home, &read("1-base"), now), None);
    }
}
