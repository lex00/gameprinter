//! `gp dataset export --hf` and `gp dataset catalogue`
//! (docs/research/hf-datasets.md): a template's model, mapping, clip set
//! and score records written as a Hugging Face style dataset repo, one
//! config a table, and the facts and mechanics of several templates
//! unioned into a cross-game catalogue.
//!
//! | Config | One row a |
//! |---|---|
//! | `facts` | model fact: its path, value, unit, class, mechanic and evidence (kind, confidence, the file and line it cites, the routine there) |
//! | `mechanics` | mechanic part the model uses, at one model path, with the options set there |
//! | `mapping` | state mapping field: the RAM it reads and how, and its role |
//! | `clips` | clip of the scoring corpus: its recipe and curve hash |
//! | `scores` | clip in a score record: the model and corpus it was scored on, and the result |
//! | `measures` | clip, frame and measure of a run (only with `--run`, and for a commercial game only with `--measures`) |
//! | `taxonomy` | mechanic part across games (catalogue only) |
//!
//! Ported from game-printer's serve/dataset_hf.rs. A template here is a
//! game package (games/donkey-kong): its clips are its corpus plan's (or an
//! older clips.toml's), and `--run` scores them into a run of this home as
//! `gp dataset export` does (crate::dataset_export). The ROM check finds
//! the ROM the package accepts in $GP_ROM_DIR, an NES image or an arcade
//! romset, and checks the files against its bytes (a romset's program,
//! sound and graphics chips).
//!
//! Nothing here reads a ROM except to check the written files against it
//! (`rom_runs`). Evidence quotes are never written: a ref's quote can be a
//! line of the game's code, so a fact carries the file, the line and the
//! label the line sits under instead.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde_json::{Value, json};

use crate::dataset_export::{self as ds, Table, b, i, n, oi, s};
use crate::dataset_tables::{WINDOW, rom_runs};
use gp_interp::mapping::{Field as MapField, Mapping};
use gp_interp::mechanics::Catalog;
use gp_model::Model;

/// The HF layout's version: the configs, their columns and the files
/// beside them. A column added is minor; one removed or changed in
/// meaning is major.
pub(crate) const HF_FORMAT: &str = "0.1.0";

/// The license page a commercial game's card links to: not decided, so
/// not for publishing.
const LICENSE_LOCAL: &str = "# License\n\nNot decided. These tables are Game Printer's text and numbers about a commercial game: model facts, mechanic options, RAM addresses with their names, clip recipes and scores. They hold no part of the game. Whether and under what license they are published is the user's decision, not made yet (docs/research/hf-datasets.md, open questions). Until then this data set stays on the machine that made it.\n\nNames and marks belong to their owners.\n";

fn facts_table() -> Table {
    Table {
        name: "facts",
        about: "One row a model fact: its value, unit, class and mechanic, and the evidence for it, cited by file, line and routine, never by the code's text",
        key: &["game", "path"],
        fields: vec![
            s("game", "", "The template's ID"),
            s(
                "path",
                "",
                "The fact's model path (entities.player.jump.gravity)",
            ),
            s(
                "value",
                "",
                "The value as the model writes it: a number as exact decimal text, a list as JSON",
            ),
            n("value_f64", "", "value as a float when it is a number"),
            s(
                "value_type",
                "",
                "integer, decimal, string, boolean, list or table",
            ),
            s(
                "unit",
                "",
                "The unit the mechanic's record gives the parameter, if any",
            ),
            s(
                "class",
                "",
                "mechanic, parameter, content or presentation (docs/mechanics.md)",
            ),
            s(
                "mechanic",
                "",
                "The mechanic part the fact belongs to (platformer.jump, core.camera, ext:<hook>)",
            ),
            s(
                "kind",
                "",
                "stated, observed or inferred (docs/model.md, Evidence)",
            ),
            i("confidence", "percent", "The evidence's confidence"),
            s(
                "evidence_entry",
                "",
                "The evidence entry that covers it: the most specific model path with an entry",
            ),
            i("refs", "count", "The entry's refs"),
            s(
                "ref_at",
                "",
                "The first ref: path:line in the repo or the pinned download, or another reference",
            ),
            s(
                "ref_in",
                "",
                "The pinned download the first ref is in (smbdis@7e8e9777), null for the repo",
            ),
            s(
                "label",
                "",
                "The nearest label at or above the first ref's line in the source (a table or a branch), when the file is there to read",
            ),
            s(
                "routine",
                "",
                "The nearest label at or above it that code calls or jumps to (jsr, jmp, .dw, .addr): the routine it is in",
            ),
            s(
                "symbol",
                "",
                "The constant or label the first ref's line defines, if any",
            ),
            s(
                "refs_json",
                "",
                "Every ref as JSON: at, in, label, routine and symbol. Never the quote",
            ),
            s("core", "", "The model's core schema version"),
            s(
                "extensions",
                "",
                "The model's genre extensions and versions, JSON",
            ),
            s("model_hash", "", "BLAKE3 of the model's canonical text"),
        ],
    }
}

fn mechanics_table() -> Table {
    Table {
        name: "mechanics",
        about: "One row a mechanic part the model uses at one model path, with the options set there",
        key: &["game", "mechanic", "used_by"],
        fields: vec![
            s("game", "", "The template's ID"),
            s(
                "mechanic",
                "",
                "The mechanic's record name (platformer.jump)",
            ),
            s("version", "", "The record's version"),
            s("implementation", "", "component, core or hook: who runs it"),
            s("carrier", "", "What carries it: entity, level, model"),
            s("system", "", "The system it runs in"),
            s("class", "", "The record's class"),
            s(
                "like",
                "",
                "For a hook: the general mechanic it instances, if it names one",
            ),
            s("hook", "", "The ext hook's name, for a hook"),
            s(
                "used_by",
                "",
                "The model path that carries it (entities.player.jump)",
            ),
            s(
                "options",
                "",
                "The keys set there and their values, JSON; a table or a long list is summarized",
            ),
            s("option_names", "", "The keys set there, JSON list"),
            i("params_declared", "count", "Parameters the record declares"),
            i("params_set", "count", "Of those, the ones set at this path"),
            s("about", "", "The record's description"),
        ],
    }
}

fn mapping_table() -> Table {
    Table {
        name: "mapping",
        about: "One row a state mapping field: where the original keeps a model value and how to read it, like a Data Crystal RAM map or a Gym Retro data.json",
        key: &["game", "scope", "path"],
        fields: vec![
            s("game", "", "The template's ID"),
            i("schema", "", "The mapping's schema"),
            s(
                "scope",
                "",
                "field, or slots[N], slots[N].kind.<entity>, slots[N].rest for a reused object slot's values",
            ),
            s("path", "", "The model path (entities.mario[0].x)"),
            s(
                "role",
                "",
                "position, speed, state, flag, counter, timer, camera, input, presentation or value",
            ),
            s("owner", "", "The mechanic that writes the value"),
            oi(
                "address",
                "",
                "The first RAM address, as Gym Retro's data.json gives it",
            ),
            s(
                "ram",
                "",
                "Every RAM address, low byte first, JSON list of hex",
            ),
            i("bytes", "count", "Bytes in the number"),
            s(
                "retro_type",
                "",
                "The bytes as a Gym Retro type (|u1, <u2, >n6) when they are contiguous; null otherwise",
            ),
            b("signed", "Two's complement"),
            s(
                "bits",
                "",
                "[from, to) of the number, JSON, if only some bits",
            ),
            s("frac", "", "The address of the fraction byte, in 256ths"),
            s("scale", "", "The model's value is the number times this"),
            s("offset", "", "Plus this"),
            b("flag", "The number is a flag"),
            b("invert", "A flag true only when zero"),
            b("digits", "Decimal digits, most significant first"),
            b("whole", "Compared in whole parts only"),
            s("values", "", "Named values by position, JSON list"),
            i("cases", "count", "Conditions that say more than the bytes"),
            b("presentation", "Loaded but never compared"),
            b("shared", "One address for every slot"),
            s("note", "", "The original's names for the bytes"),
        ],
    }
}

