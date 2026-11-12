//! Running the original and a subject side by side, through the model's
//! state mapping (docs/formats/state-mapping.md).
//!
//! The run itself is `gp_interp::compare::compare`: it steps the original
//! and the subject on the same input, lines their frames up by the
//! mapping's alignment, and reads every mapped path on both sides. [`run`]
//! turns its rows into two [`Trace`]s in model frames, which the checks
//! measure.
//!
//! [`Polled`] is the original seen as the model sees it: a console runtime
//! that answers the mapping's model paths and runs one frame per frame the
//! game reads its controller. It lets the original stand as the subject
//! (`gp validate --runtime nes`), which checks the engine itself, and gives
//! the original's trace when there's no subject at all.

use gp_interp::compare::{Hooks, compare, compare_with};
use gp_interp::mapping::Mapping;
use gp_model::Fixed;
use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, PatchResult, Rect, Runtime, Until, Value,
};

use crate::observe::{Obs, Trace, obs_of};

/// The most frames [`Polled`] runs looking for one that reads the
/// controller.
const MAX_LAG: u32 = 10_000;

fn polled(events: &[Event]) -> bool {
    events
        .iter()
        .filter(|e| e.kind == gp_interp::runtime::FRAME_KIND)
        .flat_map(|e| &e.data)
        .any(|d| d.path == "polled" && d.value == Value::Flag(true))
}

/// A console runtime read through a state mapping, a frame at a time as
/// the model runs: [`Runtime::step`] runs it on to the next frame that
/// reads the controller, and [`Runtime::query`] answers the mapped paths.
pub struct Polled<R> {
    inner: R,
    mapping: Mapping,
    frames: u64,
}

impl<R: Runtime> Polled<R> {
    /// Powers on as the model does: with the mapping's `start`, runs
    /// `inner` to the first frame the condition holds; without one,
    /// through the frames before the first one that reads the controller.
    /// Either way with those frames' inputs, as the comparison does.
    pub fn new(mut inner: R, mapping: Mapping, inputs: &[InputFrame]) -> Result<Self, String> {
        if let Some(text) = &mapping.start {
            let cond = gp_runtime::select::Condition::parse(text)
                .map_err(|e| format!("the mapping's start: {e}"))?;
            let holds = |rt: &R| {
                cond.eval(&mut |sel| {
                    rt.query(&sel.to_string())?
                        .first()
                        .and_then(|e| e.value.as_int())
                        .ok_or(format!("{sel} names no number"))
                })
            };
            let mut inputs = inputs.iter();
            while !holds(&inner)? {
                let Some(input) = inputs.next() else { break };
                inner.step(std::slice::from_ref(input))?;
            }
        } else {
            for input in inputs {
                let mut ahead = inner.fork();
                if polled(&ahead.step(std::slice::from_ref(input))?) {
                    break;
                }
                inner = ahead;
            }
        }
        Ok(Self {
            inner,
            mapping,
            frames: 0,
        })
    }

    fn read(&self, path: &str) -> Result<Option<Value>, String> {
        let Some(f) = self.mapping.fields.iter().find(|f| f.path == path) else {
            return Ok(None);
        };
        let (v, _) = f.read(&mut |sel| {
            self.inner
                .query(sel)?
                .first()
                .and_then(|e| e.value.as_int())
                .ok_or(format!("{sel} isn't a byte"))
        })?;
        Ok(Some(v))
    }
}

