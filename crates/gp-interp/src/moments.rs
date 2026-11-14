//! The game's own moments in a run (docs/interpreter.md, "Moments"): jumps,
//! landings, deaths and lives lost, counter changes and level changes,
//! read from the mapped measures a lockstep keeps with
//! [`crate::lockstep::Options::values`], and from the model's mechanic
//! events on its side.
//!
//! Nothing here names a game. A jump is any mapped entity whose `on_ground`
//! goes from true to false with upward speed (`vy` below 0, or `y` going
//! up when `vy` isn't mapped), and it lasts to the next landing. A death
//! is an entity's `alive` going false, or a model event whose kind ends
//! in `.died`. A life lost is a `lives` counter going down or a `deaths`
//! counter going up. A counter change is any mapped counter changing,
//! except a clock (a counter the model changes every so many frames). A
//! level change is the level the mapping reads changing.
//!
//! [`check`] runs the model on each moment of the original from the
//! original's state a little before it, on the same presses, and finds
//! the model's own moment of the same kind there, so a moment can say
//! "the model jumps too, 2 frames later" or "the model doesn't jump".

use std::collections::BTreeMap;

use gp_runtime::{InputFrame, Runtime, Value};
use gp_sim::{Fx, Spec};

use crate::lockstep::{self, Lockstep, Mode, Options};
use crate::mapping::Mapping;
use crate::translate::raw as num;

/// The kinds of moment, in the order a strip lists them.
pub const KINDS: [&str; 6] = ["jump", "landing", "death", "life_lost", "counter", "level"];

/// Frames before and after a moment that a check runs: half a second at
/// 60 frames a second.
pub const LEAD: u64 = 30;

/// A jump that hasn't landed after this many frames ends there.
const LONGEST: usize = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moment {
    pub kind: &'static str,
    /// Its number among the run's moments of its kind, from 1.
    pub n: usize,
    /// What it happened to: `entities.mario[0]`, `counters.coins`,
    /// `level`, or empty for a model event that names no part.
    pub part: String,
    /// The original's frames it spans, both included: a jump's takeoff
    /// to its landing; one frame for the rest.
    pub from: u64,
    pub to: u64,
    /// A jump's highest frame and how far it rose, in pixels.
    pub peak: Option<u64>,
    pub height: Option<Fx>,
    /// A counter's or a level's value before and after, as shown.
    pub before: Option<String>,
    pub after: Option<String>,
}

/// Which side of a lockstep to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Original,
    Model,
}

fn flag(v: &Value) -> Option<bool> {
    match v {
        Value::Flag(b) => Some(*b),
        Value::Int(n) => Some(*n != 0),
        _ => None,
    }
}

/// An entity path's part and field: `entities.mario[0].vy` is
/// (`entities.mario[0]`, `vy`).
fn entity_field(path: &str) -> Option<(&str, &str)> {
    let (part, field) = path.rsplit_once('.')?;
    part.starts_with("entities.").then_some((part, field))
}

