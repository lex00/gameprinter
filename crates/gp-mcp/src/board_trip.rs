//! The external-editor round trip for an arcade game's board sprites
//! (docs/sprite-editing.md, "Arcade boards"): the NES trip's export and
//! import (crate::editor_trip, crate::exchange) for a copy whose lift is an
//! arcade one (gp_assets::arcade). An MCR sprite is a 32 by 32 picture of
//! 4-bit values in the board's `mcr-sprites` sheet, so it is written whole,
//! one PNG for each picture code an entity's `board_sprite` draws, in the
//! board's colours, and read back with the same checks: its size, values 0
//! to 15 (or the exported colours), and nothing else changed.
//!
//! The trip ends where the NES's turns into a proposal: the art history
//! (crate::edit_log) keeps a picture as rows of the digits 0 to 9
//! (crate::sprite_ops::Pic::rows) and a slot's state from an NES asset map
//! (crate::catalog), and a board sprite has 16 values and an arcade map. A
//! save is checked and reported, and nothing is written over the lift.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use gp_assets::arcade::{ArcadeAsset, ArcadeMap, Format};
use gp_model::Model;
use gp_model::model::AssetKind;
use serde_json::{Value, json};

use crate::editor_trip::{TRIP_FILE, TRIP_FORMAT, Trip};
use crate::{Config, Reply};

/// A board sprite's side in pixels.
const SIDE: u32 = 32;

/// The frame id prefix of a board sprite in a trip: `board-sprite.4d`.
pub const FRAME_PREFIX: &str = "board-sprite.";

/// What the trip needs that the art history can't hold yet, said once.
pub const NO_PROPOSAL: &str = "a proposal can't be kept for an arcade board's sprites yet: the art history (asset-edits.jsonl and asset-proposals/) holds a picture as rows of the digits 0 to 9 and a slot's state from an NES asset map, and a board sprite has 16 values and an arcade map";

/// The arcade lift a copy runs on, when it is one: its folder and map.
pub fn arcade_lift(
    config: &Config,
    dir: &Path,
    model: &Model,
) -> Result<Option<(PathBuf, ArcadeMap)>, String> {
    let Some(at) = config.assets_of(dir, model.model.source.as_deref()) else {
        return Ok(None);
    };
    let Ok(text) = fs::read_to_string(at.join(gp_assets::MAP_FILE)) else {
        return Ok(None);
    };
    if !gp_assets::arcade::is_arcade(&text) {
        return Ok(None);
    }
    let map = ArcadeMap::parse(&text).map_err(|e| format!("{}: {e}", at.display()))?;
    Ok(Some((at, map)))
}

/// Whether a trip is a board sprite trip.
pub fn is_board_trip(trip: &Trip) -> bool {
    trip.frames.values().any(|id| id.starts_with(FRAME_PREFIX))
}

/// The arcade asset a slot of `kind` names.
fn slot_asset<'a>(
    model: &Model,
    map: &'a ArcadeMap,
    slot: &str,
    kind: AssetKind,
) -> Result<&'a ArcadeAsset, String> {
    let s = model
        .assets
        .get(slot)
        .ok_or(format!("the model has no asset slot `{slot}`"))?;
    if s.kind != kind {
        return Err(format!("slot `{slot}` isn't a {} slot", kind.name()));
    }
    let from = s
        .from
        .as_ref()
        .ok_or(format!("slot `{slot}` has no `from`"))?;
    map.assets
        .iter()
        .find(|a| a.name == from.asset && from.source == map.source)
        .ok_or(format!(
            "slot `{slot}` names `{}`, which the arcade map {} doesn't have",
            from.asset, map.source
        ))
}

/// A board colour of 9 bits (red 6 to 8, blue 3 to 5, green 0 to 2) as
/// RGB, each 3 bits spread over 0 to 255.
pub fn rgb(c: u16) -> [u8; 3] {
    let three = |v: u16| ((v & 7) * 255 / 7) as u8;
    [three(c >> 6), three(c), three(c >> 3)]
}

