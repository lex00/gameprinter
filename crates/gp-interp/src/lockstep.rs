//! Both games in step on the same input, measured apart on every mapped
//! path every frame (docs/interpreter.md, "Lockstep: distance and resync").
//!
//! The frames line up as [`crate::compare::compare`]'s do (the mapping's
//! `polled` alignment). Each compared frame gives each mapped path a
//! distance: for a number the absolute difference in the path's units
//! (whole parts only for a `whole` field), for a flag or a name 0 or 1. A
//! path splits when its distance is past its tolerance.
//!
//! In [`Mode::Resync`] a split ends with the original's state translated
//! into the model's ([`crate::translate`]) and loaded with the runtime
//! protocol's `load-state`, so every split is a place the model parts from
//! the original on its own, not a consequence of an earlier one. In
//! [`Mode::Free`] nothing is put back, and the distances show the drift.
//!
//! The bodies other than the player are measured too ([`crate::bodies`]):
//! each of the original's is paired with a model body of its kind for the
//! run, and a slot body's measures follow the mapped paths in
//! [`Lockstep::paths`]. A resync keeps the model's own bodies, so a body's
//! split is noted but puts nothing back.

use gp_runtime::{Entry, InputFrame, LoadNote, Runtime, Value};
use gp_sim::{Fx, Spec, World};

use crate::mapping::{Align, Field, Mapping};
use crate::translate;

/// What [`watching`] gives each compared frame.
type Watcher = Box<dyn FnMut(&Lockstep)>;

/// APU register writes: the register's offset from $4000 and the value.
type ApuWrites = Vec<(u8, u8)>;

/// Frame counts by a name, in the order the names were first seen.
type Counts = Vec<(String, u64)>;

thread_local! {
    /// The watcher on this thread.
    static WATCH: std::cell::RefCell<Option<Watcher>> = const { std::cell::RefCell::new(None) };
}

/// Runs `body` with `f` told about every compared frame of every lockstep
/// run on this thread inside it: the lockstep so far. It only watches,
/// so the results are the same with or without it. The background queue
/// streams a clip's score with it as it goes (docs/show.md, "The live
/// cards").
pub fn watching<T>(f: impl FnMut(&Lockstep) + 'static, body: impl FnOnce() -> T) -> T {
    let before = WATCH.with(|w| w.replace(Some(Box::new(f))));
    let out = body();
    WATCH.with(|w| *w.borrow_mut() = before);
    out
}

fn watch(out: &Lockstep) {
    WATCH.with(|w| {
        if let Ok(mut w) = w.try_borrow_mut()
            && let Some(f) = w.as_mut()
        {
            f(out);
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// At a split, load the original's state into the model and go on.
    #[default]
    Resync,
    /// Never put the model back.
    Free,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Resync => "resync",
            Self::Free => "free",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "resync" => Ok(Self::Resync),
            "free" => Ok(Self::Free),
            _ => Err(format!("a mode is resync or free, not `{s}`")),
        }
    }
}

/// The two games' pictures for the screen measure ([`crate::screen`]):
/// whoever runs a lockstep with it draws them. The model's pictures need
/// its presentation's animation, which steps with the model, so it is told
/// each time the model steps or is loaded.
pub trait Screens<O, M> {
    /// The model has stepped one frame, or (`loaded`) had a state loaded.
    fn model_changed(&mut self, model: &M, loaded: bool);
    /// After [`Screens::model_changed`] for a step: the step's events, for
    /// the effects they start in the model's pictures. Nothing by default.
    fn model_events(&mut self, _model: &M, _events: &[gp_runtime::Event]) {}
    /// Both games' pictures of the frame they stand at: the original's and
    /// the model's.
    /// `None` when there is nothing to compare on this frame (the model
    /// has no picture yet for the frame the original shows).
    #[allow(clippy::type_complexity)]
    fn shots(
        &mut self,
        original: &O,
        model: &M,
    ) -> Result<Option<(gp_present::raster::Frame, gp_present::raster::Frame)>, String>;
    fn options(&self) -> crate::screen::Options;
    /// Before the original steps a frame: a picture is drawn only when
    /// asked for, so this is where the drawing can be turned on.
    fn original_stepping(&mut self, _original: &mut O) {}
    /// After a load (and its [`Screens::model_changed`]): the original's
    /// values of the mapping's `present.` fields (a body's animation, by
    /// path, in whole numbers), for the model's presentation to start
    /// where the original's is. Nothing by default.
    fn animation(&mut self, _model: &M, _values: &[(String, i64)]) {}
}

/// The original's values of the mapping's `present.` fields, by path.
pub fn animation_of<O: Runtime>(
    original: &O,
    mapping: &Mapping,
) -> Result<Vec<(String, i64)>, String> {
    let m = mapping.animation();
    if m.fields.is_empty() {
        return Ok(Vec::new());
    }
    let values = translate::read_original(original, &m)?;
    Ok(m.fields
        .iter()
        .zip(values)
        .filter_map(|(f, v)| {
            let n = match v {
                Value::Int(n) => n,
                Value::Fixed(r) => r >> 16,
                _ => return None,
            };
            Some((f.path.clone(), n))
        })
        .collect())
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub mode: Mode,
    /// Each mapped path's tolerance, in the mapping's order: the distance
    /// past which it splits. Missing entries are 0 (exact).
    pub tolerance: Vec<Fx>,
    /// The original's frame to start at: the first aligned frame at or
    /// after it is translated into the model and compared from. `None`
    /// starts at the mapping's start, as `compare` does.
    pub from: Option<u64>,
    /// With `from`, the canonical model state to load there instead of
    /// translating the original's there: a start already taken
    /// ([`start_at`]), for a scenario's presses and the refinement loop's
    /// trials (docs/interpreter.md).
    pub start_state: Option<Vec<u8>>,
    /// Keep both games' value on every mapped path at every compared
    /// frame, in [`Lockstep::values`].
    pub values: bool,
    /// With `from`, stop as soon as the start is taken: only
    /// [`Lockstep::start`] is wanted (the game view's start state,
    /// docs/inspector.md, "The same commands to both").
    pub until_start: bool,
    /// Keep both games' APU writes for every compared frame, in
    /// [`Lockstep::sound`]: the original's `apu-write` events (gp-nes with
    /// its event log on) and the model's ([`crate::runtime::APU_KIND`],
    /// an interpreter made [`crate::Interp::with_apu_events`]).
    pub sound: bool,
    /// Stop at the first split: only whether the run splits, and where,
    /// is wanted (`gp capture prune --mutants` on a clip the real model
    /// plays exactly). The frames after it aren't run.
    pub stop_at_split: bool,
}

/// One mapped path over the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSeries {
    pub path: String,
    pub tolerance: Fx,
    /// The distance at each compared frame.
    pub distance: Vec<Fx>,
    /// Frames whose distance is within the tolerance.
    pub within: u64,
    pub max: Fx,
    /// The first split: the index into [`Lockstep::frames`].
    pub first_split: Option<usize>,
}

impl PathSeries {
    /// The mean distance, rounded down.
    pub fn mean(&self) -> Fx {
        if self.distance.is_empty() {
            return Fx::ZERO;
        }
        let sum: i128 = self.distance.iter().map(|d| i128::from(d.raw())).sum();
        Fx::from_raw((sum / self.distance.len() as i128) as i64)
    }
}

/// One path at a split, both values as they stood before any resync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPath {
    pub path: String,
    pub original: Value,
    pub model: Value,
    pub distance: Fx,
}

