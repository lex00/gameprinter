//! What a start can't carry: each object the original has loaded in its
//! slots that the state mapping can't turn into a model body as the
//! original has it (docs/interpreter.md, "Carrying the original's
//! objects"). A start is refused when it has one, so the rebuilt game never
//! begins from a state that is wrong in a way the notes don't show.
//!
//! The test is one rule for every kind, from the model and the mapping
//! alone. A body of an entity keeps some state between frames, by its
//! components ([`needs`]): its place, and the fraction of it when it moves
//! by fractions; its facing for a walk or a flight; its speed up and down
//! for a fall; its path's spawn end, way and wait; its mechanic words for a
//! hook or a component that counts; its machine's state, and the time in it
//! when a state ends by time. Each of these must be a slot field of its
//! kind, and each value the original holds must load ([`gaps`]). A kind is
//! carried by adding its mapping rows; nothing here names an entity.

use gp_runtime::{Runtime, Value};
use gp_sim::Spec;
use serde::{Deserialize, Serialize};

use crate::mapping::{Mapping, SlotKind, Slots};

/// Why an object can't be carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// No slot kind names the object's ID, or the model has no entity for
    /// the kind that does.
    UnmappedKind,
    /// A field of its place, speed, facing or path isn't read.
    MissingField,
    /// A counter it keeps (a mechanic word, a wait, a path's way, a
    /// state's time) isn't read.
    TimerOrPhase,
    /// Its machine's state isn't read, the original's value names no state,
    /// or the original has it in a state the model has none for (an enemy
    /// falling defeated).
    StateNotNamed,
    /// The mapping says the model can't hold it (a slot kind's `refuse`).
    Refused,
}

impl Cause {
    pub const ALL: [Cause; 5] = [
        Cause::UnmappedKind,
        Cause::MissingField,
        Cause::TimerOrPhase,
        Cause::StateNotNamed,
        Cause::Refused,
    ];

    /// In plain words: "an unmapped kind".
    pub fn words(self) -> &'static str {
        match self {
            Cause::UnmappedKind => "an unmapped kind",
            Cause::MissingField => "a missing field",
            Cause::TimerOrPhase => "a timer or phase not read",
            Cause::StateNotNamed => "a state not named",
            Cause::Refused => "refused by the mapping",
        }
    }
}

/// One part of a body's state a carried body needs from the original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    /// The slot field path that holds it: `x`, `facing`, `ext[1]`, or
    /// `x.frac` for the fraction of `x`.
    pub path: String,
    pub cause: Cause,
    /// The component or hook that keeps it, for the reason.
    pub by: String,
}

fn need(path: &str, cause: Cause, by: &str) -> Need {
    Need {
        path: path.into(),
        cause,
        by: by.into(),
    }
}

