//! `gp model static --model`'s model.toml in the model format, so
//! `gp model propose`, `gp model refine` and the rest read it
//! (`gp_model::Model::parse`). The harvest names its facts in a tree of its
//! own; this puts the ones the format has a place for in the format's
//! shapes, writes a header, and leaves out every fact the format has no
//! place for, naming each with the parser's reason. Nothing is made up: a
//! fact the format can't hold (a RAM address, a port, a slot table's
//! stride) is left out, not guessed into a field.
//!
//! The shapes moved: a machine's transitions are a list with `to` (the
//! harvest keys them by target), a level's spawns a list with `entity` (the
//! harvest keys them by entity), and a jump's or a run's button becomes an
//! action in `[input.actions]` (the harvest names the button). The header
//! is the directory's own when its model.toml has one (`gp bench scratch`
//! starts from a header), else the console's frame rate and the view the
//! harvest's gravity gives, with any placeholder said in its description.

use std::collections::HashMap;
use std::ops::Range;

use anyhow::{Result, bail};
use serde::Serialize;
use toml::{Table, Value as Toml};

/// The console the ROM is for: its controller and frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Console {
    Nes,
    Arcade,
}

/// A harvest fact the model format has no place for, by its path in the
/// harvest's tree, and the parser's words for why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LeftOut {
    pub path: String,
    pub why: String,
}

/// What `fit` wrote: the model's text, the facts it left out, and the
/// placeholders it put in fields the format needs that the code doesn't
/// give, each with an evidence entry of confidence 0 saying so (as
/// `gp game new`'s skeleton does), or the console's value.
#[derive(Debug, Clone)]
pub(crate) struct Fitted {
    pub text: String,
    pub left_out: Vec<LeftOut>,
    pub placeholders: Vec<LeftOut>,
}

/// A value for a field the format needs and the harvest didn't give, by
/// section (`entities`, `levels`, `cameras`) and field: the value, the
/// evidence's confidence and its note. None leaves the table out.
fn placeholder(section: &str, field: &str, console: Console) -> Option<(Toml, u8, &'static str)> {
    let ints = |v: &[i64]| Toml::Array(v.iter().map(|&i| Toml::Integer(i)).collect());
    const NOTE: &str = "A placeholder from gp model static, gp game new's: the code doesn't give it, so nothing here is measured";
    match (section, field) {
        ("entities", "size") => Some((ints(&[16, 16]), 0, NOTE)),
        ("entities", "hitbox") => {
            let mut r = Table::new();
            for (k, v) in [("x", 0), ("y", 0), ("w", 16), ("h", 16)] {
                r.insert(k.into(), Toml::Integer(v));
            }
            Some((Toml::Table(r), 0, NOTE))
        }
        ("levels", "tile_size") => Some((Toml::Integer(8), 0, NOTE)),
        ("levels", "size") => Some((ints(&[32, 30]), 0, NOTE)),
        ("cameras", "size") if console == Console::Nes => Some((
            ints(&[256, 240]),
            90,
            "The NES's screen, 256 by 240: the console's, not read from this game's code",
        )),
        _ => None,
    }
}

/// The NES's frame rate, the console's (as `gp game new` writes it).
const NES_FRAME_RATE: &str = "39375000/655171";
/// `gp game new`'s arcade placeholder: a board's rate isn't in its code.
const ARCADE_FRAME_RATE: &str = "30";

