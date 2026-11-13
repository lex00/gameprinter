//! The level chunk catalog format, version 0.1.0 (docs/formats/chunks.md).
//! The JSON Schema in `schema/chunks.json` is generated from these
//! types (`gp chunks --schema`), and a test fails when it's stale.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The format's name, in every catalog's `format`.
pub const FORMAT: &str = "gp-chunks";
/// The format's version.
pub const VERSION: &str = "0.1.0";

/// A game's level chunk catalog: the pieces its levels are made of, what
/// each does, and how they sit together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer level chunk catalog 0.1.0")]
pub struct Catalog {
    /// Always `gp-chunks`.
    pub format: String,
    /// The format's version, `0.1.0`.
    pub version: String,
    /// The template's folder name.
    pub template: String,
    /// The model's name.
    pub model: String,
    /// The model's source, when it names one.
    pub source: Option<String>,
    /// The settings the catalog was mined with.
    pub settings: Settings,
    /// The player's jump on a flat floor, from `gp_interp::reach::envelope`,
    /// when the model's player can be searched.
    pub envelope: Option<Envelope>,
    /// The interactive element catalog (docs/formats/interactive.md) the
    /// cells' interactive kinds come from, when one was read.
    pub interactive: Option<String>,
    /// Every level layout read, in model order.
    pub levels: Vec<LevelInfo>,
    /// Chunk kinds by layer.
    pub layers: Layers,
    /// Objects of the format that changed no cell of the grid, by kind and
    /// count: page jumps and area pointers, scenery the grid doesn't hold
    /// (Super Mario Bros.' castles and ropes are drawn but not kept), and
    /// objects a later one drew over entirely.
    pub controls: BTreeMap<String, usize>,
    /// The chunk kinds, by layer and then most frequent first.
    pub chunks: Vec<Chunk>,
    /// What the catalog couldn't do, and why.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Occurrences a mined pattern, group or compound needs to be a kind.
    pub min_count: usize,
    /// Window sizes mined, `[width, height]` in cells.
    pub windows: Vec<[usize; 2]>,
    /// Whether one-cell-wide slices across the level (columns for a level
    /// wider than high, rows otherwise) were mined.
    pub slices: bool,
    /// The most window patterns kept per size.
    pub max_patterns: usize,
    /// The largest group kept, `[width, height]` in cells; larger connected
    /// pieces count as terrain.
    pub max_group: [usize; 2],
    /// Chunk kinds per layer that got a reachability test.
    pub reach_tests: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    /// Pixels the jump rises, held to its top.
    pub rise: i64,
    /// Frames in the air.
    pub airtime: u32,
    /// Pixels carried walking and running.
    pub walk_distance: i64,
    pub run_distance: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelInfo {
    /// The first model level with this layout.
    pub name: String,
    /// Every model level with this layout.
    pub levels: Vec<String>,
    /// Where its layout comes from: `<slot>/<entry>` or the grid's slot.
    pub layout: String,
    /// Width and height in cells.
    pub size: [usize; 2],
    pub tile_size: i64,
    /// Cells that are part of the level's structure (foreground): not
    /// empty to collision, or holding a tile kind.
    pub cells: usize,
    /// Of those, the terrain: the floor's values joined to an edge of the
    /// level.
    pub terrain: usize,
    /// Objects of the format's list, when it has one.
    pub objects: usize,
    /// Entities placed, the player left out.
    pub spawns: usize,
    /// Shares of the foreground cells inside a known chunk.
    pub coverage: Coverage,
}

/// Shares of a level's structure cells inside at least one occurrence of a
/// chunk kind of each layer, and of any layer. `groups` is of the cells
/// that aren't terrain, since terrain is never a group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub objects: f64,
    pub groups: f64,
    pub patterns: f64,
    pub any: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layers {
    /// Object kinds of the format's own vocabulary (layer a).
    pub objects: usize,
    /// Connected pieces of the decoded grid, by shape (layer b).
    pub groups: usize,
    /// Gaps in the floor, by width (layer b).
    pub gaps: usize,
    /// Window and slice patterns (layer b).
    pub patterns: usize,
    /// Pairs of chunks or a chunk and an enemy that sit together (above
    /// the object layer).
    pub compounds: usize,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Objects,
    Groups,
    Gaps,
    Patterns,
    Compounds,
}

/// One chunk kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    /// Unique in the catalog: `object:<name>`, `group:<hash>`, `gap:<width>`,
    /// `window:<w>x<h>:<hash>`, `slice:<hash>`, `compound:<relation>:<a>|<b>`.
    pub id: String,
    pub layer: Layer,
    /// A name for people: the format's object name, a gap's width, the
    /// cells' names, or the parts and their relation.
    pub name: String,
    /// The example's size in cells, `[width, height]`.
    pub size: [usize; 2],
    /// The example's cells, rows from the top: the format's cell value
    /// (metatile or tile number), or null where the chunk has no cell.
    pub shape: Vec<Vec<Option<u32>>>,
    /// For an object kind, its parameters and its footprint as a function
    /// of each.
    pub params: Vec<Param>,
    /// Occurrences across every level read.
    pub count: usize,
    /// Occurrences by level.
    pub by_level: BTreeMap<String, usize>,
    /// For a compound, the relation (`with`, `over`, `above`, `left-of`)
    /// and the ids of its two parts (an enemy part is `enemy:<entity>`).
    pub relation: Option<String>,
    pub parts: Vec<String>,
    pub function: Function,
    /// Up to five occurrences, `{level, x, y}` in cells (the top left).
    pub examples: Vec<Example>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Param {
    pub name: String,
    /// The values seen, smallest first.
    pub values: Vec<i64>,
    /// The most common footprint `[width, height]` for each value seen.
    pub footprint: Vec<Footprint>,
    /// A line through the footprints, when every one is on it.
    pub fit: Option<Fit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    pub value: i64,
    pub size: [usize; 2],
    /// Occurrences with this value.
    pub count: usize,
}

/// `axis = base + per * value`, in cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fit {
    /// `width` or `height`.
    pub axis: String,
    pub base: i64,
    pub per: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub level: String,
    pub x: usize,
    pub y: usize,
}

