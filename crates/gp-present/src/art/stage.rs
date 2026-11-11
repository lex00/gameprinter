//! A large-art look's stages, built from pieces (a plank, a ladder rail, a
//! rope, a wheel, ...) over the build's own level picture: a look holds no
//! stage picture, so it holds no layout. The level picture is the one the
//! runtime has (lifted from the player's ROM, for a ROM build); its
//! floors, ladders, ropes, wheels and bonus items are found by color, and
//! the pieces are placed on them.
//!
//! The rules (in the look's `gp-art 1` file):
//!
//! - `stage SCALE R G B R G B`: stage pixels per game pixel, and the
//!   backdrop, a gradient from the first color at the top to the second.
//! - `stage_piece NAME ART W H`: a piece and its art's size in pixels
//!   (`plank` 16 by 8 is a floor square, `rail` a ladder's repeat, `rope`,
//!   `wheel`, `lump`, `rake`, `shovel`, and any piece `put` names).
//! - `stage_holes SUFFIX`: a changed cell whose picture's name ends with it
//!   (a taken rivet) is drawn as the backdrop, 3 pixels wider each side
//!   beside a ladder.
//! - `level FILE` starts a level's rules, for the level picture `FILE`:
//!   - `lit N`: a pixel brighter than this (its largest channel) can be
//!     stage (60 when not given);
//!   - `floor TEST...`, `ladder TEST...`: a pixel is floor (ladder) when
//!     every test holds; a test is `A>B+K` with `A` and `B` each one of
//!     `r g b min max 0`;
//!   - `items`: pink pictures (bonus items) are neither; a rake (12 pixels
//!     wide or more) or a shovel is drawn where each stands;
//!   - `ladder_clear N`: ladder pixels this close above or below a floor
//!     aren't ladder;
//!   - `snap_drum X FX LO HI LO2`: the floor at column `X` within `LO2` to
//!     `HI` takes the floor found at column `FX` within `LO` to `HI`;
//!   - `snap_walkway`: a floor square a few pixels above its neighbours'
//!     level is lowered to it;
//!   - `heads R G B`: wheels' color: a wheel is drawn on each, with floor
//!     squares under it, and the ropes run to their centers;
//!   - `rope R G B`: the cable's color;
//!   - `runs`: floors placed along each run of floor from its start;
//!   - `halves off`: no half planks;
//!   - `skip_ladder X0 X1 Y0`: ladders over `X0` to `X1` from `Y0` down
//!     aren't drawn;
//!   - `lump X Y`: an 8-pixel square drawn as backdrop with a lump on it;
//!   - `put PIECE X Y W H`: a piece at a box (game pixels, decimals allowed).
//!
//! [`build`] turns a level picture into a [`Stage`]: draws in stage pixels
//! (`SCALE` to a game pixel) over the backdrop.

use std::collections::BTreeSet;

/// A stage's rules.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StageRules {
    pub scale: i64,
    pub backdrop: [[u8; 3]; 2],
    /// Each piece's name, art file and size in its own pixels.
    pub pieces: Vec<(String, String, [i64; 2])>,
    pub holes: Option<String>,
    pub levels: Vec<LevelRules>,
}

/// A colour test: `a > b + k`, each side a channel, the least or most of
/// the three, or 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Test {
    a: Ch,
    b: Ch,
    k: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ch {
    R,
    G,
    B,
    Min,
    Max,
    Zero,
}

impl Ch {
    fn parse(s: &str) -> Option<Ch> {
        Some(match s {
            "r" => Ch::R,
            "g" => Ch::G,
            "b" => Ch::B,
            "min" => Ch::Min,
            "max" => Ch::Max,
            "0" => Ch::Zero,
            _ => return None,
        })
    }
    fn of(self, c: [u8; 3]) -> i64 {
        let [r, g, b] = c.map(i64::from);
        match self {
            Ch::R => r,
            Ch::G => g,
            Ch::B => b,
            Ch::Min => r.min(g).min(b),
            Ch::Max => r.max(g).max(b),
            Ch::Zero => 0,
        }
    }
}

