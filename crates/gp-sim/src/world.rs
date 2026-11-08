//! The world's state: everything that changes while a game runs, and
//! nothing else. Together with the [`Spec`] it decides every later frame,
//! so its canonical bytes are both the snapshot and what state hashes
//! cover.

use crate::fx::Fx;
use crate::rng::Rng;
use crate::spec::{Kind, Level, Role, Spec, Tile};

/// The canonical state's magic and version. The version is computed from
/// the layout (the code between the two layout marks below), never
/// counted by hand ([`crate::layout`]).
pub const MAGIC: &[u8; 4] = b"GPSM";
pub const VERSION: u32 = crate::layout::VERSION;

/// One entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Body {
    /// An index into [`Spec::kinds`].
    pub kind: usize,
    /// The top left corner, in pixels.
    pub x: Fx,
    pub y: Fx,
    /// Speed in pixels per frame, positive right and down.
    pub vx: Fx,
    pub vy: Fx,
    pub on_ground: bool,
    /// Facing (and, for a patrol, walking) left.
    pub facing_left: bool,
    /// False once stomped, collected, removed or gone behind the view: the
    /// entity no longer moves or touches anything.
    pub alive: bool,
    /// Not yet in the level: waiting for the view to come within the
    /// level's `spawn_ahead`.
    pub waiting: bool,
    /// The state of the entity's own machine, an index into its states,
    /// and the frames since entering it.
    pub state: usize,
    pub time: u32,
    /// The jump band chosen at takeoff, plus one; 0 for none.
    pub band: u8,
    /// Frames since leaving the ground without jumping, for coyote time;
    /// `u32::MAX` after a jump.
    pub air: u32,
    /// Frames a jump pressed in the air is still remembered.
    pub buffer: u32,
    /// In a jump's held phase: `gravity_held` applies.
    pub held_phase: bool,
    /// A path's way back toward its spawn end.
    pub path_back: bool,
    /// Frames a path still waits at its end.
    pub wait: u32,
    /// Where it was placed: a path's spawn end.
    pub home: [Fx; 2],
    /// Mechanic words: an `ext` hook keeps its own counters and timers in
    /// them, or, on an entity with no hook, the components that count
    /// (`hop.pattern`, `leap`, `shuttle`, `orbit`, `orbit_part`).
    pub ext: [i64; 4],
    /// Damage taken, for an entity with its own `health`.
    pub hp: i64,
    /// Frames in which no hurt counts.
    pub invuln: u32,
    /// Frames of knockback left.
    pub knock: u32,
    /// Frames walked from standing, for `run.start`.
    pub steps: u32,
    /// On a ladder.
    pub climbing: bool,
    /// Waiting to respawn: its spawn must leave the view's reach before it
    /// can appear again.
    pub away: bool,
    /// The body it stands on, plus one; 0 for none.
    pub riding: u32,
    /// The body it carries, plus one.
    pub carrying: u32,
    /// The body carrying it, plus one.
    pub carrier: u32,
    /// Frames of lifting left, holding the carrier still.
    pub lift: u32,
    /// Frames counted toward a `shoot.every`.
    pub shot: u32,
    /// With `run.move_step`, the speed's part below a step, from 0 up to
    /// the step: `vx` holds whole steps and the body moves by them, and
    /// this carries over to the next frame's speed.
    pub vx_frac: Fx,
    /// The way it last moved, for the run's options that read it: 0 not
    /// yet, 1 right, 2 left, and with `run.face_both` 3 when a slow skid
    /// turned it to facing both ways.
    pub moved: u8,
    /// Frames the run action still counts after it's let go
    /// (`run.run_hold_frames`).
    pub run_hold: u32,
    /// The fastest of `run.decel_bands` holds (`run.decel_hold_from`).
    pub decel_hold: bool,
    /// The side a wall or the feet pushed it out to on the last frame
    /// that ran its walls, for `run.push_masks`: 0 none, 1 right (out of
    /// a wall on its left), 2 left.
    pub pushed: u8,
    /// With `jump.keep_fall_gravity`, the gravity its falls use: its last
    /// jump's, or the start's until its first.
    pub fall_gravity: Fx,
    /// With `bounce`, it bounced on the last frame (platformer 0.4.10).
    pub bounced: bool,
    /// With `run.speed_before_walls`, the speed without its sign (whole
    /// steps with `move_step`) as the run last left it, before a wall
    /// stopped it: what its speed tests and the jump's bands read.
    pub seen_speed: Fx,
    /// With `run.face_both`, it last faced both ways: both directions were
    /// held where the facing turns. `facing_left` is false then.
    pub faced_both: bool,
    /// With `climb.down_through_step`: its last move across stepped it up
    /// a surface (a pixel up a sloped girder), so the ground test that
    /// ran between the move across and the step up found no ground.
    pub stepped_up: bool,
}

impl Body {
    /// In the level and alive: it moves and touches.
    pub fn active(&self) -> bool {
        self.alive && !self.waiting
    }
}

/// One of a level's numbered slots (`slot_rest`, core 0.4.11): the body
/// in it, and what the last body to leave it left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Slot {
    /// The body's index plus one, 0 while the slot is free.
    pub body: u32,
    /// The fraction of the last body's place across, its speed up and down
    /// and the fraction of its height, when it left the slot.
    pub rest: [Fx; 3],
    /// A number that belongs to the slot and outlives its bodies: a hook
    /// writes it ([`crate::ExtCx::set_slot_word`]), as a game's per-slot
    /// timer that no object's setup clears.
    pub word: i64,
}

/// One of a shooter's numbered slots (`shoot.in_slots`, platformer
/// 0.4.12): the shot in it, and what the last shot to leave it left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShotSlot {
    /// The body's index plus one, 0 while the slot is free.
    pub body: u32,
    /// The fractions of the last shot's place across, its place down and
    /// its speed down, when it left the slot.
    pub rest: [Fx; 3],
}

