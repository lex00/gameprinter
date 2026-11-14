//! `gp mechanic sweep jump`: the jumpathon. A grid of jumps from one spot
//! where the player stands on open ground, each cell a short clip (the
//! inputs from power-on that reach the spot, then the cell's buttons),
//! played on the original and on a rebuilt model in lockstep, the way the
//! whole-game measure plays a level (progress_measure.rs). The grid says
//! which takeoff speeds, A holds and steering the model gets wrong.
//!
//! Nothing here knows a game. The spot, the player's bytes and the buttons
//! come from blind discovery (blind.rs): play start, the pad rule, and the
//! jump and run buttons it found.
//!
//! Ported from game-printer's sweep.rs. What gameprinter's layout changed:
//! the game is its package in the games checkout (its folder is the default
//! model, its roms.toml names the ROM), where game-printer read
//! templates/GAME; the model runs through its module (`--module`) or the
//! built-in interpreter; blind discovery is wave 7's (crate::farm::blind);
//! and the part of game-printer's progress_measure.rs the sweep plays
//! (a start the rebuilt game takes, the lockstep from it) is [`measure`]
//! below. It plays gp-nes and reads the NES's RAM and sprite table, as
//! blind discovery's `Discovery` (a gp-nes machine at the spot) does, so
//! an arcade game is refused in words that say why: the sweep needs blind
//! discovery on the arcade board first, and a jump (Tapper has none).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::Outcome;
use gp_evidence::Paths;
use gp_evidence::input_log::{InputFrame, InputLog};
use gp_interp::lockstep::Mode;
use gp_interp::mapping::Mapping;
use gp_runtime::Runtime;

/// Speed at takeoff: frames of run-up, whether the run button is held,
/// and frames of reversing at the end of the run-up (a skid).
const SPEEDS: &[(&str, usize, bool, usize)] = &[
    ("stand", 0, false, 0),
    ("walk-short", 10, false, 0),
    ("walk", 40, false, 0),
    ("run-short", 15, true, 0),
    ("run", 60, true, 0),
    ("skid", 40, true, 6),
];
/// Frames A is held from takeoff.
const HOLDS: &[usize] = &[1, 3, 6, 10, 14, 18, 24, 32];
/// The d-pad in the air: none, forward, back, or forward then back.
const STEERS: &[&str] = &["none", "forward", "back", "switch"];
/// Frames after takeoff a cell runs for.
const AIR: usize = 90;

/// A frame with these buttons held.
fn press(pad: u8) -> InputFrame {
    InputFrame::held(pad)
}

/// The input log bits for the d-pad: right 0x80, left 0x40.
fn dpad(forward_is_right: bool, forward: bool) -> u8 {
    if forward == forward_is_right {
        0x80
    } else {
        0x40
    }
}

/// One cell's buttons after the spot.
fn cell_inputs(
    d: &crate::farm::blind::Discovery,
    speed: (&str, usize, bool, usize),
    hold: usize,
    steer: &str,
) -> Vec<InputFrame> {
    let (_, run_up, run, skid) = speed;
    let fwd = dpad(d.right, true);
    let back = dpad(d.right, false);
    let run_bit = if run { d.run } else { 0 };
    let mut out = Vec::new();
    for _ in 0..run_up {
        out.push(press(fwd | run_bit));
    }
    for _ in 0..skid {
        out.push(press(back | run_bit));
    }
    for k in 0..AIR {
        let held = if k < hold { d.jump } else { 0 };
        let pad = match steer {
            "forward" => fwd | run_bit,
            "back" => back,
            "switch" if k < 16 => fwd | run_bit,
            "switch" => back,
            _ => 0,
        };
        out.push(press(held | pad));
    }
    out
}

/// Sprites as boxes, and the one that is the player's: its top keeps
/// discovery's distance from the player's y byte.
fn player_box(
    nes: &gp_nes::Nes,
    d: &crate::farm::blind::Discovery,
) -> (Option<[i32; 4]>, Vec<[i32; 4]>) {
    let tall = if nes.tall_sprites() { 16 } else { 8 };
    let boxes = crate::mechanic::detect::sprite_boxes(&nes.oam(), tall);
    let y = i64::from(nes.wram()[usize::from(d.y)]);
    let want = y + d.box_dy;
    let p = boxes
        .iter()
        .min_by_key(|b| (i64::from(b[1]) - want).abs())
        .filter(|b| (i64::from(b[1]) - want).abs() <= 8)
        .copied();
    (p, boxes)
}

/// Walks forward from discovery's spot until the player stands with no
/// other sprite within `clear` px ahead or above, and a standing full jump
/// from there lands back on the same ground. Returns the inputs added.
fn open_ground(
    d: &crate::farm::blind::Discovery,
    clear: i32,
) -> Result<(gp_nes::Nes, Vec<InputFrame>)> {
    let mut nes = d.nes.as_nes()?.clone();
    let mut added = Vec::new();
    let fwd = dpad(d.right, true);
    let y_of = |n: &gp_nes::Nes| n.wram()[usize::from(d.y)];
    for step in 0..60 {
        // Stand still a moment, then look.
        let mut still = 0;
        let mut last = y_of(&nes);
        for _ in 0..30 {
            nes.step(press(0))?;
            added.push(press(0));
            let now = y_of(&nes);
            still = if now == last { still + 1 } else { 0 };
            last = now;
        }
        let (p, boxes) = player_box(&nes, d);
        if p.is_none() && std::env::var_os("GP_SWEEP_DEBUG").is_some() {
            eprintln!(
                "step {step}: no player box; y {} + {} vs boxes {boxes:?}",
                y_of(&nes),
                d.box_dy
            );
        }
        if let Some(p) = p
            && still >= 8
        {
            let ahead = |b: &[i32; 4]| {
                let (px0, px1) = (p[0], p[0] + p[2]);
                let near_x = if d.right {
                    b[0] < px1 + clear && b[0] + b[2] > px0 - 8
                } else {
                    b[0] + b[2] > px0 - clear && b[0] < px1 + 8
                };
                // Above or level with the player, within `clear` px.
                near_x && b[1] < p[1] + p[3] && b[1] + b[3] > p[1] - clear && *b != p
            };
            if std::env::var_os("GP_SWEEP_DEBUG").is_some() {
                eprintln!(
                    "step {step}: player {p:?}, still {still}, others ahead {:?}",
                    boxes.iter().filter(|b| ahead(b)).collect::<Vec<_>>()
                );
            }
            // Room behind too, so steering back meets no screen edge.
            let room_behind = if d.right { p[0] } else { 256 - (p[0] + p[2]) };
            if room_behind >= clear + 16 && !boxes.iter().any(ahead) {
                // A full standing jump must come back to this height.
                let mut t = nes.clone();
                let y0 = y_of(&t);
                let mut left = false;
                for k in 0..80 {
                    t.step(press(if k < 32 { d.jump } else { 0 }))?;
                    left |= y_of(&t) != y0;
                }
                if left && y_of(&t) == y0 {
                    return Ok((nes, added));
                }
            }
        }
        // Move on: walk forward a little.
        for _ in 0..(12 + step % 3) {
            nes.step(press(fwd))?;
            added.push(press(fwd));
        }
    }
    bail!("no open flat ground in the first steps forward from play start")
}

/// What a cell did on the original: whether it left the ground, came back
/// to the same height, stopped against something, or met a sprite.
fn context_of(
    base: &gp_nes::Nes,
    d: &crate::farm::blind::Discovery,
    inputs: &[InputFrame],
) -> Result<&'static str> {
    let mut nes = base.clone();
    let y_of = |n: &gp_nes::Nes| i32::from(n.wram()[usize::from(d.y)]);
    let x_of = |n: &gp_nes::Nes| i32::from(n.wram()[usize::from(d.x)]);
    let y0 = y_of(&nes);
    let (mut touched, mut stopped, mut rose) = (false, false, false);
    let mut prev_x = x_of(&nes);
    let mut still_x = 0;
    for f in inputs {
        nes.step(*f)?;
        let (p, boxes) = player_box(&nes, d);
        if let Some(p) = p {
            touched |= boxes.iter().any(|b| {
                *b != p
                    && b[0] < p[0] + p[2]
                    && p[0] < b[0] + b[2]
                    && b[1] < p[1] + p[3]
                    && p[1] < b[1] + b[3]
            });
        }
        let x = x_of(&nes);
        let held_dpad = f.pads[0] & 0xC0 != 0;
        still_x = if held_dpad && x == prev_x {
            still_x + 1
        } else {
            0
        };
        stopped |= still_x >= 4;
        prev_x = x;
        rose |= y_of(&nes) < y0;
    }
    let end = y_of(&nes);
    Ok(if touched {
        "met a sprite"
    } else if stopped {
        "stopped against something"
    } else if !rose {
        "never left the ground"
    } else if end < y0 {
        "landed higher"
    } else if end > y0 {
        "landed lower"
    } else {
        "open"
    })
}

