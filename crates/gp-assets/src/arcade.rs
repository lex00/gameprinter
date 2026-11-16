//! The arcade asset map and lift: a board's graphics chips decoded into
//! pictures a model draws with, from whichever romset the player has.
//!
//! An NES map ([`crate::AssetMap`]) describes one ROM by its SHA-256 and
//! offsets in its PRG and CHR. An arcade game comes in several sets that
//! share a board and a layout but not always their art: two sets may run
//! the same code with different sprites. So an arcade
//! map names its assets by the board's graphics region (MAME's `gfx1`,
//! `gfx2`) and the index of each tile or sprite in it, and lists the sets
//! whose chips are laid out that way. The lift reads the regions from
//! whichever of those sets the zip is (gp_arcade::Romset finds it by its
//! chips' hashes, and gp_arcade::video::Gfx decodes them; gp-cli joins the
//! two, so this crate, which game modules build, doesn't need gp-arcade),
//! so the same model loads either set's art, and the lifted
//! folder's `source.toml` says which it was.
//!
//! ```toml
//! schema = 1
//! platform = "arcade"
//! source = "madeupa"
//! title = "Made Up"
//! board = "mcr_91490"
//! sets = ["madeupa", "madeupb", "madeupc"]
//!
//! [[asset]]
//! name = "tiles"
//! format = "mcr-tiles"
//! region = "gfx1"
//! first = 0
//! count = 1024
//! per_row = 32
//! ```
//!
//! Formats, both MAME's Midway MCR layouts (mcr.cpp's gfx_mcr, decoded by
//! gp_arcade::video):
//!
//! | Format | Region | What |
//! |---|---|---|
//! | `mcr-tiles` | `gfx1` | Background tiles, 8 by 8, 4 bits a pixel. The board draws each pixel twice in both directions, as 16 by 16 |
//! | `mcr-sprites` | `gfx2` | Sprites, 32 by 32, 4 bits a pixel |
//! | `mcr-font` | `gfx1` | Background tiles that draw text, lifted as `mcr-tiles` are, with a `charset` (crate::charset) whose numbers count from the asset's `first`: the tile a character is drawn with is `first` plus its number. A model's `board-font` slot, the letters a theme or a new game draws text with |
//! | `sprite-set` | none | Sprite frames as layouts of the `mcr-sprites` pictures (crate::sprite_set): `gp sprites capture` writes it from recordings, so it names no region and lifts nothing, as in an NES map |
//! | `byte-rows` | `maincpu` | A data table of the program: `count` rows of `width` bytes from `offset` (a CPU address), lifted as a TOML file of rows like the NES map's `byte-rows`, which hooks read through an entity's `ext_rows`. `[asset.at]` gives its place in a set whose program keeps it elsewhere (another revision's) |
//! | `mcr-screen` | `maincpu` | A whole screen of the background tile layer as the program keeps it to copy into video RAM: 30 rows of 32 cells from `offset`, two bytes a cell (low byte first: the tile's number in bits 0 to 9, flip x 10, flip y 11, colour bank 12 and 13, priority 14 and 15). Lifted as a TOML file of 30 rows of 32 cell words. A model's `board-screen` slot |
//! | `mcr-palette` | `maincpu` | The board's 64 colours as the program keeps them to copy into palette RAM, two bytes a colour from `offset`, unpacked as the program's own loader unpacks them into the board's 9 bits (red, blue, green, 3 each). Lifted as a TOML file of 64 colours. A model's `board-palette` slot |
//!
//! Each asset lifts to one indexed PNG, its tiles or sprites left to right,
//! `per_row` to a row, each pixel its 4-bit value (0 to 15) with gray
//! previews. The colours aren't in the chips: the board looks them up in
//! its 64-entry palette RAM, which the game writes as it runs, so a model
//! does the same at run time. A tile's pixel shows palette entry
//! `16 * (its colour bits) + value`; a sprite's shows `16 * (3 - its
//! colour bits) + value`, and draws nothing where `value & 7` is 0.
//!
//! The lift is checked by its round trip: the pictures packed back into
//! the chips' bytes ([`encode`]) must equal the set's, byte for byte.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::{Error, File, Lifted, MAP_FILE};

