//! `gp model`: check a model and its fidelity contract, and hold them to
//! their canonical form.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use gp_assets::MAP_FILE;
use gp_assets::map::{Asset, AssetMap};
use gp_contract::Contract;
use gp_model::model::AssetKind;
use gp_model::{Model, validate};
use serde_json::{Value, json};

use crate::Outcome;

const MODEL_FILE: &str = "model.toml";
const CONTRACT_FILE: &str = "fidelity.toml";

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn problem(file: &str, path: &str, message: &str) -> Value {
    json!({ "file": file, "path": path, "message": message })
}

pub fn check(dir: &Path, assets: Option<&Path>) -> Result<Outcome> {
    if !dir.is_dir() {
        bail!("{} isn't a directory", dir.display());
    }
    let text = read(&dir.join(MODEL_FILE))?;
    let model = Model::parse(&text).with_context(|| dir.join(MODEL_FILE).display().to_string())?;
    let mut problems: Vec<Value> = validate::validate(&model)
        .into_iter()
        .map(|p| problem(MODEL_FILE, &p.path, &p.message))
        .collect();
    let profile_file = gp_model::profile::PROFILES_FILE;
    match gp_model::profile::Profiles::read_dir(dir) {
        Err(e) => problems.push(problem(profile_file, "", &e)),
        Ok(p) => {
            for x in p.check(&model) {
                problems.push(problem(profile_file, &x.path, &x.message));
            }
        }
    }
    let contract_path = dir.join(CONTRACT_FILE);
    if contract_path.exists() {
        match Contract::parse(&read(&contract_path)?) {
            Err(e) => problems.push(problem(CONTRACT_FILE, "", &e.to_string())),
            Ok(c) => {
                for p in c.check(&model) {
                    problems.push(problem(CONTRACT_FILE, &p.at, &p.message));
                }
            }
        }
    }
    let mut checked = Vec::new();
    if let Some(a) = assets {
        let map_text = read(&a.join(MAP_FILE))?;
        let map =
            AssetMap::parse(&map_text).with_context(|| a.join(MAP_FILE).display().to_string())?;
        for (path, message) in check_slots(&model, &map, &mut checked) {
            problems.push(problem(MODEL_FILE, &path, &message));
        }
    }
    let lines: Vec<String> = problems
        .iter()
        .map(|p| format!("problem  {}  {}: {}", p["file"], p["path"], p["message"]))
        .collect();
    let ok = problems.is_empty();
    let text = if ok {
        format!(
            "{}: ok ({} asset slots checked)",
            dir.display(),
            checked.len()
        )
    } else {
        lines.join("\n")
    };
    Ok(Outcome {
        json: json!({ "ok": ok, "problems": problems, "slots": checked }),
        text,
        exit: u8::from(!ok),
    })
}

pub fn fmt_check(dir: &Path) -> Result<Outcome> {
    let mut stale = Vec::new();
    let model_path = dir.join(MODEL_FILE);
    let text = read(&model_path)?;
    let canon = Model::parse(&text)
        .with_context(|| model_path.display().to_string())?
        .to_toml();
    if text != canon {
        stale.push(model_path);
    }
    let contract_path = dir.join(CONTRACT_FILE);
    if contract_path.exists() {
        let text = read(&contract_path)?;
        let canon = Contract::parse(&text)
            .with_context(|| contract_path.display().to_string())?
            .to_toml();
        if text != canon {
            stale.push(contract_path);
        }
    }
    let ok = stale.is_empty();
    let names: Vec<String> = stale.iter().map(|p| p.display().to_string()).collect();
    Ok(Outcome {
        json: json!({ "ok": ok, "not_canonical": names }),
        text: if ok {
            format!("{}: canonical", dir.display())
        } else {
            format!("not in canonical form: {}", names.join(", "))
        },
        exit: u8::from(!ok),
    })
}

