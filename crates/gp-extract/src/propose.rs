//! Measured feel as proposed model facts (docs/mechanics.md, "Proposals"):
//! each measured parameter by its model path, rounded to its grid, with the
//! samples behind it, the fact the model states now, whether the two agree
//! within the quantity's tolerance, and the evidence it would be written
//! with. Where the samples disagree among themselves, or a confident
//! measurement disagrees with a stated fact, or a track can't be told to
//! be one entity, a [`Question`] goes to the person instead.

use std::collections::BTreeMap;

use gp_model::path::{Path, Step};
use gp_model::{Fixed, Model};
use serde::Serialize;

use crate::feel::{self, Feel, round_to};
use crate::questions::{Candidate, Question};

/// One proposed fact.
#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub path: String,
    /// The mechanic record it's a parameter of.
    pub mechanic: String,
    pub unit: &'static str,
    /// Rounded to the quantity's grid, as the model would hold it.
    pub measured: String,
    /// Before rounding.
    pub raw: f64,
    pub samples: usize,
    /// The largest minus the smallest sample.
    pub spread: f64,
    /// The model's fact now, if it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stated: Option<String>,
    /// Whether the measured and stated values agree within `tolerance`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agrees: Option<bool>,
    pub tolerance: String,
    /// The confidence the evidence would carry, 0 to 100.
    pub confidence: u8,
    /// Whether it may be written without asking: its samples agree and
    /// nothing contradicts it.
    pub settled: bool,
    pub note: String,
    /// The recordings measured, by short ID.
    pub recordings: Vec<String>,
}

/// Proposals and questions from one recording.
#[derive(Debug, Clone, Serialize)]
pub struct Proposed {
    pub recording: String,
    pub proposals: Vec<Proposal>,
    pub questions: Vec<Question>,
    /// Jumps by frames held: the rise each gave.
    pub rise_by_hold: Vec<(u32, i64)>,
    /// Accuracy against the stated facts: proposals with a stated value,
    /// how many agree, and the mean relative error.
    pub accuracy: Accuracy,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Accuracy {
    pub compared: usize,
    pub agree: usize,
    pub mean_relative_error: f64,
}

/// How far a measured value may be from a stated one and still agree.
fn tolerance(unit: &str, stated: f64) -> (f64, String) {
    let rel = |p: f64, abs: f64| {
        (
            abs.max(stated.abs() * p),
            format!("{abs} or {}%", p * 100.0),
        )
    };
    match unit {
        "px/frame" => rel(0.03, 1.0 / 16.0),
        "px/frame^2" => rel(0.10, 1.0 / 64.0),
        "frames" => (0.0, "0".into()),
        "px" => (0.5, "0.5".into()),
        _ => rel(0.25, 0.0),
    }
}

/// The value at a concrete path in the model's tree, as a number.
fn stated_at(tree: &toml::Value, path: &str) -> Option<f64> {
    let p = Path::parse(path).ok()?;
    let mut node = tree;
    for s in &p.segments {
        node = node.get(&s.name)?;
        if let Some(gp_model::path::Index::At(i)) = s.index {
            node = node.get(i as usize)?;
        }
    }
    match node {
        toml::Value::Integer(i) => Some(*i as f64),
        toml::Value::String(s) => Fixed::parse(s).ok().map(|f| f.raw() as f64 / 65536.0),
        _ => None,
    }
}

/// `v` as the model writes a fixed-point number.
pub fn fixed_text(v: f64) -> String {
    Fixed::from_raw((v * 65536.0).round() as i64).to_string()
}

struct Builder<'a> {
    tree: toml::Value,
    recording: &'a str,
    proposals: Vec<Proposal>,
    questions: Vec<Question>,
}