/// A frame on which paths went past their tolerance. In free mode only the
/// paths that were within it the frame before are listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    /// The index into [`Lockstep::frames`].
    pub at: usize,
    pub frame: u64,
    pub original_frame: u64,
    pub paths: Vec<SplitPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// The model's and the original's frame the run was started at.
    pub frame: u64,
    pub original_frame: u64,
    /// How the original's state was taken.
    pub notes: Vec<LoadNote>,
    /// The canonical model state loaded there.
    pub state: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lockstep {
    pub mode: Mode,
    pub original_frames: u64,
    pub model_frames: u64,
    pub before_start: u64,
    pub lag_frames: u64,
    /// With `from`, where it started and what the translation loaded.
    pub start: Option<Start>,
    /// Each compared frame: the model's frame and the original's.
    pub frames: Vec<(u64, u64)>,
    pub paths: Vec<PathSeries>,
    pub splits: Vec<Split>,
    /// The notes of the first resync: what a resync puts back and what it
    /// can't. Each one's are the same but for values.
    pub resync_notes: Vec<LoadNote>,
    /// The model's state hash at each compared frame, after any resync.
    pub hashes: Vec<u64>,
    /// With [`Options::values`], each compared frame's values, the
    /// original's and the model's in [`Lockstep::paths`]' order (the
    /// mapping's, then the slot bodies' measures), the model's as it stood
    /// before any resync. A body measure's value is empty text on a frame
    /// its game doesn't have the body.
    pub values: Vec<(Vec<Value>, Vec<Value>)>,
    /// With [`Options::values`], each compared frame's mapped paths (the
    /// mapping's measured fields, in [`Lockstep::values`]' order) that a
    /// `[[field.case]]` leaves unmeasured there: the run counts them as
    /// agreeing whatever the two values are.
    pub unmeasured: Vec<Vec<bool>>,
    /// With [`Options::values`], each compared frame's level: the
    /// original's as the mapping reads it (`None` when it can't say), and
    /// the model's.
    pub levels: Vec<(Option<String>, String)>,
    /// With [`Options::values`], the kinds of the events the model's step
    /// to each compared frame emitted (none on the first).
    pub events: Vec<Vec<String>>,
    /// With [`Screens`], the two games' pictures compared on every
    /// compared frame, the model's as it stood before any resync.
    pub screen: Option<crate::screen::Screen>,
    /// The bodies other than the player, paired across the two games, with
    /// their measures and events ([`crate::bodies`]).
    pub bodies: Vec<crate::bodies::Body>,
    /// With the mapping's `[stretches]`, each compared frame's scene on
    /// the original's side: whether it is play (`stretches.play`, every
    /// frame without one) and the first of `stretches.state` that holds,
    /// by its index. Empty without `[stretches]`.
    pub scene: Vec<(bool, Option<u16>)>,
    /// With the mapping's `[scope]`, the original's first frame out of
    /// the template's scope: the run stopped there, uncompared.
    pub scope_end: Option<u64>,
    /// The input's frames from [`Lockstep::scope_end`] on, which the run
    /// left out of scope (0 without one).
    pub out_of_scope: u64,
    /// With [`Options::sound`], each compared frame's APU writes, the
    /// original's and the model's, each as the register's offset from
    /// $4000 and the value, in order: the original's since the compared
    /// frame before (its lag frames' too), the model's on its step to the
    /// frame. The first compared frame has none.
    pub sound: Vec<(ApuWrites, ApuWrites)>,
}

/// The APU writes among a step's events: gp-nes's `apu-write` events
/// (`addr` $4000-$4013, $4015 or $4017) and an interpreter's (`reg`).
pub fn apu_writes(events: &[gp_runtime::Event]) -> Vec<(u8, u8)> {
    let int = |e: &gp_runtime::Event, p: &str| {
        e.data
            .iter()
            .find(|d| d.path == p)
            .and_then(|d| d.value.as_int())
    };
    events
        .iter()
        .filter(|e| e.kind == crate::runtime::APU_KIND)
        .filter_map(|e| {
            let reg = match int(e, "reg") {
                Some(r) => r,
                None => int(e, "addr")? - 0x4000,
            };
            let apu = (0..=0x13).contains(&reg) || reg == 0x15 || reg == 0x17;
            Some((
                u8::try_from(reg).ok().filter(|_| apu)?,
                int(e, "value")? as u8,
            ))
        })
        .collect()
}

/// How far apart the original's value and the model's are for a field: a
/// number's absolute difference in the model's units (whole parts only
/// with `whole`), a flag's or a name's 0 when equal and 1 when not.
pub fn distance(f: &Field, original: &Value, model: &Value) -> Fx {
    distance_of(f.whole, original, model)
}

/// [`distance`], whole parts only with `whole`.
pub fn distance_of(whole: bool, original: &Value, model: &Value) -> Fx {
    let num = |v: &Value| match v {
        Value::Fixed(r) => Some(i128::from(*r)),
        Value::Int(n) => Some(i128::from(*n) << 16),
        Value::Entity(n) => Some(i128::from(*n) << 16),
        _ => None,
    };
    match (num(original), num(model)) {
        (Some(a), Some(b)) => {
            let (a, b) = if whole {
                ((a >> 16) << 16, (b >> 16) << 16)
            } else {
                (a, b)
            };
            Fx::from_raw(i64::try_from((a - b).abs()).unwrap_or(i64::MAX))
        }
        _ if crate::mapping::same(original, model) => Fx::ZERO,
        _ => Fx::ONE,
    }
}

/// The model's value of each field. A field a case leaves unmeasured on
/// this frame (`quiet`) whose path names nothing in the model's state
/// (a body of a kind the level has none of, such as Donkey Kong's hammers
/// on 75m) reads as false: it isn't compared.
fn read_model<M: Runtime>(
    model: &M,
    mapping: &Mapping,
    quiet: &[bool],
) -> Result<Vec<Value>, String> {
    mapping
        .fields
        .iter()
        .zip(quiet)
        .map(|(f, q)| match model.query(&f.path) {
            Err(_) if *q => Ok(Value::Flag(false)),
            Err(e) => Err(e),
            Ok(found) => match found.as_slice() {
                [Entry { value, .. }] => Ok(value.clone()),
                other => Err(format!(
                    "{} names {} values in the model's state, not one",
                    f.path,
                    other.len()
                )),
            },
        })
        .collect()
}

/// The mapping's start condition, parsed.
pub fn start_of(mapping: &Mapping) -> Result<Option<gp_runtime::select::Condition>, String> {
    mapping
        .start
        .as_deref()
        .map(gp_runtime::select::Condition::parse)
        .transpose()
        .map_err(|e| format!("the mapping's start: {e}"))
}

/// Whether the start condition holds on the original as it stands.
pub(crate) fn start_holds<O: Runtime>(
    original: &O,
    cond: &gp_runtime::select::Condition,
) -> Result<bool, String> {
    cond.eval(&mut |sel| {
        original
            .query(&sel.to_string())?
            .first()
            .and_then(|e| e.value.as_int())
            .ok_or(format!("{sel} names no number"))
    })
}

pub fn polled(events: &[gp_runtime::Event]) -> bool {
    events
        .iter()
        .filter(|e| e.kind == crate::runtime::FRAME_KIND)
        .flat_map(|e| &e.data)
        .any(|d| d.path == "polled" && d.value == Value::Flag(true))
}

/// Loads the original's state, translated through `mapping` (the whole
/// mapping, presentation fields too) onto `base`, into the model: a
/// lockstep's start (`resync` false) or a resync.
pub fn load<O: Runtime, M: Runtime>(
    original: &O,
    model: &mut M,
    spec: &Spec,
    base: &World,
    mapping: &Mapping,
    resync: bool,
) -> Result<Vec<LoadNote>, String> {
    let t = if resync {
        translate::for_resync(original, spec, base, mapping)?
    } else {
        translate::from_original(original, spec, base, mapping)?
    };
    let mut notes = t.notes.clone();
    notes.extend(
        model
            .load_state(&t.state())
            .map_err(|e| format!("the model: {e}"))?,
    );
    Ok(notes)
}

