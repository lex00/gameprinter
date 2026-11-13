//! The grid (game-printer's serve/grid.rs; game-printer's docs/show.md,
//! "The grid"): the game's clips laid out as mechanics by contexts, each
//! cell with the state of the clips covering it, and the cell the next
//! clip should fill. REFINE draws it as a heatmap (mechanic/refine/grid.js,
//! game-printer's editor/src/GridHeat.tsx and gridfold.ts).
//!
//! The rows are the model's parts: each mechanic the player, a level or the
//! game carries (the mechanic catalog's), each entity kind and each event a
//! mechanic emits. The columns are contexts: a level and the kind of place
//! a clip starts at there. A cell's state comes from the clips covering it,
//! worst first: needs you (a question for a person on one of its clips),
//! being fixed (a job on one), differs, close (90% or more of its frames
//! together), waiting to be scored, matches; else empty, or no start (no
//! clip starts there yet).
//!
//! What gameprinter's layout changed:
//!
//! - The clips are the game's corpus clips (crate::serve::generated) and
//!   their scores the background work's on the model as it stands
//!   (crate::serve::background). A corpus clip starts from a save point
//!   with a level, not from a start the tool classified (a pit, a wall, an
//!   enemy), so a column is the clip's level and the mechanic session it
//!   came from (`walk` of `dk.walk.001`): the place a person recorded that
//!   mechanic at. A clip with no level is in the model's start level.
//! - A clip covers its column for the mechanic rows its session or name
//!   names (a word of the mechanic's name: `jump` for `platformer.jump`),
//!   the entity rows its score finds wrong, and else the player's row.
//! - game-printer's option rows (the fixes a mutant switches off) and the
//!   doubt from scoring the mutants on each clip are left out: the
//!   background work here scores the model alone, so a clip's doubt is
//!   1000 when it doesn't match and 0 when it does.
//! - game-printer's grid made the next clip itself and kept a few in
//!   flight. gameprinter makes no clips from recordings yet (a game's clips
//!   come from its mechanic sessions through `gp corpus import`), so the
//!   grid picks the next cell, by game-printer's priorities, and says it as
//!   the clip to record next. "Clips in flight" and the mode are kept for
//!   when it makes them.
//!
//! The state is `$GP_HOME/work/serve/GAME/grid/state.json`.
//!
//! | Endpoint | What |
//! |---|---|
//! | `GET /api/capture/{run}/grid` | The rows, the columns, each cell with its state, clips and doubt, the clips in flight (none) and the next pick |
//! | `GET`, `POST /api/capture/{run}/grid/in-flight` | `{in_flight, max}`; POST `{in_flight}` sets it |
//! | `GET`, `POST /api/capture/{run}/grid/mode` | `{mode}`; POST `{mode}`: `grid` or `batch` |

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::{Value, json};

use super::game::{Run, read_json, run_dir, write_json};
use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

/// Clips in flight unless the run says.
const DEFAULT_IN_FLIGHT: u64 = 3;
/// The most clips in flight.
const MAX_IN_FLIGHT: u64 = 12;
/// A clip this close to the original (in thousandths of its frames
/// together) is close, not differing.
const CLOSE_PERMILLE: u64 = 900;
/// An empty cell's priority before its neighbours, and more for a column
/// nothing covers yet.
const EMPTY_PRIOR: u64 = 250;
const FRESH_COLUMN: u64 = 50;
/// The kinds of place in their order across a level.
const PLACES: &[&str] = &[
    "start",
    "castle",
    "underwater",
    "enemy",
    "pit",
    "wall",
    "block",
    "pipe",
    "lift",
    "air",
];

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/capture/{run}/grid",
            about: "The grid of mechanics by contexts: each cell's state, clips and doubt, and the next cell to fill",
            handler: state_route,
        },
        Route {
            path: "/api/capture/{run}/grid/in-flight",
            about: "{in_flight, max}: how many clips the grid keeps in flight; POST {in_flight} sets it",
            handler: in_flight_route,
        },
        Route {
            path: "/api/capture/{run}/grid/mode",
            about: "{mode}: grid or batch; POST {mode} sets it",
            handler: mode_route,
        },
    ]
}

fn file(run: &Run) -> PathBuf {
    run.dir.join("grid").join("state.json")
}

