//! The `ppu-strings` format: PPU update strings, the buffers NES games
//! copy to the PPU during vblank. Each update is a two-byte PPU address
//! (high byte first), a control byte and its data: bit 7 of the control
//! byte steps the address by 32 after each byte (down a column) instead of
//! by 1, bit 6 writes one byte over and over, and the low six bits are the
//! byte count. A string is updates up to an end byte. Super Mario Bros.
//! keeps its status bar and message text this way (`WriteBufferToScreen`
//! and `WriteGameText` in the disassembly), and its palettes.
//!
//! The lifted file has one `[[string]]` per entry of the map, each a list
//! of updates. An update's bytes are text when the map gives a charset and
//! every byte has a character, a list of tile numbers otherwise, or the
//! name of another asset whose bytes are the data (an embed, as for a
//! palette asset inside its update string).

use std::fmt::Write as _;

use serde::Deserialize;
use toml::{Table, Value};

use crate::charset::{Charset, quoted};
use crate::format::{self, Module, ReadFile, Rom};
use crate::map::{AssetMap, MAX_SPAN, Region, Span};
use crate::text::{self, Reader};
use crate::{Error, File};

/// The most data bytes one update holds: six bits of count.
const MAX_LEN: u32 = 0x3F;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuStrings {
    pub name: String,
    pub label: Option<String>,
    pub end_label: Option<String>,
    pub region: Region,
    pub offset: u32,
    /// The bytes the strings take: they must fill it exactly, since the
    /// game finds them at fixed addresses.
    pub size: u32,
    /// The byte that ends a string where the next update's address would
    /// start (default 0x00).
    #[serde(default)]
    pub end: u8,
    /// The game ends a string at the end byte wherever it falls, not only
    /// where an update's address would start (Super Mario Bros.'
    /// `WriteGameText`), so `pack` refuses it in any byte.
    #[serde(default)]
    pub end_anywhere: bool,
    pub charset: Option<Charset>,
    #[serde(rename = "entry")]
    pub entries: Vec<StringEntry>,
    /// Other assets whose bytes are an update's data inside this span.
    #[serde(default, rename = "embed")]
    pub embeds: Vec<Embed>,
}

/// One string. `records` is set for a string that has that many updates
/// and no end byte, running on into the next string, as Super Mario Bros.'
/// "MARIO" line runs into "TIME UP".
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StringEntry {
    pub name: String,
    pub label: Option<String>,
    pub note: Option<String>,
    pub records: Option<u32>,
}

/// An asset whose one span is an update's data, here as in the asset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Embed {
    pub asset: String,
    pub offset: u32,
    pub size: u32,
}

/// One update as stored, its data read from the span or named.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Data {
    Bytes(Vec<u8>),
    Repeat { count: u8, tile: u8 },
    Embed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Update {
    ppu: u16,
    vertical: bool,
    data: Data,
}

/// One write a string makes to the PPU: its bytes from address `ppu`,
/// stepping down a column of the name table when `vertical`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    pub ppu: u16,
    pub vertical: bool,
    pub bytes: Vec<u8>,
}

impl PpuStrings {
    /// String `name`'s writes, from the lifted file's text, for drawing it
    /// (a HUD's fixed tiles). An update whose data is an embedded asset
    /// is left out.
    pub fn writes(&self, text: &str, name: &str) -> Result<Vec<Write>, String> {
        let r = Reader::new(text, &self.name, &["string"])?;
        let strings = r.entries(r.top(), "string", self.entries.len())?;
        let s = strings
            .into_iter()
            .find(|s| matches!(s.get("name"), Some(Value::String(n)) if n == name))
            .ok_or_else(|| format!("{} has no string named {name}", self.name))?;
        let at = format!("string {name}: ");
        let mut out = Vec::new();
        for (i, v) in r.array(s, "updates", &at)?.iter().enumerate() {
            let u = self.read_update(&r, v, &format!("{at}update {i}: "))?;
            let bytes = match u.data {
                Data::Bytes(b) => b,
                Data::Repeat { count, tile } => vec![tile; usize::from(count)],
                Data::Embed(_) => continue,
            };
            out.push(Write {
                ppu: u.ppu,
                vertical: u.vertical,
                bytes,
            });
        }
        Ok(out)
    }

