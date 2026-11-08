//! Runs the original game and a model side by side on the same input and
//! compares them frame by frame through a state mapping, reporting, for
//! each mapped model path, on how many frames the two agree and where they
//! first don't. Both sides are runtimes of the runtime protocol, so the
//! original is read with RAM selectors and the model with model paths.

use gp_runtime::{Entry, InputFrame, Runtime, Value};

use crate::mapping::{Align, Field, Mapping, agrees};

/// One mapped path's agreement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldReport {
    pub path: String,
    pub compared: u64,
    pub agreed: u64,
    pub first: Option<Disagreement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disagreement {
    /// The model's frame.
    pub frame: u64,
    /// The original's frame it was compared with.
    pub original_frame: u64,
    pub original: Value,
    pub model: Value,
}

/// One compared frame: the model's frame, the original's, and every mapped
/// path's values on both sides, in the mapping's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub frame: u64,
    pub original_frame: u64,
    pub original: Vec<Value>,
    pub model: Vec<Value>,
    /// The paths a `[[field.case]]` leaves unmeasured on this frame, in
    /// the same order: counted as agreeing whatever the values are.
    pub unmeasured: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Original frames run.
    pub original_frames: u64,
    /// Model frames run.
    pub model_frames: u64,
    /// The original's frames before the model's power-on state.
    pub before_start: u64,
    /// Lag frames after the start, which the model doesn't run.
    pub lag_frames: u64,
    /// With the mapping's `[scope]`, the original's first frame out of
    /// the template's scope: the run stopped there, uncompared, as a
    /// lockstep does.
    pub scope_end: Option<u64>,
    /// With [`Hooks::start`], the model's canonical state once it took
    /// its start.
    pub start_state: Option<Vec<u8>>,
    pub fields: Vec<FieldReport>,
    pub rows: Vec<Row>,
}

impl Report {
    pub fn agrees(&self) -> bool {
        self.fields.iter().all(|f| f.first.is_none())
    }
}

/// Reads each field's value from the original.
fn read_original<O: Runtime>(
    original: &O,
    mapping: &Mapping,
) -> Result<(Vec<Value>, Vec<bool>), String> {
    Ok((
        crate::translate::read_original(original, mapping)?,
        crate::translate::unmeasured(original, mapping)?,
    ))
}

fn read_model<M: Runtime>(model: &M, mapping: &Mapping) -> Result<Vec<Value>, String> {
    mapping
        .fields
        .iter()
        .map(|f| match model.query(&f.path)?.as_slice() {
            [Entry { value, .. }] => Ok(value.clone()),
            other => Err(format!(
                "{} names {} values in the model's state, not one",
                f.path,
                other.len()
            )),
        })
        .collect()
}

fn polled(events: &[gp_runtime::Event]) -> bool {
    events
        .iter()
        .filter(|e| e.kind == crate::runtime::FRAME_KIND)
        .flat_map(|e| &e.data)
        .any(|d| d.path == "polled" && d.value == Value::Flag(true))
}

/// The hook that starts the model at the first compared frame.
type StartHook<'a, O, M> = &'a mut dyn FnMut(&O, &mut M) -> Result<(), String>;

/// What a caller adds to [`compare_with`].
pub struct Hooks<'a, O, M> {
    /// Run once at the first compared frame, before it is read, with the
    /// original standing at that frame: the model takes its start there,
    /// as a clip's score loads the original's state translated
    /// ([`crate::lockstep::load`]).
    pub start: Option<StartHook<'a, O, M>>,
    /// Run on each compared frame once it is read, with the model at it.
    pub each: Option<&'a mut dyn FnMut(&M)>,
}

impl<O, M> Default for Hooks<'_, O, M> {
    fn default() -> Self {
        Self {
            start: None,
            each: None,
        }
    }
}

/// Runs both from power-on through `inputs` and compares them at every
/// aligned frame.
pub fn compare<O: Runtime, M: Runtime>(
    original: &mut O,
    model: &mut M,
    mapping: &Mapping,
    inputs: &[InputFrame],
) -> Result<Report, String> {
    compare_with(original, model, mapping, inputs, Hooks::default())
}

