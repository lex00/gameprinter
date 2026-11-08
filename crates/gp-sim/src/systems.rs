//! The systems a frame runs, one function each, and [`step`], which runs a
//! whole frame. A generated build may call [`step`] or call the systems
//! itself in [`Spec::order`] after [`begin_frame`], skipping the ones that
//! move while the frame is frozen, as [`step`] does; either way it runs the
//! same code as the interpreter.
//!
//! Every function here is deterministic: integer and fixed-point arithmetic
//! only, entities in index order, and nothing read but the spec, the world
//! and the frame's input.

use crate::ext::{ExtCx, Exts, Request};
use crate::fx::Fx;
use crate::spec::{Change, Do, EventKind, Orbit, Role, Spec, System, Touch};
use crate::spec::{Head, Tile};
use crate::world::{
    Body, Tiles, World, column_solid_t, ground_under_t, lands_row, middle, row_solid_t,
    touches_tile,
};

/// Something that happened in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub frame: u64,
    pub kind: EventKind,
    /// The entity it happened to, by index into [`World::bodies`].
    pub entity: Option<usize>,
}

/// What a frame carries from one system to the next, and nothing after it.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    /// The events so far, in order.
    pub events: Vec<Event>,
    /// Players that died or reached the goal this frame: they touch nothing
    /// more until the next frame.
    pub out: Vec<bool>,
    /// Whether a state that freezes the world was current when the frame
    /// began. A frozen frame runs only the state machines; a state entered
    /// or left during the frame freezes or frees the world from the next.
    pub frozen: bool,
    /// Systems a part freeze stops this frame (a current state's `stops`,
    /// core 0.4.12), and those a state entered this frame stops at once
    /// (`stops_at_once`) from then on.
    pub stopped: Vec<System>,
    /// Systems that run for the rest of a frozen frame because a state
    /// left during it frees them at once (`state.frees_at_once`).
    pub freed: Vec<System>,
    /// Counters, by index, whose `every` changes wait this frame (a
    /// current state's `holds`).
    pub holds: Vec<usize>,
    /// How deeply events are firing other events, so a rule that restarts
    /// the level on `level.started` can't loop forever.
    depth: u32,
    /// Game machines and entities that entered their state this frame, so
    /// their system doesn't count this frame toward `after`.
    pub fresh_rules: Vec<bool>,
    pub fresh_bodies: Vec<bool>,
    /// A level started during this frame: the systems that move stop for
    /// the rest of it, so the new level's first moving frame is the next.
    pub level_started: bool,
    /// The level started this frame was entered (`enter-level`), which
    /// keeps the counters and their clocks: the `counters` system still
    /// counts this frame, as it would have without the level change.
    pub level_entered: bool,
    /// Bodies wall points pushed this frame and by how much across, which
    /// a camera `by_move` doesn't count as a move.
    pub pushed: Vec<(usize, i64)>,
    /// Bodies whose feet were too deep to land this frame: no wall points.
    pub feet_walled: Vec<usize>,
    /// Bodies that appeared by the view this frame in a level with
    /// `appear_still`: the enemies system doesn't move them until the next.
    pub still: Vec<usize>,
    /// The numbered slot of the body whose `contact.in_turn` touch this is,
    /// while the touch runs: a `stops_at_once` of the enemies it sets off
    /// stops them from that slot.
    pub hitter_slot: Option<usize>,
    /// The enemies system stops from this numbered slot on: the bodies in
    /// earlier slots have had their turn in the game's loop.
    pub stopped_from: Option<usize>,
    /// Who stands on what (`contact.ride`): the player, the entity and how
    /// far below it the box it stands on is. The player is placed after the
    /// entity's move.
    pub rides: Vec<(usize, usize, i64)>,
    /// The random draws script hooks draw from this frame, if the run has
    /// one ([`step_with_draws`]).
    pub draws: Option<crate::draws::DrawScript>,
    /// Core `cost.frame`: the frame's CPU cycles so far, the model's terms
    /// and what hooks added ([`frame_cost`], [`crate::ExtCx::add_cycles`]).
    pub cycles: i64,
    /// Core `cost.frame`: the counters and the game machines' states as
    /// the frame started, which a term can read. Empty without `cost`.
    pub start_counters: Vec<i64>,
    pub start_states: Vec<usize>,
    /// Core `cost.frame_pad`: controller 1's bits as pressed this frame.
    pub pad: u8,
}

/// Runs one frame with these controller bits held, and returns its events.
pub fn step(spec: &Spec, w: &mut World, buttons: u8) -> Vec<Event> {
    step_with_ext(spec, w, buttons, &Exts::new())
}

/// [`step`], running the `ext` hooks the registry has.
pub fn step_with_ext(spec: &Spec, w: &mut World, buttons: u8, exts: &Exts) -> Vec<Event> {
    step_with_draws(spec, w, buttons, exts, &mut None)
}

/// [`step_with_ext`], with the hooks drawing from `draws`, a random draws
/// script, when there is one ([`crate::draws`]). A draw of the original's
/// left untaken by the frame's end is a desync.
pub fn step_with_draws(
    spec: &Spec,
    w: &mut World,
    buttons: u8,
    exts: &Exts,
    draws: &mut Option<crate::draws::DrawScript>,
) -> Vec<Event> {
    step_pads(spec, w, [buttons, 0], exts, draws)
}

/// [`step_with_draws`] with each controller's bits, controller 1 first,
/// for a model whose players play from their own (core `input.turn`).
pub fn step_pads(
    spec: &Spec,
    w: &mut World,
    pads: [u8; 2],
    exts: &Exts,
    draws: &mut Option<crate::draws::DrawScript>,
) -> Vec<Event> {
    let mut f = begin_frame_pads(spec, w, pads);
    f.draws = draws.take();
    frame_start_hooks(spec, w, &mut f, exts);
    for system in &spec.order {
        if f.runs(*system) {
            run_system_ext(*system, spec, w, &mut f, exts);
        } else if f.frozen_for(*system) {
            run_stopped_ext(*system, spec, w, &mut f, exts);
        }
    }
    end_frame_ext(spec, w, &mut f, exts);
    *draws = f.draws.take();
    if let Some(d) = draws {
        d.check_through(w.frame);
    }
    f.events
}

impl Frame {
    /// Whether `system` runs now: not while the world is frozen or a level
    /// has started this frame, if it moves things, nor while a state stops
    /// it.
    pub fn runs(&self, system: System) -> bool {
        // An entered level's counters go on counting (`enter-level`).
        let started = self.level_started && !(self.level_entered && system == System::Counters);
        let freed = !started && self.freed.contains(&system);
        (freed || !((self.frozen || started) && system.moves())) && !self.stopped.contains(&system)
    }

    /// Whether `system` is skipped now for a freeze, whole or part, rather
    /// than for a level start: [`run_frozen`] runs in its place.
    pub fn frozen_for(&self, system: System) -> bool {
        !self.level_started
            && ((self.frozen && system.moves() && !self.freed.contains(&system))
                || self.stopped.contains(&system))
    }
}

/// The direction bits of the runtime protocol's controller byte: up, down,
/// left and right.
pub const DIRECTIONS: u8 = 0xF0;

/// Core `input.keep_direction`: the controller byte as the game sees it,
/// from the one it saw the frame before and the one held now. When some
/// direction it saw is still held, it sees only those directions; else it
/// sees every direction held. Buttons pass as they are.
pub fn keep_direction(before: u8, held: u8) -> u8 {
    let kept = before & held & DIRECTIONS;
    if kept != 0 {
        (held & !DIRECTIONS) | kept
    } else {
        held
    }
}

/// Starts a frame: counts it, takes the input, and decides whether it's
/// frozen.
pub fn begin_frame(spec: &Spec, w: &mut World, buttons: u8) -> Frame {
    begin_frame_pads(spec, w, [buttons, 0])
}

/// The controller byte the model reads from each controller's: core
/// `input.turn` takes its actions' bits from the controller of the player
/// whose turn it is (controller k + 1 while its counter is k, none past
/// the controllers given), and every other bit from controller 1.
pub fn turn_pad(spec: &Spec, w: &World, pads: [u8; 2]) -> u8 {
    match spec.turn {
        Some((c, mask)) => {
            let k = w.counters.get(c).copied().unwrap_or(0);
            let theirs = usize::try_from(k)
                .ok()
                .and_then(|k| pads.get(k))
                .copied()
                .unwrap_or(0);
            (pads[0] & !mask) | (theirs & mask)
        }
        None => pads[0],
    }
}

/// [`begin_frame`] with each controller's bits ([`turn_pad`]).
pub fn begin_frame_pads(spec: &Spec, w: &mut World, pads: [u8; 2]) -> Frame {
    let (start_counters, start_states) = if spec.cost.is_some() {
        (
            w.counters.clone(),
            w.machines.iter().map(|m| m.state).collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let buttons = turn_pad(spec, w, pads);
    w.frame = w.frame.wrapping_add(1);
    w.prev = w.held;
    w.held = if spec.keep_direction {
        keep_direction(w.prev, buttons)
    } else {
        buttons
    };
    // An exit's scroll freezes the world for its frames.
    let scrolling = w.scroll > 0;
    w.scroll = w.scroll.saturating_sub(1);
    // The current states of the game's machines and the active entities'.
    let states: Vec<(bool, &crate::spec::Part)> = w
        .machines
        .iter()
        .zip(&spec.machines)
        .map(|(m, s)| (s.states[m.state].freeze, &s.states[m.state].part))
        .chain(w.bodies.iter().filter(|b| b.active()).filter_map(|b| {
            spec.kinds[b.kind]
                .machine
                .as_ref()
                .map(|m| (m.states[b.state].freeze, &m.states[b.state].part))
        }))
        .collect();
    let mut stopped: Vec<System> = Vec::new();
    let mut holds: Vec<usize> = Vec::new();
    for (_, part) in &states {
        // `state.stops_machines`: the entities' machines wait.
        if part.stops_machines && !stopped.contains(&System::EntityStates) {
            stopped.push(System::EntityStates);
        }
        for s in &part.stops {
            if !stopped.contains(s) {
                stopped.push(*s);
            }
        }
        for c in &part.holds {
            if !holds.contains(c) {
                holds.push(*c);
            }
        }
    }
    let f = Frame {
        frozen: scrolling || states.iter().any(|(freeze, _)| *freeze),
        stopped,
        freed: Vec::new(),
        holds,
        out: vec![false; w.bodies.len()],
        events: Vec::new(),
        pushed: Vec::new(),
        feet_walled: Vec::new(),
        still: Vec::new(),
        hitter_slot: None,
        stopped_from: None,
        rides: Vec::new(),
        depth: 0,
        fresh_rules: vec![false; spec.machines.len()],
        fresh_bodies: vec![false; w.bodies.len()],
        level_started: false,
        level_entered: false,
        draws: None,
        cycles: 0,
        start_counters,
        start_states,
        pad: pads[0],
    };
    if !f.frozen {
        release_held_cells(spec, w);
    }
    let mut f = f;
    count_pending(spec, w, &mut f);
    f
}

/// [`begin_frame_pads`], then the hooks of the entities with core
/// `entity.hook_frame_start` ([`crate::Ext::frame_start`]), in body order,
/// before any system and whether the frame is frozen or not: a game's
/// sound driver, which its NMI runs first on every frame.
pub fn begin_frame_ext(spec: &Spec, w: &mut World, pads: [u8; 2], exts: &Exts) -> Frame {
    let mut f = begin_frame_pads(spec, w, pads);
    frame_start_hooks(spec, w, &mut f, exts);
    f
}

fn frame_start_hooks(spec: &Spec, w: &mut World, f: &mut Frame, exts: &Exts) {
    for i in 0..w.bodies.len() {
        if spec.kind(&w.bodies[i]).hook_frame_start {
            run_hook(spec, w, f, i, exts, Phase::FrameStart);
        }
    }
}

/// Counts down the counter changes waiting for their `after` (core
/// 0.4.13), every frame, frozen or not: one with none left is made now,
/// in the order the events fired.
fn count_pending(spec: &Spec, w: &mut World, f: &mut Frame) {
    if w.pending.is_empty() {
        return;
    }
    let mut due = Vec::new();
    w.pending.retain_mut(|(c, by, left)| {
        *left = left.saturating_sub(1);
        if *left == 0 {
            due.push((*c as usize, *by));
        }
        *left > 0
    });
    for (c, by) in due {
        change_counter(spec, w, f, c, by);
    }
}

/// Counts down the cells head hits hold solid (`hold`): each loses a
/// frame, and one with none left takes its next kind, so a cell held for
/// `n` frames stops a body on the `n` frames after the hit's and has its
/// new kind from the frame after those.
fn release_held_cells(spec: &Spec, w: &mut World) {
    if w.held_cells.is_empty() {
        return;
    }
    let level = &spec.levels[w.level];
    let mut done = Vec::new();
    w.held_cells.retain_mut(|(c, t, left)| {
        *left = left.saturating_sub(1);
        if *left == 0 {
            done.push((*c, *t));
        }
        *left > 0
    });
    for (c, t) in done {
        let (x, y) = level.cell_pixel(c as usize);
        w.set_tile(spec, x, y, t);
    }
}

pub fn run_system(system: System, spec: &Spec, w: &mut World, f: &mut Frame) {
    run_system_ext(system, spec, w, f, &Exts::new());
}

/// [`run_system`], running the `ext` hooks the registry has in the player
/// and enemy systems.
pub fn run_system_ext(system: System, spec: &Spec, w: &mut World, f: &mut Frame, exts: &Exts) {
    match system {
        System::Rules => rules(spec, w, f),
        System::EntityStates => entity_states(spec, w, f),
        System::Counters => counters(spec, w, f),
        System::Spawn => spawn(spec, w, f),
        System::Camera => camera(spec, w, f),
        System::Player => player_ext(spec, w, f, exts),
        System::Enemies => enemies_ext(spec, w, f, exts),
        System::Contact => contact_ext(spec, w, f, exts),
        System::Exits => exits(spec, w, f),
        System::TopdownPlayer => topdown_player(spec, w, f),
    }
}

/// The game's state machines: each counts a frame in its state, then takes
/// the first `after` transition whose time is up and whose condition holds.
pub fn rules(spec: &Spec, w: &mut World, f: &mut Frame) {
    for i in 0..spec.machines.len() {
        // A machine that entered its state this frame doesn't count it.
        if std::mem::take(&mut f.fresh_rules[i]) {
            continue;
        }
        let m = &mut w.machines[i];
        m.time = m.time.saturating_add(1);
        let (state, time) = (m.state, m.time);
        let due = spec.machines[i].states[state]
            .transitions
            .iter()
            .position(|t| ready(spec, w, t, time) && on_clock(spec, w, t) && guard(t, w));
        if let Some(t) = due {
            take(spec, w, f, i, t);
        }
    }
}

/// Each entity's own machine: every active entity with one counts a frame
/// in its state, then takes the first `after` transition whose time is up
/// and whose condition holds.
pub fn entity_states(spec: &Spec, w: &mut World, f: &mut Frame) {
    for i in 0..w.bodies.len() {
        // An action may have started a level and replaced the bodies.
        if i >= w.bodies.len() || f.level_started {
            break;
        }
        let b = &mut w.bodies[i];
        let Some(m) = &spec.kinds[b.kind].machine else {
            continue;
        };
        if !b.active() || std::mem::take(&mut f.fresh_bodies[i]) {
            continue;
        }
        // A state with `ground = "land"` (core 0.4.13): from the frame
        // after it is entered, a body off the ground with ground under it
        // stands, whatever moves it.
        if m.states[b.state].part.ground == Some(crate::spec::Ground::Land) && !b.on_ground {
            let nb = *b;
            let k = spec.kind(&nb);
            if k.tile_collision && ground_under_t(w.tiles(spec), k, &nb) {
                w.bodies[i].on_ground = true;
            }
        }
        let b = &mut w.bodies[i];
        b.time = b.time.saturating_add(1);
        // `state.step_word`: the step counter goes on, frozen or not.
        let cycle = m.states[b.state].part.step_word;
        if cycle > 0 {
            b.ext[0] = (b.ext[0] + 1).rem_euclid(cycle);
        }
        let (state, time) = (b.state, b.time);
        let (y, words) = (w.bodies[i].y, w.bodies[i].ext);
        let due = m.states[state].transitions.iter().position(|t| {
            let ready = match t.below {
                // A depth: taken once the body is there, its `after` up.
                Some(below) => y >= below && t.after.is_none_or(|a| time >= a),
                None => ready(spec, w, t, time),
            };
            // `transition.word`: one of its words in a range.
            let word = t.word.is_none_or(|(k, lo, hi)| {
                let v = words[k];
                lo.is_none_or(|lo| v >= lo) && hi.is_none_or(|hi| v < hi)
            });
            ready && word && on_clock(spec, w, t) && guard(t, w)
        });
        if let Some(t) = due {
            take_entity(spec, w, f, i, t);
        }
    }
}

/// Whether a transition by time or by a press is due at `time` frames in
/// its state: with `pressed` (feature `transition.pressed`), on a frame its
/// action is newly pressed and its `after`, if any, is up; else
/// [`timed_due`].
fn ready(spec: &Spec, w: &World, t: &crate::spec::Transition, time: u32) -> bool {
    match (t.pressed, t.held) {
        (Some(a), _) => w.pressed(spec, a) && t.after.is_none_or(|after| time >= after),
        (None, Some(a)) => w.held(spec, a) && t.after.is_none_or(|after| time >= after),
        (None, None) => timed_due(t, time, w.frame),
    }
}

/// Whether a timed transition is due at `time` frames in its state on
/// frame `frame`: its `after` is up and, with `align`, the frame number is
/// a multiple of it.
fn timed_due(t: &crate::spec::Transition, time: u32, frame: u64) -> bool {
    t.after.is_some_and(|after| time >= after)
        && t.align
            .is_none_or(|a| a > 0 && frame.is_multiple_of(u64::from(a)))
}

/// Whether a transition with `align_to` may be taken this frame: its
/// counter's first `every` clock has just come round, its count back at 0.
/// One without `align_to` always may.
fn on_clock(spec: &Spec, w: &World, t: &crate::spec::Transition) -> bool {
    let Some(c) = t.align_to else { return true };
    clock_of(spec, c).is_some_and(|k| w.ticks.get(k) == Some(&0))
}

/// The index in [`World::ticks`] of counter `c`'s first `every` change.
fn clock_of(spec: &Spec, c: usize) -> Option<usize> {
    let before: usize = spec.counters[..c]
        .iter()
        .map(|k| {
            k.changes
                .iter()
                .filter(|ch| matches!(ch, Change::Every(..)))
                .count()
        })
        .sum();
    spec.counters[c]
        .changes
        .iter()
        .any(|ch| matches!(ch, Change::Every(..)))
        .then_some(before)
}

/// What entering `state` does to the frame at once: the systems it stops
/// at once stop for the rest of it.
fn enter(f: &mut Frame, part: &crate::spec::Part) {
    for s in &part.stops_at_once {
        if *s == System::Enemies && f.hitter_slot.is_some() {
            f.stopped_from = f.hitter_slot;
        } else if !f.stopped.contains(s) {
            f.stopped.push(*s);
        }
    }
}

/// Takes transition `t` of entity `i`'s machine.
fn take_entity(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, t: usize) {
    let m = spec.kinds[w.bodies[i].kind]
        .machine
        .as_ref()
        .expect("an entity with a machine");
    let tr = &m.states[w.bodies[i].state].transitions[t];
    w.bodies[i].state = tr.to;
    if !m.states[tr.to].part.keep_time {
        w.bodies[i].time = 0;
    }
    enter(f, &m.states[tr.to].part);
    if m.states[tr.to].part.ground == Some(crate::spec::Ground::Leave) {
        w.bodies[i].on_ground = false;
    }
    if let Some([vx, vy]) = m.states[tr.to].part.enter_speed {
        let b = &mut w.bodies[i];
        b.vx = vx;
        b.vy = if m.states[tr.to].part.enter_keeps_fraction {
            Fx::from_int(vy.floor()).add(Fx(b.vy.frac()))
        } else {
            vy
        };
    }
    if let Some(fresh) = f.fresh_bodies.get_mut(i) {
        *fresh = true;
    }
    for action in &tr.actions {
        match *action {
            Do::Remove => w.bodies[i].alive = false,
            Do::Become(kind) => replace_by(spec, w, f, i, kind),
            a => act(spec, w, f, a),
        }
    }
}

/// A new body of `kind` at `x`, `y`, facing as given, alive and in its
/// machine's initial state, standing if there's ground under it.
pub fn new_body(spec: &Spec, w: &World, kind: usize, x: Fx, y: Fx, facing_left: bool) -> Body {
    let base = &spec.kinds[kind];
    let mut nb = Body {
        kind,
        x,
        y,
        facing_left,
        alive: true,
        state: base.machine.as_ref().map_or(0, |m| m.initial),
        home: [x, y],
        ..Body::default()
    };
    let k = spec.kind(&nb);
    nb.fall_gravity = crate::world::start_fall_gravity(k);
    nb.on_ground = k.tile_collision && ground_under_t(w.tiles(spec), k, &nb);
    nb
}

/// Replaces body `i` by a new body of `kind` at its position, facing the
/// same way, in its initial state with no speed, as a new entity of the
/// level.
fn replace_by(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, kind: usize) {
    let old = w.bodies[i];
    w.bodies[i].alive = false;
    let nb = new_body(spec, w, kind, old.x, old.y, old.facing_left);
    w.bodies.push(nb);
    f.out.push(false);
    f.fresh_bodies.push(true);
    // The new body keeps the old one's numbered slot (`slot_rest`).
    let new = w.bodies.len() as u32;
    for s in &mut w.slots {
        if s.body == i as u32 + 1 {
            s.body = new;
        }
    }
}

/// Frees each numbered slot (`slot_rest`) whose body has left, keeping the
/// fraction of its place across, its speed up and down and the fraction of
/// its height. A body's `extra_slots` hold nothing of it, so they keep what
/// they had. A pivot with `orbit.angle_in_slot` leaves its whole angle as
/// that speed's whole part.
fn release_slots(spec: &Spec, w: &mut World) {
    let mut seen = Vec::new();
    for s in &mut w.slots {
        let Some(b) = (s.body as usize)
            .checked_sub(1)
            .and_then(|i| w.bodies.get(i))
        else {
            continue;
        };
        if !b.active() {
            if !seen.contains(&s.body) {
                seen.push(s.body);
                let keeps = spec.kind(b).orbit.as_ref().is_some_and(|o| o.angle_in_slot);
                let vy = if keeps {
                    Fx::from_int(b.ext[0] >> 8).add(Fx(b.vy.frac()))
                } else {
                    b.vy
                };
                s.rest = [Fx(b.x.frac()), vy, Fx(b.y.frac())];
            }
            s.body = 0;
        }
    }
}

/// Gives body `i` the lowest free numbered slot (`slot_rest`), if it is of
/// a kind that takes one, and returns what the slot's last body left. Its
/// `extra_slots` take the next free ones, as many as there are.
pub fn claim_slot(spec: &Spec, w: &mut World, i: usize) -> Option<[Fx; 3]> {
    let kind = &spec.kinds[w.bodies.get(i)?.kind];
    if w.slots.is_empty() || kind.role == Role::Player || kind.no_slot {
        return None;
    }
    release_slots(spec, w);
    let s = w.slots.iter_mut().find(|s| s.body == 0)?;
    s.body = i as u32 + 1;
    let rest = s.rest;
    let free = w.slots.iter_mut().filter(|s| s.body == 0);
    for s in free.take(kind.extra_slots as usize) {
        s.body = i as u32 + 1;
    }
    Some(rest)
}

/// Gives a body set up by whole bytes the fractions of its place its
/// slot's last body left.
fn take_rest(b: &mut Body, x_frac: Fx, y_frac: Fx) {
    b.x = Fx::from_int(b.x.floor()).add(x_frac);
    b.y = Fx::from_int(b.y.floor()).add(y_frac);
}

fn guard(t: &crate::spec::Transition, w: &World) -> bool {
    t.guard
        .is_none_or(|g| g.cmp.holds(w.counters[g.counter], g.value))
}

/// Counters with `every` changes: each counts a frame and changes when its
/// count comes round.
pub fn counters(spec: &Spec, w: &mut World, f: &mut Frame) {
    count_every(spec, w, f, false);
}

/// What a system that moves things still does on a frozen frame: the
/// `counters` system counts the counters with `runs_when_frozen` (core
/// 0.4.8), `platformer.enemies` lands the `land_while_stopped` hoppers, and
/// the others do nothing. Every runtime's frame loop calls this
/// where it skips a moving system for a freeze.
/// [`run_frozen`], then, on a frame the system doesn't run but the
/// `counters` system does (a state's `stops` list it, or a freeze frees the
/// counters at once), the `ext` hooks of the entities with core
/// `entity.hook_through_stops` that run in it (`platformer.enemies` and
/// `platformer.player`), in body order: a game's frame timers, which count
/// whenever its timer routine runs.
/// Core `entity.hook_last`: each such hook, in body order, once at the
/// end of the frame after every system, whatever ran: a frozen frame, a
/// stopped one and a level start's too. A game's idle loop runs every
/// frame after its frame's work, as Donkey Kong's ($C7E1) steps its random
/// register while it waits for the next NMI.
pub fn end_frame_ext(spec: &Spec, w: &mut World, f: &mut Frame, exts: &Exts) {
    if spec.cost.is_some() {
        f.cycles += frame_cost(spec, w, f);
    }
    for i in 0..w.bodies.len() {
        if spec.kind(&w.bodies[i]).hook_last {
            run_hook(spec, w, f, i, exts, Phase::Step);
        }
    }
}

/// Core `cost.frame`: the model's terms for this frame, `base` and each
/// term whose tests hold, a `per` term once for each active body of its
/// kind passing its body tests. Integer cycles: nothing here is a float.
pub fn frame_cost(spec: &Spec, w: &World, f: &Frame) -> i64 {
    use crate::spec::{CostOn, CostTest};
    let Some(cost) = &spec.cost else {
        return 0;
    };
    let value = |t: &CostTest, b: Option<&crate::world::Body>| -> i64 {
        match t.on {
            CostOn::Counter(c, true) => f
                .start_counters
                .get(c)
                .or(w.counters.get(c))
                .copied()
                .unwrap_or(0),
            CostOn::Counter(c, false) => w.counters.get(c).copied().unwrap_or(0),
            CostOn::Pad(mask) => i64::from(f.pad & mask != 0),
            CostOn::Held(mask, now) => {
                let bits = if now { w.held } else { w.prev };
                i64::from(bits & mask != 0)
            }
            CostOn::State(m, st) => {
                let at = f
                    .start_states
                    .get(m)
                    .copied()
                    .or(w.machines.get(m).map(|x| x.state));
                i64::from(at == Some(st))
            }
            CostOn::Word(k) => b.map_or(0, |b| b.ext[k]),
        }
    };
    let holds = |t: &CostTest, b: Option<&crate::world::Body>| {
        let v = value(t, b);
        v >= t.lo && v < t.hi
    };
    let mut sum = cost.base;
    for t in &cost.terms {
        if !t.tests.iter().all(|x| holds(x, None)) {
            continue;
        }
        let n = match t.per {
            Some(k) => w
                .bodies
                .iter()
                .filter(|b| b.active() && b.kind == k)
                .filter(|b| t.per_tests.iter().all(|x| holds(x, Some(b))))
                .count() as i64,
            None => 1,
        };
        sum += t.cycles * n;
    }
    sum
}

pub fn run_stopped_ext(system: System, spec: &Spec, w: &mut World, f: &mut Frame, exts: &Exts) {
    run_frozen(system, spec, w, f);
    if f.level_started || f.runs(system) || !f.runs(System::Counters) {
        return;
    }
    let mine = |k: &crate::spec::Kind| match system {
        System::Enemies => k.role != Role::Player,
        System::Player => k.role == Role::Player,
        _ => false,
    };
    for i in 0..w.bodies.len() {
        let k = spec.kind(&w.bodies[i]);
        if k.hook_through_stops && !k.hook_last && mine(k) {
            run_hook(spec, w, f, i, exts, Phase::Step);
        }
    }
}

pub fn run_frozen(system: System, spec: &Spec, w: &mut World, f: &mut Frame) {
    match system {
        System::Counters => count_every(spec, w, f, true),
        // A hopper with `land_while_stopped` lands, and doesn't move, and a
        // cannon's timer counts down. One that appeared this frame has its
        // first turn on the next, as on a frame that moves (the game's
        // enemy init runs in place of the enemy's turn).
        System::Enemies => {
            for i in 0..w.bodies.len() {
                let lands = spec
                    .kind(&w.bodies[i])
                    .hop
                    .as_ref()
                    .is_some_and(|h| h.land_while_stopped);
                if lands
                    && !f.still.contains(&i)
                    && w.bodies[i].active()
                    && w.bodies[i].carrier == 0
                {
                    land_first(spec, w, i);
                }
            }
            // A launcher's timer counts down through a freeze; nothing
            // fires and no shot moves (ProcessCannons, BulletBillHandler's
            // TimerControl checks).
            let held = shot_slots(spec, w);
            launchers(spec, w, f, &Exts::new(), &held, true);
        }
        // A camera `by_move` that doesn't move still keeps where its
        // entity is in its view (core 0.4.12).
        System::Camera => {
            for i in 0..spec.cameras.len() {
                if spec.cameras[i].by_move.is_none() {
                    continue;
                }
                // A hold that is a frame timer counts down while a state
                // stops the camera (core 0.4.23): SMB's ScrollLock stops
                // ScrollHandler, not DecTimers.
                if spec.cameras[i]
                    .by_move
                    .is_some_and(|m| m.hold_with_counters)
                    && timers_run(f)
                    && let Some(h) = w.camera_hold.get_mut(i)
                {
                    *h = h.saturating_sub(1);
                }
                let follow = w.of_kind(spec.cameras[i].follow).next().map(|(j, _)| j);
                let pushed: i64 = f
                    .pushed
                    .iter()
                    .filter(|(j, _)| Some(*j) == follow)
                    .map(|(_, by)| by)
                    .sum();
                let still = f.frozen || follow.is_some_and(|j| entity_stopped(spec, w, f, j));
                note_move(spec, w, i, pushed, still);
                keep_in_view(spec, w, i);
                see(spec, w, i);
            }
        }
        _ => {}
    }
}

fn count_every(spec: &Spec, w: &mut World, f: &mut Frame, frozen: bool) {
    let mut tick = 0;
    for (i, c) in spec.counters.iter().enumerate() {
        if let Some(shift) = &c.shift
            && (!frozen || c.runs_when_frozen)
        {
            w.counters[i] = shift.step(w.counters[i]);
        }
        for change in &c.changes {
            if let Change::Every(every, by) = *change {
                if frozen && !c.runs_when_frozen {
                    tick += 1;
                    continue;
                }
                // A held counter counts up to its change and waits there.
                if f.holds.contains(&i) {
                    w.ticks[tick] = w.ticks[tick].saturating_add(1).min(every);
                    tick += 1;
                    continue;
                }
                w.ticks[tick] = w.ticks[tick].saturating_add(1);
                if w.ticks[tick] >= every {
                    w.ticks[tick] = 0;
                    change_counter(spec, w, f, i, by);
                }
                tick += 1;
            }
        }
    }
}

/// Changes counter `i` by `by` within its bounds, firing its events.
fn change_counter(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, by: i64) {
    let c = &spec.counters[i];
    let old = w.counters[i];
    let mut v = old.wrapping_add(by);
    let (reached_min, wrapped) = Spec::counter_events(i);
    let mut did_wrap = false;
    if let Some(max) = c.max
        && v > max
    {
        if c.wrap {
            // Compile makes sure the range from `min` (or 0) to `max` isn't
            // empty; i128 keeps extreme bounds from overflowing.
            let lo = i128::from(c.min.unwrap_or(0));
            let span = (i128::from(max) - lo + 1).max(1);
            v = (lo + (i128::from(v) - lo).rem_euclid(span)) as i64;
            did_wrap = true;
        } else {
            v = max;
        }
    }
    if let Some(min) = c.min
        && v < min
    {
        v = min;
    }
    w.counters[i] = v;
    // A counter that keeps this one's highest value is raised with it.
    for (j, k) in spec.counters.iter().enumerate() {
        if k.high_of == Some(i) && w.counters[j] < v {
            w.counters[j] = v;
        }
    }
    if did_wrap {
        emit(spec, w, f, wrapped, None);
    }
    if c.min.is_some_and(|min| v == min && old != min) {
        emit(spec, w, f, reached_min, None);
    }
    // Core `stages.skip`: the rules stepped the stage counter onto a stage
    // the game passes over, so it steps on the same way (Donkey Kong (JU)
    // goes from 25m's $53 = 1 to 75m's 3). Compile refuses a list that
    // skips every stage.
    if by != 0 && skipped_stage(spec, w, i) {
        change_counter(spec, w, f, i, by.signum());
    }
}

/// Fires an event: changes the counters that count it, and moves each rule
/// whose current state has a transition on it whose condition holds.
pub fn emit(spec: &Spec, w: &mut World, f: &mut Frame, kind: EventKind, entity: Option<usize>) {
    f.events.push(Event {
        frame: w.frame,
        kind,
        entity,
    });
    if f.depth >= 16 {
        return;
    }
    f.depth += 1;
    for i in 0..spec.counters.len() {
        for change in &spec.counters[i].changes {
            match change {
                Change::On(on, by) if *on == kind => change_counter(spec, w, f, i, *by),
                Change::When(c) if c.on == kind => {
                    let of = entity.and_then(|e| w.bodies.get(e)).map(|b| b.kind);
                    if !c.of.is_empty() && !of.is_some_and(|k| c.of.contains(&k)) {
                        continue;
                    }
                    if c.guard
                        .is_some_and(|g| !g.cmp.holds(w.counters[g.counter], g.value))
                    {
                        continue;
                    }
                    if c.after == 0 {
                        change_counter(spec, w, f, i, c.by);
                    } else {
                        w.pending.push((i as u32, c.by, c.after));
                    }
                }
                _ => {}
            }
        }
    }
    for i in 0..spec.machines.len() {
        let state = w.machines[i].state;
        let on = spec.machines[i].states[state]
            .transitions
            .iter()
            .position(|t| t.on == Some(kind) && guard(t, w));
        if let Some(t) = on {
            take(spec, w, f, i, t);
        }
    }
    // Each entity's machine sees the events that concern it: the player's
    // own, the stomped or kicked entity's, the collected item's and the
    // player's for a collection, and every other event.
    let started = f.level_started;
    for i in 0..w.bodies.len() {
        // A transition may have started a level and replaced the bodies;
        // the new ones don't see the event that started it.
        if i >= w.bodies.len() || f.level_started != started {
            break;
        }
        let b = w.bodies[i];
        let Some(m) = &spec.kinds[b.kind].machine else {
            continue;
        };
        if !b.active() {
            continue;
        }
        let player = spec.kinds[b.kind].role == Role::Player;
        let concerns = match kind {
            EventKind::PLAYER_DIED | EventKind::PLAYER_HURT => entity == Some(i),
            EventKind::ENEMY_STOMPED
            | EventKind::ENEMY_KICKED
            | EventKind::ENEMY_HURT_PLAYER
            | EventKind::ENEMY_HIT
            | EventKind::ENEMY_DEFEATED
            | EventKind::ENEMY_STOOD_ON => entity == Some(i),
            EventKind::ITEM_COLLECTED | EventKind::ITEM_LIFTED | EventKind::ITEM_THROWN => {
                entity == Some(i) || player
            }
            // The model's own events: one fired for an entity concerns
            // that instance only.
            EventKind(k) if k >= spec.own_events && entity.is_some() => entity == Some(i),
            _ => true,
        };
        if !concerns {
            continue;
        }
        let on = m.states[b.state]
            .transitions
            .iter()
            .position(|t| t.on == Some(kind) && guard(t, w));
        if let Some(t) = on {
            take_entity(spec, w, f, i, t);
        }
    }
    f.depth -= 1;
}

/// Takes transition `t` of machine `m`'s current state.
fn take(spec: &Spec, w: &mut World, f: &mut Frame, m: usize, t: usize) {
    let from = &spec.machines[m].states[w.machines[m].state];
    let tr = &from.transitions[t];
    // `state.frees_at_once`: leaving the state frees these systems for the
    // rest of the frame.
    for s in &from.part.frees_at_once {
        if !f.freed.contains(s) {
            f.freed.push(*s);
        }
    }
    w.machines[m].state = tr.to;
    if !spec.machines[m].states[tr.to].part.keep_time {
        w.machines[m].time = 0;
    }
    enter(f, &spec.machines[m].states[tr.to].part);
    if let Some(fresh) = f.fresh_rules.get_mut(m) {
        *fresh = true;
    }
    for action in &tr.actions {
        act(spec, w, f, *action);
    }
}

/// Core `stages`: the level that plays the stage the stage counter names
/// (the last stage's for a value past the list, the first's below it).
fn stage_level(spec: &Spec, w: &World) -> Option<usize> {
    let (c, plays) = spec.stages.as_ref()?;
    let last = plays.len().checked_sub(1)?;
    let i = usize::try_from(w.counters[*c].max(0))
        .unwrap_or(last)
        .min(last);
    // A stage the game passes over is never the counter's (change_counter
    // steps past it); one set there plays the next stage's level.
    plays[i..].iter().chain(&plays[..i]).find_map(|p| *p)
}

/// Core `stages.skip`: the stage counter's value names a stage the game
/// passes over.
fn skipped_stage(spec: &Spec, w: &World, i: usize) -> bool {
    spec.stages.as_ref().is_some_and(|(c, plays)| {
        *c == i
            && usize::try_from(w.counters[i])
                .ok()
                .and_then(|v| plays.get(v))
                .is_some_and(Option::is_none)
    })
}

/// Does an action of a transition. `remove` belongs to an entity's
/// machine, which handles it.
fn act(spec: &Spec, w: &mut World, f: &mut Frame, action: Do) {
    match action {
        // After an exit, the level last started starts again.
        Do::RestartLevel => {
            // A checkpoint the view has reached in the level the player
            // is in starts that level again from there (feature
            // `level.halfway`); otherwise the level last started starts again.
            let half = spec.levels[w.level]
                .halfway
                .filter(|(from, _)| w.cameras.first().is_some_and(|c| c[0] >= *from));
            if half.is_none() {
                w.level = stage_level(spec, w).unwrap_or(w.home_level);
                w.home_level = w.level;
            }
            start_level_at(spec, w, f, half);
        }
        Do::NextLevel => {
            if let Some(l) = stage_level(spec, w) {
                // With `stages`, the rules have stepped the stage counter
                // as the game does; its stage's level starts.
                w.level = l;
                w.home_level = l;
                start_level(spec, w, f);
            } else if let Some(next) = spec.levels[w.level].next {
                w.level = next;
                w.home_level = next;
                start_level(spec, w, f);
            }
        }
        Do::EnterLevel => {
            // `next-level`, then every counter and clock back as it was,
            // and each player's words.
            let (counters, ticks) = (w.counters.clone(), w.ticks.clone());
            let words: Vec<(usize, [i64; 4])> = w
                .bodies
                .iter()
                .filter(|b| spec.kinds[b.kind].role == Role::Player)
                .map(|b| (b.kind, b.ext))
                .collect();
            act(spec, w, f, Do::NextLevel);
            f.level_entered = f.level_started;
            w.counters = counters;
            w.ticks = ticks;
            for (kind, ext) in words {
                if let Some(b) = w.bodies.iter_mut().find(|b| b.kind == kind) {
                    b.ext = ext;
                }
            }
        }
        Do::ResetGame => {
            w.counters = spec.counters.iter().map(|c| c.start).collect();
            w.ticks.iter_mut().for_each(|t| *t = 0);
            w.level = spec.start_level;
            w.home_level = spec.start_level;
            start_level(spec, w, f);
        }
        Do::KillPlayer => {
            let player = w
                .bodies
                .iter()
                .position(|b| spec.kinds[b.kind].role == Role::Player);
            if let Some(p) = player {
                f.out[p] = true;
            }
            emit(spec, w, f, EventKind::PLAYER_DIED, player);
        }
        Do::Change(c, by) => change_counter(spec, w, f, c, by),
        Do::Set(c, v) => {
            let k = &spec.counters[c];
            let lo = k.min.unwrap_or(i64::MIN);
            let hi = k.max.unwrap_or(i64::MAX);
            w.counters[c] = v.clamp(lo, hi);
            for (j, h) in spec.counters.iter().enumerate() {
                if h.high_of == Some(c) && w.counters[j] < w.counters[c] {
                    w.counters[j] = w.counters[c];
                }
            }
        }
        Do::Add(to, from) => {
            let by = w.counters[from];
            change_counter(spec, w, f, to, by);
        }
        Do::Emit(e) => emit(spec, w, f, e, None),
        Do::Clear(kind) => {
            for b in w.bodies.iter_mut().filter(|b| b.kind == kind && b.alive) {
                b.alive = false;
            }
        }
        Do::Respawn(kind) => {
            // With `stages`, the spawns of the level that plays the stage
            // the counter names: after a goal has stepped the stage, the
            // next stage's places, which Donkey Kong's setup draws 11
            // divider wraps before that stage starts ($CADE). Without
            // `stages`, or within a stage, the level being played.
            let level = stage_level(spec, w).unwrap_or(w.level);
            let spawns = &spec.levels[level].spawns;
            for (i, b) in w.bodies.iter_mut().enumerate() {
                if b.kind != kind || !b.alive {
                    continue;
                }
                if let Some(sp) = spawns.get(i).filter(|sp| sp.kind == kind) {
                    b.x = sp.x;
                    b.y = sp.y;
                    b.vx = Fx::ZERO;
                    b.vy = Fx::ZERO;
                }
            }
        }
        Do::RemoveBody(c) => {
            let id = w.counters[c];
            if let Some(b) = usize::try_from(id - 1)
                .ok()
                .and_then(|i| w.bodies.get_mut(i))
            {
                b.alive = false;
            }
            w.counters[c] = 0;
        }
        Do::Remove | Do::Become(_) => {}
    }
}

/// Exits: the first exit of the level, in order, whose `take` the player
/// meets (its hitbox overlapping the box, the action held, standing if it
/// must be) is taken, as [`take_exit`] says.
pub fn exits(spec: &Spec, w: &mut World, f: &mut Frame) {
    let Some(p) = w
        .bodies
        .iter()
        .position(|b| spec.kinds[b.kind].role == Role::Player && b.active())
    else {
        return;
    };
    if f.out[p] {
        return;
    }
    let b = w.bodies[p];
    let hb = spec.kind(&b).hitbox;
    let (x0, y0) = (b.x.add(hb.x), b.y.add(hb.y));
    let (x1, y1) = (x0.add(hb.w), y0.add(hb.h));
    let level = &spec.levels[w.level];
    let taken = level.exits.iter().find(|e| {
        let a = e.at;
        x0 < a.x.add(a.w)
            && a.x < x1
            && y0 < a.y.add(a.h)
            && a.y < y1
            && e.hold.is_none_or(|h| w.held(spec, h))
            && (!e.standing || b.on_ground)
            && e.guard
                .is_none_or(|g| g.cmp.holds(w.counters[g.counter], g.value))
    });
    if let Some(e) = taken.cloned() {
        take_exit(spec, w, f, p, &e);
    }
}

/// Takes an exit: the level becomes its `to`, placed as at a level start,
/// with the player keeping its own state and put at `enter` (or its spawn
/// there) with no speed, and the cameras placed on it as at a start.
/// Counters keep their values and `level.started` doesn't fire. The rest
/// of the frame doesn't move, like a level start's.
pub fn take_exit(spec: &Spec, w: &mut World, f: &mut Frame, p: usize, e: &crate::spec::Exit) {
    // An exit that starts its level does what `next-level` does.
    if e.start {
        w.level = e.to;
        w.home_level = e.to;
        start_level(spec, w, f);
        return;
    }
    let old = w.bodies[p];
    let old_view = w.cameras.first().copied().unwrap_or([Fx::ZERO; 2]);
    w.level = e.to;
    w.place(spec);
    if let Some(np) = w
        .bodies
        .iter()
        .position(|b| spec.kinds[b.kind].role == Role::Player)
    {
        let mut nb = Body {
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            air: 0,
            buffer: 0,
            band: 0,
            held_phase: false,
            riding: 0,
            carrying: 0,
            lift: 0,
            ..old
        };
        if let Some([x, y]) = e.enter {
            nb.x = x;
            nb.y = y;
        } else {
            nb.x = w.bodies[np].x;
            nb.y = w.bodies[np].y;
        }
        w.bodies[np] = nb;
        for i in 0..spec.cameras.len() {
            place_camera(spec, w, i, true);
        }
        // A kept coordinate keeps its place in the view: the old view's
        // corner comes off, the new one's goes on, and the cameras are
        // placed again on where that puts the player.
        if let Some(axis) = e.keep {
            let new_view = w.cameras.first().copied().unwrap_or([Fx::ZERO; 2]);
            let at = [old.x, old.y][axis].sub(old_view[axis]).add(new_view[axis]);
            if axis == 0 {
                nb.x = at;
            } else {
                nb.y = at;
            }
            w.bodies[np] = nb;
            for i in 0..spec.cameras.len() {
                place_camera(spec, w, i, true);
            }
        }
        nb.home = [nb.x, nb.y];
        let k = spec.kind(&nb);
        // Placed through an exit, its falls take the start's gravity again,
        // as Super Mario Bros.' Entrance_GameTimerSetup sets
        // VerticalForceDown on every area's entrance.
        nb.fall_gravity = crate::world::start_fall_gravity(k);
        nb.on_ground = k.tile_collision && ground_under_t(w.tiles(spec), k, &nb);
        w.bodies[np] = nb;
    }
    w.scroll = e.scroll;
    f.out = vec![false; w.bodies.len()];
    f.fresh_bodies = vec![true; w.bodies.len()];
    f.level_started = true;
    start_draw(spec, w);
    appear(spec, w, f);
}

/// Starts the current level: entities at their spawns, cameras at their
/// starts, the level's counter values, then `level.started`.
pub fn start_level(spec: &Spec, w: &mut World, f: &mut Frame) {
    start_level_at(spec, w, f, None);
}

/// [`start_level`], from a checkpoint when `half` gives one: the first
/// camera at its x and the player at its place.
fn start_level_at(spec: &Spec, w: &mut World, f: &mut Frame, half: Option<(Fx, [Fx; 2])>) {
    // `run.face_way`: a restart keeps the way the player faced.
    let kept: Vec<(usize, bool)> = w
        .bodies
        .iter()
        .filter(|b| spec.kinds[b.kind].role == Role::Player)
        .filter(|b| spec.kinds[b.kind].run.as_ref().is_some_and(|r| r.face_way))
        .map(|b| (b.kind, b.facing_left))
        .collect();
    w.place(spec);
    for (kind, left) in kept {
        if let Some(b) = w.bodies.iter_mut().find(|b| b.kind == kind) {
            b.facing_left = left;
        }
    }
    if let Some((from, [x, y])) = half {
        if let Some(p) = w
            .bodies
            .iter()
            .position(|b| spec.kinds[b.kind].role == Role::Player)
        {
            let b = &mut w.bodies[p];
            b.x = x;
            b.y = y;
            b.home = [x, y];
            let nb = *b;
            let k = spec.kind(&nb);
            w.bodies[p].on_ground = k.tile_collision && ground_under_t(w.tiles(spec), k, &nb);
        }
        for i in 0..spec.cameras.len() {
            place_camera(spec, w, i, true);
        }
        if let Some(c) = w.cameras.first_mut() {
            c[0] = from;
        }
        if !spec.cameras.is_empty() {
            see(spec, w, 0);
        }
        start_draw(spec, w);
    }
    for &(c, v) in &spec.levels[w.level].counters {
        w.counters[c] = v;
        // Its clocks start over (feature `counter.restarts_clock`).
        if spec.counters[c].restarts_clock
            && let Some(first) = clock_of(spec, c)
        {
            let n = spec.counters[c]
                .changes
                .iter()
                .filter(|ch| matches!(ch, Change::Every(..)))
                .count();
            for k in &mut w.ticks[first..first + n] {
                *k = 0;
            }
        }
    }
    // A level's launchers fill their entries from the first
    // (InitializeArea clears Cannon_Offset).
    for (_, next, _, _) in spec.across().launchers {
        w.counters[next] = 0;
    }
    f.out = vec![false; w.bodies.len()];
    // Every instance enters its initial state now: this frame doesn't
    // count toward its `after`.
    f.fresh_bodies = vec![true; w.bodies.len()];
    f.level_started = true;
    appear(spec, w, f);
    emit(spec, w, f, EventKind::LEVEL_STARTED, None);
}

/// The player: runs, jumps and falls against tiles, then dies in a pit or
/// reaches the goal ([`run`], [`fall`], [`bounds`]), for each player.
pub fn player(spec: &Spec, w: &mut World, f: &mut Frame) {
    player_ext(spec, w, f, &Exts::new());
}

/// [`player`], then each player's `ext` hook. A player held by a camera
/// that scrolls in steps doesn't move. One lifting what it carries waits
/// out the lift. Otherwise it shoots, climbs, or is knocked back or runs,
/// then jumps and falls (not while a springboard holds it, and on a late
/// throw's frame it only leaves the ground), meets hazards, pits and the goal, lifts or throws,
/// and the carried entity follows it.
pub fn player_ext(spec: &Spec, w: &mut World, f: &mut Frame, exts: &Exts) {
    // Core `entity.hook_first`: those hooks run first, in body order.
    for i in 0..w.bodies.len() {
        let k = spec.kind(&w.bodies[i]);
        if k.hook_first && k.role != Role::Player {
            run_ext(spec, w, f, i, exts);
        }
    }
    for i in 0..w.bodies.len() {
        if f.level_started {
            break;
        }
        if spec.kinds[w.bodies[i].kind].role != Role::Player {
            continue;
        }
        if w.hold {
            continue;
        }
        let b = &mut w.bodies[i];
        b.invuln = b.invuln.saturating_sub(1);
        if b.lift > 0 {
            b.lift -= 1;
        } else {
            // The jump's bands read the speed as the frame starts, or with
            // `run.speed_before_walls` as the run last left it.
            let before_walls = spec
                .kind(&w.bodies[i])
                .run
                .as_ref()
                .is_some_and(|r| r.speed_before_walls);
            let takeoff_speed = if before_walls {
                w.bodies[i].seen_speed
            } else {
                w.bodies[i].vx
            };
            let from = w.bodies[i].y;
            let from_x = w.bodies[i].x;
            // In a jump, or its landing hold, as the frame starts.
            let from_jump = w.bodies[i].air == u32::MAX
                || spec
                    .kind(&w.bodies[i])
                    .jump
                    .as_ref()
                    .is_some_and(|j| j.land_hold > 0 && w.bodies[i].ext[LAND_HOLD_WORD] > 0);
            // Platformer `level.pickup_holes`: a hole touched last frame
            // ([`pickups`] left the body standing with `air` 1) starts its
            // fall off a ledge now.
            if w.bodies[i].on_ground
                && w.bodies[i].air == 1
                && spec.levels[w.level].pickups.iter().any(|g| !g.by_hook)
            {
                w.bodies[i].on_ground = false;
            }
            // A springboard holding it, or its throw's late frame
            // (crate::springboard).
            let held = crate::springboard::holder(spec, w, i).is_some();
            let late = !held && crate::springboard::thrown_late(spec, w, i);
            shoot_at(spec, w, f, i, false);
            if !climb(spec, w, f, i) {
                if w.bodies[i].knock > 0 {
                    knockback(spec, w, i);
                } else if down_holds(spec, w, i) {
                    w.bodies[i].vx = Fx::ZERO;
                } else if ledge_falling(spec, w, i) {
                    // A fall off a ledge moves nothing across and takes no
                    // input; the step counter steps.
                    let mut b = w.bodies[i];
                    b.vx = Fx::ZERO;
                    if let Some(r) = &spec.kind(&b).run
                        && !r.step_cycle.is_empty()
                    {
                        cycle_step(&mut b, &r.step_cycle);
                    }
                    w.bodies[i] = b;
                } else {
                    run(spec, w, f, i);
                }
                if late {
                    crate::springboard::leave_late(w, i);
                } else if !held {
                    fall(spec, w, f, i, takeoff_speed);
                }
                pickup(spec, w, f, i);
                walls(spec, w, f, i);
            }
            hazard(spec, w, f, i);
            bounds_from(spec, w, f, i, Some(from));
            if f.level_started {
                break;
            }
            pickups(spec, w, f, i, from_x, from_jump);
            shoot_at(spec, w, f, i, true);
            carry(spec, w, f, i);
        }
        follow_carrier(spec, w, i);
        run_ext(spec, w, f, i, exts);
    }
    // Feature `camera.keep_in_view_rows`: a row's bounds are a wall, so
    // they hold the player as its system ends, before the enemies and
    // contacts test its place, as Donkey Kong's $D990 refuses the walk's
    // move before the barrels test Mario ($EC44).
    if !f.level_started {
        let level_rows = spec.levels[w.level]
            .camera
            .as_ref()
            .is_some_and(|c| !c.keep_in_view_rows.is_empty());
        for c in 0..spec.cameras.len() {
            if level_rows || !spec.cameras[c].keep_in_view_rows.is_empty() {
                keep_in_view(spec, w, c);
            }
        }
    }
}

/// Top-down 0.1's `topdown.player`: each player with `move8` moves
/// `speed` across while left or right is held, then `speed` down or up
/// while down or up is held. Solid tiles stop each axis on its own, so a
/// diagonal move into a wall slides along it. No gravity.
pub fn topdown_player(spec: &Spec, w: &mut World, f: &mut Frame) {
    for i in 0..w.bodies.len() {
        if f.level_started {
            break;
        }
        let kind = spec.kind(&w.bodies[i]);
        if kind.role != Role::Player {
            continue;
        }
        let Some(m) = kind.move8 else {
            continue;
        };
        let along = |neg: usize, pos: usize| -> Fx {
            match (w.held(spec, neg), w.held(spec, pos)) {
                (true, false) => Fx::ZERO.sub(m.speed),
                (false, true) => m.speed,
                _ => Fx::ZERO,
            }
        };
        let (dx, dy) = (along(m.left, m.right), along(m.up, m.down));
        if dx != Fx::ZERO {
            w.bodies[i].facing_left = dx < Fx::ZERO;
        }
        let tiles = w.tiles(spec);
        let mut b = w.bodies[i];
        move_x(spec, tiles, &mut b, dx);
        move_y(spec, tiles, &mut b, dy);
        b.vx = dx;
        b.vy = dy;
        w.bodies[i] = b;
    }
}

/// Moves a body `dy` down (or up), one pixel at a time, stopping at the
/// first row its tile hitbox would put in a solid tile, and inside the
/// level. The vertical twin of [`move_x`], for top-down movement.
fn move_y(spec: &Spec, tiles: Tiles<'_>, b: &mut Body, dy: Fx) {
    let kind = spec.kind(b);
    let level = tiles.level;
    let target = b.y.add(dy);
    if !kind.tile_collision {
        b.y = target;
        return;
    }
    let max_y = level.pixels()[1] - kind.size[1].floor();
    let (mut py, to) = (b.y.floor(), target.floor());
    while py != to {
        let next = if to > py { py + 1 } else { py - 1 };
        let probe = Body {
            y: Fx::from_int(next),
            ..*b
        };
        let edge = if to > py {
            next + kind.tile_hitbox.y.floor() + kind.tile_hitbox.h.floor() - 1
        } else {
            next + kind.tile_hitbox.y.floor()
        };
        if next < 0 || next > max_y || crate::world::row_solid_t(tiles, kind, &probe, edge) {
            b.y = Fx::from_int(py);
            b.vy = Fx::ZERO;
            return;
        }
        py = next;
    }
    b.y = target;
}

/// Knockback: the body moves `speed` against the way it faces, its run
/// input ignored; on the first frame its vertical speed becomes `rise`.
fn knockback(spec: &Spec, w: &mut World, i: usize) {
    let mut b = w.bodies[i];
    let Some(k) = spec.kind(&b).knockback.clone() else {
        w.bodies[i].knock = 0;
        return;
    };
    if b.knock == k.frames {
        b.vy = k.rise;
        b.on_ground = false;
    }
    b.knock -= 1;
    b.vx = if b.facing_left {
        k.speed
    } else {
        k.speed.neg()
    };
    let dx = b.vx;
    move_x(spec, w.tiles(spec), &mut b, dx);
    w.bodies[i] = b;
}

/// Climbing: returns true while the body is on a ladder this frame, when
/// it neither runs nor falls. `up` with the hitbox's middle on a ladder,
/// or `down` standing on a ladder, starts a climb at the ladder's middle;
/// then `up` and `down` move it, and it lets go when `release` is pressed,
/// when its middle leaves the ladder, or when climbing down meets ground.
/// Climbing up off a ladder's top puts it standing there, with
/// `stand_on_top`.
/// Whether an action is taken away from this entity right now (feature
/// `power.locks`: while its guard holds, the listed actions do nothing).
fn locked(kind: &crate::spec::Kind, w: &World, action: usize) -> bool {
    kind.locks.as_ref().is_some_and(|(g, acts)| {
        acts.contains(&action) && g.cmp.holds(w.counters[g.counter], g.value)
    })
}

/// Up and down as a climb of `kind` reads them this frame: held, and with
/// `climb.lone_way` held alone, no other direction with it (Donkey Kong's
/// $D175 latches $56 only for a lone direction). Hooks that move a climb
/// read them here too.
pub fn climb_ways(spec: &Spec, w: &World, kind: &crate::spec::Kind) -> [bool; 2] {
    let Some(cl) = &kind.climb else {
        return [false, false];
    };
    let (up, down) = (w.held(spec, cl.up), w.held(spec, cl.down));
    if !cl.lone_way {
        return [up, down];
    }
    let (right, left) = kind.run.as_ref().map_or((false, false), |r| {
        (w.held(spec, r.right), w.held(spec, r.left))
    });
    let ways = [right, left, up, down].iter().filter(|h| **h).count();
    [up && ways == 1, down && ways == 1]
}

pub fn climb(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) -> bool {
    let mut b = w.bodies[i];
    let kind = spec.kind(&b);
    let Some(cl) = kind.climb.clone() else {
        return false;
    };
    if !b.active() || f.out[i] || b.knock > 0 {
        w.bodies[i].climbing = false;
        return false;
    }
    let t = w.tiles(spec);
    let ts = t.level.tile_size;
    let h = kind.tile_hitbox;
    let (cx, cy) = middle(kind, &b);
    let bottom = b.y.floor() + h.y.floor() + h.h.floor() - 1;
    let cycled = !cl.up_cycle.is_empty() || !cl.down_cycle.is_empty();
    // Platformer `climb.up_cycle` and `down_cycle`: the whole pixels this
    // frame's count moves, the count stepped first; without a cycle the
    // fixed speed.
    let step = |b: &mut Body, cycle: &[i64]| {
        if cycle.is_empty() {
            cl.speed
        } else {
            Fx::from_int(cycle_step(b, cycle))
        }
    };
    let snap = |b: &mut Body, col_x: i64| {
        let center = col_x.div_euclid(ts) * ts + ts / 2;
        b.x = Fx::from_int(center - kind.hitbox.x.floor() - kind.hitbox.w.floor() / 2);
        b.vx = Fx::ZERO;
        b.vy = Fx::ZERO;
        b.on_ground = false;
        b.climbing = true;
        b.steps = 0;
    };
    // A jump's landing hold (`jump.land_hold`) takes no ladder either: the
    // body is still in its jump (Donkey Kong's $94 $F0-$F3, in state 4).
    let holding = kind
        .jump
        .as_ref()
        .is_some_and(|j| j.land_hold > 0 && b.ext[LAND_HOLD_WORD] > 0);
    if !b.climbing && holding {
        return false;
    }
    let (right, left) = kind.run.as_ref().map_or((false, false), |r| {
        (w.held(spec, r.right), w.held(spec, r.left))
    });
    let (up_held, down_held) = (w.held(spec, cl.up), w.held(spec, cl.down));
    let [up_alone, down_alone] = climb_ways(spec, w, kind);
    let turn = cl.lone_way && !up_held && !down_held && (right || left);
    let turn_left = left && (!right || kind.run.as_ref().is_some_and(|r| r.both_left));
    if !b.climbing {
        // `climb.grab` in a level with ladders: up starts where the grab
        // box meets a ladder's box, edges included, below the box's top
        // row (standing on a ladder's top, up does nothing). Down keeps
        // its own test.
        let ladders = &spec.levels[w.level].ladders;
        let grab_box = cl.grab.filter(|_| !ladders.is_empty()).map(|g| {
            let (x, y) = (b.x.floor(), b.y.floor());
            [x + g[0], y + g[1], x + g[2], y + g[3]]
        });
        let ladder_at = |g: [i64; 4]| {
            ladders
                .iter()
                .find(|l| g[2] >= l[0] && l[2] >= g[0] && g[3] > l[1] && l[3] >= g[1])
                .copied()
        };
        let grabbed = grab_box.and_then(ladder_at);
        // Platformer `climb.top_test`: on the ground (unless the last move
        // across stepped up, with `down_through_step`), the grab box asks
        // the level's tops and stops: down needs a top and no stop, and up
        // refuses either.
        let level = &spec.levels[w.level];
        let test = grab_box.filter(|_| {
            cl.top_test
                && !level.ladder_tops.is_empty()
                && b.on_ground
                && !(cl.down_through_step && b.stepped_up)
        });
        let asked = test.map(|g| top_test(level, g));
        let up = up_alone
            && !locked(kind, w, cl.up)
            && asked.is_none_or(|r| r == TopTest::Off)
            && match grab_box {
                Some(_) => b.on_ground && grabbed.is_some(),
                None => t.at(cx, cy) == Tile::Ladder,
            };
        // With `top_test`, down takes a ladder of the level's that the grab
        // box meets, edges included (the walk's test of the ladder list,
        // Donkey Kong's $D28B, before $D50A's).
        let down_ladder = grab_box.filter(|_| cl.top_test).and_then(|g| {
            ladders
                .iter()
                .find(|l| g[2] >= l[0] && l[2] >= g[0] && g[3] >= l[1] && l[3] >= g[1])
                .copied()
        });
        let down = !up
            && down_alone
            && !locked(kind, w, cl.down)
            && b.on_ground
            && (!cl.top_test || grab_box.is_none() || down_ladder.is_some())
            && if let Some(r) = asked {
                // Where the test finds a top.
                r == TopTest::Top
            } else if cl.by_feet {
                // From a surface with a ladder in the cell under it, or,
                // with `down_through`, down newly pressed with the feet in a
                // ladder cell (a ladder's foot).
                t.at(cx, t.cell_top(bottom + 1) + ts) == Tile::Ladder
                    || (cl.down_through > 0
                        && w.pressed(spec, cl.down)
                        && (!cl.down_through_step || b.stepped_up)
                        && t.at(cx, bottom + 1) == Tile::Ladder)
            } else {
                t.at(cx, bottom + 1) == Tile::Ladder
            };
        // With `grab_snap`, a climb by a ladder box starts at the box's left
        // plus this, as the game places Mario by the ladder's x: up by the
        // box it grabbed, down (with `top_test`) by the one it took.
        let snap_to = |b: &mut Body| {
            let by = if up { grabbed } else { down_ladder };
            if let (Some(l), Some(s)) = (by, cl.grab_snap) {
                b.x = Fx::from_int(l[0] + s);
            }
        };
        if !up && !down {
            return false;
        }
        if cycled {
            // The count steps on each frame the climb is asked for, and
            // the climb starts on a count that moves.
            let cycle = if up { &cl.up_cycle } else { &cl.down_cycle };
            let d = step(&mut b, cycle);
            if d == Fx::ZERO {
                w.bodies[i] = b;
                return false;
            }
            snap(&mut b, cx);
            snap_to(&mut b);
            climb_move(&cl, t, kind, &mut b, if up { d.neg() } else { d });
        } else {
            snap(&mut b, cx);
            snap_to(&mut b);
            if down {
                b.y = b.y.add(cl.grab_down);
            }
        }
        w.bodies[i] = b;
        return true;
    }
    if cl.release.is_some_and(|a| w.pressed(spec, a)) {
        w.bodies[i].climbing = false;
        return false;
    }
    b.vx = Fx::ZERO;
    b.vy = Fx::ZERO;
    b.on_ground = false;
    if cl.hold_still.is_some_and(|a| w.held(spec, a)) {
        w.bodies[i] = b;
        return true;
    }
    if up_alone {
        let d = step(&mut b, &cl.up_cycle);
        // With `climb.top_test`, a move up where the grab box meets a stop
        // isn't made (the count still steps).
        let level = &spec.levels[w.level];
        let stopped = cl.top_test
            && cl.grab.is_some_and(|g| {
                let (x, y) = (b.x.floor(), b.y.floor());
                top_test(level, [x + g[0], y + g[1], x + g[2], y + g[3]]) == TopTest::Stop
            });
        if stopped {
            w.bodies[i] = b;
            return true;
        }
        climb_move(&cl, t, kind, &mut b, d.neg());
    } else if down_alone {
        let d = step(&mut b, &cl.down_cycle);
        climb_move(&cl, t, kind, &mut b, d);
    } else if turn && (right != left || turn_left) {
        b.facing_left = turn_left;
    }
    w.bodies[i] = b;
    true
}

/// What platformer `climb.top_test` finds at a grab box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopTest {
    Off,
    Top,
    Stop,
}

/// Platformer `climb.top_test`: whether the box `g` (edges included)
/// meets one of the level's stops, else one of its tops (Donkey Kong's
/// $D50A: 2, 1 or 0).
fn top_test(level: &crate::spec::Level, g: [i64; 4]) -> TopTest {
    let meets = |l: &[i64; 4]| g[2] >= l[0] && l[2] >= g[0] && g[3] >= l[1] && l[3] >= g[1];
    if level.ladder_stops.iter().any(meets) {
        TopTest::Stop
    } else if level.ladder_tops.iter().any(meets) {
        TopTest::Top
    } else {
        TopTest::Off
    }
}

/// Platformer `jump.ledge_fall`: the body is in the air from an earlier
/// frame after walking off a ledge, not a jump.
fn ledge_falling(spec: &Spec, w: &World, i: usize) -> bool {
    let b = &w.bodies[i];
    spec.kind(b)
        .jump
        .as_ref()
        .is_some_and(|j| j.ledge_fall.is_some())
        && !b.on_ground
        && !b.climbing
        && b.air != u32::MAX
        && b.air >= 1
}

/// Platformer `climb.down_holds`: the body stands, isn't climbing and
/// holds its climb's `down`, so it doesn't walk this frame. A jump comes
/// first: a frame that takes off, or a jump's landing hold, isn't held,
/// so the run steps its counter as on any jump frame (Donkey Kong's pad
/// routine sets the jump state at $D195 before the walk routine looks for
/// a ladder down, and the jump routine steps $88 on every frame of the
/// jump and its landing hold; dk-tas-zeros2, original frame 2985, Down
/// and A after Up).
fn down_holds(spec: &Spec, w: &World, i: usize) -> bool {
    let b = &w.bodies[i];
    let kind = spec.kind(b);
    let jumping = kind.jump.as_ref().is_some_and(|j| {
        (j.land_hold > 0 && b.ext[LAND_HOLD_WORD] > 0)
            || (w.pressed(spec, j.action) && !locked(kind, w, j.action))
    });
    kind.climb.as_ref().is_some_and(|cl| {
        cl.down_holds && b.on_ground && !b.climbing && !jumping && w.held(spec, cl.down)
    })
}

/// A climbing body moves `dy` (negative is up) and lets go where the
/// ladder ends. With `by_feet` the ladder is the row under its tile
/// hitbox: it moves only while that row stays in a ladder or a cell with
/// a surface, and stands once it is on a surface row. Without it, the
/// middle leaving the ladder lets go, standing on the ladder's top with
/// `stand_on_top`, and climbing down onto solid ground lets go.
fn climb_move(
    cl: &crate::spec::Climb,
    t: Tiles<'_>,
    kind: &crate::spec::Kind,
    b: &mut Body,
    dy: Fx,
) {
    let h = kind.tile_hitbox;
    if cl.by_feet {
        let (cx, _) = middle(kind, b);
        let surface_row = |y: i64| match t.at(cx, y) {
            Tile::Top => Some(t.cell_top(y) + t.level.surface(cx, y).unwrap_or(0)),
            Tile::Ladder => t.level.surface(cx, y).map(|s| t.cell_top(y) + s),
            k if k.solid() => Some(t.cell_top(y)),
            _ => None,
        };
        let holds = |y: i64| t.at(cx, y) == Tile::Ladder || surface_row(y).is_some();
        let foot = |b: &Body| b.y.floor() + h.y.floor() + h.h.floor();
        let mut moved = *b;
        moved.y = b.y.add(dy);
        // Up needs the ladder under the feet before the move, down after;
        // with `down_through` down needs nothing, and the height wraps.
        let ok = if dy < Fx::ZERO {
            holds(foot(b))
        } else {
            cl.down_through > 0 || holds(foot(&moved))
        };
        if dy == Fx::ZERO || !ok {
            return;
        }
        if cl.down_through > 0 && moved.y.floor() >= cl.down_through {
            moved.y = moved.y.sub(Fx::from_int(cl.down_through));
        }
        *b = moved;
        let f = foot(b);
        if surface_row(f) == Some(f) {
            b.climbing = false;
            b.on_ground = true;
        }
        return;
    }
    b.y = b.y.add(dy);
    let (cx, cy) = middle(kind, b);
    if dy < Fx::ZERO {
        if t.at(cx, cy) != Tile::Ladder {
            b.climbing = false;
            // Off the top: standing on the ladder's top.
            let bottom = b.y.floor() + h.y.floor() + h.h.floor();
            let top = t.cell_top(bottom);
            if cl.stand_on_top && t.at(cx, bottom) == Tile::Ladder {
                b.y = Fx::from_int(top - h.y.floor() - h.h.floor());
                b.on_ground = true;
            }
        }
    } else if dy > Fx::ZERO {
        let row = b.y.floor() + h.y.floor() + h.h.floor();
        if t.at(cx, cy) != Tile::Ladder || row_solid_t(t, kind, b, row - 1) {
            b.climbing = false;
        }
    }
}

/// Hazard tiles: a player whose hitbox touches one is killed or hurt, as
/// the level's `hazard` says.
fn hazard(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) {
    let Some((touch, damage)) = spec.levels[w.level].hazard else {
        return;
    };
    let b = w.bodies[i];
    if !b.active() || f.out[i] || !touches_tile(w.tiles(spec), spec.kind(&b), &b, Tile::Hazard) {
        return;
    }
    match touch {
        Touch::KillPlayer => {
            f.out[i] = true;
            emit(spec, w, f, EventKind::PLAYER_DIED, Some(i));
        }
        Touch::HurtPlayer => hurt_player(spec, w, f, i, damage, None),
        _ => {}
    }
}

/// A hurt to player `p` from `by` (an entity, or a tile): unless it's
/// invulnerable, its health counter loses `damage`, its invulnerability
/// and knockback start, and `player.hurt` fires, then `enemy.hurt_player`
/// for the toucher. The player touches nothing more this frame.
pub fn hurt_player(
    spec: &Spec,
    w: &mut World,
    f: &mut Frame,
    p: usize,
    damage: i64,
    by: Option<usize>,
) {
    f.out[p] = true;
    let b = w.bodies[p];
    let kind = spec.kind(&b);
    if b.invuln > 0 {
        return;
    }
    if let Some(h) = kind.health.clone() {
        w.bodies[p].invuln = h.invulnerable;
        if let Some(k) = &kind.knockback {
            w.bodies[p].knock = k.frames;
            w.bodies[p].climbing = false;
        }
        if let Some(c) = h.counter {
            change_counter(spec, w, f, c, -damage);
        }
    } else if let Some(k) = &kind.knockback {
        w.bodies[p].knock = k.frames;
        w.bodies[p].climbing = false;
    }
    if p < w.bodies.len() && !f.level_started {
        emit(spec, w, f, EventKind::PLAYER_HURT, Some(p));
    }
    if let Some(e) = by
        && e < w.bodies.len()
        && !f.level_started
    {
        emit(spec, w, f, EventKind::ENEMY_HURT_PLAYER, Some(e));
    }
}

/// Shooting: a body with `shoot` spawns its entity when the action is
/// newly pressed, or every `every` frames, while fewer than `max` of it
/// are active. The new body is `ahead` from this one (x mirrored when it
/// faces left), facing the same way.
pub fn shoot(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) {
    let b = w.bodies[i];
    let Some(sh) = spec.kind(&b).shoot.clone() else {
        return;
    };
    shoot_with(spec, w, f, i, &sh);
}

/// A player's [`shoot`], before it moves this frame or, with
/// `shoot.after_move` (platformer 0.4.10), after.
fn shoot_at(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, after: bool) {
    let b = w.bodies[i];
    let Some(sh) = spec.kind(&b).shoot.clone() else {
        return;
    };
    if sh.after_move == after {
        shoot_with(spec, w, f, i, &sh);
    }
}

fn shoot_with(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, sh: &crate::spec::Shoot) {
    let b = w.bodies[i];
    if !b.active() {
        return;
    }
    let due = match sh.action {
        Some(a) => w.pressed(spec, a),
        None => {
            let n = w.bodies[i].shot + 1;
            let due = n >= sh.every;
            w.bodies[i].shot = if due { 0 } else { n };
            due
        }
    };
    let out = w
        .bodies
        .iter()
        .filter(|o| o.active() && o.kind == sh.entity)
        .count() as u32;
    if !due || out >= sh.max {
        return;
    }
    let mut rest = None;
    if sh.in_slots {
        release_shots(w, sh.max as usize);
        let turn = w.shot_turn as usize % w.shots.len();
        if w.shots[turn].body != 0 {
            return;
        }
        rest = Some(w.shots[turn].rest);
    }
    let sh = sh.clone();
    let dx = if b.facing_left && !sh.unmirrored {
        sh.ahead[0].neg()
    } else {
        sh.ahead[0]
    };
    let (bx, by) = if sh.whole {
        (Fx::from_int(b.x.floor()), Fx::from_int(b.y.floor()))
    } else {
        (b.x, b.y)
    };
    let mut nb = new_body(
        spec,
        w,
        sh.entity,
        bx.add(dx),
        by.add(sh.ahead[1]),
        b.facing_left,
    );
    nb.vy = sh.vy;
    nb.vx = match sh.vx_both {
        Some(vx) if b.faced_both => vx,
        _ if b.facing_left => sh.vx.neg(),
        _ => sh.vx,
    };
    if let Some([x, y, vy]) = rest {
        nb.x = Fx::from_int(nb.x.floor()).add(x);
        nb.y = Fx::from_int(nb.y.floor()).add(y);
        nb.vy = Fx::from_int(nb.vy.floor()).add(vy);
        let turn = w.shot_turn as usize % w.shots.len();
        w.shots[turn].body = w.bodies.len() as u32 + 1;
        w.shot_turn = w.shot_turn.wrapping_add(1);
    }
    w.bodies.push(nb);
    f.out.push(false);
    f.fresh_bodies.push(true);
    claim_slot(spec, w, w.bodies.len() - 1);
}

/// Makes the `count` numbered shot slots if they aren't there, and frees
/// each whose shot has left, keeping the fractions of its place and its
/// speed down.
fn release_shots(w: &mut World, count: usize) {
    if w.shots.len() < count.max(1) {
        w.shots
            .resize(count.max(1), crate::world::ShotSlot::default());
    }
    for s in &mut w.shots {
        let Some(b) = (s.body as usize)
            .checked_sub(1)
            .and_then(|i| w.bodies.get(i))
        else {
            continue;
        };
        if !b.active() {
            s.rest = [Fx(b.x.frac()), Fx(b.y.frac()), Fx(b.vy.frac())];
            s.body = 0;
        }
    }
}

/// Lifting and throwing: the carry action newly pressed while carrying
/// throws, and while standing on a carryable entity lifts it.
pub fn carry(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) {
    let b = w.bodies[i];
    let Some(c) = spec.kind(&b).carry.clone() else {
        return;
    };
    if !b.active() || f.out[i] || !w.pressed(spec, c.action) {
        return;
    }
    if b.carrying > 0 {
        let item = (b.carrying - 1) as usize;
        w.bodies[i].carrying = 0;
        let Some(it) = w.bodies.get_mut(item) else {
            return;
        };
        let speed = b.vx.max(b.vx.neg());
        let [tx, ty] = match c.throw_moving {
            Some((t, above)) if speed >= above => t,
            _ => c.throw,
        };
        it.carrier = 0;
        it.vx = if b.facing_left { tx.neg() } else { tx };
        it.vy = ty;
        it.facing_left = b.facing_left;
        it.on_ground = false;
        emit(spec, w, f, EventKind::ITEM_THROWN, Some(item));
        return;
    }
    let Some(item) = b.riding.checked_sub(1).map(|r| r as usize) else {
        return;
    };
    let Some(it) = w.bodies.get(item) else { return };
    if !it.active() || it.carrier > 0 || spec.kind(it).carryable.is_none() {
        return;
    }
    w.bodies[i].carrying = item as u32 + 1;
    w.bodies[i].riding = 0;
    w.bodies[i].lift = c.lift_frames;
    w.bodies[item].carrier = i as u32 + 1;
    w.bodies[item].vx = Fx::ZERO;
    w.bodies[item].vy = Fx::ZERO;
    emit(spec, w, f, EventKind::ITEM_LIFTED, Some(item));
}

/// The entity body `i` carries rides at its offset from it, facing the
/// same way.
fn follow_carrier(spec: &Spec, w: &mut World, i: usize) {
    let b = w.bodies[i];
    let (Some(c), Some(item)) = (
        spec.kind(&b).carry.as_ref(),
        b.carrying.checked_sub(1).map(|r| r as usize),
    ) else {
        return;
    };
    let Some(it) = w.bodies.get(item).copied() else {
        return;
    };
    if !it.alive || it.carrier != i as u32 + 1 {
        w.bodies[i].carrying = 0;
        return;
    }
    let offset = spec.kind(&it).carryable.flatten().unwrap_or(c.offset);
    let it = &mut w.bodies[item];
    it.x = b.x.add(offset[0]);
    it.y = b.y.add(offset[1]);
    it.vx = Fx::ZERO;
    it.vy = Fx::ZERO;
    it.facing_left = b.facing_left;
    it.on_ground = false;
}

/// Every entity but the players: patrols and walks, then falls.
pub fn enemies(spec: &Spec, w: &mut World, f: &mut Frame) {
    enemies_ext(spec, w, f, &Exts::new());
}

/// [`enemies`], with each entity's `ext` hook after its components.
pub fn enemies_ext(spec: &Spec, w: &mut World, f: &mut Frame, exts: &Exts) {
    let held = shot_slots(spec, w);
    let bumping = spec.across().bumping;
    let before = if bumping {
        w.bodies.clone()
    } else {
        Vec::new()
    };
    for i in 0..w.bodies.len() {
        if f.level_started {
            break;
        }
        // A hitter with `hits.move_first` moved in the contact system, and
        // a launcher's shot takes its turn in [`launchers`].
        let k = spec.kind(&w.bodies[i]);
        if moves_in_contact(spec, &w.bodies[i]) || k.launched || k.launcher.is_some() {
            continue;
        }
        // A touch `in_turn` stopped the loop at its slot.
        if f.stopped_from
            .is_some_and(|from| !w.slots.iter().take(from).any(|s| s.body == i as u32 + 1))
        {
            continue;
        }
        bump(spec, w, i, &before);
        let old = w.bodies[i];
        enemy(spec, w, f, i, exts);
        carry_riders(spec, w, i, old);
        place_riders(spec, w, f, i);
        near_touch(spec, w, f, i);
        touch_after_move(spec, w, f, i);
    }
    if spec.levels[w.level].despawn_after_move && !f.level_started {
        despawn(spec, w);
    }
    if !f.level_started {
        launchers(spec, w, f, exts, &held, false);
    }
}

/// Walkers turning each other (`walk.bump`), in body `i`'s turn, before it
/// moves: it's tested against each earlier body, nearest first, as Super
/// Mario Bros.' EnemiesCollision tests a slot against the slots below it.
/// Both boxes stand where the frame started (`before`), since an earlier
/// slot's box was made ahead of its move. A touching pair turns once, so the
/// earlier body moves the new way from the next frame and this one from now.
/// The pair is forgotten when a test finds it apart (Enemy_CollisionBits).
/// A body out of `view` takes no test, and its pair stays as it was.
fn bump(spec: &Spec, w: &mut World, i: usize, before: &[Body]) {
    let bump_of = |b: &Body| {
        spec.kind(b)
            .walk
            .as_ref()
            .and_then(|k| k.bump)
            .filter(|t| t.every > 0)
    };
    let Some((me, was)) = before
        .get(i)
        .and_then(|b| Some((bump_of(&w.bodies[i])?, b)))
    else {
        return;
    };
    if !w.bodies[i].active()
        || w.bodies[i].carrier > 0
        || !w
            .frame
            .wrapping_sub(u64::from(me.phase))
            .is_multiple_of(u64::from(me.every))
    {
        return;
    }
    let left = w.cameras.first().map_or(0, |c| c[0].floor());
    let seen = |b: &Body| {
        me.view
            .is_none_or(|[lo, hi]| (lo..=hi).contains(&(b.x.floor() - left)))
    };
    if !seen(was) {
        return;
    }
    for j in (0..i).rev() {
        let other = w.bodies[j];
        if !other.active() || other.carrier > 0 || bump_of(&other).is_none() || !seen(&before[j]) {
            continue;
        }
        let [al, at, ar, ab] = hit_pixels(spec, was);
        let [bl, bt, br, bb] = hit_pixels(spec, &before[j]);
        // Each box's far edges count, as `contact.touch_edges`.
        let touch = ar + 1 >= bl && br + 1 >= al && ab + 1 >= bt && bb + 1 >= at;
        let pair = (j as u32, i as u32);
        match (touch, w.bumps.iter().position(|p| *p == pair)) {
            (true, None) => {
                w.bumps.push(pair);
                w.bodies[i].facing_left = !w.bodies[i].facing_left;
                w.bodies[j].facing_left = !w.bodies[j].facing_left;
            }
            (false, Some(n)) => {
                w.bumps.remove(n);
            }
            _ => {}
        }
    }
}

/// The numbered slots that hold a launcher's shot, or what a shot becomes
/// when defeated, as the frame starts: a launcher fires into none of them
/// this frame, even when the body goes during it. Super Mario Bros.' bills,
/// flying or falling stomped, take their turns in ProcessCannons, which
/// tests a slot's Enemy_Flag before the bill's turn erases it
/// (OffscreenBoundsCheck, EnemyGfxHandler). The defeated kind may be
/// shared, so another enemy falling defeated holds its slot here too.
fn shot_slots(spec: &Spec, w: &World) -> Vec<bool> {
    let launchers = spec.across().launchers;
    let shot = |k: usize| {
        launchers
            .iter()
            .any(|&(_, _, fires, becomes)| fires == k || becomes == Some(k))
    };
    w.slots
        .iter()
        .map(|s| {
            (s.body as usize)
                .checked_sub(1)
                .and_then(|i| w.bodies.get(i))
                .is_some_and(|b| b.active() && shot(b.kind))
        })
        .collect()
}

/// The launchers' turn, after the other enemies (platformer 0.4.28; Super
/// Mario Bros.' ProcessCannons, which GameEngine runs after its enemy
/// loop): for each launcher kind, each of its first `slots` numbered slots,
/// the last first. A slot holding a shot gives it its turn. A free one
/// reads its bits, which may pick a launcher: the pick's timer counts down,
/// or at 0 it fires, unless the world is `frozen`.
pub fn launchers(
    spec: &Spec,
    w: &mut World,
    f: &mut Frame,
    exts: &Exts,
    held: &[bool],
    frozen: bool,
) {
    // A touch this frame can stop the enemies part way: TimerControl stops
    // the shots and the firing from there on.
    let stopped = |f: &Frame| frozen || f.stopped_from.is_some() || !f.runs(System::Enemies);
    // The launcher kinds alone are read.
    for (lk, _, _, _) in spec.across().launchers {
        let Some(l) = spec.kinds[lk].launcher else {
            continue;
        };
        let level = &spec.levels[w.level];
        let mask = if level.hard {
            l.hard_mask.unwrap_or(l.mask)
        } else {
            l.mask
        };
        let bits = spec.counters[l.counter]
            .shift
            .as_ref()
            .map_or(w.counters[l.counter], |s| s.step(w.counters[l.counter]));
        for slot in (0..l.slots as usize).rev() {
            release_slots(spec, w);
            let Some(&s) = w.slots.get(slot) else {
                continue;
            };
            if s.body != 0 {
                let i = s.body as usize - 1;
                let shot = w
                    .bodies
                    .get(i)
                    .is_some_and(|b| b.active() && b.kind == l.fires);
                if shot && !stopped(f) && aim_shot(spec, w, i, &l) {
                    enemy(spec, w, f, i, exts);
                    touch_after_move(spec, w, f, i);
                }
                continue;
            }
            if held.get(slot).copied().unwrap_or(false) {
                continue;
            }
            let at = l.at.saturating_sub(l.slot_step * slot as u32);
            let pick = i64::from((bits >> at) as u32 & mask);
            if pick >= i64::from(l.table) {
                continue;
            }
            let Some(j) = (0..w.bodies.len()).find(|&j| {
                w.bodies[j].active() && w.bodies[j].kind == lk && w.bodies[j].ext[1] == pick
            }) else {
                continue;
            };
            // One that appeared this frame isn't in the table yet: Super
            // Mario Bros.' area parser runs SetupCannon after ProcessCannons.
            if f.fresh_bodies.get(j).copied().unwrap_or(false) {
                continue;
            }
            if w.bodies[j].ext[0] > 0 {
                w.bodies[j].ext[0] -= 1;
                continue;
            }
            if stopped(f) {
                continue;
            }
            w.bodies[j].ext[0] = i64::from(l.rearm);
            let x = w.bodies[j].x.add(l.offset[0]);
            let y = w.bodies[j].y.add(l.offset[1]);
            let mut b = new_body(spec, w, l.fires, x, y, false);
            // The slot's last body left the fraction of its place across.
            b.x = b.x.add(w.slots[slot].rest[0]);
            w.slots[slot].body = w.bodies.len() as u32 + 1;
            w.bodies.push(b);
            f.out.push(false);
            f.fresh_bodies.push(true);
        }
    }
}

/// A launcher's shot on its first turn (mechanic word 0 still 0): it goes
/// if it is past the view's right edge, `behind` pixels or more left of the
/// left edge, or within `near` of the player, as BulletBillHandler erases
/// one; otherwise it faces the player's side and is aimed from then on.
/// The near test is on the whole-pixel difference modulo `near_wrap`, with
/// 1 added at or right of the player: BulletBillHandler adds $28 to the
/// difference's low byte with PlayerEnemyDiff's carry and compares with
/// $50. Returns whether it stays.
fn aim_shot(spec: &Spec, w: &mut World, i: usize, l: &crate::spec::Launcher) -> bool {
    if w.bodies[i].ext[0] != 0 {
        return true;
    }
    w.bodies[i].ext[0] = 1;
    let x = w.bodies[i].x.floor();
    let (left, width) = view(spec, w);
    let left = left.floor();
    let off = x > left + width.floor() - 1 || left - x >= i64::from(l.behind);
    let player = w
        .bodies
        .iter()
        .find(|b| spec.kinds[b.kind].role == Role::Player)
        .map_or(x, |b| b.x.floor());
    let d = x - player;
    let near = i64::from(l.near);
    let carry = i64::from(d >= 0);
    let close = match l.near_wrap {
        0 => (d + near + carry) >= 0 && d + near + carry < 2 * near,
        wrap => (d + near + carry).rem_euclid(i64::from(wrap)) < 2 * near,
    };
    if off || close {
        w.bodies[i].alive = false;
        return false;
    }
    w.bodies[i].facing_left = d >= 0;
    true
}

/// A launcher that appears takes the entry its `next` counter names, and
/// `next` moves on, as SetupCannon fills Cannon_Offset's entry and steps it.
/// An entry that holds a launcher of its kind stays one body: that body
/// moves to the new place with its timer and the appearing body ends, as the
/// table's entry is written over. One placed left of `from_x` takes its entry
/// and fires nothing (Cannon_PageLoc 0 reads as no cannon), so it ends too.
fn take_entry(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) {
    let Some(l) = spec.kind(&w.bodies[i]).launcher else {
        return;
    };
    let table = i64::from(l.table.max(1));
    let entry = w.counters[l.next].rem_euclid(table);
    let kind = w.bodies[i].kind;
    let held = (0..w.bodies.len()).find(|&j| {
        let b = &w.bodies[j];
        j != i && b.active() && b.kind == kind && b.ext[1] == entry
    });
    let mut at = i;
    let mut timer = 0;
    if let Some(j) = held {
        timer = w.bodies[j].ext[0];
        w.bodies[j] = w.bodies[i];
        w.bodies[i].alive = false;
        if let Some(fr) = f.fresh_bodies.get_mut(j) {
            *fr = true;
        }
        at = j;
    }
    w.bodies[at].ext = [timer, entry, 0, 0];
    w.counters[l.next] = (entry + 1) % table;
    if w.bodies[at].x < l.from_x {
        w.bodies[at].alive = false;
    }
}

/// `contact.after_move`: the touch of body `e`, after its move.
fn touch_after_move(spec: &Spec, w: &mut World, f: &mut Frame, e: usize) {
    let after = w
        .bodies
        .get(e)
        .and_then(|b| spec.kind(b).contact.as_ref())
        .is_some_and(|c| c.after_move);
    if after && f.runs(System::Contact) {
        touches(spec, w, f, Some(e));
    }
}

/// `contact.near`: after body `e`'s move, a player with a `touch_points`
/// point near it is touched, as Super Mario Bros.' FirebarCollision tests
/// the fireball it has just placed.
fn near_touch(spec: &Spec, w: &mut World, f: &mut Frame, e: usize) {
    let Some(eb) = w.bodies.get(e).copied() else {
        return;
    };
    let Some(c) = &spec.kind(&eb).contact else {
        return;
    };
    let Some(n) = c.near else {
        return;
    };
    let (x, y) = (eb.x.floor(), eb.y.floor());
    let left = w.cameras.first().map_or(0, |v| v[0].floor());
    if !eb.active() || eb.carrier > 0 || n.right.is_some_and(|r| x - left >= r) {
        return;
    }
    for p in 0..w.bodies.len() {
        if f.level_started || p >= w.bodies.len() {
            break;
        }
        let pb = w.bodies[p];
        let kind = spec.kind(&pb);
        if kind.role != Role::Player || !pb.active() || f.out[p] {
            continue;
        }
        let (px, py) = (pb.x.floor(), pb.y.floor());
        let near =
            |t: &[i64; 2]| (px + t[0] - x).abs() < n.within && (py + t[1] - y).abs() < n.within;
        if !kind.touch_points.iter().any(near) {
            continue;
        }
        let invincible = kind
            .invincible
            .is_some_and(|g| g.cmp.holds(w.counters[g.counter], g.value));
        let t = if invincible {
            c.invincible
        } else {
            c.otherwise
        };
        touch(spec, w, f, p, e, t, c.damage);
    }
}

/// Places the players standing on body `e` on it, after its move, with the
/// fractions of their heights kept and their speed up and down 0.
fn place_riders(spec: &Spec, w: &mut World, f: &Frame, e: usize) {
    for &(p, _, below) in f.rides.iter().filter(|r| r.1 == e) {
        let eb = w.bodies[e];
        let Some(r) = spec.kind(&eb).contact.as_ref().and_then(|c| c.ride) else {
            continue;
        };
        let top = ride_top(&r, hit_pixels(spec, &eb)[1], below);
        let h = spec.kind(&w.bodies[p]).hitbox;
        let b = &mut w.bodies[p];
        b.y = Fx::from_int(top - h.y.floor() - h.h.floor()).add(Fx(b.y.frac()));
    }
}

/// The top of a `contact.ride` box `below` pixels under a body's top.
fn ride_top(r: &crate::spec::Ride, top: i64, below: i64) -> i64 {
    if r.wrap > 0 {
        (top + below).rem_euclid(r.wrap)
    } else {
        top + below
    }
}

/// Whether body `b` moves in the contact system (`hits.move_first`,
/// platformer 0.4.10) rather than the enemies system.
fn moves_in_contact(spec: &Spec, b: &Body) -> bool {
    spec.kind(b).hits.as_ref().is_some_and(|h| h.move_first)
}

/// One body's turn in [`enemies_ext`]: if it's active and not a player,
/// its patrol, walk, fly, path and fall components, its springboard, then
/// its `ext` hook.
pub fn enemy(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, exts: &Exts) {
    let k = &spec.kinds[w.bodies[i].kind];
    if k.role == Role::Player || k.hook_first || k.hook_last || !w.bodies[i].active() {
        return;
    }
    if f.still.contains(&i) {
        run_hook(spec, w, f, i, exts, Phase::Appear);
        return;
    }
    w.bodies[i].invuln = w.bodies[i].invuln.saturating_sub(1);
    // A carried entity rides its carrier and runs no components.
    if w.bodies[i].carrier > 0 {
        run_ext(spec, w, f, i, exts);
        return;
    }
    shoot(spec, w, f, i);
    patrol(spec, w, f, i);
    land_first(spec, w, i);
    walk(spec, w, i);
    fly(spec, w, i);
    path(spec, w, i);
    oscillate(spec, w, i);
    leap(spec, w, i);
    shuttle(spec, w, i);
    if let Some(fall) = &spec.kind(&w.bodies[i]).fall {
        let mut b = w.bodies[i];
        let (gravity, max_fall) = in_water(spec, w, &b, fall.gravity, fall.max_fall);
        let beat = |hop: &crate::spec::Hop| {
            hop.every == 0
                || (w.frame + u64::from(hop.phase) * i as u64).is_multiple_of(u64::from(hop.every))
        };
        let hop_speed = |hop: &crate::spec::Hop, b: &mut Body| {
            if hop.pattern.is_empty() {
                hop.speed
            } else {
                let n = b.ext[0].rem_euclid(hop.pattern.len() as i64) as usize;
                b.ext[0] = b.ext[0].wrapping_add(1);
                hop.pattern[n]
            }
        };
        let kind = spec.kind(&b);
        match &kind.hop {
            // With `land_first` it has landed already ([`land_first`]) and
            // the move doesn't land.
            Some(hop) if hop.land_first => {
                let limit = (max_fall, fall.max_fall_from_half);
                if fall.move_first {
                    b.y = b.y.add(b.vy);
                    b.vy = fall_limit(b.vy.add(gravity), limit.0, limit.1);
                } else {
                    b.vy = fall_limit(b.vy.add(gravity), limit.0, limit.1);
                    b.y = b.y.add(b.vy);
                }
                b.on_ground = false;
            }
            // With `stands_still` a body on the ground doesn't move up or
            // down: it only looks for the ground, and falls from the next
            // frame if it has gone.
            _ if fall.stands_still && b.on_ground => {
                b.on_ground = ground_under_t(w.tiles(spec), kind, &b);
            }
            _ => {
                vertical(
                    spec,
                    w.tiles(spec),
                    &mut b,
                    gravity,
                    (max_fall, fall.max_fall_from_half),
                    fall.move_first,
                    &[],
                );
                // A hopper jumps again on any frame it ends standing: the
                // frame it lands, or its first when placed on the ground.
                // On a beat, only on the beat's frames.
                if let Some(hop) = &kind.hop
                    && b.on_ground
                    && beat(hop)
                {
                    b.vy = hop_speed(hop, &mut b);
                    b.on_ground = false;
                }
            }
        }
        w.bodies[i] = b;
    }
    bounce(spec, w, i);
    if !w.bodies[i].alive {
        return;
    }
    orbit(spec, w, f, i);
    orbit_part(spec, w, i);
    crate::springboard::step(spec, w, i);
    leave_view(spec, w, i);
    run_ext(spec, w, f, i, exts);
}

/// Bouncing (platformer 0.4.10): after the move, a tile that isn't empty,
/// water or a ladder at the bounce point bounces a body moving down that
/// didn't bounce on the last frame (its speed up and down becomes the
/// bounce's, its whole height rounded down to a multiple of `snap`), and
/// removes one moving up or that bounced on the last frame, as Super Mario
/// Bros.' FireballBGCollision sets a fireball exploding. With nothing
/// there the last bounce is forgotten.
pub fn bounce(spec: &Spec, w: &mut World, i: usize) {
    let mut b = w.bodies[i];
    let Some(bo) = spec.kind(&b).bounce else {
        return;
    };
    if bo.from_y.is_some_and(|y| b.y.floor() < y.floor()) {
        b.bounced = false;
        w.bodies[i] = b;
        return;
    }
    let (px, py) = (
        b.x.floor() + bo.point[0].floor(),
        b.y.floor() + bo.point[1].floor(),
    );
    let t = w.tile_at(spec, px, py);
    if matches!(t, Tile::Empty | Tile::Water | Tile::Ladder | Tile::Coin) {
        b.bounced = false;
    } else if b.vy < Fx::ZERO || b.bounced {
        b.alive = false;
    } else {
        b.vy = Fx::from_int(bo.speed.floor()).add(Fx(b.vy.frac()));
        b.bounced = true;
        if bo.snap > 1 {
            let snap = i64::from(bo.snap);
            let whole = b.y.floor().div_euclid(snap) * snap;
            b.y = Fx::from_int(whole).add(Fx(b.y.frac()));
        }
    }
    w.bodies[i] = b;
}

/// A hopper with `land_first` lands before it moves, as Super Mario Bros.'
/// EnemyJump runs in EnemyToBGCollisionDet ahead of the enemy's move: if
/// it stands on ground (its tile hitbox's bottom row in a tile that stops
/// it, sunk into it or not) and its speed's whole part is not -2, -1 or 0
/// (the game adds 2 and compares with 3 unsigned), it is put on that
/// tile's top, its height's fraction kept, and takes the hop's speed. On a
/// beat, only on the beat's frames. With `land_while_stopped`, [`run_frozen`]
/// calls it on frames the enemies don't move.
pub fn land_first(spec: &Spec, w: &mut World, i: usize) {
    let mut b = w.bodies[i];
    let kind = spec.kind(&b);
    let Some(hop) = kind.hop.as_ref().filter(|h| h.land_first) else {
        return;
    };
    let beat = hop.every == 0
        || (w.frame + u64::from(hop.phase) * i as u64).is_multiple_of(u64::from(hop.every));
    if (-2..=0).contains(&b.vy.floor())
        || !kind.tile_collision
        || !beat
        || !ground_under_t(w.tiles(spec), kind, &b)
    {
        return;
    }
    let h = kind.tile_hitbox;
    let ts = spec.levels[w.level].tile_size;
    let row = b.y.floor() + h.y.floor() + h.h.floor();
    let top = row.div_euclid(ts) * ts;
    b.y = Fx::from_int(top - h.y.floor() - h.h.floor()).add(Fx(b.y.frac()));
    b.vy = if hop.pattern.is_empty() {
        hop.speed
    } else {
        let n = b.ext[0].rem_euclid(hop.pattern.len() as i64) as usize;
        b.ext[0] = b.ext[0].wrapping_add(1);
        hop.pattern[n]
    };
    w.bodies[i] = b;
}

/// Swinging: the speed along the axis moves by the force toward the
/// center, up to the top speed either way, and the body moves by it. With
/// `rest_creep_every`, a body at no speed loses its fraction first, and
/// short of where it's placed it creeps instead of swinging.
pub fn oscillate(spec: &Spec, w: &mut World, i: usize) {
    let mut b = w.bodies[i];
    let Some(o) = &spec.kind(&b).oscillate else {
        return;
    };
    let axis = usize::from(o.vertical);
    let home = b.home[axis];
    let offset = match o.low {
        Some((at, low)) if home >= at => low,
        _ => o.center,
    };
    let center = home.add(offset);
    let (pos, v) = if o.vertical {
        (&mut b.y, &mut b.vy)
    } else {
        (&mut b.x, &mut b.vx)
    };
    if let Some(every) = o.rest_creep_every.filter(|_| *v == Fx::ZERO) {
        *pos = Fx::from_int(pos.floor());
        if *pos < home {
            if w.frame.is_multiple_of(u64::from(every.max(1))) {
                *pos = pos.add(Fx::from_int(1));
            }
            w.bodies[i] = b;
            return;
        }
    }
    let force = if *pos < center {
        o.force
    } else {
        o.force.neg()
    };
    if o.move_first {
        *pos = pos.add(*v);
    }
    *v = v.add(force).min(o.max_speed).max(o.max_speed.neg());
    if !o.move_first {
        *pos = pos.add(*v);
    }
    w.bodies[i] = b;
}

/// Leaping: words 0 and 1 of the body's mechanic words are the frames
/// left of its wait and its phase (0 before the leap, rising slowly when
/// there's a rise; 1 in the leap). A wait that's over starts it again from
/// the start, drawing the next wait; then a rise moves it up until its
/// height, the leap's first frame sets the speed, gravity applies on its
/// frames, and it moves. Once `below` under the start it goes back there.
/// With `beat`, word 0 counts the counter's ticks instead of frames: the
/// wait is taken down on the frames the counter's `every` comes round, and
/// a new body waits one tick before its first leap. With `draw`, a byte of a
/// shift counter picks the wait and the launch speed's fraction, read by the
/// body's numbered slot.
pub fn leap(spec: &Spec, w: &mut World, i: usize) {
    let mut b = w.bodies[i];
    let Some(l) = &spec.kind(&b).leap else {
        return;
    };
    let start = l.from.unwrap_or(b.home[1]);
    let restart = |b: &mut Body| b.y = leap_start(l, b);
    let mut rising = false;
    if let Some(counter) = l.beat {
        if b.ext[0] <= 0 && b.ext[1] == 0 {
            b.ext[0] = 1;
            restart(&mut b);
            b.ext[1] = 1;
        }
        if tick_comes_round(spec, w, counter) {
            b.ext[0] -= 1;
        }
        if b.ext[0] <= 0 {
            let drawn = l.draw.as_ref().map(|d| drawn_byte(spec, w, d, i));
            let n = match drawn {
                Some(byte) => usize::from(byte) % l.waits.len(),
                None => w.rng.below(l.waits.len() as u64) as usize,
            };
            b.ext[0] = i64::from(l.waits[n]);
            restart(&mut b);
            b.vy = match (drawn, &l.draw) {
                (Some(byte), Some(d)) => {
                    l.speed.add(Fx::from_raw(i64::from(byte | d.fraction) << 8))
                }
                _ => l.speed,
            };
        }
    } else {
        if !l.waits.is_empty() {
            if b.ext[0] <= 0 {
                let n = w.rng.below(l.waits.len() as u64) as usize;
                b.ext[0] = i64::from(l.waits[n]) * i64::from(l.interval);
                restart(&mut b);
                b.ext[1] = 0;
            }
            b.ext[0] -= 1;
        }
        if b.ext[1] == 0 {
            match l.rise {
                Some((speed, height)) if b.y > start.sub(height) => {
                    b.vy = speed;
                    rising = true;
                }
                _ => {
                    b.ext[1] = 1;
                    b.vy = l.speed;
                }
            }
        }
    }
    let v = b.vy;
    if !rising && w.frame.is_multiple_of(u64::from(l.every.max(1))) {
        b.vy = b.vy.add(l.gravity);
        if let Some(m) = l.max_fall {
            b.vy = fall_limit(b.vy, m, l.max_fall_from_half);
        }
    }
    b.y = b.y.add(if l.move_first { v } else { b.vy });
    if let Some(below) = l.below
        && b.y > start.add(below)
    {
        b.y = start;
        b.ext[1] = 0;
    }
    w.bodies[i] = b;
}

/// The height a leap starts from, with the body's fraction if it keeps it.
fn leap_start(l: &crate::spec::Leap, b: &Body) -> Fx {
    let start = l.from.unwrap_or(b.home[1]);
    if l.keep_fraction {
        start.add(Fx::from_raw(b.y.frac()))
    } else {
        start
    }
}

/// Body `i`'s byte of a shift counter as it will be at the end of this frame,
/// in the counters system: a register stepped earlier in the frame, as Super
/// Mario Bros.' NMI steps PseudoRandomBitReg before its enemies run. A body
/// with no numbered slot reads the first slot's byte.
fn drawn_byte(spec: &Spec, w: &World, d: &crate::spec::LeapDraw, i: usize) -> u8 {
    let slot = w.slots.iter().position(|s| s.body == i as u32 + 1);
    let at = d.at.saturating_sub(d.slot_step * slot.unwrap_or(0) as u32);
    shift_byte(spec, w, d.counter, at)
}

/// The byte `at` bits up in a shift counter as it will be at the end of this
/// frame.
pub(crate) fn shift_byte(spec: &Spec, w: &World, counter: usize, at: u32) -> u8 {
    let v = spec.counters[counter]
        .shift
        .as_ref()
        .map_or(w.counters[counter], |s| s.step(w.counters[counter]));
    (v >> at) as u8
}

/// Whether counter `counter`'s first `every` comes round at the end of this
/// frame, in the counters system: a tick on a shared beat, read earlier in
/// the frame, as Super Mario Bros.' DecTimers ticks before its enemies run.
fn tick_comes_round(spec: &Spec, w: &World, counter: usize) -> bool {
    let mut tick = 0;
    for (i, c) in spec.counters.iter().enumerate() {
        for change in &c.changes {
            if let Change::Every(every, _) = *change {
                if i == counter {
                    return w.ticks[tick].saturating_add(1) >= every;
                }
                tick += 1;
            }
        }
    }
    false
}

/// A shuttle: on its frames, mechanic word 0 counts the move, word 1 (0
/// right or down, 1 the other way) turns once the count passes
/// `turn_after`, and the body moves its step that way, and its sway across.
pub fn shuttle(spec: &Spec, w: &mut World, i: usize) {
    let mut b = w.bodies[i];
    let Some(sh) = &spec.kind(&b).shuttle else {
        return;
    };
    let frame = w.frame;
    if !frame.is_multiple_of(u64::from(sh.every.max(1))) {
        return;
    }
    b.ext[0] += 1;
    if b.ext[0] > i64::from(sh.turn_after) {
        b.ext[0] = 0;
        b.ext[1] ^= 1;
    }
    let along = if b.ext[1] == 0 {
        sh.step
    } else {
        sh.step.neg()
    };
    let across = sh.sway.map(|(step, half)| {
        if (frame / u64::from(half.max(1))) % 2 == 1 {
            step
        } else {
            step.neg()
        }
    });
    if sh.vertical {
        b.y = b.y.add(along);
        if let Some(a) = across {
            b.x = b.x.add(a);
        }
    } else {
        b.x = b.x.add(along);
        b.facing_left = along < Fx::ZERO;
        if let Some(a) = across {
            b.y = b.y.add(a);
        }
    }
    w.bodies[i] = b;
}

/// An orbit's pivot: its first frame places it and spawns its parts
/// (mechanic word 1 marks it done); then, inside the view if it must be,
/// its angle (word 0, in 256ths of a step) moves by its spin, past any
/// angle it skips.
pub fn orbit(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) {
    let Some(o) = spec.kind(&w.bodies[i]).orbit.clone() else {
        return;
    };
    if w.bodies[i].ext[1] == 0 {
        place_orbit(spec, w, f, i, &o);
        return;
    }
    if o.in_view
        && let Some(view) = w.cameras.first()
    {
        let (left, x) = (view[0].floor(), w.bodies[i].x.floor());
        let width = spec.cameras.first().map_or(256, |c| c.size[0]);
        if x <= left || x > left + width - 1 {
            return;
        }
    }
    let mask = i64::from(o.angles) * 256 - 1;
    let spin = o.spin.raw() >> 8;
    let b = &mut w.bodies[i];
    let mut state = (b.ext[0] + spin) & mask;
    // An angle it skips is passed over a step on.
    if o.skip.iter().any(|a| i64::from(*a) == state >> 8) {
        state += 0x100;
    }
    b.ext[0] = state;
}

/// Moves an orbit's pivot by its `place` and spawns its parts there.
fn place_orbit(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, o: &Orbit) {
    let b = &mut w.bodies[i];
    b.x = b.x.add(o.place[0]);
    b.y = b.y.add(o.place[1]);
    b.ext = [b.ext[0], 1, 0, 0];
    let (x, y) = (b.x, b.y);
    for _ in 0..o.parts {
        let nb = new_body(spec, w, o.part, x, y, false);
        w.bodies.push(nb);
        f.out.push(false);
        f.fresh_bodies.push(true);
    }
}

/// A body that just appeared with an orbit's or a leap's `place_on_appear`
/// is placed now. An orbit's parts wait with it when it's `still` this frame.
fn place_on_appear(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, still: bool) {
    if let Some(l) = &spec.kind(&w.bodies[i]).leap
        && l.place_on_appear
    {
        w.bodies[i].y = leap_start(l, &w.bodies[i]);
    }
    let Some(o) = spec.kind(&w.bodies[i]).orbit.clone() else {
        return;
    };
    if !o.place_on_appear || w.bodies[i].ext[1] != 0 {
        return;
    }
    let first = w.bodies.len();
    place_orbit(spec, w, f, i, &o);
    if still {
        f.still.extend(first..w.bodies.len());
    }
}

/// An orbit's part: its first frame finds its pivot and which part it is
/// (mechanic words 0, the pivot's index plus one, and 1); then it's placed
/// from the pivot's angle by its tables, and leaves when the pivot does.
pub fn orbit_part(spec: &Spec, w: &mut World, i: usize) {
    let Some(within) = spec.kind(&w.bodies[i]).orbit_part else {
        return;
    };
    if w.bodies[i].ext[0] == 0 {
        let me = w.bodies[i];
        let near = |b: &Body| b.x.sub(me.home[0]).floor().abs() + b.y.sub(me.home[1]).floor().abs();
        let pivot = w
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| b.alive && spec.kind(b).orbit.is_some() && near(b) <= within.floor())
            .min_by_key(|(_, b)| near(b))
            .map(|(k, _)| k);
        let Some(pivot) = pivot else {
            w.bodies[i].alive = false;
            return;
        };
        let before = w.bodies[..i]
            .iter()
            .filter(|b| b.kind == me.kind && b.home == me.home && b.alive)
            .count();
        w.bodies[i].ext[0] = pivot as i64 + 1;
        w.bodies[i].ext[1] = before as i64;
    }
    let pivot = w.bodies[(w.bodies[i].ext[0] - 1) as usize];
    if !pivot.alive {
        w.bodies[i].alive = false;
        return;
    }
    let angles = spec
        .kind(&pivot)
        .orbit
        .as_ref()
        .map_or(32, |o| i64::from(o.angles));
    let (half, quarter) = (angles / 2, angles / 4);
    let angle = pivot.ext[0] >> 8;
    let fold = |a: i64| {
        let a = a.rem_euclid(half);
        if a > quarter { half - a } else { a }
    };
    let k = spec.kind(&w.bodies[i]);
    let table = |n: &str| k.ext_tables.get(n).map(Vec::as_slice).unwrap_or(&[]);
    let (offsets, rows, signs) = (table("offsets"), table("rows"), table("signs"));
    let n = w.bodies[i].ext[1] as usize;
    let (Some(&row), false) = (rows.get(n), signs.is_empty()) else {
        return;
    };
    let at = |a: i64| offsets.get((row + fold(a)) as usize).copied().unwrap_or(0);
    let (h, v) = (at(angle), at(angle + quarter));
    let sign = signs[((angle / quarter) & 3) as usize];
    let dx = if sign & 1 != 0 { h } else { -h };
    let dy = if sign & 2 != 0 { v } else { -v };
    let b = &mut w.bodies[i];
    b.x = pivot.x.add(Fx::from_int(dx));
    b.y = pivot.y.add(Fx::from_int(dy));
}

/// Leaving the view: a body whose whole-pixel position from the view's
/// near edge is under the margin, or at or past the view's size less the
/// margin, leaves the level. With `bottom_margin` (platformer 0.4.24) so
/// does one at or past the view's height less it, whatever the axis.
pub fn leave_view(spec: &Spec, w: &mut World, i: usize) {
    // `far_margin` (platformer 0.4.10) is the margin at the far edge.
    let b = w.bodies[i];
    let Some(l) = &spec.kind(&b).leave_view else {
        return;
    };
    let view = |axis: usize| match spec.cameras.first() {
        Some(c) => (w.cameras[0][axis].floor(), c.size[axis]),
        None => (0, spec.levels[w.level].pixels()[axis]),
    };
    let (edge, size) = view(usize::from(l.vertical));
    let at = if l.vertical { b.y } else { b.x }.floor() - edge;
    let m = l.margin.floor();
    let far = l.far_margin.unwrap_or(l.margin).floor();
    let (top, height) = view(1);
    let below = l
        .bottom_margin
        .is_some_and(|bm| b.y.floor() - top >= height - bm.floor());
    if !(m..size - far).contains(&at) || below {
        w.bodies[i].alive = false;
    }
}

/// Runs body `i`'s `ext` hook, if its kind (as its state makes it) names
/// one the registry has and the body is active, then applies its requests
/// in order. A spawned body goes at the end of the world's bodies, so its
/// ID is the number of bodies before it; the systems already looping this
/// frame don't reach it, and the ones after do.
pub fn run_ext(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, exts: &Exts) {
    run_hook(spec, w, f, i, exts, Phase::Step);
}

/// [`run_ext`], calling the hook's `appear` instead of its `step` if
/// `appearing`.
/// Which of a hook's methods [`run_hook`] calls.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Step,
    Appear,
    FrameStart,
}

fn run_hook(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, exts: &Exts, phase: Phase) {
    let Some(&b) = w.bodies.get(i) else {
        return;
    };
    let Some((name, hook)) = spec
        .kind(&b)
        .ext
        .as_ref()
        .and_then(|name| exts.get(name).map(|h| (name, h)))
    else {
        return;
    };
    if !b.active() {
        return;
    }
    crate::reads::note_hook(name);
    let (mut me, mut rng) = (b, w.rng);
    let mut draws = f.draws.take();
    let requests = {
        let mut cx = ExtCx {
            spec,
            frame: w.frame,
            id: i,
            me: &mut me,
            world: w,
            rng: &mut rng,
            counters_stopped: f.stopped.contains(&System::Counters),
            system_stopped: !f.runs(if spec.kind(&b).role == Role::Player {
                System::Player
            } else {
                System::Enemies
            }),
            draws: draws.as_mut(),
            requests: Vec::new(),
            cycles: f.cycles,
            start_counters: &f.start_counters,
            start_states: &f.start_states,
        };
        match phase {
            Phase::Step => hook.step(&mut cx),
            Phase::Appear => hook.appear(&mut cx),
            Phase::FrameStart => hook.frame_start(&mut cx),
        }
        f.cycles = cx.cycles;
        cx.requests
    };
    w.bodies[i] = me;
    w.rng = rng;
    f.draws = draws;
    for r in requests {
        match r {
            Request::Fire(kind, entity) => {
                if kind.0 < spec.events.len() {
                    emit(spec, w, f, kind, entity);
                }
            }
            Request::Remove(id) => {
                if let Some(b) = w.bodies.get_mut(id) {
                    b.alive = false;
                }
            }
            Request::Spawn {
                kind,
                x,
                y,
                facing_left,
                slot_fraction,
            } => {
                if kind >= spec.kinds.len() {
                    continue;
                }
                let nb = new_body(spec, w, kind, x, y, facing_left);
                w.bodies.push(nb);
                f.out.push(false);
                f.fresh_bodies.push(true);
                let at = w.bodies.len() - 1;
                if let Some([x_frac, _, y_frac]) = claim_slot(spec, w, at)
                    && slot_fraction
                {
                    take_rest(&mut w.bodies[at], x_frac, y_frac);
                }
            }
            Request::SetTile { x, y, tile } => w.set_tile(spec, x, y, tile),
            Request::WriteTile { x, y, tile } => w.write_tile(spec, x, y, tile),
            Request::SetBody(id, body) => {
                if let Some(b) = w.bodies.get_mut(id) {
                    let old = *b;
                    *b = body;
                    place_riders(spec, w, f, id);
                    // A hook that moves another ground body carries its
                    // riders; its own moves are carried after its turn.
                    if id != i {
                        carry_riders(spec, w, id, old);
                    }
                }
            }
            Request::SetSlotWord { slot, word } => w.slots[slot].word = word,
            Request::SetCounter { counter, value } => w.counters[counter] = value,
            Request::Hold { mask, bits } => w.held = (w.held & !mask) | (bits & mask),
        }
    }
}

/// The view a level's spawning reads, along its axis (y when the level's
/// camera follows y only): the first camera's corner and size, or the
/// level's own edge and size when there's none.
fn view(spec: &Spec, w: &World) -> (Fx, Fx) {
    let level = &spec.levels[w.level];
    let axis = usize::from(level.vertical());
    match spec.cameras.first() {
        Some(c) => (w.cameras[0][axis], Fx::from_int(c.size[axis])),
        None => (Fx::ZERO, Fx::from_int(level.pixels()[axis])),
    }
}

/// Entities waiting to appear whose spawn is within the level's
/// `spawn_ahead` past the view's far edge appear, a walker facing the
/// player. One waiting to respawn must first have its spawn out of the
/// view's reach (more than `spawn_ahead` beyond either edge). A level
/// with slots, an order or a draw ([`ruled`]) has its own rule.
fn appear(spec: &Spec, w: &mut World, f: &mut Frame) {
    let level = &spec.levels[w.level];
    let Some(ahead) = level.spawn_ahead else {
        return;
    };
    if ruled(level) {
        appear_ruled(spec, w, f, ahead);
        return;
    }
    let fresh = &mut f.fresh_bodies;
    let axis = usize::from(level.vertical());
    let (near, width) = view(spec, w);
    let edge = near.add(width).add(ahead);
    let back = near.sub(ahead);
    let player = w
        .bodies
        .iter()
        .find(|b| spec.kinds[b.kind].role == Role::Player)
        .map(|b| b.x);
    let tiles = Tiles {
        level,
        changed: &w.tiles,
        bodies: &[],
    };
    let mut shown = Vec::new();
    for (i, b) in w.bodies.iter_mut().enumerate() {
        let at = if axis == 0 { b.x } else { b.y };
        if b.waiting && b.away {
            if at >= edge || at < back {
                b.away = false;
            }
            continue;
        }
        if b.waiting && b.alive && at < edge {
            b.waiting = false;
            // It enters its initial state now, which this frame doesn't
            // count toward `after`.
            if let Some(f) = fresh.get_mut(i) {
                *f = true;
            }
            let kind = spec.kind(b);
            face(kind, b, player);
            b.on_ground = kind.tile_collision && ground_under_t(tiles, kind, b);
            shown.push(i);
        }
    }
    for i in shown {
        place_on_appear(spec, w, f, i, false);
    }
}

/// Whether a level's spawning by the view has core 0.4.10's rules: slots,
/// an order, a still first frame or a draw.
fn ruled(level: &crate::spec::Level) -> bool {
    level.spawn_slots.is_some()
        || level.spawn_in_order
        || level.appear_still
        || level.draw.is_some()
}

/// Sets a level's draw going from the view as it stands, as a level is
/// placed: every column left of `ahead` past the view's far edge is drawn
/// and no set is being drawn. Without a draw both counts are 0.
pub fn start_draw(spec: &Spec, w: &mut World) {
    w.draw_left = 0;
    w.draw_set = match spec.levels[w.level].draw {
        Some(d) => near_steps(spec, w, &d),
        None => 0,
    };
}

/// The view's near edge in whole steps of a draw, rounded down.
fn near_steps(spec: &Spec, w: &World, d: &crate::spec::Draw) -> i64 {
    view(spec, w).0.floor().div_euclid(d.step.max(1))
}

/// The frames a draw's set takes: one column every `every` frames.
fn set_frames(level: &crate::spec::Level, d: &crate::spec::Draw) -> u32 {
    (d.step / level.tile_size.max(1)).max(1) as u32 * d.every.max(1)
}

/// How far the level is drawn: every column left of this is.
pub fn drawn_to(spec: &Spec, w: &World) -> Option<Fx> {
    let level = &spec.levels[w.level];
    let d = level.draw?;
    let (_, width) = view(spec, w);
    let edge = |set: i64| Fx::from_int(set * d.step).add(width).add(d.ahead);
    if w.draw_left == 0 {
        return Some(edge(w.draw_set));
    }
    let done = set_frames(level, &d).saturating_sub(w.draw_left);
    let columns = i64::from(done.div_ceil(d.every.max(1)));
    Some(edge(w.draw_set - 1).add(Fx::from_int(columns * level.tile_size)))
}

/// The left edge of the tile column a spawn's place is in, along the
/// level's axis.
fn column_of(level: &crate::spec::Level, at: Fx) -> Fx {
    Fx::from_int(at.floor().div_euclid(level.tile_size.max(1)) * level.tile_size)
}

/// Whether the view has reached body `i`'s spawn by the level's rule: a
/// `drawn` spawn once its column is drawn, any other once it is within
/// `spawn_ahead` past the view's far edge. A translation from a game that
/// has already spawned what the view reached uses it.
pub fn reached(spec: &Spec, w: &World, i: usize) -> bool {
    let level = &spec.levels[w.level];
    let Some(b) = w.bodies.get(i) else {
        return false;
    };
    let axis = usize::from(level.vertical());
    let at = if axis == 0 { b.x } else { b.y };
    if level.spawns.get(i).is_some_and(|s| s.drawn)
        && let Some(to) = drawn_to(spec, w)
    {
        return column_of(level, at) < to;
    }
    let (near, width) = view(spec, w);
    at < near.add(width).add(level.spawn_ahead.unwrap_or(Fx::ZERO))
}

/// The slots of a level taken now: one for each active body but the player
/// of a kind without `no_slot`, and its `extra_slots`.
fn slots_taken(spec: &Spec, w: &World) -> i64 {
    w.bodies
        .iter()
        .filter(|b| {
            b.active() && spec.kinds[b.kind].role != Role::Player && !spec.kinds[b.kind].no_slot
        })
        .map(|b| 1 + i64::from(spec.kinds[b.kind].extra_slots))
        .sum()
}

/// Spawning by the view with core 0.4.10's rules, in the order a game
/// that keeps its enemies in slots runs it (Super Mario Bros.' enemy loop,
/// then its area parser at the end of the frame):
///
/// 1. `drawn` spawns whose column is drawn already appear, for a level
///    just placed: in order across, each while a slot is free, and the
///    rest never.
/// 2. Unless a draw's `pause` holds them, the others within `spawn_ahead`
///    appear while a slot is free. With `spawn_in_order` they go in the
///    order of the spawns, one for each slot free when the step began: one
///    waits for those before it, and one already left of the view's last
///    column at its turn is dropped and uses its turn.
/// 3. The draw goes on: when no set is being drawn and the view's near
///    edge has passed another whole step, the next set starts; each frame
///    of a set draws its next column every `every` frames, and the
///    `drawn` spawns in it appear while a slot is free, the rest never.
///
/// With `appear_still` a body that appears here doesn't move this frame.
fn appear_ruled(spec: &Spec, w: &mut World, f: &mut Frame, ahead: Fx) {
    let level = &spec.levels[w.level];
    let axis = usize::from(level.vertical());
    let (near, width) = view(spec, w);
    let edge = near.add(width).add(ahead);
    let back = near.sub(ahead);
    let last = Fx::from_int(near.floor() + width.floor() - 1);
    let mut free = level
        .spawn_slots
        .map_or(i64::MAX, |n| i64::from(n) - slots_taken(spec, w));
    let pos = |b: &Body| if axis == 0 { b.x } else { b.y };
    let drawn = |i: usize| level.draw.is_some() && level.spawns.get(i).is_some_and(|s| s.drawn);
    let n = level.spawns.len().min(w.bodies.len());

    // 1. Drawn spawns the draw has already reached.
    if let Some(to) = drawn_to(spec, w) {
        let mut due: Vec<usize> = (0..n)
            .filter(|&i| drawn(i) && w.bodies[i].waiting && w.bodies[i].alive)
            .filter(|&i| column_of(level, pos(&w.bodies[i])) < to)
            .collect();
        due.sort_by_key(|&i| (pos(&w.bodies[i]), i));
        for i in due {
            show(spec, w, f, i, &mut free);
        }
    }

    // 2. The others, by spawn_ahead.
    let paused = level.draw.is_some_and(|d| {
        w.draw_left > 0 && set_frames(level, &d).saturating_sub(w.draw_left) <= d.pause
    });
    if !paused {
        let mut turns = if level.spawn_in_order {
            free.max(0)
        } else {
            i64::MAX
        };
        for i in 0..n {
            let b = w.bodies[i];
            if drawn(i) || !b.waiting || !b.alive {
                continue;
            }
            let at = pos(&b);
            if b.away {
                if at >= edge || at < back {
                    w.bodies[i].away = false;
                }
                continue;
            }
            if at >= edge {
                if level.spawn_in_order {
                    break;
                }
                continue;
            }
            if level.spawn_in_order {
                if turns == 0 {
                    break;
                }
                turns -= 1;
                if at < last {
                    w.bodies[i].waiting = false;
                    w.bodies[i].alive = false;
                    continue;
                }
            }
            if free <= 0 {
                if level.spawn_in_order {
                    break;
                }
                continue;
            }
            show(spec, w, f, i, &mut free);
        }
    }
    stream(spec, w, f, &mut free);

    // 3. The draw goes on.
    let Some(d) = level.draw else {
        return;
    };
    if w.draw_left == 0 && near_steps(spec, w, &d) > w.draw_set {
        w.draw_set += 1;
        w.draw_left = set_frames(level, &d);
    }
    if w.draw_left == 0 {
        return;
    }
    let from = drawn_to(spec, w).unwrap_or(Fx::ZERO);
    w.draw_left -= 1;
    let to = drawn_to(spec, w).unwrap_or(Fx::ZERO);
    let mut due: Vec<usize> = (0..n)
        .filter(|&i| drawn(i) && w.bodies[i].waiting && w.bodies[i].alive)
        .filter(|&i| {
            let c = column_of(level, pos(&w.bodies[i]));
            c >= from && c < to
        })
        .collect();
    due.sort_by_key(|&i| (pos(&w.bodies[i]), i));
    for i in due {
        show(spec, w, f, i, &mut free);
    }
    // The column of a `scroll_stop`: its kinds go (ScrollLockObject_Warp's
    // KillEnemies, from the area parser at the end of the frame).
    if let Some(stop) = &level.scroll_stop {
        let c = column_of(level, stop.at);
        if c >= from && c < to {
            for b in &mut w.bodies {
                if b.active() && stop.removes.contains(&b.kind) {
                    b.alive = false;
                }
            }
        }
    }
}

/// Whether the column of the level's `scroll_stop` is drawn: no camera
/// moves from the frame after. The draw follows the view, so once it is
/// drawn it stays drawn.
fn scroll_stopped(spec: &Spec, w: &World) -> bool {
    let level = &spec.levels[w.level];
    level
        .scroll_stop
        .as_ref()
        .is_some_and(|s| drawn_to(spec, w).is_some_and(|to| column_of(level, s.at) < to))
}

/// The level's markers with a `stream`, after this frame's spawns: each one's
/// wait counts down, and at 0, while a slot is free, it sends its next body.
/// With its `until` kind in the level it ends instead. The new body doesn't
/// move this frame (InitBowserFlame runs from the area parser, on the frame's
/// free slot, and RunBowserFlame from the next).
fn stream(spec: &Spec, w: &mut World, f: &mut Frame, free: &mut i64) {
    for i in 0..w.bodies.len() {
        let m = w.bodies[i];
        let Some(st) = (if m.active() {
            &spec.kinds[m.kind].stream
        } else {
            &None
        }) else {
            continue;
        };
        if let Some(until) = st.until
            && w.of_kind(until).any(|(_, b)| b.active())
        {
            w.bodies[i].alive = false;
            continue;
        }
        if m.ext[0] > 0 {
            w.bodies[i].ext[0] -= 1;
        }
        if w.bodies[i].ext[0] > 0 || *free <= 0 {
            continue;
        }
        *free -= 1;
        let tables = &spec.kinds[m.kind].ext_tables;
        let (Some(heights), Some(times)) = (tables.get("heights"), tables.get("times")) else {
            continue;
        };
        if heights.is_empty() || times.is_empty() {
            continue;
        }
        let (near, width) = view(spec, w);
        let x = Fx::from_int(near.floor() + width.floor() - 1).add(st.past_edge);
        let nb = new_body(spec, w, st.entity, x, Fx::ZERO, true);
        w.bodies.push(nb);
        f.out.push(false);
        f.fresh_bodies.push(true);
        let at = w.bodies.len() - 1;
        claim_slot(spec, w, at);
        let row = usize::from(drawn_byte(spec, w, &st.draw, at)) % heights.len();
        let b = &mut w.bodies[at];
        b.y = Fx::from_int(heights[row]);
        b.home[1] = b.y;
        b.ext = [row as i64, 0, 0, 1];
        if spec.levels[w.level].appear_still {
            f.still.push(at);
        }
        let turn = m.ext[1].rem_euclid(times.len() as i64);
        let wait = i64::from(st.wait) + times[turn as usize];
        let b = &mut w.bodies[i];
        b.ext[0] = wait;
        b.ext[1] = turn + 1;
    }
}

/// Body `i` appears if a slot is free, taking it (unless it's of a `no_slot`
/// kind), and is dropped for good if none is (a drawn spawn: its column isn't
/// drawn again). Spawning by the view's other callers check `free` first.
fn show(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, free: &mut i64) {
    // A kind with `no_slot` takes none (platformer 0.4.28: a launcher).
    let no_slot = spec.kind(&w.bodies[i]).no_slot;
    if *free <= 0 && !no_slot {
        w.bodies[i].waiting = false;
        w.bodies[i].alive = false;
        return;
    }
    if !no_slot {
        *free -= 1 + i64::from(spec.kind(&w.bodies[i]).extra_slots);
    }
    let level = &spec.levels[w.level];
    let player = w
        .bodies
        .iter()
        .find(|b| spec.kinds[b.kind].role == Role::Player)
        .map(|b| b.x);
    let tiles = Tiles {
        level,
        changed: &w.tiles,
        bodies: &[],
    };
    let mut b = w.bodies[i];
    b.waiting = false;
    // A spawn `at_edge` is placed past the view's last column as it
    // appears (core 0.4.11).
    if let Some(off) = level.spawns.get(i).and_then(|s| s.at_edge) {
        let (near, width) = view(spec, w);
        let at = Fx::from_int(near.floor() + width.floor() - 1).add(off);
        let axis = usize::from(level.vertical());
        if axis == 0 {
            b.x = at;
        } else {
            b.y = at;
        }
        b.home[axis] = at;
    }
    let kind = spec.kind(&b);
    face(kind, &mut b, player);
    b.on_ground = kind.tile_collision && ground_under_t(tiles, kind, &b);
    w.bodies[i] = b;
    // With `slot_rest` it takes a numbered slot and what its last body left
    // there: the fraction across and the speed up and down. A body that
    // falls takes the fraction of its height too (ImposeGravity adds to
    // Super Mario Bros.' Enemy_YMF_Dummy and nothing clears it, and a mover
    // by whole pixels, as its piranha plant, never reads it). One that
    // `stands_still` starts at rest on the ground, as SetBBox runs on into
    // InitVStf, and so does a swing that `starts_at_rest`.
    if level.slot_rest
        && let Some([frac, vy, y_frac]) = claim_slot(spec, w, i)
    {
        let kind = spec.kind(&w.bodies[i]);
        let falls = kind.fall.is_some();
        let stands = kind.fall.as_ref().is_some_and(|f| f.stands_still);
        let swings = kind.oscillate.as_ref().is_some_and(|o| o.starts_at_rest);
        let b = &mut w.bodies[i];
        b.x = Fx::from_int(b.x.floor()).add(frac);
        if falls {
            b.y = Fx::from_int(b.y.floor()).add(y_frac);
        }
        if !(stands && b.on_ground || swings) {
            b.vy = vy;
        }
        // `orbit.angle_in_slot`: InitShortFirebar clears only the spin
        // state's low byte, so the high byte is the slot's Enemy_Y_Speed.
        if let Some(o) = kind.orbit.as_ref().filter(|o| o.angle_in_slot) {
            b.ext[0] = (vy.floor() & (i64::from(o.angles) - 1)) << 8;
        }
    }
    if let Some(fr) = f.fresh_bodies.get_mut(i) {
        *fr = true;
    }
    let moving = level.spawns.get(i).is_some_and(|s| s.appear_moving);
    let still = level.appear_still && !f.level_started && !moving;
    if still {
        f.still.push(i);
    }
    place_on_appear(spec, w, f, i, still);
    take_entry(spec, w, f, i);
}

/// Faces an entity that's appearing: a walker toward the player, a flier
/// as its heading says. The player's x is `player`, when there is one.
pub fn face(kind: &crate::spec::Kind, b: &mut Body, player: Option<Fx>) {
    use crate::spec::Heading;
    let toward = player.is_some_and(|px| px < b.x);
    if kind.walk.is_some() {
        b.facing_left = toward;
    }
    if let Some(fly) = &kind.fly {
        b.facing_left = match fly.heading {
            Heading::Player => toward,
            Heading::Left => true,
            Heading::Right => false,
        };
    }
}

/// Spawning by the view: entities appear as the view comes within the
/// level's `spawn_ahead` of them, and those whose far edge is more than
/// `despawn_behind` before the view, or whose near edge is more than
/// `despawn_ahead` past it, go: for good, or with `respawn` back to their
/// spawn to wait for the view to come back.
pub fn spawn(spec: &Spec, w: &mut World, f: &mut Frame) {
    loop_back(spec, w);
    appear(spec, w, f);
    if !spec.levels[w.level].despawn_after_move {
        despawn(spec, w);
    }
}

/// A level's `loop_back`, in the enemy loop's free slot as Super Mario
/// Bros.' ProcLoopCommand runs it: with a slot free and the draw finished at
/// a check's column, a player who isn't standing at the check's height goes
/// back, with the view and the draw (ExecGameLoopback). Everything else in
/// the level goes (KillAllEnemies) and waits at its spawn, but `drawn`
/// spawns behind the draw, whose columns aren't drawn again. The `keep`
/// columns behind the draw go back with the view, since the game's tile
/// buffer holds them; columns the draw will reach are as built again.
fn loop_back(spec: &Spec, w: &mut World) {
    let level = &spec.levels[w.level];
    let (Some(l), Some(d)) = (&level.loop_back, level.draw) else {
        return;
    };
    if w.draw_left != 0 {
        return;
    }
    let Some(to) = drawn_to(spec, w).map(Fx::floor) else {
        return;
    };
    let Some(check) = l.checks.iter().find(|c| c.x == to) else {
        return;
    };
    if level
        .spawn_slots
        .is_some_and(|n| slots_taken(spec, w) >= i64::from(n))
    {
        return;
    }
    let is_player = |b: &Body| spec.kinds[b.kind].role == Role::Player;
    let passed = w
        .bodies
        .iter()
        .find(|b| is_player(b) && b.active())
        .is_some_and(|b| b.on_ground && b.y.floor() == check.y);
    if passed {
        return;
    }
    let back = Fx::from_int(l.back);
    for b in &mut w.bodies {
        if is_player(b) {
            b.x = b.x.sub(back);
            b.home[0] = b.home[0].sub(back);
        } else {
            b.alive = false;
        }
    }
    release_slots(spec, w);
    for s in &mut w.shots {
        s.body = 0;
    }
    w.draw_set -= l.back / d.step.max(1);
    let new_to = Fx::from_int(to - l.back);
    for (i, s) in level.spawns.iter().enumerate() {
        if spec.kinds[s.kind].role == Role::Player || i >= w.bodies.len() {
            continue;
        }
        if !s.drawn || column_of(level, s.x) >= new_to {
            w.bodies[i] = crate::world::placed_body(spec, level, s);
        }
    }
    for c in &mut w.cameras {
        c[0] = c[0].sub(back);
    }
    for at in &mut w.follow_at {
        at[0] -= back.floor();
    }
    let t = level.tile_size.max(1);
    let (to_col, back_cols) = (to / t, l.back / t);
    let kept: Vec<(i64, i64, Tile)> = (to_col - l.keep..to_col)
        .flat_map(|c| (0..level.size[1]).map(move |r| (c, r)))
        .map(|(c, r)| (c - back_cols, r, w.tile_at(spec, c * t, r * t)))
        .collect();
    let width = level.size[0].max(1) as u32;
    let from_col = to_col - back_cols - l.keep;
    w.tiles.retain(|(i, _)| i64::from(i % width) < from_col);
    w.held_cells.clear();
    for (c, r, tile) in kept {
        w.set_tile(spec, c * t, r * t, tile);
    }
}

/// The leaving half of [`spawn`], which runs after the enemies system
/// instead with `despawn_after_move`.
fn despawn(spec: &Spec, w: &mut World) {
    let level = &spec.levels[w.level];
    if level.despawn_behind.is_none() && level.despawn_ahead.is_none() {
        return;
    }
    let axis = usize::from(level.vertical());
    let (near, width) = view(spec, w);
    for i in 0..w.bodies.len() {
        let b = w.bodies[i];
        // A launcher keeps its entry wherever the view is (SetupCannon's
        // table is cleared only with the area).
        if !b.active()
            || spec.kinds[b.kind].role == Role::Player
            || b.carrier > 0
            || spec.kind(&b).launcher.is_some()
        {
            continue;
        }
        let pos = if axis == 0 { b.x } else { b.y };
        // The far edge behind, or with `despawn_at = "position"` the
        // position's whole pixels both ways.
        let (far, pos) = if level.despawn_by_position {
            let at = Fx::from_int(pos.floor());
            (at, at)
        } else {
            (pos.add(spec.kind(&b).size[axis]), pos)
        };
        let wrapped = spec
            .kind(&b)
            .despawn_behind_wrap
            .filter(|[from, _]| near.floor() & 0xff >= i64::from(*from))
            .map(|[_, d]| Fx::from_int(i64::from(d)));
        let behind = wrapped
            .or(spec.kind(&b).despawn_behind)
            .or(level.despawn_behind)
            .is_some_and(|d| far < near.sub(d));
        let ahead = spec
            .kind(&b)
            .despawn_ahead
            .or(level.despawn_ahead)
            .is_some_and(|d| pos > near.add(width).add(d));
        if !(behind || ahead) {
            continue;
        }
        match level.spawns.get(i) {
            Some(sp) if level.respawn && level.spawn_ahead.is_some() => {
                let base = &spec.kinds[sp.kind];
                w.bodies[i] = Body {
                    kind: sp.kind,
                    x: sp.x,
                    y: sp.y,
                    alive: true,
                    waiting: true,
                    away: true,
                    state: base.machine.as_ref().map_or(0, |m| m.initial),
                    home: [sp.x, sp.y],
                    ..Body::default()
                };
            }
            _ => w.bodies[i].alive = false,
        }
    }
}

/// Flying: an entity with a fly component moves its speed a frame the way
/// it faces, through tiles, never turning.
pub fn fly(spec: &Spec, w: &mut World, i: usize) {
    let mut b = w.bodies[i];
    let Some(fly) = &spec.kind(&b).fly else {
        return;
    };
    if !fly.keep_speed || b.vx == Fx::ZERO {
        b.vx = if b.facing_left {
            fly.speed.neg()
        } else {
            fly.speed
        };
    }
    b.x = b.x.add(b.vx);
    w.bodies[i] = b;
}

/// Following a path: out from the spawn by `distance` along its axis at
/// `speed`, a wait of `pause` frames, back, and a wait at the spawn end,
/// which with `wait_near` lasts while the player is that close across.
/// With `every`, it moves only on frames whose number plus `phase` is a
/// multiple of `every`; a wait counts down on every frame.
pub fn path(spec: &Spec, w: &mut World, i: usize) {
    let mut b = w.bodies[i];
    let Some(p) = spec.kind(&b).path.clone() else {
        return;
    };
    let axis = usize::from(p.vertical);
    let home = b.home[axis];
    let far = home.add(p.distance);
    let pos = if p.vertical { b.y } else { b.x };
    if b.wait > 0 {
        b.wait -= 1;
        w.bodies[i] = b;
        return;
    }
    let at_home = !b.path_back && pos == home;
    if at_home && let Some(near) = p.wait_near {
        let player = w
            .bodies
            .iter()
            .find(|o| spec.kinds[o.kind].role == Role::Player)
            .map(|o| o.x);
        let near_now = player.is_some_and(|px| {
            if p.wait_near_wrap > 0 {
                let d = (px.floor() - b.x.floor()).abs() % i64::from(p.wait_near_wrap);
                Fx::from_int(d) < near
            } else {
                px.sub(b.x).max(b.x.sub(px)) < near
            }
        });
        if near_now {
            return;
        }
    }
    if p.every > 1 && !(w.frame + u64::from(p.phase)).is_multiple_of(u64::from(p.every)) {
        return;
    }
    let target = if b.path_back { home } else { far };
    let next = if pos < target {
        pos.add(p.speed).min(target)
    } else {
        pos.sub(p.speed).max(target)
    };
    if p.vertical {
        b.vy = next.sub(pos);
        b.y = next;
    } else {
        b.vx = next.sub(pos);
        b.x = next;
    }
    if next == target {
        b.path_back = !b.path_back;
        b.wait = p.pause;
    }
    w.bodies[i] = b;
}

/// Walking: an entity with a walk component moves its speed a frame the way
/// it faces. Blocked by a tile, it stays where it was and turns; with
/// `turn_at_ledges`, standing with no ground under its leading bottom
/// corner after the move, it does the same. With `turn_after_move` a wall
/// doesn't block it: it moves, and turns where it stands in the wall. With
/// `ledge_at` the ledge is looked for after the move instead, at that one
/// point across the body, and the walker turns where it stands.
pub fn walk(spec: &Spec, w: &mut World, i: usize) {
    let mut b = w.bodies[i];
    let kind = spec.kind(&b);
    let Some(walk) = &kind.walk else { return };
    if walk.speed == Fx::ZERO {
        b.vx = Fx::ZERO;
        w.bodies[i] = b;
        return;
    }
    let tiles = w.tiles(spec);
    let dx = if b.facing_left {
        walk.speed.neg()
    } else {
        walk.speed
    };
    // The point `ledge_at` names has no ground under it, where the body is.
    let past_ledge = |b: &Body| {
        let h = kind.tile_hitbox;
        let row = b.y.floor() + h.y.floor() + h.h.floor();
        walk.turn_at_ledges
            && b.on_ground
            && kind.tile_collision
            && walk
                .ledge_at
                .is_some_and(|at| !tiles.lands_at(b.x.floor() + at.floor(), row, row, false))
    };
    if walk.turn_at_ledges && walk.ledge_at.is_none() && b.on_ground && kind.tile_collision {
        let h = kind.tile_hitbox;
        let ahead = b.x.add(dx);
        let col = if b.facing_left {
            ahead.floor() + h.x.floor()
        } else {
            ahead.floor() + h.x.floor() + h.w.floor() - 1
        };
        let row = b.y.floor() + h.y.floor() + h.h.floor();
        if !tiles.lands_at(col, row, row, false) {
            b.facing_left = !b.facing_left;
            b.vx = Fx::ZERO;
            w.bodies[i] = b;
            return;
        }
    }
    let before = b.x;
    b.vx = dx;
    if walk.turn_after_move && kind.tile_collision {
        // The move always happens; a wall at the leading edge of the tile
        // hitbox's top row then turns it where it stands.
        let h = kind.tile_hitbox;
        let max_x = Fx::from_int(tiles.level.pixels()[0] - kind.size[0].floor());
        b.x = b.x.add(dx).max(Fx::ZERO).min(max_x);
        let edge = if dx < Fx::ZERO {
            b.x.floor() + h.x.floor()
        } else {
            b.x.floor() + h.x.floor() + h.w.floor() - 1
        };
        if past_ledge(&b) || tiles.solid_at(edge, b.y.floor() + h.y.floor()) {
            b.facing_left = !b.facing_left;
        }
        w.bodies[i] = b;
        return;
    }
    if walk.walls_at_top && kind.tile_collision {
        // Walls at the tile hitbox's top row only.
        let h = kind.tile_hitbox;
        let to = b.x.add(dx);
        let edge = if dx < Fx::ZERO {
            to.floor() + h.x.floor()
        } else {
            to.floor() + h.x.floor() + h.w.floor() - 1
        };
        let max_x = tiles.level.pixels()[0] - kind.size[0].floor();
        let row = b.y.floor() + h.y.floor();
        if to.floor() < 0 || to.floor() > max_x || tiles.solid_at(edge, row) {
            b.vx = Fx::ZERO;
        } else {
            b.x = to;
        }
    } else {
        move_x(spec, tiles, &mut b, dx);
    }
    if b.vx == Fx::ZERO {
        // Blocked: back where it was, turned round.
        b.x = before;
        b.facing_left = !b.facing_left;
    } else if past_ledge(&b) {
        b.facing_left = !b.facing_left;
    }
    w.bodies[i] = b;
}

/// Walking and running, as docs/model.md's table says. A direction held
/// moves the speed toward the top speed (the run speed while the run action
/// is held) by `acceleration`, or at once without it, and a speed past the
/// top speed drops to it; against the motion on the ground, by
/// `skid_deceleration` (or `acceleration`). With nothing held the speed
/// goes toward 0 by `deceleration` on the ground and is kept in the air, or
/// without `deceleration` stops at once. Right wins when both are held.
/// Without air control an entity in the air keeps its speed and facing.
/// Each whole pixel moved is checked against solid tiles and the level's
/// sides; a blocked entity stops at the last open pixel, its speed 0.
///
/// The platformer 0.4.2 options change this as docs/model.md says:
/// `move_step` splits the speed into whole steps (`vx`, what the body moves
/// by) and the rest (`vx_frac`), `cap_whole` caps the whole steps only,
/// `skid_by = "last_motion"` doubles the rate while facing away from the
/// way it last moved, `skid_turn_below` turns a slow skid at once (and
/// `skid_turn_with`, platformer 0.4.11, names actions that steer for it),
/// `decel_bands` and `decel_hold_from` give the walking rates by speed, and
/// `run_hold_frames` and `run_with_motion` say when the run action counts.
/// Platformer 0.4.3's: `takeoff_in_air` makes the frame a jump starts a
/// frame in the air, `face_on_ground` keeps the facing in the air, and
/// `air_run_from` lets the speed alone say whether the run counts in the
/// air. Platformer 0.4.15's `face_both` lets both directions held be a
/// facing and a way of their own, though right still wins for the speed.
pub fn run(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) {
    {
        let mut b = w.bodies[i];
        let facing_before = b.facing_left;
        let kind = spec.kind(&b);
        let Some(run) = &kind.run else { return };
        if !b.active() || f.out[i] || run.off {
            return;
        }
        // `run.lone_direction`: a direction with up or down held is none.
        let lone = !run.lone_direction
            || !kind
                .climb
                .as_ref()
                .is_some_and(|c| w.held(spec, c.up) || w.held(spec, c.down));
        // `run.both_left`: left and right held together are no way to move,
        // and turn the body left on the ground (below).
        let both_left = run.both_left && lone && w.held(spec, run.right) && w.held(spec, run.left);
        let dir_held = lone && !both_left && (w.held(spec, run.right) || w.held(spec, run.left));
        // A player a springboard holds can't take off (crate::springboard).
        let held = crate::springboard::holder(spec, w, i).is_some();
        // A frame that takes off from the ground is in the air for the run
        // with `takeoff_in_air`, as the jump would start it. A jump taken
        // away (`power.locks`) takes off from nothing.
        let lift = run.takeoff_in_air
            && !held
            && b.on_ground
            && kind.jump.as_ref().is_some_and(|j| {
                (w.pressed(spec, j.action) || b.buffer > 0) && !locked(kind, w, j.action)
            });
        let on_ground = b.on_ground && !lift;
        // Ice replaces the acceleration and deceleration while standing on it.
        let mut run = run.clone();
        if on_ground && let Some((accel, decel)) = spec.levels[w.level].ice {
            let h = kind.tile_hitbox;
            let (cx, _) = middle(kind, &b);
            let row = b.y.floor() + h.y.floor() + h.h.floor();
            if w.tile_at(spec, cx, row) == Tile::Ice {
                run.acceleration = Some(accel);
                run.run_acceleration = None;
                run.deceleration = Some(decel);
                run.skid_deceleration = None;
            }
        }
        // Frames walked from standing, for the slow first steps. A frame
        // that takes off isn't one.
        let takeoff = !held
            && kind
                .jump
                .as_ref()
                .is_some_and(|j| w.pressed(spec, j.action) && !locked(kind, w, j.action));
        if !dir_held {
            b.steps = 0;
        } else if b.on_ground && !takeoff {
            b.steps = b.steps.saturating_add(1);
        }
        // The speed with its fraction below a step, and the way the body
        // faced and last moved, as the frame starts.
        let whole = |v: Fx| match run.move_step {
            Some(step) => Fx::from_raw(v.raw().div_euclid(step.raw()) * step.raw()),
            None => v,
        };
        let speed = |v: Fx| whole(v).max(whole(v).neg());
        let mut v = match run.move_step {
            Some(_) => b.vx.add(b.vx_frac),
            None => b.vx,
        };
        // The speed the tests below read: as the frame starts, or with
        // `speed_before_walls` as the run last left it.
        let seen = if run.speed_before_walls {
            b.seen_speed
        } else {
            speed(v)
        };
        let way = |left: bool| if left { 2 } else { 1 };
        let faced = if b.faced_both { 3 } else { way(b.facing_left) };
        let moved = b.moved;
        let dir = if !lone || both_left {
            None
        } else if w.held(spec, run.right) {
            Some(false)
        } else if w.held(spec, run.left) {
            Some(true)
        } else {
            None
        };
        // The way held; with `face_both`, both directions are a way of
        // their own, 3, which is neither way alone.
        let both = run.face_both && lone && w.held(spec, run.right) && w.held(spec, run.left);
        let held_way = if both { 3 } else { dir.map_or(0, way) };
        let with_motion = dir.is_some() && moved == held_way;
        // With `push_masks`, a direction held toward the side the body was
        // pushed out of last frame changes the speed as nothing held
        // would, in the air too (ImposeFriction masked by
        // Player_CollisionBits); the facing and the run's counts still see
        // it held. The side is found again this frame.
        let masked = run.push_masks && dir.is_some_and(|left| b.pushed == if left { 1 } else { 2 });
        b.pushed = 0;
        let speed_dir = if masked { None } else { dir };
        // When the run action counts, and its hold after it's let go: the
        // way held (none, too) must be the way it last moved (or not yet),
        // and the hold is set only on the ground, not on a takeoff.
        b.run_hold = b.run_hold.saturating_sub(1);
        let run_ok = !run.run_with_motion || held_way == moved;
        let pressed = run.run.is_some_and(|(_, a)| w.held(spec, a)) && run_ok;
        if pressed && run.run_hold_frames > 0 && b.on_ground && !takeoff {
            b.run_hold = run.run_hold_frames;
        }
        // In the air with `air_run_from` the speed alone says whether the
        // run counts.
        let running = match run.air_run_from {
            Some(from) if !on_ground => seen >= from,
            _ => pressed || (b.run_hold > 0 && run_ok),
        };
        // The walking rate by speed, from the hold as the frame starts.
        let band = if running || run.decel_bands.is_empty() {
            None
        } else if b.decel_hold {
            run.decel_bands.last().map(|b| b.1)
        } else {
            run.decel_bands
                .iter()
                .rev()
                .find(|(from, _)| seen >= *from)
                .map(|b| b.1)
        };
        // The hold and a slow skid's turn change only on the ground, and
        // not on a frame that jumps.
        let grounded = b.on_ground && !takeoff;
        if let Some(from) = run.decel_hold_from
            && grounded
        {
            if seen >= from {
                b.decel_hold = true;
            } else if with_motion {
                b.decel_hold = false;
            }
        }
        let doubled = run.skid_last_motion && faced != moved;
        let rate_of = |r: Option<Fx>| {
            let r = band.or(r);
            if doubled { r.map(|r| r.add(r)) } else { r }
        };
        // A slow skid turns at once: the speed is 0 and the body has last
        // moved the way it faced. A direction held steers, and so does an
        // action of `skid_turn_with` held: then no direction, or one that
        // isn't the way it last moved, turns it.
        let mut last = moved;
        let steered = dir.is_some() || run.skid_turn_with.iter().any(|a| w.held(spec, *a));
        if let Some(below) = run.skid_turn_below
            && steered
            && grounded
            && held_way != moved
            && seen < below
        {
            v = Fx::ZERO;
            last = faced;
        }
        let starting = run
            .start
            .filter(|(_, frames)| b.on_ground && dir_held && !takeoff && b.steps <= *frames);
        let air = run
            .air_speed
            .filter(|_| !on_ground && run.air_control && dir_held);
        // With `face_on_ground` the facing stays as it was in the air.
        let turns = on_ground || !run.face_on_ground;
        if let Some(s) = starting.map(|s| s.0).or(air) {
            let left = !w.held(spec, run.right);
            if turns {
                b.facing_left = left;
            }
            v = if left { s.neg() } else { s };
        } else if run.air_control || on_ground || masked {
            let top = match run.run {
                Some((speed, _)) if running => speed,
                _ => run.walk_speed,
            };
            if masked
                && turns
                && let Some(left) = dir
            {
                b.facing_left = left;
            }
            v = match speed_dir {
                Some(left) => {
                    if turns {
                        b.facing_left = left;
                    }
                    let target = if left { top.neg() } else { top };
                    let against = if left { v > Fx::ZERO } else { v < Fx::ZERO };
                    let past_top = !against && v.max(v.neg()) > top;
                    let accel = match run.run_acceleration {
                        Some(r) if running => Some(r),
                        _ => run.acceleration,
                    };
                    let rate = if against && on_ground && !run.skid_last_motion {
                        rate_of(run.skid_deceleration.or(accel))
                    } else {
                        rate_of(accel)
                    };
                    match rate {
                        // The rate is added whatever the speed, then the
                        // whole steps are capped, the fraction kept.
                        Some(r) if run.cap_whole => {
                            let to = if left { v.sub(r) } else { v.add(r) };
                            let steps = whole(to);
                            if !left && steps >= top {
                                top.add(to.sub(steps))
                            } else if left && steps < top.neg() {
                                top.neg().add(to.sub(steps))
                            } else {
                                to
                            }
                        }
                        _ if past_top => target,
                        rate => approach(v, target, rate),
                    }
                }
                // With nothing held, the speed slows to 0 on the ground and
                // is kept in the air; without a deceleration it stops at
                // once, on the ground and in the air (Moss Hop's player
                // moves only while a direction is held). With a step, it
                // slows until its whole steps are 0 and keeps the rest.
                None => match run.deceleration {
                    None => Fx::ZERO,
                    Some(_) if !on_ground && !masked => v,
                    rate => match (run.move_step, rate_of(rate)) {
                        (Some(_), _) if whole(v) == Fx::ZERO => v,
                        (Some(_), Some(r)) if v > Fx::ZERO => v.sub(r),
                        (Some(_), Some(r)) => v.add(r),
                        (_, rate) => approach(v, Fx::ZERO, rate),
                    },
                },
            };
        }
        // A direction held where the facing turns says whether it faces
        // both ways; the rate above went by the facing as the frame began.
        if run.face_both && turns && dir_held {
            b.faced_both = both;
        }
        // Whole steps to move by, and the rest carried over.
        if run.move_step.is_some() {
            b.vx = whole(v);
            b.vx_frac = v.sub(b.vx);
        } else {
            b.vx = v;
        }
        // The speed the run leaves for next frame's tests, before any wall:
        // on the ground, and in the air with a direction held.
        if run.speed_before_walls && (on_ground || dir_held) {
            b.seen_speed = speed(v);
        }
        let tracks = run.skid_last_motion
            || run.skid_turn_below.is_some()
            || run.run_with_motion
            || run.decel_hold_from.is_some();
        if tracks {
            b.moved = if b.vx > Fx::ZERO {
                1
            } else if b.vx < Fx::ZERO {
                2
            } else {
                last
            };
        }
        // A player a springboard holds keeps its place; its speed still
        // changes.
        if held {
            w.bodies[i] = b;
            return;
        }
        // A landing hold (`jump.land_hold`, its frames left in word 2)
        // stands the body still.
        let holding = kind
            .jump
            .as_ref()
            .is_some_and(|j| j.land_hold > 0 && b.ext[LAND_HOLD_WORD] > 0);
        if holding {
            b.vx = Fx::ZERO;
        }
        // `face_way`: the facing is the way the game keeps, which a
        // takeoff takes from a lone direction held and a landing hold
        // doesn't change.
        // In the air it faces the way it moves, which only a turn at the
        // view's edge changes (`camera.keep_in_view_turn`): the frame after
        // the turn, as Donkey Kong's jump tests the edge before it moves
        // ($D55A; hammer clip 13, frames 1902 and 1903).
        if both_left && b.on_ground && !holding {
            b.facing_left = true;
        }
        if run.face_way {
            if holding {
                b.facing_left = facing_before;
            } else if lift && let Some(left) = dir {
                // A takeoff at the view's edge, toward it, turns at once:
                // Donkey Kong's jump tests the edge before its first move
                // ($D547 calls $D990), so a running jump from x 229 with
                // Right held faces left on its first frame (dk.walk-sweep.003,
                // frame 1307).
                b.facing_left = left != turns_at_bound(spec, w, &b, left);
            } else if !b.on_ground && !b.climbing && b.vx != Fx::ZERO {
                b.facing_left = b.vx < Fx::ZERO;
            }
        }
        let dx = if run.step_cycle.is_empty() {
            b.vx
        } else {
            cycle_move(&run, dir, on_ground, lift, holding, &mut b)
        };
        let tiles = w.tiles(spec);
        move_x(spec, tiles, &mut b, dx);
        if run.wall_snap && dir_held && kind.tile_collision {
            let h = kind.tile_hitbox;
            let left = !w.held(spec, run.right);
            let x = b.x.floor() + h.x.floor();
            let edge = if left { x - 1 } else { x + h.w.floor() };
            if column_solid_t(tiles, kind, &b, edge) {
                b.x = Fx::from_int(b.x.floor());
            }
        }
        w.bodies[i] = b;
    }
}

/// The mechanic word `run.step_cycle` counts in.
pub const STEP_WORD: usize = 0;
/// The mechanic word `run.air_step_cycle` counts in.
pub const AIR_STEP_WORD: usize = 1;
/// The mechanic word `jump.land_hold` counts its frames left in.
pub const LAND_HOLD_WORD: usize = 2;
/// The mechanic word `jump.fatal_drop` keeps the takeoff height in.
pub const TAKEOFF_Y_WORD: usize = 3;
/// The takeoff word once `jump.fatal_drop_in_air` has found the drop too
/// far: the landing kills. A state mapping gives the same number for the
/// original's mark (Donkey Kong's $95 at $FF).
pub const FATAL_MARK: i64 = -1000;

/// One step of a cycle on the body's step counter (mechanic word 0):
/// the counter advances by one, back to 0 at the cycle's length, and the
/// cycle's entry there is the whole pixels to move. Every cycle that
/// steps the counter (`run.step_cycle`, a climb's) has the same length.
pub fn cycle_step(b: &mut Body, cycle: &[i64]) -> i64 {
    cycle_word(b, STEP_WORD, cycle)
}

fn cycle_word(b: &mut Body, word: usize, cycle: &[i64]) -> i64 {
    if cycle.is_empty() {
        return 0;
    }
    let n = (b.ext[word] + 1).rem_euclid(cycle.len() as i64);
    b.ext[word] = n;
    cycle[n as usize]
}

/// The move across of a body with `run.step_cycle`, in whole pixels:
/// on the ground with a direction held, the step cycle's next entry that
/// way; standing, none. In the air and on a landing hold the counter still
/// advances, and in the air with `air_step_cycle` the body moves by that
/// cycle the way it took off, which the takeoff frame (`lift`) sets in its
/// speed from the way held.
fn cycle_move(
    run: &crate::spec::Run,
    dir: Option<bool>,
    on_ground: bool,
    lift: bool,
    holding: bool,
    b: &mut Body,
) -> Fx {
    let signed = |left: bool, n: i64| Fx::from_int(if left { -n } else { n });
    if holding {
        cycle_step(b, &run.step_cycle);
        return Fx::ZERO;
    }
    if on_ground {
        return match dir {
            Some(left) => {
                let n = cycle_step(b, &run.step_cycle);
                signed(left, n)
            }
            None => Fx::ZERO,
        };
    }
    cycle_step(b, &run.step_cycle);
    if run.air_step_cycle.is_empty() || run.air_control {
        return b.vx;
    }
    if lift {
        b.vx = match dir {
            Some(true) => run.walk_speed.neg(),
            Some(false) => run.walk_speed,
            None => Fx::ZERO,
        };
    }
    if b.vx == Fx::ZERO {
        return Fx::ZERO;
    }
    let n = cycle_word(b, AIR_STEP_WORD, &run.air_step_cycle);
    signed(b.vx < Fx::ZERO, n)
}

/// The gravity and fall limit for a body: the level's water values while
/// its hitbox's middle is in a water tile, else the ones given.
fn in_water(spec: &Spec, w: &World, b: &Body, gravity: Fx, max_fall: Fx) -> (Fx, Fx) {
    match spec.levels[w.level].water {
        Some(water) => {
            let (cx, cy) = middle(spec.kind(b), b);
            if w.tile_at(spec, cx, cy) == Tile::Water {
                water
            } else {
                (gravity, max_fall)
            }
        }
        None => (gravity, max_fall),
    }
}

/// `v` moved toward `target` by `rate`, or set to it without a rate.
fn approach(v: Fx, target: Fx, rate: Option<Fx>) -> Fx {
    match rate {
        None => target,
        Some(rate) if v < target => v.add(rate).min(target),
        Some(rate) => v.sub(rate).max(target),
    }
}

/// Moves a body `dx` pixels across, a whole pixel at a time when it
/// collides with tiles.
fn move_x(spec: &Spec, tiles: Tiles<'_>, b: &mut Body, dx: Fx) {
    move_x_tiles(spec, tiles, b, dx);
    if dx != Fx::ZERO {
        crate::world::follow_surface(tiles, spec.kind(b), b);
    }
}

fn move_x_tiles(spec: &Spec, tiles: Tiles<'_>, b: &mut Body, dx: Fx) {
    let kind = spec.kind(b);
    let level = tiles.level;
    let target = b.x.add(dx);
    if !kind.tile_collision {
        b.x = target;
        return;
    }
    // With wall points the move doesn't stop at tiles ([`walls`]); it
    // stays in the level.
    if kind.run.as_ref().is_some_and(|r| r.walls.is_some()) {
        let max_x = Fx::from_int(level.pixels()[0] - kind.size[0].floor());
        b.x = target.max(Fx::ZERO).min(max_x);
        return;
    }
    let max_x = level.pixels()[0] - kind.size[0].floor();
    let (mut px, to) = (b.x.floor(), target.floor());
    while px != to {
        let next = if to > px { px + 1 } else { px - 1 };
        let probe = Body {
            x: Fx::from_int(next),
            ..*b
        };
        let edge = if to > px {
            next + kind.tile_hitbox.x.floor() + kind.tile_hitbox.w.floor() - 1
        } else {
            next + kind.tile_hitbox.x.floor()
        };
        if next < 0 || next > max_x || column_solid_t(tiles, kind, &probe, edge) {
            b.x = Fx::from_int(px);
            b.vx = Fx::ZERO;
            return;
        }
        px = next;
    }
    b.x = target;
}

/// Jumping, gravity and landing. The jump action pressed while standing
/// (or within `coyote_frames` of walking off a ledge, or with a press
/// remembered for `buffer_frames` in the air) sets the vertical speed to the
/// jump speed. With `bands`, the first band whose `below` is above the
/// horizontal speed at the start of the frame (`takeoff_speed`, without its
/// sign) gives the speed and the gravities for the whole jump. Gravity is
/// added every frame (the held gravity in the held phase, from takeoff until
/// the action is let go or the entity stops rising), capped at the fall
/// limit, and the speed moves the entity ([`vertical`]).
pub fn fall(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, takeoff_speed: Fx) {
    let mut b = w.bodies[i];
    let kind = spec.kind(&b);
    let Some(jump) = &kind.jump else { return };
    if !b.active() || f.out[i] {
        return;
    }
    let pressed = w.pressed(spec, jump.action) && !locked(kind, w, jump.action);
    // A landing hold (`jump.land_hold`) takes no jump.
    let holding = jump.land_hold > 0 && b.ext[LAND_HOLD_WORD] > 0;
    let can = !holding && (b.on_ground || (jump.coyote_frames > 0 && b.air <= jump.coyote_frames));
    let took_off = can && (pressed || b.buffer > 0);
    if took_off {
        let speed = takeoff_speed.max(takeoff_speed.neg());
        let band = jump.bands.iter().position(|band| speed < band.below);
        b.band = band.map_or(0, |n| n as u8 + 1);
        b.vy = band.map_or(jump.speed, |n| jump.bands[n].speed);
        if jump.keep_fall_gravity {
            b.fall_gravity = band.map_or(jump.gravity, |n| jump.bands[n].gravity);
        }
        if jump.land_keeps_fraction {
            b.y = Fx::from_int(b.y.floor());
        }
        if jump.fatal_drop.is_some() {
            b.ext[TAKEOFF_Y_WORD] = b.y.floor();
        }
        if let Some(frac) = jump.takeoff_fraction {
            b.y = Fx::from_int(b.y.floor()).add(frac);
        }
        b.on_ground = false;
        b.air = u32::MAX;
        b.buffer = 0;
        b.held_phase = true;
    } else if pressed && !b.on_ground {
        // Remembered through the frame it lands, counted from the press: a
        // press up to `buffer_frames` frames before that frame jumps on the
        // next, the first that starts standing.
        b.buffer = if jump.buffer_frames > 0 {
            jump.buffer_frames.saturating_add(1)
        } else {
            0
        };
    } else {
        b.buffer = b.buffer.saturating_sub(1);
    }
    let (gravity, held) = match b
        .band
        .checked_sub(1)
        .and_then(|n| jump.bands.get(usize::from(n)))
    {
        Some(band) => (band.gravity, band.gravity_held),
        None => (jump.gravity, jump.gravity_held),
    };
    // With `keep_fall_gravity` the gravity outside the held phase is the
    // body's own, kept from its last jump, as Super Mario Bros.'
    // FallingSub and DumpFall use VerticalForceDown, which only InitJS (a
    // jump) and Entrance_GameTimerSetup set.
    let gravity = if jump.keep_fall_gravity {
        b.fall_gravity
    } else {
        gravity
    };
    // The held phase lasts from takeoff until the action is let go or the
    // entity stops rising, and doesn't come back until the next takeoff.
    if !w.held(spec, jump.action) || b.vy >= Fx::ZERO {
        b.held_phase = false;
    }
    let gravity = match held {
        Some(g) if b.held_phase => g,
        _ => gravity,
    };
    let was_ground = b.on_ground;
    let (gravity, max_fall) = in_water(spec, w, &b, gravity, jump.max_fall);
    // A frame that starts standing and doesn't jump (a jump has left the
    // ground above) adds no gravity, so the speed stays 0 and the body
    // stays at its height; the ground under it is still looked for.
    let gravity = if jump.no_gravity_on_ground && was_ground {
        Fx::ZERO
    } else {
        gravity
    };
    // `ledge_fall`: a fall that isn't a jump drops at its fixed speed with
    // no gravity; a frame that starts standing adds none either, so only
    // the ground leaving starts the drop.
    let ledge = jump.ledge_fall.filter(|_| b.air != u32::MAX);
    let gravity = match ledge {
        Some((speed, _, _)) => {
            if !was_ground {
                b.vy = Fx::from_int(speed);
            }
            Fx::ZERO
        }
        None => gravity,
    };
    // `fatal_drop_in_air`: the height this frame's move reaches before a
    // landing puts the body on the ground, as `vertical` moves it.
    let free_y = if jump.move_first {
        b.y.add(b.vy)
    } else {
        b.y.add(fall_limit(
            b.vy.add(gravity),
            max_fall,
            jump.max_fall_from_half,
        ))
    };
    // Platformer `level.pickup_holes`: the holes a ledge fall passes.
    let holes = if ledge.is_some() && !was_ground {
        pickup_holes(spec, w)
    } else {
        Vec::new()
    };
    let walled = vertical(
        spec,
        w.tiles(spec),
        &mut b,
        gravity,
        (max_fall, jump.max_fall_from_half),
        jump.move_first,
        &holes,
    );
    if b.air == u32::MAX && !took_off && !b.on_ground {
        land_rising(spec, w.tiles(spec), &mut b);
    }
    if let Some(drop) = jump.fatal_drop.filter(|_| jump.fatal_drop_in_air) {
        if took_off {
            b.ext[TAKEOFF_Y_WORD] = b.y.floor();
        } else if !was_ground && b.air == u32::MAX && free_y.floor() - b.ext[TAKEOFF_Y_WORD] >= drop
        {
            b.ext[TAKEOFF_Y_WORD] = FATAL_MARK;
        }
    }
    if walled {
        let mark = kind.run.as_ref().is_some_and(|r| r.push_masks);
        feet_wall(f, i, &mut b, mark);
    }
    // A landing that kills: the death fires once the body is stored.
    let mut dies = false;
    if let Some((speed, first, kills)) = ledge {
        if was_ground && !b.on_ground {
            // Off the ledge this frame: the first drop, whole pixels, and
            // the fall's first step of the counter after the walk's. On a
            // level of ground boxes the walk took no step of its own.
            let first = if spec.levels[w.level].ground_boxes {
                speed
            } else {
                first
            };
            b.y = Fx::from_int(b.y.floor() + first);
            b.vy = Fx::from_int(speed);
            if let Some(r) = &kind.run
                && !r.step_cycle.is_empty()
            {
                cycle_step(&mut b, &r.step_cycle);
            }
        } else if !was_ground && b.on_ground && kills {
            dies = true;
        }
    }
    if let Some(hd) = &jump.head {
        head(spec, w, f, i, &mut b, hd);
    }
    // Once the action is let go, a rise faster than the cap is cut to it.
    if let Some(cap) = jump.release_cap
        && !w.held(spec, jump.action)
        && b.vy < cap
    {
        b.vy = cap;
    }
    // A jump's landing starts the hold; each frame of it counts down.
    let landed_jump = b.on_ground && !was_ground && b.air == u32::MAX;
    // `fatal_drop`: a jump landing too far below its takeoff kills, as
    // the hold ends (or at once without one). The test reads only the
    // height and the takeoff word, so a state loaded mid-jump or in the
    // hold decides as the run would have.
    let mut fatal = false;
    if holding {
        b.ext[LAND_HOLD_WORD] -= 1;
        fatal = b.ext[LAND_HOLD_WORD] == 0;
    } else if jump.land_hold > 0 && landed_jump {
        b.ext[LAND_HOLD_WORD] = i64::from(jump.land_hold);
    } else {
        fatal = landed_jump;
    }
    if let Some(drop) = jump.fatal_drop
        && fatal
        && if jump.fatal_drop_in_air {
            b.ext[TAKEOFF_Y_WORD] == FATAL_MARK
        } else {
            b.y.floor() - b.ext[TAKEOFF_Y_WORD] >= drop
        }
    {
        dies = true;
    }
    if b.on_ground {
        b.band = 0;
        b.air = 0;
    } else if was_ground {
        b.air = 1;
    } else if b.air != u32::MAX {
        b.air = b.air.saturating_add(1);
    }
    w.bodies[i] = b;
    if dies && !f.out[i] {
        f.out[i] = true;
        emit(spec, w, f, EventKind::PLAYER_DIED, Some(i));
    }
}

/// Platformer `level.ground_land_rising`: a jump in the air after its
/// takeoff frame lands on the first ground box its feet meet, whichever way
/// it moves, as [`vertical`] lands a falling one (Donkey Kong's $D5CC).
/// With `level.ground_bodies` the test starts or ends a ride as well.
fn land_rising(spec: &Spec, tiles: Tiles<'_>, b: &mut Body) {
    let level = tiles.level;
    let (Some(reach), true) = (level.ground_land, level.ground_land_rising) else {
        return;
    };
    let kind = spec.kind(b);
    let h = kind.tile_hitbox;
    let row = b.y.floor() + h.y.floor() + h.h.floor();
    let hit = crate::world::ground_land_hit(tiles, b, row, reach, &[]);
    if !level.ground_bodies.is_empty() {
        crate::world::ride_by(b, hit.map_or(crate::world::GroundHit::None, |h| h.1));
    }
    if let Some((top, _)) = hit {
        let keep = kind.jump.as_ref().is_some_and(|j| j.land_keeps_fraction);
        let frac = if keep { Fx(b.y.frac()) } else { Fx::ZERO };
        b.y = Fx::from_int(top - h.y.floor() - h.h.floor()).add(frac);
        b.vy = Fx::ZERO;
        b.on_ground = true;
    }
}

/// Platformer `level.ground_bodies`: the bodies riding body `e` move by
/// its step since it stood at `old`.
fn carry_riders(spec: &Spec, w: &mut World, e: usize, old: Body) {
    let level = &spec.levels[w.level];
    let Some(nb) = w.bodies.get(e).copied() else {
        return;
    };
    if level.ground_bodies.is_empty() || !level.ground_bodies.iter().any(|(k, _)| *k == nb.kind) {
        return;
    }
    let (dx, dy) = (nb.x.sub(old.x), nb.y.sub(old.y));
    if dx == Fx::ZERO && dy == Fx::ZERO {
        return;
    }
    for r in w.bodies.iter_mut() {
        if r.riding == e as u32 + 1 && r.active() {
            r.x = r.x.add(dx);
            r.y = r.y.add(dy);
        }
    }
}

/// Gravity and landing for one body: `gravity` is added to the vertical
/// speed, capped at the fall limit, and the speed moves it; with
/// `move_first` the old speed moves it and gravity comes after. The limit
/// is `(max_fall, from_half)`: with `from_half` it acts only once the
/// speed's whole part is at least `max_fall` and its fraction at least one
/// half ([`fall_limit`]). Moving down (or not at all) into solid ground
/// lands it on the ground's top; moving up into a solid tile stops it under
/// the tile. A ledge fall doesn't land on the ground where `holes` are
/// (platformer `level.pickup_holes`, [`crate::world::ground_land_row`]).
fn vertical(
    spec: &Spec,
    tiles: Tiles<'_>,
    b: &mut Body,
    gravity: Fx,
    (max_fall, from_half): (Fx, bool),
    move_first: bool,
    holes: &[[i64; 4]],
) -> bool {
    let kind = spec.kind(b);
    let level = tiles.level;
    let h = kind.tile_hitbox;
    // The row just below the hitbox before the move, so a top-only tile
    // stops only a body that was above it.
    let from = b.y.floor() + h.y.floor() + h.h.floor();
    // Platformer `level.ground`: a body on the ground whose feet meet a box
    // on that row stands there; off the boxes the tiles decide. One that
    // rides a ground body (`level.ground_bodies`) stands without a test.
    if b.on_ground
        && (crate::world::ground_at(tiles, b, from).is_some()
            || crate::world::rides_ground(tiles, b))
    {
        b.vy = Fx::ZERO;
        return false;
    }
    let moved = if move_first {
        let v = b.vy;
        b.y = b.y.add(v);
        b.vy = fall_limit(b.vy.add(gravity), max_fall, from_half);
        v
    } else {
        b.vy = fall_limit(b.vy.add(gravity), max_fall, from_half);
        b.y = b.y.add(b.vy);
        b.vy
    };
    // With surfaces a body that stood on the ground stays on it while any
    // column of its tile hitbox does ([`crate::world::stands_row`]); one in
    // the air lands by its feet.
    let standing = b.on_ground && !level.surfaces.is_empty();
    let lands = |b: &Body, row: i64| {
        if standing {
            crate::world::stands_row(tiles, kind, b, row, from)
        } else {
            lands_row(tiles, kind, b, row, from)
        }
    };
    b.on_ground = false;
    if !meets_tiles(kind, b) {
        return false;
    }
    let ts = level.tile_size;
    // Platformer `level.ground_land`: a jump falling lands by the ground's
    // boxes alone, on the first its feet meet a few rows up, and a ledge
    // fall (`jump.ledge_fall`) lands the same way where it is.
    let jumping = b.air == u32::MAX;
    let ledge = !jumping && kind.jump.as_ref().is_some_and(|j| j.ledge_fall.is_some());
    if let Some(reach) = level.ground_land
        && moved >= Fx::ZERO
        && !standing
        && (jumping || ledge)
    {
        let row = b.y.floor() + h.y.floor() + h.h.floor();
        let holes = if ledge && !jumping { holes } else { &[] };
        let hit = crate::world::ground_land_hit(tiles, b, row, reach, holes);
        if !level.ground_bodies.is_empty() {
            crate::world::ride_by(b, hit.map_or(crate::world::GroundHit::None, |h| h.1));
        }
        if let Some((top, _)) = hit {
            if jumping {
                let keep = kind.jump.as_ref().is_some_and(|j| j.land_keeps_fraction);
                let frac = if keep { Fx(b.y.frac()) } else { Fx::ZERO };
                b.y = Fx::from_int(top - h.y.floor() - h.h.floor()).add(frac);
            }
            b.vy = Fx::ZERO;
            b.on_ground = true;
        }
        return false;
    }
    if moved >= Fx::ZERO {
        let row = b.y.floor() + h.y.floor() + h.h.floor();
        let depth = kind.jump.as_ref().and_then(|j| j.land_depth);
        if lands(b, row) && depth.is_some_and(|d| row.rem_euclid(ts) >= d) {
            // Too deep to land: the feet are in a wall ([`feet_wall`]).
            return true;
        }
        if lands(b, row) {
            let left = b.x.floor() + h.x.floor();
            let top = if level.surfaces.is_empty() {
                row.div_euclid(ts) * ts
            } else {
                // With `feet` the surface is the one under them.
                match kind.jump.as_ref().and_then(|j| j.feet.as_ref()) {
                    Some(feet) if !standing => {
                        tiles.land_top(feet.iter().map(|f| b.x.floor() + f.floor()), row, from)
                    }
                    _ => tiles.land_top(left..left + h.w.floor(), row, from),
                }
            };
            // With `land_keeps_fraction` the height's fraction stays.
            let keep = kind.jump.as_ref().is_some_and(|j| j.land_keeps_fraction);
            let frac = if keep { Fx(b.y.frac()) } else { Fx::ZERO };
            b.y = Fx::from_int(top - h.y.floor() - h.h.floor()).add(frac);
            b.vy = Fx::ZERO;
            b.on_ground = true;
        }
    } else if kind.jump.as_ref().is_none_or(|j| j.head.is_none()) {
        // A body with a head point is stopped by it instead ([`head`]).
        let row = b.y.floor() + h.y.floor();
        if row_solid_t(tiles, kind, b, row) {
            let bottom = (row.div_euclid(ts) + 1) * ts;
            b.y = Fx::from_int(bottom - h.y.floor());
            b.vy = Fx::ZERO;
        }
    }
    false
}

/// Whether a body meets tiles where it is: with `tile_collision`, and with
/// `jump.collide_y` only while its whole y is in that band (Super Mario
/// Bros.' PlayerBGCollision leaves at once below $cf or off the screen).
fn meets_tiles(kind: &crate::spec::Kind, b: &Body) -> bool {
    kind.tile_collision
        && kind
            .jump
            .as_ref()
            .and_then(|j| j.collide_y)
            .is_none_or(|[from, to]| (from..to).contains(&b.y.floor()))
}

/// Feet too deep in a tile to land (`land_depth`): the body is impeded
/// across the way it last moved, as against a wall on that side, and no
/// wall points are tested this frame, as Super Mario Bros.' feet check
/// jumps to ImpedePlayerMove with Player_MovingDir and leaves
/// PlayerBGCollision. Moving right last, it moves a pixel left unless its
/// speed takes it left; otherwise a pixel right unless its speed takes it
/// right. Its speed across becomes 0.
fn feet_wall(f: &mut Frame, i: usize, b: &mut Body, mark: bool) {
    f.feet_walled.push(i);
    let side: i64 = if b.moved == 1 { -1 } else { 1 };
    // The side is marked even when the body moves away from the wall, as
    // ImpedePlayerMove clears the collision bit either way.
    if mark {
        b.pushed = if side > 0 { 1 } else { 2 };
    }
    let away = if side < 0 {
        b.vx < Fx::ZERO
    } else {
        b.vx > Fx::ZERO
    };
    if away {
        return;
    }
    b.vx = Fx::ZERO;
    b.x = b.x.add(Fx::from_int(side));
    f.pushed.push((i, side));
}

/// Pickup points, after the frame's moves: the first point in a cell of the
/// pickup's kind changes the cell and fires its event, and no wall points
/// are tested this frame, as Super Mario Bros.' PlayerBGCollision leaves
/// through HandleCoinMetatile at the first coin it finds, one a frame.
fn pickup(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) {
    let b = w.bodies[i];
    let kind = spec.kind(&b);
    let Some(p) = kind.jump.as_ref().and_then(|j| j.pickup.as_ref()) else {
        return;
    };
    if !b.active() || !meets_tiles(kind, &b) {
        return;
    }
    let found = p
        .points
        .iter()
        .map(|q| (b.x.floor() + q[0].floor(), b.y.floor() + q[1].floor()))
        .find(|&(x, y)| w.tile_at(spec, x, y) == p.tile);
    let Some((x, y)) = found else { return };
    w.set_tile(spec, x, y, p.becomes);
    f.feet_walled.push(i);
    if let Some(e) = p.event {
        emit(spec, w, f, e, Some(i));
    }
}

/// Platformer `level.pickups`: after player `i`'s moves, on a frame it
/// moved across (from `from_x`), was in a jump or its landing hold as the
/// frame started (`from_jump`) or is in one now, and unless it is out or
/// falling off a ledge, its place less a group's `offset` touches the
/// first place, group by group in order, whose x is its x and whose y is
/// within the group's `window` of its y: a place there is taken (its
/// counter 1, the group's event fired); a taken place with a `hole`
/// (`level.pickup_holes`) starts the player's fall off a ledge on the next
/// frame unless it is in a jump or its landing hold. One touch a frame.
/// Donkey Kong's $EE79, from the play routine after Mario's ($CEE8): the
/// rivets ($EE84-$EED8) then the stage's prizes ($EEF0), on a frame his
/// step beat $BE is set (each step of the walk, every frame of a jump),
/// not while he falls off an edge or dies ($96 = 8 or $FF); a removed
/// rivet's spot sets $96 to 8 unless he is jumping ($EED9), and the fall
/// ($D697) moves him from the next frame.
pub fn pickups(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, from_x: Fx, from_jump: bool) {
    let groups = &spec.levels[w.level].pickups;
    // Platformer `level.pickup_by_hook`: a hook takes those places.
    if groups.iter().all(|g| g.by_hook) || f.out[i] {
        return;
    }
    let b = w.bodies[i];
    let hold = spec
        .kind(&b)
        .jump
        .as_ref()
        .is_some_and(|j| j.land_hold > 0 && b.ext[LAND_HOLD_WORD] > 0);
    let jumping = (b.air == u32::MAX && !b.on_ground) || hold;
    if !b.active() || b.climbing || ledge_falling(spec, w, i) {
        return;
    }
    if b.x.floor() == from_x.floor() && !from_jump && !jumping {
        return;
    }
    for g in groups.iter().filter(|g| !g.by_hook) {
        let (x, y) = (b.x.floor() - g.offset[0], b.y.floor() - g.offset[1]);
        let at = g
            .places
            .iter()
            .zip(&g.counters)
            .find(|(p, _)| p[0] == x && p[1] + g.window[0] <= y && y <= p[1] + g.window[1]);
        let Some((_, &c)) = at else {
            continue;
        };
        if w.counters[c] == 0 {
            w.counters[c] = 1;
            if let Some(e) = g.event {
                emit(spec, w, f, e, Some(i));
            }
        } else if g.hole.is_some() && !jumping && w.bodies[i].on_ground {
            // A fall off a ledge from the next frame: standing with `air`
            // 1, which only this leaves (a frame's fall ends with 0 on the
            // ground), the player system starts it there.
            let b = &mut w.bodies[i];
            b.air = 1;
            b.vy = Fx::ZERO;
        }
        return;
    }
}

/// Platformer `level.pickup_holes`: the holes of the places taken in the
/// level playing, in level pixels.
fn pickup_holes(spec: &Spec, w: &World) -> Vec<[i64; 4]> {
    let mut out = Vec::new();
    for g in &spec.levels[w.level].pickups {
        let Some(h) = g.hole else { continue };
        for (p, &c) in g.places.iter().zip(&g.counters) {
            if w.counters[c] != 0 {
                out.push([p[0] + h[0], p[1] + h[1], p[0] + h[2], p[1] + h[3]]);
            }
        }
    }
    out
}

/// Whether a body's pushes are marked for the next frame (`run.push_masks`).
fn marks_push(kind: &crate::spec::Kind) -> bool {
    kind.run.as_ref().is_some_and(|r| r.push_masks)
}

/// Wall points, after the frame's moves across and up or down, as Super
/// Mario Bros.' PlayerBGCollision checks its sides after the head and the
/// feet: the first point, left ones then right ones, in a cell that stops
/// movement impedes the body (ImpedePlayerMove): on the left unless it
/// moves right, it moves one pixel right and its speed across becomes 0;
/// on the right unless it moves left, one pixel left. Only the first point
/// found counts. A camera `by_move` following the body scrolls by its move
/// before the push (the frame's `pushed`).
fn walls(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) {
    let b = w.bodies[i];
    let kind = spec.kind(&b);
    let Some(wl) = kind.run.as_ref().and_then(|r| r.walls.as_ref()) else {
        return;
    };
    if !b.active() || !meets_tiles(kind, &b) || f.feet_walled.contains(&i) {
        return;
    }
    let at = |p: &[Fx; 2]| {
        w.tile_at(spec, b.x.floor() + p[0].floor(), b.y.floor() + p[1].floor())
            .solid()
    };
    let side = if wl.left.iter().any(at) {
        1
    } else if wl.right.iter().any(at) {
        -1
    } else {
        return;
    };
    impede(spec, w, f, i, side);
}

/// ImpedePlayerMove: body `i` is stopped by something on its left (`side`
/// is 1) or right (-1). Unless it moves away, it moves one pixel away and its
/// speed across becomes 0, which this returns as true.
fn impede(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, side: i64) -> bool {
    let b = w.bodies[i];
    if marks_push(spec.kind(&b)) {
        w.bodies[i].pushed = if side > 0 { 1 } else { 2 };
    }
    let away = if side > 0 {
        b.vx > Fx::ZERO
    } else {
        b.vx < Fx::ZERO
    };
    if away {
        return false;
    }
    let nb = &mut w.bodies[i];
    nb.vx = Fx::ZERO;
    nb.x = nb.x.add(Fx::from_int(side));
    // A camera `by_move` following it scrolls by the move before the
    // push, as Super Mario Bros.' ScrollHandler scrolls by what
    // MovePlayerHorizontally moved.
    f.pushed.push((i, side));
    true
}

/// A push of body `p` by `side` after the camera system has run, as
/// Super Mario Bros.' platforms impede the player after ScrollHandler: the
/// cameras following it keep what the camera system would have, so the
/// frame's scroll stands, the hold starts and the next move leaves out the
/// push.
fn camera_after_push(spec: &Spec, w: &mut World, p: usize, side: i64) {
    let at = |s: System| spec.order.iter().position(|o| *o == s);
    if at(System::Camera) > at(System::Contact) {
        return;
    }
    for i in 0..spec.cameras.len() {
        if w.of_kind(spec.cameras[i].follow).next().map(|(j, _)| j) != Some(p) {
            continue;
        }
        let push_hold = spec.cameras[i].by_move.map_or(0, |m| m.push_hold);
        if push_hold > 0 {
            w.camera_hold
                .resize(spec.cameras.len().max(w.camera_hold.len()), 0);
            w.camera_hold[i] = push_hold;
        }
        if let Some(at) = w.follow_at.get_mut(i) {
            at[0] += side;
        }
        see(spec, w, i);
    }
}

/// A head hit, after the vertical move: while the body rises (its speed
/// below 0), its y is at or below `above` and at least `row_from` pixels
/// into its tile row, the cell its head point is in decides. A cell that
/// stops movement sets the speed's whole part to its hit's `speed` (0
/// when no hit lists its kind), keeping the fraction with `keep_fraction`,
/// fires the hit's event and changes the cell to `becomes`. The body isn't
/// moved, as Super Mario Bros.' PlayerBGCollision moves nothing on a head
/// hit.
fn head(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, b: &mut Body, hd: &Head) {
    let ts = spec.levels[w.level].tile_size;
    let y = b.y.floor();
    if b.vy >= Fx::ZERO
        || hd.above.is_some_and(|a| b.y < a)
        || y.rem_euclid(ts) < hd.row_from
        || !meets_tiles(spec.kind(b), b)
    {
        return;
    }
    let (px, py) = (b.x.floor() + hd.point[0].floor(), y + hd.point[1].floor());
    let cell = w.tile_at(spec, px, py);
    if !cell.solid() {
        return;
    }
    // While `busy_while` holds, the cell is hit as a solid one, and
    // nothing else happens (feature `head.busy_while`).
    let busy = hd
        .busy_while
        .is_some_and(|g| g.cmp.holds(w.counters[g.counter], g.value));
    let as_kind = if busy { Tile::Solid } else { cell };
    let hit = hd.hits.iter().find(|h| h.tile == as_kind);
    let whole = hit.map_or(Fx::ZERO, |h| h.speed);
    b.vy = if hd.keep_fraction {
        whole.add(Fx(b.vy.frac()))
    } else {
        whole
    };
    let Some(hit) = hit.filter(|_| !busy) else {
        return;
    };
    let c = spec.levels[w.level].cell(px, py).map(|c| c as u32);
    if w.held_cells.iter().any(|h| Some(h.0) == c) {
        // A held cell is hit as the solid cell it is, and keeps its count
        // and its next kind.
    } else if hit.hold > 0 {
        // Held: plain solid until its frames are done, then `becomes`, or
        // the kind it was.
        if let Some(c) = c {
            w.held_cells
                .push((c, hit.becomes.unwrap_or(cell), hit.hold + 1));
        }
        w.set_tile(spec, px, py, Tile::Solid);
    } else if let Some(t) = hit.becomes {
        w.set_tile(spec, px, py, t);
    }
    // The cell above: what sits on the block (feature `head.above`).
    if let Some(a) = hit.above
        && w.tile_at(spec, px, py - ts) == a.tile
    {
        w.set_tile(spec, px, py - ts, a.becomes);
        if let Some(e) = a.event {
            w.bodies[i] = *b;
            emit(spec, w, f, e, Some(i));
            *b = w.bodies[i];
        }
    }
    if let Some(e) = hit.event {
        // The event's rules see the body as it is now.
        w.bodies[i] = *b;
        emit(spec, w, f, e, Some(i));
        *b = w.bodies[i];
    }
}

/// A vertical speed held to the fall limit. Without `from_half` it is
/// at most `max_fall`. With it, a speed whose whole part is at least
/// `max_fall` and whose fraction is at least one half is set to `max_fall`,
/// and any other is kept, as Super Mario Bros.' ImposeGravity compares the
/// speed's whole byte with the maximum and its fraction with $80.
pub fn fall_limit(v: Fx, max_fall: Fx, from_half: bool) -> Fx {
    if !from_half {
        return v.min(max_fall);
    }
    let half = Fx::ONE.raw() / 2;
    if Fx::from_int(v.floor()) >= max_fall && v.frac() >= half {
        max_fall
    } else {
        v
    }
}

/// Pits and goals: a player whose top edge is at or below the level's
/// `kill_below` dies; otherwise one whose left edge is at or past the goal
/// reaches it, unless its top edge is at or below the goal's `under`.
pub fn bounds(spec: &Spec, w: &mut World, f: &mut Frame, i: usize) {
    bounds_from(spec, w, f, i, None);
}

/// [`bounds`], for a player whose y was `from` before this frame's move.
/// With the level's `pit_event` (platformer 0.4.9), a player whose top
/// edge reaches `kill_below` from above it fires that event instead of
/// dying, once, and goes on moving.
pub fn bounds_from(spec: &Spec, w: &mut World, f: &mut Frame, i: usize, from: Option<Fx>) {
    let (kill, goal, under, pit_event) = {
        let level = &spec.levels[w.level];
        (
            level.kill_below,
            level.goal_x,
            level.goal_under,
            level.pit_event,
        )
    };
    let b = w.bodies[i];
    if !b.active() || f.out[i] {
        return;
    }
    // A body climbing down through (`climb.down_through`) wraps instead.
    let through = b.climbing
        && spec
            .kind(&b)
            .climb
            .as_ref()
            .is_some_and(|c| c.down_through > 0);
    if through {
        return;
    }
    let below = kill.is_some_and(|k| b.y >= k);
    if let (true, Some(ev)) = (below, pit_event) {
        // A fall fires the level's event once, as the edge first gets
        // there, and touches the goal no more.
        if kill.is_some_and(|k| from.is_some_and(|y| y < k)) {
            emit(spec, w, f, ev, Some(i));
        }
    } else if below {
        f.out[i] = true;
        emit(spec, w, f, EventKind::PLAYER_DIED, Some(i));
    } else if goal.is_some_and(|g| b.x >= g) && !under.is_some_and(|u| b.y >= u) {
        f.out[i] = true;
        emit(spec, w, f, EventKind::GOAL_REACHED, Some(i));
    }
}

/// Patrols: on frames whose number is a multiple of the period, each living
/// patroller steps toward the end it faces, and turns when it gets there.
/// It walks right from its spawn first.
pub fn patrol(spec: &Spec, w: &mut World, _f: &mut Frame, i: usize) {
    let level = &spec.levels[w.level];
    {
        let mut b = w.bodies[i];
        let Some(p) = &spec.kind(&b).patrol else {
            return;
        };
        if !b.active() || p.period == 0 || !w.frame.is_multiple_of(u64::from(p.period)) {
            return;
        }
        let Some(spawn) = level.spawns.get(i) else {
            return;
        };
        if b.facing_left {
            b.x = b.x.sub(p.step);
            if b.x <= spawn.x {
                b.x = spawn.x;
                b.facing_left = false;
            }
        } else {
            b.x = b.x.add(p.step);
            if let Some(end) = spawn.patrol_end
                && b.x >= end
            {
                b.x = end;
                b.facing_left = true;
            }
        }
        w.bodies[i] = b;
    }
}

/// The pixels a body's hitbox covers: left, top, right, bottom, inclusive.
fn hit_pixels(spec: &Spec, b: &Body) -> [i64; 4] {
    let h = spec.kind(b).hitbox;
    let (x, y) = (b.x.floor() + h.x.floor(), b.y.floor() + h.y.floor());
    [x, y, x + h.w.floor() - 1, y + h.h.floor() - 1]
}

/// Touches between players and entities with a contact component, in
/// entity order. A falling player whose bottom row is within the stomp
/// depth of the entity's hitbox top stomps it: the entity dies and the
/// player bounces. Any other touch does the entity's `otherwise`; a touch
/// that kills or hurts the player ends its checks for the frame.
pub fn contact(spec: &Spec, w: &mut World, f: &mut Frame) {
    contact_ext(spec, w, f, &Exts::new());
}

/// [`contact`], running the `ext` hooks of the hitters that move in it.
pub fn contact_ext(spec: &Spec, w: &mut World, f: &mut Frame, exts: &Exts) {
    touches(spec, w, f, None);
    // Hitters with `hits.move_first` move now, before their hits.
    for i in 0..w.bodies.len() {
        if f.level_started || i >= w.bodies.len() {
            break;
        }
        if moves_in_contact(spec, &w.bodies[i]) {
            enemy(spec, w, f, i, exts);
        }
    }
    hits(spec, w, f);
}

/// The touches of [`contact`]: of every entity, or of `only`, one with
/// `after_move` in its own turn.
fn touches(spec: &Spec, w: &mut World, f: &mut Frame, only: Option<usize>) {
    for p in 0..w.bodies.len() {
        if f.level_started || p >= w.bodies.len() {
            break;
        }
        if spec.kinds[w.bodies[p].kind].role != Role::Player {
            continue;
        }
        for e in only.map_or(0..w.bodies.len(), |o| o..o + 1) {
            // A touch's event may have started a level.
            if f.level_started || e >= w.bodies.len() || p >= w.bodies.len() {
                break;
            }
            let (pb, eb) = (w.bodies[p], w.bodies[e]);
            if !pb.active() || f.out[p] {
                break;
            }
            let Some(c) = &spec.kind(&eb).contact else {
                continue;
            };
            // `near` and `after_move`: tested in the entity's own turn.
            if e == p
                || !eb.active()
                || eb.carrier > 0
                || c.near.is_some()
                || c.after_move != only.is_some()
            {
                continue;
            }
            if c.every > 1
                && !w
                    .frame
                    .wrapping_sub(u64::from(c.phase))
                    .is_multiple_of(u64::from(c.every))
            {
                continue;
            }
            let ([pl, pt, pr, pbm], [el, mut et, mut er, mut ebm]) =
                (hit_pixels(spec, &pb), hit_pixels(spec, &eb));
            // `hit_widths`: this level's width for the entity.
            if let Some(&(_, width)) = spec.levels[w.level]
                .hit_widths
                .iter()
                .find(|h| h.0 == eb.kind)
            {
                er = el + width.floor() - 1;
            }
            let riding = pb.riding == e as u32 + 1;
            // `touch_edges`: each box a pixel wider and taller.
            let edge = i64::from(c.touch_edges);
            // `ride`: the first of the entity's boxes the player reaches,
            // and none while the player is out of the tiles' band, as
            // CheckPlayerVertical.
            let mut below = 0;
            if let Some(r) = c.ride {
                let tall = ebm - et;
                let found = [0, r.copy_below]
                    .into_iter()
                    .take(if r.copy_below > 0 { 2 } else { 1 })
                    .find(|&b| {
                        let top = ride_top(&r, et, b);
                        top >= r.above
                            && pbm + edge >= top - i64::from(riding)
                            && top + tall + edge >= pt
                    })
                    .filter(|_| meets_tiles(spec.kind(&pb), &pb));
                let Some(b) = found else {
                    if riding {
                        w.bodies[p].riding = 0;
                    }
                    continue;
                };
                below = b;
                et = ride_top(&r, et, b);
                ebm = et + tall;
            }
            if pr + edge < el
                || er + edge < pl
                || pbm + edge < et - i64::from(riding)
                || ebm + edge < pt
            {
                if riding {
                    w.bodies[p].riding = 0;
                }
                continue;
            }
            // Standing on it: a falling player whose bottom is within the
            // depth of its top lands there, and rides it after.
            if let Some(depth) = c.stand
                && pb.vy >= Fx::ZERO
                && Fx::from_int(pbm + edge) < Fx::from_int(et).add(depth)
            {
                let h = spec.kind(&pb).hitbox;
                if c.ride.is_some() {
                    f.rides.push((p, e, below));
                }
                let nb = &mut w.bodies[p];
                if c.ride.is_none() {
                    nb.y = Fx::from_int(et - h.y.floor() - h.h.floor());
                }
                if riding {
                    nb.x = nb.x.add(eb.vx);
                }
                nb.vy = Fx::ZERO;
                nb.on_ground = true;
                nb.riding = e as u32 + 1;
                if !riding {
                    emit(spec, w, f, EventKind::ENEMY_STOOD_ON, Some(e));
                }
                continue;
            }
            if riding {
                w.bodies[p].riding = 0;
            }
            // `sides` (platformer 0.4.22), as Super Mario Bros.'
            // ProcLPlatCollisions: near the top a falling player isn't at
            // a side; otherwise it is on the left within `left` of that
            // edge, else on the right within `right` of the other.
            if let Some(s) = c.sides
                && meets_tiles(spec.kind(&pb), &pb)
                && !(pb.vy >= Fx::ZERO && pbm + edge - et < s.top)
            {
                let side = if pr + edge - el < s.left {
                    Some(-1)
                } else if (0..s.right).contains(&(er + edge - pl - 1)) {
                    Some(1)
                } else {
                    None
                };
                if let Some(side) = side
                    && impede(spec, w, f, p, side)
                {
                    camera_after_push(spec, w, p, side);
                }
            }
            // An invincible player (platformer 0.4.10): the touch does the
            // entity's `invincible` instead of a stomp or its `otherwise`.
            let invincible = spec
                .kind(&pb)
                .invincible
                .is_some_and(|g| g.cmp.holds(w.counters[g.counter], g.value));
            if invincible {
                touch(spec, w, f, p, e, c.invincible, c.damage);
                continue;
            }
            match c.stomp {
                Some((depth, bounce))
                    if pb.vy > Fx::ZERO && Fx::from_int(pbm) < Fx::from_int(et).add(depth) =>
                {
                    if !c.keep_on_stomp {
                        w.bodies[e].alive = false;
                    }
                    w.bodies[p].vy = bounce;
                    emit(spec, w, f, EventKind::ENEMY_STOMPED, Some(e));
                }
                _ => {
                    f.hitter_slot = c
                        .in_turn
                        .then(|| w.slots.iter().position(|s| s.body == e as u32 + 1))
                        .flatten();
                    touch(spec, w, f, p, e, c.otherwise, c.damage);
                    f.hitter_slot = None;
                }
            }
        }
    }
}

/// What a touch between player `p` and entity `e` does.
fn touch(spec: &Spec, w: &mut World, f: &mut Frame, p: usize, e: usize, t: Touch, damage: i64) {
    let (pb, eb) = (w.bodies[p], w.bodies[e]);
    match t {
        Touch::KillPlayer => {
            f.out[p] = true;
            emit(spec, w, f, EventKind::PLAYER_DIED, Some(p));
        }
        Touch::HurtPlayer => hurt_player(spec, w, f, p, damage, Some(e)),
        Touch::Collect => {
            w.bodies[e].alive = false;
            emit(spec, w, f, EventKind::ITEM_COLLECTED, Some(e));
        }
        Touch::Kick => {
            // The kicked entity turns to face away from the player.
            w.bodies[e].facing_left = pb.x > eb.x;
            emit(spec, w, f, EventKind::ENEMY_KICKED, Some(e));
        }
        Touch::Defeat => {
            w.bodies[e].alive = false;
            emit(spec, w, f, EventKind::ENEMY_DEFEATED, Some(e));
            leave_defeated(spec, w, f, e);
        }
        Touch::Nothing => {}
    }
}

/// What body `e`, defeated, leaves in its place (`health.defeat`,
/// platformer 0.4.12): a body that keeps its numbered slot until it goes,
/// as Super Mario Bros.' ShellOrBlockDefeat leaves an enemy in its slot
/// with Enemy_State's d5 set.
fn leave_defeated(spec: &Spec, w: &mut World, f: &mut Frame, e: usize) {
    if f.level_started || e >= w.bodies.len() {
        return;
    }
    let old = w.bodies[e];
    let Some(d) = spec.kind(&old).health.as_ref().and_then(|h| h.defeat) else {
        return;
    };
    replace_by(spec, w, f, e, d.becomes);
    if let Some(b) = w.bodies.last_mut() {
        b.y = old.y.add(d.rise.neg());
        b.vy = Fx::from_int(d.speed.floor()).add(Fx(old.vy.frac()));
        b.on_ground = false;
    }
}

/// Hits: each active entity with `hits`, in order, against the first
/// active entity it touches whose kind it targets and that has its own
/// health. A shield takes nothing; otherwise the target takes the damage
/// (its `damage_from` for the hitter's kind, or the hitter's own) and
/// `enemy.hit` fires for it, and one with no hit points left leaves and
/// fires `enemy.defeated`. The hitter leaves unless it keeps.
pub fn hits(spec: &Spec, w: &mut World, f: &mut Frame) {
    for a in 0..w.bodies.len() {
        if f.level_started || a >= w.bodies.len() {
            break;
        }
        let ab = w.bodies[a];
        let Some(hit) = spec.kind(&ab).hits.clone() else {
            continue;
        };
        if !ab.active() || ab.carrier > 0 {
            continue;
        }
        if hit.every > 1
            && !w
                .frame
                .wrapping_sub(u64::from(hit.phase))
                .is_multiple_of(u64::from(hit.every))
        {
            continue;
        }
        let [al, at, ar, abm] = hit_pixels(spec, &ab);
        for t in 0..w.bodies.len() {
            let tb = w.bodies[t];
            if t == a || !tb.active() || tb.carrier > 0 || !hit.targets.contains(&tb.kind) {
                continue;
            }
            let Some(health) = spec.kind(&tb).health.clone() else {
                continue;
            };
            let [tl, tt, tr, tbm] = hit_pixels(spec, &tb);
            if ar < tl || tr < al || abm < tt || tbm < at {
                continue;
            }
            if !hit.keep {
                w.bodies[a].alive = false;
            }
            if health.shield || tb.invuln > 0 {
                break;
            }
            let damage = health
                .damage_from
                .iter()
                .find(|(k, _)| *k == ab.kind)
                .map_or(hit.damage, |(_, d)| *d);
            w.bodies[t].hp += damage;
            w.bodies[t].invuln = health.invulnerable;
            emit(spec, w, f, EventKind::ENEMY_HIT, Some(t));
            if !f.level_started && t < w.bodies.len() && w.bodies[t].hp >= health.points {
                w.bodies[t].alive = false;
                emit(spec, w, f, EventKind::ENEMY_DEFEATED, Some(t));
                leave_defeated(spec, w, f, t);
            }
            break;
        }
    }
}

/// Whether body `b`, moving left or right, is at or past a bound of a
/// `keep_in_view_turn` camera that follows it, on the side it moves
/// toward: a jump that takes off there turns around on its first frame.
fn turns_at_bound(spec: &Spec, w: &World, b: &Body, left: bool) -> bool {
    spec.cameras.iter().enumerate().any(|(i, cam)| {
        if !cam.keep_in_view_turn || cam.follow != b.kind {
            return false;
        }
        let Some([lo, hi]) = bounds_now(spec, w, b, i) else {
            return false;
        };
        let at = b.x.floor() - w.cameras.get(i).map_or(0, |c| c[0].floor());
        if left { at <= lo } else { at >= hi }
    })
}

/// A camera's `keep_in_view` (core 0.4.12): its followed entity's whole
/// place in the view across stays within the camera's two bounds. Past
/// one, it is put back at it with its fraction kept, and the whole steps
/// of its speed across become 0 unless only the direction away from that
/// edge is held. With `keep_in_view_touch` (core 0.4.17) a place at the
/// first bound counts as past it. A camera `by_move` with `keep_move`
/// measures the next move from there.
fn keep_in_view(spec: &Spec, w: &mut World, i: usize) {
    let Some((j, b)) = w.of_kind(spec.cameras[i].follow).next() else {
        return;
    };
    let mut b = *b;
    let Some([lo, hi]) = bounds_now(spec, w, &b, i) else {
        return;
    };
    let left = w.cameras[i][0].floor();
    let at = b.x.floor() - left;
    // With `keep_in_view_turn`, in the air at a bound or past it and moving
    // toward it, the entity turns around: its speed across reversed, its
    // place put back at the bound (Donkey Kong's jump at the screen edge),
    // less what it went past. Donkey Kong's jump tests the bound before it
    // moves ($D547 calls $D990) and moves the other way on the same frame,
    // so a jump that takes off at x 229 toward it lands its first frame at
    // 228 (wallW's wall-jump clips, 2026-11-04).
    if spec.cameras[i].keep_in_view_turn && !b.on_ground {
        let toward = if at >= hi {
            (b.vx > Fx::ZERO).then_some(hi)
        } else if at <= lo {
            (b.vx < Fx::ZERO).then_some(lo)
        } else {
            None
        };
        if let Some(edge) = toward {
            b.x = Fx::from_int(left + 2 * edge - at).add(Fx(b.x.frac()));
            b.vx = b.vx.neg();
            w.bodies[j] = b;
            if spec.cameras[i].by_move.is_some_and(|m| m.keep_move) && i < w.follow_at.len() {
                w.follow_at[i] = [b.x.floor(), b.y.floor()];
            }
            return;
        }
    }
    let (edge, away) = if at < lo || (at == lo && spec.cameras[i].keep_in_view_touch) {
        (lo, true)
    } else if at > hi {
        (hi, false)
    } else {
        return;
    };
    b.x = Fx::from_int(left + edge).add(Fx(b.x.frac()));
    // Only the way back into the view, held alone, keeps the speed.
    let held_away = spec.kind(&b).run.as_ref().is_some_and(|r| {
        let (r_, l_) = (w.held(spec, r.right), w.held(spec, r.left));
        if away { r_ && !l_ } else { l_ && !r_ }
    });
    if !held_away {
        b.vx = Fx::ZERO;
    }
    w.bodies[j] = b;
    if spec.cameras[i].by_move.is_some_and(|m| m.keep_move) && i < w.follow_at.len() {
        w.follow_at[i] = [b.x.floor(), b.y.floor()];
    }
}

/// Body `b`'s whole x as the first `keep_in_view` camera that follows it
/// will leave it this frame (`keep_in_view`, which runs after the hooks):
/// held at a bound, or with `keep_in_view_turn` in the air sent back from
/// it by what it went past. A hook that runs before it reads its place
/// here.
pub fn kept_x(spec: &Spec, w: &World, b: &Body) -> i64 {
    let x = b.x.floor();
    let Some((i, cam)) = spec
        .cameras
        .iter()
        .enumerate()
        .find(|(_, c)| c.keep_in_view.is_some() && c.follow == b.kind)
    else {
        return x;
    };
    let [lo, hi] = bounds_now(spec, w, b, i).unwrap_or_default();
    let left = w.cameras.get(i).map_or(0, |c| c[0].floor());
    let at = x - left;
    if cam.keep_in_view_turn && !b.on_ground {
        if at > hi && b.vx > Fx::ZERO {
            return left + 2 * hi - at;
        }
        if at < lo && b.vx < Fx::ZERO {
            return left + 2 * lo - at;
        }
    }
    x.clamp(left + lo, left + hi)
}

/// The level's own bounds in the view across (feature
/// `level.camera.keep_in_view`), which replace every camera's
/// `keep_in_view` and its rows while the level plays.
pub fn level_keep_in_view(spec: &Spec, w: &World) -> Option<[i64; 2]> {
    spec.levels
        .get(w.level)
        .and_then(|l| l.camera.as_ref())
        .and_then(|c| c.keep_in_view)
}

/// A camera's `keep_in_view` bounds for body `b`: those of the first of
/// `keep_in_view_rows` its row is in (camera.keep_in_view_rows), else
/// `bounds`. The row is its y on the ground, and in the air the height
/// it took off from when its jump keeps it (`jump.fatal_drop`'s word),
/// as Donkey Kong's $59 keeps the girder a jump left.
pub fn keep_bounds(spec: &Spec, b: &Body, i: usize, bounds: [i64; 2]) -> [i64; 2] {
    rows_bounds(spec, b, &spec.cameras[i].keep_in_view_rows, bounds)
}

/// The bounds for body `b` while the level plays: the level's own
/// (`level.camera.keep_in_view`, by its rows), else camera `i`'s by its
/// rows ([`keep_bounds`]), or None with neither.
fn bounds_now(spec: &Spec, w: &World, b: &Body, i: usize) -> Option<[i64; 2]> {
    match level_keep_in_view(spec, w) {
        Some(own) => {
            let rows = spec.levels[w.level]
                .camera
                .as_ref()
                .map_or(&[][..], |c| &c.keep_in_view_rows[..]);
            Some(rows_bounds(spec, b, rows, own))
        }
        None => spec.cameras[i]
            .keep_in_view
            .map(|k| keep_bounds(spec, b, i, k)),
    }
}

/// `bounds`, or those of the first of `rows` the body's row is in.
fn rows_bounds(spec: &Spec, b: &Body, rows: &[([i64; 2], [i64; 2])], bounds: [i64; 2]) -> [i64; 2] {
    if rows.is_empty() {
        return bounds;
    }
    let kept = spec
        .kind(b)
        .jump
        .as_ref()
        .is_some_and(|j| j.fatal_drop.is_some());
    let row = if !b.on_ground && !b.climbing && kept {
        b.ext[TAKEOFF_Y_WORD]
    } else {
        b.y.floor()
    };
    rows.iter()
        .find(|(r, _)| (r[0]..=r[1]).contains(&row))
        .map_or(bounds, |(_, k)| *k)
}

/// Whether a state stops body `j`'s system this frame: the player's or
/// the enemies'.
fn entity_stopped(spec: &Spec, w: &World, f: &Frame, j: usize) -> bool {
    let sys = if spec.kinds[w.bodies[j].kind].role == Role::Player {
        System::Player
    } else {
        System::Enemies
    };
    f.stopped.contains(&sys)
}

/// Cameras follow the first entity of their kind. With the level's
/// `hold_player`, a camera more than a step from its target holds the
/// player on the next frame.
pub fn camera(spec: &Spec, w: &mut World, f: &mut Frame) {
    w.hold = false;
    for i in 0..spec.cameras.len() {
        let follow = w.of_kind(spec.cameras[i].follow).next().map(|(j, _)| j);
        let pushed: i64 = f
            .pushed
            .iter()
            .filter(|(j, _)| Some(*j) == follow)
            .map(|(_, by)| by)
            .sum();
        // A `push_hold` counts down at the start of the step, holds the
        // camera while frames are left, and starts again after a push
        // this frame, after the camera has moved by the frame's move.
        if w.camera_hold.len() < spec.cameras.len() {
            w.camera_hold.resize(spec.cameras.len(), 0);
        }
        if !spec.cameras[i]
            .by_move
            .is_some_and(|m| m.hold_with_counters)
            || timers_run(f)
        {
            w.camera_hold[i] = w.camera_hold[i].saturating_sub(1);
        }
        // A state stopping the followed entity's system (`keep_move`).
        let still = follow.is_some_and(|j| entity_stopped(spec, w, f, j));
        let moved = note_move(spec, w, i, pushed, still);
        if w.camera_hold[i] == 0 {
            place_camera_moved(spec, w, i, false, pushed, moved);
        }
        keep_in_view(spec, w, i);
        see(spec, w, i);
        let push_hold = spec.cameras[i].by_move.map_or(0, |m| m.push_hold);
        if push_hold > 0 && f.pushed.iter().any(|(j, _)| Some(*j) == follow) {
            w.camera_hold[i] = push_hold;
        }
    }
}

/// Whether the frame timers count this frame: the counters system runs,
/// neither frozen nor stopped by a state (a camera's `hold_with_counters`,
/// core 0.4.23).
fn timers_run(f: &Frame) -> bool {
    !f.frozen && !f.stopped.contains(&System::Counters)
}

/// Puts camera `i` where its entity's position less the lead puts it,
/// inside the level when clamped, on the axes it follows (the level's
/// own, when it gives them). Unless it scrolls back or the level is
/// starting, it never moves left or up. With the level's `step`, it moves
/// at most that much a frame, except when a level starts. A camera
/// `by_move` moves by the entity's move instead, except when a level
/// starts. Afterwards the camera keeps the entity's place in its view.
pub fn place_camera(spec: &Spec, w: &mut World, i: usize, start: bool) {
    place_camera_pushed(spec, w, i, start, 0);
    // A level start measures the next move from here, and none before it.
    let at = w
        .of_kind(spec.cameras[i].follow)
        .next()
        .map(|(_, b)| [b.x.floor(), b.y.floor()]);
    if start
        && let Some(pos) = at
        && spec.cameras[i].by_move.is_some_and(|m| m.keep_move)
    {
        let n = spec.cameras.len();
        w.last_move.resize(n.max(w.last_move.len()), [0; 2]);
        w.follow_at.resize(n.max(w.follow_at.len()), [0; 2]);
        w.last_move[i] = [0; 2];
        w.follow_at[i] = pos;
    }
}

/// [`place_camera`], for an entity wall points pushed `pushed` pixels
/// across this frame: a camera `by_move` doesn't count the push as a move.
fn place_camera_pushed(spec: &Spec, w: &mut World, i: usize, start: bool, pushed: i64) {
    place_camera_moved(spec, w, i, start, pushed, None);
}

/// [`place_camera_pushed`], with the followed entity's move this frame as
/// [`note_move`] gave it for a camera `by_move` with `keep_move`; without
/// one the move is where the entity is less where it was in the view.
fn place_camera_moved(
    spec: &Spec,
    w: &mut World,
    i: usize,
    start: bool,
    pushed: i64,
    noted: Option<[i64; 2]>,
) {
    if !start && scroll_stopped(spec, w) {
        return;
    }
    let cam = &spec.cameras[i];
    let level = &spec.levels[w.level];
    let Some((_, b)) = w.of_kind(cam.follow).next() else {
        return;
    };
    let pos = [b.x, b.y];
    let px = level.pixels();
    let axes = level.axes(cam);
    let over = level.camera.as_ref();
    let lead = over.and_then(|c| c.lead).unwrap_or(cam.lead);
    let step = over.and_then(|c| c.step).filter(|_| !start);
    let mut at = w.cameras[i];
    let mut hold = false;
    for axis in 0..2 {
        if !axes[axis] {
            continue;
        }
        let mut t = match cam.by_move {
            Some(m) if !start => {
                let seen = w.seen.get(i).map_or(Fx::ZERO, |s| s[axis]).floor();
                // Where the entity was, in whole pixels, and how far it
                // moved since, as the corner hasn't moved yet.
                let push = if axis == 0 { pushed } else { 0 };
                let moved = noted.map_or_else(
                    || pos[axis].floor() - push - (at[axis].floor() + seen),
                    |n| n[axis],
                );
                let by = if moved <= 0 || seen < m.from.floor() {
                    0
                } else if seen < m.full.floor() {
                    (moved - m.short_by.floor()).max(1)
                } else {
                    moved
                };
                at[axis].add(Fx::from_int(by))
            }
            _ => pos[axis].sub(lead[axis]),
        };
        if cam.clamp_to_level {
            t = t.min(Fx::from_int(px[axis] - cam.size[axis])).max(Fx::ZERO);
        }
        if !cam.scroll_back && !start {
            t = t.max(at[axis]);
        }
        if let Some(step) = step {
            let d = t.sub(at[axis]);
            if d.max(d.neg()) > step {
                hold = true;
                t = if d > Fx::ZERO {
                    at[axis].add(step)
                } else {
                    at[axis].sub(step)
                };
            }
        }
        at[axis] = t;
    }
    w.cameras[i] = at;
    see(spec, w, i);
    if hold && over.is_some_and(|c| c.hold_player) {
        w.hold = true;
    }
}

/// For a camera `by_move` with `keep_move` (core 0.4.12): the followed
/// entity's whole-pixel move since the camera last noted it, less a push
/// of `pushed` across, kept as its last move; or, when a state stopped the
/// entity's system this frame (`still`), the last move again. `None` for
/// any other camera.
fn note_move(spec: &Spec, w: &mut World, i: usize, pushed: i64, still: bool) -> Option<[i64; 2]> {
    if !spec.cameras[i].by_move.is_some_and(|m| m.keep_move) {
        return None;
    }
    let (_, b) = w.of_kind(spec.cameras[i].follow).next()?;
    let pos = [b.x.floor(), b.y.floor()];
    let n = spec.cameras.len();
    if w.last_move.len() < n {
        w.last_move.resize(n, [0; 2]);
    }
    if w.follow_at.len() < n {
        w.follow_at.resize(n, [0; 2]);
    }
    if !still {
        let at = w.follow_at[i];
        w.last_move[i] = [pos[0] - pushed - at[0], pos[1] - at[1]];
    }
    w.follow_at[i] = pos;
    Some(w.last_move[i])
}

/// Keeps camera `i`'s followed entity's whole-pixel place in its view,
/// which a camera `by_move` reads on the next frame.
pub fn see(spec: &Spec, w: &mut World, i: usize) {
    let Some((_, b)) = w.of_kind(spec.cameras[i].follow).next() else {
        return;
    };
    // An entity drawn only on some frames is seen only on those (a state's
    // `seen_every`).
    let every = spec.kinds[b.kind]
        .machine
        .as_ref()
        .map_or(0, |m| m.states[b.state].part.seen_every);
    if every > 1 && !w.frame.is_multiple_of(u64::from(every)) {
        return;
    }
    let (pos, at) = ([b.x, b.y], w.cameras[i]);
    if w.seen.len() < spec.cameras.len() {
        w.seen.resize(spec.cameras.len(), [Fx::ZERO; 2]);
    }
    w.seen[i] = [0, 1].map(|a| Fx::from_int(pos[a].floor() - at[a].floor()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{
        Action, Camera, Contact, Counter, HeadHit, Jump, Kind, Level, Machine, Patrol, Rect, Run,
        Spawn, State, Transition,
    };

    fn n(v: i64) -> Fx {
        Fx::from_int(v)
    }

    /// A small game: a floor, a player and a patroller that kills on touch,
    /// a game machine that restarts the level 3 frames after a death, and a
    /// death counter.
    fn spec() -> Spec {
        let (w, h) = (8, 4);
        let mut tiles = vec![Tile::Empty; w * h];
        for c in 0..w {
            tiles[3 * w + c] = Tile::Solid;
        }
        let rect = Rect {
            x: n(0),
            y: n(0),
            w: n(16),
            h: n(16),
        };
        Spec {
            name: "test".into(),
            actions: vec![
                Action {
                    name: "jump".into(),
                    mask: 1,
                },
                Action {
                    name: "left".into(),
                    mask: 1 << 6,
                },
                Action {
                    name: "right".into(),
                    mask: 1 << 7,
                },
            ],
            kinds: vec![
                Kind {
                    name: "hero".into(),
                    role: Role::Player,
                    size: [n(16), n(16)],
                    hitbox: rect,
                    tile_hitbox: rect,
                    tile_collision: true,
                    run: Some(Run {
                        left: 1,
                        right: 2,
                        walk_speed: n(1),
                        run: None,
                        acceleration: None,
                        run_acceleration: None,
                        deceleration: None,
                        skid_deceleration: None,
                        air_control: true,
                        start: None,
                        air_speed: None,
                        wall_snap: false,
                        move_step: None,
                        cap_whole: false,
                        skid_last_motion: false,
                        skid_turn_below: None,
                        skid_turn_with: Vec::new(),
                        decel_bands: Vec::new(),
                        decel_hold_from: None,
                        run_hold_frames: 0,
                        run_with_motion: false,
                        takeoff_in_air: false,
                        face_on_ground: false,
                        air_run_from: None,
                        walls: None,
                        push_masks: false,
                        speed_before_walls: false,
                        face_both: false,
                        off: false,
                        step_cycle: Vec::new(),
                        air_step_cycle: Vec::new(),
                        lone_direction: false,
                        face_way: false,
                        both_left: false,
                    }),
                    jump: Some(Jump {
                        action: 0,
                        speed: n(-4),
                        gravity: Fx(0x4000),
                        gravity_held: None,
                        max_fall: n(4),
                        max_fall_from_half: false,
                        no_gravity_on_ground: false,
                        land_keeps_fraction: false,
                        keep_fall_gravity: false,
                        start_fall_gravity: None,
                        land_depth: None,
                        feet: None,
                        bands: Vec::new(),
                        coyote_frames: 0,
                        buffer_frames: 0,
                        move_first: false,
                        release_cap: None,
                        head: None,
                        pickup: None,
                        collide_y: None,
                        takeoff_fraction: None,
                        land_hold: 0,
                        ledge_fall: None,
                        fatal_drop: None,
                        fatal_drop_in_air: false,
                    }),
                    patrol: None,
                    contact: None,
                    walk: None,
                    fall: None,
                    fly: None,
                    hop: None,
                    oscillate: None,
                    leave_view: None,
                    leap: None,
                    launcher: None,
                    launched: false,
                    shuttle: None,
                    stream: None,
                    orbit: None,
                    orbit_part: None,
                    springboard: None,
                    path: None,
                    climb: None,
                    health: None,
                    knockback: None,
                    hits: None,
                    shoot: None,
                    carry: None,
                    carryable: None,
                    move8: None,
                    machine: None,
                    ext: None,
                    ext_params: Default::default(),
                    ext_tables: Default::default(),
                    no_slot: false,
                    hook_through_stops: false,
                    hook_frame_start: false,
                    hook_first: false,
                    hook_last: false,
                    extra_slots: 0,
                    despawn_behind: None,
                    despawn_behind_wrap: None,
                    despawn_ahead: None,
                    invincible: None,
                    locks: None,
                    bounce: None,
                    touch_points: Vec::new(),
                },
                Kind {
                    name: "bug".into(),
                    role: Role::Other,
                    size: [n(16), n(16)],
                    hitbox: rect,
                    tile_hitbox: rect,
                    tile_collision: false,
                    run: None,
                    jump: None,
                    patrol: Some(Patrol {
                        step: n(1),
                        period: 1,
                    }),
                    contact: Some(Contact {
                        stomp: None,
                        keep_on_stomp: false,
                        otherwise: Touch::KillPlayer,
                        damage: 1,
                        stand: None,
                        every: 1,
                        phase: 0,
                        invincible: Touch::Defeat,
                        touch_edges: false,
                        in_turn: false,
                        after_move: false,
                        sides: None,
                        ride: None,
                        near: None,
                    }),
                    walk: None,
                    fall: None,
                    fly: None,
                    hop: None,
                    oscillate: None,
                    leave_view: None,
                    leap: None,
                    launcher: None,
                    launched: false,
                    shuttle: None,
                    stream: None,
                    orbit: None,
                    orbit_part: None,
                    springboard: None,
                    path: None,
                    climb: None,
                    health: None,
                    knockback: None,
                    hits: None,
                    shoot: None,
                    carry: None,
                    carryable: None,
                    move8: None,
                    machine: None,
                    ext: None,
                    ext_params: Default::default(),
                    ext_tables: Default::default(),
                    no_slot: false,
                    hook_through_stops: false,
                    hook_frame_start: false,
                    hook_first: false,
                    hook_last: false,
                    extra_slots: 0,
                    despawn_behind: None,
                    despawn_behind_wrap: None,
                    despawn_ahead: None,
                    invincible: None,
                    locks: None,
                    bounce: None,
                    touch_points: Vec::new(),
                },
            ]
            .into(),
            levels: vec![Level {
                halfway: None,
                surfaces: Vec::new(),
                ladders: Vec::new(),
                ladder_tops: Vec::new(),
                ladder_stops: Vec::new(),
                ground: Vec::new(),
                ground_feet: [0, 0],
                ground_land: None,
                ground_boxes: false,
                ground_land_rising: false,
                ground_bodies: Vec::new(),
                pickups: Vec::new(),
                name: "main".into(),
                tile_size: 16,
                size: [w as i64, h as i64],
                tiles,
                spawns: vec![
                    Spawn {
                        kind: 0,
                        x: n(0),
                        y: n(32),
                        patrol_end: None,
                        drawn: false,
                        at_edge: None,
                        appear_moving: false,
                    },
                    Spawn {
                        kind: 1,
                        x: n(40),
                        y: n(32),
                        patrol_end: Some(n(60)),
                        drawn: false,
                        at_edge: None,
                        appear_moving: false,
                    },
                ],
                kill_below: Some(n(60)),
                pit_event: None,
                goal_x: None,
                goal_under: None,
                hazard: None,
                water: None,
                ice: None,
                origin: [Fx::ZERO; 2],
                camera: None,
                despawn_ahead: None,
                respawn: false,
                counters: Vec::new(),
                next: None,
                spawn_ahead: None,
                despawn_behind: None,
                spawn_slots: None,
                hit_widths: Vec::new(),
                despawn_by_position: false,
                despawn_after_move: false,
                spawn_in_order: false,
                appear_still: false,
                slot_rest: false,
                hard: false,
                draw: None,
                scroll_stop: None,
                loop_back: None,
                exits: Vec::new(),
            }]
            .into(),
            start_level: 0,
            cameras: vec![Camera {
                name: "main".into(),
                follow: 0,
                axis_x: true,
                axis_y: false,
                size: [64, 64],
                lead: [n(16), n(0)],
                clamp_to_level: true,
                scroll_back: true,
                by_move: None,
                keep_in_view: None,
                keep_in_view_touch: false,
                keep_in_view_turn: false,
                keep_in_view_rows: Vec::new(),
            }]
            .into(),
            machines: vec![Machine {
                name: "game".into(),
                initial: 0,
                states: vec![
                    State {
                        name: "playing".into(),
                        freeze: false,
                        part: Default::default(),
                        transitions: vec![Transition {
                            align_to: None,
                            below: None,
                            word: None,
                            pressed: None,
                            held: None,
                            to: 1,
                            on: Some(EventKind::PLAYER_DIED),
                            after: None,
                            align: None,
                            guard: None,
                            actions: Vec::new(),
                        }],
                    },
                    State {
                        name: "dying".into(),
                        freeze: true,
                        part: Default::default(),
                        transitions: vec![Transition {
                            align_to: None,
                            below: None,
                            word: None,
                            pressed: None,
                            held: None,
                            to: 0,
                            on: None,
                            after: Some(3),
                            align: None,
                            guard: None,
                            actions: vec![Do::RestartLevel],
                        }],
                    },
                ],
            }]
            .into(),
            counters: vec![Counter {
                restarts_clock: false,
                high_of: None,
                name: "deaths".into(),
                start: 0,
                changes: vec![Change::On(EventKind::PLAYER_DIED, 1)],
                min: None,
                max: None,
                wrap: false,
                runs_when_frozen: false,
                shift: None,
            }]
            .into(),
            events: crate::spec::BUILTIN_EVENTS
                .iter()
                .map(|e| e.to_string())
                .chain(["deaths.reached_min".into(), "deaths.wrapped".into()])
                .collect(),
            own_events: crate::spec::BUILTIN_EVENTS.len() + 2,
            order: vec![
                System::Rules,
                System::EntityStates,
                System::Counters,
                System::Spawn,
                System::Player,
                System::Enemies,
                System::Contact,
                System::Camera,
            ],
            seed: 7,
            profile: None,
            keep_direction: false,
            turn: None,
            stages: None,
            cost: None,
        }
    }

    /// Features `action.clear` and `action.respawn`: `clear:` takes every
    /// active body of an entity out at once, and `respawn:` puts each body
    /// a spawn placed back at that spawn, at rest, its state kept.
    #[test]
    fn clear_and_respawn_act_on_every_body_of_an_entity() {
        let s = spec();
        let mut w = World::power_on(&s);
        let spawn = (w.bodies[0].x, w.bodies[0].y);
        for _ in 0..10 {
            step(&s, &mut w, 1 << 7);
        }
        assert_ne!(w.bodies[0].x, spawn.0);
        let bug = w.bodies.iter().position(|b| b.kind == 1).unwrap();
        assert!(w.bodies[bug].alive);
        let mut f = begin_frame(&s, &mut w, 0);
        act(&s, &mut w, &mut f, Do::Respawn(0));
        assert_eq!((w.bodies[0].x, w.bodies[0].y), spawn);
        assert_eq!((w.bodies[0].vx, w.bodies[0].vy), (Fx::ZERO, Fx::ZERO));
        act(&s, &mut w, &mut f, Do::Clear(1));
        assert!(w.bodies.iter().filter(|b| b.kind == 1).all(|b| !b.alive));
        assert!(w.bodies[0].alive, "the player is another entity");
    }

    #[test]
    fn walking_into_the_patroller_dies_and_the_level_restarts() {
        let s = spec();
        let mut w = World::power_on(&s);
        assert!(
            w.bodies[0].on_ground,
            "the hero spawns standing on the floor"
        );
        let right = 1 << 7;
        let mut died = None;
        for i in 1..=40 {
            let events = step(&s, &mut w, right);
            if events.iter().any(|e| e.kind == EventKind::PLAYER_DIED) {
                died = Some(i);
                break;
            }
        }
        let died = died.expect("the hero runs into the bug");
        assert_eq!(w.counters[0], 1);
        assert_eq!(w.machines[0].state, 1);
        // Frozen: nothing moves for 3 frames, then the level starts over.
        let frozen = w.bodies.clone();
        step(&s, &mut w, right);
        step(&s, &mut w, right);
        assert_eq!(w.bodies, frozen);
        let events = step(&s, &mut w, right);
        assert_eq!(w.frame, died + 3);
        assert!(events.iter().any(|e| e.kind == EventKind::LEVEL_STARTED));
        assert_eq!(w.machines[0].state, 0);
        assert_eq!(
            w.bodies[0].x,
            n(0),
            "back at the spawn, and that frame stays still"
        );
        assert_eq!(w.counters[0], 1, "counters are kept");
    }

    /// [`spec`] as a top-down game: the hero moves with `move8` (up and
    /// down added to the input), with no run, jump or gravity.
    fn topdown_spec() -> Spec {
        let mut s = spec();
        s.actions.push(Action {
            name: "up".into(),
            mask: 1 << 4,
        });
        s.actions.push(Action {
            name: "down".into(),
            mask: 1 << 5,
        });
        let hero = &mut s.kinds[0];
        hero.run = None;
        hero.jump = None;
        hero.move8 = Some(crate::spec::Move8 {
            speed: n(1),
            left: 1,
            right: 2,
            up: 3,
            down: 4,
        });
        for sys in &mut s.order {
            if *sys == System::Player {
                *sys = System::TopdownPlayer;
            }
        }
        s
    }

    #[test]
    fn move8_slides_along_a_wall_and_climbs_without_gravity() {
        let s = topdown_spec();
        let mut w = World::power_on(&s);
        let (y0, x0) = (w.bodies[0].y, w.bodies[0].x);
        let (right, up, down) = (1 << 7, 1 << 4, 1 << 5);
        // Right and down into the floor: down is blocked, right goes on.
        for _ in 0..4 {
            step(&s, &mut w, right | down);
        }
        assert_eq!(w.bodies[0].y, y0, "the floor stops the move down");
        assert_eq!(w.bodies[0].x, x0.add(n(4)), "and the move across slides on");
        // Up with nothing held across: up a pixel a frame, no gravity pull.
        for _ in 0..5 {
            step(&s, &mut w, up);
        }
        assert_eq!(w.bodies[0].y, y0.sub(n(5)));
        assert_eq!(w.bodies[0].x, x0.add(n(4)));
        // Nothing held: it stays where it is.
        let at = w.bodies[0];
        step(&s, &mut w, 0);
        assert_eq!((w.bodies[0].x, w.bodies[0].y), (at.x, at.y));
        // Up to the top of the level and no further.
        for _ in 0..100 {
            step(&s, &mut w, up);
        }
        assert_eq!(w.bodies[0].y, n(0));
    }

    #[test]
    fn a_jump_rises_and_lands() {
        let s = spec();
        let mut w = World::power_on(&s);
        step(&s, &mut w, 1);
        assert!(!w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].vy, Fx(-0x3_C000));
        let mut frames = 1;
        while !w.bodies[0].on_ground {
            step(&s, &mut w, 1);
            frames += 1;
            assert!(frames < 100);
        }
        assert_eq!(w.bodies[0].y, n(32));
        // Holding the button doesn't jump again: it must be pressed anew.
        step(&s, &mut w, 1);
        assert!(w.bodies[0].on_ground);
    }

    /// A profile's name is in the state hash and not in the state: the
    /// base's hash is the state's, and two profiles' runs never share one.
    #[test]
    fn the_profile_is_in_the_hash_and_not_the_state() {
        let base = spec();
        let normal = Spec {
            profile: Some("normal".into()),
            keep_direction: false,
            turn: None,
            ..spec()
        };
        let (mut a, mut b) = (World::power_on(&base), World::power_on(&normal));
        for _ in 0..10 {
            step(&base, &mut a, 1 << 7);
            step(&normal, &mut b, 1 << 7);
        }
        assert_eq!(a.canonical(), b.canonical());
        assert_eq!(a.hash(), *blake3::hash(&a.canonical()).as_bytes());
        assert_ne!(a.hash(), b.hash());
        // Loading a state keeps the spec's profile.
        let back = World::from_canonical(&normal, &a.canonical()).unwrap();
        assert_eq!(back.hash(), b.hash());
    }

    #[test]
    fn canonical_state_round_trips_and_runs_the_same() {
        let s = spec();
        let mut a = World::power_on(&s);
        for i in 0..20u8 {
            step(&s, &mut a, if i % 5 == 0 { 1 } else { 1 << 7 });
        }
        let bytes = a.canonical();
        let mut b = World::from_canonical(&s, &bytes).unwrap();
        assert_eq!(a, b);
        for _ in 0..30 {
            step(&s, &mut a, 1 << 7);
            step(&s, &mut b, 1 << 7);
        }
        assert_eq!(a.hash(), b.hash());
        // Damaged bytes are refused, never read past.
        for cut in [0, 4, 8, bytes.len() - 1] {
            assert!(
                World::from_canonical(&s, &bytes[..cut]).is_err(),
                "cut at {cut}"
            );
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(World::from_canonical(&s, &longer).is_err());
        let mut bad_kind = bytes;
        bad_kind[34] = 9; // the first entity's kind
        assert!(World::from_canonical(&s, &bad_kind).is_err());
    }

    /// Core `cost.frame`: the base, a term on a counter as the frame
    /// started and one on it as the frame ends, and a term per body of a
    /// kind, summed by `frame_cost` on made-up numbers.
    #[test]
    fn frame_cost_sums_the_terms_that_hold() {
        use crate::spec::{Cost, CostOn, CostTerm, CostTest};
        let mut s = spec();
        s.counters = vec![Counter {
            restarts_clock: false,
            high_of: None,
            name: "timer".into(),
            start: 1,
            changes: vec![Change::Every(1, -1)],
            min: Some(0),
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
        }]
        .into();
        let above = |start: bool| CostTest {
            on: CostOn::Counter(0, start),
            lo: 1,
            hi: i64::MAX,
        };
        s.cost = Some(Cost {
            base: 100,
            terms: vec![
                CostTerm {
                    cycles: 5,
                    tests: vec![above(true)],
                    per: None,
                    per_tests: vec![],
                },
                CostTerm {
                    cycles: 7,
                    tests: vec![above(false)],
                    per: None,
                    per_tests: vec![],
                },
                CostTerm {
                    cycles: 3,
                    tests: vec![],
                    per: Some(1),
                    per_tests: vec![],
                },
            ],
        });
        let mut w = World::power_on(&s);
        let bugs = w
            .bodies
            .iter()
            .filter(|b| b.active() && b.kind == 1)
            .count() as i64;
        let mut f = begin_frame(&s, &mut w, 0);
        count_every(&s, &mut w, &mut f, false);
        // The timer was 1 as the frame started and is 0 at its end.
        assert_eq!(w.counters[0], 0);
        assert_eq!(frame_cost(&s, &w, &f), 100 + 5 + 3 * bugs);
        let f2 = begin_frame(&s, &mut w, 0);
        assert_eq!(frame_cost(&s, &w, &f2), 100 + 3 * bugs);
        s.cost = None;
        assert_eq!(frame_cost(&s, &w, &f2), 0);
    }

    #[test]
    fn counters_bound_wrap_and_tick() {
        let mut s = spec();
        s.counters = vec![Counter {
            restarts_clock: false,
            high_of: None,
            name: "time".into(),
            start: 2,
            changes: vec![Change::Every(2, -1)],
            min: Some(0),
            max: Some(3),
            wrap: false,
            runs_when_frozen: false,
            shift: None,
        }]
        .into();
        let mut w = World::power_on(&s);
        let mut reached = None;
        for i in 1..=8 {
            let e = step(&s, &mut w, 0);
            if e.iter().any(|e| e.kind == Spec::counter_events(0).0) {
                reached = Some(i);
            }
        }
        assert_eq!(w.counters[0], 0);
        assert_eq!(reached, Some(4), "2 -> 1 at frame 2, -> 0 at frame 4");
        let mut f = Frame::default();
        let mut w2 = World::power_on(&s);
        s.counters[0].wrap = true;
        change_counter(&s, &mut w2, &mut f, 0, 3);
        assert_eq!(w2.counters[0], 1, "2 + 3 wraps past 3 to 1");
        assert!(f.events.iter().any(|e| e.kind == Spec::counter_events(0).1));
    }

    /// A counter with `runs_when_frozen` counts its frames while the game's
    /// rule freezes the world; one without stops, and both count after.
    #[test]
    fn a_counter_that_runs_when_frozen_counts_through_a_freeze() {
        let mut s = spec();
        let every = |name: &str, runs_when_frozen| Counter {
            restarts_clock: false,
            high_of: None,
            name: name.into(),
            start: 0,
            changes: vec![Change::Every(1, 1)],
            min: None,
            max: None,
            wrap: false,
            runs_when_frozen,
            shift: None,
        };
        s.counters = vec![every("cycle", true), every("time", false)].into();
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        assert_eq!(w.counters, [1, 1]);
        // Dying: frozen from the next frame, for the state's 3 frames.
        w.machines[0].state = 1;
        w.machines[0].time = 0;
        step(&s, &mut w, 0);
        step(&s, &mut w, 0);
        assert_eq!(w.machines[0].state, 1);
        assert_eq!(w.counters, [3, 1], "only the cycle counts while frozen");
        assert_eq!(w.ticks, [0, 0]);
    }

    /// A shift counter steps every frame, through a freeze with
    /// `runs_when_frozen`, taking the tapped bits' exclusive-or in at the top.
    #[test]
    fn a_shift_counter_steps_every_frame_and_through_a_freeze() {
        let shift = crate::spec::Shift {
            bits: 16,
            taps: vec![9, 1],
        };
        assert_eq!(shift.step(0x0200), 0x8100, "one tap set shifts a 1 in");
        assert_eq!(shift.step(0x0202), 0x0101, "both set shift a 0 in");
        let mut s = spec();
        s.counters = vec![Counter {
            restarts_clock: false,
            high_of: None,
            name: "random".into(),
            start: 0x0200,
            changes: vec![],
            min: None,
            max: None,
            wrap: false,
            runs_when_frozen: true,
            shift: Some(shift.clone()),
        }]
        .into();
        let mut w = World::power_on(&s);
        let mut want = 0x0200;
        for _ in 0..4 {
            step(&s, &mut w, 0);
            want = shift.step(want);
            assert_eq!(w.counters[0], want);
        }
        w.machines[0].state = 1;
        w.machines[0].time = 0;
        for _ in 0..2 {
            step(&s, &mut w, 0);
            want = shift.step(want);
        }
        assert_eq!(w.machines[0].state, 1);
        assert_eq!(w.counters[0], want, "it steps while frozen");
    }

    /// A state that freezes part of the world (core 0.4.12): the bug's
    /// patrol and the counters stop from the frame after the state is
    /// entered while the player runs on, and start again the frame after
    /// it is left.
    #[test]
    fn a_part_freeze_stops_only_its_systems() {
        let mut s = spec();
        s.kinds[1].contact = None;
        s.counters[0].changes = vec![Change::Every(1, 1)];
        s.machines[0].states.push(State {
            name: "pause".into(),
            freeze: false,
            part: crate::spec::Part {
                stops: vec![System::Enemies, System::Counters],
                ..Default::default()
            },
            transitions: vec![Transition {
                align_to: None,
                below: None,
                word: None,
                pressed: None,
                held: None,
                to: 0,
                on: None,
                after: Some(2),
                align: None,
                guard: None,
                actions: Vec::new(),
            }],
        });
        let mut w = World::power_on(&s);
        let right = 1 << 7;
        step(&s, &mut w, right);
        let (bug, hero, count) = (w.bodies[1].x, w.bodies[0].x, w.counters[0]);
        w.machines[0].state = 2;
        w.machines[0].time = 0;
        step(&s, &mut w, right);
        assert_eq!(w.bodies[1].x, bug, "the patrol stops");
        assert_eq!(w.counters[0], count, "the counter stops");
        assert!(w.bodies[0].x > hero, "the player runs");
        step(&s, &mut w, right);
        assert_eq!(w.machines[0].state, 0, "left after 2 frames");
        assert_eq!(w.bodies[1].x, bug, "stopped on the frame it is left");
        step(&s, &mut w, right);
        assert!(w.bodies[1].x > bug, "free from the next");
        assert_eq!(w.counters[0], count + 1);
    }

    /// `contact.in_turn`: a touch's `stops_at_once` of the enemies stops the
    /// loop at the toucher's numbered slot, so a body in an earlier slot
    /// moves and the toucher doesn't.
    #[test]
    fn a_touch_in_turn_stops_the_enemies_from_its_slot() {
        let s = spec();
        let mut w = World::power_on(&s);
        let bug = w.bodies.iter().position(|b| b.kind == 1).expect("a bug");
        w.bodies.push(w.bodies[bug]);
        let other = w.bodies.len() - 1;
        w.slots = vec![
            crate::world::Slot {
                body: other as u32 + 1,
                ..Default::default()
            },
            crate::world::Slot {
                body: bug as u32 + 1,
                ..Default::default()
            },
        ];
        let at_once = crate::spec::Part {
            stops_at_once: vec![System::Enemies],
            ..Default::default()
        };
        let mut f = begin_frame(&s, &mut w, 0);
        f.hitter_slot = Some(1);
        enter(&mut f, &at_once);
        f.hitter_slot = None;
        assert!(f.runs(System::Enemies));
        assert_eq!(f.stopped_from, Some(1));
        // A body's turn counts its `invuln` down.
        w.bodies[bug].invuln = 2;
        w.bodies[other].invuln = 2;
        enemies(&s, &mut w, &mut f);
        assert_eq!(w.bodies[bug].invuln, 2, "the toucher has no turn");
        assert_eq!(w.bodies[other].invuln, 1, "the earlier slot has its turn");
        // Without a numbered slot the touch stops them all, as before.
        let mut f = begin_frame(&s, &mut w, 0);
        enter(&mut f, &at_once);
        assert!(!f.runs(System::Enemies));
    }

    /// `contact.after_move`: the toucher moves first and the enemies have
    /// had their turn, so a body that would only touch after the move does,
    /// and one that touched before it doesn't until then.
    #[test]
    fn a_touch_after_move_comes_in_the_turn_after_the_move() {
        let mut s = spec();
        let bug = s.kinds.iter().position(|k| k.name == "bug").expect("a bug");
        let c = s.kinds[bug].contact.as_mut().expect("a contact");
        c.after_move = true;
        c.every = 1;
        let mut w = World::power_on(&s);
        let e = w.bodies.iter().position(|b| b.kind == bug).expect("a bug");
        let p = w
            .bodies
            .iter()
            .position(|b| s.kinds[b.kind].role == Role::Player)
            .expect("a player");
        let h = s.kinds[bug].hitbox;
        let ph = s.kinds[w.bodies[p].kind].hitbox;
        // Its box a pixel past the player's right edge: no touch yet.
        w.bodies[e].x = w.bodies[p].x.add(ph.x).add(ph.w).sub(h.x).add(n(1));
        w.bodies[e].y = w.bodies[p].y;
        let mut f = begin_frame(&s, &mut w, 0);
        contact(&s, &mut w, &mut f);
        assert!(f.events.is_empty(), "the contact system leaves it");
        w.bodies[e].x = w.bodies[e].x.sub(n(2));
        touch_after_move(&s, &mut w, &mut f, e);
        assert!(!f.events.is_empty(), "the touch comes after the move");
    }

    /// `stops_at_once`: a state entered by the rules at the top of a frame
    /// stops the enemies for the rest of that frame too; `keep_time` keeps
    /// the frames counted toward the next state's `after`, and `align`
    /// waits for a frame number that is a multiple of it.
    #[test]
    fn a_state_stops_at_once_keeps_its_time_and_waits_for_its_frame() {
        let mut s = spec();
        s.kinds[1].contact = None;
        let to = |to, after, align| Transition {
            align_to: None,
            below: None,
            word: None,
            pressed: None,
            held: None,
            to,
            on: None,
            after: Some(after),
            align,
            guard: None,
            actions: Vec::new(),
        };
        let part = |stops: Vec<System>, at_once: Vec<System>, keep_time| crate::spec::Part {
            stops,
            stops_at_once: at_once,
            keep_time,
            ..Default::default()
        };
        // playing -> a (after 1) -> b (after 3 from a's entry) -> c (from
        // frame 6 of a's span on, on a frame number that is a multiple of 4).
        s.machines[0].states[0].transitions.push(to(2, 1, None));
        s.machines[0].states.push(State {
            name: "a".into(),
            freeze: false,
            part: part(vec![System::Enemies], vec![System::Enemies], false),
            transitions: vec![to(3, 3, None)],
        });
        s.machines[0].states.push(State {
            name: "b".into(),
            freeze: false,
            part: part(vec![System::Enemies], Vec::new(), true),
            transitions: vec![to(4, 6, Some(4))],
        });
        s.machines[0].states.push(State {
            name: "c".into(),
            freeze: false,
            part: part(Vec::new(), Vec::new(), true),
            transitions: Vec::new(),
        });
        let mut w = World::power_on(&s);
        let bug = w.bodies[1].x;
        step(&s, &mut w, 0);
        assert_eq!((w.frame, w.machines[0].state), (1, 2));
        assert_eq!(w.bodies[1].x, bug, "stopped on the frame a is entered");
        let mut entered = Vec::new();
        for _ in 0..10 {
            let before = w.machines[0].state;
            step(&s, &mut w, 0);
            if w.machines[0].state != before {
                entered.push((w.frame, w.machines[0].state, w.machines[0].time));
            }
        }
        // b at frame 4 (a's 3rd frame), keeping a's time; c at the first
        // frame from a's 6th (frame 7) whose number is a multiple of 4.
        assert_eq!(entered, [(4, 3, 3), (8, 4, 7)]);
    }

    /// A state's `holds`: the held counter counts its frames up to its
    /// change and waits there, and changes on the first frame after.
    #[test]
    fn a_held_counter_waits_at_its_change() {
        let mut s = spec();
        s.kinds[1].contact = None;
        s.counters[0].changes = vec![Change::Every(3, 1)];
        s.machines[0].states.push(State {
            name: "hold".into(),
            freeze: false,
            part: crate::spec::Part {
                holds: vec![0],
                ..Default::default()
            },
            transitions: Vec::new(),
        });
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        w.machines[0].state = 2;
        for _ in 0..5 {
            step(&s, &mut w, 0);
        }
        assert_eq!(
            (w.counters[0], w.ticks[0]),
            (0, 3),
            "counted to the change, waiting"
        );
        w.machines[0].state = 0;
        step(&s, &mut w, 0);
        assert_eq!(
            (w.counters[0], w.ticks[0]),
            (1, 0),
            "changed on the next frame"
        );
    }

    /// `contact.every` and `phase`: a touch is tested only on frames whose
    /// number less the phase is a multiple of `every`.
    #[test]
    fn a_touch_every_other_frame_waits_for_its_frame() {
        for (phase, frame) in [(0, 2), (1, 1)] {
            let mut s = spec();
            s.kinds[1].patrol = None;
            s.levels[0].spawns[1].x = n(8);
            let c = s.kinds[1].contact.as_mut().unwrap();
            c.every = 2;
            c.phase = phase;
            let mut w = World::power_on(&s);
            let mut died = None;
            for _ in 0..4 {
                let ev = step(&s, &mut w, 0);
                if died.is_none() && ev.iter().any(|e| e.kind == EventKind::PLAYER_DIED) {
                    died = Some(w.frame);
                }
            }
            assert_eq!(died, Some(frame), "phase {phase}");
        }
    }

    /// A level's `pit_event` (platformer 0.4.9): the player crossing
    /// `kill_below` fires the event once, doesn't die, and falls on.
    #[test]
    fn a_pit_event_fires_once_and_the_player_falls_on() {
        let mut s = quiet(spec());
        let w0 = s.levels[0].size[0] as usize;
        for c in 0..w0 {
            s.levels[0].tiles[3 * w0 + c] = Tile::Empty;
        }
        s.levels[0].kill_below = Some(n(60));
        s.levels[0].pit_event = Some(EventKind::GOAL_REACHED);
        let mut w = World::power_on(&s);
        let mut fired = 0;
        for _ in 0..40 {
            let ev = step(&s, &mut w, 0);
            fired += ev
                .iter()
                .filter(|e| e.kind == EventKind::GOAL_REACHED)
                .count();
            assert!(!ev.iter().any(|e| e.kind == EventKind::PLAYER_DIED));
        }
        assert_eq!(fired, 1);
        assert!(w.bodies[0].y > n(80), "it fell on: {:?}", w.bodies[0].y);
        assert!(w.bodies[0].alive);
        // Above the pit the goal is still reached.
        let mut s = quiet(spec());
        s.levels[0].pit_event = Some(EventKind::PLAYER_HURT);
        s.levels[0].kill_below = Some(n(60));
        s.levels[0].goal_x = Some(n(4));
        let mut w = World::power_on(&s);
        let mut goal = false;
        for _ in 0..10 {
            let ev = step(&s, &mut w, 1 << 7);
            goal |= ev.iter().any(|e| e.kind == EventKind::GOAL_REACHED);
        }
        assert!(goal);
    }

    /// A player at or below a goal's `under` walks past the goal; above it
    /// the goal is reached as ever.
    #[test]
    fn a_player_passes_under_a_goal() {
        let mut s = quiet(spec());
        s.levels[0].goal_x = Some(n(4));
        let reached = |s: &Spec| {
            let mut w = World::power_on(s);
            let mut goal = false;
            for _ in 0..10 {
                let ev = step(s, &mut w, 1 << 7);
                goal |= ev.iter().any(|e| e.kind == EventKind::GOAL_REACHED);
            }
            goal
        };
        assert!(reached(&s));
        s.levels[0].goal_under = Some(n(-1000));
        assert!(!reached(&s));
        s.levels[0].goal_under = Some(n(1000));
        assert!(reached(&s));
    }

    /// Features `state.step_word` and `transition.word`: a state that
    /// steps mechanic word 0 every frame, and a timed transition out of it
    /// taken only once that word is in its range.
    #[test]
    fn a_state_steps_its_word_and_leaves_on_the_word() {
        let mut s = quiet(spec());
        let t = |to: usize, after: u32, word| Transition {
            align_to: None,
            below: None,
            word,
            pressed: None,
            held: None,
            to,
            on: None,
            after: Some(after),
            align: None,
            guard: None,
            actions: Vec::new(),
        };
        let spin = crate::spec::EntityState {
            name: "spin".into(),
            freeze: false,
            part: crate::spec::Part {
                step_word: 15,
                ..Default::default()
            },
            kind: s.kinds[0].clone(),
            transitions: vec![t(1, 10, Some((0, Some(4), Some(5))))],
        };
        let still = crate::spec::EntityState {
            name: "still".into(),
            freeze: false,
            part: Default::default(),
            kind: s.kinds[0].clone(),
            transitions: Vec::new(),
        };
        s.kinds[0].machine = Some(crate::spec::EntityMachine {
            initial: 0,
            states: vec![spin, still],
        });
        let mut w = World::power_on(&s);
        w.bodies[0].ext[0] = 2;
        let mut left = None;
        for frame in 1..40 {
            step(&s, &mut w, 0);
            if w.bodies[0].state == 1 {
                left = Some(frame);
                break;
            }
        }
        // From 2 the word is 4 on frames 2, 17 and 32; the first at 10
        // frames or more is frame 17.
        assert_eq!(left, Some(17));
        assert_eq!(w.bodies[0].ext[0], 4);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].ext[0], 4, "the next state doesn't step it");
    }

    /// `enter_speed` sets a body's speed as a state is entered, and a run
    /// a state turns off leaves the speed alone with a direction held.
    #[test]
    fn a_state_sets_a_speed_on_entry_and_can_turn_the_run_off() {
        let mut s = quiet(spec());
        let mut off = s.kinds[0].clone();
        off.run.as_mut().expect("a run").off = true;
        let free = crate::spec::EntityState {
            name: "free".into(),
            freeze: false,
            part: Default::default(),
            kind: s.kinds[0].clone(),
            transitions: vec![Transition {
                align_to: None,
                below: None,
                word: None,
                pressed: None,
                held: None,
                to: 1,
                on: None,
                after: Some(2),
                align: None,
                guard: None,
                actions: Vec::new(),
            }],
        };
        let flung = crate::spec::EntityState {
            name: "flung".into(),
            freeze: false,
            part: crate::spec::Part {
                enter_speed: Some([Fx::ZERO, n(-3)]),
                ..Default::default()
            },
            kind: off,
            transitions: Vec::new(),
        };
        s.kinds[0].machine = Some(crate::spec::EntityMachine {
            initial: 0,
            states: vec![free, flung],
        });
        let right = 1 << 7;
        let mut w = World::power_on(&s);
        step(&s, &mut w, right);
        assert!(w.bodies[0].vx > Fx::ZERO, "it runs at first");
        step(&s, &mut w, right);
        assert_eq!(w.bodies[0].state, 1);
        assert_eq!(w.bodies[0].vx, Fx::ZERO, "stopped on entry");
        assert!(w.bodies[0].vy < Fx::ZERO, "thrown up on entry");
        let x = w.bodies[0].x;
        step(&s, &mut w, right);
        assert_eq!(w.bodies[0].vx, Fx::ZERO, "right held does nothing");
        assert_eq!(w.bodies[0].x, x);

        // `enter_keeps_fraction`: only the whole pixels up and down are
        // set, so a fraction the speed had before the state stays.
        let entered = |s: &Spec| {
            let mut w = World::power_on(s);
            step(s, &mut w, right);
            w.bodies[0].vy = Fx(0x6000);
            step(s, &mut w, right);
            assert_eq!(w.bodies[0].state, 1);
            w.bodies[0].vy
        };
        let whole = entered(&s);
        let m = s.kinds[0].machine.as_mut().unwrap();
        m.states[1].part.enter_keeps_fraction = true;
        let kept = entered(&s);
        assert_eq!(kept.sub(whole), Fx(0x6000), "the fraction it had stays");
    }

    /// A camera `by_move` with `keep_move` moves by the entity's last move
    /// while a state stops the player's system; `seen_every` keeps the
    /// entity's place in the view on even frames only; `keep_in_view` puts
    /// it back at the view's edge with its speed's steps cleared.
    #[test]
    fn a_camera_keeps_the_last_move_sees_on_drawn_frames_and_keeps_the_player_in_view() {
        let mut s = quiet(spec());
        s.levels[0].size[0] = 64;
        s.levels[0].tiles = vec![Tile::Empty; 64 * 4];
        for c in 0..64 {
            s.levels[0].tiles[3 * 64 + c] = Tile::Solid;
        }
        s.cameras[0].scroll_back = false;
        s.cameras[0].by_move = Some(crate::spec::ByMove {
            from: n(0),
            full: n(0),
            short_by: n(0),
            push_hold: 0,
            keep_move: true,
            hold_with_counters: false,
        });
        s.kinds[0].machine = Some(crate::spec::EntityMachine {
            initial: 0,
            states: vec![
                crate::spec::EntityState {
                    name: "free".into(),
                    freeze: false,
                    part: Default::default(),
                    kind: s.kinds[0].clone(),
                    transitions: Vec::new(),
                },
                crate::spec::EntityState {
                    name: "still".into(),
                    freeze: false,
                    part: crate::spec::Part {
                        stops: vec![System::Player],
                        seen_every: 2,
                        ..Default::default()
                    },
                    kind: s.kinds[0].clone(),
                    transitions: Vec::new(),
                },
            ],
        });
        let right = 1 << 7;
        let mut w = World::power_on(&s);
        for _ in 0..4 {
            step(&s, &mut w, right);
        }
        assert_eq!(w.last_move[0], [1, 0]);
        let (hero, cam) = (w.bodies[0].x, w.cameras[0][0]);
        w.bodies[0].state = 1;
        let seen = w.seen[0][0];
        step(&s, &mut w, right);
        assert_eq!(w.bodies[0].x, hero, "the player is stopped");
        assert_eq!(
            w.cameras[0][0],
            cam.add(n(1)),
            "the camera moves by the last move"
        );
        let after = (w.frame, w.seen[0][0]);
        step(&s, &mut w, right);
        let seen_now = w.seen[0][0];
        // Seen only on even frames.
        if after.0.is_multiple_of(2) {
            assert_ne!(after.1, seen);
            assert_eq!(seen_now, after.1);
        } else {
            assert_eq!(after.1, seen);
            assert_ne!(seen_now, after.1);
        }
        // In the view from 0 to 20: with the camera still, the player is
        // put back at 20 and its speed cleared while right is held.
        w.bodies[0].state = 0;
        s.cameras[0].keep_in_view = Some([0, 20]);
        s.cameras[0].by_move = None;
        s.cameras[0].lead = [n(0), n(0)];
        s.cameras[0].axis_x = false;
        w.cameras[0][0] = n(0);
        w.bodies[0].x = Fx::from_int(20).add(Fx(Fx::ONE.raw() / 4));
        w.bodies[0].vx = n(1);
        step(&s, &mut w, right);
        assert_eq!(
            w.bodies[0].x,
            Fx::from_int(20).add(Fx(Fx::ONE.raw() / 4)),
            "back at 20, the fraction kept"
        );
        assert_eq!(w.bodies[0].vx, Fx::ZERO, "toward the edge: no speed");
        w.bodies[0].vx = n(-1);
        step(&s, &mut w, 1 << 6);
        assert_eq!(
            w.bodies[0].vx,
            n(-1),
            "held away from the edge: the speed stays"
        );
        // At the first bound, not past it: only `keep_in_view_touch` stops it.
        let at_edge = |touch: bool, held: u8| {
            let mut s = s.clone();
            s.cameras[0].keep_in_view = Some([0, 20]);
            s.cameras[0].keep_in_view_touch = touch;
            let mut w = w.clone();
            w.bodies[0].x = Fx::from_int(1).add(Fx(Fx::ONE.raw() / 4));
            w.bodies[0].vx = n(-1);
            step(&s, &mut w, held);
            (w.bodies[0].x, w.bodies[0].vx)
        };
        let place = Fx(Fx::ONE.raw() / 4);
        let left = 1 << 6;
        assert_eq!(at_edge(false, left), (place, n(-1)), "off: not stopped");
        assert_eq!(at_edge(true, left), (place, Fx::ZERO), "on: stopped");
        assert_ne!(
            at_edge(true, right).1,
            Fx::ZERO,
            "held away: it keeps a speed"
        );
    }

    /// A hook on the patroller: every 5 frames it spawns another patroller
    /// 32 pixels right, counts in its own words, and fires an event; the
    /// spawned bodies come at the end in request order.
    struct Spawner;

    impl crate::ext::Ext for Spawner {
        fn step(&self, cx: &mut crate::ext::ExtCx<'_>) {
            cx.me.ext[0] += 1;
            if cx.me.ext[0] % 5 == 0 {
                let kind = cx.kind("bug").unwrap();
                let (x, y) = (cx.me.x.add(n(32)), cx.me.y);
                cx.spawn(kind, x, y, true);
                let e = cx.event("goal.reached").unwrap();
                let id = cx.id;
                cx.fire(e, Some(id));
                cx.me.ext[1] = cx.rng.below(100) as i64;
            }
        }
    }

    /// Core `entity.hook_through_stops`, on a made-up counting hook: under
    /// a rule state that stops `platformer.enemies` only the flagged
    /// entity's hook runs, and under a freeze neither does.
    struct Counts;

    impl crate::ext::Ext for Counts {
        fn step(&self, cx: &mut crate::ext::ExtCx<'_>) {
            cx.me.ext[0] += 1;
            if cx.system_stopped {
                cx.me.ext[1] += 1;
            }
        }
    }

    /// A hook that writes its count to pulse 1's volume register.
    struct Beeps;

    impl crate::ext::Ext for Beeps {
        fn step(&self, cx: &mut crate::ext::ExtCx<'_>) {
            cx.me.ext[0] += 1;
        }

        fn apu(&self, _: &Spec, world: &World, id: usize) -> Vec<(u8, u8)> {
            vec![(0x00, world.bodies[id].ext[0] as u8)]
        }
    }

    #[test]
    fn hook_apu_gives_each_hooks_writes_for_the_next_frame_and_changes_nothing() {
        let mut s = quiet(spec());
        s.kinds[1].ext = Some("beeps".into());
        let mut exts = crate::ext::Exts::new();
        exts.insert("beeps".into(), Box::new(Beeps));
        let mut w = World::power_on(&s);
        for _ in 0..3 {
            step_with_ext(&s, &mut w, 0, &exts);
        }
        let hooked = w
            .bodies
            .iter()
            .filter(|b| b.kind == 1 && b.active())
            .count();
        let before = w.hash();
        let writes = crate::ext::hook_apu(&s, &w, &exts);
        assert_eq!(w.hash(), before);
        assert_eq!(writes.len(), hooked);
        assert!(writes.iter().all(|&(r, v)| r == 0 && v == 3));
        assert!(crate::ext::hook_apu(&s, &w, &crate::ext::Exts::new()).is_empty());
    }

    #[test]
    fn a_hook_through_stops_runs_while_a_state_stops_its_system() {
        let mut s = quiet(spec());
        s.kinds[1].ext = Some("counts".into());
        let mut exts = crate::ext::Exts::new();
        exts.insert("counts".into(), Box::new(Counts));
        let run = |s: &Spec, stops: bool, freeze: bool| {
            let mut s = s.clone();
            s.machines = vec![crate::spec::Machine {
                name: "game".into(),
                states: vec![crate::spec::State {
                    name: "still".into(),
                    freeze,
                    part: crate::spec::Part {
                        stops: if stops {
                            vec![System::Enemies]
                        } else {
                            Vec::new()
                        },
                        ..Default::default()
                    },
                    transitions: Vec::new(),
                }],
                initial: 0,
            }]
            .into();
            let mut w = World::power_on(&s);
            for _ in 0..10 {
                step_with_ext(&s, &mut w, 0, &exts);
            }
            let b = w.bodies.iter().find(|b| b.kind == 1).unwrap();
            // Every step it ran was one the state stopped its system on.
            if stops {
                assert_eq!(b.ext[1], b.ext[0]);
            }
            b.ext[0]
        };
        let plain = run(&s, true, false);
        let mut through = s.clone();
        through.kinds[1].hook_through_stops = true;
        assert_eq!(plain, 0, "a stopped system runs no hook");
        assert!(run(&through, true, false) >= 9, "the flagged hook runs");
        assert_eq!(run(&through, false, true), 0, "a freeze stops it too");
    }

    /// A hook that counts its frame starts in word 2.
    struct Starts;

    impl crate::ext::Ext for Starts {
        fn step(&self, cx: &mut crate::ext::ExtCx<'_>) {
            cx.me.ext[0] += 1;
        }

        fn frame_start(&self, cx: &mut crate::ext::ExtCx<'_>) {
            cx.me.ext[2] += 1;
        }
    }

    #[test]
    fn a_frame_start_hook_runs_every_frame_frozen_or_not() {
        let mut s = quiet(spec());
        s.kinds[1].ext = Some("starts".into());
        s.machines = vec![crate::spec::Machine {
            name: "game".into(),
            states: vec![crate::spec::State {
                name: "frozen".into(),
                freeze: true,
                part: crate::spec::Part::default(),
                transitions: Vec::new(),
            }],
            initial: 0,
        }]
        .into();
        let mut exts = crate::ext::Exts::new();
        exts.insert("starts".into(), Box::new(Starts));
        let run = |s: &Spec| {
            let mut w = World::power_on(s);
            for _ in 0..10 {
                step_with_ext(s, &mut w, 0, &exts);
            }
            let b = w.bodies.iter().find(|b| b.kind == 1).unwrap();
            (b.ext[0], b.ext[2])
        };
        assert_eq!(run(&s), (0, 0), "without the flag, nothing runs frozen");
        s.kinds[1].hook_frame_start = true;
        let (steps, starts) = run(&s);
        assert_eq!(steps, 0, "the freeze still stops the step");
        assert!(starts >= 9, "the frame start runs on every frozen frame");
    }

    /// Core `entity.hook_first` and `ExtCx::hold`, on a made-up hook that
    /// holds right: it runs before the player moves, so the hero walks
    /// right on a frame nothing is pressed, and not in the enemies'
    /// system after; the next frame counts right as held before.
    struct HoldsRight;

    impl crate::ext::Ext for HoldsRight {
        fn step(&self, cx: &mut crate::ext::ExtCx<'_>) {
            cx.me.ext[0] += 1;
            cx.hold("right", true);
            cx.hold("jump", false);
        }
    }

    #[test]
    fn a_first_hook_holds_an_action_before_the_player_moves() {
        let mut s = quiet(spec());
        s.kinds[1].ext = Some("holds".into());
        s.kinds[1].hook_first = true;
        let mut exts = crate::ext::Exts::new();
        exts.insert("holds".into(), Box::new(HoldsRight));
        let mut w = World::power_on(&s);
        let x0 = w.bodies[0].x;
        for _ in 0..10 {
            step_with_ext(&s, &mut w, 0, &exts);
        }
        let hook = w.bodies.iter().find(|b| b.kind == 1).unwrap();
        assert_eq!(hook.ext[0], 10, "the hook runs once a frame");
        assert!(w.bodies[0].x > x0, "the hero walked right");
        assert_eq!(w.held & (1 << 7), 1 << 7);
        assert_eq!(w.prev & (1 << 7), 1 << 7);
    }

    /// Core `input.turn`: while the turn's counter is 1 the hero's actions
    /// read controller 2, and controller 1's right does nothing; the
    /// actions not listed still read controller 1.
    #[test]
    fn the_turn_s_player_plays_from_their_own_controller() {
        let mut s = quiet(spec());
        s.counters.push(Counter {
            name: "player".into(),
            start: 1,
            changes: vec![],
            min: None,
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
            restarts_clock: false,
            high_of: None,
        });
        s.turn = Some((s.counters.len() - 1, 1 << 7));
        let w = World::power_on(&s);
        assert_eq!(turn_pad(&s, &w, [0x80, 0]), 0);
        assert_eq!(turn_pad(&s, &w, [0, 0x80]), 0x80);
        assert_eq!(turn_pad(&s, &w, [0x41, 0x80]), 0xC1);
        let walk = |pads: [u8; 2]| {
            let mut w = World::power_on(&s);
            let x0 = w.bodies[0].x;
            for _ in 0..10 {
                step_pads(&s, &mut w, pads, &crate::ext::Exts::new(), &mut None);
            }
            w.bodies[0].x > x0
        };
        assert!(!walk([0x80, 0]), "controller 1 isn't the turn's");
        assert!(walk([0, 0x80]), "controller 2 is");
    }

    /// Feature `transition.held`: taken on every frame the action is held,
    /// newly or not; `pressed` only on the first.
    #[test]
    fn a_held_transition_is_taken_every_frame_the_action_is_held() {
        let mut s = quiet(spec());
        s.counters.push(Counter {
            name: "seen".into(),
            start: 0,
            changes: vec![],
            min: None,
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
            restarts_clock: false,
            high_of: None,
        });
        let seen = s.counters.len() - 1;
        let t = |pressed: Option<usize>, held: Option<usize>| Transition {
            to: 0,
            on: None,
            after: None,
            align: None,
            align_to: None,
            below: None,
            word: None,
            pressed,
            held,
            guard: None,
            actions: vec![Do::Change(seen, 1)],
        };
        for (pressed, held, want) in [(None, Some(0), 5), (Some(0), None, 1)] {
            let mut s = s.clone();
            s.machines.push(crate::spec::Machine {
                name: "watch".into(),
                initial: 0,
                states: vec![State {
                    name: "on".into(),
                    freeze: false,
                    part: Default::default(),
                    transitions: vec![t(pressed, held)],
                }],
            });
            let mut w = World::power_on(&s);
            for b in [1, 1, 1, 1, 1, 0, 0] {
                step(&s, &mut w, b);
            }
            assert_eq!(w.counters[seen], want);
        }
    }

    #[test]
    fn ext_hooks_change_their_body_spawn_and_fire() {
        let mut s = spec();
        s.kinds[1].ext = Some("spawner".into());
        // Only the level's own bug runs the hook: spawned ones are a kind
        // without it.
        let mut plain = s.kinds[1].clone();
        plain.ext = None;
        plain.name = "bug".into();
        s.kinds[1].name = "spawner_bug".into();
        s.kinds.push(plain);
        let mut exts = crate::ext::Exts::new();
        exts.insert("spawner".into(), Box::new(Spawner));
        let run = |frames: usize| {
            let mut w = World::power_on(&s);
            let mut fired = 0;
            for _ in 0..frames {
                let events = step_with_ext(&s, &mut w, 0, &exts);
                fired += events
                    .iter()
                    .filter(|e| e.kind == EventKind::GOAL_REACHED)
                    .count();
            }
            (w, fired)
        };
        let (w, fired) = run(12);
        assert_eq!(w.bodies.len(), 4, "two spawned, at frames 5 and 10");
        assert_eq!(fired, 2);
        assert_eq!(w.bodies[1].ext[0], 12);
        assert_eq!(w.bodies[2].kind, 2);
        assert!(w.bodies[2].facing_left && w.bodies[3].facing_left);
        // Deterministic, and the canonical state holds the hook's words and
        // the spawned bodies.
        let (again, _) = run(12);
        assert_eq!(w.hash(), again.hash());
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back, w);
        // Without the hook, nothing of it happens.
        let mut plain_world = World::power_on(&s);
        for _ in 0..12 {
            step(&s, &mut plain_world, 0);
        }
        assert_eq!(plain_world.bodies.len(), 2);
    }

    /// A hook that counts its `appear`s in one word and its `step`s in the
    /// other.
    struct Appears;

    impl crate::ext::Ext for Appears {
        fn step(&self, cx: &mut crate::ext::ExtCx<'_>) {
            cx.me.ext[0] += 1;
        }

        fn appear(&self, cx: &mut crate::ext::ExtCx<'_>) {
            cx.me.ext[1] += 1;
        }
    }

    /// A body held still on the frame it appears runs the hook's `appear`
    /// then, and its `step` from the next frame.
    #[test]
    fn a_body_held_still_as_it_appears_runs_its_hooks_appear() {
        let mut s = view_spec(&[(200, false)]);
        s.levels[0].appear_still = true;
        s.kinds[1].ext = Some("appears".into());
        let mut exts = crate::ext::Exts::new();
        exts.insert("appears".into(), Box::new(Appears));
        let run = || {
            let mut w = World::power_on(&s);
            let mut f = at_view(&s, &mut w, 200 - s.cameras[0].size[0] - 15);
            enemy(&s, &mut w, &mut f, 1, &exts);
            assert_eq!(f.still, vec![1]);
            let first = w.bodies[1].ext;
            enemy(&s, &mut w, &mut Frame::default(), 1, &exts);
            (first, w)
        };
        let (first, w) = run();
        assert_eq!(&first[..2], [0, 1]);
        assert_eq!(&w.bodies[1].ext[..2], [1, 1]);
        assert_eq!(w.hash(), run().1.hash());
    }

    /// A hook reads its numbers from the kind's `ext_params`.
    struct Pusher;

    impl crate::ext::Ext for Pusher {
        fn step(&self, cx: &mut crate::ext::ExtCx<'_>) {
            let push = cx.param("push");
            assert!(cx.has_param("push") && !cx.has_param("pull"));
            cx.me.x = cx.me.x.add(push);
        }
    }

    #[test]
    fn ext_hooks_read_their_params() {
        let mut s = spec();
        s.kinds[1].ext = Some("pusher".into());
        s.kinds[1].ext_params.insert("push".into(), n(3));
        let mut exts = crate::ext::Exts::new();
        exts.insert("pusher".into(), Box::new(Pusher));
        let mut w = World::power_on(&s);
        let mut plain = World::power_on(&s);
        step_with_ext(&s, &mut w, 0, &exts);
        step(&s, &mut plain, 0);
        assert_eq!(w.bodies[1].x.sub(plain.bodies[1].x), n(3));
    }

    #[test]
    #[should_panic(expected = "no `ext_params.push`")]
    fn an_unknown_param_panics() {
        let mut s = spec();
        s.kinds[1].ext = Some("pusher".into());
        let mut exts = crate::ext::Exts::new();
        exts.insert("pusher".into(), Box::new(Pusher));
        let mut w = World::power_on(&s);
        step_with_ext(&s, &mut w, 0, &exts);
    }

    /// A level that starts mid-frame, with fewer bodies than the one
    /// before, ends the moving systems for that frame instead of reaching
    /// past the new bodies.
    #[test]
    fn a_level_started_mid_frame_stops_the_moving_systems() {
        let mut s = spec();
        s.levels[0].goal_x = Some(n(8));
        s.order.push(System::Exits);
        let mut second = s.levels[0].clone();
        second.name = "two".into();
        second.spawns.truncate(1);
        second.goal_x = None;
        s.levels[0].next = Some(1);
        s.levels.push(second);
        s.machines[0].states[0].transitions.push(Transition {
            align_to: None,
            below: None,
            word: None,
            pressed: None,
            held: None,
            to: 0,
            on: Some(EventKind::GOAL_REACHED),
            after: None,
            align: None,
            guard: None,
            actions: vec![Do::NextLevel],
        });
        let mut w = World::power_on(&s);
        let mut started = None;
        for i in 1..=20 {
            let events = step(&s, &mut w, 1 << 7);
            if events.iter().any(|e| e.kind == EventKind::LEVEL_STARTED) {
                started = Some(i);
                break;
            }
        }
        assert!(started.is_some(), "the hero reaches the goal");
        assert_eq!((w.level, w.bodies.len()), (1, 1));
        assert_eq!(w.bodies[0].x, n(0), "nothing moved after the level started");
        step(&s, &mut w, 1 << 7);
        assert!(w.bodies[0].x > n(0));
    }

    /// Core `stages` with `action.respawn`: once the counter names a stage
    /// another level plays, respawn puts the bodies at that level's spawns,
    /// before next-level starts it.
    #[test]
    fn respawn_takes_the_stages_level_spawns() {
        let mut s = spec();
        let mut second = s.levels[0].clone();
        second.name = "two".into();
        second.spawns[0].x = n(40);
        s.levels.push(second);
        s.counters = vec![Counter {
            restarts_clock: false,
            high_of: None,
            name: "stage".into(),
            start: 0,
            changes: Vec::new(),
            min: Some(0),
            max: Some(1),
            wrap: true,
            runs_when_frozen: false,
            shift: None,
        }]
        .into();
        s.stages = Some((0, vec![Some(0), Some(1)]));
        let mut w = World::power_on(&s);
        let home = w.bodies[0].x;
        let mut f = begin_frame(&s, &mut w, 0);
        act(&s, &mut w, &mut f, Do::Respawn(0));
        assert_eq!(w.bodies[0].x, home, "stage 0 is the level being played");
        w.counters[0] = 1;
        act(&s, &mut w, &mut f, Do::Respawn(0));
        assert_eq!(
            (w.level, w.bodies[0].x),
            (0, n(40)),
            "stage 1's place, level 0 still played"
        );
    }

    /// Core `stages`: next-level starts the level that plays the stage the
    /// counter names, a stand-in for a stage the model hasn't, and
    /// restart-level starts that stage's level again.
    #[test]
    fn stages_start_the_counters_stage_or_its_stand_in() {
        let mut s = spec();
        s.levels[0].goal_x = Some(n(8));
        s.order.push(System::Exits);
        let mut second = s.levels[0].clone();
        second.name = "two".into();
        s.levels.push(second);
        s.counters = vec![Counter {
            restarts_clock: false,
            high_of: None,
            name: "stage".into(),
            start: 0,
            changes: Vec::new(),
            min: Some(0),
            max: Some(2),
            wrap: true,
            runs_when_frozen: false,
            shift: None,
        }]
        .into();
        // Stage 0 is level 0, stage 1 level 1, stage 2 isn't modelled and
        // level 0 stands in for it.
        s.stages = Some((0, vec![Some(0), Some(1), Some(0)]));
        let mut w = World::power_on(&s);
        w.counters[0] = 1;
        let mut f = begin_frame(&s, &mut w, 0);
        act(&s, &mut w, &mut f, Do::NextLevel);
        assert_eq!((w.level, w.home_level), (1, 1));
        w.counters[0] = 2;
        let mut f = begin_frame(&s, &mut w, 0);
        act(&s, &mut w, &mut f, Do::NextLevel);
        assert_eq!(
            (w.level, w.home_level),
            (0, 0),
            "the stand-in plays stage 2"
        );
        w.counters[0] = 1;
        let mut f = begin_frame(&s, &mut w, 0);
        act(&s, &mut w, &mut f, Do::RestartLevel);
        assert_eq!(w.level, 1, "a restart starts the counter's stage");
    }

    /// Core `stages.skip`: a step of the stage counter onto a stage the
    /// game passes over steps on past it, through the counter's wrap, and
    /// next-level starts the stage after it, as Donkey Kong (JU) goes from
    /// 25m to 75m with no 50m.
    #[test]
    fn a_skipped_stage_is_stepped_past() {
        let mut s = spec();
        let mut second = s.levels[0].clone();
        second.name = "two".into();
        s.levels.push(second);
        s.counters = vec![Counter {
            restarts_clock: false,
            high_of: None,
            name: "stage".into(),
            start: 0,
            changes: Vec::new(),
            min: Some(0),
            max: Some(3),
            wrap: true,
            runs_when_frozen: false,
            shift: None,
        }]
        .into();
        // Stage 1 is passed over, and so is stage 3, the last.
        s.stages = Some((0, vec![Some(0), None, Some(1), None]));
        let mut w = World::power_on(&s);
        let mut f = begin_frame(&s, &mut w, 0);
        act(&s, &mut w, &mut f, Do::Change(0, 1));
        assert_eq!(w.counters[0], 2, "stage 1 is stepped past");
        act(&s, &mut w, &mut f, Do::NextLevel);
        assert_eq!(w.level, 1, "stage 2's level starts");
        act(&s, &mut w, &mut f, Do::Change(0, 1));
        assert_eq!(
            w.counters[0], 0,
            "stage 3 is stepped past, through the wrap"
        );
    }

    // Core 0.4.0 and platformer 0.4.0, on the small game.

    fn quiet(mut s: Spec) -> Spec {
        // The bug stands still and touches nothing unless a test says so.
        s.kinds[1].patrol = None;
        s.kinds[1].contact = None;
        s.levels[0].kill_below = None;
        s
    }

    /// Platformer 0.4.19's fall `feet` and `stands_still`, on made-up
    /// numbers: the floor's top is 48 and it ends at x 32.
    #[test]
    fn a_fall_looks_at_its_feet_and_stands_still_until_the_ground_goes() {
        let mut s = quiet(spec());
        let width = s.levels[0].size[0] as usize;
        for c in 2..width {
            s.levels[0].tiles[3 * width + c] = Tile::Empty;
        }
        s.kinds[1].tile_collision = true;
        let fall = |feet: Option<Vec<Fx>>, stands_still: bool| crate::spec::Fall {
            gravity: q(0.25),
            max_fall: n(3),
            max_fall_from_half: false,
            move_first: false,
            feet,
            stands_still,
        };
        let step_at = |s: &Spec, x: i64, vy: Fx| {
            let mut w = World::power_on(s);
            let b = &mut w.bodies[1];
            (b.x, b.y, b.vy, b.on_ground) = (n(x), n(32), vy, true);
            enemy(s, &mut w, &mut Frame::default(), 1, &Exts::new());
            w.bodies[1]
        };
        // At x 26 the hitbox is over the floor's last column, but a foot
        // at 8 from x is past its edge.
        s.kinds[1].fall = Some(fall(None, false));
        assert!(step_at(&s, 26, n(0)).on_ground);
        s.kinds[1].fall = Some(fall(Some(vec![n(8)]), false));
        assert!(!step_at(&s, 26, n(0)).on_ground);
        assert!(step_at(&s, 20, n(0)).on_ground);
        // Standing still it only looks for the ground: it keeps its place
        // and its speed this frame, and falls from the next.
        s.kinds[1].fall = Some(fall(Some(vec![n(8)]), true));
        let b = step_at(&s, 20, q(0.5));
        assert!(b.on_ground);
        assert_eq!((b.y, b.vy), (n(32), q(0.5)));
        let b = step_at(&s, 26, n(0));
        assert!(!b.on_ground);
        assert_eq!((b.y, b.vy), (n(32), n(0)), "no move on the frame it goes");
    }

    #[test]
    fn land_first_lands_and_hops_before_the_move() {
        let mut s = quiet(spec());
        s.kinds[1].tile_collision = true;
        s.kinds[1].fall = Some(crate::spec::Fall {
            gravity: q(0.125),
            max_fall: n(3),
            max_fall_from_half: false,
            move_first: true,
            feet: None,
            stands_still: false,
        });
        s.kinds[1].hop = Some(crate::spec::Hop {
            speed: n(-3),
            pattern: Vec::new(),
            every: 0,
            phase: 0,
            land_first: true,
            land_while_stopped: false,
        });
        let after = |y: Fx, vy: Fx| {
            let mut w = World::power_on(&s);
            let b = &mut w.bodies[1];
            (b.x, b.y, b.vy, b.on_ground) = (n(16), y, vy, true);
            enemy(&s, &mut w, &mut Frame::default(), 1, &Exts::new());
            (w.bodies[1].y, w.bodies[1].vy)
        };
        // The floor's top is 48: standing at 32 and falling, it hops and
        // rises on the same frame.
        assert_eq!(after(n(32), n(3)), (n(29), n(-3).add(q(0.125))));
        // Rising at 3 lands too (the game's unsigned compare); at 0 it
        // doesn't, and sinks by its speed.
        assert_eq!(after(n(32), n(-3)).0, n(29));
        assert_eq!(after(n(32), n(0)), (n(32), q(0.125)));
        // Sunk into the floor, it is put on its top, the fraction kept.
        assert_eq!(after(q(34.5), n(1)).0, q(29.5));
    }

    /// `land_while_stopped`: a stopped frame lands a hopper, without moving
    /// it, as EnemyToBGCollisionDet runs before TimerControl's SkipMove.
    #[test]
    fn land_while_stopped_lands_a_hopper_on_a_stopped_frame_without_moving_it() {
        let after = |on: bool| {
            let mut s = quiet(spec());
            s.kinds[1].tile_collision = true;
            s.kinds[1].hop = Some(crate::spec::Hop {
                speed: n(-3),
                pattern: Vec::new(),
                every: 0,
                phase: 0,
                land_first: true,
                land_while_stopped: on,
            });
            let mut w = World::power_on(&s);
            let b = &mut w.bodies[1];
            (b.x, b.y, b.vy) = (n(16), q(33.5), n(3));
            let mut f = Frame {
                stopped: vec![System::Enemies],
                ..Frame::default()
            };
            run_frozen(System::Enemies, &s, &mut w, &mut f);
            (w.bodies[1].y, w.bodies[1].vy)
        };
        assert_eq!(after(false), (q(33.5), n(3)));
        assert_eq!(after(true), (q(32.5), n(-3)));
        assert_eq!(after_appearing(), (q(33.5), n(3)));
    }

    /// A hopper that appeared on a stopped frame has its first turn on the
    /// next frame.
    fn after_appearing() -> (Fx, Fx) {
        let mut s = quiet(spec());
        s.kinds[1].tile_collision = true;
        s.kinds[1].hop = Some(crate::spec::Hop {
            speed: n(-3),
            pattern: Vec::new(),
            every: 0,
            phase: 0,
            land_first: true,
            land_while_stopped: true,
        });
        let mut w = World::power_on(&s);
        let b = &mut w.bodies[1];
        (b.x, b.y, b.vy) = (n(16), q(33.5), n(3));
        let mut f = Frame {
            stopped: vec![System::Enemies],
            still: vec![1],
            ..Frame::default()
        };
        run_frozen(System::Enemies, &s, &mut w, &mut f);
        (w.bodies[1].y, w.bodies[1].vy)
    }

    #[test]
    fn turn_after_move_steps_into_the_wall_and_turns_there() {
        let mut s = quiet(spec());
        s.kinds[1].tile_collision = true;
        s.kinds[1].walk = Some(crate::spec::Walk {
            speed: n(1),
            turn_at_ledges: false,
            walls_at_top: false,
            turn_after_move: true,
            ledge_at: None,
            bump: None,
        });
        let mut w = World::power_on(&s);
        // Its top row is in the floor's row: the point ahead is solid.
        let b = &mut w.bodies[1];
        (b.x, b.y, b.facing_left) = (n(16), n(48), false);
        walk(&s, &mut w, 1);
        assert_eq!(w.bodies[1].x, n(17));
        assert!(w.bodies[1].facing_left);
    }

    /// `ledge_at`: the ledge is one point under the walker after its step,
    /// and it turns where it stands, as ChkUnderEnemy and ChkForRedKoopa.
    #[test]
    fn ledge_at_turns_on_the_point_after_the_move() {
        let step = |ledge_at: Option<Fx>, x: Fx, left: bool| {
            let mut s = quiet(spec());
            s.kinds[1].tile_collision = true;
            s.levels[0].tiles[3 * 8] = Tile::Empty;
            s.kinds[1].walk = Some(crate::spec::Walk {
                speed: q(0.5),
                turn_at_ledges: true,
                walls_at_top: false,
                turn_after_move: true,
                ledge_at,
                bump: None,
            });
            let mut w = World::power_on(&s);
            let b = &mut w.bodies[1];
            (b.x, b.y, b.on_ground, b.facing_left) = (x, n(32), true, left);
            walk(&s, &mut w, 1);
            (w.bodies[1].x, w.bodies[1].facing_left)
        };
        // Ground begins at 16. The corner turns it before it steps over.
        assert_eq!(step(None, n(16), true), (n(16), false));
        // The middle has ground: it steps on, and on past the corner.
        assert_eq!(step(Some(n(8)), n(16), true), (q(15.5), true));
        assert_eq!(step(Some(n(8)), q(8.5), true), (n(8), true));
        // The middle leaves the ground: it has stepped, and turns there.
        assert_eq!(step(Some(n(8)), n(8), true), (q(7.5), false));
        // The same point whichever way it faces.
        assert_eq!(step(Some(n(8)), n(7), false), (q(7.5), true));
        assert_eq!(step(Some(n(8)), q(7.5), false), (n(8), false));
    }

    #[test]
    fn walkers_that_touch_turn_once_on_the_frames_tested() {
        let mut s = quiet(spec());
        s.kinds[1].walk = Some(crate::spec::Walk {
            speed: n(1),
            turn_at_ledges: false,
            walls_at_top: false,
            turn_after_move: false,
            ledge_at: None,
            bump: Some(crate::spec::Bump {
                every: 2,
                phase: 1,
                view: None,
            }),
        });
        let mut w = World::power_on(&s);
        w.bodies.push(w.bodies[1]);
        let last = w.bodies.len() - 1;
        let box_w = s.kinds[1].hitbox.w.floor();
        w.bodies[1].x = n(100);
        w.bodies[last].x = n(100 + box_w);
        w.frame = 1;
        let before = w.bodies.clone();
        // An untested frame: nothing happens.
        w.frame = 2;
        bump(&s, &mut w, last, &before);
        assert!(w.bumps.is_empty());
        w.frame = 3;
        let (a, b) = (w.bodies[1].facing_left, w.bodies[last].facing_left);
        bump(&s, &mut w, last, &before);
        assert_eq!(w.bumps, vec![(1, last as u32)]);
        assert_eq!(w.bodies[1].facing_left, !a);
        assert_eq!(w.bodies[last].facing_left, !b);
        // Still touching: no second turn.
        w.frame = 5;
        bump(&s, &mut w, last, &before);
        assert_eq!(w.bodies[1].facing_left, !a);
        // Apart: forgotten, in the state's bytes both ways.
        let bytes = w.canonical();
        assert_eq!(World::from_canonical(&s, &bytes).unwrap().bumps, w.bumps);
        let mut apart = before.clone();
        apart[last].x = n(100 + box_w + 2);
        w.frame = 7;
        bump(&s, &mut w, last, &apart);
        assert!(w.bumps.is_empty());
    }

    #[test]
    fn walls_at_top_meet_walls_at_the_tile_hitboxs_top_row_only() {
        let turned = |top: bool| {
            let mut s = quiet(spec());
            s.kinds[1].tile_collision = true;
            s.kinds[1].walk = Some(crate::spec::Walk {
                speed: n(1),
                turn_at_ledges: false,
                walls_at_top: top,
                turn_after_move: false,
                ledge_at: None,
                bump: None,
            });
            let mut w = World::power_on(&s);
            let b = &mut w.bodies[1];
            // 4 pixels into the floor, walking right.
            (b.x, b.y, b.facing_left) = (n(16), n(36), false);
            walk(&s, &mut w, 1);
            w.bodies[1].facing_left
        };
        assert!(turned(false), "the hitbox's lower rows are in the floor");
        assert!(!turned(true));
    }

    #[test]
    fn a_despawn_wrap_moves_the_limit_while_the_view_is_late_in_its_page() {
        let mut s = view_spec(&[(100, false)]);
        s.levels[0].despawn_behind = Some(n(72));
        s.levels[0].despawn_by_position = true;
        s.kinds[1].despawn_behind = Some(n(16));
        s.kinds[1].despawn_behind_wrap = Some([199, 271]);
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 40);
        w.bodies[1].x = n(150);
        at_view(&s, &mut w, 200);
        assert!(w.bodies[1].alive, "199 or more into the page: 271 behind");
        at_view(&s, &mut w, 198);
        assert!(!w.bodies[1].alive, "earlier in the page: 16 behind");
    }

    #[test]
    fn a_slots_word_outlives_its_bodies_and_the_canonical_state() {
        let mut s = view_spec(&[(100, false), (300, false)]);
        s.levels[0].spawn_slots = Some(1);
        s.levels[0].slot_rest = true;
        let mut w = World::power_on(&s);
        let width = s.cameras[0].size[0];
        at_view(&s, &mut w, 100 - width);
        w.slots[0].word = 77;
        w.bodies[1].alive = false;
        at_view(&s, &mut w, 300 - width);
        assert_eq!(w.slots[0].body, 3);
        assert_eq!(w.slots[0].word, 77);
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back.slots[0].word, 77);
    }

    #[test]
    fn numbered_slots_give_a_new_body_what_the_last_left() {
        let mut s = view_spec(&[(100, false), (300, false)]);
        s.levels[0].spawn_slots = Some(1);
        s.levels[0].slot_rest = true;
        let mut w = World::power_on(&s);
        assert_eq!(w.slots.len(), 1);
        let width = s.cameras[0].size[0];
        at_view(&s, &mut w, 100 - width);
        assert_eq!(w.slots[0].body, 2, "the first spawn's body in slot 1");
        // It leaves with a fraction across and a fall.
        w.bodies[1].x = w.bodies[1].x.add(q(0.5));
        w.bodies[1].y = w.bodies[1].y.add(q(0.25));
        w.bodies[1].vy = n(3);
        w.bodies[1].alive = false;
        at_view(&s, &mut w, 300 - width);
        assert_eq!(w.slots[0].body, 3);
        assert_eq!(w.bodies[2].vy, n(3));
        assert_eq!(Fx(w.bodies[2].x.frac()), q(0.5));
        // Only a body that falls takes the height's fraction.
        assert_eq!(Fx(w.bodies[2].y.frac()), q(0.0));
        s.kinds[1].fall = Some(crate::spec::Fall {
            gravity: q(0.125),
            max_fall: n(3),
            max_fall_from_half: false,
            move_first: true,
            feet: None,
            stands_still: false,
        });
        s.levels[0].tiles.fill(Tile::Empty);
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 100 - width);
        w.bodies[1].y = w.bodies[1].y.add(q(0.25));
        w.bodies[1].alive = false;
        at_view(&s, &mut w, 300 - width);
        assert_eq!(Fx(w.bodies[2].y.frac()), q(0.25));
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back.slots, w.slots);
    }

    /// A world with a launcher at x 64 in entry 0 whose timer reads
    /// `timer`, a shot kind that flies at 1, one numbered slot, a shift
    /// counter of all ones (so the launcher is picked every frame) and the
    /// next-entry counter. The player is at x 0 unless moved.
    fn launcher_world(timer: i64, near: u32) -> (Spec, World) {
        let mut s = quiet(spec());
        let counter = |name: &str, shift: Option<crate::spec::Shift>| Counter {
            restarts_clock: false,
            high_of: None,
            name: name.into(),
            start: 0,
            changes: vec![],
            min: None,
            max: None,
            wrap: false,
            runs_when_frozen: true,
            shift,
        };
        let random = s.counters.len();
        s.counters.push(Counter {
            start: 0xFF,
            ..counter(
                "random",
                Some(crate::spec::Shift {
                    bits: 8,
                    taps: vec![0],
                }),
            )
        });
        let next = s.counters.len();
        s.counters.push(counter("next", None));
        let mut shot = s.kinds[1].clone();
        shot.name = "shot".into();
        shot.fly = Some(crate::spec::Fly {
            speed: n(1),
            heading: crate::spec::Heading::Player,
            keep_speed: false,
        });
        shot.launched = true;
        let shot_kind = s.kinds.len();
        s.kinds.push(shot);
        let mut launcher = s.kinds[1].clone();
        launcher.name = "launcher".into();
        launcher.no_slot = true;
        launcher.launcher = Some(crate::spec::Launcher {
            fires: shot_kind,
            counter: random,
            at: 0,
            slot_step: 0,
            mask: 0,
            hard_mask: None,
            slots: 1,
            table: 2,
            next,
            rearm: 3,
            offset: [n(0), n(-8)],
            from_x: n(0),
            near,
            near_wrap: 256,
            behind: 8,
        });
        let launcher_kind = s.kinds.len();
        s.kinds.push(launcher);
        s.levels[0].spawn_slots = Some(1);
        s.levels[0].slot_rest = true;
        let mut w = World::power_on(&s);
        // The player at the view's left edge, the launcher 20 pixels right
        // of it, the other bodies out of the way.
        let (left, _) = view(&s, &w);
        for b in &mut w.bodies {
            if s.kinds[b.kind].role == Role::Player {
                b.x = left;
            } else {
                b.alive = false;
            }
        }
        let mut b = new_body(&s, &w, launcher_kind, left.add(n(20)), n(32), false);
        b.ext = [timer, 0, 0, 0];
        w.bodies.push(b);
        (s, w)
    }

    fn run_launchers(s: &Spec, w: &mut World, frozen: bool) {
        let mut f = Frame {
            out: vec![false; w.bodies.len()],
            fresh_bodies: vec![false; w.bodies.len()],
            ..Frame::default()
        };
        let held = shot_slots(s, w);
        launchers(s, w, &mut f, &Exts::new(), &held, frozen);
    }

    /// Platformer 0.4.28's launcher, on made-up numbers: a pick counts the
    /// timer down, at 0 it fires into the free slot and rearms, and the
    /// shot stands that frame, then flies toward the player.
    #[test]
    fn a_launcher_counts_down_then_fires_into_a_free_slot_and_rearms() {
        let (s, mut w) = launcher_world(1, 1);
        let bodies = w.bodies.len();
        let l = bodies - 1;
        run_launchers(&s, &mut w, false);
        assert_eq!(w.bodies[l].ext[0], 0, "counted down");
        assert_eq!(w.bodies.len(), bodies, "nothing fired yet");
        run_launchers(&s, &mut w, false);
        assert_eq!(w.bodies[l].ext[0], 3, "rearmed");
        let b = w.bodies[bodies];
        assert_eq!(s.kinds[b.kind].name, "shot");
        assert_eq!(
            (b.x, b.y),
            (w.bodies[l].x, n(24)),
            "at the launcher's offset"
        );
        assert_eq!(w.slots[0].body, bodies as u32 + 1, "in the slot");
        run_launchers(&s, &mut w, false);
        let b = w.bodies[bodies];
        assert!(b.alive, "aimed on its first turn");
        assert!(b.facing_left, "toward the player's side");
        assert_eq!(b.x, w.bodies[l].x.sub(n(1)), "and flies");
        assert_eq!(w.bodies[l].ext[0], 3, "a full slot picks no launcher");
    }

    #[test]
    fn a_shot_near_the_player_goes_on_its_first_turn_and_holds_its_slot_that_frame() {
        let (s, mut w) = launcher_world(0, 30);
        let bodies = w.bodies.len();
        let l = bodies - 1;
        run_launchers(&s, &mut w, false);
        assert!(w.bodies[bodies].alive);
        run_launchers(&s, &mut w, false);
        assert!(
            !w.bodies[bodies].alive,
            "20 pixels from the player, within 30"
        );
        assert_eq!(w.bodies[l].ext[0], 3, "its slot picks nothing that frame");
        run_launchers(&s, &mut w, false);
        assert_eq!(w.bodies[l].ext[0], 2, "the slot is free again");
    }

    #[test]
    fn a_launcher_that_appeared_this_frame_is_picked_from_the_next() {
        let (s, mut w) = launcher_world(0, 1);
        let bodies = w.bodies.len();
        let mut f = Frame {
            out: vec![false; bodies],
            fresh_bodies: vec![true; bodies],
            ..Frame::default()
        };
        let held = shot_slots(&s, &w);
        launchers(&s, &mut w, &mut f, &Exts::new(), &held, false);
        assert_eq!(w.bodies.len(), bodies, "not in the table yet");
        run_launchers(&s, &mut w, false);
        assert_eq!(w.bodies.len(), bodies + 1, "fired on the next frame");
    }

    #[test]
    fn a_launcher_counts_down_through_a_freeze_but_fires_nothing() {
        let (s, mut w) = launcher_world(1, 1);
        let bodies = w.bodies.len();
        run_launchers(&s, &mut w, true);
        run_launchers(&s, &mut w, true);
        assert_eq!(w.bodies[bodies - 1].ext[0], 0);
        assert_eq!(w.bodies.len(), bodies, "a freeze fires nothing");
    }

    #[test]
    fn a_launcher_that_appears_takes_the_next_entry_and_its_timer() {
        let (s, mut w) = launcher_world(5, 1);
        let old = w.bodies.len() - 1;
        let kind = w.bodies[old].kind;
        let next = s.kinds[kind].launcher.unwrap().next;
        let x = w.bodies[old].x.add(n(32));
        let mut b = new_body(&s, &w, kind, x, n(32), false);
        b.ext = [0, 0, 0, 0];
        w.bodies.push(b);
        let mut f = Frame {
            fresh_bodies: vec![false; old + 2],
            ..Frame::default()
        };
        take_entry(&s, &mut w, &mut f, old + 1);
        assert_eq!(w.bodies[old].ext, [5, 0, 0, 0], "entry 0 and the old timer");
        assert_eq!(
            w.bodies[old].x, x,
            "the entry's body moves to the new place"
        );
        assert!(w.bodies[old].alive, "the entry stays one body");
        assert!(f.fresh_bodies[old], "and isn't picked until the next frame");
        assert!(!w.bodies[old + 1].alive, "the appearing body ends");
        assert_eq!(w.counters[next], 1);
    }

    /// `extra_slots`: the body holds two slots, the second keeps what it
    /// had, and the next body takes the slot after them.
    #[test]
    fn a_body_with_extra_slots_takes_the_next_free_one_too() {
        let mut s = view_spec(&[(100, false), (300, false)]);
        s.levels[0].spawn_slots = Some(3);
        s.levels[0].slot_rest = true;
        s.kinds[1].extra_slots = 1;
        let mut w = World::power_on(&s);
        w.slots[1].rest = [q(0.25), n(1), q(0.5)];
        let width = s.cameras[0].size[0];
        at_view(&s, &mut w, 100 - width);
        assert_eq!(w.slots[0].body, 2);
        assert_eq!(w.slots[1].body, 2);
        assert_eq!(slots_taken(&s, &w), 2);
        at_view(&s, &mut w, 300 - width);
        assert_eq!(w.slots[2].body, 3, "the next body is one slot further on");
        assert_eq!(w.slots[0].body, 2);
        w.bodies[1].alive = false;
        release_slots(&s, &mut w);
        assert_eq!(w.slots[0].body, 0);
        assert_eq!(w.slots[1].body, 0);
        assert_eq!(w.slots[1].rest, [q(0.25), n(1), q(0.5)]);
    }

    /// `orbit.angle_in_slot`: a pivot's angle stays in its slot for the next
    /// one, and a slot's speed up and down is wrapped to the angles.
    #[test]
    fn a_pivot_starts_at_the_angle_its_slot_left() {
        let run = |on: bool, left: i64| {
            let mut s = view_spec(&[(100, false), (300, false)]);
            s.levels[0].spawn_slots = Some(1);
            s.levels[0].slot_rest = true;
            s.kinds[1].orbit = Some(Orbit {
                spin: Fx::ZERO,
                angles: 32,
                parts: 0,
                part: 1,
                place: [Fx::ZERO; 2],
                skip: Vec::new(),
                in_view: false,
                place_on_appear: true,
                angle_in_slot: on,
            });
            let mut w = World::power_on(&s);
            let width = s.cameras[0].size[0];
            at_view(&s, &mut w, 100 - width);
            w.bodies[1].ext[0] = left << 8;
            w.bodies[1].alive = false;
            at_view(&s, &mut w, 300 - width);
            w.bodies[2].ext[0] >> 8
        };
        assert_eq!(run(true, 12), 12);
        assert_eq!(run(false, 12), 0);
        assert_eq!(run(true, -3), 29);
    }

    /// `oscillate.starts_at_rest`: a slot's last speed isn't taken, only
    /// its fraction across.
    #[test]
    fn a_swing_that_starts_at_rest_takes_no_speed_from_its_slot() {
        let run = |at_rest: bool| {
            let mut s = view_spec(&[(100, false), (300, false)]);
            s.levels[0].spawn_slots = Some(1);
            s.levels[0].slot_rest = true;
            s.kinds[1].oscillate = Some(crate::spec::Oscillate {
                vertical: true,
                center: n(48),
                force: q(0.0625),
                max_speed: n(2),
                low: None,
                starts_at_rest: at_rest,
                move_first: false,
                rest_creep_every: None,
            });
            let mut w = World::power_on(&s);
            let width = s.cameras[0].size[0];
            at_view(&s, &mut w, 100 - width);
            w.bodies[1].x = w.bodies[1].x.add(q(0.5));
            w.bodies[1].vy = n(3);
            w.bodies[1].alive = false;
            at_view(&s, &mut w, 300 - width);
            assert_eq!(Fx(w.bodies[2].x.frac()), q(0.5));
            w.bodies[2].vy
        };
        assert_eq!(run(false), n(3));
        assert_eq!(run(true), Fx::ZERO);
    }

    /// `oscillate.move_first`: the swing moves by the speed it had, then
    /// takes the force, chosen by where it was before the move.
    #[test]
    fn a_swing_that_moves_first_moves_by_the_old_speed() {
        let step = |move_first: bool| {
            let mut s = quiet(spec());
            s.kinds[1].oscillate = Some(crate::spec::Oscillate {
                vertical: true,
                center: n(4),
                force: q(0.25),
                max_speed: n(2),
                low: None,
                starts_at_rest: false,
                move_first,
                rest_creep_every: None,
            });
            let mut w = World::power_on(&s);
            let b = &mut w.bodies[1];
            b.home[1] = b.y;
            let y = b.y;
            b.vy = n(1);
            oscillate(&s, &mut w, 1);
            (w.bodies[1].y, w.bodies[1].vy, y)
        };
        // Above the center the force is down: 1 becomes 1.25 either way,
        // and only the move's order differs.
        let (y, vy, from) = step(false);
        assert_eq!((y, vy), (from.add(q(1.25)), q(1.25)));
        let (y, vy, from) = step(true);
        assert_eq!((y, vy), (from.add(n(1)), q(1.25)));
    }

    /// `oscillate.rest_creep_every`: at no speed the fraction goes; short
    /// of where it's placed the body creeps on the beat and takes no
    /// force, and past it the swing goes on.
    #[test]
    fn a_swing_at_rest_loses_its_fraction_and_creeps_home() {
        let step = |every: Option<u32>, from_home: i64, frame: u64| {
            let mut s = quiet(spec());
            s.kinds[1].oscillate = Some(crate::spec::Oscillate {
                vertical: true,
                center: n(-16),
                force: q(0.25),
                max_speed: n(2),
                low: None,
                starts_at_rest: false,
                move_first: true,
                rest_creep_every: every,
            });
            let mut w = World::power_on(&s);
            w.frame = frame;
            let b = &mut w.bodies[1];
            b.home[1] = b.y;
            let home = b.y;
            b.y = b.y.add(n(from_home)).add(q(0.5));
            oscillate(&s, &mut w, 1);
            (w.bodies[1].y.sub(home), w.bodies[1].vy)
        };
        assert_eq!(step(None, 0, 8), (q(0.5), q(-0.25)));
        assert_eq!(step(Some(8), 0, 8), (Fx::ZERO, q(-0.25)));
        assert_eq!(step(Some(8), -3, 8), (n(-2), Fx::ZERO));
        assert_eq!(step(Some(8), -3, 9), (n(-3), Fx::ZERO));
    }

    #[test]
    fn stands_still_looks_for_the_ground_at_its_feet_and_falls_a_frame_later() {
        let mut s = quiet(spec());
        s.kinds[1].tile_collision = true;
        s.kinds[1].fall = Some(crate::spec::Fall {
            gravity: q(0.125),
            max_fall: n(3),
            max_fall_from_half: false,
            move_first: true,
            feet: Some(vec![n(8)]),
            stands_still: true,
        });
        // The floor's top is 48, and it ends at x 32.
        let edge = 32;
        for c in 2..8 {
            s.levels[0].tiles[3 * 8 + c] = Tile::Empty;
        }
        let mut w = World::power_on(&s);
        let b = &mut w.bodies[1];
        (b.x, b.y, b.vy, b.on_ground) = (n(edge - 9), n(32), Fx::ZERO, true);
        // Its middle is over the last column: it stands, and takes no
        // gravity.
        enemy(&s, &mut w, &mut Frame::default(), 1, &Exts::new());
        assert_eq!((w.bodies[1].y, w.bodies[1].vy), (n(32), Fx::ZERO));
        assert!(w.bodies[1].on_ground);
        // Past the edge by its middle, with the rest of it still over the
        // floor, the ground has gone, but it doesn't move yet.
        w.bodies[1].x = n(edge - 8);
        enemy(&s, &mut w, &mut Frame::default(), 1, &Exts::new());
        assert_eq!((w.bodies[1].y, w.bodies[1].vy), (n(32), Fx::ZERO));
        assert!(!w.bodies[1].on_ground);
        // The next frame it falls.
        enemy(&s, &mut w, &mut Frame::default(), 1, &Exts::new());
        assert_eq!((w.bodies[1].y, w.bodies[1].vy), (n(32), q(0.125)));
        enemy(&s, &mut w, &mut Frame::default(), 1, &Exts::new());
        assert_eq!((w.bodies[1].y, w.bodies[1].vy), (q(32.125), q(0.25)));
    }

    #[test]
    fn a_body_that_stands_still_takes_a_slots_height_fraction_and_rests_on_the_ground() {
        let mut s = view_spec(&[(100, false), (300, false)]);
        s.levels[0].spawn_slots = Some(1);
        s.levels[0].slot_rest = true;
        s.kinds[1].fall = Some(crate::spec::Fall {
            gravity: q(0.125),
            max_fall: n(3),
            max_fall_from_half: false,
            move_first: true,
            feet: None,
            stands_still: false,
        });
        let width = s.cameras[0].size[0];
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 100 - width);
        w.bodies[1].y = w.bodies[1].y.add(q(0.25));
        w.bodies[1].vy = n(3);
        w.bodies[1].alive = false;
        at_view(&s, &mut w, 300 - width);
        assert_eq!(w.slots[0].body, 3);
        assert_eq!(Fx(w.bodies[2].y.frac()), q(0.25));
        assert_eq!(w.bodies[2].vy, n(3));
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back.slots, w.slots);
        // One that `stands_still` takes it too, and on the ground it starts
        // at rest.
        s.kinds[1].tile_collision = true;
        s.kinds[1].fall.as_mut().unwrap().stands_still = true;
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 100 - width);
        w.bodies[1].y = w.bodies[1].y.add(q(0.25));
        w.bodies[1].vy = n(3);
        w.bodies[1].alive = false;
        at_view(&s, &mut w, 300 - width);
        assert!(w.bodies[2].on_ground);
        assert_eq!(w.bodies[2].vy, Fx::ZERO);
        assert_eq!(Fx(w.bodies[2].y.frac()), q(0.25));
    }

    #[test]
    fn a_spawn_over_rest_keeps_the_fractions_the_last_left() {
        let s = view_spec(&[(100, false)]);
        let w = World::power_on(&s);
        let mut b = w.bodies[0];
        (b.x, b.y) = (n(40), n(16));
        take_rest(&mut b, q(0.25), q(0.75));
        assert_eq!((b.x, b.y), (q(40.25), q(16.75)));
    }

    /// Platformer `run.step_cycle`, on a made-up cycle: each walking frame
    /// advances the counter in word 0 first, then moves its entry; a stop
    /// keeps the phase, and the cycle is the same both ways.
    #[test]
    fn a_step_cycle_walks_whole_pixels_and_keeps_its_phase_through_a_stop() {
        let mut s = quiet(spec());
        s.kinds[0].run.as_mut().unwrap().step_cycle = vec![0, 1, 2];
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].ext[STEP_WORD], 0);
        let x0 = w.bodies[0].x;
        let mut moves = Vec::new();
        let mut walk = |w: &mut World, buttons: u8| {
            let x = w.bodies[0].x;
            step(&s, w, buttons);
            moves.push(w.bodies[0].x.sub(x).floor());
        };
        walk(&mut w, 1 << 7);
        walk(&mut w, 1 << 7);
        walk(&mut w, 0);
        walk(&mut w, 0);
        walk(&mut w, 1 << 6);
        walk(&mut w, 1 << 6);
        walk(&mut w, 1 << 6);
        assert_eq!(moves, vec![1, 2, 0, 0, 0, -1, -2]);
        assert_eq!(w.bodies[0].x, x0);
        assert_eq!(w.bodies[0].ext[STEP_WORD], 2);
        assert!(w.bodies[0].x.frac() == 0);
    }

    /// Platformer `jump.takeoff_fraction`, `jump.land_hold` and
    /// `run.air_step_cycle`, on made-up numbers: the takeoff sets the
    /// height's fraction, the body moves across by the air cycle the way it
    /// took off with nothing held after, the step counter advances on every
    /// frame in the air and of the hold, and after landing it neither walks
    /// nor jumps for the hold's frames.
    #[test]
    fn a_jump_moves_by_the_air_cycle_and_holds_after_landing() {
        let mut s = quiet(spec());
        {
            let r = s.kinds[0].run.as_mut().unwrap();
            r.step_cycle = vec![1, 1, 0];
            r.air_step_cycle = vec![1, 0];
            r.air_control = false;
            r.takeoff_in_air = true;
        }
        {
            let j = s.kinds[0].jump.as_mut().unwrap();
            j.takeoff_fraction = Some(Fx(0x8000));
            j.land_hold = 3;
            j.speed = n(-2);
            j.gravity = Fx(0x4000);
        }
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        let (x0, y0) = (w.bodies[0].x, w.bodies[0].y);
        // The takeoff frame: right and A. The arc's first move and the
        // air cycle's first step (word 1 goes to 1: entry 1, no move)
        // apply.
        step(&s, &mut w, 1 << 7 | JUMP);
        assert!(!w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].y, y0.add(Fx(0x8000)).add(n(-2)).add(Fx(0x4000)));
        assert_eq!(w.bodies[0].x, x0);
        assert_eq!(w.bodies[0].ext[STEP_WORD], 1);
        // Nothing held in the air: it goes on a pixel every other frame.
        let mut frames = 1;
        while !w.bodies[0].on_ground {
            step(&s, &mut w, 0);
            frames += 1;
            assert!(frames < 100);
        }
        assert_eq!(w.bodies[0].x, x0.add(n(frames / 2)));
        assert_eq!(w.bodies[0].ext[STEP_WORD], frames % 3);
        assert_eq!(w.bodies[0].ext[LAND_HOLD_WORD], 3);
        // The hold: right and A do nothing, and the counter still steps.
        let landed = (w.bodies[0].x, w.bodies[0].y);
        for k in 0..3 {
            step(&s, &mut w, 1 << 7 | JUMP);
            assert!(w.bodies[0].on_ground, "hold frame {k}");
            assert_eq!((w.bodies[0].x, w.bodies[0].y), landed);
        }
        assert_eq!(w.bodies[0].ext[STEP_WORD], (frames + 3) % 3);
        assert_eq!(w.bodies[0].ext[LAND_HOLD_WORD], 0);
        // Then it walks again.
        step(&s, &mut w, 1 << 7);
        assert_eq!(w.bodies[0].ext[STEP_WORD], (frames + 4) % 3);
    }

    /// Core `level.tile_surfaces`: the floor row as top-only tiles whose
    /// surfaces sit `left` rows down in columns 0-3 and `right` rows down in
    /// columns 4-7, the hero standing on the left half.
    fn surfaced(left: u8, right: u8) -> (Spec, World) {
        let mut s = quiet(spec());
        let width = 8;
        s.levels[0].surfaces = vec![crate::spec::NO_SURFACE; 8 * 4];
        for c in 0..width {
            s.levels[0].tiles[3 * width + c] = Tile::Top;
            s.levels[0].surfaces[3 * width + c] = if c < 4 { left } else { right };
        }
        s.levels[0].spawns[0].y = n(48 + i64::from(left) - 16);
        let w = World::power_on(&s);
        (s, w)
    }

    #[test]
    fn a_walker_steps_down_a_sloped_surface() {
        let (s, mut w) = surfaced(0, 1);
        assert!(w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].y, n(32));
        // Walking right, it stays on the higher surface while its hitbox
        // covers it, then steps down a pixel, never falling.
        for _ in 0..70 {
            step(&s, &mut w, 1 << 7);
            assert!(w.bodies[0].on_ground, "it fell at x {:?}", w.bodies[0].x);
        }
        assert!(w.bodies[0].x >= n(64));
        assert_eq!(w.bodies[0].y, n(33));
    }

    #[test]
    fn a_walker_steps_up_a_sloped_surface() {
        let (s, mut w) = surfaced(2, 1);
        assert_eq!(w.bodies[0].y, n(34));
        // While its hitbox still covers a column of 0-3 it stays at 34;
        // once it has wholly left them (x 64), it steps up a pixel.
        while w.bodies[0].x.floor() < 64 {
            assert_eq!(w.bodies[0].y, n(34), "x {}", w.bodies[0].x.floor());
            step(&s, &mut w, 1 << 7);
        }
        assert_eq!(w.bodies[0].y, n(33));
        assert!(w.bodies[0].on_ground);
    }

    /// A platform for the ground bodies' tests: from frame `ext[2]` a pixel
    /// up on every fourth frame, gone once its top reaches `ext[1]`.
    struct Lift;

    impl crate::ext::Ext for Lift {
        fn step(&self, cx: &mut crate::ext::ExtCx<'_>) {
            if cx.frame < cx.me.ext[2] as u64 || !cx.frame.is_multiple_of(4) {
                return;
            }
            cx.me.y = cx.me.y.sub(n(1));
            if cx.me.y.floor() <= cx.me.ext[1] {
                cx.remove(cx.id);
            }
        }
    }

    /// Platformer `level.ground_bodies`, on made-up numbers: a level box
    /// from x 0 to 40 on the row under the hero's feet and the bug, a
    /// platform from x 41 to 56 on the same row, which [`Lift`] raises.
    fn lift_level() -> (Spec, World, crate::ext::Exts, i64) {
        let (mut s, w) = surfaced(2, 2);
        let y = w.bodies[0].y.floor();
        let kind = &s.kinds[0];
        let below = y + kind.tile_hitbox.y.floor() + kind.tile_hitbox.h.floor();
        s.levels[0].ground = vec![[0, below, 40, below]];
        s.levels[0].ground_feet = [2, 4];
        s.levels[0].ground_boxes = true;
        s.levels[0].ground_bodies = vec![(1, [0, 0, 15, 0])];
        s.kinds[1].ext = Some("lift".into());
        s.kinds[1].fall = None;
        s.kinds[1].tile_collision = false;
        s.levels[0].spawns[1].x = n(41);
        s.levels[0].spawns[1].y = n(below);
        let mut exts = crate::ext::Exts::new();
        exts.insert("lift".into(), Box::new(Lift));
        let mut w = World::power_on(&s);
        w.bodies[1].ext[1] = below - 20;
        w.bodies[1].ext[2] = 100;
        (s, w, exts, below)
    }

    #[test]
    fn a_body_rides_a_ground_body_and_is_carried_by_its_steps() {
        let (s, mut w, exts, below) = lift_level();
        // Right until the feet are wholly past the level's box, on the lift.
        while w.bodies[0].x.floor() + 2 <= 40 {
            step_with_ext(&s, &mut w, 1 << 7, &exts);
            assert!(w.bodies[0].on_ground);
        }
        assert_eq!(w.bodies[0].riding, 2, "the walk's test met the lift");
        while w.frame < 100 {
            step_with_ext(&s, &mut w, 0, &exts);
        }
        // Standing, it goes up with the lift, a pixel at a time.
        for _ in 0..12 {
            step_with_ext(&s, &mut w, 0, &exts);
            let (h, l) = (&w.bodies[0], &w.bodies[1]);
            let feet =
                h.y.floor() + s.kinds[0].tile_hitbox.y.floor() + s.kinds[0].tile_hitbox.h.floor();
            assert_eq!(feet, l.y.floor(), "carried with the lift");
            assert!(h.on_ground);
        }
        assert!(w.bodies[1].y.floor() < below);
        // Once the lift has gone it still stands where it was carried, with
        // no test, until it moves across: then it falls.
        for _ in 0..80 {
            step_with_ext(&s, &mut w, 0, &exts);
        }
        assert!(!w.bodies[1].alive);
        let held = w.bodies[0].y;
        step_with_ext(&s, &mut w, 0, &exts);
        assert_eq!(w.bodies[0].y, held);
        assert!(w.bodies[0].on_ground);
        for _ in 0..4 {
            step_with_ext(&s, &mut w, 1 << 7, &exts);
        }
        assert_eq!(w.bodies[0].riding, 0, "the walk's test met no box");
        assert!(!w.bodies[0].on_ground);
    }

    /// The ride is in the canonical state: two runs of the same presses end
    /// with the same hash, and a state saved mid-ride plays on the same.
    #[test]
    fn a_ride_replays_to_the_same_state_hash() {
        // Right until the feet are on the lift, then nothing; after frame
        // 150 right again, off its far end.
        let presses = |w: &World| {
            if w.bodies[0].x.floor() + 2 <= 40 || w.frame >= 150 {
                1 << 7
            } else {
                0
            }
        };
        let run = |frames: usize| {
            let (s, mut w, exts, _) = lift_level();
            let mut hashes = Vec::new();
            for _ in 0..frames {
                let p = presses(&w);
                step_with_ext(&s, &mut w, p, &exts);
                hashes.push(w.hash());
            }
            (s, w, exts, hashes)
        };
        let (s, w, exts, a) = run(110);
        let (_, _, _, b) = run(110);
        assert_eq!(a, b);
        assert_eq!(w.bodies[0].riding, 2);
        let mut back = World::from_canonical(&s, &w.canonical()).unwrap();
        let mut on = w.clone();
        for _ in 110..200 {
            let p = presses(&on);
            step_with_ext(&s, &mut back, p, &exts);
            step_with_ext(&s, &mut on, p, &exts);
            assert_eq!(back.hash(), on.hash());
        }
        assert_eq!(on.bodies[0].riding, 0);
    }

    /// Platformer `level.ground`: boxes in place of the tile surfaces. The
    /// feet (columns 2 to 4 from x) keep the height while they meet a box
    /// on the row under the tile hitbox, and step to the next row's box
    /// once they meet none, whatever the tiles say.
    #[test]
    fn a_walker_follows_the_ground_boxes_by_its_feet() {
        let (mut s, mut w) = surfaced(2, 2);
        let y = w.bodies[0].y.floor();
        let kind = &s.kinds[0];
        let below = y + kind.tile_hitbox.y.floor() + kind.tile_hitbox.h.floor();
        s.levels[0].ground = vec![[0, below, 40, below], [41, below - 1, 127, below - 1]];
        s.levels[0].ground_feet = [2, 4];
        let mut stepped = false;
        for _ in 0..90 {
            step(&s, &mut w, 1 << 7);
            let b = w.bodies[0];
            assert!(b.on_ground, "it fell at x {:?}", b.x);
            // Up a pixel exactly once the feet's left column passes 40.
            let want = if b.x.floor() + 2 > 40 { y - 1 } else { y };
            assert_eq!(b.y.floor(), want, "x {}", b.x.floor());
            stepped |= b.y.floor() == y - 1;
            if stepped {
                assert!(b.x.floor() >= 39);
            }
        }
        assert!(stepped);
    }

    #[test]
    fn a_fall_lands_on_the_surface_row() {
        let (s, mut w) = surfaced(3, 3);
        w.bodies[0].y = n(10);
        w.bodies[0].on_ground = false;
        for _ in 0..40 {
            step(&s, &mut w, 0);
            if w.bodies[0].on_ground {
                break;
            }
        }
        assert!(w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].y, n(48 + 3 - 16));
    }

    #[test]
    fn keep_in_view_turn_sends_a_jump_back_from_the_edge() {
        let mut s = quiet(spec());
        s.kinds[0].run.as_mut().unwrap().air_control = false;
        s.cameras[0].keep_in_view = Some([0, 20]);
        s.cameras[0].by_move = None;
        s.cameras[0].lead = [n(0), n(0)];
        s.cameras[0].axis_x = false;
        let mut w = World::power_on(&s);
        w.cameras[0][0] = n(0);
        let air = |w: &mut World| {
            w.bodies[0].x = n(19);
            w.bodies[0].y = n(0);
            w.bodies[0].vy = n(0);
            w.bodies[0].on_ground = false;
            w.bodies[0].vx = n(1);
        };
        // Without the option the edge holds it and its speed goes.
        air(&mut w);
        step(&s, &mut w, 0);
        assert_eq!((w.bodies[0].x, w.bodies[0].vx), (n(20), n(1)));
        step(&s, &mut w, 0);
        assert_eq!((w.bodies[0].x, w.bodies[0].vx), (n(20), Fx::ZERO));
        // With it, reaching the edge in the air turns it around.
        s.cameras[0].keep_in_view_turn = true;
        let mut w = World::power_on(&s);
        w.cameras[0][0] = n(0);
        air(&mut w);
        step(&s, &mut w, 0);
        assert_eq!((w.bodies[0].x, w.bodies[0].vx), (n(20), n(-1)));
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].x, n(19));
        // At the edge already and moving toward it, it turns and moves back
        // on the same frame, as Donkey Kong's jump tests the edge before it
        // moves ($D547 calls $D990).
        let mut w = World::power_on(&s);
        w.cameras[0][0] = n(0);
        air(&mut w);
        w.bodies[0].x = n(20);
        step(&s, &mut w, 0);
        assert_eq!((w.bodies[0].x, w.bodies[0].vx), (n(19), n(-1)));
    }

    #[test]
    fn keep_in_view_rows_bound_only_their_rows() {
        let mut s = quiet(spec());
        s.kinds[0].run.as_mut().unwrap().air_control = false;
        s.cameras[0].keep_in_view = Some([0, 40]);
        s.cameras[0].keep_in_view_rows = vec![([0, 9], [0, 20])];
        s.cameras[0].by_move = None;
        s.cameras[0].lead = [n(0), n(0)];
        s.cameras[0].axis_x = false;
        let at = |y: i64| {
            let mut w = World::power_on(&s);
            w.cameras[0][0] = n(0);
            w.bodies[0].x = n(20);
            w.bodies[0].y = n(y);
            w.bodies[0].vy = n(0);
            w.bodies[0].on_ground = false;
            w.bodies[0].vx = n(1);
            w
        };
        // In the row, its bound holds the body, and holds it as the
        // player's system ends, before anything else tests its place.
        let mut w = at(0);
        let mut f = begin_frame_pads(&s, &mut w, [0, 0]);
        player_ext(&s, &mut w, &mut f, &Exts::default());
        assert_eq!(w.bodies[0].x, n(20));
        // Out of it, the camera's own bounds.
        let mut w = at(30);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].x, n(21));
        assert_eq!(keep_bounds(&s, &w.bodies[0], 0, [0, 40]), [0, 40]);
        w.bodies[0].y = n(5);
        assert_eq!(keep_bounds(&s, &w.bodies[0], 0, [0, 40]), [0, 20]);
    }

    #[test]
    fn with_surfaces_feet_land_and_the_hitbox_stands() {
        // A surfaced girder that ends at x 64, and feet at the hitbox's
        // middle column (x + 8).
        let (mut s, _) = surfaced(3, 3);
        for c in 4..8 {
            s.levels[0].tiles[3 * 8 + c] = Tile::Empty;
        }
        s.kinds[0].jump.as_mut().unwrap().feet = Some(vec![n(8)]);
        // Standing at x 56 the feet are past the end, but the hitbox still
        // covers the girder: it stays standing.
        let mut w = World::power_on(&s);
        w.bodies[0].x = n(56);
        step(&s, &mut w, 0);
        assert!(w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].y, n(48 + 3 - 16));
        // Falling there from above, it lands only by its feet: it falls past.
        let mut w = World::power_on(&s);
        w.bodies[0].x = n(56);
        w.bodies[0].y = n(10);
        w.bodies[0].on_ground = false;
        for _ in 0..40 {
            step(&s, &mut w, 0);
        }
        assert!(w.bodies[0].y > n(48));
    }

    #[test]
    fn top_tiles_stop_a_fall_but_not_a_rise() {
        let mut s = quiet(spec());
        s.kinds[0].jump.as_mut().unwrap().speed = n(-6);
        // A top-only row at row 1 (y 16 to 31) over the hero's head.
        for c in 0..8 {
            s.levels[0].tiles[8 + c] = Tile::Top;
        }
        let mut w = World::power_on(&s);
        step(&s, &mut w, 1);
        let mut top = w.bodies[0].y;
        for _ in 0..40 {
            step(&s, &mut w, 1);
            top = top.min(w.bodies[0].y);
            if w.bodies[0].on_ground {
                break;
            }
        }
        assert!(top < n(16), "it rose through the top-only row");
        assert_eq!(w.bodies[0].y, n(0), "and landed on it on the way down");
        assert!(w.bodies[0].on_ground);
    }

    #[test]
    fn climbing_a_ladder_and_standing_on_its_top() {
        let mut s = quiet(spec());
        s.actions.push(Action {
            name: "up".into(),
            mask: 1 << 4,
        });
        s.actions.push(Action {
            name: "down".into(),
            mask: 1 << 5,
        });
        s.kinds[0].climb = Some(crate::spec::Climb {
            speed: n(1),
            up: 3,
            down: 4,
            release: Some(0),
            stand_on_top: true,
            grab_down: Fx::ZERO,
            hold_still: None,
            up_cycle: vec![],
            down_cycle: vec![],
            by_feet: false,
            down_holds: false,
            down_through: 0,
            grab: None,
            grab_snap: None,
            down_through_step: false,
            top_test: false,
            lone_way: false,
        });
        // A ladder in column 0, rows 1 and 2, standing on the floor.
        s.levels[0].tiles[8] = Tile::Ladder;
        s.levels[0].tiles[16] = Tile::Ladder;
        let mut w = World::power_on(&s);
        step(&s, &mut w, 1 << 4);
        assert!(w.bodies[0].climbing);
        assert_eq!(w.bodies[0].x, Fx::ZERO, "snapped to the ladder's middle");
        let mut frames = 0;
        while w.bodies[0].climbing {
            step(&s, &mut w, 1 << 4);
            frames += 1;
            assert!(frames < 64);
            assert_eq!(w.bodies[0].vy, Fx::ZERO, "no gravity on a ladder");
        }
        assert_eq!(w.bodies[0].y, n(0), "standing on the ladder's top");
        assert!(w.bodies[0].on_ground);
        // It stays there: the top stands while it isn't climbing.
        for _ in 0..5 {
            step(&s, &mut w, 0);
        }
        assert_eq!(w.bodies[0].y, n(0));
        // Down from the top climbs back in.
        step(&s, &mut w, 1 << 5);
        assert!(w.bodies[0].climbing);
    }

    /// A hero with up (1 << 4) and down (1 << 5) and a climb, for the
    /// climb feature tests on made-up numbers.
    fn climber(c: impl FnOnce(&mut crate::spec::Climb)) -> Spec {
        let mut s = quiet(spec());
        s.actions.push(Action {
            name: "up".into(),
            mask: 1 << 4,
        });
        s.actions.push(Action {
            name: "down".into(),
            mask: 1 << 5,
        });
        let mut cl = crate::spec::Climb {
            speed: n(1),
            up: 3,
            down: 4,
            release: None,
            stand_on_top: false,
            grab_down: Fx::ZERO,
            hold_still: None,
            up_cycle: vec![],
            down_cycle: vec![],
            by_feet: false,
            down_holds: false,
            down_through: 0,
            grab: None,
            grab_snap: None,
            down_through_step: false,
            top_test: false,
            lone_way: false,
        };
        c(&mut cl);
        s.kinds[0].climb = Some(cl);
        s
    }

    /// Features `climb.up_cycle` and `down_cycle`: up held at a ladder
    /// steps mechanic word 0, the climb starts on the first count that
    /// moves and moves that frame, and a frame with neither held steps
    /// nothing.
    #[test]
    fn a_climb_cycle_starts_on_a_moving_count() {
        let s = climber(|c| {
            c.up_cycle = vec![0, 0, 1];
            c.down_cycle = vec![1, 1, 0];
        });
        let mut s = s;
        s.levels[0].tiles[8] = Tile::Ladder;
        s.levels[0].tiles[16] = Tile::Ladder;
        let mut w = World::power_on(&s);
        let up = 1 << 4;
        step(&s, &mut w, up);
        assert!(!w.bodies[0].climbing, "count 1 moves nothing, so no climb");
        assert_eq!((w.bodies[0].ext[0], w.bodies[0].y), (1, n(32)));
        step(&s, &mut w, up);
        assert!(w.bodies[0].climbing);
        assert_eq!((w.bodies[0].ext[0], w.bodies[0].y), (2, n(31)));
        for _ in 0..3 {
            step(&s, &mut w, up);
        }
        assert_eq!((w.bodies[0].ext[0], w.bodies[0].y), (2, n(30)));
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].ext[0], 2, "nothing held steps nothing");
        step(&s, &mut w, 1 << 7);
        assert_eq!(
            w.bodies[0].ext[0], 2,
            "right held on a ladder steps nothing"
        );
        step(&s, &mut w, 1 << 5);
        assert_eq!((w.bodies[0].ext[0], w.bodies[0].y), (0, n(31)));
    }

    /// Feature `climb.by_feet`: a ladder whose cell above is empty stops
    /// the climber with the gap under its feet, still climbing; one with a
    /// top tile above takes it up onto the top.
    #[test]
    fn by_feet_stops_at_a_broken_ladder_and_stands_on_a_top() {
        let up = 1 << 4;
        let mut s = climber(|c| c.by_feet = true);
        s.levels[0].tiles[16] = Tile::Ladder;
        let mut w = World::power_on(&s);
        for _ in 0..40 {
            step(&s, &mut w, up);
        }
        assert!(w.bodies[0].climbing);
        assert_eq!(w.bodies[0].y, n(15), "the gap in row 1 is under its feet");
        s.levels[0].tiles[8] = Tile::Top;
        let mut w = World::power_on(&s);
        for _ in 0..40 {
            step(&s, &mut w, up);
        }
        assert!(!w.bodies[0].climbing);
        assert!(w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].y, n(0), "standing on the top tile");
        // Down from the top, with the ladder under the top tile, climbs in.
        // Without a cycle the grab frame doesn't move.
        step(&s, &mut w, 1 << 5);
        assert!(w.bodies[0].climbing);
        assert_eq!(w.bodies[0].y, n(0));
        step(&s, &mut w, 1 << 5);
        assert_eq!(w.bodies[0].y, n(1));
    }

    /// Features `climb.grab` and `level.ladders`: up takes the ladder only
    /// where the grab box meets a ladder box, edges included, whatever the
    /// cells say; one pixel short of the box's bottom it does nothing.
    #[test]
    fn a_grab_box_takes_a_ladder_only_where_it_meets_its_box() {
        let up = 1 << 4;
        let s0 = climber(|c| c.grab = Some([0, 12, 0, 12]));
        let w0 = World::power_on(&s0);
        let (x, y) = (w0.bodies[0].x.floor(), w0.bodies[0].y.floor());
        // A ladder box whose bottom is the feet row: taken.
        let mut s = s0.clone();
        s.levels[0].ladders = vec![[x, y - 20, x + 8, y + 12]];
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        step(&s, &mut w, up);
        assert!(w.bodies[0].climbing);
        // Its bottom a pixel above the feet: nothing.
        let mut s = s0.clone();
        s.levels[0].ladders = vec![[x, y - 20, x + 8, y + 11]];
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        for _ in 0..5 {
            step(&s, &mut w, up);
        }
        assert!(!w.bodies[0].climbing);
        // A ladder cell at the middle without a box: nothing either.
        s.levels[0].tiles[16] = Tile::Ladder;
        s.levels[0].tiles[8] = Tile::Ladder;
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        step(&s, &mut w, up);
        assert!(!w.bodies[0].climbing);
    }

    /// Features `climb.top_test`, `level.ladder_tops` and
    /// `level.ladder_stops`: from standing, down takes a ladder only where
    /// the grab box meets a top and no stop, up refuses a top or a stop,
    /// and a climb up doesn't move where the box meets a stop.
    #[test]
    fn the_top_test_asks_the_tops_and_the_stops() {
        let (up, down) = (1 << 4, 1 << 5);
        let s0 = climber(|c| {
            c.grab = Some([0, 12, 0, 12]);
            c.top_test = true;
        });
        let w0 = World::power_on(&s0);
        let (x, y) = (w0.bodies[0].x.floor(), w0.bodies[0].y.floor());
        let feet = y + 12;
        let ladder = [x, feet - 20, x + 8, feet + 20];
        let run = |s: &Spec, input: u8, frames: usize| {
            let mut w = World::power_on(s);
            step(s, &mut w, 0);
            for _ in 0..frames {
                step(s, &mut w, input);
            }
            w
        };
        // A ladder through the feet and no top there: down does nothing,
        // whatever the ladder box says.
        let mut s = s0.clone();
        s.levels[0].ladders = vec![ladder];
        s.levels[0].ladder_tops = vec![[x + 20, feet, x + 28, feet + 12]];
        assert!(!run(&s, down, 3).bodies[0].climbing);
        // Up there climbs, as `climb.grab` alone does.
        assert!(run(&s, up, 1).bodies[0].climbing);
        // A top at the feet: down climbs, and up does nothing.
        s.levels[0].ladder_tops = vec![[x, feet, x + 8, feet + 12]];
        assert!(run(&s, down, 1).bodies[0].climbing);
        assert!(!run(&s, up, 3).bodies[0].climbing);
        // A stop there too: neither starts.
        s.levels[0].ladder_stops = vec![[x, feet - 4, x + 8, feet + 4]];
        assert!(!run(&s, down, 3).bodies[0].climbing);
        assert!(!run(&s, up, 3).bodies[0].climbing);
        // A stop above the feet: a climb up goes until its box meets it.
        s.levels[0].ladder_tops = vec![[x + 20, feet, x + 28, feet + 12]];
        s.levels[0].ladder_stops = vec![[x - 16, feet - 30, x + 16, feet - 3]];
        s.levels[0].tiles[8] = Tile::Ladder;
        s.levels[0].tiles[16] = Tile::Ladder;
        let w = run(&s, up, 10);
        assert!(w.bodies[0].climbing);
        assert_eq!(
            w.bodies[0].y.floor(),
            y - 3,
            "stopped where the feet meet the stop"
        );
    }

    /// Feature `input.keep_direction`: a direction still held keeps acting
    /// alone, and a new one acts once it is let go; buttons pass as they
    /// are.
    #[test]
    fn keep_direction_latches_the_direction_still_held() {
        let (up, left, right, a) = (1 << 4, 1 << 6, 1 << 7, 1);
        assert_eq!(keep_direction(0, up | left), up | left);
        assert_eq!(keep_direction(left, up | left | a), left | a);
        assert_eq!(keep_direction(left, up), up);
        assert_eq!(keep_direction(up | left, right), right);
        // In a run: walking right, up added, the run keeps walking.
        let mut s = climber(|_| {});
        s.keep_direction = true;
        let mut w = World::power_on(&s);
        step(&s, &mut w, right);
        let x = w.bodies[0].x;
        step(&s, &mut w, up | right);
        assert!(!w.held(&s, 3), "up waits while right is still held");
        assert!(w.bodies[0].x > x, "it walks on");
        step(&s, &mut w, up);
        assert!(w.held(&s, 3));
    }

    /// Feature `climb.down_through`: down at a ladder's foot climbs down
    /// through the floor, the height wraps at the given size, and the
    /// climb ends on the next surface row the feet reach.
    #[test]
    fn down_through_climbs_through_the_floor_and_wraps() {
        let mut s = climber(|c| {
            c.by_feet = true;
            c.down_through = 64;
        });
        // A ladder cell in column 0, row 2, with a surface on its top row
        // (Donkey Kong's ladder foot in a girder), and a top tile in row 1
        // to land on after the wrap.
        s.levels[0].tiles[16] = Tile::Ladder;
        s.levels[0].tiles[8] = Tile::Top;
        s.levels[0].surfaces = vec![crate::spec::NO_SURFACE; 8 * 4];
        s.levels[0].surfaces[16] = 0;
        s.levels[0].spawns[0].y = n(16);
        // No floor under the ladder.
        s.levels[0].tiles[24] = Tile::Empty;
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        assert!(w.bodies[0].on_ground);
        assert_eq!(
            w.bodies[0].y,
            n(16),
            "standing on the ladder cell's surface"
        );
        let down = 1 << 5;
        step(&s, &mut w, down);
        assert!(w.bodies[0].climbing);
        let mut wrapped = false;
        for _ in 0..80 {
            let y = w.bodies[0].y;
            step(&s, &mut w, down);
            wrapped |= w.bodies[0].y < y;
            if !w.bodies[0].climbing {
                break;
            }
        }
        assert!(wrapped, "the height wrapped at 64");
        assert!(!w.bodies[0].climbing);
        assert!(w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].y, n(0), "standing on the top tile's top");
    }

    /// Feature `climb.down_through_step`: the same press at the ladder's
    /// foot does nothing unless the body's last move across stepped it up
    /// a surface.
    #[test]
    fn down_through_step_needs_a_step_up() {
        let mut s = climber(|c| {
            c.by_feet = true;
            c.down_through = 64;
            c.down_through_step = true;
        });
        s.levels[0].tiles[16] = Tile::Ladder;
        s.levels[0].tiles[8] = Tile::Top;
        s.levels[0].surfaces = vec![crate::spec::NO_SURFACE; 8 * 4];
        s.levels[0].surfaces[16] = 0;
        s.levels[0].spawns[0].y = n(16);
        s.levels[0].tiles[24] = Tile::Empty;
        let down = 1 << 5;
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        assert!(!w.bodies[0].stepped_up);
        step(&s, &mut w, down);
        assert!(!w.bodies[0].climbing, "no step up: no climb through");
        // The same place right after a step up.
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        w.bodies[0].stepped_up = true;
        step(&s, &mut w, down);
        assert!(w.bodies[0].climbing, "after a step up it climbs through");
    }

    /// Feature `climb.lone_way`: up or down counts only held alone.
    #[test]
    fn lone_way_climbs_only_on_a_lone_up_or_down() {
        let s = climber(|c| c.lone_way = true);
        let mut w = World::power_on(&s);
        let (up, down, right) = (1u8 << 4, 1u8 << 5, 1u8 << 7);
        let kind = &s.kinds[0];
        for (pad, want) in [
            (up, [true, false]),
            (down, [false, true]),
            (up | right, [false, false]),
            (down | right, [false, false]),
        ] {
            step(&s, &mut w, 0);
            step(&s, &mut w, pad);
            assert_eq!(climb_ways(&s, &w, kind), want, "pad {pad:#x}");
        }
        let s = climber(|_| {});
        let mut w = World::power_on(&s);
        step(&s, &mut w, up | right);
        assert_eq!(climb_ways(&s, &w, &s.kinds[0]), [true, false]);
    }

    /// Feature `run.both_left`: left and right together face left and
    /// move nothing.
    #[test]
    fn both_left_faces_left_and_stands() {
        let mut s = quiet(spec());
        s.kinds[0].run.as_mut().unwrap().both_left = true;
        let mut w = World::power_on(&s);
        for _ in 0..4 {
            step(&s, &mut w, 0);
        }
        let x = w.bodies[0].x;
        let (left, right) = (1u8 << 6, 1u8 << 7);
        for _ in 0..10 {
            step(&s, &mut w, left | right);
        }
        assert_eq!(w.bodies[0].x, x);
        assert!(w.bodies[0].facing_left);
    }

    /// Feature `climb.down_holds`: down held while standing keeps the run
    /// from walking; up held doesn't.
    #[test]
    fn down_held_on_the_ground_keeps_the_climber_still() {
        let s = climber(|c| c.down_holds = true);
        let mut w = World::power_on(&s);
        let x = w.bodies[0].x;
        for _ in 0..10 {
            step(&s, &mut w, (1 << 5) | (1 << 7));
        }
        assert_eq!(w.bodies[0].x, x);
        step(&s, &mut w, (1 << 4) | (1 << 7));
        assert_eq!(w.bodies[0].x, x.add(n(1)));
    }

    /// Feature `climb.down_holds` yields to a jump: Down held with the
    /// jump pressed takes off, and with `run.takeoff_in_air` the step
    /// counter steps on the takeoff frame as on any jump frame.
    #[test]
    fn down_held_doesnt_hold_a_takeoff() {
        let mut s = climber(|c| c.down_holds = true);
        let run = s.kinds[0].run.as_mut().unwrap();
        run.step_cycle = vec![0, 1, 2];
        run.takeoff_in_air = true;
        let mut w = World::power_on(&s);
        step(&s, &mut w, 1 << 5);
        let n0 = w.bodies[0].ext[STEP_WORD];
        step(&s, &mut w, 1 << 5);
        assert_eq!(w.bodies[0].ext[STEP_WORD], n0, "down alone holds");
        step(&s, &mut w, (1 << 5) | 1);
        assert!(!w.bodies[0].on_ground, "the jump takes off");
        assert_eq!(w.bodies[0].ext[STEP_WORD], (n0 + 1) % 3, "and steps");
    }

    /// Features `level.pickups` and `level.pickup_holes`: walking across a
    /// place takes it (its counter 1, once); walking back over it, the
    /// player stands that frame, falls from the next, passes the ground
    /// box under the hole and lands, dying, on the box below.
    #[test]
    fn a_pickup_is_taken_once_and_its_hole_drops_the_walker() {
        let (mut s, mut w) = surfaced(2, 2);
        let y = w.bodies[0].y.floor();
        let kind = &s.kinds[0];
        let below = y + kind.tile_hitbox.y.floor() + kind.tile_hitbox.h.floor();
        s.levels[0].ground = vec![[0, below, 127, below], [0, below + 16, 127, below + 16]];
        s.levels[0].ground_feet = [2, 4];
        s.levels[0].ground_land = Some(2);
        s.kinds[0].jump.as_mut().unwrap().ledge_fall = Some((2, 3, true));
        let taken = s.counters.len();
        s.counters.push(Counter {
            restarts_clock: false,
            high_of: None,
            name: "rivet_0".into(),
            start: 0,
            changes: Vec::new(),
            min: None,
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
        });
        s.levels[0].pickups = vec![crate::spec::PickupGroup {
            places: vec![[30, y]],
            counters: vec![taken],
            offset: [0, 0],
            window: [-16, 0],
            event: None,
            hole: Some([2, below - y, 4, below - y]),
            cells: Vec::new(),
            by_hook: false,
        }];
        let mut w2 = World::power_on(&s);
        std::mem::swap(&mut w, &mut w2);
        let (right, left) = (1 << 7, 1 << 6);
        while w.bodies[0].x.floor() < 30 {
            assert_eq!(w.counters[taken], 0);
            step(&s, &mut w, right);
        }
        assert_eq!(w.counters[taken], 1, "taken at x 30");
        assert!(w.bodies[0].on_ground, "a place in place is no hole");
        while w.bodies[0].x.floor() < 40 {
            step(&s, &mut w, right);
        }
        while w.bodies[0].x.floor() > 30 {
            step(&s, &mut w, left);
            assert!(w.bodies[0].on_ground);
        }
        assert_eq!(w.counters[taken], 1, "not taken twice");
        assert!(w.bodies[0].on_ground, "the touch's frame stands");
        assert_eq!(w.bodies[0].y.floor(), y);
        step(&s, &mut w, left);
        assert!(!w.bodies[0].on_ground, "the next frame falls");
        assert_eq!(w.bodies[0].y.floor(), y + 2);
        assert_eq!(w.bodies[0].x.floor(), 30, "straight down");
        let mut died = false;
        for _ in 0..20 {
            let ev = step(&s, &mut w, left);
            died |= ev.iter().any(|e| e.kind == EventKind::PLAYER_DIED);
            if w.bodies[0].on_ground {
                break;
            }
        }
        assert_eq!(
            w.bodies[0].y.floor(),
            y + 16,
            "past the hole, on the box below"
        );
        assert!(died);
        // Feature `level.pickup_by_hook`: the same walk takes nothing.
        s.levels[0].pickups[0].by_hook = true;
        let mut w = World::power_on(&s);
        while w.bodies[0].x.floor() < 40 {
            step(&s, &mut w, right);
        }
        assert_eq!(w.counters[taken], 0, "a hook takes those places");
    }

    /// Feature `jump.ledge_fall`: off a ledge the body drops `first` and
    /// then `speed` a frame straight down, and with `kills` its landing
    /// kills the player.
    #[test]
    fn a_ledge_fall_drops_straight_and_kills_on_landing() {
        let mut s = quiet(spec());
        // A ledge in row 1, columns 0 and 1, over the floor.
        s.levels[0].tiles[8] = Tile::Solid;
        s.levels[0].tiles[9] = Tile::Solid;
        s.levels[0].spawns[0].y = n(0);
        s.kinds[0].jump.as_mut().unwrap().ledge_fall = Some((2, 3, true));
        let mut w = World::power_on(&s);
        let right = 1 << 7;
        while w.bodies[0].on_ground {
            step(&s, &mut w, right);
        }
        let (x, y) = (w.bodies[0].x, w.bodies[0].y);
        assert_eq!(y, n(3), "the first drop");
        let mut died = false;
        for _ in 0..20 {
            let ev = step(&s, &mut w, right);
            died |= ev.iter().any(|e| e.kind == EventKind::PLAYER_DIED);
            if w.bodies[0].on_ground {
                break;
            }
            assert_eq!(w.bodies[0].x, x, "no move across");
        }
        assert!(w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].y, n(32));
        assert!(died, "the landing kills");
        assert!(y < n(32));
    }

    /// Feature `jump.fatal_drop`: a jump landing at least that far below
    /// its takeoff kills once its landing hold ends; a shorter one doesn't.
    #[test]
    fn a_fatal_drop_kills_when_the_landing_hold_ends() {
        let run = |drop: i64| {
            let mut s = quiet(spec());
            // A ledge in row 1, columns 0 and 1, over the floor 32 px lower.
            s.levels[0].tiles[8] = Tile::Solid;
            s.levels[0].tiles[9] = Tile::Solid;
            s.levels[0].spawns[0].y = n(0);
            let j = s.kinds[0].jump.as_mut().unwrap();
            j.fatal_drop = Some(drop);
            j.land_hold = 4;
            let mut w = World::power_on(&s);
            // Walk to the ledge's end, jump right off it and land on the
            // floor.
            while w.bodies[0].x < n(20) {
                step(&s, &mut w, 1 << 7);
            }
            assert!(w.bodies[0].on_ground);
            let mut landed = None;
            let mut died = None;
            for frame in 1..200 {
                let pad = if frame == 1 { 1 | (1 << 7) } else { 1 << 7 };
                let was = w.bodies[0].on_ground;
                let ev = step(&s, &mut w, pad);
                if landed.is_none() && frame > 1 && !was && w.bodies[0].on_ground {
                    landed = Some(frame);
                }
                if ev.iter().any(|e| e.kind == EventKind::PLAYER_DIED) {
                    died = Some(frame);
                    break;
                }
                if landed.is_some_and(|l| frame > l + 10) {
                    break;
                }
            }
            (landed, died)
        };
        let (landed, died) = run(16);
        let landed = landed.expect("it lands on the floor");
        assert_eq!(died, Some(landed + 4), "dies as the 4-frame hold ends");
        let (_, died) = run(40);
        assert_eq!(died, None, "a 32 px drop is short of 40");
    }

    /// Feature `jump.fatal_drop_in_air`: the takeoff word is the height
    /// after the takeoff frame's move, it is marked on the first frame the
    /// fall reaches the drop, and the landing kills as the hold ends.
    #[test]
    fn a_fatal_drop_in_the_air_marks_the_takeoff_word_and_kills() {
        let mut s = quiet(spec());
        s.levels[0].tiles[8] = Tile::Solid;
        s.levels[0].tiles[9] = Tile::Solid;
        s.levels[0].spawns[0].y = n(0);
        let j = s.kinds[0].jump.as_mut().unwrap();
        j.fatal_drop = Some(16);
        j.fatal_drop_in_air = true;
        j.land_hold = 4;
        let mut w = World::power_on(&s);
        while w.bodies[0].x < n(20) {
            step(&s, &mut w, 1 << 7);
        }
        let ground = w.bodies[0].y.floor();
        step(&s, &mut w, 1 | (1 << 7));
        let takeoff = w.bodies[0].ext[TAKEOFF_Y_WORD];
        assert_eq!(takeoff, w.bodies[0].y.floor(), "the height after the move");
        assert!(takeoff < ground, "the first move rises");
        let mut marked = None;
        let mut died = None;
        let mut landed = None;
        for frame in 2..200 {
            let was = w.bodies[0].on_ground;
            let ev = step(&s, &mut w, 1 << 7);
            let b = &w.bodies[0];
            if marked.is_none() && b.ext[TAKEOFF_Y_WORD] == FATAL_MARK {
                marked = Some(frame);
                assert!(b.y.floor() - takeoff >= 16 || b.on_ground);
            }
            if landed.is_none() && !was && b.on_ground {
                landed = Some(frame);
            }
            if ev.iter().any(|e| e.kind == EventKind::PLAYER_DIED) {
                died = Some(frame);
                break;
            }
        }
        let (marked, landed) = (marked.expect("marked"), landed.expect("lands"));
        assert!(marked < landed, "marked in the air");
        assert_eq!(died, Some(landed + 4));
    }

    /// Feature `power.locks`, measured on Donkey Kong's hammer (clip
    /// hammer/003 against hammer/004): while its timer runs, Up at a ladder
    /// doesn't climb; once it reaches 0 the same ladder climbs.
    #[test]
    fn a_lock_takes_up_away_from_climbing_while_its_counter_holds() {
        let mut s = quiet(spec());
        s.actions.push(Action {
            name: "up".into(),
            mask: 1 << 4,
        });
        s.actions.push(Action {
            name: "down".into(),
            mask: 1 << 5,
        });
        s.kinds[0].climb = Some(crate::spec::Climb {
            speed: n(1),
            up: 3,
            down: 4,
            release: Some(0),
            stand_on_top: true,
            grab_down: Fx::ZERO,
            hold_still: None,
            up_cycle: vec![],
            down_cycle: vec![],
            by_feet: false,
            down_holds: false,
            down_through: 0,
            grab: None,
            grab_snap: None,
            down_through_step: false,
            top_test: false,
            lone_way: false,
        });
        s.levels[0].tiles[8] = Tile::Ladder;
        s.levels[0].tiles[16] = Tile::Ladder;
        s.counters.push(Counter {
            restarts_clock: false,
            high_of: None,
            name: "hammer".into(),
            start: 1,
            changes: Vec::new(),
            min: Some(0),
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
        });
        let hammer = s.counters.len() - 1;
        s.kinds[0].locks = Some((
            crate::spec::Guard {
                counter: hammer,
                cmp: crate::spec::Cmp::Gt,
                value: 0,
            },
            vec![3],
        ));
        let mut w = World::power_on(&s);
        for _ in 0..10 {
            step(&s, &mut w, 1 << 4);
        }
        assert!(
            !w.bodies[0].climbing,
            "Up does nothing while the hammer is held"
        );
        w.counters[hammer] = 0;
        step(&s, &mut w, 1 << 4);
        assert!(
            w.bodies[0].climbing,
            "the same Up climbs once the counter is 0"
        );
    }

    #[test]
    fn a_hurt_takes_health_starts_invulnerability_and_knockback() {
        let mut s = spec();
        s.levels[0].kill_below = None;
        s.counters.push(Counter {
            restarts_clock: false,
            high_of: None,
            name: "hp".into(),
            start: 10,
            changes: Vec::new(),
            min: Some(0),
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
        });
        s.events = crate::spec::BUILTIN_EVENTS
            .iter()
            .map(|e| e.to_string())
            .chain(
                [
                    "deaths.reached_min",
                    "deaths.wrapped",
                    "hp.reached_min",
                    "hp.wrapped",
                ]
                .map(String::from),
            )
            .collect();
        s.own_events = s.events.len();
        s.kinds[1].contact.as_mut().unwrap().otherwise = Touch::HurtPlayer;
        s.kinds[1].contact.as_mut().unwrap().damage = 4;
        s.kinds[1].patrol = None;
        s.kinds[0].health = Some(crate::spec::Health {
            counter: Some(1),
            points: 0,
            invulnerable: 30,
            shield: false,
            damage_from: Vec::new(),
            defeat: None,
        });
        s.kinds[0].knockback = Some(crate::spec::Knockback {
            speed: n(1),
            rise: n(-2),
            frames: 8,
        });
        let mut w = World::power_on(&s);
        let mut hurts = 0;
        let mut toucher = 0;
        for _ in 0..40 {
            for e in step(&s, &mut w, 1 << 7) {
                hurts += usize::from(e.kind == EventKind::PLAYER_HURT);
                toucher +=
                    usize::from(e.kind == EventKind::ENEMY_HURT_PLAYER && e.entity == Some(1));
            }
        }
        assert_eq!(w.counters[1], 6, "one hurt of 4 in 30 invulnerable frames");
        assert_eq!((hurts, toucher), (1, 1));
    }

    #[test]
    fn shots_hit_enemies_until_they_are_defeated() {
        let mut s = quiet(spec());
        let mut shot = s.kinds[1].clone();
        shot.name = "shot".into();
        shot.fly = Some(crate::spec::Fly {
            speed: n(4),
            heading: crate::spec::Heading::Right,
            keep_speed: false,
        });
        shot.size = [n(4), n(4)];
        shot.hitbox = Rect {
            x: n(0),
            y: n(6),
            w: n(4),
            h: n(4),
        };
        shot.hits = Some(crate::spec::Hits {
            damage: 1,
            targets: vec![1],
            keep: false,
            every: 1,
            phase: 0,
            move_first: false,
        });
        s.kinds.push(shot);
        s.kinds[0].shoot = Some(crate::spec::Shoot {
            entity: 2,
            action: Some(0),
            every: 0,
            ahead: [n(16), n(0)],
            max: 1,
            unmirrored: false,
            vy: Fx::ZERO,
            vx: Fx::ZERO,
            vx_both: None,
            after_move: false,
            whole: false,
            in_slots: false,
        });
        s.kinds[0].jump = None;
        s.kinds[1].health = Some(crate::spec::Health {
            counter: None,
            points: 2,
            invulnerable: 0,
            shield: false,
            damage_from: Vec::new(),
            defeat: None,
        });
        let mut w = World::power_on(&s);
        let mut defeated = None;
        for i in 0..40 {
            let input = if i % 10 == 0 { 1 } else { 0 };
            for e in step(&s, &mut w, input) {
                if e.kind == EventKind::ENEMY_DEFEATED {
                    defeated = Some(i);
                }
            }
        }
        assert!(defeated.is_some(), "two shots defeat it");
        assert!(!w.bodies[1].alive);
        assert!(w.bodies.iter().filter(|b| b.kind == 2).count() >= 2);
    }

    /// Platformer 0.4.22's `contact.sides`, on made-up numbers: a touch at
    /// a side stops the player as a wall does, near the top a falling
    /// player passes, and a player moving away is left alone.
    #[test]
    fn an_entitys_sides_stop_the_player() {
        let mut s = quiet(spec());
        s.kinds[1].contact = Some(Contact {
            stomp: None,
            keep_on_stomp: false,
            otherwise: Touch::Nothing,
            damage: 1,
            stand: None,
            every: 1,
            phase: 0,
            invincible: Touch::Nothing,
            touch_edges: true,
            in_turn: false,
            after_move: false,
            sides: Some(crate::spec::Sides {
                left: 8,
                right: 9,
                top: 6,
            }),
            ride: None,
            near: None,
        });
        let right = 1 << 7;
        let start = |x: i64, y: i64, vy: Fx| {
            let mut w = World::power_on(&s);
            w.bodies[1].x = n(40);
            w.bodies[1].y = n(32);
            w.bodies[0].x = n(x);
            w.bodies[0].y = n(y);
            w.bodies[0].vy = vy;
            w
        };
        // Moving into the left side: back a pixel and stopped.
        let mut w = start(24, 32, Fx::ZERO);
        step(&s, &mut w, right);
        assert_eq!(w.bodies[0].x, n(24));
        assert_eq!(w.bodies[0].vx, Fx::ZERO);
        // Still at the side with no speed: a pixel further back.
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].x, n(23));
        // Falling with its bottom within `top` of the entity's top: no side.
        let mut w = start(24, 19, Fx::ONE);
        step(&s, &mut w, right);
        assert_eq!(w.bodies[0].x, n(25));
        // Right of the entity and moving right: no push.
        let mut w = start(54, 32, Fx::ZERO);
        step(&s, &mut w, right);
        assert_eq!(w.bodies[0].x, n(55));
    }

    /// `contact.near`, on made-up numbers: the player's points touch within
    /// `within` of the entity's position, an overlap of hitboxes alone
    /// doesn't, and nothing touches at or past `right` in the view.
    #[test]
    fn a_near_touch_tests_the_players_points() {
        let touched = |x: i64, right: Option<i64>| {
            let mut s = spec();
            s.kinds[0].touch_points = vec![[0, 0]];
            s.kinds[1].patrol = None;
            let c = s.kinds[1].contact.as_mut().unwrap();
            c.otherwise = Touch::Defeat;
            c.near = Some(crate::spec::Near { within: 8, right });
            let mut w = World::power_on(&s);
            w.bodies[1].x = n(40);
            w.bodies[1].y = n(32);
            w.bodies[0].x = n(x);
            w.bodies[0].y = n(32);
            step(&s, &mut w, 0);
            !w.bodies[1].alive
        };
        assert!(touched(33, None));
        assert!(!touched(32, None));
        assert!(!touched(33, Some(40)));
    }

    /// `contact.ride`, on made-up numbers: a falling player lands on the
    /// box below the entity, moves with the entity after its move, keeps
    /// the fraction of its height, and the wrapped box doesn't hold it
    /// while it's above `above`.
    #[test]
    fn a_rider_lands_on_the_lower_box_and_moves_with_it() {
        let mut s = quiet(spec());
        s.kinds[1].patrol = None;
        s.kinds[1].contact = Some(Contact {
            stomp: None,
            keep_on_stomp: false,
            otherwise: Touch::Nothing,
            damage: 1,
            stand: Some(n(6)),
            every: 1,
            phase: 0,
            invincible: Touch::Nothing,
            touch_edges: true,
            in_turn: false,
            after_move: false,
            sides: None,
            ride: Some(crate::spec::Ride {
                copy_below: 64,
                wrap: 128,
                above: 8,
            }),
            near: None,
        });
        let start = |lift: i64, y: Fx| {
            let mut w = World::power_on(&s);
            w.bodies[1].x = n(40);
            w.bodies[1].y = n(lift);
            w.bodies[0].x = n(40);
            w.bodies[0].y = y;
            w.bodies[0].vy = Fx::ONE;
            w
        };
        let hit = s.kinds[0].hitbox;
        let bottom = hit.y.floor() + hit.h.floor();
        let y = n(70 - bottom).add(q(0.25));
        let mut w = start(6, y);
        step(&s, &mut w, 0);
        assert!(w.bodies[0].on_ground, "it stands on the lower box");
        assert_eq!(w.bodies[0].vy, Fx::ZERO);
        // The entity moves up after the touch: the player goes with it.
        let mut f = Frame::default();
        f.rides.push((0, 1, 64));
        w.bodies[1].y = n(5);
        w.bodies[0].y = n(60).add(q(0.25));
        place_riders(&s, &mut w, &f, 1);
        let on = n(69 - bottom).add(q(0.25));
        assert_eq!(w.bodies[0].y, on, "up with it, fraction kept");
        // The top box is above `above`, so it holds nothing.
        let mut w = start(6, n(6 - bottom));
        step(&s, &mut w, 0);
        assert!(!w.bodies[0].on_ground);
    }

    /// `hit_widths`, on made-up numbers: past the level's narrower width
    /// the player doesn't stand on the entity.
    #[test]
    fn a_levels_hit_width_narrows_what_can_be_stood_on() {
        let mut s = quiet(spec());
        s.kinds[1].patrol = None;
        s.kinds[1].hitbox.w = n(48);
        s.kinds[1].contact = Some(Contact {
            stomp: None,
            keep_on_stomp: false,
            otherwise: Touch::Nothing,
            damage: 1,
            stand: Some(n(6)),
            every: 1,
            phase: 0,
            invincible: Touch::Nothing,
            touch_edges: true,
            in_turn: false,
            after_move: false,
            sides: None,
            ride: None,
            near: None,
        });
        let stands = |s: &Spec| {
            let mut w = World::power_on(s);
            let hit = s.kinds[0].hitbox;
            w.bodies[1].x = n(40);
            w.bodies[1].y = n(100);
            w.bodies[0].x = n(80).sub(hit.x);
            w.bodies[0].y = n(100 - hit.y.floor() - hit.h.floor());
            w.bodies[0].vy = Fx::ONE;
            step(s, &mut w, 0);
            w.bodies[0].riding == 2
        };
        assert!(stands(&s));
        s.levels[0].hit_widths = vec![(1, n(32))];
        assert!(!stands(&s));
    }

    #[test]
    fn a_defeat_leaves_a_body_that_keeps_the_slot() {
        let mut s = view_spec(&[(100, false), (300, false)]);
        s.levels[0].spawn_slots = Some(1);
        s.levels[0].slot_rest = true;
        s.kinds[1].health = Some(crate::spec::Health {
            counter: None,
            points: 1,
            invulnerable: 0,
            shield: false,
            damage_from: Vec::new(),
            defeat: Some(crate::spec::Defeat {
                becomes: 1,
                rise: n(2),
                speed: n(-3),
            }),
        });
        let mut w = World::power_on(&s);
        let width = s.cameras[0].size[0];
        at_view(&s, &mut w, 100 - width);
        assert_eq!(w.slots[0].body, 2);
        w.bodies[1].vy = q(0.25);
        let (x, y) = (w.bodies[1].x, w.bodies[1].y);
        let mut f = Frame {
            out: vec![false; w.bodies.len()],
            fresh_bodies: vec![false; w.bodies.len()],
            ..Frame::default()
        };
        w.bodies[1].alive = false;
        leave_defeated(&s, &mut w, &mut f, 1);
        let b = w.bodies[w.bodies.len() - 1];
        assert!(b.alive);
        assert_eq!((b.x, b.y), (x, y.add(n(-2))));
        assert_eq!(b.vy, n(-3).add(q(0.25)), "the speed's fraction is kept");
        assert_eq!(w.slots[0].body, w.bodies.len() as u32, "it has the slot");
    }

    #[test]
    fn standing_on_an_entity_lifting_and_throwing_it() {
        let mut s = quiet(spec());
        s.kinds[1].contact = Some(Contact {
            stomp: None,
            keep_on_stomp: false,
            otherwise: Touch::Nothing,
            damage: 1,
            stand: Some(n(8)),
            every: 1,
            phase: 0,
            invincible: Touch::Nothing,
            touch_edges: false,
            in_turn: false,
            after_move: false,
            sides: None,
            ride: None,
            near: None,
        });
        s.kinds[1].carryable = Some(None);
        s.actions.push(Action {
            name: "lift".into(),
            mask: 1 << 1,
        });
        s.kinds[0].carry = Some(crate::spec::Carry {
            action: 3,
            lift_frames: 4,
            offset: [n(0), n(-16)],
            throw: [n(3), n(-1)],
            throw_moving: None,
        });
        // The bug sits on the floor at x 0; the hero drops onto it.
        s.levels[0].spawns[1].x = n(0);
        s.levels[0].spawns[0].y = n(0);
        let mut w = World::power_on(&s);
        let mut stood = false;
        for _ in 0..20 {
            stood |= step(&s, &mut w, 0)
                .iter()
                .any(|e| e.kind == EventKind::ENEMY_STOOD_ON);
        }
        assert!(stood);
        assert_eq!(w.bodies[0].y, n(16), "on the bug's top");
        assert_eq!(w.bodies[0].riding, 2);
        let lifted = step(&s, &mut w, 1 << 1);
        assert!(
            lifted
                .iter()
                .any(|e| e.kind == EventKind::ITEM_LIFTED && e.entity == Some(1))
        );
        assert_eq!(w.bodies[0].lift, 4);
        for _ in 0..6 {
            step(&s, &mut w, 0);
        }
        assert_eq!(w.bodies[1].y, w.bodies[0].y.sub(n(16)), "carried overhead");
        let thrown = step(&s, &mut w, 1 << 1);
        assert!(thrown.iter().any(|e| e.kind == EventKind::ITEM_THROWN));
        assert_eq!((w.bodies[1].vx, w.bodies[1].carrier), (n(3), 0));
    }

    #[test]
    fn a_stepped_camera_holds_the_player() {
        let mut s = quiet(spec());
        s.levels[0].size = [32, 4];
        s.levels[0].tiles = vec![Tile::Empty; 32 * 4];
        for c in 0..32 {
            s.levels[0].tiles[3 * 32 + c] = Tile::Solid;
        }
        s.levels[0].camera = Some(crate::spec::LevelCamera {
            axis: None,
            lead: Some([n(0), n(0)]),
            step: Some(n(4)),
            hold_player: true,
            keep_in_view: None,
            keep_in_view_rows: Vec::new(),
        });
        s.levels[0].spawns[0].x = n(100);
        let w = World::power_on(&s);
        assert_eq!(w.cameras[0][0], n(100), "a level start places it at once");
        s.levels[0].spawns[0].x = n(0);
        let mut w2 = World::power_on(&s);
        w2.bodies[0].x = n(200);
        step(&s, &mut w2, 0);
        assert_eq!(w2.cameras[0][0], n(4));
        assert!(w2.hold);
        let x = w2.bodies[0].x;
        step(&s, &mut w2, 1 << 7);
        assert_eq!(w2.bodies[0].x, x, "held while the camera steps");
        let _ = w;
    }

    #[test]
    fn a_camera_by_move_moves_by_the_whole_pixels_the_player_moved() {
        let mut s = quiet(spec());
        s.levels[0].size = [32, 4];
        s.levels[0].tiles = vec![Tile::Empty; 32 * 4];
        s.cameras[0].scroll_back = false;
        s.cameras[0].by_move = Some(crate::spec::ByMove {
            from: n(16),
            full: n(32),
            short_by: n(1),
            push_hold: 0,
            keep_move: false,
            hold_with_counters: false,
        });
        s.levels[0].spawns[0].x = n(10);
        let mut w = World::power_on(&s);
        assert_eq!(
            (w.cameras[0][0], w.seen[0][0]),
            (n(0), n(10)),
            "placed by lead"
        );
        let at = |w: &mut World, x: Fx| {
            w.bodies[0].x = x;
            camera(&s, w, &mut Frame::default());
            (w.cameras[0][0], w.seen[0][0])
        };
        let half = Fx::from_raw(0x8000);
        // Under `from` on the frame before: no move.
        assert_eq!(at(&mut w, n(15).add(half)), (n(0), n(15)));
        assert_eq!(at(&mut w, n(17)), (n(0), n(17)));
        // Past it: the whole pixels moved less one, but at least one.
        assert_eq!(at(&mut w, n(18).add(half)), (n(1), n(17)));
        assert_eq!(at(&mut w, n(21)), (n(3), n(18)));
        // No move, or a move back, doesn't move it.
        assert_eq!(at(&mut w, n(21).add(half)), (n(3), n(18)));
        assert_eq!(at(&mut w, n(20)), (n(3), n(17)));
        // From `full`: the whole move.
        w.cameras[0][0] = n(40);
        w.seen[0][0] = n(32);
        assert_eq!(at(&mut w, n(75)), (n(43), n(32)));
        // The canonical state keeps the place seen.
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back.seen, w.seen);
    }

    /// Core 0.4.23's `hold_with_counters`: the hold counts down on a frame
    /// a state stops the camera, and not on one that stops the counters.
    #[test]
    fn a_hold_with_the_counters_counts_while_the_camera_is_stopped() {
        let mut s = quiet(spec());
        s.cameras[0].by_move = Some(crate::spec::ByMove {
            from: n(16),
            full: n(16),
            short_by: n(0),
            push_hold: 16,
            keep_move: false,
            hold_with_counters: true,
        });
        let mut w = World::power_on(&s);
        w.camera_hold[0] = 5;
        let stopped = |sys: &[System]| Frame {
            stopped: sys.to_vec(),
            ..Frame::default()
        };
        // The camera stopped by a state: the hold still counts.
        run_frozen(System::Camera, &s, &mut w, &mut stopped(&[System::Camera]));
        assert_eq!(w.camera_hold[0], 4);
        // The counters stopped too: it doesn't.
        run_frozen(
            System::Camera,
            &s,
            &mut w,
            &mut stopped(&[System::Camera, System::Counters]),
        );
        assert_eq!(w.camera_hold[0], 4);
        // The camera running with the counters stopped: it doesn't either.
        camera(&s, &mut w, &mut stopped(&[System::Counters]));
        assert_eq!(w.camera_hold[0], 4);
        camera(&s, &mut w, &mut Frame::default());
        assert_eq!(w.camera_hold[0], 3);
        // Without the option the camera's own step counts it, and a stopped
        // camera doesn't.
        s.cameras[0].by_move.as_mut().unwrap().hold_with_counters = false;
        run_frozen(System::Camera, &s, &mut w, &mut stopped(&[System::Camera]));
        assert_eq!(w.camera_hold[0], 3);
        camera(&s, &mut w, &mut stopped(&[System::Counters]));
        assert_eq!(w.camera_hold[0], 2);
    }

    #[test]
    fn a_push_hold_keeps_the_camera_still_after_a_push() {
        let mut s = quiet(spec());
        s.levels[0].size = [32, 4];
        s.levels[0].tiles = vec![Tile::Empty; 32 * 4];
        s.cameras[0].scroll_back = false;
        s.cameras[0].by_move = Some(crate::spec::ByMove {
            from: n(16),
            full: n(16),
            short_by: n(0),
            push_hold: 3,
            keep_move: false,
            hold_with_counters: false,
        });
        s.levels[0].spawns[0].x = n(20);
        let mut w = World::power_on(&s);
        w.cameras[0][0] = n(0);
        w.seen[0][0] = n(20);
        let at = |w: &mut World, x: i64, pushed: bool| {
            w.bodies[0].x = n(x);
            let mut f = Frame::default();
            if pushed {
                f.pushed.push((0, -1));
            }
            camera(&s, w, &mut f);
            (w.cameras[0][0].floor(), w.camera_hold[0])
        };
        // A move scrolls by the move before the push, and the push holds
        // it on the next 2 frames.
        assert_eq!(at(&mut w, 21, true), (2, 3));
        assert_eq!(at(&mut w, 23, false), (2, 2));
        assert_eq!(at(&mut w, 25, false), (2, 1));
        assert_eq!(at(&mut w, 27, false), (4, 0));
        // The canonical state keeps the hold.
        at(&mut w, 28, true);
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back.camera_hold, w.camera_hold);
    }

    #[test]
    fn exits_keep_a_coordinate_scroll_and_need_their_condition() {
        let mut s = quiet(spec());
        s.order.push(System::Exits);
        let mut second = s.levels[0].clone();
        second.name = "two".into();
        s.levels.push(second);
        s.levels[0].exits.push(crate::spec::Exit {
            to: 1,
            at: Rect {
                x: n(20),
                y: n(0),
                w: n(4),
                h: n(64),
            },
            hold: None,
            standing: false,
            enter: Some([n(4), n(4)]),
            start: false,
            guard: Some(crate::spec::Guard {
                counter: 0,
                cmp: crate::spec::Cmp::Eq,
                value: 0,
            }),
            keep: Some(1),
            scroll: 5,
        });
        let mut w = World::power_on(&s);
        let mut took = None;
        for i in 1..40 {
            step(&s, &mut w, 1 << 7);
            if w.level == 1 {
                took = Some(i);
                break;
            }
        }
        assert!(took.is_some());
        assert_eq!(
            (w.bodies[0].x, w.bodies[0].y),
            (n(4), n(32)),
            "y kept, x from enter"
        );
        let still = w.bodies[0];
        for _ in 0..5 {
            step(&s, &mut w, 1 << 7);
        }
        assert_eq!(w.bodies[0], still, "frozen for the scroll");
        step(&s, &mut w, 1 << 7);
        assert!(w.bodies[0].x > still.x);
        // With the condition false, the exit isn't taken.
        let mut w = World::power_on(&s);
        w.counters[0] = 1;
        for _ in 1..40 {
            step(&s, &mut w, 1 << 7);
        }
        assert_eq!(w.level, 0);
    }

    #[test]
    fn own_events_reach_only_their_instance_and_tiles_change() {
        let mut s = quiet(spec());
        let again = s.levels[0].spawns[1].clone();
        s.levels[0].spawns.push(again);
        s.events.push("bug.poked".into());
        let poked = EventKind(s.events.len() - 1);
        let mut m = crate::spec::EntityMachine {
            initial: 0,
            states: Vec::new(),
        };
        for (name, to) in [("calm", 1), ("poked", 1)] {
            let mut k = s.kinds[1].clone();
            k.machine = None;
            m.states.push(crate::spec::EntityState {
                name: name.into(),
                freeze: false,
                part: Default::default(),
                kind: k,
                transitions: vec![Transition {
                    align_to: None,
                    below: None,
                    word: None,
                    pressed: None,
                    held: None,
                    to,
                    on: Some(poked),
                    after: None,
                    align: None,
                    guard: None,
                    actions: Vec::new(),
                }],
            });
        }
        s.kinds[1].machine = Some(m);
        let mut w = World::power_on(&s);
        let mut f = Frame {
            out: vec![false; 3],
            fresh_bodies: vec![false; 3],
            ..Frame::default()
        };
        emit(&s, &mut w, &mut f, poked, Some(2));
        assert_eq!((w.bodies[1].state, w.bodies[2].state), (0, 1));
        emit(&s, &mut w, &mut f, poked, None);
        assert_eq!(w.bodies[1].state, 1, "without an entity, every instance");
        // A changed tile is in the canonical state and collides.
        w.set_tile(&s, 40, 56, Tile::Empty);
        assert_eq!(w.tile_at(&s, 40, 56), Tile::Empty);
        assert_eq!(w.tiles.len(), 1);
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back, w);
        w.set_tile(&s, 40, 56, Tile::Solid);
        assert!(w.tiles.is_empty(), "back as built is no change");
    }

    // Platformer 0.4.2's run options, on made-up numbers: the test game's
    // hero alone, with a run action, and the options `f` sets.
    fn runner(f: impl FnOnce(&mut Run)) -> Spec {
        let mut s = spec();
        s.levels[0].spawns.truncate(1);
        s.levels[0].spawns[0].x = n(48);
        s.actions.push(Action {
            name: "run".into(),
            mask: 1 << 1,
        });
        let run = s.kinds[0].run.as_mut().unwrap();
        run.walk_speed = n(1);
        run.acceleration = Some(Fx(0x2000));
        run.deceleration = Some(Fx(0x2000));
        f(run);
        s
    }

    const RIGHT: u8 = 1 << 7;
    const LEFT: u8 = 1 << 6;
    const RUN: u8 = 1 << 1;

    /// The body's whole speed and its rest, after each frame of `held`.
    fn speeds(s: &Spec, w: &mut World, held: &[u8]) -> Vec<(Fx, Fx)> {
        held.iter()
            .map(|h| {
                step(s, w, *h);
                (w.bodies[0].vx, w.bodies[0].vx_frac)
            })
            .collect()
    }

    fn q(v: f64) -> Fx {
        Fx((v * 65536.0) as i64)
    }

    #[test]
    fn a_move_step_moves_by_whole_steps_and_keeps_the_rest() {
        let s = runner(|r| {
            r.move_step = Some(q(0.25));
            r.acceleration = Some(q(0.1875));
            r.deceleration = Some(q(0.1875));
        });
        let mut w = World::power_on(&s);
        let got = speeds(&s, &mut w, &[RIGHT]);
        assert_eq!(got, vec![(Fx::ZERO, q(0.1875))], "below a step: no move");
        assert_eq!(w.bodies[0].x, n(48));
        let got = speeds(&s, &mut w, &[RIGHT]);
        assert_eq!(got, vec![(q(0.25), q(0.125))]);
        assert_eq!(
            w.bodies[0].x,
            n(48).add(q(0.25)),
            "moved by the whole step only"
        );
        // Let go: it slows while its whole steps aren't 0, then the rest
        // stays, frame after frame.
        let got = speeds(&s, &mut w, &[0, 0, 0]);
        assert_eq!(got, vec![(Fx::ZERO, q(0.1875)); 3]);
        assert_eq!(w.bodies[0].x, n(48).add(q(0.25)));
        // The rest is part of the next press's speed.
        let got = speeds(&s, &mut w, &[RIGHT]);
        assert_eq!(got, vec![(q(0.25), q(0.125))]);
    }

    #[test]
    fn cap_whole_caps_the_whole_steps_and_keeps_the_fraction() {
        let s = runner(|r| {
            r.move_step = Some(q(0.25));
            r.acceleration = Some(q(0.1875));
            r.cap_whole = true;
        });
        let mut w = World::power_on(&s);
        let total: Vec<Fx> = speeds(&s, &mut w, &[RIGHT; 9])
            .into_iter()
            .map(|(v, f)| v.add(f))
            .collect();
        let want = [
            0.1875, 0.375, 0.5625, 0.75, 0.9375, 1.125, 1.0625, 1.0, 1.1875,
        ];
        assert_eq!(total, want.map(q).to_vec(), "the top speed wobbles");
        // Left caps the whole steps at -1, so the speed sits just above it.
        let mut w = World::power_on(&s);
        let total: Vec<Fx> = speeds(&s, &mut w, &[LEFT; 7])
            .into_iter()
            .map(|(v, f)| v.add(f))
            .collect();
        assert_eq!(
            total[5],
            q(-1.125).add(q(0.25)),
            "-1.125 caps to -1 + 0.125"
        );
        assert!(total.iter().all(|v| *v >= n(-1)));
    }

    #[test]
    fn skid_by_last_motion_doubles_while_facing_away_from_the_last_move() {
        let s = runner(|r| {
            r.acceleration = Some(q(0.125));
            r.skid_last_motion = true;
        });
        let mut w = World::power_on(&s);
        assert_eq!(w.bodies[0].moved, 0, "not moved yet");
        let got: Vec<Fx> = speeds(&s, &mut w, &[RIGHT, RIGHT, LEFT, LEFT, LEFT, LEFT])
            .into_iter()
            .map(|(v, _)| v)
            .collect();
        // The first step is doubled (it hasn't moved); a turn's first
        // frame isn't (it faced the way it moved as the frame started); the
        // rest of the turn is, until it moves the new way.
        let want = [0.25, 0.375, 0.25, 0.0, -0.25, -0.375];
        assert_eq!(got, want.map(q).to_vec());
        assert_eq!(w.bodies[0].moved, 2);
    }

    #[test]
    fn a_slow_skid_turns_at_once() {
        let s = runner(|r| {
            r.acceleration = Some(q(0.125));
            r.skid_turn_below = Some(q(0.5));
        });
        let mut w = World::power_on(&s);
        let got = speeds(&s, &mut w, &[RIGHT, RIGHT, RIGHT, LEFT]);
        assert_eq!(got[2].0, q(0.375));
        assert_eq!(got[3].0, q(-0.125), "0, then the rate the new way");
        assert_eq!(w.bodies[0].moved, 2);
        // At or above the speed it skids as before.
        let mut w = World::power_on(&s);
        let got = speeds(&s, &mut w, &[RIGHT, RIGHT, RIGHT, RIGHT, LEFT]);
        assert_eq!(got[4].0, q(0.375));
    }

    #[test]
    fn an_action_of_skid_turn_with_steers_for_the_quick_turn() {
        const RUN_ACTION: usize = 3;
        let with = |steer: bool| {
            runner(move |r| {
                r.acceleration = Some(q(0.125));
                r.deceleration = Some(q(0.125));
                r.skid_turn_below = Some(q(0.5));
                if steer {
                    r.skid_turn_with = vec![RUN_ACTION];
                }
            })
        };
        let s = with(true);
        assert_eq!(s.actions[RUN_ACTION].name, "run");
        let mut w = World::power_on(&s);
        speeds(&s, &mut w, &[RIGHT; 4]);
        // Let go of the direction, the run kept held: at 0.5 it slows as
        // with nothing held, and below 0.5 it stops at once, the rest too.
        let got = speeds(&s, &mut w, &[RUN, RUN, RUN]);
        assert_eq!(
            got,
            vec![
                (q(0.375), Fx::ZERO),
                (Fx::ZERO, Fx::ZERO),
                (Fx::ZERO, Fx::ZERO)
            ]
        );
        assert_eq!(w.bodies[0].moved, 1, "turned to the way it faces");
        // Nothing held slows on to 0.
        let mut w = World::power_on(&s);
        speeds(&s, &mut w, &[RIGHT; 4]);
        let got = speeds(&s, &mut w, &[0, 0]);
        assert_eq!(got[1].0, q(0.25));
        // The run held with the way it moves doesn't turn it.
        let mut w = World::power_on(&s);
        speeds(&s, &mut w, &[RIGHT; 2]);
        let got = speeds(&s, &mut w, &[RIGHT | RUN]);
        assert!(got[0].0 > q(0.25));
        // Without the option, the run held alone doesn't steer.
        let s = with(false);
        let mut w = World::power_on(&s);
        speeds(&s, &mut w, &[RIGHT; 4]);
        let got = speeds(&s, &mut w, &[RUN, RUN]);
        assert_eq!(got[1].0, q(0.25));
    }

    #[test]
    fn decel_bands_slow_faster_from_a_speed_and_the_hold_keeps_it() {
        let bands = |hold: Option<Fx>| {
            runner(move |r| {
                r.walk_speed = n(4);
                r.acceleration = Some(q(0.5));
                r.deceleration = Some(q(0.0625));
                r.decel_bands = vec![(n(2), q(0.25))];
                r.decel_hold_from = hold;
            })
        };
        let s = bands(None);
        let mut w = World::power_on(&s);
        // From 2 up the band's rate is the walking rate held too.
        let up: Vec<Fx> = speeds(&s, &mut w, &[RIGHT; 8])
            .into_iter()
            .map(|v| v.0)
            .collect();
        assert_eq!(up[3..], [2.0, 2.25, 2.5, 2.75, 3.0].map(q));
        let got: Vec<Fx> = speeds(&s, &mut w, &[0; 6])
            .into_iter()
            .map(|v| v.0)
            .collect();
        let want = [2.75, 2.5, 2.25, 2.0, 1.75, 1.6875];
        assert_eq!(got, want.map(q).to_vec(), "the band's rate from 2 up");
        // With the hold from 1.5 the band's rate goes on to the stop.
        let s = bands(Some(q(1.5)));
        let mut w = World::power_on(&s);
        speeds(&s, &mut w, &[RIGHT; 8]);
        let got: Vec<Fx> = speeds(&s, &mut w, &[0; 12])
            .into_iter()
            .map(|v| v.0)
            .collect();
        assert_eq!(got[4..7], [1.75, 1.5, 1.25].map(q));
        assert_eq!(got[11], Fx::ZERO);
        assert!(w.bodies[0].decel_hold, "still held: nothing steered it");
        // Steering the way it moved below 1.5 lets it go.
        speeds(&s, &mut w, &[RIGHT]);
        assert!(!w.bodies[0].decel_hold);
    }

    #[test]
    fn the_run_counts_with_the_motion_and_is_held_after_its_let_go() {
        let s = runner(|r| {
            r.acceleration = Some(q(0.25));
            r.run = Some((n(3), 3));
            r.run_acceleration = Some(q(0.5));
            r.run_with_motion = true;
            r.run_hold_frames = 4;
        });
        let mut w = World::power_on(&s);
        let got = speeds(&s, &mut w, &[RIGHT | RUN, RIGHT | RUN]);
        assert_eq!(got[0].0, q(0.25), "not moved yet: it walks first");
        assert_eq!(got[1].0, q(0.75), "then runs");
        speeds(&s, &mut w, &[RIGHT | RUN; 6]);
        assert_eq!(w.bodies[0].vx, n(3));
        let got: Vec<Fx> = speeds(&s, &mut w, &[RIGHT; 4])
            .into_iter()
            .map(|v| v.0)
            .collect();
        assert_eq!(
            got,
            vec![n(3), n(3), n(3), n(1)],
            "held for 4 frames less one"
        );
        // Without a direction held the run doesn't count after it's moved.
        let s2 = runner(|r| {
            r.run = Some((n(3), 3));
            r.run_with_motion = true;
            r.run_hold_frames = 4;
        });
        let mut w = World::power_on(&s2);
        speeds(&s2, &mut w, &[RUN]);
        assert_eq!(
            w.bodies[0].run_hold, 4,
            "before it has moved, nothing held is its way"
        );
        speeds(&s2, &mut w, &[RIGHT, RUN]);
        assert_eq!(
            w.bodies[0].run_hold, 2,
            "it moved right: the run alone doesn't count"
        );
    }

    /// The options are in the canonical state, and read back.
    #[test]
    fn the_run_state_reads_back() {
        let s = runner(|r| {
            r.move_step = Some(q(0.25));
            r.acceleration = Some(q(0.1875));
            r.skid_last_motion = true;
            r.decel_bands = vec![(q(0.5), q(0.25))];
            r.decel_hold_from = Some(q(0.5));
            r.run = Some((n(2), 3));
            r.run_hold_frames = 3;
        });
        let mut w = World::power_on(&s);
        speeds(&s, &mut w, &[RIGHT | RUN; 5]);
        let b = w.bodies[0];
        assert!(b.vx_frac != Fx::ZERO && b.moved == 1 && b.run_hold == 3 && b.decel_hold);
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back, w);
    }

    // Platformer 0.4.3's fall limit and air options, on made-up numbers.

    #[test]
    fn a_fall_limit_from_half_acts_on_the_whole_part_and_a_half() {
        let four = n(4);
        assert_eq!(fall_limit(q(4.4375), four, false), four, "off: at most 4");
        assert_eq!(
            fall_limit(q(4.4375), four, true),
            q(4.4375),
            "below a half: kept"
        );
        assert_eq!(fall_limit(q(4.875), four, true), four, "a half or more: 4");
        assert_eq!(fall_limit(q(4.5), four, true), four);
        assert_eq!(
            fall_limit(n(5), four, true),
            n(5),
            "5 and no fraction: kept"
        );
        assert_eq!(fall_limit(q(3.9), four, true), q(3.9));
        assert_eq!(fall_limit(q(-4.75), four, true), q(-4.75));
        // A falling body goes 4 and 4.4375 on alternate frames.
        let mut s = runner(|_| {});
        let j = s.kinds[0].jump.as_mut().unwrap();
        j.gravity = q(0.4375);
        j.max_fall_from_half = true;
        let mut w = World::power_on(&s);
        w.bodies[0].y = n(-200);
        w.bodies[0].vy = n(4);
        w.bodies[0].on_ground = false;
        let got: Vec<Fx> = (0..4)
            .map(|_| {
                step(&s, &mut w, 0);
                w.bodies[0].vy
            })
            .collect();
        assert_eq!(got, vec![q(4.4375), n(4), q(4.4375), n(4)]);
        assert_eq!(w.bodies[0].y, n(-200).add(q(16.875)));
        // Off, the same fall is 4 every frame.
        s.kinds[0].jump.as_mut().unwrap().max_fall_from_half = false;
        let mut w = World::power_on(&s);
        w.bodies[0].y = n(-200);
        w.bodies[0].vy = n(4);
        w.bodies[0].on_ground = false;
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].vy, n(4));
    }

    // Platformer 0.4.5's frame on the ground without gravity.

    #[test]
    fn no_gravity_on_ground_starts_a_fall_a_frame_after_the_ground_goes() {
        // The floor's right half is gone; the body stands, then is put
        // over the hole still standing, as after a step off a ledge.
        let off_ledge = |on: bool| {
            let mut s = runner(|_| {});
            let w8 = s.levels[0].size[0] as usize;
            for c in 4..w8 {
                s.levels[0].tiles[3 * w8 + c] = Tile::Empty;
            }
            s.kinds[0].jump.as_mut().unwrap().no_gravity_on_ground = on;
            let mut w = World::power_on(&s);
            w.bodies[0].x = n(16);
            step(&s, &mut w, 0);
            step(&s, &mut w, 0);
            assert!(w.bodies[0].on_ground);
            let (y, g) = (w.bodies[0].y, s.kinds[0].jump.as_ref().unwrap().gravity);
            assert_eq!(w.bodies[0].vy, Fx::ZERO);
            w.bodies[0].x = n(96);
            let got: Vec<(Fx, Fx, bool)> = (0..2)
                .map(|_| {
                    step(&s, &mut w, 0);
                    let b = w.bodies[0];
                    (b.y.sub(y), b.vy, b.on_ground)
                })
                .collect();
            (got, g)
        };
        let (got, g) = off_ledge(true);
        assert_eq!(
            got,
            vec![(Fx::ZERO, Fx::ZERO, false), (g, g, false)],
            "on: the first frame off the ledge stays put"
        );
        let (got, g) = off_ledge(false);
        assert_eq!(
            got,
            vec![(g, g, false), (g.add(g).add(g), g.add(g), false)],
            "off: gravity from the first frame"
        );
        // Standing on the floor it changes nothing, and a jump still
        // leaves on the frame it's pressed.
        let mut s = runner(|_| {});
        s.kinds[0].jump.as_mut().unwrap().no_gravity_on_ground = true;
        let mut w = World::power_on(&s);
        for _ in 0..3 {
            step(&s, &mut w, 0);
        }
        let y = w.bodies[0].y;
        assert!(w.bodies[0].on_ground && w.bodies[0].vy == Fx::ZERO);
        step(&s, &mut w, JUMP);
        let j = s.kinds[0].jump.as_ref().unwrap();
        assert_eq!(w.bodies[0].vy, j.speed.add(j.gravity));
        assert_eq!(w.bodies[0].y, y.add(j.speed).add(j.gravity));
    }

    #[test]
    fn land_keeps_fraction_keeps_the_heights_fraction_until_a_jump() {
        let land = |on: bool| {
            let mut s = runner(|_| {});
            s.kinds[0].jump.as_mut().unwrap().land_keeps_fraction = on;
            let mut w = World::power_on(&s);
            // Falling onto the floor from a height with a fraction.
            w.bodies[0].y = n(10).add(q(0.34375));
            w.bodies[0].vy = n(2);
            w.bodies[0].on_ground = false;
            for _ in 0..30 {
                step(&s, &mut w, 0);
            }
            assert!(w.bodies[0].on_ground);
            let landed = w.bodies[0].y;
            step(&s, &mut w, JUMP);
            (s, landed, w.bodies[0].y)
        };
        let (s, landed, jumped) = land(true);
        assert_eq!(landed.floor(), 32, "on the floor's top");
        assert_ne!(landed.frac(), 0, "the fraction kept");
        let j = s.kinds[0].jump.as_ref().unwrap();
        assert_eq!(
            jumped,
            n(32).add(j.speed).add(j.gravity),
            "a takeoff clears it"
        );
        let (_, landed, _) = land(false);
        assert_eq!(landed, n(32), "off: the fraction cleared");
    }

    // Platformer 0.4.5's head point, on made-up numbers: a ceiling of a
    // coin block over column 1 and a brick over column 4, in row 1.

    fn header(f: impl FnOnce(&mut Head)) -> (Spec, EventKind) {
        let mut s = runner(|_| {});
        s.events.push("block.bumped".into());
        let bumped = EventKind(s.events.len() - 1);
        let w8 = s.levels[0].size[0] as usize;
        s.levels[0].tiles[w8 + 1] = Tile::CoinBlock;
        s.levels[0].tiles[w8 + 4] = Tile::Brick;
        let mut hd = Head {
            busy_while: None,
            point: [n(8), n(0)],
            above: None,
            row_from: 4,
            keep_fraction: true,
            hits: vec![
                HeadHit {
                    above: None,
                    tile: Tile::CoinBlock,
                    speed: n(0),
                    event: Some(bumped),
                    becomes: Some(Tile::Solid),
                    hold: 0,
                },
                HeadHit {
                    above: None,
                    tile: Tile::Brick,
                    speed: n(-2),
                    event: None,
                    becomes: Some(Tile::Empty),
                    hold: 0,
                },
            ],
        };
        f(&mut hd);
        s.kinds[0].jump.as_mut().unwrap().head = Some(hd);
        (s, bumped)
    }

    /// The body at `x`, `y` rising at `vy`, after one frame: its height,
    /// speed and the frame's events.
    fn rise(s: &Spec, x: Fx, y: Fx, vy: Fx) -> (World, Vec<Event>) {
        let mut w = World::power_on(s);
        let b = &mut w.bodies[0];
        (b.x, b.y, b.vy, b.on_ground) = (x, y, vy, false);
        let ev = step(s, &mut w, 0);
        (w, ev)
    }

    // Platformer 0.4.18's pickup points, on made-up numbers: coins at
    // columns 1 and 2 of row 2, and a wall at column 3 of the same row.

    #[test]
    fn a_pickup_takes_one_coin_a_frame_and_skips_the_walls() {
        let mut s = runner(|r| {
            r.walls = Some(crate::spec::Walls {
                left: vec![],
                right: vec![[n(24), n(8)]],
            });
        });
        s.events.push("coin.taken".into());
        let taken = EventKind(s.events.len() - 1);
        let w8 = s.levels[0].size[0] as usize;
        s.levels[0].tiles[2 * w8 + 1] = Tile::Coin;
        s.levels[0].tiles[2 * w8 + 2] = Tile::Coin;
        s.levels[0].tiles[2 * w8 + 3] = Tile::Solid;
        s.kinds[0].jump.as_mut().unwrap().pickup = Some(crate::spec::Pickup {
            tile: Tile::Coin,
            points: vec![[n(0), n(8)], [n(8), n(8)]],
            event: Some(taken),
            becomes: Tile::Empty,
        });
        let start = |w: &mut World, x: i64| {
            let b = &mut w.bodies[0];
            (b.x, b.y, b.vx, b.vy) = (n(x), n(34), n(0), n(0));
            b.on_ground = false;
        };
        let mut w = World::power_on(&s);
        // Two points in two coins: the first point's alone goes.
        start(&mut w, 24);
        let ev = step(&s, &mut w, 0);
        assert_eq!(ev.iter().filter(|e| e.kind == taken).count(), 1);
        assert_eq!(w.tile_at(&s, 24, 40), Tile::Empty);
        assert_eq!(w.tile_at(&s, 32, 40), Tile::Coin);
        // The wall at x + 24 stayed untested, so the body wasn't pushed.
        assert_eq!(w.bodies[0].x, n(24));
        // The next frame takes the second coin, and the wall pushes then
        // only on the frame after.
        start(&mut w, 24);
        let ev = step(&s, &mut w, 0);
        assert_eq!(ev.iter().filter(|e| e.kind == taken).count(), 1);
        assert_eq!(w.tile_at(&s, 32, 40), Tile::Empty);
        assert_eq!(w.bodies[0].x, n(24));
        start(&mut w, 24);
        let ev = step(&s, &mut w, 0);
        assert!(!ev.iter().any(|e| e.kind == taken));
        assert_eq!(w.bodies[0].x, n(23));
        assert!(!Tile::Coin.solid(), "a coin stops nothing");
    }

    /// Features `head.busy_while` and `head.above`: `busy_while` makes every block a solid hit,
    /// with no event and no change; a hit's `above` takes a coin sitting
    /// on the block, as Super Mario Bros.' NYSpd and CheckTopOfBlock do.
    #[test]
    fn a_busy_head_hits_blocks_as_solid_and_a_hit_takes_the_coin_above() {
        let mut s = spec();
        s.counters.push(Counter {
            name: "busy".into(),
            start: 0,
            changes: vec![],
            min: Some(0),
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
            restarts_clock: false,
            high_of: None,
        });
        let busy = s.counters.len() - 1;
        let (mut s2, bumped) = header(|h| {
            h.busy_while = Some(crate::spec::Guard {
                counter: busy,
                cmp: crate::spec::Cmp::Gt,
                value: 0,
            });
        });
        s2.counters = s.counters.clone();
        s2.events.push("coin.above".into());
        let taken = EventKind(s2.events.len() - 1);
        s2.levels[0].tiles[1] = Tile::Coin;
        s2.kinds[0]
            .jump
            .as_mut()
            .unwrap()
            .head
            .as_mut()
            .unwrap()
            .hits[0]
            .above = Some(crate::spec::HeadAbove {
            tile: Tile::Coin,
            becomes: Tile::Empty,
            event: Some(taken),
        });
        // Busy: the coin block is hit as an unlisted solid cell, speed 0.
        let mut w = World::power_on(&s2);
        w.counters[busy] = 1;
        let b = &mut w.bodies[0];
        (b.x, b.y, b.vy, b.on_ground) = (n(12), n(34), q(-3.5), false);
        let ev = step(&s2, &mut w, 0);
        assert_eq!(w.bodies[0].vy, q(0.75));
        assert!(!ev.iter().any(|e| e.kind == bumped || e.kind == taken));
        assert_eq!(w.tile_at(&s2, 20, 30), Tile::CoinBlock, "not used");
        assert_eq!(w.tile_at(&s2, 20, 14), Tile::Coin, "the coin stays");
        // Not busy: the hit, and the coin on top goes with its event.
        let (w, ev) = rise(&s2, n(12), n(34), q(-3.5));
        assert!(ev.iter().any(|e| e.kind == bumped));
        assert!(ev.iter().any(|e| e.kind == taken && e.entity == Some(0)));
        assert_eq!(w.tile_at(&s2, 20, 14), Tile::Empty);
    }

    /// Feature `transition.align_to`: `clock_of` finds a counter's first `every` clock, and
    /// `align_to` waits for a frame on which it has just come round.
    #[test]
    fn align_to_waits_for_the_counters_clock() {
        let mut s = spec();
        let every = |n| Change::Every(n, -1);
        let counter = |name: &str, changes| Counter {
            name: name.into(),
            start: 0,
            changes,
            min: Some(0),
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
            restarts_clock: false,
            high_of: None,
        };
        let first = crate::world::tick_count(&s);
        s.counters.push(counter("a", vec![every(3), every(5)]));
        s.counters.push(counter("b", vec![every(4)]));
        let b = s.counters.len() - 1;
        assert_eq!(clock_of(&s, b), Some(first + 2));
        let t = Transition {
            to: 0,
            on: None,
            after: Some(1),
            align: None,
            align_to: Some(b),
            below: None,
            word: None,
            pressed: None,
            held: None,
            guard: None,
            actions: vec![],
        };
        let mut w = World::power_on(&s);
        w.ticks[first + 2] = 3;
        assert!(!on_clock(&s, &w, &t));
        w.ticks[first + 2] = 0;
        assert!(on_clock(&s, &w, &t));
    }

    /// Features `transition.pressed` and `state.stops_machines`, on a
    /// made-up pause: a press of the jump action enters a frozen state in
    /// which the entities' machines wait and an `every` counter stops
    /// (from the press's own frame, with `stops_at_once`); a press is
    /// taken again only after its `after`, and the counter counts on the
    /// frame it is left (`state.frees_at_once`).
    #[test]
    fn a_press_pauses_and_the_entities_machines_wait() {
        let mut s = quiet(spec());
        s.counters.push(Counter {
            name: "clock".into(),
            start: 0,
            changes: vec![Change::Every(1, 1)],
            min: None,
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
            restarts_clock: false,
            high_of: None,
        });
        let clock = s.counters.len() - 1;
        let press = |to: usize, after: Option<u32>| Transition {
            to,
            on: None,
            after,
            align: None,
            align_to: None,
            below: None,
            word: None,
            pressed: Some(0),
            held: None,
            guard: None,
            actions: vec![],
        };
        s.machines.push(crate::spec::Machine {
            name: "pause".into(),
            initial: 0,
            states: vec![
                State {
                    name: "running".into(),
                    freeze: false,
                    part: Default::default(),
                    transitions: vec![press(1, None)],
                },
                State {
                    name: "paused".into(),
                    freeze: true,
                    part: crate::spec::Part {
                        stops_machines: true,
                        stops_at_once: vec![System::Counters],
                        frees_at_once: vec![System::Counters],
                        ..Default::default()
                    },
                    transitions: vec![press(0, Some(5))],
                },
            ],
        });
        let pause = s.machines.len() - 1;
        let spin = crate::spec::EntityState {
            name: "spin".into(),
            freeze: false,
            part: crate::spec::Part {
                step_word: 1000,
                ..Default::default()
            },
            kind: s.kinds[0].clone(),
            transitions: Vec::new(),
        };
        s.kinds[0].machine = Some(crate::spec::EntityMachine {
            initial: 0,
            states: vec![spin],
        });
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        step(&s, &mut w, 0);
        let (c0, w0) = (w.counters[clock], w.bodies[0].ext[0]);
        // The press: the machine goes to paused, and the counter skips
        // this frame already; the entity's machine still ran.
        step(&s, &mut w, JUMP);
        assert_eq!(w.machines[pause].state, 1);
        assert_eq!(w.counters[clock], c0);
        assert_eq!(w.bodies[0].ext[0], w0 + 1);
        // Paused: nothing counts, and a press before 5 frames does nothing.
        for k in 0..6 {
            step(&s, &mut w, if k == 2 { JUMP } else { 0 });
            assert_eq!(w.machines[pause].state, 1, "frame {k}");
        }
        assert_eq!(w.counters[clock], c0);
        assert_eq!(w.bodies[0].ext[0], w0 + 1);
        // A press after 5 frames unpauses: the counter counts that frame
        // (`frees_at_once`), and from the next frame all counts.
        step(&s, &mut w, JUMP);
        assert_eq!(w.machines[pause].state, 0);
        assert_eq!(w.counters[clock], c0 + 1);
        assert_eq!(w.bodies[0].ext[0], w0 + 1);
        step(&s, &mut w, 0);
        assert_eq!(w.counters[clock], c0 + 2);
        assert_eq!(w.bodies[0].ext[0], w0 + 2);
    }

    /// Features `transition.below` and `action.emit`: a transition `below` is taken once the body is at that
    /// depth, and `emit:` fires the event, as Super Mario Bros.'
    /// PlayerHole ends the death at Player_Y_HighPos 4.
    #[test]
    fn a_transition_below_is_taken_at_its_depth_and_emits() {
        let mut s = runner(|_| {});
        s.events.push("player.lost".into());
        let lost = EventKind(s.events.len() - 1);
        let kind = s.kinds[0].clone();
        let state = |name: &str, transitions| crate::spec::EntityState {
            name: name.into(),
            freeze: false,
            part: crate::spec::Part::default(),
            kind: kind.clone(),
            transitions,
        };
        s.kinds[0].machine = Some(crate::spec::EntityMachine {
            initial: 0,
            states: vec![
                state(
                    "falling",
                    vec![Transition {
                        to: 1,
                        on: None,
                        after: None,
                        align: None,
                        align_to: None,
                        below: Some(n(20)),
                        word: None,
                        pressed: None,
                        held: None,
                        guard: None,
                        actions: vec![Do::Emit(lost)],
                    }],
                ),
                state("gone", vec![]),
            ],
        });
        let mut w = World::power_on(&s);
        w.bodies[0].y = n(10);
        let ev = step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].state, 0, "above the depth");
        assert!(!ev.iter().any(|e| e.kind == lost));
        w.bodies[0].y = n(20);
        let ev = step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].state, 1, "at the depth");
        assert!(ev.iter().any(|e| e.kind == lost));
    }

    #[test]
    fn a_head_hit_sets_the_speeds_whole_part_and_changes_the_cell() {
        let (s, bumped) = header(|_| {});
        // 34 - 3.25 is 30.75: the point (20, 30) is in the coin block,
        // 14 pixels into its row. The speed, -3.25 (-4 and 0.75), becomes
        // 0 and 0.75; the body stays where the move put it.
        let (w, ev) = rise(&s, n(12), n(34), q(-3.5));
        let b = w.bodies[0];
        assert_eq!((b.y, b.vy), (q(30.75), q(0.75)));
        assert!(ev.iter().any(|e| e.kind == bumped && e.entity == Some(0)));
        assert_eq!(w.tile_at(&s, 20, 30), Tile::Solid, "the block is used");
        // A brick sets -2 and is gone.
        let (w, _) = rise(&s, n(60), n(34), q(-3.5));
        assert_eq!(w.bodies[0].vy, n(-2).add(q(0.75)));
        assert_eq!(w.tile_at(&s, 68, 30), Tile::Empty);
        // Without keep_fraction the speed is the hit's.
        let (s2, _) = header(|h| h.keep_fraction = false);
        let (w, _) = rise(&s2, n(12), n(34), q(-3.5));
        assert_eq!(w.bodies[0].vy, n(0));
    }

    #[test]
    fn a_head_point_tests_only_its_point_and_rows_from_row_from() {
        let (s, _) = header(|_| {});
        // 22 - 3.25 is 18.75: 2 pixels into the row, under row_from.
        let (w, ev) = rise(&s, n(12), n(22), q(-3.5));
        assert_eq!((w.bodies[0].y, w.bodies[0].vy), (q(18.75), q(-3.25)));
        assert!(ev.is_empty());
        // The point (10, 30) is in the empty column left of the block,
        // though the hitbox reaches into it: the body rises on.
        let (w, _) = rise(&s, n(2), n(34), q(-3.5));
        assert_eq!((w.bodies[0].y, w.bodies[0].vy), (q(30.75), q(-3.25)));
        // Not while falling, nor above `above`.
        let (w, _) = rise(&s, n(12), n(26), q(0.5));
        assert_eq!(w.bodies[0].vy, q(0.75));
        let (s3, _) = header(|h| h.above = Some(n(32)));
        let (w, _) = rise(&s3, n(12), n(34), q(-3.5));
        assert_eq!(w.bodies[0].vy, q(-3.25));
        // Without a head the hitbox's top stops the rise under the row.
        let s4 = runner(|_| {});
        let mut s4 = s4;
        let w8 = s4.levels[0].size[0] as usize;
        s4.levels[0].tiles[w8 + 1] = Tile::CoinBlock;
        let (w, _) = rise(&s4, n(2), n(34), q(-3.5));
        assert_eq!((w.bodies[0].y, w.bodies[0].vy), (n(32), n(0)));
    }

    // Platformer 0.4.6's held cell.

    #[test]
    fn a_held_cell_stays_solid_for_its_frames_then_changes() {
        // The brick is held 2 frames, then empty; the coin block 3, then
        // its kind again (no `becomes`).
        let (s, _) = header(|h| {
            h.hits[1].hold = 2;
            h.hits[0].hold = 3;
            h.hits[0].becomes = None;
        });
        let (mut w, _) = rise(&s, n(60), n(34), q(-3.5));
        assert_eq!(w.bodies[0].vy, n(-2).add(q(0.75)), "the brick's hit");
        assert_eq!(
            w.tile_at(&s, 68, 30),
            Tile::Solid,
            "held on the hit's frame"
        );
        assert_eq!(w.held_cells.len(), 1);
        // Put the body back under it, rising: the held cell is solid, and
        // a solid cell no hit lists sets 0.
        let canon = w.canonical();
        assert_eq!(
            World::from_canonical(&s, &canon).unwrap(),
            w,
            "in the state"
        );
        let mut tiles = Vec::new();
        for _ in 0..3 {
            let b = &mut w.bodies[0];
            (b.x, b.y, b.vy, b.on_ground) = (n(60), n(34), q(-3.5), false);
            step(&s, &mut w, 0);
            tiles.push((w.tile_at(&s, 68, 30), w.bodies[0].vy));
        }
        assert_eq!(
            tiles,
            vec![
                (Tile::Solid, q(0.75)),
                (Tile::Solid, q(0.75)),
                (Tile::Empty, q(-3.25)),
            ],
            "solid on the 2 frames after the hit's, then empty"
        );
        assert!(w.held_cells.is_empty());
        let (mut w, _) = rise(&s, n(12), n(34), q(-3.5));
        assert_eq!(w.tile_at(&s, 20, 30), Tile::Solid);
        for _ in 0..3 {
            step(&s, &mut w, 0);
        }
        assert_eq!(w.tile_at(&s, 20, 30), Tile::Solid, "held 3 frames");
        step(&s, &mut w, 0);
        assert_eq!(w.tile_at(&s, 20, 30), Tile::CoinBlock, "back as it was");
        // Without a hold the cell changes at once.
        let (s, _) = header(|_| {});
        let (w, _) = rise(&s, n(60), n(34), q(-3.5));
        assert_eq!(w.tile_at(&s, 68, 30), Tile::Empty);
        assert!(w.held_cells.is_empty());
    }

    // Platformer 0.4.6's fall gravity kept from the last jump.

    #[test]
    fn keep_fall_gravity_falls_off_a_ledge_with_the_last_jumps_gravity() {
        // One band below 1 with gravity 0.5; the start's is 0.125. The
        // floor's right half is gone.
        let ledge = |keep: bool| {
            let mut s = runner(|_| {});
            let w8 = s.levels[0].size[0] as usize;
            for c in 4..w8 {
                s.levels[0].tiles[3 * w8 + c] = Tile::Empty;
            }
            let j = s.kinds[0].jump.as_mut().unwrap();
            j.keep_fall_gravity = keep;
            j.start_fall_gravity = keep.then(|| q(0.125));
            j.bands = vec![crate::spec::JumpBand {
                below: n(1),
                speed: n(-2),
                gravity: q(0.5),
                gravity_held: None,
            }];
            s
        };
        // A fall before any jump takes the start's gravity.
        let s = ledge(true);
        let mut w = World::power_on(&s);
        assert_eq!(w.bodies[0].fall_gravity, q(0.125));
        w.bodies[0].x = n(96);
        step(&s, &mut w, 0);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].vy, q(0.25), "two frames of the start's");
        // After a jump from standing (speed 0, the band), it lands and
        // walks off: its fall takes the band's gravity.
        let mut w = World::power_on(&s);
        w.bodies[0].x = n(16);
        step(&s, &mut w, 0);
        step(&s, &mut w, JUMP);
        assert_eq!(w.bodies[0].fall_gravity, q(0.5));
        for _ in 0..30 {
            step(&s, &mut w, 0);
        }
        assert!(w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].fall_gravity, q(0.5), "kept after landing");
        let canon = w.canonical();
        assert_eq!(
            World::from_canonical(&s, &canon).unwrap(),
            w,
            "in the state"
        );
        w.bodies[0].x = n(96);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].vy, q(0.5));
        // Off, the same fall takes `gravity`.
        let s = ledge(false);
        let mut w = World::power_on(&s);
        w.bodies[0].x = n(96);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].vy, s.kinds[0].jump.as_ref().unwrap().gravity);
    }

    // Platformer 0.4.5's wall points, on made-up numbers: a wall cell at
    // column 4 of row 2, just over the floor.

    #[test]
    fn wall_points_push_a_body_back_a_pixel_after_its_move() {
        let at = |walls: bool, x: Fx, vx: Fx, held: u8| {
            let mut s = runner(|r| {
                r.walk_speed = n(2);
                if walls {
                    r.walls = Some(crate::spec::Walls {
                        left: vec![[n(0), n(8)]],
                        right: vec![[n(15), n(8)]],
                    });
                }
            });
            let w8 = s.levels[0].size[0] as usize;
            s.levels[0].tiles[2 * w8 + 4] = Tile::Solid;
            let mut w = World::power_on(&s);
            step(&s, &mut w, 0);
            (w.bodies[0].x, w.bodies[0].vx) = (x, vx);
            step(&s, &mut w, held);
            (w.bodies[0].x, w.bodies[0].vx)
        };
        // Moving 2 into the wall: it gets in, and is pushed back one pixel.
        assert_eq!(at(true, q(48.5), n(2), RIGHT), (q(49.5), n(0)));
        // Without wall points the hitbox stops it at the wall.
        assert_eq!(at(false, q(48.5), n(2), RIGHT), (n(48), n(0)));
        // Moving away from the side the wall is on: nothing.
        assert_eq!(at(true, q(52.5), n(-2), LEFT), (q(50.5), n(-2)));
        // From the left side a wall pushes right.
        assert_eq!(at(true, q(80.5), n(-2), LEFT), (q(79.5), n(0)));
    }

    #[test]
    fn push_masks_a_direction_held_toward_last_frames_push() {
        let run = |masks: bool| {
            let mut s = runner(|r| {
                r.walk_speed = n(2);
                r.move_step = Some(q(0.0625));
                r.acceleration = Some(q(0.015625));
                r.push_masks = masks;
                r.walls = Some(crate::spec::Walls {
                    left: vec![[n(0), n(8)]],
                    right: vec![[n(15), n(8)]],
                });
            });
            let w8 = s.levels[0].size[0] as usize;
            s.levels[0].tiles[2 * w8 + 4] = Tile::Solid;
            let mut w = World::power_on(&s);
            step(&s, &mut w, 0);
            (w.bodies[0].x, w.bodies[0].vx, w.bodies[0].vx_frac) = (q(48.5), n(1), q(0.03125));
            // Into the wall: pushed back with the speed 0, the rest kept.
            step(&s, &mut w, RIGHT);
            let pushed = (w.bodies[0].vx, w.bodies[0].vx_frac, w.bodies[0].pushed);
            // The next frame, right held.
            step(&s, &mut w, RIGHT);
            let b = w.bodies[0];
            (pushed, b.vx.add(b.vx_frac), b.pushed)
        };
        let ((vx, rest, side), next, after) = run(true);
        assert_eq!((vx, rest, side), (n(0), q(0.046875), 2), "pushed left");
        assert_eq!(next, rest, "held right, masked: nothing changes");
        assert_eq!(after, 0, "found again on that frame: no wall");
        let ((_, rest, side), next, _) = run(false);
        assert_eq!(side, 0, "not marked without push_masks");
        assert_eq!(next, rest.add(q(0.015625)), "right held speeds up");
        // The canonical state keeps the side.
        let s = runner(|r| r.push_masks = true);
        let mut w = World::power_on(&s);
        w.bodies[0].pushed = 1;
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back.bodies[0].pushed, 1);
    }

    /// With `face_both`, both directions held from standing leave the
    /// whole speed at 0 for two frames, the second at a doubled rate, then
    /// it builds at the plain rate (ProcSkid, X_Physics, ImposeFriction).
    #[test]
    fn face_both_from_standing_stops_two_frames_then_builds_undoubled() {
        let rate = Fx(0x400);
        let s = runner(|r| {
            r.walk_speed = n(2);
            r.acceleration = Some(rate);
            r.move_step = Some(q(0.0625));
            r.skid_last_motion = true;
            r.skid_turn_below = Some(q(0.5));
            r.face_both = true;
        });
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        w.bodies[0].moved = 1;
        let mut speeds = Vec::new();
        for _ in 0..3 {
            step(&s, &mut w, RIGHT | LEFT);
            let b = &w.bodies[0];
            speeds.push((b.vx, b.vx_frac, b.moved));
        }
        // ProcSkid nulls the speed while the way held, both, isn't the way
        // it last moved, and copies the facing into that way: right on the
        // first frame, both on the second, when the rate is doubled since
        // the facing isn't the way. On the third the rate is plain.
        assert_eq!(
            speeds,
            [
                (Fx::ZERO, rate, 1),
                (Fx::ZERO, rate.add(rate), 3),
                (Fx::ZERO, rate.add(rate).add(rate), 3),
            ]
        );
        assert!(w.bodies[0].faced_both);
    }

    /// `face_both` (platformer 0.4.15): both directions held turn the body
    /// to neither way, so the next frame doubles the rate, right winning
    /// for the speed; a slow skid leaves it last moved both ways.
    #[test]
    fn face_both_is_a_facing_and_a_way_of_its_own() {
        let rate = Fx(0x400);
        let run = |on: bool| {
            let s = runner(|r| {
                r.walk_speed = n(2);
                r.acceleration = Some(rate);
                r.move_step = Some(q(0.0625));
                r.skid_last_motion = true;
                r.skid_turn_below = Some(q(0.5));
                r.face_both = on;
            });
            let mut w = World::power_on(&s);
            step(&s, &mut w, 0);
            let b = &mut w.bodies[0];
            (b.vx, b.moved) = (n(1), 1);
            // The rate is as the frame began: not doubled yet.
            step(&s, &mut w, RIGHT | LEFT);
            let first = (w.bodies[0].vx_frac, w.bodies[0].faced_both);
            // Right alone turns it back, the rate doubled once more.
            step(&s, &mut w, RIGHT);
            let second = (w.bodies[0].vx_frac, w.bodies[0].faced_both);
            (first, second)
        };
        assert_eq!(run(true).0, (rate, true));
        assert_eq!(run(true).1, (rate.add(rate).add(rate), false));
        assert_eq!(run(false).0, (rate, false));
        assert_eq!(run(false).1, (rate.add(rate), false));

        let s = runner(|r| {
            r.acceleration = Some(rate);
            r.move_step = Some(q(0.0625));
            r.skid_last_motion = true;
            r.skid_turn_below = Some(q(0.5));
            r.face_both = true;
        });
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        w.bodies[0].moved = 1;
        // Facing both on the frame after, it stops and takes that way.
        step(&s, &mut w, RIGHT | LEFT);
        step(&s, &mut w, RIGHT | LEFT);
        assert_eq!(w.bodies[0].moved, 3);
        assert_eq!(w.bodies[0].vx, Fx::ZERO);
        // The canonical state keeps both.
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!((back.bodies[0].faced_both, back.bodies[0].moved), (true, 3));
    }

    #[test]
    fn speed_before_walls_keeps_the_speed_the_run_left_for_its_tests() {
        let run = |before: bool| {
            let mut s = runner(|r| {
                r.walk_speed = n(2);
                r.move_step = Some(q(0.0625));
                r.acceleration = Some(q(0.0625));
                r.deceleration = Some(q(0.0625));
                r.decel_bands = vec![(n(2), q(0.25))];
                r.decel_hold_from = Some(q(1.5));
                r.speed_before_walls = before;
                r.walls = Some(crate::spec::Walls {
                    left: vec![[n(0), n(8)]],
                    right: vec![[n(15), n(8)]],
                });
            });
            let w8 = s.levels[0].size[0] as usize;
            s.levels[0].tiles[2 * w8 + 4] = Tile::Solid;
            let mut w = World::power_on(&s);
            step(&s, &mut w, 0);
            let b = &mut w.bodies[0];
            (b.x, b.vx, b.moved, b.decel_hold) = (q(48.5), n(2), 1, true);
            b.seen_speed = n(2);
            // Into the wall: the speed is 0, the speed seen is still 2.
            step(&s, &mut w, RIGHT);
            let into = (w.bodies[0].vx, w.bodies[0].seen_speed);
            // Right held the way it moved: the hold clears only below 1.5,
            // which the speed seen isn't yet.
            step(&s, &mut w, RIGHT);
            (into, w.bodies[0].decel_hold, w.bodies[0].seen_speed)
        };
        let ((vx, seen), hold, after) = run(true);
        assert_eq!((vx, seen), (n(0), n(2)), "the wall stops the speed only");
        assert!(hold, "the frame after the wall still sees 2");
        assert_eq!(after, q(0.25), "the held band's rate: the speed it made");
        let (_, hold, _) = run(false);
        assert!(!hold, "without it the speed as the frame starts is 0");
        // The canonical state keeps it.
        let s = runner(|r| r.speed_before_walls = true);
        let mut w = World::power_on(&s);
        w.bodies[0].seen_speed = q(1.25);
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back.bodies[0].seen_speed, q(1.25));
    }

    #[test]
    fn collide_y_meets_tiles_only_inside_its_band() {
        let fall = |band: Option<[i64; 2]>| {
            let mut s = runner(|_| {});
            s.kinds[0].jump.as_mut().unwrap().collide_y = band;
            let mut w = World::power_on(&s);
            let b = &mut w.bodies[0];
            (b.x, b.y, b.vy, b.on_ground) = (n(20), n(30), n(3), false);
            step(&s, &mut w, 0);
            (w.bodies[0].y.floor(), w.bodies[0].on_ground)
        };
        // The floor's top is 48 and the hero 16 tall: it lands at 32.
        assert_eq!(fall(None), (32, true));
        assert_eq!(fall(Some([0, 40])), (32, true));
        // Below the band it goes through the floor.
        assert!(!fall(Some([0, 30])).1);
    }

    #[test]
    fn land_depth_lands_only_near_a_tiles_top_and_walls_the_feet_deeper() {
        let fall_to = |depth: Option<i64>, y: Fx| {
            let mut s = runner(|r| r.skid_last_motion = true);
            s.kinds[0].jump.as_mut().unwrap().land_depth = depth;
            let mut w = World::power_on(&s);
            let b = &mut w.bodies[0];
            (b.x, b.y, b.vy, b.vx, b.on_ground, b.moved) = (n(20), y, n(3), n(0), false, 1);
            step(&s, &mut w, 0);
            let b = w.bodies[0];
            (b.x, b.y, b.on_ground)
        };
        // The floor's top is 48; the hero is 16 tall. 32 + 3.25 puts its
        // bottom 3 pixels into the floor: it lands.
        assert_eq!(fall_to(Some(5), n(32)), (n(20), n(32), true));
        // 35 + 3.25: 6 pixels in, too deep. It falls on, a pixel left of
        // where it was, the way it last moved being right.
        assert_eq!(fall_to(Some(5), n(35)), (n(19), q(38.25), false));
        // Without land_depth it lands from any depth.
        assert_eq!(fall_to(None, n(35)), (n(20), n(32), true));
    }

    #[test]
    fn ground_points_look_for_ground_at_their_columns_only() {
        let stands_at = |feet: Option<Vec<Fx>>, x: Fx| {
            let mut s = runner(|_| {});
            let w8 = s.levels[0].size[0] as usize;
            for c in 4..w8 {
                s.levels[0].tiles[3 * w8 + c] = Tile::Empty;
            }
            s.kinds[0].jump.as_mut().unwrap().feet = feet;
            let mut w = World::power_on(&s);
            w.bodies[0].x = n(16);
            step(&s, &mut w, 0);
            assert!(w.bodies[0].on_ground);
            w.bodies[0].x = x;
            step(&s, &mut w, 0);
            w.bodies[0].on_ground
        };
        // The floor ends at 64. The hitbox, 16 wide, still stands at 61.
        assert!(stands_at(None, n(61)));
        // Ground points at 3 and 12: 64 and 73 are past the ledge.
        assert!(!stands_at(Some(vec![n(3), n(12)]), n(61)));
        assert!(stands_at(Some(vec![n(3), n(12)]), n(60)));
    }

    const JUMP: u8 = 1;

    /// A body moving right at `speed` on the ground, with the run's
    /// options `f` sets.
    fn moving(speed: Fx, f: impl FnOnce(&mut Run)) -> (Spec, World) {
        let s = runner(f);
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        assert!(w.bodies[0].on_ground);
        w.bodies[0].vx = speed;
        w.bodies[0].moved = 1;
        (s, w)
    }

    #[test]
    fn takeoff_in_air_keeps_the_speed_on_the_frame_a_jump_starts() {
        // Off: the takeoff frame slows on the ground.
        let (s, mut w) = moving(n(1), |_| {});
        step(&s, &mut w, JUMP);
        assert!(!w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].vx, n(1).sub(Fx(0x2000)), "slowed");
        // On: nothing held keeps the speed, as in the air.
        let (s, mut w) = moving(n(1), |r| r.takeoff_in_air = true);
        step(&s, &mut w, JUMP);
        assert!(!w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].vx, n(1), "kept");
        // A frame on the ground that doesn't jump slows as before.
        let (s, mut w) = moving(n(1), |r| r.takeoff_in_air = true);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].vx, n(1).sub(Fx(0x2000)));
    }

    /// Feature `run.face_way`: with `takeoff_in_air` and no air control,
    /// a direction held on the takeoff frame turns the body; without it the
    /// takeoff frame is the air's and keeps the facing.
    #[test]
    fn face_way_turns_on_the_takeoff_frame() {
        for way in [false, true] {
            let (s, mut w) = moving(q(0.5), |r| {
                r.takeoff_in_air = true;
                r.air_control = false;
                r.face_way = way;
            });
            assert!(!w.bodies[0].facing_left);
            step(&s, &mut w, JUMP | LEFT);
            assert!(!w.bodies[0].on_ground);
            assert_eq!(w.bodies[0].facing_left, way, "face_way {way}");
        }
    }

    #[test]
    fn face_on_ground_keeps_the_facing_in_the_air() {
        for keep in [false, true] {
            let (s, mut w) = moving(q(0.5), |r| r.face_on_ground = keep);
            step(&s, &mut w, JUMP | RIGHT);
            step(&s, &mut w, LEFT);
            assert!(!w.bodies[0].on_ground);
            assert_eq!(w.bodies[0].facing_left, !keep, "keep {keep}");
            assert_eq!(w.bodies[0].vx, q(0.5), "the speed moves either way");
        }
        // On the ground it turns.
        let (s, mut w) = moving(q(0.5), |r| r.face_on_ground = true);
        step(&s, &mut w, LEFT);
        assert!(w.bodies[0].facing_left);
    }

    #[test]
    fn air_run_from_picks_the_run_by_speed_in_the_air() {
        let opts = |r: &mut Run| {
            r.run = Some((n(2), 3));
            r.run_acceleration = Some(q(0.25));
            r.air_run_from = Some(q(1.5));
        };
        // At 1.5 or more, in the air, the run counts without its action:
        // the run's rate, up past the walk speed.
        let aloft = |speed: Fx| {
            let (s, mut w) = moving(speed, opts);
            w.bodies[0].on_ground = false;
            w.bodies[0].y = n(0);
            (s, w)
        };
        let (s, mut w) = aloft(q(1.5));
        step(&s, &mut w, RIGHT);
        assert_eq!(w.bodies[0].vx, q(1.75));
        step(&s, &mut w, RIGHT);
        assert_eq!(w.bodies[0].vx, n(2));
        // Below it the run action doesn't count in the air.
        let (s, mut w) = aloft(q(0.5));
        step(&s, &mut w, RIGHT | RUN);
        assert_eq!(w.bodies[0].vx, q(0.625), "the walking rate");
        // On the ground the run action counts as before.
        let (s, mut w) = moving(q(0.5), opts);
        step(&s, &mut w, RIGHT | RUN);
        assert_eq!(w.bodies[0].vx, q(0.75));
    }

    /// The test game with its bug turned into a springboard under the
    /// hero: placed in the floor's row, so its top is the floor's and the
    /// hero stands on it from the first frame. It presses 2 frames down
    /// (2 px a frame, 4 px offset) and 2 up, and throws at -3, or -6 with
    /// jump newly pressed from its second step.
    fn springboard(late: bool) -> Spec {
        use crate::spec::{SpringBoost, SpringStep, Springboard};
        let mut s = spec();
        let k = &mut s.kinds[1];
        k.name = "spring".into();
        k.patrol = None;
        k.contact = None;
        k.springboard = Some(Springboard {
            steps: vec![
                SpringStep {
                    frames: 2,
                    moves: n(2),
                    offset: n(4),
                },
                SpringStep {
                    frames: 2,
                    moves: n(-2),
                    offset: n(0),
                },
            ],
            speed: n(-3),
            boost: Some(SpringBoost {
                speed: n(-6),
                action: 0,
                from_step: 1,
                held: false,
            }),
            throw_late: late,
        });
        s.levels[0].spawns[1] = Spawn {
            kind: 1,
            x: n(0),
            y: n(48),
            patrol_end: None,
            drawn: false,
            at_edge: None,
            appear_moving: false,
        };
        s
    }

    #[test]
    fn a_springboard_holds_the_player_then_throws_it() {
        let s = springboard(false);
        let mut w = World::power_on(&s);
        let ys = |w: &World| (w.bodies[0].y, w.bodies[1].y, w.bodies[1].ext[0]);
        step(&s, &mut w, 0);
        assert_eq!(
            ys(&w),
            (n(34), n(52), 1),
            "pressed on the frame it's stood on"
        );
        assert!(!w.bodies[1].on_ground, "pressing");
        step(&s, &mut w, 0);
        assert_eq!(ys(&w), (n(36), n(52), 2), "held: no gravity, no landing");
        step(&s, &mut w, 0);
        assert_eq!(ys(&w), (n(34), n(48), 3));
        step(&s, &mut w, 0);
        assert_eq!(ys(&w), (n(32), n(48), 0), "thrown on the last frame");
        assert_eq!(w.bodies[0].vy, n(-3));
        assert!(!w.bodies[0].on_ground && w.bodies[1].on_ground);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].y, n(32).add(n(-3)).add(Fx(0x4000)));
        assert_eq!(w.bodies[1].ext, [0; 4], "at rest");
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back, w);
    }

    #[test]
    fn a_new_press_from_the_boost_step_throws_higher() {
        let s = springboard(false);
        let throw = |presses: [u8; 4]| {
            let mut w = World::power_on(&s);
            for b in presses {
                step(&s, &mut w, b);
            }
            w.bodies[0].vy
        };
        assert_eq!(throw([0, 0, 1, 0]), n(-6), "pressed in the second step");
        assert_eq!(throw([0, 0, 0, 1]), n(-6), "pressed on the throw's frame");
        assert_eq!(
            throw([0, 1, 1, 1]),
            n(-3),
            "pressed in the first step and held"
        );
        // Held, the hero doesn't jump either.
        let mut w = World::power_on(&s);
        step(&s, &mut w, 0);
        step(&s, &mut w, 1);
        assert_eq!(w.bodies[0].y, n(36));
    }

    #[test]
    fn a_held_player_changes_speed_but_keeps_its_place() {
        let s = springboard(false);
        let mut w = World::power_on(&s);
        // The first frame moves: the press starts after the player's turn.
        for _ in 0..4 {
            step(&s, &mut w, 1 << 7);
        }
        assert_eq!((w.bodies[0].x, w.bodies[0].vx), (n(1), n(1)));
        step(&s, &mut w, 1 << 7);
        assert_eq!(w.bodies[0].x, n(2), "thrown, it moves across again");
    }

    #[test]
    fn a_late_throw_stands_a_frame_then_rises() {
        let s = springboard(true);
        let mut w = World::power_on(&s);
        for _ in 0..4 {
            step(&s, &mut w, 0);
        }
        assert_eq!((w.bodies[0].y, w.bodies[0].vy), (n(32), n(-3)));
        assert!(w.bodies[0].on_ground, "still standing as it's thrown");
        step(&s, &mut w, 1 << 7);
        assert_eq!((w.bodies[0].x, w.bodies[0].y), (n(1), n(32)));
        assert!(!w.bodies[0].on_ground, "left the ground, not moved up");
        assert_eq!(w.bodies[1].ext[0], 0, "not pressed again");
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].y, n(32).add(n(-3)).add(Fx(0x4000)));
        // A jump on the late frame is an ordinary jump.
        let mut w = World::power_on(&s);
        for _ in 0..4 {
            step(&s, &mut w, 0);
        }
        step(&s, &mut w, 1);
        assert!(!w.bodies[0].on_ground);
        assert_eq!(w.bodies[0].y, n(32).add(n(-4)).add(Fx(0x4000)));
    }

    #[test]
    fn without_a_late_springboard_an_upward_speed_on_the_ground_moves() {
        let s = spec();
        let mut w = World::power_on(&s);
        w.bodies[0].vy = n(-3);
        step(&s, &mut w, 0);
        assert!(w.bodies[0].y < n(32), "the rule is the springboard's only");
    }

    /// The test game in a level 64 cells wide with the hero at 0 and a bug
    /// at each x, on the floor, spawning by the view 16 pixels ahead.
    fn view_spec(xs: &[(i64, bool)]) -> Spec {
        let mut s = spec();
        let l = &mut s.levels[0];
        let (w, h) = (64usize, 4usize);
        l.size = [w as i64, h as i64];
        l.tiles = vec![Tile::Empty; w * h];
        for c in 0..w {
            l.tiles[3 * w + c] = Tile::Solid;
        }
        l.spawns.truncate(1);
        for &(x, drawn) in xs {
            l.spawns.push(Spawn {
                kind: 1,
                x: n(x),
                y: n(32),
                patrol_end: Some(n(x + 200)),
                drawn,
                at_edge: None,
                appear_moving: false,
            });
        }
        l.spawn_ahead = Some(n(16));
        l.kill_below = None;
        s
    }

    /// Runs the spawn system with the view's near edge at `near`.
    fn at_view(s: &Spec, w: &mut World, near: i64) -> Frame {
        w.cameras[0][0] = n(near);
        let mut f = Frame {
            fresh_bodies: vec![false; w.bodies.len()],
            ..Frame::default()
        };
        spawn(s, w, &mut f);
        f
    }

    fn shown(w: &World) -> Vec<bool> {
        w.bodies[1..].iter().map(|b| b.active()).collect()
    }

    #[test]
    fn a_spawn_at_edge_is_placed_past_the_views_last_column() {
        let mut s = view_spec(&[(200, false), (200, false)]);
        s.levels[0].appear_still = true;
        s.levels[0].spawns[1].at_edge = Some(n(0));
        s.levels[0].spawns[2].at_edge = Some(n(24));
        s.levels[0].spawns[2].appear_moving = true;
        let mut w = World::power_on(&s);
        let width = s.cameras[0].size[0];
        // Not reached yet: both wait.
        let near = 200 - width - 16;
        at_view(&s, &mut w, near);
        assert_eq!(shown(&w), [false, false]);
        // Reached: the first at the view's last column, the second 24 on,
        // and only the first still on its first frame.
        let f = at_view(&s, &mut w, near + 1);
        assert_eq!(shown(&w), [true, true]);
        let last = near + 1 + width - 1;
        assert_eq!(w.bodies[1].x, n(last));
        assert_eq!(
            (w.bodies[2].x, w.bodies[2].home[0]),
            (n(last + 24), n(last + 24))
        );
        assert_eq!(f.still, vec![1]);
    }

    /// `orbit.place_on_appear`: placed as it appears, its parts still with
    /// it that frame, and placed once. Without it the spawn system leaves
    /// it where the level put it.
    #[test]
    fn an_orbit_with_place_on_appear_is_placed_as_it_appears() {
        let orbit = |on| {
            let mut s = view_spec(&[(200, false)]);
            s.levels[0].appear_still = true;
            s.kinds[1].orbit = Some(Orbit {
                spin: n(1),
                angles: 32,
                parts: 2,
                part: 1,
                place: [n(4), n(4)],
                skip: Vec::new(),
                in_view: false,
                place_on_appear: on,
                angle_in_slot: false,
            });
            s
        };
        let s = orbit(true);
        let mut w = World::power_on(&s);
        let (x, y) = (w.bodies[1].x, w.bodies[1].y);
        let near = 200 - s.cameras[0].size[0] - 15;
        let f = at_view(&s, &mut w, near);
        assert_eq!((w.bodies[1].x, w.bodies[1].y), (x.add(n(4)), y.add(n(4))));
        assert_eq!(w.bodies.len(), 4);
        assert_eq!(f.still, vec![1, 2, 3]);
        at_view(&s, &mut w, near);
        assert_eq!(w.bodies[1].x, x.add(n(4)), "placed once");

        let s = orbit(false);
        let mut w = World::power_on(&s);
        let f = at_view(&s, &mut w, near);
        assert_eq!((w.bodies[1].x, w.bodies.len()), (x, 2));
        assert_eq!(f.still, vec![1]);
    }

    #[test]
    fn spawn_slots_hold_a_spawn_until_one_is_free() {
        let mut s = view_spec(&[(100, false), (110, false)]);
        s.levels[0].spawn_slots = Some(1);
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 40);
        assert_eq!(shown(&w), [true, false], "one slot: the second waits");
        w.bodies[1].alive = false;
        at_view(&s, &mut w, 40);
        assert_eq!(shown(&w), [false, true], "the slot is free again");
    }

    /// A view spec whose first spawn is a `stream` marker sending bugs, with
    /// two slots: a wait of 2 plus 3, then 2 plus 5, and heights 40 and 30.
    fn stream_spec() -> Spec {
        let mut s = view_spec(&[(200, false)]);
        s.levels[0].spawn_slots = Some(2);
        s.levels[0].spawns[1].kind = 2;
        s.counters.push(Counter {
            restarts_clock: false,
            high_of: None,
            name: "random".into(),
            start: 0x0200,
            changes: vec![],
            min: None,
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: Some(crate::spec::Shift {
                bits: 16,
                taps: vec![9, 1],
            }),
        });
        let mut marker = s.kinds[1].clone();
        marker.name = "marker".into();
        marker.no_slot = true;
        marker.ext_tables.insert("heights".into(), vec![40, 30]);
        marker.ext_tables.insert("times".into(), vec![3, 5]);
        marker.stream = Some(crate::spec::Stream {
            entity: 1,
            past_edge: n(10),
            wait: 2,
            draw: crate::spec::LeapDraw {
                counter: s.counters.len() - 1,
                at: 8,
                slot_step: 0,
                fraction: 0,
            },
            until: None,
        });
        s.kinds.push(marker);
        s
    }

    #[test]
    fn a_stream_sends_bodies_past_the_edge_in_turn_while_a_slot_is_free() {
        let s = stream_spec();
        let mut w = World::power_on(&s);
        // The marker appears and sends its first on the same frame. The byte
        // is 0x81 after this frame's step, so the row is 1.
        at_view(&s, &mut w, 150);
        assert_eq!(shown(&w), [true, true]);
        let first = w.bodies[2];
        let (near, width) = view(&s, &w);
        let last = near.floor() + width.floor() - 1;
        assert_eq!((first.x, first.y), (n(last + 10), n(30)));
        assert_eq!(first.ext, [1, 0, 0, 1]);
        assert_eq!(w.bodies.len(), 3, "one wait of 5 starts");
        assert_eq!(w.bodies[1].ext, [5, 1, 0, 0]);
        for _ in 0..4 {
            at_view(&s, &mut w, 150);
        }
        assert_eq!(w.bodies.len(), 3, "not yet");
        at_view(&s, &mut w, 150);
        assert_eq!(w.bodies.len(), 4, "the wait is out");
        assert_eq!(w.bodies[1].ext, [7, 2, 0, 0], "the next row of the times");
        for _ in 0..7 {
            at_view(&s, &mut w, 150);
        }
        assert_eq!(w.bodies.len(), 4, "both slots are taken");
        w.bodies[2].alive = false;
        at_view(&s, &mut w, 150);
        assert_eq!(w.bodies.len(), 5, "a slot is free again");
    }

    #[test]
    fn a_stream_ends_once_its_until_kind_is_in_the_level() {
        let mut s = stream_spec();
        s.kinds[2].stream.as_mut().unwrap().until = Some(0);
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 150);
        assert!(!w.bodies[1].alive, "the hero is a body of its until kind");
        assert_eq!(w.bodies.len(), 2);
    }

    #[test]
    fn a_stream_goes_on_while_its_until_kind_still_waits_to_appear() {
        let mut s = stream_spec();
        s.levels[0].spawns.push(Spawn {
            kind: 1,
            x: n(600),
            y: n(32),
            patrol_end: None,
            drawn: false,
            at_edge: None,
            appear_moving: false,
        });
        s.kinds[2].stream.as_mut().unwrap().until = Some(1);
        let mut w = World::power_on(&s);
        assert!(w.bodies[2].waiting);
        at_view(&s, &mut w, 150);
        assert!(w.bodies[1].alive, "the far body hasn't appeared yet");
        assert_eq!(w.bodies.len(), 4, "the stream sent its first");
    }

    #[test]
    fn in_order_a_spawn_waits_for_those_before_it_and_one_in_view_is_dropped() {
        // The view is 64 wide: with it at 0 its last column is 63.
        let mut s = view_spec(&[(50, false), (300, false), (70, false)]);
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 0);
        assert_eq!(shown(&w), [true, false, true], "without an order");
        s.levels[0].spawn_in_order = true;
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 0);
        assert_eq!(shown(&w), [false, false, false]);
        assert!(
            !w.bodies[1].alive && !w.bodies[1].waiting,
            "50 was in view at its turn"
        );
        assert!(w.bodies[3].waiting, "70 waits for 300");
        at_view(&s, &mut w, 237);
        // 300 comes in reach, its column the view's last; 70 is then left
        // of it, and dropped.
        assert_eq!(shown(&w), [false, true, false]);
        assert!(!w.bodies[3].alive);
    }

    #[test]
    fn in_order_a_spawn_takes_one_turn_a_free_slot() {
        let mut s = view_spec(&[(100, false), (104, false), (108, false)]);
        s.levels[0].spawn_in_order = true;
        s.levels[0].spawn_slots = Some(2);
        let mut w = World::power_on(&s);
        // All three in reach (to 117), 100 the view's last column.
        at_view(&s, &mut w, 37);
        assert_eq!(shown(&w), [true, true, false], "two slots, two turns");
        w.bodies[1].alive = false;
        at_view(&s, &mut w, 37);
        assert_eq!(shown(&w), [false, true, true]);
    }

    #[test]
    fn a_drawn_spawn_appears_as_its_column_is_drawn() {
        // Drawn 32 past the far edge at the start (to 96 with the view at
        // 0), then 32 pixels a set, a 16-pixel column every 2 frames, and
        // spawning by the view held the frame after a set starts.
        let mut s = view_spec(&[(104, true), (120, true), (116, false)]);
        s.levels[0].draw = Some(crate::spec::Draw {
            ahead: n(32),
            step: 32,
            every: 2,
            pause: 1,
        });
        let mut w = World::power_on(&s);
        assert_eq!(w.draw_set, 0);
        at_view(&s, &mut w, 31);
        assert_eq!(shown(&w), [false, false, false]);
        // The view passes 32: a set starts and draws the column at 96.
        at_view(&s, &mut w, 32);
        assert_eq!(shown(&w), [true, false, false]);
        assert_eq!((w.draw_set, w.draw_left), (1, 3));
        // 116 is in reach at 37, but the frame after the set starts holds it.
        at_view(&s, &mut w, 40);
        assert_eq!(shown(&w), [true, false, false]);
        // The set's second column, at 112, two frames after its first.
        at_view(&s, &mut w, 40);
        assert_eq!(shown(&w), [true, true, true]);
        at_view(&s, &mut w, 40);
        assert_eq!(w.draw_left, 0);
    }

    /// A level with a draw 32 past the far edge, a 16-pixel column every 2
    /// frames, and a check at column `x`.
    fn loop_spec(x: i64, y: i64) -> Spec {
        let mut s = view_spec(&[(104, false), (120, true)]);
        s.levels[0].draw = Some(crate::spec::Draw {
            ahead: n(32),
            step: 32,
            every: 2,
            pause: 0,
        });
        s.levels[0].loop_back = Some(crate::spec::LoopBack {
            back: 64,
            keep: 4,
            checks: vec![crate::spec::LoopCheck { x, y }],
        });
        s
    }

    /// The world with the view at 64 and the draw done to 160.
    fn drawn_to_check(s: &Spec) -> World {
        let mut w = World::power_on(s);
        w.cameras[0][0] = n(64);
        w.draw_set = 2;
        w.draw_left = 0;
        w
    }

    #[test]
    fn a_loop_back_sends_the_player_and_view_back_when_the_draw_reaches_it() {
        let s = loop_spec(160, 1000);
        let mut w = drawn_to_check(&s);
        let px = w.bodies[0].x;
        // A set still being drawn holds it.
        w.draw_left = 1;
        at_view(&s, &mut w, 64);
        assert_eq!(w.cameras[0][0], n(64));
        at_view(&s, &mut w, 64);
        assert_eq!(w.cameras[0][0], n(0));
        assert_eq!(w.bodies[0].x, px.sub(n(64)));
        assert_eq!(w.draw_set, 0);
    }

    #[test]
    fn a_loop_back_clears_the_level_and_keeps_drawn_spawns_behind_gone() {
        let s = loop_spec(160, 1000);
        let mut w = drawn_to_check(&s);
        w.bodies[1].waiting = false;
        w.bodies[2].waiting = false;
        at_view(&s, &mut w, 64);
        assert!(
            w.bodies[1].waiting && w.bodies[1].alive,
            "waits at its spawn"
        );
        // The draw's at 96 now and the drawn spawn's column, 112, is ahead.
        assert!(w.bodies[2].waiting && w.bodies[2].alive);
        let s = loop_spec(224, 1000);
        let mut w = drawn_to_check(&s);
        w.draw_set = 4;
        w.bodies[1].waiting = false;
        w.bodies[2].waiting = false;
        at_view(&s, &mut w, 128);
        assert!(!w.bodies[2].alive, "its column isn't drawn again");
    }

    #[test]
    fn a_loop_back_lets_a_player_standing_at_the_height_pass() {
        let s = loop_spec(160, 40);
        let mut w = drawn_to_check(&s);
        w.bodies[0].on_ground = true;
        w.bodies[0].y = n(40);
        at_view(&s, &mut w, 64);
        assert_eq!(w.cameras[0][0], n(64));
        // In the air at that height he goes back.
        let mut w = drawn_to_check(&s);
        w.bodies[0].on_ground = false;
        w.bodies[0].y = n(40);
        at_view(&s, &mut w, 64);
        assert_eq!(w.cameras[0][0], n(0));
    }

    #[test]
    fn a_loop_back_keeps_its_columns_and_redraws_the_rest() {
        let s = loop_spec(160, 1000);
        let mut w = drawn_to_check(&s);
        // A change behind the draw goes back; one the draw reaches again
        // is as built.
        w.set_tile(&s, 144, 0, Tile::Solid);
        w.set_tile(&s, 112, 16, Tile::Solid);
        at_view(&s, &mut w, 64);
        assert_eq!(w.tile_at(&s, 80, 0), Tile::Solid);
        assert_eq!(w.tile_at(&s, 48, 16), Tile::Solid);
        assert_eq!(w.tile_at(&s, 112, 16), Tile::Empty);
    }

    #[test]
    fn a_scroll_stop_column_removes_its_kinds_and_stops_the_camera() {
        // As in the test above: the set from view 32 draws the columns at
        // 96 and 112, two frames apart.
        let mut s = view_spec(&[(104, true)]);
        s.levels[0].draw = Some(crate::spec::Draw {
            ahead: n(32),
            step: 32,
            every: 2,
            pause: 1,
        });
        s.levels[0].scroll_stop = Some(crate::spec::ScrollStop {
            at: n(112),
            removes: vec![1],
        });
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 32);
        assert_eq!(shown(&w), [true], "its column isn't drawn yet");
        assert!(!scroll_stopped(&s, &w));
        at_view(&s, &mut w, 40);
        at_view(&s, &mut w, 40);
        assert_eq!(shown(&w), [false], "gone as the column is drawn");
        assert!(scroll_stopped(&s, &w));
        // The camera stays, wherever its entity goes.
        w.bodies[0].x = n(300);
        place_camera(&s, &mut w, 0, false);
        assert_eq!(w.cameras[0][0], n(40));
    }

    #[test]
    fn a_drawn_spawn_with_no_free_slot_never_appears() {
        let mut s = view_spec(&[(70, false), (104, true)]);
        s.levels[0].spawn_slots = Some(1);
        s.levels[0].draw = Some(crate::spec::Draw {
            ahead: n(32),
            step: 32,
            every: 2,
            pause: 0,
        });
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 0);
        at_view(&s, &mut w, 32);
        assert_eq!(shown(&w), [true, false]);
        assert!(!w.bodies[2].alive && !w.bodies[2].waiting);
        w.bodies[1].alive = false;
        at_view(&s, &mut w, 32);
        assert_eq!(shown(&w), [false, false], "its column isn't drawn again");
    }

    #[test]
    fn a_body_that_appears_still_moves_from_the_next_frame() {
        for still in [false, true] {
            let mut s = view_spec(&[(100, false)]);
            s.levels[0].appear_still = still;
            let mut w = World::power_on(&s);
            let mut f = at_view(&s, &mut w, 40);
            let x = w.bodies[1].x;
            enemies(&s, &mut w, &mut f);
            assert_eq!(w.bodies[1].x == x, still, "still {still}");
            let mut f = at_view(&s, &mut w, 40);
            enemies(&s, &mut w, &mut f);
            assert_ne!(w.bodies[1].x, x);
        }
    }

    #[test]
    fn despawning_by_position_and_a_kinds_own_distance() {
        let mut s = view_spec(&[(100, false)]);
        s.levels[0].despawn_behind = Some(n(8));
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 40);
        // 16 wide at 100: its far edge is 116, its position 100.
        w.bodies[1].x = n(100);
        at_view(&s, &mut w, 124);
        assert!(
            w.bodies[1].alive,
            "the far edge, 116, isn't more than 8 behind 124"
        );
        s.levels[0].despawn_by_position = true;
        at_view(&s, &mut w, 108);
        assert!(
            w.bodies[1].alive,
            "the position is more than 8 behind only past 108"
        );
        s.kinds[1].despawn_behind = Some(n(20));
        at_view(&s, &mut w, 120);
        assert!(w.bodies[1].alive, "its own 20");
        at_view(&s, &mut w, 121);
        assert!(!w.bodies[1].alive);
    }

    #[test]
    fn a_dropped_carry_puts_the_line_behind_a_page_back() {
        // OffscreenBoundsCheck's LimitB: 57 added to the low byte drops its
        // carry, so from 199 on the line is 271 behind, not 16.
        let mut s = view_spec(&[(100, false)]);
        s.levels[0].despawn_behind = Some(n(8));
        s.levels[0].despawn_by_position = true;
        s.kinds[1].despawn_behind = Some(n(16));
        s.kinds[1].despawn_behind_wrap = Some([199, 271]);
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 40);
        // 742 is page 2's 230, past 199, so the line is 471; 768 is page
        // 3's 0, so it is 752.
        w.bodies[1].x = n(720);
        at_view(&s, &mut w, 742);
        assert!(w.bodies[1].alive, "726 would take it; the carry is dropped");
        at_view(&s, &mut w, 767);
        assert!(w.bodies[1].alive, "still the low byte's 255");
        at_view(&s, &mut w, 768);
        assert!(!w.bodies[1].alive, "the line is 752 again");
    }

    #[test]
    fn a_kinds_own_distance_ahead_takes_only_that_kind() {
        let mut s = view_spec(&[(100, false), (100, false)]);
        s.levels[0].despawn_behind = Some(n(8));
        s.levels[0].despawn_by_position = true;
        let mut k = s.kinds[1].clone();
        k.despawn_ahead = Some(n(30));
        s.kinds.push(k);
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 40);
        w.bodies[2].kind = 2;
        let (_, width) = view(&s, &w);
        w.bodies[1].x = n(40).add(width).add(n(31));
        w.bodies[2].x = n(40).add(width).add(n(30));
        at_view(&s, &mut w, 40);
        assert!(w.bodies[1].alive, "no distance of its own, so it stays");
        assert!(w.bodies[2].alive, "not past its own 30");
        w.bodies[2].x = w.bodies[2].x.add(n(1));
        at_view(&s, &mut w, 40);
        assert!(w.bodies[1].alive);
        assert!(!w.bodies[2].alive, "past its own 30");
    }

    #[test]
    fn despawning_after_the_move_goes_on_the_frame_of_the_step() {
        let mut s = view_spec(&[(100, false)]);
        s.levels[0].despawn_behind = Some(n(8));
        s.levels[0].despawn_by_position = true;
        let run = |s: &Spec| {
            let mut w = World::power_on(s);
            at_view(s, &mut w, 40);
            w.bodies[1].x = n(100);
            at_view(s, &mut w, 108);
            assert!(w.bodies[1].alive, "100 is on the line, 8 behind 108");
            w.bodies[1].x = n(90);
            let mut f = Frame {
                fresh_bodies: vec![false; w.bodies.len()],
                ..Frame::default()
            };
            enemies(s, &mut w, &mut f);
            w.bodies[1].alive
        };
        assert!(run(&s), "the spawn system's next turn takes it");
        s.levels[0].despawn_after_move = true;
        assert!(!run(&s), "gone on the frame it moved past the line");
    }

    #[test]
    fn a_level_with_a_draw_keeps_its_draw_in_the_canonical_state() {
        let mut s = view_spec(&[(104, true)]);
        s.levels[0].draw = Some(crate::spec::Draw {
            ahead: n(32),
            step: 32,
            every: 2,
            pause: 1,
        });
        let mut w = World::power_on(&s);
        at_view(&s, &mut w, 32);
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!((back.draw_set, back.draw_left), (1, 3));
        assert_eq!(back.hash(), w.hash());
    }

    /// The test spec with one more counter, `name`, starting at `start`, and
    /// its events named; its index is returned.
    fn with_counter(s: &mut Spec, name: &str, start: i64) -> usize {
        s.counters.push(Counter {
            restarts_clock: false,
            high_of: None,
            name: name.into(),
            start,
            changes: Vec::new(),
            min: Some(0),
            max: None,
            wrap: false,
            runs_when_frozen: false,
            shift: None,
        });
        let mut events: Vec<String> = crate::spec::BUILTIN_EVENTS
            .iter()
            .map(|e| e.to_string())
            .collect();
        for c in &s.counters {
            events.push(format!("{}.reached_min", c.name));
            events.push(format!("{}.wrapped", c.name));
        }
        s.events = events;
        s.own_events = s.events.len();
        s.counters.len() - 1
    }

    /// Platformer 0.4.10's `invincible`: while its counter holds, a touch
    /// that would kill the player defeats the toucher, or does its
    /// `contact.invincible`; with the counter at 0 the touch kills.
    #[test]
    fn an_invincible_player_defeats_what_it_touches() {
        for (star, touch, defeated, died) in [
            (5, Touch::Defeat, true, false),
            (5, Touch::Nothing, false, false),
            (0, Touch::Defeat, false, true),
        ] {
            let mut s = spec();
            let c = with_counter(&mut s, "star", star);
            s.kinds[1].patrol = None;
            s.levels[0].spawns[1].x = n(8);
            s.kinds[1].contact.as_mut().unwrap().invincible = touch;
            s.kinds[0].invincible = Some(crate::spec::Guard {
                counter: c,
                cmp: crate::spec::Cmp::Gt,
                value: 0,
            });
            let mut w = World::power_on(&s);
            let ev = step(&s, &mut w, 0);
            let has = |k: EventKind| ev.iter().any(|e| e.kind == k);
            assert_eq!(
                has(EventKind::ENEMY_DEFEATED),
                defeated,
                "star {star} {touch:?}"
            );
            assert_eq!(!w.bodies[1].alive, defeated);
            assert_eq!(has(EventKind::PLAYER_DIED), died, "star {star} {touch:?}");
        }
    }

    /// Core 0.4.13's `after` and `of`: a change on an event of one kind's
    /// entity is made that many frames later, counted while frozen too,
    /// and an event of another kind's changes nothing.
    /// Feature `action.add`: `add:<to>:<from>` changes the first counter
    /// by the second's value, bounds included, as Donkey Kong adds the
    /// bonus timer to the score at the stage clear.
    /// Feature `counter.high_of`: a counter that keeps another's highest
    /// value is raised as that one changes, and never lowered.
    #[test]
    fn a_high_counter_keeps_the_highest_value() {
        let mut s = spec();
        let score = with_counter(&mut s, "score", 0);
        let top = with_counter(&mut s, "top", 500);
        s.counters[top].high_of = Some(score);
        let mut w = World::power_on(&s);
        let mut f = begin_frame(&s, &mut w, 0);
        act(&s, &mut w, &mut f, Do::Change(score, 300));
        assert_eq!(w.counters[top], 500, "below the top: kept");
        act(&s, &mut w, &mut f, Do::Change(score, 400));
        assert_eq!(w.counters[top], 700, "past it: raised");
        act(&s, &mut w, &mut f, Do::Set(score, 0));
        assert_eq!(
            (w.counters[score], w.counters[top]),
            (0, 700),
            "a new game keeps it"
        );
    }

    #[test]
    fn an_add_action_adds_one_counter_to_another() {
        let mut s = spec();
        let bonus = with_counter(&mut s, "bonus", 3300);
        let score = with_counter(&mut s, "score", 200);
        s.counters[score].max = Some(3000);
        let mut w = World::power_on(&s);
        let mut f = begin_frame(&s, &mut w, 0);
        act(&s, &mut w, &mut f, Do::Add(score, bonus));
        assert_eq!(w.counters[score], 3000, "held at its max");
        assert_eq!(w.counters[bonus], 3300, "the one added is unchanged");
        s.counters[score].max = None;
        let mut w = World::power_on(&s);
        let mut f = begin_frame(&s, &mut w, 0);
        act(&s, &mut w, &mut f, Do::Add(score, bonus));
        assert_eq!(w.counters[score], 3500);
        // `set:` puts a counter at a value, within its bounds.
        act(&s, &mut w, &mut f, Do::Set(bonus, 5000));
        assert_eq!(w.counters[bonus], 5000);
        s.counters[bonus].max = Some(4000);
        act(&s, &mut w, &mut f, Do::Set(bonus, 5000));
        assert_eq!(w.counters[bonus], 4000, "held at its max");
    }

    #[test]
    fn a_change_waits_its_frames_and_takes_only_its_kinds() {
        for (of, by) in [(1usize, 7i64), (0, 0)] {
            let mut s = spec();
            let star = with_counter(&mut s, "star", 5);
            let score = with_counter(&mut s, "score", 0);
            s.counters[score].changes = vec![Change::When(crate::spec::OnChange {
                on: EventKind::ENEMY_DEFEATED,
                by: 7,
                of: vec![of],
                after: 3,
                guard: None,
            })];
            s.kinds[1].patrol = None;
            s.levels[0].spawns[1].x = n(8);
            s.kinds[1].contact.as_mut().unwrap().invincible = Touch::Defeat;
            s.kinds[0].invincible = Some(crate::spec::Guard {
                counter: star,
                cmp: crate::spec::Cmp::Gt,
                value: 0,
            });
            let mut w = World::power_on(&s);
            step(&s, &mut w, 0);
            assert!(!w.bodies[1].alive);
            for _ in 0..2 {
                step(&s, &mut w, 0);
                assert_eq!(w.counters[score], 0, "not yet");
            }
            let back = World::from_canonical(&s, &w.canonical()).unwrap();
            assert_eq!(back.pending, w.pending, "pending changes round-trip");
            step(&s, &mut w, 0);
            assert_eq!(w.counters[score], by, "3 frames after, of kind {of}");
            assert!(w.pending.is_empty());
        }
    }

    /// Core 0.4.13's `ground`: entering a `leave` state takes the body off
    /// the ground, and a `land` state stands it on the ground under it from
    /// the frame after it is entered, while the player system is stopped.
    #[test]
    fn a_state_leaves_the_ground_and_another_lands() {
        let mut s = quiet(spec());
        let base = s.kinds[0].clone();
        let state = |name: &str, ground, to: Option<(usize, u32)>| crate::spec::EntityState {
            name: name.into(),
            freeze: false,
            part: crate::spec::Part {
                stops: vec![System::Player],
                ground,
                ..Default::default()
            },
            kind: base.clone(),
            transitions: to
                .map(|(to, after)| Transition {
                    align_to: None,
                    below: None,
                    word: None,
                    pressed: None,
                    held: None,
                    to,
                    on: None,
                    after: Some(after),
                    align: None,
                    guard: None,
                    actions: Vec::new(),
                })
                .into_iter()
                .collect(),
        };
        s.kinds[0].machine = Some(crate::spec::EntityMachine {
            initial: 0,
            states: vec![
                state("still", None, Some((1, 1))),
                state("hit", Some(crate::spec::Ground::Leave), Some((2, 2))),
                state("back", Some(crate::spec::Ground::Land), None),
            ],
        });
        let mut w = World::power_on(&s);
        assert!(w.bodies[0].on_ground);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].state, 1);
        assert!(
            !w.bodies[0].on_ground,
            "off the ground as the state is entered"
        );
        step(&s, &mut w, 0);
        step(&s, &mut w, 0);
        assert_eq!(w.bodies[0].state, 2);
        assert!(!w.bodies[0].on_ground, "not on the frame `land` is entered");
        step(&s, &mut w, 0);
        assert!(w.bodies[0].on_ground, "on the ground from the next");
    }

    /// Platformer 0.4.10's `bounce`: falling onto a tile at its point
    /// bounces the body up with its speed's fraction kept and its height
    /// snapped; the next tile it meets while rising removes it.
    #[test]
    fn a_bounce_off_the_ground_and_gone_on_the_next_touch() {
        let mut s = quiet(spec());
        s.kinds[1].tile_collision = false;
        s.kinds[1].fall = Some(crate::spec::Fall {
            gravity: Fx(0x5000),
            max_fall: n(3),
            max_fall_from_half: true,
            move_first: true,
            feet: None,
            stands_still: false,
        });
        s.kinds[1].bounce = Some(crate::spec::Bounce {
            point: [n(4), n(8)],
            speed: n(-3),
            snap: 8,
            from_y: None,
        });
        s.levels[0].spawns[1].y = n(30);
        let mut w = World::power_on(&s);
        w.bodies[1].vy = n(3);
        let mut bounced = None;
        for _ in 0..8 {
            step(&s, &mut w, 0);
            if w.bodies[1].vy < Fx::ZERO {
                bounced = Some(w.bodies[1]);
                break;
            }
        }
        let b = bounced.expect("it bounces");
        assert_eq!(b.y.floor() % 8, 0, "its height snapped to 8");
        assert_eq!(b.vy.floor(), -3);
        assert!(b.bounced && b.alive);
        // Rising with its point in the ground (as against a wall), it goes.
        w.bodies[1].y = n(45);
        w.bodies[1].vy = n(-1);
        w.bodies[1].bounced = false;
        step(&s, &mut w, 0);
        assert!(!w.bodies[1].alive, "a tile met while rising removes it");
    }

    /// Platformer 0.4.10's `wait_near_wrap`: the distance to the player is
    /// taken modulo it, so a body a wrap and a few pixels away waits.
    #[test]
    fn a_path_waits_by_the_distance_modulo_its_wrap() {
        for (wrap, moves) in [(0u32, true), (256, false)] {
            let mut s = quiet(spec());
            s.kinds[1].path = Some(crate::spec::Path {
                vertical: true,
                distance: n(-8),
                speed: n(1),
                pause: 0,
                wait_near: Some(n(33)),
                wait_near_wrap: wrap,
                every: 0,
                phase: 0,
            });
            s.kinds[1].tile_collision = false;
            let mut w = World::power_on(&s);
            w.bodies[1].x = n(260);
            w.bodies[1].home = [n(260), w.bodies[1].y];
            let y = w.bodies[1].y;
            step(&s, &mut w, 0);
            assert_eq!(w.bodies[1].y != y, moves, "wrap {wrap}");
        }
    }

    /// Platformer 0.4.10's shot: from the shooter's whole-pixel place after
    /// it moves, `ahead` not mirrored, with its speeds, a flier keeping its
    /// speed, and gone once its right edge's margin passes the view's far
    /// edge.
    #[test]
    fn a_shot_starts_whole_after_the_move_and_keeps_its_speed() {
        let mut s = quiet(spec());
        let mut shot = s.kinds[1].clone();
        shot.name = "shot".into();
        shot.tile_collision = false;
        shot.fly = Some(crate::spec::Fly {
            speed: n(4),
            heading: crate::spec::Heading::Right,
            keep_speed: true,
        });
        shot.leave_view = Some(crate::spec::LeaveView {
            vertical: false,
            margin: n(0),
            far_margin: Some(n(8)),
            bottom_margin: None,
        });
        s.kinds.push(shot);
        s.kinds[0].jump = None;
        s.kinds[0].shoot = Some(crate::spec::Shoot {
            entity: 2,
            action: Some(0),
            every: 0,
            ahead: [n(4), n(0)],
            max: 2,
            unmirrored: true,
            vy: n(4),
            vx: n(3),
            vx_both: None,
            after_move: true,
            whole: true,
            in_slots: false,
        });
        let mut w = World::power_on(&s);
        w.bodies[0].x = Fx(n(20).raw() + 0x8000);
        w.bodies[0].facing_left = true;
        step(&s, &mut w, 1);
        let hero = w.bodies[0];
        let b = *w.bodies.last().unwrap();
        assert_eq!(b.kind, 2);
        assert_eq!(
            b.x,
            Fx::from_int(hero.x.floor() + 4).add(n(-3)),
            "whole, unmirrored, moved by -3"
        );
        assert_eq!(b.vx, n(-3), "the shot's speed kept, mirrored");
        assert_eq!(b.vy, n(4));
        let n_bodies = w.bodies.len();
        w.bodies[n_bodies - 1].x = n(w.cameras[0][0].floor() + 256 - 8 + 3);
        w.bodies[n_bodies - 1].vx = n(-3);
        step(&s, &mut w, 0);
        assert!(!w.bodies[n_bodies - 1].alive, "gone at the far margin");
    }

    /// `shoot.vx_both`: a shooter facing both ways gives the shot that speed
    /// as it is; facing one way, or without the option, the shot takes `vx`.
    #[test]
    fn a_shot_takes_its_own_speed_from_a_shooter_facing_both_ways() {
        let shot = |vx_both: Option<Fx>, both: bool, left: bool| {
            let mut s = spec();
            let kind = s.kinds[1].clone();
            s.kinds.push(kind);
            s.kinds[0].shoot = Some(crate::spec::Shoot {
                entity: 2,
                action: Some(0),
                every: 0,
                ahead: [n(4), n(0)],
                max: 2,
                unmirrored: true,
                vy: n(0),
                vx: n(4),
                vx_both,
                after_move: false,
                whole: true,
                in_slots: false,
            });
            let mut w = World::power_on(&s);
            (w.bodies[0].faced_both, w.bodies[0].facing_left) = (both, left);
            let mut f = Frame::default();
            w.held = 1;
            shoot_at(&s, &mut w, &mut f, 0, false);
            w.bodies.last().unwrap().vx
        };
        let odd = Fx(n(-8).raw() + 0x6000);
        assert_eq!(shot(Some(odd), true, false), odd);
        assert_eq!(shot(Some(odd), false, false), n(4));
        assert_eq!(shot(Some(odd), false, true), n(-4));
        assert_eq!(shot(None, true, false), n(4));
    }

    /// `leave_view.bottom_margin` (platformer 0.4.24): a rule along x also
    /// removes a body at the view's height, and not a pixel above it.
    #[test]
    fn a_bottom_margin_leaves_at_the_views_height() {
        let mut s = quiet(spec());
        s.kinds[0].leave_view = Some(crate::spec::LeaveView {
            vertical: false,
            margin: n(0),
            far_margin: None,
            bottom_margin: Some(n(0)),
        });
        let height = s.cameras[0].size[1];
        let mut w = World::power_on(&s);
        w.bodies[0].x = n(w.cameras[0][0].floor() + 16);
        w.bodies[0].y = n(height - 1);
        leave_view(&s, &mut w, 0);
        assert!(w.bodies[0].alive, "still in the view");
        w.bodies[0].y = n(height);
        leave_view(&s, &mut w, 0);
        assert!(!w.bodies[0].alive, "gone at the view's height");
    }

    /// `shoot.in_slots` (platformer 0.4.12): slots go in turn, a busy
    /// slot refuses the shot even with another free, and a shot keeps the
    /// fractions its slot's last shot left.
    #[test]
    fn shots_in_slots_take_turns_and_keep_fractions() {
        let mut s = spec();
        let mut shot = s.kinds[1].clone();
        shot.name = "shot".into();
        s.kinds.push(shot);
        s.kinds[0].jump = None;
        s.kinds[0].shoot = Some(crate::spec::Shoot {
            entity: 2,
            action: Some(0),
            every: 0,
            ahead: [n(4), n(0)],
            max: 2,
            unmirrored: true,
            vy: n(4),
            vx: n(0),
            vx_both: None,
            after_move: false,
            whole: true,
            in_slots: true,
        });
        let mut w = World::power_on(&s);
        w.shots = vec![crate::world::ShotSlot::default(); 2];
        w.shots[1].rest = [Fx(0x10), Fx(0x20), Fx(0x30)];
        w.shot_turn = 1;
        let hero = |w: &World| (w.bodies[0].x.floor(), w.bodies[0].y.floor());
        let (x, y) = hero(&w);
        let mut f = Frame::default();
        w.held = 1;
        shoot_at(&s, &mut w, &mut f, 0, false);
        let first = w.bodies.len() - 1;
        assert_eq!(w.shots[1].body, first as u32 + 1, "slot 1's turn");
        assert_eq!(w.bodies[first].y.raw(), Fx::from_int(y).raw() + 0x20);
        assert_eq!(w.bodies[first].x.raw(), Fx::from_int(x + 4).raw() + 0x10);
        assert_eq!(w.bodies[first].vy.raw(), n(4).raw() + 0x30);
        // Slot 0's turn: taken, so slot 1's comes round with it still busy.
        w.prev = 0;
        shoot_at(&s, &mut w, &mut f, 0, false);
        let second = w.bodies.len() - 1;
        assert_eq!(w.shots[0].body, second as u32 + 1);
        w.prev = 0;
        let bodies = w.bodies.len();
        w.bodies[second].alive = false;
        shoot_at(&s, &mut w, &mut f, 0, false);
        assert_eq!(
            w.bodies.len(),
            bodies,
            "slot 1 is busy; slot 0 being free doesn't help"
        );
        // The canonical state holds the slots and the turn.
        let back = World::from_canonical(&s, &w.canonical()).unwrap();
        assert_eq!(back.shots, w.shots);
        assert_eq!(back.shot_turn, w.shot_turn);
        // Slot 1's shot leaves with its fractions; the next shot gets them.
        w.bodies[first].y = Fx(Fx::from_int(50).raw() + 0x44);
        w.bodies[first].alive = false;
        shoot_at(&s, &mut w, &mut f, 0, false);
        let third = w.bodies.len() - 1;
        assert_eq!(w.bodies[third].y.raw(), Fx::from_int(y).raw() + 0x44);
        assert_eq!(w.shot_turn, 4);
    }
}
