//! `gp level kit GAME [-o DIR]` (docs/level-kit.md): what a level designer
//! needs to make a stage for a game by hand, from the game's model, its
//! asset map and its rules file.
//!
//! A kit is two files written from three sources:
//!
//! | Source | What it gives |
//! |---|---|
//! | The model | Each level's size, bounds, goal and spawns; the hero's moves; each thing's size and touch box; the slots a level reads |
//! | The asset map | Each slot's format, rows and bytes |
//! | `templates/GAME/level-kit/rules.toml`, written by hand | What is data and what is fixed in code, the limits, the play measures to land near, how to verify, what fits on the real engine, a worked example and what isn't known yet |
//!
//! `level-kit/kit.toml` is the machine-readable kit and `level-kit/level-kit.md`
//! the document. Neither holds a ROM table: rules, sizes, counts, boxes and
//! descriptions only.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gp_model::Model;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Outcome;

/// The kit's format.
pub const FORMAT: &str = "0.1.0";
/// The folder in a template.
pub const DIR: &str = "level-kit";
/// The rules file, written by hand.
pub const RULES_FILE: &str = "rules.toml";
/// The machine-readable kit.
pub const KIT_FILE: &str = "kit.toml";
/// The document.
pub const DOC_FILE: &str = "level-kit.md";

/// A game's rules file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub format: String,
    /// What a stage of this game is, in a paragraph.
    #[serde(default)]
    pub summary: String,
    /// The levels whose spawns the document lists; all when empty.
    #[serde(default)]
    pub levels: Vec<String>,
    /// What a stage is made of that is data: a designer changes it.
    #[serde(default)]
    pub data: Vec<Part>,
    /// What the game's code fixes: a stage must keep to it.
    #[serde(default)]
    pub code: Vec<Part>,
    #[serde(default)]
    pub limit: Vec<Limit>,
    #[serde(default)]
    pub measure: Vec<Measure>,
    #[serde(default)]
    pub verify: Vec<Step>,
    /// Findings on what fits when the stage is packed into a copy of the
    /// game's ROM and run on the real engine.
    #[serde(default)]
    pub fit: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<Example>,
    /// What isn't known yet.
    #[serde(default)]
    pub open: Vec<String>,
}

/// One rule: what it is about and the rule, with where it comes from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub what: String,
    pub rule: String,
    /// The model's asset slot it is held in, when it is one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub slot: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// A limit with its number.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    pub what: String,
    pub limit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// A play measure a stage should land near.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measure {
    /// The name `gp level measure` prints it under.
    pub name: String,
    pub what: String,
    pub target: toml::Value,
    pub within: String,
    /// The worked example's value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// A step of verifying a stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub step: String,
    pub how: String,
}

/// A worked example: a template whose stage is explained against the
/// rules.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub template: String,
    pub text: Vec<String>,
}

/// A level, from the model.
#[derive(Debug, Clone, Serialize)]
pub struct LevelInfo {
    pub name: String,
    pub description: String,
    /// Cells across and down, and a cell's side in pixels.
    pub cells: [u32; 2],
    pub tile_size: u32,
    pub pixels: [u32; 2],
    pub bounds: Vec<String>,
    pub goal: String,
    pub spawns: Vec<SpawnInfo>,
    /// The asset slots the level reads.
    pub slots: Vec<String>,
    /// Tile kinds by kind, how many tile values are each.
    pub tile_kinds: BTreeMap<String, usize>,
}

/// A spawn: what starts where.
#[derive(Debug, Clone, Serialize)]
pub struct SpawnInfo {
    pub entity: String,
    pub role: String,
    pub x: i64,
    pub y: i64,
}

/// A thing a stage holds, from the model.
#[derive(Debug, Clone, Serialize)]
pub struct Thing {
    pub entity: String,
    pub role: String,
    pub size: [i64; 2],
    /// `[x, y, w, h]` from its place.
    pub touch: [i64; 4],
    pub touch_does: String,
}

