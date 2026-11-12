//! Stage 0's extraction into a ROM dataset (docs/formats/rom-dataset.md):
//! what `gp model static` reads and what `gp behave` finds in runs, from
//! the ROM alone, as rows of the dataset's tables. The ROM is run under
//! stage 0's own starts (`gp_explore::presses::start_set`) and read from
//! the vectors and from every routine the runs entered.
//!
//! A value no rule settles is left null, and an `open` row says what
//! decides it: a wrong row is worse than an open one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::rom_dataset::{Dataset, tool};
use gp_nes::codemap::{Va, va_hex, va_parse};
use gp_nes::harvest::Harvest;

/// Each RAM byte a rule places: its element, slot, quantity, source and
/// rule.
type Owner = BTreeMap<u16, (String, Option<i64>, Option<String>, String, String)>;

/// A `links` row: element, routine, link, pc, bytes, source.
type LinkRow = (String, String, &'static str, Option<String>, String, String);

/// An estimate's samples, unit, measurer and the runs they came from.
type Samples = (Vec<f64>, &'static str, &'static str, Vec<String>);

/// What an extraction ran and read, before it becomes rows.
struct Parts {
    h: Harvest,
    unread: Option<gp_nes::harvest::unread::Unread>,
    reached: crate::reach::Reached,
    per_clip: Vec<(String, String, gp_events::Levels)>,
    behave: crate::behave::Built,
    board: gp_nes::codemap::Board,
    /// What gp-track found in each run's frames: the run's recipe hash and
    /// the report, for the runs with frames to track.
    tracked: Vec<(String, gp_track::Report)>,
    /// Each run's player x by the pad ([`pad_x`]), by the run's recipe hash.
    pad_x: Vec<(String, u16)>,
    /// Each run's frames, by the run's recipe hash, for rules that read a
    /// byte's motion once its meaning is known.
    frames: Vec<(String, gp_track::Frames)>,
    /// Each run's RAM writes by frame and its sprite page ([`Snap`]).
    writes: Vec<(String, Vec<Vec<gp_events::activity::Write>>, Option<u16>)>,
}

/// The byte that is the player's x in one run, read from the RAM, the pad
/// and the sprites, for games whose drawing the tracker can't bind (a hero
/// whose sprites shift when it turns, a camera that keeps it still):
///
/// 1. it moves the way the pad says (Right up, Left down) on 95% or more of
///    the 40 or more frames it moves while one of them is held;
/// 2. on 90% of those frames a sprite is drawn within 24 pixels of the
///    byte's place on screen (its value less the scroll), at one offset;
/// 3. on 10% or more of those frames the screen doesn't scroll from the
///    frame before to the frame after (a camera byte moves only when the
///    screen does, sometimes a frame ahead of the scroll register);
/// 4. in the holds it moves in, it moves on half the frames or more (a menu
///    cursor moves once a press), and a tenth or more of its agreeing moves
///    are under each of Right and Left (a byte counting down while Left
///    happens to be held goes one way only);
/// 5. no other byte passes 1 to 4 as often (a subpixel byte passes 1 and
///    fails 2).
fn pad_x(f: &gp_track::Frames) -> Option<u16> {
    let n = f.ram.len().min(f.oam.len()).min(f.pad.len());
    let mut best: Vec<(usize, u16)> = Vec::new();
    for a in 0..0x800usize {
        let (mut moved, mut agree, mut still) = (0usize, 0usize, 0usize);
        let mut frames = Vec::new();
        // Each hold of Left or Right the byte moves in, and the frames of
        // it the byte moves on: a player held to one side moves on most of
        // them, a menu cursor once a press.
        let (mut held_in, mut moved_in) = (0usize, 0usize);
        // Agreeing moves under each direction: the player goes both ways, a
        // byte counting down only one.
        let (mut agree_r, mut agree_l) = (0usize, 0usize);
        let (mut seg_len, mut seg_moves) = (0usize, 0usize);
        for t in 1..n {
            let pad = f.pad[t];
            let (r, l) = (pad & 0x80 != 0, pad & 0x40 != 0);
            if r == l || f.pad[t] != f.pad[t - 1] {
                if seg_moves > 0 {
                    held_in += seg_len;
                    moved_in += seg_moves;
                }
                seg_len = 0;
                seg_moves = 0;
            }
            if r == l {
                continue;
            }
            seg_len += 1;
            let d = f.ram[t][a].wrapping_sub(f.ram[t - 1][a]) as i8;
            if d == 0 {
                continue;
            }
            seg_moves += 1;
            if (d > 0) == r {
                if r {
                    agree_r += 1;
                } else {
                    agree_l += 1;
                }
            }
            moved += 1;
            if (d > 0) == r {
                agree += 1;
                frames.push(t);
            }
            // The screen still from the frame before to the frame after: a
            // camera byte can move a frame before the scroll register does.
            let s0 = f.scroll_x[t - 1];
            if f.scroll_x[t] == s0 && f.scroll_x.get(t + 1).is_none_or(|&s| s == s0) {
                still += 1;
            }
        }
        if seg_moves > 0 {
            held_in += seg_len;
            moved_in += seg_moves;
        }
        if moved < 40
            || agree * 100 < moved * 95
            || still * 10 < moved
            || moved_in * 2 < held_in
            || agree_r * 10 < agree
            || agree_l * 10 < agree
        {
            continue;
        }
        // One window of 24 pixels, at one offset from the byte's place on
        // screen, must hold a sprite on 90% of the frames.
        let window = (0..256usize)
            .map(|o0| {
                frames
                    .iter()
                    .filter(|&&t| {
                        let place = f.ram[t][a].wrapping_sub(f.scroll_x[t] as u8);
                        f.oam[t].as_chunks::<4>().0.iter().any(|s| {
                            s[0] < 0xEF
                                && usize::from(place.wrapping_sub(s[3]).wrapping_sub(o0 as u8)) < 24
                        })
                    })
                    .count()
            })
            .max()
            .unwrap_or(0);
        if window * 10 >= frames.len() * 9 {
            best.push((agree, a as u16));
        }
    }
    best.sort_unstable_by(|x, y| y.cmp(x));
    match best.as_slice() {
        [(n0, a), (n1, _), ..] if n0 > n1 => Some(*a),
        [(_, a)] => Some(*a),
        _ => None,
    }
}

/// Where play starts: the first of `clips` in which the pad moves the
/// player ([`pad_x`]) and the frame it first does; else the first in which
/// the tracker sees a player, and the frame it appears.
pub(crate) fn play_start(
    bytes: &[u8],
    clips: &[crate::reach::ClipInput],
) -> Result<Option<(usize, usize)>> {
    // A run where the pad moves the player is in play at that frame; the
    // tracker's player can be a title screen's or an intro's, so it is used
    // only when no run has the first.
    let mut tracked = None;
    for (i, c) in clips.iter().enumerate() {
        let mut f = frames_of(bytes, c)?;
        if let Some(t) = pad_x(&f).and_then(|x| first_move(&f, x)).filter(|&t| t > 0) {
            return Ok(Some((i, t)));
        }
        if tracked.is_none()
            && let Ok(r) = gp_track::track(&mut f, &gp_track::Options::default())
            && let Some(s) = r.tracks.iter().filter(|t| t.player).map(|t| t.spawn).min()
            && s > 1
        {
            tracked = Some((i, (s - 1) as usize));
        }
    }
    Ok(tracked)
}

/// Runs branched from play, and each one's seed: the start set's seed
/// plus 1000 plus the branch's number, apart from the start runs' seeds.
const BRANCHES: u64 = 4;

fn branch_seed(seed: u64, k: u64) -> u64 {
    seed.wrapping_add(1000 + k)
}

/// The first frame `x` moves the way the pad says, Left or Right held.
fn first_move(f: &gp_track::Frames, x: u16) -> Option<usize> {
    let a = usize::from(x);
    (1..f.ram.len().min(f.pad.len())).find(|&t| {
        let (r, l) = (f.pad[t] & 0x80 != 0, f.pad[t] & 0x40 != 0);
        let d = f.ram[t][a].wrapping_sub(f.ram[t - 1][a]) as i8;
        r != l && d != 0 && (d > 0) == r
    })
}

/// The byte that is the speed of the position byte `pos` in one run, and
/// the scale it holds it in (the position moves by its signed value over
/// the scale a frame: 1 for whole pixels, 16 for sixteenths). On the 40 or
/// more frames `pos` moves, the move is within one of the speed on 95%
/// of them (a fraction carries the rest) and has its sign on 95% of those
/// the speed isn't zero, the speed isn't zero on half of them (a zero byte
/// is "within one" of a one-pixel step), and for whole pixels the move
/// equals it on half. The value may be the frame's or the frame before's.
/// The byte and scale must be ten points clear of any other; whole-pixel
/// candidates are decided before scaled ones.
fn speed_of(f: &gp_track::Frames, pos: u16) -> Option<(u16, u8)> {
    let n = f.ram.len();
    let p = usize::from(pos);
    let moves: Vec<(usize, i32)> = (1..n)
        .filter_map(|t| {
            let d = i32::from(f.ram[t][p].wrapping_sub(f.ram[t - 1][p]) as i8);
            (d != 0).then_some((t, d))
        })
        .collect();
    if moves.len() < 40 {
        return None;
    }
    let m = moves.len();
    let mut scored: Vec<(usize, u16, u8)> = Vec::new();
    for a in (0..0x800usize).filter(|&a| a != p) {
        // A byte's best scale only: one byte at two scales isn't a rival.
        let mut mine: Option<(usize, u16, u8)> = None;
        for k in [1u8, 2, 4, 8, 16] {
            let mut best = 0;
            for lag in [0usize, 1] {
                let (mut near, mut exact, mut nonzero, mut signed) = (0, 0, 0, 0);
                for &(t, d) in &moves {
                    let v = i32::from(f.ram[t - lag][a] as i8);
                    let s = f64::from(v) / f64::from(k);
                    near += usize::from((f64::from(d) - s).abs() <= 1.0);
                    exact += usize::from(k == 1 && d == v);
                    if v != 0 {
                        nonzero += 1;
                        signed += usize::from((d > 0) == (v > 0));
                    }
                }
                if near * 100 >= m * 95
                    && nonzero * 2 >= m
                    && signed * 100 >= nonzero * 95
                    && (k > 1 || exact * 2 >= m)
                {
                    best = best.max(near * 1000 / m);
                }
            }
            if best > 0 && mine.is_none_or(|m| best > m.0) {
                mine = Some((best, a as u16, k));
            }
        }
        scored.extend(mine);
    }
    // A whole-pixel speed also equals the move on half the frames, which
    // a scaled one can't show: whole-pixel candidates are decided first.
    let whole: Vec<(usize, u16, u8)> = scored.iter().copied().filter(|c| c.2 == 1).collect();
    let mut pool = if whole.is_empty() { scored } else { whole };
    pool.sort_unstable_by(|x, y| y.cmp(x));
    match pool.as_slice() {
        [(s0, a, k), (s1, _, _), ..] if s0 - s1 >= 100 => Some((*a, *k)),
        [(_, a, k)] => Some((*a, *k)),
        _ => None,
    }
}

/// The byte that is the facing of the object whose x is the byte `x`, in
/// one run, read from how the object is drawn. The object's sprites are
/// those drawn at one offset (within 24 pixels) from the byte's place on
/// screen; their horizontal flip bit (attribute bit 6, most of them) is
/// the way it faces. The facing byte is the one whose value decides that
/// bit on 98% or more of the frames the object is drawn, over 60 or more
/// frames with each flip on a tenth of them or more; it takes 2 to 4
/// values, and no other byte decides it as often.
fn facing_of(f: &gp_track::Frames, x: u16) -> Option<u16> {
    let n = f.ram.len().min(f.oam.len());
    let xa = usize::from(x);
    // The offset most frames have a sprite at, from the byte's place.
    let mut at = [0usize; 256];
    for t in 0..n {
        let place = f.ram[t][xa].wrapping_sub(f.scroll_x[t] as u8);
        let mut hit = [false; 256];
        for s in f.oam[t].as_chunks::<4>().0.iter().filter(|s| s[0] < 0xEF) {
            hit[usize::from(place.wrapping_sub(s[3]))] = true;
        }
        for (o, h) in hit.iter().enumerate() {
            at[o] += usize::from(*h);
        }
    }
    let (o0, _) = at.iter().enumerate().max_by_key(|(_, c)| **c)?;
    // Each frame's flip: most of the sprites within 24 pixels of the
    // offset, on frames they agree.
    let mut flips: Vec<(usize, bool)> = Vec::new();
    for t in 0..n {
        let place = f.ram[t][xa].wrapping_sub(f.scroll_x[t] as u8);
        let near: Vec<bool> = f.oam[t]
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|s| {
                s[0] < 0xEF
                    && usize::from(
                        place
                            .wrapping_sub(s[3])
                            .wrapping_sub(o0 as u8)
                            .wrapping_add(12),
                    ) < 24
            })
            .map(|s| s[2] & 0x40 != 0)
            .collect();
        let on = near.iter().filter(|b| **b).count();
        if !near.is_empty() && (on == 0 || on == near.len()) {
            flips.push((t, on > 0));
        }
    }
    let m = flips.len();
    let set = flips.iter().filter(|(_, b)| *b).count();
    if m < 60 || set * 10 < m || (m - set) * 10 < m {
        return None;
    }
    let mut scored: Vec<(usize, u16)> = Vec::new();
    for a in (0..0x800usize).filter(|&a| a != xa) {
        // The flip each value means, by majority, and how often it holds.
        let mut by: BTreeMap<u8, (usize, usize)> = BTreeMap::new();
        for &(t, b) in &flips {
            let e = by.entry(f.ram[t][a]).or_default();
            if b {
                e.0 += 1;
            } else {
                e.1 += 1;
            }
        }
        if !(2..=4).contains(&by.len()) {
            continue;
        }
        let held: usize = by.values().map(|(a, b)| (*a).max(*b)).sum();
        if held * 100 >= m * 98 {
            scored.push((held, a as u16));
        }
    }
    scored.sort_unstable_by(|x, y| y.cmp(x));
    match scored.as_slice() {
        [(s0, a), (s1, _), ..] if s0 > s1 => Some(*a),
        [(_, a)] => Some(*a),
        _ => None,
    }
}

