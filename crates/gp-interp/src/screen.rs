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

fn differs(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(b).any(|(x, y)| x.abs_diff(y) > tol)
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

/// Both layers on one frame.
pub fn measure(original: &Frame, model: &Frame, tol: u8) -> (LayerFrame, LayerFrame) {
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
    for y in 0..h {
        for x in 0..w {
            if !bits[y * w + x] {
                continue;
            }
            let e = &mut tiles[(y / 8) * tw + x / 8];
            *e = Some(match *e {
                None => (x, y, x, y, 1),
                Some((x0, y0, x1, y1, n)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y), n + 1),
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
}