fn clips_table() -> Table {
    Table {
        name: "clips",
        about: "One row a clip of the scoring corpus: a start, presses and a frame count anyone remakes from their own ROM, and the hash the original must give",
        key: &["game", "clip_id"],
        fields: vec![
            s("game", "", "The template's ID"),
            s("clip_id", "", "The clip's ID"),
            s("name", "", "In game words"),
            s("source", "", "generated, played or aimed"),
            i("frames", "frames", "Frames the clip runs"),
            s("presses", "", "Button spans from the start, JSON list"),
            s(
                "start_recording",
                "",
                "The recording the start is a frame of, by store ID",
            ),
            s("recording_name", "", "That recording's name"),
            s(
                "recording_notes",
                "",
                "Where it came from: an imported movie's file, hash and authors",
            ),
            s(
                "start_presses",
                "",
                "The .presses file of input from power-on, for a save point's start (in clips/)",
            ),
            i(
                "start_frame",
                "frames",
                "The original's frame the clip starts at, from power-on",
            ),
            oi(
                "model_frame",
                "frames",
                "The model's frame there, as the lockstep counts it",
            ),
            s("context", "", "The start's kind: level_start, save_point"),
            s("level", "", "The start's level"),
            s(
                "curve",
                "",
                "BLAKE3 of the original's mapped measures on every frame",
            ),
            s(
                "recipe",
                "",
                "The clip's recipe hash (docs/formats/scoring-corpus.md)",
            ),
            s("holds", "", "The parts it exercises, JSON list"),
            s("guards", "", "The mutants it catches, JSON list"),
            s("why", "", "Why it is kept"),
        ],
    }
}

fn scores_table() -> Table {
    Table {
        name: "scores",
        about: "One row a clip in a score record: one model scored on one corpus version",
        key: &["game", "model_hash", "corpus_hash", "clip_id"],
        fields: vec![
            s("game", "", "The template's ID"),
            s("model_hash", "", "The model scored, by canonical hash"),
            s("corpus", "", "The corpus's name"),
            s("corpus_version", "", "Its version"),
            s("corpus_hash", "", "Its content hash"),
            s("clip_id", "", "The clip, as in clips"),
            s("recipe", "", "Its recipe hash"),
            s("curve", "", "The original's curve hash"),
            s("model_curve", "", "The model's curve hash"),
            b("exact", "Every measure within tolerance on every frame"),
            oi("permille", "1/1000", "Frames together"),
            oi("score", "splits", "How far from exact"),
            s("error", "", "Why it couldn't be scored, if it couldn't"),
            i("mapping_schema", "", "The state mapping's schema"),
            s("contract", "", "The contract's hash"),
            s("runtime", "", "The runtime and its scenario version"),
            s("gp_version", "", "gp's version"),
            s("gp_build", "", "gp's build"),
            s("record", "", "The record's file in the template"),
        ],
    }
}

fn taxonomy_table() -> Table {
    Table {
        name: "taxonomy",
        about: "One row a mechanic part across the games: who uses it and with which options",
        key: &["mechanic"],
        fields: vec![
            s("mechanic", "", "The mechanic's record name"),
            s("implementation", "", "component, core or hook"),
            s("like", "", "The general mechanic a hook instances"),
            s("games", "", "The games that use it, JSON list"),
            i("game_count", "count", "How many"),
            i(
                "uses",
                "count",
                "Model paths across the games that carry it",
            ),
            s("options_seen", "", "Every option set at any use, JSON list"),
            s(
                "options_declared",
                "",
                "Every parameter the record declares, JSON list",
            ),
            i("facts", "count", "Facts across the games that belong to it"),
            i("facts_stated", "count", "Of those, the stated ones"),
            s("about", "", "The record's description"),
        ],
    }
}

/// A template read for export.
pub(crate) struct Game {
    pub id: String,
    pub dir: PathBuf,
    pub title: String,
    /// `open` or `commercial`; templates without a manifest count as
    /// commercial.
    pub tier: String,
    /// The ROM it prints from, by headerless SHA-256.
    pub rom: Option<String>,
    pub model: Model,
    pub model_hash: String,
    pub catalog: Catalog,
    pub mapping: Option<Mapping>,
}

impl Game {
    pub(crate) fn open(dir: &Path) -> anyhow::Result<Game> {
        let text = fs::read_to_string(crate::mechanics::model_file(dir))
            .with_context(|| format!("reading {}'s model", dir.display()))?;
        let model = Model::parse(&text).with_context(|| dir.display().to_string())?;
        let manifest: Option<toml::Value> = fs::read_to_string(dir.join("template.toml"))
            .ok()
            .and_then(|t| toml::from_str(&t).ok());
        let get = |a: &str, k: &str| {
            manifest
                .as_ref()
                .and_then(|m| m.get(a))
                .and_then(|t| t.get(k))
                .cloned()
        };
        let id = get("template", "id")
            .and_then(|v| v.as_str().map(str::to_owned))
            .or_else(|| dir.file_name().map(|f| f.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let title = get("template", "title")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| model.model.title.clone());
        let tier = get("template", "tier")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "commercial".into());
        let rom = get("source", "accepts")
            .and_then(|v| v.as_array().and_then(|a| a.first().cloned()))
            .and_then(|v| v.as_str().map(str::to_owned));
        let (exts, _) = gp_exts::registry(&model.model.name);
        let catalog = gp_interp::mechanics::catalog(&model, &exts);
        let mapping = fs::read_to_string(dir.join("mapping.toml"))
            .ok()
            .map(|t| Mapping::parse(&t).map_err(|e| anyhow::anyhow!("{}: {e}", dir.display())))
            .transpose()?;
        Ok(Game {
            id,
            dir: dir.to_path_buf(),
            title,
            tier,
            rom,
            model_hash: crate::corpus_score::model_key(&text),
            model,
            catalog,
            mapping,
        })
    }
}