/// An asset slot a level reads, with the asset map's format.
#[derive(Debug, Clone, Serialize)]
pub struct SlotInfo {
    pub slot: String,
    pub kind: String,
    pub asset: String,
    pub format: String,
    /// Rows and bytes a row, or the region's bytes, as the map gives them.
    pub size: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The originality rules a derived stage is held to.
#[derive(Debug, Clone, Serialize)]
pub struct Originality {
    pub cells_refuse: f64,
    pub ladders_refuse: f64,
    pub shift_cells: i64,
    pub copying: Vec<String>,
}

/// The kit.
#[derive(Debug, Clone, Serialize)]
pub struct LevelKit {
    pub format: String,
    pub game: String,
    pub model: String,
    pub title: String,
    pub screen: [u32; 2],
    pub summary: String,
    pub levels: Vec<LevelInfo>,
    /// The hero's moves, by name: the model's numbers and what they give.
    pub hero: BTreeMap<String, String>,
    pub things: Vec<Thing>,
    pub slots: Vec<SlotInfo>,
    pub data: Vec<Part>,
    pub code: Vec<Part>,
    pub limits: Vec<Limit>,
    pub measures: Vec<Measure>,
    pub originality: Originality,
    pub verify: Vec<Step>,
    pub fit: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<Example>,
    /// The worked example's spawns, from its model.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub example_spawns: Vec<SpawnInfo>,
    pub open: Vec<String>,
}

/// A template's rules file, when it has one.
pub fn load_rules(template: &Path) -> Result<Option<Rules>> {
    let f = template.join(DIR).join(RULES_FILE);
    if !f.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?;
    Ok(Some(
        toml::from_str(&text).with_context(|| format!("{}", f.display()))?,
    ))
}

fn int(f: gp_model::Fixed) -> i64 {
    f.raw() >> 16
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn round(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (v * k).round() / k
}

/// Every string in a JSON value.
fn strings(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => {
            out.insert(s.clone());
        }
        Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
        Value::Object(o) => o.values().for_each(|x| strings(x, out)),
        _ => {}
    }
}

/// The hero's moves from the player entity's components.
fn hero_moves(m: &Model) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(e) = m.entities.values().find(|e| e.core.role == "player") else {
        return out;
    };
    let p = serde_json::to_value(&e.platformer).unwrap_or(Value::Null);
    let get = |a: &str, b: &str| p.get(a).and_then(|x| x.get(b)).and_then(num);
    if let Some(w) = get("run", "walk_speed") {
        out.insert(
            "walk".into(),
            format!(
                "{} px a frame, {:.0} px a second at 60 frames",
                round(w, 4),
                w * 60.0
            ),
        );
    }
    if let Some(r) = get("run", "run_speed") {
        out.insert("run".into(), format!("{} px a frame", round(r, 4)));
    }
    if let (Some(v), Some(g)) = (get("jump", "speed"), get("jump", "gravity"))
        && g > 0.0
    {
        let v = v.abs();
        out.insert(
            "jump".into(),
            format!(
                "up at {} px a frame, gravity {} px a frame each frame: about {:.0} px high and {:.0} frames in the air on flat ground (v*v/2g and 2v/g)",
                round(v, 4),
                round(g, 5),
                v * v / (2.0 * g),
                2.0 * v / g
            ),
        );
        if let Some(gh) = get("jump", "gravity_held").filter(|g| *g > 0.0) {
            out.insert(
                "jump held".into(),
                format!(
                    "with the button held, gravity {} while rising: about {:.0} px high",
                    round(gh, 5),
                    v * v / (2.0 * gh)
                ),
            );
        }
        if let Some(w) = get("run", "walk_speed") {
            let mut t = format!("about {:.0} px walking", w * 2.0 * v / g);
            if let Some(r) = get("run", "run_speed") {
                t.push_str(&format!(", {:.0} running", r * 2.0 * v / g));
            }
            out.insert("jump across".into(), t);
        }
    }
    if let Some(f) = get("jump", "max_fall") {
        out.insert("fall".into(), format!("at most {} px a frame", round(f, 4)));
    }
    if let Some(d) = get("jump", "fatal_drop") {
        out.insert(
            "fatal drop".into(),
            format!("a fall of more than {d} px kills"),
        );
    }
    if let Some(c) = get("climb", "speed") {
        out.insert(
            "climb".into(),
            format!(
                "{} px a frame, about {:.0} frames for 30 px",
                round(c, 4),
                30.0 / c.max(1e-6)
            ),
        );
    }
    out
}

