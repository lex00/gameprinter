//! Reading a romset: a MAME zip, every chip checked by size and SHA-1
//! against the set's list.
//!
//! The chip list (names, places, sizes and hashes) comes from MAME 0.276's
//! src/mame/midway/mcr.cpp, `ROM_START( rbtapper )`:
//! license:BSD-3-Clause, copyright-holders:Aaron Giles.
//!
//! ROM bytes stay in memory. Nothing here prints or writes them.

use std::io::Read;
use std::path::Path;

/// One chip of a set: its file name, the region and offset it loads at,
/// its size and its SHA-1.
#[derive(Debug, Clone, Copy)]
pub struct Chip {
    pub name: &'static str,
    pub region: Region,
    pub offset: usize,
    pub size: usize,
    pub sha1: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// The main board's Z80 program, 0x10000 bytes.
    MainCpu,
    /// The SSIO board's Z80 program, 0x10000 bytes.
    SsioCpu,
    /// Background tiles, 0x8000 bytes.
    Gfx1,
    /// Sprites, 0x20000 bytes.
    Gfx2,
}

impl Region {
    pub fn size(self) -> usize {
        match self {
            Self::MainCpu | Self::SsioCpu => 0x10000,
            Self::Gfx1 => 0x8000,
            Self::Gfx2 => 0x20000,
        }
    }
}

const fn chip(
    name: &'static str,
    region: Region,
    offset: usize,
    size: usize,
    sha1: &'static str,
) -> Chip {
    Chip {
        name,
        region,
        offset,
        size,
        sha1,
    }
}

/// MAME's `rbtapper`: "Tapper (Root Beer)", Bally Midway 1984.
pub const RBTAPPER: [Chip; 18] = [
    chip(
        "rbtpg0.bin",
        Region::MainCpu,
        0x0000,
        0x4000,
        "31b583c98e7c2cecab42223df8b5400004c9f3d3",
    ),
    chip(
        "rbtpg1.bin",
        Region::MainCpu,
        0x4000,
        0x4000,
        "7de10632b2538524fee418175e2cc41be9c8c0e8",
    ),
    chip(
        "rbtpg2.bin",
        Region::MainCpu,
        0x8000,
        0x4000,
        "b9878c8a61a98e787e547bb6ab81c809875891f3",
    ),
    chip(
        "rbtpg3.bin",
        Region::MainCpu,
        0xc000,
        0x2000,
        "ddb21e39ede2222cb2286ec9dba06341fe1c9db7",
    ),
    chip(
        "5788",
        Region::SsioCpu,
        0x0000,
        0x1000,
        "c2c94ab26ebce30ce4efc239e555c6368794d265",
    ),
    chip(
        "5787",
        Region::SsioCpu,
        0x1000,
        0x1000,
        "f5c8585d443bca67d4065314a06431d1f104c553",
    ),
    chip(
        "5786",
        Region::SsioCpu,
        0x2000,
        0x1000,
        "2a4dc0deca48f4d2ac9fe673ecb9548415c996a9",
    ),
    chip(
        "5785",
        Region::SsioCpu,
        0x3000,
        0x1000,
        "a41323149c50adcae7675efcef69fd7d8365e527",
    ),
    chip(
        "rbtbg1.bin",
        Region::Gfx1,
        0x0000,
        0x4000,
        "9e96a3ff0b66a4b1d07fe86059f7dfab0d53231c",
    ),
    chip(
        "rbtbg0.bin",
        Region::Gfx1,
        0x4000,
        0x4000,
        "a4b24fffbbe299e0c2058abe372f00954e5edf98",
    ),
    chip(
        "rbtfg1.bin",
        Region::Gfx2,
        0x00000,
        0x4000,
        "532239605b74b137cd0f6035a6bc7ac43f559d82",
    ),
    chip(
        "rbtfg0.bin",
        Region::Gfx2,
        0x04000,
        0x4000,
        "3a8e3e927b0fd2d49222e417e4c1f28b5a45c6ca",
    ),
    chip(
        "rbtfg3.bin",
        Region::Gfx2,
        0x08000,
        0x4000,
        "3811f499aa076fb81af42f7cb522879b0af50e37",
    ),
    chip(
        "rbtfg2.bin",
        Region::Gfx2,
        0x0c000,
        0x4000,
        "d02a306f8b32b3a097fc51e75e106a40fe942d35",
    ),
    chip(
        "rbtfg5.bin",
        Region::Gfx2,
        0x10000,
        0x4000,
        "523efaba9a3cbe871868ee09caea062382f31fad",
    ),
    chip(
        "rbtfg4.bin",
        Region::Gfx2,
        0x14000,
        0x4000,
        "e2a427324fc1d4422e50a1ab79822aff0290e109",
    ),
    chip(
        "rbtfg7.bin",
        Region::Gfx2,
        0x18000,
        0x4000,
        "dab52db0bca13b9f3470060bc3babe4a6c2cdcc8",
    ),
    chip(
        "rbtfg6.bin",
        Region::Gfx2,
        0x1c000,
        0x4000,
        "400d390729601610b152e21da55399399830221f",
    ),
];

