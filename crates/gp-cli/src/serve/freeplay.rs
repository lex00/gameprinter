//! Free play (docs/show.md, "Free play"): the original played live in the
//! game view with save points and clips. The page runs the player
//! (`gp_play.wasm`) and keeps each save point's state bytes in memory for
//! fast loading; this server keeps what makes every save point
//! reproducible natively, the input from power-on that reaches it.
//!
//! A session is a folder under `$GP_HOME/play/free/<session>/`, outside
//! the store and never in git:
//!
//! | File | What it is |
//! |---|---|
//! | `session.json` | The session ([`Session`]): its capture run, its branches, save points and clips |
//! | `branches/<b>.part` | One branch's input log frame lines from power-on, as the page uploads them |
//! | `saves/<k>.log` | A save point's input log: exactly the frames from power-on to its frame |
//! | `saves/<k>.png` | Its picture, as the page drew it |
//! | `clips/<c>.json` | A clip: a played scenario whose start is the save point (`gp_interp::scenario::StartAt` with `log`) |
//!
//! `session.json` also keeps each clip's place ([`ClipMeta`]): the branch
//! and frames of the session's input it was cut from, whether it cut
//! itself from the play loop or was started by hand, what ended it, and
//! the person's mark on it (kept, skipped or starred).
//!
//! The session's input is a tree of branches. The first starts at power-on;
//! when free play starts at a frame of a recording, the page replays the
//! recording to it and uploads its player's log from power-on. Loading a save point starts a
//! new branch whose first frames are the save point's log. A branch's
//! frames are only ever appended, so the first N frames of the branch a
//! save point was made on are its log for good.
//!
//! Each save point is rebuilt natively once it's made (gp-nes, headless,
//! on its log), and the state hash and frame hash digest reached there are
//! checked against the page's. A rebuild starts from the nearest save point
//! rebuilt before it whose log is the start of this one's, else from
//! power-on, and runs one at a time, so a session of hundreds of save
//! points costs about one pass over its input. The page keeps no state
//! bytes across page loads: `saves/<k>/state` gives a save point's bytes
//! from the rebuild, checked against the hash the page sent, so a GET
//! carries them whatever their size (the 64 KB body limit is for writes).
//!
//! Ported from game-printer's serve/freeplay.rs. A session's run is a game
//! of the games checkout; the rebuilt game's start is translated through
//! the game's mapping and module as the mechanic page translates it
//! ([`crate::mechanic::live::start`]). Free play is the NES's today: on an
//! arcade game it says so.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context as _;
use base64::Engine as _;
use gp_evidence::input_log::InputLog;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::game::{Run, is_name, rom_of, run_named};
use super::{Ctx, Fail, Params, Route, body, post_only};
use crate::mechanic::http::{Request, Response};
use crate::roms::Platform;
use crate::serve::work::now_ms;

/// The session file's version.
const SCHEMA: u32 = 0;

/// NTSC frames a second: 39375000/655171.
const NTSC_FPS: f64 = 39_375_000.0 / 655_171.0;

/// The longest clip a page may save, in frames: 30 seconds. The page's
/// setting (8 seconds by default) is well under it.
pub(super) const MAX_CLIP_FRAMES: u64 = 30 * 60;

pub(super) const SESSIONS: &str = "/api/play/free";
const SESSION: &str = "/api/play/free/{session}";
pub(super) const BRANCHES: &str = "/api/play/free/{session}/branches";
pub(super) const BRANCH_LOG: &str = "/api/play/free/{session}/branches/{branch}/log";
pub(super) const SAVES: &str = "/api/play/free/{session}/saves";
const SAVE_LOG: &str = "/api/play/free/{session}/saves/{save}/input.log";
const SAVE_PICTURE: &str = "/api/play/free/{session}/saves/{save}/picture.png";
const SAVE_STATE: &str = "/api/play/free/{session}/saves/{save}/state";
pub(super) const START_STATE: &str = "/api/play/free/{session}/start-state";
pub(super) const INCLUDE: &str = "/api/play/free/{session}/include";
pub(super) const CLIP: &str = "/api/play/free/{session}/clips/{clip}";