/// The asset map's entry for an asset, as a table.
fn map_entry(map: &toml::Table, asset: &str) -> Option<toml::Table> {
    map.get("asset")?
        .as_array()?
        .iter()
        .filter_map(|a| a.as_table())
        .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(asset))
        .cloned()
}

fn size_words(e: &toml::Table) -> String {
    let i = |k: &str| e.get(k).and_then(toml::Value::as_integer);
    let mut out = Vec::new();
    match (i("count"), i("width").filter(|_| i("height").is_none())) {
        (Some(c), Some(w)) => out.push(format!("{c} rows of {w} bytes ({} bytes)", c * w)),
        (Some(c), None) if e.get("format").and_then(|f| f.as_str()) == Some("palette") => {
            out.push(format!("{c} palettes of 4 colors"));
        }
        (Some(c), None) => {
            let cols = e
                .get("column")
                .and_then(|c| c.as_array())
                .map_or(1, |c| c.len() as i64);
            out.push(format!(
                "{c} rows of {cols} column{}",
                if cols == 1 { "" } else { "s" }
            ));
        }
        _ => {}
    }
    if let (Some(w), Some(h)) = (i("width"), i("height")) {
        out.push(format!("{w} by {h} cells"));
    }
    if let Some(s) = i("size") {
        out.push(format!("{s} bytes in all"));
    }
    if let Some(n) = e.get("entry").and_then(|x| x.as_array()) {
        out.push(format!("{} entries share it", n.len()));
    }
    if let Some(t) = i("tiles") {
        out.push(format!("{t} tiles"));
    }
    out.join(", ")
}

/// A template directory: the path itself when it holds a model, else a
/// template id under ./templates or a game's folder in the games checkout
/// (gameprinter keeps commercial games there, game-printer kept them all
/// under templates/).
pub(crate) fn template_dir(arg: &Path) -> Result<PathBuf> {
    if arg.join("template.toml").is_file() || arg.join(gp_model::MODEL_FILE).is_file() {
        return Ok(arg.to_owned());
    }
    let t = Path::new("templates").join(arg);
    if t.join("template.toml").is_file() {
        return Ok(t);
    }
    if let Some(g) = crate::roms_cmd::games_dir(None).map(|g| g.join(arg))
        && g.join("template.toml").is_file()
    {
        return Ok(g);
    }
    anyhow::bail!(
        "{} isn't a template directory, a template id or a game in the games checkout",
        arg.display()
    )
}

/// The worked example's template: beside this one, else found as any
/// template is.
fn example_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let beside = dir.parent()?.join(name);
    if beside.join(gp_model::MODEL_FILE).is_file() {
        return Some(beside);
    }
    template_dir(Path::new(name)).ok()
}

/// A template's model, read from its directory or its model file.
pub(crate) fn read_model(arg: &Path) -> Result<(Model, PathBuf)> {
    let (file, dir) = if arg.is_dir() {
        (arg.join(gp_model::MODEL_FILE), arg.to_owned())
    } else {
        (
            arg.to_owned(),
            arg.parent().map(Path::to_owned).unwrap_or_default(),
        )
    };
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let m = Model::parse(&text).with_context(|| format!("{}", file.display()))?;
    Ok((m, dir))
}

