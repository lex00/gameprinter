//! `gp theme kit` and the tile-based paper painter (docs/theme-kit.md).
//!
//! A kit is what a theme maker needs to restyle a game, from its model
//! alone: each art slot's role, size, frames, timing, colors and anchor
//! with a plain description, and each sound role's cue, length and whether
//! it loops. The shareable part (`templates/GAME/themes/kit/kit.toml`)
//! holds no tile numbers, layouts or pictures of the game. With the ROM,
//! the kit also writes a local part under `$GP_HOME/work/theme-kits/GAME/`:
//! which tiles each frame is cut into and a reference sheet of sizes and
//! poses. `gp theme fit` and `gp theme paint` cut art into tiles with it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::theme::RunRecorder;
use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use gp_model::Model;
use gp_model::model::{AssetKind, Sprite};
use gp_theme::color::Rgb;
use gp_theme::pix::Rgba;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Outcome;

/// The kit's format version.
pub const KIT_FORMAT: &str = "0.1.0";
/// The shareable kit's file.
pub const KIT_FILE: &str = "kit.toml";

/// The shareable kit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kit {
    pub format: String,
    /// The template's id.
    pub game: String,
    /// The model's name.
    pub model: String,
    /// The screen, in pixels, and a pixel's width over its height.
    pub screen: [u32; 2],
    pub pixel_aspect: String,
    /// The fingerprint of the model's art slots and sound roles
    /// ([`slot_lines`], `gp_package::index::slots_digest`): their ids,
    /// kinds, sizes, frame counts, color limits and whether each sound
    /// loops, and none of the kit's words. A theme's index line records it
    /// for each model the theme was checked on (docs/theme-publishing.md).
    #[serde(default)]
    pub slots: String,
    /// Art slots, in the order `gp theme fit` fills tiles: a tile two slots
    /// share takes the first one's art.
    #[serde(default)]
    pub art: Vec<ArtSlot>,
    #[serde(default)]
    pub sound: Vec<SoundSlot>,
    /// The tolerances `gp theme check` holds the art's geometry to
    /// (docs/theme-kit.md, "Gameplay geometry").
    #[serde(default)]
    pub tolerance: crate::theme_geometry::Tolerance,
    /// Each level's floors and climbable zones, as rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<crate::theme_geometry::Surface>,
}

/// One art slot of a kit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtSlot {
    /// `entity.<name>`, `effect.<n>`, `screen-sprite.<n>.<k>`,
    /// `picture.<slot>.<entry>` or `font`.
    pub id: String,
    /// `sprite` (frames cut into tiles), `picture` (a whole layout drawn
    /// as one picture) or `font` (one glyph a frame).
    pub kind: String,
    /// The role its pixels carry.
    pub role: String,
    /// Width and height in pixels of one frame.
    pub size: [u32; 2],
    pub frames: u32,
    /// Each frame's name, in order: `base`, then `clip<i>.<j>`, or the
    /// characters of a font.
    #[serde(default)]
    pub frame_names: Vec<String>,
    /// How the frames change, in plain words.
    pub timing: String,
    /// The color limit.
    pub colors: String,
    /// Where the picture sits on its body.
    pub anchor: String,
    /// What it is, from the model's role and description, without any
    /// character's name, or from the game's words file. A prompt starts
    /// from it.
    pub description: String,
    /// A short neutral name, printed on a sheet's cells (`gp theme
    /// sheet`): the words file's, else the role's.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// One plain phrase a frame, in frame order, from the game's words
    /// file (docs/theme-kit.md, "Prompt words"). Empty without one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub poses: Vec<String>,
    /// A sprite's gameplay geometry: its boxes, danger frames, feet and
    /// readability needs (docs/theme-kit.md, "Gameplay geometry").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<crate::theme_geometry::Geometry>,
}

/// One sound role of a kit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundSlot {
    pub role: String,
    /// `music` or `sfx`.
    pub kind: String,
    /// The cue, in plain words: the counter and the test.
    pub cue: String,
    /// Its usual length in frames, when the model gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u32>,
    pub loops: bool,
    pub description: String,
}

/// One tile placed in a frame: the tile, its column and row in tiles,
/// its flips, and the four console colors (NES numbers) it's drawn with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Place {
    pub tile: u32,
    pub col: u32,
    pub row: u32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub colors: [u8; 4],
}

/// A frame cut into tiles, in the local part of a kit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cut {
    /// The tiles slot.
    pub slot: String,
    pub places: Vec<Place>,
}

/// The local part of a kit: how each sprite slot's frames are cut into the
/// game's tiles, and each picture slot's cells. It is derived from the
/// lifted assets and stays under `$GP_HOME/work/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Local {
    pub game: String,
    /// Tiles slots and their size in tiles (width, height).
    pub banks: BTreeMap<String, [u32; 2]>,
    /// Tiles to leave blank in each bank (a font's space).
    pub blank: BTreeMap<String, Vec<u32>>,
    /// Frames by art slot id.
    pub cuts: BTreeMap<String, Vec<Cut>>,
    /// Pictures: by art slot id, the cells' text where the game's font
    /// knows the tile (`?` where it doesn't, ` ` where blank), one string
    /// a row, and each cell's palette (`0` to `3`), one string a row.
    pub cells: BTreeMap<String, (Vec<String>, Vec<String>)>,
    /// Pictures whose layout couldn't be read as update strings.
    #[serde(default)]
    pub skipped: Vec<String>,
    /// Each sprite frame's visible body in the original, `[x0, y0, x1, y1]`
    /// (the right and bottom past it), by art slot id, in frame order.
    #[serde(default)]
    pub bodies: BTreeMap<String, Vec<Option<crate::theme_geometry::Body>>>,
    /// Each stage picture's floor lines and ladders, from the lifted
    /// level, by art slot id.
    #[serde(default)]
    pub stages: BTreeMap<String, crate::theme_geometry::Stage>,
    /// Levels built from objects (`level-objects` slots): each metatile's
    /// four tiles in the level tiles' bank, by its item role, in the
    /// background palette of its group (metatiles slot `n`, palette `n`).
    #[serde(default)]
    pub objects: Vec<(String, Cut)>,
    /// The font's letters and signs other than digits and the space, each
    /// in the HUD's tiles.
    #[serde(default)]
    pub letters: Vec<(char, Cut)>,
    /// A level drawn from single tiles (a name table): each kind but
    /// `empty` that `present.cells` draws a changed cell as (100m's ending
    /// lays a girder, `top`), its tile in the level's bank in background
    /// palette 0, as the presentation draws it.
    #[serde(default)]
    pub kind_cells: Vec<(String, Cut)>,
}

/// The words a role's description starts with.
fn role_words(role: &str) -> &'static str {
    match role {
        "player" => "the hero, the one the player moves",
        "enemy" => "an enemy that harms the hero",
        "boss" => "a big enemy",
        "projectile" => "something thrown or fired",
        "npc" => "a character the hero doesn't fight",
        "collectible" => "something the hero picks up",
        "power-up" => "a power-up the hero picks up",
        "hazard" => "a hazard that harms the hero on touch",
        "interactive" => "a thing the hero uses",
        "goal" => "the goal",
        _ => "a thing in the game",
    }
}

/// A short neutral label for a role, for a slot the words file doesn't
/// name.
pub fn role_label(role: &str, big: bool) -> &'static str {
    match role {
        "player" => "hero",
        "enemy" | "boss" if big => "big foe",
        "enemy" => "foe",
        "boss" => "big foe",
        "projectile" => "shot",
        "npc" => "bystander",
        "collectible" => "pickup",
        "power-up" => "power-up",
        "hazard" => "hazard",
        "interactive" => "device",
        "goal" => "goal",
        "effect" => "effect",
        "menu" => "cursor",
        "hud" => "score display",
        "background" => "stage",
        _ => "thing",
    }
}