/// A row: a part of the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    /// `mechanic:platformer.jump`, `entity:barrel`, `event:enemy.stomped`.
    pub id: String,
    /// `mechanic`, `entity` or `event`.
    pub kind: &'static str,
    pub label: String,
    /// The mechanic it is part of (a mechanic row's own), or the entity.
    pub group: String,
    /// The levels it can be in; none for every one.
    levels: Option<BTreeSet<String>>,
}

/// A column: a level and a kind of place in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Col {
    /// `m75/climb-ladder`.
    pub id: String,
    pub level: String,
    pub level_name: String,
    pub place: String,
    /// The clips starting here.
    pub starts: Vec<String>,
}

/// One clip as the grid reads it: its ID, its level, the session it came
/// from and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Clip {
    pub id: String,
    pub level: String,
    pub session: String,
    pub name: String,
}

pub(crate) struct Grid {
    pub rows: Vec<Row>,
    pub cols: Vec<Col>,
    /// The player's row.
    player: Option<String>,
}

fn words_of(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).replace('_', " ")
}

/// The value at a dotted path of the model's tree.
fn at<'a>(tree: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    path.split('.').try_fold(tree, |v, k| v.get(k))
}

/// A level's own kind of place, from its palettes' name: a castle or
/// underwater level's start is that place.
fn flavour(tree: &toml::Value, level: &str) -> Option<&'static str> {
    let p = at(tree, &format!("levels.{level}.present.palettes"))?.as_str()?;
    if p.contains("castle") {
        Some("castle")
    } else if p.contains("water") {
        Some("underwater")
    } else {
        None
    }
}

/// The grid of `model` over `clips`.
pub(crate) fn derive(
    model: &gp_model::Model,
    tree: &toml::Value,
    exts: &gp_sim::Exts,
    clips: &[Clip],
) -> Grid {
    let catalog = gp_interp::mechanics::catalog(model, exts);
    let player = gp_interp::scenario::player(model).unwrap_or_default();
    let mine = format!("entities.{player}.");
    let spawn_levels = |kind: &str| -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        if let Some(t) = tree.get("levels").and_then(toml::Value::as_table) {
            for (l, v) in t {
                let spawns = v.get("spawn").and_then(toml::Value::as_array);
                if spawns.is_some_and(|a| {
                    a.iter()
                        .any(|s| s.get("entity").and_then(toml::Value::as_str) == Some(kind))
                }) {
                    out.insert(l.clone());
                }
            }
        }
        out
    };
    let mut own_rows: Vec<Row> = Vec::new();
    let mut level_rows: Vec<Row> = Vec::new();
    let mut game_rows: Vec<Row> = Vec::new();
    let mut emitters: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for m in &catalog.mechanics {
        let row = |levels| Row {
            id: format!("mechanic:{}", m.name),
            kind: "mechanic",
            label: words_of(m.name),
            group: m.name.to_owned(),
            levels,
        };
        let mut by: BTreeSet<String> = BTreeSet::new();
        for p in &m.used_by {
            if let Some(rest) = p.strip_prefix("entities.") {
                let kind = rest.split('.').next().unwrap_or(rest);
                by.insert(if kind == player {
                    "*".into()
                } else {
                    kind.to_owned()
                });
            } else {
                by.insert("*".into());
            }
        }
        for e in &m.emits {
            emitters
                .entry((*e).to_owned())
                .or_default()
                .extend(by.iter().cloned());
        }
        if m.used_by.iter().any(|p| p.starts_with(&mine)) {
            own_rows.push(row(None));
        } else if m.carrier == "level" {
            let levels: BTreeSet<String> = m
                .used_by
                .iter()
                .filter_map(|p| p.strip_prefix("levels."))
                .map(|r| r.split('.').next().unwrap_or(r).to_owned())
                .collect();
            level_rows.push(row(Some(levels)));
        } else if m.carrier == "game" {
            game_rows.push(row(None));
        }
    }
    let mut rows: Vec<Row> = own_rows
        .into_iter()
        .chain(level_rows)
        .chain(game_rows)
        .collect();
    let player_row = (!player.is_empty()).then(|| format!("entity:{player}"));
    if let Some(id) = &player_row {
        rows.push(Row {
            id: id.clone(),
            kind: "entity",
            label: player.replace('_', " "),
            group: player.clone(),
            levels: None,
        });
    }
    for kind in model.entities.keys().filter(|k| **k != player) {
        rows.push(Row {
            id: format!("entity:{kind}"),
            kind: "entity",
            label: kind.replace('_', " "),
            group: kind.clone(),
            levels: Some(spawn_levels(kind)),
        });
    }
    for (e, by) in &emitters {
        let levels = if by.contains("*") {
            None
        } else {
            Some(by.iter().flat_map(|k| spawn_levels(k)).collect())
        };
        rows.push(Row {
            id: format!("event:{e}"),
            kind: "event",
            label: e.replace(['.', '_'], " "),
            group: String::new(),
            levels,
        });
    }

    // Columns: each level a clip starts in, by its first clip, and the
    // places there.
    let first_level = at(tree, "model.start_level")
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            tree.get("levels")
                .and_then(toml::Value::as_table)
                .and_then(|t| t.keys().next().cloned())
        })
        .unwrap_or_else(|| "game".into());
    let mut levels: Vec<String> = Vec::new();
    let mut by_col: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for c in clips {
        let level = if c.level.is_empty() {
            first_level.clone()
        } else {
            c.level.clone()
        };
        if !levels.contains(&level) {
            levels.push(level.clone());
        }
        by_col
            .entry((level, place_of(tree, c)))
            .or_default()
            .push(c.id.clone());
    }
    let rank = |p: &str| PLACES.iter().position(|x| *x == p).unwrap_or(PLACES.len());
    let mut cols: Vec<Col> = Vec::new();
    for l in &levels {
        let name = at(tree, &format!("levels.{l}.name"))
            .and_then(toml::Value::as_str)
            .map_or_else(|| l.clone(), str::to_owned);
        let mut here: Vec<(&String, &Vec<String>)> = by_col
            .iter()
            .filter(|((lv, _), _)| lv == l)
            .map(|((_, p), s)| (p, s))
            .collect();
        here.sort_by_key(|(p, _)| (rank(p), (*p).clone()));
        for (p, s) in here {
            cols.push(Col {
                id: format!("{l}/{p}"),
                level: l.clone(),
                level_name: name.clone(),
                place: p.clone(),
                starts: s.clone(),
            });
        }
    }
    Grid {
        rows,
        cols,
        player: player_row,
    }
}

