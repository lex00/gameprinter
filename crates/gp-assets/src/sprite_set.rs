//! The `sprite-set` format (docs/formats/sprite-set.md): sprite frames as
//! layouts of tiles, each tile placed in pixels from the entity's
//! position with its own flips, palette and tiles slot, plus the sprite
//! palettes each level showed. `gp sprites capture` writes it into a
//! lifted directory from the game's sprite tables and its recordings, so
//! it holds no ROM bytes of its own: the map entry names no span, `gp
//! assets lift` writes nothing for it (and so never replaces a capture),
//! and `gp assets pack` encodes nothing.

use serde::{Deserialize, Serialize};

use crate::format::{self, Module, ReadFile, Rom};
use crate::map::{AssetMap, Span};
use crate::{Error, File};

/// The file's schema version.
pub const SCHEMA: u32 = 1;

/// The map entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteSetEntry {
    pub name: String,
    /// What writes the file, for people: `gp sprites capture`.
    pub written_by: Option<String>,
}

impl Module for SpriteSetEntry {
    fn name(&self) -> &str {
        &self.name
    }

    fn format(&self) -> &'static str {
        "sprite-set"
    }

    fn validate(&self) -> Result<(), String> {
        format::one_line(&format!("{}: name", self.name), &self.name)
    }

    fn spans(&self) -> Vec<Span> {
        Vec::new()
    }

    /// Nothing is lifted: capture writes the file.
    fn files(&self) -> Vec<String> {
        Vec::new()
    }

    fn lift(&self, _source: &str, _map: &AssetMap, _rom: &Rom) -> Result<Vec<File>, String> {
        Ok(Vec::new())
    }

    fn encode(&self, _map: &AssetMap, _read: ReadFile) -> Result<Vec<Vec<u8>>, Error> {
        Ok(Vec::new())
    }
}

/// One tile of a frame.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    /// The tile's number in its tiles slot.
    pub tile: u32,
    /// The model's tiles slot; without it, the sprite's `slot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    /// Its top left in pixels from the entity's position.
    pub x: i32,
    pub y: i32,
    /// A palette in the model's numbering (sprite palettes are 4 to 7).
    pub palette: u8,
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_x: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_y: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A palette a frame brings, replacing the level's: a state's own colors.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    /// A palette in the model's numbering.
    pub palette: u8,
    /// Four NES color numbers; the first is never drawn.
    pub colors: [u8; 4],
}

/// One frame: a layout of tiles. No pieces is a frame that shows nothing
/// (a blink's off frame).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetFrame {
    /// The entity and state it was found for, for people.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entity: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    /// Where it came from: `table <name> <index>`, `observed`, or both.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from: Vec<String>,
    #[serde(default)]
    pub pieces: Vec<Piece>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<Colors>,
}

impl SetFrame {
    /// Left, top, right and bottom (exclusive) of its pieces, in pixels
    /// from the position; all 0 with no pieces.
    pub fn bounds(&self) -> [i32; 4] {
        if self.pieces.is_empty() {
            return [0; 4];
        }
        let mut b = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        for p in &self.pieces {
            b[0] = b[0].min(p.x);
            b[1] = b[1].min(p.y);
            b[2] = b[2].max(p.x + 8);
            b[3] = b[3].max(p.y + 8);
        }
        b
    }
}

/// The sprite palettes shown in some levels: 16 NES color numbers, four
/// palettes of four.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteSet {
    /// The model's levels it was seen in; empty is every other level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<String>,
    pub colors: Vec<u8>,
}

/// The lifted file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteSet {
    pub schema: u32,
    pub asset: String,
    #[serde(default, rename = "set", skip_serializing_if = "Vec::is_empty")]
    pub sets: Vec<PaletteSet>,
    #[serde(default, rename = "frame")]
    pub frames: Vec<SetFrame>,
}

impl SpriteSet {
    pub fn parse(text: &str) -> Result<Self, String> {
        let s: SpriteSet = toml::from_str(text).map_err(|e| e.to_string())?;
        if s.schema != SCHEMA {
            return Err(format!(
                "sprite-set schema {} isn't supported; this build reads {SCHEMA}",
                s.schema
            ));
        }
        for (i, set) in s.sets.iter().enumerate() {
            if set.colors.len() != 16 {
                return Err(format!("set[{i}] has {} colors, not 16", set.colors.len()));
            }
        }
        Ok(s)
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from(
            "# Sprite frames and palettes (docs/formats/sprite-set.md), written by gp sprites capture.\n",
        );
        out.push_str(&toml::to_string(self).expect("a sprite set serializes"));
        out
    }

    /// The palette set for a level: the first that lists it, else the
    /// first that lists none.
    pub fn set_for(&self, level: &str) -> Option<usize> {
        self.sets
            .iter()
            .position(|s| s.levels.iter().any(|l| l == level))
            .or_else(|| self.sets.iter().position(|s| s.levels.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_bounds() {
        let s = SpriteSet {
            schema: SCHEMA,
            asset: "made_up".into(),
            sets: vec![PaletteSet {
                levels: vec!["a".into()],
                colors: (0..16).collect(),
            }],
            frames: vec![
                SetFrame {
                    entity: "blob".into(),
                    pieces: vec![
                        Piece {
                            tile: 1,
                            slot: None,
                            x: -8,
                            y: -16,
                            palette: 4,
                            flip_x: true,
                            flip_y: false,
                        },
                        Piece {
                            tile: 2,
                            slot: Some("t".into()),
                            x: 0,
                            y: -8,
                            palette: 5,
                            flip_x: false,
                            flip_y: false,
                        },
                    ],
                    ..Default::default()
                },
                SetFrame::default(),
            ],
        };
        let back = SpriteSet::parse(&s.to_text()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.frames[0].bounds(), [-8, -16, 8, 0]);
        assert_eq!(back.frames[1].bounds(), [0; 4]);
        assert_eq!(back.set_for("a"), Some(0));
        assert_eq!(back.set_for("b"), None);
    }
}
