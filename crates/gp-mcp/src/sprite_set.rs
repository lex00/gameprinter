//! An entity's sprite set: every picture the model draws it with, as frames
//! laid out from tiles of one sheet (docs/sprite-editing.md). Each frame is
//! a list of pieces, a tile placed in the picture with its own flips, as a
//! console's sprite attribute bytes give it:
//!
//! | Frame id | What it is |
//! |---|---|
//! | `base` | The sprite's own frame: its tiles in rows of `width` (with `cells`' flips and places), or the frame of a sprite-set it names (`frame = { slot, row }`) |
//! | `clip<c>.<f>` | Frame `f` of clip `c` with tiles or a set row of its own |
//! | `state.<name>`, `state.<name>.clip<c>.<f>` | The sprite a machine state sets, and its clip frames |
//! | `set.<n>` | A frame of the sprite-set asset made for this entity that the model's sprite doesn't name |
//!
//! Frames with the same pieces are one picture, kept under the first id. A
//! frame whose pieces are in another tiles slot, or a row of a `rows` slot
//! that isn't a sprite-set, is listed as skipped with why. The model and
//! the sprite-set are read as they are: nothing here knows a game.

use std::path::Path;

use gp_assets::sprite_set::SpriteSet;
use gp_model::Model;
use gp_model::model::{AssetKind, Cell, Sprite};
use serde_json::{Value, json};

use crate::sprite_ops::Pic;

/// One tile of a frame, from the frame's top left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub tile: u32,
    pub x: u32,
    pub y: u32,
    pub flip_x: bool,
    pub flip_y: bool,
}

/// One picture of a sprite set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// `base`, `clip0.1`, `state.dead`, `set.12`.
    pub id: String,
    pub pieces: Vec<Piece>,
    pub w: u32,
    pub h: u32,
    /// The clips it's a frame of (`clip0`, `state.dead.clip0`).
    pub clips: Vec<String>,
    /// Other ids that are the same picture (`clip0.0` when it's `base`).
    pub aliases: Vec<String>,
    /// The machine state that draws it, or None for the entity's own.
    pub state: Option<String>,
    /// Its number in the sprite-set asset, when it's from one.
    pub set_index: Option<u32>,
}

impl Frame {
    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// Its tiles, in piece order, each once.
    pub fn tiles(&self) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        for p in &self.pieces {
            if !out.contains(&p.tile) {
                out.push(p.tile);
            }
        }
        out
    }

    /// Whether it's a grid of unflipped tiles in rows of `w / 8`, in order:
    /// a picture the tiles-and-width tools (critic, copy) can take as is.
    pub fn is_grid(&self) -> bool {
        let cols = self.w / 8;
        self.w.is_multiple_of(8)
            && self.h.is_multiple_of(8)
            && self.pieces.len() as u32 == cols * (self.h / 8)
            && self.pieces.iter().enumerate().all(|(k, p)| {
                !p.flip_x
                    && !p.flip_y
                    && p.x == (k as u32 % cols) * 8
                    && p.y == (k as u32 / cols) * 8
            })
    }

    /// A grid frame of `tiles` in rows of `width`.
    pub fn grid(id: &str, tiles: &[u32], width: u32, cells: &[Cell]) -> Frame {
        let w = width.max(1);
        let pieces: Vec<Piece> = tiles
            .iter()
            .enumerate()
            .map(|(k, &t)| {
                let c = cells.get(k);
                let [cx, cy] = c.and_then(|c| c.at).unwrap_or([k as u32 % w, k as u32 / w]);
                Piece {
                    tile: t,
                    x: cx * 8,
                    y: cy * 8,
                    flip_x: c.is_some_and(|c| c.flip_x),
                    flip_y: c.is_some_and(|c| c.flip_y),
                }
            })
            .collect();
        let (pw, ph) = pieces
            .iter()
            .fold((0, 0), |(a, b), p| (a.max(p.x + 8), b.max(p.y + 8)));
        Frame {
            id: id.to_owned(),
            pieces,
            w: pw.max(8),
            h: ph.max(8),
            clips: Vec::new(),
            aliases: Vec::new(),
            state: None,
            set_index: None,
        }
    }
}

/// The sprite a state draws: the entity's, with the state's `sprite`
/// table merged in key by key (docs/model.md, machine states' `set`).
pub fn state_sprite(base: &Sprite, set: &toml::Value) -> Result<Sprite, String> {
    let mut v = toml::Value::try_from(base).map_err(|e| e.to_string())?;
    if let (Some(t), Some(over)) = (v.as_table_mut(), set.as_table()) {
        for (k, x) in over {
            t.insert(k.clone(), x.clone());
        }
    }
    v.try_into().map_err(|e: toml::de::Error| e.to_string())
}