/// [`compare`], with the caller's [`Hooks`]. A run stops at the first
/// frame out of the mapping's `[scope]`, as a lockstep does.
pub fn compare_with<O: Runtime, M: Runtime>(
    original: &mut O,
    model: &mut M,
    mapping: &Mapping,
    inputs: &[InputFrame],
    mut hooks: Hooks<'_, O, M>,
) -> Result<Report, String> {
    let scope = crate::lockstep::ScopeCheck::of(mapping)?;
    // Presentation fields are loaded by a translation, never compared.
    let measured = mapping.measured();
    let mapping = &measured;
    let Align::Polled = mapping.align;
    mapping.read_original_at(original)?;
    let mut report = Report {
        original_frames: 0,
        model_frames: 0,
        before_start: 0,
        lag_frames: 0,
        scope_end: None,
        start_state: None,
        fields: mapping
            .fields
            .iter()
            .map(|f| FieldReport {
                path: f.path.clone(),
                compared: 0,
                agreed: 0,
                first: None,
            })
            .collect(),
        rows: Vec::new(),
    };
    let scope_base: std::cell::RefCell<Option<Vec<i64>>> = std::cell::RefCell::new(None);
    let mut check = |report: &mut Report,
                     of: u64,
                     at: &O,
                     orig: Option<(Vec<Value>, Vec<bool>)>,
                     model: &mut M| {
        if report.rows.is_empty() {
            if let Some(start) = hooks.start.as_deref_mut() {
                start(at, model)?;
                report.start_state = Some(model.canonical_state());
            }
            if let Some(sc) = &scope {
                *scope_base.borrow_mut() = Some(sc.values(at)?);
            }
        }
        let (orig, quiet) = match orig {
            Some(o) => o,
            None => read_original(at, mapping)?,
        };
        let mine = read_model(model, mapping)?;
        for ((((f, o), m), field), q) in report
            .fields
            .iter_mut()
            .zip(&orig)
            .zip(&mine)
            .zip(&mapping.fields)
            .zip(&quiet)
        {
            f.compared += 1;
            if *q || agrees(field, o, m) {
                f.agreed += 1;
            } else if f.first.is_none() {
                f.first = Some(Disagreement {
                    frame: model.frame(),
                    original_frame: of,
                    original: o.clone(),
                    model: m.clone(),
                });
            }
        }
        report.rows.push(Row {
            frame: model.frame(),
            original_frame: of,
            original: orig,
            model: mine,
            unmeasured: quiet,
        });
        if let Some(each) = hooks.each.as_deref_mut() {
            each(model);
        }
        Ok::<_, String>(())
    };
    let start = mapping
        .start
        .as_deref()
        .map(gp_runtime::select::Condition::parse)
        .transpose()
        .map_err(|e| format!("the mapping's start: {e}"))?;
    let holds = |original: &O, cond: &gp_runtime::select::Condition| {
        cond.eval(&mut |sel| {
            original
                .query(&sel.to_string())?
                .first()
                .and_then(|e| e.value.as_int())
                .ok_or(format!("{sel} names no number"))
        })
    };
    // Without a start condition the model's power-on state stands for the
    // frame before the first polled one: the original as it stood then.
    let mut before = original.fork();
    let mut started = false;
    if let Some(cond) = &start
        && holds(original, cond)?
    {
        started = true;
        check(&mut report, original.frame(), &*original, None, model)?;
    }
    for input in inputs {
        if start.is_none() && !started {
            before = original.fork();
        }
        let events = original
            .step(std::slice::from_ref(input))
            .map_err(|e| format!("the original: {e}"))?;
        report.original_frames += 1;
        // Out of the template's scope: the run stops on this frame.
        if let (Some(sc), Some(base)) = (&scope, scope_base.borrow().as_ref())
            && !sc.holds(original, base)?
        {
            report.scope_end = Some(original.frame());
            break;
        }
        if let Some(cond) = &start
            && !started
        {
            if holds(original, cond)? {
                started = true;
                check(&mut report, original.frame(), &*original, None, model)?;
            } else {
                report.before_start += 1;
            }
            continue;
        }
        if !polled(&events) {
            if started {
                report.lag_frames += 1;
            } else {
                report.before_start += 1;
            }
            continue;
        }
        if !started {
            started = true;
            check(&mut report, original.frame() - 1, &before, None, model)?;
        }
        model
            .step(std::slice::from_ref(input))
            .map_err(|e| format!("the model: {e}"))?;
        report.model_frames += 1;
        check(&mut report, original.frame(), &*original, None, model)?;
    }
    Ok(report)
}