/// A clip's kind of place: its mechanic session, or a castle or underwater
/// level's own flavour.
fn place_of(tree: &toml::Value, c: &Clip) -> String {
    if !c.level.is_empty()
        && let Some(f) = flavour(tree, &c.level)
    {
        return f.to_owned();
    }
    if c.session.is_empty() || c.session == "corpus" {
        "start".into()
    } else {
        c.session.clone()
    }
}

impl Grid {
    fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }

    /// Whether the row can be in the column, by the model alone.
    fn applies(&self, r: &Row, c: &Col) -> bool {
        r.levels.as_ref().is_none_or(|l| l.contains(&c.level))
    }

    /// The rows a clip covers: the mechanics its session or name names,
    /// the entities its score finds wrong, else the player's.
    fn rows_of(&self, clip: &Clip, score: &Value) -> Vec<String> {
        let words: BTreeSet<String> = format!("{} {}", clip.session, clip.name)
            .to_lowercase()
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|w| w.len() >= 3)
            .map(str::to_owned)
            .collect();
        let mut out: Vec<String> = self
            .rows
            .iter()
            .filter(|r| r.kind == "mechanic")
            .filter(|r| {
                r.group
                    .rsplit('.')
                    .next()
                    .unwrap_or(&r.group)
                    .split('_')
                    .any(|w| words.contains(w))
            })
            .map(|r| r.id.clone())
            .collect();
        for t in score["tracks"].as_array().into_iter().flatten() {
            if let Some(path) = t["path"].as_str()
                && let Some(rest) = path.strip_prefix("entities.")
            {
                let kind = rest.split(['.', '[']).next().unwrap_or(rest);
                let id = format!("entity:{kind}");
                if self.row(&id).is_some() {
                    out.push(id);
                }
            }
        }
        if out.is_empty()
            && let Some(p) = &self.player
        {
            out.push(p.clone());
        }
        out.sort();
        out.dedup();
        out
    }
}

