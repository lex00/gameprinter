//! `gp theme look play --shots` over a stretch of frames: `--frames` (a
//! shot every STEP frames of one run of the moments, or a list), `--strip`
//! (the shots side by side in one picture, cropped to `--box`) and
//! `--find` (where each named picture of the theme is drawn in each shot).
//!
//! A theme maker did these with scripts of their own in 2026-11 (a
//! moments file written from a run's presses, a strip of crops, and a
//! picture painted loud then searched for). `--find` does as they did: a
//! scratch copy of the theme
//! with each named picture painted a loud colour of its own is pictured
//! beside the theme, and a shot's pixels in that colour, where the theme's
//! own shot is another colour, are where the picture is drawn.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use gp_theme::pix::Rgba;

use super::{Moments, ShotDef, ThemePkg};

/// The most shots one `--frames` asks for.
pub(crate) const MOST_FRAMES: usize = 600;

/// The frames of `--frames`: a comma list of N, A-B (every frame) and
/// A-B/STEP (A, A+STEP, ... up to B), in order with none twice.
pub(crate) fn parse_frames(spec: &str) -> Result<Vec<u32>> {
    let num = |s: &str| -> Result<u32> {
        s.trim()
            .parse()
            .map_err(|_| anyhow!("--frames {spec}: {s:?} isn't a frame number"))
    };
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, num(s)?),
            None => (part, 1),
        };
        if step == 0 {
            bail!("--frames {spec}: a step of 0 frames");
        }
        let (a, b) = match range.split_once('-') {
            Some((a, b)) => (num(a)?, num(b)?),
            None if part.contains('/') => bail!("--frames {spec}: {part} has a step and no A-B"),
            None => {
                let n = num(range)?;
                (n, n)
            }
        };
        if b < a {
            bail!("--frames {spec}: {part} ends before it starts");
        }
        let mut f = a;
        while f <= b {
            out.push(f);
            if out.len() > MOST_FRAMES {
                bail!("--frames {spec} asks for more than {MOST_FRAMES} shots: take a bigger step");
            }
            f = match f.checked_add(step) {
                Some(n) => n,
                None => break,
            };
        }
    }
    if out.is_empty() {
        bail!("--frames {spec:?} names no frame");
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// The moments of `--frames`: one shot at each frame of the run `run`
/// (or the moments' only run), named RUN-fFRAME, and that run alone.
pub(crate) fn at_frames(m: &Moments, run: Option<&str>, frames: &[u32]) -> Result<Moments> {
    let names: Vec<&str> = m.run.iter().map(|r| r.name.as_str()).collect();
    let r = match run {
        Some(n) => m.run.iter().find(|r| r.name == n).ok_or_else(|| {
            anyhow!(
                "--run {n}: the moments have no run of that name (they have {})",
                names.join(", ")
            )
        })?,
        None => match m.run.as_slice() {
            [one] => one,
            [] => bail!("--frames: the moments have no run"),
            _ => bail!(
                "--frames: the moments have {} runs ({}): say which with --run NAME",
                names.len(),
                names.join(", ")
            ),
        },
    };
    if let Some(f) = frames.iter().find(|&&f| f as usize >= r.frames) {
        bail!(
            "--frames: frame {f} is past run {}'s {} frames",
            r.name,
            r.frames
        );
    }
    Ok(Moments {
        run: vec![r.clone()],
        shot: frames
            .iter()
            .map(|&f| ShotDef {
                name: format!("{}-f{f:05}", r.name),
                run: r.name.clone(),
                frame: f,
                shows: format!("frame {f} of {}", r.name),
            })
            .collect(),
    })
}

/// `--box X0,Y0,X1,Y1`: the left, top, right and bottom edges in the
/// pixels of your theme's shots, the right and bottom ones not taken.
pub(crate) fn parse_box(spec: &str) -> Result<[u32; 4]> {
    let v: Vec<u32> = spec
        .split(',')
        .map(|s| s.trim().parse::<u32>())
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow!("--box {spec}: give X0,Y0,X1,Y1 in whole pixels"))?;
    let [x0, y0, x1, y1] = v[..] else {
        bail!("--box {spec}: give four numbers, X0,Y0,X1,Y1");
    };
    if x1 <= x0 || y1 <= y0 {
        bail!("--box {spec}: X1 must be past X0 and Y1 past Y0");
    }
    Ok([x0, y0, x1, y1])
}

