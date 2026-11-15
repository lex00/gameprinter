//! The themes a runtime can download (docs/theme-publishing.md, "In the
//! game"): its game's themes index from a mirror (`gp_package::index`),
//! each theme's package, and a cache of the looks downloaded. The web
//! preview keeps its cache in the browser and calls these through gp-web;
//! [`install`] and [`remove`] keep one in a folder. Presentation only: the
//! simulation never reads a theme.

use std::path::{Path, PathBuf};

pub use gp_package::index::Theme;
use gp_package::index::{Index, pictures_digest};
use gp_package::{File, Manifest, checked_blob, package_key};

/// The file a downloaded look's folder holds beside `look.txt`: the id of
/// the package it came from. A look folder without it was not downloaded
/// (it shipped with the build), and [`remove`] leaves it.
pub const PACKAGE_FILE: &str = "package.txt";

/// The folder of the cached index copies, in the data folder.
pub const INDEX_DIR: &str = "catalogue";

/// The themes of an index a build can draw: those drawn against its own
/// picture list (`fits`), with an id a look may have.
pub fn offered(index: &str, pictures: &[impl AsRef<str>]) -> Result<Vec<Theme>, String> {
    offered_fits(index, &pictures_digest(pictures))
}

/// The themes of an index drawn against the picture list whose digest is
/// `fits` (a build that knows its digest, as the Bevy build does).
pub fn offered_fits(index: &str, fits: &str) -> Result<Vec<Theme>, String> {
    Ok(offer_fits(index, fits)?.0)
}

/// A theme of an index a build doesn't offer, and why, in words a player
/// reads, written to follow "because".
pub type LeftOut = (Theme, String);

/// The themes of an index a build whose picture list's digest is `fits`
/// offers ([`offered_fits`]), and each one it leaves out with why: a
/// source package (drawn over each player's own ROM art, which no runtime
/// does yet), a look drawn for other pictures, or an id a look can't have.
pub fn offer_fits(index: &str, fits: &str) -> Result<(Vec<Theme>, Vec<LeftOut>), String> {
    let ix = Index::read(index)?;
    let mut offered = Vec::new();
    let mut left = Vec::new();
    for t in ix.themes {
        let why = if !crate::looks::valid_id(&t.id) {
            Some(format!(
                "its id, `{}`, can't name a theme in the settings",
                t.id
            ))
        } else if t.kind == "source" {
            Some("it is drawn over each player's own ROM art, which this build can't do yet".into())
        } else if t.fits != fits {
            Some(format!(
                "it was drawn for another version of this game's pictures (it fits {}, this build is {})",
                short(&t.fits),
                short(fits)
            ))
        } else {
            None
        };
        match why {
            Some(w) => left.push((t, w)),
            None => offered.push(t),
        }
    }
    Ok((offered, left))
}

/// A digest's first 12 digits, or "none" for an empty one.
fn short(h: &str) -> &str {
    if h.is_empty() {
        "none"
    } else {
        &h[..h.len().min(12)]
    }
}

/// [`offer_fits`] for a build's picture names.
pub fn offer(
    index: &str,
    pictures: &[impl AsRef<str>],
) -> Result<(Vec<Theme>, Vec<LeftOut>), String> {
    offer_fits(index, &pictures_digest(pictures))
}

/// A package's look files, by their path in the look's folder, from its
/// manifest's bytes checked against the package id.
pub fn look_files(package: &str, manifest: &[u8]) -> Result<Vec<(String, File)>, String> {
    let m = Manifest::read_checked(package, manifest)?;
    let files: Vec<(String, File)> = m
        .under("look")
        .map(|(rel, f)| (rel.to_owned(), f.clone()))
        .collect();
    if !files.iter().any(|(rel, _)| rel == crate::looks::LOOK_FILE) {
        return Err(format!("package {package} holds no look"));
    }
    Ok(files)
}

/// The package a cached look came from, when it was downloaded.
pub fn cached(themes_dir: &Path, id: &str) -> Option<String> {
    let t = std::fs::read_to_string(themes_dir.join(id).join(PACKAGE_FILE)).ok()?;
    Some(t.trim().to_owned()).filter(|p| gp_package::is_hash(p))
}

/// Downloads a theme's look into `themes_dir/<id>/`: the manifest and each
/// look file through `get` (a mirror key to its bytes), every one checked
/// against its hash before any is written. The files go into a folder
/// beside it, renamed into place when all are there, so a look folder is
/// never half written.
pub fn install(
    themes_dir: &Path,
    theme: &Theme,
    mut get: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    if !crate::looks::valid_id(&theme.id) {
        return Err(format!("`{}` isn't a theme id", theme.id));
    }
    let manifest = get(&package_key(&theme.package))?;
    let files = look_files(&theme.package, &manifest)?;
    let mut bytes = Vec::new();
    for (rel, f) in &files {
        let b = checked_blob(f, get(&gp_package::blob_key(&f.hash))?)?;
        bytes.push((rel.clone(), b));
    }
    let dir = themes_dir.join(&theme.id);
    let part = themes_dir.join(format!(".{}.part", theme.id));
    let io = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    if part.exists() {
        std::fs::remove_dir_all(&part).map_err(|e| io(&part, e))?;
    }
    for (rel, b) in &bytes {
        let p = part.join(rel);
        std::fs::create_dir_all(p.parent().unwrap_or(&part)).map_err(|e| io(&p, e))?;
        std::fs::write(&p, b).map_err(|e| io(&p, e))?;
    }
    std::fs::write(part.join(PACKAGE_FILE), format!("{}\n", theme.package))
        .map_err(|e| io(&part, e))?;
    if cached(themes_dir, &theme.id).is_some() {
        std::fs::remove_dir_all(&dir).map_err(|e| io(&dir, e))?;
    } else if dir.exists() {
        return Err(format!(
            "{} holds a look that wasn't downloaded; it is left as it is",
            dir.display()
        ));
    }
    std::fs::rename(&part, &dir).map_err(|e| io(&dir, e))?;
    Ok(dir)
}

