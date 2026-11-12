//! `gp model`: check a model and its fidelity contract, and hold them to
//! their canonical form. Ported from game-printer's model.rs: the evidence
//! refs are checked against the repo or the package folder and the pinned
//! downloads (`crate::pinned`), and `fmt` rewrites as well as checks. The
//! facts by source, once metrics, are in the report.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

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
    let home = gp_evidence::Paths::from_env()?.home;
    check_in(dir, assets, None, &home)
}

/// The directory a model's evidence refs are relative to by default: a
/// package folder (with its `package.toml`) for a game's model, else the
/// git work tree the model is in, else the model's own directory.
fn ref_root(dir: &Path) -> PathBuf {
    if dir.join("package.toml").is_file() {
        return dir.to_path_buf();
    }
    gp_evidence::paths::enclosing_git_work_tree(dir).unwrap_or_else(|| dir.to_path_buf())
}

/// `gp model check`: the model against the schema, each evidence ref's file
/// and line (in the repo or a pinned download under `$GP_HOME/work`), the
/// profiles and the contract, the screen-copy counters, and with `assets`
/// the asset slots. Reports the facts by source.
pub fn check_in(
    dir: &Path,
    assets: Option<&Path>,
    root: Option<&Path>,
    gp_home: &Path,
) -> Result<Outcome> {
    if !dir.is_dir() {
        bail!("{} isn't a directory", dir.display());
    }
    let model_path = dir.join(MODEL_FILE);
    let text = read(&model_path)?;
    let mut problems = Vec::new();
    let model = match Model::parse(&text) {
        Ok(m) => m,
        Err(e) => {
            problems.push(problem(MODEL_FILE, "", &e.to_string()));
            return Ok(report(dir, None, problems, json!(null), BTreeMap::new()));
        }
    };
    for p in validate::validate(&model) {
        problems.push(problem(MODEL_FILE, &p.path, &p.message));
    }
    let root = root.map_or_else(|| ref_root(dir), Path::to_path_buf);
    let mut resolver = crate::pinned::Resolver::new(gp_home);
    let (ref_problems, skipped) =
        validate::check_refs_in(&model, |within, file| resolver.root(within, file, &root));
    for p in ref_problems {
        problems.push(problem(MODEL_FILE, &p.path, &p.message));
    }

    // Profiles: each path the model has, a mechanic's parameter or option,
    // and the model each makes valid.
    let profile_file = gp_model::profile::PROFILES_FILE;
    let mut profile_names = Vec::new();
    let mut switch_names: Vec<String> = Vec::new();
    match gp_model::profile::Profiles::read_dir(dir) {
        Err(e) => problems.push(problem(profile_file, "", &e)),
        Ok(p) => {
            for x in p.check(&model) {
                problems.push(problem(profile_file, &x.path, &x.message));
            }
            if !p.is_empty() {
                profile_names = p.names();
                switch_names = p.switches().map(|(_, s)| s.name.clone()).collect();
            }
        }
    }

    let mut contract_json = json!(null);
    let contract_path = dir.join(CONTRACT_FILE);
    if contract_path.exists() {
        match Contract::parse(&read(&contract_path)?) {
            Err(e) => problems.push(problem(CONTRACT_FILE, "", &e.to_string())),
            Ok(c) => {
                for p in c.check(&model) {
                    problems.push(problem(CONTRACT_FILE, &p.at, &p.message));
                }
                let summary: BTreeMap<String, usize> = c
                    .bind(&model)
                    .summary(&model)
                    .into_iter()
                    .map(|(l, n)| (l.name().to_owned(), n))
                    .collect();
                contract_json = json!(summary);
            }
        }
    }
    let mapping_path = dir.join(gp_interp::mapping::MAPPING_FILE);
    if let Ok(mt) = fs::read_to_string(&mapping_path)
        && let Ok(mapping) = gp_interp::mapping::Mapping::parse(&mt)
    {
        for (path, message) in screen_copies(&model, &mapping) {
            problems.push(problem(MODEL_FILE, &path, &message));
        }
    }
    let mut checked_slots = Vec::new();
    if let Some(adir) = assets {
        let map_text = read(&adir.join(MAP_FILE))?;
        let found = if gp_assets::arcade::is_arcade(&map_text) {
            // An arcade lift: its slots name assets by the board's regions,
            // and no slot kind has an arcade format yet, so a slot is held
            // to the asset being there.
            let map = gp_assets::arcade::ArcadeMap::parse(&map_text)
                .with_context(|| adir.join(MAP_FILE).display().to_string())?;
            check_arcade_slots(&model, &map, &mut checked_slots)
        } else {
            let map = AssetMap::parse(&map_text)
                .with_context(|| adir.join(MAP_FILE).display().to_string())?;
            check_slots(&model, &map, &mut checked_slots)
        };
        for (path, message) in found {
            problems.push(problem(MODEL_FILE, &path, &message));
        }
    }

    // Facts by source.
    let mut by_source: BTreeMap<String, usize> = BTreeMap::new();
    for (_, key) in &validate::evidence_for(&model) {
        let source = key
            .as_ref()
            .and_then(|k| model.evidence.get(k))
            .map_or("none", |e| e.source.name());
        *by_source.entry(source.to_owned()).or_insert(0) += 1;
    }
    let mut out = report(dir, Some(&model), problems, contract_json, by_source);
    out.json["refs_skipped"] = json!(skipped);
    out.json["profiles"] = json!(profile_names);
    out.json["switches"] = json!(switch_names);
    if !profile_names.is_empty() {
        let _ = write!(out.text, "; profiles: {}", profile_names.join(", "));
    }
    if skipped > 0 {
        let fetch: Vec<String> = resolver
            .missing
            .iter()
            .map(|(name, cmd)| format!("{name} ({cmd})"))
            .collect();
        let _ = write!(
            out.text,
            "\nnotice: {skipped} evidence refs into pinned downloads weren't checked, because these aren't in {}: {}",
            gp_home.join("work").display(),
            fetch.join(", ")
        );
    }
    if let Some(adir) = assets {
        let _ = write!(
            out.text,
            "; asset slots checked against {}: {}",
            adir.display(),
            if checked_slots.is_empty() {
                "none".to_owned()
            } else {
                checked_slots.join(", ")
            }
        );
        out.json["assets_checked"] = json!(checked_slots);
    }
    Ok(out)
}

