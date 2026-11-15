//! `gp validate GAME --target NAME=VALUE`: a changed game's measured
//! difficulty numbers held to targets the person states (issue #7;
//! docs/validation.md, "Difficulty against a target").
//!
//! Nothing new is measured here. The numbers are the ones the tools
//! already give for a stage:
//!
//! | Names | From |
//! |---|---|
//! | `route_frames`, `climb_frames_min`, `throw_to_drum_median`, `jump_window_median`, `leave_by_ladder_share`, `ladders`, `gaps`' parts... (every number `gp level measure` prints) | `gp level verify` on a girder stage of a game with a level kit |
//! | `run_frames` | The playthrough: the frames a run with every enemy in takes to the goal (`gp level verify` on a tile stage, `gp interp reach`) |
//! | `difficulty_bits` | `gp level difficulty`: the run's inputs scored by how early or late each may come (gp_interp::difficulty) |
//! | `airtime`, `rise`, `walk_distance`, `run_distance` | The jump's envelope (gp_interp::reach::envelope), the numbers `gp author`'s feel check compares |
//!
//! A target is held on every stage that measures its name. A name no stage
//! measures is said, with the names that are measured, and the result is
//! then not a full check; it never passes or fails on a number nobody
//! measured. "Feel" in the person's words has no measure: no tool here turns
//! "floatier" or "tense" into a number, so this says so and holds only the
//! numbers named.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_json::{Value, json};

/// The names measured only when a target asks for them: they cost a search
/// of their own on each stage.
pub(crate) const ON_DEMAND: &[&str] = &[
    "difficulty_bits",
    "airtime",
    "rise",
    "walk_distance",
    "run_distance",
];

/// What the result says of the feel the person asked for.
pub(crate) const FEEL: &str = "the feel you asked for, in your words, is not measured: no tool here turns words like \"floatier\" or \"tense\" into a number. The jump's numbers (airtime, rise, walk_distance, run_distance), which gp author's feel check compares, and difficulty_bits can be held with --target";

/// How far from the target a measure may be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Within {
    By(f64),
    /// A share of the target, in percent.
    Percent(f64),
}

impl Within {
    fn words(self) -> String {
        match self {
            Within::By(v) => format!("+-{}", num(v)),
            Within::Percent(p) => format!("+-{}%", num(p)),
        }
    }
    fn holds(self, have: f64, want: f64) -> bool {
        let tol = match self {
            Within::By(v) => v,
            Within::Percent(p) => want.abs() * p / 100.0,
        };
        (have - want).abs() <= tol + 1e-9
    }
}

/// One target the person stated.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Target {
    pub name: String,
    pub value: f64,
    pub within: Option<Within>,
}

/// `NAME=VALUE`, `NAME=VALUE+-TOL` or `NAME=VALUE+-TOL%`.
pub(crate) fn parse(s: &str) -> Result<Target> {
    let Some((name, rest)) = s.split_once('=') else {
        bail!("--target {s}: give NAME=VALUE, like route_frames=1300 or route_frames=1300+-10%");
    };
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("--target {s}: `{name}` isn't a measure's name (letters, digits and _)");
    }
    let (value, within) = match rest.split_once("+-") {
        Some((v, w)) => (v, Some(w.trim())),
        None => (rest, None),
    };
    let value: f64 = value
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("--target {s}: `{}` isn't a number", value.trim()))?;
    let within = match within {
        None => None,
        Some(w) => Some(match w.strip_suffix('%') {
            Some(p) => Within::Percent(
                p.trim()
                    .parse()
                    .map_err(|_| anyhow::anyhow!("--target {s}: `{w}` isn't a tolerance"))?,
            ),
            None => Within::By(
                w.parse()
                    .map_err(|_| anyhow::anyhow!("--target {s}: `{w}` isn't a tolerance"))?,
            ),
        }),
    };
    if within.is_some_and(|w| matches!(w, Within::By(v) | Within::Percent(v) if v < 0.0)) {
        bail!("--target {s}: a tolerance can't be below 0");
    }
    Ok(Target {
        name: name.to_owned(),
        value,
        within,
    })
}

/// A level kit's tolerance for a measure of this one name, as gp level
/// verify reads it: the first number of its `within`, a share when it says
/// `%`. None for one that isn't set, or covers a list or several names.
pub(crate) fn kit_within(kit: &[Value], name: &str) -> Option<Within> {
    let m = kit.iter().find(|m| m["name"].as_str() == Some(name))?;
    let w = m["within"].as_str()?;
    if w.contains("each") || w.contains("not set") {
        return None;
    }
    let n: f64 = w
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find_map(|x| x.parse().ok())?;
    Some(if w.contains('%') {
        Within::Percent(n)
    } else {
        Within::By(n)
    })
}

