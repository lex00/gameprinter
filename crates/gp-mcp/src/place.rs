//! `assets_place` (docs/stubs.md): put a new asset into a working copy's
//! slot, after checking it against the slot's shape, packing it and
//! proving the simulation unchanged. Tiles and palettes slots, whole or
//! for one entity's sprite.
//!
//! A working copy's assets live in `<copy>/assets/`, in the lifted format,
//! with the slots' state in `slots.toml` beside them. The first placement
//! copies them there from the lifted assets the server was given. Working
//! copies are outside any git work tree, so nothing placed reaches the repo.
//!
//! `assets_generate` makes its candidates from a [`Target`] and hands them
//! to [`place_content`], so a generated asset passes the same checks as one
//! an agent or a person supplies.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use gp_assets::{Asset, AssetMap, MAP_FILE, chr};
use gp_evidence::Hash;
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_interp::{Interp, Lifted};
use gp_model::Model;
use gp_model::model::{AssetKind, Sprite};
use gp_runtime::InputFrame;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::catalog::{self, Origin, SlotState, State, Status};
use crate::{Config, Reply, model_check};

/// The directory a working copy keeps its own assets in.
pub const COPY_ASSETS: &str = "assets";

/// The largest image `assets_place` reads: a whole 8KB pattern table of 512
/// tiles is 128 by 256 pixels, a few KB of PNG.
pub const MAX_IMAGE_BYTES: usize = 1 << 20;

/// The highest NES color number.
pub const MAX_COLOR: u8 = 0x3F;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceArgs {
    pub copy: String,
    /// The slot: a tiles slot such as `tiles`, or a palettes slot such as
    /// `palettes`.
    pub slot: String,
    /// Replace only what this entity's sprite uses. In a tiles slot that's
    /// its tiles: the image is the sprite's size (its `image_size` in
    /// surface_describe's `used_by`), tiles laid out row by row as the
    /// sprite lists them. In a palettes slot it's the one palette the sprite
    /// draws with, and `palettes` holds that one. Without it the image or
    /// the palettes are the whole slot.
    #[serde(default)]
    pub entity: Option<String>,
    /// For a tiles slot, the image: a path to an indexed PNG on this
    /// machine.
    #[serde(default)]
    pub file: Option<PathBuf>,
    /// For a tiles slot, the image as base64, instead of `file`.
    #[serde(default)]
    pub png_base64: Option<String>,
    /// For a palettes slot: the palettes, each four NES color numbers from
    /// 0 to 63 (0x3F). Every palette of the slot (`shape.palettes`), or with
    /// `entity` just the one its sprite uses.
    #[serde(default)]
    pub palettes: Option<Vec<Vec<u8>>>,
    /// Its license, as an SPDX expression (`CC0-1.0`, `CC-BY-4.0`).
    pub license: String,
    /// `generated` (by a model or a tool) or `uploaded` (by a person).
    pub origin: String,
    /// The tool or model that made it.
    #[serde(default)]
    pub generator: Option<String>,
    /// Who made it.
    #[serde(default)]
    pub author: Option<String>,
    /// One line on what it is.
    #[serde(default)]
    pub note: Option<String>,
}

/// What goes into a slot.
pub enum Content {
    /// An indexed PNG, read and held to the slot's shape.
    Png(Vec<u8>),
    /// Color indices row by row, of the image size a PNG would have.
    Pixels(Vec<u8>),
    /// Palettes of four NES color numbers.
    Palettes(Vec<Vec<u8>>),
}

/// Who made a placement and under what terms: the slot's new state, but
/// for its digest.
pub struct Provenance {
    pub status: Status,
    pub origin: Origin,
    pub license: String,
    pub generator: Option<String>,
    pub author: Option<String>,
    pub note: Option<String>,
}

/// What a slot holds now, read from the copy's current assets.
#[derive(Clone)]
pub enum Current {
    Tiles {
        tiles: u32,
        per_row: u32,
        /// The sheet's size in pixels.
        size: (u32, u32),
        pixels: Vec<u8>,
    },
    Palettes(Vec<[u8; 4]>),
}

/// A slot of a working copy, checked as one a placement may write: a
/// presentation slot of a kind `assets_place` takes, filled from an asset of
/// the map, with baselines to prove the simulation unchanged. Reading it
/// writes nothing.
#[derive(Clone)]
pub struct Target {
    pub copy: String,
    pub dir: PathBuf,
    pub model: Model,
    pub slot: String,
    pub kind: AssetKind,
    pub asset: Asset,
    pub entity: Option<String>,
    pub sprite: Option<Sprite>,
    pub current: Current,
    /// The slot's state now: from `slots.toml`, or original.
    pub state: SlotState,
    pub(crate) runs: Vec<(String, Vec<InputFrame>)>,
}

