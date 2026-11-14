//! The original's state as a model state: the state mapping read in
//! reverse (docs/interpreter.md, "Lockstep: distance and resync"). From
//! the original at any frame, a recording's replayed to it or an emulator
//! snapshot restored, it writes every mapped value into a model world and
//! notes, by model path, what it loaded, what it could only approximate
//! and what the mapping doesn't hold. The result is a canonical model state
//! any runtime of the model loads with the runtime protocol's `load-state`.
//!
//! Nothing is guessed: a part the mapping has no field for keeps the value
//! of the world the translation starts from (the model's own state for a
//! resync, its power-on state or the level's start to start somewhere new),
//! and its note says so.

use gp_runtime::{LoadHow, LoadNote, Runtime, Value};
use gp_sim::{Fx, Spec, World};

use crate::mapping::{Field, Mapping};

/// A model state built from the original's, and how each part was taken.
#[derive(Debug, Clone)]
pub struct Translation {
    pub world: World,
    pub notes: Vec<LoadNote>,
}

impl Translation {
    /// The canonical state, for `load-state`.
    pub fn state(&self) -> Vec<u8> {
        self.world.canonical()
    }

    /// The notes of one kind.
    pub fn with(&self, how: LoadHow) -> impl Iterator<Item = &LoadNote> {
        self.notes.iter().filter(move |n| n.how == how)
    }
}

fn note(path: impl Into<String>, how: LoadHow, why: impl Into<String>) -> LoadNote {
    LoadNote {
        path: path.into(),
        how,
        why: why.into(),
    }
}

/// Reads each mapped field's value from the original, in the mapping's
/// order, as the model's units.
pub fn read_original<O: Runtime>(original: &O, mapping: &Mapping) -> Result<Vec<Value>, String> {
    Ok(read_noted(original, mapping)?
        .into_iter()
        .map(|(v, _)| v)
        .collect())
}

/// For each mapped field, whether a case that holds leaves it unmeasured
/// on this frame (feature `field.case.unmeasured`).
pub fn unmeasured<O: Runtime>(original: &O, mapping: &Mapping) -> Result<Vec<bool>, String> {
    let mut query = |sel: &gp_runtime::select::Selector| -> Result<i64, String> {
        let sel = sel.to_string();
        original
            .query(&sel)?
            .first()
            .and_then(|e| e.value.as_int())
            .ok_or(format!("{sel} isn't a byte"))
    };
    mapping
        .fields
        .iter()
        .map(|f| {
            Ok(f.case_held(&mut query)?
                .is_some_and(|c| c.unmeasured.is_some()))
        })
        .collect()
}

/// [`read_original`] with each field's reason, from the case that holds
/// (`[[field.case]]`), that a model state taken from it is approximated.
pub fn read_noted<O: Runtime>(
    original: &O,
    mapping: &Mapping,
) -> Result<Vec<(Value, Option<String>)>, String> {
    let mut query = |sel: &str| -> Result<i64, String> {
        original
            .query(sel)?
            .first()
            .and_then(|e| e.value.as_int())
            .ok_or(format!("{sel} isn't a byte"))
    };
    mapping
        .fields
        .iter()
        .map(|f| {
            let (v, case) = f.read(&mut query)?;
            Ok((v, case.and_then(|c| c.approximated.clone())))
        })
        .collect()
}

/// The model level the original is in: the mapping's `level` field, or
/// failing that the first `[[slots.level]]` whose condition holds. `None`
/// when the mapping can't say.
pub fn level_of<O: Runtime>(
    original: &O,
    mapping: &Mapping,
    values: &[Value],
) -> Result<Option<(String, &'static str)>, String> {
    if let Some(i) = mapping.fields.iter().position(|f| f.path == "level")
        && let Value::Text(name) = &values[i]
    {
        return Ok(Some((name.clone(), "the mapping's level field")));
    }
    let Some(slots) = &mapping.slots else {
        return Ok(None);
    };
    for l in &slots.levels {
        let cond = gp_runtime::select::Condition::parse(&l.when)?;
        let holds = cond.eval(&mut |sel| {
            original
                .query(&sel.to_string())?
                .first()
                .and_then(|e| e.value.as_int())
                .ok_or(format!("{sel} names no number"))
        })?;
        if holds {
            return Ok(Some((l.level.clone(), "the slots' level conditions")));
        }
    }
    Ok(None)
}

/// Translates the original's state as it stands into a model state, from
/// `base`.
pub fn from_original<O: Runtime>(
    original: &O,
    spec: &Spec,
    base: &World,
    mapping: &Mapping,
) -> Result<Translation, String> {
    let mut t = keeping_entities(original, spec, base, mapping)?;
    // A level drawn ahead of the view (core 0.4.10's `draw`) is drawn to
    // where the view now is, with no set under way: the original's parser
    // may be partway through one, which the mapping doesn't hold.
    gp_sim::systems::start_draw(spec, &mut t.world);
    if mapping.slots.is_some() {
        carry_slots(original, spec, mapping, &mut t)?;
    }
    carry_cells(original, spec, mapping, &mut t)?;
    Ok(t)
}