/// What a chunk is to the player and where it goes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Function {
    /// The example's cells by the model's collision class (`empty`,
    /// `solid`, `top`, `ladder`, `hazard`, `water`, `ice`).
    pub classes: BTreeMap<String, usize>,
    /// The example's cells by the format's tile kinds (a coin, a block hit
    /// from below, a pipe top, a door).
    pub tile_kinds: BTreeMap<String, usize>,
    /// The example's cells by interactive element kind, matched by the
    /// cell's tile numbers (docs/formats/interactive.md).
    pub interactive: BTreeMap<String, usize>,
    /// Entities placed on or near occurrences.
    pub enemies: Vec<Near>,
    /// What getting past it demands, from a reachability search on a
    /// made-up level holding it, when one was run.
    pub demand: Option<Demand>,
    pub placement: Placement,
    /// What sits next to occurrences, by chunk id (or `terrain`, `edge`,
    /// `none`), on each side.
    pub adjacency: Adjacency,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Near {
    pub entity: String,
    /// Entities of this kind on or near an occurrence.
    pub count: usize,
    /// The share of occurrences with at least one.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Demand {
    /// Whether the search reached the goal past the chunk.
    pub reachable: bool,
    /// `walk`, `jump`, `running jump` or `blocked`, from the search and the
    /// chunk's shape.
    pub needs: String,
    /// The highest wall from the floor, and the widest gap, in pixels.
    pub height_px: i64,
    pub gap_px: i64,
    /// Whether the player stood on the chunk.
    pub stands_on_top: bool,
    /// Moves the search simulated.
    pub moves: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// Occurrences resting on something solid below, and in the air.
    pub ground: usize,
    pub air: usize,
    /// Occurrences by the third of the level's height their middle is in:
    /// top, middle, bottom.
    pub rows: [usize; 3],
    /// By the third of the level's length: early, middle, late.
    pub segment: [usize; 3],
    /// By the third of a screen (16 cells) across: left, middle, right.
    pub screen: [usize; 3],
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Adjacency {
    pub left: BTreeMap<String, usize>,
    pub right: BTreeMap<String, usize>,
    pub above: BTreeMap<String, usize>,
    pub below: BTreeMap<String, usize>,
}

/// The catalog's JSON Schema, as `schema/chunks.json` holds it.
pub fn schema() -> String {
    let s = schemars::schema_for!(Catalog);
    let mut text = serde_json::to_string_pretty(&s).expect("a schema serializes");
    text.push('\n');
    text
}
