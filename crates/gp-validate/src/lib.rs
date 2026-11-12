//! The validation engine (Milestone 2, step 5; docs/validation.md): a
//! model's fidelity contract compiled into checks, and the checks run on
//! recordings of the original.
//!
//! - [`compile`]: the contract's rules into [`compile::Check`]s, each with
//!   the level that governs it, conditions (`when`) included.
//! - [`observe`]: a run's state by state path, frame by frame, as a
//!   [`observe::Trace`].
//! - [`original`]: the original and a subject run side by side through the
//!   model's state mapping (`mapping.toml`, read by gp-interp). The subject
//!   is any runtime that answers state paths, such as the interpreter; the
//!   original, read through the mapping, can stand as one too.
//! - [`profile`]: what a run did, measured: jumps, speeds, state changes,
//!   the level graph; and the same profiles predicted from the model.
//! - [`evaluate`]: the checks on one recording, against the model and
//!   against the original.
//! - [`reach`]: the reachability checks, on the model alone.
//! - [`slots`]: the slot checks, from gp-interp's per-slot comparison.
//! - [`cache`]: each recording's result as a store object, so a corpus
//!   report only evaluates what it hasn't before.

pub mod cache;
pub mod compile;
pub mod evaluate;
pub mod observe;
pub mod original;
pub mod profile;
pub mod reach;
pub mod slots;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use evaluate::{Against, Outcome, Status};

/// The version of the checks and of [`RecordingResult`]. A change to what
/// any check measures or how it compares bumps it, so stored results from
/// before aren't reused.
pub const VALIDATE_VERSION: u32 = 4;

/// One recording's validation: what it was, what ran, and every outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingResult {
    pub validate_version: u32,
    /// What the inputs hash to: the result's key in the store
    /// ([`cache`]).
    pub key: String,
    /// The recording's ID, or for a baseline file its path.
    pub recording: String,
    pub name: String,
    /// The runtime that ran the recording as the subject, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub frames: u64,
    pub outcomes: Vec<Outcome>,
}

/// One check's outcome over every recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub part: String,
    pub kind: String,
    pub quantity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
    pub against: Against,
    pub level: String,
    /// Fail if any recording failed it, pass if any passed it and none
    /// failed, unexercised otherwise.
    pub status: Status,
    pub samples: u64,
    /// Recordings that exercised it.
    pub recordings: u64,
    /// The first failing recording's outcome, or the last passing one's.
    pub example: Outcome,
    /// The name of the recording `example` is from.
    pub from: String,
}

/// Combines each check's outcomes across recordings.
pub fn summarize(results: &[RecordingResult]) -> Vec<Summary> {
    type Key = (String, String, Option<String>, Against);
    let mut by: BTreeMap<Key, Summary> = BTreeMap::new();
    let mut order: Vec<Key> = Vec::new();
    for r in results {
        for o in &r.outcomes {
            let s = by.entry(o.key()).or_insert_with(|| {
                order.push(o.key());
                Summary {
                    part: o.part.clone(),
                    kind: o.kind.clone(),
                    quantity: o.quantity.clone(),
                    case: o.case.clone(),
                    against: o.against,
                    level: String::new(),
                    status: Status::Unexercised,
                    samples: 0,
                    recordings: 0,
                    example: o.clone(),
                    from: r.name.clone(),
                }
            });
            s.samples += o.samples;
            if o.status != Status::Unexercised {
                s.recordings += 1;
                let mut levels: Vec<&str> = s
                    .level
                    .split(',')
                    .chain(o.level.split(','))
                    .filter(|l| !l.is_empty() && *l != "none")
                    .collect();
                levels.sort_unstable();
                levels.dedup();
                s.level = levels.join(",");
            }
            let replace = match (s.status, o.status) {
                (Status::Fail, _) => false,
                (_, Status::Fail) => true,
                (_, Status::Pass) => true,
                // A note naming a frame (where the runs parted) says more
                // than one that doesn't.
                (Status::Unexercised, Status::Unexercised) => {
                    s.example.detail.is_none() || (s.example.frame.is_none() && o.frame.is_some())
                }
                _ => false,
            };
            if replace {
                s.example = o.clone();
                s.from = r.name.clone();
            }
            s.status = s.status.max_by_severity(o.status);
        }
    }
    order
        .into_iter()
        .map(|k| {
            let mut s = by.remove(&k).expect("every key was added");
            if s.level.is_empty() {
                s.level = "none".into();
            }
            s
        })
        .collect()
}

impl Status {
    /// Fail over pass over unexercised.
    fn max_by_severity(self, other: Self) -> Self {
        let rank = |s: Self| match s {
            Self::Unexercised => 0,
            Self::Pass => 1,
            Self::Fail => 2,
        };
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(status: Status, samples: u64) -> Outcome {
        Outcome {
            part: "entities.player.jump".into(),
            kind: "jump".into(),
            quantity: "height".into(),
            case: None,
            against: Against::Model,
            level: if status == Status::Unexercised {
                "none"
            } else {
                "metric"
            }
            .into(),
            status,
            samples,
            expected: None,
            actual: None,
            tolerance: None,
            frame: None,
            detail: None,
        }
    }

    fn result(name: &str, outcomes: Vec<Outcome>) -> RecordingResult {
        RecordingResult {
            validate_version: VALIDATE_VERSION,
            key: String::new(),
            recording: name.into(),
            name: name.into(),
            subject: None,
            frames: 1,
            outcomes,
        }
    }

    #[test]
    fn a_failure_anywhere_fails_the_check() {
        let s = summarize(&[
            result("a", vec![outcome(Status::Unexercised, 0)]),
            result("b", vec![outcome(Status::Pass, 2)]),
            result("c", vec![outcome(Status::Fail, 1)]),
            result("d", vec![outcome(Status::Pass, 3)]),
        ]);
        assert_eq!(s.len(), 1);
        assert_eq!(
            (s[0].status, s[0].samples, s[0].recordings),
            (Status::Fail, 6, 3)
        );
        assert_eq!(s[0].from, "c");
        assert_eq!(s[0].level, "metric");
        let s = summarize(&[result("a", vec![outcome(Status::Unexercised, 0)])]);
        assert_eq!(
            (s[0].status, s[0].level.as_str()),
            (Status::Unexercised, "none")
        );
    }
}