/// Each frame's state from a run of `clip`, as gp-track reads it: the RAM
/// at a frame's end with the sprite table and scroll drawn from it, and the
/// controller the frame ran with. Whether the sprites drawn from a frame's
/// RAM are that frame's or the next's (a game that copies its sprite
/// buffer to the PPU at the start of the next frame) is read from the run:
/// the lag at which some RAM page equals the sprite table on more frames.
fn frames_of(rom: &[u8], clip: &crate::reach::ClipInput) -> Result<gp_track::Frames> {
    Ok(snapshots_of(rom, clip)?.frames)
}

/// One run's frames with what wrote RAM on each: the shared run every rule
/// that follows a drawn object back to its code reads.
struct Snap {
    frames: gp_track::Frames,
    /// Each frame's RAM writes (address, value, writer and its routine, as
    /// PRG offsets), by the same index as `frames.ram`.
    writes: Vec<Vec<gp_events::activity::Write>>,
    /// The RAM page the game builds its sprites in, when one equals the
    /// sprite table on most frames.
    sprite_page: Option<u16>,
}

fn snapshots_of(rom: &[u8], clip: &crate::reach::ClipInput) -> Result<Snap> {
    let mut nes = gp_nes::Nes::power_on(rom, clip.power_on)?;
    nes.set_events(true);
    let mut dma: BTreeMap<u8, usize> = BTreeMap::new();
    let layout = gp_events::Layout::of(nes.bus())?;
    let act =
        gp_events::activity::Activity::watching(layout, &gp_events::Bits::new(0), BTreeSet::new());
    let mut col = gp_events::Collector::with_activity(layout, false, act);
    col.start(nes.bus_mut_for_observers());
    let (mut ram, mut oam, mut scroll, mut pad) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut writes = Vec::new();
    let mut tall = 0usize;
    for input in &clip.inputs {
        if input.reset {
            col.will_reset();
        }
        nes.step_with(*input, |bus| col.before(bus))?;
        let fa = col.end_frame_activity(nes.bus()).expect("watching");
        writes.push(fa.ram);
        for e in nes.take_events() {
            if e.kind == gp_evidence::events::EventKind::PpuWrite && e.addr == 0x4014 {
                *dma.entry(e.value).or_default() += 1;
            }
        }
        ram.push(nes.wram()[..0x800].to_vec());
        oam.push(nes.oam().to_vec());
        let sc = &nes.bus().ppu.scroll;
        let t = sc.t;
        scroll.push((t & 31) * 8 + u16::from(sc.fine_x as u8 & 7) + ((t >> 10) & 1) * 256);
        pad.push(input.pads[0]);
        tall += usize::from(nes.tall_sprites());
    }
    let equal_at = |lag: usize, p: usize| -> usize {
        (0..ram.len().saturating_sub(lag))
            .filter(|&f| {
                let o = &oam[f + lag];
                o.iter().any(|&b| b != 0) && ram[f][p * 256..p * 256 + 256] == o[..]
            })
            .count()
    };
    let best = |lag: usize| {
        (0..8)
            .map(|p| (equal_at(lag, p), p))
            .max()
            .unwrap_or((0, 0))
    };
    let (b0, b1) = (best(0), best(1));
    let lag = if b0.0 > b1.0 { 0 } else { 1 };
    let (eq, page) = if lag == 0 { b0 } else { b1 };
    let _ = (eq, page);
    // The page the game copies to the sprite table: the value it writes to
    // $4014 most often, when it writes one on most frames and it is a RAM
    // page.
    let sprite_page = dma
        .iter()
        .max_by_key(|(_, n)| **n)
        .filter(|(p, n)| **p < 8 && **n * 2 > ram.len())
        .map(|(p, _)| u16::from(*p));
    let n = ram.len().saturating_sub(lag);
    ram.truncate(n);
    pad.truncate(n);
    writes.truncate(n);
    Ok(Snap {
        frames: gp_track::Frames {
            first: 1,
            ram,
            oam: oam.split_off(lag.min(oam.len())),
            tall: tall * 2 > clip.inputs.len(),
            scroll_x: scroll.split_off(lag.min(scroll.len())),
            pad,
            // The NES's sprite table, read from `oam`.
            sprites: None,
        },
        writes,
        sprite_page,
    })
}

/// The routine's name in the dataset: its entry as `BANK:ADDR`.
fn rname(harvest_name: &str) -> String {
    let t = harvest_name.strip_prefix("sub_").unwrap_or(harvest_name);
    match t.split_once('_') {
        Some((b, a)) if b.len() == 2 => format!("{b}:{a}"),
        _ => t.to_owned(),
    }
}

/// `read:` provenance for an instruction address as the harvest writes it.
fn read_src(pc: &str) -> Option<String> {
    (!pc.is_empty()).then(|| format!("read:{pc}"))
}

/// A RAM place's base address (`$20,x` -> 0x20).
fn place_addr(p: &str) -> Option<u16> {
    let t = p.trim_start_matches('$');
    u16::from_str_radix(t.split(',').next()?, 16).ok()
}

