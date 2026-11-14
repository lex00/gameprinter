//! The generated game's pictures and presentation, made at generation
//! time from the lifted assets the model's slots name: each level's
//! picture (its metatiles drawn cell by cell) with a mask for each color
//! that cycles, each sprite frame in each palette set the levels use, the
//! HUD's fixed layer and glyphs, and a [`gp_present::Present`] that says
//! which picture shows when. Presentation only, so floats are fine here;
//! the simulation never reads any of it.
//!
//! Every pixel is drawn with its role (an entity's role for each of its
//! frames, a metatile's or else its palette's item role for a level, `sky`
//! for the backdrop, `hud` for the HUD), so a theme stack (`gp_theme`) can
//! restyle and recolor by role before the pictures are written. With no
//! theme the pictures are the model's own.

use std::collections::BTreeMap;
use std::path::Path;

use gp_assets::map::{Asset, AssetMap, TileOrder};
use gp_assets::sprite_set::SpriteSet;
use gp_model::Model;
use gp_model::model::{AssetKind, Cell, PaletteRef, RowRef, Sprite};
use gp_present::{Frame, Present};
use gp_sim::Spec;
use gp_theme::readability::Samples;
use gp_theme::style::Subject;
use gp_theme::{Resolved, RoleImage, role_id};

/// A picture the generated game loads, by its file name under `assets/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

/// A sprite's picture and whether it mirrors while facing left.
pub type SpritePic = (Picture, bool);

/// An entity type's sprites: its own frame, and one for each state of its
/// machine that has its own, in the start level's palettes. For tools that
/// draw one picture per state (`gp video`'s themed renderer); runtimes draw
/// from [`Art::present`].
#[derive(Debug, Clone, Default)]
pub struct EntityArt {
    pub base: Option<SpritePic>,
    pub states: Vec<Option<SpritePic>>,
}

/// What a runtime draws: the presentation, and its pictures in the order
/// it numbers them.
#[derive(Debug, Clone, Default)]
pub struct Art {
    pub present: Present,
    pub pictures: Vec<Picture>,
    /// The start level's background color.
    pub clear: [u8; 3],
    /// Each level's picture and background color, by level name.
    pub levels: BTreeMap<String, (Picture, [u8; 3])>,
    /// Each entity type's own frames, by entity name.
    pub sprites: BTreeMap<String, EntityArt>,
    /// Each entity type's role, by entity name.
    pub roles: BTreeMap<String, String>,
    /// What the theme stack adds beyond the pictures.
    pub theme: ThemeArt,
}

/// A theme stack's live parts and what its checks measure.
#[derive(Debug, Clone, Default)]
pub struct ThemeArt {
    /// The screen overlay (tint, vignette, scanlines, a CRT, a grid) at
    /// the view's size, as `overlay.png`, for the web preview.
    pub overlay: Option<Picture>,
    /// The overlay for each width a window draws (the visible width and
    /// each widescreen one, `gp_present::view::ASPECTS`), by that width,
    /// at `overlay_scale` overlay pixels to a game pixel, as
    /// `overlay-<width>.png`: what the Bevy build draws.
    pub overlays: Vec<(u32, Picture)>,
    pub overlay_scale: u32,
    /// Particle emitters, with the entity role each follows (None: the
    /// screen).
    pub emitters: Vec<(Option<String>, gp_theme::particles::Emitter)>,
    /// Every themed picture's pixels by role, for the readability check.
    pub samples: Samples,
    /// Every themed sprite, for the Style checks.
    pub subjects: Vec<Subject>,
}

impl Art {
    /// Adds a picture already themed (or a mask, which no theme touches).
    fn add(&mut self, file: String, width: u32, height: u32, rgba: &[u8]) -> u32 {
        let pic = picture(&file, width, height, rgba);
        self.present.pictures.push(gp_present::Picture {
            file,
            size: [width, height],
        });
        self.pictures.push(pic);
        (self.pictures.len() - 1) as u32
    }

    /// Adds a picture drawn with roles, through the theme stack.
    fn add_themed(&mut self, theme: &Resolved, file: String, img: &RoleImage) -> u32 {
        let themed = theme.apply(img);
        self.theme.samples.add(&themed, &img.roles);
        self.add(
            file,
            themed.width,
            themed.height,
            themed.pixels.as_flattened(),
        )
    }

    /// `assets/present.txt`: the presentation as `gp_present::text` writes it.
    pub fn present_text(&self) -> String {
        gp_present::text::write(&self.present)
    }
}

/// A picture as palette entries: palette times 4 plus color, or
/// [`NONE`] where nothing is drawn.
struct Indexed {
    width: u32,
    height: u32,
    px: Vec<u8>,
    /// Each pixel's role.
    roles: Vec<u8>,
    /// Pixels a theme drew whole (a restyled metatile), with the theme
    /// layer's floor; they have no palette entry.
    direct: Vec<Option<([u8; 4], u8)>>,
}

const NONE: u8 = 0xFF;

impl Indexed {
    fn new(width: u32, height: u32) -> Self {
        let n = (width * height) as usize;
        Indexed {
            width,
            height,
            px: vec![NONE; n],
            roles: vec![gp_theme::NO_ROLE; n],
            direct: vec![None; n],
        }
    }

    /// The picture with its roles, colored by `color`, for a theme.
    fn role_image(&self, color: &dyn Fn(u8) -> [u8; 4]) -> RoleImage {
        let mut img = RoleImage::new(self.width, self.height);
        for (i, &e) in self.px.iter().enumerate() {
            let (x, y) = (i as u32 % self.width, i as u32 / self.width);
            match self.direct[i] {
                Some((c, floor)) => img.set(x, y, c, self.roles[i], floor),
                None if e == NONE => {}
                None => img.set(x, y, color(e), self.roles[i], 0),
            }
        }
        img
    }

    /// Draws tile `tile` of a tiles asset at (ox, oy) in palette `palette`,
    /// mirrored as asked, its pixels in role `role`. With `opaque`, color 0
    /// is drawn too, in the role `sky` (the backdrop).
    #[allow(clippy::too_many_arguments)]
    fn tile(
        &mut self,
        t: &Tiles,
        tile: u32,
        ox: u32,
        oy: u32,
        palette: u8,
        flip: (bool, bool),
        opaque: bool,
        role: u8,
    ) {
        let (tx, ty) = ((tile % t.per_row) * 8, (tile / t.per_row) * 8);
        for y in 0..8 {
            for x in 0..8 {
                let (sx, sy) = (
                    if flip.0 { 7 - x } else { x },
                    if flip.1 { 7 - y } else { y },
                );
                let Some(&c) = t.px.get(((ty + sy) * t.width + tx + sx) as usize) else {
                    continue;
                };
                if c == 0 && !opaque {
                    continue;
                }
                let (px, py) = (ox + x, oy + y);
                if px < self.width && py < self.height {
                    let at = (py * self.width + px) as usize;
                    self.px[at] = palette * 4 + c;
                    self.roles[at] = if c == 0 { role_id("sky") } else { role };
                    self.direct[at] = None;
                }
            }
        }
    }

    fn rgba(&self, color: &dyn Fn(u8) -> [u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for &i in &self.px {
            out.extend(if i == NONE { [0, 0, 0, 0] } else { color(i) });
        }
        out
    }

    /// White where the picture has entry `entry`, clear elsewhere.
    fn mask(&self, entry: u8) -> Option<Vec<u8>> {
        if !self.px.contains(&entry) {
            return None;
        }
        Some(self.rgba(&|i| if i == entry { [255; 4] } else { [0; 4] }))
    }
}

/// A tiles asset as color indices.
struct Tiles {
    per_row: u32,
    width: u32,
    px: Vec<u8>,
}

/// A palette set (four background palettes, then four sprite palettes, of
/// four color numbers each) and the background color, when something other
/// than the set's first entry names it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Palettes {
    bytes: Vec<u8>,
    background: Option<u8>,
    /// Each palette's role (the slot's `item_roles`), where it gives them.
    roles: Vec<String>,
}

impl Palettes {
    /// RGBA for entry `entry` (palette times 4 plus color). Color 0 of
    /// every background palette is the background color, as the PPU draws
    /// it, and color 0 of a sprite palette is transparent.
    fn color(&self, rgb: &[[u8; 3]; 64], entry: u8) -> [u8; 4] {
        let (palette, index) = (usize::from(entry / 4), entry % 4);
        if index == 0 && palette >= 4 {
            return [0, 0, 0, 0];
        }
        let n = if index == 0 {
            self.background
                .unwrap_or_else(|| self.bytes.first().copied().unwrap_or(0x0F))
        } else {
            self.bytes
                .get(palette * 4 + usize::from(index))
                .copied()
                .unwrap_or(0x0F)
        };
        let [r, g, b] = rgb[usize::from(n & 0x3F)];
        [r, g, b, 255]
    }

    fn background_rgb(&self, rgb: &[[u8; 3]; 64]) -> [u8; 3] {
        let [r, g, b, _] = self.color(rgb, 0);
        [r, g, b]
    }

    /// Palette `palette`'s role id, or none.
    fn role(&self, palette: usize) -> u8 {
        self.roles
            .get(palette)
            .map_or(gp_theme::NO_ROLE, |r| role_id(r))
    }
}

/// The lifted files of a directory, by name, read as bytes.
struct Files<'a> {
    dir: &'a Path,
    map: AssetMap,
    /// Tiles assets a theme restyles whole, by asset name: the theme's file.
    tiles_from: BTreeMap<String, std::path::PathBuf>,
    /// The same by model slot, which wins over the asset's: two slots
    /// filled from one asset (Original Edition's title and 50m both draw
    /// CHR bank 1) may each have a restyle of their own.
    tiles_from_slot: BTreeMap<String, std::path::PathBuf>,
    /// Layout entries a theme draws as pictures (format 0.1.3), by model
    /// slot and entry: the picture and its layer's floor.
    pictures: BTreeMap<(String, String), (gp_theme::pix::Rgba, u8)>,
}