/// A frame of the original the rebuilt game starts from, counted as the
/// whole-game measure counts it (progress_measure.rs playthrough).
fn start_at(
    rom: &[u8],
    power_on: gp_evidence::manifest::PowerOn,
    mapping: &Mapping,
    inputs: &[gp_runtime::InputFrame],
) -> Result<measure::StartHere> {
    let mut o = gp_nes::protocol::NesRuntime::load(rom, power_on).map_err(anyhow::Error::msg)?;
    let mut model_frame = measure::ModelFrame::new(mapping, &o)?;
    for input in inputs {
        let events = o
            .step(std::slice::from_ref(input))
            .map_err(anyhow::Error::msg)?;
        model_frame.step(&o, gp_interp::lockstep::polled(&events));
    }
    Ok(measure::StartHere {
        frame: o.frame(),
        model_frame: model_frame
            .frame
            .context("the spot is before the mapping's start holds")?,
        original: crate::original::OriginalRuntime::Nes(o),
    })
}

fn log_of(frames: &[InputFrame]) -> InputLog {
    let mut l = InputLog::new(1);
    l.frames = frames.to_vec();
    l
}

pub(crate) struct Args<'a> {
    pub game: &'a str,
    pub model: Option<&'a Path>,
    pub rom: Option<&'a Path>,
    pub clips: bool,
    /// A path through a level (an input log from power-on): sweep along it
    /// instead of from one spot.
    pub along: Option<&'a Path>,
    /// The game's built module, for a model with hooks.
    pub module: Option<&'a Path>,
}

/// One cell's score: frames before the first split, frames, and the
/// split's measures; or why it couldn't run.
type CellScore = std::result::Result<(u64, u64, Option<String>), String>;

/// The jumpathon on `game`: make the cells, score each on the model, write
/// the grid.
pub(crate) fn jump(paths: &Paths, a: &Args) -> Result<Outcome> {
    if let Some(p) = a.along {
        return along(paths, a, p);
    }
    let t0 = std::time::Instant::now();
    let (game_dir, rom) = game_and_rom(paths, a.game, a.rom)?;
    let mdir: PathBuf = match a.model {
        Some(m) => m.to_path_buf(),
        None => game_dir.clone(),
    };
    let mapping = Mapping::parse(
        &fs::read_to_string(mdir.join("mapping.toml"))
            .with_context(|| format!("no mapping.toml in {}", mdir.display()))?,
    )
    .map_err(anyhow::Error::msg)?;
    let d = crate::farm::blind::discover(&rom, paths)?;
    let (spot, walk) = open_ground(&d, 48)?;
    let mut base: Vec<InputFrame> = d.log.clone();
    base.extend(walk);
    let start = start_at(
        &rom,
        d.power_on,
        &mapping,
        &crate::run::frames(&log_of(&base)),
    )?;
    let bundle = std::sync::Arc::new(
        crate::hosted::Source::open_dir(&mdir, None, paths, a.module).map(|(s, _)| s)?,
    );

    // The cells.
    struct Cell {
        speed: &'static str,
        hold: usize,
        steer: &'static str,
        inputs: Vec<InputFrame>,
        context: &'static str,
    }
    let mut cells = Vec::new();
    for &speed in SPEEDS {
        for &hold in HOLDS {
            for &steer in STEERS {
                let inputs = cell_inputs(&d, speed, hold, steer);
                let context = context_of(&spot, &d, &inputs)?;
                cells.push(Cell {
                    speed: speed.0,
                    hold,
                    steer,
                    inputs,
                    context,
                });
            }
        }
    }
    // The clips, in the person's clip format, for the page's viewer.
    let session = paths.home.join("mechanics").join(a.game).join("jumpathon");
    if a.clips {
        let clips = session.join("clips");
        if clips.is_dir() {
            fs::remove_dir_all(&clips)?;
        }
        fs::create_dir_all(&clips)?;
        let img = gp_evidence::source::identify_nes(&rom)?;
        fs::write(
            session.join("session.toml"),
            toml::to_string(&toml::toml! {
                game = (a.game)
                mechanic = "jumpathon"
                prompt = "a grid of jumps from open ground, made by gp mechanic sweep jump"
                rom = (img.sha256)
            })?,
        )?;
        for (i, c) in cells.iter().enumerate() {
            let mut all = base.clone();
            all.extend(c.inputs.iter().copied());
            let n = i + 1;
            fs::write(clips.join(format!("{n:03}.log")), log_of(&all).to_text())?;
            fs::write(
                clips.join(format!("{n:03}.toml")),
                toml::to_string(&toml::toml! {
                    by = "sweep"
                    frames = (c.inputs.len() as i64)
                    note = (format!("{} takeoff, A {} frames, steer {}, {}", c.speed, c.hold, c.steer, c.context))
                    start = (base.len() as i64)
                })?,
            )?;
        }
    }

    // Each cell on the model from the spot.
    let scored: Vec<CellScore> = crate::pool::map(
        &cells,
        || (),
        |_, _, c| {
            let inputs = crate::run::frames(&log_of(&c.inputs));
            let ls = measure::lockstep_at(&mdir, &bundle, &mapping, &start, &inputs, Mode::Free)
                .map_err(|e| format!("{e:#}"))?;
            let total = ls.frames.len() as u64;
            let first = ls.splits.first();
            let before = first.map_or(total, |s| s.at as u64);
            let what = first.map(|s| {
                s.paths
                    .iter()
                    .map(|p| gp_interp::lockstep::words(&p.path))
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            Ok((before, total, what))
        },
        &mut |_| {},
    );

    let mut rows = Vec::new();
    let (mut exact, mut open_exact, mut open_n, mut frames_ok, mut frames_all) =
        (0, 0, 0, 0u64, 0u64);
    let mut splits: std::collections::BTreeMap<String, usize> = Default::default();
    for (c, r) in cells.iter().zip(&scored) {
        let (before, total, what, err) = match r {
            Ok((b, t, w)) => (*b, *t, w.clone(), None),
            Err(e) => (0, 0, None, Some(e.clone())),
        };
        let is_exact = err.is_none() && what.is_none();
        exact += usize::from(is_exact);
        if c.context == "open" {
            open_n += 1;
            open_exact += usize::from(is_exact);
        }
        frames_ok += before;
        frames_all += total;
        if let Some(w) = &what {
            *splits.entry(w.clone()).or_default() += 1;
        }
        rows.push(json!({
            "speed": c.speed, "hold": c.hold, "steer": c.steer, "context": c.context,
            "frames_before_split": before, "frames": total, "split": what, "error": err,
        }));
    }
    // The grid: speed by A hold, cells exact of the steering variants.
    let mut grid = String::from("| speed \\ A held |");
    for h in HOLDS {
        grid.push_str(&format!(" {h} |"));
    }
    grid.push_str("\n|---|");
    for _ in HOLDS {
        grid.push_str("---|");
    }
    for (s, ..) in SPEEDS {
        grid.push_str(&format!("\n| {s} |"));
        for h in HOLDS {
            let here: Vec<&Value> = rows
                .iter()
                .filter(|r| r["speed"] == *s && r["hold"] == json!(h))
                .collect();
            let ok = here
                .iter()
                .filter(|r| r["split"].is_null() && r["error"].is_null())
                .count();
            grid.push_str(&format!(" {ok}/{} |", here.len()));
        }
    }
    let mut top: Vec<(String, usize)> = splits.into_iter().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.1));
    let seconds = t0.elapsed().as_secs_f64();
    let out = json!({
        "game": a.game, "model": mdir.display().to_string(),
        "spot_frame": base.len(), "cells": rows.len(), "exact": exact,
        "open_cells": open_n, "open_exact": open_exact,
        "frames_before_split": frames_ok, "frames": frames_all,
        "first_splits": top, "rows": rows, "seconds": seconds,
        "clips": if a.clips { Some(session.join("clips").display().to_string()) } else { None },
    });
    let dir = paths.home.join("work/sweep").join(a.game).join("jump");
    fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let file = dir.join(format!("{stamp}.json"));
    fs::write(&file, serde_json::to_vec_pretty(&out)?)?;
    let mut text = format!(
        "jumpathon on {} against {}: {} of {} cells exact ({} of {} in open air); {} of {} frames before the first split; {:.0} s\n\ncells exact of the {} steering variants:\n{grid}\n\nfirst splits:",
        a.game,
        mdir.display(),
        exact,
        rows.len(),
        open_exact,
        open_n,
        frames_ok,
        frames_all,
        seconds,
        STEERS.len()
    );
    for (w, n) in top.iter().take(8) {
        text.push_str(&format!("\n  {n} cells: {w}"));
    }
    text.push_str(&format!("\n\nwritten to {}", file.display()));
    Ok(Outcome::ok(out, text))
}

