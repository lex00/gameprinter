//! Mutants of a model, by kind (docs/benchmarks.md, "gp bench clips"):
//! the wrong models a clip set should catch. `gp model mutants` writes
//! them for any model, and `gp bench clips` makes Moss Hop's with the same
//! code, so both give the same list for the same model.
//!
//! | Kind | The change |
//! |---|---|
//! | `input` | A rule's condition, a `stops` entry or a tile kind dropped |
//! | `constant` | A number up by one (a fraction by a sixteenth), a flag turned over |
//! | `transition` | A transition deleted |
//! | `order` | Two adjacent systems swapped |
//! | `period` | A `period`, `every` or `after` changed as a constant is |
//!
//! Only the behaviour sections (entities, levels, cameras, rules,
//! counters) and the systems' order are changed, never pictures or
//! documentation. Each mutant's name is its kind and its number in the
//! kind (`constant-007`), in the order of kind, path and change, and its
//! `says` is `PATH: CHANGE`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::json;
use toml::Value as Toml;

use crate::Outcome;

// Ported from game-printer, where the mutant type and its edits lived in
// gp serve's prune (serve/prune.rs); gameprinter has no prune yet, so they
// are here.

/// One mutant: the model with some of its facts removed or set, as a fix
/// turned off.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct Mutant {
    pub name: String,
    /// The merge or commit that added what it takes away.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fix: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub says: String,
    /// Paths into model.toml to remove: `entities.mario.run.move_step`;
    /// `*` matches every key, and an array of tables is gone through
    /// element by element.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove: Vec<String>,
    /// Paths to set, with their values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, toml::Value>,
}

/// Applies `op` at `segs` under `v`: the count of places it changed.
// Used by gp bench clips and a prune once they are here, and the tests.
#[cfg_attr(not(test), allow(dead_code))]
fn apply(v: &mut toml::Value, segs: &[&str], set: Option<&toml::Value>) -> usize {
    match v {
        toml::Value::Array(a) => a.iter_mut().map(|x| apply(x, segs, set)).sum(),
        toml::Value::Table(t) => {
            let Some((head, rest)) = segs.split_first() else {
                return 0;
            };
            let keys: Vec<String> = if *head == "*" {
                t.keys().cloned().collect()
            } else {
                vec![(*head).to_owned()]
            };
            let mut n = 0;
            for k in keys {
                if rest.is_empty() {
                    match set {
                        None => n += usize::from(t.remove(&k).is_some()),
                        Some(val) => {
                            t.insert(k, val.clone());
                            n += 1;
                        }
                    }
                } else if let Some(x) = t.get_mut(&k) {
                    n += apply(x, rest, set);
                }
            }
            n
        }
        _ => 0,
    }
}

/// The model text with the mutant's edits, or why it can't be made.
// Used by gp bench clips and a prune once they are here, and the tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn mutate(text: &str, m: &Mutant) -> anyhow::Result<String> {
    let mut v: toml::Value = toml::from_str(text).context("reading model.toml")?;
    for p in &m.remove {
        let segs: Vec<&str> = p.split('.').collect();
        if apply(&mut v, &segs, None) == 0 {
            anyhow::bail!("mutant {}: nothing at {p} to remove", m.name);
        }
    }
    for (p, val) in &m.set {
        let segs: Vec<&str> = p.split('.').collect();
        if apply(&mut v, &segs, Some(val)) == 0 {
            anyhow::bail!("mutant {}: nothing at {p} to set", m.name);
        }
    }
    Ok(toml::to_string(&v)?)
}

/// The kinds of wrong model the plan lists, in its order.
pub(crate) const KINDS: &[(&str, &str)] = &[
    ("input", "drop an input from a rule"),
    ("constant", "change a constant"),
    ("transition", "delete a transition"),
    ("order", "swap two systems' order"),
    ("mapping", "drop a mapping field"),
    ("period", "change a period"),
];

