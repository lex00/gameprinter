//! `gp theme look cut` (docs/theme-kit.md, "Cutting a returned picture"):
//! one picture an image model returned for a large-art theme, cut out of
//! its backdrop and shrunk to a picture's size, with its line in
//! theme/made-from.txt. Each theme maker had a script of their own for
//! this (Last Call's object16.py, build.py, objects.py and others); this
//! is the one they all were, on `gp theme fit`'s own code.
//!
//! | Step | What |
//! |---|---|
//! | Backdrop | Fit's ([`crate::theme_fit::clear_background`]): alpha, the key color (magenta) wherever it is, or a flood from the edge over the edge's commonest color. With `--key`, every pixel near that color, inside the object too |
//! | Rim | An object pixel beside the cleared backdrop that is a mix of its neighbour and the backdrop's color is cleared too, three pixels deep |
//! | Objects | The opaque pixels' 8-connected parts; a part under a fiftieth of the largest is a speck and dropped. With `--each`, each part is an object, in reading order (rows, then left to right) |
//! | Mirror, lean | `--flip` mirrors it; `--lean K` moves each row K pixels a row right of the row below it (negative: left), the bottom row kept |
//! | Shrink | One scale both ways to fit the size, standing on the bottom, centered. Fit's area averaging ([`crate::theme_fit_large::footprint`]): a pixel is solid when half its area is the object and takes the mean of the object's pixels only. Lone pixels dropped |
//! | Record | `placed NAME...` in theme/made-from.txt of the theme OUT is in |

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use gp_theme::pix::Rgba;
use serde_json::json;

use crate::Outcome;
use crate::theme_fit::{clear_background, drop_lone, edge_color, near_color, opaque_box};
use crate::theme_fit_large::{MADE_FROM, MADE_FROM_FORMAT, footprint};

#[derive(Args)]
pub struct CutArgs {
    /// The returned picture (PNG).
    pub image: PathBuf,
    /// The picture to write (PNG). With --each, the N-th object goes to
    /// STEM-N.png beside it, or to NAME.png when --for names each object.
    pub out: PathBuf,
    /// The picture's size, WxH: the object is shrunk at one scale both
    /// ways to fit it, standing on its bottom, centered.
    #[arg(long, value_name = "WxH")]
    pub size: String,
    /// The backdrop's color (RRGGBB, #RRGGBB, white or magenta): every
    /// pixel near it is cleared, inside the object too. Without it the
    /// backdrop is found as gp theme fit finds it (alpha, magenta, or a
    /// flood from the edge), and a color inside the object stays.
    #[arg(long, value_name = "COLOR")]
    pub key: Option<String>,
    /// Mirror the object left to right.
    #[arg(long)]
    pub flip: bool,
    /// Lean the object: each row K pixels right of the row below it
    /// (negative leans left), the bottom row kept. K may be a fraction.
    #[arg(
        long,
        value_name = "K",
        default_value_t = 0.0,
        allow_hyphen_values = true
    )]
    pub lean: f64,
    /// The picture is a sheet of several objects: cut each in turn, in
    /// reading order.
    #[arg(long)]
    pub each: bool,
    /// The game picture the cut stands for, for its `placed` line in
    /// made-from.txt (default: the written file's name without .png).
    /// With --each, one per object, in order. Repeatable.
    #[arg(long = "for", value_name = "NAME")]
    pub for_names: Vec<String>,
    /// The theme whose theme/made-from.txt takes the line (its folder or
    /// its theme/ folder; default: the nearest folder above OUT holding
    /// art.txt). Out of any theme, nothing is recorded.
    #[arg(long, value_name = "DIR")]
    pub theme: Option<PathBuf>,
}