/// The file in a lifted folder naming the set the art came from.
pub const SOURCE_FILE: &str = "source.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcadeMap {
    pub schema: u32,
    /// `arcade`.
    pub platform: String,
    pub source: String,
    pub title: Option<String>,
    /// MAME's machine configuration the layout is for (such as
    /// `mcr_91490`), for people.
    pub board: Option<String>,
    /// The MAME sets whose graphics chips this map fits.
    pub sets: Vec<String>,
    #[serde(rename = "asset")]
    pub assets: Vec<ArcadeAsset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Format {
    #[serde(rename = "mcr-tiles")]
    McrTiles,
    #[serde(rename = "mcr-sprites")]
    McrSprites,
    /// Written by `gp sprites capture`, not lifted.
    #[serde(rename = "sprite-set")]
    SpriteSet,
    /// A data table of the program, `count` rows of `width` bytes from
    /// `offset` in a code region (`maincpu`), lifted as the NES map's
    /// `byte-rows` are (a TOML file of rows), for hooks to read
    /// (`ext_rows`) from the player's own romset.
    #[serde(rename = "byte-rows")]
    ByteRows,
    /// A whole screen of the background tile layer, 30 rows of 32 cells
    /// of two bytes from `offset` in `maincpu`: what the program copies
    /// into video RAM. A model's `board-screen` slot.
    #[serde(rename = "mcr-screen")]
    McrScreen,
    /// The board's 64 colours, two bytes each from `offset` in `maincpu`,
    /// as the program copies them into palette RAM. A model's
    /// `board-palette` slot.
    #[serde(rename = "mcr-palette")]
    McrPalette,
    /// Background tiles that draw text (`gfx1`, as `mcr-tiles`), with a
    /// `charset` counted from `first`. A model's `board-font` slot.
    #[serde(rename = "mcr-font")]
    McrFont,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Self::McrTiles => "mcr-tiles",
            Self::McrSprites => "mcr-sprites",
            Self::SpriteSet => "sprite-set",
            Self::ByteRows => "byte-rows",
            Self::McrScreen => "mcr-screen",
            Self::McrPalette => "mcr-palette",
            Self::McrFont => "mcr-font",
        }
    }

    /// The region it reads and the size of one of its pictures (none for
    /// a sprite-set).
    fn region(self) -> (&'static str, u32) {
        match self {
            Self::McrTiles | Self::McrFont => ("gfx1", 8),
            Self::McrSprites => ("gfx2", 32),
            Self::SpriteSet => ("", 0),
            Self::ByteRows | Self::McrScreen | Self::McrPalette => ("maincpu", 0),
        }
    }

    /// Whether it is a table of the program, lifted as a TOML file.
    pub fn table(self) -> bool {
        matches!(self, Self::ByteRows | Self::McrScreen | Self::McrPalette)
    }

    /// Whether it is pictures of a graphics region.
    pub fn picture(self) -> bool {
        matches!(self, Self::McrTiles | Self::McrSprites | Self::McrFont)
    }

    /// Whether the lift writes pictures for it.
    pub fn lifted(self) -> bool {
        self != Self::SpriteSet
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcadeAsset {
    pub name: String,
    pub format: Format,
    /// `gfx1` or `gfx2`, as the format needs (none for a sprite-set).
    #[serde(default)]
    pub region: String,
    /// The first tile or sprite, by its index in the region.
    #[serde(default)]
    pub first: u32,
    #[serde(default)]
    pub count: u32,
    /// What writes a sprite-set, for people: `gp sprites capture`.
    #[serde(default)]
    pub written_by: Option<String>,
    #[serde(default = "default_per_row")]
    pub per_row: u32,
    /// What some of them show, by index in the region, for people.
    #[serde(default)]
    pub names: std::collections::BTreeMap<String, String>,
    /// A `byte-rows` table's place in its region (a CPU address there).
    #[serde(default)]
    pub offset: u32,
    /// A `byte-rows` table's bytes a row.
    #[serde(default)]
    pub width: u32,
    /// A `byte-rows` table's place in a set whose program keeps it
    /// elsewhere, by the set's name (`madeupb = 0xC3F1`): the sets share
    /// a layout of pictures, not always of code. The public page's lift
    /// takes a set of another program only when every program table names
    /// it here, one kept at the same place with `offset` again
    /// (gp_web_lift::arcade::tables_unplaced).
    #[serde(default)]
    pub at: std::collections::BTreeMap<String, u32>,
    /// An `mcr-font`'s text encoding: each character's tile, counted from
    /// `first` (crate::charset's runs, `[[0x30, "0123456789"]]`).
    #[serde(default)]
    pub charset: Option<crate::charset::Charset>,
}

fn default_per_row() -> u32 {
    16
}

impl ArcadeAsset {
    /// An `mcr-font`'s tile in its region for a character: `first` plus
    /// the charset's number.
    pub fn glyph(&self, c: char) -> Option<u32> {
        self.charset
            .as_ref()
            .and_then(|cs| cs.tile(c))
            .map(|t| self.first + u32::from(t))
    }

    pub fn file(&self) -> String {
        if self.format == Format::SpriteSet || self.format.table() {
            format!("{}.toml", self.name)
        } else {
            format!("{}.png", self.name)
        }
    }

    /// The PNG's size.
    pub fn image_size(&self) -> (u32, u32) {
        let (_, side) = self.format.region();
        let per_row = self.per_row.min(self.count).max(1);
        (
            per_row * side,
            self.count.div_ceil(self.per_row.max(1)) * side,
        )
    }
}

/// Whether a map's text is an arcade map (`platform = "arcade"`).
pub fn is_arcade(map_text: &str) -> bool {
    toml::from_str::<toml::Table>(map_text)
        .ok()
        .and_then(|t| {
            t.get("platform")
                .and_then(|p| p.as_str().map(str::to_owned))
        })
        .is_some_and(|p| p == "arcade")
}

impl ArcadeMap {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let m: Self = toml::from_str(text).map_err(|e| Error::Map(e.to_string()))?;
        if m.schema != crate::map::SCHEMA {
            return Err(Error::Map(format!(
                "schema {} isn't supported (this gp reads {})",
                m.schema,
                crate::map::SCHEMA
            )));
        }
        if m.platform != "arcade" {
            return Err(Error::Map(format!(
                "platform {:?}: an arcade map says \"arcade\"",
                m.platform
            )));
        }
        if m.sets.is_empty() {
            return Err(Error::Map("`sets` names no set".into()));
        }
        let mut names = BTreeSet::new();
        for a in &m.assets {
            if !names.insert(a.name.as_str()) || a.name == "source" || a.name == "assets" {
                return Err(Error::Map(format!("asset name {:?} is used twice", a.name)));
            }
            let (region, _) = a.format.region();
            if (a.format == Format::McrFont) != a.charset.is_some() {
                return Err(Error::Map(format!(
                    "{}: {}",
                    a.name,
                    if a.format == Format::McrFont {
                        "an mcr-font names its charset"
                    } else {
                        "only an mcr-font has a charset"
                    }
                )));
            }
            if let Some(cs) = &a.charset {
                cs.validate(&a.name).map_err(Error::Map)?;
                if let Some(past) = cs.tiles().find(|t| u32::from(*t) >= a.count) {
                    return Err(Error::Map(format!(
                        "{}: the charset's tile {past:#04x} is past the font's {} tiles",
                        a.name, a.count
                    )));
                }
            }
            if a.format == Format::SpriteSet {
                if !a.region.is_empty() || a.count != 0 {
                    return Err(Error::Map(format!(
                        "{}: a sprite-set names no region or count; gp sprites capture writes it",
                        a.name
                    )));
                }
                continue;
            }
            if matches!(a.format, Format::McrScreen | Format::McrPalette) {
                if a.region != region || a.count != 0 || a.width != 0 {
                    return Err(Error::Map(format!(
                        "{}: {} names region {region} and its offset, and no count or width (its size is the board's)",
                        a.name,
                        a.format.name()
                    )));
                }
                continue;
            }
            if a.format == Format::ByteRows {
                if a.region != region || a.count == 0 || a.width == 0 {
                    return Err(Error::Map(format!(
                        "{}: a byte-rows table names region {region}, its offset, and count and width of at least 1",
                        a.name
                    )));
                }
                continue;
            }
            if a.region != region {
                return Err(Error::Map(format!(
                    "{}: {} reads region {region}, not {}",
                    a.name,
                    a.format.name(),
                    a.region
                )));
            }
            if a.count == 0 || a.per_row == 0 {
                return Err(Error::Map(format!(
                    "{}: count and per_row must be at least 1",
                    a.name
                )));
            }
        }
        Ok(m)
    }

    /// Checks the set is one the map fits.
    pub fn fits(&self, set: &str) -> Result<(), Error> {
        if self.sets.iter().any(|s| s == set) {
            Ok(())
        } else {
            Err(Error::Map(format!(
                "the romset is {set}, and this map is for {}",
                self.sets.join(", ")
            )))
        }
    }
}