/// Files MAME knows that aren't in the set's own list: the SSIO board's
/// 82S123 PROM (midway.cpp, `ROM_START( midway_ssio )`), which only sets
/// the sound volumes.
const KNOWN_EXTRAS: [(&str, &str); 1] = [(
    "the SSIO board's 82S123 PROM (82s123.12d), which only sets sound volumes",
    "9ac9b01d24affc0ee9227a4364c4fd8f8290343a",
)];

/// A loaded set: the four regions, and what else the zip held.
#[derive(Clone)]
pub struct Romset {
    pub name: &'static str,
    pub maincpu: Vec<u8>,
    pub ssiocpu: Vec<u8>,
    pub gfx1: Vec<u8>,
    pub gfx2: Vec<u8>,
    /// Files in the zip that aren't chips of the set, with what they are
    /// when MAME knows them. Reported, then ignored.
    pub extras: Vec<String>,
}

impl std::fmt::Debug for Romset {
    // Never the bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Romset")
            .field("name", &self.name)
            .field("extras", &self.extras)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RomError {
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("{0}: not a zip file I can read ({1})")]
    Zip(String, String),
    #[error("{set}: chip {chip} is missing")]
    Missing {
        set: &'static str,
        chip: &'static str,
    },
    #[error("{set}: chip {chip} is {found} bytes, the set's list says {want}")]
    Size {
        set: &'static str,
        chip: &'static str,
        found: usize,
        want: usize,
    },
    #[error("{set}: chip {chip} has SHA-1 {found}, the set's list says {want}")]
    Hash {
        set: &'static str,
        chip: &'static str,
        found: String,
        want: &'static str,
    },
}

impl Romset {
    /// Reads rbtapper from a MAME zip.
    pub fn load_zip(path: &Path) -> Result<Self, RomError> {
        let shown = path.display().to_string();
        let data = std::fs::read(path).map_err(|e| RomError::Io(shown.clone(), e))?;
        let files = read_zip(&data).map_err(|e| RomError::Zip(shown, e))?;
        Self::from_files("rbtapper", &RBTAPPER, files)
    }