fn value_text(v: &toml::Value) -> (String, Option<f64>, &'static str) {
    match v {
        toml::Value::String(t) => {
            let f = t.parse::<f64>().ok();
            (t.clone(), f, if f.is_some() { "decimal" } else { "string" })
        }
        toml::Value::Integer(x) => (x.to_string(), Some(*x as f64), "integer"),
        toml::Value::Float(x) => (x.to_string(), Some(*x), "decimal"),
        toml::Value::Boolean(x) => (x.to_string(), None, "boolean"),
        toml::Value::Array(_) => (serde_json::to_string(v).unwrap_or_default(), None, "list"),
        _ => (serde_json::to_string(v).unwrap_or_default(), None, "table"),
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '@' || c == '.'
}

/// The label a line defines at its start (`Name:`, ca65's `.proc Name`),
/// or the constant (`NAME = ...`).
fn defines(line: &str) -> Option<(String, bool)> {
    let t = line.trim_end();
    if let Some(rest) = t.trim_start().strip_prefix(".proc") {
        let name: String = rest
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        return (!name.is_empty()).then_some((name, true));
    }
    let first = t.chars().next()?;
    let lead = t.trim_start();
    if is_ident_start(first) && first != '.' {
        let name: String = t
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let after = t[name.len()..].trim_start();
        if !name.is_empty() && after.starts_with(':') && !after.starts_with("::") {
            return Some((name, true));
        }
    }
    // A constant, at the line's start or indented.
    let name: String = lead
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let after = lead[name.len()..].trim_start();
    if !name.is_empty()
        && name.chars().next().is_some_and(|c| !c.is_ascii_digit())
        && after.starts_with('=')
        && !after.starts_with("==")
    {
        return Some((name, false));
    }
    None
}

fn is_source(file: &str) -> bool {
    let file = file.to_ascii_lowercase();
    [".s", ".asm", ".inc", ".a65"]
        .iter()
        .any(|e| file.ends_with(e))
}

/// What a ref's line says about where it is, read from the file: the
/// routine (the nearest label at or above it) and the symbol the line
/// itself defines. Only names, never the line.
struct Lines {
    resolver: crate::pinned::Resolver<'static>,
    repo: PathBuf,
    files: BTreeMap<PathBuf, Option<Source>>,
}

/// A source file's lines and the labels its code calls or jumps to.
struct Source {
    lines: Vec<String>,
    targets: HashSet<String>,
}

impl Source {
    fn new(text: &str) -> Source {
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let mut targets = HashSet::new();
        for l in &lines {
            let code = l.split(';').next().unwrap_or("");
            let mut words = code.split_whitespace().peekable();
            while let Some(w) = words.next() {
                let lw = w.to_ascii_lowercase();
                if matches!(lw.as_str(), "jsr" | "jmp" | ".dw" | ".word" | ".addr") {
                    let rest: String = words.by_ref().collect::<Vec<_>>().join(" ");
                    for t in rest.split(',') {
                        let name: String = t
                            .trim()
                            .trim_start_matches('(')
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        if !name.is_empty() {
                            targets.insert(name);
                        }
                    }
                    break;
                }
            }
        }
        Source { lines, targets }
    }
}

/// Where a ref's line is: the nearest label, the routine and the symbol
/// the line defines.
type Place = (Option<String>, Option<String>, Option<String>);

impl Lines {
    fn place(&mut self, within: Option<&str>, at: &str) -> Place {
        let none = (None, None, None);
        let Some((file, line)) = gp_model::validate::file_ref(at) else {
            return none;
        };
        if !is_source(file) || line == 0 {
            return none;
        }
        let gp_model::validate::RefRoot::Dir(root) = self.resolver.root(within, file, &self.repo)
        else {
            return none;
        };
        let path = root.join(file);
        let src = self.files.entry(path.clone()).or_insert_with(|| {
            fs::read(&path)
                .ok()
                .map(|b| Source::new(&String::from_utf8_lossy(&b)))
        });
        let Some(src) = src else {
            return none;
        };
        let Some(here) = src.lines.get(line - 1) else {
            return none;
        };
        let symbol = defines(here).map(|(n, _)| n);
        let labels = (0..line).rev().take(6000).filter_map(|k| {
            defines(&src.lines[k])
                .filter(|(_, label)| *label)
                .map(|(n, _)| n)
        });
        let mut label = None;
        let mut routine = None;
        for l in labels {
            if label.is_none() {
                label = Some(l.clone());
            }
            if src.targets.contains(&l) {
                routine = Some(l);
                break;
            }
        }
        (label, routine, symbol)
    }
}

fn fill_facts(
    t: &mut Table,
    g: &Game,
    lines: &mut Lines,
    counts: &mut BTreeMap<String, (usize, usize)>,
) {
    let tree = g.model.to_tree();
    let evidence: BTreeMap<String, Option<String>> = gp_model::validate::evidence_for(&g.model)
        .into_iter()
        .map(|(p, k)| (gp_model::path::display(&p), k))
        .collect();
    let classes: BTreeMap<&str, &gp_model::classify::Classified> = g
        .catalog
        .paths
        .iter()
        .map(|c| (c.path.as_str(), c))
        .collect();
    let exts = serde_json::to_string(&g.model.model.extensions).unwrap_or_default();
    for (fact, value) in gp_model::facts(&tree) {
        let shown = gp_model::path::display(&fact);
        let (text, f, ty) = value_text(value);
        let class = classes.get(shown.as_str());
        let mechanic = class.and_then(|c| c.mechanic.clone());
        let unit = mechanic
            .as_deref()
            .and_then(|m| unit_of(&g.catalog, m, &shown));
        let entry = evidence.get(&shown).cloned().flatten();
        let ev = entry.as_ref().and_then(|k| g.model.evidence.get(k));
        let mut refs = Vec::new();
        for r in ev.map(|e| e.refs.as_slice()).unwrap_or_default() {
            let (label, routine, symbol) = lines.place(r.within.as_deref(), &r.at);
            refs.push((r.at.clone(), r.within.clone(), label, routine, symbol));
        }
        let kind = ev.map(|e| e.source.name().to_owned());
        if let Some(m) = &mechanic {
            let c = counts.entry(m.clone()).or_default();
            c.0 += 1;
            c.1 += usize::from(kind.as_deref() == Some("stated"));
        }
        t.push_str("game", g.id.clone());
        t.push_str("path", shown.clone());
        t.push_str("value", text);
        t.push_num("value_f64", f);
        t.push_str("value_type", ty.to_owned());
        t.push_str("unit", unit);
        t.push_str("class", class.map(|c| c.class.name().to_owned()));
        t.push_str("mechanic", mechanic);
        t.push_str("kind", kind);
        t.push_int("confidence", ev.map_or(0, |e| i64::from(e.confidence)));
        t.push_str("evidence_entry", entry);
        t.push_int("refs", refs.len() as i64);
        let first = refs.first();
        t.push_str("ref_at", first.map(|r| r.0.clone()));
        t.push_str("ref_in", first.and_then(|r| r.1.clone()));
        t.push_str("label", first.and_then(|r| r.2.clone()));
        t.push_str("routine", first.and_then(|r| r.3.clone()));
        t.push_str("symbol", first.and_then(|r| r.4.clone()));
        let all: Vec<Value> = refs
            .iter()
            .map(|(at, w, l, r, sy)| json!({"at": at, "in": w, "label": l, "routine": r, "symbol": sy}))
            .collect();
        t.push_str("refs_json", serde_json::to_string(&all).unwrap_or_default());
        t.push_str("core", g.model.model.schema.clone());
        t.push_str("extensions", exts.clone());
        t.push_str("model_hash", g.model_hash.clone());
    }
}

/// The unit the mechanic's record gives the fact's parameter.
fn unit_of(c: &Catalog, mechanic: &str, fact: &str) -> Option<String> {
    let m = c.get(mechanic)?;
    let rest = m
        .used_by
        .iter()
        .filter_map(|u| {
            fact.strip_prefix(u.as_str())
                .and_then(|r| r.strip_prefix('.'))
        })
        .min_by_key(|r| r.len())?;
    let first = rest.split(['.', '[']).next().unwrap_or(rest);
    m.params
        .iter()
        .find(|p| p.name == rest)
        .or_else(|| m.params.iter().find(|p| p.name == first))
        .map(|p| p.unit)
        .filter(|u| !u.is_empty())
        .map(str::to_owned)
}

/// The tree under a model path written with dots (`entities.player.jump`).
fn subtree<'a>(tree: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let mut at = tree;
    for k in path.split('.') {
        let k = k.split('[').next().unwrap_or(k);
        at = at.get(k)?;
    }
    Some(at)
}