/// Held while a session's file is read and replaced.
static WRITE: Mutex<()> = Mutex::new(());

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: SESSIONS,
            about: "Free play sessions with their save points and clips, newest first (?run=RUN for one run's); POST {run, recording?, frame?} starts one",
            handler: sessions,
        },
        Route {
            path: SESSION,
            about: "One free play session: its branches, save points and clips",
            handler: session,
        },
        Route {
            path: BRANCHES,
            about: "POST {from_save?}: a new branch of the session's input, from a save point (its log) or from the session's start",
            handler: branches,
        },
        Route {
            path: BRANCH_LOG,
            about: "POST {offset, lines}: the next frames of a branch's input log, as input log frame lines",
            handler: branch_log,
        },
        Route {
            path: SAVES,
            about: "POST {branch, frame, hash, digest, picture?}: a save point at a frame of a branch; it's rebuilt natively and its hashes checked against the page's",
            handler: saves,
        },
        Route {
            path: SAVE_LOG,
            about: "A save point's input log: the frames from power-on to it",
            handler: save_log,
        },
        Route {
            path: SAVE_PICTURE,
            about: "A save point's picture",
            handler: save_picture,
        },
        Route {
            path: SAVE_STATE,
            about: "A save point's player state bytes, rebuilt natively from its input log and checked against the hash the page sent, for loading it after a page load",
            handler: save_state,
        },
        Route {
            path: START_STATE,
            about: "POST {branch, at}: the original's state at frame `at` of a branch, translated into the run's model, as the game view's start-state gives it",
            handler: start_state,
        },
        Route {
            path: CLIP,
            about: "POST {mark}: keeps, skips or stars a clip (`kept`, `skipped`, `starred`); a skipped clip leaves the background pool",
            handler: clip_mark,
        },
        Route {
            path: INCLUDE,
            about: "POST {run}: the session's clips join that capture run's scenarios too",
            handler: include,
        },
    ]
}

/// One branch of the session's input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Branch {
    pub id: u64,
    /// The save point it was loaded from; none for the first.
    pub from_save: Option<u64>,
    /// The frame its own presses start at.
    pub from_frame: u64,
    /// Frames uploaded so far, from power-on.
    pub frames: u64,
}

/// What the native rebuild of a save point reached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Native {
    pub hash: String,
    pub digest: String,
    pub matches: bool,
}

/// One save point: frame `frame` of branch `branch`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavePoint {
    pub id: u64,
    pub branch: u64,
    pub frame: u64,
    /// The page's state hash and frame hash digest there (hex).
    pub hash: String,
    pub digest: String,
    pub made_ms: u64,
    /// Whether the page sent a picture.
    #[serde(default)]
    pub picture: bool,
    /// The native rebuild, once it's done.
    #[serde(default)]
    pub native: Option<Native>,
    #[serde(default)]
    pub error: Option<String>,
}

/// `session.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub schema: u32,
    pub id: String,
    /// The capture run whose ROM it plays and whose model the clips run on.
    pub run: String,
    /// Other runs whose scenarios include the clips.
    #[serde(default)]
    pub runs: Vec<String>,
    /// Where it started: a recording of the run to a frame, or power-on.
    pub recording: Option<String>,
    pub frame: u64,
    pub started_ms: u64,
    pub branches: Vec<Branch>,
    pub saves: Vec<SavePoint>,
    /// Each clip's place and mark, by clip ID.
    #[serde(default)]
    pub clip_meta: std::collections::BTreeMap<String, ClipMeta>,
}

/// Where a clip was cut from and what the person made of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ClipMeta {
    /// The save point it starts at.
    pub save: u64,
    /// The branch of the session's input its presses are frames of, and
    /// the first of them: the original's frames are that branch's from
    /// `from` for `frames`.
    pub branch: Option<u64>,
    pub from: u64,
    pub frames: u64,
    /// `auto` (cut from the play loop at a save or load) or `manual`.
    pub cut: String,
    /// What ended it: `save`, `load`, `cap` or `stop`.
    pub ended: String,
    /// `kept` (the default), `skipped` or `starred`.
    pub mark: String,
}

/// The marks a clip can have; a skipped clip leaves the background pool.
pub(crate) const MARKS: &[&str] = &["kept", "skipped", "starred"];

/// A clip a person played starts starred: it is special beside the clips
/// the tool makes (docs/show.md, "Clips made by the tool").
pub(crate) const PLAYED_MARK: &str = "starred";

/// The folder free play sessions live in.
pub(crate) fn root(home: &Path) -> PathBuf {
    home.join("play").join("free")
}

