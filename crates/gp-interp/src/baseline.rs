//! Model baselines: a scripted input that plays one level of a model in
//! the interpreter, with the digest of its state hashes and the values
//! some model paths must have (docs/originals.md). They are the
//! baselines of a game that has no ROM, such as an original template, or
//! of a variant whose own play is recorded: they run on the model alone,
//! never on a console.
//!
//! A baseline names its level, and the model is started there (its
//! `start_level` set to it) so each level has a baseline of its own.

use std::path::Path;
use std::sync::Arc;

use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::{Hash, InputLog};
use gp_model::Model;
use gp_runtime::InputFrame;
use gp_sim::Exts;
use serde::{Deserialize, Serialize};

use crate::{Bundle, Interp, Lifted};

/// The format's version.
pub const FORMAT: &str = "0.1.0";

/// The directory a template keeps its model baselines in.
pub const DIR: &str = "baselines";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelBaseline {
    pub format: String,
    /// The model's `model.name`.
    pub model: String,
    /// The level the run starts in.
    pub level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// How the input was made: `search` (`gp interp reach --record`) or
    /// `hand`.
    pub recorded: String,
    pub frames: usize,
    /// Button spans, as `gp input-log generate --hold` takes them.
    pub holds: Vec<String>,
    /// The digest `gp interp run` prints for this input, from the level.
    pub digest: String,
    #[serde(default, rename = "expect", skip_serializing_if = "Vec::is_empty")]
    pub expects: Vec<Expect>,
}

/// A model path's value, as `gp interp run` prints it, after `frame`
/// frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    pub frame: usize,
    pub path: String,
    pub equals: String,
}

impl ModelBaseline {
    pub fn parse(text: &str) -> Result<Self, String> {
        let b: ModelBaseline = toml::from_str(text).map_err(|e| e.to_string())?;
        if b.format != FORMAT {
            return Err(format!(
                "format {} isn't {FORMAT}, the one this gp reads",
                b.format
            ));
        }
        if let Some(e) = b.expects.iter().find(|e| e.frame > b.frames) {
            return Err(format!(
                "expect at frame {} is past the run's {} frames",
                e.frame, b.frames
            ));
        }
        Ok(b)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn to_toml(&self, comment: &str) -> String {
        let mut out = String::new();
        for line in comment.lines() {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&toml::to_string(self).expect("a baseline serializes"));
        out
    }

    pub fn input(&self) -> Result<InputLog, String> {
        let holds: Vec<&str> = self.holds.iter().map(String::as_str).collect();
        gp_evidence::input_log::generate(self.frames, &holds).map_err(|e| e.to_string())
    }
}

/// Button spans for a run of controller 1 buttons, one span per button
/// per stretch it's held, in the order they start.
pub fn holds_from_buttons(buttons: &[u8]) -> Vec<String> {
    const NAMES: [(u8, &str); 8] = [
        (gp_evidence::input_log::button::A, "a"),
        (gp_evidence::input_log::button::B, "b"),
        (gp_evidence::input_log::button::SELECT, "select"),
        (gp_evidence::input_log::button::START, "start"),
        (gp_evidence::input_log::button::UP, "up"),
        (gp_evidence::input_log::button::DOWN, "down"),
        (gp_evidence::input_log::button::LEFT, "left"),
        (gp_evidence::input_log::button::RIGHT, "right"),
    ];
    let mut spans: Vec<(usize, String)> = Vec::new();
    for (bit, name) in NAMES {
        let mut start = None;
        for (i, b) in buttons.iter().chain(std::iter::once(&0)).enumerate() {
            match (b & bit != 0, start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    spans.push((s, format!("{name}@{s}-{}", i - 1)));
                    start = None;
                }
                _ => {}
            }
        }
    }
    spans.sort();
    spans.into_iter().map(|(_, s)| s).collect()
}

