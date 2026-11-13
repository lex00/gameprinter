//! `gp catalog`: the pattern library's finders run on a template, beside
//! what its model holds (docs/catalog-patterns.md). Four finders, each from
//! what the machine has:
//!
//! - model patterns: each entity's patterns from its components and its
//!   hook's `like`, and the share of its parameter facts that belong to
//!   general components rather than hooks;
//! - table shapes: every asset of `assets/<source>.toml` sorted into an
//!   asset pattern by its format, shape and name, against the assets the
//!   model's slots and hook tables read;
//! - slot arrays from labels: RAM label families that form object slot
//!   arrays, against the state mapping's `[slots]`;
//! - behaviour from traces: with `--recording`, each tracked object's
//!   motion fitted to the pattern library (`gp_track::behaviour`), grouped
//!   by object ID, against the entity the mapping's `[[slots.kind]]` names
//!   for it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use serde::Serialize;
use serde_json::json;

use gp_evidence::Paths;
use gp_model::Model;
use gp_model::classify::Class;

use crate::Outcome;

#[derive(Args)]
pub struct CatalogArgs {
    /// A template directory holding model.toml (and mapping.toml for the
    /// RAM and behaviour finders), a template id under templates/ or a game
    /// in the games checkout.
    pub template: PathBuf,
    /// Store recordings (IDs with frame traces) whose tracked objects the
    /// behaviour finder fits to the pattern library.
    #[arg(long = "recording", value_name = "ID")]
    pub recordings: Vec<String>,
    /// Which frames of each recording: `N`, `A..B`, `A..=B` or `A..`.
    #[arg(long, default_value = "1..")]
    pub frames: String,
    /// The game's labels file, instead of the one `gp labels` finds for the
    /// model's source.
    #[arg(long)]
    pub labels: Option<PathBuf>,
    /// The game's built module, whose hooks the model patterns read.
    #[arg(long, value_name = "FILE")]
    pub module: Option<PathBuf>,
    /// The model's lifted assets, for `--module` (default:
    /// $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
}

/// The pattern library's names, with the component that runs each, if one
/// does (docs/catalog-patterns.md).
pub const PATTERNS: &[(&str, &str)] = &[
    ("walk", "platformer.walk"),
    ("fly", "platformer.fly"),
    ("fall", "platformer.fall"),
    ("shuttle", "platformer.path"),
    ("oscillate", "platformer.oscillate"),
    ("leap", "platformer.leap"),
    ("hop", "platformer.hop"),
    ("pursue", ""),
    ("orbit", ""),
    ("throw", "platformer.shoot"),
    ("projectile", ""),
    ("leave-view", "platformer.leave_view"),
    ("carry", "platformer.carry"),
    ("touch", "platformer.contact"),
    ("boss", ""),
    ("player-physics", "platformer.run"),
];

/// The pattern a platformer component is an instance of.
pub fn component_pattern(c: &str) -> &'static str {
    match c {
        "run" | "jump" | "climb" | "knockback" => "player-physics",
        "walk" => "walk",
        "patrol" | "path" => "shuttle",
        "fly" => "fly",
        "fall" => "fall",
        "hop" => "hop",
        "oscillate" => "oscillate",
        "leap" => "leap",
        "leave_view" => "leave-view",
        "shoot" => "throw",
        "carry" | "carryable" => "carry",
        _ => "touch",
    }
}

/// Patterns that are motion, which the behaviour finder can see.
const MOTION: &[&str] = &[
    "walk",
    "fly",
    "fall",
    "shuttle",
    "oscillate",
    "leap",
    "hop",
    "pursue",
    "orbit",
    "projectile",
    "boss",
];

#[derive(Debug, Serialize)]
struct EntityPatterns {
    entity: String,
    /// (pattern, how: `platformer.<c>` or `hook <name>`).
    patterns: Vec<(String, String)>,
}

