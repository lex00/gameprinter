//! `gp assets stub` (docs/stubs.md): a copy of a lifted directory with the
//! model's presentation slots replaced by placeholders of the same shape,
//! and the slots' state in `slots.toml`. The slots the simulation reads
//! are those `gp_interp::lifted::simulation_slots` names; they're copied
//! unchanged, so the stubbed game plays exactly as the original.
//!
//! Ported from game-printer's assets_stub.rs as it was. An arcade lift
//! is refused in words until a model slot can name its pictures.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::json;

use gp_assets::{Asset, AssetMap, MAP_FILE};
use gp_contract::Contract;
use gp_mcp::catalog::{self, Origin, SlotState, State, Status};
use gp_model::Model;

use crate::Outcome;
use crate::assets::{outside_repo, write_file};

pub(crate) fn stub(
    model_dir: &Path,
    lifted: &Path,
    out: &Path,
    original_license: Option<&str>,
    force: bool,
) -> Result<Outcome> {
    let model_path = crate::mechanics::model_file(model_dir);
    let model_text = fs::read_to_string(&model_path)
        .with_context(|| format!("reading {}", model_path.display()))?;
    let model = Model::parse(&model_text).with_context(|| model_path.display().to_string())?;
    let contract = match model_path.parent().map(|d| d.join("fidelity.toml")) {
        Some(p) if p.is_file() => Some(
            Contract::parse(&fs::read_to_string(&p)?).with_context(|| p.display().to_string())?,
        ),
        _ => None,
    };
    let map_text = fs::read_to_string(lifted.join(MAP_FILE)).with_context(|| {
        format!(
            "reading {} (is {} a lifted directory?)",
            lifted.join(MAP_FILE).display(),
            lifted.display()
        )
    })?;
    if gp_assets::arcade::is_arcade(&map_text) {
        bail!(
            "{} is an arcade lift: no model slot takes an arcade lift's pictures yet (gp_interp::Lifted reads NES maps), so there are no presentation slots to stub",
            lifted.display()
        );
    }
    let map = AssetMap::parse(&map_text).context(MAP_FILE)?;

    let out = outside_repo(out)?;
    if !force && fs::symlink_metadata(out.join(MAP_FILE)).is_ok() {
        bail!(
            "{} already holds assets ({MAP_FILE}); pass --force to replace them",
            out.display()
        );
    }
    fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;

    let simulation = gp_interp::lifted::simulation_slots(&model);
    let slot_assets = catalog::slot_assets(&model, &map).map_err(anyhow::Error::msg)?;
    // Each asset's slots. An asset that any simulation slot names is
    // simulation, whatever its other slots are.
    let mut slots_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (slot, asset) in &slot_assets {
        slots_of.entry(asset.name()).or_default().push(slot);
    }
    let original = original_license.unwrap_or(catalog::original_license(model_dir));
    let palette_roles = catalog::palette_roles(&model);
    let lifted_state = State::read(lifted).map_err(anyhow::Error::msg)?;

    let mut state = State::new(&model.model.name);
    let mut unslotted = Vec::new();
    let mut stubbed = Vec::new();
    for asset in &map.assets {
        let Some(names) = slots_of.get(asset.name()) else {
            // An asset no slot names, such as Super Mario Bros.' area
            // parser tables: kept as lifted.
            copy_files(lifted, &out, asset)?;
            unslotted.push(asset.name().to_owned());
            continue;
        };
        let reads_sim = names
            .iter()
            .any(|n| simulation.iter().any(|s| s.as_str() == *n));
        let stub = if reads_sim {
            None
        } else {
            match asset {
                Asset::Chr {
                    tiles,
                    tiles_per_row,
                    ..
                } => Some(gp_assets::stub::tiles_png(*tiles, *tiles_per_row)),
                Asset::Palette { count, .. } => {
                    let roles: Vec<Option<&str>> = (0..*count)
                        .map(|i| palette_roles.get(&i).filter(|r| r.len() == 1).map(|r| r[0]))
                        .collect();
                    Some(gp_assets::stub::palettes_toml(asset.name(), &roles).into_bytes())
                }
                _ => None,
            }
        };
        let record = match &stub {
            Some(bytes) => {
                write_file(&out.join(asset.file()), bytes)?;
                stubbed.extend(names.iter().map(|n| (*n).to_owned()));
                SlotState {
                    status: Status::Stub,
                    origin: Origin::Stub,
                    license: catalog::STUB_LICENSE.into(),
                    sha256: String::new(),
                    generator: Some("gp assets stub".into()),
                    author: None,
                    note: None,
                }
            }
            None => {
                copy_files(lifted, &out, asset)?;
                // A slot the lifted directory already has a state for (a
                // replacement, say) keeps it.
                match lifted_state.as_ref().and_then(|s| s.slots.get(names[0])) {
                    Some(s) => s.clone(),
                    None => SlotState {
                        status: Status::Original,
                        origin: Origin::Source,
                        license: original.to_owned(),
                        sha256: String::new(),
                        generator: None,
                        author: None,
                        note: Some(format!(
                            "lifted from {} (ROM sha256 {})",
                            map.source, map.sha256
                        )),
                    },
                }
            }
        };
        let mut record = record;
        record.sha256 = catalog::asset_digest(&out, asset).map_err(anyhow::Error::msg)?;
        for n in names {
            state.slots.insert((*n).to_owned(), record.clone());
        }
    }
    write_file(&out.join(MAP_FILE), map_text.as_bytes())?;
    write_file(&out.join(catalog::STATE_FILE), state.to_toml().as_bytes())?;

    let records = catalog::records(
        &model,
        contract.as_ref(),
        Some(&map),
        Some(&state),
        Some(&out),
    );
    let mut text = format!(
        "stubbed {} into {}: {} of {} slots replaced by placeholders",
        model.model.name,
        out.display(),
        stubbed.len(),
        records.len()
    );
    for r in &records {
        text.push_str(&format!(
            "\n  {:<16} {:<12} {:<9} {}",
            r["name"].as_str().unwrap_or(""),
            r["reads"].as_str().unwrap_or(""),
            r["status"].as_str().unwrap_or(""),
            r["kind"].as_str().unwrap_or(""),
        ));
    }
    text.push_str(&format!(
        "\n  state: {}",
        out.join(catalog::STATE_FILE).display()
    ));
    Ok(Outcome::ok(
        json!({
            "model": model.model.name,
            "out": out,
            "stubbed": stubbed,
            "unslotted": unslotted,
            "slots": records,
        }),
        text,
    ))
}

fn copy_files(from: &Path, to: &Path, asset: &Asset) -> Result<()> {
    for f in asset.files() {
        let dst = to.join(&f);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let bytes = fs::read(from.join(&f))
            .with_context(|| format!("reading {}", from.join(&f).display()))?;
        write_file(&dst, &bytes)?;
    }
    Ok(())
}
