//! Sprite frames to image files and back, in the layout a resolved
//! [`ImageConfig`] says, with a JSON sidecar that names every frame, its
//! place in the image, its tiles and the palette, so a frame edited in
//! any editor comes back to the same entity, state and tiles.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::{Color, Editor, ImageConfig, Layout, Order, Transparency};

/// The sidecar's format.
pub const SIDECAR_FORMAT: &str = "gp-sprite-sheet 0.1.0";
/// The sidecar's file name.
pub const SIDECAR: &str = "sprites.json";

/// One tile of a frame, from the frame's top left, with its palette (0 to
/// 3, the sprite palettes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub palette: u8,
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
}

/// One frame's pixels: entry 4p + c is color c of sprite palette p, and
/// color 0 of any palette is transparent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub name: String,
    pub entity: String,
    pub state: String,
    /// Its number in the sprite set.
    pub set_index: u32,
    pub width: u32,
    pub height: u32,
    /// Its top left from the entity's position.
    pub origin: [i32; 2],
    pub tiles: Vec<Tile>,
    pub px: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideFrame {
    pub name: String,
    pub entity: String,
    pub state: String,
    pub set_index: u32,
    pub file: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub origin: [i32; 2],
    pub tiles: Vec<Tile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    pub format: String,
    pub config: ImageConfig,
    /// The 16 sprite palette colors, `rrggbb`, entry by entry.
    pub palette: Vec<String>,
    pub frames: Vec<SideFrame>,
    #[serde(default)]
    pub tags: Vec<Tag>,
}

fn transparent(e: u8) -> bool {
    e.is_multiple_of(4)
}

fn png_indexed(w: u32, h: u32, px: &[u8], palette: &[[u8; 3]; 16]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(palette.as_flattened().to_vec());
        let trns: Vec<u8> = (0..16)
            .map(|e| if transparent(e) { 0 } else { 255 })
            .collect();
        enc.set_trns(trns);
        let mut wr = enc.write_header().expect("a png header");
        wr.write_image_data(px).expect("png data");
    }
    out
}

fn png_rgba(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().expect("a png header");
        wr.write_image_data(rgba).expect("png data");
    }
    out
}

/// Pixels of an image: indices for an indexed PNG (Ok(Left)), RGBA else.
pub enum Pixels {
    Indexed(Vec<u8>),
    Rgba(Vec<[u8; 4]>),
}

/// Reads a PNG: its size and pixels.
pub fn read_png(bytes: &[u8]) -> Result<(u32, u32, Pixels), String> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::IDENTITY);
    let info = d.read_header_info().map_err(|e| e.to_string())?;
    let indexed = info.color_type == png::ColorType::Indexed;
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    if !indexed {
        d.set_transformations(
            png::Transformations::EXPAND
                | png::Transformations::STRIP_16
                | png::Transformations::ALPHA,
        );
    }
    let mut r = d.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("the image is too large")?];
    let f = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (f.width, f.height);
    if indexed {
        let depth = f.bit_depth as u32;
        let mut px = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            let line = &buf[y * f.line_size..(y + 1) * f.line_size];
            for x in 0..w {
                let bit = x * depth;
                let shift = 8 - depth - bit % 8;
                px.push((line[(bit / 8) as usize] >> shift) & ((1u16 << depth) - 1) as u8);
            }
        }
        return Ok((w, h, Pixels::Indexed(px)));
    }
    let ch = f.color_type.samples();
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        let line = &buf[y * f.line_size..(y + 1) * f.line_size];
        for x in 0..w as usize {
            let p = &line[x * ch..x * ch + ch];
            px.push(match ch {
                4 => [p[0], p[1], p[2], p[3]],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[0], p[0], 255],
            });
        }
    }
    Ok((w, h, Pixels::Rgba(px)))
}