/// A set's graphics regions decoded (gp_arcade::video::Gfx's): 8 by 8
/// background tiles and 32 by 32 sprites, one value (0 to 15) a pixel.
pub struct Decoded {
    /// MAME's name for the set, and its identity.
    pub set: String,
    pub identity: String,
    pub tiles: Vec<[u8; 64]>,
    pub sprites: Vec<[u8; 1024]>,
    /// The program regions by MAME's name (`maincpu`), for `byte-rows`.
    pub regions: std::collections::BTreeMap<String, Vec<u8>>,
}

impl Decoded {
    fn picture(&self, f: Format, i: usize) -> Option<&[u8]> {
        match f {
            Format::McrTiles | Format::McrFont => self.tiles.get(i).map(|t| &t[..]),
            Format::McrSprites => self.sprites.get(i).map(|t| &t[..]),
            _ => None,
        }
    }

    fn picture_mut(&mut self, f: Format, i: usize) -> Option<&mut [u8]> {
        match f {
            Format::McrTiles | Format::McrFont => self.tiles.get_mut(i).map(|t| &mut t[..]),
            Format::McrSprites => self.sprites.get_mut(i).map(|t| &mut t[..]),
            _ => None,
        }
    }
}

/// A `byte-rows` table's lifted file: its rows from the program region,
/// written as the NES map's `byte-rows` are (crate::rows), so the
/// interpreter reads either the same way.
fn rows_file(a: &ArcadeAsset, gfx: &Decoded, source: &str) -> Result<Vec<u8>, Error> {
    let fail = |msg: String| Error::Decode {
        asset: a.name.clone(),
        msg,
    };
    let region = gfx
        .regions
        .get(&a.region)
        .ok_or_else(|| fail(format!("the romset has no region {}", a.region)))?;
    let from = a.at.get(&gfx.set).copied().unwrap_or(a.offset) as usize;
    let len = (a.count * a.width) as usize;
    let bytes = region.get(from..from + len).ok_or_else(|| {
        fail(format!(
            "{len} bytes from {from:#06x} are past {}",
            a.region
        ))
    })?;
    let mut out = format!(
        "# Rows of {} bytes each: asset {:?} of {source}, lifted from the player's romset.
rows = [\n",
        a.width, a.name
    );
    for (i, row) in bytes.chunks_exact(a.width as usize).enumerate() {
        let items: Vec<String> = row.iter().map(|b| format!("0x{b:02X}")).collect();
        out.push_str(&format!("  [{}], # {i}\n", items.join(", ")));
    }
    out.push_str("]\n");
    Ok(out.into_bytes())
}

/// A table's `len` bytes from its region, at its place in the set.
fn table_bytes<'a>(a: &ArcadeAsset, gfx: &'a Decoded, len: usize) -> Result<&'a [u8], Error> {
    let fail = |msg: String| Error::Decode {
        asset: a.name.clone(),
        msg,
    };
    let region = gfx
        .regions
        .get(&a.region)
        .ok_or_else(|| fail(format!("the romset has no region {}", a.region)))?;
    let from = a.at.get(&gfx.set).copied().unwrap_or(a.offset) as usize;
    region.get(from..from + len).ok_or_else(|| {
        fail(format!(
            "{len} bytes from {from:#06x} are past {}",
            a.region
        ))
    })
}

