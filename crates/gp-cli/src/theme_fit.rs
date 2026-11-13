//! `gp theme fit` (docs/theme-kit.md, "Fitting"): art made elsewhere, by
//! slot id in a folder or as filled grids from `gp theme sheet`, fitted to
//! a game's slots.
//!
//! Each frame goes through the same steps, from the research note
//! `asset-generation.md`, 2(e):
//!
//! | Step | What |
//! |---|---|
//! | Background | Clear where the picture has alpha; else the key color (magenta) wherever it is, or else a flood from the edges over the edge's most common color |
//! | Figure | The opaque pixels' box. One scale per slot (the largest any frame needs, never smaller than the sheet's box), so every frame keeps the same size |
//! | Place | By the anchor: a sheet cell's figure keeps its place against the cell's box and line; a loose picture's stands on the slot's bottom, centered |
//! | Downscale | Area averaging: a slot pixel is opaque when half its footprint is, colored by the mean of the opaque pixels in it |
//! | Clean | Lone opaque pixels dropped |
//! | Tones | One split by lightness for all the slot's frames (the 2nd and 98th percentiles of the slot's lightness in thirds), so a color keeps its tone from frame to frame |

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use gp_theme::color::Rgb;
use gp_theme::format::{SoundReplace, sha256_hex};
use gp_theme::pix::Rgba;
use serde_json::json;

use crate::Outcome;
use crate::theme_kit::{Banks, Kit, Local};
use crate::theme_paper::{
    few_colors, paper_tones, restyle, rgba_pix, scale_to, tone_rules, write_file,
};
use crate::theme_sheet::Manifest;

/// A picture scaled to `w` by `h` by area averaging: each target pixel
/// takes the mean of the source pixels it covers, alpha too.
pub fn area_scale(img: &Rgba, w: u32, h: u32) -> Rgba {
    let mut out = Rgba::new(w, h);
    if img.width == 0 || img.height == 0 || w == 0 || h == 0 {
        return out;
    }
    for ty in 0..h {
        let y0 = ty * img.height / h;
        let y1 = ((ty + 1) * img.height / h).max(y0 + 1);
        for tx in 0..w {
            let x0 = tx * img.width / w;
            let x1 = ((tx + 1) * img.width / w).max(x0 + 1);
            let (mut sum, mut n, mut a) = ([0u64; 3], 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = img.get(x, y);
                    a += u64::from(p[3]);
                    n += 1;
                    for k in 0..3 {
                        sum[k] += u64::from(p[k]) * u64::from(p[3]);
                    }
                }
            }
            let alpha = a / n.max(1);
            let c = if a > 0 {
                [0, 1, 2].map(|k| (sum[k] / a) as u8)
            } else {
                [0, 0, 0]
            };
            out.set(tx, ty, [c[0], c[1], c[2], alpha as u8]);
        }
    }
    out
}

/// Whether a color reads as the key color: strong red and blue, little
/// green (the sheet's magenta, its box and line, and a model's drift).
pub fn is_key(p: [u8; 4]) -> bool {
    p[0] > 150 && p[2] > 150 && p[1] < 110 && p[0].abs_diff(p[2]) < 90
}

