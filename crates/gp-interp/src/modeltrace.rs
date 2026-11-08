//! The frame trace of a model run (docs/interpreter.md, "Recordings").
//!
//! A model's canonical state has a length that changes with the bodies it
//! holds, which a trace's one field table can't describe. The trace holds a
//! view of the state by model path instead: state magic `GPSP`, version 1,
//! and one field for each path the run ever named, in the order the run
//! first named them, so `gp trace frame` and the inspector show model paths
//! as they show NES fields. An entity instance that isn't in the world on
//! some frame (before a level loads, say) reads as zeros and has its
//! `.present` flag false.
//!
//! Widths: a number is 8 bytes (little-endian, signed; fixed-point values
//! are in 65536ths), a flag 1 byte, and text (a level, state or facing
//! name) 24 bytes of UTF-8, zero padded. The view's hash is the frame hash's
//! `state`; the `ram` hash stays the canonical state's.

use std::collections::HashMap;

use gp_evidence::trace::{FieldDef, FieldTable, Width};
use gp_runtime::{Entry, Value};
use gp_sim::{Spec, World};

pub const MAGIC: &[u8; 4] = b"GPSP";
pub const VERSION: u32 = 1;
/// Bytes of a text value.
pub const TEXT_BYTES: u32 = 24;
const HEADER: usize = 8;

/// The kind of a value, which fixes its width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Num,
    Flag,
    Text,
}

impl Kind {
    fn of(v: &Value) -> Option<Self> {
        match v {
            Value::Int(_) | Value::Fixed(_) | Value::Entity(_) => Some(Self::Num),
            Value::Flag(_) => Some(Self::Flag),
            Value::Text(_) => Some(Self::Text),
            Value::Vec2(..) => None,
        }
    }

    fn width(self) -> u32 {
        match self {
            Self::Num => 8,
            Self::Flag => 1,
            Self::Text => TEXT_BYTES,
        }
    }
}

/// The part of a path that names an entity instance, such as
/// `entities.slug[1]`, when it does.
fn instance_of(path: &str) -> Option<&str> {
    path.starts_with("entities.")
        .then(|| path.find(']').map(|i| &path[..=i]))
        .flatten()
}

/// Collects the paths a run names, in first-seen order, then builds the
/// field table.
#[derive(Default)]
pub struct Layout {
    seen: HashMap<String, usize>,
    fields: Vec<(String, Kind)>,
}

impl Layout {
    /// Adds the paths of one frame's state.
    pub fn see(&mut self, spec: &Spec, w: &World) -> Result<(), String> {
        for (p, v) in crate::state::leaves(spec, w) {
            let name = gp_model::path::display(&p);
            if self.seen.contains_key(&name) {
                continue;
            }
            let kind = Kind::of(&v).ok_or_else(|| format!("{name} holds a value a trace can't"))?;
            if let Some(inst) = instance_of(&name) {
                let present = format!("{inst}.present");
                if !self.seen.contains_key(&present) {
                    self.seen.insert(present.clone(), self.fields.len());
                    self.fields.push((present, Kind::Flag));
                }
            }
            if name.len() > 255 {
                return Err(format!("{name} is too long for a field name"));
            }
            self.seen.insert(name.clone(), self.fields.len());
            self.fields.push((name, kind));
        }
        Ok(())
    }

    pub fn table(&self) -> FieldTable {
        FieldTable {
            state_magic: *MAGIC,
            state_version: VERSION,
            fields: self
                .fields
                .iter()
                .map(|(n, k)| FieldDef {
                    name: n.clone(),
                    width: Width::Fixed(k.width()),
                })
                .collect(),
        }
    }
}

/// Encodes states by a field table made with [`Layout`].
pub struct Encoder {
    /// Each field's offset into the state, by name.
    at: HashMap<String, usize>,
    len: usize,
}