impl<R: Runtime> Runtime for Polled<R> {
    fn capabilities(&self) -> Capabilities {
        Capabilities::FORK
    }

    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
        let mut out = Vec::new();
        for input in inputs {
            let mut n = 0;
            loop {
                let events = self.inner.step(std::slice::from_ref(input))?;
                let done = polled(&events);
                out.extend(events);
                if done {
                    break;
                }
                n += 1;
                if n > MAX_LAG {
                    return Err(format!(
                        "the original didn't read its controller in {MAX_LAG} frames"
                    ));
                }
            }
            self.frames += 1;
        }
        Ok(out)
    }

    fn run_until(&mut self, _: &str, _: &[InputFrame], _: u32) -> Result<Until, String> {
        Err("run_until isn't available through a state mapping".into())
    }

    fn frame(&self) -> u64 {
        self.frames
    }

    fn snapshot(&self) -> Vec<u8> {
        self.inner.snapshot()
    }

    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        self.inner.restore(snap)
    }

    fn load_state(&mut self, state: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
        self.inner.load_state(state)
    }

    fn fork(&self) -> Self {
        Self {
            inner: self.inner.fork(),
            mapping: self.mapping.clone(),
            frames: self.frames,
        }
    }

    fn canonical_state(&self) -> Vec<u8> {
        self.inner.canonical_state()
    }

    fn state_hash(&self) -> u64 {
        self.inner.state_hash()
    }

    fn query(&self, selector: &str) -> Result<Vec<Entry>, String> {
        match self.read(selector)? {
            Some(value) => Ok(vec![Entry {
                path: selector.to_owned(),
                value,
            }]),
            None => Err(format!("{selector} isn't in the state mapping")),
        }
    }

    fn sample(
        &mut self,
        _: &[String],
        _: &[InputFrame],
        _: u32,
    ) -> Result<Vec<Vec<Entry>>, String> {
        Err("sample isn't available through a state mapping".into())
    }

    fn entities_at(&self, _: Rect) -> Vec<u64> {
        Vec::new()
    }

    fn patch(&mut self, _: &str) -> PatchResult {
        PatchResult::Rejected("the original can't be patched".into())
    }

    fn set(&mut self, p: &str, _: Value) -> Result<(), String> {
        Err(format!("{p} can't be set on the original"))
    }

    fn explain(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

/// What the validation engine reads from the original beyond the state
/// mapping: the current level, and entities kept in slots whose kind
/// changes (`observe.toml` in the model directory, docs/validation.md).
/// The state mapping maps fixed model paths; a slot holds a goomba one
/// moment and a koopa the next, so it can't say these.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveMap {
    pub schema: u32,
    pub model: String,
    #[serde(default)]
    pub level: Option<LevelMap>,
    #[serde(default)]
    pub slots: Vec<SlotMap>,
    /// A condition over the original's state (the runtime protocol's
    /// grammar) that holds while the player controls the game: outside it
    /// the game moves the player itself (Super Mario Bros.' pipe intros and
    /// castle walk), and the input doesn't drive it. Read as the path
    /// [`crate::profile::CONTROL_PATH`], 1 or 0.
    #[serde(default)]
    pub control: Option<String>,
    #[serde(skip)]
    pub text: String,
}

/// The level, named by the values of a few bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelMap {
    pub ram: Vec<u16>,
    /// The bytes' values, in `ram`'s order and separated by spaces, to
    /// level names: `"0x00 0x00 0x25"`. `*` matches any value; the first
    /// key that matches wins.
    pub names: Vec<(String, String)>,
    /// A condition over the original's state (the runtime protocol's
    /// condition grammar) outside which there's no level, as between
    /// levels, where the bytes can name the one before.
    #[serde(default)]
    pub when: Option<String>,
}

