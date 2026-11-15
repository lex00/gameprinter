//! The screen measure (docs/interpreter.md, "The screen measure"): the two
//! games' pictures compared frame by frame, as two layers judged on their
//! own.
//!
//! - `screen.background` is the background layer: the level's tiles, the
//!   HUD and the backdrop. A pixel either game covers with a sprite is left
//!   out. When the whole layer is only shifted (the camera or the scroll
//!   is off), the best offset that lines the two up says by how much. It
//!   belongs to `core.camera`.
//! - `screen.sprites` is the sprite layer: every pixel either game covers
//!   with a sprite. It is compared in the frame's own space, and again
//!   with the model's picture moved by the background's shift, so a
//!   character that is right in the world but drawn under a wrong camera
//!   isn't counted wrong twice: its distance is the smaller of the two. It
//!   belongs to presentation.
//!
//! Each layer's distance on a frame is the number of pixels that differ,
//! by more than a color tolerance on any channel (exact by default). The
//! differing pixels are grouped into boxes: 8 by 8 tiles holding any are
//! joined with their neighbours, and each group's box is the smallest one
//! around its pixels. A layer drifts on a frame with a box of at least
//! 4 by 4 pixels, or with more than 0.5% of the visible area different.
//!
//! The original's picture is the emulator's whole frame; the model's is
//! the interpreter's scene drawn into the same frame, of which only the
//! model's visible part is compared. All whole numbers.

use gp_present::raster::Frame;
use serde::{Deserialize, Serialize};

/// The most boxes kept for one frame of one layer; the rest are counted.
pub const BOX_CAP: usize = 12;

/// How far the background's shift is looked for, in pixels left and right.
pub const SHIFT_RANGE: i64 = 48;
/// And up and down.
pub const SHIFT_RANGE_Y: i64 = 16;

/// The background layer's name, as a measure.
pub const BACKGROUND: &str = "screen.background";
/// The sprite layer's name, as a measure.
pub const SPRITES: &str = "screen.sprites";

/// The name of a box where the model draws no sprite.
pub const UNDRAWN: &str = "something the model doesn't draw";

/// What owns a layer: the camera for the background, the presentation
/// for the sprites. Neither is a mechanic the loop tunes numbers on.
pub fn owner(layer: &str) -> &'static str {
    if layer == BACKGROUND {
        "core.camera"
    } else {
        "presentation"
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    /// A channel differing by this much or less is the same (0: exact).
    pub color: u8,
    /// Differing pixels a frame may have before the layer splits:
    /// background, sprites.
    pub tolerance: [u64; 2],
    /// How many frames later the original shows a frame's state: its
    /// picture of frame N is compared with the model's of frame N - lag.
    /// A NES game writes its sprites and scroll in the vertical blank
    /// after the frame that moved them, so its picture is one frame
    /// behind its RAM: 1 there.
    #[serde(default)]
    pub lag: u8,
}

/// One group of differing pixels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub pixels: u64,
    /// What the model draws there, by name, or [`UNDRAWN`] for a sprite
    /// box where it draws nothing. Empty for a background box over
    /// nothing named.
    pub what: Vec<String>,
}

/// One layer on one frame.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerFrame {
    /// Pixels that differ, in the frame's own space.
    pub pixels: u64,
    /// Pixels compared.
    pub compared: u64,
    /// The background's best offset, when moving the model's picture by it
    /// at least halves the differing pixels: `[dx, dy]` with the model's
    /// pixel at `(x + dx, y + dy)` matching the original's at `(x, y)`, so
    /// a positive `dx` is the model's background drawn further right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<[i64; 2]>,
    /// The background's differing pixels with the shift, or the sprites'
    /// with the model moved by the background's shift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shifted: Option<u64>,
    /// The boxes, biggest first, of the distance's pixels.
    pub regions: Vec<Region>,
    /// Boxes past [`BOX_CAP`].
    pub more: usize,
}

impl LayerFrame {
    /// The frame's distance: the background's differing pixels; the
    /// sprites' fewer of the frame's own space and relative to the
    /// background.
    pub fn distance(&self, layer: &str) -> u64 {
        match (layer == SPRITES, self.shifted) {
            (true, Some(s)) => s.min(self.pixels),
            _ => self.pixels,
        }
    }

    /// Whether the layer has drifted apart on this frame: a box of at least
    /// 4 by 4 pixels, or more than 0.5% of `area` different.
    pub fn drifted(&self, layer: &str, area: u64) -> bool {
        self.regions.iter().any(|r| r.w >= 4 && r.h >= 4) || self.distance(layer) * 1000 > area * 5
    }
}

/// One layer over a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub owner: String,
    pub tolerance: u64,
    /// Each compared frame, in the order of `Lockstep::frames`.
    pub frames: Vec<LayerFrame>,
    /// Frames compared whose distance is within the tolerance.
    pub within: u64,
    /// Frames with nothing to compare ([`Screen::skip`]).
    #[serde(default)]
    pub skipped: u64,
    pub max: u64,
    /// Indexes into the frames.
    pub first_split: Option<usize>,
    pub first_drift: Option<usize>,
    /// The first frame the background is only shifted.
    pub first_shift: Option<usize>,
}

impl Layer {
    fn new(name: &str, tolerance: u64) -> Self {
        Self {
            name: name.to_owned(),
            owner: owner(name).to_owned(),
            tolerance,
            frames: Vec::new(),
            within: 0,
            skipped: 0,
            max: 0,
            first_split: None,
            first_drift: None,
            first_shift: None,
        }
    }

    fn push(&mut self, f: LayerFrame, area: u64) {
        let at = self.frames.len();
        let d = f.distance(&self.name);
        self.max = self.max.max(d);
        if d > self.tolerance {
            self.first_split.get_or_insert(at);
        } else {
            self.within += 1;
        }
        if f.drifted(&self.name, area) {
            self.first_drift.get_or_insert(at);
        }
        if f.shift.is_some() {
            self.first_shift.get_or_insert(at);
        }
        self.frames.push(f);
    }

