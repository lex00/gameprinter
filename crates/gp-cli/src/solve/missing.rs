//! Detectors for a missing body or state (docs/research/remastered-detect.md):
//! what one game has and the other never does, read from a clip's score
//! and, for states, from every clip of the run. They name no number; they
//! say which kind of thing is missing, so a round can stop asking for
//! numbers and the question can point at the thing.
//!
//! | Solver | Class | Reads | Says |
//! |---|---|---|---|
//! | `bodies` | `only_original`, `only_model` | the clip's paired bodies: a body one game has alone | "the original has a bullet bill cannon the model doesn't, from frame 5"; whether the model makes any of that kind in the clip |
//! | `states` | `unseen` | a measure of names or flags: the original's value where it is first wrong, against every value the model takes in any of the run's clips | "the original's facing is (unnamed value 3), which the rebuilt game never has in 255 clips" |
//! | `states` | `entry` | the first wrong frame, and the same body's `state` changing there in the original | "Mario's speed across parts on the frame the state turns dying_hold" |

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use gp_interp::scenario::{MeasureScore, Outcome};

/// What a body or state detector read.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// `bodies` or `states`.
    pub solver: &'static str,
    pub class: &'static str,
    /// The measure, or the body's kind.
    pub measure: String,
    /// The clip's frame (from its start).
    pub at: u64,
    /// The first thing wrong in the clip: nothing else is wrong before
    /// `at`.
    pub first: bool,
    pub says: String,
}

/// A measure's path without its slot indexes: `bodies.goomba[1].x` is
/// `bodies.goomba.x`.
pub fn key(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut skip = false;
    for c in path.chars() {
        match c {
            '[' => skip = true,
            ']' => skip = false,
            _ if !skip => out.push(c),
            _ => {}
        }
    }
    out
}

/// The entity part of a path: `entities.mario[0].vx` is
/// `entities.mario[0]`.
fn owner(path: &str) -> &str {
    path.rsplit_once('.').map_or(path, |(a, _)| a)
}

/// A measure's values as discrete names: its names, or a flag's `true`
/// and `false`. None for a number.
fn discrete(m: &MeasureScore, model: bool) -> Option<Vec<String>> {
    let names = if model {
        &m.model_names
    } else {
        &m.original_names
    };
    if !names.is_empty() {
        return Some(names.clone());
    }
    let s = if model { &m.model } else { &m.original };
    let flag = !s.is_empty() && s.iter().all(|v| v.is_empty() || v == "0" || v == "1");
    let named = m
        .first
        .as_ref()
        .is_some_and(|f| matches!(f.original.as_str(), "true" | "false"));
    (flag && named).then(|| {
        s.iter()
            .map(|v| match v.as_str() {
                "1" => "true".to_owned(),
                "0" => "false".to_owned(),
                _ => String::new(),
            })
            .collect()
    })
}

/// Every value the model takes on each discrete measure, over a run's
/// clips.
#[derive(Debug, Clone, Default)]
pub struct Seen {
    pub values: BTreeMap<String, BTreeSet<String>>,
    pub clips: usize,
}

impl Seen {
    pub fn of<'a>(outcomes: impl IntoIterator<Item = &'a Outcome>) -> Self {
        let mut s = Self::default();
        for o in outcomes {
            s.clips += 1;
            for m in &o.measures {
                if let Some(v) = discrete(m, true) {
                    s.values
                        .entry(key(&m.path))
                        .or_default()
                        .extend(v.into_iter().filter(|x| !x.is_empty()));
                }
            }
        }
        s
    }
}

/// The index of a measure's first frame past its tolerance.
fn first_index(m: &MeasureScore) -> Option<usize> {
    let tol = gp_model::Fixed::parse(&m.tolerance).map_or(0, |f| f.raw());
    m.distance
        .iter()
        .position(|d| gp_model::Fixed::parse(d).is_ok_and(|d| d.raw() > tol))
}

