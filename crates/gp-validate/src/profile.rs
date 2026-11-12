//! Measured profiles: what a run did, in the terms a model states it.
//!
//! Each function reads a [`Trace`] by state path (docs/validation.md lists
//! the paths) and measures one thing: jumps, walking and running speed, a
//! patrol's speed, the level graph. The `predicted_*` functions give the
//! same profiles from the model's own numbers, so a recording can be held
//! against what the model says as well as against another runtime.

use std::collections::{BTreeMap, BTreeSet};

use gp_model::Model;
use gp_model::model::{Button, Device};
use gp_model::{Fixed, platformer};

use crate::observe::Trace;

pub fn x_path(entity: &str, i: usize) -> String {
    format!("entities.{entity}[{i}].x")
}

pub fn y_path(entity: &str, i: usize) -> String {
    format!("entities.{entity}[{i}].y")
}

pub fn on_ground_path(entity: &str, i: usize) -> String {
    format!("entities.{entity}[{i}].on_ground")
}

pub fn alive_path(entity: &str, i: usize) -> String {
    format!("entities.{entity}[{i}].alive")
}

pub fn machine_path(machine: &str) -> String {
    format!("rules.{machine}.state")
}

pub fn counter_path(counter: &str) -> String {
    format!("counters.{counter}")
}

pub const LEVEL_PATH: &str = "level";

/// 1 while the player controls the game, where the original says
/// (`observe.toml`'s `control`). A run without it is in control throughout.
pub const CONTROL_PATH: &str = "control";

/// Whether the player controls the game on `frame`.
pub fn in_control(trace: &Trace, frame: usize) -> bool {
    !trace.has(CONTROL_PATH) || trace.num(frame, CONTROL_PATH) == Fixed::from_int(1).ok()
}

/// The level a run is in on a frame: the `level` path, or for a model with
/// one level, that level.
pub fn level_at(trace: &Trace, model: &Model, frame: usize) -> Option<String> {
    match trace.text(frame, LEVEL_PATH) {
        Some(l) => Some(l.to_owned()),
        None if model.levels.len() == 1 && !trace.has(LEVEL_PATH) => {
            model.levels.keys().next().cloned()
        }
        None => None,
    }
}

const ONE: i64 = 1 << 16;

/// Measurements, each with the frame it was made on.
pub type Samples = Vec<(usize, Fixed)>;

/// Whether every state machine stays in one state from `frame - 1` to
/// `frame`, and that state doesn't freeze the game: the frames movement is
/// measured on.
pub fn steady(trace: &Trace, model: &Model, frame: usize) -> bool {
    model.rules.iter().all(|(name, m)| {
        let p = machine_path(name);
        match (trace.text(frame - 1, &p), trace.text(frame, &p)) {
            (Some(a), Some(b)) => a == b && !m.states.get(b).is_some_and(|s| s.freeze),
            // A machine the original's mapping doesn't name can't stop a
            // measurement.
            (None, None) => true,
            _ => false,
        }
    })
}

/// One jump: from the ground back to the same height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JumpSample {
    /// The first frame off the ground.
    pub frame: usize,
    pub height: Fixed,
    /// Frames from leaving the ground to landing.
    pub airtime: i64,
}

/// Every jump an entity's instances make: leaving the ground moving up and
/// landing at the height it left from, with no state machine changing on
/// the way. Falls, bounces and landings on other ground aren't jumps, and
/// neither is a rise cut short: with the vertical speed mapped, a jump one
/// of whose frames adds more than twice what any other adds to the speed
/// (a head bumping a block sets it to 0 at once) is left out. The jump's
/// own frames are the measure, so a model with the wrong gravity is still
/// held to the original's jumps.
pub fn jumps(trace: &Trace, model: &Model, entity: &str) -> Vec<JumpSample> {
    let mut out = Vec::new();
    for i in trace.instances(entity) {
        let vy = format!("entities.{entity}[{i}].vy");
        let cut = |from: usize, to: usize| -> bool {
            let mut steps: Vec<i64> = (from + 1..=to)
                .filter_map(|f| Some(trace.num(f, &vy)?.raw() - trace.num(f - 1, &vy)?.raw()))
                .filter(|d| *d > 0)
                .collect();
            steps.sort_unstable_by(|a, b| b.cmp(a));
            matches!(steps.as_slice(), [a, b, ..] if *a > 2 * *b)
        };
        let (y, g) = (y_path(entity, i), on_ground_path(entity, i));
        let on = |f: usize| trace.num(f, &g).map(|v| v != Fixed::ZERO);
        let mut t = 1;
        while t < trace.len() {
            let (Some(true), Some(false), Some(y0), Some(y1)) =
                (on(t - 1), on(t), trace.num(t - 1, &y), trace.num(t, &y))
            else {
                t += 1;
                continue;
            };
            if y1 >= y0 {
                t += 1;
                continue;
            }
            let Some(land) = (t + 1..trace.len()).find(|f| on(*f) == Some(true)) else {
                break;
            };
            let unbroken = (t..=land).all(|f| {
                model
                    .rules
                    .keys()
                    .all(|m| trace.text(f, &machine_path(m)) == trace.text(f - 1, &machine_path(m)))
            });
            let peak = (t..=land).filter_map(|f| trace.num(f, &y)).min();
            let unbroken = unbroken && !cut(t, land);
            if let (true, Some(peak), Some(back)) = (unbroken, peak, trace.num(land, &y))
                && back == y0
            {
                out.push(JumpSample {
                    frame: t,
                    height: Fixed::from_raw(y0.raw() - peak.raw()),
                    airtime: (land - t) as i64,
                });
            }
            t = land + 1;
        }
    }
    out.sort_by_key(|s| s.frame);
    out
}