    fn embed_at(&self, rel: u32) -> Option<&Embed> {
        self.embeds.iter().find(|e| e.offset - self.offset == rel)
    }

    /// Splits the span's bytes into each entry's updates.
    fn decode(&self, bytes: &[u8]) -> Result<Vec<Vec<Update>>, String> {
        let size = bytes.len();
        let mut at = 0usize;
        let mut out = Vec::new();
        let overlaps = |from: usize, to: usize| {
            self.embeds.iter().find(|e| {
                let s = (e.offset - self.offset) as usize;
                from < s + e.size as usize && s < to
            })
        };
        for entry in &self.entries {
            let mut updates = Vec::new();
            loop {
                if entry.records == Some(updates.len() as u32) {
                    break;
                }
                let past = |what: &str| {
                    format!(
                        "string {} runs past the end of the asset's {size} bytes {what}",
                        entry.name
                    )
                };
                if at >= size {
                    return Err(past("without its end byte"));
                }
                if entry.records.is_none() && bytes[at] == self.end {
                    if let Some(e) = overlaps(at, at + 1) {
                        return Err(format!(
                            "string {}: its end byte at offset {:#06x} is inside {}",
                            entry.name,
                            self.offset as usize + at,
                            e.asset
                        ));
                    }
                    at += 1;
                    break;
                }
                if at + 3 > size {
                    return Err(past("in an update's address"));
                }
                if let Some(e) = overlaps(at, at + 3) {
                    return Err(format!(
                        "string {}: an update's address at offset {:#06x} overlaps {}",
                        entry.name,
                        self.offset as usize + at,
                        e.asset
                    ));
                }
                let ppu = u16::from(bytes[at]) << 8 | u16::from(bytes[at + 1]);
                let ctrl = bytes[at + 2];
                at += 3;
                let count = ctrl & 0x3F;
                let repeat = ctrl & 0x40 != 0;
                let len = if repeat { 1 } else { count as usize };
                if at + len > size {
                    return Err(past("in an update's data"));
                }
                let data = match (repeat, self.embed_at(at as u32)) {
                    (false, Some(e)) if e.size as usize == len => Data::Embed(e.asset.clone()),
                    _ => {
                        if let Some(e) = overlaps(at, at + len) {
                            return Err(format!(
                                "string {}: the update at offset {:#06x} holds part of {}, not all of it",
                                entry.name,
                                self.offset as usize + at - 3,
                                e.asset
                            ));
                        }
                        if repeat {
                            Data::Repeat {
                                count,
                                tile: bytes[at],
                            }
                        } else {
                            Data::Bytes(bytes[at..at + len].to_vec())
                        }
                    }
                };
                at += len;
                updates.push(Update {
                    ppu,
                    vertical: ctrl & 0x80 != 0,
                    data,
                });
            }
            out.push(updates);
        }
        if at != size {
            return Err(format!(
                "the {} strings end at offset {:#06x}, {} bytes before the asset does; is size right?",
                self.entries.len(),
                self.offset as usize + at,
                size - at
            ));
        }
        Ok(out)
    }

    fn write_update(&self, u: &Update) -> String {
        let mut fields = vec![format!("ppu = 0x{:04X}", u.ppu)];
        if u.vertical {
            fields.push("vertical = true".into());
        }
        match &u.data {
            Data::Bytes(b) => match self.charset.as_ref().and_then(|c| c.decode(b)) {
                Some(t) => fields.push(format!("text = {}", quoted(&t))),
                None => fields.push(format!("tiles = {}", text::hex_list(b))),
            },
            Data::Repeat { count, tile } => {
                fields.push(format!("repeat = {count}"));
                fields.push(format!("tile = 0x{tile:02X}"));
            }
            Data::Embed(a) => fields.push(format!("data = \"{a}\"")),
        }
        format!("{{ {} }}", fields.join(", "))
    }