/// A sprite-set frame as a frame of tiles in `slot`, or why it isn't one.
fn set_frame(set: &SpriteSet, n: u32, slot: &str, id: &str) -> Result<Frame, String> {
    let f = set
        .frames
        .get(n as usize)
        .ok_or(format!("the sprite set has no frame {n}"))?;
    if f.pieces.is_empty() {
        return Err("draws nothing".into());
    }
    if let Some(p) = f
        .pieces
        .iter()
        .find(|p| p.slot.as_deref().is_some_and(|s| s != slot))
    {
        return Err(format!(
            "its tiles are in slot `{}`, not `{slot}`",
            p.slot.as_deref().unwrap_or("")
        ));
    }
    let [l, t, r, b] = f.bounds();
    Ok(Frame {
        id: id.to_owned(),
        pieces: f
            .pieces
            .iter()
            .map(|p| Piece {
                tile: p.tile,
                x: (p.x - l) as u32,
                y: (p.y - t) as u32,
                flip_x: p.flip_x,
                flip_y: p.flip_y,
            })
            .collect(),
        w: (r - l) as u32,
        h: (b - t) as u32,
        clips: Vec::new(),
        aliases: Vec::new(),
        state: None,
        set_index: Some(n),
    })
}

struct Ctx<'a> {
    model: &'a Model,
    slot: &'a str,
    set: Option<&'a SpriteSet>,
    out: Vec<Frame>,
    skipped: Vec<Value>,
}

impl Ctx<'_> {
    /// Whether a sprite's frame names a sprite-set.
    fn in_set(&self, sp: &Sprite) -> bool {
        sp.frame.as_ref().is_some_and(|f| {
            self.model
                .assets
                .get(&f.slot)
                .is_some_and(|s| s.kind == AssetKind::SpriteSet)
        })
    }

    fn add(&mut self, r: Result<Frame, String>, id: &str, clip: Option<&str>, state: Option<&str>) {
        match r {
            Ok(mut f) => {
                f.id = id.to_owned();
                f.clips = clip.map(str::to_owned).into_iter().collect();
                f.state = state.map(str::to_owned);
                self.out.push(f);
            }
            Err(why) => self.skipped.push(json!({ "frame": id, "why": why })),
        }
    }

    fn row(&self, n: u32, id: &str) -> Result<Frame, String> {
        match self.set {
            Some(set) => set_frame(set, n, self.slot, id),
            None => {
                Err("names a sprite-set frame, and the copy's assets have no sprite set".into())
            }
        }
    }

    fn sprite(&mut self, sp: &Sprite, prefix: &str, state: Option<&str>) {
        let in_set = self.in_set(sp);
        if !in_set && sp.slot != self.slot {
            return;
        }
        let own = if prefix.is_empty() {
            "base".to_owned()
        } else {
            prefix.to_owned()
        };
        let r = match (&sp.frame, in_set) {
            (Some(f), true) => self.row(f.row, &own),
            (Some(_), false) => {
                Err("names a row of a rows slot, not tiles or a sprite-set frame".into())
            }
            (None, _) if sp.tiles.is_empty() => Err("has no tiles".into()),
            (None, _) => Ok(Frame::grid(&own, &sp.tiles, sp.width, &sp.cells)),
        };
        self.add(r, &own, None, state);
        for (c, clip) in sp.clips.iter().enumerate() {
            let cname = if prefix.is_empty() {
                format!("clip{c}")
            } else {
                format!("{prefix}.clip{c}")
            };
            for (f, fr) in clip.frames.iter().enumerate() {
                let id = format!("{cname}.{f}");
                let r = match (fr.row, in_set) {
                    (Some(n), true) => self.row(n, &id),
                    (Some(_), false) => {
                        Err("names a row of a rows slot, not tiles or a sprite-set frame".into())
                    }
                    (None, _) if fr.tiles.is_empty() => continue,
                    (None, _) => {
                        let cells = if fr.cells.is_empty() {
                            &sp.cells
                        } else {
                            &fr.cells
                        };
                        Ok(Frame::grid(&id, &fr.tiles, sp.width, cells))
                    }
                };
                self.add(r, &id, Some(&cname), state);
            }
        }
    }
}

