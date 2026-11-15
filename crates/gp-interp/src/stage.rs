//! Girder stages: what a level of sloped girders and ladders is like to
//! play, measured on the model, and how close one stage's layout is to
//! another's (`gp level measure`, `gp template check --original`).
//!
//! The layout measures read the compiled level: the ground's one-row boxes,
//! joined where they touch into girders; the ladders' boxes, each placed
//! between the girder at its foot and the next one above; and the spawns.
//! The play measures run the model with its hooks, so they hold for any
//! model whose level is built this way:
//!
//! | Measure | How |
//! |---|---|
//! | Walk speed | The player holds right for [`WALK_FRAMES`] from where the stage starts it |
//! | A ladder's climb | The player, alone in the level (every enemy body put away each frame), is set at the ladder's foot and holds up. A ladder he can't climb to the girder above in [`CLIMB_FRAMES`] is broken |
//! | The direct route | The quickest way to the top girder by walking and the ladders that climb, from the walk speed and each climb's frames, then played: the player alone walks and climbs it, and the frames to the top are counted |
//! | Rollers | The game plays with the player parked where it starts him and no contact on any other kind, so nothing touches him: each roller's (`--roller`, a barrel) life from its throw, the girders it rolls on, how it leaves each (down a ladder, off the end, or dropped through), and where it ends |
//! | Jump windows | The player parked at the middle of each girder, untouchable, while rollers come: for each roller that would meet him, a jump pressed on each of the [`WINDOW_REACH`] frames before they meet, played with contacts back, passes when he is alive [`WINDOW_AFTER`] frames later. The frames that pass are its window |
//!
//! All of it is the deterministic simulation and integer arithmetic, so a
//! model gives the same measures on every machine.

use std::collections::BTreeMap;

use gp_sim::spec::{EventKind, Level, Role, Tile};
use gp_sim::{Fx, Spec, World};

use crate::variant::{Edit, Sim, Variant};

/// Frames the player holds right for the walk speed.
pub const WALK_FRAMES: u32 = 96;
/// The most frames a climb may take.
pub const CLIMB_FRAMES: u32 = 400;
/// The frames before a roller meets the player that a jump is tried on.
pub const WINDOW_REACH: u32 = 40;
/// Frames after a jump's press the player must still be alive.
pub const WINDOW_AFTER: u32 = 80;

/// NES controller bits.
const START: u8 = 0x08;

/// A pad held: its buttons, the first frame and the last.
type Hold = (u8, u32, u32);

/// One girder: ground boxes that touch, left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Girder {
    /// Its boxes, `[x0, y, x1]`, the row a body stands on.
    pub pieces: Vec<[i64; 3]>,
}

impl Girder {
    pub fn x0(&self) -> i64 {
        self.pieces.first().map_or(0, |p| p[0])
    }
    pub fn x1(&self) -> i64 {
        self.pieces.last().map_or(0, |p| p[2])
    }
    pub fn width(&self) -> i64 {
        self.x1() - self.x0() + 1
    }
    /// The standing row at x, or none past its ends.
    pub fn surface(&self, x: i64) -> Option<i64> {
        self.pieces
            .iter()
            .find(|p| p[0] <= x && x <= p[2])
            .map(|p| p[1])
    }
    /// The row at its middle.
    pub fn mid(&self) -> i64 {
        let x = (self.x0() + self.x1()) / 2;
        self.surface(x).unwrap_or(self.pieces[0][1])
    }
    /// Its rows at the left and right ends.
    pub fn ends(&self) -> (i64, i64) {
        (self.pieces[0][1], self.pieces[self.pieces.len() - 1][1])
    }
    /// The times its row changes from piece to piece.
    pub fn steps(&self) -> usize {
        self.pieces
            .windows(2)
            .filter(|w| w[0][1] != w[1][1])
            .count()
    }
}

/// One ladder's box and the girders it joins (indices into
/// [`Layout::girders`], bottom first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ladder {
    pub bx: [i64; 4],
    pub lower: Option<usize>,
    pub upper: Option<usize>,
}

impl Ladder {
    pub fn cx(&self) -> i64 {
        (self.bx[0] + self.bx[2]) / 2
    }
}

/// One spawn the measures name: its kind's name and place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thing {
    pub kind: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    /// The girder under it, bottom first.
    pub girder: Option<usize>,
}

/// A level's girders, ladders and things.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Bottom first, by their middle row.
    pub girders: Vec<Girder>,
    pub ladders: Vec<Ladder>,
    pub things: Vec<Thing>,
    pub goal_under: Option<i64>,
    pub size: [i64; 2],
}

/// Joins the level's ground boxes into girders and places its ladders and
/// spawns on them.
pub fn layout(spec: &Spec, level: usize) -> Layout {
    let l = &spec.levels[level];
    let mut boxes: Vec<[i64; 3]> = l.ground.iter().map(|b| [b[0], b[1], b[2]]).collect();
    boxes.sort_by_key(|b| (b[0], b[1]));
    let mut girders: Vec<Girder> = Vec::new();
    for b in boxes {
        let joins = girders.iter_mut().find(|g| {
            let last = g.pieces[g.pieces.len() - 1];
            b[0] <= last[2] + 1 && (b[1] - last[1]).abs() <= 2
        });
        match joins {
            Some(g) => g.pieces.push(b),
            None => girders.push(Girder { pieces: vec![b] }),
        }
    }
    girders.sort_by_key(|g| std::cmp::Reverse(g.mid()));
    let ladders = l
        .ladders
        .iter()
        .map(|&bx| {
            let cx = (bx[0] + bx[2]) / 2;
            // The foot: the girder nearest the box's bottom; the top: the
            // next one above it there.
            let lower = girders
                .iter()
                .enumerate()
                .filter_map(|(i, g)| g.surface(cx).map(|s| (i, (s - bx[3] - 1).abs())))
                .filter(|&(_, d)| d <= 6)
                .min_by_key(|&(_, d)| d)
                .map(|(i, _)| i);
            let upper = lower.and_then(|lo| {
                let s = girders[lo].surface(cx)?;
                girders
                    .iter()
                    .enumerate()
                    .filter_map(|(i, g)| g.surface(cx).map(|t| (i, t)))
                    .filter(|&(_, t)| t < s - 8)
                    .max_by_key(|&(_, t)| t)
                    .map(|(i, _)| i)
            });
            Ladder { bx, lower, upper }
        })
        .collect();
    let things = l
        .spawns
        .iter()
        .filter(|s| spec.kinds[s.kind].size[0].floor() > 1)
        .map(|s| {
            let k = &spec.kinds[s.kind];
            let (x, y) = (s.x.floor(), s.y.floor());
            let (w, h) = (k.size[0].floor(), k.size[1].floor());
            let girder = girder_under(&girders, x + w / 2, y + h - 4);
            Thing {
                kind: k.name.clone(),
                x,
                y,
                w,
                h,
                girder,
            }
        })
        .collect();
    Layout {
        girders,
        ladders,
        things,
        goal_under: l.goal_under.map(Fx::floor),
        size: [l.size[0] * l.tile_size, l.size[1] * l.tile_size],
    }
}

