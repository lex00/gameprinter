//! A run's clips as scenarios (the parts of game-printer's
//! serve/scenarios.rs the main screen uses): saving a clip played in free
//! play, and a clip's input with the rebuilt game's start, for both games
//! to loop it.
//!
//! | Endpoint | What |
//! |---|---|
//! | `POST /api/capture/{run}/scenarios/played` | `{save, log, branch, from, cut, ended, replace?}`: a clip from a free play save point, kept in its session ([`super::freeplay::save_clip`]) |
//! | `GET /api/capture/{run}/scenarios/input?scenario=ID` | The clip's input from power-on, where it starts, and the rebuilt game's state there (the original's, translated through the game's mapping) |
//!
//! game-printer's scenario ladder on the run (`/scenarios`, its refine and
//! score) is not here yet.

use serde_json::{Value, json};

use super::game::{Run, rom_of, run_dir};
use super::{Ctx, Fail, Params, Route, post_only};
use crate::mechanic::http::{Request, Response};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/scenarios/played",
            about: "Saves a clip played from a free play save point",
            handler: played,
        },
        Route {
            path: "/api/capture/{run}/scenarios/input",
            about: "A clip's input from power-on, its start and the rebuilt game's state there",
            handler: input,
        },
    ]
}

fn played(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    run_dir(ctx, params)?;
    post_only(req)?;
    let v: Value = super::body(req)?;
    let save = v["save"].as_str().ok_or_else(|| {
        Fail::bad_request(
            "give the save point the clip starts at (save, as SESSION/K); a clip from a recording's frame needs a capture run's recordings, which gameprinter has none of",
        )
    })?;
    let log = v["log"]
        .as_str()
        .ok_or_else(|| Fail::bad_request("give the presses as an input log (log)"))?;
    let parsed = gp_evidence::InputLog::parse(log).map_err(|e| Fail::bad_request(e.to_string()))?;
    let place = super::freeplay::ClipPlace {
        branch: v["branch"].as_u64(),
        from: v["from"].as_u64(),
        cut: v["cut"].as_str().unwrap_or("manual").to_owned(),
        ended: v["ended"].as_str().unwrap_or("stop").to_owned(),
    };
    let s = super::freeplay::save_clip(
        &ctx.paths.home,
        save,
        parsed.clone(),
        v["name"].as_str(),
        v["replace"].as_str(),
        &place,
    )?;
    let session = to_session(ctx, params, &s, &parsed)?;
    Ok(Response::json(
        &json!({"saved": true, "scenario": s, "session": session}),
    ))
}

/// The mechanic session free play's clips are also kept in, so `gp corpus
/// import` and the mechanic page read them as they read the page's own.
pub(crate) const FREE_PLAY_SESSION: &str = "free-play";

/// Keeps a free play clip in the game's `free-play` mechanic session
/// (`$GP_HOME/mechanics/GAME/free-play/clips/NNN.toml` and `.log`): its
/// save point's input from power-on stored as a path, the clip's start
/// there and its own presses. The same clip saved again is the same
/// session clip. Says which clip it is, `free-play.NNN`.
fn to_session(
    ctx: &Ctx,
    params: &Params,
    s: &gp_interp::scenario::Scenario,
    presses: &gp_evidence::InputLog,
) -> Result<String, Fail> {
    let run = run_dir(ctx, params)?;
    let start = s
        .start
        .as_ref()
        .and_then(|st| st.log.as_ref())
        .ok_or_else(|| Fail::conflict("the clip has no save point log"))?;
    let text = std::fs::read_to_string(ctx.paths.home.join(start))?;
    let (path, at) = crate::mechanic::paths::put_path(&ctx.paths.home, &text)?;
    let rom = rom_of(ctx, &run)?;
    let sha = gp_evidence::source::identify_nes(&rom)
        .map(|i| i.sha256)
        .unwrap_or_default();
    let session = crate::mechanic::session::Session::create(
        &ctx.paths,
        &run.name,
        FREE_PLAY_SESSION,
        Some("clips played in the editor's free play (gp serve)"),
        &sha,
    )?;
    let (meta, log) = crate::mechanic::paths::clip_fields(&path, at, presses);
    let (n, _) = session.add_player_clip(meta, &log, &s.name)?;
    Ok(format!("{FREE_PLAY_SESSION}.{n:03}"))
}

/// A clip of the run, by ID: its input from power-on, its start frame and
/// its frames. A free play clip or a corpus clip.
pub(crate) fn clip_input(
    ctx: &Ctx,
    run: &Run,
    id: &str,
) -> Result<(gp_evidence::InputLog, usize, u64, Value), Fail> {
    if let Some(c) = super::freeplay::clips_for_run(&ctx.paths.home, &run.name)
        .into_iter()
        .find(|c| c.id == id)
    {
        let start = c
            .start
            .as_ref()
            .ok_or_else(|| Fail::conflict(format!("clip {id} has no start")))?;
        let rel = start
            .log
            .as_ref()
            .ok_or_else(|| Fail::conflict(format!("clip {id}'s start has no input log")))?;
        let file = ctx.paths.home.join(rel);
        let text = std::fs::read_to_string(&file)
            .map_err(|_| Fail::not_found(format!("clip {id}'s start log {rel} is gone")))?;
        let mut log = gp_evidence::InputLog::parse(&text).map_err(anyhow::Error::from)?;
        let presses = gp_evidence::InputLog::parse(&c.log_text().map_err(Fail::bad_request)?)
            .map_err(anyhow::Error::from)?;
        let at = log.frames.len();
        log.frames.extend(presses.frames);
        let frames = c.frames;
        return Ok((log, at, frames, json!(c)));
    }
    let (log, at) = super::generated::clip_log(run, id)?;
    let frames = (log.frames.len() - at) as u64;
    Ok((log, at, frames, json!({"id": id, "frames": frames})))
}

fn input(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    let id = req
        .param("scenario")
        .ok_or_else(|| Fail::bad_request("name a clip with ?scenario="))?;
    let (log, at, frames, scenario) = clip_input(ctx, &run, id)?;
    let rom = rom_of(ctx, &run)?;
    let (sha, _) = crate::mechanic::paths::put_path(&ctx.paths.home, &log.to_text())?;
    // The rebuilt game's start: the original's state at the clip's start,
    // translated. A start it can't take (a title screen) leaves the state
    // empty, and the page plays the original alone.
    let (start_frame, model_frame, state, why) =
        match crate::mechanic::live::start(&ctx.paths, &run.rebuild(ctx), &rom, &sha, at as u64) {
            Ok(s) => (
                s["original_frame"].as_u64().unwrap_or(at as u64),
                s["frame"].clone(),
                s["state"].clone(),
                Value::Null,
            ),
            Err(e) => (at as u64, Value::Null, json!(""), json!(format!("{e:#}"))),
        };
    Ok(Response::json(&json!({
        "scenario": scenario,
        "start_frame": start_frame,
        "model_frame": model_frame,
        "frames": frames,
        "log": log.to_text(),
        "state": state,
        "cant": why,
    })))
}