pub(crate) fn cut_cmd(a: &CutArgs) -> Result<Outcome> {
    let size = parse_size(&a.size)?;
    let key = a.key.as_deref().map(parse_color).transpose()?;
    let img = Rgba::load(&a.image).map_err(anyhow::Error::msg)?;
    let parts = cut(&img, key, a.each);
    if parts.is_empty() {
        bail!(
            "{}: nothing is left once its backdrop is cleared",
            a.image.display()
        );
    }
    if !a.for_names.is_empty() && a.for_names.len() != parts.len() {
        bail!(
            "{} objects were cut and --for names {}: give one name per object",
            parts.len(),
            a.for_names.len()
        );
    }
    let dir = a.out.parent().map(Path::to_owned).unwrap_or_default();
    let stem = a
        .out
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cut".into());
    let mut written = Vec::new();
    let mut names = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        let mut part = part;
        if a.flip {
            part = flip(&part);
        }
        if a.lean != 0.0 {
            part = lean(&part, a.lean);
        }
        let small = shrink(&part, size);
        let file = match (a.each, a.for_names.get(i)) {
            (false, _) => a.out.clone(),
            (true, Some(n)) => dir.join(format!("{n}.png")),
            (true, None) => dir.join(format!("{stem}-{}.png", i + 1)),
        };
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
        }
        fs::write(&file, small.png()).with_context(|| format!("writing {}", file.display()))?;
        names.push(a.for_names.get(i).cloned().unwrap_or_else(|| {
            file.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        }));
        written.push(file);
    }
    let theme = match &a.theme {
        Some(t) if t.join("theme").is_dir() => Some(t.join("theme")),
        Some(t) => Some(t.clone()),
        None => theme_above(&a.out),
    };
    let mut msg = format!(
        "cut {} to {}x{}: {}",
        a.image.display(),
        size[0],
        size[1],
        written
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let record = match &theme {
        Some(t) => {
            add_placed(t, &names)?;
            let _ = write!(
                msg,
                "\n`placed {}` in {}",
                names.join(" "),
                t.join(MADE_FROM).display()
            );
            Some(t.join(MADE_FROM))
        }
        None => {
            let _ = write!(
                msg,
                "\nnothing recorded: {} is in no theme folder (none above it holds art.txt; give --theme)",
                a.out.display()
            );
            None
        }
    };
    Ok(Outcome::ok(
        json!({ "written": written, "names": names, "size": size, "made_from": record }),
        msg,
    ))
}

/// `WxH`.
fn parse_size(s: &str) -> Result<[u32; 2]> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .with_context(|| format!("--size {s}: give it as WxH, such as 32x48"))?;
    let (w, h): (u32, u32) = (
        w.trim().parse().with_context(|| format!("--size {s}"))?,
        h.trim().parse().with_context(|| format!("--size {s}"))?,
    );
    if w == 0 || h == 0 {
        bail!("--size {s}: both sides are at least 1");
    }
    Ok([w, h])
}

/// `RRGGBB`, `#RRGGBB`, `white` or `magenta`.
fn parse_color(s: &str) -> Result<[u8; 3]> {
    match s.to_ascii_lowercase().as_str() {
        "white" => return Ok([255, 255, 255]),
        "magenta" => return Ok([255, 0, 255]),
        _ => {}
    }
    let h = s.trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("--key {s}: give it as RRGGBB, #RRGGBB, white or magenta");
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0);
    Ok([b(0), b(2), b(4)])
}

/// The picture's objects, each cropped to its outline, with the backdrop
/// and its rim cleared: one, or with `each` every part in reading order.
fn cut(img: &Rgba, key: Option<[u8; 3]>, each: bool) -> Vec<Rgba> {
    let (mut clear, bg) = match key {
        Some(k) => {
            let mut out = img.clone();
            for p in &mut out.pixels {
                if near_color(*p, k, 48) {
                    *p = [0; 4];
                } else {
                    p[3] = 255;
                }
            }
            (out, Some(k))
        }
        None => {
            let alpha = img.pixels.iter().any(|p| p[3] < 128);
            (
                clear_background(img),
                if alpha { None } else { edge_color(img) },
            )
        }
    };
    if let Some(bg) = bg {
        strip_rim(&mut clear, bg);
    }
    let parts = parts(&clear);
    let Some(largest) = parts.iter().map(Vec::len).max() else {
        return Vec::new();
    };
    let kept: Vec<Vec<(u32, u32)>> = parts
        .into_iter()
        .filter(|p| p.len() * 50 >= largest)
        .collect();
    let groups: Vec<Vec<(u32, u32)>> = if each {
        reading_order(kept)
    } else {
        vec![kept.into_iter().flatten().collect()]
    };
    groups.iter().map(|g| crop(&clear, g)).collect()
}