/// The model's parts mutants are made in: its behaviour, not its pictures
/// or its documentation.
const SECTIONS: &[&str] = &["entities", "levels", "cameras", "rules", "counters"];
/// Keys whose parts are presentation, names or documentation.
const SKIP: &[&str] = &[
    "sprite",
    "description",
    "note",
    "size",
    "tile_size",
    "kind_property",
    "solid_property",
];
const PERIODS: &[&str] = &["period", "every", "after"];
const INPUT_LISTS: &[&str] = &["stops", "stops_at_once"];

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Key(String),
    At(usize),
}

fn show(path: &[Step]) -> String {
    let mut out = String::new();
    for p in path {
        match p {
            Step::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Step::At(i) => out.push_str(&format!("[{i}]")),
        }
    }
    out
}

fn at_mut<'a>(v: &'a mut Toml, path: &[Step]) -> Option<&'a mut Toml> {
    let mut v = v;
    for p in path {
        v = match p {
            Step::Key(k) => v.get_mut(k.as_str())?,
            Step::At(i) => v.as_array_mut()?.get_mut(*i)?,
        };
    }
    Some(v)
}

/// A changed number: one more for an integer, a sixteenth more for a
/// fraction (`"0.25"` or 0.25).
fn bumped(v: &Toml) -> Option<Toml> {
    match v {
        Toml::Integer(i) => Some(Toml::Integer(i + 1)),
        Toml::Float(f) => Some(Toml::Float(f + 0.0625)),
        Toml::Boolean(b) => Some(Toml::Boolean(!b)),
        Toml::String(s) => {
            let f: f64 = s.parse().ok()?;
            Some(Toml::String(if s.contains('.') {
                format!("{}", f + 0.0625)
            } else {
                format!("{}", f as i64 + 1)
            }))
        }
        _ => None,
    }
}

/// One proposed mutant: its kind, the part it changes and how, and the
/// changed document.
struct Proposed {
    kind: &'static str,
    path: String,
    what: String,
    doc: Toml,
}

