//! Semantic edits on an indexed sprite: the deterministic operations
//! `assets_edit` runs (docs/research/agent-sprite-editor.md). Each takes an
//! image of color indices and returns a new one of the same size, so an
//! edit can never change a slot's shape or tile budget. The same image and
//! operation give the same pixels on every machine: there are no floats
//! and no randomness.
//!
//! The image is any indexed picture with `colors` colors, index 0
//! transparent: a slot's sprite (4 colors on the NES) or a theme's `.pix`
//! asset. Undo doesn't depend on an operation being invertible: the tool
//! keeps the picture from before each edit.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Color indices, row by row; 0 is transparent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pic {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Pic {
    pub fn new(w: u32, h: u32, px: Vec<u8>) -> Result<Self, String> {
        if px.len() != (w * h) as usize {
            return Err(format!("{} pixels for a {w}x{h} picture", px.len()));
        }
        Ok(Self { w, h, px })
    }

    fn at(&self, x: i64, y: i64) -> Option<u8> {
        (x >= 0 && y >= 0 && x < i64::from(self.w) && y < i64::from(self.h))
            .then(|| self.px[(y as u32 * self.w + x as u32) as usize])
    }

    fn set(&mut self, x: u32, y: u32, v: u8) {
        self.px[(y * self.w + x) as usize] = v;
    }

    /// Pixels drawn (not index 0).
    pub fn ink(&self) -> usize {
        self.px.iter().filter(|&&p| p != 0).count()
    }

    /// The rows as digit strings, as `.pix` rows are written: compact,
    /// readable and diffable, for the edit history.
    pub fn rows(&self) -> Vec<String> {
        self.px
            .chunks(self.w as usize)
            .map(|r| r.iter().map(|&p| char::from(b'0' + p.min(9))).collect())
            .collect()
    }

    pub fn from_rows(rows: &[String]) -> Result<Self, String> {
        let h = rows.len() as u32;
        let w = rows.first().map_or(0, |r| r.len() as u32);
        let mut px = Vec::with_capacity((w * h) as usize);
        for r in rows {
            if r.len() as u32 != w {
                return Err("rows of different lengths".into());
            }
            for c in r.bytes() {
                if !c.is_ascii_digit() {
                    return Err(format!("`{}` isn't a color index", char::from(c)));
                }
                px.push(c - b'0');
            }
        }
        Pic::new(w, h, px)
    }

    /// The pixels that differ from `other`, as (x, y).
    pub fn diff(&self, other: &Pic) -> Vec<(u32, u32)> {
        (0..self.px.len())
            .filter(|&i| self.px[i] != other.px[i])
            .map(|i| (i as u32 % self.w, i as u32 / self.w))
            .collect()
    }

    /// The box of the drawn pixels: (x0, y0, x1, y1), inclusive.
    pub fn ink_box(&self) -> Option<(u32, u32, u32, u32)> {
        let mut b: Option<(u32, u32, u32, u32)> = None;
        for (i, &p) in self.px.iter().enumerate() {
            if p == 0 {
                continue;
            }
            let (x, y) = (i as u32 % self.w, i as u32 / self.w);
            b = Some(match b {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
        b
    }

    pub fn flipped(&self, axis: Axis) -> Pic {
        let mut out = self.clone();
        for y in 0..self.h {
            for x in 0..self.w {
                let (sx, sy) = match axis {
                    Axis::X => (self.w - 1 - x, y),
                    Axis::Y => (x, self.h - 1 - y),
                };
                out.set(x, y, self.px[(sy * self.w + sx) as usize]);
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    /// Left to right.
    X,
    /// Top to bottom.
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Half {
    /// The left half for `x`, the top half for `y`.
    First,
    /// The right half for `x`, the bottom half for `y`.
    Second,
}

/// A rectangle in the picture's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Region {
    fn holds(&self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// One operation. Indices are the sprite's palette indices (0 transparent,
/// 1 to 3 on the NES), so a recolor by index recolors by what each index
/// stands for in the art (an outline, a body color, a highlight) and keeps
/// whatever palette the sprite is drawn with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Change color indices, all pairs at once, so `[[1, 3], [3, 1]]`
    /// swaps two. Within `region` when given.
    Recolor {
        pairs: Vec<[u8; 2]>,
        #[serde(default)]
        region: Option<Region>,
    },
    /// Mirror the whole picture.
    Flip { axis: Axis },
    /// Make the picture symmetric: copy one half onto the other, mirrored.
    Mirror { axis: Axis, keep: Half },
    /// Move every pixel by (dx, dy). Pixels moved off the picture are lost
    /// unless `wrap`; the space left behind is transparent.
    Shift {
        dx: i32,
        dy: i32,
        #[serde(default)]
        wrap: bool,
    },
    /// Draw `index` on each transparent pixel next to a drawn one (the four
    /// sides, and the corners with `corners`).
    Outline {
        index: u8,
        #[serde(default)]
        corners: bool,
    },
    /// Fill a rectangle with one index (0 erases).
    Fill { region: Region, index: u8 },
    /// Set single pixels: each `[x, y, index]`.
    Pixels { set: Vec<[u32; 3]> },
    /// Copy another picture of the same size over this one: another
    /// entity's sprite, or tiles of the sheet (a neighbouring frame). The
    /// tool resolves the source before the operation runs.
    Copy {
        #[serde(default)]
        from_entity: Option<String>,
        #[serde(default)]
        from_tiles: Option<Vec<u32>>,
        #[serde(default)]
        flip: Option<Axis>,
    },
    /// Replace the picture with an image file, as an external editor saved
    /// it (the round trip `assets_export` starts): an indexed PNG of the
    /// sprite's size with indices 0 to 3, or an RGB(A) PNG whose every
    /// opaque color is one of the sprite's colors as exported. The tool
    /// reads and checks the file before the operation runs.
    Load { file: String },
    /// Draw in the style of another entity's sprite: where the reference
    /// outlines its art in one index (most of its edge pixels, at least
    /// half), this sprite's edge pixels take that index too. The tool
    /// resolves the reference's picture before the operation runs.
    MatchStyle { entity: String },
    /// An in-between of two frames of the sprite set (`from` and `to`,
    /// frame ids such as `clip0.0`): each is moved half way toward the
    /// other by its drawn pixels' centre, then pixels on which they agree
    /// are kept and the rest alternate between the two in a checkerboard.
    /// Deterministic; an image model would draw a better one through the
    /// same checks.
    Tween { from: String, to: String },
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Op::Load { .. } => "load",
            Op::Recolor { .. } => "recolor",
            Op::Flip { .. } => "flip",
            Op::Mirror { .. } => "mirror",
            Op::Shift { .. } => "shift",
            Op::Outline { .. } => "outline",
            Op::Fill { .. } => "fill",
            Op::Pixels { .. } => "pixels",
            Op::Copy { .. } => "copy",
            Op::MatchStyle { .. } => "match_style",
            Op::Tween { .. } => "tween",
        }
    }
}

fn index_ok(i: u8, colors: u8) -> Result<(), String> {
    if i >= colors {
        return Err(format!(
            "color index {i}: the sprite has {colors} colors, 0 to {}",
            colors - 1
        ));
    }
    Ok(())
}

fn region_ok(r: &Region, p: &Pic) -> Result<(), String> {
    if r.w == 0 || r.h == 0 || r.x + r.w > p.w || r.y + r.h > p.h {
        return Err(format!(
            "region {}x{} at ({}, {}) isn't inside the {}x{} sprite",
            r.w, r.h, r.x, r.y, p.w, p.h
        ));
    }
    Ok(())
}

/// Runs one operation. `sources` are the pictures the tool resolved for
/// it: one for `copy`, `load` and `match_style`, two for `tween`.
/// Fails, changing nothing, on an argument that doesn't fit the picture.
pub fn apply(p: &Pic, op: &Op, colors: u8, sources: &[&Pic]) -> Result<Pic, String> {
    let source = sources.first().copied();
    let mut out = p.clone();
    match op {
        Op::Recolor { pairs, region } => {
            if pairs.is_empty() {
                return Err("recolor needs at least one [from, to] pair".into());
            }
            let mut map: Vec<u8> = (0..colors).collect();
            for [from, to] in pairs {
                index_ok(*from, colors)?;
                index_ok(*to, colors)?;
                map[*from as usize] = *to;
            }
            if let Some(r) = region {
                region_ok(r, p)?;
            }
            for y in 0..p.h {
                for x in 0..p.w {
                    if region.is_none_or(|r| r.holds(x, y)) {
                        let v = p.px[(y * p.w + x) as usize];
                        out.set(x, y, map.get(v as usize).copied().unwrap_or(v));
                    }
                }
            }
        }
        Op::Flip { axis } => out = p.flipped(*axis),
        Op::Mirror { axis, keep } => {
            let m = p.flipped(*axis);
            for y in 0..p.h {
                for x in 0..p.w {
                    let second = match axis {
                        Axis::X => x >= p.w.div_ceil(2),
                        Axis::Y => y >= p.h.div_ceil(2),
                    };
                    // Write the half that isn't kept, from the mirror.
                    if second == (*keep == Half::First) {
                        out.set(x, y, m.px[(y * p.w + x) as usize]);
                    }
                }
            }
        }
        Op::Shift { dx, dy, wrap } => {
            let (w, h) = (i64::from(p.w), i64::from(p.h));
            for y in 0..h {
                for x in 0..w {
                    let (mut sx, mut sy) = (x - i64::from(*dx), y - i64::from(*dy));
                    if *wrap {
                        sx = sx.rem_euclid(w);
                        sy = sy.rem_euclid(h);
                    }
                    out.set(x as u32, y as u32, p.at(sx, sy).unwrap_or(0));
                }
            }
        }
        Op::Outline { index, corners } => {
            index_ok(*index, colors)?;
            if *index == 0 {
                return Err("an outline of index 0 draws nothing".into());
            }
            let sides: &[(i64, i64)] = if *corners {
                &[
                    (-1, 0),
                    (1, 0),
                    (0, -1),
                    (0, 1),
                    (-1, -1),
                    (1, -1),
                    (-1, 1),
                    (1, 1),
                ]
            } else {
                &[(-1, 0), (1, 0), (0, -1), (0, 1)]
            };
            for y in 0..p.h {
                for x in 0..p.w {
                    let (xi, yi) = (i64::from(x), i64::from(y));
                    if p.px[(y * p.w + x) as usize] == 0
                        && sides
                            .iter()
                            .any(|(ox, oy)| p.at(xi + ox, yi + oy).is_some_and(|v| v != 0))
                    {
                        out.set(x, y, *index);
                    }
                }
            }
        }
        Op::Fill { region, index } => {
            index_ok(*index, colors)?;
            region_ok(region, p)?;
            for y in region.y..region.y + region.h {
                for x in region.x..region.x + region.w {
                    out.set(x, y, *index);
                }
            }
        }
        Op::Pixels { set } => {
            if set.is_empty() {
                return Err("pixels needs at least one [x, y, index]".into());
            }
            for [x, y, i] in set {
                if *x >= p.w || *y >= p.h {
                    return Err(format!(
                        "pixel ({x}, {y}) is outside the {}x{} sprite",
                        p.w, p.h
                    ));
                }
                let i = u8::try_from(*i).map_err(|_| format!("color index {i}"))?;
                index_ok(i, colors)?;
                out.set(*x, *y, i);
            }
        }
        Op::Load { .. } => {
            let src = source.ok_or("load has no picture")?;
            if (src.w, src.h) != (p.w, p.h) {
                return Err(format!(
                    "the file is {}x{}, the sprite {}x{}",
                    src.w, src.h, p.w, p.h
                ));
            }
            out = src.clone();
        }
        Op::MatchStyle { .. } => {
            let r = source.ok_or("match_style has no reference picture")?;
            if let Some((k, share)) = r.outline_index()
                && share >= 0.5
            {
                for (x, y) in p.edge() {
                    out.set(x, y, k);
                }
            }
        }
        Op::Tween { .. } => {
            let [a, b] = sources else {
                return Err("tween needs the two frames".into());
            };
            if (a.w, a.h) != (p.w, p.h) || (b.w, b.h) != (p.w, p.h) {
                return Err(format!(
                    "tween's frames are {}x{} and {}x{}, the sprite {}x{}",
                    a.w, a.h, b.w, b.h, p.w, p.h
                ));
            }
            out = tween(a, b);
        }
        Op::Copy { flip, .. } => {
            let src = source.ok_or("copy has no source: give from_entity or from_tiles")?;
            if (src.w, src.h) != (p.w, p.h) {
                return Err(format!(
                    "the source is {}x{}, the sprite {}x{}; copy takes a picture of the same size",
                    src.w, src.h, p.w, p.h
                ));
            }
            if let Some(i) = src.px.iter().find(|&&v| v >= colors) {
                return Err(format!("the source has color index {i}"));
            }
            out = match flip {
                Some(a) => src.flipped(*a),
                None => src.clone(),
            };
        }
    }
    Ok(out)
}

/// Twice the centre of the drawn pixels, rounded down: (x, y).
fn centre2(p: &Pic) -> (i64, i64) {
    let (mut sx, mut sy, mut n) = (0i64, 0i64, 0i64);
    for (i, &v) in p.px.iter().enumerate() {
        if v != 0 {
            sx += (i as u32 % p.w) as i64;
            sy += (i as u32 / p.w) as i64;
            n += 1;
        }
    }
    if n == 0 {
        return (0, 0);
    }
    ((2 * sx).div_euclid(n), (2 * sy).div_euclid(n))
}

fn shifted(p: &Pic, dx: i64, dy: i64) -> Pic {
    let mut out = p.clone();
    for y in 0..i64::from(p.h) {
        for x in 0..i64::from(p.w) {
            out.set(x as u32, y as u32, p.at(x - dx, y - dy).unwrap_or(0));
        }
    }
    out
}

/// The in-between of two pictures of one size (Op::Tween).
pub fn tween(a: &Pic, b: &Pic) -> Pic {
    let (ca, cb) = (centre2(a), centre2(b));
    // Each moves half the way: a quarter of the doubled distance.
    let (dx, dy) = ((cb.0 - ca.0) / 4, (cb.1 - ca.1) / 4);
    let a2 = shifted(a, dx, dy);
    let b2 = shifted(b, -((cb.0 - ca.0) / 2 - dx), -((cb.1 - ca.1) / 2 - dy));
    let mut out = a2.clone();
    for i in 0..out.px.len() {
        let (x, y) = (i as u32 % out.w, i as u32 / out.w);
        if a2.px[i] != b2.px[i] && (x + y) % 2 == 1 {
            out.px[i] = b2.px[i];
        }
    }
    out
}

impl Pic {
    /// Drawn pixels with a transparent pixel, or the picture's edge, on
    /// one of their four sides: the art's outline.
    pub fn edge(&self) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for y in 0..self.h {
            for x in 0..self.w {
                let (xi, yi) = (i64::from(x), i64::from(y));
                if self.px[(y * self.w + x) as usize] != 0
                    && [(-1, 0), (1, 0), (0, -1), (0, 1)]
                        .iter()
                        .any(|(ox, oy)| self.at(xi + ox, yi + oy).is_none_or(|v| v == 0))
                {
                    out.push((x, y));
                }
            }
        }
        out
    }

    /// The index most of the edge pixels have, and their share.
    pub fn outline_index(&self) -> Option<(u8, f64)> {
        let edge = self.edge();
        let mut count = [0usize; 256];
        for &(x, y) in &edge {
            count[self.px[(y * self.w + x) as usize] as usize] += 1;
        }
        let (k, n) = count
            .iter()
            .enumerate()
            .max_by_key(|&(i, n)| (*n, usize::MAX - i))?;
        (*n > 0).then(|| (k as u8, *n as f64 / edge.len() as f64))
    }
}

/// A tile of a sprite laid out in `width` tiles a row: its 8x8 pixels.
fn tile(p: &Pic, k: u32, width: u32) -> Vec<u8> {
    let (sx, sy) = ((k % width) * 8, (k / width) * 8);
    let mut t = Vec::with_capacity(64);
    for y in 0..8 {
        for x in 0..8 {
            t.push(p.px[((sy + y) * p.w + sx + x) as usize]);
        }
    }
    t
}

fn tile_flipped(t: &[u8], axis: Axis) -> Vec<u8> {
    let mut out = vec![0; 64];
    for y in 0..8 {
        for x in 0..8 {
            let (sx, sy) = match axis {
                Axis::X => (7 - x, y),
                Axis::Y => (x, 7 - y),
            };
            out[y * 8 + x] = t[sy * 8 + sx];
        }
    }
    out
}

/// What the edited sprite's tiles could save on a console that draws a
/// sprite from 8x8 tiles with per-tile flips: tiles left blank, and tiles
/// that repeat another, flipped or not. Advice for the tile budget only;
/// the edit keeps the sprite's tile list as it is.
pub fn tile_report(p: &Pic, width: u32) -> serde_json::Value {
    let n = (p.w / 8) * (p.h / 8);
    let tiles: Vec<Vec<u8>> = (0..n).map(|k| tile(p, k, width)).collect();
    let blank: Vec<u32> = (0..n)
        .filter(|&k| tiles[k as usize].iter().all(|&v| v == 0))
        .collect();
    let mut repeats = Vec::new();
    for k in 0..n as usize {
        if blank.contains(&(k as u32)) {
            continue;
        }
        for j in 0..k {
            let how = if tiles[k] == tiles[j] {
                Some("same")
            } else if tiles[k] == tile_flipped(&tiles[j], Axis::X) {
                Some("flip_x")
            } else if tiles[k] == tile_flipped(&tiles[j], Axis::Y) {
                Some("flip_y")
            } else {
                None
            };
            if let Some(how) = how {
                repeats.push(serde_json::json!({ "tile": k, "of": j, "as": how }));
                break;
            }
        }
    }
    serde_json::json!({ "count": n, "blank": blank, "repeats": repeats })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(rows: &[&str]) -> Pic {
        Pic::from_rows(&rows.iter().map(|r| (*r).to_owned()).collect::<Vec<_>>()).unwrap()
    }

    fn rows(p: &Pic) -> Vec<String> {
        p.rows()
    }

    #[test]
    fn recolor_swaps_at_once_and_keeps_to_its_region() {
        let p = pic(&["0123", "1230"]);
        let swap = Op::Recolor {
            pairs: vec![[1, 3], [3, 1]],
            region: None,
        };
        assert_eq!(rows(&apply(&p, &swap, 4, &[]).unwrap()), ["0321", "3210"]);
        let part = Op::Recolor {
            pairs: vec![[1, 2]],
            region: Some(Region {
                x: 0,
                y: 1,
                w: 2,
                h: 1,
            }),
        };
        assert_eq!(rows(&apply(&p, &part, 4, &[]).unwrap()), ["0123", "2230"]);
        let bad = Op::Recolor {
            pairs: vec![[1, 4]],
            region: None,
        };
        assert!(apply(&p, &bad, 4, &[]).unwrap_err().contains("0 to 3"));
    }

    #[test]
    fn flip_mirror_and_shift() {
        let p = pic(&["1200", "3000"]);
        assert_eq!(
            rows(&apply(&p, &Op::Flip { axis: Axis::X }, 4, &[]).unwrap()),
            ["0021", "0003"]
        );
        assert_eq!(
            rows(&apply(&p, &Op::Flip { axis: Axis::Y }, 4, &[]).unwrap()),
            ["3000", "1200"]
        );
        let m = Op::Mirror {
            axis: Axis::X,
            keep: Half::First,
        };
        assert_eq!(rows(&apply(&p, &m, 4, &[]).unwrap()), ["1221", "3003"]);
        let lose = Op::Shift {
            dx: -1,
            dy: 1,
            wrap: false,
        };
        assert_eq!(rows(&apply(&p, &lose, 4, &[]).unwrap()), ["0000", "2000"]);
        let wrap = Op::Shift {
            dx: -1,
            dy: 1,
            wrap: true,
        };
        assert_eq!(rows(&apply(&p, &wrap, 4, &[]).unwrap()), ["0003", "2001"]);
        // Flipping twice is the identity.
        let f = Op::Flip { axis: Axis::X };
        let twice = apply(&apply(&p, &f, 4, &[]).unwrap(), &f, 4, &[]).unwrap();
        assert_eq!(twice, p);
    }

    #[test]
    fn outline_draws_only_on_transparent_neighbours() {
        let p = pic(&["0000", "0220", "0000"]);
        let o = Op::Outline {
            index: 1,
            corners: false,
        };
        assert_eq!(
            rows(&apply(&p, &o, 4, &[]).unwrap()),
            ["0110", "1221", "0110"]
        );
        let c = Op::Outline {
            index: 1,
            corners: true,
        };
        assert_eq!(
            rows(&apply(&p, &c, 4, &[]).unwrap()),
            ["1111", "1221", "1111"]
        );
        assert!(
            apply(
                &p,
                &Op::Outline {
                    index: 0,
                    corners: false
                },
                4,
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn fill_pixels_and_copy_hold_to_the_picture() {
        let p = pic(&["000", "000"]);
        let f = Op::Fill {
            region: Region {
                x: 1,
                y: 0,
                w: 2,
                h: 2,
            },
            index: 3,
        };
        assert_eq!(rows(&apply(&p, &f, 4, &[]).unwrap()), ["033", "033"]);
        let out = Op::Fill {
            region: Region {
                x: 2,
                y: 0,
                w: 2,
                h: 1,
            },
            index: 3,
        };
        assert!(apply(&p, &out, 4, &[]).is_err());
        let px = Op::Pixels {
            set: vec![[0, 1, 2]],
        };
        assert_eq!(rows(&apply(&p, &px, 4, &[]).unwrap()), ["000", "200"]);
        assert!(
            apply(
                &p,
                &Op::Pixels {
                    set: vec![[3, 0, 1]]
                },
                4,
                &[]
            )
            .is_err()
        );
        let src = pic(&["120", "000"]);
        let c = Op::Copy {
            from_entity: None,
            from_tiles: None,
            flip: Some(Axis::X),
        };
        assert_eq!(rows(&apply(&p, &c, 4, &[&src]).unwrap()), ["021", "000"]);
        assert!(apply(&p, &c, 4, &[&pic(&["12"])]).is_err());
        assert!(apply(&p, &c, 4, &[]).is_err());
    }

    #[test]
    fn match_style_takes_the_references_outline() {
        let reference = pic(&["1111", "1221", "1111"]);
        let p = pic(&["0000", "0330", "0330"]);
        let m = Op::MatchStyle {
            entity: "other".into(),
        };
        assert_eq!(
            rows(&apply(&p, &m, 4, &[&reference]).unwrap()),
            ["0000", "0110", "0110"]
        );
        // A reference with no outline of its own changes nothing.
        let flat = pic(&["0000", "0123", "0321"]);
        assert_eq!(apply(&p, &m, 4, &[&flat]).unwrap(), p);
    }

    #[test]
    fn a_tween_moves_half_way_and_keeps_what_agrees() {
        let a = pic(&["1000000", "1000000"]);
        let b = pic(&["0000100", "0000100"]);
        let t = Op::Tween {
            from: "clip0.0".into(),
            to: "clip0.1".into(),
        };
        let out = apply(&a, &t, 4, &[&a, &b]).unwrap();
        assert_eq!(rows(&out), ["0010000", "0010000"]);
        assert_eq!(apply(&a, &t, 4, &[&a, &a]).unwrap(), a);
        assert!(apply(&a, &t, 4, &[&a]).is_err());
    }

    #[test]
    fn ops_read_as_tagged_json() {
        let ops: Vec<Op> = serde_json::from_str(
            r#"[{"op":"flip","axis":"x"},{"op":"recolor","pairs":[[1,2]]},{"op":"shift","dx":0,"dy":-1}]"#,
        )
        .unwrap();
        assert_eq!(ops[0], Op::Flip { axis: Axis::X });
        assert!(serde_json::from_str::<Op>(r#"{"op":"flip","axis":"x","extra":1}"#).is_err());
        assert!(serde_json::from_str::<Op>(r#"{"op":"blur"}"#).is_err());
    }

    #[test]
    fn the_tile_report_finds_blank_and_mirrored_tiles() {
        // A 16x8 sprite: the right tile is the left one mirrored.
        let mut px = vec![0u8; 16 * 8];
        px[0] = 1;
        px[15] = 1;
        let p = Pic::new(16, 8, px).unwrap();
        let r = tile_report(&p, 2);
        assert_eq!(r["repeats"][0]["as"], "flip_x");
        let blank = Pic::new(16, 8, vec![0; 128]).unwrap();
        assert_eq!(tile_report(&blank, 2)["blank"], serde_json::json!([0, 1]));
    }
}
