//! `gp theme fit --large` (docs/theme-kit.md, "Fitting a large-art
//! theme"): the filled grids of `gp theme sheet`, fitted into a large-art
//! theme folder (the one `gp theme look new` writes, the Turnips shape):
//! one picture for each game picture a cell stands for, at the theme's
//! `scale`, placed by the original's body in that frame from the kit's
//! local part, a `pic` line for each in theme/art.txt, and
//! theme/made-from.txt naming the cell each came from.
//!
//! The keying is fit's own ([`crate::theme_fit::clear_background`]: alpha,
//! the key color, or a flood from the edges) and so is the scaling: a pixel
//! of the picture is solid when half its area is figure, and takes the
//! mean color of the figure's pixels in it only, so no backdrop color
//! reaches an edge. No glow or rim is added.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use gp_theme::pix::Rgba;
use serde_json::json;

use crate::Outcome;
use crate::theme_fit::{clear_background, drop_lone, opaque_box};
use crate::theme_geometry::Body;
use crate::theme_kit::{ArtSlot, Kit, Local};
use crate::theme_sheet::{Manifest, SheetCell};

/// The record of where a theme's pictures came from, in its theme/ folder.
pub(crate) const MADE_FROM: &str = "made-from.txt";
/// Its first line.
pub(crate) const MADE_FROM_FORMAT: &str = "gp-made-from 1";

/// One game picture's new picture: the art at the theme's scale, the game
/// picture's size in game pixels, and the cell it came from.
struct Drawn {
    img: Rgba,
    size: [u32; 2],
}

/// A cell's figure placed for one frame: the picture `size` game pixels at
/// `scale`, the figure moved and scaled so its box fills the original's
/// `body` (game pixels, `[x0, y0, x1, y1]`) and stands on its bottom,
/// centered on its middle, at one scale both ways. Without a body the
/// figure keeps its place against the cell's box. `src` is the cell's
/// inside with its backdrop cleared; `box_at` the box's top left in it and
/// `sheet_scale` its pixels per game pixel.
pub(crate) fn place(
    src: &Rgba,
    box_at: [f64; 2],
    sheet_scale: f64,
    size: [u32; 2],
    scale: u32,
    body: Option<Body>,
) -> Option<Rgba> {
    let fb = opaque_box(src)?;
    // The figure in game pixels, as the cell's box puts it.
    let g = [
        (f64::from(fb[0]) - box_at[0]) / sheet_scale,
        (f64::from(fb[1]) - box_at[1]) / sheet_scale,
        (f64::from(fb[2]) - box_at[0]) / sheet_scale,
        (f64::from(fb[3]) - box_at[1]) / sheet_scale,
    ];
    // Game pixel of the figure to game pixel of the picture: x' = ox + k x.
    let (k, ox, oy) = match body {
        Some(b) if b[2] > b[0] && b[3] > b[1] => {
            let (gw, gh) = ((g[2] - g[0]).max(1e-6), (g[3] - g[1]).max(1e-6));
            let k = ((b[2] - b[0]) as f64 / gw).min((b[3] - b[1]) as f64 / gh);
            let cx = (b[0] + b[2]) as f64 / 2.0;
            (k, cx - k * (g[0] + g[2]) / 2.0, b[3] as f64 - k * g[3])
        }
        _ => (1.0, 0.0, 0.0),
    };
    let s = f64::from(scale);
    let (w, h) = (size[0] * scale, size[1] * scale);
    let mut out = Rgba::new(w, h);
    // An output pixel's footprint in the cell's pixels.
    let step = sheet_scale / (k * s);
    for y in 0..h {
        let gy0 = (f64::from(y) / s - oy) / k;
        let sy0 = box_at[1] + gy0 * sheet_scale;
        for x in 0..w {
            let gx0 = (f64::from(x) / s - ox) / k;
            let sx0 = box_at[0] + gx0 * sheet_scale;
            if let Some(p) = footprint(src, sx0, sy0, step) {
                out.set(x, y, p);
            }
        }
    }
    drop_lone(&mut out);
    Some(out)
}