    pub fn distance(&self) -> Vec<u64> {
        self.frames.iter().map(|f| f.distance(&self.name)).collect()
    }
}

/// The screen measure over a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screen {
    /// The model's visible part of the frame: left, top, width, height.
    pub visible: [u32; 4],
    pub area: u64,
    pub color: u8,
    pub background: Layer,
    pub sprites: Layer,
}

impl Screen {
    pub fn new(opts: &Options) -> Self {
        Self {
            visible: [0; 4],
            area: 0,
            color: opts.color,
            background: Layer::new(BACKGROUND, opts.tolerance[0]),
            sprites: Layer::new(SPRITES, opts.tolerance[1]),
        }
    }

    /// A frame with nothing to compare: neither the same nor different.
    pub fn skip(&mut self) {
        for l in [&mut self.background, &mut self.sprites] {
            l.frames.push(LayerFrame::default());
            l.skipped += 1;
        }
    }

    /// Measures one frame and adds it.
    pub fn push(&mut self, original: &Frame, model: &Frame) {
        let (b, s) = measure(original, model, self.color);
        self.visible = model.visible;
        self.area = u64::from(model.visible[2]) * u64::from(model.visible[3]);
        self.background.push(b, self.area);
        self.sprites.push(s, self.area);
    }

    pub fn layers(&self) -> [&Layer; 2] {
        [&self.background, &self.sprites]
    }
}

/// The visible part of the model's frame that the original's frame also
/// has: left, top, right, bottom (exclusive).
fn bounds(original: &Frame, model: &Frame) -> [i64; 4] {
    let [l, t, w, h] = model.visible.map(i64::from);
    [
        l.max(0),
        t.max(0),
        (l + w).min(i64::from(original.w)).min(i64::from(model.w)),
        (t + h).min(i64::from(original.h)).min(i64::from(model.h)),
    ]
}

/// A pixel as one number: its colour as 0xRRGGBB, and [`SPRITE_KEY`] when
/// a sprite drew it. Comparing two of these is one instruction where
/// comparing the colours and the masks apart is several, and the shift
/// search below compares about two million pairs a frame on an arcade
/// board's 512 by 480 picture.
const SPRITE_KEY: u32 = 1 << 24;
const RGB_KEY: u32 = 0x00ff_ffff;

#[inline(always)]
fn key(rgb: [u8; 3], sprite: bool) -> u32 {
    (u32::from(rgb[0]) << 16)
        | (u32::from(rgb[1]) << 8)
        | u32::from(rgb[2])
        | if sprite { SPRITE_KEY } else { 0 }
}

/// Whether two keys' colours differ by more than `tol` on any channel.
#[inline(always)]
fn keys_differ(a: u32, b: u32, tol: u8) -> bool {
    if tol == 0 {
        (a ^ b) & RGB_KEY != 0
    } else {
        [16, 8, 0]
            .into_iter()
            .any(|s| ((a >> s) as u8).abs_diff((b >> s) as u8) > tol)
    }
}

/// A grid of keys, `w` by `h`, row by row.
struct Grid {
    px: Vec<u32>,
    w: usize,
    h: usize,
}

impl Grid {
    /// A frame's keys over the area `[l, r)` by `[t, b)`.
    fn of(f: &Frame, [l, t, r, b]: [i64; 4]) -> Self {
        let (w, h) = ((r - l).max(0) as usize, (b - t).max(0) as usize);
        let fw = f.w as usize;
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            let at = (t as usize + y) * fw + l as usize;
            px.extend(
                f.rgb[at..at + w]
                    .iter()
                    .zip(&f.sprite[at..at + w])
                    .map(|(c, s)| key(*c, *s)),
            );
        }
        Self { px, w, h }
    }

    /// Every `step`th key each way from `(x0, y0)`: the keys at
    /// `(x0 + step * i, y0 + step * j)`.
    fn every(&self, step: usize, x0: usize, y0: usize) -> Self {
        let w = self.w.saturating_sub(x0).div_ceil(step);
        let h = self.h.saturating_sub(y0).div_ceil(step);
        let mut px = Vec::with_capacity(w * h);
        for j in 0..h {
            let row = &self.px[(y0 + step * j) * self.w..][..self.w];
            px.extend(row[x0..].iter().step_by(step));
        }
        Self { px, w, h }
    }
}

/// The background's differing pixels and those compared between `o` and
/// `m` with `m` moved by `(qx, qy)` cells: `o`'s cell `(i, j)` against
/// `m`'s `(i + qx, j + qy)`, where `m` has one, and neither a sprite.
fn background_counts(o: &Grid, m: &Grid, qx: i64, qy: i64, tol: u8) -> (u64, u64) {
    let (mut n, mut c) = (0u64, 0u64);
    let i0 = (-qx).max(0);
    let i1 = (o.w as i64).min(m.w as i64 - qx);
    if i1 <= i0 {
        return (0, 0);
    }
    let (i0, i1) = (i0 as usize, i1 as usize);
    let mi0 = (i0 as i64 + qx) as usize;
    for j in 0..o.h {
        let mj = j as i64 + qy;
        if mj < 0 || mj >= m.h as i64 {
            continue;
        }
        let orow = &o.px[j * o.w + i0..j * o.w + i1];
        let mrow = &m.px[mj as usize * m.w + mi0..][..i1 - i0];
        let (rn, rc) = row_counts(orow, mrow, tol);
        n += rn;
        c += rc;
    }
    (n, c)
}

/// One row's [`background_counts`]: the pairs that differ and those
/// compared, neither a sprite. Exact colours are a row of plain
/// comparisons, which the compiler does several pixels at a time.
#[inline(always)]
fn row_counts(o: &[u32], m: &[u32], tol: u8) -> (u64, u64) {
    let (mut n, mut c) = (0u32, 0u32);
    if tol == 0 {
        for (&a, &b) in o.iter().zip(m) {
            // Neither a sprite: the keys are their colours.
            let both = u32::from((a | b) & SPRITE_KEY == 0);
            c += both;
            n += both & u32::from(a != b);
        }
    } else {
        for (&a, &b) in o.iter().zip(m) {
            let both = (a | b) & SPRITE_KEY == 0;
            c += u32::from(both);
            n += u32::from(both && keys_differ(a, b, tol));
        }
    }
    (u64::from(n), u64::from(c))
}