/// Clears the object pixels beside the backdrop that are a mix of their
/// own neighbour's color and the backdrop's (the soft edge a model paints
/// over a flat backdrop), up to three pixels deep.
fn strip_rim(img: &mut Rgba, bg: [u8; 3]) {
    let solid =
        |img: &Rgba, x: i64, y: i64| x >= 0 && y >= 0 && img.get(x as u32, y as u32)[3] >= 128;
    for _ in 0..3 {
        let mut gone = Vec::new();
        for y in 0..img.height {
            for x in 0..img.width {
                let p = img.get(x, y);
                let (xi, yi) = (i64::from(x), i64::from(y));
                if p[3] < 128 {
                    continue;
                }
                let beside_clear = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                    .iter()
                    .any(|(dx, dy)| !solid(img, xi + dx, yi + dy));
                if !beside_clear {
                    continue;
                }
                // The neighbour least like the backdrop: the object's own
                // color this pixel would be a mix of.
                let mut q = p;
                let mut far = dist2(p, bg);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if solid(img, xi + dx, yi + dy) {
                            let n = img.get((xi + dx) as u32, (yi + dy) as u32);
                            if dist2(n, bg) > far {
                                far = dist2(n, bg);
                                q = n;
                            }
                        }
                    }
                }
                if near_color(p, bg, 64) || mixed(p, q, bg) {
                    gone.push((x, y));
                }
            }
        }
        if gone.is_empty() {
            break;
        }
        for (x, y) in gone {
            img.set(x, y, [0; 4]);
        }
    }
}

fn dist2(p: [u8; 4], c: [u8; 3]) -> i32 {
    (0..3)
        .map(|k| (i32::from(p[k]) - i32::from(c[k])).pow(2))
        .sum()
}

/// Whether `p` lies on the way from `q` to the backdrop `bg`, a fifth of
/// the way or more.
fn mixed(p: [u8; 4], q: [u8; 4], bg: [u8; 3]) -> bool {
    let v: [f64; 3] = [0, 1, 2].map(|k| f64::from(bg[k]) - f64::from(q[k]));
    let d: [f64; 3] = [0, 1, 2].map(|k| f64::from(p[k]) - f64::from(q[k]));
    let l2: f64 = v.iter().map(|c| c * c).sum();
    if l2 < 40.0 * 40.0 {
        return false;
    }
    let t = (0..3).map(|k| v[k] * d[k]).sum::<f64>() / l2;
    let off: f64 = (0..3)
        .map(|k| (d[k] - t * v[k]).powi(2))
        .sum::<f64>()
        .sqrt();
    (0.2..=1.1).contains(&t) && off < 24.0
}

/// The opaque pixels' 8-connected parts.
fn parts(img: &Rgba) -> Vec<Vec<(u32, u32)>> {
    let (w, h) = (img.width, img.height);
    let mut seen = vec![false; (w * h) as usize];
    let mut out = Vec::new();
    for y0 in 0..h {
        for x0 in 0..w {
            let i0 = (y0 * w + x0) as usize;
            if seen[i0] || img.get(x0, y0)[3] < 128 {
                continue;
            }
            seen[i0] = true;
            let mut part = Vec::new();
            let mut stack = vec![(x0, y0)];
            while let Some((x, y)) = stack.pop() {
                part.push((x, y));
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                        if nx < 0 || ny < 0 || nx >= i64::from(w) || ny >= i64::from(h) {
                            continue;
                        }
                        let (nx, ny) = (nx as u32, ny as u32);
                        let i = (ny * w + nx) as usize;
                        if !seen[i] && img.get(nx, ny)[3] >= 128 {
                            seen[i] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
            }
            out.push(part);
        }
    }
    out
}

/// A part's box, `[x0, y0, x1, y1)`.
fn box_of(part: &[(u32, u32)]) -> [u32; 4] {
    part.iter().fold([u32::MAX, u32::MAX, 0, 0], |b, &(x, y)| {
        [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)]
    })
}

/// An object's pixels, by place.
type Part = Vec<(u32, u32)>;

