//! The refinement loop on one corpus clip, driven by the lockstep
//! (ported from game-printer's `gp model refine --lockstep`).
//!
//! Each round runs the original and the model in lockstep in resync mode
//! from the clip's start, and takes a window of `window` of the original's
//! frames starting at the earliest split. It tries the numbers of the
//! mechanics that own the measures split there (speeds, accelerations,
//! gravity, timings, in their units and fixed-point steps), each one alone,
//! and keeps the value that brings the window closest to exact. Every
//! trial, and the window's own measure, starts from the original's state at
//! the clip's start, translated into the model ([`gp_interp::translate`]),
//! and takes the clip's presses: the owner's rule, 2026-10-29, "if im
//! starting at frame 100, it has to come from the rom state first". A value
//! is written only when a whole run with it agrees: no measure's first
//! split earlier, and every frame before the window still together, and
//! the caller's check (no other clip worse). When the window is exact, the
//! next round's window starts at the next earliest split. When no numbers
//! make it any closer, the loop stops and asks: a shape question naming the
//! window, the measures, both games' values and a general mechanic to add
//! or change, in the working copy's questions.toml. It never changes a
//! mechanic's shape itself.
//!
//! Values it writes are facts with evidence (the clip, the window and the
//! measured values), in the working copy's model.toml.
//!
//! The original runs through [`OriginalRuntime`], so any platform's
//! original works, and the model through [`Source`]: its game's module, or
//! the built-in interpreter for a model with no hooks.
//!
//! A [`Scope`] narrows it to one of the game's own moments: `--moment
//! jump:1` puts the windows in that moment's frames (half a second either
//! side) and tries only the numbers of the mechanic that owns that kind of
//! moment (`--mechanic` names another). A value it keeps is then measured
//! on every other moment of the same kind in the clip, each from the
//! original's state before it, with the model before and after: "also
//! better on 2 of 3 other jumps, worse on 0". The ones it makes worse
//! become questions naming their frames and what differs there.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Value as Json, json};

use gp_interp::lockstep::{self, Lockstep, Mode, Options};
use gp_interp::mapping::{MAPPING_FILE, Mapping, show};
use gp_interp::moments::{Answer, Moment};
use gp_model::{MODEL_FILE, Model};
use gp_runtime::{InputFrame, Runtime, Value};
use gp_sim::Fx;

use crate::Outcome;
use crate::clip::cache::Saved;
use crate::hosted::Source;
use crate::moments::{mechanic_of, moment_json};
use crate::original::OriginalRuntime;
use crate::solve::code_cited;
use gp_extract::apply::{Fact, apply, at};
use gp_extract::questions::{Candidate, QUESTIONS_FILE, Question};

/// The window's length in the original's frames, unless told otherwise:
/// 5 seconds at the NES's 60 frames a second (the owner, 2026-10-29: "like
/// 5 seconds" over 10 frames).
pub const DEFAULT_WINDOW: u64 = 300;

/// A window's length as given: frames ("300") or seconds ("5s", "2.5s"),
/// which become the game's frames at its model's frame rate
/// ([`WindowLen::frames`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowLen {
    Frames(u64),
    Seconds(f64),
}

impl WindowLen {
    /// In frames, at `fps_milli` frames a second times 1000 (the model's
    /// `frame_rate`, [`gp_model::model::Ratio::milli`]); at least one.
    pub fn frames(self, fps_milli: u64) -> u64 {
        match self {
            WindowLen::Frames(n) => n,
            WindowLen::Seconds(s) => ((s * fps_milli as f64 / 1000.0).round() as u64).max(1),
        }
    }
}

impl std::fmt::Display for WindowLen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WindowLen::Frames(n) => write!(f, "{n}"),
            WindowLen::Seconds(s) => write!(f, "{s}s"),
        }
    }
}

/// A window's length as given: frames ("300") or seconds ("5s", "2.5s").
pub fn parse_window(s: &str) -> Result<WindowLen, String> {
    let w = if let Some(secs) = s.strip_suffix('s') {
        let secs: f64 = secs
            .trim()
            .parse()
            .map_err(|_| format!("not a number of seconds: {s}"))?;
        if !(secs > 0.0 && secs.is_finite()) {
            return Err("the window must be at least one frame".into());
        }
        WindowLen::Seconds(secs)
    } else {
        let n: u64 = s
            .trim()
            .parse()
            .map_err(|_| format!("not a number of frames or seconds (5s): {s}"))?;
        if n == 0 {
            return Err("the window must be at least one frame".into());
        }
        WindowLen::Frames(n)
    };
    Ok(w)
}
/// The most rounds on a clip, unless told otherwise.
pub const DEFAULT_ROUNDS: u32 = 32;
/// How many of the best trials a round checks with a whole run.
const CHECKED: usize = 4;

/// One clip as the loop plays it.
pub(crate) struct LockstepArgs<'a> {
    /// The working copy: its model.toml and questions.toml are rewritten.
    pub dir: &'a Path,
    pub rom: &'a [u8],
    /// The clip's input from power-on.
    pub inputs: &'a [InputFrame],
    /// What the input is, for the evidence: `clip ID`.
    pub source: &'a str,
    /// The model as it stands, which every trial edits.
    pub model: &'a Source,
    /// The original at the clip's start cache and where a lockstep from
    /// power-on stands there, when the cache has it.
    pub resume: Option<(&'a Saved, lockstep::Prefix)>,
    /// With the mapping's `score_draws`, the original's random draws from
    /// the clip's start on.
    pub draws: Option<gp_sim::DrawScript>,
    pub rounds: u32,
    pub window: u64,
    /// The original's frame to start at: the first window is the earliest
    /// split at or after it, and frames before it are not held.
    pub from: Option<u64>,
}

/// What narrows a loop: where its runs start (a clip's start), the
/// mechanics whose numbers it may change, and a check every fit must pass
/// before it is written.
#[derive(Default)]
pub(crate) struct Scope<'a> {
    /// Start every run at this frame of the original, from its state there
    /// translated, instead of the mapping's start.
    pub start_at: Option<u64>,
    /// Only these mechanics' numbers, and only the measures they own are
    /// held; empty for any that own a split.
    pub mechanics: Vec<String>,
    /// A fit is written only if this gives `None`; `Some` says why not.
    #[allow(clippy::type_complexity)]
    pub accept: Option<&'a dyn Fn(&Model) -> Result<Option<String>>>,
    /// One of the original's moments by kind and number (`jump`, 1): the
    /// windows are in its frames, half a second either side, only the
    /// numbers of the mechanic that owns its kind are tried (unless
    /// `mechanics` names others), every measure is still held, and a kept
    /// value is measured on every other moment of the kind.
    pub moment: Option<(String, usize)>,
    /// No window past this frame of the original.
    pub until: Option<u64>,
    /// Drift thresholds set outright, by field name (`x`) or whole path:
    /// a window ends where a measure drifts past its threshold
    /// ([`lockstep::drift_thresholds`] has the defaults).
    pub drift: Vec<(String, Fx)>,
    /// The solvers' proposals on the model as it stands
    /// ([`crate::solve`]): tried after the early-stop detectors, before
    /// the fine pass. A scenario's refine gives them its own score.
    #[allow(clippy::type_complexity)]
    pub solve: Option<&'a dyn Fn(&Model) -> Result<Vec<crate::solve::Proposal>>>,
}

/// Frames a window runs past the frame it drifts apart on.
pub const DRIFT_AFTER: u64 = 10;

/// Drift thresholds as given: `x=2,vy=0.5`, a field name or a mapped path
/// and a distance in its units.
pub fn parse_drift(s: &str) -> Result<Vec<(String, Fx)>, String> {
    s.split(',')
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let (k, v) = p
                .split_once('=')
                .ok_or_else(|| format!("a drift threshold is FIELD=DISTANCE (x=2), not `{p}`"))?;
            let v = gp_model::Fixed::parse(v.trim()).map_err(|e| format!("{p}: {e}"))?;
            Ok((k.trim().to_owned(), Fx::from_raw(v.raw())))
        })
        .collect()
}

/// A moment given as `KIND:N` (`jump:1`) or `KIND` (the first).
pub fn parse_moment(s: &str) -> Result<(String, usize), String> {
    let (k, n) = s.split_once(':').unwrap_or((s, "1"));
    if !gp_interp::moments::KINDS.contains(&k) {
        return Err(format!(
            "a moment is KIND:N with a kind of {}, not `{s}`",
            gp_interp::moments::KINDS.join(", ")
        ));
    }
    let n: usize = n.parse().map_err(|_| format!("not a moment number: {s}"))?;
    if n == 0 {
        return Err("moments are numbered from 1".into());
    }
    Ok((k.to_owned(), n))
}

/// What every run of a round shares.
struct Setup<'a> {
    start_at: Option<u64>,
    rom: &'a [u8],
    /// The model as the loop started, whose module every run uses.
    base: &'a Source,
    resume: Option<(&'a Saved, lockstep::Prefix)>,
    mapping: Mapping,
    /// The mapping without its start condition, for trials that start in
    /// the middle of a run.
    from_mid: Mapping,
    tolerance: Vec<Fx>,
    /// Each mapped path's drift threshold: a window ends a few frames
    /// after the first frame a measure drifts past it, run free.
    drift: Vec<Fx>,
    rate: (u64, u64),
    /// With the mapping's `score_draws`, the original's random draws from
    /// the clip's start, which a run from a later start takes from there.
    draws: Option<gp_sim::DrawScript>,
}

impl Setup<'_> {
    /// A run of `model` at power-on, through the base's module.
    fn interp(&self, model: &Model) -> Result<crate::hosted::Model> {
        self.base.edited(model)?.open()
    }

    /// [`Self::interp`] for a run that starts from the original's frame
    /// `from`: it takes the original's draws after that frame. A run
    /// with no start takes none, since the script needs the original's
    /// frame its first load stands for.
    fn interp_at(&self, model: &Model, from: Option<u64>) -> Result<crate::hosted::Model> {
        let i = self.interp(model)?;
        match (&self.draws, from) {
            (Some(d), Some(f)) => {
                let mut d = d.clone();
                d.base = f;
                i.with_draws(d)
            }
            _ => Ok(i),
        }
    }

    /// The original at power-on.
    fn power_on(&self) -> Result<OriginalRuntime> {
        OriginalRuntime::of_rom(self.rom)
    }

    /// The whole run in resync mode.
    fn full(&self, model: &Model, inputs: &[InputFrame]) -> Result<Lockstep> {
        self.full_with(model, inputs, false, None)
    }

    /// The whole run from the clip's start, going on from the start cache
    /// when it has the clip, with `tolerance` in place of the setup's.
    fn full_with(
        &self,
        model: &Model,
        inputs: &[InputFrame],
        values: bool,
        tolerance: Option<&[Fx]>,
    ) -> Result<Lockstep> {
        let mut interp = self.interp_at(model, self.start_at)?;
        let spec = interp.spec().clone();
        let mut opts = self.opts(self.start_at, None, values);
        if let Some(t) = tolerance {
            opts.tolerance = t.to_vec();
        }
        match self.resume.filter(|_| self.start_at.is_some()) {
            Some((saved, at)) => {
                let mut original = OriginalRuntime::from_saved(self.rom, saved)?;
                lockstep::lockstep_resumed(
                    &mut original,
                    &mut interp,
                    &spec,
                    &self.mapping,
                    inputs,
                    &opts,
                    None,
                    at,
                )
            }
            None => {
                let mut original = self.power_on()?;
                lockstep::lockstep(
                    &mut original,
                    &mut interp,
                    &spec,
                    &self.mapping,
                    inputs,
                    &opts,
                )
            }
        }
        .map_err(anyhow::Error::msg)
    }

    /// The model's side of each of `found`, from the original's state
    /// before it.
    fn answers(
        &self,
        model: &Model,
        inputs: &[InputFrame],
        found: &[Moment],
        checks: &gp_interp::moments::Checks,
    ) -> Result<Vec<Answer>> {
        let mut original = self.power_on()?;
        let src = self.base.edited(model)?;
        let spec = src.spec.clone();
        let mut make =
            || -> Result<crate::hosted::Model, String> { src.open().map_err(|e| format!("{e:#}")) };
        gp_interp::moments::check(
            &mut original,
            &mut make,
            &spec,
            &self.mapping,
            inputs,
            found,
            checks,
        )
        .map_err(anyhow::Error::msg)
    }

    fn opts(&self, from: Option<u64>, state: Option<Vec<u8>>, values: bool) -> Options {
        Options {
            mode: Mode::Resync,
            tolerance: self.tolerance.clone(),
            from,
            start_state: state,
            values,
            ..Options::default()
        }
    }

    /// Seconds from frames.
    fn secs(&self, frames: u64) -> f64 {
        frames as f64 * self.rate.1 as f64 / self.rate.0.max(1) as f64
    }
}

/// Where a round starts its trials: the original at the frame its trials
/// start from, and its state there translated into the model's.
struct Mid {
    original: OriginalRuntime,
    state: Vec<u8>,
    /// The original's frame.
    frame: u64,
    /// What the translation carried over and what it couldn't.
    notes: Vec<gp_runtime::LoadNote>,
}

/// The window: the compared frames whose original frame is in
/// `from..from + len`.
#[derive(Debug, Clone, Copy)]
struct Window {
    /// The index into the whole run's frames of its first frame.
    at: usize,
    from: u64,
    to: u64,
    model_from: u64,
    model_to: u64,
}

