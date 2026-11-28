//! The files a released gp carries, so a theme or game maker needs no
//! gameprinter checkout (issue #11).
//!
//! A gp built with the `bundle` feature (`just release-build`) holds the built
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

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

/// The module kit's archive in the bundle.
pub(crate) const KIT: &str = "module-kit.zip";

/// A set of carried files and its id: this gp's own ([`Carried::own`]),
/// or, in a test, any list.
#[derive(Clone, Copy)]
pub(crate) struct Carried<'a> {
    pub files: &'a [(&'a str, &'a [u8])],
    pub id: &'a str,
}

impl Carried<'static> {
    /// The files this gp carries.
    pub(crate) fn own() -> Self {
        Carried {
            files: FILES,
            id: ID,
        }
    }
}

impl<'a> Carried<'a> {
    /// The carried file at `rel`.
    pub(crate) fn file(&self, rel: &str) -> Option<&'a [u8]> {
        let found = self.files.iter().find(|(p, _)| *p == rel);
        found.map(|(_, b)| *b)
    }

    /// The module kit's files by path, read from its archive. None when
    /// these files hold no kit.
    pub(crate) fn kit_files(&self) -> Result<Option<BTreeMap<String, Vec<u8>>>> {
        let Some(bytes) = self.file(KIT) else {
            return Ok(None);
        };
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .context("reading the carried module kit")?;
        let mut out = BTreeMap::new();
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            let Some(rel) = f.enclosed_name() else {
                bail!(
                    "the carried module kit names a file outside it: {:?}",
                    f.name()
                );
            };
            if f.is_dir() {
                continue;
            }
            let rel = rel.to_string_lossy().replace('\\', "/");
            let mut b = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut b)?;
            out.insert(rel, b);
        }
        Ok(Some(out))
    }

    /// The files, all but the kit, unpacked under
    /// `home/work/bundle/<id>/`, the first time only.
    pub(crate) fn unpack(&self, home: &Path) -> Result<PathBuf> {
        unpack(home, self.files, self.id)
    }
}

/// Whether this gp carries its files.
pub(crate) fn carried() -> bool {
    !FILES.is_empty()
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
        match Carried::own().unpack(&home) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("gp: couldn't unpack the files this gp carries: {e:#}");
                None
            }
        }
    })
    .clone()
}

fn unpack(home: &Path, files: &[(&str, &[u8])], id: &str) -> Result<PathBuf> {
    let dir = home.join("work").join("bundle").join(id);
    let done = dir.join(".unpacked");
    if done.is_file() {
        return Ok(dir);
    }
    let tmp = dir.with_extension(format!("tmp{}", std::process::id()));
    for (rel, bytes) in files {
        if *rel == KIT {
            continue;
        }
        let to = tmp.join(rel);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p).with_context(|| p.display().to_string())?;
        }
        fs::write(&to, bytes).with_context(|| to.display().to_string())?;
    }
    fs::create_dir_all(&tmp)?;
    fs::write(tmp.join(".unpacked"), id)?;
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
#[cfg(test)]
pub(crate) fn has_kit() -> bool {
    Carried::own().file(KIT).is_some()
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
        assert!(Carried::own().files.is_empty());
    }

    /// The row page-deploy-out-of-tools: the public page's release check
    /// with ROMs and its deploy live in the games repo, not here. The tools
    /// keep build-page.sh, the page check and pages.sh; no wrangler config,
    /// no deploy script and no recipe that reads a ROM to check the page.
    /// A failure lists what is left.
    #[test]
    fn the_page_deploy_is_out_of_the_tools() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut left = Vec::new();
        for f in [
            "scripts/web/release-check.sh",
            "scripts/web/deploy.sh",
            "scripts/web/deploy-old.sh",
            "scripts/web/test-deploy.sh",
            "scripts/web/wrangler-upload.py",
            "scripts/web/deploy",
            "scripts/web/deploy-old",
        ] {
            if root.join(f).exists() {
                left.push(format!("{f} is still in the repo"));
            }
        }
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(rd) = fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name();
                let name = name.to_string_lossy();
                if p.is_dir() {
                    if !["target", "node_modules", ".git"].contains(&name.as_ref()) {
                        walk(&p, out);
                    }
                } else {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        for d in [
            "scripts", "crates", "mechanic", "plugin", "docs", "site/src",
        ] {
            walk(&root.join(d), &mut files);
        }
        for f in ["README.md", "ARCHITECTURE.md", "justfile"] {
            files.push(root.join(f));
        }
        let at = |f: &Path| f.strip_prefix(&root).unwrap_or(f).display().to_string();
        for f in &files {
            if f.file_name().is_some_and(|n| n == "wrangler.toml") {
                left.push(format!("{} is a wrangler config", at(f)));
            }
        }
        let justfile = fs::read_to_string(root.join("justfile")).unwrap();
        for line in justfile.lines() {
            if line.starts_with("web-page-release") || line.starts_with("web-page-deploy") {
                left.push(format!("the justfile has the recipe {line}"));
            }
        }
        let names = fs::read_to_string(root.join("scripts/check/no-game-names.sh")).unwrap();
        if names.contains("deploy-old") {
            left.push("scripts/check/no-game-names.sh still leaves out deploy-old".into());
        }
        // This file names them to look for them; the changelog and
        // docs/evaluation.md (the older repo as it was on 2026-11-08) are
        // records of what was.
        let own = root.join("crates/gp-cli/src/bundle.rs");
        for f in &files {
            if f.ends_with("CHANGELOG.md") || f.ends_with("docs/evaluation.md") || *f == own {
                continue;
            }
            let Ok(text) = fs::read_to_string(f) else {
                continue;
            };
            for name in [
                "scripts/web/release-check.sh",
                "scripts/web/deploy.sh",
                "scripts/web/deploy-old.sh",
                "scripts/web/test-deploy.sh",
                "scripts/web/wrangler-upload.py",
                "scripts/web/deploy/",
                "scripts/web/deploy-old/",
                "just web-page-release",
                "just web-page-deploy",
            ] {
                if text.contains(name) {
                    left.push(format!("{} names {name}", at(f)));
                }
            }
        }
        assert!(
            left.is_empty(),
            "the page's deploy is still in the tools:\n{}",
            left.join("\n")
        );
    }
}
