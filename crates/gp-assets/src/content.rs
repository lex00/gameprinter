//! What each span of an asset map holds, as hashes (docs/assets.md,
//! "Content"): the BLAKE3 of its bytes and its length, and a short
//! fingerprint of each 16 bytes of it, so `gp assets find` can find the
//! span in another dump or version of the game at any offset. A map may
//! also hold the fingerprints of its ROM's whole PRG (`[code]`), which tell
//! which source a ROM's program is closest to.
//!
//! None of it is ROM data: a hash can't be turned back into the bytes. The
//! section is written by `gp assets hash` from the person's own ROM, at the
//! end of the map after [`MARKER`].

use serde::Deserialize;

use crate::Error;
use crate::map::{AssetMap, Region, Span};

/// The bytes each fingerprint covers. A span's last fingerprint covers what
/// is left, fewer bytes when its length isn't a multiple of this.
pub const CHUNK: usize = 16;

/// Fingerprints per line of a `chunks` list (8 hex digits each).
const LINE: usize = 32;

/// The line the written section starts with. Everything from it to the end
/// of the file is the section, written again by each `gp assets hash`.
pub const MARKER: &str = "# Content, written by `gp assets hash` from the ROM the map describes";

/// One span's content.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    /// The span's part name, as `Asset::spans` gives it.
    pub part: String,
    pub region: Region,
    pub offset: u32,
    pub len: u32,
    /// BLAKE3 of the span's bytes, 64 hex digits.
    pub blake3: String,
    /// The fingerprint of each [`CHUNK`] bytes, 8 hex digits each, in lines.
    #[serde(default)]
    pub chunks: Vec<String>,
}

/// The fingerprints of the ROM's whole PRG, [`CHUNK`] bytes each.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Code {
    pub region: Region,
    pub len: u32,
    pub chunks: Vec<String>,
}

/// A short hash of a few bytes, for finding them: not cryptographic, and
/// fixed forever, since maps hold its values.
pub fn fingerprint(bytes: &[u8]) -> u32 {
    let mut h: u64 = 0x9E37_79B9_7F4A_7C15 ^ bytes.len() as u64;
    for c in bytes.chunks(8) {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        h ^= h >> 31;
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 32) as u32
}

/// The fingerprint of each [`CHUNK`] bytes of `bytes`.
pub fn chunk_prints(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks(CHUNK).map(fingerprint).collect()
}

/// BLAKE3 in hex.
pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn lines(prints: &[u32]) -> Vec<String> {
    prints
        .chunks(LINE)
        .map(|l| l.iter().map(|p| format!("{p:08x}")).collect())
        .collect()
}

/// The fingerprints a `chunks` list holds.
pub fn parse_prints(lines: &[String]) -> Result<Vec<u32>, String> {
    let mut out = Vec::new();
    for l in lines {
        if l.len() % 8 != 0 || !l.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("chunks must be lines of 8-hex-digit fingerprints".into());
        }
        for i in (0..l.len()).step_by(8) {
            out.push(u32::from_str_radix(&l[i..i + 8], 16).map_err(|e| e.to_string())?);
        }
    }
    Ok(out)
}

/// How many fingerprints a span of `len` bytes has.
pub fn chunk_count(len: u32) -> usize {
    (len as usize).div_ceil(CHUNK)
}

/// Every span's content and the PRG's fingerprints, from the ROM the map
/// describes (`prg` and `chr` its data without the header).
pub fn compute(
    map: &AssetMap,
    prg: &[u8],
    chr: &[u8],
    with_code: bool,
) -> Result<(Vec<Content>, Option<Code>), Error> {
    let region = |r: Region| if r == Region::Prg { prg } else { chr };
    let bytes = |s: &Span| -> Result<&[u8], Error> {
        let data = region(s.region);
        let end = s.offset as usize + s.len as usize;
        data.get(s.offset as usize..end).ok_or(Error::OutOfRange {
            part: s.part.clone(),
            region: s.region.name(),
            offset: s.offset,
            len: s.len,
            size: data.len(),
        })
    };
    let mut out = Vec::new();
    for asset in &map.assets {
        let spans = asset.spans();
        for s in spans {
            let b = bytes(&s)?;
            out.push(Content {
                part: s.part.clone(),
                region: s.region,
                offset: s.offset,
                len: s.len,
                blake3: blake3_hex(b),
                chunks: lines(&chunk_prints(b)),
            });
        }
    }
    let code = with_code.then(|| Code {
        region: Region::Prg,
        len: prg.len() as u32,
        chunks: lines(&chunk_prints(prg)),
    });
    Ok((out, code))
}