/// The picture with its background clear: alpha when it has any, else the
/// key color when its edge is mostly key, else a flood from the edges.
pub fn clear_background(img: &Rgba) -> Rgba {
    let mut out = img.clone();
    if img.pixels.iter().any(|p| p[3] < 128) {
        for p in &mut out.pixels {
            if p[3] < 128 {
                *p = [0; 4];
            } else {
                p[3] = 255;
            }
        }
        return out;
    }
    let (w, h) = (img.width, img.height);
    let mut edge = Vec::new();
    for x in 0..w {
        edge.push((x, 0));
        edge.push((x, h.saturating_sub(1)));
    }
    for y in 0..h {
        edge.push((0, y));
        edge.push((w.saturating_sub(1), y));
    }
    let keyed = edge.iter().filter(|(x, y)| is_key(img.get(*x, *y))).count();
    if keyed * 2 > edge.len() {
        for p in &mut out.pixels {
            if is_key(*p) {
                *p = [0; 4];
            }
        }
        return out;
    }
    // The edge's most common color, and a flood over what is near it.
    let mut count: BTreeMap<[u8; 3], usize> = BTreeMap::new();
    for (x, y) in &edge {
        let p = img.get(*x, *y);
        *count
            .entry([p[0] & 0xF0, p[1] & 0xF0, p[2] & 0xF0])
            .or_default() += 1;
    }
    let Some((bg, _)) = count.into_iter().max_by_key(|(_, n)| *n) else {
        return out;
    };
    let near = |p: [u8; 4]| {
        let d: i32 = (0..3)
            .map(|k| (i32::from(p[k]) - i32::from(bg[k]) - 8).pow(2))
            .sum();
        d < 40 * 40
    };
    let mut seen = vec![false; (w * h) as usize];
    let mut stack: Vec<(u32, u32)> = edge
        .into_iter()
        .filter(|(x, y)| near(img.get(*x, *y)))
        .collect();
    while let Some((x, y)) = stack.pop() {
        let i = (y * w + x) as usize;
        if seen[i] || !near(img.get(x, y)) {
            continue;
        }
        seen[i] = true;
        out.pixels[i] = [0; 4];
        if x > 0 {
            stack.push((x - 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        if x + 1 < w {
            stack.push((x + 1, y));
        }
        if y + 1 < h {
            stack.push((x, y + 1));
        }
    }
    out
}

/// The box of a picture's opaque pixels, `[x0, y0, x1, y1)`.
pub fn opaque_box(img: &Rgba) -> Option<[u32; 4]> {
    let mut b: Option<[u32; 4]> = None;
    for y in 0..img.height {
        for x in 0..img.width {
            if img.get(x, y)[3] >= 128 {
                b = Some(match b {
                    None => [x, y, x + 1, y + 1],
                    Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)],
                });
            }
        }
    }
    b
}

/// Drops opaque pixels with no opaque 8-neighbour.
pub fn drop_lone(img: &mut Rgba) {
    let src = img.clone();
    for y in 0..img.height {
        for x in 0..img.width {
            if src.get(x, y)[3] < 128 {
                continue;
            }
            let mut any = false;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if (dx, dy) == (0, 0) {
                        continue;
                    }
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && src.get(nx as u32, ny as u32)[3] >= 128 {
                        any = true;
                    }
                }
            }
            if !any {
                img.set(x, y, [0; 4]);
            }
        }
    }
}

/// One frame's source: the picture, cleared, with where the figure is
/// meant to stand: the center column and the line (a sheet cell's box),
/// and the scale the box was drawn at.
pub struct Source {
    pub img: Rgba,
    pub center_x: f64,
    pub baseline: f64,
    pub box_scale: f64,
    pub from_sheet: bool,
    /// For a loose picture, the box its figure fills in the slot,
    /// `[x0, y0, x1, y1]`: the original's body in that frame (the local
    /// kit's, docs/theme-kit.md, "Gameplay geometry"). Without it the
    /// figure fills the slot and stands on its bottom, centered.
    pub target: Option<[f64; 4]>,
}

/// A slot's frames fitted to `w` by `h`, all at one scale and placed by
/// the anchor (see the module's table).
pub fn fit_frames(sources: &[Option<Source>], w: u32, h: u32) -> Vec<Option<Rgba>> {
    // The scale: the largest any frame needs to fit, never less than the
    // sheet's own box.
    let mut f = 0.0f64;
    for s in sources.iter().flatten() {
        let Some(b) = opaque_box(&s.img) else {
            continue;
        };
        if s.from_sheet {
            let half = (s.center_x - f64::from(b[0])).max(f64::from(b[2]) - s.center_x);
            let up = (s.baseline - f64::from(b[1])).max(1.0);
            let down = (f64::from(b[3]) - s.baseline).max(0.0);
            f = f
                .max(s.box_scale)
                .max(2.0 * half / f64::from(w))
                .max((up + down) / f64::from(h));
        } else {
            let t = s.target.unwrap_or([0.0, 0.0, f64::from(w), f64::from(h)]);
            f = f
                .max(f64::from(b[2] - b[0]) / (t[2] - t[0]).max(1.0))
                .max(f64::from(b[3] - b[1]) / (t[3] - t[1]).max(1.0));
        }
    }
    let f = f.max(1.0);
    sources
        .iter()
        .map(|s| {
            let s = s.as_ref()?;
            let b = opaque_box(&s.img)?;
            let (cx, base) = if s.from_sheet {
                (s.center_x, s.baseline)
            } else {
                (f64::from(b[0] + b[2]) / 2.0, f64::from(b[3]))
            };
            // The source region the slot covers, at scale f, then averaged.
            let sw = (f64::from(w) * f).round() as u32;
            let sh = (f64::from(h) * f).round() as u32;
            // Where the figure's middle and feet go in the slot.
            let (tx, tb) = match (s.from_sheet, s.target) {
                (false, Some(t)) => ((t[0] + t[2]) / 2.0, t[3]),
                _ => (f64::from(w) / 2.0, f64::from(h)),
            };
            let x0 = cx - tx * f;
            let mut y0 = base - tb * f;
            if s.from_sheet {
                let down = (f64::from(b[3]) - base).max(0.0);
                y0 += down;
            }
            let mut region = Rgba::new(sw.max(1), sh.max(1));
            for y in 0..region.height {
                for x in 0..region.width {
                    let (sx, sy) = (x0 + f64::from(x), y0 + f64::from(y));
                    if sx >= 0.0 && sy >= 0.0 {
                        region.set(x, y, s.img.get(sx as u32, sy as u32));
                    }
                }
            }
            let mut small = area_scale(&region, w, h);
            for p in &mut small.pixels {
                if p[3] >= 128 {
                    p[3] = 255;
                } else {
                    *p = [0; 4];
                }
            }
            drop_lone(&mut small);
            Some(small)
        })
        .collect()
}

