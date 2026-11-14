//! The grader of stage 0 (`gp model static --grade KEY`), and the one
//! module the bench calls to grade model facts: a harvest's facts against
//! the real model, its elements and routines against the game's ground
//! truth, its mapping rows against the real mapping. Only the grader reads
//! the answer key (games/GAME/static-key.toml), which names each of the
//! harvest's names in the real model's terms.
//!
//! | Measure | What it counts |
//! |---|---|
//! | Elements found | The ground truth's element tags some harvest element's routines are mostly in |
//! | Routines placed | Routines whose harvest element is mostly that routine's own tag |
//! | Facts right, wrong, missing | The real model's behaviour numbers (the bench's set: every number in `sections` but the `keep` keys), by path |
//! | Shape right, wrong | The harvest's other facts that land on a real path |
//! | Unplaced | Harvest facts with no real path (the model doesn't state them) |
//! | Share built statically | Numbers right over all numbers |
//! | Mapping rows | Real mapping fields whose RAM is in the matching element's rows |

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use toml::Value as Toml;

use gp_nes::codemap::va_parse;
use gp_nes::harvest::Harvest;

fn is_number(v: &Toml) -> bool {
    match v {
        Toml::Integer(_) | Toml::Float(_) => true,
        Toml::String(s) => s.parse::<f64>().is_ok(),
        Toml::Array(a) => !a.is_empty() && a.iter().all(|x| x.is_integer() || x.is_float()),
        _ => false,
    }
}

/// A value as numbers to compare: `"0.25"`, `0.25` the same.
pub(crate) fn as_number(v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::Number(n) => n.as_f64().map(|x| vec![x]),
        Value::String(s) => s.parse().ok().map(|x| vec![x]),
        Value::Array(a) => a
            .iter()
            .map(|x| as_number(x).and_then(|v| v.first().copied()))
            .collect(),
        _ => None,
    }
}

/// A ground-truth routine's address with its bank, as the harvest writes
/// banked addresses ([`gp_nes::codemap::va_hex`]): `bank = N` for code in a
/// switched region.
fn banked(r: &Toml, addr: i64) -> u32 {
    let a = addr as u32 & 0xFFFF;
    match r.get("bank").and_then(Toml::as_integer) {
        Some(b) => ((b as u32 + 1) << 16) | a,
        None => a,
    }
}

/// A routine's address from its harvest name: `sub_C123`, `sub_01_8123`.
fn routine_va(name: &str) -> Option<u32> {
    let t = name.strip_prefix("sub_")?;
    va_parse(&t.replacen('_', ":", 1))
}

pub(crate) fn toml_json(v: &Toml) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// The real model's leaves by path with list positions
/// (`rules.game.states.dying.transition[1].after`): numbers and the rest.
pub(crate) fn leaves(
    doc: &Toml,
    sections: &[String],
    keep: &[String],
) -> (BTreeMap<String, Toml>, BTreeMap<String, Toml>) {
    fn walk(
        v: &Toml,
        at: &str,
        keep: &[String],
        nums: &mut BTreeMap<String, Toml>,
        shape: &mut BTreeMap<String, Toml>,
    ) {
        match v {
            Toml::Table(t) => {
                for (k, x) in t {
                    if keep.iter().any(|kk| kk == k) {
                        continue;
                    }
                    walk(x, &format!("{at}.{k}"), keep, nums, shape);
                }
            }
            Toml::Array(a) if a.iter().any(Toml::is_table) => {
                for (i, x) in a.iter().enumerate() {
                    walk(x, &format!("{at}[{i}]"), keep, nums, shape);
                }
            }
            x if is_number(x) => {
                nums.insert(at.to_owned(), x.clone());
            }
            x => {
                shape.insert(at.to_owned(), x.clone());
            }
        }
    }
    let (mut nums, mut shape) = (BTreeMap::new(), BTreeMap::new());
    for sec in sections {
        if let Some(v) = doc.get(sec.as_str()) {
            walk(v, sec, keep, &mut nums, &mut shape);
        }
    }
    (nums, shape)
}