/// What a body of `kind` keeps between frames that a start must read from
/// the original, over its own components and every state's. The time in a
/// state is needed only while the body is in a state that ends by time,
/// which [`gaps`] checks by the state the original holds. An orbit's
/// "placed" word and a part's words are set by the carry itself, and a
/// bounce's last-frame memory follows from the speed, so they aren't here.
pub fn needs(spec: &Spec, kind: usize) -> Vec<Need> {
    use Cause::*;
    let k = &spec.kinds[kind];
    let mut out = vec![
        need("x", MissingField, "place"),
        need("y", MissingField, "place"),
    ];
    let add = |n: Need, out: &mut Vec<Need>| {
        if !out.iter().any(|m| m.path == n.path) {
            out.push(n);
        }
    };
    let mut kinds = vec![k];
    if let Some(m) = &k.machine {
        kinds.extend(m.states.iter().map(|s| &s.kind));
        if m.states.len() > 1 {
            add(need("state", StateNotNamed, "machine"), &mut out);
        }
    }
    let frac = |f: gp_sim::Fx| f.frac() != 0;
    for s in kinds {
        if let Some(w) = s.walk.as_ref().filter(|w| w.speed != gp_sim::Fx::ZERO) {
            add(need("facing", MissingField, "walk"), &mut out);
            if frac(w.speed) {
                add(
                    need("x.frac", MissingField, "walk (a speed in fractions)"),
                    &mut out,
                );
            }
        }
        if s.patrol.is_some() {
            add(need("facing", MissingField, "patrol"), &mut out);
        }
        if let Some(f) = s
            .fly
            .as_ref()
            .filter(|f| f.speed != gp_sim::Fx::ZERO || f.keep_speed)
        {
            add(need("facing", MissingField, "fly"), &mut out);
            if f.keep_speed {
                add(need("vx", MissingField, "fly (keep_speed)"), &mut out);
            }
            if frac(f.speed) {
                add(
                    need("x.frac", MissingField, "fly (a speed in fractions)"),
                    &mut out,
                );
            }
        }
        if let Some(f) = &s.fall {
            add(need("vy", MissingField, "fall"), &mut out);
            if frac(f.gravity) {
                add(
                    need("y.frac", MissingField, "fall (gravity in fractions)"),
                    &mut out,
                );
            }
        }
        if s.hop.as_ref().is_some_and(|h| !h.pattern.is_empty()) {
            add(
                need("ext[0]", TimerOrPhase, "hop (its hops counted)"),
                &mut out,
            );
        }
        if let Some(p) = &s.path {
            let home = if p.vertical { "home_y" } else { "home_x" };
            add(need(home, MissingField, "path (its spawn end)"), &mut out);
            add(need("path_back", TimerOrPhase, "path (its way)"), &mut out);
            add(
                need("wait", TimerOrPhase, "path (its wait at an end)"),
                &mut out,
            );
        }
        if let Some(o) = &s.oscillate {
            let (home, v) = if o.vertical {
                ("home_y", "vy")
            } else {
                ("home_x", "vx")
            };
            add(
                need(home, MissingField, "oscillate (where it was placed)"),
                &mut out,
            );
            add(
                need(v, MissingField, "oscillate (its swing's speed)"),
                &mut out,
            );
        }
        if let Some(l) = &s.leap {
            add(need("ext[0]", TimerOrPhase, "leap (its wait)"), &mut out);
            add(need("ext[1]", TimerOrPhase, "leap (its phase)"), &mut out);
            add(need("vy", MissingField, "leap (its speed)"), &mut out);
            if l.from.is_none() {
                add(need("home_y", MissingField, "leap (its start)"), &mut out);
            }
        }
        // A launcher's entry is its slot in its group; its timer is a word.
        if s.launcher.is_some() {
            add(
                need("ext[0]", TimerOrPhase, "launcher (its timer)"),
                &mut out,
            );
        }
        if s.launched {
            add(
                need("ext[0]", TimerOrPhase, "launched (aimed on its first turn)"),
                &mut out,
            );
        }
        if s.shuttle.is_some() {
            add(
                need("ext[0]", TimerOrPhase, "shuttle (its count)"),
                &mut out,
            );
            add(need("ext[1]", TimerOrPhase, "shuttle (its way)"), &mut out);
        }
        if s.springboard.is_some() {
            for i in 0..3 {
                add(
                    need(
                        &format!("ext[{i}]"),
                        TimerOrPhase,
                        "springboard (its press)",
                    ),
                    &mut out,
                );
            }
        }
        if s.orbit.is_some() {
            add(need("ext[0]", TimerOrPhase, "orbit (its angle)"), &mut out);
        }
        if s.health
            .as_ref()
            .is_some_and(|h| h.points > 1 || h.invulnerable > 0)
        {
            add(need("hp", TimerOrPhase, "health"), &mut out);
        }
        if s.shoot.is_some() {
            add(
                need("shot", TimerOrPhase, "shoot (its count to the next)"),
                &mut out,
            );
        }
        if let Some(h) = &s.ext {
            for i in 0..4 {
                add(
                    need(&format!("ext[{i}]"), TimerOrPhase, &format!("the hook {h}")),
                    &mut out,
                );
            }
        }
    }
    out
}

