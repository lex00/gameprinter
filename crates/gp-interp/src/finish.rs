//! Finishing a stage by search in the whole world: the stages a direct
//! route up the ladders can't play (`gp level verify`, docs/level-kit.md
//! "Verifying a stage").
//!
//! Two kinds of stage need it:
//!
//! | Kind | What the model says | What finishes it |
//! |---|---|---|
//! | Crossed on a carrier | A body the level's ground holds (platformer `level.ground.bodies`), moved by the model each frame, that the player stands on and rides | The goal reached, as for any stage |
//! | Ended by a condition | No goal; the game's rules end the stage some other way (Donkey Kong's 100m when every rivet is taken: its pickups' counters and a hook's event) | Play leaves the stage with the player alive and the game moves on to another level |
//!
//! The search plays the stage as the game reaches it ([`crate::stage::enter`]),
//! everything in it: enemies, timers, hooks and the game's rules. From
//! each place the player stands it tries the player's own moves (walk a
//! short or a long way, jump up or either way, climb up or down a ladder,
//! wait), breadth first, and keeps each place it first stands on with the
//! whole world then. Waiting is a move, so the player can wait for a
//! carrier and ride it: near a carrier, the place is kept by the
//! carriers' rows too, so the same spot with the platform at another
//! height is another place. A stage that ends by a condition is played
//! in legs: each leg searches until one of the level's pickups is taken
//! (or the stage ends), and the next starts from there.
//!
//! The moves are a sample, as [`crate::reach`]'s are, so a stage no run is
//! found for is said to be that: no run found in so many places.

use std::collections::{BTreeMap, VecDeque};

use gp_sim::spec::EventKind;
use gp_sim::{Spec, World};

use crate::stage::{self, Options, masks, player, playing};
use crate::variant::{Edit, Sim, Variant};

/// The most places one leg searches.
pub const LIMIT: usize = 12_000;
/// The most frames play may stay off the stage's play state (a scene, a
/// hammer's smash) before the search stops waiting for it.
const AWAY: u32 = 4000;
/// The most frames one move takes.
const MOVE_FRAMES: u32 = 400;
/// Pixels across from a carrier within which a place is kept by the
/// carriers' rows too.
const NEAR: i64 = 24;
/// Pixels across a place is kept by.
const STEP: i64 = 4;

/// How a stage was finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum How {
    /// The goal was reached (`goal.reached`).
    Goal,
    /// Play left the stage with the player alive on frame `left`, and the
    /// game went on to level `next` on frame `next_at`.
    Left {
        left: u64,
        next: String,
        next_at: u64,
    },
}

/// A run found that finishes the stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub level: String,
    /// The frame the stage was finished on, counted from power-on: the
    /// goal reached, or play leaving the stage.
    pub frame: u64,
    pub how: How,
    /// The stages ended at their goals on the way to it.
    pub entered: Vec<String>,
    /// Moves of the run made standing on a carrier, and the carriers'
    /// kinds by name.
    pub rides: usize,
    pub carriers: Vec<String>,
    /// The frames each pickup on the way was taken on.
    pub taken: Vec<u64>,
    /// Places searched, moves and frames played, over every leg.
    pub searched: usize,
    pub moves: u64,
    pub frames: u64,
}

/// No run found: where the search stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotFound {
    pub level: String,
    pub entered: Vec<String>,
    /// The pickups taken before the leg that found nothing.
    pub taken: Vec<u64>,
    pub searched: usize,
    pub moves: u64,
    pub frames: u64,
    /// Whether the last leg stopped at [`LIMIT`] places.
    pub at_limit: bool,
}

/// What a stage's level ends by, by the model.
pub fn ends_by_goal(spec: &Spec, level: usize) -> bool {
    let l = &spec.levels[level];
    l.goal_x.is_some() || l.goal_under.is_some()
}

/// The kinds whose bodies the level's ground holds: its carriers.
pub fn carriers(spec: &Spec, level: usize) -> Vec<usize> {
    let mut k: Vec<usize> = spec.levels[level]
        .ground_bodies
        .iter()
        .map(|b| b.0)
        .collect();
    k.sort_unstable();
    k.dedup();
    k
}

