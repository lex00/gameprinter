//! What a runtime draws each frame (docs/generator.md, "Presentation").
//!
//! A model's presentation (its sprites with their layouts and animation
//! clips, each level's palettes and color cycles, the HUD and the screen)
//! is compiled once, by the generator, into a [`Present`]: plain data that
//! names pictures by number. [`Animator`] steps once per simulation frame,
//! reading the world and never writing it, and [`Animator::scene`] says
//! what to draw: each body's picture, flips and place, the level picture
//! and its cycling colors, and the HUD's tiles. The interpreter preview
//! draws from it.
//!
//! Presentation is the class of change that leaves the state hashes equal
//! (docs/research/mechanics.md): nothing here is read by the simulation.
//! It still keeps to whole numbers and no clock, so two runtimes stepping
//! the same frames pick the same pictures.

use gp_sim::{Spec, World};

pub mod art;
pub mod catalogue;
pub mod looks;
pub mod pad;
pub mod palette;
pub mod raster;
pub mod settings;
pub mod shell;
pub mod sound;
pub mod text;
pub mod view;

/// The screen a game draws to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// Width and height in pixels.
    pub size: [u32; 2],
    /// A pixel's width over its height, as a ratio.
    pub aspect: [u32; 2],
    /// Pixels hidden at the left, top, right and bottom.
    pub overscan: [u32; 4],
    /// The console's sprites per line, when the model draws the limit.
    pub sprite_limit: Option<SpriteLimit>,
    /// Core `present.sprite_x_wraps`: a body's hardware sprite (8 pixels
    /// across) left of the screen is drawn 256 pixels on.
    pub x_wraps: bool,
}

/// At most `per_line` hardware sprites, each `width` pixels wide, on one
/// line; with `flicker` the game turns its sprites' order every frame, so
/// the ones past the limit take turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteLimit {
    pub per_line: u32,
    pub width: u32,
    pub flicker: bool,
}

impl Default for Screen {
    fn default() -> Self {
        Screen {
            size: [256, 240],
            aspect: [1, 1],
            overscan: [0; 4],
            sprite_limit: None,
            x_wraps: false,
        }
    }
}

impl Screen {
    /// The part of the screen that shows: left, top, width, height.
    pub fn visible(&self) -> [u32; 4] {
        let [w, h] = self.size;
        let [l, t, r, b] = self.overscan;
        [l, t, w.saturating_sub(l + r), h.saturating_sub(t + b)]
    }

    /// A window's size, in whole pixels, showing the visible part `scale`
    /// times over with the pixels' shape.
    pub fn window(&self, scale: u32) -> [u32; 2] {
        let [_, _, w, h] = self.visible();
        let [n, d] = self.aspect;
        [
            (u64::from(w) * u64::from(scale) * u64::from(n.max(1)) / u64::from(d.max(1))) as u32,
            h * scale,
        ]
    }
}

/// A picture a runtime loads, by its file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub file: String,
    pub size: [u32; 2],
}

/// A picture to show, by sprite frame number, with whole-picture flips.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frame {
    /// An index into [`Present::frames`].
    pub id: u32,
    pub flip_x: bool,
    pub flip_y: bool,
}

/// Conditions on a body for a clip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct When {
    pub ground: Option<bool>,
    pub moving: Option<bool>,
    pub skidding: Option<bool>,
    pub rising: Option<bool>,
    /// Raw fixed-point speed.
    pub speed_at_least: Option<i64>,
    /// In the air after a jump.
    pub jumped: Option<bool>,
    /// In frames where no hurt counts.
    pub invulnerable: Option<bool>,
    /// On a ladder.
    pub climbing: Option<bool>,
    /// One of the body's `ext` words (0 to 3) is at least the first bound
    /// and below the second, each when given.
    pub word: Option<(u8, Option<i64>, Option<i64>)>,
    /// More word tests, each of which must hold too.
    pub words: Vec<(u8, Option<i64>, Option<i64>)>,
    /// Counters, by their index in the spec, each at least the first bound
    /// and below the second, each when given.
    pub counters: Vec<(usize, Option<i64>, Option<i64>)>,
    /// Counters by the body's slot: by slot number, the counter's index
    /// in the spec (`usize::MAX` for none), each in its range.
    pub slot_counters: Vec<(Vec<usize>, Option<i64>, Option<i64>)>,
    /// The body's x in whole pixels at least the first bound and below
    /// the second, each when given (core `clip.when_x`).
    pub x: (Option<i64>, Option<i64>),
}

/// The numbered slot body `i` is in, if any.
fn slot_of(w: &gp_sim::World, i: usize) -> Option<usize> {
    let id = u32::try_from(i).ok()?.checked_add(1)?;
    w.slots.iter().position(|s| s.body == id)
}

/// Whether `v` is in the range a test's bounds give.
fn in_range(v: i64, at_least: Option<i64>, below: Option<i64>) -> bool {
    at_least.is_none_or(|a| v >= a) && below.is_none_or(|l| v < l)
}

impl When {
    /// Whether the condition holds for body `b`, with the world's counters
    /// and the numbered slot `b` is in.
    pub fn holds(&self, b: &gp_sim::world::Body, counters: &[i64], slot: Option<usize>) -> bool {
        let speed = b.vx.raw().abs();
        let moving = speed != 0;
        let skidding = moving && (b.vx.raw() < 0) != b.facing_left;
        let rising = b.vy.raw() < 0;
        let jumped = !b.on_ground && b.air == u32::MAX;
        let invulnerable = b.invuln > 0;
        self.climbing.is_none_or(|c| c == b.climbing)
            && self.jumped.is_none_or(|j| j == jumped)
            && self.invulnerable.is_none_or(|v| v == invulnerable)
            && self.ground.is_none_or(|g| g == b.on_ground)
            && self.moving.is_none_or(|m| m == moving)
            && self.skidding.is_none_or(|s| s == skidding)
            && self.rising.is_none_or(|r| r == rising)
            && self.speed_at_least.is_none_or(|s| speed >= s)
            && self
                .word
                .iter()
                .chain(&self.words)
                .all(|&(i, at_least, below)| {
                    in_range(
                        b.ext.get(usize::from(i)).copied().unwrap_or(0),
                        at_least,
                        below,
                    )
                })
            && self.counters.iter().all(|&(k, at_least, below)| {
                in_range(counters.get(k).copied().unwrap_or(0), at_least, below)
            })
            && in_range(b.x.floor(), self.x.0, self.x.1)
            && self.slot_counters.iter().all(|(by_slot, at_least, below)| {
                slot.and_then(|s| by_slot.get(s))
                    .and_then(|&k| counters.get(k))
                    .is_some_and(|&v| in_range(v, *at_least, *below))
            })
    }
}

/// A run of frames shown while `when` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clip {
    pub when: When,
    pub frames: Vec<Frame>,
    /// Frames per picture by speed: the first step whose raw speed the
    /// body's reaches (None always does). One step without a speed is a
    /// fixed time. Empty: the clip doesn't step.
    pub timing: Vec<(Option<i64>, u32)>,
    /// Counted by the game's frames (shared by every body) rather than a
    /// timer per body.
    pub game_clock: bool,
    /// Counted by the body's one animation timer, which runs down whatever
    /// clip shows: on each frame the clip draws its picture, then steps to
    /// the next and sets the timer again if it had run out. It starts at
    /// the picture shown before when that is one of its own.
    pub body_clock: bool,
    pub while_frozen: bool,
    /// Never steps: keeps the picture shown when the clip started, if it
    /// is one of the clip's, else its first.
    pub hold: bool,
    /// `timing` reads the speed the body had on the frame before, as a
    /// game that picks its animation's time before it moves.
    pub speed_before: bool,
    /// Frames each picture shows for, by picture, in place of `timing`;
    /// empty when every picture takes the clip's time.
    pub lengths: Vec<u32>,
}