// Along a level ---------------------------------------------------------

/// The jumps tried at each column along a path.
const ALONG: &[(&str, usize, bool, bool, usize)] = &[
    // name, A frames, run held, steer forward (else back), frames
    ("stand-full", 32, false, false, 70),
    ("hop", 3, false, false, 40),
    ("run-forward", 24, true, true, 70),
    ("run-back", 24, true, false, 70),
];

fn along_inputs(
    d: &crate::farm::blind::Discovery,
    j: (&str, usize, bool, bool, usize),
) -> Vec<InputFrame> {
    let (name, hold, run, fwd, frames) = j;
    let steer = if name.starts_with("run") {
        dpad(d.right, fwd)
    } else {
        0
    };
    let run_bit = if run { d.run } else { 0 };
    (0..frames)
        .map(|k| press(if k < hold { d.jump } else { 0 } | steer | run_bit))
        .collect()
}

/// The frames in `ys` where a rise turns over at once: rising, then
/// falling 3 px a frame more than the frame before (gravity turns a jump
/// over 1 px a frame at a time). The same rule as `gp mechanic contacts`.
fn head_hits(ys: &[i32]) -> Vec<usize> {
    (1..ys.len().saturating_sub(1))
        .filter(|&i| {
            let (d0, d1) = (ys[i] - ys[i - 1], ys[i + 1] - ys[i]);
            d0 < 0 && d1 >= 0 && d1 - d0 >= 3
        })
        .collect()
}

/// One jump from a column's savestate on the original: the player's y each
/// frame, RAM each frame, sprite box counts, and the code it ran.
struct Played {
    ys: Vec<i32>,
    ram: Vec<Vec<u8>>,
    boxes: Vec<usize>,
    code: gp_nes::coverage::CodeMap,
}

fn play(
    from: &gp_nes::protocol::NesRuntime,
    d: &crate::farm::blind::Discovery,
    inputs: &[InputFrame],
) -> Result<Played> {
    let mut o = from.clone();
    let mut out = Played {
        ys: Vec::new(),
        ram: Vec::new(),
        boxes: Vec::new(),
        code: gp_nes::coverage::CodeMap::new(),
    };
    for f in crate::run::frames(&log_of(inputs)) {
        o.step_covered(&f, &mut out.code)
            .map_err(anyhow::Error::msg)?;
        let n = o.nes();
        out.ys.push(i32::from(n.wram()[usize::from(d.y)]));
        out.ram.push(n.wram()[..0x800].to_vec());
        let tall = if n.tall_sprites() { 16 } else { 8 };
        out.boxes
            .push(crate::mechanic::detect::sprite_boxes(&n.oam(), tall).len());
    }
    Ok(out)
}

/// RAM bytes a head hit changed for good: held still for 8 frames before
/// the hit, changed within 3 frames of it, and still changed at the end.
/// The stack and the sprite page are left out.
fn lasting(p: &Played, hit: usize) -> Vec<u16> {
    let last = p.ram.len() - 1;
    let from = hit.saturating_sub(1).max(8);
    (0..0x800usize)
        .filter(|&a| !(0x100..0x300).contains(&a))
        .filter(|&a| {
            let before = p.ram[from - 1][a];
            (from - 8..from).all(|k| p.ram[k][a] == before)
                && (from..=(hit + 3).min(last)).any(|k| p.ram[k][a] != before)
                && p.ram[last][a] != before
        })
        .map(|a| a as u16)
        .collect()
}

/// Where in a column the standing jumps are tried from, in px from its
/// left edge (16 is the next column's edge).
const OFFSETS: [i64; 5] = [0, 4, 8, 12, 16];

/// Frames the player's y holds still to count as standing: longer than a
/// jump hangs at its top.
const STAND: usize = 8;
/// Probe moves tried in all.
const PROBE_BUDGET: usize = 3000;
/// The probe moves: A frames, run held, steer (1 forward, -1 back, 0 none).
const PROBE_MOVES: &[(usize, bool, i32)] = &[
    (32, true, 1),
    (32, false, 1),
    (32, true, -1),
    (32, false, -1),
    (16, false, 1),
    (16, false, -1),
    (24, true, 1),
    (8, false, 1),
    (32, false, 0),
    (0, false, 1),
    (0, false, -1),
    (24, true, -1),
];

fn probe_inputs(d: &crate::farm::blind::Discovery, m: (usize, bool, i32)) -> Vec<InputFrame> {
    let (hold, run, steer) = m;
    let pad = match steer {
        1 => dpad(d.right, true),
        -1 => dpad(d.right, false),
        _ => 0,
    } | if run { d.run } else { 0 };
    (0..90)
        .map(|k| press(if k < hold { d.jump } else { 0 } | pad))
        .collect()
}

/// The player's place in the level as the game runs: level x from the x
/// byte's moves (and the scroll's, when the byte is a screen position),
/// y, and how long y has held still.
#[derive(Clone, Default)]
struct Track {
    level_x: i64,
    /// The x byte's high byte, when found: level x is then read from the
    /// two bytes, and the scroll only checks it.
    hi: Option<u16>,
    /// Level x as the x byte's and the scroll's moves add up, the check.
    by_moves: i64,
    /// Frames where the two disagree by more than 8 px.
    mismatched: usize,
    xb: u8,
    scroll: i64,
    y: i64,
    ground: usize,
    still_x: usize,
}

impl Track {
    fn col(&self) -> i64 {
        self.level_x.div_euclid(16)
    }

    /// One frame on. `fresh` only takes the values (before play starts).
    fn step(&mut self, n: &gp_nes::Nes, d: &crate::farm::blind::Discovery, fresh: bool) {
        let sc = &n.bus().ppu.scroll;
        let scroll =
            i64::from((sc.t & 31) * 8 + u16::from(sc.fine_x as u8 & 7) + ((sc.t >> 10) & 1) * 256);
        let xb = n.wram()[usize::from(d.x)];
        let y = i64::from(n.wram()[usize::from(d.y)]);
        let fwd = if d.right { 1 } else { -1 };
        let from_bytes = self
            .hi
            .map(|h| (i64::from(n.wram()[usize::from(h)]) * 256 + i64::from(xb)) * fwd);
        if !fresh {
            let mut dx = (i64::from(xb) - i64::from(self.xb) + 384).rem_euclid(256) - 128;
            if dx.abs() > 32 {
                dx = 0;
            }
            let ds = {
                let s = (scroll - self.scroll).rem_euclid(512);
                if s > 256 { s - 512 } else { s }
            };
            let step = dx * fwd + if d.screen_x { ds * fwd } else { 0 };
            self.by_moves += step;
            self.still_x = if dx == 0 { self.still_x + 1 } else { 0 };
            match from_bytes {
                Some(b) => {
                    let moved = b - self.level_x;
                    // A jump of the two bytes (a pipe, a respawn) resets the check.
                    if moved.abs() <= 32 && (moved - step).abs() > 8 {
                        self.mismatched += 1;
                    }
                    if moved.abs() > 32 {
                        self.by_moves = b;
                    }
                    self.level_x = b;
                }
                None => self.level_x += step,
            }
            self.ground = if y == self.y { self.ground + 1 } else { 0 };
        } else if let Some(b) = from_bytes {
            self.level_x = b;
            self.by_moves = b;
        }
        (self.xb, self.scroll, self.y) = (xb, scroll, y);
    }
}