impl Target {
    /// The image size a tiles placement takes: the sprite's or the sheet's.
    pub fn image_size(&self) -> Option<(u32, u32)> {
        match (&self.current, &self.sprite) {
            (Current::Tiles { .. }, Some(sp)) => Some(sprite_size(sp)),
            (Current::Tiles { size, .. }, None) => Some(*size),
            _ => None,
        }
    }

    /// The pixels the placement replaces now: the sprite's tiles laid out
    /// as it lists them, or the whole sheet.
    pub fn current_pixels(&self) -> Option<Vec<u8>> {
        let Current::Tiles {
            per_row,
            size,
            pixels,
            ..
        } = &self.current
        else {
            return None;
        };
        Some(match &self.sprite {
            None => pixels.clone(),
            Some(sp) => {
                let (sw, sh) = sprite_size(sp);
                let mut out = vec![0u8; (sw * sh) as usize];
                for (k, &t) in sp.tiles.iter().enumerate() {
                    let w = sp.width.max(1);
                    let (sx, sy) = ((k as u32 % w) * 8, (k as u32 / w) * 8);
                    let (tx, ty) = ((t % per_row) * 8, (t / per_row) * 8);
                    for y in 0..8 {
                        for x in 0..8 {
                            out[((sy + y) * sw + sx + x) as usize] =
                                pixels[((ty + y) * size.0 + tx + x) as usize];
                        }
                    }
                }
                out
            }
        })
    }

    /// The role of each palette, by the entities whose sprites use it.
    pub fn palette_roles(&self) -> Vec<Option<String>> {
        let Current::Palettes(p) = &self.current else {
            return Vec::new();
        };
        let roles = catalog::palette_roles(&self.model);
        (0..p.len() as u32)
            .map(|i| match roles.get(&i).map(Vec::as_slice) {
                Some([one]) => Some((*one).to_owned()),
                _ => None,
            })
            .collect()
    }

    /// The entity's role, when the placement is for one.
    pub fn role(&self) -> Option<&str> {
        let e = self.model.entities.get(self.entity.as_deref()?)?;
        Some(&e.core.role)
    }
}

fn sprite_size(sp: &Sprite) -> (u32, u32) {
    let w = sp.width.max(1);
    (w * 8, (sp.tiles.len() as u32).div_ceil(w) * 8)
}

/// The assets a working copy runs on now and their map: its own, or the
/// lifted assets the server was given. Reads only.
pub(crate) fn current_assets(
    config: &Config,
    dir: &Path,
    model: &Model,
) -> Result<(PathBuf, AssetMap), String> {
    let at = config
        .assets_of(dir, model.model.source.as_deref())
        .filter(|a| a.join(MAP_FILE).is_file())
        .ok_or("no lifted assets for this model: start the server with --assets, or lift them with gp assets lift")?;
    let map = read_map(&at)?;
    Ok((at, map))
}

/// The files of a lifted assets directory a copy of it needs: the map,
/// every asset's files, and the files capture writes beside them that a
/// lift doesn't (a sprite-set's `<name>.toml`, docs/formats/sprite-set.md),
/// when they're there.
pub(crate) fn asset_files(map: &AssetMap, from: &Path) -> Vec<String> {
    let mut names = vec![MAP_FILE.to_owned()];
    for a in &map.assets {
        names.extend(a.files());
        if matches!(a, Asset::SpriteSet(_)) {
            let f = format!("{}.toml", a.name());
            if from.join(&f).is_file() {
                names.push(f);
            }
        }
    }
    names
}

/// The working copy's own assets directory, made from the server's lifted
/// assets on first use. It's filled under another name and renamed into
/// place, with its `slots.toml`, so a copy that failed halfway is never
/// taken for a finished one.
pub fn copy_assets(
    config: &Config,
    dir: &Path,
    model: &Model,
) -> Result<(PathBuf, AssetMap), String> {
    let to = dir.join(COPY_ASSETS);
    if !to.join(MAP_FILE).is_file() {
        let (from, map) = current_assets(config, dir, model)?;
        let tmp = dir.join(format!("{COPY_ASSETS}.tmp-{}", std::process::id()));
        fs::create_dir_all(&tmp).map_err(|e| format!("creating {}: {e}", tmp.display()))?;
        for name in asset_files(&map, &from) {
            let dst = tmp.join(&name);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).map_err(|e| format!("creating {}: {e}", p.display()))?;
            }
            fs::copy(from.join(&name), &dst).map_err(|e| format!("copying {name}: {e}"))?;
        }
        // The state of the assets copied, or, for a plain lift, every slot
        // original.
        let state = match State::read(&from)? {
            Some(s) => s,
            None => original_state(model, &map, &tmp, dir)?,
        };
        fs::write(tmp.join(catalog::STATE_FILE), state.to_toml())
            .map_err(|e| format!("writing {}: {e}", catalog::STATE_FILE))?;
        fs::rename(&tmp, &to).map_err(|e| format!("renaming {} into place: {e}", tmp.display()))?;
    }
    let map = read_map(&to)?;
    Ok((to, map))
}

