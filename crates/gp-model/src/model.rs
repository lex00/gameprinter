//! The model core schema, version [`CORE_VERSION`], as Rust types. The JSON
//! Schema in `schema/model-core.json` is generated from them
//! (`gp model schema core`), and docs/model.md describes each part.
//!
//! Units: positions and sizes are pixels, times are frames, speeds are
//! pixels per frame. Positions grow right and down. Every number the
//! simulation reads is a [`Fixed`].

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::fixed::Fixed;
use crate::platformer;
use crate::topdown;

/// The model core schema's version, `major.minor`. It bumps only for a
/// breaking change, once per release; an optional part added since is a
/// named feature ([`crate::features`]), not a bump. A model names the
/// version it was written for in `model.schema` (a patch there, as in
/// "0.4.23", is from before features and is read as "0.4").
pub const CORE_VERSION: &str = "0.4";

/// A whole game model: one `model.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer model core schema 0.4")]
pub struct Model {
    /// What the model is and which schema versions it uses.
    pub model: Header,
    /// Named actions and the buttons that trigger them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Input>,
    /// Asset slots by name: what each holds, its role, and where it's filled from.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assets: BTreeMap<String, AssetSlot>,
    /// Entity types by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entities: BTreeMap<String, Entity>,
    /// Levels by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub levels: BTreeMap<String, Level>,
    /// Cameras by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cameras: BTreeMap<String, Camera>,
    /// The game's state machines by name, such as playing, dying and won.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, Machine>,
    /// Numbers the game keeps across levels, such as deaths or score.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counters: BTreeMap<String, Counter>,
    /// Feature `cost.frame`: the CPU cycles the original spends on a
    /// frame's work, as terms over the model's state, for a game that steps
    /// its random generator in the time the frame's work leaves
    /// (docs/cycles.md, "The cost model").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    /// Feature `stages`: the original game's stages in the order it plays
    /// them, for a model that holds fewer levels than the game has stages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stages: Option<Stages>,
    /// How the game is shown: the screen, the HUD and palette changes over
    /// time. Presentation only: the simulation never reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present: Option<Present>,
    /// Where each fact comes from, by model path. An entry covers the part
    /// it names and everything under it, unless a more specific entry
    /// covers that.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub evidence: BTreeMap<String, Evidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Header {
    /// The core schema version the model is written for, such as "0.4.1".
    pub schema: String,
    /// A short name: letters, digits and `_`.
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The source ID the model was made from, as in sources/*.toml or an
    /// asset map, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub view: View,
    /// Frames per second as presented: "60", or a ratio such as
    /// "39375000/655171" (the NTSC NES). The simulation counts frames and
    /// never reads it.
    pub frame_rate: Ratio,
    /// The genre extensions the model uses, with their versions.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, String>,
    /// The optional features the model uses, by name, such as
    /// `launcher` or `contact.touch_edges` ([`crate::features`]). A reader
    /// refuses a model listing one it doesn't know. Without the list the
    /// features are derived from the fields the model sets; with it, each
    /// one used must be listed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// The order the systems run in each frame: every system of the core
    /// and of each declared extension, once each ([`CORE_SYSTEMS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub systems: Vec<String>,
    /// The level the game starts in. Needed when there's more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_level: Option<String>,
    /// Events of the model's own, which `ext` hooks fire and transitions
    /// and counter changes name, such as `ladder.grabbed`. One fired with
    /// an entity concerns only that instance's machine; one fired without
    /// concerns every instance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    /// The profile the model was built in (core 0.4.5), such as "normal":
    /// set only by applying one from the template's `profiles.toml`
    /// ([`crate::profile`]), never in a `model.toml`. Absent, the model is
    /// its base profile, "retro". Runtimes put it in the state hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// The core's systems, which `model.systems` puts in order with the
/// extensions' systems. Each runs once a frame, in that order:
///
/// - `rules`: the game's state machines count time and take `after`
///   transitions.
/// - `entity_states`: the same for each entity's own state machine.
/// - `counters`: counters with an `every` change count time and change.
/// - `spawn`: entities whose spawn comes within the level's `spawn_ahead`
///   of the view appear, and those `despawn_behind` it go.
/// - `exits`: the player takes an exit it's in the `take` box of.
/// - `camera`: cameras follow.
///
/// Transitions on events and counter changes on events don't wait for
/// their system: they happen when the event fires, so the systems after it
/// in the same frame see the new state. Freezing is the exception: whether
/// the world is frozen is settled at the start of each frame, so entering a
/// `freeze` state stops the world from the next frame, and leaving one
/// starts it again from the next frame.
pub const CORE_SYSTEMS: &[&str] = &[
    "rules",
    "entity_states",
    "counters",
    "spawn",
    "exits",
    "camera",
];

/// How the world is seen. Isometric arrives with its projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum View {
    /// A side view: down is the direction gravity pulls.
    Side,
    TopDown,
}

/// A positive ratio of whole numbers, written "N" or "N/D".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    pub num: u64,
    pub den: u64,
}

impl Ratio {
    pub fn parse(text: &str) -> Result<Self, String> {
        let (n, d) = text.split_once('/').unwrap_or((text, "1"));
        let num = |s: &str| -> Result<u64, String> {
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("`{text}` isn't \"N\" or \"N/D\""));
            }
            s.parse().map_err(|_| format!("`{text}` is out of range"))
        };
        let r = Ratio {
            num: num(n)?,
            den: num(d)?,
        };
        if r.num == 0 || r.den == 0 {
            return Err(format!("`{text}` must be above zero"));
        }
        // In lowest terms, so "120/2" and "60" are the same text.
        let (mut a, mut b) = (r.num, r.den);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Ratio {
            num: r.num / a,
            den: r.den / a,
        })
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl Serialize for Ratio {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Ratio {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ratio::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Ratio {
    fn schema_name() -> Cow<'static, str> {
        "Ratio".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "A positive ratio of whole numbers: \"N\" or \"N/D\".",
            "type": "string",
            "pattern": "^[0-9]+(/[0-9]+)?$"
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub device: Device,
    /// Action names to buttons. Extensions read actions by name.
    pub actions: BTreeMap<String, Button>,
    /// Feature `input.keep_direction`: the pad's directions are latched as
    /// a game filters them. A direction that acted on the frame before and
    /// is still held stays the only one, and newly held ones wait until it
    /// is let go; with none of them held, every direction held acts.
    /// Donkey Kong's pad routine ($F549) keeps the old directions that are
    /// still held, so Up added while walking left doesn't climb until Left
    /// is let go.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_direction: bool,
    /// Feature `input.turn`: in a game whose players take turns, the
    /// player whose turn it is plays from their own controller. While the
    /// counter is k, these actions read controller k + 1; every other
    /// action reads controller 1. Donkey Kong's player routine reads the
    /// pad at $15 + 2 times the player in $52 ($D175), so player 2 plays
    /// from controller 2, while Start and Select are read from controller
    /// 1 only ($CF42, $CE8B).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Turn>,
}

/// Feature `stages`: the original's stages in order, each played by the
/// level of its name or, for a stage the model doesn't hold yet, by a
/// stand-in level. A counter the model's rules step as the game does says
/// which stage is being played; `next-level` and `restart-level` start the
/// level that plays it. Donkey Kong's NES port plays 25m, 75m and 100m and
/// then 25m again a loop harder ($D096-$D0AE); a model with 25m only plays
/// 25m in each slot, while the stage counter, the loops and what they
/// change (the bonus, the difficulty, the level number) go on as the
/// game's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stages {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The stages in the order the game plays them: each a level's name,
    /// or the name of a stage the model has no level for yet.
    pub order: Vec<String>,
    /// The level that plays a stage of `order` the model has no level for.
    pub stand_in: String,
    /// The counter holding the index in `order` of the stage being
    /// played, 0 the first. The model's rules step it, as the game steps
    /// its own stage number.
    pub counter: String,
    /// Feature `stages.skip`: stages of `order` the game passes over.
    /// When the rules step the counter onto one, it steps on again the
    /// same way (past the end with its wrap), so no level plays it.
    /// Donkey Kong (JU) has no 50m and steps its stage number $53 from 1
    /// (25m) to 3 (75m); Original Edition's model, loaded with a JU ROM,
    /// skips 50m's place rather than playing a stand-in in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip: Vec<String>,
}

/// Feature `input.turn`: which controller each turn plays from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    /// The counter that says whose turn it is: 0 the first player.
    pub counter: String,
    /// The actions that read the turn's controller.
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Device {
    /// The standard NES controller.
    NesPad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    A,
    B,
    Select,
    Start,
    Up,
    Down,
    Left,
    Right,
}

/// A named place for an asset. The model refers to assets only by slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetSlot {
    pub kind: AssetKind,
    /// What the asset is for (docs/model.md lists the roles).
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The lifting recipe that fills the slot: an asset of an asset map
    /// (assets/<source>.toml). A commercial-game template ships the slot
    /// empty with only this recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<AssetSource>,
    /// A role for each item in the asset, in order (each metatile of a
    /// tileset, each palette of a palettes slot), for themes and the
    /// readability check. Required for metatiles and palettes slots from
    /// core 0.3.1 on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_roles: Vec<String>,
    /// For a `spawn-objects` slot: the entity type each object kind number
    /// places, by entity name. An object whose number isn't listed places
    /// nothing in the model.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entities: BTreeMap<String, Vec<u32>>,
    /// For a `songs` or `sound-effects` slot: the capture that fills it,
    /// a recording's frames read by `gp sound capture`. Like `from`, a
    /// recipe: the songs are game content and stay outside the repo.
    /// Since core 0.4.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<SoundCapture>,
    /// For a `screen-text` slot: the `tiles` slot its layouts are drawn
    /// with where a screen shows them (`present.screens`), in place of
    /// the level's (feature `assets.screen_tiles`): a screen the game
    /// draws with another pattern table or CHR bank. Donkey Kong: Original
    /// Edition draws its title with CHR bank 1 (the hook at $C8F3).
    /// For a `level-objects` slot filled from update strings: the `tiles`
    /// slot a level drawn from it uses, in place of the level's (feature
    /// `assets.layout_tiles`); Original Edition draws 50m with CHR bank 1
    /// ($CB5D -> $838A). Presentation only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiles: Option<String>,
}

/// Where a sound slot's songs are captured from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundCapture {
    /// The recording's ID (or a unique prefix of it).
    pub recording: String,
    /// Its frames, as `gp sound capture --frames` takes them (`A..B`).
    pub frames: String,
    /// The capture's name, which its files are kept under.
    pub name: String,
}

/// One entry of an asset that holds a list per entry, such as one area's
/// objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntryRef {
    /// The slot.
    pub slot: String,
    /// The entry's `name` in the asset map.
    pub entry: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AssetKind {
    /// 8x8 tiles (the asset map format `chr-2bpp`).
    Tiles,
    /// Palettes (`palette`).
    Palettes,
    /// Metatiles with their properties (`metatiles`).
    Metatiles,
    /// A grid of metatile numbers (`level-grid`).
    LevelGrid,
    /// Rows of numbers (`table`).
    Table,
    /// Lists of objects that build a level's tiles, one list per entry
    /// (`smb-area-objects`, `smb2-level-objects`, or Mega Man 2's
    /// `mm2-screens`).
    LevelObjects,
    /// Lists of objects that place entities in a level, one list per entry
    /// (`smb-enemy-objects`, `smb2-enemy-objects`, or `mm2-spawns`).
    SpawnObjects,
    /// Rows of bytes, such as a game's sprite frames (`byte-rows`).
    Rows,
    /// A text encoding: which tile stands for each character, read from
    /// the `charset` of a text asset (`ppu-strings`). Presentation only.
    Font,
    /// Tiles written to places on the screen, as a console's update
    /// strings write them (`ppu-strings`): a status bar's fixed text.
    /// Presentation only.
    ScreenText,
    /// Sprite frames as layouts of tiles, each tile with its own place in
    /// pixels, flips, palette and tiles slot, with the sprite palettes
    /// each level showed (`sprite-set`, written by `gp sprites capture`
    /// from a game's sprite tables and recordings; docs/formats/sprite-set.md).
    /// Presentation only. Since core 0.4.2.
    SpriteSet,
    /// Songs as notes per channel, with their patterns, instruments and
    /// tempo in frames (`gp-songs`, written by `gp sound capture` from a
    /// recording's APU register log or by `gp sound decode` from a known
    /// driver's data; docs/formats/songs.md). Presentation only. Since
    /// core 0.4.3.
    Songs,
    /// Sound effects in the same form, each tied to the model event it
    /// marks. Presentation only. Since core 0.4.3.
    SoundEffects,
}

impl AssetKind {
    /// The asset map formats that lift into this kind of slot. Each object
    /// list format has a builder in the runtimes (docs/model.md, "Asset
    /// slots"); a runtime refuses a format it has no builder for.
    pub fn map_formats(self) -> &'static [&'static str] {
        match self {
            Self::Tiles => &["chr-2bpp"],
            Self::Palettes => &["palette"],
            Self::Metatiles => &["metatiles"],
            Self::LevelGrid => &["level-grid"],
            Self::Table => &["table"],
            Self::LevelObjects => &[
                "smb-area-objects",
                "smb2-level-objects",
                "mm2-screens",
                "ppu-strings",
            ],
            Self::SpawnObjects => &["smb-enemy-objects", "smb2-enemy-objects", "mm2-spawns"],
            Self::Rows => &["byte-rows"],
            // A font is a charset: a string asset's, or a tile asset's own.
            Self::Font => &["ppu-strings", "chr-2bpp"],
            Self::ScreenText => &["ppu-strings"],
            Self::SpriteSet => &["sprite-set"],
            Self::Songs | Self::SoundEffects => &["gp-songs", "smb-music"],
        }
    }

    /// The first of [`AssetKind::map_formats`], for messages.
    pub fn map_format(self) -> &'static str {
        self.map_formats()[0]
    }

    /// The kind's name as a slot's `kind` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Tiles => "tiles",
            Self::Palettes => "palettes",
            Self::Metatiles => "metatiles",
            Self::LevelGrid => "level-grid",
            Self::Table => "table",
            Self::LevelObjects => "level-objects",
            Self::SpawnObjects => "spawn-objects",
            Self::Rows => "rows",
            Self::Font => "font",
            Self::ScreenText => "screen-text",
            Self::SpriteSet => "sprite-set",
            Self::Songs => "songs",
            Self::SoundEffects => "sound-effects",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetSource {
    /// The asset map's `source`.
    pub source: String,
    /// The asset's `name` in the map.
    pub asset: String,
}

/// An entity type: core fields plus components from extensions.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub core: EntityCore,
    pub platformer: platformer::EntityComponents,
    pub topdown: topdown::EntityComponents,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityCore {
    /// What the entity is for (docs/model.md lists the roles).
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Width and height.
    pub size: [Fixed; 2],
    /// The box that touches other entities and tiles, from the entity's
    /// position (its top left corner).
    pub hitbox: Rect,
    /// Stops against solid tiles and stands on them.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tile_collision: bool,
    /// The box that touches tiles, when it isn't `hitbox`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile_hitbox: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<Sprite>,
    /// The entity's own state machine (small and big, walking and in its
    /// shell). Each instance runs its own copy. A state's `set` changes
    /// the entity's fields while in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<Machine>,
    /// A hand-written behavior behind the `ext` boundary, by name, for what
    /// the components can't say. It runs in the system of the entity's
    /// role: `platformer.player` for the player, `platformer.enemies` for
    /// the rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    /// The numbers the `ext` hook reads, by name: speeds, timers, offsets.
    /// Each is a fact with evidence, like any other, and a state's `set`
    /// can change them. Only an entity with `ext` has them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext_params: BTreeMap<String, Fixed>,
    /// The tables the `ext` hook reads, by name: each a column of one of
    /// the model's `table` asset slots, as `<slot>.<column>`, so indexed
    /// data the hook needs is lifted from the original like any asset.
    /// Only an entity with `ext` has them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext_tables: BTreeMap<String, String>,
    /// Feature `entity.ext_rows`: more tables the `ext` hook reads, by
    /// name, each a `rows` asset slot whose rows' numbers, in order, are
    /// one table, for a hook that reads by address a span the model lifts
    /// as rows. Only an entity with `ext` has them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext_rows: BTreeMap<String, String>,
    /// Takes none of a level's `spawn_slots`: Super Mario Bros.' hammers
    /// and a firebar's fireballs, which the game keeps outside its enemy
    /// slots. Since core 0.4.10.
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_slot: bool,
    /// Feature `entity.hook_through_stops`: its `ext` hook still runs on a
    /// frame a state's `stops` lists the system it runs in (not under a
    /// `freeze`). Donkey Kong's oil drum keeps the game's frame timers,
    /// which count on through a death while the fires stand still.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hook_through_stops: bool,
    /// Feature `entity.hook_frame_start`: its `ext` hook's `frame_start`
    /// runs at the start of every frame, before the first system, frozen
    /// or not, as a game's NMI runs its sound driver before anything else.
    /// Donkey Kong's sound driver steps there.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hook_frame_start: bool,
    /// Feature `entity.hook_first`: its `ext` hook runs at the start of the
    /// `platformer.player` system, before the player moves, rather than in
    /// `platformer.enemies` after it: a game routine that sets what the
    /// player's routine reads. Donkey Kong's attract demo ($EBDA) puts the
    /// script's direction in $56 just before Mario's routine reads it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hook_first: bool,
    /// Feature `entity.hook_last`: its `ext` hook runs once at the end of
    /// every frame, after every system, frozen, stopped or a level start
    /// alike, and not in `platformer.enemies`: a game's idle loop, which
    /// runs after the frame's work whatever it was. Donkey Kong's ($C7E1)
    /// steps its random register once a pass while it waits for the NMI.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hook_last: bool,
    /// More of a level's `spawn_slots` it takes besides its own, numbered
    /// slots included: Super Mario Bros.' long firebar, for which
    /// DuplicateEnemyObj takes the next free enemy slot.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub extra_slots: u32,
    /// Its own `despawn_behind`, in place of the level's: Super Mario
    /// Bros.' piranha plants and hammer brothers go sooner than the rest
    /// (OffscreenBoundsCheck). Since core 0.4.10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub despawn_behind: Option<Fixed>,
    /// `[from, behind]`: while the view's left edge is `from` or more into
    /// its 256-pixel page, `behind` stands in for `despawn_behind`. Super
    /// Mario Bros.' OffscreenBoundsCheck adds 57 to the edge's low byte and
    /// the carry out of that takes a whole page off the limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub despawn_behind_wrap: Option<[u32; 2]>,
    /// Its own `despawn_ahead`, in place of the level's: Super Mario Bros.'
    /// cannon bill flies off to the right and goes past the same line
    /// (OffscreenBoundsCheck), where a level's would also take what a start
    /// or a layout leaves out there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub despawn_ahead: Option<Fixed>,
}

impl Entity {
    /// The entity as it is in one of its own states: its fields with that
    /// state's `set` merged in.
    pub fn in_state(&self, state: &str) -> Result<Entity, String> {
        let s = self
            .core
            .machine
            .as_ref()
            .and_then(|m| m.states.get(state))
            .ok_or_else(|| format!("no state named `{state}`"))?;
        if s.set.is_empty() {
            return Ok(self.clone());
        }
        let mut t = toml::Table::try_from(self).map_err(|e| e.to_string())?;
        merge_into(&mut t, &s.set);
        toml::Value::Table(t).try_into::<Entity>().map_err(|e| {
            format!(
                "with `set` merged in, the entity doesn't parse: {}",
                e.message()
            )
        })
    }

    /// Rewrites each state's `set` values in canonical form, as the
    /// entity's own fields are written. A state whose `set` doesn't fit is
    /// left for validation to report.
    pub(crate) fn canonicalize_sets(&mut self) {
        let Some(m) = &self.core.machine else { return };
        let mut canonical = Vec::new();
        for (name, state) in &m.states {
            let Ok(e) = self.in_state(name) else { continue };
            let Ok(t) = toml::Table::try_from(&e) else {
                continue;
            };
            canonical.push((name.clone(), restrict(&state.set, &t)));
        }
        let m = self.core.machine.as_mut().expect("checked above");
        for (name, set) in canonical {
            m.states.get_mut(&name).expect("a state").set = set;
        }
    }
}

/// Merges `set` into `t`: a table into a table key by key, anything else
/// replacing.
fn merge_into(t: &mut toml::Table, set: &BTreeMap<String, toml::Value>) {
    for (k, v) in set {
        match (t.get_mut(k), v) {
            (Some(toml::Value::Table(inner)), toml::Value::Table(sub)) => {
                let sub: BTreeMap<String, toml::Value> =
                    sub.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                merge_into(inner, &sub);
            }
            _ => {
                t.insert(k.clone(), v.clone());
            }
        }
    }
}

/// The keys of `set`, with their values taken from `from`.
fn restrict(
    set: &BTreeMap<String, toml::Value>,
    from: &toml::Table,
) -> BTreeMap<String, toml::Value> {
    let mut out = BTreeMap::new();
    for (k, v) in set {
        let canon = match (v, from.get(k)) {
            (toml::Value::Table(sub), Some(toml::Value::Table(f))) => {
                let sub: BTreeMap<String, toml::Value> =
                    sub.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                toml::Value::Table(restrict(&sub, f).into_iter().collect())
            }
            (_, Some(f)) => f.clone(),
            (v, None) => v.clone(),
        };
        out.insert(k.clone(), canon);
    }
    out
}

/// Fields of [`EntityCore`], so an entity's other keys can go to the
/// extensions. A test checks the list against the type.
pub const ENTITY_CORE_FIELDS: &[&str] = &[
    "role",
    "description",
    "size",
    "hitbox",
    "tile_collision",
    "tile_hitbox",
    "sprite",
    "machine",
    "ext",
    "ext_params",
    "ext_tables",
    "ext_rows",
    "no_slot",
    "hook_through_stops",
    "hook_frame_start",
    "hook_first",
    "hook_last",
    "extra_slots",
    "despawn_behind",
    "despawn_behind_wrap",
    "despawn_ahead",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: Fixed,
    pub y: Fixed,
    pub w: Fixed,
    pub h: Fixed,
}

/// A sprite's second picture (`copy`), in pixels. Presentation only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpriteCopy {
    /// How far below the first it is.
    pub below: u32,
    /// Its height wraps at this y.
    pub wrap: u32,
    /// Neither picture shows with its top above this y.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub above: u32,
}

/// How an entity is drawn. Presentation only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sprite {
    /// A slot of kind `tiles`.
    pub slot: String,
    /// Tile numbers, row by row. Give these or `frame`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<u32>,
    /// The tile numbers from a lifted table instead: a row of a `rows`
    /// slot, such as one of the game's sprite frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<RowRef>,
    /// Tiles per row.
    pub width: u32,
    /// Which palette of the game's palettes the tiles use.
    pub palette: u32,
    /// Mirrored left to right while the entity faces left.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_when_facing_left: bool,
    /// Since core 0.4.20: a second picture below the first (Super Mario Bros.'
    /// DrawSmallPlatform draws a small lift twice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<SpriteCopy>,
    /// Each tile's own flips and palette, in the order of the tiles: the
    /// sprite as a layout of tiles, as a console's sprite attribute bytes
    /// give it. A game that saves tiles by mirroring half a sprite says so
    /// here. Without it every tile is drawn as it is, in `palette`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<Cell>,
    /// Colors of its own for `palette`: one palette of a `palettes` slot,
    /// such as a power-up's colors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<PaletteRef>,
    /// Animation: the first clip whose `when` holds is drawn; when none
    /// does, the frame above is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<Clip>,
    /// Hidden on every other run of `every` frames while `when` holds,
    /// whatever it shows otherwise: a hurt entity's blink. Counted by the
    /// game's frames. Since core 0.4.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blink: Option<Blink>,
    /// The picture's top left in pixels from the entity's position, in
    /// place of standing it on the entity's bottom edge; mirrored about
    /// the middle of the hitbox when it faces left, as a sprite-set frame
    /// is. A console that shows a sprite a line below where its table puts
    /// it (the NES) is drawn at `[0, 1]`. Since core 0.4.18.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[i32; 2]>,
    /// Drawn behind the level's picture: its pixels show only where the
    /// level draws its backdrop, as the NES's sprite priority bit does. A
    /// plant in a pipe. A state's `set` can turn it on and off. Since core
    /// 0.4.18.
    #[serde(default, skip_serializing_if = "is_false")]
    pub behind: bool,
    /// Drawn behind the level's picture, as `behind`, on the frames this
    /// holds: a game that sets a sprite's priority bit by where the object
    /// is or what it is doing, as Donkey Kong gives a barrel rolling on
    /// the bottom girder left of x $30 attribute $23, behind the oil drum
    /// ($DFC3). Presentation only. Feature `sprite.behind_when`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind_when: Option<When>,
    /// A part of the picture held where the body is while the rest is
    /// drawn moved by two counters' values: a game that moves one of an
    /// object's hardware sprites and leaves the others where its last
    /// draw put them for a frame, as Donkey Kong moves only Mario's first
    /// sprite on the frame he walks off a girder's end with the hammer
    /// ($D70A/$D710, $D72B) and draws none of the others. Presentation
    /// only. Feature `sprite.split`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<Split>,
    /// Drawn as the picture the body it replaced showed last: a body a
    /// defeat leaves (`health.defeat.becomes`) looks like the enemy it
    /// was, turned over with `flip_y`, as Super Mario Bros.' defeated
    /// enemies fall upside down. A body with nothing to copy (one a start
    /// carried in) draws the sprite's own frame. Since core 0.4.21.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced: Option<Replaced>,
    /// Drawn over every body without it, whatever their order: a console
    /// draws its first sprites over the rest, and Super Mario Bros. puts
    /// Mario's first (Player_SprDataOffset, DefaultSprOffsets' $04).
    /// Since core 0.4.21.
    #[serde(default, skip_serializing_if = "is_false")]
    pub front: bool,
    /// Tiles the game writes into the background's name table rather
    /// than sprites, drawn as a body: Donkey Kong is background tiles, his
    /// poses rewritten in place. The screen measure counts its pixels as
    /// the background's, and the sprite limit doesn't count it. Feature
    /// `sprite.name_table`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub name_table: bool,
    /// Mirrored where it is, as a console flips a sprite by its attribute
    /// bits: a facing or clip flip leaves the picture's left edge at
    /// `offset` instead of turning it about the middle of the hitbox.
    /// Donkey Kong draws Mario facing left at the same $0203 as facing
    /// right. Feature `sprite.flip_in_place`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_in_place: bool,
    /// The picture a body that replaces this one draws, when its sprite
    /// has `replaced`, in place of this one's last picture turned over: a
    /// game that draws a defeated enemy with a frame of its own, as Super
    /// Mario Bros.' koopas fall as an upside-down shell
    /// (CheckUpsideDownShell). Its slot, frame, width, palette, cells and
    /// offset are read. Since core 0.4.21.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub leaves: Option<Box<Sprite>>,
}

/// How a body draws the picture of the body it replaced. Presentation
/// only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Replaced {
    /// The picture mirrored top to bottom.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_y: bool,
}

/// A sprite's blink. Presentation only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Blink {
    #[serde(default, skip_serializing_if = "When::is_always")]
    pub when: When,
    /// Frames shown, then as many hidden.
    pub every: u32,
}

/// A sprite's `split` (feature `sprite.split`): the part `keep` (left,
/// top, width and height in pixels from the picture's top left as drawn,
/// after any flip) stays where the body's picture is, and the rest of the
/// picture is drawn moved by `x` and `y`, counters' values (0 without
/// one). The kept part is drawn over the rest, as a console draws its
/// first sprites over later ones. While both values are 0 the picture is
/// drawn whole as usual.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Split {
    pub keep: [u32; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
}

/// One tile of a sprite's layout. Presentation only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    /// Mirrored left to right.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_x: bool,
    /// Mirrored top to bottom.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_y: bool,
    /// A palette for this tile instead of the sprite's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<u32>,
    /// Where the tile goes, as a column and row of 8-pixel tiles from the
    /// picture's top left, instead of its place in the rows of `width`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[u32; 2]>,
}

/// One palette of a `palettes` slot, by number from 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaletteRef {
    pub slot: String,
    pub palette: u32,
}

/// A run of frames a sprite shows while its `when` holds. Presentation
/// only: it reads the entity's state and never changes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    /// What must hold for the clip to show. Empty: always.
    #[serde(default, skip_serializing_if = "When::is_always")]
    pub when: When,
    /// The frames, shown in turn and then from the first again.
    pub frames: Vec<ClipFrame>,
    /// Frames each picture shows for. Give this or `by_speed`; with
    /// neither, a clip of one frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u32>,
    /// Frames each picture shows for, by the entity's speed: the first
    /// step whose `at_least` its speed reaches (a step without one always
    /// does). The time is picked again each time the picture changes, so a
    /// faster entity steps faster.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_speed: Vec<SpeedStep>,
    /// `by_speed` reads the speed the entity had on the frame before, as
    /// a game that picks its animation's time before it moves. Super Mario
    /// Bros.' GetPlayerAnimSpeed runs before ImposeFriction. Since core
    /// 0.4.18.
    #[serde(default, skip_serializing_if = "is_false")]
    pub speed_before: bool,
    /// What counts the time: `own` (the default), a timer per entity that
    /// starts when the clip does; `game`, the frames the game has run,
    /// so every entity showing the clip changes picture together; or
    /// (since core 0.4.9) `body`, one timer per entity that runs down
    /// whatever clip shows, as a game keeps one animation timer per
    /// object: the clip steps after drawing a picture when the timer has
    /// run out, and sets it again.
    #[serde(default, skip_serializing_if = "Clock::is_own")]
    pub clock: Clock,
    /// Keeps changing pictures while the world is frozen. Without it the
    /// clip stops while a `freeze` state holds the world.
    #[serde(default, skip_serializing_if = "is_false")]
    pub while_frozen: bool,
    /// Never changes picture: it keeps the picture the entity showed when
    /// the clip started, when that is one of its frames, and else shows
    /// its first. A walk cycle that stops in the air is a held clip of the
    /// walk's frames. Since core 0.4.2.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hold: bool,
}

/// Conditions on an entity's state for a clip. Each one given must hold.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// Standing on something (true) or in the air (false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground: Option<bool>,
    /// Moving sideways at all (true) or still (false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moving: Option<bool>,
    /// Moving sideways against the way it faces (true), or not (false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skidding: Option<bool>,
    /// Moving up (true), or not (false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rising: Option<bool>,
    /// Its sideways speed, either way, is at least this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_at_least: Option<Fixed>,
    /// In the air after a jump (true), or not (false): falling off a ledge
    /// is in the air without a jump. Since core 0.4.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jumped: Option<bool>,
    /// In frames where no hurt counts, as after being hurt (true), or not
    /// (false). Since core 0.4.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invulnerable: Option<bool>,
    /// On a ladder or vine (true), or not (false). Since core 0.4.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub climbing: Option<bool>,
    /// One of the entity's `ext` hook's words is in a range, such as
    /// Super Mario Bros.' Lakitu ducking into its cloud while its copy of
    /// FrenzyEnemyTimer is below 16. Since core 0.4.21.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<WordTest>,
    /// More word tests, each of which must hold too, for a hook that keeps
    /// what a picture depends on in two words (Donkey Kong's barrels: their
    /// state and their girder). Feature `clip.when_words`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<WordTest>,
    /// The model's counters are in ranges, each one given, for a picture
    /// the game picks by a timer or a flag the model keeps as a counter
    /// (Donkey Kong's oil drum burns while `drum` is above 0). Presentation
    /// reads the counters; nothing in the simulation reads the clip.
    /// Feature `clip.when_counter`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counters: Vec<CounterTest>,
    /// Counters named by the body's slot, each in its range: the test's
    /// `name` followed by the number of the slot the body is in (Donkey
    /// Kong's `barrel_tile` names `barrel_tile0` for the barrel in slot
    /// 0), for a picture the game keeps per object slot. A body in no slot
    /// fails it. Presentation only. Feature `clip.when_slot_counter`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slot_counters: Vec<CounterTest>,
    /// The entity's x in whole pixels is at least this, for a picture the
    /// game picks by where the object is on the screen (Donkey Kong's
    /// barrel goes behind the oil drum left of x $30, $DFC3). Presentation
    /// only. Feature `clip.when_x`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_at_least: Option<i64>,
    /// The entity's x in whole pixels is below this. Feature
    /// `clip.when_x`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_below: Option<i64>,
}

/// A test on one of the model's counters. Presentation only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CounterTest {
    /// The counter, by name.
    pub name: String,
    /// The counter is at least this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<i64>,
    /// The counter is below this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<i64>,
}

/// A test on one of a body's `ext` hook words. Presentation only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WordTest {
    /// The word, 0 to 3.
    pub index: u8,
    /// The word is at least this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<i64>,
    /// The word is below this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<i64>,
}

impl When {
    pub fn is_always(&self) -> bool {
        *self == When::default()
    }
}

/// One frame of a clip: a row of the sprite's `frame` slot or tile
/// numbers, or else the sprite's own frame, with flips and a layout.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipFrame {
    /// A row of the slot the sprite's `frame` names, or of `slot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<u32>,
    /// The `rows` slot `row` is read from, in place of the sprite's frame
    /// slot, for a game that keeps its pictures in tables of their own
    /// (Donkey Kong's poses, each behind a size byte). Feature
    /// `clip.frame_slot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    /// Tile numbers, row by row, in the sprite's `width`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<u32>,
    /// The whole picture mirrored left to right, on top of any facing flip.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_x: bool,
    /// The whole picture mirrored top to bottom.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_y: bool,
    /// Its tiles' flips, palettes and places, as the sprite's `cells`;
    /// without them, the sprite's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<Cell>,
    /// Its picture's top left from the entity's position, as the sprite's
    /// `offset`, for a game that places each picture of a turn apart
    /// (Super Mario Bros.' hammer, DrawHammer). Without it, the sprite's.
    /// Since core 0.4.21.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[i32; 2]>,
    /// Frames this picture shows for, in place of the clip's `every`, for
    /// a game that holds some pictures longer than others (Donkey Kong's
    /// Pauline calls for help in eight 16-frame steps, then waits). Every
    /// frame of the clip gives one, or none does. Feature
    /// `clip.frame_every`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u32>,
    /// Its colors, as the sprite's `colors`, for a game that writes a
    /// sprite palette while something lasts (Donkey Kong's 100m turns the
    /// fires' palette blue from a hammer's touch to its end, $D89A and
    /// $D7F2). Without it, the sprite's. Feature `clip.frame_colors`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<PaletteRef>,
}

/// Frames per picture at and above a speed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeedStep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<Fixed>,
    pub every: u32,
}

/// What counts a clip's time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Clock {
    #[default]
    Own,
    Game,
    /// Since core 0.4.9.
    Body,
}

impl Clock {
    pub fn is_own(&self) -> bool {
        *self == Clock::Own
    }
}

/// One row of a lifted table, by number from 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RowRef {
    pub slot: String,
    pub row: u32,
}

/// How a level looks: presentation only, which the simulation never reads
/// and a theme may replace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelPresent {
    /// A slot of kind `palettes`: the level's palettes, background first,
    /// then sprites. Without it the level has the model's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palettes: Option<String>,
    /// The background color: a row of a `table` slot whose first column is
    /// a color number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<RowRef>,
    /// The level's name as the HUD shows it, such as "1-1".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Palette colors that change over time in this level, instead of the
    /// model's `present.cycles`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cycles: Vec<ColorCycle>,
    /// Feature `present.sprite_order`: each entity's place in the
    /// console's sprite table in this level, by entity name, which the
    /// sprite limit (`present.screen.sprite_limit`) reads: on a line past
    /// the limit the sprites that come last in the table are the ones not
    /// drawn. Entities not named come after those named, in body order.
    /// Presentation only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sprite_order: BTreeMap<String, SpriteOrder>,
    /// Feature `present.effect_order`: each effect's place in the same
    /// table, by the event that starts it, so it is painted under or over
    /// the bodies as the console draws it. Donkey Kong's points sprites
    /// are $02C0 and $02C8 ($CFC6), sprites 48 and 50, over the hammers'
    /// 52 and under the barrels' 12 on. Presentation only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub effect_order: BTreeMap<String, EffectOrder>,
}

/// Feature `present.effect_order`: an effect takes the table's sprites
/// from `at + step * k`, k being the place of the counter its phase took
/// among the phase's `until_counters` (the points sprite a game with two
/// gave it), else 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectOrder {
    /// The first hardware sprite's number in the table (0 to 63).
    pub at: u32,
    /// Hardware sprites each of its places takes (default 4).
    #[serde(default = "four")]
    pub step: u32,
}

/// Feature `present.sprite_order`: body k of an entity takes the table's
/// sprites from `at + step * k`, k being its slot (its `ext` word `word`
/// less one, for a hook that keeps the slot there, when set; else the
/// level's numbered spawn slot it is in) or else its count among the
/// entity's drawn bodies before it. Donkey Kong's 25m keeps Mario at 0,
/// the fires from 4, the barrels from 12 ($0230 + 16 * X), the hammers
/// at 52, the drum's flame at 56 and Pauline at 58. Original Edition's 50m
/// lays its table out as Mario 0, the fires from 4 ($0210, 16 bytes a
/// slot, $EFE7), the pies from 12 ($0230 + 16 * $5D, $EFD5), the moving
/// ladders from 48 ($02C0 + 16 * X, $8899) and the hammers at 60.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpriteOrder {
    /// The first hardware sprite's number in the table (0 to 63).
    pub at: u32,
    /// Hardware sprites a body takes (default 4).
    #[serde(default = "four")]
    pub step: u32,
    /// The `ext` word that holds the body's slot plus one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<u32>,
    /// The game copies the slots into the table starting from a slot that
    /// turns: k is the slot less this counter's value, around `count`
    /// slots. Original Edition's 50m copies its seven pie slots from
    /// $0493, one further each frame ($895F), so the pie a crowded line
    /// drops takes turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<SpriteTurn>,
}

/// Feature `present.sprite_order`: the counter whose value is the first
/// slot copied, and the slots' count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpriteTurn {
    pub counter: String,
    pub count: u32,
}

fn four() -> u32 {
    4
}

/// How the game is shown. Presentation only: the simulation never reads
/// it, and a theme may replace any of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Present {
    /// The picture the game puts out: its size, its pixels' shape and the
    /// edges a television hides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<Screen>,
    /// The heads-up display drawn over the view, such as a status bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hud: Option<Hud>,
    /// Palette colors that change over time in every level whose `present`
    /// doesn't give its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cycles: Vec<ColorCycle>,
    /// What plays: music by level and by rule state, sound effects by
    /// event. Since core 0.4.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<Sound>,
    /// The feature `present.cells`: how a cell the game changes is drawn (a coin
    /// taken, a block used), and a held cell's bump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<CellsPresent>,
    /// The feature `present.effects`: short-lived pictures the model's events start,
    /// such as a coin popping out of a block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectPresent>,
    /// Feature `present.screens`: whole screens shown in place of the
    /// level, its bodies and the HUD while a rule state holds, such as a
    /// title and its menu.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screens: Vec<StateScreen>,
    /// Feature `present.hidden`: bodies the game doesn't draw while a
    /// rule state holds and counters are in ranges, though they are
    /// still in play, such as Donkey Kong's sprites from a death's 13th
    /// divider wrap ($F08C) until the 21st draws the stage's objects
    /// again ($CC34).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<Hidden>,
}

/// Bodies not drawn while one of the rule states holds and every counter
/// test does (feature `present.hidden`). Presentation only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hidden {
    /// The rule states, as `<machine>.<state>`.
    pub states: Vec<String>,
    /// The model's counters in ranges, each of which must hold too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<CounterTest>,
    /// The entity types hidden, whichever layer they are drawn in. None
    /// given: the console's sprite layer, every body but those drawn as
    /// name-table tiles and the HUD's sprites, as `present.sprites_off`,
    /// unless `hud` is given.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// The HUD not drawn either: its layer, its fields and its sprites,
    /// as Donkey Kong's 100m ending blanks the status rows ($C6E4, entry
    /// $0A of $C03C). With no `kinds`, only the HUD.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hud: bool,
}

/// A screen shown in place of the level while a rule state holds
/// (feature `present.screens`): its picture, drawn from update strings as
/// the game writes its name table and palettes, and sprites fixed on it,
/// each moved by a counter. Donkey Kong's title is the string the title
/// routine draws ($C807 with 8) and a cursor sprite 16 lines down a menu
/// line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StateScreen {
    /// The rule states it shows in, as `<machine>.<state>`.
    pub states: Vec<String>,
    /// The model's counters in ranges, each of which must hold too
    /// (feature `present.screens_when`): Donkey Kong's screen is blank
    /// while a death's wait $43 is in the stage setup's part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<CounterTest>,
    /// An entry of a `screen-text` (update strings) slot: the screen's
    /// name table, attributes and palettes as the game writes them. The
    /// palettes it doesn't write are the first level's. A screen gives it
    /// or `blank`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<EntryRef>,
    /// The screen is the first level's backdrop color alone, as a console
    /// shows with its picture turned off, in place of a layout (feature
    /// `present.screens_blank`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blank: bool,
    /// With `blank`: the screen line the picture goes off at (feature
    /// `present.screens_from_line`). The lines above it show the frame as
    /// it would be drawn without this screen, the level, its bodies and
    /// the HUD, and the lines from it the backdrop, as a console whose
    /// picture is turned off partway down the frame. Donkey Kong's stage
    /// clear turns it off at line 34 ($CBB7 -> $D19A on the clear script's
    /// $7A step), so the status rows show on that frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_line: Option<u32>,
    /// Sprites over it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sprite: Vec<ScreenSprite>,
    /// Text written into its name table over the layout or the blank
    /// (feature `present.screens_text`), in the characters of the HUD's
    /// font (`present.hud.font`): Donkey Kong's turn screen, PLAYER I or
    /// PLAYER II on a cleared name table ($CBCA, $C6AA).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<ScreenText>,
}

/// A line of text on a [`StateScreen`], written into its name table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenText {
    /// The first character's tile column and row.
    pub at: [u32; 2],
    /// The characters, each one of the HUD font's.
    pub text: String,
    /// The background palette its cells are drawn in, 0 to 3, in place
    /// of the attribute bytes'. Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<u32>,
}

/// One sprite tile on a [`StateScreen`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenSprite {
    /// The tile's top left corner, in screen pixels, with the counter at 0.
    pub at: [i32; 2],
    /// A slot of kind `tiles`: the sprite tiles.
    pub tiles: String,
    /// The tile's number in that slot.
    pub tile: u32,
    /// The sprite palette, 0 to 3. Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<u32>,
    /// A counter that moves it: `step` pixels for each 1 of its value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<String>,
    /// Pixels across and down for each 1 of the counter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<[i32; 2]>,
}

/// How the cells the game changes are drawn (the feature `present.cells`). Presentation
/// only: the simulation changes the cells' kinds, and this says which
/// picture each new kind shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellsPresent {
    /// By tile kind (`empty`, `solid`, ...): the metatile a cell changed
    /// to that kind is drawn with, by its number in the level's tileset
    /// (a grid level's `metatiles` slot, or a layout's metatile sets).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kinds: BTreeMap<String, u32>,
    /// The metatile a cell shows while a head hit holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<u32>,
    /// While a head hit holds a cell, the picture it will show (or its
    /// own, when it stays as it was) rises and falls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bump: Option<BumpPresent>,
}

/// A held cell's bump: how it moves, where its picture is, and the
/// picture by the kind it will have, as a game draws the bouncing block
/// as sprites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BumpPresent {
    pub speed: Fixed,
    pub gravity: Fixed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fall: Option<Fixed>,
    /// The picture's top left in pixels from the cell's.
    #[serde(default, skip_serializing_if = "is_zero2")]
    pub offset: [i32; 2],
    /// By the tile kind the cell will have: the bump's picture, tiles
    /// from a `tiles` slot as a sprite's (`slot`, `tiles`, `width`,
    /// `palette`, `cells`). A kind without one draws the cell's metatile
    /// with its backdrop clear.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sprites: BTreeMap<String, TilesPicture>,
}

/// A picture of tiles, as a sprite lays them out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TilesPicture {
    /// A slot of kind `tiles`.
    pub slot: String,
    /// Tile numbers, row by row.
    pub tiles: Vec<u32>,
    /// Tiles per row.
    pub width: u32,
    /// A palette of the level's palettes (4 to 7 for the sprite ones).
    pub palette: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<Cell>,
}

/// A move up and down: each frame the place goes by `speed`, then the
/// speed by `gravity`, at most `max_fall`. Down is positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MotionPresent {
    pub speed: Fixed,
    pub gravity: Fixed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fall: Option<Fixed>,
}

/// Where an effect starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EffectAt {
    /// The cell a head hit took on the event's frame.
    Hit,
    /// The event's entity's position.
    Entity,
}

/// A short-lived picture an event starts (the feature `present.effects`): its phases in
/// turn, then gone. Presentation only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectPresent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// An event of the model.
    pub event: String,
    pub at: EffectAt,
    /// The picture's top left in pixels from the cell's top left or the
    /// entity's position.
    #[serde(default, skip_serializing_if = "is_zero2")]
    pub offset: [i32; 2],
    pub phases: Vec<EffectPhase>,
}

/// One phase of an effect: its pictures, how it moves and when it ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectPhase {
    /// A slot of kind `tiles`.
    pub slot: String,
    /// The pictures, each a list of tile numbers row by row, shown in turn
    /// by the game's frames.
    pub frames: Vec<Vec<u32>>,
    /// Frames each picture shows for. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u32>,
    /// Tiles per row.
    pub width: u32,
    /// A sprite palette of the level's palettes, as a sprite's.
    pub palette: u32,
    /// Each tile's flips, palette and place, as a sprite's `cells`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<Cell>,
    /// How it moves up and down. The first phase's speed is the start's;
    /// a later phase with a motion starts at its own speed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<MotionPresent>,
    /// Rises a pixel on each frame whose count is a multiple of this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rise_every: Option<u32>,
    /// Moves across as the camera does, so it keeps its place on screen.
    #[serde(default, skip_serializing_if = "is_false")]
    pub with_camera: bool,
    /// Ends on the frame its speed's whole part reaches this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_speed: Option<Fixed>,
    /// Ends after this many frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u32>,
    /// Goes on while a state freezes the world, as a game object the
    /// freeze doesn't stop; its pictures and rise count the world's
    /// frames.
    #[serde(default, skip_serializing_if = "is_false")]
    pub while_frozen: bool,
    /// Ends on the frame after one on which this counter is 0 or less, as
    /// a game that hides a sprite by a timer it reads before the frame's
    /// timers step (feature `present.effects.until_counter`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_counter: Option<String>,
    /// As `until_counter`, with the one of these counters that is highest
    /// as the phase starts: the timer the event's picture took, for a game
    /// that shows it in whichever of several slots is free (feature
    /// `present.effects.until_counters`). Donkey Kong's points sprites:
    /// `points_timer` and `points_timer_2` ($41, $42; $CFC6 takes the
    /// first free).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub until_counters: Vec<String>,
    /// Ends on the frame the event's entity is gone, and the next phase
    /// starts on that frame (feature `present.effects.until_gone`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub until_gone: bool,
}

fn is_zero2(v: &[i32; 2]) -> bool {
    *v == [0, 0]
}

/// Music and sound effects, bound to the model. Presentation only: no
/// state hash depends on them, and a theme may restyle them by the `sound`
/// role group.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sound {
    /// Songs and the levels or rule states they play in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub music: Vec<MusicBinding>,
    /// Effects and the events they mark.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sfx: Vec<SfxBinding>,
    /// Feature `present.sound_roles`: the game's sound requests named as
    /// sound roles, each cued by a counter that holds the request. A theme
    /// replaces a role's sound with a file (docs/formats/theme.md, "Sound
    /// replacements"), played on the role's cue in place of what the
    /// model's own sound plays.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<SoundRole>,
    /// Feature `present.sound_silent_while`: counter tests while any of
    /// which the game makes no sound, though it asks for some (an attract
    /// demo that sets its tune's request with the sound channels off). A
    /// theme's sound per role is silent then too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub silent_while: Vec<SoundTest>,
}

/// A counter test for the sound (feature `present.sound_silent_while`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundTest {
    pub counter: String,
    /// The bits tested (default every bit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<i64>,
    /// The masked counter equals this (default: any value but 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<i64>,
    /// Why, in plain words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A sound role (feature `present.sound_roles`): a request the game makes,
/// named, with the counter test that marks it. The cue reads the world
/// after a tick, as every other presentation does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundRole {
    /// The role's name: lowercase letters, digits and `_`, such as `jump`.
    pub name: String,
    /// `music` or `sfx`.
    pub kind: SoundRoleKind,
    /// The counter that holds the request.
    pub counter: String,
    /// The bits of the counter tested (default every bit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<i64>,
    /// The masked counter equals this for the cue (default: any value but
    /// 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<i64>,
    /// The request stays set while the sound plays (a held tune): the cue
    /// starts the role on the first frame the test holds and stops it on
    /// the first that it doesn't. Otherwise the game clears the request
    /// each frame, and every frame the test holds starts the role again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
    /// The sound repeats until another music role starts or a held role
    /// stops.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub loops: bool,
    /// Its usual length in frames, as a theme maker's guide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u32>,
    /// When it plays, in plain words and without any character's name: a
    /// theme maker's prompt starts from it.
    pub description: String,
}

/// What a sound role is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SoundRoleKind {
    Music,
    Sfx,
}

/// A song in a `songs` slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SongRef {
    pub slot: String,
    /// The song's name in the slot's set.
    pub song: String,
}

/// A song that plays while a level plays, or while a rule state holds (a
/// state's music takes over from the level's).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MusicBinding {
    #[serde(flatten)]
    pub song: SongRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// `<rules machine>.<state>`, such as `game.dying`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// An effect played when an event fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SfxBinding {
    pub event: String,
    pub slot: String,
    /// The effect's name in the slot's set.
    pub effect: String,
}

/// The screen a game draws to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Screen {
    /// Width and height in pixels, as the console puts them out (256 by
    /// 240 for the NES).
    pub size: [u32; 2],
    /// A pixel's width over its height on the display the game was made
    /// for ("8/7" for the NTSC NES). Default "1".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_aspect: Option<Ratio>,
    /// Pixels hidden at the left, top, right and bottom edges, as a
    /// television's frame hides them. Default none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overscan: Option<[u32; 4]>,
    /// Since core 0.4.14: the console draws at most this many sprites on
    /// a line, and the game's own sprites past it drop out or flicker.
    /// Presentation only; a player can turn it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite_limit: Option<SpriteLimit>,
    /// Rule states, each `<machine>.<state>`, in which the console's
    /// sprite layer is off: the level, the HUD's tiles and the bodies
    /// drawn as name-table tiles show, and no sprite does. Donkey Kong
    /// turns its sprites off once a Start pause's 64 frames are out
    /// ($CFA1 sets bit 4 of $11, which the NMI writes to $2001 inverted).
    /// Presentation only. Feature `present.sprites_off`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sprites_off: Vec<String>,
    /// The console's sprite x is a byte, so a sprite the game places left
    /// of the screen (x below 0) shows at x + 256, at the right edge: each
    /// hardware sprite of a body's picture (8 pixels across) whose left
    /// column is left of the screen is drawn 256 pixels on. Donkey Kong's
    /// hammer swung left of Mario at the screen's left edge puts its head
    /// at x 255 ($02D3 wraps). Presentation only, for a game that never
    /// scrolls. Feature `present.sprite_x_wraps`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sprite_x_wraps: bool,
}

/// How many sprites the console draws on one line (core 0.4.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpriteLimit {
    /// Hardware sprites a line holds (8 on the NES).
    pub per_line: u32,
    /// A hardware sprite's width in pixels (default 8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite_width: Option<u32>,
    /// The game turns its sprites' order each frame, so the ones past the
    /// limit take turns and flicker (default false: the same ones drop).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flicker: bool,
}

/// A heads-up display: fixed tiles and fields bound to counters and the
/// level, drawn in screen positions over the view. Its role is `hud`, for
/// themes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hud {
    /// A slot of kind `tiles`: the tiles the HUD is drawn with.
    pub tiles: String,
    /// A slot of kind `font`: the tile for each character the fields show.
    pub font: String,
    /// An entry of a `screen-text` slot: the HUD's fixed tiles and their
    /// palettes, such as the status bar's names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<EntryRef>,
    /// Width and height in tiles of the part of the screen the HUD covers,
    /// from the top left. Default the whole screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[u32; 2]>,
    /// Text drawn from the game's state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field: Vec<HudField>,
    /// Pictures the game draws as sprites over its status bar, such as the
    /// one it uses to split the screen. Since core 0.4.7.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sprite: Vec<HudSprite>,
}

/// One sprite tile fixed on the screen, drawn over the HUD. The screen
/// measure counts its pixels as the sprite layer's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HudSprite {
    /// The tile's top left corner, in screen pixels.
    pub at: [i32; 2],
    /// A slot of kind `tiles`: the sprite tiles.
    pub tiles: String,
    /// The tile's number in that slot.
    pub tile: u32,
    /// The sprite palette, 0 to 3. Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<u32>,
    /// Behind the background, as the NES's priority bit puts a sprite: its
    /// pixels show only where the HUD's fixed layer draws nothing.
    #[serde(default, skip_serializing_if = "is_false")]
    pub behind: bool,
}

/// One piece of HUD text: a counter, the level's label or fixed text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HudField {
    /// Column and row of its first character, in tiles from the screen's
    /// top left.
    pub at: [u32; 2],
    /// A counter, by name, shown as a number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<String>,
    /// Shows the current level's `present.label`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub level_label: bool,
    /// Fixed text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// For a counter: how many digits, padded on the left with `pad`. A
    /// number too long for them shows its last digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digits: Option<u32>,
    /// For a counter: shown divided by this, rounding down, for a game
    /// that keeps a number's last digit fixed in the HUD's layer (a score
    /// whose ones are always 0). Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divide: Option<u32>,
    /// The character a counter is padded with. Default "0".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<String>,
    /// The background palette its tiles use, or with `sprite` the sprite
    /// palette. Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<u32>,
    /// Drawn as sprites, one 8x8 sprite a character, as a game that puts
    /// its counters in the sprite table does (Moss Hop): the screen
    /// measure counts its pixels as the sprite layer's, like a HUD sprite.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sprite: bool,
    /// Feature `present.hud.field_when`: drawn only while every one of
    /// these counter tests holds. Donkey Kong draws player 2's label and
    /// score only in a two-player game, and each player's score in its own
    /// place whoever's turn it is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<Guard>,
}

/// A palette color that steps through a list over time, as a game's
/// palette rotation does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColorCycle {
    /// The background palette, 0 to 3.
    pub palette: u32,
    /// The color within it, 1 to 3.
    pub index: u32,
    /// A `table` slot whose first column lists the colors in turn.
    pub colors: String,
    /// Frames each color shows for.
    pub every: u32,
    /// Since core 0.4.7: a counter whose value picks the color instead of
    /// the frame count, for a game that keeps its color rotation's place in
    /// a variable (a mapping can then carry the place into a clip's start).
    /// The cycle's own `every` is then ignored. Give the counter a `max`
    /// of the list's length less one, `wrap`, and an `every` change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<String>,
    /// With `counter`: added to its value (modulo the list's length) to
    /// find the color shown, for a game whose counter has already stepped
    /// past the color on screen (-1). Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<i32>,
}

/// A level: core fields plus fields from extensions.
#[derive(Debug, Clone, PartialEq)]
pub struct Level {
    pub core: LevelCore,
    pub platformer: platformer::LevelFields,
    pub topdown: topdown::LevelFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelCore {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A slot of kind `level-grid`: the level's tiles. A level has either
    /// a grid or a `layout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<String>,
    /// An entry of a `level-objects` slot: the objects that build the
    /// level's tiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<EntryRef>,
    /// Feature `level.empty`: a level of `size` empty cells, with no grid
    /// and no layout. A new game's skeleton (`gp game new`) starts with one,
    /// so its model runs and its clips can be scored before the level is
    /// lifted from the game.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub empty: bool,
    /// A slot of kind `metatiles`: what each grid number is. Needed with
    /// `grid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tileset: Option<String>,
    /// The tileset property that makes a tile solid. Needed with `grid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_property: Option<String>,
    /// With `grid`: the tileset property whose value `tile_kinds` reads,
    /// instead of solid or not by `solid_property`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_property: Option<String>,
    /// The kind of tile each value is: of `kind_property` for a grid, or of
    /// the object format's collision class for a layout (Mega Man 2's
    /// stage classes). Keys are the values in decimal. A value not listed
    /// keeps the kind the format gives it (solid or empty).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tile_kinds: BTreeMap<String, TileKind>,
    /// Feature `level.tile_surfaces`: for a tile value, the pixel row
    /// inside its cell where a top-only or ladder tile stands (0 is the
    /// cell's top). A body standing on such tiles follows the surface as
    /// it walks, a pixel at a time (Donkey Kong's sloped girders). Keys are
    /// tile values in decimal, as `tile_kinds`'.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tile_surfaces: BTreeMap<String, u8>,
    /// The level's place in a stage of several levels (rooms), in pixels:
    /// what a state mapping adds to a position in it to compare with a
    /// game that counts positions across the whole stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<[Fixed; 2]>,
    /// How the cameras move in this level, where it differs from each
    /// camera's own settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<LevelCamera>,
    /// Feature `level.halfway`: where `restart-level` starts this level again,
    /// rather than the level last started, once the view got far enough
    /// in it (a checkpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub halfway: Option<Halfway>,
    /// Width and height of a grid cell in pixels.
    pub tile_size: u32,
    /// Width and height in grid cells.
    pub size: [u32; 2],
    /// Where entities start, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spawn: Vec<Spawn>,
    /// An entry of a `spawn-objects` slot: more entities, placed by the
    /// slot's `entities`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_objects: Option<EntryRef>,
    /// Entities other than the player appear when their spawn's x comes
    /// within this many pixels right of the view (along y, below it, when
    /// the level's camera follows y only). Without it every entity appears
    /// when the level starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_ahead: Option<Fixed>,
    /// Entities other than the player go when their right edge is this many
    /// pixels left of the view. Without it they stay. Along y (their bottom
    /// edge above the view) when the level's camera follows y only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub despawn_behind: Option<Fixed>,
    /// Entities other than the player go when their left edge is this many
    /// pixels right of the view's right edge (along y: their top below the
    /// view's bottom). Without it they stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub despawn_ahead: Option<Fixed>,
    /// An entity that leaves by `despawn_behind` or `despawn_ahead` waits at
    /// its spawn to appear again when the view comes back within
    /// `spawn_ahead` of it, rather than going for good.
    #[serde(default, skip_serializing_if = "is_false")]
    pub respawn: bool,
    /// At most this many entities other than the player (and those with
    /// `no_slot`) are in the level at once: one whose spawn comes within
    /// reach of the view while all are taken waits, and a `drawn` one is
    /// left out. Super Mario Bros.' five enemy slots. Since core 0.4.10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_slots: Option<u32>,
    /// What `despawn_behind` and `despawn_ahead` measure from the view's
    /// edges: `edge` (the default), the entity's far edge behind and its
    /// near edge ahead; or `position`, its position's whole pixels both
    /// ways (Super Mario Bros.' OffscreenBoundsCheck compares
    /// Enemy_X_Position). Since core 0.4.10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub despawn_at: Option<DespawnAt>,
    /// `despawn_behind` and `despawn_ahead` are tested after the enemies
    /// system has moved everything, rather than in the `spawn` system:
    /// Super Mario Bros.' RunNormalEnemies ends in OffscreenBoundsCheck, so
    /// an enemy goes on the frame its own step takes it past the line.
    /// Since core 0.4.18.
    #[serde(default, skip_serializing_if = "is_false")]
    pub despawn_after_move: bool,
    /// Entities appear by the view in the order of their spawns (the
    /// `drawn` ones aside), at most one for each free slot a frame: one
    /// waits for those before it, and one whose spawn is already left of
    /// the view's last column when its turn comes never appears. Super
    /// Mario Bros.' ProcessEnemyData. Since core 0.4.10.
    #[serde(default, skip_serializing_if = "is_false")]
    pub spawn_in_order: bool,
    /// An entity that appears by the view first moves on the frame after,
    /// as a game that fills an enemy slot instead of running it does.
    /// Since core 0.4.10.
    #[serde(default, skip_serializing_if = "is_false")]
    pub appear_still: bool,
    /// With `spawn_slots`, the slots are numbered and keep what their last
    /// entity left: an entity that appears takes the lowest free one and
    /// starts with that entity's speed up and down and the fraction of
    /// its place across, as Super Mario Bros.' enemy init routines leave
    /// Enemy_Y_Speed and Enemy_X_MoveForce as the slot had them. A level
    /// starts with them 0. The slots are world state. Since core 0.4.11.
    #[serde(default, skip_serializing_if = "is_false")]
    pub slot_rest: bool,
    /// The width, in pixels, an entity's touches use in this level in place
    /// of its hitbox's, by entity name, as Super Mario Bros.' SPBBox picks
    /// a narrower box for lifts in castles and hard mode.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hit_widths: BTreeMap<String, u32>,
    /// The level is drawn ahead of the view in sets of columns, and a
    /// spawn marked `drawn` appears as its column is drawn. Since core
    /// 0.4.10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw: Option<Draw>,
    /// With `draw`: what drawing one column does. Super Mario Bros.'
    /// ScrollLockObject_Warp, a warp zone's area object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_stop: Option<ScrollStop>,
    /// With `draw`: places the level tests the player as it is drawn, and
    /// sends them back if they aren't where it wants them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_back: Option<LoopBack>,
    /// Values counters take when the level starts, by counter name, such as
    /// the time limit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counters: BTreeMap<String, i64>,
    /// The level the `next-level` action goes to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// Ways to other levels besides `next`, for the progression graph.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exit: Vec<Exit>,
    /// How the level looks. Presentation only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present: Option<LevelPresent>,
}

/// A way from one level to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Exit {
    /// The level it leads to.
    pub to: String,
    pub via: Via,
    /// What takes it. Without it the exit is only in the progression
    /// graph and nothing in the game takes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<Take>,
    /// Where the player comes out in `to`, its top left corner. Without it,
    /// the player's spawn there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter: Option<Enter>,
    /// Taking it starts `to` as a level, as `next-level` does (its counter
    /// values, `level.started`, and `restart-level` starts it again),
    /// rather than entering it as a room of the level being played. A
    /// pipe intro's pipe and a warp zone's pipes are.
    #[serde(default, skip_serializing_if = "is_false")]
    pub start: bool,
    /// The player's coordinate on this axis keeps its place in the view
    /// across the exit (its position less the view's corner), as a room
    /// change that scrolls does; `enter` gives the other one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<KeepAxis>,
    /// The exit takes this many frames, as a room change that scrolls the
    /// view does: the world is frozen and the player held until they're
    /// over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What takes an exit: the player's hitbox overlapping `at`, with `hold`
/// held, and standing if `standing`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Take {
    /// A box in the level, in pixels.
    pub at: Rect,
    /// An input action that must be held, such as `down` on a pipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub standing: bool,
    /// A counter condition that must also hold, as a transition's `if`:
    /// a door that needs a key.
    #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
    pub guard: Option<Guard>,
}

/// Which coordinate an exit keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeepAxis {
    X,
    Y,
}

/// What kind of tile a cell is. The simulation collides by kind: `solid`
/// stops from every side, `top` only an entity moving down onto its top
/// row, `ladder` is climbed (and its top stands, for a climber that says
/// so), `hazard` touches the player as the level's `hazard` says, `water`
/// and `ice` change falling and running as the level's `water` and `ice`
/// say. Since core 0.4.6, `coin_block`, `item_block` and `brick` are
/// solid like `solid`, and a head hit (platformer `jump.head`) tells them
/// apart: a block with a coin in it, one with an item, a brick. Since core
/// 0.4.16, `coin` stops nothing and a pickup (platformer `jump.pickup`)
/// takes it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum TileKind {
    Empty,
    Solid,
    Top,
    Ladder,
    Hazard,
    Water,
    Ice,
    #[serde(rename = "coin_block")]
    CoinBlock,
    #[serde(rename = "item_block")]
    ItemBlock,
    Brick,
    Coin,
}

impl TileKind {
    /// The kind by its name in a model, such as `coin_block`.
    pub fn parse(name: &str) -> Option<Self> {
        use serde::de::IntoDeserializer;
        let d: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
            name.into_deserializer();
        Self::deserialize(d).ok()
    }

    /// Whether the kind is one core 0.4.6 added.
    pub fn since_0_4_6(self) -> bool {
        matches!(self, Self::CoinBlock | Self::ItemBlock | Self::Brick)
    }
}

/// A level's checkpoint (feature `level.halfway`): when `restart-level` starts the
/// level again while the first camera's x is at least `from`, that camera
/// starts at `from` and the player at `at`, and the entities come as the
/// view reaches them from there. Super Mario Bros.' PlayerLoseLife sets
/// HalfwayPage from HalfwayPageNybbles when ScreenLeft_PageLoc has reached
/// the level's page, and InitializeArea starts the area at that page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Halfway {
    pub from: Fixed,
    pub at: [Fixed; 2],
}

/// What despawning by the view measures (core 0.4.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DespawnAt {
    /// The far edge behind the view, the near edge ahead of it.
    Edge,
    /// The entity's position, in whole pixels, both ways.
    Position,
}

/// How a level is drawn ahead of the view (Super Mario Bros.' area
/// parser): when it starts, every column left of `ahead` pixels past the
/// view's far edge; then, each time the view's near edge passes a whole
/// `step` (counted from the level's left edge), the next `step` pixels of
/// columns, one column every `every` frames from that frame. Since core
/// 0.4.10.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Draw {
    /// Pixels drawn past the view's far edge when the level starts.
    pub ahead: Fixed,
    /// Pixels of columns in a set: a whole number of tiles.
    pub step: u32,
    /// Frames from one column of a set to the next.
    pub every: u32,
    /// Frames after a set starts on which no entity appears by the view
    /// but `drawn` ones (Super Mario Bros.' enemy loop leaves while
    /// AreaParserTaskNum is 7).
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub pause: u32,
}

/// A level's column that stops the scroll when it is drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScrollStop {
    /// Where the column is along the level's axis, in pixels. From the
    /// frame after it is drawn no camera moves in this level.
    pub at: Fixed,
    /// Entities of these kinds in the level as the column is drawn go,
    /// with no event (KillEnemies clears the enemy slots holding one).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removes: Vec<String>,
}

/// A level that loops: when its draw finishes a check's column with a slot
/// free, a player who isn't standing at the check's height goes back
/// `back` pixels with the view and the draw, the level's other entities go
/// and wait at their spawns (a `drawn` spawn behind the draw never comes
/// back), and the last `keep` columns drawn go back with the view. Super
/// Mario Bros.' castle mazes (ProcLoopCommand, ExecGameLoopback).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopBack {
    /// Pixels back: a whole number of the draw's steps and of tiles.
    pub back: u32,
    /// Columns behind the draw that go back with the view: the columns the
    /// game's tile buffer holds.
    pub keep: u32,
    pub checks: Vec<LoopCheck>,
}

/// One place a [`LoopBack`] tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopCheck {
    /// The column, in pixels, the draw has reached.
    pub x: u32,
    /// The player's whole pixels down that pass, standing.
    pub y: u32,
}

/// A level's camera settings, for every camera while that level plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelCamera {
    /// The axes it follows along, instead of the camera's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<Axis>,
    /// The lead, instead of the camera's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<[Fixed; 2]>,
    /// The most pixels it moves in a frame; without it, it moves at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<Fixed>,
    /// While it's more than `step` from where it's going, the followed
    /// entity doesn't move: a game that scrolls in steps and holds the
    /// player meanwhile.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hold_player: bool,
    /// Feature `level.camera.keep_in_view`: the followed entity's bounds
    /// in the view across while this level plays, instead of the camera's
    /// `keep_in_view` and its `keep_in_view_rows`. Donkey Kong's walk
    /// stops at a stage's own edges ($D99D reads them by $53 from $C1B4):
    /// sprite x 16 to 224 on 25m, 8 to 232 on 50m.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_in_view: Option<[Fixed; 2]>,
    /// Feature `level.camera.keep_in_view_rows`: with `keep_in_view`, other
    /// bounds while the followed entity's row is in a range, as a camera's
    /// `keep_in_view_rows`. Donkey Kong's $D990 stops a walk left at sprite
    /// x 104 on 75m's fifth girder as on 25m's sixth ($D9C3-$D9DB).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keep_in_view_rows: Vec<KeepRow>,
}

/// Where the player comes out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Enter {
    pub x: Fixed,
    pub y: Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Via {
    /// A pipe or door entered by pressing a direction on it.
    Pipe,
    /// A vine or ladder climbed out of the view.
    Vine,
    /// A pipe or door that skips ahead past the next level.
    Warp,
    /// Walking or falling off an edge of the level.
    Edge,
}

/// Fields of [`LevelCore`]; a test checks the list against the type.
pub const LEVEL_CORE_FIELDS: &[&str] = &[
    "description",
    "grid",
    "tileset",
    "solid_property",
    "kind_property",
    "tile_kinds",
    "tile_surfaces",
    "origin",
    "camera",
    "halfway",
    "layout",
    "empty",
    "tile_size",
    "size",
    "spawn",
    "spawn_objects",
    "spawn_ahead",
    "despawn_behind",
    "despawn_ahead",
    "respawn",
    "spawn_slots",
    "despawn_at",
    "despawn_after_move",
    "spawn_in_order",
    "appear_still",
    "slot_rest",
    "hit_widths",
    "draw",
    "scroll_stop",
    "loop_back",
    "counters",
    "next",
    "exit",
    "present",
];

/// Entities placed in a level: one at `x` and `y`, or one per row of a
/// table slot, with `columns` naming the column each field is read from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    /// The entity type.
    pub entity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<Fixed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<Fixed>,
    /// A slot of kind `table`, one entity per row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// Field name to the table column it's read from (`x`, `y`, or a
    /// parameter).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub columns: BTreeMap<String, String>,
    /// Per-entity parameters extensions define, such as `patrol_end`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Fixed>,
    /// It appears when the level's `draw` draws the column its x is in,
    /// rather than by `spawn_ahead`, and only if a slot is free then:
    /// Super Mario Bros.' piranha plants, which VerticalPipe makes as it
    /// draws the pipe's first column. Since core 0.4.10.
    #[serde(default, skip_serializing_if = "is_false")]
    pub drawn: bool,
    /// When it appears it is placed this many pixels past the view's last
    /// column along the level's axis, instead of at its x (or y), which is
    /// then only where the view reaches it: Super Mario Bros.' group
    /// enemies, which HandleGroupEnemies starts at the screen's right edge
    /// and 24 pixels apart. Since core 0.4.11.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_edge: Option<Fixed>,
    /// With the level's `appear_still`, it moves on the frame it appears
    /// all the same: a group's members after the first, which
    /// HandleGroupEnemies puts in slots the enemy loop runs later in the
    /// same frame. Since core 0.4.11.
    #[serde(default, skip_serializing_if = "is_false")]
    pub appear_moving: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The entity type it follows (the first one spawned).
    pub follow: String,
    /// The axes it moves along.
    pub axis: Axis,
    /// Width and height of the view in pixels.
    pub size: [u32; 2],
    /// How far the followed entity's position is kept from the view's top
    /// left corner, on each axis it moves along.
    pub lead: [Fixed; 2],
    /// Keeps the view inside the level.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clamp_to_level: bool,
    /// Can move back left (or up). Super Mario Bros.' camera can't.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub scroll_back: bool,
    /// Moves by the followed entity's move instead of keeping it `lead`
    /// from the corner; `lead` places it when a level starts. Since core
    /// 0.4.4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_move: Option<ByMove>,
    /// Since core 0.4.12: the followed entity's whole-pixel place in the
    /// view across stays from the first to the second, checked each frame
    /// after the camera's step, whether the camera moves or not. Past
    /// either, the entity is put back at it, keeping the fraction of its
    /// place, and its speed's whole steps across become 0 unless only the
    /// direction away from that edge is held (its `run` actions). Super
    /// Mario Bros.' ScrollHandler ends in ChkPOffscr and KeepOnscr: 0 and
    /// 239, the screen's right edge less 16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_in_view: Option<[i64; 2]>,
    /// Since core 0.4.17: `keep_in_view`'s first bound acts when the place
    /// reaches it, not only past it, so an entity at that edge loses its
    /// speed as one past it does. Super Mario Bros.' GetXOffscreenBits
    /// sets bit 7 from a difference of 0, while the right edge's test
    /// starts a pixel past its bound.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_in_view_touch: bool,
    /// Feature `camera.keep_in_view_turn`: an entity in the air at either
    /// bound (or past it) moving toward it turns around, its speed across
    /// reversed, instead of losing its speed there. Donkey Kong's jump
    /// ($D547) flips Mario's way ($56) at the screen's edge ($D990).
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_in_view_turn: bool,
    /// Feature `camera.keep_in_view_rows`: other bounds for `keep_in_view`
    /// while the followed entity's row is in a range: its whole y on the
    /// ground, and in the air the height it took off from when
    /// `jump.fatal_drop` keeps it (mechanic word 3), else its y. The first
    /// row that holds gives the bounds. Donkey Kong's edge test $D990 also
    /// stops a walk left on 25m's sixth girder ($59 = 6) at sprite x 104,
    /// short of Kong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keep_in_view_rows: Vec<KeepRow>,
}

/// A row of a camera's `keep_in_view_rows`: from `rows[0]` to `rows[1]`
/// (whole pixels down, both in), the bounds are `bounds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeepRow {
    pub rows: [i64; 2],
    pub bounds: [i64; 2],
}

/// A camera that moves forward (right or down) by the followed entity's
/// whole-pixel move this frame, by where the entity was in the view on the
/// frame before, in whole pixels from the view's top left corner: not at
/// all while that place is under `from`, the move less `short_by` (but at
/// least a pixel) from `from`, and the whole move from `full` on. A move
/// back or none doesn't move it. Super Mario Bros.' ScrollHandler is 80,
/// 112 and 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ByMove {
    /// The place in the view, in pixels, where it starts to move.
    pub from: Fixed,
    /// The place where it moves by the whole move.
    pub full: Fixed,
    /// How much less than the move it moves between `from` and `full`.
    pub short_by: Fixed,
    /// Since core 0.4.11: after the followed entity is pushed out of a
    /// wall (a platformer `run.walls` point or `jump.land_depth`), the
    /// camera doesn't move on this many frames less one, from the next.
    /// Super Mario Bros.' ImpedePlayerMove sets SideCollisionTimer to $10,
    /// which ScrollHandler waits for, and DecTimers counts it down at the
    /// start of each frame. The frames left are world state
    /// (`cameras.<name>.hold`).
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub push_hold: u32,
    /// Since core 0.4.12: on a frame when a state stops the followed
    /// entity's system (`stops`) and not the camera's, the camera moves as
    /// if the entity had moved by its last move again. Super Mario Bros.'
    /// ScrollHandler reads Player_X_Scroll, which only the player's
    /// movement routines set, so the screen goes on scrolling while Mario
    /// shrinks. The last move is world state (`cameras.<name>.move`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_move: bool,
    /// Since core 0.4.23: `push_hold` is a frame timer: it counts down on
    /// every frame the `counters` system runs (not frozen, not stopped by a
    /// state), whether a state stops the camera or not, and on no other.
    /// Super Mario Bros.' DecTimers counts SideCollisionTimer down at the
    /// start of each frame unless TimerControl is set, and ScrollLock, which
    /// stops ScrollHandler, doesn't stop it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hold_with_counters: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
    Both,
}

/// A state machine: the game's (`rules`) or an entity's (`states`). The
/// current state's first transition whose event fires, or whose time is up,
/// and whose `if` holds, moves it to another state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub initial: String,
    pub states: BTreeMap<String, State>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Nothing in the world moves on a frame that starts with the game, or
    /// any entity, in this state: entering it stops the world from the
    /// next frame, and leaving it starts it again from the next frame.
    /// State machines and timers still run.
    #[serde(default, skip_serializing_if = "is_false")]
    pub freeze: bool,
    /// Since core 0.4.12: a part freeze. These systems (names from
    /// `model.systems` that move things) don't run on a frame that starts
    /// in this state, from the next frame as with `freeze`, while the
    /// others do. Super Mario Bros.' injury: TimerControl stops the enemies
    /// and the frame timers while the player routine runs on its own
    /// schedule. With `freeze` it is ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stops: Vec<String>,
    /// Since core 0.4.12: of the systems this state stops (all that move,
    /// with `freeze`), these also stop for the rest of the frame it is
    /// entered in, when they haven't run yet: InjurePlayer sets
    /// TimerControl inside the enemy loop, so the enemy that hurt Mario
    /// and the ones after it don't move that frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stops_at_once: Vec<String>,
    /// Since core 0.4.12: counters whose `every` changes wait while the
    /// world starts a frame in this state: the frames still count, up to
    /// the change, and the change is made on the first frame after the
    /// state that comes round to it. Super Mario Bros.' RunGameTimer leaves
    /// while Mario is below the screen, though DecTimers goes on counting
    /// GameTimerCtrlTimer down to 0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<String>,
    /// Since core 0.4.12: entering this state keeps the frames counted in
    /// the state before, so its `after` counts from when the earlier state
    /// was entered. A run of states that time one span, such as the parts
    /// of a pause, says each part's end from the span's start.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_time: bool,
    /// Feature `state.stops_machines`: while a frame begins in this state,
    /// the entities' own machines don't run (their time, their
    /// transitions and a `step_word`), as the game machines still do.
    /// Donkey Kong's Start pause stops Mario's routine, his death spin's
    /// counter included, and only the Start check runs.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stops_machines: bool,
    /// Feature `state.frees_at_once`, a game machine's state: systems that
    /// move things which run for the rest of the frame on which the state
    /// is left, from the system after the one that left it, though the
    /// frame began frozen in it. Donkey Kong's unpause clears $0516 in the
    /// Start check and then counts the frame timers on that frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frees_at_once: Vec<String>,
    /// Since core 0.4.12, an entity's state only: while a camera `by_move`
    /// follows the entity in this state, it keeps the entity's place in
    /// its view only on frames whose number is a multiple of this, the
    /// frames the entity is drawn. Super Mario Bros.' PlayerGfxHandler
    /// leaves on odd frames while InjuryTimer is set (the blink), before
    /// RenderPlayerSub sets Player_Pos_ForScroll, which ScrollHandler reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_every: Option<u32>,
    /// Since core 0.4.13, an entity's state only: what entering it does to
    /// the entity's standing. `leave` takes it off the ground, as Super
    /// Mario Bros.' ForceInjury sets Player_State to 1 though Mario stays
    /// where he is; `land` puts it back on the ground if there is ground
    /// under it, as PlayerBGCollision does on the first frame
    /// PlayerCtrlRoutine runs again, though PlayerMovementSubs doesn't move
    /// him while he shrinks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground: Option<GroundOnEnter>,
    /// Since core 0.4.15, an entity's state only: entering it sets the
    /// entity's speed across and up and down. Super Mario Bros.'
    /// KillPlayer stops Mario across and sets his vertical speed to -4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter_speed: Option<[Fixed; 2]>,
    /// Since core 0.4.15, with `enter_speed`: the speed up and down keeps
    /// the fraction below a pixel it had, and only its whole pixels are
    /// set. KillPlayer stores Player_Y_Speed and leaves Player_Y_MoveForce
    /// as it was, so Mario's speed is -4 plus the fraction of his last.
    #[serde(default, skip_serializing_if = "is_false")]
    pub enter_keeps_fraction: bool,
    /// Feature `state.step_word`, an entity's state only: on each frame
    /// the entity is in it, frozen or not, its mechanic word 0 (the step
    /// counter of `run.step_cycle`) goes up by one, back to 0 at this, before
    /// its transitions are tested. Donkey Kong's death spin calls $D9E6
    /// every frame, so $88 goes on counting while the world stands still.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_word: Option<u32>,
    /// An entity's state only: the entity's fields while in this state,
    /// written as the entity's own are (`hitbox = { ... }`,
    /// `walk = { speed = 2 }`). Tables merge into the entity's, key by key,
    /// and other values replace its.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub set: BTreeMap<String, toml::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transition: Vec<Transition>,
}

/// What entering a state does to an entity's standing (core 0.4.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GroundOnEnter {
    Leave,
    Land,
}

/// Exactly one of `on` and `after`, or, in an entity's machine, `below`
/// (with or without `after`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub to: String,
    /// An event (docs/model.md lists them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// Frames after entering the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<u32>,
    /// Since core 0.4.12, with `after`: once the time is up, it is taken
    /// only on a frame whose number (the frame count from power-on) is a
    /// multiple of this. Super Mario Bros.' HandleChangeSize steps the
    /// shrink on frames whose FrameCounter is a multiple of 4, so its end
    /// depends on that counter's phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<u32>,
    /// Feature `transition.align_to`, with `after`: once the time is up, it is taken
    /// only on a frame whose counter's first `every` clock has just come
    /// round (its count is 0), so the state lasts to a beat that clock
    /// keeps. Super Mario Bros.' lives screen waits for ScreenTimer, an
    /// interval timer DecTimers counts down once every 21 frames, when
    /// IntervalTimerControl runs out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align_to: Option<String>,
    /// Feature `transition.below`, in an entity's machine: taken once the entity's
    /// y is at or past this (and its `after` is up, with one). Super Mario
    /// Bros.' PlayerHole ends a death once Player_Y_HighPos reaches 4,
    /// however high Mario was hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<Fixed>,
    /// Feature `transition.word`, in an entity's machine with `after`:
    /// taken only on a frame on which one of the entity's mechanic words
    /// is in this range. Donkey Kong's death spin ends on the frame $88
    /// comes round to 4, 120 frames or more after it starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<WordTest>,
    /// Feature `transition.pressed`, without `on`: taken on a frame on
    /// which this input action is newly pressed (held, and not on the
    /// frame before), and with `after` only once that many frames have
    /// passed in the state. Donkey Kong's Start pauses the game, and
    /// unpauses it once the pause's 64-frame timer ($0517) has run out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressed: Option<String>,
    /// Feature `transition.held`, without `on` or `pressed`: taken on every
    /// frame on which this input action is held, and with `after` only
    /// once that many frames have passed in the state. Donkey Kong's title
    /// sets its idle timer to 64 on each frame Select is held ($C95D).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    /// A condition that must also hold.
    #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
    pub guard: Option<Guard>,
    /// Actions to take on the way (docs/model.md lists them).
    #[serde(default, rename = "do", skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
}

/// A comparison of a counter with a number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Guard {
    pub counter: String,
    pub cmp: Cmp,
    pub value: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Cmp {
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = "==")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = ">")]
    Gt,
}

impl Cmp {
    pub fn holds(self, a: i64, b: i64) -> bool {
        match self {
            Self::Lt => a < b,
            Self::Le => a <= b,
            Self::Eq => a == b,
            Self::Ne => a != b,
            Self::Ge => a >= b,
            Self::Gt => a > b,
        }
    }
}

/// Feature `cost.frame`: a frame's CPU cycles in the original, `base`
/// plus each term whose tests hold, times the bodies of its `per` kind
/// that pass its `per_tests` when it has one. The simulation sums it at the end of each
/// frame, before the `hook_last` hooks, adds what the hooks added
/// (`ExtCx::add_cycles`), and a hook reads the sum
/// (`ExtCx::frame_cycles`): Donkey Kong's idle loop steps its random
/// register in the cycles the frame's work leaves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Cost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Cycles every frame.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub base: i64,
    /// The terms, each a routine's part or a branch of one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terms: Vec<CostTerm>,
}

/// One term of a [`Cost`]: `cycles` on a frame every test holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostTerm {
    /// The routine or branch it stands for, by the code's address, and
    /// why: informative.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub routine: String,
    pub cycles: i64,
    /// Each must hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<CostTest>,
    /// An entity kind: the term counts once for each active body of it
    /// that passes `per_tests`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per: Option<String>,
    /// Tests on each body of `per`: its `ext` words (`word`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub per_tests: Vec<CostTest>,
}

/// A test of a [`CostTerm`]: one thing it reads, in a range.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostTest {
    /// A counter, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<String>,
    /// With `counter`: read as the frame started (`start`, the default) or
    /// as it ends (`end`), for a routine that runs before or after the
    /// frame's other work changes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// An input action held this frame (1) or not (0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    /// An input action held the frame before (1) or not (0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_before: Option<String>,
    /// Feature `cost.frame_pad`: an input action's bits on controller 1 as
    /// pressed this frame (1) or not (0), before `keep_direction`, a turn's
    /// controller or a hook's held actions: the pad a controller routine
    /// reads, which runs on every frame of an attract demo too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<String>,
    /// A game machine's state (`machine:state`) as the frame started: 1 in
    /// it, 0 not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// In `per_tests`: the body's `ext` word, 0 to 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<u8>,
    /// The value is at least this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<i64>,
    /// The value is below this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<i64>,
}

/// A number the game keeps: score, coins, lives, time. It starts at
/// `start` when the game does, and a level's `counters` set it when the
/// level starts; restarting a level leaves it alone otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Counter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: i64,
    /// What changes it, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub change: Vec<Change>,
    /// It never goes below this. Reaching it fires `<counter>.reached_min`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// It never goes above this: a change past it stops at it, or with
    /// `wrap`, goes on from `min` (or 0) and fires `<counter>.wrapped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub wrap: bool,
    /// Its `every` changes go on counting while the world is frozen
    /// (Super Mario Bros.' color rotation, which runs while Mario dies).
    /// Since core 0.4.8.
    #[serde(default, skip_serializing_if = "is_false")]
    pub runs_when_frozen: bool,
    /// Since core 0.4.20: the counter is a shift register: it steps every frame the `counters`
    /// system runs (Super Mario Bros.' PseudoRandomBitReg, which the NMI
    /// rotates every frame).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<CounterShift>,
    /// Feature `counter.restarts_clock`: a level start that sets the counter (the level's
    /// `counters`) also starts its `every` clocks over, as Super Mario
    /// Bros.' InitializeArea clears GameTimerCtrlTimer with the other frame
    /// timers when an area starts.
    #[serde(default, skip_serializing_if = "is_false")]
    pub restarts_clock: bool,
    /// Feature `counter.high_of`: the counter keeps the highest value the
    /// named counter has had, raised as that one changes, as a top score
    /// does (Donkey Kong's $21-$23, which $D03E raises from the score after
    /// every award).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_of: Option<String>,
}

/// A linear feedback shift register in a counter's low `bits` bits: each
/// step moves them one bit down, and the top bit becomes the exclusive-or of
/// the old bits at `taps`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CounterShift {
    pub bits: u32,
    pub taps: Vec<u32>,
}

/// One way a counter changes: when an event fires, or every so many
/// frames. Exactly one of `on` and `every`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Change {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// Frames between changes, counted in the `counters` system while
    /// nothing freezes the world (or always, with the counter's
    /// `runs_when_frozen`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u32>,
    /// How much it changes by.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub by: i64,
    /// Since core 0.4.13, with `on`: only an event that concerns an entity
    /// of one of these kinds changes it. Default any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub of: Vec<String>,
    /// Since core 0.4.13, with `on`: the change is made this many frames
    /// after the event, counted every frame, frozen or not, from the next
    /// one. Super Mario Bros. adds an enemy's points when its floatey
    /// number's timer, set to $30 by SetupFloateyNumber, reads $2b in
    /// FloateyNumbersRoutine, 5 frames after the kill. Default 0, at once.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub after: u32,
    /// Feature `change.if`, with `on`: the event changes the counter only
    /// while this counter condition holds, tested as the event fires.
    /// Donkey Kong's points routine ($D008) adds nothing while the attract
    /// demo plays ($58), though the points sprite still shows.
    #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
    pub guard: Option<Guard>,
}

/// Where a fact comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: Source,
    /// How sure, in percent.
    pub confidence: u8,
    /// What supports it: files with line numbers, baselines, recordings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Measured by the pipeline from recordings.
    Observed,
    /// Proposed by an agent.
    Inferred,
    /// Said by a person, in words or in the game's own source.
    Stated,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Inferred => "inferred",
            Self::Stated => "stated",
        }
    }
}

/// One piece of support. `at` is `path:line` for a file in the repo, which
/// `gp model check` checks, or any other reference (a recording ID).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ref {
    pub at: String,
    /// A pinned download the file is in rather than the repo, as
    /// `<name>@<revision>`, such as `smbdis@7e8e9777`: `at` is then a path
    /// within it.
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    /// Text the line must contain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_true(b: &bool) -> bool {
    *b
}
fn yes() -> bool {
    true
}
fn is_zero(n: &i64) -> bool {
    *n == 0
}
fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}
fn is_one(n: &i64) -> bool {
    *n == 1
}
fn one() -> i64 {
    1
}

/// Splits a table into its core fields and the rest, which go to the
/// extensions.
fn split(table: toml::Table, core: &[&str]) -> (toml::Table, toml::Table) {
    table
        .into_iter()
        .partition(|(k, _)| core.contains(&k.as_str()))
}

fn merge<C: Serialize, E: Serialize>(core: &C, ext: &E) -> Result<toml::Table, String> {
    let mut t = toml::Table::try_from(core).map_err(|e| e.to_string())?;
    let ext = toml::Table::try_from(ext).map_err(|e| e.to_string())?;
    for (k, v) in ext {
        t.insert(k, v);
    }
    Ok(t)
}

macro_rules! extended {
    ($ty:ident, $core:ident, $fields:ident, $ext:path, $td:path, $td_names:expr, $what:literal) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                use serde::de::Error;
                let (core, ext) = split(toml::Table::deserialize(d)?, $fields);
                // The top-down extension's keys go to it, the rest to the
                // platformer's; their names never overlap.
                let (td, ext) = split(ext, $td_names);
                let core = toml::Value::Table(core).try_into::<$core>().map_err(|e| D::Error::custom(e.message().to_owned()))?;
                let platformer = toml::Value::Table(ext).try_into::<$ext>().map_err(|e| {
                    D::Error::custom(format!(
                        "{} (a key that isn't a core {} field must be a component of a declared extension)",
                        e.message(),
                        $what
                    ))
                })?;
                let topdown = toml::Value::Table(td).try_into::<$td>().map_err(|e| {
                    D::Error::custom(format!("{} (in the top-down extension's {} fields)", e.message(), $what))
                })?;
                Ok(Self { core, platformer, topdown })
            }
        }

        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut t = merge(&self.core, &self.platformer).map_err(serde::ser::Error::custom)?;
                let td = toml::Table::try_from(&self.topdown).map_err(serde::ser::Error::custom)?;
                t.extend(td);
                t.serialize(s)
            }
        }

        impl JsonSchema for $ty {
            fn schema_name() -> Cow<'static, str> {
                stringify!($ty).into()
            }
            fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                let mut schema = $core::json_schema(generator);
                schema.insert(
                    "description".into(),
                    concat!(
                        "Core ",
                        $what,
                        " fields. Other keys are components of the genre extensions the model declares, each checked against its extension's schema."
                    )
                    .into(),
                );
                schema.insert("additionalProperties".into(), true.into());
                schema
            }
        }
    };
}

extended!(
    Entity,
    EntityCore,
    ENTITY_CORE_FIELDS,
    platformer::EntityComponents,
    topdown::EntityComponents,
    topdown::EXTENSION.entity_components,
    "entity"
);
extended!(
    Level,
    LevelCore,
    LEVEL_CORE_FIELDS,
    platformer::LevelFields,
    topdown::LevelFields,
    topdown::EXTENSION.level_fields,
    "level"
);