/// The first girder at or below row `y` at `x`.
fn girder_under(girders: &[Girder], x: i64, y: i64) -> Option<usize> {
    girders
        .iter()
        .enumerate()
        .filter_map(|(i, g)| g.surface(x).map(|s| (i, s)))
        .filter(|&(_, s)| s >= y)
        .min_by_key(|&(_, s)| s)
        .map(|(i, _)| i)
}

/// The girder a body of height `h` at `(x, y)` rolls or stands on: one
/// whose row is within 4 pixels of its feet.
fn girder_at(girders: &[Girder], x: i64, y: i64, h: i64) -> Option<usize> {
    let feet = y + h;
    girders
        .iter()
        .enumerate()
        .filter_map(|(i, g)| g.surface(x).map(|s| (i, (s - feet).abs())))
        .filter(|&(_, d)| d <= 4)
        .min_by_key(|&(_, d)| d)
        .map(|(i, _)| i)
}

// --- Layout similarity -------------------------------------------------------

/// How close one level's layout is to another's.
#[derive(Debug, Clone, PartialEq)]
pub struct Similarity {
    /// The share of cells that aren't empty in either level, with the same
    /// kind in both, at the best shift (and mirror) of the second.
    pub cells: f64,
    /// The shift `[dx, dy]` in cells, and whether the second was mirrored.
    pub shift: [i64; 2],
    pub mirrored: bool,
    /// The share of ladders (of the level with more) with one in the other
    /// within a tile across, on the same girders counted from the bottom,
    /// mirrored as the best cell shift is.
    pub ladders: f64,
    pub ladders_matched: usize,
    pub ladders_of: usize,
    /// The share of things (by kind) within a tile of the other's.
    pub things: f64,
}

/// The most cells a shift moves each way.
pub const SHIFT_REACH: i64 = 4;

/// A level's cells, by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cells {
    /// Width and height in cells.
    pub size: [i64; 2],
    pub tile_size: i64,
    /// Row by row.
    pub tiles: Vec<Tile>,
}

impl Cells {
    pub fn of(l: &Level) -> Self {
        Cells {
            size: l.size,
            tile_size: l.tile_size,
            tiles: l.tiles.clone(),
        }
    }
}

/// Compares two levels' cells, ladders and things.
pub fn similarity(a: (&Cells, &Layout), b: (&Cells, &Layout)) -> Similarity {
    let ((la, ya), (lb, yb)) = (a, b);
    let cell = |l: &Cells, x: i64, y: i64| -> Tile {
        if x < 0 || y < 0 || x >= l.size[0] || y >= l.size[1] {
            return Tile::Empty;
        }
        l.tiles[(y * l.size[0] + x) as usize]
    };
    let (w, h) = (la.size[0].max(lb.size[0]), la.size[1].max(lb.size[1]));
    let mut best = (0.0, [0, 0], false);
    for mirrored in [false, true] {
        for dy in -SHIFT_REACH..=SHIFT_REACH {
            for dx in -SHIFT_REACH..=SHIFT_REACH {
                let (mut same, mut any) = (0u32, 0u32);
                for y in 0..h {
                    for x in 0..w {
                        let ta = cell(la, x, y);
                        let bxx = if mirrored { lb.size[0] - 1 - x } else { x };
                        let tb = cell(lb, bxx + dx, y + dy);
                        if ta == Tile::Empty && tb == Tile::Empty {
                            continue;
                        }
                        any += 1;
                        if ta == tb {
                            same += 1;
                        }
                    }
                }
                let share = if any == 0 {
                    0.0
                } else {
                    f64::from(same) / f64::from(any)
                };
                if share > best.0 {
                    best = (share, [dx, dy], mirrored);
                }
            }
        }
    }
    let tile = la.tile_size.max(1);
    // The second's x as the first's, mirrored and shifted as the cells
    // were best: the first's cell x met the second's x + dx, or mirrored,
    // its width - 1 - x + dx.
    let flip = |x: i64| {
        if best.2 {
            yb.size[0] - 1 - x + best.1[0] * tile
        } else {
            x - best.1[0] * tile
        }
    };
    let ladder_keys = |y: &Layout, mirror: bool| -> Vec<(Option<usize>, Option<usize>, i64)> {
        y.ladders
            .iter()
            .map(|l| (l.lower, l.upper, if mirror { flip(l.cx()) } else { l.cx() }))
            .collect()
    };
    let (ka, kb) = (ladder_keys(ya, false), ladder_keys(yb, true));
    let mut used = vec![false; kb.len()];
    let mut matched = 0;
    for (lo, up, x) in &ka {
        if let Some(j) = (0..kb.len())
            .find(|&j| !used[j] && kb[j].0 == *lo && kb[j].1 == *up && (kb[j].2 - x).abs() <= tile)
        {
            used[j] = true;
            matched += 1;
        }
    }
    let of = ka.len().max(kb.len());
    let mut things_used = vec![false; yb.things.len()];
    let mut things_matched = 0;
    for t in &ya.things {
        if let Some(j) = (0..yb.things.len()).find(|&j| {
            let u = &yb.things[j];
            let ux = flip(u.x + u.w / 2) - u.w / 2;
            let uy = u.y - best.1[1] * tile;
            !things_used[j]
                && u.kind == t.kind
                && (ux - t.x).abs() <= tile
                && (uy - t.y).abs() <= tile
        }) {
            things_used[j] = true;
            things_matched += 1;
        }
    }
    let things_of = ya.things.len().max(yb.things.len());
    Similarity {
        cells: best.0,
        shift: best.1,
        mirrored: best.2,
        ladders: if of == 0 {
            0.0
        } else {
            matched as f64 / of as f64
        },
        ladders_matched: matched,
        ladders_of: of,
        things: if things_of == 0 {
            0.0
        } else {
            f64::from(things_matched) / things_of as f64
        },
    }
}

// --- Play measures -----------------------------------------------------------

/// What [`measure`] is told about the model.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The kinds put away when the player is measured alone: the model's
    /// enemies.
    pub enemies: Vec<usize>,
    /// The kind whose routes are measured, a barrel.
    pub roller: Option<usize>,
    /// The game machine and its playing state's name.
    pub playing: Option<(usize, usize)>,
    /// Frames each roller run lasts, and how many runs (each starting the
    /// game a little later, so the random draws differ).
    pub run_frames: u32,
    pub runs: u32,
}

/// One ladder's climb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Climb {
    pub ladder: usize,
    /// Frames from the foot to standing on the girder above; none when it
    /// can't be climbed.
    pub frames: Option<u32>,
    /// The player's middle less the ladder's where the climb began.
    pub dx: i64,
}

