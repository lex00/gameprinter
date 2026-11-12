//! Running compiled checks on observed runs.
//!
//! Every check is held against two things. Against the model: the
//! original's run measured and compared with what the model's own numbers
//! say (a jump of 47.5 pixels, a dying state that lasts 90 frames). Against
//! the original: a subject runtime given the same input, measured the same
//! way and compared with the original's run. Each comparison gives an
//! [`Outcome`] per quantity.

use std::collections::{BTreeMap, BTreeSet};

use gp_contract::{Level, Tolerance};
use gp_model::{Fixed, Model};
use gp_runtime::select::{Condition, Index, Selector};
use serde::{Deserialize, Serialize};

use crate::compile::{At, Check, Governing, Kind, Plan};
use crate::observe::Trace;
use crate::profile::{self, Graph, JumpSample};

/// What a run is compared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Against {
    /// The model's stated numbers, with the original's run measured.
    Model,
    /// The original's run, with the subject's run measured.
    Original,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
    /// No frame of the run gave the check anything to measure.
    Unexercised,
}

/// One quantity of one check, compared once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// The part's model path.
    pub part: String,
    pub kind: String,
    pub quantity: String,
    /// Which of several measurements of the quantity, such as `walk`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
    pub against: Against,
    /// The levels it was checked at, comma separated.
    pub level: String,
    pub status: Status,
    /// How many measurements (jumps, frames, transitions) it compared.
    pub samples: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<String>,
    /// The first frame that failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Outcome {
    /// The check's identity across recordings.
    pub fn key(&self) -> (String, String, Option<String>, Against) {
        (
            self.part.clone(),
            self.quantity.clone(),
            self.case.clone(),
            self.against,
        )
    }
}

/// Whether `actual` is within `tolerance` of `expected`. No tolerance
/// means equal.
pub fn within(expected: Fixed, actual: Fixed, tolerance: Option<&Tolerance>) -> bool {
    let diff = i128::from((actual.raw() - expected.raw()).abs());
    match tolerance {
        None => diff == 0,
        Some(Tolerance::Absolute(a)) => diff <= i128::from(a.raw()),
        // diff / |expected| <= p / 100, in raw units on both sides.
        Some(Tolerance::Relative(p)) => {
            diff * 100 * (1 << 16) <= i128::from(expected.raw().abs()) * i128::from(p.raw())
        }
    }
}

/// The path a selector names in a trace: indexes in decimal.
fn trace_path(sel: &Selector) -> Result<String, String> {
    let mut out = String::new();
    for (i, s) in sel.segments.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        out.push_str(&s.name);
        match s.index {
            None => {}
            Some(Index::At(n)) => out.push_str(&format!("[{n}]")),
            Some(_) => return Err(format!("{sel}: a condition reads one value")),
        }
    }
    Ok(out)
}

/// Evaluates a `when` on one frame of the original's run. A path reads its
/// whole part, rounded down; a name or a missing value is an error.
pub fn holds(cond: &Condition, trace: &Trace, frame: usize) -> Result<bool, String> {
    cond.eval(&mut |sel| {
        let p = trace_path(sel)?;
        match trace.get(frame, &p) {
            Some(crate::observe::Obs::Num(n)) => Ok(n.raw() >> 16),
            Some(crate::observe::Obs::Text(t)) => Err(format!(
                "{p} is the name `{t}`; a condition compares numbers"
            )),
            None => Err(format!(
                "{p} isn't in the original's state on frame {frame}"
            )),
        }
    })
}

/// The level of a check's quantity on each frame of the original's run.
struct Levels<'a> {
    check: &'a Check,
    quantity: &'a str,
    original: &'a Trace,
    /// The hooks that may write this check's counter (a [`Kind::Counter`]
    /// check's), from [`Plan::kept_by_hooks`].
    hooks: &'a [String],
}

impl Levels<'_> {
    /// The governing level on `frame`, or why there's none.
    fn at(&self, frame: usize) -> Result<Governing, String> {
        let mut error = None;
        let at = self.check.at(self.quantity, &mut |c| {
            holds(c, self.original, frame).unwrap_or_else(|e| {
                error.get_or_insert(e);
                false
            })
        });
        if let Some(e) = error {
            return Err(format!("a `when` can't be read: {e}"));
        }
        match at {
            At::Level(g) => Ok(g),
            At::Conflict(a, b) => Err(format!(
                "rule[{a}] and rule[{b}] both hold, with different levels"
            )),
        }
    }
}

/// Collects the comparisons of one quantity into an outcome.
struct Tally {
    outcome: Outcome,
    levels: BTreeSet<Level>,
}

impl Tally {
    fn new(check: &Check, quantity: &str, case: Option<&str>, against: Against) -> Self {
        Self {
            outcome: Outcome {
                part: check.part.clone(),
                kind: check.kind.name().to_owned(),
                quantity: quantity.to_owned(),
                case: case.map(str::to_owned),
                against,
                level: String::new(),
                status: Status::Unexercised,
                samples: 0,
                expected: None,
                actual: None,
                tolerance: None,
                frame: None,
                detail: None,
            },
            levels: BTreeSet::new(),
        }
    }

    /// Records one comparison; the first failure's values are kept, and
    /// until then the latest pass's.
    fn add(
        &mut self,
        level: Level,
        ok: bool,
        frame: usize,
        expected: impl FnOnce() -> String,
        actual: impl FnOnce() -> String,
        tolerance: Option<&Tolerance>,
    ) {
        self.levels.insert(level);
        self.outcome.samples += 1;
        if self.outcome.status == Status::Fail {
            return;
        }
        self.outcome.expected = Some(expected());
        self.outcome.actual = Some(actual());
        self.outcome.tolerance = tolerance.map(ToString::to_string);
        if ok {
            self.outcome.status = Status::Pass;
        } else {
            self.outcome.status = Status::Fail;
            self.outcome.frame = Some(frame as u64);
        }
    }

    /// A failure that isn't a comparison: a condition that can't be read,
    /// or runs that can't be matched up.
    fn fail(&mut self, frame: Option<usize>, detail: String) {
        if self.outcome.status != Status::Fail {
            self.outcome.status = Status::Fail;
            self.outcome.frame = frame.map(|f| f as u64);
            self.outcome.detail = Some(detail);
        }
    }

    fn note(&mut self, detail: String) {
        self.outcome.detail.get_or_insert(detail);
    }

    fn done(mut self) -> Outcome {
        self.outcome.level = self
            .levels
            .iter()
            .map(|l| l.name())
            .collect::<Vec<_>>()
            .join(",");
        if self.outcome.level.is_empty() {
            self.outcome.level = "none".into();
        }
        self.outcome
    }
}