/// The sprite layer between `o` and `m` with `m` moved by `(dx, dy)`: the
/// pixels either covers with a sprite, those that differ (also those
/// whose model pixel is off the area) set in the returned bits, and the
/// counts of both.
fn sprite_counts(o: &Grid, m: &Grid, dx: i64, dy: i64, tol: u8) -> (Vec<bool>, u64, u64) {
    let (w, h) = (o.w, o.h);
    let mut d = vec![false; w * h];
    let (mut n, mut c) = (0u64, 0u64);
    // The columns whose moved pixel is on the area.
    let x0 = (-dx).clamp(0, w as i64) as usize;
    let x1 = (w as i64 - dx).clamp(x0 as i64, w as i64) as usize;
    for y in 0..h {
        let row = &o.px[y * w..(y + 1) * w];
        let bits = &mut d[y * w..(y + 1) * w];
        let my = y as i64 + dy;
        let (lo, hi) = if my < 0 || my >= h as i64 {
            (0, 0)
        } else {
            (x0, x1)
        };
        // Off the area: only the original's sprites count, all apart.
        for x in (0..lo).chain(hi.max(lo)..w) {
            if row[x] & SPRITE_KEY != 0 {
                c += 1;
                n += 1;
                bits[x] = true;
            }
        }
        if hi > lo {
            let mrow = &m.px[my as usize * w..][..w];
            for x in lo..hi {
                let (a, b) = (row[x], mrow[(x as i64 + dx) as usize]);
                if (a | b) & SPRITE_KEY == 0 {
                    continue;
                }
                c += 1;
                if keys_differ(a, b, tol) {
                    bits[x] = true;
                    n += 1;
                }
            }
        }
    }
    (d, n, c)
}

/// Both layers on one frame.
pub fn measure(original: &Frame, model: &Frame, tol: u8) -> (LayerFrame, LayerFrame) {
    let area = bounds(original, model);
    let [l, t, _, _] = area;
    // Each side's keys over the compared area: every read below is inside it.
    let o = Grid::of(original, area);
    let m = Grid::of(model, area);
    let (w, h) = (o.w, o.h);

    // The background, in the frame's own space.
    let mut bg = vec![false; w * h];
    let mut bg_n = 0u64;
    let mut bg_compared = 0u64;
    for ((&a, &b), bit) in o.px.iter().zip(&m.px).zip(bg.iter_mut()) {
        let both = (a | b) & SPRITE_KEY == 0;
        let apart = both && keys_differ(a, b, tol);
        bg_compared += u64::from(both);
        bg_n += u64::from(apart);
        *bit = apart;
    }
    let mut shift = None;
    let mut shifted = None;
    // Looked for only when much of the layer differs (past the drift
    // share): a few tiles apart are not a shift.
    if bg_n * 1000 > bg_compared * 5 {
        // Moved by an offset: the background's differing pixels, on every
        // 4th pixel each way, as a share of those compared. The model's
        // pixels at an offset are its every-4th grid from the offset's
        // remainder, made once, moved by the offset's quotient.
        const STEP: i64 = 4;
        let o4 = o.every(STEP as usize, 0, 0);
        let mut m4: [Option<Grid>; 16] = Default::default();
        let mut moved4 = |dx: i64, dy: i64| {
            let (px, py) = (dx.rem_euclid(STEP), dy.rem_euclid(STEP));
            let g = m4[(py * STEP + px) as usize]
                .get_or_insert_with(|| m.every(STEP as usize, px as usize, py as usize));
            background_counts(&o4, g, dx.div_euclid(STEP), dy.div_euclid(STEP), tol)
        };
        let rate = |(n, c): (u64, u64)| (n * 65536).checked_div(c).unwrap_or(u64::MAX);
        let mut best = (rate(moved4(0, 0)), 0i64, 0i64);
        let better = |cand: (u64, i64, i64), best: (u64, i64, i64)| {
            cand.0 < best.0
                || (cand.0 == best.0 && cand.1.abs() + cand.2.abs() < best.1.abs() + best.2.abs())
        };
        // Across first, the way a side-scroller's camera drifts, then
        // down at the best offset across.
        for pass in 0..2 {
            let range = if pass == 0 {
                SHIFT_RANGE
            } else {
                SHIFT_RANGE_Y
            };
            for d in -range..=range {
                let (dx, dy) = if pass == 1 { (best.1, d) } else { (d, best.2) };
                let cand = (rate(moved4(dx, dy)), dx, dy);
                if better(cand, best) {
                    best = cand;
                }
            }
        }
        if (best.1, best.2) != (0, 0) {
            let (n, _) = background_counts(&o, &m, best.1, best.2, tol);
            if n * 2 <= bg_n {
                shift = Some([best.1, best.2]);
                shifted = Some(n);
            }
        }
    }
    let background = LayerFrame {
        pixels: bg_n,
        compared: bg_compared,
        shift,
        shifted,
        ..LayerFrame::default()
    };

    // The sprites, in the frame's own space and relative to the background.
    let (sp, sp_n, sp_compared) = sprite_counts(&o, &m, 0, 0, tol);
    let mut sprites = LayerFrame {
        pixels: sp_n,
        compared: sp_compared,
        ..LayerFrame::default()
    };
    let mut sprite_bits = sp;
    if let Some([dx, dy]) = shift {
        let (rel, rel_n, _) = sprite_counts(&o, &m, dx, dy, tol);
        sprites.shifted = Some(rel_n);
        if rel_n < sp_n {
            sprite_bits = rel;
        }
    }

    let mut background = background;
    let (regions, more) = regions_of(&bg, w, h, l, t);
    background.regions = name(regions, model, false);
    background.more = more;
    let (regions, more) = regions_of(&sprite_bits, w, h, l, t);
    sprites.regions = name(regions, model, true);
    sprites.more = more;
    (background, sprites)
}

