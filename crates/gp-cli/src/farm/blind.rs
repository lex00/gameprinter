//! `gp farm --blind ROM`: the farm's search on a game it knows nothing
//! about. No ROM dataset, no template, no mapping: everything it uses is
//! learned from the running game.
//!
//! | What | How it is found |
//! |---|---|
//! | Play start | stage 0's start runs and its play-start rule (the first frame the pad moves something) |
//! | The player's x | the RAM byte that goes up under Right and down under Left, a few pixels a frame |
//! | The player's y and the jump button | the byte that A (else B) sends up and back down to where it was |
//! | The player's box | the sprite box that moves with the pad; its height is the power state |
//! | Forward and progress | the side the pad moves the player and the scroll goes; progress is the player's x summed frame by frame, plus the scroll when the x byte is a screen position |
//! | Scenes | the RAM bytes that hold still in play; a scene is their values, and a byte that changes in plain play is dropped from them |
//! | Death | the pad stops moving the player, then the player jumps back (a respawn) in the same scene, or nothing answers for 20 s |
//! | Level end | a jump to another scene; a scene the path later comes back from (a bonus room) isn't counted |
//! | Walls, pits | learned: where a move stops dead against something, and where a fall ended in a death |
//! | Two x bytes that move alike | on the board, the one the game copies from (lanes.rs); on the NES, the later address |
//! | No jump: lanes | a stick way that steps y by one amount and holds, and the lane number that changes with it (lanes.rs); then play starts at the first frame after the first Start press where the stick moves x, the held button, the score as progress, and moves from those |
//!
//! No game's addresses, constants or names are in this file.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::lanes::{self, Held, Lanes};
use super::machine::{Machine, Pads};
use super::support as m;
use super::{self as farm, A, B, LEFT, Move, RIGHT, Rng};
use gp_evidence::Paths;
use gp_evidence::input_log::InputFrame;