fn propose(real: &Toml) -> Vec<Proposed> {
    let mut out = Vec::new();
    let mut edit = |kind: &'static str, path: &[Step], what: String, f: &dyn Fn(&mut Toml)| {
        let mut doc = real.clone();
        if let Some(v) = at_mut(&mut doc, path) {
            f(v);
            if &doc != real {
                out.push(Proposed {
                    kind,
                    path: show(path),
                    what,
                    doc,
                });
            }
        }
    };
    // Walk the behaviour sections.
    fn walk(v: &Toml, path: &mut Vec<Step>, seen: &mut dyn FnMut(&[Step], &Toml)) {
        seen(path, v);
        match v {
            Toml::Table(t) => {
                for (k, x) in t {
                    if SKIP.contains(&k.as_str()) {
                        continue;
                    }
                    path.push(Step::Key(k.clone()));
                    walk(x, path, seen);
                    path.pop();
                }
            }
            Toml::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    path.push(Step::At(i));
                    walk(x, path, seen);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut found: Vec<(Vec<Step>, Toml)> = Vec::new();
    for sec in SECTIONS {
        if let Some(v) = real.get(*sec) {
            let mut path = vec![Step::Key((*sec).to_owned())];
            walk(v, &mut path, &mut |p, x| {
                found.push((p.to_vec(), x.clone()))
            });
        }
    }
    for (path, v) in &found {
        let key = path.iter().rev().find_map(|p| match p {
            Step::Key(k) => Some(k.as_str()),
            Step::At(_) => None,
        });
        let last_is_key = matches!(path.last(), Some(Step::Key(_)));
        let parent = &path[..path.len() - 1];
        match (key, v) {
            (Some("transition"), Toml::Array(a)) => {
                for i in 0..a.len() {
                    let what = format!("transition {i} deleted");
                    edit("transition", path, what, &move |x: &mut Toml| {
                        if let Some(a) = x.as_array_mut() {
                            a.remove(i);
                        }
                    });
                }
            }
            (Some("if"), Toml::Table(_)) if last_is_key => {
                edit(
                    "input",
                    parent,
                    "its condition dropped".into(),
                    &|x: &mut Toml| {
                        if let Some(t) = x.as_table_mut() {
                            t.remove("if");
                        }
                    },
                );
            }
            (Some(k), Toml::Array(a)) if INPUT_LISTS.contains(&k) => {
                for (i, item) in a.iter().enumerate() {
                    let what = format!("{} dropped", item.as_str().unwrap_or("?"));
                    edit("input", path, what, &move |x: &mut Toml| {
                        if let Some(a) = x.as_array_mut() {
                            a.remove(i);
                        }
                    });
                }
            }
            (Some("tile_kinds"), Toml::Table(t)) => {
                for k in t.keys() {
                    let k = k.clone();
                    let what = format!("tile kind {k} dropped");
                    edit("input", path, what, &move |x: &mut Toml| {
                        if let Some(t) = x.as_table_mut() {
                            t.remove(&k);
                        }
                    });
                }
            }
            (Some(k), x) if last_is_key && bumped(x).is_some() => {
                let kind = if PERIODS.contains(&k) {
                    "period"
                } else {
                    "constant"
                };
                let b = bumped(x).expect("bumped");
                let what = format!("{} for {}", show_value(&b), show_value(x));
                edit(kind, path, what, &move |y: &mut Toml| *y = b.clone());
            }
            (Some(_), Toml::Array(a))
                if last_is_key
                    && !a.is_empty()
                    && a.iter().all(|x| x.is_integer() || x.is_float()) =>
            {
                for i in 0..a.len() {
                    let b = bumped(&a[i]).expect("a number");
                    let what = format!("element {i} {} for {}", show_value(&b), show_value(&a[i]));
                    edit("constant", path, what, &move |y: &mut Toml| {
                        if let Some(a) = y.as_array_mut() {
                            a[i] = b.clone();
                        }
                    });
                }
            }
            _ => {}
        }
    }
    // Adjacent systems swapped.
    if let Some(sys) = real
        .get("model")
        .and_then(|m| m.get("systems"))
        .and_then(Toml::as_array)
    {
        let path = [Step::Key("model".into()), Step::Key("systems".into())];
        for i in 0..sys.len().saturating_sub(1) {
            let what = format!(
                "{} after {}",
                sys[i].as_str().unwrap_or("?"),
                sys[i + 1].as_str().unwrap_or("?")
            );
            edit("order", &path, what, &move |x: &mut Toml| {
                if let Some(a) = x.as_array_mut() {
                    a.swap(i, i + 1);
                }
            });
        }
    }
    out
}

fn show_value(v: &Toml) -> String {
    match v {
        Toml::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The edits that turn `a` into `b`, as a mutant's `remove` and `set`
/// paths: through tables only, so an edit inside a list sets the list.
fn ops(
    a: &Toml,
    b: &Toml,
    prefix: &str,
    remove: &mut Vec<String>,
    set: &mut BTreeMap<String, Toml>,
) {
    let (Some(ta), Some(tb)) = (a.as_table(), b.as_table()) else {
        set.insert(prefix.to_owned(), b.clone());
        return;
    };
    let join = |k: &str| {
        if prefix.is_empty() {
            k.to_owned()
        } else {
            format!("{prefix}.{k}")
        }
    };
    for (k, va) in ta {
        match tb.get(k) {
            None => remove.push(join(k)),
            Some(vb) if vb != va => {
                if va.is_table() && vb.is_table() {
                    ops(va, vb, &join(k), remove, set);
                } else {
                    set.insert(join(k), vb.clone());
                }
            }
            Some(_) => {}
        }
    }
    for (k, vb) in tb {
        if !ta.contains_key(k) {
            set.insert(join(k), vb.clone());
        }
    }
}

/// One mutant as made: its kind, the part it changes, the change, and the
/// `[[mutant]]` entry.
pub(crate) struct Made {
    pub kind: &'static str,
    pub path: String,
    pub what: String,
    pub mutant: Mutant,
}

/// Every mutant of `real`, named and in order: by kind, path and change.
pub(crate) fn make(real: &Toml) -> Vec<Made> {
    let mut proposed = propose(real);
    proposed.sort_by(|x, y| (x.kind, &x.path, &x.what).cmp(&(y.kind, &y.path, &y.what)));
    let mut counter: BTreeMap<&str, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for p in proposed {
        let n = counter.entry(p.kind).or_default();
        *n += 1;
        let name = format!("{}-{:03}", p.kind, n);
        let (mut remove, mut set) = (Vec::new(), BTreeMap::new());
        ops(real, &p.doc, "", &mut remove, &mut set);
        out.push(Made {
            kind: p.kind,
            mutant: Mutant {
                name,
                says: format!("{}: {}", p.path, p.what),
                remove,
                set,
                ..Mutant::default()
            },
            path: p.path,
            what: p.what,
        });
    }
    out
}

/// The text of a mutants file (`[[mutant]]`), as `gp capture prune
/// --mutants` reads it.
pub(crate) fn file_text(mutants: &[Mutant]) -> Result<String> {
    #[derive(serde::Serialize)]
    struct File<'a> {
        mutant: &'a [Mutant],
    }
    Ok(toml::to_string(&File { mutant: mutants })?)
}

/// The kind a mutant's name gives (`constant-007` is a constant), or
/// none for a name `make` didn't give.
// Used by gp bench clips and a prune once they are here, and the tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn kind_of(name: &str) -> Option<&'static str> {
    let (k, n) = name.rsplit_once('-')?;
    (n.len() >= 3 && n.bytes().all(|b| b.is_ascii_digit()))
        .then(|| KINDS.iter().find(|(x, _)| *x == k).map(|(x, _)| *x))
        .flatten()
}

/// `gp model mutants DIR`: writes the model's mutants to `out` (default
/// DIR/mutants.toml) and says how many of each kind.
pub(crate) fn cli(dir: &Path, out: Option<&Path>) -> Result<Outcome> {
    let model = if dir.is_dir() {
        dir.join(gp_model::MODEL_FILE)
    } else {
        dir.to_owned()
    };
    let text =
        fs::read_to_string(&model).with_context(|| format!("reading {}", model.display()))?;
    let real: Toml = toml::from_str(&text).with_context(|| model.display().to_string())?;
    let made = make(&real);
    let file = out.map_or_else(
        || model.with_file_name("mutants.toml"),
        std::borrow::ToOwned::to_owned,
    );
    let list: Vec<Mutant> = made.iter().map(|m| m.mutant.clone()).collect();
    fs::write(&file, file_text(&list)?).with_context(|| format!("writing {}", file.display()))?;
    let by_kind: Vec<serde_json::Value> = KINDS
        .iter()
        .map(|(k, says)| {
            json!({"kind": k, "says": says, "made": made.iter().filter(|m| m.kind == *k).count()})
        })
        .collect();
    let mut text = vec![format!(
        "{} mutants of {} written to {}",
        made.len(),
        model.display(),
        file.display()
    )];
    for k in &by_kind {
        text.push(format!(
            "{:10}  {:4}  {}",
            k["kind"].as_str().unwrap_or(""),
            k["made"],
            k["says"].as_str().unwrap_or("")
        ));
    }
    let rows: Vec<serde_json::Value> = made
        .iter()
        .map(|m| json!({"name": m.mutant.name, "kind": m.kind, "part": m.path, "change": m.what}))
        .collect();
    Ok(Outcome::ok(
        json!({"model": model, "file": file, "made": made.len(), "kinds": by_kind, "mutants": rows}),
        text.join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Toml {
        toml::from_str(
            r#"
[model]
systems = ["a", "b", "c"]
[entities.p.jump]
gravity = "0.25"
speed = -5
[entities.p.sprite]
tiles = [1, 2]
[entities.p.jump.head]
point = [8, 0]
[[rules.game.states.dying.transition]]
to = "playing"
after = 90
[rules.game.states.dying.transition.if]
counter = "lives"
[[rules.game.states.dying.transition]]
to = "over"
after = 90
[levels.main.tile_kinds]
2 = "block"
"#,
        )
        .unwrap()
    }

    #[test]
    fn mutants_by_kind_leave_presentation_alone() {
        let ps = propose(&model());
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for p in &ps {
            *kinds.entry(p.kind).or_default() += 1;
            assert!(!p.path.contains("sprite"), "{}", p.path);
        }
        // gravity, speed, point x and y; the two afters; two transitions;
        // the condition and the tile kind; two swaps.
        assert_eq!(kinds.get("constant"), Some(&4));
        assert_eq!(kinds.get("period"), Some(&2));
        assert_eq!(kinds.get("transition"), Some(&2));
        assert_eq!(kinds.get("input"), Some(&2));
        assert_eq!(kinds.get("order"), Some(&2));
    }

    #[test]
    fn a_mutant_inside_a_list_sets_the_list() {
        let real = model();
        let ps = propose(&real);
        let p = ps
            .iter()
            .find(|p| p.kind == "period" && p.path.ends_with("[1].after"))
            .unwrap();
        let (mut remove, mut set) = (Vec::new(), BTreeMap::new());
        ops(&real, &p.doc, "", &mut remove, &mut set);
        assert!(remove.is_empty());
        assert_eq!(
            set.keys().collect::<Vec<_>>(),
            ["rules.game.states.dying.transition"]
        );
        let m = Mutant {
            name: "x".into(),
            remove,
            set,
            ..Mutant::default()
        };
        let text = mutate(&toml::to_string(&real).unwrap(), &m).unwrap();
        let back: Toml = toml::from_str(&text).unwrap();
        assert_eq!(back, p.doc);
    }

    #[test]
    fn numbers_change_by_one_or_a_sixteenth() {
        assert_eq!(
            bumped(&Toml::String("0.25".into())),
            Some(Toml::String("0.3125".into()))
        );
        assert_eq!(
            bumped(&Toml::String("-5".into())),
            Some(Toml::String("-4".into()))
        );
        assert_eq!(bumped(&Toml::Integer(7)), Some(Toml::Integer(8)));
        assert_eq!(bumped(&Toml::String("walk".into())), None);
    }

    #[test]
    fn mutants_are_named_by_kind_in_order_and_read_back_as_written() {
        let made = make(&model());
        let names: Vec<&str> = made.iter().map(|m| m.mutant.name.as_str()).collect();
        assert_eq!(names[0], "constant-001");
        assert!(names.contains(&"order-002") && names.contains(&"transition-002"));
        for m in &made {
            assert_eq!(kind_of(&m.mutant.name), Some(m.kind));
            assert_eq!(m.mutant.says, format!("{}: {}", m.path, m.what));
        }
        assert_eq!(kind_of("control"), None);
        assert_eq!(kind_of("constant-1"), None);
        let list: Vec<Mutant> = made.iter().map(|m| m.mutant.clone()).collect();
        #[derive(serde::Deserialize)]
        struct File {
            mutant: Vec<Mutant>,
        }
        let back: File = toml::from_str(&file_text(&list).unwrap()).unwrap();
        assert_eq!(back.mutant, list);
        // The same model gives the same file.
        let again: Vec<Mutant> = make(&model()).into_iter().map(|m| m.mutant).collect();
        assert_eq!(file_text(&again).unwrap(), file_text(&list).unwrap());
    }
}