/// A state machine's current state and the frames since it entered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MachineState {
    pub state: usize,
    pub time: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct World {
    /// Frames run since power-on; frame 1 is the first step.
    pub frame: u64,
    /// An index into [`Spec::levels`].
    pub level: usize,
    /// The level last started (by power-on, `next-level` or `reset-game`),
    /// which `restart-level` starts again after the player took an exit.
    pub home_level: usize,
    pub rng: Rng,
    /// Controller bits held this frame, and the frame before.
    pub held: u8,
    pub prev: u8,
    /// Every entity of the level, in spawn order, dead ones included, so an
    /// entity's index is its ID for the whole level.
    pub bodies: Vec<Body>,
    /// Each camera's top left corner.
    pub cameras: Vec<[Fx; 2]>,
    /// Each camera's followed entity's whole-pixel place in the view when
    /// the camera last moved (the entity's position and the corner, each
    /// rounded down, apart), which a camera `by_move` reads.
    pub seen: Vec<[Fx; 2]>,
    /// Each camera's frames left that a `by_move` with `push_hold` doesn't
    /// move, counted down at the start of each camera step: set after its
    /// followed entity is pushed out of a wall.
    pub camera_hold: Vec<u32>,
    /// Each camera's followed entity's last whole-pixel move across and
    /// down, as a camera `by_move` saw it on the last frame the entity's
    /// system ran, which one with `keep_move` moves by again while a state
    /// stops that system (core 0.4.12).
    pub last_move: Vec<[i64; 2]>,
    /// With `keep_move`, each camera's followed entity's whole-pixel place
    /// when the camera last noted its move, which the next move is
    /// measured from (core 0.4.12).
    pub follow_at: Vec<[i64; 2]>,
    pub machines: Vec<MachineState>,
    pub counters: Vec<i64>,
    /// Frames counted toward each `every` change, in counter order.
    pub ticks: Vec<u32>,
    /// Cells changed since the level was placed, by cell index, in order:
    /// a tile a hook replaced.
    pub tiles: Vec<(u32, Tile)>,
    /// Cells a head hit holds solid (`hold`), in the order hit: the cell
    /// index, the kind it takes next and the frames left, counted down at
    /// the start of each frame that isn't frozen; at 0 it takes the kind.
    pub held_cells: Vec<(u32, Tile, u32)>,
    /// Frames left of an exit's scroll, which freeze the world.
    pub scroll: u32,
    /// A camera that moves in steps is more than a step from where it's
    /// going, and the level holds the player meanwhile.
    pub hold: bool,
    /// A level with `draw`: the steps of the view's near edge the drawn
    /// columns have reached (the near edge over `step`, rounded down, when
    /// the last set started), and the frames left of the set being drawn,
    /// 0 when none is. Both 0 in a level without `draw`.
    pub draw_set: i64,
    pub draw_left: u32,
    /// A level with `slot_rest`: its `spawn_slots` numbered slots, empty
    /// otherwise.
    pub slots: Vec<Slot>,
    /// Counter changes waiting for their `after` (core 0.4.13), in the
    /// order their events fired: the counter's index, the amount and the
    /// frames left.
    pub pending: Vec<(u32, i64, u32)>,
    /// The numbered slots of a `shoot` with `in_slots`, made by the first
    /// shot or a translation. They keep their rests across levels.
    pub shots: Vec<ShotSlot>,
    /// How many shots have taken a slot, whose remainder by the slot count
    /// is the slot whose turn it is.
    pub shot_turn: u32,
    /// The pairs of bodies that `walk.bump` found touching and turned, by
    /// index, the earlier first, until a test finds them apart.
    pub bumps: Vec<(u32, u32)>,
    /// The spec's profile, from [`Spec::profile`]: not in the canonical
    /// state, which is the same in every profile, but in the hash.
    pub profile: Option<String>,
}

/// Bytes of one entity in the canonical state.
pub const BODY_BYTES: usize = 176;

/// A level's tiles as the game stands: the built cells with the world's
/// changes over them.
#[derive(Debug, Clone, Copy)]
pub struct Tiles<'a> {
    pub level: &'a Level,
    pub changed: &'a [(u32, Tile)],
    /// The world's bodies, whose kinds in the level's `ground_bodies` are
    /// ground too (platformer `level.ground_bodies`); empty where only the
    /// tiles are asked.
    pub bodies: &'a [Body],
}

impl<'a> Tiles<'a> {
    /// The level as built, with no changes.
    pub fn built(level: &'a Level) -> Self {
        Tiles {
            level,
            changed: &[],
            bodies: &[],
        }
    }

    /// The kind of the cell the pixel at `x`, `y` is in. Everything outside
    /// the grid is empty.
    pub fn at(&self, x: i64, y: i64) -> Tile {
        let Some(c) = self.level.cell(x, y) else {
            return Tile::Empty;
        };
        match self.changed.binary_search_by_key(&(c as u32), |(i, _)| *i) {
            Ok(i) => self.changed[i].1,
            Err(_) => self.level.tiles[c],
        }
    }

    pub fn solid_at(&self, x: i64, y: i64) -> bool {
        self.at(x, y).solid()
    }

    /// Whether the pixel at `x`, `y` stops a body moving down onto it
    /// whose bottom was at `from` (the row just below it) before the move:
    /// a solid cell, or a top-only cell (or, with `ladder_tops`, a ladder
    /// with no ladder above) whose top row `from` wasn't past.
    pub fn lands_at(&self, x: i64, y: i64, from: i64, ladder_tops: bool) -> bool {
        // With a surface (core `level.tile_surfaces`) a top-only or ladder
        // cell stands at that row inside it: the body lands once its
        // bottom reaches the row, having been above it.
        if let (Tile::Top | Tile::Ladder, Some(s)) = (self.at(x, y), self.level.surface(x, y)) {
            let top = self.cell_top(y) + s;
            return y >= top && from <= top;
        }
        match self.at(x, y) {
            t if t.solid() => true,
            Tile::Top => from <= self.cell_top(y),
            Tile::Ladder if ladder_tops => {
                let top = self.cell_top(y);
                from <= top && self.at(x, top - 1) != Tile::Ladder
            }
            _ => false,
        }
    }