struct Key {
    names: Vec<(String, String)>,
    states: BTreeMap<String, BTreeMap<String, String>>,
    lists: BTreeMap<String, String>,
    real: Toml,
}

impl Key {
    /// A harvest path in the real model's names, with each named list
    /// element at its position.
    fn translate(&self, path: &str) -> Option<String> {
        let mut p = path.to_owned();
        // Longest prefix first.
        let mut names = self.names.clone();
        names.sort_by_key(|(a, _)| std::cmp::Reverse(a.len()));
        for (a, b) in &names {
            if p == *a || p.starts_with(&format!("{a}.")) || p.starts_with(&format!("{a}[")) {
                p = format!("{b}{}", &p[a.len()..]);
                break;
            }
        }
        // State names, under the longest machine prefix.
        let mut machines: Vec<&String> = self.states.keys().collect();
        machines.sort_by_key(|m| std::cmp::Reverse(m.len()));
        let mut states_of: Option<&BTreeMap<String, String>> = None;
        for m in machines {
            if p.starts_with(&format!("{m}.")) {
                states_of = self.states.get(m);
                let rest = &p[m.len()..];
                let rest = rest
                    .split('.')
                    .map(|seg| {
                        let mut seg = seg.to_owned();
                        for (s, n) in &self.states[m] {
                            if seg == *s {
                                seg = n.clone();
                            }
                            seg = seg.replace(&format!("[{s}]"), &format!("[{n}]"));
                        }
                        seg
                    })
                    .collect::<Vec<_>>()
                    .join(".");
                p = format!("{m}{rest}");
                break;
            }
        }
        let _ = states_of;
        // Named list elements: `list[name]` -> `list[i]` where the real
        // element's naming field is `name`.
        let mut out = String::new();
        let mut rest = p.as_str();
        while let Some(i) = rest.find('[') {
            let j = rest[i..].find(']')? + i;
            let inner = &rest[i + 1..j];
            out.push_str(&rest[..i]);
            if inner.parse::<usize>().is_ok() {
                out.push_str(&rest[i..=j]);
            } else {
                let list_path = out.clone();
                let last = list_path.rsplit('.').next().unwrap_or("");
                let field = self
                    .lists
                    .get(&list_path)
                    .or_else(|| self.lists.get(last))?;
                let arr = get(&self.real, &list_path)?.as_array()?;
                let idx = arr
                    .iter()
                    .position(|e| e.get(field.as_str()).and_then(Toml::as_str) == Some(inner))?;
                out.push_str(&format!("[{idx}]"));
            }
            rest = &rest[j + 1..];
        }
        out.push_str(rest);
        Some(out)
    }

    /// A value in the real model's names: a harvest name, or a state of
    /// the machine the path is under.
    fn value(&self, v: &Value, path: &str) -> Value {
        let Value::String(s) = v else {
            return v.clone();
        };
        for (a, b) in &self.names {
            if a == s {
                return json!(b);
            }
        }
        let mut machines: Vec<&String> = self.states.keys().collect();
        machines.sort_by_key(|m| std::cmp::Reverse(m.len()));
        for m in machines {
            if path.starts_with(m.as_str())
                && let Some(n) = self.states[m].get(s)
            {
                return json!(n);
            }
        }
        v.clone()
    }
}

/// A path with list positions into a TOML document.
fn get<'a>(doc: &'a Toml, path: &str) -> Option<&'a Toml> {
    let mut cur = doc;
    for seg in path.split('.') {
        let (name, idx) = match seg.find('[') {
            Some(i) => (
                &seg[..i],
                Some(seg[i + 1..seg.len() - 1].parse::<usize>().ok()?),
            ),
            None => (seg, None),
        };
        cur = cur.get(name)?;
        if let Some(i) = idx {
            cur = cur.as_array()?.get(i)?;
        }
    }
    Some(cur)
}

