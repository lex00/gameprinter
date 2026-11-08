//! Junk around a dump: what ROM tools and dumpers leave around otherwise
//! exact game data, how it's found, and how it's removed. docs/sources.md,
//! "Junk around a dump", describes each case.
//!
//! Only bytes outside the game data are ever changed: header bytes the
//! header rules don't trust, a trainer, and bytes after the stated PRG and
//! CHR sizes. PRG and CHR are never touched. A dump whose game data was
//! modified (a hack, a translation, an intro patched into PRG) can't be
//! cleaned; it's reported as matching no known release.

use std::borrow::Cow;

use serde::Serialize;

use crate::registry::{BoardFacts, BoardMirroring};
use crate::source::{
    HEADER_LEN, HeaderFormat, INES_MAGIC, Mirroring, NesHeader, NesImage, SourceError, identify_nes,
};

/// Text a ROM tool is known to write into header bytes 7 to 15. FCEUX
/// (src/ines.h, `iNES_HEADER::cleanup`) recognises these three and clears
/// the bytes they occupy: "DiskDude" or "demiforce" at byte 7 (bytes 7 to
/// 15 cleared), "Ni03" at byte 10 (bytes 10 to 15). The NESdev wiki's iNES
/// page names "DiskDude!" as the common one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// How reports name it.
    pub name: &'static str,
    /// The header byte it starts at.
    pub offset: usize,
    /// The bytes that identify it.
    pub prefix: &'static [u8],
}

pub const KNOWN_STAMPS: &[Stamp] = &[
    Stamp {
        name: "DiskDude!",
        offset: 7,
        prefix: b"DiskDude",
    },
    Stamp {
        name: "demiforce",
        offset: 7,
        prefix: b"demiforce",
    },
    Stamp {
        name: "Ni0330",
        offset: 10,
        prefix: b"Ni03",
    },
];

/// The known stamps in a header, in offset order.
pub fn stamps(header: &[u8]) -> Vec<&'static Stamp> {
    KNOWN_STAMPS
        .iter()
        .filter(|s| header.get(s.offset..s.offset + s.prefix.len()) == Some(s.prefix))
        .collect()
}

/// One thing found around the game data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Finding {
    /// An archaic iNES header: bytes 7 to 15 aren't trusted and read as zero.
    IgnoredHeaderBytes {
        /// The first and last nonzero byte among 7 to 15.
        first: usize,
        last: usize,
        /// Bytes `first..=last` as hex, and as text with `.` for bytes that
        /// aren't printable ASCII.
        hex: String,
        text: String,
        /// Known stamps among them.
        stamps: Vec<String>,
        /// The mapper from byte 6 alone, which is used.
        mapper: u16,
        /// The mapper byte 7's high nibble would have made.
        literal_mapper: u16,
    },
    /// Byte 7 carries the NES 2.0 signature, but the sizes the header then
    /// gives don't fit in the file, so it was read as archaic iNES.
    Nes20SizesDontFit {
        prg_rom_bytes: u64,
        chr_rom_bytes: u64,
    },
    /// A 512-byte trainer after the header, flagged by byte 6 bit 2.
    Trainer {
        offset: u64,
        len: u64,
        blake3: String,
    },
    /// Byte 6 bit 2 is set, but the file has no room for a trainer, so none
    /// was skipped.
    TrainerFlagWithoutTrainer,
    /// Bytes after the PRG and CHR sizes the header states.
    Trailing {
        offset: u64,
        len: u64,
        blake3: String,
    },
}

