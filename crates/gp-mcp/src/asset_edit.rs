//! `assets_edit`: semantic edits on an entity's sprite set in a working
//! copy, for an agent told "make the player's outline thicker" or "turn him
//! to face the other way" (docs/sprite-editing.md).
//!
//! An edit is a list of [`sprite_ops::Op`]s run in order on each frame
//! picked from the entity's sprite set (`crate::sprite_set`), or a list of
//! [`PaletteOp`]s on its palette. Each picture keeps its size, so the tile
//! budget holds. The result is reviewed by the consistency critic, checked
//! against the entity's hitbox and standing edge, drawn into the frames the
//! recorded baselines show it in (`crate::scene`) with its readability
//! against the backgrounds it was seen on, and placed with `assets_place`'s
//! checks: packed, every baseline's state digest the same, the model
//! checked.
//!
//! With `place: false` the edit is a proposal: shown, written nowhere but
//! the copy's art history, and placed when accepted. Every edit has a
//! sequence number and a status in the history (`crate::edit_log`): undo
//! goes back any number of steps, or undoes one older edit whose tiles no
//! later edit touched; redo puts undone edits back; accept and reject
//! decide a proposal or a placed edit.
//!
//! Everything here is deterministic and local; an image model would be a
//! `Generator` behind `assets_generate`, and an op that calls one (redraw a
//! region from a description) would come back through the same checks.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use gp_model::model::AssetKind;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::catalog::{self, Origin, State, Status};
use crate::critic;
use crate::edit_log::{self, apply_palette};
pub use crate::edit_log::{
    EditStatus, Entry, HISTORY_FILE, PaletteChange, PaletteOp, TileEdit, read_history,
};
use crate::generate::{Candidate, DEFAULT_MIN_SCORE};
use crate::place::{self, Content, Current, Provenance, Target};
use crate::sprite_ops::{self, Axis, Op, Pic};
use crate::sprite_set::{self, Frame};
use crate::{Config, Reply};

/// Where previews are written in a working copy.
pub const PREVIEW_DIR: &str = "previews";

/// Colors of a NES sprite: index 0 is transparent.
pub const COLORS: u8 = 4;

/// The most operations one call runs.
pub const MAX_OPS: usize = 64;

/// How many times bigger the preview draws each pixel.
pub const PREVIEW_SCALE: u32 = 8;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    pub copy: String,
    /// A tiles slot, such as `tiles` (or, for palette_ops, a palettes
    /// slot; by default the model's first).
    #[serde(default)]
    pub slot: Option<String>,
    /// The entity whose sprite is edited. Needed for ops and palette_ops,
    /// unless `tile` is given.
    #[serde(default)]
    pub entity: Option<String>,
    /// A level tile kind to edit instead of an entity: its name in the
    /// asset map (`stone block`), `<metatiles slot>[<index>]`, or its four
    /// tile numbers as hex (`05060708`). ops run on its 16x16 picture (its
    /// four tiles in the level tiles slot) and palette_ops on the
    /// background palette it's drawn with (docs/sprite-editing.md, "Level
    /// tiles").
    #[serde(default)]
    pub tile: Option<String>,
    /// The operations, run in order on each frame: recolor (pairs of
    /// indices, swapped at once, optional region), flip (axis x or y),
    /// mirror (axis, keep first or second half), shift (dx, dy, wrap),
    /// outline (index, corners), fill (region, index), pixels (set: [x, y,
    /// index]), copy (from_entity or from_tiles, optional flip), load
    /// (file: a PNG an external editor saved, from assets_export),
    /// match_style (entity: take its outline), tween (from, to: frame ids;
    /// an in-between of two frames).
    #[serde(default)]
    pub ops: Vec<Op>,
    /// Which frames of the sprite set the ops run on: frame ids as
    /// surface_describe and the reply's `frames` list them (`base`,
    /// `clip0.1`, `state.dead`), or `all`. Default `base`.
    #[serde(default)]
    pub frames: Vec<String>,
    /// Operations on the entity's palette instead of its pixels: color
    /// (index 1-3, nes 0-63), swap (a, b), hue (by steps of 12), brightness
    /// (by rows of 4), copy_from (entity). Not with ops.
    #[serde(default)]
    pub palette_ops: Vec<PaletteOp>,
    /// Resize the sprite to [width, height] in tiles, the art kept at its
    /// bottom left (where it stands on screen), taking tiles nothing draws;
    /// the model's sprite gets the new tiles and width. The ops then run on
    /// the resized picture. Only the sprite's own frame, without cells or
    /// clip frames of tiles.
    #[serde(default)]
    pub resize: Option<[u32; 2]>,
    /// Whole pictures by frame id, as rows of color indices 0 to 3, put in
    /// before the ops: an external editor's save (gp assets open), or an
    /// agent drawing a frame outright.
    #[serde(default)]
    pub pictures: BTreeMap<String, Vec<String>>,
    /// The external editor the pictures came from (`pixelorama`): the slot
    /// records them as uploaded by a person, with the editor named.
    #[serde(default)]
    pub editor: Option<String>,
    /// A folder assets_export (or gp assets open) wrote and an editor
    /// saved into: its frames are read back with the image format's checks
    /// and become `pictures` (and `entity`, `slot`, `editor`).
    #[serde(default)]
    pub import: Option<String>,
    /// With palette_ops: apply them to every palette whose role (the slot's
    /// item_roles) is the entity's palette's, so copies of it stay alike.
    #[serde(default)]
    pub same_role: bool,
    /// Place the result (default true). With false the edit is a proposal:
    /// previewed, kept in the history with status `proposed`, and placed
    /// by accept.
    #[serde(default = "yes")]
    pub place: bool,
    /// Draw the edit into the recorded frames it shows in (default true).
    #[serde(default)]
    pub in_context: Option<bool>,
    /// Undo instead (no ops): the newest placed edit, or `seq`.
    #[serde(default)]
    pub undo: bool,
    /// With undo, accept or reject: the edit's sequence number.
    #[serde(default)]
    pub seq: Option<u64>,
    /// Put back the edit undone last.
    #[serde(default)]
    pub redo: bool,
    /// Keep an edit (a proposal is placed): its sequence number.
    #[serde(default)]
    pub accept: Option<u64>,
    /// Turn an edit down (a placed one is undone): its sequence number.
    #[serde(default)]
    pub reject: Option<u64>,
    /// The consistency critic: `off`, `advise` (default) or `enforce`
    /// (place only when the review passes with `min_score`).
    #[serde(default)]
    pub critic: Option<String>,
    #[serde(default)]
    pub min_score: Option<f64>,
    /// What the person asked for, recorded in the history and the slot's
    /// note.
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

fn yes() -> bool {
    true
}

/// The sprite of a checked target, as a picture.
pub fn sprite_pic(t: &Target) -> Result<Pic, String> {
    let (w, h) = t.image_size().ok_or("not a tiles slot")?;
    Pic::new(w, h, t.current_pixels().ok_or("not a tiles slot")?)
}

/// The picture a `copy` reads: another entity's sprite in the same sheet,
/// or tiles of the sheet laid out as the target's sprite is.
fn copy_source(t: &Target, op: &Op, colors: [[u8; 3]; 4]) -> Result<Option<Pic>, String> {
    if let Op::Load { file } = op {
        let (w, h) = t.image_size().ok_or("not a tiles slot")?;
        return Ok(Some(crate::exchange::read_image(
            Path::new(file),
            (w, h),
            colors,
        )?));
    }
    let Op::Copy {
        from_entity,
        from_tiles,
        ..
    } = op
    else {
        return Ok(None);
    };
    let Current::Tiles {
        tiles: count,
        per_row,
        size,
        pixels,
    } = &t.current
    else {
        return Err("copy needs a tiles slot".into());
    };
    let sprite = t.sprite.as_ref().ok_or("copy needs an entity")?;
    let (list, width) = match (from_entity, from_tiles) {
        (Some(e), None) => {
            let sp = t
                .model
                .entities
                .get(e)
                .and_then(|e| e.core.sprite.as_ref())
                .ok_or(format!("entity `{e}` has no sprite"))?;
            if sp.slot != t.slot {
                return Err(format!("entity `{e}`'s sprite is in slot `{}`", sp.slot));
            }
            (sp.tiles.clone(), sp.width.max(1))
        }
        (None, Some(list)) => (list.clone(), sprite.width.max(1)),
        _ => return Err("copy takes one of from_entity and from_tiles".into()),
    };
    if list.is_empty() {
        return Err("copy's source has no tiles".into());
    }
    if let Some(bad) = list.iter().find(|&&n| n >= *count) {
        return Err(format!("tile {bad} is past the slot's {count}"));
    }
    let (w, h) = (width * 8, (list.len() as u32).div_ceil(width) * 8);
    let mut px = vec![0u8; (w * h) as usize];
    for (k, &n) in list.iter().enumerate() {
        let (sx, sy) = ((k as u32 % width) * 8, (k as u32 / width) * 8);
        let (tx, ty) = ((n % per_row) * 8, (n / per_row) * 8);
        for y in 0..8 {
            for x in 0..8 {
                px[((sy + y) * w + sx + x) as usize] =
                    pixels[((ty + y) * size.0 + tx + x) as usize];
            }
        }
    }
    Ok(Some(Pic::new(w, h, px)?))
}