/// The level on `frame` when the check runs there, `None` when it doesn't
/// (Free, Style, a level this kind isn't checked at), or the failure.
fn active(levels: &Levels<'_>, frame: usize, tally: &mut Tally) -> Option<Governing> {
    match levels.at(frame) {
        Ok(g) if levels.check.kind.checks_at(g.level) => Some(g),
        Ok(_) => None,
        Err(e) => {
            tally.fail(Some(frame), e);
            None
        }
    }
}

/// The tolerance for a quantity at a level: none at Exact.
fn tolerance_of<'g>(g: &'g Governing, quantity: &str) -> Option<&'g Tolerance> {
    match g.level {
        Level::Metric => g.tolerance.get(quantity),
        _ => None,
    }
}

/// Evaluates every check of a plan on one recording: against the model on
/// the original's run, and against the original on the subject's run, if
/// there is one.
pub fn evaluate(
    plan: &Plan,
    model: &Model,
    original: &Trace,
    subject: Option<&Trace>,
) -> Vec<Outcome> {
    GRAPHS.with(|g| g.borrow_mut().1.clear());
    let mut out = Vec::new();
    for check in &plan.checks {
        for quantity in check.kind.quantities() {
            if !check
                .possible(quantity)
                .iter()
                .any(|l| check.kind.checks_at(*l))
            {
                continue;
            }
            let levels = Levels {
                check,
                quantity,
                original,
                hooks: plan
                    .kept_by_hooks
                    .get(&check.name)
                    .filter(|_| check.kind == Kind::Counter)
                    .map_or(&[], Vec::as_slice),
            };
            out.extend(against_model(&levels, model));
            if let Some(subject) = subject {
                out.extend(against_original(&levels, model, subject));
            }
        }
    }
    out
}

fn fixed_text(f: Fixed) -> String {
    f.to_string()
}

fn against_model(lv: &Levels<'_>, model: &Model) -> Vec<Outcome> {
    let (check, q, original) = (lv.check, lv.quantity, lv.original);
    let entity = model.entities.get(&check.name);
    match check.kind {
        Kind::Jump => {
            let mut t = Tally::new(check, q, None, Against::Model);
            let jump = entity.and_then(|e| e.platformer.jump.as_ref());
            match jump.and_then(profile::predicted_jump) {
                None => t.note("the model's jump doesn't go up".into()),
                Some((height, airtime)) => {
                    for s in profile::jumps(original, model, &check.name) {
                        let Some(g) = active(lv, s.frame, &mut t) else {
                            continue;
                        };
                        let tol = tolerance_of(&g, q);
                        let (want, got) = jump_quantity(q, height, airtime, &s);
                        t.add(
                            g.level,
                            within(want, got, tol),
                            s.frame,
                            || fixed_text(want),
                            || fixed_text(got),
                            tol,
                        );
                    }
                }
            }
            vec![t.done()]
        }
        Kind::Run => {
            let run = entity.and_then(|e| e.platformer.run.as_ref());
            let Some(run) = run else { return Vec::new() };
            let (walk, running) =
                profile::run_speeds(original, model, &check.name, run.run_action.as_deref());
            let mut out = vec![speed_vs_stated(
                lv,
                "walk",
                Some(run.walk_speed),
                &walk,
                profile::mode,
            )];
            if run.run_speed.is_some() {
                out.push(speed_vs_stated(
                    lv,
                    "run",
                    run.run_speed,
                    &running,
                    profile::mode,
                ));
            }
            out
        }
        Kind::Patrol => {
            let p = entity.and_then(|e| e.platformer.patrol.as_ref());
            let steps = profile::patrol_steps(original, model, &check.name);
            vec![speed_vs_stated(
                lv,
                "patrol",
                p.and_then(profile::predicted_patrol),
                &steps,
                profile::mode,
            )]
        }
        Kind::Walk => {
            let w = entity.and_then(|e| e.platformer.walk.as_ref());
            let steps = profile::patrol_steps(original, model, &check.name);
            vec![speed_vs_stated(
                lv,
                "walk",
                w.map(|w| w.speed),
                &steps,
                profile::mode,
            )]
        }
        // Measured beside the original's slots ([`crate::slots`]), and on
        // the model alone ([`crate::reach`]).
        Kind::Slots | Kind::Reach => Vec::new(),
        Kind::Camera => {
            let lead = model.cameras.get(&check.name).map(|c| c.lead[0]);
            let leads = profile::camera_leads(original, model, &check.name);
            let mut o = speed_vs_stated(lv, "scrolling", lead, &leads, profile::mode);
            if let Some(d) = range_text(&leads) {
                o.detail.get_or_insert(d);
            }
            vec![o]
        }
        Kind::Machine => vec![machine_vs_model(lv, model)],
        Kind::States => vec![states_vs_model(lv, model)],
        Kind::Counter => vec![counter_vs_model(lv, model)],
        Kind::Level => {
            let mut t = Tally::new(check, q, None, Against::Model);
            let (graph, _, level) = graph_of(lv, original, model, &mut t);
            let frames = frames_in(original, model, &check.name);
            if !graph.levels.contains(&check.name) {
                t.note("the run never enters this level".into());
            } else if let Some(level) = level {
                let stated = profile::model_graph(model);
                let (ok, detail) = graph_within(&stated, &graph, original, &check.name);
                t.add(
                    level,
                    ok,
                    0,
                    || graph_text(&stated, &check.name),
                    || graph_text(&graph, &check.name),
                    None,
                );
                t.outcome.samples = frames;
                if let Some(d) = detail {
                    t.note(d);
                }
            }
            vec![t.done()]
        }
    }
}

fn jump_quantity(q: &str, height: Fixed, airtime: i64, s: &JumpSample) -> (Fixed, Fixed) {
    if q == "height" {
        (height, s.height)
    } else {
        (profile::whole(airtime), profile::whole(s.airtime))
    }
}

/// Groups samples by the level governing their frame, so each group is
/// summarized and compared under its own tolerance.
fn by_level(
    lv: &Levels<'_>,
    samples: &[(usize, Fixed)],
    t: &mut Tally,
) -> Vec<(Governing, Vec<Fixed>, usize)> {
    let mut groups: Vec<(Governing, Vec<Fixed>, usize)> = Vec::new();
    for (f, v) in samples {
        let Some(g) = active(lv, *f, t) else { continue };
        match groups.iter_mut().find(|(h, _, _)| *h == g) {
            Some((_, vs, _)) => vs.push(*v),
            None => groups.push((g, vec![*v], *f)),
        }
    }
    groups
}