/// A tile's differing pixels: their box (left, top, right, bottom) and count.
type TileBox = (usize, usize, usize, usize, u64);

/// The differing pixels in `bits` (a `w` by `h` area at `(l, t)`) as boxes:
/// tiles of 8 by 8 with any are joined with the tiles around them, and
/// each group's box is the smallest around its pixels. Biggest first, at
/// most [`BOX_CAP`], with the number left out.
pub fn regions_of(bits: &[bool], w: usize, h: usize, l: i64, t: i64) -> (Vec<Region>, usize) {
    let (tw, th) = (w.div_ceil(8), h.div_ceil(8));
    // Each tile's box and count, by tile.
    let mut tiles: Vec<Option<TileBox>> = vec![None; tw * th];
    // A tile's row at a time: its first and last pixel and how many.
    for y in 0..h {
        let row = &bits[y * w..(y + 1) * w];
        for (tx, cells) in row.chunks(8).enumerate() {
            let Some(first) = cells.iter().position(|b| *b) else {
                continue;
            };
            let last = cells.iter().rposition(|b| *b).unwrap_or(first);
            let k = cells[first..=last].iter().filter(|b| **b).count() as u64;
            let (a, z) = (tx * 8 + first, tx * 8 + last);
            let e = &mut tiles[(y / 8) * tw + tx];
            *e = Some(match *e {
                None => (a, y, z, y, k),
                Some((x0, y0, x1, y1, n)) => (x0.min(a), y0.min(y), x1.max(z), y1.max(y), n + k),
            });
        }
    }
    let mut seen = vec![false; tw * th];
    let mut out = Vec::new();
    for start in 0..tw * th {
        if seen[start] || tiles[start].is_none() {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (usize::MAX, usize::MAX, 0, 0, 0u64);
        while let Some(k) = stack.pop() {
            let (a, b, c, d, m) = tiles[k].expect("a tile in a group has pixels");
            (x0, y0, x1, y1, n) = (x0.min(a), y0.min(b), x1.max(c), y1.max(d), n + m);
            let (tx, ty) = ((k % tw) as i64, (k / tw) as i64);
            for (ox, oy) in [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let (nx, ny) = (tx + ox, ty + oy);
                if nx < 0 || ny < 0 || nx >= tw as i64 || ny >= th as i64 {
                    continue;
                }
                let j = ny as usize * tw + nx as usize;
                if !seen[j] && tiles[j].is_some() {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        out.push(Region {
            x: l + x0 as i64,
            y: t + y0 as i64,
            w: (x1 - x0 + 1) as i64,
            h: (y1 - y0 + 1) as i64,
            pixels: n,
            what: Vec::new(),
        });
    }
    out.sort_by_key(|r| (std::cmp::Reverse(r.pixels), r.y, r.x));
    let more = out.len().saturating_sub(BOX_CAP);
    out.truncate(BOX_CAP);
    (out, more)
}

fn overlaps(r: &Region, b: [i64; 4]) -> bool {
    r.x < b[0] + b[2] && b[0] < r.x + r.w && r.y < b[1] + b[3] && b[1] < r.y + r.h
}

/// Names each box by what the model draws there of its layer.
fn name(mut regions: Vec<Region>, model: &Frame, sprites: bool) -> Vec<Region> {
    for r in &mut regions {
        for th in &model.things {
            if th.sprite == sprites
                && !th.name.is_empty()
                && overlaps(r, th.rect)
                && !r.what.contains(&th.name)
            {
                r.what.push(th.name.clone());
            }
        }
        if sprites && r.what.is_empty() {
            r.what.push(UNDRAWN.to_owned());
        }
    }
    regions
}

/// One layer's score over a scenario or a clip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerScore {
    pub name: String,
    pub owner: String,
    /// Frames within the tolerance, of `total`.
    pub same: u64,
    pub total: u64,
    pub max: u64,
    /// The first frame past the tolerance, from the start, with the
    /// original's frame and that frame's boxes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_original: Option<u64>,
    #[serde(default)]
    pub boxes: Vec<Region>,
    #[serde(default)]
    pub more: usize,
    /// The first frame it drifts apart, from the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift: Option<u64>,
    /// The background's shift on its first different frame, when it is
    /// only shifted there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<[i64; 2]>,
}

/// The most boxes a [`Span`] keeps for a frame.
pub const SPAN_BOXES: usize = 6;

/// Frames in a row on which a layer's boxes stay the same, for drawing
/// them over the game as it plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// The first and last frame, from the start.
    pub from: u64,
    pub to: u64,
    /// `sprites` or `background`.
    pub layer: String,
    /// Left, top, width, height, biggest first, at most [`SPAN_BOXES`].
    pub boxes: Vec<[i64; 4]>,
}

/// The screen measure's score: each layer's, a line each in plain words,
/// and every frame's boxes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Score {
    pub background: LayerScore,
    pub sprites: LayerScore,
    pub says: Vec<String>,
    #[serde(default)]
    pub spans: Vec<Span>,
}

fn spans(l: &Layer, layer: &str, frames: &[(u64, u64)], from: u64) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    // Whether the last span reaches the frame before this one.
    let mut open = false;
    for (k, f) in l.frames.iter().enumerate() {
        let Some(at) = frames.get(k).map(|f| f.1.saturating_sub(from)) else {
            break;
        };
        let boxes: Vec<[i64; 4]> = f
            .regions
            .iter()
            .take(SPAN_BOXES)
            .map(|r| [r.x, r.y, r.w, r.h])
            .collect();
        if boxes.is_empty() {
            open = false;
            continue;
        }
        match out.last_mut() {
            Some(s) if open && s.boxes == boxes => s.to = at,
            _ => out.push(Span {
                from: at,
                to: at,
                layer: layer.to_owned(),
                boxes,
            }),
        }
        open = true;
    }
    out
}