/// A cell's state, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CellState {
    NoStart,
    Empty,
    Exact,
    Waiting,
    Close,
    Differs,
    Fixing,
    NeedsYou,
}

impl CellState {
    fn name(self) -> &'static str {
        match self {
            Self::NoStart => "no_start",
            Self::Empty => "empty",
            Self::Exact => "exact",
            Self::Waiting => "waiting",
            Self::Close => "close",
            Self::Differs => "differs",
            Self::Fixing => "fixing",
            Self::NeedsYou => "needs_you",
        }
    }
}

/// A cell of the grid as the page reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cell {
    pub row: String,
    pub col: String,
    pub state: CellState,
    pub clips: Vec<String>,
    /// How much doubt, in thousandths.
    pub doubt: u64,
    /// The pick's priority, in thousandths; none when it can't be picked.
    pub priority: Option<u64>,
}

/// How the clips stand: their scores on the model as it stands, the clips
/// with a question for a person, and the clips a job has now.
pub(crate) struct Standing<'a> {
    pub scores: &'a Value,
    pub model: &'a str,
    pub needs: &'a BTreeSet<String>,
    pub working: &'a BTreeSet<String>,
}

impl Standing<'_> {
    fn stand(&self, id: &str) -> (CellState, u64) {
        let s = &self.scores[id];
        let s = (s["model"].as_str() == Some(self.model)).then_some(s);
        let mut state = match s {
            None => CellState::Waiting,
            Some(s) if !s["error"].is_null() => CellState::NoStart,
            Some(s) if s["exact"] == json!(true) => CellState::Exact,
            Some(s) if s["permille"].as_u64().unwrap_or(0) >= CLOSE_PERMILLE => CellState::Close,
            Some(_) => CellState::Differs,
        };
        if self.needs.contains(id) {
            state = CellState::NeedsYou;
        } else if self.working.contains(id)
            && matches!(state, CellState::Close | CellState::Differs)
        {
            state = CellState::Fixing;
        }
        let wrong = s.is_some_and(|s| s["exact"] != json!(true) && s["error"].is_null());
        (state, u64::from(wrong) * 1000)
    }
}

