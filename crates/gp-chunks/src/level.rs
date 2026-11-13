//! A level as the miner reads it, whatever its format: a grid of cell
//! values with the model's collision class for each, what the format says
//! each value is, the entities placed, and, for a format with an object
//! list, every object with the cells it drew.

use std::collections::BTreeMap;

use gp_sim::spec::Tile;

/// One level layout, decoded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelGrid {
    /// The first model level with this layout.
    pub name: String,
    /// Every model level with it.
    pub levels: Vec<String>,
    /// `<slot>/<entry>` or the grid's slot.
    pub layout: String,
    pub width: usize,
    pub height: usize,
    /// Pixels per cell.
    pub tile_size: i64,
    /// Each cell's value in the format (a metatile or tile number), row by
    /// row from the top left.
    pub ids: Vec<u32>,
    /// Each cell's collision class, as the model's level has it.
    pub class: Vec<Tile>,
    /// A name for each value, when the format has one.
    pub names: BTreeMap<u32, String>,
    /// What the format says each value is to the player (`coin`,
    /// `bumpable`, `pipe top`): its tile kinds.
    pub kinds: BTreeMap<u32, Vec<String>>,
    /// Tile kinds of single cells (doors and jars the format places).
    pub cell_kinds: BTreeMap<usize, Vec<String>>,
    /// Each value's four tile numbers by rows, for matching interactive
    /// element kinds.
    pub tiles: BTreeMap<u32, [u8; 4]>,
    /// Entities placed, the player left out.
    pub spawns: Vec<Placed>,
    /// The format's objects, in list order, when it has an object list.
    pub objects: Vec<Object>,
    /// Whether falling off the bottom kills, so a hole in the floor is a gap.
    pub pits: bool,
    /// The level's index in the compiled model, for reachability tests.
    pub spec_level: Option<usize>,
}

/// An entity placed in a level, in pixels from its top left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub entity: String,
    pub x: i64,
    pub y: i64,
}

/// One object of a level's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// Its kind in the format's vocabulary, without its parameters.
    pub kind: String,
    /// Its parameters, by name.
    pub params: Vec<(String, i64)>,
    /// The cells it drew (cell indexes), with the value each got: what
    /// changed when it was added to the list's earlier objects.
    pub cells: Vec<(usize, u32)>,
}

impl LevelGrid {
    pub fn at(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    /// Whether a level runs across (wider than high) rather than down.
    pub fn across(&self) -> bool {
        self.width >= self.height
    }

    /// The cell value of empty space: the commonest value of the cells
    /// that are empty to collision.
    pub fn background(&self) -> u32 {
        let mut n: BTreeMap<u32, usize> = BTreeMap::new();
        for (i, &id) in self.ids.iter().enumerate() {
            if self.class[i] == Tile::Empty {
                *n.entry(id).or_default() += 1;
            }
        }
        n.into_iter()
            .max_by_key(|(id, c)| (*c, std::cmp::Reverse(*id)))
            .map_or(0, |(id, _)| id)
    }

    /// Whether a cell is part of the level's structure: not empty to
    /// collision, or a value or cell with a tile kind.
    pub fn foreground(&self, i: usize) -> bool {
        self.class[i] != Tile::Empty
            || self.kinds.get(&self.ids[i]).is_some_and(|k| !k.is_empty())
            || self.cell_kinds.contains_key(&i)
    }

    /// Whether the player can stand on a cell.
    pub fn standable(&self, i: usize) -> bool {
        self.class[i].solid() || matches!(self.class[i], Tile::Top | Tile::Ice)
    }
}

/// A collision class's name in the catalog.
pub fn class_name(t: Tile) -> &'static str {
    match t {
        Tile::Empty => "empty",
        Tile::Solid => "solid",
        Tile::Top => "top",
        Tile::Ladder => "ladder",
        Tile::Hazard => "hazard",
        Tile::Water => "water",
        Tile::Ice => "ice",
        Tile::CoinBlock => "coin_block",
        Tile::ItemBlock => "item_block",
        Tile::Brick => "brick",
        Tile::Coin => "coin",
    }
}
