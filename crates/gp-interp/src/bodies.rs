//! The bodies other than the player as measures of their own
//! (docs/interpreter.md, "Bodies: enemies as measures"). The lockstep pairs
//! each of the original's bodies with a model body of the same kind and
//! keeps the pairing for the run, so an enemy's place, direction and phase
//! are compared like the player's, and its turns and changes are events on
//! both sides.
//!
//! A body comes from one of two places:
//!
//! - The mapping's own fields: `entities.slug[1].x` names the second slug
//!   in both games, so the pairing is the mapping's and the measures are
//!   its fields (Moss Hop).
//! - The original's reused slots (`[slots]`): each slot life is paired
//!   with a living model body of its entity, the nearest one within a
//!   view's width, when either side appears. A start translated from the
//!   original places each slot object as a body exactly where the slot is,
//!   so on the first frame the nearest body is the one the start carried
//!   in. A body with no partner is a measure of its own: its `alive`
//!   differs while only one game has it. Its measures are named
//!   `bodies.<kind>[n]`, the run's n-th body of that kind.
//!
//! Everything here is integer arithmetic over the two games' values, in a
//! fixed order (slots by number, model bodies by ID), so the same run
//! pairs the same way every time.

use std::collections::BTreeMap;

use gp_runtime::{Runtime, Value};
use gp_sim::Spec;

use crate::lockstep::{Lockstep, PathSeries};
use crate::mapping::{Mapping, Slots};

/// The first segment of a slot body's measures: `bodies.goomba[1].x` is the
/// run's second goomba's place across.
pub const PREFIX: &str = "bodies";

/// The way a slot body last moved across: `left`, `right`, or `none`
/// before it has moved. From the slots' `vx` when they map it, else from
/// its place, frame to frame.
pub const MOVING: &str = "moving";

/// The way a slot body last moved up and down: `up`, `down`, or `none`
/// before it has moved, for a slot kind with `up_down` (schema 9), such as
/// a fireball that bounces. From its place, frame to frame.
pub const MOVING_Y: &str = "moving_y";

/// A path mover's phase (an entity with a `path` component, such as a
/// piranha plant): moving out along its path or back (`up`, `down`,
/// `left` or `right`), paused at the far end (`out`), paused back home
/// (`hidden`), or `still` before it has moved.
pub const PHASE: &str = "phase";

/// Whether the body is in its game: a living slot life, or a living model
/// body that has appeared.
pub const ALIVE: &str = "alive";

/// Jumps of more than this many pixels in a frame are wraps or a slot
/// reused, not motion.
const WRAP: i64 = 128;

/// The frames a path mover stands still before it counts as paused: a
/// mover at half a pixel a frame moves a whole pixel every other frame.
const PAUSED: u32 = 4;

/// One of the original's bodies and the model body paired with it, or a
/// body only one game has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// The model entity.
    pub kind: String,
    /// Its place among the run's bodies of its kind, from 0.
    pub nth: usize,
    /// The original's slot, for a slot body that has an original side.
    pub slot: Option<u16>,
    /// The model body's ID, for a slot body that has a model side.
    pub model: Option<u64>,
    /// The mapping's own fields name it: paired by them for the run.
    pub mapped: bool,
    /// The compared frame (an index into [`Lockstep::frames`]) it was
    /// first seen on, and the one both games had it on.
    pub from: usize,
    pub paired: Option<usize>,
    /// Its measures: indexes into [`Lockstep::paths`].
    pub paths: Vec<usize>,
    /// Its turns and changes in both games, by frame.
    pub events: Vec<Event>,
}

impl Body {
    /// "the goomba", "the second goomba".
    pub fn name(&self) -> String {
        name(&self.kind, self.nth)
    }

    /// Whether one game has it and the other never did.
    pub fn one_sided(&self) -> Option<&'static str> {
        if self.mapped || self.paired.is_some() {
            return None;
        }
        Some(if self.slot.is_some() {
            "original"
        } else {
            "model"
        })
    }
}

/// A turn or a change of one body in one game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The compared frame: an index into [`Lockstep::frames`].
    pub at: usize,
    /// The original's, or else the model's.
    pub original: bool,
    /// In words: "turns left", "rises", "hides", "becomes flat", "is gone".
    pub what: String,
}

/// An event the two games have at different frames, or that only one has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub what: String,
    /// The compared frame of each game's, if it has one.
    pub original: Option<usize>,
    pub model: Option<usize>,
}

impl Mismatch {
    /// The earlier of the two frames.
    pub fn at(&self) -> usize {
        match (self.original, self.model) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => 0,
        }
    }
}

/// A kind and its place as a person says it: "the goomba" for the first,
/// "the second goomba", "the piranha plant".
pub fn name(kind: &str, nth: usize) -> String {
    const ORD: [&str; 9] = [
        "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth",
    ];
    let kind = kind.replace('_', " ");
    match nth {
        0 => format!("the {kind}"),
        n if n <= ORD.len() => format!("the {} {kind}", ORD[n - 1]),
        n => format!("{kind} {}", n + 1),
    }
}