/// One entity's agreement over every slot life that held it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotReport {
    pub entity: String,
    pub lives: u64,
    /// Frames its bodies stepped.
    pub frames: u64,
    pub fields: Vec<SlotField>,
    /// Whether a body's hook drew a random byte: its draws are its own,
    /// not the original's, so its choices can differ from the slot's.
    pub drew: bool,
}

/// One compared value of an entity, with what both sides measured of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotField {
    pub path: String,
    pub compared: u64,
    /// Frames within a pixel (or, for an `ext` word, equal).
    pub agreed: u64,
    /// The largest difference, raw fixed-point.
    pub max_error: i64,
    pub original: Measure,
    pub model: Measure,
}

/// What a side's values did over the lives' stepped frames.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Measure {
    /// The sum of each step's change, raw fixed-point, unsigned: divided by
    /// `steps` it's the speed.
    pub travel: i64,
    pub steps: u64,
    /// How often the value turned back, and the frames from each life's
    /// first turn to its last: their quotient is half the period.
    pub turns: u64,
    pub turn_frames: u64,
}

impl Measure {
    /// Mean change a frame, in pixels (an `ext` word's, in its units).
    pub fn speed(&self) -> Option<gp_sim::Fx> {
        (self.steps > 0).then(|| gp_sim::Fx::from_raw(self.travel / self.steps as i64))
    }

    /// Frames from one turn to the one after the next: a full swing.
    pub fn period(&self) -> Option<gp_sim::Fx> {
        (self.turns > 0)
            .then(|| gp_sim::Fx::from_raw(((self.turn_frames as i64) << 17) / self.turns as i64))
    }
}

/// One measured number of an entity's value on both sides, as a contract
/// tolerance reads it: `speed` or `period`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotMeasure {
    pub entity: String,
    /// The compared value: `x`, `y` or an `ext` word.
    pub field: String,
    /// `speed` or `period`.
    pub what: &'static str,
    pub original: gp_sim::Fx,
    pub model: gp_sim::Fx,
}

impl SlotMeasure {
    /// How far the model's is from the original's, in hundredths of a
    /// percent of the original's; `None` when the original's is zero.
    pub fn off_bp(&self) -> Option<i64> {
        let (a, b) = (self.original.raw(), self.model.raw());
        (a != 0).then(|| ((i128::from(b - a).abs() * 10_000) / i128::from(a).abs()) as i64)
    }
}

impl SlotReport {
    /// Every speed and period both sides measured.
    pub fn measures(&self) -> Vec<SlotMeasure> {
        let mut out = Vec::new();
        for f in &self.fields {
            let pairs = [
                ("speed", f.original.speed(), f.model.speed()),
                ("period", f.original.period(), f.model.period()),
            ];
            for (what, a, b) in pairs {
                if let (Some(original), Some(model)) = (a, b) {
                    out.push(SlotMeasure {
                        entity: self.entity.clone(),
                        field: f.path.clone(),
                        what,
                        original,
                        model,
                    });
                }
            }
        }
        out
    }
}

/// One compared value of one life on one frame, for a caller that wants
/// to see them.
pub struct SlotRow<'a> {
    pub entity: &'a str,
    /// Which of the entity's lives, from 1 in the order they start.
    pub life: u64,
    /// The life's frame: its low byte is the original's clock.
    pub frame: u64,
    pub field: &'a str,
    pub original: gp_sim::Fx,
    pub model: gp_sim::Fx,
    /// The body's `ext` words.
    pub words: [i64; 4],
}

/// A life's running state for one side and one value.
#[derive(Default)]
struct Track {
    last: Option<i64>,
    sign: i64,
    first_turn: Option<u64>,
    last_turn: u64,
    turns: u64,
}

