//! The person's ROMs, found by headerless hash. A folder named `sealed` is
//! never opened.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_evidence::source::{identify_nes, looks_like_nes};

/// A ROM: its file bytes and headerless SHA-256.
pub struct Rom {
    pub bytes: Vec<u8>,
    pub sha256: String,
}

impl Rom {
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let img = identify_nes(&bytes).with_context(|| path.display().to_string())?;
        Ok(Self {
            bytes,
            sha256: img.sha256,
        })
    }

    /// The data after the iNES header.
    pub fn data(&self) -> &[u8] {
        self.bytes.get(16..).unwrap_or(&[])
    }

    /// A short name for reports: the id and the start of the hash, no path.
    pub fn label(&self, id: &str) -> String {
        format!("{id} ({})", &self.sha256[..12])
    }
}

/// A ROM's headerless SHA-256.
pub fn sha256_of(bytes: &[u8]) -> Result<String> {
    Ok(identify_nes(bytes)?.sha256)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let e = e?;
        let path = e.path();
        if e.file_type()?.is_dir() {
            if !e.file_name().eq_ignore_ascii_case("sealed") {
                walk(&path, out)?;
            }
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn is_nes(path: &Path) -> bool {
    let mut head = [0u8; 4];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && looks_like_nes(&head)
}

/// The ROMs with the wanted headerless hashes, by hash: each file given
/// (which must be one of them), then the first file by path in `dir`.
pub fn find(wanted: &[String], given: &[PathBuf], dir: &Path) -> Result<BTreeMap<String, Rom>> {
    let mut found = BTreeMap::new();
    for p in given {
        let rom = Rom::read(p)?;
        if !wanted.contains(&rom.sha256) {
            bail!(
                "{} (sha256 {}) isn't a ROM this package names",
                p.display(),
                rom.sha256
            );
        }
        found.insert(rom.sha256.clone(), rom);
    }
    if !dir.is_dir() || found.len() == wanted.len() {
        return Ok(found);
    }
    let mut files = Vec::new();
    walk(dir, &mut files)?;
    files.sort();
    for path in files {
        if !is_nes(&path) {
            continue;
        }
        let Ok(rom) = Rom::read(&path) else { continue };
        if wanted.contains(&rom.sha256) && !found.contains_key(&rom.sha256) {
            found.insert(rom.sha256.clone(), rom);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_rom(path: &Path, fill: u8) -> String {
        let mut rom = b"NES\x1a\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        rom.extend(vec![fill; 16384 + 8192]);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &rom).unwrap();
        crate::template::sha256_hex(&rom[16..])
    }

    #[test]
    fn roms_are_found_by_hash_and_a_sealed_folder_is_never_opened() {
        let d = tempfile::tempdir().unwrap();
        let a = write_rom(&d.path().join("nes/a.nes"), 1);
        let b = write_rom(&d.path().join("sealed/b.nes"), 2);
        let found = find(&[a.clone(), b.clone()], &[], d.path()).unwrap();
        assert!(found.contains_key(&a));
        assert!(!found.contains_key(&b));
    }

    #[test]
    fn a_rom_the_package_does_not_name_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.nes");
        write_rom(&p, 1);
        assert!(find(&["00".repeat(32)], &[p], d.path()).is_err());
    }

    #[test]
    fn no_rom_folder_finds_nothing() {
        let d = tempfile::tempdir().unwrap();
        let found = find(&["00".repeat(32)], &[], &d.path().join("none")).unwrap();
        assert!(found.is_empty());
    }
}