/// The part of `img` inside `b`, where `b` is in the pixels of a picture
/// `base` wide: scaled to `img`'s width, and kept inside it.
pub(crate) fn crop(img: &Rgba, b: [u32; 4], base: u32) -> Rgba {
    let s = |v: u32| ((u64::from(v) * u64::from(img.width)) / u64::from(base.max(1))) as u32;
    let (x0, y0) = (s(b[0]).min(img.width), s(b[1]).min(img.height));
    let (x1, y1) = (s(b[2]).clamp(x0, img.width), s(b[3]).clamp(y0, img.height));
    let mut out = Rgba::new(x1 - x0, y1 - y0);
    for y in 0..out.height {
        for x in 0..out.width {
            out.set(x, y, img.get(x0 + x, y0 + y));
        }
    }
    out
}

/// The ground between and behind the strip's cells.
pub(crate) const GROUND: [u8; 4] = [24, 24, 24, 255];
/// The gap between two cells, in pixels.
pub(crate) const GAP: u32 = 4;

/// The strip: one row for each side, one column for each shot in order,
/// each cell its shot cropped to `b` (in the first side's pixels), the
/// smaller cells made a whole number of times bigger to the biggest's
/// height as `pair` does, each set at its cell's top left. A shot missing
/// leaves its cell bare.
pub(crate) fn strip(rows: &[Vec<Option<Rgba>>], b: Option<[u32; 4]>) -> Rgba {
    let base = rows
        .first()
        .and_then(|r| r.iter().flatten().next())
        .or_else(|| rows.iter().flatten().flatten().next())
        .map_or(1, |i| i.width);
    let cut: Vec<Vec<Option<Rgba>>> = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| {
                    c.as_ref()
                        .map(|i| b.map_or_else(|| i.clone(), |b| crop(i, b, base)))
                })
                .collect()
        })
        .collect();
    let tallest = cut
        .iter()
        .flatten()
        .flatten()
        .map(|i| i.height)
        .max()
        .unwrap_or(0);
    let cells: Vec<Vec<Option<Rgba>>> = cut
        .into_iter()
        .map(|r| {
            r.into_iter()
                .map(|c| {
                    c.map(|i| {
                        let k = (tallest / i.height.max(1)).max(1);
                        if k > 1 { super::bigger(&i, k) } else { i }
                    })
                })
                .collect()
        })
        .collect();
    let cw = cells
        .iter()
        .flatten()
        .flatten()
        .map(|i| i.width)
        .max()
        .unwrap_or(0);
    let ch = cells
        .iter()
        .flatten()
        .flatten()
        .map(|i| i.height)
        .max()
        .unwrap_or(0);
    let cols = cells.iter().map(Vec::len).max().unwrap_or(0) as u32;
    let nrows = cells.len() as u32;
    let span = |n: u32, c: u32| (n * c + n.saturating_sub(1) * GAP).max(1);
    let mut out = Rgba::new(span(cols, cw), span(nrows, ch));
    for y in 0..out.height {
        for x in 0..out.width {
            out.set(x, y, GROUND);
        }
    }
    for (ri, r) in cells.iter().enumerate() {
        for (ci, c) in r.iter().enumerate() {
            let Some(img) = c else { continue };
            let (dx, dy) = (ci as u32 * (cw + GAP), ri as u32 * (ch + GAP));
            for y in 0..img.height {
                for x in 0..img.width {
                    out.set(dx + x, dy + y, img.get(x, y));
                }
            }
        }
    }
    out
}