impl Files<'_> {
    /// Draws a theme's picture of a layout entry over `ix` (format 0.1.3):
    /// its opaque pixels are the theme's, keeping the role the game's
    /// tiles gave them (`fill` where the game drew only the backdrop), and
    /// its clear pixels are the backdrop, or nothing with `clear`.
    fn picture(
        &self,
        slot: &str,
        entry: &str,
        ix: &mut Indexed,
        fill: &str,
        clear: bool,
    ) -> Result<(), String> {
        let Some((img, floor)) = self.pictures.get(&(slot.to_owned(), entry.to_owned())) else {
            return Ok(());
        };
        if (img.width, img.height) != (ix.width, ix.height) {
            return Err(format!(
                "the theme's picture of {slot} {entry} is {} by {}, not {} by {}",
                img.width, img.height, ix.width, ix.height
            ));
        }
        for i in 0..ix.px.len() {
            let c = img.pixels[i];
            let backdrop = ix.px[i] == NONE || ix.px[i].is_multiple_of(4);
            if c[3] >= 128 {
                if backdrop {
                    ix.roles[i] = role_id(fill);
                }
                if ix.px[i] == NONE {
                    ix.px[i] = 1;
                }
                ix.direct[i] = Some(([c[0], c[1], c[2], 255], *floor));
            } else if clear {
                ix.px[i] = NONE;
                ix.direct[i] = None;
            } else {
                ix.px[i] = 0;
                ix.roles[i] = role_id("sky");
                ix.direct[i] = None;
            }
        }
        Ok(())
    }

    fn asset(&self, model: &Model, slot: &str, want: AssetKind) -> Result<&Asset, String> {
        let s = model
            .assets
            .get(slot)
            .ok_or(format!("the model has no asset slot `{slot}`"))?;
        if s.kind != want {
            return Err(format!("asset slot `{slot}` isn't of kind {want:?}"));
        }
        let from = s.from.as_ref().ok_or(format!(
            "asset slot `{slot}` has no `from`, so nothing fills it"
        ))?;
        if from.source != self.map.source {
            return Err(format!(
                "asset slot `{slot}` is filled from `{}`, but the lifted assets are `{}`'s",
                from.source, self.map.source
            ));
        }
        self.map
            .assets
            .iter()
            .find(|a| a.name() == from.asset)
            .ok_or(format!(
                "asset slot `{slot}` names `{}`, which the asset map doesn't have",
                from.asset
            ))
    }

    /// The bytes each span of `asset` packs to, by part name, through the
    /// same encoder `gp assets pack` uses.
    fn parts(&self, asset: &Asset) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let sub = AssetMap {
            assets: vec![asset.clone()],
            ..self.map.clone()
        };
        let read = |file: &str| std::fs::read(self.dir.join(file));
        let encoded = gp_assets::encode(&sub, &read).map_err(|e| e.to_string())?;
        Ok(encoded
            .into_iter()
            .map(|e| (e.span.part, e.bytes))
            .collect())
    }

    /// Row `row` of a `rows` slot (a `byte-rows` asset).
    fn row(&self, model: &Model, slot: &str, row: u32) -> Result<Vec<u8>, String> {
        let asset = self.asset(model, slot, AssetKind::Rows)?;
        let Asset::ByteRows(rows) = asset else {
            return Err(format!("asset slot `{slot}` isn't rows"));
        };
        let bytes = self.part(asset)?;
        let w = rows.width as usize;
        bytes
            .get(row as usize * w..(row as usize + 1) * w)
            .map(<[u8]>::to_vec)
            .ok_or(format!("slot `{slot}` has no row {row}"))
    }

    /// The first column of a `table` slot.
    fn first_column(&self, model: &Model, slot: &str) -> Result<Vec<u8>, String> {
        let asset = self.asset(model, slot, AssetKind::Table)?;
        let Asset::Table { name, columns, .. } = asset else {
            return Err(format!("asset slot `{slot}` isn't a table"));
        };
        let col = columns
            .first()
            .ok_or(format!("table `{name}` has no columns"))?;
        let mut parts = self.parts(asset)?;
        parts
            .remove(&format!("{name}.{}", col.name))
            .or_else(|| parts.remove(&format!("{name}.{} (low bytes)", col.name)))
            .ok_or(format!("table `{name}` decoded to nothing"))
    }

    /// The first column of row `r` of a `table` slot.
    fn table_cell(&self, model: &Model, r: &RowRef) -> Result<u8, String> {
        self.first_column(model, &r.slot)?
            .get(r.row as usize)
            .copied()
            .ok_or(format!("slot `{}` has no row {}", r.slot, r.row))
    }

    /// A palettes slot, with the background color `present` names.
    ///
    /// A slot shorter than a whole set (Super Mario Bros.' snow palettes are
    /// one palette) is written over the start of `base`, as the game writes
    /// it over the area type's set.
    fn palettes(
        &self,
        model: &Model,
        slot: &str,
        background: Option<&RowRef>,
        base: Option<&[u8]>,
        base_roles: &[String],
    ) -> Result<Palettes, String> {
        let mut bytes = self.part(self.asset(model, slot, AssetKind::Palettes)?)?;
        let mut roles = model
            .assets
            .get(slot)
            .map(|s| s.item_roles.clone())
            .unwrap_or_default();
        if bytes.len() < 32 {
            let Some(base) = base.filter(|b| b.len() >= bytes.len()) else {
                return Err(format!(
                    "palettes slot `{slot}` holds {} colors, not a whole set of 32",
                    bytes.len()
                ));
            };
            let mut whole = base.to_vec();
            whole[..bytes.len()].copy_from_slice(&bytes);
            bytes = whole;
            // The palettes it doesn't hold keep the base set's roles.
            let mut all = base_roles.to_vec();
            for (i, r) in roles.iter().enumerate() {
                if let Some(slot) = all.get_mut(i) {
                    *slot = r.clone();
                }
            }
            roles = all;
        }
        Ok(Palettes {
            bytes,
            background: background.map(|b| self.table_cell(model, b)).transpose()?,
            roles,
        })
    }

    /// One palette's four color numbers from a palettes slot.
    fn palette(&self, model: &Model, p: &PaletteRef) -> Result<[u8; 4], String> {
        let bytes = self.part(self.asset(model, &p.slot, AssetKind::Palettes)?)?;
        let at = p.palette as usize * 4;
        bytes
            .get(at..at + 4)
            .map(|b| [b[0], b[1], b[2], b[3]])
            .ok_or(format!(
                "palettes slot `{}` has no palette {}",
                p.slot, p.palette
            ))
    }

    fn part(&self, asset: &Asset) -> Result<Vec<u8>, String> {
        let mut parts = self.parts(asset)?;
        parts
            .remove(asset.name())
            .ok_or(format!("asset `{}` decoded to nothing", asset.name()))
    }

    /// A tiles asset as color indices.
    fn tiles(&self, asset: &Asset) -> Result<Tiles, String> {
        let Asset::Chr { name, .. } = asset else {
            return Err(format!("asset `{}` isn't tiles", asset.name()));
        };
        let file = self
            .tiles_from
            .get(name)
            .cloned()
            .unwrap_or_else(|| self.dir.join(asset.file()));
        self.tiles_in(asset, &file)
    }

    /// Tiles asset `asset`'s pictures from `file`: the lifted PNG or a
    /// theme's restyle of it.
    fn tiles_in(&self, asset: &Asset, file: &std::path::Path) -> Result<Tiles, String> {
        let Asset::Chr {
            name,
            tiles,
            tiles_per_row,
            ..
        } = asset
        else {
            return Err(format!("asset `{}` isn't tiles", asset.name()));
        };
        let size = gp_assets::chr::image_size(*tiles, *tiles_per_row);
        let file = file.to_path_buf();
        let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        // A theme's restyle may be text pixel art (`.pix`), its colors the
        // indices in the order they're listed.
        let px = if file.extension().is_some_and(|e| e == "pix") {
            let text =
                String::from_utf8(bytes).map_err(|_| format!("{}: not text", file.display()))?;
            let (w, h, px) =
                gp_theme::pix::indexed(&text).map_err(|e| format!("{}: {e}", file.display()))?;
            if (w, h) != size {
                return Err(format!(
                    "{}: a restyle of `{name}` is {w} by {h}, not {} by {}",
                    file.display(),
                    size.0,
                    size.1
                ));
            }
            px
        } else {
            gp_assets::chr::read_png(&bytes, size).map_err(|e| format!("{name}: {e}"))?
        };
        Ok(Tiles {
            per_row: *tiles_per_row,
            width: size.0,
            px,
        })
    }

    fn tiles_slot(&self, model: &Model, slot: &str) -> Result<Tiles, String> {
        let asset = self.asset(model, slot, AssetKind::Tiles)?;
        match self.tiles_from_slot.get(slot) {
            Some(file) => self.tiles_in(asset, file),
            None => self.tiles(asset),
        }
    }

    /// A sprite-set slot's file, which `gp sprites capture` writes.
    fn sprite_set(&self, model: &Model, slot: &str) -> Result<SpriteSet, String> {
        let asset = self.asset(model, slot, AssetKind::SpriteSet)?;
        let file = self.dir.join(asset.file());
        let text = std::fs::read_to_string(&file)
            .map_err(|e| format!("{}: {e} (gp sprites capture writes it)", file.display()))?;
        SpriteSet::parse(&text).map_err(|e| format!("{}: {e}", file.display()))
    }
}

/// A clip condition as `gp_present` holds it.
fn present_when(w: &gp_model::model::When, spec: &Spec) -> gp_present::When {
    let word = |t: &gp_model::model::WordTest| (t.index, t.at_least, t.below);
    gp_present::When {
        ground: w.ground,
        moving: w.moving,
        skidding: w.skidding,
        rising: w.rising,
        speed_at_least: w.speed_at_least.map(|v| v.raw()),
        jumped: w.jumped,
        invulnerable: w.invulnerable,
        climbing: w.climbing,
        word: w.word.as_ref().map(word),
        words: w.words.iter().map(word).collect(),
        // `gp model check` refuses a counter the model doesn't have, so
        // one the spec lacks only happens in a model it didn't check.
        counters: w
            .counters
            .iter()
            .filter_map(|t| {
                spec.counters
                    .iter()
                    .position(|c| c.name == t.name)
                    .map(|k| (k, t.at_least, t.below))
            })
            .collect(),
        // By slot number: `name` then the number, while the spec has one.
        slot_counters: w
            .slot_counters
            .iter()
            .map(|t| {
                let by_slot = (0..)
                    .map_while(|s| {
                        let name = format!("{}{s}", t.name);
                        spec.counters.iter().position(|c| c.name == name)
                    })
                    .collect();
                (by_slot, t.at_least, t.below)
            })
            .collect(),
        x: (w.x_at_least, w.x_below),
    }
}

/// The first sprite-set slot of a model, if it has one.
fn set_slot(model: &Model) -> Option<String> {
    model
        .assets
        .iter()
        .find(|(_, s)| s.kind == AssetKind::SpriteSet)
        .map(|(n, _)| n.clone())
}

/// An entity type's own frame, or a state's: (kind, state or none,
/// frame, size, model path).
type Own = (String, Option<usize>, u32, [u32; 2], String);

/// One sprite picture before it's colored: its tiles, layout and colors.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FrameKey {
    Tiles {
        slot: String,
        tiles: Vec<u32>,
        width: u32,
        palette: u32,
        /// Each tile's flips, palette and place.
        cells: Vec<CellKey>,
        /// Colors replacing `palette`'s.
        colors: Option<[u8; 4]>,
        /// The entity's role, on every tile: a theme restyles and recolors
        /// each frame by it.
        role: String,
        /// The sprite's `offset`: its top left from the entity's position.
        offset: Option<[i32; 2]>,
    },
    /// A box of an entity's size, for one with no sprite.
    Box { size: [u32; 2], role: String },
    /// Frame `index` of a sprite-set slot: tiles placed in pixels from the
    /// entity's position.
    Set {
        set: String,
        index: u32,
        /// The tiles slot of pieces that name none.
        slot: String,
        /// The sprite's `colors`, on its `palette`.
        colors: Option<(u32, [u8; 4])>,
        role: String,
    },
}

type CellKey = (bool, bool, Option<u32>, Option<[u32; 2]>);

fn cells_key(cells: &[Cell]) -> Vec<CellKey> {
    cells
        .iter()
        .map(|c| (c.flip_x, c.flip_y, c.palette, c.at))
        .collect()
}

/// Makes the pictures and the presentation: each level with a grid and
/// tileset or a layout, in its own palettes, every sprite frame of every
/// entity type, state and clip in each sprite palette set, and the HUD.
pub fn art(model: &Model, spec: &Spec, dir: &Path) -> Result<Art, String> {
    art_themed(model, spec, dir, &Resolved::none())
}

/// [`art`] with a theme stack applied: restyled art in place of the
/// model's where the stack has it, then palette rules and picture effects
/// by role, and the stack's overlay and particles. A whole-slot restyle of
/// a slot the simulation reads is refused. Masks of cycling colors aren't
/// themed; the colors they're tinted with are, by the `sky` role's rules
/// for a level and the `hud` role's for the HUD.
pub fn art_themed(model: &Model, spec: &Spec, dir: &Path, theme: &Resolved) -> Result<Art, String> {
    art_named(model, spec, dir, theme, None)
}