/// The template's id and its asset map, from template.toml.
pub(crate) fn manifest(dir: &Path) -> (String, Option<PathBuf>) {
    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Ok(v) = fs::read_to_string(dir.join("template.toml"))
        .map_err(|_| ())
        .and_then(|t| t.parse::<toml::Table>().map_err(|_| ()))
    else {
        return (id, None);
    };
    let id = v
        .get("template")
        .and_then(|t| t.get("id"))
        .and_then(|x| x.as_str())
        .map_or(id, str::to_owned);
    let map = v
        .get("recipe")
        .and_then(|r| r.get("asset_map"))
        .and_then(|x| x.as_str())
        .map(|m| dir.join(m));
    (id, map)
}

/// The kit, from a template's model, asset map and rules.
pub fn kit_of(dir: &Path, m: &Model, rules: &Rules) -> Result<LevelKit> {
    let (id, map_file) = manifest(dir);
    let map: toml::Table = match &map_file {
        Some(f) => fs::read_to_string(f)
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or_default(),
        None => toml::Table::new(),
    };
    let present = m.present.clone().unwrap_or_default();
    let screen = present.screen.as_ref().map_or([256, 240], |s| s.size);
    let role_of = |n: &str| {
        m.entities
            .get(n)
            .map(|e| e.core.role.clone())
            .unwrap_or_default()
    };
    let mut levels = Vec::new();
    let mut read = BTreeSet::new();
    for (name, l) in &m.levels {
        let mut names = BTreeSet::new();
        strings(&serde_json::to_value(l).unwrap_or(Value::Null), &mut names);
        let slots: Vec<String> = names
            .into_iter()
            .filter(|s| m.assets.contains_key(s))
            .collect();
        read.extend(slots.iter().cloned());
        let ts = l.core.tile_size;
        let pixels = [l.core.size[0] * ts, l.core.size[1] * ts];
        let p = &l.platformer;
        let mut bounds = vec![format!(
            "the level is {} by {} px; the screen {} by {}",
            pixels[0], pixels[1], screen[0], screen[1]
        )];
        if let Some(k) = p.kill_below {
            bounds.push(format!("a body below row {} is out", int(k)));
        }
        let goal = match &p.goal {
            Some(g) => match g.under {
                Some(u) => format!(
                    "the hero's left edge at x {} or past, with its top above row {}",
                    int(g.x),
                    int(u)
                ),
                None => format!("the hero's left edge at x {} or past", int(g.x)),
            },
            None => "set by the rules or a hook, not a place".into(),
        };
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for k in l.core.tile_kinds.values() {
            let k = serde_json::to_value(k)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("{k:?}"));
            *kinds.entry(k).or_default() += 1;
        }
        let shown = rules.levels.is_empty() || rules.levels.contains(name);
        let spawns = l
            .core
            .spawn
            .iter()
            .filter(|_| shown)
            .filter(|s| {
                m.entities
                    .get(&s.entity)
                    .is_some_and(|e| e.core.sprite.is_some())
            })
            .map(|s| SpawnInfo {
                entity: s.entity.clone(),
                role: role_of(&s.entity),
                x: s.x.map_or(0, int),
                y: s.y.map_or(0, int),
            })
            .collect();
        levels.push(LevelInfo {
            name: name.clone(),
            description: l.core.description.clone().unwrap_or_default(),
            cells: l.core.size,
            tile_size: ts,
            pixels,
            bounds,
            goal,
            spawns,
            slots,
            tile_kinds: kinds,
        });
    }
    for p in rules.data.iter().chain(&rules.code) {
        if !p.slot.is_empty() {
            read.insert(p.slot.clone());
        }
    }
    let mut slots = Vec::new();
    for s in &read {
        let Some(a) = m.assets.get(s) else { continue };
        let asset = a.from.as_ref().map(|f| f.asset.clone()).unwrap_or_default();
        let e = map_entry(&map, &asset);
        slots.push(SlotInfo {
            slot: s.clone(),
            kind: serde_json::to_value(a.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            asset: asset.clone(),
            format: e
                .as_ref()
                .and_then(|e| e.get("format").and_then(|f| f.as_str()).map(str::to_owned))
                .unwrap_or_else(|| "not in the asset map".into()),
            size: e.as_ref().map(size_words).unwrap_or_default(),
            note: a.description.clone().unwrap_or_default(),
        });
    }
    let mut things = Vec::new();
    for (n, e) in &m.entities {
        if e.core.sprite.is_none() {
            continue;
        }
        let h = &e.core.hitbox;
        let touch = [int(h.x), int(h.y), int(h.w), int(h.h)];
        if touch[2] * touch[3] <= 1 && e.core.role != "player" {
            continue;
        }
        let does = e
            .platformer
            .contact
            .as_ref()
            .and_then(|c| serde_json::to_value(c.otherwise).ok())
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| {
                if e.core.role == "player" {
                    "the hero's box".into()
                } else {
                    "a hook decides".into()
                }
            });
        things.push(Thing {
            entity: n.clone(),
            role: e.core.role.clone(),
            size: e.core.size.map(int),
            touch,
            touch_does: does,
        });
    }
    let example_spawns = rules
        .example
        .as_ref()
        .and_then(|x| {
            let d = example_dir(dir, &x.template)?;
            let (em, _) = read_model(&d).ok()?;
            let l = em.levels.values().next()?;
            Some(
                l.core
                    .spawn
                    .iter()
                    .filter(|s| {
                        em.entities
                            .get(&s.entity)
                            .is_some_and(|e| e.core.sprite.is_some())
                    })
                    .map(|s| SpawnInfo {
                        entity: s.entity.clone(),
                        role: em
                            .entities
                            .get(&s.entity)
                            .map(|e| e.core.role.clone())
                            .unwrap_or_default(),
                        x: s.x.map_or(0, int),
                        y: s.y.map_or(0, int),
                    })
                    .collect(),
            )
        })
        .unwrap_or_default();
    Ok(LevelKit {
        format: FORMAT.into(),
        game: id,
        model: m.model.name.clone(),
        title: m.model.title.clone(),
        screen,
        summary: rules.summary.clone(),
        levels,
        hero: hero_moves(m),
        things,
        slots,
        data: rules.data.clone(),
        code: rules.code.clone(),
        limits: rules.limit.clone(),
        measures: rules.measure.clone(),
        originality: Originality {
            cells_refuse: crate::template_original::CELLS_REFUSE,
            ladders_refuse: crate::template_original::LADDERS_REFUSE,
            shift_cells: gp_interp::stage::SHIFT_REACH,
            copying: vec![
                format!(
                    "the layout: {:.0}% or more of the cells either level fills (ground, ladder) the same kind in both, at the best shift of up to {} cells each way, mirrored or not",
                    crate::template_original::CELLS_REFUSE * 100.0,
                    gp_interp::stage::SHIFT_REACH
                ),
                format!(
                    "the ladders: {:.0}% or more of them within a tile across of one of the original's, between the same floors counted from the bottom, at that shift",
                    crate::template_original::LADDERS_REFUSE * 100.0
                ),
                "the data: any line of 24 characters or more of the stage's built files that the original's lifted files hold too (gp template served's rule)".into(),
                "tracing: a stage drawn over the original's picture, or laid out from a guide made from the ROM, is a copy whatever it scores".into(),
                "the art: a theme's sprites and pictures are held to the theme kit's originality check (docs/theme-kit.md, \"The originality check\")".into(),
            ],
        },
        verify: rules.verify.clone(),
        fit: rules.fit.clone(),
        example: rules.example.clone(),
        example_spawns,
        open: rules.open.clone(),
    })
}