fn load(dir: &Path) -> anyhow::Result<Session> {
    let bytes = fs::read(dir.join("session.json"))
        .with_context(|| format!("no free play session at {}", dir.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn save(dir: &Path, s: &Session) -> anyhow::Result<()> {
    let tmp = dir.join(".session.json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    fs::rename(&tmp, dir.join("session.json"))?;
    Ok(())
}

fn update<T>(dir: &Path, f: impl FnOnce(&mut Session) -> Result<T, Fail>) -> Result<T, Fail> {
    let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = load(dir)?;
    let out = f(&mut s)?;
    save(dir, &s)?;
    Ok(out)
}

fn session_dir(ctx: &Ctx, params: &Params) -> Result<PathBuf, Fail> {
    let id = params.get("session").map(String::as_str).unwrap_or("");
    session_dir_of(&ctx.paths.home, id)
}

fn session_dir_of(home: &Path, id: &str) -> Result<PathBuf, Fail> {
    if !is_name(id) {
        return Err(Fail::not_found("no such free play session"));
    }
    let dir = root(home).join(id);
    if !dir.join("session.json").is_file() {
        return Err(Fail::not_found(format!("no free play session {id}")));
    }
    Ok(dir)
}

fn branch_file(dir: &Path, b: u64) -> PathBuf {
    dir.join("branches").join(format!("{b}.part"))
}

fn save_file(dir: &Path, k: u64, ext: &str) -> PathBuf {
    dir.join("saves").join(format!("{k}.{ext}"))
}

/// Frame lines under a header, as an input log.
fn parse_lines(lines: &str) -> Result<InputLog, String> {
    InputLog::parse(&format!("gp-input-log 1\ncontrollers 1\n{lines}")).map_err(|e| e.to_string())
}

/// The input log's frame lines, without its header.
fn lines_of(log: &InputLog) -> String {
    let text = log.to_text();
    text.lines()
        .skip_while(|l| !l.starts_with("controllers"))
        .skip(1)
        .map(|l| format!("{l}\n"))
        .collect()
}

/// The first `frames` frames of a branch, as an input log.
fn branch_prefix(dir: &Path, b: u64, frames: u64) -> Result<InputLog, Fail> {
    let text = fs::read_to_string(branch_file(dir, b)).unwrap_or_default();
    let mut log = parse_lines(&text).map_err(Fail::bad_request)?;
    if (log.frames.len() as u64) < frames {
        return Err(Fail {
            status: 409,
            message: format!(
                "branch {b} has {} frames uploaded; send them to frame {frames} first",
                log.frames.len()
            ),
        });
    }
    log.frames
        .truncate(usize::try_from(frames).unwrap_or(usize::MAX));
    Ok(log)
}

/// Every session, newest first.
fn all(home: &Path) -> Vec<Session> {
    let mut out: Vec<Session> = fs::read_dir(root(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| load(&e.path()).ok())
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.started_ms));
    out
}

/// A session as the page reads it: the file, with its clips.
fn view(home: &Path, s: &Session) -> Value {
    let mut v = json!(s);
    v["clips"] = json!(clips_of(&root(home).join(&s.id)));
    v
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    run: String,
    /// Starts from this recording of the run at `frame`; power-on without.
    #[serde(default)]
    recording: Option<String>,
    #[serde(default)]
    frame: u64,
}

fn sessions(ctx: &Ctx, req: &Request, _params: &Params) -> Result<Response, Fail> {
    let home = &ctx.paths.home;
    if req.method == "GET" || req.method == "HEAD" {
        let run = req.param("run");
        let list: Vec<Value> = all(home)
            .iter()
            .filter(|s| run.is_none_or(|r| s.run == r || s.runs.iter().any(|x| x == r)))
            .map(|s| view(home, s))
            .collect();
        return Ok(Response::json(&json!({"sessions": list})));
    }
    post_only(req)?;
    let st: Start = body(req)?;
    let rdir = run_named(ctx, &st.run)?;
    if rdir.platform() != Platform::Nes {
        return Err(Fail::conflict(format!(
            "free play on {} isn't here yet: the editor's player is the NES's (gp_play.wasm), and {} is an arcade game",
            st.run, st.run
        )));
    }
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(home) {
        return Err(Fail {
            status: 500,
            message: format!(
                "$GP_HOME is inside the git work tree {}; free play isn't written there",
                repo.display()
            ),
        });
    }
    // The first branch's frames come from the page, from power-on: its
    // player's log, with the game's own polled frames, even where it
    // replayed the recording to the start.
    if let Some(name) = &st.recording {
        return Err(Fail::not_found(format!(
            "free play from a recording ({name}) isn't here yet: a game has no capture run's recordings; start from power-on"
        )));
    } else if st.frame != 0 {
        return Err(Fail::bad_request(
            "a start frame needs the recording it's in",
        ));
    }
    let started_ms = now_ms();
    let id = format!("free-{started_ms}");
    let dir = root(home).join(&id);
    fs::create_dir_all(dir.join("branches")).map_err(anyhow::Error::from)?;
    fs::write(branch_file(&dir, 1), "").map_err(anyhow::Error::from)?;
    let s = Session {
        schema: SCHEMA,
        id,
        run: st.run,
        runs: Vec::new(),
        recording: st.recording,
        frame: st.frame,
        started_ms,
        branches: vec![Branch {
            id: 1,
            from_save: None,
            from_frame: st.frame,
            frames: 0,
        }],
        saves: Vec::new(),
        clip_meta: std::collections::BTreeMap::new(),
    };
    {
        let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
        save(&dir, &s)?;
    }
    Ok(Response::json(&json!({"session": view(home, &s)})))
}

fn session(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let dir = session_dir(ctx, params)?;
    Ok(Response::json(
        &json!({"session": view(&ctx.paths.home, &load(&dir)?)}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewBranch {
    #[serde(default)]
    from_save: Option<u64>,
}

fn branches(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let dir = session_dir(ctx, params)?;
    let nb: NewBranch = body(req)?;
    let b = update(&dir, |s| {
        let (lines, from_frame) = match nb.from_save {
            Some(k) => {
                let sp = s
                    .saves
                    .iter()
                    .find(|x| x.id == k)
                    .ok_or_else(|| Fail::not_found(format!("no save point {k}")))?;
                let log = fs::read_to_string(save_file(&dir, k, "log"))
                    .map_err(|_| Fail::not_found(format!("save point {k} has no log")))?;
                let log = InputLog::parse(&log).map_err(|e| Fail::bad_request(e.to_string()))?;
                (lines_of(&log), sp.frame)
            }
            None => {
                let first = s.branches.first().map_or(1, |b| b.id);
                let log = branch_prefix(&dir, first, s.frame)?;
                (lines_of(&log), s.frame)
            }
        };
        let id = s.branches.iter().map(|b| b.id).max().unwrap_or(0) + 1;
        fs::write(branch_file(&dir, id), lines).map_err(anyhow::Error::from)?;
        let b = Branch {
            id,
            from_save: nb.from_save,
            from_frame,
            frames: from_frame,
        };
        s.branches.push(b.clone());
        Ok(b)
    })?;
    Ok(Response::json(&json!({"branch": b})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lines {
    offset: u64,
    lines: String,
}

fn branch_log(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let dir = session_dir(ctx, params)?;
    let b: u64 = params
        .get("branch")
        .and_then(|b| b.parse().ok())
        .ok_or_else(|| Fail::not_found("no such branch"))?;
    let l: Lines = body(req)?;
    let n = parse_lines(&l.lines)
        .map_err(Fail::bad_request)?
        .frames
        .len() as u64;
    let frames = update(&dir, |s| {
        let br = s
            .branches
            .iter_mut()
            .find(|x| x.id == b)
            .ok_or_else(|| Fail::not_found(format!("no branch {b}")))?;
        if l.offset != br.frames {
            return Err(Fail {
                status: 409,
                message: format!("branch {b} has {} frames; send from there", br.frames),
            });
        }
        let mut text = l.lines.clone();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(branch_file(&dir, b))
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(anyhow::Error::from)?;
        br.frames += n;
        Ok(br.frames)
    })?;
    Ok(Response::json(&json!({"frames": frames})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewSave {
    branch: u64,
    frame: u64,
    hash: String,
    digest: String,
    /// A PNG as base64 or a data URL.
    #[serde(default)]
    picture: Option<String>,
}

fn saves(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    if req.method == "GET" || req.method == "HEAD" {
        let dir = session_dir(ctx, params)?;
        return Ok(Response::json(&json!({"saves": load(&dir)?.saves})));
    }
    post_only(req)?;
    let dir = session_dir(ctx, params)?;
    let ns: NewSave = body(req)?;
    let log = branch_prefix(&dir, ns.branch, ns.frame)?;
    let picture = ns
        .picture
        .as_deref()
        .map(|p| {
            let b64 = p.strip_prefix("data:image/png;base64,").unwrap_or(p);
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| Fail::bad_request(format!("the picture: {e}")))
        })
        .transpose()?;
    if picture
        .as_ref()
        .is_some_and(|p| !p.starts_with(b"\x89PNG\r\n\x1a\n"))
    {
        return Err(Fail::bad_request("the picture isn't a PNG"));
    }
    let sp = update(&dir, |s| {
        if !s.branches.iter().any(|b| b.id == ns.branch) {
            return Err(Fail::not_found(format!("no branch {}", ns.branch)));
        }
        let id = s.saves.iter().map(|x| x.id).max().unwrap_or(0) + 1;
        fs::create_dir_all(dir.join("saves")).map_err(anyhow::Error::from)?;
        fs::write(save_file(&dir, id, "log"), log.to_text()).map_err(anyhow::Error::from)?;
        if let Some(p) = &picture {
            fs::write(save_file(&dir, id, "png"), p).map_err(anyhow::Error::from)?;
        }
        let sp = SavePoint {
            id,
            branch: ns.branch,
            frame: ns.frame,
            hash: ns.hash.clone(),
            digest: ns.digest.clone(),
            made_ms: now_ms(),
            picture: picture.is_some(),
            native: None,
            error: None,
        };
        s.saves.push(sp.clone());
        Ok(sp)
    })?;
    // Rebuilt natively in the background, so the page's F5 doesn't wait.
    let rom = rom_of(ctx, &run_dir_of(ctx, &dir)?)?;
    let (dir2, k, sid) = (dir.clone(), sp.id, session_id(&dir));
    std::thread::spawn(move || {
        let r = rebuild_saved(&rom, &log, &sid, k).map(|(h, d, _)| (h, d));
        let _ = update(&dir2, |s| {
            if let Some(x) = s.saves.iter_mut().find(|x| x.id == k) {
                match r {
                    Ok((hash, digest)) => {
                        x.native = Some(Native {
                            matches: hash == x.hash && digest == x.digest,
                            hash,
                            digest,
                        });
                    }
                    Err(e) => x.error = Some(e),
                }
            }
            Ok(())
        });
    });
    Ok(Response::json(&json!({"save": sp})))
}

/// A save point's log played natively from power-on: the state hash and
/// the frame hash digest it reaches, in hex.
#[cfg(test)]
fn rebuild(rom: &[u8], log: &InputLog) -> Result<(String, String), String> {
    let mut p = gp_play::Player::replay(rom, &log.to_text())?;
    p.run_to(log.frames.len() as u64)?;
    Ok((p.state_hash().to_hex(), p.digest().to_hex()))
}

/// Save points' rebuilt states kept in memory, newest last: the base a
/// later save point's rebuild starts from.
static STATES: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// A rebuilt state kept: its session, its save point and its bytes.
type Kept = (String, u64, std::sync::Arc<Vec<u8>>);

/// How many rebuilt states are kept. Each holds the log and the hashes
/// from power-on, a few megabytes for a long session.
const KEPT_STATES: usize = 12;

/// Held while a save point is rebuilt: one at a time, so a burst of F5s
/// doesn't take every core.
static REBUILD: Mutex<()> = Mutex::new(());

fn session_id(dir: &Path) -> String {
    dir.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_owned()
}

/// Save point `k` of session `sid` rebuilt natively on its log: the state
/// hash, the digest and the player's state bytes. It starts from the
/// newest kept state of the session whose log is the start of `log`,
/// else from power-on, and the result is kept for the next.
pub(crate) fn rebuild_saved(
    rom: &[u8],
    log: &InputLog,
    sid: &str,
    k: u64,
) -> Result<(String, String, std::sync::Arc<Vec<u8>>), String> {
    let _g = REBUILD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(bytes) = kept_state(sid, k) {
        let mut p = gp_play::Player::new(rom)?;
        p.restore(&bytes)?;
        if p.log().frames == log.frames {
            return Ok((p.state_hash().to_hex(), p.digest().to_hex(), bytes));
        }
    }
    let n = log.frames.len();
    let bases: Vec<std::sync::Arc<Vec<u8>>> = STATES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .rev()
        .filter(|(s, _, _)| s == sid)
        .map(|(_, _, b)| b.clone())
        .collect();
    let mut player = None;
    for b in bases {
        let mut p = gp_play::Player::new(rom)?;
        if p.restore(&b).is_err() {
            continue;
        }
        let m = p.log().frames.len();
        if m <= n && p.log().frames[..] == log.frames[..m] {
            player = Some(p);
            break;
        }
    }
    let p = match player {
        Some(mut p) => {
            let m = p.log().frames.len();
            for f in &log.frames[m..] {
                p.step(f.pads[0], f.reset)?;
            }
            p
        }
        None => {
            let mut p = gp_play::Player::replay(rom, &log.to_text())?;
            p.run_to(n as u64)?;
            p
        }
    };
    let bytes = std::sync::Arc::new(p.save()?);
    {
        let mut kept = STATES.lock().unwrap_or_else(|e| e.into_inner());
        kept.retain(|(s, x, _)| !(s == sid && *x == k));
        kept.push((sid.to_owned(), k, bytes.clone()));
        let over = kept.len().saturating_sub(KEPT_STATES);
        kept.drain(..over);
    }
    Ok((p.state_hash().to_hex(), p.digest().to_hex(), bytes))
}

fn kept_state(sid: &str, k: u64) -> Option<std::sync::Arc<Vec<u8>>> {
    STATES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(s, x, _)| s == sid && *x == k)
        .map(|(_, _, b)| b.clone())
}

fn save_state(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let dir = session_dir(ctx, params)?;
    let k = save_id(params)?;
    let s = load(&dir)?;
    let sp = s
        .saves
        .iter()
        .find(|x| x.id == k)
        .ok_or_else(|| Fail::not_found(format!("no save point {k}")))?;
    let text = fs::read_to_string(save_file(&dir, k, "log"))
        .map_err(|_| Fail::not_found(format!("save point {k} has no log")))?;
    let log = InputLog::parse(&text).map_err(|e| Fail::bad_request(e.to_string()))?;
    let rom = rom_of(ctx, &run_dir_of(ctx, &dir)?)?;
    let (hash, _, bytes) = rebuild_saved(&rom, &log, &s.id, k).map_err(|e| Fail {
        status: 500,
        message: format!("save point {k} couldn't be rebuilt: {e}"),
    })?;
    if hash != sp.hash {
        return Err(Fail {
            status: 409,
            message: format!(
                "save point {k} rebuilds to another state ({hash}) than the page saved ({})",
                sp.hash
            ),
        });
    }
    Ok(Response::bytes("application/octet-stream", bytes.to_vec()))
}

fn run_dir_of(ctx: &Ctx, dir: &Path) -> Result<Run, Fail> {
    run_named(ctx, &load(dir)?.run)
}

fn save_id(params: &Params) -> Result<u64, Fail> {
    params
        .get("save")
        .and_then(|k| k.parse().ok())
        .ok_or_else(|| Fail::not_found("no such save point"))
}

fn save_log(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let dir = session_dir(ctx, params)?;
    let k = save_id(params)?;
    let bytes = fs::read(save_file(&dir, k, "log"))
        .map_err(|_| Fail::not_found(format!("no save point {k}")))?;
    Ok(Response::bytes("text/plain; charset=utf-8", bytes))
}

fn save_picture(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let dir = session_dir(ctx, params)?;
    let k = save_id(params)?;
    let bytes = fs::read(save_file(&dir, k, "png"))
        .map_err(|_| Fail::not_found(format!("save point {k} has no picture")))?;
    Ok(Response::bytes("image/png", bytes))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartIn {
    branch: u64,
    at: u64,
}

fn start_state(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let dir = session_dir(ctx, params)?;
    let si: StartIn = body(req)?;
    let frames = load(&dir)?
        .branches
        .iter()
        .find(|b| b.id == si.branch)
        .map(|b| b.frames)
        .ok_or_else(|| Fail::not_found(format!("no branch {}", si.branch)))?;
    let log = branch_prefix(&dir, si.branch, frames)?;
    let run = run_dir_of(ctx, &dir)?;
    if !run.game.join(gp_interp::mapping::MAPPING_FILE).is_file() {
        return Err(Fail::not_found(
            "the game has no state mapping (mapping.toml)",
        ));
    }
    let rom = rom_of(ctx, &run)?;
    // The branch kept as a path, so the start comes from the start cache
    // the mechanic page shares.
    let (sha, _) = crate::mechanic::paths::put_path(&ctx.paths.home, &log.to_text())?;
    let s = match crate::mechanic::live::start(&ctx.paths, &run.rebuild(ctx), &rom, &sha, si.at) {
        Ok(s) => s,
        // Not yet: the start is taken a frame or more after `at`, and the
        // branch doesn't reach it, or the game isn't under way. The page
        // asks again as it plays on.
        Err(e) => {
            return Ok(Response::json(
                &json!({"asked": si.at, "ready": false, "why": format!("{e:#}")}),
            ));
        }
    };
    Ok(Response::json(&json!({
        "asked": si.at,
        "ready": true,
        "original_frame": s["original_frame"],
        "frame": s["frame"],
        "state": s["state"],
        "missing": s["missing"],
        "notes": [],
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Include {
    run: String,
}

fn include(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let dir = session_dir(ctx, params)?;
    let inc: Include = body(req)?;
    run_named(ctx, &inc.run)?;
    let s = update(&dir, |s| {
        if s.run != inc.run && !s.runs.contains(&inc.run) {
            s.runs.push(inc.run.clone());
        }
        Ok(s.clone())
    })?;
    Ok(Response::json(
        &json!({"session": view(&ctx.paths.home, &s)}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkIn {
    mark: String,
}

fn clip_mark(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    post_only(req)?;
    let dir = session_dir(ctx, params)?;
    let id = params.get("clip").cloned().unwrap_or_default();
    let m: MarkIn = body(req)?;
    if !MARKS.contains(&m.mark.as_str()) {
        return Err(Fail::bad_request(
            "a clip's mark is kept, skipped or starred",
        ));
    }
    let clip = clips_of(&dir)
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| Fail::not_found(format!("no clip {id}")))?;
    let s = update(&dir, |s| {
        let save = clip
            .start
            .as_ref()
            .and_then(|st| st.input.rsplit('/').next())
            .and_then(|k| k.parse().ok())
            .unwrap_or(0);
        let e = s.clip_meta.entry(id.clone()).or_insert_with(|| ClipMeta {
            save,
            branch: None,
            from: clip.start.as_ref().map_or(0, |st| st.frame),
            frames: clip.frames,
            cut: "manual".into(),
            ended: "stop".into(),
            mark: PLAYED_MARK.into(),
        });
        e.mark.clone_from(&m.mark);
        Ok(s.clone())
    })?;
    Ok(Response::json(
        &json!({"session": view(&ctx.paths.home, &s)}),
    ))
}

/// The marks of every clip of the sessions on `run`, by clip ID.
pub(crate) fn marks_for_run(home: &Path, run: &str) -> std::collections::BTreeMap<String, String> {
    all(home)
        .into_iter()
        .filter(|s| s.run == run || s.runs.iter().any(|r| r == run))
        .flat_map(|s| s.clip_meta.into_iter().map(|(k, m)| (k, m.mark)))
        .collect()
}

/// A session's clips, by file name.
fn clips_of(dir: &Path) -> Vec<gp_interp::scenario::Scenario> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir.join("clips"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| fs::read(f).ok())
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect()
}

/// The clips of every session that plays on `run` or was included in it,
/// for its scenarios.
pub(crate) fn clips_for_run(home: &Path, run: &str) -> Vec<gp_interp::scenario::Scenario> {
    let mut sessions = all(home);
    sessions.reverse();
    sessions
        .iter()
        .filter(|s| s.run == run || s.runs.iter().any(|r| r == run))
        .flat_map(|s| clips_of(&root(home).join(&s.id)))
        .collect()
}

/// A clip from a save point (`SESSION/K`): the scenario a played sequence
/// is, whose start is the save point's log and frame, written into the
/// session's `clips/`. `replace` names a clip of the same save point to
/// overwrite (a take recorded again).
pub(super) fn save_clip(
    home: &Path,
    save: &str,
    presses: InputLog,
    name: Option<&str>,
    replace: Option<&str>,
    place: &ClipPlace,
) -> Result<gp_interp::scenario::Scenario, Fail> {
    use gp_interp::scenario::{Scenario, Source, StartAt};
    let (sid, k) = save
        .split_once('/')
        .and_then(|(s, k)| Some((s, k.parse::<u64>().ok()?)))
        .ok_or_else(|| Fail::bad_request("name the save point as SESSION/K"))?;
    let dir = session_dir_of(home, sid)?;
    let s = load(&dir)?;
    let sp = s
        .saves
        .iter()
        .find(|x| x.id == k)
        .ok_or_else(|| Fail::not_found(format!("no save point {k} in {sid}")))?;
    let n = presses.frames.len() as u64;
    if n == 0 {
        return Err(Fail::bad_request("the clip has no frames"));
    }
    if n > MAX_CLIP_FRAMES {
        return Err(Fail::bad_request(format!(
            "a clip is at most {MAX_CLIP_FRAMES} frames"
        )));
    }
    if !["auto", "manual"].contains(&place.cut.as_str()) {
        return Err(Fail::bad_request("a clip's cut is auto or manual"));
    }
    if !["save", "load", "cap", "stop"].contains(&place.ended.as_str()) {
        return Err(Fail::bad_request(
            "a clip ends at a save, a load, the cap or a stop",
        ));
    }
    let clips = dir.join("clips");
    let g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let (file, id) = match replace {
        Some(id) => {
            let c = clips_of(&dir)
                .into_iter()
                .find(|c| c.id == id)
                .ok_or_else(|| Fail::not_found(format!("no clip {id} in {sid}")))?;
            let same = c
                .start
                .as_ref()
                .is_some_and(|st| st.input == start_name(sid, k));
            if !same {
                return Err(Fail::bad_request(format!(
                    "clip {id} starts at another save point"
                )));
            }
            let c_no = id.rsplit('.').next().unwrap_or("0").to_owned();
            (clips.join(format!("{c_no:0>4}.json")), id.to_owned())
        }
        None => {
            let c = fs::read_dir(&clips).map(|d| d.count()).unwrap_or(0) + 1;
            (
                clips.join(format!("{c:04}.json")),
                format!("clip.{}.{c}", short(sid)),
            )
        }
    };
    let rel = save_file(Path::new("play/free").join(sid).as_path(), k, "log");
    let secs = n as f64 / NTSC_FPS;
    let scenario = Scenario {
        id,
        name: name.map_or_else(
            || {
                format!(
                    "Clip from save point {k} at {}, {secs:.1} s",
                    clock(sp.frame)
                )
            },
            str::to_owned,
        ),
        tier: 0,
        mechanic: String::new(),
        params: Vec::new(),
        frames: n,
        presses: Vec::new(),
        requires: Vec::new(),
        log: Some(presses.to_text()),
        start: Some(StartAt {
            input: start_name(sid, k),
            frame: sp.frame,
            snapshot: None,
            log: Some(rel.to_string_lossy().into_owned()),
            model_frame: None,
        }),
        source: Source::Played {
            when: String::new(),
            by: "free play".into(),
        },
    };
    fs::create_dir_all(&clips).map_err(anyhow::Error::from)?;
    fs::write(
        &file,
        serde_json::to_vec_pretty(&scenario).map_err(anyhow::Error::from)?,
    )
    .map_err(anyhow::Error::from)?;
    drop(g);
    let meta = ClipMeta {
        save: k,
        branch: place.branch,
        from: place.from.unwrap_or(sp.frame),
        frames: n,
        cut: place.cut.clone(),
        ended: place.ended.clone(),
        mark: PLAYED_MARK.to_owned(),
    };
    update(&dir, |s| {
        // Recorded again: the mark stays.
        let mark = s
            .clip_meta
            .get(&scenario.id)
            .map_or_else(|| meta.mark.clone(), |m| m.mark.clone());
        s.clip_meta
            .insert(scenario.id.clone(), ClipMeta { mark, ..meta });
        Ok(())
    })?;
    Ok(scenario)
}

/// Where a clip came from, as the page says when it saves one.
pub(crate) struct ClipPlace {
    pub branch: Option<u64>,
    pub from: Option<u64>,
    pub cut: String,
    pub ended: String,
}

/// How a clip's start names its save point.
fn start_name(session: &str, k: u64) -> String {
    format!("save point {session}/{k}")
}

/// The session's number without its prefix, for short clip IDs.
fn short(session: &str) -> &str {
    session.strip_prefix("free-").unwrap_or(session)
}

/// Minutes and seconds of play at a frame: `0:41`.
fn clock(frame: u64) -> String {
    let s = (frame as f64 / NTSC_FPS) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_evidence::input_log::{InputFrame, button};

    #[test]
    fn lines_round_trip_without_the_header() {
        let mut log = InputLog::new(1);
        for (polled, pad) in [(true, button::A), (false, 0), (true, button::RIGHT)] {
            log.frames.push(InputFrame {
                polled,
                pads: [pad, 0, 0, 0],
                reset: false,
            });
        }
        let lines = lines_of(&log);
        assert!(!lines.contains("gp-input-log"));
        assert_eq!(parse_lines(&lines).unwrap(), log);
    }

    #[test]
    fn a_save_points_log_rebuilds_to_the_players_hashes() {
        let rom = gp_nes::testrom::build();
        let mut p = gp_play::Player::new(&rom).unwrap();
        for i in 0..80u32 {
            p.step(if i % 5 < 2 { button::A } else { button::LEFT }, false)
                .unwrap();
        }
        let saved = p.save().unwrap();
        for _ in 0..30 {
            p.step(button::B, false).unwrap();
        }
        // A branch from frame 80: the page restores the bytes and plays on.
        p.restore(&saved).unwrap();
        for _ in 0..25 {
            p.step(button::RIGHT, false).unwrap();
        }
        let log = InputLog::parse(&p.input_log()).unwrap();
        assert_eq!(log.frames.len(), 105);
        let want = (p.state_hash().to_hex(), p.digest().to_hex());
        assert_eq!(rebuild(&rom, &log).unwrap(), want);
        // From the save point at frame 80 rebuilt first, the same.
        let mut first = InputLog::parse(&p.input_log()).unwrap();
        first.frames.truncate(80);
        let (h, _, b80) = rebuild_saved(&rom, &first, "test-chain", 1).unwrap();
        let mut q = gp_play::Player::new(&rom).unwrap();
        q.restore(&saved).unwrap();
        assert_eq!(h, q.state_hash().to_hex());
        assert_eq!(b80.len(), saved.len());
        let (h, d, bytes) = rebuild_saved(&rom, &log, "test-chain", 2).unwrap();
        assert_eq!((h, d), want);
        assert_eq!(*bytes, p.save().unwrap());
        // A log that isn't the kept one's continuation starts from power-on.
        let mut other = log.clone();
        other.frames[3].pads[0] = button::B;
        let (h, ..) = rebuild_saved(&rom, &other, "test-chain", 3).unwrap();
        assert_eq!(h, rebuild(&rom, &other).unwrap().0);
    }

    #[test]
    fn clocks_read_as_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(2470), "0:41");
        assert_eq!(clock(3700), "1:01");
    }
}
