//! Watching a runtime run: the state the checks read, frame by frame.
//!
//! A [`Trace`] holds the value at each state path (`entities.player[0].y`,
//! `rules.game.state`) at every frame of a run, frame 0 (power-on) first,
//! and the controller byte that drove each frame. [`crate::original::run`]
//! makes one for the original and one for the subject.

use std::collections::HashMap;

use gp_model::Fixed;
use gp_runtime::Value;

/// One observed value: a number (fixed-point, so whole numbers and
/// subpixels alike) or a name, such as a state machine's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Obs {
    Num(Fixed),
    Text(String),
}

impl std::fmt::Display for Obs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Num(n) => write!(f, "{n}"),
            Self::Text(t) => f.write_str(t),
        }
    }
}

/// A run, observed: every path's value at every frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    paths: Vec<String>,
    index: HashMap<String, usize>,
    /// Each entity's instance numbers, from the paths.
    instances: HashMap<String, Vec<usize>>,
    /// `frames[f][i]` is path `i` at frame `f`; `None` where the runtime
    /// couldn't say.
    frames: Vec<Vec<Option<Obs>>>,
    /// `pads[f]` is controller 1's byte for the frame that ran from `f` to
    /// `f + 1`.
    pads: Vec<u8>,
}

impl Trace {
    pub fn new(paths: Vec<String>) -> Self {
        let index = paths
            .iter()
            .enumerate()
            .map(|(i, p)| (p.clone(), i))
            .collect();
        let mut instances: HashMap<String, Vec<usize>> = HashMap::new();
        for p in &paths {
            if let Some((entity, rest)) =
                p.strip_prefix("entities.").and_then(|r| r.split_once('['))
                && let Some(n) = rest.split_once(']').and_then(|(n, _)| n.parse().ok())
            {
                instances.entry(entity.to_owned()).or_default().push(n);
            }
        }
        for v in instances.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
        Self {
            paths,
            index,
            instances,
            frames: Vec::new(),
            pads: Vec::new(),
        }
    }

    /// Adds the next frame. `pad` is the controller byte that led to it,
    /// and is ignored for frame 0.
    pub fn push(&mut self, values: Vec<Option<Obs>>, pad: u8) {
        assert_eq!(values.len(), self.paths.len(), "one value per path");
        if !self.frames.is_empty() {
            self.pads.push(pad);
        }
        self.frames.push(values);
    }

    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// Frames observed, power-on included.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn has(&self, path: &str) -> bool {
        self.index.contains_key(path)
    }

    pub fn get(&self, frame: usize, path: &str) -> Option<&Obs> {
        let i = *self.index.get(path)?;
        self.frames.get(frame)?.get(i)?.as_ref()
    }

    pub fn num(&self, frame: usize, path: &str) -> Option<Fixed> {
        match self.get(frame, path)? {
            Obs::Num(n) => Some(*n),
            Obs::Text(_) => None,
        }
    }

    pub fn text(&self, frame: usize, path: &str) -> Option<&str> {
        match self.get(frame, path)? {
            Obs::Text(t) => Some(t),
            Obs::Num(_) => None,
        }
    }

    /// The controller byte held for the frame that ran into `frame`.
    pub fn pad_into(&self, frame: usize) -> Option<u8> {
        frame.checked_sub(1).and_then(|f| self.pads.get(f).copied())
    }

    /// The instance numbers of an entity that any path names, such as 0
    /// and 1 for `entities.slug[0].x` and `entities.slug[1].x`.
    pub fn instances(&self, entity: &str) -> Vec<usize> {
        self.instances.get(entity).cloned().unwrap_or_default()
    }
}

/// A runtime value as an observation. `fixed` values carry 16 fraction
/// bits, as gp-model's numbers do.
pub fn obs_of(v: &Value) -> Option<Obs> {
    match v {
        Value::Int(n) => Fixed::from_int(*n).ok().map(Obs::Num),
        Value::Fixed(raw) => Some(Obs::Num(Fixed::from_raw(*raw))),
        Value::Flag(b) => Some(Obs::Num(Fixed::from_raw(i64::from(*b) << 16))),
        Value::Text(t) => Some(Obs::Text(t.clone())),
        Value::Entity(_) | Value::Vec2(..) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_reads_back_by_path_and_frame() {
        let mut t = Trace::new(vec![
            "entities.slug[0].x".into(),
            "entities.slug[1].x".into(),
            "rules.game.state".into(),
        ]);
        let n = |v: i64| Some(Obs::Num(Fixed::from_int(v).unwrap()));
        t.push(vec![n(1), n(2), Some(Obs::Text("playing".into()))], 0xff);
        t.push(vec![n(3), None, Some(Obs::Text("won".into()))], 0x80);
        assert_eq!(t.len(), 2);
        assert_eq!(t.num(1, "entities.slug[0].x"), Fixed::from_int(3).ok());
        assert_eq!(t.num(1, "entities.slug[1].x"), None);
        assert_eq!(t.text(1, "rules.game.state"), Some("won"));
        assert_eq!(t.pad_into(1), Some(0x80));
        assert_eq!(t.pad_into(0), None);
        assert_eq!(t.instances("slug"), [0, 1]);
        assert!(t.instances("player").is_empty());
    }
}