/// Checks of the edited sprite against what the entity does: its hitbox
/// (the box the simulation touches with) and the edge it stands on.
/// Advice, not refusals: art may overhang a hitbox on purpose.
fn body_checks(t: &Target, before: &Pic, after: &Pic) -> Value {
    let Some(e) = t.entity.as_deref().and_then(|n| t.model.entities.get(n)) else {
        return json!([]);
    };
    let int = |f: gp_model::Fixed| f.to_int().unwrap_or(f.raw() >> 16);
    let body_h = int(e.core.size[1]);
    // A sprite stands on the body's bottom edge (gp-present).
    let top = body_h - i64::from(after.h);
    let hb = &e.core.hitbox;
    let (hx, hy, hw, hh) = (int(hb.x), int(hb.y), int(hb.w), int(hb.h));
    let outside = |p: &Pic| {
        (0..p.px.len())
            .filter(|&i| {
                let (x, y) = (i as i64 % i64::from(p.w), i as i64 / i64::from(p.w) + top);
                p.px[i] != 0 && !(x >= hx && x < hx + hw && y >= hy && y < hy + hh)
            })
            .count()
    };
    let (ob, oa) = (outside(before), outside(after));
    let mut checks = vec![json!({
        "name": "body.hitbox",
        "verdict": if oa > ob + 8 { "warn" } else { "pass" },
        "value": oa,
        "before": ob,
        "message": format!("{oa} drawn pixels outside the hitbox ({ob} before); a player judges hits by the art"),
    })];
    let bottom = |p: &Pic| p.ink_box().map(|b| b.3);
    let (bb, ba) = (bottom(before), bottom(after));
    let stands = bb == Some(before.h - 1);
    checks.push(json!({
        "name": "body.stands",
        "verdict": if stands && ba != Some(after.h - 1) { "warn" } else { "pass" },
        "value": ba,
        "before": bb,
        "message": if stands && ba != Some(after.h - 1) {
            "the art no longer reaches the bottom row, so it will float above the ground"
        } else {
            "the art's lowest row is where it was, or it didn't stand on the edge before"
        },
    }));
    json!(checks)
}

/// The NES colors the sprite draws with, as RGB, from the copy's palettes:
/// index 0 is the backdrop the sprite shows through to.
pub fn sprite_colors(subject: &critic::Subject) -> [[u8; 3]; 4] {
    let pal = subject.palette.unwrap_or([0x0F, 0x00, 0x10, 0x30]);
    let mut c = pal.map(critic::nes_rgb);
    if let Some(b) = subject.backdrop {
        c[0] = critic::nes_rgb(b);
    }
    c
}

/// Before, after, after facing left (when the sprite flips) and the changed
/// pixels, side by side, each pixel drawn `PREVIEW_SCALE` times over: an
/// indexed PNG in the sprite's colors.
pub fn preview_png(before: &Pic, after: &Pic, colors: [[u8; 3]; 4], facing_flip: bool) -> Vec<u8> {
    const GAP: u8 = 4;
    const CHANGED: u8 = 5;
    const DIM: u8 = 6;
    let mut panels = vec![before.clone(), after.clone()];
    if facing_flip {
        panels.push(after.flipped(Axis::X));
    }
    let changed: Vec<(u32, u32)> = before.diff(after);
    let mut marks = after.clone();
    for p in &mut marks.px {
        if *p != 0 {
            *p = DIM;
        }
    }
    for &(x, y) in &changed {
        marks.px[(y * marks.w + x) as usize] = CHANGED;
    }
    panels.push(marks);
    let s = PREVIEW_SCALE;
    let gap = 1;
    let (pw, ph) = (before.w * s, before.h * s);
    let n = panels.len() as u32;
    let (w, h) = (n * pw + (n + 1) * gap * s, ph + 2 * gap * s);
    let mut px = vec![GAP; (w * h) as usize];
    for (k, p) in panels.iter().enumerate() {
        let ox = gap * s + k as u32 * (pw + gap * s);
        for y in 0..ph {
            for x in 0..pw {
                px[((gap * s + y) * w + ox + x) as usize] = p.px[((y / s) * p.w + x / s) as usize];
            }
        }
    }
    let mut palette = Vec::with_capacity(7 * 3);
    for c in colors {
        palette.extend_from_slice(&c);
    }
    palette.extend_from_slice(&[0x60, 0x60, 0x60, 0xff, 0x30, 0xc0, 0x50, 0x50, 0x50]);
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(palette);
    let mut wr = enc.write_header().expect("PNG header into memory");
    wr.write_image_data(&px).expect("PNG data into memory");
    wr.finish().expect("PNG end into memory");
    out
}

/// Every edited frame of a sprite set, a row each: before, then after,
/// `PREVIEW_SCALE / 2` times over, in the sprite's colors (index 0 drawn
/// as the backdrop). One picture for a clip's frames side by side.
pub fn frames_png(pairs: &[(&Pic, &Pic)], colors: [[u8; 3]; 4]) -> Vec<u8> {
    let s = (PREVIEW_SCALE / 2).max(1);
    let gap = s;
    let cw = pairs.iter().map(|(b, _)| b.w).max().unwrap_or(8) * s;
    let ch = pairs.iter().map(|(b, _)| b.h).max().unwrap_or(8) * s;
    let (w, h) = (2 * cw + 3 * gap, pairs.len() as u32 * (ch + gap) + gap);
    let mut px = vec![4u8; (w * h) as usize];
    for (r, (b, a)) in pairs.iter().enumerate() {
        for (k, p) in [b, a].iter().enumerate() {
            let (ox, oy) = (gap + k as u32 * (cw + gap), gap + r as u32 * (ch + gap));
            for y in 0..p.h * s {
                for x in 0..p.w * s {
                    px[((oy + y) * w + ox + x) as usize] =
                        p.px[((y / s) * p.w + x / s) as usize].min(3);
                }
            }
        }
    }
    let mut palette = Vec::with_capacity(5 * 3);
    for c in colors {
        palette.extend_from_slice(&c);
    }
    palette.extend_from_slice(&[0x60, 0x60, 0x60]);
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(palette);
    let mut wr = enc.write_header().expect("PNG header into memory");
    wr.write_image_data(&px).expect("PNG data into memory");
    wr.finish().expect("PNG end into memory");
    out
}

/// One frame of the sprite set as a target the tiles-and-width tools take
/// (the critic, copy): the entity's, with a grid of the frame's size in
/// place of the sprite's own. A frame laid out otherwise (flips, places)
/// gets a stand-in grid of its size, since those tools read only its size
/// from the target; None when its size isn't whole tiles.
fn frame_target(t: &Target, f: &Frame) -> Option<Target> {
    if !f.w.is_multiple_of(8) || !f.h.is_multiple_of(8) {
        return None;
    }
    let mut ft = t.clone();
    let sp = ft.sprite.as_mut()?;
    let n = ((f.w / 8) * (f.h / 8)) as usize;
    sp.tiles = if f.is_grid() {
        f.tiles()
    } else {
        vec![f.pieces.first().map_or(0, |p| p.tile); n]
    };
    sp.width = f.w / 8;
    sp.frame = None;
    sp.cells.clear();
    sp.clips.clear();
    Some(ft)
}

/// The whole slot as the placement's target, for the entity: an edit that
/// may touch several frames writes the whole sheet (or every palette).
fn slot_target(t: &Target) -> Target {
    let mut s = t.clone();
    s.sprite = None;
    s
}