/// Frames a death or a scene change may take before play answers again.
const RECOVER: usize = 1200;
/// Progress in pixels a freeze needs before it counts as a level clear.
const CLEAR_MIN: i64 = 128;
/// A one-frame move of the x byte further than this is a jump (a respawn or
/// a new scene), not walking.
const TELEPORT: i64 = 24;
/// The first run to each wall, pit and landing, by (kind, scene, column,
/// row): its inputs from power-on and the frame it got there.
type FirstRuns = BTreeMap<(&'static str, usize, i64, i64), (Vec<InputFrame>, usize)>;

/// What the search was given to run with.
#[derive(Clone)]
pub(crate) struct Opts {
    pub decide: String,
    pub rounds: usize,
    pub top: usize,
    pub unsure: f64,
    pub frontier: i64,
    pub seed: u64,
    pub seconds: f64,
    pub levels: usize,
    /// A mechanic session, as GAME/MECHANIC, to show the search's progress
    /// in: its queue row and status line, and its best run so far as a
    /// clip the page loops.
    pub session: Option<String>,
}

/// A goal the search scores instead of progress (docs/research/farm-ledger.md,
/// "Probes"). Read from a probes file: a JSON list of objects with `id`,
/// `kind` and the kind's fields.
///
/// | kind | Reached when | Fields |
/// |---|---|---|
/// | touch | the player's box meets a sprite box of the shape from the side named | shape [w, h], side "above", "side" or "below" (any if left out) |
/// | jump_speed | the player leaves the ground going up at this speed or faster | speed (px a frame) |
/// | head_hit | a rise turns over by 3 px a frame at once (the head met something) | |
/// | stand_near | a sprite box of the shape stays within 32 px for this many frames | shape, frames |
/// | take_item | the player's box meets a box of the shape while its height (8 px steps) is this | shape, power |
#[derive(Clone, Debug)]
pub(crate) struct Probe {
    pub id: String,
    pub kind: String,
    pub shape: Option<[i64; 2]>,
    pub side: Option<String>,
    pub speed: Option<i64>,
    pub frames: usize,
    pub power: Option<i64>,
    pub seconds: Option<f64>,
}

impl Probe {
    fn from(v: &Value) -> Result<Probe> {
        let kind = v["kind"]
            .as_str()
            .context("a probe needs a kind")?
            .to_owned();
        if !["touch", "jump_speed", "head_hit", "stand_near", "take_item"].contains(&kind.as_str())
        {
            bail!("unknown probe kind {kind}");
        }
        Ok(Probe {
            id: v["id"].as_str().map_or_else(|| kind.clone(), str::to_owned),
            kind,
            shape: v["shape"]
                .as_array()
                .and_then(|a| Some([a.first()?.as_i64()?, a.get(1)?.as_i64()?])),
            side: v["side"].as_str().map(str::to_owned),
            speed: v["speed"].as_i64(),
            frames: v["frames"].as_u64().unwrap_or(60) as usize,
            power: v["power"].as_i64(),
            seconds: v["seconds"].as_f64(),
        })
    }
}

/// What was learned about the game before the search.
#[derive(Clone, Debug)]
struct Learned {
    x: u16,
    y: u16,
    /// The x byte is a place on the screen: progress adds the scroll.
    screen_x: bool,
    /// Right is forward (else Left).
    right: bool,
    jump: u8,
    run: u8,
    /// The player's box top less its y byte.
    box_dy: i64,
    /// The device's buttons.
    pads: Pads,
    /// A player who changes lanes instead of jumping (then `jump` is 0).
    lanes: Option<Lanes>,
    /// A button that acts after being held (then it is no run button).
    held: Option<Held>,
    /// The score's digits: progress where x doesn't advance.
    score: Option<Vec<usize>>,
}

/// Where a run is.
#[derive(Clone, Copy, Debug)]
struct Pos {
    /// Progress in pixels, summed frame by frame from the start.
    x: i64,
    xb: u8,
    y: i64,
    dx: i64,
    ground: bool,
    /// The player's box height, in 8 px steps: grown or small.
    tall: i64,
    /// The scroll, for summing it.
    scroll: i64,
    /// Where on the screen the player's box is (its left), if seen.
    sx: Option<i64>,
    /// The score, when progress is the score's (a lane player).
    sc: i64,
}

struct Cell {
    snap: Vec<u8>,
    frame: u64,
    held: [u8; 4],
    log: Vec<InputFrame>,
    pos: Pos,
    /// The scenes the path went through, in order (the last is this one's).
    scenes: Vec<usize>,
    visits: u32,
    failed: BTreeSet<&'static str>,
    /// The probe's distance at this cell (0 without a probe).
    dist: i64,
}

type Key = (usize, usize, i64, i64, bool, i64, i64);

fn key_of(scenes: &[usize], p: &Pos) -> Key {
    (
        scenes.len(),
        *scenes.last().unwrap_or(&0),
        p.x / 16,
        p.y / 16,
        p.ground,
        p.dx.abs().min(2),
        p.tall,
    )
}

enum End {
    At(Pos),
    Died,
    /// Play answers again in another scene: where, and the scene's values.
    Scene(Pos, Vec<u8>),
    /// The pad stopped moving the player and nothing came back, with the
    /// player held in place, not falling: a level clear that freezes.
    Clear(Pos),
    Stalled,
}

struct World {
    nes: Machine,
    l: Learned,
    /// The scene bytes: addresses that hold still in play.
    still: Vec<u16>,
    /// Each scene's values at those addresses.
    scenes: Vec<Vec<u8>>,
    /// Scene bytes a respawn changed: address -> (from, to, times, the
    /// path's frame of the first).
    died_bytes: BTreeMap<u16, (u8, u8, usize, usize)>,
    /// Scene bytes and others that changed between a clear's last move and
    /// its freeze: (address, from, to), and the path's frame.
    clear_bytes: ClearBytes,
    probe: Option<Probe>,
    /// Frames in a row a stand_near target has been close.
    near_run: usize,
    /// Where the probe was reached: the path frame and what was seen.
    probe_hit: Option<(usize, Value)>,
}

fn wrap8(d: i64) -> i64 {
    (d + 384).rem_euclid(256) - 128
}

impl World {
    fn wram(&self) -> &[u8] {
        &self.nes.wram()[..0x800]
    }

    /// The NES's stack and sprite buffer pages; nothing on the board.
    fn noise_page(&self, a: usize) -> bool {
        !self.nes.is_arcade() && (0x100..0x300).contains(&a)
    }

    fn byte(&self, a: u16) -> i64 {
        i64::from(self.wram()[usize::from(a)])
    }

    fn scroll(&self) -> i64 {
        self.nes.scroll()
    }

    fn step(&mut self, pad: u8, log: &mut Vec<InputFrame>) -> Result<()> {
        let f = InputFrame {
            polled: true,
            pads: [pad, 0, 0, 0],
            reset: false,
        };
        self.nes.step(f)?;
        log.push(f);
        Ok(())
    }

    /// The player's box: the sprite box whose top sits `box_dy` from the y
    /// byte, nearest the last place it was.
    fn player_box(&self, near: Option<i64>) -> Option<[i32; 4]> {
        let boxes = self.nes.boxes();
        let y = self.screen_y(self.byte(self.l.y));
        boxes
            .into_iter()
            .filter(|b| (i64::from(b[1]) - y - self.l.box_dy).abs() <= 12)
            .min_by_key(|b| near.map_or(0, |n| (i64::from(b[0]) - n).abs()))
    }

    /// A y byte where the screen has it: the other way round for a y that
    /// counts up the screen.
    fn screen_y(&self, y: i64) -> i64 {
        if self.l.lanes.as_ref().is_some_and(|l| l.y_up) {
            -y
        } else {
            y
        }
    }

    /// The score now, where progress is the score's.
    fn score(&self) -> Option<i64> {
        self.l
            .score
            .as_ref()
            .filter(|_| self.l.lanes.is_some())
            .map(|d| lanes::score_of(self.wram(), d))
    }

    /// Whether x jumped further than walking goes since `p`: a respawn or
    /// a new scene, unless y changed lanes on the same frame (a lane
    /// player lands at the new lane's place).
    fn jumped(&self, p: &Pos) -> bool {
        let xb = self.byte(self.l.x);
        wrap8(xb - i64::from(p.xb)).abs() > TELEPORT
            && !(self.l.lanes.is_some() && self.byte(self.l.y) != p.y)
    }

    /// The position after one more frame, from the one before.
    fn advance(&self, p: &Pos) -> Pos {
        let xb = self.byte(self.l.x) as u8;
        let y = self.byte(self.l.y);
        let scroll = self.scroll();
        let mut d = wrap8(i64::from(xb) - i64::from(p.xb));
        if d.abs() > TELEPORT {
            d = 0;
        }
        let ds = {
            let s = (scroll - p.scroll).rem_euclid(512);
            if s > 256 { s - 512 } else { s }
        };
        let fwd = if self.l.right { 1 } else { -1 };
        let mut gain = d * fwd + if self.l.screen_x { ds * fwd } else { 0 };
        // A lane player's progress is the score.
        let sc = self.score().unwrap_or(0);
        if self.score().is_some() {
            gain = sc - p.sc;
        }
        let b = self.player_box(p.sx);
        Pos {
            x: p.x + gain,
            xb,
            y,
            dx: gain,
            ground: y == p.y,
            tall: b.map_or(p.tall, |b| i64::from(b[3]) / 8),
            scroll,
            sx: b.map(|b| i64::from(b[0])),
            sc,
        }
    }

    fn here(&self, x: i64) -> Pos {
        let b = self.player_box(None);
        Pos {
            x,
            xb: self.byte(self.l.x) as u8,
            y: self.byte(self.l.y),
            dx: 0,
            ground: true,
            tall: b.map_or(2, |b| i64::from(b[3]) / 8),
            scroll: self.scroll(),
            sx: b.map(|b| i64::from(b[0])),
            sc: self.score().unwrap_or(0),
        }
    }

    fn scene_values(&self) -> Vec<u8> {
        self.still
            .iter()
            .map(|a| self.wram()[usize::from(*a)])
            .collect()
    }

    /// The known scene these values are, if one is within 2 bytes.
    fn scene_of(&self, v: &[u8]) -> Option<usize> {
        self.scenes
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.iter().zip(v).filter(|(a, b)| a != b).count()))
            .inspect(|(i, n)| {
                if std::env::var_os("GP_BLIND_DEBUG").is_some() {
                    eprintln!("scene {i}: {n} of {} bytes differ", v.len());
                }
            })
            .filter(|(_, n)| *n <= 2)
            .min_by_key(|(_, n)| *n)
            .map(|(i, _)| i)
    }

    /// The sprite boxes other than the player's, and the player's.
    fn boxes(&self, p: &Pos) -> (Vec<[i64; 4]>, Option<[i64; 4]>) {
        let all: Vec<[i64; 4]> = self
            .nes
            .boxes()
            .into_iter()
            .map(|b| {
                [
                    i64::from(b[0]),
                    i64::from(b[1]),
                    i64::from(b[2]),
                    i64::from(b[3]),
                ]
            })
            .collect();
        let me = self.player_box(p.sx).map(|b| {
            [
                i64::from(b[0]),
                i64::from(b[1]),
                i64::from(b[2]),
                i64::from(b[3]),
            ]
        });
        (all.into_iter().filter(|b| Some(*b) != me).collect(), me)
    }

    /// The probe's targets on screen: boxes of its shape (within 4 px).
    fn targets(&self, p: &Pos) -> (Vec<[i64; 4]>, Option<[i64; 4]>) {
        let (others, me) = self.boxes(p);
        let shape = self.probe.as_ref().and_then(|pr| pr.shape);
        let t = others
            .into_iter()
            .filter(|b| shape.is_none_or(|[w, h]| (b[2] - w).abs() <= 4 && (b[3] - h).abs() <= 4))
            .collect();
        (t, me)
    }

    /// How far the run is from the probe's goal: the gap between the
    /// player's box and the nearest target, or less progress when the goal
    /// isn't about a sprite (or none is on screen).
    fn probe_dist(&self, p: &Pos) -> i64 {
        let Some(pr) = &self.probe else { return 0 };
        let shaped = matches!(pr.kind.as_str(), "touch" | "stand_near" | "take_item");
        if shaped {
            let (t, me) = self.targets(p);
            if let Some(me) = me
                && let Some(d) = t
                    .iter()
                    .map(|b| {
                        ((b[0] + b[2] / 2) - (me[0] + me[2] / 2)).abs()
                            + ((b[1] + b[3] / 2) - (me[1] + me[3] / 2)).abs()
                    })
                    .min()
            {
                return d;
            }
        }
        100_000 - p.x
    }

    /// Whether this frame reached the probe's goal; records it.
    fn check_probe(&mut self, p: &Pos, q: &Pos, prev_dy: i64, at: usize) {
        let Some(pr) = self.probe.clone() else { return };
        let dy = q.y - p.y;
        let hit: Option<Value> = match pr.kind.as_str() {
            "jump_speed" => (p.ground && dy < 0 && q.dx.abs() >= pr.speed.unwrap_or(1))
                .then(|| json!({"took_off_at_px_a_frame": q.dx, "rise": dy})),
            "head_hit" => (prev_dy < 0 && dy >= 0 && dy - prev_dy >= 3)
                .then(|| json!({"rise_then": prev_dy, "then": dy, "y": q.y})),
            "touch" | "take_item" => {
                let (t, me) = self.targets(q);
                me.and_then(|b| {
                    t.iter().find_map(|o| {
                        let ox = (b[0] + b[2]).min(o[0] + o[2]) - b[0].max(o[0]);
                        let oy = (b[1] + b[3]).min(o[1] + o[3]) - b[1].max(o[1]);
                        let side = if ox > 0 && (b[1] + b[3] - o[1]).abs() <= 4 && dy > 0 {
                            "above"
                        } else if ox > 0 && (o[1] + o[3] - b[1]).abs() <= 4 && dy < 0 {
                            "below"
                        } else if oy > 0
                            && ((b[0] + b[2] - o[0]).abs() <= 3 || (o[0] + o[2] - b[0]).abs() <= 3)
                        {
                            "side"
                        } else {
                            return None;
                        };
                        let want = pr.side.as_deref().is_none_or(|w| w == side);
                        let power =
                            pr.kind != "take_item" || pr.power.is_none_or(|pw| pw == q.tall);
                        (want && power).then(
                            || json!({"side": side, "player": b, "target": o, "tall": q.tall}),
                        )
                    })
                })
            }
            "stand_near" => {
                let (t, me) = self.targets(q);
                let close = me.is_some_and(|b| {
                    t.iter()
                        .any(|o| (o[0] - b[0]).abs() <= 32 && (o[1] - b[1]).abs() <= 32)
                });
                self.near_run = if close { self.near_run + 1 } else { 0 };
                (self.near_run >= pr.frames).then(|| json!({"frames_near": self.near_run}))
            }
            _ => None,
        };
        if let Some(h) = hit {
            self.probe_hit = Some((at, h));
        }
    }

    /// The scene a state before a move was in: the nearest known one.
    fn scene_of_before(&self, v: &[u8]) -> Option<usize> {
        self.scenes
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.iter().zip(v).filter(|(a, b)| a != b).count()))
            .min_by_key(|(_, n)| *n)
            .map(|(i, _)| i)
    }

    /// Whether the player stands here: y holds for `n` frames with nothing
    /// pressed. Gives those frames' inputs; the console is put back.
    fn stands(&mut self, n: usize) -> Result<Option<Vec<InputFrame>>> {
        let (snap, frame, held) = (self.nes.snapshot()?, self.nes.frame(), self.nes.held());
        let y0 = self.byte(self.l.y);
        let mut out = Vec::new();
        let mut ok = true;
        for _ in 0..n {
            self.step(0, &mut out)?;
            if self.byte(self.l.y) != y0 {
                ok = false;
                break;
            }
        }
        self.nes.restore(&snap, frame, held)?;
        Ok(ok.then_some(out))
    }

    /// Whether 8 frames of forward and 8 of back end in different places;
    /// the console is put back as it was.
    fn responsive(&mut self) -> Result<bool> {
        let (snap, frame, held) = (self.nes.snapshot()?, self.nes.frame(), self.nes.held());
        let mut ends = Vec::new();
        for pad in [self.l.pads.right, self.l.pads.left] {
            self.nes.restore(&snap, frame, held)?;
            let mut scratch = Vec::new();
            for _ in 0..8 {
                self.step(pad, &mut scratch)?;
            }
            ends.push((self.byte(self.l.x), self.byte(self.l.y)));
        }
        self.nes.restore(&snap, frame, held)?;
        Ok(ends[0] != ends[1])
    }

    /// A pad of the farm's moves for this game: its jump and run buttons,
    /// and forward for Right.
    fn pad(&self, p: u8) -> u8 {
        let mut o = 0;
        if p & A != 0 {
            o |= self.l.jump;
            // A lane player's A is the held button.
            if let Some(h) = &self.l.held {
                o |= h.button;
            }
        }
        if let Some(ln) = &self.l.lanes {
            if p & farm::UP != 0 {
                o |= ln.up;
            }
            if p & farm::DOWN != 0 {
                o |= ln.down;
            }
        }
        if p & B != 0 {
            o |= self.l.run;
        }
        let (r, lft) = (p & RIGHT != 0, p & LEFT != 0);
        let (r, lft) = if self.l.right { (r, lft) } else { (lft, r) };
        if r {
            o |= self.l.pads.right;
        }
        if lft {
            o |= self.l.pads.left;
        }
        o
    }

    /// Runs a move from `p`; the inputs go on `log`.
    fn play(&mut self, mv: &Move, p0: &Pos, log: &mut Vec<InputFrame>) -> Result<End> {
        let ram_before = self.wram().to_vec();
        let mut p = *p0;
        let mut still = 0;
        let mut frames = 0;
        let mut teleported = false;
        let mut seq: Vec<(u8, usize)> = mv.press.clone();
        self.near_run = 0;
        // Standing near something for N frames needs a wait that long.
        if mv.name == "wait"
            && let Some(pr) = self.probe.as_ref().filter(|pr| pr.kind == "stand_near")
        {
            seq = vec![(0, pr.frames + 4)];
        }
        if mv.settle > 0 {
            seq.push((mv.then, mv.settle));
        }
        let mut left_ground = false;
        let mut prev_dy = 0i64;
        'outer: for (k, &(pad, n)) in seq.iter().enumerate() {
            let settling = mv.settle > 0 && k + 1 == seq.len();
            for _ in 0..n {
                let pad = self.pad(pad);
                self.step(pad, log)?;
                frames += 1;
                if self.jumped(&p) {
                    teleported = true;
                    break 'outer;
                }
                let q = self.advance(&p);
                if self.probe.is_some() && self.probe_hit.is_none() {
                    self.check_probe(&p, &q, prev_dy, log.len());
                }
                prev_dy = q.y - p.y;
                still = if q.y == p.y { still + 1 } else { 0 };
                if still == 0 {
                    left_ground = true;
                }
                p = q;
                if settling && left_ground && still >= 2 {
                    break 'outer;
                }
            }
        }
        // A lane player stalls only walking: a lane change or a pour may
        // move nothing the rule reads and still have happened.
        let can_stall = self.l.lanes.is_none() || matches!(mv.name, "walk" | "back");
        if !teleported && frames > 0 && p.x == p0.x && p.y == p0.y && mv.name != "wait" && can_stall
        {
            return Ok(End::Stalled);
        }
        if !teleported && self.responsive()? {
            p.ground = still >= 1;
            return Ok(End::At(p));
        }
        // The pad no longer moves the player, or it jumped: dying, a scene
        // change, or a cut scene. Play on with nothing held until the pad
        // answers again, then see where it is.
        let before = self.scene_values();
        let (y_at, mut y_far) = (p.y, 0i64);
        let (mut y_last, mut fell_off) = (p.y, false);
        for i in 0..RECOVER {
            self.step(0, log)?;
            let xb = self.byte(self.l.x);
            if self.jumped(&p) {
                teleported = true;
            }
            if !teleported {
                p = self.advance(&p);
            } else {
                p.xb = xb as u8;
            }
            y_far = y_far.max((self.byte(self.l.y) - y_at).abs());
            // Off the bottom of the screen: low on it, or wrapped round
            // from low to high.
            let yy = self.byte(self.l.y);
            if !teleported && (yy >= 224 || (y_last > 200 && yy < 32)) {
                fell_off = true;
            }
            y_last = yy;
            if i % 30 == 29 && self.responsive()? {
                let v = self.scene_values();
                let same = v.iter().zip(&before).filter(|(a, b)| a != b).count() <= 2;
                if !teleported {
                    p.ground = true;
                    return Ok(End::At(p));
                }
                if same || fell_off {
                    // A fall off the screen and back is a respawn, however
                    // much the reload changed: this scene's values become
                    // the respawned ones.
                    if fell_off && !same {
                        let cur = self.scene_of_before(&before);
                        if let Some(c) = cur {
                            self.scenes[c] = v.clone();
                        }
                    }
                    for (k, (a, b)) in v.iter().zip(&before).enumerate() {
                        if a != b {
                            let e = self.died_bytes.entry(self.still[k]).or_insert((
                                *b,
                                *a,
                                0,
                                log.len(),
                            ));
                            e.2 += 1;
                        }
                    }
                    return Ok(End::Died);
                }
                let mut q = self.here(0);
                q.x = 0;
                return Ok(End::Scene(q, v));
            }
        }
        // Nothing answered: a death that ended the game moved the player
        // off (it fell or hopped away); a clear that freezes held it still.
        // A clear also comes after real progress: half a screen or more
        // from where play started, reached by this move (a freeze at the
        // start, or one that follows no movement, is a cut scene or a
        // death that holds the player still).
        if !teleported && y_far <= 8 && p.x >= CLEAR_MIN && p.x > p0.x {
            if self.clear_bytes.is_none() {
                let now = self.wram();
                let d: Vec<(u16, u8, u8)> = (0..0x800usize)
                    .filter(|&a| !self.noise_page(a) && now[a] != ram_before[a])
                    .filter(|&a| self.still.contains(&(a as u16)) || ram_before[a] < 8)
                    .map(|a| (a as u16, ram_before[a], now[a]))
                    .collect();
                self.clear_bytes = Some((d, log.len()));
            }
            return Ok(End::Clear(p));
        }
        Ok(End::Died)
    }
}

