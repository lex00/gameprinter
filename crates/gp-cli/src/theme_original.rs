//! The originality check (`gp theme check --original`, docs/theme-kit.md,
//! "The originality check"): with the player's own lift of the game, each
//! themed art slot is compared with the original's after scaling, and each
//! sound replacement's melody with the original cue's, and a theme too
//! close to the original is refused. A theme folder must also hold no run
//! of the ROM's bytes.
//!
//! The measures:
//!
//! | Measure | What it is | Refused at |
//! |---|---|---|
//! | Art agreement | Over the pixels opaque in either picture, the share whose tone (clear, dark, middle, light) is the same, the themed picture scaled to the original's size and its tones matched to the original's in the best of the six orders, the best over shifts of up to [`SHIFT`] pixels and the mirror | [`ART_REFUSE`] or more, on a slot with at least [`ART_MIN_PIXELS`] such pixels |
//! | Shape | The F1 of the two outlines' pixels, the best over the same moves | [`SHAPE_REFUSE`] or more |
//! | Logo | Both of the above over the title's cells the game's font can't read | As art |
//! | Names | Text read from a title or score display ([`read_picture`]) holding a name | Any |
//! | Melody | The longest common run of note steps (intervals in semitones) of the replacement and of each channel of the original cue, over the longer's count, so a transposed copy scores 1 | [`MELODY_REFUSE`] or more, when both have at least [`MELODY_MIN_STEPS`] steps |
//! | ROM bytes | A run of [`ROM_RUN`] bytes or more of the ROM in any file of the theme | Any |

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use gp_model::Model;
use gp_theme::pix::Rgba;
use serde::Serialize;

use crate::theme_kit::{Kit, Local};

/// Art agreement at or above this refuses a slot.
pub const ART_REFUSE: f64 = 0.75;
/// A slot needs this many pixels opaque in either picture to be judged.
pub const ART_MIN_PIXELS: usize = 24;
/// Melody similarity at or above this refuses a sound.
pub const MELODY_REFUSE: f64 = 0.6;
/// Both melodies need this many note steps to be judged.
pub const MELODY_MIN_STEPS: usize = 6;
/// A run of this many ROM bytes in a theme file refuses it.
pub const ROM_RUN: usize = 16;
/// Shape similarity at or above this refuses a slot.
pub const SHAPE_REFUSE: f64 = 0.85;
/// The shifts the art is compared at, each way, in pixels.
pub const SHIFT: i32 = 2;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Score {
    pub what: String,
    /// Art: the tone agreement, the best over the shifts and the mirror.
    /// Sound: the melody similarity. Text: 1 when a name was read.
    pub value: f64,
    /// Art: the shape similarity (outlines within a pixel), the best over
    /// the shifts and the mirror.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<f64>,
    /// What gave the best score: a shift, a mirror, a word read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub how: String,
    /// Whether it was judged (enough pixels or notes).
    pub judged: bool,
    pub refused: bool,
}

/// An RGBA picture's tones: clear (or its most common color, a
/// backdrop) 0, then thirds of its lightness.
pub(crate) fn picture_tones(img: &Rgba, backdrop: bool) -> Vec<u8> {
    let mut img = img.clone();
    if backdrop {
        let mut count: BTreeMap<[u8; 4], usize> = BTreeMap::new();
        for p in &img.pixels {
            *count.entry(*p).or_default() += 1;
        }
        if let Some((c, _)) = count.into_iter().max_by_key(|(_, n)| *n) {
            for p in &mut img.pixels {
                if *p == c {
                    *p = [0; 4];
                }
            }
        }
    }
    crate::theme_kit::tones_of(&img)
}

/// A bank of tile pixels as indices: the original's, the theme's when it
/// restyles the slot, and the bank's size in tiles.
pub(crate) type BankPair = (Vec<u8>, Option<Vec<u8>>, [u32; 2]);

/// The tiles banks, original and themed, by slot.
pub(crate) fn banks_of(
    m: &Model,
    local: &Local,
    theme: &gp_theme::Resolved,
    assets: &Path,
    map: &gp_assets::AssetMap,
) -> Result<BTreeMap<String, BankPair>> {
    let mut banks = BTreeMap::new();
    for (slot, sz) in &local.banks {
        let Some(from) = m.assets.get(slot).and_then(|s| s.from.as_ref()) else {
            continue;
        };
        let Some(asset) = map.assets.iter().find(|a| a.name() == from.asset) else {
            continue;
        };
        let size = (sz[0] * 8, sz[1] * 8);
        let orig = gp_assets::chr::read_png(&fs::read(assets.join(asset.file()))?, size)
            .map_err(anyhow::Error::msg)?;
        let themed = match theme.restyle_slot(slot) {
            Some((_, file, _)) if file.extension().is_some_and(|e| e == "pix") => Some(
                gp_theme::pix::indexed(&fs::read_to_string(&file)?)
                    .map_err(anyhow::Error::msg)?
                    .2,
            ),
            Some((_, file, _)) => Some(
                gp_assets::chr::read_png(&fs::read(&file)?, size).map_err(anyhow::Error::msg)?,
            ),
            None => None,
        };
        banks.insert(slot.clone(), (orig, themed, *sz));
    }
    Ok(banks)
}