impl Finding {
    /// One line for `gp roms scan`.
    pub fn line(&self) -> String {
        match self {
            Finding::IgnoredHeaderBytes {
                first,
                last,
                text,
                stamps,
                mapper,
                literal_mapper,
                ..
            } => {
                let what = if stamps.is_empty() {
                    String::new()
                } else {
                    format!(" (the {} stamp)", stamps.join(", "))
                };
                let literal = if literal_mapper == mapper {
                    String::new()
                } else {
                    format!(" (read literally, byte 7 would make it mapper {literal_mapper})")
                };
                format!(
                    "header: bytes {first}-{last} hold {text:?}{what}, ignored; mapper {mapper} from byte 6{literal}"
                )
            }
            Finding::Nes20SizesDontFit {
                prg_rom_bytes,
                chr_rom_bytes,
            } => format!(
                "header: NES 2.0 signature, but its sizes ({prg_rom_bytes} PRG, {chr_rom_bytes} CHR bytes) don't fit in the file; read as archaic iNES"
            ),
            Finding::Trainer { len, .. } => {
                format!("trainer: {len} bytes after the header, stripped from the hash")
            }
            Finding::TrainerFlagWithoutTrainer => {
                "trainer: byte 6 flags a trainer, but the file has no room for one; none skipped"
                    .into()
            }
            Finding::Trailing { len, .. } => {
                format!("overdump: {} trailing bytes, not hashed", thousands(*len))
            }
        }
    }
}

/// `n` with commas between thousands, e.g. `435,069`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What surrounds the game data in `bytes`, in file order. Fails as
/// [`identify_nes`] does, including on an underdump (a file shorter than the
/// sizes its header states), which is never padded or otherwise fixed.
pub fn inspect(bytes: &[u8]) -> Result<(NesImage, Vec<Finding>), SourceError> {
    let img = identify_nes(bytes)?;
    let h = &bytes[..HEADER_LEN];
    let mut out = Vec::new();
    if img.header.format == HeaderFormat::ArchaicInes {
        if h[7] & 0x0C == 0x08 {
            out.push(Finding::Nes20SizesDontFit {
                prg_rom_bytes: crate::source::nes20_size(h[4], h[9] & 0x0F, 16 * 1024),
                chr_rom_bytes: crate::source::nes20_size(h[5], h[9] >> 4, 8 * 1024),
            });
        }
        let nonzero: Vec<usize> = (7..HEADER_LEN).filter(|&i| h[i] != 0).collect();
        if let (Some(&first), Some(&last)) = (nonzero.first(), nonzero.last()) {
            let run = &h[first..=last];
            out.push(Finding::IgnoredHeaderBytes {
                first,
                last,
                hex: hex::encode(run),
                text: run
                    .iter()
                    .map(|&b| {
                        if b.is_ascii_graphic() || b == b' ' {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect(),
                stamps: stamps(h).iter().map(|s| s.name.to_owned()).collect(),
                mapper: img.header.mapper,
                literal_mapper: u16::from(h[6] >> 4) | u16::from(h[7] & 0xF0),
            });
        }
    }
    if img.trainer_bytes > 0 {
        let t = &bytes[HEADER_LEN..HEADER_LEN + img.trainer_bytes as usize];
        out.push(Finding::Trainer {
            offset: HEADER_LEN as u64,
            len: img.trainer_bytes,
            blake3: blake3::hash(t).to_hex().to_string(),
        });
    } else if img.header.trainer {
        out.push(Finding::TrainerFlagWithoutTrainer);
    }
    if img.trailing_bytes > 0 {
        let start = bytes.len() - img.trailing_bytes as usize;
        out.push(Finding::Trailing {
            offset: start as u64,
            len: img.trailing_bytes,
            blake3: blake3::hash(&bytes[start..]).to_hex().to_string(),
        });
    }
    Ok((img, out))
}

/// One way a file's header disagrees with the registry's board facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Contradiction {
    pub field: &'static str,
    pub header: String,
    pub registry: String,
}

impl std::fmt::Display for Contradiction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: header says {}, registry says {}",
            self.field, self.header, self.registry
        )
    }
}