/// Runs both from power-on through `inputs`, measuring every aligned
/// frame. `spec` is the model's, to read and build its canonical states.
pub fn lockstep<O: Runtime, M: Runtime>(
    original: &mut O,
    model: &mut M,
    spec: &Spec,
    mapping: &Mapping,
    inputs: &[InputFrame],
    opts: &Options,
) -> Result<Lockstep, String> {
    lockstep_with(original, model, spec, mapping, inputs, opts, None)
}

/// [`lockstep`], with the screen measure when `screens` draws the two
/// games' pictures.
pub fn lockstep_with<O: Runtime, M: Runtime>(
    original: &mut O,
    model: &mut M,
    spec: &Spec,
    mapping: &Mapping,
    inputs: &[InputFrame],
    opts: &Options,
    screens: Option<&mut dyn Screens<O, M>>,
) -> Result<Lockstep, String> {
    run(
        original, model, spec, mapping, inputs, opts, screens, None, None,
    )
    .map(|r| r.0)
}

/// Where a lockstep from power-on with a `from` stands after its first
/// `inputs` frames, before its start is taken: the counts so far, which
/// depend only on the original and the mapping's start condition. With
/// the original's state there, it is all [`lockstep_resumed`] needs to go
/// on as if it had played them (a clip's start snapshot, kept outside the
/// repo by the caller).
///
/// The model plays those frames too in a run from power-on, but its world
/// is replaced when the start is taken (translated from the original onto
/// a power-on world at the model's frame), so only its frame carries
/// over: the model steps it took, one frame each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Prefix {
    /// Frames of the input played.
    pub inputs: usize,
    pub original_frames: u64,
    pub model_frames: u64,
    pub before_start: u64,
    pub lag_frames: u64,
    /// Whether the mapping's start held (or, with none, the game had read
    /// the controller).
    pub started: bool,
}

impl Prefix {
    fn of(out: &Lockstep, inputs: usize, started: bool) -> Self {
        Prefix {
            inputs,
            original_frames: out.original_frames,
            model_frames: out.model_frames,
            before_start: out.before_start,
            lag_frames: out.lag_frames,
            started,
        }
    }
}

/// Plays the first `n` frames of `inputs` as [`lockstep_with`] would
/// with `opts` (which must have a `from` past them), and gives where the
/// run stands ([`Prefix`]); `original` is left after those frames. An
/// error when the start comes before frame `n`, or when the model's frame
/// isn't one a step.
pub fn prefix<O: Runtime, M: Runtime>(
    original: &mut O,
    model: &mut M,
    spec: &Spec,
    mapping: &Mapping,
    inputs: &[InputFrame],
    opts: &Options,
    n: usize,
) -> Result<Prefix, String> {
    if opts.from.is_none() || n > inputs.len() {
        return Err("a prefix needs a start frame and the input to cover it".into());
    }
    let power_on = World::power_on(spec).frame;
    let (_, p) = run(
        original,
        model,
        spec,
        mapping,
        inputs,
        opts,
        None,
        None,
        Some(n),
    )?;
    let p = p.ok_or("the start was taken before the prefix's end")?;
    if model.frame() != power_on + p.model_frames {
        return Err(format!(
            "the model stands at frame {} after {} steps",
            model.frame(),
            p.model_frames
        ));
    }
    Ok(p)
}

/// [`lockstep_with`] going on from `at`: `original` stands after the
/// first `at.inputs` frames of `inputs` (restored from a snapshot taken
/// there), and the model is put at a power-on world at the frame those
/// frames' steps left it at. Gives the same run as from power-on.
#[allow(clippy::too_many_arguments)]
pub fn lockstep_resumed<O: Runtime, M: Runtime>(
    original: &mut O,
    model: &mut M,
    spec: &Spec,
    mapping: &Mapping,
    inputs: &[InputFrame],
    opts: &Options,
    screens: Option<&mut dyn Screens<O, M>>,
    at: Prefix,
) -> Result<Lockstep, String> {
    if opts.from.is_none_or(|f| original.frame() >= f)
        || at.inputs > inputs.len()
        || original.frame() != at.original_frames
    {
        return Err(
            "a resumed lockstep needs the original at the prefix's end and its start ahead".into(),
        );
    }
    let mut w = World::power_on(spec);
    w.frame += at.model_frames;
    model
        .restore(&w.canonical())
        .map_err(|e| format!("the model: {e}"))?;
    run(
        original,
        model,
        spec,
        mapping,
        inputs,
        opts,
        screens,
        Some(at),
        None,
    )
    .map(|r| r.0)
}