    /// The row a body landing at row `y` stands on, its bottom having been
    /// at row `from`: the cell's top, or in a level with surfaces the
    /// highest surface across `xs` that it reached from above (one it
    /// stands on, for a body on the ground, not a higher one beside it).
    pub fn land_top(&self, xs: impl Iterator<Item = i64>, y: i64, from: i64) -> i64 {
        let top = self.cell_top(y);
        xs.filter_map(|x| match self.at(x, y) {
            Tile::Top | Tile::Ladder => self.level.surface(x, y).map(|s| top + s),
            _ => None,
        })
        .filter(|t| (from..=y).contains(t))
        .min()
        .unwrap_or(top)
    }

    /// The top pixel row of the cell row `y` is in.
    pub fn cell_top(&self, y: i64) -> i64 {
        y.div_euclid(self.level.tile_size) * self.level.tile_size
    }
}

/// The fall gravity a body of this kind starts with, at a level start or
/// through an exit: `jump.start_fall_gravity` with `keep_fall_gravity`,
/// else 0, which nothing reads.
pub fn start_fall_gravity(k: &Kind) -> Fx {
    match &k.jump {
        Some(j) if j.keep_fall_gravity => j.start_fall_gravity.unwrap_or(j.gravity),
        _ => Fx::ZERO,
    }
}

fn bands_of(k: &Kind) -> usize {
    k.jump.as_ref().map_or(0, |j| j.bands.len())
}

/// The body of a spawn as its level places it.
pub(crate) fn placed_body(spec: &Spec, level: &Level, s: &crate::spec::Spawn) -> Body {
    let base = &spec.kinds[s.kind];
    let mut b = Body {
        kind: s.kind,
        x: s.x,
        y: s.y,
        alive: true,
        waiting: level.spawn_ahead.is_some() && base.role != Role::Player,
        state: base.machine.as_ref().map_or(0, |m| m.initial),
        home: [s.x, s.y],
        ..Body::default()
    };
    let kind = spec.kind(&b);
    b.fall_gravity = start_fall_gravity(kind);
    b.on_ground = kind.tile_collision && ground_under(level, kind, &b);
    // A walker starts facing the player (the level's first spawn of a
    // player), and a flier as its heading says.
    let player = level
        .spawns
        .iter()
        .find(|p| spec.kinds[p.kind].role == Role::Player)
        .map(|p| p.x);
    crate::systems::face(kind, &mut b, player);
    b
}

/// How many `every` changes the spec's counters have.
pub fn tick_count(spec: &Spec) -> usize {
    spec.counters
        .iter()
        .flat_map(|c| &c.changes)
        .filter(|c| matches!(c, crate::spec::Change::Every(..)))
        .count()
}

impl World {
    /// The state at power-on: the start level as its spawns place it, with
    /// `level.started` taken.
    pub fn power_on(spec: &Spec) -> Self {
        let mut w = World {
            frame: 0,
            level: spec.start_level,
            home_level: spec.start_level,
            rng: Rng::new(spec.seed),
            machines: spec
                .machines
                .iter()
                .map(|m| MachineState {
                    state: m.initial,
                    time: 0,
                })
                .collect(),
            counters: spec.counters.iter().map(|c| c.start).collect(),
            ticks: vec![0; tick_count(spec)],
            profile: spec.profile.clone(),
            ..World::default()
        };
        let mut f = crate::systems::Frame::default();
        crate::systems::start_level(spec, &mut w, &mut f);
        w
    }

    /// Puts every entity at its spawn and every camera at its start, with
    /// the level's tiles as built.
    pub(crate) fn place(&mut self, spec: &Spec) {
        self.tiles.clear();
        self.held_cells.clear();
        self.pending.clear();
        self.bumps.clear();
        for s in &mut self.shots {
            s.body = 0;
        }
        self.hold = false;
        let level = &spec.levels[self.level];
        self.slots = match level.spawn_slots {
            Some(n) if level.slot_rest => vec![Slot::default(); n as usize],
            _ => Vec::new(),
        };
        self.bodies = level
            .spawns
            .iter()
            .map(|s| placed_body(spec, level, s))
            .collect();
        self.cameras = vec![[Fx::ZERO; 2]; spec.cameras.len()];
        self.seen = vec![[Fx::ZERO; 2]; spec.cameras.len()];
        self.camera_hold = vec![0; spec.cameras.len()];
        self.last_move = vec![[0; 2]; spec.cameras.len()];
        self.follow_at = vec![[0; 2]; spec.cameras.len()];
        for i in 0..spec.cameras.len() {
            crate::systems::place_camera(spec, self, i, true);
        }
        crate::systems::start_draw(spec, self);
    }

    /// The level's tiles as the game stands.
    pub fn tiles<'a>(&'a self, spec: &'a Spec) -> Tiles<'a> {
        Tiles {
            level: &spec.levels[self.level],
            changed: &self.tiles,
            bodies: &self.bodies,
        }
    }

    /// The kind of the tile at a pixel of the current level, changes
    /// included.
    pub fn tile_at(&self, spec: &Spec, x: i64, y: i64) -> Tile {
        self.tiles(spec).at(x, y)
    }

    /// Changes the cell at a pixel of the current level to `tile`, until
    /// the level is placed again. A pixel outside the grid changes nothing.
    pub fn set_tile(&mut self, spec: &Spec, x: i64, y: i64, tile: Tile) {
        let level = &spec.levels[self.level];
        let Some(c) = level.cell(x, y) else { return };
        let c = c as u32;
        match self.tiles.binary_search_by_key(&c, |(i, _)| *i) {
            Ok(i) if level.tiles[c as usize] == tile => {
                self.tiles.remove(i);
            }
            Ok(i) => self.tiles[i].1 = tile,
            Err(_) if level.tiles[c as usize] == tile => {}
            Err(i) => self.tiles.insert(i, (c, tile)),
        }
    }