/// The jump the model's numbers give on flat ground: its height and its
/// frames in the air. Each frame gravity is added to the speed, up to the
/// fall limit, then the speed to the height; it lands the first frame it's
/// falling with its feet at or below the ground's row of pixels.
pub fn predicted_jump(jump: &platformer::Jump) -> Option<(Fixed, i64)> {
    if jump.speed >= Fixed::ZERO {
        return None;
    }
    let (mut y, mut vy, mut top) = (0i64, jump.speed.raw(), 0i64);
    for n in 1..100_000i64 {
        vy = (vy + jump.gravity.raw()).min(jump.max_fall.raw());
        y += vy;
        if vy > 0 && y >> 16 >= 0 {
            return Some((Fixed::from_raw(-top), n - 1));
        }
        top = top.min(y);
    }
    None
}

/// The controller bit of a button, as the runtime protocol orders the
/// model's device's buttons (on the NES: A, B, Select, Start, Up, Down,
/// Left, Right from bit 0); none when the device hasn't the button.
pub fn button_bit(device: Device, b: Button) -> Option<u8> {
    device.bit(b)
}

fn action_bit(model: &Model, action: &str) -> Option<u8> {
    model
        .input
        .as_ref()
        .and_then(|i| i.actions.get(action))
        .and_then(|b| button_bit(model.input.as_ref()?.device, *b))
}

/// Frames a speed must hold, with the same input, on the ground, before it
/// counts as the speed walking or running settles at. Longer than Super
/// Mario Bros.' running timer, which keeps Mario at running speed for 10
/// frames after B is let go.
pub const SETTLE_FRAMES: usize = 16;

/// A frame's direction held (left, right), whether running, and speed.
type Settling = ((bool, bool), bool, Fixed);

/// Horizontal speeds an entity settles at under the player's control, one
/// per frame, split into walking and running by whether the run action was
/// held. A frame counts when, over it and the [`SETTLE_FRAMES`] - 1 before,
/// the player controlled the game ([`in_control`]), one direction (and the
/// run action, or not) was held, the entity was on
/// the ground (where `on_ground` is mapped), no state machine changed, none
/// froze the game, and the speed stayed the same. So the speeds of
/// speeding up and slowing down, and speed kept in the air, are left out.
/// The speed is the entity's mapped `vx` where there is one, and otherwise
/// the distance moved each frame.
pub fn run_speeds(
    trace: &Trace,
    model: &Model,
    entity: &str,
    run_action: Option<&str>,
) -> (Samples, Samples) {
    let (mut walk, mut run) = (Vec::new(), Vec::new());
    let (Some(left), Some(right)) = (action_bit(model, "left"), action_bit(model, "right")) else {
        return (walk, run);
    };
    let run_bit = run_action.and_then(|a| action_bit(model, a));
    for i in trace.instances(entity) {
        let x = x_path(entity, i);
        let vx = format!("entities.{entity}[{i}].vx");
        let ground = on_ground_path(entity, i);
        let by_vx = trace.has(&vx);
        let has_ground = trace.has(&ground);
        // Each frame's input kind (walking left, running right...) and
        // speed, where the frame qualifies at all.
        let at = |f: usize| -> Option<Settling> {
            let (p0, p1) = (trace.pad_into(f - 1)?, trace.pad_into(f)?);
            let dir = |p: u8| (p & left != 0, p & right != 0);
            let d = dir(p1);
            if dir(p0) != d || !(d == (true, false) || d == (false, true)) {
                return None;
            }
            let running = run_bit.is_some_and(|b| p0 & b != 0 && p1 & b != 0);
            let walking = run_bit.is_none_or(|b| p0 & b == 0 && p1 & b == 0);
            if !(running || walking) || !steady(trace, model, f) || !in_control(trace, f) {
                return None;
            }
            if has_ground && trace.num(f, &ground).is_none_or(|g| g == Fixed::ZERO) {
                return None;
            }
            let speed = if by_vx {
                Fixed::from_raw(trace.num(f, &vx)?.raw().abs())
            } else {
                let (a, b) = (trace.num(f - 1, &x)?, trace.num(f, &x)?);
                Fixed::from_raw((b.raw() - a.raw()).abs())
            };
            Some((d, running, speed))
        };
        let frames: Vec<Option<Settling>> = (0..trace.len())
            .map(|f| if f >= 2 { at(f) } else { None })
            .collect();
        for f in SETTLE_FRAMES + 1..trace.len() {
            let Some(now) = frames[f] else { continue };
            let settled = frames[f + 1 - SETTLE_FRAMES..f]
                .iter()
                .all(|w| *w == Some(now));
            if settled && now.2 != Fixed::ZERO {
                if now.1 { &mut run } else { &mut walk }.push((f, now.2));
            }
        }
    }
    (walk, run)
}