/// Whether a key such as `"0x00 * 0x25"` matches these bytes.
fn key_matches(key: &str, bytes: &[u8]) -> Result<bool, String> {
    let parts: Vec<&str> = key.split_whitespace().collect();
    if parts.len() != bytes.len() {
        return Err(format!("`{key}` doesn't give {} values", bytes.len()));
    }
    for (p, b) in parts.iter().zip(bytes) {
        if *p != "*" && byte_key(p)? != *b {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A table of entity slots: slot `i` reads each address plus `i`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotMap {
    pub count: u16,
    /// The byte that says which entity a slot holds.
    pub kind: u16,
    /// Nonzero while the slot is in use.
    pub active: u16,
    /// The position's whole part, low byte first.
    pub x: Vec<u16>,
    /// Kind values, written `"0x06"`, to entity names. A slot holding
    /// another kind is ignored.
    pub names: std::collections::BTreeMap<String, String>,
}

/// The file a model directory keeps its observation map in.
pub const OBSERVE_FILE: &str = "observe.toml";

fn byte_key(text: &str) -> Result<u8, String> {
    let t = text.trim();
    let n = match t.strip_prefix("0x") {
        Some(h) => u8::from_str_radix(h, 16),
        None => t.parse(),
    };
    n.map_err(|_| format!("`{text}` isn't a byte value"))
}

fn names(m: &std::collections::BTreeMap<String, String>) -> Result<Vec<(u8, String)>, String> {
    m.iter()
        .map(|(k, v)| Ok((byte_key(k)?, v.clone())))
        .collect()
}

impl ObserveMap {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut m: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if m.schema != 0 {
            return Err(format!(
                "observation map schema {}; this build reads 0",
                m.schema
            ));
        }
        if let Some(l) = &m.level {
            if let Some(w) = &l.when {
                gp_runtime::select::Condition::parse(w).map_err(|e| format!("level.when: {e}"))?;
            }
            for (k, _) in &l.names {
                key_matches(k, &vec![0; l.ram.len()])?;
            }
        }
        if let Some(c) = &m.control {
            gp_runtime::select::Condition::parse(c).map_err(|e| format!("control: {e}"))?;
        }
        for s in &m.slots {
            names(&s.names)?;
            if s.x.is_empty() || s.x.len() > 4 {
                return Err("a slot's x is 1 to 4 bytes".into());
            }
        }
        m.text = text.to_owned();
        Ok(m)
    }

    /// Reads the state mapping's `[slots]` as a slot table, when this map
    /// has none of its own: the mapping's slot kinds name the entities, and
    /// its `x` field (whole part) places them. So a game's slots are said
    /// once, in the mapping, for both the slot checks and the level graph.
    pub fn take_slots(&mut self, slots: &gp_interp::mapping::Slots) {
        if !self.slots.is_empty() {
            return;
        }
        let Some(x) = slots.fields.iter().find(|f| f.path == "x") else {
            return;
        };
        // These checks read IDs and places one byte a slot apart.
        let (Some(kind), None, None, None) = (slots.id, x.stride, slots.active_stride, slots.free)
        else {
            return;
        };
        let mut names = std::collections::BTreeMap::new();
        for k in &slots.kinds {
            for id in &k.ids {
                names.insert(format!("{id:#04x}"), k.entity.clone());
            }
        }
        self.slots.push(SlotMap {
            count: slots.count,
            kind,
            active: slots.active,
            x: x.ram.clone(),
            names,
        });
    }

    /// The level and the slots' entities in the original's state now.
    fn read<R: Runtime>(&self, rt: &R) -> Result<Extra, String> {
        let byte = |addr: u16| -> Result<u8, String> {
            let sel = format!("ram[{addr:#06x}]");
            rt.query(&sel)?
                .first()
                .and_then(|e| e.value.as_int())
                .map(|v| v as u8)
                .ok_or(format!("{sel} isn't a byte"))
        };
        let mut out = Extra::new();
        let holds = |text: &str| -> Result<bool, String> {
            let cond = gp_runtime::select::Condition::parse(text)?;
            cond.eval(&mut |sel| {
                rt.query(&sel.to_string())?
                    .first()
                    .and_then(|e| e.value.as_int())
                    .ok_or(format!("{sel} names no number"))
            })
        };
        if let Some(l) = &self.level
            && l.when.as_deref().map_or(Ok(true), holds)?
        {
            let bytes = l
                .ram
                .iter()
                .map(|a| byte(*a))
                .collect::<Result<Vec<u8>, _>>()?;
            let mut name = None;
            for (k, n) in &l.names {
                if key_matches(k, &bytes)? {
                    name = Some(n.clone());
                    break;
                }
            }
            // Bytes no key names, as while an area loads, are no level.
            if let Some(name) = name {
                out.insert(crate::profile::LEVEL_PATH.to_owned(), Obs::Text(name));
            }
        }
        if let Some(c) = &self.control {
            let on = holds(c)?;
            out.insert(
                crate::profile::CONTROL_PATH.to_owned(),
                Obs::Num(Fixed::from_raw(i64::from(on) << 16)),
            );
        }
        for s in &self.slots {
            let kinds = names(&s.names)?;
            for i in 0..s.count {
                let kind = byte(s.kind + i)?;
                let active = byte(s.active + i)? != 0;
                let mut x: i64 = 0;
                for (k, a) in s.x.iter().enumerate() {
                    x |= i64::from(byte(a + i)?) << (8 * k);
                }
                // Every kind gets the slot's paths, so a kind the slot
                // doesn't hold reads as not alive there.
                let mut seen = std::collections::BTreeSet::new();
                for (_, name) in &kinds {
                    // Several kind values may name one entity.
                    if !seen.insert(name) {
                        continue;
                    }
                    let here = active && kinds.iter().any(|(k, n)| n == name && *k == kind);
                    out.insert(
                        format!("entities.{name}[{i}].alive"),
                        Obs::Num(Fixed::from_raw(i64::from(here) << 16)),
                    );
                    if here {
                        out.insert(
                            format!("entities.{name}[{i}].x"),
                            Obs::Num(Fixed::from_raw(x << 16)),
                        );
                    }
                }
            }
        }
        Ok(out)
    }
}

/// Values beyond the mapping's, by path, on one frame.
type Extra = std::collections::BTreeMap<String, Obs>;

/// The path of how many of the original's level cells hold the byte the
/// mapping's `[cells]` gives a kind (`cells.becomes.solid`).
pub fn cells_path(kind: gp_model::model::TileKind) -> String {
    let name = serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    format!("cells.becomes.{name}")
}

/// How many of the level's cells hold each byte a cell becomes (the
/// mapping's `becomes`, or 0 for `empty`), so a coin taken or a block used
/// shows in the original as one more.
fn read_cells<R: Runtime>(rt: &R, c: &gp_interp::mapping::Cells) -> Result<Extra, String> {
    let size = u16::try_from((c.page_columns * c.rows).max(0)).unwrap_or(0);
    let mut bytes = Vec::new();
    for &p in &c.pages {
        for a in p..p.saturating_add(size) {
            let sel = format!("ram[{a:#06x}]");
            let v = rt
                .query(&sel)?
                .first()
                .and_then(|e| e.value.as_int())
                .ok_or(format!("{sel} isn't a byte"))?;
            bytes.push(v as u8);
        }
    }
    let becomes: Vec<(u8, gp_model::model::TileKind)> = if c.becomes.is_empty() {
        vec![(0, gp_model::model::TileKind::Empty)]
    } else {
        c.becomes.clone()
    };
    Ok(becomes
        .into_iter()
        .map(|(b, k)| {
            let n = bytes.iter().filter(|&&x| x == b).count() as i64;
            (cells_path(k), Obs::Num(Fixed::from_raw(n << 16)))
        })
        .collect())
}

/// The parts of an entity's state the checks read.
const ENTITY_FIELDS: &[&str] = &["x", "y", "vx", "vy", "on_ground", "alive"];

/// What a subject that answers model paths says of the level and its
/// entities now. An entity waiting to spawn isn't alive yet.
fn read_subject<S: Runtime>(rt: &S, mapped: &[String]) -> Extra {
    let mut out = Extra::new();
    if let Ok(entries) = rt.query(crate::profile::LEVEL_PATH)
        && let [e] = entries.as_slice()
        && let Some(o) = obs_of(&e.value)
    {
        out.insert(e.path.clone(), o);
    }
    let Ok(entries) = rt.query("entities") else {
        return out;
    };
    let waiting: std::collections::BTreeSet<String> = entries
        .iter()
        .filter(|e| e.path.ends_with(".waiting") && e.value == Value::Flag(true))
        .map(|e| e.path.trim_end_matches(".waiting").to_owned())
        .collect();
    for e in &entries {
        let Some((owner, field)) = e.path.rsplit_once('.') else {
            continue;
        };
        if !ENTITY_FIELDS.contains(&field) || mapped.contains(&e.path) {
            continue;
        }
        let Some(mut o) = obs_of(&e.value) else {
            continue;
        };
        if field == "alive" && waiting.contains(owner) {
            o = Obs::Num(Fixed::ZERO);
        }
        out.insert(e.path.clone(), o);
    }
    out
}

/// The two runs, observed in model frames, and how the original's frames
/// lined up with them.
pub struct Runs {
    pub original: Trace,
    pub subject: Trace,
    /// The original's frames before the model's power-on state.
    pub before_start: u64,
    /// The original's lag frames after it, which the model doesn't run.
    pub lag_frames: u64,
    /// With the mapping's `[scope]`, the original's frame the run stopped
    /// at, out of the template's scope.
    pub scope_end: Option<u64>,
    /// With a subject's `start`, its canonical state once it took it.
    pub start_state: Option<Vec<u8>>,
    /// The input of each model frame after power-on, as the subject ran
    /// them, so another runtime can run the same.
    pub model_inputs: Vec<InputFrame>,
}

/// The hook that starts the model at the first compared frame.
type ModelStart<'a, O, S> = &'a mut dyn FnMut(&O, &mut S) -> Result<(), String>;

/// What the subject is.
pub enum Subject<'a, O, S> {
    /// A runtime that answers model paths, made fresh by `make`. With
    /// `start`, it takes its start at the first compared frame from the
    /// original standing there, as a clip's score does: the original's
    /// state translated through the mapping, and its random draws given.
    Model {
        make: &'a dyn Fn() -> Result<S, String>,
        start: Option<ModelStart<'a, O, S>>,
    },
    /// The original itself, through the mapping ([`Polled`]).
    Original,
}

/// One compared frame of one side: the mapped paths' values, which of
/// them a `[[field.case]]` leaves unmeasured there, and what else was read.
type Frame<'r> = (&'r [Value], &'r [bool], Extra);

/// A side's trace. A path left unmeasured on a frame reads as nothing
/// there, on both sides, as a lockstep counts it agreeing whatever the
/// values are.
fn trace_of(paths: &[String], frames: &[Frame<'_>], pads: &[u8]) -> Trace {
    let mut extra: Vec<String> = frames
        .iter()
        .flat_map(|(_, _, e)| e.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|p| !paths.contains(p))
        .collect();
    let mut all = paths.to_vec();
    all.append(&mut extra);
    let mut t = Trace::new(all.clone());
    for ((values, quiet, e), pad) in frames.iter().zip(pads) {
        let mut row: Vec<Option<Obs>> = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if quiet.get(i).copied().unwrap_or(false) {
                    None
                } else {
                    obs_of(v)
                }
            })
            .collect();
        row.extend(all[paths.len()..].iter().map(|p| e.get(p).cloned()));
        t.push(row, *pad);
    }
    t
}

