//! The asset map: where one ROM keeps its assets and in which format, as
//! data (`assets/<source-id>.toml`, docs/assets.md). Adding a game means
//! writing its map, not code, as long as its formats are ones this crate
//! knows.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::Error;
use crate::format::Module;

/// The schema version of asset maps and of the files `lift` writes.
pub const SCHEMA: u32 = 1;

/// The most bytes one span may hold: more than any NES ROM's PRG or CHR
/// data, and small enough that span arithmetic can't overflow.
pub const MAX_SPAN: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetMap {
    pub schema: u32,
    /// The source ID, as in `sources/<platform>.toml` (or the game's own
    /// name for one that isn't registered, such as Moss Hop).
    pub source: String,
    pub title: Option<String>,
    /// SHA-256 of the ROM's PRG and CHR data, header excluded, as
    /// `gp roms scan` prints it. A map describes one revision only.
    pub sha256: String,
    #[serde(rename = "asset")]
    pub assets: Vec<Asset>,
    /// What each span holds, as hashes, for finding it in other dumps
    /// (crate::content, the feature `asset_map.content`).
    #[serde(default)]
    pub content: Vec<crate::content::Content>,
    /// The fingerprints of the ROM's whole PRG (`asset_map.code`).
    #[serde(default)]
    pub code: Option<crate::content::Code>,
}

/// Which part of the ROM an offset counts from: PRG ROM or CHR ROM, both
/// after the header (and trainer, if any).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Prg,
    Chr,
}

impl Region {
    pub fn name(self) -> &'static str {
        match self {
            Region::Prg => "prg",
            Region::Chr => "chr",
        }
    }
}

/// One asset. `name` is also its file name in a lifted directory; `label`
/// is the symbol the game's source or disassembly gives its data, kept for
/// people and for tests that check the map against a debug file.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "format", deny_unknown_fields)]
pub enum Asset {
    /// NES pattern tiles: 16 bytes per 8x8 tile, two bit planes.
    #[serde(rename = "chr-2bpp")]
    Chr {
        name: String,
        label: Option<String>,
        region: Region,
        offset: u32,
        tiles: u32,
        #[serde(default = "default_tiles_per_row")]
        tiles_per_row: u32,
        /// The character each of its tiles shows, for a game that draws
        /// text from tiles and keeps no strings: Moss Hop's counters are
        /// the digits from tile $30. A HUD's `font` slot can be filled from
        /// it, as from a string asset's charset.
        #[serde(default)]
        charset: Option<crate::charset::Charset>,
    },
    /// Palettes of four NES color numbers each.
    #[serde(rename = "palette")]
    Palette {
        name: String,
        label: Option<String>,
        /// The symbol at the byte after the span, for the label check.
        end_label: Option<String>,
        region: Region,
        offset: u32,
        count: u32,
    },
    /// 16x16 metatiles: four tile numbers each (top left, top right,
    /// bottom left, bottom right), plus one byte per metatile for each
    /// property, from its own table.
    #[serde(rename = "metatiles")]
    Metatiles {
        name: String,
        label: Option<String>,
        end_label: Option<String>,
        region: Region,
        offset: u32,
        count: u32,
        /// One name per metatile, for the lifted file's comments.
        names: Vec<String>,
        #[serde(default, rename = "property")]
        properties: Vec<Column>,
        /// The order of each metatile's four tile numbers in the ROM. The
        /// lifted file always lists them by rows (top left, top right,
        /// bottom left, bottom right).
        #[serde(default)]
        tile_order: TileOrder,
    },
    /// A level stored as a grid of metatile numbers, one byte per cell, row
    /// by row.
    #[serde(rename = "level-grid")]
    Level {
        name: String,
        label: Option<String>,
        end_label: Option<String>,
        region: Region,
        offset: u32,
        width: u32,
        height: u32,
        /// One character per metatile number: the character at index N
        /// stands for byte N in the lifted file's rows.
        legend: String,
        /// The metatiles asset whose names the lifted file's comments use.
        metatiles: Option<String>,
    },
    /// Rows of numbers stored as parallel tables, one per column.
    #[serde(rename = "table")]
    Table {
        name: String,
        count: u32,
        #[serde(rename = "column")]
        columns: Vec<Column>,
    },
    /// PPU update strings, such as status bar text (crate::ppu).
    #[serde(rename = "ppu-strings")]
    PpuStrings(crate::ppu::PpuStrings),
    /// A table stored row by row (crate::rows).
    #[serde(rename = "byte-rows")]
    ByteRows(crate::rows::ByteRows),
    /// Variable-length records that pointer tables point into
    /// (crate::blobs).
    #[serde(rename = "blobs")]
    Blobs(crate::blobs::Blobs),
    /// A table of 16-bit CPU addresses, each pointing at the start of
    /// another asset or of an entry of an object-list asset.
    #[serde(rename = "pointers")]
    Pointers {
        name: String,
        label: Option<String>,
        hi_label: Option<String>,
        end_label: Option<String>,
        hi_end_label: Option<String>,
        region: Region,
        /// The low bytes, or the whole little-endian words when there's no
        /// `hi_offset`.
        offset: u32,
        /// The high bytes, when the table is split in two.
        hi_offset: Option<u32>,
        count: u32,
        /// The CPU address of PRG offset 0 (0x8000 for 32KB of NROM PRG).
        cpu_base: u32,
        /// The PRG offset `cpu_base` stands for, when the pointers point
        /// into a switched bank: 0x10000 for Super Mario Bros. 2's banks 8
        /// and 9, which it maps at $8000. 0 by default.
        #[serde(default)]
        prg_base: u32,
        /// How many bytes from `prg_base` the bank maps at `cpu_base`
        /// (0x4000 for a 16KB bank). With it, `pack` refuses a pointer to
        /// data that moved past the bank's end. Without it, any address up
        /// to 64KB is written.
        window: Option<u32>,
    },
    /// Sprite frames and palettes that `gp sprites capture` writes, with no
    /// ROM span of its own (crate::sprite_set).
    #[serde(rename = "sprite-set")]
    SpriteSet(crate::sprite_set::SpriteSetEntry),
}

