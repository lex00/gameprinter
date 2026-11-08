//! A model's lifted assets, held in memory, and the bundle a recording
//! stores so it can be replayed with nothing else.
//!
//! The interpreter reads only what the simulation needs: the asset map a
//! lift copies into its directory (`assets.toml`) and the files of the
//! assets a level's grid, tileset, layout, spawn objects and spawn tables
//! come from (an object-list asset is a file per entry). Tiles and
//! palettes are presentation, so they stay out. The files are decoded by
//! `gp_assets::encode`, the same code `gp assets pack` uses, so the
//! interpreter reads them exactly as packing does.

use std::collections::BTreeMap;
use std::path::Path;

use gp_assets::map::{Asset, AssetMap};
use gp_model::Model;
use serde::{Deserialize, Serialize};

/// The file a lifted directory keeps its asset map in.
pub const MAP_FILE: &str = "assets.toml";

/// The bundle format's version.
pub const BUNDLE_SCHEMA: u32 = 1;

/// Lifted asset files by name in the lifted directory, `assets.toml`
/// included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifted {
    pub files: BTreeMap<String, String>,
}

impl Lifted {
    /// Reads, from a lifted directory, the asset map and the files of the
    /// assets the model's slots name that the simulation reads.
    pub fn read_dir(dir: &Path, model: &Model) -> Result<Self, String> {
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name))
                .map_err(|e| format!("{}: {e}", dir.join(name).display()))
        };
        let mut files = BTreeMap::new();
        let map_text = read(MAP_FILE)?;
        let map = AssetMap::parse(&map_text).map_err(|e| format!("{MAP_FILE}: {e}"))?;
        for asset in needed(model, &map)? {
            for file in asset.files() {
                files.insert(file.clone(), read(&file)?);
            }
        }
        files.insert(MAP_FILE.into(), map_text);
        Ok(Self { files })
    }

    pub fn map(&self) -> Result<AssetMap, String> {
        let text = self
            .files
            .get(MAP_FILE)
            .ok_or(format!("the lifted assets have no {MAP_FILE}"))?;
        AssetMap::parse(text).map_err(|e| format!("{MAP_FILE}: {e}"))
    }

    /// The bytes each span of `asset` packs to, by part name (`level`,
    /// `metatiles.solid`, `slugs.start (low bytes)`).
    pub fn decode(&self, map: &AssetMap, asset: &str) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let one = map
            .assets
            .iter()
            .find(|a| a.name() == asset)
            .ok_or(format!("the asset map has no asset `{asset}`"))?;
        let sub = AssetMap {
            assets: vec![one.clone()],
            ..map.clone()
        };
        let read = |file: &str| {
            self.files
                .get(file)
                .map(|t| t.as_bytes().to_vec())
                .ok_or_else(|| std::io::Error::other(format!("{file} wasn't read")))
        };
        let encoded = gp_assets::encode(&sub, &read).map_err(|e| e.to_string())?;
        Ok(encoded
            .into_iter()
            .map(|e| (e.span.part, e.bytes))
            .collect())
    }
}

