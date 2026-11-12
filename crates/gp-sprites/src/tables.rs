//! Sprite frames from a game's sprite tables, read from the lifted files
//! the asset map wrote (docs/formats/sprites.md, "Tables"). Each shape is
//! general: rows of tile numbers, records of tile and attribute bytes
//! with a layout table of offsets, OAM records, and animation sequences.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::config::{Blob, Config, Table};

/// One sprite of a table frame, before its tile is matched to a lifted
/// tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPiece {
    /// The tile number: in `slot` when the table names one, else in the
    /// pattern table the PPU draws sprites from.
    pub tile: u32,
    pub slot: Option<String>,
    pub x: i32,
    pub y: i32,
    /// In the model's numbering: sprite palettes are 4 to 7.
    pub palette: u8,
    pub flip_x: bool,
    pub flip_y: bool,
}

/// A frame a table defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableFrame {
    pub table: String,
    pub index: u32,
    pub pieces: Vec<RawPiece>,
    /// Its places are from the entity's position, as the game draws it;
    /// otherwise they're a grid from the picture's top left.
    pub anchored: bool,
}

/// An animation a table defines: frame numbers in `frames` (a table's
/// name), each shown `every` frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence {
    pub table: String,
    pub id: u32,
    pub frames: String,
    pub steps: Vec<u32>,
    pub every: u32,
}

/// Everything the tables define, and which entity each part belongs to.
#[derive(Debug, Clone, Default)]
pub struct Tables {
    /// By (table, index).
    pub frames: BTreeMap<(String, u32), TableFrame>,
    pub sequences: Vec<Sequence>,
    /// Each entity's frames, as (table, index).
    pub of_entity: BTreeMap<String, BTreeSet<(String, u32)>>,
    /// Each entity's sequences, by index into `sequences`.
    pub sequences_of: BTreeMap<String, Vec<usize>>,
    /// Each entity's palette for its rows, from an index's attributes.
    pub palette_of: BTreeMap<String, u8>,
}

fn read(dir: &Path, asset: &str) -> Result<toml::Table, String> {
    let file = dir.join(format!("{asset}.toml"));
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))
}

fn ints(v: &toml::Value) -> Vec<i64> {
    v.as_array()
        .map(|a| a.iter().filter_map(toml::Value::as_integer).collect())
        .unwrap_or_default()
}

/// A lifted `byte-rows` file's rows.
pub fn byte_rows(dir: &Path, asset: &str) -> Result<Vec<Vec<i64>>, String> {
    let t = read(dir, asset)?;
    Ok(t.get("rows")
        .and_then(toml::Value::as_array)
        .map(|rows| rows.iter().map(ints).collect())
        .unwrap_or_default())
}

/// A column of a lifted `table` file.
pub fn table_column(dir: &Path, asset: &str, column: &str) -> Result<Vec<i64>, String> {
    let t = read(dir, asset)?;
    let rows = t
        .get("row")
        .and_then(toml::Value::as_array)
        .ok_or(format!("{asset}: no rows"))?;
    Ok(rows
        .iter()
        .map(|r| {
            r.get(column)
                .and_then(toml::Value::as_integer)
                .unwrap_or(-1)
        })
        .collect())
}

/// One blob: its head, rows and bytes.
pub type BlobData = (Vec<i64>, Vec<Vec<i64>>, Vec<i64>);

/// A lifted `blobs` file: its blobs (head, rows, bytes) and its pointer
/// tables (a blob's number, or None for an address outside the blobs).
pub struct Blobs {
    pub blobs: Vec<BlobData>,
    pub pointers: BTreeMap<String, Vec<Option<u32>>>,
}