/// The mean of a square of a picture `step` pixels wide from (x0, y0),
/// each source pixel weighted by the area of it the square covers: solid
/// when half the square is figure, colored by the figure's pixels only.
pub(crate) fn footprint(src: &Rgba, x0: f64, y0: f64, step: f64) -> Option<[u8; 4]> {
    let (x1, y1) = (x0 + step, y0 + step);
    if x1 <= 0.0 || y1 <= 0.0 || x0 >= f64::from(src.width) || y0 >= f64::from(src.height) {
        return None;
    }
    let (mut sum, mut cover, mut area) = ([0f64; 3], 0f64, 0f64);
    let ys = y0.floor().max(0.0) as u32..(y1.ceil() as u32).min(src.height);
    for sy in ys {
        let hy = (y1.min(f64::from(sy + 1)) - y0.max(f64::from(sy))).max(0.0);
        let xs = x0.floor().max(0.0) as u32..(x1.ceil() as u32).min(src.width);
        for sx in xs {
            let wx = (x1.min(f64::from(sx + 1)) - x0.max(f64::from(sx))).max(0.0);
            let a = wx * hy;
            area += a;
            let p = src.get(sx, sy);
            if p[3] >= 128 {
                cover += a;
                for c in 0..3 {
                    sum[c] += a * f64::from(p[c]);
                }
            }
        }
    }
    let full = step * step;
    if area <= 0.0 || cover * 2.0 < full {
        return None;
    }
    Some([
        (sum[0] / cover).round() as u8,
        (sum[1] / cover).round() as u8,
        (sum[2] / cover).round() as u8,
        255,
    ])
}

/// A whole picture (a screen) at `size` times `scale`, by area averaging;
/// its backdrop is its own, so nothing is cleared.
fn whole(img: &Rgba, size: [u32; 2], scale: u32) -> Rgba {
    crate::theme_fit::area_scale(img, size[0] * scale, size[1] * scale)
}

/// A file name for a game picture's art: its name without `.png`, kept to
/// letters, digits, `-` and `_`.
fn art_file(name: &str) -> String {
    let stem = name.strip_suffix(".png").unwrap_or(name);
    let s: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("art/{s}.png")
}

