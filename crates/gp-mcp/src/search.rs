//! Input search: find button holds that reach a goal in the copy's own
//! model, so a variant can get a baseline that wins (docs/host.md,
//! "Variant baselines").
//!
//! The search starts from holds (a baseline's, or given) and moves them: each
//! hold span is shifted and lengthened or shortened in turn, keeping the move
//! that does best, for a few rounds. "Best" is, in order, the goal being met
//! at the last frame, then the furthest the `progress` path reaches during
//! the run. The interpreter runs a few hundred frames in about a
//! millisecond, so a few thousand tries take seconds. It is a local search,
//! not a planner: it finds a way through when the original's way is close,
//! as a 20% higher jump is, and says so when it doesn't.

use std::path::Path;

use crate::Config;
use crate::tools;
use gp_interp::{Interp, Lifted};
use gp_runtime::{InputFrame, Runtime};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub copy: String,
    /// Start from a baseline of the model's source (its frames and holds)...
    #[serde(default)]
    pub baseline: Option<String>,
    /// ...or from these.
    #[serde(default)]
    pub holds: Vec<String>,
    #[serde(default)]
    pub frames: Option<usize>,
    /// The goal: a state path and the value it must hold at the last frame,
    /// such as `rules.game.state` = `won`.
    pub goal_path: String,
    pub goal_value: String,
    /// A number to push as far as it goes, such as `entities.player[0].x`.
    pub progress_path: String,
    /// How far a span's start may move either way (default 12 frames).
    #[serde(default)]
    pub window: Option<i64>,
    /// Passes over the spans (default 3).
    #[serde(default)]
    pub rounds: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct Span {
    buttons: String,
    start: i64,
    /// None runs to the end.
    end: Option<i64>,
}

impl Span {
    fn parse(s: &str) -> Result<Self, String> {
        let (buttons, range) = s
            .split_once('@')
            .ok_or(format!("`{s}` isn't a hold like a@10-20"))?;
        let (a, b) = range
            .split_once('-')
            .ok_or(format!("`{s}` isn't a hold like a@10-20"))?;
        let num = |t: &str| {
            t.parse::<i64>()
                .map_err(|_| format!("`{s}` has a frame that isn't a number"))
        };
        Ok(Self {
            buttons: buttons.to_owned(),
            start: num(a)?,
            end: if b.is_empty() { None } else { Some(num(b)?) },
        })
    }
    fn text(&self) -> String {
        format!(
            "{}@{}-{}",
            self.buttons,
            self.start,
            self.end.map_or(String::new(), |e| e.to_string())
        )
    }
}

fn texts(spans: &[Span]) -> Vec<String> {
    spans.iter().map(Span::text).collect()
}

/// How a try did: (goal met, furthest progress).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
struct Score(bool, f64);

struct Runner<'a> {
    config: &'a Config,
    model: &'a gp_model::Model,
    lifted: &'a Lifted,
    frames: usize,
    goal: (&'a str, &'a str),
    progress: &'a str,
}