/// The speed most frames moved at; the faster one on a tie.
pub fn mode(speeds: &[Fixed]) -> Option<Fixed> {
    let mut counts: BTreeMap<Fixed, usize> = BTreeMap::new();
    for s in speeds {
        *counts.entry(*s).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(s, n)| (*n, *s))
        .map(|(s, _)| s)
}

/// Frames a walking or patrolling entity's speed is measured over: its
/// distance moved in that many frames, divided by them.
pub const WINDOW_FRAMES: usize = 4;

/// The speeds each living instance of an entity moves at, each over
/// [`WINDOW_FRAMES`] frames that are all steady and in which it lives: the
/// samples a patrol's or a walk's speed is the most common of. Windows in
/// which it doesn't move are left out. Over a few
/// frames a speed with a fraction (half a pixel a frame, as one pixel every
/// other frame) shows whole, and the most common is the walking speed even
/// when the game also holds its enemies still for a moment (Super Mario
/// Bros. does while Mario grows or dies), kicks them, or reuses a slot.
pub fn patrol_steps(trace: &Trace, model: &Model, entity: &str) -> Vec<(usize, Fixed)> {
    let w = WINDOW_FRAMES;
    let mut out = Vec::new();
    for i in trace.instances(entity) {
        let (x, alive) = (x_path(entity, i), alive_path(entity, i));
        let living =
            |f: usize| !trace.has(&alive) || trace.num(f, &alive) == Fixed::from_int(1).ok();
        let ok: Vec<bool> = (0..trace.len())
            .map(|f| f >= 1 && living(f - 1) && living(f) && steady(trace, model, f))
            .collect();
        for f in w..trace.len() {
            if !ok[f + 1 - w..=f].iter().all(|o| *o) {
                continue;
            }
            if let (Some(a), Some(b)) = (trace.num(f - w, &x), trace.num(f, &x)) {
                // Standing still isn't walking at some speed.
                let d = (b.raw() - a.raw()).abs() / w as i64;
                if d != 0 {
                    out.push((f, Fixed::from_raw(d)));
                }
            }
        }
    }
    out.sort_by_key(|(f, _)| *f);
    out
}

/// Where a camera keeps the entity it follows along x while it scrolls:
/// on each steady frame the camera moved right, the entity's first
/// instance's `x` less the camera's, in whole pixels. A camera that only moves along y, or
/// whose `x` isn't in the run's state, gives none.
pub fn camera_leads(trace: &Trace, model: &Model, camera: &str) -> Vec<(usize, Fixed)> {
    let Some(c) = model.cameras.get(camera) else {
        return Vec::new();
    };
    if c.axis == gp_model::model::Axis::Y {
        return Vec::new();
    }
    let (cx, px) = (format!("cameras.{camera}.x"), x_path(&c.follow, 0));
    let mut out = Vec::new();
    for f in 1..trace.len() {
        let (Some(a), Some(b), Some(p)) =
            (trace.num(f - 1, &cx), trace.num(f, &cx), trace.num(f, &px))
        else {
            continue;
        };
        if b > a && steady(trace, model, f) {
            // In whole pixels: the view's position is whole on screen.
            out.push((f, Fixed::from_raw(((p.raw() - b.raw()) >> 16) << 16)));
        }
    }
    out
}