/// `map_text` with its content section (from [`MARKER`] on) replaced by
/// this one. The rest of the file is kept as it is.
pub fn write_section(map_text: &str, content: &[Content], code: Option<&Code>) -> String {
    let head = match map_text.find(MARKER) {
        Some(i) => &map_text[..i],
        None => map_text,
    };
    let mut out = head.trim_end().to_owned();
    out.push_str("\n\n");
    out.push_str(MARKER);
    out.push_str(
        ".\n# Each span's BLAKE3 and length, and a fingerprint of each 16 bytes of it,\n\
         # so `gp assets find` finds the span in another dump (docs/assets.md,\n\
         # \"Content\"). Hashes, not ROM data. Don't edit by hand.\n",
    );
    let list = |out: &mut String, chunks: &[String]| {
        if chunks.is_empty() {
            return;
        }
        out.push_str("chunks = [\n");
        for l in chunks {
            out.push_str(&format!("  \"{l}\",\n"));
        }
        out.push_str("]\n");
    };
    for c in content {
        out.push_str(&format!(
            "\n[[content]]\npart = {:?}\nregion = \"{}\"\noffset = {:#06X}\nlen = {}\nblake3 = \"{}\"\n",
            c.part,
            c.region.name(),
            c.offset,
            c.len,
            c.blake3
        ));
        list(&mut out, &c.chunks);
    }
    if let Some(code) = code {
        out.push_str(&format!(
            "\n# The PRG's fingerprints, which `gp assets find` compares a ROM's program\n\
             # with to pick the model.\n[code]\nregion = \"{}\"\nlen = {}\n",
            code.region.name(),
            code.len
        ));
        list(&mut out, &code.chunks);
    }
    out.replace("offset = 0X", "offset = 0x")
}

/// Checks the content section against the map's spans: each entry names a
/// span of the same place and length (or, `relative`, a pointers asset with
/// targets), once, with well-formed hashes.
pub(crate) fn validate(map: &AssetMap) -> Result<(), Error> {
    let bad = |m: String| Err(Error::Map(format!("content: {m}")));
    let spans = map.spans();
    let mut seen = std::collections::BTreeSet::new();
    for c in &map.content {
        if !seen.insert(c.part.as_str()) {
            return bad(format!("{} is given twice", c.part));
        }
        if c.blake3.len() != 64 || !c.blake3.bytes().all(|b| b.is_ascii_hexdigit()) {
            return bad(format!("{}: blake3 isn't 64 hex digits", c.part));
        }
        let prints =
            parse_prints(&c.chunks).map_err(|e| Error::Map(format!("content: {}: {e}", c.part)))?;
        if !spans.iter().any(|s| {
            s.part == c.part && s.region == c.region && s.offset == c.offset && s.len == c.len
        }) {
            return bad(format!(
                "{} isn't a span of the map at {} {:#06x}, {} bytes (run gp assets hash again)",
                c.part,
                c.region.name(),
                c.offset,
                c.len
            ));
        }
        if prints.len() != chunk_count(c.len) {
            return bad(format!(
                "{}: {} fingerprints for {} bytes, where there are {}",
                c.part,
                prints.len(),
                c.len,
                chunk_count(c.len)
            ));
        }
    }
    if let Some(code) = &map.code {
        let n = parse_prints(&code.chunks).map_err(|e| Error::Map(format!("code: {e}")))?;
        if code.region != Region::Prg || n.len() != chunk_count(code.len) {
            return Err(Error::Map(format!(
                "code: the PRG's fingerprints, {} for {} bytes, where there are {}",
                n.len(),
                code.len,
                chunk_count(code.len)
            )));
        }
    }
    Ok(())
}