/// Changes the cells in view the original has changed, for a mapping whose
/// `[cells]` hold its level grid: one of the kinds it names, here in the
/// model, whose byte in the original is 0 is emptied, or with `becomes`
/// takes the kind its byte gives. Super Mario Bros.' block buffer has the
/// coins already taken gone, and the model's level has them all; Moss
/// Hop's has its ? block used.
fn carry_cells<O: Runtime>(
    original: &O,
    spec: &Spec,
    mapping: &Mapping,
    t: &mut Translation,
) -> Result<(), String> {
    let Some(cells) = &mapping.cells else {
        return Ok(());
    };
    let kinds: Vec<_> = cells
        .kinds
        .iter()
        .map(|k| crate::compile::tile_kind(*k))
        .collect();
    let w = &mut t.world;
    let ts = spec.levels[w.level].tile_size;
    let left = w.cameras.first().map_or(0, |c| c[0].floor()).div_euclid(ts);
    let mut taken = 0;
    for col in left.max(0)..left + cells.view {
        let page = cells.pages[col.div_euclid(cells.page_columns) as usize % cells.pages.len()];
        let at = col.rem_euclid(cells.page_columns);
        for row in 0..cells.rows {
            let (x, y) = (col * ts, (row + cells.top) * ts);
            if !kinds.contains(&w.tile_at(spec, x, y)) {
                continue;
            }
            let addr = i64::from(page) + at + row * cells.page_columns;
            let byte = crate::bodies::read_byte(original, addr as u16)?;
            let now = if cells.becomes.is_empty() {
                (byte == 0).then_some(gp_sim::spec::Tile::Empty)
            } else {
                cells
                    .becomes
                    .iter()
                    .find(|(b, _)| *b == byte)
                    .map(|(_, k)| crate::compile::tile_kind(*k))
            };
            if let Some(kind) = now
                && kind != w.tile_at(spec, x, y)
            {
                w.set_tile(spec, x, y, kind);
                taken += 1;
            }
        }
    }
    let path = format!("levels.{}.tiles", spec.levels[w.level].name);
    if let Some(n) = t.notes.iter_mut().find(|n| n.path == path) {
        n.how = LoadHow::Approximated;
        n.why = format!(
            "{taken} cells in view the original has changed are as it has them; the rest are the model's"
        );
    }
    Ok(())
}

/// What a resync loads: [`keeping_entities`], and with `slots.resync` the
/// original's slot objects carried in place of the model's own.
pub fn for_resync<O: Runtime>(
    original: &O,
    spec: &Spec,
    base: &World,
    mapping: &Mapping,
) -> Result<Translation, String> {
    let mut t = keeping_entities(original, spec, base, mapping)?;
    if mapping.slots.as_ref().is_some_and(|s| s.resync) {
        carry_slots(original, spec, mapping, &mut t)?;
    }
    Ok(t)
}

/// [`from_original`] without carrying the original's slot objects: the
/// entities the mapping doesn't hold are `base`'s. A resync keeps the
/// model's own enemies this way, unless the mapping's `slots.resync`
/// says otherwise ([`for_resync`]).
pub fn keeping_entities<O: Runtime>(
    original: &O,
    spec: &Spec,
    base: &World,
    mapping: &Mapping,
) -> Result<Translation, String> {
    let read = read_noted(original, mapping)?;
    let (values, why): (Vec<Value>, Vec<Option<String>>) = read.into_iter().unzip();
    let level = level_of(original, mapping, &values)?;
    let mut t = translate(spec, base, mapping, &values, level)?;
    // A case that holds makes its field's value approximate.
    for (f, why) in mapping.fields.iter().zip(why) {
        let Some(why) = why else { continue };
        if let Some(n) = t.notes.iter_mut().find(|n| n.path == f.path)
            && n.how != LoadHow::Missing
        {
            n.how = LoadHow::Approximated;
            n.why = format!("{}; {why}", n.why);
        }
    }
    carry_pending(original, spec, mapping, &mut t)?;
    Ok(t)
}

/// Loads the changes the original holds back (the mapping's `pending`) as
/// the model's own waiting changes, in place of any the base world has for
/// that counter: an award made just before the start still reaches the
/// counter on the original's frame. A timer that reads `at` now makes its
/// change on the next frame.
fn carry_pending<O: Runtime>(
    original: &O,
    spec: &Spec,
    mapping: &Mapping,
    t: &mut Translation,
) -> Result<(), String> {
    for p in &mapping.pending {
        let c = spec
            .counters
            .index_of(&p.counter)
            .ok_or(format!("pending: the model has no counter {}", p.counter))?
            as u32;
        t.world.pending.retain(|(n, _, _)| *n != c);
        let mut waiting = 0;
        for slot in 0..p.count {
            let amount = crate::bodies::read_byte(original, p.amount + slot)?;
            let timer = crate::bodies::read_byte(original, p.timer + slot)?;
            let by = p.by[usize::from(amount).min(p.by.len() - 1)];
            if amount != 0 && by != 0 && timer >= p.at {
                let left = u32::from(timer - p.at) + 1;
                t.world.pending.push((c, by, left));
                waiting += 1;
            }
        }
        t.notes.push(note(
            format!("pending.{}", p.counter),
            LoadHow::Loaded,
            format!("{waiting} changes still to come"),
        ));
    }
    Ok(())
}