/// Runs the original and the subject on `inputs` through `mapping` with
/// gp-interp's comparison, and observes both: the frames a lockstep
/// compares (from the alignment's start to the end of the mapping's
/// `[scope]`), with what it leaves unmeasured read as nothing. With an
/// observation map the original runs a second time to read what the
/// mapping doesn't say, at the same frames; the subject's entities the
/// mapping doesn't name are read as it runs.
pub fn run<O: Runtime, S: Runtime>(
    make_original: &dyn Fn() -> Result<O, String>,
    subject: Subject<'_, O, S>,
    mapping: &Mapping,
    observe: Option<&ObserveMap>,
    inputs: &[InputFrame],
) -> Result<Runs, String> {
    // The rows hold the compared fields only: presentation fields are
    // loaded, never compared.
    let mapped: Vec<String> = mapping
        .measured()
        .fields
        .iter()
        .map(|f| f.path.clone())
        .collect();
    let mut original = make_original()?;
    let mut theirs: Option<Vec<Extra>> = None;
    let report = match subject {
        Subject::Model { make, start } => {
            let mut seen: Vec<Extra> = Vec::new();
            let mut each = |m: &S| seen.push(read_subject(m, &mapped));
            let mut start = start;
            let hooks = Hooks {
                start: match start.as_mut() {
                    Some(s) => Some(&mut **s as &mut dyn FnMut(&O, &mut S) -> Result<(), String>),
                    None => None,
                },
                each: Some(&mut each),
            };
            let report = compare_with(&mut original, &mut make()?, mapping, inputs, hooks)?;
            theirs = Some(seen);
            report
        }
        Subject::Original => {
            let mut polled = Polled::new(make_original()?, mapping.clone(), inputs)?;
            compare(&mut original, &mut polled, mapping, inputs)?
        }
    };
    let input_of = |of: u64| of.checked_sub(1).and_then(|f| inputs.get(f as usize));
    let pads: Vec<u8> = report
        .rows
        .iter()
        .map(|r| input_of(r.original_frame).map_or(0, |i| i.buttons as u8))
        .collect();

    // The original again, read at the frames the rows came from.
    let mut ours: Vec<Extra> = vec![Extra::new(); report.rows.len()];
    if observe.is_some() || mapping.cells.is_some() {
        let mut rt = make_original()?;
        let mut frame = 0u64;
        for (k, row) in report.rows.iter().enumerate() {
            while frame < row.original_frame {
                let input = inputs.get(frame as usize).cloned().unwrap_or_default();
                rt.step(std::slice::from_ref(&input))?;
                frame += 1;
            }
            if let Some(map) = observe {
                ours[k] = map.read(&rt)?;
            }
            if let Some(c) = &mapping.cells {
                ours[k].extend(read_cells(&rt, c)?);
            }
        }
    }
    let theirs = theirs.unwrap_or_else(|| ours.clone());
    let side = |model: bool, extra: Vec<Extra>| {
        let frames: Vec<Frame<'_>> = report
            .rows
            .iter()
            .zip(extra)
            .map(|(r, e)| {
                let v = if model { &r.model } else { &r.original };
                (v.as_slice(), r.unmeasured.as_slice(), e)
            })
            .collect();
        trace_of(&mapped, &frames, &pads)
    };
    Ok(Runs {
        original: side(false, ours),
        subject: side(true, theirs),
        before_start: report.before_start,
        lag_frames: report.lag_frames,
        scope_end: report.scope_end,
        start_state: report.start_state.clone(),
        model_inputs: report
            .rows
            .iter()
            .skip(1)
            .map(|r| input_of(r.original_frame).cloned().unwrap_or_default())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A runtime that is only 2KB of RAM, for reading maps.
    struct Ram(Vec<u8>);

    impl Ram {
        fn query(&self, sel: &str) -> Result<Vec<Entry>, String> {
            let addr = sel
                .strip_prefix("ram[0x")
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|h| usize::from_str_radix(h, 16).ok())
                .ok_or("not a ram selector")?;
            Ok(vec![Entry {
                path: sel.to_owned(),
                value: Value::Int(i64::from(self.0[addr])),
            }])
        }
    }

    fn read(map: &ObserveMap, ram: &[u8]) -> Extra {
        // The map reads through `query` only.
        struct R<'a>(&'a Ram);
        impl Runtime for R<'_> {
            fn capabilities(&self) -> Capabilities {
                Capabilities::empty()
            }
            fn step(&mut self, _: &[InputFrame]) -> Result<Vec<Event>, String> {
                unreachable!()
            }
            fn run_until(&mut self, _: &str, _: &[InputFrame], _: u32) -> Result<Until, String> {
                unreachable!()
            }
            fn frame(&self) -> u64 {
                0
            }
            fn snapshot(&self) -> Vec<u8> {
                Vec::new()
            }
            fn restore(&mut self, _: &[u8]) -> Result<(), String> {
                unreachable!()
            }
            fn load_state(&mut self, _: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
                unreachable!()
            }
            fn fork(&self) -> Self {
                unreachable!()
            }
            fn canonical_state(&self) -> Vec<u8> {
                Vec::new()
            }
            fn state_hash(&self) -> u64 {
                0
            }
            fn query(&self, s: &str) -> Result<Vec<Entry>, String> {
                self.0.query(s)
            }
            fn sample(
                &mut self,
                _: &[String],
                _: &[InputFrame],
                _: u32,
            ) -> Result<Vec<Vec<Entry>>, String> {
                unreachable!()
            }
            fn entities_at(&self, _: Rect) -> Vec<u64> {
                Vec::new()
            }
            fn patch(&mut self, _: &str) -> PatchResult {
                unreachable!()
            }
            fn set(&mut self, _: &str, _: Value) -> Result<(), String> {
                unreachable!()
            }
            fn explain(&self, _: &str) -> Vec<String> {
                Vec::new()
            }
        }
        let ram = Ram(ram.to_vec());
        map.read(&R(&ram)).unwrap()
    }

    const MAP: &str = r#"