/// The range of a camera's leads and how often each end shows, for the
/// report: a camera that scrolls from a window, rather than one place,
/// shows several.
fn range_text(leads: &[(usize, Fixed)]) -> Option<String> {
    let values: Vec<Fixed> = leads.iter().map(|(_, v)| *v).collect();
    let (lo, hi) = (values.iter().min()?, values.iter().max()?);
    let mode = profile::mode(&values)?;
    let at_mode = values.iter().filter(|v| **v == mode).count();
    Some(format!(
        "{} scrolling frames, the entity from {lo} to {hi} px into the view, {at_mode} of them at {mode}",
        values.len()
    ))
}

fn speed_vs_stated(
    lv: &Levels<'_>,
    case: &str,
    stated: Option<Fixed>,
    samples: &[(usize, Fixed)],
    summary: fn(&[Fixed]) -> Option<Fixed>,
) -> Outcome {
    let mut t = Tally::new(lv.check, lv.quantity, Some(case), Against::Model);
    let Some(stated) = stated else {
        t.note("the model states no speed".into());
        return t.done();
    };
    for (g, values, first) in by_level(lv, samples, &mut t) {
        let Some(measured) = summary(&values) else {
            continue;
        };
        let tol = tolerance_of(&g, lv.quantity);
        t.add(
            g.level,
            within(stated, measured, tol),
            first,
            || fixed_text(stated),
            || fixed_text(measured),
            tol,
        );
        t.outcome.samples += values.len() as u64 - 1;
    }
    t.done()
}

/// The model's rules, seen in the original: every change of state is a
/// transition the model has, a timed transition takes its time exactly,
/// and nothing moves in a frozen state.
fn machine_vs_model(lv: &Levels<'_>, model: &Model) -> Outcome {
    let (check, original) = (lv.check, lv.original);
    let mut t = Tally::new(check, lv.quantity, None, Against::Model);
    let machine = &model.rules[&check.name];
    let p = profile::machine_path(&check.name);
    if !original.has(&p) {
        t.note(format!("the original's state mapping has no {p}"));
        return t.done();
    }
    let mut entered: BTreeMap<String, usize> = BTreeMap::new();
    for f in 1..original.len() {
        let (Some(a), Some(b)) = (original.text(f - 1, &p), original.text(f, &p)) else {
            continue;
        };
        if a == b {
            let frozen = machine.states.get(b).is_some_and(|s| s.freeze);
            if frozen && let Some(g) = active(lv, f, &mut t) {
                let moved = moved(original, model, f);
                t.add(
                    g.level,
                    moved.is_none(),
                    f,
                    || format!("{b}: nothing moves"),
                    || {
                        moved
                            .clone()
                            .unwrap_or_else(|| format!("{b}: nothing moved"))
                    },
                    None,
                );
            }
            continue;
        }
        // When the state it leaves was entered, if this run saw that.
        let since = entered.get(a).copied();
        entered.insert(b.to_owned(), f);
        let Some(g) = active(lv, f, &mut t) else {
            continue;
        };
        let transition = machine
            .states
            .get(a)
            .and_then(|s| s.transition.iter().find(|tr| tr.to == b));
        match transition {
            None => t.add(
                g.level,
                false,
                f,
                || format!("no transition from {a} to {b}"),
                || format!("{a} -> {b}"),
                None,
            ),
            Some(tr) => match (tr.after, since) {
                (Some(after), Some(since)) => {
                    let took = (f - since) as u32;
                    t.add(
                        g.level,
                        takes(&machine.states[a].transition, b, took),
                        f,
                        || format!("{a} -> {b} after {after} frames"),
                        || format!("{a} -> {b} after {took} frames"),
                        None,
                    );
                }
                _ => t.add(
                    g.level,
                    true,
                    f,
                    || format!("{a} -> {b}"),
                    || format!("{a} -> {b}"),
                    None,
                ),
            },
        }
    }
    t.done()
}

/// The frames a rule's transition starts the level or the game again
/// (`restart-level`, `reset-game`, `next-level`): every entity then starts
/// its machine afresh.
/// Whether a state's transitions to `to` let it change after `took`
/// frames, as the interpreter runs them: an untimed one at any time, a
/// timed one with nothing else to wait for on its `after` frame, and one
/// that waits for something more too (a word, a button, a height, an
/// event or an alignment) on any frame from its `after` on, since it fires
/// on the first frame both hold. A counter condition (`if`) picks between
/// transitions rather than waiting, so it keeps the exact frame (Moss
/// Hop's dying state lasts 90 frames, then goes on by its lives). Donkey Kong's spin ends
/// 121 frames or more after it starts, on the next frame its step word is
/// 4 or 12 (128 frames in dk-walk-user-1 at frame 1238).
fn takes(transitions: &[gp_model::model::Transition], to: &str, took: u32) -> bool {
    transitions
        .iter()
        .filter(|tr| tr.to == to)
        .any(|tr| match tr.after {
            None => true,
            Some(after) => {
                let waits = tr.on.is_some()
                    || tr.align.is_some()
                    || tr.align_to.is_some()
                    || tr.below.is_some()
                    || tr.word.is_some()
                    || tr.pressed.is_some()
                    || tr.held.is_some();
                if waits { took >= after } else { took == after }
            }
        })
}

fn restarts(original: &Trace, model: &Model) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for (name, machine) in &model.rules {
        let p = profile::machine_path(name);
        if !original.has(&p) {
            continue;
        }
        for (from, st) in &machine.states {
            for tr in st.transition.iter().filter(|tr| {
                tr.actions
                    .iter()
                    .any(|a| matches!(a.as_str(), "restart-level" | "reset-game" | "next-level"))
            }) {
                for f in 1..original.len() {
                    if original.text(f - 1, &p) == Some(from.as_str())
                        && original.text(f, &p) == Some(tr.to.as_str())
                    {
                        out.insert(f);
                    }
                }
            }
        }
    }
    out
}

