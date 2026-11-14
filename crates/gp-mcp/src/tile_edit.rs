//! `assets_edit` with `tile`: edits of a level tile kind's look, for an
//! agent told "make the stone blocks orange" (docs/sprite-editing.md,
//! "Level tiles").
//!
//! A tile kind is an item of a metatiles slot: four tiles, named by the
//! asset map (`names`, the same names the lifted metatile files carry and
//! point and tell shows), with the slot's `item_roles` as its role. Its
//! picture is its four tiles in the level tiles slot, laid out 2 by 2; its
//! colors are the background palette the level picture draws it with.
//! Nothing here knows a game: every metatiles slot of every model with an
//! asset map has its kinds, found the same way.
//!
//! The edit reuses the sprite edit's machinery: the same deterministic
//! [`sprite_ops::Op`]s on the picture (the ones that read another entity's
//! sprite aside), the same [`PaletteOp`]s on the palette (with
//! `same_role`), the copy's history with proposals, accept, reject, undo
//! and redo (`crate::edit_log`, entries with `tile` set and no entity), the
//! close-up preview, and `assets_place`'s checks. What differs is where it
//! shows: [`crate::scene::tile_in_context`] replays the baselines and
//! draws crops around cells of the kind on the frames the camera shows
//! them, and `also_changes` names the other tile kinds and the sprites that
//! draw a changed tile or palette.

use std::fs;
use std::path::Path;

use base64::Engine as _;
use gp_assets::map::{Asset, TileOrder};
use gp_interp::Lifted;
use gp_model::Model;
use gp_model::model::AssetKind;
use serde_json::{Value, json};

use crate::asset_edit::{self, COLORS, EditArgs, MAX_OPS, PREVIEW_SCALE};
use crate::critic;
use crate::edit_log::{self, EditStatus, Entry, PaletteChange, PaletteOp, TileEdit};
use crate::place::{self, Content, Current, Target};
use crate::scene::{self, CellsAt};
use crate::sprite_ops::{self, Pic};
use crate::sprite_set::{self, Frame};
use crate::{Config, Reply};

/// The ops that read another entity's sprite or a file laid out as one,
/// which a tile kind has no use for.
const ENTITY_OPS: &[&str] = &["copy", "load", "match_style", "tween"];

/// One level tile kind: an item of a metatiles slot.
#[derive(Debug, Clone, PartialEq)]
pub struct TileKind {
    /// The metatiles slot and the item's index in it.
    pub slot: String,
    pub index: usize,
    /// The asset map's name, or `<slot> <index>` when it has none.
    pub name: String,
    pub named: bool,
    pub role: Option<String>,
    /// Its four tiles by rows: top left, top right, bottom left, bottom
    /// right.
    pub tiles: [u32; 4],
    /// The background palette the level picture draws it in.
    pub palette: u32,
    /// Where the levels draw it.
    pub cells: CellsAt,
}

impl TileKind {
    /// Its four tiles as point and tell writes them (`05060708`).
    pub fn sig(&self) -> String {
        self.tiles.iter().map(|t| format!("{t:02x}")).collect()
    }

    pub fn drawn(&self) -> usize {
        self.cells.values().map(Vec::len).sum()
    }

    fn frame(&self) -> Frame {
        Frame::grid("base", &self.tiles, 2, &[])
    }
}

/// The background palette a cell of a metatile grid is drawn in. The level
/// picture (`gp_look::present`, `grid_picture`) draws every cell of a
/// grid level in background palette 0, which is what the reference game
/// and every grid level lifted so far do; a per-cell palette (the NES
/// attribute tables) isn't in the model yet.
pub fn cell_palette(_model: &Model, _slot: &str, _index: usize) -> u32 {
    0
}