/// Carries the original's slot objects into the translated world, for a
/// mapping whose `[slots]` hold its enemies in reused slots
/// (docs/open-questions.md, "How should enemies be carried into a start
/// past a level's start?"). The model's own entities but the player go:
/// those that have appeared, those a hook or a spawn made, and those still
/// waiting whose spawn the view has already reached, since the original
/// has spawned those too and keeps them in its slots or has let them go.
/// A level's fixture stays: a body its spawns placed whose entity takes no
/// slot (`no_slot`) and isn't a kind the slots name.
/// Then each living slot object of a kind the slots name becomes a body of
/// its entity at the slot's values, facing the player as an appearing
/// walker does unless the slots hold its facing, with what the slots don't
/// hold as a new body has it: every slot field loads as a mapped entity's
/// field of the same name does (`vx`, `facing`, `state`, `ext[i]`, a
/// path's `home_y`, `path_back` and `wait`). A path's ends say which way
/// it heads. The model's frame count takes the slots' clock as its low
/// byte. The note for each entity says it is approximated.
fn carry_slots<O: Runtime>(
    original: &O,
    spec: &Spec,
    mapping: &Mapping,
    t: &mut Translation,
) -> Result<(), String> {
    use gp_sim::spec::Role;
    let Some(slots) = &mapping.slots else {
        return Ok(());
    };
    let w = &mut t.world;
    let level = &spec.levels[w.level];
    // What the view has reached, by each spawn's rule: within
    // `spawn_ahead`, or a `drawn` spawn's column drawn.
    let reached: Vec<bool> = (0..w.bodies.len())
        .map(|i| gp_sim::systems::reached(spec, w, i))
        .collect();
    // A level's own fixture stays: a body its spawns placed whose entity
    // takes no slot and no slot names (Donkey Kong's oil drum and Donkey
    // Kong himself, which the game keeps outside any slot).
    let slotted: Vec<&str> = mapping
        .slot_groups()
        .into_iter()
        .flat_map(|g| g.kinds.iter().map(|k| k.entity.as_str()))
        .collect();
    let fixture = |i: usize, b: &gp_sim::world::Body| {
        let k = &spec.kinds[b.kind];
        i < level.spawns.len() && k.no_slot && !slotted.contains(&k.name.as_str())
    };
    let mut gone = 0;
    for (i, b) in w.bodies.iter_mut().enumerate() {
        if spec.kinds[b.kind].role == Role::Player || !b.alive || fixture(i, b) {
            continue;
        }
        if !b.waiting || i >= level.spawns.len() || reached[i] {
            b.alive = false;
            b.waiting = false;
            gone += 1;
        }
    }
    let player = w
        .bodies
        .iter()
        .find(|b| spec.kinds[b.kind].role == Role::Player)
        .map(|b| b.x);
    // Numbered slots (`slot_rest`) start empty, and take the carried
    // bodies and the empty slots' rests below.
    w.slots = match level.spawn_slots {
        Some(n) if level.slot_rest => vec![gp_sim::world::Slot::default(); n as usize],
        _ => Vec::new(),
    };
    let mut carried: Vec<(usize, Fx)> = Vec::new();
    // What each kind's slot fields couldn't load as they are, by path.
    let mut short: Vec<(usize, String, String)> = Vec::new();
    let origin = level.origin;
    // Every group's objects (schema 9): the first group's in their
    // numbered slots, the others' as bodies, and a group with a `rest`
    // (schema 11) in the shot slots too.
    let groups = mapping.slot_groups();
    // The shot group: the first after the enemies' with a `rest`.
    let shot_group = groups
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, g)| !g.rest.is_empty())
        .map(|(gi, _)| gi);
    w.shots = shot_group.map_or_else(Vec::new, |gi| {
        vec![gp_sim::world::ShotSlot::default(); usize::from(groups[gi].count)]
    });
    let mut objects = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        for o in crate::bodies::carried_slot_objects(original, g)? {
            let k = &g.kinds[o.kind];
            objects.push((gi, k.entity.as_str(), g.fields_of(k), o, k.holds_slot));
        }
    }
    // An object past the end of its life that still has its slot (`held`)
    // is a body too, so the next one takes the slot the game gives it.
    if let Some(held) = &slots.held {
        for o in crate::bodies::held_objects(original, slots)? {
            if !objects.iter().any(|x| x.0 == 0 && x.3.slot == o.slot) {
                objects.push((
                    0,
                    held.entity.as_str(),
                    slots.fields_with(&held.fields),
                    o,
                    false,
                ));
            }
        }
    }
    for (gi, entity, fields, o, holds) in objects {
        let Some(kind) = spec.kinds.index_of(entity) else {
            continue;
        };
        let get = |p: &str| {
            o.values
                .iter()
                .find(|v| v.0 == p)
                .and_then(|v| raw(&v.1))
                .map(Fx::from_raw)
        };
        // An object that only holds its slot (`holds_slot`: a timer, a
        // record) needs no place: it is carried where the level starts
        // when its slot maps none.
        let (x, y) = match (get("x"), get("y")) {
            (Some(x), Some(y)) => (x, y),
            _ if holds => (Fx::ZERO, Fx::ZERO),
            _ => continue,
        };
        let left = player.is_some_and(|p| p < x);
        let mut b = gp_sim::systems::new_body(spec, w, kind, x, y, left);
        let at = format!("entities.{entity}[*]");
        for (path, v) in &o.values {
            if path == "x" || path == "y" {
                continue;
            }
            let Some(f) = fields.iter().find(|f| &f.path == path) else {
                continue;
            };
            let (field, index) = match path.split_once('[') {
                Some((name, rest)) => (name, rest.trim_end_matches(']').parse::<i64>().ok()),
                None => (path.as_str(), None),
            };
            let mut scratch = Vec::new();
            match set_body(
                spec,
                mapping,
                &mut scratch,
                &mut b,
                (field, index),
                f,
                v,
                origin,
                &at,
            ) {
                Ok(None) => {}
                Ok(Some(why)) | Err(why) => short.push((kind, path.clone(), why)),
            }
        }
        at_path_end(spec, &mut b);
        // A carried orbit pivot has had its first frame: it is in place, so
        // `place` isn't added again, and its parts come with it.
        let orbit = spec.kinds[kind].orbit.clone();
        if orbit.is_some() {
            b.ext[1] = 1;
        }
        w.bodies.push(b);
        carried.push((kind, x));
        // A level with numbered slots (`slot_rest`) puts it in the
        // original's slot.
        let index = w.bodies.len() as u32;
        if gi == 0
            && let Some(s) = w.slots.get_mut(usize::from(o.slot))
        {
            s.body = index;
        }
        if Some(gi) == shot_group
            && !slots.rest.is_empty()
            && let Some(s) = w.shots.get_mut(usize::from(o.slot))
        {
            s.body = index;
        }
        // A launcher's entry is its slot in its group (platformer 0.4.28):
        // Super Mario Bros.' cannon table.
        if spec.kinds[kind].launcher.is_some()
            && let Some(b) = w.bodies.last_mut()
        {
            b.ext[1] = i64::from(o.slot);
        }
        if let Some(o) = orbit {
            for _ in 0..o.parts {
                let part = gp_sim::systems::new_body(spec, w, o.part, x, y, false);
                w.bodies.push(part);
            }
        }
    }
    // A shot group's empty slots hold what their last shots left, and its
    // `turn` says whose turn it is.
    for g in &groups[1.min(groups.len())..] {
        if g.rest.is_empty() {
            continue;
        }
        for slot in 0..g.count {
            if w.shots[usize::from(slot)].body != 0 {
                continue;
            }
            for f in &g.rest {
                let mut bytes = Vec::new();
                for sel in f.slot_selectors(slot) {
                    bytes.push(crate::bodies::read_sel(original, &sel)?);
                }
                let Some(v) = raw(&f.value(&bytes)).map(Fx::from_raw) else {
                    continue;
                };
                let rest = &mut w.shots[usize::from(slot)].rest;
                match f.path.as_str() {
                    "x_frac" => rest[0] = v,
                    "y_frac" => rest[1] = v,
                    _ => rest[2] = v,
                }
            }
        }
        if let Some(turn) = g.turn {
            w.shot_turn = u32::from(crate::bodies::read_byte(original, turn)?);
        }
    }
    // What the original's other slots hold, by the slots' `rest`, for the
    // next body in each: an empty slot's last object, or one the model
    // doesn't carry (an enemy falling defeated), which leaves its values
    // much as they are.
    let mut rest_loaded = 0;
    if !w.slots.is_empty() && !slots.rest.is_empty() {
        for slot in 0..slots.count.min(w.slots.len() as u16) {
            let taken = w.slots[usize::from(slot)].body != 0;
            for f in &slots.rest {
                if taken && f.path != "word" {
                    continue;
                }
                let mut bytes = Vec::new();
                for sel in f.slot_selectors(slot) {
                    bytes.push(crate::bodies::read_sel(original, &sel)?);
                }
                let Some(v) = raw(&f.value(&bytes)).map(Fx::from_raw) else {
                    continue;
                };
                let s = &mut w.slots[usize::from(slot)];
                match f.path.as_str() {
                    "x_frac" => s.rest[0] = v,
                    "y_frac" => s.rest[2] = v,
                    "word" => s.word = v.floor(),
                    _ => s.rest[1] = v,
                }
            }
            rest_loaded += i32::from(!taken);
        }
    }
    // A second part's slot (`part_flag`) stays taken, by the body in the
    // slot its flag names, for an entity with `extra_slots`.
    if let Some(bit) = slots.part_flag {
        for slot in 0..slots.count.min(w.slots.len() as u16) {
            let flag = slots.flag_of(slot, |a| crate::bodies::read_byte(original, a))?;
            if flag & bit == 0 || w.slots[usize::from(slot)].body != 0 {
                continue;
            }
            let body = w.slots.get(usize::from(flag & !bit)).map_or(0, |s| s.body);
            if let Some(b) = (body as usize).checked_sub(1).map(|i| &w.bodies[i])
                && spec.kinds[b.kind].extra_slots > 0
            {
                w.slots[usize::from(slot)].body = body;
            }
        }
    }
    // The original's enemies run on the slots' clock: the model's frame
    // count takes it as its low byte, as `gp interp slots` does, so the
    // components that move on some frames (`every`) move on the
    // original's. The count only goes forward.
    // Slots with no clock (`slots.no_clock`) leave the count as it is.
    if let Some(at) = slots.clock {
        let clock = crate::bodies::read_byte(original, at)?;
        let moved = clock.wrapping_sub(w.frame as u8);
        w.frame += u64::from(moved);
        let frame = w.frame;
        t.notes.retain(|n| n.path != "frame");
        t.notes.push(note(
            "frame",
            LoadHow::Approximated,
            format!(
                "the model's own frame count moved on {moved} to {frame}, its low byte the original's clock ({clock}, the slots' `clock`)"
            ),
        ));
    }
    // The entities' notes say what was carried.
    let named: Vec<&str> = groups
        .iter()
        .flat_map(|g| &g.kinds)
        .map(|k| k.entity.as_str())
        .chain(slots.held.iter().map(|h| h.entity.as_str()))
        .collect();
    t.notes.retain(|n| {
        !n.path
            .strip_prefix("entities.")
            .and_then(|p| p.strip_suffix("[*]"))
            .is_some_and(|k| named.contains(&k))
    });
    for k in 0..spec.kinds.len() {
        let here: Vec<String> = carried
            .iter()
            .filter(|c| c.0 == k)
            .map(|c| crate::mapping::show(&Value::Fixed(c.1.raw())))
            .collect();
        if here.is_empty() {
            continue;
        }
        let kind = &spec.kinds[k];
        let mut not: Vec<String> = short
            .iter()
            .filter(|s| s.0 == k)
            .map(|s| format!("{} ({})", s.1, s.2))
            .collect();
        not.dedup();
        let not = if not.is_empty() {
            String::new()
        } else {
            format!("; as noted: {}", not.join(", "))
        };
        t.notes.push(note(
            format!("entities.{}[*]", kind.name),
            LoadHow::Approximated,
            format!(
                "{} from the original's slots, at x {}; what the slots don't hold is as a new one has it{not}",
                here.len(),
                here.join(", ")
            ),
        ));
    }
    t.notes.push(note(
        "entities (slots)",
        LoadHow::Approximated,
        format!(
            "the model's own {gone} entities here went; {} of the original's slot objects carried over",
            carried.len()
        ),
    ));
    if !w.slots.is_empty() {
        t.notes.push(note(
            "slots",
            if slots.rest.is_empty() {
                LoadHow::Missing
            } else {
                LoadHow::Loaded
            },
            format!(
                "the carried bodies in their original slots; what the other {rest_loaded} slots hold, by the slots' rest, as their last objects left"
            ),
        ));
    }
    Ok(())
}