/// Words that may start a sentence; any other capitalized word is taken
/// for a name and left out of a kit.
const COMMON: &[&str] = &[
    "A", "An", "The", "It", "Its", "Each", "Every", "When", "While", "One", "Two", "This", "That",
    "These", "Those", "On", "In", "At", "If", "With", "After", "Before", "Once", "Until", "For",
    "From", "His", "Her", "Their", "To", "Not", "No", "And", "But", "Or", "As", "By", "Of", "Up",
    "Down", "Left", "Right", "Start", "Select", "Its", "What", "Where", "Who",
];

/// A description without names, addresses or notes in parentheses: the
/// capitalized words that aren't common sentence starters are taken for
/// names and left out, and added to `names`.
pub fn scrub(text: &str, names: &mut BTreeSet<String>) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let mut words = Vec::new();
    for w in out.split_whitespace() {
        let bare: String = w
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '\'')
            .collect();
        let stem = bare.trim_end_matches("'s").to_owned();
        if w.contains('$') || stem.chars().any(|c| c.is_ascii_digit()) && stem.len() > 2 {
            continue;
        }
        if stem.chars().next().is_some_and(char::is_uppercase) && !COMMON.contains(&stem.as_str()) {
            names.insert(stem.to_lowercase());
            continue;
        }
        let lower = stem.to_lowercase();
        if names.contains(&lower) || names.contains(lower.trim_end_matches('s')) {
            continue;
        }
        words.push(w.to_owned());
    }
    let s = words.join(" ");
    let s = s.trim_matches(|c: char| c == ',' || c == ';' || c == ':' || c.is_whitespace());
    let mut s = s.replace(" ,", ",").replace(",,", ",");
    if let Some(f) = s.get(..1) {
        s = f.to_lowercase() + &s[1..];
    }
    s
}