/// Each entity's patterns, from its components (its machine's states'
/// included) and its hook's records.
fn model_patterns(model: &Model, exts: &gp_sim::Exts) -> Vec<EntityPatterns> {
    let mut out = Vec::new();
    for (name, e) in &model.entities {
        let mut got: Vec<(String, String)> = Vec::new();
        let mut add = |p: &str, how: String| {
            if !got.iter().any(|(q, h)| q == p && *h == how) {
                got.push((p.to_owned(), how));
            }
        };
        let mut states = vec![e.clone()];
        if let Some(m) = &e.core.machine {
            for s in m.states.keys() {
                if let Ok(se) = e.in_state(s) {
                    states.push(se);
                }
            }
        }
        for se in &states {
            if let Ok(toml::Value::Table(t)) = toml::Value::try_from(&se.platformer) {
                for c in t.keys() {
                    add(component_pattern(c), format!("platformer.{c}"));
                }
            }
            if let Some(hook) = &se.core.ext {
                for r in exts.get(hook).map(|h| h.describe()).unwrap_or_default() {
                    let like = if r.like.is_empty() { "?" } else { r.like };
                    add(like, format!("hook {hook}"));
                }
            }
        }
        out.push(EntityPatterns {
            entity: name.clone(),
            patterns: got,
        });
    }
    out
}

/// The asset pattern of one asset map entry, by its format, shape and name.
pub fn asset_pattern(format: &str, name: &str, columns: &[String]) -> &'static str {
    let n = name.to_ascii_lowercase();
    let has = |words: &[&str]| {
        words.iter().any(|w| n.contains(w))
            || columns
                .iter()
                .any(|c| words.iter().any(|w| c.to_ascii_lowercase().contains(w)))
    };
    match format {
        "chr-2bpp" | "mm2-chr-uploads" => return "tile graphics",
        "palette" | "mm2-palettes" => return "palettes",
        "metatiles" | "mm2-metatiles" => return "metatile sets",
        "smb-area-objects" | "smb2-level-objects" | "mm2-screens" | "mm2-overlay-sets" => {
            return "object lists";
        }
        "smb-enemy-objects" | "smb2-enemy-objects" | "mm2-spawns" => return "spawn lists",
        "pointers" => return "pointer tables",
        "smb-music" => return "music and sound",
        "ppu-strings" => return "text",
        "blobs" => return "code and other bytes",
        _ => {}
    }
    if has(&[
        "music",
        "note",
        "sfx",
        "envelope",
        "freq",
        "dmc",
        "dpcm",
        "noise",
        "sound",
        "tempo",
        "instrument",
    ]) {
        "music and sound"
    } else if has(&["palette", "_pal", "color", "colour", "fade", "brightness"]) {
        "palettes"
    } else if has(&["hitbox", "bounding", "collision", "solid", "wall_", "_box"]) {
        "collision tables"
    } else if has(&["rng", "random", "delay", "ranges", "heights", "_times"]) {
        "timing and chance tables"
    } else if has(&[
        "ppu_",
        "nametable",
        "_nt_",
        "layout",
        "buffer",
        "cast_roll",
        "dream",
        "chr_bank",
    ]) {
        "screen layouts"
    } else if has(&[
        "text",
        "name_text",
        "_name",
        "message",
        "story",
        "string",
        "letter",
    ]) {
        "text"
    } else if has(&["ptr", "pointer", "address", "addr"]) {
        "pointer tables"
    } else if has(&[
        "speed", "vel", "accel", "gravity", "force", "jump", "physics", "friction", "fall",
        "throw", "climb", "bounce", "decel", "stats", "cost",
    ]) {
        "physics tables"
    } else if has(&["position", "offset", "_x", "_y", "mirror", "part_rows"]) {
        "motion tables"
    } else if has(&["frame", "sprite", "oam", "anim", "tile", "attr", "metatile"]) {
        "sprite frames and layouts"
    } else if has(&[
        "enemy", "boss", "weapon", "damage", "hp", "object", "ai", "phase", "behavior", "init",
    ]) {
        "per-kind property tables"
    } else if has(&[
        "area",
        "world",
        "level",
        "screen",
        "checkpoint",
        "link",
        "stage",
        "room",
        "page",
        "timer",
        "start",
        "scenery",
        "ground",
        "warp",
        "destination",
        "scroll",
        "gate",
        "coins",
        "demo",
    ]) {
        "level structure tables"
    } else {
        "unclassified"
    }
}

#[derive(Debug, Default, Serialize)]
struct Shapes {
    /// pattern -> (assets, of them read by the model)
    by_pattern: BTreeMap<&'static str, (usize, usize)>,
    total: usize,
    classified: usize,
    unclassified: Vec<String>,
}

/// The assets the model reads: its slots' `from.asset`.
fn model_assets(model_text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Ok(t) = toml::from_str::<toml::Table>(model_text)
        && let Some(toml::Value::Table(assets)) = t.get("assets")
    {
        for slot in assets.values() {
            if let Some(a) = slot
                .get("from")
                .and_then(|f| f.get("asset"))
                .and_then(|a| a.as_str())
            {
                out.insert(a.to_owned());
            }
        }
    }
    out
}