/// A table row: cells joined by `|`, with any `|` in them escaped.
fn row(cells: &[String]) -> String {
    format!(
        "| {} |\n",
        cells
            .iter()
            .map(|c| c.replace('|', "\\|").replace('\n', " "))
            .collect::<Vec<_>>()
            .join(" | ")
    )
}

fn value_text(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// The document.
pub fn doc(k: &LevelKit) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# Level kit: {} (`{}`)\n\nWritten by `gp level kit {}` from the model, the asset map and `level-kit/{RULES_FILE}` (docs/level-kit.md). Edit the rules file, not this one. It holds rules, sizes, counts and boxes, never a table of the game's ROM. `.claude/skills/level-design/SKILL.md` says how to design a stage from it by hand.\n",
        k.title, k.game, k.game
    );
    if !k.summary.is_empty() {
        let _ = writeln!(s, "{}\n", k.summary.trim());
    }
    s.push_str("## The levels\n\n| Level | Size | Bounds | Goal |\n|---|---|---|---|\n");
    for l in &k.levels {
        s.push_str(&row(&[
            format!("`{}`: {}", l.name, l.description),
            format!(
                "{} by {} cells of {} px",
                l.cells[0], l.cells[1], l.tile_size
            ),
            l.bounds.join("; "),
            l.goal.clone(),
        ]));
    }
    for l in &k.levels {
        if l.spawns.is_empty() {
            continue;
        }
        let _ = writeln!(
            s,
            "\nWhat `{}` starts with (the model's spawns, places in pixels from the level's top left):\n\n| Thing | Role | x | y |\n|---|---|---|---|",
            l.name
        );
        for sp in &l.spawns {
            s.push_str(&row(&[
                format!("`{}`", sp.entity),
                sp.role.clone(),
                sp.x.to_string(),
                sp.y.to_string(),
            ]));
        }
    }
    s.push_str("\n## What's data and what's fixed in code\n\n");
    if k.data.is_empty() && k.code.is_empty() {
        s.push_str("Not written yet: the rules file lists none (see Open).\n");
    } else {
        s.push_str("A stage changes what is data. What the code fixes, a stage keeps to: change it and the stage no longer runs on the game's own engine.\n\n| | What | Rule | Slot | Source |\n|---|---|---|---|---|\n");
        for (kind, parts) in [("data", &k.data), ("code", &k.code)] {
            for p in parts {
                s.push_str(&row(&[
                    kind.into(),
                    p.what.clone(),
                    p.rule.clone(),
                    if p.slot.is_empty() {
                        String::new()
                    } else {
                        format!("`{}`", p.slot)
                    },
                    p.source.clone(),
                ]));
            }
        }
    }
    s.push_str("\n## Limits\n\n");
    if k.limits.is_empty() {
        s.push_str(
            "Not written yet (see Open). The slots below give the sizes the asset map holds.\n",
        );
    } else {
        s.push_str("| What | Limit | Why | Source |\n|---|---|---|---|\n");
        for l in &k.limits {
            s.push_str(&row(&[
                l.what.clone(),
                l.limit.clone(),
                l.why.clone(),
                l.source.clone(),
            ]));
        }
    }
    if !k.slots.is_empty() {
        s.push_str("\nThe data slots the levels read, as the asset map holds them:\n\n| Slot | Kind | Format | Size | What |\n|---|---|---|---|---|\n");
        for sl in &k.slots {
            s.push_str(&row(&[
                format!("`{}`", sl.slot),
                sl.kind.clone(),
                sl.format.clone(),
                sl.size.clone(),
                sl.note.chars().take(160).collect(),
            ]));
        }
    }
    if !k.hero.is_empty() || !k.things.is_empty() {
        s.push_str("\n## The hero and the things in a stage\n\nFrom the model. A gap the hero must jump, a ladder it must reach and a fall it must survive are measured against these.\n\n");
        if !k.hero.is_empty() {
            s.push_str("| Move | The model's numbers |\n|---|---|\n");
            for (n, v) in &k.hero {
                s.push_str(&row(&[n.clone(), v.clone()]));
            }
        }
        if !k.things.is_empty() {
            s.push_str("\n| Thing | Role | Size | Touch box (x, y, w, h from its place) | A touch |\n|---|---|---|---|---|\n");
            for t in &k.things {
                s.push_str(&row(&[
                    format!("`{}`", t.entity),
                    t.role.clone(),
                    format!("{} by {}", t.size[0], t.size[1]),
                    format!(
                        "{}, {}, {}, {}",
                        t.touch[0], t.touch[1], t.touch[2], t.touch[3]
                    ),
                    t.touch_does.clone(),
                ]));
            }
        }
    }
    if !k.things.is_empty() {
        s.push_str("\nThe touch boxes are the model's, as its touch tests read them. The theme kit (`themes/kit/kit.toml`, each slot's `geometry`) gives each in its picture's pixels, as the screen shows it.\n");
    }
    s.push_str("\n## Play measures to land near\n\n");
    if k.measures.is_empty() {
        s.push_str("None yet (see Open).\n");
    } else {
        s.push_str("`gp level measure --template T` prints each under its name. A stage of this kind should land within the tolerance of each target.\n\n| Measure | What | Target | Within | Worked example | Source |\n|---|---|---|---|---|---|\n");
        for m in &k.measures {
            s.push_str(&row(&[
                format!("`{}`", m.name),
                m.what.clone(),
                value_text(&m.target),
                m.within.clone(),
                m.example.as_ref().map(value_text).unwrap_or_default(),
                m.source.clone(),
            ]));
        }
    }
    let o = &k.originality;
    let _ = writeln!(
        s,
        "\n## Originality\n\n`gp template check T --original` compares a stage derived from this game with the original's, lifted in memory from your ROM. It is refused at cells {} or ladders {}. What counts as copying:\n",
        o.cells_refuse, o.ladders_refuse
    );
    for c in &o.copying {
        let _ = writeln!(s, "- {c}");
    }
    s.push_str("\n## How to verify\n\n");
    if k.verify.is_empty() {
        s.push_str("Not written yet (see Open).\n");
    } else {
        s.push_str("| Step | How |\n|---|---|\n");
        for v in &k.verify {
            s.push_str(&row(&[v.step.clone(), v.how.clone()]));
        }
    }
    if !k.fit.is_empty() {
        s.push_str(
            "\n## What fits on the real engine\n\n| What | Finding | Source |\n|---|---|---|\n",
        );
        for f in &k.fit {
            s.push_str(&row(&[f.what.clone(), f.rule.clone(), f.source.clone()]));
        }
    }
    if let Some(x) = &k.example {
        let _ = writeln!(s, "\n## Worked example: `{}`\n", x.template);
        for p in &x.text {
            let _ = writeln!(s, "{}\n", p.trim());
        }
        if !k.example_spawns.is_empty() {
            s.push_str(
                "Its spawns, from its model:\n\n| Thing | Role | x | y |\n|---|---|---|---|\n",
            );
            for sp in &k.example_spawns {
                s.push_str(&row(&[
                    format!("`{}`", sp.entity),
                    sp.role.clone(),
                    sp.x.to_string(),
                    sp.y.to_string(),
                ]));
            }
        }
    }
    s.push_str("\n## Open\n\n");
    if k.open.is_empty() {
        s.push_str("Nothing listed.\n");
    } else {
        for o in &k.open {
            let _ = writeln!(s, "- {o}");
        }
    }
    s
}

