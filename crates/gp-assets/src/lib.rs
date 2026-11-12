//! Asset lifting (Milestone 2, step 1; docs/assets.md): a game's tiles,
//! palettes, metatiles and level data decoded from its ROM into files
//! people and tools can edit, and encoded back to bytes.
//!
//! An [`AssetMap`] says where each asset is and in which format. [`lift`]
//! decodes every asset from a ROM into files; [`encode`] turns the files
//! back into ROM bytes; [`differences`] compares those with a ROM, which is
//! the round-trip check, and [`apply`] writes them into a copy of one.
//! Everything works on bytes in memory: the CLI does the file system.
//!
//! The lifted files' bytes depend only on the ROM and the map, so lifting
//! the same ROM gives the same files on every platform.

pub mod arcade;
pub mod authored;
pub mod blobs;
pub mod charset;
pub mod chr;
pub mod content;
pub mod find;
pub mod format;
pub mod map;
pub mod ppu;
pub mod rows;
pub mod sprite_set;
pub mod stub;
mod text;

use gp_evidence::source::{SourceError, identify_nes};
use serde::Serialize;

pub use map::{Asset, AssetMap, Column, Entry, Region, Span, TileOrder};

/// The file name of the map's copy in a lifted directory.
pub const MAP_FILE: &str = "assets.toml";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("asset map: {0}")]
    Map(String),
    #[error("not an NES ROM: {0}")]
    Rom(#[from] SourceError),
    #[error("the asset map is for ROM sha256 {expected} ({source_id}); this ROM's is {actual}")]
    WrongRom {
        source_id: String,
        expected: String,
        actual: String,
    },
    #[error(
        "{part}: {region} offset {offset:#06x} + {len} bytes is past the end of the ROM's {region} data ({size} bytes)"
    )]
    OutOfRange {
        part: String,
        region: &'static str,
        offset: u32,
        len: u32,
        size: usize,
    },
    #[error("{asset}: {msg}")]
    Decode { asset: String, msg: String },
    #[error("{file}: {msg}")]
    File { file: String, msg: String },
}

/// Where PRG and CHR data sit in an iNES file.
#[derive(Debug, Clone)]
pub struct RomLayout {
    pub sha256: String,
    prg: std::ops::Range<usize>,
    chr: std::ops::Range<usize>,
}

impl RomLayout {
    pub fn of(rom: &[u8]) -> Result<Self, Error> {
        let img = identify_nes(rom)?;
        let start = 16 + if img.header.trainer { 512 } else { 0 };
        let prg_end = start + img.header.prg_rom_bytes as usize;
        let chr_end = prg_end + img.header.chr_rom_bytes as usize;
        Ok(Self {
            sha256: img.sha256,
            prg: start..prg_end,
            chr: prg_end..chr_end,
        })
    }

    /// The file offset of a span, after checking it fits its region.
    pub fn file_range(&self, span: &Span) -> Result<std::ops::Range<usize>, Error> {
        let region = match span.region {
            Region::Prg => &self.prg,
            Region::Chr => &self.chr,
        };
        let size = region.len();
        let end = u64::from(span.offset) + u64::from(span.len);
        if end > size as u64 {
            return Err(Error::OutOfRange {
                part: span.part.clone(),
                region: span.region.name(),
                offset: span.offset,
                len: span.len,
                size,
            });
        }
        let start = region.start + span.offset as usize;
        Ok(start..start + span.len as usize)
    }

    /// Whether this is the ROM the map describes.
    pub fn is_for(&self, map: &AssetMap) -> bool {
        self.sha256.eq_ignore_ascii_case(&map.sha256)
    }

    /// Checks the ROM is the map's and every span fits it.
    pub fn check(&self, map: &AssetMap) -> Result<(), Error> {
        if !self.is_for(map) {
            return Err(Error::WrongRom {
                source_id: map.source.clone(),
                expected: map.sha256.clone(),
                actual: self.sha256.clone(),
            });
        }
        self.check_spans(map)
    }