/// A body's events that the two games don't have on the same frame: each
/// kind of event in turn, the i-th of the original's against the i-th of
/// the model's, earliest first.
pub fn mismatches(b: &Body) -> Vec<Mismatch> {
    let mut whats: Vec<&str> = Vec::new();
    for e in &b.events {
        if !whats.contains(&e.what.as_str()) {
            whats.push(&e.what);
        }
    }
    let mut out = Vec::new();
    for w in whats {
        let side = |original: bool| -> Vec<usize> {
            b.events
                .iter()
                .filter(|e| e.what == w && e.original == original)
                .map(|e| e.at)
                .collect()
        };
        let (o, m) = (side(true), side(false));
        for i in 0..o.len().max(m.len()) {
            let (a, c) = (o.get(i).copied(), m.get(i).copied());
            if a != c {
                out.push(Mismatch {
                    what: w.to_owned(),
                    original: a,
                    model: c,
                });
            }
        }
    }
    out.sort_by(|x, y| x.at().cmp(&y.at()).then(x.what.cmp(&y.what)));
    out
}

/// One slot object of a kind the slots name, as the original holds it this
/// frame: its slot, the kind (an index into `slots.kinds`) and its values
/// by slot path, in the kind's field order.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotObject {
    pub slot: u16,
    pub kind: usize,
    pub values: Vec<(String, Value)>,
}

/// The original's RAM byte at `addr`.
pub fn read_byte<O: Runtime>(original: &O, addr: u16) -> Result<u8, String> {
    byte_at(original, &format!("ram[{addr:#06x}]"))
}

/// The original's byte a selector such as `ram[0x00a0]` names.
pub fn read_sel<O: Runtime>(original: &O, sel: &str) -> Result<u8, String> {
    byte_at(original, sel)
}

fn byte_at<O: Runtime>(original: &O, sel: &str) -> Result<u8, String> {
    original
        .query(sel)?
        .first()
        .and_then(|e| e.value.as_int())
        .map(|v| v as u8)
        .ok_or(format!("{sel} isn't a byte"))
}

/// The original's living slot objects of the kinds the slots name: a slot
/// whose flag is set with an ID a kind lists (and a flag it lists, and no
/// state bit that ends it), by slot number. Those that only hold their slot
/// (`holds_slot`) aren't among them.
pub fn slot_objects<O: Runtime>(original: &O, slots: &Slots) -> Result<Vec<SlotObject>, String> {
    Ok(carried_slot_objects(original, slots)?
        .into_iter()
        .filter(|o| !slots.kinds[o.kind].holds_slot)
        .collect())
}

/// What a start carries: [`slot_objects`] and those that only hold their
/// slot.
pub fn carried_slot_objects<O: Runtime>(
    original: &O,
    slots: &Slots,
) -> Result<Vec<SlotObject>, String> {
    let byte = |sel: String| -> Result<u8, String> { byte_at(original, &sel) };
    let mut out = Vec::new();
    let holds = |c: &str, what: &str| -> Result<bool, String> {
        let cond = gp_runtime::select::Condition::parse(c).map_err(|e| format!("{what}: {e}"))?;
        cond.eval(&mut |sel| {
            original
                .query(&sel.to_string())?
                .first()
                .and_then(|e| e.value.as_int())
                .ok_or(format!("{sel} names no number"))
        })
    };
    if let Some(c) = &slots.vacant
        && holds(c, "slots.vacant")?
    {
        return Ok(out);
    }
    // Feature `slots.vacant_from`: the slots from one on read free.
    let count = match &slots.vacant_from {
        Some(v) if holds(&v.when, "slots.vacant_from")? => v.slot,
        _ => slots.count,
    };
    for slot in 0..count {
        let flag = slots.flag_of(slot, |a| read_byte(original, a))?;
        if flag == 0 {
            continue;
        }
        let id = slots.id_of(slot, |a| read_byte(original, a))?;
        let state = match slots.state {
            Some(at) => byte(format!("ram[{:#06x}]", at + slot))?,
            None => 0,
        };
        let Some(kind) = slots.kinds.iter().position(|k| {
            k.carried()
                && k.ids.contains(&id)
                && slots.flag_fits(k, flag)
                && state & k.ends.or(slots.ends).unwrap_or(0) == 0
        }) else {
            continue;
        };
        let mut values = Vec::new();
        for f in slots.fields_of(&slots.kinds[kind]) {
            let mut bytes = Vec::new();
            for sel in f.slot_selectors(slot) {
                bytes.push(byte(sel)?);
            }
            values.push((f.path.clone(), f.slot_value(slot, &bytes)));
        }
        out.push(SlotObject { slot, kind, values });
    }
    Ok(out)
}

/// The objects past the end of their lives that still have their slots
/// (the slots' `held`, schema 11), in slot order. `kind` is 0: they have
/// no slot kind.
pub fn held_objects<O: Runtime>(original: &O, slots: &Slots) -> Result<Vec<SlotObject>, String> {
    let mut out = Vec::new();
    let (Some(held), Some(state)) = (&slots.held, slots.state) else {
        return Ok(out);
    };
    let byte = |addr: u16| read_byte(original, addr);
    for slot in 0..slots.count {
        let flag = slots.flag_of(slot, byte)?;
        if flag == 0
            || slots.is_part(flag)
            || !held.ids.contains(&slots.id_of(slot, byte)?)
            || byte(state + slot)? & held.state == 0
        {
            continue;
        }
        let mut values = Vec::new();
        for f in slots.fields_with(&held.fields) {
            let mut bytes = Vec::new();
            for sel in f.slot_selectors(slot) {
                bytes.push(byte_at(original, &sel)?);
            }
            values.push((f.path.clone(), f.slot_value(slot, &bytes)));
        }
        out.push(SlotObject {
            slot,
            kind: 0,
            values,
        });
    }
    Ok(out)
}