/// A roller's life from its throw.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Life {
    pub start: u64,
    pub end: u64,
    /// The girders it rolled on, in order, with the least and most x it
    /// had on each.
    pub girders: Vec<(usize, i64, i64)>,
    /// How it left each girder but the last: `ladder N`, `end` or `drop`.
    pub leaves: Vec<String>,
    /// Where it was last.
    pub last: (i64, i64),
    /// Still alive when the run ended.
    pub open: bool,
}

/// One roller met while the player stood on a girder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub girder: usize,
    pub x: i64,
    /// The frame they meet.
    pub frame: u64,
    /// The frames before meeting a jump pressed on passes.
    pub passes: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct Measures {
    pub layout: Option<Layout>,
    /// The frame play starts (the first a held direction moves the player).
    pub ready: u64,
    /// Pixels walked in [`WALK_FRAMES`].
    pub walk_px: i64,
    pub climbs: Vec<Climb>,
    /// The direct route: the ladders it climbs, the frames it was planned
    /// to take and the frames it took when played; the holds played.
    pub route: Vec<usize>,
    pub route_plan: u32,
    pub route_frames: Option<u32>,
    pub route_holds: Vec<Hold>,
    /// Whether a direct route to the top girder was found (an empty
    /// [`Measures::route`] is one when play starts on the top girder).
    pub routed: bool,
    /// Pixels walked on each girder by the route.
    pub route_walk: BTreeMap<usize, i64>,
    /// The level measured, an index into `Spec::levels`.
    pub level: usize,
    /// The stages ended at their goals on the way to it ([`enter`]): none
    /// when play starts on it.
    pub entered: Vec<String>,
    pub lives: Vec<Life>,
    pub windows: Vec<Window>,
    pub notes: Vec<String>,
}

pub(crate) fn player(spec: &Spec, w: &World) -> Option<usize> {
    w.bodies
        .iter()
        .position(|b| spec.kinds[b.kind].role == Role::Player)
}

pub(crate) fn masks(spec: &Spec, kind: usize) -> (u8, u8, u8, u8) {
    let k = &spec.kinds[kind];
    let m = |a: usize| spec.actions[a].mask;
    let (left, right) = k
        .run
        .as_ref()
        .map_or((0x40, 0x80), |r| (m(r.left), m(r.right)));
    let jump = k.jump.as_ref().map_or(0x01, |j| m(j.action));
    let up = k.climb.as_ref().map_or(0x10, |c| m(c.up));
    (left, right, up, jump)
}

fn put_away(w: &mut World, enemies: &[usize]) {
    for b in &mut w.bodies {
        if enemies.contains(&b.kind) {
            b.alive = false;
        }
    }
}

pub(crate) fn playing(w: &World, o: &Options) -> bool {
    o.playing
        .is_none_or(|(m, s)| w.machines.get(m).is_some_and(|x| x.state == s))
}

fn died(ev: &[gp_sim::Event]) -> bool {
    ev.iter().any(|e| e.kind == EventKind::PLAYER_DIED)
}

/// Powers on, presses Start at `start` and plays until a held direction
/// moves the player: the world then, and its frame.
fn ready(spec: &Variant, o: &Options, start: u64) -> Result<World, String> {
    let mut w = World::power_on(spec);
    let p = player(spec, &w).ok_or("the level has no player")?;
    let (_, right, _, _) = masks(spec, w.bodies[p].kind);
    for f in 0..4000u64 {
        let pad = if (start..start + 5).contains(&f) {
            START
        } else {
            0
        };
        if f > start + 5 && f % 4 == 0 && playing(&w, o) {
            let mut t = w.clone();
            let Some(p) = player(spec, &t) else { continue };
            let x0 = t.bodies[p].x;
            for _ in 0..6 {
                spec.step(&mut t, right);
            }
            if let Some(q) = player(spec, &t)
                && t.bodies[q].x != x0
                && t.bodies[q].alive
            {
                return Ok(w);
            }
        }
        spec.step(&mut w, pad);
    }
    Err("the player never moved in 4000 frames from power-on".into())
}

/// Whether a held direction moves the living player now: play can start.
fn moves_now(spec: &Variant, w: &World, o: &Options) -> bool {
    if !playing(w, o) {
        return false;
    }
    let mut t = w.clone();
    let Some(p) = player(spec, &t) else {
        return false;
    };
    let (_, right, _, _) = masks(spec, t.bodies[p].kind);
    let x0 = t.bodies[p].x;
    for _ in 0..6 {
        spec.step(&mut t, right);
    }
    player(spec, &t).is_some_and(|q| t.bodies[q].x != x0 && t.bodies[q].alive)
}

/// The world on the first frame play can start on `level`, reached as the
/// game reaches it: power-on and Start pressed at `start` as [`ready`]
/// plays, then, while another stage plays, that stage ended at its goal
/// (the hero put where the level's goal is, so the model's own goal rule,
/// its clear and the next stage's setup run, in the model's stage order),
/// until `level` plays. With the world, the stages ended on the way, in
/// order; none when play starts on `level`.
///
/// Nothing here is read from a ROM or a recording: the stages before are
/// passed by the model's own rules from the place of its goal. A stage
/// with no goal (one ended some other way, as Donkey Kong's 100m by its
/// rivets) can't be passed, and is said to be the reason.
pub fn enter(
    spec: &Variant,
    o: &Options,
    level: usize,
    start: u64,
) -> Result<(World, Vec<String>), String> {
    let mut w = ready(spec, o, start)?;
    let mut passed: Vec<String> = Vec::new();
    let name = |l: usize| spec.levels[l].name.clone();
    // Each stage at most once, and once more for a stage order that
    // starts on a stand-in.
    for _ in 0..=spec.levels.len() {
        if w.level == level {
            return Ok((w, passed));
        }
        let here = w.level;
        end_at_goal(spec, &mut w).map_err(|e| {
            format!(
                "level {} is reached only by finishing {} first, and {e}",
                name(level),
                name(here)
            )
        })?;
        passed.push(name(here));
        let mut started = false;
        for _ in 0..6000 {
            if w.level != here && w.frame % 4 == 0 && moves_now(spec, &w, o) {
                started = true;
                break;
            }
            spec.step(&mut w, 0);
        }
        if !started {
            return Err(format!(
                "after {} was ended at its goal, no stage started in 6000 frames",
                name(here)
            ));
        }
    }
    Err(format!(
        "the model's stage order never comes to level {} from power-on (it played {})",
        name(level),
        passed.join(", ")
    ))
}