impl Test {
    /// `A>B+K` (or `A>B-K`, or `A>B`).
    pub fn parse(s: &str) -> Option<Test> {
        let (a, rest) = s.split_once('>')?;
        let (b, k) = match rest.find(['+', '-']) {
            Some(i) => (&rest[..i], rest[i..].trim_start_matches('+').parse().ok()?),
            None => (rest, 0),
        };
        Some(Test {
            a: Ch::parse(a)?,
            b: Ch::parse(b)?,
            k,
        })
    }
    fn holds(&self, c: [u8; 3]) -> bool {
        self.a.of(c) > self.b.of(c) + self.k
    }
}

/// One level's rules.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LevelRules {
    pub level: String,
    pub lit: i64,
    pub floor: Vec<Test>,
    pub ladder: Vec<Test>,
    pub items: bool,
    pub ladder_clear: i64,
    pub snap_drum: Option<[i64; 5]>,
    pub snap_walkway: bool,
    pub heads: Option<[u8; 3]>,
    pub rope: Option<[u8; 3]>,
    pub runs: bool,
    pub halves: bool,
    pub skip_ladders: Vec<[i64; 3]>,
    pub lumps: Vec<[i64; 2]>,
    /// A piece's name and its box in thousandths of a game pixel.
    pub put: Vec<(String, [i64; 4])>,
}

/// A number with up to three decimals, in thousandths.
fn milli(s: &str) -> Option<i64> {
    let (w, f) = s.split_once('.').unwrap_or((s, ""));
    if f.len() > 3 || !f.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let neg = w.starts_with('-');
    let w: i64 = w.parse().ok()?;
    let f: i64 = if f.is_empty() {
        0
    } else {
        format!("{f:0<3}").parse().ok()?
    };
    Some(w * 1000 + if neg { -f } else { f })
}

fn rgb(r: &str, g: &str, b: &str) -> Option<[u8; 3]> {
    Some([r.parse().ok()?, g.parse().ok()?, b.parse().ok()?])
}

fn need(s: &mut Option<StageRules>) -> Result<&mut StageRules, ()> {
    s.as_mut().ok_or(())
}

