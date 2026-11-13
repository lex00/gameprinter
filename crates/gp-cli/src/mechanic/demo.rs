//! `gp mechanic demo`: a decision model at the controller. From a clip's
//! start in the session, every few frames the model gets the player's
//! state and the hint and picks the buttons to hold, until the mechanic
//! shows (for a jump, `--times` jumps up and back down) or the frames run
//! out. As it plays it writes `live.ndjson` in the session, which the page
//! plays as it comes (WATCHING YOUR AGENT PLAY); the clip goes into the
//! session made by the model, with each decision's probabilities beside it
//! (`clips/NNN.decisions.ndjson`). And `gp mechanic say --judge`: the model
//! picks among a question's choices from a clip's measured jumps.
//!
//! The model is Laya (Convai's open, Jev-compatible decision model), over
//! Jev's wire format on 127.0.0.1:7720 (scripts/laya/serve.mjs). When none
//! answers there, `gp serve` starts it from the install and stops it when
//! gp serve stops ([`crate::laya`]); `just laya` installs and serves it by
//! hand. The decision is the model's; where play
//! starts, the state it reads and the check that the mechanic happened are
//! the tools'.
//!
//! Ported from game-printer's mechanic.rs. game-printer started a demo
//! where stage 0's start runs reach play (`reach::Starts`,
//! `rom_extract::play_start`), which gameprinter hasn't got; a demo here
//! starts from a clip of the session (`--from`, by default its newest), as
//! the person left the game there. The original runs through
//! `OriginalRuntime`, and the player's place is read from the game's state
//! mapping (`entities.player[0].x` and `.y` and the RAM they name), not a
//! ROM dataset. On an arcade game (port wave 6) the model picks among the
//! board's chords instead ([`ARCADE_CHORDS`], gp-explore's arcade palette:
//! the stick, the tap, a coin and Start), the player's place again from
//! the game's mapping; the jump measures are a platformer's, so an arcade
//! demo plays its `--frames` and makes its clip without counting jumps.

use std::fs;
use std::io::Write as _;
use std::net::TcpStream;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_evidence::input_log::{InputFrame, InputLog};
use gp_runtime::Runtime as _;
use serde_json::{Value, json};

use super::paths as clip_paths;
use super::session::Session;
use crate::Outcome;
use crate::original::{OriginalRuntime, Platform};

/// What `gp mechanic demo` was asked.
pub(crate) struct Demo<'a> {
    pub game: &'a str,
    pub mechanic: &'a str,
    pub hint: &'a str,
    pub decide: &'a str,
    pub times: u32,
    pub steps: &'a [String],
    pub frames: usize,
    pub every: usize,
    pub from: Option<u64>,
    pub games: Option<&'a Path>,
}

/// A movement step's pace: a step that starts "walk" or "run" is done when
/// the player moves across at that pace, not when it jumps.
fn pace_of(step: &str) -> Option<&'static str> {
    let w = step.trim_start().to_lowercase();
    if w.starts_with("walk") {
        Some("walk")
    } else if w.starts_with("run") {
        Some("run")
    } else {
        None
    }
}

/// The buttons a decision can hold, by the names the model picks among
/// (an NES pad's bits: A 0x01, B 0x02, Left 0x40, Right 0x80).
const CHORDS: &[(&str, &str, u8)] = &[
    ("right", "hold Right only: walk right", 0x80),
    ("tap_jump", "tap A once and let go: a short, low jump", 0x01),
    (
        "jump",
        "press and hold A: jump straight up, higher the longer A is held",
        0x01,
    ),
    (
        "right_jump",
        "hold Right and A: jump while moving right",
        0x81,
    ),
    ("run_jump", "hold Right, B and A: a running jump", 0x83),
    ("run", "hold Right and B: run right", 0x82),
    ("wait", "hold nothing", 0x00),
    ("left", "hold Left only: walk left", 0x40),
];