/// Ends the stage being played the way a player does: the hero where the
/// level's goal is, until the goal is reached (`goal.reached`).
fn end_at_goal(spec: &Variant, w: &mut World) -> Result<(), String> {
    let l = &spec.levels[w.level];
    let (gx, under) = (l.goal_x, l.goal_under);
    if gx.is_none() && under.is_none() {
        return Err(format!(
            "{} has no goal to reach (it ends some other way), so this check can't play past it",
            l.name
        ));
    }
    let p = player(spec, w).ok_or("there is no player")?;
    let h = spec.kinds[w.bodies[p].kind].size[1];
    for _ in 0..8 {
        let b = &mut w.bodies[p];
        if let Some(g) = gx
            && b.x < g
        {
            b.x = g;
        }
        if let Some(u) = under {
            let above = u.sub(h).max(Fx::ZERO);
            if b.y > above {
                b.y = above;
            }
        }
        b.vx = Fx::ZERO;
        b.vy = Fx::ZERO;
        let ev = spec.step(w, 0);
        if ev.iter().any(|e| e.kind == EventKind::GOAL_REACHED) {
            return Ok(());
        }
    }
    Err(format!(
        "the hero put at {}'s goal didn't reach it (the game's code keeps the hero's place), so this check can't play past it",
        l.name
    ))
}

/// Whether the player stands where the level's goal is reached, as the
/// simulation's bounds test it: at or past its x, above its `under`.
fn at_goal(spec: &Spec, w: &World, p: usize) -> bool {
    let l = &spec.levels[w.level];
    let b = &w.bodies[p];
    (l.goal_x.is_some() || l.goal_under.is_some())
        && l.goal_x.is_none_or(|g| b.x >= g)
        && l.goal_under.is_none_or(|u| b.y < u)
}

/// Sets the player standing at `x` (its box's middle) on `girder`.
fn place(spec: &Spec, w: &mut World, lay: &Layout, girder: usize, cx: i64) -> bool {
    let Some(p) = player(spec, w) else {
        return false;
    };
    let k = &spec.kinds[w.bodies[p].kind];
    let half = k.size[0].floor() / 2;
    let feet = k.tile_hitbox.y.floor() + k.tile_hitbox.h.floor();
    let Some(s) = lay.girders[girder].surface(cx) else {
        return false;
    };
    let b = &mut w.bodies[p];
    b.x = Fx::from_int(cx - half);
    b.y = Fx::from_int(s - feet);
    b.vx = Fx::ZERO;
    b.vy = Fx::ZERO;
    b.on_ground = true;
    b.climbing = false;
    true
}

fn feet_row(spec: &Spec, w: &World, p: usize) -> i64 {
    let k = &spec.kinds[w.bodies[p].kind];
    w.bodies[p].y.floor() + k.tile_hitbox.y.floor() + k.tile_hitbox.h.floor()
}

fn center(spec: &Spec, w: &World, p: usize) -> i64 {
    w.bodies[p].x.floor() + spec.kinds[w.bodies[p].kind].size[0].floor() / 2
}

/// The player alone: climbs ladder `i` from its foot; the frames to stand
/// on the girder above.
fn climb(spec: &Variant, o: &Options, lay: &Layout, base: &World, i: usize) -> Option<(u32, i64)> {
    let l = &lay.ladders[i];
    let (lo, up) = (l.lower?, l.upper?);
    let top = lay.girders[up].surface(l.cx())?;
    let mut best: Option<(u32, i64)> = None;
    // Walked into place from either side, as a player comes to it.
    for side in [-24, 24] {
        for dx in [0, -1, 1, -2, 2, -3, 3] {
            let mut w = base.clone();
            put_away(&mut w, &o.enemies);
            if !place(spec, &mut w, lay, lo, l.cx() + side) {
                continue;
            }
            let p = player(spec, &w)?;
            let mut holds = Vec::new();
            if walk_to(spec, o, &mut w, l.cx() + dx, &mut holds, base.frame).is_err() {
                continue;
            }
            let at = w.frame;
            if up_to(spec, o, &mut w, p, top, &mut holds, base.frame) {
                let f = (w.frame - at) as u32;
                if best.is_none_or(|b| f < b.0) {
                    best = Some((f, dx));
                }
                break;
            }
        }
    }
    best
}

/// Holds up until the player stands at row `top`: whether he did.
#[allow(clippy::too_many_arguments)]
fn up_to(
    spec: &Variant,
    o: &Options,
    w: &mut World,
    p: usize,
    top: i64,
    holds: &mut Vec<Hold>,
    from: u64,
) -> bool {
    let (_, _, upm, _) = masks(spec, w.bodies[p].kind);
    let mut started = false;
    for n in 0..CLIMB_FRAMES {
        let f = (w.frame - from) as u32 + 1;
        match holds.last_mut() {
            Some(h) if h.0 == upm && h.2 + 1 == f => h.2 = f,
            _ => holds.push((upm, f, f)),
        }
        put_away(w, &o.enemies);
        if died(&spec.step(w, upm)) {
            return false;
        }
        let b = &w.bodies[p];
        started |= b.climbing;
        if !b.climbing && b.on_ground && (feet_row(spec, w, p) - top).abs() <= 2 {
            return true;
        }
        if !playing(w, o) {
            return true;
        }
        if !started && n > 12 {
            return false;
        }
    }
    false
}

/// Walks the player alone to `x` on its girder, then climbs: the holds
/// played, as (buttons, first frame, last frame) from `from`.
fn walk_to(
    spec: &Variant,
    o: &Options,
    w: &mut World,
    x: i64,
    holds: &mut Vec<Hold>,
    from: u64,
) -> Result<(), String> {
    walk_jumping(spec, o, w, x, holds, from, None)
}

/// [`walk_to`], jumping when a roller on the player's girder coming his
/// way is `jump` pixels or nearer.
#[allow(clippy::too_many_arguments)]
fn walk_jumping(
    spec: &Variant,
    o: &Options,
    w: &mut World,
    x: i64,
    holds: &mut Vec<Hold>,
    from: u64,
    jump: Option<i64>,
) -> Result<(), String> {
    let p = player(spec, w).ok_or("no player")?;
    let (left, right, _, jm) = masks(spec, w.bodies[p].kind);
    let mut jumped = 0u32;
    for _ in 0..1200 {
        let c = center(spec, w, p);
        let b = &w.bodies[p];
        if c == x && b.on_ground {
            return Ok(());
        }
        let mut pad = if c < x { right } else { left };
        if let (Some(reach), Some(r)) = (jump, o.roller)
            && b.on_ground
            && jumped == 0
        {
            let (py, toward) = (b.y.floor(), if c < x { 1 } else { -1 });
            let near = w.bodies.iter().any(|q| {
                if q.kind != r || !q.alive {
                    return false;
                }
                let qc = q.x.floor() + spec.kinds[r].size[0].floor() / 2;
                let ahead = (qc - c) * toward;
                (q.y.floor() - py).abs() <= 6 && (0..=reach).contains(&ahead)
                    || (q.y.floor() - py).abs() <= 6
                        && (-reach..0).contains(&ahead)
                        && (q.vx.raw() * toward) < 0
            });
            if near {
                pad |= jm;
                jumped = 2;
            }
        } else if jumped > 0 && b.on_ground {
            jumped -= 1;
        }
        let f = (w.frame - from) as u32 + 1;
        match holds.last_mut() {
            Some(h) if h.0 == pad && h.2 + 1 == f => h.2 = f,
            _ => holds.push((pad, f, f)),
        }
        put_away(w, &o.enemies);
        if died(&spec.step(w, pad)) {
            return Err("the player died walking".into());
        }
    }
    Err(format!("the player never reached x {x}"))
}

