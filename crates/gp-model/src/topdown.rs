//! The top-down genre extension, version [`VERSION`]: eight-way movement
//! with walls on both axes, player classes and the class select screen.
//! docs/research/topdown-extension.md says what the genre needs and what
//! later versions add (generators, shots, keys and doors, magic).
//!
//! A model uses it by declaring `extensions = { topdown = "0.1" }`. Its
//! entity components and level fields sit beside the core's in an entity
//! or level table, as the platformer's do; the two extensions' names never
//! overlap, which a test checks.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fixed::Fixed;
use crate::platformer::Extension;

pub const NAME: &str = "topdown";
/// The extension's version, `major.minor`: it bumps only for a breaking
/// change ([`crate::features`]).
pub const VERSION: &str = "0.1";

/// What the extension adds to the core.
pub const EXTENSION: Extension = Extension {
    name: NAME,
    version: VERSION,
    core: "0.4",
    systems: SYSTEMS,
    entity_components: &["move8", "class"],
    level_fields: &["class_select"],
    spawn_params: &[],
    events: &[],
    actions: &[],
};

/// The top-down systems:
///
/// - `topdown.player`: each player with `move8` moves across, then down,
///   by the actions held, stopping at solid tiles on each axis.
pub const SYSTEMS: &[&str] = &["topdown.player"];

/// The components a top-down entity can have. Each is optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer top-down extension 0.1: entity components")]
pub struct EntityComponents {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move8: Option<Move8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<Class>,
}

impl EntityComponents {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Moving in eight directions with no gravity: each frame the entity
/// moves `speed` pixels across while `left` or `right` is held, and
/// `speed` pixels down or up while `down` or `up` is held, both on a
/// diagonal. Solid tiles stop each axis on its own, so it slides along a
/// wall it meets at an angle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Move8 {
    /// Pixels a frame on each axis.
    pub speed: Fixed,
    /// The input actions it moves by. Default `left`, `right`, `up` and
    /// `down`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Move8Actions>,
}

/// The action names [`Move8`] reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Move8Actions {
    pub left: String,
    pub right: String,
    pub up: String,
    pub down: String,
}

impl Move8 {
    /// The action names in the order left, right, up, down.
    pub fn action_names(&self) -> [&str; 4] {
        match &self.actions {
            Some(a) => [&a.left, &a.right, &a.up, &a.down],
            None => ["left", "right", "up", "down"],
        }
    }
}

/// A player class, chosen before play: its name as the game shows it, and
/// its stats. In 0.1 the stats are stated facts only; the simulation reads
/// the class's own components (its `move8.speed` is its speed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Class {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armor: Option<Fixed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shot_power: Option<Fixed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magic: Option<Fixed>,
}

/// The fields the extension adds to levels.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer top-down extension 0.1: level fields")]
pub struct LevelFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_select: Option<ClassSelect>,
}

impl LevelFields {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The class select screen before play: the classes in menu order (each a
/// player entity with a `class`), the action that moves the cursor to the
/// next one, and the action that picks the one under it. A rebuild starts
/// as one class by pressing `next` that class's index times, then `pick`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClassSelect {
    pub classes: Vec<String>,
    pub next: String,
    pub pick: String,
}

/// The extension's whole schema, for `gp model schema topdown`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer top-down extension 0.1")]
pub struct Schema {
    /// Components it adds to entities.
    pub entity: EntityComponents,
    /// Fields it adds to levels.
    pub level: LevelFields,
}