/// Where `header` (read by the header rules, so ignored bytes are already
/// zero) disagrees with `facts`. The mirroring bit on a board whose mapper
/// sets the mirroring is no board fact, so it never disagrees (the header
/// built from the facts normalises it to 0); the four-screen bit always is
/// one. Submapper and RAM sizes are compared only for a NES 2.0
/// header, the only kind that states them.
pub fn contradictions(header: &NesHeader, facts: &BoardFacts) -> Vec<Contradiction> {
    let mut out = Vec::new();
    let mut check = |field, header: String, registry: String| {
        if header != registry {
            out.push(Contradiction {
                field,
                header,
                registry,
            });
        }
    };
    check(
        "mapper",
        header.mapper.to_string(),
        facts.mapper.to_string(),
    );
    check(
        "PRG ROM bytes",
        header.prg_rom_bytes.to_string(),
        facts.prg_rom_bytes.to_string(),
    );
    check(
        "CHR ROM bytes",
        header.chr_rom_bytes.to_string(),
        facts.chr_rom_bytes.to_string(),
    );
    let mirroring = |m: Mirroring| {
        match m {
            Mirroring::Horizontal => "horizontal",
            Mirroring::Vertical => "vertical",
            Mirroring::FourScreen => "four-screen",
        }
        .to_owned()
    };
    match facts.mirroring {
        BoardMirroring::Mapper => check(
            "four-screen",
            (header.mirroring == Mirroring::FourScreen).to_string(),
            "false".into(),
        ),
        fixed => check(
            "mirroring",
            mirroring(header.mirroring),
            mirroring(match fixed {
                BoardMirroring::Horizontal => Mirroring::Horizontal,
                BoardMirroring::Vertical => Mirroring::Vertical,
                _ => Mirroring::FourScreen,
            }),
        ),
    }
    check(
        "battery",
        header.battery.to_string(),
        facts.battery.to_string(),
    );
    if header.format == HeaderFormat::Nes20 {
        check(
            "submapper",
            header.submapper.to_string(),
            facts.submapper.to_string(),
        );
        if let Some(prg_ram) = header.prg_ram_bytes {
            check(
                "PRG RAM bytes",
                prg_ram.to_string(),
                facts.prg_ram_bytes.to_string(),
            );
        }
        if let Some(chr_ram) = header.chr_ram_bytes {
            check(
                "CHR RAM bytes",
                chr_ram.to_string(),
                facts.chr_ram_bytes.to_string(),
            );
        }
    }
    out
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DumpError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(
        "the header contradicts the source registry ({}); `gp roms clean` writes a copy with the registry's header",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
    )]
    Contradicts(Vec<Contradiction>),
    #[error("the registry's board facts can't be written as a header: {0}")]
    Unrepresentable(String),
}

/// The header the registry's facts make. A plain iNES header (bytes 8 to 15
/// zero, which is what tetanes-core reads for the boards Game Printer runs)
/// when that can state them, else NES 2.0. The mirroring bit on a board
/// whose mapper sets it is 0.
pub fn header_from_facts(facts: &BoardFacts) -> Result<[u8; 16], DumpError> {
    let unrepresentable = |why: String| Err(DumpError::Unrepresentable(why));
    let (prg_units, chr_units) = (facts.prg_rom_bytes / 0x4000, facts.chr_rom_bytes / 0x2000);
    if !facts.prg_rom_bytes.is_multiple_of(0x4000) || !facts.chr_rom_bytes.is_multiple_of(0x2000) {
        return unrepresentable(format!(
            "PRG ROM ({}) or CHR ROM ({}) isn't a whole number of 16KB or 8KB banks",
            facts.prg_rom_bytes, facts.chr_rom_bytes
        ));
    }
    let nes20 = facts.mapper > 0xFF || facts.submapper != 0 || prg_units > 0xFF || chr_units > 0xFF;
    if facts.mapper > 0xFFF || prg_units > 0xEFF || chr_units > 0xEFF || facts.submapper > 0xF {
        return unrepresentable("mapper, submapper or a size is out of NES 2.0's range".into());
    }
    let mut h = [0u8; 16];
    h[..4].copy_from_slice(INES_MAGIC);
    h[4] = prg_units as u8;
    h[5] = chr_units as u8;
    h[6] = ((facts.mapper & 0x0F) as u8) << 4
        | match facts.mirroring {
            BoardMirroring::Vertical => 0x01,
            BoardMirroring::FourScreen => 0x08,
            BoardMirroring::Horizontal | BoardMirroring::Mapper => 0,
        }
        | if facts.battery { 0x02 } else { 0 };
    h[7] = (facts.mapper & 0xF0) as u8;
    if nes20 {
        let shift = |bytes: u64| -> Result<u8, DumpError> {
            if bytes == 0 {
                return Ok(0);
            }
            (1..=14u8)
                .find(|&n| 64u64 << n == bytes)
                .ok_or_else(|| DumpError::Unrepresentable(format!("a RAM size of {bytes} bytes")))
        };
        h[7] |= 0x08;
        h[8] = facts.submapper << 4 | (facts.mapper >> 8) as u8;
        h[9] = ((chr_units >> 8) as u8) << 4 | (prg_units >> 8) as u8;
        let prg_ram = shift(facts.prg_ram_bytes)?;
        h[10] = if facts.battery { prg_ram << 4 } else { prg_ram };
        h[11] = shift(facts.chr_ram_bytes)?;
    }
    Ok(h)
}