    /// Builds a set from (name, bytes) pairs. A chip is found by its name,
    /// or else by its SHA-1, as MAME does.
    pub fn from_files(
        name: &'static str,
        chips: &[Chip],
        files: Vec<(String, Vec<u8>)>,
    ) -> Result<Self, RomError> {
        let hashed: Vec<(String, String, Vec<u8>)> = files
            .into_iter()
            .map(|(n, d)| (n, hex(&crate::sha1::sha1(&d)), d))
            .collect();
        let mut used = vec![false; hashed.len()];
        let mut regions = [
            vec![0u8; Region::MainCpu.size()],
            vec![0u8; Region::SsioCpu.size()],
            vec![0u8; Region::Gfx1.size()],
            vec![0u8; Region::Gfx2.size()],
        ];
        for c in chips {
            let found = hashed
                .iter()
                .position(|(n, _, _)| n == c.name)
                .or_else(|| hashed.iter().position(|(_, h, _)| h == c.sha1));
            let Some(i) = found else {
                return Err(RomError::Missing {
                    set: name,
                    chip: c.name,
                });
            };
            let (_, hash, bytes) = &hashed[i];
            if bytes.len() != c.size {
                return Err(RomError::Size {
                    set: name,
                    chip: c.name,
                    found: bytes.len(),
                    want: c.size,
                });
            }
            if hash != c.sha1 {
                return Err(RomError::Hash {
                    set: name,
                    chip: c.name,
                    found: hash.clone(),
                    want: c.sha1,
                });
            }
            used[i] = true;
            let r = match c.region {
                Region::MainCpu => 0,
                Region::SsioCpu => 1,
                Region::Gfx1 => 2,
                Region::Gfx2 => 3,
            };
            regions[r][c.offset..c.offset + c.size].copy_from_slice(bytes);
        }
        let extras = hashed
            .iter()
            .zip(&used)
            .filter(|(_, u)| !**u)
            .map(
                |((n, h, d), _)| match KNOWN_EXTRAS.iter().find(|(_, s)| s == h) {
                    Some((what, _)) => format!("{n} ({} bytes): {what}; ignored", d.len()),
                    None => format!(
                        "{n} ({} bytes, SHA-1 {h}): not in the set's list; ignored",
                        d.len()
                    ),
                },
            )
            .collect();
        let [maincpu, ssiocpu, gfx1, gfx2] = regions;
        Ok(Self {
            name,
            maincpu,
            ssiocpu,
            gfx1,
            gfx2,
            extras,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn le16(d: &[u8], at: usize) -> Result<usize, String> {
    d.get(at..at + 2)
        .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
        .ok_or_else(|| "truncated".to_string())
}

fn le32(d: &[u8], at: usize) -> Result<usize, String> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or_else(|| "truncated".to_string())
}

/// The files of a zip: (name, contents). Stored and deflated entries only,
/// which is what MAME romsets use.
pub fn read_zip(d: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    // The end of central directory record, searched for from the end.
    let eocd = (0..=d.len().saturating_sub(22))
        .rev()
        .find(|&i| d[i..].starts_with(b"PK\x05\x06"))
        .ok_or("no end of central directory")?;
    let count = le16(d, eocd + 10)?;
    let mut at = le32(d, eocd + 16)?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if !d.get(at..).is_some_and(|s| s.starts_with(b"PK\x01\x02")) {
            return Err("bad central directory entry".into());
        }
        let method = le16(d, at + 10)?;
        let csize = le32(d, at + 20)?;
        let usize_ = le32(d, at + 24)?;
        let nlen = le16(d, at + 28)?;
        let elen = le16(d, at + 30)?;
        let clen = le16(d, at + 32)?;
        let local = le32(d, at + 42)?;
        let name = String::from_utf8_lossy(d.get(at + 46..at + 46 + nlen).ok_or("truncated")?)
            .into_owned();
        at += 46 + nlen + elen + clen;
        if name.ends_with('/') {
            continue;
        }
        if !d.get(local..).is_some_and(|s| s.starts_with(b"PK\x03\x04")) {
            return Err(format!("{name}: bad local header"));
        }
        let start = local + 30 + le16(d, local + 26)? + le16(d, local + 28)?;
        let raw = d.get(start..start + csize).ok_or("truncated entry")?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_);
                flate2::read::DeflateDecoder::new(raw)
                    .read_to_end(&mut v)
                    .map_err(|e| format!("{name}: {e}"))?;
                v
            }
            m => return Err(format!("{name}: compression method {m}")),
        };
        if bytes.len() != usize_ {
            return Err(format!(
                "{name}: {} bytes, the directory says {usize_}",
                bytes.len()
            ));
        }
        out.push((name, bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(chips: &[Chip]) -> Vec<(String, Vec<u8>)> {
        chips
            .iter()
            .map(|c| (c.name.to_string(), vec![0u8; c.size]))
            .collect()
    }

    #[test]
    fn a_wrong_chip_is_named() {
        let err = Romset::from_files("rbtapper", &RBTAPPER, fake(&RBTAPPER)).unwrap_err();
        assert!(
            matches!(
                err,
                RomError::Hash {
                    chip: "rbtpg0.bin",
                    ..
                }
            ),
            "{err}"
        );
        let mut files = fake(&RBTAPPER);
        files.remove(0);
        let err = Romset::from_files("rbtapper", &RBTAPPER, files).unwrap_err();
        assert!(
            matches!(
                err,
                RomError::Missing {
                    chip: "rbtpg0.bin",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn checks_against_a_made_up_list_and_reports_extras() {
        // A two-chip "set" whose hashes are those of the made-up contents.
        let a = vec![1u8; 0x1000];
        let b = vec![2u8; 0x4000];
        let ha: &'static str = Box::leak(hex(&crate::sha1::sha1(&a)).into_boxed_str());
        let hb: &'static str = Box::leak(hex(&crate::sha1::sha1(&b)).into_boxed_str());
        let chips = [
            chip("a", Region::SsioCpu, 0x1000, 0x1000, ha),
            chip("b", Region::Gfx1, 0x4000, 0x4000, hb),
        ];
        let files = vec![
            ("renamed".to_string(), b.clone()),
            ("a".to_string(), a),
            ("junk".to_string(), vec![9u8; 7]),
        ];
        let set = Romset::from_files("test", &chips, files).unwrap();
        assert_eq!(set.ssiocpu[0x1000], 1);
        assert_eq!(set.ssiocpu[0x0fff], 0);
        assert_eq!(set.gfx1[0x4000], 2);
        assert_eq!(set.extras.len(), 1);
        assert!(set.extras[0].starts_with("junk (7 bytes"));
    }

    #[test]
    fn reads_a_stored_zip() {
        // A zip with one stored file, built by hand.
        let body = b"hello";
        let mut z = Vec::new();
        z.extend_from_slice(b"PK\x03\x04");
        z.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&0u32.to_le_bytes()); // crc, unchecked here
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&1u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.push(b'x');
        z.extend_from_slice(body);
        let cd = z.len();
        z.extend_from_slice(b"PK\x01\x02");
        z.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&1u16.to_le_bytes());
        z.extend_from_slice(&[0; 12]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.push(b'x');
        let cdlen = z.len() - cd;
        z.extend_from_slice(b"PK\x05\x06");
        z.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        z.extend_from_slice(&(cdlen as u32).to_le_bytes());
        z.extend_from_slice(&(cd as u32).to_le_bytes());
        z.extend_from_slice(&[0, 0]);
        let files = read_zip(&z).unwrap();
        assert_eq!(files, vec![("x".to_string(), body.to_vec())]);
    }
}