/// The board's screen: 30 rows of 32 cells.
pub const SCREEN_ROWS: usize = 30;
pub const SCREEN_COLUMNS: usize = 32;

/// An `mcr-screen`'s lifted file: its 960 cells as words, row by row.
fn screen_file(a: &ArcadeAsset, gfx: &Decoded, source: &str) -> Result<Vec<u8>, Error> {
    let bytes = table_bytes(a, gfx, SCREEN_ROWS * SCREEN_COLUMNS * 2)?;
    let mut out = format!(
        "# A screen of the board's background tile layer, 30 rows of 32 cells as video\n\
         # RAM holds them (tile bits 0 to 9, flip x 10, flip y 11, colour bank 12 and 13,\n\
         # priority 14 and 15): asset {:?} of {source}, lifted from the player's romset.\n\
         rows = [\n",
        a.name
    );
    for (i, row) in bytes
        .as_chunks::<{ SCREEN_COLUMNS * 2 }>()
        .0
        .iter()
        .enumerate()
    {
        let cells: Vec<String> = row
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| format!("0x{:04X}", u16::from_le_bytes(*c)))
            .collect();
        out.push_str(&format!("  [{}], # {i}\n", cells.join(", ")));
    }
    out.push_str("]\n");
    Ok(out.into_bytes())
}

/// One colour of an `mcr-palette` table, two bytes, unpacked as the
/// program's own loader does: the first byte's nibbles go to bits 0 to 3 and 3
/// to 6 (or'd), the second byte turned right twice is or'd in less its
/// low bit, and that low bit is the ninth bit (the loader writes the
/// colour's odd byte of palette RAM for it).
pub fn palette_color(lo: u8, hi: u8) -> u16 {
    let c = hi.rotate_right(2);
    let a = ((lo & 0xf0) >> 1) | (lo & 0x0f) | (c & 0xfe);
    u16::from(a) | (u16::from(c & 1) << 8)
}

/// The board's colours.
pub const PALETTE_COLORS: usize = 64;

/// An `mcr-palette`'s lifted file: 64 colours of 9 bits.
fn palette_file(a: &ArcadeAsset, gfx: &Decoded, source: &str) -> Result<Vec<u8>, Error> {
    let bytes = table_bytes(a, gfx, PALETTE_COLORS * 2)?;
    let colors: Vec<String> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| format!("0x{:03X}", palette_color(c[0], c[1])))
        .collect();
    let mut out = format!(
        "# The board's 64 colours, 9 bits each (red 6 to 8, blue 3 to 5, green 0 to 2):\n\
         # asset {:?} of {source}, lifted from the player's romset.\n\
         colors = [\n",
        a.name
    );
    for (i, row) in colors.chunks(16).enumerate() {
        out.push_str(&format!("  {}, # {}\n", row.join(", "), i * 16));
    }
    out.push_str("]\n");
    Ok(out.into_bytes())
}