/// Every cell the model allows or a clip covers, with its state, doubt and
/// the pick's priority (game-printer's `board`).
pub(crate) fn board(grid: &Grid, clips: &[Clip], st: &Standing) -> Vec<Cell> {
    let col_of: BTreeMap<&str, &str> = grid
        .cols
        .iter()
        .flat_map(|c| c.starts.iter().map(move |s| (s.as_str(), c.id.as_str())))
        .collect();
    type Stands = Vec<(String, CellState, u64)>;
    let mut by_cell: BTreeMap<(String, String), Stands> = BTreeMap::new();
    for c in clips {
        let Some(col) = col_of.get(c.id.as_str()) else {
            continue;
        };
        let (state, doubt) = st.stand(&c.id);
        for r in grid.rows_of(c, &st.scores[&c.id]) {
            by_cell
                .entry((r, (*col).to_owned()))
                .or_default()
                .push((c.id.clone(), state, doubt));
        }
    }
    let mut explored: BTreeSet<String> = BTreeSet::new();
    for (_, c) in by_cell.keys() {
        explored.insert(c.clone());
    }
    let mut cells: Vec<Cell> = Vec::new();
    for r in &grid.rows {
        for c in &grid.cols {
            let here = by_cell.get(&(r.id.clone(), c.id.clone()));
            if here.is_none() && !grid.applies(r, c) {
                continue;
            }
            let (state, ids, doubt) = match here {
                Some(v) => (
                    v.iter().map(|x| x.1).max().unwrap_or(CellState::Empty),
                    v.iter().map(|x| x.0.clone()).collect(),
                    v.iter().map(|x| x.2).sum::<u64>() / v.len() as u64,
                ),
                None if c.starts.is_empty() => (CellState::NoStart, Vec::new(), 0),
                None => (CellState::Empty, Vec::new(), 0),
            };
            cells.push(Cell {
                row: r.id.clone(),
                col: c.id.clone(),
                state,
                clips: ids,
                doubt,
                priority: None,
            });
        }
    }
    // Priorities: a covered cell's doubt shared by its clips; an empty
    // cell's prior and half its covered neighbours' doubt (the same row
    // elsewhere in the level and at the same place in the levels either
    // side, and the other rows of its mechanic in the same column).
    let idx: BTreeMap<(&str, &str), usize> = cells
        .iter()
        .enumerate()
        .map(|(i, c)| ((c.row.as_str(), c.col.as_str()), i))
        .collect();
    let col_at: BTreeMap<&str, usize> = grid
        .cols
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();
    let mut pri: Vec<Option<u64>> = vec![None; cells.len()];
    for (i, cell) in cells.iter().enumerate() {
        let Some(ci) = col_at.get(cell.col.as_str()).copied() else {
            continue;
        };
        let col = &grid.cols[ci];
        if col.starts.is_empty() || cell.clips.iter().any(|c| st.working.contains(c)) {
            continue;
        }
        if !cell.clips.is_empty() {
            pri[i] = Some(cell.doubt / (1 + cell.clips.len() as u64));
            continue;
        }
        let mut near: Vec<u64> = Vec::new();
        let mut look = |r: &str, c: &str| {
            if let Some(&j) = idx.get(&(r, c))
                && !cells[j].clips.is_empty()
            {
                near.push(cells[j].doubt);
            }
        };
        for other in grid
            .cols
            .iter()
            .filter(|o| o.level == col.level && o.id != col.id)
        {
            look(&cell.row, &other.id);
        }
        for d in [ci.wrapping_sub(1), ci + 1] {
            if let Some(o) = grid.cols.get(d)
                && o.place == col.place
            {
                look(&cell.row, &o.id);
            }
        }
        if let Some(r) = grid.row(&cell.row).filter(|r| !r.group.is_empty()) {
            for o in grid
                .rows
                .iter()
                .filter(|o| o.group == r.group && o.id != r.id)
            {
                look(&o.id, &cell.col);
            }
        }
        let mean = if near.is_empty() {
            0
        } else {
            near.iter().sum::<u64>() / near.len() as u64
        };
        let fresh = if explored.contains(&cell.col) {
            0
        } else {
            FRESH_COLUMN
        };
        pri[i] = Some(EMPTY_PRIOR + fresh + mean / 2);
    }
    for (c, p) in cells.iter_mut().zip(pri) {
        c.priority = p;
    }
    cells
}

/// Ties between equal priorities go by a hash of the seed and the cell,
/// so the same grid and seed pick the same cell.
fn mix(seed: u64, s: &str) -> u64 {
    let mut h = blake3::Hasher::new();
    h.update(&seed.to_le_bytes());
    h.update(s.as_bytes());
    u64::from_le_bytes(h.finalize().as_bytes()[..8].try_into().unwrap_or([0; 8]))
}

/// The next cell to fill: an empty cell with the most doubt around it
/// (game-printer picked among every cell; a covered cell's next clip is
/// the background work's to fix here).
pub(crate) fn next(grid: &Grid, cells: &[Cell], seed: u64) -> Option<Value> {
    let best = cells
        .iter()
        .filter(|c| c.clips.is_empty())
        .filter_map(|c| c.priority.map(|p| (p, c)))
        .max_by_key(|(p, c)| (*p, mix(seed, &format!("{}|{}", c.row, c.col))))?
        .1;
    let row = grid.row(&best.row)?;
    let col = grid.cols.iter().find(|c| c.id == best.col)?;
    let why = if best.priority.unwrap_or(0) > EMPTY_PRIOR + FRESH_COLUMN {
        "doubt"
    } else {
        "empty"
    };
    let around = if why == "doubt" {
        "the model differs next to it"
    } else {
        "nothing near it is covered yet"
    };
    Some(json!({
        "row": best.row,
        "col": best.col,
        "why": why,
        "says": format!(
            "record a clip of {} at {} {} ({around}), and bring it in with gp corpus import",
            row.label, col.level_name, col.place
        ),
        "clip": Value::Null,
    }))
}