/// Every tile kind of the model's metatiles slots, from the lifted assets
/// in `assets`, with where each level's grid draws it.
pub fn tile_kinds(model: &Model, assets: &Path) -> Result<Vec<TileKind>, String> {
    let lifted = Lifted::read_dir(assets, model)?;
    let map = lifted.map()?;
    let asset_of = |slot: &str| -> Option<&Asset> {
        let from = model.assets.get(slot)?.from.as_ref()?;
        map.assets.iter().find(|a| a.name() == from.asset)
    };
    let mut out = Vec::new();
    for (slot, s) in &model.assets {
        if s.kind != AssetKind::Metatiles {
            continue;
        }
        let Some(
            asset @ Asset::Metatiles {
                name,
                names,
                tile_order,
                ..
            },
        ) = asset_of(slot)
        else {
            continue;
        };
        let bytes = lifted
            .decode(&map, name)?
            .remove(asset.name())
            .unwrap_or_default();
        // Where each level whose tileset this is draws each item.
        let mut grids: Vec<(String, u32, Vec<u8>)> = Vec::new();
        for (lname, level) in &model.levels {
            let c = &level.core;
            if c.tileset.as_deref() != Some(slot.as_str()) {
                continue;
            }
            let Some(g) = c.grid.as_deref() else {
                continue;
            };
            if let Some(Asset::Level {
                name: gname, width, ..
            }) = asset_of(g)
            {
                let cells = lifted
                    .decode(&map, gname)?
                    .remove(gname.as_str())
                    .unwrap_or_default();
                grids.push((lname.clone(), *width, cells));
            }
        }
        for (i, q) in bytes.as_chunks::<4>().0.iter().enumerate() {
            let tiles = match tile_order {
                TileOrder::Rows => [q[0], q[1], q[2], q[3]],
                TileOrder::Columns => [q[0], q[2], q[1], q[3]],
            }
            .map(u32::from);
            let mut cells = CellsAt::new();
            for (lname, width, grid) in &grids {
                let w = (*width).max(1) as usize;
                let at: Vec<[i64; 2]> = grid
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| usize::from(**c) == i)
                    .map(|(k, _)| [(k % w) as i64 * 16, (k / w) as i64 * 16])
                    .collect();
                if !at.is_empty() {
                    cells.insert(lname.clone(), at);
                }
            }
            let named = names.get(i).filter(|n| !n.trim().is_empty());
            out.push(TileKind {
                slot: slot.clone(),
                index: i,
                name: named.cloned().unwrap_or_else(|| format!("{slot} {i}")),
                named: named.is_some(),
                role: s.item_roles.get(i).cloned(),
                tiles,
                palette: cell_palette(model, slot, i),
                cells,
            });
        }
    }
    Ok(out)
}