/// Every frame of an entity's sprite set in `slot`, each picture once, and
/// the frames left out with why. `set` is the copy's sprite-set asset, for
/// sprites that name its frames.
pub fn frames(
    model: &Model,
    entity: &str,
    slot: &str,
    set: Option<&SpriteSet>,
) -> Result<(Vec<Frame>, Vec<Value>), String> {
    let e = model
        .entities
        .get(entity)
        .ok_or(format!("the model has no entity `{entity}`"))?;
    let sp = e
        .core
        .sprite
        .as_ref()
        .ok_or(format!("`{entity}` has no sprite"))?;
    let mut cx = Ctx {
        model,
        slot,
        set,
        out: Vec::new(),
        skipped: Vec::new(),
    };
    cx.sprite(sp, "", None);
    if let Some(m) = &e.core.machine {
        for (name, s) in &m.states {
            if let Some(over) = s.set.get("sprite") {
                let ssp = state_sprite(sp, over)?;
                cx.sprite(&ssp, &format!("state.{name}"), Some(name));
            }
        }
    }
    // The set's other frames made for this entity.
    if let Some(set) = set {
        for (n, f) in set.frames.iter().enumerate() {
            let n = n as u32;
            if f.entity != entity || cx.out.iter().any(|o| o.set_index == Some(n)) {
                continue;
            }
            let id = format!("set.{n}");
            let r = set_frame(set, n, slot, &id);
            cx.add(
                r,
                &id,
                None,
                (!f.state.is_empty()).then_some(f.state.as_str()),
            );
        }
    }
    let mut out: Vec<Frame> = Vec::new();
    for f in cx.out {
        match out.iter_mut().find(|o| o.pieces == f.pieces) {
            Some(o) => {
                o.aliases.push(f.id);
                for c in f.clips {
                    if !o.clips.contains(&c) {
                        o.clips.push(c);
                    }
                }
            }
            None => out.push(f),
        }
    }
    Ok((out, cx.skipped))
}

/// The copy's sprite-set asset for a model, if it has a sprite-set slot
/// and the file is in `assets`.
pub fn read_set(model: &Model, assets: &Path) -> Result<Option<SpriteSet>, String> {
    let Some((slot, s)) = model
        .assets
        .iter()
        .find(|(_, s)| s.kind == AssetKind::SpriteSet)
    else {
        return Ok(None);
    };
    let asset = s.from.as_ref().map_or(slot.clone(), |f| f.asset.clone());
    let file = assets.join(format!("{asset}.toml"));
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Ok(None);
    };
    SpriteSet::parse(&text)
        .map(Some)
        .map_err(|e| format!("{}: {e}", file.display()))
}

/// Picks frames by id: `all`, a clip by its name (`clip0`,
/// `state.dead.clip1`: every frame of it), or each frame named.
pub fn pick(all: &[Frame], want: &[String]) -> Result<Vec<Frame>, String> {
    if want.iter().any(|w| w == "all") {
        return Ok(all.to_vec());
    }
    let mut out: Vec<Frame> = Vec::new();
    for w in want {
        let found: Vec<&Frame> = match all.iter().find(|f| &f.id == w || f.aliases.contains(w)) {
            Some(f) => vec![f],
            None => all.iter().filter(|f| f.clips.contains(w)).collect(),
        };
        if found.is_empty() {
            return Err(format!(
                "no frame or clip `{w}`; the sprite set has {:?}",
                all.iter().map(|f| f.id.as_str()).collect::<Vec<_>>()
            ));
        }
        for f in found {
            if !out.iter().any(|o| o.id == f.id) {
                out.push(f.clone());
            }
        }
    }
    Ok(out)
}

fn sheet_px(sheet: &[u8], sheet_w: u32, per_row: u32, t: u32, x: u32, y: u32) -> Option<u8> {
    let (tx, ty) = ((t % per_row) * 8, (t / per_row) * 8);
    sheet.get(((ty + y) * sheet_w + tx + x) as usize).copied()
}

/// A frame's picture, read from a sheet of `per_row` tiles a row: each
/// piece's drawn pixels over the ones before.
pub fn read(frame: &Frame, sheet: &[u8], sheet_w: u32, per_row: u32) -> Result<Pic, String> {
    let (w, h) = frame.size();
    let mut px = vec![0u8; (w * h) as usize];
    for p in &frame.pieces {
        for y in 0..8 {
            for x in 0..8 {
                let sx = if p.flip_x { 7 - x } else { x };
                let sy = if p.flip_y { 7 - y } else { y };
                let v = sheet_px(sheet, sheet_w, per_row, p.tile, sx, sy)
                    .ok_or(format!("tile {} is past the sheet", p.tile))?;
                let (fx, fy) = (p.x + x, p.y + y);
                if v != 0 && fx < w && fy < h {
                    px[(fy * w + fx) as usize] = v;
                }
            }
        }
    }
    Pic::new(w, h, px)
}

