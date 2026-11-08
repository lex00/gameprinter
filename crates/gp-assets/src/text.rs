//! The text formats: palettes, metatiles, levels and tables as TOML,
//! written by hand so their bytes are fixed and their layout reads like the
//! data (hex for tile and color numbers, one string per level row).
//! Reading is strict: every field the format needs, nothing else, and every
//! number in range, with errors naming the file and the entry.

use std::fmt::Write as _;

use toml::{Table, Value};

use crate::map::{Column, SCHEMA};

/// The lines every lifted text file starts with.
pub(crate) fn header(out: &mut String, what: &str, source: &str, asset: &str) {
    let _ = writeln!(
        out,
        "# {what}.\n\
         # Lifted from {source} by gp assets lift; gp assets pack writes it back (docs/assets.md)."
    );
    let _ = writeln!(out, "schema = {SCHEMA}\nasset = \"{asset}\"");
}

pub(crate) fn hex_list(bytes: &[u8]) -> String {
    let items: Vec<String> = bytes.iter().map(|b| format!("0x{b:02X}")).collect();
    format!("[{}]", items.join(", "))
}

pub fn write_palettes(source: &str, asset: &str, bytes: &[u8]) -> String {
    let mut out = String::new();
    header(
        &mut out,
        "Palettes: four NES color numbers each",
        source,
        asset,
    );
    let count = bytes.len() / 4;
    out.push_str("palettes = [\n");
    for (i, p) in bytes.as_chunks::<4>().0.iter().enumerate() {
        // Eight palettes are the PPU's whole palette RAM: four for the
        // background, then four for sprites.
        let what = match (count, i) {
            (8, 0..4) => format!("background {i}"),
            (8, _) => format!("sprites {}", i - 4),
            _ => i.to_string(),
        };
        let _ = writeln!(out, "  {}, # {what}", hex_list(p));
    }
    out.push_str("]\n");
    out
}

pub fn write_metatiles(
    source: &str,
    asset: &str,
    tiles: &[u8],
    names: &[String],
    legend: Option<&str>,
    properties: &[(&Column, Vec<u32>)],
) -> String {
    let mut out = String::new();
    header(
        &mut out,
        "Metatiles: four tile numbers each (top left, top right, bottom left, bottom right)",
        source,
        asset,
    );
    for (i, t) in tiles.as_chunks::<4>().0.iter().enumerate() {
        let ch = legend
            .and_then(|l| l.chars().nth(i))
            .map(|c| format!(", level character \"{c}\""))
            .unwrap_or_default();
        let _ = writeln!(out, "\n# {i}: {}{ch}\n[[metatile]]", names[i]);
        let _ = writeln!(out, "tiles = {}", hex_list(t));
        for (col, values) in properties {
            let _ = writeln!(out, "{} = {}", col.name, values[i]);
        }
    }
    out
}

pub fn write_level(
    source: &str,
    asset: &str,
    width: u32,
    cells: &[u8],
    legend: &str,
    names: Option<&[String]>,
) -> String {
    let mut out = String::new();
    header(
        &mut out,
        "A level, one character per metatile",
        source,
        asset,
    );
    let height = cells.len() / width as usize;
    let _ = writeln!(
        out,
        "# {width} metatiles wide and {height} high. The characters, from the map's legend:"
    );
    for (i, c) in legend.chars().enumerate() {
        let name = names
            .and_then(|n| n.get(i))
            .map(|n| format!(" {n}"))
            .unwrap_or_default();
        let _ = writeln!(out, "#   {c}  {i}{name}");
    }
    out.push_str("rows = [\n");
    let chars: Vec<char> = legend.chars().collect();
    for (y, row) in cells.chunks_exact(width as usize).enumerate() {
        let text: String = row.iter().map(|&b| chars[b as usize]).collect();
        let _ = writeln!(out, "  \"{text}\", # {y}");
    }
    out.push_str("]\n");
    out
}

pub fn write_table(source: &str, asset: &str, columns: &[(&Column, Vec<u32>)]) -> String {
    let mut out = String::new();
    header(&mut out, "A table, one entry per row", source, asset);
    let rows = columns.first().map_or(0, |(_, v)| v.len());
    for i in 0..rows {
        let _ = writeln!(out, "\n# {i}\n[[row]]");
        for (col, values) in columns {
            let _ = writeln!(out, "{} = {}", col.name, values[i]);
        }
    }
    out
}