impl Builder<'_> {
    /// Proposes `path` from `samples` (already in the quantity's unit).
    fn add(
        &mut self,
        path: String,
        mechanic: &str,
        unit: &'static str,
        samples: &[f64],
        what: &str,
        settle: &str,
    ) {
        if samples.is_empty() {
            return;
        }
        let den = feel::grid(unit);
        let mut sorted: Vec<f64> = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let raw = sorted[sorted.len() / 2];
        let measured = round_to(raw, den);
        let spread = sorted[sorted.len() - 1] - sorted[0];
        // Samples by rounded value: two values each with a third of them
        // are a conflict.
        let mut by: BTreeMap<i64, usize> = BTreeMap::new();
        for s in samples {
            *by.entry((round_to(*s, den) * den as f64).round() as i64)
                .or_insert(0) += 1;
        }
        let strong: Vec<(i64, usize)> = by
            .iter()
            .filter(|(_, n)| **n * 3 >= samples.len() && samples.len() >= 3)
            .map(|(v, n)| (*v, *n))
            .collect();
        let conflict = strong.len() >= 2;
        let stated = stated_at(&self.tree, &path);
        let (tol, tol_text) = tolerance(unit, stated.unwrap_or(measured));
        let agrees = stated.map(|s| (s - measured).abs() <= tol + 1e-9);
        let confidence = match (conflict, samples.len()) {
            (true, _) => 50,
            (false, 1..=2) => 70,
            _ => 90,
        };
        let note = format!(
            "gp model propose: {what}, {} samples in recording {}, median {raw:.4}",
            samples.len(),
            self.recording
        );
        let mut settled = !conflict;
        if conflict {
            self.questions.push(Question {
                id: format!("conflict:{path}"),
                kind: "conflict".into(),
                path: path.clone(),
                text: format!("The samples of {path} disagree: they round to {} values. Which is the mechanic's?", strong.len()),
                settle: settle.into(),
                candidates: strong
                    .iter()
                    .map(|(v, n)| Candidate {
                        value: fixed_text(*v as f64 / den as f64),
                        from: format!("{n} of {} samples in recording {}", samples.len(), self.recording),
                        refs: Vec::new(),
                    })
                    .collect(),
                answer: None,
            });
        } else if agrees == Some(false) && confidence >= 90 {
            settled = false;
            self.questions.push(Question {
                id: format!("disagree:{path}"),
                kind: "disagree".into(),
                path: path.clone(),
                text: format!(
                    "{path} is {} in the model, and recording {} measures {}. Keep the model's or take the measurement?",
                    fixed_text(stated.unwrap_or_default()),
                    self.recording,
                    fixed_text(measured)
                ),
                settle: format!("{settle}; or the game's code at the fact's evidence ref"),
                candidates: vec![
                    Candidate {
                        value: fixed_text(stated.unwrap_or_default()),
                        from: "the model's fact and its evidence".into(),
                        refs: Vec::new(),
                    },
                    Candidate {
                        value: fixed_text(measured),
                        from: format!("measured: {} samples in recording {}", samples.len(), self.recording),
                        refs: Vec::new(),
                    },
                ],
                answer: None,
            });
        }
        self.proposals.push(Proposal {
            path,
            mechanic: mechanic.into(),
            unit,
            measured: fixed_text(measured),
            raw,
            samples: samples.len(),
            spread,
            stated: stated.map(fixed_text),
            agrees,
            tolerance: tol_text,
            confidence,
            settled,
            note,
            recordings: vec![self.recording.to_owned()],
        });
    }
}

/// The player entity: the one with role `player`.
pub fn player_of(model: &Model) -> Option<&str> {
    model
        .entities
        .iter()
        .find(|(_, e)| e.core.role == "player")
        .map(|(n, _)| n.as_str())
}