/// Parts in reading order: a row is the parts whose rows overlap the
/// first one's, top to bottom, each row left to right.
fn reading_order(parts: Vec<Part>) -> Vec<Part> {
    let mut left: Vec<([u32; 4], Part)> = parts.into_iter().map(|p| (box_of(&p), p)).collect();
    left.sort_by_key(|(b, _)| (b[1], b[0]));
    let mut out = Vec::new();
    while !left.is_empty() {
        let top = left[0].0;
        let (mut row, rest): (Vec<_>, Vec<_>) = left.into_iter().partition(|(b, _)| b[1] < top[3]);
        row.sort_by_key(|(b, _)| b[0]);
        out.extend(row.into_iter().map(|(_, p)| p));
        left = rest;
    }
    out
}

/// The pixels of a part, alone, cropped to their box.
fn crop(img: &Rgba, part: &[(u32, u32)]) -> Rgba {
    let b = box_of(part);
    let mut out = Rgba::new(b[2] - b[0], b[3] - b[1]);
    for &(x, y) in part {
        out.set(x - b[0], y - b[1], img.get(x, y));
    }
    out
}

/// Mirrored left to right.
fn flip(img: &Rgba) -> Rgba {
    let mut out = Rgba::new(img.width, img.height);
    for y in 0..img.height {
        for x in 0..img.width {
            out.set(img.width - 1 - x, y, img.get(x, y));
        }
    }
    out
}

/// Each row moved `k` pixels a row right of the row below it, the bottom
/// row kept, on a canvas wide enough for all of it.
fn lean(img: &Rgba, k: f64) -> Rgba {
    let h = img.height;
    if h == 0 {
        return img.clone();
    }
    let reach = k * f64::from(h - 1);
    let extra = reach.abs().ceil() as u32;
    let base = if k < 0.0 { reach.abs() } else { 0.0 };
    let mut out = Rgba::new(img.width + extra, h);
    for y in 0..h {
        let shift = (base + k * f64::from(h - 1 - y)).round() as u32;
        for x in 0..img.width {
            out.set(x + shift, y, img.get(x, y));
        }
    }
    let Some(b) = opaque_box(&out) else {
        return out;
    };
    let whole: Vec<(u32, u32)> = (b[1]..b[3])
        .flat_map(|y| (b[0]..b[2]).map(move |x| (x, y)))
        .collect();
    crop(&out, &whole)
}

/// The object shrunk at one scale both ways to fit `size`, standing on
/// the bottom, centered, by fit's area averaging; lone pixels dropped.
fn shrink(obj: &Rgba, size: [u32; 2]) -> Rgba {
    let [w, h] = size;
    let mut out = Rgba::new(w, h);
    if obj.width == 0 || obj.height == 0 {
        return out;
    }
    // Object pixels per output pixel.
    let f = (f64::from(obj.width) / f64::from(w)).max(f64::from(obj.height) / f64::from(h));
    let (ow, oh) = (f64::from(obj.width) / f, f64::from(obj.height) / f);
    let (ox, oy) = ((f64::from(w) - ow) / 2.0, f64::from(h) - oh);
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = ((f64::from(x) - ox) * f, (f64::from(y) - oy) * f);
            if let Some(p) = footprint(obj, sx, sy, f) {
                out.set(x, y, p);
            }
        }
    }
    drop_lone(&mut out);
    out
}

/// The theme/ folder above `out`: the nearest folder holding art.txt.
fn theme_above(out: &Path) -> Option<PathBuf> {
    let start = out.parent()?;
    let start = fs::canonicalize(start).unwrap_or_else(|_| start.to_owned());
    start
        .ancestors()
        .find(|d| d.join("art.txt").is_file())
        .map(Path::to_owned)
}

