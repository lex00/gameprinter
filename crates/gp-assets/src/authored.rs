//! Authored assets: an original template's assets written by hand as data
//! (`authored.toml`, docs/originals.md), built into a data image with the
//! same formats a NES cartridge's assets have, so that every asset tool
//! (lift, pack, stubs, the interpreter's reader) works on an original the
//! way it works on a lifted game.
//!
//! [`build`] lays the assets out one after another in PRG and CHR, writes
//! the asset map that says where each one is, and wraps the bytes in an
//! iNES header. The image holds data only, no code, so it doesn't boot; it
//! is the "ROM" the asset map describes, and its headerless SHA-256 is the
//! one the template accepts. The same `authored.toml` always builds the
//! same image and the same map, byte for byte.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;

use crate::Error;
use crate::chr;
use crate::stub;

/// The authored file's name in a template directory.
pub const AUTHORED_FILE: &str = "authored.toml";

/// The authored format's version. A reader takes the same major and minor
/// version with a patch no newer than its own.
///
/// 0.1.1 (2026-11-03) added the `byte-rows` and `ppu-strings` formats,
/// `bits = 8` tables and tiles drawn from a file (`file`), so an original
/// can fill the slots of a model written for a lifted game (Rivet Rodeo on
/// Donkey Kong's model, removed on 2026-11-11).
pub const FORMAT: &str = "0.1.1";

/// PRG is laid out in 16KB banks and CHR in 8KB ones, as iNES counts them.
const PRG_BANK: usize = 16 * 1024;
const CHR_BANK: usize = 8 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authored {
    pub format: String,
    /// The source name the model's slots are filled from (`from.source`):
    /// the template's id.
    pub source: String,
    pub title: Option<String>,
    #[serde(rename = "asset")]
    pub assets: Vec<AuthoredAsset>,
    /// The text of the files tiles assets name (`file`), by name, read by
    /// [`Authored::load`] from the template directory.
    #[serde(skip)]
    pub files: BTreeMap<String, String>,
}

/// One authored asset, in a lifted format's terms.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "format", deny_unknown_fields)]
pub enum AuthoredAsset {
    /// Pattern tiles, generated: `generate = "stub"` makes each tile show
    /// its number (crate::stub), licensed CC0-1.0. Or drawn: `file` names
    /// a text picture in the template directory, `tiles_per_row` tiles
    /// wide, one character a pixel, `0` to `3` for the four colors and `.`
    /// for 0; lines starting with `#` are comments.
    #[serde(rename = "chr-2bpp")]
    Tiles {
        name: String,
        tiles: u32,
        #[serde(default = "sixteen")]
        tiles_per_row: u32,
        generate: Option<Generate>,
        file: Option<String>,
        /// The tile numbers that stand for characters, as the map's
        /// `charset`: runs of a first tile and the characters from it.
        #[serde(default)]
        charset: Vec<(u8, String)>,
    },
    /// Palettes of four NES color numbers.
    #[serde(rename = "palette")]
    Palettes {
        name: String,
        palettes: Vec<[u8; 4]>,
    },
    /// Metatiles, each with the character levels draw it with.
    #[serde(rename = "metatiles")]
    Metatiles {
        name: String,
        /// The property names every metatile gives, such as `solid`.
        #[serde(default)]
        properties: Vec<String>,
        #[serde(rename = "metatile")]
        metatiles: Vec<Metatile>,
    },
    /// A level, one character per cell, from its metatiles' characters.
    #[serde(rename = "level-grid")]
    Level {
        name: String,
        metatiles: String,
        rows: Vec<String>,
    },
    /// A table of 16-bit numbers, one row per entry, or of bytes with
    /// `bits = 8`.
    #[serde(rename = "table")]
    Table {
        name: String,
        columns: Vec<String>,
        rows: Vec<Vec<u32>>,
        #[serde(default = "sixteen_bits")]
        bits: u8,
        /// Rows in all: the rows given, then rows of zeros up to it.
        count: Option<u32>,
        /// Single values set after that, each `[row, column, value]`, for
        /// a long table that is mostly zeros.
        #[serde(default)]
        set: Vec<[u32; 3]>,
    },
    /// Rows of `width` bytes each.
    #[serde(rename = "byte-rows")]
    Rows {
        name: String,
        width: u32,
        rows: Vec<Vec<u8>>,
    },
    /// PPU update strings, each a list of updates, ended by the byte 0.
    #[serde(rename = "ppu-strings")]
    Strings {
        name: String,
        /// Runs of a first tile number and the characters from it, as the
        /// map's `charset`, for updates written as `text`.
        #[serde(default)]
        charset: Vec<(u8, String)>,
        #[serde(rename = "string")]
        strings: Vec<AuthoredString>,
    },
}