impl StageRules {
    /// Reads one of the stage's lines; false when it isn't one.
    pub(crate) fn line(stage: &mut Option<StageRules>, w: &[&str]) -> Result<bool, ()> {
        let n = |s: &str| s.parse::<i64>().map_err(|_| ());
        match w {
            ["stage", s, r1, g1, b1, r2, g2, b2] => {
                *stage = Some(StageRules {
                    scale: n(s)?.clamp(1, 16),
                    backdrop: [rgb(r1, g1, b1).ok_or(())?, rgb(r2, g2, b2).ok_or(())?],
                    ..StageRules::default()
                });
            }
            ["stage_piece", name, art, pw, ph] => {
                if !super::inside(art) {
                    return Err(());
                }
                need(stage)?.pieces.push((
                    (*name).into(),
                    (*art).into(),
                    [n(pw)?.max(1), n(ph)?.max(1)],
                ));
            }
            ["stage_holes", suffix] => need(stage)?.holes = Some((*suffix).into()),
            ["level", file] => need(stage)?.levels.push(LevelRules {
                level: (*file).into(),
                lit: 60,
                halves: true,
                ..LevelRules::default()
            }),
            [k, rest @ ..] if LEVEL_KEYS.contains(k) => {
                let l = need(stage)?.levels.last_mut().ok_or(())?;
                match (*k, rest) {
                    ("lit", [v]) => l.lit = n(v)?,
                    ("floor", t) => {
                        l.floor = t
                            .iter()
                            .map(|x| Test::parse(x))
                            .collect::<Option<_>>()
                            .ok_or(())?
                    }
                    ("ladder", t) => {
                        l.ladder = t
                            .iter()
                            .map(|x| Test::parse(x))
                            .collect::<Option<_>>()
                            .ok_or(())?
                    }
                    ("items", []) => l.items = true,
                    ("ladder_clear", [v]) => l.ladder_clear = n(v)?,
                    ("snap_drum", [a, b, c, d, e]) => {
                        l.snap_drum = Some([n(a)?, n(b)?, n(c)?, n(d)?, n(e)?])
                    }
                    ("snap_walkway", []) => l.snap_walkway = true,
                    ("heads", [r, g, b]) => l.heads = Some(rgb(r, g, b).ok_or(())?),
                    ("rope", [r, g, b]) => l.rope = Some(rgb(r, g, b).ok_or(())?),
                    ("runs", []) => l.runs = true,
                    ("halves", ["off"]) => l.halves = false,
                    ("skip_ladder", [a, b, c]) => l.skip_ladders.push([n(a)?, n(b)?, n(c)?]),
                    ("lump", [x, y]) => l.lumps.push([n(x)?, n(y)?]),
                    ("put", [p, x, y, ww, h]) => l.put.push((
                        (*p).into(),
                        [
                            milli(x).ok_or(())?,
                            milli(y).ok_or(())?,
                            milli(ww).ok_or(())?,
                            milli(h).ok_or(())?,
                        ],
                    )),
                    _ => return Err(()),
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

const LEVEL_KEYS: [&str; 14] = [
    "lit",
    "floor",
    "ladder",
    "items",
    "ladder_clear",
    "snap_drum",
    "snap_walkway",
    "heads",
    "rope",
    "runs",
    "halves",
    "skip_ladder",
    "lump",
    "put",
];

/// What a stage draw paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageSrc {
    /// A piece, by its number in [`StageRules::pieces`].
    Piece(u32),
    /// The backdrop's gradient, over the box.
    Backdrop,
}

/// One draw of a stage, in stage pixels. `part` is the piece's part drawn,
/// `[x, w]` in thousandths of its width (all of its height).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageDraw {
    pub src: StageSrc,
    pub part: [i64; 2],
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub flip_x: bool,
    /// Brightness in thousandths.
    pub shade: i64,
    pub clip: Option<[i64; 4]>,
}

impl StageDraw {
    fn piece(p: u32, x: i64, y: i64, w: i64, h: i64) -> Self {
        StageDraw {
            src: StageSrc::Piece(p),
            part: [0, 1000],
            x,
            y,
            w,
            h,
            flip_x: false,
            shade: 1000,
            clip: None,
        }
    }
}

/// A built stage: its size in game pixels, its scale and backdrop, the
/// draws over the backdrop, and the ladders' boxes `[x0, x1, y0, y1]` in
/// game pixels (for the holes beside them).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stage {
    pub size: [i64; 2],
    pub scale: i64,
    pub backdrop: [[u8; 3]; 2],
    pub draws: Vec<StageDraw>,
    pub ladders: Vec<[i64; 4]>,
}

type Grid = Vec<Vec<bool>>;

/// Builds the stage `l` over the level picture `px` (`w` by `h`, RGBA row
/// by row).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build(r: &StageRules, l: &LevelRules, w: usize, h: usize, px: &[[u8; 4]]) -> Stage {
    let (wi, hi) = (w as i64, h as i64);
    let s = r.scale;
    let at = |x: usize, y: usize| -> [u8; 3] {
        let c = px[y * w + x];
        [c[0], c[1], c[2]]
    };
    let piece = |name: &str| {
        r.pieces
            .iter()
            .position(|p| p.0 == name)
            .map(|i| (i as u32, r.pieces[i].2))
    };
    let grid = || vec![vec![false; w]; h];
    let (mut p, mut lm, mut pink) = (grid(), grid(), grid());
    for y in 0..h {
        for x in 0..w {
            let c = at(x, y);
            if i64::from(c[0].max(c[1]).max(c[2])) <= l.lit {
                continue;
            }
            p[y][x] = l.floor.iter().all(|t| t.holds(c));
            lm[y][x] = l.ladder.iter().all(|t| t.holds(c));
            if c[0] > 200 && c[2] > 200 && c[1] < 150 {
                pink[y][x] = true;
            }
        }
    }
    // Bonus items: kept out of both, a tool drawn where each stands.
    let items = if l.items {
        item_boxes(&pink, w, h)
    } else {
        Vec::new()
    };
    for &[x0, y0, x1, y1] in &items {
        for y in y0..y1 {
            for x in x0..x1 {
                p[y as usize][x as usize] = false;
                lm[y as usize][x as usize] = false;
            }
        }
    }
    // A floor's one-to-three-row gaps closed.
    let wrap = |y: i64| y.rem_euclid(hi) as usize;
    for dy in 1..=3 {
        let mut q = p.clone();
        for y in 0..hi {
            for x in 0..w {
                if p[wrap(y - dy)][x] && p[wrap(y - dy + 4)][x] {
                    q[y as usize][x] = true;
                }
            }
        }
        p = q;
    }
    if l.ladder_clear > 0 {
        let k = l.ladder_clear;
        for y in 0..hi {
            for x in 0..w {
                if (-k..=k).any(|d| p[wrap(y + d)][x]) {
                    lm[y as usize][x] = false;
                }
            }
        }
    }
    // Floor squares: 16 pixels across, where the floor covers the middle.
    let mut tops: Vec<[i64; 2]> = Vec::new();
    for x in (0..w).step_by(16) {
        let cx = (w - 1).min(x + 8);
        let mut y = 0;
        while y < h {
            if !p[y][cx] {
                y += 1;
                continue;
            }
            let y0 = y;
            while y < h && p[y][cx] {
                y += 1;
            }
            if y - y0 >= 3 {
                tops.push([x as i64, y0 as i64]);
            }
        }
    }
    let has = |tops: &[[i64; 2]], x: i64, y: i64| tops.contains(&[x, y]);
    if let Some([sx, fx, lo, hi2, lo2]) = l.snap_drum {
        let f: Vec<i64> = tops
            .iter()
            .filter(|t| t[0] == fx && t[1] >= lo && t[1] <= hi2)
            .map(|t| t[1])
            .collect();
        if let Some(&nf) = f.iter().min() {
            let moved = tops
                .iter()
                .map(|&[x, y]| {
                    [
                        x,
                        if x == sx && y >= lo2 && y <= hi2 {
                            nf
                        } else {
                            y
                        },
                    ]
                })
                .collect();
            tops = dedupe(moved);
        }
    }
    if l.runs {
        let ys: BTreeSet<i64> = tops.iter().map(|t| t[1]).collect();
        let mut by = Vec::new();
        for y in ys {
            let xs: Vec<i64> = (0..wi).filter(|&x| p[y as usize][x as usize]).collect();
            let mut start = 0;
            for k in 1..=xs.len() {
                if k == xs.len() || xs[k] - xs[k - 1] > 8 {
                    let run = &xs[start..k];
                    if run.len() >= 16 {
                        let mut x = run[0];
                        while x < run[run.len() - 1] - 14 {
                            by.push([x, y]);
                            x += 16;
                        }
                    }
                    start = k;
                }
            }
        }
        tops = by;
    }
    if l.snap_walkway {
        let old = tops.clone();
        let moved = old
            .iter()
            .map(|&[x, y]| {
                for y2 in (y + 1)..(y + 9).min(hi) {
                    if (has(&old, x - 16, y2) || has(&old, x + 16, y2))
                        && p[y2 as usize][(wi - 1).min(x + 8) as usize]
                    {
                        return [x, y2];
                    }
                }
                [x, y]
            })
            .collect();
        tops = dedupe(moved);
    }
    let mut heads: Vec<[i64; 2]> = Vec::new();
    if let Some(hc) = l.heads {
        let mut seen = BTreeSet::new();
        for y in 0..h {
            for x in 0..w {
                if at(x, y) == hc {
                    let key = [(x as i64 >> 4) << 4, (y as i64 >> 4) << 4];
                    if seen.insert(key) {
                        heads.push(key);
                    }
                }
            }
        }
        heads.sort_unstable();
        let ys: BTreeSet<i64> = tops.iter().map(|t| t[1]).collect();
        for &[hx, hy] in &heads {
            for &y in &ys {
                if y >= hy - 8 && y <= hy + 16 && has(&tops, hx - 16, y) && has(&tops, hx + 16, y) {
                    tops.push([hx, y]);
                    break;
                }
            }
        }
    }

    let mut out = Stage {
        size: [wi, hi],
        scale: s,
        backdrop: r.backdrop,
        draws: Vec::new(),
        ladders: Vec::new(),
    };
    // Ladders, under the floors.
    if let Some((rail, _)) = piece("rail") {
        for [x0, y0, x1, y1] in boxes_of(&closing(&lm, w, h), w, h, 1) {
            if y1 - y0 < 5 {
                continue;
            }
            if l.skip_ladders
                .iter()
                .any(|&[a, b, c]| x0 < b && x1 > a && y0 >= c)
            {
                continue;
            }
            out.ladders.push([x0, x1, y0, y1]);
            let cx = (x0 + x1) >> 1;
            let mut floors: Vec<i64> = tops
                .iter()
                .filter(|t| t[0] <= cx && cx < t[0] + 16)
                .map(|t| t[1])
                .collect();
            floors.sort_unstable();
            let below: Vec<i64> = floors
                .iter()
                .copied()
                .filter(|&t| t >= y1 - 6 && t - y1 <= 8)
                .collect();
            let above: Vec<i64> = floors
                .iter()
                .copied()
                .filter(|&t| t + 8 <= y0 + 6 && y0 - (t + 8) <= 8)
                .collect();
            let bot = below.first().map_or(y1, |b| b + 2);
            let tp = above.last().map_or(y0, |a| a + 6);
            let (lw, step, lh) = ((x1 - x0 + 4) * s, 5 * s, (bot - tp) * s);
            let clip = [(x0 - 2) * s, tp * s, lw, lh];
            let mut yy = lh - step;
            while yy > -step {
                let mut d = StageDraw::piece(rail, (x0 - 2) * s, tp * s + yy, lw, step);
                d.clip = Some(clip);
                out.draws.push(d);
                yy -= step;
            }
        }
    }
    // Ropes, from wheel center to wheel center.
    if let (Some(rc), Some((rope, [pw, ph]))) = (l.rope, piece("rope")) {
        let rw = 4 * s;
        let rh = 8.max((ph * rw + pw / 2) / pw);
        for x in 0..w {
            let ys: Vec<i64> = (0..h)
                .filter(|&y| at(x, y) == rc)
                .map(|y| y as i64)
                .collect();
            if ys.len() <= 20 {
                continue;
            }
            let x = x as i64;
            let ctrs: Vec<i64> = heads
                .iter()
                .filter(|&&[hx, _]| hx <= x && x < hx + 16)
                .map(|&[_, hy]| hy + 8)
                .collect();
            let (lo, hi3) = (ys[0], ys[ys.len() - 1]);
            let y_t = ctrs
                .iter()
                .copied()
                .filter(|&c| c <= lo)
                .max()
                .unwrap_or(lo);
            let y_e = ctrs
                .iter()
                .copied()
                .filter(|&c| c >= hi3)
                .min()
                .unwrap_or(hi3 + 1);
            let rx = (x - 2) * s + s / 2;
            let clip = [rx, y_t * s, rw, (y_e - y_t) * s];
            let mut y = y_t * s;
            while y < y_e * s {
                let mut d = StageDraw::piece(rope, rx, y, rw, rh);
                d.clip = Some(clip);
                out.draws.push(d);
                y += rh;
            }
        }
    }
    // Floors: a plank a square, turned and shaded by place so they don't
    // repeat; half a plank where the real floor covers only half the square.
    if let Some((plank, _)) = piece("plank") {
        let half = |x: i64, y: i64| {
            let cols = (x..(x + 8).min(wi))
                .filter(|&xx| {
                    ((y - 3).max(0)..(y + 5).min(hi)).any(|yy| p[yy as usize][xx as usize])
                })
                .count();
            x + 8 <= wi && cols >= 6
        };
        let put_plank = |out: &mut Stage, x: i64, y: i64, part: Option<bool>| {
            let v = ((x >> 4) * 7 + y * 3 + (x >> 4) * (x >> 4)).rem_euclid(6);
            let flip = v % 2 == 1;
            let shade = [1000, 1060, 930, 1000, 1040, 960][v as usize];
            // part: Some(true) the left half, Some(false) the right.
            let (sx, dx, dw) = match part {
                Some(true) => (0, x, 8),
                Some(false) => (500, x + 8, 8),
                None => (0, x, 16),
            };
            let sw = if part.is_some() { 500 } else { 1000 };
            // A turned half takes the other half of the picture.
            let sx = if flip && part.is_some() {
                1000 - sx - sw
            } else {
                sx
            };
            let mut d = StageDraw::piece(plank, dx * s, y * s, dw * s, 8 * s);
            d.part = [sx, sw];
            d.flip_x = flip;
            d.shade = shade;
            out.draws.push(d);
        };
        for &[x, y] in &tops {
            let (lh, rh) = (half(x, y), half(x + 8, y));
            if !l.runs && lh != rh {
                put_plank(&mut out, x, y, Some(lh));
            } else {
                put_plank(&mut out, x, y, None);
            }
        }
        if l.halves && !l.runs {
            for x8 in (0..wi).step_by(8) {
                let cx = (wi - 1).min(x8 + 4) as usize;
                let mut y = 0;
                while y < h {
                    if !p[y][cx] {
                        y += 1;
                        continue;
                    }
                    let y0 = y as i64;
                    while y < h && p[y][cx] {
                        y += 1;
                    }
                    let near = |t: i64| (t - y0).abs() <= 3;
                    if (y as i64) - y0 < 3
                        || tops
                            .iter()
                            .any(|&[pp, t]| pp <= x8 && x8 < pp + 16 && near(t))
                    {
                        continue;
                    }
                    let left = tops.iter().any(|&[pp, t]| pp + 16 == x8 && near(t));
                    let right = tops.iter().any(|&[pp, t]| pp == x8 + 8 && near(t));
                    if !left && !right {
                        continue;
                    }
                    // The plank's right half after a floor's end, its left half before one.
                    let mut d = StageDraw::piece(plank, x8 * s, y0 * s, 8 * s, 8 * s);
                    d.part = if left { [500, 500] } else { [0, 500] };
                    out.draws.push(d);
                }
            }
        }
    }
    if let Some((wheel, _)) = piece("wheel") {
        for &[hx, hy] in &heads {
            out.draws.push(StageDraw::piece(
                wheel,
                (hx + 2) * s,
                (hy + 2) * s,
                12 * s,
                12 * s,
            ));
        }
    }
    let lump = piece("lump");
    for &[x, y] in &l.lumps {
        out.draws.push(StageDraw {
            src: StageSrc::Backdrop,
            ..StageDraw::piece(0, x * s, y * s, 8 * s, 8 * s)
        });
        if let Some((lp, _)) = lump {
            out.draws
                .push(StageDraw::piece(lp, x * s, y * s, 8 * s, 8 * s));
        }
    }
    for &[x0, _, x1, y1] in &items {
        let (kind, tall) = if x1 - x0 >= 12 {
            ("rake", 16)
        } else {
            ("shovel", 15)
        };
        let Some((img, [pw, ph])) = piece(kind) else {
            continue;
        };
        let th = tall * s;
        let tw = (pw * th + ph / 2) / ph;
        out.draws.push(StageDraw::piece(
            img,
            (x0 + x1) * s / 2 - tw / 2,
            y1 * s - th + s,
            tw,
            th,
        ));
    }
    for (name, [x, y, pw, ph]) in &l.put {
        if let Some((pc, _)) = piece(name) {
            let c = |v: i64| (v * s + 500).div_euclid(1000);
            out.draws
                .push(StageDraw::piece(pc, c(*x), c(*y), c(*pw), c(*ph)));
        }
    }
    out
}

fn dedupe(list: Vec<[i64; 2]>) -> Vec<[i64; 2]> {
    let set: BTreeSet<[i64; 2]> = list.into_iter().collect();
    set.into_iter().collect()
}

fn dilate(m: &Grid, w: usize, h: usize) -> Grid {
    let mut o = m.clone();
    for y in 0..h {
        for x in 0..w {
            if o[y][x] {
                continue;
            }
            'near: for dy in [h - 1, 0, 1] {
                for dx in [w - 1, 0, 1] {
                    if m[(y + dy) % h][(x + dx) % w] {
                        o[y][x] = true;
                        break 'near;
                    }
                }
            }
        }
    }
    o
}

