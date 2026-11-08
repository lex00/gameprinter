//! Compiles a model and its lifted assets into a `gp_sim::Spec`: names
//! become indexes, slots become the tables they hold, and every number
//! becomes the fixed-point value the simulation reads. A model the
//! simulation can't run yet (a component field it doesn't implement, an
//! extension it doesn't know) is refused here, by model path, rather than
//! run differently from what it says.

use std::collections::BTreeMap;

use gp_assets::map::{Asset, AssetMap};
use gp_model::model::{Axis, Button};
use gp_model::{Fixed, Model};
use gp_sim::Fx;
use gp_sim::spec::{
    Action, BUILTIN_EVENTS, Camera, Carry, Change, Climb, Cmp, Contact, Counter, Defeat, Do,
    EntityMachine, EntityState, EventKind, Exit, Fall, Fly, Guard, Head, HeadHit, Heading, Health,
    Hits, Hop, Jump, JumpBand, Kind, Knockback, Leap, LeaveView, Level, LevelCamera, Machine,
    Orbit, Oscillate, Path, Patrol, Pickup, Rect, Ride, Role, Run, Shoot, Shuttle, Sides, Spawn,
    Spec, SpringBoost, SpringStep, Springboard, State, System, Tile, Touch, Transition, Walk,
    Walls,
};

use crate::lifted::Lifted;

/// The core schema and extension versions this interpreter runs, as
/// `major.minor`.
pub const CORE: &str = "0.4";
pub const PLATFORMER: &str = "0.4";
pub const TOPDOWN: &str = "0.1";

fn fx(f: Fixed) -> Fx {
    Fx::from_raw(f.raw())
}

/// The controller bit a button is, in the runtime protocol's order.
fn bit(b: Button) -> u8 {
    1 << match b {
        Button::A => 0,
        Button::B => 1,
        Button::Select => 2,
        Button::Start => 3,
        Button::Up => 4,
        Button::Down => 5,
        Button::Left => 6,
        Button::Right => 7,
    }
}

fn major_minor(v: &str) -> &str {
    match v.match_indices('.').nth(1) {
        Some((i, _)) => &v[..i],
        None => v,
    }
}