/// An `mcr-screen`'s lifted file read back: 960 cell words, row by row.
pub fn read_screen(text: &str) -> Result<Vec<u16>, String> {
    #[derive(Deserialize)]
    struct F {
        rows: Vec<Vec<u16>>,
    }
    let f: F = toml::from_str(text).map_err(|e| e.to_string())?;
    if f.rows.len() != SCREEN_ROWS || f.rows.iter().any(|r| r.len() != SCREEN_COLUMNS) {
        return Err(format!(
            "a screen is {SCREEN_ROWS} rows of {SCREEN_COLUMNS} cells"
        ));
    }
    Ok(f.rows.concat())
}

/// An `mcr-palette`'s lifted file read back: 64 colours of 9 bits.
pub fn read_palette(text: &str) -> Result<Vec<u16>, String> {
    #[derive(Deserialize)]
    struct F {
        colors: Vec<u16>,
    }
    let f: F = toml::from_str(text).map_err(|e| e.to_string())?;
    if f.colors.len() != PALETTE_COLORS || f.colors.iter().any(|c| *c > 0x1ff) {
        return Err(format!("a palette is {PALETTE_COLORS} colours of 9 bits"));
    }
    Ok(f.colors)
}

/// One asset's pixels from the decoded regions, laid out as its PNG.
fn pixels(a: &ArcadeAsset, gfx: &Decoded) -> Result<Vec<u8>, Error> {
    let (w, h) = a.image_size();
    let (_, side) = a.format.region();
    let mut out = vec![0u8; (w * h) as usize];
    for k in 0..a.count {
        let i = (a.first + k) as usize;
        let src = gfx.picture(a.format, i).ok_or_else(|| Error::Decode {
            asset: a.name.clone(),
            msg: format!("{} from {} is past the region", a.count, a.first),
        })?;
        let (cx, cy) = ((k % a.per_row) * side, (k / a.per_row) * side);
        for y in 0..side {
            for x in 0..side {
                out[((cy + y) * w + cx + x) as usize] = src[(y * side + x) as usize];
            }
        }
    }
    Ok(out)
}