    /// Checks every span of the map fits this ROM, whatever ROM it is.
    pub fn check_spans(&self, map: &AssetMap) -> Result<(), Error> {
        for span in map.spans() {
            self.file_range(&span)?;
        }
        Ok(())
    }
}

/// One file of a lifted directory.
#[derive(Debug, Clone)]
pub struct File {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// What `lift` decoded, per asset.
#[derive(Debug, Clone, Serialize)]
pub struct Lifted {
    pub name: String,
    pub format: &'static str,
    pub file: String,
    pub bytes: u64,
}

/// Decodes every asset of `map` from `rom` into files. The first file is
/// the map itself (`assets.toml`, from `map_text` unchanged), so a lifted
/// directory says how to pack it.
pub fn lift(rom: &[u8], map: &AssetMap, map_text: &str) -> Result<(Vec<File>, Vec<Lifted>), Error> {
    RomLayout::of(rom)?.check(map)?;
    lift_any(rom, map, map_text)
}

/// [`lift`] for a ROM that isn't the map's, such as the image
/// [`find::assemble`] makes from another dump: only that the spans fit is
/// checked.
pub fn lift_any(
    rom: &[u8],
    map: &AssetMap,
    map_text: &str,
) -> Result<(Vec<File>, Vec<Lifted>), Error> {
    let layout = RomLayout::of(rom)?;
    layout.check_spans(map)?;
    let read = |span: &Span| -> Result<&[u8], Error> { Ok(&rom[layout.file_range(span)?]) };
    let mut files = vec![File {
        name: MAP_FILE.into(),
        bytes: map_text.as_bytes().to_vec(),
    }];
    let mut lifted = Vec::new();
    let source = map.title.as_deref().unwrap_or(&map.source);
    let regions = format::Rom {
        prg: &rom[layout.prg.clone()],
        chr: &rom[layout.chr.clone()],
    };
    for asset in &map.assets {
        let spans = asset.spans();
        let name = asset.name();
        if let Some(m) = asset.module() {
            let asset_files = m.lift(source, map, &regions).map_err(|msg| Error::Decode {
                asset: name.into(),
                msg,
            })?;
            lifted.push(Lifted {
                name: name.into(),
                format: asset.format(),
                file: asset.file(),
                bytes: spans.iter().map(|s| u64::from(s.len)).sum(),
            });
            files.extend(asset_files);
            continue;
        }
        let column_values = |cols: &[Column], spans: &[Span]| -> Result<Vec<Vec<u32>>, Error> {
            let mut values = Vec::new();
            let mut it = spans.iter();
            for c in cols {
                let lo = read(it.next().expect("a span per column"))?;
                let v: Vec<u32> = if c.hi_offset.is_some() {
                    let hi = read(it.next().expect("a high span"))?;
                    lo.iter()
                        .zip(hi)
                        .map(|(&l, &h)| u32::from(l) | u32::from(h) << 8)
                        .collect()
                } else {
                    lo.iter().map(|&b| u32::from(b)).collect()
                };
                values.push(v);
            }
            Ok(values)
        };
        let file = |bytes: Vec<u8>| {
            vec![File {
                name: asset.file(),
                bytes,
            }]
        };
        let asset_files = match asset {
            Asset::Chr { tiles_per_row, .. } => {
                let (w, h, pixels) = chr::decode(read(&spans[0])?, *tiles_per_row);
                file(chr::write_png(w, h, &pixels))
            }
            Asset::Palette { .. } => {
                file(text::write_palettes(source, name, read(&spans[0])?).into_bytes())
            }
            Asset::Metatiles {
                names,
                properties,
                tile_order,
                ..
            } => {
                let values = column_values(properties, &spans[1..])?;
                let props: Vec<_> = properties.iter().zip(values).collect();
                let legend = map.assets.iter().find_map(|a| match a {
                    Asset::Level {
                        legend,
                        metatiles: Some(m),
                        ..
                    } if m == name => Some(legend.as_str()),
                    _ => None,
                });
                let tiles = reorder(read(&spans[0])?, *tile_order);
                file(
                    text::write_metatiles(source, name, &tiles, names, legend, &props).into_bytes(),
                )
            }
            Asset::Level {
                width,
                legend,
                metatiles,
                ..
            } => {
                let cells = read(&spans[0])?;
                let symbols = legend.chars().count();
                if let Some(i) = cells.iter().position(|&b| b as usize >= symbols) {
                    return Err(Error::Decode {
                        asset: name.into(),
                        msg: format!(
                            "cell {} (row {}, column {}) holds {}, which the legend's {symbols} characters don't cover",
                            i,
                            i / *width as usize,
                            i % *width as usize,
                            cells[i]
                        ),
                    });
                }
                let names = metatiles.as_ref().and_then(|m| match map.asset(m) {
                    Some(Asset::Metatiles { names, .. }) => Some(names.as_slice()),
                    _ => None,
                });
                file(text::write_level(source, name, *width, cells, legend, names).into_bytes())
            }
            Asset::Table { columns, .. } => {
                let values = column_values(columns, &spans)?;
                let cols: Vec<_> = columns.iter().zip(values).collect();
                file(text::write_table(source, name, &cols).into_bytes())
            }
            Asset::Pointers {
                hi_offset,
                cpu_base,
                prg_base,
                window,
                ..
            } => {
                let values: Vec<u32> = match hi_offset {
                    Some(_) => {
                        let (lo, hi) = (read(&spans[0])?, read(&spans[1])?);
                        lo.iter()
                            .zip(hi)
                            .map(|(&l, &h)| u32::from(l) | u32::from(h) << 8)
                            .collect()
                    }
                    None => read(&spans[0])?
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|w| u32::from(w[0]) | u32::from(w[1]) << 8)
                        .collect(),
                };
                let places = rom_targets(map);
                let mut names = Vec::with_capacity(values.len());
                for (i, &v) in values.iter().enumerate() {
                    let at: Vec<&(u32, String)> = match v
                        .checked_sub(*cpu_base)
                        .filter(|off| window.is_none_or(|w| *off < w))
                    {
                        Some(off) => places
                            .iter()
                            .filter(|(o, _)| *o == off + prg_base)
                            .collect(),
                        None => Vec::new(),
                    };
                    if at.len() > 1 {
                        let names: Vec<&str> = at.iter().map(|(_, n)| n.as_str()).collect();
                        return Err(Error::Decode {
                            asset: name.into(),
                            msg: format!(
                                "pointer {i} holds {v:#06x}, where {} all start, so it can't name one",
                                names.join(", ")
                            ),
                        });
                    }
                    let found = at.first();
                    let Some((_, target)) = found else {
                        if let (Some(w), Some(off)) = (window, v.checked_sub(*cpu_base))
                            && off >= *w
                        {
                            return Err(Error::Decode {
                                asset: name.into(),
                                msg: format!(
                                    "pointer {i} holds {v:#06x}, past the {w:#x} bytes the bank maps from cpu_base {cpu_base:#06x}"
                                ),
                            });
                        }
                        return Err(Error::Decode {
                            asset: name.into(),
                            msg: format!(
                                "pointer {i} holds {v:#06x}, which isn't where any asset in prg starts"
                            ),
                        });
                    };
                    names.push((target.clone(), v));
                }
                file(text::write_pointers(source, name, &names).into_bytes())
            }
            _ => unreachable!("module formats lift above"),
        };
        lifted.push(Lifted {
            name: name.into(),
            format: asset.format(),
            file: asset.file(),
            bytes: spans.iter().map(|s| u64::from(s.len)).sum(),
        });
        files.extend(asset_files);
    }
    Ok((files, lifted))
}

