//! The `blobs` format: a span of variable-length records that one or more
//! pointer tables point into, such as Mega Man 2's sprite frames (split
//! low and high tables, 256 frame numbers each) or Super Mario Bros. 2's
//! sound effects (one-byte offsets from the table). The span is cut where
//! any pointer points, and each piece is a blob. The lifted file lists the
//! blobs in storage order and gives each pointer as a blob's number, so a
//! blob may grow or shrink and `pack` moves the ones after it and writes
//! their pointers again.
//!
//! Data that holds CPU addresses of its own, such as music whose headers
//! point at their patterns, can't move: with `fixed = true` each blob
//! keeps its `address` in the file and `pack` refuses one that has moved.

use std::fmt::Write as _;

use serde::Deserialize;
use toml::Value;

use crate::format::{self, Module, ReadFile, Rom};
use crate::map::{AssetMap, Region, Span};
use crate::text::{self, Reader};
use crate::{Error, File};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blobs {
    pub name: String,
    pub label: Option<String>,
    pub end_label: Option<String>,
    pub region: Region,
    /// The blobs' span.
    pub offset: u32,
    pub size: u32,
    /// The CPU address of PRG offset `prg_base`.
    pub cpu_base: u32,
    #[serde(default)]
    pub prg_base: u32,
    /// Bytes at the start of each blob that the file shows on their own
    /// (`head`), before rows of `width` bytes. Both 0 by default: one list.
    #[serde(default)]
    pub head: u32,
    #[serde(default)]
    pub width: u32,
    /// True when the blobs hold addresses of their own and can't move.
    #[serde(default)]
    pub fixed: bool,
    /// The byte `pack` fills the span's end with when the blobs take less.
    /// Without it they must take the whole span.
    pub fill: Option<u8>,
    /// One-line notes for the file's comments, by blob number.
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(rename = "table")]
    pub tables: Vec<Table>,
}

/// One pointer table into the blobs.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// Its key in the lifted file's `[pointers]`.
    pub name: String,
    pub label: Option<String>,
    pub end_label: Option<String>,
    pub hi_label: Option<String>,
    pub hi_end_label: Option<String>,
    /// The low bytes, the little-endian words, or the one-byte offsets.
    pub offset: u32,
    /// The high bytes, for a table split in two.
    pub hi_offset: Option<u32>,
    pub count: u32,
    /// For a table of one-byte offsets: the CPU address they count from.
    pub base: Option<u32>,
}

impl Table {
    fn spans(&self, region: Region, asset: &str) -> Vec<Span> {
        let part = format!("{asset} ({})", self.name);
        match (self.hi_offset, self.base) {
            (Some(hi), _) => vec![
                format::span(
                    region,
                    self.offset,
                    self.count,
                    format!("{part} low bytes"),
                    self.label.clone(),
                ),
                format::span(
                    region,
                    hi,
                    self.count,
                    format!("{part} high bytes"),
                    self.hi_label.clone(),
                ),
            ],
            (None, Some(_)) => vec![format::span(
                region,
                self.offset,
                self.count,
                part,
                self.label.clone(),
            )],
            (None, None) => vec![format::span(
                region,
                self.offset,
                self.count * 2,
                part,
                self.label.clone(),
            )],
        }
    }

    /// The addresses the table holds, from its spans' bytes.
    fn values(&self, bytes: &[&[u8]]) -> Vec<u32> {
        match (self.hi_offset, self.base) {
            (Some(_), _) => bytes[0]
                .iter()
                .zip(bytes[1])
                .map(|(&l, &h)| u32::from(l) | u32::from(h) << 8)
                .collect(),
            (None, Some(base)) => bytes[0].iter().map(|&b| base + u32::from(b)).collect(),
            (None, None) => bytes[0]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|w| u32::from(w[0]) | u32::from(w[1]) << 8)
                .collect(),
        }
    }
}

/// A pointer as the file gives it: a blob's number, or an address that
/// points outside the blobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pointer {
    Blob(usize),
    Raw(u32),
}

impl Blobs {
    /// The CPU address of PRG offset `prg`, if the bank maps it.
    fn address(&self, prg: u32) -> Option<u32> {
        prg.checked_sub(self.prg_base).map(|o| self.cpu_base + o)
    }

    /// The PRG offset in the span a CPU address points at.
    fn target(&self, address: u32) -> Option<u32> {
        let prg = address.checked_sub(self.cpu_base)? + self.prg_base;
        (prg >= self.offset && prg < self.offset + self.size).then_some(prg)
    }