fn closing(m: &Grid, w: usize, h: usize) -> Grid {
    let d = dilate(m, w, h);
    let inv: Grid = d.iter().map(|r| r.iter().map(|v| !v).collect()).collect();
    dilate(&inv, w, h)
        .iter()
        .map(|r| r.iter().map(|v| !v).collect())
        .collect()
}

/// Bounding boxes `[x0, y0, x1, y1)` of the mask's 8-way regions, pixels
/// up to `reach` apart counted as touching.
fn boxes_of(m: &Grid, w: usize, h: usize, reach: i64) -> Vec<[i64; 4]> {
    let mut seen = vec![vec![false; w]; h];
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if !m[y][x] || seen[y][x] {
                continue;
            }
            let mut b = [x as i64, y as i64, x as i64 + 1, y as i64 + 1];
            let mut stack = vec![(x as i64, y as i64)];
            seen[y][x] = true;
            while let Some((cx, cy)) = stack.pop() {
                b = [
                    b[0].min(cx),
                    b[1].min(cy),
                    b[2].max(cx + 1),
                    b[3].max(cy + 1),
                ];
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        let (nx, ny) = (cx + dx, cy + dy);
                        if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                            continue;
                        }
                        let (ux, uy) = (nx as usize, ny as usize);
                        if seen[uy][ux] || !m[uy][ux] {
                            continue;
                        }
                        seen[uy][ux] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            out.push(b);
        }
    }
    out
}