fn layer_score(l: &Layer, frames: &[(u64, u64)], from: u64) -> LayerScore {
    let at = |k: usize| frames.get(k).map(|f| f.1);
    let first = l.first_split.and_then(|k| Some((k, at(k)?)));
    let f = l.first_split.and_then(|k| l.frames.get(k));
    LayerScore {
        name: l.name.clone(),
        owner: l.owner.clone(),
        same: l.within,
        total: l.frames.len() as u64 - l.skipped,
        max: l.max,
        first: first.map(|(_, o)| o.saturating_sub(from)),
        first_original: first.map(|(_, o)| o),
        boxes: f.map(|f| f.regions.clone()).unwrap_or_default(),
        more: f.map_or(0, |f| f.more),
        drift: l.first_drift.and_then(at).map(|o| o.saturating_sub(from)),
        shift: f.and_then(|f| f.shift),
    }
}

/// The names in `boxes`, each once, in order; a background box over
/// nothing named is "the background".
fn names(boxes: &[Region], background: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in boxes {
        let mut what = b.what.clone();
        if what.is_empty() && background {
            what.push("the background".into());
        }
        for w in what {
            if !out.contains(&w) {
                out.push(w);
            }
        }
    }
    out
}

/// How a shift reads: "3 px further left in the rebuilt game".
pub fn shift_words([dx, dy]: [i64; 2]) -> String {
    let mut parts = Vec::new();
    if dx != 0 {
        parts.push(format!(
            "{} px further {}",
            dx.abs(),
            if dx > 0 { "right" } else { "left" }
        ));
    }
    if dy != 0 {
        parts.push(format!(
            "{} px further {}",
            dy.abs(),
            if dy > 0 { "down" } else { "up" }
        ));
    }
    format!("{} in the rebuilt game", parts.join(" and "))
}

/// A layer's line: "Characters: 96% of frames the same; different from
/// frame 1: 3 places (Mario, something the model doesn't draw)".
fn says(label: &str, s: &LayerScore, background: bool) -> String {
    let pct = (s.same * 100).checked_div(s.total).unwrap_or(100);
    let Some(first) = s.first else {
        return format!("{label}: the same on every frame.");
    };
    if let Some(shift) = s.shift {
        return format!(
            "{label}: {pct}% of frames the same; shifted from frame {first} (the camera): {}.",
            shift_words(shift)
        );
    }
    let n = s.boxes.len() + s.more;
    let places = if n == 1 {
        "1 place".to_owned()
    } else {
        format!("{n} places")
    };
    format!(
        "{label}: {pct}% of frames the same; different from frame {first}: {places} ({}).",
        names(&s.boxes, background).join(", ")
    )
}

/// The screen measure's score over `frames` (the lockstep's), counted from
/// the original's frame `from`.
pub fn score(screen: &Screen, frames: &[(u64, u64)], from: u64) -> Score {
    let sprites = layer_score(&screen.sprites, frames, from);
    let background = layer_score(&screen.background, frames, from);
    let says = vec![
        says("Characters", &sprites, false),
        says("Background", &background, true),
    ];
    let mut spans_all = spans(&screen.sprites, "sprites", frames, from);
    spans_all.extend(spans(&screen.background, "background", frames, from));
    Score {
        background,
        sprites,
        says,
        spans: spans_all,
    }
}

/// The measure as it was before it compared keys by rows (kept for the
/// tests, which hold the two to the same answers on made-up frames).
#[cfg(test)]
mod reference {
    use super::{BOX_CAP, LayerFrame, Region, SHIFT_RANGE, SHIFT_RANGE_Y, TileBox, bounds, name};
    use gp_present::raster::Frame;

    fn differs(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
        a.iter().zip(b).any(|(x, y)| x.abs_diff(y) > tol)
    }