/// The tile kind `want` names: its name (any case, an "a " or "the "
/// before it allowed), `<slot>[<index>]`, or its four tiles in hex.
pub fn find<'a>(kinds: &'a [TileKind], want: &str) -> Result<&'a TileKind, String> {
    let w = want.trim().to_lowercase();
    let bare = w
        .strip_prefix("the ")
        .or_else(|| w.strip_prefix("a "))
        .or_else(|| w.strip_prefix("an "))
        .unwrap_or(&w);
    let by_index = bare
        .strip_suffix(']')
        .and_then(|x| x.split_once('['))
        .and_then(|(s, i)| Some((s.to_owned(), i.parse::<usize>().ok()?)));
    kinds
        .iter()
        .find(|k| k.name.to_lowercase() == bare)
        .or_else(|| {
            by_index
                .as_ref()
                .and_then(|(s, i)| kinds.iter().find(|k| &k.slot == s && k.index == *i))
        })
        .or_else(|| kinds.iter().find(|k| k.sig() == bare))
        .ok_or_else(|| {
            if kinds.is_empty() {
                "the model has no metatiles slot with lifted tile kinds, so there's no level tile to edit".to_owned()
            } else {
                format!(
                    "no tile kind `{want}`; the model's are {}",
                    kinds
                        .iter()
                        .map(|k| format!("{} ({}[{}])", k.name, k.slot, k.index))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        })
}

/// The model's first palettes slot and its palettes in `assets`.
pub fn palettes_now(model: &Model, assets: &Path) -> Option<(String, Vec<[u8; 4]>)> {
    let slot = asset_edit::palettes_slot(model)?;
    let map = place::read_map(assets).ok()?;
    let slots = crate::catalog::slot_assets(model, &map).ok()?;
    let asset = slots.get(slot.as_str())?;
    let text = fs::read_to_string(assets.join(asset.file())).ok()?;
    Some((slot, place::read_palettes(&text).ok()?))
}

/// Whether two slots are filled from the same asset (or are one slot).
fn same_asset(model: &Model, a: &str, b: &str) -> bool {
    let from = |s: &str| model.assets.get(s).and_then(|x| x.from.clone());
    a == b || from(a).is_some_and(|f| Some(f) == from(b))
}

/// The entities whose sprites draw from `slot` (or a slot of the same
/// asset) with one of `tiles`.
fn sprites_drawing(model: &Model, slot: &str, tiles: &[u32]) -> Vec<String> {
    model
        .entities
        .iter()
        .filter(|(_, e)| {
            e.core.sprite.as_ref().is_some_and(|s| {
                same_asset(model, &s.slot, slot)
                    && (s.tiles.iter().any(|t| tiles.contains(t))
                        || s.clips.iter().any(|c| {
                            c.frames
                                .iter()
                                .any(|f| f.tiles.iter().any(|t| tiles.contains(t)))
                        }))
            })
        })
        .map(|(n, _)| n.clone())
        .collect()
}

/// A tile kind's facts in plain data: what point and tell's explanation
/// and the edit's reply name.
pub fn facts(
    model: &Model,
    kinds: &[TileKind],
    k: &TileKind,
    palettes: Option<&(String, Vec<[u8; 4]>)>,
) -> Value {
    let tiles_slot = gp_look::present::level_tiles_slot(model);
    let shares_tiles: Vec<&str> = kinds
        .iter()
        .filter(|o| o != &k && o.tiles.iter().any(|t| k.tiles.contains(t)))
        .map(|o| o.name.as_str())
        .collect();
    let shares_palette: Vec<&str> = kinds
        .iter()
        .filter(|o| o != &k && o.palette == k.palette)
        .map(|o| o.name.as_str())
        .collect();
    let palette = palettes.map(|(slot, p)| {
        let roles = model
            .assets
            .get(slot)
            .map(|s| s.item_roles.clone())
            .unwrap_or_default();
        let role = roles.get(k.palette as usize).cloned();
        json!({
            "slot": slot,
            "index": k.palette,
            "colors": p.get(k.palette as usize),
            "role": role,
            "same_role": role.as_ref().map(|r| (0..p.len()).filter(|i| roles.get(*i) == Some(r)).collect::<Vec<_>>()),
        })
    });
    json!({
        "tile": k.name,
        "slot": k.slot,
        "index": k.index,
        "role": k.role,
        "tiles": k.tiles,
        "tiles_hex": k.sig(),
        "tiles_slot": tiles_slot,
        "palette": palette,
        "drawn": k.cells.iter().map(|(l, c)| (l.clone(), json!(c.len()))).collect::<serde_json::Map<_, _>>(),
        "shares_tiles_with": shares_tiles,
        "shares_palette_with": shares_palette,
        "sprites_sharing_tiles": tiles_slot.as_deref().map(|s| sprites_drawing(model, s, &k.tiles)).unwrap_or_default(),
        "tool": "assets_edit",
        "args": { "tile": k.name },
    })
}

/// The tile kind `want` names in a model with lifted assets in `assets`,
/// with its facts: for `gp request explain`.
pub fn describe(model: &Model, assets: &Path, want: &str) -> Result<Value, String> {
    let kinds = tile_kinds(model, assets)?;
    let k = find(&kinds, want)?;
    Ok(facts(
        model,
        &kinds,
        k,
        palettes_now(model, assets).as_ref(),
    ))
}

fn item(name: &str, ok: bool, detail: String, fill: Option<&str>) -> Value {
    let mut v =
        json!({ "item": name, "status": if ok { "ok" } else { "missing" }, "detail": detail });
    if let (false, Some(f)) = (ok, fill) {
        v["fill"] = json!(f);
    }
    v
}

/// Whether the model knows the tile kind well enough to edit it: named by
/// the asset map, its tiles in the level tiles slot, its palette in the
/// palettes slot, drawn by a level, and seen on the recorded baselines'
/// frames (`seen`, from [`scene::tile_in_context`]).
pub fn readiness(model: &Model, k: &TileKind, tiles_count: Option<u32>, seen: u64) -> Value {
    let tiles_slot = gp_look::present::level_tiles_slot(model);
    let palettes = model
        .assets
        .iter()
        .find(|(_, s)| s.kind == AssetKind::Palettes);
    let items = vec![
        item(
            "name",
            k.named,
            if k.named {
                format!("the asset map names {}[{}] \"{}\"", k.slot, k.index, k.name)
            } else {
                format!("the asset map gives {}[{}] no name", k.slot, k.index)
            },
            Some("name the metatiles in the asset map (names = [...]) and lift again"),
        ),
        match (&tiles_slot, tiles_count) {
            (Some(s), Some(n)) => {
                let past: Vec<u32> = k.tiles.iter().copied().filter(|t| *t >= n).collect();
                item(
                    "tiles",
                    past.is_empty(),
                    if past.is_empty() {
                        format!("tiles {} of `{s}`", k.sig())
                    } else {
                        format!("tiles {past:?} are past `{s}`'s {n}")
                    },
                    None,
                )
            }
            _ => item(
                "tiles",
                false,
                "no one tiles slot draws the levels (one slot, or the one no sprite uses)".into(),
                Some("give the level's tiles their own slot, or give `slot`"),
            ),
        },
        match palettes {
            Some((name, slot)) => {
                let n = slot.item_roles.len();
                let ok = n == 0 || (k.palette as usize) < n;
                item(
                    "palette",
                    ok,
                    format!("background palette {} of `{name}`", k.palette),
                    None,
                )
            }
            None => item(
                "palette",
                false,
                "the model has no palettes slot, so the tile's colors aren't known".into(),
                Some("lift the palettes and add a palettes slot"),
            ),
        },
        item(
            "drawn",
            k.drawn() > 0,
            if k.drawn() > 0 {
                format!("{} cells in the levels", k.drawn())
            } else {
                "no level's grid draws it".into()
            },
            Some("a level that uses it (the edit would show nowhere)"),
        ),
        item(
            "seen",
            seen > 0,
            format!("{seen} recorded frames show it on screen"),
            Some("record a baseline that passes it (gp capture)"),
        ),
    ];
    let ready = items.iter().all(|i| i["status"] == "ok");
    json!({ "ready": ready, "items": items })
}

fn refusal(k: &TileKind, ready: &Value) -> Reply {
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
    Reply::refused(json!({
        "ok": false,
        "written": false,
        "stage": "model",
        "error": format!("the model doesn't know the tile kind `{}` well enough to edit it: {}", k.name, missing.join("; ")),
        "readiness": ready,
    }))
}

/// RGB of a background palette: index 0 is the backdrop, color 0 of
/// palette 0, as the PPU draws it.
fn bg_rgb(palettes: &[[u8; 4]], p: [u8; 4]) -> [[u8; 3]; 4] {
    let mut c = p.map(critic::nes_rgb);
    c[0] = critic::nes_rgb(palettes.first().map_or(p[0], |b| b[0]));
    c
}

/// The edit drawn into the recorded frames, merged into `out` as
/// `in_context`; the shots' PNG is returned for the copy's previews.
fn draw(
    t: &Target,
    assets_now: &Path,
    file: &str,
    bytes: &[u8],
    k: &TileKind,
    out: &mut Value,
) -> Result<Option<Vec<u8>>, String> {
    let stage = asset_edit::stage_assets(t, assets_now, file, bytes)?;
    let r = scene::tile_in_context(t, assets_now, Some(&stage), &k.cells);
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

/// The copy, its model and assets now, and the tile kinds.
fn open(config: &Config, copy: &str) -> Result<(Model, std::path::PathBuf, Vec<TileKind>), String> {
    let dir = config.existing_copy(copy)?;
    let text = fs::read_to_string(dir.join(gp_model::MODEL_FILE))
        .map_err(|e| format!("reading {}: {e}", gp_model::MODEL_FILE))?;
    let model = Model::parse(&text).map_err(|e| e.to_string())?;
    let (assets_now, _) = place::current_assets(config, &dir, &model)?;
    let kinds = tile_kinds(&model, &assets_now)?;
    Ok((model, assets_now, kinds))
}

/// `assets_edit` with `tile` (the caller checked the modes and that no
/// entity is given).
pub fn edit(config: &Config, a: &EditArgs, want: &str) -> Result<Reply, String> {
    let n = a.ops.len() + a.palette_ops.len();
    if n > MAX_OPS {
        return Err(format!("{n} ops; one call runs at most {MAX_OPS}"));
    }
    if a.resize.is_some() {
        return Err("resize takes an entity's sprite; a tile kind is 16 by 16".into());
    }
    if let Some(op) = a.ops.iter().find(|o| ENTITY_OPS.contains(&o.name())) {
        return Err(format!(
            "op {} reads another entity's sprite; a tile kind takes recolor, flip, mirror, shift, outline, fill, pixels and pictures",
            op.name()
        ));
    }
    let (model, assets_now, kinds) = open(config, &a.copy)?;
    let k = find(&kinds, want)?.clone();
    match (
        a.ops.is_empty() && a.pictures.is_empty(),
        a.palette_ops.is_empty(),
    ) {
        (true, true) => Err("give at least one op".into()),
        (false, false) => Err(
            "give ops or palette_ops, not both: pixels and colors are in different slots".into(),
        ),
        (_, false) => palette_edit(config, a, &model, &kinds, &k, &assets_now),
        (false, true) => pixel_edit(config, a, &model, &kinds, &k, &assets_now),
    }
}

fn entry(a: &EditArgs, t: &Target, k: &TileKind, slot: &str) -> Result<Entry, String> {
    Ok(Entry {
        seq: edit_log::next_seq(&t.dir)?,
        slot: slot.to_owned(),
        entity: String::new(),
        ops: Vec::new(),
        palette_ops: Vec::new(),
        prompt: a.prompt.clone(),
        before: Vec::new(),
        after: Vec::new(),
        state_before: t.state.clone(),
        frames: Vec::new(),
        tiles: Vec::new(),
        palettes: None,
        pictures: Vec::new(),
        editor: a.editor.clone(),
        sprite: None,
        status: EditStatus::Placed,
        decided_by: None,
        run: a.run.clone(),
        tile: Some(k.name.clone()),
    })
}

fn pixel_edit(
    config: &Config,
    a: &EditArgs,
    model: &Model,
    kinds: &[TileKind],
    k: &TileKind,
    assets_now: &Path,
) -> Result<Reply, String> {
    let slot = match &a.slot {
        Some(s) => s.clone(),
        None => gp_look::present::level_tiles_slot(model).ok_or(
            "no one tiles slot draws the levels (one slot, or the one no sprite uses); give slot",
        )?,
    };
    let t = place::target(config, &a.copy, &slot, None)?;
    let Current::Tiles {
        tiles: count,
        per_row,
        size: (sw, sh),
        pixels,
    } = &t.current
    else {
        return Err(format!("slot `{slot}` isn't a tiles slot"));
    };
    let (sw, sh, per_row) = (*sw, *sh, *per_row);
    let (_, seen) = scene::tile_in_context(&t, assets_now, None, &k.cells)?;
    let ready = readiness(model, k, Some(*count), seen["seen"].as_u64().unwrap_or(0));
    if ready["ready"] != json!(true) {
        return Ok(refusal(k, &ready));
    }
    let frame = k.frame();
    let before = sprite_set::read(&frame, pixels, sw, per_row)?;
    let mut now = before.clone();
    let mut steps = Vec::new();
    if let Some(rows) = a.pictures.get("base").or_else(|| a.pictures.get(&k.name)) {
        let p = Pic::from_rows(rows).map_err(|e| format!("the picture: {e}"))?;
        if (p.w, p.h) != (before.w, before.h) {
            return Err(format!(
                "the picture is {}x{}, the tile kind 16x16",
                p.w, p.h
            ));
        }
        if let Some(v) = p.px.iter().find(|&&v| v >= COLORS) {
            return Err(format!("the picture has color index {v}"));
        }
        steps.push(json!({ "op": "picture", "changed": now.diff(&p).len() }));
        now = p;
    }
    for (i, op) in a.ops.iter().enumerate() {
        let next = sprite_ops::apply(&now, op, COLORS, &[])
            .map_err(|e| format!("op {i} ({}): {e}", op.name()))?;
        steps.push(json!({ "op": op.name(), "changed": now.diff(&next).len() }));
        now = next;
    }
    let mut sheet = pixels.clone();
    sprite_set::write(&frame, &now, &mut sheet, sw, per_row);
    // A kind that draws one tile twice (a sky of four alike) must draw it
    // alike after the ops.
    let bad = sprite_set::holds(&[(&frame, &now)], &sheet, sw, per_row)?;
    if !bad.is_empty() {
        return Ok(Reply::refused(json!({
            "ok": false,
            "written": false,
            "stage": "frames",
            "error": format!(
                "`{}` draws a tile more than once ({}) and the ops drew its copies differently; edit it so they agree (a recolor does)",
                k.name, k.sig()
            ),
        })));
    }
    let mut uniq: Vec<u32> = k.tiles.to_vec();
    uniq.sort_unstable();
    uniq.dedup();
    let tile_edits: Vec<TileEdit> = uniq
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
    let changed_tiles: Vec<u32> = tile_edits.iter().map(|e| e.tile).collect();
    let also_tiles: Vec<&str> = kinds
        .iter()
        .filter(|o| *o != k && o.tiles.iter().any(|t| changed_tiles.contains(t)))
        .map(|o| o.name.as_str())
        .collect();
    let also_sprites = sprites_drawing(model, &slot, &changed_tiles);
    let pal = palettes_now(model, assets_now);
    let colors = pal
        .as_ref()
        .and_then(|(_, p)| p.get(k.palette as usize).map(|c| bg_rgb(p, *c)))
        .unwrap_or([[0, 0, 0], [0x60; 3], [0xa0; 3], [0xff; 3]]);
    let png = asset_edit::preview_png(&before, &now, colors, false);
    let changed = before.diff(&now).len();
    let mut out = json!({
        "copy": a.copy,
        "slot": slot,
        "tile": facts(model, kinds, k, pal.as_ref()),
        "ops": steps,
        "frames": [{ "frame": "base", "tiles": k.tiles, "size": [16, 16], "changed": changed, "ops": steps }],
        "also_changes": also_tiles,
        "also_changes_sprites": also_sprites,
        "pixels_changed": changed,
        "ink": { "before": before.ink(), "after": now.ink() },
        "checks": [],
        "tiles": sprite_ops::tile_report(&now, 2),
        "critic": null,
        "contexts": seen,
        "readiness": ready,
        "preview": {
            "panels": ["before", "after", "changed"],
            "frame": "base",
            "scale": PREVIEW_SCALE,
            "png_base64": base64::engine::general_purpose::STANDARD.encode(&png),
        },
        "after": now.rows(),
    });
    let context = if changed > 0 && a.in_context != Some(false) {
        let new_sheet = gp_assets::chr::write_png(sw, sh, &sheet);
        draw(&t, assets_now, &t.asset.file(), &new_sheet, k, &mut out)?
    } else {
        None
    };
    if changed == 0 {
        out["ok"] = json!(true);
        out["written"] = json!(false);
        out["note"] = json!("the ops changed no pixel, so nothing was placed");
        return Ok(Reply::ok(out));
    }
    let mut e = entry(a, &t, k, &slot)?;
    e.ops = a.ops.clone();
    e.before = before.rows();
    e.after = now.rows();
    e.frames = vec!["base".into()];
    e.tiles = tile_edits;
    e.pictures = a
        .pictures
        .keys()
        .filter(|p| *p == "base" || **p == k.name)
        .cloned()
        .collect();
    asset_edit::record(
        config,
        a,
        &t,
        e,
        &mut out,
        &png,
        context.as_deref(),
        Content::Pixels(sheet),
    )
}

fn palette_edit(
    config: &Config,
    a: &EditArgs,
    model: &Model,
    kinds: &[TileKind],
    k: &TileKind,
    assets_now: &Path,
) -> Result<Reply, String> {
    let slot = match &a.slot {
        Some(s) => s.clone(),
        None => asset_edit::palettes_slot(model).ok_or("the model has no palettes slot")?,
    };
    let t = place::target(config, &a.copy, &slot, None)?;
    let Current::Palettes(old) = &t.current else {
        return Err(format!("slot `{slot}` isn't a palettes slot"));
    };
    let pi = k.palette as usize;
    if pi >= old.len() {
        return Err(format!("palette {pi} is past `{slot}`'s {}", old.len()));
    }
    let (_, seen) = scene::tile_in_context(&t, assets_now, None, &k.cells)?;
    let tiles_slot = gp_look::present::level_tiles_slot(model);
    let tt = match &tiles_slot {
        Some(s) => Some(place::target(config, &a.copy, s, None)?),
        None => None,
    };
    let count = tt.as_ref().and_then(|t| match &t.current {
        Current::Tiles { tiles, .. } => Some(*tiles),
        Current::Palettes(_) => None,
    });
    let ready = readiness(model, k, count, seen["seen"].as_u64().unwrap_or(0));
    if ready["ready"] != json!(true) {
        return Ok(refusal(k, &ready));
    }
    let new = asset_edit::palettes_after(model, &slot, old, pi, a)?;
    let changed: Vec<usize> = (0..old.len()).filter(|&i| old[i] != new[i]).collect();
    // Another tile kind changes when its palette changed in a color its
    // pixels use (the sky draws only the backdrop, so a new face color
    // leaves it alone).
    let sheet = tt.as_ref().and_then(|t| match &t.current {
        Current::Tiles {
            per_row,
            size,
            pixels,
            ..
        } => Some((pixels, size.0, *per_row)),
        Current::Palettes(_) => None,
    });
    let uses_changed = |o: &TileKind| -> bool {
        let p = o.palette as usize;
        if !changed.contains(&p) {
            return false;
        }
        let colors: Vec<u8> = (0..4u8)
            .filter(|&c| old[p][usize::from(c)] != new[p][usize::from(c)])
            .collect();
        match sheet {
            Some((px, w, per_row)) => sprite_set::read(&o.frame(), px, w, per_row)
                .map_or(true, |pic| pic.px.iter().any(|v| colors.contains(v))),
            None => true,
        }
    };
    let also_tiles: Vec<&str> = kinds
        .iter()
        .filter(|o| *o != k && uses_changed(o))
        .map(|o| o.name.as_str())
        .collect();
    let also_sprites: Vec<&String> = model
        .entities
        .iter()
        .filter(|(_, e)| {
            e.core
                .sprite
                .as_ref()
                .is_some_and(|s| changed.contains(&(s.palette as usize)))
        })
        .map(|(n, _)| n)
        .collect();
    // The tile kind in both sets of colors.
    let pic = match &tt {
        Some(t) => match &t.current {
            Current::Tiles {
                per_row,
                size,
                pixels,
                ..
            } => sprite_set::read(&k.frame(), pixels, size.0, *per_row)?,
            Current::Palettes(_) => Pic::new(16, 16, vec![0; 256])?,
        },
        None => Pic::new(16, 16, vec![0; 256])?,
    };
    let png = asset_edit::colors_preview(&pic, bg_rgb(old, old[pi]), bg_rgb(&new, new[pi]));
    let pal = Some((slot.clone(), old.clone()));
    let mut out = json!({
        "copy": a.copy,
        "slot": slot,
        "tile": facts(model, kinds, k, pal.as_ref()),
        "palette": pi,
        "palettes_changed": changed,
        "also_changes": also_tiles,
        "also_changes_sprites": also_sprites,
        "shared_with": also_sprites,
        "colors": { "before": old[pi], "after": new[pi] },
        "pixels_changed": 0,
        "checks": [],
        "critic": null,
        "contexts": seen,
        "readiness": ready,
        "preview": {
            "panels": ["before", "after"],
            "scale": PREVIEW_SCALE,
            "png_base64": base64::engine::general_purpose::STANDARD.encode(&png),
        },
    });
    let by = asset_edit::edit_label(
        &a.palette_ops
            .iter()
            .map(PaletteOp::name)
            .collect::<Vec<_>>(),
    );
    let file_text = place::write_palettes(t.asset.name(), &new, &by);
    let context = if !changed.is_empty() && a.in_context != Some(false) {
        draw(
            &t,
            assets_now,
            &t.asset.file(),
            file_text.as_bytes(),
            k,
            &mut out,
        )?
    } else {
        None
    };
    if changed.is_empty() {
        out["ok"] = json!(true);
        out["written"] = json!(false);
        out["note"] = json!("the palette ops changed no color, so nothing was placed");
        return Ok(Reply::ok(out));
    }
    let mut e = entry(a, &t, k, &slot)?;
    e.palette_ops = a.palette_ops.clone();
    e.palettes = Some(PaletteChange {
        before: old.clone(),
        after: new.clone(),
    });
    asset_edit::record(
        config,
        a,
        &t,
        e,
        &mut out,
        &png,
        context.as_deref(),
        Content::Palettes(new.iter().map(|p| p.to_vec()).collect()),
    )
}