/// A layout entry drawn from the original's tiles, `w` by `h`, as tones.
pub(crate) fn layout_tones(
    m: &Model,
    assets: &Path,
    banks: &BTreeMap<String, BankPair>,
    slot: &str,
    entry: &str,
    w: u32,
    h: u32,
) -> Option<Tones> {
    let (names, _) = gp_look::present::layout_cells(m, assets, slot, entry).ok()?;
    let (_, (orig_bank, _, sz)) = banks.iter().find(|(s, _)| {
        m.present
            .as_ref()
            .and_then(|p| p.hud.as_ref())
            .is_some_and(|h| &h.tiles == *s)
            || gp_look::present::level_tiles_slot(m).as_deref() == Some(s.as_str())
    })?;
    let mut orig = vec![0u8; (w * h) as usize];
    for r in 0..h / 8 {
        for c in 0..w / 8 {
            let Some(t) = names[(r * 32 + c) as usize] else {
                continue;
            };
            let t = u32::from(t);
            let (tx, ty) = ((t % sz[0]) * 8, (t / sz[0]) * 8);
            for y in 0..8 {
                for x in 0..8 {
                    let v = orig_bank[((ty + y) * sz[0] * 8 + tx + x) as usize];
                    orig[((r * 8 + y) * w + c * 8 + x) as usize] = v;
                }
            }
        }
    }
    Some(Tones { w, h, t: orig })
}

/// The art scores: each sprite slot (its frames' tiles in the themed and
/// the original banks) and each picture slot.
pub fn art_scores(
    m: &Model,
    kit: &Kit,
    local: &Local,
    theme: &gp_theme::Resolved,
    assets: &Path,
    map: &gp_assets::AssetMap,
    authored: bool,
) -> Result<Vec<Score>> {
    let mut out = Vec::new();
    let banks = banks_of(m, local, theme, assets, map)?;
    for a in &kit.art {
        if a.kind == "picture" {
            let Some(rest) = a.id.strip_prefix("picture.") else {
                continue;
            };
            let Some((slot, entry)) = rest.split_once('.') else {
                continue;
            };
            let Some((_, file, _)) = theme.restyle_entry(slot, entry) else {
                continue;
            };
            let themed = Rgba::load(&file).map_err(anyhow::Error::msg)?;
            let (w, h) = (a.size[0], a.size[1]);
            let Some(o) = layout_tones(m, assets, &banks, slot, entry, w, h) else {
                continue;
            };
            let themed = crate::theme_paper::scale_to(&themed, w, h);
            let t = Tones {
                w,
                h,
                t: picture_tones(&themed, true),
            };
            out.push(score(
                a.id.clone(),
                &compare(std::slice::from_ref(&o), std::slice::from_ref(&t)),
            ));
            // The original's logo: the cells the game's font can't read,
            // compared on their own.
            if a.role == "menu"
                && let Some((text, _)) = local.cells.get(&a.id)
            {
                let mut b: Option<[u32; 4]> = None;
                for (r, line) in text.iter().enumerate() {
                    for (c, ch) in line.chars().enumerate() {
                        if ch == '?' {
                            let (x, y) = (c as u32 * 8, r as u32 * 8);
                            b = Some(match b {
                                None => [x, y, x + 8, y + 8],
                                Some(b) => {
                                    [b[0].min(x), b[1].min(y), b[2].max(x + 8), b[3].max(y + 8)]
                                }
                            });
                        }
                    }
                }
                if let Some(b) = b {
                    let cut = |p: &Tones| Tones {
                        w: b[2] - b[0],
                        h: b[3] - b[1],
                        t: (b[1]..b[3])
                            .flat_map(|y| (b[0]..b[2]).map(move |x| (x, y)))
                            .map(|(x, y)| p.t[(y * p.w + x) as usize])
                            .collect(),
                    };
                    out.push(score(
                        format!("{} logo", a.id),
                        &compare(&[cut(&o)], &[cut(&t)]),
                    ));
                }
            }
            // Words in a title or a score display: no name of the original.
            if a.role == "menu" || a.role == "hud" {
                let mut glyphs = own_glyphs();
                for (c, cut) in &local.letters {
                    if let Some((bank, _, sz)) = banks.get(&cut.slot) {
                        let f = frame_of(bank, *sz, cut);
                        let on: Vec<bool> = f.t.iter().map(|v| *v != 0).collect();
                        if let Some((w, h, on)) = tight(8, 8, &on) {
                            glyphs.push(Glyph {
                                c: c.to_ascii_uppercase(),
                                w,
                                h,
                                on,
                            });
                        }
                    }
                }
                let read = read_picture(&themed, &glyphs);
                out.push(text_score(
                    format!("{} text", a.id),
                    &read,
                    &names_for_text(m, authored),
                ));
            }
            continue;
        }
        let Some(cuts) = local.cuts.get(&a.id) else {
            continue;
        };
        let (mut o_all, mut t_all) = (Vec::new(), Vec::new());
        for c in cuts {
            let Some((orig, Some(themed), sz)) = banks.get(&c.slot) else {
                continue;
            };
            o_all.push(frame_of(orig, *sz, c));
            t_all.push(frame_of(themed, *sz, c));
        }
        if o_all.is_empty() {
            continue;
        }
        let mut c = compare(&o_all, &t_all);
        // Digits and 8 by 8 marks have the shape their meaning gives them
        // (a 0, an arrow): only their tones are judged.
        if a.kind == "font" || a.size[0] * a.size[1] <= 64 {
            c.shape = 0.0;
            c.how.push_str("; shape not judged at 8 by 8");
        }
        out.push(score(a.id.clone(), &c));
    }
    Ok(out)
}