/// Compiles `model` with its lifted assets. Errors name the model path
/// they're about.
pub fn compile(model: &Model, assets: &Lifted) -> Result<Spec, String> {
    if major_minor(&model.model.schema) != CORE {
        return Err(format!(
            "model.schema: the model is core {}; the interpreter runs core {CORE}.x",
            model.model.schema
        ));
    }
    for (name, version) in &model.model.extensions {
        match name.as_str() {
            "platformer" if major_minor(version) == PLATFORMER => {}
            "topdown" if major_minor(version) == TOPDOWN => {}
            _ => {
                return Err(format!(
                    "model.extensions.{name}: the interpreter runs platformer {PLATFORMER}.x and topdown {TOPDOWN}.x only, not {name} {version}"
                ));
            }
        }
    }
    let actions: Vec<Action> = model
        .input
        .iter()
        .flat_map(|i| &i.actions)
        .map(|(name, b)| Action {
            name: name.clone(),
            mask: bit(*b),
        })
        .collect();
    let action = |at: &str, name: &str| {
        actions
            .iter()
            .position(|a| a.name == name)
            .ok_or(format!("{at}: the input has no action `{name}`"))
    };
    let mut order = Vec::new();
    for (i, name) in model.model.systems.iter().enumerate() {
        let sys = System::from_name(name).ok_or(format!(
            "model.systems[{i}]: the interpreter has no system `{name}`"
        ))?;
        if order.contains(&sys) {
            return Err(format!("model.systems[{i}]: `{name}` is listed twice"));
        }
        order.push(sys);
    }
    // Every system of the core and of each declared extension, once.
    let mut required: Vec<System> = System::CORE.to_vec();
    if model.model.extensions.contains_key("platformer") {
        required.extend(System::PLATFORMER);
    }
    if model.model.extensions.contains_key("topdown") {
        required.extend(System::TOPDOWN);
    }
    if let Some(s) = order.iter().find(|s| !required.contains(s)) {
        return Err(format!(
            "model.systems: `{}` belongs to an extension the model doesn't declare",
            s.name()
        ));
    }
    if let Some(missing) = required.iter().find(|s| !order.contains(s)) {
        return Err(format!(
            "model.systems: `{}` is missing; the order lists every system once",
            missing.name()
        ));
    }
    let kind_names: Vec<&String> = model.entities.keys().collect();
    let kind_of = |at: &str, name: &str| {
        kind_names
            .iter()
            .position(|k| *k == name)
            .ok_or(format!("{at}: there's no entity `{name}`"))
    };

    let counter_names: Vec<&String> = model.counters.keys().collect();
    let mut events: Vec<String> = BUILTIN_EVENTS.iter().map(|e| (*e).to_owned()).collect();
    for c in &counter_names {
        events.push(format!("{c}.reached_min"));
        events.push(format!("{c}.wrapped"));
    }
    let own_events = events.len();
    for e in &model.model.events {
        if events.contains(e) {
            return Err(format!("model.events: `{e}` is already an event"));
        }
        events.push(e.clone());
    }
    let counter_of = |at: &str, name: &str| {
        counter_names
            .iter()
            .position(|c| *c == name)
            .ok_or(format!("{at}: there's no counter `{name}`"))
    };
    let level_names: Vec<&String> = model.levels.keys().collect();
    let level_of = |at: &str, name: &str| {
        level_names
            .iter()
            .position(|l| *l == name)
            .ok_or(format!("{at}: there's no level `{name}`"))
    };

    let event = |at: &str, name: &str| {
        events
            .iter()
            .position(|e| e == name)
            .map(EventKind)
            .ok_or(format!("{at}: the interpreter has no event `{name}`"))
    };

    let roles: Vec<(&String, &str)> = model
        .entities
        .iter()
        .map(|(n, e)| (n, e.core.role.as_str()))
        .collect();
    let refs = Refs {
        action: &action,
        kind_of: &kind_of,
        counter_of: &counter_of,
        event: &event,
        roles: &roles,
    };
    let mut kinds = Vec::new();
    for (name, e) in &model.entities {
        let at = format!("entities.{name}");
        let mut kind = entity_kind(&at, name, e, &refs)?;
        kind.ext_tables = ext_tables(&at, model, assets, &e.core.ext_tables)?;
        kind.ext_tables
            .extend(ext_rows(&at, model, assets, &e.core.ext_rows)?);
        if let Some(m) = &e.core.machine {
            let names: Vec<&String> = m.states.keys().collect();
            let state_of = |at: &str, s: &str| {
                names
                    .iter()
                    .position(|n| *n == s)
                    .ok_or(format!("{at}: there's no state `{s}`"))
            };
            let mut states = Vec::new();
            for (sname, st) in &m.states {
                let sat = format!("{at}.machine.states.{sname}");
                let variant = e
                    .in_state(sname)
                    .map_err(|err| format!("{sat}.set: {err}"))?;
                states.push(EntityState {
                    name: sname.clone(),
                    freeze: st.freeze,
                    part: part(&sat, st, &counter_of)?,
                    kind: Kind {
                        ext_tables: kind.ext_tables.clone(),
                        ..entity_kind(&sat, name, &variant, &refs)?
                    },
                    transitions: transitions(
                        &sat,
                        st,
                        &state_of,
                        &event,
                        &counter_of,
                        &action,
                        Some(&kind_of),
                        &kind_of,
                    )?,
                });
            }
            kind.machine = Some(EntityMachine {
                initial: state_of(&format!("{at}.machine.initial"), &m.initial)?,
                states,
            });
        }
        kinds.push(kind);
    }
    // The kinds a launcher fires take their turns with it (platformer
    // 0.4.27).
    let fired: Vec<usize> = kinds
        .iter()
        .filter_map(|k| k.launcher.map(|l| l.fires))
        .collect();
    for f in fired {
        kinds[f].launched = true;
        if let Some(m) = &mut kinds[f].machine {
            for st in &mut m.states {
                st.kind.launched = true;
            }
        }
    }

    let map = if model.levels.is_empty() {
        None
    } else {
        Some(assets.map()?)
    };
    let mut levels = Vec::new();
    for (name, l) in &model.levels {
        let map = map.as_ref().expect("read when there are levels");
        levels.push(level(
            model,
            assets,
            map,
            name,
            l,
            &kind_of,
            &counter_of,
            &level_of,
            &action,
            &event,
        )?);
    }
    let start_level = match (&model.model.start_level, levels.len()) {
        (Some(name), _) => level_of("model.start_level", name)?,
        (None, 1) => 0,
        (None, n) => {
            return Err(format!(
                "model.start_level: the model has {n} levels, so it must say which comes first"
            ));
        }
    };

    let mut cameras = Vec::new();
    for (name, c) in &model.cameras {
        let at = format!("cameras.{name}");
        cameras.push(Camera {
            name: name.clone(),
            follow: kind_of(&format!("{at}.follow"), &c.follow)?,
            axis_x: matches!(c.axis, Axis::X | Axis::Both),
            axis_y: matches!(c.axis, Axis::Y | Axis::Both),
            size: [i64::from(c.size[0]), i64::from(c.size[1])],
            lead: [fx(c.lead[0]), fx(c.lead[1])],
            clamp_to_level: c.clamp_to_level,
            scroll_back: c.scroll_back,
            by_move: c.by_move.map(|m| gp_sim::spec::ByMove {
                from: fx(m.from),
                full: fx(m.full),
                short_by: fx(m.short_by),
                push_hold: m.push_hold,
                keep_move: m.keep_move,
                hold_with_counters: m.hold_with_counters,
            }),
            keep_in_view: c.keep_in_view,
            keep_in_view_touch: c.keep_in_view_touch,
            keep_in_view_turn: c.keep_in_view_turn,
            keep_in_view_rows: c
                .keep_in_view_rows
                .iter()
                .map(|r| (r.rows, r.bounds))
                .collect(),
        });
    }

    let mut machines = Vec::new();
    for (name, m) in &model.rules {
        let at = format!("rules.{name}");
        let names: Vec<&String> = m.states.keys().collect();
        let state = |at: &str, s: &str| {
            names
                .iter()
                .position(|n| *n == s)
                .ok_or(format!("{at}: there's no state `{s}`"))
        };
        let mut states = Vec::new();
        for (sname, s) in &m.states {
            let transitions = transitions(
                &format!("{at}.states.{sname}"),
                s,
                &state,
                &event,
                &counter_of,
                &action,
                None,
                &kind_of,
            )?;
            if !s.set.is_empty() {
                return Err(format!(
                    "{at}.states.{sname}.set: `set` belongs to an entity's machine"
                ));
            }
            states.push(State {
                name: sname.clone(),
                freeze: s.freeze,
                part: part(&format!("{at}.states.{sname}"), s, &counter_of)?,
                transitions,
            });
        }
        machines.push(Machine {
            name: name.clone(),
            initial: state(&format!("{at}.initial"), &m.initial)?,
            states,
        });
    }

    let mut counters = Vec::new();
    for (name, c) in &model.counters {
        let mut changes = Vec::new();
        for (i, ch) in c.change.iter().enumerate() {
            let at = format!("counters.{name}.change[{i}]");
            if ch.guard.is_some() && ch.on.is_none() {
                return Err(format!("{at}.if: only a change `on` an event takes one"));
            }
            changes.push(match (&ch.on, ch.every) {
                (Some(on), None) if ch.after > 0 || !ch.of.is_empty() || ch.guard.is_some() => {
                    Change::When(gp_sim::spec::OnChange {
                        on: event(&format!("{at}.on"), on)?,
                        by: ch.by,
                        of: ch
                            .of
                            .iter()
                            .map(|k| kind_of(&format!("{at}.of"), k))
                            .collect::<Result<_, _>>()?,
                        after: ch.after,
                        guard: ch
                            .guard
                            .as_ref()
                            .map(|g| guard(&format!("{at}.if"), g, &counter_of))
                            .transpose()?,
                    })
                }
                (Some(on), None) => Change::On(event(&format!("{at}.on"), on)?, ch.by),
                (None, Some(every)) if every > 0 => Change::Every(every, ch.by),
                _ => {
                    return Err(format!(
                        "{at}: give exactly one of `on` and a nonzero `every`"
                    ));
                }
            });
        }
        if c.wrap && c.max.is_none() {
            return Err(format!("counters.{name}.wrap: needs a max"));
        }
        if let Some(max) = c.max
            && max < c.min.unwrap_or(if c.wrap { 0 } else { i64::MIN })
        {
            return Err(format!(
                "counters.{name}.max: below its min (or 0, the bottom of a wrap)"
            ));
        }
        counters.push(Counter {
            name: name.clone(),
            start: c.start,
            changes,
            min: c.min,
            max: c.max,
            wrap: c.wrap,
            runs_when_frozen: c.runs_when_frozen,
            shift: c.shift.as_ref().map(|s| gp_sim::spec::Shift {
                bits: s.bits,
                taps: s.taps.clone(),
            }),
            restarts_clock: c.restarts_clock,
            high_of: None,
        });
    }
    // `high_of` names another counter, known once all are listed.
    for (i, (name, c)) in model.counters.iter().enumerate() {
        if let Some(of) = &c.high_of {
            let j = model
                .counters
                .keys()
                .position(|k| k == of)
                .ok_or(format!("counters.{name}.high_of: no counter named {of}"))?;
            counters[i].high_of = Some(j);
        }
    }

    // Core `input.turn`: the counter and the bits of its actions.
    let turn = match model.input.as_ref().and_then(|i| i.turn.as_ref()) {
        Some(t) => {
            let c = model
                .counters
                .keys()
                .position(|k| *k == t.counter)
                .ok_or(format!(
                    "input.turn.counter: no counter named {}",
                    t.counter
                ))?;
            let mut mask = 0u8;
            for a in &t.actions {
                mask |= actions
                    .iter()
                    .find(|x| x.name == *a)
                    .ok_or(format!("input.turn.actions: no action named {a}"))?
                    .mask;
            }
            Some((c, mask))
        }
        None => None,
    };
    // Core `stages`: the counter, and the level that plays each stage.
    let stages = match &model.stages {
        Some(st) => {
            let c = counter_of("stages.counter", &st.counter)?;
            let stand_in = level_of("stages.stand_in", &st.stand_in)?;
            let plays = st
                .order
                .iter()
                .map(|name| {
                    // Core `stages.skip`: a stage the game passes over.
                    (!st.skip.contains(name))
                        .then(|| level_of("stages.order", name).unwrap_or(stand_in))
                })
                .collect::<Vec<_>>();
            if plays.iter().all(Option::is_none) {
                return Err("stages.skip: skips every stage".to_owned());
            }
            Some((c, plays))
        }
        None => None,
    };
    // Core `cost.frame`.
    let cost = match &model.cost {
        Some(c) => {
            let machine_names: Vec<&String> = model.rules.keys().collect();
            let test = |at: &str,
                        t: &gp_model::model::CostTest,
                        body: bool|
             -> Result<gp_sim::spec::CostTest, String> {
                use gp_sim::spec::CostOn;
                let on = if let Some(n) = &t.counter {
                    let start = match t.at.as_deref() {
                        None | Some("start") => true,
                        Some("end") => false,
                        Some(o) => return Err(format!("{at}.at: `{o}`, not start or end")),
                    };
                    CostOn::Counter(counter_of(at, n)?, start)
                } else if let Some(a) = &t.pad {
                    let mask = actions
                        .iter()
                        .find(|x| x.name == *a)
                        .ok_or(format!("{at}: no action named {a}"))?
                        .mask;
                    CostOn::Pad(mask)
                } else if let Some(a) = t.held.as_ref().or(t.held_before.as_ref()) {
                    let mask = actions
                        .iter()
                        .find(|x| x.name == *a)
                        .ok_or(format!("{at}: no action named {a}"))?
                        .mask;
                    CostOn::Held(mask, t.held.is_some())
                } else if let Some(st) = &t.state {
                    let (m, s) = st
                        .split_once(':')
                        .ok_or(format!("{at}.state: `{st}` isn't machine:state"))?;
                    let mi = machine_names
                        .iter()
                        .position(|k| *k == m)
                        .ok_or(format!("{at}.state: no machine `{m}`"))?;
                    let si = machines[mi]
                        .states
                        .iter()
                        .position(|x: &gp_sim::spec::State| x.name == s)
                        .ok_or(format!("{at}.state: `{m}` has no state `{s}`"))?;
                    CostOn::State(mi, si)
                } else if let Some(w) = t.word.filter(|_| body) {
                    if w > 3 {
                        return Err(format!("{at}.word: {w}, not 0 to 3"));
                    }
                    CostOn::Word(usize::from(w))
                } else {
                    return Err(format!(
                        "{at}: a test reads a counter, held, held_before, state or (per body) word"
                    ));
                };
                Ok(gp_sim::spec::CostTest {
                    on,
                    lo: t.at_least.unwrap_or(i64::MIN),
                    hi: t.below.unwrap_or(i64::MAX),
                })
            };
            let mut terms = Vec::new();
            for (i, t) in c.terms.iter().enumerate() {
                let at = format!("cost.terms[{i}]");
                terms.push(gp_sim::spec::CostTerm {
                    cycles: t.cycles,
                    tests: t
                        .tests
                        .iter()
                        .map(|x| test(&at, x, false))
                        .collect::<Result<_, _>>()?,
                    per: t.per.as_deref().map(|k| kind_of(&at, k)).transpose()?,
                    per_tests: t
                        .per_tests
                        .iter()
                        .map(|x| test(&at, x, true))
                        .collect::<Result<_, _>>()?,
                });
            }
            Some(gp_sim::spec::Cost {
                base: c.base,
                terms,
            })
        }
        None => None,
    };
    Ok(Spec {
        name: model.model.name.clone(),
        actions,
        kinds: kinds.into(),
        levels: levels.into(),
        start_level,
        cameras: cameras.into(),
        machines: machines.into(),
        counters: counters.into(),
        events,
        own_events,
        order,
        seed: 0,
        profile: model.model.profile.clone(),
        keep_direction: model.input.as_ref().is_some_and(|i| i.keep_direction),
        turn,
        stages,
        cost,
    })
}

/// The resolvers an entity's components need.
struct Refs<'a> {
    action: Resolve<'a, usize>,
    kind_of: Resolve<'a, usize>,
    counter_of: Resolve<'a, usize>,
    event: Resolve<'a, EventKind>,
    /// Each entity's name and role, in kind order.
    roles: &'a [(&'a String, &'a str)],
}

/// Turns names into indexes, with an error naming the path.
type Resolve<'a, T> = &'a dyn Fn(&str, &str) -> Result<T, String>;

