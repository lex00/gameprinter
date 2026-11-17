//! A large-art look's rooms: an arcade game's screens (its board's tile
//! layer) built from the look's pieces, laid on the room's own layout as
//! the player's lift gives it, as [`super::stage`] builds an NES stage over
//! its level picture. A look holds no picture of a room and no place in
//! one: the runtime hands over the screen as a grid of cells (each one's
//! tile code, colour bank, priority and the colours it draws, from the
//! player's lift), the look's rules sort the cells into classes by those,
//! and the pieces are laid on the classes' cells.
//!
//! The rules are `#+` lines (an older reader takes them for notes and draws
//! the board's own screen), after the `stage` line, whose scale and
//! backdrop they use, and with their pieces given by `stage_piece` lines:
//!
//! - `#+ room SCREEN...`: the rules after it, up to the next `#+ room`, are
//!   those screens' (each a `board-screen` slot, as a `pic` line names it).
//! - `#+ room_cells CLASS TEST...`: a class of cells, those for which every
//!   test holds: `tile=N` or `tile=LO-HI` (the cell's tile code, decimal or
//!   `0x` hex; several are any one), `bank=N` and `prio=N` (its colour bank
//!   and priority, 0 to 3; several are any one), `!CLASS` (not in a class
//!   given before), and colour tests as a stage's `floor` takes them
//!   (`r>g+40`), which hold for a cell when at least `share=P` percent
//!   (50 when not given) of its pixels pass all of them.
//! - `#+ room_fill CLASS PIECE`: the piece repeated over the class's cells
//!   at its own size (`stage_piece`'s `W H`, in stage pixels) from the
//!   screen's top left, cut to the cells.
//! - `#+ room_run CLASS LEFT MID RIGHT`: on each group of the class's cells
//!   (cells side by side or one above the other), over its box, `LEFT` at
//!   its left end, `RIGHT` at its right end and `MID` repeated between,
//!   each as tall as the box and as wide as its art's shape gives; `-` for
//!   none. A counter, a bar, a shelf.
//! - `#+ room_box CLASS PIECE`: the piece over each group's box. A door, a
//!   tap, a sign.
//!
//! The pieces are drawn in the order of their lines, the last on top, over
//! the stage's backdrop.

use super::stage::{Stage, StageDraw, StageRules, Test};

/// A screen's cells as the runtime gives them: `cols` by `rows`, each
/// `size` of the screen's pixels square, row by row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cells {
    pub cols: usize,
    pub rows: usize,
    pub size: i64,
    pub cells: Vec<Cell>,
}

/// One cell of a screen: its tile code, colour bank and priority, and the
/// colours of its pixels (any number of them, the same for every cell).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    pub tile: u16,
    pub bank: u8,
    pub prio: u8,
    pub colors: Vec<[u8; 3]>,
}

/// A class of cells (`#+ room_cells`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CellClass {
    pub name: String,
    pub tiles: Vec<[u16; 2]>,
    pub banks: Vec<u8>,
    pub prios: Vec<u8>,
    pub colors: Vec<Test>,
    /// Percent of a cell's pixels that pass `colors`.
    pub share: u32,
    pub not: Vec<String>,
}

/// How a rule lays its pieces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lay {
    Fill(String),
    /// Left, middle, right.
    Run([Option<String>; 3]),
    Box(String),
}

/// A rule laying pieces on a class's cells, with its line in the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub class: String,
    pub lay: Lay,
    pub line: usize,
}

impl Place {
    /// The pieces it names.
    #[must_use]
    pub fn pieces(&self) -> Vec<&str> {
        match &self.lay {
            Lay::Fill(p) | Lay::Box(p) => vec![p],
            Lay::Run(r) => r.iter().flatten().map(String::as_str).collect(),
        }
    }
}

/// One `#+ room` line's rules.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomRules {
    pub screens: Vec<String>,
    pub classes: Vec<CellClass>,
    pub places: Vec<Place>,
}