#[allow(clippy::too_many_arguments)]
fn run<O: Runtime, M: Runtime>(
    original: &mut O,
    model: &mut M,
    spec: &Spec,
    mapping: &Mapping,
    inputs: &[InputFrame],
    opts: &Options,
    mut screens: Option<&mut dyn Screens<O, M>>,
    resume: Option<Prefix>,
    pause: Option<usize>,
) -> Result<(Lockstep, Option<Prefix>), String> {
    let Align::Polled = mapping.align;
    mapping.read_original_at(original)?;
    // A translation loads every field; a run compares all but the
    // presentation ones.
    let full = mapping;
    let measured = mapping.measured();
    let mapping = &measured;
    let mut out = Lockstep {
        mode: opts.mode,
        original_frames: resume.map_or(0, |p| p.original_frames),
        model_frames: resume.map_or(0, |p| p.model_frames),
        before_start: resume.map_or(0, |p| p.before_start),
        lag_frames: resume.map_or(0, |p| p.lag_frames),
        start: None,
        frames: Vec::new(),
        paths: mapping
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| PathSeries {
                path: f.path.clone(),
                tolerance: opts.tolerance.get(i).copied().unwrap_or(Fx::ZERO),
                distance: Vec::new(),
                within: 0,
                max: Fx::ZERO,
                first_split: None,
            })
            .collect(),
        splits: Vec::new(),
        resync_notes: Vec::new(),
        hashes: Vec::new(),
        values: Vec::new(),
        unmeasured: Vec::new(),
        levels: Vec::new(),
        events: Vec::new(),
        screen: screens
            .as_ref()
            .map(|s| crate::screen::Screen::new(&s.options())),
        bodies: Vec::new(),
        scene: Vec::new(),
        scope_end: None,
        out_of_scope: 0,
        sound: Vec::new(),
    };
    let scene = Scene::of(full)?;
    let scope = ScopeCheck::of(full)?;
    // The `leaves` selectors' values on the first compared frame.
    let scope_base: std::cell::RefCell<Option<Vec<i64>>> = std::cell::RefCell::new(None);
    let mut tracker = crate::bodies::Tracker::new(spec, mapping, &mut out);
    // Whether each path was past its tolerance the frame before, for free
    // mode's splits, and each body measure's.
    let mut apart = vec![false; mapping.fields.len()];
    let mut body_apart: Vec<bool> = Vec::new();
    let mut check = |out: &mut Lockstep,
                     original: &O,
                     model: &mut M,
                     events: Vec<String>,
                     screens: &mut Option<&mut dyn Screens<O, M>>|
     -> Result<(), String> {
        let of = original.frame();
        if let Some(from) = opts.from {
            if of < from {
                return Ok(());
            }
            if out.start.is_none() {
                // A runtime that takes random draws starts them here.
                let _ = model.set(crate::runtime::DRAWS_BASE, Value::Int(of as i64));
                let notes = match &opts.start_state {
                    Some(state) => model
                        .load_state(state)
                        .map_err(|e| format!("the model: {e}"))?,
                    None => {
                        let mut base = World::power_on(spec);
                        base.frame = model.frame();
                        load(original, model, spec, &base, full, false)?
                    }
                };
                if let Some(sc) = screens.as_deref_mut() {
                    sc.model_changed(model, true);
                    sc.animation(model, &animation_of(original, full)?);
                }
                out.start = Some(Start {
                    frame: model.frame(),
                    original_frame: of,
                    notes,
                    state: model.canonical_state(),
                });
            }
        }
        let orig = translate::read_original(original, mapping)?;
        let quiet = translate::unmeasured(original, mapping)?;
        let mine = read_model(model, mapping, &quiet)?;
        let at = out.frames.len();
        if at == 0
            && let Some(sc) = &scope
        {
            *scope_base.borrow_mut() = Some(sc.values(original)?);
        }
        let bodies = tracker.frame(out, spec, original, model, at, &orig, &mine, opts.values)?;
        out.frames.push((model.frame(), of));
        if let Some(sc) = &scene {
            out.scene.push(sc.read(original)?);
        }
        if opts.values {
            let theirs = translate::level_of(original, mapping, &orig)?.map(|(l, _)| l);
            let ours = match model.query("level")?.first().map(|e| &e.value) {
                Some(Value::Text(l)) => l.clone(),
                _ => String::new(),
            };
            out.levels.push((theirs, ours));
            out.events.push(events);
            let (mut o, mut m) = (orig.clone(), mine.clone());
            for (_, ov, mv, _) in &bodies {
                o.push(ov.clone());
                m.push(mv.clone());
            }
            out.values.push((o, m));
            out.unmeasured.push(quiet.clone());
        }
        if let (Some(sc), Some(screen)) = (screens.as_deref_mut(), out.screen.as_mut()) {
            match sc.shots(original, model)? {
                Some((theirs, ours)) => screen.push(&theirs, &ours),
                None => screen.skip(),
            }
        }
        let mut split = Vec::new();
        for (i, f) in mapping.fields.iter().enumerate() {
            // A case that leaves the field unmeasured counts it as agreeing.
            let d = if quiet[i] {
                Fx::ZERO
            } else {
                distance(f, &orig[i], &mine[i])
            };
            let s = &mut out.paths[i];
            s.distance.push(d);
            s.max = s.max.max(d);
            let past = d > s.tolerance;
            if past {
                if s.first_split.is_none() {
                    s.first_split = Some(at);
                }
                if opts.mode == Mode::Resync || !apart[i] {
                    split.push(SplitPath {
                        path: f.path.clone(),
                        original: orig[i].clone(),
                        model: mine[i].clone(),
                        distance: d,
                    });
                }
            } else {
                s.within += 1;
            }
            apart[i] = past;
        }
        // A body measure splits when it goes past its tolerance, in either
        // mode: a resync doesn't put the model's bodies back.
        let mapped_split = !split.is_empty();
        body_apart.resize(out.paths.len(), false);
        for (j, ov, mv, d) in bodies {
            let s = &mut out.paths[j];
            s.distance.push(d);
            s.max = s.max.max(d);
            let past = d > s.tolerance;
            if past {
                if s.first_split.is_none() {
                    s.first_split = Some(at);
                }
                if !body_apart[j] {
                    split.push(SplitPath {
                        path: s.path.clone(),
                        original: ov,
                        model: mv,
                        distance: d,
                    });
                }
            } else {
                s.within += 1;
            }
            body_apart[j] = past;
        }
        if !split.is_empty() {
            out.splits.push(Split {
                at,
                frame: model.frame(),
                original_frame: of,
                paths: split,
            });
            // With `slots.resync` a body's split puts the world back too, its
            // bodies carried from the original's slots.
            let carries = full.slots.as_ref().is_some_and(|s| s.resync);
            if opts.mode == Mode::Resync && (mapped_split || carries) {
                let base = World::from_canonical(spec, &model.canonical_state())?;
                let notes = load(original, model, spec, &base, full, true)?;
                if carries {
                    tracker.resynced(out);
                }
                if let Some(sc) = screens.as_deref_mut() {
                    sc.model_changed(model, true);
                    sc.animation(model, &animation_of(original, full)?);
                }
                if out.resync_notes.is_empty() {
                    out.resync_notes = notes;
                }
                apart.iter_mut().for_each(|a| *a = false);
            }
        }
        out.hashes.push(model.state_hash());
        watch(out);
        Ok(())
    };
    let start = start_of(mapping)?;
    let holds = start_holds::<O>;
    // Without a start condition the model's power-on state stands for the
    // frame before the first polled one, so the original is kept one frame
    // behind until then.
    let mut before = original.fork();
    let mut started = resume.is_some_and(|p| p.started);
    // The original's APU writes since the last compared frame, and the
    // model's on its last step.
    let mut heard: Vec<(u8, u8)> = Vec::new();
    let mut said: Vec<(u8, u8)> = Vec::new();
    // After a check: a compared frame takes what was heard since the last
    // one, the first none.
    let hear = |out: &mut Lockstep, heard: &mut Vec<(u8, u8)>, said: &mut Vec<(u8, u8)>| {
        if opts.sound && out.frames.len() > out.sound.len() {
            let (o, m) = (std::mem::take(heard), std::mem::take(said));
            let first = out.sound.is_empty();
            out.sound.push(if first {
                (Vec::new(), Vec::new())
            } else {
                (o, m)
            });
        }
    };
    // A resumed run took this look at power-on, in its prefix.
    if resume.is_none()
        && let Some(cond) = &start
        && holds(original, cond)?
    {
        started = true;
        check(&mut out, original, model, Vec::new(), &mut screens)?;
        hear(&mut out, &mut heard, &mut said);
    }
    let skip = resume.map_or(0, |p| p.inputs);
    for (i, input) in inputs.iter().enumerate().skip(skip) {
        if pause == Some(i) {
            if out.start.is_some() {
                return Ok((out, None));
            }
            let p = Prefix::of(&out, i, started);
            return Ok((out, Some(p)));
        }
        if opts.until_start && out.start.is_some() {
            break;
        }
        if opts.stop_at_split && !out.splits.is_empty() {
            break;
        }
        if start.is_none() && !started {
            before = original.fork();
        }
        if let Some(sc) = screens.as_deref_mut() {
            sc.original_stepping(original);
        }
        let events = original
            .step(std::slice::from_ref(input))
            .map_err(|e| format!("the original: {e}"))?;
        if opts.sound {
            heard.extend(apu_writes(&events));
        }
        out.original_frames += 1;
        // Out of the template's scope: the run stops on this frame, and it
        // and the input's frames after it are counted apart.
        if let (Some(sc), Some(base)) = (&scope, scope_base.borrow().as_ref())
            && !sc.holds(original, base)?
        {
            out.scope_end = Some(original.frame());
            out.out_of_scope = (inputs.len() - i) as u64;
            break;
        }
        if let Some(cond) = &start
            && !started
        {
            if holds(original, cond)? {
                started = true;
                check(&mut out, original, model, Vec::new(), &mut screens)?;
                hear(&mut out, &mut heard, &mut said);
            } else {
                out.before_start += 1;
            }
            continue;
        }
        if !polled(&events) {
            if started {
                out.lag_frames += 1;
                // A runtime that takes random draws counts it too.
                let _ = model.set(crate::runtime::DRAWS_LAG, Value::Int(1));
            } else {
                out.before_start += 1;
            }
            continue;
        }
        if !started {
            started = true;
            check(&mut out, &before, model, Vec::new(), &mut screens)?;
            hear(&mut out, &mut heard, &mut said);
        }
        let mut events = model
            .step(std::slice::from_ref(input))
            .map_err(|e| format!("the model: {e}"))?;
        if opts.sound {
            said = apu_writes(&events);
        }
        events.retain(|e| e.kind != crate::runtime::APU_KIND);
        if let Some(sc) = screens.as_deref_mut() {
            sc.model_changed(model, false);
            sc.model_events(model, &events);
        }
        out.model_frames += 1;
        let kinds = if opts.values {
            events.into_iter().map(|e| e.kind).collect()
        } else {
            Vec::new()
        };
        check(&mut out, original, model, kinds, &mut screens)?;
        hear(&mut out, &mut heard, &mut said);
    }
    if pause == Some(inputs.len()) && out.start.is_none() {
        let p = Prefix::of(&out, inputs.len(), started);
        return Ok((out, Some(p)));
    }
    Ok((out, None))
}