/// An identifier with any name in it hidden: `pauline_shown` -> `x_shown`.
fn redact(id: &str, names: &BTreeSet<String>) -> String {
    id.split('.')
        .map(|part| {
            part.split('_')
                .map(|w| {
                    if names.contains(&w.to_lowercase()) {
                        "x"
                    } else {
                        w
                    }
                })
                .collect::<Vec<_>>()
                .join("_")
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Words of an entity's name a kit may keep: plain words for things. Any
/// other word of an entity's name is taken for a name (a species or a
/// character a game made up) and left out.
const GENERIC: &[&str] = &[
    "player", "enemy", "barrel", "fire", "hammer", "oil", "drum", "coin", "block", "brick",
    "platform", "spring", "flag", "pipe", "vine", "star", "flower", "axe", "bridge", "fireball",
    "bullet", "points", "door", "key", "lift", "spike", "spikes", "cannon", "ball", "rope", "bird",
    "fish", "bat", "plant", "shell", "small", "big", "red", "green", "grey", "gray", "blue",
    "left", "right", "up", "down", "1", "2", "3", "a", "b", "goal", "pole", "cloud", "item",
    "power", "life", "bonus", "timer", "score", "hud", "camera", "box", "crate", "sound", "demo",
    "idle", "spawner", "turtle", "beetle", "bug", "frog", "ghost", "heart", "fallen",
];

/// The names a kit leaves out: the model title's words and every
/// capitalized word of the entities' descriptions that isn't a common
/// sentence starter.
pub fn names_of(m: &Model) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = m
        .model
        .title
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    for (n, e) in &m.entities {
        if let Some(d) = &e.core.description {
            scrub(d, &mut names);
        }
        for w in n.split('_') {
            if !GENERIC.contains(&w) {
                names.insert(w.to_owned());
            }
        }
    }
    names
}

/// An entity's art slot id: `entity.<name>`, or by its role where the
/// name is a character's (`entity.npc.pauline` -> `entity.npc_1`).
pub fn entity_id(name: &str, role: &str, names: &BTreeSet<String>) -> String {
    if redact(name, names) == name && name.split('_').all(|w| GENERIC.contains(&w)) {
        format!("entity.{name}")
    } else {
        let mut h = 0u32;
        for b in name.bytes() {
            h = h.wrapping_mul(31).wrapping_add(u32::from(b));
        }
        format!("entity.{}_{:03}", role.replace('-', "_"), h % 1000)
    }
}

/// The template and model a kit command names: a game's folder, or a
/// game or template id in the games checkout (game-printer's was a
/// template id under ./templates).
pub fn template_dir(arg: &Path) -> Result<PathBuf> {
    if arg.join("template.toml").is_file() || arg.join(gp_model::MODEL_FILE).is_file() {
        return Ok(arg.to_owned());
    }
    if let Some(d) = arg.to_str().and_then(crate::theme_template::find_game) {
        return Ok(d);
    }
    bail!(
        "{} isn't a game's folder or a game in the games checkout (set GP_GAMES_DIR)",
        arg.display()
    )
}

/// The template's id and its accepted ROM hashes, from template.toml.
pub(crate) fn manifest(dir: &Path) -> (String, Vec<String>, Option<PathBuf>) {
    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Ok(text) = fs::read_to_string(dir.join("template.toml")) else {
        return (id, Vec::new(), None);
    };
    let Ok(v) = text.parse::<toml::Table>() else {
        return (id, Vec::new(), None);
    };
    let id = v
        .get("template")
        .and_then(|t| t.get("id"))
        .and_then(|x| x.as_str())
        .map_or(id, str::to_owned);
    let accepts = v
        .get("source")
        .and_then(|s| s.get("accepts"))
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let map = v
        .get("recipe")
        .and_then(|r| r.get("asset_map"))
        .and_then(|x| x.as_str())
        .map(|m| dir.join(m));
    (id, accepts, map)
}

fn px(f: gp_model::Fixed) -> u32 {
    (f.raw().max(0) >> 16) as u32
}

/// A sprite's tiles slot and its tile size in the model, without the
/// lifted rows (a frame from a table counts its cells).
fn sprite_size(s: &Sprite, e_size: [u32; 2]) -> [u32; 2] {
    if !s.cells.is_empty() && s.cells.iter().all(|c| c.at.is_some()) {
        let w = s
            .cells
            .iter()
            .filter_map(|c| c.at)
            .map(|a| a[0] + 1)
            .max()
            .unwrap_or(1);
        let h = s
            .cells
            .iter()
            .filter_map(|c| c.at)
            .map(|a| a[1] + 1)
            .max()
            .unwrap_or(1);
        return [w * 8, h * 8];
    }
    if s.tiles.is_empty() {
        return e_size;
    }
    let w = s.width.max(1);
    [w * 8, (s.tiles.len() as u32).div_ceil(w) * 8]
}

/// The model's kit, from the model alone. Its descriptions keep the
/// model's words (the paper painter reads them); [`kit_for`] puts the
/// game's prompt words in.
pub fn kit_of(m: &Model, game: &str) -> Kit {
    kit_named(m, game, &BTreeSet::new(), None)
}

/// The kit as `gp theme kit` writes it: [`kit_of`] with the template's
/// words file applied (docs/theme-kit.md, "Prompt words"), the words it
/// denies hidden in counter names too. Returns what doesn't fit the words.
pub fn kit_for(m: &Model, game: &str, template: &Path) -> Result<(Kit, Vec<String>)> {
    let geo = crate::theme_geometry::load(template)?;
    let Some(w) = crate::theme_words::load(template)? else {
        return Ok((
            kit_named(m, game, &BTreeSet::new(), geo.as_ref()),
            Vec::new(),
        ));
    };
    let extra: BTreeSet<String> = w
        .deny
        .iter()
        .flat_map(|d| {
            d.split_whitespace()
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut kit = kit_named(m, game, &extra, geo.as_ref());
    let mut problems = crate::theme_words::apply(&mut kit, &w);
    problems.extend(crate::theme_words::check(&kit, &w.deny));
    Ok((kit, problems))
}

fn kit_named(
    m: &Model,
    game: &str,
    extra: &BTreeSet<String>,
    geo: Option<&crate::theme_geometry::GeoFile>,
) -> Kit {
    // Slot ids come from the model's names alone, so a words file never
    // moves them; its denied words are hidden in counter names too.
    let id_names = names_of(m);
    let mut names = id_names.clone();
    names.extend(extra.iter().cloned());
    let present = m.present.clone().unwrap_or_default();
    let screen = present.screen.as_ref().map_or([256, 240], |s| s.size);
    let aspect = present
        .screen
        .as_ref()
        .and_then(|s| s.pixel_aspect.as_ref())
        .map_or("1".to_owned(), |r| format!("{}/{}", r.num, r.den));
    let tiles_based = m.assets.values().any(|s| s.kind == AssetKind::Tiles);
    let nes_colors = if tiles_based {
        "3 colors and transparent in each 8 by 8 tile (a console palette); paint flat shapes in a dark, a middle and a light tone"
    } else {
        "any colors; keep to the theme's palette"
    };
    let mut art = Vec::new();
    for (name, e) in &m.entities {
        let Some(s) = &e.core.sprite else { continue };
        let size = sprite_size(s, e.core.size.map(px));
        let mut frame_names = vec!["base".to_owned()];
        let mut timing = Vec::new();
        for (i, c) in s.clips.iter().enumerate() {
            for j in 0..c.frames.len() {
                frame_names.push(format!("clip{i}.{j}"));
            }
            let mut when = Vec::new();
            for t in &c.when.counters {
                let mut w = redact(&t.name, &names);
                if let Some(a) = t.at_least {
                    w.push_str(&format!(" >= {a}"));
                }
                if let Some(b) = t.below {
                    w.push_str(&format!(" < {b}"));
                }
                when.push(w);
            }
            for (k, v) in [
                ("on the ground", c.when.ground),
                ("moving", c.when.moving),
                ("rising", c.when.rising),
                ("climbing", c.when.climbing),
                ("jumped", c.when.jumped),
            ] {
                if let Some(v) = v {
                    when.push(if v { k.to_owned() } else { format!("not {k}") });
                }
            }
            let every = c
                .every
                .map(|e| format!(", {e} frames each"))
                .unwrap_or_default();
            timing.push(format!(
                "clip{i}: {} frames{every}, while {}",
                c.frames.len(),
                if when.is_empty() {
                    "its own state says (a word of its hook)".into()
                } else {
                    when.join(" and ")
                }
            ));
        }
        let mut anchor = match s.offset {
            Some([x, y]) => format!("top left at {x}, {y} px from the body's place"),
            None => "stands on the body's bottom edge".into(),
        };
        if s.flip_when_facing_left {
            anchor.push_str("; drawn facing right, mirrored to face left");
        }
        let desc = e
            .core
            .description
            .as_deref()
            .map(|d| scrub(d, &mut names))
            .unwrap_or_default();
        let words = role_words(&e.core.role);
        let words = if size[0] * size[1] >= 1024 {
            match words
                .strip_prefix("an ")
                .or_else(|| words.strip_prefix("a "))
            {
                Some(rest) => format!("a large {rest}"),
                None => format!("{words}, large"),
            }
        } else {
            words.to_owned()
        };
        let mut description = format!("{words}, {} by {} pixels", size[0], size[1]);
        if desc.split_whitespace().count() >= 3 {
            description.push_str(&format!(": {desc}"));
        }
        let id = entity_id(name, &e.core.role, &id_names);
        let geometry = Some(crate::theme_geometry::of_entity(
            m,
            e,
            &id,
            size,
            &frame_names,
            geo,
        ));
        art.push(ArtSlot {
            id,
            kind: "sprite".into(),
            role: e.core.role.clone(),
            size,
            frames: frame_names.len() as u32,
            frame_names,
            timing: if timing.is_empty() {
                "one picture".into()
            } else {
                timing.join("; ")
            },
            colors: nes_colors.into(),
            anchor,
            description,
            label: role_label(&e.core.role, size[0] * size[1] >= 1024).into(),
            poses: Vec::new(),
            geometry,
        });
    }
    for (i, fx) in present.effects.iter().enumerate() {
        let frames: usize = fx.phases.iter().map(|p| p.frames.len()).sum();
        let w = fx.phases.iter().map(|p| p.width.max(1)).max().unwrap_or(1);
        let h = fx
            .phases
            .iter()
            .flat_map(|p| {
                p.frames
                    .iter()
                    .map(|f| (f.len() as u32).div_ceil(p.width.max(1)))
            })
            .max()
            .unwrap_or(1)
            .max(1);
        art.push(ArtSlot {
            id: format!("effect.{i}"),
            kind: "sprite".into(),
            role: "effect".into(),
            size: [w * 8, h * 8],
            frames: frames as u32,
            frame_names: (0..frames).map(|k| k.to_string()).collect(),
            timing: format!("shown when `{}` fires", redact(&fx.event, &names)),
            colors: nes_colors.into(),
            anchor: "at the event's entity".into(),
            description: format!(
                "a short-lived picture when `{}` fires (points won, a spark)",
                redact(&fx.event, &names)
            ),
            label: "effect".into(),
            poses: Vec::new(),
            geometry: None,
        });
    }
    for (i, sc) in present.screens.iter().enumerate() {
        for k in 0..sc.sprite.len() {
            art.push(ArtSlot {
                id: format!("screen-sprite.{i}.{k}"),
                kind: "sprite".into(),
                role: "menu".into(),
                size: [8, 8],
                frames: 1,
                frame_names: vec!["base".into()],
                timing: "one picture".into(),
                colors: nes_colors.into(),
                anchor: "fixed on the screen, moved by a counter".into(),
                description: "a menu's cursor, 8 by 8".into(),
                label: "cursor".into(),
                poses: Vec::new(),
                geometry: None,
            });
        }
    }
    // Pictures: each level's layout, the HUD's layer, each screen's.
    let mut seen = BTreeSet::new();
    let mut pics = Vec::new();
    let mut extra = Vec::new();
    let mut picture = |slot: &str, entry: &str, role: &str, size: [u32; 2], what: String| {
        if seen.insert((slot.to_owned(), entry.to_owned())) {
            pics.push(ArtSlot {
                id: format!("picture.{slot}.{entry}"),
                kind: "picture".into(),
                role: role.into(),
                size,
                frames: 1,
                frame_names: vec!["base".into()],
                timing: "one picture".into(),
                colors: "any colors (drawn whole, not cut into tiles)".into(),
                anchor: "the screen's top left".into(),
                description: what,
                label: match role {
                    "menu" => "title screen".into(),
                    r => role_label(r, true).into(),
                },
                poses: Vec::new(),
                geometry: None,
            });
        }
    };
    for (name, l) in &m.levels {
        if let Some(lay) = &l.core.layout {
            let ladders = if l.platformer.ladders.is_some() {
                " and ladders"
            } else {
                ""
            };
            let ground = if l.platformer.ground.is_some() {
                "ground strips"
            } else {
                "solid ground"
            };
            picture(
                &lay.slot,
                &lay.entry,
                "background",
                screen,
                format!(
                    "the stage `{name}`, {} by {}: {ground}{ladders} where the model's geometry puts them (the hero stands on them, so keep them in place), on a plain backdrop",
                    screen[0], screen[1]
                ),
            );
        }
    }
    if let Some(h) = &present.hud {
        if let Some(lay) = &h.layer {
            let [w, hh] = h.size.unwrap_or([screen[0] / 8, screen[1] / 8]);
            picture(
                &lay.slot,
                &lay.entry,
                "hud",
                [w * 8, hh * 8],
                format!(
                    "the score display's fixed part, {} by {}: labels and boxes; the numbers are drawn over it from the font",
                    w * 8,
                    hh * 8
                ),
            );
        }
        extra.push(ArtSlot {
            id: "font".into(),
            kind: "font".into(),
            role: "hud".into(),
            size: [8, 8],
            frames: 10,
            frame_names: (0..10).map(|d| d.to_string()).collect(),
            timing: "one glyph a character".into(),
            colors: nes_colors.into(),
            anchor: "a cell of the score display".into(),
            description: "the digits 0 to 9 of the score, the bonus and the lives".into(),
            label: "digits".into(),
            poses: (0..10).map(|d| format!("the digit {d}")).collect(),
            geometry: None,
        });
    }
    for sc in &present.screens {
        if let Some(lay) = &sc.layout {
            picture(
                &lay.slot,
                &lay.entry,
                "menu",
                screen,
                format!(
                    "a whole screen shown while {} holds (a title and its menu): the game's own title and menu words, never the original's logo or names",
                    sc.states.join(" or ")
                ),
            );
        }
    }
    art.extend(pics);
    art.extend(extra);
    let mut sound = Vec::new();
    if let Some(snd) = &present.sound {
        for r in &snd.roles {
            let mut cue = format!("counter `{}`", redact(&r.counter, &names));
            if let Some(mk) = r.mask {
                cue.push_str(&format!(" & {mk:#04x}"));
            }
            match r.equals {
                Some(e) => cue.push_str(&format!(" == {e:#04x}")),
                None => cue.push_str(" != 0"),
            }
            cue.push_str(if r.held {
                ", from the frame it holds until it doesn't"
            } else {
                ", each frame it holds"
            });
            sound.push(SoundSlot {
                role: r.name.clone(),
                kind: match r.kind {
                    gp_model::model::SoundRoleKind::Music => "music".into(),
                    gp_model::model::SoundRoleKind::Sfx => "sfx".into(),
                },
                cue,
                frames: r.frames,
                loops: r.loops,
                description: scrub(&r.description, &mut names.clone()),
            });
        }
    }
    let slots = gp_package::index::slots_digest(&slot_lines(screen, &art, &sound, tiles_based));
    Kit {
        format: KIT_FORMAT.into(),
        game: game.into(),
        model: m.model.name.clone(),
        screen,
        pixel_aspect: aspect,
        slots,
        art,
        sound,
        tolerance: crate::theme_geometry::Tolerance::default(),
        surfaces: crate::theme_geometry::surfaces(m),
    }
}

/// The lines a kit's slot fingerprint hashes: the screen, then one an art
/// slot (id, kind, width by height, frames, color limit) and one a sound
/// role (role, kind, whether it loops). Labels, descriptions, poses,
/// timing, cues and anchors are words and are left out, so a words file
/// or a reworded kit gives the same fingerprint. `tiles` is whether the
/// model's art is cut into console tiles (3 colors and transparent in
/// each 8 by 8 tile).
pub fn slot_lines(
    screen: [u32; 2],
    art: &[ArtSlot],
    sound: &[SoundSlot],
    tiles: bool,
) -> Vec<String> {
    let mut v = vec![format!("screen {}x{}", screen[0], screen[1])];
    for a in art {
        let colors = if tiles && a.kind != "picture" {
            "tile-3+clear"
        } else {
            "any"
        };
        v.push(format!(
            "art {} {} {}x{} frames={} colors={colors}",
            a.id, a.kind, a.size[0], a.size[1], a.frames
        ));
    }
    for s in sound {
        v.push(format!("sound {} {} loops={}", s.role, s.kind, s.loops));
    }
    v
}

/// The kit's text, with a header for people.
pub fn kit_text(kit: &Kit) -> String {
    format!(
        "# The theme kit for {} (docs/theme-kit.md), written by `gp theme kit` from\n# the model. It names no character and holds no art: describe and draw\n# each slot from its role and description, and replace each sound role\n# with an original sound. .claude/skills/theme-generation/SKILL.md says how.\n{}",
        kit.game,
        toml::to_string(kit).expect("a kit serializes")
    )
}

/// The rows of a `rows` lifted file.
fn read_rows(dir: &Path, model: &Model, slot: &str) -> Result<Vec<Vec<u32>>> {
    let s = model
        .assets
        .get(slot)
        .with_context(|| format!("no slot `{slot}`"))?;
    let asset = s
        .from
        .as_ref()
        .map(|f| f.asset.clone())
        .with_context(|| format!("slot `{slot}` has no from"))?;
    let file = dir.join(format!("{asset}.toml"));
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let v: toml::Table = text
        .parse()
        .with_context(|| format!("{}", file.display()))?;
    Ok(v.get("rows")
        .and_then(|r| r.as_array())
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    r.as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_integer())
                                .map(|x| x as u32)
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The lifted palettes slot's colors (NES numbers), 8 palettes of 4.
fn read_palettes(dir: &Path, model: &Model) -> Result<Vec<[u8; 4]>> {
    let Some((_, s)) = model
        .assets
        .iter()
        .find(|(_, s)| s.kind == AssetKind::Palettes)
    else {
        return Ok(Vec::new());
    };
    let asset = s.from.as_ref().map(|f| f.asset.clone()).unwrap_or_default();
    let file = dir.join(format!("{asset}.toml"));
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let v: toml::Table = text.parse()?;
    Ok(v.get("palettes")
        .and_then(|r| r.as_array())
        .map(|ps| {
            ps.iter()
                .map(|p| {
                    let mut c = [0x0F; 4];
                    for (i, x) in p.as_array().into_iter().flatten().take(4).enumerate() {
                        c[i] = x.as_integer().unwrap_or(15) as u8;
                    }
                    c
                })
                .collect()
        })
        .unwrap_or_default())
}

/// A tiles slot's lifted file and its size in tiles.
pub(crate) fn bank_size(map: &gp_assets::AssetMap, model: &Model, slot: &str) -> Option<[u32; 2]> {
    let from = model.assets.get(slot)?.from.as_ref()?;
    map.assets
        .iter()
        .find(|a| a.name() == from.asset)
        .and_then(|a| match a {
            gp_assets::Asset::Chr {
                tiles,
                tiles_per_row,
                ..
            } => Some([*tiles_per_row, tiles.div_ceil(*tiles_per_row)]),
            _ => None,
        })
}

/// A sprite frame cut into tiles.
#[allow(clippy::too_many_arguments)]
fn cut(
    s: &Sprite,
    tiles: &[u32],
    cells: &[gp_model::model::Cell],
    slot: &str,
    pals: &[[u8; 4]],
    sprite_bank: bool,
) -> Cut {
    let base_pal = s.palette;
    let colors_of = |p: u32| -> [u8; 4] {
        let i = match &s.colors {
            Some(c) => c.palette as usize,
            None if sprite_bank => 4 + p as usize,
            None => p as usize,
        };
        pals.get(i).copied().unwrap_or([0x0F, 0x00, 0x10, 0x30])
    };
    let w = s.width.max(1);
    let mut places = Vec::new();
    for (k, &t) in tiles.iter().enumerate() {
        let cell = cells.get(k);
        let [col, row] = cell
            .and_then(|c| c.at)
            .unwrap_or([k as u32 % w, k as u32 / w]);
        places.push(Place {
            tile: t,
            col,
            row,
            flip_x: cell.is_some_and(|c| c.flip_x),
            flip_y: cell.is_some_and(|c| c.flip_y),
            colors: colors_of(cell.and_then(|c| c.palette).unwrap_or(base_pal)),
        });
    }
    Cut {
        slot: slot.into(),
        places,
    }
}

/// The local part of a kit, from the model and its lifted assets.
pub fn local_of(m: &Model, kit: &Kit, dir: &Path, map: &gp_assets::AssetMap) -> Result<Local> {
    let pals = read_palettes(dir, m)?;
    let names = names_of(m);
    let mut local = Local {
        game: kit.game.clone(),
        ..Local::default()
    };
    for (name, s) in &m.assets {
        if s.kind == AssetKind::Tiles
            && let Some(sz) = bank_size(map, m, name)
        {
            local.banks.insert(name.clone(), sz);
        }
    }
    let sprite_slots: BTreeSet<&str> = m
        .entities
        .values()
        .filter_map(|e| e.core.sprite.as_ref())
        .filter(|s| !s.name_table)
        .map(|s| s.slot.as_str())
        .collect();
    let present = m.present.clone().unwrap_or_default();
    for (name, e) in &m.entities {
        let Some(s) = &e.core.sprite else { continue };
        let sprite_bank = sprite_slots.contains(s.slot.as_str()) && !s.name_table;
        let base_tiles = if !s.tiles.is_empty() {
            s.tiles.clone()
        } else if let Some(f) = &s.frame {
            read_rows(dir, m, &f.slot)?
                .get(f.row as usize)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut cuts = vec![cut(s, &base_tiles, &s.cells, &s.slot, &pals, sprite_bank)];
        for c in &s.clips {
            for f in &c.frames {
                let tiles = if !f.tiles.is_empty() {
                    f.tiles.clone()
                } else if let Some(row) = f.row {
                    let slot = f
                        .slot
                        .clone()
                        .or_else(|| s.frame.as_ref().map(|x| x.slot.clone()))
                        .unwrap_or_default();
                    read_rows(dir, m, &slot)?
                        .get(row as usize)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    base_tiles.clone()
                };
                let cells = if f.cells.is_empty() {
                    &s.cells
                } else {
                    &f.cells
                };
                let mut c = cut(s, &tiles, cells, &s.slot, &pals, sprite_bank);
                if f.flip_x || f.flip_y {
                    let w = c.places.iter().map(|p| p.col + 1).max().unwrap_or(1);
                    let h = c.places.iter().map(|p| p.row + 1).max().unwrap_or(1);
                    for p in &mut c.places {
                        if f.flip_x {
                            p.col = w - 1 - p.col;
                            p.flip_x = !p.flip_x;
                        }
                        if f.flip_y {
                            p.row = h - 1 - p.row;
                            p.flip_y = !p.flip_y;
                        }
                    }
                }
                cuts.push(c);
            }
        }
        local
            .cuts
            .insert(entity_id(name, &e.core.role, &names), cuts);
    }
    for (i, fx) in present.effects.iter().enumerate() {
        let mut cuts = Vec::new();
        for ph in &fx.phases {
            let s = Sprite {
                slot: ph.slot.clone(),
                width: ph.width,
                palette: ph.palette,
                cells: ph.cells.clone(),
                ..sprite_default()
            };
            for f in &ph.frames {
                cuts.push(cut(&s, f, &ph.cells, &ph.slot, &pals, true));
            }
        }
        local.cuts.insert(format!("effect.{i}"), cuts);
    }
    for (i, sc) in present.screens.iter().enumerate() {
        for (k, sp) in sc.sprite.iter().enumerate() {
            let s = Sprite {
                slot: sp.tiles.clone(),
                width: 1,
                palette: sp.palette.unwrap_or(0),
                ..sprite_default()
            };
            local.cuts.insert(
                format!("screen-sprite.{i}.{k}"),
                vec![cut(&s, &[sp.tile], &[], &sp.tiles, &pals, true)],
            );
        }
    }
    // The font: its charset's characters in the HUD's tiles.
    let charset = font_charset(m, map);
    if let Some(h) = &present.hud {
        let hud_pal = h.field.first().and_then(|f| f.palette).unwrap_or(0) as usize;
        let colors = pals
            .get(hud_pal)
            .copied()
            .unwrap_or([0x0F, 0x00, 0x10, 0x30]);
        let mut cuts = Vec::new();
        for d in 0..10u8 {
            let t = charset
                .get(&char::from(b'0' + d))
                .copied()
                .unwrap_or(u32::from(d));
            cuts.push(Cut {
                slot: h.tiles.clone(),
                places: vec![Place {
                    tile: t,
                    col: 0,
                    row: 0,
                    flip_x: false,
                    flip_y: false,
                    colors,
                }],
            });
        }
        local.cuts.insert("font".into(), cuts);
        for (c, t) in &charset {
            if c.is_ascii_digit() || *c == ' ' {
                continue;
            }
            local.letters.push((
                *c,
                Cut {
                    slot: h.tiles.clone(),
                    places: vec![Place {
                        tile: *t,
                        col: 0,
                        row: 0,
                        flip_x: false,
                        flip_y: false,
                        colors,
                    }],
                },
            ));
        }
        if let Some(&space) = charset.get(&' ') {
            local.blank.entry(h.tiles.clone()).or_default().push(space);
        }
    }
    // Levels built from objects: each metatile's tiles by its role.
    if m.assets.values().any(|s| s.kind == AssetKind::LevelObjects)
        && let Some(bank) = gp_look::present::level_tiles_slot(m)
    {
        let mut groups: Vec<(&String, &gp_model::model::AssetSlot)> = m
            .assets
            .iter()
            .filter(|(_, s)| s.kind == AssetKind::Metatiles)
            .collect();
        groups.sort_by_key(|(n, _)| n.as_str());
        for (g, (_, s)) in groups.iter().enumerate() {
            let Some(from) = &s.from else { continue };
            let file = dir.join(format!("{}.toml", from.asset));
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let v: toml::Table = text.parse()?;
            let colors = pals.get(g).copied().unwrap_or([0x0F, 0x00, 0x10, 0x30]);
            for (i, mt) in v
                .get("metatile")
                .and_then(|x| x.as_array())
                .into_iter()
                .flatten()
                .enumerate()
            {
                let tiles: Vec<u32> = mt
                    .get("tiles")
                    .and_then(|t| t.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.as_integer().map(|v| v as u32))
                    .collect();
                if tiles.len() != 4 {
                    continue;
                }
                let role = s
                    .item_roles
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| "background".into());
                let places = tiles
                    .iter()
                    .enumerate()
                    .map(|(k, &tile)| Place {
                        tile,
                        col: k as u32 % 2,
                        row: k as u32 / 2,
                        flip_x: false,
                        flip_y: false,
                        colors,
                    })
                    .collect();
                local.objects.push((
                    role,
                    Cut {
                        slot: bank.clone(),
                        places,
                    },
                ));
            }
        }
    }
    // The kinds a changed cell of a single-tile level is drawn as, in
    // each bank a level's layout is drawn with (the layout slot's own
    // `tiles`, else the level's: Original Edition's 50m has a bank of its
    // own). A level of metatiles (`objects`, above) draws them its own way.
    if local.objects.is_empty()
        && let Some(cp) = &present.cells
    {
        let colors = pals.first().copied().unwrap_or([0x0F, 0x00, 0x10, 0x30]);
        let banks: BTreeSet<String> = m
            .levels
            .values()
            .filter_map(|l| l.core.layout.as_ref())
            .filter_map(|lay| {
                m.assets
                    .get(&lay.slot)
                    .and_then(|s| s.tiles.clone())
                    .or_else(|| gp_look::present::level_tiles_slot(m))
            })
            .collect();
        for (bank, (kind, &tile)) in banks
            .iter()
            .flat_map(|b| cp.kinds.iter().map(move |k| (b, k)))
        {
            // A cell the game empties is drawn with this tile: it stays
            // blank, as the font's space does, so a figure painted there
            // (Original Edition's 50m foe, whose pose shows it in its
            // corners) never shows in a place the game cleared.
            if kind == "empty" {
                let blank = local.blank.entry(bank.clone()).or_default();
                if !blank.contains(&tile) {
                    blank.push(tile);
                }
                continue;
            }
            local.kind_cells.push((
                kind.clone(),
                Cut {
                    slot: bank.clone(),
                    places: vec![Place {
                        tile,
                        col: 0,
                        row: 0,
                        flip_x: false,
                        flip_y: false,
                        colors,
                    }],
                },
            ));
        }
    }
    // Pictures: each cell's character, where the font knows it.
    let by_tile: BTreeMap<u32, char> = charset.iter().map(|(c, t)| (*t, *c)).collect();
    for a in kit.art.iter().filter(|a| a.kind == "picture") {
        let Some(rest) = a.id.strip_prefix("picture.") else {
            continue;
        };
        let Some((slot, entry)) = rest.split_once('.') else {
            continue;
        };
        let (names, cell_pals) = match gp_look::present::layout_cells(m, dir, slot, entry) {
            Ok(x) => x,
            Err(e) => {
                local.skipped.push(format!("{}: {e}", a.id));
                continue;
            }
        };
        let (cols, rows) = (a.size[0] / 8, a.size[1] / 8);
        let mut text = Vec::new();
        let mut role_rows = Vec::new();
        for r in 0..rows.min(30) {
            let mut line = String::new();
            let mut rl = String::new();
            for c in 0..cols.min(32) {
                let i = (r * 32 + c) as usize;
                line.push(match names[i] {
                    None => ' ',
                    Some(t) => by_tile.get(&u32::from(t)).copied().unwrap_or('?'),
                });
                rl.push(char::from(b'0' + cell_pals[i]));
            }
            text.push(line);
            role_rows.push(rl);
        }
        local.cells.insert(a.id.clone(), (text, role_rows));
    }
    // Each sprite frame's visible body in the original.
    let mut orig = BTreeMap::new();
    for (slot, sz) in &local.banks {
        let Some(asset) = m
            .assets
            .get(slot)
            .and_then(|s| s.from.as_ref())
            .and_then(|f| map.assets.iter().find(|a| a.name() == f.asset))
        else {
            continue;
        };
        let Ok(png) = fs::read(dir.join(asset.file())) else {
            continue;
        };
        if let Ok(px) = gp_assets::chr::read_png(&png, (sz[0] * 8, sz[1] * 8)) {
            orig.insert(slot.clone(), (*sz, px));
        }
    }
    for a in kit.art.iter().filter(|a| a.kind == "sprite") {
        let Some(cuts) = local.cuts.get(&a.id) else {
            continue;
        };
        let bodies = cuts
            .iter()
            .map(|c| {
                let (sz, px) = orig.get(&c.slot)?;
                let f = crate::theme_original::frame_of(px, *sz, c);
                crate::theme_geometry::body_of(&f.t, f.w)
            })
            .collect();
        local.bodies.insert(a.id.clone(), bodies);
    }
    Ok(local)
}

fn sprite_default() -> Sprite {
    Sprite {
        slot: String::new(),
        tiles: Vec::new(),
        frame: None,
        width: 1,
        palette: 0,
        flip_when_facing_left: false,
        copy: None,
        cells: Vec::new(),
        colors: None,
        clips: Vec::new(),
        blink: None,
        offset: None,
        behind: false,
        behind_when: None,
        split: None,
        replaced: None,
        front: false,
        name_table: false,
        flip_in_place: false,
        leaves: None,
    }
}

/// The font's charset, by character: from the HUD's font slot in the
/// template's own asset map.
fn font_charset(m: &Model, map: &gp_assets::AssetMap) -> BTreeMap<char, u32> {
    let mut out = BTreeMap::new();
    let Some(h) = m.present.as_ref().and_then(|p| p.hud.as_ref()) else {
        return out;
    };
    let Some(from) = m.assets.get(&h.font).and_then(|s| s.from.as_ref()) else {
        return out;
    };
    if let Some(a) = map.assets.iter().find(|a| a.name() == from.asset)
        && let Some(cs) = a.charset()
    {
        for c in (32u8..127).map(char::from) {
            if let Some(t) = cs.tile(c) {
                out.insert(c, u32::from(t));
            }
        }
    }
    out
}

/// The local kit folder for a game.
pub fn local_dir(paths: &Paths, game: &str) -> PathBuf {
    paths.home.join("work").join("theme-kits").join(game)
}

/// A NES color's lightness, 0 to 100.
fn lightness(n: u8) -> f64 {
    let rgb = gp_look::present::nes_colors()[usize::from(n & 0x3F)];
    Rgb(rgb).lab()[0]
}

/// For a tile's colors, the color index (1 to 3) of each tone: dark,
/// middle and light, by the colors' lightness.
pub fn tone_index(colors: [u8; 4]) -> [u8; 3] {
    let mut idx = [1u8, 2, 3];
    idx.sort_by(|a, b| {
        lightness(colors[usize::from(*a)])
            .partial_cmp(&lightness(colors[usize::from(*b)]))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    idx
}

/// A slot's size, its tile indices and the slot that took each tile.
type Bank = ([u32; 2], Vec<u8>, BTreeMap<u32, String>);

/// Tiles banks being filled: each slot's indices and which tiles are
/// taken, with the slot that took each.
pub struct Banks {
    pub banks: BTreeMap<String, Bank>,
    blank: BTreeMap<String, Vec<u32>>,
    pub conflicts: Vec<String>,
}

impl Banks {
    pub fn new(local: &Local) -> Self {
        Self {
            banks: local
                .banks
                .iter()
                .map(|(k, sz)| {
                    (
                        k.clone(),
                        (*sz, vec![0; (sz[0] * sz[1] * 64) as usize], BTreeMap::new()),
                    )
                })
                .collect(),
            blank: local.blank.clone(),
            conflicts: Vec::new(),
        }
    }

    /// Cuts a frame of tones (0 clear, 1 dark, 2 middle, 3 light; `w` wide)
    /// into its tiles. A tile another slot took keeps that slot's art.
    pub fn put(&mut self, id: &str, cut: &Cut, tones: &[u8], w: u32) {
        let Some((sz, px, owner)) = self.banks.get_mut(&cut.slot) else {
            return;
        };
        let blank = self.blank.get(&cut.slot).cloned().unwrap_or_default();
        for p in &cut.places {
            if blank.contains(&p.tile) || p.tile >= sz[0] * sz[1] {
                continue;
            }
            if let Some(o) = owner.get(&p.tile) {
                if o != id {
                    self.conflicts.push(format!(
                        "tile {:#04x} of {}: {id} and {o}",
                        p.tile, cut.slot
                    ));
                }
                continue;
            }
            owner.insert(p.tile, id.to_owned());
            let map = tone_index(p.colors);
            let (tx, ty) = ((p.tile % sz[0]) * 8, (p.tile / sz[0]) * 8);
            for y in 0..8 {
                for x in 0..8 {
                    let sx = p.col * 8 + if p.flip_x { 7 - x } else { x };
                    let sy = p.row * 8 + if p.flip_y { 7 - y } else { y };
                    let t = tones.get((sy * w + sx) as usize).copied().unwrap_or(0);
                    let v = if t == 0 {
                        0
                    } else {
                        map[usize::from(t.min(3) - 1)]
                    };
                    px[((ty + y) * sz[0] * 8 + tx + x) as usize] = v;
                }
            }
        }
    }

    /// Fills the clear pixels of a frame's tiles that `id` itself took
    /// with the frame's tones, so a frame that shares a tile with another
    /// of its slot at another place shows its whole shape: the tile holds
    /// both frames' ink. Tiles another slot took are left alone.
    pub fn merge(&mut self, id: &str, cut: &Cut, tones: &[u8], w: u32) {
        let Some((sz, px, owner)) = self.banks.get_mut(&cut.slot) else {
            return;
        };
        for p in &cut.places {
            if p.tile >= sz[0] * sz[1] || owner.get(&p.tile).is_none_or(|o| o != id) {
                continue;
            }
            let map = tone_index(p.colors);
            let (tx, ty) = ((p.tile % sz[0]) * 8, (p.tile / sz[0]) * 8);
            for y in 0..8 {
                for x in 0..8 {
                    let sx = p.col * 8 + if p.flip_x { 7 - x } else { x };
                    let sy = p.row * 8 + if p.flip_y { 7 - y } else { y };
                    let t = tones.get((sy * w + sx) as usize).copied().unwrap_or(0);
                    let i = ((ty + y) * sz[0] * 8 + tx + x) as usize;
                    if t != 0 && px[i] == 0 {
                        px[i] = map[usize::from(t.min(3) - 1)];
                    }
                }
            }
        }
    }

    /// Each bank as an indexed `.pix` text.
    pub fn pix(&self, note: &str) -> Vec<(String, String)> {
        self.banks
            .iter()
            .map(|(slot, (sz, px, _))| {
                let (w, h) = (sz[0] * 8, sz[1] * 8);
                let mut s = format!(
                    "pix 1\n# {note}\n# The tiles slot `{slot}`: color 1, 2 and 3 are the console palette's\n# entries 1, 2 and 3; the game's palettes color them, and the theme's rules after.\nsize {w} {h}\n. transparent\n1 #555555\n2 #aaaaaa\n3 #ffffff\n"
                );
                for y in 0..h {
                    for x in 0..w {
                        s.push(match px[(y * w + x) as usize] {
                            0 => '.',
                            1 => '1',
                            2 => '2',
                            _ => '3',
                        });
                    }
                    s.push('\n');
                }
                (slot.clone(), s)
            })
            .collect()
    }
}

/// The frames of an RGBA picture, as tones by lightness: clear stays 0,
/// and the opaque pixels split into thirds of their lightness range.
pub fn tones_of(img: &Rgba) -> Vec<u8> {
    let l: Vec<Option<f64>> = img
        .pixels
        .iter()
        .map(|p| (p[3] >= 128).then(|| Rgb([p[0], p[1], p[2]]).lab()[0]))
        .collect();
    let (lo, hi) = l
        .iter()
        .flatten()
        .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let span = (hi - lo).max(1.0);
    l.iter()
        .map(|v| match v {
            None => 0,
            Some(v) => 1 + (((v - lo) / span) * 2.999).floor() as u8,
        })
        .collect()
}

/// `gp theme kit`: writes the shareable kit, and with `--rom` the local
/// part.
pub fn kit(
    game: &Path,
    rom: Option<&Path>,
    out: Option<&Path>,
    assets: Option<&Path>,
    paths: &Paths,
    rec: &mut RunRecorder,
) -> Result<Outcome> {
    let dir = template_dir(game)?;
    let (m, _) = crate::theme::read_model(&dir)?;
    let (id, accepts, map_file) = manifest(&dir);
    let (kit, problems) = kit_for(&m, &id, &dir)?;
    if !problems.is_empty() {
        bail!(
            "the words file doesn't fit the kit (docs/theme-kit.md, \"Prompt words\"):\n  {}",
            problems.join("\n  ")
        );
    }
    let out = out.map_or_else(|| dir.join("themes").join("kit"), Path::to_owned);
    fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    fs::write(out.join(KIT_FILE), kit_text(&kit)).context("writing kit.toml")?;
    let mut text = format!(
        "wrote {} with {} art slots and {} sound roles; {}'s slot fingerprint {}",
        out.join(KIT_FILE).display(),
        kit.art.len(),
        kit.sound.len(),
        kit.model,
        kit.slots
    );
    let mut local_path = None;
    if let Some(rom) = rom {
        let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
        let sum = gp_theme::format::sha256_hex(&bytes);
        let body = gp_theme::format::sha256_hex(bytes.get(16..).unwrap_or_default());
        if !accepts.is_empty() && !accepts.contains(&sum) && !accepts.contains(&body) {
            bail!(
                "{} has SHA-256 {sum}, which the template doesn't accept ({})",
                rom.display(),
                accepts.join(", ")
            );
        }
        let source = m.model.source.clone().unwrap_or_default();
        let adir = assets.map_or_else(
            || paths.home.join("work").join("assets").join(&source),
            Path::to_owned,
        );
        if !adir.join("assets.toml").is_file() {
            bail!(
                "no lifted assets in {}; run `gp template extract {}` with your ROM first",
                adir.display(),
                dir.display()
            );
        }
        let map_text = fs::read_to_string(map_file.as_deref().unwrap_or(&adir.join("assets.toml")))
            .context("reading the asset map")?;
        let map = gp_assets::AssetMap::parse(&map_text).map_err(anyhow::Error::msg)?;
        let mut local = local_of(&m, &kit, &adir, &map)?;
        let ldir = local_dir(paths, &id);
        if gp_evidence::paths::enclosing_git_work_tree(&ldir).is_some() {
            bail!(
                "{} is inside a git work tree; the local kit stays outside",
                ldir.display()
            );
        }
        fs::create_dir_all(&ldir)?;
        let sheet = reference_sheet(&m, &local, &adir, &map)?;
        fs::write(ldir.join("reference.png"), sheet.png())?;
        // A guide for each stage picture: the model's ground boxes (the
        // rows the hero stands on, with the 8 rows below) and its ladders,
        // which a stage's art must keep in place. From the model's level
        // geometry, not the game's pictures.
        let model_file = dir.join(gp_model::MODEL_FILE);
        let bundle = crate::theme_run::load_bundle(&model_file, &adir)?;
        let spec = gp_interp::compile::compile(&m, &bundle.assets).map_err(anyhow::Error::msg)?;
        for (name, l) in &m.levels {
            let Some(lay) = &l.core.layout else { continue };
            let Some(li) = spec.levels.iter().position(|x| &x.name == name) else {
                continue;
            };
            let sl = &spec.levels[li];
            local.stages.insert(
                format!("picture.{}.{}", lay.slot, lay.entry),
                crate::theme_geometry::stage_of(&spec, li),
            );
            let mut g = Rgba::new(kit.screen[0], kit.screen[1]);
            for b in &sl.ground {
                for x in b[0]..=b[2] {
                    for y in b[1]..b[1] + 8 {
                        if x >= 0 && y >= 0 {
                            g.set(x as u32, y as u32, [200, 140, 80, 255]);
                        }
                    }
                }
            }
            for b in &sl.ladders {
                for y in b[1]..=b[3] {
                    for x in b[0]..=b[2] {
                        if x >= 0 && y >= 0 && (x == b[0] || x == b[2] || (y - b[1]) % 4 == 0) {
                            g.set(x as u32, y as u32, [120, 180, 230, 255]);
                        }
                    }
                }
            }
            fs::write(
                ldir.join(format!("guide-picture.{}.{}.png", lay.slot, lay.entry)),
                g.png(),
            )?;
        }
        fs::write(
            ldir.join("local.json"),
            serde_json::to_string_pretty(&local)?,
        )?;
        rec.output("kit-local", ldir.display().to_string(), None);
        text.push_str(&format!(
            "\nwrote the local part (tiles each frame is cut into, and reference.png, the original's sizes and poses: never an image reference for new art) to {}",
            ldir.display()
        ));
        local_path = Some(ldir);
    }
    Ok(Outcome::ok(
        json!({ "kit": out.join(KIT_FILE), "model": kit.model, "slots": kit.slots, "art": kit.art.len(), "sound": kit.sound.len(), "local": local_path }),
        text,
    ))
}

/// The reference sheet: each sprite slot's frames from the lifted tiles,
/// in gray tones, a row a slot. Local only.
fn reference_sheet(
    m: &Model,
    local: &Local,
    dir: &Path,
    map: &gp_assets::AssetMap,
) -> Result<Rgba> {
    let mut banks = BTreeMap::new();
    for (slot, sz) in &local.banks {
        let from = m
            .assets
            .get(slot)
            .and_then(|s| s.from.as_ref())
            .map(|f| f.asset.clone());
        let Some(asset) = from.and_then(|a| map.assets.iter().find(|x| x.name() == a)) else {
            continue;
        };
        let png = fs::read(dir.join(asset.file()))?;
        let px =
            gp_assets::chr::read_png(&png, (sz[0] * 8, sz[1] * 8)).map_err(anyhow::Error::msg)?;
        banks.insert(slot.clone(), (*sz, px));
    }
    let rows: Vec<(&String, &Vec<Cut>)> = local.cuts.iter().collect();
    let cell = 56u32;
    let width = cell
        * rows
            .iter()
            .map(|(_, c)| c.len() as u32)
            .max()
            .unwrap_or(1)
            .min(40);
    let mut img = Rgba::new(width.max(cell), cell * rows.len().max(1) as u32);
    for (r, (_, cuts)) in rows.iter().enumerate() {
        for (k, c) in cuts.iter().enumerate().take(40) {
            let Some((sz, px)) = banks.get(&c.slot) else {
                continue;
            };
            for p in &c.places {
                let (tx, ty) = ((p.tile % sz[0]) * 8, (p.tile / sz[0]) * 8);
                for y in 0..8 {
                    for x in 0..8 {
                        let sx = if p.flip_x { 7 - x } else { x };
                        let sy = if p.flip_y { 7 - y } else { y };
                        let v = px
                            .get(((ty + sy) * sz[0] * 8 + tx + sx) as usize)
                            .copied()
                            .unwrap_or(0);
                        if v == 0 {
                            continue;
                        }
                        let g = 60 + v * 60;
                        img.set(
                            k as u32 * cell + p.col * 8 + x,
                            r as u32 * cell + p.row * 8 + y,
                            [g, g, g, 255],
                        );
                    }
                }
            }
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_leaves_out_names_and_addresses() {
        let mut names = BTreeSet::new();
        assert_eq!(
            scrub("Donkey Kong, who throws the barrels ($EB06)", &mut names),
            "who throws the barrels"
        );
        assert!(names.contains("kong"));
        assert_eq!(scrub("Mario", &mut names), "");
        assert_eq!(
            scrub("A fire out of the oil drum", &mut names),
            "a fire out of the oil drum"
        );
        assert_eq!(
            redact("pauline_shown", &["pauline".into()].into()),
            "x_shown"
        );
    }

    #[test]
    fn a_tile_takes_its_tones_by_the_colors_lightness() {
        // 0x0F black, 0x30 white, 0x16 red: dark is 0x16? No: black is
        // index 0, so the three are white (lightest), red and 0x0F.
        let t = tone_index([0x0F, 0x30, 0x16, 0x0F]);
        assert_eq!(t[2], 1, "white is the light tone");
        assert_eq!(t[0], 3, "black is the dark tone");
    }

    /// A made-up slot, with words in every field the fingerprint leaves out.
    fn slot(id: &str, kind: &str, size: [u32; 2], frames: u32) -> ArtSlot {
        ArtSlot {
            id: id.into(),
            kind: kind.into(),
            role: "enemy".into(),
            size,
            frames,
            frame_names: (0..frames).map(|k| k.to_string()).collect(),
            timing: "one picture".into(),
            colors: "any colors".into(),
            anchor: "top left".into(),
            description: "a made-up thing".into(),
            label: "thing".into(),
            poses: Vec::new(),
            geometry: None,
        }
    }

    fn made_up() -> (Vec<ArtSlot>, Vec<SoundSlot>) {
        let art = vec![
            slot("entity.a", "sprite", [16, 16], 3),
            slot("picture.stage.main", "picture", [256, 240], 1),
        ];
        let sound = vec![SoundSlot {
            role: "jump".into(),
            kind: "sfx".into(),
            cue: "counter `x` != 0".into(),
            frames: Some(8),
            loops: false,
            description: "a made-up jump".into(),
        }];
        (art, sound)
    }

    fn digest(art: &[ArtSlot], sound: &[SoundSlot]) -> String {
        gp_package::index::slots_digest(&slot_lines([256, 240], art, sound, true))
    }

    /// The fingerprint of a made-up kit, written out, so a run on any
    /// machine that hashes it differently fails here.
    #[test]
    fn the_slot_fingerprint_is_the_same_on_every_run() {
        let (art, sound) = made_up();
        assert_eq!(
            slot_lines([256, 240], &art, &sound, true),
            [
                "screen 256x240",
                "art entity.a sprite 16x16 frames=3 colors=tile-3+clear",
                "art picture.stage.main picture 256x240 frames=1 colors=any",
                "sound jump sfx loops=false",
            ]
        );
        assert_eq!(digest(&art, &sound), digest(&art, &sound));
        assert_eq!(digest(&art, &sound), FINGERPRINT);
    }

    const FINGERPRINT: &str = "14217500030c58a4d375c0df0a3c729c11fb8b4be05ffe6505f62ca92dc2796f";

    #[test]
    fn the_slot_fingerprint_changes_with_a_shape_and_not_with_words() {
        let (art, sound) = made_up();
        let d = digest(&art, &sound);
        // Words: every field a words file or a rewording changes.
        let (mut a2, mut s2) = made_up();
        let w = crate::theme_words::Words {
            art: vec![crate::theme_words::ArtWords {
                id: "entity.a".into(),
                label: Some("a made-up foe".into()),
                description: Some("a small round made-up foe".into()),
                poses: vec!["one".into(), "two".into(), "three".into()],
                ..Default::default()
            }],
            sound: vec![crate::theme_words::SoundWords {
                role: "jump".into(),
                description: Some("a short hop".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut kit = Kit {
            format: KIT_FORMAT.into(),
            game: "made-up".into(),
            model: "made_up".into(),
            screen: [256, 240],
            pixel_aspect: "1".into(),
            slots: d.clone(),
            art: a2.clone(),
            sound: s2.clone(),
            tolerance: Default::default(),
            surfaces: Vec::new(),
        };
        assert!(crate::theme_words::apply(&mut kit, &w).is_empty());
        assert_eq!(kit.art[0].label, "a made-up foe");
        assert_eq!(digest(&kit.art, &kit.sound), d, "a words file");
        a2[0].timing = "reworded timing".into();
        a2[0].colors = "reworded colors".into();
        a2[0].anchor = "reworded anchor".into();
        a2[0].role = "hazard".into();
        a2[0].frame_names = vec!["x".into(), "y".into(), "z".into()];
        s2[0].cue = "reworded cue".into();
        s2[0].frames = Some(12);
        assert_eq!(digest(&a2, &s2), d, "reworded fields");
        // Shapes: each change gives another fingerprint.
        let mut changed = Vec::new();
        let (mut a, s) = made_up();
        a[0].size = [16, 24];
        changed.push(("a slot's size", digest(&a, &s)));
        let (mut a, s) = made_up();
        a[0].frames = 4;
        changed.push(("a slot's frames", digest(&a, &s)));
        let (mut a, s) = made_up();
        a.pop();
        changed.push(("a slot gone", digest(&a, &s)));
        let (mut a, s) = made_up();
        a[0].id = "entity.b".into();
        changed.push(("a slot renamed", digest(&a, &s)));
        let (a, mut s) = made_up();
        s[0].loops = true;
        changed.push(("a sound that loops", digest(&a, &s)));
        let (a, mut s) = made_up();
        s[0].role = "hop".into();
        changed.push(("a sound role renamed", digest(&a, &s)));
        let (a, mut s) = made_up();
        s.clear();
        changed.push(("a sound role gone", digest(&a, &s)));
        let (a, s) = made_up();
        changed.push((
            "art not cut into tiles",
            gp_package::index::slots_digest(&slot_lines([256, 240], &a, &s, false)),
        ));
        changed.push((
            "another screen",
            gp_package::index::slots_digest(&slot_lines([256, 224], &a, &s, true)),
        ));
        for (what, x) in &changed {
            assert_ne!(x, &d, "{what}");
        }
        let all: BTreeSet<&String> = changed.iter().map(|(_, x)| x).collect();
        assert_eq!(all.len(), changed.len(), "each change its own fingerprint");
    }

    /// Every template's kit.toml holds the fingerprint its model gives now,
    /// words file and all; Donkey Kong's two versions differ.
    #[test]
    fn templates_kits_hold_their_models_fingerprint() {
        // gameprinter: its own templates (game-printer's had every game's).
        let mut by_model = BTreeMap::new();
        for dir in crate::theme_template::test_own_templates() {
            let kit_file = dir.join("themes/kit").join(KIT_FILE);
            let Ok(text) = fs::read_to_string(&kit_file) else {
                continue;
            };
            let on_disk: Kit = toml::from_str(&text).unwrap();
            let (m, _) = crate::theme::read_model(&dir).unwrap();
            let (kit, _) = kit_for(&m, &on_disk.game, &dir).unwrap();
            assert_eq!(
                kit.slots,
                kit_of(&m, &on_disk.game).slots,
                "{}: words",
                dir.display()
            );
            assert_eq!(
                on_disk.slots,
                kit.slots,
                "{}: the kit's slot fingerprint isn't its model's; run gp theme kit {}",
                kit_file.display(),
                dir.display()
            );
            by_model.insert(kit.model.clone(), kit.slots);
        }
        assert!(!by_model.is_empty(), "{by_model:?}");
    }
}