/// How far from exact a window is: first how late its first wrong frame
/// is (later is closer), then how many times a measure split on a frame of
/// it (a measure and a frame), then the distances added up. Only the
/// measures the round aims at count ([`Aim`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Score {
    /// The original's frame of the first wrong frame, `u64::MAX` for none.
    wrong_at: std::cmp::Reverse<u64>,
    split: usize,
    distance: i128,
}

impl Score {
    fn exact(self) -> bool {
        self.split == 0
    }

    fn wrong_at(self) -> u64 {
        self.wrong_at.0
    }
}

fn score(ls: &Lockstep, from: u64, to: u64, aim: &[bool]) -> Score {
    let mut s = Score {
        wrong_at: std::cmp::Reverse(u64::MAX),
        split: 0,
        distance: 0,
    };
    for (k, (_, of)) in ls.frames.iter().enumerate() {
        if *of < from || *of > to {
            continue;
        }
        for (i, p) in ls.paths.iter().enumerate() {
            if !aim.get(i).copied().unwrap_or(true) {
                continue;
            }
            let d = p.distance[k];
            if d > p.tolerance {
                s.split += 1;
                s.distance += i128::from(d.raw());
                if s.wrong_at.0 == u64::MAX {
                    s.wrong_at = std::cmp::Reverse(*of);
                }
            }
        }
    }
    s
}

/// Which measures a round works on and which it guards. While a body is
/// wrong, the camera is left out of both: it follows the bodies, so its
/// splits are not a target of their own (the user, 2026-10-29: "the
/// background drifting apart is a separate and easily noticeable thing").
/// Once the bodies match, the round aims at the camera's measures alone
/// and guards every measure.
struct Aim {
    aim: Vec<bool>,
    guard: Vec<bool>,
}

impl Aim {
    fn new(ls: &Lockstep, camera: bool) -> Self {
        let cam: Vec<bool> = ls
            .paths
            .iter()
            .map(|p| lockstep::is_camera(&p.path))
            .collect();
        Aim {
            aim: cam.iter().map(|c| *c == camera).collect(),
            guard: cam.iter().map(|c| camera || !*c).collect(),
        }
    }
}

/// The first frame at or after the original's frame `from` on which a
/// measure in `mask` is past its tolerance: its index into the frames, the
/// original's frame, and the measures wrong there.
fn first_on(ls: &Lockstep, from: u64, mask: &[bool]) -> Option<(usize, u64, Vec<usize>)> {
    ls.frames.iter().enumerate().find_map(|(k, (_, of))| {
        if *of < from {
            return None;
        }
        let wrong: Vec<usize> = (0..ls.paths.len())
            .filter(|&i| {
                mask.get(i).copied().unwrap_or(true)
                    && ls.paths[i].distance[k] > ls.paths[i].tolerance
            })
            .collect();
        (!wrong.is_empty()).then_some((k, *of, wrong))
    })
}

/// Frames on which no measure in `mask` is past its tolerance.
fn together_on(ls: &Lockstep, mask: &[bool]) -> usize {
    (0..ls.frames.len())
        .filter(|&k| {
            ls.paths
                .iter()
                .enumerate()
                .all(|(i, p)| !mask.get(i).copied().unwrap_or(true) || p.distance[k] <= p.tolerance)
        })
        .count()
}

/// Each path's first split at or after the original's frame `cut`, as
/// the original's frame; `None` for a path outside `mask`.
fn firsts(ls: &Lockstep, cut: u64, mask: &[bool]) -> Vec<Option<u64>> {
    ls.paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if !mask.get(i).copied().unwrap_or(true) {
                return None;
            }
            ls.frames
                .iter()
                .zip(&p.distance)
                .find(|((_, of), d)| *of >= cut && **d > p.tolerance)
                .map(|((_, of), _)| *of)
        })
        .collect()
}

/// The earliest split at or after the original's frame `cut`.
fn earliest(ls: &Lockstep, cut: u64) -> Option<&lockstep::Split> {
    ls.splits.iter().find(|s| s.original_frame >= cut)
}

fn together(ls: &Lockstep) -> usize {
    ls.frames.len() - ls.splits.len()
}

/// A number the loop may change: its model path, value and unit.
#[derive(Debug, Clone)]
struct Knob {
    mechanic: String,
    path: String,
    value: Fx,
    /// `px/frame`, `px/frame^2`, `frames`, `px` or empty.
    unit: String,
    /// Frames and counts are whole numbers.
    whole: bool,
    range: String,
}

fn fx_of(v: &toml::Value) -> Option<Fx> {
    match v {
        toml::Value::Integer(i) => Some(Fx::from_raw(i.checked_mul(1 << 16)?)),
        toml::Value::String(s) => gp_model::Fixed::parse(s)
            .ok()
            .map(|f| Fx::from_raw(f.raw())),
        _ => None,
    }
}

fn get<'a>(tree: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let p = gp_model::Path::parse(path).ok()?;
    let mut node = tree;
    for s in &p.segments {
        node = node.get(s.name.as_str())?;
        if let Some(gp_model::path::Index::At(i)) = s.index {
            node = node.get(usize::try_from(i).ok()?)?;
        }
    }
    Some(node)
}

/// The part of the model a mapped path belongs to, as the catalog's
/// `used_by` names it: `entities.mario[0].x` is `entities.mario`.
fn part_of(path: &str) -> String {
    let Ok(p) = gp_model::Path::parse(path) else {
        return String::new();
    };
    let n: Vec<&str> = p.segments.iter().map(|s| s.name.as_str()).collect();
    match n.as_slice() {
        ["entities" | "counters" | "cameras" | "rules", k, ..] => format!("{}.{k}", n[0]),
        _ => String::new(),
    }
}