    /// As `set_tile`, but the cell stays a change even when `tile` is its
    /// built kind: the game wrote it, so presentation draws it by its kind.
    pub fn write_tile(&mut self, spec: &Spec, x: i64, y: i64, tile: Tile) {
        let level = &spec.levels[self.level];
        let Some(c) = level.cell(x, y) else { return };
        let c = c as u32;
        match self.tiles.binary_search_by_key(&c, |(i, _)| *i) {
            Ok(i) => self.tiles[i].1 = tile,
            Err(i) => self.tiles.insert(i, (c, tile)),
        }
    }

    /// The bodies of one kind, with their indexes.
    pub fn of_kind(&self, kind: usize) -> impl Iterator<Item = (usize, &Body)> {
        self.bodies
            .iter()
            .enumerate()
            .filter(move |(_, b)| b.kind == kind)
    }

    /// Whether `action` is held this frame.
    pub fn held(&self, spec: &Spec, action: usize) -> bool {
        self.held & spec.actions[action].mask != 0
    }

    /// Whether `action` is held this frame and wasn't the frame before.
    pub fn pressed(&self, spec: &Spec, action: usize) -> bool {
        let mask = spec.actions[action].mask;
        self.held & mask != 0 && self.prev & mask == 0
    }

    // state layout: begin
    /// The canonical state: a header (magic `GPSM`, version), then every
    /// field in order, little-endian (docs/interpreter.md).
    pub fn canonical(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.bodies.len() * BODY_BYTES);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.frame.to_le_bytes());
        out.extend_from_slice(&(self.level as u32).to_le_bytes());
        out.extend_from_slice(&(self.home_level as u32).to_le_bytes());
        out.extend_from_slice(&self.rng.state.to_le_bytes());
        out.push(self.held);
        out.push(self.prev);
        out.extend_from_slice(&(self.bodies.len() as u32).to_le_bytes());
        for b in &self.bodies {
            out.extend_from_slice(&(b.kind as u32).to_le_bytes());
            out.extend_from_slice(&(b.state as u32).to_le_bytes());
            out.extend_from_slice(&b.time.to_le_bytes());
            for v in [b.x, b.y, b.vx, b.vy] {
                out.extend_from_slice(&v.raw().to_le_bytes());
            }
            out.push(
                u8::from(b.on_ground)
                    | u8::from(b.facing_left) << 1
                    | u8::from(b.alive) << 2
                    | u8::from(b.waiting) << 3
                    | u8::from(b.held_phase) << 4
                    | u8::from(b.path_back) << 5
                    | u8::from(b.climbing) << 6
                    | u8::from(b.away) << 7,
            );
            out.push(b.band);
            out.extend_from_slice(&b.air.to_le_bytes());
            out.extend_from_slice(&b.buffer.to_le_bytes());
            for word in b.ext {
                out.extend_from_slice(&word.to_le_bytes());
            }
            out.extend_from_slice(&b.wait.to_le_bytes());
            for v in b.home {
                out.extend_from_slice(&v.raw().to_le_bytes());
            }
            out.extend_from_slice(&b.hp.to_le_bytes());
            for v in [
                b.invuln, b.knock, b.steps, b.riding, b.carrying, b.carrier, b.lift, b.shot,
                b.run_hold,
            ] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&b.vx_frac.raw().to_le_bytes());
            out.push(
                b.moved
                    | u8::from(b.decel_hold) << 2
                    | b.pushed << 3
                    | u8::from(b.faced_both) << 5
                    | u8::from(b.stepped_up) << 6,
            );
            out.extend_from_slice(&b.fall_gravity.raw().to_le_bytes());
            out.push(u8::from(b.bounced));
            out.extend_from_slice(&b.seen_speed.raw().to_le_bytes());
        }
        out.extend_from_slice(&(self.cameras.len() as u32).to_le_bytes());
        for (i, c) in self.cameras.iter().enumerate() {
            let seen = self.seen.get(i).copied().unwrap_or([Fx::ZERO; 2]);
            for v in [c[0], c[1], seen[0], seen[1]] {
                out.extend_from_slice(&v.raw().to_le_bytes());
            }
            let hold = self.camera_hold.get(i).copied().unwrap_or(0);
            out.extend_from_slice(&hold.to_le_bytes());
            for v in self.last_move.get(i).copied().unwrap_or([0; 2]) {
                out.extend_from_slice(&v.to_le_bytes());
            }
            for v in self.follow_at.get(i).copied().unwrap_or([0; 2]) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out.extend_from_slice(&(self.machines.len() as u32).to_le_bytes());
        for m in &self.machines {
            out.extend_from_slice(&(m.state as u32).to_le_bytes());
            out.extend_from_slice(&m.time.to_le_bytes());
        }
        out.extend_from_slice(&(self.counters.len() as u32).to_le_bytes());
        for c in &self.counters {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&(self.ticks.len() as u32).to_le_bytes());
        for t in &self.ticks {
            out.extend_from_slice(&t.to_le_bytes());
        }
        out.extend_from_slice(&(self.tiles.len() as u32).to_le_bytes());
        for (c, t) in &self.tiles {
            out.extend_from_slice(&c.to_le_bytes());
            out.push(*t as u8);
        }
        out.extend_from_slice(&(self.held_cells.len() as u32).to_le_bytes());
        for (c, t, left) in &self.held_cells {
            out.extend_from_slice(&c.to_le_bytes());
            out.push(*t as u8);
            out.extend_from_slice(&left.to_le_bytes());
        }
        out.extend_from_slice(&self.scroll.to_le_bytes());
        out.push(u8::from(self.hold));
        out.extend_from_slice(&self.draw_set.to_le_bytes());
        out.extend_from_slice(&self.draw_left.to_le_bytes());
        out.extend_from_slice(&(self.slots.len() as u32).to_le_bytes());
        for s in &self.slots {
            out.extend_from_slice(&s.body.to_le_bytes());
            out.extend_from_slice(&s.rest[0].raw().to_le_bytes());
            out.extend_from_slice(&s.rest[1].raw().to_le_bytes());
            out.extend_from_slice(&s.rest[2].raw().to_le_bytes());
            out.extend_from_slice(&s.word.to_le_bytes());
        }
        out.extend_from_slice(&(self.pending.len() as u32).to_le_bytes());
        for (c, by, left) in &self.pending {
            out.extend_from_slice(&c.to_le_bytes());
            out.extend_from_slice(&by.to_le_bytes());
            out.extend_from_slice(&left.to_le_bytes());
        }
        out.extend_from_slice(&(self.shots.len() as u32).to_le_bytes());
        for s in &self.shots {
            out.extend_from_slice(&s.body.to_le_bytes());
            for r in &s.rest {
                out.extend_from_slice(&r.raw().to_le_bytes());
            }
        }
        out.extend_from_slice(&self.shot_turn.to_le_bytes());
        out.extend_from_slice(&(self.bumps.len() as u32).to_le_bytes());
        for (a, b) in &self.bumps {
            out.extend_from_slice(&a.to_le_bytes());
            out.extend_from_slice(&b.to_le_bytes());
        }
        out
    }

    /// Reads canonical state written by [`World::canonical`] for this spec.
    /// Refuses bytes of another version, length or game.
    pub fn from_canonical(spec: &Spec, bytes: &[u8]) -> Result<Self, String> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(4)? != MAGIC {
            return Err("not a model state (no GPSM header)".into());
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(format!(
                "model state version {version}; this build reads {VERSION}"
            ));
        }
        let mut w = World {
            frame: r.u64()?,
            level: r.index(spec.levels.len(), "level")?,
            home_level: r.index(spec.levels.len(), "level")?,
            rng: Rng::new(r.u64()?),
            held: r.u8()?,
            prev: r.u8()?,
            profile: spec.profile.clone(),
            ..World::default()
        };
        let n = r.count(BODY_BYTES)?;
        for _ in 0..n {
            let kind = r.index(spec.kinds.len(), "entity kind")?;
            let base = &spec.kinds[kind];
            let states = base.machine.as_ref().map_or(1, |m| m.states.len());
            let state = r.index(states, "entity state")?;
            let time = r.u32()?;
            let [x, y, vx, vy] = [r.fx()?, r.fx()?, r.fx()?, r.fx()?];
            let flags = r.u8()?;
            let band = r.u8()?;
            let bands = match &base.machine {
                Some(m) => m
                    .states
                    .iter()
                    .map(|s| bands_of(&s.kind))
                    .max()
                    .unwrap_or(0),
                None => bands_of(base),
            };
            if usize::from(band) > bands {
                return Err(format!("jump band {band} is past the model's {bands}"));
            }
            let (air, buffer) = (r.u32()?, r.u32()?);
            let ext = [
                r.u64()? as i64,
                r.u64()? as i64,
                r.u64()? as i64,
                r.u64()? as i64,
            ];
            let wait = r.u32()?;
            let home = [r.fx()?, r.fx()?];
            let hp = r.u64()? as i64;
            let [
                invuln,
                knock,
                steps,
                riding,
                carrying,
                carrier,
                lift,
                shot,
                run_hold,
            ] = [
                r.u32()?,
                r.u32()?,
                r.u32()?,
                r.u32()?,
                r.u32()?,
                r.u32()?,
                r.u32()?,
                r.u32()?,
                r.u32()?,
            ];
            let vx_frac = r.fx()?;
            let run_flags = r.u8()?;
            let fall_gravity = r.fx()?;
            let bounced = match r.u8()? {
                0 => false,
                1 => true,
                v => return Err(format!("bounce flag {v} isn't 0 or 1")),
            };
            let seen_speed = r.fx()?;
            let moved = run_flags & 3;
            let pushed = run_flags >> 3 & 3;
            if pushed == 3 || run_flags >> 7 != 0 {
                return Err(format!("run flags {run_flags:#x} aren't a body's"));
            }
            w.bodies.push(Body {
                faced_both: run_flags & 32 != 0,
                stepped_up: run_flags & 64 != 0,
                bounced,
                fall_gravity,
                seen_speed,
                vx_frac,
                moved,
                run_hold,
                decel_hold: run_flags & 4 != 0,
                pushed,
                hp,
                invuln,
                knock,
                steps,
                riding,
                carrying,
                carrier,
                lift,
                shot,
                climbing: flags & 64 != 0,
                away: flags & 128 != 0,
                kind,
                x,
                y,
                vx,
                vy,
                on_ground: flags & 1 != 0,
                facing_left: flags & 2 != 0,
                alive: flags & 4 != 0,
                waiting: flags & 8 != 0,
                held_phase: flags & 16 != 0,
                path_back: flags & 32 != 0,
                wait,
                home,
                state,
                time,
                band,
                air,
                buffer,
                ext,
            });
        }
        let n = r.count(68)?;
        if n != spec.cameras.len() {
            return Err(format!("{n} cameras; the model has {}", spec.cameras.len()));
        }
        for _ in 0..n {
            w.cameras.push([r.fx()?, r.fx()?]);
            w.seen.push([r.fx()?, r.fx()?]);
            w.camera_hold.push(r.u32()?);
            w.last_move.push([r.u64()? as i64, r.u64()? as i64]);
            w.follow_at.push([r.u64()? as i64, r.u64()? as i64]);
        }
        let n = r.count(8)?;
        if n != spec.machines.len() {
            return Err(format!("{n} rules; the model has {}", spec.machines.len()));
        }
        for m in &spec.machines {
            w.machines.push(MachineState {
                state: r.index(m.states.len(), "rule state")?,
                time: r.u32()?,
            });
        }
        let n = r.count(8)?;
        if n != spec.counters.len() {
            return Err(format!(
                "{n} counters; the model has {}",
                spec.counters.len()
            ));
        }
        for _ in 0..n {
            w.counters.push(r.u64()? as i64);
        }
        let n = r.count(4)?;
        if n != tick_count(spec) {
            return Err(format!(
                "{n} counter clocks; the model has {}",
                tick_count(spec)
            ));
        }
        for _ in 0..n {
            w.ticks.push(r.u32()?);
        }
        let n = r.count(5)?;
        let cells = spec.levels[w.level].tiles.len();
        for _ in 0..n {
            let c = r.u32()?;
            let t = r.u8()?;
            let t = Tile::from_u8(t).ok_or(format!("tile kind {t} isn't defined"))?;
            if c as usize >= cells || w.tiles.last().is_some_and(|(p, _)| *p >= c) {
                return Err(format!(
                    "changed cell {c} is out of order or past the level"
                ));
            }
            w.tiles.push((c, t));
        }
        let n = r.count(9)?;
        for _ in 0..n {
            let c = r.u32()?;
            let t = r.u8()?;
            let t = Tile::from_u8(t).ok_or(format!("tile kind {t} isn't defined"))?;
            if c as usize >= cells {
                return Err(format!("held cell {c} is past the level"));
            }
            w.held_cells.push((c, t, r.u32()?));
        }
        w.scroll = r.u32()?;
        w.hold = match r.u8()? {
            0 => false,
            1 => true,
            v => return Err(format!("hold flag {v} isn't 0 or 1")),
        };
        w.draw_set = r.u64()? as i64;
        w.draw_left = r.u32()?;
        let n = r.count(28)?;
        for _ in 0..n {
            let body = r.u32()?;
            if body as usize > w.bodies.len() {
                return Err(format!("slot body {body}; there are {}", w.bodies.len()));
            }
            w.slots.push(Slot {
                body,
                rest: [r.fx()?, r.fx()?, r.fx()?],
                word: r.u64()? as i64,
            });
        }
        let n = r.count(16)?;
        for _ in 0..n {
            let c = r.u32()?;
            if c as usize >= spec.counters.len() {
                return Err(format!(
                    "pending change to counter {c}; the model has {}",
                    spec.counters.len()
                ));
            }
            w.pending.push((c, r.u64()? as i64, r.u32()?));
        }
        let n = r.count(28)?;
        for _ in 0..n {
            let body = r.u32()?;
            if body as usize > w.bodies.len() {
                return Err(format!("shot body {body}; there are {}", w.bodies.len()));
            }
            w.shots.push(ShotSlot {
                body,
                rest: [r.fx()?, r.fx()?, r.fx()?],
            });
        }
        w.shot_turn = r.u32()?;
        let n = r.count(8)?;
        for _ in 0..n {
            let (a, b) = (r.u32()?, r.u32()?);
            if a.max(b) as usize >= w.bodies.len() {
                return Err(format!(
                    "bump of bodies {a} and {b}; there are {}",
                    w.bodies.len()
                ));
            }
            w.bumps.push((a, b));
        }
        if r.at != bytes.len() {
            return Err(format!(
                "{} bytes left over after the model state",
                bytes.len() - r.at
            ));
        }
        Ok(w)
    }
    // state layout: end

    /// The BLAKE3 hash of the canonical state, what recordings store for
    /// each frame. In a profile other than the base, the hash is of
    /// `gp-profile `, the profile's name and a zero byte before the state,
    /// so runs in two profiles never share a hash; the base's is the state
    /// alone, as before profiles.
    pub fn hash(&self) -> [u8; 32] {
        match &self.profile {
            None => *blake3::hash(&self.canonical()).as_bytes(),
            Some(p) => {
                let mut h = blake3::Hasher::new();
                h.update(b"gp-profile ");
                h.update(p.as_bytes());
                h.update(&[0]);
                h.update(&self.canonical());
                *h.finalize().as_bytes()
            }
        }
    }
}