impl Track {
    fn push(&mut self, at: u64, v: i64, m: &mut Measure) {
        if let Some(last) = self.last {
            let d = v - last;
            // A jump of more than half the screen is a wrap or a move, not
            // motion.
            if d.abs() > 128 << 16 {
                self.last = Some(v);
                return;
            }
            m.travel += d.abs();
            m.steps += 1;
            if d != 0 {
                let s = d.signum();
                if self.sign != 0 && s != self.sign {
                    if self.first_turn.is_none() {
                        self.first_turn = Some(at);
                    } else {
                        self.turns += 1;
                    }
                    self.last_turn = at;
                }
                self.sign = s;
            }
        }
        self.last = Some(v);
    }

    fn end(&mut self, m: &mut Measure) {
        if let Some(first) = self.first_turn
            && self.turns > 0
        {
            m.turns += self.turns;
            m.turn_frames += self.last_turn - first;
        }
        *self = Track::default();
    }
}

/// A slot life: the slot's entity and the world its body steps in.
struct Life {
    id: u8,
    /// The entity's life number ([`SlotRow::life`]).
    n: u64,
    entry: usize,
    world: gp_sim::World,
    tracks: Vec<[Track; 2]>,
}

fn raw_of(v: &Value) -> i64 {
    match v {
        Value::Fixed(r) => *r,
        Value::Int(n) => *n << 16,
        _ => 0,
    }
}

/// A body's value by slot path, raw fixed-point (an `ext` word as a whole
/// number).
fn body_raw(b: &gp_sim::world::Body, path: &str) -> i64 {
    match path {
        "x" => b.x.raw(),
        "y" => b.y.raw(),
        "vx" => b.vx.raw(),
        "vy" => b.vy.raw(),
        _ => {
            let i: usize = path[4..5].parse().unwrap_or(0);
            b.ext[i] << 16
        }
    }
}