/// The needs of `kind` that the slots' fields for a slot kind don't read.
pub fn unread(spec: &Spec, slots: &Slots, sk: &SlotKind, kind: usize) -> Vec<Need> {
    let fields = slots.fields_of(sk);
    needs(spec, kind)
        .into_iter()
        .filter(|n| {
            let (path, frac) = match n.path.strip_suffix(".frac") {
                Some(p) => (p, true),
                None => (n.path.as_str(), false),
            };
            !fields
                .iter()
                .any(|f| f.path == path && (!frac || f.frac.is_some()))
        })
        .collect()
}

/// One loaded object a start can't carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    /// The slot group, 0 for the first slots.
    pub group: usize,
    pub slot: u16,
    pub id: u8,
    /// The model entity the mapping names for it, when one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    pub cause: Cause,
    /// The slot field, when the cause is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// "ext[1] isn't read: the hook smb_lakitu keeps it".
    pub why: String,
}

impl Gap {
    /// What it is, in game words: the entity, or the ID.
    pub fn what(&self) -> String {
        match &self.entity {
            Some(e) => e.replace('_', " "),
            None => format!("object {:#04x}", self.id),
        }
    }
}

/// Every loaded object in the original's slots that a start there can't
/// carry, slot by slot, group by group. Empty when the mapping has no
/// slots, or every loaded object is one the mapping and the model hold.
/// A slot with an ID a kind lists under another flag value is that kind's
/// second part (`flags`), carried with it.
pub fn gaps<O: Runtime>(o: &O, spec: &Spec, mapping: &Mapping) -> Result<Vec<Gap>, String> {
    let mut out = Vec::new();
    for (gi, g) in mapping.slot_groups().into_iter().enumerate() {
        for slot in 0..g.count {
            let flag = g.flag_of(slot, |a| crate::bodies::read_byte(o, a))?;
            if flag == 0 {
                continue;
            }
            let id = g.id_of(slot, |a| crate::bodies::read_byte(o, a))?;
            let state = match g.state {
                Some(at) => crate::bodies::read_byte(o, at + slot)?,
                None => 0,
            };
            out.extend(object_gaps(
                o,
                spec,
                mapping,
                (gi, g),
                slot,
                (flag, id, state),
            )?);
        }
    }
    Ok(out)
}