fn table_shapes(map_text: &str, used: &BTreeSet<String>) -> Result<Shapes> {
    let v = toml::Value::Table(toml::from_str::<toml::Table>(map_text)?);
    let mut s = Shapes::default();
    for a in v
        .get("asset")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        let format = a.get("format").and_then(|f| f.as_str()).unwrap_or("");
        let name = a.get("name").and_then(|f| f.as_str()).unwrap_or("");
        let columns: Vec<String> = a
            .get("column")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
            .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(str::to_owned))
            .collect();
        let p = asset_pattern(format, name, &columns);
        let e = s.by_pattern.entry(p).or_default();
        e.0 += 1;
        if used.contains(name) {
            e.1 += 1;
        }
        s.total += 1;
        if p == "unclassified" {
            s.unclassified.push(name.to_owned());
        } else {
            s.classified += 1;
        }
    }
    Ok(s)
}

/// A RAM object slot array a labels family suggests.
#[derive(Debug, Clone, Serialize)]
pub struct SlotArray {
    /// How it was found: `sized` (names with a size) or `family` (names
    /// sharing a suffix at consecutive addresses).
    pub how: &'static str,
    /// Its classes and their slot counts: one class for a sized array.
    pub classes: Vec<(String, u16)>,
    /// Each field, with where each of its classes starts.
    pub fields: Vec<(String, Vec<(String, u16)>)>,
}

impl SlotArray {
    /// The class and slot an address is in, if it's in a field.
    pub fn covers(&self, addr: u16) -> Option<(String, u16)> {
        for (_, layout) in &self.fields {
            for (class, at) in layout {
                let n = self
                    .classes
                    .iter()
                    .find(|c| &c.0 == class)
                    .map_or(1, |c| c.1);
                if addr >= *at && addr < at + n {
                    return Some((class.clone(), addr - at));
                }
            }
        }
        None
    }
}

/// A class's counts in an array's fields: known counts, estimated counts,
/// the sum of its offsets and how many fields it's in.
type ClassCounts = (BTreeMap<u16, usize>, BTreeMap<u16, usize>, i64, i64);

/// A pattern's tracks and the first one's numbers.
type Votes = (usize, Vec<(String, f64)>);

/// A RAM label: name, address, size.
pub type RamLabel = (String, u16, u16);

/// A name's class prefix and field suffix: before and after its first `_`
/// (`Enemy_X_Position`), or its first capitalized word and the rest
/// (`ObjectXLo`).
fn split_name(n: &str) -> Option<(&str, &str)> {
    if let Some((pre, suf)) = n.split_once('_') {
        return (!pre.is_empty() && !suf.is_empty()).then_some((pre, suf));
    }
    let b = n.as_bytes();
    if !b.first()?.is_ascii_uppercase() {
        return None;
    }
    let cut = (1..b.len()).find(|&i| b[i].is_ascii_uppercase())?;
    Some((&n[..cut], &n[cut..]))
}