/// How a metatile's four tile numbers are stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileOrder {
    /// Top left, top right, bottom left, bottom right.
    #[default]
    Rows,
    /// Top left, bottom left, top right, bottom right, as Super Mario
    /// Bros. stores them.
    Columns,
}

/// One list of an object-list asset. `name` is its file name in the
/// asset's directory.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    pub label: Option<String>,
    /// One line for the lifted file's comments, such as the levels that
    /// use the area.
    pub note: Option<String>,
    /// An empty list with no bytes of its own: its pointer points at the
    /// previous entry's end marker. Super Mario Bros.' `E_GroundArea10`
    /// is one. Enemy lists only, since an area list has a header.
    #[serde(default)]
    pub tail: bool,
    /// How many pages a Super Mario Bros. 2 enemy list has. The game counts
    /// them from the area's level, so the list's bytes don't say.
    pub pages: Option<u32>,
    /// A Super Mario Bros. 2 level list that is only its end marker, with
    /// no header (`LevelData_7_3_Area0`, an area no level uses).
    #[serde(default)]
    pub bare: bool,
}

fn default_tiles_per_row() -> u32 {
    16
}

/// One column of a table, or one metatile property: a byte per row from
/// the table at `offset`, or a 16-bit value split into a low-byte table at
/// `offset` and a high-byte table at `hi_offset`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    pub label: Option<String>,
    pub hi_label: Option<String>,
    pub end_label: Option<String>,
    pub hi_end_label: Option<String>,
    pub region: Region,
    pub offset: u32,
    pub hi_offset: Option<u32>,
}

/// A run of ROM bytes an asset owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub region: Region,
    pub offset: u32,
    pub len: u32,
    /// What the bytes are, e.g. "palettes" or "slugs.start (high bytes)".
    pub part: String,
    pub label: Option<String>,
}

impl Asset {
    /// The asset's format module, for formats kept in a module of their
    /// own (crate::format). Adding one is a variant above and a line here.
    pub fn module(&self) -> Option<&dyn Module> {
        match self {
            Asset::PpuStrings(m) => Some(m),
            Asset::ByteRows(m) => Some(m),
            Asset::Blobs(m) => Some(m),
            Asset::SpriteSet(m) => Some(m),
            _ => None,
        }
    }

    /// The text encoding the asset gives: a string asset's, or a tile
    /// asset's own.
    pub fn charset(&self) -> Option<&crate::charset::Charset> {
        match self {
            Asset::Chr { charset, .. } => charset.as_ref(),
            Asset::PpuStrings(m) => m.charset.as_ref(),
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        if let Some(m) = self.module() {
            return m.name();
        }
        match self {
            Asset::Chr { name, .. }
            | Asset::Palette { name, .. }
            | Asset::Metatiles { name, .. }
            | Asset::Level { name, .. }
            | Asset::Table { name, .. }
            | Asset::Pointers { name, .. } => name,
            _ => unreachable!("module formats return above"),
        }
    }

    pub fn format(&self) -> &'static str {
        if let Some(m) = self.module() {
            return m.format();
        }
        match self {
            Asset::Chr { .. } => "chr-2bpp",
            Asset::Palette { .. } => "palette",
            Asset::Metatiles { .. } => "metatiles",
            Asset::Level { .. } => "level-grid",
            Asset::Table { .. } => "table",
            Asset::Pointers { .. } => "pointers",
            _ => unreachable!("module formats return above"),
        }
    }