/// An entity type as the simulation runs it, from an entity's fields (its
/// own, or as one of its states makes it). Its machine is left out.
fn entity_kind(
    at: &str,
    name: &str,
    e: &gp_model::model::Entity,
    refs: &Refs<'_>,
) -> Result<Kind, String> {
    let action = refs.action;
    let p = &e.platformer;
    let run = match &p.run {
        None => None,
        Some(r) => Some(Run {
            left: action(&format!("{at}.run"), "left")?,
            right: action(&format!("{at}.run"), "right")?,
            walk_speed: fx(r.walk_speed),
            run: match (r.run_speed, &r.run_action) {
                (Some(s), Some(a)) => Some((fx(s), action(&format!("{at}.run.run_action"), a)?)),
                (None, None) => None,
                _ => return Err(format!("{at}.run: run_speed and run_action go together")),
            },
            acceleration: r.acceleration.map(fx),
            run_acceleration: r.run_acceleration.map(fx),
            deceleration: r.deceleration.map(fx),
            skid_deceleration: r.skid_deceleration.map(fx),
            air_control: r.air_control,
            start: r.start.as_ref().map(|s| (fx(s.speed), s.frames)),
            air_speed: r.air_speed.map(fx),
            wall_snap: r.wall_snap,
            move_step: r.move_step.map(fx),
            cap_whole: r.cap_whole,
            skid_last_motion: r.skid_by == gp_model::platformer::SkidBy::LastMotion,
            skid_turn_below: r.skid_turn_below.map(fx),
            skid_turn_with: r
                .skid_turn_with
                .iter()
                .enumerate()
                .map(|(i, a)| action(&format!("{at}.run.skid_turn_with[{i}]"), a))
                .collect::<Result<_, _>>()?,
            decel_bands: r
                .decel_bands
                .iter()
                .map(|b| (fx(b.from), fx(b.rate)))
                .collect(),
            decel_hold_from: r.decel_hold_from.map(fx),
            run_hold_frames: r.run_hold_frames,
            run_with_motion: r.run_with_motion,
            takeoff_in_air: r.takeoff_in_air,
            face_on_ground: r.face_on_ground,
            air_run_from: r.air_run_from.map(fx),
            push_masks: r.push_masks,
            speed_before_walls: r.speed_before_walls,
            face_both: r.face_both,
            off: r.off,
            step_cycle: r.step_cycle.iter().map(|n| i64::from(*n)).collect(),
            air_step_cycle: r.air_step_cycle.iter().map(|n| i64::from(*n)).collect(),
            lone_direction: r.lone_direction,
            face_way: r.face_way,
            both_left: r.both_left,
            walls: r.walls.as_ref().map(|wl| {
                let pts = |v: &Vec<[gp_model::Fixed; 2]>| {
                    v.iter().map(|p| [fx(p[0]), fx(p[1])]).collect()
                };
                Walls {
                    left: pts(&wl.left),
                    right: pts(&wl.right),
                }
            }),
        }),
    };
    let jump = match &p.jump {
        None => None,
        Some(j) => Some(Jump {
            action: action(&format!("{at}.jump.action"), &j.action)?,
            speed: fx(j.speed),
            gravity: fx(j.gravity),
            gravity_held: j.gravity_held.map(fx),
            max_fall: fx(j.max_fall),
            max_fall_from_half: j.max_fall_from_half,
            no_gravity_on_ground: j.no_gravity_on_ground,
            land_keeps_fraction: j.land_keeps_fraction,
            keep_fall_gravity: j.keep_fall_gravity,
            start_fall_gravity: j.start_fall_gravity.map(fx),
            land_depth: j.land_depth.map(i64::from),
            feet: j.feet.as_ref().map(|f| f.iter().map(|x| fx(*x)).collect()),
            bands: j
                .bands
                .iter()
                .map(|b| JumpBand {
                    below: fx(b.below),
                    speed: fx(b.speed),
                    gravity: fx(b.gravity),
                    gravity_held: b.gravity_held.map(fx),
                })
                .collect(),
            coyote_frames: j.coyote_frames,
            buffer_frames: j.buffer_frames,
            move_first: j.move_first,
            release_cap: j.release_cap.map(fx),
            collide_y: j.collide_y,
            takeoff_fraction: j.takeoff_fraction.map(fx),
            land_hold: j.land_hold,
            fatal_drop: j.fatal_drop.map(i64::from),
            fatal_drop_in_air: j.fatal_drop_in_air,
            ledge_fall: j.ledge_fall.as_ref().map(|l| {
                let speed = i64::from(l.speed);
                (speed, l.first.map_or(speed, i64::from), l.kills)
            }),
            pickup: match &j.pickup {
                None => None,
                Some(pk) => Some(Pickup {
                    tile: tile_kind(pk.tile),
                    points: pk.points.iter().map(|q| [fx(q[0]), fx(q[1])]).collect(),
                    event: pk
                        .event
                        .as_deref()
                        .map(|e| (refs.event)(&format!("{at}.jump.pickup.event"), e))
                        .transpose()?,
                    becomes: tile_kind(pk.becomes),
                }),
            },
            head: match &j.head {
                None => None,
                Some(h) => Some(Head {
                    point: [fx(h.point[0]), fx(h.point[1])],
                    above: h.above.map(fx),
                    row_from: i64::from(h.row_from),
                    keep_fraction: h.keep_fraction,
                    hits: h
                        .hits
                        .iter()
                        .enumerate()
                        .map(|(i, hit)| {
                            let hat = format!("{at}.jump.head.hits[{i}]");
                            Ok(HeadHit {
                                tile: tile_kind(hit.tile),
                                speed: fx(hit.speed),
                                event: hit
                                    .event
                                    .as_deref()
                                    .map(|e| (refs.event)(&format!("{hat}.event"), e))
                                    .transpose()?,
                                becomes: hit.becomes.map(tile_kind),
                                hold: hit.hold,
                                above: match &hit.above {
                                    None => None,
                                    Some(a) => Some(gp_sim::spec::HeadAbove {
                                        tile: tile_kind(a.tile),
                                        becomes: tile_kind(a.becomes),
                                        event: a
                                            .event
                                            .as_deref()
                                            .map(|e| (refs.event)(&format!("{hat}.above.event"), e))
                                            .transpose()?,
                                    }),
                                },
                            })
                        })
                        .collect::<Result<_, String>>()?,
                    busy_while: h
                        .busy_while
                        .as_ref()
                        .map(|g| guard(&format!("{at}.jump.head.busy_while"), g, refs.counter_of))
                        .transpose()?,
                }),
            },
        }),
    };
    let patrol = match &p.patrol {
        None => None,
        Some(pt) if pt.period == 0 => {
            return Err(format!("{at}.patrol.period: must be at least 1"));
        }
        Some(pt) => Some(Patrol {
            step: fx(pt.step),
            period: pt.period,
        }),
    };
    let contact = p.contact.as_ref().map(|c| Contact {
        stomp: if c.stand {
            None
        } else {
            c.stomp_depth.zip(c.bounce).map(|(d, b)| (fx(d), fx(b)))
        },
        keep_on_stomp: c.keep_on_stomp,
        otherwise: touch(c.otherwise),
        damage: i64::from(c.damage.unwrap_or(1)),
        stand: if c.stand { c.stomp_depth.map(fx) } else { None },
        every: c.every.unwrap_or(1).max(1),
        phase: c.phase,
        touch_edges: c.touch_edges,
        in_turn: c.in_turn,
        after_move: c.after_move,
        sides: c.sides.as_ref().map(|s| Sides {
            left: i64::from(s.left),
            right: i64::from(s.right),
            top: i64::from(s.top),
        }),
        ride: c.ride.as_ref().map(|r| Ride {
            copy_below: i64::from(r.copy_below),
            wrap: i64::from(r.wrap),
            above: i64::from(r.above),
        }),
        near: c.near.as_ref().map(|n| gp_sim::spec::Near {
            within: i64::from(n.within),
            right: n.right.map(i64::from),
        }),
        invincible: match c.invincible {
            Some(t) => touch(t),
            None if !c.stand
                && (c.stomp_depth.is_some()
                    || matches!(
                        c.otherwise,
                        gp_model::platformer::Touch::HurtPlayer
                            | gp_model::platformer::Touch::KillPlayer
                            | gp_model::platformer::Touch::Kick
                    )) =>
            {
                Touch::Defeat
            }
            None => touch(c.otherwise),
        },
    });
    let opt_action = |k: &str, a: &Option<String>| {
        a.as_deref()
            .map(|a| action(&format!("{at}.{k}"), a))
            .transpose()
    };
    let climb = match &p.climb {
        None => None,
        Some(c) => Some(Climb {
            speed: fx(c.speed),
            up: action(&format!("{at}.climb.up"), &c.up)?,
            down: action(&format!("{at}.climb.down"), &c.down)?,
            release: opt_action("climb.release", &c.release)?,
            stand_on_top: c.stand_on_top,
            grab_down: c.grab_down.map_or(Fx::ZERO, fx),
            hold_still: opt_action("climb.hold_still", &c.hold_still)?,
            up_cycle: c.up_cycle.iter().map(|&d| i64::from(d)).collect(),
            down_cycle: c.down_cycle.iter().map(|&d| i64::from(d)).collect(),
            by_feet: c.by_feet,
            down_holds: c.down_holds,
            down_through: c.down_through.map_or(0, i64::from),
            grab: c.grab.map(|g| g.map(i64::from)),
            grab_snap: c.grab_snap.map(i64::from),
            down_through_step: c.down_through_step,
            top_test: c.top_test,
            lone_way: c.lone_way,
        }),
    };
    let health = match &p.health {
        None => None,
        Some(h) => Some(Health {
            counter: h
                .counter
                .as_deref()
                .map(|c| (refs.counter_of)(&format!("{at}.health.counter"), c))
                .transpose()?,
            points: i64::from(h.points.unwrap_or(0)),
            invulnerable: h.invulnerable,
            shield: h.shield,
            damage_from: h
                .damage_from
                .iter()
                .map(|(k, d)| {
                    Ok((
                        (refs.kind_of)(&format!("{at}.health.damage_from"), k)?,
                        i64::from(*d),
                    ))
                })
                .collect::<Result<_, String>>()?,
            defeat: h
                .defeat
                .as_ref()
                .map(|d| {
                    Ok::<_, String>(Defeat {
                        becomes: (refs.kind_of)(&format!("{at}.health.defeat"), &d.becomes)?,
                        rise: fx(d.rise),
                        speed: fx(d.speed),
                    })
                })
                .transpose()?,
        }),
    };
    let hits = p.hits.as_ref().map(|h| Hits {
        damage: i64::from(h.damage),
        targets: refs
            .roles
            .iter()
            .enumerate()
            .filter(|(_, (_, r))| h.roles.iter().any(|x| x == r))
            .map(|(i, _)| i)
            .collect(),
        keep: h.keep,
        every: h.every.unwrap_or(1).max(1),
        phase: h.phase,
        move_first: h.move_first,
    });
    let shoot = match &p.shoot {
        None => None,
        Some(sh) => Some(Shoot {
            entity: (refs.kind_of)(&format!("{at}.shoot.entity"), &sh.entity)?,
            action: opt_action("shoot.action", &sh.action)?,
            every: sh.every.unwrap_or(0),
            ahead: [fx(sh.ahead[0]), fx(sh.ahead[1])],
            max: sh.max,
            unmirrored: sh.unmirrored,
            vy: sh.vy.map_or(Fx::ZERO, fx),
            vx: sh.vx.map_or(Fx::ZERO, fx),
            vx_both: sh.vx_both.map(fx),
            after_move: sh.after_move,
            whole: sh.whole,
            in_slots: sh.in_slots,
        }),
    };
    let carry = match &p.carry {
        None => None,
        Some(c) => Some(Carry {
            action: action(&format!("{at}.carry.action"), &c.action)?,
            lift_frames: c.lift_frames,
            offset: [fx(c.offset[0]), fx(c.offset[1])],
            throw: [fx(c.throw[0]), fx(c.throw[1])],
            throw_moving: c
                .throw_moving
                .zip(c.moving_above)
                .map(|(t, a)| ([fx(t[0]), fx(t[1])], fx(a))),
        }),
    };
    let rect = |h: gp_model::model::Rect| Rect {
        x: fx(h.x),
        y: fx(h.y),
        w: fx(h.w),
        h: fx(h.h),
    };
    Ok(Kind {
        name: name.to_owned(),
        role: match e.core.role.as_str() {
            "player" => Role::Player,
            _ => Role::Other,
        },
        size: [fx(e.core.size[0]), fx(e.core.size[1])],
        hitbox: rect(e.core.hitbox),
        tile_hitbox: rect(e.core.tile_hitbox.unwrap_or(e.core.hitbox)),
        tile_collision: e.core.tile_collision,
        run,
        jump,
        patrol,
        contact,
        walk: p.walk.as_ref().map(|w| Walk {
            speed: fx(w.speed),
            turn_at_ledges: w.turn_at_ledges,
            walls_at_top: w.walls_at_top,
            turn_after_move: w.turn_after_move,
            ledge_at: w.ledge_at.map(fx),
            bump: w.bump.as_ref().map(|b| gp_sim::spec::Bump {
                every: b.every,
                phase: b.phase,
                view: b.view,
            }),
        }),
        fall: p.fall.as_ref().map(|f| Fall {
            gravity: fx(f.gravity),
            max_fall: fx(f.max_fall),
            max_fall_from_half: f.max_fall_from_half,
            move_first: f.move_first,
            feet: f.feet.as_ref().map(|f| f.iter().map(|x| fx(*x)).collect()),
            stands_still: f.stands_still,
        }),
        fly: p.fly.as_ref().map(|f| {
            use gp_model::platformer::Heading as H;
            Fly {
                speed: fx(f.speed),
                heading: match f.heading {
                    H::Player => Heading::Player,
                    H::Left => Heading::Left,
                    H::Right => Heading::Right,
                },
                keep_speed: f.keep_speed,
            }
        }),
        hop: p.hop.as_ref().map(|h| Hop {
            speed: fx(h.speed),
            pattern: h.pattern.iter().map(|s| fx(*s)).collect(),
            every: h.every.unwrap_or(0),
            phase: h.phase,
            land_first: h.land_first,
            land_while_stopped: h.land_while_stopped,
        }),
        oscillate: p.oscillate.as_ref().map(|o| Oscillate {
            vertical: o.axis == gp_model::platformer::PathAxis::Y,
            center: fx(o.center),
            force: fx(o.force),
            max_speed: fx(o.max_speed),
            low: o.low_at.zip(o.low_center).map(|(a, c)| (fx(a), fx(c))),
            starts_at_rest: o.starts_at_rest,
            move_first: o.move_first,
            rest_creep_every: o.rest_creep_every,
        }),
        leap: p
            .leap
            .as_ref()
            .map(|l| {
                Ok::<_, String>(Leap {
                    speed: fx(l.speed),
                    gravity: fx(l.gravity),
                    max_fall: l.max_fall.map(fx),
                    every: l.every,
                    from: l.from.map(fx),
                    rise: l.rise.as_ref().map(|r| (fx(r.speed), fx(r.height))),
                    interval: l.interval,
                    waits: l.waits.clone(),
                    below: l.below.map(fx),
                    move_first: l.move_first,
                    max_fall_from_half: l.max_fall_from_half,
                    beat: l
                        .beat
                        .as_ref()
                        .map(|c| (refs.counter_of)(&format!("{at}.leap.beat"), c))
                        .transpose()?,
                    keep_fraction: l.keep_fraction,
                    place_on_appear: l.place_on_appear,
                    draw: l
                        .draw
                        .as_ref()
                        .map(|d| {
                            Ok::<_, String>(gp_sim::spec::LeapDraw {
                                counter: (refs.counter_of)(
                                    &format!("{at}.leap.draw.counter"),
                                    &d.counter,
                                )?,
                                at: d.at,
                                slot_step: d.slot_step,
                                fraction: d.fraction,
                            })
                        })
                        .transpose()?,
                })
            })
            .transpose()?,
        launcher: p
            .launcher
            .as_ref()
            .map(|l| {
                Ok::<_, String>(gp_sim::spec::Launcher {
                    fires: (refs.kind_of)(&format!("{at}.launcher.fires"), &l.fires)?,
                    counter: (refs.counter_of)(&format!("{at}.launcher.counter"), &l.counter)?,
                    at: l.at,
                    slot_step: l.slot_step,
                    mask: l.mask,
                    hard_mask: l.hard_mask,
                    slots: l.slots,
                    table: l.table,
                    next: (refs.counter_of)(&format!("{at}.launcher.next"), &l.next)?,
                    rearm: l.rearm,
                    offset: [fx(l.offset[0]), fx(l.offset[1])],
                    from_x: Fx::from_int(i64::from(l.from_x)),
                    near: l.near,
                    near_wrap: l.near_wrap,
                    behind: l.behind,
                })
            })
            .transpose()?,
        launched: false,
        shuttle: p.shuttle.as_ref().map(|sh| Shuttle {
            vertical: sh.axis == gp_model::platformer::PathAxis::Y,
            step: fx(sh.step),
            every: sh.every,
            turn_after: sh.turn_after,
            sway: sh.sway.as_ref().map(|w| (fx(w.step), w.half)),
        }),
        orbit: match &p.orbit {
            None => None,
            Some(o) => Some(Orbit {
                spin: fx(o.spin),
                angles: o.angles,
                parts: o.parts,
                part: (refs.kind_of)(&format!("{at}.orbit.part"), &o.part)?,
                place: [fx(o.place[0]), fx(o.place[1])],
                skip: o.skip.clone(),
                in_view: o.in_view,
                place_on_appear: o.place_on_appear,
                angle_in_slot: o.angle_in_slot,
            }),
        },
        orbit_part: p.orbit_part.as_ref().map(|o| fx(o.within)),
        springboard: match &p.springboard {
            None => None,
            Some(sb) => Some(Springboard {
                steps: sb
                    .steps
                    .iter()
                    .map(|st| SpringStep {
                        frames: st.frames,
                        moves: fx(st.moves),
                        offset: fx(st.offset),
                    })
                    .collect(),
                speed: fx(sb.speed),
                boost: match &sb.boost {
                    None => None,
                    Some(b) => Some(SpringBoost {
                        speed: fx(b.speed),
                        action: action(&format!("{at}.springboard.boost.action"), &b.action)?,
                        from_step: b.from_step,
                        held: b.held,
                    }),
                },
                throw_late: sb.throw_late,
            }),
        },
        stream: match &p.stream {
            None => None,
            Some(s) => Some(gp_sim::spec::Stream {
                entity: (refs.kind_of)(&format!("{at}.stream.entity"), &s.entity)?,
                past_edge: fx(s.past_edge),
                wait: s.wait,
                draw: gp_sim::spec::LeapDraw {
                    counter: (refs.counter_of)(
                        &format!("{at}.stream.draw.counter"),
                        &s.draw.counter,
                    )?,
                    at: s.draw.at,
                    slot_step: s.draw.slot_step,
                    fraction: s.draw.fraction,
                },
                until: s
                    .until
                    .as_ref()
                    .map(|u| (refs.kind_of)(&format!("{at}.stream.until"), u))
                    .transpose()?,
            }),
        },
        leave_view: p.leave_view.as_ref().map(|l| LeaveView {
            vertical: l.axis == gp_model::platformer::PathAxis::Y,
            margin: fx(l.margin),
            far_margin: l.far_margin.map(fx),
            bottom_margin: l.bottom_margin.map(fx),
        }),
        path: p.path.as_ref().map(|pa| Path {
            vertical: pa.axis == gp_model::platformer::PathAxis::Y,
            distance: fx(pa.distance),
            speed: fx(pa.speed),
            pause: pa.pause,
            wait_near: pa.wait_near.map(fx),
            wait_near_wrap: pa.wait_near_wrap,
            every: pa.every,
            phase: pa.phase,
        }),
        climb,
        health,
        knockback: p.knockback.as_ref().map(|k| Knockback {
            speed: fx(k.speed),
            rise: fx(k.rise),
            frames: k.frames,
        }),
        hits,
        shoot,
        carry,
        carryable: p
            .carryable
            .as_ref()
            .map(|c| c.offset.map(|o| [fx(o[0]), fx(o[1])])),
        move8: match &e.topdown.move8 {
            None => None,
            Some(m) => {
                let [l, r, u, d] = m.action_names();
                let at = format!("{at}.move8");
                Some(gp_sim::spec::Move8 {
                    speed: fx(m.speed),
                    left: action(&at, l)?,
                    right: action(&at, r)?,
                    up: action(&at, u)?,
                    down: action(&at, d)?,
                })
            }
        },
        machine: None,
        ext: e.core.ext.clone(),
        ext_params: e
            .core
            .ext_params
            .iter()
            .map(|(k, v)| (k.clone(), fx(*v)))
            .collect(),
        ext_tables: Default::default(),
        no_slot: e.core.no_slot,
        hook_through_stops: e.core.hook_through_stops,
        hook_frame_start: e.core.hook_frame_start,
        hook_first: e.core.hook_first,
        hook_last: e.core.hook_last,
        extra_slots: e.core.extra_slots,
        despawn_behind: e.core.despawn_behind.map(fx),
        despawn_behind_wrap: e.core.despawn_behind_wrap,
        despawn_ahead: e.core.despawn_ahead.map(fx),
        invincible: p
            .invincible
            .as_ref()
            .map(|i| {
                guard(
                    &format!("{at}.invincible.while"),
                    &i.while_,
                    refs.counter_of,
                )
            })
            .transpose()?,
        locks: p
            .locks
            .as_ref()
            .map(|l| -> Result<_, String> {
                let g = guard(&format!("{at}.locks.while"), &l.while_, refs.counter_of)?;
                let acts = l
                    .actions
                    .iter()
                    .map(|a| action(&format!("{at}.locks.actions"), a))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((g, acts))
            })
            .transpose()?,
        bounce: p.bounce.as_ref().map(|b| gp_sim::spec::Bounce {
            point: [fx(b.point[0]), fx(b.point[1])],
            speed: fx(b.speed),
            snap: b.snap,
            from_y: b.from_y.map(fx),
        }),
        touch_points: p.touch_points.iter().map(|t| t.map(i64::from)).collect(),
    })
}