#[derive(Debug, Clone, Copy)]
enum Move {
    Walk(u8, u32),
    /// Walk until the player's middle is at this x, to a ladder.
    WalkTo(i64),
    Jump(u8),
    Climb(u8),
    Wait(u32),
}

enum End {
    Stand,
    Died,
    /// The stage finished, and the frame a pickup was taken on in the
    /// move, if one was.
    Done(How, Option<u64>),
    /// One of the level's pickups taken.
    Took,
    Nothing,
}

struct Play<'a, 'b> {
    v: &'a Variant<'b>,
    o: &'a Options,
    level: usize,
    p: usize,
    pickups: Vec<usize>,
    carriers: Vec<usize>,
    frames: u64,
    /// The frame of the move's first pickup taken, if any.
    took_at: Option<u64>,
}

impl Play<'_, '_> {
    fn taken(&self, w: &World) -> usize {
        self.pickups
            .iter()
            .filter(|&&c| w.counters.get(c).is_some_and(|v| *v != 0))
            .count()
    }

    /// One frame: what it ended, if anything.
    fn frame(&mut self, w: &mut World, pad: u8, before: usize) -> Option<End> {
        let ev = self.v.step(w, pad);
        self.frames += 1;
        if self.took_at.is_none() && self.taken(w) > before {
            self.took_at = Some(w.frame);
        }
        for e in &ev {
            if e.kind == EventKind::PLAYER_DIED {
                return Some(End::Died);
            }
            if e.kind == EventKind::GOAL_REACHED {
                return Some(End::Done(How::Goal, None));
            }
        }
        if !w.bodies.get(self.p).is_some_and(|b| b.alive) {
            return Some(End::Died);
        }
        if w.level != self.level {
            return Some(End::Done(
                How::Left {
                    left: w.frame,
                    next: self.v.levels[w.level].name.clone(),
                    next_at: w.frame,
                },
                None,
            ));
        }
        if !playing(w, self.o) {
            // Off the play state with the player alive: a scene, a smash,
            // or the stage's end. Wait for play to come back or the game
            // to go on to another level.
            let left = w.frame;
            for _ in 0..AWAY {
                let ev = self.v.step(w, 0);
                self.frames += 1;
                if ev.iter().any(|e| e.kind == EventKind::PLAYER_DIED)
                    || !w.bodies.get(self.p).is_some_and(|b| b.alive)
                {
                    return Some(End::Died);
                }
                if w.level != self.level {
                    return Some(End::Done(
                        How::Left {
                            left,
                            next: self.v.levels[w.level].name.clone(),
                            next_at: w.frame,
                        },
                        None,
                    ));
                }
                if playing(w, self.o) {
                    break;
                }
            }
            if !playing(w, self.o) {
                return Some(End::Died);
            }
        }
        (self.taken(w) > before).then_some(End::Took)
    }

    fn standing(&self, w: &World) -> bool {
        let b = &w.bodies[self.p];
        b.on_ground && !b.climbing
    }