/// Where `--strip OUT.png` writes one play's strip: OUT itself when there
/// is one play, else OUT-ROM.png beside it.
pub(crate) fn strip_file(out: &Path, rom: &str, plays: usize) -> PathBuf {
    if plays <= 1 {
        return out.to_owned();
    }
    let stem = out
        .file_stem()
        .map_or_else(|| "strip".into(), |s| s.to_string_lossy().into_owned());
    out.with_file_name(format!("{stem}-{rom}.png"))
}

// ---------------------------------------------------------------------
// --find.

/// The loud colours `--find` paints its pictures, one each, in order.
pub(crate) const LOUD: [[u8; 3]; 8] = [
    [255, 0, 255],
    [0, 255, 255],
    [255, 255, 0],
    [0, 255, 0],
    [255, 0, 0],
    [0, 0, 255],
    [255, 128, 0],
    [128, 0, 255],
];

/// How far a channel may be from the loud colour and still count as it (a
/// look drawn smooth, or blended at its edges).
const NEAR: i32 = 24;

fn near(p: [u8; 4], c: [u8; 3]) -> bool {
    p[3] > 0 && (0..3).all(|i| (i32::from(p[i]) - i32::from(c[i])).abs() <= NEAR)
}

/// A picture named on `--find`, its file in the look and its colour.
#[derive(Debug, Clone)]
pub(crate) struct Sought {
    pub name: String,
    pub file: String,
    pub colour: [u8; 3],
}

/// Where a sought picture is drawn in one shot: its pixels and their box
/// (X0,Y0,X1,Y1, the right and bottom edges not taken).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub pixels: u64,
    pub bbox: [u32; 4],
}

/// The pixels of `painted` in `colour` where `own` (the same shot of the
/// theme as it is) isn't: where the picture painted that colour is drawn.
pub(crate) fn find_in(own: &Rgba, painted: &Rgba, colour: [u8; 3]) -> Option<Found> {
    let (w, h) = (own.width.min(painted.width), own.height.min(painted.height));
    let mut f: Option<Found> = None;
    for y in 0..h {
        for x in 0..w {
            if near(painted.get(x, y), colour) && !near(own.get(x, y), colour) {
                let g = f.get_or_insert(Found {
                    pixels: 0,
                    bbox: [x, y, x + 1, y + 1],
                });
                g.pixels += 1;
                g.bbox = [
                    g.bbox[0].min(x),
                    g.bbox[1].min(y),
                    g.bbox[2].max(x + 1),
                    g.bbox[3].max(y + 1),
                ];
            }
        }
    }
    f
}