/// A model body of a kind the slots name, as the model's state names it.
#[derive(Debug, Clone, Default)]
struct ModelBody {
    kind: usize,
    id: u64,
    x: i64,
    y: i64,
    vx: i64,
    vy: i64,
    alive: bool,
    waiting: bool,
}

/// The model's bodies of the kinds in `wanted`, by ID.
fn model_bodies<M: Runtime>(
    model: &M,
    spec: &Spec,
    wanted: &[bool],
) -> Result<Vec<ModelBody>, String> {
    let entries = match model.query("entities") {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };
    let mut by: BTreeMap<(usize, usize), ModelBody> = BTreeMap::new();
    for e in entries {
        let Some(rest) = e.path.strip_prefix("entities.") else {
            continue;
        };
        let Some((name, rest)) = rest.split_once('[') else {
            continue;
        };
        let Some((n, field)) = rest.split_once("].") else {
            continue;
        };
        let (Some(kind), Ok(n)) = (spec.kinds.index_of(name), n.parse::<usize>()) else {
            continue;
        };
        if !wanted.get(kind).copied().unwrap_or(false) {
            continue;
        }
        let b = by.entry((kind, n)).or_insert(ModelBody {
            kind,
            ..ModelBody::default()
        });
        let num = |v: &Value| match v {
            Value::Fixed(r) => *r,
            Value::Int(n) => n << 16,
            _ => 0,
        };
        match field {
            "id" => {
                if let Value::Entity(id) = e.value {
                    b.id = id;
                }
            }
            "x" => b.x = num(&e.value),
            "y" => b.y = num(&e.value),
            "vx" => b.vx = num(&e.value),
            "vy" => b.vy = num(&e.value),
            "alive" => b.alive = e.value == Value::Flag(true),
            "waiting" => b.waiting = e.value == Value::Flag(true),
            _ => {}
        }
    }
    let mut out: Vec<ModelBody> = by.into_values().collect();
    out.sort_by_key(|b| b.id);
    Ok(out)
}

/// What one of a slot body's measures reads.
#[derive(Debug, Clone)]
enum Measure {
    /// A slot value: `x`, `y`, `vx` or `vy`, compared on whole pixels when
    /// the original keeps no fraction.
    Field {
        name: String,
        whole: bool,
    },
    Moving,
    MovingY,
    Phase,
    Alive,
}

impl Measure {
    fn name(&self) -> &str {
        match self {
            Self::Field { name, .. } => name,
            Self::Moving => MOVING,
            Self::MovingY => MOVING_Y,
            Self::Phase => PHASE,
            Self::Alive => ALIVE,
        }
    }
}

/// One game's side of a slot body as it goes.
#[derive(Debug, Clone, Default)]
struct Side {
    present: bool,
    done: bool,
    /// Its place, speed across and up, raw.
    at: Option<[i64; 4]>,
    moving: &'static str,
    moving_y: &'static str,
    /// For a path mover, whether it last moved out along its path.
    out: Option<bool>,
    /// Frames in a row it hasn't moved along its path.
    still: u32,
    phase: &'static str,
}

impl Side {
    fn start(at: [i64; 4]) -> Self {
        Self {
            present: true,
            done: false,
            at: Some(at),
            moving: "none",
            moving_y: "none",
            out: None,
            still: 0,
            phase: "still",
        }
    }
}

/// A path mover's axis and which way is out: along y, and out is up.
#[derive(Debug, Clone, Copy)]
struct PathWay {
    vertical: bool,
    negative: bool,
}

impl PathWay {
    fn word(self, out: bool) -> &'static str {
        match (self.vertical, self.negative == out) {
            (true, true) => "up",
            (true, false) => "down",
            (false, true) => "left",
            (false, false) => "right",
        }
    }
}

/// A slot body's own bookkeeping, beside its [`Body`].
#[derive(Debug, Clone)]
struct Live {
    kind: usize,
    measures: Vec<Measure>,
    /// The slots map a speed across, so the direction is its sign.
    by_speed: bool,
    path: Option<PathWay>,
    o: Side,
    m: Side,
    /// Let go of its model side by a resync that carried the original's
    /// objects: the carried body it pairs with next starts from the
    /// original's side, as the resync made it the same.
    rejoin: bool,
}

/// A mapped body's previous values on its event fields, each game's.
#[derive(Debug, Clone)]
struct MappedBody {
    body: usize,
    /// Its event fields: indexes into the mapping's fields, with the field.
    fields: Vec<(usize, String)>,
    prev: Vec<(Option<Value>, Option<Value>)>,
}