fn score(what: String, c: &Compared) -> Score {
    let judged = c.union >= ART_MIN_PIXELS;
    Score {
        what,
        value: c.tone,
        shape: Some(c.shape),
        how: c.how.clone(),
        judged,
        refused: judged && (c.tone >= ART_REFUSE || c.shape >= SHAPE_REFUSE),
    }
}

/// A picture as tones (0 clear, 1 to 3), `w` by `h`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tones {
    pub w: u32,
    pub h: u32,
    pub t: Vec<u8>,
}

impl Tones {
    fn get(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            0
        } else {
            self.t[(y as u32 * self.w + x as u32) as usize]
        }
    }

    /// Moved by `dx`, `dy` and mirrored left to right first when `mirror`.
    pub fn moved(&self, dx: i32, dy: i32, mirror: bool) -> Self {
        let mut t = vec![0; self.t.len()];
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let sx = x - dx;
                let sx = if mirror { self.w as i32 - 1 - sx } else { sx };
                t[(y as u32 * self.w + x as u32) as usize] = self.get(sx, y - dy);
            }
        }
        Self {
            w: self.w,
            h: self.h,
            t,
        }
    }

    /// The outline: opaque pixels with a clear 4-neighbour.
    fn outline(&self) -> Vec<bool> {
        let mut o = vec![false; self.t.len()];
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                if self.get(x, y) == 0 {
                    continue;
                }
                if [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| self.get(x + dx, y + dy) == 0)
                {
                    o[(y as u32 * self.w + x as u32) as usize] = true;
                }
            }
        }
        o
    }
}

/// A frame of a sprite slot from a bank, by its cut.
pub(crate) fn frame_of(bank: &[u8], sz: [u32; 2], cut: &crate::theme_kit::Cut) -> Tones {
    let w = cut.places.iter().map(|p| p.col + 1).max().unwrap_or(1) * 8;
    let h = cut.places.iter().map(|p| p.row + 1).max().unwrap_or(1) * 8;
    let mut t = vec![0u8; (w * h) as usize];
    for p in &cut.places {
        if p.tile >= sz[0] * sz[1] {
            continue;
        }
        let (tx, ty) = ((p.tile % sz[0]) * 8, (p.tile / sz[0]) * 8);
        for y in 0..8 {
            for x in 0..8 {
                let sx = if p.flip_x { 7 - x } else { x };
                let sy = if p.flip_y { 7 - y } else { y };
                let v = bank
                    .get(((ty + sy) * sz[0] * 8 + tx + sx) as usize)
                    .copied()
                    .unwrap_or(0);
                t[((p.row * 8 + y) * w + p.col * 8 + x) as usize] = v;
            }
        }
    }
    Tones { w, h, t }
}

/// What [`compare`] found.
#[derive(Debug, Clone, Default)]
pub struct Compared {
    /// The best tone agreement (tones matched in the best of six orders).
    pub tone: f64,
    /// The best shape similarity: the F1 of the two outlines' pixels
    /// (the share of each outline's pixels on the other's).
    pub shape: f64,
    /// The pixels opaque in either, at no shift.
    pub union: usize,
    pub how: String,
}

const PERMS: [[u8; 3]; 6] = [
    [1, 2, 3],
    [1, 3, 2],
    [2, 1, 3],
    [2, 3, 1],
    [3, 1, 2],
    [3, 2, 1],
];