    /// Plays one move from a standing place.
    fn play(&mut self, w: &mut World, m: Move) -> End {
        let k = w.bodies[self.p].kind;
        let (lm, rm, _, jump) = masks(self.v, k);
        let half = self.v.kinds[k].size[0].floor() / 2;
        let held = self.v.kinds[k].jump.as_ref().is_some_and(|j| {
            j.gravity_held.is_some() || j.bands.iter().any(|b| b.gravity_held.is_some())
        });
        let before = self.taken(w);
        self.took_at = None;
        let (x0, y0) = (w.bodies[self.p].x, w.bodies[self.p].y);
        let mut t = 0u32;
        let mut left_ground = false;
        let mut climbed = false;
        loop {
            if t >= MOVE_FRAMES {
                return End::Nothing;
            }
            let b = &w.bodies[self.p];
            let pad = match m {
                Move::Walk(d, n) => {
                    if t < n {
                        d
                    } else {
                        0
                    }
                }
                Move::WalkTo(x) => {
                    let c = b.x.floor() + half;
                    if c < x {
                        rm
                    } else if c > x {
                        lm
                    } else {
                        0
                    }
                }
                Move::Jump(d) => {
                    if (!left_ground && t < 2) || (held && t < 40 && b.vy.raw() < 0) {
                        jump | d
                    } else if !b.on_ground {
                        d
                    } else {
                        0
                    }
                }
                Move::Climb(d) => {
                    if t < 12 || b.climbing {
                        d
                    } else {
                        0
                    }
                }
                Move::Wait(_) => 0,
            };
            match self.frame(w, pad, before) {
                Some(End::Took) => {}
                Some(End::Done(how, _)) => return End::Done(how, self.took_at),
                Some(e) => return e,
                None => {}
            }
            t += 1;
            let b = &w.bodies[self.p];
            left_ground |= !b.on_ground;
            climbed |= b.climbing;
            let done = match m {
                Move::Walk(_, n) => t >= n && self.standing(w),
                Move::WalkTo(x) => {
                    if !self.standing(w) {
                        return End::Nothing;
                    }
                    w.bodies[self.p].x.floor() + half == x
                }
                Move::Jump(_) => (left_ground || t > 8) && self.standing(w),
                Move::Climb(_) => {
                    if t >= 12 && !climbed {
                        return End::Nothing;
                    }
                    climbed && self.standing(w)
                }
                Move::Wait(n) => t >= n && self.standing(w),
            };
            if done {
                let b = &w.bodies[self.p];
                return if self.took_at.is_some() {
                    End::Took
                } else if b.x == x0 && b.y == y0 && !matches!(m, Move::Wait(_)) {
                    End::Nothing
                } else {
                    End::Stand
                };
            }
        }
    }

    fn moves(&self, w: &World) -> Vec<Move> {
        let k = w.bodies[self.p].kind;
        let (left, right, up, jump) = masks(self.v, k);
        let kind = &self.v.kinds[k];
        let mut out = Vec::new();
        for d in [left, right] {
            for n in [8, 32] {
                out.push(Move::Walk(d, n));
            }
        }
        if kind.jump.is_some() && jump != 0 {
            for d in [0, left, right] {
                out.push(Move::Jump(d));
            }
        }
        if let Some(c) = &kind.climb {
            // To each ladder whose foot or top is at the player's feet,
            // near enough to walk to: its middle and a pixel either side.
            let b = &w.bodies[self.p];
            let cx = b.x.floor() + kind.size[0].floor() / 2;
            let feet = b.y.floor() + kind.tile_hitbox.y.floor() + kind.tile_hitbox.h.floor();
            for l in &self.v.levels[self.level].ladders {
                let lx = (l[0] + l[2]) / 2;
                let meets = (l[3] + 1 - feet).abs() <= 6 || (l[1] - feet).abs() <= 6;
                if meets && (lx - cx).abs() <= 96 {
                    for dx in [-1, 0, 1] {
                        if lx + dx != cx {
                            out.push(Move::WalkTo(lx + dx));
                        }
                    }
                }
            }
            out.push(Move::Climb(up));
            out.push(Move::Climb(self.v.actions[c.down].mask));
        }
        out.push(Move::Wait(8));
        out.push(Move::Wait(32));
        out
    }

    /// The place a world is kept by: the player's row, its x by
    /// [`STEP`] pixels (by the pixel at a ladder) and whether it rides a
    /// carrier; and, standing
    /// beside a carrier's column ([`NEAR`] pixels across), the rows of the
    /// carriers there by [`STEP`] * 2, so the same spot with a platform at
    /// another height is another place. Riding, the row it is carried to
    /// says where the platform is.
    fn key(&self, w: &World) -> Vec<i64> {
        let b = &w.bodies[self.p];
        let (x, y) = (b.x.floor(), b.y.floor());
        let riding = self.on_carrier(w);
        // At a ladder the pixel counts: a climb takes only a few.
        let c = x + self.v.kinds[b.kind].size[0].floor() / 2;
        let at_ladder = self.v.levels[self.level]
            .ladders
            .iter()
            .any(|l| ((l[0] + l[2]) / 2 - c).abs() <= 2);
        let kx = if at_ladder {
            x * STEP
        } else {
            x.div_euclid(STEP)
        };
        let mut key = vec![kx, i64::from(at_ladder), y, i64::from(riding)];
        if !riding {
            let mut rows: Vec<i64> = w
                .bodies
                .iter()
                .filter(|c| {
                    c.alive && self.carriers.contains(&c.kind) && (c.x.floor() - x).abs() <= NEAR
                })
                .map(|c| c.x.floor() * 1000 + c.y.floor().div_euclid(2 * STEP))
                .collect();
            rows.sort_unstable();
            key.extend(rows);
        }
        key
    }

