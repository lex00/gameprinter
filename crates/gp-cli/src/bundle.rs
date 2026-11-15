//! The files a released gp carries, so a theme or game maker needs no
//! gameprinter checkout (issue #11).
//!
//! A gp built with the `bundle` feature (`just release`) holds the built
//! files it reads at run time, laid out as they are in a checkout:
//! `editor/dist/` (gp serve's editor), `mechanic/` (the game's page),
//! `scripts/web/page/` (the preview's files), `target/play-web/` (the two
//! players), `page/` (the public page, built and release-checked), and
//! `module-kit.zip` (the module kit, `gp game kit`'s output). A dev build
//! carries nothing and reads the checkout as before.
//!
//! The first command that needs them unpacks them into
//! `$GP_HOME/work/bundle/<ID>/`, a folder shaped like a checkout, and
//! [`crate::repo::find_repo`] falls back to it after the working folder
//! and the checkout gp was built from. So the files on disk in a checkout
//! always win.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

/// The module kit's archive in the bundle.
const KIT: &str = "module-kit.zip";

/// Whether this gp carries its files.
pub(crate) fn carried() -> bool {
    !FILES.is_empty()
}

/// The carried file at `rel`.
pub(crate) fn file(rel: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(p, _)| *p == rel).map(|(_, b)| *b)
}

/// How many files and bytes this gp carries under `prefix` ("" for all).
pub(crate) fn count(prefix: &str) -> (usize, usize) {
    FILES
        .iter()
        .filter(|(p, _)| p.starts_with(prefix))
        .fold((0, 0), |(n, b), (_, x)| (n + 1, b + x.len()))
}

/// The bundle's id: a hash of every file in it.
pub(crate) fn id() -> &'static str {
    ID
}

/// The folder the bundle is unpacked into, unpacking it the first time.
/// None for a gp that carries nothing, or when it can't be written.
pub(crate) fn root() -> Option<PathBuf> {
    if !carried() {
        return None;
    }
    static ROOT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let home = gp_evidence::Paths::from_env().ok()?.home;
        match unpack(&home) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("gp: couldn't unpack the files this gp carries: {e:#}");
                None
            }
        }
    })
    .clone()
}

fn unpack(home: &Path) -> Result<PathBuf> {
    let dir = home.join("work").join("bundle").join(ID);
    let done = dir.join(".unpacked");
    if done.is_file() {
        return Ok(dir);
    }
    let tmp = dir.with_extension(format!("tmp{}", std::process::id()));
    for (rel, bytes) in FILES {
        if *rel == KIT {
            continue;
        }
        let to = tmp.join(rel);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p).with_context(|| p.display().to_string())?;
        }
        fs::write(&to, bytes).with_context(|| to.display().to_string())?;
    }
    fs::write(tmp.join(".unpacked"), ID)?;
    // Another gp may have unpacked it meanwhile; theirs is the same.
    if fs::rename(&tmp, &dir).is_err() {
        if done.is_file() {
            let _ = fs::rename(
                &tmp,
                home.join("work")
                    .join("bundle")
                    .join(format!(".old{}", std::process::id())),
            );
            return Ok(dir);
        }
        bail!("couldn't move the unpacked files to {}", dir.display());
    }
    Ok(dir)
}

/// The built public page this gp carries, unpacked: a folder holding its
/// index.html and page.json. None for a gp that carries none.
pub(crate) fn page() -> Option<PathBuf> {
    root()
        .map(|r| r.join("page"))
        .filter(|p| p.join("index.html").is_file())
}

/// Whether this gp carries the module kit.
pub(crate) fn has_kit() -> bool {
    file(KIT).is_some()
}

/// Unpacks the carried module kit into `out`. False when this gp carries
/// none.
pub(crate) fn unpack_kit(out: &Path) -> Result<bool> {
    let Some(bytes) = file(KIT) else {
        return Ok(false);
    };
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .context("reading the carried module kit")?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let Some(rel) = f.enclosed_name() else {
            bail!(
                "the carried module kit names a file outside it: {:?}",
                f.name()
            );
        };
        let to = out.join(rel);
        if f.is_dir() {
            fs::create_dir_all(&to)?;
            continue;
        }
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        let mut b = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut b)?;
        fs::write(&to, b).with_context(|| to.display().to_string())?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_build_carries_nothing_and_unpacks_nothing() {
        if cfg!(feature = "bundle") {
            return;
        }
        assert!(!carried());
        assert!(root().is_none());
        assert!(page().is_none());
        assert!(!has_kit());
        assert_eq!(count(""), (0, 0));
    }
}