    pub(super) fn measure(original: &Frame, model: &Frame, tol: u8) -> (LayerFrame, LayerFrame) {
        let [l, t, r, b] = bounds(original, model);
        let ow = i64::from(original.w);
        let mw = i64::from(model.w);
        let oi = |x: i64, y: i64| (y * ow + x) as usize;
        let mi = |x: i64, y: i64| (y * mw + x) as usize;
        let inside = |x: i64, y: i64| x >= l && y >= t && x < r && y < b;
        let w = (r - l).max(0) as usize;
        let h = (b - t).max(0) as usize;

        // The background, in the frame's own space.
        let mut bg = vec![false; w * h];
        let mut bg_n = 0u64;
        let mut bg_compared = 0u64;
        for y in t..b {
            for x in l..r {
                let (o, m) = (oi(x, y), mi(x, y));
                if original.sprite[o] || model.sprite[m] {
                    continue;
                }
                bg_compared += 1;
                if differs(original.rgb[o], model.rgb[m], tol) {
                    bg[((y - t) as usize) * w + (x - l) as usize] = true;
                    bg_n += 1;
                }
            }
        }
        // Moved by an offset: the background's differing pixels, on every
        // `step`th pixel each way, as a share of those compared.
        let moved = |dx: i64, dy: i64, step: i64| -> (u64, u64) {
            let (mut n, mut c) = (0u64, 0u64);
            let mut y = t;
            while y < b {
                let mut x = l;
                while x < r {
                    let (mx, my) = (x + dx, y + dy);
                    if inside(mx, my) {
                        let (o, m) = (oi(x, y), mi(mx, my));
                        if !original.sprite[o] && !model.sprite[m] {
                            c += 1;
                            n += u64::from(differs(original.rgb[o], model.rgb[m], tol));
                        }
                    }
                    x += step;
                }
                y += step;
            }
            (n, c)
        };
        let rate = |(n, c): (u64, u64)| (n * 65536).checked_div(c).unwrap_or(u64::MAX);
        let mut shift = None;
        let mut shifted = None;
        // Looked for only when much of the layer differs (past the drift
        // share): a few tiles apart are not a shift.
        if bg_n * 1000 > bg_compared * 5 {
            let mut best = (rate(moved(0, 0, 4)), 0i64, 0i64);
            let better = |cand: (u64, i64, i64), best: (u64, i64, i64)| {
                cand.0 < best.0
                    || (cand.0 == best.0
                        && cand.1.abs() + cand.2.abs() < best.1.abs() + best.2.abs())
            };
            // Across first, the way a side-scroller's camera drifts, then
            // down at the best offset across.
            for pass in 0..2 {
                let range = if pass == 0 {
                    SHIFT_RANGE
                } else {
                    SHIFT_RANGE_Y
                };
                for d in -range..=range {
                    let (dx, dy) = if pass == 1 { (best.1, d) } else { (d, best.2) };
                    let cand = (rate(moved(dx, dy, 4)), dx, dy);
                    if better(cand, best) {
                        best = cand;
                    }
                }
            }
            if (best.1, best.2) != (0, 0) {
                let (n, _) = moved(best.1, best.2, 1);
                if n * 2 <= bg_n {
                    shift = Some([best.1, best.2]);
                    shifted = Some(n);
                }
            }
        }
        let background = LayerFrame {
            pixels: bg_n,
            compared: bg_compared,
            shift,
            shifted,
            ..LayerFrame::default()
        };

        // The sprites, in the frame's own space and relative to the background.
        let sprites_at = |dx: i64, dy: i64| -> (Vec<bool>, u64, u64) {
            let mut d = vec![false; w * h];
            let (mut n, mut c) = (0u64, 0u64);
            for y in t..b {
                for x in l..r {
                    let (mx, my) = (x + dx, y + dy);
                    let o = oi(x, y);
                    let m = inside(mx, my).then(|| mi(mx, my));
                    let theirs = original.sprite[o];
                    let ours = m.is_some_and(|m| model.sprite[m]);
                    if !theirs && !ours {
                        continue;
                    }
                    c += 1;
                    let apart = m.is_none_or(|m| differs(original.rgb[o], model.rgb[m], tol));
                    if apart {
                        d[((y - t) as usize) * w + (x - l) as usize] = true;
                        n += 1;
                    }
                }
            }
            (d, n, c)
        };
        let (sp, sp_n, sp_compared) = sprites_at(0, 0);
        let mut sprites = LayerFrame {
            pixels: sp_n,
            compared: sp_compared,
            ..LayerFrame::default()
        };
        let mut sprite_bits = sp;
        if let Some([dx, dy]) = shift {
            let (rel, rel_n, _) = sprites_at(dx, dy);
            sprites.shifted = Some(rel_n);
            if rel_n < sp_n {
                sprite_bits = rel;
            }
        }

        let mut background = background;
        let (regions, more) = regions_of_ref(&bg, w, h, l, t);
        background.regions = name(regions, model, false);
        background.more = more;
        let (regions, more) = regions_of_ref(&sprite_bits, w, h, l, t);
        sprites.regions = name(regions, model, true);
        sprites.more = more;
        (background, sprites)
    }

