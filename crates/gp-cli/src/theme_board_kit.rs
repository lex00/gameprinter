//! The theme kit of an arcade game drawn by its board's art
//! (docs/theme-kit.md, "A game drawn by its board"): one art slot for each
//! entity the board draws, with the picture codes its sprite entry shows
//! as frames; one for each screen the levels' looks show; one for each
//! counter drawn as digits or as a tile repeated. The shareable kit lists
//! what the model says (a code the model fixes); the codes a body shows
//! come from the person's own romset, so `gp theme kit GAME --rom ROM`
//! reads them from `gp theme look sheet`'s names.toml and keeps them, with
//! each code's visible body, in the local part (local.json, never
//! committed).
//!
//! A slot's frames are named by the code in hex (`4d`), a screen's and a
//! tile's `base`, a digit's by the digit, and `picture` gives the name a
//! theme's rules draw it by (`sprite-ENTITY-CODE`, the screen's slot,
//! `counter-NAME-D`), so the kit itself names no entity whose name may be
//! a character's.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Result, anyhow};
use gp_look::board::{BoardArt, CELL_SIZE, SCREEN_SIZE, SPRITE_SIZE};
use gp_model::Model;
use gp_model::model::BoardByte;

use crate::theme_kit::{ArtSlot, Kit, Local, entity_id, role_label, role_words};

/// A board sprite's slot kind: one picture a code, 32 by 32.
pub(crate) const SPRITE: &str = "board-sprite";
/// A whole screen of the board, 512 by 480.
pub(crate) const SCREEN: &str = "board-screen";
/// A counter's digits or its repeated tile, 16 by 16 a cell.
pub(crate) const COUNTER: &str = "board-counter";

/// Whether a slot is one of a board-drawn game's.
pub(crate) fn is_board(a: &ArtSlot) -> bool {
    matches!(a.kind.as_str(), SPRITE | SCREEN | COUNTER)
}

/// A picture code the model fixes: the picture byte's number with the
/// ninth bit from a fixed flags byte (bit 3), None when a word gives it.
fn fixed_code(picture: &BoardByte, flags: Option<&BoardByte>) -> Option<u16> {
    let p = picture.number?;
    let high = match flags {
        None => 0,
        Some(f) => (f.number? >> 3) & 1,
    };
    Some(((high << 8) | (p & 0xff)) as u16)
}

/// The code a frame name stands for (`4d`).
pub(crate) fn code_of(frame: &str) -> Option<u16> {
    u16::from_str_radix(frame, 16).ok()
}

/// A code's frame name: lowercase hex, two digits at least, as
/// `gp_look::board::sprite_name` writes it.
fn frame_of(code: u16) -> String {
    format!("{code:02x}")
}