/// A slot's frames, the theme's against the original's, at every shift
/// of up to [`SHIFT`] pixels each way, plain and mirrored (the same move
/// for all of a slot's frames): the best tone agreement and the best
/// shape similarity, each with its move.
pub fn compare(orig: &[Tones], themed: &[Tones]) -> Compared {
    let mut out = Compared::default();
    let outlines: Vec<Vec<bool>> = orig.iter().map(Tones::outline).collect();
    for mirror in [false, true] {
        for dy in -SHIFT..=SHIFT {
            for dx in -SHIFT..=SHIFT {
                let (mut agree, mut union) = ([0usize; 6], 0usize);
                let (mut ea, mut ma, mut eb, mut mb) = (0usize, 0usize, 0usize, 0usize);
                for ((o, t), oo) in orig.iter().zip(themed).zip(&outlines) {
                    if (o.w, o.h) != (t.w, t.h) {
                        continue;
                    }
                    let m = t.moved(dx, dy, mirror);
                    for (a, b) in m.t.iter().zip(&o.t) {
                        if *a == 0 && *b == 0 {
                            continue;
                        }
                        union += 1;
                        if *a != 0 && *b != 0 {
                            for (k, p) in PERMS.iter().enumerate() {
                                if p[usize::from(*a - 1).min(2)] == *b {
                                    agree[k] += 1;
                                }
                            }
                        }
                    }
                    let mo = m.outline();
                    let near = |e: &[bool], x: i32, y: i32| e[(y as u32 * o.w + x as u32) as usize];
                    for y in 0..o.h as i32 {
                        for x in 0..o.w as i32 {
                            let i = (y as u32 * o.w + x as u32) as usize;
                            if mo[i] {
                                ea += 1;
                                if near(oo, x, y) {
                                    ma += 1;
                                }
                            }
                            if oo[i] {
                                eb += 1;
                                if near(&mo, x, y) {
                                    mb += 1;
                                }
                            }
                        }
                    }
                }
                if dx == 0 && dy == 0 && !mirror {
                    out.union = union;
                }
                let how = format!("{}shift {dx},{dy}", if mirror { "mirrored, " } else { "" });
                if union > 0 {
                    let tone = *agree.iter().max().unwrap_or(&0) as f64 / union as f64;
                    if tone > out.tone {
                        out.tone = tone;
                        out.how = format!("tone at {how}");
                    }
                }
                if ea > 0 && eb > 0 {
                    let (p, r) = (ma as f64 / ea as f64, mb as f64 / eb as f64);
                    let f1 = if p + r > 0.0 {
                        2.0 * p * r / (p + r)
                    } else {
                        0.0
                    };
                    if f1 > out.shape {
                        out.shape = f1;
                        if out.how.is_empty() || f1 >= SHAPE_REFUSE {
                            out.how = format!("shape at {how}");
                        }
                    }
                }
            }
        }
    }
    out
}

/// A glyph as a mask: the theme's own 5 by 7 font at a scale, or one of
/// the game's 8 by 8 tiles.
#[derive(Debug, Clone)]
pub struct Glyph {
    pub c: char,
    pub w: u32,
    pub h: u32,
    pub on: Vec<bool>,
}

/// A mask cut to its ink's box.
fn tight(w: u32, h: u32, on: &[bool]) -> Option<(u32, u32, Vec<bool>)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if on[(y * w + x) as usize] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == u32::MAX {
        return None;
    }
    let (tw, th) = (x1 - x0, y1 - y0);
    let mut out = Vec::with_capacity((tw * th) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            out.push(on[(y * w + x) as usize]);
        }
    }
    Some((tw, th, out))
}

/// The theme's own 5 by 7 font: letters and digits, each cut to its ink.
pub fn own_glyphs() -> Vec<Glyph> {
    let mut out = Vec::new();
    for c in ('A'..='Z').chain('0'..='9') {
        let Some(g) = crate::theme_paper::glyph(c) else {
            continue;
        };
        let on: Vec<bool> = g
            .iter()
            .flat_map(|r| r.bytes().map(|b| b != b'.' && b != b' '))
            .collect();
        if let Some((w, h, on)) = tight(5, 7, &on) {
            out.push(Glyph { c, w, h, on });
        }
    }
    out
}

/// A glyph's name for a letter's mask, the best match at most an eighth
/// of its box wrong, the box's shape within a third.
fn name_letter(bw: u32, bh: u32, bon: &[bool], glyphs: &[Glyph]) -> Option<char> {
    let mut best: Option<(f64, char)> = None;
    for g in glyphs {
        let (ra, rb) = (
            f64::from(bw) / f64::from(bh),
            f64::from(g.w) / f64::from(g.h),
        );
        if (ra - rb).abs() > 0.34 * rb.max(0.2) + 0.08 {
            continue;
        }
        let mut bad = 0;
        for gy in 0..g.h {
            for gx in 0..g.w {
                let sx = ((gx * 2 + 1) * bw / (g.w * 2)).min(bw - 1);
                let sy = ((gy * 2 + 1) * bh / (g.h * 2)).min(bh - 1);
                if bon[(sy * bw + sx) as usize] != g.on[(gy * g.w + gx) as usize] {
                    bad += 1;
                }
            }
        }
        let wrong = f64::from(bad) / f64::from(g.w * g.h);
        if wrong <= 0.125 && best.is_none_or(|(b, _)| wrong < b) {
            best = Some((wrong, g.c));
        }
    }
    best.map(|(_, c)| c)
}