/// One stage's numbers by name, and its kit's play measures (gp level
/// verify's `measures`).
#[derive(Debug, Clone, Default)]
pub(crate) struct Measured {
    pub level: String,
    pub numbers: BTreeMap<String, f64>,
    pub kit: Vec<Value>,
    /// Why an on-demand measure couldn't be taken here, by name.
    pub not: BTreeMap<String, String>,
}

/// The numbers in a measures map: the top level's, as gp level measure
/// gives them by name.
pub(crate) fn numbers_of(m: &Value) -> BTreeMap<String, f64> {
    m.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x)))
        .collect()
}

/// The targets held on the stages: JSON, lines, and whether every target
/// that could be measured is met, and whether every one could be.
pub(crate) fn hold(targets: &[Target], stages: &[Measured]) -> (Value, Vec<String>, bool, bool) {
    let mut rows = Vec::new();
    let mut lines = vec![format!(
        "difficulty against your target{}:",
        if targets.len() == 1 { "" } else { "s" }
    )];
    let mut all_met = true;
    let mut all_measured = true;
    for t in targets {
        let on: Vec<&Measured> = stages
            .iter()
            .filter(|s| s.numbers.contains_key(&t.name))
            .collect();
        if on.is_empty() {
            all_measured = false;
            let mut names: Vec<&str> = stages
                .iter()
                .flat_map(|s| s.numbers.keys().map(String::as_str))
                .collect();
            names.sort_unstable();
            names.dedup();
            let why_not: Vec<String> = stages
                .iter()
                .filter_map(|s| s.not.get(&t.name).map(|w| format!("{}: {w}", s.level)))
                .collect();
            let why = if !why_not.is_empty() {
                format!("it couldn't be measured here ({})", why_not.join("; "))
            } else if names.is_empty() {
                "no stage was measured".to_owned()
            } else {
                format!(
                    "no stage measures it. The names measured here: {}",
                    names.join(", ")
                )
            };
            lines.push(format!("  NOT MEASURED {}: {why}", t.name));
            rows.push(json!({"target": t.name, "want": t.value, "measured": false, "why": why}));
            continue;
        }
        for s in on {
            let have = s.numbers[&t.name];
            let (within, from) = match t.within.or_else(|| kit_within(&s.kit, &t.name)) {
                Some(w) if t.within.is_some() => (Some(w), "yours"),
                Some(w) => (Some(w), "the level kit's"),
                None => (None, "none given, so it must be equal"),
            };
            let ok = within.map_or((have - t.value).abs() < 1e-9, |w| w.holds(have, t.value));
            all_met &= ok;
            let tol = within.map_or("exactly".to_owned(), Within::words);
            lines.push(format!(
                "  {} {} on {}: {} against {} {tol} (tolerance: {from}){}",
                if ok { "ok  " } else { "OFF " },
                t.name,
                s.level,
                num(have),
                num(t.value),
                if ok {
                    String::new()
                } else if have > t.value {
                    ", higher than the target".to_owned()
                } else {
                    ", lower than the target".to_owned()
                }
            ));
            rows.push(json!({
                "target": t.name, "level": s.level, "measured": true, "have": have,
                "want": t.value, "within": tol, "tolerance_from": from, "ok": ok,
            }));
        }
    }
    lines.push(format!("  feel: {FEEL}"));
    (
        json!({
            "ok": all_met, "complete": all_measured, "targets": rows,
            "feel": {"measured": false, "why": FEEL},
        }),
        lines,
        all_met,
        all_measured,
    )
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{:.3}", v)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

/// The on-demand measures of one stage that the targets name, on the model
/// as the tools run it.
pub(crate) fn on_demand(
    src: &crate::hosted::Source,
    level: &str,
    targets: &[Target],
    m: &mut Measured,
) {
    let wanted: Vec<&str> = targets
        .iter()
        .map(|t| t.name.as_str())
        .filter(|n| ON_DEMAND.contains(n) && !m.numbers.contains_key(*n))
        .collect();
    if wanted.is_empty() {
        return;
    }
    let spec = &src.spec;
    let Some(i) = spec.levels.iter().position(|l| l.name == level) else {
        for n in wanted {
            m.not
                .insert(n.into(), "the interpreter has no such level".into());
        }
        return;
    };
    if wanted.contains(&"difficulty_bits") {
        match gp_interp::difficulty::score(spec, i, src, Default::default()) {
            Ok(d) if d.cleared => {
                m.numbers
                    .insert("difficulty_bits".into(), f64::from(d.millibits) / 1000.0);
            }
            Ok(d) => {
                m.not.insert(
                    "difficulty_bits".into(),
                    format!(
                        "gp level difficulty found no run to the goal in {} places",
                        d.searched
                    ),
                );
            }
            Err(e) => {
                m.not.insert(
                    "difficulty_bits".into(),
                    format!("gp level difficulty: {e}"),
                );
            }
        }
    }
    let jump = ["airtime", "rise", "walk_distance", "run_distance"];
    if wanted.iter().any(|n| jump.contains(n)) {
        match gp_interp::reach::envelope(spec, i, src) {
            Ok(e) => {
                m.numbers.insert("airtime".into(), f64::from(e.airtime));
                m.numbers.insert("rise".into(), e.rise as f64);
                m.numbers
                    .insert("walk_distance".into(), e.walk_distance as f64);
                match e.run_distance {
                    Some(r) => {
                        m.numbers.insert("run_distance".into(), r as f64);
                    }
                    None => {
                        m.not
                            .insert("run_distance".into(), "the player can't run".into());
                    }
                }
            }
            Err(e) => {
                for n in jump {
                    m.not.insert(
                        n.into(),
                        format!("the jump's envelope can't be measured: {e}"),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_parses_with_its_tolerance_and_a_bad_one_is_refused_in_words() {
        assert_eq!(
            parse("route_frames=1300").unwrap(),
            Target {
                name: "route_frames".into(),
                value: 1300.0,
                within: None
            }
        );
        assert_eq!(
            parse("route_frames=1300+-10%").unwrap().within,
            Some(Within::Percent(10.0))
        );
        assert_eq!(
            parse("jump_window_median = 20 +- 3").unwrap().within,
            Some(Within::By(3.0))
        );
        for bad in ["route_frames", "=3", "a b=3", "x=fast", "x=3+-y", "x=3+--1"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_kits_tolerance_is_read_only_for_one_number() {
        let kit = vec![
            json!({"name": "route_frames", "within": "10%"}),
            json!({"name": "jump_window_median", "within": "3 frames"}),
            json!({"name": "gaps", "within": "8 px each"}),
            json!({"name": "goal_height", "within": "not set"}),
        ];
        assert_eq!(
            kit_within(&kit, "route_frames"),
            Some(Within::Percent(10.0))
        );
        assert_eq!(
            kit_within(&kit, "jump_window_median"),
            Some(Within::By(3.0))
        );
        assert_eq!(kit_within(&kit, "gaps"), None);
        assert_eq!(kit_within(&kit, "goal_height"), None);
        assert_eq!(kit_within(&kit, "nothing"), None);
    }

    #[test]
    fn targets_are_held_where_measured_and_said_where_not() {
        let stage = |level: &str, n: &[(&str, f64)]| Measured {
            level: level.into(),
            numbers: n.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            kit: vec![json!({"name": "route_frames", "within": "10%"})],
            not: BTreeMap::new(),
        };
        let stages = vec![
            stage("first", &[("route_frames", 1200.0), ("run_frames", 900.0)]),
            stage("second", &[("run_frames", 700.0)]),
        ];
        // The kit's 10% holds 1200 against 1300; run_frames must be equal
        // with no tolerance, and is held on both stages.
        let t = vec![
            parse("route_frames=1300").unwrap(),
            parse("run_frames=900").unwrap(),
        ];
        let (j, lines, met, complete) = hold(&t, &stages);
        assert!(!met && complete, "{lines:?}");
        let rows = j["targets"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["ok"], true);
        assert_eq!(rows[0]["tolerance_from"], "the level kit's");
        assert_eq!(rows[1]["ok"], true);
        assert_eq!(rows[2]["ok"], false);
        assert!(lines[3].contains("lower than the target"), "{lines:?}");
        // A name no stage measures: said, with the names that are, and
        // the result isn't complete.
        let (j, lines, met, complete) = hold(&[parse("tension=3").unwrap()], &stages);
        assert!(met && !complete);
        assert_eq!(j["targets"][0]["measured"], false);
        assert!(lines[1].contains("route_frames, run_frames"), "{lines:?}");
        // Feel is never measured, and says so.
        assert_eq!(j["feel"]["measured"], false);
        assert!(lines.last().unwrap().contains("not measured"));
        // A stated tolerance wins over the kit's.
        let (j, _, met, _) = hold(&[parse("route_frames=1300+-50").unwrap()], &stages);
        assert!(!met);
        assert_eq!(j["targets"][0]["tolerance_from"], "yours");
    }
}