/// Whether ground is right under a body's hitbox, in the level as built:
/// [`ground_under_t`] with no changed tiles.
pub fn ground_under(level: &Level, kind: &Kind, b: &Body) -> bool {
    ground_under_t(Tiles::built(level), kind, b)
}

/// Whether ground is right under a body's hitbox: in the row of pixels
/// just below it, at any column it covers, a solid tile, or the top row of
/// a top-only tile (and of a ladder's top, for a climber that stands on
/// them while it isn't climbing).
pub fn ground_under_t(t: Tiles<'_>, kind: &Kind, b: &Body) -> bool {
    let row = b.y.floor() + kind.tile_hitbox.y.floor() + kind.tile_hitbox.h.floor();
    lands_row(t, kind, b, row, row)
}

/// Whether the row of pixels `row` stops the body landing anywhere across
/// its tile hitbox, its bottom having been at `from` before it moved.
pub fn lands_row(t: Tiles<'_>, kind: &Kind, b: &Body, row: i64, from: i64) -> bool {
    let ladder_tops = !b.climbing && kind.climb.as_ref().is_some_and(|c| c.stand_on_top);
    // With ground points the ground is looked for at those columns only.
    let feet = kind.jump.as_ref().and_then(|j| j.feet.as_ref());
    if let Some(feet) = feet.or(kind.fall.as_ref().and_then(|f| f.feet.as_ref())) {
        return feet
            .iter()
            .any(|fx| t.lands_at(b.x.floor() + fx.floor(), row, from, ladder_tops));
    }
    let left = b.x.floor() + kind.tile_hitbox.x.floor();
    let right = left + kind.tile_hitbox.w.floor() - 1;
    span_solid(t.level, left, right, |x| {
        t.lands_at(x, row, from, ladder_tops)
    })
}