/// A metatile table's tile numbers in rows order (top left, top right,
/// bottom left, bottom right), from `order`, or back: swapping the middle
/// two is its own inverse.
fn reorder(tiles: &[u8], order: TileOrder) -> Vec<u8> {
    let mut out = tiles.to_vec();
    if order == TileOrder::Columns {
        for m in out.as_chunks_mut::<4>().0.iter_mut() {
            m.swap(1, 2);
        }
    }
    out
}

/// Where each thing a pointer table may name starts in the ROM's PRG:
/// every asset with data in PRG.
fn rom_targets(map: &AssetMap) -> Vec<(u32, String)> {
    map.assets
        .iter()
        .filter_map(|a| {
            let s = a.spans().into_iter().next()?;
            (s.region == Region::Prg).then(|| (s.offset, a.name().to_owned()))
        })
        .collect()
}

/// The bytes one span of an asset encodes to.
#[derive(Debug, Clone)]
pub struct Encoded {
    pub asset: String,
    pub span: Span,
    pub bytes: Vec<u8>,
}

/// Encodes every asset of `map` from its lifted files, which `read` returns
/// by path in the lifted directory. Pointer tables are encoded last, from
/// where the other assets' data ended up.
pub fn encode(
    map: &AssetMap,
    read: &dyn Fn(&str) -> std::io::Result<Vec<u8>>,
) -> Result<Vec<Encoded>, Error> {
    let mut chunks: Vec<Option<Vec<Vec<u8>>>> = vec![None; map.assets.len()];
    for (i, asset) in map.assets.iter().enumerate() {
        if matches!(asset, Asset::Pointers { .. }) {
            continue;
        }
        if let Some(m) = asset.module() {
            chunks[i] = Some(m.encode(map, read)?);
            continue;
        }
        let file = asset.file();
        let fail = |msg: String| Error::File {
            file: file.clone(),
            msg,
        };
        let raw = read(&file).map_err(|e| fail(e.to_string()))?;
        chunks[i] = Some(encode_asset(asset, &raw).map_err(fail)?);
    }
    for (i, asset) in map.assets.iter().enumerate() {
        if let Asset::Pointers { .. } = asset {
            let file = asset.file();
            let raw = read(&file).map_err(|e| Error::File {
                file: file.clone(),
                msg: e.to_string(),
            })?;
            chunks[i] =
                Some(encode_pointers(map, asset, &raw).map_err(|msg| Error::File { file, msg })?);
        }
    }
    let mut out = Vec::new();
    for (asset, chunks) in map.assets.iter().zip(chunks) {
        let chunks = chunks.expect("every asset encoded");
        let fail = |msg: String| Error::File {
            file: asset.file(),
            msg,
        };
        let spans = asset.spans();
        if spans.len() != chunks.len() {
            return Err(fail(format!(
                "encoded {} tables for {} spans (a bug in gp-assets)",
                chunks.len(),
                spans.len()
            )));
        }
        for (span, bytes) in spans.into_iter().zip(chunks) {
            if span.len as usize != bytes.len() {
                return Err(fail(format!(
                    "{} encoded to {} bytes for a {}-byte span (a bug in gp-assets)",
                    span.part,
                    bytes.len(),
                    span.len
                )));
            }
            out.push(Encoded {
                asset: asset.name().into(),
                span,
                bytes,
            });
        }
    }
    Ok(out)
}

