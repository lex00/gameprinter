//! Slot checks ([`Kind::Slots`]): enemies the original keeps in reused
//! object slots, each slot life compared with one body of its entity by
//! gp-interp's `compare_slots` (docs/formats/state-mapping.md, "Slots").
//!
//! The body steps beside the slot with the player held where the
//! original's is, so these checks see every enemy a recording meets, where
//! a side-by-side run soon leaves the recording's path. They're held
//! against the original: the body is the model.
//!
//! - `speed`: on each field the entity's components move (or every mapped
//!   field, for a hook), the most common speed over
//!   [`profile::WINDOW_FRAMES`] steps of a life, on both sides over the
//!   same steps, in whole pixels: for a hook, whose speed can change every
//!   step and that doesn't walk, or a hop's `y`, the mean of them all;
//!   otherwise the most common. As for a walk
//!   or a patrol, windows without movement are
//!   left out, so a stomped or kicked enemy, or one the game holds still,
//!   doesn't count against its walking speed.
//! - `period`: the steps between every second turn back, as
//!   `compare_slots` measures it, on `x` and `y`, for entities that turn
//!   by themselves (a `path`, or a hook that doesn't walk), held on a
//!   recording where the original turns back. A walker turns where it meets other enemies, which
//!   a body alone doesn't, so its turns aren't the model's.
//!
//! An entity a hook moves whose hook drew a random byte in the run
//! (`SlotReport::drew`) is left unmeasured: the body draws its own bytes,
//! not the original's, so its choices aren't the slot's.

use std::collections::{BTreeMap, BTreeSet};

use gp_contract::Level;
use gp_interp::compare::{SlotReport, SlotRow};
use gp_model::{Fixed, Model, platformer::PathAxis};

use crate::compile::{Check, Kind, Plan};
use crate::evaluate::{Against, Outcome, Status, within};
use crate::profile;

/// A jump of more than this many pixels in one window is a wrap or a
/// move, not motion, as `compare_slots` treats it.
const WRAP_PIXELS: i64 = 128;

/// Every compared value `compare_slots` traced, by entity, life and field:
/// (the body's frame, the original's raw value, the model's).
#[derive(Debug, Default)]
pub struct Rows {
    lives: BTreeMap<LifeField, Vec<(u64, i64, i64)>>,
}

/// An entity, one of its lives, and a compared field.
type LifeField = (String, u64, String);

impl Rows {
    pub fn push(&mut self, row: &SlotRow<'_>) {
        self.lives
            .entry((row.entity.to_owned(), row.life, row.field.to_owned()))
            .or_default()
            .push((row.frame, row.original.raw(), row.model.raw()));
    }

    /// Both sides' window speeds of one entity's field over all its lives.
    /// With `still`, windows without movement count too, as zero.
    fn speeds(&self, entity: &str, field: &str, still: bool) -> (Vec<Fixed>, Vec<Fixed>) {
        let w = profile::WINDOW_FRAMES;
        let (mut original, mut model) = (Vec::new(), Vec::new());
        for ((e, _, f), rows) in &self.lives {
            if e != entity || f != field {
                continue;
            }
            for i in w..rows.len() {
                let (a, b) = (rows[i - w], rows[i]);
                // Only steps in a row: a life the game held still between
                // steps (TimerControl) restarts its windows.
                if b.0 - a.0 != w as u64 {
                    continue;
                }
                // In whole pixels on both sides: a slot's position can be
                // whole pixels only (Super Mario Bros.' y has no fraction
                // byte), and 0.9375 a frame then moves 4 pixels in most
                // windows, not 3.75.
                for (side, from, to) in [(&mut original, a.1, b.1), (&mut model, a.2, b.2)] {
                    let d = ((to >> 16) - (from >> 16)).abs();
                    if (d != 0 || still) && d <= WRAP_PIXELS {
                        side.push(Fixed::from_raw((d << 16) / w as i64));
                    }
                }
            }
        }
        (original, model)
    }
}

/// The fields of an entity whose speed is held: those its components move,
/// or every field the slots map for an entity with a hook.
fn fields_of(model: &Model, entity: &str, report: &SlotReport) -> Vec<String> {
    let Some(e) = model.entities.get(entity) else {
        return Vec::new();
    };
    let mapped: Vec<String> = report.fields.iter().map(|f| f.path.clone()).collect();
    if e.core.ext.is_some() {
        return mapped;
    }
    let p = &e.platformer;
    let mut want = BTreeSet::new();
    if p.walk.is_some() || p.fly.is_some() || p.patrol.is_some() {
        want.insert("x");
    }
    if p.hop.is_some() {
        want.insert("y");
    }
    if let Some(path) = &p.path {
        want.insert(match path.axis {
            PathAxis::X => "x",
            PathAxis::Y => "y",
        });
    }
    mapped
        .into_iter()
        .filter(|f| want.contains(f.as_str()))
        .collect()
}