/// The x byte's high byte, read off the path as blind discovery reads it:
/// the byte that steps by one, the same way, each time x wraps, and hardly
/// changes otherwise. None when x doesn't wrap or no byte fits.
fn x_high(
    rom: &[u8],
    power: gp_evidence::manifest::PowerOn,
    frames: &[gp_runtime::InputFrame],
    x: u16,
) -> Result<Option<u16>> {
    let mut o = gp_nes::protocol::NesRuntime::load(rom, power).map_err(anyhow::Error::msg)?;
    let mut prev: Option<Vec<u8>> = None;
    let mut wraps = 0usize;
    let mut fits = vec![0usize; 0x800];
    let mut other = vec![0usize; 0x800];
    for f in frames {
        o.step(std::slice::from_ref(f))
            .map_err(anyhow::Error::msg)?;
        let w = o.nes().wram()[..0x800].to_vec();
        if let Some(p) = &prev {
            let (a, b) = (p[usize::from(x)], w[usize::from(x)]);
            let dx = (i64::from(b) - i64::from(a) + 384).rem_euclid(256) - 128;
            let wrapped = dx.abs() <= 8 && ((dx > 0 && b < a) || (dx < 0 && b > a));
            for k in 0..0x800 {
                let dk = (i64::from(w[k]) - i64::from(p[k]) + 384).rem_euclid(256) - 128;
                if wrapped {
                    if dk == dx.signum() {
                        fits[k] += 1;
                    }
                } else if dk != 0 {
                    other[k] += 1;
                }
            }
            wraps += usize::from(wrapped);
        }
        prev = Some(w);
    }
    if wraps < 2 {
        return Ok(None);
    }
    Ok((0..0x800u16)
        .filter(|&k| k != x && fits[usize::from(k)] == wraps && other[usize::from(k)] <= wraps)
        .min_by_key(|&k| other[usize::from(k)]))
}

/// Walks from `from` to level x `target` (forward or back, no run), then
/// lets go until the player stands still. The buttons, the game there and
/// its track; None if the player can't stop within 3 px of it.
fn walk_to(
    from: &gp_nes::protocol::NesRuntime,
    track: &Track,
    d: &crate::farm::blind::Discovery,
    target: i64,
) -> Result<Option<(Vec<InputFrame>, gp_nes::protocol::NesRuntime, Track)>> {
    let mut o = from.clone();
    let mut t = track.clone();
    let mut inputs = Vec::new();
    let mut step = |o: &mut gp_nes::protocol::NesRuntime, t: &mut Track, pad: u8| -> Result<()> {
        let f = press(pad);
        o.step(&crate::run::frames(&log_of(std::slice::from_ref(&f))))
            .map_err(anyhow::Error::msg)?;
        t.step(o.nes(), d, false);
        inputs.push(f);
        Ok(())
    };
    // Far off: hold the d-pad. Near: tap it, a frame at a time with a
    // rest between, so the player's slide doesn't carry it past.
    for k in 0..240 {
        let gap = target - t.level_x;
        let stopped = t.still_x >= 4 && t.ground >= 4;
        if gap.abs() <= 1 && stopped {
            break;
        }
        let pad = dpad(d.right, gap > 0);
        let press_now = gap.abs() > 12 || (gap != 0 && stopped && k % 4 == 0);
        step(&mut o, &mut t, if press_now { pad } else { 0 })?;
    }
    for _ in 0..40 {
        if t.ground >= 4 && t.still_x >= 4 {
            break;
        }
        step(&mut o, &mut t, 0)?;
    }
    let ok = (t.level_x - target).abs() <= 3 && t.ground >= 4;
    Ok(ok.then_some((inputs, o, t)))
}

/// A place the player stood: its column and height, a copy of the game
/// there, the model's frame count, and how many probe moves it took to
/// get there (0 on the path).
struct Entry {
    col: i64,
    y: i64,
    frame: usize,
    /// The path frame the place was reached from, and the buttons from
    /// there (empty for a place on the path).
    path_frame: usize,
    chain: Vec<InputFrame>,
    at: gp_nes::protocol::NesRuntime,
    model_frame: Option<u64>,
    track: Track,
    depth: usize,
    tries: usize,
}

/// The model's frame after one more frame, counted as the whole-game
/// measure counts it.
fn next_model_frame(
    mf: Option<u64>,
    polled: bool,
    o: &gp_nes::protocol::NesRuntime,
    start: Option<&gp_runtime::select::Condition>,
) -> Option<u64> {
    match (mf, start) {
        (Some(m), _) if polled => Some(m + 1),
        (Some(m), _) => Some(m),
        (None, Some(c)) if measure::holds(o, c) => Some(0),
        (None, None) if polled => Some(1),
        (None, _) => None,
    }
}