pub(crate) fn read_map(dir: &Path) -> Result<AssetMap, String> {
    let text = fs::read_to_string(dir.join(MAP_FILE))
        .map_err(|e| format!("reading {}: {e}", dir.join(MAP_FILE).display()))?;
    AssetMap::parse(&text).map_err(|e| format!("{MAP_FILE}: {e}"))
}

/// One slot as lifted from the source.
fn original_slot(
    map: &AssetMap,
    assets: &Path,
    asset: &Asset,
    license: &str,
) -> Result<SlotState, String> {
    Ok(SlotState {
        status: Status::Original,
        origin: Origin::Source,
        license: license.into(),
        sha256: catalog::asset_digest(assets, asset)?,
        generator: None,
        author: None,
        note: Some(format!(
            "lifted from {} (ROM sha256 {})",
            map.source, map.sha256
        )),
    })
}

/// Every slot as lifted from the source, licensed as
/// `catalog::original_license` says for the template the copy was forked
/// from.
pub fn original_state(
    model: &Model,
    map: &AssetMap,
    assets: &Path,
    copy: &Path,
) -> Result<State, String> {
    let license = catalog::original_license(copy);
    let mut state = State::new(&model.model.name);
    for (slot, asset) in catalog::slot_assets(model, map)? {
        state
            .slots
            .insert(slot.to_owned(), original_slot(map, assets, asset, license)?);
    }
    Ok(state)
}

/// Writes a file through a temporary file and a rename, so a reader never
/// sees half of it.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp-place");
    fs::write(&tmp, bytes).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("renaming into {}: {e}", path.display()))
}

/// The input of every baseline of the model's source, by name: the
/// scenarios the simulation must play the same on.
fn scenarios(config: &Config, model: &Model) -> Result<Vec<(String, Vec<InputFrame>)>, String> {
    let Some(source) = model
        .model
        .source
        .as_deref()
        .filter(|s| crate::is_source_name(s))
    else {
        return Ok(Vec::new());
    };
    let mut dir = config.repo.join("baselines").join(source);
    // An original has no ROM baselines; its model baselines are its
    // template's (docs/originals.md).
    let model = !dir.is_dir() && crate::model_baselines_dir(config, source).is_dir();
    if model {
        dir = crate::model_baselines_dir(config, source);
    }
    let mut paths: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(d) => d
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect(),
        Err(_) => return Ok(Vec::new()),
    };
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let log = if model {
                gp_interp::baseline::ModelBaseline::load(&p)?.input()?
            } else {
                gp_evidence::baseline::Baseline::load(&p)
                    .map_err(|e| e.to_string())?
                    .input()
                    .map_err(|e| e.to_string())?
            };
            let frames = log
                .frames
                .iter()
                .map(|f| InputFrame {
                    buttons: f.buttons_word(),
                    axes: Vec::new(),
                    reset: f.reset,
                })
                .collect();
            let name = p
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            Ok((name, frames))
        })
        .collect()
}

/// The digest of a run's per-frame state hashes, as `gp interp run`
/// prints it.
pub fn state_digest(model: &Model, assets: &Path, input: &[InputFrame]) -> Result<String, String> {
    let lifted = Lifted::read_dir(assets, model)?;
    let mut rt = Interp::load(model, &lifted)?;
    let hash = |rt: &Interp| {
        let h = Hash::from_bytes(rt.world().hash());
        FrameHash { state: h, ram: h }
    };
    let mut hashes = vec![hash(&rt)];
    for f in input {
        rt.advance(f)?;
        hashes.push(hash(&rt));
    }
    Ok(Hash::of(&frame_hashes::encode(&hashes)).to_string())
}

fn digests(
    model: &Model,
    assets: &Path,
    runs: &[(String, Vec<InputFrame>)],
) -> Result<BTreeMap<String, String>, String> {
    runs.iter()
        .map(|(n, f)| Ok((n.clone(), state_digest(model, assets, f)?)))
        .collect()
}