/// Removes a downloaded look from the cache. A look that shipped with the
/// build is refused.
pub fn remove(themes_dir: &Path, id: &str) -> Result<(), String> {
    if !crate::looks::valid_id(id) || cached(themes_dir, id).is_none() {
        return Err(format!("`{id}` isn't a downloaded theme"));
    }
    let d = themes_dir.join(id);
    std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A made-up package with a look, in a map of mirror keys.
    fn mirror(pictures: &[&str]) -> (BTreeMap<String, Vec<u8>>, Index) {
        let mut keys = BTreeMap::new();
        let mut m = Manifest::new("theme", "night");
        for (p, b) in [
            (
                "look/look.txt",
                b"gp-look 1\nid night\ntitle Night\nfile frame-0-0.png\n".to_vec(),
            ),
            ("look/frame-0-0.png", b"\x89PNG made up".to_vec()),
            ("theme/theme.toml", b"id = \"night\"\n".to_vec()),
        ] {
            let h = m.add(p, &b).unwrap();
            keys.insert(gp_package::blob_key(&h), b);
        }
        keys.insert(package_key(&m.id()), m.write().into_bytes());
        let mut ix = Index::new("demo");
        ix.put(Theme {
            id: "night".into(),
            title: "Night".into(),
            package: m.id(),
            fits: pictures_digest(pictures),
            ..Theme::default()
        });
        ix.put(Theme {
            id: "other".into(),
            package: m.id(),
            fits: pictures_digest(&["another.png"]),
            ..Theme::default()
        });
        (keys, ix)
    }

    /// Each theme of the index a build doesn't offer comes back with why:
    /// a look drawn for other pictures, a source package, a bad id.
    #[test]
    fn a_theme_the_build_does_not_offer_is_named_with_why() {
        let pics = ["frame-0-0.png"];
        let (_, mut ix) = mirror(&pics);
        let package = ix.themes[0].package.clone();
        ix.put(Theme {
            id: "rom".into(),
            kind: "source".into(),
            package: package.clone(),
            ..Theme::default()
        });
        ix.put(Theme {
            id: "9lives".into(),
            fits: pictures_digest(&pics),
            package,
            ..Theme::default()
        });
        let (offered, left) = offer(&ix.write(), &pics).unwrap();
        assert_eq!(
            offered.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["night"]
        );
        let why = |id: &str| left.iter().find(|(t, _)| t.id == id).unwrap().1.clone();
        assert!(
            why("other").starts_with("it was drawn for another version of this game's pictures"),
            "{left:?}"
        );
        assert!(why("rom").contains("own ROM art"), "{left:?}");
        assert_eq!(
            why("9lives"),
            "its id, `9lives`, can't name a theme in the settings"
        );
    }

    #[test]
    fn a_look_downloads_checked_and_is_removed_from_the_cache() {
        let pics = ["frame-0-0.png"];
        let (mut keys, ix) = mirror(&pics);
        let offered = offered(&ix.write(), &pics).unwrap();
        assert_eq!(
            offered.len(),
            1,
            "a theme drawn for other pictures isn't offered"
        );
        let t = &offered[0];
        let d = tempfile::tempdir().unwrap();
        let get = |k: &str| keys.get(k).cloned().ok_or_else(|| format!("{k}: 404"));
        let dir = install(d.path(), t, get).unwrap();
        assert_eq!(
            std::fs::read(dir.join("frame-0-0.png")).unwrap(),
            b"\x89PNG made up"
        );
        assert!(
            !dir.join("theme.toml").exists(),
            "only the look is downloaded"
        );
        assert_eq!(
            cached(d.path(), "night").as_deref(),
            Some(t.package.as_str())
        );
        // A changed blob is refused, and the cached look is kept.
        let h = Manifest::read(std::str::from_utf8(&keys[&package_key(&t.package)]).unwrap())
            .unwrap()
            .get("look/frame-0-0.png")
            .unwrap()
            .hash
            .clone();
        keys.insert(gp_package::blob_key(&h), b"\x89PNG changed".to_vec());
        let get = |k: &str| keys.get(k).cloned().ok_or_else(|| format!("{k}: 404"));
        let e = install(d.path(), t, get).unwrap_err();
        assert!(
            e.contains("hash mismatch") || e.contains("bytes where"),
            "{e}"
        );
        assert!(dir.join("frame-0-0.png").is_file());
        remove(d.path(), "night").unwrap();
        assert!(!dir.exists());
        // A look that shipped with the build isn't removed.
        std::fs::create_dir_all(d.path().join("paper")).unwrap();
        assert!(remove(d.path(), "paper").is_err());
        assert!(d.path().join("paper").exists());
    }
}