impl Clip {
    /// The time picture `k` shows for: its own length, else the clip's.
    fn every_at(&self, k: usize, b: &gp_sim::world::Body, before: i64) -> u32 {
        match self.lengths.get(k) {
            Some(&n) => n,
            None => self.every_with(b, before),
        }
    }

    /// The picture a clock of `clock` frames shows, for a clip on the
    /// game's clock: each picture its own length, or the clip's time.
    pub(crate) fn at_clock(&self, clock: u64) -> usize {
        let n = self.frames.len().max(1);
        if self.lengths.len() == self.frames.len() && !self.lengths.is_empty() {
            let total: u64 = self.lengths.iter().map(|&l| u64::from(l.max(1))).sum();
            let mut t = clock % total;
            for (k, &l) in self.lengths.iter().enumerate() {
                if t < u64::from(l.max(1)) {
                    return k;
                }
                t -= u64::from(l.max(1));
            }
            return 0;
        }
        let every = self.timing.first().map_or(1, |t| u64::from(t.1.max(1)));
        ((clock / every) % n as u64) as usize
    }

    /// The time for a body whose speed was `before` on the frame before.
    fn every_with(&self, b: &gp_sim::world::Body, before: i64) -> u32 {
        let speed = if self.speed_before {
            before
        } else {
            b.vx.raw().abs()
        };
        self.timing
            .iter()
            .find(|(at, _)| at.is_none_or(|a| speed >= a))
            .map_or(0, |&(_, e)| e)
    }

    fn every(&self, b: &gp_sim::world::Body) -> u32 {
        self.every_with(b, b.vx.raw().abs())
    }
}

/// How an entity type looks in one state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Look {
    pub base: Frame,
    pub facing_flip: bool,
    /// A second picture: how far below, where the heights wrap and the y
    /// above which neither picture shows. A body's top is its y.
    pub copy: Option<[i64; 3]>,
    /// Twice the x, from the body's position, about which a picture placed
    /// by its origin mirrors (the hitbox's middle).
    pub mirror: i64,
    pub clips: Vec<Clip>,
    /// Hidden on every other run of this many frames, by the game's
    /// frames, while the condition holds.
    pub blink: Option<(When, u32)>,
    /// Drawn behind the level's picture: its pixels show only where the
    /// level shows its backdrop (the NES's sprite priority bit).
    pub behind: bool,
    /// Drawn behind the level's picture on the frames this holds (core
    /// `sprite.behind_when`).
    pub behind_when: Option<When>,
    /// A part held where the picture is while the rest is drawn moved by
    /// counters (core `sprite.split`).
    pub split: Option<SplitLook>,
    /// Drawn as the picture the body it replaced (a defeat's successor's
    /// predecessor) showed last, flipped top to bottom when true. A body
    /// with none draws its own look.
    pub replaced: Option<bool>,
    /// The frame a body that replaces this one draws (with `replaced`),
    /// in place of this one's last picture.
    pub leaves: Option<u32>,
    /// Drawn over every body without it.
    pub front: bool,
    /// Drawn into the background's name table by the game, not as
    /// sprites: the screen measure counts its pixels as the background's,
    /// and the sprite limit doesn't count it.
    pub name_table: bool,
    /// A picture placed by its origin mirrors where it stands, its left
    /// edge kept at the origin, as a console flips a sprite, whatever its
    /// width (feature `sprite.flip_in_place`); without it, about `mirror`.
    pub in_place: bool,
}

/// A palette color stepping through a list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cycle {
    /// The level picture's pixels drawn in that color, as a white mask.
    pub mask: Option<u32>,
    /// The HUD layer's, likewise.
    pub hud_mask: Option<u32>,
    pub colors: Vec<[u8; 3]>,
    pub every: u32,
    /// A counter (by index into the spec's counters) whose value picks the
    /// color, in place of the clock, and what is added to it first.
    pub counter: Option<(usize, i32)>,
}

impl Cycle {
    /// The color this frame: from the counter's value when the cycle has
    /// one, otherwise from the clock, a color for each `every` frames.
    pub fn color(&self, clock: u64, counters: &[i64]) -> Option<[u8; 3]> {
        if self.colors.is_empty() {
            return None;
        }
        let n = self.colors.len() as i64;
        if let Some((k, shift)) = self.counter {
            let v = counters.get(k).copied().unwrap_or(0);
            return Some(self.colors[(v + i64::from(shift)).rem_euclid(n) as usize]);
        }
        if self.every == 0 {
            return None;
        }
        let i = (clock / u64::from(self.every)) % n as u64;
        Some(self.colors[i as usize])
    }
}

/// How a level looks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelLook {
    pub picture: Option<u32>,
    pub clear: [u8; 3],
    /// Which of each frame's pictures sprites use here: the level's
    /// palette set.
    pub set: u32,
    pub cycles: Vec<Cycle>,
    pub label: String,
    /// The HUD's fixed layer, in the level's palettes.
    pub hud_layer: Option<u32>,
    /// How the cells the game changes are drawn.
    pub cells: Option<CellLook>,
    /// By the spec's kinds: each kind's place in the console's sprite
    /// table here (core `present.sprite_order`), for the sprite limit.
    pub order: Vec<Option<SpriteOrder>>,
    /// By the present's effects: each effect's place in the table here
    /// (core `present.effect_order`), `at` and `step`: it takes `at +
    /// step * k`, k the place among its phase's `until_counters` of the
    /// counter the phase took.
    pub effect_order: Vec<Option<(u32, u32)>>,
}

/// A kind's place in the console's sprite table in one level: body k of
/// the kind takes sprites from `at + step * k`, k being its slot (the
/// body's word `word` less one, when the kind keeps its slot there and it
/// is set; else the world's numbered slot it is in) or else its count
/// among the kind's drawn bodies before it.
/// With `turn` (a counter by the spec's index, and the slots' count) the
/// slots are copied into the table from the counter's: k is the slot less
/// the counter, around the count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpriteOrder {
    pub at: u32,
    pub step: u32,
    pub word: Option<u32>,
    pub turn: Option<(usize, u32)>,
}

/// How a level draws the cells the game changes as it runs (a coin taken,
/// a block used, a brick broken), over its picture, which shows the cells
/// as they were built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellLook {
    /// By tile kind (`gp_sim::spec::Tile` as a number): the picture of a
    /// cell changed to that kind, and the same picture with its backdrop
    /// clear, for a bump.
    pub kinds: Vec<(u8, u32, u32)>,
    /// The picture a cell shows while a head hit holds it.
    pub held: Option<u32>,
    /// Cells a head hit can hold, by cell index in order: the picture of
    /// the cell as built, its backdrop clear, for a bump that leaves the
    /// cell as it was.
    pub own: Vec<(u32, u32)>,
}

impl CellLook {
    fn kind(&self, t: gp_sim::spec::Tile) -> Option<(u32, u32)> {
        self.kinds
            .iter()
            .find(|k| k.0 == t as u8)
            .map(|&(_, bg, sp)| (bg, sp))
    }

    fn own(&self, cell: u32) -> Option<u32> {
        self.own
            .binary_search_by_key(&cell, |o| o.0)
            .ok()
            .map(|i| self.own[i].1)
    }
}

/// A move up and down by a speed and a pull, in raw fixed point
/// (`gp_sim::Fx`): each frame the place goes by the speed, then the speed
/// by the pull, at most `max_fall`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Motion {
    pub speed: i64,
    pub gravity: i64,
    pub max_fall: Option<i64>,
}

impl Motion {
    fn step(&self, y: &mut i64, vy: &mut i64) {
        *y += *vy;
        *vy += self.gravity;
        if let Some(m) = self.max_fall {
            *vy = (*vy).min(m);
        }
    }

