//! The round trip with an external pixel editor (docs/sprite-editing.md,
//! phase 3): an entity's sprite set written for the editor through
//! `gp-imagefmt` in the layout the image format config gives
//! (docs/formats/image-config.md), and each save read back through the
//! same crate's checks, then proposed as an `assets_edit` with every edit
//! check. Pixelorama is the first target: it reads the PNG sheet and its
//! own JSON palette, and saves PNGs over the exported ones. Its `.pxo`
//! project isn't written (an open-questions row).
//!
//! The export adds `gp-edit.json` beside the imagefmt sidecar: which copy,
//! entity and slot the files are, and each exported frame's name with its
//! frame id in the sprite set, so a save comes back to the same frames.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use gp_imagefmt::config::ImageConfig;
use gp_imagefmt::sheet;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::place::{self, Current};
use crate::sprite_ops::Pic;
use crate::{Config, Reply, critic, sprite_set};

/// The round trip's own file beside the sidecar.
pub const TRIP_FILE: &str = "gp-edit.json";
/// Its format.
pub const TRIP_FORMAT: &str = "gp-edit-trip 0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trip {
    pub format: String,
    pub copy: String,
    pub entity: String,
    pub slot: String,
    /// The editor the files were made for.
    pub editor: String,
    /// Exported frame name -> frame id in the sprite set.
    pub frames: BTreeMap<String, String>,
    /// The image files a save changes.
    pub images: Vec<String>,
}

/// The 16 sprite palette colors frames are exported in: the copy's sprite
/// palettes (4 to 7 of a palettes slot of eight, else its first four).
fn sprite_palette(
    config: &Config,
    copy: &str,
    model: &gp_model::Model,
) -> Result<[[u8; 3]; 16], String> {
    let slot = model
        .assets
        .iter()
        .find(|(_, s)| s.kind == gp_model::model::AssetKind::Palettes)
        .map(|(n, _)| n.clone())
        .ok_or("the model has no palettes slot")?;
    let t = place::target(config, copy, &slot, None)?;
    let Current::Palettes(p) = &t.current else {
        return Err(format!("`{slot}` isn't a palettes slot"));
    };
    let first = if p.len() >= 8 { 4 } else { 0 };
    Ok(std::array::from_fn(|i| {
        critic::nes_rgb(p.get(first + i / 4).map_or(0x0F, |q| q[i % 4]))
    }))
}