/// The pairing as a lockstep goes: which body each slot and each model ID
/// stands for, and each slot body's sides.
pub(crate) struct Tracker {
    /// The slot groups (the mapping's `[slots]` and, since schema 9, its
    /// `[[slots.group]]`), whose slots and kinds are numbered on from one
    /// group to the next.
    groups: Vec<Slots>,
    /// Each slot kind's model entity: an index into the spec's kinds.
    entity_of: Vec<Option<usize>>,
    /// Each spec kind's slot kind, if the slots name it.
    slot_kind_of: Vec<Option<usize>>,
    /// How far apart two bodies can be paired, raw.
    reach: i64,
    /// For each of `Lockstep::bodies`, its bookkeeping (`None` for a
    /// mapped body).
    live: Vec<Option<Live>>,
    in_slot: Vec<Option<usize>>,
    by_id: BTreeMap<u64, usize>,
    mapped: Vec<MappedBody>,
}

/// The mapped fields a body's events come from.
const EVENT_FIELDS: [&str; 6] = ["facing", "moved", "state", "alive", "vx", "moving"];

impl Tracker {
    /// The tracker for `mapping`, with the bodies its own fields name
    /// already in `out`.
    pub(crate) fn new(spec: &Spec, mapping: &Mapping, out: &mut Lockstep) -> Self {
        use gp_sim::spec::Role;
        let slot_groups: Vec<Slots> = mapping.slot_groups().into_iter().cloned().collect();
        let entity_of: Vec<Option<usize>> = slot_groups
            .iter()
            .flat_map(|s| &s.kinds)
            .map(|k| k.bodied().then(|| spec.kinds.index_of(&k.entity)).flatten())
            .collect();
        let mut slot_kind_of = vec![None; spec.kinds.len()];
        for (sk, e) in entity_of.iter().enumerate() {
            if let Some(e) = e
                && slot_kind_of[*e].is_none()
            {
                slot_kind_of[*e] = Some(sk);
            }
        }
        let width = spec.cameras.first().map_or(256, |c| c.size[0]);
        // The mapping's own bodies: every `entities.<kind>[i]` of a kind
        // that isn't the player's, in the order the mapping first names
        // each.
        // Each one's kind, index and fields (the mapping's index and the
        // field's name).
        type Group = (String, usize, Vec<(usize, String)>);
        let mut groups: Vec<Group> = Vec::new();
        for (i, f) in mapping.fields.iter().enumerate() {
            let Ok(p) = gp_model::Path::parse(&f.path) else {
                continue;
            };
            let seg = &p.segments;
            if seg.len() != 3 || seg[0].name != "entities" {
                continue;
            }
            let Some(gp_model::path::Index::At(n)) = seg[1].index else {
                continue;
            };
            let kind = &seg[1].name;
            // The kind's role alone, which the scorer reads whole.
            if !spec
                .kinds
                .index_of(kind)
                .and_then(|k| spec.kinds.role_of(k))
                .is_some_and(|r| r != Role::Player)
            {
                continue;
            }
            let n = usize::try_from(n).unwrap_or(0);
            let field = seg[2].name.clone();
            match groups.iter_mut().find(|g| &g.0 == kind && g.1 == n) {
                Some(g) => g.2.push((i, field)),
                None => groups.push((kind.clone(), n, vec![(i, field)])),
            }
        }
        let mut live = Vec::new();
        let mut mapped = Vec::new();
        for (kind, n, fields) in groups {
            let has_turn = fields
                .iter()
                .any(|(_, f)| matches!(f.as_str(), "facing" | "moved"));
            let events: Vec<(usize, String)> = fields
                .iter()
                .filter(|(_, f)| EVENT_FIELDS.contains(&f.as_str()) && !(has_turn && f == "vx"))
                .cloned()
                .collect();
            out.bodies.push(Body {
                kind,
                nth: n,
                slot: None,
                model: None,
                mapped: true,
                from: 0,
                paired: Some(0),
                paths: fields.iter().map(|(i, _)| *i).collect(),
                events: Vec::new(),
            });
            live.push(None);
            mapped.push(MappedBody {
                body: out.bodies.len() - 1,
                prev: vec![(None, None); events.len()],
                fields: events,
            });
        }
        let count = slot_groups.iter().map(|s| usize::from(s.count)).sum();
        Self {
            groups: slot_groups,
            entity_of,
            slot_kind_of,
            reach: width << 16,
            live,
            in_slot: vec![None; count],
            by_id: BTreeMap::new(),
            mapped,
        }
    }

    /// After a resync that carried the original's slot objects into the
    /// model (feature `slots.resync`): the model's bodies are new, so each
    /// body the original still holds lets go of its model side and pairs
    /// again, with the carried body nearest it, on the next frame. A model
    /// body the original no longer holds is gone with the carry.
    pub(crate) fn resynced(&mut self, out: &mut Lockstep) {
        for (b, l) in self.live.iter_mut().enumerate() {
            let Some(l) = l else { continue };
            if !l.o.present || l.o.done {
                continue;
            }
            if let Some(id) = out.bodies[b].model.take() {
                self.by_id.remove(&id);
            }
            l.m = Side::default();
            l.rejoin = true;
        }
    }