/// The candidate image's bytes.
fn image_bytes(a: &PlaceArgs) -> Result<Vec<u8>, String> {
    let bytes = match (&a.file, &a.png_base64) {
        (Some(p), None) => fs::read(p).map_err(|e| format!("reading {}: {e}", p.display()))?,
        (None, Some(b)) if b.len() > MAX_IMAGE_BYTES / 3 * 4 + 4 => {
            return Err(format!(
                "png_base64 is longer than an image of {MAX_IMAGE_BYTES} bytes"
            ));
        }
        (None, Some(b)) => base64::engine::general_purpose::STANDARD
            .decode(b.trim())
            .map_err(|e| format!("png_base64: {e}"))?,
        _ => return Err("give the image as `file` or `png_base64`, one of them".into()),
    };
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "the image is {} bytes, more than {MAX_IMAGE_BYTES}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

/// Names the pixels whose color index is above 3, at most five.
fn bad_indices(px: &[u8], width: u32) -> Vec<String> {
    px.iter()
        .enumerate()
        .filter(|(_, p)| **p > 3)
        .take(5)
        .map(|(i, p)| {
            format!(
                "pixel ({}, {}) has color index {p}; tiles have 4 colors, 0 to 3",
                i as u32 % width,
                i as u32 / width
            )
        })
        .collect()
}

/// Reads an indexed PNG of exactly `want` pixels with indices 0 to 3, and
/// names every problem it finds.
fn read_tiles_image(bytes: &[u8], want: (u32, u32)) -> Result<Vec<u8>, Vec<String>> {
    let px = match chr::read_png(bytes, want) {
        Ok(px) => px,
        // An editor that saved RGBA: read through the image format crate,
        // each pixel's index by the lifted files' four preview grays, and
        // a clear pixel as index 0 (docs/formats/image-config.md).
        Err(e) => {
            rgba_tiles(bytes, want).map_err(|e2| vec![format!("image: {e}; as RGBA: {e2}")])?
        }
    };
    let bad = bad_indices(&px, want.0);
    if bad.is_empty() { Ok(px) } else { Err(bad) }
}

/// An RGBA image of tiles as indices 0 to 3, by the preview grays.
fn rgba_tiles(bytes: &[u8], want: (u32, u32)) -> Result<Vec<u8>, String> {
    let (w, h, px) = gp_imagefmt::sheet::read_png(bytes)?;
    if (w, h) != want {
        return Err(format!(
            "the image is {w}x{h}; the slot's tiles make {}x{}",
            want.0, want.1
        ));
    }
    let gp_imagefmt::sheet::Pixels::Rgba(px) = px else {
        return Err("it isn't RGBA either".into());
    };
    px.iter()
        .enumerate()
        .map(|(i, p)| match (p[3], p[0], p[1], p[2]) {
            (0, ..) => Ok(0),
            (_, r, g, b) if r == g && g == b && r % 0x55 == 0 => Ok(r / 0x55),
            _ => Err(format!(
                "pixel ({}, {}) is #{:02x}{:02x}{:02x}, not one of the four grays",
                i as u32 % w,
                i as u32 / w,
                p[0],
                p[1],
                p[2]
            )),
        })
        .collect()
}

/// Reads a palettes file in the lifted format.
pub fn read_palettes(text: &str) -> Result<Vec<[u8; 4]>, String> {
    let t: toml::Table = text.parse().map_err(|e| format!("palettes: {e}"))?;
    let list = t
        .get("palettes")
        .and_then(toml::Value::as_array)
        .ok_or("palettes: no `palettes` list")?;
    list.iter()
        .enumerate()
        .map(|(i, p)| {
            let c: Vec<u8> = p
                .as_array()
                .ok_or(format!("palette {i} isn't a list"))?
                .iter()
                .map(|v| {
                    v.as_integer()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or(format!("palette {i} holds a value that isn't a byte"))
                })
                .collect::<Result<_, _>>()?;
            <[u8; 4]>::try_from(c).map_err(|_| format!("palette {i} hasn't four colors"))
        })
        .collect()
}

/// A palettes file in the lifted format, as `gp assets pack` reads it.
pub fn write_palettes(asset: &str, palettes: &[[u8; 4]], by: &str) -> String {
    let mut out = format!(
        "# Palettes: four NES color numbers each.\n# Placed by {by} (docs/stubs.md).\nschema = {}\nasset = \"{asset}\"\npalettes = [\n",
        gp_assets::map::SCHEMA
    );
    let count = palettes.len();
    for (i, p) in palettes.iter().enumerate() {
        let what = match (count, i) {
            (8, 0..4) => format!("background {i}"),
            (8, _) => format!("sprites {}", i - 4),
            _ => i.to_string(),
        };
        let items: Vec<String> = p.iter().map(|c| format!("0x{c:02X}")).collect();
        let _ = writeln!(out, "  [{}], # {what}", items.join(", "));
    }
    out.push_str("]\n");
    out
}

fn refused(problems: Vec<String>) -> Reply {
    Reply::refused(json!({ "ok": false, "written": false, "problems": problems }))
}

/// Why a placement stopped after the file was written: refused (a result
/// for the agent) or an error. Either way the files go back.
enum Stop {
    Refused(Reply),
    Error(String),
}

impl From<String> for Stop {
    fn from(e: String) -> Self {
        Stop::Error(e)
    }
}

/// Checks a license given for a new asset: an SPDX-style expression, and
/// not the one reserved for the person's own copy.
pub fn check_license(license: &str) -> Result<&str, String> {
    let license = license.trim();
    if license.is_empty()
        || !license
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " .-+:()".contains(c))
        || license.contains(catalog::OWN_COPY_LICENSE)
    {
        return Err(format!(
            "license `{license}` isn't an SPDX expression for a new asset (such as CC0-1.0 or CC-BY-4.0)"
        ));
    }
    Ok(license)
}