/// The 16 colours a sprite of colour bits `bits` is drawn in: palette
/// entries `16 * (3 - bits)` on (gp_assets::arcade), from the level's
/// board palette as lifted (the program's writes over it aren't applied).
fn colours(
    model: &Model,
    map: &ArcadeMap,
    lifted: &Path,
    bits: u8,
) -> Result<[[u8; 3]; 16], String> {
    let palette = model
        .levels
        .values()
        .find_map(|l| l.core.present.as_ref()?.board.as_ref())
        .map(|b| b.palette.clone())
        .ok_or("no level of the model has present.board, so its sprites have no colours")?;
    let a = slot_asset(model, map, &palette, AssetKind::BoardPalette)?;
    let text = fs::read_to_string(lifted.join(a.file()))
        .map_err(|e| format!("reading {}: {e}", a.file()))?;
    let all = gp_assets::arcade::read_palette(&text).map_err(|e| format!("{}: {e}", a.file()))?;
    let base = 16 * (3 - usize::from(bits & 3));
    Ok(std::array::from_fn(|i| rgb(all[base + i])))
}

/// The picture codes an entity's board sprite draws: a fixed `number`, or
/// those `gp theme look sheet` saw it show (its names.toml under
/// `$GP_HOME/work/theme-kits/MODEL/board`).
fn codes(config: &Config, model: &Model, entity: &str) -> Result<Vec<u32>, String> {
    let e = model
        .entities
        .get(entity)
        .ok_or(format!("the model has no entity `{entity}`"))?;
    let b = e.core.board_sprite.as_ref().ok_or(format!(
        "`{entity}` has no board_sprite, so the board draws none of its pictures"
    ))?;
    // A fixed picture; its ninth bit is bit 3 of the flags, when they
    // are fixed too.
    if let Some(p) = b.picture.number {
        let flags = b.flags.as_ref().and_then(|f| f.number).unwrap_or(0);
        return Ok(vec![p | ((flags >> 3) & 1) << 8]);
    }
    let seen = config
        .home
        .join("work")
        .join("theme-kits")
        .join(&model.model.name)
        .join("board")
        .join("names.toml");
    let t: toml::Table = fs::read_to_string(&seen)
        .ok()
        .and_then(|t| t.parse().ok())
        .ok_or(format!(
            "`{entity}`'s picture is read from its words, so its codes are the ones the game shows: run gp theme look sheet --game {} first, which writes them to {}",
            model.model.name,
            seen.display()
        ))?;
    let prefix = format!("sprite-{entity}-");
    let out: Vec<u32> = t
        .get("seen")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|s| s.keys())
        .filter_map(|k| u32::from_str_radix(k.strip_prefix(&prefix)?, 16).ok())
        .collect();
    if out.is_empty() {
        return Err(format!(
            "{} names no picture of `{entity}`: the recorded inputs never showed it",
            seen.display()
        ));
    }
    Ok(out)
}

/// The board sprites sheet as lifted: each sprite's 1024 values by code.
fn read_sheet(lifted: &Path, a: &ArcadeAsset) -> Result<Vec<Vec<u8>>, String> {
    let bytes =
        fs::read(lifted.join(a.file())).map_err(|e| format!("reading {}: {e}", a.file()))?;
    let (w, _) = a.image_size();
    let px = gp_assets::chr::read_png(&bytes, a.image_size())
        .map_err(|e| format!("{}: {e}", a.file()))?;
    Ok((0..a.count)
        .map(|k| {
            let (cx, cy) = ((k % a.per_row) * SIDE, (k / a.per_row) * SIDE);
            (0..SIDE * SIDE)
                .map(|i| px[((cy + i / SIDE) * w + cx + i % SIDE) as usize])
                .collect()
        })
        .collect())
}

/// An indexed PNG of one sprite in its 16 colours, values 0 and 8
/// transparent (the board draws nothing where `value & 7` is 0).
pub fn write_sprite(px: &[u8], colours: &[[u8; 3]; 16]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, SIDE, SIDE);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(colours.concat());
    let mut trns = vec![255u8; 16];
    trns[0] = 0;
    trns[8] = 0;
    enc.set_trns(trns);
    let mut wr = enc.write_header().expect("PNG header into memory");
    wr.write_image_data(px).expect("PNG data into memory");
    wr.finish().expect("PNG end into memory");
    out
}