/// An entity's own machine, seen in the original: each instance's state
/// changes only along a transition (on its `after` frame when it has one),
/// or to the initial state on a frame the level or the game starts again or
/// the instance appears.
fn states_vs_model(lv: &Levels<'_>, model: &Model) -> Outcome {
    let (check, original) = (lv.check, lv.original);
    let mut t = Tally::new(check, lv.quantity, None, Against::Model);
    let Some(machine) = model
        .entities
        .get(&check.name)
        .and_then(|e| e.core.machine.as_ref())
    else {
        return t.done();
    };
    let again = restarts(original, model);
    let mut seen = false;
    for i in original.instances(&check.name) {
        let p = format!("entities.{}[{i}].state", check.name);
        if !original.has(&p) {
            continue;
        }
        seen = true;
        let alive = profile::alive_path(&check.name, i);
        let one = Fixed::from_int(1).ok();
        let mut entered: BTreeMap<String, usize> = BTreeMap::new();
        for f in 1..original.len() {
            let (Some(a), Some(b)) = (original.text(f - 1, &p), original.text(f, &p)) else {
                continue;
            };
            // A transition that removes the instance after its time: the
            // frame it goes, counted from the frame its state began.
            let gone = original.has(&alive)
                && original.num(f - 1, &alive) == one
                && original.num(f, &alive) == Some(Fixed::ZERO);
            if gone
                && let Some(after) = machine.states.get(a).and_then(|s| {
                    s.transition
                        .iter()
                        .find(|tr| tr.actions.iter().any(|x| x == "remove"))
                        .and_then(|tr| tr.after)
                })
                && let Some(since) = entered.get(a).copied()
                && let Some(g) = active(lv, f, &mut t)
            {
                let took = (f - since) as u32;
                t.add(
                    g.level,
                    took == after,
                    f,
                    || format!("{a}: removed after {after} frames"),
                    || format!("{p}: {a}: removed after {took} frames"),
                    None,
                );
            }
            if a == b {
                continue;
            }
            let since = entered.get(a).copied();
            entered.insert(b.to_owned(), f);
            let Some(g) = active(lv, f, &mut t) else {
                continue;
            };
            let appeared = original.has(&alive)
                && original.num(f - 1, &alive) != one
                && original.num(f, &alive) == one;
            if b == machine.initial && (again.contains(&f) || appeared) {
                t.add(
                    g.level,
                    true,
                    f,
                    || format!("{a} -> {b}"),
                    || format!("{a} -> {b}"),
                    None,
                );
                continue;
            }
            let tr = machine
                .states
                .get(a)
                .and_then(|s| s.transition.iter().find(|tr| tr.to == b));
            match (tr.and_then(|tr| tr.after), since) {
                _ if tr.is_none() => t.add(
                    g.level,
                    false,
                    f,
                    || format!("no transition from {a} to {b}"),
                    || format!("{p}: {a} -> {b}"),
                    None,
                ),
                (Some(after), Some(since)) => {
                    let took = (f - since) as u32;
                    t.add(
                        g.level,
                        takes(&machine.states[a].transition, b, took),
                        f,
                        || format!("{a} -> {b} after {after} frames"),
                        || format!("{p}: {a} -> {b} after {took} frames"),
                        None,
                    );
                }
                _ => t.add(
                    g.level,
                    true,
                    f,
                    || format!("{a} -> {b}"),
                    || format!("{a} -> {b}"),
                    None,
                ),
            }
        }
    }
    if !seen {
        t.note(format!(
            "the original's state mapping has no entities.{}[*].state",
            check.name
        ));
    }
    t.done()
}

/// Each instance's state the same in both runs on every frame.
fn states_same(lv: &Levels<'_>, model: &Model, subject: &Trace) -> Outcome {
    let original = lv.original;
    let mut t = Tally::new(lv.check, lv.quantity, None, Against::Original);
    let _ = model;
    let paths: Vec<String> = original
        .instances(&lv.check.name)
        .into_iter()
        .map(|i| format!("entities.{}[{i}].state", lv.check.name))
        .filter(|p| original.has(p))
        .collect();
    if paths.is_empty() {
        t.note(format!(
            "the original's state mapping has no entities.{}[*].state",
            lv.check.name
        ));
        return t.done();
    }
    for f in 0..original.len().min(subject.len()) {
        let Some(g) = active(lv, f, &mut t) else {
            continue;
        };
        for p in &paths {
            let (a, b) = (original.get(f, p), subject.get(f, p));
            let show = |v: Option<&crate::observe::Obs>| {
                v.map_or_else(|| "nothing".to_owned(), ToString::to_string)
            };
            t.add(
                g.level,
                a == b,
                f,
                || format!("{p} {}", show(a)),
                || format!("{p} {}", show(b)),
                None,
            );
        }
    }
    if t.outcome.status == Status::Pass {
        t.outcome.expected = None;
        t.outcome.actual = None;
    }
    t.done()
}

/// The first instance that moved into `frame`, if any did.
fn moved(trace: &Trace, model: &Model, frame: usize) -> Option<String> {
    for e in model.entities.keys() {
        for i in trace.instances(e) {
            for p in [profile::x_path(e, i), profile::y_path(e, i)] {
                if trace.get(frame - 1, &p) != trace.get(frame, &p) {
                    return Some(format!("{p} changed"));
                }
            }
        }
    }
    None
}

/// The frames an event happens on in the original's run, and how many
/// times on each, or why it can't be seen there.
fn event_frames(
    original: &Trace,
    model: &Model,
    event: &str,
) -> Result<BTreeMap<usize, i64>, String> {
    let mut out = BTreeMap::new();
    let mut seen = false;
    // A transition that fires on the event: the frames the state changes.
    for (name, machine) in &model.rules {
        let p = profile::machine_path(name);
        let edges: Vec<(&str, &str)> = machine
            .states
            .iter()
            .flat_map(|(s, st)| {
                st.transition
                    .iter()
                    .filter(|t| t.on.as_deref() == Some(event))
                    .map(move |t| (s.as_str(), t.to.as_str()))
            })
            .collect();
        if edges.is_empty() || !original.has(&p) {
            continue;
        }
        seen = true;
        for f in 1..original.len() {
            let (a, b) = (original.text(f - 1, &p), original.text(f, &p));
            if edges.iter().any(|(x, y)| a == Some(*x) && b == Some(*y)) {
                *out.entry(f).or_insert(0) += 1;
            }
        }
    }
    if seen {
        return Ok(out);
    }
    // An entity's own machine that moves on the event: the frames an
    // instance's state changes along that edge (a slug squashed by
    // enemy.stomped, a player whose machine starts dying on player.died).
    for (e, entity) in &model.entities {
        let Some(machine) = &entity.core.machine else {
            continue;
        };
        let edges: Vec<(&str, &str)> = machine
            .states
            .iter()
            .flat_map(|(s, st)| {
                st.transition
                    .iter()
                    .filter(|t| t.on.as_deref() == Some(event) && t.to != *s)
                    .map(move |t| (s.as_str(), t.to.as_str()))
            })
            .collect();
        if edges.is_empty() {
            continue;
        }
        for i in original.instances(e) {
            let p = format!("entities.{e}[{i}].state");
            if !original.has(&p) {
                continue;
            }
            seen = true;
            for f in 1..original.len() {
                let (a, b) = (original.text(f - 1, &p), original.text(f, &p));
                if edges.iter().any(|(x, y)| a == Some(*x) && b == Some(*y)) {
                    *out.entry(f).or_insert(0) += 1;
                }
            }
        }
    }
    if seen {
        return Ok(out);
    }
    // A stomp: an enemy that was alive isn't.
    if event == "enemy.stomped" {
        let one = Fixed::from_int(1).ok();
        for (e, entity) in &model.entities {
            if entity.core.role != "enemy" {
                continue;
            }
            for i in original.instances(e) {
                let alive = profile::alive_path(e, i);
                if !original.has(&alive) {
                    continue;
                }
                seen = true;
                for f in 1..original.len() {
                    if original.num(f - 1, &alive) == one
                        && original.num(f, &alive) == Some(Fixed::ZERO)
                    {
                        *out.entry(f).or_insert(0) += 1;
                    }
                }
            }
        }
        if seen {
            return Ok(out);
        }
    }
    // A head hit or a pickup that turns a cell into another kind: the
    // frames more of the original's cells hold that kind's byte (the
    // mapping's `[cells]`), one event for each.
    for entity in model.entities.values() {
        let Some(j) = &entity.platformer.jump else {
            continue;
        };
        let mut kinds = Vec::new();
        if let Some(h) = &j.head {
            kinds.extend(
                h.hits
                    .iter()
                    .filter(|x| x.event.as_deref() == Some(event))
                    .filter_map(|x| x.becomes),
            );
        }
        if let Some(p) = j
            .pickup
            .as_ref()
            .filter(|p| p.event.as_deref() == Some(event))
        {
            kinds.push(p.becomes);
        }
        for k in kinds {
            let p = crate::original::cells_path(k);
            if !original.has(&p) {
                continue;
            }
            seen = true;
            for f in 1..original.len() {
                if let (Some(a), Some(b)) = (original.num(f - 1, &p), original.num(f, &p))
                    && b > a
                {
                    *out.entry(f).or_insert(0) += (b.raw() - a.raw()) >> 16;
                }
            }
        }
    }
    if seen {
        return Ok(out);
    }
    Err(format!(
        "`{event}` can't be seen in the original's mapped state"
    ))
}