    /// Whole pixels from the start after `n` frames.
    pub fn offset(&self, n: u32) -> i64 {
        let (mut y, mut vy) = (0, self.speed);
        for _ in 0..n {
            self.step(&mut y, &mut vy);
        }
        y >> gp_sim::fx::FRAC_BITS
    }
}

/// A short-lived picture an event starts: it shows its phases in turn,
/// then goes. Presentation only, as a coin popping out of a block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Effect {
    /// The event, by index into the spec's events.
    pub event: usize,
    /// Starts at the cell a head hit took this frame (true), or at the
    /// event's entity.
    pub at_hit: bool,
    /// The picture's top left from the cell's or the entity's.
    pub offset: [i32; 2],
    pub phases: Vec<Phase>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Phase {
    /// Sprite frames, by index into [`Present::frames`], shown in turn by
    /// the game's frames, `every` frames each.
    pub frames: Vec<Frame>,
    pub every: u32,
    pub motion: Option<Motion>,
    /// Rises a pixel on each frame whose count is a multiple of this.
    pub rise_every: u32,
    /// Moves across as the camera does, so it keeps its place on screen.
    pub with_camera: bool,
    /// Ends on the frame its speed's whole part reaches this (raw).
    pub until_speed: Option<i64>,
    /// Ends after this many frames.
    pub length: Option<u32>,
    /// Goes on while a state freezes the world, counted by the world's
    /// frames, as a game object the freeze doesn't stop.
    pub while_frozen: bool,
    /// Ends on the frame after one on which this counter (by index into
    /// the spec's counters) is 0 or less, as a game that hides a sprite by
    /// a timer it reads before the frame's timers step.
    pub until_counter: Option<usize>,
    /// As `until_counter`, with the one of these counters that is highest
    /// as the phase starts: the timer the event's picture took, for a game
    /// with several slots for it (Donkey Kong's two points sprites, $41
    /// and $42, of which $CFC6 takes the first free).
    pub until_counters: Vec<usize>,
    /// Ends on the frame its event's entity is gone (no longer alive), and
    /// the next phase starts on that frame.
    pub until_gone: bool,
}

/// How a held cell's picture moves: by `motion`, from the cell's top left
/// plus `offset`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bump {
    pub motion: Motion,
    pub offset: [i32; 2],
}

/// An effect showing: its place (raw fixed point), speed, phase and the
/// frames in it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Live {
    effect: usize,
    phase: usize,
    t: u32,
    x: i64,
    y: i64,
    vy: i64,
    /// The event's entity, by index into the world's bodies.
    entity: Option<usize>,
    /// The phase's `until_counter` read 0 on an earlier frame.
    out: bool,
    /// The counter of the phase's `until_counters` it took as it started.
    counter: Option<usize>,
}

/// Where a HUD field's text comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A counter, by index into the spec's counters.
    Counter(usize),
    /// The level's label.
    Level,
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Column and row in tiles.
    pub at: [u32; 2],
    pub source: Source,
    /// Digits for a counter; 0 for as many as it needs.
    pub digits: u32,
    /// A counter is shown divided by this (at least 1).
    pub divide: u32,
    pub pad: char,
    /// The palette: 0 to 3 a background palette, 4 to 7 a sprite palette
    /// (the field is drawn as sprites).
    pub palette: u32,
    /// Drawn only while each of these counter tests holds (core
    /// `present.hud.field_when`).
    pub when: Vec<gp_sim::spec::Guard>,
}

impl Field {
    /// Whether it is drawn in this world.
    pub fn shown(&self, w: &World) -> bool {
        self.when.iter().all(|g| {
            g.cmp
                .holds(w.counters.get(g.counter).copied().unwrap_or(0), g.value)
        })
    }

    /// The field's text in this world.
    pub fn text(&self, w: &World, level: &LevelLook) -> String {
        match &self.source {
            Source::Text(t) => t.clone(),
            Source::Level => level.label.clone(),
            Source::Counter(i) => {
                let v = w.counters.get(*i).copied().unwrap_or(0) / i64::from(self.divide.max(1));
                let s = v.max(0).to_string();
                let d = self.digits as usize;
                if d == 0 {
                    s
                } else if s.len() >= d {
                    s[s.len() - d..].to_string()
                } else {
                    let mut out: String = std::iter::repeat_n(self.pad, d - s.len()).collect();
                    out.push_str(&s);
                    out
                }
            }
        }
    }
}

/// A glyph's picture: in palette set `set`, background palette `palette`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    pub set: u32,
    pub palette: u32,
    pub ch: char,
    pub pic: u32,
}

/// A sprite fixed on the screen: its top left corner in screen pixels and
/// its picture in each level's palettes, by level (a level past the list
/// uses the first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudSprite {
    pub at: [i32; 2],
    pub pics: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hud {
    pub fields: Vec<Field>,
    pub glyphs: Vec<Glyph>,
    /// Sprites fixed on the screen, drawn over the fields.
    pub sprites: Vec<HudSprite>,
    /// The palette set each level's HUD glyphs are in, by level.
    pub sets: Vec<u32>,
}

/// A model's presentation, compiled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Present {
    pub screen: Screen,
    pub pictures: Vec<Picture>,
    /// Each sprite frame's picture in each sprite palette set.
    pub frames: Vec<Vec<u32>>,
    /// Each sprite frame's top left from the body's position, for a frame
    /// laid out in pixels from it (a sprite-set's); None stands the
    /// picture on the body's bottom edge. Shorter than `frames` means None
    /// for the rest.
    pub origins: Vec<Option<[i32; 2]>>,
    /// Each entity type's look in each state of its machine (one entry
    /// for a type without a machine), by the spec's kinds.
    pub kinds: Vec<Vec<Option<Look>>>,
    /// By the spec's levels.
    pub levels: Vec<LevelLook>,
    pub hud: Option<Hud>,
    /// A held cell's picture rises and falls while it is held.
    pub bump: Option<Bump>,
    pub effects: Vec<Effect>,
    /// Whole screens shown in place of the level, its bodies and the HUD
    /// while a rule state holds (core `present.screens`).
    pub screens: Vec<ScreenLook>,
    /// The rule states, each a machine and a state of it by the spec's
    /// index, in which no sprite shows (core `present.sprites_off`): no
    /// body but those drawn as name-table tiles, and no HUD sprite.
    pub sprites_off: Vec<(usize, usize)>,
    /// Bodies not drawn while a rule state and counters hold (core
    /// `present.hidden`).
    pub hidden: Vec<HiddenLook>,
}

/// Bodies not drawn while one of its rule states holds and its counters
/// are in their ranges (core `present.hidden`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenLook {
    /// The states, each a machine and a state of it, by the spec's index.
    pub states: Vec<(usize, usize)>,
    /// Counters, by the spec's index, each at least the first bound and
    /// below the second, each when given.
    pub counters: Vec<(usize, Option<i64>, Option<i64>)>,
    /// The entity types hidden, by the spec's index, in any layer; empty
    /// for the sprite layer (every body but the name table's, and the
    /// HUD's sprites) unless `hud` is set.
    pub kinds: Vec<usize>,
    /// The HUD not drawn either: its layer, fields and sprites. With no
    /// `kinds`, only the HUD.
    pub hud: bool,
}

impl HiddenLook {
    /// Whether it holds now: one of its states is its machine's current,
    /// and its counters are in their ranges.
    pub fn holds(&self, w: &World) -> bool {
        self.states
            .iter()
            .any(|&(m, s)| w.machines.get(m).is_some_and(|ms| ms.state == s))
            && self.counters.iter().all(|&(k, at_least, below)| {
                in_range(w.counters.get(k).copied().unwrap_or(0), at_least, below)
            })
    }
}