fn along(paths: &Paths, a: &Args, path: &Path) -> Result<Outcome> {
    let t0 = std::time::Instant::now();
    let (_, rom) = game_and_rom(paths, a.game, a.rom)?;
    let path_text = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let path_sha = crate::template::sha256_hex(&path_text);
    let log = InputLog::parse(
        &fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    // A clip's start, from the toml beside it (NNN.log -> NNN.toml).
    let start = fs::read_to_string(path.with_extension("toml"))
        .ok()
        .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
        .and_then(|t| t.get("start").and_then(|v| v.as_integer()))
        .unwrap_or(0) as usize;
    let d = crate::farm::blind::discover(&rom, paths)?;
    let model = match a.model {
        Some(m) => {
            let mapping = Mapping::parse(
                &fs::read_to_string(m.join("mapping.toml"))
                    .with_context(|| format!("no mapping.toml in {}", m.display()))?,
            )
            .map_err(anyhow::Error::msg)?;
            let bundle = std::sync::Arc::new(
                crate::hosted::Source::open_dir(m, None, paths, a.module).map(|(s, _)| s)?,
            );
            Some((m.to_path_buf(), mapping, bundle))
        }
        None => None,
    };
    let start_cond = model
        .as_ref()
        .and_then(|(_, mp, _)| mp.start.as_deref())
        .map(gp_runtime::select::Condition::parse)
        .transpose()
        .map_err(|e| anyhow::anyhow!("the mapping's start: {e}"))?;

    // One pass along the path: the player's level x (the x byte's moves,
    // plus the scroll's when the byte is a screen position), and a copy of
    // the game at each place the player first stands: each 16 px column
    // and each height in it.
    let mut o = gp_nes::protocol::NesRuntime::load(&rom, d.power_on).map_err(anyhow::Error::msg)?;
    let mut seen = gp_nes::coverage::CodeMap::new();
    let mut model_frame: Option<u64> = None;
    let frames = crate::run::frames(&log);
    let x_hi = x_high(&rom, d.power_on, &frames, d.x)?;
    let mut track = Track {
        hi: x_hi,
        ..Track::default()
    };
    let mut archive: Vec<Entry> = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let polled = o.step_covered(f, &mut seen).map_err(anyhow::Error::msg)?;
        model_frame = next_model_frame(model_frame, polled, &o, start_cond.as_ref());
        let fresh = i + 1 < start;
        track.step(o.nes(), &d, fresh);
        if fresh {
            continue;
        }
        if track.ground >= STAND {
            let key = (track.col(), track.y);
            if !archive.iter().any(|e| (e.col, e.y) == key) {
                archive.push(Entry {
                    col: key.0,
                    y: key.1,
                    frame: i + 1,
                    path_frame: i + 1,
                    chain: Vec::new(),
                    at: o.clone(),
                    model_frame,
                    track: track.clone(),
                    depth: 0,
                    tries: 0,
                });
            }
        }
    }
    let on_path = archive.len();
    if archive
        .iter()
        .map(|e| e.col)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        < 2
    {
        bail!(
            "the path never moves the player across: wrong power-on, or not a path through a level"
        );
    }
    // Probes: from places already stood on, the moves below, keeping each
    // new place the player comes to stand (a column and a height). New
    // places are probed from in turn, so a high ledge reached only from a
    // lower one in another column is found by a chain.
    let cols_on_path: std::collections::BTreeSet<i64> = archive.iter().map(|e| e.col).collect();
    let (lo, hi) = (
        *cols_on_path.first().unwrap_or(&0),
        *cols_on_path.last().unwrap_or(&0),
    );
    let mut moves_tried = 0usize;
    while moves_tried < PROBE_BUDGET {
        // The least-tried place, nearest the path's start first.
        let Some(k) = (0..archive.len())
            .filter(|&k| archive[k].tries < PROBE_MOVES.len())
            .min_by_key(|&k| (archive[k].tries, archive[k].depth, archive[k].col))
        else {
            break;
        };
        let m = PROBE_MOVES[archive[k].tries];
        archive[k].tries += 1;
        moves_tried += 1;
        let (from_at, from_track, from_mf, from_depth, from_frame) = (
            archive[k].at.clone(),
            archive[k].track.clone(),
            archive[k].model_frame,
            archive[k].depth,
            archive[k].frame,
        );
        let (from_path_frame, from_chain) = (archive[k].path_frame, archive[k].chain.clone());
        let move_inputs = probe_inputs(&d, m);
        let mut o = from_at;
        let mut t = from_track;
        let mut mf = from_mf;
        let mut left = false;
        for (n, f) in move_inputs.iter().enumerate() {
            let rf = &crate::run::frames(&log_of(std::slice::from_ref(f)))[0];
            let events = o
                .step(std::slice::from_ref(rf))
                .map_err(anyhow::Error::msg)?;
            mf = next_model_frame(
                mf,
                gp_interp::lockstep::polled(&events),
                &o,
                start_cond.as_ref(),
            );
            t.step(o.nes(), &d, false);
            left |= t.ground == 0;
            if t.y > 232 {
                break; // fell off the bottom
            }
            if left && t.ground >= STAND {
                let key = (t.col(), t.y);
                if key.0 >= lo - 2
                    && key.0 <= hi + 2
                    && !archive.iter().any(|e| (e.col, e.y) == key)
                {
                    let mut chain = from_chain.clone();
                    chain.extend_from_slice(&move_inputs[..=n]);
                    archive.push(Entry {
                        col: key.0,
                        y: key.1,
                        frame: from_frame + n + 1,
                        path_frame: from_path_frame,
                        chain,
                        at: o.clone(),
                        model_frame: mf,
                        track: t.clone(),
                        depth: from_depth + 1,
                        tries: 0,
                    });
                }
                break;
            }
        }
    }
    archive.sort_by_key(|e| (e.col, -e.y));

    // The jumps at each place, on the original (and on the model).
    let mut strip = Vec::new();
    let mut new_code_total = 0usize;
    let mut jumps = 0usize;
    let (mut ok_frames, mut all_frames, mut exact) = (0u64, 0u64, 0usize);
    for c in &archive {
        let idle = play(&c.at, &d, &vec![press(0); 70])?;
        let idle_boxes = idle.boxes.iter().copied().max().unwrap_or(0);
        let mut row = Vec::new();
        // The standing jumps from several places across the column (and
        // the next column's edge), so a block over one part isn't missed;
        // the running jumps from where the player stood.
        let mut tries: Vec<(String, Vec<InputFrame>)> = Vec::new();
        for &j in ALONG {
            if j.0.starts_with("run") {
                tries.push((j.0.to_owned(), along_inputs(&d, j)));
                continue;
            }
            for off in OFFSETS {
                let target = c.col * 16 + off;
                if let Some((mut walk, _, _)) = walk_to(&c.at, &c.track, &d, target)? {
                    walk.extend(along_inputs(&d, j));
                    tries.push((format!("{}@+{off}", j.0), walk));
                }
            }
        }
        for (name, inputs) in &tries {
            let inputs = inputs.clone();
            let p = play(&c.at, &d, &inputs)?;
            jumps += 1;
            let hits = head_hits(&p.ys);
            let kept: Vec<String> = hits
                .iter()
                .flat_map(|&h| lasting(&p, h))
                .map(|a| format!("${a:04X}"))
                .collect();
            let new_code = p.code.new_over(&seen);
            let first_new: Vec<String> = (0..0x1_0000u32)
                .filter(|&id| p.code.get(id) && !seen.get(id))
                .take(4)
                .map(|id| format!("${id:04X}"))
                .collect();
            seen.union(&p.code);
            new_code_total += new_code;
            let more_boxes = p
                .boxes
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
                .saturating_sub(idle_boxes);
            let mut v = json!({
                "jump": name, "inputs": log_of(&inputs).to_text(),
                "head_hits": hits.len(), "lasting": kept,
                "new_code": new_code, "first_new_code": first_new,
                "more_sprites": more_boxes,
            });
            if let (Some((mdir, mapping, bundle)), Some(mf)) = (&model, c.model_frame) {
                let start = measure::StartHere {
                    frame: c.frame as u64,
                    model_frame: mf,
                    original: crate::original::OriginalRuntime::Nes(c.at.clone()),
                };
                match measure::lockstep_at(
                    mdir,
                    bundle,
                    mapping,
                    &start,
                    &crate::run::frames(&log_of(&inputs)),
                    Mode::Free,
                ) {
                    Ok(ls) => {
                        let total = ls.frames.len() as u64;
                        let first = ls.splits.first();
                        let before = first.map_or(total, |s| s.at as u64);
                        ok_frames += before;
                        all_frames += total;
                        exact += usize::from(first.is_none());
                        v["frames"] = json!(total);
                        v["frames_before_split"] = json!(before);
                        v["split"] = json!(first.map(|s| {
                            s.paths
                                .iter()
                                .map(|p| gp_interp::lockstep::words(&p.path))
                                .collect::<Vec<_>>()
                                .join(", ")
                        }));
                    }
                    Err(e) => v["error"] = json!(format!("{e:#}")),
                }
            }
            row.push(v);
        }
        strip.push(json!({
            "column": c.col, "level_x": c.col * 16, "y": c.y, "frame": c.frame,
            "path_frame": c.path_frame, "chain": log_of(&c.chain).to_text(),
            "reached_by": if c.depth == 0 { "path".to_owned() } else { format!("probe, chain {}", c.depth) },
            "jumps": row,
        }));
    }
    let mismatched = track.mismatched;
    let mut per_col: std::collections::BTreeMap<i64, usize> = Default::default();
    for e in &archive {
        *per_col.entry(e.col).or_default() += 1;
    }
    let mut heights_hist: std::collections::BTreeMap<usize, usize> = Default::default();
    for n in per_col.values() {
        *heights_hist.entry(*n).or_default() += 1;
    }
    let by_probe = archive.len() - on_path;
    let deepest = archive.iter().map(|e| e.depth).max().unwrap_or(0);
    let cols = per_col.len();
    let seconds = t0.elapsed().as_secs_f64();
    let notable: Vec<&Value> = strip
        .iter()
        .filter(|c| {
            c["jumps"].as_array().is_some_and(|js| {
                js.iter().any(|j| {
                    j["new_code"].as_u64().unwrap_or(0) > 0
                        || j["more_sprites"].as_u64().unwrap_or(0) > 0
                        || !j["lasting"].as_array().is_none_or(Vec::is_empty)
                })
            })
        })
        .collect();
    let mut text = format!(
        "jumps along {}: {} columns, {} places to stand ({} on the path, {} reached by probe, deepest chain {}, {} probe moves), {} jumps, {} code addresses run for the first time, {:.0} s\nplaces per column: {}",
        path.display(),
        cols,
        archive.len(),
        on_path,
        by_probe,
        deepest,
        moves_tried,
        jumps,
        new_code_total,
        seconds,
        heights_hist
            .iter()
            .map(|(h, n)| format!("{n} columns with {h}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if model.is_some() {
        text.push_str(&format!(
            "\non the model: {exact} of {jumps} jumps exact, {ok_frames} of {all_frames} frames before the first split"
        ));
    }
    text.push_str(&format!(
        "\nlevel x from the x byte and {} ({} path frames where the moves' count disagrees by over 8 px)",
        x_hi.map_or("the scroll's moves (no high byte found)".to_owned(), |h| format!("its high byte ${h:04X}")),
        mismatched
    ));
    text.push_str(
        "\n\ncolumns where a jump ran new code, brought a sprite, or a head hit kept a change:",
    );
    for c in &notable {
        let js: Vec<String> = c["jumps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|j| {
                j["new_code"].as_u64().unwrap_or(0) > 0
                    || j["more_sprites"].as_u64().unwrap_or(0) > 0
                    || !j["lasting"].as_array().is_none_or(Vec::is_empty)
            })
            .map(|j| {
                format!(
                    "{} (new code {}, +{} sprites, {} head hits, kept {})",
                    j["jump"].as_str().unwrap_or(""),
                    j["new_code"],
                    j["more_sprites"],
                    j["head_hits"],
                    j["lasting"].as_array().map_or(0, Vec::len)
                )
            })
            .collect();
        text.push_str(&format!(
            "\n  x {} y {} ({}): {}",
            c["level_x"],
            c["y"],
            c["reached_by"].as_str().unwrap_or(""),
            js.join("; ")
        ));
    }
    let out = json!({
        "game": a.game, "along": path.display().to_string(), "columns": cols,
        "path_sha256": path_sha,
        "places": archive.len(), "on_path": on_path, "by_probe": by_probe,
        "deepest_chain": deepest, "probe_moves": moves_tried,
        "x_high": x_hi.map(|h| format!("${h:04X}")), "level_x_mismatched_frames": mismatched,
        "places_per_column": heights_hist,
        "jumps": jumps, "new_code": new_code_total, "seconds": seconds,
        "model": a.model.map(|m| m.display().to_string()),
        "exact": exact, "frames_before_split": ok_frames, "frames": all_frames,
        "strip": strip,
    });
    let dir = paths
        .home
        .join("work/sweep")
        .join(a.game)
        .join("jump-along");
    fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let file = dir.join(format!("{stamp}.json"));
    fs::write(&file, serde_json::to_vec_pretty(&out)?)?;
    text.push_str(&format!("\n\nwritten to {}", file.display()));
    Ok(Outcome::ok(out, text))
}

/// One replayed run for the jump fit: each frame's RAM and controller 1,
/// from `keep` frames before `from` to the end.
fn trace_of(rom: &[u8], log: &InputLog, from: usize) -> Result<(Vec<Vec<u8>>, Vec<u8>)> {
    let mut nes = gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
    let keep = from.saturating_sub(8);
    let (mut ram, mut pads) = (Vec::new(), Vec::new());
    for (i, f) in log.frames.iter().enumerate() {
        nes.step(*f)?;
        if i >= keep {
            ram.push(nes.wram()[..0x800].to_vec());
            pads.push(f.pads[0]);
        }
    }
    Ok((ram, pads))
}

/// The player's vertical speed byte, blind: in the air, the byte whose
/// signed value is y's change that frame (or the next, for a game that adds
/// gravity first), to within the fraction's pixel.
fn speed_byte(traces: &[(Vec<Vec<u8>>, Vec<u8>)], yb: usize) -> Option<usize> {
    let s = |b: u8| i32::from(b as i8);
    let mut best: Option<(usize, usize)> = None;
    for b in 0..0x800usize {
        if b == yb {
            continue;
        }
        let (mut hit, mut n) = (0usize, 0usize);
        for (ram, _) in traces {
            for i in 1..ram.len().saturating_sub(1) {
                let dy = i32::from(ram[i + 1][yb]) - i32::from(ram[i][yb]);
                if dy == 0 || dy.abs() > 8 {
                    continue;
                }
                n += 1;
                let v0 = s(ram[i][b]);
                let v1 = s(ram[i + 1][b]);
                if (dy - v0).abs() <= 1 || (dy - v1).abs() <= 1 {
                    hit += 1;
                }
            }
        }
        if n >= 200 && hit * 10 >= n * 9 && best.is_none_or(|(h, _)| hit > h) {
            best = Some((hit, b));
        }
    }
    best.map(|(_, b)| b)
}

/// Fits the jump blind and writes a scratch model with only its jump facts
/// replaced (`gp mechanic sweep jump GAME --fit-blind OUT`).
pub(crate) fn fit_blind(
    paths: &Paths,
    game: &str,
    model: Option<&Path>,
    rom: Option<&Path>,
    out: &Path,
) -> Result<Outcome> {
    let t0 = std::time::Instant::now();
    let (game_dir, rom) = game_and_rom(paths, game, rom)?;
    let d = crate::farm::blind::discover(&rom, paths)?;
    let (xb, yb) = (usize::from(d.x), usize::from(d.y));

    // The runs: the game's jumpathon clips, its farm clips and its blind
    // paths. Never a recording: those are the held-out walkathon's kind.
    let mut logs: Vec<(String, InputLog, usize)> = Vec::new();
    let mdir = paths.home.join("mechanics").join(game);
    for session in ["jumpathon", "farm-1-1"] {
        let dir = mdir.join(session);
        let Ok(rd) = fs::read_dir(dir.join("clips")) else {
            continue;
        };
        let mut ns: Vec<u64> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.strip_suffix(".toml").and_then(|s| s.parse().ok())
            })
            .collect();
        ns.sort_unstable();
        for n in ns {
            let (text, start) = crate::mechanic::paths::full_log(&paths.home, &dir, n)?;
            let log = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("{session} {n}: {e}"))?;
            logs.push((format!("{session}/{n}"), log, start));
        }
    }
    let blind = paths.home.join("work/farm/blind").join(game);
    if let Ok(latest) = fs::read_to_string(blind.join("latest")) {
        let p = Path::new(latest.trim()).join("path.log");
        if let Ok(text) = fs::read_to_string(&p) {
            let log =
                InputLog::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?;
            logs.push(("blind path".into(), log, 0));
        }
    }
    if logs.is_empty() {
        bail!(
            "no jumpathon, farm or blind runs for {game}: run gp mechanic sweep jump and gp farm first"
        );
    }
    let mut traces = Vec::new();
    for (_, log, start) in &logs {
        traces.push(trace_of(&rom, log, *start)?);
    }
    let vyb = speed_byte(&traces, yb).context("no vertical speed byte found blind")?;
    // A third held out: every third run.
    let (mut train, mut test) = (Vec::new(), Vec::new());
    // Each jump's run speed at takeoff (x byte change over the 4 frames
    // before it left the ground, px a frame) with its first air speed's
    // whole byte and the RAM there, to order the takeoff classes by run
    // speed once the fit has found the speed's fraction byte.
    let mut takeoffs: Vec<(Vec<u8>, f64)> = Vec::new();
    for (i, t) in traces.into_iter().enumerate() {
        let (ram, pads) = &t;
        for k in 6..ram.len().saturating_sub(1) {
            let y = |j: usize| ram[j][yb];
            if y(k) == y(k - 1) && y(k - 1) == y(k - 2) && y(k + 1) < y(k) && pads[k + 1] & 1 != 0 {
                let dx =
                    (i32::from(ram[k][xb]) - i32::from(ram[k - 4][xb]) + 384).rem_euclid(256) - 128;
                takeoffs.push((ram[k + 1].clone(), f64::from(dx.abs()) / 4.0));
            }
        }
        if i % 3 == 2 {
            test.push(t)
        } else {
            train.push(t)
        }
    }
    let fit = crate::rule_refine::fit_and_test(train, test, yb, vyb);

    // Copy the model and replace the jump facts it can express.
    let src = model
        .map(Path::to_path_buf)
        .unwrap_or_else(|| game_dir.clone());
    fs::create_dir_all(out)?;
    for e in fs::read_dir(&src)? {
        let e = e?;
        if e.file_type()?.is_file() {
            fs::copy(e.path(), out.join(e.file_name()))?;
        }
    }
    // The first air speed in 1/256 px, with the fraction byte the fit found.
    let vf = fit["speed_fraction"].as_u64().map(|v| v as usize);
    let takeoff_x: Vec<(i32, f64)> = takeoffs
        .iter()
        .map(|(r, xs)| {
            let whole = i32::from(r[vyb] as i8) * 256;
            (whole + vf.map_or(0, |f| i32::from(r[f])), *xs)
        })
        .collect();
    let notes = rewrite_jump(&out.join("model.toml"), &fit, &takeoff_x)?;
    let res = json!({
        "game": game, "x": format!("${xb:04X}"), "y": format!("${yb:04X}"), "vy": format!("${vyb:04X}"),
        "runs": logs.iter().map(|l| l.0.clone()).collect::<Vec<_>>(),
        "fit": fit, "model": out.display().to_string(), "notes": notes,
        "seconds": t0.elapsed().as_secs_f64(),
    });
    fs::write(
        out.join("blind-jump-fit.json"),
        serde_json::to_string_pretty(&res)?,
    )?;
    let text = format!(
        "blind jump fit on {} runs: vy ${vyb:04X}; fitted {} of {} frames, held out {} of {}; model written to {}\n{}",
        logs.len(),
        res["fit"]["fitted"]["matched"],
        res["fit"]["fitted"]["total"],
        res["fit"]["held_out"]["matched"],
        res["fit"]["held_out"]["total"],
        out.display(),
        notes.join("\n")
    );
    Ok(Outcome::ok(res, text))
}