/// [`art_themed`] with the pictures named as `names` names them: a model,
/// and a spec of it (which may lack levels: only its kinds' order is
/// read), for a model that is `names`' with levels and entity kinds taken
/// out: a ROM loaded only to play another template's model
/// (Donkey Kong (JU) plays Original Edition's without 50m and the
/// stage-clear scene). The sprite palette sets and the HUD's (`frame-N-S`,
/// `hud-S`, `glyph-S-...`) are numbered by `names`' levels and the sprite
/// frames (`frame-N`) by `names`' kinds, so a large-art look made on
/// `names`' pictures fits these too. What `names` has and the model hasn't
/// is drawn as well, under its own name, and nothing draws it.
pub fn art_named(
    model: &Model,
    spec: &Spec,
    dir: &Path,
    theme: &Resolved,
    names: Option<(&Model, &Spec)>,
) -> Result<Art, String> {
    let map_file = dir.join("assets.toml");
    let text =
        std::fs::read_to_string(&map_file).map_err(|e| format!("{}: {e}", map_file.display()))?;
    let map =
        gp_interp::lifted::map_of(&text).map_err(|e| format!("{}: {e}", map_file.display()))?;
    let simulation = gp_interp::lifted::simulation_slots(model);
    let mut tiles_from = BTreeMap::new();
    let mut tiles_from_slot = BTreeMap::new();
    for (name, slot) in &model.assets {
        let Some((_, file, _)) = theme.restyle_slot(name) else {
            continue;
        };
        if simulation.contains(&name) {
            return Err(format!(
                "a theme restyles slot `{name}`, which the simulation reads; themes change presentation only"
            ));
        }
        if slot.kind != AssetKind::Tiles {
            return Err(format!(
                "a theme restyles slot `{name}` whole, but only a tiles slot can be (restyle its items by role instead)"
            ));
        }
        tiles_from_slot.insert(name.clone(), file.clone());
        if let Some(from) = &slot.from {
            tiles_from.insert(from.asset.clone(), file);
        }
    }
    let mut pictures = BTreeMap::new();
    for (name, slot) in &model.assets {
        if !matches!(slot.kind, AssetKind::ScreenText | AssetKind::LevelObjects) {
            continue;
        }
        for (i, l) in theme.layers.iter().enumerate() {
            for r in &l.theme.restyle {
                if r.slot.as_deref() != Some(name.as_str()) || r.target.is_some() {
                    continue;
                }
                let Some(entry) = &r.entry else { continue };
                let file = gp_theme::format::restyle_path(&l.dir, &r.file)?;
                let img = gp_theme::pix::Rgba::load(&file)?;
                pictures.insert((name.clone(), entry.clone()), (img, i as u8 + 1));
            }
        }
    }
    let files = Files {
        dir,
        map,
        tiles_from,
        tiles_from_slot,
        pictures,
    };
    let rgb = nes_colors();
    let mut art = Art::default();
    let themed_rgb = |role: &str, c: [u8; 3]| {
        let gp_theme::color::Rgb(c) = theme.color(Some(role), 0, gp_theme::color::Rgb(c));
        c
    };
    let present = model.present.clone().unwrap_or_default();

    // The screen: the model's, or the first camera's view.
    let view = model
        .cameras
        .values()
        .next()
        .map(|c| c.size)
        .unwrap_or([256, 240]);
    art.present.screen = match &present.screen {
        Some(s) => gp_present::Screen {
            size: s.size,
            aspect: s
                .pixel_aspect
                .map_or([1, 1], |r| [r.num as u32, r.den as u32]),
            overscan: s.overscan.unwrap_or_default(),
            sprite_limit: s.sprite_limit.map(|l| gp_present::SpriteLimit {
                per_line: l.per_line,
                width: l.sprite_width.unwrap_or(8),
                flicker: l.flicker,
            }),
            x_wraps: s.sprite_x_wraps,
        },
        None => gp_present::Screen {
            size: view,
            ..gp_present::Screen::default()
        },
    };

    let default_slot = model
        .assets
        .iter()
        .find(|(_, s)| s.kind == AssetKind::Palettes)
        .map(|(n, _)| n.clone());
    // A model with no palettes slot draws its levels by tile kind; with a
    // sprite-set its sprites are still the set's, in the palettes each
    // level showed.
    let own_sets = match (&default_slot, set_slot(model)) {
        (Some(_), _) => None,
        (None, None) => return Ok(tile_art(spec, art.present.screen.clone())),
        (None, Some(slot)) => Some(files.sprite_set(model, &slot)?),
    };
    let fallback = own_sets
        .as_ref()
        .map(|_| tile_art(spec, art.present.screen.clone()));
    let set_palettes = |level: &str| -> Palettes {
        let sets = own_sets.as_ref().expect("only without a palettes slot");
        let mut bytes = vec![0x0F; 16];
        match sets
            .set_for(level)
            .or(if sets.sets.is_empty() { None } else { Some(0) })
        {
            Some(i) => bytes.extend(&sets.sets[i].colors),
            None => bytes.extend([0x0F, 0x00, 0x10, 0x30].repeat(4)),
        }
        Palettes {
            bytes,
            background: None,
            roles: Vec::new(),
        }
    };
    // A level's palettes: its `present`, else the model's first palettes
    // slot, which a slot shorter than a set is written over.
    let base_set = match &default_slot {
        Some(slot) => files.palettes(model, slot, None, None, &[])?,
        None => set_palettes(""),
    };
    let (base, base_roles) = (base_set.bytes, base_set.roles);
    let level_palettes = |model: &Model, name: &str| -> Result<Palettes, String> {
        if own_sets.is_some() {
            return Ok(set_palettes(name));
        }
        let present = model.levels.get(name).and_then(|l| l.core.present.as_ref());
        let mut p = match present.and_then(|p| p.palettes.as_ref().map(|s| (p, s))) {
            Some((p, slot)) => files
                .palettes(model, slot, p.background.as_ref(), Some(&base), &base_roles)
                .map_err(|e| format!("levels.{name}.present: {e}"))?,
            None => Palettes {
                bytes: base.clone(),
                background: None,
                roles: base_roles.clone(),
            },
        };
        layout_palette_writes(model, &files, name, &mut p)?;
        Ok(p)
    };

    // Palette sets, by the spec's levels: sprites use the second half,
    // the HUD the first. With `names`, its levels' sets come first, in
    // its order, so each set has the number it has there.
    let mut level_pals = Vec::new();
    let mut sprite_sets: Vec<Palettes> = Vec::new();
    let mut hud_sets: Vec<Palettes> = Vec::new();
    let mut set_of = Vec::new();
    let mut hud_set_of = Vec::new();
    if let Some((nm, _)) = names {
        // Every level of `names`' (a spec orders its levels by name, as
        // the model keeps them), those the model hasn't too.
        for name in nm.levels.keys() {
            let p = level_palettes(nm, name)?;
            if !sprite_sets
                .iter()
                .any(|s| s.bytes.get(16..) == p.bytes.get(16..))
            {
                sprite_sets.push(p.clone());
            }
            if !hud_sets
                .iter()
                .any(|s| s.bytes.get(..16) == p.bytes.get(..16))
            {
                hud_sets.push(p);
            }
        }
    }
    for l in &spec.levels {
        let p = level_palettes(model, &l.name)?;
        let i = match sprite_sets
            .iter()
            .position(|s| s.bytes.get(16..) == p.bytes.get(16..))
        {
            Some(i) => i,
            None => {
                sprite_sets.push(p.clone());
                sprite_sets.len() - 1
            }
        };
        set_of.push(i as u32);
        let h = match hud_sets
            .iter()
            .position(|s| s.bytes.get(..16) == p.bytes.get(..16))
        {
            Some(h) => h,
            None => {
                hud_sets.push(p.clone());
                hud_sets.len() - 1
            }
        };
        hud_set_of.push(h as u32);
        level_pals.push(p);
    }
    if sprite_sets.is_empty() {
        sprite_sets.push(Palettes {
            bytes: base.clone(),
            background: None,
            roles: base_roles.clone(),
        });
        hud_sets.push(sprite_sets[0].clone());
    }

    // Levels, with a mask for each color that cycles.
    let cycles_of = |name: &str| {
        model
            .levels
            .get(name)
            .and_then(|l| l.core.present.as_ref())
            .map(|p| p.cycles.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| present.cycles.clone())
    };
    // The tiles slots a bump's pictures are drawn from.
    let mut bump_tiles: BTreeMap<String, Tiles> = BTreeMap::new();
    for t in present
        .cells
        .iter()
        .flat_map(|c| c.bump.iter())
        .flat_map(|b| b.sprites.values())
    {
        if !bump_tiles.contains_key(&t.slot) {
            let tiles = files
                .tiles_slot(model, &t.slot)
                .map_err(|e| format!("present.cells.bump.sprites: {e}"))?;
            bump_tiles.insert(t.slot.clone(), tiles);
        }
    }
    let mut cycle_colors: BTreeMap<String, Vec<[u8; 3]>> = BTreeMap::new();
    for (li, l) in spec.levels.iter().enumerate() {
        let pal = &level_pals[li];
        let Some(level) = model.levels.get(&l.name) else {
            continue;
        };
        let c = &level.core;
        let indexed = if own_sets.is_some() {
            None
        } else if let Some(layout) = &c.layout {
            Some(layout_picture(model, &files, &l.name, layout, c.size, pal)?)
        } else if let (Some(grid), Some(tileset)) = (&c.grid, &c.tileset) {
            let item_roles = model
                .assets
                .get(tileset)
                .map(|s| s.item_roles.clone())
                .unwrap_or_default();
            let mut cells: BTreeMap<String, (u8, gp_theme::pix::Rgba)> = BTreeMap::new();
            for r in &item_roles {
                if let Some((layer, file, _)) = theme.restyle_item(tileset, r)
                    && !cells.contains_key(r)
                {
                    let i = gp_theme::pix::Rgba::load(&file)?;
                    if (i.width, i.height) != (16, 16) {
                        return Err(format!(
                            "{}: a restyled metatile is 16 by 16, not {} by {}",
                            file.display(),
                            i.width,
                            i.height
                        ));
                    }
                    cells.insert(r.clone(), (layer as u8 + 1, i));
                }
            }
            Some(grid_picture(
                model,
                &files,
                &l.name,
                grid,
                tileset,
                c.tile_size,
                &item_roles,
                &cells,
            )?)
        } else {
            None
        };
        let (indexed, level_tiles) = match indexed {
            Some((ix, lt)) => (Some(ix), Some(lt)),
            None => (None, None),
        };
        let fallback_level = fallback.as_ref().and_then(|f| f.levels.get(&l.name));
        let clear = match fallback_level {
            Some((_, c)) => *c,
            None => themed_rgb("sky", pal.background_rgb(&rgb)),
        };
        art.theme
            .samples
            .add_color("sky", gp_theme::color::Rgb(clear), 1);
        // Core `present.sprite_order`: each kind's place in the table.
        let order = spec
            .kinds
            .iter()
            .map(|k| {
                c.present
                    .as_ref()
                    .and_then(|p| p.sprite_order.get(&k.name))
                    .map(|o| gp_present::SpriteOrder {
                        at: o.at,
                        step: o.step,
                        word: o.word,
                        turn: o.turn.as_ref().and_then(|t| {
                            let c = spec.counters.iter().position(|x| x.name == t.counter)?;
                            Some((c, t.count))
                        }),
                    })
            })
            .collect();
        // Core `present.effect_order`: each effect's place, by its event.
        let effect_order = present
            .effects
            .iter()
            .map(|e| {
                c.present
                    .as_ref()
                    .and_then(|p| p.effect_order.get(&e.event))
                    .map(|o| (o.at, o.step))
            })
            .collect();
        let mut look = gp_present::LevelLook {
            order,
            effect_order,
            clear,
            set: set_of[li],
            label: c
                .present
                .as_ref()
                .and_then(|p| p.label.clone())
                .unwrap_or_default(),
            ..Default::default()
        };
        if let (Some(cp), Some(lt)) = (&present.cells, &level_tiles) {
            look.cells = Some(cell_look(
                &mut art,
                theme,
                cp,
                lt,
                &bump_tiles,
                &spec.levels[li],
                &l.name,
                &|i| pal.color(&rgb, i),
            )?);
        }
        if let Some(ix) = &indexed {
            let img = ix.role_image(&|i| pal.color(&rgb, i));
            let pic = art.add_themed(theme, format!("level-{}.png", l.name), &img);
            look.picture = Some(pic);
            art.levels
                .insert(l.name.clone(), (art.pictures[pic as usize].clone(), clear));
        } else if let Some((p, _)) = fallback_level {
            art.present.pictures.push(gp_present::Picture {
                file: p.file.clone(),
                size: [p.width, p.height],
            });
            art.pictures.push(p.clone());
            look.picture = Some((art.pictures.len() - 1) as u32);
            art.levels.insert(l.name.clone(), (p.clone(), clear));
        }
        for (k, cy) in cycles_of(&l.name).iter().enumerate() {
            let colors = match cycle_colors.get(&cy.colors) {
                Some(c) => c.clone(),
                None => {
                    let c: Vec<[u8; 3]> = files
                        .first_column(model, &cy.colors)
                        .map_err(|e| format!("cycle colors: {e}"))?
                        .iter()
                        .map(|&n| themed_rgb("sky", rgb[usize::from(n & 0x3F)]))
                        .collect();
                    cycle_colors.insert(cy.colors.clone(), c.clone());
                    c
                }
            };
            let entry = (cy.palette * 4 + cy.index) as u8;
            let mask = indexed.as_ref().and_then(|ix| ix.mask(entry)).map(|m| {
                let ix = indexed.as_ref().unwrap();
                art.add(
                    format!("level-{}-cycle{k}.png", l.name),
                    ix.width,
                    ix.height,
                    &m,
                )
            });
            let counter = match &cy.counter {
                Some(name) => Some((
                    spec.counters
                        .iter()
                        .position(|x| &x.name == name)
                        .ok_or(format!("cycle counter: no counter `{name}`"))?,
                    cy.shift.unwrap_or(0),
                )),
                None => None,
            };
            look.cycles.push(gp_present::Cycle {
                mask,
                hud_mask: None,
                colors,
                every: cy.every,
                counter,
            });
        }
        art.present.levels.push(look);
    }

    // Sprite frames, each drawn once per sprite palette set.
    let mut frame_ids: BTreeMap<FrameKey, u32> = BTreeMap::new();
    let mut frame_keys: Vec<FrameKey> = Vec::new();
    let mut frame_of = |key: FrameKey| -> u32 {
        if let Some(&id) = frame_ids.get(&key) {
            return id;
        }
        let id = frame_keys.len() as u32;
        frame_ids.insert(key.clone(), id);
        frame_keys.push(key);
        id
    };
    let numbers = |sprite: &Sprite, row: Option<u32>, tiles: &[u32]| -> Result<Vec<u32>, String> {
        if !tiles.is_empty() {
            return Ok(tiles.to_vec());
        }
        match (&sprite.frame, row) {
            (Some(f), Some(r)) => Ok(files
                .row(model, &f.slot, r)?
                .into_iter()
                .map(u32::from)
                .collect()),
            (Some(f), None) => Ok(files
                .row(model, &f.slot, f.row)?
                .into_iter()
                .map(u32::from)
                .collect()),
            (None, _) => Ok(sprite.tiles.clone()),
        }
    };
    // Each kind's own frame and each state's, for EntityArt and the
    // Style checks: (kind, state or none, frame, size, part).
    let mut owns: Vec<Own> = Vec::new();
    // With `names`, its kinds' frames are numbered first, in its order
    // (`keep` false: only their numbers are kept), so each frame the model
    // shares with it has the number it has there.
    let seeds = names
        .iter()
        .flat_map(|(nm, ns)| ns.kinds.iter().map(move |k| (k, *nm, false)));
    let kinds: Vec<_> = seeds
        .chain(spec.kinds.iter().map(|k| (k, model, true)))
        .collect();
    for (kind, from, keep) in kinds {
        let Some(entity) = from.entities.get(&kind.name) else {
            if keep {
                art.present.kinds.push(vec![None]);
            }
            continue;
        };
        if keep {
            art.roles
                .insert(kind.name.clone(), entity.core.role.clone());
        }
        let base_sprite = entity.core.sprite.clone();
        let mut states = Vec::new();
        let names: Vec<Option<String>> = match &kind.machine {
            Some(m) => m.states.iter().map(|s| Some(s.name.clone())).collect(),
            None => vec![None],
        };
        for (sn, st) in names.into_iter().enumerate() {
            let e = match &st {
                Some(n) => entity.in_state(n)?,
                None => entity.clone(),
            };
            let at = match &st {
                Some(n) => format!("entities.{}.machine.states.{n}.sprite", kind.name),
                None => format!("entities.{}.sprite", kind.name),
            };
            let Some(sprite) = &e.core.sprite else {
                let size = e.core.size.map(|v| (v.raw() >> 16).max(1) as u32);
                let id = frame_of(FrameKey::Box {
                    size,
                    role: e.core.role.clone(),
                });
                states.push(Some(gp_present::Look {
                    base: Frame {
                        id,
                        ..Frame::default()
                    },
                    ..Default::default()
                }));
                continue;
            };
            let colors = sprite
                .colors
                .as_ref()
                .map(|p| files.palette(model, p))
                .transpose()
                .map_err(|e| format!("{at}.colors: {e}"))?;
            // A frame of a sprite-set is the set's layout, by its number.
            let in_set = sprite.frame.as_ref().filter(|f| {
                model
                    .assets
                    .get(&f.slot)
                    .is_some_and(|s| s.kind == AssetKind::SpriteSet)
            });
            let set_key = |row: Option<u32>| {
                in_set.map(|f| FrameKey::Set {
                    set: f.slot.clone(),
                    index: row.unwrap_or(f.row),
                    slot: sprite.slot.clone(),
                    colors: colors.map(|c| (sprite.palette, c)),
                    role: e.core.role.clone(),
                })
            };
            let hb = e.core.hitbox;
            // A flip in place (feature `sprite.flip_in_place`) turns about
            // the picture's own middle: its offset plus half its tiles'
            // width, 8 pixels a tile.
            let mirror = match (sprite.flip_in_place, sprite.offset) {
                (true, Some([ox, _])) => 2 * i64::from(ox) + 8 * i64::from(sprite.width.max(1)),
                _ => (2 * hb.x.raw() + hb.w.raw()) >> 16,
            };
            let key = |nums: Vec<u32>,
                       cells: &[Cell],
                       offset: Option<[i32; 2]>,
                       own: Option<[u8; 4]>| FrameKey::Tiles {
                slot: sprite.slot.clone(),
                tiles: nums,
                width: sprite.width.max(1),
                palette: sprite.palette,
                cells: cells_key(cells),
                colors: own.or(colors),
                role: e.core.role.clone(),
                offset: offset.or(sprite.offset),
            };
            let base_nums = match in_set {
                Some(_) => Vec::new(),
                None => numbers(sprite, None, &[]).map_err(|e| format!("{at}: {e}"))?,
            };
            let base = frame_of(match set_key(None) {
                Some(k) => k,
                None => key(base_nums.clone(), &sprite.cells, None, None),
            });
            if keep && (st.is_none() || Some(sprite) != base_sprite.as_ref()) {
                let size = e.core.size.map(|v| (v.raw().max(0) >> 16) as u32);
                owns.push((
                    kind.name.clone(),
                    st.as_ref().map(|_| sn),
                    base,
                    size,
                    at.clone(),
                ));
            }
            let mut clips = Vec::new();
            for (ci, c) in sprite.clips.iter().enumerate() {
                let mut frames = Vec::new();
                for f in &c.frames {
                    if let Some(k) = set_key(f.row) {
                        frames.push(Frame {
                            id: frame_of(k),
                            flip_x: f.flip_x,
                            flip_y: f.flip_y,
                        });
                        continue;
                    }
                    let nums = if let (Some(slot), Some(row)) = (&f.slot, f.row) {
                        files
                            .row(model, slot, row)
                            .map_err(|e| format!("{at}.clips[{ci}]: {e}"))?
                            .into_iter()
                            .map(u32::from)
                            .collect()
                    } else if f.row.is_none() && f.tiles.is_empty() {
                        base_nums.clone()
                    } else {
                        numbers(sprite, f.row, &f.tiles)
                            .map_err(|e| format!("{at}.clips[{ci}]: {e}"))?
                    };
                    let cells = if f.cells.is_empty() {
                        &sprite.cells
                    } else {
                        &f.cells
                    };
                    // Feature `clip.frame_colors`.
                    let own = f
                        .colors
                        .as_ref()
                        .map(|p| files.palette(model, p))
                        .transpose()
                        .map_err(|e| format!("{at}.clips[{ci}].colors: {e}"))?;
                    frames.push(Frame {
                        id: frame_of(key(nums, cells, f.offset, own)),
                        flip_x: f.flip_x,
                        flip_y: f.flip_y,
                    });
                }
                let timing = match (c.every, c.by_speed.is_empty()) {
                    (Some(e), _) => vec![(None, e)],
                    (None, false) => c
                        .by_speed
                        .iter()
                        .map(|b| (b.at_least.map(|v| v.raw()), b.every))
                        .collect(),
                    (None, true) => Vec::new(),
                };
                clips.push(gp_present::Clip {
                    when: present_when(&c.when, spec),
                    frames,
                    timing,
                    game_clock: c.clock == gp_model::model::Clock::Game,
                    body_clock: c.clock == gp_model::model::Clock::Body,
                    while_frozen: c.while_frozen,
                    hold: c.hold,
                    speed_before: c.speed_before,
                    lengths: c.frames.iter().filter_map(|f| f.every).collect(),
                });
            }
            // The picture a successor with `replaced` draws: its own
            // sprite-set frame, or its own tiles.
            let leaves = match &sprite.leaves {
                None => None,
                Some(ls) => {
                    let lat = format!("{at}.leaves");
                    let set = ls.frame.as_ref().filter(|f| {
                        model
                            .assets
                            .get(&f.slot)
                            .is_some_and(|s| s.kind == AssetKind::SpriteSet)
                    });
                    let k = match set {
                        Some(f) => FrameKey::Set {
                            set: f.slot.clone(),
                            index: f.row,
                            slot: ls.slot.clone(),
                            colors: None,
                            role: e.core.role.clone(),
                        },
                        None => FrameKey::Tiles {
                            slot: ls.slot.clone(),
                            tiles: numbers(ls, None, &[]).map_err(|e| format!("{lat}: {e}"))?,
                            width: ls.width.max(1),
                            palette: ls.palette,
                            cells: cells_key(&ls.cells),
                            colors: None,
                            role: e.core.role.clone(),
                            offset: ls.offset,
                        },
                    };
                    Some(frame_of(k))
                }
            };
            states.push(Some(gp_present::Look {
                leaves,
                base: Frame {
                    id: base,
                    ..Frame::default()
                },
                facing_flip: sprite.flip_when_facing_left,
                copy: sprite
                    .copy
                    .as_ref()
                    .map(|c| [c.below, c.wrap, c.above].map(i64::from)),
                mirror,
                clips,
                blink: sprite
                    .blink
                    .as_ref()
                    .map(|b| (present_when(&b.when, spec), b.every)),
                behind: sprite.behind,
                behind_when: sprite.behind_when.as_ref().map(|c| present_when(c, spec)),
                split: sprite.split.as_ref().map(|sp| {
                    let counter = |n: &Option<String>| {
                        n.as_ref()
                            .and_then(|n| spec.counters.iter().position(|c| &c.name == n))
                    };
                    gp_present::SplitLook {
                        keep: sp.keep.map(|v| v as i32),
                        x: counter(&sp.x),
                        y: counter(&sp.y),
                    }
                }),
                replaced: sprite.replaced.as_ref().map(|r| r.flip_y),
                front: sprite.front,
                name_table: sprite.name_table,
                in_place: sprite.flip_in_place,
            }));
        }
        if keep {
            art.present.kinds.push(states);
        }
    }
    // Effects: each phase's pictures as sprite frames, and how it moves.
    let motion = |m: &gp_model::model::MotionPresent| gp_present::Motion {
        speed: m.speed.raw(),
        gravity: m.gravity.raw(),
        max_fall: m.max_fall.map(|v| v.raw()),
    };
    art.present.bump = present
        .cells
        .as_ref()
        .and_then(|c| c.bump.as_ref())
        .map(|b| gp_present::Bump {
            motion: gp_present::Motion {
                speed: b.speed.raw(),
                gravity: b.gravity.raw(),
                max_fall: b.max_fall.map(|v| v.raw()),
            },
            offset: b.offset,
        });
    for (i, e) in present.effects.iter().enumerate() {
        let at = format!("present.effects[{i}]");
        let event = spec
            .events
            .iter()
            .position(|n| *n == e.event)
            .ok_or(format!(
                "{at}.event: `{}` isn't one of the model's events",
                e.event
            ))?;
        let mut phases = Vec::new();
        for ph in &e.phases {
            let frames = ph
                .frames
                .iter()
                .map(|tiles| Frame {
                    id: frame_of(FrameKey::Tiles {
                        slot: ph.slot.clone(),
                        tiles: tiles.clone(),
                        width: ph.width.max(1),
                        palette: ph.palette,
                        cells: cells_key(&ph.cells),
                        colors: None,
                        role: "collectible".into(),
                        offset: None,
                    }),
                    ..Frame::default()
                })
                .collect();
            phases.push(gp_present::Phase {
                frames,
                every: ph.every.unwrap_or(1).max(1),
                motion: ph.motion.as_ref().map(motion),
                rise_every: ph.rise_every.unwrap_or(0),
                with_camera: ph.with_camera,
                until_speed: ph.until_speed.map(|v| v.raw()),
                length: ph.length,
                while_frozen: ph.while_frozen,
                until_counter: match &ph.until_counter {
                    Some(n) => Some(spec.counters.iter().position(|c| c.name == *n).ok_or(
                        format!("{at}.until_counter: `{n}` isn't one of the model's counters"),
                    )?),
                    None => None,
                },
                until_counters: ph
                    .until_counters
                    .iter()
                    .map(|n| {
                        spec.counters
                            .iter()
                            .position(|c| c.name == *n)
                            .ok_or(format!(
                                "{at}.until_counters: `{n}` isn't one of the model's counters"
                            ))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                until_gone: ph.until_gone,
            });
        }
        art.present.effects.push(gp_present::Effect {
            event,
            at_hit: e.at == gp_model::model::EffectAt::Hit,
            offset: e.offset,
            phases,
        });
    }
    let mut tiles_cache: BTreeMap<String, Tiles> = BTreeMap::new();
    let mut set_cache: BTreeMap<String, SpriteSet> = BTreeMap::new();
    for (id, key) in frame_keys.iter().enumerate() {
        let mut pics = Vec::new();
        art.present.origins.push(None);
        match key {
            FrameKey::Set {
                set,
                index,
                slot,
                colors,
                role,
            } => {
                if !set_cache.contains_key(set) {
                    set_cache.insert(set.clone(), files.sprite_set(model, set)?);
                }
                let frame = set_cache[set]
                    .frames
                    .get(*index as usize)
                    .cloned()
                    .ok_or(format!("sprite-set slot `{set}` has no frame {index}"))?;
                let [l, t, r, b] = frame.bounds();
                // A theme's restyle for the role replaces every frame of it
                // here too, standing on the body's bottom edge.
                if let Some((layer, path, _)) = theme.restyle_role(role) {
                    let i = gp_theme::pix::Rgba::load(&path)?;
                    let n = i.pixels.len();
                    let mut img = RoleImage {
                        image: i,
                        roles: vec![gp_theme::NO_ROLE; n],
                        floor: vec![layer as u8 + 1; n],
                    };
                    let rid = role_id(role);
                    for (r, p) in img.roles.iter_mut().zip(&img.image.pixels) {
                        if p[3] != 0 {
                            *r = rid;
                        }
                    }
                    let pic = art.add_themed(theme, format!("frame-{id}.png"), &img);
                    pics.resize(sprite_sets.len(), pic);
                    art.present.origins[id] = None;
                    art.present.frames.push(pics);
                    continue;
                }
                let (w, h) = (((r - l).max(1)) as u32, ((b - t).max(1)) as u32);
                let mut ix = Indexed::new(w, h);
                for p in &frame.pieces {
                    let ts = p.slot.as_ref().unwrap_or(slot);
                    if !tiles_cache.contains_key(ts) {
                        tiles_cache.insert(ts.clone(), files.tiles_slot(model, ts)?);
                    }
                    ix.tile(
                        &tiles_cache[ts],
                        p.tile,
                        (p.x - l) as u32,
                        (p.y - t) as u32,
                        p.palette,
                        (p.flip_x, p.flip_y),
                        false,
                        role_id(role),
                    );
                }
                for (si, set) in sprite_sets.iter().enumerate() {
                    let mut set = set.clone();
                    let mut put = |palette: usize, c: &[u8; 4]| {
                        let at = palette * 4;
                        if set.bytes.len() >= at + 4 {
                            set.bytes[at..at + 4].copy_from_slice(c);
                        }
                    };
                    for c in &frame.colors {
                        put(usize::from(c.palette), &c.colors);
                    }
                    if let Some((p, c)) = colors {
                        put(*p as usize, c);
                    }
                    let img = ix.role_image(&|i| set.color(&rgb, i));
                    pics.push(art.add_themed(theme, format!("frame-{id}-{si}.png"), &img));
                }
                art.present.origins[id] = Some([l, t]);
            }
            FrameKey::Box { size, role } => {
                let c = if role == "player" {
                    [216, 40, 0, 255]
                } else {
                    [136, 112, 0, 255]
                };
                let mut img = RoleImage::new(size[0], size[1]);
                for y in 0..size[1] {
                    for x in 0..size[0] {
                        img.set(x, y, c, role_id(role), 0);
                    }
                }
                let pic = art.add_themed(theme, format!("frame-{id}.png"), &img);
                pics.resize(sprite_sets.len(), pic);
            }
            FrameKey::Tiles {
                slot,
                tiles,
                width,
                palette,
                cells,
                colors,
                role,
                offset,
            } => {
                art.present.origins[id] = *offset;
                // A theme's restyle for the role replaces every frame of it
                // (restyles by state and frame are a later theme row).
                if let Some((layer, path, _)) = theme.restyle_role(role) {
                    let i = gp_theme::pix::Rgba::load(&path)?;
                    let n = i.pixels.len();
                    let mut img = RoleImage {
                        image: i,
                        roles: vec![gp_theme::NO_ROLE; n],
                        floor: vec![layer as u8 + 1; n],
                    };
                    let rid = role_id(role);
                    for (r, p) in img.roles.iter_mut().zip(&img.image.pixels) {
                        if p[3] != 0 {
                            *r = rid;
                        }
                    }
                    let pic = art.add_themed(theme, format!("frame-{id}.png"), &img);
                    pics.resize(sprite_sets.len(), pic);
                    art.present.frames.push(pics);
                    continue;
                }
                if !tiles_cache.contains_key(slot) {
                    tiles_cache.insert(slot.clone(), files.tiles_slot(model, slot)?);
                }
                let t = &tiles_cache[slot];
                let cols = *width;
                let place = |i: usize| {
                    let at = cells.get(i).and_then(|c| c.3);
                    at.unwrap_or([i as u32 % cols, i as u32 / cols])
                };
                let (w, h) = (0..tiles.len()).fold((cols, 1), |(w, h), i| {
                    let [c, r] = place(i);
                    (w.max(c + 1), h.max(r + 1))
                });
                let mut ix = Indexed::new(w * 8, h * 8);
                for (i, &tile) in tiles.iter().enumerate() {
                    let (fx, fy, p, _) = cells.get(i).copied().unwrap_or_default();
                    let pal = p.unwrap_or(*palette) as u8;
                    let [c, r] = place(i);
                    ix.tile(t, tile, c * 8, r * 8, pal, (fx, fy), false, role_id(role));
                }
                for (si, set) in sprite_sets.iter().enumerate() {
                    let mut set = set.clone();
                    if let Some(c) = colors {
                        let at = *palette as usize * 4;
                        if set.bytes.len() >= at + 4 {
                            set.bytes[at..at + 4].copy_from_slice(c);
                        }
                    }
                    let img = ix.role_image(&|i| set.color(&rgb, i));
                    pics.push(art.add_themed(theme, format!("frame-{id}-{si}.png"), &img));
                }
            }
        }
        art.present.frames.push(pics);
    }

    // Each kind's own frames in the start level's set, for tools that draw
    // one picture per state, and for the Style checks.
    let start_set = set_of.get(spec.start_level).copied().unwrap_or(0) as usize;
    if let Some(l) = art.present.levels.get(spec.start_level) {
        art.clear = l.clear;
    }
    for (kind, state, frame, size, part) in owns {
        let Some(&pic) = art.present.frames[frame as usize]
            .get(start_set)
            .or_else(|| art.present.frames[frame as usize].first())
        else {
            continue;
        };
        let p = art.pictures[pic as usize].clone();
        let flip = art
            .present
            .kinds
            .get(spec.kinds.iter().position(|k| k.name == kind).unwrap_or(0))
            .and_then(|s| s.get(state.unwrap_or(0)).cloned().flatten())
            .is_some_and(|l| l.facing_flip);
        let role = art.roles.get(&kind).cloned().unwrap_or_default();
        if let Ok(image) = gp_theme::pix::Rgba::from_png(&p.png) {
            art.theme.subjects.push(Subject {
                part,
                role,
                image,
                size,
            });
        }
        let e = art.sprites.entry(kind.clone()).or_default();
        match state {
            None => e.base = Some((p, flip)),
            Some(s) => {
                if e.states.len() <= s {
                    e.states.resize(s + 1, None);
                }
                e.states[s] = Some((p, flip));
            }
        }
    }
    // Every kind with a machine has a state list as long as its states.
    for k in &spec.kinds {
        if let (Some(m), Some(e)) = (&k.machine, art.sprites.get_mut(&k.name)) {
            e.states.resize(m.states.len(), None);
        }
    }
    let view = model
        .cameras
        .values()
        .next()
        .map(|c| c.size)
        .unwrap_or([256, 240]);
    art.theme.overlay = theme
        .overlay(view[0], view[1])
        .map(|o| picture("overlay.png", o.width, o.height, o.pixels.as_flattened()));
    art.theme.overlay_scale = theme.overlay_scale();
    let screen = &art.present.screen;
    let hidden = view[0].saturating_sub(screen.visible()[2]);
    let mut widths: Vec<u32> = std::iter::once(screen.visible()[2])
        .chain(
            gp_present::view::ASPECTS
                .iter()
                .map(|a| screen.wide_width(*a)),
        )
        .collect();
    widths.dedup();
    for w in widths {
        if let Some(o) = theme.overlay_fine(w + hidden, view[1]) {
            let file = format!("overlay-{w}.png");
            art.theme.overlays.push((
                w,
                picture(&file, o.width, o.height, o.pixels.as_flattened()),
            ));
        }
    }
    art.theme.emitters = theme.emitters();

    if let Some(hud) = &present.hud {
        art.present.hud = Some(hud_art(
            model,
            &files,
            hud,
            spec,
            &mut art,
            &hud_sets,
            &hud_set_of,
            &level_pals,
            &rgb,
            &present.cycles,
            theme,
        )?);
    }
    for (i, sc) in present.screens.iter().enumerate() {
        let look = screen_art(
            model,
            &files,
            spec,
            &mut art,
            sc,
            i,
            &level_pals,
            &rgb,
            theme,
        )?;
        art.present.screens.push(look);
    }
    // The states with the sprite layer off (core `present.sprites_off`).
    for st in present.screen.iter().flat_map(|s| &s.sprites_off) {
        let found = st.split_once('.').and_then(|(m, s)| {
            let mi = spec.machines.iter().position(|x| x.name == m)?;
            let si = spec.machines[mi].states.iter().position(|x| x.name == s)?;
            Some((mi, si))
        });
        art.present
            .sprites_off
            .push(found.ok_or(format!("present.screen.sprites_off: no rule state `{st}`"))?);
    }
    // Bodies not drawn while a state and counters hold (core
    // `present.hidden`).
    for (i, h) in present.hidden.iter().enumerate() {
        let at = format!("present.hidden[{i}]");
        let mut look = gp_present::HiddenLook::default();
        for st in &h.states {
            let found = st.split_once('.').and_then(|(m, s)| {
                let mi = spec.machines.iter().position(|x| x.name == m)?;
                let si = spec.machines[mi].states.iter().position(|x| x.name == s)?;
                Some((mi, si))
            });
            look.states
                .push(found.ok_or(format!("{at}.states: no rule state `{st}`"))?);
        }
        for t in &h.when {
            let k = spec
                .counters
                .iter()
                .position(|x| x.name == t.name)
                .ok_or(format!("{at}.when: no counter `{}`", t.name))?;
            look.counters.push((k, t.at_least, t.below));
        }
        for k in &h.kinds {
            let n = spec
                .kinds
                .iter()
                .position(|x| &x.name == k)
                .ok_or(format!("{at}.kinds: no entity type `{k}`"))?;
            look.kinds.push(n);
        }
        look.hud = h.hud;
        art.present.hidden.push(look);
    }
    Ok(art)
}

/// A screen shown in place of the level while a rule state holds (core
/// `present.screens`): its name table drawn from its update strings in the
/// palettes they write over the first level's, and its sprites in the
/// sprite palettes so made.
#[allow(clippy::too_many_arguments)]
fn screen_art(
    model: &Model,
    files: &Files,
    spec: &Spec,
    art: &mut Art,
    sc: &gp_model::model::StateScreen,
    i: usize,
    level_pals: &[Palettes],
    rgb: &[[u8; 3]; 64],
    theme: &Resolved,
) -> Result<gp_present::ScreenLook, String> {
    let at = format!("present.screens[{i}]");
    let mut states = Vec::new();
    for st in &sc.states {
        let found = st.split_once('.').and_then(|(m, s)| {
            let mi = spec.machines.iter().position(|x| x.name == m)?;
            let si = spec.machines[mi].states.iter().position(|x| x.name == s)?;
            Some((mi, si))
        });
        states.push(found.ok_or(format!("{at}.states: no rule state `{st}`"))?);
    }
    let mut counters = Vec::new();
    for t in &sc.when {
        let k = spec
            .counters
            .iter()
            .position(|x| x.name == t.name)
            .ok_or(format!("{at}.when: no counter `{}`", t.name))?;
        counters.push((k, t.at_least, t.below));
    }
    // Without a layout, nothing is written: the backdrop alone.
    let writes = match &sc.layout {
        Some(layout) => {
            let asset = files.asset(model, &layout.slot, AssetKind::ScreenText)?;
            let Asset::PpuStrings(ps) = asset else {
                return Err(format!(
                    "{at}.layout: the slot's asset isn't update strings"
                ));
            };
            let text = std::fs::read_to_string(files.dir.join(asset.file()))
                .map_err(|e| format!("{}: {e}", asset.file()))?;
            ps.writes(&text, &layout.entry)
                .map_err(|e| format!("{at}.layout: {e}"))?
        }
        None => Vec::new(),
    };
    let (mut names, attrs) = name_table(&writes);
    // Its text (core `present.screens_text`): the HUD font's tiles written
    // into the name table, each cell in the text's own palette.
    let mut cell_pal: Vec<Option<u8>> = vec![None; 32 * 30];
    if !sc.text.is_empty() {
        let hud = model
            .present
            .as_ref()
            .and_then(|p| p.hud.as_ref())
            .ok_or(format!("{at}.text: no present.hud to take the font from"))?;
        let font = files.asset(model, &hud.font, AssetKind::Font)?;
        let charset = font
            .charset()
            .ok_or(format!("{at}.text: the HUD font's asset has no charset"))?;
        for (k, t) in sc.text.iter().enumerate() {
            let [c0, r] = t.at;
            for (j, ch) in t.text.chars().enumerate() {
                let tile = charset
                    .tile(ch)
                    .ok_or(format!("{at}.text[{k}]: the HUD font has no `{ch}`"))?;
                let i = (r * 32 + c0) as usize + j;
                if let Some(cell) = names.get_mut(i) {
                    *cell = Some(tile);
                    cell_pal[i] = Some(t.palette.unwrap_or(0) as u8);
                }
            }
        }
    }
    // The palettes: the first level's, with the string's writes over them.
    let mut pal = Palettes {
        bytes: level_pals
            .first()
            .map_or(vec![0x0F; 32], |p| p.bytes.clone()),
        background: None,
        roles: level_pals
            .first()
            .map(|p| p.roles.clone())
            .unwrap_or_default(),
    };
    palette_writes(&mut pal.bytes, &writes);
    // The layout's slot may name the tiles it is drawn with
    // (`assets.<slot>.tiles`), else the level's.
    let own = sc
        .layout
        .as_ref()
        .and_then(|l| model.assets.get(&l.slot))
        .and_then(|s| s.tiles.clone());
    let tiles_slot = match own {
        Some(t) => t,
        None => {
            level_tiles_slot(model).ok_or(format!("{at}: no one tiles slot to draw it with"))?
        }
    };
    let tiles = files.tiles_slot(model, &tiles_slot)?;
    let mut ix = Indexed::new(256, 240);
    ix.px.fill(0);
    ix.roles.fill(role_id("menu"));
    for r in 0..30u32 {
        for c in 0..32u32 {
            let Some(tile) = names[(r * 32 + c) as usize] else {
                continue;
            };
            let a = attrs[((r / 4) * 8 + c / 4) as usize];
            let shift = ((r % 4) / 2) * 4 + ((c % 4) / 2) * 2;
            let pal = cell_pal[(r * 32 + c) as usize].unwrap_or((a >> shift) & 3);
            ix.tile(
                &tiles,
                u32::from(tile),
                c * 8,
                r * 8,
                pal,
                (false, false),
                true,
                role_id("menu"),
            );
        }
    }
    if let Some(layout) = &sc.layout {
        files.picture(&layout.slot, &layout.entry, &mut ix, "menu", false)?;
    }
    let img = ix.role_image(&|e| pal.color(rgb, e));
    let picture = art.add_themed(theme, format!("screen-{i}.png"), &img);
    let mut sprites = Vec::new();
    for (k, sp) in sc.sprite.iter().enumerate() {
        let st = files
            .tiles_slot(model, &sp.tiles)
            .map_err(|e| format!("{at}.sprite[{k}].tiles: {e}"))?;
        let mut ix = Indexed::new(8, 8);
        ix.tile(
            &st,
            sp.tile,
            0,
            0,
            4 + sp.palette.unwrap_or(0) as u8,
            (false, false),
            false,
            role_id("menu"),
        );
        let img = ix.role_image(&|e| pal.color(rgb, e));
        let pic = art.add_themed(theme, format!("screen-{i}-sprite-{k}.png"), &img);
        let counter = match &sp.counter {
            Some(c) => Some(
                spec.counters
                    .iter()
                    .position(|x| x.name == *c)
                    .ok_or(format!("{at}.sprite[{k}].counter: no counter `{c}`"))?,
            ),
            None => None,
        };
        sprites.push(gp_present::ScreenSprite {
            pic,
            at: sp.at,
            counter,
            step: sp.step.unwrap_or([0, 0]),
        });
    }
    Ok(gp_present::ScreenLook {
        states,
        counters,
        picture,
        clear: pal.background_rgb(rgb),
        sprites,
        from_line: sc.from_line,
    })
}

/// The HUD: its fixed layer in each background palette set (one picture
/// per level's set), a mask for each cycling color the layer uses, and a
/// glyph for each character a field can show, in each set and palette.
#[allow(clippy::too_many_arguments)]
fn hud_art(
    model: &Model,
    files: &Files,
    hud: &gp_model::model::Hud,
    spec: &Spec,
    art: &mut Art,
    sets: &[Palettes],
    set_of: &[u32],
    level_pals: &[Palettes],
    rgb: &[[u8; 3]; 64],
    cycles: &[gp_model::model::ColorCycle],
    theme: &Resolved,
) -> Result<gp_present::Hud, String> {
    let t = files
        .tiles_slot(model, &hud.tiles)
        .map_err(|e| format!("present.hud.tiles: {e}"))?;
    let font = files.asset(model, &hud.font, AssetKind::Font)?;
    let charset = font.charset().ok_or(
        "present.hud.font: the font's asset has no charset (strings with one, or tiles with one)",
    )?;
    let [sw, sh] = model
        .present
        .as_ref()
        .and_then(|p| p.screen.as_ref())
        .map_or([256, 240], |s| s.size);
    let [cols, rows] = hud.size.unwrap_or([sw / 8, sh / 8]);
    // The fixed layer, written into a name table as the game writes it.
    let mut layer_pics = Vec::new();
    let mut masks = Vec::new();
    let mut layer_ix: Option<Indexed> = None;
    if let Some(l) = &hud.layer {
        let asset = files.asset(model, &l.slot, AssetKind::ScreenText)?;
        let Asset::PpuStrings(ps) = asset else {
            return Err("present.hud.layer: the slot's asset isn't update strings".into());
        };
        let text = std::fs::read_to_string(files.dir.join(asset.file()))
            .map_err(|e| format!("{}: {e}", asset.file()))?;
        let writes = ps
            .writes(&text, &l.entry)
            .map_err(|e| format!("present.hud.layer: {e}"))?;
        let (names, mut attrs) = name_table(&writes);
        // A layer that writes no attribute bytes (Donkey Kong's status
        // box) takes the palettes the level's own string gives the name
        // table, which the game writes once for both.
        if !writes_attributes(&writes)
            && let Some(level) = level_attributes(model, files)?
        {
            attrs = level;
        }
        let mut ix = Indexed::new(cols * 8, rows * 8);
        for r in 0..rows.min(30) {
            for c in 0..cols.min(32) {
                let Some(tile) = names[(r * 32 + c) as usize] else {
                    continue;
                };
                let a = attrs[((r / 4) * 8 + c / 4) as usize];
                let shift = ((r % 4) / 2) * 4 + ((c % 4) / 2) * 2;
                let pal = (a >> shift) & 3;
                ix.tile(
                    &t,
                    u32::from(tile),
                    c * 8,
                    r * 8,
                    pal,
                    (false, false),
                    false,
                    role_id("hud"),
                );
            }
        }
        files.picture(&l.slot, &l.entry, &mut ix, "hud", true)?;
        for (i, set) in sets.iter().enumerate() {
            let img = ix.role_image(&|e| set.color(rgb, e));
            layer_pics.push(art.add_themed(theme, format!("hud-{i}.png"), &img));
        }
        for (k, cy) in cycles.iter().enumerate() {
            let m = ix
                .mask((cy.palette * 4 + cy.index) as u8)
                .map(|m| art.add(format!("hud-cycle{k}.png"), ix.width, ix.height, &m));
            masks.push(m);
        }
        layer_ix = Some(ix);
    }
    // Sprites fixed on the screen: one picture for each level's sprite
    // palettes, the pixels the layer covers left out for a sprite behind
    // the background.
    let mut sprites = Vec::new();
    for (i, sp) in hud.sprite.iter().enumerate() {
        let st = files
            .tiles_slot(model, &sp.tiles)
            .map_err(|e| format!("present.hud.sprite[{i}].tiles: {e}"))?;
        let mut cache: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
        let mut pics = Vec::new();
        for (li, pal) in level_pals.iter().enumerate() {
            let key = pal.bytes.get(16..).unwrap_or(&[]).to_vec();
            if let Some(&p) = cache.get(&key) {
                pics.push(p);
                continue;
            }
            let mut ix = Indexed::new(8, 8);
            ix.tile(
                &st,
                sp.tile,
                0,
                0,
                4 + sp.palette.unwrap_or(0) as u8,
                (false, false),
                false,
                role_id("hud"),
            );
            if sp.behind
                && let Some(layer) = &layer_ix
            {
                for y in 0..8i64 {
                    for x in 0..8i64 {
                        let (lx, ly) = (i64::from(sp.at[0]) + x, i64::from(sp.at[1]) + y);
                        if lx < 0 || ly < 0 || lx >= i64::from(layer.width) {
                            continue;
                        }
                        let at = (ly * i64::from(layer.width) + lx) as usize;
                        if layer.px.get(at).is_some_and(|&e| e != NONE) {
                            ix.px[(y * 8 + x) as usize] = NONE;
                        }
                    }
                }
            }
            let img = ix.role_image(&|e| pal.color(rgb, e));
            let p = art.add_themed(theme, format!("hud-sprite-{i}-{li}.png"), &img);
            cache.insert(key, p);
            pics.push(p);
        }
        sprites.push(gp_present::HudSprite { at: sp.at, pics });
    }
    for (li, look) in art.present.levels.iter_mut().enumerate() {
        let set = set_of.get(li).copied().unwrap_or(0) as usize;
        look.hud_layer = layer_pics.get(set).copied();
        // The model's cycles come first in a level that uses them.
        let own = spec
            .levels
            .get(li)
            .and_then(|l| model.levels.get(&l.name))
            .and_then(|l| l.core.present.as_ref())
            .is_some_and(|p| !p.cycles.is_empty());
        if !own {
            for (c, m) in look.cycles.iter_mut().zip(&masks) {
                c.hud_mask = *m;
            }
        }
    }
    // Fields and their glyphs.
    let mut fields = Vec::new();
    let mut chars: BTreeMap<u32, std::collections::BTreeSet<char>> = BTreeMap::new();
    for (i, f) in hud.field.iter().enumerate() {
        let at = format!("present.hud.field[{i}]");
        let pad = f.pad.as_ref().and_then(|p| p.chars().next()).unwrap_or('0');
        // A field drawn as sprites takes a sprite palette, 4 to 7 in the
        // picture's palette numbers.
        let palette = f.palette.unwrap_or(0) + if f.sprite { 4 } else { 0 };
        let source = if let Some(c) = &f.counter {
            let k = spec
                .counters
                .iter()
                .position(|x| &x.name == c)
                .ok_or(format!("{at}.counter: no counter `{c}`"))?;
            chars
                .entry(palette)
                .or_default()
                .extend("0123456789".chars().chain([pad]));
            gp_present::Source::Counter(k)
        } else if f.level_label {
            for l in model.levels.values() {
                if let Some(label) = l.core.present.as_ref().and_then(|p| p.label.as_ref()) {
                    chars.entry(palette).or_default().extend(label.chars());
                }
            }
            gp_present::Source::Level
        } else {
            let t = f.text.clone().unwrap_or_default();
            chars.entry(palette).or_default().extend(t.chars());
            gp_present::Source::Text(t)
        };
        let mut when = Vec::new();
        for g in &f.when {
            use gp_model::model::Cmp as C;
            use gp_sim::spec::Cmp;
            when.push(gp_sim::spec::Guard {
                counter: spec
                    .counters
                    .iter()
                    .position(|c| c.name == g.counter)
                    .ok_or(format!(
                        "present.hud.field when: no counter `{}`",
                        g.counter
                    ))?,
                cmp: match g.cmp {
                    C::Lt => Cmp::Lt,
                    C::Le => Cmp::Le,
                    C::Eq => Cmp::Eq,
                    C::Ne => Cmp::Ne,
                    C::Ge => Cmp::Ge,
                    C::Gt => Cmp::Gt,
                },
                value: g.value,
            });
        }
        fields.push(gp_present::Field {
            at: f.at,
            source,
            digits: f.digits.unwrap_or(0),
            divide: f.divide.unwrap_or(1),
            pad,
            palette,
            when,
        });
    }
    let mut glyphs = Vec::new();
    for (si, set) in sets.iter().enumerate() {
        for (&palette, cs) in &chars {
            for &ch in cs {
                let Some(tile) = charset.tile(ch) else {
                    continue;
                };
                let mut ix = Indexed::new(8, 8);
                // A field in the background (palettes 0 to 3) is name
                // table cells, which cover what the layer had there with
                // their color 0 too; one drawn as sprites shows through.
                ix.tile(
                    &t,
                    u32::from(tile),
                    0,
                    0,
                    palette as u8,
                    (false, false),
                    palette < 4,
                    role_id("hud"),
                );
                let img = ix.role_image(&|e| set.color(rgb, e));
                let pic = art.add_themed(
                    theme,
                    format!("glyph-{si}-{palette}-{:x}.png", ch as u32),
                    &img,
                );
                glyphs.push(gp_present::Glyph {
                    set: si as u32,
                    palette,
                    ch,
                    pic,
                });
            }
        }
    }
    Ok(gp_present::Hud {
        fields,
        glyphs,
        sprites,
        sets: set_of.to_vec(),
    })
}

/// Whether any of the writes lands in the first name table's attributes.
fn writes_attributes(writes: &[gp_assets::ppu::Write]) -> bool {
    writes.iter().any(|w| {
        let step = if w.vertical { 32 } else { 1 };
        (0..w.bytes.len()).any(|i| (0x23C0..0x2400).contains(&(usize::from(w.ppu) + i * step)))
    })
}

/// The attribute bytes the first level drawn from a PPU string writes, if
/// one is.
fn level_attributes(model: &Model, files: &Files) -> Result<Option<Vec<u8>>, String> {
    for level in model.levels.values() {
        let Some(layout) = &level.core.layout else {
            continue;
        };
        let asset = files.asset(model, &layout.slot, AssetKind::LevelObjects)?;
        let Asset::PpuStrings(ps) = asset else {
            continue;
        };
        let text = std::fs::read_to_string(files.dir.join(asset.file()))
            .map_err(|e| format!("{}: {e}", asset.file()))?;
        let writes = ps.writes(&text, &layout.entry)?;
        if writes_attributes(&writes) {
            return Ok(Some(name_table(&writes).1));
        }
    }
    Ok(None)
}

/// The first name table and its attributes after a list of writes: each
/// tile written (None where nothing was), and the 64 attribute bytes.
fn name_table(writes: &[gp_assets::ppu::Write]) -> (Vec<Option<u8>>, Vec<u8>) {
    let mut names = vec![None; 32 * 30];
    let mut attrs = vec![0u8; 64];
    for w in writes {
        let mut a = usize::from(w.ppu);
        for &b in &w.bytes {
            match a {
                0x2000..0x23C0 => names[a - 0x2000] = Some(b),
                0x23C0..0x2400 => attrs[a - 0x23C0] = b,
                _ => {}
            }
            a += if w.vertical { 32 } else { 1 };
        }
    }
    (names, attrs)
}

/// The pictures of a model with no palettes slot, so no art of its own:
/// each level's cells colored by their tile kind and every entity a box of
/// its size, so the generated game is playable and the simulation can be
/// seen while the art is missing. A last resort: a model gives its art
/// through its asset slots and `present` (Mega Man 2's and Super Mario
/// Bros. 2's do). It reads only the spec, never a theme.
pub fn tile_art(spec: &Spec, screen: gp_present::Screen) -> Art {
    use gp_sim::spec::Tile;
    let color = |t: Tile| -> [u8; 4] {
        match t {
            Tile::Empty => [0, 0, 0, 0],
            Tile::Solid => [120, 120, 120, 255],
            Tile::Top => [170, 140, 90, 255],
            Tile::Ladder => [200, 170, 60, 255],
            Tile::Hazard => [200, 60, 60, 255],
            Tile::Water => [60, 90, 200, 255],
            Tile::Ice => [170, 220, 240, 255],
            Tile::CoinBlock => [220, 160, 40, 255],
            Tile::ItemBlock => [230, 120, 40, 255],
            Tile::Brick => [160, 80, 50, 255],
            Tile::Coin => [240, 200, 60, 255],
        }
    };
    let mut art = Art {
        clear: [24, 24, 32],
        ..Art::default()
    };
    art.present.screen = screen;
    // A changed cell draws its new kind's color, a cell's size; every
    // level has the same size of cell or its own pictures.
    let mut kind_pics: BTreeMap<i64, Vec<(u8, u32, u32)>> = BTreeMap::new();
    for level in &spec.levels {
        let ts = level.tile_size.max(1);
        if kind_pics.contains_key(&ts) {
            continue;
        }
        let mut pics = Vec::new();
        for t in Tile::ALL {
            // An emptied cell shows the clear color over what was there.
            let [r, g, b] = art.clear;
            let c = if t == Tile::Empty {
                [r, g, b, 255]
            } else {
                color(t)
            };
            let rgba: Vec<u8> = (0..ts * ts).flat_map(|_| c).collect();
            let pic = art.add(
                format!("cell-{ts}-{}.png", t as u8),
                ts as u32,
                ts as u32,
                &rgba,
            );
            pics.push((t as u8, pic, pic));
        }
        kind_pics.insert(ts, pics);
    }
    for level in &spec.levels {
        let ts = level.tile_size.max(1) as usize;
        let [cw, ch] = [level.size[0] as usize, level.size[1] as usize];
        let (w, h) = (cw * ts, ch * ts);
        let mut rgba = vec![0u8; w * h * 4];
        for (i, t) in level.tiles.iter().enumerate() {
            let (cx, cy) = (i % cw.max(1), i / cw.max(1));
            let c = color(*t);
            for y in cy * ts..(cy + 1) * ts {
                for x in cx * ts..(cx + 1) * ts {
                    rgba[(y * w + x) * 4..][..4].copy_from_slice(&c);
                }
            }
        }
        let pic = art.add(
            format!("level-{}.png", level.name),
            w as u32,
            h as u32,
            &rgba,
        );
        art.levels.insert(
            level.name.clone(),
            (art.pictures[pic as usize].clone(), art.clear),
        );
        art.present.levels.push(gp_present::LevelLook {
            picture: Some(pic),
            clear: art.clear,
            cells: Some(gp_present::CellLook {
                kinds: kind_pics
                    .get(&level.tile_size.max(1))
                    .cloned()
                    .unwrap_or_default(),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    for k in &spec.kinds {
        let mut looks = Vec::new();
        let states: Vec<&gp_sim::spec::Kind> = match &k.machine {
            Some(m) => m.states.iter().map(|s| &s.kind).collect(),
            None => vec![k],
        };
        for kk in states {
            let size = kk.size.map(|v| (v.raw() >> 16).max(1) as u32);
            let c = if kk.role == gp_sim::spec::Role::Player {
                [216, 40, 0, 255]
            } else {
                [136, 112, 0, 255]
            };
            let rgba: Vec<u8> = (0..size[0] * size[1]).flat_map(|_| c).collect();
            let id = art.present.frames.len() as u32;
            let pic = art.add(format!("frame-{id}.png"), size[0], size[1], &rgba);
            art.present.frames.push(vec![pic]);
            looks.push(Some(gp_present::Look {
                base: Frame {
                    id,
                    ..Frame::default()
                },
                ..Default::default()
            }));
        }
        art.present.kinds.push(looks);
    }
    art
}

/// A level from a `level-grid` slot and its `metatiles` tileset.
#[allow(clippy::too_many_arguments)]
fn grid_picture(
    model: &Model,
    files: &Files,
    name: &str,
    grid: &str,
    tileset: &str,
    tile_size: u32,
    item_roles: &[String],
    restyled: &BTreeMap<String, (u8, gp_theme::pix::Rgba)>,
) -> Result<(Indexed, LevelTiles), String> {
    let grid = files.asset(model, grid, AssetKind::LevelGrid)?;
    let Asset::Level { width, height, .. } = grid else {
        return Err(format!("levels.{name}.grid isn't a level grid"));
    };
    let cells = files.part(grid)?;
    let tileset = files.asset(model, tileset, AssetKind::Metatiles)?;
    let Asset::Metatiles { tile_order, .. } = tileset else {
        return Err(format!("levels.{name}.tileset isn't metatiles"));
    };
    let metatiles = files.part(tileset)?;
    let Some(tiles_slot) = level_tiles_slot(model) else {
        return Err(format!(
            "levels.{name}: no one tiles slot to draw it with (one slot, or one no sprite uses)"
        ));
    };
    let tiles = files.tiles_slot(model, &tiles_slot)?;
    // Metatiles are 2 by 2 tiles of 8 pixels.
    if tile_size != 16 {
        return Err(format!(
            "levels.{name}.tile_size is {tile_size}, but metatiles are drawn 16 pixels square"
        ));
    }
    let mut ix = Indexed::new(width * 16, height * 16);
    let mut lt = LevelTiles {
        cells: cells.iter().map(|&c| Some(c)).collect(),
        quads: Vec::new(),
        tiles,
        single: false,
    };
    for mt in 0..=255usize {
        let quad = metatiles.get(mt * 4..mt * 4 + 4).map(|q| {
            let q = match tile_order {
                TileOrder::Rows => [q[0], q[1], q[2], q[3]],
                TileOrder::Columns => [q[0], q[2], q[1], q[3]],
            };
            (
                q.map(u32::from),
                0u8,
                item_roles.get(mt).map_or(gp_theme::NO_ROLE, |r| role_id(r)),
            )
        });
        lt.quads.push(quad);
    }
    let tiles = &lt.tiles;
    for (i, &cell) in cells.iter().enumerate() {
        let (cx, cy) = (i as u32 % width, i as u32 / width);
        let role = item_roles.get(usize::from(cell));
        let id = role.map_or(gp_theme::NO_ROLE, |r| role_id(r));
        if let Some((floor, art)) = role.and_then(|r| restyled.get(r)) {
            for y in 0..16 {
                for x in 0..16 {
                    let at = ((cy * 16 + y) * ix.width + cx * 16 + x) as usize;
                    ix.direct[at] = Some((art.get(x, y), *floor));
                    ix.roles[at] = id;
                    ix.px[at] = NONE;
                }
            }
            continue;
        }
        let Some(quad) = metatiles.get(usize::from(cell) * 4..usize::from(cell) * 4 + 4) else {
            return Err(format!(
                "levels.{name}: metatile {cell} is past the tileset"
            ));
        };
        // The lifted file lists tiles by rows; the bytes are in ROM order.
        let quad = match tile_order {
            TileOrder::Rows => [quad[0], quad[1], quad[2], quad[3]],
            TileOrder::Columns => [quad[0], quad[2], quad[1], quad[3]],
        };
        for (q, &tile) in quad.iter().enumerate() {
            let (ox, oy) = (cx * 16 + (q as u32 % 2) * 8, cy * 16 + (q as u32 / 2) * 8);
            ix.tile(tiles, tile.into(), ox, oy, 0, (false, false), true, id);
        }
    }
    Ok((ix, lt))
}

/// A level drawn from a `ppu-strings` entry.
fn layout_picture(
    model: &Model,
    files: &Files,
    name: &str,
    layout: &gp_model::model::EntryRef,
    size: [u32; 2],
    palettes: &Palettes,
) -> Result<(Indexed, LevelTiles), String> {
    let at = format!("levels.{name}.layout");
    let asset = files.asset(model, &layout.slot, AssetKind::LevelObjects)?;
    let Asset::PpuStrings(ps) = asset else {
        return Err(format!(
            "{at}: slot `{}` isn't a ppu-strings asset",
            layout.slot
        ));
    };
    let (mut ix, lt) = ppu_layout_picture(
        model,
        files,
        &at,
        ps,
        asset,
        &layout.slot,
        &layout.entry,
        size,
        palettes,
    )?;
    files.picture(&layout.slot, &layout.entry, &mut ix, "background", false)?;
    Ok((ix, lt))
}

/// A level drawn from a PPU update string (core `level.layout_ppu_strings`)
/// shows the palettes the string writes ($3F00 to $3F1F) over its own, in
/// the string's order, as the game's screen does: Donkey Kong's 50m, 75m
/// and 100m strings each write a stage's colors after the first set.
/// A level drawn otherwise is left as it is.
fn layout_palette_writes(
    model: &Model,
    files: &Files,
    name: &str,
    pal: &mut Palettes,
) -> Result<(), String> {
    let Some(layout) = model.levels.get(name).and_then(|l| l.core.layout.as_ref()) else {
        return Ok(());
    };
    let Ok(asset) = files.asset(model, &layout.slot, AssetKind::LevelObjects) else {
        return Ok(());
    };
    let Asset::PpuStrings(ps) = asset else {
        return Ok(());
    };
    let text = std::fs::read_to_string(files.dir.join(asset.file()))
        .map_err(|e| format!("{}: {e}", asset.file()))?;
    let writes = ps
        .writes(&text, &layout.entry)
        .map_err(|e| format!("levels.{name}.layout: {e}"))?;
    palette_writes(&mut pal.bytes, &writes);
    Ok(())
}

/// The palette RAM after update strings' writes: each byte a write puts
/// at $3F00 to $3F1F replaces that color, in order, over `bytes` (made 32
/// long, 0x0F where it was short). Writes elsewhere are left out.
pub(crate) fn palette_writes(bytes: &mut Vec<u8>, writes: &[gp_assets::ppu::Write]) {
    if bytes.len() < 32 {
        bytes.resize(32, 0x0F);
    }
    for w in writes {
        for (a, &b) in (usize::from(w.ppu)..).zip(&w.bytes) {
            if (0x3F00..0x3F20).contains(&a) {
                bytes[a - 0x3F00] = b;
            }
        }
    }
}

/// Core `level.layout_ppu_strings`: a level drawn from a PPU update
/// string, as the game draws the screen. Its name table cells are drawn
/// with the level's tiles slot (the one no sprite uses) in the background
/// palette the string's attribute bytes give each 2 by 2 cell block, color
/// 0 as the backdrop. A cell the string doesn't write shows the backdrop.
/// The level's changed cells have no metatiles, so `present.cells` draws
/// each kind's as one tile, its number the kind's, in background palette 0
/// with color 0 the backdrop (a blank tile shows the backdrop).
#[allow(clippy::too_many_arguments)]
fn ppu_layout_picture(
    model: &Model,
    files: &Files,
    at: &str,
    ps: &gp_assets::ppu::PpuStrings,
    asset: &Asset,
    slot: &str,
    entry: &str,
    size: [u32; 2],
    palettes: &Palettes,
) -> Result<(Indexed, LevelTiles), String> {
    // The layout's slot may name the tiles it is drawn with
    // (`assets.<slot>.tiles`, feature `assets.layout_tiles`), else the
    // level's: a stage the game draws with another CHR bank.
    let own = model.assets.get(slot).and_then(|s| s.tiles.clone());
    let Some(tiles_slot) = own.or_else(|| level_tiles_slot(model)) else {
        return Err(format!(
            "{at}: no one tiles slot to draw it with (one slot, or one no sprite uses)"
        ));
    };
    let tiles = files.tiles_slot(model, &tiles_slot)?;
    let text = std::fs::read_to_string(files.dir.join(asset.file()))
        .map_err(|e| format!("{}: {e}", asset.file()))?;
    let writes = ps.writes(&text, entry).map_err(|e| format!("{at}: {e}"))?;
    let (names, attrs) = name_table(&writes);
    let (cols, rows) = (size[0].min(32), size[1].min(30));
    // Color 0 is left clear, so the backdrop (the scene's clear color)
    // shows there and a sprite behind the background (attribute bit 5)
    // shows through it, as the PPU draws: Donkey Kong's 75m elevators in
    // the pulleys ($E968).
    let mut ix = Indexed::new(size[0] * 8, size[1] * 8);
    ix.roles.fill(role_id("sky"));
    for r in 0..rows {
        for c in 0..cols {
            let Some(tile) = names[(r * 32 + c) as usize] else {
                continue;
            };
            let a = attrs[((r / 4) * 8 + c / 4) as usize];
            let shift = ((r % 4) / 2) * 4 + ((c % 4) / 2) * 2;
            let pal = (a >> shift) & 3;
            ix.tile(
                &tiles,
                u32::from(tile),
                c * 8,
                r * 8,
                pal,
                (false, false),
                false,
                palettes.role(usize::from(pal)),
            );
        }
    }
    Ok((
        ix,
        LevelTiles {
            cells: vec![None; (size[0] * size[1]) as usize],
            quads: vec![None; 256],
            tiles,
            single: true,
        },
    ))
}

/// A level's metatiles: the one each cell is drawn with (None above a
/// layout's buffer), and each metatile's four tiles, palette and role.
struct LevelTiles {
    cells: Vec<Option<u8>>,
    quads: Vec<Option<([u32; 4], u8, u8)>>,
    tiles: Tiles,
    /// A cell is one tile (a PPU string's name table), not a metatile.
    single: bool,
}

impl LevelTiles {
    /// Metatile `mt` alone, 16 pixels square. `opaque` draws color 0 as
    /// the backdrop, as the level's picture does; without it color 0 is
    /// clear, as a sprite's is. A single tile's ink is in role `ink`
    /// (`sky` when None, as the backdrop's).
    fn metatile(&self, mt: u8, opaque: bool, ink: Option<u8>) -> Option<Indexed> {
        // One tile in background palette 0, 8 pixels square.
        if self.single {
            let mut ix = Indexed::new(8, 8);
            ix.tile(
                &self.tiles,
                u32::from(mt),
                0,
                0,
                0,
                (false, false),
                opaque,
                ink.unwrap_or_else(|| role_id("sky")),
            );
            return Some(ix);
        }
        let (quad, palette, role) = self.quads.get(usize::from(mt)).copied().flatten()?;
        let mut ix = Indexed::new(16, 16);
        for (q, &tile) in quad.iter().enumerate() {
            let (ox, oy) = ((q as u32 % 2) * 8, (q as u32 / 2) * 8);
            ix.tile(
                &self.tiles,
                tile,
                ox,
                oy,
                palette,
                (false, false),
                opaque,
                role,
            );
        }
        Some(ix)
    }
}

/// How a level draws the cells the game changes: each kind `cells.kinds`
/// names drawn from its metatile, as the level's picture draws it and as a
/// sprite with its backdrop clear; the held cell's metatile; and the own
/// picture, as a sprite, of each cell a head hit can hold (a coin block,
/// an item block or a brick as built).
#[allow(clippy::too_many_arguments)]
fn cell_look(
    art: &mut Art,
    theme: &Resolved,
    cp: &gp_model::model::CellsPresent,
    lt: &LevelTiles,
    bump_tiles: &BTreeMap<String, Tiles>,
    level: &gp_sim::spec::Level,
    name: &str,
    color: &dyn Fn(u8) -> [u8; 4],
) -> Result<gp_present::CellLook, String> {
    use gp_sim::spec::Tile;
    // A level whose cells are single tiles (a PPU string's name table,
    // Donkey Kong) has no role for a tile: a cell the game changes to a
    // kind that isn't empty (100m's ending lays a girder under the fallen
    // foe) is drawn in role `terrain`, so a theme colors it as ground,
    // not as the backdrop it tints (paper's cream, 2026-11-07).
    let ink_of = |t: Tile| (lt.single && t != Tile::Empty).then(|| role_id("terrain"));
    let mut pics: BTreeMap<(u8, bool, Option<u8>), u32> = BTreeMap::new();
    let mut pic = |art: &mut Art, mt: u8, opaque: bool, ink: Option<u8>| -> Result<u32, String> {
        if let Some(&p) = pics.get(&(mt, opaque, ink)) {
            return Ok(p);
        }
        let ix = lt.metatile(mt, opaque, ink).ok_or(format!(
            "present.cells: metatile {mt:#04x} isn't in level `{name}`'s tileset"
        ))?;
        let img = ix.role_image(color);
        let file = format!(
            "level-{name}-mt{mt:02x}{}{}.png",
            if opaque { "" } else { "-sprite" },
            if ink.is_some() { "-terrain" } else { "" }
        );
        let p = art.add_themed(theme, file, &img);
        pics.insert((mt, opaque, ink), p);
        Ok(p)
    };
    // A bump's own picture for a kind, as a sprite: color 0 clear.
    let mut bumps: BTreeMap<u8, u32> = BTreeMap::new();
    for (kind, t) in cp.bump.iter().flat_map(|b| &b.sprites) {
        let k = gp_model::model::TileKind::parse(kind).ok_or(format!(
            "present.cells.bump.sprites: `{kind}` isn't a tile kind"
        ))?;
        let Some(tiles) = bump_tiles.get(&t.slot) else {
            continue;
        };
        let cols = t.width.max(1);
        let place = |i: usize| {
            t.cells
                .get(i)
                .and_then(|c| c.at)
                .unwrap_or([i as u32 % cols, i as u32 / cols])
        };
        let (w, h) = (0..t.tiles.len()).fold((cols, 1), |(w, h), i| {
            let [c, r] = place(i);
            (w.max(c + 1), h.max(r + 1))
        });
        let mut ix = Indexed::new(w * 8, h * 8);
        for (i, &tile) in t.tiles.iter().enumerate() {
            let cell = t.cells.get(i).cloned().unwrap_or_default();
            let [c, r] = place(i);
            ix.tile(
                tiles,
                tile,
                c * 8,
                r * 8,
                cell.palette.unwrap_or(t.palette) as u8,
                (cell.flip_x, cell.flip_y),
                false,
                role_id("interactive"),
            );
        }
        let img = ix.role_image(color);
        let p = art.add_themed(theme, format!("level-{name}-bump-{kind}.png"), &img);
        bumps.insert(k as u8, p);
    }
    let mut look = gp_present::CellLook::default();
    for (kind, &mt) in &cp.kinds {
        let t = gp_model::model::TileKind::parse(kind)
            .ok_or(format!("present.cells.kinds: `{kind}` isn't a tile kind"))?;
        let t = Tile::from_u8(t as u8).ok_or(format!("present.cells.kinds: `{kind}`"))?;
        let mt = u8::try_from(mt).map_err(|_| format!("present.cells.kinds.{kind}: past 255"))?;
        let bg = pic(art, mt, true, ink_of(t))?;
        let sp = match bumps.get(&(t as u8)) {
            Some(&b) => b,
            None => pic(art, mt, false, ink_of(t))?,
        };
        look.kinds.push((t as u8, bg, sp));
    }
    if let Some(mt) = cp.held {
        let mt = u8::try_from(mt).map_err(|_| "present.cells.held: past 255".to_string())?;
        look.held = Some(pic(art, mt, true, None)?);
    }
    for (c, t) in level.tiles.iter().enumerate() {
        if !matches!(t, Tile::CoinBlock | Tile::ItemBlock | Tile::Brick) {
            continue;
        }
        if let Some(&b) = bumps.get(&(*t as u8)) {
            look.own.push((c as u32, b));
        } else if let Some(Some(mt)) = lt.cells.get(c) {
            look.own.push((c as u32, pic(art, *mt, false, None)?));
        }
    }
    Ok(look)
}

/// The tiles slot a level's metatiles are drawn from: the only one, or else
/// the only one no entity's sprite draws from (Super Mario Bros. has a
/// background bank and a sprite bank). None when that doesn't pick one.
pub fn level_tiles_slot(model: &Model) -> Option<String> {
    let all: Vec<&String> = model
        .assets
        .iter()
        .filter(|(_, s)| s.kind == AssetKind::Tiles)
        .map(|(n, _)| n)
        .collect();
    if all.len() == 1 {
        return Some(all[0].clone());
    }
    // A model whose sprites draw from the background's table too (Donkey
    // Kong, drawn into the name table) names its level's slot by role.
    let tilesets: Vec<&&String> = all
        .iter()
        .filter(|n| {
            model
                .assets
                .get(n.as_str())
                .is_some_and(|s| s.role == "tileset")
        })
        .collect();
    if tilesets.len() == 1 {
        return Some(tilesets[0].to_string());
    }
    let sprites: Vec<&str> = model
        .entities
        .values()
        .filter_map(|e| e.core.sprite.as_ref().map(|s| s.slot.as_str()))
        .collect();
    let rest: Vec<&&String> = all
        .iter()
        .filter(|n| !sprites.contains(&n.as_str()))
        .collect();
    (rest.len() == 1).then(|| rest[0].to_string())
}

fn picture(file: &str, width: u32, height: u32, rgba: &[u8]) -> Picture {
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("PNG header into memory");
        w.write_image_data(rgba).expect("PNG data into memory");
        w.finish().expect("PNG end into memory");
    }
    Picture {
        file: file.into(),
        width,
        height,
        png,
    }
}

/// The 64 NES colors as RGB: `gp_present::palette::NES`, the table the
/// original's emulator draws with, so the same color number is the same
/// RGB in the rebuild and the original.
pub fn nes_colors() -> [[u8; 3]; 64] {
    gp_present::palette::NES
}

/// A layout entry's name table as the game writes it, for tools that
/// paint over a layout (`gp theme paint`): each of the 32 by 30 cells'
/// tile (None where the entry writes nothing) and palette. A layer that
/// writes no attributes takes the first level's, as the HUD's does.
pub fn layout_cells(
    model: &Model,
    dir: &Path,
    slot: &str,
    entry: &str,
) -> Result<(Vec<Option<u8>>, Vec<u8>), String> {
    let map_file = dir.join("assets.toml");
    let text =
        std::fs::read_to_string(&map_file).map_err(|e| format!("{}: {e}", map_file.display()))?;
    let map =
        gp_interp::lifted::map_of(&text).map_err(|e| format!("{}: {e}", map_file.display()))?;
    let files = Files {
        dir,
        map,
        tiles_from: BTreeMap::new(),
        tiles_from_slot: BTreeMap::new(),
        pictures: BTreeMap::new(),
    };
    let kind = model
        .assets
        .get(slot)
        .map(|s| s.kind)
        .ok_or(format!("the model has no asset slot `{slot}`"))?;
    let asset = files.asset(model, slot, kind)?;
    let Asset::PpuStrings(ps) = asset else {
        return Err(format!("slot `{slot}` isn't update strings"));
    };
    let text = std::fs::read_to_string(files.dir.join(asset.file()))
        .map_err(|e| format!("{}: {e}", asset.file()))?;
    let writes = ps.writes(&text, entry)?;
    let (names, mut attrs) = name_table(&writes);
    if !writes_attributes(&writes)
        && let Some(level) = level_attributes(model, &files)?
    {
        attrs = level;
    }
    let mut pals = vec![0u8; 32 * 30];
    for r in 0..30usize {
        for c in 0..32usize {
            let a = attrs[(r / 4) * 8 + c / 4];
            let shift = ((r % 4) / 2) * 4 + ((c % 4) / 2) * 2;
            pals[r * 32 + c] = (a >> shift) & 3;
        }
    }
    Ok((names, pals))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sprite whose right half is its left half mirrored, as a game that
    /// saves tiles draws one (Super Mario Bros.' standing Mario's feet):
    /// the right tile must be drawn flipped, or the two halves don't meet
    /// and the background shows through between them. A made-up tile
    /// whose right four columns are color 1 stands for the left half.
    #[test]
    fn a_mirrored_half_closes_the_gap() {
        let mut px = vec![0u8; 64];
        for y in 0..8 {
            for x in 4..8 {
                px[y * 8 + x] = 1;
            }
        }
        let t = Tiles {
            per_row: 1,
            width: 8,
            px,
        };
        let draw = |flip_right: bool| {
            let mut ix = Indexed::new(16, 8);
            ix.tile(&t, 0, 0, 0, 4, (false, false), false, 0);
            ix.tile(&t, 0, 8, 0, 4, (flip_right, false), false, 0);
            (0..16).map(|x| ix.px[x] != NONE).collect::<Vec<bool>>()
        };
        // Drawn as the layout says: one solid block from column 4 to 11.
        let laid_out = draw(true);
        assert!(laid_out[4..12].iter().all(|&o| o), "{laid_out:?}");
        assert!(!laid_out[..4].iter().any(|&o| o) && !laid_out[12..].iter().any(|&o| o));
        // Drawn without the flip, columns 8 to 11 are a hole.
        let unflipped = draw(false);
        assert!(!unflipped[8..12].iter().any(|&o| o), "{unflipped:?}");
    }

    /// An arcade game's lifted folder, as `gp assets lift` writes it for
    /// Tapper: its map names `mcr-tiles` and `mcr-sprites` pictures, which
    /// an NES map hasn't got, beside a `byte-rows` table the model's slot
    /// reads. The model's pictures are drawn from it (by tile kind, as it
    /// names no picture slot) instead of the map being refused, so the
    /// screen measure and the mechanic page's pictures run on it.
    #[test]
    fn an_arcade_lift_draws() {
        let model = Model::parse(
            r#"
[model]
schema = "0.4"
name = "cab"
title = "Cab"
source = "cab"
view = "side"
frame_rate = "30"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]

[model.extensions]
platformer = "0.4"

[input]
device = "arcade"

[input.actions]
left = "left"
right = "right"

[assets.steps]
kind = "rows"
role = "behavior"

[assets.steps.from]
source = "cab"
asset = "steps"

[entities.player]
role = "player"
size = [16, 16]

[entities.player.hitbox]
x = 0
y = 0
w = 16
h = 16

[levels.main]
empty = true
size = [32, 30]
tile_size = 16

[[levels.main.spawn]]
entity = "player"
x = 0
y = 0

[present.screen]
size = [512, 480]
"#,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("assets.toml"),
            "schema = 1\nplatform = \"arcade\"\nsource = \"cab\"\nsets = [\"cab\"]\n\n\
             [[asset]]\nname = \"tiles\"\nformat = \"mcr-tiles\"\nregion = \"gfx1\"\nfirst = 0\ncount = 4\nper_row = 4\n\n\
             [[asset]]\nname = \"sprites\"\nformat = \"mcr-sprites\"\nregion = \"gfx2\"\nfirst = 0\ncount = 4\nper_row = 4\n\n\
             [[asset]]\nname = \"steps\"\nformat = \"byte-rows\"\nregion = \"maincpu\"\noffset = 0x100\ncount = 2\nwidth = 1\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("steps.toml"), "rows = [[0x05], [0xFC]]\n").unwrap();
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = art(&model, &spec, dir.path()).unwrap();
        assert_eq!(art.present.screen.size, [512, 480]);
    }

    #[test]
    fn nes_colors_have_black_gray_and_white() {
        let c = nes_colors();
        assert_eq!(c[0x0F], [3, 3, 3], "$0F is black");
        assert_eq!(c[0x1D], [3, 3, 3], "$1D is black");
        let gray = c[0x00];
        assert!(
            gray[0] == gray[1] && gray[1] == gray[2],
            "$00 is a gray: {gray:?}"
        );
        assert!(
            c[0x30].iter().all(|&v| v > 240),
            "$30 is white: {:?}",
            c[0x30]
        );
        // $16 is a red and $1A a green, $12 a blue.
        assert!(c[0x16][0] > c[0x16][1] && c[0x16][0] > c[0x16][2]);
        assert!(c[0x1A][1] > c[0x1A][0] && c[0x1A][1] > c[0x1A][2]);
        assert!(c[0x12][2] > c[0x12][0] && c[0x12][2] > c[0x12][1]);
    }
}