/// A screen shown while one of its rule states holds: its picture, its
/// backdrop and its sprites.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenLook {
    /// The states, each a machine and a state of it, by the spec's index.
    pub states: Vec<(usize, usize)>,
    /// Counters, by the spec's index, each at least the first bound and
    /// below the second, each when given; all must hold too.
    pub counters: Vec<(usize, Option<i64>, Option<i64>)>,
    pub picture: u32,
    pub clear: [u8; 3],
    pub sprites: Vec<ScreenSprite>,
    /// With a blank screen: the screen line its blank starts at (core
    /// `present.screens_from_line`), the frame drawn as without it above.
    pub from_line: Option<u32>,
}

/// A sprite on a screen: its picture and top left in screen pixels, moved
/// `step` pixels for each 1 of a counter, by the spec's index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenSprite {
    pub pic: u32,
    pub at: [i32; 2],
    pub counter: Option<usize>,
    pub step: [i32; 2],
}

impl ScreenLook {
    /// Whether it shows now: one of its states is its machine's current,
    /// and its counters are in their ranges.
    pub fn shows(&self, w: &World) -> bool {
        self.states
            .iter()
            .any(|&(m, s)| w.machines.get(m).is_some_and(|ms| ms.state == s))
            && self.counters.iter().all(|&(k, at_least, below)| {
                in_range(w.counters.get(k).copied().unwrap_or(0), at_least, below)
            })
    }
}

/// A look's `split` (core `sprite.split`): the part `keep` (left, top,
/// width, height from the picture's top left as drawn) stays, and the
/// rest moves by the values of the counters `x` and `y` (by the spec's
/// index; 0 without one).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SplitLook {
    pub keep: [i32; 4],
    pub x: Option<usize>,
    pub y: Option<usize>,
}

/// One picture placed: its top left corner in world pixels (or, for the
/// HUD, screen pixels), with flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    pub pic: u32,
    pub x: i64,
    pub y: i64,
    pub flip_x: bool,
    pub flip_y: bool,
}

/// What to draw this frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scene {
    pub clear: [u8; 3],
    /// The camera's top left corner in world pixels.
    pub camera: [i64; 2],
    /// The level picture, at the world's origin.
    pub level: Option<u32>,
    /// Masks over the level picture and their color this frame.
    pub tints: Vec<(u32, [u8; 3])>,
    /// The cells the game changed, in world pixels: drawn over the level
    /// picture and its tints, before any body.
    pub cells: Vec<Draw>,
    /// Each body's picture, by body; None for a body not drawn (dead,
    /// waiting, or with no sprite). The second pictures of bodies with a
    /// `copy` come after them, then each bumped cell's picture and each
    /// effect's.
    pub bodies: Vec<Option<Draw>>,
    /// By body: drawn behind the level's picture, so it shows only where
    /// the level shows the clear color. A runtime draws these bodies
    /// before the level and the rest after it.
    pub behind: Vec<bool>,
    /// By body: drawn over every body without it (and over the level). A
    /// runtime draws these bodies last.
    pub front: Vec<bool>,
    /// By body: tiles the game writes into the background's name table
    /// (Donkey Kong), drawn as any body but counted as the background's.
    pub name_table: Vec<bool>,
    /// The HUD's layer and tiles, in screen pixels, then its sprites.
    pub hud: Vec<Draw>,
    /// How many of the last entries of `hud` are sprites (the screen
    /// measure counts their pixels as the sprite layer's).
    pub hud_sprites: usize,
    /// Masks over the HUD layer and their color.
    pub hud_tints: Vec<(u32, [u8; 3])>,
    /// By body: its place in the console's sprite table (core
    /// `present.sprite_order`), lower first; a body past the end has none.
    /// The sprite limit drops the sprites that come last on a line.
    pub rank: Vec<u32>,
    /// By body: the parts of its picture the console's sprite limit hides
    /// this frame (`view::sprite_cuts`): left, top, width and height from
    /// the picture's top left on the screen, before any flip. Empty when
    /// the limit is off.
    pub cut: Vec<Vec<[i64; 4]>>,
    /// By body: a split picture this frame (core `sprite.split`): the part
    /// `[left, top, width, height]` from the picture's top left as drawn
    /// stays at the draw's place, drawn over the rest, and the rest moves
    /// by the two numbers. None draws the picture whole.
    pub split: Vec<Option<([i64; 4], i64, i64)>>,
    /// The screen line the picture goes off at and the color it shows from
    /// there down (core `present.screens_from_line`): a console whose
    /// picture is turned off partway down the frame.
    pub off_from: Option<(i64, [u8; 3])>,
}

impl Scene {
    /// The order a frame paints its bodies in, the last on top: those
    /// behind the level, then those the game draws into the name table
    /// (background tiles, under every sprite), then the rest, then those in
    /// front of every body. Within each, by the console's sprite table
    /// (`rank`, core `present.sprite_order`): where two sprites overlap the
    /// NES draws the one earlier in the table, so a higher place paints
    /// first, and a body with none (past the table's end) before them. Ties
    /// and a level without an order keep body order.
    #[must_use]
    pub fn paint_order(&self) -> Vec<usize> {
        let at = |v: &[bool], b: usize| v.get(b).copied().unwrap_or(false);
        let group = |b: usize| {
            if at(&self.behind, b) {
                0
            } else if at(&self.name_table, b) {
                1
            } else if at(&self.front, b) {
                3
            } else {
                2
            }
        };
        let mut order: Vec<usize> = (0..self.bodies.len()).collect();
        order.sort_by_key(|&b| {
            let rank = self.rank.get(b).copied().unwrap_or(u32::MAX);
            (group(b), std::cmp::Reverse(rank))
        });
        order
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BodyAnim {
    kind: usize,
    state: usize,
    clip: Option<usize>,
    frame: usize,
    timer: u32,
    /// The frame id shown after the last step, for a held clip. For a
    /// body-clock clip it is the picture it steps to, which a held clip
    /// after it keeps.
    shown: Option<u32>,
    /// The body's own animation timer, for body-clock clips: frames until
    /// it runs out, counted down every frame whatever clip shows.
    body_timer: u32,
    /// The picture a body-clock clip drew this frame, before it stepped.
    drawn: usize,
    /// The body's speed across (raw, either way) when last stepped: the
    /// frame before's for a clip timed by it.
    speed: Option<i64>,
    /// The picture this body showed after the last step, placed: kept so
    /// a body that replaces it can draw it.
    last: Option<Shown>,
    /// For a look with `replaced`: the picture of the body it replaced.
    like: Option<Shown>,
}

/// A picture as a body showed it: the frame with its flips, the axis a
/// laid-out frame mirrors about, and the body's height, which a picture
/// without an origin stands on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Shown {
    frame: Frame,
    mirror: i64,
    in_place: bool,
    height: i64,
}

/// The animation state a runtime keeps beside the world: which clip each
/// body shows, its picture and its timer, and a clock that counts the
/// frames the world wasn't frozen. The clock starts at the world's frame
/// count, so a world loaded partway through a game (a start whose frame
/// count a mapping carries) shows its game-clock clips in the game's
/// phase. Step it once after each simulation frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Animator {
    level: Option<usize>,
    bodies: Vec<BodyAnim>,
    clock: u64,
    started: bool,
    /// Each held cell and the frames it has been held, counted as the
    /// world counts the hold (not while frozen).
    bumps: Vec<(u32, u32)>,
    /// Cells a head hit took on the last step, for an effect at a hit.
    hits: Vec<u32>,
    live: Vec<Live>,
    /// The first camera's x in whole pixels on the last step.
    cam: Option<i64>,
}