/// The look's file a `--find` name means: its path in the look folder
/// (with or without .png), or the one picture whose file name is it.
fn look_file(look: &Path, files: &[String], name: &str) -> Result<String> {
    for f in [name.to_owned(), format!("{name}.png")] {
        if files.contains(&f) && look.join(&f).is_file() {
            return Ok(f);
        }
    }
    let by_stem: Vec<&String> = files
        .iter()
        .filter(|f| {
            Path::new(f.as_str())
                .file_stem()
                .is_some_and(|s| s.to_string_lossy() == name)
        })
        .collect();
    match by_stem.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!(
            "--find {name}: the theme's look has no picture of that name (gp theme look check lists them)"
        ),
        many => bail!(
            "--find {name}: the look has {} pictures of that name ({}): give its path",
            many.len(),
            many.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Every pixel of `img` that is drawn (not clear) made `colour`.
pub(crate) fn paint(img: &mut Rgba, colour: [u8; 3]) {
    for y in 0..img.height {
        for x in 0..img.width {
            let p = img.get(x, y);
            if p[3] > 0 {
                img.set(x, y, [colour[0], colour[1], colour[2], p[3]]);
            }
        }
    }
}

/// A scratch copy of the theme in `into` with each picture named on
/// `--find` painted its loud colour: the package to picture beside it.
pub(crate) fn painted_copy(
    theme: &ThemePkg,
    names: &[String],
    into: &Path,
) -> Result<(ThemePkg, Vec<Sought>)> {
    if names.len() > LOUD.len() {
        bail!(
            "--find takes at most {} pictures at a time ({} given)",
            LOUD.len(),
            names.len()
        );
    }
    let look = crate::journeys::look_dir(&theme.root)
        .ok_or_else(|| anyhow!("--find: {} has no look folder", theme.root.display()))?;
    let manifest = look.join(gp_present::looks::LOOK_FILE);
    let l = gp_present::looks::Look::read(
        &fs::read_to_string(&manifest).with_context(|| manifest.display().to_string())?,
    )
    .map_err(|e| anyhow!("{}: {e}", manifest.display()))?;
    let rel = look
        .strip_prefix(&theme.root)
        .with_context(|| format!("{} isn't in the theme's folder", look.display()))?
        .to_owned();
    for (k, v) in super::theme_files(&theme.root)? {
        let to = into.join(&k);
        if let Some(d) = to.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(&to, v)?;
    }
    let mut sought = Vec::new();
    for (i, n) in names.iter().enumerate() {
        let file = look_file(&look, &l.files, n)?;
        if !file.ends_with(".png") {
            bail!("--find {n}: {file} isn't a PNG, the only pictures --find paints");
        }
        let at = into.join(&rel).join(&file);
        let mut img = Rgba::load(&at).map_err(|e| anyhow!("{}: {e}", at.display()))?;
        paint(&mut img, LOUD[i]);
        fs::write(&at, img.png())?;
        sought.push(Sought {
            name: n.clone(),
            file,
            colour: LOUD[i],
        });
    }
    let mut p = theme.clone();
    p.root = into.to_owned();
    Ok((p, sought))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme_play::RunDef;

    #[test]
    fn the_frame_list_reads_ranges_steps_and_single_frames() {
        assert_eq!(parse_frames("100-130/10").unwrap(), [100, 110, 120, 130]);
        assert_eq!(parse_frames("5-7").unwrap(), [5, 6, 7]);
        assert_eq!(
            parse_frames("40, 3,10-20/5,3").unwrap(),
            [3, 10, 15, 20, 40]
        );
        // A step that passes B stops before it.
        assert_eq!(parse_frames("0-25/10").unwrap(), [0, 10, 20]);
        for bad in ["", "x", "20-10", "1-9/0", "9/2", "1-2-3", "1-100000"] {
            assert!(parse_frames(bad).is_err(), "{bad:?} read");
        }
    }

    #[test]
    fn frames_make_shots_on_the_named_run() {
        let run = |n: &str| RunDef {
            name: n.into(),
            rom: "a".into(),
            frames: 300,
            holds: vec![],
        };
        let m = Moments {
            run: vec![run("serve"), run("level2")],
            shot: vec![],
        };
        let got = at_frames(&m, Some("level2"), &[10, 20]).unwrap();
        assert_eq!(got.run.len(), 1);
        assert_eq!(
            got.shot
                .iter()
                .map(|s| (s.name.as_str(), s.frame))
                .collect::<Vec<_>>(),
            [("level2-f00010", 10), ("level2-f00020", 20)]
        );
        assert!(got.check().is_ok());
        assert!(
            at_frames(&m, None, &[10]).is_err(),
            "two runs and none named"
        );
        assert!(at_frames(&m, Some("nope"), &[10]).is_err());
        assert!(
            at_frames(&m, Some("serve"), &[300]).is_err(),
            "past the run"
        );
    }

    #[test]
    fn the_box_reads_four_edges() {
        assert_eq!(parse_box("1,2,30,40").unwrap(), [1, 2, 30, 40]);
        for bad in ["1,2,3", "5,0,5,9", "0,9,9,9", "a,b,c,d"] {
            assert!(parse_box(bad).is_err(), "{bad:?} read");
        }
    }

    fn filled(w: u32, h: u32, c: [u8; 4]) -> Rgba {
        let mut i = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                i.set(x, y, c);
            }
        }
        i
    }

    #[test]
    fn the_strip_lays_each_side_in_a_row_cropped_to_the_box() {
        // Your theme's shots 10 by 10; the original's twice that, so the
        // box takes twice the pixels of it and your crops are doubled.
        let mut a0 = filled(10, 10, [10, 0, 0, 255]);
        a0.set(2, 2, [200, 0, 0, 255]);
        let a1 = filled(10, 10, [20, 0, 0, 255]);
        let b0 = filled(20, 20, [0, 10, 0, 255]);
        let b2 = filled(20, 20, [0, 30, 0, 255]);
        let rows = vec![
            vec![Some(a0), Some(a1), None],
            vec![Some(b0), None, Some(b2)],
        ];
        let s = strip(&rows, Some([2, 2, 6, 6]));
        // Cells 8 by 8: three columns and two rows with their gaps.
        assert_eq!((s.width, s.height), (3 * 8 + 2 * GAP, 2 * 8 + GAP));
        // The box's corner, doubled, at the first cell's top left.
        assert_eq!(s.get(0, 0), [200, 0, 0, 255]);
        assert_eq!(s.get(1, 1), [200, 0, 0, 255]);
        assert_eq!(s.get(2, 0), [10, 0, 0, 255]);
        // The gap, the second shot, and the missing third left bare.
        assert_eq!(s.get(8, 0), GROUND);
        assert_eq!(s.get(8 + GAP, 0), [20, 0, 0, 255]);
        assert_eq!(s.get(2 * (8 + GAP), 0), GROUND);
        // The second row: the original's, its second shot missing.
        assert_eq!(s.get(0, 8 + GAP), [0, 10, 0, 255]);
        assert_eq!(s.get(8 + GAP, 8 + GAP), GROUND);
        assert_eq!(s.get(2 * (8 + GAP) + 7, 8 + GAP + 7), [0, 30, 0, 255]);
        // No box: the whole shots, your 10s made 20s.
        let whole = strip(&rows, None);
        assert_eq!(
            (whole.width, whole.height),
            (3 * 20 + 2 * GAP, 2 * 20 + GAP)
        );
    }

    #[test]
    fn a_strip_of_several_roms_names_each() {
        let out = Path::new("/t/strip.png");
        assert_eq!(strip_file(out, "a", 1), out);
        assert_eq!(strip_file(out, "b", 2), Path::new("/t/strip-b.png"));
    }

    #[test]
    fn find_says_where_the_painted_picture_is_drawn() {
        let own = filled(16, 16, [255, 0, 255, 255]);
        let mut painted = own.clone();
        // A loud colour the theme itself draws isn't taken for the picture.
        assert_eq!(find_in(&own, &painted, LOUD[0]), None);
        let mut own2 = filled(16, 16, [40, 40, 40, 255]);
        own2.set(0, 0, [0, 255, 255, 255]);
        painted = own2.clone();
        for (x, y) in [(3, 4), (7, 5), (5, 9)] {
            painted.set(x, y, [0, 250, 250, 255]);
        }
        assert_eq!(
            find_in(&own2, &painted, LOUD[1]),
            Some(Found {
                pixels: 3,
                bbox: [3, 4, 8, 10]
            })
        );
        assert_eq!(find_in(&own2, &painted, LOUD[2]), None);
        let mut p = filled(2, 1, [9, 9, 9, 255]);
        p.set(1, 0, [9, 9, 9, 0]);
        paint(&mut p, LOUD[2]);
        assert_eq!(p.get(0, 0), [255, 255, 0, 255]);
        assert_eq!(p.get(1, 0)[3], 0);
    }
}