/// Puts the fitted jump into the player's `jump` table and its `bands` in
/// the model's units: speed is the first air speed less one held-gravity
/// step (the model moves first), gravities in px a frame per frame. Takeoff
/// classes go to bands by run speed at takeoff: each band takes the class
/// whose median run speed falls in its range. The bands' run-speed bounds
/// aren't measured blind, so they stay as the model had them; so do the
/// form flags (latch, fall cap from half). Returns what was replaced and
/// what was kept.
fn rewrite_jump(model: &Path, fit: &Value, takeoff_x: &[(i32, f64)]) -> Result<Vec<String>> {
    let text = fs::read_to_string(model)?;
    let mut doc: toml::Table = toml::from_str(&text)?;
    let mut notes = Vec::new();
    let (Some(takeoff), Some(gravity)) = (fit["takeoff"].as_object(), fit["gravity"].as_object())
    else {
        bail!("the fit found no takeoff or gravity");
    };
    // Classes: (first air speed in 1/256, held gravity, other gravity,
    // median run speed, takeoffs seen).
    type Class = (i32, Option<i64>, Option<i64>, f64, usize);
    let mut classes: Vec<Class> = Vec::new();
    for (name, v) in takeoff {
        let Some(v0) = v.as_i64() else { continue };
        let held = gravity
            .get(&format!("{name}, A held while rising"))
            .and_then(Value::as_i64);
        let other = gravity
            .get(&format!("{name}, otherwise"))
            .and_then(Value::as_i64);
        // The class's takeoffs (exact first air speed) and their run speeds.
        let mut xs: Vec<f64> = takeoff_x
            .iter()
            .filter(|(w, _)| i64::from(*w) == v0)
            .map(|t| t.1)
            .collect();
        xs.sort_by(|a, b| a.total_cmp(b));
        let med = xs.get(xs.len() / 2).copied().unwrap_or(f64::NAN);
        notes.push(format!("class {v0}/256: {} takeoffs, median run speed {med:.2} px a frame, held gravity {held:?}, other {other:?}", xs.len()));
        classes.push((v0 as i32, held, other, med, xs.len()));
    }
    let px = |v: i64| toml::Value::String(format!("{}", v as f64 / 256.0));
    let speed_of = |c: &(i32, Option<i64>, Option<i64>, f64)| -> Option<i64> {
        c.1.map(|h| i64::from(c.0) - h)
            .filter(|s| s % 256 == 0)
            .map(|s| s / 256)
    };
    // Find the player's entity: the one whose jump table has an action.
    let ents = doc
        .get_mut("entities")
        .and_then(toml::Value::as_table_mut)
        .context("no entities")?;
    let name = ents
        .iter()
        .find(|(_, e)| e.get("jump").and_then(|j| j.get("action")).is_some())
        .map(|(n, _)| n.clone())
        .context("no entity with a jump action")?;
    let j = ents
        .get_mut(&name)
        .and_then(|e| e.get_mut("jump"))
        .and_then(toml::Value::as_table_mut)
        .context("no jump table")?;
    let set = |t: &mut toml::Table,
               c: &(i32, Option<i64>, Option<i64>, f64),
               notes: &mut Vec<String>,
               at: &str| {
        if let Some(s) = speed_of(c) {
            notes.push(format!(
                "{at}: speed {} -> {s}",
                t.get("speed").map(|v| v.to_string()).unwrap_or_default()
            ));
            t.insert("speed".into(), toml::Value::Integer(s));
        } else {
            notes.push(format!(
                "{at}: speed kept (the fitted first speed less held gravity isn't whole pixels)"
            ));
        }
        if let Some(h) = c.1 {
            notes.push(format!(
                "{at}: gravity_held {} -> {}",
                t.get("gravity_held")
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                px(h)
            ));
            t.insert("gravity_held".into(), px(h));
        }
        if let Some(g) = c.2 {
            notes.push(format!(
                "{at}: gravity {} -> {}",
                t.get("gravity").map(|v| v.to_string()).unwrap_or_default(),
                px(g)
            ));
            t.insert("gravity".into(), px(g));
        }
    };
    // Bands in order of their run-speed bound; the table itself is the band
    // above the last bound.
    let bounds: Vec<f64> = j
        .get("bands")
        .and_then(toml::Value::as_array)
        .map(|b| {
            b.iter()
                .map(|x| match x.get("below") {
                    Some(toml::Value::String(s)) => s.parse().unwrap_or(f64::NAN),
                    Some(toml::Value::Integer(i)) => *i as f64,
                    Some(toml::Value::Float(f)) => *f,
                    _ => f64::NAN,
                })
                .collect()
        })
        .unwrap_or_default();
    // A band takes the class with the most takeoffs whose median run speed
    // is in its range (a class seen once, such as a bounce, never wins
    // against a real one).
    let pick = |lo: f64, hi: f64| {
        classes
            .iter()
            .filter(|c| c.3 >= lo && c.3 < hi && c.1.is_some() && c.2.is_some())
            .max_by_key(|c| c.4)
            .map(|c| (c.0, c.1, c.2, c.3))
    };
    let mut lo = f64::NEG_INFINITY;
    if let Some(bands) = j.get_mut("bands").and_then(toml::Value::as_array_mut) {
        for (k, b) in bands.iter_mut().enumerate() {
            let hi = bounds[k];
            match (pick(lo, hi), b.as_table_mut()) {
                (Some(c), Some(t)) => set(t, &c, &mut notes, &format!("band below {hi}")),
                _ => notes.push(format!("band below {hi}: kept (no fitted takeoff class has its median run speed in [{lo}, {hi}))")),
            }
            lo = hi;
        }
    }
    match pick(lo, f64::INFINITY) {
        Some(c) => set(j, &c, &mut notes, "top band"),
        None => notes.push("top band: kept (no fitted class at that run speed)".into()),
    }
    if let (Some(to), Some(above)) = (fit["cap_to"].as_i64(), fit["cap_above"].as_i64()) {
        if to % 256 == 0 {
            notes.push(format!(
                "max_fall {} -> {} (falls at or above {}/256 drop back to it)",
                j.get("max_fall").map(|v| v.to_string()).unwrap_or_default(),
                to / 256,
                above
            ));
            j.insert("max_fall".into(), toml::Value::Integer(to / 256));
        } else {
            notes.push("max_fall kept: the fitted cap isn't whole pixels".into());
        }
    }
    notes.push("kept, not measured blind: the bands' run-speed bounds, the release latch and fall-cap form flags (keep_fall_gravity, max_fall_from_half, start_fall_gravity, move_first)".into());
    fs::write(model, toml::to_string(&doc)?)?;
    Ok(notes)
}

