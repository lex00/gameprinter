//! Deriving a model's facts from what their evidence cites. For each fact,
//! the refs of the evidence entry that covers it are read back: the
//! constant, immediate, data row or table at the cited line (evidence),
//! the value the code before it computes there or the values its routine
//! stores (code), the lifted rows of the table the line's label names
//! through the asset map (data), and values built from those (derived:
//! a box from its edges, a doubled rate). A fact agrees when one of them,
//! read in one of the encodings, is its value; it differs when its entry
//! cites exactly one number for it and that number isn't it.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use gp_model::Model;
use gp_model::classify::{Class as PathClass, classify};
use gp_model::fixed::Fixed;
use gp_model::model::Ref;
use gp_model::path::{Concrete, Step, display};
use gp_model::validate::{evidence_for, file_ref, pinned};

use crate::asm::{Line, Mode, Source, Stmt};
use crate::encode::{Encoding, decode, matches, width_of};
use crate::eval;

/// Where a derived value comes from, in the order agreement prefers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum From {
    /// The number at the cited line: a constant, an immediate, a data row,
    /// the table a label starts.
    Evidence,
    /// What the code before the cited line computes, or its routine stores.
    Code,
    /// The ROM's data, lifted through the asset map.
    Data,
    /// Measured from recordings.
    Recordings,
    /// Built from other values: a box from its edges, a doubled rate.
    Derived,
}

impl From {
    pub fn name(self) -> &'static str {
        match self {
            Self::Evidence => "evidence",
            Self::Code => "code",
            Self::Data => "data",
            Self::Recordings => "recordings",
            Self::Derived => "derived",
        }
    }

    /// The inventory's letter (docs/derivation.md).
    pub fn letter(self) -> char {
        match self {
            Self::Evidence | Self::Code => 'a',
            Self::Data => 'b',
            Self::Recordings => 'c',
            Self::Derived => 'd',
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "evidence" => Self::Evidence,
            "code" => Self::Code,
            "data" => Self::Data,
            "recordings" => Self::Recordings,
            "derived" => Self::Derived,
            _ => return None,
        })
    }
}

/// What a derivation says about one fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// A derived value equals the model's.
    Agree,
    /// The one value derived for it isn't the model's.
    Differ,
    /// Derived for a path the model doesn't have.
    New,
    /// Nothing derived it.
    Underivable,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Self::Agree => "agree",
            Self::Differ => "differ",
            Self::New => "new",
            Self::Underivable => "underivable",
        }
    }
}

/// One fact's derivation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Derived {
    pub path: String,
    /// The model's value, as its text (`None` for a new fact).
    pub value: Option<String>,
    pub status: Status,
    /// The deriver that agreed or differed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<From>,
    /// Every deriver that agreed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<From>,
    /// The value derived, when it differs or is new.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed: Option<String>,
    /// How the raw number reads as the value (`16ths`, `signed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// How sure, in percent.
    pub confidence: u8,
    /// The refs the value was read at, `file:line` (with `@pin`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    /// For an underivable fact: the inventory class it could be derived by
    /// (`a` to `d`) or `e`, and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub could: Option<char>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Other derivers' values that differ from an agreed one: the
    /// deriver and its value (a recording measuring a stated speed
    /// differently).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub against: Vec<(From, String)>,
    /// The evidence entry's source: `stated`, `inferred` or `observed`.
    pub source: String,
    /// The path's class: mechanic, parameter, content, presentation.
    pub class: String,
}

/// What derivation reads.
pub struct Inputs<'a> {
    /// Parsed sources, by the pin a ref names with `in` (`None`: the repo).
    pub sources: &'a BTreeMap<Option<String>, Source>,
    /// Lifted tables by the disassembly label the asset map gives them.
    pub tables: &'a BTreeMap<String, Vec<Vec<Option<i64>>>>,
    /// Which derivers to run.
    pub from: BTreeSet<From>,
}

/// One number a ref gives.
#[derive(Debug, Clone)]
struct Candidate {
    raw: i64,
    width: Option<u8>,
    from: From,
    at: String,
    /// Read straight off the line (a constant or an immediate), not from a
    /// table or a routine.
    direct: bool,
    /// Written in the cited line's comment.
    comment: bool,
}

/// A table's rows a ref gives, kept for values built from a row.
#[derive(Debug, Clone)]
struct Row {
    items: Vec<i64>,
    from: From,
    at: String,
}

#[derive(Default)]
struct Gathered {
    nums: Vec<Candidate>,
    rows: Vec<Row>,
}

fn ref_name(r: &Ref) -> String {
    match &r.within {
        Some(w) => format!("{}@{}", r.at, w),
        None => r.at.clone(),
    }
}