/// `$GP_HOME/...` for a path under the home, so the record names no one's
/// folders; the path as it is otherwise.
fn home_form(p: &Path, home: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) => format!("$GP_HOME/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// A `sheets` line's path, with `$GP_HOME` put back.
pub(crate) fn sheets_path(s: &str, home: &Path) -> PathBuf {
    match s.strip_prefix("$GP_HOME/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(s),
    }
}

/// A cell's game pictures with each one's frame index in its slot: the
/// cell's own list (`gp theme sheet` writes it), else the board's names,
/// else the game's pictures from its build.
#[allow(clippy::too_many_arguments)]
fn cell_pictures(
    m: &gp_model::Model,
    slot: &ArtSlot,
    c: &SheetCell,
    built: &mut Option<Result<crate::theme_look::GamePictures, String>>,
    game_dir: &Path,
    kit: &Kit,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Vec<(String, usize)>, String> {
    let mut out = Vec::new();
    for (i, &f) in c.frames.iter().enumerate() {
        let f = f as usize;
        if let Some(list) = c.pictures.get(i).filter(|l| !l.is_empty()) {
            out.extend(list.iter().map(|p| (p.clone(), f)));
            continue;
        }
        if crate::theme_board_kit::is_board(slot) {
            if let Some(p) = slot
                .frame_names
                .get(f)
                .and_then(|n| crate::theme_board_kit::picture(m, slot, n))
            {
                out.push((p, f));
            }
            continue;
        }
        let g = built
            .get_or_insert_with(|| crate::theme_look::game_pictures(game_dir, kit, rom, paths));
        let g = g.as_ref().map_err(Clone::clone)?;
        if let Some(list) = g.frames.get(&slot.id).and_then(|v| v.get(f)) {
            out.extend(list.iter().map(|p| (p.clone(), f)));
        }
    }
    Ok(out)
}

/// The original's body of a game picture in its frame, game pixels.
fn body_of(local: Option<&Local>, slot: &ArtSlot, name: &str, frame: usize) -> Option<Body> {
    let l = local?;
    if let Some(b) = l.board_bodies.get(name) {
        return Some(*b);
    }
    l.bodies.get(&slot.id)?.get(frame).copied().flatten()
}

/// `gp theme fit GAME --sheets DIR -o THEME --large`.
#[allow(clippy::too_many_lines)]
pub(crate) fn fit_large(
    game: &Path,
    sheets: &Path,
    out: &Path,
    scale_arg: Option<u32>,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let tdir = crate::theme_kit::template_dir(game)?;
    let (m, _) = crate::theme::read_model(&tdir)?;
    let (id, _, _) = crate::theme_kit::manifest(&tdir);
    let ldir = crate::theme_kit::local_dir(paths, &id);
    let local: Option<Local> = fs::read_to_string(ldir.join("local.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let board = gp_look::board::names_board(&m);
    if board && local.as_ref().is_none_or(|l| l.board_bodies.is_empty()) {
        bail!(
            "no local kit with the original's bodies in {}: run `gp theme look sheet --game {}`, then `gp theme kit {}`, so each picture is placed where the original's stands",
            ldir.display(),
            tdir.display(),
            tdir.display()
        );
    }
    let (kit, problems) = crate::theme_kit::kit_with_local(&m, &id, &tdir, local.as_ref())?;
    if !problems.is_empty() {
        bail!(
            "the words file doesn't fit the kit:\n  {}",
            problems.join("\n  ")
        );
    }
    let manifest = Manifest::load(sheets)?;
    let theme = out.join("theme");
    let look = theme.join(gp_present::looks::LOOK_FILE);
    if !look.is_file() {
        bail!(
            "{} has no theme/look.txt: start the theme with `gp theme look new {} --game {} --name NS/NAME --title \"...\"`, then fit into it",
            out.display(),
            out.display(),
            tdir.display()
        );
    }
    let art_path = theme.join("art.txt");
    let old = fs::read_to_string(&art_path).unwrap_or_else(|_| "gp-art 1\n".into());
    let rules = gp_present::art::ArtRules::read(&old)
        .map_err(|e| anyhow::anyhow!("{}: {e}", art_path.display()))?;
    let has_scale = old.lines().any(|l| l.trim_start().starts_with("scale "));
    let scale = scale_arg
        .or_else(|| has_scale.then_some(rules.scale))
        .unwrap_or(if kit.screen[0] >= 512 { 2 } else { 6 })
        .max(1);

    let mut drawn: BTreeMap<String, Drawn> = BTreeMap::new();
    let mut record: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut built = None;
    for s in &manifest.sheets {
        let Some(img) = crate::theme_sheet::filled(sheets, s)? else {
            missing.extend(s.cells.iter().map(|c| format!("{} {}", s.id, c.cell)));
            continue;
        };
        for c in &s.cells {
            let Some(slot) = kit.art.iter().find(|a| a.id == c.slot) else {
                missing.push(format!("{} {} (no slot {})", s.id, c.cell, c.slot));
                continue;
            };
            let names = cell_pictures(&m, slot, c, &mut built, &tdir, &kit, rom, paths)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "{e}: without the game's pictures, fit can't tell which of them cell {} stands for. Put your ROM in $GP_ROM_DIR or give it with --rom",
                        c.cell
                    )
                })?;
            if names.is_empty() {
                continue;
            }
            let src = clear_background(&crate::theme_sheet::crop(&img, c.rect));
            let box_at = [
                f64::from(c.body[0]) - f64::from(c.rect[0]),
                f64::from(c.body[1]) - f64::from(c.rect[1]),
            ];
            let size = c.size;
            let mut line = format!("cell {} {}", s.id, c.cell);
            for (name, frame) in &names {
                let body = body_of(local.as_ref(), slot, name, *frame);
                let Some(pic) = place(&src, box_at, f64::from(s.scale), size, scale, body) else {
                    missing.push(format!("{} {} (nothing drawn in it)", s.id, c.cell));
                    break;
                };
                drawn.insert(name.clone(), Drawn { img: pic, size });
                let _ = write!(line, " {name}");
            }
            record.push(line);
        }
    }
    for p in &manifest.pictures {
        let Some(slot) = kit.art.iter().find(|a| a.id == p.slot) else {
            continue;
        };
        if slot.kind != crate::theme_board_kit::SCREEN {
            // A stage is built from pieces by `stage` lines, never one
            // picture; a score display or title by its own lines.
            missing.push(format!(
                "{} (a stage or a screen of tiles: build it with `stage` lines)",
                p.id
            ));
            continue;
        }
        let f = sheets.join(&p.out);
        if !f.is_file() {
            missing.push(p.id.clone());
            continue;
        }
        let names: Vec<String> = if p.pictures.is_empty() {
            crate::theme_board_kit::picture(&m, slot, "base")
                .into_iter()
                .collect()
        } else {
            p.pictures.clone()
        };
        let img = Rgba::load(&f).map_err(anyhow::Error::msg)?;
        let mut line = format!("picture {}", p.id);
        for n in names {
            drawn.insert(
                n.clone(),
                Drawn {
                    img: whole(&img, p.size, scale),
                    size: p.size,
                },
            );
            let _ = write!(line, " {n}");
        }
        record.push(line);
    }
    if drawn.is_empty() {
        bail!(
            "nothing to fit: no filled grid or picture in {} (each sheet's `out`)",
            sheets.display()
        );
    }

    // The pictures, then the rules: each game picture drawn now gets its
    // `pic` line, in place of any it had; every other line stays.
    fs::create_dir_all(theme.join("art"))?;
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    for (name, d) in &drawn {
        let f = art_file(name);
        fs::write(theme.join(&f), d.img.png()).with_context(|| format!("writing {f}"))?;
        files.insert(name.clone(), f);
    }
    let mut text = String::new();
    let mut wrote_scale = false;
    for l in old.lines() {
        let t = l.trim_start();
        if let Some(rest) = t.strip_prefix("pic ")
            && rest
                .split_whitespace()
                .next()
                .is_some_and(|b| drawn.contains_key(b))
        {
            continue;
        }
        if t.starts_with("scale ") {
            let _ = writeln!(text, "scale {scale}");
            wrote_scale = true;
            continue;
        }
        let _ = writeln!(text, "{l}");
        if t == "gp-art 1" && !has_scale {
            let _ = writeln!(text, "scale {scale}");
            wrote_scale = true;
        }
    }
    if !wrote_scale {
        bail!("{} doesn't start with `gp-art 1`", art_path.display());
    }
    let _ = writeln!(
        text,
        "\n# Fitted by `gp theme fit --large` from {}: one line per game picture, placed by the original's body.",
        manifest
            .sheets
            .first()
            .map_or("the sheets", |_| "the sheets' cells")
    );
    for (name, d) in &drawn {
        let _ = writeln!(
            text,
            "pic {name} {} {} {} 0 0",
            files[name], d.size[0], d.size[1]
        );
    }
    gp_present::art::ArtRules::read(&text)
        .map_err(|e| anyhow::anyhow!("the rules fit wrote don't read: {e}"))?;
    fs::write(&art_path, &text)?;

    // The record: each cell or picture and the game pictures it gave,
    // merged over the one there (a placed line or a cell fitted before
    // keeps its names that weren't drawn now).
    let rec_path = theme.join(MADE_FROM);
    let mut kept: Vec<String> = Vec::new();
    if let Ok(t) = fs::read_to_string(&rec_path) {
        for l in t.lines().skip(1) {
            let w: Vec<&str> = l.split_whitespace().collect();
            let at = match w.first() {
                Some(&"cell") => 3,
                Some(&"picture") => 2,
                Some(&"placed" | &"hand") => 1,
                _ => continue,
            };
            let rest: Vec<&str> = w
                .iter()
                .skip(at)
                .copied()
                .filter(|n| !drawn.contains_key(*n))
                .collect();
            if !rest.is_empty() {
                kept.push(format!("{} {}", w[..at].join(" "), rest.join(" ")));
            }
        }
    }
    let sheets_abs = fs::canonicalize(sheets.join(crate::theme_sheet::SHEETS_FILE))
        .unwrap_or_else(|_| sheets.join(crate::theme_sheet::SHEETS_FILE));
    let mut rec = format!(
        "{MADE_FROM_FORMAT}\n# Where each picture of this theme came from (`gp theme look check` reads it): a cell of\n# the sheets `gp theme sheet` wrote, and the game pictures it was fitted to by\n# `gp theme fit --large`; `placed NAME...` for a picture made for those another way.\nsheets {}\n",
        home_form(&sheets_abs, &paths.home)
    );
    let mut seen_lines = BTreeSet::new();
    for l in kept.iter().chain(&record) {
        if seen_lines.insert(l.clone()) {
            let _ = writeln!(rec, "{l}");
        }
    }
    fs::write(&rec_path, rec)?;

    let n = drawn.len();
    let mut msg = format!(
        "fitted {n} game pictures into {} at scale {scale}: theme/art/, a `pic` line each in theme/art.txt, and theme/{MADE_FROM} naming the cell each came from",
        out.display()
    );
    if !missing.is_empty() {
        let shown: Vec<&str> = missing.iter().take(12).map(String::as_str).collect();
        let _ = write!(
            msg,
            "\n{} cells or pictures not fitted: {}{}",
            missing.len(),
            shown.join(", "),
            if missing.len() > shown.len() {
                ", ..."
            } else {
                ""
            }
        );
    }
    let _ = write!(
        msg,
        "\nNext: `gp theme look check {} --game {}`",
        out.display(),
        tdir.display()
    );
    Ok(Outcome::ok(
        json!({ "theme": out, "scale": scale, "fitted": n, "missing": missing }),
        msg,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn figure(w: u32, h: u32, bg: [u8; 4], fig: [u8; 4], at: (u32, u32), size: (u32, u32)) -> Rgba {
        let mut img = Rgba::new(w, h);
        img.pixels.fill(bg);
        for y in at.1..at.1 + size.1 {
            for x in at.0..at.0 + size.0 {
                img.set(x, y, fig);
            }
        }
        img
    }

    #[test]
    fn a_figure_fills_the_originals_body_at_the_themes_scale() {
        // A made-up 32 by 32 picture whose original body is 10 by 20 at
        // (11, 8): a cell drawn at 8 pixels a game pixel, the figure 120
        // by 200 cell pixels off its box, fitted at scale 2.
        let cell = clear_background(&figure(
            300,
            300,
            [255, 0, 255, 255],
            [30, 140, 60, 255],
            (40, 60),
            (120, 200),
        ));
        let out = place(&cell, [10.0, 10.0], 8.0, [32, 32], 2, Some([11, 8, 21, 28])).unwrap();
        assert_eq!((out.width, out.height), (64, 64));
        let b = opaque_box(&out).unwrap();
        // The figure (15 by 25 game pixels as drawn) at one scale both
        // ways: its width fills the body's 10 (20 at scale 2), it stands on
        // the body's bottom (row 28, 56) and is 16.7 tall (33 at scale 2).
        assert_eq!(b[3], 56, "{b:?}");
        assert!(b[0].abs_diff(22) <= 1 && b[2].abs_diff(42) <= 1, "{b:?}");
        assert!(b[1].abs_diff(23) <= 1, "{b:?}");
        // No key color and no backdrop mixed into an edge.
        for p in out.pixels.iter().filter(|p| p[3] > 0) {
            assert_eq!(*p, [30, 140, 60, 255]);
        }
    }

    #[test]
    fn a_pixel_is_solid_when_half_its_area_is_figure() {
        // A figure 3 cell pixels wide at 4 cell pixels a picture pixel:
        // the picture pixel it covers three quarters of is solid, and one
        // a quarter covered is clear.
        let mut src = Rgba::new(8, 4);
        for y in 0..4 {
            for x in 1..4 {
                src.set(x, y, [200, 0, 0, 255]);
            }
            src.set(4, y, [200, 0, 0, 255]);
        }
        assert_eq!(footprint(&src, 0.0, 0.0, 4.0), Some([200, 0, 0, 255]));
        assert_eq!(footprint(&src, 4.0, 0.0, 4.0), None);
    }

    #[test]
    fn the_record_names_the_home_not_its_folder() {
        let home = Path::new("/home/someone/.gameprinter");
        let p = home.join("work/themes/x/sheets/sheets.json");
        let s = home_form(&p, home);
        assert_eq!(s, "$GP_HOME/work/themes/x/sheets/sheets.json");
        assert_eq!(sheets_path(&s, home), p);
        assert_eq!(art_file("frame-1-0.png"), "art/frame-1-0.png");
        assert_eq!(art_file("sprite-cab-4d"), "art/sprite-cab-4d.png");
    }
}