/// The board's slots of a model, from the model alone. `id_names` are the
/// names an entity's slot id hides (`theme_kit::names_of`). A sprite's
/// description is its role's words only: a board game's model describes a
/// body by the program's addresses and routines, which no prompt can use,
/// so the words file gives each its own.
pub(crate) fn slots(m: &Model, id_names: &BTreeSet<String>) -> Vec<ArtSlot> {
    let Some(n) = crate::theme_board::model_names(m) else {
        return Vec::new();
    };
    let colors = "any colors: drawn whole over the board's picture";
    let mut out = Vec::new();
    for name in &n.entities {
        let Some(e) = m.entities.get(name) else {
            continue;
        };
        let Some(b) = &e.core.board_sprite else {
            continue;
        };
        let mut codes: Vec<u16> = std::iter::once(fixed_code(&b.picture, b.flags.as_ref()))
            .chain(
                b.extra
                    .iter()
                    .map(|x| fixed_code(&x.picture, x.flags.as_ref())),
            )
            .flatten()
            .filter(|c| *c != 0)
            .collect();
        codes.sort_unstable();
        codes.dedup();
        let description = format!(
            "{}, {} by {} pixels",
            role_words(&e.core.role),
            SPRITE_SIZE[0],
            SPRITE_SIZE[1]
        );
        let frame_names: Vec<String> = codes.into_iter().map(frame_of).collect();
        out.push(ArtSlot {
            id: entity_id(name, &e.core.role, id_names),
            kind: SPRITE.into(),
            role: e.core.role.clone(),
            size: SPRITE_SIZE,
            frames: frame_names.len() as u32,
            frame_names,
            timing: "a picture for each code its sprite entry draws, the code in hex: the codes the model fixes are listed here, and `gp theme kit GAME --rom ROM` adds the codes your romset's recorded plays show (from `gp theme look sheet`) in the local part".into(),
            colors: colors.into(),
            anchor: format!(
                "the top left of the {} by {} picture the board draws for its entry, flipped as the board flips it",
                SPRITE_SIZE[0], SPRITE_SIZE[1]
            ),
            description,
            label: role_label(&e.core.role, false).into(),
            poses: Vec::new(),
            geometry: None,
        });
    }
    for (i, s) in n.screens.iter().enumerate() {
        out.push(ArtSlot {
            id: format!("screen.{s}"),
            kind: SCREEN.into(),
            role: "background".into(),
            size: SCREEN_SIZE,
            frames: 1,
            frame_names: vec!["base".into()],
            timing: "one picture, while a level's look shows this screen".into(),
            colors: colors.into(),
            anchor: "the board's picture's top left".into(),
            description: format!(
                "a whole screen the board draws, {} by {}: the room the play happens in (screen {} of {}), its floors, counters, walls and doors where the game's own are, since the bodies stand and walk on them; no bodies and no score in it",
                SCREEN_SIZE[0],
                SCREEN_SIZE[1],
                i + 1,
                n.screens.len()
            ),
            label: "room".into(),
            poses: Vec::new(),
            geometry: None,
        });
    }
    for c in &n.digits {
        out.push(ArtSlot {
            id: format!("digits.{c}"),
            kind: COUNTER.into(),
            role: "hud".into(),
            size: CELL_SIZE,
            frames: 10,
            frame_names: (0..10).map(|d| d.to_string()).collect(),
            timing: "one glyph a digit, in the counter's cells".into(),
            colors: colors.into(),
            anchor: "a cell of the board's tile layer".into(),
            description: format!(
                "the digits 0 to 9 of a number the game shows, {} by {} each",
                CELL_SIZE[0], CELL_SIZE[1]
            ),
            label: "digits".into(),
            poses: (0..10).map(|d| format!("the digit {d}")).collect(),
            geometry: None,
        });
    }
    for c in &n.tiles {
        out.push(ArtSlot {
            id: format!("tile.{c}"),
            kind: COUNTER.into(),
            role: "hud".into(),
            size: CELL_SIZE,
            frames: 1,
            frame_names: vec!["base".into()],
            timing: "one picture, repeated once for each the counter holds".into(),
            colors: colors.into(),
            anchor: "a cell of the board's tile layer".into(),
            description: format!(
                "a small mark the game repeats to count something, such as a life, {} by {}",
                CELL_SIZE[0], CELL_SIZE[1]
            ),
            label: "counter mark".into(),
            poses: vec!["one mark".into()],
            geometry: None,
        });
    }
    out
}

/// The entity a board sprite slot is: the one whose slot id is the slot's.
fn entity_of<'a>(m: &'a Model, slot: &str) -> Option<&'a str> {
    let ids = crate::theme_kit::names_of(m);
    m.entities.iter().find_map(|(n, e)| {
        (e.core.board_sprite.is_some() && entity_id(n, &e.core.role, &ids) == slot)
            .then_some(n.as_str())
    })
}