    /// Each span's label and end label, for the formats kept in this
    /// file (a format module checks its own).
    fn labels(&self) -> Vec<(String, Option<&String>, Option<&String>)> {
        let name = self.name().to_owned();
        fn cols<'a>(
            asset: &str,
            cs: &'a [Column],
        ) -> Vec<(String, Option<&'a String>, Option<&'a String>)> {
            cs.iter()
                .flat_map(|c| {
                    let what = format!("{asset}.{}", c.name);
                    [
                        (what.clone(), c.label.as_ref(), c.end_label.as_ref()),
                        (what, c.hi_label.as_ref(), c.hi_end_label.as_ref()),
                    ]
                })
                .collect()
        }
        match self {
            Asset::Palette {
                label, end_label, ..
            }
            | Asset::Level {
                label, end_label, ..
            } => vec![(name, label.as_ref(), end_label.as_ref())],
            Asset::Metatiles {
                label,
                end_label,
                properties,
                ..
            } => {
                let mut out = cols(&name, properties);
                out.insert(0, (name, label.as_ref(), end_label.as_ref()));
                out
            }
            Asset::Table { columns, .. } => cols(&name, columns),
            Asset::Pointers {
                label,
                hi_label,
                end_label,
                hi_end_label,
                ..
            } => vec![
                (name.clone(), label.as_ref(), end_label.as_ref()),
                (name, hi_label.as_ref(), hi_end_label.as_ref()),
            ],
            _ => Vec::new(),
        }
    }

    /// The file this asset lifts to, in the lifted directory. An
    /// object-list asset lifts to a directory of that name with a file per
    /// entry (see [`Asset::files`]), and this is the directory, ending in
    /// `/`.
    pub fn file(&self) -> String {
        if let Some(m) = self.module() {
            return m.file();
        }
        match self {
            Asset::Chr { name, .. } => format!("{name}.png"),
            _ => format!("{}.toml", self.name()),
        }
    }

    /// Every file this asset lifts to, as paths in the lifted directory
    /// with `/` between parts.
    pub fn files(&self) -> Vec<String> {
        if let Some(m) = self.module() {
            return m.files();
        }
        vec![self.file()]
    }

    /// The ROM bytes the asset owns, in the order its encoder writes them.
    pub fn spans(&self) -> Vec<Span> {
        if let Some(m) = self.module() {
            return m.spans();
        }
        let span = |region, offset, len, part: String, label: &Option<String>| Span {
            region,
            offset,
            len,
            part,
            label: label.clone(),
        };
        let name = self.name().to_owned();
        let columns = |columns: &[Column], rows: u32| {
            let mut out = Vec::new();
            for c in columns {
                let part = format!("{name}.{}", c.name);
                if let Some(hi) = c.hi_offset {
                    out.push(span(
                        c.region,
                        c.offset,
                        rows,
                        format!("{part} (low bytes)"),
                        &c.label,
                    ));
                    out.push(span(
                        c.region,
                        hi,
                        rows,
                        format!("{part} (high bytes)"),
                        &c.hi_label,
                    ));
                } else {
                    out.push(span(c.region, c.offset, rows, part, &c.label));
                }
            }
            out
        };
        match self {
            Asset::Chr {
                label,
                region,
                offset,
                tiles,
                ..
            } => vec![span(*region, *offset, tiles * 16, name.clone(), label)],
            Asset::Palette {
                label,
                region,
                offset,
                count,
                ..
            } => vec![span(*region, *offset, count * 4, name.clone(), label)],
            Asset::Metatiles {
                label,
                region,
                offset,
                count,
                properties,
                ..
            } => {
                let mut out = vec![span(*region, *offset, count * 4, name.clone(), label)];
                out.extend(columns(properties, *count));
                out
            }
            Asset::Level {
                label,
                region,
                offset,
                width,
                height,
                ..
            } => vec![span(*region, *offset, width * height, name.clone(), label)],
            Asset::Table {
                count, columns: c, ..
            } => columns(c, *count),
            Asset::Pointers {
                label,
                hi_label,
                region,
                offset,
                hi_offset,
                count,
                ..
            } => match hi_offset {
                Some(hi) => vec![
                    span(
                        *region,
                        *offset,
                        *count,
                        format!("{name} (low bytes)"),
                        label,
                    ),
                    span(
                        *region,
                        *hi,
                        *count,
                        format!("{name} (high bytes)"),
                        hi_label,
                    ),
                ],
                None => vec![span(*region, *offset, count * 2, name.clone(), label)],
            },
            _ => unreachable!("module formats return above"),
        }
    }
}