/// A clip of one jump of an along-sweep, rebuilt from its strip: the path
/// file's sha256 (checked against the file the strip names), the frame the
/// place is reached at (where the jump's buttons start), and the inputs
/// from power-on: the path up to the place's path frame, the probe chain,
/// then the jump's own buttons (walking into the column, then the jump).
pub(crate) fn place_clip(
    strip_file: &Path,
    column: i64,
    y: i64,
    jump: &str,
) -> Result<(String, usize, Vec<InputFrame>)> {
    let strip: Value = serde_json::from_slice(
        &fs::read(strip_file).with_context(|| format!("reading {}", strip_file.display()))?,
    )?;
    let sha = strip["path_sha256"]
        .as_str()
        .context("the strip has no path_sha256: made before places kept their inputs")?
        .to_owned();
    let along = Path::new(strip["along"].as_str().context("the strip names no path")?);
    let text = fs::read(along).with_context(|| format!("reading the path {}", along.display()))?;
    let found = crate::template::sha256_hex(&text);
    if found != sha {
        bail!(
            "{} changed since the sweep (sha256 {found}, the strip says {sha})",
            along.display()
        );
    }
    let place = strip["strip"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["column"] == json!(column) && p["y"] == json!(y))
        .with_context(|| format!("no place at column {column}, y {y} in the strip"))?;
    let j = place["jumps"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|j| j["jump"] == json!(jump))
        .with_context(|| format!("no jump {jump} at column {column}, y {y}"))?;
    let parse = |t: &str, what: &str| {
        InputLog::parse(t).map_err(|e| anyhow::anyhow!("the strip's {what}: {e}"))
    };
    let path_frame = place["path_frame"]
        .as_u64()
        .context("the place has no path_frame")? as usize;
    let path = InputLog::parse(&String::from_utf8_lossy(&text))
        .map_err(|e| anyhow::anyhow!("{}: {e}", along.display()))?;
    if path_frame > path.frames.len() {
        bail!("the place's path frame {path_frame} is past the path's end");
    }
    let mut frames: Vec<InputFrame> = path.frames[..path_frame].to_vec();
    frames.extend(parse(place["chain"].as_str().unwrap_or(""), "chain")?.frames);
    let start = frames.len();
    frames.extend(
        parse(
            j["inputs"].as_str().context("the jump has no inputs")?,
            "jump inputs",
        )?
        .frames,
    );
    Ok((sha, start, frames))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_clip_is_the_path_then_the_chain_then_the_jump() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("path.log");
        let frames: Vec<InputFrame> = (0..10).map(|k| press(k as u8)).collect();
        fs::write(&path, log_of(&frames).to_text()).unwrap();
        let sha = crate::template::sha256_hex(&fs::read(&path).unwrap());
        let strip = json!({
            "along": path.display().to_string(), "path_sha256": sha,
            "strip": [{"column": 3, "y": 176, "path_frame": 4,
                       "chain": log_of(&[press(0x80), press(0x80)]).to_text(),
                       "jumps": [{"jump": "hop@+4", "inputs": log_of(&[press(1), press(0)]).to_text()}]}],
        });
        let sf = d.path().join("strip.json");
        fs::write(&sf, strip.to_string()).unwrap();
        let (got, start, all) = place_clip(&sf, 3, 176, "hop@+4").unwrap();
        assert_eq!(got, sha);
        assert_eq!(start, 6);
        let pads: Vec<u8> = all.iter().map(|f| f.pads[0]).collect();
        assert_eq!(pads, vec![0, 1, 2, 3, 0x80, 0x80, 1, 0]);
        fs::write(&path, "gp-input-log 1\ncontrollers 1\n").unwrap();
        assert!(place_clip(&sf, 3, 176, "hop@+4").is_err());
    }
}