impl Encoder {
    pub fn new(table: &FieldTable) -> Result<Self, String> {
        if &table.state_magic != MAGIC || table.state_version != VERSION {
            return Err("not a model trace field table".into());
        }
        let mut at = HashMap::new();
        let mut off = HEADER;
        for f in &table.fields {
            let Width::Fixed(n) = f.width else {
                return Err(format!("{} is a region", f.name));
            };
            at.insert(f.name.clone(), off);
            off += n as usize;
        }
        Ok(Self { at, len: off })
    }

    /// The view of a world. A path the table doesn't have is an error: the
    /// run named a path in the second pass that the first never did.
    pub fn encode(&self, spec: &Spec, w: &World) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; self.len];
        out[..4].copy_from_slice(MAGIC);
        out[4..HEADER].copy_from_slice(&VERSION.to_le_bytes());
        for (p, v) in crate::state::leaves(spec, w) {
            let name = gp_model::path::display(&p);
            let off = *self
                .at
                .get(&name)
                .ok_or_else(|| format!("{name} isn't in the trace's field table"))?;
            if let Some(inst) = instance_of(&name)
                && let Some(&p) = self.at.get(&format!("{inst}.present"))
            {
                out[p] = 1;
            }
            match v {
                Value::Int(n) | Value::Fixed(n) => {
                    out[off..off + 8].copy_from_slice(&n.to_le_bytes())
                }
                Value::Entity(n) => out[off..off + 8].copy_from_slice(&(n as i64).to_le_bytes()),
                Value::Flag(b) => out[off] = u8::from(b),
                Value::Text(t) => {
                    let bytes = t.as_bytes();
                    let n = bytes.len().min(TEXT_BYTES as usize);
                    out[off..off + n].copy_from_slice(&bytes[..n]);
                }
                Value::Vec2(..) => return Err(format!("{name} holds a value a trace can't")),
            }
        }
        Ok(out)
    }
}

/// The value a field holds, given its name, from its bytes. Numbers named
/// `x`, `y`, `vx` and `vy` (of entities and cameras) are fixed-point and
/// `id` is an entity; the field table has widths only, so the kind of a
/// number comes from its name.
pub fn value_of(name: &str, bytes: &[u8]) -> Value {
    match bytes.len() {
        1 => Value::Flag(bytes[0] != 0),
        8 => {
            let n = i64::from_le_bytes(bytes.try_into().expect("8"));
            let last = name.rsplit('.').next().unwrap_or("");
            let moving = name.starts_with("entities.") || name.starts_with("cameras.");
            if moving && matches!(last, "x" | "y" | "vx" | "vy") {
                Value::Fixed(n)
            } else if name.starts_with("entities.") && last == "id" {
                Value::Entity(n as u64)
            } else {
                Value::Int(n)
            }
        }
        _ => {
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            Value::Text(String::from_utf8_lossy(&bytes[..end]).into_owned())
        }
    }
}

/// Every field of a state as model path entries, leaving out instances
/// that weren't in the world and their `.present` flags.
pub fn entries(table: &FieldTable, state: &[u8]) -> Result<Vec<Entry>, String> {
    let fields = table.split(state).map_err(|e| e.to_string())?;
    let present: HashMap<&str, bool> = fields
        .iter()
        .filter_map(|(n, b)| {
            n.strip_suffix(".present")
                .map(|inst| (inst, b.first().is_some_and(|b| *b != 0)))
        })
        .collect();
    Ok(fields
        .into_iter()
        .filter(|(n, _)| !n.ends_with(".present"))
        .filter(|(n, _)| instance_of(n).is_none_or(|i| present.get(i).copied().unwrap_or(true)))
        .map(|(n, b)| Entry {
            path: n.to_owned(),
            value: value_of(n, b),
        })
        .collect())
}

/// Like `Value`'s display in `gp query`: whole numbers, fixed-point with
/// its fraction, flags, text.
pub fn show(v: &Value) -> String {
    crate::mapping::show(v)
}