fn read_toml(p: &Path) -> Result<Toml> {
    toml::from_str(&fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?)
        .with_context(|| format!("parsing {}", p.display()))
}

fn strings(v: Option<&Toml>) -> Vec<String> {
    v.and_then(Toml::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Reads an answer key: its names, states and lists, and the real model.
fn load_key(key_path: &Path) -> Result<(Key, Toml, std::path::PathBuf)> {
    let k = read_toml(key_path)?;
    let dir = key_path.parent().unwrap_or(Path::new("."));
    let head = k.get("key").context("the key has no [key]")?;
    let at = |n: &str| head.get(n).and_then(Toml::as_str).map(|p| dir.join(p));
    let real = read_toml(&at("model").context("[key] model")?)?;
    let names: Vec<(String, String)> = k
        .get("names")
        .and_then(Toml::as_table)
        .map(|t| {
            t.iter()
                .filter_map(|(a, b)| Some((a.clone(), b.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let states = k
        .get("states")
        .and_then(Toml::as_table)
        .map(|t| {
            t.iter()
                .map(|(m, s)| {
                    let sm = s
                        .as_table()
                        .map(|x| {
                            x.iter()
                                .filter_map(|(a, b)| Some((a.clone(), b.as_str()?.to_owned())))
                                .collect()
                        })
                        .unwrap_or_default();
                    (m.clone(), sm)
                })
                .collect()
        })
        .unwrap_or_default();
    let lists = k
        .get("lists")
        .and_then(Toml::as_table)
        .map(|t| {
            t.iter()
                .filter_map(|(a, b)| Some((a.clone(), b.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let key = Key {
        names,
        states,
        lists,
        real,
    };
    Ok((key, k, dir.to_path_buf()))
}

/// The harvest's facts in the real model's names and list positions, for
/// a caller that applies them to a model (the bench's stage 0): each
/// `(path, value, harvest path)` the key can place.
pub(crate) fn in_model_names(h: &Harvest, key_path: &Path) -> Result<Vec<(String, Value, String)>> {
    let key = load_key(key_path)?.0;
    Ok(h.facts
        .iter()
        .filter_map(|f| {
            let p = key.translate(&f.path)?;
            Some((p.clone(), key.value(&f.value, &p), f.path.clone()))
        })
        .collect())
}

/// Grades a harvest with an answer key.
pub(crate) fn grade(h: &Harvest, key_path: &Path) -> Result<Value> {
    let (key, k, dir) = load_key(key_path)?;
    let head = k.get("key").context("the key has no [key]")?;
    let at = |n: &str| head.get(n).and_then(Toml::as_str).map(|p| dir.join(p));
    let real = key.real.clone();
    let sections = strings(head.get("sections"));
    let keep = strings(head.get("keep"));
    let (nums, shape) = leaves(&real, &sections, &keep);

    // Facts.
    let mut have: BTreeMap<String, (Value, String)> = BTreeMap::new();
    let mut unplaced = Vec::new();
    for f in &h.facts {
        match key.translate(&f.path) {
            Some(p) if nums.contains_key(&p) || shape.contains_key(&p) => {
                let val = key.value(&f.value, &p);
                have.entry(p).or_insert((val, f.path.clone()));
            }
            Some(p) => unplaced.push(json!({"path": f.path, "as": p})),
            None => unplaced.push(json!({"path": f.path})),
        }
    }
    let (mut right, mut wrong, mut missing) = (Vec::new(), Vec::new(), Vec::new());
    for (p, v) in &nums {
        match have.get(p) {
            None => missing.push(json!(p)),
            Some((hv, hp)) if as_number(hv) == as_number(&toml_json(v)) => {
                right.push(json!({"path": p, "from": hp}))
            }
            Some((hv, hp)) => {
                wrong.push(json!({"path": p, "real": toml_json(v), "static": hv, "from": hp}))
            }
        }
    }
    let (mut s_right, mut s_wrong, mut s_missing) = (Vec::new(), Vec::new(), Vec::new());
    for (p, v) in &shape {
        if let Some((hv, hp)) = have.get(p) {
            if *hv == toml_json(v) {
                s_right.push(json!(p));
            } else {
                s_wrong.push(json!({"path": p, "real": toml_json(v), "static": hv, "from": hp}));
            }
        } else {
            s_missing.push(json!({"path": p, "real": toml_json(v)}));
        }
    }

    // Open facts: each names a routine of the ground truth, and bytes that
    // routine's own instructions read or write (by the source's RAM map).
    let mut open = json!(null);
    if let (Some(gt), Some(ram)) = (at("ground_truth"), at("ram")) {
        let g = read_toml(&gt)?;
        let ram_text =
            fs::read_to_string(&ram).with_context(|| format!("reading {}", ram.display()))?;
        let mut addr_of: BTreeMap<String, u16> = BTreeMap::new();
        for line in ram_text.lines() {
            let l = line.split(';').next().unwrap_or("");
            if let Some((n, v)) = l.split_once('=')
                && let Some(h) = v.trim().strip_prefix('$')
                && let Ok(a) = u16::from_str_radix(h, 16)
            {
                addr_of.insert(n.trim().to_owned(), a);
            }
        }
        let mut routines: Vec<(u32, String, BTreeSet<u16>)> = Vec::new();
        for r in g
            .get("routine")
            .and_then(Toml::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let (Some(a), Some(n)) = (
                r.get("addr").and_then(Toml::as_integer),
                r.get("name").and_then(Toml::as_str),
            ) else {
                continue;
            };
            let mut bytes = BTreeSet::new();
            for x in r
                .get("reads")
                .and_then(Toml::as_array)
                .cloned()
                .unwrap_or_default()
            {
                if let Some(b) = x.get("addr").and_then(Toml::as_integer) {
                    bytes.insert(b as u16);
                }
            }
            for w in strings(r.get("writes")) {
                if let Some(b) = addr_of.get(&w) {
                    bytes.insert(*b);
                }
            }
            routines.push((banked(&r, a), n.to_owned(), bytes));
        }
        routines.sort();
        let at_pc = |pc: u32| routines.iter().rev().find(|r| r.0 <= pc);
        let (mut ok, mut checked, mut bad) = (0, 0, Vec::new());
        for o in &h.open {
            let Some(pc) = va_parse(&o.pc) else {
                continue;
            };
            checked += 1;
            let r = at_pc(pc);
            let named = routine_va(&o.routine);
            let routine_ok = r.is_some_and(|r| named.is_some_and(|n| n <= pc && r.0 <= n));
            let bytes_ok = o.bytes.iter().all(|b| {
                let t = b.trim_start_matches('$');
                let base = t.split(',').next().unwrap_or("");
                u16::from_str_radix(base, 16).is_ok_and(|a| r.is_some_and(|r| r.2.contains(&a)))
            });
            if routine_ok && bytes_ok {
                ok += 1;
            } else {
                bad.push(json!({"what": o.what, "routine": r.map(|r| r.1.clone()), "routine_ok": routine_ok, "bytes_ok": bytes_ok}));
            }
        }
        open = json!({"with_a_routine": checked, "named_right": ok, "named_wrong": bad});
    }

    // Elements and routines against the ground truth.
    let mut elements = json!(null);
    if let Some(gt) = at("ground_truth") {
        let g = read_toml(&gt)?;
        let mut tag_of: BTreeMap<u32, (String, String)> = BTreeMap::new();
        for r in g
            .get("routine")
            .and_then(Toml::as_array)
            .cloned()
            .unwrap_or_default()
        {
            if let (Some(a), Some(e), Some(n)) = (
                r.get("addr").and_then(Toml::as_integer),
                r.get("element").and_then(Toml::as_str),
                r.get("name").and_then(Toml::as_str),
            ) {
                tag_of.insert(banked(&r, a), (e.to_owned(), n.to_owned()));
            }
        }
        let tags: BTreeSet<String> = tag_of.values().map(|t| t.0.clone()).collect();
        let addr = |s: &str| va_parse(s);
        let found_routines = h
            .routines
            .iter()
            .filter(|r| addr(&r.addr).is_some_and(|a| tag_of.contains_key(&a)))
            .count();
        let mut label: BTreeMap<String, String> = BTreeMap::new();
        for e in &h.elements {
            let mut cnt: BTreeMap<String, usize> = BTreeMap::new();
            for rn in &e.routines {
                if let Some(r) = h.routines.iter().find(|r| &r.name == rn)
                    && let Some((t, _)) = addr(&r.addr).and_then(|a| tag_of.get(&a))
                {
                    *cnt.entry(t.clone()).or_default() += 1;
                }
            }
            if let Some((t, _)) = cnt
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            {
                label.insert(e.name.clone(), t);
            }
        }
        let found: BTreeSet<String> = label.values().cloned().collect();
        let mut placed = 0;
        let mut misplaced = Vec::new();
        for r in &h.routines {
            let Some((t, n)) = addr(&r.addr).and_then(|a| tag_of.get(&a)) else {
                continue;
            };
            if label.get(&r.element) == Some(t) {
                placed += 1;
            } else {
                misplaced.push(json!({"routine": n, "is": t, "static": r.element}));
            }
        }
        elements = json!({
            "source": tags.len(),
            "found": found.len(),
            "found_names": found,
            "missing": tags.difference(&found).cloned().collect::<Vec<_>>(),
            "static_elements": h.elements.len(),
            "routines_source": tag_of.len(),
            "routines_found": found_routines,
            "routines_placed": placed,
            "misplaced": misplaced,
        });
    }

    // Mapping rows.
    let mut mapping = json!(null);
    if let Some(mp) = at("mapping") {
        let m = read_toml(&mp)?;
        let el: BTreeMap<String, String> = k
            .get("elements")
            .and_then(Toml::as_table)
            .map(|t| {
                t.iter()
                    .filter_map(|(a, b)| Some((b.as_str()?.to_owned(), a.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let mut rows: BTreeMap<String, BTreeSet<u16>> = BTreeMap::new();
        let mut counter_bytes: BTreeSet<u16> = BTreeSet::new();
        for e in &h.elements {
            for r in &e.mapping {
                let t = r.byte.trim_start_matches('$');
                let (a, indexed) = match t.split_once(',') {
                    Some((a, _)) => (a, true),
                    None => (t, false),
                };
                let Ok(a) = u16::from_str_radix(a, 16) else {
                    continue;
                };
                let n = if indexed {
                    h.facts
                        .iter()
                        .find(|f| f.path == format!("entities.{}.count", e.name))
                        .and_then(|f| f.value.as_u64())
                        .unwrap_or(1) as u16
                } else {
                    1
                };
                for i in 0..n {
                    rows.entry(e.name.clone()).or_default().insert(a + i);
                    if r.role == "counter" {
                        counter_bytes.insert(a + i);
                    }
                }
            }
        }
        let (mut ok, mut total, mut miss) = (0, 0, Vec::new());
        for f in m
            .get("field")
            .and_then(Toml::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let Some(path) = f.get("path").and_then(Toml::as_str) else {
                continue;
            };
            let ram: Vec<u16> = f
                .get("ram")
                .and_then(Toml::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_integer().map(|i| i as u16))
                        .collect()
                })
                .unwrap_or_default();
            let prefix = el
                .keys()
                .filter(|p| path.starts_with(p.as_str()))
                .max_by_key(|p| p.len())
                .cloned();
            total += 1;
            let hit = match prefix.as_deref() {
                Some("counters") => ram.iter().all(|b| counter_bytes.contains(b)),
                Some(p) => {
                    let e = &el[p];
                    rows.get(e)
                        .is_some_and(|s| ram.iter().all(|b| s.contains(b)))
                }
                None => false,
            };
            if hit {
                ok += 1;
            } else {
                miss.push(json!(path));
            }
        }
        mapping = json!({"fields": total, "found": ok, "missing": miss});
    }

    // State machine skeletons: each harvest machine's transitions, from
    // and to, against the real machine's.
    let mut machines = Vec::new();
    for m in &h.machines {
        let Some(rp) = key.translate(&m.path) else {
            continue;
        };
        let Some(states) = get(&real, &format!("{rp}.states")).and_then(Toml::as_table) else {
            continue;
        };
        let mut real_t: BTreeSet<(String, String)> = BTreeSet::new();
        for (from, st) in states {
            for t in st
                .get("transition")
                .and_then(Toml::as_array)
                .cloned()
                .unwrap_or_default()
            {
                if let Some(to) = t.get("to").and_then(Toml::as_str)
                    && to != from
                {
                    real_t.insert((from.clone(), to.to_owned()));
                }
            }
        }
        let names = key.states.get(&rp).cloned().unwrap_or_default();
        let n = |s: &str| names.get(s).cloned().unwrap_or_else(|| s.to_owned());
        let mine: BTreeSet<(String, String)> = m
            .transitions
            .iter()
            .map(|t| (n(&t.from), n(&t.to)))
            .filter(|(a, b)| a != b)
            .collect();
        let found: Vec<_> = real_t.intersection(&mine).cloned().collect();
        let extra: Vec<_> = mine.difference(&real_t).cloned().collect();
        let missing: Vec<_> = real_t.difference(&mine).cloned().collect();
        machines.push(json!({"machine": rp, "real": real_t.len(), "found": found.len(), "extra": extra, "missing": missing}));
    }

    Ok(json!({
        "machines": machines,
        "open_graded": open,
        "shape_missing": s_missing.len(),
        "shape_missing_paths": s_missing,
        "numbers": nums.len(),
        "right": right.len(),
        "wrong": wrong.len(),
        "missing": missing.len(),
        "share": if nums.is_empty() { 0.0 } else { right.len() as f64 / nums.len() as f64 },
        "shape_right": s_right.len(),
        "shape_wrong": s_wrong.len(),
        "unplaced": unplaced.len(),
        "open": h.open.len(),
        "right_paths": right,
        "wrong_paths": wrong,
        "missing_paths": missing,
        "shape_wrong_paths": s_wrong,
        "unplaced_paths": unplaced,
        "elements": elements,
        "mapping": mapping,
    }))
}

/// The grade as the plan's table row's text.
pub(crate) fn table(g: &Value) -> String {
    let e = &g["elements"];
    let m = &g["mapping"];
    format!(
        "| Measure | Value |\n|---|---|\n| Elements found | {} of {} ({} static elements) |\n| Routines placed | {} of {} |\n| Facts right, wrong, missing | {} right, {} wrong, {} missing of {} numbers |\n| Shape facts right, wrong, missing | {}, {}, {} |\n| Open facts | {} ({} named with their routine and bytes right) |\n| Share built statically | {:.1}% |\n| Mapping fields found | {} of {} |\n| Transitions found | {} of {}, {} extra |",
        e["found"],
        e["source"],
        e["static_elements"],
        e["routines_placed"],
        e["routines_source"],
        g["right"],
        g["wrong"],
        g["missing"],
        g["numbers"],
        g["shape_right"],
        g["shape_wrong"],
        g["shape_missing"],
        g["open"],
        g["open_graded"]["named_right"],
        g["share"].as_f64().unwrap_or(0.0) * 100.0,
        m["found"],
        m["fields"],
        sum(g, "found"),
        sum(g, "real"),
        g["machines"].as_array().map_or(0, |a| a
            .iter()
            .map(|x| x["extra"].as_array().map_or(0, Vec::len))
            .sum::<usize>())
    )
}

fn sum(g: &Value, k: &str) -> u64 {
    g["machines"]
        .as_array()
        .map_or(0, |a| a.iter().filter_map(|x| x[k].as_u64()).sum())
}