/// The mapping's `[scope]`, parsed once a run.
pub(crate) struct ScopeCheck {
    while_: Option<gp_runtime::select::Condition>,
    leaves: Vec<String>,
}

impl ScopeCheck {
    pub(crate) fn of(mapping: &Mapping) -> Result<Option<Self>, String> {
        let Some(sc) = &mapping.scope else {
            return Ok(None);
        };
        Ok(Some(Self {
            while_: sc
                .while_
                .as_deref()
                .map(gp_runtime::select::Condition::parse)
                .transpose()
                .map_err(|e| format!("scope.while: {e}"))?,
            leaves: sc
                .leaves
                .iter()
                .map(|l| {
                    gp_runtime::select::Selector::parse(l)
                        .map(|x| x.to_string())
                        .map_err(|e| format!("scope.leaves `{l}`: {e}"))
                })
                .collect::<Result<_, _>>()?,
        }))
    }

    /// The `leaves` selectors' values.
    pub(crate) fn values<O: Runtime>(&self, original: &O) -> Result<Vec<i64>, String> {
        self.leaves
            .iter()
            .map(|l| {
                original
                    .query(l)?
                    .first()
                    .and_then(|e| e.value.as_int())
                    .ok_or(format!("scope.leaves {l} names no number"))
            })
            .collect()
    }

    /// Whether the original is still in scope, `base` the `leaves`
    /// values on the first compared frame.
    pub(crate) fn holds<O: Runtime>(&self, original: &O, base: &[i64]) -> Result<bool, String> {
        if let Some(c) = &self.while_ {
            let ok = c.eval(&mut |sel| {
                original
                    .query(&sel.to_string())?
                    .first()
                    .and_then(|e| e.value.as_int())
                    .ok_or(format!("{sel} names no number"))
            })?;
            if !ok {
                return Ok(false);
            }
        }
        Ok(self.values(original)? == base)
    }
}

/// How a run's frames were paired: the original's frame for each model
/// frame. A lag frame, on which the original doesn't read its input, is
/// skipped by the model, so from it on the original is one frame further
/// ahead.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Pairing {
    /// The first compared frame: the model's and the original's.
    pub model: u64,
    pub original: u64,
    /// The original's frame less the model's on the first compared frame:
    /// model frame n is the original's n + offset.
    pub offset: i64,
    /// The original's lag frames the run skipped between compared frames.
    pub lag: Vec<u64>,
    /// The offset on the last compared frame.
    pub last_offset: i64,
}

impl Pairing {
    /// "model frame n is the original's n+3", for people.
    pub fn says(&self) -> String {
        let n = |o: i64| match o {
            0 => "n".to_owned(),
            o if o > 0 => format!("n+{o}"),
            o => format!("n{o}"),
        };
        let mut s = format!(
            "model frame n is the original's {} (model {} = original {})",
            n(self.offset),
            self.model,
            self.original
        );
        if self.lag.is_empty() {
            s.push_str("; no lag frames skipped");
        } else {
            s.push_str(&format!(
                "; {} lag frame{} skipped: {}; after them, {}",
                self.lag.len(),
                if self.lag.len() == 1 { "" } else { "s" },
                spans(&self.lag),
                n(self.last_offset)
            ));
        }
        s
    }
}

/// Frames as spans: `1450, 2210-2212`.
fn spans(frames: &[u64]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < frames.len() {
        let mut j = i;
        while j + 1 < frames.len() && frames[j + 1] == frames[j] + 1 {
            j += 1;
        }
        out.push(if i == j {
            frames[i].to_string()
        } else {
            format!("{}-{}", frames[i], frames[j])
        });
        i = j + 1;
    }
    out.join(", ")
}

/// The pairing of a run's compared frames ([`Lockstep::frames`]): the
/// offset on the first, and every original frame skipped between two
/// compared ones (a lag frame). `None` when nothing was compared.
pub fn pairing(frames: &[(u64, u64)]) -> Option<Pairing> {
    let &(m0, o0) = frames.first()?;
    let &(ml, ol) = frames.last()?;
    let mut lag = Vec::new();
    for w in frames.windows(2) {
        let ((ma, oa), (mb, ob)) = (w[0], w[1]);
        // The original steps once with each model step, and once more on
        // each lag frame, which the run passes before the next polled one.
        let steps = mb.saturating_sub(ma).max(1);
        if ob > oa + steps {
            lag.extend(oa + 1..ob + 1 - steps);
        }
    }
    let off = |m: u64, o: u64| o as i64 - m as i64;
    Some(Pairing {
        model: m0,
        original: o0,
        offset: off(m0, o0),
        lag,
        last_offset: off(ml, ol),
    })
}

/// The mapping's `[stretches]` conditions, parsed once a run.
struct Scene {
    play: Option<gp_runtime::select::Condition>,
    states: Vec<gp_runtime::select::Condition>,
}

impl Scene {
    fn of(mapping: &Mapping) -> Result<Option<Self>, String> {
        let Some(st) = &mapping.stretches else {
            return Ok(None);
        };
        let parse = |c: &str| {
            gp_runtime::select::Condition::parse(c).map_err(|e| format!("stretches `{c}`: {e}"))
        };
        Ok(Some(Self {
            play: st.play.as_deref().map(parse).transpose()?,
            states: st
                .state
                .iter()
                .map(|n| parse(&n.when))
                .collect::<Result<_, _>>()?,
        }))
    }

    fn read<O: Runtime>(&self, original: &O) -> Result<(bool, Option<u16>), String> {
        let holds = |cond: &gp_runtime::select::Condition| {
            cond.eval(&mut |sel| {
                original
                    .query(&sel.to_string())?
                    .first()
                    .and_then(|e| e.value.as_int())
                    .ok_or(format!("{sel} names no number"))
            })
        };
        let play = match &self.play {
            Some(c) => holds(c)?,
            None => true,
        };
        let mut state = None;
        for (i, c) in self.states.iter().enumerate() {
            if holds(c)? {
                state = Some(i as u16);
                break;
            }
        }
        Ok((play, state))
    }
}

/// A run longer than this many frames is a long run, scored by its exact
/// stretches in resync mode as well (docs/benchmarks.md, "Long runs").
pub const LONG_RUN: u64 = 600;

/// Frames a second, NTSC, for stretches in seconds.
pub const NTSC_FPS_MILLI: u64 = 60_099;

/// A run's exact stretches (docs/benchmarks.md, "Long runs"): runs of
/// play frames on which every measure is within its tolerance, each ended
/// by a split event (one or more split frames in a row). A frame outside
/// play ends a stretch and isn't counted. A stretch that starts after
/// such a frame and the one the run's end cuts off aren't whole; the one
/// from the run's start is, since the start loads the original's state.
/// A clean stretch that leaving play ends (a clear scene, a death) isn't
/// whole either, but the longest counts it, as it counts the one the end
/// cuts off: a run exact until the stage clears is exact that long. So it
/// counts a stretch that starts after a frame out of play too, such as a
/// title's demo or a game started from a menu: the frames are exact all
/// the same.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Stretches {
    /// The whole stretches' lengths in frames, shortest first.
    pub whole: Vec<u64>,
    /// The stretch the run's end cut off, in frames (0 when the last
    /// frame split or wasn't play), wherever it started.
    pub open_end: u64,
    /// The longest other stretch that isn't whole, in frames: one a frame
    /// out of play ended (the model exact until play stopped), or one that
    /// started after a frame out of play and a split ended. Not whole, so
    /// not in the low-end statistics.
    #[serde(default)]
    pub cut_by_scene: u64,
    /// Frames on which every measure is within its tolerance, and frames
    /// compared.
    pub together: u64,
    pub frames: u64,
    /// Play frames compared, and split events.
    pub play: u64,
    pub events: u64,
}

