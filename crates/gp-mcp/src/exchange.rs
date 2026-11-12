//! The external-editor round trip, minimal form
//! (docs/research/agent-sprite-editor.md, "External editors"):
//! `assets_export` writes one entity's sprite for a pixel editor, and
//! `assets_edit`'s `load` op reads the file the editor saved back through
//! every check an agent's edit gets (readiness, critic, body checks, the
//! simulation digests, history and undo).
//!
//! The files are an indexed PNG in the sprite's own colors with index 0
//! transparent, a GIMP palette (`.gpl`, which Pixelorama, LibreSprite and
//! Aseprite all import) and a JSON sidecar saying what the picture is in
//! the model. The layout is provisional: the image format config (another
//! lane) will name sheet layout, scale, palette format and frame tags, and
//! this module will read it instead of fixing them here.

use std::fs;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::asset_edit::{sprite_colors, sprite_pic};
use crate::critic;
use crate::generate::Candidate;
use crate::place;
use crate::sprite_ops::Pic;
use crate::{Config, Reply};

/// The sidecar's format.
pub const SIDECAR_FORMAT: &str = "gp-sprite-exchange 0";

/// Where a copy keeps exported sprites.
pub const EXCHANGE_DIR: &str = "exchange";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportArgs {
    pub copy: String,
    /// A tiles slot (default `tiles`).
    #[serde(default)]
    pub slot: Option<String>,
    /// The entity whose sprite to export.
    pub entity: String,
    /// The editor the sprite set is written for, by the image format
    /// config (docs/formats/image-config.md): `pixelorama` (default),
    /// `aseprite` (also for LibreSprite) or `png`.
    #[serde(default)]
    pub editor: Option<String>,
}

/// The image config for an export from the server: the defaults, then
/// `$GP_HOME/config/image.toml`, for the template and the entity, with the
/// editor asked for.
pub fn image_config(
    config: &Config,
    template: &str,
    entity: &str,
    editor: &str,
) -> Result<gp_imagefmt::config::ImageConfig, String> {
    use gp_imagefmt::config::{Editor, Layer, PaletteFile};
    let mut layers = Vec::new();
    let home = config.home.join("config").join("image.toml");
    if let Ok(t) = fs::read_to_string(&home) {
        layers.push(Layer::parse(&t).map_err(|e| format!("{}: {e}", home.display()))?);
    }
    let mut cfg = gp_imagefmt::config::resolve(&layers, Some(template), Some(entity))?;
    match editor {
        "pixelorama" => {
            cfg.editor = Editor::Pixelorama;
            if cfg.palette_file == PaletteFile::Gpl {
                cfg.palette_file = PaletteFile::Pixelorama;
            }
        }
        "aseprite" | "libresprite" => cfg.editor = Editor::Aseprite,
        "png" => cfg.editor = Editor::Png,
        e => {
            return Err(format!(
                "editor `{e}` isn't pixelorama, aseprite, libresprite or png"
            ));
        }
    }
    Ok(cfg)
}

/// An indexed 8-bit PNG with the given colors, index 0 transparent.
pub fn write_indexed(p: &Pic, colors: [[u8; 3]; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, p.w, p.h);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(colors.concat());
    enc.set_trns(vec![0u8, 255, 255, 255]);
    let mut wr = enc.write_header().expect("PNG header into memory");
    wr.write_image_data(&p.px).expect("PNG data into memory");
    wr.finish().expect("PNG end into memory");
    out
}

/// A GIMP palette of the sprite's four colors.
pub fn gpl(name: &str, colors: [[u8; 3]; 4], nes: &[u8]) -> String {
    let mut s = format!("GIMP Palette\nName: {name}\nColumns: 4\n#\n");
    for (i, c) in colors.iter().enumerate() {
        let label = match (i, nes.get(i)) {
            (0, _) => "transparent".to_owned(),
            (_, Some(n)) => format!("index {i} NES ${n:02X}"),
            _ => format!("index {i}"),
        };
        s.push_str(&format!("{:3} {:3} {:3}\t{label}\n", c[0], c[1], c[2]));
    }
    s
}