/// The bodies and states detectors on one scored clip. `seen` is the
/// run's; with no clips in it, `unseen` doesn't run.
pub fn findings(o: &Outcome, seen: &Seen) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let first = o.first.as_ref().map(|f| f.frame);
    let is_first = |at: u64| first.is_none_or(|f| at <= f);
    // A body one game has alone.
    for b in &o.bodies {
        if b.side == "both" {
            continue;
        }
        let at = b.first_frame.unwrap_or(b.from);
        let any = o
            .bodies
            .iter()
            .any(|x| x.kind == b.kind && x.side != b.side);
        let (class, says) = if b.side == "original" {
            (
                "only_original",
                format!(
                    "the original has a {} the model doesn't, from frame {at}{}",
                    b.kind.replace('_', " "),
                    if any {
                        ""
                    } else {
                        "; the model makes none of that kind in the clip"
                    }
                ),
            )
        } else {
            (
                "only_model",
                format!(
                    "the model has a {} the original doesn't, from frame {at}",
                    b.kind.replace('_', " ")
                ),
            )
        };
        if out.iter().any(|f| f.class == class && f.measure == b.kind) {
            continue;
        }
        out.push(Finding {
            solver: "bodies",
            class,
            measure: b.kind.clone(),
            at,
            first: is_first(at),
            says,
        });
    }
    // A value the original takes where it's wrong that the model never
    // takes in any clip.
    if seen.clips > 0 {
        for m in &o.measures {
            let (Some(i), Some(f)) = (first_index(m), m.first.as_ref()) else {
                continue;
            };
            let Some(orig) = discrete(m, false) else {
                continue;
            };
            let Some(v) = orig.get(i).filter(|v| !v.is_empty()) else {
                continue;
            };
            let k = key(&m.path);
            if seen.values.get(&k).is_some_and(|s| s.contains(v)) {
                continue;
            }
            out.push(Finding {
                solver: "states",
                class: "unseen",
                measure: m.path.clone(),
                at: f.frame,
                first: is_first(f.frame),
                says: format!(
                    "{}: the original's is {v} from frame {}, which the rebuilt game never has in {} clip{}",
                    gp_interp::lockstep::words(&m.path),
                    f.frame,
                    seen.clips,
                    if seen.clips == 1 { "" } else { "s" }
                ),
            });
        }
    }
    // The first wrong frame is a state's entry in the original.
    if let Some(fs) = o.first.as_ref()
        && let Some(m) = o.measures.iter().find(|m| m.path == fs.path)
        && let Some(i) = first_index(m)
    {
        let who = owner(&m.path);
        for s in &o.measures {
            // A machine state: a body's direction turning to none is the
            // body coming or going, which `bodies` reads.
            if s.path == m.path
                || owner(&s.path) != who
                || s.original_names.is_empty()
                || !s.path.ends_with(".state")
            {
                continue;
            }
            let n = &s.original_names;
            // Changes on the step to the wrong frame or the one before.
            let changed = (i.saturating_sub(1)..=i)
                .filter(|&j| j > 0 && j < n.len())
                .find(|&j| n[j] != n[j - 1]);
            let Some(j) = changed else { continue };
            let both = s.model_names.get(j) == n.get(j);
            out.push(Finding {
                solver: "states",
                class: "entry",
                measure: m.path.clone(),
                at: fs.frame,
                first: true,
                says: format!(
                    "{} parts on the frame the state turns {} in the original ({}): the gap is what that state does, not the measure's own mechanic",
                    gp_interp::lockstep::words(&m.path),
                    n[j],
                    if both {
                        "the rebuilt game turns too"
                    } else {
                        "the rebuilt game doesn't"
                    }
                ),
            });
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(path: &str, orig: &[&str], model: &[&str], first: Option<u64>) -> serde_json::Value {
        let d: Vec<&str> = orig
            .iter()
            .zip(model)
            .map(|(a, b)| if a == b { "0" } else { "1" })
            .collect();
        serde_json::json!({"path": path, "together": 0, "total": orig.len(), "tolerance": "0",
            "max": "1", "mean": "0", "bins": [], "owners": [], "distance": d,
            "original": orig.iter().map(|_| "").collect::<Vec<_>>(),
            "model": model.iter().map(|_| "").collect::<Vec<_>>(),
            "original_names": orig, "model_names": model,
            "first": first.map(|f| serde_json::json!({"frame": f, "original": "", "model": ""}))})
    }

    fn outcome(
        measures: Vec<serde_json::Value>,
        bodies: serde_json::Value,
        first: (u64, &str),
    ) -> Outcome {
        serde_json::from_value(serde_json::json!({"id": "c", "exact": false, "score": 1,
            "measured": 1, "original_from": 0, "original_to": 9, "says": "",
            "first": {"frame": first.0, "original_frame": first.0, "path": first.1, "original": "",
                "model": "", "distance": "1", "mechanic": "", "paths": []},
            "measures": measures, "bodies": bodies,
            "headline": {"exact": 0, "measures": 1, "together_permille": 0, "says": ""}}))
        .unwrap()
    }

    #[test]
    fn a_value_the_model_never_takes_and_a_body_only_the_original_has() {
        let facing = "entities.mario[0].facing";
        let o = outcome(
            vec![measure(
                facing,
                &["right", "(unnamed value 3)"],
                &["right", "right"],
                Some(1),
            )],
            serde_json::json!([{"kind": "bullet_bill_cannon", "name": "the bullet bill cannon",
                "paired": false, "side": "original", "from": 5, "paths": [], "events": [],
                "first_frame": 5, "says": ""}]),
            (1, facing),
        );
        let seen = Seen::of([&o]);
        let f = findings(&o, &seen);
        assert!(
            f.iter()
                .any(|x| x.class == "only_original" && !x.first && x.says.contains("makes none")),
            "{f:?}"
        );
        let u = f.iter().find(|x| x.class == "unseen").unwrap();
        assert!(
            u.first && u.says.contains("(unnamed value 3)"),
            "{}",
            u.says
        );
    }

    #[test]
    fn a_split_on_the_frame_a_state_turns_is_its_entry() {
        let vx = "entities.mario[0].vx";
        let o = outcome(
            vec![
                measure(vx, &["a", "a", "b"], &["a", "a", "a"], Some(2)),
                measure(
                    "entities.mario[0].state",
                    &["small", "small", "dying_hold"],
                    &["small", "small", "dying_hold"],
                    None,
                ),
            ],
            serde_json::json!([]),
            (2, vx),
        );
        let f = findings(&o, &Seen::default());
        let e = f.iter().find(|x| x.class == "entry").unwrap();
        assert!(
            e.says.contains("turns dying_hold") && e.says.contains("turns too"),
            "{}",
            e.says
        );
    }

    #[test]
    fn a_path_loses_its_slot_indexes() {
        assert_eq!(key("bodies.goomba[1].x"), "bodies.goomba.x");
        assert_eq!(key("entities.mario[0].facing"), "entities.mario.facing");
        assert_eq!(owner("entities.mario[0].vx"), "entities.mario[0]");
    }
}