    fn on_carrier(&self, w: &World) -> bool {
        let r = w.bodies[self.p].riding as usize;
        r > 0
            && w.bodies
                .get(r - 1)
                .is_some_and(|c| self.carriers.contains(&c.kind))
    }
}

/// Where a leg ended: the world, how the stage was finished (and the
/// frame a pickup was taken on in the last move) when it was, and the
/// moves on carriers on the way.
type Reached = (World, Option<(How, Option<u64>)>, usize);

/// One leg: breadth first from `start` until the stage is finished (or,
/// with `legs`, a pickup is taken). The world it ended on, how, and the
/// moves on its path made on a carrier.
fn leg(
    pl: &mut Play,
    start: &World,
    legs: bool,
    searched: &mut usize,
    moves: &mut u64,
) -> (Option<Reached>, bool) {
    // Each place: its world and the moves on carriers on the way.
    let mut nodes: Vec<(World, usize)> = vec![(start.clone(), 0)];
    let mut seen: BTreeMap<Vec<i64>, usize> = BTreeMap::new();
    seen.insert(pl.key(start), 0);
    let mut queue = VecDeque::from([0usize]);
    let mut at_limit = false;
    while let Some(i) = queue.pop_front() {
        let tries = pl.moves(&nodes[i].0);
        let riding = pl.on_carrier(&nodes[i].0);
        for m in tries {
            let mut w = nodes[i].0.clone();
            *moves += 1;
            let rides = nodes[i].1 + usize::from(riding);
            match pl.play(&mut w, m) {
                End::Done(how, took) => {
                    *searched += nodes.len();
                    return (Some((w, Some((how, took)), rides)), false);
                }
                End::Took if legs => {
                    *searched += nodes.len();
                    return (Some((w, None, rides)), false);
                }
                End::Stand | End::Took => {
                    if nodes.len() >= LIMIT {
                        at_limit = true;
                        continue;
                    }
                    let k = pl.key(&w);
                    if seen.contains_key(&k) {
                        continue;
                    }
                    seen.insert(k, nodes.len());
                    queue.push_back(nodes.len());
                    nodes.push((w, rides));
                }
                End::Died | End::Nothing => {}
            }
        }
    }
    *searched += nodes.len();
    (None, at_limit)
}