/// The name a theme's rules give a board slot's frame: `sprite-ENTITY-CODE`,
/// the screen's slot, `counter-NAME-D` or `counter-NAME`.
pub(crate) fn picture(m: &Model, slot: &ArtSlot, frame: &str) -> Option<String> {
    match slot.kind.as_str() {
        SPRITE => Some(gp_look::board::sprite_name(
            entity_of(m, &slot.id)?,
            code_of(frame)?,
        )),
        SCREEN => slot.id.strip_prefix("screen.").map(str::to_owned),
        COUNTER => {
            if let Some(c) = slot.id.strip_prefix("digits.") {
                Some(gp_look::board::counter_name(c, Some(frame.parse().ok()?)))
            } else {
                slot.id
                    .strip_prefix("tile.")
                    .map(|c| gp_look::board::counter_name(c, None))
            }
        }
        _ => None,
    }
}

/// The kit with the local part's codes added to each board sprite slot,
/// in code order: what the person's romset was seen to show.
pub(crate) fn add_seen(kit: &mut Kit, local: &Local) {
    for a in kit.art.iter_mut().filter(|a| a.kind == SPRITE) {
        let Some(seen) = local.board_frames.get(&a.id) else {
            continue;
        };
        let mut codes: Vec<u16> = a
            .frame_names
            .iter()
            .chain(seen)
            .filter_map(|f| code_of(f))
            .collect();
        codes.sort_unstable();
        codes.dedup();
        a.frame_names = codes.into_iter().map(frame_of).collect();
        a.frames = a.frame_names.len() as u32;
    }
}

/// The local part of a board-drawn game's kit: from the names its recorded
/// plays showed on the person's romset (`seen`, names.toml), each board
/// sprite slot's codes and each code's visible body in the original
/// (`[x0, y0, x1, y1]` in its 32 by 32 picture, as the board draws it
/// unflipped), from the person's own lift.
pub(crate) fn local_of(
    m: &Model,
    kit: &Kit,
    seen: &BTreeMap<String, u64>,
    lifted: &Path,
) -> Result<Local> {
    let bundle = gp_look::bundle(m, lifted).map_err(anyhow::Error::msg)?;
    let spec = gp_interp::compile::compile(m, &bundle.assets).map_err(anyhow::Error::msg)?;
    let art = BoardArt::load(m, &spec, lifted)
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow!("the model names no board art"))?;
    let level = (0..spec.levels.len())
        .find(|&l| art.sprite_rgba(l, 0, 0).is_some())
        .ok_or_else(|| anyhow!("no level of the model is drawn by the board"))?;
    let ids = crate::theme_kit::names_of(m);
    let slot_of: BTreeMap<&str, String> = m
        .entities
        .iter()
        .filter(|(_, e)| e.core.board_sprite.is_some())
        .map(|(n, e)| (n.as_str(), entity_id(n, &e.core.role, &ids)))
        .collect();
    let mut local = Local {
        game: kit.game.clone(),
        ..Local::default()
    };
    for name in seen.keys() {
        let Some((entity, code)) = name
            .strip_prefix("sprite-")
            .and_then(|r| r.rsplit_once('-'))
        else {
            continue;
        };
        let (Some(slot), Some(c)) = (slot_of.get(entity), code_of(code)) else {
            continue;
        };
        local
            .board_frames
            .entry(slot.clone())
            .or_default()
            .push(frame_of(c));
        if let Some(px) = art.sprite_rgba(level, c, 0) {
            let tones: Vec<u8> = px.iter().map(|p| u8::from(p[3] > 0)).collect();
            if let Some(b) = crate::theme_geometry::body_of(&tones, SPRITE_SIZE[0]) {
                local.board_bodies.insert(name.clone(), b);
            }
        }
    }
    for v in local.board_frames.values_mut() {
        v.sort_by_key(|f| code_of(f));
        v.dedup();
    }
    local.board_cycles = cycles_of(m, lifted)?;
    Ok(local)
}