/// Whether a state that freezes the world holds it, as `gp_sim` decides
/// at the start of a frame.
pub fn frozen(spec: &Spec, w: &World) -> bool {
    w.machines
        .iter()
        .zip(&spec.machines)
        .any(|(m, s)| s.states.get(m.state).is_some_and(|st| st.freeze))
        || w.bodies.iter().any(|b| {
            b.active()
                && spec.kinds.get(b.kind).is_some_and(|k| {
                    k.machine
                        .as_ref()
                        .is_some_and(|m| m.states.get(b.state).is_some_and(|s| s.freeze))
                })
        })
}

impl Present {
    fn look(&self, b: &gp_sim::world::Body) -> Option<&Look> {
        let states = self.kinds.get(b.kind)?;
        states
            .get(b.state)
            .or_else(|| states.first())
            .and_then(Option::as_ref)
    }
}

impl Animator {
    pub fn new() -> Self {
        Self::default()
    }

    /// The frames counted while the world wasn't frozen.
    pub fn clock(&self) -> u64 {
        self.clock
    }

    /// Advances one simulation frame.
    pub fn step(&mut self, p: &Present, spec: &Spec, w: &World) {
        let frozen = frozen(spec, w);
        // Bodies are only ever added within a level, so a longer list
        // keeps each body's animation and starts the new ones.
        let had = self.bodies.len();
        if self.level != Some(w.level) {
            self.bumps.clear();
            self.live.clear();
            self.cam = None;
        }
        if self.level != Some(w.level) || had > w.bodies.len() {
            self.level = Some(w.level);
            self.bodies.clear();
        }
        let kept = self.bodies.len();
        for b in &w.bodies[kept..] {
            self.bodies.push(BodyAnim {
                kind: usize::MAX,
                state: b.state,
                ..BodyAnim::default()
            });
        }
        self.find_replaced(p, spec, w, kept);
        if !self.started {
            self.started = true;
            self.clock = w.frame;
        } else if !frozen {
            self.clock += 1;
        }
        self.step_cells(p, w, frozen);
        for (i, (a, b)) in self.bodies.iter_mut().zip(&w.bodies).enumerate() {
            let slot = slot_of(w, i);
            // The speed the frame before, for a clip timed by it.
            let now = b.vx.raw().abs();
            let before = a.speed.replace(now).unwrap_or(now);
            let Some(look) = p.look(b) else {
                *a = BodyAnim {
                    kind: b.kind,
                    state: b.state,
                    speed: Some(now),
                    like: a.like,
                    ..BodyAnim::default()
                };
                continue;
            };
            if !frozen {
                a.body_timer = a.body_timer.saturating_sub(1);
            }
            let clip = look
                .clips
                .iter()
                .position(|c| c.when.holds(b, &w.counters, slot));
            if (a.kind, a.state, a.clip) != (b.kind, b.state, clip) {
                let shown = a.shown;
                // A held clip, and a body-clock clip (whose place is the
                // body's), go on from the picture shown when it is theirs.
                let frame = clip
                    .map(|c| &look.clips[c])
                    .filter(|c| c.hold || c.body_clock)
                    .and_then(|c| c.frames.iter().position(|f| Some(f.id) == shown))
                    .unwrap_or(0);
                // The body's timer carries over a change of state too: it
                // is the body's, not the clip's.
                let body_timer = if a.kind == b.kind { a.body_timer } else { 0 };
                *a = BodyAnim {
                    kind: b.kind,
                    state: b.state,
                    clip,
                    frame,
                    timer: clip.map_or(0, |c| look.clips[c].every_at(frame, b, before)),
                    shown,
                    body_timer,
                    drawn: frame,
                    speed: Some(now),
                    last: None,
                    like: a.like,
                };
                Self::body_step(look, a, b, frozen, before);
                a.shown = Self::shown(look, a);
                continue;
            }
            if clip.is_some_and(|c| look.clips[c].body_clock) {
                Self::body_step(look, a, b, frozen, before);
                a.shown = Self::shown(look, a);
                continue;
            }
            if let Some(c) = clip.map(|c| &look.clips[c])
                && !(c.hold || c.game_clock || (frozen && !c.while_frozen) || c.frames.len() < 2)
            {
                if a.timer <= 1 {
                    a.frame = (a.frame + 1) % c.frames.len();
                    a.timer = c.every_at(a.frame, b, before).max(1);
                } else {
                    a.timer -= 1;
                }
            }
            a.drawn = a.frame;
            a.shown = Self::shown(look, a);
        }
        for i in 0..self.bodies.len() {
            let last = self.placed(p, spec, w, i);
            self.bodies[i].last = last;
        }
    }