/// The run's clips, as the grid reads them.
fn clips_of(run: &Run) -> Vec<Clip> {
    let levels: BTreeMap<String, String> = crate::clipset::read_game(&run.game)
        .map(|s| s.clips.into_iter().map(|r| (r.id, r.start.level)).collect())
        .unwrap_or_default();
    let marks = super::generated::marks(run);
    super::generated::made(run)
        .into_iter()
        .filter(|c| marks.get(&c.id).map(String::as_str) != Some("skipped"))
        .map(|c| Clip {
            level: levels.get(&c.id).cloned().unwrap_or_default(),
            session: c.group,
            name: c.name,
            id: c.id,
        })
        .collect()
}

/// The grid's state as the page reads it.
pub(crate) fn state_json(ctx: &Ctx, run: &Run) -> Value {
    let saved = read_json(&file(run));
    let in_flight = saved["in_flight"].as_u64().unwrap_or(DEFAULT_IN_FLIGHT);
    let mode = saved["mode"].as_str().unwrap_or("grid").to_owned();
    let seed = saved["seed"].as_u64().unwrap_or(1);
    let base = json!({
        "run": run.name, "mode": mode, "in_flight": in_flight,
        "max_in_flight": MAX_IN_FLIGHT, "made": 0, "left": 0, "flight": [],
    });
    let fail = |e: String| {
        let mut v = base.clone();
        v["error"] = json!(e);
        v
    };
    let dir = run.model_dir();
    let text = match std::fs::read_to_string(dir.join(gp_model::MODEL_FILE)) {
        Ok(t) => t,
        Err(e) => return fail(format!("no model in {}: {e}", dir.display())),
    };
    let model = match gp_model::Model::parse(&text) {
        Ok(m) => m,
        Err(e) => return fail(format!("the model doesn't read: {e}")),
    };
    let tree: toml::Value = toml::from_str(&text).unwrap_or(toml::Value::Table(toml::Table::new()));
    let (exts, _) = gp_exts::registry(&model.model.name);
    let clips = clips_of(run);
    let grid = derive(&model, &tree, &exts, &clips);
    let bg = super::background::saved_state(run);
    let needs: BTreeSet<String> = super::background::open_questions(run)
        .iter()
        .filter(|q| q["answer"].is_null())
        .filter_map(|q| q["clip"].as_str().map(str::to_owned))
        .collect();
    let working = super::background::working_on(run);
    let model_key = super::background::model_key(ctx, run);
    let cells = board(
        &grid,
        &clips,
        &Standing {
            scores: &bg["scores"],
            model: &model_key,
            needs: &needs,
            working: &working,
        },
    );
    let next = next(&grid, &cells, seed);
    let filled = cells.iter().filter(|c| !c.clips.is_empty()).count();
    let names: serde_json::Map<String, Value> = clips
        .iter()
        .map(|c| (c.id.clone(), json!(c.name)))
        .collect();
    let says = match &next {
        Some(n) => format!(
            "{filled} of {} cells have a clip. Next: {}.",
            cells.len(),
            n["says"].as_str().unwrap_or("")
        ),
        None => format!("{filled} of {} cells have a clip.", cells.len()),
    };
    let mut v = base;
    v["says"] = json!(says);
    v["rows"] = grid
        .rows
        .iter()
        .map(|r| json!({"id": r.id, "kind": r.kind, "label": r.label, "group": r.group}))
        .collect();
    v["cols"] = grid
        .cols
        .iter()
        .map(|c| json!({"id": c.id, "level": c.level, "level_name": c.level_name, "place": c.place, "starts": c.starts}))
        .collect();
    v["cells"] = cells
        .iter()
        .map(|c| {
            let mut j = json!({"row": c.row, "col": c.col, "state": c.state.name(), "clips": c.clips, "doubt": c.doubt});
            if let Some(p) = c.priority {
                j["priority"] = json!(p);
            }
            j
        })
        .collect();
    v["names"] = Value::Object(names);
    v["filled"] = json!(filled);
    v["cell_count"] = json!(cells.len());
    v["next"] = next.unwrap_or(Value::Null);
    v
}

fn update(run: &Run, f: impl FnOnce(&mut Value)) -> Result<Value, Fail> {
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut v = read_json(&file(run));
    if !v.is_object() {
        v = json!({"schema": 0});
    }
    f(&mut v);
    write_json(&file(run), &v)?;
    Ok(v)
}

fn state_route(ctx: &Ctx, _req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    Ok(Response::json(&state_json(ctx, &run)))
}