/// Checks each slot filled from `map`'s source against the asset it names:
/// the format fits the slot's kind, and the model's uses of it fit the
/// asset (level sizes, tileset properties and item counts, table columns,
/// sprite tiles). Returns (model path, message) for each problem and lists
/// the slots checked.
fn check_slots(model: &Model, map: &AssetMap, checked: &mut Vec<String>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut found: BTreeMap<&str, &Asset> = BTreeMap::new();
    for (name, slot) in &model.assets {
        let Some(from) = slot.from.as_ref().filter(|f| f.source == map.source) else {
            continue;
        };
        let at = format!("assets.{name}.from");
        match map.asset(&from.asset) {
            None => out.push((
                at,
                format!("{} has no asset named `{}`", map.source, from.asset),
            )),
            Some(a) if !slot.kind.map_formats().contains(&a.format()) => out.push((
                at,
                format!(
                    "`{}` is {}, but a {} slot holds {}",
                    from.asset,
                    a.format(),
                    slot.kind.name(),
                    slot.kind.map_formats().join(" or ")
                ),
            )),
            Some(a) if slot.kind == AssetKind::Font && a.charset().is_none() => out.push((
                at,
                format!("`{}` has no charset, which a font slot needs", from.asset),
            )),
            Some(a) => {
                found.insert(name, a);
                checked.push(name.clone());
            }
        }
        // One role per metatile, or per palette.
        let items = match found.get(name.as_str()) {
            Some(Asset::Metatiles { count, .. }) => Some((*count, "metatiles")),
            Some(Asset::Palette { count, .. }) => Some((*count, "palettes")),
            _ => None,
        };
        if let Some((count, what)) = items
            && !slot.item_roles.is_empty()
            && slot.item_roles.len() != count as usize
        {
            out.push((
                format!("assets.{name}.item_roles"),
                format!("{} roles for {count} {what}", slot.item_roles.len()),
            ));
        }
    }
    for (name, l) in &model.levels {
        let at = format!("levels.{name}");
        if let Some(grid) = &l.core.grid
            && let Some(Asset::Level { width, height, .. }) = found.get(grid.as_str())
            && [*width, *height] != l.core.size
        {
            out.push((
                format!("{at}.size"),
                format!("the grid `{grid}` is {width} by {height}"),
            ));
        }
        if let (Some(tileset), Some(solid)) = (&l.core.tileset, &l.core.solid_property)
            && let Some(Asset::Metatiles { properties, .. }) = found.get(tileset.as_str())
            && !properties.iter().any(|p| &p.name == solid)
        {
            out.push((
                format!("{at}.solid_property"),
                format!("the tileset `{tileset}` has no property `{solid}`"),
            ));
        }
        for (i, s) in l.core.spawn.iter().enumerate() {
            let Some(Asset::Table { columns, .. }) = s.table.as_deref().and_then(|t| found.get(t))
            else {
                continue;
            };
            for (field, column) in &s.columns {
                if !columns.iter().any(|c| &c.name == column) {
                    out.push((
                        format!("{at}.spawn[{i}].columns.{field}"),
                        format!("the table has no column `{column}`"),
                    ));
                }
            }
        }
    }
    for (name, e) in &model.entities {
        let Some(sprite) = &e.core.sprite else {
            continue;
        };
        if let Some(Asset::Chr { tiles, .. }) = found.get(sprite.slot.as_str())
            && let Some(t) = sprite.tiles.iter().find(|t| **t >= *tiles)
        {
            out.push((
                format!("entities.{name}.sprite.tiles"),
                format!("tile {t} is past the {tiles} tiles in `{}`", sprite.slot),
            ));
        }
        if let Some(slot) = model
            .assets
            .values()
            .find(|s| s.kind == AssetKind::Palettes)
            && let Some(from) = slot.from.as_ref().filter(|f| f.source == map.source)
            && let Some(Asset::Palette { count, .. }) = map.asset(&from.asset)
            && sprite.palette >= *count
        {
            out.push((
                format!("entities.{name}.sprite.palette"),
                format!("palette {} is past the {count} palettes", sprite.palette),
            ));
        }
    }
    out
}