/// The arcade board's chords a decision can hold, by name, as gp-explore's
/// arcade palette has them (coin 0x01, start 0x02, right 0x04, left 0x08,
/// down 0x10, up 0x20, tap 0x40).
const ARCADE_CHORDS: &[(&str, &str, u8)] = &[
    (
        "up",
        "push the stick up: move up a bar",
        gp_explore::arcade::UP,
    ),
    (
        "down",
        "push the stick down: move down a bar",
        gp_explore::arcade::DOWN,
    ),
    (
        "left",
        "push the stick left: walk left along the bar",
        gp_explore::arcade::LEFT,
    ),
    (
        "right",
        "push the stick right: walk right along the bar",
        gp_explore::arcade::RIGHT,
    ),
    (
        "tap",
        "hold the tap: pour; let go to send the drink",
        gp_explore::arcade::TAP,
    ),
    ("wait", "hold nothing", 0),
    ("coin", "drop a coin", gp_explore::arcade::COIN),
    ("start", "press Start 1", gp_explore::arcade::START),
];

/// POSTs JSON to `url` (`http://HOST:PORT/PATH`) on this machine and reads
/// the JSON answer.
fn post_json(url: &str, body: &Value) -> Result<Value> {
    use std::io::Read as _;
    let rest = url
        .strip_prefix("http://")
        .context("the decide URL is http://HOST:PORT")?;
    let (host, path) = rest
        .split_once('/')
        .map_or((rest, "/".to_owned()), |(h, p)| (h, format!("/{p}")));
    let mut conn = TcpStream::connect(host)
        .with_context(|| format!("no decision server at {url}: gp serve starts Laya when a tool needs it, or start one with `just laya`"))?;
    let text = body.to_string();
    write!(
        conn,
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    )?;
    let mut out = String::new();
    conn.read_to_string(&mut out)?;
    let (head, body) = out.split_once("\r\n\r\n").context("no HTTP answer")?;
    if !head.starts_with("HTTP/1.1 200") {
        bail!(
            "the decision server answered {}: {body}",
            head.lines().next().unwrap_or("")
        );
    }
    Ok(serde_json::from_str(body)?)
}

/// The player's x and y RAM addresses, from the game's state mapping
/// (`entities.player[0].x` and `.y`, each field's first `ram`).
pub(crate) fn player_bytes(game_dir: &Path) -> Result<(Option<u16>, u16)> {
    let file = game_dir.join(gp_interp::mapping::MAPPING_FILE);
    let text = fs::read_to_string(&file).with_context(|| {
        format!(
            "no {}: a demo reads the player's place from the game's state mapping",
            file.display()
        )
    })?;
    let m: toml::Table = toml::from_str(&text).with_context(|| file.display().to_string())?;
    let field = |path: &str| -> Option<u16> {
        m.get("field")?.as_array()?.iter().find_map(|f| {
            (f.get("path")?.as_str()? == path)
                .then(|| f.get("ram")?.as_array()?.first()?.as_integer())
                .flatten()
                .and_then(|a| u16::try_from(a).ok())
        })
    };
    let y = field("entities.player[0].y").with_context(|| {
        format!(
            "{} has no entities.player[0].y with a ram byte; a demo needs one to see a jump",
            file.display()
        )
    })?;
    Ok((field("entities.player[0].x"), y))
}

/// A RAM byte of the original, as its `ram[...]` query reads it.
fn ram(o: &OriginalRuntime, addr: u16) -> Result<u8> {
    let e = o
        .query(&format!("ram[{addr:#06x}]"))
        .map_err(anyhow::Error::msg)?;
    let v = e
        .first()
        .and_then(|e| e.value.as_int())
        .with_context(|| format!("ram[{addr:#06x}] reads no number"))?;
    Ok(v as u8)
}

/// Steps the original one frame on an NES pad byte.
fn step(o: &mut OriginalRuntime, f: InputFrame) -> Result<()> {
    let one = InputLog {
        controllers: 1,
        frames: vec![f],
        ..InputLog::new(1)
    };
    o.step(&crate::run::frames(&one))
        .map_err(anyhow::Error::msg)?;
    Ok(())
}

fn pad(p: u8) -> InputFrame {
    InputFrame {
        polled: true,
        pads: [p, 0, 0, 0],
        reset: false,
    }
}

/// The game's folder in the games checkout and its ROM.
fn game_and_rom(
    paths: &Paths,
    game: &str,
    games: Option<&Path>,
) -> Result<(std::path::PathBuf, Vec<u8>)> {
    let games = crate::roms_cmd::games_dir(games)
        .context("no games checkout: pass --games or set GP_GAMES_DIR")?;
    let dir = games.join(game);
    let rom = crate::clip_score::rom_of(paths, &dir, None)?;
    Ok((dir, rom))
}

