//! Blind mode for top-down games (docs/research/topdown-extension.md): a
//! ROM with no dataset, template or mapping, played from power-on. It
//! gets through the title and a class select, finds the player's x and y
//! bytes from the d-pad on both axes, a health byte that drains, and
//! plays by search from savestates with a decision model ordering eight-way
//! moves. Progress is area: new 16 px cells reached, on both axes, in each
//! maze. A big scene change after the player has moved, which stays, is a
//! new maze.
//!
//! Generic: nothing here names a game, an address or a constant of one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use serde_json::{Value, json};

use super::support as m;
use super::{Rng, post};
use crate::Outcome;
use gp_evidence::Paths;
use gp_evidence::input_log::InputFrame;

const A: u8 = 0x01;
const B: u8 = 0x02;
const START: u8 = 0x08;
const UP: u8 = 0x10;
const DOWN: u8 = 0x20;
const LEFT: u8 = 0x40;
const RIGHT: u8 = 0x80;

#[derive(Args, Debug)]
pub(crate) struct TopdownArgs {
    /// The ROM to play.
    pub rom: PathBuf,
    /// The decision server (Laya).
    #[arg(long, default_value = "http://127.0.0.1:7720")]
    pub decide: String,
    /// Order the moves at random instead of asking the decision model.
    #[arg(long)]
    pub uniform: bool,
    /// Wall time for the search, in seconds.
    #[arg(long, default_value_t = 300)]
    pub seconds: u64,
    /// Moves simulated a round, in the order given.
    #[arg(long, default_value_t = 3)]
    pub top: usize,
    /// The search's random seed.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// Where to write the run (default
    /// $GP_HOME/work/farm/topdown/<rom name>/<ms>).
    #[arg(long)]
    pub out: Option<PathBuf>,
}

fn frame(pad: u8) -> InputFrame {
    InputFrame::held(pad)
}

/// The console and the inputs from power-on that reached it.
struct Run {
    nes: gp_nes::Nes,
    log: Vec<InputFrame>,
}

impl Run {
    fn hold(&mut self, pad: u8, n: usize) -> Result<()> {
        for _ in 0..n {
            self.nes.step(frame(pad))?;
            self.log.push(frame(pad));
        }
        Ok(())
    }
    fn save(&self) -> Result<(Vec<u8>, u64, [u8; 4], usize)> {
        Ok((
            self.nes.snapshot()?,
            self.nes.frame(),
            self.nes.held(),
            self.log.len(),
        ))
    }
    fn load(&mut self, s: &(Vec<u8>, u64, [u8; 4], usize)) -> Result<()> {
        self.nes.restore(&s.0, s.1, s.2)?;
        self.log.truncate(s.3);
        Ok(())
    }
    fn ram(&self) -> Vec<u8> {
        self.nes.wram()[..0x800].to_vec()
    }
}

pub(super) fn wrap8(d: i64) -> i64 {
    (d + 128).rem_euclid(256) - 128
}

/// RAM each frame while holding `pads` in turn, from the current state; the
/// state is put back after.
fn trace(r: &mut Run, pads: &[(u8, usize)]) -> Result<Vec<Vec<u8>>> {
    let s = r.save()?;
    let mut out = vec![r.ram()];
    for &(p, n) in pads {
        for _ in 0..n {
            r.nes.step(frame(p))?;
            out.push(r.ram());
        }
    }
    r.load(&s)?;
    Ok(out)
}

/// The byte that follows a d-pad axis: it steps a few pixels at a time
/// with `plus` held and back with `minus` held, on most frames (a menu
/// cursor jumps, then stops; a player keeps going). Its score, and
/// whether it grows with `plus`.
fn axis_byte(r: &mut Run, plus: u8, minus: u8) -> Result<Option<(usize, i64, bool)>> {
    let a = trace(r, &[(plus, 40), (minus, 40)])?;
    let b = trace(r, &[(minus, 40), (plus, 40)])?;
    let score = |t: &[Vec<u8>], at: usize, first: i64| -> i64 {
        let mut moved = 0;
        for i in 1..t.len() {
            let d = wrap8(i64::from(t[i][at]) - i64::from(t[i - 1][at]));
            if d.abs() > 6 {
                return -1000;
            }
            let want = if i <= 40 { first } else { -first };
            if d != 0 {
                moved += if d.signum() == want { 1 } else { -2 };
            }
        }
        moved
    };
    let mut best: Option<(usize, i64, bool)> = None;
    for at in 0..0x800usize {
        if (0x100..0x200).contains(&at) {
            continue;
        }
        // Either sign: the byte may grow with `minus`.
        for sign in [1i64, -1] {
            // A wall may block one way at the start: the better order counts.
            let s = score(&a, at, sign).max(0) + score(&b, at, -sign).max(0);
            let rank = s + if at >= 0x10 { 5 } else { 0 };
            if std::env::var_os("GP_BLIND_DEBUG").is_some() && s >= 15 {
                eprintln!("axis {plus:02x}: ${at:04X} sign {sign} score {s}");
            }
            if s >= 30 && best.is_none_or(|(_, bs, _)| rank > bs) {
                best = Some((at, rank, sign == 1));
            }
        }
    }
    Ok(best)
}