    /// Follows the held cells (a bump's time counts as the world counts
    /// the hold, not while frozen; a cell held this frame is a hit) and
    /// steps the effects showing.
    fn step_cells(&mut self, p: &Present, w: &World, frozen: bool) {
        self.hits.clear();
        self.bumps
            .retain(|(c, _)| w.held_cells.iter().any(|h| h.0 == *c));
        if !frozen {
            for b in &mut self.bumps {
                b.1 += 1;
            }
        }
        for &(c, _, _) in &w.held_cells {
            if !self.bumps.iter().any(|b| b.0 == c) {
                self.bumps.push((c, 0));
                self.hits.push(c);
            }
        }
        let cam = w.cameras.first().map(|c| c[0].floor());
        let dx = match (self.cam, cam) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        };
        self.cam = cam;
        let (clock, frame) = (self.clock, w.frame);
        self.live.retain_mut(|l| {
            let ph = p.effects.get(l.effect).and_then(|e| e.phases.get(l.phase));
            match ph {
                Some(ph) if ph.while_frozen => Self::step_live(p, w, l, dx, frame),
                Some(_) if frozen => true,
                _ => Self::step_live(p, w, l, dx, clock),
            }
        });
    }

    /// The counter of a phase's `until_counters` highest now (the first
    /// of equal ones), if it has any.
    fn taken_counter(ph: &Phase, w: &World) -> Option<usize> {
        ph.until_counters
            .iter()
            .copied()
            .rev()
            .max_by_key(|&c| w.counters.get(c).copied().unwrap_or(i64::MIN))
    }

    /// One frame of a showing effect: it ends its phase when the phase's
    /// frames are done, its counter read 0 on an earlier frame or its
    /// entity has gone, moves by its motion and with the camera, starts
    /// the next phase when its speed reaches the phase's end, and rises.
    /// False when it has gone.
    fn step_live(p: &Present, w: &World, l: &mut Live, dx: i64, clock: u64) -> bool {
        let Some(e) = p.effects.get(l.effect) else {
            return false;
        };
        let next = |l: &mut Live| {
            l.phase += 1;
            l.t = 0;
            l.out = false;
            l.counter = e
                .phases
                .get(l.phase)
                .and_then(|ph| Self::taken_counter(ph, w));
            if let Some(m) = e.phases.get(l.phase).and_then(|ph| ph.motion) {
                l.vy = m.speed;
            }
            l.phase < e.phases.len()
        };
        let Some(ph) = e.phases.get(l.phase) else {
            return false;
        };
        if ph.length.is_some_and(|n| l.t >= n) && !next(l) {
            return false;
        }
        let gone = |l: &Live| {
            l.entity
                .and_then(|i| w.bodies.get(i))
                .is_none_or(|b| !b.alive)
        };
        if e.phases[l.phase].until_gone && gone(l) && !next(l) {
            return false;
        }
        let counted = |l: &Live| e.phases[l.phase].until_counter.or(l.counter);
        if counted(l).is_some() && l.out && !next(l) {
            return false;
        }
        let ph = &e.phases[l.phase];
        if let Some(c) = counted(l)
            && w.counters.get(c).is_none_or(|v| *v <= 0)
        {
            l.out = true;
        }
        if let Some(m) = ph.motion {
            m.step(&mut l.y, &mut l.vy);
        }
        if ph.with_camera {
            l.x += dx << gp_sim::fx::FRAC_BITS;
        }
        if ph
            .until_speed
            .is_some_and(|u| l.vy >> gp_sim::fx::FRAC_BITS == u >> gp_sim::fx::FRAC_BITS)
            && !next(l)
        {
            return false;
        }
        let ph = &e.phases[l.phase];
        if ph.rise_every > 0 && clock.is_multiple_of(u64::from(ph.rise_every)) {
            l.y -= 1 << gp_sim::fx::FRAC_BITS;
        }
        l.t += 1;
        true
    }

    /// Starts the effects the frame's events start, after [`step`] on the
    /// same frame. An effect at a hit starts at the cell a head hit took
    /// this frame (the last one, when there are more); one at its entity,
    /// at the entity's position. It takes its first frame now: a game that
    /// makes an object in its frame moves and draws it in the same frame.
    ///
    /// [`step`]: Animator::step
    pub fn fired(&mut self, p: &Present, spec: &Spec, w: &World, events: &[gp_sim::Event]) {
        let list: Vec<(usize, Option<usize>)> =
            events.iter().map(|e| (e.kind.0, e.entity)).collect();
        self.fired_kinds(p, spec, w, &list);
    }

    /// [`Animator::fired`] with each event as its index into the spec's
    /// events and its entity's into the world's bodies.
    pub fn fired_kinds(
        &mut self,
        p: &Present,
        spec: &Spec,
        w: &World,
        events: &[(usize, Option<usize>)],
    ) {
        if p.effects.is_empty() {
            return;
        }
        let level = &spec.levels[w.level.min(spec.levels.len().saturating_sub(1))];
        for &(ev, entity) in events {
            for (k, e) in p.effects.iter().enumerate() {
                if e.event != ev {
                    continue;
                }
                let at = if e.at_hit {
                    self.hits.last().map(|&c| level.cell_pixel(c as usize))
                } else {
                    entity
                        .and_then(|i| w.bodies.get(i))
                        .map(|b| (b.x.floor(), b.y.floor()))
                };
                let Some((x, y)) = at else { continue };
                let mut l = Live {
                    effect: k,
                    x: (x + i64::from(e.offset[0])) << gp_sim::fx::FRAC_BITS,
                    y: (y + i64::from(e.offset[1])) << gp_sim::fx::FRAC_BITS,
                    vy: e
                        .phases
                        .first()
                        .and_then(|ph| ph.motion)
                        .map_or(0, |m| m.speed),
                    entity,
                    counter: e.phases.first().and_then(|ph| Self::taken_counter(ph, w)),
                    ..Live::default()
                };
                if Self::step_live(p, w, &mut l, 0, self.clock) {
                    self.live.push(l);
                }
            }
        }
    }

    /// The cells the game changed, drawn over the level, and each held
    /// cell's bump and each effect's picture, drawn as sprites.
    fn cell_draws(
        &self,
        p: &Present,
        spec: &Spec,
        w: &World,
        set: usize,
    ) -> (Vec<Draw>, Vec<Draw>, Vec<u32>) {
        let (mut cells, mut sprites) = (Vec::new(), Vec::new());
        let still = |pic: u32, x: i64, y: i64| Draw {
            pic,
            x,
            y,
            flip_x: false,
            flip_y: false,
        };
        if let (Some(look), Some(level)) = (
            p.levels.get(w.level).and_then(|l| l.cells.as_ref()),
            spec.levels.get(w.level),
        ) {
            // Platformer `level.pickup_cells`: a taken place's cells show
            // an empty cell, under any cell the game changed since.
            if let Some((empty, _)) = look.kind(gp_sim::spec::Tile::Empty) {
                for g in &level.pickups {
                    for (own, &c) in g.cells.iter().zip(&g.counters) {
                        if w.counters.get(c).is_some_and(|v| *v != 0) {
                            for &cell in own {
                                let (x, y) = level.cell_pixel(cell as usize);
                                cells.push(still(empty, x, y));
                            }
                        }
                    }
                }
            }
            for &(c, t) in &w.tiles {
                let (x, y) = level.cell_pixel(c as usize);
                let held = w.held_cells.iter().find(|h| h.0 == c);
                let pic = match held {
                    Some(_) => look.held,
                    None => look.kind(t).map(|k| k.0),
                };
                if let Some(pic) = pic {
                    cells.push(still(pic, x, y));
                }
            }
            for &(c, t) in &self.bumps {
                let Some(&(_, next, _)) = w.held_cells.iter().find(|h| h.0 == c) else {
                    continue;
                };
                let built = level.tiles.get(c as usize).copied();
                let pic = if built == Some(next) {
                    look.own(c)
                } else {
                    look.kind(next).map(|k| k.1)
                };
                let (Some(pic), Some(b)) = (pic, p.bump) else {
                    continue;
                };
                let (x, y) = level.cell_pixel(c as usize);
                sprites.push(still(
                    pic,
                    x + i64::from(b.offset[0]),
                    y + i64::from(b.offset[1]) + b.motion.offset(t + 1),
                ));
            }
        }
        // The bumps have no place in the sprite table; the effects may
        // (core `present.effect_order`).
        let mut ranks = vec![u32::MAX; sprites.len()];
        let level = p.levels.get(w.level);
        for l in &self.live {
            let Some(ph) = p.effects.get(l.effect).and_then(|e| e.phases.get(l.phase)) else {
                continue;
            };
            if ph.frames.is_empty() {
                continue;
            }
            let n = ph.frames.len() as u64;
            let clock = if ph.while_frozen { w.frame } else { self.clock };
            let f = ph.frames[((clock / u64::from(ph.every.max(1))) % n) as usize];
            let Some(pics) = p.frames.get(f.id as usize) else {
                continue;
            };
            let Some(&pic) = pics.get(set).or_else(|| pics.first()) else {
                continue;
            };
            sprites.push(Draw {
                pic,
                x: l.x >> gp_sim::fx::FRAC_BITS,
                y: l.y >> gp_sim::fx::FRAC_BITS,
                flip_x: f.flip_x,
                flip_y: f.flip_y,
            });
            let order = level.and_then(|v| v.effect_order.get(l.effect).copied().flatten());
            ranks.push(order.map_or(u32::MAX, |(at, step)| {
                let k = l
                    .counter
                    .and_then(|c| ph.until_counters.iter().position(|&x| x == c))
                    .unwrap_or(0) as u32;
                at + step * k
            }));
        }
        (cells, sprites, ranks)
    }

    /// Gives each body from `from` on whose look draws the body it
    /// replaced that body's last picture: a body that went this frame, of
    /// a kind whose defeat becomes the new body's kind, at the same place
    /// across. Each one is copied once.
    fn find_replaced(&mut self, p: &Present, spec: &Spec, w: &World, from: usize) {
        let mut used = Vec::new();
        for n in from..w.bodies.len() {
            let b = &w.bodies[n];
            if p.look(b).and_then(|l| l.replaced).is_none() {
                continue;
            }
            let old = (0..from).find(|&j| {
                let o = &w.bodies[j];
                !used.contains(&j)
                    && !o.active()
                    && self.bodies[j].last.is_some()
                    && o.x.floor() == b.x.floor()
                    && spec
                        .kinds
                        .get(o.kind)
                        .and_then(|k| k.health.as_ref())
                        .and_then(|h| h.defeat.as_ref())
                        .is_some_and(|d| d.becomes == b.kind)
            });
            if let Some(j) = old {
                used.push(j);
                let last = self.bodies[j].last;
                // The old body's own picture for its successor, unturned:
                // the flip `replaced` adds is undone here.
                let flip = p.look(b).and_then(|l| l.replaced).unwrap_or(false);
                let leaves = p.look(&w.bodies[j]).and_then(|l| l.leaves);
                self.bodies[n].like = match (leaves, last) {
                    (Some(id), Some(l)) => Some(Shown {
                        frame: Frame {
                            id,
                            // Turned the way the old body faced.
                            flip_x: p.look(&w.bodies[j]).is_some_and(|o| o.facing_flip)
                                && (w.bodies[j].facing_left || w.bodies[j].faced_both),
                            flip_y: flip,
                        },
                        ..l
                    }),
                    _ => last,
                };
            }
        }
    }

    /// A body-clock clip's frame: it draws its picture, then steps to the
    /// next one and sets the body's timer again if the timer had run out.
    fn body_step(
        look: &Look,
        a: &mut BodyAnim,
        b: &gp_sim::world::Body,
        frozen: bool,
        before: i64,
    ) {
        a.drawn = a.frame;
        let Some(c) = a.clip.map(|c| &look.clips[c]) else {
            return;
        };
        if !c.body_clock || c.hold || c.frames.len() < 2 || (frozen && !c.while_frozen) {
            return;
        }
        if a.body_timer == 0 {
            a.frame = (a.frame + 1) % c.frames.len();
            a.body_timer = c.every_at(a.frame, b, before).max(1);
        }
    }

    /// Puts body `i`'s animation where a game holds it, after [`step`] on
    /// the frame it was loaded at: the picture its clip is at (`frame`,
    /// an index into the clip's frames) and the frames left on the body's
    /// timer (`timer`), as the game keeps them after its frame. A game
    /// that draws before it steps has drawn the picture before `frame`
    /// when the timer was just set again (it equals the clip's time), so
    /// that is the one shown. A body without a look or clip is left as it
    /// is.
    ///
    /// [`step`]: Animator::step
    pub fn seed(
        &mut self,
        p: &Present,
        w: &World,
        i: usize,
        frame: Option<u32>,
        timer: Option<u32>,
    ) {
        let (Some(a), Some(b)) = (self.bodies.get_mut(i), w.bodies.get(i)) else {
            return;
        };
        let Some(look) = p.look(b) else {
            return;
        };
        if let Some(t) = timer {
            a.body_timer = t;
        }
        let Some(c) = a.clip.map(|c| &look.clips[c]) else {
            return;
        };
        let n = c.frames.len();
        if let Some(f) = frame
            && n > 0
            && !c.game_clock
        {
            a.frame = f as usize % n;
            a.drawn = a.frame;
            if c.body_clock
                && !c.hold
                && n > 1
                && timer.is_some_and(|t| t > 0 && t == c.every(b).max(1))
            {
                a.drawn = (a.frame + n - 1) % n;
            }
        }
        a.shown = Self::shown(look, a);
    }

    /// The frame id a body's own animation state shows (a game-clock clip
    /// counts as its first frame here).
    fn shown(look: &Look, a: &BodyAnim) -> Option<u32> {
        match a.clip.map(|c| &look.clips[c]) {
            Some(c) if !c.frames.is_empty() => Some(c.frames[a.frame % c.frames.len()].id),
            _ => Some(look.base.id),
        }
    }

    /// The frame body `i` shows, if it's drawn.
    pub fn frame(&self, p: &Present, w: &World, i: usize) -> Option<Frame> {
        let b = w.bodies.get(i)?;
        if !b.active() {
            return None;
        }
        let look = p.look(b)?;
        let slot = slot_of(w, i);
        if let (Some(flip), Some(like)) = (look.replaced, self.bodies.get(i).and_then(|a| a.like)) {
            let mut f = like.frame;
            f.flip_y ^= flip;
            return Some(f);
        }
        let a = self
            .bodies
            .get(i)
            .filter(|a| a.kind == b.kind && a.state == b.state);
        let clip = match a {
            Some(a) => a.clip,
            None => look
                .clips
                .iter()
                .position(|c| c.when.holds(b, &w.counters, slot)),
        };
        let mut f = match clip.map(|c| &look.clips[c]) {
            Some(c) if !c.frames.is_empty() => {
                let n = c.frames.len();
                let i = if c.game_clock {
                    let clock = if c.while_frozen { w.frame } else { self.clock };
                    c.at_clock(clock)
                } else if c.body_clock {
                    a.map_or(0, |a| a.drawn % n)
                } else {
                    a.map_or(0, |a| a.frame % n)
                };
                c.frames[i]
            }
            _ => look.base,
        };
        if let Some((when, every)) = &look.blink
            && when.holds(b, &w.counters, slot)
            && (w.frame / u64::from((*every).max(1))) % 2 == 1
        {
            return None;
        }
        // A body facing both ways (`run.face_both`) draws as facing left:
        // Super Mario Bros.' DrawSpriteObject reads PlayerFacingDir's bit 1.
        if look.facing_flip && (b.facing_left || b.faced_both) {
            f.flip_x = !f.flip_x;
        }
        Some(f)
    }

    /// The frame body `i` shows, with the axis it mirrors about and the
    /// height it stands on: its own, or for a body drawing the one it
    /// replaced, that body's.
    fn placed(&self, p: &Present, spec: &Spec, w: &World, i: usize) -> Option<Shown> {
        let b = w.bodies.get(i)?;
        let frame = self.frame(p, w, i)?;
        let like = p
            .look(b)
            .and_then(|l| l.replaced)
            .and(self.bodies.get(i).and_then(|a| a.like));
        Some(match like {
            Some(l) => Shown { frame, ..l },
            None => Shown {
                frame,
                mirror: p.look(b).map_or(0, |l| l.mirror),
                in_place: p.look(b).is_some_and(|l| l.in_place),
                height: spec.kind(b).size[1].floor(),
            },
        })
    }

    /// What to draw this frame.
    pub fn scene(&self, p: &Present, spec: &Spec, w: &World) -> Scene {
        // A blank from a line down (core `present.screens_from_line`): the
        // frame as it would be without it, then the blank below the line.
        if let Some(sc) = p.screens.iter().find(|sc| sc.shows(w))
            && let Some(line) = sc.from_line
        {
            let mut s = self.scene_under(p, spec, w, true);
            s.off_from = Some((i64::from(line), sc.clear));
            return s;
        }
        self.scene_under(p, spec, w, false)
    }

    /// [`Self::scene`], leaving out the screens with a `from_line` when
    /// `under`.
    fn scene_under(&self, p: &Present, spec: &Spec, w: &World, under: bool) -> Scene {
        // A screen in place of the level (core `present.screens`): its
        // picture, no body, and its sprites as the HUD's.
        let shown = p
            .screens
            .iter()
            .filter(|sc| !under || sc.from_line.is_none())
            .find(|sc| sc.shows(w));
        if let Some(sc) = shown {
            let n = w.bodies.len();
            let mut s = Scene {
                clear: sc.clear,
                level: Some(sc.picture),
                bodies: vec![None; n],
                behind: vec![false; n],
                front: vec![false; n],
                name_table: vec![false; n],
                ..Scene::default()
            };
            for sp in &sc.sprites {
                let k = sp
                    .counter
                    .and_then(|c| w.counters.get(c))
                    .copied()
                    .unwrap_or(0);
                s.hud.push(Draw {
                    pic: sp.pic,
                    x: i64::from(sp.at[0]) + k * i64::from(sp.step[0]),
                    y: i64::from(sp.at[1]) + k * i64::from(sp.step[1]),
                    flip_x: false,
                    flip_y: false,
                });
                s.hud_sprites += 1;
            }
            return s;
        }
        let level = p.levels.get(w.level);
        let [cx, cy] = w.cameras.first().copied().unwrap_or_default();
        let set = level.map_or(0, |l| l.set) as usize;
        let mut s = Scene {
            clear: level.map_or([0, 0, 0], |l| l.clear),
            camera: [cx.floor(), cy.floor()],
            level: level.and_then(|l| l.picture),
            ..Scene::default()
        };
        if let Some(l) = level {
            for c in &l.cycles {
                if let Some(color) = c.color(self.clock, &w.counters) {
                    if let Some(m) = c.mask {
                        s.tints.push((m, color));
                    }
                    if let Some(m) = c.hud_mask {
                        s.hud_tints.push((m, color));
                    }
                }
            }
        }
        let mut copies = Vec::new();
        // Drawn bodies of each kind so far, for the sprite table's order.
        let mut of_kind = vec![0u32; spec.kinds.len()];
        for (i, b) in w.bodies.iter().enumerate() {
            let draw = self.placed(p, spec, w, i).and_then(|sh| {
                let f = sh.frame;
                let pics = p.frames.get(f.id as usize)?;
                let pic = *pics.get(set).or_else(|| pics.first())?;
                let size = p.pictures.get(pic as usize)?.size;
                if let Some(Some([ox, oy])) = p.origins.get(f.id as usize) {
                    // Laid out from the position: a mirrored picture turns
                    // about the look's axis, or where it stands.
                    let mirror = sh.mirror;
                    let ox = i64::from(*ox);
                    let x = if f.flip_x && !sh.in_place {
                        mirror - ox - i64::from(size[0])
                    } else {
                        ox
                    };
                    return Some(Draw {
                        pic,
                        x: b.x.floor() + x,
                        y: b.y.floor() + i64::from(*oy),
                        flip_x: f.flip_x,
                        flip_y: f.flip_y,
                    });
                }
                let h = sh.height;
                Some(Draw {
                    pic,
                    x: b.x.floor(),
                    // A sprite stands on the body's bottom edge.
                    y: b.y.floor() + h - i64::from(size[1]),
                    flip_x: f.flip_x,
                    flip_y: f.flip_y,
                })
            });
            s.behind.push(
                draw.is_some()
                    && p.look(b).is_some_and(|l| {
                        l.behind
                            || l.behind_when
                                .as_ref()
                                .is_some_and(|c| c.holds(b, &w.counters, slot_of(w, i)))
                    }),
            );
            s.front
                .push(draw.is_some() && p.look(b).is_some_and(|l| l.front));
            // Core `sprite.split`: only while a counter moves the rest.
            s.split.push(draw.and_then(|_| {
                let sp = p.look(b)?.split?;
                let at = |c: Option<usize>| c.and_then(|k| w.counters.get(k).copied()).unwrap_or(0);
                let (dx, dy) = (at(sp.x), at(sp.y));
                ((dx, dy) != (0, 0)).then(|| (sp.keep.map(i64::from), dx, dy))
            }));
            s.name_table
                .push(draw.is_some() && p.look(b).is_some_and(|l| l.name_table));
            // Core `present.sprite_order`: the body's place in the table.
            let order = level.and_then(|l| l.order.get(b.kind).copied().flatten());
            let seen = of_kind.get(b.kind).copied().unwrap_or(0);
            s.rank.push(order.map_or(u32::MAX, |o| {
                let slot = o
                    .word
                    .and_then(|k| b.ext.get(k as usize).copied())
                    .filter(|v| *v > 0)
                    .map(|v| (v - 1).clamp(0, 255) as u32)
                    .or_else(|| slot_of(w, i).map(|n| n as u32))
                    .unwrap_or(seen);
                let slot = match o.turn {
                    Some((c, n)) if n > 0 => {
                        let first = w.counters.get(c).copied().unwrap_or(0);
                        (i64::from(slot) - first).rem_euclid(i64::from(n)) as u32
                    }
                    _ => slot,
                };
                o.at + o.step * slot
            }));
            if draw.is_some()
                && let Some(n) = of_kind.get_mut(b.kind)
            {
                *n += 1;
            }
            let copy = draw.zip(p.look(b).and_then(|l| l.copy));
            let top = b.y.floor();
            s.bodies
                .push(draw.filter(|_| copy.is_none_or(|(_, [_, _, above])| top >= above)));
            if let Some((d, [below, wrap, above])) = copy {
                let y = (top + below).rem_euclid(wrap);
                if y >= above {
                    copies.push(Draw {
                        y: d.y + y - top,
                        ..d
                    });
                }
            }
        }
        // After the bodies, whose draws are by body: the copies, then the
        // bumps and the effects.
        let (cells, sprites, ranks) = self.cell_draws(p, spec, w, set);
        s.cells = cells;
        // The copies have no place of their own in the table.
        s.rank.resize(s.bodies.len() + copies.len(), u32::MAX);
        s.rank.extend(ranks);
        copies.extend(sprites);
        s.behind.resize(s.bodies.len() + copies.len(), false);
        s.front.resize(s.bodies.len() + copies.len(), false);
        s.name_table.resize(s.bodies.len() + copies.len(), false);
        s.bodies.extend(copies.into_iter().map(Some));
        if let (Some(hud), Some(l)) = (&p.hud, level) {
            if let Some(layer) = l.hud_layer {
                s.hud.push(Draw {
                    pic: layer,
                    x: 0,
                    y: 0,
                    flip_x: false,
                    flip_y: false,
                });
            }
            let gset = hud.sets.get(w.level).copied().unwrap_or(0);
            // Fields in the background palettes, then those drawn as
            // sprites (palettes 4 to 7), which are the sprite layer's with
            // the HUD's sprites after them.
            let in_bg = hud.fields.iter().filter(|f| f.palette < 4 && f.shown(w));
            let as_sprites = hud.fields.iter().filter(|f| f.palette >= 4 && f.shown(w));
            for (sprite, f) in in_bg
                .map(|f| (false, f))
                .chain(as_sprites.map(|f| (true, f)))
            {
                for (k, ch) in f.text(w, l).chars().enumerate() {
                    let g = hud
                        .glyphs
                        .iter()
                        .find(|g| g.set == gset && g.palette == f.palette && g.ch == ch);
                    if let Some(g) = g {
                        s.hud.push(Draw {
                            pic: g.pic,
                            x: i64::from(f.at[0] + k as u32) * 8,
                            y: i64::from(f.at[1]) * 8,
                            flip_x: false,
                            flip_y: false,
                        });
                        if sprite {
                            s.hud_sprites += 1;
                        }
                    }
                }
            }
            for sp in &hud.sprites {
                let Some(&pic) = sp.pics.get(w.level).or_else(|| sp.pics.first()) else {
                    continue;
                };
                s.hud.push(Draw {
                    pic,
                    x: i64::from(sp.at[0]),
                    y: i64::from(sp.at[1]),
                    flip_x: false,
                    flip_y: false,
                });
                s.hud_sprites += 1;
            }
        }
        // The console's sprite layer off (core `present.sprites_off`).
        if p.sprites_off
            .iter()
            .any(|&(m, st)| w.machines.get(m).is_some_and(|ms| ms.state == st))
        {
            for (b, d) in s.bodies.iter_mut().enumerate() {
                if !s.name_table.get(b).copied().unwrap_or(false) {
                    *d = None;
                }
            }
            s.hud.truncate(s.hud.len() - s.hud_sprites);
            s.hud_sprites = 0;
        }
        // Bodies the game doesn't draw now (core `present.hidden`).
        for h in p.hidden.iter().filter(|h| h.holds(w)) {
            let layer = h.kinds.is_empty() && !h.hud;
            for (b, d) in s.bodies.iter_mut().enumerate() {
                let hide = if layer {
                    !s.name_table.get(b).copied().unwrap_or(false)
                } else {
                    w.bodies.get(b).is_some_and(|x| h.kinds.contains(&x.kind))
                };
                if hide {
                    *d = None;
                }
            }
            if h.hud {
                s.hud.clear();
                s.hud_tints.clear();
                s.hud_sprites = 0;
            } else if layer {
                s.hud.truncate(s.hud.len() - s.hud_sprites);
                s.hud_sprites = 0;
            }
        }
        s
    }
}

#[cfg(test)]
mod tests;