/// Whether an entity moves by an `ext` hook rather than walking: a walker
/// with a hook (Super Mario Bros.' spiny, for its egg) is a walker.
fn hook_of(model: &Model, entity: &str) -> bool {
    model
        .entities
        .get(entity)
        .is_some_and(|e| e.core.ext.is_some() && e.platformer.walk.is_none())
}

/// Whether an entity hops (`hop`): its `y` speed changes every frame.
fn hops(model: &Model, entity: &str) -> bool {
    model
        .entities
        .get(entity)
        .is_some_and(|e| e.platformer.hop.is_some())
}

/// Whether an entity turns back by itself, so its period is the model's:
/// a path, or a hook that doesn't walk.
fn turns_by_itself(model: &Model, entity: &str) -> bool {
    hook_of(model, entity)
        || model
            .entities
            .get(entity)
            .is_some_and(|e| e.platformer.path.is_some())
}

/// The fields a period is held on: the position's. A hook's own words
/// (`ext[i]`) are its state, whose turns aren't a rhythm the player sees.
fn turning_field(field: &str) -> bool {
    field == "x" || field == "y"
}

fn blank(check: &Check, quantity: &str) -> Outcome {
    Outcome {
        part: check.part.clone(),
        kind: check.kind.name().to_owned(),
        quantity: quantity.to_owned(),
        case: None,
        against: Against::Original,
        level: "none".into(),
        status: Status::Unexercised,
        samples: 0,
        expected: None,
        actual: None,
        tolerance: None,
        frame: None,
        detail: None,
    }
}

/// The slot checks of a plan left unmeasured on one recording, each saying
/// `why`: what a runner that can't step the bodies reports.
pub fn unmeasured(plan: &Plan, model: &Model, why: &str) -> Vec<Outcome> {
    let mut out = Vec::new();
    for check in plan.checks.iter().filter(|c| c.kind == Kind::Slots) {
        for quantity in check.kind.quantities() {
            if *quantity == "period" && !turns_by_itself(model, &check.name) {
                continue;
            }
            let (base, conditional) = &check.levels[quantity];
            if !check.kind.checks_at(base.level) && conditional.is_empty() {
                continue;
            }
            let mut o = blank(check, quantity);
            o.detail = Some(format!("not measured: {why}"));
            out.push(o);
        }
    }
    out
}