/// The player's position bytes, when the d-pad moves something on both
/// axes the way a player moves.
fn player_bytes(r: &mut Run) -> Result<Option<(usize, usize, bool, bool)>> {
    let Some((x, _, xr)) = axis_byte(r, RIGHT, LEFT)? else {
        return Ok(None);
    };
    let Some((y, _, yd)) = axis_byte(r, DOWN, UP)? else {
        return Ok(None);
    };
    if x == y {
        return Ok(None);
    }
    Ok(Some((x, y, xr, yd)))
}

/// What a menu position gave: the class's speed (px a frame, best of the
/// four directions) and the tiles its sprite is drawn with.
#[derive(Debug, Clone)]
struct ClassSeen {
    menu: String,
    speed_x16: i64,
    tiles: Vec<u8>,
}

/// The sprite tiles within 24 px of the player's place on screen.
fn player_tiles(r: &Run) -> Vec<u8> {
    let oam = r.nes.oam();
    let mut t: BTreeSet<u8> = BTreeSet::new();
    let boxes = m::sprite_boxes(&oam, 8);
    // The player's sprites move with it; take the tiles of the sprites in
    // the biggest box group that has the most sprites near its middle.
    if boxes.is_empty() {
        return Vec::new();
    }
    for e in oam.chunks(4) {
        if e[0] < 0xEF {
            t.insert(e[1]);
        }
    }
    t.into_iter().collect()
}

/// From a state where the player answers the pad: its speed, in 16ths of
/// a pixel a frame, the best of the four directions over 32 frames.
fn speed_of(r: &mut Run, x: usize, y: usize) -> Result<i64> {
    let mut best = 0;
    for (p, at) in [(RIGHT, x), (LEFT, x), (DOWN, y), (UP, y)] {
        let t = trace(r, &[(p, 32)])?;
        let mut moved = 0i64;
        for i in 1..t.len() {
            moved += wrap8(i64::from(t[i][at]) - i64::from(t[i - 1][at])).abs();
        }
        best = best.max(moved * 16 / 32);
    }
    Ok(best)
}