    fn read_update(&self, r: &Reader, v: &Value, at: &str) -> Result<Update, String> {
        let Value::Table(t) = v else {
            return Err(format!("{at}must be a table"));
        };
        let t: &Table = t;
        r.only(
            t,
            &["ppu", "vertical", "text", "tiles", "repeat", "tile", "data"],
            at,
        )?;
        let ppu = r.int(t, "ppu", at, 0xFFFF)? as u16;
        if (ppu >> 8) as u8 == self.end {
            return Err(format!(
                "{at}ppu = {ppu:#06x} starts with the end byte {:#04x}, so the game would read it as the string's end",
                self.end
            ));
        }
        let vertical = match t.get("vertical") {
            None => false,
            Some(Value::Boolean(b)) => *b,
            Some(_) => return Err(format!("{at}vertical must be true or false")),
        };
        let has = |k: &str| t.contains_key(k);
        let kinds = ["text", "tiles", "repeat", "data"]
            .iter()
            .filter(|k| has(k))
            .count();
        if kinds != 1 || (has("tile") != has("repeat")) {
            return Err(format!(
                "{at}needs one of text, tiles, data, or repeat with tile"
            ));
        }
        let data = if let Some(v) = t.get("text") {
            let (Value::String(s), Some(c)) = (v, &self.charset) else {
                return Err(format!(
                    "{at}text must be a string, and needs a charset in assets.toml"
                ));
            };
            Data::Bytes(c.encode(s).map_err(|e| format!("{at}{e}"))?)
        } else if let Some(v) = t.get("tiles") {
            let Value::Array(items) = v else {
                return Err(format!("{at}tiles must be a list of numbers"));
            };
            Data::Bytes(r.bytes(v, items.len(), &format!("{at}tiles "))?)
        } else if has("repeat") {
            Data::Repeat {
                count: r.int(t, "repeat", at, i64::from(MAX_LEN))? as u8,
                tile: r.int(t, "tile", at, 0xFF)? as u8,
            }
        } else {
            let Some(Value::String(a)) = t.get("data") else {
                return Err(format!("{at}data must be an asset's name in quotes"));
            };
            if !self.embeds.iter().any(|e| &e.asset == a) {
                return Err(format!(
                    "{at}data = {a:?} isn't one of the assets this one embeds in assets.toml"
                ));
            }
            Data::Embed(a.clone())
        };
        if let Data::Bytes(b) = &data
            && b.len() as u32 > MAX_LEN
        {
            return Err(format!(
                "{at}holds {} bytes; an update holds at most {MAX_LEN}",
                b.len()
            ));
        }
        Ok(Update {
            ppu,
            vertical,
            data,
        })
    }
}

impl Module for PpuStrings {
    fn name(&self) -> &str {
        &self.name
    }

