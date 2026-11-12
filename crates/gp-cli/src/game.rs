//! `gp game fetch`: a game by its exact id into a working copy, and its
//! assets lifted from the person's own ROMs, each checked against its pins.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use gp_assets::RomLayout;
use gp_package::registry::Reader;
use serde_json::json;

use crate::Outcome;
use crate::template::{lifted_digest, sha256_hex};

/// Each ROM in the folder, by its headerless SHA-256.
fn roms(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "nes") {
                let bytes = fs::read(&p)?;
                if let Ok(l) = RomLayout::of(&bytes) {
                    out.insert(l.sha256.clone(), bytes);
                }
            }
        }
    }
    Ok(out)
}

pub fn fetch(
    spec: &str,
    id: &str,
    from: &str,
    root_file: Option<&Path>,
    into: &Path,
    rom_dir: &Path,
) -> Result<Outcome> {
    let (name, version) = spec
        .rsplit_once('@')
        .with_context(|| format!("wanted NAME@VERSION, not {spec}"))?;
    let into = std::path::absolute(into)?;
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&into) {
        bail!(
            "{} is inside the git work tree at {}",
            into.display(),
            repo.display()
        );
    }
    let root = root_file.map(crate::registry::read_root).transpose()?;
    let target = crate::registry::open(from);
    let reader = match &root {
        Some(root) => Reader::open(&*target, root),
        None => Reader::unsigned(&*target),
    }
    .map_err(|e| anyhow!(e))?;
    let (_, files) = reader.fetch(name, version, id).map_err(|e| anyhow!(e))?;
    for (path, bytes) in &files {
        let out = into.join(path);
        if let Some(p) = out.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(out, bytes)?;
    }
    let text = |path: &str| -> Result<String> {
        let b = files
            .get(path)
            .ok_or_else(|| anyhow!("the package has no {path}"))?;
        Ok(String::from_utf8(b.clone())?)
    };
    let pkg: toml::Table = text("package.toml")?.parse().context("package.toml")?;
    let entries = pkg
        .get("rom")
        .and_then(|r| r.get("entry"))
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let have = roms(rom_dir)?;
    let mut lines = vec![format!(
        "fetched {spec} ({} files), every {} checked",
        files.len(),
        if root.is_some() {
            "signature and hash"
        } else {
            "hash"
        }
    )];
    let (mut bad, mut results) = (0u32, Vec::new());
    for e in &entries {
        let field = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
        let (rom_id, sha, map_path) = (field("id"), field("sha256"), field("asset_map"));
        let Some(rom) = have.get(&sha) else {
            bad += 1;
            lines.push(format!(
                "MISSING {rom_id}: no ROM with its hash in the ROM folder"
            ));
            continue;
        };
        let map_text = text(&map_path)?;
        let map_sha = sha256_hex(map_text.as_bytes());
        let map_ok = map_sha == field("asset_map_sha256");
        let (lifted, _) =
            crate::assets::lift_files(rom, &map_text).with_context(|| map_path.clone())?;
        let out = into.join("lifted").join(&rom_id);
        let lifted: BTreeMap<String, Vec<u8>> =
            lifted.into_iter().map(|f| (f.name, f.bytes)).collect();
        for (n, b) in &lifted {
            let p = out.join(n);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d)?;
            }
            fs::write(p, b)?;
        }
        let digest = lifted_digest(&lifted);
        let pin = field("lifted_sha256");
        let ok = map_ok && (pin.is_empty() || pin == digest);
        bad += u32::from(!ok);
        lines.push(format!(
            "{} {rom_id}: {} files, lifted {digest}{}{}",
            if ok { "ok     " } else { "FAILED " },
            lifted.len(),
            if pin.is_empty() {
                " (no pin to compare)"
            } else {
                ""
            },
            if map_ok {
                ""
            } else {
                ", asset map isn't its pin"
            },
        ));
        results.push(json!({ "id": rom_id, "ok": ok, "digest": digest, "files": lifted.len() }));
    }
    Ok(Outcome {
        json: json!({ "ok": bad == 0, "files": files.len(), "roms": results }),
        text: lines.join("\n"),
        exit: u8::from(bad > 0),
    })
}
