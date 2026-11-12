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

/// Writes a file, refusing to follow a symlink there: one could point into
/// a git work tree, or anywhere else, past the check on its directory.
fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "{} is a symlink; gp assets writes only plain files",
            path.display()
        );
    }
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

pub fn lift(rom_path: &Path, out: &Path, map_path: &Path, force: bool) -> Result<Outcome> {
    let rom = fs::read(rom_path).with_context(|| rom_path.display().to_string())?;
    let map_text = fs::read_to_string(map_path).with_context(|| map_path.display().to_string())?;
    let map = AssetMap::parse(&map_text).with_context(|| map_path.display().to_string())?;
    let out = outside_repo(out)?;
    // symlink_metadata, so a dangling link to a map counts as a lift too.
    if !force && fs::symlink_metadata(out.join(MAP_FILE)).is_ok() {
        bail!(
            "{} already holds a lift; pass --force to replace it",
            out.display()
        );
    }
    let (files, lifted) = gp_assets::lift(&rom, &map, &map_text)?;
    if fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "{} is a symlink; gp assets writes only plain files and directories",
            out.display()
        );
    }
    fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    for f in &files {
        // An object-list asset lifts to a directory of files, one per
        // entry. The directory must be a real one, for the same reason a
        // file mustn't be a symlink.
        let path = out.join(&f.name);
        if let Some(parent) = path.parent().filter(|p| *p != out) {
            if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
                bail!(
                    "{} is a symlink; gp assets writes only plain files and directories",
                    parent.display()
                );
            }
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        write_file(&path, &f.bytes)?;
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

#[cfg(test)]
mod symlink_tests {
    use super::write_file;

    #[test]
    fn write_file_refuses_a_symlink() {
        let dir = std::env::temp_dir().join(format!("gp-assets-symlink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target.bin");
        std::fs::write(&target, b"old").unwrap();
        let link = dir.join("link.bin");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = write_file(&link, b"new").unwrap_err().to_string();
        assert!(err.contains("is a symlink"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        write_file(&dir.join("plain.bin"), b"new").unwrap();
        assert_eq!(std::fs::read(dir.join("plain.bin")).unwrap(), b"new");
    }
}