/// A counter, seen in the original: on each frame it changes by the `by`
/// of each event change whose event happens there (as many times as it
/// happens), kept within `min` and `max`, and on no other frame. A timed
/// change (`every`) may add its `by` on any frame: its amount is checked,
/// and its timing isn't yet.
fn counter_vs_model(lv: &Levels<'_>, model: &Model) -> Outcome {
    let (check, original) = (lv.check, lv.original);
    let mut t = Tally::new(check, lv.quantity, None, Against::Model);
    let counter = &model.counters[&check.name];
    let p = profile::counter_path(&check.name);
    if !original.has(&p) {
        t.note(format!("the original's state mapping has no {p}"));
        return t.done();
    }
    let mut on_events = Vec::new();
    let mut timed = Vec::new();
    for change in &counter.change {
        match (&change.on, change.every) {
            (Some(on), _) => match event_frames(original, model, on) {
                Ok(frames) => on_events.push((frames, change.by)),
                Err(e) => {
                    t.note(e);
                    return t.done();
                }
            },
            (None, Some(_)) => timed.push(change.by),
            (None, None) => {}
        }
    }
    // Changes the rules' actions make: `change:<counter>:<by>` and
    // `reset-game` on a transition, seen as the frames the machine's state
    // changes along that edge.
    let mut acted: BTreeMap<usize, i64> = BTreeMap::new();
    let mut resets: BTreeSet<usize> = BTreeSet::new();
    for (name, machine) in &model.rules {
        let mp = profile::machine_path(name);
        if !original.has(&mp) {
            continue;
        }
        for (from, st) in &machine.states {
            for tr in &st.transition {
                if tr.to == *from {
                    continue;
                }
                let by: i64 = tr
                    .actions
                    .iter()
                    .filter_map(|a| {
                        let mut w = a.split(':');
                        (w.next() == Some("change") && w.next() == Some(check.name.as_str()))
                            .then(|| w.next().and_then(|n| n.parse::<i64>().ok()))
                            .flatten()
                    })
                    .sum();
                let reset = tr.actions.iter().any(|a| a == "reset-game");
                if by == 0 && !reset {
                    continue;
                }
                for f in 1..original.len() {
                    if original.text(f - 1, &mp) == Some(from.as_str())
                        && original.text(f, &mp) == Some(tr.to.as_str())
                    {
                        *acted.entry(f).or_insert(0) += by;
                        if reset {
                            resets.insert(f);
                        }
                    }
                }
            }
        }
    }
    let clamp = |v: i64| wrap_or_clamp(counter.min, counter.max, counter.wrap, v);
    let whole = |v: Fixed| v.raw() >> 16;
    let mut left_to_hooks = 0u64;
    for f in 1..original.len() {
        let (Some(a), Some(b)) = (original.num(f - 1, &p), original.num(f, &p)) else {
            continue;
        };
        let delta: i64 = on_events
            .iter()
            .map(|(frames, by)| by * frames.get(&f).copied().unwrap_or(0))
            .sum::<i64>()
            + acted.get(&f).copied().unwrap_or(0);
        // Only frames with a change, an event or an action say anything.
        if a == b && delta == 0 && !resets.contains(&f) {
            continue;
        }
        let Some(g) = active(lv, f, &mut t) else {
            continue;
        };
        let base = if resets.contains(&f) {
            counter.start
        } else {
            whole(a)
        };
        let want = clamp(base + delta);
        let ok = whole(b) == want || timed.iter().any(|by| whole(b) == clamp(want + by));
        // A hook that may write the counter keeps the frames the model's
        // own changes don't explain; its value there is held against the
        // original, frame for frame, by the counter's other check.
        if !ok && !lv.hooks.is_empty() {
            left_to_hooks += 1;
            continue;
        }
        t.add(
            g.level,
            ok,
            f,
            || want.to_string(),
            || whole(b).to_string(),
            None,
        );
    }
    if left_to_hooks > 0 {
        t.note(format!(
            "{left_to_hooks} changes the model's own don't explain are left to the hooks that may write it ({})",
            lv.hooks.join(", ")
        ));
    }
    t.done()
}

/// A counter's value after a change, as the interpreter steps it
/// (`gp_sim`'s `change_counter`): past `max` it stops at `max`, or with
/// `wrap` goes round to `min` (or 0); below `min` it stops at `min`.
fn wrap_or_clamp(min: Option<i64>, max: Option<i64>, wrap: bool, v: i64) -> i64 {
    let mut v = v;
    if let Some(max) = max
        && v > max
    {
        v = if wrap {
            let lo = i128::from(min.unwrap_or(0));
            let span = (i128::from(max) - lo + 1).max(1);
            (lo + (i128::from(v) - lo).rem_euclid(span)) as i64
        } else {
            max
        };
    }
    match min {
        Some(min) if v < min => min,
        _ => v,
    }
}

type GraphMemo = (usize, Vec<((usize, Vec<bool>), Graph)>);

thread_local! {
    /// Graphs built during one [`evaluate`], by trace and frames.
    static GRAPHS: std::cell::RefCell<GraphMemo> = const { std::cell::RefCell::new((0, Vec::new())) };
}