/// Object slot arrays from RAM labels: (name, addr, size) triples.
///
/// A sized array is three or more labels of one size from 2 to 64 (the
/// fields of a struct of arrays). A family is a field suffix shared by two
/// or more class prefixes at nearby addresses (`Player_X_Position` at $86,
/// `Enemy_X_Position` at $87): each class's slot count is the gap to the
/// next class, the last one's the gap to the next label. Three or more
/// suffixes with the same classes are one array, each class taking its
/// most common count; then any other label whose prefix is one of its
/// classes (`Enemy_Flag`) is a field of that class alone.
pub fn slot_arrays(labels: &[(String, u16, u16)]) -> Vec<SlotArray> {
    let mut out = Vec::new();
    let mut by_size: BTreeMap<u16, Vec<(String, u16)>> = BTreeMap::new();
    for (n, a, s) in labels {
        if (2..=64).contains(s) {
            by_size.entry(*s).or_default().push((n.clone(), *a));
        }
    }
    for (size, mut fields) in by_size {
        if fields.len() >= 3 {
            fields.sort_by_key(|f| f.1);
            out.push(SlotArray {
                how: "sized",
                classes: vec![("slot".into(), size)],
                fields: fields
                    .into_iter()
                    .map(|(n, a)| (n, vec![("slot".to_owned(), a)]))
                    .collect(),
            });
        }
    }
    let mut addrs: Vec<u16> = labels.iter().map(|l| l.1).collect();
    addrs.sort_unstable();
    addrs.dedup();
    let next_after = |a: u16| addrs.iter().copied().find(|x| *x > a);
    // How often each prefix is used, to pick between names of one address.
    let mut uses: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_suffix: BTreeMap<&str, Vec<(&str, u16)>> = BTreeMap::new();
    for (n, a, _) in labels {
        if let Some((pre, suf)) = split_name(n) {
            *uses.entry(pre).or_default() += 1;
            by_suffix.entry(suf).or_default().push((pre, *a));
        }
    }
    // Each suffix's layout: [(class, addr, count, known)], `known` when
    // the count is the gap to the next class rather than to the next label.
    type Layout<'a> = Vec<(&'a str, u16, u16, bool)>;
    let mut fields: Vec<(&str, Layout<'_>)> = Vec::new();
    for (suf, mut members) in by_suffix {
        members.sort_by_key(|m| (m.1, std::cmp::Reverse(uses.get(m.0).copied().unwrap_or(0))));
        members.dedup_by_key(|m| m.1);
        members.dedup_by_key(|m| m.0);
        if members.len() < 2 {
            continue;
        }
        let span = members.last().map(|m| m.1).unwrap_or(0) - members[0].1;
        if span > 64 {
            continue;
        }
        let mut layout = Vec::new();
        for (k, m) in members.iter().enumerate() {
            let (end, known) = match members.get(k + 1) {
                Some(n) => (n.1, true),
                None => (next_after(m.1).unwrap_or(m.1 + 1), false),
            };
            layout.push((m.0, m.1, (end - m.1).min(32), known));
        }
        fields.push((suf, layout));
    }
    // Classes that share a suffix are one array (union-find by name).
    let mut group: BTreeMap<&str, usize> = BTreeMap::new();
    let mut parent: Vec<usize> = Vec::new();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for (_, l) in &fields {
        let ids: Vec<usize> = l
            .iter()
            .map(|(c, ..)| {
                *group.entry(c).or_insert_with(|| {
                    parent.push(parent.len());
                    parent.len() - 1
                })
            })
            .collect();
        for w in ids.windows(2) {
            let (a, b) = (root(&mut parent, w[0]), root(&mut parent, w[1]));
            parent[a] = b;
        }
    }
    let mut arrays: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (k, (_, l)) in fields.iter().enumerate() {
        let g = group[l[0].0];
        arrays.entry(root(&mut parent, g)).or_default().push(k);
    }
    for members in arrays.values() {
        if members.len() < 3 {
            continue;
        }
        // Each class: its most common known count, else its most common
        // estimate; its order by its mean offset from the field's first.
        let mut counts: BTreeMap<&str, ClassCounts> = BTreeMap::new();
        for &k in members {
            let l = &fields[k].1;
            for (c, a, n, known) in l {
                let e = counts.entry(c).or_default();
                *if *known { &mut e.0 } else { &mut e.1 }
                    .entry(*n)
                    .or_default() += 1;
                e.2 += i64::from(*a) - i64::from(l[0].1);
                e.3 += 1;
            }
        }
        let mode =
            |m: &BTreeMap<u16, usize>| m.iter().max_by_key(|(n, c)| (**c, **n)).map(|x| *x.0);
        // A class in fewer than three of the fields is a name that happens
        // to share a suffix, not a class of objects.
        let mut classes: Vec<(String, u16, i64)> = counts
            .iter()
            .filter(|(_, e)| e.3 >= 3)
            .map(|(c, (known, est, sum, n))| {
                (
                    (*c).to_owned(),
                    mode(known).or(mode(est)).unwrap_or(1),
                    sum / n.max(&1),
                )
            })
            .collect();
        classes.sort_by_key(|c| c.2);
        let classes: Vec<(String, u16)> = classes.into_iter().map(|(c, n, _)| (c, n)).collect();
        if !classes.iter().any(|c| c.1 > 1) {
            continue;
        }
        let mut fs: Vec<(String, Vec<(String, u16)>)> = members
            .iter()
            .map(|&k| {
                (
                    fields[k].0.to_owned(),
                    fields[k]
                        .1
                        .iter()
                        .map(|(c, a, ..)| ((*c).to_owned(), *a))
                        .collect(),
                )
            })
            .collect();
        let taken: BTreeSet<&str> = members.iter().map(|&k| fields[k].0).collect();
        for (n, a, _) in labels {
            if let Some((pre, suf)) = split_name(n)
                && !taken.contains(suf)
                && classes.iter().any(|c| c.0 == pre && c.1 > 1)
                && !fs.iter().any(|f| f.0 == suf)
            {
                fs.push((suf.to_owned(), vec![(pre.to_owned(), *a)]));
            }
        }
        fs.sort_by_key(|f| f.1.first().map_or(0, |x| x.1));
        out.push(SlotArray {
            how: "family",
            classes,
            fields: fs,
        });
    }
    out
}