/// Writes a frame's picture into a sheet, each piece's 8x8 square back
/// into its tile, its flips undone. Pieces that overlap, or share a tile
/// the picture draws differently, can't all hold: [`holds`] says whether
/// they did.
pub fn write(frame: &Frame, pic: &Pic, sheet: &mut [u8], sheet_w: u32, per_row: u32) {
    for p in &frame.pieces {
        let (tx, ty) = ((p.tile % per_row) * 8, (p.tile / per_row) * 8);
        for y in 0..8 {
            for x in 0..8 {
                let sx = if p.flip_x { 7 - x } else { x };
                let sy = if p.flip_y { 7 - y } else { y };
                let (fx, fy) = (p.x + x, p.y + y);
                let v = if fx < pic.w && fy < pic.h {
                    pic.px[(fy * pic.w + fx) as usize]
                } else {
                    0
                };
                sheet[((ty + sy) * sheet_w + tx + sx) as usize] = v;
            }
        }
    }
}

/// Whether the sheet draws each frame as its picture: the check that an
/// edit holds across frames that share tiles (a clip's frames, mirrored
/// halves). Returns the frames that don't, with how many pixels differ.
pub fn holds(
    frames: &[(&Frame, &Pic)],
    sheet: &[u8],
    sheet_w: u32,
    per_row: u32,
) -> Result<Vec<(String, usize)>, String> {
    let mut bad = Vec::new();
    for (f, want) in frames {
        let got = read(f, sheet, sheet_w, per_row)?;
        let n = got.diff(want).len();
        if n > 0 {
            bad.push((f.id.clone(), n));
        }
    }
    Ok(bad)
}

/// One 8x8 tile of a sheet as 64 digits, row by row.
pub fn tile_digits(sheet: &[u8], sheet_w: u32, per_row: u32, t: u32) -> String {
    let (tx, ty) = ((t % per_row) * 8, (t / per_row) * 8);
    let mut s = String::with_capacity(64);
    for y in 0..8 {
        for x in 0..8 {
            s.push(char::from(
                b'0' + sheet[((ty + y) * sheet_w + tx + x) as usize].min(9),
            ));
        }
    }
    s
}