/// The label a quote or a line names: `Name:` at its start.
fn quoted_label(q: &str) -> Option<&str> {
    let q = q.trim();
    let name = q.strip_suffix(':')?;
    crate::asm::is_global(name)
        .then_some(name)
        .filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

fn gather(r: &Ref, inputs: &Inputs<'_>, out: &mut Gathered) {
    let Some((file, line)) = file_ref(&r.at) else {
        return;
    };
    let at = ref_name(r);
    let key = r
        .within
        .as_deref()
        .and_then(pinned)
        .map(|(n, _)| n.to_owned());
    let want = |f: From| inputs.from.contains(&f);
    let mut label = r.quote.as_deref().and_then(quoted_label).map(str::to_owned);
    let mut add = |raw: i64, width: Option<u8>, from: From, direct: bool, out: &mut Gathered| {
        if want(from) {
            out.nums.push(Candidate {
                raw,
                width,
                from,
                at: at.clone(),
                direct,
                comment: false,
            });
        }
    };
    if let Some(src) = inputs.sources.get(&key)
        && let Some(f) = src.files.get(file)
        && let Some(l) = f.line(line)
    {
        if label.is_none() {
            label = l.global_label().map(str::to_owned);
        }
        read_line(src, file, line, l, &at, inputs, out, &mut add);
        if want(From::Evidence) {
            let comment = l.comment.as_deref().unwrap_or("");
            // A comment the ref quotes is what the person meant to cite.
            let quoted = r
                .quote
                .as_deref()
                .is_some_and(|q| !q.trim().is_empty() && comment.contains(q.trim()));
            for n in comment_numbers(comment) {
                out.nums.push(Candidate {
                    raw: n,
                    width: None,
                    from: From::Evidence,
                    at: format!("{at} (comment)"),
                    direct: quoted,
                    comment: true,
                });
            }
        }
    }
    // The ROM's data through the asset map: the lifted rows of the table
    // the line's label names, whether or not the disassembly is here.
    if want(From::Data)
        && let Some(rows) = label.as_deref().and_then(|l| inputs.tables.get(l))
    {
        push_rows(rows, From::Data, &at, None, out, &mut add);
    }
}

/// The numbers a comment writes: decimal, `$hex` and `0x` hex, with a
/// leading minus, each as a whole word (not inside a name or a range).
fn comment_numbers(text: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let prev_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        let neg =
            b[i] == b'-' && i + 1 < b.len() && (b[i + 1].is_ascii_digit() || b[i + 1] == b'$');
        let start = if neg { i + 1 } else { i };
        if !prev_ok || start >= b.len() {
            i += 1;
            continue;
        }
        let (radix, digits_at) = if b[start] == b'$' {
            (16, start + 1)
        } else if b[start..].starts_with(b"0x") {
            (16, start + 2)
        } else if b[start].is_ascii_digit() {
            (10, start)
        } else {
            i += 1;
            continue;
        };
        let n = b[digits_at..]
            .iter()
            .take_while(|c| {
                if radix == 16 {
                    c.is_ascii_hexdigit()
                } else {
                    c.is_ascii_digit()
                }
            })
            .count();
        let end = digits_at + n;
        let next_ok = end >= b.len() || !(b[end].is_ascii_alphanumeric() || b[end] == b'_');
        if n > 0
            && next_ok
            && let Ok(v) = i64::from_str_radix(&text[digits_at..end], radix)
        {
            out.push(if neg { -v } else { v });
        }
        i = end.max(i + 1);
    }
    out
}

type Add<'a> = dyn FnMut(i64, Option<u8>, From, bool, &mut Gathered) + 'a;

