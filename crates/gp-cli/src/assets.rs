//! `gp assets`: lift a ROM's assets into files and pack them back, with a
//! byte-for-byte check. The formats are in `gp-assets`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use gp_assets::{AssetMap, MAP_FILE, RomLayout};
use serde_json::json;

use crate::Outcome;

/// Lifted assets must never be committed, so an output inside a git work
/// tree is refused.
fn outside_repo(path: &Path) -> Result<std::path::PathBuf> {
    let path = std::path::absolute(path).with_context(|| path.display().to_string())?;
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&path) {
        bail!(
            "{} is inside the git work tree at {}; lifted assets stay outside any repo",
            path.display(),
            repo.display()
        );
    }
    Ok(path)
}

pub fn lift(rom_path: &Path, out: &Path, map_path: &Path, force: bool) -> Result<Outcome> {
    let rom = fs::read(rom_path).with_context(|| rom_path.display().to_string())?;
    let map_text = fs::read_to_string(map_path).with_context(|| map_path.display().to_string())?;
    let map = AssetMap::parse(&map_text).with_context(|| map_path.display().to_string())?;
    let out = outside_repo(out)?;
    if !force && out.join(MAP_FILE).exists() {
        bail!(
            "{} already holds a lift; pass --force to replace it",
            out.display()
        );
    }
    let (files, lifted) = gp_assets::lift(&rom, &map, &map_text)?;
    for f in &files {
        let path = out.join(&f.name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &f.bytes).with_context(|| path.display().to_string())?;
    }
    Ok(Outcome {
        json: json!({ "out": out, "assets": lifted.len(), "files": files.len() }),
        text: format!("lifted {} assets into {}", lifted.len(), out.display()),
        exit: 0,
    })
}

pub fn check_dir(dir: &Path, rom_path: &Path) -> Result<Outcome> {
    let rom = fs::read(rom_path).with_context(|| rom_path.display().to_string())?;
    let map_path = dir.join(MAP_FILE);
    let text = fs::read_to_string(&map_path).with_context(|| map_path.display().to_string())?;
    let map = AssetMap::parse(&text).with_context(|| map_path.display().to_string())?;
    let encoded = gp_assets::encode(&map, &|name| fs::read(dir.join(name)))?;
    let diffs = gp_assets::differences(&rom, &map, &encoded)?;
    let ok = diffs.is_empty() && RomLayout::of(&rom)?.is_for(&map);
    Ok(Outcome {
        json: json!({ "ok": ok, "differences": diffs.len() }),
        text: if ok {
            format!(
                "round trip ok: {} assets identical to the ROM",
                map.assets.len()
            )
        } else {
            format!("round trip FAILED: {} bytes differ", diffs.len())
        },
        exit: u8::from(!ok),
    })
}