/// Reads an image an editor saved, held to the sprite's size and colors:
/// an indexed PNG's indices, or an RGB(A) PNG's colors matched exactly to
/// the sprite's (alpha under 128 is transparent). Every pixel that fits
/// neither is listed, up to ten.
pub fn read_image(path: &Path, want: (u32, u32), colors: [[u8; 3]; 4]) -> Result<Pic, String> {
    let bytes = fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    if bytes.len() > place::MAX_IMAGE_BYTES {
        return Err(format!("{} is over 1 MB", path.display()));
    }
    let head = png::Decoder::new(std::io::Cursor::new(&bytes))
        .read_info()
        .map_err(|e| format!("{}: not a PNG ({e})", path.display()))?;
    let indexed = head.info().color_type == png::ColorType::Indexed;
    drop(head);
    let px = if indexed {
        gp_assets::chr::read_png(&bytes, want).map_err(|e| format!("{}: {e}", path.display()))?
    } else {
        let mut dec = png::Decoder::new(std::io::Cursor::new(&bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut r = dec
            .read_info()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let (w, h) = (r.info().width, r.info().height);
        if (w, h) != want {
            return Err(format!(
                "{} is {w}x{h}; the sprite is {}x{}",
                path.display(),
                want.0,
                want.1
            ));
        }
        let mut buf = vec![0u8; r.output_buffer_size().ok_or("image too large")?];
        let info = r
            .next_frame(&mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let step = info.color_type.samples();
        if step < 3 {
            return Err(format!(
                "{}: a gray image; save it indexed or in color",
                path.display()
            ));
        }
        let mut px = Vec::with_capacity((w * h) as usize);
        let mut bad = Vec::new();
        for (i, c) in buf[..info.buffer_size()].chunks(step).enumerate() {
            let alpha = if step == 4 { c[3] } else { 255 };
            let rgb = [c[0], c[1], c[2]];
            let v = if alpha < 128 {
                Some(0)
            } else {
                (1..4u8).find(|&k| colors[k as usize] == rgb)
            };
            match v {
                Some(v) => px.push(v),
                None => {
                    if bad.len() < 10 {
                        bad.push(format!(
                            "({}, {}) #{:02x}{:02x}{:02x}",
                            i as u32 % w,
                            i as u32 / w,
                            rgb[0],
                            rgb[1],
                            rgb[2]
                        ));
                    }
                    px.push(0);
                }
            }
        }
        if !bad.is_empty() {
            return Err(format!(
                "{}: pixels in colors the sprite doesn't have (use the exported palette): {}",
                path.display(),
                bad.join(", ")
            ));
        }
        px
    };
    if let Some(i) = px.iter().position(|&v| v > 3) {
        return Err(format!(
            "{}: pixel ({}, {}) has index {}; the sprite has 4 colors, 0 to 3",
            path.display(),
            i as u32 % want.0,
            i as u32 / want.0,
            px[i]
        ));
    }
    Pic::new(want.0, want.1, px)
}

pub fn export(config: &Config, a: ExportArgs) -> Result<Reply, String> {
    let slot = a.slot.clone().unwrap_or_else(|| "tiles".to_owned());
    let t = place::target(config, &a.copy, &slot, Some(&a.entity))?;
    let (assets, _) = place::current_assets(config, &t.dir, &t.model)?;
    let seen = crate::contexts::contexts(&t, &assets, &a.entity)?;
    let ready = crate::ready::readiness(&t, &a.entity, seen["seen"].as_u64().unwrap_or(0));
    // The whole sprite set, by the image format config: what an editor
    // round trip uses (crate::editor_trip).
    let template = t
        .model
        .model
        .source
        .clone()
        .unwrap_or_else(|| t.model.model.name.clone());
    let editor = a.editor.clone().unwrap_or_else(|| "pixelorama".to_owned());
    let cfg = image_config(config, &template, &a.entity, &editor)?;
    let set_dir = t.dir.join(EXCHANGE_DIR).join(&a.entity).join("set");
    let (trip, set_files) =
        crate::editor_trip::export(config, &a.copy, &a.entity, &slot, &cfg, &set_dir)?;
    let set = json!({
        "dir": set_dir,
        "files": set_files,
        "frames": trip.frames,
        "config": cfg,
        "import": { "tool": "assets_edit", "args": { "copy": a.copy, "import": set_dir } },
    });
    // The sprite's own frame as one PNG and a .gpl too, when it's a grid of
    // tiles (the prototype's files, read back by the load op).
    let Ok(pic) = sprite_pic(&t) else {
        return Ok(Reply::ok(json!({
            "ok": true, "set": set, "readiness": ready,
            "open_with": [format!("pixelorama {}", set_dir.join(trip.images.first().cloned().unwrap_or_default()).display())],
            "import": set["import"],
        })));
    };
    let subject = critic::subject(config, &t, &Candidate::Pixels(pic.px.clone()), None)?;
    let colors = sprite_colors(&subject);
    let nes: Vec<u8> = subject.palette.map(|p| p.to_vec()).unwrap_or_default();
    let dir: PathBuf = t.dir.join(EXCHANGE_DIR).join(&a.entity);
    fs::create_dir_all(&dir).map_err(|e| format!("making {}: {e}", dir.display()))?;
    let png_path = dir.join(format!("{}.png", a.entity));
    let gpl_path = dir.join(format!("{}.gpl", a.entity));
    let side_path = dir.join(format!("{}.json", a.entity));
    let sp = t.sprite.as_ref().ok_or("no sprite")?;
    fs::write(&png_path, write_indexed(&pic, colors))
        .map_err(|e| format!("writing {}: {e}", png_path.display()))?;
    fs::write(
        &gpl_path,
        gpl(
            &format!("{} {}", t.model.model.name, a.entity),
            colors,
            &nes,
        ),
    )
    .map_err(|e| format!("writing {}: {e}", gpl_path.display()))?;
    let sidecar = json!({
        "format": SIDECAR_FORMAT,
        "model": t.model.model.name,
        "copy": a.copy,
        "slot": slot,
        "entity": a.entity,
        "role": t.role(),
        "image": png_path.file_name().map(|f| f.to_string_lossy().into_owned()),
        "palette_file": gpl_path.file_name().map(|f| f.to_string_lossy().into_owned()),
        "image_size": [pic.w, pic.h],
        "tiles": sp.tiles,
        "tiles_per_row": sp.width,
        "palette": { "index": sp.palette, "nes": nes, "transparent_index": 0 },
        "flip_when_facing_left": sp.flip_when_facing_left,
        "frames": [{ "name": "frame", "tag": "default", "poses": seen["poses"] }],
        "rules": [
            "keep the size and the four colors of the palette file",
            "index 0 (the first palette entry) is transparent",
            "save as PNG over this file, indexed or RGB(A)",
        ],
        "readiness": ready,
        "import": {
            "tool": "assets_edit",
            "ops": [{ "op": "load", "file": png_path }],
        },
    });
    fs::write(
        &side_path,
        serde_json::to_string_pretty(&sidecar).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("writing {}: {e}", side_path.display()))?;
    Ok(Reply::ok(json!({
        "ok": true,
        "image": png_path,
        "palette_file": gpl_path,
        "sidecar": side_path,
        "image_size": [pic.w, pic.h],
        "readiness": ready,
        "open_with": [
            format!("pixelorama {}", png_path.display()),
            format!("libresprite {}", png_path.display()),
        ],
        "import": sidecar["import"],
        "set": set,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLORS: [[u8; 3]; 4] = [[0, 0, 0], [10, 20, 30], [40, 50, 60], [70, 80, 90]];

    #[test]
    fn indexed_and_rgba_files_read_back_to_the_same_indices() {
        let dir = tempfile::tempdir().unwrap();
        let p = Pic::new(8, 2, (0..16).map(|i| (i % 4) as u8).collect()).unwrap();
        let f = dir.path().join("s.png");
        fs::write(&f, write_indexed(&p, COLORS)).unwrap();
        assert_eq!(read_image(&f, (8, 2), COLORS).unwrap(), p);

        // The same picture as an editor's RGBA export.
        let mut rgba = Vec::new();
        for &v in &p.px {
            let c = COLORS[v as usize];
            rgba.extend_from_slice(&[c[0], c[1], c[2], if v == 0 { 0 } else { 255 }]);
        }
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, 8, 2);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&rgba).unwrap();
        w.finish().unwrap();
        fs::write(&f, &out).unwrap();
        assert_eq!(read_image(&f, (8, 2), COLORS).unwrap(), p);

        // A color off the palette and a wrong size are refused.
        rgba[4..8].copy_from_slice(&[1, 2, 3, 255]);
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, 8, 2);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&rgba).unwrap();
        w.finish().unwrap();
        fs::write(&f, &out).unwrap();
        let err = read_image(&f, (8, 2), COLORS).unwrap_err();
        assert!(err.contains("(1, 0) #010203"), "{err}");
        assert!(read_image(&f, (16, 2), COLORS).is_err());
    }

    #[test]
    fn the_palette_file_is_gimp_format() {
        let s = gpl("tiny hero", COLORS, &[0x0F, 0x01, 0x11, 0x21]);
        assert!(s.starts_with("GIMP Palette\nName: tiny hero\n"));
        assert!(s.contains("index 3 NES $21"));
        assert_eq!(s.lines().count(), 8);
    }
}
