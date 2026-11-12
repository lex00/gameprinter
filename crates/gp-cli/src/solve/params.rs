//! From a detector's class to the numbers that would give it: the
//! mechanic parameters of the measure's owners (the refine loop's
//! owners), each with its unit, and the exact value the class names.

use gp_interp::mechanics::Catalog;
use gp_sim::Fx;

use super::arith::{self, ONE};
use super::timers;

/// One number a mechanic has for a part of the model, set or not.
#[derive(Debug, Clone)]
pub struct Number {
    pub mechanic: String,
    pub path: String,
    /// Its raw value; `None` for an optional parameter the model doesn't
    /// set (`every`, `phase`, `align`).
    pub value: Option<i64>,
    /// `px/frame`, `px/frame^2`, `px`, `frames` or empty.
    pub unit: String,
    /// Frames and counts are whole numbers.
    pub whole: bool,
    pub range: String,
    /// The parameter's own name: the path's last key.
    pub name: String,
}

/// One value change.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Edit {
    pub path: String,
    /// The value now, as the model writes it; `None` when it isn't set.
    pub old: Option<String>,
    pub new: String,
}

/// The part of the model a measure belongs to: `entities.mario[0].x` and
/// `bodies.koopa[1].x` are `entities.mario` and `entities.koopa`,
/// `ticks.time[0]` is `counters.time`.
pub fn part_of(path: &str) -> String {
    let Ok(p) = gp_model::Path::parse(path) else {
        return String::new();
    };
    let n: Vec<&str> = p.segments.iter().map(|s| s.name.as_str()).collect();
    match n.as_slice() {
        ["entities" | "bodies", k, ..] => format!("entities.{k}"),
        ["counters" | "ticks", k, ..] => format!("counters.{k}"),
        ["cameras", k, ..] => format!("cameras.{k}"),
        ["rules", k, ..] => format!("rules.{k}"),
        _ => String::new(),
    }
}

/// The node at a model path.
pub fn get<'a>(tree: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
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

/// A model number as raw 16.16.
pub fn raw_of(v: &toml::Value) -> Option<i64> {
    match v {
        toml::Value::Integer(i) => i.checked_mul(ONE),
        toml::Value::String(s) => gp_model::Fixed::parse(s).ok().map(|f| f.raw()),
        _ => None,
    }
}

/// A raw value as the model writes it.
pub fn text_of(raw: i64) -> String {
    Fx::from_raw(raw).to_string()
}

/// The unit a nested key takes from its parameter's unit text, which
/// names its fields: `from px, full px, push_hold frames, keep_move`.
fn nested_unit(unit: &str, key: &str) -> Option<String> {
    unit.split(',').find_map(|f| {
        let mut w = f.split_whitespace();
        (w.next()? == key).then(|| w.collect::<Vec<_>>().join(" "))
    })
}