pub fn mean(steps: &[Fixed]) -> Option<Fixed> {
    let n = i64::try_from(steps.len()).ok().filter(|n| *n > 0)?;
    let sum: i64 = steps.iter().map(|s| s.raw()).sum();
    Some(Fixed::from_raw(sum / n))
}

/// The speed a patrol states: `step` pixels every `period` frames.
pub fn predicted_patrol(p: &platformer::Patrol) -> Option<Fixed> {
    (p.period > 0).then(|| Fixed::from_raw(p.step.raw() / i64::from(p.period)))
}

/// The level graph: the levels, the ways between them, and the objects in
/// each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    pub levels: BTreeSet<String>,
    /// From a level to another level, or to an end state such as
    /// `rules.game=clear`.
    pub edges: BTreeSet<(String, String)>,
    /// By level, each entity's count: the most instances alive at once, or
    /// `None` when the model's spawns don't say.
    pub objects: BTreeMap<String, BTreeMap<String, Option<u32>>>,
    /// Levels whose objects come from lifted data (`spawn_objects`), which
    /// the model alone doesn't list.
    pub lifted_objects: BTreeSet<String>,
}

/// An end state: one with no transitions out of it.
fn ends(model: &Model) -> BTreeSet<(String, String)> {
    model
        .rules
        .iter()
        .flat_map(|(m, machine)| {
            machine
                .states
                .iter()
                .filter(|(_, s)| s.transition.is_empty())
                .map(move |(s, _)| (m.clone(), s.clone()))
        })
        .collect()
}

pub fn end_name(machine: &str, state: &str) -> String {
    format!("rules.{machine}={state}")
}

/// The graph a run shows, from the frames `keep` admits.
pub fn observed_graph(trace: &Trace, model: &Model, keep: &impl Fn(usize) -> bool) -> Graph {
    let mut g = Graph::default();
    let ends = ends(model);
    // The level last seen, so a way between levels counts across frames
    // that are in none, as while the next one loads.
    let mut last: Option<String> = None;
    for f in (0..trace.len()).filter(|f| keep(*f)) {
        let Some(level) = level_at(trace, model, f) else {
            continue;
        };
        if let Some(before) = last.replace(level.clone())
            && before != level
        {
            g.edges.insert((before, level.clone()));
        }
        let level = level.as_str();
        g.levels.insert(level.to_owned());
        if f > 0 {
            for (m, s) in &ends {
                let p = machine_path(m);
                if trace.text(f, &p) == Some(s) && trace.text(f - 1, &p) != Some(s) {
                    g.edges.insert((level.to_owned(), end_name(m, s)));
                }
            }
        }
        let objects = g.objects.entry(level.to_owned()).or_default();
        for e in model.entities.keys() {
            let present = trace
                .instances(e)
                .into_iter()
                .filter(|i| {
                    trace.get(f, &x_path(e, *i)).is_some()
                        && (!trace.has(&alive_path(e, *i))
                            || trace.num(f, &alive_path(e, *i)) == Fixed::from_int(1).ok())
                })
                .count() as u32;
            if present > 0 {
                let n = objects.entry(e.clone()).or_insert(Some(0));
                *n = Some(n.unwrap_or(0).max(present));
            }
        }
    }
    g
}

/// The graph the model states: its levels, an edge from each level to its
/// `next` and its exits' levels, an edge from each level with a goal to
/// every end state `goal.reached` leads to, and each level's spawns,
/// counted when no spawn reads a table.
pub fn model_graph(model: &Model) -> Graph {
    let mut g = Graph::default();
    for (name, level) in &model.levels {
        g.levels.insert(name.clone());
        // The level after it, and its other exits (pipes, vines).
        for to in level
            .core
            .next
            .iter()
            .chain(level.core.exit.iter().map(|e| &e.to))
        {
            g.edges.insert((name.clone(), to.clone()));
        }
        if level.platformer.goal.is_some() {
            for (m, machine) in &model.rules {
                for state in machine.states.values() {
                    for t in &state.transition {
                        if t.on.as_deref() == Some("goal.reached")
                            && machine
                                .states
                                .get(&t.to)
                                .is_some_and(|s| s.transition.is_empty())
                        {
                            g.edges.insert((name.clone(), end_name(m, &t.to)));
                        }
                    }
                }
            }
        }
        if level.core.spawn_objects.is_some() {
            g.lifted_objects.insert(name.clone());
        }
        let objects = g.objects.entry(name.clone()).or_default();
        for s in &level.core.spawn {
            let n = objects.entry(s.entity.clone()).or_insert(Some(0));
            *n = match (*n, &s.table) {
                (Some(k), None) => Some(k + 1),
                _ => None,
            };
        }
    }
    // The stages in order (core `stages`): each leads to the next, the last
    // back to the first. A stage the model has no level for is a level of
    // the graph too, as the original's mapping names it (Donkey Kong's 25m
    // to 75m, played by the stand-in).
    if let Some(st) = &model.stages {
        for (i, a) in st.order.iter().enumerate() {
            let b = &st.order[(i + 1) % st.order.len()];
            g.levels.insert(a.clone());
            if a != b {
                g.edges.insert((a.clone(), b.clone()));
            }
        }
    }
    g
}