/// The license of a slot after part of it is replaced: both, unless the
/// old already covers the new.
pub fn combine_licenses(old: &str, new: &str) -> String {
    let terms: Vec<&str> = old.split(" AND ").map(str::trim).collect();
    if terms.contains(&new) {
        return old.to_owned();
    }
    if old.contains(" OR ") || old.contains(" WITH ") {
        format!("({old}) AND {new}")
    } else {
        format!("{old} AND {new}")
    }
}

pub fn place(config: &Config, a: PlaceArgs) -> Result<Reply, String> {
    let origin = match a.origin.as_str() {
        "generated" => Origin::Generated,
        "uploaded" => Origin::Uploaded,
        o => return Err(format!("origin `{o}` isn't `generated` or `uploaded`")),
    };
    let license = check_license(&a.license)?.to_owned();
    let t = target(config, &a.copy, &a.slot, a.entity.as_deref())?;
    let content = match t.kind {
        AssetKind::Tiles => {
            if a.palettes.is_some() {
                return Err(format!(
                    "slot `{}` is a tiles slot: give an image, not palettes",
                    a.slot
                ));
            }
            Content::Png(image_bytes(&a)?)
        }
        _ => {
            if a.file.is_some() || a.png_base64.is_some() {
                return Err(format!(
                    "slot `{}` is a palettes slot: give `palettes`, not an image",
                    a.slot
                ));
            }
            Content::Palettes(
                a.palettes
                    .clone()
                    .ok_or("give the palettes as `palettes`")?,
            )
        }
    };
    let p = Provenance {
        status: Status::Replaced,
        origin,
        license,
        generator: a.generator.clone(),
        author: a.author.clone(),
        note: a.note.clone(),
    };
    place_content(config, &t, &p, content)
}

/// Reads and checks a slot of a working copy for a placement. Writes
/// nothing; refusals here are errors, since the call can't be fixed by a
/// different asset.
pub fn target(
    config: &Config,
    copy: &str,
    slot_name: &str,
    entity: Option<&str>,
) -> Result<Target, String> {
    let dir = config.existing_copy(copy)?;
    let model_text = fs::read_to_string(dir.join(gp_model::MODEL_FILE))
        .map_err(|e| format!("reading {}: {e}", gp_model::MODEL_FILE))?;
    let model = Model::parse(&model_text).map_err(|e| format!("{}: {e}", gp_model::MODEL_FILE))?;
    let slot = model
        .assets
        .get(slot_name)
        .ok_or(format!("the model has no asset slot `{slot_name}`"))?;
    let kind = slot.kind;
    if !matches!(kind, AssetKind::Tiles | AssetKind::Palettes) {
        return Err(format!(
            "slot `{slot_name}` is {}; assets_place takes tiles and palettes slots so far",
            catalog::kind_name(kind)
        ));
    }
    let simulation = gp_interp::lifted::simulation_slots(&model);
    if simulation.iter().any(|s| s.as_str() == slot_name) {
        return Err(format!(
            "the simulation reads slot `{slot_name}`; change it with model_edit"
        ));
    }
    // A slot whose asset a simulation slot also fills is simulation too.
    let from = slot
        .from
        .as_ref()
        .ok_or(format!(
            "slot `{slot_name}` has no `from`, so it has no shape to keep"
        ))?
        .clone();
    if let Some(other) = simulation
        .iter()
        .find(|s| model.assets.get(s.as_str()).and_then(|o| o.from.as_ref()) == Some(&from))
    {
        return Err(format!(
            "slot `{slot_name}` shares its asset with `{other}`, which the simulation reads; change it with model_edit"
        ));
    }
    let runs = scenarios(config, &model)?;
    if runs.is_empty() {
        return Err("the model's source has no baselines, so nothing can show the simulation unchanged; assets_place needs at least one".into());
    }
    let (at, map) = current_assets(config, &dir, &model)?;
    let asset = (*catalog::slot_assets(&model, &map)?
        .get(slot_name)
        .ok_or(format!("slot `{slot_name}` isn't in the asset map"))?)
    .clone();
    let file = asset.file();
    let current = match (&asset, kind) {
        (
            Asset::Chr {
                tiles,
                tiles_per_row,
                ..
            },
            AssetKind::Tiles,
        ) => {
            let size = chr::image_size(*tiles, *tiles_per_row);
            let png = fs::read(at.join(&file)).map_err(|e| format!("reading {file}: {e}"))?;
            Current::Tiles {
                tiles: *tiles,
                per_row: *tiles_per_row,
                size,
                pixels: chr::read_png(&png, size).map_err(|e| format!("{file}: {e}"))?,
            }
        }
        (Asset::Palette { count, .. }, AssetKind::Palettes) => {
            let text =
                fs::read_to_string(at.join(&file)).map_err(|e| format!("reading {file}: {e}"))?;
            let p = read_palettes(&text).map_err(|e| format!("{file}: {e}"))?;
            if p.len() != *count as usize {
                return Err(format!(
                    "{file} holds {} palettes, but the asset map says {count}",
                    p.len()
                ));
            }
            Current::Palettes(p)
        }
        _ => {
            return Err(format!(
                "slot `{slot_name}` isn't {} in the asset map",
                kind.map_format()
            ));
        }
    };
    let sprite = match entity {
        None => None,
        Some(name) => {
            let e = model
                .entities
                .get(name)
                .ok_or(format!("the model has no entity `{name}`"))?;
            let sp = e
                .core
                .sprite
                .as_ref()
                .ok_or(format!("entity `{name}` has no sprite"))?
                .clone();
            match &current {
                Current::Tiles { tiles, .. } => {
                    if sp.slot != slot_name {
                        return Err(format!(
                            "entity `{name}`'s sprite is in slot `{}`, not `{slot_name}`",
                            sp.slot
                        ));
                    }
                    if let Some(t) = sp.tiles.iter().find(|&&t| t >= *tiles) {
                        return Err(format!(
                            "entity `{name}` uses tile {t}, past the slot's {tiles}"
                        ));
                    }
                }
                Current::Palettes(p) => {
                    if sp.palette as usize >= p.len() {
                        return Err(format!(
                            "entity `{name}` uses palette {}, past the slot's {}",
                            sp.palette,
                            p.len()
                        ));
                    }
                }
            }
            Some(sp)
        }
    };
    let state = match State::read(&at)?.and_then(|s| s.slots.get(slot_name).cloned()) {
        Some(s) => s,
        None => original_slot(&map, &at, &asset, catalog::original_license(&dir))?,
    };
    Ok(Target {
        copy: copy.to_owned(),
        dir,
        model,
        slot: slot_name.to_owned(),
        kind,
        asset,
        entity: entity.map(str::to_owned),
        sprite,
        current,
        state,
        runs,
    })
}