/// Where each frame goes: its cell's top left in the sheet (or 0, 0 in
/// its own file), and the sheet's size.
fn place(cfg: &ImageConfig, frames: &[Frame]) -> (Vec<(u32, u32)>, u32, u32) {
    let cw = if cfg.cell[0] > 0 {
        cfg.cell[0]
    } else {
        frames.iter().map(|f| f.width).max().unwrap_or(1)
    };
    let ch = if cfg.cell[1] > 0 {
        cfg.cell[1]
    } else {
        frames.iter().map(|f| f.height).max().unwrap_or(1)
    };
    let (cw, ch) = (cw + 2 * cfg.padding, ch + 2 * cfg.padding);
    let mut at = Vec::new();
    let (mut row, mut col) = (0u32, 0u32);
    let (mut cols, mut rows) = (0u32, 0u32);
    let mut last_state: Option<(&str, &str)> = None;
    for f in frames {
        match cfg.order {
            Order::StateFrame => {
                let key = (f.entity.as_str(), f.state.as_str());
                if last_state.is_some_and(|k| k != key) {
                    row += 1;
                    col = 0;
                }
                last_state = Some(key);
            }
            Order::Frame => {
                if col == cfg.columns {
                    row += 1;
                    col = 0;
                }
            }
        }
        at.push((
            col * (cw + cfg.spacing) + cfg.padding,
            row * (ch + cfg.spacing) + cfg.padding,
        ));
        col += 1;
        cols = cols.max(col);
        rows = row + 1;
    }
    let w = cols * (cw + cfg.spacing) - cfg.spacing.min(cols * (cw + cfg.spacing));
    let h = rows * (ch + cfg.spacing) - cfg.spacing.min(rows * (ch + cfg.spacing));
    (at, w.max(1), h.max(1))
}

/// The files for `frames`, as (name, bytes): images, the palette file and
/// the sidecar (and Aseprite's sheet JSON for that editor).
pub fn write(
    cfg: &ImageConfig,
    title: &str,
    frames: &[Frame],
    palette: &[[u8; 3]; 16],
) -> Vec<(String, Vec<u8>)> {
    let s = cfg.scale;
    let key = crate::palette::parse_hex(&cfg.key).unwrap_or([255, 0, 255]);
    let rgba_of = |e: u8| -> [u8; 4] {
        if transparent(e) {
            match cfg.transparency {
                Transparency::Key => [key[0], key[1], key[2], 255],
                Transparency::Alpha => [0, 0, 0, 0],
            }
        } else {
            let c = palette[usize::from(e & 15)];
            [c[0], c[1], c[2], 255]
        }
    };
    let encode = |w: u32, h: u32, px: &[u8]| -> Vec<u8> {
        // Scaled up by repeating pixels.
        let (sw, sh) = (w * s, h * s);
        let mut big = vec![0u8; (sw * sh) as usize];
        for y in 0..sh {
            for x in 0..sw {
                big[(y * sw + x) as usize] = px[((y / s) * w + x / s) as usize];
            }
        }
        match cfg.color {
            Color::Indexed => png_indexed(sw, sh, &big, palette),
            Color::Rgba => png_rgba(
                sw,
                sh,
                &big.iter().flat_map(|&e| rgba_of(e)).collect::<Vec<u8>>(),
            ),
        }
    };
    let mut out = Vec::new();
    let mut side = Vec::new();
    match cfg.layout {
        Layout::Sheet => {
            let (at, w, h) = place(cfg, frames);
            let mut px = vec![0u8; (w * h) as usize];
            for (f, &(x0, y0)) in frames.iter().zip(&at) {
                for y in 0..f.height {
                    for x in 0..f.width {
                        let (sx, sy) = (x0 + x, y0 + y);
                        if sx < w && sy < h {
                            px[(sy * w + sx) as usize] = f.px[(y * f.width + x) as usize];
                        }
                    }
                }
                side.push((f, "sheet.png".to_owned(), x0 * s, y0 * s));
            }
            out.push(("sheet.png".to_owned(), encode(w, h, &px)));
        }
        Layout::Frames => {
            for f in frames {
                let file = format!("{}.png", f.name);
                out.push((file.clone(), encode(f.width, f.height, &f.px)));
                side.push((f, file, 0, 0));
            }
        }
    }
    let pal: Vec<[u8; 3]> = palette.to_vec();
    if let Some(ext) = cfg.palette_file.extension() {
        let name = if ext == "json" {
            "palette.json".to_owned()
        } else {
            format!("palette.{ext}")
        };
        out.push((
            name,
            crate::palette::write(cfg.palette_file, title, &pal).into_bytes(),
        ));
    }
    let sframes: Vec<SideFrame> = side
        .iter()
        .map(|(f, file, x, y)| SideFrame {
            name: f.name.clone(),
            entity: f.entity.clone(),
            state: f.state.clone(),
            set_index: f.set_index,
            file: file.clone(),
            x: *x,
            y: *y,
            w: f.width * s,
            h: f.height * s,
            origin: f.origin,
            tiles: f.tiles.clone(),
        })
        .collect();
    let mut tags: Vec<Tag> = Vec::new();
    if cfg.state_tags {
        for (i, f) in sframes.iter().enumerate() {
            let name = format!(
                "{} {}",
                f.entity,
                if f.state.is_empty() {
                    "default"
                } else {
                    &f.state
                }
            );
            match tags.last_mut() {
                Some(t) if t.name == name && t.to + 1 == i => t.to = i,
                _ => tags.push(Tag {
                    name,
                    from: i,
                    to: i,
                }),
            }
        }
    }
    if cfg.editor == Editor::Aseprite {
        let mut fr = serde_json::Map::new();
        for f in &sframes {
            fr.insert(
                f.name.clone(),
                serde_json::json!({ "frame": { "x": f.x, "y": f.y, "w": f.w, "h": f.h }, "duration": 100 }),
            );
        }
        let v = serde_json::json!({
            "frames": fr,
            "meta": {
                "app": "gp sprites export",
                "image": sframes.first().map(|f| f.file.clone()).unwrap_or_default(),
                "frameTags": tags.iter().map(|t| serde_json::json!({ "name": t.name, "from": t.from, "to": t.to, "direction": "forward" })).collect::<Vec<_>>(),
            }
        });
        out.push((
            "sprites.aseprite.json".into(),
            (serde_json::to_string_pretty(&v).expect("json") + "\n").into_bytes(),
        ));
    }
    let sc = Sidecar {
        format: SIDECAR_FORMAT.into(),
        config: cfg.clone(),
        palette: pal.iter().map(|c| crate::palette::hex(*c)).collect(),
        frames: sframes,
        tags,
    };
    out.push((
        SIDECAR.into(),
        (serde_json::to_string_pretty(&sc).expect("json") + "\n").into_bytes(),
    ));
    out
}