/// A carried body at an end of its path heads for the other end, as the
/// path itself has it: at the far end it is on its way back (waiting there
/// first, for its `wait`), and at the spawn end on its way out. Between the
/// ends it keeps the way the slots give (`path_back`).
fn at_path_end(spec: &Spec, b: &mut gp_sim::world::Body) {
    let Some(p) = &spec.kind(b).path else {
        return;
    };
    let axis = usize::from(p.vertical);
    let pos = if p.vertical { b.y } else { b.x };
    if pos == b.home[axis].add(p.distance) {
        b.path_back = true;
    } else if pos == b.home[axis] {
        b.path_back = false;
    }
}

/// Writes the mapped `values` (the original's, read with
/// [`read_original`]) into a copy of `base`, after starting `level` if it
/// isn't the base's.
pub fn translate(
    spec: &Spec,
    base: &World,
    mapping: &Mapping,
    values: &[Value],
    level: Option<(String, &'static str)>,
) -> Result<Translation, String> {
    let mut w = base.clone();
    let mut notes = Vec::new();
    match level {
        Some((name, from)) => {
            let i = spec.levels.index_of(&name).ok_or(format!(
                "the original is in `{name}`, which the model hasn't"
            ))?;
            if i == w.level {
                notes.push(note(
                    "level",
                    LoadHow::Loaded,
                    format!("{name}, from {from}"),
                ));
            } else {
                start_level(spec, &mut w, i);
                notes.push(note(
                    "level",
                    LoadHow::Approximated,
                    format!(
                        "{name}, from {from}, started afresh: what the mapping doesn't hold is as the level starts"
                    ),
                ));
            }
        }
        None => notes.push(note(
            "level",
            LoadHow::Missing,
            format!(
                "the mapping doesn't say which level the original is in; kept the model's ({})",
                spec.levels[w.level].name
            ),
        )),
    }
    for (f, v) in mapping.fields.iter().zip(values) {
        if f.path == "level" {
            continue;
        }
        match set(spec, &mut w, mapping, f, v, &mut notes) {
            Ok(None) => notes.push(note(&f.path, LoadHow::Loaded, crate::mapping::show(v))),
            Ok(Some(why)) => notes.push(note(&f.path, LoadHow::Approximated, why)),
            Err(why) => notes.push(note(&f.path, LoadHow::Missing, why)),
        }
    }
    // A camera by_move reads where its entity was in the view; unmapped,
    // that is where the loaded positions put it.
    for (i, c) in spec.cameras.iter().enumerate() {
        let p = format!("cameras.{}.seen", c.name);
        let mapped = |a: &str| mapping.fields.iter().any(|f| f.path == format!("{p}_{a}"));
        if c.by_move.is_some() && !(mapped("x") && mapped("y")) {
            let kept = w.seen.get(i).copied();
            gp_sim::systems::see(spec, &mut w, i);
            for (a, axis) in ["x", "y"].into_iter().enumerate() {
                if mapped(axis) {
                    if let Some(k) = kept {
                        w.seen[i][a] = k[a];
                    }
                } else {
                    notes.push(note(
                        format!("{p}_{axis}"),
                        LoadHow::Approximated,
                        "from the followed entity's and the camera's places as loaded",
                    ));
                }
            }
        }
    }
    // A camera with keep_move measures its entity's next move from where
    // the entity is as loaded.
    for (i, c) in spec.cameras.iter().enumerate() {
        if !c.by_move.is_some_and(|m| m.keep_move) {
            continue;
        }
        let at = w
            .of_kind(c.follow)
            .next()
            .map(|(_, b)| [b.x.floor(), b.y.floor()]);
        if let Some(pos) = at {
            w.follow_at
                .resize(spec.cameras.len().max(w.follow_at.len()), [0; 2]);
            w.follow_at[i] = pos;
        }
    }
    // A path noted already (a machine's time, by its state) isn't again.
    let extra: Vec<LoadNote> = unmapped(spec, &w, mapping)
        .into_iter()
        .filter(|n| !notes.iter().any(|m| m.path == n.path))
        .collect();
    notes.extend(extra);
    Ok(Translation { world: w, notes })
}

/// Starts level `i` as `next-level` does, its events dropped.
fn start_level(spec: &Spec, w: &mut World, i: usize) {
    w.level = i;
    w.home_level = i;
    let mut f = gp_sim::systems::Frame::default();
    gp_sim::systems::start_level(spec, w, &mut f);
}

/// A number in the model's units, as raw fixed-point.
pub(crate) fn raw(v: &Value) -> Option<i64> {
    match v {
        Value::Fixed(r) => Some(*r),
        Value::Int(n) => Some(n << 16),
        _ => None,
    }
}

/// Sets one mapped path in the world. `Ok(Some(why))` when the value is
/// taken only in part; `Err(why)` when the path can't be set.
/// Notes for other paths it sets go to `notes`.
fn set(
    spec: &Spec,
    w: &mut World,
    mapping: &Mapping,
    f: &Field,
    v: &Value,
    notes: &mut Vec<LoadNote>,
) -> Result<Option<String>, String> {
    let path = gp_model::Path::parse(&f.path)?;
    let seg = &path.segments;
    let name = |i: usize| seg.get(i).map(|s| s.name.as_str());
    if let Value::Text(t) = v
        && t.starts_with("(unnamed value")
    {
        return Err(format!(
            "the original holds {t}, which the mapping names nothing"
        ));
    }
    let fixed = |now: Fx| fixed_of(f, v, now);
    let int = |what: &str| int_of(v, what);
    match (name(0), name(1), name(2)) {
        // A body's animation is the presentation's, not the world's: the
        // screen measure loads it (`Screens::animation`).
        (Some("present"), Some(k), Some(_)) => {
            if spec.kinds.iter().any(|x| x.name == k) {
                Ok(None)
            } else {
                Err(format!("the model has no entity `{k}`"))
            }
        }
        // An input action held on the original's frame (schema 9): the
        // model's next frame counts it as held the frame before, so a
        // press the original doesn't see as new isn't new in the model.
        (Some("input"), Some(a), None) => {
            let i = spec
                .actions
                .iter()
                .position(|x| x.name == a)
                .ok_or(format!("the model has no input action `{a}`"))?;
            let held = match v {
                Value::Flag(b) => *b,
                other => int_of(other, "an action held")?.0 != 0,
            };
            let mask = spec.actions[i].mask;
            if held {
                w.held |= mask;
            } else {
                w.held &= !mask;
            }
            Ok(None)
        }
        (Some("counters"), Some(c), None) => {
            let i = spec
                .counters
                .index_of(c)
                .ok_or(format!("the model has no counter `{c}`"))?;
            let (n, why) = int("a counter")?;
            w.counters[i] = n;
            Ok(why)
        }
        (Some("ticks"), Some(c), None) => {
            let i = spec
                .counters
                .index_of(c)
                .ok_or(format!("the model has no counter `{c}`"))?;
            let Some(gp_model::path::Index::At(k)) = seg[1].index else {
                return Err("a counter's ticks path names one `every` change".into());
            };
            let k = usize::try_from(k).map_err(|_| "a bad index")?;
            let (tick, every) =
                tick_of(spec, i, k).ok_or(format!("counter `{c}` has no `every` change {k}"))?;
            let (n, why) = int("a count of frames")?;
            // A count at or past the last frame before the change changes
            // on the next frame either way; one below zero is the start.
            let top = i64::from(every.max(1) - 1);
            w.ticks[tick] = u32::try_from(n.clamp(0, top)).expect("in range");
            if n < 0 {
                return Ok(Some(format!(
                    "{n} is below zero; loaded 0, the count just after a change"
                )));
            }
            Ok(why)
        }
        (Some("cameras"), Some(c), Some(axis)) => {
            let i = spec
                .cameras
                .index_of(c)
                .ok_or(format!("the model has no camera `{c}`"))?;
            let origin = spec.levels[w.level].origin;
            if axis == "hold" {
                if w.camera_hold.len() < spec.cameras.len() {
                    w.camera_hold.resize(spec.cameras.len(), 0);
                }
                let (n, why) = int("hold")?;
                w.camera_hold[i] =
                    u32::try_from(n).map_err(|_| "a camera's hold is a count of frames")?;
                return Ok(why);
            }
            if let Some(a) = ["move_x", "move_y"].iter().position(|s| *s == axis) {
                if w.last_move.len() < spec.cameras.len() {
                    w.last_move.resize(spec.cameras.len(), [0; 2]);
                }
                let (n, why) = int("a move in whole pixels")?;
                w.last_move[i][a] = n;
                return Ok(why);
            }
            if let Some(a) = ["seen_x", "seen_y"].iter().position(|s| *s == axis) {
                if w.seen.len() < spec.cameras.len() {
                    w.seen.resize(spec.cameras.len(), [Fx::ZERO; 2]);
                }
                let (x, why) = fixed(w.seen[i][a])?;
                w.seen[i][a] = Fx::from_int(x.floor());
                return Ok(why);
            }
            let (a, stage) = match axis {
                "x" => (0, false),
                "y" => (1, false),
                "stage_x" => (0, true),
                "stage_y" => (1, true),
                _ => return Err(format!("a camera has no `{axis}`")),
            };
            let now = if stage {
                w.cameras[i][a].add(origin[a])
            } else {
                w.cameras[i][a]
            };
            let (x, why) = fixed(now)?;
            w.cameras[i][a] = if stage { x.sub(origin[a]) } else { x };
            Ok(why)
        }
        (Some("rules"), Some(m), Some(field)) => {
            let i = spec
                .machines
                .index_of(m)
                .ok_or(format!("the model has no rule `{m}`"))?;
            match field {
                "state" => {
                    let Value::Text(s) = v else {
                        return Err("a rule's state is a name".into());
                    };
                    let st = spec.machines[i]
                        .states
                        .iter()
                        .position(|x| &x.name == s)
                        .ok_or(format!("rule `{m}` has no state `{s}`"))?;
                    if w.machines[i].state != st {
                        w.machines[i].state = st;
                        if entered(mapping, &format!("rules.{m}"), s, notes) {
                            w.machines[i].time = 0;
                        }
                    }
                    Ok(None)
                }
                "time" => {
                    let (n, why) = int("a rule's time")?;
                    w.machines[i].time = u32::try_from(n).map_err(|_| "a time isn't negative")?;
                    Ok(why)
                }
                _ => Err(format!("a rule has no settable `{field}`")),
            }
        }
        (Some("entities"), Some(k), Some(field)) => {
            let kind = spec
                .kinds
                .index_of(k)
                .ok_or(format!("the model has no entity `{k}`"))?;
            let Some(gp_model::path::Index::At(n)) = seg[1].index else {
                return Err("an entity path names one instance".into());
            };
            let id = w
                .of_kind(kind)
                .nth(usize::try_from(n).map_err(|_| "a bad index")?)
                .map(|(id, _)| id)
                .ok_or(format!(
                    "the model's world has no {k}[{n}] here: the mapping can't add bodies yet"
                ))?;
            let origin = spec.levels[w.level].origin;
            let index = match seg[2].index {
                Some(gp_model::path::Index::At(i)) => Some(i),
                Some(_) => return Err("a mapped path names one value".into()),
                None => None,
            };
            let at = f.path.rsplit_once('.').map_or(f.path.as_str(), |p| p.0);
            set_body(
                spec,
                mapping,
                notes,
                &mut w.bodies[id],
                (field, index),
                f,
                v,
                origin,
                at,
            )
        }
        _ => Err(format!("`{}` can't be loaded yet", f.path)),
    }
}

/// Loads one of a body's fields, `field` with its index (`ext[2]`), from
/// the original's value `v` read through `f`. `at` is the body's own path,
/// for the notes a machine state's time takes. A mapped entity and a
/// carried slot object both load this way.
#[allow(clippy::too_many_arguments)]
pub(crate) fn set_body(
    spec: &Spec,
    mapping: &Mapping,
    notes: &mut Vec<LoadNote>,
    b: &mut gp_sim::world::Body,
    (field, index): (&str, Option<i64>),
    f: &Field,
    v: &Value,
    origin: [Fx; 2],
    at: &str,
) -> Result<Option<String>, String> {
    let fixed = |now: Fx| fixed_of(f, v, now);
    let int = |what: &str| int_of(v, what);
    if index.is_some() && field != "ext" {
        return Err(format!("an entity's `{field}` has no index"));
    }
    let flag = || match v {
        Value::Flag(x) => Ok(*x),
        _ => Err(format!("`{field}` takes a flag")),
    };
    let mut why = None;
    match field {
        "x" | "y" | "vx" | "vy" | "stage_x" | "stage_y" => {
            let (slot, off) = match field {
                "x" => (&mut b.x, Fx::ZERO),
                "y" => (&mut b.y, Fx::ZERO),
                "vx" => (&mut b.vx, Fx::ZERO),
                "vy" => (&mut b.vy, Fx::ZERO),
                "stage_x" => (&mut b.x, origin[0]),
                _ => (&mut b.y, origin[1]),
            };
            let (x, w) = fixed(slot.add(off))?;
            *slot = x.sub(off);
            why = w;
        }
        "vx_frac" => {
            let (x, w) = fixed(b.vx_frac)?;
            b.vx_frac = x;
            why = w;
        }
        "fall_gravity" => {
            let (x, w) = fixed(b.fall_gravity)?;
            b.fall_gravity = x;
            why = w;
        }
        "seen_speed" => {
            let (x, w) = fixed(b.seen_speed)?;
            b.seen_speed = x;
            why = w;
        }
        "moved" => match v {
            Value::Text(t) if t != "both" || crate::state::faces_both(&spec.kinds[b.kind]) => {
                b.moved = crate::state::MOVED
                    .iter()
                    .position(|m| m == t)
                    .ok_or("moved is \"none\", \"right\" or \"left\"")?
                    as u8
            }
            _ => return Err("moved is \"none\", \"right\" or \"left\"".into()),
        },
        "run_hold" => {
            let (n, w) = int("run_hold")?;
            b.run_hold = u32::try_from(n).map_err(|_| "run_hold is a count of frames")?;
            why = w;
        }
        "decel_hold" => b.decel_hold = flag()?,
        "pushed" => match v {
            Value::Text(t) => {
                b.pushed = crate::state::MOVED[..3]
                    .iter()
                    .position(|m| m == t)
                    .ok_or("pushed is \"none\", \"right\" or \"left\"")?
                    as u8
            }
            _ => return Err("pushed is \"none\", \"right\" or \"left\"".into()),
        },
        "on_ground" => b.on_ground = flag()?,
        // Feature `entity.jumping`: a body in a jump's air, which a start
        // inside a jump needs so the arc goes on from its speed.
        "jumping" => {
            if flag()? {
                b.air = u32::MAX;
                b.on_ground = false;
            } else if b.air == u32::MAX {
                b.air = 0;
            }
        }
        "alive" => b.alive = flag()?,
        "waiting" => b.waiting = flag()?,
        "climbing" => b.climbing = flag()?,
        "facing" => match v {
            Value::Text(t) if t == "left" => (b.facing_left, b.faced_both) = (true, false),
            Value::Text(t) if t == "right" => (b.facing_left, b.faced_both) = (false, false),
            Value::Text(t) if t == "both" && crate::state::faces_both(&spec.kinds[b.kind]) => {
                (b.facing_left, b.faced_both) = (false, true)
            }
            _ => return Err("facing is \"left\" or \"right\"".into()),
        },
        "time" => {
            let (n, w) = int("a state's time")?;
            b.time = u32::try_from(n).map_err(|_| "a time isn't negative")?;
            why = w;
        }
        "hp" => {
            let (n, w) = int("hp")?;
            b.hp = n;
            why = w;
        }
        "state" => {
            let Value::Text(s) = v else {
                return Err("a state is a name".into());
            };
            let m = spec.kinds[b.kind]
                .machine
                .as_ref()
                .ok_or(format!("`{}` has no machine", spec.kinds[b.kind].name))?;
            let st = m
                .states
                .iter()
                .position(|x| &x.name == s)
                .ok_or(format!("`{}` has no state `{s}`", spec.kinds[b.kind].name))?;
            // What the state's `set` changes (a hitbox, a sprite)
            // is the state's own kind, so entering it applies it.
            if b.state != st {
                b.state = st;
                if entered(mapping, at, s, notes) {
                    b.time = 0;
                }
            }
        }
        "ext" => {
            let i = index
                .and_then(|i| usize::try_from(i).ok())
                .filter(|i| *i < b.ext.len())
                .ok_or("ext takes an index, 0 to 3")?;
            let (n, w) = int("a mechanic word")?;
            b.ext[i] = n;
            why = w;
        }
        "home_x" | "home_y" => {
            let a = usize::from(field == "home_y");
            let (x, w) = fixed(b.home[a])?;
            b.home[a] = x;
            why = w;
        }
        "path_back" => b.path_back = flag()?,
        "wait" => {
            let (n, w) = int("wait")?;
            // Below zero is done waiting, as 0 is.
            b.wait = u32::try_from(n.max(0)).map_err(|_| "wait is a count of frames")?;
            why = w;
        }
        _ => return Err(format!("an entity's `{field}` can't be loaded yet")),
    }
    Ok(why)
}

/// A fixed-point value from the original's, for a model value now `now`:
/// a whole field keeps the model's fraction under the original's whole
/// part, since the original keeps none, unless it maps the fraction too
/// (`frac`), which then loads.
fn fixed_of(f: &Field, v: &Value, now: Fx) -> Result<(Fx, Option<String>), String> {
    let r = raw(v).ok_or("the original's value isn't a number")?;
    if f.whole && f.frac.is_none() {
        let whole = r >> 16;
        let kept = Fx::from_raw((whole << 16) + now.frac());
        let why = (now.frac() != 0).then(|| {
            format!(
                "{whole}, the original's whole part, with the model's fraction ({kept}): the original keeps none"
            )
        });
        Ok((kept, why))
    } else {
        Ok((Fx::from_raw(r), None))
    }
}

/// A whole number from the original's value, rounded down.
fn int_of(v: &Value, what: &str) -> Result<(i64, Option<String>), String> {
    if let Value::Int(n) = v {
        return Ok((*n, None));
    }
    let r = raw(v).ok_or(format!("{what} takes a number"))?;
    let why = (r & 0xFFFF != 0).then(|| {
        format!(
            "{} rounded down: the model keeps {what} whole",
            Fx::from_raw(r)
        )
    });
    Ok((r >> 16, why))
}

/// Notes a machine's time when its state `s` was entered by a translation:
/// approximated as 0, unless the mapping holds the time (`at.time`), which
/// sets it in whichever order the fields come. True when the time is 0.
fn entered(mapping: &Mapping, at: &str, s: &str, notes: &mut Vec<LoadNote>) -> bool {
    let p = format!("{at}.time");
    if mapping.fields.iter().any(|f| f.path == p) {
        return false;
    }
    notes.push(note(
        p,
        LoadHow::Approximated,
        format!("0: {s} entered now, since the original's time in it isn't mapped"),
    ));
    true
}

/// The index into [`World::ticks`] of counter `counter`'s `k`-th `every`
/// change, and its period.
fn tick_of(spec: &Spec, counter: usize, k: usize) -> Option<(usize, u32)> {
    let before: usize = spec.counters[..counter]
        .iter()
        .map(crate::state::every_changes)
        .sum();
    spec.counters[counter]
        .changes
        .iter()
        .filter_map(|ch| match ch {
            gp_sim::spec::Change::Every(every, _) => Some(*every),
            gp_sim::spec::Change::On(..) | gp_sim::spec::Change::When(_) => None,
        })
        .nth(k)
        .map(|every| (before + k, every))
}

/// Fields a body's view shows that are another's view (`stage_x` of `x`)
/// or its identity, never loaded on their own.
const DERIVED: [&str; 3] = ["id", "stage_x", "stage_y"];

/// Notes for every part of the world the mapping doesn't hold.
fn unmapped(spec: &Spec, w: &World, mapping: &Mapping) -> Vec<LoadNote> {
    let mapped = |p: &str| mapping.fields.iter().any(|f| f.path == p);
    let kept = "not in the mapping; kept the model's own";
    let mut out = vec![
        note(
            "frame",
            LoadHow::Approximated,
            format!(
                "the model's own frame count ({}), the frames it has run in step with the original",
                w.frame
            ),
        ),
        note(
            "rng",
            LoadHow::Missing,
            "the original's random state isn't mapped; kept the model's generator",
        ),
    ];
    let kinds_mapped = |k: &str| {
        mapping
            .fields
            .iter()
            .any(|f| f.path.starts_with(&format!("entities.{k}[")))
    };
    for k in 0..spec.kinds.len() {
        let n = w.of_kind(k).count();
        if n == 0 {
            continue;
        }
        let kind = &spec.kinds[k];
        if !kinds_mapped(&kind.name) {
            out.push(note(
                format!("entities.{}[*]", kind.name),
                LoadHow::Missing,
                format!("{n} in the model's world here; the mapping holds none of them, so they're as the model has them"),
            ));
            continue;
        }
        let mut fields = vec![
            "x",
            "y",
            "vx",
            "vy",
            "on_ground",
            "facing",
            "alive",
            "waiting",
            "hp",
            "climbing",
        ];
        if kind.machine.is_some() {
            fields.extend(["state", "time"]);
        }
        if crate::state::runs(kind) {
            fields.extend(["vx_frac", "moved", "run_hold", "decel_hold"]);
        }
        if crate::state::masks_pushes(kind) {
            fields.push("pushed");
        }
        if crate::state::keeps_fall_gravity(kind) {
            fields.push("fall_gravity");
        }
        if crate::state::sees_speed_before_walls(kind) {
            fields.push("seen_speed");
        }
        for i in 0..n {
            for field in &fields {
                let p = format!("entities.{}[{i}].{field}", kind.name);
                let stage = format!("entities.{}[{i}].stage_{field}", kind.name);
                if !mapped(&p) && !mapped(&stage) && !DERIVED.contains(field) {
                    out.push(note(p, LoadHow::Missing, kept));
                }
            }
        }
    }
    for c in &spec.cameras {
        for axis in ["x", "y"] {
            let p = format!("cameras.{}.{axis}", c.name);
            if !mapped(&p) && !mapped(&format!("cameras.{}.stage_{axis}", c.name)) {
                out.push(note(p, LoadHow::Missing, kept));
            }
        }
    }
    for m in &spec.machines {
        for field in ["state", "time"] {
            let p = format!("rules.{}.{field}", m.name);
            if !mapped(&p) {
                out.push(note(p, LoadHow::Missing, kept));
            }
        }
    }
    for c in &spec.counters {
        let p = format!("counters.{}", c.name);
        if !mapped(&p) {
            out.push(note(p, LoadHow::Missing, kept));
        }
        for k in 0..crate::state::every_changes(c) {
            let p = format!("ticks.{}[{k}]", c.name);
            if !mapped(&p) {
                out.push(note(
                    p,
                    LoadHow::Missing,
                    "the frames counted toward this `every` change aren't mapped; kept the model's own, so it changes on the model's count",
                ));
            }
        }
    }
    out.push(note(
        "(mechanic state)",
        LoadHow::Missing,
        "what the state view doesn't name (jump bands and buffers, timers, hook words) is the model's own",
    ));
    out.push(note(
        format!("levels.{}.tiles", spec.levels[w.level].name),
        LoadHow::Missing,
        format!(
            "the original's changed cells (blocks hit, coins taken) aren't mapped; kept the model's {} changed cells",
            w.tiles.len()
        ),
    ));
    out
}
