//! A level drawn by what each thing is, for `gp level render` and the
//! picture every `gp level edit` returns (docs/level-kit.md, "Editing a
//! stage"). It draws the model's compiled level: the ground's boxes as
//! girders, the ladders' boxes (full ones bright, ones that don't reach
//! the girder above dim), the ladder tops and stops, each thing where it
//! starts as a box by its role, and the goal's row. No picture from the
//! ROM goes in it, so it can be shown anywhere; the numbers drawn are the
//! pieces' rows, as `gp level show` and the edit operations name them.

use std::path::Path;

use anyhow::{Context, Result};
use gp_interp::stage::Layout;
use gp_sim::Spec;

/// Pixels a level pixel is drawn as.
pub(crate) const SCALE: u32 = 3;

pub(crate) const BACK: [u8; 3] = [16, 16, 24];
const GRID: [u8; 3] = [26, 26, 38];
pub(crate) const GIRDER: [u8; 3] = [208, 48, 80];
const GIRDER_EDGE: [u8; 3] = [255, 140, 160];
pub(crate) const LADDER: [u8; 3] = [64, 200, 224];
const LADDER_SHORT: [u8; 3] = [60, 100, 120];
const TOP: [u8; 3] = [240, 240, 240];
const STOP: [u8; 3] = [255, 64, 64];
const GOAL: [u8; 3] = [80, 220, 100];
const OUT: [u8; 3] = [90, 90, 90];
const PLAYER: [u8; 3] = [120, 160, 255];
const POWER: [u8; 3] = [250, 210, 60];
const ENEMY: [u8; 3] = [250, 140, 40];
const OTHER: [u8; 3] = [220, 120, 220];
const HAZARD: [u8; 3] = [255, 70, 40];

/// What one picture shows, beside the compiled level.
pub(crate) struct Marks<'a> {
    /// The ground's runs: each run's first piece's box and its row, so the
    /// row's number is drawn beside it.
    pub runs: &'a [(usize, [i64; 4])],
    /// Each ladder's box, whether it reaches the girder above, and its row.
    pub ladders: &'a [([i64; 4], bool)],
    /// Each spec kind's role in the model (`player`, `enemy`, ...).
    pub roles: &'a [String],
}

/// An RGB picture.
pub(crate) struct Canvas {
    pub w: u32,
    pub h: u32,
    pub px: Vec<[u8; 3]>,
    /// Girders drawn as outlines, over a picture of the game.
    pub outlines: bool,
}

impl Canvas {
    pub(crate) fn new(w: u32, h: u32, c: [u8; 3]) -> Self {
        Canvas {
            w,
            h,
            px: vec![c; (w * h) as usize],
            outlines: false,
        }
    }

