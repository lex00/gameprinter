//! Identifying source games. Game Printer reads ROMs from `$GP_ROM_DIR` to
//! identify them and never copies them anywhere.
//!
//! A ROM's identity is the SHA-256 of its PRG and CHR data with the header
//! (and any trainer) removed, which is how No-Intro names dumps. The header is
//! recorded separately because it carries the mapper and mirroring.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const INES_MAGIC: &[u8; 4] = b"NES\x1a";
pub(crate) const HEADER_LEN: usize = 16;
pub(crate) const TRAINER_LEN: usize = 512;

/// Which header rules a file's header is read by. The detection follows the
/// NESdev wiki's "iNES" page (https://www.nesdev.org/wiki/INES), "Recommended
/// detection procedure": NES 2.0 if byte 7 AND $0C is $08 and the sizes it
/// gives fit in the file; plain iNES if byte 7 AND $0C is $00 and bytes 12
/// to 15 are all zero; otherwise "iNES 0.7 or archaic iNES", which Game
/// Printer always reads as archaic iNES: only bytes 0 to 6 are trusted and
/// bytes 7 to 15 are treated as zero. ROM tools of the 1990s wrote text such
/// as "DiskDude!" there, which read literally adds 64 to the mapper number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeaderFormat {
    Ines,
    Nes20,
    ArchaicInes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mirroring {
    Horizontal,
    Vertical,
    FourScreen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NesHeader {
    pub format: HeaderFormat,
    pub mapper: u16,
    pub submapper: u8,
    pub prg_rom_bytes: u64,
    pub chr_rom_bytes: u64,
    pub mirroring: Mirroring,
    pub battery: bool,
    /// Byte 6 bit 2, the trainer flag, as set in the file. Whether a trainer
    /// is actually there is [`NesImage::trainer_bytes`].
    pub trainer: bool,
    /// NES 2.0 only: PRG RAM and CHR RAM sizes in bytes, volatile plus
    /// battery-backed. `None` for iNES headers, which don't give them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prg_ram_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chr_ram_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NesImage {
    pub header: NesHeader,
    /// SHA-256 of PRG and CHR without header or trainer: the ROM's identity.
    pub sha256: String,
    /// SHA-256 of the whole file as found on disk.
    pub sha256_file: String,
    pub file_bytes: u64,
    /// 512 when the trainer flag is set and the file has room for one after
    /// the header, else 0. The trainer is not hashed.
    #[serde(default)]
    pub trainer_bytes: u64,
    /// Bytes after PRG and CHR (an overdump, or PlayChoice data), not hashed.
    pub trailing_bytes: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SourceError {
    #[error("not an iNES or NES 2.0 image (missing `NES<EOF>` header)")]
    NotNes,
    #[error("image is truncated: header declares {declared} bytes of data, file has {actual}")]
    Truncated { declared: u64, actual: u64 },
}

/// Every NES image under `dir`, searched recursively, by headerless SHA-256.
/// Files that aren't NES images are skipped, and when two files share a hash
/// the first in path order wins. A missing `dir` is an error, not an empty
/// index, so a mistyped ROM folder can't make every check skip. Used to find a recording's or a baseline's
/// ROM, natively and under wasmtime (through a preopened directory).
pub fn index_nes_roms(
    dir: &std::path::Path,
) -> std::io::Result<std::collections::HashMap<String, std::path::PathBuf>> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.is_file() {
                out.push(path);
            }
        }
        Ok(())
    }
    if !dir.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "the ROM folder doesn't exist",
        ));
    }
    let mut files = Vec::new();
    walk(dir, &mut files)?;
    files.sort();
    let mut index = std::collections::HashMap::new();
    for path in files {
        if !file_looks_like_nes(&path) {
            continue;
        }
        if let Ok(image) = identify_nes(&std::fs::read(&path)?) {
            index.entry(image.sha256).or_insert(path);
        }
    }
    Ok(index)
}

pub fn looks_like_nes(bytes: &[u8]) -> bool {
    bytes.starts_with(INES_MAGIC)
}

/// Whether the file at `path` starts as an iNES image does, reading only
/// its first four bytes; a file that can't be read or is shorter doesn't.
pub fn file_looks_like_nes(path: &std::path::Path) -> bool {
    let mut head = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .is_ok()
        && looks_like_nes(&head)
}