/// Measures a girder stage's play.
pub fn measure(
    sim: &dyn Sim,
    base_spec: &Spec,
    level: usize,
    o: &Options,
) -> Result<Measures, String> {
    let v = sim.open(base_spec, &Edit::Full)?;
    let spec = &v;
    let lay = layout(spec, level);
    let mut m = Measures::default();
    if lay.girders.len() < 2 {
        return Err(format!(
            "level {}: {} girders, so it isn't a girder stage",
            spec.levels[level].name,
            lay.girders.len()
        ));
    }
    let (base, passed) = enter(spec, o, level, 40)?;
    m.level = level;
    m.entered = passed;
    m.ready = base.frame;
    let p = player(spec, &base).ok_or("no player")?;
    let (_, right, _, jump) = masks(spec, base.bodies[p].kind);
    // The walk.
    let mut w = base.clone();
    let x0 = w.bodies[p].x;
    for _ in 0..WALK_FRAMES {
        put_away(&mut w, &o.enemies);
        spec.step(&mut w, right);
    }
    m.walk_px = (w.bodies[p].x.sub(x0)).floor().max(1);
    // Each ladder's climb.
    for i in 0..lay.ladders.len() {
        let c = climb(spec, o, &lay, &base, i);
        m.climbs.push(Climb {
            ladder: i,
            frames: c.map(|c| c.0),
            dx: c.map_or(0, |c| c.1),
        });
    }
    // The direct route: the quickest by the walk speed and the climbs,
    // from where the player starts to the top girder.
    let start_x = center(spec, &base, p);
    let start_g = girder_at(&lay.girders, start_x, feet_row(spec, &base, p), 0)
        .or_else(|| girder_under(&lay.girders, start_x, feet_row(spec, &base, p) - 2))
        .ok_or("the player stands on no girder")?;
    let top_g = lay.girders.len() - 1;
    let px =
        |dx: i64| -> u32 { (dx.unsigned_abs() as u32 * WALK_FRAMES).div_ceil(m.walk_px as u32) };
    // Dijkstra over (girder, x): x is where the player stands.
    let mut best: BTreeMap<(usize, i64), (u32, Vec<usize>)> = BTreeMap::new();
    best.insert((start_g, start_x), (0, Vec::new()));
    let mut open = vec![(0u32, start_g, start_x, Vec::<usize>::new())];
    let mut done: Option<(u32, Vec<usize>)> = None;
    while let Some(i) = (0..open.len()).min_by_key(|&i| open[i].0) {
        let (cost, g, x, path) = open.swap_remove(i);
        if g == top_g {
            done = Some((cost, path));
            break;
        }
        for c in &m.climbs {
            let l = &lay.ladders[c.ladder];
            let (Some(f), Some(lo), Some(up)) = (c.frames, l.lower, l.upper) else {
                continue;
            };
            if lo != g {
                continue;
            }
            let n = cost + px(l.cx() - x) + f;
            let key = (up, l.cx());
            if best.get(&key).is_none_or(|b| n < b.0) {
                let mut pth = path.clone();
                pth.push(c.ladder);
                best.insert(key, (n, pth.clone()));
                open.push((n, up, l.cx(), pth));
            }
        }
    }
    if let Some((plan, path)) = done {
        m.routed = true;
        m.route_plan = plan;
        m.route = path.clone();
        // Played: walk to each ladder, climb it, coming back to it from
        // past it when it can't be climbed from the side the route comes.
        let mut w = base.clone();
        let mut holds = Vec::new();
        let mut ok = true;
        let mut at = start_x;
        let mut g = start_g;
        for &i in &path {
            let l = &lay.ladders[i];
            let dx = m.climbs[i].dx;
            let top = lay.girders[l.upper.unwrap_or(g)]
                .surface(l.cx())
                .unwrap_or(0);
            let way = if l.cx() + dx >= at { 1 } else { -1 };
            let mut done = None;
            for past in [0, 12] {
                let (mut t, mut h) = (w.clone(), holds.clone());
                let mut walked = (l.cx() + dx - at).abs();
                if past > 0 {
                    if walk_to(
                        spec,
                        o,
                        &mut t,
                        l.cx() + dx + way * past,
                        &mut h,
                        base.frame,
                    )
                    .is_err()
                    {
                        continue;
                    }
                    walked += 2 * past;
                }
                if walk_to(spec, o, &mut t, l.cx() + dx, &mut h, base.frame).is_ok()
                    && up_to(spec, o, &mut t, p, top, &mut h, base.frame)
                {
                    done = Some((t, h, walked));
                    break;
                }
            }
            let Some((t, h, walked)) = done else {
                m.notes
                    .push(format!("route: ladder {i} wasn't climbed when played"));
                ok = false;
                break;
            };
            *m.route_walk.entry(g).or_default() += walked;
            w = t;
            holds = h;
            at = l.cx() + dx;
            g = l.upper.unwrap_or(g);
        }
        if ok {
            m.route_frames = Some((w.frame - base.frame) as u32);
        }
        m.route_holds = holds;
    } else {
        m.notes
            .push("route: no way up by walking and climbing".into());
    }
    // Rollers.
    if let Some(roller) = o.roller {
        // Nothing touches the player: no kind but his has a contact.
        let safe = sim.open(base_spec, &Edit::Contactless(base.bodies[p].kind))?;
        let rh = spec.kinds[roller].size[1].floor();
        let rw = spec.kinds[roller].size[0].floor();
        for run in 0..o.runs {
            let (mut w, _) = enter(spec, o, level, 40 + u64::from(run) * 37)?;
            let mut live: BTreeMap<usize, Life> = BTreeMap::new();
            for _ in 0..o.run_frames {
                let ev = safe.step(&mut w, 0);
                for (i, b) in w.bodies.iter().enumerate() {
                    if b.kind != roller {
                        continue;
                    }
                    let (x, y) = (b.x.floor() + rw / 2, b.y.floor());
                    if !b.alive {
                        if let Some(mut l) = live.remove(&i) {
                            l.end = w.frame;
                            m.lives.push(l);
                        }
                        continue;
                    }
                    let l = live.entry(i).or_insert_with(|| Life {
                        start: w.frame,
                        ..Default::default()
                    });
                    l.last = (x, y);
                    if let Some(g) = girder_at(&lay.girders, x, y, rh - 2) {
                        match l.girders.last_mut() {
                            Some(e) if e.0 == g => {
                                e.1 = e.1.min(x);
                                e.2 = e.2.max(x);
                            }
                            prev => {
                                // It left the girder before by a ladder
                                // whose foot is here, off an end, or
                                // dropped through.
                                if let Some(&(pg, lo, hi)) = prev.map(|p| &*p) {
                                    let lad = lay.ladders.iter().position(|d| {
                                        d.upper == Some(pg)
                                            && d.lower == Some(g)
                                            && (d.cx() - x).abs() <= 6
                                    });
                                    let pgd = &lay.girders[pg];
                                    let how = match lad {
                                        Some(n) => format!("ladder {n}"),
                                        None if lo <= pgd.x0() + 12 || hi >= pgd.x1() - 12 => {
                                            "end".to_owned()
                                        }
                                        None => "drop".to_owned(),
                                    };
                                    l.leaves.push(how);
                                }
                                l.girders.push((g, x, x));
                            }
                        }
                    }
                }
                if died(&ev) || !playing(&w, o) {
                    m.notes.push(format!(
                        "rollers: run {run} ended at frame {} ({})",
                        w.frame,
                        if died(&ev) {
                            "the player died"
                        } else {
                            "play stopped"
                        }
                    ));
                    break;
                }
            }
            for (_, mut l) in live {
                l.end = w.frame;
                l.open = true;
                m.lives.push(l);
            }
        }
        // Jump windows: parked mid-girder on each girder a roller rolls on.
        for g in 0..lay.girders.len().saturating_sub(1) {
            let gd = &lay.girders[g];
            let cx = (gd.x0() + gd.x1()) / 2;
            let mut w = base.clone();
            if !place(spec, &mut w, &lay, g, cx) {
                continue;
            }
            let mut back: std::collections::VecDeque<World> = std::collections::VecDeque::new();
            let mut met: BTreeMap<usize, u64> = BTreeMap::new();
            for _ in 0..o.run_frames {
                back.push_back(w.clone());
                if back.len() > WINDOW_REACH as usize + 1 {
                    back.pop_front();
                }
                // Keep him where he stands.
                let ev = safe.step(&mut w, 0);
                if died(&ev) || !playing(&w, o) {
                    m.notes.push(format!(
                        "windows: girder {g} ended at frame {} ({})",
                        w.frame,
                        if died(&ev) {
                            "the player died"
                        } else {
                            "play stopped"
                        }
                    ));
                    break;
                }
                let pb = &w.bodies[p];
                let hb = spec.kinds[pb.kind].hitbox;
                let (ml, mt) = (pb.x.floor() + hb.x.floor(), pb.y.floor() + hb.y.floor());
                let (mr, mb) = (ml + hb.w.floor() - 1, mt + hb.h.floor() - 1);
                for (i, b) in w.bodies.iter().enumerate() {
                    if b.kind != roller || !b.alive {
                        continue;
                    }
                    let rb = spec.kinds[roller].hitbox;
                    let (bl, bt) = (b.x.floor() + rb.x.floor(), b.y.floor() + rb.y.floor());
                    let (br, bb) = (bl + rb.w.floor() - 1, bt + rb.h.floor() - 1);
                    let meets = ml <= br && bl <= mr && mt <= bb && bt <= mb;
                    if meets && met.get(&i).is_none_or(|&f| w.frame > f + 60) {
                        met.insert(i, w.frame);
                        // Try a jump on each frame before.
                        let mut passes = Vec::new();
                        for (k, past) in back.iter().enumerate() {
                            let before = (back.len() - k) as u32;
                            let mut t = past.clone();
                            let mut alive = true;
                            for f in 0..WINDOW_AFTER {
                                let pad = if f < 2 { jump } else { 0 };
                                if died(&spec.step(&mut t, pad)) {
                                    alive = false;
                                    break;
                                }
                            }
                            if alive {
                                passes.push(before);
                            }
                        }
                        m.windows.push(Window {
                            girder: g,
                            x: cx,
                            frame: w.frame,
                            passes,
                        });
                    }
                }
            }
        }
    }
    m.layout = Some(lay);
    Ok(m)
}