/// A pointer table's bytes, from the names in its file and where those
/// assets or entries are.
fn encode_pointers(map: &AssetMap, asset: &Asset, raw: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let Asset::Pointers {
        name,
        hi_offset,
        count,
        cpu_base,
        prg_base,
        window,
        ..
    } = asset
    else {
        unreachable!("only pointer tables");
    };
    let text = std::str::from_utf8(raw).map_err(|_| "isn't UTF-8 text".to_string())?;
    let r = text::Reader::new(text, name, &["pointers"])?;
    let list = r.array(r.top(), "pointers", "")?;
    if list.len() != *count as usize {
        return Err(format!(
            "{} pointers, but assets.toml says {count}",
            list.len()
        ));
    }
    let (mut lo, mut hi, mut words) = (Vec::new(), Vec::new(), Vec::new());
    for (i, v) in list.iter().enumerate() {
        let toml::Value::String(target) = v else {
            return Err(format!("pointer {i} must be a name in quotes"));
        };
        let offset = map
            .asset(target)
            .and_then(|a| a.spans().into_iter().next())
            .filter(|s| s.region == Region::Prg)
            .map(|s| s.offset)
            .ok_or_else(|| format!("pointer {i}: {target:?} isn't an asset in prg"))?;
        let Some(address) = offset.checked_sub(*prg_base).map(|o| cpu_base + o) else {
            return Err(format!(
                "pointer {i}: {target:?} is at prg {offset:#x}, before prg_base {prg_base:#x}"
            ));
        };
        if let Some(w) = window
            && offset - prg_base >= *w
        {
            return Err(format!(
                "pointer {i}: {target:?} is at prg {offset:#x}, past the {w:#x} bytes the bank maps from prg_base {prg_base:#x}"
            ));
        }
        if address > 0xFFFF {
            return Err(format!(
                "pointer {i}: {target:?} is at {address:#x}, past the 6502's 64KB"
            ));
        }
        lo.push(address as u8);
        hi.push((address >> 8) as u8);
        words.extend([address as u8, (address >> 8) as u8]);
    }
    Ok(if hi_offset.is_some() {
        vec![lo, hi]
    } else {
        vec![words]
    })
}