fn report(
    dir: &Path,
    model: Option<&Model>,
    problems: Vec<Value>,
    contract: Value,
    by_source: BTreeMap<String, usize>,
) -> Outcome {
    let ok = problems.is_empty();
    let facts: usize = by_source.values().sum();
    let mut text = String::new();
    for p in &problems {
        let at = p["path"].as_str().unwrap_or("");
        let _ = writeln!(
            text,
            "{}{}{}: {}",
            p["file"].as_str().unwrap_or(""),
            if at.is_empty() { "" } else { " " },
            at,
            p["message"].as_str().unwrap_or("")
        );
    }
    if let Some(m) = model {
        let sources: Vec<String> = by_source.iter().map(|(s, n)| format!("{n} {s}")).collect();
        let _ = write!(
            text,
            "{}: {} ({}), {} facts: {}",
            dir.display(),
            if ok { "valid" } else { "invalid" },
            m.model.name,
            facts,
            sources.join(", ")
        );
        if let Some(c) = contract.as_object() {
            let levels: Vec<String> = c.iter().map(|(l, n)| format!("{n} {l}")).collect();
            let _ = write!(text, "; contract: {}", levels.join(", "));
        }
        let _ = write!(text, "; {} features", m.features_used().len());
    } else {
        let _ = write!(text, "{}: invalid", dir.display());
    }
    Outcome {
        json: json!({
            "ok": ok,
            "dir": dir,
            "model": model.map(|m| m.model.name.clone()),
            "facts": facts,
            "facts_by_source": by_source,
            "features": model.map(|m| m.features_used().into_iter().collect::<Vec<_>>()),
            "contract": contract,
            "problems": problems,
        }),
        text,
        exit: u8::from(!ok),
    }
}