/// Proposals from a recording's measured feel. `ids` names the entity of
/// each object ID, from the state mapping's slot kinds; a track without
/// one is the model's only moving enemy, or a question.
pub fn propose(
    model: &Model,
    feel: &Feel,
    recording: &str,
    ids: &BTreeMap<u8, String>,
) -> Proposed {
    let mut b = Builder {
        tree: model.to_tree(),
        recording,
        proposals: Vec::new(),
        questions: Vec::new(),
    };
    if let Some(player) = player_of(model) {
        let e = &model.entities[player];
        let at = format!("entities.{player}");
        if let Some(run) = &e.platformer.run {
            let has_run = run.run_action.is_some();
            for (running, field) in [(false, "walk_speed"), (true, "run_speed")] {
                if running && !has_run {
                    continue;
                }
                let rs: Vec<&feel::Run> =
                    feel.runs.iter().filter(|r| r.running == running).collect();
                let top = rs.iter().map(|r| r.speed).fold(0.0, f64::max);
                // Runs much slower than the fastest were blocked.
                let speeds: Vec<f64> = rs
                    .iter()
                    .map(|r| r.speed)
                    .filter(|s| *s >= top / 2.0 && *s > 0.0)
                    .collect();
                let held = if running {
                    "right or left with run"
                } else {
                    "right or left"
                };
                b.add(
                    format!("{at}.run.{field}"),
                    "platformer.run",
                    "px/frame",
                    &speeds,
                    &format!("ground runs holding {held}"),
                    &format!("a run of at least 64 frames on flat ground holding {held}"),
                );
                let accel_field = if running {
                    "run_acceleration"
                } else {
                    "acceleration"
                };
                let stated = if running {
                    run.run_acceleration.is_some()
                } else {
                    run.acceleration.is_some()
                };
                let accels: Vec<f64> = rs
                    .iter()
                    .filter(|r| r.from_standing && r.speed >= top / 2.0)
                    .filter_map(|r| r.acceleration)
                    .collect();
                let instant = rs
                    .iter()
                    .filter(|r| r.from_standing && r.acceleration.is_none() && r.speed >= top / 2.0)
                    .count();
                if stated || accels.len() > instant {
                    b.add(
                        format!("{at}.run.{accel_field}"),
                        "platformer.run",
                        "px/frame^2",
                        &accels,
                        "the distance runs from a standing start fall short of top speed",
                        &format!("a run from standing still holding {held} for at least 64 frames"),
                    );
                }
            }
        }
        if let Some(jump) = &e.platformer.jump {
            let held =
                jump.gravity_held.is_some() || jump.bands.iter().any(|b| b.gravity_held.is_some());
            // A jump's band, by its horizontal speed at takeoff.
            let band = |vx: f64| {
                jump.bands
                    .iter()
                    .position(|b| vx < b.below.raw() as f64 / 65536.0)
            };
            let mut groups: BTreeMap<Option<usize>, Vec<&feel::Jump>> = BTreeMap::new();
            for j in &feel.jumps {
                groups.entry(band(j.takeoff_vx)).or_default().push(j);
            }
            for (band, js) in groups {
                let base = match band {
                    Some(i) => format!("{at}.jump.bands[{i}]"),
                    None => format!("{at}.jump"),
                };
                let pick = |f: fn(&feel::Jump) -> Option<f64>| {
                    js.iter().filter_map(|j| f(j)).collect::<Vec<f64>>()
                };
                let settle =
                    "jumps from standing still, some held to the top and some let go at once";
                if held {
                    b.add(
                        format!("{base}.speed"),
                        "platformer.jump",
                        "px/frame",
                        &pick(|j| j.speed_held),
                        "the held part of each jump's arc, fitted",
                        settle,
                    );
                    b.add(
                        format!("{base}.gravity_held"),
                        "platformer.jump",
                        "px/frame^2",
                        &pick(|j| j.gravity_held),
                        "the held part of each jump's arc, fitted",
                        settle,
                    );
                    b.add(
                        format!("{base}.gravity"),
                        "platformer.jump",
                        "px/frame^2",
                        &pick(|j| j.gravity),
                        "the released part of each jump's arc, fitted",
                        settle,
                    );
                } else {
                    b.add(
                        format!("{base}.speed"),
                        "platformer.jump",
                        "px/frame",
                        &pick(|j| j.speed),
                        "each jump's arc, fitted",
                        settle,
                    );
                    b.add(
                        format!("{base}.gravity"),
                        "platformer.jump",
                        "px/frame^2",
                        &pick(|j| j.gravity),
                        "each jump's arc, fitted",
                        settle,
                    );
                }
            }
            let falls: Vec<f64> = feel.jumps.iter().filter_map(|j| j.max_fall).collect();
            b.add(
                format!("{at}.jump.max_fall"),
                "platformer.jump",
                "px/frame",
                &falls,
                "the steepest fall of each jump",
                "a jump down from a high ledge, falling for at least 30 frames",
            );
        }
    }

    // Enemies, by entity.
    let movers: Vec<&String> = model
        .entities
        .iter()
        .filter(|(_, e)| {
            e.core.role != "player"
                && (e.platformer.patrol.is_some()
                    || e.platformer.walk.is_some()
                    || e.platformer.fly.is_some())
        })
        .map(|(n, _)| n)
        .collect();
    let mut by: BTreeMap<String, Vec<&feel::Enemy>> = BTreeMap::new();
    for t in &feel.enemies {
        let entity =
            t.id.and_then(|i| ids.get(&i).cloned())
                .or_else(|| (movers.len() == 1).then(|| movers[0].clone()));
        match entity {
            Some(e) => by.entry(e).or_default().push(t),
            None if !movers.is_empty() => {
                let id = format!("which-entity:track{}", t.track);
                if !b.questions.iter().any(|q| q.id == id) {
                    b.questions.push(Question {
                        id,
                        kind: "which-entity".into(),
                        path: "entities".into(),
                        text: format!(
                            "Track {} (palette {}, id {:?}) moves {:.3} px a frame. Which entity is it?",
                            t.track, t.palette, t.id, t.speed
                        ),
                        settle: "the state mapping's [[slots.kind]] naming its object ID".into(),
                        candidates: movers
                            .iter()
                            .map(|m| Candidate {
                                value: (*m).clone(),
                                from: "a moving enemy of the model".into(),
                                refs: Vec::new(),
                            })
                            .collect(),
                        answer: None,
                    });
                }
            }
            None => {}
        }
    }
    for (entity, ts) in by {
        let Some(e) = model.entities.get(&entity) else {
            continue;
        };
        let at = format!("entities.{entity}");
        let settle = "a longer recording where it walks on open ground without turning";
        if e.platformer.patrol.is_some() {
            let steps: Vec<f64> = ts.iter().filter_map(|t| t.step).map(|s| s as f64).collect();
            let periods: Vec<f64> = ts.iter().filter_map(|t| t.period).map(f64::from).collect();
            b.add(
                format!("{at}.patrol.step"),
                "platformer.patrol",
                "px",
                &steps,
                "the most common move of each track",
                settle,
            );
            b.add(
                format!("{at}.patrol.period"),
                "platformer.patrol",
                "frames",
                &periods,
                "the most common frames between moves of each track",
                settle,
            );
        } else if e.platformer.walk.is_some() || e.platformer.fly.is_some() {
            let field = if e.platformer.walk.is_some() {
                "walk"
            } else {
                "fly"
            };
            let speeds: Vec<f64> = ts.iter().map(|t| t.speed).collect();
            b.add(
                format!("{at}.{field}.speed"),
                &format!("platformer.{field}"),
                "px/frame",
                &speeds,
                "each track's pixels over its frames",
                settle,
            );
        }
    }

    let mut acc = Accuracy::default();
    let mut err = 0.0;
    for p in &b.proposals {
        if let (Some(s), Some(a)) = (&p.stated, p.agrees) {
            acc.compared += 1;
            acc.agree += usize::from(a);
            let s = Fixed::parse(s).map_or(0.0, |f| f.raw() as f64 / 65536.0);
            if s != 0.0 {
                err += ((p.raw - s) / s).abs();
            }
        }
    }
    if acc.compared > 0 {
        acc.mean_relative_error = err / acc.compared as f64;
    }
    let rise_by_hold = feel.jumps.iter().map(|j| (j.hold, j.rise)).collect();
    Proposed {
        recording: recording.into(),
        proposals: b.proposals,
        questions: b.questions,
        rise_by_hold,
        accuracy: acc,
    }
}

/// The steps of a concrete path such as `entities.player.jump.bands[0].speed`.
pub fn steps(path: &str) -> Result<Vec<Step>, String> {
    let p = Path::parse(path)?;
    let mut out = Vec::new();
    for s in p.segments {
        out.push(Step::Key(s.name));
        match s.index {
            None => {}
            Some(gp_model::path::Index::At(i)) if i >= 0 => out.push(Step::Item(i as usize)),
            Some(_) => return Err(format!("`{path}` isn't a concrete path")),
        }
    }
    Ok(out)
}