/// One byte buffer per span of `asset`, in `spans()` order.
fn encode_asset(asset: &Asset, raw: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    if let Asset::Chr {
        tiles,
        tiles_per_row,
        ..
    } = asset
    {
        let (w, h) = chr::image_size(*tiles, *tiles_per_row);
        let pixels = chr::read_png(raw, (w, h))
            .map_err(|e| format!("{e} ({tiles} tiles, {tiles_per_row} to a row)"))?;
        if let Some(i) = pixels.iter().position(|&p| p > 3) {
            return Err(format!(
                "pixel ({}, {}) has color index {}; tiles have 4 colors, 0 to 3",
                i as u32 % w,
                i as u32 / w,
                pixels[i]
            ));
        }
        let used = |i: usize| {
            let (x, y) = (i as u32 % w / 8, i as u32 / w / 8);
            y * tiles_per_row + x < *tiles
        };
        if let Some(i) = (0..pixels.len()).find(|&i| pixels[i] != 0 && !used(i)) {
            return Err(format!(
                "pixel ({}, {}) is past the last tile and must stay index 0",
                i as u32 % w,
                i as u32 / w
            ));
        }
        return Ok(vec![chr::encode(&pixels, w, *tiles, *tiles_per_row)]);
    }

    let text = std::str::from_utf8(raw).map_err(|_| "isn't UTF-8 text".to_string())?;
    let columns = |r: &text::Reader, rows: &[&toml::Table], cols: &[Column], what: &str| {
        let mut out = Vec::new();
        for c in cols {
            let max = text::column_max(c);
            let mut lo = Vec::with_capacity(rows.len());
            let mut hi = Vec::with_capacity(rows.len());
            for (i, row) in rows.iter().enumerate() {
                let v = r.int(row, &c.name, &format!("{what} {i}: "), max)?;
                lo.push(v as u8);
                hi.push((v >> 8) as u8);
            }
            out.push(lo);
            if c.hi_offset.is_some() {
                out.push(hi);
            }
        }
        Ok::<_, String>(out)
    };
    match asset {
        Asset::Chr { .. } => unreachable!("handled above"),
        Asset::Palette { name, count, .. } => {
            let r = text::Reader::new(text, name, &["palettes"])?;
            let list = r.array(r.top(), "palettes", "")?;
            if list.len() != *count as usize {
                return Err(format!(
                    "{} palettes, but assets.toml says {count}",
                    list.len()
                ));
            }
            let mut bytes = Vec::new();
            for (i, p) in list.iter().enumerate() {
                bytes.extend(r.bytes(p, 4, &format!("palette {i} "))?);
            }
            Ok(vec![bytes])
        }
        Asset::Metatiles {
            name,
            count,
            properties,
            tile_order,
            ..
        } => {
            let r = text::Reader::new(text, name, &["metatile"])?;
            let rows = r.entries(r.top(), "metatile", *count as usize)?;
            let mut tiles = Vec::new();
            let mut keys = vec!["tiles"];
            keys.extend(properties.iter().map(|p| p.name.as_str()));
            for (i, m) in rows.iter().enumerate() {
                let at = format!("metatile {i}: ");
                r.only(m, &keys, &at)?;
                let t = m.get("tiles").ok_or(format!("{at}tiles is missing"))?;
                tiles.extend(r.bytes(t, 4, &format!("{at}tiles "))?);
            }
            let mut out = vec![reorder(&tiles, *tile_order)];
            out.extend(columns(&r, &rows, properties, "metatile")?);
            Ok(out)
        }
        Asset::Level {
            name,
            width,
            height,
            legend,
            ..
        } => {
            let r = text::Reader::new(text, name, &["rows"])?;
            let rows = r.array(r.top(), "rows", "")?;
            if rows.len() != *height as usize {
                return Err(format!(
                    "{} rows, but the level is {height} high",
                    rows.len()
                ));
            }
            let mut cells = Vec::with_capacity((width * height) as usize);
            for (y, row) in rows.iter().enumerate() {
                let toml::Value::String(s) = row else {
                    return Err(format!("row {y} must be a string"));
                };
                if s.chars().count() != *width as usize {
                    return Err(format!(
                        "row {y} has {} characters; the level is {width} wide",
                        s.chars().count()
                    ));
                }
                for (x, c) in s.chars().enumerate() {
                    let Some(n) = legend.chars().position(|l| l == c) else {
                        return Err(format!(
                            "row {y}, column {x}: {c:?} isn't in the legend {legend:?}"
                        ));
                    };
                    cells.push(n as u8);
                }
            }
            Ok(vec![cells])
        }
        Asset::Table {
            name,
            count,
            columns: cols,
        } => {
            let r = text::Reader::new(text, name, &["row"])?;
            let rows = r.entries(r.top(), "row", *count as usize)?;
            let keys: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
            for (i, row) in rows.iter().enumerate() {
                r.only(row, &keys, &format!("entry {i}: "))?;
            }
            columns(&r, &rows, cols, "entry")
        }
        _ => unreachable!("encoded by encode_lists, encode_pointers or a module"),
    }
}