/// The kit's TOML.
pub fn kit_text(k: &LevelKit) -> Result<String> {
    Ok(format!(
        "# The level kit for {} (docs/level-kit.md), written by `gp level kit` from the\n# model, the asset map and level-kit/{RULES_FILE}. Edit the rules file, not this one.\n{}",
        k.game,
        toml::to_string(k).context("the kit serializes")?
    ))
}

/// A rules file for a new game: the parts no tool knows yet, as open items.
pub fn stub_rules(game: &str) -> String {
    format!(
        r#"# The level kit's rules for {game} (docs/level-kit.md), written by hand.
# `gp level kit {game}` puts them into kit.toml and level-kit.md with what the
# model and asset map say. Fill each list as the game's code is read.
format = "{FORMAT}"
summary = "What a stage of this game is, in a paragraph: what the hero does in it and how it ends."

# What a stage is made of that is data, one entry a part:
# [[data]]
# what = "floors"
# rule = "the runs the walk reads: x, y, width"
# slot = "ground_runs"
# source = "where it was read"

# What the game's code fixes, as [[code]] entries of the same keys.

# Limits with numbers: [[limit]] what, limit, why, source.

# Play measures a stage should land near: [[measure]] name, what, target, within, source.

open = [
  "What is data and what is fixed in code: not read yet",
  "Limits: counts and the space left in the ROM, not measured yet",
  "Play measures: none taken yet",
]
"#
    )
}