/// The pictures an op reads besides the one it edits.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sources(
    config: &Config,
    ft: &Target,
    op: &Op,
    colors: [[u8; 3]; 4],
    all: &[Frame],
    sheet: &[u8],
    sheet_w: u32,
    per_row: u32,
) -> Result<Vec<Pic>, String> {
    Ok(match op {
        Op::Copy { .. } | Op::Load { .. } => copy_source(ft, op, colors)?.into_iter().collect(),
        Op::MatchStyle { entity } => {
            let sp = ft
                .model
                .entities
                .get(entity)
                .and_then(|e| e.core.sprite.as_ref())
                .ok_or(format!("entity `{entity}` has no sprite"))?;
            let rt = place::target(config, &ft.copy, &sp.slot, Some(entity))?;
            vec![sprite_pic(&rt)?]
        }
        Op::Tween { from, to } => [from, to]
            .iter()
            .map(|id| {
                let f = all
                    .iter()
                    .find(|f| &f.id == *id)
                    .ok_or(format!("no frame `{id}` in the sprite set"))?;
                sprite_set::read(f, sheet, sheet_w, per_row)
            })
            .collect::<Result<_, _>>()?,
        _ => Vec::new(),
    })
}

/// The readiness gate (crate::ready): the replayed contexts, the readiness
/// list, and the copy's assets now; or the refusal to return.
pub(crate) fn gate(
    config: &Config,
    t: &Target,
    entity: &str,
) -> Result<Result<(Value, Value, PathBuf), Reply>, String> {
    // An edit reads the model, so the model must know the sprite fully.
    // The recorded baselines are its evidence and the preview's input:
    // where the sprite was seen comes from replaying them.
    let (assets_now, _) = place::current_assets(config, &t.dir, &t.model)?;
    let seen = crate::contexts::contexts(t, &assets_now, entity)?;
    let ready = crate::ready::readiness(t, entity, seen["seen"].as_u64().unwrap_or(0));
    if ready["ready"] == json!(true) {
        return Ok(Ok((seen, ready, assets_now)));
    }
    let missing: Vec<String> = ready["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["status"] == "missing")
        .map(|i| {
            format!(
                "{}: {}{}",
                i["item"].as_str().unwrap_or(""),
                i["detail"].as_str().unwrap_or(""),
                i["fill"]
                    .as_str()
                    .map_or(String::new(), |f| format!(" (fill: {f})"))
            )
        })
        .collect();
    Ok(Err(Reply::refused(json!({
        "ok": false,
        "written": false,
        "stage": "model",
        "error": format!("the model doesn't know `{entity}`'s sprite well enough to edit it: {}", missing.join("; ")),
        "readiness": ready,
    }))))
}

/// A copy of the assets in `from` with one file replaced, for drawing the
/// edit before it's placed. The caller removes it.
pub(crate) fn stage_assets(
    t: &Target,
    from: &Path,
    file: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let map = place::read_map(from)?;
    let stage = t.dir.join(format!(".edit-stage-{}", std::process::id()));
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| format!("clearing {}: {e}", stage.display()))?;
    }
    for name in place::asset_files(&map, from) {
        let dst = stage.join(&name);
        if let Some(p) = dst.parent() {
            fs::create_dir_all(p).map_err(|e| format!("creating {}: {e}", p.display()))?;
        }
        fs::copy(from.join(&name), &dst).map_err(|e| format!("staging {name}: {e}"))?;
    }
    fs::write(stage.join(file), bytes).map_err(|e| format!("staging {file}: {e}"))?;
    Ok(stage)
}

/// The edit drawn into the recorded frames, with its readability, merged
/// into `out` as `in_context`; the shots' PNG is returned for the copy's
/// previews.
fn draw_in_context(
    t: &Target,
    assets_now: &Path,
    file: &str,
    bytes: &[u8],
    entity: &str,
    seen: &Value,
    out: &mut Value,
) -> Result<Option<Vec<u8>>, String> {
    let stage = stage_assets(t, assets_now, file, bytes)?;
    let r = crate::scene::in_context(t, assets_now, &stage, entity, seen);
    let _ = fs::remove_dir_all(&stage);
    let (png, mut j) = r?;
    if let Some(p) = &png {
        j["png_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(p));
    }
    if let Some(checks) = out["checks"].as_array_mut() {
        checks.push(j["readability"].clone());
    }
    out["in_context"] = j;
    Ok(png)
}

pub(crate) fn edit_label(names: &[&str]) -> String {
    format!(
        "gp-mcp {} assets_edit {}",
        env!("CARGO_PKG_VERSION"),
        names.join(",")
    )
}

/// What a placed edit records as the slot's new state: an edit is made
/// from what the slot holds, so it keeps its license.
fn edit_provenance(t: &Target, e: &Entry, author: Option<&str>) -> Provenance {
    Provenance {
        status: Status::Replaced,
        // A save from an external editor is a person's drawing.
        origin: if e.editor.is_some() {
            Origin::Uploaded
        } else {
            Origin::Generated
        },
        license: t.state.license.clone(),
        generator: Some(match &e.editor {
            Some(ed) => format!("{ed} via gp assets open ({})", edit_label(&e.op_names())),
            None => edit_label(&e.op_names()),
        }),
        author: author.map(str::to_owned),
        note: e.prompt.clone(),
    }
}

/// Writes previews beside the history and returns their paths.
pub(crate) fn write_previews(
    dir: &Path,
    seq: u64,
    png: &[u8],
    context: Option<&[u8]>,
) -> Result<Value, String> {
    let previews = dir.join(PREVIEW_DIR);
    fs::create_dir_all(&previews).map_err(|e| format!("making {PREVIEW_DIR}/: {e}"))?;
    let file = previews.join(format!("edit-{seq}.png"));
    fs::write(&file, png).map_err(|e| format!("writing {}: {e}", file.display()))?;
    let mut out = json!({ "edit": file });
    if let Some(c) = context {
        let f = previews.join(format!("edit-{seq}-context.png"));
        fs::write(&f, c).map_err(|e| format!("writing {}: {e}", f.display()))?;
        out["context"] = json!(f);
    }
    Ok(out)
}

pub fn edit(config: &Config, mut a: EditArgs) -> Result<Reply, String> {
    if let Some(dir) = a.import.take() {
        let dir = PathBuf::from(dir);
        let trip = crate::editor_trip::read_trip(&dir)?;
        if trip.copy != a.copy {
            return Err(format!(
                "{} was exported from copy `{}`",
                dir.display(),
                trip.copy
            ));
        }
        match crate::editor_trip::import(&dir)? {
            Ok(pics) => {
                a.pictures = pics.into_iter().map(|(id, p)| (id, p.rows())).collect();
            }
            Err(problems) => {
                return Ok(Reply::refused(json!({
                    "ok": false, "written": false, "stage": "import", "problems": problems,
                })));
            }
        }
        a.entity = Some(trip.entity);
        a.slot = Some(trip.slot);
        a.editor = a.editor.or(Some(trip.editor));
    }
    let modes = [a.undo, a.redo, a.accept.is_some(), a.reject.is_some()]
        .iter()
        .filter(|m| **m)
        .count();
    if modes > 1 {
        return Err("give one of undo, redo, accept and reject".into());
    }
    if modes == 1 && !(a.ops.is_empty() && a.palette_ops.is_empty()) {
        return Err("undo, redo, accept and reject take no ops".into());
    }
    if a.undo {
        return undo(config, &a);
    }
    if a.redo {
        return redo(config, &a);
    }
    if let Some(seq) = a.accept.or(a.reject) {
        let by = a.author.as_deref().unwrap_or("agent");
        return decide(config, &a.copy, seq, a.accept.is_some(), by);
    }
    if a.seq.is_some() {
        return Err("seq goes with undo".into());
    }
    if let Some(tile) = a.tile.clone() {
        if a.entity.is_some() {
            return Err("give entity or tile, not both".into());
        }
        return crate::tile_edit::edit(config, &a, &tile);
    }
    let entity = a.entity.clone().ok_or(
        "give the entity whose sprite to edit (surface_describe lists them in the slot's used_by)",
    )?;
    let n = a.ops.len() + a.palette_ops.len();
    if n > MAX_OPS {
        return Err(format!("{n} ops; one call runs at most {MAX_OPS}"));
    }
    if a.resize.is_some() && !a.palette_ops.is_empty() {
        return Err("resize goes with ops, not palette_ops".into());
    }
    match (
        a.ops.is_empty() && a.resize.is_none() && a.pictures.is_empty(),
        a.palette_ops.is_empty(),
    ) {
        (true, true) => Err("give at least one op".into()),
        (false, false) => Err(
            "give ops or palette_ops, not both: pixels and colors are in different slots".into(),
        ),
        (_, false) => palette_edit(config, &a, &entity),
        (false, true) => pixel_edit(config, &a, &entity),
    }
}

/// Keeps a finished edit: a proposal is written to the
/// history as proposed; a placed one is pushed on the history and the redo
/// list is emptied.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    config: &Config,
    a: &EditArgs,
    t: &Target,
    mut entry: Entry,
    out: &mut Value,
    png: &[u8],
    context: Option<&[u8]>,
    content: Content,
) -> Result<Reply, String> {
    let files = write_previews(&t.dir, entry.seq, png, context)?;
    out["item"] = json!(entry.item(&a.copy));
    out["seq"] = json!(entry.seq);
    out["previews"] = files.clone();
    if !a.place {
        entry.status = EditStatus::Proposed;
        edit_log::write_proposal(&t.dir, &entry)?;
        out["ok"] = json!(true);
        out["written"] = json!(false);
        out["status"] = json!("proposed");
        out["note"] = json!(format!(
            "a proposal: accept {} to place it, reject it to drop it",
            entry.seq
        ));
        return Ok(Reply::ok(out.take()));
    }
    let p = edit_provenance(t, &entry, a.author.as_deref());
    let placed = place_with_sprite(config, &a.copy, t, &entry, true, &p, content)?;
    if placed.json["written"] == json!(true) {
        let mut history = read_history(&t.dir)?;
        history.push(entry.clone());
        edit_log::write_history(&t.dir, &history)?;
        edit_log::write_redo(&t.dir, &[])?;
        out["status"] = json!("placed");
        out["history"] =
            json!({ "seq": entry.seq, "depth": history.len(), "preview_file": files["edit"] });
    }
    if let (Some(o), Some(r)) = (out.as_object_mut(), placed.json.as_object()) {
        o.extend(r.clone());
    }
    Ok(Reply {
        json: out.take(),
        is_error: placed.is_error,
    })
}