/// One of the board's colour cycles, as the local part lists it: the
/// level whose look names it, where its list is in the lift, and its steps
/// in order.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Cycle {
    pub level: String,
    /// The `rows` slot, row and column the list starts at.
    pub slot: String,
    pub row: u32,
    pub column: u32,
    pub steps: Vec<CycleStep>,
}

/// A step of a colour cycle: the frames it waits, the colour slot it
/// writes and the colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CycleStep {
    pub frames: u32,
    pub slot: u32,
    pub colour: u32,
}

/// The steps of a list from byte `column` of `row`: three bytes a step,
/// up to a zero frames byte or the row's end (a step cut short by the
/// row's end isn't one).
fn steps_of(row: &[u32], column: u32) -> Vec<CycleStep> {
    row.get(column as usize..)
        .unwrap_or_default()
        .as_chunks::<3>()
        .0
        .iter()
        .take_while(|s| s[0] != 0)
        .map(|s| CycleStep {
            frames: s[0],
            slot: s[1],
            colour: s[2],
        })
        .collect()
}

/// Every colour cycle the model's levels' board looks name, in level
/// order, each list once (the first level naming it), read from the lift.
fn cycles_of(m: &Model, lifted: &Path) -> Result<Vec<Cycle>> {
    let mut rows: BTreeMap<&str, Vec<Vec<u32>>> = BTreeMap::new();
    let mut out: Vec<Cycle> = Vec::new();
    for (name, l) in &m.levels {
        let Some(b) = l.core.present.as_ref().and_then(|p| p.board.as_ref()) else {
            continue;
        };
        for c in &b.cycles {
            let s = &c.steps;
            if out
                .iter()
                .any(|o| (o.slot.as_str(), o.row, o.column) == (s.slot.as_str(), s.row, s.column))
            {
                continue;
            }
            if !rows.contains_key(s.slot.as_str()) {
                let r = crate::theme_kit::read_rows(lifted, m, &s.slot)?;
                rows.insert(s.slot.as_str(), r);
            }
            let row = rows[s.slot.as_str()].get(s.row as usize).ok_or_else(|| {
                anyhow!(
                    "levels.{name}.present.board.cycles: `{}` has no row {}",
                    s.slot,
                    s.row
                )
            })?;
            out.push(Cycle {
                level: name.clone(),
                slot: s.slot.clone(),
                row: s.row,
                column: s.column,
                steps: steps_of(row, s.column),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up arcade model: a hero whose picture a word gives, a
    /// coin fixed at code 0x14d (the ninth bit in its fixed flags), two
    /// screens, a score drawn as digits and lives as a tile.
    const MODEL: &str = r#"
[model]
schema = "0.4"
name = "cab"
title = "Cab"
view = "side"
frame_rate = "30"
systems = ["rules"]

[entities.player]
role = "player"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[entities.player.board_sprite]
slot = "sprites"
picture = { word = 1, byte = 1 }

[entities.coin]
role = "collectible"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[entities.coin.board_sprite]
slot = "sprites"
picture = { number = 0x4d }
flags = { number = 8 }

[levels.main]
size = [32, 30]
tile_size = 16

[levels.main.present.board]
palette = "colors"
tiles = "tiles"
screen = "stage"

[[levels.main.present.board.looks]]
when = [{ counter = "bar", cmp = "==", value = 1 }]
screen = "stage2"

[[levels.main.present.board.counters]]
counter = "score"
at = [2, 1]
digits = 4
tile = { slot = "codes" }
high = { slot = "codes", column = 1 }

[[levels.main.present.board.repeats]]
counter = "lives"
at = [8, 2]
most = 3
tile = { slot = "codes", column = 3 }
high = { slot = "codes", column = 1 }
"#;

    #[test]
    fn a_board_drawn_game_has_a_slot_for_each_entity_screen_and_counter() {
        let m = Model::parse(MODEL).unwrap();
        let kit = crate::theme_kit::kit_of(&m, "cab");
        let ids: Vec<(&str, &str, u32)> = kit
            .art
            .iter()
            .map(|a| (a.id.as_str(), a.kind.as_str(), a.frames))
            .collect();
        assert_eq!(
            ids,
            [
                ("entity.coin", SPRITE, 1),
                ("entity.player", SPRITE, 0),
                ("screen.stage", SCREEN, 1),
                ("screen.stage2", SCREEN, 1),
                ("digits.score", COUNTER, 10),
                ("tile.lives", COUNTER, 1),
            ]
        );
        let coin = &kit.art[0];
        assert_eq!(coin.frame_names, ["14d"]);
        assert_eq!(coin.size, [32, 32]);
        assert_eq!(kit.art[2].size, [512, 480]);
        assert_eq!(kit.art[4].size, [16, 16]);
        // Each frame's picture name, as a theme's rules draw it.
        let names: Vec<String> = kit
            .art
            .iter()
            .flat_map(|a| a.frame_names.iter().filter_map(|f| picture(&m, a, f)))
            .collect();
        assert_eq!(
            names
                .iter()
                .filter(|n| n.starts_with("counter-score-"))
                .count(),
            10
        );
        for n in ["sprite-coin-14d", "stage", "stage2", "counter-lives"] {
            assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
        }
        // The board's own names hold every one of them.
        assert!(
            kit.art.iter().all(|a| a.geometry.is_none()),
            "the board's slots have no tile geometry"
        );
        // The slot fingerprint holds them, so a kit with them differs.
        let without = crate::theme_kit::slot_lines([512, 480], &[], &[], false);
        assert_ne!(
            gp_package::index::slots_digest(&without),
            kit.slots,
            "the board's slots are in the fingerprint"
        );
    }

    #[test]
    fn the_local_part_adds_the_codes_the_romset_showed() {
        let m = Model::parse(MODEL).unwrap();
        let mut kit = crate::theme_kit::kit_of(&m, "cab");
        let local = Local {
            board_frames: [(
                "entity.player".to_owned(),
                vec!["4e".to_owned(), "12".to_owned()],
            )]
            .into(),
            ..Local::default()
        };
        add_seen(&mut kit, &local);
        let p = kit.art.iter().find(|a| a.id == "entity.player").unwrap();
        assert_eq!(p.frame_names, ["12", "4e"]);
        assert_eq!(p.frames, 2);
        assert_eq!(picture(&m, p, "4e").as_deref(), Some("sprite-player-4e"));
        // A code the model fixes stays when the plays didn't show it.
        let mk = kit.art.iter().find(|a| a.id == "entity.coin").unwrap();
        assert_eq!(mk.frame_names, ["14d"]);
    }

    #[test]
    fn a_board_sprites_pose_words_go_by_code() {
        let m = Model::parse(MODEL).unwrap();
        let d = tempfile::tempdir().unwrap();
        let kd = d.path().join("themes").join("kit");
        std::fs::create_dir_all(&kd).unwrap();
        let words = |art: &str| {
            std::fs::write(
                kd.join(crate::theme_words::WORDS_FILE),
                format!("format = \"0.1.0\"\ndeny = [\"taxi\"]\n\n[[art]]\nid = \"entity.player\"\nlabel = \"driver\"\n{art}"),
            )
            .unwrap();
        };
        words(
            "[art.frames]\n\"4e\" = \"turning the wheel\"\n\"12\" = \"waving\"\n\"77\" = \"a taxi horn\"\n",
        );
        let local = Local {
            board_frames: [("entity.player".to_owned(), vec!["4e".to_owned()])].into(),
            ..Local::default()
        };
        let (kit, problems) =
            crate::theme_kit::kit_with_local(&m, "cab", d.path(), Some(&local)).unwrap();
        let p = kit.art.iter().find(|a| a.id == "entity.player").unwrap();
        assert_eq!(p.poses, ["turning the wheel"]);
        // A code this romset didn't show keeps to the denied words too.
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("frame 77"), "{problems:?}");
        // Poses in order are refused: the codes are the romset's.
        words("poses = [\"standing\", \"walking\"]\n");
        let (_, problems) =
            crate::theme_kit::kit_with_local(&m, "cab", d.path(), Some(&local)).unwrap();
        assert!(
            problems.iter().any(|p| p.contains("[art.frames]")),
            "{problems:?}"
        );
    }

    /// A made-up arcade lift for the local part: 1024 blank tiles but tile
    /// 1, four sprites of one value each, a screen of tile 1, a palette
    /// whose entry n is n, and `lights`, two rows of colour-cycle steps
    /// (frames to wait, colour slot, colour), each list ending at a zero
    /// byte. No game's bytes.
    fn board_lift(dir: &Path) {
        let map = "schema = 1\nplatform = \"arcade\"\nsource = \"cab\"\nsets = [\"cab\"]\n\n\
                   [[asset]]\nname = \"tiles\"\nformat = \"mcr-tiles\"\nregion = \"gfx1\"\ncount = 1024\nper_row = 32\n\n\
                   [[asset]]\nname = \"sprites\"\nformat = \"mcr-sprites\"\nregion = \"gfx2\"\ncount = 4\nper_row = 4\n\n\
                   [[asset]]\nname = \"stage\"\nformat = \"mcr-screen\"\nregion = \"maincpu\"\noffset = 0\n\n\
                   [[asset]]\nname = \"colors\"\nformat = \"mcr-palette\"\nregion = \"maincpu\"\noffset = 0\n\n\
                   [[asset]]\nname = \"lights\"\nformat = \"byte-rows\"\nregion = \"maincpu\"\noffset = 0\ncount = 2\nwidth = 10\n";
        std::fs::write(dir.join("assets.toml"), map).unwrap();
        let png = |w: u32, h: u32, px: &[u8]| {
            let mut out = Vec::new();
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(png::ColorType::Indexed);
            e.set_depth(png::BitDepth::Eight);
            e.set_palette((0..=255u8).flat_map(|i| [i; 3]).collect::<Vec<_>>());
            let mut wr = e.write_header().unwrap();
            wr.write_image_data(px).unwrap();
            wr.finish().unwrap();
            out
        };
        let (tw, th) = (32 * 8, 32 * 8);
        let mut t = vec![0u8; (tw * th) as usize];
        for y in 0..8 {
            for x in 8..16 {
                t[y * tw as usize + x] = 5;
            }
        }
        std::fs::write(dir.join("tiles.png"), png(tw, th, &t)).unwrap();
        let (sw, sh) = (4 * 32, 32);
        let s: Vec<u8> = (0..sw * sh)
            .map(|i| [5, 3, 3, 2][(i % sw / 32) as usize])
            .collect();
        std::fs::write(dir.join("sprites.png"), png(sw, sh, &s)).unwrap();
        let row = vec!["0x1001"; 32].join(", ");
        let rows = vec![format!("[{row}]"); 30].join(",\n");
        std::fs::write(dir.join("stage.toml"), format!("rows = [{rows}]\n")).unwrap();
        let colors: Vec<String> = (0..64).map(|n| n.to_string()).collect();
        std::fs::write(
            dir.join("colors.toml"),
            format!("colors = [{}]\n", colors.join(", ")),
        )
        .unwrap();
        // Two lists: three steps, then one; the bytes after a list's zero
        // aren't its steps.
        std::fs::write(
            dir.join("lights.toml"),
            "rows = [[8, 33, 63, 4, 33, 7, 12, 34, 28, 0], [16, 48, 21, 0, 9, 9, 9, 0, 0, 0]]\n",
        )
        .unwrap();
    }

    /// The made-up board's model, its level's look naming the colour
    /// cycles (`cycles`) when `cycles` is given.
    fn board_model(cycles: &str) -> String {
        format!(
            r#"
[model]
schema = "0.4"
name = "cab"
title = "Cab"
description = "A made-up arcade game"
source = "cab"
view = "side"
frame_rate = "30"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]
start_level = "main"

[model.extensions]
platformer = "0.4"

[input]
device = "arcade"

[input.actions]
left = "left"

[assets.tiles]
kind = "board-tiles"
role = "graphics"
[assets.tiles.from]
source = "cab"
asset = "tiles"

[assets.sprites]
kind = "board-sprites"
role = "graphics"
[assets.sprites.from]
source = "cab"
asset = "sprites"

[assets.stage]
kind = "board-screen"
role = "layout"
[assets.stage.from]
source = "cab"
asset = "stage"

[assets.colors]
kind = "board-palette"
role = "palettes"
[assets.colors.from]
source = "cab"
asset = "colors"

[assets.lights]
kind = "rows"
role = "palettes"
[assets.lights.from]
source = "cab"
asset = "lights"

[entities.player]
role = "player"
size = [16, 16]
hitbox = {{ x = 0, y = 0, w = 16, h = 16 }}

[entities.player.board_sprite]
slot = "sprites"
picture = {{ number = 2 }}

[levels.main]
description = "A screen"
empty = true
size = [32, 30]
tile_size = 16

[levels.main.present.board]
palette = "colors"
tiles = "tiles"
screen = "stage"
{cycles}
"#
        )
    }

    /// The row theme-kit-board-colour-cycles's measure: the local
    /// part of a board-drawn game's kit lists its board's colour cycles,
    /// each list's steps in order with the frames each waits, the colour
    /// slot it writes and the colour, read up to the list's zero byte, so a
    /// theme's flashes can keep the board's time with no script reading
    /// the ROM (a theme maker's script read three such lists of
    /// a game's space room). The model names them in
    /// `levels.*.present.board.cycles` (feature `level.board.cycles`).
    #[test]
    fn the_local_part_lists_the_boards_colour_cycles() {
        let d = tempfile::tempdir().unwrap();
        board_lift(d.path());
        // The made-up board's local part builds with no cycles named, and
        // lists none.
        let plain = Model::parse(&board_model("")).unwrap();
        let kit = crate::theme_kit::kit_of(&plain, "cab");
        let local = local_of(&plain, &kit, &BTreeMap::new(), d.path()).unwrap();
        let json = serde_json::to_value(&local).unwrap();
        assert!(
            json.get("board_cycles")
                .is_none_or(|c| c.as_array().is_some_and(Vec::is_empty)),
            "{json}"
        );
        // Its look names two lists of steps, each a row of `lights`.
        let m = Model::parse(&board_model(
            r#"cycles = [{ steps = { slot = "lights", row = 0 } }, { steps = { slot = "lights", row = 1 } }]"#,
        ))
        .unwrap_or_else(|e| panic!("the model can't name a board's colour cycles: {e}"));
        let kit = crate::theme_kit::kit_of(&m, "cab");
        let local = local_of(&m, &kit, &BTreeMap::new(), d.path()).unwrap();
        let json = serde_json::to_value(&local).unwrap();
        let lists = json
            .get("board_cycles")
            .and_then(|c| c.as_array())
            .unwrap_or_else(|| panic!("the local part lists no board_cycles: {json}"));
        let steps: Vec<Vec<[u64; 3]>> = lists
            .iter()
            .map(|l| {
                l["steps"]
                    .as_array()
                    .unwrap_or_else(|| panic!("a cycle with no steps: {l}"))
                    .iter()
                    .map(|s| {
                        ["frames", "slot", "colour"]
                            .map(|k| s[k].as_u64().unwrap_or_else(|| panic!("a step's {k}: {s}")))
                    })
                    .collect()
            })
            .collect();
        assert_eq!(
            steps,
            [
                vec![[8, 33, 63], [4, 33, 7], [12, 34, 28]],
                vec![[16, 48, 21]],
            ],
            "{json}"
        );
    }
}
