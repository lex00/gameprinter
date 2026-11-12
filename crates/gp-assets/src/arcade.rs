//! The arcade asset map and lift: a board's graphics chips decoded into
//! pictures a model draws with, from whichever romset the player has.
//!
//! An NES map ([`crate::AssetMap`]) describes one ROM by its SHA-256 and
//! offsets in its PRG and CHR. An arcade game comes in several sets that
//! share a board and a layout but not always their art: Tapper's `tapper`
//! and `tapperg` run the same code with different sprites. So an arcade
//! map names its assets by the board's graphics region (MAME's `gfx1`,
//! `gfx2`) and the index of each tile or sprite in it, and lists the sets
//! whose chips are laid out that way. The lift reads the regions from
//! whichever of those sets the zip is (gp_arcade::Romset finds it by its
//! chips' hashes, and gp_arcade::video::Gfx decodes them; gp-cli joins the
//! two, so this crate, which game modules build, doesn't need gp-arcade),
//! so the same model loads `tapper`'s art or `tapperg`'s, and the lifted
//! folder's `source.toml` says which it was.
//!
//! ```toml
//! schema = 1
//! platform = "arcade"
//! source = "tapper"
//! title = "Tapper"
//! board = "mcr_91490"
//! sets = ["tapper", "tapperg", "tappera", "rbtapper", "sutapper"]
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
    /// MAME's machine configuration the layout is for (Tapper's is
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
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Self::McrTiles => "mcr-tiles",
            Self::McrSprites => "mcr-sprites",
        }
    }

    /// The region it reads and the size of one of its pictures.
    fn region(self) -> (&'static str, u32) {
        match self {
            Self::McrTiles => ("gfx1", 8),
            Self::McrSprites => ("gfx2", 32),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcadeAsset {
    pub name: String,
    pub format: Format,
    /// `gfx1` or `gfx2`, as the format needs.
    pub region: String,
    /// The first tile or sprite, by its index in the region.
    #[serde(default)]
    pub first: u32,
    pub count: u32,
    #[serde(default = "default_per_row")]
    pub per_row: u32,
    /// What some of them show, by index in the region, for people.
    #[serde(default)]
    pub names: std::collections::BTreeMap<String, String>,
}

fn default_per_row() -> u32 {
    16
}

impl ArcadeAsset {
    pub fn file(&self) -> String {
        format!("{}.png", self.name)
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
}

impl Decoded {
    fn picture(&self, f: Format, i: usize) -> Option<&[u8]> {
        match f {
            Format::McrTiles => self.tiles.get(i).map(|t| &t[..]),
            Format::McrSprites => self.sprites.get(i).map(|t| &t[..]),
        }
    }

    fn picture_mut(&mut self, f: Format, i: usize) -> Option<&mut [u8]> {
        match f {
            Format::McrTiles => self.tiles.get_mut(i).map(|t| &mut t[..]),
            Format::McrSprites => self.sprites.get_mut(i).map(|t| &mut t[..]),
        }
    }
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
fn write_png(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
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
    for a in &map.assets {
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
    for a in &map.assets {
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
/// boards with the 91490 CPU board (all of Tapper's): every tile and
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

    const SETS: [&str; 2] = ["tapper", "tapperg"];

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
        }
    }

    /// The map lifts any listed set, its pictures name the set they came
    /// from, two sets with other sprites lift other pictures under the
    /// same names, and each unpacks back to its own regions.
    #[test]
    fn a_map_lifts_any_listed_set_and_unpacks_back() {
        let text = mcr_91490_map("tapper", "Tapper", &SETS);
        let map = ArcadeMap::parse(&text).unwrap();
        assert!(is_arcade(&text));
        let a = decoded("tapper", 0);
        let g = decoded("tapperg", 5);
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
        assert!(String::from_utf8_lossy(&fg[1].bytes).contains("tapperg"));
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

    #[test]
    fn a_map_is_checked() {
        let bad = mcr_91490_map("t", "T", &SETS).replace("region = \"gfx2\"", "region = \"gfx1\"");
        assert!(ArcadeMap::parse(&bad).is_err());
        assert!(!is_arcade("schema = 1\nsource = \"x\"\n"));
    }
}