/// A GIMP palette of the 16 colours.
fn gpl(name: &str, colours: &[[u8; 3]; 16]) -> String {
    let mut s = format!("GIMP Palette\nName: {name}\nColumns: 8\n#\n");
    for (i, c) in colours.iter().enumerate() {
        let label = if i & 7 == 0 {
            format!("value {i} (transparent)")
        } else {
            format!("value {i}")
        };
        s.push_str(&format!("{:3} {:3} {:3}\t{label}\n", c[0], c[1], c[2]));
    }
    s
}

/// Whether a folder is inside a git work tree.
fn in_repo(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(std::path::absolute(dir).unwrap_or_else(|_| dir.to_owned()));
    while let Some(p) = d {
        if p.join(".git").exists() {
            return Some(p);
        }
        d = p.parent().map(Path::to_path_buf);
    }
    None
}

/// Writes an entity's board sprites for an editor into `out`: one PNG a
/// picture code, `ENTITY-CODE.png`, its palette file and the trip file.
pub fn export(
    config: &Config,
    copy: &str,
    entity: &str,
    editor: &str,
    out: &Path,
) -> Result<(Trip, Vec<PathBuf>), String> {
    let dir = config.existing_copy(copy)?;
    let model = Model::parse(
        &fs::read_to_string(dir.join(gp_model::MODEL_FILE))
            .map_err(|e| format!("reading {}: {e}", gp_model::MODEL_FILE))?,
    )
    .map_err(|e| format!("{}: {e}", gp_model::MODEL_FILE))?;
    let (lifted, map) =
        arcade_lift(config, &dir, &model)?.ok_or("the copy's lift isn't an arcade one")?;
    if let Some(repo) = in_repo(out) {
        return Err(format!(
            "{} is inside the repository {}: the pictures are your ROM's, which never go in a repo; name a folder outside it",
            out.display(),
            repo.display()
        ));
    }
    let b = model
        .entities
        .get(entity)
        .and_then(|e| e.core.board_sprite.clone())
        .ok_or(format!("`{entity}` has no board_sprite"))?;
    let a = slot_asset(&model, &map, &b.slot, AssetKind::BoardSprites)?;
    if a.format != Format::McrSprites {
        return Err(format!("`{}` isn't an mcr-sprites asset", a.name));
    }
    let sheet = read_sheet(&lifted, a)?;
    let bits = b.flags.as_ref().and_then(|f| f.number).unwrap_or(0) as u8;
    let colours = colours(&model, &map, &lifted, bits)?;
    fs::create_dir_all(out).map_err(|e| format!("making {}: {e}", out.display()))?;
    let mut frames = BTreeMap::new();
    let mut images = Vec::new();
    let mut written = Vec::new();
    for code in codes(config, &model, entity)? {
        let Some(k) = code.checked_sub(a.first).filter(|k| *k < a.count) else {
            continue;
        };
        let name = format!("{entity}-{code:02x}");
        let file = format!("{name}.png");
        let p = out.join(&file);
        fs::write(&p, write_sprite(&sheet[k as usize], &colours))
            .map_err(|e| format!("writing {}: {e}", p.display()))?;
        frames.insert(name, format!("{FRAME_PREFIX}{code:02x}"));
        images.push(file);
        written.push(p);
    }
    if images.is_empty() {
        return Err(format!(
            "none of `{entity}`'s picture codes is in `{}`",
            a.name
        ));
    }
    let pal = out.join(format!("{entity}.gpl"));
    fs::write(
        &pal,
        gpl(&format!("{} {entity}", model.model.name), &colours),
    )
    .map_err(|e| format!("writing {}: {e}", pal.display()))?;
    written.push(pal);
    let trip = Trip {
        format: TRIP_FORMAT.into(),
        copy: copy.to_owned(),
        entity: entity.to_owned(),
        slot: b.slot.clone(),
        editor: editor.to_owned(),
        frames,
        images,
    };
    let p = out.join(TRIP_FILE);
    fs::write(
        &p,
        serde_json::to_string_pretty(&trip).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("writing {}: {e}", p.display()))?;
    written.push(p);
    Ok((trip, written))
}