/// Each bonus item's box: from its first pink pixel (top left first), the
/// pink within 2 pixels above to 16 below and 10 left to 16 right of it;
/// then nothing within 2 pixels of that box starts another.
fn item_boxes(m: &Grid, w: usize, h: usize) -> Vec<[i64; 4]> {
    let (wi, hi) = (w as i64, h as i64);
    let mut seen = vec![vec![false; w]; h];
    let mut out = Vec::new();
    for y in 0..hi {
        for x in 0..wi {
            if !m[y as usize][x as usize] || seen[y as usize][x as usize] {
                continue;
            }
            let mut b = [wi, hi, 0, 0];
            for yy in (y - 2).max(0)..(y + 16).min(hi) {
                for xx in (x - 10).max(0)..(x + 16).min(wi) {
                    if m[yy as usize][xx as usize] {
                        b = [
                            b[0].min(xx),
                            b[1].min(yy),
                            b[2].max(xx + 1),
                            b[3].max(yy + 1),
                        ];
                    }
                }
            }
            for yy in (b[1] - 2).max(0)..(b[3] + 2).min(hi) {
                for xx in (b[0] - 2).max(0)..(b[2] + 2).min(wi) {
                    seen[yy as usize][xx as usize] = true;
                }
            }
            out.push(b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_and_numbers_read() {
        assert!(Test::parse("r>g+50").unwrap().holds([200, 100, 0]));
        assert!(!Test::parse("r>g+50").unwrap().holds([140, 100, 0]));
        assert!(Test::parse("min>0+150").unwrap().holds([160, 170, 180]));
        assert!(Test::parse("b>g").unwrap().holds([0, 1, 2]));
        assert!(Test::parse("x>g").is_none());
        assert_eq!(milli("34.5"), Some(34500));
        assert_eq!(milli("96.833"), Some(96833));
        assert_eq!(milli("16"), Some(16000));
        assert_eq!(milli("1.2345"), None);
    }

    /// A 64 by 32 picture: a floor row of 4 pixels at y 20 across x 0 to
    /// 40, a ladder at x 24 to 28 from y 8 to 20.
    fn picture() -> (usize, usize, Vec<[u8; 4]>) {
        let (w, h) = (64, 32);
        let mut px = vec![[0, 0, 0, 255]; w * h];
        for y in 20..24 {
            for x in 0..40 {
                px[y * w + x] = [220, 40, 90, 255];
            }
        }
        for y in 8..20 {
            for x in 24..28 {
                px[y * w + x] = [60, 200, 220, 255];
            }
        }
        (w, h, px)
    }

    fn rules() -> (StageRules, LevelRules) {
        let mut st = None;
        for l in [
            "stage 6 236 220 190 214 190 150",
            "stage_piece plank pieces/plank.png 96 48",
            "stage_piece rail pieces/rail.png 48 30",
            "level level.png",
            "floor r>g+50 b>g+0",
            "ladder g>r+40 b>r+40",
        ] {
            let w: Vec<&str> = l.split_whitespace().collect();
            assert!(StageRules::line(&mut st, &w).unwrap(), "{l}");
        }
        let st = st.unwrap();
        let l = st.levels[0].clone();
        (st, l)
    }

    #[test]
    fn planks_go_on_the_floor_and_never_past_its_end() {
        let (st, l) = rules();
        let (w, h, px) = picture();
        let s = build(&st, &l, w, h, &px);
        let planks: Vec<&StageDraw> = s
            .draws
            .iter()
            .filter(|d| d.src == StageSrc::Piece(0))
            .collect();
        assert!(!planks.is_empty());
        for d in &planks {
            assert_eq!(d.y, 20 * 6, "{d:?}");
            assert!(d.x + d.w <= 40 * 6, "a plank past the floor's end: {d:?}");
        }
        // The floor's last 8 pixels (32 to 40) get a half plank.
        assert!(planks.iter().any(|d| d.x == 32 * 6 && d.w == 8 * 6));
    }

    #[test]
    fn a_ladder_is_drawn_down_to_its_floor() {
        let (st, l) = rules();
        let (w, h, px) = picture();
        let s = build(&st, &l, w, h, &px);
        assert_eq!(s.ladders, vec![[24, 28, 8, 20]]);
        let rails: Vec<&StageDraw> = s
            .draws
            .iter()
            .filter(|d| d.src == StageSrc::Piece(1))
            .collect();
        // Clipped to the ladder, from its top to 2 pixels into the floor.
        assert_eq!(rails[0].clip, Some([22 * 6, 8 * 6, 8 * 6, 14 * 6]));
        assert!(rails.len() >= 3);
    }
}
