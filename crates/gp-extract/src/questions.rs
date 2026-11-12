//! Active questions for the person (docs/mechanics.md, "Questions"): where
//! measurement can't decide a fact, the tools ask, with the candidate
//! answers and what would settle it. They're kept in the working copy's
//! `questions.toml`, beside model.toml, never in the repo; `gp model
//! questions` lists and answers them, and an answer is written into the
//! model as a fact with evidence.

use gp_model::model::Ref;
use serde::{Deserialize, Serialize};

/// The file questions are kept in, in a model directory.
pub const QUESTIONS_FILE: &str = "questions.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// The value the answer writes: a number in the model's fixed-point
    /// text, or for `which-entity` an entity name.
    pub value: String,
    /// Where the candidate comes from.
    pub from: String,
    /// The sources the candidate was read at, kept in the fact's evidence
    /// when it's chosen, so whatever read them can read them again (a
    /// derived value's cited line; docs/derivation.md).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<Ref>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    /// Stable across runs: the kind and the path, so asking again doesn't
    /// add a second copy.
    pub id: String,
    /// `conflict` (the samples disagree), `disagree` (a confident
    /// measurement against the model's fact), `which-entity` (a track
    /// that can't be told to be one entity) or `shape` (no numbers make a
    /// lockstep window exact, so a general mechanic is proposed; its
    /// answer approves or declines it and writes no fact).
    pub kind: String,
    /// The model path it's about.
    pub path: String,
    pub text: String,
    /// What would settle it without asking: a recording to make, or code
    /// to read.
    pub settle: String,
    #[serde(default, rename = "candidate")]
    pub candidates: Vec<Candidate>,
    /// The candidate chosen, by index, once answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Questions {
    #[serde(default, rename = "question")]
    pub questions: Vec<Question>,
}

impl Questions {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("questions serialize")
    }

    /// Adds `q` unless a question with its ID is there; an open one is
    /// replaced, so its candidates are the latest measurement's. Returns
    /// whether it was new.
    pub fn ask(&mut self, q: Question) -> bool {
        match self.questions.iter_mut().find(|o| o.id == q.id) {
            Some(o) if o.answer.is_none() => {
                *o = q;
                false
            }
            Some(_) => false,
            None => {
                self.questions.push(q);
                true
            }
        }
    }

    pub fn open(&self) -> impl Iterator<Item = &Question> {
        self.questions.iter().filter(|q| q.answer.is_none())
    }

    /// Whether the question about `path` has been answered.
    pub fn answered(&self, path: &str) -> Option<&Question> {
        self.questions
            .iter()
            .find(|q| q.path == path && q.answer.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_round_trip_and_aren_t_asked_twice() {
        let q = Question {
            id: "conflict:entities.p.run.walk_speed".into(),
            kind: "conflict".into(),
            path: "entities.p.run.walk_speed".into(),
            text: "Which?".into(),
            settle: "a longer run".into(),
            candidates: vec![
                Candidate {
                    value: "1".into(),
                    from: "a".into(),
                    refs: Vec::new(),
                },
                Candidate {
                    value: "1.5".into(),
                    from: "b".into(),
                    refs: vec![Ref {
                        at: "game.s:12".into(),
                        within: None,
                        quote: None,
                    }],
                },
            ],
            answer: None,
        };
        let mut qs = Questions::default();
        assert!(qs.ask(q.clone()));
        assert!(!qs.ask(q.clone()));
        assert_eq!(qs.questions.len(), 1);
        let back = Questions::parse(&qs.to_toml()).unwrap();
        assert_eq!(back, qs);
        qs.questions[0].answer = Some(1);
        assert_eq!(qs.open().count(), 0);
        assert!(qs.answered("entities.p.run.walk_speed").is_some());
    }
}