/// Writes an entity's sprite set for an editor into `out`: the images,
/// palette file and sidecar `gp-imagefmt` writes for `cfg`, and the trip
/// file. Returns the trip and the files written.
pub fn export(
    config: &Config,
    copy: &str,
    entity: &str,
    slot: &str,
    cfg: &ImageConfig,
    out: &Path,
) -> Result<(Trip, Vec<PathBuf>), String> {
    let t = place::target(config, copy, slot, Some(entity))?;
    let Current::Tiles {
        per_row,
        size,
        pixels,
        ..
    } = &t.current
    else {
        return Err(format!("`{slot}` isn't a tiles slot"));
    };
    let (assets, _) = place::current_assets(config, &t.dir, &t.model)?;
    let set = sprite_set::read_set(&t.model, &assets)?;
    let (frames, _) = sprite_set::frames(&t.model, entity, slot, set.as_ref())?;
    if frames.is_empty() {
        return Err(format!("`{entity}` has no frame of tiles in `{slot}`"));
    }
    let pal = t
        .sprite
        .as_ref()
        .map_or(0, |s| (s.palette.saturating_sub(4) & 3) as u8);
    let mut out_frames = Vec::new();
    let mut names = BTreeMap::new();
    for (k, f) in frames.iter().enumerate() {
        let pic = sprite_set::read(f, pixels, size.0, *per_row)?;
        let state = f.state.clone().unwrap_or_else(|| "default".to_owned());
        let n = f.set_index.unwrap_or(k as u32);
        let mut name = cfg.name(entity, &state, n);
        if names.contains_key(&name) {
            name = format!("{name}_{}", f.id.replace('.', "_"));
        }
        names.insert(name.clone(), f.id.clone());
        out_frames.push(sheet::Frame {
            name,
            entity: entity.to_owned(),
            state,
            set_index: n,
            width: f.w,
            height: f.h,
            origin: [0, 0],
            tiles: f
                .pieces
                .iter()
                .map(|p| sheet::Tile {
                    x: p.x,
                    y: p.y,
                    palette: pal,
                    flip_x: p.flip_x,
                    flip_y: p.flip_y,
                })
                .collect(),
            px: pic
                .px
                .iter()
                .map(|&c| if c == 0 { 0 } else { pal * 4 + c })
                .collect(),
        });
    }
    let palette = sprite_palette(config, copy, &t.model)?;
    fs::create_dir_all(out).map_err(|e| format!("making {}: {e}", out.display()))?;
    let files = sheet::write(cfg, &t.copy, &out_frames, &palette);
    let mut written = Vec::new();
    let mut images = Vec::new();
    for (name, bytes) in &files {
        let p = out.join(name);
        fs::write(&p, bytes).map_err(|e| format!("writing {}: {e}", p.display()))?;
        if name.ends_with(".png") {
            images.push(name.clone());
        }
        written.push(p);
    }
    let trip = Trip {
        format: TRIP_FORMAT.into(),
        copy: copy.to_owned(),
        entity: entity.to_owned(),
        slot: slot.to_owned(),
        editor: format!("{:?}", cfg.editor).to_lowercase(),
        frames: names,
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

pub fn read_trip(dir: &Path) -> Result<Trip, String> {
    let p = dir.join(TRIP_FILE);
    let text = fs::read_to_string(&p).map_err(|e| format!("reading {}: {e}", p.display()))?;
    let t: Trip = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
    if t.format != TRIP_FORMAT {
        return Err(format!(
            "{TRIP_FILE}: format {} isn't {TRIP_FORMAT}",
            t.format
        ));
    }
    Ok(t)
}

/// Reads what an editor saved in `dir` back as pictures by frame id, with
/// every `gp-imagefmt` import check; Err(problems) when one fails.
pub fn import(dir: &Path) -> Result<Result<BTreeMap<String, Pic>, Vec<String>>, String> {
    let trip = read_trip(dir)?;
    let got = sheet::read(&|n| fs::read(dir.join(n)).ok(), None)?;
    if !got.problems.is_empty() {
        return Ok(Err(got
            .problems
            .iter()
            .map(|p| format!("{}: {}", p.frame, p.message))
            .collect()));
    }
    let mut out = BTreeMap::new();
    for f in &got.frames {
        let Some(id) = trip.frames.get(&f.name) else {
            continue;
        };
        let px = f.px.iter().map(|&e| e % 4).collect();
        out.insert(id.clone(), Pic::new(f.width, f.height, px)?);
    }
    Ok(Ok(out))
}

/// A save brought back: imported, checked and proposed (or placed) as an
/// `assets_edit` of the frames that changed, with the editor as its origin.
pub fn propose_save(
    config: &Config,
    dir: &Path,
    place: bool,
    run: Option<&str>,
    author: Option<&str>,
) -> Result<Reply, String> {
    let trip = read_trip(dir)?;
    let pics = match import(dir)? {
        Ok(p) => p,
        Err(problems) => {
            return Ok(Reply::refused(json!({
                "ok": false, "written": false, "stage": "import", "problems": problems,
            })));
        }
    };
    let pictures: BTreeMap<String, Vec<String>> =
        pics.into_iter().map(|(id, p)| (id, p.rows())).collect();
    let args: Value = json!({
        "copy": trip.copy,
        "entity": trip.entity,
        "slot": trip.slot,
        "pictures": pictures,
        "place": place,
        "editor": trip.editor,
        "prompt": format!("saved in {}", trip.editor),
        "author": author,
        "run": run,
    });
    let a: crate::asset_edit::EditArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
    let _lock = crate::CopyLock::take(&trip.copy);
    crate::asset_edit::edit(config, a)
}

/// A digest of the image files a save changes, to notice one.
pub fn images_digest(dir: &Path, trip: &Trip) -> String {
    let mut all = Vec::new();
    for f in &trip.images {
        all.extend(fs::read(dir.join(f)).unwrap_or_default());
        all.push(0);
    }
    crate::catalog::sha256_hex(&all)
}