/// Writes a template's kit into `out` (default `templates/GAME/level-kit`).
pub fn write(dir: &Path, m: &Model, out: &Path) -> Result<(LevelKit, Vec<PathBuf>)> {
    let rules = load_rules(dir)?.unwrap_or_else(|| Rules {
        format: FORMAT.into(),
        open: vec![
            "No rules file yet: what's data and what's fixed in code, the limits and the measures aren't written".into(),
        ],
        ..Rules::default()
    });
    let k = kit_of(dir, m, &rules)?;
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let files = vec![out.join(KIT_FILE), out.join(DOC_FILE)];
    fs::write(&files[0], kit_text(&k)?)?;
    fs::write(&files[1], doc(&k))?;
    Ok((k, files))
}

/// `gp level kit`.
pub fn kit_cmd(game: &Path, out: Option<&Path>) -> Result<Outcome> {
    let dir = template_dir(game)?;
    let (m, _) = read_model(&dir)?;
    let out = out.map_or_else(|| dir.join(DIR), Path::to_owned);
    let (k, files) = write(&dir, &m, &out)?;
    let text = format!(
        "wrote {} and {}: {} levels, {} data and {} code rules, {} limits, {} measures, {} open items",
        files[0].display(),
        files[1].display(),
        k.levels.len(),
        k.data.len(),
        k.code.len(),
        k.limits.len(),
        k.measures.len(),
        k.open.len()
    );
    Ok(Outcome::ok(
        json!({"kit": files[0], "doc": files[1], "levels": k.levels.len(),
               "data": k.data.len(), "code": k.code.len(), "limits": k.limits.len(),
               "measures": k.measures.len(), "open": k.open}),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn committed_level_kits_are_current() {
        // Each template with a rules file has its kit and document as
        // `gp level kit` writes them now.
        // gameprinter holds Moss Hop under templates/; the commercial games'
        // kits are in the games checkout, whose own checks keep them.
        let dirs: Vec<PathBuf> = fs::read_dir(repo().join("templates"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        let mut seen = 0;
        for dir in dirs {
            if !dir.join(DIR).join(RULES_FILE).is_file() {
                continue;
            }
            seen += 1;
            let (m, _) = read_model(&dir).unwrap();
            let rules = load_rules(&dir).unwrap().unwrap();
            let k = kit_of(&dir, &m, &rules).unwrap();
            for (file, want) in [(KIT_FILE, kit_text(&k).unwrap()), (DOC_FILE, doc(&k))] {
                let have = fs::read_to_string(dir.join(DIR).join(file)).unwrap_or_default();
                assert!(
                    have == want,
                    "{} is stale; run gp level kit {}",
                    dir.join(DIR).join(file).display(),
                    dir.display()
                );
            }
            for t in [kit_text(&k).unwrap(), doc(&k)] {
                assert!(t.is_ascii(), "{}: not ASCII", dir.display());
                assert!(!t.contains('\u{2014}'), "{}: an em dash", dir.display());
            }
        }
        assert!(seen >= 1, "Moss Hop's kit at least");
    }

    #[test]
    fn a_new_games_stub_parses_and_lists_what_is_open() {
        let r: Rules = toml::from_str(&stub_rules("made-up")).unwrap();
        assert_eq!(r.format, FORMAT);
        assert_eq!(r.open.len(), 3);
        assert!(r.data.is_empty() && r.measure.is_empty());
    }
}