fn run_parts(rom: &Path, runs: usize, seed: u64, paths: &gp_evidence::Paths) -> Result<Parts> {
    let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    let starts = crate::reach::Starts {
        runs,
        seed,
        start_frames: gp_explore::presses::START_FRAMES,
        no_starts: false,
    };
    let mut clips = starts.with(&[], paths)?;
    // Runs branched from play: the first start run in which the pad moves
    // the player ([`pad_x`]), or the tracker sees one, is kept up to the
    // frame it first does, and
    // [`BRANCHES`] runs of presses with no Start or Select go on from
    // there, so more runs play than random presses from power-on reach.
    let source = play_start(&bytes, &clips)?;
    if let Some((i, t)) = source {
        let src = clips[i].clone();
        for k in 0..BRANCHES {
            let mut inputs = src.inputs[..t].to_vec();
            inputs.extend(gp_explore::presses::log_frames(
                &gp_explore::presses::play_run(
                    branch_seed(seed, k),
                    gp_explore::presses::START_FRAMES - t,
                ),
            ));
            clips.push(crate::reach::ClipInput {
                set: format!("branches of {} at {t}, seed {seed}", src.id),
                id: format!("play{}", k + 1),
                power_on: src.power_on,
                inputs,
            });
        }
    }
    let store = gp_events::store::Store::at(paths.home.join("work").join("events"));
    let keep = crate::reach::Keep {
        store: Some(&store),
        ..Default::default()
    };
    let reached = crate::reach::run_clips(&bytes, &clips, &keep)?;
    let mut per_clip = Vec::new();
    for c in &clips {
        let one = crate::reach::run_clips(&bytes, std::slice::from_ref(c), &keep)?;
        let key = one.clips.first().map(|x| x.3.clone()).unwrap_or_default();
        per_clip.push((c.id.clone(), key, one.levels));
    }
    let (mut h, unread) =
        crate::model_static::harvest_seeded(rom, None, true, Some(&reached.levels))?;
    let behave = crate::behave::build(&bytes, &clips)?;
    h.open.extend(crate::behave::open_facts(&behave));
    let (prg, mapper) = gp_nes::harvest::ines_prg(&bytes).context("not an iNES file")?;
    let mut board = gp_nes::codemap::Board::of(prg, mapper);
    board.switched =
        gp_events::names::Namer::observed_switched(&board, &reached.levels.footprint.windows);
    let mut tracked = Vec::new();
    let mut pads = Vec::new();
    let mut kept = Vec::new();
    let mut snaps = Vec::new();
    for (c, (_, key, _)) in clips.iter().zip(&per_clip) {
        let snap = snapshots_of(&bytes, c)?;
        snaps.push((key.clone(), snap.writes, snap.sprite_page));
        let mut frames = snap.frames;
        if let Some(x) = pad_x(&frames) {
            pads.push((key.clone(), x));
        }
        // A run with nothing to track (no sprite moves long enough) gives
        // no report, not an error.
        let r = gp_track::track(&mut frames, &gp_track::Options::default());
        if let Ok(r) = r {
            tracked.push((key.clone(), r));
        }
        kept.push((key.clone(), frames));
    }
    Ok(Parts {
        h,
        unread,
        reached,
        per_clip,
        behave,
        board,
        tracked,
        pad_x: pads,
        frames: kept,
        writes: snaps,
    })
}

/// The element a harvest machine's path belongs to.
fn machine_element(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split('.').collect();
    match segs.as_slice() {
        ["entities", e, ..] => Some((*e).to_owned()),
        ["rules", ..] => Some("rules".to_owned()),
        _ => None,
    }
}

/// The schema's element kind for a harvest element.
fn element_kind(name: &str, kind: &str) -> &'static str {
    match (kind, name) {
        ("player", _) => "player",
        ("entity kind", _) => "slots",
        ("camera", _) => "camera",
        ("hud", _) => "hud",
        ("game rules", _) => "game",
        (_, "hud") => "hud",
        (_, "input") => "input",
        (_, "level") => "level",
        (_, "sound") => "sound",
        _ => "other",
    }
}

/// A harvest fact's element: the name after `entities.`, or the element a
/// counter, camera or the rules belong to.
fn fact_element(path: &str, elements: &BTreeSet<String>) -> Option<String> {
    let segs: Vec<&str> = path.split(['.', '[']).collect();
    let cand = match segs.as_slice() {
        ["entities", e, ..] => (*e).to_owned(),
        ["cameras", ..] => "camera".to_owned(),
        ["rules", ..] => "rules".to_owned(),
        _ => return None,
    };
    elements.contains(&cand).then_some(cand)
}

/// The mechanic part and parameter a fact path names, when the part is
/// one the catalogue has (`entities.player.jump.gravity` ->
/// `platformer.jump`, `gravity`).
fn mechanic_of(path: &str) -> Option<(String, String)> {
    let segs: Vec<&str> = path.split('.').collect();
    let ["entities", _, part, param] = segs.as_slice() else {
        return None;
    };
    let name = format!("platformer.{part}");
    gp_interp::mechanics::all_components()
        .iter()
        .any(|m| m.name == name)
        .then(|| (name, (*param).to_owned()))
}

/// The schema's constant quantity for a mechanic parameter, where the
/// parameter's meaning is the quantity's.
fn quantity_of_param(param: &str) -> Option<&'static str> {
    Some(match param {
        "gravity" => "gravity",
        "speed" => "jump_speed",
        "walk_speed" | "run_speed" => "max_speed",
        "period" => "period",
        "after" => "duration",
        _ => return None,
    })
}

/// The fixed-point format that turns `raw` into `number`, if one of the
/// schema's forms does, the plainest first.
fn format_of(raw: i64, number: f64) -> Option<&'static str> {
    let s8 = if raw >= 128 { raw - 256 } else { raw };
    let forms: [(&str, f64); 5] = [
        ("u8", raw as f64),
        ("s8", s8 as f64),
        ("4.4", raw as f64 / 16.0),
        ("0.8", raw as f64 / 256.0),
        ("s4.4", s8 as f64 / 16.0),
    ];
    forms
        .iter()
        .find(|(_, v)| (*v - number).abs() < 1e-9)
        .map(|(f, _)| match *f {
            "s4.4" => "4.4",
            x => x,
        })
}

