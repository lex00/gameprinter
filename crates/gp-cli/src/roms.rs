//! The person's ROMs, found by headerless hash. A folder named `sealed` is
//! never opened.
//!
//! A ROM is an NES image (`.nes`, known by the SHA-256 of its data without
//! the header) or an arcade romset (a MAME zip, known by its set's
//! identity: gp_arcade::romset's module docs say how it is made).

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_evidence::source::{identify_nes, looks_like_nes};

/// The machine a ROM is for: what a package entry's and a roms.toml
/// source's `platform` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Nes,
    Arcade,
}

impl Platform {
    /// The machine a ROM file is for, by its first bytes: a zip is an
    /// arcade romset, anything else is taken as an NES image.
    pub fn of_file(bytes: &[u8]) -> Self {
        if gp_arcade::romset::looks_like_zip(bytes) {
            Self::Arcade
        } else {
            Self::Nes
        }
    }

    /// From its name in package.toml or roms.toml; none is the NES.
    pub fn named(name: Option<&str>) -> Result<Self> {
        match name {
            None | Some("nes") => Ok(Self::Nes),
            Some("arcade") => Ok(Self::Arcade),
            Some(p) => bail!("no platform {p}: nes or arcade"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Nes => "nes",
            Self::Arcade => "arcade",
        }
    }
}

/// A ROM: its file bytes, its identity and the machine it is for.
pub struct Rom {
    pub bytes: Vec<u8>,
    /// An NES image's headerless SHA-256, or a romset's identity.
    pub sha256: String,
    pub platform: Platform,
    /// A romset's MAME set name (rbtapper); none for the NES.
    pub system: Option<String>,
    /// A romset's chips, unzipped, one after another in the set's order:
    /// what a copy of its bytes would hold. Empty for the NES.
    chips: Vec<u8>,
}

impl Rom {
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::of_bytes(bytes).with_context(|| path.display().to_string())
    }

    pub fn of_bytes(bytes: Vec<u8>) -> Result<Self> {
        match Platform::of_file(&bytes) {
            Platform::Arcade => {
                let set = gp_arcade::Romset::from_zip(&bytes)?;
                Ok(Self {
                    sha256: set.identity.clone(),
                    system: Some(set.name.to_owned()),
                    chips: chips_of(&bytes, set.name)?,
                    bytes,
                    platform: Platform::Arcade,
                })
            }
            Platform::Nes => {
                let img = identify_nes(&bytes)?;
                Ok(Self {
                    bytes,
                    sha256: img.sha256,
                    platform: Platform::Nes,
                    system: None,
                    chips: Vec::new(),
                })
            }
        }
    }

    /// The data after the iNES header; a romset's chips unzipped.
    pub fn data(&self) -> &[u8] {
        match self.platform {
            Platform::Nes => self.bytes.get(16..).unwrap_or(&[]),
            Platform::Arcade => &self.chips,
        }
    }

    /// A short name for reports: the id and the start of the hash, no path.
    pub fn label(&self, id: &str) -> String {
        format!("{id} ({})", &self.sha256[..12])
    }
}

/// A romset zip's chips of the set `name`, in its list's order.
fn chips_of(zip: &[u8], name: &str) -> Result<Vec<u8>> {
    let set = gp_arcade::romset::set(name).with_context(|| format!("no arcade set {name}"))?;
    let files: Vec<(String, Vec<u8>)> =
        gp_arcade::romset::read_zip(zip).map_err(anyhow::Error::msg)?;
    let hashed: Vec<(String, &[u8])> = files
        .iter()
        .map(|(_, d)| (hex::encode(gp_arcade::sha1::sha1(d)), d.as_slice()))
        .collect();
    let mut out = Vec::new();
    for c in set.chips {
        let (_, d) = hashed
            .iter()
            .find(|(h, _)| h == c.sha1)
            .with_context(|| format!("{name}: no chip {}", c.name))?;
        out.extend_from_slice(d);
    }
    Ok(out)
}

/// A ROM's headerless SHA-256, or a romset's identity.
pub fn sha256_of(bytes: &[u8]) -> Result<String> {
    Ok(Rom::of_bytes(bytes.to_vec())?.sha256)
}

/// Whether a file starts as an NES image or a zip does.
pub fn is_rom_file(path: &Path) -> bool {
    let mut head = [0u8; 4];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && (looks_like_nes(&head) || gp_arcade::romset::looks_like_zip(&head))
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
    crate::files::walk_unsealed(dir, &mut files)?;
    files.sort();
    for path in files {
        if !is_rom_file(&path) {
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