    fn table_bytes<'r>(&self, t: &Table, rom: &'r Rom) -> Result<Vec<&'r [u8]>, String> {
        t.spans(self.region, &self.name)
            .iter()
            .map(|s| rom.bytes(s.region, s.offset, s.len))
            .collect()
    }

    fn write_blob(&self, out: &mut String, bytes: &[u8]) {
        let (head, width) = (self.head as usize, self.width as usize);
        let fits = bytes.len() >= head;
        if fits && head > 0 {
            let _ = writeln!(out, "head = {}", text::hex_list(&bytes[..head]));
        }
        let rest = if fits && head > 0 {
            &bytes[head..]
        } else {
            bytes
        };
        if width > 0 && fits && rest.len() % width == 0 {
            if rest.is_empty() {
                out.push_str("rows = []\n");
                return;
            }
            out.push_str("rows = [\n");
            for row in rest.chunks_exact(width) {
                let _ = writeln!(out, "  {},", text::hex_list(row));
            }
            out.push_str("]\n");
        } else if rest.len() <= 16 {
            let _ = writeln!(out, "bytes = {}", text::hex_list(rest));
        } else {
            out.push_str("bytes = [\n");
            for line in rest.chunks(16) {
                let items: Vec<String> = line.iter().map(|b| format!("0x{b:02X}")).collect();
                let _ = writeln!(out, "  {},", items.join(", "));
            }
            out.push_str("]\n");
        }
    }

    /// One blob's bytes from its entry in the file.
    fn read_blob(&self, r: &Reader, t: &toml::Table, i: usize) -> Result<Vec<u8>, String> {
        let at = format!("blob {i}: ");
        r.only(t, &["address", "head", "rows", "bytes"], &at)?;
        let mut out = Vec::new();
        let list = |key: &str| -> Result<Option<Vec<u8>>, String> {
            match t.get(key) {
                None => Ok(None),
                Some(v @ Value::Array(a)) => r.bytes(v, a.len(), &format!("{at}{key} ")).map(Some),
                Some(_) => Err(format!("{at}{key} must be a list of numbers")),
            }
        };
        if let Some(h) = list("head")? {
            out.extend(h);
        }
        if let Some(rows) = t.get("rows") {
            if self.width == 0 {
                return Err(format!(
                    "{at}has rows, but assets.toml gives no width; write bytes"
                ));
            }
            let Value::Array(rows) = rows else {
                return Err(format!("{at}rows must be a list of rows"));
            };
            for (j, row) in rows.iter().enumerate() {
                out.extend(r.bytes(row, self.width as usize, &format!("{at}row {j} "))?);
            }
        }
        if let Some(b) = list("bytes")? {
            out.extend(b);
        }
        if out.is_empty() {
            return Err(format!("{at}holds no bytes"));
        }
        Ok(out)
    }
}

impl Module for Blobs {
    fn name(&self) -> &str {
        &self.name
    }