/// The new file for a slot and what changed, or the problems that refuse
/// the content.
type Prepared = (Vec<u8>, Value);

fn prepare_tiles(t: &Target, content: Content) -> Result<Result<Prepared, Vec<String>>, String> {
    let Current::Tiles {
        tiles,
        per_row,
        size: (w, h),
        pixels: old,
    } = &t.current
    else {
        unreachable!("a tiles target");
    };
    let (tiles, per_row, w, h) = (*tiles, *per_row, *w, *h);
    let want = t.image_size().expect("a tiles target");
    let image = match content {
        Content::Png(bytes) => match read_tiles_image(&bytes, want) {
            Ok(px) => px,
            Err(p) => return Ok(Err(p)),
        },
        Content::Pixels(px) => {
            if px.len() != (want.0 * want.1) as usize {
                return Err(format!(
                    "{} pixels for an image of {}x{}",
                    px.len(),
                    want.0,
                    want.1
                ));
            }
            let bad = bad_indices(&px, want.0);
            if !bad.is_empty() {
                return Ok(Err(bad));
            }
            px
        }
        Content::Palettes(_) => {
            return Ok(Err(vec![format!(
                "slot `{}` is a tiles slot: give an image, not palettes",
                t.slot
            )]));
        }
    };
    let new = match &t.sprite {
        None => {
            let used = |i: usize| {
                let (x, y) = (i as u32 % w / 8, i as u32 / w / 8);
                y * per_row + x < tiles
            };
            if let Some(i) = (0..image.len()).find(|&i| image[i] != 0 && !used(i)) {
                return Ok(Err(vec![format!(
                    "pixel ({}, {}) is past the last tile and must stay index 0",
                    i as u32 % w,
                    i as u32 / w
                )]));
            }
            image
        }
        Some(sp) => {
            let sw = sp.width.max(1);
            let iw = sw * 8;
            let mut sheet = old.clone();
            for (k, &tile) in sp.tiles.iter().enumerate() {
                let (sx, sy) = ((k as u32 % sw) * 8, (k as u32 / sw) * 8);
                let (tx, ty) = ((tile % per_row) * 8, (tile / per_row) * 8);
                for y in 0..8 {
                    for x in 0..8 {
                        sheet[((ty + y) * w + tx + x) as usize] =
                            image[((sy + y) * iw + sx + x) as usize];
                    }
                }
            }
            sheet
        }
    };
    let changed: Vec<u32> = (0..tiles)
        .filter(|tile| {
            let (tx, ty) = ((tile % per_row) * 8, (tile / per_row) * 8);
            (0..8).any(|y| {
                (0..8).any(|x| {
                    let i = ((ty + y) * w + tx + x) as usize;
                    old[i] != new[i]
                })
            })
        })
        .collect();
    Ok(Ok((
        chr::write_png(w, h, &new),
        json!({ "tiles_changed": changed }),
    )))
}