pub(crate) fn critic_review(
    a: &EditArgs,
    subject: &critic::Subject,
) -> Result<(Option<critic::Review>, &'static str), String> {
    let mode = match a.critic.as_deref().unwrap_or("advise") {
        "off" => "off",
        "advise" => "advise",
        "enforce" => "enforce",
        m => return Err(format!("critic `{m}` isn't off, advise or enforce")),
    };
    if mode == "off" {
        return Ok((None, mode));
    }
    let c = critic::critics()
        .into_iter()
        .find(|c| c.name() == critic::DEFAULT)
        .ok_or("no default critic")?;
    Ok((Some(c.review(subject)?), mode))
}

/// With the critic enforcing, the reply that refuses a failing review.
pub(crate) fn enforce(
    a: &EditArgs,
    review: &Option<critic::Review>,
    mode: &str,
    out: &mut Value,
) -> Option<Reply> {
    let r = review.as_ref()?;
    let min_score = a.min_score.unwrap_or(DEFAULT_MIN_SCORE);
    if mode != "enforce" || (r.pass && r.score >= min_score) {
        return None;
    }
    out["ok"] = json!(false);
    out["written"] = json!(false);
    out["stage"] = json!("critic");
    out["problems"] = json!(
        r.problems()
            .iter()
            .map(|c| format!("{}: {}", c.name, c.message))
            .collect::<Vec<_>>()
    );
    Some(Reply::ok(out.take()))
}

fn pixel_edit(config: &Config, a: &EditArgs, entity: &str) -> Result<Reply, String> {
    let slot = a.slot.clone().unwrap_or_else(|| "tiles".to_owned());
    let t = place::target(config, &a.copy, &slot, Some(entity))?;
    let Current::Tiles {
        per_row,
        size: (sw, sh),
        pixels,
        ..
    } = &t.current
    else {
        return Err(format!(
            "slot `{slot}` isn't a tiles slot; assets_edit edits sprites (palette_ops edit colors)"
        ));
    };
    let (sw, sh, per_row) = (*sw, *sh, *per_row);
    let (assets_here, _) = place::current_assets(config, &t.dir, &t.model)?;
    let set = sprite_set::read_set(&t.model, &assets_here)?;
    let (all, skipped) = sprite_set::frames(&t.model, entity, &slot, set.as_ref())?;
    if all.is_empty() {
        return Err(format!(
            "`{entity}`'s sprite set has no frame of tiles in `{slot}`: {}",
            json!(skipped)
        ));
    }
    let want = match (a.frames.is_empty(), a.pictures.is_empty()) {
        (false, _) => a.frames.clone(),
        (true, false) => a.pictures.keys().cloned().collect(),
        (true, true) => vec![all[0].id.clone()],
    };
    let picked = sprite_set::pick(&all, &want)?;
    let (seen, ready, assets_now) = match gate(config, &t, entity)? {
        Ok(x) => x,
        Err(refused) => return Ok(refused),
    };
    if let Some([rw, rh]) = a.resize {
        return resize_edit(config, a, &t, entity, rw, rh, seen, ready, &assets_now);
    }
    let first = sprite_set::read(&picked[0], pixels, sw, per_row)?;
    let colors_now = match frame_target(&t, &picked[0]) {
        Some(bt) => critic::subject(config, &bt, &Candidate::Pixels(first.px.clone()), None)
            .map(|s| sprite_colors(&s))?,
        None => [[0, 0, 0], [0x60; 3], [0xa0; 3], [0xff; 3]],
    };
    let mut sheet = pixels.clone();
    let mut done: Vec<(Frame, Target, Pic, Pic, Vec<Value>)> = Vec::new();
    for f in &picked {
        let ft = frame_target(&t, f).unwrap_or_else(|| t.clone());
        let before = sprite_set::read(f, pixels, sw, per_row)?;
        let mut now = before.clone();
        let mut steps = Vec::new();
        let given = a
            .pictures
            .get(&f.id)
            .or_else(|| f.aliases.iter().find_map(|x| a.pictures.get(x)));
        if let Some(rows) = given {
            let p = Pic::from_rows(rows).map_err(|e| format!("picture for frame {}: {e}", f.id))?;
            if (p.w, p.h) != (before.w, before.h) {
                return Err(format!(
                    "the picture for frame {} is {}x{}, the frame {}x{}",
                    f.id, p.w, p.h, before.w, before.h
                ));
            }
            if let Some(v) = p.px.iter().find(|&&v| v >= COLORS) {
                return Err(format!(
                    "the picture for frame {} has color index {v}",
                    f.id
                ));
            }
            steps.push(json!({ "op": "picture", "changed": now.diff(&p).len() }));
            now = p;
        }
        for (i, op) in a.ops.iter().enumerate() {
            let at = |e: String| format!("frame {} op {i} ({}): {e}", f.id, op.name());
            let src =
                sources(config, &ft, op, colors_now, &all, pixels, sw, per_row).map_err(at)?;
            let refs: Vec<&Pic> = src.iter().collect();
            let next = sprite_ops::apply(&now, op, COLORS, &refs).map_err(at)?;
            steps.push(json!({ "op": op.name(), "changed": now.diff(&next).len() }));
            now = next;
        }
        sprite_set::write(f, &now, &mut sheet, sw, per_row);
        done.push((f.clone(), ft, before, now, steps));
    }
    // The edit must hold in every frame it touched: frames that share a
    // tile (a clip's frames, mirrored halves) must draw it alike.
    let pairs: Vec<(&Frame, &Pic)> = done.iter().map(|(f, _, _, a, _)| (f, a)).collect();
    let bad = sprite_set::holds(&pairs, &sheet, sw, per_row)?;
    if !bad.is_empty() {
        return Ok(Reply::refused(json!({
            "ok": false,
            "written": false,
            "stage": "frames",
            "error": format!(
                "the edit doesn't hold in {}: those frames share tiles (or their pieces overlap) and the ops drew the shared tiles differently; edit the frames so shared tiles agree (a recolor does), or edit one frame",
                bad.iter().map(|(f, n)| format!("{f} ({n} pixels)")).collect::<Vec<_>>().join(", ")
            ),
        })));
    }
    let mut tiles: Vec<u32> = picked.iter().flat_map(Frame::tiles).collect();
    tiles.sort_unstable();
    tiles.dedup();
    let tile_edits: Vec<TileEdit> = tiles
        .iter()
        .filter_map(|&n| {
            let b = sprite_set::tile_digits(pixels, sw, per_row, n);
            let af = sprite_set::tile_digits(&sheet, sw, per_row, n);
            (b != af).then_some(TileEdit {
                tile: n,
                before: b,
                after: af,
            })
        })
        .collect();
    let (f0, ft0, before, after, steps0) = &done[0];
    let subject = match frame_target(&t, f0) {
        Some(_) => Some(critic::subject(
            config,
            ft0,
            &Candidate::Pixels(after.px.clone()),
            a.prompt.as_deref(),
        )?),
        None => None,
    };
    let (review, mode) = match &subject {
        Some(s) => critic_review(a, s)?,
        None => (None, "off"),
    };
    let facing = t.sprite.as_ref().is_some_and(|s| s.flip_when_facing_left);
    let colors = subject.as_ref().map_or(colors_now, sprite_colors);
    // One frame: the close-up with its facing flip and changes; several:
    // a row a frame, so a clip's frames are seen together.
    let png = if done.len() == 1 {
        preview_png(before, after, colors, facing)
    } else {
        frames_png(
            &done
                .iter()
                .map(|(_, _, b, a, _)| (b, a))
                .collect::<Vec<_>>(),
            colors,
        )
    };
    // Frames not picked that draw a changed tile change too.
    let changed_tiles: Vec<u32> = tile_edits.iter().map(|e| e.tile).collect();
    let also: Vec<&str> = all
        .iter()
        .filter(|f| !picked.iter().any(|p| p.id == f.id))
        .filter(|f| f.tiles().iter().any(|t| changed_tiles.contains(t)))
        .map(|f| f.id.as_str())
        .collect();
    let changed: usize = done.iter().map(|(_, _, b, af, _)| b.diff(af).len()).sum();
    let body = body_checks(ft0, before, after);
    let frames_json: Vec<Value> = done
        .iter()
        .map(|(f, ft, b, af, steps)| {
            json!({
                "frame": f.id, "tiles": f.tiles(), "size": [b.w, b.h], "state": f.state, "clips": f.clips, "aliases": f.aliases,
                "set_index": f.set_index, "changed": b.diff(af).len(), "ops": steps, "body": body_checks(ft, b, af),
            })
        })
        .collect();
    let mut out = json!({
        "copy": a.copy,
        "slot": slot,
        "entity": entity,
        "ops": steps0,
        "frames": frames_json,
        "sprite_set": {
            "frames": all.iter().map(|f| json!({ "frame": f.id, "tiles": f.tiles(), "size": [f.w, f.h], "state": f.state, "clips": f.clips, "aliases": f.aliases, "set_index": f.set_index })).collect::<Vec<_>>(),
            "skipped": skipped,
        },
        "also_changes": also,
        "pixels_changed": changed,
        "ink": { "before": before.ink(), "after": after.ink() },
        "body": body,
        "checks": body.clone(),
        "tiles": if picked[0].is_grid() { sprite_ops::tile_report(after, picked[0].w / 8) } else { json!(null) },
        "critic": review.as_ref().map(critic::Review::json),
        "contexts": seen,
        "readiness": ready,
        "preview": {
            "panels": if done.len() > 1 { json!(["before", "after"]) } else if facing { json!(["before", "after", "after facing left", "changed"]) } else { json!(["before", "after", "changed"]) },
            "rows": if done.len() > 1 { json!(picked.iter().map(|f| f.id.clone()).collect::<Vec<_>>()) } else { json!([picked[0].id]) },
            "frame": picked[0].id,
            "scale": PREVIEW_SCALE,
            "png_base64": base64::engine::general_purpose::STANDARD.encode(&png),
        },
        "after": after.rows(),
    });
    let new_sheet = gp_assets::chr::write_png(sw, sh, &sheet);
    let context = if changed > 0 && a.in_context != Some(false) {
        draw_in_context(
            &t,
            &assets_now,
            &t.asset.file(),
            &new_sheet,
            entity,
            &out["contexts"].clone(),
            &mut out,
        )?
    } else {
        None
    };
    if let Some(r) = enforce(a, &review, mode, &mut out) {
        return Ok(r);
    }
    if changed == 0 {
        out["ok"] = json!(true);
        out["written"] = json!(false);
        out["note"] = json!("the ops changed no pixel, so nothing was placed");
        return Ok(Reply::ok(out));
    }
    let entry = Entry {
        seq: edit_log::next_seq(&t.dir)?,
        slot: slot.clone(),
        entity: entity.to_owned(),
        ops: a.ops.clone(),
        palette_ops: Vec::new(),
        prompt: a.prompt.clone(),
        before: before.rows(),
        after: after.rows(),
        state_before: t.state.clone(),
        frames: picked.iter().map(|f| f.id.clone()).collect(),
        tiles: tile_edits,
        palettes: None,
        pictures: a.pictures.keys().cloned().collect(),
        editor: a.editor.clone(),
        sprite: None,
        status: EditStatus::Placed,
        decided_by: None,
        tile: None,
    };
    record(
        config,
        a,
        &t,
        entry,
        &mut out,
        &png,
        context.as_deref(),
        Content::Pixels(sheet),
    )
}