fn in_flight_route(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    if req.method == "POST" {
        let b: Value = super::body(req)?;
        let n = b["in_flight"]
            .as_u64()
            .ok_or_else(|| Fail::bad_request("give {in_flight}"))?
            .min(MAX_IN_FLIGHT);
        update(&run, |v| v["in_flight"] = json!(n))?;
    }
    let n = read_json(&file(&run))["in_flight"]
        .as_u64()
        .unwrap_or(DEFAULT_IN_FLIGHT);
    Ok(Response::json(
        &json!({"in_flight": n, "max": MAX_IN_FLIGHT}),
    ))
}

fn mode_route(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let run = run_dir(ctx, params)?;
    if req.method == "POST" {
        let b: Value = super::body(req)?;
        let m = b["mode"].as_str().unwrap_or("");
        if m != "grid" && m != "batch" {
            return Err(Fail::bad_request("the mode is grid or batch"));
        }
        update(&run, |v| v["mode"] = json!(m))?;
    }
    let m = read_json(&file(&run))["mode"]
        .as_str()
        .unwrap_or("grid")
        .to_owned();
    Ok(Response::json(&json!({"mode": m})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        let row = |id: &str, kind, group: &str| Row {
            id: id.into(),
            kind,
            label: words_of(id),
            group: group.into(),
            levels: None,
        };
        let col = |id: &str, starts: &[&str]| Col {
            id: id.into(),
            level: id.split('/').next().unwrap().into(),
            level_name: "L".into(),
            place: id.split('/').nth(1).unwrap().into(),
            starts: starts.iter().map(|s| (*s).to_owned()).collect(),
        };
        Grid {
            rows: vec![
                row("mechanic:platformer.jump", "mechanic", "platformer.jump"),
                row("mechanic:platformer.walk", "mechanic", "platformer.walk"),
                row("entity:mario", "entity", "mario"),
            ],
            cols: vec![
                col("a/jump", &["g.jump.001"]),
                col("a/walk", &["g.walk.001"]),
                col("b/walk", &[]),
            ],
            player: Some("entity:mario".into()),
        }
    }

    fn clip(id: &str, session: &str) -> Clip {
        Clip {
            id: id.into(),
            level: "a".into(),
            session: session.into(),
            name: format!("{session} 001"),
        }
    }

    #[test]
    fn a_clip_covers_the_mechanic_its_session_names_and_states_come_from_scores() {
        let g = grid();
        let clips = [clip("g.jump.001", "jump"), clip("g.walk.001", "walk")];
        assert_eq!(
            g.rows_of(&clips[0], &Value::Null),
            vec!["mechanic:platformer.jump".to_owned()]
        );
        let scores = json!({
            "g.jump.001": {"model": "m", "exact": false, "permille": 500, "error": null},
            "g.walk.001": {"model": "m", "exact": true, "permille": 1000, "error": null},
        });
        let (needs, working) = (BTreeSet::new(), BTreeSet::new());
        let cells = board(
            &g,
            &clips,
            &Standing {
                scores: &scores,
                model: "m",
                needs: &needs,
                working: &working,
            },
        );
        let at = |r: &str, c: &str| cells.iter().find(|x| x.row == r && x.col == c).unwrap();
        assert_eq!(
            at("mechanic:platformer.jump", "a/jump").state,
            CellState::Differs
        );
        assert_eq!(
            at("mechanic:platformer.walk", "a/walk").state,
            CellState::Exact
        );
        assert_eq!(
            at("mechanic:platformer.walk", "b/walk").state,
            CellState::NoStart
        );
        // The empty cell beside the one that differs goes first.
        let n = next(&g, &cells, 1).unwrap();
        assert_eq!(
            (n["row"].as_str(), n["col"].as_str()),
            (Some("mechanic:platformer.jump"), Some("a/walk"))
        );
        assert_eq!(n["why"], "doubt");
        // A question for a person on a clip makes its cells need you.
        let needs: BTreeSet<String> = ["g.jump.001".to_owned()].into();
        let cells = board(
            &g,
            &clips,
            &Standing {
                scores: &scores,
                model: "m",
                needs: &needs,
                working: &working,
            },
        );
        assert!(cells.iter().any(|c| c.state == CellState::NeedsYou));
    }
}