impl Stretches {
    /// The `p`-th percentile whole stretch, `p` from 0 to 100, as
    /// `runs[floor(p/100 * (n - 1))]`.
    pub fn percentile(&self, p: u64) -> u64 {
        if self.whole.is_empty() {
            return 0;
        }
        let i = (p * (self.whole.len() as u64 - 1) / 100) as usize;
        self.whole[i]
    }

    /// The mean of the shortest tenth (at least one), rounded down.
    pub fn low_mean(&self) -> u64 {
        if self.whole.is_empty() {
            return 0;
        }
        let k = (self.whole.len() / 10).max(1);
        self.whole[..k].iter().sum::<u64>() / k as u64
    }

    pub fn median(&self) -> u64 {
        self.whole.get(self.whole.len() / 2).copied().unwrap_or(0)
    }

    pub fn longest(&self) -> u64 {
        self.whole.last().copied().unwrap_or(0)
    }

    /// The longest, counting the stretches the run's end or leaving play
    /// cut off: a run exact to its end, or until its stage clears, has
    /// that one.
    pub fn longest_any(&self) -> u64 {
        self.longest().max(self.open_end).max(self.cut_by_scene)
    }

    /// Whole stretches of `secs` seconds or more.
    pub fn at_least(&self, secs: u64) -> usize {
        self.whole
            .iter()
            .filter(|f| **f * 1000 >= secs * NTSC_FPS_MILLI)
            .count()
    }
}

/// Frames as seconds, to a tenth, for people.
pub fn seconds(frames: u64) -> String {
    let tenths = (frames * 10_000 + NTSC_FPS_MILLI / 2) / NTSC_FPS_MILLI;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Whether every measure is within its tolerance at each compared frame.
pub fn together(ls: &Lockstep) -> Vec<bool> {
    (0..ls.frames.len())
        .map(|i| {
            ls.paths
                .iter()
                .all(|p| p.distance.get(i).is_none_or(|d| *d <= p.tolerance))
        })
        .collect()
}

/// The run's exact stretches ([`Stretches`]).
pub fn stretches(ls: &Lockstep) -> Stretches {
    let ok = together(ls);
    let mut out = Stretches {
        frames: ok.len() as u64,
        together: ok.iter().filter(|o| **o).count() as u64,
        ..Stretches::default()
    };
    let (mut cur, mut clean) = (0u64, true);
    let mut last_split = false;
    for (i, ok) in ok.iter().enumerate() {
        let play = ls.scene.get(i).is_none_or(|s| s.0);
        if !play {
            out.cut_by_scene = out.cut_by_scene.max(cur);
            cur = 0;
            clean = false;
            last_split = false;
            continue;
        }
        out.play += 1;
        if *ok {
            cur += 1;
            last_split = false;
        } else {
            if !last_split {
                out.events += 1;
            }
            if cur > 0 && clean {
                out.whole.push(cur);
            } else {
                out.cut_by_scene = out.cut_by_scene.max(cur);
            }
            cur = 0;
            clean = true;
            last_split = true;
        }
    }
    out.open_end = cur;
    out.whole.sort_unstable();
    out
}

/// Split frames counted by the original's state there (the mapping's
/// `stretches.state`; `None` where none holds) and by measure, each in
/// the order first seen. A split's measures are those past their
/// tolerance on that frame.
pub fn split_causes(ls: &Lockstep, names: &[String]) -> (Counts, Counts) {
    let mut by_state: Vec<(String, u64)> = Vec::new();
    let mut by_measure: Vec<(String, u64)> = Vec::new();
    let bump = |v: &mut Vec<(String, u64)>, k: &str| match v.iter_mut().find(|(n, _)| n == k) {
        Some(e) => e.1 += 1,
        None => v.push((k.to_owned(), 1)),
    };
    for s in &ls.splits {
        let st = match ls.scene.get(s.at).map(|x| x.1) {
            Some(Some(i)) => names.get(usize::from(i)).cloned().unwrap_or_default(),
            Some(None) => "(no state)".to_owned(),
            None => "(not read)".to_owned(),
        };
        bump(&mut by_state, &st);
        for p in &s.paths {
            bump(&mut by_measure, &p.path);
        }
    }
    by_state.sort_by_key(|a| std::cmp::Reverse(a.1));
    by_measure.sort_by_key(|a| std::cmp::Reverse(a.1));
    (by_state, by_measure)
}

/// The mechanics that own a mapped path, by the catalog's records: those
/// the model's part carries whose record writes the value. An entity's
/// `x` belongs to its movers, a counter (and its `every` changes' counts)
/// to `core.counters`, a camera to
/// `core.camera`, a rule to `core.flow`. The part's own mechanics come
/// first, the core's after.
pub fn owners(catalog: &crate::mechanics::Catalog, path: &str) -> Vec<&'static str> {
    let Ok(p) = gp_model::Path::parse(path) else {
        return Vec::new();
    };
    let seg = &p.segments;
    let name = |i: usize| seg.get(i).map(|s| s.name.as_str());
    let (part, writes): (String, Vec<String>) = match (name(0), name(1), name(2)) {
        (Some("entities"), Some(k), Some(field)) => {
            let field = field.strip_prefix("stage_").unwrap_or(field);
            (format!("entities.{k}"), vec![format!("self.{field}")])
        }
        // A body's direction is what turns it and moves it across, its
        // phase what moves it along its path.
        (Some(crate::bodies::PREFIX), Some(k), Some(field)) => {
            let writes: Vec<String> = match field {
                crate::bodies::MOVING => {
                    vec!["self.facing".into(), "self.x".into(), "self.vx".into()]
                }
                crate::bodies::PHASE => vec!["self.y".into(), "self.x".into()],
                f => vec![format!("self.{f}")],
            };
            (format!("entities.{k}"), writes)
        }
        (Some("counters" | "ticks"), Some(c), _) => {
            (format!("counters.{c}"), vec!["counters".into()])
        }
        (Some("cameras"), Some(c), _) => (format!("cameras.{c}"), vec!["camera".into()]),
        (Some("rules"), Some(r), _) => (format!("rules.{r}"), Vec::new()),
        (Some("level"), _, _) => (String::new(), vec!["level".into()]),
        _ => return Vec::new(),
    };
    let used = |m: &crate::mechanics::Mechanic| {
        part.is_empty()
            || m.used_by
                .iter()
                .any(|u| u == &part || u.starts_with(&format!("{part}.")))
    };
    let mut out: Vec<&'static str> = catalog
        .mechanics
        .iter()
        .filter(|m| {
            used(m) && (writes.is_empty() || writes.iter().any(|w| m.writes.contains(&w.as_str())))
        })
        .map(|m| m.name)
        .collect();
    if out.is_empty() {
        // No record says it writes the value: every mechanic of the part.
        out = catalog
            .mechanics
            .iter()
            .filter(|m| !part.is_empty() && used(m))
            .map(|m| m.name)
            .collect();
    }
    // The part's own mechanics (its components and hooks) before the
    // core's, which every entity shares: a speed that parts is the run's
    // before it's the tile collision's.
    out.sort_by_key(|n| n.starts_with("core."));
    out
}

/// Splits grouped by the mechanic that owns each path: its first owner
/// ([`owners`]), the most specific one. For each mechanic,
/// its splits (indexes into `splits`) and the paths that split, in the
/// order the mechanics first split. A path with no owner is grouped under
/// `(unowned)`.
pub fn by_mechanic(
    catalog: &crate::mechanics::Catalog,
    splits: &[Split],
) -> Vec<(String, Vec<usize>, Vec<String>)> {
    let mut out: Vec<(String, Vec<usize>, Vec<String>)> = Vec::new();
    for (i, s) in splits.iter().enumerate() {
        for p in &s.paths {
            let names = vec![
                owners(catalog, &p.path)
                    .first()
                    .map_or("(unowned)".to_owned(), |n| (*n).to_owned()),
            ];
            for n in names {
                let at = match out.iter().position(|(m, ..)| *m == n) {
                    Some(at) => at,
                    None => {
                        out.push((n, Vec::new(), Vec::new()));
                        out.len() - 1
                    }
                };
                let e = &mut out[at];
                if e.1.last() != Some(&i) {
                    e.1.push(i);
                }
                if !e.2.contains(&p.path) {
                    e.2.push(p.path.clone());
                }
            }
        }
    }
    out
}