/// The file's header with what the header rules don't trust set to zero:
/// bytes 7 to 15 of an archaic iNES header, and the trainer flag (the
/// trainer itself is removed with it).
pub fn trusted_header(bytes: &[u8], img: &NesImage) -> [u8; 16] {
    let mut h = [0u8; 16];
    h.copy_from_slice(&bytes[..HEADER_LEN]);
    if img.header.format == HeaderFormat::ArchaicInes {
        h[7..].fill(0);
    }
    h[6] &= !0x04;
    h
}

/// Which header a cleaned image got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeaderRule {
    /// Built from the registry's board facts.
    Registry,
    /// The file's own header as the header rules read it.
    File,
}

/// A cleaned image and what cleaning did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    pub bytes: Vec<u8>,
    pub rule: HeaderRule,
    pub original_header: [u8; 16],
    pub header: [u8; 16],
    pub findings: Vec<Finding>,
    /// Where the file's header disagreed with the registry, which won.
    pub contradictions: Vec<Contradiction>,
}

/// Bytes of the input that the cleaned image doesn't keep, as the sidecar
/// next to a cleaned copy records them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Removed {
    /// `header` (the original 16 bytes, replaced), `trainer` or `trailing`.
    pub what: &'static str,
    pub offset: u64,
    pub len: u64,
    pub blake3: String,
    /// The bytes themselves, for the header only: never game data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hex: Option<String>,
}

impl Cleaned {
    /// What was removed or replaced, in file order.
    pub fn removed(&self) -> Vec<Removed> {
        let mut out = Vec::new();
        if self.original_header != self.header {
            out.push(Removed {
                what: "header",
                offset: 0,
                len: HEADER_LEN as u64,
                blake3: blake3::hash(&self.original_header).to_hex().to_string(),
                hex: Some(hex::encode(self.original_header)),
            });
        }
        for f in &self.findings {
            match f {
                Finding::Trainer {
                    offset,
                    len,
                    blake3,
                } => out.push(Removed {
                    what: "trainer",
                    offset: *offset,
                    len: *len,
                    blake3: blake3.clone(),
                    hex: None,
                }),
                Finding::Trailing {
                    offset,
                    len,
                    blake3,
                } => out.push(Removed {
                    what: "trailing",
                    offset: *offset,
                    len: *len,
                    blake3: blake3.clone(),
                    hex: None,
                }),
                _ => {}
            }
        }
        out
    }
}

/// Header, PRG and CHR: the trainer and trailing bytes dropped, the header
/// built from `facts` when given (a registered source), else the file's
/// trusted header. Contradictions with `facts` are recorded, not refused:
/// cleaning was asked for.
pub fn clean(bytes: &[u8], facts: Option<&BoardFacts>) -> Result<Cleaned, DumpError> {
    let (img, findings) = inspect(bytes)?;
    let (rule, header, contradictions) = match facts {
        Some(f) => (
            HeaderRule::Registry,
            header_from_facts(f)?,
            contradictions(&img.header, f),
        ),
        None => (HeaderRule::File, trusted_header(bytes, &img), Vec::new()),
    };
    let start = HEADER_LEN + img.trainer_bytes as usize;
    let end = bytes.len() - img.trailing_bytes as usize;
    let mut out = Vec::with_capacity(HEADER_LEN + end - start);
    out.extend_from_slice(&header);
    out.extend_from_slice(&bytes[start..end]);
    let mut original_header = [0u8; 16];
    original_header.copy_from_slice(&bytes[..HEADER_LEN]);
    Ok(Cleaned {
        bytes: out,
        rule,
        original_header,
        header,
        findings,
        contradictions,
    })
}