/// Text read from an ink mask (`w` by `h`): its 8-connected pieces 5 to
/// 64 pixels tall are letters, each scaled to each glyph's box and named
/// by the best match; letters of one height and row chain left to right
/// into lines, a gap wider than half a letter's height a space. Lines are
/// joined by spaces; a letter no glyph matches is `?`.
pub fn read_mask(w: u32, h: u32, ink: &[bool], glyphs: &[Glyph]) -> String {
    let mut seen = vec![false; ink.len()];
    // (y0, x0, x1, y1, char)
    let mut letters: Vec<(u32, u32, u32, u32, char)> = Vec::new();
    for start in 0..ink.len() {
        if !ink[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let mut px = Vec::new();
        while let Some(i) = stack.pop() {
            px.push(i);
            let (x, y) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = (ny as u32 * w + nx as u32) as usize;
                    if ink[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            if px.len() > 6000 {
                break;
            }
        }
        let xs = px.iter().map(|i| *i as u32 % w);
        let ys = px.iter().map(|i| *i as u32 / w);
        let (x0, x1) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0) + 1);
        let (y0, y1) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0) + 1);
        let (bw, bh) = (x1 - x0, y1 - y0);
        if !(5..=64).contains(&bh) || bw > bh * 2 {
            continue;
        }
        let mut on = vec![false; (bw * bh) as usize];
        for i in &px {
            let (x, y) = (*i as u32 % w, *i as u32 / w);
            on[((y - y0) * bw + x - x0) as usize] = true;
        }
        let c = name_letter(bw, bh, &on, glyphs).unwrap_or('?');
        letters.push((y0, x0, x1, y1, c));
    }
    letters.sort_by_key(|l| (l.0, l.1));
    let mut used = vec![false; letters.len()];
    let mut lines = Vec::new();
    for i in 0..letters.len() {
        if used[i] {
            continue;
        }
        let (ty, _, _, by, _) = letters[i];
        let lh = by - ty;
        let mut row: Vec<usize> = (i..letters.len())
            .filter(|&j| {
                !used[j]
                    && letters[j].0.abs_diff(ty) <= lh / 3 + 1
                    && (letters[j].3 - letters[j].0).abs_diff(lh) <= lh / 3 + 1
            })
            .collect();
        row.sort_by_key(|&j| letters[j].1);
        let mut line = String::new();
        let mut last: Option<u32> = None;
        for j in row {
            used[j] = true;
            let (_, x0, x1, _, c) = letters[j];
            if let Some(e) = last
                && x0.saturating_sub(e) * 2 > lh
            {
                line.push(' ');
            }
            line.push(c);
            last = Some(x1);
        }
        lines.push(line);
    }
    lines.join(" ")
}

/// Text read from a picture's tones: [`read_mask`] on all its ink.
#[cfg(test)]
pub fn read_text(p: &Tones, glyphs: &[Glyph]) -> String {
    let ink: Vec<bool> = p.t.iter().map(|v| *v != 0).collect();
    read_mask(p.w, p.h, &ink, glyphs)
}

/// Text read from a picture: [`read_mask`] on each color drawn in 30
/// pixels or more (a letter's own color), then on everything well darker
/// or lighter than the picture's most common color (a letter with a
/// shadow), joined.
pub fn read_picture(img: &Rgba, glyphs: &[Glyph]) -> String {
    let mut count: BTreeMap<[u8; 4], usize> = BTreeMap::new();
    for p in &img.pixels {
        *count.entry(*p).or_default() += 1;
    }
    let mut colors: Vec<([u8; 4], usize)> = count.into_iter().collect();
    colors.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let mut out = Vec::new();
    let bg = colors.first().map_or([0; 4], |c| c.0);
    for (c, n) in colors.iter().skip(1).take(12) {
        if *n < 30 || c[3] < 128 {
            continue;
        }
        let ink: Vec<bool> = img.pixels.iter().map(|p| p == c).collect();
        out.push(read_mask(img.width, img.height, &ink, glyphs));
    }
    let lb = gp_theme::color::Rgb([bg[0], bg[1], bg[2]]).lab()[0];
    let ink: Vec<bool> = img
        .pixels
        .iter()
        .map(|p| {
            p[3] >= 128 && (gp_theme::color::Rgb([p[0], p[1], p[2]]).lab()[0] - lb).abs() > 25.0
        })
        .collect();
    out.push(read_mask(img.width, img.height, &ink, glyphs));
    // Pieces no glyph names are left out.
    let mut words: Vec<String> = Vec::new();
    for t in out {
        let w: Vec<&str> = t
            .split_whitespace()
            .filter(|w| w.chars().any(|c| c != '?'))
            .collect();
        if !w.is_empty() {
            words.push(w.join(" "));
        }
    }
    words.join(" | ")
}

/// The names a theme's text may not hold: the model's title words and the
/// names the kit leaves out, of four letters or more. An original
/// template's (`source.how = "authored"`) title is its own, so with
/// `authored` its words are left out unless a name of the model's
/// entities holds them too: Rivet Rodeo's paper says RIVET RODEO, and
/// Donkey Kong's names, which its model keeps, are still refused.
pub fn names_for_text(m: &Model, authored: bool) -> Vec<String> {
    let mut own: Vec<String> = Vec::new();
    if authored {
        let mut rest = m.clone();
        rest.model.title = String::new();
        let others = crate::theme_kit::names_of(&rest);
        own = m
            .model
            .title
            .split_whitespace()
            .map(str::to_lowercase)
            .filter(|w| !others.contains(w))
            .collect();
    }
    let mut n: Vec<String> = crate::theme_kit::names_of(m)
        .into_iter()
        .filter(|w| w.len() >= 4 && w.chars().all(|c| c.is_ascii_alphabetic()))
        .filter(|w| !own.contains(w))
        .collect();
    n.sort();
    n.dedup();
    n
}