schema = 0
model = "m"

[level]
ram = [0x10, 0x11]
names = [["0x00 0x05", "one"], ["0x01 *", "two"]]

[[slots]]
count = 2
kind = 0x20
active = 0x30
x = [0x40, 0x50]

[slots.names]
"0x06" = "goomba"
"0x07" = "goomba"
"0x00" = "koopa"
"#;

    #[test]
    fn reads_the_level_and_slotted_entities() {
        let map = ObserveMap::parse(MAP).unwrap();
        let mut ram = vec![0u8; 0x800];
        ram[0x10] = 1;
        ram[0x11] = 9;
        ram[0x20] = 6; // slot 0: a goomba at 300
        ram[0x30] = 1;
        ram[0x40] = 0x2c;
        ram[0x50] = 1;
        ram[0x21] = 7; // slot 1: another goomba kind, not in use
        let e = read(&map, &ram);
        let text = |p: &str| e.get(p).map(ToString::to_string);
        assert_eq!(text("level").as_deref(), Some("two"));
        assert_eq!(text("entities.goomba[0].x").as_deref(), Some("300"));
        assert_eq!(text("entities.goomba[0].alive").as_deref(), Some("1"));
        assert_eq!(text("entities.koopa[0].alive").as_deref(), Some("0"));
        assert_eq!(text("entities.goomba[1].alive").as_deref(), Some("0"));
        assert_eq!(text("entities.goomba[1].x"), None);
        // Bytes no key names are no level.
        ram[0x10] = 2;
        assert_eq!(read(&map, &ram).get("level"), None);
        ram[0x10] = 0;
        ram[0x11] = 5;
        assert_eq!(text_of(&read(&map, &ram), "level"), Some("one".into()));
    }

    fn text_of(e: &Extra, p: &str) -> Option<String> {
        e.get(p).map(ToString::to_string)
    }

    #[test]
    fn refuses_maps_it_cant_read() {
        for (bad, want) in [
            (MAP.replace("\"0x00 0x05\"", "\"0x00\""), "values"),
            (MAP.replace("\"0x06\"", "\"six\""), "byte value"),
            (MAP.replace("schema = 0", "schema = 1"), "schema"),
            (MAP.replace("x = [0x40, 0x50]", "x = []"), "1 to 4"),
        ] {
            let e = ObserveMap::parse(&bad).unwrap_err();
            assert!(e.contains(want), "{e}");
        }
    }

    /// A runtime of named values, answering any selector it holds, that
    /// reads its controller on every frame.
    #[derive(Clone)]
    struct Toy(std::collections::HashMap<String, Value>, u64);

    impl Runtime for Toy {
        fn capabilities(&self) -> Capabilities {
            Capabilities::FORK
        }
        fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
            let mut out = Vec::new();
            for _ in inputs {
                self.1 += 1;
                out.push(Event {
                    frame: self.1,
                    kind: gp_interp::runtime::FRAME_KIND.into(),
                    entity: None,
                    data: vec![Entry {
                        path: "polled".into(),
                        value: Value::Flag(true),
                    }],
                });
            }
            Ok(out)
        }
        fn run_until(&mut self, _: &str, _: &[InputFrame], _: u32) -> Result<Until, String> {
            unreachable!()
        }
        fn frame(&self) -> u64 {
            self.1
        }
        fn snapshot(&self) -> Vec<u8> {
            Vec::new()
        }
        fn restore(&mut self, _: &[u8]) -> Result<(), String> {
            unreachable!()
        }
        fn load_state(&mut self, _: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
            unreachable!()
        }
        fn fork(&self) -> Self {
            self.clone()
        }
        fn canonical_state(&self) -> Vec<u8> {
            Vec::new()
        }
        fn state_hash(&self) -> u64 {
            0
        }
        fn query(&self, s: &str) -> Result<Vec<Entry>, String> {
            // $12 counts the frames, for a scope that ends.
            let value = if s == "ram[0x0012]" {
                Value::Int(self.1 as i64)
            } else {
                self.0.get(s).cloned().ok_or(format!("no {s}"))?
            };
            Ok(vec![Entry {
                path: s.to_owned(),
                value,
            }])
        }
        fn sample(
            &mut self,
            _: &[String],
            _: &[InputFrame],
            _: u32,
        ) -> Result<Vec<Vec<Entry>>, String> {
            unreachable!()
        }
        fn entities_at(&self, _: Rect) -> Vec<u64> {
            Vec::new()
        }
        fn patch(&mut self, _: &str) -> PatchResult {
            unreachable!()
        }
        fn set(&mut self, _: &str, _: Value) -> Result<(), String> {
            unreachable!()
        }
        fn explain(&self, _: &str) -> Vec<String> {
            Vec::new()
        }
    }

    /// The traces hold the frames a clip's lockstep compares: a path a
    /// case leaves unmeasured reads as nothing on both sides, the run stops
    /// at the mapping's scope, and a start hook runs once, at the first
    /// compared frame. Before, validate read Donkey Kong's unmeasured
    /// frames as values and failed on them.
    #[test]
    fn the_traces_leave_out_what_a_lockstep_does() {
        let mapping = Mapping::parse(
            r#"
schema = 22
model = "m"
source = "s"
align = "polled"

[[field]]
path = "entities.player[0].x"
ram = [0x10]

[[field.case]]
when = "ram[0x0011] == 1"
unmeasured = "x means nothing here"

[[field]]
path = "counters.c"
ram = [0x13]

[scope]
while = "ram[0x0012] < 4"
"#,
        )
        .unwrap();
        let toy = |pairs: &[(&str, i64)]| {
            Toy(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), Value::Int(*v)))
                    .collect(),
                0,
            )
        };
        let original = toy(&[("ram[0x0010]", 7), ("ram[0x0011]", 1), ("ram[0x0013]", 2)]);
        let model = toy(&[("entities.player[0].x", 9), ("counters.c", 2)]);
        let make_original = || Ok(original.clone());
        let make_model = || Ok(model.clone());
        let mut starts = Vec::new();
        let mut start = |o: &Toy, _: &mut Toy| {
            starts.push(o.frame());
            Ok(())
        };
        let inputs = vec![InputFrame::default(); 10];
        let runs = run(
            &make_original,
            Subject::Model {
                make: &make_model,
                start: Some(&mut start),
            },
            &mapping,
            None,
            &inputs,
        )
        .unwrap();
        // Frames 0 to 3 of the original, then out of scope at 4.
        assert_eq!(runs.scope_end, Some(4));
        assert_eq!(runs.original.len(), 4);
        assert_eq!(runs.subject.len(), 4);
        for f in 0..4 {
            assert_eq!(runs.original.get(f, "entities.player[0].x"), None);
            assert_eq!(runs.subject.get(f, "entities.player[0].x"), None);
            assert_eq!(runs.subject.num(f, "counters.c"), Fixed::from_int(2).ok());
        }
        assert_eq!(starts, vec![0]);
    }

    /// A presentation field is loaded, never compared, so the comparison's
    /// rows leave it out; the traces hold one value per compared path
    /// (Donkey Kong's 47 presentation fields made `gp validate` panic).
    #[test]
    fn presentation_fields_stay_out_of_the_traces() {
        let mapping = Mapping::parse(
            r#"
schema = 22
model = "m"
source = "s"
align = "polled"

[[field]]
path = "entities.player[0].x"
ram = [0x10]

[[field]]
path = "counters.sound"
ram = [0x11]
presentation = true
"#,
        )
        .unwrap();
        let toy = |pairs: &[(&str, i64)]| {
            Toy(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), Value::Int(*v)))
                    .collect(),
                0,
            )
        };
        let original = toy(&[("ram[0x0010]", 7), ("ram[0x0011]", 1)]);
        let model = toy(&[("entities.player[0].x", 7), ("counters.sound", 1)]);
        let make_original = || Ok(original.clone());
        let make_model = || Ok(model.clone());
        let inputs = vec![InputFrame::default(); 3];
        let runs = run(
            &make_original,
            Subject::Model {
                make: &make_model,
                start: None,
            },
            &mapping,
            None,
            &inputs,
        )
        .unwrap();
        assert!(runs.original.has("entities.player[0].x"));
        assert!(!runs.original.has("counters.sound"));
        assert!(runs.subject.len() > 1);
        assert_eq!(
            runs.subject.num(1, "entities.player[0].x"),
            Fixed::from_int(7).ok()
        );
    }
}