/// A run with the game's enemies in it: Start pressed, then the direct
/// route ([`Measures::route`]) played `n` times, each stage from where play
/// starts, waiting before each walk until the player lives through it and
/// its climb (up to [`WAIT_REACH`] frames, in steps of 8, jumping straight
/// up when a roller comes), and jumping a roller on the way when a walk
/// can't be lived through without, going back a walk when no wait lets the
/// next one through. The holds from power-on, the frames each stage was cleared on, and
/// the frame the next stage after the last starts on. For a model
/// baseline of an authored stage.
///
/// The stage is the one measured ([`Measures::level`]), entered as
/// [`enter`] enters it; for a stage play doesn't start on, the holds are
/// from the frame it was entered (no buttons replay the stages ended at
/// their goals before it). A stage is cleared when play ends with the
/// player at the level's goal, not by a death; a level with no goal, or no
/// route up its ladders, is refused in words.
pub fn clears(
    sim: &dyn Sim,
    base_spec: &Spec,
    o: &Options,
    m: &Measures,
    n: u32,
) -> Result<(Vec<Hold>, Vec<u64>, u64), String> {
    let v = sim.open(base_spec, &Edit::Full)?;
    let spec = &v;
    let lay = m.layout.as_ref().ok_or("not measured")?;
    let live = Options {
        enemies: Vec::new(),
        ..o.clone()
    };
    let level = m.level;
    let name = &spec.levels[level].name;
    if spec.levels[level].goal_x.is_none() && spec.levels[level].goal_under.is_none() {
        return Err(format!(
            "level {name} has no goal to reach (it ends some other way), and this check plays a route to a goal"
        ));
    }
    if !m.routed {
        return Err(format!(
            "level {name}: no route to the top girder by walking and climbing its ladders, so there is no route to play"
        ));
    }
    let mut holds: Vec<Hold> = Vec::new();
    let mut w = if m.entered.is_empty() {
        let mut w = World::power_on(spec);
        let start = 40u64;
        while w.frame < start + 5 {
            let pad = if w.frame >= start { START } else { 0 };
            if pad != 0 {
                let f = w.frame as u32 + 1;
                match holds.last_mut() {
                    Some(h) if h.0 == pad && h.2 + 1 == f => h.2 = f,
                    _ => holds.push((pad, f, f)),
                }
            }
            spec.step(&mut w, pad);
        }
        w
    } else {
        // Reached past the stages before it, which no buttons replay:
        // the holds are from this world on.
        enter(spec, o, level, 40)?.0
    };
    let p = player(spec, &w).ok_or("no player")?;
    let mut cleared = Vec::new();
    let wait_ready = |w: &mut World| -> Result<(), String> {
        let (_, right, _, _) = masks(spec, w.bodies[p].kind);
        for _ in 0..2000 {
            if playing(w, o) {
                let mut t = w.clone();
                let x0 = t.bodies[p].x;
                for _ in 0..6 {
                    spec.step(&mut t, right);
                }
                if t.bodies[p].x != x0 && t.bodies[p].alive {
                    return Ok(());
                }
            }
            spec.step(w, 0);
        }
        Err("play never started".into())
    };
    // One walk and climb of the route after waiting `d` frames: the world
    // and holds after it, when the player lives through it.
    let step =
        |w: &World, holds: &[Hold], i: usize, g: usize, d: u32, past: i64, jump: Option<i64>| {
            let l = &lay.ladders[i];
            let dx = m.climbs[i].dx;
            let top = lay.girders[l.upper.unwrap_or(g)]
                .surface(l.cx())
                .unwrap_or(0);
            let (mut t, mut h) = (w.clone(), holds.to_vec());
            // Waiting, jumping straight up when a roller comes.
            let (_, _, _, jm) = masks(spec, t.bodies[p].kind);
            let mut air = 0;
            for _ in 0..d {
                let b = &t.bodies[p];
                let mut pad = 0;
                if let Some(r) = o.roller
                    && b.on_ground
                    && air == 0
                {
                    let (c, py) = (center(spec, &t, p), b.y.floor());
                    let near = t.bodies.iter().any(|q| {
                        q.kind == r
                            && q.alive
                            && (q.y.floor() - py).abs() <= 6
                            && (q.x.floor() + spec.kinds[r].size[0].floor() / 2 - c).abs()
                                <= jump.unwrap_or(20)
                    });
                    if near {
                        pad = jm;
                        air = 8;
                    }
                } else if air > 0 && b.on_ground {
                    air -= 1;
                }
                if pad != 0 {
                    let f = t.frame as u32 + 1;
                    h.push((pad, f, f));
                }
                if died(&spec.step(&mut t, pad)) {
                    return None;
                }
            }
            let at = center(spec, &t, p);
            let way = if l.cx() + dx >= at { 1 } else { -1 };
            if past > 0
                && walk_to(spec, &live, &mut t, l.cx() + dx + way * past, &mut h, 0).is_err()
            {
                return None;
            }
            walk_jumping(spec, &live, &mut t, l.cx() + dx, &mut h, 0, jump).ok()?;
            if !up_to(spec, &live, &mut t, p, top, &mut h, 0) {
                return None;
            }
            Some((t, h))
        };
    for k in 0..n {
        wait_ready(&mut w)?;
        if w.level != level {
            return Err(format!(
                "stage {} plays level {}, not {name}, whose route this is",
                k + 1,
                spec.levels[w.level].name
            ));
        }
        // Depth first over the route's walks, each tried after each wait,
        // going back a walk when none of the next one's waits lives.
        let route: Vec<(usize, usize)> = {
            let mut g = 0;
            m.route
                .iter()
                .map(|&i| {
                    let here = g;
                    g = lay.ladders[i].upper.unwrap_or(g);
                    (i, here)
                })
                .collect()
        };
        let waits: Vec<(u32, i64, Option<i64>)> = (0..=WAIT_REACH)
            .step_by(8)
            .flat_map(|d| {
                [
                    (d, 0, None),
                    (d, 0, Some(20)),
                    (d, 0, Some(28)),
                    (d, 12, None),
                ]
            })
            .collect();
        let mut stack: Vec<(World, Vec<Hold>, usize)> = vec![(w.clone(), holds.clone(), 0)];
        let mut tries = 0u32;
        let mut deepest = 0usize;
        let mut found = None;
        while !stack.is_empty() {
            let depth = stack.len() - 1;
            deepest = deepest.max(depth);
            let (sw, sh, next) = &mut stack[depth];
            if depth == route.len() {
                found = Some((sw.clone(), sh.clone()));
                break;
            }
            if *next >= waits.len() || tries > 20_000 {
                stack.pop();
                continue;
            }
            let (d, past, jump) = waits[*next];
            *next += 1;
            tries += 1;
            let (i, g) = route[depth];
            if let Some((t, h)) = step(sw, sh, i, g, d, past, jump) {
                stack.push((t, h, 0));
            }
        }
        let (t, h) = found.ok_or(format!("stage {}: no waits get the player up ({tries} tries, {deepest} of {} ladders the most)", k + 1, route.len()))?;
        w = t;
        holds = h;
        // On to the clear: play ends with the player at the goal, and
        // not by a death.
        for _ in 0..600 {
            if !playing(&w, o) {
                break;
            }
            if died(&spec.step(&mut w, 0)) {
                return Err(format!(
                    "stage {}: the player climbed the route and then died before reaching the goal",
                    k + 1
                ));
            }
        }
        if playing(&w, o) {
            return Err(format!(
                "stage {}: the route was climbed, and play went on 600 frames more without the goal reached",
                k + 1
            ));
        }
        if w.level != level || !at_goal(spec, &w, p) {
            return Err(format!(
                "stage {}: play ended with the player at x {}, y {}, which isn't level {name}'s goal",
                k + 1,
                w.bodies[p].x.floor(),
                w.bodies[p].y.floor()
            ));
        }
        cleared.push(w.frame);
    }
    wait_ready(&mut w)?;
    // Held from the step that starts at frame f, as a baseline's holds
    // count them (frame 0 the first).
    let holds = holds
        .into_iter()
        .map(|(b, a, z)| (b, a - 1, z - 1))
        .collect();
    Ok((holds, cleared, w.frame))
}