impl Runner<'_> {
    fn run(&self, spans: &[Span]) -> Result<Score, String> {
        let t = texts(spans);
        let args = tools::PreviewArgs {
            copy: String::new(),
            frames: Some(self.frames),
            holds: t,
            baseline: None,
            paths: Vec::new(),
            at: Vec::new(),
            picture: false,
        };
        let input: Vec<InputFrame> = tools::input(self.config, self.model, &args)?;
        let mut rt = Interp::load(self.model, self.lifted)?;
        let mut best = f64::MIN;
        for f in &input {
            rt.step(std::slice::from_ref(f))?;
            let v = rt
                .query(self.progress)
                .map_err(|e| format!("`{}`: {e}", self.progress))?;
            if let Some(x) = v.first().and_then(|e| number(&tools::value_json(&e.value))) {
                best = best.max(x);
            }
        }
        let g = tools::read_paths(&rt, &[self.goal.0.to_owned()])?;
        let met = g
            .as_object()
            .and_then(|m| m.values().next())
            .is_some_and(|v| match v {
                Value::String(s) => s == self.goal.1,
                other => other == self.goal.1,
            });
        Ok(Score(met, best))
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn search(config: &Config, dir: &Path, req: &Search) -> Result<Value, String> {
    let (_, model) = tools::load_model(dir)?;
    let assets = config
        .assets_of(dir, model.model.source.as_deref())
        .ok_or("the model names no source, so start with --assets")?;
    let lifted = Lifted::read_dir(&assets, &model).map_err(|e| e.to_string())?;
    let (frames, start_holds) = match &req.baseline {
        Some(name) => {
            let source = model
                .model
                .source
                .as_deref()
                .filter(|s| crate::is_source_name(s))
                .ok_or("the model names no source ID, so it has no baselines")?;
            if !crate::is_source_name(name) {
                return Err(format!("`{name}` isn't a baseline name"));
            }
            let b = gp_evidence::baseline::Baseline::load(
                &config
                    .repo
                    .join("baselines")
                    .join(source)
                    .join(format!("{name}.toml")),
            )
            .map_err(|e| e.to_string())?;
            (b.frames, b.holds)
        }
        None => (
            req.frames
                .ok_or("give `frames` with `holds`, or a `baseline`")?,
            req.holds.clone(),
        ),
    };
    let mut spans: Vec<Span> = start_holds
        .iter()
        .map(|h| Span::parse(h))
        .collect::<Result<_, _>>()?;
    let runner = Runner {
        config,
        model: &model,
        lifted: &lifted,
        frames,
        goal: (&req.goal_path, &req.goal_value),
        progress: &req.progress_path,
    };
    let window = req.window.unwrap_or(12).clamp(1, 60);
    let rounds = req.rounds.unwrap_or(3).clamp(1, 10);
    let start = runner.run(&spans)?;
    let mut best = start;
    let mut tries = 1usize;
    'rounds: for _ in 0..rounds {
        let mut improved = false;
        for i in 0..spans.len() {
            let here = spans[i].clone();
            for ds in -window..=window {
                for dl in [-2i64, -1, 0, 1, 2, 4] {
                    // A span that runs to the end only moves its start.
                    if here.end.is_none() && dl != 0 {
                        continue;
                    }
                    let mut s = here.clone();
                    s.start = (here.start + ds).max(0);
                    s.end = here.end.map(|e| (e + ds + dl).max(s.start));
                    if s == here {
                        continue;
                    }
                    let mut trial = spans.clone();
                    trial[i] = s.clone();
                    tries += 1;
                    let score = runner.run(&trial)?;
                    if score > best {
                        best = score;
                        spans[i] = s;
                        improved = true;
                    }
                }
            }
        }
        if best.0 || !improved {
            break 'rounds;
        }
    }
    Ok(json!({
        "ok": best.0,
        "goal": { "path": req.goal_path, "value": req.goal_value },
        "goal_met": best.0,
        "frames": frames,
        "holds": texts(&spans),
        "started_from": start_holds,
        "start": { "goal_met": start.0, "progress": start.1 },
        "progress": best.1,
        "tries": tries,
        "note": if best.0 { "these holds reach the goal in this model" } else { "no holds found that reach the goal; progress is the furthest the search got" },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_parse_and_print_the_same() {
        for s in ["a@68-70", "right+b@10-"] {
            assert_eq!(Span::parse(s).unwrap().text(), s);
        }
        assert!(Span::parse("a").is_err());
        assert!(Span::parse("a@x-3").is_err());
    }

    #[test]
    fn a_goal_that_is_met_scores_above_any_progress() {
        assert!(Score(true, 0.0) > Score(false, 1e9));
        assert!(Score(false, 2.0) > Score(false, 1.0));
    }
}