    pub(crate) fn set(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            self.px[(y as u32 * self.w + x as u32) as usize] = c;
        }
    }

    /// A box in level pixels, filled, at `SCALE`.
    fn fill(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: [u8; 3]) {
        let s = i64::from(SCALE);
        for y in y0 * s..(y1 + 1) * s {
            for x in x0 * s..(x1 + 1) * s {
                self.set(x, y, c);
            }
        }
    }

    /// A box's outline in level pixels, one picture pixel wide.
    fn outline(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: [u8; 3]) {
        let s = i64::from(SCALE);
        let (a, b, cc, d) = (x0 * s, y0 * s, (x1 + 1) * s - 1, (y1 + 1) * s - 1);
        for x in a..=cc {
            self.set(x, b, c);
            self.set(x, d, c);
        }
        for y in b..=d {
            self.set(a, y, c);
            self.set(cc, y, c);
        }
    }

    /// A row across the picture, dashed.
    fn dashed_row(&mut self, y: i64, c: [u8; 3]) {
        let s = i64::from(SCALE);
        for x in 0..i64::from(self.w) {
            if (x / (4 * s)) % 2 == 0 {
                self.set(x, y * s, c);
            }
        }
    }

    /// A number in the 3 by 5 digit font, `SCALE` picture pixels a dot,
    /// its top left at level pixel (x, y).
    fn number(&mut self, n: usize, x: i64, y: i64, c: [u8; 3]) {
        let s = i64::from(SCALE);
        let text = n.to_string();
        for (k, ch) in text.bytes().enumerate() {
            let g = DIGITS[usize::from(ch - b'0')];
            for (row, bits) in g.iter().enumerate() {
                for col in 0..3 {
                    if bits >> (2 - col) & 1 == 1 {
                        let px = (x + k as i64 * 4 + col) * s;
                        let py = (y + row as i64) * s;
                        for dy in 0..s {
                            for dx in 0..s {
                                self.set(px + dx, py + dy, c);
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn write_png(&self, file: &Path) -> Result<()> {
        if let Some(d) = file.parent() {
            std::fs::create_dir_all(d).with_context(|| d.display().to_string())?;
        }
        let f = std::fs::File::create(file).with_context(|| file.display().to_string())?;
        let mut e = png::Encoder::new(std::io::BufWriter::new(f), self.w, self.h);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header()?;
        let bytes: Vec<u8> = self.px.iter().flatten().copied().collect();
        w.write_image_data(&bytes)?;
        Ok(())
    }
}

/// 3 by 5 digits, a row a byte, the left column the high bit of three.
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

fn role_color(r: &str) -> [u8; 3] {
    match r {
        "player" => PLAYER,
        "power-up" => POWER,
        "enemy" => ENEMY,
        "hazard" => HAZARD,
        _ => OTHER,
    }
}

/// Level `i` of `spec` drawn by kind, with `marks` numbering its pieces.
pub(crate) fn by_kind(spec: &Spec, i: usize, lay: &Layout, marks: &Marks<'_>) -> Canvas {
    let l = &spec.levels[i];
    let (w, h) = (
        (l.size[0] * l.tile_size).max(1) as u32,
        (l.size[1] * l.tile_size).max(1) as u32,
    );
    let mut c = Canvas::new(w * SCALE, h * SCALE, BACK);
    // A faint cell grid, so places can be read off the picture.
    let t = l.tile_size.max(1);
    for y in 0..i64::from(h) {
        for x in 0..i64::from(w) {
            if x % t == 0 || y % t == 0 {
                let s = i64::from(SCALE);
                c.set(x * s, y * s, GRID);
            }
        }
    }
    draw_layout(&mut c, spec, i, lay, marks);
    c
}

/// The level's pieces drawn over `c` (an empty canvas, or the model's own
/// picture for `--art`, where they are outlines).
pub(crate) fn draw_layout(c: &mut Canvas, spec: &Spec, i: usize, lay: &Layout, marks: &Marks<'_>) {
    let l = &spec.levels[i];
    let outline_only = c.outlines;
    // Girders: each one-row box with the 4 rows under it, as a girder
    // stands on its top row.
    for b in &l.ground {
        if outline_only {
            c.outline(b[0], b[1], b[2], b[1] + 3, GIRDER);
        } else {
            c.fill(b[0], b[1], b[2], b[1] + 3, GIRDER);
            c.fill(b[0], b[1], b[2], b[1], GIRDER_EDGE);
        }
    }
    // Ladders: two rails and a rung every 4 rows.
    for (k, (bx, full)) in marks.ladders.iter().enumerate() {
        let col = if *full { LADDER } else { LADDER_SHORT };
        c.fill(bx[0], bx[1], bx[0], bx[3], col);
        c.fill(bx[2] - 1, bx[1], bx[2] - 1, bx[3], col);
        let mut y = bx[1];
        while y <= bx[3] {
            if *full || (y / 4) % 2 == 0 {
                c.fill(bx[0], y, bx[2] - 1, y, col);
            }
            y += 4;
        }
        c.number(k, bx[0] + 1, bx[1] - 7, col);
    }
    for t in &l.ladder_tops {
        c.outline(t[0], t[1] - 1, t[0] + 7, t[1] + 1, TOP);
    }
    for s in &l.ladder_stops {
        for d in 0..6 {
            c.set(
                (s[0] + 1 + d) * i64::from(SCALE),
                (s[1] + d) * i64::from(SCALE),
                STOP,
            );
            c.set(
                (s[0] + 6 - d) * i64::from(SCALE),
                (s[1] + d) * i64::from(SCALE),
                STOP,
            );
        }
    }
    for (row, b) in marks.runs {
        c.number(*row, (b[0] + 2).clamp(0, 248), b[1] + 5, GIRDER_EDGE);
    }
    // Things where they start, by role.
    for s in &l.spawns {
        let k = &spec.kinds[s.kind];
        let (sw, sh) = (k.size[0].floor(), k.size[1].floor());
        if sw <= 1 || sh <= 1 {
            continue;
        }
        let (x, y) = (s.x.floor(), s.y.floor());
        let role = marks.roles.get(s.kind).map_or("", String::as_str);
        c.outline(x, y, x + sw - 1, y + sh - 1, role_color(role));
    }
    if let Some(g) = lay.goal_under {
        c.dashed_row(g, GOAL);
    }
    if let Some(k) = l.kill_below {
        c.dashed_row(k.floor(), OUT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_draw_inside_the_picture() {
        let mut c = Canvas::new(60, 30, BACK);
        c.number(1234567890, 0, 0, TOP);
        c.number(5, 300, 300, TOP);
        assert!(c.px.contains(&TOP));
    }

    #[test]
    fn a_box_is_drawn_at_the_scale() {
        let mut c = Canvas::new(30, 30, BACK);
        c.fill(1, 1, 1, 1, GIRDER);
        let at = |x: u32, y: u32| c.px[(y * 30 + x) as usize];
        assert_eq!(at(3, 3), GIRDER);
        assert_eq!(at(5, 5), GIRDER);
        assert_eq!(at(6, 6), BACK);
    }
}