fn prepare_palettes(
    t: &Target,
    content: Content,
    by: &str,
) -> Result<Result<Prepared, Vec<String>>, String> {
    let Current::Palettes(old) = &t.current else {
        unreachable!("a palettes target");
    };
    let Content::Palettes(given) = content else {
        return Ok(Err(vec![format!(
            "slot `{}` is a palettes slot: give palettes, not an image",
            t.slot
        )]));
    };
    let want = if t.sprite.is_some() { 1 } else { old.len() };
    let mut problems = Vec::new();
    if given.len() != want {
        problems.push(match &t.entity {
            Some(e) => format!(
                "give one palette, the one entity `{e}`'s sprite uses; {} given",
                given.len()
            ),
            None => format!(
                "the slot has {want} palettes and {} were given",
                given.len()
            ),
        });
    }
    for (i, p) in given.iter().enumerate() {
        if p.len() != 4 {
            problems.push(format!("palette {i} has {} colors, not 4", p.len()));
        }
        if let Some(c) = p.iter().find(|c| **c > MAX_COLOR) {
            problems.push(format!(
                "palette {i} holds {c}, past the last NES color number 63 (0x3F)"
            ));
        }
    }
    if !problems.is_empty() {
        return Ok(Err(problems));
    }
    let given: Vec<[u8; 4]> = given.iter().map(|p| [p[0], p[1], p[2], p[3]]).collect();
    let mut new = old.clone();
    let mut shared_with = Vec::new();
    match &t.sprite {
        Some(sp) => {
            new[sp.palette as usize] = given[0];
            shared_with = t
                .model
                .entities
                .iter()
                .filter(|(n, e)| {
                    Some(n.as_str()) != t.entity.as_deref()
                        && e.core
                            .sprite
                            .as_ref()
                            .is_some_and(|s| s.palette == sp.palette)
                })
                .map(|(n, _)| n.clone())
                .collect();
        }
        None => new = given,
    }
    let changed: Vec<usize> = (0..new.len()).filter(|&i| old[i] != new[i]).collect();
    let mut detail = json!({ "palettes_changed": changed });
    if !shared_with.is_empty() {
        detail["shared_with"] = json!(shared_with);
    }
    Ok(Ok((
        write_palettes(t.asset.name(), &new, by).into_bytes(),
        detail,
    )))
}