/// A byte found on the path, and how many cases it was tested on.
type Found = Option<(u16, usize)>;
/// Bytes that changed at a clear, (address, from, to), and the path frame.
type ClearBytes = Option<(Vec<(u16, u8, u8)>, usize)>;

/// What blind discovery found: the console at play start (settled), the
/// inputs from power-on that reached it, and the player's bytes, controls
/// and forward. Reads nothing under $GP_HOME/work.
pub(crate) struct Discovery {
    pub nes: Machine,
    pub log: Vec<InputFrame>,
    pub x: u16,
    pub y: u16,
    pub screen_x: bool,
    pub right: bool,
    pub jump: u8,
    pub run: u8,
    pub box_dy: i64,
    /// The device's buttons.
    pub pads: Pads,
    /// A lane player's lanes, its held button and the score (Wave T).
    pub lanes: Option<Lanes>,
    pub held: Option<Held>,
    pub score: Option<Vec<usize>>,
    /// The power-on the inputs replay from, and the start run used.
    pub power_on: gp_evidence::manifest::PowerOn,
    pub start_run: String,
}

/// Play start (stage 0's rule, no dataset), 240 frames with nothing held,
/// then `learn`. An error starts "play start" or "player" for the stage
/// that failed.
pub(crate) fn discover(bytes: &[u8], paths: &Paths) -> Result<Discovery> {
    let arcade = crate::original::Platform::of_file(bytes) == crate::original::Platform::Arcade;
    let starts = crate::reach::Starts {
        runs: 4,
        seed: 1,
        start_frames: if arcade {
            gp_explore::arcade::START_FRAMES
        } else {
            gp_explore::presses::START_FRAMES
        },
        no_starts: false,
    };
    // A person's real-input path into play (a clip's path and start, as
    // GP_BLIND_START_PATH and GP_BLIND_START_FRAME) stands in for stage 0's
    // play start: the assisted way in, for games whose start runs don't
    // reach play.
    let given = match (
        std::env::var("GP_BLIND_START_PATH"),
        std::env::var("GP_BLIND_START_FRAME"),
    ) {
        (Ok(sha), Ok(n)) => {
            let n: usize = n
                .parse()
                .context("GP_BLIND_START_FRAME is a frame number")?;
            let log = m::read_path(&paths.home, &sha)?;
            if log.frames.len() < n {
                bail!(
                    "play start: the given path has {} frames, not {n}",
                    log.frames.len()
                );
            }
            Some((log.frames[..n].to_vec(), sha))
        }
        _ => None,
    };
    let (mut log, power_on, start_run) = match given {
        Some((log, sha)) => (
            log,
            gp_evidence::manifest::PowerOn::default(),
            format!("path {sha}"),
        ),
        None => {
            let clips = starts.with_platform(&[], paths, arcade)?;
            let (ci, t_start) =
                m::play_start(bytes, &clips)?.context("play start: no start run reaches play")?;
            (
                clips[ci].inputs[..t_start].to_vec(),
                clips[ci].power_on,
                clips[ci].id.clone(),
            )
        }
    };
    let mut nes = Machine::power_on(bytes, power_on)?;
    let pads = nes.pads();
    for f in &log {
        nes.step(*f)?;
    }
    // Settle: nothing held for 4 s, so the player stands.
    for _ in 0..240 {
        let f = InputFrame {
            polled: true,
            pads: [0, 0, 0, 0],
            reset: false,
        };
        nes.step(f)?;
        log.push(f);
    }
    // A menu, a stage select or a cut scene may come before play: press
    // Start (then A) and wait, until the pad moves a player.
    let mut tries = 0;
    if let Ok(d) = std::env::var("GP_BLIND_SHOTS") {
        shot(&mut nes, &Path::new(&d).join("start.png"))?;
    }
    let l = loop {
        match learn(&mut nes) {
            Ok(l) => break l,
            Err(e) if tries >= 16 => return Err(e).context("player"),
            Err(_) => {
                tries += 1;
                if let Ok(d) = std::env::var("GP_BLIND_SHOTS") {
                    shot(&mut nes, &Path::new(&d).join(format!("try{tries:02}.png")))?;
                }
                // On a board that takes coins, a coin goes in first.
                let first = pads.actions.first().copied().unwrap_or(0);
                let press = if tries % 2 == 1 { pads.start } else { first };
                let mut seq = vec![(press, 2), (0, 180)];
                if tries % 2 == 1 && pads.coin != 0 {
                    seq.splice(0..0, [(pads.coin, 6), (0, 54)]);
                }
                for (pad, n) in seq {
                    for _ in 0..n {
                        let f = InputFrame {
                            polled: true,
                            pads: [pad, 0, 0, 0],
                            reset: false,
                        };
                        nes.step(f)?;
                        log.push(f);
                    }
                }
            }
        }
    };
    let mut l = l;
    if let Some(ln) = l.lanes.clone() {
        lane_start(&mut nes, &mut log, bytes, power_on, &l)?;
        if let Ok(d) = std::env::var("GP_BLIND_SHOTS") {
            shot(&mut nes, &Path::new(&d).join("lane-start.png"))?;
            eprintln!("lane start at frame {}", log.len());
        }
        let at = (nes.snapshot()?, nes.frame(), nes.held());
        let (x, y) = (usize::from(l.x), usize::from(l.y));
        let pads = l.pads.clone();
        let skip: Vec<usize> = [Some(x), Some(y), ln.lane].into_iter().flatten().collect();
        l.held = lanes::find_held(
            &mut nes,
            &at,
            pads.actions.first().copied().unwrap_or(0),
            &skip,
        )?;
        // Play that should score, each lane on its own run from here (a
        // miss can cost a life, and a game over's demo writes its own
        // numbers): the button held until it acts and let go, twice.
        let act = l
            .held
            .as_ref()
            .map_or((pads.actions.first().copied().unwrap_or(0), 2), |h| {
                (h.button, h.frames + 1)
            });
        let plays: Vec<Vec<(u8, usize)>> = (0..4)
            .map(|k| {
                let mut p = Vec::new();
                for _ in 0..k {
                    p.push((ln.up, 10));
                    p.push((0, 10));
                }
                for _ in 0..2 {
                    p.push(act);
                    p.push((0, 40));
                }
                p
            })
            .collect();
        l.score = lanes::find_score(&mut nes, &at, &plays)?;
    }
    if std::env::var_os("GP_BLIND_DEBUG").is_some() {
        let b = nes.ram_base();
        eprintln!(
            "learned: x ${:04X} y ${:04X} right {} jump {:#x} run {:#x} box_dy {} lanes {:?} held {:?} score {:?} (RAM from ${b:04X})",
            usize::from(l.x) + b,
            usize::from(l.y) + b,
            l.right,
            l.jump,
            l.run,
            l.box_dy,
            l.lanes,
            l.held,
            l.score
        );
    }
    Ok(Discovery {
        nes,
        log,
        x: l.x,
        y: l.y,
        screen_x: l.screen_x,
        right: l.right,
        jump: l.jump,
        run: l.run,
        box_dy: l.box_dy,
        pads: l.pads,
        lanes: l.lanes,
        held: l.held,
        score: l.score,
        power_on,
        start_run,
    })
}

/// A picture of the next frame, for looking by hand (GP_BLIND_SHOTS).
fn shot(nes: &mut Machine, to: &Path) -> Result<()> {
    nes.shot(to)
}