/// The start of a run from the original's frame `at` of `inputs`: the
/// first frame the model stands for at or after it, the original's state
/// there translated into the model's ([`crate::translate`]), as `from`
/// takes it. Returns the original standing at that frame, ready for
/// [`lockstep_from`], and the start. `original` and `model` are at
/// power-on.
pub fn start_at<O: Runtime, M: Runtime>(
    original: &O,
    model: &mut M,
    spec: &Spec,
    mapping: &Mapping,
    inputs: &[InputFrame],
    at: u64,
) -> Result<(O, Start), String> {
    let mut scan = original.fork();
    // On until the start is taken: past a lag frame, or to the mapping's
    // start when `at` is before it (the title screen).
    let opts = Options {
        mode: Mode::Free,
        from: Some(at),
        until_start: true,
        ..Options::default()
    };
    let ls = lockstep(&mut scan, model, spec, mapping, inputs, &opts)?;
    let start = ls.start.ok_or_else(|| {
        format!(
            "the original never reached a frame the model stands for at or after frame {at} (the mapping's start doesn't hold, or the input ends first)"
        )
    })?;
    let mut there = original.fork();
    let k = usize::try_from(start.original_frame).unwrap_or(usize::MAX);
    for input in &inputs[..k.min(inputs.len())] {
        there
            .step(std::slice::from_ref(input))
            .map_err(|e| format!("the original: {e}"))?;
    }
    Ok((there, start))
}

/// Both games from a start already taken ([`start_at`]) on an explicit
/// input: `original` stands at the start's frame, the model loads the
/// start's state, and both take `inputs` from there, measured on every
/// aligned frame from the start on. The mapping's start condition is not
/// waited for, since the start is past it. A scenario's few presses and a
/// trial of the refinement loop both run this way.
pub fn lockstep_from<O: Runtime, M: Runtime>(
    original: &mut O,
    model: &mut M,
    spec: &Spec,
    mapping: &Mapping,
    start: &Start,
    inputs: &[InputFrame],
    opts: &Options,
) -> Result<Lockstep, String> {
    lockstep_from_with(original, model, spec, mapping, start, inputs, opts, None)
}

/// [`lockstep_from`] with the screen measure ([`lockstep_with`]).
#[allow(clippy::too_many_arguments)]
pub fn lockstep_from_with<O: Runtime, M: Runtime>(
    original: &mut O,
    model: &mut M,
    spec: &Spec,
    mapping: &Mapping,
    start: &Start,
    inputs: &[InputFrame],
    opts: &Options,
    screens: Option<&mut dyn Screens<O, M>>,
) -> Result<Lockstep, String> {
    let mut from_mid = mapping.clone();
    from_mid.start = None;
    let opts = Options {
        from: Some(start.original_frame),
        start_state: Some(start.state.clone()),
        ..opts.clone()
    };
    let mut ls = lockstep_with(original, model, spec, &from_mid, inputs, &opts, screens)?;
    if let Some(s) = &mut ls.start {
        s.notes.clone_from(&start.notes);
    }
    Ok(ls)
}

/// The distance past which a measure has drifted apart, from its units,
/// unless told otherwise (the user, 2026-10-29: "once they drift past a
/// threshold it should stop and refine"): 2 px for a position, 0.5 px a
/// frame for a speed, 1 for a counter, a flag or a name. It is separate
/// from the tolerance: a comparison stops where a measure drifts, while
/// a refinement still aims for exact.
pub fn drift_default(path: &str) -> Fx {
    let field = path.rsplit('.').next().unwrap_or("");
    match field {
        "x" | "y" | "stage_x" | "stage_y" => Fx::from_int(2),
        "vx" | "vy" => Fx::from_raw(1 << 15),
        _ => Fx::ONE,
    }
}

/// Each mapped path's drift threshold: its default, raised to its
/// tolerance when the contract allows more, and set outright by
/// `overrides` (a field name, `x`, or a whole path, and its distance).
pub fn drift_thresholds(paths: &[String], tolerance: &[Fx], overrides: &[(String, Fx)]) -> Vec<Fx> {
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let field = p.rsplit('.').next().unwrap_or("");
            if let Some((_, v)) = overrides.iter().rev().find(|(k, _)| k == p || k == field) {
                return *v;
            }
            drift_default(p).max(tolerance.get(i).copied().unwrap_or(Fx::ZERO))
        })
        .collect()
}

/// Where a run drifts apart: the first compared frame at or after the
/// original's frame `from` on which a measure is past its drift threshold,
/// with the measure (the farthest past it) and its distance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    /// The index into [`Lockstep::frames`].
    pub at: usize,
    pub original_frame: u64,
    pub path: String,
    pub distance: Fx,
    pub threshold: Fx,
}

pub fn drift_point(ls: &Lockstep, thresholds: &[Fx], from: u64) -> Option<Drift> {
    for (k, (_, of)) in ls.frames.iter().enumerate() {
        if *of < from {
            continue;
        }
        let worst = ls
            .paths
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let d = *p.distance.get(k)?;
                let t = thresholds.get(i).copied().unwrap_or(Fx::ONE);
                (d > t).then_some((i, d, t))
            })
            .max_by_key(|(_, d, t)| i128::from(d.raw()) * 65536 / i128::from(t.raw().max(1)));
        if let Some((i, d, t)) = worst {
            return Some(Drift {
                at: k,
                original_frame: *of,
                path: ls.paths[i].path.clone(),
                distance: d,
                threshold: t,
            });
        }
    }
    None
}

/// Whether a mapped path is the camera's (`cameras.*`, `core.camera`). The
/// camera is its own track (the user, 2026-10-29: "the background drifting
/// apart is a separate and easily noticeable thing isolated from getting
/// the character movement correct"): it follows the bodies, so a camera
/// split while a body is wrong is not a target of its own, and the camera
/// is fitted only once the bodies match.
pub fn is_camera(path: &str) -> bool {
    path.starts_with("cameras.") || path.starts_with("core.camera")
}

/// The track a mapped path is scored on: the entity kind it belongs to
/// (`mario` for `entities.mario[0].vx`), `camera`, or `game` for the
/// counters, ticks, rules and level.
pub fn track(path: &str) -> String {
    if is_camera(path) {
        return "camera".into();
    }
    let body = format!("{}.", crate::bodies::PREFIX);
    match path
        .strip_prefix("entities.")
        .or_else(|| path.strip_prefix(body.as_str()))
    {
        Some(rest) => rest.split(['[', '.']).next().unwrap_or(rest).to_owned(),
        None => "game".into(),
    }
}

/// The first wrong frame: the first compared frame at or after the
/// original's frame `from` on which a measure is past its exact tolerance,
/// with every measure wrong there. A camera measure is counted only when
/// no other measure is wrong at or after `from`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstWrong {
    /// The index into [`Lockstep::frames`].
    pub at: usize,
    pub frame: u64,
    pub original_frame: u64,
    /// The indexes into [`Lockstep::paths`] wrong on that frame.
    pub paths: Vec<usize>,
    /// Whether it is the camera's: no other measure is wrong.
    pub camera: bool,
}

pub fn first_wrong(ls: &Lockstep, from: u64) -> Option<FirstWrong> {
    let on = |camera: bool| {
        ls.frames.iter().enumerate().find_map(|(k, (f, of))| {
            if *of < from {
                return None;
            }
            let paths: Vec<usize> = ls
                .paths
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    is_camera(&p.path) == camera
                        && p.distance.get(k).is_some_and(|d| *d > p.tolerance)
                })
                .map(|(i, _)| i)
                .collect();
            (!paths.is_empty()).then_some(FirstWrong {
                at: k,
                frame: *f,
                original_frame: *of,
                paths,
                camera,
            })
        })
    };
    on(false).or_else(|| on(true))
}