/// The graph of the frames the check runs on, how many there were, and
/// the strictest level among them.
fn graph_of(
    lv: &Levels<'_>,
    trace: &Trace,
    model: &Model,
    t: &mut Tally,
) -> (Graph, u64, Option<Level>) {
    let levels: Vec<Option<Level>> = (0..lv.original.len())
        .map(|f| active(lv, f, t).map(|g| g.level))
        .collect();
    let frames = levels.iter().flatten().count() as u64;
    // Every level's check of a run asks for the same graph unless a `when`
    // differs, so each is built once.
    let keep: Vec<bool> = levels.iter().map(Option::is_some).collect();
    let key = (std::ptr::from_ref(trace) as usize, keep);
    let graph = GRAPHS.with(|g| {
        let mut g = g.borrow_mut();
        if g.0 != std::ptr::from_ref(model) as usize {
            *g = (std::ptr::from_ref(model) as usize, Vec::new());
        }
        if let Some((_, graph)) = g.1.iter().find(|(k, _)| *k == key) {
            return graph.clone();
        }
        let graph =
            profile::observed_graph(trace, model, &|f| key.1.get(f).copied().unwrap_or(false));
        g.1.push((key.clone(), graph.clone()));
        graph
    });
    (graph, frames, levels.iter().flatten().min().copied())
}

/// What a graph says about one level: whether it was entered, the ways
/// out of it, and the objects in it.
type LevelPart = (
    bool,
    Vec<(String, String)>,
    Option<BTreeMap<String, Option<u32>>>,
);

fn part_of(g: &Graph, level: &str) -> LevelPart {
    (
        g.levels.contains(level),
        g.edges
            .iter()
            .filter(|(a, _)| a == level)
            .cloned()
            .collect(),
        g.objects.get(level).cloned(),
    )
}

/// Frames of a run spent in a level.
fn frames_in(trace: &Trace, model: &Model, level: &str) -> u64 {
    (0..trace.len())
        .filter(|f| profile::level_at(trace, model, *f).as_deref() == Some(level))
        .count() as u64
}