/// Whether the d-pad changes RAM here beyond what changes with nothing
/// held: a menu that answers.
fn menu_answers(r: &mut Run) -> Result<bool> {
    let still = trace(r, &[(0, 30)])?;
    let changes_still: BTreeSet<usize> = (0..0x800)
        .filter(|&a| still.iter().any(|f| f[a] != still[0][a]))
        .collect();
    for p in [RIGHT, DOWN, LEFT, UP] {
        let t = trace(r, &[(p, 4), (0, 26)])?;
        let moved = (0..0x800).any(|a| {
            !changes_still.contains(&a) && !(0x100..0x200).contains(&a) && t[30][a] != t[0][a]
        });
        if moved {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What blind discovery found before play.
struct Start {
    run: Run,
    x: usize,
    y: usize,
    x_grows_right: bool,
    y_grows_down: bool,
    classes: Vec<ClassSeen>,
    menu: Option<String>,
    shots: Vec<PathBuf>,
}

/// The ways a menu cursor is tried: none, each direction, and twice right
/// or down.
const WAYS: [(&str, &[u8]); 7] = [
    ("none", &[]),
    ("right", &[RIGHT]),
    ("left", &[LEFT]),
    ("up", &[UP]),
    ("down", &[DOWN]),
    ("right x2", &[RIGHT, RIGHT]),
    ("down x2", &[DOWN, DOWN]),
];

/// A player found from a run: its bytes and directions.
type Found = (usize, usize, bool, bool);

/// From here, presses Start and A until the player answers the pad. A
/// screen where the d-pad changes RAM but no player moves is a menu: each
/// way of moving its cursor, then A, is tried, and the search goes on
/// from there (menus nest: a mode menu, then a class select). At the menu
/// where different ways give players that move at different speeds or are
/// drawn with different tiles, those are the classes.
fn reach_play(
    r: &mut Run,
    depth: usize,
    presses: usize,
    path: &str,
    classes: &mut Vec<ClassSeen>,
    menus: &mut Vec<String>,
) -> Result<Option<Found>> {
    for i in 0..presses {
        // A game may show its maze a while before the player can move.
        for _ in 0..3 {
            if let Some(p) = player_bytes(r)? {
                return Ok(Some(p));
            }
            r.hold(0, 40)?;
        }
        if depth > 0 && menu_answers(r)? {
            let at = r.save()?;
            let mut seen: Vec<(ClassSeen, Run, Found)> = Vec::new();
            for (name, way) in WAYS {
                r.load(&at)?;
                for p in way {
                    r.hold(*p, 4)?;
                    r.hold(0, 20)?;
                }
                r.hold(A, 4)?;
                r.hold(0, 40)?;
                let here = format!("{path}{}{name}", if path.is_empty() { "" } else { ", " });
                let mut sub = Vec::new();
                if let Some(f) = reach_play(r, depth - 1, 4, &here, &mut sub, menus)? {
                    let c = ClassSeen {
                        menu: here.clone(),
                        speed_x16: speed_of(r, f.0, f.1)?,
                        tiles: player_tiles(r),
                    };
                    let mut copy = Run {
                        nes: r.nes.clone(),
                        log: r.log.clone(),
                    };
                    copy.nes.set_video(false);
                    seen.push((c, copy, f));
                    classes.extend(sub);
                    // One way through is enough unless the ways differ:
                    // try them all at the first menu that leads to play.
                    if seen.len() >= 2
                        && seen.iter().all(|(c, _, _)| {
                            c.speed_x16 == seen[0].0.speed_x16 && c.tiles == seen[0].0.tiles
                        })
                    {
                        break;
                    }
                }
            }
            if !seen.is_empty() {
                menus.push(path.to_owned());
                let distinct: BTreeSet<(i64, Vec<u8>)> = seen
                    .iter()
                    .map(|(c, _, _)| (c.speed_x16, c.tiles.clone()))
                    .collect();
                if distinct.len() >= 2 {
                    classes.extend(seen.iter().map(|(c, _, _)| c.clone()));
                }
                let (_, run, f) = seen.swap_remove(0);
                *r = run;
                return Ok(Some(f));
            }
            r.load(&at)?;
        }
        let press = if i % 2 == 0 { START } else { A };
        r.hold(press, 4)?;
        r.hold(0, 90)?;
    }
    Ok(None)
}

/// Power-on to play, through any menus ([`reach_play`]).
fn play_start(bytes: &[u8], out: &Path) -> Result<Start> {
    let mut r = Run {
        nes: gp_nes::Nes::power_on(bytes, gp_evidence::manifest::PowerOn::default())?,
        log: Vec::new(),
    };
    r.nes.set_video(false);
    r.hold(0, 120)?;
    let mut classes = Vec::new();
    let mut menus = Vec::new();
    let Some((x, y, xr, yd)) = reach_play(&mut r, 3, 12, "", &mut classes, &mut menus)? else {
        let p = out.join("stuck.png");
        shot(&mut r, &p)?;
        bail!(
            "play start: nothing answered the d-pad like a player through Start, A and up to 3 menus (see {})",
            p.display()
        );
    };
    // The same class reached through other paths counts once: by the
    // menu's last step and what was seen.
    let mut seen_keys = BTreeSet::new();
    classes.retain(|c: &ClassSeen| {
        let last = c.menu.rsplit(", ").next().unwrap_or("").to_owned();
        seen_keys.insert((last, c.speed_x16, c.tiles.clone()))
    });
    let p = out.join("start.png");
    shot(&mut r, &p)?;
    Ok(Start {
        run: r,
        x,
        y,
        x_grows_right: xr,
        y_grows_down: yd,
        classes,
        menu: if menus.is_empty() {
            None
        } else {
            Some(format!("{} menu(s)", menus.len()))
        },
        shots: vec![p],
    })
}

/// A picture of the next frame, for looking by hand.
fn shot(r: &mut Run, to: &Path) -> Result<()> {
    let s = r.save()?;
    r.nes.set_video(true);
    r.nes.step(frame(0))?;
    let mut rgb = Box::new([0u8; 256 * 240 * 3]);
    r.nes.frame_rgb(&mut rgb);
    r.nes.set_video(false);
    r.load(&s)?;
    if let Some(d) = to.parent() {
        fs::create_dir_all(d)?;
    }
    let mut enc = png::Encoder::new(fs::File::create(to)?, 256, 240);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgb[..])?;
    Ok(())
}

/// Bytes that only go down while the player stands still for 20 s, in
/// several steps: a drain. Health is the one that went down most often.
fn drains(r: &mut Run) -> Result<Vec<(usize, usize)>> {
    let t = trace(r, &[(0, 1200)])?;
    let mut out = Vec::new();
    for a in 0..0x800usize {
        if (0x100..0x200).contains(&a) {
            continue;
        }
        let mut downs = 0;
        let mut ups = 0;
        for i in 1..t.len() {
            match t[i][a].cmp(&t[i - 1][a]) {
                std::cmp::Ordering::Less => downs += 1,
                std::cmp::Ordering::Greater => ups += 1,
                _ => {}
            }
        }
        // A digit of a decimal count wraps up once per ten steps down. A
        // drain is slow: a few steps a second at most (a byte that falls
        // most frames is a timer, not health).
        if (4..=60).contains(&downs) && ups * 5 <= downs {
            out.push((a, downs));
        }
    }
    out.sort_by_key(|&(a, d)| (std::cmp::Reverse(d), a));
    out.truncate(6);
    Ok(out)
}

/// A move: the d-pad held for some frames, maybe with a button.
#[derive(Clone, Copy)]
struct Move {
    name: &'static str,
    pad: u8,
    frames: usize,
    about: &'static str,
}

const MOVES: [Move; 11] = [
    Move {
        name: "n",
        pad: UP,
        frames: 16,
        about: "walk up",
    },
    Move {
        name: "ne",
        pad: UP | RIGHT,
        frames: 16,
        about: "walk up and right",
    },
    Move {
        name: "e",
        pad: RIGHT,
        frames: 16,
        about: "walk right",
    },
    Move {
        name: "se",
        pad: DOWN | RIGHT,
        frames: 16,
        about: "walk down and right",
    },
    Move {
        name: "s",
        pad: DOWN,
        frames: 16,
        about: "walk down",
    },
    Move {
        name: "sw",
        pad: DOWN | LEFT,
        frames: 16,
        about: "walk down and left",
    },
    Move {
        name: "w",
        pad: LEFT,
        frames: 16,
        about: "walk left",
    },
    Move {
        name: "nw",
        pad: UP | LEFT,
        frames: 16,
        about: "walk up and left",
    },
    Move {
        name: "shoot",
        pad: A,
        frames: 12,
        about: "fire a shot where the player faces",
    },
    Move {
        name: "magic",
        pad: B,
        frames: 12,
        about: "use the other button, often magic or a bomb",
    },
    Move {
        name: "wait",
        pad: 0,
        frames: 16,
        about: "stand still",
    },
];

/// A place reached: its savestate, and how it was reached.
struct Cell {
    snap: (Vec<u8>, u64, [u8; 4], usize),
    log: Vec<InputFrame>,
    visits: u32,
    maze: u32,
    /// The RAM there, for the key counter once it's known.
    ram: Vec<u8>,
}

type Key = (u32, i64, i64);

/// What the search sees of the player.
struct Probe {
    x: usize,
    y: usize,
    xh: Option<usize>,
    yh: Option<usize>,
    health: Option<usize>,
}

impl Probe {
    fn pos(&self, ram: &[u8]) -> (i64, i64) {
        let hi = |h: Option<usize>| h.map_or(0, |a| i64::from(ram[a]) * 256);
        (
            hi(self.xh) + i64::from(ram[self.x]),
            hi(self.yh) + i64::from(ram[self.y]),
        )
    }
}

/// How a move ended.
enum End {
    /// Still playing, at a cell of this maze.
    At(Key),
    /// The player died or stopped answering.
    Dead,
    /// A new maze loaded.
    NewMaze,
}

/// Whether two frames of RAM differ a lot: a new scene.
fn big_change(a: &[u8], b: &[u8]) -> bool {
    let n = (0..0x800)
        .filter(|&i| !(0x100..0x300).contains(&i) && a[i] != b[i])
        .count();
    n > 0x700 / 4
}

/// What the moves have shown so far, beyond places: votes for the high
/// bytes (a byte stepping by one, the right way, on a frame its low byte
/// wraps), and bytes that went up or down by one and stayed (a pickup
/// counted, a key used).
#[derive(Default)]
struct Seen {
    high: [BTreeMap<usize, i64>; 2],
    gains: BTreeMap<usize, u32>,
    uses: BTreeMap<usize, u32>,
}

impl Seen {
    /// The high byte for x (0) or y (1), once its votes are clear.
    fn high_of(&self, k: usize) -> Option<usize> {
        let mut v: Vec<(i64, usize)> = self.high[k].iter().map(|(a, n)| (*n, *a)).collect();
        v.sort_unstable_by(|a, b| b.cmp(a));
        match v.as_slice() {
            [(n, a), rest @ ..]
                if *n >= 3 && rest.first().is_none_or(|(m, _)| *n >= 2 * m.max(&1)) =>
            {
                Some(*a)
            }
            _ => None,
        }
    }

    /// The counter keys go in: a byte that has gone up by one and down by
    /// one, each more than once.
    fn key_counter(&self) -> Option<usize> {
        self.gains
            .iter()
            .filter(|(a, g)| **g >= 2 && self.uses.get(a).is_some_and(|u| *u >= 1))
            .max_by_key(|(_, g)| **g)
            .map(|(a, _)| *a)
    }
}

/// Plays one move from the run's state, watching for death and a new maze,
/// and noting what [`Seen`] collects.
fn play(r: &mut Run, p: &Probe, mv: &Move, maze: u32, seen: &mut Seen) -> Result<End> {
    let first = r.ram();
    let mut last = first.clone();
    let mut frames = vec![first.clone()];
    for _ in 0..mv.frames {
        r.hold(mv.pad, 1)?;
        let now = r.ram();
        if big_change(&last, &now) {
            // A new scene: wait 2 s and see whether the player answers in
            // it (a new maze) or not (a death, a game over).
            r.hold(0, 120)?;
            return Ok(if player_bytes(r)?.is_some() {
                End::NewMaze
            } else {
                End::Dead
            });
        }
        for (k, lo) in [p.x, p.y].into_iter().enumerate() {
            let d = i64::from(now[lo]) - i64::from(last[lo]);
            if d.abs() > 128 {
                let carry = -d.signum();
                for a in 0..0x800usize {
                    if a == lo || (0x100..0x200).contains(&a) {
                        continue;
                    }
                    let dh = wrap8(i64::from(now[a]) - i64::from(last[a]));
                    let v = seen.high[k].entry(a).or_default();
                    if dh == carry {
                        *v += 1;
                    } else if dh != 0 {
                        *v -= 1;
                    }
                }
            }
        }
        frames.push(now.clone());
        last = now;
    }
    // Bytes that stepped by one once in the move and held there to its end,
    // other than the player's own: a counter that went up or down.
    for a in 0..0x800usize {
        if [Some(p.x), Some(p.y), p.xh, p.yh].contains(&Some(a)) || (0x100..0x300).contains(&a) {
            continue;
        }
        let steps = frames.windows(2).filter(|w| w[0][a] != w[1][a]).count();
        if steps != 1 {
            continue;
        }
        match i64::from(last[a]) - i64::from(first[a]) {
            1 => *seen.gains.entry(a).or_default() += 1,
            -1 => *seen.uses.entry(a).or_default() += 1,
            _ => {}
        }
    }
    let (x, y) = p.pos(&last);
    Ok(End::At((maze, x.div_euclid(16), y.div_euclid(16))))
}

/// The high bytes of x and y: bytes that step by one, the right way, on
/// the frames the low byte wraps, seen over a long random walk.
fn high_bytes(
    r: &mut Run,
    x: usize,
    y: usize,
    rng: &mut Rng,
) -> Result<(Option<usize>, Option<usize>)> {
    let s = r.save()?;
    let mut votes: [BTreeMap<usize, i64>; 2] = [BTreeMap::new(), BTreeMap::new()];
    let mut last = r.ram();
    for _ in 0..240 {
        let mv = MOVES[(rng.next() % 8) as usize];
        for _ in 0..mv.frames {
            r.hold(mv.pad, 1)?;
            let now = r.ram();
            for (k, lo) in [x, y].into_iter().enumerate() {
                let d = i64::from(now[lo]) - i64::from(last[lo]);
                if d.abs() > 128 {
                    let carry = -d.signum();
                    for a in 0..0x800usize {
                        if a == lo || (0x100..0x200).contains(&a) {
                            continue;
                        }
                        let dh = wrap8(i64::from(now[a]) - i64::from(last[a]));
                        if dh == carry {
                            *votes[k].entry(a).or_default() += 1;
                        } else if dh != 0 {
                            *votes[k].entry(a).or_default() -= 1;
                        }
                    }
                }
            }
            last = now;
        }
    }
    r.load(&s)?;
    let pick = |v: &BTreeMap<usize, i64>| {
        v.iter()
            .filter(|(_, n)| **n >= 2)
            .max_by_key(|(a, n)| (**n, std::cmp::Reverse(**a)))
            .map(|(a, _)| *a)
    };
    Ok((pick(&votes[0]), pick(&votes[1])))
}

pub(crate) fn run(a: &TopdownArgs, paths: &Paths) -> Result<Outcome> {
    let t0 = std::time::Instant::now();
    let bytes = fs::read(&a.rom).with_context(|| format!("reading {}", a.rom.display()))?;
    crate::original::nes_only(
        crate::roms::Platform::of_file(&bytes),
        "the top-down farm's search",
    )?;
    let name = a
        .rom
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("rom")
        .replace(' ', "_");
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let out = a.out.clone().unwrap_or_else(|| {
        paths
            .home
            .join("work/farm/topdown")
            .join(&name)
            .join(ms.to_string())
    });
    fs::create_dir_all(&out)?;
    let mut rng = Rng(a.seed.max(1));

    // 1. Play start, through a class select if there is one.
    let mut st = play_start(&bytes, &out)?;
    let start_frames = st.run.log.len();
    // 2. The player's bytes, and their high bytes.
    let (xh, yh) = high_bytes(&mut st.run, st.x, st.y, &mut rng)?;
    // 3. Health: what drains while the player stands still.
    let drain = drains(&mut st.run)?;
    let mut probe = Probe {
        x: st.x,
        y: st.y,
        xh,
        yh,
        health: drain.first().map(|d| d.0),
    };

    // 4 and 5. Search: an archive of cells, a frontier cell each round, the
    // moves in the decision model's order (or at random), the top few
    // simulated.
    let mut cells: BTreeMap<Key, Cell> = BTreeMap::new();
    let p0 = probe.pos(&st.run.ram());
    let k0 = (0, p0.0.div_euclid(16), p0.1.div_euclid(16));
    cells.insert(
        k0,
        Cell {
            snap: st.run.save()?,
            log: st.run.log.clone(),
            visits: 0,
            maze: 0,
            ram: st.run.ram(),
        },
    );
    let criteria: serde_json::Map<String, Value> = MOVES
        .iter()
        .map(|mv| (mv.name.to_owned(), json!(mv.about)))
        .collect();
    let (mut rounds, mut sims, mut calls, mut deaths, mut mazes) = (0u64, 0u64, 0u64, 0u64, 0u32);
    let mut maze_logs: Vec<Vec<InputFrame>> = Vec::new();
    let mut seen = Seen::default();
    // The search's own time starts after discovery.
    let discovered = t0.elapsed().as_secs_f64();
    let t_search = std::time::Instant::now();
    let deadline = std::time::Duration::from_secs(a.seconds);
    while t_search.elapsed() < deadline {
        rounds += 1;
        if probe.xh.is_none() {
            probe.xh = seen.high_of(0);
        }
        if probe.yh.is_none() {
            probe.yh = seen.high_of(1);
        }
        let keyc = seen.key_counter();
        // The frontier: cells of the newest maze, fewest visits first,
        // weighted to the ones with unvisited neighbours.
        let newest = cells.values().map(|c| c.maze).max().unwrap_or(0);
        let mut pool: Vec<(Key, f64)> = cells
            .iter()
            .filter(|(_, c)| c.maze == newest)
            .map(|(k, c)| {
                let open = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                    .iter()
                    .filter(|(dx, dy)| !cells.contains_key(&(k.0, k.1 + dx, k.2 + dy)))
                    .count() as f64;
                // With a key in hand, a door can open: those cells count
                // double ("touch a door with a key").
                let keyed = keyc.is_some_and(|a| c.ram[a] > 0);
                (
                    *k,
                    (1.0 + open) * if keyed { 2.0 } else { 1.0 } / f64::from(c.visits + 1).sqrt(),
                )
            })
            .collect();
        let total: f64 = pool.iter().map(|p| p.1).sum();
        let mut pick = rng.unit() * total;
        let mut key = pool[0].0;
        for (k, w) in pool.drain(..) {
            pick -= w;
            key = k;
            if pick <= 0.0 {
                break;
            }
        }
        let (snap, log, maze) = {
            let c = cells.get_mut(&key).expect("a cell");
            c.visits += 1;
            (c.snap.clone(), c.log.clone(), c.maze)
        };
        st.run.load(&snap)?;
        st.run.log = log.clone();
        let order: Vec<&'static str> = if a.uniform {
            let mut o: Vec<&'static str> = MOVES.iter().map(|mv| mv.name).collect();
            for i in (1..o.len()).rev() {
                o.swap(i, (rng.next() % (i as u64 + 1)) as usize);
            }
            o
        } else {
            let ram = st.run.ram();
            let (x, y) = probe.pos(&ram);
            let open: Vec<&str> = [("n", 0, -1), ("e", 1, 0), ("s", 0, 1), ("w", -1, 0)]
                .iter()
                .filter(|(_, dx, dy)| !cells.contains_key(&(key.0, key.1 + dx, key.2 + dy)))
                .map(|(n, _, _)| *n)
                .collect();
            let state = json!({
                "goal": "explore the maze: reach places not yet visited, and find the exit",
                "unvisited_directions": open,
                "health": probe.health.map(|h| ram[h]),
                "position": [x, y],
                "maze": maze,
            });
            let ask = json!({"model": "laya", "state": state, "questions": {"move": {
                "type": "choice",
                "instructions": "Which move should the player try first, to reach a place not visited yet without dying?",
                "criteria": criteria}}});
            match post(
                &format!("{}/v1/systemone", a.decide.trim_end_matches('/')),
                &ask,
            ) {
                Ok(ans) => {
                    calls += 1;
                    let pr = &ans["answers"]["move"]["probabilities"];
                    let mut o: Vec<(&'static str, f64)> = MOVES
                        .iter()
                        .map(|mv| (mv.name, pr[mv.name].as_f64().unwrap_or(0.0)))
                        .collect();
                    o.sort_by(|a, b| b.1.total_cmp(&a.1));
                    o.into_iter().map(|(n, _)| n).collect()
                }
                Err(e) => bail!("the decision model: {e:#}"),
            }
        };
        for name in order.iter().take(a.top.max(1)) {
            let mv = MOVES.iter().find(|mv| mv.name == *name).expect("a move");
            st.run.load(&snap)?;
            st.run.log = log.clone();
            sims += 1;
            match play(&mut st.run, &probe, mv, maze, &mut seen)? {
                End::Dead => deaths += 1,
                End::NewMaze => {
                    mazes += 1;
                    maze_logs.push(st.run.log.clone());
                    let p = probe.pos(&st.run.ram());
                    let k = (maze + 1, p.0.div_euclid(16), p.1.div_euclid(16));
                    cells.entry(k).or_insert(Cell {
                        snap: st.run.save()?,
                        log: st.run.log.clone(),
                        visits: 0,
                        maze: maze + 1,
                        ram: st.run.ram(),
                    });
                    if mazes == 1 {
                        shot(&mut st.run, &out.join("maze2.png"))?;
                    }
                }
                End::At(k) => {
                    if let std::collections::btree_map::Entry::Vacant(e) = cells.entry(k) {
                        e.insert(Cell {
                            snap: st.run.save()?,
                            log: st.run.log.clone(),
                            visits: 0,
                            maze,
                            ram: st.run.ram(),
                        });
                    }
                }
            }
        }
    }
    // The run that got furthest: into the newest maze, then the most cells.
    let newest = cells.values().map(|c| c.maze).max().unwrap_or(0);
    let far = cells
        .iter()
        .filter(|(_, c)| c.maze == newest)
        .max_by_key(|(_, c)| c.log.len())
        .map(|(_, c)| c.log.clone())
        .unwrap_or_default();
    let text = gp_evidence::input_log::InputLog {
        frames: far,
        ..gp_evidence::input_log::InputLog::new(1)
    }
    .to_text();
    fs::write(out.join("path.log"), &text)?;
    if let Some((_, c)) = cells.iter().rfind(|(_, c)| c.maze == newest) {
        st.run.load(&c.snap)?;
        shot(&mut st.run, &out.join("furthest.png"))?;
    }
    // Health and death: from the cell reached last (deepest in the maze,
    // where the generators' enemies have had longest to gather), stand
    // still up to 100 s. The health candidates should fall faster than
    // when standing alone (enemies hurt), and a death is a scene change
    // after which no player answers, with health at or near 0.
    let busiest = cells
        .values()
        .filter(|c| c.maze == newest)
        .max_by_key(|c| c.log.len())
        .map(|c| c.snap.clone());
    let mut health_check = json!(null);
    if let Some(snap) = busiest {
        st.run.load(&snap)?;
        let cands: Vec<usize> = drain.iter().map(|d| d.0).take(4).collect();
        let at0: Vec<u8> = cands.iter().map(|&a| st.run.ram()[a]).collect();
        let mut last = st.run.ram();
        let mut downs = vec![0u32; cands.len()];
        let mut died = None;
        for f in 0..6000usize {
            st.run.hold(0, 1)?;
            let now = st.run.ram();
            for (i, &a) in cands.iter().enumerate() {
                if now[a] < last[a] {
                    downs[i] += 1;
                }
            }
            if big_change(&last, &now) {
                let at: Vec<u8> = cands.iter().map(|&a| last[a]).collect();
                st.run.hold(0, 120)?;
                let answers = player_bytes(&mut st.run)?.is_some();
                died =
                    Some(json!({"frame": f, "player_answers_after": answers, "health_before": at}));
                shot(&mut st.run, &out.join("after-death.png"))?;
                break;
            }
            last = now;
        }
        health_check = json!({
            "candidates": cands.iter().map(|a| format!("${a:04X}")).collect::<Vec<_>>(),
            "start": at0,
            "downs": downs,
            "scene_change": died,
        });
    }
    let per_maze: BTreeMap<u32, usize> = cells.values().fold(BTreeMap::new(), |mut m, c| {
        *m.entry(c.maze).or_default() += 1;
        m
    });
    let result = json!({
        "format": "farm-topdown 0.1",
        "rom": a.rom.display().to_string(),
        "orderer": if a.uniform { "uniform" } else { "laya" },
        "play_start_frames": start_frames,
        "class_menu": st.menu,
        "classes": st.classes.iter().map(|c| json!({"menu": c.menu, "speed_px16": c.speed_x16, "tiles": c.tiles.len()})).collect::<Vec<_>>(),
        "player": {"x": format!("${:04X}", st.x), "y": format!("${:04X}", st.y),
                   "x_high": xh.map(|v| format!("${v:04X}")), "y_high": yh.map(|v| format!("${v:04X}")),
                   "x_grows_right": st.x_grows_right, "y_grows_down": st.y_grows_down},
        "health": drain.iter().map(|(a, n)| json!({"byte": format!("${a:04X}"), "drops_in_20s": n})).collect::<Vec<_>>(),
        "rounds": rounds, "moves_simulated": sims, "laya_calls": calls, "deaths": deaths,
        "new_mazes": mazes, "cells": cells.len(), "cells_by_maze": per_maze,
        "seconds": t0.elapsed().as_secs_f64(),
        "high_votes_x": seen.high[0].iter().filter(|(_, n)| **n >= 2).map(|(a, n)| (format!("${a:04X}"), *n)).collect::<BTreeMap<_, _>>(),
        "high_votes_y": seen.high[1].iter().filter(|(_, n)| **n >= 2).map(|(a, n)| (format!("${a:04X}"), *n)).collect::<BTreeMap<_, _>>(),
        "high_used": [probe.xh.map(|v| format!("${v:04X}")), probe.yh.map(|v| format!("${v:04X}"))],
        "gains": seen.gains.iter().filter(|(_, n)| **n >= 2).map(|(a, n)| (format!("${a:04X}"), *n)).collect::<BTreeMap<_, _>>(),
        "uses": seen.uses.iter().filter(|(_, n)| **n >= 1).map(|(a, n)| (format!("${a:04X}"), *n)).collect::<BTreeMap<_, _>>(),
        "key_counter": seen.key_counter().map(|a| format!("${a:04X}")),
        "health_check": health_check,
        "seconds_discovery": discovered,
        "path": out.join("path.log").display().to_string(),
        "shots": st.shots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    });
    fs::write(
        out.join("result.json"),
        serde_json::to_string_pretty(&result)?,
    )?;
    let text = format!(
        "{}: play start in {} frames ({}), {} classes seen; player x {} y {} (high {:?} {:?}); health {}; {} rounds, {} cells over {} mazes, {} new mazes, {} deaths, {} Laya calls, {:.0} s. Written to {}",
        a.rom.display(),
        start_frames,
        st.menu.as_deref().unwrap_or("no menu"),
        st.classes.len(),
        result["player"]["x"],
        result["player"]["y"],
        xh,
        yh,
        probe
            .health
            .map_or("none".to_owned(), |h| format!("${h:04X}")),
        rounds,
        cells.len(),
        per_maze.len(),
        mazes,
        deaths,
        calls,
        t0.elapsed().as_secs_f64(),
        out.display()
    );
    Ok(Outcome::ok(result, text))
}