/// The lightness split of a slot: its frames' 2nd and 98th percentiles.
pub fn slot_range(frames: &[&Rgba]) -> (f64, f64) {
    let mut l: Vec<f64> = frames
        .iter()
        .flat_map(|img| img.pixels.iter())
        .filter(|p| p[3] >= 128)
        .map(|p| Rgb([p[0], p[1], p[2]]).lab()[0])
        .collect();
    if l.is_empty() {
        return (0.0, 100.0);
    }
    l.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let at = |q: f64| l[((l.len() - 1) as f64 * q).round() as usize];
    (at(0.02), at(0.98))
}

/// A frame's tones (0 clear, 1 dark, 2 middle, 3 light) by a slot's range.
pub fn tones_in(img: &Rgba, (lo, hi): (f64, f64)) -> Vec<u8> {
    let span = (hi - lo).max(1.0);
    img.pixels
        .iter()
        .map(|p| {
            if p[3] < 128 {
                0
            } else {
                let v = Rgb([p[0], p[1], p[2]]).lab()[0];
                1 + (((v - lo) / span).clamp(0.0, 0.9999) * 3.0).floor() as u8
            }
        })
        .collect()
}

/// The frames of a slot from sheets: per frame index, its cell's picture.
fn sheet_sources(dir: &Path, m: &Manifest) -> Result<BTreeMap<(String, u32), Source>> {
    let mut out = BTreeMap::new();
    for s in &m.sheets {
        let Some(img) = crate::theme_sheet::filled(dir, s)? else {
            continue;
        };
        for c in &s.cells {
            let cell = crate::theme_sheet::crop(&img, c.rect);
            let cleared = clear_background(&cell);
            for &f in &c.frames {
                out.insert(
                    (c.slot.clone(), f),
                    Source {
                        img: cleared.clone(),
                        center_x: f64::from(c.body[0] + c.body[2] / 2 - c.rect[0]),
                        baseline: f64::from(c.baseline - c.rect[1]),
                        box_scale: f64::from(s.scale),
                        from_sheet: true,
                        target: None,
                    },
                );
            }
        }
    }
    Ok(out)
}

/// A side by side of each slot's fitted frames at 1x and 4x, for people.
fn contact(rows: &[(String, Vec<Option<Rgba>>)]) -> Rgba {
    let cell = 4 * 52;
    let w = rows.iter().map(|(_, f)| f.len()).max().unwrap_or(1).min(40) as u32 * (cell + 4) + 4;
    let row_h = rows
        .iter()
        .map(|(_, f)| f.iter().flatten().map(|i| i.height).max().unwrap_or(8))
        .collect::<Vec<_>>();
    let h: u32 = row_h.iter().map(|r| r * 4 + 8).sum::<u32>() + 4;
    let mut img = Rgba::new(w, h.max(8));
    img.pixels.fill([236, 232, 220, 255]);
    let mut y = 4;
    for ((_, frames), rh) in rows.iter().zip(&row_h) {
        for (k, f) in frames.iter().enumerate().take(40) {
            let Some(f) = f else { continue };
            let x = 4 + k as u32 * (cell + 4);
            for yy in 0..f.height {
                for xx in 0..f.width {
                    let p = f.get(xx, yy);
                    if p[3] == 0 {
                        continue;
                    }
                    img.set(x + xx, y + yy, p);
                    for dy in 0..4 {
                        for dx in 0..4 {
                            img.set(x + 52 + xx * 4 + dx, y + yy * 4 + dy, p);
                        }
                    }
                }
            }
        }
        y += rh * 4 + 8;
    }
    img
}

