//! The `ext` boundary: hand-written behavior for what a model's components
//! can't say, as Rust that implements [`Ext`]. The interpreter and a
//! generated build link the same hook crate, which depends on `gp-sim`
//! only, and find hooks by the name the model's `ext` gives, in an
//! [`Exts`] registry passed to [`crate::systems::step_with_ext`].
//!
//! A hook runs once a frame for each active instance of an entity that
//! names it, in that entity's system (`platformer.player` for a player,
//! `platformer.enemies` for the rest), after its components. It gets the
//! instance's body to change, the rest of the world to read, and the
//! world's random number generator; it asks for anything else (firing an
//! event, spawning or removing an entity) through requests, applied in
//! order when it returns. Hooks follow the simulation's rules: fixed-point
//! only, no clock, no state outside the world (a body's `ext` words hold a
//! hook's own counters), no iteration over a hash map.

use std::collections::BTreeMap;

use crate::fx::Fx;
use crate::rng::Rng;
use crate::spec::{EventKind, Spec, Tile};
use crate::world::{Body, World};

/// A hook. `step` must give the same result for the same inputs on every
/// runtime and platform.
pub trait Ext: Send + Sync {
    fn step(&self, cx: &mut ExtCx<'_>);

    /// Runs in place of `step` on the frame a level's `appear_still` holds
    /// the instance still, for a game that sets an object up on the frame
    /// it appears and moves it from the next. The default does nothing.
    fn appear(&self, _cx: &mut ExtCx<'_>) {}

    /// Runs at the start of every frame, before the first system, frozen
    /// or not, for an entity with core `entity.hook_frame_start`
    /// ([`crate::systems::begin_frame_ext`]): what a game's NMI runs before
    /// anything else, such as its sound driver taking the requests the
    /// last frame made. The default does nothing.
    fn frame_start(&self, _cx: &mut ExtCx<'_>) {}

    /// The mechanics this hook is, as records (crate::mechanic), so the
    /// hook isn't opaque to `gp model mechanics`, composition and the
    /// clean-room check. Usually one; a hook that is several mechanics
    /// chosen by a parameter (a lift's routines) may give more. The default
    /// is none: the hook is then reported as opaque.
    fn describe(&self) -> Vec<crate::mechanic::Record> {
        Vec::new()
    }

    /// The APU register writes, each as its offset from $4000 and the
    /// value in the order made, that this instance's hook makes on the
    /// frame after `world`. It's for presentation: a hook that keeps a
    /// game's sound driver as state (Donkey Kong's `dk_sound`) gives the
    /// writes its next step makes, worked out on a copy, so a runtime
    /// plays the game's own sound data through gp-sound's APU
    /// ([`hook_apu`]). It never changes the world. The default makes none.
    fn apu(&self, _spec: &Spec, _world: &World, _id: usize) -> Vec<(u8, u8)> {
        Vec::new()
    }

    /// [`Self::apu`], for a hook whose writes read a soft draw site
    /// ([`ExtCx::draw_soft`]): `peek` gives the byte the next frame's
    /// draw at a site will take, as [`crate::DrawScript::peek`] says, or
    /// `None`. The default ignores it.
    fn apu_peek(
        &self,
        spec: &Spec,
        world: &World,
        id: usize,
        _peek: &dyn Fn(&str) -> Option<u8>,
    ) -> Vec<(u8, u8)> {
        self.apu(spec, world, id)
    }
}

/// The APU writes every hooked instance makes on the frame after `world`
/// ([`Ext::apu`]), in body order: what a runtime plays for a model whose
/// sound is a game's driver kept as state.
pub fn hook_apu(spec: &Spec, world: &World, exts: &Exts) -> Vec<(u8, u8)> {
    hook_apu_peek(spec, world, exts, &|_| None)
}

/// [`hook_apu`], with the soft draws the next frame will take
/// ([`Ext::apu_peek`]): a lockstep's model, which runs with the original's
/// draws script, hears what its next frame plays.
pub fn hook_apu_peek(
    spec: &Spec,
    world: &World,
    exts: &Exts,
    peek: &dyn Fn(&str) -> Option<u8>,
) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    for (i, b) in world.bodies.iter().enumerate() {
        if !b.active() {
            continue;
        }
        if let Some((n, h)) = spec
            .kind(b)
            .ext
            .as_ref()
            .and_then(|n| exts.get(n).map(|h| (n, h)))
        {
            crate::reads::note_hook(n);
            out.extend(h.apu_peek(spec, world, i, peek));
        }
    }
    out
}

/// Hooks by the name a model's `ext` gives them.
pub type Exts = BTreeMap<String, Box<dyn Ext>>;

/// What a hook asks for, applied in order when it returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Fire an event, handled as the components' events are.
    Fire(EventKind, Option<usize>),
    /// Add an entity of this kind at this position.
    Spawn {
        kind: usize,
        x: Fx,
        y: Fx,
        facing_left: bool,
        /// It starts at its whole place plus the fractions its numbered
        /// slot's last body left (`slot_rest`).
        slot_fraction: bool,
    },
    /// Take an entity out of the level.
    Remove(usize),
    /// Change the tile at a level pixel until the level is placed again.
    SetTile { x: i64, y: i64, tile: Tile },
    /// As `SetTile`, kept as a change even when it is the built kind.
    WriteTile { x: i64, y: i64, tile: Tile },
    /// Replace another body, as a pair moved by one routine needs. Players
    /// standing on it are placed on it again.
    SetBody(usize, Body),
    /// Set a numbered slot's word.
    SetSlotWord { slot: usize, word: i64 },
    /// Set a counter, by its index in the model's, firing no events.
    SetCounter { counter: usize, value: i64 },
    /// Replace the controller bits in `mask` with `bits` for the rest of
    /// this frame ([`ExtCx::hold`]).
    Hold { mask: u8, bits: u8 },
}

/// What a hook gets for one instance on one frame.
pub struct ExtCx<'a> {
    pub spec: &'a Spec,
    pub frame: u64,
    /// This instance's ID: its index in the world's bodies.
    pub id: usize,
    /// This instance, which the hook may change. It's written back when the
    /// hook returns.
    pub me: &'a mut Body,
    /// The world as it stands, read only. This instance and the random
    /// number generator in it are as they were before the hook ran; use
    /// `me` and `rng`.
    pub world: &'a World,
    /// The world's random number generator, the only randomness allowed.
    pub rng: &'a mut Rng,
    /// Whether a state stops the `counters` system this frame, as a game's
    /// frame timers stop for a frame the enemies still move on.
    pub counters_stopped: bool,
    /// Whether a state stops the system this hook runs in this frame, so it
    /// runs only for core `entity.hook_through_stops`: a game's frame
    /// timers that count while the routine that resets them doesn't run.
    pub system_stopped: bool,
    /// The run's random draws script, if it has one: [`Self::draw`].
    pub(crate) draws: Option<&'a mut crate::draws::DrawScript>,
    pub(crate) requests: Vec<Request>,
    /// Core `cost.frame`: the frame's CPU cycles so far.
    pub(crate) cycles: i64,
    /// Core `cost.frame`: the counters and the machines' states as the
    /// frame started ([`Self::counter_at_start`], [`Self::state_at_start`]).
    /// Empty without `cost`.
    pub(crate) start_counters: &'a [i64],
    pub(crate) start_states: &'a [usize],
}