fn touch(t: gp_model::platformer::Touch) -> Touch {
    use gp_model::platformer::Touch as T;
    match t {
        T::KillPlayer => Touch::KillPlayer,
        T::HurtPlayer => Touch::HurtPlayer,
        T::Collect => Touch::Collect,
        T::Kick => Touch::Kick,
        T::Nothing => Touch::Nothing,
        T::Defeat => Touch::Defeat,
    }
}

pub(crate) fn tile_kind(k: gp_model::model::TileKind) -> Tile {
    use gp_model::model::TileKind as K;
    match k {
        K::Empty => Tile::Empty,
        K::Solid => Tile::Solid,
        K::Top => Tile::Top,
        K::Ladder => Tile::Ladder,
        K::Hazard => Tile::Hazard,
        K::Water => Tile::Water,
        K::Ice => Tile::Ice,
        K::CoinBlock => Tile::CoinBlock,
        K::ItemBlock => Tile::ItemBlock,
        K::Brick => Tile::Brick,
        K::Coin => Tile::Coin,
    }
}

/// A level's `tile_kinds`, by value.
fn kinds_by_value(at: &str, c: &gp_model::model::LevelCore) -> Result<BTreeMap<u32, Tile>, String> {
    c.tile_kinds
        .iter()
        .map(|(k, v)| {
            k.parse::<u32>()
                .map(|n| (n, tile_kind(*v)))
                .map_err(|_| format!("{at}.tile_kinds: `{k}` isn't a whole number"))
        })
        .collect()
}