/// The slot checks of a plan on one recording, from `compare_slots`'
/// reports and the rows it traced.
pub fn evaluate(plan: &Plan, model: &Model, reports: &[SlotReport], rows: &Rows) -> Vec<Outcome> {
    let mut out = Vec::new();
    for check in plan.checks.iter().filter(|c| c.kind == Kind::Slots) {
        let report = reports.iter().find(|r| r.entity == check.name);
        for quantity in check.kind.quantities() {
            if *quantity == "period" && !turns_by_itself(model, &check.name) {
                continue;
            }
            let (base, conditional) = &check.levels[quantity];
            if !check.kind.checks_at(base.level) && conditional.is_empty() {
                continue;
            }
            let mut o = blank(check, quantity);
            if !conditional.is_empty() {
                o.status = Status::Fail;
                o.detail = Some(
                    "a rule with a `when` governs it, which slot checks don't read yet".into(),
                );
                out.push(o);
                continue;
            }
            let Some(report) = report.filter(|r| r.lives > 0) else {
                // A period is held where the original turns back, which a
                // recording without the entity doesn't say.
                if *quantity == "period" {
                    continue;
                }
                o.detail = Some("no slot holds it in this recording".into());
                out.push(o);
                continue;
            };
            // A hook that draws random bytes chooses by them (Donkey Kong's
            // barrels at a ladder and after a fall), and the body beside the
            // slot draws its own, not the original's, so its path and its
            // turns aren't the slot's. The clip scorer, which gives the
            // rebuild the original's draws, holds it frame by frame instead.
            if hook_of(model, &check.name) && report.drew {
                o.detail = Some(format!(
                    "not measured: the {} hook draws random bytes, and the body beside the slot draws its own, not the original's; the clip scores hold it on the original's draws",
                    check.name
                ));
                out.push(o);
                continue;
            }
            let tol = match base.level {
                Level::Metric => base.tolerance.get(*quantity),
                _ => None,
            };
            let (mut expected, mut actual, mut failed) = (Vec::new(), Vec::new(), Vec::new());
            for field in fields_of(model, &check.name, report) {
                if *quantity == "period" && !turning_field(&field) {
                    continue;
                }
                let (theirs, mine) = if *quantity == "speed" {
                    // A hook's speed can change every step (a lift slowing
                    // to turn), so its mean is held; a component's walking
                    // speed is the most common one.
                    // So is a hop's height, which changes every frame.
                    let hooked =
                        hook_of(model, &check.name) || (field == "y" && hops(model, &check.name));
                    let (a, b) = rows.speeds(&check.name, &field, hooked);
                    o.samples += a.len() as u64;
                    let summary = if hooked { profile::mean } else { profile::mode };
                    let (a, b) = (summary(&a), summary(&b));
                    // All still is no speed.
                    let moving = |v: Option<Fixed>| v.filter(|v| *v != Fixed::ZERO);
                    (moving(a), moving(b))
                } else {
                    let f = report.fields.iter().find(|f| f.path == field);
                    let p = |m: &gp_interp::compare::Measure| {
                        m.period().map(|p| Fixed::from_raw(p.raw()))
                    };
                    let (a, b) = f.map_or((None, None), |f| (p(&f.original), p(&f.model)));
                    o.samples += f.map_or(0, |f| f.original.turns);
                    (a, b)
                };
                let text = |v: Option<Fixed>| match v {
                    Some(v) => format!("{field} {v}"),
                    None if *quantity == "speed" => format!("{field} still"),
                    None => format!("{field} never turns"),
                };
                if theirs.is_none() && mine.is_none() {
                    continue;
                }
                // A period the original never shows isn't held; a speed
                // only one side shows is a difference.
                if *quantity == "period" && theirs.is_none() {
                    continue;
                }
                expected.push(text(theirs));
                actual.push(text(mine));
                let ok = matches!((theirs, mine), (Some(a), Some(b)) if within(a, b, tol));
                if !ok {
                    failed.push(field.clone());
                }
            }
            if expected.is_empty() {
                // An entity that never turns back here (a firebar spins
                // one way) has no period to hold.
                if *quantity == "period" {
                    continue;
                }
                o.detail = Some(format!(
                    "{} lives, {} frames, none {}",
                    report.lives,
                    report.frames,
                    if *quantity == "speed" {
                        "moving"
                    } else {
                        "turning back"
                    }
                ));
                out.push(o);
                continue;
            }
            o.level = base.level.name().to_owned();
            o.status = if failed.is_empty() {
                Status::Pass
            } else {
                Status::Fail
            };
            o.expected = Some(expected.join(", "));
            o.actual = Some(actual.join(", "));
            o.tolerance = tol.map(ToString::to_string);
            o.detail = Some(format!(
                "{} lives, {} frames, beside the original's slots{}",
                report.lives,
                report.frames,
                if failed.is_empty() {
                    String::new()
                } else {
                    format!("; off: {}", failed.join(", "))
                }
            ));
            out.push(o);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(entity: &'a str, life: u64, frame: u64, original: i64, model: i64) -> SlotRow<'a> {
        SlotRow {
            entity,
            life,
            frame,
            field: "x",
            original: gp_sim::Fx::from_raw(original),
            model: gp_sim::Fx::from_raw(model),
            words: [0; 4],
        }
    }

    #[test]
    fn window_speeds_leave_out_stops_and_gaps() {
        let mut rows = Rows::default();
        // Half a pixel a frame on both sides, then the original stands
        // still (stomped) while the body walks on, then a gap.
        for f in 0..12u64 {
            let o = f.min(4) as i64 * 32768;
            rows.push(&row("goomba", 1, f, o, f as i64 * 32768));
        }
        rows.push(&row("goomba", 1, 40, 0, 0));
        let (a, b) = rows.speeds("goomba", "x", false);
        assert_eq!(profile::mode(&a), Some(Fixed::from_raw(32768)));
        assert_eq!(profile::mode(&b), Some(Fixed::from_raw(32768)));
        // The original's windows that end standing still are left out; the
        // step after the gap starts no window.
        assert_eq!(a.len(), 4);
        assert_eq!(b.len(), 8);
    }

    /// A hooked entity whose hook drew in the run is left unmeasured: the
    /// body beside the slot drew its own bytes. Donkey Kong's barrels
    /// choose ladders by the random byte, and their speed and period
    /// failed against the original's slots on every recording.
    #[test]
    fn a_hook_that_draws_is_left_unmeasured() {
        // The games checkout beside this one; skipped without it.
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/donkey-kong");
        if !dir.join("model.toml").is_file() {
            return;
        }
        let model =
            Model::parse(&std::fs::read_to_string(dir.join("model.toml")).unwrap()).unwrap();
        let contract = gp_contract::Contract::parse(
            &std::fs::read_to_string(dir.join("fidelity.toml")).unwrap(),
        )
        .unwrap();
        let slotted = BTreeSet::from(["barrel".to_owned()]);
        let plan = crate::compile::compile_with_slots(&model, &contract, &slotted).unwrap();
        assert!(hook_of(&model, "barrel"));
        let report = |drew: bool| SlotReport {
            entity: "barrel".into(),
            lives: 1,
            frames: 10,
            drew,
            fields: Vec::new(),
        };
        let unmeasured = |o: &Outcome| {
            o.status == Status::Unexercised
                && o.detail
                    .as_deref()
                    .is_some_and(|d| d.starts_with("not measured"))
        };
        let drew = evaluate(&plan, &model, &[report(true)], &Rows::default());
        assert!(!drew.is_empty() && drew.iter().all(unmeasured), "{drew:?}");
        let kept = evaluate(&plan, &model, &[report(false)], &Rows::default());
        assert!(!kept.iter().any(unmeasured), "{kept:?}");
    }
}