#[derive(Debug, Serialize)]
struct KindFit {
    /// The object ID, or the palette (`pal N`) for tracks with no ID.
    kind: String,
    entity: Option<String>,
    tracks: usize,
    pattern: String,
    share: f64,
    params: Vec<(String, f64)>,
    /// The model's motion patterns for the entity.
    model: Vec<String>,
    agrees: Option<bool>,
}

/// The mapping's `[[slots.kind]]`: object ID -> entity.
fn slot_kinds(mapping: &toml::Value) -> BTreeMap<u8, String> {
    let mut out = BTreeMap::new();
    let kinds = mapping
        .get("slots")
        .and_then(|s| s.get("kind"))
        .and_then(|k| k.as_array());
    for k in kinds.into_iter().flatten() {
        let Some(e) = k.get("entity").and_then(|e| e.as_str()) else {
            continue;
        };
        for id in k
            .get("ids")
            .and_then(|i| i.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(id) = id.as_integer().and_then(|i| u8::try_from(i).ok()) {
                out.insert(id, e.to_owned());
            }
        }
    }
    out
}

fn behaviour(
    paths: &Paths,
    ids: &[String],
    frames: &str,
    kinds: &BTreeMap<u8, String>,
    patterns: &[EntityPatterns],
) -> Result<Vec<KindFit>> {
    // id -> pattern -> (tracks, params sums)
    let mut votes: BTreeMap<String, BTreeMap<&'static str, Votes>> = BTreeMap::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut entity_of: BTreeMap<String, String> = BTreeMap::new();
    for id in ids {
        let t = crate::track::tracked(paths, id, frames, 1)?;
        let player: Vec<[i64; 3]> = t
            .report
            .tracks
            .iter()
            .filter(|tr| tr.player)
            .flat_map(|tr| tr.positions.iter().copied())
            .collect();
        for tr in &t.report.tracks {
            if tr.player || tr.seen < 32 {
                continue;
            }
            let oid = match tr.id {
                Some(id) => {
                    let k = format!("${id:02X}");
                    if let Some(e) = kinds.get(&id) {
                        entity_of.insert(k.clone(), e.clone());
                    }
                    k
                }
                None => format!("pal {}", tr.palette),
            };
            *counts.entry(oid.clone()).or_default() += 1;
            if let Some(fit) = gp_track::behaviour::classify(
                &tr.positions,
                (!player.is_empty()).then_some(&player[..]),
            ) {
                let e = votes
                    .entry(oid)
                    .or_default()
                    .entry(fit.pattern)
                    .or_default();
                e.0 += 1;
                if e.1.is_empty() {
                    e.1 = fit
                        .params
                        .iter()
                        .map(|(k, v)| ((*k).to_owned(), *v))
                        .collect();
                }
            }
        }
    }
    let mut out = Vec::new();
    for (oid, v) in votes {
        let total: usize = v.values().map(|x| x.0).sum();
        let Some((best, (n, params))) = v.into_iter().max_by_key(|(_, x)| x.0) else {
            continue;
        };
        let entity = entity_of.get(&oid).cloned();
        let model: Vec<String> = entity
            .as_ref()
            .and_then(|e| patterns.iter().find(|p| &p.entity == e))
            .map(|p| {
                let mut m: Vec<String> = p
                    .patterns
                    .iter()
                    .map(|(q, _)| q.clone())
                    .filter(|q| MOTION.contains(&q.as_str()))
                    .collect();
                m.dedup();
                m
            })
            .unwrap_or_default();
        // A fit of `still` agrees with a model whose entity doesn't move
        // by itself; `walk` and `shuttle` stand for each other, as do `hop`
        // and `leap` with `fall` (a hop falls between hops), and `fly` with
        // `walk` (a walker seen only between turns) and `projectile`.
        let agrees = entity.as_ref().map(|_| {
            let near = |a: &str, b: &str| {
                a == b
                    || matches!(
                        (a, b),
                        ("walk", "shuttle")
                            | ("shuttle", "walk")
                            | ("fall", "hop")
                            | ("fall", "leap")
                            | ("fly", "walk")
                            | ("fly", "projectile")
                    )
            };
            if best == "still" {
                model.is_empty()
            } else {
                model.iter().any(|m| near(best, m) || near(m, best))
            }
        });
        out.push(KindFit {
            tracks: counts.get(&oid).copied().unwrap_or(n),
            kind: oid,
            entity,
            pattern: best.to_owned(),
            share: n as f64 / total.max(1) as f64,
            params,
            model,
            agrees,
        });
    }
    Ok(out)
}

