//! Reading and editing a game's levels in the kit's own terms
//! (docs/level-kit.md, "Editing a stage"): `gp level list`, `show`,
//! `render`, `edit`, `undo` and `verify`. They are how an agent makes a
//! stage for a person over MCP (gp_run), and they work the same by hand.
//!
//! A stage is the data its level reads, as the person's ROM lifted it into
//! the copy's assets: the ground's runs and their shapes, the ladders and
//! their shapes, the ladder tops and stops (the game's own lists, kept in
//! a byte table's column up to an end mark), each thing's start in the
//! model, and the other slots the level kit's rules name as the stage's
//! data. An edit is a small named operation on those pieces, or the whole
//! of them replaced, and it is checked before anything is written: every
//! slot keeps its rows and widths and its bytes, the level compiles, the
//! pieces stay on the screen, the runs make the girders the code reads,
//! ladders the code climbs stay where the model's numbers put them, the
//! hero starts on ground and the goal can be reached by full ladders. A
//! refusal names the rule (the kit's own `what` where the kit has one)
//! and the piece.
//!
//! Only a copy is ever edited: a folder in this home's work area, outside
//! any git work tree, that is not a published package. The first edit
//! gives the copy its own assets (`<copy>/assets/`, as the MCP art tools
//! do, gp_mcp::place::copy_assets), so the original lift is never
//! written. Each edit keeps the files it changed as they were in
//! `<copy>/level-edits.jsonl`, the way a copy's sound edits keep theirs
//! (`sound-edits.jsonl`), and `gp level undo` puts the newest back.
//!
//! The picture (crate::level_picture) is drawn by what each piece is, with
//! no ROM art, so it can be shown anywhere. With `--art` it is the model's
//! own picture of the stage at the start of play, from the person's lift,
//! with the edited pieces outlined over it; that one stays on this
//! computer.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use gp_assets::{Asset, AssetMap};
use gp_evidence::Paths;
use gp_interp::Lifted;
use gp_interp::stage::{self, Cells, Layout};
use gp_model::{Fixed, Model};
use gp_sim::Spec;
use gp_sim::spec::Tile;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Outcome;
use crate::level_kit::{self, Rules};
use crate::level_picture::{self, Canvas, Marks};

/// A copy's level history, one JSON object a line, oldest first.
pub(crate) const HISTORY_FILE: &str = "level-edits.jsonl";

/// Where the MCP server keeps a person's copies (`gp mcp --work`'s
/// default), so a copy can be named by its name alone.
const MCP_COPIES: &str = "mcp";

/// The pixels a ladder's end may be from the girder it meets.
const MEETS: i64 = 4;

// --- Finding the game ---------------------------------------------------------

/// A game's folder: a path, a copy's name in this home
/// (`$GP_HOME/work/mcp/<name>`), a name under templates/ or a game's
/// folder in the games checkout.
pub(crate) fn resolve(game: &Path, paths: &Paths) -> PathBuf {
    if game.exists() || game.components().count() != 1 {
        return game.to_owned();
    }
    let copy = paths.home.join("work").join(MCP_COPIES).join(game);
    if copy.join(gp_model::MODEL_FILE).is_file() {
        return copy;
    }
    crate::search::template_dir(game)
}

/// A game opened for its levels.
pub(crate) struct Game {
    pub dir: PathBuf,
    /// The name the person knows it by: the folder's name.
    pub name: String,
    pub model: Model,
    /// The lifted assets it runs on, and whether they are its own.
    pub assets: PathBuf,
    pub own_assets: bool,
    pub lifted: Lifted,
    /// The lift the game's source has in this home, as the person's ROM
    /// gave it: what a copy's own assets were made from, never written.
    pub original: Lifted,
    pub map: AssetMap,
    pub rules: Option<Rules>,
    /// None when the folder may be edited; else why not.
    pub not_editable: Option<String>,
}

impl Game {
    pub(crate) fn open(game: &Path, paths: &Paths) -> Result<Self> {
        let dir = resolve(game, paths);
        let file = dir.join(gp_model::MODEL_FILE);
        if !file.is_file() {
            bail!(
                "{} isn't a game's folder: it has no {}. Name a copy (template_fork's name), a game's folder, or a game in the games checkout",
                dir.display(),
                gp_model::MODEL_FILE
            );
        }
        let dir = std::path::absolute(&dir)?;
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("the game")
            .to_owned();
        let text = fs::read_to_string(&file).with_context(|| file.display().to_string())?;
        let model = Model::parse(&text).with_context(|| file.display().to_string())?;
        let own = dir.join("assets");
        let (assets, own_assets) = if own.join(gp_assets::MAP_FILE).is_file() {
            (own, true)
        } else {
            let source = model
                .model
                .source
                .clone()
                .context("the model names no source, so it has no lifted assets to read")?;
            (paths.home.join("work").join("assets").join(source), false)
        };
        let lifted = Lifted::read_dir(&assets, &model).map_err(|e| {
            anyhow!(
                "{name}'s lifted assets aren't there ({e}). Its levels come from the person's own ROM: lift them with gp assets lift ROM (template_fork does it when the ROM is in $GP_ROM_DIR)"
            )
        })?;
        let map = lifted.map().map_err(|e| anyhow!("{e}"))?;
        let shared = model
            .model
            .source
            .as_ref()
            .map(|s| paths.home.join("work").join("assets").join(s));
        let original = match shared {
            Some(s) if own_assets => Lifted::read_dir(&s, &model).map_err(|e| {
                anyhow!(
                    "the original lift of {name} isn't in {} ({e}): the layout can't be held apart from the original's without it; lift it again with gp assets lift ROM",
                    s.display()
                )
            })?,
            _ => lifted.clone(),
        };
        let rules = level_kit::load_rules(&dir)?;
        let not_editable = editable(&dir, paths).err();
        Ok(Self {
            dir,
            name,
            model,
            assets,
            own_assets,
            lifted,
            original,
            map,
            rules,
            not_editable,
        })
    }

    /// The kit's rules when they are more than the stub every new game
    /// starts with.
    fn kit(&self) -> Option<&Rules> {
        self.rules
            .as_ref()
            .filter(|r| !r.data.is_empty() || !r.code.is_empty() || !r.limit.is_empty())
    }

    /// Why the game has no kit, in words, when it has none.
    fn no_kit(&self) -> Option<String> {
        no_kit_words(&self.name, &self.dir, self.rules.as_ref())
    }
}

/// Why a game has no level kit, in words; None when it has one (a rules
/// file with more than the stub every new game starts with).
fn no_kit_words(name: &str, dir: &Path, rules: Option<&Rules>) -> Option<String> {
    if rules.is_some_and(|r| !r.data.is_empty() || !r.code.is_empty() || !r.limit.is_empty()) {
        return None;
    }
    let what = if rules.is_some() {
        "its level-kit/rules.toml is still the stub gp game new writes, with every part open"
    } else {
        "it has no level-kit/rules.toml"
    };
    Some(format!(
        "{name} has no level kit yet: {what}. A kit says what of a stage is data and in which slot, what the game's code fixes, the limits with numbers and the play measures to land near (docs/level-kit.md); someone reads the game's code for each data slot first and writes level-kit/rules.toml, then runs gp level kit {}",
        dir.display()
    ))
}

/// Whether `dir` is a person's copy that an edit may write: in this
/// home's work area, outside any git work tree and not a package as
/// `gp game fetch` writes one. The words say what to do instead.
pub(crate) fn editable(dir: &Path, paths: &Paths) -> Result<(), String> {
    editable_in(
        dir,
        &paths.home,
        crate::roms_cmd::games_dir(None).as_deref(),
    )
}

fn editable_in(dir: &Path, home: &Path, games: Option<&Path>) -> Result<(), String> {
    let dir = std::path::absolute(dir).map_err(|e| e.to_string())?;
    let make = "make a copy first (MCP: template_fork; by hand: gp template extract) and edit that";
    if let Some(g) = games.and_then(|g| std::path::absolute(g).ok())
        && dir.starts_with(&g)
    {
        return Err(format!(
            "{} is the games checkout's original of this game, which an edit never changes: {make}",
            dir.display()
        ));
    }
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&dir) {
        return Err(format!(
            "{} is inside the git work tree {}: originals and checked-in packages are never edited by a level edit; {make}",
            dir.display(),
            repo.display()
        ));
    }
    if dir.join("lifted").is_dir() && dir.join("package.toml").is_file() {
        return Err(format!(
            "{} is a published package as gp game fetch wrote it, which stays as published: {make}",
            dir.display()
        ));
    }
    let work = std::path::absolute(home.join("work")).map_err(|e| e.to_string())?;
    if !dir.starts_with(&work) {
        return Err(format!(
            "{} isn't a copy in this home ({}): {make}",
            dir.display(),
            work.display()
        ));
    }
    Ok(())
}

// --- Slots as rows ----------------------------------------------------------

/// How a lifted slot's file holds its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Format {
    /// `byte-rows`: `rows = [[..], ..]`.
    Rows,
    /// `table`: `[[row]]` entries, these columns each.
    Table(Vec<String>),
    /// `level-grid`: `rows = ["..", ..]`, one character a cell, each the
    /// metatile of its place in the legend (the file's `#   C  N` lines,
    /// as gp_assets::text::write_level writes them). A row's values are
    /// the metatiles' numbers.
    Grid(Vec<char>),
}

/// The legend a lifted level grid's header lists: `#   C  N name` lines,
/// the character of metatile N, in order.
fn grid_legend(header: &str) -> Vec<char> {
    let mut legend = Vec::new();
    for line in header.lines() {
        let Some(rest) = line.strip_prefix("#   ") else {
            continue;
        };
        let mut chars = rest.chars();
        let (Some(c), Some(' '), Some(' ')) = (chars.next(), chars.next(), chars.next()) else {
            continue;
        };
        let n: String = chars.take_while(char::is_ascii_digit).collect();
        if n.parse::<usize>().ok() == Some(legend.len()) {
            legend.push(c);
        }
    }
    legend
}

/// One slot's rows, as its lifted file holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SlotData {
    file: String,
    format: Format,
    rows: Vec<Vec<i64>>,
    /// The file's lines before its rows.
    header: String,
}

impl SlotData {
    fn parse(file: &str, text: &str) -> Result<Self, String> {
        let doc: toml::Table = toml::from_str(text).map_err(|e| format!("{file}: {e}"))?;
        let int = |v: &toml::Value, i: usize| {
            v.as_integer()
                .ok_or(format!("{file}: row {i} holds something not a number"))
        };
        if let Some(rows) = doc.get("rows").and_then(toml::Value::as_array) {
            let header = text
                .split_once("\nrows = [")
                .map(|(h, _)| format!("{h}\n"))
                .unwrap_or_default();
            if !rows.is_empty() && rows.iter().all(toml::Value::is_str) {
                let legend = grid_legend(&header);
                let rows = rows
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        r.as_str()
                            .unwrap_or_default()
                            .chars()
                            .map(|c| {
                                legend.iter().position(|&l| l == c).map(|p| p as i64).ok_or(
                                    format!(
                                        "{file}: row {i} has `{c}`, which its legend doesn't list"
                                    ),
                                )
                            })
                            .collect()
                    })
                    .collect::<Result<_, _>>()?;
                return Ok(Self {
                    file: file.to_owned(),
                    format: Format::Grid(legend),
                    rows,
                    header,
                });
            }
            let rows = rows
                .iter()
                .enumerate()
                .map(|(i, r)| match r.as_array() {
                    Some(a) => a.iter().map(|v| int(v, i)).collect(),
                    None => Err(format!(
                        "{file}: row {i} is text, which a level edit doesn't change"
                    )),
                })
                .collect::<Result<_, _>>()?;
            return Ok(Self {
                file: file.to_owned(),
                format: Format::Rows,
                rows,
                header,
            });
        }
        if let Some(rows) = doc.get("row").and_then(toml::Value::as_array) {
            let cols: Vec<String> = rows
                .first()
                .and_then(toml::Value::as_table)
                .map(|t| t.keys().cloned().collect())
                .unwrap_or_default();
            let rows = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    cols.iter()
                        .map(|c| {
                            r.get(c)
                                .ok_or(format!("{file}: row {i} has no `{c}`"))
                                .and_then(|v| int(v, i))
                        })
                        .collect()
                })
                .collect::<Result<_, _>>()?;
            let mut header = String::new();
            for line in text.lines() {
                if line.trim() == "[[row]]" {
                    break;
                }
                header.push_str(line);
                header.push('\n');
            }
            // The comment naming the first row goes back with the rows.
            while header.ends_with("\n\n") || header.trim_end().ends_with("# 0") {
                let t = header.trim_end_matches('\n');
                header = match t.rsplit_once('\n') {
                    Some((h, last)) if last.starts_with('#') && last.trim() == "# 0" => {
                        format!("{h}\n")
                    }
                    _ => format!("{t}\n"),
                };
                if !header.trim_end().ends_with("# 0") {
                    break;
                }
            }
            return Ok(Self {
                file: file.to_owned(),
                format: Format::Table(cols),
                rows,
                header,
            });
        }
        Err(format!(
            "{file} holds neither rows nor a table, so a level edit can't change it"
        ))
    }

    /// The file's text with these rows.
    fn text(&self) -> String {
        let mut out = self.header.clone();
        match &self.format {
            Format::Rows => {
                out.push_str("rows = [\n");
                for (i, r) in self.rows.iter().enumerate() {
                    let items: Vec<String> = r.iter().map(|b| format!("0x{b:02X}")).collect();
                    if r.len() > 16 {
                        let _ = writeln!(out, "  [ # {i}");
                        for line in items.chunks(16) {
                            let _ = writeln!(out, "    {},", line.join(", "));
                        }
                        out.push_str("  ],\n");
                    } else {
                        let _ = writeln!(out, "  [{}], # {i}", items.join(", "));
                    }
                }
                out.push_str("]\n");
            }
            Format::Grid(legend) => {
                out.push_str("rows = [\n");
                for (i, r) in self.rows.iter().enumerate() {
                    let text: String = r
                        .iter()
                        .map(|&v| {
                            usize::try_from(v)
                                .ok()
                                .and_then(|v| legend.get(v))
                                .copied()
                                .unwrap_or('?')
                        })
                        .collect();
                    let _ = writeln!(out, "  \"{text}\", # {i}");
                }
                out.push_str("]\n");
            }
            Format::Table(cols) => {
                if !out.ends_with("\n\n") {
                    out.push('\n');
                }
                for (i, r) in self.rows.iter().enumerate() {
                    let _ = writeln!(out, "# {i}\n[[row]]");
                    for (c, v) in cols.iter().zip(r) {
                        let _ = writeln!(out, "{c} = {v}");
                    }
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Refuses a value a row of this slot can't hold: in a level grid,
    /// a number its legend has no metatile for.
    fn holds(&self, v: i64, what: &str) -> Result<(), String> {
        match &self.format {
            Format::Grid(legend) if usize::try_from(v).map_or(true, |v| v >= legend.len()) => {
                Err(format!(
                    "{what} is {v}: a cell of this grid is a metatile from 0 to {}",
                    legend.len().saturating_sub(1)
                ))
            }
            _ => Ok(()),
        }
    }

    fn column(&self, name: &str) -> Option<usize> {
        match &self.format {
            Format::Table(cols) => cols.iter().position(|c| c == name),
            Format::Rows | Format::Grid(_) => None,
        }
    }
}

/// The asset a model slot is filled from.
fn slot_asset<'a>(model: &Model, map: &'a AssetMap, slot: &str) -> Result<&'a Asset, String> {
    let from = model
        .assets
        .get(slot)
        .ok_or(format!("the model has no slot `{slot}`"))?
        .from
        .as_ref()
        .ok_or(format!("the slot `{slot}` names no asset"))?;
    map.assets
        .iter()
        .find(|a| a.name() == from.asset)
        .ok_or(format!(
            "the slot `{slot}` names `{}`, which the lift doesn't have",
            from.asset
        ))
}

/// A list of rows the level reads: a whole slot, or the game's own list in
/// a byte table's column (feature `level.ladder_lists`) from `at`, `width`
/// bytes a row, up to a row that starts with `end`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListRef {
    slot: String,
    column: Option<String>,
    at: usize,
    width: usize,
    end: Option<i64>,
}

impl ListRef {
    fn of(name: &str, at: Option<u32>, width: usize, end: Option<i64>) -> Self {
        match (name.split_once('.'), at) {
            (Some((slot, col)), Some(at)) => ListRef {
                slot: slot.to_owned(),
                column: Some(col.to_owned()),
                at: at as usize,
                width,
                end,
            },
            _ => ListRef {
                slot: name.to_owned(),
                column: None,
                at: 0,
                width,
                end: None,
            },
        }
    }

    /// Where it is, in words: `ladders` or `climb_rom.byte from 0`.
    fn named(&self) -> String {
        match &self.column {
            None => self.slot.clone(),
            Some(c) => format!("{}.{c} from row {}", self.slot, self.at),
        }
    }
}

/// Where a slot's file is found: the model and the lift's map.
type At<'a> = (&'a Model, &'a AssetMap);

impl Game {
    fn at(&self) -> At<'_> {
        (&self.model, &self.map)
    }
}

/// The slots an edit reads and writes, loaded as they are named.
#[derive(Clone)]
struct Data {
    /// The lift the slots are read from.
    base: Lifted,
    slots: BTreeMap<String, SlotData>,
    changed: BTreeSet<String>,
    /// Files written whole, by name: the stage's picture redrawn.
    raw: BTreeMap<String, String>,
}

impl Data {
    fn of(base: &Lifted) -> Self {
        Data {
            base: base.clone(),
            slots: BTreeMap::new(),
            changed: BTreeSet::new(),
            raw: BTreeMap::new(),
        }
    }

    fn load(&mut self, g: At<'_>, slot: &str) -> Result<&mut SlotData, String> {
        if !self.slots.contains_key(slot) {
            let asset = slot_asset(g.0, g.1, slot)?;
            let file = asset
                .files()
                .into_iter()
                .next()
                .ok_or(format!("the slot `{slot}` has no file"))?;
            let text = self
                .base
                .files
                .get(&file)
                .ok_or(format!("the lift has no {file}"))?;
            self.slots
                .insert(slot.to_owned(), SlotData::parse(&file, text)?);
        }
        Ok(self.slots.get_mut(slot).expect("loaded"))
    }