/// Core `level.tile_surfaces`: whether a body standing on ground stands at
/// `row`, on any column of its tile hitbox (not only its `feet`, which say
/// where a body in the air lands).
pub fn stands_row(t: Tiles<'_>, kind: &Kind, b: &Body, row: i64, from: i64) -> bool {
    let ladder_tops = !b.climbing && kind.climb.as_ref().is_some_and(|c| c.stand_on_top);
    let left = b.x.floor() + kind.tile_hitbox.x.floor();
    let right = left + kind.tile_hitbox.w.floor() - 1;
    (left..=right).any(|x| t.lands_at(x, row, from, ladder_tops))
}

/// Core `level.tile_surfaces`: a body standing on ground, moved across,
/// follows the surface under it up or down by at most [`SURFACE_STEP`]
/// pixels (Donkey Kong's sloped girders, a pixel every 24). It keeps its
/// height while any column of its tile hitbox stands on it, then goes to
/// the highest surface the hitbox covers. A step only goes to a row with a
/// surface.
pub fn follow_surface(t: Tiles<'_>, kind: &Kind, b: &mut Body) {
    if !b.on_ground || !kind.tile_collision || b.climbing {
        return;
    }
    // Platformer `level.ground`: the feet keep the height while they meet
    // a box on the row under the tile hitbox; meeting none, they go to the
    // nearest row up or down whose box they meet (Donkey Kong's $D91A and
    // $D8EB, after each move across).
    if !t.level.ground.is_empty() {
        let below = b.y.floor() + kind.tile_hitbox.y.floor() + kind.tile_hitbox.h.floor();
        // Platformer `level.ground_bodies`: the walk's test after its move
        // across starts or ends a ride.
        let moving = !t.level.ground_bodies.is_empty();
        if let Some(hit) = ground_at(t, b, below) {
            if moving {
                ride_by(b, hit);
            }
            b.stepped_up = false;
            return;
        }
        if moving {
            ride_by(b, GroundHit::None);
        }
        // Platformer `level.ground_boxes`: no step where the feet meet none.
        if t.level.ground_boxes && moving {
            return;
        }
        let on = |row: i64| on_ground_box(t.level, b, row);
        for d in (1..=SURFACE_STEP).flat_map(|d| [-d, d]) {
            if on(below + d) {
                b.y = b.y.add(Fx::from_int(d));
                b.stepped_up = d < 0;
                return;
            }
        }
        return;
    }
    if t.level.surfaces.is_empty() {
        return;
    }
    let below = b.y.floor() + kind.tile_hitbox.y.floor() + kind.tile_hitbox.h.floor();
    let left = b.x.floor() + kind.tile_hitbox.x.floor();
    let right = left + kind.tile_hitbox.w.floor() - 1;
    // It keeps its height while any column of its tile hitbox still stands
    // on it, so a step comes once the hitbox has wholly left the surface
    // it stood on, whichever way it walks; then the highest surface found.
    let order = std::iter::once(0).chain((-SURFACE_STEP..=SURFACE_STEP).filter(|d| *d != 0));
    for d in order {
        let row = below + d;
        let surfaced = (left..=right).any(|x| t.level.surface(x, row).is_some());
        if (d == 0 || surfaced) && stands_row(t, kind, b, row, row) {
            b.y = b.y.add(Fx::from_int(d));
            b.stepped_up = d < 0;
            return;
        }
    }
}

