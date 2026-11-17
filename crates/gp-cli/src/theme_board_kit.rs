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
    Ok(local)
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
}