/// A text score: refused when the text read holds a name, as a word or
/// run together with others.
pub fn text_score(what: String, read: &str, names: &[String]) -> Score {
    let mut found = crate::theme_words::denied(read, names);
    let squashed: String = read
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase();
    for n in names {
        if squashed.contains(n.as_str()) && !found.contains(n) {
            found.push(n.clone());
        }
    }
    Score {
        what,
        value: if found.is_empty() { 0.0 } else { 1.0 },
        shape: None,
        how: if read.is_empty() {
            "no text read".into()
        } else {
            format!(
                "read \"{read}\"{}",
                if found.is_empty() {
                    String::new()
                } else {
                    format!(", names {}", found.join(", "))
                }
            )
        },
        judged: true,
        refused: !found.is_empty(),
    }
}

/// The note steps of a melody: the semitone change between each two notes
/// in a row that differ.
pub fn steps(notes: &[u8]) -> Vec<i32> {
    let mut dedup: Vec<u8> = Vec::new();
    for &n in notes {
        if dedup.last() != Some(&n) {
            dedup.push(n);
        }
    }
    dedup
        .windows(2)
        .map(|w| i32::from(w[1]) - i32::from(w[0]))
        .collect()
}

/// The longest common substring of two step lists, over the longer's
/// length: 1 for the same melody in any key.
pub fn melody_similarity(a: &[i32], b: &[i32]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut best = 0;
    let mut prev = vec![0usize; b.len() + 1];
    for x in a {
        let mut cur = vec![0usize; b.len() + 1];
        for (j, y) in b.iter().enumerate() {
            if x == y {
                cur[j + 1] = prev[j] + 1;
                best = best.max(cur[j + 1]);
            }
        }
        prev = cur;
    }
    best as f64 / a.len().max(b.len()) as f64
}

/// A pulse or triangle period's MIDI note, on the NTSC NES.
fn period_note(period: u16, triangle: bool) -> Option<u8> {
    if period < 8 {
        return None;
    }
    let cpu = 1_789_773.0;
    let f = cpu / (f64::from(if triangle { 32 } else { 16 }) * (f64::from(period) + 1.0));
    let n = 69.0 + 12.0 * (f / 440.0).log2();
    (0.0..128.0).contains(&n).then(|| n.round() as u8)
}

/// Each tone channel's notes from a run of APU register writes: a note
/// each time a channel's period high byte is written (a new note on the
/// NES) or its period changes by a semitone or more.
pub fn apu_melodies(frames: &[Vec<(u8, u8)>]) -> [Vec<u8>; 3] {
    let mut out: [Vec<u8>; 3] = Default::default();
    let mut lo = [0u8; 3];
    let mut hi = [0u8; 3];
    // The frame and note each channel last noted, so a note's low and
    // high byte written in one frame count once.
    let mut noted: [Option<(usize, u8)>; 3] = [None; 3];
    for (fi, f) in frames.iter().enumerate() {
        for &(r, v) in f {
            let (ch, high) = match r {
                0x02 => (0, false),
                0x03 => (0, true),
                0x06 => (1, false),
                0x07 => (1, true),
                0x0A => (2, false),
                0x0B => (2, true),
                _ => continue,
            };
            if high {
                hi[ch] = v & 7;
            } else {
                lo[ch] = v;
            }
            let p = u16::from(lo[ch]) | u16::from(hi[ch]) << 8;
            let Some(n) = period_note(p, ch == 2) else {
                continue;
            };
            let same_frame = noted[ch] == Some((fi, n));
            let changed = out[ch].last() != Some(&n);
            if (high && !same_frame) || changed {
                out[ch].push(n);
                noted[ch] = Some((fi, n));
            }
        }
    }
    out
}

/// A sound file's notes: a tune's own, or a WAV's by pitch tracking (an
/// autocorrelation a sixtieth of a second at a time).
pub fn file_notes(file: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    if file.ends_with(".tune") {
        let t =
            gp_theme::tune::Tune::parse(std::str::from_utf8(bytes)?).map_err(anyhow::Error::msg)?;
        return Ok(t.melody().into_iter().map(|(_, n)| n).collect());
    }
    let s = gp_theme::tune::samples_of(file, bytes).map_err(anyhow::Error::msg)?;
    let rate = f64::from(gp_theme::tune::RATE);
    let win = (rate / 60.0) as usize * 2;
    let mut notes = Vec::new();
    for chunk in s.chunks(win) {
        if chunk.len() < win {
            break;
        }
        let energy: f64 = chunk.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / win as f64;
        if energy < 1.0e5 {
            continue;
        }
        let (lo, hi) = ((rate / 1500.0) as usize, (rate / 60.0) as usize);
        let mut best = (0.0, 0);
        for lag in lo..hi.min(win - 1) {
            let c: f64 = chunk[..win - lag]
                .iter()
                .zip(&chunk[lag..])
                .map(|(a, b)| f64::from(*a) * f64::from(*b))
                .sum();
            if c > best.0 {
                best = (c, lag);
            }
        }
        if best.1 > 0 {
            let f = rate / best.1 as f64;
            let n = 69.0 + 12.0 * (f / 440.0).log2();
            notes.push(n.round().clamp(0.0, 127.0) as u8);
        }
    }
    Ok(notes)
}