/// The numbers of `mechanics` the model sets for `part`: each parameter
/// that is a fixed-point number, a frame count or a count, and each
/// number in a list parameter's items (a jump's bands).
fn knobs(
    catalog: &gp_interp::mechanics::Catalog,
    tree: &toml::Value,
    mechanics: &[&str],
    part: &str,
) -> Vec<Knob> {
    let mut out = Vec::new();
    for name in mechanics {
        let Some(m) = catalog.get(name) else { continue };
        for u in &m.used_by {
            if !part.is_empty() && u != part && !u.starts_with(&format!("{part}.")) {
                continue;
            }
            // A component's parameters are in its table; a field such as
            // `tile_collision` has its parameters beside it.
            let base = match get(tree, u) {
                Some(toml::Value::Table(_)) => u.clone(),
                _ => u
                    .rsplit_once('.')
                    .map_or(String::new(), |(a, _)| a.to_owned()),
            };
            let Some(toml::Value::Table(t)) = get(tree, &base) else {
                continue;
            };
            for p in &m.params {
                let numeric = matches!(p.ty, "fixed" | "frames" | "count");
                match t.get(p.name) {
                    Some(v) if numeric => {
                        if let Some(value) = fx_of(v) {
                            out.push(Knob {
                                mechanic: (*name).to_owned(),
                                path: format!("{base}.{}", p.name),
                                value,
                                unit: p.unit.to_owned(),
                                whole: p.ty != "fixed",
                                range: p.range.to_owned(),
                            });
                        }
                    }
                    Some(toml::Value::Array(items)) if p.ty == "list" => {
                        for (i, item) in items.iter().enumerate() {
                            let Some(it) = item.as_table() else { continue };
                            for (k, v) in it {
                                let Some(value) = fx_of(v) else { continue };
                                // An item's field takes the unit of the
                                // parameter it is named for.
                                let like = m.params.iter().find(|q| q.name == k.as_str());
                                out.push(Knob {
                                    mechanic: (*name).to_owned(),
                                    path: format!("{base}.{}[{i}].{k}", p.name),
                                    value,
                                    unit: like.map_or("", |q| q.unit).to_owned(),
                                    whole: like.is_some_and(|q| q.ty != "fixed"),
                                    range: like.map_or("", |q| q.range).to_owned(),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    out.retain(|k| seen.insert(k.path.clone()));
    out
}

/// Whether `name` is `ext:HOOK` for a hook some entity runs: how a hook
/// that describes no mechanic is named to `--mechanic`.
fn hook_named(model: &Model, name: &str) -> bool {
    name.strip_prefix("ext:").is_some_and(|h| {
        model
            .entities
            .values()
            .any(|e| e.core.ext.as_deref() == Some(h))
    })
}

/// Whether the hook mechanic `name` (`ext:HOOK`, or a mechanic a hook's
/// record describes) owns the mapped `path`: a field of an entity running
/// the hook, or a counter the hook may write.
fn hook_owns(
    model: &Model,
    catalog: &gp_interp::mechanics::Catalog,
    name: &str,
    path: &str,
) -> bool {
    let hook = match name.strip_prefix("ext:") {
        Some(h) => h.to_owned(),
        None => match catalog.get(name).and_then(|m| m.hook.clone()) {
            Some(h) => h,
            None => return false,
        },
    };
    let part = part_of(path);
    let runs = |entity_part: &str| {
        entity_part
            .strip_prefix("entities.")
            .and_then(|e| model.entities.get(e))
            .is_some_and(|e| e.core.ext.as_deref() == Some(hook.as_str()))
    };
    if runs(&part) {
        return true;
    }
    part.strip_prefix("counters.")
        .is_some_and(|c| counter_hooks(model, catalog, c).iter().any(|p| runs(p)))
}

/// The entities whose hooks may write the counter `c`: a hook whose
/// records write `counters` or `counters.C`, or one that describes nothing
/// (gp_validate's rule for a counter a hook keeps).
fn counter_hooks(model: &Model, catalog: &gp_interp::mechanics::Catalog, c: &str) -> Vec<String> {
    let one = format!("counters.{c}");
    model
        .entities
        .iter()
        .filter(|(_, e)| {
            let Some(hook) = e.core.ext.as_ref() else {
                return false;
            };
            let records: Vec<&gp_interp::mechanics::Mechanic> = catalog
                .mechanics
                .iter()
                .filter(|m| m.hook.as_deref() == Some(hook.as_str()))
                .collect();
            records.is_empty()
                || records
                    .iter()
                    .any(|m| m.writes.iter().any(|w| *w == "counters" || *w == one))
        })
        .map(|(n, _)| format!("entities.{n}"))
        .collect()
}

/// The numbers of `part`'s hook: each number in the entity's `ext_params`
/// (a hook's own numbers, such as a walk's step), by the mechanic the hook
/// describes for it, or `ext:HOOK` for a hook that describes none.
fn hook_knobs(
    model: &Model,
    catalog: &gp_interp::mechanics::Catalog,
    tree: &toml::Value,
    part: &str,
) -> Vec<Knob> {
    let Some(entity) = part.strip_prefix("entities.") else {
        return Vec::new();
    };
    let Some(hook) = model.entities.get(entity).and_then(|e| e.core.ext.as_ref()) else {
        return Vec::new();
    };
    let base = format!("{part}.ext_params");
    let Some(toml::Value::Table(t)) = get(tree, &base) else {
        return Vec::new();
    };
    let records: Vec<&gp_interp::mechanics::Mechanic> = catalog
        .mechanics
        .iter()
        .filter(|m| m.hook.as_deref() == Some(hook.as_str()))
        .collect();
    let mut out = Vec::new();
    for (name, v) in t {
        let Some(value) = fx_of(v) else { continue };
        let declared = records.iter().find_map(|m| {
            m.params
                .iter()
                .find(|p| p.name == name.as_str())
                .map(|p| (m.name, p))
        });
        let (mechanic, unit, whole, range) = match declared {
            // A number the model writes whole (an integer) is tried whole:
            // a hook reads it as a count or a step in pixels.
            Some((m, p)) => (
                m.to_owned(),
                p.unit.to_owned(),
                p.ty != "fixed" || v.is_integer(),
                p.range.to_owned(),
            ),
            // An integer the hook reads, with nothing declared: a whole
            // number, as a frame count or a step in pixels is.
            None => (
                format!("ext:{hook}"),
                String::new(),
                v.is_integer(),
                String::new(),
            ),
        };
        out.push(Knob {
            mechanic,
            path: format!("{base}.{name}"),
            value,
            unit,
            whole,
            range,
        });
    }
    out
}

fn in_range(range: &str, v: Fx) -> bool {
    match range.trim() {
        "> 0" => v > Fx::ZERO,
        "< 0" => v < Fx::ZERO,
        ">= 0" => v >= Fx::ZERO,
        "<= 0" => v <= Fx::ZERO,
        _ => true,
    }
}

fn num(v: &Value) -> Option<i64> {
    match v {
        Value::Fixed(r) => Some(*r),
        Value::Int(n) => Some(n << 16),
        Value::Entity(n) => Some((*n as i64) << 16),
        _ => None,
    }
}

/// The values a knob is tried at that the original's own numbers give:
/// for an acceleration, how much a measured speed changes from frame to
/// frame in the window; for a speed, the speeds and how far a position
/// moves a frame.
fn measured(k: &Knob, trial: &Lockstep, idx: &[usize]) -> Vec<Fx> {
    let mut out: Vec<Fx> = Vec::new();
    let mut push = |raw: i64| {
        let mut v = Fx::from_raw(raw.abs());
        if k.range.trim().starts_with('<') {
            v = Fx::from_raw(-v.raw());
        }
        if raw != 0 && !out.contains(&v) && in_range(&k.range, v) && out.len() < 8 {
            out.push(v);
        }
    };
    for &i in idx {
        let series: Vec<i64> = trial
            .values
            .iter()
            .filter_map(|(o, _)| num(&o[i]))
            .collect();
        let field = trial.paths[i].path.rsplit('.').next().unwrap_or("");
        let speed = field.starts_with('v');
        for w in series.windows(2) {
            let d = w[1] - w[0];
            match k.unit.as_str() {
                "px/frame^2" if speed => push(d),
                "px/frame" if !speed => push(d),
                _ => {}
            }
        }
        if k.unit == "px/frame" && speed {
            for v in &series {
                push(*v);
            }
        }
    }
    out
}

/// The model with one number changed.
fn with(text_tree: &toml::Value, path: &str, v: Fx) -> Result<Model> {
    let mut tree = text_tree.clone();
    let p = gp_model::Path::parse(path).map_err(anyhow::Error::msg)?;
    let (last, parents) = p.segments.split_last().context("an empty path")?;
    let mut node = &mut tree;
    for s in parents {
        node = node
            .get_mut(s.name.as_str())
            .with_context(|| format!("{path}: no {}", s.name))?;
        if let Some(gp_model::path::Index::At(i)) = s.index {
            node = usize::try_from(i)
                .ok()
                .and_then(|i| node.get_mut(i))
                .with_context(|| format!("{path}: no item {i}"))?;
        }
    }
    let t = node.as_table_mut().context("not a table")?;
    t.insert(last.name.clone(), value_toml(v));
    let text = toml::to_string(&tree)?;
    Model::parse(&text).map_err(|e| anyhow::anyhow!("{path} = {v}: {e}"))
}

fn value_toml(v: Fx) -> toml::Value {
    let s = v.to_string();
    match s.parse::<i64>() {
        Ok(i) => toml::Value::Integer(i),
        Err(_) => toml::Value::String(s),
    }
}

/// One trial: the model run through the window from the original's state
/// at the frame its trials start from.
fn trial(
    setup: &Setup,
    mid: &Mid,
    model: &Model,
    inputs: &[InputFrame],
    w: Window,
    values: bool,
) -> Result<Lockstep> {
    let mut original = mid.original.clone();
    let mut interp = setup.interp_at(model, Some(mid.frame))?;
    let spec = interp.spec().clone();
    let end = usize::try_from(w.to + 1)
        .unwrap_or(usize::MAX)
        .min(inputs.len());
    let start = usize::try_from(mid.frame).unwrap_or(usize::MAX).min(end);
    lockstep::lockstep(
        &mut original,
        &mut interp,
        &spec,
        &setup.from_mid,
        &inputs[start..end],
        &setup.opts(Some(mid.frame), Some(mid.state.clone()), values),
    )
    .map_err(anyhow::Error::msg)
}

/// The original at `frame`, and its state there translated into the
/// model's, as a scenario starts ([`lockstep::start_at`]): the model's
/// numbers have no say in where a trial starts.
fn mid(setup: &Setup, model: &Model, inputs: &[InputFrame], frame: u64) -> Result<Mid> {
    let power = setup.power_on()?;
    let mut interp = setup.interp(model)?;
    let spec = interp.spec().clone();
    let (original, start) =
        lockstep::start_at(&power, &mut interp, &spec, &setup.mapping, inputs, frame)
            .map_err(anyhow::Error::msg)?;
    Ok(Mid {
        state: start.state,
        frame: start.original_frame,
        notes: start.notes,
        original,
    })
}

/// A window's start in words and JSON: the original's frame it was taken
/// at, and what the translation couldn't carry over ("started from the
/// original's state at 3.3 s; not carried over: slug (y, vx)").
fn start_json(setup: &Setup, m: &Mid) -> Json {
    use gp_runtime::LoadHow;
    let count = |h: LoadHow| m.notes.iter().filter(|n| n.how == h).count();
    let missing: Vec<&str> = m
        .notes
        .iter()
        .filter(|n| n.how == LoadHow::Missing)
        .map(|n| n.path.as_str())
        .collect();
    let says = format!(
        "started from the original's state at {:.1} s; not carried over: {}",
        setup.secs(m.frame),
        if missing.is_empty() {
            "nothing".to_owned()
        } else {
            not_carried(&missing)
        }
    );
    json!({"original_frame": m.frame, "seconds": setup.secs(m.frame),
           "loaded": count(LoadHow::Loaded), "approximated": count(LoadHow::Approximated),
           "missing": missing, "says": says})
}

/// Missing paths, shortened: an entity kind's fields together without
/// their instances (`slug (y, vx)`), everything else as it is.
fn not_carried(paths: &[&str]) -> String {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for p in paths {
        let (head, field) = match p.strip_prefix("entities.").and_then(|r| r.split_once('.')) {
            Some((kind, field)) => (
                kind.split('[').next().unwrap_or(kind).to_owned(),
                Some(field.to_owned()),
            ),
            None => ((*p).to_owned(), None),
        };
        let g = match groups.iter().position(|(h, _)| *h == head) {
            Some(i) => &mut groups[i],
            None => {
                groups.push((head, Vec::new()));
                groups.last_mut().unwrap()
            }
        };
        if let Some(f) = field
            && !g.1.contains(&f)
        {
            g.1.push(f);
        }
    }
    groups
        .iter()
        .map(|(h, fs)| {
            if fs.is_empty() {
                h.clone()
            } else {
                format!("{h} ({})", fs.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The values to try a knob at, after the measured ones: steps of a
/// whole, a sixteenth, a 256th, a 4096th and a 65536th, out from the best
/// so far, keeping each step that helps.
fn climb(k: &Knob, start: (Fx, Score), eval: &mut dyn FnMut(Fx) -> Option<Score>) -> (Fx, Score) {
    let (mut best, mut best_score) = start;
    let steps: &[i64] = if k.whole {
        &[1 << 16]
    } else {
        &[1 << 16, 1 << 12, 1 << 8, 1 << 4, 1]
    };
    for &step in steps {
        for _ in 0..8 {
            let mut moved = false;
            for m in [1i64, -1, 2, -2, 4, -4] {
                let v = Fx::from_raw(best.raw() + m * step);
                if !in_range(&k.range, v) {
                    continue;
                }
                if let Some(s) = eval(v)
                    && s < best_score
                {
                    best = v;
                    best_score = s;
                    moved = true;
                    break;
                }
            }
            if !moved || best_score.exact() {
                break;
            }
        }
        if best_score.exact() {
            break;
        }
    }
    (best, best_score)
}

/// Whether `a` moves its number (from `a_old`) less than `b` moves its own
/// (from `b_old`), each relative to its old value: |a - a_old| / |a_old|
/// against |b - b_old| / |b_old|, with an old value of zero counted as one
/// raw step, in whole numbers so the order is the same everywhere.
fn less_change(a: Fx, a_old: Fx, b: Fx, b_old: Fx) -> bool {
    let d = |v: Fx, o: Fx| (i128::from(v.raw()) - i128::from(o.raw())).abs();
    let base = |o: Fx| i128::from(o.raw()).abs().max(1);
    d(a, a_old) * base(b_old) < d(b, b_old) * base(a_old)
}

fn window_json(setup: &Setup, w: Window) -> Json {
    json!({
        "from": w.model_from,
        "to": w.model_to,
        "original_from": w.from,
        "original_to": w.to,
        "seconds_from": setup.secs(w.model_from),
        "seconds_to": setup.secs(w.model_to),
    })
}

/// The window at the first wrong frame of `ls` at or after `cut`
/// ([`lockstep::first_wrong`]: a body's before the camera's), and that
/// frame.
fn window_at(ls: &Lockstep, len: u64, cut: u64) -> Option<(Window, lockstep::FirstWrong)> {
    let s = lockstep::first_wrong(ls, cut)?;
    let from = s.original_frame;
    let to = from + len.max(1) - 1;
    let last = ls
        .frames
        .iter()
        .rev()
        .find(|(_, o)| *o <= to)
        .copied()
        .unwrap_or((s.frame, from));
    Some((
        Window {
            at: s.at,
            from,
            to,
            model_from: s.frame,
            model_to: last.0,
        },
        s,
    ))
}

/// Why no numbers make a measure exact in the window, read from the
/// original's own values there, and the general mechanic to propose.
fn why(
    catalog: &gp_interp::mechanics::Catalog,
    tree: &toml::Value,
    trial: &Lockstep,
    i: usize,
    mechanic: &str,
) -> (String, String) {
    let path = &trial.paths[i].path;
    let series: Vec<Option<i64>> = trial.values.iter().map(|(o, _)| num(&o[i])).collect();
    let field = path.rsplit('.').next().unwrap_or("");
    let fx = |r: i64| Fx::from_raw(r).to_string();
    let deltas: Vec<i64> = series
        .windows(2)
        .filter_map(|w| Some(w[1]? - w[0]?))
        .collect();
    let list = |v: &[i64]| v.iter().map(|d| fx(*d)).collect::<Vec<_>>().join(", ");
    // An optional table or list the mechanic offers and the model leaves
    // out: the general option to try before a new mechanic.
    let unused = catalog.get(mechanic).and_then(|m| {
        m.params.iter().find(|p| {
            p.optional
                && matches!(p.ty, "table" | "list")
                && !m
                    .used_by
                    .iter()
                    .any(|u| get(tree, u).and_then(|t| t.get(p.name)).is_some())
        })
    });
    if series.iter().all(Option::is_none) {
        return (
            format!("the original's {path} changes where the model's doesn't"),
            format!(
                "a general rule in {mechanic} for when {path} changes, or a new general mechanic that changes it"
            ),
        );
    }
    let distinct: BTreeSet<i64> = deltas.iter().copied().filter(|d| *d != 0).collect();
    if field.starts_with('v') && distinct.len() > 1 {
        let why = format!(
            "the original's {field} changes by {} from frame to frame: one number gives the same change every frame",
            list(&deltas)
        );
        let proposal = match unused {
            Some(p) => format!(
                "use {mechanic}'s `{}` ({}), a general option the model doesn't set yet, to give the changes frame by frame",
                p.name, p.unit
            ),
            None => format!(
                "a general table for {mechanic}: the change in {field} by frames since it began, read from the table"
            ),
        };
        return (why, proposal);
    }
    // A position beside its speed: does it move by it?
    let speed = format!("v{field}");
    let sibling = trial.paths.iter().position(|p| {
        p.path.rsplit_once('.') == path.rsplit_once('.').map(|(a, _)| (a, speed.as_str()))
    });
    if let Some(j) = sibling {
        let vs: Vec<i64> = trial
            .values
            .iter()
            .skip(1)
            .filter_map(|(o, _)| num(&o[j]))
            .collect();
        if !deltas.is_empty() && deltas.iter().zip(&vs).any(|(d, v)| d != v) {
            return (
                format!(
                    "the original's {field} moves by {} a frame while its {speed} is {}: it doesn't move by its speed",
                    list(&deltas),
                    list(&vs)
                ),
                format!(
                    "a general option for how {mechanic} moves a body by its speed (the fraction carried apart from the speed, or the move before the speed's change), once the mapping's fraction for {field} is checked"
                ),
            );
        }
    }
    (
        format!(
            "the original's {field} goes {} in the window, and no single value of {mechanic}'s numbers gives that",
            series
                .iter()
                .map(|v| v.map_or("-".into(), fx))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        match unused {
            Some(p) => format!(
                "use {mechanic}'s `{}` ({}), a general option the model doesn't set yet",
                p.name, p.unit
            ),
            None => format!(
                "a new general mechanic, or a new option of {mechanic}, that gives this {field}"
            ),
        },
    )
}

/// The loop on one clip, narrowed by `scope`. `progress` hears each
/// window, target, impact pass and round as JSON as it goes.
pub(crate) fn run_scoped(
    a: &LockstepArgs<'_>,
    scope: &Scope<'_>,
    progress: &mut dyn FnMut(&Json),
) -> Result<Outcome> {
    let model_file = a.dir.join(MODEL_FILE);
    let text = fs::read_to_string(&model_file)
        .with_context(|| format!("reading {}", model_file.display()))?;
    let mut model = Model::parse(&text).with_context(|| model_file.display().to_string())?;
    let mapping_text = fs::read_to_string(a.dir.join(MAPPING_FILE))
        .with_context(|| format!("{} has no state mapping", a.dir.display()))?;
    let mapping = Mapping::parse(&mapping_text).map_err(anyhow::Error::msg)?;
    let mut from_mid = mapping.clone();
    from_mid.start = None;
    let exts = a.model.exts.clone();
    let (mut tolerance, _) = crate::lockstep::tolerances(a.dir, &model, &mapping);
    let paths: Vec<String> = mapping.fields.iter().map(|f| f.path.clone()).collect();
    let mut drift = lockstep::drift_thresholds(&paths, &tolerance, &scope.drift);
    // Every measure held as the contract says, for the user's rules on a
    // moment's loop, which holds only its mechanic's measures.
    let strict = tolerance.clone();
    if !scope.mechanics.is_empty() {
        // A measure no scoped mechanic owns is not held: its splits are
        // another mechanic's to fix, and `accept` keeps them from getting
        // worse.
        let catalog = gp_interp::mechanics::catalog(&model, &exts);
        for (i, f) in mapping.fields.iter().enumerate() {
            let owned = lockstep::owners(&catalog, &f.path)
                .iter()
                .any(|o| scope.mechanics.iter().any(|m| m == o))
                || scope
                    .mechanics
                    .iter()
                    .any(|m| hook_owns(&model, &catalog, m, &f.path));
            if !owned && let Some(t) = tolerance.get_mut(i) {
                *t = Fx::from_raw(i64::MAX);
            }
        }
    }
    // A measure that isn't held doesn't stop a window either.
    for (d, t) in drift.iter_mut().zip(&tolerance) {
        if t.raw() == i64::MAX {
            *d = *t;
        }
    }
    let rate = (model.model.frame_rate.num, model.model.frame_rate.den);
    let mut setup = Setup {
        draws: a.draws.clone(),
        drift,
        start_at: scope.start_at,
        rom: a.rom,
        base: a.model,
        resume: a.resume,
        mapping,
        from_mid,
        tolerance,
        rate,
    };
    let mut rounds: Vec<Json> = Vec::new();
    let mut text_out = String::new();
    let mut cut = a.from.unwrap_or(0);
    let start_model = model.clone();
    let clocks = gp_interp::moments::clocks(&model.to_tree());
    // A moment: its frames and its mechanic, and the others of its kind.
    let mut until = scope.until;
    let mut mechanic = scope.mechanics.first().cloned();
    let mut target: Option<(Moment, Vec<Moment>, u64)> = None;
    // Where a scenario's, a clip's or a moment's trials start: its own
    // start, the original's state there, so the presses that make the
    // motion are in the trial (a jump's takeoff, which sets its band and
    // held phase), whichever frame of it the window is at.
    let mut anchor = scope.start_at;
    if let Some((kind, n)) = &scope.moment {
        let whole = setup.full_with(&model, a.inputs, true, None)?;
        let same: Vec<Moment> =
            gp_interp::moments::detect(&whole, gp_interp::moments::Side::Original, &clocks)
                .into_iter()
                .filter(|m| m.kind == kind)
                .collect();
        let m = same.iter().find(|m| m.n == *n).cloned().with_context(|| {
            format!(
                "the clip has {} {kind} moments, so no {kind} {n}",
                same.len()
            )
        })?;
        let first = whole.frames.first().map_or(0, |f| f.1);
        let (f, t) = gp_interp::moments::span(&m, gp_interp::moments::LEAD, first);
        cut = cut.max(f);
        anchor = Some(anchor.map_or(f, |s| s.max(f)));
        until = Some(until.map_or(t, |u| u.min(t)));
        if mechanic.is_none() {
            let catalog = gp_interp::mechanics::catalog(&model, &exts);
            mechanic = mechanic_of(&catalog, &whole, &m).map(str::to_owned);
        }
        // The moment's loop holds only its mechanic's measures (a jump's
        // isn't stopped by a run's), as a scenario's does; the user's rules
        // are checked on every measure below.
        if scope.mechanics.is_empty()
            && let Some(name) = &mechanic
        {
            let catalog = gp_interp::mechanics::catalog(&model, &exts);
            for (i, f) in setup.mapping.fields.iter().enumerate() {
                let owned = lockstep::owners(&catalog, &f.path)
                    .iter()
                    .any(|o| o == name);
                if !owned {
                    setup.tolerance[i] = Fx::from_raw(i64::MAX);
                    setup.drift[i] = Fx::from_raw(i64::MAX);
                }
            }
        }
        target = Some((m, same, first));
    }
    if let Some(name) = &mechanic
        && gp_interp::mechanics::catalog(&model, &exts)
            .get(name)
            .is_none()
        && !hook_named(&model, name)
    {
        bail!("the model's catalog has no mechanic {name} (gp mechanics list names them)");
    }
    let stopped = || false;
    let mut ls = setup.full(&model, a.inputs)?;
    // The user's rules on every measure, for a moment's loop: no measure's
    // first split earlier, no fewer frames together.
    let strict_on = target.is_some() && setup.tolerance != strict;
    let strict_run = |setup: &Setup, m: &Model| -> Result<Lockstep> {
        setup.full_with(m, a.inputs, false, Some(&strict))
    };
    let mut strict_cur = if strict_on {
        Some(strict_run(&setup, &model)?)
    } else {
        None
    };
    let first_before = earliest(&ls, cut).map(|s| s.frame);
    let together_before = together(&ls);
    let mut questions: Vec<Json> = Vec::new();
    let stop;
    let mut round = 0u32;
    let mut windows = 0u32;
    let mut last_window: Option<u64> = None;
    let mut exact_windows: Vec<Json> = Vec::new();
    // Measures wrong first that no mechanic's numbers write (an arcade
    // game's score, read through the mapping but not yet modelled):
    // recorded, and not held from there on, so the loop goes on to the
    // next window a mechanic owns.
    let mut unheld: Vec<Json> = Vec::new();
    loop {
        let Some((mut w, fw)) = window_at(&ls, a.window, cut) else {
            stop = "every measure is exact on every frame".into();
            break;
        };
        if let Some(u) = until {
            if w.from > u {
                stop = format!("every measure is exact up to frame {u}");
                break;
            }
            w = clip(&ls, w, u);
        }
        if stopped() {
            stop = "stopped".into();
            break;
        }
        if last_window != Some(w.from) {
            windows += 1;
            last_window = Some(w.from);
        }
        if round >= a.rounds {
            stop = format!("{} rounds ran", a.rounds);
            break;
        }
        round += 1;
        // The round aims at the bodies' measures while one is wrong, and at
        // the camera's once they match.
        let aim = Aim::new(&ls, fw.camera);
        let cur_first = firsts(&ls, cut, &aim.guard);
        let cur_together = together_on(&ls, &aim.guard);
        let catalog = gp_interp::mechanics::catalog(&model, &exts);
        let tree = model.to_tree();
        // The measures wrong on the first wrong frame, and only the
        // mechanics that own them (the user, 2026-10-29: "its like ONE
        // frame in things are already WRONG"). A measure wrong only later
        // in the window waits until it is the first wrong one.
        let mut split_paths: Vec<usize> = fw.paths.clone();
        let owners_of = |paths: &[usize]| -> Vec<(&'static str, String)> {
            let mut owners: Vec<(&'static str, String)> = Vec::new();
            for &i in paths {
                let part = part_of(&ls.paths[i].path);
                for o in lockstep::owners(&catalog, &ls.paths[i].path) {
                    if !scope.mechanics.is_empty() && !scope.mechanics.iter().any(|m| m == o) {
                        continue;
                    }
                    if !owners.iter().any(|(n, p)| *n == o && *p == part) {
                        owners.push((o, part.clone()));
                    }
                }
            }
            // A moment's loop tries only its mechanics' numbers, for the
            // parts wrong here and the moment's own, even where those
            // mechanics own no measure wrong there.
            if let Some((m, ..)) = &target {
                let names: Vec<&'static str> = if scope.mechanics.is_empty() {
                    mechanic
                        .iter()
                        .filter_map(|n| catalog.get(n).map(|x| x.name))
                        .collect()
                } else {
                    scope
                        .mechanics
                        .iter()
                        .filter_map(|n| catalog.get(n).map(|x| x.name))
                        .collect()
                };
                let mut parts: Vec<String> =
                    paths.iter().map(|&i| part_of(&ls.paths[i].path)).collect();
                parts.push(part_of(&format!("{}.x", m.part)));
                parts.sort();
                parts.dedup();
                owners = names
                    .iter()
                    .flat_map(|n| parts.iter().map(move |p| (*n, p.clone())))
                    .collect();
            }
            owners
        };
        let knobs_of = |owners: &[(&'static str, String)], paths: &[usize]| -> Vec<Knob> {
            let mut ks: Vec<Knob> = Vec::new();
            for (o, part) in owners {
                for k in knobs(&catalog, &tree, &[o], part) {
                    if !ks.iter().any(|x| x.path == k.path) {
                        ks.push(k);
                    }
                }
            }
            // The numbers of each wrong part's hook (its `ext_params`), tried
            // like any mechanic's, whether its record describes them or not.
            let mut parts: Vec<String> =
                paths.iter().map(|&i| part_of(&ls.paths[i].path)).collect();
            if let Some((m, ..)) = &target {
                parts.push(part_of(&format!("{}.x", m.part)));
            }
            // A counter's hooks: each that says it writes counters (or that
            // one), and each that describes nothing, which may write any.
            let counters: Vec<String> = parts
                .iter()
                .filter_map(|p| p.strip_prefix("counters."))
                .map(str::to_owned)
                .collect();
            for c in &counters {
                parts.extend(counter_hooks(&model, &catalog, c));
            }
            parts.sort();
            parts.dedup();
            for part in &parts {
                for k in hook_knobs(&model, &catalog, &tree, part) {
                    let wanted = |n: &str| {
                        scope.mechanics.is_empty() || scope.mechanics.iter().any(|m| m == n)
                    };
                    let in_moment =
                        target.is_none() || mechanic.as_deref().is_some_and(|n| n == k.mechanic);
                    if wanted(&k.mechanic) && in_moment && !ks.iter().any(|x| x.path == k.path) {
                        ks.push(k);
                    }
                }
            }
            ks
        };
        let mut owners = owners_of(&split_paths);
        let mut ks = knobs_of(&owners, &split_paths);
        if ks.is_empty() && target.is_none() && !split_paths.is_empty() {
            let frame = ls.frames.get(w.at).map_or(w.from, |f| f.1);
            let says = unhold(&mut setup, &ls, &split_paths, frame, &mut unheld);
            let _ = writeln!(text_out, "{says}");
            progress(&json!({"unheld": unheld.last()}));
            ls = setup.full(&model, a.inputs)?;
            round -= 1;
            continue;
        }
        if w.at == 0 {
            stop = "the two part on the first frame measured, before either takes a step: the model's starting state, not a mechanic's numbers".into();
            questions.push(shape_question(
                &setup,
                &ls,
                w,
                &split_paths,
                None,
                &catalog,
                &tree,
                &owners,
                &ks,
                &stop,
            ));
            break;
        }
        // The window's start: the original's state at the frame before it,
        // translated, whatever the model's numbers did up to there.
        let before_window = ls.frames[w.at - 1].1;
        let m = mid(
            &setup,
            &model,
            a.inputs,
            anchor
                .filter(|s| *s <= before_window)
                .unwrap_or(before_window),
        )?;
        // Measured from the frame after the start: what the trial plays.
        let lo = (m.frame + 1).min(w.from);
        let start_j = start_json(&setup, &m);
        // Where the two drift apart, run free from the window's start: the
        // window ends a few frames after it.
        let drift_on: Vec<Fx> = setup
            .drift
            .iter()
            .zip(&aim.aim)
            .map(|(d, on)| if *on { *d } else { Fx::from_raw(i64::MAX) })
            .collect();
        let drifted = drift_in(&setup, &drift_on, &m, &model, a.inputs, w)?;
        if let Some(d) = &drifted {
            w = clip(&ls, w, (d.original_frame + DRIFT_AFTER).max(w.from));
        }
        let drift_j = drifted.as_ref().map(|d| drift_json(&setup, d, w));
        progress(
            &json!({"window": window_json(&setup, w), "round": round, "drift": drift_j, "start": start_j}),
        );
        // The window is measured from that start too, with the same presses.
        let base = trial(&setup, &m, &model, a.inputs, w, true)?;
        let before = score(&base, lo, w.to, &aim.aim);
        let base_score = before;
        if before.exact() {
            // Exact from the original's state: the whole run parts here
            // from what it carries in from before the window, and no
            // number of this window's would change that. On to the next.
            let _ = writeln!(
                text_out,
                "frames {} to {} ({:.1} to {:.1} s): exact from the original's state; {}",
                w.model_from,
                w.model_to,
                setup.secs(w.model_from),
                setup.secs(w.model_to),
                start_j["says"].as_str().unwrap_or("")
            );
            exact_windows.push(json!({"window": window_json(&setup, w), "start": start_j}));
            cut = w.to + 1;
            round -= 1;
            continue;
        }
        // The first wrong frame from the original's state, and only the
        // mechanics that own the measures wrong there.
        if let Some((_, _, wrong)) = first_on(&base, lo, &aim.aim) {
            split_paths = wrong;
            owners = owners_of(&split_paths);
            ks = knobs_of(&owners, &split_paths);
        }
        if ks.is_empty() && target.is_none() && !split_paths.is_empty() {
            let frame = first_on(&base, lo, &aim.aim).map_or(w.from, |(_, f, _)| f);
            let says = unhold(&mut setup, &base, &split_paths, frame, &mut unheld);
            let _ = writeln!(text_out, "{says}");
            progress(&json!({"unheld": unheld.last()}));
            ls = setup.full(&model, a.inputs)?;
            round -= 1;
            continue;
        }
        let target_j = target_json(
            &base,
            lo,
            &aim.aim,
            scope.start_at.or(a.from).unwrap_or(0),
            &owners,
            &ks,
        );
        if let Some(t) = target_j["says"].as_str() {
            let _ = writeln!(text_out, "{t}");
        }
        progress(&json!({"target": target_j}));
        // The measures the round works on: those wrong on the first wrong
        // frame. A trial's curve is the first of them, frame by frame.
        let curve_of = |t: &Lockstep| -> Vec<Option<f64>> {
            let i = split_paths.first().copied().unwrap_or(0);
            t.values.iter().map(|(_, m)| curve_num(&m[i])).collect()
        };
        let frames_of = |t: &Lockstep| -> Vec<u64> { t.frames.iter().map(|f| f.1).collect() };
        let base_curve = curve_of(&base);
        // The rebuilt measure less the original's on the first wrong frame.
        let base_err = split_paths.first().and_then(|&i| {
            let k = base
                .frames
                .iter()
                .position(|f| f.1 == base_score.wrong_at())?;
            let (o, mv) = base.values.get(k)?;
            Some(curve_num(&mv[i])? - curve_num(&o[i])?)
        });
        // Each knob alone. First the impact pass: drastic values (a
        // quarter, a half, twice and four times the value, and zero),
        // ranking the numbers by how much they move the measures wrong
        // here and whether toward the original's (the user, 2026-10-29:
        // "very drastic measurements at first to identify which ones have
        // the most impact before trying to fine tune"). Then the fine pass
        // on the few that move it toward the original: their measured
        // values, a bracket between the drastic values either side of the
        // best, and steps down to the number's resolution.
        let mut tried: Vec<(Knob, Fx, Score)> = Vec::new();
        let mut impacts: Vec<Impact> = Vec::new();
        let trial_of = |k: &Knob,
                        v: Fx,
                        pass: &str,
                        say: &mut dyn FnMut(&Json)|
         -> Option<(Score, Lockstep)> {
            if stopped() {
                return None;
            }
            let mm = with(&tree, &k.path, v).ok()?;
            let t = trial(&setup, &m, &mm, a.inputs, w, true).ok()?;
            let sc = score(&t, lo, w.to, &aim.aim);
            say(
                &json!({"trying": {"round": round, "mechanic": k.mechanic, "param": k.path, "unit": k.unit,
                    "value": v.to_string(), "pass": pass, "words": param_words(&k.path), "split": sc.split,
                    "curve": {"path": split_paths.first().map(|&i| base.paths[i].path.clone()),
                              "original_frames": frames_of(&t), "values": curve_of(&t)}}}),
            );
            Some((sc, t))
        };
        // The passes' sizes, for the live card's bars: the impact pass's
        // trials, and the window's splits from the original's state.
        progress(
            &json!({"pass": {"name": "impact", "trials": ks.iter().map(|k| drastic(k).len()).sum::<usize>(),
            "base": base_score.split}}),
        );
        for k in &ks {
            let mut imp = Impact::new(k);
            for (label, v) in drastic(k) {
                let Some((sc, t)) = trial_of(k, v, "impact", progress) else {
                    continue;
                };
                imp.add(
                    label,
                    v,
                    sc,
                    &base_curve,
                    &curve_of(&t),
                    &frames_of(&t),
                    base_score,
                    &t,
                    &split_paths,
                    base_err,
                );
            }
            impacts.push(imp);
        }
        impacts.sort_by_key(|x| std::cmp::Reverse(x.rank()));
        let top_move = impacts.first().map_or(0.0, |x| x.moved);
        for x in &mut impacts {
            x.level = if x.moved == 0.0 {
                "none"
            } else if x.moved * 2.0 >= top_move {
                "big"
            } else {
                "some"
            };
        }
        // A missing mechanic, read from the impact pass before the fine
        // pass spends its trials ([`missing_mechanic`]).
        let missing = if early_stop() {
            missing_mechanic(&model, &ks, &impacts, base_score)
        } else {
            None
        };
        let impact_j = impact_json(
            &impacts,
            split_paths.first().map(|&i| base.paths[i].path.as_str()),
        );
        if let Some(t) = impact_j["says"].as_str() {
            let _ = writeln!(text_out, "{t}");
        }
        progress(&json!({"impact": impact_j}));
        // The whole-run check a value passes before it is written: no
        // measure's first split earlier, every frame before the window still
        // together, the window's first wrong frame later, and `accept`
        // (no other clip worse).
        let check = |mm: &Model,
                     after: Score,
                     strict_cur: &mut Option<Lockstep>|
         -> Result<Option<Lockstep>> {
            let full = setup.full(mm, a.inputs)?;
            let f = firsts(&full, cut, &aim.guard);
            let no_earlier = f.iter().zip(&cur_first).all(|(n, c)| match (n, c) {
                (_, None) => n.is_none(),
                (None, Some(_)) => true,
                (Some(n), Some(c)) => n >= c,
            });
            let held = first_on(&full, cut, &aim.guard).is_none_or(|(_, of, _)| of >= w.from);
            // And the whole run: no fewer frames together, and nothing
            // worse before `from`, so a window after it doesn't buy its
            // fit with frames before it.
            let whole = together_on(&full, &aim.guard) >= cur_together
                && (cut == 0
                    || score(&full, 0, cut - 1, &aim.guard) <= score(&ls, 0, cut - 1, &aim.guard));
            if !(no_earlier
                && held
                && whole
                && after.wrong_at() > before.wrong_at()
                && scope.accept.map_or(Ok(None), |f| f(mm))?.is_none())
            {
                return Ok(None);
            }
            if let Some(cur) = strict_cur.as_ref() {
                let st = strict_run(&setup, mm)?;
                let earlier =
                    firsts(&st, cut, &[])
                        .iter()
                        .zip(firsts(cur, cut, &[]))
                        .any(|(n, c)| match (n, c) {
                            (Some(n), Some(c)) => *n < c,
                            (Some(_), None) => true,
                            _ => false,
                        });
                if earlier || together(&st) < together(cur) {
                    return Ok(None);
                }
                *strict_cur = Some(st);
            }
            Ok(Some(full))
        };
        // The solvers: exact values read off the clip's diffs (an
        // arithmetic shape, a timer's change points), each tried on the
        // window from the original's state; the best that makes the first
        // wrong frame later is checked once, as a fit is. A number the
        // game's code gives is never written: its proposal goes into the
        // question with its evidence.
        let mut solver_notes: Vec<Json> = Vec::new();
        let mut solved: Option<(crate::solve::Proposal, Model, Lockstep, Score)> = None;
        if let Some(f) = scope.solve.filter(|_| crate::solve::on()) {
            let mut cands: Vec<(usize, crate::solve::Proposal, Model, Score)> = Vec::new();
            for p in f(&model)? {
                if stopped() {
                    break;
                }
                let Ok(mm) = crate::solve::apply(&model, &p.edits) else {
                    continue;
                };
                let Ok(t) = trial(&setup, &m, &mm, a.inputs, w, false) else {
                    continue;
                };
                let s = score(&t, lo, w.to, &aim.aim);
                let later = s.wrong_at() > base_score.wrong_at();
                let wrong = |x: Score| (x.wrong_at() != u64::MAX).then_some(x.wrong_at());
                solver_notes.push(json!({"by": p.by, "class": p.class, "measure": p.measure,
                    "edits": p.edits, "evidence": p.evidence, "code": p.code, "later": later,
                    "first_wrong": {"before": wrong(base_score), "after": wrong(s)}}));
                if later {
                    cands.push((solver_notes.len() - 1, p, mm, s));
                }
            }
            cands.sort_by_key(|x| x.3);
            if let Some((i, p, mm, s)) = cands.into_iter().find(|c| c.1.code.is_none()) {
                let full = check(&mm, s, &mut strict_cur)?;
                solver_notes[i]["checked"] = Json::from(full.is_some());
                if let Some(full) = full {
                    solved = Some((p, mm, full, s));
                }
            }
        }
        if let Some((p, mm, full, after)) = solved {
            let facts: Vec<Fact> = p
                .edits
                .iter()
                .map(|e| Fact {
                    path: e.path.clone(),
                    value: e.new.clone(),
                    source: "observed",
                    confidence: 90,
                    note: format!(
                        "Solved by gp model refine's {} ({}) on {}, in the window of the original's frames {} to {}: {}",
                        p.by, p.class, a.source, w.from, w.to, p.evidence
                    ),
                    refs: vec![
                        at(a.source.to_owned()),
                        at(format!("frames {}-{}", w.from, w.to)),
                    ],
                })
                .collect();
            let old = fs::read_to_string(&model_file)?;
            let new = apply(&old, &facts).map_err(anyhow::Error::msg)?;
            fs::write(&model_file, &new)?;
            model = mm;
            let next = lockstep::first_wrong(&full, cut).map(|s| s.frame);
            let e0 = &p.edits[0];
            let r = json!({
                "target": target_j,
                "impact": impact_j,
                "tried": [],
                "fine": [],
                "ties": [],
                "round": round,
                "window": window_json(&setup, w),
                "mechanic": p.mechanic,
                "by": p.by,
                "solver": {"class": p.class, "measure": p.measure, "evidence": p.evidence, "edits": p.edits, "tried": solver_notes},
                "param": e0.path,
                "unit": "",
                "old": e0.old.clone().unwrap_or_default(),
                "new": e0.new,
                "window_splits": {"before": before.split, "after": after.split},
                "drift": drift_j,
                "start": start_j,
                "exact": after.exact(),
                "first_split": {"before": earliest(&ls, cut).map(|s| s.frame), "after": next,
                    "seconds_before": earliest(&ls, cut).map(|s| setup.secs(s.frame)),
                    "seconds_after": next.map(|f| setup.secs(f))},
                "next_window": if after.exact() { next } else { None },
                "together": {"before": cur_together, "after": together_on(&full, &aim.guard)},
            });
            let _ = writeln!(
                text_out,
                "round {round}: frames {} to {} ({:.1} to {:.1} s): {} {} -> {}, solved by {}: {}; {} splits in the window -> {}",
                w.model_from,
                w.model_to,
                setup.secs(w.model_from),
                setup.secs(w.model_to),
                e0.path,
                e0.old.as_deref().unwrap_or("(unset)"),
                e0.new,
                p.by,
                p.evidence,
                before.split,
                after.split
            );
            progress(&json!({"round": r}));
            rounds.push(r);
            ls = full;
            continue;
        }
        let fine: Vec<usize> = (0..impacts.len())
            .filter(|&i| missing.is_none() && impacts[i].toward)
            .take(FINE)
            .collect();
        progress(&json!({"pass": {"name": "fine", "knobs": fine.len(), "base": base_score.split}}));
        for &fi in &fine {
            let k = impacts[fi].knob.clone();
            let mut eval =
                |v: Fx| -> Option<Score> { trial_of(&k, v, "fine", progress).map(|(s, _)| s) };
            let mut best = (k.value, base_score);
            let consider = |best: &mut (Fx, Score), v: Fx, s: Score| {
                // On a tie the value that moves the number least wins.
                if s < best.1
                    || (s == best.1
                        && best.0 != k.value
                        && less_change(v, k.value, best.0, k.value))
                {
                    *best = (v, s);
                }
            };
            for (_, v, s) in &impacts[fi].trials {
                consider(&mut best, *v, *s);
            }
            for v in measured(&k, &base, &split_paths) {
                if v == k.value || impacts[fi].trials.iter().any(|t| t.1 == v) {
                    continue;
                }
                if let Some(s) = eval(v) {
                    consider(&mut best, v, s);
                }
            }
            // The bracket: halve the gap to the drastic values either side
            // of the best, down to the number's resolution.
            let mut points: Vec<Fx> = impacts[fi].trials.iter().map(|t| t.1).collect();
            points.push(k.value);
            points.push(best.0);
            points.sort();
            points.dedup();
            let res = if k.whole { 1i64 << 16 } else { 1 };
            let at = points.iter().position(|p| *p == best.0).unwrap_or(0);
            let mut lo_b = points.get(at.wrapping_sub(1)).copied().unwrap_or(best.0);
            let mut hi_b = points.get(at + 1).copied().unwrap_or(best.0);
            for _ in 0..24 {
                let mut moved = false;
                for low in [true, false] {
                    let side = if low { lo_b } else { hi_b };
                    let mid = Fx::from_raw((best.0.raw() + side.raw()) / 2 / res * res);
                    if mid == best.0 || mid == side || !in_range(&k.range, mid) {
                        continue;
                    }
                    let Some(s) = eval(mid) else { continue };
                    let was = best.0;
                    consider(&mut best, mid, s);
                    if best.0 == mid {
                        // The old best bounds the new one on its far side.
                        if mid < was {
                            hi_b = was;
                        } else {
                            lo_b = was;
                        }
                        moved = true;
                        break;
                    } else if low {
                        lo_b = mid;
                    } else {
                        hi_b = mid;
                    }
                }
                if best.1.exact() || (!moved && hi_b.raw() - lo_b.raw() <= 2 * res) {
                    break;
                }
            }
            let (v, s) = climb(&k, best, &mut eval);
            // Only a later first wrong frame counts: fewer splits further
            // on, or a distance shaved off the first wrong frame, is no step
            // toward exact there.
            if s.wrong_at() > base_score.wrong_at() && v != k.value {
                tried.push((k.clone(), v, s));
            }
        }
        // The best first; between two equally good, the one that changes
        // its number least relative to its old value (the user's rule,
        // docs/open-questions.md, "Which fit wins when two numbers each
        // make a window exact?").
        tried.sort_by(|a, b| {
            a.2.cmp(&b.2).then_with(|| {
                if less_change(a.1, a.0.value, b.1, b.0.value) {
                    std::cmp::Ordering::Less
                } else if less_change(b.1, b.0.value, a.1, a.0.value) {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
        });
        // The best trials, each checked with a whole run.
        let mut kept = None;
        for (k, v, s) in tried.iter().take(CHECKED) {
            let mm = with(&tree, &k.path, *v)?;
            if let Some(full) = check(&mm, *s, &mut strict_cur)? {
                kept = Some((k.clone(), *v, *s, mm, full, *s));
                break;
            }
        }
        // A number the game's code gives is never changed by a fit: the
        // fit is asked about first, as a conflict, ahead of any other.
        if let Some((k, v, ..)) = &kept
            && let Some(cites) = code_cited(&model, &k.path)
        {
            stop = format!(
                "the code says {} is {}, the fit wants {}: check the mapping first",
                k.path, k.value, v
            );
            let mut q = conflict_question(&setup, w, k, *v, &cites, a.source, &stop);
            q["target"] = target_j.clone();
            q["impact"] = impact_j.clone();
            q["start"] = start_j.clone();
            questions.insert(0, q);
            break;
        }
        let Some((k, v, _, mm, full, after)) = kept else {
            if stopped() {
                stop = "stopped".into();
                break;
            }
            let at = target_j["frame"]
                .as_u64()
                .map_or("none".into(), |f| format!("frame {f}"));
            // No number the model has writes the measure: a missing
            // mechanic too, found without a trial.
            let missing = missing.or_else(|| {
                (early_stop() && ks.is_empty()).then(|| Missing {
                    detector: "nothing writes it",
                    says: "no mechanic in the model writes it".into(),
                })
            });
            stop = match &missing {
                Some(x) => format!(
                    "a missing mechanic at the first wrong frame ({at}): {}; a question for you",
                    x.says
                ),
                None => format!("no numbers fix the first wrong frame ({at}): a question for you"),
            };
            questions.push(shape_question(
                &setup,
                &ls,
                w,
                &split_paths,
                Some(&base),
                &catalog,
                &tree,
                &owners,
                &ks,
                &stop,
            ));
            if let Some(q) = questions.last_mut() {
                q["start"] = start_j.clone();
                q["target"] = target_j.clone();
                q["impact"] = impact_j.clone();
                let mut closest = closest_says(&impacts, before.wrong_at())
                    .map_or(String::new(), |c| format!(" No number fits: {c}."));
                // Every number here is the game's code: the mismatch is
                // in the reading or the mechanic, not in a number.
                let coded =
                    !ks.is_empty() && ks.iter().all(|k| code_cited(&model, &k.path).is_some());
                q["code"] = Json::from(coded);
                // The detector that stopped it, and what the original does
                // there, in words: "Mario's speed up and down: the
                // original's vy changes by -5, 0, 6 from frame to frame;
                // every number platformer.jump has for it is the game's
                // code, and none from a quarter to four times makes the
                // first wrong frame later".
                if let Some(x) = &missing {
                    let does = q["measures"][0]["why"].as_str().unwrap_or("").to_owned();
                    let what = split_paths
                        .first()
                        .map_or(String::new(), |&i| lockstep::words(&base.paths[i].path));
                    let says = format!("Missing mechanic: {what}: {does}; {}.", x.says);
                    q["missing"] = json!({"detector": x.detector, "says": says});
                    closest = format!(" {says}");
                } else if coded {
                    closest.push_str(" Its numbers match the game's code; the mismatch is elsewhere (the measurement, or a mechanic shape): check the mapping first.");
                }
                if let Some(t) = target_j["says"].as_str() {
                    let text = format!("{t}{closest} {}", q["text"].as_str().unwrap_or(""));
                    q["question"]["text"] = Json::from(text.clone());
                    q["text"] = Json::from(text);
                }
            }
            break;
        };
        // Write it, with what it was measured on.
        let orig_vals: Vec<String> = split_paths
            .iter()
            .map(|&i| {
                format!(
                    "{} {}",
                    base.paths[i].path,
                    base.values
                        .iter()
                        .skip(1)
                        .map(|(o, _)| show(&o[i]))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect();
        let fact = Fact {
            path: k.path.clone(),
            value: v.to_string(),
            source: "observed",
            confidence: 90,
            note: format!(
                "Fitted by gp model refine --lockstep on {}: in the window of the original's frames {} to {}, {} splits before and {} after (a measure on a frame); the original's values there: {}",
                a.source,
                w.from,
                w.to,
                before.split,
                after.split,
                orig_vals.join("; ")
            ),
            refs: vec![
                at(a.source.to_owned()),
                at(format!("frames {}-{}", w.from, w.to)),
            ],
        };
        let old = fs::read_to_string(&model_file)?;
        let new = apply(&old, &[fact]).map_err(anyhow::Error::msg)?;
        fs::write(&model_file, &new)?;
        model = mm;
        let next = lockstep::first_wrong(&full, cut).map(|s| s.frame);
        let ties: Vec<Json> = tried
            .iter()
            .filter(|(o, ov, os)| *os == after && (o.path != k.path || *ov != v))
            .map(|(o, ov, _)| json!({"param": o.path, "old": o.value.to_string(), "new": ov.to_string()}))
            .collect();
        let r = json!({
            "target": target_j,
            "impact": impact_j,
            "tried": ks.iter().map(|k| k.path.clone()).collect::<Vec<_>>(),
            "fine": fine.iter().map(|&i| impacts[i].knob.path.clone()).collect::<Vec<_>>(),
            "ties": ties,
            "round": round,
            "window": window_json(&setup, w),
            "mechanic": k.mechanic,
            "param": k.path,
            "unit": k.unit,
            "old": k.value.to_string(),
            "new": v.to_string(),
            "window_splits": {"before": before.split, "after": after.split},
            "drift": drift_j,
            "start": start_j,
            "exact": after.exact(),
            "first_split": {"before": earliest(&ls, cut).map(|s| s.frame), "after": next,
                "seconds_before": earliest(&ls, cut).map(|s| setup.secs(s.frame)),
                "seconds_after": next.map(|f| setup.secs(f))},
            "next_window": if after.exact() { next } else { None },
            "together": {"before": cur_together, "after": together_on(&full, &aim.guard)},
        });
        if let Some(d) = drift_j.as_ref().and_then(|d| d["says"].as_str()) {
            let _ = writeln!(text_out, "{d}");
        }
        let _ = writeln!(text_out, "{}", r["start"]["says"].as_str().unwrap_or(""));
        let _ = writeln!(
            text_out,
            "round {round}: frames {} to {} ({:.1} to {:.1} s): {} {} -> {}; {} splits in the window -> {}{}",
            w.model_from,
            w.model_to,
            setup.secs(w.model_from),
            setup.secs(w.model_to),
            k.path,
            k.value,
            v,
            before.split,
            after.split,
            if after.exact() {
                match next {
                    Some(n) => format!("; now exact, next window at frame {n}"),
                    None => "; now exact, and no split remains".into(),
                }
            } else {
                String::new()
            }
        );
        progress(&json!({"round": r}));
        rounds.push(r);
        ls = full;
    }
    // A kept value, measured on every other moment of the kind.
    let also = match &target {
        Some((m, same, first)) if !rounds.is_empty() => Some(others_check(
            &setup,
            a,
            (&start_model, &model),
            m,
            same,
            gp_interp::moments::Checks {
                lead: gp_interp::moments::LEAD,
                first: *first,
                clocks: clocks.clone(),
                tolerance: setup.tolerance.clone(),
            },
            &mut questions,
        )?),
        _ => None,
    };
    if let Some(x) = &also {
        let _ = writeln!(text_out, "{}", x["line"].as_str().unwrap_or(""));
    }
    let first_after = earliest(&ls, cut).map(|s| s.frame);
    let _ = writeln!(text_out, "{round} rounds: {stop}");
    for q in &questions {
        let _ = writeln!(
            text_out,
            "question {}: {}",
            q["id"].as_str().unwrap_or(""),
            q["text"].as_str().unwrap_or("")
        );
    }
    // The questions go to the working copy's questions.toml.
    let mut qs = super::read_questions(a.dir)?;
    for q in &questions {
        if let Ok(question) = serde_json::from_value::<Question>(q["question"].clone()) {
            qs.ask(question);
        }
    }
    fs::write(a.dir.join(QUESTIONS_FILE), qs.to_toml())?;
    let j = json!({
        "source": a.source,
        "window": a.window,
        "from": a.from,
        "rounds": rounds,
        "stop": stop,
        "stuck": !questions.is_empty(),
        "first_split": {"before": first_before, "after": first_after,
            "seconds_before": first_before.map(|f| setup.secs(f)),
            "seconds_after": first_after.map(|f| setup.secs(f))},
        "together": {"before": together_before, "after": together(&ls), "measured": ls.frames.len()},
        "windows": windows,
        "exact_from_start": exact_windows,
        "unheld": unheld,
        "questions": questions,
        "scope": {
            "moment": target.as_ref().map(|(m, _, first)| moment_json(m, None, mechanic.as_deref(), &|f| setup.secs(f), *first)),
            "mechanic": mechanic,
            "until": until,
        },
        "also": also,
    });
    progress(&json!({"done": j}));
    Ok(Outcome::ok(j, text_out.trim_end().to_owned()))
}

/// Stops holding `paths`, measures no mechanic's numbers write (none owns
/// them, or those that do have no numbers, as a counter the model only
/// counts) that are wrong first
/// at the original's frame `frame`: their tolerance and drift become
/// unlimited from here on, and a record of it goes into `unheld`. Returns
/// the record in words.
fn unhold(
    setup: &mut Setup,
    ls: &Lockstep,
    paths: &[usize],
    frame: u64,
    unheld: &mut Vec<Json>,
) -> String {
    let names: Vec<String> = paths.iter().map(|&i| ls.paths[i].path.clone()).collect();
    for &i in paths {
        if let Some(t) = setup.tolerance.get_mut(i) {
            *t = Fx::from_raw(i64::MAX);
        }
        if let Some(d) = setup.drift.get_mut(i) {
            *d = Fx::from_raw(i64::MAX);
        }
    }
    let says = format!(
        "{} wrong from the original's frame {frame}, and no mechanic owns it: not held from here, on to the next window a mechanic owns",
        names.join(", ")
    );
    unheld.push(json!({"original_frame": frame, "paths": names, "says": says}));
    says
}

/// Where the model, run free from the window's start, drifts past a
/// threshold in the window.
fn drift_in(
    setup: &Setup,
    drift: &[Fx],
    m: &Mid,
    model: &Model,
    inputs: &[InputFrame],
    w: Window,
) -> Result<Option<lockstep::Drift>> {
    let mut original = m.original.fork();
    let mut interp = setup.interp_at(model, Some(m.frame))?;
    let spec = interp.spec().clone();
    let end = usize::try_from(w.to + 1)
        .unwrap_or(usize::MAX)
        .min(inputs.len());
    let start = usize::try_from(m.frame).unwrap_or(usize::MAX).min(end);
    let free = lockstep::lockstep(
        &mut original,
        &mut interp,
        &spec,
        &setup.from_mid,
        &inputs[start..end],
        &Options {
            mode: Mode::Free,
            ..setup.opts(Some(m.frame), Some(m.state.clone()), false)
        },
    )
    .map_err(anyhow::Error::msg)?;
    Ok(lockstep::drift_point(
        &free,
        drift,
        (m.frame + 1).min(w.from),
    ))
}

/// The question a fit on a code-cited number asks instead of writing it:
/// "the code says 1, the fit wants 1.5: check the mapping first". Its
/// first answer keeps the code's value, its second writes the fit.
fn conflict_question(
    setup: &Setup,
    w: Window,
    k: &Knob,
    v: Fx,
    cites: &[String],
    source: &str,
    stop: &str,
) -> Json {
    let text = format!(
        "The game's code says {} is {} ({}), and the fit on {} in the original's frames {} to {} wants {}: check the mapping first. A measure read wrongly (a fraction, a scale) makes a fit move a number the code already gives.",
        k.path,
        k.value,
        cites.join(", "),
        source,
        w.from,
        w.to,
        v
    );
    let id = format!("conflict:{}@{}", k.path, w.from);
    let question = Question {
        id: id.clone(),
        kind: "conflict".into(),
        path: k.path.clone(),
        text: text.clone(),
        settle: "a person or an agent checks the mapping's reading of the measures against the game's code, and fixes it if it is wrong".into(),
        candidates: vec![
            Candidate {
                value: k.value.to_string(),
                from: format!("the game's code ({}); the mapping is checked", cites.join(", ")),
                refs: Vec::new(),
            },
            Candidate {
                value: v.to_string(),
                from: format!("the fit on {source}: the code's value is wrong here"),
                refs: Vec::new(),
            },
        ],
        answer: None,
    };
    json!({
        "id": id,
        "kind": "conflict",
        "text": text,
        "says": stop,
        "window": window_json(setup, w),
        "mechanic": k.mechanic,
        "param": k.path,
        "code": k.value.to_string(),
        "fit": v.to_string(),
        "cites": cites,
        "measures": [],
        "tried": [k.path],
        "proposal": "",
        "question": serde_json::to_value(&question).unwrap_or(Json::Null),
    })
}

/// How many numbers the fine pass works on, the most effect first.
const FINE: usize = 3;

/// Whether the loop stops at a missing mechanic before the fine pass
/// (docs/interpreter.md, "Refining from the lockstep"). On unless
/// `GP_REFINE_EARLY_STOP` is `off`.
fn early_stop() -> bool {
    std::env::var("GP_REFINE_EARLY_STOP").map_or(true, |v| v != "off")
}

/// A round stopped early: the mismatch is a mechanic's shape, not a
/// number.
struct Missing {
    /// Which detector fired: `nothing moves it`, `the code's numbers`
    /// or `nothing writes it`.
    detector: &'static str,
    /// What it found, in words.
    says: String,
}

/// The early stop, read from the impact pass alone (docs/research/
/// refine-loop-study.md): none of the numbers moves the measure at all,
/// at any drastic value, so no value between them can either; or every
/// number is the game's code, which a fit never changes, and no drastic
/// value makes the first wrong frame any later.
fn missing_mechanic(
    model: &Model,
    ks: &[Knob],
    impacts: &[Impact],
    base: Score,
) -> Option<Missing> {
    if ks.is_empty() || impacts.is_empty() {
        return None;
    }
    let mechanics = || {
        let mut names: Vec<&str> = ks.iter().map(|k| k.mechanic.as_str()).collect();
        names.dedup();
        names.join(", ")
    };
    let later = impacts
        .iter()
        .flat_map(|x| &x.trials)
        .any(|t| t.2.wrong_at() > base.wrong_at());
    if !later
        && impacts
            .iter()
            .all(|x| !x.trials.is_empty() && x.moved == 0.0)
    {
        return Some(Missing {
            detector: "nothing moves it",
            says: format!(
                "none of {}'s numbers changes it at all, from zero to four times their value",
                mechanics()
            ),
        });
    }
    if !later && ks.iter().all(|k| code_cited(model, &k.path).is_some()) {
        return Some(Missing {
            detector: "the code's numbers",
            says: format!(
                "every number {} has for it is the game's code, and none from a quarter to four times makes the first wrong frame later",
                mechanics()
            ),
        });
    }
    None
}

use gp_interp::scenario::curve_value as curve_num;

/// A number's name in words: `entities.player.run.walk_speed` is "walk
/// speed", `entities.mario.jump.bands[1].gravity` is "bands 2 gravity".
fn param_words(path: &str) -> String {
    let segs: Vec<&str> = path.split('.').collect();
    let last = segs.last().copied().unwrap_or(path).replace('_', " ");
    match segs.len().checked_sub(2).map(|i| segs[i]) {
        Some(item) if item.contains('[') => {
            let (list, n) = item.split_once('[').unwrap_or((item, "0]"));
            let n: usize = n.trim_end_matches(']').parse().unwrap_or(0);
            format!("{} {} {last}", list.replace('_', " "), n + 1)
        }
        _ => last,
    }
}

/// The drastic values the impact pass tries a number at: a quarter, a
/// half, twice and four times it, and zero, in its range and step (a
/// whole number moves by one at least). A number at zero is tried at one
/// and four, either way.
fn drastic(k: &Knob) -> Vec<(&'static str, Fx)> {
    let v = k.value.raw();
    let step = if k.whole { 1i64 << 16 } else { 1 };
    let snap = |raw: i64| raw / step * step;
    let mut out: Vec<(&'static str, Fx)> = Vec::new();
    let cands: Vec<(&'static str, i64)> = if v == 0 {
        vec![
            ("1", 1 << 16),
            ("-1", -(1 << 16)),
            ("4", 4 << 16),
            ("-4", -(4 << 16)),
        ]
    } else {
        vec![
            ("x0.25", snap(v / 4)),
            ("x0.5", snap(v / 2)),
            ("x2", v.saturating_mul(2)),
            ("x4", v.saturating_mul(4)),
            ("0", 0),
        ]
    };
    for (label, raw) in cands {
        let x = Fx::from_raw(raw);
        if raw != v && in_range(&k.range, x) && !out.iter().any(|(_, y)| *y == x) {
            out.push((label, x));
        }
    }
    out
}

/// What the impact pass found for one number.
struct Impact {
    knob: Knob,
    /// Each drastic value tried: its label, value and score.
    trials: Vec<(&'static str, Fx, Score)>,
    /// The most the measure's curve moved, added up over the frames.
    moved: f64,
    /// The original's first frame it moved on.
    from_frame: Option<u64>,
    /// Some drastic value brings the window closer.
    toward: bool,
    /// The best drastic value: its label, value, score, and whether the
    /// rebuilt measure is past the original's on the frame it goes wrong.
    best: Option<(&'static str, Fx, Score, Option<bool>)>,
    level: &'static str,
}

impl Impact {
    fn new(k: &Knob) -> Self {
        Impact {
            knob: k.clone(),
            trials: Vec::new(),
            moved: 0.0,
            from_frame: None,
            toward: false,
            best: None,
            level: "none",
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        label: &'static str,
        v: Fx,
        sc: Score,
        base_curve: &[Option<f64>],
        curve: &[Option<f64>],
        frames: &[u64],
        base: Score,
        t: &Lockstep,
        paths: &[usize],
        base_err: Option<f64>,
    ) {
        let mut moved = 0.0;
        for (j, (b, c)) in base_curve.iter().zip(curve).enumerate() {
            if let (Some(b), Some(c)) = (b, c)
                && (b - c).abs() > 0.0
            {
                moved += (b - c).abs();
                let f = frames.get(j).copied().unwrap_or(0);
                if self.from_frame.is_none_or(|x| f < x) {
                    self.from_frame = Some(f);
                }
            }
        }
        self.moved = self.moved.max(moved);
        self.trials.push((label, v, sc));
        // Toward the original: closer to exact, or past the original's
        // value on the first wrong frame where the number now is short of
        // it, so the right value is between the two.
        let err = |t: &Lockstep| -> Option<f64> {
            let i = *paths.first()?;
            let k = t.frames.iter().position(|f| f.1 == base.wrong_at())?;
            let (o, m) = t.values.get(k)?;
            Some(curve_num(&m[i])? - curve_num(&o[i])?)
        };
        let crosses = match (err(t), base_err) {
            (Some(e), Some(e0)) => e.abs() < e0.abs() || (e != 0.0 && e.signum() != e0.signum()),
            _ => false,
        };
        if sc < base || crosses {
            self.toward = true;
        }
        if self.best.is_none_or(|b| sc < b.2) {
            // Past or short of the original where it goes wrong.
            let over = paths.first().and_then(|&i| {
                let k = t.frames.iter().position(|f| f.1 == sc.wrong_at())?;
                let (o, m) = t.values.get(k)?;
                Some(curve_num(&m[i])?.abs() > curve_num(&o[i])?.abs())
            });
            self.best = Some((label, v, sc, over));
        }
    }

    /// The order of the ranking, the most effect toward the original first.
    fn rank(&self) -> (bool, std::cmp::Reverse<Option<Score>>, i64) {
        (
            self.toward,
            std::cmp::Reverse(self.best.map(|b| b.2)),
            (self.moved * 65536.0) as i64,
        )
    }
}

/// The impact pass in words and JSON: "Most effect on Mario's speed across:
/// walk speed (big), gravity (none)", and the closest drastic value.
fn impact_json(impacts: &[Impact], path: Option<&str>) -> Json {
    let what = path.map_or("the measure".to_owned(), lockstep::words);
    let rows: Vec<Json> = impacts
        .iter()
        .map(|x| {
            json!({"param": x.knob.path, "words": param_words(&x.knob.path), "mechanic": x.knob.mechanic,
                   "level": x.level, "toward": x.toward, "moved": x.moved, "from_original_frame": x.from_frame,
                   "tried": x.trials.iter().map(|(l, v, s)| json!({"label": l, "value": v.to_string(),
                        "first_wrong": (s.wrong_at() != u64::MAX).then_some(s.wrong_at()), "splits": s.split})).collect::<Vec<_>>(),
                   "best": x.best.map(|(l, v, ..)| json!({"label": l, "value": v.to_string()}))})
        })
        .collect();
    let list = impacts
        .iter()
        .map(|x| {
            let how = if x.level != "none" && !x.toward {
                format!("{}, the wrong way", x.level)
            } else {
                x.level.to_owned()
            };
            format!("{} ({how})", param_words(&x.knob.path))
        })
        .collect::<Vec<_>>()
        .join(", ");
    json!({"path": path, "ranking": rows,
           "says": if impacts.is_empty() { String::new() } else { format!("Most effect on {what}: {list}.") }})
}

/// The closest drastic value, for a stuck window's question: "the closest
/// was walk speed x2, which got the first 3 frames right but overshot
/// after".
fn closest_says(impacts: &[Impact], first: u64) -> Option<String> {
    let x = impacts
        .iter()
        .filter(|x| x.best.is_some())
        .min_by_key(|x| x.best.map(|b| b.2))?;
    let (label, _, sc, over) = x.best?;
    let name = param_words(&x.knob.path);
    if sc.wrong_at() == u64::MAX {
        return Some(format!(
            "the closest was {name} {label}, which made the window exact but broke a check on the whole run"
        ));
    }
    let right = sc.wrong_at().saturating_sub(first);
    let after = match over {
        Some(true) => "overshot after",
        Some(false) => "fell short after",
        None => "went wrong after",
    };
    Some(if right == 0 {
        format!("the closest was {name} {label}, which still goes wrong on the same frame")
    } else {
        format!(
            "the closest was {name} {label}, which got the first {right} frame{} right but {after}",
            if right == 1 { "" } else { "s" }
        )
    })
}

/// The round's target in words and JSON: the first wrong frame from the
/// original's state, counted from `start`, each measure wrong there with
/// both games' values, and the mechanics whose numbers are tried: "Wrong
/// from frame 1: Mario's speed across (original 0.0625, rebuilt
/// 0.0371). Trying only platformer.run's numbers."
fn target_json(
    base: &Lockstep,
    lo: u64,
    aim: &[bool],
    start: u64,
    owners: &[(&'static str, String)],
    ks: &[Knob],
) -> Json {
    let Some((k, of, wrong)) = first_on(base, lo, aim) else {
        return Json::Null;
    };
    let short = |v: &Value| match v {
        Value::Fixed(r) => {
            let x = *r as f64 / 65536.0;
            let s = format!("{x:.4}");
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        }
        other => show(other),
    };
    let measures: Vec<Json> = wrong
        .iter()
        .map(|&i| {
            let (o, m) = base
                .values
                .get(k)
                .map_or((String::new(), String::new()), |(o, m)| {
                    (short(&o[i]), short(&m[i]))
                });
            json!({"path": base.paths[i].path, "words": lockstep::words(&base.paths[i].path),
                   "original": o, "model": m})
        })
        .collect();
    let what = measures
        .iter()
        .map(|m| {
            if m["original"] == "" {
                m["words"].as_str().unwrap_or("").to_owned()
            } else {
                format!(
                    "{} (original {}, rebuilt {})",
                    m["words"].as_str().unwrap_or(""),
                    m["original"].as_str().unwrap_or(""),
                    m["model"].as_str().unwrap_or("")
                )
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    // The mechanics whose numbers are tried: an owner with no numbers
    // (tile collision) has none to try.
    let mut names: Vec<&str> = owners
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| ks.iter().any(|k| k.mechanic == *n))
        .collect();
    names.dedup();
    let names: Vec<String> = names.iter().map(|n| format!("{n}'s")).collect();
    let trying = if names.is_empty() {
        "No mechanic owns it.".to_owned()
    } else {
        format!("Trying only {} numbers.", names.join(" and "))
    };
    let frame = of.saturating_sub(start);
    json!({"frame": frame, "original_frame": of, "measures": measures,
           "mechanics": owners.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
           "says": format!("Wrong from frame {frame}: {what}. {trying}")})
}

/// Where a window drifted, in words: "Stopped at 3.1 s: entities.mario[0].x
/// 2.4 px apart. Working on 2.6 to 3.1 s".
fn drift_json(setup: &Setup, d: &lockstep::Drift, w: Window) -> Json {
    let f = |x: Fx| x.raw() as f64 / 65536.0;
    let unit = match d.path.rsplit('.').next().unwrap_or("") {
        "x" | "y" | "stage_x" | "stage_y" => " px",
        "vx" | "vy" => " px a frame",
        _ => "",
    };
    let says = format!(
        "Stopped at {:.1} s: {} {}{unit} apart (past {}{unit}). Working on {:.1} to {:.1} s",
        setup.secs(d.original_frame),
        d.path,
        f(d.distance),
        f(d.threshold),
        setup.secs(w.from),
        setup.secs(w.to)
    );
    json!({"original_frame": d.original_frame, "seconds": setup.secs(d.original_frame),
           "path": d.path, "distance": f(d.distance), "threshold": f(d.threshold), "says": says})
}

/// A window ended at the original's frame `u` at most.
fn clip(ls: &Lockstep, w: Window, u: u64) -> Window {
    let to = w.to.min(u);
    let last = ls
        .frames
        .iter()
        .rev()
        .find(|(_, o)| *o <= to)
        .map_or(w.model_from, |f| f.0);
    Window {
        to,
        model_to: last.max(w.model_from),
        ..w
    }
}

/// Every other moment of `m`'s kind, measured with the model before the
/// loop and after it, each from the original's state before it: how many
/// it makes closer, how many further, and a question for each further
/// one naming its frames and what differs there.
#[allow(clippy::too_many_arguments)]
fn others_check(
    setup: &Setup,
    a: &LockstepArgs<'_>,
    (old, new): (&Model, &Model),
    m: &Moment,
    same: &[Moment],
    checks: gp_interp::moments::Checks,
    questions: &mut Vec<Json>,
) -> Result<Json> {
    let mut all: Vec<Moment> = same.to_vec();
    if !all.iter().any(|x| x.n == m.n) {
        all.push(m.clone());
    }
    let before = setup.answers(old, a.inputs, &all, &checks)?;
    let after = setup.answers(new, a.inputs, &all, &checks)?;
    let score = |x: &Answer| (x.apart, x.distance);
    let px = |f: Fx| f.raw() as f64 / 65536.0;
    let side = |x: &Answer| {
        json!({"apart": x.apart, "found": x.model.is_some(),
               "offset": x.model.as_ref().map(|y| y.from as i64 - m.from as i64),
               "height": x.model.as_ref().and_then(|y| y.height).map(px),
               "measures": x.paths.iter().map(|(p, d)| json!({"path": p, "max": px(*d)})).collect::<Vec<_>>()})
    };
    let (mut better, mut worse, mut same_n) = (0usize, Vec::new(), 0usize);
    let mut own = Json::Null;
    for ((x, b), f) in all.iter().zip(&before).zip(&after) {
        let entry = json!({"n": x.n, "from": x.from, "to": x.to, "seconds": setup.secs(x.from),
            "original_height": x.height.map(px), "before": side(b), "after": side(f)});
        if x.n == m.n {
            own = entry;
            continue;
        }
        match score(f).cmp(&score(b)) {
            std::cmp::Ordering::Less => better += 1,
            std::cmp::Ordering::Greater => worse.push((x.clone(), b.clone(), f.clone(), entry)),
            std::cmp::Ordering::Equal => same_n += 1,
        }
    }
    let others = all.len() - 1;
    let kind = m.kind;
    let plural = |n: usize| {
        if n == 1 {
            kind.to_owned()
        } else {
            format!("{kind}s")
        }
    };
    let line = format!(
        "also better on {better} of {others} other {}, worse on {}, the same on {same_n}",
        plural(others),
        worse.len()
    );
    for (x, b, f, _) in &worse {
        let differs: Vec<String> = f
            .paths
            .iter()
            .map(|(p, d)| {
                let was = b
                    .paths
                    .iter()
                    .find(|(q, _)| q == p)
                    .map_or(Fx::ZERO, |(_, d)| *d);
                format!("{p} up to {} (was {})", px(*d), px(was))
            })
            .collect();
        let (from, to) = gp_interp::moments::span(x, checks.lead, checks.first);
        let text = format!(
            "{kind} {} at frames {} to {} ({:.1} s): the kept value makes it further from the original ({} measure-frames apart, was {}): {}. Is this {kind} different, or does the mechanic need another number?",
            x.n,
            from,
            to,
            setup.secs(x.from),
            f.apart,
            b.apart,
            if differs.is_empty() {
                "nothing past its tolerance".to_owned()
            } else {
                differs.join("; ")
            }
        );
        let id = format!("moment:{kind}:{}", x.n);
        let question = Question {
            id: id.clone(),
            kind: "worse".into(),
            path: f.paths.first().map_or(String::new(), |(p, _)| p.clone()),
            text: text.clone(),
            settle: "a person looks at the two games over these frames".into(),
            candidates: vec![
                Candidate {
                    value: "keep".into(),
                    from:
                        "keep the value; this one differs for a reason the model doesn't have yet"
                            .into(),
                    refs: Vec::new(),
                },
                Candidate {
                    value: "look".into(),
                    from: "the mechanic needs another number or option here".into(),
                    refs: Vec::new(),
                },
            ],
            answer: None,
        };
        questions.push(json!({"id": id, "kind": "worse", "text": text,
            "window": {"original_from": from, "original_to": to, "from": from, "to": to,
                "seconds_from": setup.secs(from), "seconds_to": setup.secs(to)},
            "mechanic": "", "measures": [], "tried": [], "proposal": "",
            "question": serde_json::to_value(&question).unwrap_or(Json::Null)}));
    }
    Ok(json!({
        "kind": kind,
        "others": others,
        "better": better,
        "worse": worse.len(),
        "same": same_n,
        "line": line,
        "moment": own,
        "worse_moments": worse.iter().map(|(.., e)| e.clone()).collect::<Vec<_>>(),
    }))
}

/// The question a stuck window asks: the window, each measure split there
/// with both games' values, why no numbers fit, and the general mechanic
/// to add or change.
#[allow(clippy::too_many_arguments)]
fn shape_question(
    setup: &Setup,
    ls: &Lockstep,
    w: Window,
    split_paths: &[usize],
    trial: Option<&Lockstep>,
    catalog: &gp_interp::mechanics::Catalog,
    tree: &toml::Value,
    owners: &[(&'static str, String)],
    ks: &[Knob],
    stop: &str,
) -> Json {
    let first = split_paths.first().copied().unwrap_or(0);
    let path = ls.paths[first].path.clone();
    let mechanic = owners.first().map_or("(unowned)", |(n, _)| *n).to_owned();
    let mut measures = Vec::new();
    let mut whys = Vec::new();
    let mut proposal = String::new();
    for &i in split_paths {
        let (orig, mine): (Vec<String>, Vec<String>) = match trial {
            Some(t) => t
                .values
                .iter()
                .skip(1)
                .map(|(o, m)| (show(&o[i]), show(&m[i])))
                .unzip(),
            None => (Vec::new(), Vec::new()),
        };
        let owner = lockstep::owners(catalog, &ls.paths[i].path)
            .first()
            .copied()
            .unwrap_or("(unowned)");
        let (y, p) = match trial {
            Some(t) => why(catalog, tree, t, i, owner),
            None => (
                stop.to_owned(),
                "the model's starting state, set from the original's".to_owned(),
            ),
        };
        if proposal.is_empty() {
            proposal = p.clone();
        }
        whys.push(y.clone());
        measures.push(json!({"path": ls.paths[i].path, "mechanic": owner, "original": orig, "model": mine, "why": y, "proposal": p}));
    }
    let tried: Vec<String> = ks.iter().map(|k| k.path.clone()).collect();
    let text = format!(
        "In frames {} to {} ({:.1} to {:.1} s, the original's {} to {}), no numbers of {} make {} exact: {}. Approve a general change: {}?",
        w.model_from,
        w.model_to,
        setup.secs(w.model_from),
        setup.secs(w.model_to),
        w.from,
        w.to,
        {
            let mut names: Vec<&str> = owners.iter().map(|(n, _)| *n).collect();
            names.dedup();
            names.join(", ")
        },
        split_paths
            .iter()
            .map(|&i| ls.paths[i].path.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        whys.join("; "),
        proposal
    );
    let id = format!("lockstep:{path}@{}", w.from);
    let question = Question {
        id: id.clone(),
        kind: "shape".into(),
        path: path.clone(),
        text: text.clone(),
        settle: "a person approves the general mechanic; an agent then builds it as an option other games can use, and the loop runs again".into(),
        candidates: vec![
            Candidate {
                value: "approve".into(),
                from: proposal.clone(),
                refs: Vec::new(),
            },
            Candidate {
                value: "keep".into(),
                from: "keep the mechanic as it is and accept the split".into(),
                refs: Vec::new(),
            },
        ],
        answer: None,
    };
    json!({
        "id": id,
        "kind": "shape",
        "text": text,
        "window": window_json(setup, w),
        "mechanic": mechanic,
        "measures": measures,
        "tried": tried,
        "proposal": proposal,
        "question": serde_json::to_value(&question).unwrap_or(Json::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tie_keeps_the_value_that_changes_its_number_least() {
        let f = |x: f64| Fx::from_raw((x * 65536.0) as i64);
        // The trial-start lane's tie: `gravity_held` 0.1875 -> 0.125 (33%)
        // against `below` 0.5625 -> -0.4375 (178%).
        assert!(less_change(f(0.125), f(0.1875), f(-0.4375), f(0.5625)));
        assert!(!less_change(f(-0.4375), f(0.5625), f(0.125), f(0.1875)));
        // The same step on a larger number is the smaller change.
        assert!(less_change(f(4.25), f(4.0), f(1.25), f(1.0)));
        // Equal changes are no less than each other.
        assert!(!less_change(f(2.0), f(1.0), f(4.0), f(2.0)));
        // From zero, a raw step stands for the old value.
        assert!(less_change(f(0.5), f(1.0), Fx::from_raw(1), f(0.0)));
    }

    const CODED: &str = r#"
[model]
schema = "0.3.0"
name = "madeup"
title = "Made Up"
source = "madeup"
view = "side"
frame_rate = "60"
systems = ["platformer.player"]

[model.extensions]
platformer = "0.3.0"

[input]
device = "nes-pad"

[input.actions]
jump = "a"

[entities.hero]
role = "player"
size = [16, 16]

[entities.hero.hitbox]
x = 2
y = 4
w = 12
h = 10

[entities.hero.run]
air_control = true
walk_speed = "1.5"

[entities.hero.jump]
action = "jump"
speed = -4
gravity = "0.4375"
max_fall = 5

[evidence."entities.hero.jump"]
source = "stated"
confidence = 100

[[evidence."entities.hero.jump".refs]]
at = "game.s:11"
"#;

    fn knob(path: &str, v: f64) -> Knob {
        Knob {
            mechanic: "platformer.jump".into(),
            path: path.into(),
            value: Fx::from_raw((v * 65536.0) as i64),
            unit: String::new(),
            whole: false,
            range: String::new(),
        }
    }

    fn at(frame: u64) -> Score {
        Score {
            wrong_at: std::cmp::Reverse(frame),
            split: 1,
            distance: 1,
        }
    }

    /// An impact pass on `k`: each drastic value's first wrong frame, and
    /// how far the measure's curve moved.
    fn impact(k: &Knob, firsts: &[u64], moved: f64) -> Impact {
        let mut x = Impact::new(k);
        for f in firsts {
            x.trials.push(("x2", k.value, at(*f)));
        }
        x.moved = moved;
        x
    }

    #[test]
    fn a_missing_mechanic_stops_the_round_before_the_fine_pass() {
        let model = Model::parse(CODED).unwrap();
        let gravity = knob("entities.hero.jump.gravity", 0.4375);
        let walk = knob("entities.hero.run.walk_speed", 1.5);
        let base = at(40);
        // No drastic value moves the measure: none between them can.
        let ks = vec![walk.clone()];
        let m = missing_mechanic(&model, &ks, &[impact(&walk, &[40, 40], 0.0)], base);
        assert_eq!(m.map(|x| x.detector), Some("nothing moves it"));
        // Every number is the game's code and none makes the first wrong
        // frame later: a fit could never be kept.
        let ks = vec![gravity.clone()];
        let m = missing_mechanic(&model, &ks, &[impact(&gravity, &[40, 38], 3.0)], base);
        assert_eq!(m.map(|x| x.detector), Some("the code's numbers"));
        // One drastic value gets further: the fine pass runs, so a fit the
        // code disagrees with becomes a conflict question.
        let m = missing_mechanic(&model, &ks, &[impact(&gravity, &[40, 44], 3.0)], base);
        assert!(m.is_none());
        // A measured number that moves the measure gets its fine pass.
        let ks = vec![walk.clone()];
        let m = missing_mechanic(&model, &ks, &[impact(&walk, &[40, 38], 3.0)], base);
        assert!(m.is_none());
        // With no numbers there is no impact pass to read.
        assert!(missing_mechanic(&model, &[], &[], base).is_none());
    }

    #[test]
    fn a_window_is_given_in_frames_or_seconds() {
        let (nes, thirty) = (60_099, 30_000);
        let at = |s: &str, fps: u64| parse_window(s).map(|w| w.frames(fps));
        assert_eq!(at("10", nes), Ok(10));
        assert_eq!(at("10", thirty), Ok(10));
        assert_eq!(at("5s", nes), Ok(DEFAULT_WINDOW));
        assert_eq!(at("2.5s", nes), Ok(150));
        // Seconds are the game's own: 5 s of a 30 Hz board is 150 frames.
        assert_eq!(at("5s", thirty), Ok(150));
        assert_eq!(at("2.5s", thirty), Ok(75));
        assert!(parse_window("0").is_err());
        assert!(parse_window("0s").is_err());
        assert!(parse_window("five").is_err());
    }

    /// A hook's numbers are tried like a mechanic's: each number in its
    /// entity's ext_params, whole when the model writes it whole, under
    /// `ext:HOOK` for a hook that describes nothing; and a counter's hooks
    /// are those that may write it. Donkey Kong's model from the games
    /// checkout beside this one; skipped without it.
    #[test]
    fn a_hooks_numbers_are_knobs() {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../games/donkey-kong/model.toml");
        let Ok(text) = std::fs::read_to_string(&file) else {
            return;
        };
        let model = Model::parse(&text).unwrap();
        let catalog = gp_interp::mechanics::catalog(&model, &gp_sim::ext::Exts::new());
        let tree = model.to_tree();
        let ks = hook_knobs(&model, &catalog, &tree, "entities.kong");
        let hold = ks
            .iter()
            .find(|k| k.path == "entities.kong.ext_params.hold")
            .unwrap();
        assert_eq!(hold.mechanic, "ext:dk_throw");
        assert!(hold.whole);
        assert_eq!(hold.value, Fx::from_int(16));
        assert!(
            counter_hooks(&model, &catalog, "drop_timer").contains(&"entities.kong".to_owned())
        );
        assert!(hook_named(&model, "ext:dk_throw") && !hook_named(&model, "ext:nope"));
        assert!(hook_owns(
            &model,
            &catalog,
            "ext:dk_throw",
            "entities.kong[0].x"
        ));
        assert!(!hook_owns(
            &model,
            &catalog,
            "ext:dk_throw",
            "entities.player[0].x"
        ));
    }
}