/// The options set at a use: scalars and short lists of scalars as they
/// are, tables and long lists by their size, so no level or table data
/// rides along.
fn options_at(tree: &toml::Value, path: &str) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Some(t) = subtree(tree, path).and_then(toml::Value::as_table) else {
        return out;
    };
    for (k, v) in t {
        if k == "description" {
            continue;
        }
        let shown = match v {
            toml::Value::Table(t) => json!(format!("{{table of {}}}", t.len())),
            toml::Value::Array(a)
                if a.len() > 8 || a.iter().any(|x| x.is_table() || x.is_array()) =>
            {
                json!(format!("[list of {}]", a.len()))
            }
            other => serde_json::to_value(other).unwrap_or(Value::Null),
        };
        out.insert(k.clone(), shown);
    }
    out
}

fn fill_mechanics(t: &mut Table, g: &Game) {
    let tree = g.model.to_tree();
    for m in &g.catalog.mechanics {
        for u in &m.used_by {
            let opts = options_at(&tree, u);
            let declared: BTreeSet<&str> = m.params.iter().map(|p| p.name).collect();
            let set = opts
                .keys()
                .filter(|k| declared.contains(k.as_str()))
                .count();
            t.push_str("game", g.id.clone());
            t.push_str("mechanic", m.name.to_owned());
            t.push_str("version", m.version.to_owned());
            t.push_str("implementation", m.implementation.to_owned());
            t.push_str("carrier", m.carrier.to_owned());
            t.push_str("system", m.system.to_owned());
            t.push_str("class", m.class.to_owned());
            t.push_str("like", (!m.like.is_empty()).then(|| m.like.to_owned()));
            t.push_str("hook", m.hook.clone());
            t.push_str("used_by", u.clone());
            t.push_str("options", serde_json::to_string(&opts).unwrap_or_default());
            t.push_str(
                "option_names",
                serde_json::to_string(&opts.keys().collect::<Vec<_>>()).unwrap_or_default(),
            );
            t.push_int("params_declared", m.params.len() as i64);
            t.push_int("params_set", set as i64);
            t.push_str("about", m.about.to_owned());
        }
    }
}

/// A field's role, from its path: what kind of value it holds.
fn role(f: &MapField) -> &'static str {
    let p = f.path.as_str();
    let head = p.split(['.', '[']).next().unwrap_or("");
    let last = p.rsplit('.').next().unwrap_or(p);
    let last = last.split('[').next().unwrap_or(last);
    if f.presentation || head == "present" {
        return "presentation";
    }
    match head {
        "input" => return "input",
        "ticks" => return "timer",
        "counters" => return "counter",
        "cameras" => return "camera",
        "rules" => return "state",
        _ => {}
    }
    if f.flag {
        return "flag";
    }
    if !f.values.is_empty() {
        return "state";
    }
    match last {
        "x" | "y" | "home_x" | "home_y" | "x_frac" | "y_frac" => "position",
        l if l.starts_with('v') || l.contains("speed") => "speed",
        "state" | "facing" | "moved" | "moving" => "state",
        _ => "value",
    }
}

/// The bytes as a Gym Retro type string, when they are contiguous.
fn retro_type(f: &MapField) -> Option<String> {
    let n = f.ram.len();
    if n == 0 {
        return None;
    }
    if f.digits {
        let up = f.ram.windows(2).all(|w| w[1] == w[0] + 1);
        return up.then(|| format!(">n{n}"));
    }
    if n == 1 {
        return Some(if f.signed { "|i1" } else { "|u1" }.into());
    }
    let up = f.ram.windows(2).all(|w| w[1] == w[0] + 1);
    let down = f.ram.windows(2).all(|w| w[0] == w[1] + 1);
    let sign = if f.signed { 'i' } else { 'u' };
    // Low byte first at rising addresses is little endian.
    if up {
        Some(format!("<{sign}{n}"))
    } else if down {
        Some(format!(">{sign}{n}"))
    } else {
        None
    }
}