fn number(s: &str) -> Option<u16> {
    match s.strip_prefix("0x") {
        Some(h) => u16::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

fn small(s: &str) -> Result<u8, ()> {
    s.parse::<u8>().ok().filter(|v| *v < 4).ok_or(())
}

impl RoomRules {
    /// Reads a `#+ room...` line's words into `rooms`; false when they
    /// aren't one. A room needs the stage (its scale and backdrop).
    pub(crate) fn line(
        rooms: &mut Vec<RoomRules>,
        stage: Option<&StageRules>,
        w: &[&str],
        line: usize,
    ) -> Result<bool, ()> {
        let name_ok = |n: &str| super::region_name(n);
        match w {
            ["room", screens @ ..] => {
                if stage.is_none() || screens.is_empty() {
                    return Err(());
                }
                rooms.push(RoomRules {
                    screens: screens.iter().map(|s| (*s).to_owned()).collect(),
                    ..RoomRules::default()
                });
            }
            ["room_cells", name, tests @ ..] => {
                let room = rooms.last_mut().ok_or(())?;
                if !name_ok(name) || room.classes.iter().any(|c| c.name == *name) {
                    return Err(());
                }
                let mut c = CellClass {
                    name: (*name).to_owned(),
                    share: 50,
                    ..CellClass::default()
                };
                for t in tests {
                    if let Some(n) = t.strip_prefix('!') {
                        if !room.classes.iter().any(|c| c.name == n) {
                            return Err(());
                        }
                        c.not.push(n.to_owned());
                        continue;
                    }
                    match t.split_once('=') {
                        Some(("tile", v)) => {
                            let (lo, hi) = v.split_once('-').unwrap_or((v, v));
                            let (lo, hi) = (number(lo).ok_or(())?, number(hi).ok_or(())?);
                            if lo > hi || hi > 0x3ff {
                                return Err(());
                            }
                            c.tiles.push([lo, hi]);
                        }
                        Some(("bank", v)) => c.banks.push(small(v)?),
                        Some(("prio", v)) => c.prios.push(small(v)?),
                        Some(("share", v)) => {
                            c.share = v.parse().ok().filter(|p| (1..=100).contains(p)).ok_or(())?;
                        }
                        Some(_) => return Err(()),
                        None => c.colors.push(Test::parse(t).ok_or(())?),
                    }
                }
                room.classes.push(c);
            }
            [
                k @ ("room_fill" | "room_run" | "room_box"),
                class,
                pieces @ ..,
            ] => {
                let room = rooms.last_mut().ok_or(())?;
                if !room.classes.iter().any(|c| c.name == *class) {
                    return Err(());
                }
                let piece = |p: &str| (p != "-").then(|| p.to_owned());
                let lay = match (*k, pieces) {
                    ("room_fill", [p]) if *p != "-" => Lay::Fill((*p).to_owned()),
                    ("room_box", [p]) if *p != "-" => Lay::Box((*p).to_owned()),
                    ("room_run", [l, m, r]) => {
                        let run = [piece(l), piece(m), piece(r)];
                        if run.iter().all(Option::is_none) {
                            return Err(());
                        }
                        Lay::Run(run)
                    }
                    _ => return Err(()),
                };
                room.places.push(Place {
                    class: (*class).to_owned(),
                    lay,
                    line,
                });
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

impl CellClass {
    fn holds(&self, c: &Cell) -> bool {
        if !self.tiles.is_empty() && !self.tiles.iter().any(|[a, b]| (*a..=*b).contains(&c.tile)) {
            return false;
        }
        if !self.banks.is_empty() && !self.banks.contains(&c.bank) {
            return false;
        }
        if !self.prios.is_empty() && !self.prios.contains(&c.prio) {
            return false;
        }
        if self.colors.is_empty() {
            return true;
        }
        let pass = c
            .colors
            .iter()
            .filter(|px| self.colors.iter().all(|t| t.holds(**px)))
            .count();
        !c.colors.is_empty() && pass * 100 >= self.share as usize * c.colors.len()
    }
}

/// A built room: the stage it draws (pieces by their number in the stage's
/// `pieces`), and for each of the room's rules, by its place in
/// [`RoomRules::places`], the cells its class holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Room {
    pub stage: Stage,
    pub cells: Vec<usize>,
}

/// Each class's cells, by class, as a grid of `cols` by `rows`.
fn classify(r: &RoomRules, cells: &Cells) -> Vec<Vec<bool>> {
    let mut out: Vec<Vec<bool>> = Vec::with_capacity(r.classes.len());
    for c in &r.classes {
        let not: Vec<usize> = c
            .not
            .iter()
            .filter_map(|n| r.classes.iter().position(|x| x.name == *n))
            .collect();
        let grid = cells
            .cells
            .iter()
            .enumerate()
            .map(|(i, cell)| c.holds(cell) && !not.iter().any(|&k| out[k][i]))
            .collect();
        out.push(grid);
    }
    out
}

/// The boxes `[c0, r0, c1, r1)` of the grid's groups (cells side by side or
/// one above the other), in the order their first cell is met row by row.
fn groups(grid: &[bool], cols: usize, rows: usize) -> Vec<[usize; 4]> {
    let mut seen = vec![false; grid.len()];
    let mut out = Vec::new();
    for start in 0..grid.len().min(cols * rows) {
        if !grid[start] || seen[start] {
            continue;
        }
        let mut b = [
            start % cols,
            start / cols,
            start % cols + 1,
            start / cols + 1,
        ];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % cols, i / cols);
            b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            let mut near = Vec::with_capacity(4);
            if x > 0 {
                near.push(i - 1);
            }
            if x + 1 < cols {
                near.push(i + 1);
            }
            if y > 0 {
                near.push(i - cols);
            }
            if y + 1 < rows {
                near.push(i + cols);
            }
            for n in near {
                if grid[n] && !seen[n] {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        out.push(b);
    }
    out
}

/// The rectangles `[c0, r0, c1, r1)` that cover the grid's cells: each
/// row's runs, a run the same as the one above joined to it.
fn rects(grid: &[bool], cols: usize, rows: usize) -> Vec<[usize; 4]> {
    let mut out: Vec<[usize; 4]> = Vec::new();
    // The rectangles still open at the row above, by their run.
    let mut open: Vec<usize> = Vec::new();
    for y in 0..rows {
        let mut now = Vec::new();
        let mut x = 0;
        while x < cols {
            if !grid[y * cols + x] {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < cols && grid[y * cols + x] {
                x += 1;
            }
            match open.iter().find(|&&k| out[k][0] == x0 && out[k][2] == x) {
                Some(&k) => {
                    out[k][3] = y + 1;
                    now.push(k);
                }
                None => {
                    out.push([x0, y, x, y + 1]);
                    now.push(out.len() - 1);
                }
            }
        }
        open = now;
    }
    out
}

/// Builds the room `r` over the screen's `cells` with the stage's scale,
/// backdrop and pieces.
#[must_use]
pub fn build(st: &StageRules, r: &RoomRules, cells: &Cells) -> Room {
    let s = st.scale;
    let (cols, rows) = (cells.cols, cells.rows);
    let side = cells.size * s;
    let mut out = Stage {
        size: [cols as i64 * cells.size, rows as i64 * cells.size],
        scale: s,
        backdrop: st.backdrop,
        draws: Vec::new(),
        ladders: Vec::new(),
    };
    let piece = |name: &str| {
        st.pieces
            .iter()
            .position(|p| p.0 == name)
            .map(|i| (i as u32, st.pieces[i].2))
    };
    let classes = classify(r, cells);
    let mut counts = Vec::with_capacity(r.places.len());
    for place in &r.places {
        let Some(k) = r.classes.iter().position(|c| c.name == place.class) else {
            counts.push(0);
            continue;
        };
        let grid = &classes[k];
        counts.push(grid.iter().filter(|c| **c).count());
        let px = |b: [usize; 4]| {
            [
                b[0] as i64 * side,
                b[1] as i64 * side,
                b[2] as i64 * side,
                b[3] as i64 * side,
            ]
        };
        match &place.lay {
            Lay::Fill(p) => {
                let Some((p, [tw, th])) = piece(p) else {
                    continue;
                };
                for b in rects(grid, cols, rows) {
                    let [x0, y0, x1, y1] = px(b);
                    let clip = [x0, y0, x1 - x0, y1 - y0];
                    let mut ty = y0.div_euclid(th) * th;
                    while ty < y1 {
                        let mut tx = x0.div_euclid(tw) * tw;
                        while tx < x1 {
                            let mut d = StageDraw::piece(p, tx, ty, tw, th);
                            d.clip = Some(clip);
                            out.draws.push(d);
                            tx += tw;
                        }
                        ty += th;
                    }
                }
            }
            Lay::Box(p) => {
                let Some((p, _)) = piece(p) else { continue };
                for b in groups(grid, cols, rows) {
                    let [x0, y0, x1, y1] = px(b);
                    out.draws
                        .push(StageDraw::piece(p, x0, y0, x1 - x0, y1 - y0));
                }
            }
            Lay::Run([l, m, rr]) => {
                let get = |n: &Option<String>| n.as_deref().and_then(piece);
                let (l, m, rr) = (get(l), get(m), get(rr));
                for b in groups(grid, cols, rows) {
                    let [x0, y0, x1, y1] = px(b);
                    let h = y1 - y0;
                    // A piece's width at the box's height, by its shape.
                    let wide = |p: Option<(u32, [i64; 2])>| {
                        p.map_or(0, |(_, [pw, ph])| ((pw * h + ph / 2) / ph).max(1))
                    };
                    let (mut lw, mut rw) = (wide(l), wide(rr));
                    let width = x1 - x0;
                    if lw + rw > width {
                        // Ends that don't fit share the box by their widths.
                        let all = lw + rw;
                        lw = lw * width / all;
                        rw = width - lw;
                    }
                    if let Some((p, _)) = m {
                        let (a, z) = (x0 + lw, x1 - rw);
                        let mw = wide(m);
                        let clip = [a, y0, z - a, h];
                        let mut x = a;
                        while x < z {
                            let mut d = StageDraw::piece(p, x, y0, mw, h);
                            d.clip = Some(clip);
                            out.draws.push(d);
                            x += mw;
                        }
                    }
                    if let Some((p, _)) = l
                        && lw > 0
                    {
                        out.draws.push(StageDraw::piece(p, x0, y0, lw, h));
                    }
                    if let Some((p, _)) = rr
                        && rw > 0
                    {
                        out.draws.push(StageDraw::piece(p, x1 - rw, y0, rw, h));
                    }
                }
            }
        }
    }
    Room {
        stage: out,
        cells: counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::stage::StageSrc;

    /// Reads `#+` room lines (without the `#+`) after a stage.
    fn rules(lines: &[&str]) -> (StageRules, RoomRules) {
        let mut st = None;
        for l in [
            "stage 2 10 10 10 20 20 20",
            "stage_piece wall pieces/wall.png 64 64",
            "stage_piece top pieces/top.png 32 16",
            "stage_piece end pieces/end.png 16 32",
            "stage_piece door pieces/door.png 40 80",
        ] {
            let w: Vec<&str> = l.split_whitespace().collect();
            assert!(StageRules::line(&mut st, &w).unwrap(), "{l}");
        }
        let mut rooms = Vec::new();
        for (n, l) in lines.iter().enumerate() {
            let w: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(
                RoomRules::line(&mut rooms, st.as_ref(), &w, n + 1),
                Ok(true),
                "{l}"
            );
        }
        (st.unwrap(), rooms.pop().unwrap())
    }

    /// An 8 by 6 made-up screen of 16-pixel cells, all tile 1 in bank 0,
    /// grey; a counter of tile 2 in bank 1 (red) across cells 2 to 6 of row
    /// 3; a door of tile 3 (green) at cell 0 of rows 1 and 2.
    fn screen() -> Cells {
        let cell = |tile: u16, bank: u8, c: [u8; 3]| Cell {
            tile,
            bank,
            prio: 0,
            colors: vec![c; 4],
        };
        let mut cells = vec![cell(1, 0, [90, 90, 90]); 8 * 6];
        for x in 2..7 {
            cells[3 * 8 + x] = cell(2, 1, [200, 40, 30]);
        }
        cells[8] = cell(3, 0, [30, 200, 40]);
        cells[16] = cell(3, 0, [30, 200, 40]);
        Cells {
            cols: 8,
            rows: 6,
            size: 16,
            cells,
        }
    }

    #[test]
    fn lines_read_and_refuse_what_they_cant() {
        let (_, r) = rules(&[
            "room screen_a screen_b",
            "room_cells bar tile=2 bank=1 r>g+50 share=30",
            "room_cells floor tile=0x1-0x3 !bar",
            "room_run bar end top -",
            "room_fill floor wall",
        ]);
        assert_eq!(r.screens, ["screen_a", "screen_b"]);
        assert_eq!(r.classes[0].tiles, [[2, 2]]);
        assert_eq!(r.classes[0].share, 30);
        assert_eq!(r.classes[1].tiles, [[1, 3]]);
        assert_eq!(r.classes[1].not, ["bar"]);
        assert_eq!(
            r.places[0].lay,
            Lay::Run([Some("end".into()), Some("top".into()), None])
        );
        assert_eq!(r.places[1].line, 5);
        let mut rooms = vec![r];
        let st = StageRules::default();
        for bad in [
            "room_cells x tile=5-2",
            "room_cells x bank=4",
            "room_cells x !nothing",
            "room_cells x what=1",
            "room_fill nothing wall",
            "room_run bar - - -",
            "room_box bar",
        ] {
            let w: Vec<&str> = bad.split_whitespace().collect();
            assert_eq!(
                RoomRules::line(&mut rooms, Some(&st), &w, 1),
                Err(()),
                "{bad}"
            );
        }
        // A room needs the stage line before it.
        assert_eq!(
            RoomRules::line(&mut Vec::new(), None, &["room", "s"], 1),
            Err(())
        );
        assert_eq!(
            RoomRules::line(&mut rooms, None, &["stage_piece"], 1),
            Ok(false)
        );
    }

    #[test]
    fn a_counter_is_a_run_on_its_own_cells() {
        let (st, r) = rules(&[
            "room s",
            "room_cells bar bank=1 r>g+50",
            "room_run bar end top end",
        ]);
        let room = build(&st, &r, &screen());
        assert_eq!(room.cells, [5]);
        // The counter's box in stage pixels: x 64 to 224, y 96 to 128.
        let d = &room.stage.draws;
        let ends: Vec<&StageDraw> = d.iter().filter(|d| d.src == StageSrc::Piece(2)).collect();
        assert_eq!(ends.len(), 2);
        // The box is one cell high, 32 stage pixels at scale 2, so an end
        // (16 by 32) is 16 wide.
        assert_eq!(
            [ends[0].x, ends[0].y, ends[0].w, ends[0].h],
            [64, 96, 16, 32]
        );
        assert_eq!([ends[1].x, ends[1].w], [224 - 16, 16]);
        let tops: Vec<&StageDraw> = d.iter().filter(|d| d.src == StageSrc::Piece(1)).collect();
        // The top (32 by 16) is 64 wide at 32 high, from 80 to 208, cut there.
        assert_eq!(tops[0].x, 80);
        assert_eq!(tops[0].w, 64);
        assert!(tops.iter().all(|t| t.clip == Some([80, 96, 128, 32])));
        assert_eq!(tops.len(), 2);
        assert_eq!(room.stage.size, [128, 96]);
    }

    #[test]
    fn a_fill_covers_its_cells_and_a_box_each_group() {
        let (st, r) = rules(&[
            "room s",
            "room_cells bar tile=2",
            "room_cells door g>r+100",
            "room_cells back !bar !door",
            "room_fill back wall",
            "room_box door door",
            "room_box bar door",
        ]);
        let room = build(&st, &r, &screen());
        assert_eq!(room.cells, [48 - 5 - 2, 2, 5]);
        let fills: Vec<&StageDraw> = room
            .stage
            .draws
            .iter()
            .filter(|d| d.src == StageSrc::Piece(0))
            .collect();
        // Every fill is a whole 64-pixel tile on the tiles' grid, cut to
        // cells of the class.
        for f in &fills {
            assert_eq!((f.x % 64, f.y % 64, f.w, f.h), (0, 0, 64, 64), "{f:?}");
            let [cx, cy, cw, ch] = f.clip.unwrap();
            for y in (cy..cy + ch).step_by(32) {
                for x in (cx..cx + cw).step_by(32) {
                    let i = (y / 32 * 8 + x / 32) as usize;
                    assert!(i != 8 && i != 16 && !(26..31).contains(&i), "{f:?}");
                }
            }
        }
        let boxes: Vec<[i64; 4]> = room
            .stage
            .draws
            .iter()
            .filter(|d| d.src == StageSrc::Piece(3))
            .map(|d| [d.x, d.y, d.w, d.h])
            .collect();
        assert_eq!(boxes, [[0, 32, 32, 64], [64, 96, 160, 32]]);
    }

    #[test]
    fn a_class_that_matches_nothing_counts_none() {
        let (st, r) = rules(&["room s", "room_cells sky b>r+100", "room_fill sky wall"]);
        let room = build(&st, &r, &screen());
        assert_eq!(room.cells, [0]);
        assert!(room.stage.draws.is_empty());
    }
}