/// The harvest's tree `doc` as a model.toml the format takes, and what was
/// left out. `old` is the directory's model.toml before, if any: its
/// `[model]` header is kept.
pub(crate) fn fit(
    mut doc: Table,
    name: &str,
    console: Console,
    old: Option<&str>,
) -> Result<Fitted> {
    doc.remove("model");
    let actions = buttons_to_actions(&mut doc);
    for (_, e) in table_mut(&mut doc, "entities") {
        if let Some(m) = e.get_mut("machine").and_then(Toml::as_table_mut) {
            transitions_to_lists(m);
        }
    }
    for (_, r) in table_mut(&mut doc, "rules") {
        if let Some(m) = r.as_table_mut() {
            transitions_to_lists(m);
        }
    }
    for (_, l) in table_mut(&mut doc, "levels") {
        if let Some(l) = l.as_table_mut() {
            spawns_to_list(l);
        }
    }
    if !actions.is_empty() {
        let input = doc
            .entry("input")
            .or_insert_with(|| Toml::Table(Table::new()));
        if let Some(t) = input.as_table_mut() {
            t.insert(
                "device".into(),
                Toml::String(
                    match console {
                        Console::Nes => "nes-pad",
                        Console::Arcade => "arcade",
                    }
                    .into(),
                ),
            );
            t.insert("actions".into(), Toml::Table(actions));
        }
    }
    let kept = old
        .and_then(|t| toml::from_str::<Table>(t).ok())
        .is_some_and(|t| t.get("model").is_some_and(Toml::is_table));
    let mut declared = false;
    let head = header(name, console, has_gravity(&doc), old);
    doc.insert("model".into(), Toml::Table(head));
    let mut left = Vec::new();
    let mut placed = Vec::new();
    let mut evidence = Table::new();
    // Each refusal names one place; at most one pass a fact.
    for _ in 0..10_000 {
        // A section all of whose facts were left out says nothing.
        doc.retain(|_, v| v.as_table().is_none_or(|t| !t.is_empty()));
        let text = toml::to_string(&doc)?;
        let e = match toml::from_str::<gp_model::Model>(&text) {
            Ok(_) => {
                if !declared && uses_platformer(&doc) {
                    // The platformer's components need its extension
                    // declared (gp model check); a kept header is the
                    // directory's to say.
                    declared = true;
                    if let Some(h) = doc.get_mut("model").and_then(Toml::as_table_mut)
                        && !kept
                    {
                        let mut x = Table::new();
                        x.insert(
                            "platformer".into(),
                            Toml::String(gp_model::platformer::VERSION.into()),
                        );
                        h.insert("extensions".into(), Toml::Table(x));
                        continue;
                    }
                }
                if let Err(e) = gp_model::Model::parse(&text) {
                    bail!("the model gp model static wrote is refused: {e}");
                }
                return Ok(Fitted {
                    text,
                    left_out: left,
                    placeholders: placed,
                });
            }
            Err(e) => e,
        };
        let Some(span) = e.span() else {
            bail!(
                "the model gp model static wrote is refused, at no place: {}",
                e.message()
            );
        };
        let mut path = path_at(&text, span.start);
        let mut why = e.message().to_owned();
        // An entity's or a level's error names the whole table (its parts
        // are read apart, core and extension): read the parts here to find
        // the place.
        if let [Seg::Key(section), Seg::Key(name)] = path.as_slice()
            && let Some(t) = doc
                .get(section.as_str())
                .and_then(|s| s.get(name.as_str()))
                .and_then(Toml::as_table)
            && let Some((rel, w)) = locate(section, t)
        {
            path.extend(rel);
            why = w;
        }
        // An unknown key the error places at its table.
        if let Some(k) = why
            .strip_prefix("unknown field `")
            .and_then(|m| m.split('`').next())
            && node(&doc, &path)
                .and_then(Toml::as_table)
                .is_some_and(|t| t.contains_key(k))
        {
            path.push(Seg::Key(k.to_owned()));
        }
        if path.is_empty() || matches!(&path[0], Seg::Key(k) if k == "model") {
            bail!("the model gp model static wrote is refused in its header: {why}");
        }
        // A field the format needs on an entity, a level or a camera the
        // harvest found: a placeholder marked as one.
        if let (Some(field), [Seg::Key(section), Seg::Key(name)]) = (
            why.strip_prefix("missing field `")
                .and_then(|m| m.split('`').next()),
            path.as_slice(),
        ) && let Some((v, confidence, note)) = placeholder(section, field, console)
            && let Some(t) = doc
                .get_mut(section.as_str())
                .and_then(|s| s.get_mut(name.as_str()))
                .and_then(Toml::as_table_mut)
            && !t.contains_key(field)
        {
            t.insert(field.to_owned(), v);
            let at = format!("{section}.{name}.{field}");
            let mut ev = Table::new();
            ev.insert("source".into(), Toml::String("inferred".into()));
            ev.insert("confidence".into(), Toml::Integer(confidence.into()));
            ev.insert("note".into(), Toml::String(note.into()));
            evidence.insert(at.clone(), Toml::Table(ev));
            doc.insert("evidence".into(), Toml::Table(evidence.clone()));
            placed.push(LeftOut {
                path: at,
                why: note.to_owned(),
            });
            continue;
        }
        if !remove(&mut doc, &path) {
            bail!(
                "the model gp model static wrote is refused at {}, which isn't in it: {why}",
                display(&path)
            );
        }
        // A list the harvest had items in, left with none, would say there
        // are none: it goes too.
        if let Some((Seg::Index(_), list)) = path.split_last()
            && node(&doc, list)
                .and_then(Toml::as_array)
                .is_some_and(Vec::is_empty)
        {
            remove(&mut doc, list);
        }
        left.push(LeftOut {
            path: display(&path),
            why: why.lines().next().unwrap_or_default().to_owned(),
        });
    }
    bail!("the model gp model static wrote is still refused after 10000 facts left out")
}