/// Runs the original through `inputs` and, for each slot life whose ID
/// the mapping's slots name, steps one body of that entity beside it and
/// compares the slot's values with the body's.
///
/// Each life gets a world of its own: the model's, in the level the
/// slots' `level` list names for the life's first frame. It holds the
/// player and the life's body. The player is placed each frame where the
/// mapping's `entities.<player>[0].x` and `.y` fields read, and the cameras
/// where its `cameras.<name>.x` and `.y` read. The body starts at the
/// slot's first `x` and `y` and steps (its components, then its hook) once
/// each time the slots' clock advances while `moving` holds, with the clock as
/// its frame, so a hook that reads the frame counter reads the original's.
/// The body steps as the enemies system steps it: its components (`walk`,
/// `fly`, `path`, `fall`, `hop`), then its hook. Bodies the hook spawns
/// step too, but aren't compared. That pairs every life with exactly one
/// body, whatever the model's own spawning would do. The body is alone
/// with the player, so it meets no other enemy and doesn't turn where the
/// original's meets one.
pub fn compare_slots<O: Runtime>(
    original: &mut O,
    mapping: &Mapping,
    inputs: &[InputFrame],
    spec: &gp_sim::Spec,
    exts: &gp_sim::Exts,
    trace: &mut dyn FnMut(&SlotRow<'_>),
) -> Result<Vec<SlotReport>, String> {
    let slots = mapping.slots.as_ref().ok_or("the mapping has no slots")?;
    let kind_index = |name: &str| {
        spec.kinds.index_of(name).ok_or(format!(
            "the mapping's slots name `{name}`, which the model hasn't"
        ))
    };
    // A refused or left-out kind (schema 15) has no body to step: its
    // index is never used, since no life takes it.
    let mut kinds = Vec::new();
    for k in &slots.kinds {
        kinds.push(if k.bodied() {
            kind_index(&k.entity)?
        } else {
            0
        });
    }
    // The player, from the mapping's own fields.
    let player = mapping.fields.iter().find_map(|f| {
        let rest = f.path.strip_prefix("entities.")?;
        let (name, field) = rest.split_once("[0].")?;
        (field == "x").then(|| name.to_string())
    });
    let player = player.map(|n| kind_index(&n)).transpose()?;
    // The cameras, from the mapping's own fields too: (camera, x?).
    let camera_fields: Vec<(&Field, usize, bool)> = mapping
        .fields
        .iter()
        .filter_map(|f| {
            let rest = f.path.strip_prefix("cameras.")?;
            let (name, axis) = rest.split_once('.')?;
            let c = spec.cameras.index_of(name)?;
            match axis {
                "x" => Some((f, c, true)),
                "y" => Some((f, c, false)),
                _ => None,
            }
        })
        .collect();
    // The player's x, y and vx: 0, 1 and 2.
    let player_fields: Vec<(&Field, u8)> = mapping
        .fields
        .iter()
        .filter_map(|f| {
            let p = player.map(|k| format!("entities.{}[0].", spec.kinds[k].name))?;
            match f.path.strip_prefix(&p)? {
                "x" => Some((f, 0)),
                "y" => Some((f, 1)),
                "vx" => Some((f, 2)),
                _ => None,
            }
        })
        .collect();
    let mut reports: Vec<SlotReport> = slots
        .kinds
        .iter()
        .map(|k| SlotReport {
            entity: k.entity.clone(),
            lives: 0,
            frames: 0,
            drew: false,
            fields: slots
                .measured_of(k)
                .iter()
                .map(|f| SlotField {
                    path: f.path.clone(),
                    compared: 0,
                    agreed: 0,
                    max_error: 0,
                    original: Measure::default(),
                    model: Measure::default(),
                })
                .collect(),
        })
        .collect();
    let mut levels = Vec::new();
    for l in &slots.levels {
        let at = spec.levels.index_of(&l.level).ok_or(format!(
            "slots.level names `{}`, which the model hasn't",
            l.level
        ))?;
        let cond = gp_runtime::select::Condition::parse(&l.when)
            .map_err(|e| format!("slots.level {}: {e}", l.level))?;
        levels.push((at, cond));
    }
    let moving = slots
        .moving
        .as_deref()
        .map(gp_runtime::select::Condition::parse)
        .transpose()
        .map_err(|e| format!("slots.moving: {e}"))?;
    let vacant = slots
        .vacant
        .as_deref()
        .map(gp_runtime::select::Condition::parse)
        .transpose()
        .map_err(|e| format!("slots.vacant: {e}"))?;
    let vacant_from = slots
        .vacant_from
        .as_ref()
        .map(|v| gp_runtime::select::Condition::parse(&v.when).map(|c| (v.slot, c)))
        .transpose()
        .map_err(|e| format!("slots.vacant_from: {e}"))?;
    let byte = |o: &O, sel: &str| -> Result<u8, String> {
        o.query(sel)?
            .first()
            .and_then(|e| e.value.as_int())
            .map(|v| v as u8)
            .ok_or(format!("{sel} isn't a byte"))
    };
    let read = |o: &O, f: &Field, slot: u16| -> Result<Value, String> {
        let mut bytes = Vec::new();
        for sel in f.slot_selectors(slot) {
            bytes.push(byte(o, &sel)?);
        }
        Ok(f.slot_value(slot, &bytes))
    };
    let read_player = |o: &O| -> Result<Vec<(u8, Value)>, String> {
        player_fields
            .iter()
            .map(|(f, which)| {
                let mut bytes = Vec::new();
                for sel in f.selectors() {
                    bytes.push(byte(o, &sel)?);
                }
                Ok((*which, f.value(&bytes)))
            })
            .collect()
    };
    let read_cameras = |o: &O| -> Result<Vec<(usize, bool, Value)>, String> {
        camera_fields
            .iter()
            .map(|(f, c, is_x)| {
                let mut bytes = Vec::new();
                for sel in f.selectors() {
                    bytes.push(byte(o, &sel)?);
                }
                Ok((*c, *is_x, f.value(&bytes)))
            })
            .collect()
    };
    let pin = |w: &mut gp_sim::World, me: &[(u8, Value)], cams: &[(usize, bool, Value)]| {
        if let Some(p) = w.bodies.first_mut() {
            for (which, v) in me {
                let v = gp_sim::Fx::from_raw(raw_of(v));
                match which {
                    0 => p.x = v,
                    1 => p.y = v,
                    _ => p.vx = v,
                }
            }
        }
        for (c, is_x, v) in cams {
            if let Some(cam) = w.cameras.get_mut(*c) {
                cam[usize::from(!is_x)] = gp_sim::Fx::from_raw(raw_of(v));
            }
        }
    };
    let mut lives: Vec<Option<Life>> = (0..slots.count).map(|_| None).collect();
    // Without a clock byte (`slots.no_clock`), one that ticks every frame.
    let read_clock = |original: &O, last: u8| match slots.clock {
        Some(at) => byte(original, &format!("ram[{at:#06x}]")),
        None => Ok(last.wrapping_add(1)),
    };
    let mut clock = read_clock(original, 255)?;
    let close = |life: Life, reports: &mut Vec<SlotReport>| {
        let r = &mut reports[life.entry];
        for (field, mut t) in r.fields.iter_mut().zip(life.tracks) {
            t[0].end(&mut field.original);
            t[1].end(&mut field.model);
        }
    };
    for input in inputs {
        original
            .step(std::slice::from_ref(input))
            .map_err(|e| format!("the original: {e}"))?;
        let now = read_clock(original, clock)?;
        let ticks = now.wrapping_sub(clock);
        clock = now;
        let holds = |c: &gp_runtime::select::Condition| {
            c.eval(&mut |sel| {
                original
                    .query(&sel.to_string())?
                    .first()
                    .and_then(|e| e.value.as_int())
                    .ok_or(format!("{sel} names no number"))
            })
        };
        let moves = match &moving {
            None => true,
            Some(c) => holds(c)?,
        };
        let vacant = match &vacant {
            None => false,
            Some(c) => holds(c)?,
        };
        let free_from = match &vacant_from {
            Some((from, c)) if holds(c)? => *from,
            _ => slots.count,
        };
        let mut level = None;
        let mut level_read = false;
        let me = read_player(original)?;
        let cams = read_cameras(original)?;
        for slot in 0..slots.count {
            let flag = if vacant || slot >= free_from {
                0
            } else {
                slots.flag_of(slot, |a| byte(original, &format!("ram[{a:#06x}]")))?
            };
            let id = slots.id_of(slot, |a| byte(original, &format!("ram[{a:#06x}]")))?;
            let state = match slots.state {
                Some(at) => byte(original, &format!("ram[{:#06x}]", at + slot))?,
                None => 0,
            };
            let entry = (flag != 0)
                .then(|| {
                    slots.kinds.iter().position(|k| {
                        k.bodied()
                            && k.ids.contains(&id)
                            && slots.flag_fits(k, flag)
                            && state & k.ends.or(slots.ends).unwrap_or(0) == 0
                    })
                })
                .flatten();
            let same = matches!(
                (&lives[slot as usize], entry),
                (Some(l), Some(e)) if l.id == id && l.entry == e
            );
            if !same {
                if let Some(old) = lives[slot as usize].take() {
                    close(old, &mut reports);
                }
                let Some(entry) = entry else {
                    continue;
                };
                if !level_read {
                    level_read = true;
                    if levels.is_empty() {
                        level = Some(spec.start_level);
                    }
                    for (at, c) in &levels {
                        if holds(c)? {
                            level = Some(*at);
                            break;
                        }
                    }
                }
                let Some(level) = level else {
                    continue;
                };
                // A new life: its world, with the player and its body.
                let kind = kinds[entry];
                let k = &slots.kinds[entry];
                let fields = slots.measured_of(k);
                let mut w = gp_sim::World::power_on(spec);
                w.level = level;
                w.bodies.clear();
                w.bodies.push(gp_sim::world::Body {
                    kind: player.unwrap_or(kind),
                    alive: true,
                    ..Default::default()
                });
                pin(&mut w, &me, &cams);
                let mut b = gp_sim::world::Body {
                    kind,
                    alive: true,
                    facing_left: true,
                    state: spec.kinds[kind].machine.as_ref().map_or(0, |m| m.initial),
                    ..Default::default()
                };
                for f in &fields {
                    let v = gp_sim::Fx::from_raw(raw_of(&read(original, f, slot)?));
                    match f.path.as_str() {
                        "x" => b.x = v,
                        "y" => b.y = v,
                        _ => {}
                    }
                }
                b.home = [b.x, b.y];
                w.bodies.push(b);
                w.frame = u64::from(now);
                // One step sets the hook up (the game's init routine has
                // run by now), then the body takes the slot's values: its
                // position, so the setup's own offsets to the placement
                // aren't compared, and any `ext` words the kind maps, which
                // can start where the slot's last occupant left them.
                let drew = step_hooks(spec, &mut w, exts);
                reports[entry].drew |= drew;
                w.bodies[1].x = b.x;
                w.bodies[1].y = b.y;
                for f in &fields {
                    if let Some(i) = f
                        .path
                        .strip_prefix("ext[")
                        .and_then(|r| r.strip_suffix(']'))
                        .and_then(|i| i.parse::<usize>().ok())
                    {
                        w.bodies[1].ext[i] = raw_of(&read(original, f, slot)?) >> 16;
                    }
                }
                reports[entry].lives += 1;
                let mut life = Life {
                    id,
                    n: reports[entry].lives,
                    entry,
                    world: w,
                    tracks: fields.iter().map(|_| Default::default()).collect(),
                };
                compare_life(
                    &mut life,
                    &fields,
                    original,
                    slot,
                    &read,
                    &mut reports,
                    true,
                    trace,
                )?;
                lives[slot as usize] = Some(life);
                continue;
            }
            let life = lives[slot as usize].as_mut().unwrap();
            pin(&mut life.world, &me, &cams);
            if !moves || ticks == 0 {
                continue;
            }
            // The hook's frame has the original's clock as its low byte.
            let base = now.wrapping_sub(ticks);
            let w = &mut life.world;
            w.frame += u64::from(base.wrapping_sub(w.frame as u8));
            for _ in 0..ticks {
                w.frame += 1;
                reports[life.entry].drew |= step_hooks(spec, w, exts);
            }
            reports[life.entry].frames += u64::from(ticks);
            let fields = slots.measured_of(&slots.kinds[life.entry]);
            compare_life(
                life,
                &fields,
                original,
                slot,
                &read,
                &mut reports,
                false,
                trace,
            )?;
        }
    }
    for life in lives.into_iter().flatten() {
        close(life, &mut reports);
    }
    Ok(reports)
}

/// Steps every body but the player once, as a frame of the enemies'
/// system would: its components, then its hook. Returns whether a hook
/// drew a random byte, which an empty draws script that keeps its calls
/// records (the hook then draws from its own source, as with no script).
fn step_hooks(spec: &gp_sim::Spec, w: &mut gp_sim::World, exts: &gp_sim::Exts) -> bool {
    let n = w.bodies.len();
    let mut f = gp_sim::systems::Frame::default();
    f.out = vec![false; n];
    f.fresh_bodies = vec![false; n];
    f.draws = Some(gp_sim::DrawScript {
        calls: Some(Vec::new()),
        ..gp_sim::DrawScript::default()
    });
    for i in 1..n {
        gp_sim::systems::enemy(spec, w, &mut f, i, exts);
    }
    f.draws.and_then(|d| d.calls).is_some_and(|c| !c.is_empty())
}

#[allow(clippy::too_many_arguments)]
/// Compares a life's body with its slot on this frame and adds both sides'
/// values to their measures.
fn compare_life<O: Runtime>(
    life: &mut Life,
    fields: &[&Field],
    original: &O,
    slot: u16,
    read: &dyn Fn(&O, &Field, u16) -> Result<Value, String>,
    reports: &mut [SlotReport],
    first: bool,
    trace: &mut dyn FnMut(&SlotRow<'_>),
) -> Result<(), String> {
    let at = life.world.frame;
    let body = life.world.bodies[1];
    let r = &mut reports[life.entry];
    for ((f, out), t) in fields
        .iter()
        .zip(r.fields.iter_mut())
        .zip(life.tracks.iter_mut())
    {
        let theirs = raw_of(&read(original, f, slot)?);
        let mine = body_raw(&body, &f.path);
        t[0].push(at, theirs, &mut out.original);
        t[1].push(at, mine, &mut out.model);
        if first {
            continue;
        }
        let err = (theirs - mine).abs();
        trace(&SlotRow {
            entity: &r.entity,
            life: life.n,
            frame: at,
            field: &f.path,
            original: gp_sim::Fx::from_raw(theirs),
            model: gp_sim::Fx::from_raw(mine),
            words: body.ext,
        });
        out.compared += 1;
        let within = if f.path.starts_with("ext") {
            0
        } else {
            1 << 16
        };
        if err <= within {
            out.agreed += 1;
        }
        out.max_error = out.max_error.max(err);
    }
    Ok(())
}