fn graph_text(g: &Graph, level: &str) -> String {
    let edges: Vec<String> = g
        .edges
        .iter()
        .filter(|(a, _)| a == level)
        .map(|(_, b)| b.clone())
        .collect();
    let objects: Vec<String> = g
        .objects
        .get(level)
        .map(|o| {
            o.iter()
                .map(|(e, n)| match n {
                    Some(n) => format!("{e} x{n}"),
                    None => e.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    format!(
        "exits [{}], objects [{}]",
        edges.join(", "),
        objects.join(", ")
    )
}

/// Whether what a run shows of the level fits what the model states: the
/// run's levels and exits are the model's, and the entities the original's
/// mapping can see are the ones the model spawns, in the model's numbers
/// where it gives them. Returns the problem, or the exits not seen.
fn graph_within(
    stated: &Graph,
    seen: &Graph,
    original: &Trace,
    level: &str,
) -> (bool, Option<String>) {
    let mut problems = Vec::new();
    if !stated.levels.contains(level) {
        problems.push(format!("level {level} isn't in the model"));
    }
    // Only this level's ways out: each level's check speaks for its own.
    for (a, b) in seen
        .edges
        .difference(&stated.edges)
        .filter(|(a, _)| a == level)
    {
        problems.push(format!("the model has no way from {a} to {b}"));
    }
    let none = BTreeMap::new();
    let want = stated.objects.get(level).unwrap_or(&none);
    let got = seen.objects.get(level).unwrap_or(&none);
    // What the run shows must be in the model; a run needn't show all of
    // it, as it needn't take every exit: a run can pass an enemy before
    // the original loads it (c8f4e908 never shows w3_2's piranha plant).
    let mut not_seen = Vec::new();
    for (e, n) in want {
        if original.instances(e).is_empty() {
            continue;
        }
        // More at once than the model spawns is a difference; fewer isn't,
        // since a level that scrolls despawns some before others appear.
        match (n, got.get(e)) {
            (_, None) => not_seen.push(e.clone()),
            (Some(n), Some(Some(m))) if m > n => {
                problems.push(format!("{m} of {e} at once, and the model spawns {n}"));
            }
            _ => {}
        }
    }
    // Objects placed from lifted data can be any entity.
    let lifted = stated.lifted_objects.contains(level);
    for e in got.keys().filter(|e| !lifted && !want.contains_key(*e)) {
        problems.push(format!("{e} was seen, and the model doesn't spawn it"));
    }
    if !problems.is_empty() {
        return (false, Some(problems.join("; ")));
    }
    let unseen: Vec<String> = stated
        .edges
        .difference(&seen.edges)
        .filter(|(a, _)| a == level)
        .map(|(_, b)| b.clone())
        .collect();
    let mut notes = Vec::new();
    if !unseen.is_empty() {
        notes.push(format!(
            "exits not taken in this run: {}",
            unseen.join(", ")
        ));
    }
    if !not_seen.is_empty() {
        notes.push(format!("not seen in this run: {}", not_seen.join(", ")));
    }
    (true, (!notes.is_empty()).then(|| notes.join("; ")))
}

/// The subject's run against the original's, for one quantity.
fn against_original(lv: &Levels<'_>, model: &Model, subject: &Trace) -> Vec<Outcome> {
    let (check, q, original) = (lv.check, lv.quantity, lv.original);
    match check.kind {
        Kind::Jump => {
            let mut t = Tally::new(check, q, None, Against::Original);
            // A jump is found by the ground under the entity. With no
            // on_ground in the original's mapping its jumps can't be found,
            // and the clip scorer doesn't compare it either, so the
            // subject's own jumps would be held to none.
            let unseen = original
                .instances(&check.name)
                .iter()
                .all(|i| !original.has(&profile::on_ground_path(&check.name, *i)));
            if unseen {
                t.note(format!(
                    "not measured: the state mapping gives the original no entities.{}[N].on_ground, so its jumps can't be found",
                    check.name
                ));
                return vec![t.done()];
            }
            let ours = profile::jumps(original, model, &check.name);
            let theirs = profile::jumps(subject, model, &check.name);
            if ours.len() != theirs.len() {
                t.fail(
                    None,
                    format!(
                        "the original jumps {} times, the subject {}",
                        ours.len(),
                        theirs.len()
                    ),
                );
            }
            for (o, s) in ours.iter().zip(&theirs) {
                let Some(g) = active(lv, o.frame, &mut t) else {
                    continue;
                };
                let tol = tolerance_of(&g, q);
                let (want, got) = if q == "height" {
                    (o.height, s.height)
                } else {
                    (profile::whole(o.airtime), profile::whole(s.airtime))
                };
                t.add(
                    g.level,
                    within(want, got, tol),
                    o.frame,
                    || fixed_text(want),
                    || fixed_text(got),
                    tol,
                );
            }
            vec![t.done()]
        }
        Kind::Run => {
            let Some(run) = model
                .entities
                .get(&check.name)
                .and_then(|e| e.platformer.run.as_ref())
            else {
                return Vec::new();
            };
            let action = run.run_action.as_deref();
            let (ow, or) = profile::run_speeds(original, model, &check.name, action);
            let (sw, sr) = profile::run_speeds(subject, model, &check.name, action);
            let mut out = vec![speed_vs_original(lv, "walk", &ow, &sw, profile::mode)];
            if run.run_speed.is_some() {
                out.push(speed_vs_original(lv, "run", &or, &sr, profile::mode));
            }
            out
        }
        Kind::Patrol | Kind::Walk => {
            let o = profile::patrol_steps(original, model, &check.name);
            let s = profile::patrol_steps(subject, model, &check.name);
            let case = if check.kind == Kind::Walk {
                "walk"
            } else {
                "patrol"
            };
            vec![speed_vs_original(lv, case, &o, &s, profile::mode)]
        }
        Kind::Slots | Kind::Reach => Vec::new(),
        Kind::Camera => {
            let ours = profile::camera_leads(original, model, &check.name);
            let theirs = profile::camera_leads(subject, model, &check.name);
            vec![speed_vs_original(
                lv,
                "scrolling",
                &ours,
                &theirs,
                profile::mode,
            )]
        }
        Kind::States => vec![states_same(lv, model, subject)],
        Kind::Machine | Kind::Counter => {
            let p = if check.kind == Kind::Machine {
                profile::machine_path(&check.name)
            } else {
                profile::counter_path(&check.name)
            };
            vec![same_every_frame(lv, &p, subject)]
        }
        Kind::Level => {
            // The two runs are compared while they're on the same path:
            // up to the first frame both are in a level and not the same
            // one. A subject that leaves the original's path (a model whose
            // Mario misses a jump the recording makes) can't be held to the
            // levels the original goes on to, which the model's own graph
            // is held to instead. Objects depend on how long a run stays,
            // so only the levels entered and the ways out are compared.
            let mut t = Tally::new(check, q, None, Against::Original);
            let n = original.len().min(subject.len());
            let split = (0..n)
                .find(|&f| {
                    match (
                        profile::level_at(original, model, f),
                        profile::level_at(subject, model, f),
                    ) {
                        (Some(a), Some(b)) => a != b,
                        _ => false,
                    }
                })
                .unwrap_or(n);
            let levels: Vec<Option<Level>> = (0..n)
                .map(|f| {
                    if f < split {
                        active(lv, f, &mut t).map(|g| g.level)
                    } else {
                        None
                    }
                })
                .collect();
            let keep = |f: usize| levels.get(f).copied().flatten().is_some();
            let ours = profile::observed_graph(original, model, &keep);
            let theirs = profile::observed_graph(subject, model, &keep);
            let path = |g: &Graph| {
                let (entered, ways, _) = part_of(g, &check.name);
                (entered, ways)
            };
            let entered = |g: &Graph| g.levels.contains(&check.name);
            if !entered(&ours) && !entered(&theirs) {
                // Whether the original goes there later, past the frame
                // the runs part: then the subject left its path first.
                let later = (split..original.len()).any(|f| {
                    profile::level_at(original, model, f).as_deref() == Some(check.name.as_str())
                });
                let at = |g: &Trace, f: usize| {
                    profile::level_at(g, model, f).unwrap_or_else(|| "no level".into())
                };
                if split < n && later {
                    t.note(format!(
                        "the original enters this level after the runs part at model frame {split} (the original in {}, the subject in {})",
                        at(original, split),
                        at(subject, split)
                    ));
                    t.outcome.frame = Some(split as u64);
                } else if later {
                    t.note("the original enters this level after the subject's run ends".into());
                } else {
                    t.note("the original's run doesn't enter this level".into());
                }
            } else if let Some(level) = levels.iter().flatten().min().copied() {
                t.add(
                    level,
                    path(&ours) == path(&theirs),
                    split.min(n.saturating_sub(1)),
                    || graph_text(&ours, &check.name),
                    || graph_text(&theirs, &check.name),
                    None,
                );
                t.outcome.samples = (0..split)
                    .filter(|f| {
                        profile::level_at(original, model, *f).as_deref()
                            == Some(check.name.as_str())
                    })
                    .count() as u64;
            }
            vec![t.done()]
        }
    }
}

fn speed_vs_original(
    lv: &Levels<'_>,
    case: &str,
    original: &[(usize, Fixed)],
    subject: &[(usize, Fixed)],
    summary: fn(&[Fixed]) -> Option<Fixed>,
) -> Outcome {
    let mut t = Tally::new(lv.check, lv.quantity, Some(case), Against::Original);
    let theirs = by_level(lv, subject, &mut t);
    for (g, values, first) in by_level(lv, original, &mut t) {
        let Some(want) = summary(&values) else {
            continue;
        };
        let got = theirs
            .iter()
            .find(|(h, _, _)| *h == g)
            .and_then(|(_, vs, _)| summary(vs));
        let tol = tolerance_of(&g, lv.quantity);
        match got {
            Some(got) => t.add(
                g.level,
                within(want, got, tol),
                first,
                || fixed_text(want),
                || fixed_text(got),
                tol,
            ),
            None => t.add(
                g.level,
                false,
                first,
                || fixed_text(want),
                || "no movement".into(),
                tol,
            ),
        }
        t.outcome.samples += values.len() as u64 - 1;
    }
    t.done()
}

/// Exact state: the subject's value at `path` equals the original's on
/// every frame the check runs.
fn same_every_frame(lv: &Levels<'_>, path: &str, subject: &Trace) -> Outcome {
    let original = lv.original;
    let mut t = Tally::new(lv.check, lv.quantity, None, Against::Original);
    if !original.has(path) {
        t.note(format!("the original's state mapping has no {path}"));
        return t.done();
    }
    if original.len() != subject.len() {
        t.fail(
            None,
            format!(
                "the original ran {} frames, the subject {}",
                original.len(),
                subject.len()
            ),
        );
    }
    for f in 0..original.len().min(subject.len()) {
        let Some(g) = active(lv, f, &mut t) else {
            continue;
        };
        let (a, b) = (original.get(f, path), subject.get(f, path));
        let show = |v: Option<&crate::observe::Obs>| {
            v.map_or_else(|| "nothing".to_owned(), ToString::to_string)
        };
        t.add(g.level, a == b, f, || show(a), || show(b), None);
    }
    if t.outcome.status == Status::Pass {
        t.outcome.expected = None;
        t.outcome.actual = None;
    }
    t.done()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wrapping counter goes round past `max` as the interpreter's does:
    /// Donkey Kong's divider (0 to 10, up one a frame) failed its check
    /// at every wrap, 10 + 1 read as 11, not 0.
    #[test]
    fn a_wrapping_counter_goes_round_past_its_max() {
        assert_eq!(wrap_or_clamp(Some(0), Some(10), true, 11), 0);
        assert_eq!(wrap_or_clamp(None, Some(10), true, 13), 2);
        assert_eq!(wrap_or_clamp(Some(1), Some(3), true, 5), 2);
        assert_eq!(wrap_or_clamp(Some(0), Some(10), false, 11), 10);
        assert_eq!(wrap_or_clamp(Some(0), Some(10), true, -1), 0);
        assert_eq!(wrap_or_clamp(None, None, true, 99), 99);
    }

    /// A counter a hook may write is held to the model's own changes only
    /// where they explain it; the rest is the hook's, held against the
    /// original frame for frame. Donkey Kong's hooks keep 50 counters no
    /// change of the model's names (walk_pose, the barrels' tiles), each of
    /// which failed against the model on its first change.
    #[test]
    fn a_counter_a_hook_writes_is_left_to_it() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop");
        let model =
            Model::parse(&std::fs::read_to_string(dir.join("model.toml")).unwrap()).unwrap();
        let contract = gp_contract::Contract::parse(
            &std::fs::read_to_string(dir.join("fidelity.toml")).unwrap(),
        )
        .unwrap();
        let mut plan = crate::compile::compile(&model, &contract).unwrap();
        plan.checks.retain(|c| c.part == "counters.lives");
        assert_eq!(plan.checks.len(), 1);
        let mut t = Trace::new(vec!["counters.lives".into()]);
        let n = |v: i64| Some(crate::observe::Obs::Num(Fixed::from_int(v).unwrap()));
        for v in [2, 2, 7, 7] {
            t.push(vec![n(v)], 0);
        }
        let vs_model = |plan: &Plan| {
            evaluate(plan, &model, &t, None)
                .into_iter()
                .find(|o| o.against == Against::Model)
                .unwrap()
        };
        let o = vs_model(&plan);
        assert_eq!((o.status, o.frame), (Status::Fail, Some(2)));
        plan.kept_by_hooks
            .insert("lives".into(), vec!["a_hook".into()]);
        let o = vs_model(&plan);
        assert_eq!(o.status, Status::Unexercised);
        assert!(
            o.detail
                .unwrap()
                .contains("left to the hooks that may write it (a_hook)")
        );
    }

    #[test]
    fn a_guarded_timed_transition_fires_from_its_after_on() {
        let tr = |after: u32, word: bool| {
            let mut t: gp_model::model::Transition =
                toml::from_str(&format!("to = \"b\"\nafter = {after}")).unwrap();
            if word {
                t.word = toml::from_str("index = 0\nat_least = 4\nbelow = 5").ok();
                assert!(t.word.is_some());
            }
            t
        };
        assert!(takes(&[tr(121, false)], "b", 121));
        assert!(!takes(&[tr(121, false)], "b", 128));
        assert!(takes(&[tr(121, true)], "b", 128));
        assert!(!takes(&[tr(121, true)], "b", 120));
        assert!(!takes(&[tr(121, true)], "c", 128));
        // Moss Hop's dying state goes on by its lives after 90 frames.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop");
        let model =
            Model::parse(&std::fs::read_to_string(dir.join("model.toml")).unwrap()).unwrap();
        let dying = &model.rules["game"].states["dying"].transition;
        assert!(takes(dying, "playing", 90), "{dying:?}");
        assert!(!takes(dying, "playing", 100), "{dying:?}");
    }

    #[test]
    fn tolerances() {
        let f = |s: &str| Fixed::parse(s).unwrap();
        let rel = Tolerance::Relative(f("5"));
        assert!(within(f("47.5"), f("47.5"), None));
        assert!(!within(f("47.5"), f("47"), None));
        // 5% of 47.5 is 2.375.
        assert!(within(f("47.5"), f("45.125"), Some(&rel)));
        assert!(!within(f("47.5"), f("45"), Some(&rel)));
        assert!(within(f("-2"), f("-2.0625"), Some(&rel)));
        let abs = Tolerance::Absolute(f("2"));
        assert!(within(f("39"), f("41"), Some(&abs)));
        assert!(!within(f("39"), f("41.5"), Some(&abs)));
    }

    #[test]
    fn a_pickups_event_is_one_more_cell_of_the_kind_it_leaves() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../templates/mosshop/model.toml"),
        )
        .unwrap();
        let model = Model::parse(&text).unwrap();
        let empty = crate::original::cells_path(gp_model::model::TileKind::Empty);
        let solid = crate::original::cells_path(gp_model::model::TileKind::Solid);
        let mut t = Trace::new(vec![empty.clone(), solid.clone()]);
        let n = |v: i64| Some(crate::observe::Obs::Num(Fixed::from_int(v).unwrap()));
        for (e, s) in [(300, 40), (301, 40), (301, 41), (299, 40)] {
            t.push(vec![n(e), n(s)], 0);
        }
        // A coin taken on frame 1, the block used on frame 2; frame 3 is
        // a restart that puts both back, which is no event.
        assert_eq!(
            event_frames(&t, &model, "coin.touched").unwrap(),
            [(1, 1)].into_iter().collect()
        );
        assert_eq!(
            event_frames(&t, &model, "block.coin").unwrap(),
            [(2, 1)].into_iter().collect()
        );
    }

    #[test]
    fn conditions_read_the_original() {
        let mut t = Trace::new(vec![
            "world".into(),
            "entities.slug[1].x".into(),
            "rules.game.state".into(),
        ]);
        t.push(
            vec![
                Some(crate::observe::Obs::Num(Fixed::from_int(1).unwrap())),
                Some(crate::observe::Obs::Num(Fixed::parse("7.5").unwrap())),
                Some(crate::observe::Obs::Text("playing".into())),
            ],
            0,
        );
        let c = |s: &str| Condition::parse(s).unwrap();
        assert!(holds(&c("world == 1"), &t, 0).unwrap());
        assert!(holds(&c("entities.slug[1].x == 7"), &t, 0).unwrap());
        assert!(!holds(&c("world == 1 && entities.slug[1].x > 7"), &t, 0).unwrap());
        assert!(
            holds(&c("rules.game.state == 0"), &t, 0)
                .unwrap_err()
                .contains("name")
        );
        assert!(
            holds(&c("lives > 0"), &t, 0)
                .unwrap_err()
                .contains("isn't in")
        );
    }
}