/// The game's package folder in the games checkout and its ROM (`rom`, or
/// the one its roms.toml names in $GP_ROM_DIR). Refuses an arcade game:
/// the sweep plays gp-nes.
fn game_and_rom(paths: &Paths, game: &str, rom: Option<&Path>) -> Result<(PathBuf, Vec<u8>)> {
    let games =
        crate::roms_cmd::games_dir(None).context("no games checkout found: set GP_GAMES_DIR")?;
    let dir = crate::labels::game_folder(&games, game)
        .with_context(|| format!("no game {game} in {}", games.display()))?;
    let rom = match rom {
        Some(r) => fs::read(r).with_context(|| format!("reading {}", r.display()))?,
        None => crate::clip_score::rom_of(paths, &dir, None)?,
    };
    if crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade {
        bail!(
            "gp mechanic sweep jump is a grid of jumps (run-up speed, how long the jump button is held, steering in the air) from a spot blind discovery finds; blind discovery (crate::farm::blind) runs on gp-nes and learns the jump as the button that sends the player's y up and back down, so it has no arcade form yet, and {game} is an arcade game. An arcade sweep needs blind discovery on OriginalRuntime, and a game with a jump button"
        );
    }
    Ok((dir, rom))
}

/// What the sweep plays of game-printer's whole-game measure
/// (progress_measure.rs): a frame the rebuilt game can start from, a
/// condition on the original, and the lockstep from a start.
pub(crate) mod measure {
    use std::path::Path;

    use anyhow::Result;
    use gp_interp::lockstep::Mode;
    use gp_interp::mapping::Mapping;
    use gp_runtime::Runtime;
    use gp_runtime::select::Condition;

    use crate::hosted::Source;

    /// A frame of the original the rebuilt game can start from: in play,
    /// the frame before a polled one (so the lockstep's next frame is
    /// compared), with a copy of the original there, on either platform
    /// (`gp progress --measure` plays an arcade romset's levels from it).
    #[derive(Clone)]
    pub(crate) struct StartHere {
        pub frame: u64,
        /// The model's frame there, as the lockstep counts it: polled
        /// frames since the mapping's start.
        pub model_frame: u64,
        pub original: crate::original::OriginalRuntime,
    }

    /// The model's frame at the original's, as the lockstep counts it: 0
    /// where the mapping's start first holds (with no start, 1 on the first
    /// polled frame), then one more each polled frame.
    pub(crate) struct ModelFrame {
        start: Option<Condition>,
        pub frame: Option<u64>,
    }

    impl ModelFrame {
        /// With the original `o` at its first frame.
        pub(crate) fn new<R: Runtime>(mapping: &Mapping, o: &R) -> Result<Self> {
            let start = gp_interp::lockstep::start_of(mapping).map_err(anyhow::Error::msg)?;
            let frame = match &start {
                Some(c) if holds(o, c) => Some(0),
                _ => None,
            };
            Ok(Self { start, frame })
        }

        /// After the original `o` ran a frame, `polled` or not.
        pub(crate) fn step<R: Runtime>(&mut self, o: &R, polled: bool) {
            match (&self.start, &mut self.frame) {
                (_, Some(m)) if polled => *m += 1,
                (Some(c), None) if holds(o, c) => self.frame = Some(0),
                (None, None) if polled => self.frame = Some(1),
                _ => {}
            }
        }
    }

    pub(crate) fn holds<R: Runtime>(o: &R, c: &Condition) -> bool {
        c.eval(&mut |sel| {
            o.query(&sel.to_string())?
                .first()
                .and_then(|e| e.value.as_int())
                .ok_or(format!("{sel} names no number"))
        })
        .unwrap_or(false)
    }

    /// The rebuilt game from the original's state at `start`, on `inputs`:
    /// the original's state there translated into the model's (the start
    /// translation, carrying every loaded object), with the contract's
    /// tolerances, as `gp interp compare --from` runs them but with no
    /// report.
    pub(crate) fn lockstep_at(
        mdir: &Path,
        source: &Source,
        mapping: &Mapping,
        start: &StartHere,
        inputs: &[gp_runtime::InputFrame],
        mode: Mode,
    ) -> Result<gp_interp::lockstep::Lockstep> {
        let model = source.parse_model()?;
        let mut interp = source.open()?;
        let spec = source.spec.clone();
        let (tolerance, _) = crate::lockstep::tolerances(mdir, &model, mapping);
        let mut base = gp_sim::World::power_on(&spec);
        base.frame = start.model_frame;
        let t = gp_interp::translate::from_original(&start.original, &spec, &base, mapping)
            .map_err(anyhow::Error::msg)?;
        let state = t.state();
        let notes = interp
            .load_state(&state)
            .map_err(|e| anyhow::anyhow!("the model: {e}"))?;
        let st = gp_interp::lockstep::Start {
            frame: start.model_frame,
            original_frame: start.frame,
            notes: t.notes.into_iter().chain(notes).collect(),
            state,
        };
        let opts = gp_interp::lockstep::Options {
            mode,
            tolerance,
            ..Default::default()
        };
        let mut original = start.original.clone();
        gp_interp::lockstep::lockstep_from(
            &mut original,
            &mut interp,
            &spec,
            mapping,
            &st,
            inputs,
            &opts,
        )
        .map_err(anyhow::Error::msg)
    }
}