fn fill_mapping(t: &mut Table, g: &Game) {
    let Some(m) = &g.mapping else { return };
    let mut rows: Vec<(String, &MapField)> =
        m.fields.iter().map(|f| ("field".to_owned(), f)).collect();
    for (k, sl) in m.slot_groups().into_iter().enumerate() {
        rows.extend(sl.fields.iter().map(|f| (format!("slots[{k}]"), f)));
        for kind in &sl.kinds {
            rows.extend(
                kind.fields
                    .iter()
                    .map(|f| (format!("slots[{k}].kind.{}", kind.entity), f)),
            );
        }
        rows.extend(sl.rest.iter().map(|f| (format!("slots[{k}].rest"), f)));
    }
    let hex = |a: u16| format!("0x{a:04X}");
    for (scope, f) in rows {
        let owner = gp_interp::lockstep::owners(&g.catalog, &f.path)
            .first()
            .map(|o| (*o).to_owned());
        t.push_str("game", g.id.clone());
        t.push_int("schema", i64::from(m.schema));
        t.push_str("scope", scope);
        t.push_str("path", f.path.clone());
        t.push_str("role", role(f).to_owned());
        t.push_str("owner", owner);
        t.push_opt_int("address", f.ram.first().map(|a| i64::from(*a)));
        t.push_str(
            "ram",
            serde_json::to_string(&f.ram.iter().map(|a| hex(*a)).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        t.push_int("bytes", f.ram.len() as i64);
        t.push_str("retro_type", retro_type(f));
        t.push_bool("signed", f.signed);
        t.push_str("bits", f.bits.map(|b| format!("[{}, {}]", b[0], b[1])));
        t.push_str("frac", f.frac.map(hex));
        t.push_str("scale", f.scale.map(|x| x.to_string()));
        t.push_str("offset", f.offset.map(|x| x.to_string()));
        t.push_bool("flag", f.flag);
        t.push_bool("invert", f.invert);
        t.push_bool("digits", f.digits);
        t.push_bool("whole", f.whole);
        t.push_str(
            "values",
            (!f.values.is_empty()).then(|| serde_json::to_string(&f.values).unwrap_or_default()),
        );
        t.push_int("cases", f.cases.len() as i64);
        t.push_bool("presentation", f.presentation);
        t.push_bool("shared", f.shared);
        t.push_str("note", f.note.clone());
    }
}

/// The clip set's clips, and the `.presses` files they name (to copy
/// beside the tables).
fn fill_clips(t: &mut Table, g: &Game) -> anyhow::Result<Vec<String>> {
    let Some(set) = clip_set(g)? else {
        return Ok(Vec::new());
    };
    let read = |p: &str| -> anyhow::Result<String> {
        fs::read_to_string(g.dir.join(p)).with_context(|| format!("reading {p}"))
    };
    let mut files = BTreeSet::new();
    for c in &set.clips {
        let recipe = crate::clipset::recipe_hash(c, &read)?;
        let presses = match &c.presses_file {
            Some(p) => {
                files.insert(p.clone());
                crate::clipset::parse_presses(&read(p)?)?.spans
            }
            None => c.presses.clone(),
        };
        let rec = c
            .start
            .recording
            .as_ref()
            .and_then(|r| set.recordings.get(r));
        if let Some(p) = &c.start.presses_from_power_on {
            files.insert(p.clone());
        }
        let js =
            |v: &[String]| (!v.is_empty()).then(|| serde_json::to_string(v).unwrap_or_default());
        t.push_str("game", g.id.clone());
        t.push_str("clip_id", c.id.clone());
        t.push_str("name", c.name.clone());
        t.push_str("source", c.source.clone());
        t.push_int("frames", c.frames as i64);
        t.push_str(
            "presses",
            serde_json::to_string(&presses).unwrap_or_default(),
        );
        t.push_str("start_recording", c.start.recording.clone());
        let field = |k: &str| {
            rec.and_then(|r| r.get(k))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        };
        t.push_str("recording_name", field("name"));
        t.push_str("recording_notes", field("notes").filter(|n| !n.is_empty()));
        t.push_str("start_presses", c.start.presses_from_power_on.clone());
        t.push_int("start_frame", c.start.frame as i64);
        t.push_opt_int("model_frame", c.start.model_frame.map(|f| f as i64));
        t.push_str(
            "context",
            (!c.start.context.is_empty()).then(|| c.start.context.clone()),
        );
        t.push_str(
            "level",
            (!c.start.level.is_empty()).then(|| c.start.level.clone()),
        );
        t.push_str("curve", (!c.curve.is_empty()).then(|| c.curve.clone()));
        t.push_str("recipe", recipe);
        t.push_str("holds", js(&c.holds));
        t.push_str("guards", js(&c.guards));
        t.push_str("why", (!c.why.is_empty()).then(|| c.why.clone()));
    }
    Ok(files.into_iter().collect())
}

/// Whether a clip set's recordings were imported from movies, whose
/// authors a published copy credits.
fn clips_name_movies(g: &Game) -> bool {
    clip_set(g).ok().flatten().is_some_and(|set| {
        set.recordings.values().any(|r| {
            r.get("notes")
                .and_then(|n| n.as_str())
                .is_some_and(|n| n.contains("movie"))
        })
    })
}

/// The package's clips: its corpus plan, else its clips.toml; none with
/// neither.
fn clip_set(g: &Game) -> anyhow::Result<Option<crate::clipset::ClipSet>> {
    if crate::plan::load(&g.dir)?.is_none() && !g.dir.join(crate::clipset::FILE).is_file() {
        return Ok(None);
    }
    crate::clipset::read_game(&g.dir).map(Some)
}

fn fill_scores(t: &mut Table, g: &Game) {
    for r in crate::corpus_w9::records(&g.dir) {
        let Some(f) = r["file"].as_str().map(PathBuf::from) else {
            continue;
        };
        let Ok(v) = fs::read(&f)
            .map_err(anyhow::Error::from)
            .and_then(|b| Ok(serde_json::from_slice::<Value>(&b)?))
        else {
            continue;
        };
        let p = &v["predicate"];
        let st = |v: &Value| v.as_str().map(str::to_owned);
        let rel = f
            .strip_prefix(&g.dir)
            .map_or_else(|_| f.display().to_string(), |x| x.display().to_string());
        for c in p["clips"].as_array().into_iter().flatten() {
            t.push_str("game", g.id.clone());
            t.push_str("model_hash", st(&p["model"]["hash"]));
            t.push_str("corpus", st(&p["corpus"]["name"]));
            t.push_str("corpus_version", st(&p["corpus"]["version"]));
            t.push_str("corpus_hash", st(&p["corpus"]["hash"]));
            t.push_str("clip_id", st(&c["id"]));
            t.push_str("recipe", st(&c["recipe"]));
            t.push_str("curve", st(&c["curve"]));
            t.push_str("model_curve", st(&c["model_curve"]));
            t.push_bool("exact", c["exact"].as_bool().unwrap_or(false));
            t.push_opt_int("permille", c["permille"].as_i64());
            t.push_opt_int("score", c["score"].as_i64());
            t.push_str("error", st(&c["error"]));
            t.push_int(
                "mapping_schema",
                p["mapping"]["schema"]
                    .as_i64()
                    .or_else(|| p["mapping"]["schema"].as_str().and_then(|x| x.parse().ok()))
                    .unwrap_or(0),
            );
            t.push_str("contract", st(&p["contract"]));
            t.push_str(
                "runtime",
                Some(format!(
                    "{} {}",
                    p["runtime"]["name"].as_str().unwrap_or(""),
                    p["runtime"]["scenario"].as_str().unwrap_or("")
                )),
            );
            t.push_str("gp_version", st(&p["gp"]["version"]));
            t.push_str("gp_build", st(&p["gp"]["build"]));
            t.push_str("record", rel.clone());
        }
    }
}

/// The ROM's bytes as the check reads them, when a ROM the package
/// accepts is in $GP_ROM_DIR: an NES image's headerless PRG and CHR, a
/// romset's chips (crate::arcade_stage0::rom_bytes).
fn rom_bytes(paths: &gp_evidence::Paths, dir: &Path) -> Option<Vec<u8>> {
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(dir.join("template.toml")).ok()?).ok()?;
    let path = crate::model_static::template_rom(dir, &manifest, paths).ok()?;
    let bytes = fs::read(path).ok()?;
    if crate::original::Platform::of_file(&bytes) == crate::original::Platform::Arcade {
        let set = gp_arcade::Romset::from_zip(&bytes).ok()?;
        return Some(crate::arcade_stage0::rom_bytes(&set));
    }
    let (_, prg, chr) = gp_evidence::source::nes_banks(&bytes).ok()?;
    let mut out = prg.to_vec();
    out.extend_from_slice(chr);
    Some(out)
}

type Files = Vec<(String, String, usize)>;

fn write_tables(out: &Path, tables: &[Table], meta: &[(String, String)]) -> anyhow::Result<Files> {
    let data = out.join("data");
    fs::create_dir_all(&data).with_context(|| format!("making {}", data.display()))?;
    let mut files = Vec::new();
    for t in tables {
        let rel = format!("data/{}.parquet", t.name);
        let p = out.join(&rel);
        ds::write_parquet(t, &p, meta)?;
        files.push((rel, ds::sha256(&fs::read(&p)?), t.rows()));
    }
    Ok(files)
}

fn size_category(rows: usize) -> &'static str {
    match rows {
        0..1_000 => "n<1K",
        1_000..10_000 => "1K<n<10K",
        10_000..100_000 => "10K<n<100K",
        100_000..1_000_000 => "100K<n<1M",
        _ => "1M<n<10M",
    }
}

