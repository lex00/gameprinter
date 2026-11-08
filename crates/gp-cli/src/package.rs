//! `gp package`: what is asked of a package folder.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use gp_package::Manifest;
use serde_json::json;

use crate::Outcome;

/// Every file under a folder, by its path relative to it.
pub fn files_under(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
                files.insert(rel, fs::read(&p)?);
            }
        }
    }
    Ok(files)
}

/// The hash of a package folder's hook code, as a player's build names it.
pub fn hooks(dir: &Path) -> Result<Outcome> {
    let mut m = Manifest::new("game", "x");
    for (path, bytes) in files_under(&dir.join("ext"))? {
        m.add(&format!("ext/{path}"), &bytes)
            .map_err(|e| anyhow!(e))?;
    }
    let h = m
        .hook_hash()
        .ok_or_else(|| anyhow!("{} has no hook code", dir.display()))?;
    Ok(Outcome {
        json: json!({ "hooks": h }),
        text: h,
        exit: 0,
    })
}