/// Facts that are parameters of a general component vs of a hook.
fn coverage(catalog: &gp_interp::mechanics::Catalog) -> (usize, usize) {
    let (mut comp, mut hook) = (0, 0);
    for c in &catalog.paths {
        if c.class != Class::Parameter {
            continue;
        }
        match c.mechanic.as_deref() {
            Some(m) if m.starts_with("platformer.") || m.starts_with("core.") => comp += 1,
            Some(_) => hook += 1,
            None => {}
        }
    }
    (comp, hook)
}

fn labels_for(model: &Model, args: &CatalogArgs) -> Option<(PathBuf, Vec<RamLabel>)> {
    let path = match &args.labels {
        Some(p) => p.clone(),
        None => {
            let source = model.model.source.as_deref()?;
            crate::labels::locate(source).ok()?
        }
    };
    let text = std::fs::read_to_string(&path).ok()?;
    let file = crate::labels::parse(&text).ok()?;
    Some((
        path,
        file.label
            .iter()
            .map(|l| (l.name.clone(), l.addr, l.size))
            .collect(),
    ))
}

pub(crate) fn run(args: &CatalogArgs, paths: &Paths) -> Result<Outcome> {
    let dir = &crate::level_kit::template_dir(&args.template)?;
    let model = crate::mechanics::load_model(dir)?;
    let model_text = std::fs::read_to_string(crate::mechanics::model_file(dir))?;
    let name = dir
        .canonicalize()
        .unwrap_or_else(|_| dir.clone())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| model.model.name.clone());
    // The hooks: the game's module when given (gameprinter's hooks live in
    // each game's module), else those built into gp, as gp model
    // mechanics reads them.
    let exts = match &args.module {
        Some(m) => {
            crate::hosted::Source::open_dir(dir, args.assets.as_deref(), paths, Some(m))?
                .0
                .exts
        }
        None => std::sync::Arc::new(gp_exts::registry(&model.model.name).0),
    };
    let catalog = gp_interp::mechanics::catalog(&model, &exts);
    let patterns = model_patterns(&model, &exts);
    let hooks: BTreeSet<&str> = model
        .entities
        .values()
        .filter_map(|e| e.core.ext.as_deref())
        .collect();
    let (comp, hook) = coverage(&catalog);
    let covered = comp as f64 / (comp + hook).max(1) as f64;

    // Patterns: which the model holds as data, which only as hooks.
    let mut held: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for e in &patterns {
        let mut seen = BTreeSet::new();
        for (p, how) in &e.patterns {
            if !seen.insert((p.clone(), how.starts_with("hook"))) {
                continue;
            }
            let x = held.entry(p.clone()).or_default();
            if how.starts_with("hook") {
                x.1 += 1;
            } else {
                x.0 += 1;
            }
        }
    }

    // The asset map: the one template.toml's recipe names (a game's
    // package holds its own), else assets/<source>.toml here, as
    // game-printer kept them all.
    let repo = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let source = model.model.source.clone().unwrap_or_default();
    let map_path = crate::level_kit::manifest(dir)
        .1
        .unwrap_or_else(|| repo.join("assets").join(format!("{source}.toml")));
    let shapes = match std::fs::read_to_string(&map_path) {
        Ok(text) => Some(table_shapes(&text, &model_assets(&model_text))?),
        Err(_) => None,
    };

    let mapping: Option<toml::Value> = std::fs::read_to_string(dir.join("mapping.toml"))
        .ok()
        .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
        .map(toml::Value::Table);
    let labels = labels_for(&model, args);
    let arrays = labels.as_ref().map(|(_, l)| slot_arrays(l));
    // The mapping's slots against the arrays.
    let mut slot_match = Vec::new();
    if let (Some(m), Some(arrays)) = (&mapping, &arrays)
        && let Some(slots) = m.get("slots")
    {
        let count = slots.get("count").and_then(|c| c.as_integer()).unwrap_or(0);
        for role in ["active", "id", "state"] {
            let Some(addr) = slots
                .get(role)
                .and_then(|a| a.as_integer())
                .and_then(|a| u16::try_from(a).ok())
            else {
                continue;
            };
            let found = arrays.iter().find_map(|a| {
                a.covers(addr).map(|(class, slot)| {
                    let n = a.classes.iter().find(|c| c.0 == class).map_or(0, |c| c.1);
                    (a.how, class, slot, n)
                })
            });
            slot_match.push(json!({
                "role": role,
                "ram": addr,
                "count": count,
                "found": found.as_ref().map(|(how, class, slot, n)| json!({"how": how, "class": class, "slot": slot, "slots": n})),
            }));
        }
    }

    let kinds = mapping.as_ref().map(slot_kinds).unwrap_or_default();
    let fits = if args.recordings.is_empty() {
        Vec::new()
    } else {
        behaviour(paths, &args.recordings, &args.frames, &kinds, &patterns)?
    };

    // Text.
    let mut t = String::new();
    let _ = writeln!(
        t,
        "{name}: {} hooks, {} debt; parameter facts: {comp} in components, {hook} in hooks ({:.1}% general)",
        hooks.len(),
        catalog.debt.len(),
        covered * 100.0
    );
    let _ = writeln!(t, "patterns the model holds (entities as data / as hooks):");
    for (p, (d, h)) in &held {
        let comp = PATTERNS
            .iter()
            .find(|x| x.0 == p)
            .map_or("not in the library", |x| {
                if x.1.is_empty() {
                    "no component yet"
                } else {
                    x.1
                }
            });
        let _ = writeln!(t, "  {p:<16} {d:>3} / {h:<3} {comp}");
    }
    for e in &patterns {
        let hooked: Vec<String> = e
            .patterns
            .iter()
            .filter(|(_, how)| how.starts_with("hook"))
            .map(|(p, how)| format!("{p} ({})", how.trim_start_matches("hook ")))
            .collect();
        if !hooked.is_empty() {
            let _ = writeln!(t, "  hook  {:<24} {}", e.entity, hooked.join(", "));
        }
    }
    match &shapes {
        Some(s) => {
            let _ = writeln!(
                t,
                "asset map {}: {} assets, {} classified ({:.0}%) into {} patterns (assets / read by the model):",
                map_path.strip_prefix(&repo).unwrap_or(&map_path).display(),
                s.total,
                s.classified,
                100.0 * s.classified as f64 / s.total.max(1) as f64,
                s.by_pattern.len()
            );
            for (p, (n, used)) in &s.by_pattern {
                let _ = writeln!(t, "  {p:<28} {n:>4} / {used}");
            }
        }
        None => {
            let _ = writeln!(t, "asset map: none for source `{source}`");
        }
    }
    match (&labels, &arrays) {
        (Some((path, l)), Some(a)) => {
            let _ = writeln!(
                t,
                "labels {}: {} RAM labels, {} slot arrays proposed",
                path.display(),
                l.len(),
                a.len()
            );
            for arr in a.iter().take(8) {
                let classes: Vec<String> = arr
                    .classes
                    .iter()
                    .map(|(c, n)| format!("{c}:{n}"))
                    .collect();
                let fields: Vec<String> = arr
                    .fields
                    .iter()
                    .take(6)
                    .map(|(f, l)| format!("{f}@${:04X}", l.first().map_or(0, |x| x.1)))
                    .collect();
                let _ = writeln!(
                    t,
                    "  {:<6} [{}] {} fields: {}{}",
                    arr.how,
                    classes.join(" "),
                    arr.fields.len(),
                    fields.join(", "),
                    if arr.fields.len() > 6 { ", ..." } else { "" }
                );
            }
            for s in &slot_match {
                let role = s["role"].as_str().unwrap_or("");
                let ram = s["ram"].as_u64().unwrap_or(0);
                match s["found"].as_object() {
                    Some(f) => {
                        let _ = writeln!(
                            t,
                            "  mapping slots.{role} ${ram:04X}: in {} class {} ({} slots), slot {}",
                            f["how"].as_str().unwrap_or(""),
                            f["class"].as_str().unwrap_or(""),
                            f["slots"],
                            f["slot"]
                        );
                    }
                    None => {
                        let _ =
                            writeln!(t, "  mapping slots.{role} ${ram:04X}: in no proposed array");
                    }
                }
            }
        }
        _ => {
            let _ = writeln!(
                t,
                "labels: none found (docs/formats/labels.md says which recipe writes them)"
            );
        }
    }
    if !fits.is_empty() {
        let _ = writeln!(
            t,
            "behaviour of tracked objects, by object ID (fit vs the model):"
        );
        for f in &fits {
            let _ = writeln!(
                t,
                "  {:<6} {:<24} {:>3} tracks  {:<10} {:>3.0}%  model: {:<20} {}",
                f.kind,
                f.entity.as_deref().unwrap_or("(unmapped)"),
                f.tracks,
                f.pattern,
                f.share * 100.0,
                if f.model.is_empty() {
                    "-".to_owned()
                } else {
                    f.model.join(",")
                },
                match f.agrees {
                    Some(true) => "agrees",
                    Some(false) => "DIFFERS",
                    None => "",
                }
            );
        }
    }

    let json = json!({
        "template": name,
        "hooks": hooks,
        "debt": catalog.debt.len(),
        "facts": {"component": comp, "hook": hook, "covered_share": covered},
        "patterns": held.iter().map(|(p, (d, h))| json!({"pattern": p, "data": d, "hooks": h})).collect::<Vec<_>>(),
        "entities": patterns,
        "assets": shapes,
        "labels": labels.as_ref().map(|(p, l)| json!({"file": p, "ram": l.len()})),
        "slot_arrays": arrays,
        "slots": slot_match,
        "behaviour": fits,
    });
    Ok(Outcome::ok(json, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made-up labels in the two layouts: a family (`Hero_X`, `Foe_X`,
    /// `Shot_X` with five foes) and sized fields.
    #[test]
    fn slot_arrays_from_families_and_sizes() {
        let mut labels = Vec::new();
        for (i, f) in ["X", "Y", "Speed", "State"].iter().enumerate() {
            let base = 0x40 + 8 * i as u16;
            labels.push((format!("Hero_{f}"), base, 1));
            labels.push((format!("Foe_{f}"), base + 1, 1));
            labels.push((format!("Shot_{f}"), base + 6, 1));
        }
        labels.push(("Foe_Flag".to_owned(), 0x30, 1));
        for (i, f) in ["ent_x", "ent_y", "ent_flags"].iter().enumerate() {
            labels.push(((*f).to_owned(), 0x400 + 0x20 * i as u16, 32));
        }
        let a = slot_arrays(&labels);
        let fam = a.iter().find(|a| a.how == "family").unwrap();
        assert_eq!(
            fam.classes,
            vec![("Hero".into(), 1), ("Foe".into(), 5), ("Shot".into(), 2)]
        );
        // The four suffixes, and Foe_Flag as the foes' own field.
        assert_eq!(fam.fields.len(), 5);
        assert_eq!(fam.covers(0x34), Some(("Foe".into(), 4)));
        assert_eq!(fam.covers(0x40 + 8 + 3), Some(("Foe".into(), 2)));
        let sized = a.iter().find(|a| a.how == "sized").unwrap();
        assert_eq!(sized.classes, vec![("slot".into(), 32)]);
        assert_eq!(sized.covers(0x445), Some(("slot".into(), 5)));
    }

    #[test]
    fn asset_patterns_by_format_and_name() {
        assert_eq!(asset_pattern("chr-2bpp", "x", &[]), "tile graphics");
        assert_eq!(
            asset_pattern("table", "player_physics", &[]),
            "physics tables"
        );
        assert_eq!(
            asset_pattern("byte-rows", "enemy_frames", &[]),
            "sprite frames and layouts"
        );
        assert_eq!(
            asset_pattern("table", "boss_ai_flags", &[]),
            "per-kind property tables"
        );
        assert_eq!(
            asset_pattern("byte-rows", "note_lengths", &[]),
            "music and sound"
        );
        assert_eq!(
            asset_pattern("table", "world_areas", &[]),
            "level structure tables"
        );
    }

    #[test]
    fn components_name_their_patterns() {
        for c in gp_model::platformer::EXTENSION.entity_components {
            let p = component_pattern(c);
            assert!(PATTERNS.iter().any(|x| x.0 == p), "{c} -> {p}");
        }
    }
}