/// Platformer `level.ground`: whether the body's feet (`ground_feet` from
/// its x) meet one of the level's ground boxes on `row`, edges included.
pub fn on_ground_box(level: &Level, b: &Body, row: i64) -> bool {
    let [f0, f1] = level.ground_feet;
    let (l, r) = (b.x.floor() + f0, b.x.floor() + f1);
    level
        .ground
        .iter()
        .any(|g| g[1] <= row && row <= g[3] && g[0] <= r && l <= g[2])
}

/// What a ground test meets (platformer `level.ground` and
/// `level.ground_bodies`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundHit {
    /// No box.
    None,
    /// One of the level's own boxes.
    Level,
    /// The box of this body, a kind in the level's `ground_bodies`.
    Body(usize),
}

/// Platformer `level.ground_bodies`: the boxes of the bodies that are
/// ground, `(body, [x0, y0, x1, y1])`, in the bodies' order.
pub fn ground_body_boxes<'a>(t: Tiles<'a>) -> impl Iterator<Item = (usize, [i64; 4])> + 'a {
    let kinds = &t.level.ground_bodies;
    t.bodies
        .iter()
        .enumerate()
        .filter(move |(_, b)| b.alive && !b.waiting && !kinds.is_empty())
        .filter_map(move |(i, b)| {
            let (_, e) = kinds.iter().find(|(k, _)| *k == b.kind)?;
            let (x, y) = (b.x.floor(), b.y.floor());
            Some((i, [x + e[0], y + e[1], x + e[2], y + e[3]]))
        })
}

/// Platformer `level.ground` and `level.ground_bodies`: what the body's
/// feet (`ground_feet` from its x) meet on `row`, edges included: the
/// level's boxes first, then the ground bodies'.
pub fn ground_at(t: Tiles<'_>, b: &Body, row: i64) -> Option<GroundHit> {
    if on_ground_box(t.level, b, row) {
        return Some(GroundHit::Level);
    }
    let [f0, f1] = t.level.ground_feet;
    let (l, r) = (b.x.floor() + f0, b.x.floor() + f1);
    ground_body_boxes(t)
        .find(|(_, g)| g[1] <= row && row <= g[3] && g[0] <= r && l <= g[2])
        .map(|(i, _)| GroundHit::Body(i))
}

/// Platformer `level.ground_bodies`: a ground test's result on the ride.
/// A ground body starts a ride on it, no box ends one, and one of the
/// level's boxes leaves it as it was (Donkey Kong's $D326 runs only when
/// the stage's boxes meet nothing, $D316).
pub fn ride_by(b: &mut Body, hit: GroundHit) {
    match hit {
        GroundHit::Body(i) => b.riding = i as u32 + 1,
        GroundHit::None => b.riding = 0,
        GroundHit::Level => {}
    }
}