/// Whether the model and contract are in canonical form, as the gate asks:
/// [`fmt`] with `--check`.
pub fn fmt_check(dir: &Path) -> Result<Outcome> {
    fmt(dir, true)
}

/// `gp model fmt`: rewrites model.toml and fidelity.toml in their canonical
/// form; with `check`, writes nothing and exits 1 if either isn't in it.
pub fn fmt(dir: &Path, check: bool) -> Result<Outcome> {
    let mut files = Vec::new();
    let model_path = dir.join(MODEL_FILE);
    let text = read(&model_path)?;
    let canon = Model::parse(&text)
        .with_context(|| model_path.display().to_string())?
        .to_toml();
    files.push((model_path, text, canon));
    let contract_path = dir.join(CONTRACT_FILE);
    if contract_path.exists() {
        let text = read(&contract_path)?;
        let canon = Contract::parse(&text)
            .with_context(|| contract_path.display().to_string())?
            .to_toml();
        files.push((contract_path, text, canon));
    }
    let mut changed = Vec::new();
    for (path, text, canon) in &files {
        if text != canon {
            changed.push(path.clone());
            if !check {
                fs::write(path, canon).with_context(|| format!("writing {}", path.display()))?;
            }
        }
    }
    let names: Vec<String> = changed.iter().map(|p| p.display().to_string()).collect();
    let text = match (check, changed.is_empty()) {
        (_, true) => format!("{}: canonical", dir.display()),
        (true, false) => format!(
            "not in canonical form (run gp model fmt): {}",
            names.join(", ")
        ),
        (false, false) => format!("rewrote {}", names.join(", ")),
    };
    Ok(Outcome {
        json: json!({ "dir": dir, "check": check, "changed": changed }),
        text,
        exit: if check && !changed.is_empty() { 1 } else { 0 },
    })
}

/// Each slot filled from an arcade map's source names an asset the map has.
fn check_arcade_slots(
    model: &Model,
    map: &gp_assets::arcade::ArcadeMap,
    checked: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, slot) in &model.assets {
        let Some(from) = slot.from.as_ref().filter(|f| f.source == map.source) else {
            continue;
        };
        if map.assets.iter().any(|a| a.name == from.asset) {
            checked.push(name.clone());
        } else {
            out.push((
                format!("assets.{name}.from"),
                format!("{} has no asset named `{}`", map.source, from.asset),
            ));
        }
    }
    out
}

/// Sprite clips and blinks keyed on a counter the state mapping reads from
/// the screen (the name table or OAM): the screen shows a frame after the
/// game's RAM, so the model's picture would show a frame late.
fn screen_copies(model: &Model, mapping: &gp_interp::mapping::Mapping) -> Vec<(String, String)> {
    let screen: BTreeMap<&str, &str> = mapping
        .fields
        .iter()
        .filter(|f| f.ram.is_empty() && f.cases.iter().all(|c| c.ram.is_none()))
        .filter_map(|f| {
            let from = if !f.nametable.is_empty() {
                "the name table"
            } else if !f.oam.is_empty() {
                "the sprite table (OAM)"
            } else {
                return None;
            };
            Some((f.path.strip_prefix("counters.")?, from))
        })
        .collect();
    let mut out = Vec::new();
    for (name, e) in &model.entities {
        let Some(s) = &e.core.sprite else { continue };
        let blink = s.blink.iter().map(|b| ("blink.when".to_owned(), &b.when));
        let clips = s
            .clips
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("clips[{i}].when"), &c.when));
        for (at, when) in clips.chain(blink) {
            for t in when.counters.iter() {
                if let Some(from) = screen.get(t.name.as_str()) {
                    out.push((
                        format!("entities.{name}.sprite.{at}"),
                        format!(
                            "reads counter `{}`, which the state mapping reads from {from}: it is what the screen shows, a frame after the game's RAM, so the picture shows a frame late; key it on the counter the game writes",
                            t.name
                        ),
                    ));
                }
            }
        }
    }
    out
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