/// A resize (EditArgs::resize): new tiles from the slot's free ones, the
/// picture at its new size, the ops on it, then as any edit, with the
/// model's sprite changed in the same placement.
#[allow(clippy::too_many_arguments)]
fn resize_edit(
    config: &Config,
    a: &EditArgs,
    t: &Target,
    entity: &str,
    rw: u32,
    rh: u32,
    seen: Value,
    ready: Value,
    assets_now: &Path,
) -> Result<Reply, String> {
    let sp = t.sprite.clone().ok_or("no sprite")?;
    if !(1..=8).contains(&rw) || !(1..=8).contains(&rh) {
        return Err("resize is 1 to 8 tiles each way".into());
    }
    if !sp.cells.is_empty()
        || sp
            .clips
            .iter()
            .any(|c| c.frames.iter().any(|f| !f.tiles.is_empty()))
    {
        return Err(format!(
            "`{entity}`'s sprite has cells or clip frames of its own tiles; resize takes a sprite of one frame so far"
        ));
    }
    if a.frames.iter().any(|f| f != "base") {
        return Err("resize changes the sprite's own frame (base)".into());
    }
    let Current::Tiles {
        per_row,
        size: (sw, sh),
        pixels,
        ..
    } = &t.current
    else {
        return Err("not a tiles slot".into());
    };
    let before = sprite_pic(t)?;
    let free = crate::resize::free_tiles(t, assets_now)?;
    let list = crate::resize::tile_list(&sp.tiles, &free, rw, rh)?;
    let frame = Frame::grid("base", &list, rw, &[]);
    // The target as it will be: the model's sprite with the new tiles.
    let mut nt = t.clone();
    if let Some(s) = nt
        .model
        .entities
        .get_mut(entity)
        .and_then(|e| e.core.sprite.as_mut())
    {
        s.tiles = list.clone();
        s.width = rw;
    }
    if let Some(s) = nt.sprite.as_mut() {
        s.tiles = list.clone();
        s.width = rw;
    }
    let grown = crate::resize::resized(&before, rw, rh);
    let colors_now = critic::subject(config, t, &Candidate::Pixels(before.px.clone()), None)
        .map(|s| sprite_colors(&s))?;
    let mut now = grown.clone();
    let mut steps = vec![json!({ "op": "resize", "changed": 0 })];
    for (i, op) in a.ops.iter().enumerate() {
        let at = |e: String| format!("op {i} ({}): {e}", op.name());
        let src = sources(
            config,
            &nt,
            op,
            colors_now,
            std::slice::from_ref(&frame),
            pixels,
            *sw,
            *per_row,
        )
        .map_err(at)?;
        let refs: Vec<&Pic> = src.iter().collect();
        let next = sprite_ops::apply(&now, op, COLORS, &refs).map_err(at)?;
        steps.push(json!({ "op": op.name(), "changed": now.diff(&next).len() }));
        now = next;
    }
    let after = now;
    let mut sheet = pixels.clone();
    sprite_set::write(&frame, &after, &mut sheet, *sw, *per_row);
    let tile_edits: Vec<TileEdit> = list
        .iter()
        .filter_map(|&n| {
            let b = sprite_set::tile_digits(pixels, *sw, *per_row, n);
            let af = sprite_set::tile_digits(&sheet, *sw, *per_row, n);
            (b != af).then_some(TileEdit {
                tile: n,
                before: b,
                after: af,
            })
        })
        .collect();
    let subject = critic::subject(
        config,
        &nt,
        &Candidate::Pixels(after.px.clone()),
        a.prompt.as_deref(),
    )?;
    let (review, mode) = critic_review(a, &subject)?;
    let facing = sp.flip_when_facing_left;
    let png = preview_png(&grown, &after, sprite_colors(&subject), facing);
    let body = body_checks(&nt, &grown, &after);
    let mut out = json!({
        "copy": a.copy,
        "slot": t.slot,
        "entity": entity,
        "ops": steps,
        "resize": { "from": [sp.width, before.h / 8], "to": [rw, rh], "tiles": list, "free_tiles": free.len() },
        "pixels_changed": tile_edits.iter().map(|e| e.before.bytes().zip(e.after.bytes()).filter(|(x, y)| x != y).count()).sum::<usize>(),
        "ink": { "before": before.ink(), "after": after.ink() },
        "body": body,
        "checks": body.clone(),
        "critic": review.as_ref().map(critic::Review::json),
        "contexts": seen,
        "readiness": ready,
        "preview": {
            "panels": if facing { json!(["before (at the new size)", "after", "after facing left", "changed"]) } else { json!(["before (at the new size)", "after", "changed"]) },
            "scale": PREVIEW_SCALE,
            "png_base64": base64::engine::general_purpose::STANDARD.encode(&png),
        },
        "after": after.rows(),
    });
    let new_sheet = gp_assets::chr::write_png(*sw, *sh, &sheet);
    let context = if a.in_context != Some(false) {
        draw_in_context(
            &nt,
            assets_now,
            &t.asset.file(),
            &new_sheet,
            entity,
            &out["contexts"].clone(),
            &mut out,
        )?
    } else {
        None
    };
    if let Some(r) = enforce(a, &review, mode, &mut out) {
        return Ok(r);
    }
    let entry = Entry {
        seq: edit_log::next_seq(&t.dir)?,
        slot: t.slot.clone(),
        entity: entity.to_owned(),
        ops: a.ops.clone(),
        palette_ops: Vec::new(),
        prompt: a.prompt.clone(),
        before: grown.rows(),
        after: after.rows(),
        state_before: t.state.clone(),
        frames: vec!["base".into()],
        tiles: tile_edits,
        palettes: None,
        pictures: Vec::new(),
        editor: None,
        sprite: Some(crate::resize::SpriteChange {
            before: (sp.tiles.clone(), sp.width),
            after: (list, rw),
        }),
        status: EditStatus::Placed,
        decided_by: None,
        tile: None,
    };
    record(
        config,
        a,
        t,
        entry,
        &mut out,
        &png,
        context.as_deref(),
        Content::Pixels(sheet),
    )
}