/// A ROM byte the encoded assets disagree with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Difference {
    pub asset: String,
    pub part: String,
    pub region: &'static str,
    /// Offset in the region (PRG or CHR data).
    pub offset: u32,
    /// Offset in the ROM file, header included.
    pub file_offset: u64,
    pub rom: u8,
    pub encoded: u8,
}

/// Every byte where `encoded` differs from `rom`, in map order. The ROM
/// may be another than the map's (an edited one, say): comparing can only
/// find real differences, so only the spans' bounds are checked, and
/// [`RomLayout::is_for`] says whether it's the map's ROM.
pub fn differences(
    rom: &[u8],
    map: &AssetMap,
    encoded: &[Encoded],
) -> Result<Vec<Difference>, Error> {
    let layout = RomLayout::of(rom)?;
    layout.check_spans(map)?;
    let mut out = Vec::new();
    for e in encoded {
        let range = layout.file_range(&e.span)?;
        for (i, (&have, &want)) in rom[range.clone()].iter().zip(&e.bytes).enumerate() {
            if have != want {
                out.push(Difference {
                    asset: e.asset.clone(),
                    part: e.span.part.clone(),
                    region: e.span.region.name(),
                    offset: e.span.offset + i as u32,
                    file_offset: (range.start + i) as u64,
                    rom: have,
                    encoded: want,
                });
            }
        }
    }
    Ok(out)
}

/// A copy of `rom` with the encoded assets written over their spans.
pub fn apply(rom: &[u8], map: &AssetMap, encoded: &[Encoded]) -> Result<Vec<u8>, Error> {
    let layout = RomLayout::of(rom)?;
    layout.check(map)?;
    let mut out = rom.to_vec();
    for e in encoded {
        let range = layout.file_range(&e.span)?;
        out[range].copy_from_slice(&e.bytes);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod module_tests;
