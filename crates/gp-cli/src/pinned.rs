//! Pinned downloads that model evidence refs can point into with `in`
//! (docs/model.md, Evidence): sources that can't be in the repo, such as a
//! commercial game's disassembly. Each is downloaded and hash-checked by
//! its script into `$GP_HOME/work/`; `gp model check` reads a ref's file
//! from there once its hash matches the pin, and skips the ref with a
//! notice when the download isn't there.
//!
//! Ported from game-printer's pinned.rs. Its three pins were Super Mario
//! Bros., Mega Man 2 and Super Mario Bros. 2's disassemblies, cut with those
//! games, so no game here cites a pinned download yet and [`PINS`] is
//! empty: a ref with `in` is refused by name until a game adds its pin.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gp_model::validate::{RefRoot, pinned};
use sha2::{Digest, Sha256};

/// One pinned download.
pub struct Pin {
    /// The name refs use before `@`.
    pub name: &'static str,
    /// The full revision; a ref names it or a prefix of at least 7.
    pub revision: &'static str,
    /// Where the download goes, under `$GP_HOME/work/`.
    pub dir: &'static str,
    /// Each file and its SHA-256.
    pub files: &'static [(&'static str, &'static str)],
    /// The command that downloads it.
    pub fetch: &'static str,
}

/// The pins.
pub const PINS: &[Pin] = &[];

/// Resolves refs' `in` against the pins and `$GP_HOME/work`, remembering
/// which downloads were missing so the report can say so.
pub struct Resolver<'a> {
    pins: &'a [Pin],
    work: PathBuf,
    checked: BTreeMap<&'static str, Result<PathBuf, String>>,
    /// Downloads that weren't there: (name, the command that fetches it).
    pub missing: Vec<(&'static str, &'static str)>,
}

impl Resolver<'static> {
    pub fn new(gp_home: &Path) -> Self {
        Resolver::with_pins(gp_home, PINS)
    }
}

impl<'a> Resolver<'a> {
    pub fn with_pins(gp_home: &Path, pins: &'a [Pin]) -> Self {
        Resolver {
            pins,
            work: gp_home.join("work"),
            checked: BTreeMap::new(),
            missing: Vec::new(),
        }
    }

    pub fn root(&mut self, within: Option<&str>, file: &str, repo: &Path) -> RefRoot {
        let Some(within) = within else {
            return RefRoot::Dir(repo.to_path_buf());
        };
        let Some((name, rev)) = pinned(within) else {
            return RefRoot::Bad(format!("`{within}` isn't `<name>@<revision>`"));
        };
        let Some(pin) = self.pins.iter().find(|p| p.name == name) else {
            return RefRoot::Bad(format!("no pinned download is named `{name}`"));
        };
        if !pin.files.iter().any(|(f, _)| *f == file) {
            return RefRoot::Bad(format!(
                "{name} pins {}; `{file}` isn't one of its files",
                pin.files
                    .iter()
                    .map(|(f, _)| *f)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if rev.len() < 7 || !pin.revision.starts_with(rev) {
            return RefRoot::Bad(format!(
                "{name} is pinned at {}; `{rev}` isn't that revision (give at least 7 characters of it)",
                pin.revision
            ));
        }
        let work = &self.work;
        let missing = &mut self.missing;
        let state = self.checked.entry(pin.name).or_insert_with(|| {
            let dir = work.join(pin.dir);
            for (file, sha) in pin.files {
                let path = dir.join(file);
                match std::fs::read(&path) {
                    Err(_) => {
                        missing.push((pin.name, pin.fetch));
                        return Err(String::new());
                    }
                    Ok(bytes) if hex::encode(Sha256::digest(&bytes)) != *sha => {
                        return Err(format!(
                            "{} doesn't match the pinned hash; download it again ({} after removing it)",
                            path.display(),
                            pin.fetch
                        ));
                    }
                    Ok(_) => {}
                }
            }
            Ok(dir)
        });
        match state {
            Ok(dir) => RefRoot::Dir(dir.clone()),
            Err(m) if m.is_empty() => RefRoot::Skip,
            Err(m) => RefRoot::Bad(m.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up pin, in place of the old disassemblies' (no game's file:
    /// the hash is of the text written below).
    const TEST_PINS: &[Pin] = &[Pin {
        name: "testdis",
        revision: "7e8e9777a53a2f94fd968e79c891445a97a8ddde",
        dir: "testdis/download",
        files: &[(
            "TEST.ASM",
            // SHA-256 of "the disassembly\n".
            "dc2db861d4b3699593c81f8b4b492e7d89f83d50fdc901a39ad9c304b32b9e9e",
        )],
        fetch: "just test-labels",
    }];

    #[test]
    fn refs_resolve_against_the_pins() {
        let home = tempfile::tempdir().unwrap();
        let mut r = Resolver::with_pins(home.path(), TEST_PINS);
        let repo = Path::new("/repo");
        let f = "TEST.ASM";
        assert!(matches!(r.root(None, f, repo), RefRoot::Dir(d) if d == repo));
        assert!(matches!(
            r.root(Some("testdis@7e8e9777"), f, repo),
            RefRoot::Skip
        ));
        assert_eq!(r.missing, [("testdis", "just test-labels")]);
        assert!(matches!(
            r.root(Some("testdis@7e8e"), f, repo),
            RefRoot::Bad(_)
        ));
        assert!(matches!(
            r.root(Some("testdis@deadbeef0"), f, repo),
            RefRoot::Bad(_)
        ));
        assert!(matches!(
            r.root(Some("otherdis@7e8e9777"), f, repo),
            RefRoot::Bad(_)
        ));
        assert!(matches!(
            r.root(Some("testdis@7e8e9777"), "other.asm", repo),
            RefRoot::Bad(_)
        ));
        // A download whose hash isn't the pin's is refused, not read.
        let dir = home.path().join("work/testdis/download");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(f), "not the disassembly\n").unwrap();
        let mut r = Resolver::with_pins(home.path(), TEST_PINS);
        assert!(
            matches!(r.root(Some("testdis@7e8e9777"), f, repo), RefRoot::Bad(m) if m.contains("pinned hash"))
        );
        // The pinned file is read from the download.
        std::fs::write(dir.join(f), "the disassembly\n").unwrap();
        let mut r = Resolver::with_pins(home.path(), TEST_PINS);
        assert!(matches!(r.root(Some("testdis@7e8e9777"), f, repo), RefRoot::Dir(d) if d == dir));
    }

    /// No game in this repo cites a pinned download.
    #[test]
    fn no_pin_is_known_here() {
        let home = tempfile::tempdir().unwrap();
        let mut r = Resolver::new(home.path());
        assert!(matches!(
            r.root(Some("smbdis@7e8e9777"), "SMBDIS.ASM", Path::new("/repo")),
            RefRoot::Bad(m) if m.contains("no pinned download")
        ));
    }
}