/// A 4-bit indexed PNG with 16 gray previews: only the values are data.
/// The lift writes its pictures so, and an edit written back into a
/// lifted folder (gp_mcp::board_trip) does the same.
pub fn write_png(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
    let grays: Vec<u8> = (0..16u8).flat_map(|i| [i * 17; 3]).collect();
    let mut packed = Vec::with_capacity((w.div_ceil(2) * h) as usize);
    for row in px.chunks_exact(w as usize) {
        for two in row.chunks(2) {
            packed.push((two[0] << 4) | two.get(1).copied().unwrap_or(0));
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Four);
    enc.set_palette(grays);
    enc.set_compression(png::Compression::Balanced);
    let mut writer = enc.write_header().expect("PNG header into memory");
    writer
        .write_image_data(&packed)
        .expect("PNG data into memory");
    writer.finish().expect("PNG end into memory");
    out
}

/// Decodes every asset of `map` from the set into files: the map itself
/// (`assets.toml`), `source.toml` naming the set, and a PNG an asset.
pub fn lift(
    gfx: &Decoded,
    map: &ArcadeMap,
    map_text: &str,
) -> Result<(Vec<File>, Vec<Lifted>), Error> {
    map.fits(&gfx.set)?;
    let mut files = vec![
        File {
            name: MAP_FILE.into(),
            bytes: map_text.as_bytes().to_vec(),
        },
        File {
            name: SOURCE_FILE.into(),
            bytes: format!(
                "# The romset this folder's pictures were lifted from.\nset = {:?}\nidentity = {:?}\n",
                gfx.set, gfx.identity
            )
            .into_bytes(),
        },
    ];
    let mut lifted = Vec::new();
    for a in map.assets.iter().filter(|a| a.format.table()) {
        let bytes = match a.format {
            Format::McrScreen => screen_file(a, gfx, &map.source)?,
            Format::McrPalette => palette_file(a, gfx, &map.source)?,
            _ => rows_file(a, gfx, &map.source)?,
        };
        lifted.push(Lifted {
            name: a.name.clone(),
            format: a.format.name(),
            file: a.file(),
            bytes: bytes.len() as u64,
        });
        files.push(File {
            name: a.file(),
            bytes,
        });
    }
    for a in map.assets.iter().filter(|a| a.format.picture()) {
        let (w, h) = a.image_size();
        let px = pixels(a, gfx)?;
        let png = write_png(w, h, &px);
        lifted.push(Lifted {
            name: a.name.clone(),
            format: a.format.name(),
            file: a.file(),
            bytes: png.len() as u64,
        });
        files.push(File {
            name: a.file(),
            bytes: png,
        });
    }
    Ok((files, lifted))
}

/// Puts a lifted folder's pictures back into the decoded regions, over
/// the set's own where the map names nothing: packed back into the chips'
/// bytes (gp_arcade::video::Gfx::encode), they must be the set's.
pub fn unpack(
    gfx: &mut Decoded,
    map: &ArcadeMap,
    read: &dyn Fn(&str) -> std::io::Result<Vec<u8>>,
) -> Result<(), Error> {
    for a in map.assets.iter().filter(|a| a.format.picture()) {
        let file = a.file();
        let bytes = read(&file).map_err(|e| Error::File {
            file: file.clone(),
            msg: e.to_string(),
        })?;
        let px = crate::chr::read_png(&bytes, a.image_size()).map_err(|msg| Error::File {
            file: file.clone(),
            msg,
        })?;
        if let Some(i) = px.iter().position(|&p| p > 15) {
            return Err(Error::File {
                file,
                msg: format!("pixel {i} is {}, past 15", px[i]),
            });
        }
        let (w, _) = a.image_size();
        let (_, side) = a.format.region();
        for k in 0..a.count {
            let i = (a.first + k) as usize;
            let (cx, cy) = ((k % a.per_row) * side, (k / a.per_row) * side);
            let dst = gfx.picture_mut(a.format, i).ok_or_else(|| Error::Decode {
                asset: a.name.clone(),
                msg: format!("index {i} is past the region"),
            })?;
            for y in 0..side {
                for x in 0..side {
                    dst[(y * side + x) as usize] = px[((cy + y) * w + cx + x) as usize];
                }
            }
        }
    }
    Ok(())
}

/// The arcade map `gp game new` writes for the sets of Midway's MCR
/// boards with the 91490 CPU board: every tile and
/// sprite.
pub fn mcr_91490_map(source: &str, title: &str, sets: &[&str]) -> String {
    let sets: Vec<String> = sets.iter().map(|s| format!("{s:?}")).collect();
    format!(
        "# Arcade asset map for {title} (gp_assets::arcade in gameprinter): its\n\
         # pictures by graphics region and index, the same in every set listed,\n\
         # lifted from whichever of them the player has. Colours come from the\n\
         # board's palette RAM as the game runs, not from here.\n\
         schema = 1\nplatform = \"arcade\"\nsource = {source:?}\ntitle = {title:?}\nboard = \"mcr_91490\"\n\
         sets = [{}]\n\n\
         # gfx1: 1024 background tiles of 8 by 8, drawn doubled as 16 by 16.\n\
         [[asset]]\nname = \"tiles\"\nformat = \"mcr-tiles\"\nregion = \"gfx1\"\nfirst = 0\ncount = 1024\nper_row = 32\n\n\
         # gfx2: 256 sprites of 32 by 32.\n\
         [[asset]]\nname = \"sprites\"\nformat = \"mcr-sprites\"\nregion = \"gfx2\"\nfirst = 0\ncount = 256\nper_row = 16\n",
        sets.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETS: [&str; 2] = ["madeupa", "madeupd"];

    /// Made-up decoded regions: a pattern, no game's bytes.
    fn decoded(set: &str, salt: u8) -> Decoded {
        Decoded {
            set: set.into(),
            identity: format!("id-{set}"),
            tiles: (0..1024)
                .map(|i| std::array::from_fn(|k| ((i * 7 + k) as u8 ^ salt) & 15))
                .collect(),
            sprites: (0..256)
                .map(|i| std::array::from_fn(|k| ((i * 13 + k * 3) as u8 ^ salt) & 15))
                .collect(),
            regions: [("maincpu".to_owned(), (0..64u8).map(|b| b ^ salt).collect())].into(),
        }
    }

    /// The map lifts any listed set, its pictures name the set they came
    /// from, two sets with other sprites lift other pictures under the
    /// same names, and each unpacks back to its own regions.
    #[test]
    fn a_map_lifts_any_listed_set_and_unpacks_back() {
        let text = mcr_91490_map("madeupa", "Made Up", &SETS);
        let map = ArcadeMap::parse(&text).unwrap();
        assert!(is_arcade(&text));
        let a = decoded("madeupa", 0);
        let g = decoded("madeupd", 5);
        let (fa, la) = lift(&a, &map, &text).unwrap();
        let (fg, _) = lift(&g, &map, &text).unwrap();
        assert_eq!(la.len(), 2);
        let names = |f: &[File]| f.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(&fa),
            ["assets.toml", "source.toml", "tiles.png", "sprites.png"]
        );
        assert_eq!(names(&fa), names(&fg));
        assert_ne!(fa[3].bytes, fg[3].bytes);
        assert!(String::from_utf8_lossy(&fg[1].bytes).contains("madeupd"));
        for (d, files) in [(&a, &fa), (&g, &fg)] {
            let read = |n: &str| {
                files
                    .iter()
                    .find(|f| f.name == n)
                    .map(|f| f.bytes.clone())
                    .ok_or_else(|| std::io::Error::other("missing"))
            };
            let mut back = decoded(&d.set, 0x0a);
            unpack(&mut back, &map, &read).unwrap();
            assert!(back.tiles == d.tiles && back.sprites == d.sprites);
        }
        assert!(lift(&decoded("dkong", 0), &map, &text).is_err());
    }

    /// A byte-rows table lifts its rows from the program region of the
    /// set the player has, as a TOML file of rows; one past the region or
    /// naming a graphics region is refused.
    #[test]
    fn a_byte_rows_table_lifts_from_the_program() {
        let text = format!(
            "{}\n[[asset]]\nname = \"ends\"\nformat = \"byte-rows\"\nregion = \"maincpu\"\noffset = 4\ncount = 2\nwidth = 3\n",
            mcr_91490_map("madeupa", "Made Up", &SETS)
        );
        let map = ArcadeMap::parse(&text).unwrap();
        let (files, lifted) = lift(&decoded("madeupa", 0), &map, &text).unwrap();
        assert_eq!(lifted.len(), 3);
        let moved = text.replace("width = 3\n", "width = 3\n[asset.at]\nmadeupd = 10\n");
        let (g, _) = lift(
            &decoded("madeupd", 0),
            &ArcadeMap::parse(&moved).unwrap(),
            &moved,
        )
        .unwrap();
        let g = g.iter().find(|f| f.name == "ends.toml").unwrap();
        assert!(String::from_utf8_lossy(&g.bytes).contains("0x0A, 0x0B, 0x0C"));
        let f = files.iter().find(|f| f.name == "ends.toml").unwrap();
        let doc: toml::Table = toml::from_str(std::str::from_utf8(&f.bytes).unwrap()).unwrap();
        let rows: Vec<Vec<i64>> = doc["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                r.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_integer().unwrap())
                    .collect()
            })
            .collect();
        assert_eq!(rows, [[4, 5, 6], [7, 8, 9]]);
        let far = text.replace("offset = 4", "offset = 62");
        assert!(
            lift(
                &decoded("madeupa", 0),
                &ArcadeMap::parse(&far).unwrap(),
                &far
            )
            .is_err()
        );
        assert!(
            ArcadeMap::parse(
                &text.replace("region = \"maincpu\"\noffset", "region = \"gfx1\"\noffset")
            )
            .is_err()
        );
    }

    /// A sprite-set entry names no region and lifts nothing; one that
    /// names a region is refused.
    #[test]
    fn a_sprite_set_entry_lifts_nothing() {
        let text = format!(
            "{}\n[[asset]]\nname = \"sprite_set\"\nformat = \"sprite-set\"\nwritten_by = \"gp sprites capture\"\n",
            mcr_91490_map("madeupa", "Made Up", &SETS)
        );
        let map = ArcadeMap::parse(&text).unwrap();
        assert_eq!(map.assets[2].file(), "sprite_set.toml");
        let (files, lifted) = lift(&decoded("madeupa", 0), &map, &text).unwrap();
        assert_eq!(lifted.len(), 2);
        assert!(!files.iter().any(|f| f.name.starts_with("sprite_set")));
        let bad = text.replace(
            "format = \"sprite-set\"",
            "format = \"sprite-set\"\nregion = \"gfx2\"",
        );
        assert!(ArcadeMap::parse(&bad).is_err());
    }

    /// A screen and a palette are tables of the program: made-up bytes
    /// lift to a screen's cell words and the board's 9-bit colours, read
    /// back as they were written, from another place in a set that names
    /// one.
    #[test]
    fn a_screen_and_a_palette_lift_from_the_program() {
        let text = format!(
            "{}\n[[asset]]\nname = \"stage\"\nformat = \"mcr-screen\"\nregion = \"maincpu\"\noffset = 0x10\n\n\
             [[asset]]\nname = \"colors\"\nformat = \"mcr-palette\"\nregion = \"maincpu\"\noffset = 0x800\n\n\
             [asset.at]\nmadeupd = 0x900\n",
            mcr_91490_map("madeupa", "Made Up", &SETS)
        );
        let map = ArcadeMap::parse(&text).unwrap();
        let mut gfx = decoded("madeupa", 0);
        let program: Vec<u8> = (0..0xA00u32).map(|i| (i * 7 % 251) as u8).collect();
        gfx.regions.insert("maincpu".into(), program.clone());
        let (files, lifted) = lift(&gfx, &map, &text).unwrap();
        assert_eq!(lifted.len(), 4);
        let file = |n: &str| {
            let f = files.iter().find(|f| f.name == n).unwrap();
            String::from_utf8(f.bytes.clone()).unwrap()
        };
        let cells = read_screen(&file("stage.toml")).unwrap();
        assert_eq!(cells.len(), 960);
        assert_eq!(cells[0], u16::from_le_bytes([program[0x10], program[0x11]]));
        assert_eq!(
            cells[959],
            u16::from_le_bytes([program[0x10 + 1918], program[0x10 + 1919]])
        );
        let colors = read_palette(&file("colors.toml")).unwrap();
        assert_eq!(colors[1], palette_color(program[0x802], program[0x803]));
        // The program's loader: the first byte's high nibble goes a bit down
        // and its low nibble stays ($FF $00 is 0x07F); the second byte
        // turned right twice gives bit 7 up ($00 $02 is 0x080) and, from
        // its low bit, the ninth ($00 $04 is 0x100); $12 $08 is 0x08 or
        // 0x02 or 0x02, 0x00A.
        assert_eq!(palette_color(0xFF, 0x00), 0x07F);
        assert_eq!(palette_color(0x00, 0x02), 0x080);
        assert_eq!(palette_color(0x00, 0x04), 0x100);
        assert_eq!(palette_color(0x12, 0x08), 0x00A);
        // madeupd's table is elsewhere.
        let mut g = gfx;
        g.set = "madeupd".into();
        let (files, _) = lift(&g, &map, &text).unwrap();
        let f = files.iter().find(|f| f.name == "colors.toml").unwrap();
        let colors = read_palette(std::str::from_utf8(&f.bytes).unwrap()).unwrap();
        assert_eq!(colors[0], palette_color(program[0x900], program[0x901]));
        // Their size is the board's: a count is refused.
        assert!(
            ArcadeMap::parse(&text.replace("offset = 0x10\n", "offset = 0x10\ncount = 2\n"))
                .is_err()
        );
        assert!(read_screen("rows = [[1, 2]]").is_err());
        assert!(read_palette("colors = [0x200]").is_err());
    }

    /// An mcr-font lifts its tiles as mcr-tiles do, from its `first`, and
    /// its charset counts from there; one with no charset, a charset on
    /// another format, or a glyph past its tiles is refused.
    #[test]
    fn a_font_lifts_its_tiles_and_names_its_glyphs() {
        let text = format!(
            "{}\n[[asset]]\nname = \"font\"\nformat = \"mcr-font\"\nregion = \"gfx1\"\nfirst = 0x100\ncount = 16\nper_row = 16\ncharset = [[0, \"0123456789\"], [12, \"AB\"]]\n",
            mcr_91490_map("madeupa", "Made Up", &SETS)
        );
        let map = ArcadeMap::parse(&text).unwrap();
        let font = map.assets.iter().find(|a| a.name == "font").unwrap();
        assert_eq!(font.glyph('7'), Some(0x107));
        assert_eq!(font.glyph('B'), Some(0x10D));
        assert_eq!(font.glyph('C'), None);
        assert_eq!(font.image_size(), (128, 8));
        let gfx = decoded("madeupa", 3);
        let (files, lifted) = lift(&gfx, &map, &text).unwrap();
        assert_eq!(lifted.len(), 3);
        let f = files.iter().find(|f| f.name == "font.png").unwrap();
        let px = crate::chr::read_png(&f.bytes, (128, 8)).unwrap();
        // Glyph 7's tile is the region's 0x107.
        assert_eq!(px[7 * 8 + 1], gfx.tiles[0x107][1]);
        let read = |n: &str| {
            files
                .iter()
                .find(|f| f.name == n)
                .map(|f| f.bytes.clone())
                .ok_or_else(|| std::io::Error::other("missing"))
        };
        let mut back = decoded("madeupa", 9);
        unpack(&mut back, &map, &read).unwrap();
        assert!(back.tiles == gfx.tiles);
        let no_charset = text.replace("charset = [[0, \"0123456789\"], [12, \"AB\"]]\n", "");
        assert!(
            ArcadeMap::parse(&no_charset)
                .unwrap_err()
                .to_string()
                .contains("names its charset")
        );
        let past = text.replace("[12, \"AB\"]", "[15, \"AB\"]");
        assert!(
            ArcadeMap::parse(&past)
                .unwrap_err()
                .to_string()
                .contains("past")
        );
        let on_tiles = mcr_91490_map("madeupa", "Made Up", &SETS)
            .replace("per_row = 32\n", "per_row = 32\ncharset = [[0, \"0\"]]\n");
        assert!(
            ArcadeMap::parse(&on_tiles)
                .unwrap_err()
                .to_string()
                .contains("only an mcr-font")
        );
    }

    #[test]
    fn a_map_is_checked() {
        let bad = mcr_91490_map("t", "T", &SETS).replace("region = \"gfx2\"", "region = \"gfx1\"");
        assert!(ArcadeMap::parse(&bad).is_err());
        assert!(!is_arcade("schema = 1\nsource = \"x\"\n"));
    }
}