/// The image a runtime should load: [`clean`]'s, but a header that
/// contradicts `facts` is refused rather than replaced. For a file that's
/// already clean this is the file itself, unchanged.
pub fn runnable<'a>(
    bytes: &'a [u8],
    facts: Option<&BoardFacts>,
) -> Result<Cow<'a, [u8]>, DumpError> {
    let cleaned = clean(bytes, facts)?;
    if !cleaned.contradictions.is_empty() {
        return Err(DumpError::Contradicts(cleaned.contradictions));
    }
    Ok(if cleaned.bytes == bytes {
        Cow::Borrowed(bytes)
    } else {
        Cow::Owned(cleaned.bytes)
    })
}

/// A tag in a file name in the GoodNES or No-Intro style, such as `[o]` or
/// `(Hack)`. Names are labels and can be wrong, so tags are hints only and
/// never evidence (docs/sources.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NameTag {
    pub tag: String,
    pub meaning: &'static str,
}

/// The tags in `name` that say something about how the dump was made or
/// changed. Region and revision tags such as `(U)` or `(PRG 0)` are left
/// out.
pub fn name_tags(name: &str) -> Vec<NameTag> {
    let mut out = Vec::new();
    let mut rest = name;
    while let Some(open) = rest.find(['[', '(']) {
        let close = if rest.as_bytes()[open] == b'[' {
            ']'
        } else {
            ')'
        };
        let Some(len) = rest[open + 1..].find(close) else {
            break;
        };
        let inner = &rest[open + 1..open + 1 + len];
        let bracket = close == ']';
        let meaning = if bracket {
            match inner.chars().next() {
                Some('!') => Some("a verified good dump"),
                Some('a') => Some("an alternate dump"),
                Some('b') => Some("a bad dump"),
                Some('f') => Some("fixed to run on emulators"),
                Some('h') => Some("a hack"),
                Some('o') => Some("an overdump"),
                Some('p') => Some("a pirate release"),
                Some('t') => Some("trained (cheats or a trainer added)"),
                Some('T') => Some("a translation"),
                _ => None,
            }
        } else {
            match inner {
                "Hack" => Some("a hack"),
                "Beta" | "Proto" | "Prototype" => Some("a prototype"),
                "Pirate" => Some("a pirate release"),
                "Unl" => Some("an unlicensed release"),
                _ => None,
            }
        };
        if let Some(meaning) = meaning {
            out.push(NameTag {
                tag: rest[open..=open + 1 + len].to_owned(),
                meaning,
            });
        }
        rest = &rest[open + 1 + len + 1..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::TRAINER_LEN;

    fn header(prg16k: u8, chr8k: u8, flags6: u8, flags7: u8) -> [u8; 16] {
        let mut h = [0u8; 16];
        h[..4].copy_from_slice(INES_MAGIC);
        h[4] = prg16k;
        h[5] = chr8k;
        h[6] = flags6;
        h[7] = flags7;
        h
    }

    fn game(h: [u8; 16]) -> Vec<u8> {
        let len = usize::from(h[4]) * 0x4000 + usize::from(h[5]) * 0x2000;
        let mut v = h.to_vec();
        v.extend((0..len).map(|i| (i * 7 % 253) as u8));
        v
    }

    fn uxrom() -> Vec<u8> {
        game(header(8, 0, 0x21, 0x00)) // mapper 2, vertical
    }

    fn with_bytes(rom: &[u8], at: usize, text: &[u8]) -> Vec<u8> {
        let mut v = rom.to_vec();
        v[at..at + text.len()].copy_from_slice(text);
        v
    }

    #[test]
    fn diskdude_is_archaic_and_mapper_comes_from_byte_6() {
        let clean = uxrom();
        let dirty = with_bytes(&clean, 7, b"DiskDude!");
        let (img, findings) = inspect(&dirty).unwrap();
        assert_eq!(img.header.format, HeaderFormat::ArchaicInes);
        assert_eq!(img.header.mapper, 2);
        assert_eq!(img.sha256, identify_nes(&clean).unwrap().sha256);
        let [
            Finding::IgnoredHeaderBytes {
                first,
                last,
                stamps,
                literal_mapper,
                ..
            },
        ] = &findings[..]
        else {
            panic!("{findings:?}");
        };
        assert_eq!((*first, *last, *literal_mapper), (7, 15, 66));
        assert_eq!(stamps, &["DiskDude!"]);
        assert_eq!(
            findings[0].line(),
            "header: bytes 7-15 hold \"DiskDude!\" (the DiskDude! stamp), ignored; mapper 2 from byte 6 (read literally, byte 7 would make it mapper 66)"
        );
        assert_eq!(runnable(&dirty, None).unwrap(), &clean[..]);
    }

    #[test]
    fn ni0330_and_demiforce_are_named() {
        let clean = game(header(2, 1, 0x42, 0x00));
        let ni = with_bytes(&clean, 10, b"Ni0330");
        let (img, f) = inspect(&ni).unwrap();
        assert_eq!(img.header.mapper, 4);
        assert!(
            matches!(&f[0], Finding::IgnoredHeaderBytes { first: 10, last: 15, stamps, .. } if stamps == &["Ni0330"])
        );
        assert_eq!(clean_bytes(&ni), clean);
        let demi = with_bytes(&clean, 7, b"demiforce");
        assert_eq!(stamps(&demi[..16])[0].name, "demiforce");
        assert_eq!(clean_bytes(&demi), clean);
    }

    fn clean_bytes(rom: &[u8]) -> Vec<u8> {
        clean(rom, None).unwrap().bytes
    }

    #[test]
    fn unknown_text_is_ignored_without_a_name() {
        let dirty = with_bytes(&uxrom(), 12, b"abcd");
        let (_, f) = inspect(&dirty).unwrap();
        assert!(
            matches!(&f[0], Finding::IgnoredHeaderBytes { first: 12, stamps, .. } if stamps.is_empty())
        );
    }

    #[test]
    fn trainer_and_trailing_bytes_are_dropped() {
        let plain = uxrom();
        let mut dirty = plain[..16].to_vec();
        dirty[6] |= 0x04;
        dirty.extend(std::iter::repeat_n(0xEE, TRAINER_LEN));
        dirty.extend_from_slice(&plain[16..]);
        dirty.extend(std::iter::repeat_n(0x55, 435_069));
        let (img, f) = inspect(&dirty).unwrap();
        assert_eq!(img.trainer_bytes, 512);
        assert_eq!(img.trailing_bytes, 435_069);
        assert_eq!(img.sha256, identify_nes(&plain).unwrap().sha256);
        assert_eq!(f.len(), 2);
        assert_eq!(
            f[0].line(),
            "trainer: 512 bytes after the header, stripped from the hash"
        );
        assert_eq!(f[1].line(), "overdump: 435,069 trailing bytes, not hashed");
        let Finding::Trailing { offset, .. } = f[1] else {
            panic!()
        };
        assert_eq!(offset as usize, 16 + 512 + plain.len() - 16);
        let cleaned = clean(&dirty, None).unwrap();
        assert_eq!(cleaned.bytes, plain);
        let removed = cleaned.removed();
        let what: Vec<_> = removed.iter().map(|r| (r.what, r.offset, r.len)).collect();
        assert_eq!(
            what,
            [
                ("header", 0, 16),
                ("trainer", 16, 512),
                ("trailing", 528 + plain.len() as u64 - 16, 435_069)
            ]
        );
        assert_eq!(
            removed[0].hex.as_deref(),
            Some(&hex::encode(&dirty[..16])[..])
        );
        assert_eq!(
            removed[1].blake3,
            blake3::hash(&[0xEE; 512]).to_hex().to_string()
        );
    }

    #[test]
    fn a_trainer_flag_without_room_skips_nothing() {
        let mut dirty = uxrom();
        dirty[6] |= 0x04;
        let (img, f) = inspect(&dirty).unwrap();
        assert_eq!(img.trainer_bytes, 0);
        assert_eq!(f, [Finding::TrainerFlagWithoutTrainer]);
        assert_eq!(clean_bytes(&dirty), uxrom());
    }

    #[test]
    fn an_underdump_is_refused() {
        let short = &uxrom()[..1000];
        assert!(matches!(inspect(short), Err(SourceError::Truncated { .. })));
        assert!(clean(short, None).is_err());
    }

    #[test]
    fn nes20_that_doesnt_fit_is_read_as_archaic() {
        let mut dirty = uxrom();
        dirty[7] = 0x08;
        dirty[9] = 0x01; // PRG $108 banks, far more than the file has
        let (img, f) = inspect(&dirty).unwrap();
        assert_eq!(img.header.format, HeaderFormat::ArchaicInes);
        assert_eq!(img.sha256, identify_nes(&uxrom()).unwrap().sha256);
        assert!(matches!(f[0], Finding::Nes20SizesDontFit { .. }));
        assert!(matches!(
            f[1],
            Finding::IgnoredHeaderBytes {
                first: 7,
                last: 9,
                ..
            }
        ));
    }

    fn facts(mapper: u16, mirroring: BoardMirroring, battery: bool) -> BoardFacts {
        BoardFacts {
            mapper,
            submapper: 0,
            mirroring,
            battery,
            prg_rom_bytes: 0x20000,
            chr_rom_bytes: 0,
            prg_ram_bytes: 0,
            chr_ram_bytes: 0x2000,
        }
    }

    #[test]
    fn registry_facts_make_the_header() {
        let f = facts(2, BoardMirroring::Vertical, false);
        assert_eq!(header_from_facts(&f).unwrap(), header(8, 0, 0x21, 0));
        let dirty = with_bytes(&uxrom(), 7, b"DiskDude!");
        assert_eq!(runnable(&dirty, Some(&f)).unwrap(), &uxrom()[..]);
        // A mapper-set mirroring bit is no fact: either value runs, as 0.
        let m = facts(2, BoardMirroring::Mapper, false);
        assert!(contradictions(&identify_nes(&uxrom()).unwrap().header, &m).is_empty());
        assert_eq!(runnable(&uxrom(), Some(&m)).unwrap()[6], 0x20);
    }

    #[test]
    fn a_contradicting_header_is_refused_but_cleaned() {
        let f = facts(2, BoardMirroring::Horizontal, true);
        let err = runnable(&uxrom(), Some(&f)).unwrap_err();
        let DumpError::Contradicts(c) = &err else {
            panic!("{err}")
        };
        let fields: Vec<_> = c.iter().map(|c| c.field).collect();
        assert_eq!(fields, ["mirroring", "battery"]);
        let cleaned = clean(&uxrom(), Some(&f)).unwrap();
        assert_eq!(cleaned.contradictions.len(), 2);
        assert_eq!(cleaned.bytes[6], 0x22);
    }

    #[test]
    fn nes20_headers_from_facts_and_back() {
        let mut f = facts(0x123, BoardMirroring::Horizontal, true);
        f.submapper = 2;
        f.prg_ram_bytes = 0x2000;
        let h = header_from_facts(&f).unwrap();
        let mut rom = h.to_vec();
        rom.extend(std::iter::repeat_n(0, 0x20000));
        let img = identify_nes(&rom).unwrap();
        assert_eq!(img.header.format, HeaderFormat::Nes20);
        assert_eq!((img.header.mapper, img.header.submapper), (0x123, 2));
        assert_eq!(img.header.prg_ram_bytes, Some(0x2000));
        assert_eq!(img.header.chr_ram_bytes, Some(0x2000));
        assert!(contradictions(&img.header, &f).is_empty());
        assert_eq!(runnable(&rom, Some(&f)).unwrap(), &rom[..]);
    }

    #[test]
    fn clean_is_idempotent() {
        let dirty = with_bytes(&uxrom(), 7, b"DiskDude!");
        let once = clean_bytes(&dirty);
        assert_eq!(clean_bytes(&once), once);
        assert!(matches!(runnable(&once, None).unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn name_tags_are_hints() {
        let tags = name_tags("Mega Man (U) [o1][T+Eng1.0_Foo] (Hack) (PRG 0) [!].nes");
        let got: Vec<_> = tags.iter().map(|t| (t.tag.as_str(), t.meaning)).collect();
        assert_eq!(
            got,
            [
                ("[o1]", "an overdump"),
                ("[T+Eng1.0_Foo]", "a translation"),
                ("(Hack)", "a hack"),
                ("[!]", "a verified good dump"),
            ]
        );
        assert!(name_tags("Super Mario Bros (JU) (PRG 1).nes").is_empty());
        assert!(name_tags("broken [h").is_empty());
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(435_069), "435,069");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