/// An entity's `ext_rows` (core `entity.ext_rows`), each a `rows` asset
/// read from its file: every row's numbers, in order, as one table.
fn ext_rows(
    at: &str,
    model: &Model,
    assets: &Lifted,
    rows: &std::collections::BTreeMap<String, String>,
) -> Result<std::collections::BTreeMap<String, Vec<i64>>, String> {
    let mut out = std::collections::BTreeMap::new();
    if rows.is_empty() {
        return Ok(out);
    }
    let map = assets.map()?;
    for (name, slot) in rows {
        let here = format!("{at}.ext_rows.{name}");
        let asset = slot_asset(model, &map, &here, slot)?;
        let file = asset
            .files()
            .into_iter()
            .next()
            .ok_or(format!("{here}: asset `{}` has no file", asset.name()))?;
        let text = assets
            .files
            .get(&file)
            .ok_or(format!("{here}: the lifted assets have no {file}"))?;
        let doc: toml::Table = toml::from_str(text).map_err(|e| format!("{here}: {file}: {e}"))?;
        let rows = doc
            .get("rows")
            .and_then(|r| r.as_array())
            .ok_or(format!("{here}: {file} has no rows"))?;
        let mut values = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            let r = r
                .as_array()
                .ok_or(format!("{here}: {file} row {i} isn't a list"))?;
            for v in r {
                values.push(v.as_integer().ok_or(format!(
                    "{here}: {file} row {i} holds a value that isn't a number"
                ))?);
            }
        }
        out.insert(name.clone(), values);
    }
    Ok(out)
}

/// An entity's `ext_tables`, each a column of a lifted table, read from
/// the table's file: one number a row.
fn ext_tables(
    at: &str,
    model: &Model,
    assets: &Lifted,
    tables: &std::collections::BTreeMap<String, String>,
) -> Result<std::collections::BTreeMap<String, Vec<i64>>, String> {
    let mut out = std::collections::BTreeMap::new();
    if tables.is_empty() {
        return Ok(out);
    }
    let map = assets.map()?;
    for (name, of) in tables {
        let here = format!("{at}.ext_tables.{name}");
        out.insert(name.clone(), table_column(&here, model, assets, &map, of)?);
    }
    Ok(out)
}

/// A lifted table's column, `slot.column`, read from the table's file: one
/// number a row.
fn table_column(
    here: &str,
    model: &Model,
    assets: &Lifted,
    map: &AssetMap,
    of: &str,
) -> Result<Vec<i64>, String> {
    let (slot, column) = of
        .split_once('.')
        .ok_or(format!("{here}: `{of}` isn't `<slot>.<column>`"))?;
    let asset = slot_asset(model, map, here, slot)?;
    let file = asset
        .files()
        .into_iter()
        .next()
        .ok_or(format!("{here}: asset `{}` has no file", asset.name()))?;
    let text = assets
        .files
        .get(&file)
        .ok_or(format!("{here}: the lifted assets have no {file}"))?;
    let doc: toml::Table = toml::from_str(text).map_err(|e| format!("{here}: {file}: {e}"))?;
    let rows = doc
        .get("row")
        .and_then(|r| r.as_array())
        .ok_or(format!("{here}: {file} has no rows"))?;
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            r.get(column)
                .and_then(|v| v.as_integer())
                .ok_or(format!("{here}: {file} row {i} has no number `{column}`"))
        })
        .collect()
}

/// A state's transitions. `remove` and `become:<entity>` are allowed in an
/// entity's machine only, which gives `kind_of` to resolve the entity.
// The resolvers are one lookup each and the function has two callers;
// a struct to carry them would be built twice and read once.
#[allow(clippy::too_many_arguments)]
fn transitions(
    at: &str,
    s: &gp_model::model::State,
    state_of: Resolve<'_, usize>,
    event: Resolve<'_, EventKind>,
    counter_of: Resolve<'_, usize>,
    action_of: Resolve<'_, usize>,
    kind_of: Option<Resolve<'_, usize>>,
    clear_of: Resolve<'_, usize>,
) -> Result<Vec<Transition>, String> {
    let entity = kind_of.is_some();
    let mut out = Vec::new();
    for (i, t) in s.transition.iter().enumerate() {
        let tat = format!("{at}.transition[{i}]");
        if t.below.is_some() {
            if !entity || t.on.is_some() {
                return Err(format!(
                    "{tat}.below: belongs to an entity's machine, without `on`"
                ));
            }
        } else if t.pressed.is_some() || t.held.is_some() {
            let key = if t.pressed.is_some() {
                "pressed"
            } else {
                "held"
            };
            if t.on.is_some() {
                return Err(format!("{tat}.{key}: goes without `on`"));
            }
            if t.pressed.is_some() && t.held.is_some() {
                return Err(format!("{tat}: give `pressed` or `held`, not both"));
            }
        } else if t.on.is_some() == t.after.is_some() {
            return Err(format!("{tat}: give exactly one of `on` and `after`"));
        }
        if t.word.is_some() && (!entity || t.after.is_none()) {
            return Err(format!(
                "{tat}.word: belongs to an entity's machine, with `after`"
            ));
        }
        if t.word.as_ref().is_some_and(|wt| wt.index > 3) {
            return Err(format!("{tat}.word.index: a mechanic word, 0 to 3"));
        }
        let mut actions = Vec::new();
        for a in &t.actions {
            if a.starts_with(gp_model::validate::CHANGE) {
                let (c, by) = gp_model::validate::change_action(a)
                    .ok_or(format!("{tat}.do: `{a}` isn't `change:<counter>:<by>`"))?;
                actions.push(Do::Change(counter_of(&format!("{tat}.do"), c)?, by));
                continue;
            }
            if let Some(e) = a.strip_prefix(gp_model::validate::CLEAR) {
                actions.push(Do::Clear(clear_of(&format!("{tat}.do"), e)?));
                continue;
            }
            if let Some(e) = a.strip_prefix(gp_model::validate::RESPAWN) {
                actions.push(Do::Respawn(clear_of(&format!("{tat}.do"), e)?));
                continue;
            }
            if let Some(c) = a.strip_prefix(gp_model::validate::REMOVE_BODY) {
                actions.push(Do::RemoveBody(counter_of(&format!("{tat}.do"), c)?));
                continue;
            }
            if a.starts_with(gp_model::validate::SET) {
                let (c, v) = gp_model::validate::set_action(a)
                    .ok_or(format!("{tat}.do: `{a}` isn't `set:<counter>:<value>`"))?;
                actions.push(Do::Set(counter_of(&format!("{tat}.do"), c)?, v));
                continue;
            }
            if a.starts_with(gp_model::validate::ADD) {
                let (to, from) = gp_model::validate::add_action(a)
                    .ok_or(format!("{tat}.do: `{a}` isn't `add:<counter>:<counter>`"))?;
                let at = format!("{tat}.do");
                actions.push(Do::Add(counter_of(&at, to)?, counter_of(&at, from)?));
                continue;
            }
            if let Some(e) = a.strip_prefix(gp_model::validate::EMIT) {
                actions.push(Do::Emit(event(&format!("{tat}.do"), e)?));
                continue;
            }
            if let Some(e) = a.strip_prefix("become:") {
                let Some(kind_of) = kind_of else {
                    return Err(format!("{tat}.do: `{a}` belongs to an entity's machine"));
                };
                actions.push(Do::Become(kind_of(&format!("{tat}.do"), e)?));
                continue;
            }
            match Do::from_name(a) {
                Some(Do::Remove) if !entity => {
                    return Err(format!("{tat}.do: `remove` belongs to an entity's machine"));
                }
                Some(d) => actions.push(d),
                None => return Err(format!("{tat}.do: the interpreter has no action `{a}`")),
            }
        }
        out.push(Transition {
            to: state_of(&format!("{tat}.to"), &t.to)?,
            on: t
                .on
                .as_deref()
                .map(|e| event(&format!("{tat}.on"), e))
                .transpose()?,
            after: t.after,
            align: t.align,
            align_to: t
                .align_to
                .as_deref()
                .map(|c| counter_of(&format!("{tat}.align_to"), c))
                .transpose()?,
            below: t.below.map(fx),
            word: t
                .word
                .as_ref()
                .map(|wt| (usize::from(wt.index), wt.at_least, wt.below)),
            pressed: t
                .pressed
                .as_deref()
                .map(|a| action_of(&format!("{tat}.pressed"), a))
                .transpose()?,
            held: t
                .held
                .as_deref()
                .map(|a| action_of(&format!("{tat}.held"), a))
                .transpose()?,
            guard: t
                .guard
                .as_ref()
                .map(|g| guard(&format!("{tat}.if"), g, counter_of))
                .transpose()?,
            actions,
        });
    }
    Ok(out)
}

