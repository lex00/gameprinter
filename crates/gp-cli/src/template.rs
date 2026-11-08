//! `gp template check`: a ROM held to a template's pins: the ROM's hash,
//! the asset map's hash and the digest of what the map lifts.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use gp_assets::{AssetMap, RomLayout};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::Outcome;

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The SHA-256 of `sha256sum`'s output for the files, sorted by name.
fn lifted_digest(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut lines = String::new();
    for (name, bytes) in files {
        let _ = writeln!(lines, "{}  {name}", sha256_hex(bytes));
    }
    sha256_hex(lines.as_bytes())
}

pub fn check(dir: &Path, rom_path: &Path) -> Result<Outcome> {
    let manifest: toml::Table = fs::read_to_string(dir.join("template.toml"))
        .context("reading template.toml")?
        .parse()
        .context("parsing template.toml")?;
    let text = |sect: &str, key: &str| -> Result<String> {
        manifest
            .get(sect)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("template.toml has no {sect}.{key}"))
    };
    let accepts: Vec<String> = manifest
        .get("source")
        .and_then(|s| s.get("accepts"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let map_pin = text("recipe", "asset_map_sha256")?;
    let lifted_pin = text("recipe", "lifted_sha256")?;
    let map_path = dir.join(text("recipe", "asset_map")?);

    let rom = fs::read(rom_path).with_context(|| rom_path.display().to_string())?;
    let layout = RomLayout::of(&rom)?;
    let map_text = fs::read_to_string(&map_path).with_context(|| map_path.display().to_string())?;
    let map = AssetMap::parse(&map_text).with_context(|| map_path.display().to_string())?;
    let (files, _) = gp_assets::lift(&rom, &map, &map_text)?;
    let files: BTreeMap<String, Vec<u8>> = files.into_iter().map(|f| (f.name, f.bytes)).collect();

    let map_sha = sha256_hex(map_text.as_bytes());
    let lifted = lifted_digest(&files);
    let steps = [
        (
            "rom",
            accepts.contains(&layout.sha256),
            layout.sha256.clone(),
        ),
        ("asset map", map_sha == map_pin, map_sha.clone()),
        ("lifted", lifted == lifted_pin, lifted.clone()),
    ];
    let ok = steps.iter().all(|s| s.1);
    let mut out = format!("{}:", dir.display());
    for (name, pass, value) in &steps {
        let _ = write!(
            out,
            "\n  {name:<10} {}{value}",
            if *pass { "" } else { "FAILED: " }
        );
    }
    Ok(Outcome {
        json: json!({ "ok": ok, "rom_sha256": layout.sha256, "asset_map_sha256": map_sha, "lifted_sha256": lifted, "files": files.len() }),
        text: out,
        exit: u8::from(!ok),
    })
}