pub fn identify_nes(bytes: &[u8]) -> Result<NesImage, SourceError> {
    if bytes.len() < HEADER_LEN || !looks_like_nes(bytes) {
        return Err(SourceError::NotNes);
    }
    let h = &bytes[..HEADER_LEN];
    let flags6 = h[6];
    let flags7 = h[7];
    let trainer = flags6 & 0x04 != 0;
    let nes20_sizes = (
        nes20_size(h[4], h[9] & 0x0F, 16 * 1024),
        nes20_size(h[5], h[9] >> 4, 8 * 1024),
    );
    // The fit is checked without a trainer: a trainer flag with no room for
    // one is reported and ignored below, and mustn't demote the header.
    let nes20 = flags7 & 0x0C == 0x08
        && (HEADER_LEN as u64)
            .saturating_add(nes20_sizes.0)
            .saturating_add(nes20_sizes.1)
            <= bytes.len() as u64;
    let format = if nes20 {
        HeaderFormat::Nes20
    } else if flags7 & 0x0C == 0 && h[12..16].iter().all(|&b| b == 0) {
        HeaderFormat::Ines
    } else {
        HeaderFormat::ArchaicInes
    };

    let mut mapper = u16::from(flags6 >> 4);
    let mut submapper = 0;
    let (mut prg_ram_bytes, mut chr_ram_bytes) = (None, None);
    let (prg_rom_bytes, chr_rom_bytes) = match format {
        HeaderFormat::Nes20 => {
            mapper |= u16::from(flags7 & 0xF0) | u16::from(h[8] & 0x0F) << 8;
            submapper = h[8] >> 4;
            prg_ram_bytes = Some(nes20_ram(h[10] & 0x0F) + nes20_ram(h[10] >> 4));
            chr_ram_bytes = Some(nes20_ram(h[11] & 0x0F) + nes20_ram(h[11] >> 4));
            nes20_sizes
        }
        HeaderFormat::Ines => {
            mapper |= u16::from(flags7 & 0xF0);
            (u64::from(h[4]) * 16 * 1024, u64::from(h[5]) * 8 * 1024)
        }
        HeaderFormat::ArchaicInes => (u64::from(h[4]) * 16 * 1024, u64::from(h[5]) * 8 * 1024),
    };

    let mirroring = if flags6 & 0x08 != 0 {
        Mirroring::FourScreen
    } else if flags6 & 0x01 != 0 {
        Mirroring::Vertical
    } else {
        Mirroring::Horizontal
    };
    let header = NesHeader {
        format,
        mapper,
        submapper,
        prg_rom_bytes,
        chr_rom_bytes,
        mirroring,
        battery: flags6 & 0x02 != 0,
        trainer,
        prg_ram_bytes,
        chr_ram_bytes,
    };

    let declared = prg_rom_bytes.saturating_add(chr_rom_bytes);
    // A trainer only when the flag is set and the file has room for one; a
    // flag with no room for the 512 bytes is reported (dump.rs) and ignored.
    let trainer_bytes = if trainer
        && (bytes.len() as u64).saturating_sub(HEADER_LEN as u64) >= declared + TRAINER_LEN as u64
    {
        TRAINER_LEN as u64
    } else {
        0
    };
    let start = HEADER_LEN + trainer_bytes as usize;
    let available = bytes.len().saturating_sub(start) as u64;
    if available < declared {
        return Err(SourceError::Truncated {
            declared,
            actual: available,
        });
    }
    let end = start + declared as usize;
    Ok(NesImage {
        header,
        sha256: hex::encode(Sha256::digest(&bytes[start..end])),
        sha256_file: hex::encode(Sha256::digest(bytes)),
        file_bytes: bytes.len() as u64,
        trainer_bytes,
        trailing_bytes: (bytes.len() - end) as u64,
    })
}

/// A NES 2.0 RAM size nibble: 0 for none, else `64 << n` bytes.
fn nes20_ram(nibble: u8) -> u64 {
    if nibble == 0 { 0 } else { 64u64 << nibble }
}

