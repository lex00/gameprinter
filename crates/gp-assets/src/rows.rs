//! The `byte-rows` format: a table stored row by row, `width` bytes to a
//! row, such as Super Mario Bros.' sprite tile tables (six tile numbers per
//! enemy frame) or a sound effect's frequencies. The lifted file has one
//! list per row, or a string when the map gives a charset and every byte
//! of the row has a character.

use std::fmt::Write as _;

use serde::Deserialize;
use toml::Value;

use crate::charset::{Charset, quoted};
use crate::format::{self, Module, ReadFile, Rom};
use crate::map::{AssetMap, MAX_SPAN, Region, Span};
use crate::text::{self, Reader};
use crate::{Error, File};

/// Numbers per line in a row longer than this.
const WRAP: usize = 16;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ByteRows {
    pub name: String,
    pub label: Option<String>,
    pub end_label: Option<String>,
    pub region: Region,
    pub offset: u32,
    pub count: u32,
    pub width: u32,
    /// One name per row, for the lifted file's comments.
    #[serde(default)]
    pub names: Vec<String>,
    pub charset: Option<Charset>,
}

impl Module for ByteRows {
    fn name(&self) -> &str {
        &self.name
    }

    fn format(&self) -> &'static str {
        "byte-rows"
    }

    fn validate(&self) -> Result<(), String> {
        let name = &self.name;
        if self.count == 0 || self.width == 0 {
            return Err(format!("{name}: count and width must be at least 1"));
        }
        if u64::from(self.count) * u64::from(self.width) > MAX_SPAN {
            return Err(format!(
                "{name}: {} rows of {} bytes is more than any NES ROM holds",
                self.count, self.width
            ));
        }
        format::check_span(
            name,
            self.offset,
            u64::from(self.count) * u64::from(self.width),
        )?;
        if !self.names.is_empty() && self.names.len() != self.count as usize {
            return Err(format!(
                "{name}: {} names for {} rows",
                self.names.len(),
                self.count
            ));
        }
        for n in &self.names {
            format::one_line(&format!("{name}: each name"), n)?;
        }
        if let Some(c) = &self.charset {
            c.validate(name)?;
        }
        format::check_labels(name, self.label.as_ref(), self.end_label.as_ref())
    }

    fn spans(&self) -> Vec<Span> {
        vec![format::span(
            self.region,
            self.offset,
            self.count * self.width,
            self.name.clone(),
            self.label.clone(),
        )]
    }

    fn lift(&self, source: &str, _map: &AssetMap, rom: &Rom) -> Result<Vec<File>, String> {
        let bytes = rom.bytes(self.region, self.offset, self.count * self.width)?;
        let mut out = String::new();
        text::header(
            &mut out,
            &format!("Rows of {} bytes each", self.width),
            source,
            &self.name,
        );
        if let Some(l) = &self.label {
            let _ = writeln!(out, "# Label: {l}");
        }
        if self.charset.is_some() {
            out.push_str("# A row is text in the map's charset, or a list of numbers.\n");
        }
        out.push_str("rows = [\n");
        for (i, row) in bytes.chunks_exact(self.width as usize).enumerate() {
            let what = match self.names.get(i) {
                Some(n) => format!("{i}: {n}"),
                None => i.to_string(),
            };
            match self.charset.as_ref().and_then(|c| c.decode(row)) {
                Some(t) => {
                    let _ = writeln!(out, "  {}, # {what}", quoted(&t));
                }
                // A long row of numbers is written 16 to a line.
                None if row.len() > WRAP => {
                    let _ = writeln!(out, "  [ # {what}");
                    for line in row.chunks(WRAP) {
                        let items: Vec<String> =
                            line.iter().map(|b| format!("0x{b:02X}")).collect();
                        let _ = writeln!(out, "    {},", items.join(", "));
                    }
                    out.push_str("  ],\n");
                }
                None => {
                    let _ = writeln!(out, "  {}, # {what}", text::hex_list(row));
                }
            }
        }
        out.push_str("]\n");
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
        let r = Reader::new(&text, &self.name, &["rows"]).map_err(fail)?;
        let rows = r.array(r.top(), "rows", "").map_err(fail)?;
        if rows.len() != self.count as usize {
            return Err(fail(format!(
                "{} rows, but assets.toml says {}",
                rows.len(),
                self.count
            )));
        }
        let width = self.width as usize;
        let mut bytes = Vec::with_capacity(rows.len() * width);
        for (i, row) in rows.iter().enumerate() {
            let at = format!("row {i} ");
            let b = match (row, &self.charset) {
                (Value::String(s), Some(c)) => {
                    let b = c.encode(s).map_err(|e| fail(format!("{at}{e}")))?;
                    if b.len() != width {
                        return Err(fail(format!("{at}has {} characters, not {width}", b.len())));
                    }
                    b
                }
                (Value::String(_), None) => {
                    return Err(fail(format!(
                        "{at}is text, but assets.toml gives no charset; write a list of numbers"
                    )));
                }
                _ => r.bytes(row, width, &at).map_err(fail)?,
            };
            bytes.extend(b);
        }
        Ok(vec![bytes])
    }
}