fn push_rows(
    rows: &[Vec<Option<i64>>],
    from: From,
    at: &str,
    width: Option<u8>,
    out: &mut Gathered,
    add: &mut Add<'_>,
) {
    for row in rows {
        for v in row.iter().flatten() {
            add(*v, width, from, false, out);
        }
        if row.iter().all(Option::is_some) {
            out.rows.push(Row {
                items: row.iter().flatten().copied().collect(),
                from,
                at: at.to_owned(),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn read_line(
    src: &Source,
    file: &str,
    line: usize,
    l: &Line,
    at: &str,
    inputs: &Inputs<'_>,
    out: &mut Gathered,
    add: &mut Add<'_>,
) {
    let want = |f: From| inputs.from.contains(&f);
    match &l.stmt {
        Stmt::Const { value, .. } => {
            if let Some(v) = src.eval(value) {
                add(v, None, From::Evidence, true, out);
            }
        }
        Stmt::Data { width, .. } => {
            if let Some(rows) = src.table_at(file, line) {
                // The cited row first: a row on its own line is direct.
                let direct = rows.len() == 1;
                for (k, row) in rows.iter().enumerate() {
                    for v in row.iter().flatten() {
                        add(*v, Some(*width), From::Evidence, direct && k == 0, out);
                    }
                    if row.iter().all(Option::is_some) {
                        out.rows.push(Row {
                            items: row.iter().flatten().copied().collect(),
                            from: From::Evidence,
                            at: at.to_owned(),
                        });
                    }
                }
            }
        }
        Stmt::Op { mode, arg, .. } => {
            // A routine's label on the same line as its first instruction:
            // the routine's values too.
            if l.global_label().is_some() && want(From::Code) {
                for f in eval::routine_values(src, file, line) {
                    add(f.value, Some(1), From::Code, false, out);
                }
            }
            if *mode == Mode::Immediate
                && let Some(v) = arg.as_ref().and_then(|e| src.eval(e))
            {
                add(v & 0xFF, Some(1), From::Evidence, true, out);
            } else if let Some(table) = arg
                .as_ref()
                .and_then(|e| Source::name_of(e))
                .and_then(|n| src.table(n))
            {
                // An operand that names a data table: its entries.
                push_rows(&table, From::Evidence, at, Some(1), out, add);
            } else if want(From::Code)
                && let Some(f) = eval::value_at(src, file, line)
            {
                add(f.value, Some(1), From::Code, false, out);
            }
        }
        Stmt::Empty | Stmt::Other => {
            if l.global_label().is_some() {
                // A label: the table it starts, or the routine.
                if let Some(rows) = src.table_at(file, line) {
                    push_rows(&rows, From::Evidence, at, None, out, add);
                } else if want(From::Code) {
                    for f in eval::routine_values(src, file, line) {
                        add(f.value, Some(1), From::Code, false, out);
                    }
                }
            }
        }
    }
}

/// A model value as 1/65536ths, if it's a number.
pub fn number(v: &toml::Value) -> Option<i64> {
    match v {
        toml::Value::Integer(i) => Fixed::from_int(*i).ok().map(Fixed::raw),
        toml::Value::String(s) => Fixed::parse(s).ok().map(Fixed::raw),
        _ => None,
    }
}

/// A model value as text: a string as itself, anything else as TOML writes it.
pub fn value_text(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn fixed_text(raw: i64) -> String {
    Fixed::from_raw(raw).to_string()
}

/// A match: how sure, and how it was read.
#[derive(Debug, Clone)]
struct Hit {
    from: From,
    encoding: Encoding,
    confidence: u8,
    at: String,
}

/// How sure a match is. A small whole number matches something by chance
/// in most routines and tables, unless the cited line itself is that
/// constant or immediate; a whole value read through a fraction (64
/// sixteenths as 4) is likelier chance than the same number written
/// whole; a number the source writes in a comment is a person's word.
fn confidence(c: &Candidate, enc: Encoding, want: i64, pool: usize) -> u8 {
    let base: u8 = match c.from {
        _ if c.comment => 60,
        From::Evidence => 90,
        From::Code => 80,
        From::Data => 85,
        From::Recordings => 80,
        From::Derived => 60,
    };
    let whole = want % (1 << 16) == 0;
    if enc.fractional() && !whole {
        // A small raw number in a table among many reads as some fraction
        // by chance (8 sixteenths is a half).
        if !c.direct && c.raw.abs() < 0x10 && pool > 3 {
            return base.min(40);
        }
        return base;
    }
    let base = if enc.fractional() && c.raw.abs() < 0x100 {
        base.saturating_sub(15)
    } else {
        base
    };
    if c.raw.abs() >= 0x100 || (enc == Encoding::Signed && c.raw >= 0x80) {
        return base;
    }
    let n = (want >> 16).abs();
    // Few numbers cited: a small one is likely the one meant.
    let few = pool <= 3;
    match n {
        0..=2 if few && c.direct => base.min(60),
        0..=2 => base.min(40),
        3..=8 if few => base.saturating_sub(10),
        3..=8 if c.direct && !enc.fractional() => base.saturating_sub(20),
        3..=8 => base.min(40),
        9..=16 if !few && !c.direct => base.saturating_sub(20),
        _ => base,
    }
}

/// Facts whose agreement counts: at least this sure.
pub const SURE: u8 = 50;

fn hits_for(want: i64, g: &Gathered, key: Option<&str>) -> Vec<Hit> {
    let mut hits = Vec::new();
    let pool = g.nums.iter().map(|c| c.raw).collect::<BTreeSet<_>>().len();
    for c in &g.nums {
        let width = width_of(c.raw, c.width);
        for enc in matches(c.raw, width, want) {
            hits.push(Hit {
                from: c.from,
                encoding: enc,
                confidence: confidence(c, enc, want, pool),
                at: c.at.clone(),
            });
        }
        // A rate the code doubles (a shift left), from a fraction.
        for enc in Encoding::ALL.into_iter().filter(|e| e.fractional()) {
            if decode(c.raw, width, enc).map(|v| v * 2) == Some(want) && want % (1 << 16) != 0 {
                hits.push(Hit {
                    from: From::Derived,
                    encoding: enc,
                    confidence: confidence(
                        &Candidate {
                            from: From::Derived,
                            ..c.clone()
                        },
                        enc,
                        want,
                        pool,
                    ),
                    at: format!("{} (doubled)", c.at),
                });
            }
        }
    }
    // A box from a row of edges: left, top, right, bottom.
    if let Some(k) = key {
        for r in g.rows.iter().filter(|r| r.items.len() == 4) {
            let e = &r.items;
            let v = match k {
                "w" => e[2] - e[0],
                "h" => e[3] - e[1],
                _ => continue,
            };
            if v << 16 == want && v > 0 {
                hits.push(Hit {
                    from: if r.from == From::Data {
                        From::Data
                    } else {
                        From::Derived
                    },
                    encoding: Encoding::Whole,
                    confidence: 60,
                    at: format!("{} (edges)", r.at),
                });
            }
        }
    }
    let rank = |e: Encoding| prefer(key, e);
    hits.sort_by_key(|h| (std::cmp::Reverse(h.confidence), rank(h.encoding), h.from));
    hits
}

/// Which encoding a quantity is likelier kept in, by its name, for ties:
/// accelerations and gravities are small fractions (256ths, 4096ths),
/// speeds whole or sixteenths, counts and positions whole.
fn prefer(key: Option<&str>, e: Encoding) -> u8 {
    let k = key.unwrap_or("");
    let order: &[Encoding] = if [
        "gravity",
        "acceleration",
        "deceleration",
        "force",
        "friction",
    ]
    .iter()
    .any(|w| k.contains(w))
    {
        &[
            Encoding::Fraction256,
            Encoding::SignedFraction256,
            Encoding::Fraction4096,
            Encoding::Sixteenths,
            Encoding::SignedSixteenths,
        ]
    } else if k.contains("speed") || k == "below" || k.contains("fall") || k == "bounce" {
        &[
            Encoding::Whole,
            Encoding::Signed,
            Encoding::Sixteenths,
            Encoding::SignedSixteenths,
            Encoding::SignedFraction256,
            Encoding::Fraction256,
        ]
    } else {
        &[Encoding::Whole, Encoding::Signed]
    };
    order.iter().position(|&o| o == e).map_or(9, |p| p as u8)
}

/// The parameter paths `gp model propose` measures (docs/mechanics.md).
fn measured(path: &str) -> bool {
    const MEASURED: [&str; 14] = [
        ".run.walk_speed",
        ".run.run_speed",
        ".run.acceleration",
        ".run.run_acceleration",
        ".jump.speed",
        ".jump.gravity",
        ".jump.gravity_held",
        ".jump.max_fall",
        ".patrol.step",
        ".patrol.period",
        ".walk.speed",
        ".fly.speed",
        "].speed",
        "].gravity",
    ];
    MEASURED.iter().any(|m| path.ends_with(m)) || path.contains(".jump.bands[")
}

/// Why a fact nothing derived is underivable, and the class that could
/// derive it.
fn could(
    path: &str,
    value: &toml::Value,
    class: PathClass,
    refs: &[Ref],
    tried: bool,
) -> (char, String) {
    let cites_script = refs.iter().any(|r| r.at.starts_with("scripts/"));
    let cites_recording = refs.iter().any(|r| r.at.starts_with("recording:"));
    let cites_asm = refs
        .iter()
        .any(|r| file_ref(&r.at).is_some() && !r.at.starts_with("scripts/"));
    if cites_recording {
        return ('c', "measured from recordings it cites; gp model derive --from recordings measures the feel parameters among them".into());
    }
    if measured(path) {
        return (
            'c',
            "a feel parameter gp model propose measures from recordings".into(),
        );
    }
    if cites_script {
        return ('b', "written by a generator script over the lifted assets; gp model derive --from data --assets DIR reruns it".into());
    }
    let numeric = number(value).is_some()
        || value
            .as_array()
            .is_some_and(|a| a.iter().all(|v| number(v).is_some()));
    match class {
        PathClass::Mechanic => (
            'e',
            "a mechanic choice (which component, system, transition or hook): needs a classifier from routines to mechanics".into(),
        ),
        PathClass::Presentation => (
            'e',
            "presentation (sprite tiles, palettes, frames): sprite capture finds these (docs/capture.md)".into(),
        ),
        PathClass::Content if !cites_asm => (
            'e',
            "content placed by a person or a script it doesn't cite".into(),
        ),
        _ if !numeric => (
            'e',
            "a choice, not a number (an action, a flag, a name): no cited line states it".into(),
        ),
        _ if !cites_asm => ('e', "its evidence cites no source line".into()),
        _ if !tried => ('a', "its evidence cites a source that isn't here (download it)".into()),
        _ => (
            'e',
            "no number at the cited lines reads as it: a value the code computes beyond one routine, or a ref that cites the routine rather than the number".into(),
        ),
    }
}

/// A word of a model path as a constant's name would hold it: lower case,
/// no underscores.
fn squash(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// A constant in a file the entry cites whose name holds the fact's own
/// name (its last key, or the entity or level it's under when that key is
/// generic), and whose value reads as the fact: `ENTITY_BATTON = $16` for
/// `assets.spawns.entities.batton = [22]`. Names shorter than four letters
/// aren't matched.
fn named_constant(
    fact: &Concrete,
    want: i64,
    refs: &[Ref],
    inputs: &Inputs<'_>,
) -> Option<(String, String)> {
    // The last key, and the one before it; a generic last key (`start`,
    // `max`) needs both in the name.
    const GENERIC: [&str; 12] = [
        "start", "max", "min", "speed", "size", "step", "period", "every", "after", "value",
        "count", "entry",
    ];
    let keys: Vec<String> = fact
        .iter()
        .rev()
        .filter_map(|s| match s {
            Step::Key(k) => Some(squash(k)),
            Step::Item(_) => None,
        })
        .take(2)
        .collect();
    let last = keys.first()?;
    let words: Vec<&str> = if GENERIC.contains(&last.as_str()) {
        keys.iter().map(String::as_str).collect()
    } else {
        vec![last.as_str()]
    };
    if words.len() < 2 && (words[0].len() < 4 || GENERIC.contains(&words[0])) {
        return None;
    }
    for r in refs {
        let Some((file, _)) = file_ref(&r.at) else {
            continue;
        };
        let key = r
            .within
            .as_deref()
            .and_then(pinned)
            .map(|(n, _)| n.to_owned());
        let Some(src) = inputs.sources.get(&key) else {
            continue;
        };
        for (name, (f, line, e)) in &src.consts {
            let n = squash(name);
            if f != file || !words.iter().all(|w| n.contains(w)) {
                continue;
            }
            let Some(v) = src.eval(e) else { continue };
            if !matches(v, width_of(v, None), want).is_empty() {
                let at = match &r.within {
                    Some(w) => format!("{f}:{line}@{w}"),
                    None => format!("{f}:{line}"),
                };
                return Some((name.clone(), at));
            }
        }
    }
    None
}

/// Derives every fact of `model` from its evidence with the derivers
/// `inputs.from` names.
pub fn derive(model: &Model, inputs: &Inputs<'_>) -> Vec<Derived> {
    let tree = model.to_tree();
    let classes: BTreeMap<String, PathClass> = classify(model)
        .into_iter()
        .map(|c| (c.path, c.class))
        .collect();
    let facts: BTreeMap<Concrete, &toml::Value> = gp_model::facts(&tree).into_iter().collect();
    let covering = evidence_for(model);
    // The facts each entry covers, and how many are numbers.
    let mut by_entry: BTreeMap<String, Vec<Concrete>> = BTreeMap::new();
    for (fact, key) in &covering {
        if let Some(k) = key {
            by_entry.entry(k.clone()).or_default().push(fact.clone());
        }
    }
    let mut gathered: BTreeMap<String, Gathered> = BTreeMap::new();
    let mut tried: BTreeMap<String, bool> = BTreeMap::new();
    let mut out = Vec::new();
    for (fact, key) in covering {
        let path = display(&fact);
        let value = facts[&fact];
        let class = classes.get(&path).copied().unwrap_or(PathClass::Parameter);
        let ev = key.as_ref().and_then(|k| model.evidence.get(k));
        let refs: &[Ref] = ev.map_or(&[], |e| e.refs.as_slice());
        let g = gathered
            .entry(key.clone().unwrap_or_default())
            .or_insert_with(|| {
                let mut g = Gathered::default();
                for r in refs {
                    gather(r, inputs, &mut g);
                }
                g
            });
        let here = tried
            .entry(key.clone().unwrap_or_default())
            .or_insert_with(|| {
                refs.iter().any(|r| {
                    let k = r
                        .within
                        .as_deref()
                        .and_then(pinned)
                        .map(|(n, _)| n.to_owned());
                    file_ref(&r.at).is_some_and(|(f, _)| {
                        inputs
                            .sources
                            .get(&k)
                            .is_some_and(|s| s.files.contains_key(f))
                    })
                })
            });
        let last_key = match fact.last() {
            Some(Step::Key(k)) => Some(k.as_str()),
            _ => None,
        };
        let mut d = Derived {
            path: path.clone(),
            value: Some(value_text(value)),
            status: Status::Underivable,
            from: None,
            also: Vec::new(),
            proposed: None,
            encoding: None,
            confidence: 0,
            refs: Vec::new(),
            could: None,
            reason: None,
            against: Vec::new(),
            source: ev.map_or("none".into(), |e| e.source.name().to_owned()),
            class: class.name().to_owned(),
        };
        let wants: Option<Vec<i64>> = match value {
            toml::Value::Array(items) if !items.is_empty() => {
                items.iter().map(number).collect::<Option<Vec<_>>>()
            }
            v => number(v).map(|n| vec![n]),
        };
        if let Some(wants) = wants.as_ref().filter(|_| class != PathClass::Mechanic) {
            // In a list, a zero nothing cites is a default (a lead of
            // [120, 0]) as long as another item is matched.
            let per: Vec<Vec<Hit>> = wants
                .iter()
                .map(|&w| hits_for(w, g, last_key))
                .filter(|h| !h.is_empty() || wants.len() == 1)
                .collect();
            let zeros_only = wants
                .iter()
                .zip(wants.iter().map(|&w| hits_for(w, g, last_key).is_empty()))
                .all(|(&w, none)| !none || w == 0);
            if !per.is_empty() && zeros_only && per.iter().all(|h| !h.is_empty()) {
                let conf = per.iter().map(|h| h[0].confidence).min().unwrap_or(0);
                let best = &per[0][0];
                let mut also: Vec<From> = per[0].iter().map(|h| h.from).collect();
                also.sort();
                also.dedup();
                d.confidence = conf;
                d.from = Some(best.from);
                d.also = also;
                d.encoding = Some(best.encoding.name().to_owned());
                d.refs = per
                    .iter()
                    .map(|h| h[0].at.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if conf >= SURE {
                    d.status = Status::Agree;
                } else {
                    d.reason = Some(format!(
                        "only a small whole number matched ({}), which could be chance",
                        best.at
                    ));
                }
            } else if wants.len() == 1 {
                // Differs: the entry covers this one number and cites one.
                let numeric_facts = key.as_ref().and_then(|k| by_entry.get(k)).map_or(0, |fs| {
                    fs.iter().filter(|f| number(facts[*f]).is_some()).count()
                });
                let direct: BTreeSet<(i64, Option<u8>)> = g
                    .nums
                    .iter()
                    .filter(|c| c.direct && c.from == From::Evidence)
                    .map(|c| (c.raw, c.width))
                    .collect();
                if numeric_facts == 1
                    && direct.len() == 1
                    && let Some(&(raw, width)) = direct.iter().next()
                {
                    let want = wants[0];
                    let width = width_of(raw, width);
                    let best = Encoding::ALL
                        .into_iter()
                        .filter_map(|e| decode(raw, width, e).map(|v| (e, v)))
                        .min_by_key(|&(_, v)| (v - want).unsigned_abs());
                    if let Some((enc, v)) = best {
                        let at = g
                            .nums
                            .iter()
                            .find(|c| c.direct)
                            .map(|c| c.at.clone())
                            .unwrap_or_default();
                        d.status = Status::Differ;
                        d.from = Some(From::Evidence);
                        d.proposed = Some(fixed_text(v));
                        d.encoding = Some(enc.name().to_owned());
                        d.confidence = 70;
                        d.refs = vec![at];
                    }
                }
            }
        }
        if d.status == Status::Underivable
            && inputs.from.contains(&From::Evidence)
            && let Some(w) = wants.as_ref().filter(|w| w.len() == 1)
            && let Some((name, at)) = named_constant(&fact, w[0], refs, inputs)
        {
            d.status = Status::Agree;
            d.from = Some(From::Evidence);
            d.also = vec![From::Evidence];
            d.encoding = Some(format!("the constant {name}"));
            d.confidence = 80;
            d.refs = vec![at];
            d.reason = None;
        }
        if d.status == Status::Underivable {
            let (c, why) = could(&path, value, class, refs, *here);
            d.could = Some(c);
            if d.reason.is_none() {
                d.reason = Some(why);
            }
        }
        out.push(d);
    }
    out
}

/// Folds a later deriver's result for a path into `all`: an agreement
/// replaces an underivable result, a difference is kept beside it, and a
/// path the model lacks is added as new.
pub fn merge(all: &mut Vec<Derived>, more: Vec<Derived>) {
    let mut at: BTreeMap<String, usize> = all
        .iter()
        .enumerate()
        .map(|(i, d)| (d.path.clone(), i))
        .collect();
    for m in more {
        match at.get(&m.path) {
            None => {
                at.insert(m.path.clone(), all.len());
                all.push(m);
            }
            Some(&i) => {
                let old = &mut all[i];
                match (old.status, m.status) {
                    (Status::Agree, Status::Agree) => {
                        if let Some(f) = m.from
                            && !old.also.contains(&f)
                        {
                            old.also.push(f);
                            old.also.sort();
                        }
                    }
                    (Status::Agree, Status::Differ) => {
                        if let (Some(f), Some(p)) = (m.from, m.proposed) {
                            old.against.push((f, p));
                        }
                    }
                    (Status::Agree, _) => {}
                    (_, Status::Agree) | (Status::Underivable, Status::Differ) => {
                        let (source, class) = (old.source.clone(), old.class.clone());
                        *old = m;
                        old.source = source;
                        old.class = class;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Counts for one model: hand-written facts (stated or inferred), how many
/// any deriver derived (agree or differ) and how many agree.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Score {
    pub facts: usize,
    pub hand_written: usize,
    pub derived: usize,
    pub agree: usize,
    pub differ: usize,
    pub new: usize,
    /// Agreed facts another deriver contradicts.
    pub contradicted: usize,
    /// Agreements by deriver.
    pub by_from: BTreeMap<String, usize>,
    /// Underivable facts by the class that could derive them (`a` to `e`).
    pub could: BTreeMap<String, usize>,
}

impl Score {
    pub fn of(all: &[Derived]) -> Self {
        let mut s = Score::default();
        for d in all {
            if d.status == Status::New {
                s.new += 1;
                continue;
            }
            s.facts += 1;
            if d.source == "observed" {
                continue;
            }
            s.hand_written += 1;
            match d.status {
                Status::Agree => {
                    s.derived += 1;
                    s.agree += 1;
                    s.contradicted += usize::from(!d.against.is_empty());
                    *s.by_from
                        .entry(d.from.map_or("?", From::name).to_owned())
                        .or_default() += 1;
                }
                Status::Differ => {
                    s.derived += 1;
                    s.differ += 1;
                }
                _ => {
                    *s.could
                        .entry(d.could.map_or("e".into(), |c| c.to_string()))
                        .or_default() += 1;
                }
            }
        }
        s
    }

    /// Derived over hand-written.
    pub fn derivable_share(&self) -> f64 {
        if self.hand_written == 0 {
            0.0
        } else {
            self.derived as f64 / self.hand_written as f64
        }
    }

    /// Agree over derived, an agreement another deriver contradicts
    /// counted as a difference.
    pub fn agree_share(&self) -> f64 {
        if self.derived == 0 {
            0.0
        } else {
            (self.agree - self.contradicted) as f64 / self.derived as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = r#"
[model]
schema = "0.3.0"
name = "madeup"
title = "Made Up"
source = "madeup"
view = "side"
frame_rate = "60"
systems = ["platformer.player"]

[model.extensions]
platformer = "0.3.0"

[input]
device = "nes-pad"

[input.actions]
jump = "a"
left = "left"
right = "right"

[entities.hero]
role = "player"
size = [16, 16]

[entities.hero.hitbox]
x = 2
y = 4
w = 12
h = 10

[entities.hero.run]
air_control = true
walk_speed = "1.5"

[entities.hero.jump]
action = "jump"
speed = -4
gravity = "0.4375"
max_fall = 5

[evidence.input]
source = "stated"
confidence = 100

[evidence.entities]
source = "stated"
confidence = 100

[[evidence.entities.refs]]
at = "game.s:1"

[evidence."entities.hero.hitbox"]
source = "stated"
confidence = 100

[[evidence."entities.hero.hitbox".refs]]
at = "game.s:6"
quote = "Boxes:"

[evidence."entities.hero.run"]
source = "stated"
confidence = 100

[[evidence."entities.hero.run".refs]]
at = "game.s:9"

[evidence."entities.hero.jump"]
source = "stated"
confidence = 100

[[evidence."entities.hero.jump".refs]]
at = "game.s:2"

[[evidence."entities.hero.jump".refs]]
at = "game.s:11"

[evidence."entities.hero.jump.max_fall"]
source = "stated"
confidence = 100

[[evidence."entities.hero.jump.max_fall".refs]]
at = "game.s:3"
"#;

    // A made-up game's source: a size constant, a jump speed, a fall cap
    // that the model gets wrong, a box as edges, a speed table, and a
    // routine that sets gravity.
    const GAME: &str = "SIZE = 16\n\
        JUMP = $fc\n\
        CAP = 4\n\
        \n\
        \n\
        Boxes:\n\
        \x20 .db $02, $04, $0e, $0e\n\
        \n\
        \x20 .db $18, $28\n\
        \n\
        SetGravity:\n\
        \x20 lda #$70\n\
        \x20 sta Gravity\n\
        \x20 rts\n";

    fn run(from: &[From]) -> Vec<Derived> {
        let model = Model::parse(MODEL).expect("the made-up model parses");
        let mut src = Source::default();
        src.add("game.s", GAME);
        let sources = BTreeMap::from([(None, src)]);
        let tables = BTreeMap::new();
        derive(
            &model,
            &Inputs {
                sources: &sources,
                tables: &tables,
                from: from.iter().copied().collect(),
            },
        )
    }

    fn at<'a>(all: &'a [Derived], path: &str) -> &'a Derived {
        all.iter().find(|d| d.path == path).expect(path)
    }

    #[test]
    fn facts_read_back_from_their_evidence() {
        let all = run(&[From::Evidence, From::Code, From::Derived]);
        let d = at(&all, "entities.hero.run.walk_speed");
        assert_eq!(
            (d.status, d.encoding.as_deref()),
            (Status::Agree, Some("16ths"))
        );
        let d = at(&all, "entities.hero.jump.speed");
        assert_eq!((d.status, d.from), (Status::Agree, Some(From::Evidence)));
        // 0x70 / 256 is 0.4375, stored by the routine the ref cites.
        let d = at(&all, "entities.hero.jump.gravity");
        assert_eq!((d.status, d.from), (Status::Agree, Some(From::Code)));
        // The box's width and height are its edges' differences.
        let d = at(&all, "entities.hero.hitbox.w");
        assert_eq!((d.status, d.from), (Status::Agree, Some(From::Derived)));
        let d = at(&all, "entities.hero.hitbox.h");
        assert_eq!(d.status, Status::Agree);
        // The size is the constant the entry cites.
        assert_eq!(at(&all, "entities.hero.size").status, Status::Agree);
        // The model says 5; the one number its entry cites is 4.
        let d = at(&all, "entities.hero.jump.max_fall");
        assert_eq!(
            (d.status, d.proposed.as_deref()),
            (Status::Differ, Some("4"))
        );
        // An action is a choice, not a number.
        let d = at(&all, "input.actions.jump");
        assert_eq!((d.status, d.could), (Status::Underivable, Some('e')));
        let s = Score::of(&all);
        assert_eq!(s.differ, 1);
        assert!(s.agree >= 6, "{s:?}");
    }

    #[test]
    fn a_routine_ref_gives_its_stores() {
        // Only the routine's label is cited for gravity's entry.
        let model = Model::parse(&MODEL.replace(
            "at = \"game.s:11\"",
            "at = \"game.s:11\"\nquote = \"SetGravity:\"",
        ))
        .expect("parses");
        let mut src = Source::default();
        src.add("game.s", &GAME.replace("lda #$70", "lda #$38\n  asl"));
        let sources = BTreeMap::from([(None, src)]);
        let tables = BTreeMap::new();
        let all = derive(
            &model,
            &Inputs {
                sources: &sources,
                tables: &tables,
                from: [From::Evidence, From::Code].into_iter().collect(),
            },
        );
        let d = at(&all, "entities.hero.jump.gravity");
        assert_eq!((d.status, d.from), (Status::Agree, Some(From::Code)));
    }

    #[test]
    fn lifted_tables_stand_in_for_a_missing_disassembly() {
        let model = Model::parse(MODEL).expect("parses");
        let sources = BTreeMap::new();
        let tables = BTreeMap::from([(
            "Boxes".to_owned(),
            vec![vec![Some(2), Some(4), Some(14), Some(14)]],
        )]);
        let all = derive(
            &model,
            &Inputs {
                sources: &sources,
                tables: &tables,
                from: [From::Data].into_iter().collect(),
            },
        );
        let d = at(&all, "entities.hero.hitbox.w");
        assert_eq!((d.status, d.from), (Status::Agree, Some(From::Data)));
        let d = at(&all, "entities.hero.run.walk_speed");
        assert_eq!((d.status, d.could), (Status::Underivable, Some('c')));
    }

    #[test]
    fn a_constant_named_like_the_fact() {
        // A made-up constants file cited as a whole, as an entity list is.
        let extra = "[counters.lives]\nstart = 37\n\n[evidence.counters]\nsource = \"stated\"\nconfidence = 100\n\n[[evidence.counters.refs]]\nat = \"kinds.inc:1\"\n\n[evidence.input]";
        let model = Model::parse(&MODEL.replace("[evidence.input]", extra)).expect("parses");
        let mut src = Source::default();
        src.add("game.s", GAME);
        // A generic `start` needs the counter's name beside it.
        src.add(
            "kinds.inc",
            "; made-up kinds\nKIND_START = $25\nLIVES_START = $25\n",
        );
        let sources = BTreeMap::from([(None, src)]);
        let tables = BTreeMap::new();
        let all = derive(
            &model,
            &Inputs {
                sources: &sources,
                tables: &tables,
                from: [From::Evidence].into_iter().collect(),
            },
        );
        let d = at(&all, "counters.lives.start");
        assert_eq!(
            (d.status, d.encoding.as_deref()),
            (Status::Agree, Some("the constant LIVES_START"))
        );
    }
}