/// NES 2.0 sizes: a plain count of `unit`s, or when the high nibble is 0xF an
/// exponent-multiplier form, `2^E * (MM * 2 + 1)` bytes.
pub(crate) fn nes20_size(lsb: u8, msb_nibble: u8, unit: u64) -> u64 {
    if msb_nibble == 0x0F {
        let exponent = u32::from(lsb >> 2);
        let multiplier = u64::from(lsb & 0x03) * 2 + 1;
        // Saturates, so a nonsense size fails the fit and truncation checks.
        1u64.checked_shl(exponent)
            .and_then(|v| v.checked_mul(multiplier))
            .unwrap_or(u64::MAX)
    } else {
        (u64::from(msb_nibble) << 8 | u64::from(lsb)) * unit
    }
}

/// An NES image split into its PRG and CHR data, header and trainer removed.
pub fn nes_banks(bytes: &[u8]) -> Result<(NesImage, &[u8], &[u8]), SourceError> {
    let img = identify_nes(bytes)?;
    let start = HEADER_LEN + img.trainer_bytes as usize;
    let prg_end = start + img.header.prg_rom_bytes as usize;
    let chr_end = prg_end + img.header.chr_rom_bytes as usize;
    Ok((img, &bytes[start..prg_end], &bytes[prg_end..chr_end]))
}

/// A run of bytes that differs between two buffers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ByteRange {
    pub offset: u64,
    pub len: u64,
    /// How many bytes in the run actually differ (runs absorb short equal gaps).
    pub differing: u64,
    /// Up to [`HEX_PREVIEW`] bytes of each side, as hex.
    pub a: String,
    pub b: String,
}

pub const HEX_PREVIEW: usize = 32;