/// Searches for a run that finishes `level` (an index into
/// `base_spec.levels`), played as the game reaches it, with everything in
/// it: to its goal when it has one, else until play leaves the stage with
/// the player alive, taking its pickups on the way.
pub fn finish(
    sim: &dyn Sim,
    base_spec: &Spec,
    o: &Options,
    level: usize,
) -> Result<Result<Finished, NotFound>, String> {
    let v = sim.open(base_spec, &Edit::Full)?;
    let (start, entered) = stage::enter(&v, o, level, 40)?;
    let p = player(&v, &start).ok_or("the level has no player")?;
    let name = v.levels[level].name.clone();
    let by_goal = ends_by_goal(&v, level);
    if !by_goal && o.playing.is_none() {
        return Err(format!(
            "level {name} has no goal, and the model has no game machine with a playing state, so the search can't tell when the stage ends"
        ));
    }
    let carriers = carriers(&v, level);
    let pickups: Vec<usize> = v.levels[level]
        .pickups
        .iter()
        .flat_map(|g| g.counters.iter().copied())
        .collect();
    let places = pickups.len();
    let mut pl = Play {
        v: &v,
        o,
        level,
        p,
        pickups,
        carriers: carriers.clone(),
        frames: 0,
        took_at: None,
    };
    let (mut searched, mut moves) = (0usize, 0u64);
    let mut w = start;
    let mut taken = Vec::new();
    let mut rides = 0;
    // A stage that ends by a condition is played a pickup at a time.
    let legs = !by_goal && places > 0;
    for _ in 0..=places {
        let (found, at_limit) = leg(&mut pl, &w, legs, &mut searched, &mut moves);
        let Some((t, how, r)) = found else {
            return Ok(Err(NotFound {
                level: name,
                entered,
                taken,
                searched,
                moves,
                frames: pl.frames,
                at_limit,
            }));
        };
        rides += r;
        w = t;
        match how {
            Some((how, took)) => {
                taken.extend(took);
                // The frame it was finished on: for a stage left, the
                // frame play left it.
                let frame = match &how {
                    How::Left { left, .. } => *left,
                    How::Goal => w.frame,
                };
                return Ok(Ok(Finished {
                    level: name,
                    frame,
                    how,
                    entered,
                    rides,
                    carriers: carriers.iter().map(|&k| v.kinds[k].name.clone()).collect(),
                    taken,
                    searched,
                    moves,
                    frames: pl.frames,
                }));
            }
            None => taken.push(w.frame),
        }
    }
    Ok(Err(NotFound {
        level: name,
        entered,
        taken,
        searched,
        moves,
        frames: pl.frames,
        at_limit: false,
    }))
}

#[cfg(test)]
mod tests {
    //! Moss Hop's model (templates/mosshop, our own game) on a made-up
    //! stage of ground boxes: a low floor, a ledge 80 pixels above it
    //! (higher than the hero's jump), and between them a platform a
    //! made-up hook moves up and down on a fixed cycle. Nothing is a ROM's.

    use super::*;
    use crate::reach::tests::{level, spec};
    use crate::variant::NativeSim;
    use gp_sim::spec::Tile;
    use gp_sim::{Exts, Fx};

    /// A platform from row 192 up to 112 and back, a pixel every two
    /// frames, waiting 40 frames at each end: a cycle of 400 frames.
    struct Shuttle;

    impl gp_sim::ext::Ext for Shuttle {
        fn step(&self, cx: &mut gp_sim::ext::ExtCx<'_>) {
            let p = (cx.frame % 400) as i64;
            let up = match p {
                0..40 => 0,
                40..200 => (p - 40) / 2,
                200..240 => 80,
                _ => 80 - (p - 240) / 2,
            };
            cx.me.y = Fx::from_int(192 - up);
        }
    }

    /// The stage, with the platform (the slug's kind made a carrier) or
    /// with none.
    fn stage(carrier: bool) -> Spec {
        // Solid cells under the floor (columns 0 to 5) and the ledge (row
        // 7 from column 9), and the same as ground boxes, which a ride
        // needs; a pit between them.
        let mut s = spec(level(0, &[]));
        let l = &mut s.levels[0];
        let w = l.size[0] as usize;
        l.tiles = vec![Tile::Empty; l.tiles.len()];
        for c in 0..6 {
            l.tiles[12 * w + c] = Tile::Solid;
        }
        for c in 9..w {
            l.tiles[7 * w + c] = Tile::Solid;
        }
        l.ground = vec![[0, 192, 95, 192], [144, 112, 511, 112]];
        l.ground_feet = [2, 13];
        l.ground_boxes = true;
        l.ground_land = Some(2);
        l.goal_x = Some(Fx::from_int(300));
        let slug = l.spawns[1].kind;
        l.spawns.truncate(2);
        l.spawns[1].x = Fx::from_int(104);
        l.spawns[1].y = Fx::from_int(192);
        l.spawns[1].patrol_end = None;
        if carrier {
            l.ground_bodies = vec![(slug, [0, 0, 31, 0])];
        } else {
            l.spawns.truncate(1);
        }
        let k = &mut s.kinds[slug];
        k.ext = Some("shuttle".into());
        k.contact = None;
        k.patrol = None;
        k.walk = None;
        k.fall = None;
        k.machine = None;
        k.tile_collision = false;
        s
    }