/// The gaps of one loaded object: its flag, ID and state byte.
fn object_gaps<O: Runtime>(
    o: &O,
    spec: &Spec,
    mapping: &Mapping,
    (gi, g): (usize, &Slots),
    slot: u16,
    (flag, id, state): (u8, u8, u8),
) -> Result<Vec<Gap>, String> {
    let gap = |entity: Option<&str>, cause, field: Option<&str>, why: String| Gap {
        group: gi,
        slot,
        id,
        entity: entity.map(str::to_owned),
        cause,
        field: field.map(str::to_owned),
        why,
    };
    // An object the original has ended (an enemy falling defeated, a
    // fireball exploding) is gone in the model, as the model's own body is
    // once it is defeated: its slot is free with what it left.
    if state & g.ends.unwrap_or(0) != 0 {
        return Ok(Vec::new());
    }
    let of_id: Vec<&SlotKind> = g.kinds.iter().filter(|k| k.ids.contains(&id)).collect();
    if g.is_part(flag) && !of_id.iter().any(|k| g.flag_fits(k, flag)) {
        // Its ID means nothing: the object is in the slot its flag names.
        return Ok(Vec::new());
    }
    if of_id.is_empty() {
        return Ok(vec![gap(
            None,
            Cause::UnmappedKind,
            None,
            format!("no slot kind names ID {id:#04x}"),
        )]);
    }
    let Some(sk) = of_id.iter().find(|k| g.flag_fits(k, flag)) else {
        // A second part of a kind kept under another flag.
        return Ok(Vec::new());
    };
    let name = sk.entity.as_str();
    if let Some(why) = &sk.refuse {
        return Ok(vec![gap(Some(name), Cause::Refused, None, why.clone())]);
    }
    if sk.leave.is_some() || sk.holds_slot || state & sk.ends.or(g.ends).unwrap_or(0) != 0 {
        return Ok(Vec::new());
    }
    let Some(kind) = spec.kinds.index_of(name) else {
        return Ok(vec![gap(
            Some(name),
            Cause::UnmappedKind,
            None,
            format!("the model has no entity `{name}`"),
        )]);
    };
    let mut out: Vec<Gap> = unread(spec, g, sk, kind)
        .into_iter()
        .map(|n| {
            gap(
                Some(name),
                n.cause,
                Some(&n.path),
                format!("{} isn't read: {} keeps it", n.path, n.by),
            )
        })
        .collect();
    // Each value read must load as the carry loads it.
    let base = gp_sim::World::power_on(spec);
    let origin = spec
        .levels
        .get(base.level)
        .map_or([gp_sim::Fx::ZERO; 2], |l| l.origin);
    let mut b =
        gp_sim::systems::new_body(spec, &base, kind, gp_sim::Fx::ZERO, gp_sim::Fx::ZERO, false);
    let fields = g.fields_of(sk);
    for f in &fields {
        let mut bytes = Vec::new();
        for sel in f.slot_selectors(slot) {
            bytes.push(crate::bodies::read_sel(o, &sel)?);
        }
        let v = f.slot_value(slot, &bytes);
        if let Value::Text(t) = &v
            && t.starts_with("(unnamed value")
        {
            out.push(gap(
                Some(name),
                Cause::StateNotNamed,
                Some(&f.path),
                format!(
                    "the original holds {t} for {}, which the mapping names nothing",
                    f.path
                ),
            ));
            continue;
        }
        if f.path == "x" || f.path == "y" {
            continue;
        }
        let (field, index) = match f.path.split_once('[') {
            Some((n, rest)) => (n, rest.trim_end_matches(']').parse::<i64>().ok()),
            None => (f.path.as_str(), None),
        };
        let mut scratch = Vec::new();
        if let Err(why) = crate::translate::set_body(
            spec,
            mapping,
            &mut scratch,
            &mut b,
            (field, index),
            f,
            &v,
            origin,
            &format!("entities.{name}[*]"),
        ) {
            let cause = needs(spec, kind)
                .into_iter()
                .find(|n| n.path == f.path)
                .map_or(Cause::MissingField, |n| n.cause);
            out.push(gap(Some(name), cause, Some(&f.path), why));
        }
    }
    // A state that ends by time needs the time in it.
    if let Some(m) = &spec.kinds[kind].machine
        && m.states[b.state]
            .transitions
            .iter()
            .any(|t| t.after.is_some())
        && !fields.iter().any(|f| f.path == "time")
    {
        out.push(gap(
            Some(name),
            Cause::TimerOrPhase,
            Some("time"),
            format!(
                "time isn't read: it is {}, a state that ends by time",
                m.states[b.state].name
            ),
        ));
    }
    Ok(out)
}

/// The gaps in words, for a start's `cant`: "the original has a lakitu
/// (ext[1] isn't read: the hook smb_lakitu keeps it) and 2 more here that
/// a start can't carry".
pub fn says(gaps: &[Gap]) -> Option<String> {
    let first = gaps.first()?;
    let objects = {
        let mut seen: Vec<(usize, u16)> = gaps.iter().map(|g| (g.group, g.slot)).collect();
        seen.dedup();
        seen.len()
    };
    let more = if objects > 1 {
        format!(" and {} more", objects - 1)
    } else {
        String::new()
    };
    Some(format!(
        "the original has a {} ({}){more} here that a start can't carry",
        first.what(),
        first.why
    ))
}

#[cfg(test)]
mod tests;