pub fn blobs(dir: &Path, asset: &str) -> Result<Blobs, String> {
    let t = read(dir, asset)?;
    let mut out = Blobs {
        blobs: Vec::new(),
        pointers: BTreeMap::new(),
    };
    if let Some(p) = t.get("pointers").and_then(toml::Value::as_table) {
        for (k, v) in p {
            let list = v
                .as_array()
                .map(|a| a.iter().map(|x| x.as_integer().map(|n| n as u32)).collect())
                .unwrap_or_default();
            out.pointers.insert(k.clone(), list);
        }
    }
    for b in t
        .get("blob")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let head = b.get("head").map(ints).unwrap_or_default();
        let rows = b
            .get("rows")
            .and_then(toml::Value::as_array)
            .map(|r| r.iter().map(ints).collect())
            .unwrap_or_default();
        let bytes = b.get("bytes").map(ints).unwrap_or_default();
        out.blobs.push((head, rows, bytes));
    }
    Ok(out)
}

fn blob_of<'a>(b: &'a Blobs, r: &Blob, n: u32) -> Option<&'a BlobData> {
    let i = (*b.pointers.get(&r.pointers)?.get(n as usize)?)?;
    b.blobs.get(i as usize)
}

/// Palette and flips from a sprite attribute byte, the palette in the
/// model's numbering.
pub fn attribute(a: u8) -> (u8, bool, bool) {
    (4 + (a & 3), a & 0x40 != 0, a & 0x80 != 0)
}