    /// Reads both games' bodies on compared frame `at`, pairs the new ones,
    /// notes the events, and gives every slot body measure's values and
    /// distance this frame: the path's index, the original's value, the
    /// model's and the distance. `orig` and `mine` are the mapped fields'
    /// values this frame. New measures are added to `out.paths` (and, with
    /// `values`, to every earlier frame's values) as within on the frames
    /// before.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn frame<O: Runtime, M: Runtime>(
        &mut self,
        out: &mut Lockstep,
        spec: &Spec,
        original: &O,
        model: &M,
        at: usize,
        orig: &[Value],
        mine: &[Value],
        values: bool,
    ) -> Result<Vec<(usize, Value, Value, gp_sim::Fx)>, String> {
        self.mapped_events(out, at, orig, mine);
        if self.groups.is_empty() {
            return Ok(Vec::new());
        }
        // Every group's objects, their slots and kinds numbered on from the
        // groups before.
        let mut objects: Vec<SlotObject> = Vec::new();
        let (mut slot0, mut kind0) = (0u16, 0usize);
        for g in &self.groups {
            for mut o in slot_objects(original, g)? {
                o.slot += slot0;
                o.kind += kind0;
                objects.push(o);
            }
            slot0 += g.count;
            kind0 += g.kinds.len();
        }
        let objects: Vec<SlotObject> = objects
            .into_iter()
            .filter(|o| self.entity_of.get(o.kind).copied().flatten().is_some())
            .collect();
        let wanted: Vec<bool> = self.slot_kind_of.iter().map(Option::is_some).collect();
        let mbs: Vec<ModelBody> = model_bodies(model, spec, &wanted)?
            .into_iter()
            .filter(|b| b.alive && !b.waiting)
            .collect();
        let o_at = |o: &SlotObject| -> [i64; 4] {
            let get = |p: &str| {
                o.values
                    .iter()
                    .find(|v| v.0 == p)
                    .and_then(|v| match v.1 {
                        Value::Fixed(r) => Some(r),
                        Value::Int(n) => Some(n << 16),
                        _ => None,
                    })
                    .unwrap_or(0)
            };
            [get("x"), get("y"), get("vx"), get("vy")]
        };
        let m_at = |b: &ModelBody| [b.x, b.y, b.vx, b.vy];

        // The original's side: lives that go on, lives that end, new ones.
        let mut fresh_o: Vec<usize> = Vec::new();
        let mut holding = vec![false; self.in_slot.len()];
        for (k, o) in objects.iter().enumerate() {
            let s = usize::from(o.slot);
            let kind = self.entity_of[o.kind];
            match self.in_slot[s] {
                Some(b) if self.live[b].as_ref().is_some_and(|l| Some(l.kind) == kind) => {
                    holding[s] = true;
                }
                _ => fresh_o.push(k),
            }
        }
        for (slot, held) in self.in_slot.iter_mut().zip(&holding) {
            if !held
                && let Some(b) = slot.take()
                && let Some(l) = self.live[b].as_mut()
            {
                l.o.present = false;
                l.o.done = true;
            }
        }
        // The model's side, by ID.
        let mut fresh_m: Vec<usize> = Vec::new();
        let here: std::collections::BTreeSet<u64> = mbs.iter().map(|b| b.id).collect();
        // A restart or a level start rebuilds the model's bodies: every
        // body of a kind the pairing holds is gone and new ones of that
        // kind are here on the same frame. The original's bodies of that
        // kind that go on let go of their model side and pair again, as
        // after a resync (Donkey Kong's 75m draws its platforms and fires
        // on a death's 21st divider wrap and keeps them through the setup,
        // while the model's restart places them anew).
        let mut held: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
        for (id, &b) in &self.by_id {
            if let Some(l) = self.live[b].as_ref() {
                let e = held.entry(l.kind).or_default();
                e.0 += 1;
                if !here.contains(id) {
                    e.1 += 1;
                }
            }
        }
        let rebuilt: std::collections::BTreeSet<usize> = held
            .iter()
            .filter(|(kind, (n, gone))| {
                n == gone
                    && mbs
                        .iter()
                        .any(|m| m.kind == **kind && !self.by_id.contains_key(&m.id))
            })
            .map(|(kind, _)| *kind)
            .collect();
        if !rebuilt.is_empty() {
            let ids: Vec<(u64, usize)> = self.by_id.iter().map(|(id, b)| (*id, *b)).collect();
            for (id, b) in ids {
                let Some(l) = self.live[b].as_mut() else {
                    continue;
                };
                if !rebuilt.contains(&l.kind) || !l.o.present || l.o.done {
                    continue;
                }
                self.by_id.remove(&id);
                out.bodies[b].model = None;
                l.m = Side::default();
                l.rejoin = true;
            }
        }
        let gone: Vec<u64> = self
            .by_id
            .keys()
            .filter(|id| !here.contains(id))
            .copied()
            .collect();
        for id in gone {
            if let Some(b) = self.by_id.remove(&id)
                && let Some(l) = self.live[b].as_mut()
            {
                l.m.present = false;
                l.m.done = true;
            }
        }
        for (k, b) in mbs.iter().enumerate() {
            if !self.by_id.contains_key(&b.id) {
                fresh_m.push(k);
            }
        }

        // Pairing: a new body on one side with a new one on the other, or
        // with a body the other game has alone so far, of the same kind,
        // nearest first within the reach.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Who {
            New(usize),
            Alone(usize),
        }
        let mut o_side: Vec<(Who, usize, [i64; 4], u16)> = fresh_o
            .iter()
            .map(|&k| {
                let o = &objects[k];
                (
                    Who::New(k),
                    self.entity_of[o.kind].unwrap_or(0),
                    o_at(o),
                    o.slot,
                )
            })
            .collect();
        let mut m_side: Vec<(Who, usize, [i64; 4], u64)> = fresh_m
            .iter()
            .map(|&k| (Who::New(k), mbs[k].kind, m_at(&mbs[k]), mbs[k].id))
            .collect();
        for (b, l) in self.live.iter().enumerate() {
            let Some(l) = l else { continue };
            let body = &out.bodies[b];
            if l.o.present && body.model.is_none() && !l.m.done {
                o_side.push((
                    Who::Alone(b),
                    l.kind,
                    l.o.at.unwrap_or_default(),
                    body.slot.unwrap_or(0),
                ));
            }
            if l.m.present && body.slot.is_none() && !l.o.done {
                m_side.push((
                    Who::Alone(b),
                    l.kind,
                    l.m.at.unwrap_or_default(),
                    body.model.unwrap_or(0),
                ));
            }
        }
        let mut pairs: Vec<(i64, u16, u64, usize, usize)> = Vec::new();
        for (i, o) in o_side.iter().enumerate() {
            for (j, m) in m_side.iter().enumerate() {
                if o.1 != m.1 || matches!((o.0, m.0), (Who::Alone(_), Who::Alone(_))) {
                    continue;
                }
                let d = (o.2[0] - m.2[0]).abs() + (o.2[1] - m.2[1]).abs();
                if d <= self.reach {
                    pairs.push((d, o.3, m.3, i, j));
                }
            }
        }
        pairs.sort_unstable();
        let mut o_used = vec![false; o_side.len()];
        let mut m_used = vec![false; m_side.len()];
        let mut made: Vec<(Option<usize>, Option<usize>)> = Vec::new();
        for (_, _, _, i, j) in pairs {
            if o_used[i] || m_used[j] {
                continue;
            }
            o_used[i] = true;
            m_used[j] = true;
            made.push((Some(i), Some(j)));
        }
        for (i, o) in o_side.iter().enumerate() {
            if !o_used[i] && matches!(o.0, Who::New(_)) {
                made.push((Some(i), None));
            }
        }
        for (j, m) in m_side.iter().enumerate() {
            if !m_used[j] && matches!(m.0, Who::New(_)) {
                made.push((None, Some(j)));
            }
        }
        for (oi, mi) in made {
            let o = oi.map(|i| o_side[i]);
            let m = mi.map(|j| m_side[j]);
            let b = match (o.map(|x| x.0), m.map(|x| x.0)) {
                (Some(Who::Alone(b)), _) | (_, Some(Who::Alone(b))) => b,
                _ => {
                    let kind = o.map_or_else(|| m.map_or(0, |x| x.1), |x| x.1);
                    self.add(out, spec, kind, at, values)
                }
            };
            if let Some((Who::New(k), ..)) = o {
                let obj = &objects[k];
                out.bodies[b].slot = Some(obj.slot);
                self.in_slot[usize::from(obj.slot)] = Some(b);
                if let Some(l) = self.live[b].as_mut() {
                    l.o = Side::start(o_at(obj));
                }
                if at > 0 {
                    out.bodies[b].events.push(Event {
                        at,
                        original: true,
                        what: "appears".into(),
                    });
                }
            }
            if let Some((Who::New(k), ..)) = m {
                let mb = &mbs[k];
                out.bodies[b].model = Some(mb.id);
                self.by_id.insert(mb.id, b);
                if let Some(l) = self.live[b].as_mut() {
                    l.m = if std::mem::take(&mut l.rejoin) {
                        Side {
                            at: Some(m_at(mb)),
                            ..l.o.clone()
                        }
                    } else {
                        Side::start(m_at(mb))
                    };
                }
                if at > 0 {
                    out.bodies[b].events.push(Event {
                        at,
                        original: false,
                        what: "appears".into(),
                    });
                }
            }
            if out.bodies[b].slot.is_some()
                && out.bodies[b].model.is_some()
                && out.bodies[b].paired.is_none()
            {
                out.bodies[b].paired = Some(at);
            }
        }

        // Each slot body's sides move on, with their events.
        for (k, o) in objects.iter().enumerate() {
            if fresh_o.contains(&k) {
                continue;
            }
            if let Some(b) = self.in_slot[usize::from(o.slot)] {
                self.moved(out, b, true, o_at(o), at);
            }
        }
        for (k, mb) in mbs.iter().enumerate() {
            if fresh_m.contains(&k) {
                continue;
            }
            if let Some(&b) = self.by_id.get(&mb.id) {
                self.moved(out, b, false, m_at(mb), at);
            }
        }
        // Those that ended this frame.
        for (b, l) in self.live.iter_mut().enumerate() {
            let Some(l) = l else { continue };
            for (side, original) in [(&mut l.o, true), (&mut l.m, false)] {
                if side.done && side.at.is_some() {
                    side.at = None;
                    out.bodies[b].events.push(Event {
                        at,
                        original,
                        what: "is gone".into(),
                    });
                }
            }
        }

        // The measures.
        let mut res = Vec::new();
        for (b, l) in self.live.iter().enumerate() {
            let Some(l) = l else { continue };
            for (mi, ms) in l.measures.iter().enumerate() {
                let path = out.bodies[b].paths[mi];
                let read = |s: &Side| -> Value {
                    if !s.present {
                        return match ms {
                            Measure::Alive => Value::Flag(false),
                            _ => Value::Text(String::new()),
                        };
                    }
                    let a = s.at.unwrap_or_default();
                    match ms {
                        Measure::Field { name, .. } => Value::Fixed(match name.as_str() {
                            "x" => a[0],
                            "y" => a[1],
                            "vx" => a[2],
                            _ => a[3],
                        }),
                        Measure::Moving => Value::Text(s.moving.into()),
                        Measure::MovingY => Value::Text(s.moving_y.into()),
                        Measure::Phase => Value::Text(s.phase.into()),
                        Measure::Alive => Value::Flag(true),
                    }
                };
                let (ov, mv) = (read(&l.o), read(&l.m));
                let d = match ms {
                    Measure::Alive => crate::lockstep::distance_of(false, &ov, &mv),
                    Measure::Field { whole, .. } if l.o.present && l.m.present => {
                        crate::lockstep::distance_of(*whole, &ov, &mv)
                    }
                    _ if l.o.present && l.m.present => {
                        crate::lockstep::distance_of(false, &ov, &mv)
                    }
                    _ => gp_sim::Fx::ZERO,
                };
                res.push((path, ov, mv, d));
            }
        }
        Ok(res)
    }

    /// A new slot body of spec kind `kind`, first seen on frame `at`, with
    /// its measures added to `out.paths`.
    fn add(
        &mut self,
        out: &mut Lockstep,
        spec: &Spec,
        kind: usize,
        at: usize,
        values: bool,
    ) -> usize {
        let name = spec.kinds[kind].name.clone();
        let nth = out.bodies.iter().filter(|b| b.kind == name).count();
        let mut measures = Vec::new();
        let mut by_speed = false;
        let mut up_down = false;
        if let Some(sk) = self.slot_kind_of[kind] {
            // The group the slot kind is in, and its place there.
            let (mut g, mut k) = (0, sk);
            while g + 1 < self.groups.len() && k >= self.groups[g].kinds.len() {
                k -= self.groups[g].kinds.len();
                g += 1;
            }
            let slots = &self.groups[g];
            up_down = slots.kinds[k].up_down;
            for f in slots.fields_of(&slots.kinds[k]) {
                // A presentation field only loads into a start.
                if matches!(f.path.as_str(), "x" | "y" | "vx" | "vy") && !f.presentation {
                    by_speed |= f.path == "vx";
                    measures.push(Measure::Field {
                        name: f.path.clone(),
                        whole: f.whole || (f.frac.is_none() && f.scale.is_none()),
                    });
                }
            }
        }
        if measures
            .iter()
            .any(|m| matches!(m, Measure::Field { name, .. } if name == "x"))
        {
            measures.push(Measure::Moving);
        }
        if up_down {
            measures.push(Measure::MovingY);
        }
        let path = spec.kinds[kind].path.as_ref().map(|p| PathWay {
            vertical: p.vertical,
            negative: p.distance.raw() < 0,
        });
        if path.is_some() {
            measures.push(Measure::Phase);
        }
        measures.push(Measure::Alive);
        let mut paths = Vec::new();
        for m in &measures {
            paths.push(out.paths.len());
            out.paths.push(PathSeries {
                path: format!("{PREFIX}.{name}[{nth}].{}", m.name()),
                tolerance: gp_sim::Fx::ZERO,
                distance: vec![gp_sim::Fx::ZERO; at],
                within: at as u64,
                max: gp_sim::Fx::ZERO,
                first_split: None,
            });
            if values {
                for (o, m) in &mut out.values {
                    o.push(Value::Text(String::new()));
                    m.push(Value::Text(String::new()));
                }
            }
        }
        out.bodies.push(Body {
            kind: name,
            nth,
            slot: None,
            model: None,
            mapped: false,
            from: at,
            paired: None,
            paths,
            events: Vec::new(),
        });
        self.live.push(Some(Live {
            kind,
            measures,
            by_speed,
            path,
            o: Side::default(),
            m: Side::default(),
            rejoin: false,
        }));
        out.bodies.len() - 1
    }

    /// One side of body `b` stands at `now` this frame: its direction and
    /// phase move on, with an event when either changes.
    fn moved(&mut self, out: &mut Lockstep, b: usize, original: bool, now: [i64; 4], at: usize) {
        let Some(l) = self.live[b].as_mut() else {
            return;
        };
        let (by_speed, path) = (l.by_speed, l.path);
        let has_y = l.measures.iter().any(|m| matches!(m, Measure::MovingY));
        let s = if original { &mut l.o } else { &mut l.m };
        let before = s.at.unwrap_or(now);
        s.at = Some(now);
        let dx = now[0] - before[0];
        let dy = now[1] - before[1];
        let across = if by_speed { now[2] } else { dx };
        let was = (s.moving, s.phase);
        let was_y = s.moving_y;
        if dx.abs() <= WRAP << 16 {
            if across < 0 {
                s.moving = "left";
            } else if across > 0 {
                s.moving = "right";
            }
        }
        if dy.abs() <= WRAP << 16 {
            if dy < 0 {
                s.moving_y = "up";
            } else if dy > 0 {
                s.moving_y = "down";
            }
        }
        if let Some(p) = path {
            let d = if p.vertical { dy } else { dx };
            if d != 0 && d.abs() <= WRAP << 16 {
                let out_now = (d < 0) == p.negative;
                s.out = Some(out_now);
                s.still = 0;
                s.phase = p.word(out_now);
            } else if d == 0 && s.still + 1 < PAUSED {
                s.still += 1;
            } else {
                s.phase = match s.out {
                    Some(true) => "out",
                    Some(false) => "hidden",
                    None => "still",
                };
            }
        }
        let mut events = Vec::new();
        if was.0 != s.moving && was.0 != "none" {
            events.push(format!("turns {}", s.moving));
        }
        if has_y && was_y != s.moving_y && was_y != "none" {
            events.push(format!("turns {}", s.moving_y));
        }
        if was.1 != s.phase {
            events.push(phase_event(s.phase).to_owned());
        }
        for what in events {
            out.bodies[b].events.push(Event { at, original, what });
        }
    }

    /// The mapped bodies' events this frame, from their fields' changes.
    fn mapped_events(&mut self, out: &mut Lockstep, at: usize, orig: &[Value], mine: &[Value]) {
        for mb in &mut self.mapped {
            for (k, (i, field)) in mb.fields.iter().enumerate() {
                let (po, pm) = &mut mb.prev[k];
                for (prev, now, original) in [(po, &orig[*i], true), (pm, &mine[*i], false)] {
                    if let Some(p) = prev.as_ref()
                        && let Some(what) = change(field, p, now)
                    {
                        out.bodies[mb.body]
                            .events
                            .push(Event { at, original, what });
                    }
                    *prev = Some(now.clone());
                }
            }
        }
    }
}