/// A state's part freeze and time (core 0.4.12).
fn part(
    at: &str,
    s: &gp_model::model::State,
    counter_of: Resolve<'_, usize>,
) -> Result<gp_sim::spec::Part, String> {
    let system = |key: &str, name: &String| {
        gp_sim::spec::System::from_name(name)
            .filter(|s| s.moves())
            .ok_or(format!(
                "{at}.{key}: `{name}` isn't a system that moves things"
            ))
    };
    Ok(gp_sim::spec::Part {
        stops: s
            .stops
            .iter()
            .map(|n| system("stops", n))
            .collect::<Result<_, _>>()?,
        stops_at_once: s
            .stops_at_once
            .iter()
            .map(|n| system("stops_at_once", n))
            .collect::<Result<_, _>>()?,
        holds: s
            .holds
            .iter()
            .map(|n| counter_of(&format!("{at}.holds"), n))
            .collect::<Result<_, _>>()?,
        keep_time: s.keep_time,
        stops_machines: s.stops_machines,
        frees_at_once: s
            .frees_at_once
            .iter()
            .map(|n| system("frees_at_once", n))
            .collect::<Result<_, _>>()?,
        seen_every: s.seen_every.unwrap_or(0),
        ground: s.ground.map(|g| match g {
            gp_model::model::GroundOnEnter::Leave => gp_sim::spec::Ground::Leave,
            gp_model::model::GroundOnEnter::Land => gp_sim::spec::Ground::Land,
        }),
        enter_speed: s.enter_speed.map(|[vx, vy]| [fx(vx), fx(vy)]),
        enter_keeps_fraction: s.enter_keeps_fraction,
        step_word: s.step_word.map_or(0, i64::from),
    })
}

/// A counter condition.
fn guard(
    at: &str,
    g: &gp_model::model::Guard,
    counter_of: Resolve<'_, usize>,
) -> Result<Guard, String> {
    use gp_model::model::Cmp as C;
    Ok(Guard {
        counter: counter_of(&format!("{at}.counter"), &g.counter)?,
        cmp: match g.cmp {
            C::Lt => Cmp::Lt,
            C::Le => Cmp::Le,
            C::Eq => Cmp::Eq,
            C::Ne => Cmp::Ne,
            C::Ge => Cmp::Ge,
            C::Gt => Cmp::Gt,
        },
        value: g.value,
    })
}

/// The lifted asset a slot is filled from.
fn slot_asset<'a>(
    model: &Model,
    map: &'a AssetMap,
    at: &str,
    slot: &str,
) -> Result<&'a Asset, String> {
    let s = model
        .assets
        .get(slot)
        .ok_or(format!("{at}: there's no asset slot `{slot}`"))?;
    let from = s
        .from
        .as_ref()
        .ok_or(format!("{at}: slot `{slot}` has no `from`"))?;
    map.assets
        .iter()
        .find(|a| a.name() == from.asset)
        .ok_or(format!(
            "{at}: slot `{slot}` names asset `{}`, which the lifted assets don't have",
            from.asset
        ))
}

/// A lifted `byte-rows` slot's rows.
fn slot_rows(
    at: &str,
    model: &Model,
    assets: &Lifted,
    map: &AssetMap,
    slot: &str,
) -> Result<Vec<Vec<i64>>, String> {
    let asset = slot_asset(model, map, at, slot)?;
    let file = asset
        .files()
        .into_iter()
        .next()
        .ok_or(format!("{at}: asset `{}` has no file", asset.name()))?;
    let text = assets
        .files
        .get(&file)
        .ok_or(format!("{at}: the lifted assets have no {file}"))?;
    let doc: toml::Table = toml::from_str(text).map_err(|e| format!("{at}: {file}: {e}"))?;
    let rows = doc
        .get("rows")
        .and_then(|r| r.as_array())
        .ok_or(format!("{at}: {file} has no rows"))?;
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            r.as_array()
                .ok_or(format!("{at}: {file} row {i} isn't a list of bytes"))?
                .iter()
                .map(|v| {
                    v.as_integer()
                        .ok_or(format!("{at}: {file} row {i} holds something not a number"))
                })
                .collect()
        })
        .collect()
}

/// Platformer `level.ladders` (and `ladder_tops`, `ladder_stops`, the
/// level's `field`): each box in level pixels.
fn ladder_boxes(
    at: &str,
    field: &str,
    model: &Model,
    assets: &Lifted,
    map: &AssetMap,
    l: &gp_model::platformer::Ladders,
) -> Result<Vec<[i64; 4]>, String> {
    let here = format!("{at}.{field}");
    // Feature `level.ladder_lists`: with `places_at`, the game's own list in
    // a byte table's column, rows of 3 up to the end mark; with
    // `shapes_at`, rows of 4 from there.
    let places = match l.places_at {
        None => slot_rows(&format!("{here}.places"), model, assets, map, &l.places)?,
        Some(from) => list_rows(
            &table_column(&format!("{here}.places"), model, assets, map, &l.places)?,
            from,
            3,
            Some(i64::from(l.end.unwrap_or(0xFE))),
        ),
    };
    let shapes = match l.shapes_at {
        None => slot_rows(&format!("{here}.shapes"), model, assets, map, &l.shapes)?,
        Some(from) => list_rows(
            &table_column(&format!("{here}.shapes"), model, assets, map, &l.shapes)?,
            from,
            4,
            None,
        ),
    };
    let step = i64::from(l.shape_step.max(1));
    places
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let &[x, y, shape, ..] = &p[..] else {
                return Err(format!("{here}.places: row {i} has fewer than 3 bytes"));
            };
            let s = shapes
                .get(usize::try_from(shape / step).unwrap_or(usize::MAX))
                .filter(|s| s.len() >= 4)
                .ok_or(format!(
                    "{here}.places: row {i}'s shape {shape} isn't a row of 4 in shapes"
                ))?;
            Ok([x + s[0], y + s[1], x + s[2], y + s[3]])
        })
        .collect()
}

/// Feature `level.ladder_lists`: rows of `width` from index `from` of a
/// byte table's column, up to a row that starts with `end` (or the
/// column's end).
fn list_rows(col: &[i64], from: u32, width: usize, end: Option<i64>) -> Vec<Vec<i64>> {
    col.get(from as usize..)
        .unwrap_or_default()
        .chunks(width)
        .take_while(|r| end.is_none_or(|e| r.first() != Some(&e)))
        .map(<[i64]>::to_vec)
        .collect()
}

/// Platformer `level.ground`: each run's boxes in level pixels, each box
/// `rise` pixels above the last, x and y wrapping as bytes.
fn ground_boxes(
    at: &str,
    model: &Model,
    assets: &Lifted,
    map: &AssetMap,
    g: &gp_model::platformer::Ground,
) -> Result<Vec<[i64; 4]>, String> {
    let here = format!("{at}.ground");
    let runs = slot_rows(&format!("{here}.runs"), model, assets, map, &g.runs)?;
    let shapes = slot_rows(&format!("{here}.shapes"), model, assets, map, &g.shapes)?;
    let step = i64::from(g.shape_step.max(1));
    let mut out = Vec::new();
    for (i, r) in runs.iter().enumerate() {
        // Platformer `level.ground_boxes`: a row is one box, x, y and shape.
        let (x, y, shape, across, count) = match (&r[..], g.boxes) {
            (&[x, y, shape, ..], true) => (x, y, shape, 0, 1),
            (&[x, y, shape, across, count, ..], false) => (x, y, shape, across, count),
            _ => {
                let need = if g.boxes { 3 } else { 5 };
                return Err(format!("{here}.runs: row {i} has fewer than {need} bytes"));
            }
        };
        let s = shapes
            .get(usize::try_from(shape / step).unwrap_or(usize::MAX))
            .filter(|s| s.len() >= 4)
            .ok_or(format!(
                "{here}.runs: row {i}'s shape {shape} isn't a row of 4 in shapes"
            ))?;
        // The step across is a signed byte.
        let across = if across >= 128 { across - 256 } else { across };
        for k in 0..count {
            let px = (x + k * across).rem_euclid(256);
            let py = (y - k * i64::from(g.rise)).rem_euclid(256);
            out.push([px + s[0], py + s[1], px + s[2], py + s[3]]);
        }
    }
    Ok(out)
}

/// Platformer `level.pickups`: a group's places from its byte table, its
/// counters and its event, and with `level.pickup_cells` each place's
/// cells in a level of `size` cells.
#[allow(clippy::too_many_arguments)]
fn pickup_group(
    at: &str,
    model: &Model,
    assets: &Lifted,
    map: &AssetMap,
    p: &gp_model::platformer::Pickups,
    size: [u32; 2],
    counter_of: &dyn Fn(&str, &str) -> Result<usize, String>,
    event: &dyn Fn(&str, &str) -> Result<EventKind, String>,
) -> Result<gp_sim::spec::PickupGroup, String> {
    let table = table_column(&format!("{at}.table"), model, assets, map, &p.table)?;
    let byte = |i: u32| -> Result<i64, String> {
        table.get(i as usize).copied().ok_or(format!(
            "{at}: index {i} is past the end of {} ({} bytes)",
            p.table,
            table.len()
        ))
    };
    let mut places = Vec::new();
    let mut counters = Vec::new();
    let mut cells = Vec::new();
    for k in 0..p.count {
        places.push([byte(p.x_at + k)?, byte(p.y_at + k)?]);
        let name = format!("{}_{k}", p.counter);
        counters.push(counter_of(&format!("{at}.counter"), &name)?);
        // Platformer `level.pickup_cells`: the place's cells.
        if let Some(c) = &p.cells {
            let first = (byte(c.at[0] + k)? << 8 | byte(c.at[1] + k)?) - i64::from(c.base);
            let w = i64::from(size[0].max(1));
            let (col, row) = (first.rem_euclid(w), first.div_euclid(w));
            let mut own = Vec::new();
            for dy in 0..i64::from(c.size[1]) {
                for dx in 0..i64::from(c.size[0]) {
                    // A cell outside the level (a place no one can take,
                    // as 75m's third prize at 0, 0) covers nothing.
                    let (x, y) = (col + dx, row + dy);
                    if first >= 0 && x < w && y < i64::from(size[1]) {
                        own.push((y * w + x) as u32);
                    }
                }
            }
            cells.push(own);
        }
    }
    Ok(gp_sim::spec::PickupGroup {
        places,
        counters,
        offset: p.offset.map(i64::from),
        window: p.window.map(i64::from),
        event: p
            .event
            .as_deref()
            .map(|e| event(&format!("{at}.event"), e))
            .transpose()?,
        hole: p.hole.map(|h| h.map(i64::from)),
        cells,
        by_hook: p.by_hook,
    })
}