/// Every number the mechanics `owners` have for `part`, set or not.
pub fn numbers(
    catalog: &Catalog,
    tree: &toml::Value,
    owners: &[String],
    part: &str,
) -> Vec<Number> {
    let mut out: Vec<Number> = Vec::new();
    for name in owners {
        let Some(m) = catalog.get(name) else { continue };
        for u in &m.used_by {
            if part.is_empty() || (u != part && !u.starts_with(&format!("{part}."))) {
                continue;
            }
            // A component's parameters are in its table, a hook's in its
            // entity's `ext_params`, a field's beside it.
            let base = match get(tree, u) {
                Some(toml::Value::Table(_)) => u.clone(),
                _ if u.ends_with(".ext") => format!("{part}.ext_params"),
                _ => u
                    .rsplit_once('.')
                    .map_or(String::new(), |(a, _)| a.to_owned()),
            };
            let Some(toml::Value::Table(t)) = get(tree, &base) else {
                continue;
            };
            for p in &m.params {
                let numeric = matches!(p.ty, "fixed" | "frames" | "count");
                let mut push = |path: String,
                                value: Option<i64>,
                                unit: &str,
                                whole: bool,
                                range: &str,
                                name: &str| {
                    if !out.iter().any(|x| x.path == path) {
                        out.push(Number {
                            mechanic: m.name.to_owned(),
                            path,
                            value,
                            unit: unit.to_owned(),
                            whole,
                            range: range.to_owned(),
                            name: name.to_owned(),
                        });
                    }
                };
                match t.get(p.name) {
                    Some(v) if numeric => {
                        if let Some(raw) = raw_of(v) {
                            push(
                                format!("{base}.{}", p.name),
                                Some(raw),
                                p.unit,
                                p.ty != "fixed",
                                p.range,
                                p.name,
                            );
                        }
                    }
                    Some(toml::Value::Table(inner)) => {
                        for (k, v) in inner {
                            if let Some(raw) = raw_of(v) {
                                let unit = nested_unit(p.unit, k).unwrap_or_default();
                                let whole =
                                    !matches!(v, toml::Value::String(_)) && unit == "frames";
                                push(
                                    format!("{base}.{}.{k}", p.name),
                                    Some(raw),
                                    &unit,
                                    whole,
                                    "",
                                    k,
                                );
                            }
                        }
                    }
                    Some(toml::Value::Array(items)) => {
                        for (i, item) in items.iter().enumerate() {
                            let Some(it) = item.as_table() else { continue };
                            for (k, v) in it {
                                let Some(raw) = raw_of(v) else { continue };
                                let like = m.params.iter().find(|q| q.name == k.as_str());
                                let unit = like
                                    .map(|q| q.unit.to_owned())
                                    .or_else(|| nested_unit(p.unit, k))
                                    .unwrap_or_default();
                                let whole = like.is_some_and(|q| q.ty != "fixed");
                                push(
                                    format!("{base}.{}[{i}].{k}", p.name),
                                    Some(raw),
                                    &unit,
                                    whole,
                                    like.map_or("", |q| q.range),
                                    k,
                                );
                            }
                        }
                    }
                    None if p.optional
                        && numeric
                        && matches!(p.name, "every" | "phase" | "align" | "wait_near_wrap") =>
                    {
                        push(
                            format!("{base}.{}", p.name),
                            None,
                            p.unit,
                            true,
                            p.range,
                            p.name,
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

fn in_range(range: &str, v: i64) -> bool {
    match range.trim() {
        "> 0" => v > 0,
        "< 0" => v < 0,
        ">= 0" => v >= 0,
        "<= 0" => v <= 0,
        _ => true,
    }
}

/// A candidate: one number to a new value, or `None` when it can't take
/// it (out of its range, a fraction in a whole number, no change).
fn to(n: &Number, new: i64) -> Option<Edit> {
    if Some(new) == n.value || !in_range(&n.range, new) || (n.whole && new % ONE != 0) {
        return None;
    }
    if n.unit == "frames" && new < 0 {
        return None;
    }
    Some(Edit {
        path: n.path.clone(),
        old: n.value.map(text_of),
        new: text_of(new),
    })
}

/// The edits an arithmetic class names, each a candidate on its own (a
/// list of edits is one candidate).
pub fn for_arith(class: &arith::Class, field: &str, nums: &[Number]) -> Vec<Vec<Edit>> {
    let speed = field.starts_with('v');
    let mut out: Vec<Vec<Edit>> = Vec::new();
    let mut one = |e: Option<Edit>| {
        if let Some(e) = e
            && !out.iter().any(|x| x.len() == 1 && x[0] == e)
        {
            out.push(vec![e]);
        }
    };
    match class {
        // A rate off by a fraction of a pixel; a whole pixel or more a
        // frame is no fraction to recover.
        arith::Class::Linear { rate, .. } if rate.abs() < ONE => {
            // A position that drifts by a rate: a speed off by it; a speed
            // that drifts: an acceleration or gravity off by it.
            let unit = if speed { "px/frame^2" } else { "px/frame" };
            for n in nums.iter().filter(|n| n.unit == unit) {
                let Some(v) = n.value else { continue };
                one(to(n, v - rate));
                one(to(n, v + rate));
            }
        }
        // An anchor or spawn offset: whole pixels off from the clip's
        // start on a place. A fraction from the start is a fraction the
        // translation doesn't carry, and an offset that starts later
        // follows a frame the two did something different: neither is a
        // number to change.
        arith::Class::Constant { by, from_start } if *from_start && !speed && by % ONE == 0 => {
            for n in nums.iter().filter(|n| n.unit == "px") {
                let Some(v) = n.value else { continue };
                one(to(n, v - by));
                one(to(n, v + by));
            }
        }
        arith::Class::Lag { frames } => {
            for n in nums
                .iter()
                .filter(|n| n.unit == "frames" && n.name != "every")
            {
                match n.value {
                    Some(v) => {
                        one(to(n, v + frames * ONE));
                        one(to(n, v - frames * ONE));
                    }
                    None if n.name == "phase" => one(to(n, ONE)),
                    None => {}
                }
            }
        }
        arith::Class::SignFlip { model, .. } => {
            for n in nums {
                if let Some(v) = n.value
                    && v != 0
                    && v.abs() == model.abs()
                {
                    one(to(n, -v));
                }
            }
        }
        arith::Class::EveryN { every, .. } => {
            let e = nums.iter().find(|n| n.name == "every");
            let p = nums.iter().find(|n| n.name == "phase");
            if let Some(e) = e {
                let set = to(e, *every as i64 * ONE);
                match (set, p) {
                    (Some(set), Some(p)) => {
                        // The phase's meaning differs by mechanic: each one
                        // is a candidate, and the trial picks.
                        for r in 0..*every as i64 {
                            let mut c = vec![set.clone()];
                            if let Some(pe) = to(p, r * ONE) {
                                c.push(pe);
                            } else if p.value != Some(r * ONE) {
                                continue;
                            }
                            out.push(c);
                        }
                    }
                    (Some(set), None) => out.push(vec![set]),
                    _ => {}
                }
            }
        }
        arith::Class::Wrap { .. } => {
            for n in nums.iter().filter(|n| n.name.ends_with("_wrap")) {
                one(to(n, 256 * ONE));
            }
        }
        arith::Class::Linear { .. }
        | arith::Class::Constant { .. }
        | arith::Class::Stops { .. }
        | arith::Class::Start { .. } => {}
    }
    out
}

/// The edits a timer class names.
pub fn for_timer(class: &timers::Class, nums: &[Number]) -> Vec<Vec<Edit>> {
    let mut out: Vec<Vec<Edit>> = Vec::new();
    let mut one = |e: Option<Edit>| {
        if let Some(e) = e
            && !out.iter().any(|x| x.len() == 1 && x[0] == e)
        {
            out.push(vec![e]);
        }
    };
    let counts = |n: &&Number| matches!(n.unit.as_str(), "frames" | "" | "moves") || n.whole;
    match class {
        timers::Class::Period { original, model } => {
            for n in nums.iter().filter(|n| n.unit == "frames") {
                if n.value == Some(*model as i64 * ONE) {
                    one(to(n, *original as i64 * ONE));
                }
            }
        }
        timers::Class::Reload { original, model } => {
            // A reload of N shows as N, or as N less one where the count
            // steps on the frame it is set: the same difference either way.
            for n in nums.iter().filter(counts) {
                let Some(v) = n.value else { continue };
                if (v - model).abs() <= ONE {
                    one(to(n, v + (original - model)));
                }
            }
        }
        timers::Class::Shift { frames } => {
            for n in nums
                .iter()
                .filter(|n| n.unit == "frames" && n.name != "every")
            {
                match n.value {
                    Some(v) => {
                        one(to(n, v + frames * ONE));
                        one(to(n, v - frames * ONE));
                    }
                    None if n.name == "phase" => one(to(n, frames.abs() * ONE)),
                    None => {}
                }
            }
        }
        timers::Class::Aligned { every, phase } => {
            for n in nums.iter().filter(|n| n.name == "align") {
                one(to(n, *every as i64 * ONE));
            }
            let e = nums.iter().find(|n| n.name == "every" && n.value.is_none());
            let p = nums.iter().find(|n| n.name == "phase");
            if let (Some(e), Some(p)) = (e, p)
                && let Some(set) = to(e, *every as i64 * ONE)
            {
                let mut c = vec![set];
                c.extend(to(p, *phase as i64 * ONE));
                out.push(c);
            }
        }
        timers::Class::Stops {
            rebuilt: true,
            value,
            other,
        } => {
            // A bound the model's count stops at.
            for n in nums.iter().filter(counts) {
                if n.value == Some(*value) {
                    one(to(n, *other));
                }
            }
        }
        timers::Class::Stops { .. } | timers::Class::Start { .. } => {}
    }
    out
}