impl ExtCx<'_> {
    /// Core `cost.frame`: adds `n` cycles to the frame's cost, for a hook
    /// that runs a routine of the original's and knows the branches it
    /// took.
    pub fn add_cycles(&mut self, n: i64) {
        self.cycles += n;
    }

    /// Core `cost.frame`: the frame's cost so far, the model's terms
    /// (summed before the `hook_last` hooks) and what hooks added, or
    /// `None` when the model has no `cost`.
    pub fn frame_cycles(&self) -> Option<i64> {
        self.spec.cost.as_ref().map(|_| self.cycles)
    }

    /// A random byte for the draw site `site`, as the original reads one of
    /// its random bytes there: the run's random draws script's, when it
    /// has one and hasn't desynced ([`crate::draws`]), else the low byte of
    /// the generator's next number. A hook tests the byte as the original
    /// does (Donkey Kong's ladder choice: `byte & 0x88 == 0`), so the odds
    /// are the original's too.
    pub fn draw(&mut self, site: &str) -> u8 {
        if let Some(v) = self.scripted_draw(site) {
            return v;
        }
        (self.rng.next_u64() & 0xFF) as u8
    }

    /// The run's random draws script's byte for the draw site `site`, as
    /// [`Self::draw`] takes it, or None when the run has no script or it
    /// has none here. A hook whose game keeps its random register as model
    /// state reads the register then (Donkey Kong's $18-$1F, stepped by its
    /// idle loop) in place of the generator.
    pub fn scripted_draw(&mut self, site: &str) -> Option<u8> {
        self.draws
            .as_deref_mut()
            .and_then(|d| d.take(self.frame, site))
    }

    /// The draws script's byte for the soft draw site `site` on this frame
    /// ([`crate::DrawScript::soft`]), or `None` without one: a byte read
    /// only for presentation, such as a sound driver's random pitch, which
    /// takes nothing from the generator.
    pub fn draw_soft(&mut self, site: &str) -> Option<u8> {
        self.draws.as_deref_mut()?.take(self.frame, site)
    }

    /// An event by the name the model uses, such as `player.hurt`.
    pub fn event(&self, name: &str) -> Option<EventKind> {
        self.spec
            .events
            .iter()
            .position(|e| e == name)
            .map(EventKind)
    }

    /// A number the model gives this instance's hook, by name, from its
    /// entity's `ext_params` as its state makes them. Panics on a name the
    /// model doesn't give, so a hook and a model that disagree fail loudly
    /// rather than run with a made-up number.
    pub fn param(&self, name: &str) -> Fx {
        let kind = self.spec.kind(self.me);
        *kind.ext_params.get(name).unwrap_or_else(|| {
            panic!(
                "the model gives entity `{}`'s ext hook no `ext_params.{name}`",
                kind.name
            )
        })
    }

    /// A table the model gives this instance's hook, by name, from its
    /// entity's `ext_tables`: a column of a lifted table, one number a row.
    /// Panics on a name the model doesn't give, as [`ExtCx::param`] does.
    pub fn table(&self, name: &str) -> &[i64] {
        let kind = self.spec.kind(self.me);
        kind.ext_tables.get(name).unwrap_or_else(|| {
            panic!(
                "the model gives entity `{}`'s ext hook no `ext_tables.{name}`",
                kind.name
            )
        })
    }

    /// Whether the model gives this instance's hook the number `name`.
    pub fn has_param(&self, name: &str) -> bool {
        self.spec.kind(self.me).ext_params.contains_key(name)
    }

    /// The byte `at` bits up in the shift counter `name`, as it will be at
    /// the end of this frame in the counters system (a register the game
    /// steps before its enemies run). None when the model has no such
    /// counter.
    pub fn shift_byte(&self, name: &str, at: u32) -> Option<u8> {
        let counter = self.spec.counters.index_of(name)?;
        self.spec.counters[counter].shift.as_ref()?;
        Some(crate::systems::shift_byte(
            self.spec, self.world, counter, at,
        ))
    }

    /// A counter's value by its name in the model, None if it has no such
    /// counter.
    pub fn counter(&self, name: &str) -> Option<i64> {
        let counter = self.spec.counters.index_of(name)?;
        Some(self.world.counters[counter])
    }

    /// Core `cost.frame`: a counter's value as the frame started, for a
    /// hook that costs a routine of the original's reading it before the
    /// frame changes it. The value now without `cost`, None if the model
    /// has no such counter.
    pub fn counter_at_start(&self, name: &str) -> Option<i64> {
        let counter = self.spec.counters.index_of(name)?;
        Some(
            self.start_counters
                .get(counter)
                .copied()
                .unwrap_or(self.world.counters[counter]),
        )
    }

    /// Whether the machine `machine` was in its state `state` as the frame
    /// started (core `cost.frame`; as it stands now without `cost`).
    pub fn state_at_start(&self, machine: &str, state: &str) -> bool {
        self.spec.machines.iter().enumerate().any(|(i, m)| {
            m.name == machine
                && self
                    .start_states
                    .get(i)
                    .copied()
                    .or(self.world.machines.get(i).map(|s| s.state))
                    .and_then(|s| m.states.get(s))
                    .is_some_and(|st| st.name == state)
        })
    }

    /// Sets a counter by its name in the model, for a value a game keeps
    /// apart from any object. Does nothing without such a counter.
    pub fn set_counter(&mut self, name: &str, value: i64) {
        if let Some(counter) = self.spec.counters.index_of(name) {
            self.requests.push(Request::SetCounter { counter, value });
        }
    }

    /// This instance's numbered slot, if it has one.
    pub fn slot(&self) -> Option<usize> {
        let id = self.id as u32 + 1;
        self.world.slots.iter().position(|s| s.body == id)
    }

    /// Sets this instance's numbered slot's word, which stays for the
    /// slot's next body. Does nothing without a slot.
    pub fn set_slot_word(&mut self, word: i64) {
        if let Some(slot) = self.slot() {
            self.requests.push(Request::SetSlotWord { slot, word });
        }
    }

    /// An entity kind by its name in the model.
    pub fn kind(&self, name: &str) -> Option<usize> {
        self.spec.kinds.index_of(name)
    }

    /// Whether an input action, by name, is held this frame, or newly
    /// pressed.
    pub fn held(&self, action: &str) -> bool {
        self.action(action)
            .is_some_and(|a| self.world.held(self.spec, a))
    }

    pub fn pressed(&self, action: &str) -> bool {
        self.action(action)
            .is_some_and(|a| self.world.pressed(self.spec, a))
    }

    /// Holds an input action, by name, for the rest of this frame, or lets
    /// it go: the systems after this hook see the pad so, and the next
    /// frame counts it as held on this one. A game that plays itself, as
    /// Donkey Kong's attract demo puts its script's direction where the
    /// player's routine reads the pad ($EBDA), with core
    /// `entity.hook_first` so it runs before the player moves. Does
    /// nothing for an action the model hasn't.
    pub fn hold(&mut self, action: &str, on: bool) {
        if let Some(a) = self.action(action) {
            let mask = self.spec.actions[a].mask;
            let bits = if on { mask } else { 0 };
            self.requests.push(Request::Hold { mask, bits });
        }
    }

    fn action(&self, name: &str) -> Option<usize> {
        self.spec.actions.iter().position(|a| a.name == name)
    }

    /// Whether the level pixel at `x`, `y` is in a solid tile, with the
    /// tiles changed so far.
    pub fn solid_at(&self, x: i64, y: i64) -> bool {
        self.world.tile_at(self.spec, x, y).solid()
    }

    /// The kind of the tile at a level pixel, with the tiles changed so
    /// far.
    pub fn tile_at(&self, x: i64, y: i64) -> Tile {
        self.world.tile_at(self.spec, x, y)
    }

    /// Asks for the tile at a level pixel to become `tile`, as pulling a
    /// vegetable clears its grass.
    pub fn set_tile(&mut self, x: i64, y: i64, tile: Tile) {
        self.requests.push(Request::SetTile { x, y, tile });
    }

    /// As `set_tile`, for a game that writes the cell's picture: the cell
    /// stays a change even when `tile` is its built kind, so presentation
    /// draws it by that kind (`present.cells.kinds`) in place of its built
    /// picture. Donkey Kong's 100m ending blanks the prizes, built as empty
    /// cells, and lays girder over the rivets' places.
    pub fn write_tile(&mut self, x: i64, y: i64, tile: Tile) {
        self.requests.push(Request::WriteTile { x, y, tile });
    }

    /// Fires one of the model's own events (`model.events`) for this
    /// instance only, so a hook moves its own machine by a transition the
    /// model names. Returns false when the model has no event of that name.
    pub fn fire_own(&mut self, name: &str) -> bool {
        match self.event(name) {
            Some(e) => {
                let id = self.id;
                self.fire(e, Some(id));
                true
            }
            None => false,
        }
    }

    pub fn fire(&mut self, kind: EventKind, entity: Option<usize>) {
        self.requests.push(Request::Fire(kind, entity));
    }

    pub fn spawn(&mut self, kind: usize, x: Fx, y: Fx, facing_left: bool) {
        self.requests.push(Request::Spawn {
            kind,
            x,
            y,
            facing_left,
            slot_fraction: false,
        });
    }

    /// [`Self::spawn`], with the new body at its whole place plus the
    /// fractions its numbered slot's last body left, for a game that copies
    /// only the whole pixels to a slot.
    pub fn spawn_in_slot(&mut self, kind: usize, x: Fx, y: Fx, facing_left: bool) {
        self.requests.push(Request::Spawn {
            kind,
            x,
            y,
            facing_left,
            slot_fraction: true,
        });
    }

    pub fn remove(&mut self, id: usize) {
        self.requests.push(Request::Remove(id));
    }

    /// Asks for body `id` to become `body`, a changed copy of it from the
    /// world, for a hook that moves two bodies as one.
    pub fn set_body(&mut self, id: usize, body: Body) {
        self.requests.push(Request::SetBody(id, body));
    }

    /// The requests so far, in order.
    pub fn requests(&self) -> &[Request] {
        &self.requests
    }
}