    fn format(&self) -> &'static str {
        "ppu-strings"
    }

    fn validate(&self) -> Result<(), String> {
        let name = &self.name;
        if self.size == 0 || u64::from(self.size) > MAX_SPAN {
            return Err(format!("{name}: size must be 1 to {MAX_SPAN} bytes"));
        }
        format::check_span(name, self.offset, u64::from(self.size))?;
        if self.entries.is_empty() {
            return Err(format!("{name}: ppu strings need an entry"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for e in &self.entries {
            if !format::safe_name(&e.name) || !seen.insert(e.name.as_str()) {
                return Err(format!(
                    "{name}: entry name {:?} must be lowercase letters, digits, - and _, and given once",
                    e.name
                ));
            }
            format::check_labels(&format!("{name}.{}", e.name), e.label.as_ref(), None)?;
            if let Some(n) = &e.note {
                format::one_line(&format!("{name}.{}: note", e.name), n)?;
            }
            if e.records == Some(0) {
                return Err(format!("{name}.{}: records must be at least 1", e.name));
            }
        }
        if let Some(c) = &self.charset {
            c.validate(name)?;
        }
        let end = u64::from(self.offset) + u64::from(self.size);
        let mut at = u64::from(self.offset);
        for e in &self.embeds {
            let (s, len) = (u64::from(e.offset), u64::from(e.size));
            if s < at || s + len > end || len == 0 || e.size > MAX_LEN {
                return Err(format!(
                    "{name}: embed {} must be 1 to {MAX_LEN} bytes inside the span, in order and not overlapping",
                    e.asset
                ));
            }
            at = s + len;
        }
        format::check_labels(name, self.label.as_ref(), self.end_label.as_ref())
    }

    fn check_refs(&self, map: &AssetMap) -> Result<(), String> {
        for e in &self.embeds {
            let spans = map
                .asset(&e.asset)
                .filter(|a| a.name() != self.name)
                .map(|a| a.spans());
            let ok = spans.is_some_and(|s| {
                s.len() == 1
                    && s[0].region == self.region
                    && s[0].offset == e.offset
                    && s[0].len == e.size
            });
            if !ok {
                return Err(format!(
                    "{}: embed {} must name another asset whose one span is {} offset {:#06x}, {} bytes",
                    self.name,
                    e.asset,
                    self.region.name(),
                    e.offset,
                    e.size
                ));
            }
        }
        Ok(())
    }

    fn spans(&self) -> Vec<Span> {
        let mut out = Vec::new();
        let mut at = self.offset;
        let gaps = self
            .embeds
            .iter()
            .map(|e| (e.offset, e.offset + e.size))
            .chain([(self.offset + self.size, self.offset + self.size)]);
        for (from, to) in gaps {
            if from > at {
                out.push((at, from - at));
            }
            at = to;
        }
        let one = out.len() == 1;
        out.into_iter()
            .enumerate()
            .map(|(i, (o, len))| {
                let part = if one {
                    self.name.clone()
                } else {
                    format!("{} (part {})", self.name, i + 1)
                };
                let label = (o == self.offset).then(|| self.label.clone()).flatten();
                format::span(self.region, o, len, part, label)
            })
            .collect()
    }

    fn lift(&self, source: &str, _map: &AssetMap, rom: &Rom) -> Result<Vec<File>, String> {
        let bytes = rom.bytes(self.region, self.offset, self.size)?;
        let strings = self.decode(bytes)?;
        let mut out = String::new();
        text::header(&mut out, "PPU update strings", source, &self.name);
        let _ = writeln!(
            out,
            "# Each update writes its data to the PPU from address ppu, stepping down a\n\
             # column if vertical is set: text in the map's charset, tiles as numbers,\n\
             # repeat (a count) times one tile, or data, another asset's bytes. A\n\
             # string ends with the byte {:#04x}, except one with a set number of\n\
             # updates in assets.toml, which runs on into the next. The strings must\n\
             # take exactly the {} bytes they take in the ROM.",
            self.end, self.size
        );
        for (entry, updates) in self.entries.iter().zip(&strings) {
            out.push('\n');
            if let Some(l) = &entry.label {
                let _ = writeln!(out, "# Label: {l}");
            }
            if let Some(n) = &entry.note {
                let _ = writeln!(out, "# {n}");
            }
            if let Some(n) = entry.records {
                let updates = if n == 1 { "update" } else { "updates" };
                let _ = writeln!(
                    out,
                    "# {n} {updates} and no end byte: the game runs on into the next string."
                );
            }
            let _ = writeln!(out, "[[string]]\nname = \"{}\"\nupdates = [", entry.name);
            for u in updates {
                let _ = writeln!(out, "  {},", self.write_update(u));
            }
            out.push_str("]\n");
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
        let r = Reader::new(&text, &self.name, &["string"]).map_err(fail)?;
        let strings = r
            .entries(r.top(), "string", self.entries.len())
            .map_err(fail)?;
        // The whole span, embeds included as placeholders, then cut into
        // the spans around them.
        let mut bytes: Vec<u8> = Vec::with_capacity(self.size as usize);
        // Which embeds an update has placed, so each is placed once.
        let mut placed = vec![false; self.embeds.len()];
        for (entry, s) in self.entries.iter().zip(strings) {
            let at = format!("string {}: ", entry.name);
            r.only(s, &["name", "updates"], &at).map_err(fail)?;
            match s.get("name") {
                Some(Value::String(n)) if *n == entry.name => {}
                _ => {
                    return Err(fail(format!(
                        "{at}name must be \"{}\", as in assets.toml",
                        entry.name
                    )));
                }
            }
            let updates = r.array(s, "updates", &at).map_err(fail)?;
            if let Some(n) = entry.records
                && updates.len() != n as usize
            {
                return Err(fail(format!(
                    "{at}{} updates, but assets.toml says it has {n} and runs on into the next string",
                    updates.len()
                )));
            }
            for (i, v) in updates.iter().enumerate() {
                let u = self
                    .read_update(&r, v, &format!("{at}update {i}: "))
                    .map_err(fail)?;
                let ctrl = |count: u32, repeat: bool| {
                    (u.vertical as u8) << 7 | (repeat as u8) << 6 | count as u8
                };
                let start = bytes.len();
                bytes.extend([(u.ppu >> 8) as u8, u.ppu as u8]);
                match &u.data {
                    Data::Bytes(b) => {
                        bytes.push(ctrl(b.len() as u32, false));
                        bytes.extend(b);
                    }
                    Data::Repeat { count, tile } => {
                        bytes.push(ctrl(u32::from(*count), true));
                        bytes.push(*tile);
                    }
                    Data::Embed(a) => {
                        let k = self
                            .embeds
                            .iter()
                            .position(|e| &e.asset == a)
                            .expect("read_update checked it");
                        if std::mem::replace(&mut placed[k], true) {
                            return Err(fail(format!(
                                "{at}update {i}: {a} is placed twice; each embed is one update's data"
                            )));
                        }
                        let e = &self.embeds[k];
                        bytes.push(ctrl(e.size, false));
                        let want = (e.offset - self.offset) as usize;
                        if bytes.len() != want {
                            return Err(fail(format!(
                                "{at}update {i}: {a} is at {} offset {:#06x}, but the updates before it put its data at {:#06x}",
                                self.region.name(),
                                e.offset,
                                self.offset as usize + bytes.len()
                            )));
                        }
                        bytes.resize(want + e.size as usize, 0);
                    }
                }
                // An embed's own bytes are its asset's, checked there.
                let own = match &u.data {
                    Data::Embed(_) => &bytes[start..start + 3],
                    _ => &bytes[start..],
                };
                if self.end_anywhere && own.contains(&self.end) {
                    return Err(fail(format!(
                        "{at}update {i} holds the end byte {:#04x}, where the game would end the string",
                        self.end
                    )));
                }
            }
            if entry.records.is_none() {
                bytes.push(self.end);
            }
        }
        if let Some(k) = placed.iter().position(|p| !p) {
            return Err(fail(format!(
                "no update has data = \"{}\"; each embed must be one update's data",
                self.embeds[k].asset
            )));
        }
        if bytes.len() != self.size as usize {
            return Err(fail(format!(
                "the strings take {} bytes, but they must take exactly the {} they have in the ROM (size in assets.toml), since the game finds them at fixed addresses",
                bytes.len(),
                self.size
            )));
        }
        Ok(self
            .spans()
            .iter()
            .map(|s| {
                let from = (s.offset - self.offset) as usize;
                bytes[from..from + s.len as usize].to_vec()
            })
            .collect())
    }
}