/// Puts a tile's 64 digits back into a sheet.
pub fn put_tile(
    sheet: &mut [u8],
    sheet_w: u32,
    per_row: u32,
    t: u32,
    digits: &str,
) -> Result<(), String> {
    if digits.len() != 64 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("tile {t}: not 64 color digits"));
    }
    let (tx, ty) = ((t % per_row) * 8, (t / per_row) * 8);
    for (i, b) in digits.bytes().enumerate() {
        let (x, y) = (i as u32 % 8, i as u32 / 8);
        sheet[((ty + y) * sheet_w + tx + x) as usize] = b - b'0';
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = r#"
[model]
schema = "0.4.0"
name = "set"
title = "Set"
start_level = "main"
view = "side"
frame_rate = "60"
systems = ["rules"]

[assets.tiles]
kind = "tiles"
role = "graphics"

[assets.sprites]
kind = "sprite-set"
role = "graphics"

[entities.hero]
role = "player"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[entities.hero.sprite]
slot = "tiles"
tiles = [0, 1, 2, 3]
width = 2
palette = 0

[[entities.hero.sprite.clips]]
when = { moving = true }
every = 8
frames = [{ tiles = [4, 5, 6, 7] }, { tiles = [0, 1, 2, 3] }, {}]

[entities.hero.machine]
initial = "alive"

[entities.hero.machine.states.alive]

[entities.hero.machine.states.dead]
set = { sprite = { tiles = [8, 9], width = 2, clips = [] } }

[entities.walker]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[entities.walker.sprite]
slot = "tiles"
width = 1
palette = 5
frame = { slot = "sprites", row = 0 }

[[entities.walker.sprite.clips]]
every = 8
frames = [{ row = 0 }, { row = 1 }]

[levels.main]
size = [1, 1]
tile_size = 16

[rules.game]
initial = "playing"

[rules.game.states.playing]
"#;

    /// Two walk frames of a made-up walker: the left half is tile 10 and
    /// the right half the same tile mirrored, then its legs swap.
    const SET: &str = r#"
schema = 1
asset = "sprites"

[[frame]]
entity = "walker"
state = "walk"
pieces = [{ tile = 10, slot = "tiles", x = 0, y = 0, palette = 5 }, { tile = 10, slot = "tiles", x = 8, y = 0, palette = 5, flip_x = true }, { tile = 11, slot = "tiles", x = 0, y = 8, palette = 5 }, { tile = 12, slot = "tiles", x = 8, y = 8, palette = 5 }]

[[frame]]
entity = "walker"
state = "walk"
pieces = [{ tile = 10, slot = "tiles", x = 0, y = 0, palette = 5 }, { tile = 10, slot = "tiles", x = 8, y = 0, palette = 5, flip_x = true }, { tile = 12, slot = "tiles", x = 0, y = 8, palette = 5, flip_x = true }, { tile = 11, slot = "tiles", x = 8, y = 8, palette = 5, flip_x = true }]

[[frame]]
entity = "walker"
state = "flat"
pieces = [{ tile = 13, slot = "tiles", x = 0, y = 8, palette = 5 }, { tile = 13, slot = "tiles", x = 8, y = 8, palette = 5, flip_x = true }]
"#;

    #[test]
    fn a_sprite_set_lists_each_picture_once() {
        let model = Model::parse(MODEL).unwrap();
        let (fs, skipped) = frames(&model, "hero", "tiles", None).unwrap();
        let ids: Vec<&str> = fs.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["base", "clip0.0", "state.dead"]);
        assert!(skipped.is_empty());
        assert_eq!(fs[1].clips, ["clip0"]);
        assert_eq!(
            fs[0].aliases,
            ["clip0.1"],
            "clip0's second frame is the base's tiles"
        );
        assert_eq!(pick(&fs, &["clip0".into()]).unwrap().len(), 2);
        assert_eq!(fs[2].size(), (16, 8));
        assert!(fs[0].is_grid());
        assert_eq!(pick(&fs, &["all".into()]).unwrap().len(), 3);
        assert_eq!(pick(&fs, &["clip0.1".into()]).unwrap()[0].id, "base");
        assert!(pick(&fs, &["clip9.0".into()]).is_err());
    }

    #[test]
    fn sprite_set_frames_resolve_by_row() {
        let model = Model::parse(MODEL).unwrap();
        let set = SpriteSet::parse(SET).unwrap();
        let (none, skipped) = frames(&model, "walker", "tiles", None).unwrap();
        assert!(
            none.is_empty() && !skipped.is_empty(),
            "no set: {skipped:?}"
        );
        let (fs, skipped) = frames(&model, "walker", "tiles", Some(&set)).unwrap();
        let ids: Vec<&str> = fs.iter().map(|f| f.id.as_str()).collect();
        // base and clip0.0 are row 0, once; set.2 is a frame the model
        // doesn't draw yet.
        assert_eq!(ids, ["base", "clip0.1", "set.2"], "{skipped:?}");
        assert_eq!(fs[0].tiles(), [10, 11, 12]);
        assert!(!fs[0].is_grid());
        assert_eq!(fs[2].size(), (16, 8));
        assert_eq!(fs[2].state.as_deref(), Some("flat"));
    }

    #[test]
    fn pieces_read_and_write_through_the_sheet_with_their_flips() {
        let model = Model::parse(MODEL).unwrap();
        let set = SpriteSet::parse(SET).unwrap();
        let (fs, _) = frames(&model, "walker", "tiles", Some(&set)).unwrap();
        let (sw, per_row) = (128, 16);
        let mut sheet = vec![0u8; 128 * 8];
        // Tile 10's top left pixel.
        sheet[10 * 8] = 1;
        let p = read(&fs[0], &sheet, sw, per_row).unwrap();
        assert_eq!(p.px[0], 1);
        assert_eq!(p.px[15], 1, "the mirrored half");
        // A recolor of both halves holds; drawing only the left one can't.
        let mut q = p.clone();
        q.px[0] = 2;
        q.px[15] = 2;
        let mut s2 = sheet.clone();
        write(&fs[0], &q, &mut s2, sw, per_row);
        assert!(holds(&[(&fs[0], &q)], &s2, sw, per_row).unwrap().is_empty());
        let mut r = p.clone();
        r.px[0] = 3;
        let mut s3 = sheet.clone();
        write(&fs[0], &r, &mut s3, sw, per_row);
        assert_eq!(
            holds(&[(&fs[0], &r)], &s3, sw, per_row).unwrap(),
            [("base".into(), 1)]
        );
        assert_eq!(tile_digits(&s2, sw, per_row, 10).as_bytes()[0], b'2');
        put_tile(&mut s2, sw, per_row, 10, &"0".repeat(64)).unwrap();
        assert_eq!(s2[80], 0);
    }
}