    /// A list's rows.
    fn read(&mut self, g: At<'_>, l: &ListRef) -> Result<Vec<Vec<i64>>, String> {
        let s = self.load(g, &l.slot)?;
        let Some(col) = &l.column else {
            return Ok(s.rows.clone());
        };
        let c = s
            .column(col)
            .ok_or(format!("{} has no column `{col}`", l.slot))?;
        let bytes: Vec<i64> = s.rows.iter().map(|r| r[c]).collect();
        Ok(bytes
            .get(l.at..)
            .unwrap_or_default()
            .chunks(l.width)
            .take_while(|r| l.end.is_none_or(|e| r.first() != Some(&e)))
            .map(<[i64]>::to_vec)
            .collect())
    }

    /// Writes a list's rows back: for a whole slot, the same number of
    /// rows; for a list with an end mark, up to `room` rows and the mark
    /// after them, the freed bytes set to 0.
    fn write(
        &mut self,
        g: At<'_>,
        l: &ListRef,
        rows: &[Vec<i64>],
        room: usize,
    ) -> Result<(), String> {
        let s = self.load(g, &l.slot)?;
        match &l.column {
            None => {
                if rows.len() != s.rows.len() {
                    return Err(format!(
                        "{} holds {} rows, a place the game keeps that size; this would make it {}",
                        l.slot,
                        s.rows.len(),
                        rows.len()
                    ));
                }
                s.rows = rows.to_vec();
            }
            Some(col) => {
                let c = s
                    .column(col)
                    .ok_or(format!("{} has no column `{col}`", l.slot))?;
                if rows.len() > room {
                    return Err(format!(
                        "{} has room for {room} rows (its list ends with an end mark the game reads, and the bytes after it are other data); this would make {}",
                        l.named(),
                        rows.len()
                    ));
                }
                let n_old = room * l.width + usize::from(l.end.is_some());
                for k in 0..n_old {
                    if let Some(r) = s.rows.get_mut(l.at + k) {
                        r[c] = 0;
                    }
                }
                let mut k = l.at;
                for r in rows {
                    for &b in r {
                        let row = s
                            .rows
                            .get_mut(k)
                            .ok_or(format!("{} ends before row {k}", l.slot))?;
                        row[c] = b;
                        k += 1;
                    }
                }
                if let Some(e) = l.end {
                    let row = s
                        .rows
                        .get_mut(k)
                        .ok_or(format!("{} ends before its end mark's row {k}", l.slot))?;
                    row[c] = e;
                }
            }
        }
        self.changed.insert(l.slot.clone());
        Ok(())
    }

    /// The lift with the changed slots' files written anew.
    fn lifted(&self) -> Lifted {
        let mut l = self.base.clone();
        for s in &self.changed {
            let d = &self.slots[s];
            l.files.insert(d.file.clone(), d.text());
        }
        for (f, t) in &self.raw {
            l.files.insert(f.clone(), t.clone());
        }
        l
    }
}

// --- The stage's pieces -----------------------------------------------------

/// What a level is made of, by the model: where its pieces' rows are.
#[derive(Debug, Clone)]
struct StageRefs {
    level: String,
    runs: Option<ListRef>,
    run_shapes: Option<ListRef>,
    /// Feature `level.ground_boxes`: a run is one box of x, y, shape.
    boxes: bool,
    rise: i64,
    shape_step: i64,
    ladders: Option<(ListRef, ListRef, i64)>,
    tops: Option<(ListRef, ListRef, i64)>,
    stops: Option<(ListRef, ListRef, i64)>,
    /// The tile grid's slot of a stage made of tiles (`levels.L.grid`),
    /// which no other level reads.
    grid: Option<String>,
}

fn ladder_refs(l: &gp_model::platformer::Ladders) -> (ListRef, ListRef, i64) {
    (
        ListRef::of(
            &l.places,
            l.places_at,
            3,
            Some(i64::from(l.end.unwrap_or(0xFE))),
        ),
        ListRef::of(&l.shapes, l.shapes_at, 4, None),
        i64::from(l.shape_step.max(1)),
    )
}

impl StageRefs {
    fn of(model: &Model, level: &str) -> Result<Self, String> {
        let l = model
            .levels
            .get(level)
            .ok_or(format!("the model has no level `{level}`"))?;
        let p = &l.platformer;
        Ok(StageRefs {
            level: level.to_owned(),
            runs: p
                .ground
                .as_ref()
                .map(|g| ListRef::of(&g.runs, None, if g.boxes { 3 } else { 6 }, None)),
            run_shapes: p
                .ground
                .as_ref()
                .map(|g| ListRef::of(&g.shapes, None, 4, None)),
            boxes: p.ground.as_ref().is_some_and(|g| g.boxes),
            rise: p.ground.as_ref().map_or(1, |g| i64::from(g.rise)),
            shape_step: p
                .ground
                .as_ref()
                .map_or(1, |g| i64::from(g.shape_step.max(1))),
            ladders: p.ladders.as_ref().map(ladder_refs),
            tops: p.ladder_tops.as_ref().map(ladder_refs),
            stops: p.ladder_stops.as_ref().map(ladder_refs),
            grid: l.core.grid.clone(),
        })
    }

    fn is_girder_stage(&self) -> bool {
        self.runs.is_some() && self.ladders.is_some()
    }

    /// A stage made of a grid of tiles, as Moss Hop's: not a girder
    /// stage, and its cells come from a grid slot.
    fn is_tile_stage(&self) -> bool {
        !self.is_girder_stage() && self.grid.is_some()
    }

    /// The slots that are this level's own: its runs and ladders, which
    /// no other level reads (tops and stops may share a table with them).
    fn own_slots(&self) -> BTreeSet<String> {
        let mut s = BTreeSet::new();
        if let Some(r) = &self.runs {
            s.insert(r.slot.clone());
        }
        if let Some((l, _, _)) = &self.ladders {
            s.insert(l.slot.clone());
        }
        if let Some(g) = self.grid.as_ref().filter(|_| self.is_tile_stage()) {
            s.insert(g.clone());
        }
        s
    }

    /// Every slot the level's pieces live in.
    fn slots(&self) -> BTreeSet<String> {
        let mut s = BTreeSet::new();
        for l in [&self.runs, &self.run_shapes].into_iter().flatten() {
            s.insert(l.slot.clone());
        }
        for (a, b, _) in [&self.ladders, &self.tops, &self.stops]
            .into_iter()
            .flatten()
        {
            s.insert(a.slot.clone());
            s.insert(b.slot.clone());
        }
        s
    }
}

/// One run's pieces' boxes, `[x0, y0, x1, y1]`, as the model compiles them
/// (gp_interp::compile's ground boxes), and the first piece whose x
/// wrapped past the screen's edge, if any.
fn run_boxes(
    r: &[i64],
    shapes: &[Vec<i64>],
    refs: &StageRefs,
) -> Result<(Vec<[i64; 4]>, Option<i64>), String> {
    let (x, y, shape, across, count) = match (r, refs.boxes) {
        (&[x, y, shape, ..], true) => (x, y, shape, 0, 1),
        (&[x, y, shape, across, count, ..], false) => (x, y, shape, across, count),
        _ => return Err("a run with too few bytes".into()),
    };
    let s = shapes
        .get(usize::try_from(shape / refs.shape_step).unwrap_or(usize::MAX))
        .filter(|s| s.len() >= 4)
        .ok_or(format!(
            "its shape {shape} isn't one of the {} shapes",
            shapes.len()
        ))?;
    let across = if across >= 128 { across - 256 } else { across };
    let mut out = Vec::new();
    let mut wrapped = None;
    for k in 0..count {
        let raw = x + k * across;
        if wrapped.is_none() && !(0..256).contains(&raw) {
            wrapped = Some(k);
        }
        let px = raw.rem_euclid(256);
        let py = (y - k * refs.rise).rem_euclid(256);
        out.push([px + s[0], py + s[1], px + s[2], py + s[3]]);
    }
    Ok((out, wrapped))
}

/// The spawns of a level, with their place in the model's list and the
/// index among their entity's spawns.
fn spawns(model: &Model, level: &str) -> Vec<(usize, String, usize, i64, i64)> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    model.levels[level]
        .core
        .spawn
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let k = seen.entry(s.entity.clone()).or_default();
            let out = (
                i,
                s.entity.clone(),
                *k,
                s.x.map_or(0, |f| f.raw() >> 16),
                s.y.map_or(0, |f| f.raw() >> 16),
            );
            *k += 1;
            out
        })
        .collect()
}

// --- Operations -------------------------------------------------------------

/// One edit operation, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    MoveRun {
        row: usize,
        set: BTreeMap<String, i64>,
    },
    MoveLadder {
        row: usize,
        set: BTreeMap<String, i64>,
    },
    MoveSpawn {
        entity: String,
        index: usize,
        x: Option<i64>,
        y: Option<i64>,
        dx: i64,
        dy: i64,
    },
    MakeLadder {
        row: usize,
        full: bool,
    },
    SetRow {
        slot: String,
        row: usize,
        values: Vec<i64>,
    },
}

/// The operations, in words, for help and refusals.
pub(crate) const OPS_HELP: &str = "move girder N [x=X] [y=Y] [shape=S] [step=D] [count=C] [dx=..] [dy=..] (girder N is row N of the ground's runs); move ladder N [x=X] [y=Y] [shape=S] [dx=..] [dy=..] (its top or stop moves with it); move spawn ENTITY[#K] [x=X] [y=Y] [dx=..] [dy=..] (the K-th start of that thing, from 0); make ladder N full|broken (a full ladder reaches the girder above and has a top; a broken one has a stop); set row SLOT ROW V,V,.. (one row of any slot the kit names as the stage's data; a table slot of one column takes several values for rows in turn)";

fn number(s: &str) -> Result<i64, String> {
    let (neg, t) = s.strip_prefix('-').map_or((false, s), |t| (true, t));
    let v = match t.strip_prefix("0x").or_else(|| t.strip_prefix('$')) {
        Some(h) => i64::from_str_radix(h, 16),
        None => t.parse(),
    }
    .map_err(|_| format!("`{s}` isn't a number"))?;
    Ok(if neg { -v } else { v })
}

fn pairs(words: &[&str], allowed: &[&str]) -> Result<BTreeMap<String, i64>, String> {
    let mut out = BTreeMap::new();
    for w in words {
        let (k, v) = w.split_once('=').ok_or(format!("`{w}` isn't NAME=VALUE"))?;
        if !allowed.contains(&k) {
            return Err(format!("`{k}` isn't one of {}", allowed.join(", ")));
        }
        out.insert(k.to_owned(), number(v)?);
    }
    if out.is_empty() {
        return Err(format!(
            "it changes nothing: give one of {}",
            allowed.join(", ")
        ));
    }
    Ok(out)
}

fn index(w: Option<&&str>, what: &str) -> Result<usize, String> {
    let w = w.ok_or(format!("which {what}? give its number"))?;
    w.parse()
        .map_err(|_| format!("`{w}` isn't a {what}'s number"))
}

impl Op {
    pub(crate) fn parse(s: &str) -> Result<Op, String> {
        let words: Vec<&str> = s.split_whitespace().collect();
        let fixed = |piece: &str| {
            format!(
                "a stage's {piece} are a list the game keeps at a fixed size (the slot's rows, which its code reads at fixed addresses), so one can't be placed or removed in the kit's format; move one instead (`move {} N ...`), or put a spare one where it does no harm",
                piece.trim_end_matches('s')
            )
        };
        match words.as_slice() {
            ["move", "girder" | "run", rest @ ..] => Ok(Op::MoveRun {
                row: index(rest.first(), "girder")?,
                set: pairs(&rest[1..], &["x", "y", "shape", "step", "count", "dx", "dy"])?,
            }),
            ["move", "ladder", rest @ ..] => Ok(Op::MoveLadder {
                row: index(rest.first(), "ladder")?,
                set: pairs(&rest[1..], &["x", "y", "shape", "dx", "dy"])?,
            }),
            ["move" | "set", "spawn", who, rest @ ..] => {
                let (entity, k) = match who.split_once('#') {
                    Some((e, k)) => (e, index(Some(&k), "start")?),
                    None => (*who, 0),
                };
                let p = pairs(rest, &["x", "y", "dx", "dy"])?;
                Ok(Op::MoveSpawn {
                    entity: entity.to_owned(),
                    index: k,
                    x: p.get("x").copied(),
                    y: p.get("y").copied(),
                    dx: p.get("dx").copied().unwrap_or(0),
                    dy: p.get("dy").copied().unwrap_or(0),
                })
            }
            ["make", "ladder", n, kind] => Ok(Op::MakeLadder {
                row: index(Some(n), "ladder")?,
                full: match *kind {
                    "full" => true,
                    "broken" => false,
                    k => return Err(format!("`{k}`: a ladder is made full or broken")),
                },
            }),
            ["set", "row", slot, row, values] => Ok(Op::SetRow {
                slot: (*slot).to_owned(),
                row: index(Some(row), "row")?,
                values: values
                    .split(',')
                    .map(|v| number(v.trim()))
                    .collect::<Result<_, _>>()?,
            }),
            ["place" | "add", "ladder" | "ladders", ..] | ["remove" | "delete", "ladder" | "ladders", ..] => {
                Err(fixed("ladders"))
            }
            ["place" | "add", "girder" | "girders" | "run", ..]
            | ["remove" | "delete", "girder" | "girders" | "run", ..] => Err(fixed("girders")),
            ["place" | "add" | "remove" | "delete", "top" | "stop", ..] => Err(
                "a ladder's top and stop come and go with it: make ladder N full (a top) or make ladder N broken (a stop)".into(),
            ),
            _ => Err(format!("`{s}` isn't an operation. The operations: {OPS_HELP}")),
        }
    }
}

/// A byte in a row of the kit's format: 0 to 255.
fn byte(v: i64, what: &str) -> Result<i64, String> {
    if (0..=255).contains(&v) {
        Ok(v)
    } else {
        Err(format!(
            "{what} would be {v}, and the stage's data holds bytes, 0 to 255"
        ))
    }
}

/// The slots `set row` may write: the level's own and the kit's data
/// slots, when the kit's rules cover this level.
fn writable_slots(g: &Game, refs: &StageRefs) -> BTreeSet<String> {
    let mut s = refs.slots();
    if kit_covers(g, refs) {
        for d in &g.kit().map(|k| k.data.clone()).unwrap_or_default() {
            if !d.slot.is_empty() {
                s.insert(d.slot.clone());
            }
        }
    }
    s
}

/// Whether the kit's rules are about this level: one of its data slots
/// is one of the level's.
fn kit_covers(g: &Game, refs: &StageRefs) -> bool {
    let own = refs.own_slots();
    g.kit()
        .is_some_and(|k| k.data.iter().any(|d| own.contains(&d.slot)))
}

/// A ladder's top or stop, by its row in that list: the one at the
/// ladder's x whose y is its own (a top) or above it (a stop).
fn partner(list: &[Vec<i64>], lad: &[i64], top: bool) -> Option<usize> {
    list.iter().position(|r| {
        r.len() >= 2
            && r[0] == lad[0]
            && if top {
                r[1] == lad[1]
            } else {
                (1..=40).contains(&(lad[1] - r[1]))
            }
    })
}

/// The room a list has: its rows in the original's lift (the game's own
/// list ends there, and other data follows its end mark).
fn room(original: &mut Data, g: &Game, l: &ListRef, now: usize) -> usize {
    original.read(g.at(), l).map_or(now, |r| r.len().max(now))
}

/// Runs `ops` on the data and model; every error names the operation.
fn apply(
    g: &Game,
    refs: &StageRefs,
    data: &mut Data,
    model: &mut Model,
    ops: &[Op],
    original: &mut Data,
) -> Result<(), String> {
    let writable = writable_slots(g, refs);
    for (n, op) in ops.iter().enumerate() {
        let at = |e: String| format!("operation {} ({}): {e}", n + 1, describe(op));
        apply_one(g, refs, data, model, op, &writable, original).map_err(at)?;
    }
    Ok(())
}