/// The original cue of each sound role: the model's own sound (its hooks'
/// APU writes) from the frame the role starts, for its length, in the
/// first of `inputs` that starts it.
pub fn original_cues(
    bundle: &gp_interp::lifted::Bundle,
    m: &Model,
    inputs: &[Vec<gp_runtime::InputFrame>],
    (dir, paths): (&std::path::Path, &gp_evidence::Paths),
) -> Result<BTreeMap<String, [Vec<u8>; 3]>> {
    let mut out = BTreeMap::new();
    let Some(snd) = m.present.as_ref().and_then(|p| p.sound.as_ref()) else {
        return Ok(out);
    };
    for frames in inputs {
        let mut rt = crate::theme_run::Run::open(bundle, dir, paths)?;
        let mut notes = Vec::new();
        let cues = gp_present::sound::Cues {
            roles: gp_look::sound::role_cues(snd, rt.spec(), &mut notes),
            ..Default::default()
        };
        let mut dir = gp_present::sound::Director::new();
        type Open = (u32, Vec<Vec<(u8, u8)>>);
        let mut open: BTreeMap<String, Open> = BTreeMap::new();
        for f in frames {
            let (_, w) = rt.advance(f).map_err(anyhow::Error::msg)?;
            for v in open.values_mut() {
                v.1.push(w.clone());
            }
            let (starts, _) = dir.roles(&cues, rt.world());
            for s in starts {
                if !out.contains_key(&s) && !open.contains_key(&s) {
                    let len = snd
                        .roles
                        .iter()
                        .find(|r| r.name == s)
                        .and_then(|r| r.frames)
                        .unwrap_or(120);
                    open.insert(s, (len, Vec::new()));
                }
            }
            let done: Vec<String> = open
                .iter()
                .filter(|(_, v)| v.1.len() as u32 >= v.0)
                .map(|(k, _)| k.clone())
                .collect();
            for k in done {
                if let Some((_, w)) = open.remove(&k) {
                    out.insert(k, apu_melodies(&w));
                }
            }
        }
        for (k, (_, w)) in open {
            out.entry(k).or_insert_with(|| apu_melodies(&w));
        }
        if out.len() == snd.roles.len() {
            break;
        }
    }
    Ok(out)
}

/// The sound scores: each replacement's melody against its role's
/// original cue, every channel.
pub fn sound_scores(
    theme: &gp_theme::Resolved,
    cues: &BTreeMap<String, [Vec<u8>; 3]>,
) -> Result<Vec<Score>> {
    let mut out = Vec::new();
    for l in &theme.layers {
        for s in &l.theme.sound {
            let bytes = fs::read(l.dir.join(&s.file)).with_context(|| s.file.clone())?;
            let Ok(notes) = file_notes(&s.file, &bytes) else {
                out.push(Score {
                    what: format!("sound {}", s.role),
                    value: 0.0,
                    judged: false,
                    refused: false,
                    ..Score::default()
                });
                continue;
            };
            let mine = steps(&notes);
            let Some(orig) = cues.get(&s.role) else {
                out.push(Score {
                    what: format!("sound {} (no cue heard)", s.role),
                    value: 0.0,
                    judged: false,
                    refused: false,
                    ..Score::default()
                });
                continue;
            };
            let (mut best, mut judged) = (0.0f64, false);
            for ch in orig {
                let o = steps(ch);
                let v = melody_similarity(&mine, &o);
                if mine.len() >= MELODY_MIN_STEPS && o.len() >= MELODY_MIN_STEPS {
                    judged = true;
                }
                best = best.max(v);
            }
            out.push(Score {
                what: format!("sound {}", s.role),
                value: best,
                judged,
                refused: judged && best >= MELODY_REFUSE,
                ..Score::default()
            });
        }
    }
    Ok(out)
}