/// A whole number of frames or pixels as a fixed-point number.
pub fn whole(n: i64) -> Fixed {
    Fixed::from_raw(n * ONE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::Obs;

    fn model() -> Model {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop");
        Model::parse(&std::fs::read_to_string(dir.join("model.toml")).unwrap()).unwrap()
    }

    fn num(v: &str) -> Option<Obs> {
        Some(Obs::Num(Fixed::parse(v).unwrap()))
    }

    #[test]
    fn moss_hops_jump_is_47_and_a_half_pixels_and_39_frames() {
        let m = model();
        let jump = m.entities["player"].platformer.jump.as_ref().unwrap();
        let (h, a) = predicted_jump(jump).unwrap();
        assert_eq!((h.to_string(), a), ("47.5".to_owned(), 39));
        let patrol = m.entities["slug"].platformer.patrol.as_ref().unwrap();
        assert_eq!(predicted_patrol(patrol).unwrap().to_string(), "0.5");
    }

    /// A trace of the player at these heights and ground flags, playing.
    fn player(ys: &[(&str, u8)]) -> Trace {
        let mut t = Trace::new(vec![
            "entities.player[0].y".into(),
            "entities.player[0].on_ground".into(),
            "rules.game.state".into(),
        ]);
        for (y, g) in ys {
            t.push(
                vec![
                    num(y),
                    num(&g.to_string()),
                    Some(Obs::Text("playing".into())),
                ],
                0,
            );
        }
        t
    }

    #[test]
    fn finds_jumps_and_passes_over_falls() {
        let m = model();
        let t = player(&[
            ("176", 1),
            ("172", 0),
            ("170.5", 0),
            ("172", 0),
            ("176", 1),
            // A fall from a ledge: down first.
            ("177", 0),
            ("180", 1),
            // A jump onto higher ground.
            ("176", 0),
            ("170", 1),
        ]);
        assert_eq!(
            jumps(&t, &m, "player"),
            [JumpSample {
                frame: 1,
                height: Fixed::parse("5.5").unwrap(),
                airtime: 3
            }]
        );
    }

    #[test]
    fn walking_speed_is_the_most_common_over_a_few_frames() {
        let m = model();
        let mut t = Trace::new(vec!["entities.slug[0].x".into()]);
        // A pixel every other frame, a long stop, a jump to another place
        // (a slot reused), then walking on.
        for x in [0, 1, 1, 2, 2, 3, 3, 3, 3, 3, 3, 3, 90, 91, 91, 92, 92, 93] {
            t.push(vec![num(&x.to_string())], 0);
        }
        let steps: Vec<Fixed> = patrol_steps(&t, &m, "slug")
            .into_iter()
            .map(|(_, d)| d)
            .collect();
        assert_eq!(mode(&steps).unwrap().to_string(), "0.5", "{steps:?}");
    }

    #[test]
    fn modes_and_means() {
        let f = |s: &str| Fixed::parse(s).unwrap();
        assert_eq!(mode(&[f("1"), f("2"), f("2"), f("1")]), Some(f("2")));
        assert_eq!(mode(&[f("1"), f("1"), f("2")]), Some(f("1")));
        assert_eq!(mode(&[]), None);
        assert_eq!(mean(&[f("1"), f("0")]), Some(f("0.5")));
        assert_eq!(mean(&[]), None);
    }

    #[test]
    fn moss_hops_stated_graph() {
        let g = model_graph(&model());
        assert_eq!(g.levels.iter().collect::<Vec<_>>(), ["main"]);
        assert_eq!(
            g.edges.iter().cloned().collect::<Vec<_>>(),
            [("main".to_owned(), "rules.game=clear".to_owned())]
        );
        let objects: Vec<String> = g.objects["main"]
            .iter()
            .map(|(e, n)| format!("{e} {n:?}"))
            .collect();
        assert_eq!(objects, ["player Some(1)", "slug None"]);
    }
}