/// The most frames [`clears`] waits before a walk.
pub const WAIT_REACH: u32 = 240;

#[cfg(test)]
mod tests {
    use super::*;

    fn girder(pieces: &[[i64; 3]]) -> Girder {
        Girder {
            pieces: pieces.to_vec(),
        }
    }

    #[test]
    fn a_girder_reads_its_rows() {
        let g = girder(&[[0, 100, 23], [24, 101, 47], [48, 102, 71]]);
        assert_eq!(g.surface(30), Some(101));
        assert_eq!(g.surface(80), None);
        assert_eq!(g.steps(), 2);
        assert_eq!(g.ends(), (100, 102));
        assert_eq!(g.width(), 72);
    }

    #[test]
    fn a_body_is_on_the_girder_at_its_feet() {
        let gs = vec![girder(&[[0, 200, 255]]), girder(&[[0, 170, 255]])];
        assert_eq!(girder_at(&gs, 50, 186, 14), Some(0));
        assert_eq!(girder_at(&gs, 50, 156, 14), Some(1));
        assert_eq!(girder_at(&gs, 50, 175, 14), None);
        assert_eq!(girder_under(&gs, 50, 180), Some(0));
    }

    /// A made-up stage of two girders and a ladder, 8 by 6 cells.
    fn stage(ladder_col: i64, shift_rows: i64) -> (Cells, Layout) {
        let (w, h) = (8, 6);
        let mut tiles = vec![Tile::Empty; (w * h) as usize];
        for x in 0..w - 1 {
            tiles[((4 + shift_rows) * w + x) as usize] = Tile::Top;
            tiles[((1 + shift_rows) * w + x) as usize] = Tile::Top;
        }
        for y in 2..4 {
            tiles[((y + shift_rows) * w + ladder_col) as usize] = Tile::Ladder;
        }
        let lay = Layout {
            girders: vec![
                girder(&[[0, 32 + shift_rows * 8, 63]]),
                girder(&[[0, 8 + shift_rows * 8, 63]]),
            ],
            ladders: vec![Ladder {
                bx: [ladder_col * 8, 16, ladder_col * 8 + 7, 31],
                lower: Some(0),
                upper: Some(1),
            }],
            things: Vec::new(),
            goal_under: None,
            size: [w * 8, h * 8],
        };
        (
            Cells {
                size: [w, h],
                tile_size: 8,
                tiles,
            },
            lay,
        )
    }