/// One level's tiles, spawns and fields. The resolvers turn names into
/// indexes, with an error naming the path.
#[allow(clippy::too_many_arguments)]
fn level(
    model: &Model,
    assets: &Lifted,
    map: &AssetMap,
    name: &str,
    l: &gp_model::model::Level,
    kind_of: &dyn Fn(&str, &str) -> Result<usize, String>,
    counter_of: &dyn Fn(&str, &str) -> Result<usize, String>,
    level_of: &dyn Fn(&str, &str) -> Result<usize, String>,
    action: &dyn Fn(&str, &str) -> Result<usize, String>,
    event: &dyn Fn(&str, &str) -> Result<EventKind, String>,
) -> Result<Level, String> {
    let at = format!("levels.{name}");
    let c = &l.core;
    if c.tile_size == 0 || c.size[0] == 0 || c.size[1] == 0 {
        return Err(format!("{at}: tile_size and size must be above zero"));
    }
    let by_value = kinds_by_value(&at, c)?;
    // Core `level.layout_ppu_strings`: a level drawn from a PPU update
    // string. Its cells are the drawn tile numbers, which `tile_kinds` and
    // `tile_surfaces` read.
    let drawn = match &c.layout {
        Some(layout) => ppu_cells(model, assets, map, &format!("{at}.layout"), layout, c)?,
        None => None,
    };
    // The cell values `tile_surfaces` reads: the drawn tile numbers, or a
    // grid's metatile numbers.
    let values: Option<Vec<u8>> = match (&drawn, &c.grid) {
        _ if c.tile_surfaces.is_empty() => None,
        (Some(cells), _) => Some(cells.clone()),
        (None, Some(grid_slot)) => {
            let grid = slot_asset(model, map, &format!("{at}.grid"), grid_slot)?;
            Some(
                assets
                    .decode(map, grid.name())?
                    .remove(grid.name())
                    .ok_or(format!("{at}.grid: the grid decoded to nothing"))?,
            )
        }
        (None, None) => None,
    };
    let surfaces = match &values {
        _ if c.tile_surfaces.is_empty() => Vec::new(),
        Some(cells) => {
            let by: BTreeMap<u32, u8> = c
                .tile_surfaces
                .iter()
                .map(|(k, v)| {
                    k.parse::<u32>()
                        .map(|n| (n, *v))
                        .map_err(|_| format!("{at}.tile_surfaces: `{k}` isn't a whole number"))
                })
                .collect::<Result<_, _>>()?;
            if let Some((_, s)) = by.iter().find(|(_, s)| u32::from(**s) >= c.tile_size) {
                return Err(format!(
                    "{at}.tile_surfaces: {s} isn't a row inside a {}-pixel cell",
                    c.tile_size
                ));
            }
            cells
                .iter()
                .map(|t| {
                    by.get(&u32::from(*t))
                        .copied()
                        .unwrap_or(gp_sim::spec::NO_SURFACE)
                })
                .collect()
        }
        None => {
            return Err(format!(
                "{at}.tile_surfaces: only a grid or a level drawn from a PPU string has tile surfaces"
            ));
        }
    };
    let tiles = match (&c.layout, &c.grid, &c.tileset, &c.solid_property) {
        (Some(_), None, _, _) if drawn.is_some() => drawn
            .as_ref()
            .map(|cells| {
                cells
                    .iter()
                    .map(|t| by_value.get(&u32::from(*t)).copied().unwrap_or(Tile::Empty))
                    .collect()
            })
            .unwrap_or_default(),
        (None, Some(grid_slot), Some(tileset_slot), Some(solid_property)) => grid_tiles(
            model,
            assets,
            map,
            &at,
            c,
            grid_slot,
            tileset_slot,
            solid_property,
            &by_value,
        )?,
        (None, None, None, None) if c.empty => {
            vec![Tile::Empty; (c.size[0] as usize) * (c.size[1] as usize)]
        }
        _ => {
            return Err(format!(
                "{at}: a level needs a layout, or a grid with a tileset and a solid_property, or `empty = true`"
            ));
        }
    };

    let mut spawns = Vec::new();
    for (i, s) in c.spawn.iter().enumerate() {
        let sat = format!("{at}.spawn[{i}]");
        let kind = kind_of(&format!("{sat}.entity"), &s.entity)?;
        let rows: Vec<BTreeMap<String, Fx>> = match &s.table {
            None => vec![BTreeMap::new()],
            Some(table) => table_rows(model, assets, map, &format!("{sat}.table"), table)?,
        };
        for row in rows {
            let field = |f: &str, given: Option<Fixed>| -> Result<Option<Fx>, String> {
                if let Some(col) = s.columns.get(f) {
                    return row.get(col).copied().map(Some).ok_or(format!(
                        "{sat}.columns.{f}: the table has no column `{col}`"
                    ));
                }
                Ok(given.map(fx))
            };
            let x = field("x", s.x)?.ok_or(format!("{sat}: no x"))?;
            let y = field("y", s.y)?.ok_or(format!("{sat}: no y"))?;
            let patrol_end = field("patrol_end", s.params.get("patrol_end").copied())?;
            spawns.push(Spawn {
                kind,
                x,
                y,
                patrol_end,
                drawn: s.drawn,
                at_edge: s.at_edge.map(fx),
                appear_moving: s.appear_moving,
            });
        }
    }
    if c.spawn_objects.is_some() {
        return Err(format!(
            "{at}.spawn_objects: this tool has no spawn-object list formats"
        ));
    }
    Ok(Level {
        name: name.to_owned(),
        halfway: c
            .halfway
            .as_ref()
            .map(|h| (fx(h.from), [fx(h.at[0]), fx(h.at[1])])),
        tile_size: i64::from(c.tile_size),
        size: [i64::from(c.size[0]), i64::from(c.size[1])],
        tiles,
        surfaces,
        ladders: match &l.platformer.ladders {
            None => Vec::new(),
            Some(ld) => ladder_boxes(&at, "ladders", model, assets, map, ld)?,
        },
        ladder_tops: match &l.platformer.ladder_tops {
            None => Vec::new(),
            Some(ld) => ladder_boxes(&at, "ladder_tops", model, assets, map, ld)?,
        },
        ladder_stops: match &l.platformer.ladder_stops {
            None => Vec::new(),
            Some(ld) => ladder_boxes(&at, "ladder_stops", model, assets, map, ld)?,
        },
        ground: match &l.platformer.ground {
            None => Vec::new(),
            Some(g) => ground_boxes(&at, model, assets, map, g)?,
        },
        ground_feet: l
            .platformer
            .ground
            .as_ref()
            .map_or([0, 0], |g| g.feet.map(i64::from)),
        ground_land: l
            .platformer
            .ground
            .as_ref()
            .and_then(|g| g.land)
            .map(i64::from),
        ground_boxes: l.platformer.ground.as_ref().is_some_and(|g| g.boxes),
        ground_land_rising: l.platformer.ground.as_ref().is_some_and(|g| g.land_rising),
        ground_bodies: match &l.platformer.ground {
            None => Vec::new(),
            Some(g) => g
                .bodies
                .iter()
                .enumerate()
                .map(|(n, gb)| {
                    let kind = kind_of(&format!("{at}.ground.bodies[{n}].entity"), &gb.entity)?;
                    Ok((kind, gb.edges.map(i64::from)))
                })
                .collect::<Result<Vec<_>, String>>()?,
        },
        pickups: l
            .platformer
            .pickups
            .iter()
            .enumerate()
            .map(|(n, p)| {
                pickup_group(
                    &format!("{at}.pickups[{n}]"),
                    model,
                    assets,
                    map,
                    p,
                    l.core.size,
                    counter_of,
                    event,
                )
            })
            .collect::<Result<_, _>>()?,
        spawns,
        kill_below: l.platformer.kill_below.map(fx),
        pit_event: l
            .platformer
            .pit_event
            .as_deref()
            .map(|e| event(&format!("{at}.pit_event"), e))
            .transpose()?,
        goal_x: l.platformer.goal.as_ref().map(|g| fx(g.x)),
        goal_under: l.platformer.goal.as_ref().and_then(|g| g.under.map(fx)),
        hazard: l
            .platformer
            .hazard
            .as_ref()
            .map(|h| (touch(h.touch), i64::from(h.damage.unwrap_or(1)))),
        water: l
            .platformer
            .water
            .as_ref()
            .map(|wt| (fx(wt.gravity), fx(wt.max_fall))),
        ice: l
            .platformer
            .ice
            .as_ref()
            .map(|i| (fx(i.acceleration), fx(i.deceleration))),
        origin: c.origin.map_or([Fx::ZERO; 2], |o| [fx(o[0]), fx(o[1])]),
        camera: c.camera.as_ref().map(|cam| LevelCamera {
            axis: cam.axis.map(|a| {
                [
                    matches!(a, Axis::X | Axis::Both),
                    matches!(a, Axis::Y | Axis::Both),
                ]
            }),
            lead: cam.lead.map(|l| [fx(l[0]), fx(l[1])]),
            step: cam.step.map(fx),
            hold_player: cam.hold_player,
            keep_in_view: cam
                .keep_in_view
                .map(|k| [fx(k[0]).floor(), fx(k[1]).floor()]),
            keep_in_view_rows: cam
                .keep_in_view_rows
                .iter()
                .map(|r| (r.rows, r.bounds))
                .collect(),
        }),
        despawn_ahead: c.despawn_ahead.map(fx),
        respawn: c.respawn,
        counters: c
            .counters
            .iter()
            .map(|(k, v)| Ok((counter_of(&format!("{at}.counters.{k}"), k)?, *v)))
            .collect::<Result<_, String>>()?,
        next: c
            .next
            .as_deref()
            .map(|n| level_of(&format!("{at}.next"), n))
            .transpose()?,
        spawn_ahead: c.spawn_ahead.map(fx),
        spawn_slots: c.spawn_slots,
        despawn_by_position: c.despawn_at == Some(gp_model::model::DespawnAt::Position),
        despawn_after_move: c.despawn_after_move,
        spawn_in_order: c.spawn_in_order,
        appear_still: c.appear_still,
        slot_rest: c.slot_rest,
        hard: l.platformer.hard_spawns,
        hit_widths: c
            .hit_widths
            .iter()
            .map(|(k, w)| {
                let kind = kind_of(&format!("{at}.hit_widths.{k}"), k)?;
                Ok((kind, Fx::from_int(i64::from(*w))))
            })
            .collect::<Result<_, String>>()?,
        draw: c.draw.as_ref().map(|d| gp_sim::spec::Draw {
            ahead: fx(d.ahead),
            step: i64::from(d.step),
            every: d.every,
            pause: d.pause,
        }),
        scroll_stop: c
            .scroll_stop
            .as_ref()
            .map(|s| {
                if c.draw.is_none() {
                    return Err(format!("{at}.scroll_stop: needs the level's draw"));
                }
                Ok(gp_sim::spec::ScrollStop {
                    at: fx(s.at),
                    removes: s
                        .removes
                        .iter()
                        .map(|k| kind_of(&format!("{at}.scroll_stop.removes"), k))
                        .collect::<Result<_, String>>()?,
                })
            })
            .transpose()?,
        loop_back: c.loop_back.as_ref().map(|l| gp_sim::spec::LoopBack {
            back: i64::from(l.back),
            keep: i64::from(l.keep),
            checks: l
                .checks
                .iter()
                .map(|k| gp_sim::spec::LoopCheck {
                    x: i64::from(k.x),
                    y: i64::from(k.y),
                })
                .collect(),
        }),
        despawn_behind: c.despawn_behind.map(fx),
        exits: c
            .exit
            .iter()
            .enumerate()
            .filter_map(|(i, x)| x.take.as_ref().map(|t| (i, x, t)))
            .map(|(i, x, t)| {
                let xat = format!("{at}.exit[{i}]");
                Ok(Exit {
                    to: level_of(&format!("{xat}.to"), &x.to)?,
                    at: Rect {
                        x: fx(t.at.x),
                        y: fx(t.at.y),
                        w: fx(t.at.w),
                        h: fx(t.at.h),
                    },
                    hold: t
                        .hold
                        .as_deref()
                        .map(|h| action(&format!("{xat}.take.hold"), h))
                        .transpose()?,
                    standing: t.standing,
                    enter: x.enter.as_ref().map(|e| [fx(e.x), fx(e.y)]),
                    start: x.start,
                    guard: t
                        .guard
                        .as_ref()
                        .map(|g| guard(&format!("{xat}.take.if"), g, counter_of))
                        .transpose()?,
                    keep: x.keep.map(|k| match k {
                        gp_model::model::KeepAxis::X => 0,
                        gp_model::model::KeepAxis::Y => 1,
                    }),
                    scroll: x.scroll.unwrap_or(0),
                })
            })
            .collect::<Result<_, String>>()?,
    })
}

