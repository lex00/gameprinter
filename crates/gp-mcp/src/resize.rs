//! Resizing a sprite within its slot's tile budget (docs/sprite-editing.md):
//! a sprite that grows takes tiles nothing draws, and the model's sprite
//! gets the new tile list and width. Presentation only: the body's size and
//! hitbox stay, so the simulation is the same.
//!
//! A tile is free when no entity's sprite set names it and no picture the
//! generator draws changes when it's painted: the level, the HUD and every
//! sprite, as `gp_look::present::art` builds them from the copy's
//! assets. That's found by painting the candidates and building the art
//! again, halving the set of candidates that changed something until each
//! used one is found, so a sheet with a few used blank tiles (a sky tile)
//! takes a few builds. Nothing here knows a game.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use gp_interp::{Interp, Lifted};
use serde::{Deserialize, Serialize};

use crate::place::{Current, Target};
use crate::sprite_ops::Pic;
use crate::sprite_set;

/// A sprite's tile list and width, before and after a resize, so undo and
/// redo can write the model back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteChange {
    pub before: (Vec<u32>, u32),
    pub after: (Vec<u32>, u32),
}

/// Every tile any entity's sprite set in `slot` names.
fn named_tiles(t: &Target, assets: &Path) -> BTreeSet<u32> {
    let set = sprite_set::read_set(&t.model, assets).ok().flatten();
    let mut out = BTreeSet::new();
    for name in t.model.entities.keys() {
        if let Ok((frames, _)) = sprite_set::frames(&t.model, name, &t.slot, set.as_ref()) {
            for f in frames {
                out.extend(f.tiles());
            }
        }
    }
    if let Some(set) = &set {
        for f in &set.frames {
            out.extend(
                f.pieces
                    .iter()
                    .filter(|p| p.slot.as_deref().is_none_or(|s| s == t.slot))
                    .map(|p| p.tile),
            );
        }
    }
    out
}

fn art_files(t: &Target, dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let lifted = Lifted::read_dir(dir, &t.model)?;
    let rt = Interp::load(&t.model, &lifted)?;
    let art = gp_look::present::art(&t.model, rt.spec(), dir)?;
    Ok(art.pictures.into_iter().map(|p| (p.file, p.png)).collect())
}

/// The tiles of the slot that are blank, named by no sprite set and drawn
/// by nothing, given the copy's assets in `assets`.
pub fn free_tiles(t: &Target, assets: &Path) -> Result<Vec<u32>, String> {
    let Current::Tiles {
        tiles,
        per_row,
        size,
        pixels,
    } = &t.current
    else {
        return Err("free tiles are a tiles slot's".into());
    };
    let named = named_tiles(t, assets);
    let blank = "0".repeat(64);
    let candidates: Vec<u32> = (0..*tiles)
        .filter(|n| !named.contains(n))
        .filter(|&n| sprite_set::tile_digits(pixels, size.0, *per_row, n) == blank)
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let base = art_files(t, assets)?;
    let file = t.asset.file();
    // Whether painting `set` changes any picture.
    let changes = |set: &[u32]| -> Result<bool, String> {
        let mut sheet = pixels.clone();
        for &n in set {
            sprite_set::put_tile(&mut sheet, size.0, *per_row, n, &"1".repeat(64))?;
        }
        let png = gp_assets::chr::write_png(size.0, size.1, &sheet);
        let stage = crate::asset_edit::stage_assets(t, assets, &file, &png)?;
        let r = art_files(t, &stage);
        let _ = fs::remove_dir_all(&stage);
        Ok(r? != base)
    };
    let mut used = Vec::new();
    let mut todo = vec![candidates.clone()];
    while let Some(set) = todo.pop() {
        if !changes(&set)? {
            continue;
        }
        if set.len() == 1 {
            used.push(set[0]);
            continue;
        }
        let (a, b) = set.split_at(set.len() / 2);
        todo.push(a.to_vec());
        todo.push(b.to_vec());
    }
    Ok(candidates
        .into_iter()
        .filter(|n| !used.contains(n))
        .collect())
}

/// The picture at a new size in tiles, the old one at its bottom left, so
/// the art stays where it is on screen (a sprite is drawn from the body's
/// left edge, standing on its bottom edge). Cut when it shrinks.
pub fn resized(p: &Pic, width: u32, height: u32) -> Pic {
    let (w, h) = (width * 8, height * 8);
    let mut px = vec![0u8; (w * h) as usize];
    let dy = i64::from(h) - i64::from(p.h);
    for y in 0..p.h {
        for x in 0..p.w.min(w) {
            let ny = i64::from(y) + dy;
            if ny >= 0 {
                px[(ny as u32 * w + x) as usize] = p.px[(y * p.w + x) as usize];
            }
        }
    }
    Pic { w, h, px }
}

/// The tile list for a sprite resized to `width` x `height` tiles: its own
/// tiles first, then free ones. Err when the slot hasn't enough.
pub fn tile_list(own: &[u32], free: &[u32], width: u32, height: u32) -> Result<Vec<u32>, String> {
    let n = (width * height) as usize;
    let mut out: Vec<u32> = own.iter().copied().take(n).collect();
    let more: Vec<u32> = free
        .iter()
        .copied()
        .filter(|t| !own.contains(t))
        .take(n - out.len())
        .collect();
    out.extend(more);
    if out.len() < n {
        return Err(format!(
            "the sprite needs {n} tiles; it has {} and the slot has {} free (blank, named by no sprite and drawn by nothing)",
            own.len(),
            free.len()
        ));
    }
    Ok(out)
}

/// Writes an entity's sprite tiles and width into a copy's model.
pub fn set_sprite(dir: &Path, entity: &str, tiles: &[u32], width: u32) -> Result<(), String> {
    let path = dir.join(gp_model::MODEL_FILE);
    let text = fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut model = gp_model::Model::parse(&text).map_err(|e| e.to_string())?;
    let sp = model
        .entities
        .get_mut(entity)
        .and_then(|e| e.core.sprite.as_mut())
        .ok_or(format!("`{entity}` has no sprite"))?;
    sp.tiles = tiles.to_vec();
    sp.width = width;
    fs::write(&path, model.to_toml()).map_err(|e| format!("writing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resized_picture_keeps_its_bottom_left() {
        let p = Pic::from_rows(&vec!["1".repeat(8); 8]).unwrap();
        let big = resized(&p, 2, 2);
        assert_eq!((big.w, big.h), (16, 16));
        assert_eq!(big.px[8 * 16], 1, "row 8, column 0");
        assert_eq!(big.px[0], 0);
        assert_eq!(big.px[15 * 16 + 8], 0, "the new column is blank");
        let small = resized(&big, 1, 1);
        assert_eq!(small, p);
    }

    #[test]
    fn a_tile_list_keeps_its_own_tiles_first() {
        assert_eq!(tile_list(&[4, 5], &[1, 4, 9], 2, 2).unwrap(), [4, 5, 1, 9]);
        assert_eq!(tile_list(&[4, 5, 6, 7], &[], 1, 2).unwrap(), [4, 5]);
        assert!(tile_list(&[4], &[1], 2, 2).unwrap_err().contains("needs 4"));
    }
}