    fn format(&self) -> &'static str {
        "blobs"
    }

    fn validate(&self) -> Result<(), String> {
        let name = &self.name;
        if self.region != Region::Prg {
            return Err(format!(
                "{name}: blobs and their pointers are read from prg"
            ));
        }
        if self.size == 0 {
            return Err(format!("{name}: size must be at least 1"));
        }
        format::check_span(name, self.offset, u64::from(self.size))?;
        if self.cpu_base > 0xFFFF || self.prg_base > crate::map::MAX_SPAN as u32 {
            return Err(format!(
                "{name}: cpu_base {:#x} is past the 6502's 64KB, or prg_base {:#x} past any ROM",
                self.cpu_base, self.prg_base
            ));
        }
        if self
            .address(self.offset)
            .is_none_or(|a| a + self.size - 1 > 0xFFFF)
        {
            return Err(format!(
                "{name}: the span isn't inside the CPU's 64KB at cpu_base {:#x} for prg_base {:#x}",
                self.cpu_base, self.prg_base
            ));
        }
        if self.head > 0xFF || self.width > 0xFF {
            return Err(format!("{name}: head and width must be under 256"));
        }
        if self.tables.is_empty() {
            return Err(format!("{name}: give at least one [[asset.table]]"));
        }
        for n in &self.notes {
            format::one_line(&format!("{name}: each note"), n)?;
        }
        format::check_labels(name, self.label.as_ref(), self.end_label.as_ref())?;
        let mut keys = std::collections::BTreeSet::new();
        for t in &self.tables {
            let what = format!("{name}.{}", t.name);
            if !format::safe_name(&t.name) || !keys.insert(t.name.as_str()) {
                return Err(format!(
                    "{what}: a table's name must be lowercase letters, digits, - and _, and not repeat"
                ));
            }
            if t.count == 0 || u64::from(t.count) * 2 > crate::map::MAX_SPAN {
                return Err(format!(
                    "{what}: count must be at least 1, and its table no bigger than any NES ROM"
                ));
            }
            if t.hi_offset.is_some() && t.base.is_some() {
                return Err(format!(
                    "{what}: a table is split (hi_offset) or one-byte offsets (base), not both"
                ));
            }
            if t.base.is_some_and(|b| b + 0xFF > 0xFFFF) {
                return Err(format!(
                    "{what}: base + 255 is past the 6502's 64KB, so an offset could point past it"
                ));
            }
            if t.hi_offset.is_none() && (t.hi_label.is_some() || t.hi_end_label.is_some()) {
                return Err(format!("{what}: hi_label without hi_offset"));
            }
            for s in t.spans(self.region, name) {
                format::check_span(&what, s.offset, u64::from(s.len))?;
            }
            format::check_labels(&what, t.label.as_ref(), t.end_label.as_ref())?;
            format::check_labels(&what, t.hi_label.as_ref(), t.hi_end_label.as_ref())?;
        }
        Ok(())
    }

    fn spans(&self) -> Vec<Span> {
        let mut out = vec![format::span(
            self.region,
            self.offset,
            self.size,
            self.name.clone(),
            self.label.clone(),
        )];
        for t in &self.tables {
            out.extend(t.spans(self.region, &self.name));
        }
        out
    }

    fn lift(&self, source: &str, _map: &AssetMap, rom: &Rom) -> Result<Vec<File>, String> {
        let data = rom.bytes(self.region, self.offset, self.size)?;
        let mut values = Vec::new();
        for t in &self.tables {
            values.push(t.values(&self.table_bytes(t, rom)?));
        }
        // Cut the span where any pointer points.
        let mut cuts: Vec<u32> = std::iter::once(self.offset)
            .chain(values.iter().flatten().filter_map(|&v| self.target(v)))
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let blob_at = |prg: u32| cuts.binary_search(&prg).expect("every target is a cut");
        let mut refs: Vec<Vec<String>> = vec![Vec::new(); cuts.len()];
        for (t, vals) in self.tables.iter().zip(&values) {
            for (i, &v) in vals.iter().enumerate() {
                if let Some(prg) = self.target(v) {
                    refs[blob_at(prg)].push(format!("{} {i:#04x}", t.name));
                }
            }
        }

        let mut out = String::new();
        text::header(
            &mut out,
            "Blobs and the pointers into them",
            source,
            &self.name,
        );
        if let Some(l) = &self.label {
            let _ = writeln!(out, "# Label: {l}");
        }
        out.push_str(
            "# Each pointer is a blob's number, or an address in quotes for one that\n\
             # points outside the blobs. A blob's bytes are its head, rows and bytes in turn.\n",
        );
        if self.fixed {
            out.push_str(
                "# The blobs hold addresses of their own, so none may move: each keeps its address.\n",
            );
        }
        out.push_str("\n[pointers]\n");
        for (t, vals) in self.tables.iter().zip(&values) {
            let _ = writeln!(out, "{} = [", t.name);
            for (row, chunk) in vals.chunks(8).enumerate() {
                let items: Vec<String> = chunk
                    .iter()
                    .map(|&v| match self.target(v) {
                        Some(prg) => blob_at(prg).to_string(),
                        None => format!("\"0x{v:04X}\""),
                    })
                    .collect();
                let _ = writeln!(out, "  {}, # {:#04x}", items.join(", "), row * 8);
            }
            out.push_str("]\n");
        }
        for (i, &start) in cuts.iter().enumerate() {
            let end = cuts.get(i + 1).copied().unwrap_or(self.offset + self.size);
            let bytes = &data[(start - self.offset) as usize..(end - self.offset) as usize];
            let what = match self.notes.get(i) {
                Some(n) => format!(": {n}"),
                None => String::new(),
            };
            let pointed = if refs[i].is_empty() {
                "no pointer".to_string()
            } else {
                refs[i].join(", ")
            };
            let address = self.address(start).expect("checked in validate");
            let _ = writeln!(
                out,
                "\n# {i}{what} ({pointed}), {} bytes at 0x{address:04X}\n[[blob]]",
                bytes.len()
            );
            if self.fixed {
                let _ = writeln!(out, "address = 0x{address:04X}");
            }
            self.write_blob(&mut out, bytes);
        }
        Ok(vec![File {
            name: self.file(),
            bytes: out.into_bytes(),
        }])
    }

    fn encode(&self, _map: &AssetMap, read: ReadFile) -> Result<Vec<Vec<u8>>, Error> {
        let file = self.file();
        let fail = |msg: String| Error::File {
            file: file.clone(),
            msg,
        };
        let text = format::read_text(read, &file)?;
        let r = Reader::new(&text, &self.name, &["pointers", "blob"]).map_err(fail)?;
        let blobs = r.array(r.top(), "blob", "").map_err(fail)?;
        let mut data = Vec::new();
        let mut starts = Vec::with_capacity(blobs.len());
        for (i, b) in blobs.iter().enumerate() {
            let Value::Table(t) = b else {
                return Err(fail(format!("blob {i} must be a table")));
            };
            let prg = self.offset + data.len() as u32;
            let address = self.address(prg).expect("checked in validate");
            match (self.fixed, t.get("address")) {
                (true, Some(Value::Integer(a))) if *a == i64::from(address) => {}
                (true, Some(Value::Integer(a))) => {
                    return Err(fail(format!(
                        "blob {i} would be at {address:#06x}, but its address is {a:#06x}: \
                         these blobs hold addresses of their own, so none may change length"
                    )));
                }
                (true, _) => return Err(fail(format!("blob {i}: address is missing"))),
                (false, Some(_)) => {
                    return Err(fail(format!(
                        "blob {i}: address is only for blobs that can't move (fixed in assets.toml)"
                    )));
                }
                (false, None) => {}
            }
            let bytes = self.read_blob(&r, t, i).map_err(fail)?;
            if data.len() + bytes.len() > self.size as usize {
                return Err(fail(format!(
                    "the blobs take more than the {} bytes assets.toml gives them",
                    self.size
                )));
            }
            starts.push(address);
            data.extend(bytes);
        }
        if starts.is_empty() {
            return Err(fail("there are no blobs".into()));
        }
        match self.fill {
            Some(f) => data.resize(self.size as usize, f),
            None if data.len() < self.size as usize => {
                return Err(fail(format!(
                    "the blobs take {} bytes; assets.toml gives them {} and no fill",
                    data.len(),
                    self.size
                )));
            }
            None => {}
        }

        let ptrs = match r.top().get("pointers") {
            Some(Value::Table(p)) => p,
            _ => return Err(fail("pointers must be a table of lists".into())),
        };
        let names: Vec<&str> = self.tables.iter().map(|t| t.name.as_str()).collect();
        r.only(ptrs, &names, "pointers: ").map_err(fail)?;
        let mut out = vec![data];
        for t in &self.tables {
            let list = r.array(ptrs, &t.name, "pointers: ").map_err(fail)?;
            if list.len() != t.count as usize {
                return Err(fail(format!(
                    "pointers.{}: {} pointers, but assets.toml says {}",
                    t.name,
                    list.len(),
                    t.count
                )));
            }
            let mut values = Vec::with_capacity(list.len());
            for (i, v) in list.iter().enumerate() {
                let at = format!("pointers.{} {i}: ", t.name);
                let p = match v {
                    Value::Integer(n) if (0..starts.len() as i64).contains(n) => {
                        Pointer::Blob(*n as usize)
                    }
                    Value::Integer(n) => {
                        return Err(fail(format!(
                            "{at}there's no blob {n}; there are {}",
                            starts.len()
                        )));
                    }
                    Value::String(s) => match parse_address(s) {
                        Some(a) if self.target(a).is_none() => Pointer::Raw(a),
                        Some(_) => {
                            return Err(fail(format!(
                                "{at}{s} points into the blobs; give the blob's number"
                            )));
                        }
                        None => {
                            return Err(fail(format!(
                                "{at}{s:?} isn't an address such as \"0xFFFF\""
                            )));
                        }
                    },
                    _ => {
                        return Err(fail(format!(
                            "{at}must be a blob's number or an address in quotes"
                        )));
                    }
                };
                let value = match p {
                    Pointer::Blob(b) => starts[b],
                    Pointer::Raw(a) => a,
                };
                values.push(value);
            }
            match (t.hi_offset, t.base) {
                (Some(_), _) => {
                    out.push(values.iter().map(|&v| v as u8).collect());
                    out.push(values.iter().map(|&v| (v >> 8) as u8).collect());
                }
                (None, Some(base)) => {
                    let mut b = Vec::with_capacity(values.len());
                    for (i, &v) in values.iter().enumerate() {
                        match v.checked_sub(base).filter(|d| *d <= 0xFF) {
                            Some(d) => b.push(d as u8),
                            None => {
                                return Err(fail(format!(
                                    "pointers.{} {i}: {v:#06x} is more than 255 bytes past the table's base {base:#06x}",
                                    t.name
                                )));
                            }
                        }
                    }
                    out.push(b);
                }
                (None, None) => out.push(
                    values
                        .iter()
                        .flat_map(|&v| [v as u8, (v >> 8) as u8])
                        .collect(),
                ),
            }
        }
        Ok(out)
    }
}

/// "0x1234" as a 16-bit address.
fn parse_address(s: &str) -> Option<u32> {
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    u32::from_str_radix(hex, 16).ok().filter(|a| *a <= 0xFFFF)
}