    fn exts() -> Exts {
        let mut e = Exts::new();
        e.insert("shuttle".into(), Box::new(Shuttle));
        e
    }

    /// The hero waits for the platform, rides it up and walks off onto the
    /// ledge to the goal; with no platform every place is searched and
    /// none reaches it.
    #[test]
    fn a_carrier_is_waited_for_and_ridden_to_the_goal() {
        let e = exts();
        let s = stage(true);
        assert_eq!(carriers(&s, 0), vec![s.levels[0].spawns[1].kind]);
        let f = finish(&NativeSim(&e), &s, &Options::default(), 0)
            .unwrap()
            .expect("a run");
        assert_eq!(f.how, How::Goal);
        assert!(f.rides > 0, "{f:?}");
        assert!(f.frame > 0);
        let n = finish(&NativeSim(&e), &stage(false), &Options::default(), 0)
            .unwrap()
            .expect_err("no run without the platform");
        assert!(!n.at_limit, "{n:?}");
    }

    /// Moss Hop made two stages long, the first with no goal: it ends when
    /// its second pickup (of two made-up places on the floor, x 120 and
    /// 300) is taken, its clear going on to the second level 30 frames
    /// later. With `third` the stage needs a pickup there is no place for.
    fn condition(unreachable: bool) -> (Spec, Options) {
        use gp_sim::spec::{Cmp, Do, Guard, PickupGroup};
        let mut s = spec(level(0, &[]));
        let second = s.levels[0].clone();
        s.levels.push(second);
        s.levels[1].name = "second".into();
        let base = s.counters.len();
        for n in ["taken_0", "taken_1", "taken_2"] {
            let mut c = s.counters[0].clone();
            c.name = n.into();
            c.start = 0;
            c.changes.clear();
            c.min = None;
            c.max = None;
            s.counters.push(c);
        }
        let l = &mut s.levels[0];
        l.goal_x = None;
        l.goal_under = None;
        l.next = Some(1);
        l.pickups = vec![PickupGroup {
            places: vec![[120, 176], [300, 176]],
            counters: vec![base, base + 1],
            offset: [0, 0],
            window: [-8, 8],
            event: None,
            hole: None,
            cells: Vec::new(),
            by_hook: false,
        }];
        let g = s.machines.index_of("game").unwrap();
        let m = s.machines.get_mut(g).unwrap();
        let state = |n: &str| m.states.iter().position(|x| x.name == n).unwrap();
        let (clear, play, dying) = (state("clear"), state("playing"), state("dying"));
        let mut t = m.states[dying].transitions[0].clone();
        t.to = play;
        t.after = Some(30);
        t.guard = None;
        t.actions = vec![Do::NextLevel];
        m.states[clear].transitions.push(t.clone());
        t.to = clear;
        t.after = Some(1);
        t.actions = Vec::new();
        t.guard = Some(Guard {
            counter: base + if unreachable { 2 } else { 1 },
            cmp: Cmp::Ge,
            value: 1,
        });
        m.states[play].transitions.push(t);
        let o = Options {
            playing: Some((g, play)),
            ..Options::default()
        };
        (s, o)
    }

    /// A stage with no goal is finished when its condition is: both
    /// pickups taken, a leg each, and the game goes on to the next level;
    /// one whose condition no pickup meets is searched out with none.
    #[test]
    fn a_stage_that_ends_by_a_condition_is_played_to_it() {
        let e = Exts::new();
        let (s, o) = condition(false);
        let f = finish(&NativeSim(&e), &s, &o, 0).unwrap().expect("a run");
        assert_eq!(f.taken.len(), 2, "{f:?}");
        let How::Left {
            left,
            next,
            next_at,
        } = &f.how
        else {
            panic!("{f:?}")
        };
        assert_eq!(next, "second");
        assert_eq!(*left, f.frame);
        assert!(f.taken[1] <= f.frame && f.frame < *next_at, "{f:?}");
        let (s, o) = condition(true);
        let n = finish(&NativeSim(&e), &s, &o, 0)
            .unwrap()
            .expect_err("no run");
        assert_eq!(n.taken.len(), 2, "{n:?}");
        assert!(!n.at_limit);
    }
}