    /// Moss Hop (our own game) on made-up levels, made two stages long:
    /// its clear state goes on to the next level after 30 frames, and the
    /// second level is the first's copy, with no goal unless `goal`.
    fn two_stages(goal: bool) -> Spec {
        use gp_sim::spec::Do;
        let mut s = crate::reach::tests::spec(crate::reach::tests::level(0, &[]));
        let mut second = s.levels[0].clone();
        second.name = "second".into();
        if !goal {
            second.goal_x = None;
            second.goal_under = None;
        }
        s.levels.push(second);
        s.levels.get_mut(0).unwrap().next = Some(1);
        let g = s.machines.index_of("game").unwrap();
        let m = s.machines.get_mut(g).unwrap();
        let state = |n: &str| m.states.iter().position(|x| x.name == n).unwrap();
        let (clear, play, dying) = (state("clear"), state("playing"), state("dying"));
        let mut t = m.states[dying].transitions[0].clone();
        t.to = play;
        t.after = Some(30);
        t.guard = None;
        t.actions = vec![Do::NextLevel];
        m.states[clear].transitions.push(t);
        s
    }

    fn moss_options(s: &Spec) -> Options {
        let g = s.machines.index_of("game").unwrap();
        let play = s.machines[g]
            .states
            .iter()
            .position(|x| x.name == "playing")
            .unwrap();
        Options {
            playing: Some((g, play)),
            ..Options::default()
        }
    }

    #[test]
    fn a_later_stage_is_entered_past_the_one_before_by_its_goal() {
        let exts = gp_sim::Exts::new();
        let sim = crate::variant::NativeSim(&exts);
        let base = two_stages(true);
        let v = sim.open(&base, &Edit::Full).unwrap();
        let o = moss_options(&v);
        // Play starts on the first: nothing is ended on the way.
        let (w, passed) = enter(&v, &o, 0, 40).unwrap();
        assert_eq!((w.level, passed.len()), (0, 0));
        // The second: the first ended at its goal, then the model's own
        // clear and next-level bring the second in, its player at its
        // spawn and playing.
        let (w, passed) = enter(&v, &o, 1, 40).unwrap();
        assert_eq!(w.level, 1);
        assert_eq!(passed, vec!["main".to_owned()]);
        let p = player(&v, &w).unwrap();
        assert_eq!(w.bodies[p].x.floor(), 24, "at the second's spawn");
        assert!(playing(&w, &o));
    }

    #[test]
    fn a_stage_with_no_goal_is_neither_passed_nor_played() {
        let exts = gp_sim::Exts::new();
        let sim = crate::variant::NativeSim(&exts);
        let mut base = two_stages(false);
        // A third stage after the goalless second.
        let mut third = base.levels[0].clone();
        third.name = "third".into();
        third.next = None;
        base.levels.push(third);
        base.levels.get_mut(1).unwrap().next = Some(2);
        let v = sim.open(&base, &Edit::Full).unwrap();
        let o = moss_options(&v);
        let e = enter(&v, &o, 2, 40).unwrap_err();
        assert!(
            e.contains("level third is reached only by finishing second first"),
            "{e}"
        );
        assert!(e.contains("second has no goal to reach"), "{e}");
        // The goalless stage itself is refused by the clears in words,
        // where an empty route used to count the end of play as a clear.
        let m = Measures {
            layout: Some(layout(&v, 1)),
            level: 1,
            routed: true,
            entered: vec!["main".into()],
            ..Measures::default()
        };
        let e = clears(&sim, &base, &o, &m, 1).unwrap_err();
        assert!(e.contains("level second has no goal to reach"), "{e}");
        let m = Measures {
            layout: Some(layout(&v, 0)),
            level: 0,
            ..Measures::default()
        };
        let e = clears(&sim, &base, &o, &m, 1).unwrap_err();
        assert!(e.contains("no route to the top girder"), "{e}");
    }

    #[test]
    fn the_goal_is_where_the_bounds_test_it() {
        let base = two_stages(true);
        let mut w = World::power_on(&base);
        let p = player(&base, &w).unwrap();
        assert!(!at_goal(&base, &w, p), "the spawn isn't the goal");
        let gx = base.levels[0].goal_x.unwrap();
        w.bodies[p].x = gx;
        assert!(at_goal(&base, &w, p));
        w.level = 1;
        let mut nogoal = base.clone();
        nogoal.levels.get_mut(1).unwrap().goal_x = None;
        assert!(
            !at_goal(&nogoal, &w, p),
            "a level with no goal has no place for it"
        );
    }

    #[test]
    fn a_shifted_copy_is_as_close_as_the_original() {
        let a = stage(2, 0);
        let b = stage(2, 1);
        let s = similarity((&a.0, &a.1), (&b.0, &b.1));
        assert_eq!(s.cells, 1.0);
        assert_eq!(s.shift, [0, 1]);
        assert_eq!(s.ladders, 1.0);
    }

    #[test]
    fn a_mirrored_copy_is_found() {
        let a = stage(2, 0);
        // Mirrored, the girders (columns 0 to 6) take columns 1 to 7, so
        // the best shift is a column back, and the ladder at column 4 goes
        // to 3 and back to 2.
        let b = stage(4, 0);
        let s = similarity((&a.0, &a.1), (&b.0, &b.1));
        assert_eq!(s.cells, 1.0);
        assert!(s.mirrored);
        assert_eq!(s.ladders, 1.0);
    }

    #[test]
    fn a_moved_ladder_lowers_both_measures() {
        // Two columns over: a shift or a mirror that lines the ladders up
        // leaves more girder cells apart than it puts together.
        let a = stage(1, 0);
        let b = stage(3, 0);
        let s = similarity((&a.0, &a.1), (&b.0, &b.1));
        assert!(s.cells < 1.0, "{s:?}");
        assert_eq!(s.shift, [0, 0]);
        assert_eq!(s.ladders, 0.0);
    }
}
