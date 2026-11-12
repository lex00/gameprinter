//! A template's `sprites.toml` (docs/formats/sprites.md): where the game's
//! sprite tables are in its lifted assets and what shape each has, which
//! entity each table entry belongs to, and how a frame's RAM says which of
//! the model's states an entity is in. Everything particular to a game is
//! here, as data; the capture that reads it is the same for every game.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The file's schema version.
pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    /// The model's sprite-set slot and the asset it's filled from (both
    /// `sprites` by default).
    #[serde(default = "sprites")]
    pub slot: String,
    #[serde(default = "sprites")]
    pub asset: String,
    /// Frames the sprite table lags the game's RAM: 1 when the game copies
    /// sprites to the PPU at the start of the next frame (most do).
    #[serde(default = "one")]
    pub oam_lag: u64,
    /// Pixels around an entity's hitbox within which a sprite is its.
    #[serde(default = "four_i32")]
    pub margin: i32,
    /// Fewest frames a frame must be seen in to count.
    #[serde(default = "two")]
    pub min_seen: u32,
    /// When an entity in an object slot faces left, as a condition over
    /// `slot[A]`, for entities with no `facing_left` of their own.
    pub facing_left: Option<String>,
    /// Frames to leave out whole, as a condition over `ram[A]`: where the
    /// game is somewhere the model doesn't have (another stage).
    pub skip: Option<String>,
    #[serde(default, rename = "table")]
    pub tables: Vec<Table>,
    #[serde(default, rename = "entity")]
    pub entities: BTreeMap<String, EntityConf>,
}

fn sprites() -> String {
    "sprites".into()
}
fn one() -> u64 {
    1
}
fn four_i32() -> i32 {
    4
}
fn two() -> u32 {
    2
}

/// A sprite table in the lifted assets, by its shape.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "shape", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Table {
    /// A `byte-rows` asset whose rows are frames: tile numbers, `width` to
    /// a row of the picture (Super Mario Bros.' EnemyGraphicsTable).
    Rows {
        name: String,
        asset: String,
        /// The model's tiles slot the numbers index.
        slot: String,
        width: u32,
        /// The palette, in the model's numbering, when the table doesn't
        /// say (4, the first sprite palette, by default).
        #[serde(default = "four")]
        palette: u8,
        /// Each entity ID's first row: a column of a `table` asset holding
        /// an offset, divided by `divide`. An ID's rows run to the next
        /// ID's first row.
        index: Option<Index>,
    },
    /// Frames as records of tile and attribute bytes, with the sprites'
    /// places in a second table of layouts (Mega Man 2's frames and OAM
    /// layouts): a `blobs` asset whose head gives the sprite count and the
    /// layout number, and rows `[tile, attribute]`; layouts are `blobs`
    /// with rows `[y, x]`, signed.
    Pieces {
        name: String,
        frames: Blob,
        layouts: Blob,
        /// Which head byte is the sprite count and which the layout.
        #[serde(default)]
        count_at: usize,
        #[serde(default = "one_usize")]
        layout_at: usize,
    },
    /// Frames as OAM records: a `blobs` asset whose first head byte is the
    /// count and whose bytes are `y, tile, attribute, x` per sprite.
    Oam { name: String, frames: Blob },
    /// Animation sequences: a `blobs` asset whose head gives the last step
    /// and the frames per step, and whose bytes are frame numbers in the
    /// table `frames` names, by entity ID (Mega Man 2's sprite
    /// definitions).
    Sequences {
        name: String,
        asset: Blob,
        /// The frame table the numbers index.
        frames: String,
        #[serde(default)]
        last_at: usize,
        #[serde(default = "one_usize")]
        every_at: usize,
    },
}

fn four() -> u8 {
    4
}
fn one_usize() -> usize {
    1
}

impl Table {
    pub fn name(&self) -> &str {
        match self {
            Table::Rows { name, .. }
            | Table::Pieces { name, .. }
            | Table::Oam { name, .. }
            | Table::Sequences { name, .. } => name,
        }
    }
}

/// A column of a `table` asset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub asset: String,
    pub column: String,
    #[serde(default = "one_u32")]
    pub divide: u32,
    /// A column of the same asset with each ID's sprite attribute byte,
    /// whose low two bits are its palette.
    pub attributes: Option<String>,
}

fn one_u32() -> u32 {
    1
}

/// A pointer table of a `blobs` asset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blob {
    pub asset: String,
    pub pointers: String,
}

/// What the capture needs to know of one entity.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityConf {
    /// Its object IDs; by default the state mapping's `[[slots.kind]]`.
    #[serde(default)]
    pub ids: Vec<u8>,
    /// Entries of tables, by table name, that are its frames, beyond what
    /// its IDs index.
    #[serde(default)]
    pub frames: BTreeMap<String, Vec<u32>>,
    /// Which model state it's in on a frame: the first whose condition
    /// holds. Conditions are the runtime protocol's, over `ram[A]` and,
    /// for an object in a slot, `slot[A]` (A plus the slot's number).
    #[serde(default, rename = "state")]
    pub states: Vec<Rule>,
    /// Conditions on its state for clips, by name: `ground`, `jumped`,
    /// `invulnerable`, `moving`, `rising`. Without one, `moving` and
    /// `rising` come from its position, and `ground` from the state
    /// mapping's `on_ground` when it has one.
    #[serde(default)]
    pub when: BTreeMap<String, String>,
    /// Frames to leave out, as a condition: what the model can't choose a
    /// sprite by yet, such as a cutscene.
    pub skip: Option<String>,
    /// When it faces left, as a condition, where the state mapping has no
    /// `facing`. Without either, the way it last moved.
    pub facing_left: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub name: String,
    pub when: String,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        if c.schema != SCHEMA {
            return Err(format!(
                "sprites.toml schema {} isn't supported; this build reads {SCHEMA}",
                c.schema
            ));
        }
        for (name, e) in &c.entities {
            for r in &e.states {
                gp_runtime::select::Condition::parse(&r.when)
                    .map_err(|err| format!("entity.{name}.state {}: {err}", r.name))?;
            }
            for (k, w) in &e.when {
                if !crate::observe::CONDITIONS.contains(&k.as_str()) {
                    return Err(format!(
                        "entity.{name}.when.{k}: not one of {}",
                        crate::observe::CONDITIONS.join(", ")
                    ));
                }
                gp_runtime::select::Condition::parse(w)
                    .map_err(|err| format!("entity.{name}.when.{k}: {err}"))?;
            }
            if let Some(w) = &e.skip {
                gp_runtime::select::Condition::parse(w)
                    .map_err(|err| format!("entity.{name}.skip: {err}"))?;
            }
            for t in e.frames.keys() {
                if !c.tables.iter().any(|x| x.name() == t) {
                    return Err(format!("entity.{name}.frames: no table named `{t}`"));
                }
            }
        }
        for t in &c.tables {
            if let Table::Sequences { name, frames, .. } = t
                && !c.tables.iter().any(|x| x.name() == frames)
            {
                return Err(format!("table {name}: no frame table named `{frames}`"));
            }
        }
        Ok(c)
    }

    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name() == name)
    }
}