/// Files of a theme folder holding a run of [`ROM_RUN`] ROM bytes, raw.
pub fn rom_runs(dir: &Path, rom: &[u8]) -> Result<Vec<String>> {
    let mut windows = std::collections::HashSet::new();
    for w in rom.windows(ROM_RUN) {
        // Runs of one byte (blank tiles, padding) are in every file.
        if w.iter().any(|b| *b != w[0]) {
            windows.insert(w);
        }
    }
    let mut out = Vec::new();
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if !p.is_file() {
            continue;
        }
        let bytes = fs::read(&p)?;
        if bytes.windows(ROM_RUN).any(|w| windows.contains(w)) {
            out.push(p.display().to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up 16 by 16 sprite: a head, a body with a stripe, two
    /// legs apart, not symmetric.
    fn sprite() -> Tones {
        let rows = [
            "................",
            ".....1111.......",
            "....122221......",
            "....123321......",
            "....122221......",
            ".....1111.......",
            "...11222211.....",
            "..1222222221....",
            "..1233333321111.",
            "..1222222221....",
            "...12222221.....",
            "....1....1......",
            "...11....11.....",
            "..12......21....",
            "..11......11....",
            "................",
        ];
        let t = rows
            .iter()
            .flat_map(|r| r.bytes().map(|b| if b == b'.' { 0 } else { b - b'0' }))
            .collect();
        Tones { w: 16, h: 16, t }
    }

    #[test]
    fn shifted_mirrored_and_recolored_copies_are_refused() {
        let a = sprite();
        let recolor = |t: &Tones| Tones {
            t: t.t
                .iter()
                .map(|v| match v {
                    1 => 3,
                    3 => 2,
                    2 => 1,
                    x => *x,
                })
                .collect(),
            ..t.clone()
        };
        let cases = [
            ("copy", a.clone()),
            ("shifted 2, -1", a.moved(2, -1, false)),
            ("shifted -2, 2", a.moved(-2, 2, false)),
            ("mirrored", a.moved(0, 0, true)),
            ("mirrored and shifted", a.moved(1, 1, true)),
            ("recolored", recolor(&a)),
            (
                "recolored, mirrored, shifted",
                recolor(&a.moved(-1, 2, true)),
            ),
        ];
        for (what, t) in cases {
            let c = compare(std::slice::from_ref(&a), &[t]);
            let s = score(what.into(), &c);
            assert!(s.refused, "{what}: {c:?}");
        }
        // A copy with its inside repainted flat keeps its outline: shape.
        let flat = Tones {
            t: a.t.iter().map(|v| u8::from(*v != 0) * 2).collect(),
            ..a.clone()
        };
        let c = compare(std::slice::from_ref(&a), &[flat]);
        assert!(c.shape >= SHAPE_REFUSE, "{c:?}");
        // A different figure in the same box isn't refused: a round blob.
        let mut blob = Tones {
            w: 16,
            h: 16,
            t: vec![0; 256],
        };
        for y in 0..16i32 {
            for x in 0..16i32 {
                let (dx, dy) = (x - 8, y - 9);
                if dx * dx + dy * dy <= 30 {
                    blob.t[(y * 16 + x) as usize] = if dx * dx + dy * dy > 18 { 1 } else { 3 };
                }
            }
        }
        let c = compare(std::slice::from_ref(&a), &[blob]);
        assert!(!score("blob".into(), &c).refused, "{c:?}");
    }

    /// Every glyph of the theme's font reads as itself at 1x and 2x, so a
    /// name in it is never missed (a W read as U was, before W got its
    /// own bottom).
    #[test]
    fn the_theme_font_reads_as_itself() {
        let all = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        for scale in [1u32, 2] {
            let w = all.len() as u32 * 8 * scale;
            let h = 9 * scale;
            let mut p = Tones {
                w,
                h,
                t: vec![0; (w * h) as usize],
            };
            for (i, ch) in all.chars().enumerate() {
                let g = crate::theme_paper::glyph(ch).unwrap();
                for y in 0..7 * scale {
                    for x in 0..5 * scale {
                        if g[(y / scale) as usize].as_bytes()[(x / scale) as usize] == b'#' {
                            let px = i as u32 * 8 * scale + x;
                            p.t[((y + scale) * w + px) as usize] = 1;
                        }
                    }
                }
            }
            let read: String = read_text(&p, &own_glyphs()).split_whitespace().collect();
            assert_eq!(read, all, "at {scale}x");
        }
    }

    // game-printer's two Rivet Rodeo tests (an original's own title, and
    // the paper digits against Rivet Rodeo's tiles.txt) were cut: the
    // owner deleted Rivet Rodeo on 2026-11-11 and gameprinter has no
    // template of it.

    #[test]
    fn a_name_in_a_picture_is_read_and_refused() {
        // KONG and CLIMB in the theme's own font at 2x, on a 96 by 40
        // picture.
        let mut p = Tones {
            w: 120,
            h: 40,
            t: vec![0; 4800],
        };
        let mut put = |word: &str, x0: u32, y0: u32| {
            for (i, ch) in word.chars().enumerate() {
                let g = crate::theme_paper::glyph(ch).unwrap();
                for y in 0..14u32 {
                    for x in 0..10u32 {
                        if g[(y / 2) as usize].as_bytes()[(x / 2) as usize] != b'.' {
                            p.t[((y0 + y) * 120 + x0 + i as u32 * 12 + x) as usize] = 1;
                        }
                    }
                }
            }
        };
        put("KONG", 4, 4);
        put("CLIMB", 50, 22);
        let read = read_text(&p, &own_glyphs());
        assert!(read.contains("KONG") && read.contains("CLIMB"), "{read}");
        let s = text_score("t".into(), &read, &["kong".into(), "donkey".into()]);
        assert!(s.refused, "{s:?}");
        let s = text_score("t".into(), "RIVET RODEO", &["kong".into()]);
        assert!(!s.refused);
    }

    #[test]
    fn a_transposed_melody_is_refused() {
        let tune = [60, 62, 64, 65, 67, 65, 64, 62, 60];
        let up: Vec<u8> = tune.iter().map(|n| n + 5).collect();
        let s = melody_similarity(&steps(&tune), &steps(&up));
        assert_eq!(s, 1.0);
        assert!(s >= MELODY_REFUSE);
        let other = [60, 67, 60, 67, 72, 60, 64, 69, 71];
        assert!(melody_similarity(&steps(&other), &steps(&tune)) < MELODY_REFUSE);
    }

    #[test]
    fn apu_writes_become_notes() {
        // A4 on pulse 1: period 253 (1789773 / 16 / 254 = 440.4 Hz).
        let w = vec![vec![(0x02, 253), (0x03, 0)], vec![(0x02, 169), (0x03, 0)]];
        let m = apu_melodies(&w);
        assert_eq!(m[0], vec![69, 76]);
    }
}