    pub(super) fn regions_of_ref(
        bits: &[bool],
        w: usize,
        h: usize,
        l: i64,
        t: i64,
    ) -> (Vec<Region>, usize) {
        let (tw, th) = (w.div_ceil(8), h.div_ceil(8));
        // Each tile's box and count, by tile.
        let mut tiles: Vec<Option<TileBox>> = vec![None; tw * th];
        for y in 0..h {
            for x in 0..w {
                if !bits[y * w + x] {
                    continue;
                }
                let e = &mut tiles[(y / 8) * tw + x / 8];
                *e = Some(match *e {
                    None => (x, y, x, y, 1),
                    Some((x0, y0, x1, y1, n)) => {
                        (x0.min(x), y0.min(y), x1.max(x), y1.max(y), n + 1)
                    }
                });
            }
        }
        let mut seen = vec![false; tw * th];
        let mut out = Vec::new();
        for start in 0..tw * th {
            if seen[start] || tiles[start].is_none() {
                continue;
            }
            seen[start] = true;
            let mut stack = vec![start];
            let (mut x0, mut y0, mut x1, mut y1, mut n) = (usize::MAX, usize::MAX, 0, 0, 0u64);
            while let Some(k) = stack.pop() {
                let (a, b, c, d, m) = tiles[k].expect("a tile in a group has pixels");
                (x0, y0, x1, y1, n) = (x0.min(a), y0.min(b), x1.max(c), y1.max(d), n + m);
                let (tx, ty) = ((k % tw) as i64, (k / tw) as i64);
                for (ox, oy) in [
                    (-1, -1),
                    (0, -1),
                    (1, -1),
                    (-1, 0),
                    (1, 0),
                    (-1, 1),
                    (0, 1),
                    (1, 1),
                ] {
                    let (nx, ny) = (tx + ox, ty + oy);
                    if nx < 0 || ny < 0 || nx >= tw as i64 || ny >= th as i64 {
                        continue;
                    }
                    let j = ny as usize * tw + nx as usize;
                    if !seen[j] && tiles[j].is_some() {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            out.push(Region {
                x: l + x0 as i64,
                y: t + y0 as i64,
                w: (x1 - x0 + 1) as i64,
                h: (y1 - y0 + 1) as i64,
                pixels: n,
                what: Vec::new(),
            });
        }
        out.sort_by_key(|r| (std::cmp::Reverse(r.pixels), r.y, r.x));
        let more = out.len().saturating_sub(BOX_CAP);
        out.truncate(BOX_CAP);
        (out, more)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_present::raster::Thing;

    /// A made-up 32 by 24 frame: a flat backdrop with a stripe every 4
    /// columns (so a shift shows), and a 4 by 4 sprite at `(sx, sy)`.
    fn frame(sx: i64, sy: i64, scroll: i64) -> Frame {
        let (w, h) = (32u32, 24u32);
        let mut f = Frame {
            w,
            h,
            rgb: vec![[0, 0, 80]; (w * h) as usize],
            sprite: vec![false; (w * h) as usize],
            visible: [0, 0, w, h],
            things: vec![Thing {
                name: "hero".into(),
                rect: [sx, sy, 4, 4],
                sprite: true,
            }],
        };
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                if (x + scroll).rem_euclid(5) == 0 && y >= 8 {
                    f.rgb[(y * w as i64 + x) as usize] = [0, 160, 0];
                }
            }
        }
        for y in sy..sy + 4 {
            for x in sx..sx + 4 {
                let i = (y * w as i64 + x) as usize;
                f.rgb[i] = [255, 200, 0];
                f.sprite[i] = true;
            }
        }
        f
    }

    #[test]
    fn the_same_picture_differs_nowhere() {
        let (b, s) = measure(&frame(10, 12, 0), &frame(10, 12, 0), 0);
        assert_eq!((b.pixels, s.pixels), (0, 0));
        assert!(b.regions.is_empty() && s.regions.is_empty());
        assert!(b.compared > 0 && s.compared == 16);
    }

    #[test]
    fn a_sprite_moved_one_pixel_is_one_box_there() {
        let (b, s) = measure(&frame(10, 12, 0), &frame(11, 12, 0), 0);
        assert_eq!(b.pixels, 0, "the background under both sprites is left out");
        assert_eq!(s.regions.len(), 1, "{:?}", s.regions);
        let r = &s.regions[0];
        // The original's left column and the model's new right column.
        assert_eq!((r.x, r.y, r.w, r.h, r.pixels), (10, 12, 5, 4, 8));
        assert_eq!(r.what, vec!["hero".to_owned()]);
        assert!(s.drifted(SPRITES, 32 * 24));
    }

    #[test]
    fn a_shifted_background_says_by_how_much() {
        // The model's camera is 2 px behind: its stripes are drawn 2 px
        // further right, and the hero with them.
        let (b, s) = measure(&frame(10, 12, 0), &frame(12, 12, -2), 0);
        assert!(b.pixels > 0);
        assert_eq!(b.shift, Some([2, 0]));
        assert_eq!(b.shifted, Some(0));
        assert_eq!(
            s.shifted,
            Some(0),
            "right in the world, so not counted twice"
        );
        assert_eq!(s.distance(SPRITES), 0);
    }

    #[test]
    fn a_color_tolerance_takes_small_differences() {
        let mut m = frame(10, 12, 0);
        m.rgb.iter_mut().for_each(|c| c[2] = c[2].saturating_add(3));
        assert!(measure(&frame(10, 12, 0), &m, 0).0.pixels > 0);
        assert_eq!(measure(&frame(10, 12, 0), &m, 3).0.pixels, 0);
    }

    #[test]
    fn only_the_models_visible_part_is_compared() {
        let mut m = frame(10, 12, 0);
        m.visible = [0, 8, 32, 16];
        let mut o = frame(10, 12, 0);
        for c in &mut o.rgb[..32 * 8] {
            *c = [9, 9, 9];
        }
        let (b, _) = measure(&o, &m, 0);
        assert_eq!(b.pixels, 0);
        assert_eq!(b.compared, 32 * 16 - 16);
    }

    #[test]
    fn the_same_input_gives_the_same_counts_and_boxes() {
        let a = measure(&frame(3, 5, 1), &frame(9, 14, -3), 0);
        let b = measure(&frame(3, 5, 1), &frame(9, 14, -3), 0);
        assert_eq!(a, b);
    }

    /// A made-up model drawn by the rasterizer: a level picture and one
    /// 8 by 8 sprite, drawn twice with the sprite 1 px apart.
    #[test]
    fn a_made_up_model_with_one_sprite_moved_one_pixel_has_one_box_there() {
        use gp_present::raster::{Image, draw};
        use gp_present::{Draw, Picture, Present, Scene, Screen};
        let level = Image {
            w: 64,
            h: 48,
            px: (0..64 * 48)
                .map(|i| {
                    if (i % 64) % 7 == 0 {
                        [0, 120, 0, 255]
                    } else {
                        [90, 90, 250, 255]
                    }
                })
                .collect(),
        };
        let sprite = Image {
            w: 8,
            h: 8,
            px: (0..64)
                .map(|i| {
                    if i % 9 == 0 {
                        [0, 0, 0, 0]
                    } else {
                        [250, 40, 40, 255]
                    }
                })
                .collect(),
        };
        let p = Present {
            screen: Screen {
                size: [64, 48],
                aspect: [1, 1],
                overscan: [0, 8, 0, 8],
                sprite_limit: None,
                x_wraps: false,
            },
            pictures: vec![
                Picture {
                    file: "level.png".into(),
                    size: [64, 48],
                },
                Picture {
                    file: "hero.png".into(),
                    size: [8, 8],
                },
            ],
            ..Present::default()
        };
        let pics = vec![Some(level), Some(sprite)];
        let scene = |x: i64| Scene {
            clear: [0, 0, 0],
            camera: [0, 0],
            level: Some(0),
            bodies: vec![Some(Draw {
                pic: 1,
                x,
                y: 20,
                flip_x: false,
                flip_y: false,
            })],
            ..Scene::default()
        };
        let world = gp_sim::World::default();
        let names = vec!["hero".to_owned()];
        let a = draw(&p, &world, &pics, &scene(30), &names, &[]);
        let b = draw(&p, &world, &pics, &scene(31), &names, &[]);
        assert_eq!(a.visible, [0, 8, 64, 32]);
        let (bg, sp) = measure(&a, &b, 0);
        assert_eq!(bg.pixels, 0);
        assert_eq!(sp.regions.len(), 1, "{:?}", sp.regions);
        let r = &sp.regions[0];
        assert!(
            r.x >= 30 && r.x + r.w <= 39 && r.y >= 20 && r.y + r.h <= 28,
            "{r:?}"
        );
        assert_eq!(r.what, vec!["hero".to_owned()]);
        // The same model drawn the same way is the same.
        let (bg, sp) = measure(&a, &draw(&p, &world, &pics, &scene(30), &names, &[]), 0);
        assert_eq!((bg.pixels, sp.pixels), (0, 0));
    }

    #[test]
    fn a_score_says_each_layer_and_keeps_its_boxes_by_frame() {
        let mut sc = Screen::new(&Options::default());
        sc.skip();
        sc.push(&frame(10, 12, 0), &frame(10, 12, 0));
        sc.push(&frame(10, 12, 0), &frame(11, 12, 0));
        sc.push(&frame(10, 12, 0), &frame(11, 12, 0));
        sc.push(&frame(10, 12, 0), &frame(12, 12, -2));
        let frames: Vec<(u64, u64)> = (0..5).map(|k| (k, 100 + k)).collect();
        let s = score(&sc, &frames, 100);
        assert_eq!(s.sprites.first, Some(2));
        assert_eq!(s.sprites.total, 4, "the skipped frame isn't counted");
        assert_eq!(s.sprites.same, 2);
        assert_eq!(
            s.says[0],
            "Characters: 50% of frames the same; different from frame 2: 1 place (hero)."
        );
        assert_eq!(s.background.first, Some(4));
        assert_eq!(
            s.says[1],
            "Background: 75% of frames the same; shifted from frame 4 (the camera): 2 px further right in the rebuilt game."
        );
        let sprites: Vec<_> = s.spans.iter().filter(|x| x.layer == "sprites").collect();
        assert_eq!((sprites[0].from, sprites[0].to), (2, 3));
        assert_eq!(sprites[0].boxes, vec![[10, 12, 5, 4]]);
    }

    /// A made-up frame from a seed: a few colours in runs, so pixels of
    /// the two sides often match, sprite blobs, and a size and visible
    /// part that vary.
    fn noise(seed: u64, w: u32, h: u32, sprites: u32) -> Frame {
        let mut x = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407)
            | 1;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let palette = [
            [0, 0, 80],
            [0, 160, 0],
            [255, 200, 0],
            [250, 40, 40],
            [1, 0, 80],
        ];
        let mut f = Frame {
            w,
            h,
            rgb: Vec::with_capacity((w * h) as usize),
            sprite: vec![false; (w * h) as usize],
            visible: [0, 0, w, h],
            things: vec![Thing {
                name: "hero".into(),
                rect: [3, 4, 10, 10],
                sprite: true,
            }],
        };
        let mut c = palette[0];
        for _ in 0..w * h {
            if next() % 7 == 0 {
                c = palette[(next() % 5) as usize];
            }
            f.rgb.push(c);
        }
        for _ in 0..sprites {
            let (sx, sy) = (
                (next() % u64::from(w)) as u32,
                (next() % u64::from(h)) as u32,
            );
            let (sw, sh) = (1 + (next() % 9) as u32, 1 + (next() % 9) as u32);
            for y in sy..(sy + sh).min(h) {
                for x in sx..(sx + sw).min(w) {
                    let i = (y * w + x) as usize;
                    f.sprite[i] = true;
                    f.rgb[i] = palette[2 + (next() % 3) as usize];
                }
            }
        }
        f
    }

    /// `f` moved by `(dx, dy)`, the pixels moved in from off the frame
    /// left as they were, and some pixels changed.
    fn moved_noise(f: &Frame, dx: i64, dy: i64, seed: u64) -> Frame {
        let mut m = f.clone();
        let (w, h) = (i64::from(f.w), i64::from(f.h));
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (x - dx, y - dy);
                if sx >= 0 && sy >= 0 && sx < w && sy < h {
                    let (to, from) = ((y * w + x) as usize, (sy * w + sx) as usize);
                    m.rgb[to] = f.rgb[from];
                    m.sprite[to] = f.sprite[from];
                }
            }
        }
        let other = noise(seed, f.w, f.h, 3);
        for i in (seed as usize % 11..m.rgb.len()).step_by(37) {
            m.rgb[i] = other.rgb[i];
            m.sprite[i] = other.sprite[i];
        }
        m
    }

    /// The keyed measure gives the old measure's answers, field for
    /// field, on made-up frames: equal and different sizes, a visible
    /// part off the corner, shifted backgrounds both ways (so the shift
    /// search runs and finds one), sprites everywhere and a colour
    /// tolerance.
    #[test]
    fn the_keyed_measure_is_the_old_measure() {
        let mut shifted = 0;
        for seed in 0..240u64 {
            let (w, h) = (24 + (seed % 5) as u32 * 13, 20 + (seed % 3) as u32 * 11);
            let o = noise(seed, w, h, (seed % 9) as u32);
            let (dx, dy) = ((seed % 13) as i64 - 6, (seed % 7) as i64 - 3);
            let mut m = match seed % 4 {
                0 => noise(seed + 1000, w, h, 4),
                1 => moved_noise(&o, dx, dy, seed),
                2 => moved_noise(&o, dx * 3, 0, seed),
                _ => {
                    // A bigger model whose visible part sits off the corner.
                    let mut big = noise(seed + 2000, w + 9, h + 5, 6);
                    big.visible = [2, 3, w + 1, h - 1];
                    big
                }
            };
            if seed % 6 == 5 {
                m.visible = [1, 2, w.saturating_sub(5), h.saturating_sub(4)];
            }
            for tol in [0u8, 1, 40] {
                let new = measure(&o, &m, tol);
                let old = reference::measure(&o, &m, tol);
                assert_eq!(new, old, "seed {seed} tolerance {tol}");
                shifted += usize::from(new.0.shift.is_some());
            }
        }
        assert!(
            shifted > 20,
            "the shift search found a shift {shifted} times"
        );
        // Nothing to compare: a visible part off the original's frame.
        let o = noise(1, 20, 20, 2);
        let mut m = noise(2, 40, 40, 2);
        m.visible = [25, 25, 10, 10];
        assert_eq!(measure(&o, &m, 0), reference::measure(&o, &m, 0));
    }

    /// The boxes by a tile's row at a time are the boxes by pixel.
    #[test]
    fn the_boxes_by_rows_are_the_boxes_by_pixel() {
        for seed in 0..200u64 {
            let (w, h) = (1 + (seed % 37) as usize, 1 + (seed * 7 % 29) as usize);
            let f = noise(seed, w as u32, h as u32, 5);
            let bits: Vec<bool> = f
                .rgb
                .iter()
                .zip(&f.sprite)
                .map(|(c, s)| *s || c[1] == 160)
                .collect();
            assert_eq!(
                regions_of(&bits, w, h, 5, 7),
                reference::regions_of_ref(&bits, w, h, 5, 7),
                "seed {seed}"
            );
        }
    }
}