/// A problem an import found, with the frame it's in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub frame: String,
    pub message: String,
}

/// What an import read and checked.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Imported {
    #[serde(skip)]
    pub frames: Vec<Frame>,
    pub problems: Vec<Problem>,
    /// Distinct tiles, after flips, and how many flips saved.
    pub tiles: usize,
    pub saved_by_flips: usize,
}

/// Reads frames back from files `gp sprites export` wrote, and checks each
/// against its tiles: every pixel a color of the palette, each tile one
/// palette's colors, nothing drawn outside the tiles, and the distinct
/// tiles (a tile and its mirror count once) within the budget.
pub fn read(
    read_file: &dyn Fn(&str) -> Option<Vec<u8>>,
    budget_override: Option<u32>,
) -> Result<Imported, String> {
    let side = read_file(SIDECAR).ok_or(format!("no {SIDECAR}"))?;
    let sc: Sidecar = serde_json::from_slice(&side).map_err(|e| format!("{SIDECAR}: {e}"))?;
    if sc.format != SIDECAR_FORMAT {
        return Err(format!(
            "{SIDECAR}: format {} isn't {SIDECAR_FORMAT}",
            sc.format
        ));
    }
    let palette: Vec<[u8; 3]> = sc
        .palette
        .iter()
        .map(|h| crate::palette::parse_hex(h).ok_or(format!("{SIDECAR}: `{h}` isn't a color")))
        .collect::<Result<_, _>>()?;
    let cfg = &sc.config;
    let key = crate::palette::parse_hex(&cfg.key).unwrap_or([255, 0, 255]);
    let s = cfg.scale.max(1);
    let mut images: std::collections::BTreeMap<String, (u32, u32, Pixels)> = Default::default();
    let mut out = Imported::default();
    let mut distinct: BTreeSet<[u8; 64]> = BTreeSet::new();
    let mut plain: BTreeSet<[u8; 64]> = BTreeSet::new();
    for f in &sc.frames {
        if !images.contains_key(&f.file) {
            let bytes = read_file(&f.file).ok_or(format!("no {}", f.file))?;
            images.insert(
                f.file.clone(),
                read_png(&bytes).map_err(|e| format!("{}: {e}", f.file))?,
            );
        }
        let (iw, ih, img) = &images[&f.file];
        let (w, h) = (f.w / s, f.h / s);
        let mut problem = |m: String| {
            out.problems.push(Problem {
                frame: f.name.clone(),
                message: m,
            })
        };
        if f.x + f.w > *iw || f.y + f.h > *ih {
            problem(format!(
                "its place ({}, {}, {}x{}) is past the image's {iw}x{ih}",
                f.x, f.y, f.w, f.h
            ));
            continue;
        }
        // The palette a pixel's tile uses, for RGBA colors.
        let tile_of = |x: u32, y: u32| {
            f.tiles
                .iter()
                .find(|t| x >= t.x && x < t.x + 8 && y >= t.y && y < t.y + 8)
        };
        let mut px = vec![0u8; (w * h) as usize];
        let mut bad = 0usize;
        for y in 0..h {
            for x in 0..w {
                let at = ((f.y + y * s) * iw + f.x + x * s) as usize;
                let e = match img {
                    Pixels::Indexed(v) => Some(v[at]),
                    Pixels::Rgba(v) => {
                        let c = v[at];
                        let clear =
                            c[3] == 0 || (cfg.transparency == Transparency::Key && c[..3] == key);
                        if clear {
                            Some(0)
                        } else {
                            let pal = tile_of(x, y).map(|t| t.palette);
                            (1u8..16)
                                .filter(|e| !transparent(*e))
                                .filter(|e| pal.is_none_or(|p| e / 4 == p))
                                .find(|e| palette[usize::from(*e)] == [c[0], c[1], c[2]])
                        }
                    }
                };
                match e {
                    Some(e) if e < 16 => px[(y * w + x) as usize] = e,
                    _ => bad += 1,
                }
            }
        }
        if bad > 0 {
            problem(format!(
                "{bad} pixels aren't a color of the palette (or of their tile's palette)"
            ));
        }
        // Each tile in one palette; nothing outside the tiles.
        let mut outside = 0usize;
        for y in 0..h {
            for x in 0..w {
                let e = px[(y * w + x) as usize];
                if !transparent(e) {
                    match tile_of(x, y) {
                        None => outside += 1,
                        Some(t) if e / 4 != t.palette => {
                            problem(format!(
                                "the tile at ({}, {}) mixes palettes: pixel ({x}, {y}) is palette {}, the tile's is {}",
                                t.x,
                                t.y,
                                e / 4,
                                t.palette
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }
        if outside > 0 {
            problem(format!(
                "{outside} pixels are drawn outside the frame's tiles"
            ));
        }
        for t in &f.tiles {
            let mut tile = [0u8; 64];
            for y in 0..8 {
                for x in 0..8 {
                    let (sx, sy) = (t.x + x, t.y + y);
                    if sx < w && sy < h {
                        tile[(y * 8 + x) as usize] = px[(sy * w + sx) as usize] % 4;
                    }
                }
            }
            let flips = [
                tile,
                std::array::from_fn(|i| tile[(i / 8) * 8 + 7 - i % 8]),
                std::array::from_fn(|i| tile[(7 - i / 8) * 8 + i % 8]),
                std::array::from_fn(|i| tile[63 - i]),
            ];
            let canon = *flips.iter().min().expect("four");
            if tile.iter().any(|&c| c != 0) {
                distinct.insert(canon);
                plain.insert(tile);
            }
        }
        out.frames.push(Frame {
            name: f.name.clone(),
            entity: f.entity.clone(),
            state: f.state.clone(),
            set_index: f.set_index,
            width: w,
            height: h,
            origin: f.origin,
            tiles: f.tiles.clone(),
            px,
        });
    }
    out.tiles = distinct.len();
    out.saved_by_flips = plain.len() - distinct.len();
    let budget = budget_override.unwrap_or(cfg.tile_budget);
    if budget > 0 && out.tiles > budget as usize {
        out.problems.push(Problem {
            frame: String::new(),
            message: format!(
                "{} distinct tiles, more than the budget of {budget}",
                out.tiles
            ),
        });
    }
    Ok(out)
}

/// A frame as RGBA (transparent pixels clear), for a theme.
pub fn rgba(f: &Frame, palette: &[[u8; 3]; 16]) -> Vec<u8> {
    f.px.iter()
        .flat_map(|&e| {
            if transparent(e) {
                [0, 0, 0, 0]
            } else {
                let c = palette[usize::from(e)];
                [c[0], c[1], c[2], 255]
            }
        })
        .collect()
}

/// An RGBA PNG of a frame.
pub fn rgba_png(f: &Frame, palette: &[[u8; 3]; 16]) -> Vec<u8> {
    png_rgba(f.width, f.height, &rgba(f, palette))
}