/// Reads a saved sprite back: an indexed PNG's values, or an RGB(A) one's
/// colours matched exactly to the exported 16 (alpha under 128 is 0).
pub fn read_sprite(bytes: &[u8], colours: &[[u8; 3]; 16]) -> Result<Vec<u8>, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    let indexed = dec
        .read_header_info()
        .map_err(|e| format!("not a PNG ({e})"))?
        .color_type
        == png::ColorType::Indexed;
    let px = if indexed {
        gp_assets::chr::read_png(bytes, (SIDE, SIDE))?
    } else {
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut r = dec.read_info().map_err(|e| e.to_string())?;
        let (w, h) = (r.info().width, r.info().height);
        if (w, h) != (SIDE, SIDE) {
            return Err(format!("{w}x{h}; a board sprite is {SIDE}x{SIDE}"));
        }
        let mut buf = vec![0u8; r.output_buffer_size().ok_or("image too large")?];
        let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let step = info.color_type.samples();
        if step < 3 {
            return Err("a gray image; save it indexed or in colour".into());
        }
        let mut px = Vec::new();
        for (i, c) in buf[..info.buffer_size()].chunks(step).enumerate() {
            let rgb = [c[0], c[1], c[2]];
            let v = if step == 4 && c[3] < 128 {
                Some(0)
            } else {
                colours.iter().position(|k| *k == rgb).map(|v| v as u8)
            };
            px.push(v.ok_or(format!(
                "pixel ({}, {}) #{:02x}{:02x}{:02x} is none of the exported colours",
                i as u32 % SIDE,
                i as u32 / SIDE,
                rgb[0],
                rgb[1],
                rgb[2]
            ))?);
        }
        px
    };
    if px.len() != (SIDE * SIDE) as usize {
        return Err(format!("a board sprite is {SIDE}x{SIDE}"));
    }
    if let Some(i) = px.iter().position(|v| *v > 15) {
        return Err(format!(
            "pixel ({}, {}) is {}; a board sprite's values are 0 to 15",
            i as u32 % SIDE,
            i as u32 / SIDE,
            px[i]
        ));
    }
    Ok(px)
}

/// Each saved picture's values by frame id, with how many pixels differ
/// from the lift.
pub type Saved = BTreeMap<String, (Vec<u8>, usize)>;

/// A save brought back: each exported picture read back with the checks
/// and compared with the lift. Ok(Err(problems)) when one fails.
pub fn import(config: &Config, dir: &Path) -> Result<Result<Saved, Vec<String>>, String> {
    let trip = crate::editor_trip::read_trip(dir)?;
    let copy = config.existing_copy(&trip.copy)?;
    let model = Model::parse(
        &fs::read_to_string(copy.join(gp_model::MODEL_FILE))
            .map_err(|e| format!("reading {}: {e}", gp_model::MODEL_FILE))?,
    )
    .map_err(|e| format!("{}: {e}", gp_model::MODEL_FILE))?;
    let (lifted, map) =
        arcade_lift(config, &copy, &model)?.ok_or("the copy's lift isn't an arcade one")?;
    let b = model
        .entities
        .get(&trip.entity)
        .and_then(|e| e.core.board_sprite.clone())
        .ok_or(format!("`{}` has no board_sprite", trip.entity))?;
    let a = slot_asset(&model, &map, &trip.slot, AssetKind::BoardSprites)?;
    let sheet = read_sheet(&lifted, a)?;
    let bits = b.flags.as_ref().and_then(|f| f.number).unwrap_or(0) as u8;
    let colours = colours(&model, &map, &lifted, bits)?;
    let mut problems = Vec::new();
    let mut out = BTreeMap::new();
    for (name, id) in &trip.frames {
        let Some(code) = id
            .strip_prefix(FRAME_PREFIX)
            .and_then(|c| u32::from_str_radix(c, 16).ok())
        else {
            problems.push(format!("{name}: frame {id} isn't a board sprite"));
            continue;
        };
        let file = dir.join(format!("{name}.png"));
        let got = fs::read(&file)
            .map_err(|e| e.to_string())
            .and_then(|b| read_sprite(&b, &colours));
        match got {
            Err(e) => problems.push(format!("{name}: {e}")),
            Ok(px) => {
                let was = &sheet[(code - a.first) as usize];
                let changed = px.iter().zip(was).filter(|(a, b)| a != b).count();
                out.insert(id.clone(), (px, changed));
            }
        }
    }
    Ok(if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    })
}