/// For a lane player, the earliest start in the run that reached play:
/// the play-start rule takes the first frame the pad moved the player in a
/// start run, which on a board can come long after the game began (the
/// stage 0 runs press nothing for minutes) with lives already lost; and
/// discover's settle waits 4 s more. From the first Start press on, every
/// 10 frames, the first state where 8 frames of Right and of Left end x in
/// different places is play's real start: the machine and its inputs are
/// put there. The first Start press, not the last: a press in a game
/// already going does nothing.
fn lane_start(
    nes: &mut Machine,
    log: &mut Vec<InputFrame>,
    bytes: &[u8],
    power_on: gp_evidence::manifest::PowerOn,
    l: &Learned,
) -> Result<()> {
    let Some(last_start) = log
        .iter()
        .position(|f| f.pads[0] & l.pads.start != 0 && l.pads.start != 0)
    else {
        return Ok(());
    };
    let mut m = Machine::power_on(bytes, power_on)?;
    for (i, f) in log.iter().enumerate() {
        m.step(*f)?;
        if i > last_start && (i - last_start) % 10 == 0 {
            let (snap, frame, held) = (m.snapshot()?, m.frame(), m.held());
            // Nothing, Right and Left: x answers the pad when Right ends it
            // no further left than nothing does, Left no further right, and
            // the two differ (an intro that moves x moves it all three ways
            // alike).
            let mut ends = Vec::new();
            for pad in [0, l.pads.right, l.pads.left] {
                m.restore(&snap, frame, held)?;
                for _ in 0..8 {
                    m.step(InputFrame {
                        polled: true,
                        pads: [pad, 0, 0, 0],
                        reset: false,
                    })?;
                }
                ends.push(m.wram()[usize::from(l.x)]);
            }
            m.restore(&snap, frame, held)?;
            let d = |a: u8, b: u8| wrap8(i64::from(a) - i64::from(b));
            if d(ends[1], ends[0]) >= 0 && d(ends[2], ends[0]) <= 0 && ends[1] != ends[2] {
                log.truncate(i + 1);
                *nes = m;
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Learns the player's bytes, the jump button and forward from play start.
fn learn(nes: &mut Machine) -> Result<Learned> {
    // A slow walker (a pixel every few frames) moves too little in 40-frame
    // holds to be told from noise: only when nothing is found at 40, try
    // holds three times as long.
    match learn_with(nes, 40) {
        Ok(l) => Ok(l),
        Err(e) if format!("{e}").starts_with("no RAM byte moves") => learn_with(nes, 120),
        Err(e) => Err(e),
    }
}

/// `learn` with each hold of Right and Left this many frames long.
fn learn_with(nes: &mut Machine, hold: usize) -> Result<Learned> {
    let (snap, frame, held) = (nes.snapshot()?, nes.frame(), nes.held());
    let run = |nes: &mut Machine, pads: &[(u8, usize)]| -> Result<Vec<Vec<u8>>> {
        Ok(run_seen(nes, &snap, frame, held, pads)?.0)
    };
    // x: up under Right then down under Left (or the other way at a wall),
    // a few pixels a frame at most, moving on most frames, and a sprite
    // drawn at one offset from it (less the scroll, or not) on 85% of the
    // frames: a counter or a scratch byte that happens to move with the pad
    // has no sprite with it.
    let pads = nes.pads();
    let (rt, lt) = (pads.right, pads.left);
    let (r, r_seen) = run_seen(nes, &snap, frame, held, &[(rt, hold), (lt, hold)])?;
    let (l, l_seen) = run_seen(nes, &snap, frame, held, &[(lt, hold), (rt, hold)])?;
    let score = |t: &[Vec<u8>], a: usize, first: i64| -> (i64, i64) {
        let mut moved = 0;
        let mut net = 0;
        for i in 1..t.len() {
            let d = wrap8(i64::from(t[i][a]) - i64::from(t[i - 1][a]));
            if d.abs() > 8 {
                return (0, 0);
            }
            let want = if i <= hold { first } else { -first };
            if d != 0 {
                if d.signum() == want {
                    moved += 1;
                } else {
                    moved -= 2;
                }
            }
            net += d * want;
        }
        (moved, net)
    };
    let drawn = |t: &[Vec<u8>], seen: &[(Vec<i64>, i64)], a: usize| -> bool {
        // An offset from the first frame's boxes, held within 3 px (the
        // sprites may be drawn a frame behind) on 85% of the frames.
        for with_scroll in [false, true] {
            let v = |i: usize| i64::from(t[i][a]) - if with_scroll { seen[i].1 } else { 0 };
            for b0 in &seen[0].0 {
                let off = (b0 - v(0)).rem_euclid(256);
                let ok = (1..t.len())
                    .filter(|&i| {
                        seen[i].0.iter().any(|bx| {
                            [v(i), v(i - 1)].iter().any(|vv| {
                                let d = (bx - vv - off).rem_euclid(256);
                                d <= 3 || d >= 253
                            })
                        })
                    })
                    .count();
                if ok * 100 >= (t.len() - 1) * 85 {
                    return true;
                }
            }
        }
        false
    };
    let mut cands: Vec<(i64, usize, bool)> = Vec::new();
    for a in 0..0x800 {
        if nes.noise(a) {
            continue;
        }
        let (mr, nr) = score(&r, a, 1);
        let (ml, nl) = score(&l, a, -1);
        // Moving one way may be blocked at the start; the other has to move.
        let s = mr.max(0) + ml.max(0);
        if std::env::var_os("GP_BLIND_DEBUG").is_some() && s >= 20 {
            eprintln!(
                "x candidate ${:04X}: moves {s} (net {nr} {nl}), drawn {} {}",
                a + nes.ram_base(),
                drawn(&r, &r_seen, a),
                drawn(&l, &l_seen, a)
            );
        }
        // A sprite drawn with it counts for it, but isn't needed: the
        // scroll read from the PPU can be another write's by the frame's end.
        if s >= 30 && nr.max(nl) > 0 {
            let seen = i64::from(drawn(&r, &r_seen, a)) + i64::from(drawn(&l, &l_seen, a));
            cands.push((s + seen * 1000, a, nr >= nl));
        }
    }
    // Of those, the one that gets furthest in 10 s of going forward with
    // jumps: a byte for the place on the screen stops where the screen
    // starts to scroll, and the place in the level goes on.
    // The first 16 bytes of RAM are where NES programs keep their scratch
    // values (a copy of the player's x made while drawing it, say): left
    // out when anything else qualifies.
    if cands.iter().any(|c| !nes.scratch(c.1)) {
        cands.retain(|c| !nes.scratch(c.1));
    }
    let mut best: Option<(i64, usize, bool)> = None;
    let mut ranked: Vec<(i64, usize, bool)> = Vec::new();
    for &(s0, a, right) in &cands {
        let fwd = if right { rt } else { lt };
        // The device's second action held (B, run, on the NES), and its
        // first (A, jump) pressed on and off.
        let (a1, a2) = (
            pads.actions.first().copied().unwrap_or(0),
            pads.actions.get(1).copied().unwrap_or(0),
        );
        let held_pads: Vec<(u8, usize)> = (0..15)
            .flat_map(|_| [(fwd | a2, 20), (fwd | a2 | a1, 20)])
            .collect();
        let t = run(nes, &held_pads)?;
        let mut far = 0i64;
        let mut gone = 0i64;
        for i in 1..t.len() {
            let d = wrap8(i64::from(t[i][a]) - i64::from(t[i - 1][a]));
            if d.abs() <= TELEPORT {
                gone += d * if right { 1 } else { -1 };
                far = far.max(gone);
            }
        }
        // Furthest first, then drawn, then most agreeing moves.
        let rank = far * 1_000_000 + (s0 / 1000) * 1000 + s0 % 1000;
        // On a tie, the later address: a zero-page byte that copies the
        // player's x each frame is a scratch copy of the table's.
        if best.is_none_or(|(b, _, _)| rank >= b) {
            best = Some((rank, a, right));
        }
        ranked.push((s0, a, right));
    }
    // Bytes that move alike under Right and Left move together: the one
    // the game copies from, by the board's access log (none on the NES,
    // where the furthest, then the later address, stands).
    if let Some((_, b, _)) = best
        && let Some(&(s_best, _, _)) = ranked.iter().find(|c| c.1 == b)
    {
        let tied: Vec<(i64, usize, bool)> =
            ranked.iter().copied().filter(|c| c.0 == s_best).collect();
        if tied.len() > 1 {
            let at = (snap.clone(), frame, held);
            let walk = [(lt, hold), (rt, hold)];
            let addrs: Vec<usize> = tied.iter().map(|c| c.1).collect();
            if let Some(src) = lanes::copy_source(nes, &at, &walk, &addrs)? {
                best = best.map(|(rk, _, r)| {
                    (rk, src, tied.iter().find(|c| c.1 == src).map_or(r, |c| c.2))
                });
            }
        }
    }
    let (_, x, right) =
        best.context("no RAM byte moves with Right and Left: the player wasn't found")?;
    // Forward: the side it can go further.
    let reach = |t: &[Vec<u8>]| -> i64 {
        (1..=hold)
            .map(|i| wrap8(i64::from(t[i][x]) - i64::from(t[i - 1][x])))
            .sum::<i64>()
    };
    let right = if reach(&r) != -reach(&l) {
        reach(&r) >= -reach(&l)
    } else {
        right
    };
    // y and the jump button: a byte the button sends away and back to
    // where it was, a few pixels a frame.
    let mut found = None;
    for (jump, other) in pads.jump_tries() {
        let t = run(nes, &[(jump, 24), (0, 72)])?;
        let mut best: Option<(i64, usize)> = None;
        for a in 0..0x800 {
            if a == x {
                continue;
            }
            let v0 = i64::from(t[0][a]);
            let ok_steps = (1..t.len())
                .all(|i| wrap8(i64::from(t[i][a]) - i64::from(t[i - 1][a])).abs() <= 10);
            if !ok_steps || i64::from(t[t.len() - 1][a]) != v0 {
                continue;
            }
            let ext = (0..t.len())
                .map(|i| -wrap8(i64::from(t[i][a]) - v0))
                .max()
                .unwrap_or(0);
            let moving = (1..t.len()).filter(|&i| t[i][a] != t[i - 1][a]).count() as i64;
            // Past the first 16 bytes (scratch) and the stack, unless
            // nothing else qualifies.
            let rank = ext
                + if !nes.scratch(a) && !nes.noise(a) {
                    10_000
                } else {
                    0
                };
            if ext >= 12 && moving >= 10 && best.is_none_or(|(b, _)| rank > b) {
                best = Some((rank, a));
            }
        }
        if let Some((_, a)) = best {
            found = Some((a, jump, other));
            break;
        }
    }
    // No jump: a player who changes lanes, with a button held to act.
    let at = (snap.clone(), frame, held);
    let mut lanes_found = None;
    if found.is_none() {
        lanes_found = lanes::find_lanes(nes, &at, &pads, x)?;
        if let Some(ln) = &lanes_found {
            found = Some((ln.y, 0, 0));
        }
    }
    let (y, jump, run_b) = found.with_context(|| {
        format!(
            "no button sends the player up and back down: no jump found, and no stick way changes a lane (x at ${:04X}, forward {})",
            x + nes.ram_base(),
            if right { "right" } else { "left" }
        )
    })?;
    let y_sign = if lanes_found.as_ref().is_some_and(|l| l.y_up) {
        -1
    } else {
        1
    };
    // The held button and the score are found from discover's start.
    let (held_found, score) = (None, None);
    // The player's box: the one that moves with the pad, at one offset from y.
    nes.restore(&snap, frame, held)?;
    let boxes0 = nes.boxes();
    let fwd = if right { rt } else { lt };
    for _ in 0..12 {
        nes.step(InputFrame {
            polled: true,
            pads: [fwd, 0, 0, 0],
            reset: false,
        })?;
    }
    let y1 = y_sign * i64::from(nes.wram()[y]);
    let boxes1 = nes.boxes();
    let moved = boxes1
        .iter()
        .filter(|b| !boxes0.contains(b))
        .min_by_key(|b| (i64::from(b[1]) - y1).abs())
        .or_else(|| boxes1.iter().min_by_key(|b| (i64::from(b[1]) - y1).abs()));
    let box_dy = moved.map_or(0, |b| i64::from(b[1]) - y1);
    // Screen x: the scroll moves while the x byte stands still. Run
    // forward with jumps for 10 s and see.
    nes.restore(&snap, frame, held)?;
    let (mut with, mut without) = (0, 0);
    let mut last = (i64::from(nes.wram()[x]), 0i64);
    for i in 0..600 {
        let p = fwd | run_b | if i % 40 < 20 { jump } else { 0 };
        nes.step(InputFrame {
            polled: true,
            pads: [p, 0, 0, 0],
            reset: false,
        })?;
        let s = nes.scroll();
        let xv = i64::from(nes.wram()[x]);
        if i > 0 && s != last.1 {
            if xv != last.0 {
                with += 1;
            } else {
                without += 1;
            }
        }
        last = (xv, s);
    }
    nes.restore(&snap, frame, held)?;
    Ok(Learned {
        x: x as u16,
        y: y as u16,
        screen_x: without > with * 2,
        right,
        jump,
        run: run_b,
        box_dy,
        pads,
        lanes: lanes_found,
        held: held_found,
        score,
    })
}

/// A run from a savestate: each frame's RAM, and its sprite boxes' x and
/// the scroll.
#[allow(clippy::type_complexity)]
fn run_seen(
    nes: &mut Machine,
    snap: &[u8],
    frame: u64,
    held: [u8; 4],
    pads: &[(u8, usize)],
) -> Result<(Vec<Vec<u8>>, Vec<(Vec<i64>, i64)>)> {
    nes.restore(snap, frame, held)?;
    let seen = |nes: &Machine| (nes.drawn_xs(), nes.scroll());
    let mut out = vec![nes.wram()[..0x800].to_vec()];
    let mut vis = vec![seen(nes)];
    for &(p, n) in pads {
        for _ in 0..n {
            nes.step(InputFrame {
                polled: true,
                pads: [p, 0, 0, 0],
                reset: false,
            })?;
            out.push(nes.wram()[..0x800].to_vec());
            vis.push(seen(nes));
        }
    }
    Ok((out, vis))
}

/// The bytes that hold still in 10 s of play from here: the scene's bytes.
fn still_bytes(nes: &mut Machine, l: &Learned) -> Result<Vec<u16>> {
    let (snap, frame, held) = (nes.snapshot()?, nes.frame(), nes.held());
    let w0 = nes.wram()[..0x800].to_vec();
    let mut moved = vec![false; 0x800];
    let fwd = if l.right { l.pads.right } else { l.pads.left };
    for i in 0..600 {
        let p = match (i / 60) % 4 {
            0 => fwd,
            1 => fwd | l.run | if i % 30 < 15 { l.jump } else { 0 },
            2 => 0,
            _ => {
                if l.right {
                    l.pads.left
                } else {
                    l.pads.right
                }
            }
        };
        nes.step(InputFrame {
            polled: true,
            pads: [p, 0, 0, 0],
            reset: false,
        })?;
        for (a, mv) in moved.iter_mut().enumerate() {
            if nes.wram()[a] != w0[a] {
                *mv = true;
            }
        }
    }
    nes.restore(&snap, frame, held)?;
    // The stack and the sprite buffer change around anything.
    Ok((0..0x800u16)
        .filter(|a| !moved[usize::from(*a)] && !nes.noise(usize::from(*a)))
        .collect())
}

/// The search's moves for a lane player (in the farm's pad bits, which
/// `World::pad` turns into the game's): `act` frames hold the button
/// until it acts.
fn lane_moves(act: usize) -> Vec<Move> {
    let mv = |name, about, press: Vec<(u8, usize)>| Move {
        name,
        about,
        press,
        then: 0,
        settle: 0,
    };
    vec![
        mv(
            "walk",
            "walk forward along the lane a short way",
            vec![(RIGHT, 16)],
        ),
        mv(
            "back",
            "walk back along the lane a short way",
            vec![(LEFT, 16)],
        ),
        mv(
            "lane_up",
            "move to the next lane up",
            vec![(farm::UP, 8), (0, 8)],
        ),
        mv(
            "lane_down",
            "move to the next lane down",
            vec![(farm::DOWN, 8), (0, 8)],
        ),
        mv(
            "act",
            "hold the button until it acts, let go to send what it made, and watch it go",
            // Long enough after the release for what was sent to arrive:
            // the score moves then, and a cell kept sooner wouldn't show it.
            vec![(A, act), (0, 45)],
        ),
        mv(
            "press",
            "press the button and let go at once",
            vec![(A, 2), (0, 12)],
        ),
        mv("wait", "stand still and let things move", vec![(0, 16)]),
    ]
}

/// One blind farm of a ROM. Returns the result JSON.
pub(crate) fn farm(rom: &Path, o: &Opts, paths: &Paths, out_dir: &Path) -> Result<Value> {
    farm_for(rom, o, None, paths, out_dir)
}

/// Shows a running search in a mechanic session, through `gp mechanic`'s
/// own commands, so the page's queue, status line and clip viewer follow it.
struct Publish {
    exe: PathBuf,
    game: String,
    mech: String,
    dir: PathBuf,
    last: Option<std::time::Instant>,
    clips: usize,
}

impl Publish {
    fn new(session: &str, paths: &Paths, rom_sha: &str) -> Result<Publish> {
        let (game, mech) = session
            .split_once('/')
            .context("--session is GAME/MECHANIC")?;
        let dir = paths.home.join("mechanics").join(game).join(mech);
        fs::create_dir_all(dir.join("clips"))?;
        let st = dir.join("session.toml");
        if !st.is_file() {
            let mut t = toml::Table::new();
            t.insert("game".into(), game.into());
            t.insert("mechanic".into(), mech.into());
            t.insert("prompt".into(), "the blind search, live".into());
            t.insert("rom".into(), rom_sha.into());
            fs::write(&st, toml::to_string(&t)?)?;
        }
        let clips = fs::read_dir(dir.join("clips"))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .count();
        Ok(Publish {
            exe: std::env::current_exe()?,
            game: game.to_owned(),
            mech: mech.to_owned(),
            dir,
            last: None,
            clips,
        })
    }

    fn gp(&self, args: &[&str]) {
        // Best effort: the search goes on if the page's files can't be written.
        let _ = std::process::Command::new(&self.exe)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    fn status(&self, line: &str, done: bool) {
        let st = if done { "done" } else { "agent working" };
        self.gp(&[
            "mechanic", "queue", &self.game, &self.mech, "--status", st, "--note", line,
        ]);
        if !done {
            self.gp(&["mechanic", "busy", &self.game, &self.mech, line]);
        }
    }

    /// The run so far as a clip (a stored path, then the frames from play
    /// start); at most every 30 s
    /// unless `now`. Returns the clip's number.
    fn best(
        &mut self,
        paths: &Paths,
        rom: &[u8],
        log: &[InputFrame],
        play_frame: usize,
        now: bool,
    ) -> Result<Option<usize>> {
        if !now && self.last.is_some_and(|t| t.elapsed().as_secs() < 30) {
            return Ok(None);
        }
        if log.len() <= play_frame {
            return Ok(None);
        }
        self.last = Some(std::time::Instant::now());
        let mut il = gp_evidence::input_log::InputLog::new(1);
        il.frames = log[..play_frame].to_vec();
        let (sha, _) = m::put_path(&paths.home, &il.to_text())?;
        let (mut t, text) = m::new_clip(&paths.home, rom, &sha, play_frame, &log[play_frame..])?;
        t.insert("by".into(), "agent".into());
        t.insert("note".into(), "the blind search's best run so far".into());
        self.clips += 1;
        let n = self.clips;
        let clips = self.dir.join("clips");
        fs::write(clips.join(format!("{n:03}.log")), text)?;
        fs::write(clips.join(format!("{n:03}.toml")), toml::to_string(&t)?)?;
        // game-printer measured the clip for the page's overlays here with
        // `gp mechanic contacts`, which gameprinter hasn't got yet.
        Ok(Some(n))
    }
}

/// A blind farm toward a probe's goal, or for progress without one.
fn farm_for(
    rom: &Path,
    o: &Opts,
    probe: Option<&Probe>,
    paths: &Paths,
    out_dir: &Path,
) -> Result<Value> {
    let t0 = std::time::Instant::now();
    let bytes = fs::read(rom).with_context(|| format!("no ROM at {}", rom.display()))?;
    let sha256 = super::machine::rom_sha256(&bytes)?;
    fs::create_dir_all(out_dir)?;
    let mut flog = fs::File::create(out_dir.join("log.ndjson"))?;
    let fail = |stage: &str, why: String, secs: f64| {
        json!({"rom": rom.display().to_string(), "sha256": sha256, "stage": stage,
               "error": why, "levels": 0, "seconds": secs})
    };

    let d = match discover(&bytes, paths) {
        Ok(d) => d,
        Err(e) => {
            let msg = format!("{e:#}");
            let stage = if msg.starts_with("play start") {
                "play start"
            } else {
                "player"
            };
            return Ok(fail(stage, msg, t0.elapsed().as_secs_f64()));
        }
    };
    let learned = Learned {
        x: d.x,
        y: d.y,
        screen_x: d.screen_x,
        right: d.right,
        jump: d.jump,
        run: d.run,
        box_dy: d.box_dy,
        pads: d.pads.clone(),
        lanes: d.lanes.clone(),
        held: d.held.clone(),
        score: d.score.clone(),
    };
    let (mut nes, log) = (d.nes, d.log);
    let clip = crate::reach::ClipInput {
        set: String::new(),
        id: d.start_run.clone(),
        power_on: d.power_on,
        inputs: Vec::new(),
    };
    let still = still_bytes(&mut nes, &learned)?;
    let mut w = World {
        nes,
        l: learned.clone(),
        still,
        scenes: Vec::new(),
        died_bytes: BTreeMap::new(),
        clear_bytes: None,
        probe: probe.cloned(),
        near_run: 0,
        probe_hit: None,
    };
    w.scenes.push(w.scene_values());
    let start_pos = w.here(0);
    let play_frame = log.len();

    // Hold lengths: the person's clips if there are any, else defaults.
    let holds: Vec<usize> = farm::a_holds(&paths.home.join("mechanics"))
        .into_iter()
        .filter(|h| *h <= 40)
        .collect();
    let pick = |q: f64, or: usize| {
        if holds.is_empty() {
            or
        } else {
            holds[((holds.len() - 1) as f64 * q) as usize]
        }
    };
    let (hop, full) = (pick(0.0, 3).max(2), pick(1.0, 32).max(8));
    // A lane player's moves come from what discovery found: walking along
    // the lane, a lane up or down, the held button until it acts and let
    // go, a press, and a wait.
    let moves = match (&w.l.lanes, &w.l.held) {
        (Some(_), held) => lane_moves(held.as_ref().map_or(2, |h| h.frames + 1)),
        _ => farm::moves(hop, ((hop + full) / 2).max(hop + 1), full),
    };

    let mut cells: BTreeMap<Key, Cell> = BTreeMap::new();
    let first = vec![0usize];
    cells.insert(
        key_of(&first, &start_pos),
        Cell {
            snap: w.nes.snapshot()?,
            frame: w.nes.frame(),
            held: w.nes.held(),
            log: log.clone(),
            pos: start_pos,
            scenes: first,
            visits: 0,
            failed: BTreeSet::new(),
            dist: w.probe_dist(&start_pos),
        },
    );
    let mut best_dist = cells.values().map(|c| c.dist).min().unwrap_or(0);
    let mut probe_won: Option<(Vec<InputFrame>, Value, usize, f64)> = None;
    let uniform = o.decide == "uniform";
    let mut rng = Rng(o.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let plan = if w.l.lanes.is_some() {
        "Score as many points as possible without losing a life. The player changes lanes and walks along them; holding the button acts after a while and letting go sends what it made."
    } else {
        "Get the player as far forward as possible without dying. Jump over gaps and walls. Avoid what hurts; stomp or jump over enemies."
    };
    let criteria: serde_json::Map<String, Value> = moves
        .iter()
        .map(|mv| (mv.name.to_owned(), json!(mv.about)))
        .collect();
    // Learned places: (scene, 16 px column) where a move stopped dead, and
    // where a move began that ended in a death.
    let mut walls: BTreeSet<(usize, i64)> = BTreeSet::new();
    let mut pits: BTreeSet<(usize, i64)> = BTreeSet::new();
    let (mut calls, mut sims, mut deaths) = (0usize, 0usize, 0usize);
    let mut rounds = 0;
    let mut raised: BTreeSet<(usize, i64, i64)> = BTreeSet::new();
    // The first run to each wall, pit and landing: (kind, scene, column,
    // row) -> its inputs from power-on and the frame it got there.
    let mut first: FirstRuns = BTreeMap::new();
    // Each scene reached for the first time: (round, seconds).
    let mut scene_times: Vec<(usize, usize, f64)> = Vec::new();
    let mut best_rank: (usize, i64) = (1, 0);
    // A clear that freezes the game ends the search: (round, seconds, x).
    let mut clears: Vec<(usize, f64, i64)> = Vec::new();
    let mut cleared: Option<Vec<InputFrame>> = None;
    // The page's view of the search, when asked for and when the run's
    // clips can be replayed from a plain power-on.
    let mut publish = match &o.session {
        Some(sess)
            if probe.is_none()
                && clip.power_on == gp_evidence::manifest::PowerOn::default()
                && !w.nes.is_arcade() =>
        {
            Some(Publish::new(sess, paths, &sha256)?)
        }
        _ => None,
    };
    let progress = |round: usize,
                    best: (usize, i64),
                    deaths: usize,
                    raised: usize,
                    cells: usize| {
        format!(
            "round {round} of {}, furthest x {}, {} scene(s), {cells} places, {raised} ledges, {deaths} deaths",
            o.rounds, best.1, best.0
        )
    };
    if let Some(p) = &publish {
        p.status(&progress(0, best_rank, 0, 0, cells.len()), false);
    }
    for round in 0..o.rounds {
        if t0.elapsed().as_secs_f64() > o.seconds {
            break;
        }
        rounds = round + 1;
        if let Some(p) = &publish
            && round > 0
            && round % 50 == 0
        {
            p.status(
                &progress(round, best_rank, deaths, raised.len(), cells.len()),
                false,
            );
        }
        // The frontier: the deepest scene path, near its furthest x.
        let weights: Vec<(Key, f64)> = cells
            .iter()
            .map(|(k, c)| {
                let near = if probe.is_some() {
                    // The cells nearest the probe's goal.
                    if c.dist <= best_dist + 32 { 20.0 } else { 1.0 }
                } else if c.scenes.len() == best_rank.0 && c.pos.x + o.frontier >= best_rank.1 {
                    20.0
                } else if c.scenes.len() < best_rank.0 {
                    0.05
                } else {
                    1.0
                };
                (*k, near / f64::from(c.visits + 1).sqrt())
            })
            .collect();
        let total: f64 = weights.iter().map(|(_, w)| w).sum();
        let mut t = rng.unit() * total;
        let mut key = weights[0].0;
        for (k, wt) in &weights {
            t -= wt;
            if t <= 0.0 {
                key = *k;
                break;
            }
        }
        let (snap, frame, held, base_log, pos, scenes, failed) = {
            let c = cells.get_mut(&key).expect("picked");
            c.visits += 1;
            (
                c.snap.clone(),
                c.frame,
                c.held,
                c.log.clone(),
                c.pos,
                c.scenes.clone(),
                c.failed.clone(),
            )
        };
        let scene = *scenes.last().expect("a scene");
        w.nes.restore(&snap, frame, held)?;
        let order: (Vec<&'static str>, f64) = if uniform {
            let mut o: Vec<&'static str> = moves.iter().map(|mv| mv.name).collect();
            for i in (1..o.len()).rev() {
                o.swap(i, (rng.next() % (i as u64 + 1)) as usize);
            }
            (o, 1.0)
        } else {
            let col = pos.x / 16;
            let ahead = |set: &BTreeSet<(usize, i64)>| {
                (1..=10)
                    .find(|d| set.contains(&(scene, col + d)))
                    .map_or(json!("none known"), |d| {
                        json!(format!("{} px ahead", d * 16 - pos.x % 16))
                    })
            };
            // Sprites ahead that aren't the player.
            let boxes = w.nes.boxes();
            let mut thing: Option<(i64, i64)> = None;
            if let Some(sx) = pos.sx {
                let fwd = if w.l.right { 1 } else { -1 };
                for b in &boxes {
                    let d = (i64::from(b[0]) - sx) * fwd;
                    let dy = i64::from(b[1]) - (w.screen_y(pos.y) + w.l.box_dy);
                    if d <= 8 || d > 160 || dy.abs() > 64 {
                        continue;
                    }
                    if thing.is_none_or(|(e, _)| d < e) {
                        thing = Some((d, dy));
                    }
                }
            }
            let speed = pos.dx.abs().max(1);
            let st = json!({
                "plan": plan,
                "on_ground": pos.ground,
                "speed_px_a_frame": pos.dx,
                "pit_ahead": ahead(&pits),
                "wall_ahead": ahead(&walls),
                "sprite_ahead": thing.map_or(json!("none"), |(d, dy)| json!(format!("{d} px ahead, {}, {} frames away", if dy < -8 { "higher up" } else if dy > 8 { "lower down" } else { "level with the player" }, d / speed))),
                "failed_here": failed.iter().collect::<Vec<_>>(),
            });
            let ask = json!({"model": "laya", "state": st, "questions": {"move": {
                "type": "choice",
                "instructions": "Which move should the player try first from this moment, to get further forward without dying?",
                "criteria": criteria}}});
            let ans = farm::post(
                &format!("{}/v1/systemone", o.decide.trim_end_matches('/')),
                &ask,
            )?;
            calls += 1;
            let q = &ans["answers"]["move"];
            let pr = &q["probabilities"];
            let mut ord: Vec<(&'static str, f64)> = moves
                .iter()
                .map(|mv| (mv.name, pr[mv.name].as_f64().unwrap_or(0.0)))
                .collect();
            ord.sort_by(|a, b| b.1.total_cmp(&a.1));
            (
                ord.into_iter().map(|(n, _)| n).collect(),
                q["answer_confidence"].as_f64().unwrap_or(0.0),
            )
        };
        // Laya's top 3 (top 5 when it's unsure): the rest are rarely
        // better and cost a simulation each.
        let n = if order.1 < o.unsure {
            (o.top + 2).min(moves.len())
        } else {
            o.top.min(moves.len())
        };
        let mut tried = Vec::new();
        for name in order.0.iter().take(n) {
            let mv = moves.iter().find(|mv| mv.name == *name).expect("a move");
            w.nes.restore(&snap, frame, held)?;
            let mut l = base_log.clone();
            sims += 1;
            let end = w.play(mv, &pos, &mut l)?;
            if let Some((at, seen)) = w.probe_hit.take() {
                l.truncate(at);
                tried.push(json!({"move": name, "end": "probe reached"}));
                probe_won = Some((l, seen, round, t0.elapsed().as_secs_f64()));
                break;
            }
            let (new_pos, new_scenes) = match end {
                End::Died => {
                    deaths += 1;
                    pits.insert((scene, pos.x / 16 + 1));
                    first
                        .entry(("pit", scene, pos.x / 16 + 1, 0))
                        .or_insert_with(|| (l.clone(), l.len()));
                    cells.get_mut(&key).map(|c| c.failed.insert(mv.name));
                    tried.push(json!({"move": name, "end": "died"}));
                    continue;
                }
                End::Stalled => {
                    walls.insert((scene, pos.x / 16 + 1));
                    first
                        .entry(("wall", scene, pos.x / 16 + 1, 0))
                        .or_insert_with(|| (l.clone(), l.len()));
                    cells.get_mut(&key).map(|c| c.failed.insert(mv.name));
                    tried.push(json!({"move": name, "end": "stalled"}));
                    continue;
                }
                End::At(p) => {
                    // A plain-play stretch: scene bytes that changed aren't
                    // scene bytes.
                    let v = w.scene_values();
                    let keep: Vec<bool> = v
                        .iter()
                        .zip(&w.scenes[scene])
                        .map(|(a, b)| a == b)
                        .collect();
                    if keep.iter().any(|k| !k) {
                        let mut i = 0;
                        w.still.retain(|_| {
                            let k = keep[i];
                            i += 1;
                            k
                        });
                        for s in &mut w.scenes {
                            let mut i = 0;
                            s.retain(|_| {
                                let k = keep[i];
                                i += 1;
                                k
                            });
                        }
                    }
                    // A landing is standing: y holds for 10 frames with
                    // nothing pressed (a jump's top holds for 6 at most).
                    // The run kept for it includes those frames.
                    if p.ground
                        && p.y < start_pos.y - 8
                        && !raised.contains(&(scene, p.x / 16, p.y / 16))
                        && let Some(stood) = w.stands(10)?
                    {
                        raised.insert((scene, p.x / 16, p.y / 16));
                        let mut run = l.clone();
                        run.extend(stood);
                        first
                            .entry(("landing", scene, p.x / 16, p.y / 16))
                            .or_insert_with(|| (run, l.len()));
                    }
                    (p, scenes.clone())
                }
                End::Clear(p) => {
                    clears.push((round, t0.elapsed().as_secs_f64(), p.x));
                    tried.push(json!({"move": name, "end": "clear"}));
                    cleared = Some(l);
                    break;
                }
                End::Scene(_, ref v) if w.scene_of(v) == Some(scene) => {
                    // Back at the start of this same scene: a respawn.
                    deaths += 1;
                    pits.insert((scene, pos.x / 16 + 1));
                    first
                        .entry(("pit", scene, pos.x / 16 + 1, 0))
                        .or_insert_with(|| (l.clone(), l.len()));
                    cells.get_mut(&key).map(|c| c.failed.insert(mv.name));
                    tried.push(json!({"move": name, "end": "died"}));
                    continue;
                }
                End::Scene(p, v) => {
                    let id = match w.scene_of(&v) {
                        Some(i) => i,
                        None => {
                            w.scenes.push(v);
                            scene_times.push((
                                w.scenes.len() - 1,
                                round,
                                t0.elapsed().as_secs_f64(),
                            ));
                            w.scenes.len() - 1
                        }
                    };
                    let mut s = scenes.clone();
                    s.push(id);
                    tried.push(json!({"move": name, "end": "scene", "scene": id}));
                    (p, s)
                }
            };
            let k = key_of(&new_scenes, &new_pos);
            let better = cells.get(&k).is_none_or(|c| l.len() < c.log.len());
            if better {
                let visits = cells.get(&k).map_or(0, |c| c.visits);
                let rank = (new_scenes.len(), new_pos.x);
                if rank > best_rank {
                    best_rank = rank;
                    if let Some(p) = publish.as_mut() {
                        p.best(paths, &bytes, &l, play_frame, false)?;
                    }
                }
                cells.insert(
                    k,
                    Cell {
                        snap: w.nes.snapshot()?,
                        frame: w.nes.frame(),
                        held: w.nes.held(),
                        log: l,
                        pos: new_pos,
                        scenes: new_scenes,
                        visits,
                        failed: BTreeSet::new(),
                        dist: w.probe_dist(&new_pos),
                    },
                );
                if let Some(c) = cells.get(&k) {
                    best_dist = best_dist.min(c.dist);
                }
            }
            if tried.len() < 4 {
                tried.push(json!({"move": name, "x": new_pos.x, "new": better}));
            }
        }
        writeln!(
            flog,
            "{}",
            json!({"round": round, "scene": scene, "from_x": pos.x, "best": [best_rank.0, best_rank.1],
                   "cells": cells.len(), "confidence": order.1, "tried": tried})
        )?;
        if probe_won.is_some()
            || (probe.is_none() && (cleared.is_some() || levels_of(&best_path(&cells)) >= o.levels))
        {
            break;
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    let path = best_path(&cells);
    let levels = levels_of(&path) + clears.len().min(1);
    // Seconds to each level end along the best path: when its scene was
    // first reached.
    let level_scenes = new_scenes_of(&path);
    let level_times: Vec<Value> = level_scenes
        .iter()
        .filter_map(|s| scene_times.iter().find(|(id, _, _)| id == s))
        .map(|(id, r, t)| json!({"scene": id, "round": r, "seconds": (t * 10.0).round() / 10.0}))
        .chain(clears.iter().take(1).map(
            |(r, t, x)| json!({"clear_at_x": x, "round": r, "seconds": (t * 10.0).round() / 10.0}),
        ))
        .collect();
    let best = cells
        .values()
        .max_by_key(|c| (c.scenes.len(), c.pos.x))
        .expect("a cell");
    if let Some(p) = publish.as_mut() {
        let line = progress(rounds, best_rank, deaths, raised.len(), cells.len());
        let run = cleared.clone().unwrap_or_else(|| best.log.clone());
        if let Some(n) = p.best(paths, &bytes, &run, play_frame, true)? {
            let text = format!(
                "The blind search is done: {line}.{}\nThe loop is its best run, with what was measured drawn on it.\nNothing to answer. A goes back to the game.",
                if cleared.is_some() {
                    " It cleared the level."
                } else {
                    ""
                }
            );
            p.gp(&[
                "mechanic",
                "say",
                &p.game,
                &p.mech,
                "--show",
                &n.to_string(),
                "--measure",
                &text,
            ]);
        }
        p.status(&line, true);
    }
    if let Some(pr) = probe {
        let (reached, clip_frames, seen, r, t) = match &probe_won {
            Some((l, seen, r, t)) => (true, Some(l.len()), seen.clone(), Some(*r), Some(*t)),
            None => (false, None, Value::Null, None, None),
        };
        if let Some((l, _, _, _)) = &probe_won {
            let mut il = gp_evidence::input_log::InputLog::new(1);
            il.frames = l.clone();
            fs::write(out_dir.join("clip.log"), il.to_text())?;
        }
        let v = json!({
            "probe": pr.id, "kind": pr.kind, "reached": reached,
            "round": r, "seconds": t.map(|t| (t * 10.0).round() / 10.0),
            "rounds_run": rounds, "decision_calls": calls, "moves_simulated": sims, "deaths": deaths,
            "closest": best_dist, "seen": seen,
            "clip": reached.then(|| out_dir.join("clip.log").display().to_string()),
            "clip_frames_from_power_on": clip_frames, "play_start_frame": play_frame,
        });
        fs::write(
            out_dir.join("probe.json"),
            serde_json::to_string_pretty(&v)?,
        )?;
        return Ok(v);
    }
    let v = json!({
        "rom": rom.display().to_string(), "sha256": sha256,
        "prior": if uniform { "uniform" } else { "laya" },
        "learned": {"x": format!("${:04X}", learned.x), "y": format!("${:04X}", learned.y),
                    "forward": if learned.right { "right" } else { "left" },
                    "screen_x": learned.screen_x, "jump": learned.jump, "run": learned.run,
                    "scene_bytes": w.still.len()},
        "player_found": true, "forward_found": true,
        "levels": levels, "level_times": level_times,
        "rounds": rounds, "seconds": (secs * 10.0).round() / 10.0,
        "decision_calls": calls, "moves_simulated": sims, "deaths": deaths,
        "cells": cells.len(), "scenes_seen": w.scenes.len(),
        "furthest": {"scenes_on_path": best.scenes.len(), "x": best.pos.x},
        "events": {"deaths": deaths, "walls": walls.len(), "pits": pits.len(),
                   "raised_landings": raised.len(), "scene_changes": w.scenes.len() - 1,
                   "grew": cells.values().filter(|c| c.pos.tall >= start_pos.tall + 2).count() > 2},
        "path_frames": cleared.as_ref().map_or(best.log.len(), Vec::len) - play_frame,
    });
    // The path: the clear's, else the furthest cell's, as an input log
    // from power-on (play starts at `play_frame`).
    let mut il = gp_evidence::input_log::InputLog::new(1);
    il.frames = cleared.clone().unwrap_or_else(|| best.log.clone());
    fs::write(out_dir.join("path.log"), il.to_text())?;
    if let Ok(d) = std::env::var("GP_BLIND_SHOTS") {
        // The path's last frame, to look at by hand.
        let mut n = Machine::power_on(&bytes, clip.power_on)?;
        for f in &il.frames {
            n.step(*f)?;
        }
        shot(&mut n, &Path::new(&d).join("end.png"))?;
    }
    if let Ok(a) = std::env::var("GP_FARM_PEEK") {
        // A check by hand: one RAM byte at the path's end.
        if let Ok(a) = u16::from_str_radix(a.trim_start_matches('$'), 16) {
            let mut n = Machine::power_on(&bytes, clip.power_on)?;
            for f in &il.frames {
                n.step(*f)?;
            }
            eprintln!("${a:04X} at the path's end: {}", n.wram()[usize::from(a)]);
        }
    }
    fs::write(
        out_dir.join("result.json"),
        serde_json::to_string_pretty(&v)?,
    )?;
    let f = findings(
        &w,
        &learned,
        &bytes,
        &clip,
        play_frame,
        &il,
        &walls,
        &pits,
        &raised,
        &first,
        &paths.home,
        &v,
        out_dir,
    )?;
    fs::write(
        out_dir.join("findings.json"),
        serde_json::to_string_pretty(&f)?,
    )?;
    Ok(v)
}

/// The x byte's high byte and the y byte's fraction, read off the path:
/// the byte that steps by one exactly when x wraps, and the byte that
/// changes on most frames y moves by a pixel or less.
fn hi_and_fraction(
    rom: &[u8],
    power: gp_evidence::manifest::PowerOn,
    path: &[InputFrame],
    from: usize,
    x: u16,
    y: u16,
) -> Result<(Found, Found)> {
    let mut nes = Machine::power_on(rom, power)?;
    let mut prev: Option<Vec<u8>> = None;
    let mut hi: BTreeMap<u16, (usize, usize)> = BTreeMap::new();
    let mut wraps = 0;
    let mut frac: BTreeMap<u16, usize> = BTreeMap::new();
    let mut ymoves = 0;
    // Bytes that change while y holds still: a counter, not y's fraction.
    let mut busy: BTreeMap<u16, usize> = BTreeMap::new();
    let mut standing = 0usize;
    let mut air = 0u8;
    // A page that holds the sprite table is the sprite buffer, not state.
    let mut oam_page = [0usize; 8];
    for (i, f) in path.iter().enumerate() {
        nes.step(*f)?;
        let w = nes.wram()[..0x800].to_vec();
        // The board's sprites aren't in its work RAM: no page is skipped.
        let oam: Vec<u8> = match &nes {
            Machine::Nes(n) => n.oam().to_vec(),
            Machine::Arcade(_) => Vec::new(),
        };
        for (pg, n) in oam_page.iter_mut().enumerate() {
            if oam.is_empty() {
                break;
            }
            // This frame's, or the frame before's (copied up a frame late).
            // Most bytes alike: the attribute byte's unused bits may differ.
            let alike =
                |page: &[u8]| page.iter().zip(oam.iter()).filter(|(a, b)| a == b).count() >= 200;
            let lagged = prev
                .as_ref()
                .is_some_and(|p| alike(&p[pg * 256..pg * 256 + 256]));
            if alike(&w[pg * 256..pg * 256 + 256]) || lagged {
                *n += 1;
            }
        }
        if i >= from
            && let Some(p) = &prev
        {
            let (a, b) = (p[usize::from(x)], w[usize::from(x)]);
            let d = wrap8(i64::from(b) - i64::from(a));
            let wrapped = (d > 0 && b < a) || (d < 0 && b > a);
            if wrapped && d.abs() <= 8 {
                wraps += 1;
                for k in 0..0x800u16 {
                    let dk = wrap8(i64::from(w[usize::from(k)]) - i64::from(p[usize::from(k)]));
                    let e = hi.entry(k).or_default();
                    if dk == d.signum() {
                        e.0 += 1;
                    }
                }
            } else {
                for k in 0..0x800u16 {
                    if w[usize::from(k)] != p[usize::from(k)] {
                        hi.entry(k).or_default().1 += 1;
                    }
                }
            }
            let dy = wrap8(i64::from(w[usize::from(y)]) - i64::from(p[usize::from(y)]));
            if dy == 0 {
                standing += 1;
                for k in 0..0x800u16 {
                    if w[usize::from(k)] != p[usize::from(k)] {
                        *busy.entry(k).or_default() += 1;
                    }
                }
            }
            // y's fraction carries into y: in the air, when y moves a
            // pixel down the fraction went past 255 to a smaller value
            // (and up, the other way); when y holds, it didn't wrap.
            if dy.abs() <= 1 && (dy != 0 || air > 0) {
                ymoves += 1;
                for k in 0..0x800u16 {
                    if k == y || k == x {
                        continue;
                    }
                    let (b0, b1) = (p[usize::from(k)], w[usize::from(k)]);
                    let fits = match dy {
                        1 => b1 < b0,
                        -1 => b1 > b0,
                        _ => b1 != b0 && (b1 > b0) == (b1.wrapping_sub(b0) < 128),
                    };
                    if fits {
                        *frac.entry(k).or_default() += 1;
                    }
                }
            }
            air = if dy != 0 { 8 } else { air.saturating_sub(1) };
        }
        prev = Some(w);
    }
    let frames = path.len().saturating_sub(from).max(1);
    let state = |k: u16| !nes.noise(usize::from(k)) && oam_page[usize::from(k) / 256] * 4 < frames;
    // A high byte steps at every wrap and hardly ever otherwise.
    let h = hi
        .iter()
        .filter(|(k, _)| state(**k))
        .filter(|(k, (at, other))| **k != x && wraps > 0 && *at == wraps && *other <= wraps * 2)
        .min_by_key(|(_, (_, other))| *other)
        .map(|(k, _)| (*k, wraps));
    let fr = frac
        .iter()
        .filter(|(k, _)| state(**k))
        .filter(|(k, _)| busy.get(k).copied().unwrap_or(0) * 100 <= standing.max(1) * 20)
        .filter(|(_, n)| ymoves > 20 && **n * 100 >= ymoves * 80)
        .max_by_key(|(_, n)| **n)
        .map(|(k, _)| (*k, ymoves));
    Ok((h, fr))
}

/// What blind mode learned, in a stable shape for another worker to turn
/// into a mapping and model facts (format "farm-findings 0.1"). Each
/// finding names the rule that found it, its evidence, and a confidence
/// from 0 to 1 (how well the rule's test held, not a promise).
#[allow(clippy::too_many_arguments)]
fn findings(
    w: &World,
    l: &Learned,
    rom: &[u8],
    clip: &crate::reach::ClipInput,
    play_frame: usize,
    path: &gp_evidence::input_log::InputLog,
    walls: &BTreeSet<(usize, i64)>,
    pits: &BTreeSet<(usize, i64)>,
    raised: &BTreeSet<(usize, i64, i64)>,
    first: &FirstRuns,
    home: &Path,
    result: &Value,
    out_dir: &Path,
) -> Result<Value> {
    let hx = |a: u16| format!("${a:04X}");
    let start_log = {
        let mut il = gp_evidence::input_log::InputLog::new(1);
        il.frames = path.frames[..play_frame.min(path.frames.len())].to_vec();
        let f = out_dir.join("play-start.log");
        fs::write(&f, il.to_text())?;
        f
    };
    let (hi, frac) = hi_and_fraction(rom, clip.power_on, &path.frames, play_frame, l.x, l.y)?;
    let btn = |b: u8| match (l.pads.device, b) {
        (gp_model::model::Device::NesPad, A) => "A",
        (gp_model::model::Device::NesPad, B) => "B",
        (gp_model::model::Device::NesPad, 0x10) => "Up",
        (gp_model::model::Device::NesPad, _) => "?",
        (_, b) => l.pads.name(b),
    };
    // A place with the first run to it, stored once by its content.
    let with_run = |mut v: Value, run: Option<&(Vec<InputFrame>, usize)>| -> Result<Value> {
        if let Some((frames, at)) = run {
            let mut il = gp_evidence::input_log::InputLog::new(1);
            il.frames = frames.clone();
            let (sha, _) = m::put_path(home, &il.to_text())?;
            v["run_path"] = json!(sha);
            v["run_frame"] = json!(at);
        }
        Ok(v)
    };
    let cleared = result["level_times"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|t| t.get("clear_at_x").is_some());
    Ok(json!({
        "format": "farm-findings 0.1",
        "rom_sha256": result["sha256"],
        "path_log": out_dir.join("path.log").display().to_string(),
        "play_start": {
            "frame": play_frame,
            "inputs_log": start_log.display().to_string(),
            "start_run": clip.id,
            "rule": "stage 0 play start (first frame the pad moves something), then 240 frames with nothing held",
            "confidence": 0.9,
        },
        "player": {
            "x": {"addr": hx(l.x), "means": if l.screen_x { "place on the screen (progress adds the scroll)" } else { "place in the level" },
                  "rule": "goes up under Right and down under Left, a sprite drawn at one offset from it, and furthest in 10 s of going forward",
                  "evidence": "learning runs from the play-start frame", "confidence": 0.8},
            "x_hi": hi.map(|(a, n)| json!({"addr": hx(a), "rule": "steps by one each time x wraps, and hardly ever otherwise", "evidence": format!("{n} wraps on the path"), "confidence": if n >= 2 { 0.8 } else { 0.5 }})),
            "y": {"addr": hx(l.y), "rule": "the jump button sends it away and back to where it was", "evidence": "learning runs from the play-start frame", "confidence": 0.8},
            "y_fraction": frac.map(|(a, n)| json!({"addr": hx(a), "rule": "carries into y: wraps on the frames y moves a pixel and not on the frames it holds, on 80% of the frames in the air", "evidence": format!("{n} such frames on the path"), "confidence": 0.5})),
            "box_top_less_y": {"value": l.box_dy, "rule": "the sprite box that moved with the pad", "confidence": 0.6},
        },
        "controls": {
            "jump": btn(l.jump), "run": btn(l.run), "forward": if l.right { "Right" } else { "Left" },
            "rule": "the first of A, B, Up that sends y away and back; forward is the side x goes further",
            "confidence": 0.8,
        },
        "death": {
            "signal": "the pad stops moving the player, then the player jumps back in the same scene",
            "bytes": w.died_bytes.iter().map(|(a, (f, t, n, at))| json!({"addr": hx(*a), "example": [f, t], "times": n, "first_at_path_frame": at})).collect::<Vec<_>>(),
            "count": result["deaths"],
            "rule": "scene bytes that a respawn changed (a lives count changes like this)",
            "confidence": if w.died_bytes.is_empty() { 0.3 } else { 0.6 },
        },
        "level_end": {
            "signal": if cleared { "the pad stops moving the player, and nothing moves it for 20 s, with the player held still: a clear that freezes" } else { "a jump to another scene" },
            "bytes": w.clear_bytes.as_ref().map(|(d, _)| d.iter().map(|(a, f, t)| json!({"addr": hx(*a), "from": f, "to": t})).collect::<Vec<_>>()),
            "at_path_frame": w.clear_bytes.as_ref().map(|(_, at)| at),
            "levels": result["levels"],
            "rule": "bytes that changed from the clear's last move to the freeze: held-still bytes, and small values (a state number)",
            "confidence": if cleared { 0.6 } else { 0.4 },
        },
        "scenes": {"bytes": w.still.len(), "seen": w.scenes.len(), "rule": "RAM that holds still in play; a byte that changes in plain play is dropped"},
        "places": {
            "units": "progress pixels from the play-start position, in 16 px columns; tile values aren't read blind",
            "walls": walls.iter().map(|(s, c)| with_run(json!({"scene": s, "column": c}), first.get(&("wall", *s, *c, 0)))).collect::<Result<Vec<_>>>()?,
            "pits": pits.iter().map(|(s, c)| with_run(json!({"scene": s, "column": c}), first.get(&("pit", *s, *c, 0)))).collect::<Result<Vec<_>>>()?,
            "landings": raised.iter().map(|(s, c, r)| with_run(json!({"scene": s, "column": c, "y_row": r}), first.get(&("landing", *s, *c, *r)))).collect::<Result<Vec<_>>>()?,
            "runs": "each place's run_path is a stored path (gp clip paths, clip::put_path): the inputs from power-on of the first search run to stand there or hit it, and run_frame the frame it got there",
            "rule": "walls: a move that stopped dead; pits: the column after a move that ended in a death (so some are enemies, not pits); landings: standing higher than the start",
            "confidence": 0.4,
        },
        "sprite_groups": {"labelled": [], "note": "not labelled yet: contact outcomes aren't tracked per sprite group in blind mode"},
    }))
}

/// A blind farm into its own folder under `game_dir` (named by the time it
/// started), so no run overwrites another. `game_dir/latest` names the
/// newest folder, and the newest result.json and findings.json are copied
/// next to it.
pub(crate) fn farm_run(rom: &Path, o: &Opts, paths: &Paths, game_dir: &Path) -> Result<Value> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let out = game_dir.join(stamp.to_string());
    let mut v = farm(rom, o, paths, &out)?;
    v["run_dir"] = json!(out.display().to_string());
    fs::write(game_dir.join("latest"), format!("{}\n", out.display()))?;
    for f in ["result.json", "findings.json"] {
        if out.join(f).is_file() {
            fs::copy(out.join(f), game_dir.join(f))?;
        }
    }
    Ok(v)
}

/// Each probe in a probes file, searched for in turn from play start; the
/// results go in one run folder (probes/<id>/probe.json and clip.log).
pub(crate) fn probes(
    rom: &Path,
    o: &Opts,
    paths: &Paths,
    game_dir: &Path,
    file: &Path,
) -> Result<Value> {
    let list: Value = serde_json::from_str(&fs::read_to_string(file)?)?;
    let list = list.as_array().context("a probes file is a JSON list")?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let run = game_dir.join(stamp.to_string());
    let mut out = Vec::new();
    for p in list {
        let pr = Probe::from(p)?;
        let mut oo = o.clone();
        if let Some(s) = pr.seconds {
            oo.seconds = s;
        }
        let dir = run.join("probes").join(&pr.id);
        out.push(
            farm_for(rom, &oo, Some(&pr), paths, &dir)
                .unwrap_or_else(|e| json!({"probe": pr.id, "error": format!("{e:#}")})),
        );
    }
    let v = json!({"run_dir": run.display().to_string(), "probes": out});
    fs::create_dir_all(&run)?;
    fs::write(run.join("probes.json"), serde_json::to_string_pretty(&v)?)?;
    fs::write(
        game_dir.join("latest-probes"),
        format!("{}\n", run.display()),
    )?;
    Ok(v)
}

/// The scene path of the cell furthest along.
fn best_path(cells: &BTreeMap<Key, Cell>) -> Vec<usize> {
    cells
        .values()
        .max_by_key(|c| (c.scenes.len(), c.pos.x))
        .map(|c| c.scenes.clone())
        .unwrap_or_default()
}

/// The new scenes a path entered and didn't come back from: [A, B, A, C]
/// went into B and back (a side room), and on to C, one level.
fn new_scenes_of(path: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, s) in path.iter().enumerate().skip(1) {
        let later_back = path[i + 1..].iter().any(|t| path[..i].contains(t));
        if !path[..i].contains(s) && !later_back {
            out.push(*s);
        }
    }
    out
}

fn levels_of(path: &[usize]) -> usize {
    new_scenes_of(path).len()
}

/// The bench file: games by name, a ROM by hash or a path, train or held out.
pub(crate) fn bench(
    file: &Path,
    held_out: bool,
    o: &Opts,
    paths: &Paths,
    only: Option<&str>,
) -> Result<Value> {
    let t: toml::Table = toml::from_str(&fs::read_to_string(file)?)?;
    let games = t
        .get("game")
        .and_then(|g| g.as_array())
        .context("the bench file has no [[game]]")?;
    let repo = m::repo_root()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    // A fixed round budget a game, the same on every pass: a search cut
    // by wall time does less on a busy machine. Wall time is a backstop.
    let rounds = t.get("rounds").and_then(|v| v.as_integer()).unwrap_or(1500) as usize;
    let mut o = o.clone();
    o.rounds = rounds;
    o.seconds = o.seconds.max(3600.0);
    let o = &o;
    let mut out = Vec::new();
    for g in games {
        let name = g.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let set = g.get("set").and_then(|v| v.as_str()).unwrap_or("train");
        if (set == "held-out") != held_out {
            continue;
        }
        if only.is_some_and(|n| n != name) {
            continue;
        }
        if let Some(why) = g.get("skip").and_then(|v| v.as_str()) {
            out.push(json!({"name": name, "skipped": why}));
            continue;
        }
        let rom = match (
            g.get("sha256").and_then(|v| v.as_str()),
            g.get("path").and_then(|v| v.as_str()),
        ) {
            (Some(h), _) => find_by_hash(&paths.roms, h),
            (None, Some(p)) => Some(repo.join(p)).filter(|p| p.is_file()),
            _ => None,
        };
        let Some(rom) = rom else {
            out.push(json!({"name": name, "skipped": "its ROM isn't in $GP_ROM_DIR (or built)"}));
            continue;
        };
        let dir = paths.home.join("work/farm/blind").join(name);
        let mut r = farm_run(&rom, o, paths, &dir)
            .unwrap_or_else(|e| json!({"error": format!("{e:#}"), "levels": 0}));
        r["name"] = json!(name);
        out.push(r);
    }
    let levels: u64 = out.iter().filter_map(|r| r["levels"].as_u64()).sum();
    // Rounds to each level end (deterministic), and seconds (the machine's).
    let mut per: Vec<f64> = Vec::new();
    let mut per_r: Vec<u64> = Vec::new();
    for r in &out {
        let (mut last, mut last_r) = (0.0, 0u64);
        for t in r["level_times"].as_array().into_iter().flatten() {
            let s = t["seconds"].as_f64().unwrap_or(0.0);
            let rr = t["round"].as_u64().unwrap_or(0) + 1;
            per.push(s - last);
            per_r.push(rr - last_r);
            last = s;
            last_r = rr;
        }
    }
    per.sort_by(f64::total_cmp);
    per_r.sort_unstable();
    let median = per.get(per.len() / 2).copied();
    let median_r = per_r.get(per_r.len() / 2).copied();
    let calls: u64 = out
        .iter()
        .filter_map(|r| r["decision_calls"].as_u64())
        .sum();
    let score = format!(
        "{levels} levels, median {} rounds a level ({} s); {calls} Laya calls in {rounds} rounds a game",
        median_r.map_or("-".to_owned(), |m| m.to_string()),
        median.map_or("-".to_owned(), |m| format!("{m:.0}"))
    );
    let v = json!({"set": if held_out { "held-out" } else { "train" }, "score": score,
                   "levels": levels, "median_rounds_a_level": median_r, "median_seconds_a_level": median,
                   "rounds_a_game": rounds, "decision_calls": calls, "games": out});
    fs::create_dir_all(paths.home.join("work/farm/bench"))?;
    fs::write(
        paths
            .home
            .join("work/farm/bench")
            .join(format!("{stamp}.json")),
        serde_json::to_string_pretty(&v)?,
    )?;
    Ok(v)
}

fn find_by_hash(dir: &Path, sha: &str) -> Option<std::path::PathBuf> {
    let mut files = Vec::new();
    m::walk(dir, &mut files).ok()?;
    files.sort();
    files.into_iter().find(|p| {
        fs::read(p)
            .ok()
            .and_then(|b| gp_evidence::source::identify_nes(&b).ok())
            .is_some_and(|i| i.sha256 == sha)
    })
}

/// Refuses a ROM the bench file lists as held out: those are played only
/// by `gp farm bench --held-out`, never one at a time.
pub(crate) fn refuse_held_out(rom: &Path, bench_file: &Path) -> Result<()> {
    let Ok(text) = fs::read_to_string(bench_file) else {
        return Ok(());
    };
    let t: toml::Table = toml::from_str(&text)?;
    let sha = super::machine::rom_sha256(&fs::read(rom)?)?;
    for g in t
        .get("game")
        .and_then(|g| g.as_array())
        .into_iter()
        .flatten()
    {
        if g.get("set").and_then(|v| v.as_str()) == Some("held-out")
            && g.get("sha256").and_then(|v| v.as_str()) == Some(sha.as_str())
        {
            bail!(
                "{} is held out in {} ({}): it runs only under `gp farm bench --held-out`",
                rom.display(),
                bench_file.display(),
                g.get("name").and_then(|v| v.as_str()).unwrap_or("?")
            );
        }
    }
    Ok(())
}

pub(crate) fn check_rom(rom: &Path) -> Result<()> {
    if !rom.is_file() {
        bail!("no ROM at {}", rom.display());
    }
    Ok(())
}