/// A phase as an event: "rises", "sinks", "stops out", "hides".
fn phase_event(phase: &str) -> &'static str {
    match phase {
        "up" => "rises",
        "down" => "sinks",
        "left" => "moves out left",
        "right" => "moves out right",
        "out" => "stops out",
        "hidden" => "hides",
        _ => "stops",
    }
}

/// A mapped field's change as an event, if it is one.
fn change(field: &str, before: &Value, now: &Value) -> Option<String> {
    if crate::mapping::same(before, now) {
        return None;
    }
    let text = |v: &Value| match v {
        Value::Text(t) => t.clone(),
        other => crate::mapping::show(other),
    };
    match field {
        "facing" | "moved" | "moving" => {
            let t = text(now);
            (t != "none").then(|| format!("turns {t}"))
        }
        "state" => Some(format!("becomes {}", text(now))),
        "alive" => Some(
            if *now == Value::Flag(true) {
                "appears"
            } else {
                "is gone"
            }
            .into(),
        ),
        "vx" => {
            let sign = |v: &Value| match v {
                Value::Fixed(r) => r.signum(),
                Value::Int(n) => n.signum(),
                _ => 0,
            };
            let (a, b) = (sign(before), sign(now));
            (b != 0 && a != 0 && a != b).then(|| {
                if b < 0 {
                    "turns left".into()
                } else {
                    "turns right".into()
                }
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_are_named_by_their_place_and_events_are_matched_in_order() {
        assert_eq!(name("goomba", 0), "the goomba");
        assert_eq!(name("goomba", 1), "the second goomba");
        assert_eq!(name("piranha_plant", 0), "the piranha plant");
        let ev = |at, original, what: &str| Event {
            at,
            original,
            what: what.into(),
        };
        let b = Body {
            kind: "goomba".into(),
            nth: 1,
            slot: Some(2),
            model: Some(9),
            mapped: false,
            from: 0,
            paired: Some(0),
            paths: Vec::new(),
            events: vec![
                ev(10, true, "turns right"),
                ev(10, false, "turns right"),
                ev(40, true, "turns left"),
                ev(44, false, "turns left"),
                ev(60, true, "is gone"),
            ],
        };
        let m = mismatches(&b);
        assert_eq!(m.len(), 2, "{m:?}");
        assert_eq!(
            (m[0].what.as_str(), m[0].original, m[0].model),
            ("turns left", Some(40), Some(44))
        );
        assert_eq!(m[0].at(), 40);
        assert_eq!((m[1].original, m[1].model), (Some(60), None));
    }

    #[test]
    fn a_mapped_fields_change_is_an_event() {
        let t = |s: &str| Value::Text(s.into());
        assert_eq!(
            change("facing", &t("right"), &t("left")).as_deref(),
            Some("turns left")
        );
        assert_eq!(change("facing", &t("left"), &t("left")), None);
        assert_eq!(
            change("vx", &Value::Fixed(1 << 16), &Value::Fixed(-(1 << 15))).as_deref(),
            Some("turns left")
        );
        assert_eq!(change("vx", &Value::Fixed(0), &Value::Fixed(1)), None);
        assert_eq!(
            change("alive", &Value::Flag(true), &Value::Flag(false)).as_deref(),
            Some("is gone")
        );
        assert_eq!(
            change("state", &t("walking"), &t("flat")).as_deref(),
            Some("becomes flat")
        );
    }
}