/// What a run of a baseline's input gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub digest: String,
    /// Each expectation's value, as printed, in the baseline's order.
    pub got: Vec<Option<String>>,
    /// Every model path's value after the last frame, as printed.
    pub last: Vec<(String, String)>,
}

/// The model started at `level`.
pub fn at_level(model: &Model, level: &str) -> Result<Model, String> {
    if !model.levels.contains_key(level) {
        return Err(format!("the model has no level `{level}`"));
    }
    let mut m = model.clone();
    m.model.start_level = Some(level.to_owned());
    Ok(m)
}

/// Runs `input` on `model` from `level` in the interpreter with `exts`,
/// reading the values of `expects` at their frames.
pub fn run(
    model: &Model,
    lifted: &Lifted,
    exts: Arc<Exts>,
    level: &str,
    input: &InputLog,
    expects: &[Expect],
) -> Result<Run, String> {
    let m = at_level(model, level)?;
    let bundle = Bundle::new(&m, lifted.clone());
    let mut rt = Interp::from_bundle(&bundle)?.with_exts(exts);
    let hash = |rt: &Interp| {
        let h = Hash::from_bytes(rt.world().hash());
        FrameHash { state: h, ram: h }
    };
    let values = |rt: &Interp| -> Vec<(String, String)> {
        crate::state::leaves(rt.spec(), rt.world())
            .iter()
            .map(|(p, v)| (gp_model::path::display(p), crate::mapping::show(v)))
            .collect()
    };
    let mut got = vec![None; expects.len()];
    let read = |frame: usize, rt: &Interp, got: &mut Vec<Option<String>>| {
        if expects.iter().any(|e| e.frame == frame) {
            let v = values(rt);
            for (i, e) in expects.iter().enumerate() {
                if e.frame == frame {
                    got[i] = v.iter().find(|(p, _)| *p == e.path).map(|(_, x)| x.clone());
                }
            }
        }
    };
    let mut hashes = vec![hash(&rt)];
    read(0, &rt, &mut got);
    for (i, f) in input.frames.iter().enumerate() {
        rt.advance(&InputFrame {
            buttons: f.buttons_word(),
            axes: Vec::new(),
            reset: f.reset,
        })?;
        hashes.push(hash(&rt));
        read(i + 1, &rt, &mut got);
    }
    Ok(Run {
        digest: Hash::of(&frame_hashes::encode(&hashes)).to_string(),
        got,
        last: values(&rt),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_round_trip_through_the_input_log() {
        use gp_evidence::input_log::button::{A, B, RIGHT};
        let buttons = [0, RIGHT, RIGHT | B, RIGHT | B | A, A, 0, RIGHT];
        let holds = holds_from_buttons(&buttons);
        assert_eq!(holds, ["right@1-3", "b@2-3", "a@3-4", "right@6-6"]);
        let refs: Vec<&str> = holds.iter().map(String::as_str).collect();
        let log = gp_evidence::input_log::generate(buttons.len(), &refs).unwrap();
        let back: Vec<u8> = log.frames.iter().map(|f| f.pads[0]).collect();
        assert_eq!(back, buttons);
    }

    #[test]
    fn a_baseline_round_trips_and_refuses_late_expectations() {
        let b = ModelBaseline {
            format: FORMAT.into(),
            model: "m".into(),
            level: "one".into(),
            description: Some("d".into()),
            recorded: "hand".into(),
            frames: 10,
            holds: vec!["right@0-9".into()],
            digest: "x".into(),
            expects: vec![Expect {
                frame: 10,
                path: "rules.game.state".into(),
                equals: "won".into(),
            }],
        };
        let text = b.to_toml("A comment");
        assert!(text.starts_with("# A comment\n"));
        assert_eq!(ModelBaseline::parse(&text).unwrap(), b);
        let late = text.replace("frame = 10", "frame = 11");
        assert!(ModelBaseline::parse(&late).is_err());
    }
}