/// The model's first palettes slot.
pub(crate) fn palettes_slot(model: &gp_model::Model) -> Option<String> {
    model
        .assets
        .iter()
        .find(|(_, s)| s.kind == AssetKind::Palettes)
        .map(|(n, _)| n.clone())
}

/// Two panels of the same picture in two sets of colors, `PREVIEW_SCALE`
/// times over, as an RGB PNG: a palette edit's before and after.
pub(crate) fn colors_preview(p: &Pic, before: [[u8; 3]; 4], after: [[u8; 3]; 4]) -> Vec<u8> {
    let s = PREVIEW_SCALE;
    let (pw, ph) = (p.w * s, p.h * s);
    let (w, h) = (2 * pw + 3 * s, ph + 2 * s);
    let mut px = vec![0x60u8; (w * h * 3) as usize];
    for (k, colors) in [before, after].iter().enumerate() {
        let ox = s + k as u32 * (pw + s);
        for y in 0..ph {
            for x in 0..pw {
                let c = colors[p.px[((y / s) * p.w + x / s) as usize] as usize % 4];
                let i = (((s + y) * w + ox + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&c);
            }
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().expect("PNG header into memory");
    wr.write_image_data(&px).expect("PNG data into memory");
    wr.finish().expect("PNG end into memory");
    out
}

/// The slot's palettes after `a`'s palette ops on palette `pi`, or, with
/// `same_role`, on every palette the slot gives the same role.
pub(crate) fn palettes_after(
    model: &gp_model::Model,
    slot: &str,
    old: &[[u8; 4]],
    pi: usize,
    a: &EditArgs,
) -> Result<Vec<[u8; 4]>, String> {
    let roles = model
        .assets
        .get(slot)
        .map(|s| s.item_roles.clone())
        .unwrap_or_default();
    let which: Vec<usize> = match (a.same_role, roles.get(pi)) {
        (true, Some(role)) => (0..old.len())
            .filter(|i| roles.get(*i) == Some(role))
            .collect(),
        (true, None) => {
            return Err(format!(
                "slot `{slot}` gives palette {pi} no role (item_roles), so same_role has nothing to match"
            ));
        }
        (false, _) => vec![pi],
    };
    let mut new = old.to_vec();
    for &i in &which {
        for (k, op) in a.palette_ops.iter().enumerate() {
            let other = match op {
                PaletteOp::CopyFrom { entity: o } => {
                    let osp = model
                        .entities
                        .get(o)
                        .and_then(|e| e.core.sprite.as_ref())
                        .ok_or(format!("entity `{o}` has no sprite"))?;
                    Some(*old.get(osp.palette as usize).ok_or(format!(
                        "`{o}`'s palette {} is past the slot's {}",
                        osp.palette,
                        old.len()
                    ))?)
                }
                _ => None,
            };
            new[i] = apply_palette(new[i], op, other)
                .map_err(|e| format!("palette op {k} ({}): {e}", op.name()))?;
        }
    }
    Ok(new)
}

fn palette_edit(config: &Config, a: &EditArgs, entity: &str) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let text = fs::read_to_string(dir.join(gp_model::MODEL_FILE))
        .map_err(|e| format!("reading {}: {e}", gp_model::MODEL_FILE))?;
    let model = gp_model::Model::parse(&text).map_err(|e| e.to_string())?;
    let slot = match &a.slot {
        Some(s) => s.clone(),
        None => palettes_slot(&model).ok_or("the model has no palettes slot")?,
    };
    let t = place::target(config, &a.copy, &slot, Some(entity))?;
    let Current::Palettes(old) = &t.current else {
        return Err(format!("slot `{slot}` isn't a palettes slot"));
    };
    let sp = t
        .sprite
        .clone()
        .ok_or(format!("`{entity}` has no sprite"))?;
    let pi = sp.palette as usize;
    let (seen, ready, assets_now) = match gate(config, &t, entity)? {
        Ok(x) => x,
        Err(refused) => return Ok(refused),
    };
    let new = palettes_after(&model, &slot, old, pi, a)?;
    let changed: Vec<usize> = (0..old.len()).filter(|&i| old[i] != new[i]).collect();
    let shared_with: Vec<&String> = model
        .entities
        .iter()
        .filter(|(n, e)| {
            n.as_str() != entity
                && e.core
                    .sprite
                    .as_ref()
                    .is_some_and(|s| changed.contains(&(s.palette as usize)))
        })
        .map(|(n, _)| n)
        .collect();
    let subject = critic::subject(
        config,
        &t,
        &Candidate::Palettes(vec![new[pi]]),
        a.prompt.as_deref(),
    )?;
    let (review, mode) = critic_review(a, &subject)?;
    // The sprite in both sets of colors.
    let tt = place::target(config, &a.copy, &sp.slot, Some(entity))?;
    let pic = sprite_pic(&tt)?;
    let rgb = |p: [u8; 4]| {
        let mut c = p.map(critic::nes_rgb);
        c[0] = critic::nes_rgb(old.first().map_or(0x0F, |b| b[0]));
        c
    };
    let png = colors_preview(&pic, rgb(old[pi]), rgb(new[pi]));
    let mut out = json!({
        "copy": a.copy,
        "slot": slot,
        "entity": entity,
        "palette": pi,
        "palettes_changed": changed,
        "shared_with": shared_with,
        "colors": { "before": old[pi], "after": new[pi] },
        "pixels_changed": 0,
        "checks": [],
        "critic": review.as_ref().map(critic::Review::json),
        "contexts": seen,
        "readiness": ready,
        "preview": {
            "panels": ["before", "after"],
            "scale": PREVIEW_SCALE,
            "png_base64": base64::engine::general_purpose::STANDARD.encode(&png),
        },
    });
    let by = edit_label(
        &a.palette_ops
            .iter()
            .map(PaletteOp::name)
            .collect::<Vec<_>>(),
    );
    let file_text = place::write_palettes(t.asset.name(), &new, &by);
    let context = if !changed.is_empty() && a.in_context != Some(false) {
        draw_in_context(
            &t,
            &assets_now,
            &t.asset.file(),
            file_text.as_bytes(),
            entity,
            &out["contexts"].clone(),
            &mut out,
        )?
    } else {
        None
    };
    if let Some(r) = enforce(a, &review, mode, &mut out) {
        return Ok(r);
    }
    if changed.is_empty() {
        out["ok"] = json!(true);
        out["written"] = json!(false);
        out["note"] = json!("the palette ops changed no color, so nothing was placed");
        return Ok(Reply::ok(out));
    }
    let entry = Entry {
        seq: edit_log::next_seq(&t.dir)?,
        slot: slot.clone(),
        entity: entity.to_owned(),
        ops: Vec::new(),
        palette_ops: a.palette_ops.clone(),
        prompt: a.prompt.clone(),
        before: Vec::new(),
        after: Vec::new(),
        state_before: t.state.clone(),
        frames: Vec::new(),
        tiles: Vec::new(),
        palettes: Some(PaletteChange {
            before: old.clone(),
            after: new.clone(),
        }),
        pictures: Vec::new(),
        editor: None,
        sprite: None,
        status: EditStatus::Placed,
        decided_by: None,
        tile: None,
    };
    record(
        config,
        a,
        &t,
        entry,
        &mut out,
        &png,
        context.as_deref(),
        Content::Palettes(new.iter().map(|p| p.to_vec()).collect()),
    )
}

/// An edit made before the history kept tiles (one frame, the sprite's
/// own): its tiles, from the rows.
fn legacy_tiles(t: &Target, e: &Entry) -> Result<Vec<TileEdit>, String> {
    let sp = t.sprite.as_ref().ok_or("the edit's entity has no sprite")?;
    let (b, a) = (Pic::from_rows(&e.before)?, Pic::from_rows(&e.after)?);
    let w = sp.width.max(1);
    let digits = |p: &Pic, k: u32| {
        let (sx, sy) = ((k % w) * 8, (k / w) * 8);
        let mut s = String::with_capacity(64);
        for y in 0..8 {
            for x in 0..8 {
                s.push(char::from(
                    b'0' + p.px[((sy + y) * p.w + sx + x) as usize].min(9),
                ));
            }
        }
        s
    };
    Ok(sp
        .tiles
        .iter()
        .enumerate()
        .filter_map(|(k, &n)| {
            let (bd, ad) = (digits(&b, k as u32), digits(&a, k as u32));
            (bd != ad).then_some(TileEdit {
                tile: n,
                before: bd,
                after: ad,
            })
        })
        .collect())
}

/// Writes an edit into the copy, forward (its after) or back (its before),
/// once what's there now is what the other side says. Err(Ok(why)) when
/// it isn't: something changed those tiles since.
fn put(
    config: &Config,
    copy: &str,
    e: &Entry,
    forward: bool,
    p: &Provenance,
) -> Result<Result<Reply, String>, String> {
    let t = place::target(config, copy, &e.slot, e.entity_ref())?;
    match &t.current {
        Current::Tiles {
            per_row,
            size,
            pixels,
            ..
        } => {
            let tiles = if e.tiles.is_empty() {
                legacy_tiles(&t, e)?
            } else {
                e.tiles.clone()
            };
            let mut sheet = pixels.clone();
            for te in &tiles {
                let now = sprite_set::tile_digits(pixels, size.0, *per_row, te.tile);
                let (want, new) = if forward {
                    (&te.before, &te.after)
                } else {
                    (&te.after, &te.before)
                };
                if &now != want {
                    return Ok(Err(format!(
                        "tile {} changed since edit {} (by another edit, a tool or by hand)",
                        te.tile, e.seq
                    )));
                }
                sprite_set::put_tile(&mut sheet, size.0, *per_row, te.tile, new)?;
            }
            Ok(Ok(place_with_sprite(
                config,
                copy,
                &t,
                e,
                forward,
                p,
                Content::Pixels(sheet),
            )?))
        }
        Current::Palettes(now) => {
            let pc = e.palettes.as_ref().ok_or("the edit has no palettes")?;
            let (want, new) = if forward {
                (&pc.before, &pc.after)
            } else {
                (&pc.after, &pc.before)
            };
            if now != want {
                return Ok(Err(format!(
                    "the palettes changed since edit {} (by another edit, a tool or by hand)",
                    e.seq
                )));
            }
            Ok(Ok(place::place_content(
                config,
                &slot_target(&t),
                p,
                Content::Palettes(new.iter().map(|c| c.to_vec()).collect()),
            )?))
        }
    }
}

/// Places content; for a resize, with the model's sprite set to the
/// edit's tiles and width first (forward) or after (back), and put back if
/// the placement doesn't write.
fn place_with_sprite(
    config: &Config,
    copy: &str,
    t: &Target,
    e: &Entry,
    forward: bool,
    p: &Provenance,
    content: Content,
) -> Result<Reply, String> {
    let Some(sc) = &e.sprite else {
        return place::place_content(config, &slot_target(t), p, content);
    };
    let (to, from) = if forward {
        (&sc.after, &sc.before)
    } else {
        (&sc.before, &sc.after)
    };
    crate::resize::set_sprite(&t.dir, &e.entity, &to.0, to.1)?;
    let r = place::target(config, copy, &e.slot, e.entity_ref())
        .and_then(|t2| place::place_content(config, &slot_target(&t2), p, content));
    if !matches!(&r, Ok(r) if r.json["written"] == json!(true)) {
        crate::resize::set_sprite(&t.dir, &e.entity, &from.0, from.1)?;
    }
    r
}

/// Takes the edit at `idx` out of the history by writing its tiles back.
/// The newest edit puts back the slot's state exactly; an older one leaves
/// the slot replaced, with a note. The entry goes to the redo list as
/// undone, or to the proposals as rejected.
fn revert(
    config: &Config,
    copy: &str,
    dir: &Path,
    mut history: Vec<Entry>,
    idx: usize,
    status: EditStatus,
    by: Option<&str>,
) -> Result<(Reply, Entry), String> {
    let e = history[idx].clone();
    let newest = idx + 1 == history.len();
    let t = place::target(config, copy, &e.slot, e.entity_ref())?;
    let p = if newest {
        let s = &e.state_before;
        Provenance {
            status: s.status,
            origin: s.origin,
            license: s.license.clone(),
            generator: s.generator.clone(),
            author: s.author.clone(),
            note: s.note.clone(),
        }
    } else {
        Provenance {
            status: Status::Replaced,
            origin: t.state.origin,
            license: t.state.license.clone(),
            generator: t.state.generator.clone(),
            author: t.state.author.clone(),
            note: Some(format!("edit {} undone", e.seq)),
        }
    };
    let placed = match put(config, copy, &e, false, &p)? {
        Ok(r) => r,
        Err(why) => {
            return Err(format!(
                "{why}, so undo would lose that; place the picture you want with assets_place"
            ));
        }
    };
    if placed.json["written"] != json!(true) {
        return Ok((placed, e));
    }
    let mut out = placed.json;
    if newest {
        // The state goes back as it was, the original's license and
        // status included, for every slot that shares the asset.
        let assets = dir.join(place::COPY_ASSETS);
        let mut state = State::read(&assets)?.ok_or("the copy's slots.toml is gone")?;
        let mut restored = e.state_before.clone();
        restored.sha256 = catalog::asset_digest(&assets, &t.asset)?;
        let map = place::read_map(&assets)?;
        let name = t.asset.name();
        for (other, asset) in catalog::slot_assets(&t.model, &map)? {
            if asset.name() == name {
                state.slots.insert(other.to_owned(), restored.clone());
            }
        }
        state.slots.insert(e.slot.clone(), restored.clone());
        fs::write(assets.join(catalog::STATE_FILE), state.to_toml())
            .map_err(|e| format!("writing {}: {e}", catalog::STATE_FILE))?;
        out["state"] = json!(restored);
    }
    history.remove(idx);
    edit_log::write_history(dir, &history)?;
    let mut done = e.clone();
    done.status = status;
    done.decided_by = by.map(str::to_owned);
    if status == EditStatus::Undone {
        let mut redo = edit_log::read_redo(dir)?;
        redo.push(done.clone());
        edit_log::write_redo(dir, &redo)?;
    } else {
        edit_log::write_proposal(dir, &done)?;
    }
    out["undone"] = json!({ "seq": e.seq, "ops": e.op_names() });
    out["item"] = json!(e.item(copy));
    out["status"] = json!(status.name());
    out["history"] = json!({ "depth": history.len() });
    if !e.before.is_empty() {
        let (now, back) = (Pic::from_rows(&e.after)?, Pic::from_rows(&e.before)?);
        let colors = critic::subject(config, &t, &Candidate::Pixels(back.px.clone()), None)
            .map(|s| sprite_colors(&s))
            .unwrap_or([[0; 3]; 4]);
        let facing = t.sprite.as_ref().is_some_and(|s| s.flip_when_facing_left);
        if (now.w, now.h) == (back.w, back.h) {
            out["preview"] = json!({
                "png_base64": base64::engine::general_purpose::STANDARD.encode(preview_png(&now, &back, colors, facing)),
                "scale": PREVIEW_SCALE,
            });
        }
    }
    Ok((Reply::ok(out), done))
}

/// Undoes the newest placed edit, or the one `seq` names when no later
/// edit changed its tiles.
fn undo(config: &Config, a: &EditArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let history = read_history(&dir)?;
    let idx = match a.seq {
        None => history
            .len()
            .checked_sub(1)
            .ok_or("there's no edit to undo")?,
        Some(s) => match history.iter().position(|e| e.seq == s) {
            Some(i) => i,
            None => {
                return Err(match edit_log::find(&dir, s)? {
                    Some(e) => format!("edit {s} is {}, not placed", e.status.name()),
                    None => format!("there's no edit {s}"),
                });
            }
        },
    };
    let e = &history[idx];
    if a.seq.is_none()
        && (a.entity.as_ref().is_some_and(|x| *x != e.entity)
            || a.slot.as_ref().is_some_and(|x| *x != e.slot))
    {
        return Err(format!(
            "the last edit was to {}'s {}; undo goes back one edit at a time (or give seq)",
            e.entity, e.slot
        ));
    }
    if let Some(l) = history[idx + 1..].iter().find(|l| l.overlaps(e)) {
        return Err(format!(
            "edit {} changed the same tiles since edit {}; undo it first",
            l.seq, e.seq
        ));
    }
    let (r, _) = revert(
        config,
        &a.copy,
        &dir,
        history,
        idx,
        EditStatus::Undone,
        None,
    )?;
    Ok(r)
}

/// Puts back the edit undone last.
fn redo(config: &Config, a: &EditArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let mut redo = edit_log::read_redo(&dir)?;
    let mut e = redo.pop().ok_or("there's nothing to redo")?;
    let t = place::target(config, &a.copy, &e.slot, e.entity_ref())?;
    let state_before = t.state.clone();
    let p = edit_provenance(&t, &e, a.author.as_deref());
    let placed = match put(config, &a.copy, &e, true, &p)? {
        Ok(r) => r,
        Err(why) => return Err(format!("{why}, so redo would lose that")),
    };
    if placed.json["written"] != json!(true) {
        return Ok(placed);
    }
    e.status = EditStatus::Placed;
    e.state_before = state_before;
    let mut history = read_history(&dir)?;
    history.push(e.clone());
    edit_log::write_history(&dir, &history)?;
    edit_log::write_redo(&dir, &redo)?;
    let mut out = placed.json;
    out["redone"] = json!({ "seq": e.seq, "ops": e.op_names() });
    out["item"] = json!(e.item(&a.copy));
    out["status"] = json!("placed");
    out["history"] = json!({ "depth": history.len(), "redo": redo.len() });
    Ok(Reply::ok(out))
}

/// Accepts or rejects an edit. A proposal accepted is placed with every
/// check; rejected, it's dropped. A placed edit accepted is kept; rejected,
/// it's undone (when no later edit changed its tiles). `by` is who decided:
/// `person` or `agent`. The caller holds the copy (`crate::CopyLock`), as
/// `tools::call` does.
pub fn decide(
    config: &Config,
    copy: &str,
    seq: u64,
    accept: bool,
    by: &str,
) -> Result<Reply, String> {
    let dir = config.existing_copy(copy)?;
    let mut history = read_history(&dir)?;
    if let Some(idx) = history.iter().position(|e| e.seq == seq) {
        if accept && history[idx].status == EditStatus::Accepted {
            let e = &history[idx];
            return Ok(Reply::ok(json!({
                "ok": true, "written": false, "item": e.item(copy), "status": "accepted",
                "decided_by": e.decided_by, "note": "accepted already",
            })));
        }
        if accept {
            history[idx].status = EditStatus::Accepted;
            history[idx].decided_by = Some(by.to_owned());
            edit_log::write_history(&dir, &history)?;
            let e = history[idx].clone();
            return Ok(Reply::ok(
                json!({ "ok": true, "written": false, "item": e.item(copy), "status": "accepted", "decided_by": by }),
            ));
        }
        if let Some(l) = history[idx + 1..]
            .iter()
            .find(|l| l.overlaps(&history[idx]))
        {
            return Err(format!(
                "edit {} changed the same tiles since edit {seq}; reject or undo it first",
                l.seq
            ));
        }
        let (mut r, _) = revert(
            config,
            copy,
            &dir,
            history,
            idx,
            EditStatus::Rejected,
            Some(by),
        )?;
        r.json["decided_by"] = json!(by);
        return Ok(r);
    }
    let mut e = match edit_log::read_proposal(&dir, seq)? {
        Some(e) if e.status == EditStatus::Proposed => e,
        Some(e) => return Err(format!("edit {seq} is {} already", e.status.name())),
        None => {
            return Err(match edit_log::find(&dir, seq)? {
                Some(e) => format!(
                    "edit {seq} is {}; redo it to place it again",
                    e.status.name()
                ),
                None => format!("there's no edit {seq}"),
            });
        }
    };
    if !accept {
        e.status = EditStatus::Rejected;
        e.decided_by = Some(by.to_owned());
        edit_log::write_proposal(&dir, &e)?;
        return Ok(Reply::ok(
            json!({ "ok": true, "written": false, "item": e.item(copy), "status": "rejected", "decided_by": by }),
        ));
    }
    let t = place::target(config, copy, &e.slot, e.entity_ref())?;
    let state_before = t.state.clone();
    let p = edit_provenance(&t, &e, Some(by));
    let placed = match put(config, copy, &e, true, &p)? {
        Ok(r) => r,
        Err(why) => {
            return Ok(Reply::refused(json!({
                "ok": false, "written": false, "item": e.item(copy),
                "error": format!("{why}; the proposal was made from art that's gone, so ask for the edit again"),
            })));
        }
    };
    let mut out = placed.json.clone();
    out["item"] = json!(e.item(copy));
    if placed.json["written"] == json!(true) {
        e.status = EditStatus::Accepted;
        e.decided_by = Some(by.to_owned());
        e.state_before = state_before;
        history.push(e.clone());
        edit_log::write_history(&dir, &history)?;
        edit_log::write_redo(&dir, &[])?;
        edit_log::write_proposal(&dir, &e)?;
        out["status"] = json!("accepted");
        out["decided_by"] = json!(by);
        out["history"] = json!({ "seq": e.seq, "depth": history.len() });
    }
    Ok(Reply {
        json: out,
        is_error: placed.is_error,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryArgs {
    pub copy: String,
}

/// `assets_history`: a copy's art edits and their status, so an agent sees
/// what a person accepted or rejected.
pub fn history(config: &Config, a: HistoryArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let s = |v: Vec<Entry>| v.iter().map(Entry::summary).collect::<Vec<_>>();
    let proposals = edit_log::proposals(&dir)?;
    let open: Vec<u64> = proposals
        .iter()
        .filter(|e| e.status == EditStatus::Proposed)
        .map(|e| e.seq)
        .collect();
    Ok(Reply::ok(json!({
        "copy": a.copy,
        "placed": s(read_history(&dir)?),
        "undone": s(edit_log::read_redo(&dir)?),
        "proposals": s(proposals),
        "waiting": open,
        "next_seq": edit_log::next_seq(&dir)?,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_draws_each_panel_at_its_scale() {
        let before = Pic::new(8, 8, vec![1; 64]).unwrap();
        let mut after = before.clone();
        after.px[0] = 2;
        let png = preview_png(&before, &after, [[0; 3], [1; 3], [2; 3], [3; 3]], true);
        let dec = png::Decoder::new(std::io::Cursor::new(png));
        let r = dec.read_info().unwrap();
        let info = r.info();
        // Four panels of 64 pixels and five gaps of 8.
        assert_eq!((info.width, info.height), (4 * 64 + 5 * 8, 64 + 16));
        let c = colors_preview(&before, [[0; 3]; 4], [[9; 3]; 4]);
        let r = png::Decoder::new(std::io::Cursor::new(c))
            .read_info()
            .unwrap();
        assert_eq!((r.info().width, r.info().height), (2 * 64 + 24, 64 + 16));
    }
}