/// A measure in a person's words: "mario's speed across",
/// "the camera's place across", "counters.time".
pub fn words(path: &str) -> String {
    let field = path.rsplit('.').next().unwrap_or(path);
    let what = match field.split('[').next().unwrap_or(field) {
        "x" | "stage_x" => "place across",
        "y" | "stage_y" => "height",
        "vx" => "speed across",
        "vx_frac" => "speed's fraction",
        "moved" => "way last moved",
        "vy" => "speed up and down",
        "fall_gravity" => "fall gravity",
        "seen_speed" => "speed as the run left it",
        "on_ground" => "standing",
        "facing" => "facing",
        "alive" => "being alive",
        crate::bodies::MOVING => "direction",
        crate::bodies::PHASE => "phase",
        _ => return path.to_owned(),
    };
    let t = track(path);
    if let Some(rest) = path.strip_prefix(&format!("{}.", crate::bodies::PREFIX)) {
        let nth = rest
            .split_once('[')
            .and_then(|(_, r)| r.split(']').next())
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(0);
        return format!("{}'s {what}", crate::bodies::name(&t, nth));
    }
    if t == "camera" {
        format!("the camera's {what}")
    } else {
        let mut c = t.chars();
        let name: String = c
            .next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default();
        format!("{name}'s {what}")
    }
}

#[cfg(test)]
mod track_tests {
    use super::*;

    #[test]
    fn the_camera_is_its_own_track() {
        assert!(is_camera("cameras.main.x"));
        assert!(!is_camera("entities.mario[0].x"));
        assert_eq!(track("entities.mario[0].vx"), "mario");
        assert_eq!(track("entities.slug[1].x"), "slug");
        assert_eq!(track("cameras.main.x"), "camera");
        assert_eq!(track("counters.time"), "game");
        assert_eq!(words("entities.mario[0].vx"), "Mario's speed across");
        assert_eq!(words("cameras.main.x"), "the camera's place across");
        assert_eq!(track("bodies.piranha_plant[0].phase"), "piranha_plant");
        assert_eq!(
            words("bodies.goomba[1].x"),
            "the second goomba's place across"
        );
        assert_eq!(
            words("bodies.piranha_plant[0].y"),
            "the piranha plant's height"
        );
        assert_eq!(
            words("bodies.koopa_red[0].moving"),
            "the koopa red's direction"
        );
    }

    /// A run whose frames agree where `ok` says, in play where `play`
    /// says, one measure with tolerance 0.
    fn run(ok: &str, play: &str) -> Lockstep {
        let n = ok.len();
        Lockstep {
            mode: Mode::Resync,
            original_frames: n as u64,
            model_frames: n as u64,
            before_start: 0,
            lag_frames: 0,
            start: None,
            frames: (0..n as u64).map(|i| (i, i)).collect(),
            paths: vec![PathSeries {
                path: "entities.player[0].x".into(),
                tolerance: Fx::ZERO,
                distance: ok
                    .chars()
                    .map(|c| if c == '.' { Fx::ZERO } else { Fx::ONE })
                    .collect(),
                within: 0,
                max: Fx::ONE,
                first_split: None,
            }],
            splits: ok
                .chars()
                .enumerate()
                .filter(|(_, c)| *c == 'x')
                .map(|(i, _)| Split {
                    at: i,
                    frame: i as u64,
                    original_frame: i as u64,
                    paths: vec![SplitPath {
                        path: "entities.player[0].x".into(),
                        original: Value::Int(0),
                        model: Value::Int(1),
                        distance: Fx::ONE,
                    }],
                })
                .collect(),
            resync_notes: Vec::new(),
            hashes: Vec::new(),
            values: Vec::new(),
            unmeasured: Vec::new(),
            levels: Vec::new(),
            events: Vec::new(),
            screen: None,
            bodies: Vec::new(),
            scene: play
                .chars()
                .map(|c| (c != '-', Some(u16::from(c == 'b'))))
                .collect(),
            scope_end: None,
            out_of_scope: 0,
            sound: Vec::new(),
        }
    }

    #[test]
    fn the_pairing_gives_the_offset_and_every_lag_frame() {
        assert_eq!(pairing(&[]), None);
        // Model 0 is the original's 3; the original's 6 and 9 to 10 are
        // lag frames the model skipped.
        let frames = [(0, 3), (1, 4), (2, 5), (3, 7), (4, 8), (5, 11), (6, 12)];
        let p = pairing(&frames).unwrap();
        assert_eq!((p.model, p.original, p.offset), (0, 3, 3));
        assert_eq!(p.lag, vec![6, 9, 10]);
        assert_eq!(p.last_offset, 6);
        assert_eq!(
            p.says(),
            "model frame n is the original's n+3 (model 0 = original 3); 3 lag frames skipped: 6, 9-10; after them, n+6"
        );
        let none = pairing(&[(10, 10), (11, 11)]).unwrap();
        assert_eq!(
            none.says(),
            "model frame n is the original's n (model 10 = original 10); no lag frames skipped"
        );
    }

    #[test]
    fn stretches_are_ended_by_split_events_and_frames_out_of_play() {
        // Three together, two split frames (one event), two together, a
        // split frame out of play, four together and a split, then two cut
        // off by the end.
        let ls = run("...xx..x....x..", "aaaaaab-aaaabaa");
        let st = stretches(&ls);
        // The first stretch is whole; the one before the frame out of play
        // ends there and isn't counted; the four after it started after a
        // frame out of play, so they aren't whole either.
        assert_eq!(st.whole, vec![3]);
        assert_eq!(st.open_end, 2);
        // The two before the frame out of play are a stretch the scene cut
        // off, and the four after it one a split ended; the longest counts
        // both, since their frames are exact all the same.
        assert_eq!(st.cut_by_scene, 4);
        assert_eq!((st.events, st.play, st.together), (2, 14, 11));
        assert_eq!(st.longest_any(), 4);
        // A run exact from its start until play stops: no whole stretch,
        // but the longest is all of it.
        let clear = stretches(&run("......xx...", "aaaaaa--aaa"));
        assert!(clear.whole.is_empty());
        assert_eq!(clear.cut_by_scene, 6);
        assert_eq!(clear.longest_any(), 6);
        // A run that starts out of play (a title, a menu) and is exact
        // from there to its end: its longest is all of its play, though no
        // stretch is whole (dk.title.001 and dk.two-players.001 scored 0 s
        // with no split before).
        let title = stretches(&run(".........", "---aaaaaa"));
        assert!(title.whole.is_empty());
        assert_eq!((title.play, title.events), (6, 0));
        assert_eq!(title.open_end, 6);
        assert_eq!(title.longest_any(), 6);
        let (by_state, by_measure) = split_causes(&ls, &["walk".into(), "climb".into()]);
        assert_eq!(
            by_state,
            vec![("walk".to_owned(), 3), ("climb".to_owned(), 1)]
        );
        // A split frame out of play is still a split, listed by its state,
        // but no split event.
        assert_eq!(by_measure, vec![("entities.player[0].x".to_owned(), 4)]);
    }

    #[test]
    fn stretch_statistics_read_the_sorted_whole_stretches() {
        let st = Stretches {
            whole: (1..=20).map(|i| i * 61).collect(),
            ..Stretches::default()
        };
        // The 10th percentile of 20 is the second: floor(0.1 * 19).
        assert_eq!(st.percentile(10), 122);
        assert_eq!(st.median(), 11 * 61);
        assert_eq!(st.low_mean(), (61 + 122) / 2);
        // 61 frames is just over a second: 5 s is 301 frames.
        assert_eq!(st.at_least(5), 16);
        assert_eq!(seconds(601), "10.0");
    }
}