/// `placed NAME...` in `theme`/made-from.txt, each name taken off any
/// line that named it before (a game picture has one source).
fn add_placed(theme: &Path, names: &[String]) -> Result<()> {
    let path = theme.join(MADE_FROM);
    let new: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let mut text = format!("{MADE_FROM_FORMAT}\n");
    if let Ok(t) = fs::read_to_string(&path) {
        let mut lines = t.lines();
        if lines.next().map(str::trim) != Some(MADE_FROM_FORMAT) {
            bail!(
                "{}: its first line isn't `{MADE_FROM_FORMAT}`, so it isn't changed",
                path.display()
            );
        }
        for l in lines {
            let w: Vec<&str> = l.split_whitespace().collect();
            let at = match w.first() {
                Some(&"cell") => 3,
                Some(&"picture") => 2,
                Some(&"placed" | &"hand") => 1,
                _ => {
                    let _ = writeln!(text, "{l}");
                    continue;
                }
            };
            let rest: Vec<&str> = w
                .iter()
                .skip(at)
                .copied()
                .filter(|n| !new.contains(n))
                .collect();
            if rest.len() == w.len().saturating_sub(at) {
                let _ = writeln!(text, "{l}");
            } else if !rest.is_empty() {
                let _ = writeln!(text, "{} {}", w[..at].join(" "), rest.join(" "));
            }
        }
    }
    let _ = writeln!(text, "placed {}", names.join(" "));
    fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREEN: [u8; 4] = [30, 140, 60, 255];

    /// A 200 by 130 picture on a flat `bg`, a 100 by 60 green object at
    /// (50, 40) with a soft edge two pixels deep (two thirds and one third
    /// of the way to the backdrop), and a 10 by 10 square of backdrop in
    /// its middle.
    fn returned(bg: [u8; 3]) -> Rgba {
        let mix = |t: f64| {
            let c = [0, 1, 2].map(|k| {
                (f64::from(GREEN[k]) + t * (f64::from(bg[k]) - f64::from(GREEN[k]))).round() as u8
            });
            [c[0], c[1], c[2], 255]
        };
        let mut img = Rgba::new(200, 130);
        img.pixels.fill([bg[0], bg[1], bg[2], 255]);
        for (d, t) in [(2u32, 2.0 / 3.0), (1, 1.0 / 3.0)] {
            for y in 40 - d..100 + d {
                for x in 50 - d..150 + d {
                    img.set(x, y, mix(t));
                }
            }
        }
        for y in 40..100 {
            for x in 50..150 {
                img.set(x, y, GREEN);
            }
        }
        for y in 65..75 {
            for x in 95..105 {
                img.set(x, y, [bg[0], bg[1], bg[2], 255]);
            }
        }
        img
    }

    #[test]
    fn a_flat_backdrop_cut_comes_out_at_its_size_clear_and_rimless() {
        let d = tempfile::tempdir().unwrap();
        let theme = d.path().join("theme");
        fs::create_dir_all(theme.join("art")).unwrap();
        fs::write(theme.join("art.txt"), "gp-art 1\n").unwrap();
        // White, found by the flood from the edge; magenta, as fit's key
        // color; and a tan backdrop named with --key.
        for (bg, key, pocket_clear) in [
            ([255, 255, 255], None, false),
            ([255, 0, 255], None, true),
            ([200, 170, 120], Some("c8aa78"), true),
        ] {
            let src = d.path().join("returned.png");
            fs::write(&src, returned(bg).png()).unwrap();
            let out = theme.join("art/sprite-mug-4d.png");
            let a = CutArgs {
                image: src,
                out: out.clone(),
                size: "20x12".into(),
                key: key.map(str::to_owned),
                flip: false,
                lean: 0.0,
                each: false,
                for_names: Vec::new(),
                theme: None,
            };
            cut_cmd(&a).unwrap();
            let img = Rgba::load(&out).unwrap();
            assert_eq!((img.width, img.height), (20, 12), "{bg:?}");
            // The object at the asked size: the soft edge gone, so the
            // 100 by 60 object fills 20 by 12 at a fifth.
            assert_eq!(opaque_box(&img), Some([0, 0, 20, 12]), "{bg:?}");
            for y in 0..12 {
                for x in 0..20 {
                    let p = img.get(x, y);
                    let pocket = (9..11).contains(&x) && (5..7).contains(&y);
                    if pocket && pocket_clear {
                        assert_eq!(p[3], 0, "{bg:?} at {x},{y}");
                    } else if !pocket {
                        // Only the object's own color, none of the
                        // backdrop's mixed into a rim.
                        assert_eq!(p, GREEN, "{bg:?} at {x},{y}");
                    }
                }
            }
            let rec = fs::read_to_string(theme.join(MADE_FROM)).unwrap();
            assert_eq!(rec, "gp-made-from 1\nplaced sprite-mug-4d\n", "{bg:?}");
        }
        // Outside a theme the picture is written and nothing recorded.
        let src = d.path().join("returned.png");
        let a = CutArgs {
            image: src,
            out: d.path().join("loose/cut.png"),
            size: "10x6".into(),
            key: None,
            flip: false,
            lean: 0.0,
            each: false,
            for_names: Vec::new(),
            theme: None,
        };
        let o = cut_cmd(&a).unwrap();
        assert!(o.json["made_from"].is_null(), "{}", o.json);
        assert!(d.path().join("loose/cut.png").is_file());
    }

    #[test]
    fn a_sheet_cuts_each_object_in_reading_order_and_records_its_names() {
        let d = tempfile::tempdir().unwrap();
        let theme = d.path().join("theme");
        fs::create_dir_all(theme.join("art")).unwrap();
        fs::write(theme.join("art.txt"), "gp-art 1\n").unwrap();
        fs::write(
            theme.join(MADE_FROM),
            "gp-made-from 1\n# mine\nplaced sprite-glass sprite-old\n",
        )
        .unwrap();
        // Two objects on white, the right one higher (one row), and a
        // speck that is dropped.
        let mut img = Rgba::new(120, 60);
        img.pixels.fill([255, 255, 255, 255]);
        for y in 20..50 {
            for x in 10..40 {
                img.set(x, y, [200, 0, 0, 255]);
            }
        }
        for y in 10..40 {
            for x in 70..100 {
                img.set(x, y, [0, 0, 200, 255]);
            }
        }
        img.set(60, 55, [0, 0, 0, 255]);
        let src = d.path().join("sheet.png");
        fs::write(&src, img.png()).unwrap();
        let a = CutArgs {
            image: src,
            out: theme.join("art/x.png"),
            size: "6x6".into(),
            key: None,
            flip: false,
            lean: 0.0,
            each: true,
            for_names: vec!["sprite-mug".into(), "sprite-glass".into()],
            theme: None,
        };
        cut_cmd(&a).unwrap();
        let mug = Rgba::load(&theme.join("art/sprite-mug.png")).unwrap();
        let glass = Rgba::load(&theme.join("art/sprite-glass.png")).unwrap();
        assert_eq!(mug.get(3, 3), [200, 0, 0, 255]);
        assert_eq!(glass.get(3, 3), [0, 0, 200, 255]);
        let rec = fs::read_to_string(theme.join(MADE_FROM)).unwrap();
        assert_eq!(
            rec,
            "gp-made-from 1\n# mine\nplaced sprite-old\nplaced sprite-mug sprite-glass\n"
        );
        // A name count that isn't the object count is refused.
        let a = CutArgs {
            for_names: vec!["one".into()],
            ..a
        };
        assert!(cut_cmd(&a).is_err());
    }

    #[test]
    fn flip_mirrors_and_lean_moves_each_row_off_the_bottom() {
        // A column 1 wide, 5 tall, with a mark on its top left.
        let mut img = Rgba::new(2, 5);
        for y in 0..5 {
            img.set(0, y, GREEN);
        }
        img.set(1, 0, [9, 9, 9, 255]);
        let f = flip(&img);
        assert_eq!(f.get(1, 4), GREEN);
        assert_eq!(f.get(0, 0), [9, 9, 9, 255]);
        // Half a pixel a row: the top row (4 rows up) 2 pixels right.
        let l = lean(&img, 0.5);
        assert_eq!((l.width, l.height), (4, 5));
        assert_eq!(l.get(0, 4), GREEN);
        assert_eq!(l.get(2, 0), GREEN);
        assert_eq!(l.get(3, 0), [9, 9, 9, 255]);
        let l = lean(&img, -0.5);
        assert_eq!(l.get(2, 4), GREEN);
        assert_eq!(l.get(0, 0), GREEN);
        assert_eq!(parse_size("32x48").unwrap(), [32, 48]);
        assert!(parse_size("32").is_err());
        assert_eq!(parse_color("#FF00ff").unwrap(), [255, 0, 255]);
    }
}