/// `gp theme fit`: art by slot id, or filled sheets, into a theme
/// directory.
#[allow(clippy::too_many_lines)]
pub fn fit_cmd(
    game: &Path,
    art: Option<&Path>,
    sheets: Option<&Path>,
    out: &Path,
    contact_file: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    if art.is_none() && sheets.is_none() {
        bail!("give --art DIR, --sheets DIR or both");
    }
    let tdir = crate::theme_kit::template_dir(game)?;
    let kit_file = tdir
        .join("themes")
        .join("kit")
        .join(crate::theme_kit::KIT_FILE);
    let kit: Kit = toml::from_str(
        &fs::read_to_string(&kit_file)
            .with_context(|| format!("reading {}; run gp theme kit first", kit_file.display()))?,
    )?;
    let ldir = crate::theme_kit::local_dir(paths, &kit.game);
    let local: Local = serde_json::from_str(
        &fs::read_to_string(ldir.join("local.json")).with_context(|| {
            format!(
                "no local kit in {}; run gp theme kit {} --rom ROM",
                ldir.display(),
                tdir.display()
            )
        })?,
    )?;
    let manifest = match sheets {
        Some(d) => Some(Manifest::load(d)?),
        None => None,
    };
    let from_sheets = match (sheets, &manifest) {
        (Some(d), Some(m)) => sheet_sources(d, m)?,
        _ => BTreeMap::new(),
    };
    fs::create_dir_all(out)?;
    let generator = "gp theme fit";
    let mut banks = Banks::new(&local);
    let mut restyles = Vec::new();
    let mut used = Vec::new();
    let mut sums: BTreeMap<(String, u8), [u64; 4]> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut rows = Vec::new();
    for a in &kit.art {
        if a.kind == "picture" {
            let mut f = art.map(|d| d.join(format!("{}.png", a.id)));
            if !f.as_ref().is_some_and(|f| f.is_file())
                && let (Some(d), Some(m)) = (sheets, &manifest)
                && let Some(p) = m.pictures.iter().find(|p| p.slot == a.id)
            {
                f = Some(d.join(&p.out));
            }
            let Some(f) = f.filter(|f| f.is_file()) else {
                missing.push(a.id.clone());
                continue;
            };
            let img = Rgba::load(&f).map_err(anyhow::Error::msg)?;
            let img = few_colors(&area_scale(&img, a.size[0], a.size[1]));
            let rest = a.id.trim_start_matches("picture.");
            let Some((slot, entry)) = rest.split_once('.') else {
                continue;
            };
            let file = format!("{}.pix", a.id.replace('.', "-"));
            let sha = write_file(
                out,
                &file,
                &rgba_pix(&img, &format!("{} fitted by {generator}", a.id))?,
            )?;
            restyles.push(restyle(
                Some(slot),
                Some(entry),
                &file,
                sha,
                generator,
                "uploaded",
            ));
            used.push(a.id.clone());
            continue;
        }
        let Some(cuts) = local.cuts.get(&a.id) else {
            continue;
        };
        // Each frame's source, then all of them fitted at one scale.
        let mut sources = Vec::new();
        let mut sizes = Vec::new();
        for (k, c) in cuts.iter().enumerate() {
            let w = c.places.iter().map(|p| p.col + 1).max().unwrap_or(1) * 8;
            let h = c.places.iter().map(|p| p.row + 1).max().unwrap_or(1) * 8;
            sizes.push((w, h));
            let name = a
                .frame_names
                .get(k)
                .cloned()
                .unwrap_or_else(|| k.to_string());
            let file = art.and_then(|d| {
                [
                    d.join(&a.id).join(format!("{name}.png")),
                    d.join(&a.id).join(format!("{k}.png")),
                    d.join(format!("{}.png", a.id)),
                ]
                .into_iter()
                .find(|f| f.is_file())
            });
            let src = match file {
                Some(f) => {
                    let img = Rgba::load(&f).map_err(anyhow::Error::msg)?;
                    Some(Source {
                        img: clear_background(&img),
                        center_x: 0.0,
                        baseline: 0.0,
                        box_scale: 1.0,
                        from_sheet: false,
                        target: local
                            .bodies
                            .get(&a.id)
                            .and_then(|b| b.get(k).copied().flatten())
                            .map(|b| b.map(|v| v as f64)),
                    })
                }
                None => from_sheets.get(&(a.id.clone(), k as u32)).map(|s| Source {
                    img: s.img.clone(),
                    center_x: s.center_x,
                    baseline: s.baseline,
                    box_scale: s.box_scale,
                    from_sheet: true,
                    target: None,
                }),
            };
            if src.is_none() {
                missing.push(format!("{} {name}", a.id));
            }
            sources.push(src);
        }
        // One fit a frame size (a slot's frames are almost always one).
        let (w, h) = sizes.first().copied().unwrap_or((a.size[0], a.size[1]));
        let fitted = fit_frames(&sources, w, h);
        let present: Vec<&Rgba> = fitted.iter().flatten().collect();
        let range = slot_range(&present);
        for (k, (c, img)) in cuts.iter().zip(&fitted).enumerate() {
            let Some(img) = img else { continue };
            let (fw, fh) = sizes[k];
            let img = if (img.width, img.height) == (fw, fh) {
                img.clone()
            } else {
                scale_to(img, fw, fh)
            };
            let tones = tones_in(&img, range);
            for (p, t) in img.pixels.iter().zip(&tones) {
                if *t > 0 {
                    let e = sums.entry((a.role.clone(), *t)).or_insert([0u64; 4]);
                    for q in 0..3 {
                        e[q] += u64::from(p[q]);
                    }
                    e[3] += 1;
                }
            }
            banks.put(&a.id, c, &tones, fw);
            used.push(format!(
                "{} {}",
                a.id,
                a.frame_names
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| k.to_string())
            ));
        }
        rows.push((a.id.clone(), fitted));
    }
    for (slot, text) in banks.pix(&format!("Fitted by {generator}")) {
        let file = format!("{slot}.pix");
        let sha = write_file(out, &file, &text)?;
        restyles.push(restyle(
            Some(&slot),
            None,
            &file,
            sha,
            generator,
            "uploaded",
        ));
    }
    let mean = |role: &str, t: u8| -> Option<[u8; 3]> {
        let e = sums.get(&(role.to_owned(), t))?;
        (e[3] > 0).then(|| [0, 1, 2].map(|k| (e[k] / e[3]) as u8))
    };
    let palette = tone_rules(&kit, &local, &|r| {
        let p = paper_tones(r);
        let role = if r == "*" { "effect" } else { r };
        [
            mean(role, 1).unwrap_or(p[0]),
            mean(role, 2).unwrap_or(p[1]),
            mean(role, 3).unwrap_or(p[2]),
        ]
    });
    let tfile = out.join(gp_theme::format::THEME_FILE);
    let mut theme = if tfile.is_file() {
        gp_theme::Theme::load(out).map_err(anyhow::Error::msg)?
    } else {
        let id = out
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or("mine".into());
        gp_theme::Theme {
            format: gp_theme::FORMAT_VERSION.into(),
            id,
            title: "My theme".into(),
            description: None,
            author: None,
            license: "CC0-1.0".into(),
            for_models: vec![kit.model.clone()],
            palette: Vec::new(),
            effect: Vec::new(),
            restyle: Vec::new(),
            style: None,
            audio: Vec::new(),
            sound: Vec::new(),
        }
    };
    theme.format = gp_theme::FORMAT_VERSION.into();
    theme.restyle.retain(|r| {
        !restyles
            .iter()
            .any(|n| n.slot == r.slot && n.entry == r.entry && n.target == r.target)
    });
    theme.restyle.extend(restyles);
    // Sounds: ART/sound/<role>.tune or .wav, copied and named.
    for snd in &kit.sound {
        for ext in ["tune", "wav", "ogg"] {
            let Some(f) = art.map(|d| d.join("sound").join(format!("{}.{ext}", snd.role))) else {
                continue;
            };
            if !f.is_file() {
                continue;
            }
            let bytes = fs::read(&f)?;
            let file = format!("{}.{ext}", snd.role);
            fs::write(out.join(&file), &bytes)?;
            theme.sound.retain(|x| x.role != snd.role);
            theme.sound.push(SoundReplace {
                role: snd.role.clone(),
                file,
                origin: "uploaded".into(),
                license: theme.license.clone(),
                sha256: sha256_hex(&bytes),
                generator: Some(generator.into()),
                author: None,
                note: None,
            });
            used.push(format!("sound {}", snd.role));
            break;
        }
        if !theme.sound.iter().any(|x| x.role == snd.role) {
            missing.push(format!("sound {}", snd.role));
        }
    }
    theme
        .palette
        .retain(|r| !palette.iter().any(|p| p.target == r.target));
    theme.palette.extend(palette);
    fs::write(&tfile, theme.to_text())?;
    let problems = theme.check(out);
    if !problems.is_empty() {
        bail!("the fitted theme doesn't check: {problems:?}");
    }
    if let Some(cf) = contact_file {
        fs::write(cf, contact(&rows).png())?;
    }
    Ok(Outcome::ok(
        json!({ "theme": out, "fitted": used.len(), "missing": missing, "conflicts": banks.conflicts.len() }),
        format!(
            "fitted {} frames and pictures into {}; {} slots or frames had no art (they stay blank){}",
            used.len(),
            out.display(),
            missing.len(),
            if missing.is_empty() {
                String::new()
            } else {
                format!(
                    ": {}",
                    missing
                        .iter()
                        .take(12)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        ),
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
    fn a_key_or_a_plain_ground_is_cleared() {
        let k = figure(
            40,
            40,
            [255, 0, 255, 255],
            [20, 120, 60, 255],
            (10, 10),
            (20, 20),
        );
        let c = clear_background(&k);
        assert_eq!(opaque_box(&c), Some([10, 10, 30, 30]));
        let w = figure(
            40,
            40,
            [250, 250, 245, 255],
            [20, 120, 60, 255],
            (5, 8),
            (10, 30),
        );
        assert_eq!(opaque_box(&clear_background(&w)), Some([5, 8, 15, 38]));
    }

    #[test]
    fn frames_keep_one_scale_and_stand_on_the_bottom() {
        // Two loose pictures, a figure 80 tall and one 40 tall, in a 16 by
        // 16 slot: one scale (5), both on the bottom row, the small one
        // half the height.
        let a = figure(100, 100, [0; 4], [200, 10, 10, 255], (40, 10), (20, 80));
        let b = figure(100, 100, [0; 4], [200, 10, 10, 255], (40, 50), (20, 40));
        let src = |img: Rgba| {
            Some(Source {
                img,
                center_x: 0.0,
                baseline: 0.0,
                box_scale: 1.0,
                from_sheet: false,
                target: None,
            })
        };
        let out = fit_frames(&[src(a), src(b)], 16, 16);
        let ba = opaque_box(out[0].as_ref().unwrap()).unwrap();
        let bb = opaque_box(out[1].as_ref().unwrap()).unwrap();
        assert_eq!(ba[3], 16);
        assert_eq!(bb[3], 16);
        assert_eq!(ba[3] - ba[1], 16);
        assert_eq!(bb[3] - bb[1], 8);
    }

    #[test]
    fn a_loose_picture_fills_the_originals_body() {
        // A made-up slot whose original body is 12 by 10, 2 px in from the
        // left and with its feet 3 px over the bottom: a loose figure is
        // scaled to fill that box and stands on its bottom, not the slot's.
        let a = figure(100, 100, [0; 4], [200, 10, 10, 255], (20, 20), (60, 50));
        let out = fit_frames(
            &[Some(Source {
                img: a,
                center_x: 0.0,
                baseline: 0.0,
                box_scale: 1.0,
                from_sheet: false,
                target: Some([2.0, 3.0, 14.0, 13.0]),
            })],
            16,
            16,
        );
        let b = opaque_box(out[0].as_ref().unwrap()).unwrap();
        assert_eq!(b, [2, 3, 14, 13]);
    }

    #[test]
    fn one_split_for_a_slot_keeps_a_color_in_one_tone() {
        // A mid gray that is the lightest of one frame and the darkest of
        // another gets one tone across both.
        let mid = [128, 128, 128, 255];
        let a = figure(4, 1, [0; 4], mid, (0, 0), (2, 1));
        let mut b = a.clone();
        b.set(2, 0, [255, 255, 255, 255]);
        b.set(3, 0, [0, 0, 0, 255]);
        let r = slot_range(&[&a, &b]);
        assert_eq!(tones_in(&a, r)[0], tones_in(&b, r)[0]);
    }

    #[test]
    fn area_averaging_keeps_a_thin_line() {
        // A 1 pixel line in 8x: nearest may miss it; averaging keeps its
        // color in the mix.
        let mut img = Rgba::new(16, 16);
        img.pixels.fill([255, 255, 255, 255]);
        for y in 0..16 {
            img.set(3, y, [0, 0, 0, 255]);
        }
        let s = area_scale(&img, 2, 2);
        assert!(s.get(0, 0)[0] < 255);
    }
}