impl Tables {
    /// Reads every table the config names from a lifted directory, and
    /// binds frames and sequences to entities by their IDs.
    pub fn read(
        dir: &Path,
        config: &Config,
        ids: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Tables, String> {
        let mut t = Tables::default();
        // Sequences name frames of other tables, so they come last.
        let mut order: Vec<&Table> = config.tables.iter().collect();
        order.sort_by_key(|t| matches!(t, Table::Sequences { .. }));
        for table in order {
            match table {
                Table::Rows {
                    name,
                    asset,
                    slot,
                    width,
                    palette,
                    index,
                } => {
                    let rows = byte_rows(dir, asset)?;
                    let w = (*width).max(1) as usize;
                    for (i, row) in rows.iter().enumerate() {
                        let pieces = row
                            .iter()
                            .enumerate()
                            .map(|(k, &tile)| RawPiece {
                                tile: tile as u32,
                                slot: Some(slot.clone()),
                                x: (k % w) as i32 * 8,
                                y: (k / w) as i32 * 8,
                                palette: *palette,
                                flip_x: false,
                                flip_y: false,
                            })
                            .collect();
                        t.frames.insert(
                            (name.clone(), i as u32),
                            TableFrame {
                                table: name.clone(),
                                index: i as u32,
                                pieces,
                                anchored: false,
                            },
                        );
                    }
                    if let Some(ix) = index {
                        let col = table_column(dir, &ix.asset, &ix.column)?;
                        let attrs = match &ix.attributes {
                            Some(c) => table_column(dir, &ix.asset, c)?,
                            None => Vec::new(),
                        };
                        for (entity, eids) in ids {
                            if let Some(&a) =
                                eids.first().and_then(|&id| attrs.get(usize::from(id)))
                                && a >= 0
                            {
                                t.palette_of.insert(entity.clone(), attribute(a as u8).0);
                            }
                        }
                        let d = i64::from(ix.divide.max(1));
                        let starts: BTreeSet<i64> =
                            col.iter().filter(|&&v| v >= 0).map(|v| v / d).collect();
                        for (entity, eids) in ids {
                            for &id in eids {
                                let Some(&v) = col.get(usize::from(id)) else {
                                    continue;
                                };
                                if v < 0 {
                                    continue;
                                }
                                let first = v / d;
                                let end = starts
                                    .range(first + 1..)
                                    .next()
                                    .copied()
                                    .unwrap_or(rows.len() as i64);
                                for r in first..end.min(rows.len() as i64) {
                                    t.of_entity
                                        .entry(entity.clone())
                                        .or_default()
                                        .insert((name.clone(), r as u32));
                                }
                            }
                        }
                    }
                }
                Table::Pieces {
                    name,
                    frames,
                    layouts,
                    count_at,
                    layout_at,
                } => {
                    let fb = blobs(dir, &frames.asset)?;
                    let lb = blobs(dir, &layouts.asset)?;
                    let n = fb.pointers.get(&frames.pointers).map_or(0, Vec::len);
                    for i in 0..n as u32 {
                        let Some((head, rows, _)) = blob_of(&fb, frames, i) else {
                            continue;
                        };
                        let count = head.get(*count_at).copied().unwrap_or(0) as usize;
                        let layout = head.get(*layout_at).copied().unwrap_or(0) as u32;
                        let Some((_, offsets, _)) = blob_of(&lb, layouts, layout) else {
                            continue;
                        };
                        let pieces = rows
                            .iter()
                            .take(count)
                            .zip(offsets)
                            .map(|(r, o)| {
                                let (palette, fx, fy) =
                                    attribute(r.get(1).copied().unwrap_or(0) as u8);
                                RawPiece {
                                    tile: r.first().copied().unwrap_or(0) as u32,
                                    slot: None,
                                    x: i32::from(o.get(1).copied().unwrap_or(0) as u8 as i8),
                                    y: i32::from(o.first().copied().unwrap_or(0) as u8 as i8),
                                    palette,
                                    flip_x: fx,
                                    flip_y: fy,
                                }
                            })
                            .collect();
                        t.frames.insert(
                            (name.clone(), i),
                            TableFrame {
                                table: name.clone(),
                                index: i,
                                pieces,
                                anchored: true,
                            },
                        );
                    }
                }
                Table::Oam { name, frames } => {
                    let fb = blobs(dir, &frames.asset)?;
                    let n = fb.pointers.get(&frames.pointers).map_or(0, Vec::len);
                    for i in 0..n as u32 {
                        let Some((head, _, bytes)) = blob_of(&fb, frames, i) else {
                            continue;
                        };
                        let count = head.first().copied().unwrap_or(0) as usize;
                        let pieces = bytes
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .take(count)
                            .map(|r| {
                                let (palette, fx, fy) = attribute(r[2] as u8);
                                RawPiece {
                                    tile: r[1] as u32,
                                    slot: None,
                                    x: i32::from(r[3] as u8 as i8),
                                    y: i32::from(r[0] as u8 as i8),
                                    palette,
                                    flip_x: fx,
                                    flip_y: fy,
                                }
                            })
                            .collect();
                        t.frames.insert(
                            (name.clone(), i),
                            TableFrame {
                                table: name.clone(),
                                index: i,
                                pieces,
                                anchored: true,
                            },
                        );
                    }
                }
                Table::Sequences {
                    name,
                    asset,
                    frames,
                    last_at,
                    every_at,
                } => {
                    let sb = blobs(dir, &asset.asset)?;
                    let n = sb.pointers.get(&asset.pointers).map_or(0, Vec::len);
                    for i in 0..n as u32 {
                        let Some((head, _, bytes)) = blob_of(&sb, asset, i) else {
                            continue;
                        };
                        let last = head.get(*last_at).copied().unwrap_or(0).max(0) as usize;
                        let every = head.get(*every_at).copied().unwrap_or(1).max(1) as u32;
                        let steps: Vec<u32> =
                            bytes.iter().take(last + 1).map(|&b| b as u32).collect();
                        t.sequences.push(Sequence {
                            table: name.clone(),
                            id: i,
                            frames: frames.clone(),
                            steps,
                            every,
                        });
                    }
                    for (entity, eids) in ids {
                        for &id in eids {
                            if let Some(k) = t
                                .sequences
                                .iter()
                                .position(|s| s.table == *name && s.id == u32::from(id))
                            {
                                t.sequences_of.entry(entity.clone()).or_default().push(k);
                                for &f in &t.sequences[k].steps {
                                    // Frame 0 removes the entity in Mega
                                    // Man 2; a frame the table doesn't
                                    // hold is skipped either way.
                                    if t.frames.contains_key(&(frames.clone(), f)) {
                                        t.of_entity
                                            .entry(entity.clone())
                                            .or_default()
                                            .insert((frames.clone(), f));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for (entity, e) in &config.entities {
            for (table, list) in &e.frames {
                for &i in list {
                    if t.frames.contains_key(&(table.clone(), i)) {
                        t.of_entity
                            .entry(entity.clone())
                            .or_default()
                            .insert((table.clone(), i));
                    }
                }
            }
        }
        Ok(t)
    }
}