impl AssetMap {
    /// Parses and checks a map: known schema, a SHA-256, unique names that
    /// are safe file names, consistent counts and legends, and no two
    /// assets claiming the same byte. Whether the spans fit a ROM is
    /// checked when one is given.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let map: AssetMap = toml::from_str(text).map_err(|e| Error::Map(e.to_string()))?;
        map.validate()?;
        Ok(map)
    }

    fn validate(&self) -> Result<(), Error> {
        let bad = |msg: String| Err(Error::Map(msg));
        if self.schema != SCHEMA {
            return bad(format!(
                "schema {} isn't supported; this gp reads schema {SCHEMA}",
                self.schema
            ));
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return bad(format!(
                "sha256 {:?} isn't 64 hex digits (the ROM's PRG and CHR data, as gp roms scan prints it)",
                self.sha256
            ));
        }
        if !safe_name(&self.source) {
            return bad(format!(
                "source {:?} must be lowercase letters, digits, - and _ (it names the default output directory)",
                self.source
            ));
        }
        if self
            .title
            .as_ref()
            .is_some_and(|t| t.contains(['\n', '\r']))
        {
            return bad("title must be one line".into());
        }
        let mut names = BTreeSet::new();
        for asset in &self.assets {
            let module = asset.module();
            if let Some(m) = module {
                m.validate().map_err(Error::Map)?;
            }
            let name = asset.name();
            if let Asset::Chr {
                charset: Some(c), ..
            } = asset
            {
                c.validate(name).map_err(Error::Map)?;
            }
            let size = |count: u32, each: u64| u64::from(count) * each;
            let largest = match asset {
                _ if module.is_some() => 0,
                Asset::Chr { tiles, .. } => size(*tiles, 16),
                Asset::Palette { count, .. } | Asset::Metatiles { count, .. } => size(*count, 4),
                Asset::Level { width, height, .. } => u64::from(*width) * u64::from(*height),
                Asset::Table { count, .. } => u64::from(*count),
                Asset::Pointers { count, .. } => size(*count, 2),
                _ => unreachable!("module formats return 0 above"),
            };
            if largest > MAX_SPAN {
                return bad(format!(
                    "{name}: {largest} bytes in one span is more than any NES ROM holds ({MAX_SPAN})"
                ));
            }
            if !safe_name(name) {
                return bad(format!(
                    "asset name {name:?} must be lowercase letters, digits, - and _ (it's a file name)"
                ));
            }
            if name == "assets" {
                return bad(
                    "no asset may be named \"assets\": assets.toml is the map's copy".into(),
                );
            }
            if name == "slots" {
                return bad(
                    "no asset may be named \"slots\": slots.toml is the slots' state (docs/stubs.md)"
                        .into(),
                );
            }
            if !names.insert(name) {
                return bad(format!("asset {name:?} is named twice"));
            }
            for (what, label, end) in asset.labels() {
                crate::format::check_labels(&what, label, end).map_err(Error::Map)?;
            }
            match asset {
                _ if module.is_some() => {}
                Asset::Chr {
                    tiles,
                    tiles_per_row,
                    ..
                } => {
                    if *tiles == 0 || *tiles_per_row == 0 {
                        return bad(format!(
                            "{name}: tiles and tiles_per_row must be at least 1"
                        ));
                    }
                }
                Asset::Palette { count, .. } => {
                    if *count == 0 {
                        return bad(format!("{name}: count must be at least 1"));
                    }
                }
                Asset::Metatiles {
                    count,
                    names: labels,
                    properties,
                    ..
                } => {
                    if *count == 0 {
                        return bad(format!("{name}: count must be at least 1"));
                    }
                    if labels.len() != *count as usize {
                        return bad(format!(
                            "{name}: {} names for {count} metatiles",
                            labels.len()
                        ));
                    }
                    if labels.iter().any(|l| l.contains(['\n', '\r'])) {
                        return bad(format!("{name}: names must be one line each"));
                    }
                    check_columns(name, properties, &["tiles"])?;
                }
                Asset::Level {
                    width,
                    height,
                    legend,
                    metatiles,
                    ..
                } => {
                    if *width == 0 || *height == 0 {
                        return bad(format!("{name}: width and height must be at least 1"));
                    }
                    let chars: Vec<char> = legend.chars().collect();
                    if chars.is_empty() || chars.len() > 256 {
                        return bad(format!("{name}: the legend needs 1 to 256 characters"));
                    }
                    if let Some(c) = chars
                        .iter()
                        .find(|c| !c.is_ascii_graphic() || **c == '"' || **c == '\\')
                    {
                        return bad(format!(
                            "{name}: legend character {c:?} must be printable ASCII other than a space, \" or \\"
                        ));
                    }
                    if chars.iter().collect::<BTreeSet<_>>().len() != chars.len() {
                        return bad(format!("{name}: the legend repeats a character"));
                    }
                    if let Some(m) = metatiles {
                        match self.assets.iter().find(|a| a.name() == m) {
                            Some(Asset::Metatiles { .. }) => {}
                            _ => {
                                return bad(format!(
                                    "{name}: metatiles = {m:?} names no metatiles asset"
                                ));
                            }
                        }
                    }
                }
                Asset::Table { count, columns, .. } => {
                    if *count == 0 || columns.is_empty() {
                        return bad(format!("{name}: a table needs a count and a column"));
                    }
                    check_columns(name, columns, &[])?;
                }
                Asset::Pointers {
                    region,
                    count,
                    cpu_base,
                    hi_offset,
                    hi_label,
                    prg_base,
                    window,
                    ..
                } => {
                    if *count == 0 {
                        return bad(format!("{name}: count must be at least 1"));
                    }
                    if let Some(w) = window
                        && (*w == 0 || cpu_base + w > 0x10000)
                    {
                        return bad(format!(
                            "{name}: window {w:#x} must be at least 1 and end within the 6502's 64KB from cpu_base"
                        ));
                    }
                    if *region != Region::Prg {
                        return bad(format!("{name}: pointer tables are read from prg"));
                    }
                    if *cpu_base > 0xFFFF || *prg_base > MAX_SPAN as u32 {
                        return bad(format!(
                            "{name}: cpu_base {cpu_base:#x} is past the 6502's 64KB, or prg_base {prg_base:#x} past any ROM"
                        ));
                    }
                    if hi_offset.is_none() && hi_label.is_some() {
                        return bad(format!("{name}: hi_label without hi_offset"));
                    }
                }
                _ => unreachable!("module formats are checked above"),
            }
        }
        for m in self.assets.iter().filter_map(Asset::module) {
            m.check_refs(self).map_err(Error::Map)?;
        }
        self.check_overlaps()?;
        crate::content::validate(self)
    }

    fn check_overlaps(&self) -> Result<(), Error> {
        let mut spans: Vec<Span> = self.assets.iter().flat_map(Asset::spans).collect();
        spans.sort_by_key(|s| (s.region, s.offset));
        for pair in spans.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if a.region == b.region && u64::from(a.offset) + u64::from(a.len) > u64::from(b.offset)
            {
                return Err(Error::Map(format!(
                    "{} and {} both claim {} offset {:#06x}",
                    a.part,
                    b.part,
                    b.region.name(),
                    b.offset
                )));
            }
        }
        Ok(())
    }

    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name() == name)
    }

    /// Every span of every asset.
    pub fn spans(&self) -> Vec<Span> {
        self.assets.iter().flat_map(Asset::spans).collect()
    }

    /// How many ROM bytes the map's assets cover.
    pub fn bytes_covered(&self) -> u64 {
        self.spans().iter().map(|s| u64::from(s.len)).sum()
    }
}

use crate::format::safe_name;

fn check_columns(asset: &str, columns: &[Column], reserved: &[&str]) -> Result<(), Error> {
    let mut seen = BTreeSet::new();
    for c in columns {
        let ok = !c.name.is_empty()
            && c.name.starts_with(|ch: char| ch.is_ascii_lowercase())
            && c.name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !ok {
            return Err(Error::Map(format!(
                "{asset}: column name {:?} must start with a lowercase letter and hold only lowercase letters, digits and _",
                c.name
            )));
        }
        if reserved.contains(&c.name.as_str()) || !seen.insert(c.name.as_str()) {
            return Err(Error::Map(format!(
                "{asset}: column {:?} is named twice or uses a reserved name",
                c.name
            )));
        }
        if c.hi_offset.is_none() && c.hi_label.is_some() {
            return Err(Error::Map(format!(
                "{asset}.{}: hi_label without hi_offset",
                c.name
            )));
        }
    }
    Ok(())
}
