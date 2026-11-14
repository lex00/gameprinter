//! Every file under a folder, three ways: stopping at the first error,
//! the same but never opening a folder named `sealed`, and quietly passing
//! over what can't be read.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Every file under `dir`.
pub(crate) fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Every file under `dir`, never opening a folder named `sealed` (in any
/// case): the ROM folder's sealed runs stay unread (roms.rs).
pub(crate) fn walk_unsealed(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let e = e?;
        let path = e.path();
        if e.file_type()?.is_dir() {
            if !e.file_name().eq_ignore_ascii_case("sealed") {
                walk_unsealed(&path, out)?;
            }
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Every entry under `dir` that isn't a folder, passing over any folder or
/// entry that can't be read.
pub(crate) fn walk_lenient(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_lenient(&p, out);
        } else {
            out.push(p);
        }
    }
}