/// The card's front matter: the Hub's keys only.
fn front_matter(
    pretty: &str,
    license: &str,
    tables: &[Table],
    files: &Files,
    measures: bool,
) -> String {
    let mut c = String::from("---\n");
    let _ = writeln!(c, "pretty_name: \"{pretty}\"");
    if license == "other" {
        c.push_str(
            "license: other\nlicense_name: game-printer-recipes\nlicense_link: LICENSE.md\n",
        );
    } else {
        let _ = writeln!(c, "license: {license}");
    }
    c.push_str("language:\n- en\n");
    c.push_str("task_categories:\n- other\n");
    if measures {
        c.push_str("- time-series-forecasting\n");
    }
    c.push_str("- reinforcement-learning\n");
    c.push_str(
        "tags:\n- game-printer\n- game-mechanics\n- nes\n- reverse-engineering\n- tabular\n",
    );
    let most = files.iter().map(|f| f.2).max().unwrap_or(0);
    let _ = writeln!(c, "size_categories:\n- {}", size_category(most));
    c.push_str("configs:\n");
    for (k, t) in tables.iter().enumerate() {
        let _ = writeln!(c, "- config_name: {}", t.name);
        let _ = writeln!(
            c,
            "  data_files:\n  - split: train\n    path: \"data/{}.parquet\"",
            t.name
        );
        if k == 0 {
            c.push_str("  default: true\n");
        }
    }
    c.push_str("---\n\n");
    c
}

fn table_docs(c: &mut String, tables: &[Table], files: &Files) {
    c.push_str("## Configs\n\n");
    for t in tables {
        let rows = files
            .iter()
            .find(|f| f.0 == format!("data/{}.parquet", t.name))
            .map_or(0, |f| f.2);
        let _ = writeln!(
            c,
            "### {}\n\n{}. Key: {}. Rows: {rows}.\n",
            t.name,
            t.about,
            t.key.join(", ")
        );
        c.push_str("| Column | Type | Unit | Meaning |\n|---|---|---|---|\n");
        for f in &t.fields {
            let ty = f.col.croissant_type().trim_start_matches("sc:");
            let _ = writeln!(c, "| `{}` | {ty} | {} | {} |", f.name, f.unit, f.about);
        }
        c.push('\n');
    }
}

fn files_doc(c: &mut String, files: &Files) {
    c.push_str("## Files\n\n| File | SHA-256 | Rows |\n|---|---|---|\n");
    for (f, h, rows) in files {
        let _ = writeln!(c, "| `{f}` | `{h}` | {rows} |");
    }
    c.push('\n');
}

fn scan_line(scan: &Value) -> String {
    match scan["rom"].as_str() {
        Some(rom) => format!(
            "Every file was checked for runs of {WINDOW} bytes of the ROM `{rom}`: {} found.",
            scan["matches"].as_array().map_or(0, Vec::len)
        ),
        None => format!(
            "The ROM wasn't in $GP_ROM_DIR, so the files weren't checked against it ({}).",
            scan["why"].as_str().unwrap_or("")
        ),
    }
}