/// A fact's value as one number: `0.25`, `"0.25"` the same.
fn number_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub(crate) fn extract(
    rom: &Path,
    runs: usize,
    seed: u64,
    paths: &gp_evidence::Paths,
) -> Result<Dataset> {
    let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    let (img, _, _) = gp_evidence::source::nes_banks(&bytes)?;
    let sha = img.sha256.clone();
    let p = run_parts(rom, runs, seed, paths)?;
    let tool = tool();
    let mut d = Dataset::default();
    let lv = &p.reached.levels;
    crate::rom_dataset::rom_row(
        &mut d,
        &img,
        p.per_clip.len() as i64,
        lv.frames as i64,
        None,
    )?;
    let namer = gp_events::names::Namer::of_board(p.board.clone());
    let ran_at = |v: Va| {
        p.board
            .prg_offset(v)
            .is_some_and(|o| lv.footprint.starts.contains(o))
    };

    // Runs.
    for (id, key, l) in &p.per_clip {
        let entered: BTreeSet<u32> = l
            .edges
            .iter()
            .filter(|e| e.kind.enters())
            .map(|e| e.to)
            .collect();
        let (kind, seed) = match id.as_str() {
            "idle" => ("idle", None),
            "start" => ("start", None),
            // `randK` is seeded the start set's seed plus K - 1.
            r if r.starts_with("rand") => (
                "random",
                r[4..].parse::<i64>().ok().map(|k| seed as i64 + k - 1),
            ),
            r if r.starts_with("play") => (
                "explore",
                r[4..]
                    .parse::<u64>()
                    .ok()
                    .map(|k| branch_seed(seed, k - 1) as i64),
            ),
            _ => ("clip", None),
        };
        d.push(
            "runs",
            json!({"rom": sha, "run": key, "kind": kind, "seed": seed, "frames": l.frames,
                   "routines": entered.len(), "prg_bytes": l.footprint.executed.count(),
                   "ram_written": l.footprint.ram_written.count(),
                   "rule": "stage 0's starts", "tool": tool}),
        )?;
    }

    // Elements.
    let element_names: BTreeSet<String> = p.h.elements.iter().map(|e| e.name.clone()).collect();
    let fact_num = |path: &str| -> Option<i64> {
        p.h.facts
            .iter()
            .find(|f| f.path == path)
            .and_then(|f| f.value.as_i64())
    };

    // RAM: which element and quantity each byte is, where a rule says so.
    // owner: byte -> (element, slot, quantity, src, rule)
    let mut owner: Owner = BTreeMap::new();
    // links rows: element, routine, link, pc, bytes, src.
    let mut link_rows: Vec<LinkRow> = Vec::new();
    let mut ambiguous: BTreeSet<u16> = BTreeSet::new();
    let claim = |a: u16,
                 el: &str,
                 slot: Option<i64>,
                 q: Option<&str>,
                 src: String,
                 rule: &str,
                 owner: &mut Owner,
                 amb: &mut BTreeSet<u16>| {
        match owner.get(&a) {
            None => {
                owner.insert(
                    a,
                    (
                        el.to_owned(),
                        slot,
                        q.map(str::to_owned),
                        src,
                        rule.to_owned(),
                    ),
                );
            }
            Some((e, ..)) if e == el => {}
            Some(_) => {
                amb.insert(a);
            }
        }
    };
    // 1. Facts that name an element's bytes (`entities.player.bytes.x`).
    for f in &p.h.facts {
        let segs: Vec<&str> = f.path.split('.').collect();
        let (el, q) = match segs.as_slice() {
            ["entities", e, "bytes", q] => ((*e).to_owned(), Some(*q)),
            ["cameras", _, "bytes", q] => ("camera".to_owned(), Some(*q)),
            _ => continue,
        };
        if !element_names.contains(&el) {
            continue;
        }
        let q = q.filter(|q| matches!(*q, "x" | "y" | "count"));
        let src = f
            .evidence
            .first()
            .and_then(|e| read_src(&e.pc))
            .unwrap_or_default();
        let vals: Vec<String> = match &f.value {
            Value::String(s) => vec![s.clone()],
            Value::Array(a) => a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect(),
            _ => continue,
        };
        let n = fact_num(&format!("entities.{el}.count"));
        for v in vals {
            let Some(a) = place_addr(&v) else { continue };
            if v.contains(',') {
                for s in 0..n.unwrap_or(0) {
                    claim(
                        a + s as u16,
                        &el,
                        Some(s),
                        q,
                        src.clone(),
                        "a fact names the element's bytes",
                        &mut owner,
                        &mut ambiguous,
                    );
                }
            } else {
                claim(
                    a,
                    &el,
                    None,
                    q,
                    src.clone(),
                    "a fact names the element's bytes",
                    &mut owner,
                    &mut ambiguous,
                );
            }
        }
    }
    // 2. Machines: the byte that holds the state.
    for m in &p.h.machines {
        if let (Some(a), Some(el)) = (place_addr(&m.byte), machine_element(&m.path))
            && element_names.contains(&el)
        {
            let src = m
                .transitions
                .first()
                .and_then(|t| read_src(&t.at))
                .unwrap_or_default();
            if m.byte.contains(',') {
                let n = fact_num(&format!("entities.{el}.count")).unwrap_or(0);
                for s in 0..n {
                    claim(
                        a + s as u16,
                        &el,
                        Some(s),
                        Some("state"),
                        src.clone(),
                        "a machine's state byte",
                        &mut owner,
                        &mut ambiguous,
                    );
                }
            } else {
                claim(
                    a,
                    &el,
                    None,
                    Some("state"),
                    src,
                    "a machine's state byte",
                    &mut owner,
                    &mut ambiguous,
                );
            }
        }
    }
    // 3. A slot element's indexed bytes, one a slot.
    for e in &p.h.elements {
        if e.kind != "entity kind" {
            continue;
        }
        let Some(n) = fact_num(&format!("entities.{}.count", e.name)) else {
            continue;
        };
        for r in &e.mapping {
            if !r.byte.contains(',') || r.role == "frame counter" {
                continue;
            }
            let Some(a) = place_addr(&r.byte) else {
                continue;
            };
            // Only the slot arrays the element's own facts start at or
            // after: a sprite table or a level row indexed by the same
            // register isn't a slot.
            let base =
                p.h.facts
                    .iter()
                    .filter(|f| f.path == format!("entities.{}.bytes.x", e.name))
                    .filter_map(|f| f.value.as_str().and_then(place_addr))
                    .next();
            let Some(base) = base else { continue };
            if a < base || a >= base + 0x20 {
                continue;
            }
            for s in 0..n {
                claim(
                    a + s as u16,
                    &e.name,
                    Some(s),
                    None,
                    String::new(),
                    "a slot array of the element",
                    &mut owner,
                    &mut ambiguous,
                );
            }
        }
    }
    // 4. What moves on screen (gp-track over each run's frames): the
    // object the controller moves is the player; objects whose bytes sit
    // at one stride are a group of slots, named by their x array as the
    // harvest names its slot elements. A find counts when at least two
    // runs make it and no other find of the same thing has as many runs:
    // one run's find can be a title screen's or a demo's.
    let mut part_of: BTreeMap<u16, &'static str> = BTreeMap::new();
    // A speed byte's scale: the position moves by its value over this.
    let mut speed_scale: BTreeMap<u16, u8> = BTreeMap::new();
    let mut seen_elements: BTreeMap<String, (&'static str, Option<i64>, String)> = BTreeMap::new();
    /// The value most runs give, if two or more do and no other value has
    /// as many.
    fn agreed<T: Ord + Clone>(votes: &[T]) -> Option<T> {
        let mut n: BTreeMap<&T, usize> = BTreeMap::new();
        for v in votes {
            *n.entry(v).or_default() += 1;
        }
        let best = n.values().copied().max()?;
        let top: Vec<&&T> = n
            .iter()
            .filter(|(_, c)| **c == best)
            .map(|(v, _)| v)
            .collect();
        (best >= 2 && top.len() == 1).then(|| (*top[0]).clone())
    }
    let src_of = |runs: &[String]| -> String {
        serde_json::to_string(&runs.iter().map(|r| format!("run:{r}")).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let players: Vec<(String, &gp_track::slots::Player)> = p
        .tracked
        .iter()
        .filter_map(|(k, r)| Some((k.clone(), r.player.as_ref()?)))
        .collect();
    // One vote a run: the byte the pad moves, or else the tracker's player.
    // The pad's test is on the world (a camera that keeps the player still
    // on screen fails the tracker's, which can bind a screen copy).
    let votes: Vec<(String, u16)> = p
        .per_clip
        .iter()
        .filter_map(|(_, k, _)| {
            p.pad_x
                .iter()
                .find(|(pk, _)| pk == k)
                .map(|(_, x)| *x)
                .or_else(|| {
                    players
                        .iter()
                        .find(|(pk, _)| pk == k)
                        .map(|(_, pl)| pl.x.ram)
                })
                .map(|x| (k.clone(), x))
        })
        .collect();
    let player_x = agreed(&votes.iter().map(|(_, x)| *x).collect::<Vec<_>>());
    if let Some(x) = player_x {
        let with: Vec<&(String, &gp_track::slots::Player)> =
            players.iter().filter(|(_, pl)| pl.x.ram == x).collect();
        let src = src_of(
            &votes
                .iter()
                .filter(|(_, v)| *v == x)
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>(),
        );
        let rule = "the byte the controller moves with a sprite drawn by it, in two or more runs (gp-track, the pad)";
        let page = agreed(
            &with
                .iter()
                .filter_map(|(_, pl)| pl.page.as_ref().map(|f| f.ram))
                .collect::<Vec<_>>(),
        );
        let y = agreed(
            &with
                .iter()
                .filter_map(|(_, pl)| pl.y.as_ref().map(|f| f.ram))
                .collect::<Vec<_>>(),
        );
        claim(
            x,
            "player",
            None,
            Some("x"),
            src.clone(),
            rule,
            &mut owner,
            &mut ambiguous,
        );
        // Without a page byte the runs don't say whether x is a whole
        // value or the low byte of one.
        if page.is_some() {
            part_of.insert(x, "lo");
        }
        if let Some(pg) = page {
            claim(
                pg,
                "player",
                None,
                Some("x"),
                src.clone(),
                rule,
                &mut owner,
                &mut ambiguous,
            );
            part_of.insert(pg, "hi");
        }
        if let Some(y) = y {
            claim(
                y,
                "player",
                None,
                Some("y"),
                src.clone(),
                rule,
                &mut owner,
                &mut ambiguous,
            );
        }
        // The speed of each position byte, where two runs agree.
        for (pos, q) in [(Some(x), "speed_x"), (y, "speed_y")] {
            let Some(pos) = pos else { continue };
            let sv: Vec<(String, (u16, u8))> = p
                .frames
                .iter()
                .filter_map(|(k, f)| Some((k.clone(), speed_of(f, pos)?)))
                .collect();
            if let Some((v, scale)) = agreed(&sv.iter().map(|(_, a)| *a).collect::<Vec<_>>()) {
                speed_scale.insert(v, scale);
                let vsrc = src_of(
                    &sv.iter()
                        .filter(|(_, a)| a.0 == v)
                        .map(|(k, _)| k.clone())
                        .collect::<Vec<_>>(),
                );
                claim(
                    v,
                    "player",
                    None,
                    Some(q),
                    vsrc,
                    "the byte whose value is the position's change each frame, in two or more runs",
                    &mut owner,
                    &mut ambiguous,
                );
            }
        }
        // Its facing, read from how its sprites are drawn, where two runs
        // agree.
        let fv: Vec<(String, u16)> = p
            .frames
            .iter()
            .filter_map(|(k, f)| Some((k.clone(), facing_of(f, x)?)))
            .collect();
        if let Some(b) = agreed(&fv.iter().map(|(_, a)| *a).collect::<Vec<_>>()) {
            let fsrc = src_of(
                &fv.iter()
                    .filter(|(_, a)| *a == b)
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<_>>(),
            );
            claim(
                b,
                "player",
                None,
                Some("facing"),
                fsrc,
                "the byte whose value decides the flip of the sprites drawn at the player's x, in two or more runs",
                &mut owner,
                &mut ambiguous,
            );
        }
        seen_elements.insert("player".into(), ("player", None, src));
    }
    // The player's code, by following its sprites back: in each run, the
    // instruction whose writes to the sprite buffer's x bytes equal the
    // player's x less the scroll, plus a constant, on 90% of 40 or more
    // writes draws it; the routines whose writes change its x update it.
    // The bytes the drawing routine reads (as the harvest read it) that the
    // updating routines write (as the runs saw) are the player's.
    let mut draw_votes: Vec<(String, (u32, u32))> = Vec::new();
    let mut updaters: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let mut written_by: BTreeMap<u32, BTreeSet<u16>> = BTreeMap::new();
    // How many writes to the player's known bytes each routine made.
    let mut update_frames: BTreeMap<u32, usize> = BTreeMap::new();
    if let Some(x) = player_x {
        let xa = usize::from(x);
        // The player's bytes the runs already know: its x, page, y and
        // speeds. A routine that writes them moves the player.
        let known: BTreeSet<u16> = owner
            .iter()
            .filter(|(_, o)| o.0 == "player")
            .map(|(a, _)| *a)
            .collect();
        for (k, writes, page) in &p.writes {
            let Some((_, f)) = p.frames.iter().find(|(fk, _)| fk == k) else {
                continue;
            };
            for ws in writes {
                for w in ws {
                    if w.routine != gp_events::NO_ROM {
                        written_by.entry(w.routine).or_default().insert(w.addr);
                        if known.contains(&w.addr) {
                            *update_frames.entry(w.routine).or_default() += 1;
                            updaters.entry(w.routine).or_default().insert(k.clone());
                        }
                    }
                }
            }
            let Some(page) = page else { continue };
            // Per frame: each instruction's offsets from the player's place
            // among its writes to sprite x bytes. One instruction draws
            // every sprite of every object in a loop, so the test is that
            // one of its writes on a frame is at the offset, on 90% of the
            // frames it writes.
            let mut fit: BTreeMap<(u32, u8), usize> = BTreeMap::new();
            let mut all: BTreeMap<u32, (usize, u32)> = BTreeMap::new();
            for (t, ws) in writes.iter().enumerate() {
                let Some(ram) = f.ram.get(t) else { break };
                if t == 0 || f.ram[t - 1][xa] == ram[xa] {
                    continue;
                }
                let place = ram[xa].wrapping_sub(f.scroll_x[t] as u8);
                let mut seen: BTreeMap<u32, BTreeSet<u8>> = BTreeMap::new();
                for w in ws
                    .iter()
                    .filter(|w| w.addr >> 8 == *page && w.addr % 4 == 3)
                {
                    seen.entry(w.pc)
                        .or_default()
                        .insert(w.value.wrapping_sub(place));
                    all.entry(w.pc).or_insert((0, w.routine));
                }
                for (pc, offs) in seen {
                    all.get_mut(&pc).expect("inserted").0 += 1;
                    for o in offs {
                        *fit.entry((pc, o)).or_default() += 1;
                    }
                }
            }
            let best = fit
                .iter()
                .filter(|((pc, _), n)| **n >= 40 && **n * 10 >= all[pc].0 * 9)
                .max_by_key(|(_, n)| **n);
            if let Some(((pc, _), _)) = best {
                draw_votes.push((k.clone(), (*pc, all[pc].1)));
            }
        }
    }
    let windows = &lv.footprint.windows;
    let va_of = |o: u32| -> Option<String> {
        let page = (o / 0x400) as usize;
        let mask = *windows.get(page)?;
        if mask == 0 {
            return None;
        }
        let cpu = (mask.trailing_zeros() as u16) * 0x400 + (o % 0x400) as u16;
        namer.va(o, cpu).map(va_hex)
    };
    if let (Some(x), Some((dpc, droutine))) = (
        player_x,
        agreed(&draw_votes.iter().map(|(_, d)| *d).collect::<Vec<_>>()),
    ) && let (Some(dname), Some(dpc_va)) = (va_of(droutine), va_of(dpc))
    {
        // Each routine's reads, as the harvest read them, and who calls it.
        let mut reads_of: BTreeMap<String, BTreeSet<u16>> = BTreeMap::new();
        for e in &p.h.elements {
            for r in &e.mapping {
                if r.byte.contains(',') {
                    continue;
                }
                if let Some(a) = place_addr(&r.byte) {
                    for n in &r.read_by {
                        reads_of.entry(rname(n)).or_default().insert(a);
                    }
                }
            }
        }
        let mut callers_of: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for r in &p.h.routines {
            for c in &r.calls {
                callers_of
                    .entry(rname(c))
                    .or_default()
                    .insert(r.addr.clone());
            }
        }
        // The player's drawing code: the routine that wrote the sprite, or
        // up to two callers above it, that reads the player's x itself (a
        // shared sprite routine takes its place from its caller).
        let mut drawers: BTreeSet<String> = BTreeSet::new();
        let mut level: BTreeSet<String> = [dname.clone()].into();
        for _ in 0..3 {
            let mut next = BTreeSet::new();
            for r in &level {
                if reads_of.get(r).is_some_and(|s| s.contains(&x)) {
                    drawers.insert(r.clone());
                } else if let Some(cs) = callers_of.get(r) {
                    next.extend(cs.iter().cloned());
                }
            }
            level = next;
        }
        let reads: BTreeSet<u16> = drawers
            .iter()
            .filter_map(|r| reads_of.get(r))
            .flatten()
            .copied()
            .collect();
        // Routines that write the player's known bytes in two runs or more,
        // and half as often as the busiest one: a level start or a restart
        // sets them once.
        let busiest = update_frames.values().copied().max().unwrap_or(0);
        let ups: Vec<u32> = updaters
            .iter()
            .filter(|(r, runs)| {
                runs.len() >= 2 && update_frames.get(r).copied().unwrap_or(0) * 2 >= busiest
            })
            .map(|(r, _)| *r)
            .collect();
        let up_writes: BTreeSet<u16> = ups
            .iter()
            .filter_map(|r| written_by.get(r))
            .flatten()
            .copied()
            .collect();
        let sprite_pages: BTreeSet<u16> = p.writes.iter().filter_map(|(_, _, pg)| *pg).collect();
        let theirs: Vec<u16> = reads
            .intersection(&up_writes)
            .copied()
            .filter(|a| *a < 0x800 && !sprite_pages.contains(&(a >> 8)) && *a != x)
            .collect();
        let dsrc = src_of(
            &draw_votes
                .iter()
                .filter(|(_, d)| *d == (dpc, droutine))
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>(),
        );
        // The bytes both sides touch are listed with the links, not stated
        // as the player's: on Moss Hop the first such byte belonged to
        // another element in the key, and stating it moved the elements'
        // matching.
        let _ = &theirs;
        let hex = |v: &[u16]| {
            serde_json::to_string(&v.iter().map(|a| format!("${a:04X}")).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        link_rows.push((
            "player".to_owned(),
            dname.clone(),
            "draws",
            Some(dpc_va),
            "[]".to_owned(),
            dsrc.clone(),
        ));
        for r in drawers.iter().filter(|r| **r != dname) {
            let rd: Vec<u16> = reads_of
                .get(r)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            link_rows.push((
                "player".to_owned(),
                r.clone(),
                "draws",
                None,
                hex(&rd),
                dsrc.clone(),
            ));
        }
        for r in ups {
            if let Some(name) = va_of(r) {
                let w: Vec<u16> = written_by
                    .get(&r)
                    .map(|s| s.iter().copied().collect())
                    .unwrap_or_default();
                link_rows.push((
                    "player".to_owned(),
                    name,
                    "updates",
                    None,
                    hex(&w),
                    "run".to_owned(),
                ));
            }
        }
    }
    let mut groups: BTreeMap<(u16, usize), Vec<(String, &gp_track::slots::Group)>> =
        BTreeMap::new();
    for (k, r) in &p.tracked {
        for g in &r.groups {
            if r.player.as_ref().is_some_and(|pl| pl.x.ram == g.x.ram) {
                continue;
            }
            groups
                .entry((g.x.ram, g.stride))
                .or_default()
                .push((k.clone(), g));
        }
    }
    // A group's x array where two or more runs agree on its stride.
    let by_x: BTreeMap<u16, usize> = {
        let mut v: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
        for (key, runs) in &groups {
            for _ in runs {
                v.entry(key.0).or_default().push(key.1);
            }
        }
        v.into_iter()
            .filter_map(|(x, s)| Some((x, agreed(&s)?)))
            .collect()
    };
    for (x, stride) in by_x {
        let runs = &groups[&(x, stride)];
        let count = runs.iter().map(|(_, g)| g.count).min().unwrap_or(0);
        // A game that keeps the player in its object arrays has the player
        // as the first slot: the group is the slots after it. The player
        // anywhere else in a group leaves too little said to state it.
        let first = match player_x {
            Some(px) if px == x => 1,
            Some(px) if px > x && px < x + (stride * count) as u16 => continue,
            _ => 0,
        };
        if count <= first {
            continue;
        }
        let x0 = x + (stride * first) as u16;
        let name = format!("slots_{x0:02X}");
        let rule = "objects drawn from bytes at one stride, in two or more runs (gp-track)";
        let src = src_of(&runs.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>());
        let field = |f: &dyn Fn(&gp_track::slots::Group) -> Option<u16>| {
            agreed(&runs.iter().filter_map(|(_, g)| f(g)).collect::<Vec<_>>())
        };
        let fields: [(Option<u16>, &str, Option<&'static str>); 2] = [
            (Some(x), "x", None),
            (field(&|g| g.y.as_ref().map(|f| f.ram)), "y", None),
        ];
        // A group states its x and y bytes only. The tracker's active-flag
        // and kind bytes named wrong bytes on SMB1, and its page byte on
        // Mega Man 2; its slot numbers count from the first slot the runs
        // drew, not from the game's array, so no slot number is stated.
        for i in first..count {
            let off = (stride * i) as u16;
            for (base, q, part) in &fields {
                let Some(base) = base else { continue };
                let a = base + off;
                claim(
                    a,
                    &name,
                    None,
                    Some(q),
                    src.clone(),
                    rule,
                    &mut owner,
                    &mut ambiguous,
                );
                if let Some(pt) = part {
                    part_of.insert(a, pt);
                }
            }
        }
        // How many slots the runs drew is a lower bound, not the count.
        let _ = count;
        seen_elements.insert(name, ("slots", None, src));
    }
    for a in &ambiguous {
        owner.remove(a);
        part_of.remove(a);
    }

    // Each byte the code or the runs touch.
    let mut readers: BTreeMap<u16, BTreeSet<String>> = BTreeMap::new();
    let mut writers: BTreeMap<u16, BTreeSet<String>> = BTreeMap::new();
    let mut harvest_role: BTreeMap<u16, BTreeSet<String>> = BTreeMap::new();
    for e in &p.h.elements {
        for r in &e.mapping {
            let Some(a) = place_addr(&r.byte) else {
                continue;
            };
            if r.byte.contains(',') {
                continue;
            }
            readers
                .entry(a)
                .or_default()
                .extend(r.read_by.iter().map(|x| rname(x)));
            writers
                .entry(a)
                .or_default()
                .extend(r.written_by.iter().map(|x| rname(x)));
            harvest_role.entry(a).or_default().insert(r.role.clone());
        }
    }
    let class: BTreeMap<u16, &gp_nes::ramrole::RamRole> =
        p.behave.roles.roles.iter().map(|r| (r.addr, r)).collect();
    let timers: BTreeSet<u16> = p
        .behave
        .timers
        .iter()
        .filter(|t| t.step < 0 && t.values.len() > 2)
        .map(|t| t.addr)
        .collect();
    let mut bytes_seen: BTreeSet<u16> = BTreeSet::new();
    for i in 0..lv.footprint.ram_written.len().min(0x800) {
        if lv.footprint.ram_written.contains(i) {
            bytes_seen.insert(i as u16);
        }
    }
    for i in 0..lv.footprint.prg_ram_written.len().min(0x2000) {
        if lv.footprint.prg_ram_written.contains(i) {
            bytes_seen.insert(0x6000 + i as u16);
        }
    }
    bytes_seen.extend(owner.keys());
    bytes_seen.extend(harvest_role.keys().filter(|&&a| a < 0x800));
    let mut ram_element: BTreeMap<String, usize> = BTreeMap::new();
    for &a in &bytes_seen {
        let written = if a >= 0x6000 {
            lv.footprint.prg_ram_written.contains((a - 0x6000) as usize)
        } else {
            lv.footprint.ram_written.contains(a as usize)
        };
        let hr = harvest_role.get(&a);
        // Only the classifier's input role is exact enough to state: graded
        // on three games by other authors, its state, timer and scratch
        // classes were wrong on 20% to 40% of the bytes it gave them, and
        // pointer on a byte the code also uses as scratch (docs/research/deterministic-inference.md, 2026-11-01). Its
        // other guesses and the harvest's own go into the open row.
        let _ = &timers;
        let guess: Option<String> =
            if hr.is_some_and(|s| s.contains("timer")) || timers.contains(&a) {
                Some("timer".into())
            } else {
                class.get(&a).map(|c| c.role.name().to_owned())
            };
        let role: Option<&str> = class
            .get(&a)
            .filter(|c| !c.review)
            .map(|c| c.role.name())
            .filter(|r| *r == "input");
        let (element, slot, quantity, osrc, orule) = match owner.get(&a) {
            Some((e, s, q, src, rule)) => (
                Some(e.clone()),
                *s,
                q.clone(),
                Some(src.clone()),
                Some(rule.clone()),
            ),
            None => (None, None, None, None, None),
        };
        if let Some(e) = &element {
            *ram_element.entry(e.clone()).or_default() += 1;
        }
        let mut src = Vec::new();
        if let Some(s) = osrc.filter(|s| !s.is_empty()) {
            src.push(s);
        }
        if class.contains_key(&a) || written {
            src.push("run".to_owned());
        }
        let rule = match (&orule, role) {
            (Some(r), Some(_)) => format!("{r}; role from the runs"),
            (Some(r), None) => r.clone(),
            (None, Some(_)) => "role from the runs".to_owned(),
            (None, None) => "written by a run".to_owned(),
        };
        d.push(
            "ram",
            json!({"rom": sha, "addr": a, "role": role, "element": element, "slot": slot,
                   "quantity": quantity, "part": part_of.get(&a), "written": written,
                   "readers": serde_json::to_string(&readers.get(&a).cloned().unwrap_or_default())?,
                   "writers": serde_json::to_string(&writers.get(&a).cloned().unwrap_or_default())?,
                   "src": if src.len() == 1 { json!(src[0]) } else { json!(serde_json::to_string(&src)?) },
                   "rule": rule, "tool": tool}),
        )?;
        let why = match (role.is_none(), element.is_none()) {
            (true, true) => Some("no rule gives its role or its element".to_owned()),
            (true, false) => Some("no rule gives its role".to_owned()),
            (false, true) => Some("no rule gives the element it belongs to".to_owned()),
            (false, false) => None,
        };
        let why = why.map(|w| match (&guess, role) {
            (Some(g), None) => {
                format!("{w}; the runs suggest {g}, a class not exact enough to state")
            }
            _ => w,
        });
        if let Some(why) = why {
            let rs: Vec<String> = writers
                .get(&a)
                .into_iter()
                .flatten()
                .chain(readers.get(&a).into_iter().flatten())
                .cloned()
                .collect();
            d.push(
                "open",
                json!({"rom": sha, "what": format!("ram:${a:04X}"), "element": element,
                       "routine": rs.first(), "pc": "", "bytes": serde_json::to_string(&[format!("${a:04X}")])?,
                       "why": why, "settle_by": "trace", "src": "run", "rule": "every written byte has a row", "tool": tool}),
            )?;
        }
    }

    for e in &p.h.elements {
        let tiles: Vec<Value> =
            p.h.facts
                .iter()
                .filter(|f| f.path.starts_with(&format!("entities.{}.tile", e.name)))
                .map(|f| f.value.clone())
                .collect();
        d.push(
            "elements",
            json!({"rom": sha, "element": e.name, "kind": element_kind(&e.name, &e.kind),
                   "slots": fact_num(&format!("entities.{}.count", e.name)),
                   "routines": e.routines.len(), "ram": ram_element.get(&e.name).copied().unwrap_or(0),
                   "tile_kinds": if tiles.is_empty() { Value::Null } else { json!(serde_json::to_string(&tiles)?) },
                   "src": e.routines.first().map(|r| format!("read:{}", rname(r))),
                   "rule": "routines grouped by the bytes they share", "tool": tool}),
        )?;
    }
    // Estimates: values the runs measure close but not exactly, to be
    // refined by later stages. Each is the median of its samples, with
    // their spread.
    let mut samples: BTreeMap<(String, String, String), Samples> = BTreeMap::new();
    let mut sample = |el: &str,
                      mech: &str,
                      prm: &str,
                      v: f64,
                      unit: &'static str,
                      method: &'static str,
                      run: &str| {
        if v.is_finite() {
            let e = samples
                .entry((el.to_owned(), mech.to_owned(), prm.to_owned()))
                .or_insert((Vec::new(), unit, method, Vec::new()));
            e.0.push(v);
            if !e.3.iter().any(|r| r == run) {
                e.3.push(run.to_owned());
            }
        }
    };
    // The player's feel, where a run found the player: walk and run speed,
    // the jump's speed, gravity and fall limit. Which of A and B is the
    // jump is whichever measures more jumps.
    for (k, r) in &p.tracked {
        if r.player.is_none() {
            continue;
        }
        let Some((_, f)) = p.frames.iter().find(|(fk, _)| fk == k) else {
            continue;
        };
        let feel = [(0x01u8, 0x02u8), (0x02, 0x01)]
            .into_iter()
            .map(|(jump, run)| {
                let pad = gp_extract::feel::Pad {
                    left: 0x40,
                    right: 0x80,
                    run,
                    jump,
                };
                gp_extract::feel::measure(&r.tracks, f.first, &f.pad, pad, false, true)
            })
            .max_by_key(|fl| fl.jumps.len())
            .expect("two");
        for run in &feel.runs {
            let prm = if run.running {
                "run_speed"
            } else {
                "walk_speed"
            };
            sample(
                "player",
                "platformer.run",
                prm,
                run.speed,
                "pixels a frame",
                "feel",
                k,
            );
        }
        for j in &feel.jumps {
            for (prm, v, unit) in [
                ("gravity", j.gravity, "pixels a frame a frame"),
                ("gravity_held", j.gravity_held, "pixels a frame a frame"),
                ("speed", j.speed.or(j.speed_held), "pixels a frame"),
                ("max_fall", j.max_fall, "pixels a frame"),
            ] {
                if let Some(v) = v {
                    sample("player", "platformer.jump", prm, v, unit, "feel", k);
                }
            }
        }
        // Each enemy track's step and period, as a patrol, by its group.
        for en in &feel.enemies {
            let Some(t) = r.tracks.get(en.track) else {
                continue;
            };
            let Some((gi, _)) = t.slot else { continue };
            let Some(g) = r.groups.get(gi) else { continue };
            let base = if player_x == Some(g.x.ram) {
                g.x.ram + g.stride as u16
            } else {
                g.x.ram
            };
            let name = format!("slots_{base:02X}");
            if !seen_elements.contains_key(&name) {
                continue;
            }
            if let Some(step) = en.step {
                sample(
                    &name,
                    "platformer.patrol",
                    "step",
                    step.abs() as f64,
                    "pixels",
                    "feel",
                    k,
                );
            }
            if let Some(period) = en.period {
                sample(
                    &name,
                    "platformer.patrol",
                    "period",
                    f64::from(period),
                    "frames",
                    "feel",
                    k,
                );
            }
        }
    }
    // Each enemy group's pattern and its numbers, by the group's element.
    for (k, r) in &p.tracked {
        let player_pos = r
            .tracks
            .iter()
            .find(|t| t.player)
            .map(|t| t.positions.as_slice());
        for t in r.tracks.iter().filter(|t| !t.player) {
            let Some((gi, _)) = t.slot else { continue };
            let Some(g) = r.groups.get(gi) else { continue };
            let base = if player_x == Some(g.x.ram) {
                g.x.ram + g.stride as u16
            } else {
                g.x.ram
            };
            let name = format!("slots_{base:02X}");
            if !seen_elements.contains_key(&name) {
                continue;
            }
            let Some(fit) = gp_track::behaviour::classify(&t.positions, player_pos) else {
                continue;
            };
            for (prm, v) in &fit.params {
                let unit = match *prm {
                    "speed" => "pixels a frame",
                    "gravity" => "pixels a frame a frame",
                    "period" => "frames",
                    _ => "pixels",
                };
                sample(
                    &name,
                    &format!("pattern.{}", fit.pattern),
                    prm,
                    *v,
                    unit,
                    "pattern",
                    k,
                );
            }
        }
    }
    for ((el, mech, prm), (mut v, unit, method, runs)) in samples {
        if v.len() < 2 {
            continue;
        }
        v.sort_by(f64::total_cmp);
        let median = v[v.len() / 2];
        let spread = (v[v.len() - 1] - v[0]) / 2.0;
        let quantity = if mech.starts_with("platformer.") {
            quantity_of_param(&prm)
        } else {
            None
        };
        d.push(
            "estimates",
            json!({"rom": sha, "element": el, "mechanic": mech, "parameter": prm, "quantity": quantity,
                   "value": median, "spread": spread, "samples": v.len(), "unit": unit, "method": method,
                   "refine": "the instruction that writes the value, read once the byte it changes is known",
                   "src": src_of(&runs), "rule": format!("the median of {} {method} samples", v.len()), "tool": tool}),
        )?;
    }
    // Timers: a byte that counts by one, its period and the values it is
    // set to, as gp behave's timer steps read them.
    for t in &p.behave.timers {
        if t.steps < 8 {
            continue;
        }
        // The byte's element, else the one element every routine that
        // writes it belongs to, else the game's.
        let by_writers = || -> Option<String> {
            let els: BTreeSet<String> = writers
                .get(&t.addr)?
                .iter()
                .filter_map(|w| {
                    p.h.routines
                        .iter()
                        .find(|r| r.addr == *w)
                        .map(|r| r.element.clone())
                })
                .filter(|e| !e.is_empty())
                .collect();
            (els.len() == 1).then(|| els.into_iter().next().expect("one"))
        };
        let el = owner
            .get(&t.addr)
            .map(|o| o.0.clone())
            .or_else(by_writers)
            .unwrap_or_else(|| "game".to_owned());
        let periods: Vec<f64> = t.periods.iter().map(|&x| x as f64).collect();
        let reloads: Vec<f64> = t.reloads.iter().map(|&x| x as f64).collect();
        for (prm, vals, unit) in [("period", periods, "frames"), ("reload", reloads, "count")] {
            if vals.is_empty() {
                continue;
            }
            let mut v = vals;
            v.sort_by(f64::total_cmp);
            d.push(
                "estimates",
                json!({"rom": sha, "element": el, "mechanic": "timer", "parameter": format!("${:04X}.{prm}", t.addr),
                       "value": v[v.len() / 2], "spread": (v[v.len() - 1] - v[0]) / 2.0, "samples": t.steps,
                       "unit": unit, "method": "timer",
                       "refine": "the branch that tests the byte and the store that reloads it",
                       "src": "run", "rule": format!("a byte stepping by {:+}", t.step), "tool": tool}),
            )?;
        }
    }

    for (el, routine, link, pc, bytes, src) in &link_rows {
        d.push(
            "links",
            json!({"rom": sha, "element": el, "routine": routine, "link": link, "pc": pc,
                   "bytes": bytes, "src": src,
                   "rule": "the element's sprites followed back to the code that wrote them", "tool": tool}),
        )?;
    }
    // Elements only the screen shows: the player or a group of slots no
    // rule of the code found.
    for (name, (kind, slots, src)) in &seen_elements {
        if element_names.contains(name) {
            continue;
        }
        d.push(
            "elements",
            json!({"rom": sha, "element": name, "kind": kind, "slots": slots, "routines": 0,
                   "ram": ram_element.get(name).copied().unwrap_or(0), "tile_kinds": null,
                   "src": src, "rule": "objects the runs draw (gp-track)", "tool": tool}),
        )?;
    }

    // Routines.
    let mut called_by: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in &p.h.routines {
        for c in &r.calls {
            called_by
                .entry(rname(c))
                .or_default()
                .insert(r.addr.clone());
        }
    }
    let mut listed: BTreeSet<String> = BTreeSet::new();
    for r in &p.h.routines {
        let v = va_parse(&r.addr);
        let bank = r
            .addr
            .split_once(':')
            .and_then(|(b, _)| i64::from_str_radix(b, 16).ok());
        let cpu = v.map(gp_nes::codemap::va_cpu).unwrap_or(0);
        listed.insert(r.addr.clone());
        let callees: BTreeSet<String> = r.calls.iter().map(|c| rname(c)).collect();
        d.push(
            "routines",
            json!({"rom": sha, "routine": r.addr, "bank": bank, "addr": cpu, "size": r.size,
                   "element": (!r.element.is_empty()).then(|| r.element.clone()),
                   "read": true, "ran": v.is_some_and(ran_at),
                   "callers": serde_json::to_string(&called_by.get(&r.addr).cloned().unwrap_or_default())?,
                   "callees": serde_json::to_string(&callees)?,
                   "src": format!("read:{}", r.addr), "rule": r.why, "tool": tool}),
        )?;
    }
    if let Some(u) = &p.unread {
        for n in &u.unread_routines {
            let addr = rname(n);
            if !listed.insert(addr.clone()) {
                continue;
            }
            let v = va_parse(&addr);
            let bank = addr
                .split_once(':')
                .and_then(|(b, _)| i64::from_str_radix(b, 16).ok());
            d.push(
                "routines",
                json!({"rom": sha, "routine": addr, "bank": bank,
                       "addr": v.map(gp_nes::codemap::va_cpu).unwrap_or(0), "size": 0,
                       "read": false, "ran": v.is_some_and(ran_at),
                       "callers": "[]", "callees": "[]",
                       "src": format!("read:{addr}"), "rule": "in the code map, no path read it", "tool": tool}),
            )?;
        }
        for s in &u.stops {
            d.push(
                "open",
                json!({"rom": sha, "what": format!("routines behind {}", s.id), "routine": rname(&s.routine),
                       "pc": s.pc, "bytes": serde_json::to_string(&s.bytes)?,
                       "why": format!("the reader stops here ({}); {} routines are behind it", s.reason, s.behind),
                       "settle_by": if s.reason.contains("bank") { "bank" } else { "run" },
                       "src": read_src(&s.pc), "rule": "where the reader stops", "tool": tool}),
            )?;
        }
    }

    // Branches, from the code map of the routines read, with the runs'
    // counts of each way.
    let starts: Vec<Va> =
        p.h.routines
            .iter()
            .filter_map(|r| va_parse(&r.addr))
            .collect();
    let code = gp_nes::codemap::RomCode::of_board(&p.board, &starts, &[]);
    let mut ways: BTreeMap<(u32, bool), u64> = BTreeMap::new();
    for e in &lv.edges {
        let taken = match e.kind {
            gp_events::EdgeKind::Branch => true,
            gp_events::EdgeKind::Fall => false,
            _ => continue,
        };
        if let Some(v) = namer.va(e.from, e.stat.from_cpu) {
            *ways.entry((v, taken)).or_default() += e.stat.count;
        }
    }
    let routine_at = |pc: Va| -> Option<String> {
        p.h.routines
            .iter()
            .filter_map(|r| Some((va_parse(&r.addr)?, r)))
            .filter(|(a, r)| *a <= pc && pc < gp_nes::codemap::va_add(*a, r.size as u16))
            .max_by_key(|(a, _)| *a)
            .map(|(_, r)| r.addr.clone())
    };
    let mut prev: Option<&gp_nes::codemap::Insn> = None;
    for (pc, ins) in &code.insns {
        let m = ins.mn.name();
        if matches!(
            m,
            "bcc" | "bcs" | "beq" | "bne" | "bmi" | "bpl" | "bvc" | "bvs"
        ) && let Some(routine) = routine_at(*pc)
        {
            let reached = ways.contains_key(&(*pc, true)) || ways.contains_key(&(*pc, false));
            // The flags come from the instruction just before, when it
            // reads a RAM place or compares with a constant.
            let (tests, compare) = match prev.filter(|q| q.next() == *pc) {
                Some(q) => {
                    let t = q.text();
                    let place = t
                        .split_once(' ')
                        .map(|x| x.1.to_owned())
                        .filter(|o| o.starts_with('$'));
                    let imm = t
                        .split_once('#')
                        .and_then(|x| u8::from_str_radix(x.1.trim_start_matches('$'), 16).ok());
                    (
                        place.into_iter().collect::<Vec<_>>(),
                        imm.filter(|_| q.mn.name().starts_with("cp") || q.mn.name() == "cmp"),
                    )
                }
                None => (Vec::new(), None),
            };
            d.push(
                "branches",
                json!({"rom": sha, "pc": va_hex(*pc), "routine": routine, "op": m,
                       "tests": serde_json::to_string(&tests)?, "compare": compare,
                       "taken": reached.then(|| ways.get(&(*pc, true)).copied().unwrap_or(0)),
                       "not_taken": reached.then(|| ways.get(&(*pc, false)).copied().unwrap_or(0)),
                       "src": format!("read:{}", va_hex(*pc)), "rule": "a branch in a routine read; its test from the instruction before", "tool": tool}),
            )?;
        }
        prev = Some(ins);
    }

    // States and transitions.
    for m in &p.h.machines {
        let el = machine_element(&m.path);
        let byte = place_addr(&m.byte).unwrap_or(0);
        for s in &m.states {
            d.push(
                "states",
                json!({"rom": sha, "machine": m.name, "byte": byte, "element": el, "value": s.value,
                       "state": s.name, "initial": m.initial.as_deref() == Some(s.name.as_str()),
                       "src": m.transitions.first().and_then(|t| read_src(&t.at)),
                       "rule": "the values a state byte is compared with and set to", "tool": tool}),
            )?;
        }
        for t in &m.transitions {
            d.push(
                "transitions",
                json!({"rom": sha, "machine": m.name, "from_state": t.from, "to_state": t.to, "pc": t.at,
                       "routine": rname(&t.routine), "guards": serde_json::to_string(&t.guards)?,
                       "after": t.after, "src": read_src(&t.at),
                       "rule": "a write of the state byte on a path from the other state", "tool": tool}),
            )?;
        }
    }

    // Constants: each read from an instruction, with what a fact says it
    // is. A fact cites a constant when the instruction the fact's evidence
    // names is the one that uses the constant (`lda #$D0` then the `sta`
    // the fact cites) and the constant decodes to the fact's number: one
    // byte in one of the schema's formats, or two such constants as the
    // two bytes of an 8.8 number.
    let mut cited: BTreeMap<String, (&gp_nes::harvest::Fact, f64, &'static str)> = BTreeMap::new();
    for f in &p.h.facts {
        let Some(n) = number_of(&f.value) else {
            continue;
        };
        let used: Vec<&gp_nes::harvest::Constant> = f
            .evidence
            .iter()
            .flat_map(|e| {
                p.h.constants
                    .iter()
                    .filter(move |c| c.routine == e.routine && c.then == e.insn)
            })
            .collect();
        if let [a, b] = used.as_slice() {
            let pairs = [(a, b), (b, a)];
            let fits: Vec<_> = pairs
                .iter()
                .filter(|(hi, lo)| {
                    let raw = i64::from(hi.value) << 8 | i64::from(lo.value);
                    let s16 = if raw >= 0x8000 { raw - 0x10000 } else { raw };
                    (s16 as f64 / 256.0 - n).abs() < 1e-9
                })
                .collect();
            if let [(hi, lo)] = fits.as_slice() {
                cited.entry(hi.pc.clone()).or_insert((f, n, "8.8 hi"));
                cited.entry(lo.pc.clone()).or_insert((f, n, "8.8 lo"));
                continue;
            }
        }
        let singles: Vec<_> = used
            .iter()
            .filter_map(|c| Some((c, format_of(i64::from(c.value), n)?)))
            .collect();
        if let [(c, fm)] = singles.as_slice() {
            cited.entry(c.pc.clone()).or_insert((f, n, fm));
        }
    }
    // The RAM place each instruction's constant is compared with, from the
    // harvest's open facts (`bytes`), and the unit each placed byte holds.
    let compared_at: BTreeMap<String, String> =
        p.h.open
            .iter()
            .filter(|o| !o.pc.is_empty() && o.bytes.len() == 1)
            .map(|o| (o.pc.clone(), o.bytes[0].clone()))
            .collect();
    let unit_of = |place: &str| -> Option<(&'static str, String)> {
        if place.contains(',') {
            return None;
        }
        let a = place_addr(place)?;
        let (el, _, q, _, _) = owner.get(&a)?;
        // A byte that is a whole value or the low byte of a position, or a
        // whole speed: its unit is pixels, signed for a speed.
        let part = part_of.get(&a).copied();
        match (q.as_deref()?, part) {
            ("x" | "y", None | Some("lo")) => Some(("u8", el.clone())),
            ("speed_x" | "speed_y", None | Some("whole")) => {
                let fm = match speed_scale.get(&a).copied().unwrap_or(1) {
                    2 => "7.1",
                    4 => "6.2",
                    8 => "5.3",
                    16 => "4.4",
                    _ => "s8",
                };
                Some((fm, el.clone()))
            }
            _ => None,
        }
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for c in &p.h.constants {
        if !seen.insert(c.pc.clone()) {
            continue;
        }
        let op = c.insn.split(' ').next().unwrap_or("").to_owned();
        let (mut format, mut number, mut quantity, mut mechanic, mut parameter, mut element) =
            (None, None, None, None, None, None);
        let mut rule = format!("a constant {} ({})", c.what, c.then);
        if let Some((f, n, fm)) = cited.get(&c.pc) {
            format = Some(fm.split(' ').next().unwrap_or(fm));
            number = Some(*n);
            element = fact_element(&f.path, &element_names);
            if let Some((m, prm)) = mechanic_of(&f.path) {
                quantity = quantity_of_param(&prm);
                mechanic = Some(m);
                parameter = Some(prm);
            }
            rule = format!("{} ({})", f.rule, f.path);
        }
        // A constant stored into a byte whose unit the runs gave, or
        // compared with one: the number in that unit.
        if number.is_none() {
            let place = if c.what == "stored" {
                c.then
                    .split_once(' ')
                    .filter(|(m, _)| m.starts_with("st"))
                    .map(|(_, a)| a.to_owned())
            } else if op.starts_with("cp") || op == "cmp" {
                compared_at.get(&c.pc).cloned()
            } else {
                None
            };
            // The element isn't the byte's: comparing the player's y with
            // a kill line is the level's fact.
            if let Some((fm, _)) = place.as_deref().and_then(unit_of) {
                let raw = i64::from(c.value);
                let signed = if fm != "u8" && raw >= 128 {
                    raw - 256
                } else {
                    raw
                } as f64;
                let scale = match fm {
                    "7.1" => 2.0,
                    "6.2" => 4.0,
                    "5.3" => 8.0,
                    "4.4" => 16.0,
                    _ => 1.0,
                };
                number = Some(signed / scale);
                format = Some(fm);
                rule = format!(
                    "a constant {} with {}, a byte the runs give a unit",
                    if c.what == "stored" {
                        "stored"
                    } else {
                        "compared"
                    },
                    place.unwrap_or_default()
                );
            }
        }
        let compared = (op.starts_with("cp") || op == "cmp")
            .then(|| c.then.split_once(' ').map(|x| x.1.to_owned()))
            .flatten();
        d.push(
            "constants",
            json!({"rom": sha, "pc": c.pc, "entry": 0, "routine": rname(&c.routine), "op": op,
                   "raw": c.value, "format": format, "number": number, "quantity": quantity,
                   "element": element, "mechanic": mechanic, "parameter": parameter,
                   "compared": compared.filter(|s| s.starts_with('$')),
                   "src": read_src(&c.pc), "rule": rule, "tool": tool}),
        )?;
    }

    // Facts and mechanics.
    let mut mech_rows: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    for f in &p.h.facts {
        let number = number_of(&f.value);
        let src: Vec<String> = f.evidence.iter().filter_map(|e| read_src(&e.pc)).collect();
        d.push(
            "facts",
            json!({"rom": sha, "path": f.path, "value": f.value.to_string(), "number": number,
                   "element": fact_element(&f.path, &element_names), "settled_by": "read",
                   "src": serde_json::to_string(&src)?, "rule": f.rule, "tool": tool}),
        )?;
        if let Some((m, prm)) = mechanic_of(&f.path)
            && let Some(el) = fact_element(&f.path, &element_names)
        {
            mech_rows.insert((el, m, prm, f.value.to_string()));
        }
    }
    let mut parts: BTreeSet<(String, String)> = BTreeSet::new();
    for (el, m, prm, v) in &mech_rows {
        parts.insert((el.clone(), m.clone()));
        d.push(
            "mechanics",
            json!({"rom": sha, "element": el, "mechanic": m, "option": prm, "value": v,
                   "src": null, "rule": "a fact of the part", "tool": tool}),
        )?;
    }
    for (el, m) in parts {
        d.push(
            "mechanics",
            json!({"rom": sha, "element": el, "mechanic": m, "option": "*", "value": null,
                   "src": null, "rule": "the element has facts of the part", "tool": tool}),
        )?;
    }

    // Open facts the harvest and the runs left.
    for o in &p.h.open {
        let settle = if o.why.contains("gp behave") {
            "trace"
        } else if o.why.contains("bank") {
            "bank"
        } else {
            "trace"
        };
        d.push(
            "open",
            json!({"rom": sha, "what": o.what, "element": null,
                   "routine": (!o.routine.is_empty()).then(|| rname(&o.routine)),
                   "pc": o.pc, "bytes": serde_json::to_string(&o.bytes)?, "why": o.why,
                   "settle_by": settle,
                   "src": read_src(&o.pc).unwrap_or_else(|| "run".into()),
                   "rule": "a fact no rule settles", "tool": tool}),
        )?;
    }
    d.sort();
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routine_is_named_by_its_banked_entry() {
        assert_eq!(rname("sub_C1FB"), "C1FB");
        assert_eq!(rname("sub_03_8123"), "03:8123");
        assert_eq!(place_addr("$20,x"), Some(0x20));
        assert_eq!(place_addr("$0200,y"), Some(0x200));
    }

    #[test]
    fn a_format_is_the_plainest_that_gives_the_number() {
        assert_eq!(format_of(4, 4.0), Some("u8"));
        assert_eq!(format_of(251, -5.0), Some("s8"));
        assert_eq!(format_of(0x40, 4.0), Some("4.4"));
        assert_eq!(format_of(0x40, 0.25), Some("0.8"));
        assert_eq!(format_of(0xF0, -1.0), Some("4.4"));
        assert_eq!(format_of(7, 0.3), None);
    }

    /// 200 frames: Right held for 100, then Left for 100. `$10` is the
    /// player's x (a sprite drawn 4 pixels left of it), `$20` moves the same
    /// way three times as fast with no sprite staying near it, `$30` is a camera
    /// byte that moves only with the scroll.
    fn walk() -> gp_track::Frames {
        let (mut ram, mut oam, mut scroll, mut pad) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut x, mut far, mut cam) = (100u8, 10u8, 0u8);
        for t in 0..200u16 {
            let right = t < 100;
            let p = if right { 0x80 } else { 0x40 };
            if t > 0 {
                x = if right { x + 1 } else { x - 1 };
                far = if right {
                    far.wrapping_add(3)
                } else {
                    far.wrapping_sub(3)
                };
                if right && t > 50 {
                    cam += 1;
                }
            }
            let mut r = vec![0u8; 0x800];
            r[0x10] = x;
            r[0x20] = far;
            r[0x30] = cam;
            let mut o = vec![0xFFu8; 256];
            o[0] = 100;
            o[3] = x.wrapping_sub(4).wrapping_sub(cam);
            ram.push(r);
            oam.push(o);
            scroll.push(u16::from(cam));
            pad.push(p);
        }
        gp_track::Frames {
            first: 1,
            ram,
            oam,
            tall: false,
            scroll_x: scroll,
            pad,
            sprites: None,
        }
    }

    #[test]
    fn the_player_x_is_the_byte_the_pad_moves_with_a_sprite_by_it() {
        assert_eq!(pad_x(&walk()), Some(0x10));
    }

    #[test]
    fn a_byte_the_pad_moves_with_no_sprite_by_it_is_not_the_player() {
        let mut f = walk();
        for o in &mut f.oam {
            o[0] = 0xFF;
        }
        assert_eq!(pad_x(&f), None);
    }

    #[test]
    fn a_mechanic_is_named_only_when_the_catalogue_has_the_part() {
        assert_eq!(
            mechanic_of("entities.player.jump.gravity"),
            Some(("platformer.jump".into(), "gravity".into()))
        );
        assert_eq!(mechanic_of("entities.player.nonesuch.x"), None);
        assert_eq!(mechanic_of("counters.c_09.start"), None);
    }
}