/// Differing ranges between `a` and `b`. Runs separated by `gap` or fewer
/// equal bytes are merged. If the lengths differ, the extra tail is one range.
pub fn diff_ranges(a: &[u8], b: &[u8], gap: usize) -> Vec<ByteRange> {
    let common = a.len().min(b.len());
    let mut out: Vec<(usize, usize, u64)> = Vec::new();
    for i in (0..common).filter(|&i| a[i] != b[i]) {
        match out.last_mut() {
            Some((_, end, n)) if i <= *end + gap => {
                *end = i + 1;
                *n += 1;
            }
            _ => out.push((i, i + 1, 1)),
        }
    }
    let longer = a.len().max(b.len());
    if longer > common {
        out.push((common, longer, (longer - common) as u64));
    }
    let preview = |buf: &[u8], s: usize, e: usize| {
        let e = e.min(buf.len()).min(s + HEX_PREVIEW);
        hex::encode(buf.get(s..e).unwrap_or_default())
    };
    out.into_iter()
        .map(|(s, e, n)| ByteRange {
            offset: s as u64,
            len: (e - s) as u64,
            differing: n,
            a: preview(a, s, e),
            b: preview(b, s, e),
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a synthetic image. Tests never use real game data.
    fn image(header: [u8; 16], payload_len: usize, extra: usize) -> Vec<u8> {
        let mut v = header.to_vec();
        if header[6] & 0x04 != 0 {
            v.extend(std::iter::repeat_n(0xEE, TRAINER_LEN));
        }
        v.extend((0..payload_len).map(|i| (i % 251) as u8));
        v.extend(std::iter::repeat_n(0xAA, extra));
        v
    }

    /// An iNES header for these bank counts and flags.
    pub(crate) fn ines(prg16k: u8, chr8k: u8, flags6: u8, flags7: u8) -> [u8; 16] {
        let mut h = [0u8; 16];
        h[..4].copy_from_slice(INES_MAGIC);
        h[4] = prg16k;
        h[5] = chr8k;
        h[6] = flags6;
        h[7] = flags7;
        h
    }

    #[test]
    fn nrom_vertical() {
        let bytes = image(ines(2, 1, 0x01, 0x00), 32 * 1024 + 8 * 1024, 0);
        let img = identify_nes(&bytes).unwrap();
        assert_eq!(img.header.mapper, 0);
        assert_eq!(img.header.mirroring, Mirroring::Vertical);
        assert_eq!(img.header.prg_rom_bytes, 32 * 1024);
        assert_eq!(img.header.chr_rom_bytes, 8 * 1024);
        assert_eq!(img.header.format, HeaderFormat::Ines);
        assert_eq!(img.sha256, hex::encode(Sha256::digest(&bytes[16..])));
    }

    #[test]
    fn identity_ignores_header_trainer_and_trailing_data() {
        let payload = 16 * 1024;
        let plain = image(ines(1, 0, 0x00, 0x00), payload, 0);
        let with_trainer = image(ines(1, 0, 0x04, 0x00), payload, 0);
        let mut renamed_header = ines(1, 0, 0x00, 0x00);
        renamed_header[12] = 0x42; // junk some dumping tools leave in padding
        let dirty = image(renamed_header, payload, 100);
        let a = identify_nes(&plain).unwrap();
        let b = identify_nes(&with_trainer).unwrap();
        let c = identify_nes(&dirty).unwrap();
        assert_eq!(a.sha256, b.sha256);
        assert_eq!(a.sha256, c.sha256);
        assert_ne!(a.sha256_file, c.sha256_file);
        assert_eq!(c.trailing_bytes, 100);
    }

    #[test]
    fn mapper_number_from_both_nibbles() {
        // MMC3 (mapper 4) and MMC1 (mapper 1)
        let mmc3 = image(ines(1, 1, 0x40, 0x00), 24 * 1024, 0);
        assert_eq!(identify_nes(&mmc3).unwrap().header.mapper, 4);
        let high = image(ines(1, 0, 0x10, 0x40), 16 * 1024, 0);
        assert_eq!(identify_nes(&high).unwrap().header.mapper, 0x41);
    }

    #[test]
    fn nes20_header() {
        let mut h = ines(2, 0, 0x12, 0x08);
        h[8] = 0x31; // submapper 3, mapper bits 8-11 = 1
        let bytes = image(h, 32 * 1024, 0);
        let img = identify_nes(&bytes).unwrap();
        assert_eq!(img.header.format, HeaderFormat::Nes20);
        assert_eq!(img.header.mapper, 0x101);
        assert_eq!(img.header.submapper, 3);
        assert!(img.header.battery);
    }

    #[test]
    fn banks_split_prg_and_chr() {
        let bytes = image(ines(1, 1, 0x04, 0x00), 24 * 1024, 7);
        let (img, prg, chr) = nes_banks(&bytes).unwrap();
        assert_eq!(prg.len(), 16 * 1024);
        assert_eq!(chr.len(), 8 * 1024);
        assert_eq!(prg[0], 0);
        assert_eq!(img.trailing_bytes, 7);
    }

    #[test]
    fn diff_merges_close_runs_and_reports_tails() {
        let a = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let mut b = a;
        b[2] = 0xFF;
        b[4] = 0xFF;
        b[9] = 0xFF;
        let d = diff_ranges(&a, &b, 2);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].offset, d[0].len, d[0].differing), (2, 3, 2));
        assert_eq!(d[0].a, "020304");
        assert_eq!(d[0].b, "ff03ff");
        assert_eq!((d[1].offset, d[1].len), (9, 1));

        let tail = diff_ranges(&a[..8], &a, 0);
        assert_eq!((tail[0].offset, tail[0].len), (8, 2));
        assert!(diff_ranges(&a, &a, 4).is_empty());
    }

    #[test]
    fn nes20_with_a_spurious_trainer_flag_stays_nes20() {
        // Mapper $105, submapper 3, and a trainer flag with no trainer.
        let mut h = ines(2, 1, 0x50, 0x08);
        h[8] = 0x31;
        let mut bytes = image(h, 40 * 1024, 0);
        bytes[6] |= 0x04;
        let img = identify_nes(&bytes).unwrap();
        assert_eq!(img.header.format, HeaderFormat::Nes20);
        assert_eq!((img.header.mapper, img.header.submapper), (0x105, 3));
        assert_eq!(img.trainer_bytes, 0);
    }

    #[test]
    fn nes20_exponent_size() {
        assert_eq!(nes20_size(0b0000_1001, 0x0F, 16 * 1024), (1 << 2) * 3);
        assert_eq!(nes20_size(2, 0x01, 16 * 1024), 258 * 16 * 1024);
    }

    #[test]
    fn rejects_non_nes_and_truncated() {
        assert_eq!(
            identify_nes(b"PK\x03\x04 not a rom"),
            Err(SourceError::NotNes)
        );
        let short = image(ines(2, 0, 0, 0), 1000, 0);
        assert!(matches!(
            identify_nes(&short),
            Err(SourceError::Truncated { .. })
        ));
    }
}