fn describe(op: &Op) -> String {
    let kv = |m: &BTreeMap<String, i64>| {
        m.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    match op {
        Op::MoveRun { row, set } => format!("move girder {row} {}", kv(set)),
        Op::MoveLadder { row, set } => format!("move ladder {row} {}", kv(set)),
        Op::MoveSpawn { entity, index, .. } => format!("move spawn {entity}#{index}"),
        Op::MakeLadder { row, full } => {
            format!(
                "make ladder {row} {}",
                if *full { "full" } else { "broken" }
            )
        }
        Op::SetRow { slot, row, .. } => format!("set row {slot} {row}"),
    }
}

fn apply_one(
    g: &Game,
    refs: &StageRefs,
    data: &mut Data,
    model: &mut Model,
    op: &Op,
    writable: &BTreeSet<String>,
    original: &mut Data,
) -> Result<(), String> {
    let girder_stage = || {
        if let Some(why) = arcade_level(g, &refs.level) {
            return Err(why);
        }
        if refs.is_girder_stage() {
            Ok(())
        } else {
            Err(format!(
                "level {} has no ground runs and ladders in its model, so it has no girders or ladders to move",
                refs.level
            ))
        }
    };
    match op {
        Op::MoveRun { row, set } => {
            girder_stage()?;
            let l = refs.runs.clone().expect("a girder stage");
            let mut rows = data.read(g.at(), &l)?;
            let n = rows.len();
            let r = rows.get_mut(*row).ok_or(format!(
                "there are girder rows 0 to {}",
                n.saturating_sub(1)
            ))?;
            let names = ["x", "y", "shape", "step", "count"];
            for (k, v) in set {
                match k.as_str() {
                    "dx" => r[0] = byte(r[0] + v, "x")?,
                    "dy" => r[1] = byte(r[1] + v, "y")?,
                    "step" | "count" if refs.boxes => {
                        return Err(
                            "this level's ground is one box a row, with no step or count".into(),
                        );
                    }
                    "step" => r[3] = byte(v.rem_euclid(256), "the step")?,
                    "shape" => r[2] = byte(v * refs.shape_step, "the shape")?,
                    name => {
                        let i = names.iter().position(|x| *x == name).expect("checked");
                        r[i] = byte(*v, name)?;
                    }
                }
            }
            data.write(g.at(), &l, &rows, n)
        }
        Op::MoveLadder { row, set } => {
            girder_stage()?;
            let (l, _, step) = refs.ladders.clone().expect("a girder stage");
            let mut rows = data.read(g.at(), &l)?;
            let n = rows.len();
            let old = rows
                .get(*row)
                .cloned()
                .ok_or(format!("there are ladders 0 to {}", n.saturating_sub(1)))?;
            // Moved across only, it is fitted to its girders where it lands,
            // as the game's own are: a full ladder from the girder above's
            // row down to its own girder's (a shape of that height), a
            // broken one keeping its height with its foot on its girder.
            let across_only = set.keys().all(|k| k == "x" || k == "dx");
            let fit = if across_only {
                let spec = compile(g, data, model)?;
                let lay = stage::layout(&spec, level_index(&spec, &refs.level)?);
                lay.ladders
                    .get(*row)
                    .map(|x| (x.lower, x.upper, reaches(&lay, x), lay.girders.clone()))
            } else {
                None
            };
            let r = &mut rows[*row];
            for (k, v) in set {
                match k.as_str() {
                    "x" => r[0] = byte(*v, "x")?,
                    "y" => r[1] = byte(*v, "y")?,
                    "dx" => r[0] = byte(r[0] + v, "x")?,
                    "dy" => r[1] = byte(r[1] + v, "y")?,
                    "shape" => r[2] = byte(v * step, "the shape")?,
                    _ => unreachable!("checked"),
                }
            }
            if let Some((Some(lo), up, full, girders)) = fit {
                let (_, sl, _) = refs.ladders.clone().expect("a girder stage");
                let shapes = data.read(g.at(), &sl)?;
                let height = |s: &Vec<i64>| s[3] - s[1];
                let w = shapes
                    .get(usize::try_from(r[2] / step).unwrap_or(0))
                    .map_or(8, |s| s[2] - s[0]);
                let cx = r[0] + w / 2;
                let low = girders[lo].surface(cx).ok_or(format!(
                    "ladder {row} stands on girder {lo}, which doesn't reach x {cx}: move the girder first, or give y= and shape="
                ))?;
                match (full, up.and_then(|u| girders[u].surface(cx))) {
                    (true, Some(high)) => {
                        let want = low - high;
                        let k = shapes
                            .iter()
                            .position(|s| height(s) == want)
                            .or_else(|| {
                                shapes
                                    .iter()
                                    .position(|s| (0..=2).contains(&(height(s) - want)))
                            })
                            .ok_or(format!(
                                "at x {} ladder {row} would run {want} rows from girder {lo} up to the next, and no ladder shape is that tall (they are {:?}): pick another x, or set a shape with set row {} K 0,0,8,{want}",
                                r[0],
                                shapes.iter().map(height).collect::<Vec<_>>(),
                                sl.slot
                            ))?;
                        r[1] = byte(high, "y")?;
                        r[2] = byte(k as i64 * step, "the shape")?;
                    }
                    (true, None) => {
                        return Err(format!(
                            "ladder {row} is full, and at x {} there's no girder above girder {lo} for it to reach",
                            r[0]
                        ));
                    }
                    (false, _) => {
                        let h = shapes
                            .get(usize::try_from(r[2] / step).unwrap_or(0))
                            .map_or(0, height);
                        r[1] = byte(low - h, "y")?;
                    }
                }
            }
            let (dx, dy) = (r[0] - old[0], r[1] - old[1]);
            data.write(g.at(), &l, &rows, n)?;
            // Its top or stop moves with it: a top is the ladder's own
            // place; a stop stands on the girder above, where the game's
            // own stops all are, or moves as the ladder did when no girder
            // is above it now.
            for (lists, top) in [(&refs.tops, true), (&refs.stops, false)] {
                if let Some((tl, _, _)) = lists {
                    let mut t = data.read(g.at(), tl)?;
                    if let Some(i) = partner(&t, &old, top) {
                        t[i][0] = byte(t[i][0] + dx, "its top's or stop's x")?;
                        t[i][1] = byte(t[i][1] + dy, "its top's or stop's y")?;
                        if !top {
                            let spec = compile(g, data, model)?;
                            let lay = stage::layout(&spec, level_index(&spec, &refs.level)?);
                            if let Some(s) = lay
                                .ladders
                                .get(*row)
                                .and_then(|x| x.upper.and_then(|u| lay.girders[u].surface(x.cx())))
                            {
                                t[i][1] = byte(s, "its stop's y")?;
                            }
                        }
                        let room = room(original, g, tl, t.len());
                        data.write(g.at(), tl, &t, room)?;
                    }
                }
            }
            Ok(())
        }
        Op::MakeLadder { row, full } => {
            girder_stage()?;
            let (Some((tl, _, _)), Some((sl, _, _))) = (&refs.tops, &refs.stops) else {
                return Err(format!(
                    "level {} keeps no ladder tops and stops, so its ladders are full or broken by their boxes alone: move ladder N y=.. shape=..",
                    refs.level
                ));
            };
            let (l, _, _) = refs.ladders.clone().expect("a girder stage");
            let lad = data
                .read(g.at(), &l)?
                .get(*row)
                .cloned()
                .ok_or(format!("there's no ladder {row}"))?;
            let mut tops = data.read(g.at(), tl)?;
            let mut stops = data.read(g.at(), sl)?;
            let t = partner(&tops, &lad, true);
            let s = partner(&stops, &lad, false);
            // The girder above at the ladder's middle: where a top stands
            // and where a stop's box starts.
            let spec = compile(g, data, model)?;
            let li = level_index(&spec, &refs.level)?;
            let lay = stage::layout(&spec, li);
            let upper = lay
                .ladders
                .get(*row)
                .and_then(|x| x.upper.and_then(|u| lay.girders[u].surface(x.cx())));
            if *full {
                if t.is_some() {
                    return Err(format!("ladder {row} is full already"));
                }
                if let Some(i) = s {
                    stops.remove(i);
                }
                let room_t = room(original, g, tl, tops.len());
                if tops.len() >= room_t {
                    return Err(format!(
                        "the game's list of ladder tops is full ({room_t}, the kit's limit): make another ladder broken first"
                    ));
                }
                tops.push(vec![lad[0], lad[1], 0]);
                let room_s = room(original, g, sl, stops.len());
                data.write(g.at(), sl, &stops, room_s)?;
                data.write(g.at(), tl, &tops, room_t)?;
            } else {
                if s.is_some() {
                    return Err(format!("ladder {row} is broken already"));
                }
                let up = upper.ok_or(format!("ladder {row} has no girder above it to stop at"))?;
                if let Some(i) = t {
                    tops.remove(i);
                }
                let room_s = room(original, g, sl, stops.len());
                if stops.len() >= room_s {
                    return Err(format!(
                        "the game's list of broken ladders' stops is full ({room_s}, the kit's limit): make another ladder full first"
                    ));
                }
                stops.push(vec![lad[0], byte(up, "the stop's y")?, 0]);
                let room_t = room(original, g, tl, tops.len());
                data.write(g.at(), tl, &tops, room_t)?;
                data.write(g.at(), sl, &stops, room_s)?;
            }
            Ok(())
        }
        Op::MoveSpawn {
            entity,
            index,
            x,
            y,
            dx,
            dy,
        } => {
            let l = model.levels.get_mut(&refs.level).ok_or("no such level")?;
            let found = l
                .core
                .spawn
                .iter_mut()
                .filter(|s| s.entity == *entity)
                .nth(*index);
            let Some(s) = found else {
                let have: BTreeSet<&str> = spawns(&g.model, &refs.level)
                    .into_iter()
                    .map(|_| "")
                    .collect();
                let _ = have;
                let names: BTreeSet<String> = g.model.levels[&refs.level]
                    .core
                    .spawn
                    .iter()
                    .map(|s| s.entity.clone())
                    .collect();
                return Err(format!(
                    "level {} has no start {entity}#{index}; its things are {}",
                    refs.level,
                    names.into_iter().collect::<Vec<_>>().join(", ")
                ));
            };
            let int = |f: Option<Fixed>| f.map_or(0, |f| f.raw() >> 16);
            let nx = x.unwrap_or(int(s.x)) + dx;
            let ny = y.unwrap_or(int(s.y)) + dy;
            s.x = Some(Fixed::from_int(nx)?);
            s.y = Some(Fixed::from_int(ny)?);
            Ok(())
        }
        Op::SetRow { slot, row, values } => {
            if !writable.contains(slot) {
                return Err(format!(
                    "`{slot}` isn't one of this level's data slots; these are: {}",
                    writable.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            for v in values {
                byte(*v, &format!("a value of row {row}"))?;
            }
            let s = data.load(g.at(), slot)?;
            for v in values {
                s.holds(*v, &format!("a value of row {row}"))?;
            }
            let n = s.rows.len();
            let one_col = matches!(&s.format, Format::Table(c) if c.len() == 1);
            if one_col {
                if row + values.len() > n {
                    return Err(format!("{slot} has rows 0 to {}", n - 1));
                }
                for (k, v) in values.iter().enumerate() {
                    s.rows[row + k][0] = *v;
                }
            } else {
                let r = s
                    .rows
                    .get_mut(*row)
                    .ok_or(format!("{slot} has rows 0 to {}", n.saturating_sub(1)))?;
                if values.len() != r.len() {
                    return Err(format!(
                        "a row of {slot} is {} values, not {}",
                        r.len(),
                        values.len()
                    ));
                }
                *r = values.clone();
            }
            data.changed.insert(slot.clone());
            Ok(())
        }
    }
}

/// The whole of a level's data, as `gp level show` gives it under `data`
/// and `gp level edit --from` takes it back.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Whole {
    /// Each slot's rows (a table slot's rows as their columns' values).
    #[serde(default)]
    pub slots: BTreeMap<String, Vec<Vec<i64>>>,
    /// Each start, by its place in the level's list.
    #[serde(default)]
    pub spawns: Vec<WholeSpawn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WholeSpawn {
    pub spawn: usize,
    pub entity: String,
    pub x: i64,
    pub y: i64,
}

fn apply_whole(
    g: &Game,
    refs: &StageRefs,
    data: &mut Data,
    model: &mut Model,
    w: &Whole,
) -> Result<(), String> {
    let writable = writable_slots(g, refs);
    for (slot, rows) in &w.slots {
        if !writable.contains(slot) {
            return Err(format!(
                "`{slot}` isn't one of this level's data slots; these are: {}",
                writable.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        let s = data.load(g.at(), slot)?;
        if rows.len() != s.rows.len() {
            return Err(format!(
                "{slot} holds {} rows in the kit's format, a place the game keeps that size; the replacement has {}",
                s.rows.len(),
                rows.len()
            ));
        }
        for (i, (new, old)) in rows.iter().zip(&s.rows).enumerate() {
            if new.len() != old.len() {
                return Err(format!(
                    "{slot} row {i} is {} values in the kit's format, not {}",
                    old.len(),
                    new.len()
                ));
            }
            for v in new {
                byte(*v, &format!("{slot} row {i}"))?;
                s.holds(*v, &format!("{slot} row {i}"))?;
            }
        }
        if *rows != s.rows {
            s.rows = rows.clone();
            data.changed.insert(slot.clone());
        }
    }
    let l = model.levels.get_mut(&refs.level).ok_or("no such level")?;
    for sp in &w.spawns {
        let s = l
            .core
            .spawn
            .get_mut(sp.spawn)
            .ok_or(format!("level {} has no start {}", refs.level, sp.spawn))?;
        if s.entity != sp.entity {
            return Err(format!(
                "start {} is {}, not {}: a replacement moves starts, it doesn't change what they are",
                sp.spawn, s.entity, sp.entity
            ));
        }
        s.x = Some(Fixed::from_int(sp.x)?);
        s.y = Some(Fixed::from_int(sp.y)?);
    }
    Ok(())
}

// --- The stage's picture ---------------------------------------------------

/// The stage's picture after an edit: its bytes in the kit's format, what
/// it took before, and the words for the person.
struct Redrawn {
    bytes: usize,
    was: usize,
    said: String,
}

/// The bytes one update takes in a PPU string: two of address, one of
/// count and flags, then its data (one byte for a repeat).
fn update_bytes(n: usize, repeat: bool) -> usize {
    3 + if repeat { 1 } else { n }
}

/// The most a stage picture may take: what the kit's limit gives the
/// level's entry with every other stage's picture dropped, when the kit
/// says so in words ("with 75m and 100m dropped, 894"); else the entry's
/// own bytes.
fn picture_room(g: &Game, was: usize) -> (usize, usize) {
    let most = g
        .kit()
        .and_then(|k| {
            k.limit
                .iter()
                .find(|l| l.what.to_lowercase().contains("picture"))
                .map(|l| numbers_in(&l.limit))
        })
        .and_then(|n| n.last().copied())
        .map_or(was, |n| (n as usize).max(was));
    (was, most)
}

/// Redraws the play tiles of the level's picture (core
/// `level.layout_ppu_strings`) where the edit moved girders or ladders:
/// the cells the pieces covered before are cleared, and each piece is
/// drawn again with the tiles the original's picture uses for that kind
/// and standing row, learned from it. Updates that aren't the name
/// table's cells (palettes, attributes) are kept as they are. None when
/// the level has no such picture or no piece moved.
fn redraw(
    g: &Game,
    refs: &StageRefs,
    data: &mut Data,
    model: &Model,
) -> Result<Option<Redrawn>, String> {
    let moved = data.changed.iter().any(|s| refs.slots().contains(s));
    if !moved {
        return Ok(None);
    }
    let Some(layout) = model.levels[&refs.level].core.layout.clone() else {
        return Ok(None);
    };
    let Ok(Asset::PpuStrings(p)) = slot_asset(model, &g.map, &layout.slot) else {
        return Ok(None);
    };
    let level = &model.levels[&refs.level].core;
    let kind_of = |t: u8| {
        level
            .tile_kinds
            .get(&t.to_string())
            .map(|k| format!("{k:?}").to_lowercase())
    };
    let surface_of = |t: u8| level.tile_surfaces.get(&t.to_string()).copied();
    let file = format!("{}.toml", p.name);
    let text = data
        .raw
        .get(&file)
        .cloned()
        .or_else(|| data.base.files.get(&file).cloned())
        .ok_or(format!("the lift has no {file}"))?;
    let grid = |t: &str| -> Result<Vec<u8>, String> {
        let mut cells = vec![0u8; 32 * 30];
        for w in p.writes(t, &layout.entry)? {
            let mut ppu = w.ppu;
            for b in w.bytes {
                if (0x2000..0x23C0).contains(&ppu) {
                    cells[usize::from(ppu - 0x2000)] = b;
                }
                ppu = ppu.wrapping_add(if w.vertical { 32 } else { 1 });
            }
        }
        Ok(cells)
    };
    // What was there: the copy before this edit, and the original's
    // picture with its pieces, which the tiles are learned from.
    let before = gp_interp::compile::compile(&g.model, &g.lifted).map_err(|e| e.to_string())?;
    let orig_spec =
        gp_interp::compile::compile(&g.model, &g.original).map_err(|e| e.to_string())?;
    let after = compile(g, data, model)?;
    let li = level_index(&after, &refs.level)?;
    let orig_text = g
        .original
        .files
        .get(&file)
        .ok_or(format!("the original lift has no {file}"))?;
    let orig = grid(orig_text)?;
    let mut cells = grid(&text)?;
    let at = |x: i64, y: i64| -> Option<usize> {
        (0..32).contains(&(x / 8)).then_some(())?;
        (0..30).contains(&(y / 8)).then_some(())?;
        (x >= 0 && y >= 0).then_some((y / 8 * 32 + x / 8) as usize)
    };
    // Learn: each standing row's top tile, the tile under it, and the
    // tile where a ladder crosses a girder at that row.
    let mut tops: BTreeMap<i64, BTreeMap<u8, usize>> = BTreeMap::new();
    let mut unders: BTreeMap<u8, BTreeMap<u8, usize>> = BTreeMap::new();
    let mut crossing: BTreeMap<i64, BTreeMap<u8, usize>> = BTreeMap::new();
    let mut ladder: BTreeMap<u8, usize> = BTreeMap::new();
    let ol = &orig_spec.levels[li];
    for (i, &t) in orig.iter().enumerate() {
        let (x, y) = ((i % 32) as i64 * 8, (i / 32) as i64 * 8);
        let k = kind_of(t);
        let in_ladder = ol.ladders.iter().any(|b| {
            b[0] / 8 <= x / 8 && x / 8 <= (b[2] - 1) / 8 && b[1] / 8 <= y / 8 && y / 8 <= b[3] / 8
        });
        match (k.as_deref(), surface_of(t)) {
            (Some("top"), Some(sf)) => {
                *tops.entry(i64::from(sf)).or_default().entry(t).or_default() += 1;
                if let Some(&u) = orig.get(i + 32) {
                    *unders.entry(t).or_default().entry(u).or_default() += 1;
                }
            }
            (Some("ladder"), Some(sf)) => {
                *crossing
                    .entry(i64::from(sf))
                    .or_default()
                    .entry(t)
                    .or_default() += 1;
            }
            (Some("ladder"), None) if in_ladder => {
                *ladder.entry(t).or_default() += 1;
            }
            _ => {}
        }
    }
    let most = |m: &BTreeMap<u8, usize>| m.iter().max_by_key(|(_, n)| **n).map(|(t, _)| *t);
    let plain_ladder =
        most(&ladder).ok_or("the original's picture has no ladder tile to learn from")?;
    let top_tile = |sf: i64| tops.get(&sf).and_then(most);
    // The cells the pieces covered before: cleared, with the girders'
    // lower halves.
    let lb = &before.levels[li];
    let mut clear = BTreeSet::new();
    for b in &lb.ground {
        let mut x = b[0];
        while x <= b[2].min(255) {
            if let Some(c) = at(x, b[1]) {
                clear.insert(c);
                if unders.contains_key(&cells[c]) {
                    clear.insert(c + 32);
                }
            }
            x += 8;
        }
        if let Some(c) = at(b[2].min(255), b[1]) {
            clear.insert(c);
        }
    }
    for b in &lb.ladders {
        let cx = (b[0] + b[2]) / 2;
        let mut y = b[1];
        while y <= b[3] {
            if let Some(c) = at(cx, y) {
                clear.insert(c);
            }
            y += 8;
        }
        if let Some(c) = at(cx, b[3]) {
            clear.insert(c);
        }
    }
    for c in &clear {
        if let Some(t) = cells.get_mut(*c) {
            *t = 0;
        }
    }
    // Draw the pieces as they are now: each girder piece's cells with the
    // top tile of its standing row and the tile under it, then each
    // ladder's column, crossing a girder with the crossing tile.
    let la = &after.levels[li];
    let mut standing: BTreeMap<usize, i64> = BTreeMap::new();
    for b in &la.ground {
        let sf = b[1].rem_euclid(8);
        let Some(t) = top_tile(sf) else {
            return Err(format!(
                "the original's picture has no girder tile that stands at row {sf} of its cell, so a girder at row {} can't be drawn: move it a pixel up or down",
                b[1]
            ));
        };
        let mut x = b[0];
        while x <= b[2].min(255) {
            if let Some(c) = at(x, b[1]) {
                cells[c] = t;
                standing.insert(c, sf);
                if let (Some(u), Some(cell)) =
                    (unders.get(&t).and_then(most), cells.get_mut(c + 32))
                    && *cell == 0
                {
                    *cell = u;
                }
            }
            x += 8;
        }
    }
    for b in &la.ladders {
        let cx = (b[0] + b[2]) / 2;
        let mut y = b[1] - b[1].rem_euclid(8);
        while y <= b[3] {
            if let Some(c) = at(cx, y) {
                cells[c] = match standing.get(&c) {
                    Some(sf) => crossing.get(sf).and_then(most).unwrap_or(cells[c]),
                    None => plain_ladder,
                };
            }
            y += 8;
        }
    }
    // The string again: its updates that aren't name table cells as they
    // were, then the cells, a run a row.
    let (head, kept, tail) = split_updates(&text, &layout.entry)?;
    let mut lines = kept.clone();
    let mut bytes: usize = kept_bytes(p, &text, &layout.entry)?;
    for row in 0..30 {
        let r = &cells[row * 32..row * 32 + 32];
        let mut x = 0;
        while x < 32 {
            if r[x] == 0 {
                x += 1;
                continue;
            }
            // A run of drawn cells, at most 63 bytes. An empty cell is
            // never written: one the string doesn't draw shows the screen
            // as cleared, where a written 0 would show tile 0.
            let mut last = x;
            while last + 1 < 32 && r[last + 1] != 0 && last + 1 - x < 63 {
                last += 1;
            }
            let run = &r[x..=last];
            let ppu = 0x2000 + row * 32 + x;
            if run.len() >= 3 && run.iter().all(|t| *t == run[0]) {
                lines.push(format!(
                    "  {{ ppu = 0x{ppu:04X}, repeat = {}, tile = 0x{:02X} }},",
                    run.len(),
                    run[0]
                ));
                bytes += update_bytes(run.len(), true);
            } else {
                let items: Vec<String> = run.iter().map(|t| format!("0x{t:02X}")).collect();
                lines.push(format!(
                    "  {{ ppu = 0x{ppu:04X}, tiles = [{}] }},",
                    items.join(", ")
                ));
                bytes += update_bytes(run.len(), false);
            }
            x = last + 1;
        }
    }
    let was =
        kept_bytes(p, orig_text, &layout.entry)? + nametable_bytes(p, orig_text, &layout.entry)?;
    let (own, room) = picture_room(g, was);
    if bytes > room {
        return Err(format!(
            "the kit's limit \"Space for the stage's picture\": redrawn for this layout, the picture would take {bytes} bytes, past the {room} the level's picture can have at most; keep more of the pieces in rows the picture draws as one run"
        ));
    }
    let new_text = format!("{head}{}\n{tail}", lines.join("\n"));
    data.raw.insert(file, new_text);
    let said = if bytes > own {
        format!(
            "the stage's picture (its play tiles: a climb holds on ladder tiles and ends on a girder's) was redrawn for the moved pieces: {bytes} bytes, past the {own} its picture took in the ROM, which fits only with other stages' pictures dropped (the kit's limit); on the model nothing else changes"
        )
    } else {
        format!(
            "the stage's picture (its play tiles: a climb holds on ladder tiles and ends on a girder's) was redrawn for the moved pieces: {bytes} bytes of the {own} it may take"
        )
    };
    Ok(Some(Redrawn { bytes, was, said }))
}

/// The text of a picture's file split around one string's updates: what
/// comes before its first update, those of its updates that aren't name
/// table cells (each a line, as written), and what comes after its last.
fn split_updates(text: &str, entry: &str) -> Result<(String, Vec<String>, String), String> {
    let name_line = format!("name = \"{entry}\"");
    let start = text
        .find(&name_line)
        .ok_or(format!("the picture has no string {entry}"))?;
    let open = text[start..]
        .find("updates = [\n")
        .map(|i| start + i + "updates = [\n".len())
        .ok_or(format!("string {entry} has no updates"))?;
    let close = text[open..]
        .find("\n]")
        .map(|i| open + i)
        .ok_or(format!("string {entry}'s updates don't end"))?;
    let kept = text[open..close]
        .lines()
        .filter(|l| {
            let Some(at) = l.find("ppu = 0x") else {
                return true;
            };
            let hex = &l[at + 8..at + 12];
            u16::from_str_radix(hex, 16).is_ok_and(|a| !(0x2000..0x23C0).contains(&a))
        })
        .map(str::to_owned)
        .collect();
    Ok((text[..open].to_owned(), kept, text[close + 1..].to_owned()))
}

/// The bytes a string's updates take, as its lines say: those of the
/// name table's cells, or the others. A repeat is one byte of data; an
/// embedded asset's update its embed's size.
fn string_bytes(
    p: &gp_assets::ppu::PpuStrings,
    text: &str,
    entry: &str,
    cells: bool,
) -> Result<usize, String> {
    let name_line = format!("name = \"{entry}\"");
    let start = text
        .find(&name_line)
        .ok_or(format!("the picture has no string {entry}"))?;
    let open = text[start..]
        .find("updates = [\n")
        .map(|i| start + i)
        .ok_or("no updates")?;
    let close = text[open..].find("\n]").map(|i| open + i).ok_or("no end")?;
    let embed = p.embeds.first().map_or(0, |e| e.size as usize);
    let mut n = 0;
    for l in text[open..close].lines() {
        let Some(at) = l.find("ppu = 0x") else {
            continue;
        };
        let a = u16::from_str_radix(&l[at + 8..at + 12], 16).unwrap_or(0);
        if (0x2000..0x23C0).contains(&a) != cells {
            continue;
        }
        n += if l.contains("repeat =") {
            4
        } else if let Some(t) = l.find("tiles = [") {
            3 + l[t..].matches("0x").count()
        } else if l.contains("data = ") {
            3 + embed
        } else if let Some(t) = l.find("text = \"") {
            3 + l[t + 8..]
                .trim_end_matches([' ', '}', ','])
                .len()
                .saturating_sub(1)
        } else {
            3
        };
    }
    Ok(n)
}

fn kept_bytes(p: &gp_assets::ppu::PpuStrings, text: &str, entry: &str) -> Result<usize, String> {
    string_bytes(p, text, entry, false)
}

fn nametable_bytes(
    p: &gp_assets::ppu::PpuStrings,
    text: &str,
    entry: &str,
) -> Result<usize, String> {
    string_bytes(p, text, entry, true)
}

// --- Compiling and checking -------------------------------------------------

fn compile(_g: &Game, data: &Data, model: &Model) -> Result<Spec, String> {
    gp_interp::compile::compile(model, &data.lifted())
        .map_err(|e| format!("the level no longer compiles: {e}"))
}

fn level_index(spec: &Spec, level: &str) -> Result<usize, String> {
    spec.levels
        .iter()
        .position(|l| l.name == level)
        .ok_or(format!("the model has no level `{level}`"))
}

/// One rule a stage broke, or kept by hand.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Finding {
    /// The rule: the kit's `what` where it has one.
    pub rule: String,
    /// What and where, in words.
    pub said: String,
}

/// The kit's code rule whose words hold every one of `words`, by its
/// `what`; else `fallback`.
fn kit_rule(g: &Game, words: &[&str], fallback: &str) -> String {
    g.kit()
        .and_then(|k| {
            k.code.iter().chain(&k.data).find(|p| {
                let t = format!("{} {}", p.what, p.rule).to_lowercase();
                words.iter().all(|w| t.contains(w))
            })
        })
        .map_or_else(
            || fallback.to_owned(),
            |p| format!("the kit's rule \"{}\"", p.what),
        )
}

/// The pieces of a compiled girder stage, numbered as the edits name them.
struct Pieces {
    /// Each run's row and its first piece's box.
    runs: Vec<(usize, [i64; 4])>,
    /// Each ladder's box and whether it reaches the girder above.
    ladders: Vec<([i64; 4], bool)>,
}

fn pieces(spec: &Spec, li: usize, lay: &Layout, runs: &[Vec<[i64; 4]>]) -> Pieces {
    let l = &spec.levels[li];
    Pieces {
        runs: runs
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.first().map(|b| (i, *b)))
            .collect(),
        ladders: l
            .ladders
            .iter()
            .zip(&lay.ladders)
            .map(|(b, x)| (*b, reaches(lay, x)))
            .collect(),
    }
}

/// Whether a ladder's box runs from its girder up to the next one.
fn reaches(lay: &Layout, l: &stage::Ladder) -> bool {
    match (l.lower, l.upper) {
        (Some(_), Some(u)) => lay.girders[u]
            .surface(l.cx())
            .is_some_and(|s| (l.bx[1] - s).abs() <= MEETS),
        _ => false,
    }
}

/// The girder each run's first piece is on.
fn run_girders(lay: &Layout, runs: &[Vec<[i64; 4]>]) -> Vec<Option<usize>> {
    runs.iter()
        .map(|r| {
            r.first().and_then(|b| {
                lay.girders
                    .iter()
                    .position(|g| g.pieces.iter().any(|p| p[0] == b[0] && p[1] == b[1]))
            })
        })
        .collect()
}

/// Everything a stage is checked against before it is written, and after.
struct Checked {
    spec: Spec,
    li: usize,
    lay: Layout,
    runs: Vec<Vec<[i64; 4]>>,
    broke: Vec<Finding>,
    /// Rules the kit states that these checks can't hold a stage to.
    by_hand: Vec<String>,
}

fn check(
    g: &Game,
    refs: &StageRefs,
    data: &mut Data,
    model: &Model,
    original: &mut Data,
) -> Result<Checked, String> {
    let spec = compile(g, data, model)?;
    let li = level_index(&spec, &refs.level)?;
    let lay = stage::layout(&spec, li);
    let l = &spec.levels[li];
    let mut broke = Vec::new();
    let mut by_hand = Vec::new();
    let (w, h) = (l.size[0] * l.tile_size, l.size[1] * l.tile_size);
    let mut runs = Vec::new();
    if let (Some(rl), Some(sl)) = (&refs.runs, &refs.run_shapes) {
        let rows = data.read(g.at(), rl)?;
        let shapes = data.read(g.at(), sl)?;
        for (i, r) in rows.iter().enumerate() {
            match run_boxes(r, &shapes, refs) {
                Ok((boxes, wrapped)) => {
                    if let Some(k) = wrapped {
                        broke.push(Finding {
                            rule: "the screen".into(),
                            said: format!(
                                "girder row {i} runs off the screen's edge at its piece {k} (x wraps past 0 or 255)"
                            ),
                        });
                    }
                    for b in &boxes {
                        // A piece may run past the right edge, as the
                        // original's bottom girder's last does; it starts on
                        // the screen.
                        if b[0] >= w || b[1] >= h {
                            broke.push(Finding {
                                rule: "the screen".into(),
                                said: format!(
                                    "girder row {i} has a piece starting at x {}, y {} (it runs to x {}): off the level's {w} by {h} pixels",
                                    b[0], b[1], b[2]
                                ),
                            });
                            break;
                        }
                    }
                    runs.push(boxes);
                }
                Err(e) => {
                    broke.push(Finding {
                        rule: "the format".into(),
                        said: format!("girder row {i}: {e}"),
                    });
                    runs.push(Vec::new());
                }
            }
        }
    }
    if refs.is_girder_stage() {
        // The runs make the girders the code reads: as many, each row on
        // the same girder as the original's rows were.
        let ospec = compile(g, original, &g.model)?;
        let olay = stage::layout(&ospec, li);
        let mut oruns = Vec::new();
        if let (Some(rl), Some(sl)) = (&refs.runs, &refs.run_shapes) {
            let rows = original.read(g.at(), rl)?;
            let shapes = original.read(g.at(), sl)?;
            for r in &rows {
                oruns.push(run_boxes(r, &shapes, refs).map(|b| b.0).unwrap_or_default());
            }
        }
        let (now, was) = (run_girders(&lay, &runs), run_girders(&olay, &oruns));
        let rule = kit_rule(g, &["row", "run"], "the girder rows");
        if lay.girders.len() != olay.girders.len() {
            broke.push(Finding {
                rule: rule.clone(),
                said: format!(
                    "the runs now make {} girders where the game's code reads {}: two runs that touch end to end (2 pixels or less apart in height) make one girder, and the original's rows {}",
                    lay.girders.len(),
                    olay.girders.len(),
                    group_words(&was)
                ),
            });
        } else {
            for (i, (a, b)) in now.iter().zip(&was).enumerate() {
                let same_as = |v: &[Option<usize>], x: Option<usize>| {
                    v.iter()
                        .enumerate()
                        .filter(|(j, y)| *j != i && **y == x)
                        .map(|(j, _)| j)
                        .collect::<Vec<_>>()
                };
                if same_as(&now, *a) != same_as(&was, *b) {
                    broke.push(Finding {
                        rule: rule.clone(),
                        said: format!(
                            "girder row {i} is now on the same girder as rows {:?}, where the code reads it with rows {:?}",
                            same_as(&now, *a),
                            same_as(&was, *b)
                        ),
                    });
                    break;
                }
            }
        }
        // Ladders on the screen.
        for (i, b) in l.ladders.iter().enumerate() {
            if b[0] < 0 || b[2] >= w || b[1] < 0 || b[3] >= h {
                broke.push(Finding {
                    rule: "the screen".into(),
                    said: format!(
                        "ladder {i}'s box (x {} to {}, y {} to {}) is past the level's {w} by {h} pixels",
                        b[0], b[2], b[1], b[3]
                    ),
                });
            }
        }
        // Ladders the code climbs, by the model's numbers: the kit's
        // rule for the stages it covers (one game's barrels and fires
        // climb its first stage's ladders by these; another stage's code reads its own
        // tables, which these numbers aren't).
        let held = kit_covers(g, refs) || g.kit().is_none();
        for (entity, name, x, foot, size) in code_ladders(model, &spec).into_iter().filter(|_| held)
        {
            let (cx, bottom) = (x + size[0] / 2, foot + size[1]);
            let there = l
                .ladders
                .iter()
                .any(|b| ((b[0] + b[2]) / 2 - cx).abs() <= 2 && (b[3] - bottom).abs() <= 3);
            if !there {
                broke.push(Finding {
                    rule: kit_rule(g, &[&entity, "ladder"], "a ladder the code climbs"),
                    said: format!(
                        "the {entity}'s code climbs a ladder at x {x} from row {foot} (the model's {entity} {name}): this stage needs a ladder whose middle is at x {cx} and whose foot is at row {bottom} there, and none is"
                    ),
                });
            }
        }
        // The hero starts on ground.
        if let Some(p) = l
            .spawns
            .iter()
            .find(|s| spec.kinds[s.kind].role == gp_sim::spec::Role::Player)
        {
            let k = &spec.kinds[p.kind];
            let cx = p.x.floor() + k.size[0].floor() / 2;
            let feet = p.y.floor() + k.tile_hitbox.y.floor() + k.tile_hitbox.h.floor();
            let on = lay
                .girders
                .iter()
                .filter_map(|gd| gd.surface(cx))
                .any(|s| (s - feet).abs() <= 2);
            if !on {
                broke.push(Finding {
                    rule: "the start".into(),
                    said: format!(
                        "the hero ({}) starts at x {}, y {} with its feet at row {feet}, and no girder is there at x {cx}: it would fall",
                        k.name,
                        p.x.floor(),
                        p.y.floor()
                    ),
                });
            }
            // The goal, by full ladders from the start's girder: the kit's
            // rule where it covers the stage (one game's elevator stage
            // is crossed by its elevators; the play check says what it couldn't play).
            if let Some(goal) = lay.goal_under.filter(|_| held) {
                let stand = feet - p.y.floor();
                let start = lay
                    .girders
                    .iter()
                    .position(|gd| gd.surface(cx).is_some_and(|s| (s - feet).abs() <= 2));
                let tops: Vec<usize> = (0..lay.girders.len())
                    .filter(|&i| lay.girders[i].pieces.iter().any(|pc| pc[1] - stand < goal))
                    .collect();
                if let Some(s0) = start {
                    let mut seen = BTreeSet::from([s0]);
                    let mut todo = vec![s0];
                    while let Some(gi) = todo.pop() {
                        for ld in &lay.ladders {
                            if ld.lower == Some(gi) && reaches(&lay, ld) {
                                let u = ld.upper.expect("reaches");
                                if seen.insert(u) {
                                    todo.push(u);
                                }
                            }
                        }
                    }
                    if tops.is_empty() {
                        broke.push(Finding {
                            rule: kit_rule(g, &["goal"], "the goal"),
                            said: format!(
                                "no girder is high enough for the goal: the hero's top must be above row {goal}, so it must stand on a girder at row {} or above",
                                goal + stand - 1
                            ),
                        });
                    } else if !tops.iter().any(|t| seen.contains(t)) {
                        broke.push(Finding {
                            rule: kit_rule(g, &["goal"], "the goal"),
                            said: format!(
                                "the goal can't be reached: from the start's girder full ladders reach girders {:?}, and the goal is on girder {:?} (a full ladder's top must be within {MEETS} pixels of the girder above)",
                                seen.iter().collect::<Vec<_>>(),
                                tops
                            ),
                        });
                    }
                }
            }
        }
        // Each ladder top and stop sits on a ladder.
        for (what, list) in [("top", &l.ladder_tops), ("stop", &l.ladder_stops)] {
            for (i, t) in list.iter().enumerate() {
                if !l.ladders.iter().any(|b| b[0] == t[0]) {
                    broke.push(Finding {
                        rule: kit_rule(g, &["ladder", what], &format!("a ladder's {what}")),
                        said: format!(
                            "{what} {i} at x {}, y {} has no ladder at its x: move ladder N moves a ladder's {what} with it",
                            t[0], t[1]
                        ),
                    });
                }
            }
        }
        if kit_covers(g, refs)
            && let Some(k) = g.kit()
        {
            for c in &k.code {
                let t = format!("{} {}", c.what, c.rule).to_lowercase();
                let held = (t.contains("row") && t.contains("run"))
                    || (t.contains("ladder") && t.contains("fire"))
                    || t.contains("goal");
                if !held {
                    by_hand.push(format!("{}: {}", c.what, c.rule));
                }
            }
        }
    } else if refs.is_tile_stage() {
        tile_rules(g, refs, &spec, li, &mut broke, &mut by_hand);
    }
    Ok(Checked {
        spec,
        li,
        lay,
        runs,
        broke,
        by_hand,
    })
}

/// A tile stage's rules, by the model's numbers ([`tile_breaks`]). The
/// kit's own rules on the start and the goal name them; its others are
/// kept by hand where it covers the stage.
fn tile_rules(
    g: &Game,
    refs: &StageRefs,
    spec: &Spec,
    li: usize,
    broke: &mut Vec<Finding>,
    by_hand: &mut Vec<String>,
) {
    for (what, said) in tile_breaks(spec, li) {
        let rule = match what {
            "start" => kit_rule(g, &["start"], "the start"),
            "goal" => kit_rule(g, &["goal"], "the goal"),
            _ => "the level's size".to_owned(),
        };
        broke.push(Finding { rule, said });
    }
    if kit_covers(g, refs)
        && let Some(k) = g.kit()
    {
        for c in &k.code {
            let t = format!("{} {}", c.what, c.rule).to_lowercase();
            if !(t.contains("goal") || t.contains("start")) {
                by_hand.push(format!("{}: {}", c.what, c.rule));
            }
        }
    }
}

/// What a tile stage breaks, each with its rule's key (`size`, `start` or
/// `goal`): every start inside the level, the hero's on ground (a solid
/// or top-only cell under its feet), and the goal inside the level.
fn tile_breaks(spec: &Spec, li: usize) -> Vec<(&'static str, String)> {
    use gp_sim::spec::Role;
    let l = &spec.levels[li];
    let (w, h) = (l.size[0] * l.tile_size, l.size[1] * l.tile_size);
    let mut out = Vec::new();
    for s in &l.spawns {
        let k = &spec.kinds[s.kind];
        let (x, y) = (s.x.floor(), s.y.floor());
        if !(0..w).contains(&x) || !(0..h).contains(&y) {
            out.push((
                "size",
                format!(
                    "the {} starts at x {x}, y {y}, outside the level's {w} by {h} pixels",
                    k.name
                ),
            ));
        }
        if k.role != Role::Player {
            continue;
        }
        let hb = k.tile_hitbox;
        let feet = y + hb.y.floor() + hb.h.floor();
        let (x0, x1) = (x + hb.x.floor(), x + hb.x.floor() + hb.w.floor() - 1);
        let on = (x0..=x1).any(|px| {
            let t = l.tile_at(px, feet);
            t.solid() || t == Tile::Top
        });
        if !on {
            out.push((
                "start",
                format!(
                    "the hero ({}) starts at x {x}, y {y} with its feet at row {feet}, and no solid cell is under them from x {x0} to {x1}: it would fall",
                    k.name
                ),
            ));
        }
    }
    if let Some(gx) = l.goal_x.map(gp_sim::Fx::floor)
        && !(0..w).contains(&gx)
    {
        out.push((
            "goal",
            format!("the goal is at x {gx}, outside the level's {w} pixels across"),
        ));
    }
    out
}

fn group_words(v: &[Option<usize>]) -> String {
    let mut by: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
    for (i, g) in v.iter().enumerate() {
        by.entry(*g).or_default().push(i);
    }
    by.values()
        .map(|rows| format!("{rows:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Ladders the model's numbers say a thing's code climbs: an entity's
/// params `ladder_<k>_x` with `ladder_<k>_foot`, or `ladder_c<k>` with
/// `ladder_cfoot<k>` (one game's fires' four and its barrels' third).
/// Each as (entity, the params' name, x, foot, the entity's size).
fn code_ladders(model: &Model, spec: &Spec) -> Vec<(String, String, i64, i64, [i64; 2])> {
    let mut out = Vec::new();
    for (name, e) in &model.entities {
        let p = &e.core.ext_params;
        let size = spec
            .kinds
            .iter()
            .find(|k| k.name == *name)
            .map_or([16, 16], |k| [k.size[0].floor(), k.size[1].floor()]);
        let int = |k: &str| p.get(k).map(|f| f.raw() >> 16);
        for k in p.keys() {
            if let Some(stem) = k.strip_prefix("ladder_").and_then(|s| s.strip_suffix("_x"))
                && let (Some(x), Some(foot)) = (int(k), int(&format!("ladder_{stem}_foot")))
            {
                out.push((
                    name.clone(),
                    format!("ladder_{stem}_x and _foot"),
                    x,
                    foot,
                    size,
                ));
            }
            if let Some(n) = k.strip_prefix("ladder_c")
                && n.chars().all(|c| c.is_ascii_digit())
                && let (Some(x), Some(foot)) = (int(k), int(&format!("ladder_cfoot{n}")))
            {
                out.push((
                    name.clone(),
                    format!("ladder_c{n} and ladder_cfoot{n}"),
                    x,
                    foot,
                    size,
                ));
            }
        }
    }
    out
}

// --- Originality --------------------------------------------------------------

/// A level's cells from its boxes: ground pieces as tops, ladders as
/// ladders, so two stages of the same engine compare by what they lay out
/// (the drawn picture, `stage_screens`, isn't edited and would compare
/// the same).
fn box_cells(spec: &Spec, li: usize) -> Cells {
    let l = &spec.levels[li];
    let t = l.tile_size.max(1);
    let mut tiles = vec![Tile::Empty; (l.size[0] * l.size[1]) as usize];
    let mut put = |x: i64, y: i64, k: Tile| {
        let (cx, cy) = (x / t, y / t);
        if cx >= 0 && cy >= 0 && cx < l.size[0] && cy < l.size[1] {
            tiles[(cy * l.size[0] + cx) as usize] = k;
        }
    };
    for b in &l.ground {
        let mut x = b[0];
        while x <= b[2] {
            put(x, b[1], Tile::Top);
            x += 1;
        }
    }
    for b in &l.ladders {
        let mut y = b[1];
        while y <= b[3] {
            put((b[0] + b[2]) / 2, y, Tile::Ladder);
            y += 1;
        }
    }
    Cells {
        size: l.size,
        tile_size: l.tile_size,
        tiles,
    }
}

fn originality(
    g: &Game,
    refs: &StageRefs,
    now: &Checked,
    original: &mut Data,
) -> Result<Value, String> {
    let ospec = compile(g, original, &g.model)?;
    let olay = stage::layout(&ospec, now.li);
    let a = box_cells(&now.spec, now.li);
    let b = box_cells(&ospec, now.li);
    let s = stage::similarity((&a, &now.lay), (&b, &olay));
    let (cells_at, ladders_at) = (
        crate::template_original::CELLS_REFUSE,
        crate::template_original::LADDERS_REFUSE,
    );
    let too_close = s.cells >= cells_at || s.ladders >= ladders_at;
    let _ = refs;
    Ok(json!({
        "cells": (s.cells * 1000.0).round() / 1000.0,
        "ladders": (s.ladders * 1000.0).round() / 1000.0,
        "things": (s.things * 1000.0).round() / 1000.0,
        "shift_cells": s.shift,
        "mirrored": s.mirrored,
        "refused_at": {"cells": cells_at, "ladders": ladders_at},
        "apart": !too_close,
        "said": if too_close {
            format!(
                "too close to the original's layout: {:.0}% of the cells either fills are the same (refused at {:.0}%) and {:.0}% of the ladders are where the original's are (refused at {:.0}%), at the best shift of up to {} cells, mirrored or not. Move girders and ladders further from where the game has them",
                s.cells * 100.0, cells_at * 100.0, s.ladders * 100.0, ladders_at * 100.0, stage::SHIFT_REACH
            )
        } else {
            format!(
                "apart from the original's layout: {:.0}% of cells the same (refused at {:.0}%), {:.0}% of ladders (refused at {:.0}%)",
                s.cells * 100.0, cells_at * 100.0, s.ladders * 100.0, ladders_at * 100.0
            )
        },
    }))
}

// --- Measures against the kit -------------------------------------------------

/// The numbers in a target or a tolerance, leaving out what is in
/// brackets.
fn numbers_in(s: &str) -> Vec<f64> {
    let mut depth = 0;
    let clean: String = s
        .chars()
        .map(|c| {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth > 0 || c == ')' { ' ' } else { c }
        })
        .collect();
    clean
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter_map(|w| w.parse::<f64>().ok())
        .collect()
}

/// The kit's play measures against what was measured: each with what it
/// is, the target, the tolerance and whether it lands within it (null
/// when not measured, or the kit sets no tolerance).
fn against_kit(rules: &Rules, s: &BTreeMap<String, Value>) -> Vec<Value> {
    let num = |k: &str| s.get(k).and_then(Value::as_f64);
    let list = |k: &str| -> Option<Vec<f64>> {
        match s.get(k)? {
            Value::Array(a) => a.iter().map(Value::as_f64).collect(),
            v => v.as_f64().map(|x| vec![x]),
        }
    };
    let mut out = Vec::new();
    for m in &rules.measure {
        let target: Vec<f64> = match &m.target {
            toml::Value::Integer(i) => vec![*i as f64],
            toml::Value::Float(f) => vec![*f],
            toml::Value::Array(a) => a
                .iter()
                .filter_map(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
                .collect(),
            toml::Value::String(t) => numbers_in(t),
            _ => Vec::new(),
        };
        let within = numbers_in(&m.within).first().copied();
        let pct = m.within.contains('%');
        let keys: Vec<&str> = m.name.split(',').map(str::trim).collect();
        let have: Option<Vec<f64>> = match (keys.as_slice(), &m.target) {
            // "6 + 1": the girders and the platforms.
            (["girders"], toml::Value::String(t)) if t.contains('+') => Some(vec![
                num("girders").unwrap_or(0.0) + num("platforms").unwrap_or(0.0),
            ]),
            (["things"], _) => s.get("things").and_then(Value::as_array).map(|a| {
                let p: Vec<&Value> = a.iter().filter(|t| t["kind"] == "hammer").collect();
                p.iter()
                    .filter_map(|t| t["height"].as_f64())
                    .chain(p.iter().filter_map(|t| t["to_ladder"].as_f64()))
                    .collect()
            }),
            _ => keys
                .iter()
                .map(|k| list(k))
                .collect::<Option<Vec<Vec<f64>>>>()
                .map(|v| v.concat()),
        };
        let target = if matches!(&m.target, toml::Value::String(t) if t.contains('+')) {
            vec![target.iter().sum()]
        } else {
            target
        };
        let ok = match (&have, within) {
            (Some(h), Some(w)) if h.len() == target.len() && !h.is_empty() => {
                Some(h.iter().zip(&target).all(|(a, b)| {
                    let tol = if pct { b.abs() * w / 100.0 } else { w };
                    (a - b).abs() <= tol + 1e-9
                }))
            }
            (Some(h), Some(_)) if h.len() != target.len() => Some(false),
            _ => None,
        };
        out.push(json!({
            "name": m.name,
            "what": m.what,
            "have": have,
            "target": target,
            "within": m.within,
            "ok": ok,
        }));
    }
    out
}

// --- Running on the model -----------------------------------------------------

fn source(
    g: &Game,
    data: &Data,
    model: &Model,
    paths: &Paths,
    module: Option<&Path>,
) -> Result<crate::hosted::Source> {
    let bundle = gp_interp::Bundle::new(model, data.lifted());
    let own = match module {
        Some(_) => None,
        None => crate::module::built_for(&paths.home, &g.dir)?,
    };
    let module = module.or(own.as_ref().map(|o| o.0.as_path()));
    crate::hosted::Source::new(bundle, module)
}

fn options(
    model: &Model,
    spec: &Spec,
    roller: Option<&str>,
    runs: u32,
    frames: u32,
) -> stage::Options {
    let kind = |name: &str| spec.kinds.iter().position(|k| k.name == name);
    stage::Options {
        enemies: model
            .entities
            .iter()
            .filter(|(_, e)| e.core.role == "enemy")
            .filter_map(|(n, _)| kind(n))
            .collect(),
        roller: roller.and_then(kind),
        playing: spec
            .machines
            .iter()
            .position(|m| m.name == "game")
            .and_then(|mi| {
                spec.machines[mi]
                    .states
                    .iter()
                    .position(|s| s.name == "playing")
                    .map(|si| (mi, si))
            }),
        run_frames: frames,
        runs,
    }
}

// --- Pictures -----------------------------------------------------------------

fn picture_path(paths: &Paths, g: &Game, level: &str, art: bool) -> PathBuf {
    paths.home.join("work").join("level-pictures").join(format!(
        "{}-{level}{}.png",
        g.name,
        if art { "-art" } else { "" }
    ))
}

fn roles(model: &Model, spec: &Spec) -> Vec<String> {
    spec.kinds
        .iter()
        .map(|k| {
            model
                .entities
                .get(&k.name)
                .map(|e| e.core.role.clone())
                .unwrap_or_default()
        })
        .collect()
}

fn draw(c: &Checked, model: &Model, file: &Path) -> Result<()> {
    let p = pieces(&c.spec, c.li, &c.lay, &c.runs);
    let roles = roles(model, &c.spec);
    let marks = Marks {
        runs: &p.runs,
        ladders: &p.ladders,
        roles: &roles,
    };
    level_picture::by_kind(&c.spec, c.li, &c.lay, &marks).write_png(file)
}

/// The model's own picture of the level at the start of play, from the
/// person's lift, with the pieces outlined over it.
fn draw_art(
    g: &Game,
    c: &Checked,
    src: &crate::hosted::Source,
    model: &Model,
    file: &Path,
) -> Result<()> {
    use gp_interp::variant::{Edit, Sim};
    let file = std::path::absolute(file)?;
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&file) {
        bail!(
            "{} is inside the git work tree at {}; a picture with the game's art stays outside any repo",
            file.display(),
            repo.display()
        );
    }
    let spec = &c.spec;
    let units = gp_interp::mapping::board_units_in(&g.dir).map_err(anyhow::Error::msg)?;
    let art = crate::screen::art(model, spec, &g.assets, units)?;
    let v = Sim::open(src, spec, &Edit::Full).map_err(|e| anyhow!("{e}"))?;
    let mut w = gp_sim::World::power_on(&v);
    let mut anim = gp_present::Animator::new();
    let o = options(model, spec, None, 0, 0);
    let mut since = None;
    for f in 0..2400u64 {
        let pad = if (40..45).contains(&f) { 0x08 } else { 0 };
        let ev = v.step(&mut w, pad);
        anim.fired(&art.present, spec, &w, &ev);
        anim.step(&art.present, spec, &w);
        let playing = o
            .playing
            .is_none_or(|(m, s)| w.machines.get(m).is_some_and(|x| x.state == s));
        if playing && w.level == c.li {
            let n = since.get_or_insert(f);
            if f >= *n + 60 {
                break;
            }
        }
    }
    let names: Vec<String> = w
        .bodies
        .iter()
        .map(|b| {
            spec.kinds
                .get(b.kind)
                .map(|k| k.name.clone())
                .unwrap_or_default()
        })
        .collect();
    let s = level_picture::SCALE;
    let mut canvas = if art.in_board_units() {
        // An arcade model in the board's own units: the board's picture
        // (its art and bodies placed as the board draws them, 2 screen
        // pixels a unit), shown at the level's size in units so the
        // pieces' outlines land on it.
        let frame = crate::screen::world_frame(&art, &anim, spec, &w);
        let l = &spec.levels[c.li];
        let (cw, ch) = (
            (l.size[0] * l.tile_size).max(1) as u32 * s,
            (l.size[1] * l.tile_size).max(1) as u32 * s,
        );
        let u = gp_arcade::video::UNIT as u32;
        let mut canvas = Canvas::new(cw, ch, level_picture::BACK);
        for y in 0..ch {
            for x in 0..cw {
                let (fx, fy) = (x * u / s, y * u / s);
                if fx < frame.w && fy < frame.h {
                    canvas.px[(y * cw + x) as usize] = frame.rgb[(fy * frame.w + fx) as usize];
                }
            }
        }
        canvas
    } else {
        let scene = anim.scene(&art.present, spec, &w);
        let frame =
            gp_present::raster::draw(&art.present, &w, &art.pics, &scene, &names, &art.fields);
        let mut canvas = Canvas::new(frame.w * s, frame.h * s, level_picture::BACK);
        for y in 0..frame.h * s {
            for x in 0..frame.w * s {
                canvas.px[(y * frame.w * s + x) as usize] =
                    frame.rgb[((y / s) * frame.w + x / s) as usize];
            }
        }
        canvas
    };
    canvas.outlines = true;
    let p = pieces(spec, c.li, &c.lay, &c.runs);
    let roles = roles(model, spec);
    let marks = Marks {
        runs: &p.runs,
        ladders: &p.ladders,
        roles: &roles,
    };
    level_picture::draw_layout(&mut canvas, spec, c.li, &c.lay, &marks);
    canvas.write_png(&file)
}

// --- History ------------------------------------------------------------------

/// One edit as the copy's history keeps it: each file it changed, as it
/// was before, by its path in the copy (None for a file it made).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    seq: u64,
    level: String,
    ops: Vec<String>,
    before: BTreeMap<String, Option<String>>,
}

fn read_history(dir: &Path) -> Result<Vec<Entry>> {
    let Ok(t) = fs::read_to_string(dir.join(HISTORY_FILE)) else {
        return Ok(Vec::new());
    };
    t.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).with_context(|| HISTORY_FILE.to_owned()))
        .collect()
}

fn write_history(dir: &Path, h: &[Entry]) -> Result<()> {
    let mut s = String::new();
    for e in h {
        s.push_str(&serde_json::to_string(e)?);
        s.push('\n');
    }
    fs::write(dir.join(HISTORY_FILE), s).with_context(|| HISTORY_FILE.to_owned())
}

/// Gives the copy its own assets, as the MCP art tools do, so the shared
/// lift (the person's ROM's) is never written.
fn own_assets(g: &mut Game, paths: &Paths) -> Result<()> {
    if g.own_assets {
        return Ok(());
    }
    let config = gp_mcp::Config {
        gp: std::env::current_exe().context("finding the gp binary")?,
        repo: g.dir.clone(),
        work: g.dir.parent().map(Path::to_owned).unwrap_or_default(),
        home: paths.home.clone(),
        rom: None,
        assets: None,
        games: None,
    };
    let (to, _) =
        gp_mcp::place::copy_assets(&config, &g.dir, &g.model).map_err(|e| anyhow!("{e}"))?;
    g.assets = to;
    g.own_assets = true;
    Ok(())
}

// --- The commands -------------------------------------------------------------

/// `gp level list GAME`.
pub(crate) fn list_cmd(game: &Path, paths: &Paths) -> Result<Outcome> {
    let dir = resolve(game, paths);
    let text = fs::read_to_string(dir.join(gp_model::MODEL_FILE)).with_context(|| {
        format!(
            "{} isn't a game's folder: it has no model.toml",
            dir.display()
        )
    })?;
    let model = Model::parse(&text)?;
    let name = std::path::absolute(&dir)?
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("the game")
        .to_owned();
    let rules = level_kit::load_rules(&dir)?;
    let kit_slots: BTreeSet<String> = rules
        .iter()
        .flat_map(|r| r.data.iter().map(|d| d.slot.clone()))
        .collect();
    let mut levels = Vec::new();
    let mut lines = Vec::new();
    // An arcade model in the board's own units counts its cells in them.
    let unit = if gp_interp::mapping::board_units_in(&dir).map_err(anyhow::Error::msg)? {
        "units (the board's own, 2 screen pixels each)"
    } else {
        "px"
    };
    for (lname, l) in &model.levels {
        let refs = StageRefs::of(&model, lname).map_err(|e| anyhow!("{e}"))?;
        let covered = refs.own_slots().iter().any(|s| kit_slots.contains(s));
        let kind = if refs.is_girder_stage() {
            "girders and ladders"
        } else if l.core.empty {
            "no tiles yet"
        } else {
            "tiles"
        };
        lines.push(format!(
            "{lname}: {} ({kind}; {} by {} cells of {} {unit}; {} starts){}",
            l.core.description.clone().unwrap_or_default(),
            l.core.size[0],
            l.core.size[1],
            l.core.tile_size,
            l.core.spawn.len(),
            if covered {
                "; the level kit's rules cover it"
            } else {
                ""
            }
        ));
        levels.push(json!({
            "level": lname,
            "description": l.core.description,
            "kind": kind,
            "cells": l.core.size,
            "tile_size": l.core.tile_size,
            "starts": l.core.spawn.len(),
            "slots": refs.slots(),
            "kit_covers": covered,
        }));
    }
    let stub = rules
        .as_ref()
        .is_none_or(|r| r.data.is_empty() && r.code.is_empty() && r.limit.is_empty());
    let mut j = json!({"game": name, "dir": dir, "levels": levels, "kit": !stub});
    if stub {
        let why = format!(
            "{name} has no level kit yet: {}. gp level show, edit and verify need one; gp level render draws any level's starts",
            if rules.is_some() {
                "its level-kit/rules.toml is still the stub"
            } else {
                "it has no level-kit/rules.toml"
            }
        );
        lines.push(why.clone());
        j["no_kit"] = json!(why);
    }
    let editable = editable(&dir, paths);
    j["editable"] = json!(editable.is_ok());
    if let Err(e) = editable {
        j["not_editable"] = json!(e);
    }
    Ok(Outcome::ok(j, lines.join("\n")))
}

/// Why a level of an arcade model can't be verified or have its girders
/// edited, naming it: the stage measure (gp_interp::stage) plays girders
/// and ladders with the NES's pad, and none reads an arcade level yet.
/// None for an NES model's level.
fn arcade_level(g: &Game, level: &str) -> Option<String> {
    let arcade = g
        .model
        .input
        .as_ref()
        .is_some_and(|i| i.device == gp_model::model::Device::Arcade);
    arcade.then(|| {
        format!(
            "level {level} of {} is an arcade board's, and the stage measure plays a stage of girders and ladders on the NES's pad, so no stage measure reads it yet",
            g.name
        )
    })
}

/// The level named, or the kit's first one, or `main`.
fn level_of(g: &Game, level: Option<&str>) -> Result<String> {
    if let Some(l) = level {
        if !g.model.levels.contains_key(l) {
            bail!(
                "{} has no level `{l}`; its levels are {}",
                g.name,
                g.model
                    .levels
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return Ok(l.to_owned());
    }
    let kit_slots: BTreeSet<String> = g
        .kit()
        .map(|k| k.data.iter().map(|d| d.slot.clone()).collect())
        .unwrap_or_default();
    let covered = g.model.levels.keys().find(|l| {
        StageRefs::of(&g.model, l)
            .is_ok_and(|r| r.own_slots().iter().any(|s| kit_slots.contains(s)))
    });
    covered
        .or_else(|| g.model.levels.keys().find(|l| *l == "main"))
        .or_else(|| g.model.levels.keys().next())
        .cloned()
        .context("the model has no levels")
}

/// Refuses in words a game with no level kit, before its lift is read
/// (a game with no kit may have no lift in this home either).
fn kit_first(game: &Path, paths: &Paths) -> Result<()> {
    let dir = resolve(game, paths);
    if !dir.join(gp_model::MODEL_FILE).is_file() {
        return Ok(());
    }
    let rules = level_kit::load_rules(&dir)?;
    let name = std::path::absolute(&dir)?
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("the game")
        .to_owned();
    match no_kit_words(&name, &dir, rules.as_ref()) {
        Some(why) => Err(anyhow!("{why}")),
        None => Ok(()),
    }
}

/// `gp level show GAME [--level L]`.
pub(crate) fn show_cmd(
    game: &Path,
    level: Option<&str>,
    with_data: bool,
    paths: &Paths,
) -> Result<Outcome> {
    kit_first(game, paths)?;
    let g = Game::open(game, paths)?;
    let level = level_of(&g, level)?;
    let refs = StageRefs::of(&g.model, &level).map_err(|e| anyhow!("{e}"))?;
    let mut data = Data::of(&g.lifted);
    let mut original = Data::of(&g.original);
    let c = check(&g, &refs, &mut data, &g.model, &mut original).map_err(|e| anyhow!("{e}"))?;
    let j = describe_level(&g, &refs, &mut data, &c, with_data)?;
    let pieces = if refs.is_tile_stage() {
        let l = &c.spec.levels[c.li];
        format!(
            "a grid of {} by {} cells of {} px (slot {})",
            l.size[0],
            l.size[1],
            l.tile_size,
            refs.grid.as_deref().unwrap_or("")
        )
    } else {
        format!(
            "{} girders, {} ladders",
            c.lay.girders.len(),
            c.lay.ladders.len()
        )
    };
    let mut text = vec![format!(
        "{} level {level}: {pieces}, {} starts{}",
        g.name,
        g.model.levels[&level].core.spawn.len(),
        if kit_covers(&g, &refs) {
            ""
        } else {
            "; the kit's rules don't cover this level, so only the format, the screen and the reach are checked"
        }
    )];
    for p in j["pieces"]["girders"].as_array().into_iter().flatten() {
        text.push(format!(
            "  girder {}: {}",
            p["girder"],
            p["said"].as_str().unwrap_or("")
        ));
    }
    for p in j["pieces"]["ladders"].as_array().into_iter().flatten() {
        text.push(format!(
            "  ladder {}: {}",
            p["ladder"],
            p["said"].as_str().unwrap_or("")
        ));
    }
    for p in j["pieces"]["spawns"].as_array().into_iter().flatten() {
        text.push(format!(
            "  start {}#{}: x {}, y {}",
            p["entity"].as_str().unwrap_or(""),
            p["index"],
            p["x"],
            p["y"]
        ));
    }
    for f in &c.broke {
        text.push(format!("  breaks {}: {}", f.rule, f.said));
    }
    text.push(format!("operations: {OPS_HELP}"));
    Ok(Outcome::ok(j, text.join("\n")))
}

fn describe_level(
    g: &Game,
    refs: &StageRefs,
    data: &mut Data,
    c: &Checked,
    with_data: bool,
) -> Result<Value> {
    let l = &c.spec.levels[c.li];
    let e = |e: String| anyhow!("{e}");
    let mut girders = Vec::new();
    if let Some(rl) = &refs.runs {
        let rows = data.read(g.at(), rl).map_err(e)?;
        let at = run_girders(&c.lay, &c.runs);
        for (i, r) in rows.iter().enumerate() {
            let b = c.runs.get(i).cloned().unwrap_or_default();
            let (x0, x1) = (
                b.iter().map(|p| p[0]).min().unwrap_or(0),
                b.iter().map(|p| p[2]).max().unwrap_or(0),
            );
            let (y_first, y_last) = (b.first().map_or(0, |p| p[1]), b.last().map_or(0, |p| p[1]));
            let step = if refs.boxes {
                0
            } else if r[3] >= 128 {
                r[3] - 256
            } else {
                r[3]
            };
            girders.push(json!({
                "girder": i,
                "row": r,
                "x": r[0], "y": r[1], "shape": r[2] / refs.shape_step,
                "step": step, "count": if refs.boxes { 1 } else { r[4] },
                "from": [x0, x1], "rows": [y_first, y_last],
                "on": at.get(i).copied().flatten(),
                "said": format!(
                    "x {x0} to {x1}, row {y_first} at its first piece to {y_last} at its last, {} pieces{}",
                    b.len(),
                    at.get(i).copied().flatten().map(|gi| format!(", girder {gi} from the bottom")).unwrap_or_default()
                ),
            }));
        }
    }
    let mut ladders = Vec::new();
    if let Some((ll, _, step)) = &refs.ladders {
        let rows = data.read(g.at(), ll).map_err(e)?;
        let tops = match &refs.tops {
            Some((t, _, _)) => data.read(g.at(), t).map_err(e)?,
            None => Vec::new(),
        };
        let stops = match &refs.stops {
            Some((t, _, _)) => data.read(g.at(), t).map_err(e)?,
            None => Vec::new(),
        };
        for (i, (r, x)) in rows.iter().zip(&c.lay.ladders).enumerate() {
            let full = reaches(&c.lay, x);
            let top = partner(&tops, r, true);
            let stop = partner(&stops, r, false);
            ladders.push(json!({
                "ladder": i,
                "row": r,
                "x": r[0], "y": r[1], "shape": r[2] / step,
                "box": x.bx,
                "lower": x.lower, "upper": x.upper,
                "reaches_above": full,
                "top": top, "stop": stop,
                "said": format!(
                    "x {} to {}, rows {} to {}, from girder {} {}{}{}",
                    x.bx[0], x.bx[2], x.bx[1], x.bx[3],
                    x.lower.map_or("none".into(), |v| v.to_string()),
                    match (full, x.upper) {
                        (true, Some(u)) => format!("up to girder {u} (full)"),
                        (false, Some(u)) => format!("short of girder {u} (broken)"),
                        _ => "to no girder above".into(),
                    },
                    top.map(|t| format!(", top {t}")).unwrap_or_default(),
                    stop.map(|t| format!(", stop {t}")).unwrap_or_default(),
                ),
            }));
        }
    }
    let spawns: Vec<Value> = spawns(&g.model, &refs.level)
        .into_iter()
        .map(|(i, entity, k, x, y)| {
            let role = g.model.entities.get(&entity).map(|e| e.core.role.clone());
            json!({"spawn": i, "entity": entity, "index": k, "x": x, "y": y, "role": role})
        })
        .collect();
    let mut slots = BTreeMap::new();
    let mut whole = Whole::default();
    for s in writable_slots(g, refs) {
        if let Ok(d) = data.load(g.at(), &s) {
            slots.insert(
                s.clone(),
                json!({"file": d.file, "format": match &d.format { Format::Rows => json!("rows"), Format::Table(c) => json!({"table": c}), Format::Grid(l) => json!({"grid": l.iter().collect::<String>()}) }, "rows": d.rows.len()}),
            );
            whole.slots.insert(s, d.rows.clone());
        }
    }
    whole.spawns = spawns
        .iter()
        .map(|s| WholeSpawn {
            spawn: s["spawn"].as_u64().unwrap_or(0) as usize,
            entity: s["entity"].as_str().unwrap_or("").to_owned(),
            x: s["x"].as_i64().unwrap_or(0),
            y: s["y"].as_i64().unwrap_or(0),
        })
        .collect();
    let kit = g.kit().map(|k| {
        json!({
            "covers_this_level": kit_covers(g, refs),
            "summary": k.summary.trim(),
            "data": k.data.iter().map(|p| json!({"what": p.what, "rule": p.rule, "slot": p.slot})).collect::<Vec<_>>(),
            "code": k.code.iter().map(|p| json!({"what": p.what, "rule": p.rule})).collect::<Vec<_>>(),
            "limits": k.limit.iter().map(|p| json!({"what": p.what, "limit": p.limit})).collect::<Vec<_>>(),
            "measures": k.measure.iter().map(|p| json!({"name": p.name, "what": p.what, "target": p.target.to_string(), "within": p.within})).collect::<Vec<_>>(),
            "open": k.open,
        })
    });
    Ok(json!({
        "game": g.name,
        "dir": g.dir,
        "level": refs.level,
        "size": [l.size[0] * l.tile_size, l.size[1] * l.tile_size],
        "goal_under": c.lay.goal_under,
        "editable": g.not_editable.is_none(),
        "not_editable": g.not_editable,
        "assets": g.assets,
        "own_assets": g.own_assets,
        "pieces": {
            "girders": girders,
            "girders_from_the_bottom": c.lay.girders.iter().enumerate().map(|(i, gd)| json!({"girder": i, "from": [gd.x0(), gd.x1()], "ends": gd.ends(), "mid": gd.mid()})).collect::<Vec<_>>(),
            "ladders": ladders,
            "spawns": spawns,
        },
        "slots": slots,
        "data": if with_data { json!(whole) } else { json!("the level's whole data (every slot's rows and each start), which gp level edit --from takes back: gp level show --data") },
        "breaks": c.broke,
        "kept_by_hand": c.by_hand,
        "kit": kit,
        "operations": OPS_HELP,
    }))
}

/// `gp level render GAME [--level L] [-o FILE] [--art]`.
pub(crate) fn render_cmd(
    game: &Path,
    level: Option<&str>,
    out: Option<&Path>,
    art: bool,
    module: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let g = Game::open(game, paths)?;
    let level = level_of(&g, level)?;
    let refs = StageRefs::of(&g.model, &level).map_err(|e| anyhow!("{e}"))?;
    let mut data = Data::of(&g.lifted);
    let mut original = Data::of(&g.original);
    let c = check(&g, &refs, &mut data, &g.model, &mut original).map_err(|e| anyhow!("{e}"))?;
    let file = match out {
        Some(f) => f.to_owned(),
        None => picture_path(paths, &g, &level, art),
    };
    if art {
        let src = source(&g, &data, &g.model, paths, module)?;
        draw_art(&g, &c, &src, &g.model, &file)?;
    } else {
        draw(&c, &g.model, &file)?;
    }
    let mut lines = vec![format!(
        "wrote {}: {} level {level} {}",
        file.display(),
        g.name,
        if art {
            "as the model draws it at the start of play, from the person's own lift, with the stage's pieces outlined; it shows the game's art, so it stays on this computer. The background is the stage's picture slot, which an edit doesn't redraw: the outlines are where the pieces are"
        } else {
            "drawn by what each piece is (girders red, full ladders cyan, broken ones dim, tops white, stops as red crosses, each start as a box by its role, the goal's row green): no art from the ROM, so it can be shown anywhere. The numbers are the girder rows and ladders as gp level edit names them"
        }
    )];
    if let Some(why) = g.no_kit() {
        lines.push(why);
    }
    Ok(Outcome::ok(
        json!({"game": g.name, "level": level, "picture": file, "art": art, "no_kit": g.no_kit()}),
        lines.join("\n"),
    ))
}

/// The quick play measures an edit returns: the walk, each ladder's climb
/// and the direct route, with no enemies.
fn quick_measures(
    g: &Game,
    c: &Checked,
    data: &Data,
    model: &Model,
    paths: &Paths,
    module: Option<&Path>,
) -> Result<(BTreeMap<String, Value>, Vec<String>)> {
    let src = source(g, data, model, paths, module)?;
    let o = options(model, &c.spec, None, 0, 0);
    let m = stage::measure(&src, &c.spec, c.li, &o).map_err(|e| anyhow!("{e}"))?;
    let mut s = crate::search::summary(&m);
    // No roller ran, so what the rollers give isn't measured.
    for k in [
        "leave_by_ladder_share",
        "throw_to_drum_median",
        "jump_window_median",
    ] {
        s.remove(k);
    }
    Ok((s, m.notes))
}

#[allow(clippy::too_many_arguments)]
/// `gp level edit GAME [--level L] --op ... | --from FILE [--dry-run]`.
pub(crate) fn edit_cmd(
    game: &Path,
    level: Option<&str>,
    ops: &[String],
    from: Option<&Path>,
    dry_run: bool,
    measure: bool,
    module: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    kit_first(game, paths)?;
    let mut g = Game::open(game, paths)?;
    if let Some(why) = &g.not_editable {
        return Ok(refused(&g, why, "the copy", Vec::new()));
    }
    let level = level_of(&g, level)?;
    let refs = StageRefs::of(&g.model, &level).map_err(|e| anyhow!("{e}"))?;
    if ops.is_empty() && from.is_none() {
        bail!(
            "give an operation (--op) or a whole level (--from FILE). The operations: {OPS_HELP}"
        );
    }
    let parsed: Vec<Op> = match ops.iter().map(|o| Op::parse(o)).collect::<Result<_, _>>() {
        Ok(p) => p,
        Err(e) => return Ok(refused(&g, &e, "the operation", Vec::new())),
    };
    let mut data = Data::of(&g.lifted);
    let mut model = g.model.clone();
    let mut original = Data::of(&g.original);
    let applied = match from {
        Some(f) => {
            let text = fs::read_to_string(f).with_context(|| f.display().to_string())?;
            let whole: Whole = if text.trim_start().starts_with('{') {
                serde_json::from_str(&text).with_context(|| f.display().to_string())?
            } else {
                toml::from_str(&text).with_context(|| f.display().to_string())?
            };
            apply_whole(&g, &refs, &mut data, &mut model, &whole)
                .and_then(|()| apply(&g, &refs, &mut data, &mut model, &parsed, &mut original))
        }
        None => apply(&g, &refs, &mut data, &mut model, &parsed, &mut original),
    };
    if let Err(e) = applied {
        return Ok(refused(&g, &e, "the operation", Vec::new()));
    }
    // The stage's picture is play data on the model: its ladder and girder
    // tiles are where a climb holds and ends. It follows the pieces.
    let picture = match redraw(&g, &refs, &mut data, &model) {
        Ok(p) => p,
        Err(e) => return Ok(refused(&g, &e, "a rule", Vec::new())),
    };
    let c = match check(&g, &refs, &mut data, &model, &mut original) {
        Ok(c) => c,
        Err(e) => return Ok(refused(&g, &e, "the format", Vec::new())),
    };
    if !c.broke.is_empty() {
        let words = c
            .broke
            .iter()
            .map(|f| format!("{}: {}", f.rule, f.said))
            .collect::<Vec<_>>()
            .join("; ");
        return Ok(refused(&g, &words, "a rule", c.broke.clone()));
    }
    // On the model, before anything is written: the walk, each ladder's
    // climb and the direct route to the goal. No route is a refusal: the
    // kit's goal must be reached by ladders the hero climbs.
    let measured = if measure && g.kit().is_some() {
        match quick_measures(&g, &c, &data, &model, paths, module) {
            Ok((s, notes)) => {
                if s.get("route_frames").is_none_or(Value::is_null) {
                    let short: Vec<String> = s
                        .get("ladder_places")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter(|l| l["broken"] == true)
                        .filter(|l| {
                            let i = l["ladder"].as_u64().unwrap_or(0) as usize;
                            c.lay.ladders.get(i).is_some_and(|x| reaches(&c.lay, x))
                        })
                        .map(|l| l["ladder"].to_string())
                        .collect();
                    let why = format!(
                        "{}: on the model the hero can't get from the start to the goal by walking and climbing{}",
                        kit_rule(&g, &["goal"], "the goal"),
                        if short.is_empty() {
                            String::new()
                        } else {
                            format!(
                                "; ladders {} reach the girder above but the hero can't climb them (move one a little across, or fit it with move ladder N x=..)",
                                short.join(", ")
                            )
                        }
                    );
                    return Ok(refused(&g, &why, "a rule", Vec::new()));
                }
                Some(Ok((s, notes)))
            }
            Err(e) => Some(Err(e)),
        }
    } else {
        None
    };
    let mut file = picture_path(paths, &g, &level, false);
    if dry_run {
        // A dry run's picture doesn't take the written stage's place.
        file.set_extension("dry-run.png");
    }
    let mut lines = Vec::new();
    let mut j = json!({"game": g.name, "level": level, "ops": ops});
    if !dry_run {
        let first = !g.own_assets;
        own_assets(&mut g, paths)?;
        let mut before = BTreeMap::new();
        let mut writes: Vec<(PathBuf, String)> = Vec::new();
        for s in &data.changed {
            let d = &data.slots[s];
            let path = g.assets.join(&d.file);
            let rel = format!("assets/{}", d.file);
            before.insert(rel, fs::read_to_string(&path).ok());
            writes.push((path, d.text()));
        }
        for (f, t) in &data.raw {
            let path = g.assets.join(f);
            before.insert(format!("assets/{f}"), fs::read_to_string(&path).ok());
            writes.push((path, t.clone()));
        }
        if model.to_toml() != g.model.to_toml() {
            let path = g.dir.join(gp_model::MODEL_FILE);
            before.insert(
                gp_model::MODEL_FILE.to_owned(),
                fs::read_to_string(&path).ok(),
            );
            writes.push((path, model.to_toml()));
        }
        if writes.is_empty() {
            lines.push("it changes nothing, so nothing was written".to_owned());
        } else {
            let mut h = read_history(&g.dir)?;
            let seq = h.last().map_or(1, |e| e.seq + 1);
            for (p, t) in &writes {
                fs::write(p, t).with_context(|| p.display().to_string())?;
            }
            h.push(Entry {
                seq,
                level: level.clone(),
                ops: if ops.is_empty() {
                    vec!["the whole level replaced".into()]
                } else {
                    ops.to_vec()
                },
                before: before.clone(),
            });
            write_history(&g.dir, &h)?;
            lines.push(format!(
                "edit {seq} written to {}: {}{}; gp level undo {} puts it back",
                g.dir.display(),
                before.keys().cloned().collect::<Vec<_>>().join(", "),
                if first {
                    " (the copy now has its own assets, made from the person's lift)"
                } else {
                    ""
                },
                g.dir.display()
            ));
            j["seq"] = json!(seq);
            j["wrote"] = json!(writes.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>());
        }
    } else {
        lines.push("a dry run: it keeps to the kit's rules, and nothing was written".to_owned());
        j["dry_run"] = json!(true);
    }
    draw(&c, &model, &file)?;
    lines.push(format!("picture by kind (no ROM art): {}", file.display()));
    j["picture"] = json!(file);
    if let Some(p) = &picture {
        lines.push(p.said.clone());
        j["stage_picture"] = json!({"bytes": p.bytes, "was": p.was, "said": p.said});
    }
    let mut original2 = Data::of(&g.original);
    let orig = originality(&g, &refs, &c, &mut original2).map_err(|e| anyhow!("{e}"))?;
    lines.push(orig["said"].as_str().unwrap_or("").to_owned());
    j["original"] = orig;
    if let (Some(m), Some(rules)) = (measured, g.kit()) {
        match m {
            Ok((s, notes)) => {
                let k = against_kit(rules, &s);
                for m in &k {
                    if m["have"].is_null() {
                        continue;
                    }
                    lines.push(format!(
                        "  {} {}: {} (target {}, within {})",
                        match m["ok"].as_bool() {
                            Some(true) => "ok  ",
                            Some(false) => "OUT ",
                            None => "    ",
                        },
                        m["name"].as_str().unwrap_or(""),
                        m["have"],
                        m["target"],
                        m["within"].as_str().unwrap_or("")
                    ));
                }
                for n in &notes {
                    lines.push(format!("  note: {n}"));
                }
                j["measures"] = json!(k);
                j["notes"] = json!(notes);
            }
            Err(e) => {
                lines.push(format!("the play measures couldn't run: {e:#}"));
                j["measures_error"] = json!(format!("{e:#}"));
            }
        }
    }
    if !c.by_hand.is_empty() {
        j["kept_by_hand"] = json!(c.by_hand);
    }
    if data
        .changed
        .iter()
        .any(|s| refs.runs.as_ref().is_some_and(|r| r.slot == *s))
    {
        let follow: Vec<String> = g
            .kit()
            .map(|k| {
                k.data
                    .iter()
                    .filter(|d| !refs.slots().contains(&d.slot) && !d.slot.is_empty())
                    .filter(|d| {
                        picture.is_none()
                            || model.levels[&level]
                                .core
                                .layout
                                .as_ref()
                                .is_none_or(|l| l.slot != d.slot)
                    })
                    .map(|d| format!("{} ({})", d.what, d.slot))
                    .collect()
            })
            .unwrap_or_default();
        if !follow.is_empty() {
            let w = format!(
                "the girders moved: the kit names more of the stage as data that follows them, which this edit didn't change: {}. gp level verify shows how the barrels and fires take the new girders",
                follow.join("; ")
            );
            lines.push(w.clone());
            j["follow"] = json!(w);
        }
    }
    Ok(Outcome::ok(j, lines.join("\n")))
}

/// A refused edit: nothing written, and why in words. `what` is what was
/// refused: the copy, the operation, the format or a rule.
fn refused(g: &Game, why: &str, what: &str, broke: Vec<Finding>) -> Outcome {
    let lead = match what {
        "a rule" => "the change breaks the kit's rules",
        "the copy" => "this folder isn't a copy an edit may change",
        "the format" => "the change doesn't fit the kit's format",
        _ => "the operation can't be done",
    };
    Outcome {
        json: json!({"game": g.name, "refused": why, "broke": broke, "nothing_written": true, "operations": OPS_HELP}),
        text: format!("refused, nothing written: {lead}. {why}"),
        exit: 1,
    }
}

/// `gp level undo GAME`.
pub(crate) fn undo_cmd(game: &Path, paths: &Paths) -> Result<Outcome> {
    let dir = std::path::absolute(resolve(game, paths))?;
    if let Err(why) = editable(&dir, paths) {
        bail!("{why}");
    }
    let mut h = read_history(&dir)?;
    let Some(last) = h.pop() else {
        bail!("{} has no level edit to undo", dir.display());
    };
    for (rel, text) in &last.before {
        if rel.contains("..") || Path::new(rel).is_absolute() {
            bail!("{HISTORY_FILE} names {rel}, which isn't a file of the copy");
        }
        let p = dir.join(rel);
        match text {
            Some(t) => fs::write(&p, t).with_context(|| p.display().to_string())?,
            None => {
                if p.exists() {
                    fs::rename(&p, p.with_extension("undone"))?;
                }
            }
        }
    }
    write_history(&dir, &h)?;
    Ok(Outcome::ok(
        json!({"undone": last.seq, "level": last.level, "ops": last.ops, "files": last.before.keys().collect::<Vec<_>>(), "left": h.len()}),
        format!(
            "undid edit {} on level {} ({}); {} edits left to undo",
            last.seq,
            last.level,
            last.ops.join("; "),
            h.len()
        ),
    ))
}

/// What [`verify_cmd`]'s play of a stage found: its lines, whether it can
/// be finished (`finished`: true, false, or null with `checked` false and
/// the reason in `not_checked`), its measures and notes.
struct Played {
    lines: Vec<String>,
    finished: Value,
    not_checked: Option<String>,
    measured: BTreeMap<String, Value>,
    notes: Vec<String>,
}

impl Played {
    fn not_checked(mut self, why: String) -> Self {
        self.lines
            .push(format!("NOT CHECKED whether it can be finished: {why}"));
        self.finished = json!({"finished": null, "checked": false, "why": why});
        self.not_checked = Some(why);
        self
    }
}

/// A girder stage, played: the direct route up its ladders with the
/// enemies in, on the stage as the game reaches it (stage::enter).
fn play_girders(
    g: &Game,
    c: &Checked,
    src: &crate::hosted::Source,
    runs: u32,
    frames: u32,
) -> Played {
    let roller = g.model.entities.contains_key("barrel").then_some("barrel");
    let o = options(&g.model, &c.spec, roller, runs, frames);
    let mut p = Played {
        lines: Vec::new(),
        finished: Value::Null,
        not_checked: None,
        measured: BTreeMap::new(),
        notes: Vec::new(),
    };
    // The stage named, as the game reaches it: the stages before it are
    // ended at their goals by the model's own rules (stage::enter). A
    // stage this search can't play is said to be NOT CHECKED, never
    // passed.
    let m = match stage::measure(src, &c.spec, c.li, &o) {
        Ok(m) => m,
        Err(e) => return p.not_checked(format!("it couldn't be played: {e}")),
    };
    p.measured = crate::search::summary(&m);
    p.notes = m.notes.clone();
    if !m.entered.is_empty() {
        p.lines.push(format!(
            "played where the game starts it: after {} ended at {} goal by the model's own rules, its clear and this stage's setup",
            m.entered.join(", then "),
            if m.entered.len() == 1 { "its" } else { "each one's" }
        ));
    }
    // A stage the direct route can't play to its end (no goal, or no
    // route up its ladders) is played by the search in the whole world
    // (gp_interp::finish): waiting for and riding a carrier, or until the
    // stage ends by its condition.
    if !gp_interp::finish::ends_by_goal(&c.spec, c.li) || !m.routed {
        return play_search(p, src, &c.spec, c.li, &o, m.routed);
    }
    p.finished = match stage::clears(src, &c.spec, &o, &m, 1) {
        Ok((_, cleared, _)) if !cleared.is_empty() => {
            p.lines.push(format!(
                "it can be finished: the direct route played with the enemies in reached the goal at frame {}",
                cleared[0]
            ));
            json!({"finished": true, "checked": true, "frame": cleared[0], "entered_after": m.entered})
        }
        Ok(_) => {
            p.lines.push("it could NOT be finished: the direct route, played with the enemies in, never reached the goal".into());
            json!({"finished": false, "checked": true})
        }
        Err(e) => {
            p.lines.push(format!("it could NOT be finished: {e}"));
            json!({"finished": false, "checked": true, "why": e})
        }
    };
    p
}

/// A girder stage the direct route can't finish, played by
/// gp_interp::finish's search: its words, by what the stage is.
fn play_search(
    mut p: Played,
    src: &crate::hosted::Source,
    spec: &Spec,
    li: usize,
    o: &stage::Options,
    routed: bool,
) -> Played {
    use gp_interp::finish::{self, How};
    let level = spec.levels[li].name.clone();
    let by_goal = finish::ends_by_goal(spec, li);
    let carriers: Vec<String> = finish::carriers(spec, li)
        .into_iter()
        .map(|k| spec.kinds[k].name.clone())
        .collect();
    let what = if !by_goal {
        format!(
            "level {level} has no goal: it ends some other way, so it was played until play leaves the stage with the player alive, taking its pickups on the way"
        )
    } else if carriers.is_empty() {
        format!(
            "no route of walks and ladder climbs reaches the top of level {level}, so it was played by walks, jumps, climbs and waits"
        )
    } else {
        format!(
            "no route of walks and ladder climbs reaches the top of level {level}; its ground carries the player on {} bodies, so it was played by walks, jumps, climbs and waits, riding them",
            carriers.join(" and ")
        )
    };
    p.lines.push(what);
    let r = match finish::finish(src, spec, o, li) {
        Ok(r) => r,
        Err(e) => return p.not_checked(format!("the search couldn't play it: {e}")),
    };
    match r {
        Ok(f) => {
            p.measured.insert("search_places".into(), json!(f.searched));
            p.measured.insert("search_frames".into(), json!(f.frames));
            let taken = if f.taken.is_empty() {
                String::new()
            } else {
                format!(
                    ", after taking {} of its pickups (at frames {})",
                    f.taken.len(),
                    f.taken
                        .iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let rode = if f.rides > 0 {
                format!(", {} of its moves made riding", f.rides)
            } else {
                String::new()
            };
            let (said, next) = match &f.how {
                How::Goal => (
                    format!(
                        "it can be finished: a run with everything in reached the goal at frame {}{taken}{rode} ({} places searched)",
                        f.frame, f.searched
                    ),
                    Value::Null,
                ),
                How::Left { next, next_at, .. } => (
                    format!(
                        "it can be finished: a run with everything in brought the stage to its end at frame {}{taken}{rode}, the player alive; the game went on to level {next} at frame {next_at} ({} places searched)",
                        f.frame, f.searched
                    ),
                    json!({"level": next, "frame": next_at}),
                ),
            };
            p.lines.push(said);
            p.finished = json!({"finished": true, "checked": true, "frame": f.frame,
                "entered_after": f.entered, "by": if by_goal { "goal" } else { "condition" },
                "taken": f.taken, "rides": f.rides, "carriers": f.carriers, "next": next,
                "searched": f.searched});
        }
        Err(n) if n.at_limit => {
            return p.not_checked(format!(
                "the search stopped at its limit of {} places a leg with no run found{}, so it says neither way",
                finish::LIMIT,
                if n.taken.is_empty() {
                    String::new()
                } else {
                    format!(" (after taking {} pickups)", n.taken.len())
                }
            ));
        }
        Err(n) => {
            p.lines.push(format!(
                "it could NOT be finished: every place the player's walks, jumps, climbs and waits reach was searched ({} places{}) and none {}",
                n.searched,
                if n.taken.is_empty() {
                    String::new()
                } else {
                    format!(", after taking {} pickups", n.taken.len())
                },
                if by_goal { "reaches the goal" } else { "ends the stage" }
            ));
            p.finished = json!({"finished": false, "checked": true, "searched": n.searched,
                "taken": n.taken, "routed": routed});
        }
    }
    p
}

/// A tile stage, played by the reach search and the playthrough
/// (gp_interp::reach, `gp interp reach`): the goal reached by the
/// player's own walks and jumps alone, then a run from the stage's start
/// with every enemy in that reaches it. The playthrough's frame is the
/// one the goal is reached on, counted from power-on of a game started
/// on this stage.
fn play_tiles(src: &crate::hosted::Source, li: usize) -> Played {
    use gp_interp::reach;
    let mut p = Played {
        lines: Vec::new(),
        finished: Value::Null,
        not_checked: None,
        measured: BTreeMap::new(),
        notes: Vec::new(),
    };
    let spec = &src.spec;
    let name = spec.levels[li].name.clone();
    if spec.levels[li].goal_x.is_none() && spec.levels[li].goal_under.is_none() {
        return p.not_checked(format!(
            "level {name} has no goal, and this check plays a tile stage to its goal"
        ));
    }
    let r = match reach::reach(spec, li, src) {
        Ok(r) => r,
        Err(e) => {
            return p.not_checked(format!(
                "the reach search can't play it ({e}): it moves a platformer's player by its walks and jumps"
            ));
        }
    };
    p.measured
        .insert("stands_alone".into(), json!(r.stands.len()));
    p.measured.insert("moves_alone".into(), json!(r.moves));
    if r.goal_path.is_none() {
        p.lines.push(format!(
            "it could NOT be finished: the player alone, by every walk and jump the search tries, never reaches the goal ({} places to stand found, {} moves played)",
            r.stands.len(),
            r.moves
        ));
        p.finished = json!({"finished": false, "checked": true, "reachable": false,
            "stands": r.stands.len()});
        return p;
    }
    p.lines.push(format!(
        "the player alone reaches the goal by its walks and jumps ({} places to stand on the way there)",
        r.goal_path.as_ref().map_or(0, Vec::len)
    ));
    p.finished = match reach::playthrough(spec, li, src) {
        Ok(Some(t)) => {
            let frame = t.buttons.len();
            p.measured.insert("run_frames".into(), json!(frame));
            p.measured.insert("run_searched".into(), json!(t.searched));
            p.lines.push(format!(
                "it can be finished: a run with every enemy in, from the stage's start, reached the goal at frame {frame} ({} places searched)",
                t.searched
            ));
            json!({"finished": true, "checked": true, "frame": frame, "searched": t.searched})
        }
        Ok(None) => {
            p.lines.push(format!(
                "it could NOT be finished: the goal is reached by the player alone, but no run with the enemies in was found in {} places",
                reach::PLAYTHROUGH_LIMIT
            ));
            json!({"finished": false, "checked": true, "reachable": true})
        }
        Err(e) => {
            return p.not_checked(format!(
                "the goal is reached by the player alone, but the run with the enemies in couldn't be played: {e}"
            ));
        }
    };
    p
}

/// How a game made here is built, when its folder's template says it is
/// (`[source] how = "build"` or `"authored"`, as Moss Hop's): its stages
/// have no original's layout to be kept apart from.
fn made_here(dir: &Path) -> Option<String> {
    let t: toml::Table = fs::read_to_string(dir.join("template.toml"))
        .ok()?
        .parse()
        .ok()?;
    let how = t.get("source")?.get("how")?.as_str()?;
    matches!(how, "build" | "authored").then(|| how.to_owned())
}

/// `gp level verify GAME [--level L]`.
pub(crate) fn verify_cmd(
    game: &Path,
    level: Option<&str>,
    runs: u32,
    frames: u32,
    module: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    kit_first(game, paths)?;
    let g = Game::open(game, paths)?;
    let level = level_of(&g, level)?;
    if let Some(why) = arcade_level(&g, &level) {
        bail!("{why}: gp level render draws it");
    }
    let refs = StageRefs::of(&g.model, &level).map_err(|e| anyhow!("{e}"))?;
    let mut data = Data::of(&g.lifted);
    let mut original = Data::of(&g.original);
    let c = check(&g, &refs, &mut data, &g.model, &mut original).map_err(|e| anyhow!("{e}"))?;
    let mut lines = vec![format!("{} level {level}", g.name)];
    let mut ok = true;
    if c.broke.is_empty() {
        lines.push("the kit's rules: kept".into());
    } else {
        ok = false;
        for f in &c.broke {
            lines.push(format!("BROKEN {}: {}", f.rule, f.said));
        }
    }
    let src = source(&g, &data, &g.model, paths, module)?;
    let played = if refs.is_tile_stage() {
        play_tiles(&src, c.li)
    } else {
        play_girders(&g, &c, &src, runs, frames)
    };
    lines.extend(played.lines.iter().cloned());
    let not_checked = played.not_checked.clone();
    let mut finished = played.finished.clone();
    // The line that says whether it can be finished, for gp validate.
    if let Some(l) =
        played.lines.iter().rev().find(|l| {
            l.starts_with("it can be finished") || l.starts_with("it could NOT be finished")
        })
    {
        finished["said"] = json!(l);
    }
    if played.finished["finished"] != true {
        ok = false;
    }
    let s = played.measured.clone();
    let measures = g
        .kit()
        .filter(|_| !refs.is_tile_stage())
        .map(|k| against_kit(k, &s))
        .unwrap_or_default();
    let mut out_of = 0;
    for mm in &measures {
        let state = match mm["ok"].as_bool() {
            Some(true) => "ok  ",
            Some(false) => {
                out_of += 1;
                "OUT "
            }
            None => "    ",
        };
        lines.push(format!(
            "{state}{}: {} (target {}, within {})",
            mm["name"].as_str().unwrap_or(""),
            mm["have"],
            mm["target"],
            mm["within"].as_str().unwrap_or("")
        ));
    }
    let orig = match made_here(&g.dir) {
        Some(how) => json!({"apart": true, "held": false, "said": format!(
            "{} is a game made here (its template says how = \"{how}\"), so there is no original's layout to keep this stage apart from",
            g.name
        )}),
        None => {
            originality(&g, &refs, &c, &mut Data::of(&g.original)).map_err(|e| anyhow!("{e}"))?
        }
    };
    if orig["apart"] == false {
        ok = false;
    }
    lines.push(orig["said"].as_str().unwrap_or("").to_owned());
    if !g.own_assets && orig["held"] != false {
        lines.push("this is the game's own stage as the person's ROM has it (the folder has no assets of its own), so it is the original's layout".into());
    }
    for n in &played.notes {
        lines.push(format!("note: {n}"));
    }
    if !c.by_hand.is_empty() {
        lines.push(format!(
            "kept by hand (the kit states them; these checks don't hold a stage to them): {}",
            c.by_hand.len()
        ));
    }
    let file = picture_path(paths, &g, &level, false);
    draw(&c, &g.model, &file)?;
    lines.push(format!("picture by kind (no ROM art): {}", file.display()));
    let failed = !c.broke.is_empty() || orig["apart"] == false || finished["finished"] == false;
    lines.push(if ok {
        format!(
            "PASS: the stage keeps the kit's rules, can be finished{}{}",
            if orig["held"] == false { " (a game made here has no original's layout to be apart from)" } else { " and is apart from the original's layout" },
            if out_of > 0 { format!("; {out_of} play measures are outside their tolerances, which says what to move, not that it can't be played") } else { String::new() }
        )
    } else if failed {
        format!(
            "FAIL: see above{}",
            if not_checked.is_some() { "; and whether it can be finished was NOT CHECKED" } else { "" }
        )
    } else {
        "NOT CHECKED: nothing above failed, but this check couldn't play the stage to its end, so nothing here says it can be finished".to_owned()
    });
    Ok(Outcome {
        json: json!({
            "game": g.name, "level": level, "ok": ok, "checked": not_checked.is_none(),
            "breaks": c.broke, "finished": finished, "measures": measures,
            "measured": s, "original": orig, "notes": played.notes,
            "kept_by_hand": c.by_hand, "picture": file,
        }),
        text: lines.join("\n"),
        exit: u8::from(!ok),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_parse_and_a_fixed_list_is_refused_in_words() {
        assert_eq!(
            Op::parse("move ladder 6 x=136 dy=-2").unwrap(),
            Op::MoveLadder {
                row: 6,
                set: BTreeMap::from([("x".into(), 136), ("dy".into(), -2)])
            }
        );
        assert_eq!(
            Op::parse("move spawn hammer#1 x=0x28 y=100").unwrap(),
            Op::MoveSpawn {
                entity: "hammer".into(),
                index: 1,
                x: Some(40),
                y: Some(100),
                dx: 0,
                dy: 0
            }
        );
        assert_eq!(
            Op::parse("set row play_heights 2 1,2,3").unwrap(),
            Op::SetRow {
                slot: "play_heights".into(),
                row: 2,
                values: vec![1, 2, 3]
            }
        );
        assert_eq!(
            Op::parse("make ladder 3 full").unwrap(),
            Op::MakeLadder { row: 3, full: true }
        );
        let e = Op::parse("remove ladder 3").unwrap_err();
        assert!(
            e.contains("fixed size") && e.contains("move ladder N"),
            "{e}"
        );
        let e = Op::parse("place girder x=3").unwrap_err();
        assert!(e.contains("girders"), "{e}");
        assert!(
            Op::parse("move ladder 3")
                .unwrap_err()
                .contains("changes nothing")
        );
        assert!(
            Op::parse("move ladder 3 z=1")
                .unwrap_err()
                .contains("isn't one of")
        );
        assert!(
            Op::parse("jump around")
                .unwrap_err()
                .contains("The operations")
        );
    }

    // Made-up bytes, in the two formats a lift writes.
    const ROWS: &str = "# Rows of 3 bytes each.\n# Lifted from a made-up game.\nschema = 1\nasset = \"lads\"\nrows = [\n  [0x10, 0x20, 0x00], # 0\n  [0x30, 0x40, 0x04], # 1\n]\n";
    const TABLE: &str = "# A table, one entry per row.\nschema = 1\nasset = \"tab\"\n\n# 0\n[[row]]\nbyte = 5\n\n# 1\n[[row]]\nbyte = 6\n\n# 2\n[[row]]\nbyte = 254\n\n# 3\n[[row]]\nbyte = 0\n\n# 4\n[[row]]\nbyte = 9\n";

    // A made-up level grid, as gp_assets::text::write_level writes one.
    const GRID: &str = "# A level, one character per metatile.\nschema = 1\nasset = \"level\"\n# 4 metatiles wide and 2 high. The characters, from the map's legend:\n#   .  0 air\n#   #  1 ground\n#   G  2 goal\nrows = [\n  \"..G.\", # 0\n  \"####\", # 1\n]\n";

    #[test]
    fn a_level_grid_reads_as_metatile_numbers_and_writes_back_the_same() {
        let mut g = SlotData::parse("level.toml", GRID).unwrap();
        assert_eq!(g.format, Format::Grid(vec!['.', '#', 'G']));
        assert_eq!(g.rows, vec![vec![0, 0, 2, 0], vec![1, 1, 1, 1]]);
        assert_eq!(g.text(), GRID);
        g.rows[0] = vec![1, 0, 0, 2];
        assert!(g.text().contains("\"#..G\", # 0"));
        assert!(g.holds(2, "a value").is_ok());
        assert!(g.holds(3, "a value").unwrap_err().contains("from 0 to 2"));
        let bad = GRID.replace("..G.", "..X.");
        assert!(
            SlotData::parse("level.toml", &bad)
                .unwrap_err()
                .contains("`X`")
        );
    }

    #[test]
    fn a_slot_reads_and_writes_back_the_same() {
        let r = SlotData::parse("lads.toml", ROWS).unwrap();
        assert_eq!(r.rows, vec![vec![16, 32, 0], vec![48, 64, 4]]);
        assert_eq!(r.text(), ROWS);
        let t = SlotData::parse("tab.toml", TABLE).unwrap();
        assert_eq!(t.format, Format::Table(vec!["byte".into()]));
        assert_eq!(t.rows.len(), 5);
        let again = SlotData::parse("tab.toml", &t.text()).unwrap();
        assert_eq!(again.rows, t.rows);
        assert!(t.text().starts_with(
            "# A table, one entry per row.\nschema = 1\nasset = \"tab\"\n\n# 0\n[[row]]\nbyte = 5\n"
        ));
    }

    #[test]
    fn a_list_with_an_end_mark_keeps_its_room() {
        let mut base = Lifted {
            files: BTreeMap::new(),
        };
        base.files.insert("tab.toml".into(), TABLE.into());
        let mut d = Data::of(&base);
        d.slots
            .insert("tab".into(), SlotData::parse("tab.toml", TABLE).unwrap());
        let l = ListRef::of("tab.byte", Some(0), 2, Some(254));
        // Rows of 2 from 0 up to the mark: [5, 6].
        let g_rows = |d: &mut Data| {
            let s = d.slots.get("tab").unwrap();
            s.rows.iter().map(|r| r[0]).collect::<Vec<_>>()
        };
        let col = g_rows(&mut d);
        assert_eq!(col, vec![5, 6, 254, 0, 9]);
        // Write none: the mark moves to the start, the freed bytes are 0.
        let (model, map) = made_up();
        let at = (&model, &map);
        assert_eq!(d.read(at, &l).unwrap(), vec![vec![5, 6]]);
        d.write(at, &l, &[], 1).unwrap();
        assert_eq!(g_rows(&mut d), vec![254, 0, 0, 0, 9]);
        d.write(at, &l, &[vec![7, 8]], 1).unwrap();
        assert_eq!(g_rows(&mut d), vec![7, 8, 254, 0, 9]);
        let e = d.write(at, &l, &[vec![1, 1], vec![2, 2]], 1).unwrap_err();
        assert!(e.contains("room for 1"), "{e}");
    }

    /// Moss Hop's model (any model does: the slots are loaded already) and
    /// an empty map.
    fn made_up() -> (Model, AssetMap) {
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop/model.toml");
        let model = Model::parse(&fs::read_to_string(f).unwrap()).unwrap();
        let map = AssetMap {
            schema: 1,
            source: "made-up".into(),
            title: None,
            sha256: String::new(),
            assets: Vec::new(),
            content: Vec::new(),
            code: None,
        };
        (model, map)
    }

    #[test]
    fn only_a_copy_in_the_home_is_editable() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let games = t.path().join("games");
        let copy = home.join("work").join("mcp").join("mine");
        fs::create_dir_all(&copy).unwrap();
        fs::create_dir_all(games.join("rooftop")).unwrap();
        assert!(editable_in(&copy, &home, Some(&games)).is_ok());
        let e = editable_in(&games.join("rooftop"), &home, Some(&games)).unwrap_err();
        assert!(e.contains("games checkout's original"), "{e}");
        let e = editable_in(&t.path().join("elsewhere"), &home, Some(&games)).unwrap_err();
        assert!(e.contains("isn't a copy in this home"), "{e}");
        // A published package as gp game fetch writes it.
        let fetched = home.join("work").join("fetched");
        fs::create_dir_all(fetched.join("lifted")).unwrap();
        fs::write(fetched.join("package.toml"), "").unwrap();
        let e = editable_in(&fetched, &home, Some(&games)).unwrap_err();
        assert!(e.contains("published package"), "{e}");
        // This repo is a git work tree: its templates are originals.
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let e = editable_in(here, &home, None).unwrap_err();
        assert!(e.contains("git work tree"), "{e}");
    }

    #[test]
    fn measures_are_held_to_the_kit_s_tolerances() {
        assert_eq!(numbers_in("13, 4 (0.31)"), vec![13.0, 4.0]);
        assert_eq!(numbers_in("8 px each"), vec![8.0]);
        let rules: Rules = toml::from_str(
            r#"format = "0.1.0"
[[measure]]
name = "girders"
what = "Girders, and the top platform"
target = "6 + 1"
within = "1"
[[measure]]
name = "route_frames"
what = "The route"
target = 1000
within = "10%"
[[measure]]
name = "gaps"
what = "Gaps"
target = [30, 30]
within = "8 px each"
[[measure]]
name = "goal_height"
what = "The goal"
target = 186
within = "not set"
"#,
        )
        .unwrap();
        let s: BTreeMap<String, Value> = [
            ("girders", json!(6)),
            ("platforms", json!(1)),
            ("route_frames", json!(1150)),
            ("gaps", json!([31, 20])),
            ("goal_height", json!(186)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let k = against_kit(&rules, &s);
        assert_eq!(k[0]["ok"], json!(true));
        assert_eq!(k[1]["ok"], json!(false), "1150 is past 1000's 10%");
        assert_eq!(k[2]["ok"], json!(false), "20 is 10 from 30");
        assert_eq!(k[3]["ok"], Value::Null, "no tolerance");
    }

    #[test]
    fn a_picture_string_keeps_what_isnt_a_cell_and_counts_its_bytes() {
        // A made-up string: a palette set, an attribute row and three cell
        // updates (a repeat, a list, one vertical).
        let text = "schema = 1\n\n[[string]]\nname = \"s\"\nupdates = [\n  { ppu = 0x3F00, tiles = [0x0F, 0x01] },\n  { ppu = 0x23C0, repeat = 8, tile = 0xFF },\n  { ppu = 0x2040, repeat = 6, tile = 0x30 },\n  { ppu = 0x2060, tiles = [0x31, 0x32, 0x33] },\n  { ppu = 0x2045, vertical = true, repeat = 4, tile = 0x3F },\n]\n\n[[string]]\nname = \"t\"\nupdates = [\n  { ppu = 0x2000, repeat = 2, tile = 0x01 },\n]\n";
        let (head, kept, tail) = split_updates(text, "s").unwrap();
        assert!(head.ends_with("updates = [\n"));
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert!(kept[0].contains("0x3F00") && kept[1].contains("0x23C0"));
        assert!(tail.starts_with("]\n\n[[string]]\nname = \"t\""));
        let p: gp_assets::ppu::PpuStrings = toml::from_str(
            "name = \"pics\"\nregion = \"prg\"\noffset = 0\nsize = 64\nentry = [{ name = \"s\" }, { name = \"t\" }]\n",
        )
        .unwrap();
        // Palettes 3 + 2, attributes 3 + 1.
        assert_eq!(kept_bytes(&p, text, "s").unwrap(), 9);
        // Cells: 4 + (3 + 3) + 4.
        assert_eq!(nametable_bytes(&p, text, "s").unwrap(), 14);
    }

    /// Moss Hop's model on a made-up tile stage, 32 by 15 cells: sky over
    /// three rows of ground with a gap of `gap` columns from column 12, and
    /// the hero's start at `start`. Nothing in it is the game's level.
    fn tile_source(gap: usize, start: (i64, i64)) -> crate::hosted::Source {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut text = fs::read_to_string(root.join("templates/mosshop/model.toml")).unwrap();
        text = text.replacen(
            "[[levels.main.spawn]]\nentity = \"player\"\nx = 24\ny = 176",
            &format!(
                "[[levels.main.spawn]]\nentity = \"player\"\nx = {}\ny = {}",
                start.0, start.1
            ),
            1,
        );
        let model = Model::parse(&text).unwrap();
        let mut metatiles = String::from("schema = 1\nasset = \"metatiles\"\n");
        for (i, (solid, kind)) in [
            (0, 0),
            (1, 0),
            (1, 0),
            (1, 0),
            (0, 0),
            (0, 0),
            (1, 2),
            (1, 0),
            (0, 5),
        ]
        .into_iter()
        .enumerate()
        {
            write!(
                metatiles,
                "\n[[metatile]]\ntiles = [{i}, {i}, {i}, {i}]\nsolid = {solid}\nkind = {kind}\n"
            )
            .unwrap();
        }
        let rows: Vec<String> = (0..15)
            .map(|r| {
                let row: String = (0..32)
                    .map(|c| match r {
                        _ if (12..12 + gap).contains(&c) && r >= 12 => '.',
                        12 => 'M',
                        13 | 14 => 'S',
                        _ => '.',
                    })
                    .collect();
                format!("  \"{row}\",")
            })
            .collect();
        let mut files = BTreeMap::new();
        files.insert(
            "assets.toml".to_owned(),
            fs::read_to_string(root.join("assets/mosshop.toml")).unwrap(),
        );
        files.insert("metatiles.toml".to_owned(), metatiles);
        files.insert(
            "slugs.toml".to_owned(),
            "schema = 1\nasset = \"slugs\"\n\n[[row]]\nstart = 64\nend = 96\n\n[[row]]\nstart = 160\nend = 192\n".to_owned(),
        );
        files.insert(
            "level.toml".to_owned(),
            format!(
                "schema = 1\nasset = \"level\"\nrows = [\n{}\n]\n",
                rows.join("\n")
            ),
        );
        let bundle = gp_interp::Bundle::new(&model, Lifted { files });
        crate::hosted::Source::new(bundle, None).unwrap()
    }

    /// A tile stage is played to its goal by the reach search and the
    /// playthrough with the slugs in, and one whose gap no jump clears
    /// can't be finished; neither is NOT CHECKED.
    #[test]
    fn a_tile_stage_is_played_to_its_goal() {
        let near = play_tiles(&tile_source(3, (24, 176)), 0);
        assert_eq!(near.finished["finished"], json!(true), "{:?}", near.lines);
        assert!(near.finished["frame"].as_u64().unwrap() > 0);
        assert!(near.not_checked.is_none());
        assert!(
            near.lines
                .iter()
                .any(|l| l.starts_with("it can be finished")),
            "{:?}",
            near.lines
        );
        // A running jump carries 80 pixels: six columns (96) aren't cleared.
        let far = play_tiles(&tile_source(6, (24, 176)), 0);
        assert_eq!(far.finished["finished"], json!(false), "{:?}", far.lines);
        assert_eq!(far.finished["checked"], json!(true));
        assert!(far.not_checked.is_none());
    }

    /// The hero's start over the gap breaks the start rule, by the model's
    /// cells; over ground it keeps it.
    #[test]
    fn a_tile_stage_start_needs_ground_under_it() {
        let src = tile_source(3, (24, 176));
        assert!(tile_breaks(&src.spec, 0).is_empty());
        let src = tile_source(3, (12 * 16 + 8, 176));
        let b = tile_breaks(&src.spec, 0);
        assert_eq!(b.len(), 1, "{b:?}");
        assert_eq!(b[0].0, "start");
        assert!(b[0].1.contains("it would fall"), "{}", b[0].1);
    }
}