/// A level's tiles from a level grid and its tileset's properties: solid
/// where `solid_property` is nonzero, and with `kind_property`, the kind
/// `tile_kinds` gives its value.
#[allow(clippy::too_many_arguments)]
fn grid_tiles(
    model: &Model,
    assets: &Lifted,
    map: &AssetMap,
    at: &str,
    c: &gp_model::model::LevelCore,
    grid_slot: &str,
    tileset_slot: &str,
    solid_property: &str,
    by_value: &BTreeMap<u32, Tile>,
) -> Result<Vec<Tile>, String> {
    let grid = slot_asset(model, map, &format!("{at}.grid"), grid_slot)?;
    let Asset::Level { width, height, .. } = grid else {
        return Err(format!("{at}.grid: slot `{grid_slot}` isn't a level grid"));
    };
    if [*width, *height] != c.size {
        return Err(format!(
            "{at}.size: the grid is {width} by {height}, not {} by {}",
            c.size[0], c.size[1]
        ));
    }
    let cells = assets
        .decode(map, grid.name())?
        .remove(grid.name())
        .ok_or(format!("{at}.grid: the grid decoded to nothing"))?;
    let tileset = slot_asset(model, map, &format!("{at}.tileset"), tileset_slot)?;
    let parts = assets.decode(map, tileset.name())?;
    let property = |name: &str, field: &str| -> Result<Vec<u16>, String> {
        let prop = |suffix: &str| parts.get(&format!("{}.{}{suffix}", tileset.name(), name));
        match (prop(""), prop(" (low bytes)"), prop(" (high bytes)")) {
            (Some(v), _, _) => Ok(v.iter().map(|b| u16::from(*b)).collect()),
            (None, Some(lo), hi) => Ok(lo
                .iter()
                .enumerate()
                .map(|(i, b)| u16::from(*b) | hi.map_or(0, |h| u16::from(h[i])) << 8)
                .collect()),
            _ => Err(format!(
                "{at}.{field}: the tileset has no property `{name}`"
            )),
        }
    };
    let solid_of = property(solid_property, "solid_property")?;
    let kind_of = c
        .kind_property
        .as_deref()
        .map(|k| property(k, "kind_property"))
        .transpose()?;
    cells
        .iter()
        .enumerate()
        .map(|(i, &cell)| {
            let m = usize::from(cell);
            let solid = solid_of.get(m).ok_or(format!(
                "{at}.grid: cell {} of row {} is metatile {cell}, past the tileset's {}",
                i % *width as usize,
                i / *width as usize,
                solid_of.len()
            ))?;
            let base = if *solid != 0 {
                Tile::Solid
            } else {
                Tile::Empty
            };
            Ok(kind_of
                .as_ref()
                .and_then(|k| k.get(m))
                .and_then(|v| by_value.get(&u32::from(*v)))
                .copied()
                .unwrap_or(base))
        })
        .collect()
}

/// Core `level.layout_ppu_strings`: when the layout's slot is a
/// `ppu-strings` asset, the tile numbers its string draws on the first name
/// table, one a cell row by row (0 where nothing is drawn). The level is
/// 32 by 30 cells of 8 pixels, the name table's shape.
fn ppu_cells(
    model: &Model,
    assets: &Lifted,
    map: &AssetMap,
    at: &str,
    layout: &gp_model::model::EntryRef,
    c: &gp_model::model::LevelCore,
) -> Result<Option<Vec<u8>>, String> {
    let Asset::PpuStrings(p) = slot_asset(model, map, &format!("{at}.slot"), &layout.slot)? else {
        return Ok(None);
    };
    if c.tile_size != 8 || c.size != [32, 30] {
        return Err(format!(
            "{at}: a level drawn from a PPU string is the name table's 32 by 30 cells of 8 pixels"
        ));
    }
    let file = format!("{}.toml", p.name);
    let text = assets
        .files
        .get(&file)
        .ok_or(format!("{at}: {file} wasn't read"))?;
    let mut cells = vec![0u8; 32 * 30];
    for w in p
        .writes(text, &layout.entry)
        .map_err(|e| format!("{at}: {e}"))?
    {
        let mut ppu = w.ppu;
        for b in w.bytes {
            if (0x2000..0x23C0).contains(&ppu) {
                cells[usize::from(ppu - 0x2000)] = b;
            }
            ppu = ppu.wrapping_add(if w.vertical { 32 } else { 1 });
        }
    }
    Ok(Some(cells))
}

/// A table slot's rows, each column's value as a whole number.
fn table_rows(
    model: &Model,
    assets: &Lifted,
    map: &AssetMap,
    at: &str,
    slot: &str,
) -> Result<Vec<BTreeMap<String, Fx>>, String> {
    let asset = slot_asset(model, map, at, slot)?;
    let Asset::Table { count, columns, .. } = asset else {
        return Err(format!("{at}: slot `{slot}` isn't a table"));
    };
    let parts = assets.decode(map, asset.name())?;
    let mut rows = vec![BTreeMap::new(); *count as usize];
    for c in columns {
        let part = |suffix: &str| parts.get(&format!("{}.{}{suffix}", asset.name(), c.name));
        let (lo, hi) = match (part(""), part(" (low bytes)"), part(" (high bytes)")) {
            (Some(v), _, _) => (v, None),
            (None, Some(lo), hi) => (lo, hi),
            _ => return Err(format!("{at}: column `{}` decoded to nothing", c.name)),
        };
        for (i, row) in rows.iter_mut().enumerate() {
            let v = i64::from(lo[i]) | hi.map_or(0, |h| i64::from(h[i]) << 8);
            row.insert(c.name.clone(), Fx::from_int(v));
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod ladder_list_tests {
    use super::list_rows;

    /// Feature `level.ladder_lists`: a list's rows from its index up to its
    /// end mark, and the shapes' rows of 4 from theirs (a made-up table).
    #[test]
    fn a_list_in_a_table_reads_its_rows_to_the_end_mark() {
        let col = [9, 9, 10, 20, 0, 30, 40, 4, 0xFE, 1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(
            list_rows(&col, 2, 3, Some(0xFE)),
            vec![vec![10, 20, 0], vec![30, 40, 4]]
        );
        assert_eq!(
            list_rows(&col, 9, 4, None),
            vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]]
        );
        assert!(list_rows(&col, 40, 3, Some(0xFE)).is_empty());
    }
}