/// Where in an entity's or a level's table `t` the format refuses it, and
/// why: each part read on its own as the model's reader splits it (core
/// fields, the top-down extension's, the platformer's).
fn locate(section: &str, t: &Table) -> Option<(Vec<Seg>, String)> {
    use gp_model::{model, platformer, topdown};
    let (core, td) = match section {
        "entities" => (
            model::ENTITY_CORE_FIELDS,
            topdown::EXTENSION.entity_components,
        ),
        "levels" => (model::LEVEL_CORE_FIELDS, topdown::EXTENSION.level_fields),
        _ => return None,
    };
    let part = |keep: &dyn Fn(&str) -> bool| -> Table {
        t.iter()
            .filter(|(k, _)| keep(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    let is_core = |k: &str| core.contains(&k);
    let is_td = |k: &str| !core.contains(&k) && td.contains(&k);
    let is_pf = |k: &str| !core.contains(&k) && !td.contains(&k);
    let parts = [part(&is_core), part(&is_td), part(&is_pf)];
    for (i, p) in parts.iter().enumerate() {
        // A blank first line: a span at 0 is then the part's own (a field
        // it lacks), not its first table's.
        let text = format!("\n{}", toml::to_string(p).ok()?);
        let err = match (section, i) {
            ("entities", 0) => toml::from_str::<model::EntityCore>(&text).err(),
            ("entities", 1) => toml::from_str::<topdown::EntityComponents>(&text).err(),
            ("entities", _) => toml::from_str::<platformer::EntityComponents>(&text).err(),
            (_, 0) => toml::from_str::<model::LevelCore>(&text).err(),
            (_, 1) => toml::from_str::<topdown::LevelFields>(&text).err(),
            (_, _) => toml::from_str::<platformer::LevelFields>(&text).err(),
        };
        if let Some(e) = err {
            let rel = e
                .span()
                .filter(|s| s.start > 0)
                .map(|s| path_at(&text, s.start))
                .unwrap_or_default();
            return Some((rel, e.message().to_owned()));
        }
    }
    None
}

/// Whether an entity or a level holds a key that is the platformer's (not
/// a core field nor the top-down extension's).
fn uses_platformer(doc: &Table) -> bool {
    use gp_model::{model, topdown};
    let any = |section: &str, core: &[&str], td: &[&str]| {
        doc.get(section).and_then(Toml::as_table).is_some_and(|s| {
            s.values().filter_map(Toml::as_table).any(|t| {
                t.keys()
                    .any(|k| !core.contains(&k.as_str()) && !td.contains(&k.as_str()))
            })
        })
    };
    any(
        "entities",
        model::ENTITY_CORE_FIELDS,
        topdown::EXTENSION.entity_components,
    ) || any(
        "levels",
        model::LEVEL_CORE_FIELDS,
        topdown::EXTENSION.level_fields,
    )
}

/// The node at `path`, if any.
fn node<'a>(doc: &'a Table, path: &[Seg]) -> Option<&'a Toml> {
    let (Seg::Key(first), rest) = path.split_first()? else {
        return None;
    };
    let mut cur = doc.get(first)?;
    for s in rest {
        cur = match (s, cur) {
            (Seg::Key(k), Toml::Table(t)) => t.get(k)?,
            (Seg::Index(i), Toml::Array(a)) => a.get(*i)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// The tables under `doc[key]`, each by name.
fn table_mut<'a>(doc: &'a mut Table, key: &str) -> Vec<(&'a String, &'a mut Toml)> {
    doc.get_mut(key)
        .and_then(Toml::as_table_mut)
        .map(|t| t.iter_mut().collect())
        .unwrap_or_default()
}

/// Each entity's `jump.button` and `run.button` (the harvest's: the button
/// the code tests) as an action the jump or run names: `jump` and `run`,
/// or `ENTITY_jump` when two entities' buttons differ.
fn buttons_to_actions(doc: &mut Table) -> Table {
    let mut actions = Table::new();
    for (ent, e) in table_mut(doc, "entities") {
        let Some(e) = e.as_table_mut() else { continue };
        for (part, field) in [("jump", "action"), ("run", "run_action")] {
            let Some(p) = e.get_mut(part).and_then(Toml::as_table_mut) else {
                continue;
            };
            let Some(button) = p.remove("button") else {
                continue;
            };
            let mut action = part.to_owned();
            if actions.get(&action).is_some_and(|b| b != &button) {
                action = format!("{ent}_{part}");
            }
            actions.insert(action.clone(), button);
            p.entry(field).or_insert(Toml::String(action));
        }
    }
    actions
}

/// A machine's `states.S.transition.T` tables (keyed by target) as the
/// format's list, each with `to = "T"`.
fn transitions_to_lists(machine: &mut Table) {
    let Some(states) = machine.get_mut("states").and_then(Toml::as_table_mut) else {
        return;
    };
    for (_, s) in states.iter_mut() {
        let Some(s) = s.as_table_mut() else { continue };
        let Some(Toml::Table(by_to)) = s.get("transition").cloned() else {
            continue;
        };
        let list = by_to
            .into_iter()
            .map(|(to, t)| {
                let mut t = match t {
                    Toml::Table(t) => t,
                    other => Table::from_iter([("value".to_owned(), other)]),
                };
                t.insert("to".into(), Toml::String(to));
                Toml::Table(t)
            })
            .collect();
        s.insert("transition".into(), Toml::Array(list));
    }
}

/// A level's `spawn.E` tables (keyed by entity) as the format's list, each
/// with `entity = "E"`.
fn spawns_to_list(level: &mut Table) {
    let Some(Toml::Table(by)) = level.get("spawn").cloned() else {
        return;
    };
    let list = by
        .into_iter()
        .map(|(ent, s)| {
            let mut s = match s {
                Toml::Table(t) => t,
                other => Table::from_iter([("value".to_owned(), other)]),
            };
            s.entry("entity").or_insert(Toml::String(ent));
            Toml::Table(s)
        })
        .collect();
    level.insert("spawn".into(), Toml::Array(list));
}

/// Whether an entity's jump has a gravity: down is then where it pulls,
/// the format's side view.
fn has_gravity(doc: &Table) -> bool {
    doc.get("entities")
        .and_then(Toml::as_table)
        .is_some_and(|t| {
            t.values()
                .any(|e| e.get("jump").and_then(|j| j.get("gravity")).is_some())
        })
}

/// The directory's header if its model.toml has one, else one from the
/// console, saying which of its values are placeholders.
fn header(name: &str, console: Console, gravity: bool, old: Option<&str>) -> Table {
    if let Some(Toml::Table(h)) = old
        .and_then(|t| toml::from_str::<Table>(t).ok())
        .and_then(|mut t| t.remove("model"))
    {
        return h;
    }
    let mut says = vec!["Written by gp model static from the ROM's code alone.".to_owned()];
    let rate = match console {
        Console::Nes => {
            says.push("The frame rate is the NES's.".into());
            NES_FRAME_RATE
        }
        Console::Arcade => {
            says.push(
                "The frame rate is a placeholder, gp game new's for an arcade board: the code doesn't give it."
                    .into(),
            );
            ARCADE_FRAME_RATE
        }
    };
    if gravity {
        says.push("The side view is from the player's jump, whose gravity pulls down.".into());
    } else {
        says.push(
            "The side view is a placeholder: the harvest found no gravity to say which way is down."
                .into(),
        );
    }
    says.push("The title is the model's name. Facts the model format has no place for (RAM addresses, ports, slot tables) are left out; static.json (--write) has them all.".into());
    let mut h = Table::new();
    h.insert("schema".into(), Toml::String(gp_model::CORE_VERSION.into()));
    h.insert("name".into(), Toml::String(name.into()));
    h.insert("title".into(), Toml::String(name.into()));
    h.insert("description".into(), Toml::String(says.join(" ")));
    h.insert("view".into(), Toml::String("side".into()));
    h.insert("frame_rate".into(), Toml::String(rate.into()));
    h
}

/// One step of a path in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Key(String),
    Index(usize),
}

fn display(path: &[Seg]) -> String {
    let mut s = String::new();
    for p in path {
        match p {
            Seg::Key(k) => {
                if !s.is_empty() {
                    s.push('.');
                }
                s.push_str(k);
            }
            Seg::Index(i) => s.push_str(&format!("[{i}]")),
        }
    }
    s
}

/// Removes the node at `path`; false when there is none.
fn remove(doc: &mut Table, path: &[Seg]) -> bool {
    let Some((last, init)) = path.split_last() else {
        return false;
    };
    let mut cur: &mut Toml = match init.first() {
        None => {
            return match last {
                Seg::Key(k) => doc.remove(k).is_some(),
                Seg::Index(_) => false,
            };
        }
        Some(Seg::Key(k)) => match doc.get_mut(k) {
            Some(v) => v,
            None => return false,
        },
        Some(Seg::Index(_)) => return false,
    };
    for s in &init[1..] {
        let next = match (s, cur) {
            (Seg::Key(k), Toml::Table(t)) => t.get_mut(k),
            (Seg::Index(i), Toml::Array(a)) => a.get_mut(*i),
            _ => None,
        };
        match next {
            Some(n) => cur = n,
            None => return false,
        }
    }
    match (last, cur) {
        (Seg::Key(k), Toml::Table(t)) => t.remove(k).is_some(),
        (Seg::Index(i), Toml::Array(a)) if *i < a.len() => {
            a.remove(*i);
            true
        }
        _ => false,
    }
}

/// The path of the node at byte `at` of `text`, a document `toml` wrote:
/// a table header's key under `at`, or the key of the line `at` is on,
/// below the header before it. Arrays of tables count by their `[[..]]`
/// headers.
fn path_at(text: &str, at: usize) -> Vec<Seg> {
    let mut cur: Vec<Seg> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut found = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        let trimmed = line.trim_start();
        let lead = line.len() - trimmed.len();
        let is_array = trimmed.starts_with("[[");
        let header = trimmed.starts_with('[');
        if header {
            let open = if is_array { 2 } else { 1 };
            let keys = dotted_keys(&trimmed[open..]);
            let mut path = Vec::new();
            let mut cut = None;
            for (j, (k, r)) in keys.iter().enumerate() {
                path.push(Seg::Key(k.clone()));
                let here = display(&path);
                let last = j + 1 == keys.len();
                if last && is_array {
                    let n = seen.entry(here).or_insert(0);
                    *n += 1;
                    path.push(Seg::Index(*n - 1));
                } else if let Some(n) = seen.get(&here) {
                    path.push(Seg::Index(n - 1));
                }
                let r = (start + lead + open + r.start)..(start + lead + open + r.end);
                if r.contains(&at) {
                    cut = Some(path.len());
                }
            }
            if (start..end).contains(&at) {
                if let Some(c) = cut {
                    path.truncate(c);
                }
                return path;
            }
            cur = path;
        } else if trimmed.contains('=') && !trimmed.starts_with('#') {
            let keys = dotted_keys(trimmed);
            if !keys.is_empty() {
                let mut path = cur.clone();
                let mut cut = None;
                for (k, r) in &keys {
                    path.push(Seg::Key(k.clone()));
                    if ((start + lead + r.start)..(start + lead + r.end)).contains(&at) {
                        cut = Some(path.len());
                    }
                }
                if let Some(c) = cut {
                    path.truncate(c);
                }
                found = path;
            }
        }
        if (start..end).contains(&at) {
            return found;
        }
        start = end;
    }
    found
}

/// A dotted key at the start of `s` (bare, "basic" or 'literal' parts),
/// each part with its byte range in `s`. Stops at `]` or `=`.
fn dotted_keys(s: &str) -> Vec<(String, Range<usize>)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        if i >= b.len() {
            return out;
        }
        let from = i;
        let key = match b[i] {
            b'"' => {
                let mut k = String::new();
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        i += 1;
                        match b[i] {
                            b'n' => k.push('\n'),
                            b't' => k.push('\t'),
                            c => k.push(c as char),
                        }
                    } else {
                        let ch = s[i..].chars().next().unwrap_or(' ');
                        k.push(ch);
                        i += ch.len_utf8() - 1;
                    }
                    i += 1;
                }
                i += 1;
                k
            }
            b'\'' => {
                i += 1;
                let st = i;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
                let k = s[st..i.min(b.len())].to_owned();
                i += 1;
                k
            }
            _ => {
                let st = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'-')
                {
                    i += 1;
                }
                if i == st {
                    return out;
                }
                s[st..i].to_owned()
            }
        };
        out.push((key, from..i.min(b.len())));
        while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
        } else {
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(t: &str) -> Table {
        toml::from_str(t).unwrap()
    }

    #[test]
    fn a_header_key_or_a_line_key_is_found_by_its_place() {
        let text = "[a]\nx = 1\n\n[[a.l]]\ny = 2\n\n[[a.l]]\ny = 3\n\n[a.l.m]\nz = 4\n\n[\"q.r\".s]\nw = 5\n";
        let at = |needle: &str| text.find(needle).unwrap();
        assert_eq!(display(&path_at(text, at("x ="))), "a.x");
        assert_eq!(display(&path_at(text, at("3"))), "a.l[1].y");
        assert_eq!(display(&path_at(text, at("m]"))), "a.l[1].m");
        assert_eq!(display(&path_at(text, at("z"))), "a.l[1].m.z");
        assert_eq!(display(&path_at(text, at("s]"))), "q.r.s");
        assert_eq!(display(&path_at(text, at("w"))), "q.r.s.w");
    }

    #[test]
    fn the_harvests_shapes_become_the_formats_and_the_rest_is_left_out() {
        // Made-up facts in the harvest's names and shapes.
        let doc = tree(
            r#"
[cameras.main]
axis = "x"
follow = "player"
lead = [120, 0]
[cameras.main.bytes]
x = "$1A"

[counters.c_07]
bytes = "$07"
start = 0

[entities.player]
role = "player"
[entities.player.bytes]
x = "$10"
[entities.player.jump]
button = "a"
gravity = "0.25"
max_fall = 4
speed = -5
[entities.player.run]
button = "b"
walk_speed = 1
run_speed = 2

[entities.slots_20]
base = "E062"
stride = 2

[hardware]
frame_counter = "$00"

[input]
pad = "$02"

[levels.main.spawn.player]
x = 24
y = 176
[levels.main.spawn.slots_20]
y = 176

[rules.state_05]
initial = "s0"
[rules.state_05.states.s1.transition.s0]
after = 90
[rules.state_05.states.s1.transition.s0.if]
cmp = ">="
counter = "c_07"
value = 1
"#,
        );
        let f = fit(doc, "static", Console::Nes, None).unwrap();
        let (text, left) = (f.text, f.left_out);
        let m = gp_model::Model::parse(&text).unwrap();
        assert_eq!(m.model.name, "static");
        assert_eq!(m.model.frame_rate.to_string(), NES_FRAME_RATE);
        let input = m.input.as_ref().unwrap();
        assert_eq!(
            input.actions.keys().cloned().collect::<Vec<_>>(),
            ["jump", "run"]
        );
        let jump = m.entities["player"].platformer.jump.as_ref().unwrap();
        assert_eq!(jump.action, "jump");
        let t = &m.rules["state_05"].states["s1"].transition;
        assert_eq!((t.len(), t[0].to.as_str(), t[0].after), (1, "s0", Some(90)));
        let spawn = &m.levels["main"].core.spawn;
        assert_eq!(
            spawn.iter().map(|s| s.entity.as_str()).collect::<Vec<_>>(),
            ["player", "slots_20"]
        );
        let paths: Vec<&str> = left.iter().map(|l| l.path.as_str()).collect();
        for p in [
            "cameras.main.bytes",
            "counters.c_07.bytes",
            "entities.player.bytes",
            "entities.slots_20",
            "hardware",
            "input.pad",
        ] {
            assert!(paths.contains(&p), "{p} not left out: {paths:?}");
        }
        assert!(left.iter().all(|l| !l.why.is_empty()));
    }

    #[test]
    fn the_directorys_header_is_kept_and_an_arcade_header_says_its_placeholders() {
        let old = "[model]\nschema = \"0.4\"\nname = \"mine\"\ntitle = \"Mine\"\nview = \"top-down\"\nframe_rate = \"60\"\n";
        let text = fit(Table::new(), "static", Console::Nes, Some(old))
            .unwrap()
            .text;
        let m = gp_model::Model::parse(&text).unwrap();
        assert_eq!(
            (m.model.name.as_str(), m.model.title.as_str()),
            ("mine", "Mine")
        );
        let doc = tree(
            "[counters]\nc_E08E = \"E08E\"\n\n[rules.state_E397]\nstates = [\"s0\", \"s1\"]\n",
        );
        let f = fit(doc, "static", Console::Arcade, None).unwrap();
        let (text, left) = (f.text, f.left_out);
        let m = gp_model::Model::parse(&text).unwrap();
        assert_eq!(m.model.frame_rate.to_string(), ARCADE_FRAME_RATE);
        assert!(m.model.description.unwrap().contains("placeholder"));
        assert!(m.counters.is_empty() && m.rules.is_empty(), "{text}");
        assert!(!left.is_empty());
    }
}