/// The model's clocks: counters it changes every so many frames, which
/// change too often to be moments.
pub fn clocks(model: &toml::Value) -> Vec<String> {
    let Some(counters) = model.get("counters").and_then(|c| c.as_table()) else {
        return Vec::new();
    };
    counters
        .iter()
        .filter(|(_, c)| {
            c.get("change")
                .and_then(|ch| ch.as_array())
                .is_some_and(|a| a.iter().any(|x| x.get("every").is_some()))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// The moments on one side of a lockstep kept with values, in frame order,
/// numbered by kind.
pub fn detect(ls: &Lockstep, side: Side, clocks: &[String]) -> Vec<Moment> {
    let n = ls.values.len();
    let val = |k: usize, i: usize| -> &Value {
        match side {
            Side::Original => &ls.values[k].0[i],
            Side::Model => &ls.values[k].1[i],
        }
    };
    let frame = |k: usize| ls.frames[k].1;
    let mut out: Vec<Moment> = Vec::new();
    let one = |kind: &'static str, part: &str, f: u64| Moment {
        kind,
        n: 0,
        part: part.to_owned(),
        from: f,
        to: f,
        peak: None,
        height: None,
        before: None,
        after: None,
    };
    // Each entity's mapped fields.
    let mut parts: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for (i, p) in ls.paths.iter().enumerate() {
        if let Some((part, field)) = entity_field(&p.path) {
            parts.entry(part).or_default().insert(field, i);
        }
    }
    for (part, fields) in &parts {
        let y = fields.get("y").or_else(|| fields.get("stage_y")).copied();
        let vy = fields.get("vy").copied();
        let ground = fields.get("on_ground").copied();
        let alive = fields.get("alive").copied();
        // An open jump: its takeoff index and the height it left from.
        let mut open: Option<(usize, Option<i64>)> = None;
        let close = |out: &mut Vec<Moment>, at: usize, end: usize, start_y: Option<i64>| {
            let mut m = one("jump", part, frame(at));
            m.to = frame(end);
            if let Some(yi) = y {
                let top = (at..=end)
                    .filter_map(|k| num(val(k, yi)).map(|v| (v, k)))
                    .min_by_key(|(v, k)| (*v, *k));
                if let (Some((top, k)), Some(s)) = (top, start_y) {
                    m.peak = Some(frame(k));
                    m.height = Some(Fx::from_raw((s - top).max(0)));
                }
            }
            out.push(m);
        };
        for k in 1..n {
            if let Some(a) = alive
                && flag(val(k - 1, a)) == Some(true)
                && flag(val(k, a)) == Some(false)
            {
                if let Some((at, sy)) = open.take() {
                    close(&mut out, at, k, sy);
                }
                out.push(one("death", part, frame(k)));
            }
            let Some(g) = ground else { continue };
            let (was, is) = (flag(val(k - 1, g)), flag(val(k, g)));
            if was == Some(true) && is == Some(false) {
                let up = match (vy, y) {
                    (Some(v), _) => num(val(k, v)).is_some_and(|v| v < 0),
                    (None, Some(yi)) => match (num(val(k - 1, yi)), num(val(k, yi))) {
                        (Some(a), Some(b)) => b < a,
                        _ => false,
                    },
                    _ => false,
                };
                if up && open.is_none() {
                    open = Some((k, y.and_then(|yi| num(val(k - 1, yi)))));
                }
            } else if was == Some(false) && is == Some(true) {
                if let Some((at, sy)) = open.take() {
                    close(&mut out, at, k, sy);
                }
                out.push(one("landing", part, frame(k)));
            }
            if let Some((at, sy)) = open
                && k - at >= LONGEST
            {
                close(&mut out, at, k, sy);
                open = None;
            }
        }
        if let Some((at, sy)) = open
            && n > 0
        {
            close(&mut out, at, n - 1, sy);
        }
    }
    // Counters, and lives lost: by the lives counter, or by the deaths
    // counter when no lives counter is mapped, so one death is one life.
    let has_lives = ls.paths.iter().any(|p| p.path == "counters.lives");
    for (i, p) in ls.paths.iter().enumerate() {
        let Some(name) = p.path.strip_prefix("counters.") else {
            continue;
        };
        if clocks.iter().any(|c| c == name) {
            continue;
        }
        for k in 1..n {
            let (a, b) = (val(k - 1, i), val(k, i));
            if crate::mapping::same(a, b) {
                continue;
            }
            let mut m = one("counter", &p.path, frame(k));
            m.before = Some(crate::mapping::show(a));
            m.after = Some(crate::mapping::show(b));
            out.push(m);
            let (x, y) = (num(a), num(b));
            let lost = match name {
                "lives" => x.zip(y).is_some_and(|(x, y)| y < x),
                "deaths" => !has_lives && x.zip(y).is_some_and(|(x, y)| y > x),
                _ => false,
            };
            if lost {
                out.push(one("life_lost", &p.path, frame(k)));
            }
        }
    }
    // Levels.
    for k in 1..ls.levels.len().min(n) {
        let (a, b) = match side {
            Side::Original => (ls.levels[k - 1].0.clone(), ls.levels[k].0.clone()),
            Side::Model => (
                Some(ls.levels[k - 1].1.clone()),
                Some(ls.levels[k].1.clone()),
            ),
        };
        if let (Some(a), Some(b)) = (a, b)
            && a != b
            && !a.is_empty()
            && !b.is_empty()
        {
            let mut m = one("level", "level", frame(k));
            m.before = Some(a);
            m.after = Some(b);
            out.push(m);
        }
    }
    // The model's own deaths, from its events, where no entity's `alive`
    // already said so.
    if side == Side::Model {
        for k in 0..ls.events.len().min(n) {
            if ls.events[k].iter().any(|e| e.ends_with(".died"))
                && !out.iter().any(|m| m.kind == "death" && m.from == frame(k))
            {
                out.push(one("death", "", frame(k)));
            }
        }
    }
    number(&mut out);
    out
}

/// Sorts moments by frame and kind and numbers each kind from 1.
fn number(out: &mut [Moment]) {
    out.sort_by_key(|m| {
        (
            m.from,
            KINDS.iter().position(|k| *k == m.kind).unwrap_or(9),
            m.part.clone(),
        )
    });
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for m in out.iter_mut() {
        let c = seen.entry(m.kind).or_default();
        *c += 1;
        m.n = *c;
    }
}

/// Whether a model's moment answers an original's: the same kind (a death
/// and a life lost answer each other) and the same part, when both name one.
fn answers(orig: &Moment, mine: &Moment) -> bool {
    fn family(k: &'static str) -> &'static str {
        if k == "life_lost" { "death" } else { k }
    }
    family(orig.kind) == family(mine.kind)
        && (orig.part == mine.part || orig.part.is_empty() || mine.part.is_empty())
        && (orig.kind != "counter" || orig.after == mine.after)
        && (orig.kind != "level" || orig.after == mine.after)
}

/// The original's frames a check of `m` runs over: from `lead` before it
/// to `lead` after, no earlier than `first`.
pub fn span(m: &Moment, lead: u64, first: u64) -> (u64, u64) {
    (m.from.saturating_sub(lead).max(first), m.to + lead)
}

/// What the model does at one of the original's moments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The original's frame the check started at, from its state.
    pub start: u64,
    /// The model's moment of the same kind nearest the original's, if any.
    pub model: Option<Moment>,
    /// The original's own moment as the check saw it (the same frames, on
    /// the same presses).
    pub original: Option<Moment>,
    /// How far from exact the check's frames are: how many times a measure
    /// was past its tolerance on a frame, then the distances added up (raw
    /// fixed point). Lower is closer.
    pub apart: usize,
    pub distance: i128,
    /// The measures apart in the check and the farthest each got.
    pub paths: Vec<(String, Fx)>,
}

/// How [`check`] runs: the frames before and after each moment, the
/// original's first frame the model stands for in the whole run, the
/// model's clocks, and each mapped path's tolerance.
#[derive(Debug, Clone, Default)]
pub struct Checks {
    pub lead: u64,
    pub first: u64,
    pub clocks: Vec<String>,
    pub tolerance: Vec<Fx>,
}

/// A check's run: its span, the lockstep, and the original's and the
/// model's moments in it.
type Trial = ((u64, u64), Lockstep, Vec<Moment>, Vec<Moment>);

/// The model's side of each of the original's `moments`: one pass of the
/// original over `inputs` from power-on, and at each moment a model made
/// by `make`, loaded with the original's state [`span`] before it
/// (through the state mapping, as a lockstep from there starts), run free
/// on the same presses to `lead` after it. `mapping` is the run's; its
/// start condition is left out, since every check starts from a frame the
/// model stands for.
pub fn check<O: Runtime, M: Runtime>(
    original: &mut O,
    make: &mut dyn FnMut() -> Result<M, String>,
    spec: &Spec,
    mapping: &Mapping,
    inputs: &[InputFrame],
    moments: &[Moment],
    c: &Checks,
) -> Result<Vec<Answer>, String> {
    let (lead, first, clocks) = (c.lead, c.first, &c.clocks[..]);
    let mut mid = mapping.clone();
    mid.start = None;
    let mut order: Vec<usize> = (0..moments.len()).collect();
    order.sort_by_key(|&i| span(&moments[i], lead, first));
    let mut out: Vec<Option<Answer>> = vec![None; moments.len()];
    // Trials shared by moments with the same span.
    let mut last: Option<Trial> = None;
    for i in order {
        let m = &moments[i];
        let (from, to) = span(m, lead, first);
        while original.frame() < from && (original.frame() as usize) < inputs.len() {
            let f = original.frame() as usize;
            original
                .step(std::slice::from_ref(&inputs[f]))
                .map_err(|e| format!("the original: {e}"))?;
        }
        // A trial whose frames already cover this moment's answers it too:
        // a landing is in its jump's, a coin in the jump that took it.
        if last
            .as_ref()
            .is_none_or(|((f, t), ..)| *f > from || *t < to)
        {
            let mut fork = original.fork();
            let mut model = make()?;
            let start = usize::try_from(fork.frame()).unwrap_or(usize::MAX);
            let end = usize::try_from(to + 1)
                .unwrap_or(usize::MAX)
                .min(inputs.len());
            let ls = lockstep::lockstep(
                &mut fork,
                &mut model,
                spec,
                &mid,
                &inputs[start.min(end)..end],
                &Options {
                    mode: Mode::Free,
                    tolerance: c.tolerance.clone(),
                    from: Some(from),
                    start_state: None,
                    values: true,
                    until_start: false,
                    sound: false,
                    stop_at_split: false,
                    watch: Vec::new(),
                },
            )?;
            let theirs = detect(&ls, Side::Original, clocks);
            let mine = detect(&ls, Side::Model, clocks);
            last = Some(((from, to), ls, theirs, mine));
        }
        let (_, ls, theirs, mine) = last.as_ref().expect("a trial");
        let near = |list: &[Moment]| {
            list.iter()
                .filter(|x| answers(m, x))
                .min_by_key(|x| x.from.abs_diff(m.from))
                .cloned()
        };
        let mut apart = 0;
        let mut distance = 0i128;
        let mut paths = Vec::new();
        for p in &ls.paths {
            let past: Vec<Fx> = p
                .distance
                .iter()
                .copied()
                .filter(|d| *d > p.tolerance)
                .collect();
            apart += past.len();
            distance += past.iter().map(|d| i128::from(d.raw())).sum::<i128>();
            if !past.is_empty() {
                paths.push((p.path.clone(), p.max));
            }
        }
        out[i] = Some(Answer {
            start: ls.start.as_ref().map_or(from, |s| s.original_frame),
            model: near(mine),
            original: near(theirs),
            apart,
            distance,
            paths,
        });
    }
    Ok(out.into_iter().map(|a| a.expect("every moment")).collect())
}

/// The moment a phrase names, as a person types it to the agent: "the very
/// first jump" is jump 1, "the third coin" the third change of a counter
/// named for coins, "the last death" the last death, "jump 2" jump 2. A
/// kind by its words, a number by an ordinal or digits, else the first.
pub fn find<'a>(moments: &'a [Moment], said: &str) -> Option<&'a Moment> {
    let s = said.to_lowercase();
    let has = |w: &str| {
        s.split(|c: char| !c.is_alphanumeric())
            .any(|t| t == w || (t.starts_with(w) && w.len() > 3))
    };
    let kind = if has("jump") {
        Some("jump")
    } else if has("land") || has("landing") {
        Some("landing")
    } else if has("life") || has("lives") {
        Some("life_lost")
    } else if has("die") || has("died") || has("death") || has("dies") {
        Some("death")
    } else if has("level") {
        Some("level")
    } else {
        None
    };
    let pick: Vec<&Moment> = match kind {
        Some(k) => moments.iter().filter(|m| m.kind == k).collect(),
        None => {
            // A counter by its name, said in the singular or the plural.
            let one = |n: &str| match n.strip_suffix("ies") {
                Some(stem) => format!("{stem}y"),
                None => n.strip_suffix('s').unwrap_or(n).to_owned(),
            };
            let part = moments.iter().find(|m| {
                m.kind == "counter"
                    && m.part
                        .strip_prefix("counters.")
                        .is_some_and(|c| s.contains(&one(&c.replace('_', " "))))
            })?;
            moments
                .iter()
                .filter(|m| m.kind == "counter" && m.part == part.part)
                .collect()
        }
    };
    if has("last") {
        return pick.last().copied();
    }
    const ORD: [&str; 10] = [
        "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth",
        "tenth",
    ];
    let n = ORD
        .iter()
        .position(|w| has(w))
        .map(|i| i + 1)
        .or_else(|| {
            s.split(|c: char| !c.is_alphanumeric()).find_map(|t| {
                t.trim_end_matches(|c: char| c.is_alphabetic())
                    .parse::<usize>()
                    .ok()
            })
        })
        .unwrap_or(1);
    pick.get(n.checked_sub(1)?).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(kind: &'static str, n: usize, part: &str, from: u64) -> Moment {
        Moment {
            kind,
            n,
            part: part.into(),
            from,
            to: from,
            peak: None,
            height: None,
            before: None,
            after: None,
        }
    }

    #[test]
    fn a_phrase_names_a_moment() {
        let ms = vec![
            m("jump", 1, "entities.frog[0]", 10),
            m("counter", 1, "counters.flies", 12),
            m("jump", 2, "entities.frog[0]", 40),
            m("death", 1, "entities.frog[0]", 90),
        ];
        let at = |s: &str| find(&ms, s).map(|x| (x.kind, x.n));
        assert_eq!(at("refine the very first jump"), Some(("jump", 1)));
        assert_eq!(at("refine this jump"), Some(("jump", 1)));
        assert_eq!(at("the second jump"), Some(("jump", 2)));
        assert_eq!(at("jump 2"), Some(("jump", 2)));
        assert_eq!(at("the last jump"), Some(("jump", 2)));
        assert_eq!(at("the 1st fly"), Some(("counter", 1)));
        assert_eq!(at("where the frog dies"), Some(("death", 1)));
        assert_eq!(at("the third jump"), None);
        assert_eq!(at("the score"), None);
    }
}