/// A pointer table: one name per pointer, with the address it held.
pub fn write_pointers(source: &str, asset: &str, names: &[(String, u32)]) -> String {
    let mut out = String::new();
    header(
        &mut out,
        "A pointer table, one name per pointer",
        source,
        asset,
    );
    let _ = writeln!(
        out,
        "# Each pointer names an asset, by where its data starts. gp assets pack writes its address.\n\
         # The comments give the index and the address the ROM held."
    );
    out.push_str("pointers = [\n");
    for (i, (name, address)) in names.iter().enumerate() {
        let _ = writeln!(out, "  \"{name}\", # {i}: {address:#06x}");
    }
    out.push_str("]\n");
    out
}

/// A lifted text file being read back.
pub struct Reader {
    table: Table,
}

impl Reader {
    /// Parses the file and checks its schema and asset name. `keys` are the
    /// top-level keys the format has, besides those two.
    pub fn new(text: &str, asset: &str, keys: &[&str]) -> Result<Self, String> {
        let table: Table = text.parse().map_err(|e| format!("{e}"))?;
        let r = Self { table };
        let schema = r.int(&r.table, "schema", "", u32::MAX as i64)?;
        if schema != i64::from(SCHEMA) {
            return Err(format!(
                "schema {schema} isn't supported; expected {SCHEMA}"
            ));
        }
        match r.table.get("asset") {
            Some(Value::String(s)) if s == asset => {}
            _ => return Err(format!("asset must be \"{asset}\", as in assets.toml")),
        }
        r.only(&r.table, &[&["schema", "asset"], keys].concat(), "")?;
        Ok(r)
    }

    /// Fails on any key of `t` not in `keys`, which would otherwise be
    /// dropped without a word.
    pub fn only(&self, t: &Table, keys: &[&str], at: &str) -> Result<(), String> {
        for k in t.keys() {
            if !keys.contains(&k.as_str()) {
                return Err(format!("{at}unknown key {k:?}"));
            }
        }
        Ok(())
    }

    pub fn int(&self, t: &Table, key: &str, at: &str, max: i64) -> Result<i64, String> {
        match t.get(key) {
            Some(Value::Integer(n)) if (0..=max).contains(n) => Ok(*n),
            Some(Value::Integer(n)) => Err(format!("{at}{key} = {n} is outside 0 to {max}")),
            Some(_) => Err(format!("{at}{key} must be a whole number")),
            None => Err(format!("{at}{key} is missing")),
        }
    }

    pub fn array<'t>(&self, t: &'t Table, key: &str, at: &str) -> Result<&'t [Value], String> {
        match t.get(key) {
            Some(Value::Array(a)) => Ok(a),
            Some(_) => Err(format!("{at}{key} must be a list")),
            None => Err(format!("{at}{key} is missing")),
        }
    }

    pub fn top(&self) -> &Table {
        &self.table
    }

    /// A list of `len` bytes.
    pub fn bytes(&self, v: &Value, len: usize, at: &str) -> Result<Vec<u8>, String> {
        let Value::Array(items) = v else {
            return Err(format!("{at}must be a list of {len} numbers"));
        };
        if items.len() != len {
            return Err(format!("{at}has {} numbers, not {len}", items.len()));
        }
        items
            .iter()
            .map(|i| match i {
                Value::Integer(n) if (0..=255).contains(n) => Ok(*n as u8),
                _ => Err(format!("{at}holds {i}, which isn't a byte (0 to 255)")),
            })
            .collect()
    }

    /// The `[[name]]` entries, exactly `count` of them.
    pub fn entries<'t>(
        &self,
        t: &'t Table,
        key: &str,
        count: usize,
    ) -> Result<Vec<&'t Table>, String> {
        let items = self.array(t, key, "")?;
        if items.len() != count {
            return Err(format!(
                "{} [[{key}]] entries, but assets.toml says {count}",
                items.len()
            ));
        }
        items
            .iter()
            .enumerate()
            .map(|(i, v)| match v {
                Value::Table(t) => Ok(t),
                _ => Err(format!("{key} {i} must be a table")),
            })
            .collect()
    }
}

/// The largest value a column holds: a byte, or 16 bits when split.
pub fn column_max(c: &Column) -> i64 {
    if c.hi_offset.is_some() { 0xFFFF } else { 0xFF }
}