/// The play up to a clip's start, kept as a path: its hash and its frames.
fn start_of(paths: &Paths, s: &Session, n: u64) -> Result<(String, InputLog, usize)> {
    let (text, start) = clip_paths::full_log(&paths.home, &s.dir, n)?;
    let mut log = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("clip {n}: {e}"))?;
    log.frames.truncate(start);
    let (sha, _) = clip_paths::put_path(&paths.home, &log.to_text())?;
    Ok((sha, log, start))
}

pub(crate) fn demo(paths: &Paths, a: &Demo) -> Result<Outcome> {
    let session = Session::open(paths, a.game, a.mechanic)?;
    // Laya answering before the first decision: gp serve starts it when
    // none does ([`crate::laya`]).
    eprintln!(
        "gp mechanic demo: {}",
        crate::laya::for_tool(&paths.home, a.decide)?
    );
    let (dir, rom) = game_and_rom(paths, a.game, a.games)?;
    let arcade = Platform::of_file(&rom) == Platform::Arcade;
    let chords = if arcade { ARCADE_CHORDS } else { CHORDS };
    // The log a clip is kept in: the board's columns on an arcade game.
    let new_log = || -> Result<InputLog> {
        Ok(if arcade {
            let set = gp_arcade::Romset::from_zip(&rom)?;
            InputLog::new_arcade(set.name, "CSRLDUT")
        } else {
            InputLog::new(1)
        })
    };
    let (xb, yb) = player_bytes(&dir).map_err(|e| {
        if arcade {
            anyhow::anyhow!(
                "{} is an arcade game whose state mapping doesn't name the player yet, and a demo reads the player's place from it: {e:#}",
                a.game
            )
        } else {
            e
        }
    })?;
    let from = match a.from {
        Some(n) => n,
        None => session
            .clips()?
            .last()
            .and_then(|c| c["n"].as_u64())
            .context("no clip in this session to start from: record one where the demo should start, or give --from N")?,
    };
    let (sha, prefix, at) = start_of(paths, &session, from)?;
    let mut o = clip_paths::original_at(&paths.home, &rom, &sha, at)?;
    o.set_video(false);
    let mut inputs: Vec<InputFrame> = prefix.frames.clone();
    let idle = pad(0);
    // The clip starts with the player standing: the start can be the middle
    // of a jump. Nothing is held until its y stays put for 3 frames (at
    // most 3 seconds).
    let mut still = 0usize;
    let mut y_prev = ram(&o, yb)?;
    for _ in 0..180 {
        if still >= 3 {
            break;
        }
        step(&mut o, idle)?;
        inputs.push(idle);
        let y = ram(&o, yb)?;
        still = if y == y_prev { still + 1 } else { 0 };
        y_prev = y;
    }
    let start = inputs.len();
    // What the page plays as it happens: the start, each decision with the
    // presses it held, then the clip.
    let live_path = session.dir.join("live.ndjson");
    let mut head = new_log()?;
    head.frames = inputs.clone();
    fs::write(
        &live_path,
        json!({"kind": "start", "hint": a.hint, "steps": a.steps, "start": start, "log": head.to_text(), "times": a.times, "from": from}).to_string() + "\n",
    )?;
    let live = |v: Value| -> Result<()> {
        let mut f = fs::OpenOptions::new().append(true).open(&live_path)?;
        writeln!(f, "{v}")?;
        Ok(())
    };
    let criteria: serde_json::Map<String, Value> = chords
        .iter()
        .map(|(n, d, _)| ((*n).to_owned(), json!(d)))
        .collect();
    // Each jump: the y it left from, its highest point, frames A was held,
    // and pixels a frame across over the 4 frames before it left the ground.
    struct Jump {
        from: u8,
        top: u8,
        held_a: usize,
        takeoff: f64,
    }
    let rise = |j: &Jump| i32::from(j.from) - i32::from(j.top);
    let mut xs: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
    let mut jumps: Vec<Jump> = Vec::new();
    let mut air: Option<Jump> = None;
    let mut last_dy = 0i32;
    let mut moved_x = 0i32;
    // The step the model is on, and decisions spent at a movement step's
    // pace.
    let mut step_i = 0usize;
    let mut at_pace = 0usize;
    let mut decisions = Vec::new();
    let mut played = 0usize;
    while played < a.frames && (jumps.len() as u32) < a.times {
        let vertical = match &air {
            None => "standing on the ground",
            Some(_) if last_dy < 0 => "in the air, going up",
            Some(_) => "in the air, coming down",
        };
        // How the player moves across, from its x byte over the last
        // decision: still, walking, or running (2 px a frame or more).
        let across = match xb {
            Some(_) if moved_x == 0 => "standing still",
            Some(_) if moved_x.unsigned_abs() >= 2 * a.every as u32 => "running",
            Some(_) => "walking",
            None => "unknown",
        };
        let mut state = json!({
            "goal": a.hint,
            "player": vertical,
            "moving_across": across,
            "jumps_done": jumps.len(),
            "jumps_wanted": a.times,
            "jump_heights_so_far": jumps.iter().map(|j| format!("{} px", rise(j))).collect::<Vec<_>>(),
        });
        if let Some(st) = a.steps.get(step_i) {
            state["now"] = json!(st);
            state["step"] = json!(format!("{} of {}", step_i + 1, a.steps.len()));
        }
        // A movement step is offered only the choices without A.
        let moving = a.steps.get(step_i).and_then(|st| pace_of(st)).is_some();
        let offered: serde_json::Map<String, Value> = criteria
            .iter()
            .filter(|(k, _)| {
                arcade || !moving || chords.iter().any(|c| c.0 == k.as_str() && c.2 & 0x01 == 0)
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let ask = json!({
            "model": "laya",
            "state": state,
            "questions": {"buttons": {"type": "choice",
                "instructions": "Which buttons should the player hold now to do what the step or goal says?",
                "criteria": offered}},
        });
        let answer = post_json(
            &format!("{}/v1/systemone", a.decide.trim_end_matches('/')),
            &ask,
        )?;
        let pick = answer["answers"]["buttons"]["choice"]
            .as_str()
            .unwrap_or("wait")
            .to_owned();
        let held = chords.iter().find(|c| c.0 == pick).map_or(0, |c| c.2);
        decisions.push(json!({"frame": played, "state": ask["state"], "choice": pick, "probabilities": answer["answers"]["buttons"]["probabilities"]}));
        // A jump starts on a new press of A: holding A on into a standing
        // decision jumps no more, so A is let go for its first frame, as a
        // thumb would; a tap holds A for one frame only.
        let prev = inputs.last().map_or(0, |f| f.pads[0]);
        // (The NES's A; on the arcade board bit 0 is the coin, held as is.)
        let repress = !arcade && held & 0x01 != 0 && prev & 0x01 != 0 && air.is_none();
        let first = inputs.len();
        let x_before = match xb {
            Some(b) => Some(ram(&o, b)?),
            None => None,
        };
        for i in 0..a.every {
            let up = (repress && i == 0) || (pick == "tap_jump" && i > usize::from(repress));
            let p = if up { held & !0x01 } else { held };
            step(&mut o, pad(p))?;
            inputs.push(pad(p));
            played += 1;
            // A jump: the player rises from where it stood; it ends when it
            // stands still again for 3 frames.
            let y = ram(&o, yb)?;
            last_dy = i32::from(y) - i32::from(y_prev);
            if let Some(b) = xb {
                xs.push_back(ram(&o, b)?);
                if xs.len() > 5 {
                    xs.pop_front();
                }
            }
            if !arcade && air.is_none() && still >= 3 && y < y_prev {
                let takeoff = match (xs.front(), xs.back()) {
                    (Some(a0), Some(a1)) if xs.len() > 1 => {
                        f64::from(a1.wrapping_sub(*a0) as i8) / (xs.len() - 1) as f64
                    }
                    _ => 0.0,
                };
                air = Some(Jump {
                    from: y_prev,
                    top: y,
                    held_a: 0,
                    takeoff,
                });
            }
            still = if y == y_prev { still + 1 } else { 0 };
            if let Some(j) = air.as_mut() {
                j.top = j.top.min(y);
                j.held_a += usize::from(p & 0x01 != 0);
                if still >= 3 {
                    jumps.push(air.take().expect("in the air"));
                    if a.steps.get(step_i).is_some_and(|st| pace_of(st).is_none()) {
                        step_i += 1;
                    }
                }
            }
            y_prev = y;
            if jumps.len() as u32 >= a.times {
                break;
            }
        }
        moved_x = match (x_before, xb) {
            (Some(a0), Some(b)) => i32::from(ram(&o, b)?.wrapping_sub(a0) as i8),
            _ => 0,
        };
        // A movement step is done once the player has moved across at its
        // pace for 6 decisions (time to reach the pace): the tool keeps the
        // order, the model only picks the buttons.
        if let Some(want) = a.steps.get(step_i).and_then(|st| pace_of(st)) {
            let per = moved_x.unsigned_abs() as usize;
            let ok = match want {
                "run" => per >= 2 * a.every,
                _ => per > 0,
            };
            at_pace = if ok { at_pace + 1 } else { 0 };
            if at_pace >= 6 {
                step_i += 1;
                at_pace = 0;
            }
        }
        live(
            json!({"kind": "decide", "choice": pick, "probabilities": answer["answers"]["buttons"]["probabilities"],
                    "pad": held, "frames": inputs.len() - first, "done": jumps.len(),
                    "step": a.steps.get(jumps.len().min(a.steps.len().saturating_sub(1))),
                    "heights": jumps.iter().map(rise).collect::<Vec<_>>()}),
        )?;
    }
    let heights: Vec<i32> = jumps.iter().map(rise).collect();
    let held: Vec<usize> = jumps.iter().map(|j| j.held_a).collect();
    let takeoff: Vec<f64> = jumps
        .iter()
        .map(|j| (j.takeoff * 100.0).round() / 100.0)
        .collect();
    let mut log = new_log()?;
    log.frames = inputs;
    let shown: Vec<String> = heights
        .iter()
        .zip(&held)
        .zip(&takeoff)
        .map(|((h, a), t)| format!("rose {h} px, A held {a} frames, took off at {t} px a frame"))
        .collect();
    let note = format!(
        "{} (made by Laya from clip {from}'s start: {}; {} of {})",
        a.hint,
        if shown.is_empty() {
            "no jump".to_owned()
        } else {
            shown.join("; ")
        },
        jumps.len(),
        a.times
    );
    let n = session.add_clip(&log.to_text(), start as u64, &note, "laya")?;
    live(
        json!({"kind": "end", "clip": n, "done": jumps.len(), "times": a.times, "heights": heights, "held": held, "takeoff": takeoff}),
    )?;
    let lines: Vec<String> = decisions.iter().map(Value::to_string).collect();
    fs::write(
        session.dir.join(format!("clips/{n:03}.decisions.ndjson")),
        lines.join("\n") + "\n",
    )?;
    let text = format!(
        "clip {n}: {played} frames from frame {start} (clip {from}'s start), {} decisions, {} of {} jumps: {}",
        decisions.len(),
        jumps.len(),
        a.times,
        shown.join("; ")
    );
    Ok(Outcome::ok(
        json!({"clip": n, "frames": played, "start": start, "from": from, "decisions": decisions.len(), "shown": jumps.len(), "heights": heights, "held": held, "takeoff": takeoff}),
        text,
    ))
}

/// Each jump in a clip: how high it rose, frames A was held, and the speed
/// across it took off at.
pub(super) fn measure_jumps(
    paths: &Paths,
    s: &Session,
    n: u64,
    rom: &[u8],
    game_dir: &Path,
) -> Result<Vec<Value>> {
    let (xb, yb) = player_bytes(game_dir)?;
    let (sha, _, at) = start_of(paths, s, n)?;
    let (text, _) = clip_paths::full_log(&paths.home, &s.dir, n)?;
    let log = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("clip {n}: {e}"))?;
    let mut o = clip_paths::original_at(&paths.home, rom, &sha, at)?;
    o.set_video(false);
    let mut out = Vec::new();
    let mut still = 0usize;
    let mut y_prev = ram(&o, yb)?;
    let mut xs: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
    // (from, top, held_a, takeoff)
    let mut air: Option<(u8, u8, usize, f64)> = None;
    for f in &log.frames[at.min(log.frames.len())..] {
        step(&mut o, *f)?;
        let y = ram(&o, yb)?;
        if let Some(b) = xb {
            xs.push_back(ram(&o, b)?);
            if xs.len() > 5 {
                xs.pop_front();
            }
        }
        if air.is_none() && still >= 3 && y < y_prev {
            let takeoff = match (xs.front(), xs.back()) {
                (Some(a0), Some(a1)) if xs.len() > 1 => {
                    f64::from(a1.wrapping_sub(*a0) as i8) / (xs.len() - 1) as f64
                }
                _ => 0.0,
            };
            air = Some((y_prev, y, 0, takeoff));
        }
        still = if y == y_prev { still + 1 } else { 0 };
        if let Some(j) = air.as_mut() {
            j.1 = j.1.min(y);
            j.2 += usize::from(f.pads[0] & 0x01 != 0);
            if still >= 3 {
                let (from, top, held, takeoff) = air.take().expect("in the air");
                out.push(
                    json!({"rose_px": i32::from(from) - i32::from(top), "a_held_frames": held,
                    "takeoff_px_a_frame": (takeoff * 100.0).round() / 100.0}),
                );
            }
        }
        y_prev = y;
    }
    Ok(out)
}

/// The decision model's pick among a question's choices, from a clip's
/// measured jumps and the other clips' notes for comparison: the turn's
/// `judge`, which the page shows beside the choices to confirm or dispute.
#[allow(clippy::too_many_arguments)]
pub(crate) fn judge(
    paths: &Paths,
    s: &Session,
    game: &str,
    games: Option<&Path>,
    clip: u64,
    question: &str,
    choices: &[String],
    facts: &[String],
    decide: &str,
) -> Result<Value> {
    let (dir, rom) = game_and_rom(paths, game, games)?;
    eprintln!(
        "gp mechanic say --judge: {}",
        crate::laya::for_tool(&paths.home, decide)?
    );
    let jumps = measure_jumps(paths, s, clip, &rom, &dir)?;
    let others: Vec<String> = s
        .clips()?
        .iter()
        .filter(|c| c["n"].as_u64() != Some(clip))
        .filter_map(|c| c["note"].as_str().map(|n| format!("clip {}: {n}", c["n"])))
        .collect();
    let criteria: serde_json::Map<String, Value> =
        choices.iter().map(|c| (c.clone(), json!(c))).collect();
    let ask = json!({
        "model": "laya",
        "state": {"clip": clip, "facts": facts, "jumps_measured": jumps, "other_clips": others},
        "questions": {"answer": {"type": "choice", "instructions": question, "criteria": criteria}},
    });
    let a = post_json(
        &format!("{}/v1/systemone", decide.trim_end_matches('/')),
        &ask,
    )?;
    let ans = &a["answers"]["answer"];
    Ok(json!({
        "choice": ans["choice"],
        "probabilities": ans["probabilities"],
        "confidence": ans["answer_confidence"],
        "measured": jumps,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arcade chords are the palette's buttons, one name each, and
    /// each holds what an arcade log's columns say (CSRLDUT from bit 0).
    #[test]
    fn the_arcade_chords_are_the_boards_buttons() {
        let mut names: Vec<&str> = ARCADE_CHORDS.iter().map(|c| c.0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ARCADE_CHORDS.len());
        let bits = |n: &str| ARCADE_CHORDS.iter().find(|c| c.0 == n).unwrap().2;
        let c = gp_arcade::Controls::from_bits(u32::from(bits("tap")));
        assert!(c.tap && !c.coin);
        assert!(gp_arcade::Controls::from_bits(u32::from(bits("coin"))).coin);
        assert!(gp_arcade::Controls::from_bits(u32::from(bits("up"))).up);
        assert!(gp_arcade::Controls::from_bits(u32::from(bits("start"))).start);
    }

    #[test]
    fn a_step_names_its_pace() {
        assert_eq!(pace_of("walk right a while"), Some("walk"));
        assert_eq!(pace_of(" Run to the edge"), Some("run"));
        assert_eq!(pace_of("a short hop"), None);
    }

    #[test]
    fn the_players_bytes_come_from_the_mapping() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join(gp_interp::mapping::MAPPING_FILE),
            "[[field]]\npath = \"entities.player[0].x\"\nram = [515]\n\n[[field]]\npath = \"entities.player[0].y\"\nram = [512]\n",
        )
        .unwrap();
        assert_eq!(player_bytes(d.path()).unwrap(), (Some(515), 512));
        fs::write(d.path().join(gp_interp::mapping::MAPPING_FILE), "").unwrap();
        let e = player_bytes(d.path()).unwrap_err().to_string();
        assert!(e.contains("entities.player[0].y"), "{e}");
    }

    #[test]
    fn no_decision_server_says_how_to_start_one() {
        // Port 9 on this machine: nothing listens there.
        let e = post_json("http://127.0.0.1:9/v1/systemone", &json!({}))
            .unwrap_err()
            .to_string();
        assert!(e.contains("just laya"), "{e}");
    }
}