/// Writes the scan's verdict and refuses an export that holds the ROM.
fn scan(
    paths: &gp_evidence::Paths,
    out: &Path,
    dir: &Path,
    rom: Option<&str>,
) -> anyhow::Result<Value> {
    let Some(sha) = rom else {
        return Ok(json!({"rom": null, "why": "the template names no ROM"}));
    };
    let Some(bytes) = rom_bytes(paths, dir) else {
        return Ok(json!({"rom": null, "why": "not found"}));
    };
    let found = rom_runs(out, &bytes)?;
    if !found.is_empty() {
        anyhow::bail!(
            "the export at {} holds runs of {WINDOW} bytes of the ROM {sha}: {}; nothing of the game may be in a data set (AGENTS.md, Never commit)",
            out.display(),
            found
                .iter()
                .map(|(f, at)| format!("{f} at byte {at}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(json!({"rom": sha, "window": WINDOW, "matches": []}))
}

fn lines_for(paths: &gp_evidence::Paths, dir: &Path) -> Lines {
    Lines {
        resolver: crate::pinned::Resolver::new(&paths.home),
        repo: gp_evidence::paths::enclosing_git_work_tree(dir).unwrap_or_else(|| dir.to_path_buf()),
        files: BTreeMap::new(),
    }
}

/// `gp dataset export --hf DIR --template T [--run R [--measures]]`.
pub(crate) fn export(
    paths: &gp_evidence::Paths,
    template: &Path,
    out: &Path,
    run: Option<&str>,
    measures: bool,
    how: &crate::clip_score::Open,
) -> anyhow::Result<crate::Outcome> {
    let g = Game::open(template)?;
    let open = g.tier == "open";
    let mut lines = lines_for(paths, template);
    let mut t = vec![
        facts_table(),
        mechanics_table(),
        mapping_table(),
        clips_table(),
        scores_table(),
    ];
    let mut counts = BTreeMap::new();
    fill_facts(&mut t[0], &g, &mut lines, &mut counts);
    fill_mechanics(&mut t[1], &g);
    fill_mapping(&mut t[2], &g);
    let presses = fill_clips(&mut t[3], &g)?;
    fill_scores(&mut t[4], &g);
    let with_measures = match run {
        Some(r) if open || measures => {
            let (tables, _, _, _, _, _) = ds::scored(paths, template, r, how)?;
            let [_, m, _, _] = tables;
            t.push(m);
            true
        }
        Some(_) => false,
        None => {
            if measures {
                anyhow::bail!(
                    "--measures needs --run: the measures are a run's clips scored on the model"
                );
            }
            false
        }
    };

    let meta = vec![
        ("gp.dataset.hf_format".to_owned(), HF_FORMAT.to_owned()),
        ("gp.game".to_owned(), g.id.clone()),
        ("gp.model".to_owned(), g.model_hash.clone()),
        (
            "gp.version".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
        ),
    ];
    let mut files = write_tables(out, &t, &meta)?;
    for p in &presses {
        let to = out.join(p);
        if let Some(d) = to.parent() {
            fs::create_dir_all(d)?;
        }
        fs::copy(g.dir.join(p), &to).with_context(|| format!("copying {p}"))?;
        files.push((p.clone(), ds::sha256(&fs::read(&to)?), 0));
    }

    let corpus = fs::read_to_string(g.dir.join("corpus.toml"))
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok());
    let cget = |a: &str, k: &str| {
        corpus
            .as_ref()
            .and_then(|c| c.get(a))
            .and_then(|x| x.get(k))
            .and_then(|x| x.as_str())
            .map(str::to_owned)
    };
    let manifest = json!({
        "hf_format": HF_FORMAT,
        "game": g.id,
        "title": g.title,
        "tier": g.tier,
        "rom": {"sha256": g.rom, "how": "your own copy, found by `gp roms add`; never in this data set"},
        "model": {"name": g.model.model.name, "hash": g.model_hash, "core": g.model.model.schema, "extensions": g.model.model.extensions},
        "mapping": {"schema": g.mapping.as_ref().map(|m| m.schema)},
        "corpus": {"name": cget("corpus", "name"), "version": cget("corpus", "version"), "hash": cget("corpus", "hash")},
        "gp": {"version": env!("CARGO_PKG_VERSION"), "build": crate::corpus_score::build()},
        "measures": if with_measures { json!({"run": run}) } else { Value::Null },
        "files": files.iter().map(|(f, h, r)| json!({"path": f, "sha256": h, "rows": r})).collect::<Vec<_>>(),
    });

    // A hack's patch, by hash, with its author and whether they agreed to
    // anything printed from it being shown (sources/<platform>.toml).
    let patch = g.rom.as_deref().and_then(|sha| {
        let reg = crate::open::load_registry().ok()?;
        let p = reg.by_sha256(sha)?.patch.clone()?;
        Some(json!({"base": p.base, "sha256": p.sha256, "url": p.url, "author": p.author, "permission": p.permission}))
    });
    let mut manifest = manifest;
    manifest["rom"]["patch"] = patch.clone().unwrap_or(Value::Null);
    let license = if open { "cc0-1.0" } else { "other" };
    let mut c = front_matter(
        &format!("Game Printer: {}", g.title),
        license,
        &t,
        &files,
        with_measures,
    );
    let _ = writeln!(c, "# Game Printer: {}\n", g.title);
    let _ = writeln!(
        c,
        "The data model Game Printer has of `{}`: every fact of its model with the evidence for it, the mechanic parts it uses, where the original keeps its state, the clips it is scored on and the scores. Made by `gp dataset export --hf` (layout {HF_FORMAT}), locally. It is not published.\n",
        g.id
    );
    c.push_str("Nothing of the game is in it: no ROM, frames, sprites, tiles, level data, sound, snapshots, RAM dumps or code. Evidence cites the disassembly by file, line and routine name, never by its text. Clips are recipes that anyone with their own ROM makes again; `curve` is the hash they must give.\n\n");
    c.push_str("## What it needs\n\n| Input | Value |\n|---|---|\n");
    for (k, v) in [
        (
            "ROM (headerless SHA-256)",
            g.rom.clone().unwrap_or_else(|| "none named".into()),
        ),
        (
            "Model",
            format!("{} ({})", g.model.model.name, g.model_hash),
        ),
        ("Core schema", g.model.model.schema.clone()),
        (
            "Extensions",
            g.model
                .model
                .extensions
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        (
            "Mapping schema",
            g.mapping
                .as_ref()
                .map_or("none".into(), |m| m.schema.to_string()),
        ),
        (
            "Corpus",
            match (cget("corpus", "name"), cget("corpus", "version")) {
                (Some(n), Some(v)) => format!("{n} {v}"),
                _ => "none".into(),
            },
        ),
        ("gp", env!("CARGO_PKG_VERSION").to_owned()),
    ]
    .into_iter()
    .chain(patch.as_ref().map(|p| {
        (
            "Patch (applied to the base by `gp roms add --patch`)",
            format!(
                "{} by {}, SHA-256 {}, from {}",
                p["base"].as_str().unwrap_or(""),
                p["author"].as_str().unwrap_or(""),
                p["sha256"].as_str().unwrap_or(""),
                p["url"].as_str().unwrap_or("")
            ),
        )
    })) {
        let _ = writeln!(c, "| {k} | `{v}` |");
    }
    let _ = writeln!(
        c,
        "\n## Use it\n\n```sh\n# Your own ROM, found by its hash; then the game printed from it\ngp roms add /path/to/your.nes\ngp template extract templates/{id}\ngp gen bevy $GP_HOME/work/templates/{id} --out /tmp/{id}-print\n\n# Every clip made again from your ROM and every curve checked\ngp corpus verify templates/{id} --run RUN\n```\n\n```python\nimport duckdb\nduckdb.sql(\"SELECT mechanic, kind, count(*) FROM 'data/facts.parquet' GROUP BY ALL ORDER BY 3 DESC\").show()\n\nfrom datasets import load_dataset\nfacts = load_dataset(\"parquet\", data_files=\"data/facts.parquet\")[\"train\"]\n```\n",
        id = g.id
    );
    table_docs(&mut c, &t, &files);
    files_doc(&mut c, &files);
    c.push_str("## Use and license\n\n");
    if open {
        c.push_str("The game is Game Printer's own, so every table may be shared, measures included. CC0 1.0.\n\n");
    } else {
        c.push_str("The tables are Game Printer's text and numbers about the game: model facts, mechanic options, RAM addresses with their names (as Data Crystal and Gym Retro publish), clip recipes and scores. Whether to publish them is the user's decision, not made yet; keep this data set local until then. Per-frame `measures` are held back unless `--measures` is given. Names and marks belong to their owners.\n\n");
        if let Some(p) = &patch {
            let _ = writeln!(
                c,
                "This is a hack by {}. Their permission to show anything printed from it: `{}`. Until it is granted, this data set stays on this machine.\n",
                p["author"].as_str().unwrap_or(""),
                p["permission"].as_str().unwrap_or("")
            );
        }
        if t[3].rows() > 0 && clips_name_movies(&g) {
            c.push_str("Clip starts that name a recording credit the movie it was imported from in `recording_notes`; a published copy credits those authors (CC BY, as TASVideos asks).\n\n");
        }
    }
    let verdict = scan(paths, out, &g.dir, g.rom.as_deref())?;
    let _ = writeln!(c, "{}\n", scan_line(&verdict));
    manifest["rom_scan"] = verdict.clone();
    fs::write(out.join("README.md"), &c)?;
    if !open {
        fs::write(out.join("LICENSE.md"), LICENSE_LOCAL)?;
    }
    fs::write(
        out.join("gp-dataset.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    fs::write(
        out.join("croissant.json"),
        serde_json::to_string_pretty(&ds::croissant_for(
            &format!("game-printer-{}", g.id),
            &format!(
                "Game Printer's data model of {}: model facts with evidence, mechanics, the state mapping, clip recipes and scores. No game data.",
                g.title
            ),
            HF_FORMAT,
            &t,
            "parquet",
            &files,
        ))?,
    )?;
    // The scan again, now over the card and metadata too.
    let verdict = scan(paths, out, &g.dir, g.rom.as_deref())?;

    let mut text = format!(
        "{}: wrote {} to {}\n",
        g.id,
        if with_measures {
            "6 configs"
        } else {
            "5 configs"
        },
        out.display()
    );
    for (f, _, r) in files.iter().filter(|f| f.0.starts_with("data/")) {
        let _ = writeln!(text, "  {f}: {r} rows");
    }
    let _ = writeln!(text, "  {}", scan_line(&verdict));
    Ok(crate::Outcome::ok(
        json!({"game": g.id, "out": out, "files": manifest["files"], "rom_scan": verdict}),
        text,
    ))
}

/// `gp dataset catalogue --out DIR TEMPLATE...`: the facts and mechanics
/// of every template, unioned, and the taxonomy of mechanic parts.
pub(crate) fn catalogue(
    paths: &gp_evidence::Paths,
    templates: &[PathBuf],
    out: &Path,
) -> anyhow::Result<crate::Outcome> {
    let mut t = vec![facts_table(), mechanics_table(), taxonomy_table()];
    let mut games = Vec::new();
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    struct Seen {
        implementation: String,
        like: Option<String>,
        about: String,
        games: BTreeSet<String>,
        uses: usize,
        options: BTreeSet<String>,
        declared: BTreeSet<String>,
    }
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    for dir in templates {
        let g = Game::open(dir)?;
        let mut lines = lines_for(paths, dir);
        fill_facts(&mut t[0], &g, &mut lines, &mut counts);
        fill_mechanics(&mut t[1], &g);
        let tree = g.model.to_tree();
        for m in &g.catalog.mechanics {
            let e = seen.entry(m.name.to_owned()).or_insert_with(|| Seen {
                implementation: m.implementation.to_owned(),
                like: (!m.like.is_empty()).then(|| m.like.to_owned()),
                about: m.about.to_owned(),
                games: BTreeSet::new(),
                uses: 0,
                options: BTreeSet::new(),
                declared: BTreeSet::new(),
            });
            e.games.insert(g.id.clone());
            e.uses += m.used_by.len();
            e.declared
                .extend(m.params.iter().map(|p| p.name.to_owned()));
            for u in &m.used_by {
                e.options.extend(options_at(&tree, u).into_keys());
            }
        }
        games.push(json!({"game": g.id, "title": g.title, "model": g.model_hash, "core": g.model.model.schema, "extensions": g.model.model.extensions}));
    }
    for (name, e) in &seen {
        let tx = &mut t[2];
        let (facts, stated) = counts.get(name).copied().unwrap_or_default();
        tx.push_str("mechanic", name.clone());
        tx.push_str("implementation", e.implementation.clone());
        tx.push_str("like", e.like.clone());
        tx.push_str("games", serde_json::to_string(&e.games).unwrap_or_default());
        tx.push_int("game_count", e.games.len() as i64);
        tx.push_int("uses", e.uses as i64);
        tx.push_str(
            "options_seen",
            serde_json::to_string(&e.options).unwrap_or_default(),
        );
        tx.push_str(
            "options_declared",
            serde_json::to_string(&e.declared).unwrap_or_default(),
        );
        tx.push_int("facts", facts as i64);
        tx.push_int("facts_stated", stated as i64);
        tx.push_str("about", e.about.clone());
    }
    let meta = vec![
        ("gp.dataset.hf_format".to_owned(), HF_FORMAT.to_owned()),
        ("gp.games".to_owned(), serde_json::to_string(&games)?),
        (
            "gp.version".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
        ),
    ];
    let files = write_tables(out, &t, &meta)?;
    let mut c = front_matter(
        "Game Printer: 2D game mechanics with code evidence",
        "other",
        &t,
        &files,
        false,
    );
    c.push_str("# Game Printer: 2D game mechanics with code evidence\n\n");
    let _ = writeln!(
        c,
        "The facts and mechanic parts of {} Game Printer models, unioned with a `game` column, and a taxonomy of the mechanic parts: which games use each and with which options. Made by `gp dataset catalogue` (layout {HF_FORMAT}), locally. It is not published, and holds nothing of any game: values are the models' own, evidence is cited by file, line and routine name.\n",
        games.len()
    );
    c.push_str("| Game | Model | Core |\n|---|---|---|\n");
    for g in &games {
        let _ = writeln!(
            c,
            "| {} | `{}` | {} |",
            g["game"].as_str().unwrap_or(""),
            g["model"].as_str().unwrap_or(""),
            g["core"].as_str().unwrap_or("")
        );
    }
    c.push('\n');
    table_docs(&mut c, &t, &files);
    files_doc(&mut c, &files);
    c.push_str("## Use and license\n\nIt includes models of commercial games, and of a hack whose author hasn't agreed to anything printed from it being shown, so it stays on this machine until the user decides (LICENSE.md). Names and marks belong to their owners.\n");
    fs::write(out.join("README.md"), &c)?;
    fs::write(out.join("LICENSE.md"), LICENSE_LOCAL)?;
    fs::write(
        out.join("croissant.json"),
        serde_json::to_string_pretty(&ds::croissant_for(
            "game-printer-mechanics",
            "Facts and mechanic parts of Game Printer models across games, with code evidence by routine name, and a taxonomy of the parts. No game data.",
            HF_FORMAT,
            &t,
            "parquet",
            &files,
        ))?,
    )?;
    fs::write(
        out.join("gp-dataset.json"),
        serde_json::to_string_pretty(&json!({"hf_format": HF_FORMAT, "games": games,
            "gp": {"version": env!("CARGO_PKG_VERSION"), "build": crate::corpus_score::build()},
            "files": files.iter().map(|(f, h, r)| json!({"path": f, "sha256": h, "rows": r})).collect::<Vec<_>>()}))?,
    )?;
    let mut text = format!(
        "{} games: wrote the catalogue to {}\n",
        games.len(),
        out.display()
    );
    for (f, _, r) in &files {
        let _ = writeln!(text, "  {f}: {r} rows");
    }
    Ok(crate::Outcome::ok(
        json!({"games": games, "out": out, "files": files.iter().map(|(f, h, r)| json!({"path": f, "sha256": h, "rows": r})).collect::<Vec<_>>()}),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset_tables::trivial;

    #[test]
    fn a_line_defines_a_label_or_a_constant() {
        assert_eq!(
            defines("BoundBoxCtrlData:"),
            Some(("BoundBoxCtrlData".into(), true))
        );
        assert_eq!(defines("SBnce: lda #$fc"), Some(("SBnce".into(), true)));
        assert_eq!(
            defines("JUMP_SPEED   = $FB00"),
            Some(("JUMP_SPEED".into(), false))
        );
        assert_eq!(defines(".proc main"), Some(("main".into(), true)));
        assert_eq!(defines("         lda #$fe"), None);
        assert_eq!(defines("  cmp #$09"), None);
    }

    #[test]
    fn a_column_of_small_64_bit_numbers_is_not_a_rom_run() {
        let w: Vec<u8> = [0x0102u64, 0x0103]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        assert!(trivial(&w));
    }

    #[test]
    fn a_rom_run_is_found_in_any_file_and_regular_runs_are_not() {
        let dir = tempfile::tempdir().unwrap();
        // A made-up ROM: no real game's bytes.
        let rom: Vec<u8> = (0..4096u32)
            .map(|k| (k.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        fs::write(dir.path().join("clean.txt"), b"nothing here").unwrap();
        fs::write(dir.path().join("zeros.bin"), [0u8; 64]).unwrap();
        assert!(rom_runs(dir.path(), &rom).unwrap().is_empty());
        let mut leaked = b"prefix ".to_vec();
        leaked.extend_from_slice(&rom[1000..1000 + WINDOW]);
        fs::create_dir_all(dir.path().join("data")).unwrap();
        fs::write(dir.path().join("data/leak.parquet"), &leaked).unwrap();
        assert_eq!(
            rom_runs(dir.path(), &rom).unwrap(),
            [("data/leak.parquet".to_owned(), 7)]
        );
        assert!(trivial(&[1, 2, 3, 4].repeat(4)));
        assert!(trivial(&(0..16).collect::<Vec<u8>>()));
    }
}