/// A save checked and reported, as the NES trip's `propose_save` replies:
/// refused at the import stage when a check fails, and otherwise at the
/// proposal stage with what changed, since the history can't keep it.
pub fn propose_save(config: &Config, dir: &Path) -> Result<Reply, String> {
    match import(config, dir)? {
        Err(problems) => Ok(Reply::refused(json!({
            "ok": false, "written": false, "stage": "import", "problems": problems,
        }))),
        Ok(pics) => {
            let changed: Value = pics
                .iter()
                .filter(|(_, (_, n))| *n > 0)
                .map(|(id, (_, n))| (id.clone(), json!(n)))
                .collect::<serde_json::Map<_, _>>()
                .into();
            let pixels: usize = pics.values().map(|(_, n)| n).sum();
            Ok(Reply::refused(json!({
                "ok": false,
                "written": false,
                "stage": "proposal",
                "checked": pics.len(),
                "changed": changed,
                "pixels_changed": pixels,
                "error": NO_PROPOSAL,
            })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up arcade model: a body drawn as board sprite 1 in colour
    /// bits 2, the board's palette and sprites. No game's bytes.
    const MODEL: &str = r#"
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

[assets.sprites]
kind = "board-sprites"
role = "graphics"
[assets.sprites.from]
source = "cab"
asset = "sprites"

[assets.colors]
kind = "board-palette"
role = "palettes"
[assets.colors.from]
source = "cab"
asset = "colors"

[entities.player]
role = "player"
size = [16, 16]

[entities.player.hitbox]
x = 0
y = 0
w = 16
h = 16

[entities.player.board_sprite]
slot = "sprites"
picture = { number = 1 }
flags = { number = 2 }

[levels.main]
description = "A screen"
empty = true
size = [32, 30]
tile_size = 16

[levels.main.present.board]
palette = "colors"

[[levels.main.spawn]]
entity = "player"
x = 53
y = 191
"#;

    const MAP: &str = "schema = 1\nplatform = \"arcade\"\nsource = \"cab\"\nsets = [\"cab\"]\n\n\
        [[asset]]\nname = \"sprites\"\nformat = \"mcr-sprites\"\nregion = \"gfx2\"\nfirst = 0\ncount = 2\nper_row = 2\n\n\
        [[asset]]\nname = \"colors\"\nformat = \"mcr-palette\"\nregion = \"maincpu\"\noffset = 0\n";

    /// An 8-bit indexed PNG of values, for the made-up lift.
    fn indexed(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Indexed);
        e.set_depth(png::BitDepth::Eight);
        e.set_palette(vec![0u8; 16 * 3]);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(px).unwrap();
        wr.finish().unwrap();
        out
    }

    /// A copy of the made-up game on a made-up arcade lift: the export
    /// writes sprite 1 in the colours of bits 2 (palette entries 16 on)
    /// and no more, a save that changes three pixels comes back checked
    /// with those three counted and is reported at the proposal stage
    /// with nothing written over the lift, a save with a value past 15 is
    /// refused at the import stage, and a folder in a repo is refused.
    #[test]
    fn an_arcade_copy_exports_its_board_sprites_and_reads_a_save_back() {
        let home = tempfile::tempdir().unwrap();
        let work = home.path().join("work").join("mcp");
        let copy = work.join("cab");
        fs::create_dir_all(&copy).unwrap();
        fs::write(copy.join(gp_model::MODEL_FILE), MODEL).unwrap();
        let lifted = home.path().join("lift");
        fs::create_dir_all(&lifted).unwrap();
        fs::write(lifted.join(gp_assets::MAP_FILE), MAP).unwrap();
        let sheet: Vec<u8> = (0..64 * 32u32)
            .map(|i| ((i % 64) / 32 * 5 + i % 3) as u8)
            .collect();
        fs::write(lifted.join("sprites.png"), indexed(64, 32, &sheet)).unwrap();
        let colors: Vec<String> = (0..64).map(|n| (n * 8).to_string()).collect();
        fs::write(
            lifted.join("colors.toml"),
            format!("colors = [{}]\n", colors.join(", ")),
        )
        .unwrap();
        let config = Config {
            gp: PathBuf::from("gp"),
            repo: home.path().to_owned(),
            work,
            home: home.path().to_owned(),
            rom: None,
            assets: Some(lifted.clone()),
            games: None,
        };
        let out = home.path().join("out");
        let (trip, files) = export(&config, "cab", "player", "png", &out).unwrap();
        assert!(is_board_trip(&trip));
        assert_eq!(trip.images, ["player-01.png"]);
        assert_eq!(files.len(), 3);
        let c = super::colours(
            &Model::parse(MODEL).unwrap(),
            &ArcadeMap::parse(MAP).unwrap(),
            &lifted,
            2,
        )
        .unwrap();
        assert_eq!(c[1], rgb(17 * 8));
        let png = fs::read(out.join("player-01.png")).unwrap();
        let mut px = read_sprite(&png, &c).unwrap();
        assert_eq!(px[0], sheet[32]);
        // Nothing changed: checked, nothing to keep.
        let r = propose_save(&config, &out).unwrap();
        assert_eq!(r.json["stage"], "proposal");
        assert_eq!(r.json["pixels_changed"], 0);
        for i in [0, 40, 1023] {
            px[i] = 15 - px[i];
        }
        fs::write(out.join("player-01.png"), write_sprite(&px, &c)).unwrap();
        let r = propose_save(&config, &out).unwrap();
        assert!(r.is_error);
        assert_eq!(r.json["stage"], "proposal");
        assert_eq!(r.json["changed"]["board-sprite.01"], 3);
        assert_eq!(r.json["written"], false);
        assert_eq!(
            fs::read(lifted.join("sprites.png")).unwrap(),
            indexed(64, 32, &sheet)
        );
        px[5] = 16;
        fs::write(out.join("player-01.png"), indexed(32, 32, &px)).unwrap();
        let r = propose_save(&config, &out).unwrap();
        assert_eq!(r.json["stage"], "import");
        assert!(
            r.json["problems"][0].as_str().unwrap().contains("past")
                || r.json["problems"][0].as_str().unwrap().contains("0 to 15")
        );
        fs::create_dir_all(home.path().join(".git")).unwrap();
        let e = export(
            &config,
            "cab",
            "player",
            "png",
            &home.path().join("in-repo"),
        )
        .unwrap_err();
        assert!(e.contains("never go in a repo"), "{e}");
    }

    fn made_up_colours() -> [[u8; 3]; 16] {
        std::array::from_fn(|i| rgb((i as u16) * 33))
    }

    /// A made-up sprite written for an editor reads back to the same
    /// values, as an indexed PNG or as the editor's RGBA; a colour off the
    /// palette and a wrong size are refused.
    #[test]
    fn a_board_sprite_reads_back_from_an_editor() {
        let c = made_up_colours();
        let px: Vec<u8> = (0..1024u32).map(|i| ((i * 7) % 16) as u8).collect();
        assert_eq!(read_sprite(&write_sprite(&px, &c), &c).unwrap(), px);
        let mut rgba = Vec::new();
        for &v in &px {
            let k = c[v as usize];
            rgba.extend_from_slice(&[k[0], k[1], k[2], if v == 0 { 0 } else { 255 }]);
        }
        let png_of = |w: u32, h: u32, data: &[u8]| {
            let mut out = Vec::new();
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            let mut wr = e.write_header().unwrap();
            wr.write_image_data(data).unwrap();
            wr.finish().unwrap();
            out
        };
        assert_eq!(read_sprite(&png_of(32, 32, &rgba), &c).unwrap(), px);
        let mut off = rgba.clone();
        off[4..8].copy_from_slice(&[1, 2, 3, 255]);
        let e = read_sprite(&png_of(32, 32, &off), &c).unwrap_err();
        assert!(e.contains("(1, 0) #010203"), "{e}");
        assert!(read_sprite(&png_of(16, 64, &rgba), &c).is_err());
        // Nine bits of colour spread to eight a channel: red 6 to 8, blue
        // 3 to 5, green 0 to 2.
        assert_eq!(rgb(0x1C0), [255, 0, 0]);
        assert_eq!(rgb(0x038), [0, 0, 255]);
        assert_eq!(rgb(0x007), [0, 255, 0]);
        assert!(gpl("t", &c).contains("value 8 (transparent)"));
    }
}