/// Platformer `level.ground_bodies`: whether the body rides a ground body
/// (one that may be gone since), and so stands without a test.
pub fn rides_ground(t: Tiles<'_>, b: &Body) -> bool {
    b.riding > 0
        && t.bodies
            .get(b.riding as usize - 1)
            .is_some_and(|r| t.level.ground_bodies.iter().any(|(k, _)| *k == r.kind))
}

/// Platformer `level.ground_land` with `level.ground_bodies`: as
/// [`ground_land_row`], then the ground bodies' boxes in the bodies'
/// order; the row it lands on and what it met.
pub fn ground_land_hit(
    t: Tiles<'_>,
    b: &Body,
    row: i64,
    reach: i64,
    holes: &[[i64; 4]],
) -> Option<(i64, GroundHit)> {
    if let Some(top) = ground_land_row(t.level, b, row, reach, holes) {
        return Some((top, GroundHit::Level));
    }
    let [f0, f1] = t.level.ground_feet;
    let (l, r) = (b.x.floor() + f0, b.x.floor() + f1);
    ground_body_boxes(t)
        .find(|(_, g)| g[1] <= row && row - reach <= g[3] && g[0] <= r && l <= g[2])
        .map(|(i, g)| (g[1].max(row - reach), GroundHit::Body(i)))
}

/// Platformer `level.ground_land`: the row of the first ground box, in the
/// level's order, that the body's feet (`ground_feet` from its x) meet on a
/// row from `reach` pixels above `row` down to `row`, edges included.
/// Donkey Kong's jump tests Mario's feet box, a row 3 pixels deep, against
/// the girder pieces of his floor in their list's order ($D91A) and puts
/// him on the first it meets ($D5D3).
///
/// A box is passed on the rows of one of `holes` (platformer
/// `level.pickup_holes`) whose columns hold the feet whole.
pub fn ground_land_row(
    level: &Level,
    b: &Body,
    row: i64,
    reach: i64,
    holes: &[[i64; 4]],
) -> Option<i64> {
    let [f0, f1] = level.ground_feet;
    let (l, r) = (b.x.floor() + f0, b.x.floor() + f1);
    let holed = |g: &[i64; 4]| {
        holes
            .iter()
            .any(|h| h[1] <= g[3] && g[1] <= h[3] && h[0] <= l && r <= h[2])
    };
    level
        .ground
        .iter()
        .find(|g| g[1] <= row && row - reach <= g[3] && g[0] <= r && l <= g[2] && !holed(g))
        .map(|g| g[1].max(row - reach))
}

/// The most pixels [`follow_surface`] moves a body up or down in one move.
pub const SURFACE_STEP: i64 = 2;

/// Whether the row of pixels `row` is solid anywhere across the body's
/// tile hitbox, in the level as built.
pub fn row_solid(level: &Level, kind: &Kind, b: &Body, row: i64) -> bool {
    row_solid_t(Tiles::built(level), kind, b, row)
}

pub fn row_solid_t(t: Tiles<'_>, kind: &Kind, b: &Body, row: i64) -> bool {
    let left = b.x.floor() + kind.tile_hitbox.x.floor();
    let right = left + kind.tile_hitbox.w.floor() - 1;
    span_solid(t.level, left, right, |x| t.solid_at(x, row))
}

/// Whether the column of pixels `col` is solid anywhere down the body's
/// tile hitbox, in the level as built.
pub fn column_solid(level: &Level, kind: &Kind, b: &Body, col: i64) -> bool {
    column_solid_t(Tiles::built(level), kind, b, col)
}

pub fn column_solid_t(t: Tiles<'_>, kind: &Kind, b: &Body, col: i64) -> bool {
    let top = b.y.floor() + kind.tile_hitbox.y.floor();
    let bottom = top + kind.tile_hitbox.h.floor() - 1;
    span_solid(t.level, top, bottom, |y| t.solid_at(col, y))
}

/// Whether any cell the body's hitbox covers is of kind `tile`.
pub fn touches_tile(t: Tiles<'_>, kind: &Kind, b: &Body, tile: Tile) -> bool {
    let h = kind.hitbox;
    let (x0, y0) = (b.x.floor() + h.x.floor(), b.y.floor() + h.y.floor());
    let (x1, y1) = (x0 + h.w.floor() - 1, y0 + h.h.floor() - 1);
    span_solid(t.level, y0, y1, |y| {
        span_solid(t.level, x0, x1, |x| t.at(x, y) == tile)
    })
}

/// The pixel at the middle of a body's hitbox.
pub fn middle(kind: &Kind, b: &Body) -> (i64, i64) {
    let h = kind.hitbox;
    (
        b.x.floor() + h.x.floor() + h.w.floor() / 2,
        b.y.floor() + h.y.floor() + h.h.floor() / 2,
    )
}

/// Checks one pixel per cell from `from` to `to`, and `to` itself.
fn span_solid(level: &Level, from: i64, to: i64, mut solid: impl FnMut(i64) -> bool) -> bool {
    let mut at = from;
    while at < to {
        if solid(at) {
            return true;
        }
        at = (at.div_euclid(level.tile_size) + 1) * level.tile_size;
    }
    solid(to)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.bytes.len())
            .ok_or("the model state is truncated")?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }
    fn fx(&mut self) -> Result<Fx, String> {
        Ok(Fx(self.u64()? as i64))
    }
    fn index(&mut self, len: usize, what: &str) -> Result<usize, String> {
        let i = self.u32()? as usize;
        if i >= len {
            return Err(format!("{what} {i} is past the model's {len}"));
        }
        Ok(i)
    }
    /// A count of items at least `size` bytes each, bounded by what's left.
    fn count(&mut self, size: usize) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n.saturating_mul(size) > self.bytes.len() - self.at {
            return Err("the model state is truncated".into());
        }
        Ok(n)
    }
}