/// The asset slots the simulation reads: each level's grid, tileset,
/// layout, spawn objects and spawn tables. Every other slot is
/// presentation, which a stub or a theme may replace without changing
/// how the game plays (docs/stubs.md).
pub fn simulation_slots(model: &Model) -> Vec<&String> {
    let mut slots = Vec::new();
    for level in model.levels.values() {
        slots.extend(&level.core.grid);
        slots.extend(&level.core.tileset);
        slots.extend(level.core.layout.iter().map(|l| &l.slot));
        slots.extend(level.core.spawn_objects.iter().map(|l| &l.slot));
        slots.extend(level.core.spawn.iter().filter_map(|s| s.table.as_ref()));
        // Platformer `level.ladders`: its places and shapes.
        // And `ladder_tops` and `ladder_stops`, read the same way.
        for l in [
            &level.platformer.ladders,
            &level.platformer.ladder_tops,
            &level.platformer.ladder_stops,
        ]
        .into_iter()
        .flatten()
        {
            // A list in a byte table (`level.ladder_lists`) names
            // `slot.column`.
            for of in [&l.places, &l.shapes] {
                match of.split_once('.') {
                    None => slots.push(of),
                    Some((slot, _)) => {
                        slots.extend(model.assets.get_key_value(slot).map(|(k, _)| k))
                    }
                }
            }
        }
        // Platformer `level.ground`: its runs and shapes.
        if let Some(g) = &level.platformer.ground {
            slots.push(&g.runs);
            slots.push(&g.shapes);
        }
    }
    // The tables hooks read (`slot.part`), so a stub keeps them.
    for e in model.entities.values() {
        for table in e.core.ext_tables.values() {
            if let Some((name, _)) = table
                .split_once('.')
                .and_then(|(slot, _)| model.assets.get_key_value(slot))
            {
                slots.push(name);
            }
        }
        for slot in e.core.ext_rows.values() {
            if let Some((name, _)) = model.assets.get_key_value(slot) {
                slots.push(name);
            }
        }
    }
    slots
}

/// The assets of the slots a level reads: its grid, its tileset and its
/// spawn tables, and the tables hooks read, filled from this map's source.
fn needed<'a>(model: &Model, map: &'a AssetMap) -> Result<Vec<&'a Asset>, String> {
    for (name, e) in &model.entities {
        for table in e.core.ext_tables.values() {
            let slot = table
                .split_once('.')
                .map_or(table.as_str(), |(slot, _)| slot);
            if !model.assets.contains_key(slot) {
                return Err(format!(
                    "entity `{name}`'s ext table `{table}` names no asset slot `{slot}`"
                ));
            }
        }
        for slot in e.core.ext_rows.values() {
            if !model.assets.contains_key(slot) {
                return Err(format!(
                    "entity `{name}`'s ext rows `{slot}` name no asset slot"
                ));
            }
        }
    }
    let mut out: Vec<&Asset> = Vec::new();
    for slot in simulation_slots(model) {
        let s = model
            .assets
            .get(slot)
            .ok_or(format!("the model has no asset slot `{slot}`"))?;
        let from = s.from.as_ref().ok_or(format!(
            "asset slot `{slot}` has no `from`, so nothing fills it"
        ))?;
        if from.source != map.source {
            return Err(format!(
                "asset slot `{slot}` is filled from `{}`, but the lifted assets are `{}`'s",
                from.source, map.source
            ));
        }
        let asset = map
            .assets
            .iter()
            .find(|a| a.name() == from.asset)
            .ok_or(format!(
                "asset slot `{slot}` names `{}`, which the asset map doesn't have",
                from.asset
            ))?;
        if !out.iter().any(|a| a.name() == asset.name()) {
            out.push(asset);
        }
    }
    Ok(out)
}

/// Everything a model runtime needs to run a model: its canonical text and
/// the lifted files it reads. A recording names its bundle by hash and the
/// store keeps it, so replay needs no model directory or lifted assets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub schema: u32,
    /// The model's canonical text (`gp model fmt`), so two ways of writing
    /// the same model give the same bundle.
    pub model: String,
    pub assets: Lifted,
}

impl Bundle {
    pub fn new(model: &Model, assets: Lifted) -> Self {
        Self {
            schema: BUNDLE_SCHEMA,
            model: model.to_toml(),
            assets,
        }
    }

    /// JSON with keys in order, so the same bundle is the same bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a bundle serializes")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let b: Bundle = serde_json::from_slice(bytes).map_err(|e| format!("model bundle: {e}"))?;
        if b.schema != BUNDLE_SCHEMA {
            return Err(format!(
                "model bundle schema {}; this build reads {BUNDLE_SCHEMA}",
                b.schema
            ));
        }
        Ok(b)
    }

    pub fn parse_model(&self) -> Result<Model, String> {
        Model::parse(&self.model).map_err(|e| format!("the bundle's model: {e}"))
    }
}