/// One PPU update string.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredString {
    pub name: String,
    pub note: Option<String>,
    pub updates: Vec<AuthoredUpdate>,
}

/// One update: from PPU address `ppu`, `tiles`, `text` in the charset, or
/// `repeat` times `tile`, across or (`vertical`) down a column.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredUpdate {
    pub ppu: u16,
    #[serde(default)]
    pub vertical: bool,
    pub tiles: Option<Vec<u8>>,
    pub text: Option<String>,
    pub repeat: Option<u8>,
    pub tile: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Generate {
    Stub,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Metatile {
    pub name: String,
    /// The one character a level draws it with.
    pub char: String,
    /// Tile numbers: top left, top right, bottom left, bottom right.
    pub tiles: [u8; 4],
    /// Every name in the asset's `properties`, with its byte.
    #[serde(flatten)]
    pub props: BTreeMap<String, u8>,
}

fn sixteen() -> u32 {
    16
}

fn sixteen_bits() -> u8 {
    16
}

impl AuthoredAsset {
    pub fn name(&self) -> &str {
        match self {
            AuthoredAsset::Tiles { name, .. }
            | AuthoredAsset::Palettes { name, .. }
            | AuthoredAsset::Metatiles { name, .. }
            | AuthoredAsset::Level { name, .. }
            | AuthoredAsset::Table { name, .. }
            | AuthoredAsset::Rows { name, .. }
            | AuthoredAsset::Strings { name, .. } => name,
        }
    }
}

/// A built image: the iNES file, its headerless SHA-256 and the asset map
/// that describes it, as text.
#[derive(Debug, Clone)]
pub struct Built {
    pub image: Vec<u8>,
    pub sha256: String,
    pub map_text: String,
    /// Bytes of PRG and CHR the assets fill.
    pub prg_used: usize,
    pub chr_used: usize,
}

fn bad(msg: impl Into<String>) -> Error {
    Error::Map(format!("{AUTHORED_FILE}: {}", msg.into()))
}

fn format_readable(f: &str) -> bool {
    let parse = |s: &str| -> Option<(u32, u32, u32)> {
        let mut it = s.split('.').map(|p| p.parse::<u32>().ok());
        let v = (it.next()??, it.next()??, it.next()??);
        it.next().is_none().then_some(v)
    };
    matches!((parse(f), parse(FORMAT)), (Some((a, b, c)), Some((x, y, z))) if a == x && b == y && c <= z)
}

/// A TOML basic string.
fn q(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Authored {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let a: Authored = toml::from_str(text).map_err(|e| bad(e.to_string()))?;
        if !format_readable(&a.format) {
            return Err(bad(format!(
                "format {} isn't readable by this gp (format {FORMAT})",
                a.format
            )));
        }
        Ok(a)
    }

    /// Parses the template directory's `authored.toml` and reads the files
    /// its tiles assets name. A name with `..` or an absolute path is
    /// refused.
    pub fn load(dir: &std::path::Path) -> Result<Self, Error> {
        let path = dir.join(AUTHORED_FILE);
        let text =
            std::fs::read_to_string(&path).map_err(|e| bad(format!("{}: {e}", path.display())))?;
        let mut a = Self::parse(&text)?;
        for asset in &a.assets {
            if let AuthoredAsset::Tiles {
                file: Some(f),
                name,
                ..
            } = asset
            {
                let rel = std::path::Path::new(f);
                if rel.is_absolute()
                    || rel
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err(bad(format!(
                        "{name}: file {f} must be a path inside the template directory"
                    )));
                }
                let t = std::fs::read_to_string(dir.join(rel))
                    .map_err(|e| bad(format!("{name}: {f}: {e}")))?;
                a.files.insert(f.clone(), t);
            }
        }
        Ok(a)
    }

    /// The pixels of a drawn tiles picture, `w` by `h`, color numbers 0
    /// to 3.
    fn picture(&self, name: &str, file: &str, w: u32, h: u32) -> Result<Vec<u8>, Error> {
        let text = self.files.get(file).ok_or_else(|| {
            bad(format!(
                "{name}: file {file} isn't loaded (build with Authored::load)"
            ))
        })?;
        let rows: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .collect();
        if rows.len() != h as usize {
            return Err(bad(format!(
                "{name}: {file} has {} rows of pixels; the tiles take {h}",
                rows.len()
            )));
        }
        let mut out = Vec::with_capacity((w * h) as usize);
        for (y, r) in rows.iter().enumerate() {
            if r.chars().count() != w as usize {
                return Err(bad(format!(
                    "{name}: {file} row {y} is {} pixels wide; the tiles take {w}",
                    r.chars().count()
                )));
            }
            for (x, c) in r.chars().enumerate() {
                out.push(match c {
                    '.' | '0' => 0,
                    '1' => 1,
                    '2' => 2,
                    '3' => 3,
                    _ => {
                        return Err(bad(format!(
                            "{name}: {file} row {y}, column {x} is `{c}`; a pixel is 0 to 3 or ."
                        )));
                    }
                });
            }
        }
        Ok(out)
    }

    /// The legend of a metatiles asset: one character per metatile.
    fn legend(&self, name: &str) -> Option<String> {
        self.assets.iter().find_map(|a| match a {
            AuthoredAsset::Metatiles {
                name: n, metatiles, ..
            } if n == name => Some(metatiles.iter().map(|m| m.char.as_str()).collect()),
            _ => None,
        })
    }

    /// Lays the assets out and writes the image and its asset map.
    pub fn build(&self) -> Result<Built, Error> {
        let mut names = std::collections::BTreeSet::new();
        for a in &self.assets {
            if !names.insert(a.name()) {
                return Err(bad(format!("two assets are named `{}`", a.name())));
            }
        }
        let mut prg: Vec<u8> = Vec::new();
        let mut chr_bytes: Vec<u8> = Vec::new();
        let mut map = String::new();
        let _ = writeln!(
            map,
            "# Asset map for {}, an original template (docs/originals.md). Written by\n\
             # gp template build from the template's {AUTHORED_FILE}; don't edit it by hand.\n\
             # The offsets are into the data image the build makes, which holds the\n\
             # authored assets one after another and no code.",
            self.title.as_deref().unwrap_or(&self.source)
        );
        let _ = writeln!(map, "schema = {}", crate::map::SCHEMA);
        let _ = writeln!(map, "source = {}", q(&self.source));
        if let Some(t) = &self.title {
            let _ = writeln!(map, "title = {}", q(t));
        }
        // The hash goes here once the bytes are known.
        let _ = writeln!(map, "sha256 = \"@SHA256@\"");
        for a in &self.assets {
            map.push('\n');
            map.push_str("[[asset]]\n");
            let _ = writeln!(map, "name = {}", q(a.name()));
            match a {
                AuthoredAsset::Tiles {
                    tiles,
                    tiles_per_row,
                    generate,
                    file,
                    charset,
                    ..
                } => {
                    if *tiles == 0 || *tiles_per_row == 0 || tiles % tiles_per_row != 0 {
                        return Err(bad(format!(
                            "{}: tiles must be a nonzero multiple of tiles_per_row",
                            a.name()
                        )));
                    }
                    let (w, h) = chr::image_size(*tiles, *tiles_per_row);
                    let pixels = match (generate, file) {
                        (Some(Generate::Stub), None) => {
                            let mut pixels = vec![0u8; (w * h) as usize];
                            for t in 0..*tiles {
                                let (tx, ty) = ((t % tiles_per_row) * 8, (t / tiles_per_row) * 8);
                                for (i, p) in stub::tile_pixels(t).iter().enumerate() {
                                    let (x, y) = (i as u32 % 8, i as u32 / 8);
                                    pixels[((ty + y) * w + tx + x) as usize] = *p;
                                }
                            }
                            pixels
                        }
                        (None, Some(f)) => self.picture(a.name(), f, w, h)?,
                        _ => {
                            return Err(bad(format!(
                                "{}: tiles need one of generate = \"stub\" and file",
                                a.name()
                            )));
                        }
                    };
                    let _ = writeln!(
                        map,
                        "format = \"chr-2bpp\"\nregion = \"chr\"\noffset = {:#06x}\ntiles = {tiles}\ntiles_per_row = {tiles_per_row}",
                        chr_bytes.len()
                    );
                    if !charset.is_empty() {
                        let runs: Vec<String> = charset
                            .iter()
                            .map(|(f, r)| format!("[{f:#04x}, {}]", q(r)))
                            .collect();
                        let _ = writeln!(map, "charset = [{}]", runs.join(", "));
                    }
                    chr_bytes.extend(chr::encode(&pixels, w, *tiles, *tiles_per_row));
                }
                AuthoredAsset::Palettes { palettes, .. } => {
                    if palettes.is_empty() {
                        return Err(bad(format!("{}: no palettes", a.name())));
                    }
                    if let Some(c) = palettes.iter().flatten().find(|c| **c > 0x3F) {
                        return Err(bad(format!(
                            "{}: color {c:#04x} isn't an NES color number (0 to 0x3F)",
                            a.name()
                        )));
                    }
                    let _ = writeln!(
                        map,
                        "format = \"palette\"\nregion = \"prg\"\noffset = {:#06x}\ncount = {}",
                        prg.len(),
                        palettes.len()
                    );
                    prg.extend(palettes.iter().flatten());
                }
                AuthoredAsset::Metatiles {
                    properties,
                    metatiles,
                    ..
                } => {
                    if metatiles.is_empty() {
                        return Err(bad(format!("{}: no metatiles", a.name())));
                    }
                    for m in metatiles {
                        if m.char.chars().count() != 1 || m.char == "\"" || m.char == "\\" {
                            return Err(bad(format!(
                                "{}: metatile `{}`'s char must be one character other than a quote or backslash",
                                a.name(),
                                m.name
                            )));
                        }
                        let given: Vec<&String> = m.props.keys().collect();
                        let mut want: Vec<&String> = properties.iter().collect();
                        want.sort();
                        if given != want {
                            return Err(bad(format!(
                                "{}: metatile `{}` gives properties {given:?}; the asset's are {want:?}",
                                a.name(),
                                m.name
                            )));
                        }
                    }
                    let legend: String = metatiles.iter().map(|m| m.char.as_str()).collect();
                    let unique: std::collections::BTreeSet<char> = legend.chars().collect();
                    if unique.len() != metatiles.len() {
                        return Err(bad(format!("{}: two metatiles share a char", a.name())));
                    }
                    let names: Vec<String> = metatiles.iter().map(|m| q(&m.name)).collect();
                    let _ = writeln!(
                        map,
                        "format = \"metatiles\"\nregion = \"prg\"\noffset = {:#06x}\ncount = {}\nnames = [{}]",
                        prg.len(),
                        metatiles.len(),
                        names.join(", ")
                    );
                    for m in metatiles {
                        prg.extend(m.tiles);
                    }
                    if !properties.is_empty() {
                        map.push_str("property = [\n");
                        for p in properties {
                            let _ = writeln!(
                                map,
                                "  {{ name = {}, region = \"prg\", offset = {:#06x} }},",
                                q(p),
                                prg.len()
                            );
                            prg.extend(metatiles.iter().map(|m| m.props[p]));
                        }
                        map.push_str("]\n");
                    }
                }
                AuthoredAsset::Level {
                    metatiles, rows, ..
                } => {
                    let legend = self.legend(metatiles).ok_or_else(|| {
                        bad(format!(
                            "{}: no metatiles asset named `{metatiles}`",
                            a.name()
                        ))
                    })?;
                    let width = rows.first().map_or(0, |r| r.chars().count());
                    if width == 0 {
                        return Err(bad(format!("{}: no rows", a.name())));
                    }
                    let _ = writeln!(
                        map,
                        "format = \"level-grid\"\nregion = \"prg\"\noffset = {:#06x}\nwidth = {width}\nheight = {}\nlegend = {}\nmetatiles = {}",
                        prg.len(),
                        rows.len(),
                        q(&legend),
                        q(metatiles)
                    );
                    for (r, row) in rows.iter().enumerate() {
                        if row.chars().count() != width {
                            return Err(bad(format!(
                                "{}: row {r} is {} cells wide, row 0 is {width}",
                                a.name(),
                                row.chars().count()
                            )));
                        }
                        for (c, ch) in row.chars().enumerate() {
                            let i = legend.chars().position(|l| l == ch).ok_or_else(|| {
                                bad(format!(
                                    "{}: row {r}, column {c} is `{ch}`, which no metatile of `{metatiles}` draws",
                                    a.name()
                                ))
                            })?;
                            prg.push(i as u8);
                        }
                    }
                }
                AuthoredAsset::Table {
                    columns,
                    rows,
                    bits,
                    count,
                    set,
                    ..
                } => {
                    let mut rows = rows.clone();
                    if let Some(n) = count {
                        if (*n as usize) < rows.len() {
                            return Err(bad(format!(
                                "{}: count {n} is fewer than the {} rows given",
                                a.name(),
                                rows.len()
                            )));
                        }
                        rows.resize(*n as usize, vec![0; columns.len()]);
                    }
                    for [r, c, v] in set {
                        let cell = rows
                            .get_mut(*r as usize)
                            .and_then(|row| row.get_mut(*c as usize))
                            .ok_or_else(|| {
                                bad(format!(
                                    "{}: set [{r}, {c}, {v}] is outside the table",
                                    a.name()
                                ))
                            })?;
                        *cell = *v;
                    }
                    let rows = &rows;
                    if *bits != 8 && *bits != 16 {
                        return Err(bad(format!("{}: bits must be 8 or 16", a.name())));
                    }
                    let most = if *bits == 8 { 0xFF } else { 0xFFFF };
                    if columns.is_empty() || rows.is_empty() {
                        return Err(bad(format!("{}: a table needs columns and rows", a.name())));
                    }
                    for (i, r) in rows.iter().enumerate() {
                        if r.len() != columns.len() {
                            return Err(bad(format!(
                                "{}: row {i} has {} values for {} columns",
                                a.name(),
                                r.len(),
                                columns.len()
                            )));
                        }
                        if let Some(v) = r.iter().find(|v| **v > most) {
                            return Err(bad(format!(
                                "{}: row {i}: {v} is more than {bits} bits",
                                a.name()
                            )));
                        }
                    }
                    let _ = writeln!(map, "format = \"table\"\ncount = {}", rows.len());
                    map.push_str("column = [\n");
                    for (c, col) in columns.iter().enumerate() {
                        let lo = prg.len();
                        prg.extend(rows.iter().map(|r| (r[c] & 0xFF) as u8));
                        if *bits == 8 {
                            let _ = writeln!(
                                map,
                                "  {{ name = {}, region = \"prg\", offset = {lo:#06x} }},",
                                q(col)
                            );
                            continue;
                        }
                        let hi = prg.len();
                        prg.extend(rows.iter().map(|r| (r[c] >> 8) as u8));
                        let _ = writeln!(
                            map,
                            "  {{ name = {}, region = \"prg\", offset = {lo:#06x}, hi_offset = {hi:#06x} }},",
                            q(col)
                        );
                    }
                    map.push_str("]\n");
                }
                AuthoredAsset::Rows { width, rows, .. } => {
                    if *width == 0 || rows.is_empty() {
                        return Err(bad(format!(
                            "{}: byte rows need a width and rows",
                            a.name()
                        )));
                    }
                    if let Some(i) = rows.iter().position(|r| r.len() != *width as usize) {
                        return Err(bad(format!(
                            "{}: row {i} has {} bytes; the width is {width}",
                            a.name(),
                            rows[i].len()
                        )));
                    }
                    let _ = writeln!(
                        map,
                        "format = \"byte-rows\"\nregion = \"prg\"\noffset = {:#06x}\ncount = {}\nwidth = {width}",
                        prg.len(),
                        rows.len()
                    );
                    prg.extend(rows.iter().flatten());
                }
                AuthoredAsset::Strings {
                    charset, strings, ..
                } => {
                    if strings.is_empty() {
                        return Err(bad(format!("{}: no strings", a.name())));
                    }
                    let tile_of = |c: char| {
                        charset.iter().find_map(|(first, run)| {
                            run.chars().position(|r| r == c).map(|i| first + i as u8)
                        })
                    };
                    let mut bytes = Vec::new();
                    for s in strings {
                        for (i, u) in s.updates.iter().enumerate() {
                            let at = format!("{}: string {}, update {i}", a.name(), s.name);
                            if u.ppu >> 8 == 0 {
                                return Err(bad(format!(
                                    "{at}: ppu {:#06x} starts with the end byte 0",
                                    u.ppu
                                )));
                            }
                            let kinds = [u.tiles.is_some(), u.text.is_some(), u.repeat.is_some()]
                                .iter()
                                .filter(|k| **k)
                                .count();
                            if kinds != 1 || u.repeat.is_some() != u.tile.is_some() {
                                return Err(bad(format!(
                                    "{at}: needs one of tiles, text, or repeat with tile"
                                )));
                            }
                            let ctrl = |n: usize, rep: bool| {
                                (u.vertical as u8) << 7 | (rep as u8) << 6 | n as u8
                            };
                            bytes.extend([(u.ppu >> 8) as u8, u.ppu as u8]);
                            let data = match (&u.tiles, &u.text) {
                                (Some(t), _) => t.clone(),
                                (_, Some(t)) => t
                                    .chars()
                                    .map(|c| {
                                        tile_of(c).ok_or_else(|| {
                                            bad(format!("{at}: {c:?} isn't in the charset"))
                                        })
                                    })
                                    .collect::<Result<Vec<u8>, Error>>()?,
                                _ => Vec::new(),
                            };
                            if let (Some(n), Some(t)) = (u.repeat, u.tile) {
                                if n == 0 || n > 0x3F {
                                    return Err(bad(format!("{at}: repeat is 1 to 63")));
                                }
                                bytes.extend([ctrl(n as usize, true), t]);
                            } else {
                                if data.is_empty() || data.len() > 0x3F {
                                    return Err(bad(format!(
                                        "{at}: an update holds 1 to 63 bytes"
                                    )));
                                }
                                bytes.push(ctrl(data.len(), false));
                                bytes.extend(&data);
                            }
                        }
                        bytes.push(0);
                    }
                    let _ = writeln!(
                        map,
                        "format = \"ppu-strings\"\nregion = \"prg\"\noffset = {:#06x}\nsize = {}\nend = 0x00",
                        prg.len(),
                        bytes.len()
                    );
                    if !charset.is_empty() {
                        let runs: Vec<String> = charset
                            .iter()
                            .map(|(f, r)| format!("[{f:#04x}, {}]", q(r)))
                            .collect();
                        let _ = writeln!(map, "charset = [{}]", runs.join(", "));
                    }
                    map.push_str("entry = [\n");
                    for s in strings {
                        match &s.note {
                            Some(n) => {
                                let _ = writeln!(
                                    map,
                                    "  {{ name = {}, note = {} }},",
                                    q(&s.name),
                                    q(n)
                                );
                            }
                            None => {
                                let _ = writeln!(map, "  {{ name = {} }},", q(&s.name));
                            }
                        }
                    }
                    map.push_str("]\n");
                    prg.extend(bytes);
                }
            }
        }
        let (prg_used, chr_used) = (prg.len(), chr_bytes.len());
        let banks = |n: usize, bank: usize| n.div_ceil(bank).max(1);
        let prg_banks = banks(prg_used, PRG_BANK);
        let chr_banks = banks(chr_used, CHR_BANK);
        if prg_banks > 255 || chr_banks > 255 {
            return Err(bad("the assets don't fit an iNES image"));
        }
        prg.resize(prg_banks * PRG_BANK, 0);
        chr_bytes.resize(chr_banks * CHR_BANK, 0);
        let mut image = b"NES\x1a".to_vec();
        image.push(prg_banks as u8);
        image.push(chr_banks as u8);
        image.extend([0u8; 10]);
        image.extend(&prg);
        image.extend(&chr_bytes);
        let sha256 = gp_evidence::source::identify_nes(&image)?.sha256;
        let map_text = map.replace("@SHA256@", &sha256);
        Ok(Built {
            image,
            sha256,
            map_text,
            prg_used,
            chr_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AssetMap;

    const TEXT: &str = r#"
format = "0.1.0"
source = "made-up"
title = "Made Up"

[[asset]]
name = "tiles"
format = "chr-2bpp"
tiles = 32
generate = "stub"

[[asset]]
name = "palettes"
format = "palette"
palettes = [[0x21, 0x0F, 0x10, 0x30], [0x21, 0x06, 0x16, 0x36]]

[[asset]]
name = "metatiles"
format = "metatiles"
properties = ["solid"]
metatile = [
  { name = "air", char = ".", tiles = [0, 0, 0, 0], solid = 0 },
  { name = "floor", char = "=", tiles = [1, 2, 3, 4], solid = 1 },
]

[[asset]]
name = "level"
format = "level-grid"
metatiles = "metatiles"
rows = ["....", "=..=", "===="]

[[asset]]
name = "walkers"
format = "table"
columns = ["start", "end"]
rows = [[64, 300], [400, 512]]
"#;

    #[test]
    fn an_authored_file_builds_the_same_image_and_map_every_time() {
        let a = Authored::parse(TEXT).unwrap();
        let one = a.build().unwrap();
        let two = a.build().unwrap();
        assert_eq!(one.image, two.image);
        assert_eq!(one.map_text, two.map_text);
        assert_eq!(one.image.len(), 16 + PRG_BANK + CHR_BANK);
        AssetMap::parse(&one.map_text).unwrap();
    }

    #[test]
    fn the_lift_of_the_image_is_the_authored_data() {
        let built = Authored::parse(TEXT).unwrap().build().unwrap();
        let map = AssetMap::parse(&built.map_text).unwrap();
        let (files, _) = crate::lift(&built.image, &map, &built.map_text).unwrap();
        let get = |n: &str| {
            String::from_utf8(files.iter().find(|f| f.name == n).unwrap().bytes.clone()).unwrap()
        };
        let level = get("level.toml");
        assert!(level.contains("\"=..=\""), "{level}");
        let table = get("walkers.toml");
        assert!(table.contains("end = 300"), "{table}");
        assert!(table.contains("start = 400"), "{table}");
        let mt = get("metatiles.toml");
        assert!(mt.contains("solid = 1"), "{mt}");
        // And it packs back to the image's bytes.
        let read = |n: &str| {
            files
                .iter()
                .find(|f| f.name == n)
                .map(|f| f.bytes.clone())
                .ok_or_else(|| std::io::Error::other(n.to_owned()))
        };
        let encoded = crate::encode(&map, &read).unwrap();
        let diffs = crate::differences(&built.image, &map, &encoded).unwrap();
        assert!(diffs.is_empty());
    }

    #[test]
    fn a_cell_no_metatile_draws_is_refused() {
        let text = TEXT.replace("\"=..=\"", "\"=..X\"");
        let e = Authored::parse(&text).unwrap().build().unwrap_err();
        assert!(e.to_string().contains("`X`"), "{e}");
    }

    #[test]
    fn a_missing_property_is_refused() {
        let text = TEXT.replace(", solid = 1 }", " }");
        let e = Authored::parse(&text).unwrap().build().unwrap_err();
        assert!(e.to_string().contains("properties"), "{e}");
    }

    const MORE: &str = r#"
format = "0.1.1"
source = "made-up"

[[asset]]
name = "art"
format = "chr-2bpp"
tiles = 2
tiles_per_row = 2
file = "art.txt"
charset = [[0x00, "0"], [0x01, " "]]

[[asset]]
name = "boxes"
format = "byte-rows"
width = 3
rows = [[1, 2, 3], [250, 0, 7]]

[[asset]]
name = "steps"
format = "table"
bits = 8
count = 3
set = [[2, 0, 4]]
columns = ["way", "frames"]
rows = [[1, 200], [8, 96]]

[[asset]]
name = "screens"
format = "ppu-strings"
charset = [[0x00, "0123456789"], [0x0A, "ABCDEFGHIJKLMNOPQRSTUVWXYZ"], [0x24, " "]]
string = [
  { name = "one", updates = [{ ppu = 0x2040, text = "HELLO 1" }, { ppu = 0x2081, vertical = true, repeat = 5, tile = 0x3F }] },
  { name = "two", note = "the second", updates = [{ ppu = 0x3F00, tiles = [0x0F, 0x30] }] },
]
"#;

    fn more() -> Authored {
        let mut a = Authored::parse(MORE).unwrap();
        let mut art = String::from("# two tiles\n");
        for y in 0..8 {
            let row: String = (0..16)
                .map(|x| char::from(b'0' + ((x + y) % 4) as u8))
                .collect();
            art.push_str(&row);
            art.push('\n');
        }
        a.files.insert("art.txt".into(), art);
        a
    }

    #[test]
    fn rows_bytes_tables_strings_and_drawn_tiles_lift_and_pack_back() {
        let built = more().build().unwrap();
        let map = AssetMap::parse(&built.map_text).unwrap();
        let (files, _) = crate::lift(&built.image, &map, &built.map_text).unwrap();
        let get = |n: &str| {
            String::from_utf8(files.iter().find(|f| f.name == n).unwrap().bytes.clone()).unwrap()
        };
        assert!(
            get("boxes.toml").contains("[0xFA, 0x00, 0x07]"),
            "{}",
            get("boxes.toml")
        );
        assert!(get("steps.toml").contains("frames = 200"));
        assert!(
            get("steps.toml").contains("# 2\n[[row]]\nway = 4\nframes = 0"),
            "{}",
            get("steps.toml")
        );
        let screens = get("screens.toml");
        assert!(screens.contains("text = \"HELLO 1\""), "{screens}");
        assert!(screens.contains("repeat = 5, tile = 0x3F"), "{screens}");
        let read = |n: &str| {
            files
                .iter()
                .find(|f| f.name == n)
                .map(|f| f.bytes.clone())
                .ok_or_else(|| std::io::Error::other(n.to_owned()))
        };
        let encoded = crate::encode(&map, &read).unwrap();
        assert!(
            crate::differences(&built.image, &map, &encoded)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_drawn_picture_of_the_wrong_size_or_an_unloaded_file_is_refused() {
        let e = Authored::parse(MORE).unwrap().build().unwrap_err();
        assert!(e.to_string().contains("isn't loaded"), "{e}");
        let mut a = more();
        a.files.insert("art.txt".into(), "0123\n".into());
        assert!(
            a.build()
                .unwrap_err()
                .to_string()
                .contains("rows of pixels")
        );
        let text = MORE.replace("rows = [[1, 200]", "rows = [[1, 300]");
        let mut b = Authored::parse(&text).unwrap();
        b.files = more().files;
        assert!(b.build().unwrap_err().to_string().contains("8 bits"));
    }
}