/// Puts content into a target slot: checks it against the shape, writes it
/// into the copy's own assets, packs them, runs every baseline before and
/// after, checks the model, and records the slot's new state. A refusal or
/// a failure after the write puts the file and `slots.toml` back.
pub fn place_content(
    config: &Config,
    t: &Target,
    p: &Provenance,
    content: Content,
) -> Result<Reply, String> {
    let by = p.generator.as_deref().unwrap_or("assets_place");
    let prepared = match t.kind {
        AssetKind::Tiles => prepare_tiles(t, content)?,
        _ => prepare_palettes(t, content, by)?,
    };
    let (new_file, detail) = match prepared {
        Ok(x) => x,
        Err(problems) => return Ok(refused(problems)),
    };

    // The copy's own assets, and the file in them.
    let (assets, map) = copy_assets(config, &t.dir, &t.model)?;
    let file = t.asset.file();
    let path = assets.join(&file);
    let state_path = assets.join(catalog::STATE_FILE);
    let old_file = fs::read(&path).map_err(|e| format!("reading {file}: {e}"))?;
    let old_state = fs::read(&state_path).ok();

    // Before: the digest of every scenario, and the packed bytes.
    let before = digests(&t.model, &assets, &t.runs)?;
    let encode =
        |dir: &Path| gp_assets::encode(&map, &|n| fs::read(dir.join(n))).map_err(|e| e.to_string());
    let packed_before = encode(&assets)?;

    write_atomic(&path, &new_file)?;
    // Everything after the write either finishes or puts both files back.
    let finish = || -> Result<Reply, Stop> {
        let packed_after =
            encode(&assets).map_err(|e| Stop::Refused(refused(vec![format!("pack: {e}")])))?;
        let bytes_changed: usize = packed_before
            .iter()
            .zip(&packed_after)
            .map(|(b, a)| b.bytes.iter().zip(&a.bytes).filter(|(x, y)| x != y).count())
            .sum();
        let after = digests(&t.model, &assets, &t.runs)?;
        if after != before {
            let differ: Vec<&String> = before.keys().filter(|k| before[*k] != after[*k]).collect();
            return Err(Stop::Refused(refused(vec![format!(
                "the simulation changed on {differ:?}: slot `{}` isn't presentation",
                t.slot
            )])));
        }
        let (ok, report) = model_check(config, &t.dir, t.model.model.source.as_deref())?;
        if !ok {
            return Err(Stop::Refused(Reply::refused(json!({
                "ok": false,
                "written": false,
                "problems": report["problems"],
            }))));
        }
        let mut state = State::read(&assets)?.unwrap_or_else(|| State::new(&t.model.model.name));
        // Part of a slot keeps the rest's terms: its license is both.
        let license = if t.entity.is_some() {
            let old = state
                .slots
                .get(&t.slot)
                .map_or(t.state.license.as_str(), |s| s.license.as_str());
            combine_licenses(old, &p.license)
        } else {
            p.license.clone()
        };
        let what = match (&t.entity, &detail) {
            (Some(e), d) if d.get("tiles_changed").is_some() => {
                Some(format!("{e}'s sprite, tiles {}", d["tiles_changed"]))
            }
            (Some(e), _) => Some(format!(
                "{e}'s palette {}",
                t.sprite.as_ref().map_or(0, |s| s.palette)
            )),
            _ => None,
        };
        let record = SlotState {
            status: p.status,
            origin: p.origin,
            license,
            sha256: catalog::asset_digest(&assets, &t.asset)?,
            generator: p.generator.clone(),
            author: p.author.clone(),
            note: p.note.clone().or(what),
        };
        // Slots filled from the same asset share its file, so they share
        // its state.
        let name = t.asset.name();
        for (other, asset) in catalog::slot_assets(&t.model, &map)? {
            if asset.name() == name {
                state.slots.insert(other.to_owned(), record.clone());
            }
        }
        state.slots.insert(t.slot.clone(), record.clone());
        write_atomic(&state_path, state.to_toml().as_bytes())?;
        let mut out = json!({
            "ok": true,
            "written": true,
            "copy": t.copy,
            "slot": t.slot,
            "entity": t.entity,
            "assets": assets,
            "file": file,
            "bytes_changed": bytes_changed,
            "state": record,
            "simulation": {
                "unchanged": true,
                "baselines": t.runs.len(),
                "digests": after,
            },
        });
        if let (Some(o), Some(d)) = (out.as_object_mut(), detail.as_object()) {
            o.extend(d.clone());
        }
        Ok(Reply::ok(out))
    };
    match finish() {
        Ok(r) => Ok(r),
        Err(stop) => {
            write_atomic(&path, &old_file)?;
            if let Some(s) = &old_state {
                write_atomic(&state_path, s)?;
            }
            match stop {
                Stop::Refused(r) => Ok(r),
                Stop::Error(e) => Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 8-bit indexed PNG, as image editors save them.
    fn png8(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Indexed);
        e.set_depth(png::BitDepth::Eight);
        e.set_palette(vec![0u8; 3 * 256]);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(px).unwrap();
        wr.finish().unwrap();
        out
    }

    #[test]
    fn images_are_held_to_the_shape() {
        let mut px = vec![1u8; 16 * 16];
        assert!(read_tiles_image(&png8(16, 16, &px), (16, 16)).is_ok());
        px[17] = 5;
        let p = read_tiles_image(&png8(16, 16, &px), (16, 16)).unwrap_err();
        assert_eq!(
            p,
            ["pixel (1, 1) has color index 5; tiles have 4 colors, 0 to 3"]
        );
        assert!(read_tiles_image(&png8(16, 8, &px[..128]), (16, 16)).is_err());
        assert!(read_tiles_image(b"not a png", (16, 16)).is_err());
    }

    #[test]
    fn palettes_round_trip_through_the_lifted_format() {
        let p = vec![[0x0F, 0x01, 0x11, 0x21], [0x0F, 0x3F, 0x00, 0x10]];
        let text = write_palettes("palettes", &p, "a test");
        assert_eq!(read_palettes(&text).unwrap(), p);
        assert!(read_palettes("palettes = [[1, 2, 3]]").is_err());
        assert!(read_palettes("palettes = [[1, 2, 3, 256]]").is_err());
    }

    #[test]
    fn licenses_combine_when_part_of_a_slot_is_replaced() {
        assert_eq!(
            combine_licenses("LicenseRef-own-copy", "CC0-1.0"),
            "LicenseRef-own-copy AND CC0-1.0"
        );
        assert_eq!(
            combine_licenses("LicenseRef-own-copy AND CC0-1.0", "CC0-1.0"),
            "LicenseRef-own-copy AND CC0-1.0"
        );
        assert_eq!(combine_licenses("CC0-1.0", "CC0-1.0"), "CC0-1.0");
        assert_eq!(
            combine_licenses("MIT OR Apache-2.0", "CC0-1.0"),
            "(MIT OR Apache-2.0) AND CC0-1.0"
        );
        assert!(check_license("CC-BY-4.0").is_ok());
        assert!(check_license("LicenseRef-own-copy").is_err());
        assert!(check_license("CC0-1.0 AND LicenseRef-own-copy").is_err());
        assert!(check_license("CC0; rm").is_err());
    }
}
