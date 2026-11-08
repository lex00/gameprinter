//! Format modules: a format that keeps its map keys, its spans, its lifted
//! files and its encoder in one module of its own, behind [`Module`]. The
//! asset map gives each such format a variant of [`crate::Asset`] and a
//! line in [`crate::Asset::module`]; nothing else in the crate changes when
//! one is added.

use crate::map::{AssetMap, Region, Span};
use crate::{Error, File};

/// Reads a lifted file by its path in the lifted directory.
pub type ReadFile<'a> = &'a dyn Fn(&str) -> std::io::Result<Vec<u8>>;

/// A ROM's PRG and CHR data, for a module's `lift`.
pub struct Rom<'a> {
    pub(crate) prg: &'a [u8],
    pub(crate) chr: &'a [u8],
}

impl Rom<'_> {
    /// `len` bytes of `region` from `offset`, or an error if they run past
    /// its end.
    pub fn bytes(&self, region: Region, offset: u32, len: u32) -> Result<&[u8], String> {
        let data = match region {
            Region::Prg => self.prg,
            Region::Chr => self.chr,
        };
        let end = u64::from(offset) + u64::from(len);
        if end > data.len() as u64 {
            return Err(format!(
                "{} offset {offset:#06x} + {len} bytes is past the end of the ROM's {} data ({} bytes)",
                region.name(),
                region.name(),
                data.len()
            ));
        }
        Ok(&data[offset as usize..end as usize])
    }
}

/// What a format module does for one asset of its format.
pub trait Module {
    fn name(&self) -> &str;
    /// The format's name in the map, such as "byte-rows".
    fn format(&self) -> &'static str;
    /// Checks the asset's own keys. Runs before [`Module::spans`], so it
    /// must check every size that `spans` multiplies or adds.
    fn validate(&self) -> Result<(), String>;
    /// Checks what the asset says about other assets of the map, once
    /// every asset has passed [`Module::validate`].
    fn check_refs(&self, _map: &AssetMap) -> Result<(), String> {
        Ok(())
    }
    /// The ROM bytes the asset owns, in the order `encode` returns them.
    fn spans(&self) -> Vec<Span>;
    /// The file the asset lifts to; a directory ends in `/`.
    fn file(&self) -> String {
        format!("{}.toml", self.name())
    }
    /// Every file the asset lifts to.
    fn files(&self) -> Vec<String> {
        vec![self.file()]
    }
    /// Decodes the asset from the ROM into its files. `source` is the
    /// game's title, for the files' comments.
    fn lift(&self, source: &str, map: &AssetMap, rom: &Rom) -> Result<Vec<File>, String>;
    /// Encodes the asset from its lifted files: one buffer per span.
    fn encode(&self, map: &AssetMap, read: ReadFile) -> Result<Vec<Vec<u8>>, Error>;
}

/// A lifted text file, read as UTF-8, with errors naming it.
pub fn read_text(read: ReadFile, file: &str) -> Result<String, Error> {
    let fail = |msg: String| Error::File {
        file: file.to_owned(),
        msg,
    };
    let raw = read(file).map_err(|e| fail(e.to_string()))?;
    String::from_utf8(raw).map_err(|_| fail("isn't UTF-8 text".into()))
}

/// A span with a part name: the asset's, or the asset's and a suffix.
pub fn span(region: Region, offset: u32, len: u32, part: String, label: Option<String>) -> Span {
    Span {
        region,
        offset,
        len,
        part,
        label,
    }
}

/// Checks a span of `len` bytes from `offset` ends within [`MAX_SPAN`],
/// so no sum of the two can overflow.
pub fn check_span(what: &str, offset: u32, len: u64) -> Result<(), String> {
    if u64::from(offset) + len > crate::map::MAX_SPAN {
        return Err(format!(
            "{what}: offset {offset:#x} + {len} bytes is past {} bytes, more than any NES ROM holds",
            crate::map::MAX_SPAN
        ));
    }
    Ok(())
}

/// Lowercase letters, digits, `-` and `_`: safe as a file name.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// A symbol, optionally plus a byte count: `Name` or `Name+3`.
pub fn symbol_label(label: &str) -> bool {
    let (name, plus) = label.split_once('+').unwrap_or((label, "0"));
    symbol(name) && !plus.is_empty() && plus.bytes().all(|b| b.is_ascii_digit())
}

/// Where a span ends: a symbol, plus or minus a byte count (`Name`,
/// `Name+3`, `Name-3`). Checked against the disassembly's build by
/// `scripts/disasm/check_asset_labels.py`.
pub fn end_label(label: &str) -> bool {
    let (name, n) = match label.find(['+', '-']) {
        Some(i) => (&label[..i], &label[i + 1..]),
        None => (label, "0"),
    };
    symbol(name) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
}

fn symbol(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Checks a label and an end label, if given.
pub fn check_labels(
    what: &str,
    label: Option<&String>,
    end: Option<&String>,
) -> Result<(), String> {
    if let Some(l) = label.filter(|l| !symbol_label(l)) {
        return Err(format!(
            "{what}: label {l:?} must be a symbol, or a symbol plus a byte count (Name+3)"
        ));
    }
    if let Some(l) = end.filter(|l| !end_label(l)) {
        return Err(format!(
            "{what}: end_label {l:?} must be a symbol, plus or minus a byte count if needed (Name+3, Name-3)"
        ));
    }
    Ok(())
}

/// Checks a one-line note or name.
pub fn one_line(what: &str, s: &str) -> Result<(), String> {
    if s.contains(['\n', '\r']) {
        return Err(format!("{what} must be one line"));
    }
    Ok(())
}
