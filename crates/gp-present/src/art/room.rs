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
//!   screen's top left, cut to the cells. A piece one cell square is laid
//!   cell by cell instead, mirrored where the board mirrors the cell's
//!   tile, so a piece for each tile code skins a room tile for tile.
//! - `#+ room_run CLASS LEFT MID RIGHT`: on each group of the class's cells
//!   (cells side by side or one above the other), over its box, `LEFT` at
//!   its left end, `RIGHT` at its right end and `MID` repeated between,
//!   each as tall as the box and as wide as its art's shape gives; `-` for
//!   none. A counter, a bar, a shelf.
//! - `#+ room_box CLASS PIECE`: the piece over each group's box. A door, a
//!   tap, a sign.
//! - `#+ room_at CLASS PIECE DX DY`: the piece at its own size at each of
//!   the class's cells, its top left `DX DY` screen pixels from the cell's
//!   (either may be negative). A class of one tile that each door or cask
//!   has once, as its anchor, places a whole door or cask where the
//!   original draws it, however its cells touch another's. Where the board
//!   mirrors the anchor's tile left to right, the piece is mirrored and
//!   its offset with it, so a room the game draws mirrored needs no rules
//!   of its own.
//!
//! - `#+ room_react CLASS DX DY PIECE:FRAMES...`: a piece placed as
//!   `room_at` places it, mirrored with its cell, whose picture is chosen
//!   on each frame rather than once with the room: each `PIECE` in turn,
//!   held `FRAMES` game frames (1 or more), over and over, counted on the
//!   game's own frame number so a clip replays the same (a trophy's eyes
//!   looking left, ahead, right). Drawn over the room, under the bodies.
//! - `#+ room_react_when PIECE COUNTER CMP VALUE...`, after a
//!   `room_react`: on the frames every test holds (a model counter's name,
//!   one of `<` `<=` `==` `!=` `>=` `>`, a whole number), that piece is
//!   drawn instead of the cycle's; the first such line that holds wins. A
//!   counter the model hasn't got holds no test.
//! - `#+ room_react_drift VX VY PER`, after a `room_react`: the piece moves
//!   `VX, VY` screen pixels (whole numbers, either may be negative or 0,
//!   not both) every `PER` game frames from where it is placed, counted on
//!   the game's frame number as its cycle is; once it has left the screen
//!   by one edge it comes back in by the other. A cloud, a blimp, a bird.
//!   Where its cell is mirrored it drifts the other way.
//!
//! - `#+ room_mask CLASS PIECE TEST...`: the piece laid on the class's
//!   cells, cell by cell as a fill of one cell, but only on the cell's own
//!   pixels whose colour passes every test (as `room_cells` takes them):
//!   the floor's edge through a cell the room shares with a wall, a door
//!   or a cask, where the original draws it.
//!   A piece larger than a cell is not squeezed into each cell: it repeats
//!   from the screen's top left as a fill's does, so one painting (a
//!   field, a sky) carries on under a room's edges.
//! - `#+ room_front C0 R0 C1 R1`: the room drawn again over the bodies in
//!   the cells from column C0, row R0 to column C1, row R1: a wall a body
//!   walks or is thrown into hides it there.
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
    /// Whether the board draws the tile mirrored, left to right and top
    /// to bottom.
    pub flip: [bool; 2],
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
    /// A piece at each cell, offset in screen pixels.
    At(String, [i64; 2]),
    /// A piece on each cell's pixels that pass the tests.
    Mask(String, Vec<Test>),
    /// A piece at each cell, offset in screen pixels, its picture chosen
    /// each frame (`#+ room_react`).
    React(React, [i64; 2]),
}

/// A test of a model counter: its name, its comparison (one of
/// [`super::CMPS`]) and the value.
pub type CounterTest = (String, String, i64);

/// A `#+ room_react` piece's pictures: its cycle, and the pictures its
/// `#+ room_react_when` lines draw in its place while their tests hold.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct React {
    /// Each piece of the cycle and the game frames it holds (1 or more).
    pub cycle: Vec<(String, u64)>,
    /// Each piece drawn instead while every test holds.
    pub when: Vec<(String, Vec<CounterTest>)>,
    /// `#+ room_react_drift VX VY PER`: it moves `VX, VY` screen pixels
    /// every `PER` game frames, wrapping round the screen.
    pub drift: Option<([i64; 2], u64)>,
}

impl React {
    /// Every piece it names, the cycle's then the `when` lines', by the
    /// number [`React::shown`] gives.
    #[must_use]
    pub fn pieces(&self) -> Vec<&str> {
        self.cycle
            .iter()
            .map(|(p, _)| p.as_str())
            .chain(self.when.iter().map(|(p, _)| p.as_str()))
            .collect()
    }

    /// The number (as [`React::pieces`] has it) of the piece shown on game
    /// frame `frame`: the first `when` line whose tests all hold, `holds`
    /// answering each test (counter, comparison, value), else the cycle's
    /// piece at that frame.
    pub fn shown(&self, frame: u64, holds: impl Fn(&str, &str, i64) -> bool) -> usize {
        if let Some(k) = self
            .when
            .iter()
            .position(|(_, tests)| tests.iter().all(|(c, cmp, v)| holds(c, cmp, *v)))
        {
            return self.cycle.len() + k;
        }
        let all: u64 = self.cycle.iter().map(|(_, n)| *n).sum();
        let mut at = frame % all.max(1);
        for (i, (_, n)) in self.cycle.iter().enumerate() {
            if at < *n {
                return i;
            }
            at -= n;
        }
        0
    }
}

/// A `#+ room_react` piece placed at one cell of a built room: the rule's
/// pictures and, for each piece by [`React::pieces`]' number, its draw
/// there (None for a piece the stage has no `stage_piece` for).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub react: React,
    pub draws: Vec<Option<StageDraw>>,
    /// The screen's size in stage pixels and the stage's scale, which a
    /// drift moves in and wraps round.
    pub screen: [i64; 2],
    pub scale: i64,
}

impl Live {
    /// Its draw on game frame `frame` ([`React::shown`]), if any, moved by
    /// its drift: `VX, VY` screen pixels each `PER` frames from where it is
    /// placed (`VX` the other way where its cell is mirrored), and once it
    /// has left the screen by one edge, coming back in by the other.
    pub fn draw(&self, frame: u64, holds: impl Fn(&str, &str, i64) -> bool) -> Option<StageDraw> {
        let mut d = self
            .draws
            .get(self.react.shown(frame, holds))
            .and_then(Option::as_ref)?
            .clone();
        let Some(([vx, vy], per)) = self.react.drift else {
            return Some(d);
        };
        let vx = if d.flip_x { -vx } else { vx };
        let f = i64::try_from(frame).unwrap_or(i64::MAX / 2);
        let per = i64::try_from(per.max(1)).unwrap_or(1);
        // Whole frames of the step, at the stage's own pixels, so a piece
        // at scale 2 moves a stage pixel at a time rather than two.
        let go = |v: i64| (v * self.scale).saturating_mul(f).div_euclid(per);
        // From just off one edge to just off the other, then round again.
        let wrap = |at: i64, size: i64, room: i64| (at + size).rem_euclid(room + size) - size;
        if vx != 0 {
            d.x = wrap(d.x + go(vx), d.w, self.screen[0]);
        }
        if vy != 0 {
            d.y = wrap(d.y + go(vy), d.h, self.screen[1]);
        }
        // Half off an edge, it shows only its part on the screen.
        d.clip = Some([0, 0, self.screen[0], self.screen[1]]);
        Some(d)
    }
}

/// A rule laying pieces on a class's cells, with its line in the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub class: String,
    pub lay: Lay,
    pub line: usize,
    /// `pick=NAME` on the rule: its alternatives are picked by NAME in
    /// place of their own text ([`pick_named`]), so lists of the same
    /// length with the same NAME, in any room, pick the same one.
    pub pick: Option<String>,
}

impl Place {
    /// The pieces it names.
    #[must_use]
    pub fn pieces(&self) -> Vec<&str> {
        match &self.lay {
            Lay::Fill(p) | Lay::Box(p) | Lay::At(p, _) | Lay::Mask(p, _) => p.split('|').collect(),
            Lay::Run(r) => r.iter().flatten().flat_map(|p| p.split('|')).collect(),
            Lay::React(r, _) => r.pieces(),
        }
    }
}

/// One `#+ room` line's rules.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomRules {
    pub screens: Vec<String>,
    pub classes: Vec<CellClass>,
    pub places: Vec<Place>,
    /// `#+ room_front`: boxes of cells, `[c0, r0, c1, r1]`, the room drawn
    /// over the bodies.
    pub front: Vec<[usize; 4]>,
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
            ["room_front", c0, r0, c1, r1] => {
                let room = rooms.last_mut().ok_or(())?;
                let n = |s: &str| s.parse::<usize>().map_err(|_| ());
                let b = [n(c0)?, n(r0)?, n(c1)?, n(r1)?];
                if b[0] > b[2] || b[1] > b[3] || b[2] > 63 || b[3] > 63 {
                    return Err(());
                }
                room.front.push(b);
            }
            ["room_react_when", p, tests @ ..]
                if *p != "-" && !tests.is_empty() && tests.len() % 3 == 0 =>
            {
                let room = rooms.last_mut().ok_or(())?;
                let Some(Place {
                    lay: Lay::React(r, _),
                    ..
                }) = room.places.last_mut()
                else {
                    return Err(());
                };
                let mut when = Vec::new();
                for t in tests.chunks(3) {
                    let [c, cmp, v] = t else { return Err(()) };
                    if !super::counter_word(c) || !super::CMPS.contains(cmp) {
                        return Err(());
                    }
                    when.push((
                        (*c).to_owned(),
                        (*cmp).to_owned(),
                        v.parse().map_err(|_| ())?,
                    ));
                }
                r.when.push(((*p).to_owned(), when));
            }
            ["room_react_drift", vx, vy, per] => {
                let room = rooms.last_mut().ok_or(())?;
                let Some(Place {
                    lay: Lay::React(r, _),
                    ..
                }) = room.places.last_mut()
                else {
                    return Err(());
                };
                let v: [i64; 2] = [vx.parse().map_err(|_| ())?, vy.parse().map_err(|_| ())?];
                let per: u64 = per.parse().map_err(|_| ())?;
                if r.drift.is_some() || per == 0 || v == [0, 0] || v.iter().any(|n| n.abs() > 64) {
                    return Err(());
                }
                r.drift = Some((v, per));
            }
            [
                k
                @ ("room_fill" | "room_run" | "room_box" | "room_at" | "room_mask" | "room_react"),
                class,
                pieces @ ..,
            ] => {
                let room = rooms.last_mut().ok_or(())?;
                let (pieces, pick) = match pieces.split_last() {
                    Some((p, rest)) if p.starts_with("pick=") => {
                        let n = &p["pick=".len()..];
                        if !name_ok(n) {
                            return Err(());
                        }
                        (rest, Some(n.to_owned()))
                    }
                    _ => (pieces, None),
                };
                if !room.classes.iter().any(|c| c.name == *class) {
                    return Err(());
                }
                let piece = |p: &str| (p != "-").then(|| p.to_owned());
                let lay = match (*k, pieces) {
                    ("room_fill", [p]) if *p != "-" => Lay::Fill((*p).to_owned()),
                    ("room_box", [p]) if *p != "-" => Lay::Box((*p).to_owned()),
                    ("room_mask", [p, tests @ ..]) if *p != "-" && !tests.is_empty() => Lay::Mask(
                        (*p).to_owned(),
                        tests
                            .iter()
                            .map(|t| Test::parse(t).ok_or(()))
                            .collect::<Result<_, _>>()?,
                    ),
                    ("room_at", [p, dx, dy]) if *p != "-" => Lay::At(
                        (*p).to_owned(),
                        [dx.parse().map_err(|_| ())?, dy.parse().map_err(|_| ())?],
                    ),
                    ("room_react", [dx, dy, steps @ ..]) if !steps.is_empty() && pick.is_none() => {
                        let mut cycle = Vec::new();
                        for s in steps {
                            let (p, n) = s.split_once(':').ok_or(())?;
                            let n: u64 = n.parse().map_err(|_| ())?;
                            if p.is_empty() || p == "-" || n == 0 {
                                return Err(());
                            }
                            cycle.push((p.to_owned(), n));
                        }
                        Lay::React(
                            React {
                                cycle,
                                ..React::default()
                            },
                            [dx.parse().map_err(|_| ())?, dy.parse().map_err(|_| ())?],
                        )
                    }
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
                    pick,
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
    /// The `#+ room_react` pieces, one for each cell of their class, in
    /// the order of their rules: drawn frame by frame over the stage.
    pub live: Vec<Live>,
}

/// Each class's cells, by class, as a grid of `cols` by `rows`.
#[must_use]
pub fn classify(r: &RoomRules, cells: &Cells) -> Vec<Vec<bool>> {
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

/// One of a piece name's alternatives (`a|b|c`), each as likely, by
/// `seed`: the same name and seed pick the same one, so a room drawn on
/// several screens shows the same piece on each.
#[must_use]
pub fn pick(name: &str, seed: u64) -> &str {
    pick_named(name, None, seed)
}

/// [`pick`], hashing `key` (a rule's `pick=NAME`) in place of the name's
/// text when given: two lists of the same length with the same key pick
/// the same place in each for every seed, as a bar's painting and the
/// bonus room's painting of it.
#[must_use]
pub fn pick_named<'a>(name: &'a str, key: Option<&str>, seed: u64) -> &'a str {
    let alts: Vec<&str> = name.split('|').collect();
    if alts.len() < 2 {
        return name;
    }
    let mut h = seed ^ 0x9e37_79b9_7f4a_7c15;
    for b in key.unwrap_or(name).bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    alts[(h % alts.len() as u64) as usize]
}

/// Builds the room `r` over the screen's `cells` with the stage's scale,
/// backdrop and pieces: [`build_seeded`] with seed 0.
#[must_use]
pub fn build(st: &StageRules, r: &RoomRules, cells: &Cells) -> Room {
    build_seeded(st, r, cells, 0)
}

/// The room `r` over `cells`, a piece name with alternatives (`a|b`)
/// taking the one `seed` picks ([`pick`]).
#[must_use]
pub fn build_seeded(st: &StageRules, r: &RoomRules, cells: &Cells, seed: u64) -> Room {
    let s = st.scale;
    let (cols, rows) = (cells.cols, cells.rows);
    let side = cells.size * s;
    let mut out = Stage {
        size: [cols as i64 * cells.size, rows as i64 * cells.size],
        scale: s,
        backdrop: st.backdrop,
        draws: Vec::new(),
        ladders: Vec::new(),
        front: r
            .front
            .iter()
            .map(|&[c0, r0, c1, r1]| {
                let z = cells.size;
                [
                    c0 as i64 * z,
                    r0 as i64 * z,
                    (c1 - c0 + 1) as i64 * z,
                    (r1 - r0 + 1) as i64 * z,
                ]
            })
            .collect(),
    };
    let classes = classify(r, cells);
    let mut counts = Vec::with_capacity(r.places.len());
    let mut live = Vec::new();
    // A piece at its own size at cell `i`, its top left `d` screen pixels
    // from the cell's; where the board mirrors the cell's tile, mirrored,
    // its left edge mirrored about the cell's middle.
    let at = |i: usize, (p, [w, h]): (u32, [i64; 2]), [dx, dy]: [i64; 2]| {
        let (x, y) = ((i % cols) as i64 * side, (i / cols) as i64 * side);
        let mirror = cells.cells[i].flip[0];
        let x = if mirror {
            x + side - dx * s - w
        } else {
            x + dx * s
        };
        let mut d = StageDraw::piece(p, x, y + dy * s, w, h);
        d.flip_x = mirror;
        d
    };
    for place in &r.places {
        let piece = |name: &str| {
            let name = pick_named(name, place.pick.as_deref(), seed);
            st.pieces
                .iter()
                .position(|p| p.0 == name)
                .map(|i| (i as u32, st.pieces[i].2))
        };
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
                if tw == side && th == side {
                    for (i, _) in grid.iter().enumerate().filter(|(_, c)| **c) {
                        let (x, y) = ((i % cols) as i64 * side, (i / cols) as i64 * side);
                        let mut d = StageDraw::piece(p, x, y, side, side);
                        d.flip_x = cells.cells[i].flip[0];
                        out.draws.push(d);
                    }
                    continue;
                }
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
            Lay::Mask(p, tests) => {
                let Some((p, [tw, th])) = piece(p) else {
                    continue;
                };
                // A piece one cell square is laid cell by cell; a larger
                // one repeats from the screen's top left, as a fill does,
                // so one painting runs unbroken under a room's edges.
                let large = tw != side || th != side;
                for (i, _) in grid.iter().enumerate().filter(|(_, c)| **c) {
                    let c = &cells.cells[i];
                    // The cell's pixels as a square, row by row, in the
                    // tile's own way round (mirrored as the board draws it).
                    let k = (c.colors.len() as f64).sqrt() as usize;
                    if k == 0 || k * k != c.colors.len() {
                        continue;
                    }
                    let (x, y) = ((i % cols) as i64 * side, (i / cols) as i64 * side);
                    let px = |v: usize| v as i64 * side / k as i64;
                    for r in 0..k {
                        let sr = if c.flip[1] { k - 1 - r } else { r };
                        let on = |col: usize| {
                            let sc = if c.flip[0] { k - 1 - col } else { col };
                            tests.iter().all(|t| t.holds(c.colors[sr * k + sc]))
                        };
                        let mut col = 0;
                        while col < k {
                            if !on(col) {
                                col += 1;
                                continue;
                            }
                            let c0 = col;
                            while col < k && on(col) {
                                col += 1;
                            }
                            let clip = [x + px(c0), y + px(r), px(col) - px(c0), px(r + 1) - px(r)];
                            let mut d = if large {
                                let (tx, ty) =
                                    (clip[0].div_euclid(tw) * tw, clip[1].div_euclid(th) * th);
                                StageDraw::piece(p, tx, ty, tw, th)
                            } else {
                                StageDraw::piece(p, x, y, side, side)
                            };
                            d.clip = Some(clip);
                            out.draws.push(d);
                        }
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
            Lay::At(p, d) => {
                let Some(p) = piece(p) else {
                    continue;
                };
                for (i, _) in grid.iter().enumerate().filter(|(_, c)| **c) {
                    out.draws.push(at(i, p, *d));
                }
            }
            Lay::React(r, d) => {
                // Not in the stage, which is painted once: each frame
                // draws the piece it shows.
                for (i, _) in grid.iter().enumerate().filter(|(_, c)| **c) {
                    live.push(Live {
                        react: r.clone(),
                        draws: r
                            .pieces()
                            .iter()
                            .map(|p| Some(at(i, piece(p)?, *d)))
                            .collect(),
                        screen: [cols as i64 * side, rows as i64 * side],
                        scale: s,
                    });
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
        live,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `#+ room_mask` lays its piece only on a cell's pixels that pass its
    /// tests, a run of them a draw, cut to it.
    #[test]
    fn a_rooms_mask_lays_a_piece_on_the_pixels_that_pass() {
        let mut sc = screen();
        // Cell 9: its top row red, its bottom row grey (2 by 2 pixels).
        sc.cells[9].colors = vec![[200, 40, 30], [200, 40, 30], [90, 90, 90], [90, 90, 90]];
        let (st, r) = rules(&["room s", "room_cells c tile=1", "room_mask c cell r>g+100"]);
        let room = build(&st, &r, &sc);
        let z = sc.size * st.scale;
        // Only cell 9's top row: one draw, cut to it.
        assert_eq!(room.stage.draws.len(), 1);
        let d = &room.stage.draws[0];
        assert_eq!((d.x, d.y), (z, z));
        assert_eq!(d.clip, Some([z, z, z, z / 2]));
        // Mirrored, the same row: a whole row is the same either way.
        sc.cells[9].flip = [true, false];
        assert_eq!(build(&st, &r, &sc).stage.draws.len(), 1);
        // Turned top to bottom, the bottom row.
        sc.cells[9].flip = [false, true];
        let room = build(&st, &r, &sc);
        assert_eq!(room.stage.draws[0].clip, Some([z, z + z / 2, z, z / 2]));
    }

    /// A mask's piece larger than a cell is laid from the screen's top left,
    /// as a fill's is, and only cut to the pixels that pass: one painting
    /// runs on under the room's edges, never squeezed into a cell.
    #[test]
    fn a_rooms_mask_lays_a_large_piece_from_the_screens_corner() {
        let mut sc = screen();
        sc.cells[9].colors = vec![[200, 40, 30], [200, 40, 30], [90, 90, 90], [90, 90, 90]];
        let (st, r) = rules(&["room s", "room_cells c tile=1", "room_mask c wall r>g+100"]);
        let room = build(&st, &r, &sc);
        let z = sc.size * st.scale;
        assert_eq!(room.stage.draws.len(), 1);
        let d = &room.stage.draws[0];
        // The wall is 64 by 64: its tile that holds the cell, at its own size.
        assert_eq!((d.x, d.y), (z.div_euclid(64) * 64, z.div_euclid(64) * 64));
        assert_eq!(d.clip, Some([z, z, z, z / 2]));
    }

    /// `#+ room_front` keeps its boxes of cells, and the built room gives
    /// them in game pixels; a box turned round is refused.
    #[test]
    fn a_rooms_front_boxes_are_its_cells() {
        let (st, r) = rules(&["room s", "room_cells c tile=1", "room_front 0 1 2 3"]);
        assert_eq!(r.front, vec![[0, 1, 2, 3]]);
        let room = build(&st, &r, &screen());
        let z = screen().size;
        assert_eq!(room.stage.front, vec![[0, z, 3 * z, 3 * z]]);
        let mut rooms = Vec::new();
        let w: Vec<&str> = ["room", "s"].to_vec();
        RoomRules::line(&mut rooms, Some(&st), &w, 1).unwrap();
        let w: Vec<&str> = "room_front 2 0 1 0".split_whitespace().collect();
        assert!(RoomRules::line(&mut rooms, Some(&st), &w, 2).is_err());
    }

    /// A piece name's alternatives are each picked about as often over
    /// seeds, a seed always picks the same, and the rule names them all.
    #[test]
    fn a_pieces_alternatives_are_each_as_likely() {
        let name = "p0|p1|p2|p3|p4|p5|p6|p7|p8|p9";
        let mut n = [0u32; 10];
        for seed in 0..20_000u64 {
            let k: usize = pick(name, seed)[1..].parse().unwrap();
            n[k] += 1;
        }
        assert!(n.iter().all(|c| (1_700..2_300).contains(c)), "{n:?}");
        assert_eq!(pick(name, 7), pick(name, 7));
        assert_eq!(pick("one", 7), "one");
        let (_, r) = rules(&["room s", "room_cells c", "room_box c wall|door"]);
        assert_eq!(r.places[0].pieces(), ["wall", "door"]);
    }

    /// Two rooms' rules with the same `pick=NAME` and lists of the same
    /// length pick the same place in each for every seed (a bar's
    /// painting and the bonus room's painting of it), where the lists'
    /// own text alone picks them apart; a bad name is refused.
    #[test]
    fn rules_with_the_same_pick_name_pick_in_step() {
        let (_, bar) = rules(&[
            "room bar",
            "room_cells c",
            "room_at c a0|a1|a2|a3 0 0 pick=art",
        ]);
        let (_, bonus) = rules(&[
            "room bonus",
            "room_cells c",
            "room_box c b0|b1|b2|b3 pick=art",
        ]);
        assert_eq!(bar.places[0].pick.as_deref(), Some("art"));
        assert_eq!(bar.places[0].lay, Lay::At("a0|a1|a2|a3".into(), [0, 0]));
        let (a, b) = ("a0|a1|a2|a3", "b0|b1|b2|b3");
        let mut apart = 0;
        for seed in 0..2_000u64 {
            let x = pick_named(a, bar.places[0].pick.as_deref(), seed);
            let y = pick_named(b, bonus.places[0].pick.as_deref(), seed);
            assert_eq!(x[1..], y[1..], "seed {seed}");
            apart += usize::from(pick(a, seed)[1..] != pick(b, seed)[1..]);
        }
        assert!(apart > 0);
        let mut rooms = Vec::new();
        let (st, _) = rules(&["room s", "room_cells c"]);
        for l in ["room s", "room_cells c"] {
            let w: Vec<&str> = l.split_whitespace().collect();
            RoomRules::line(&mut rooms, Some(&st), &w, 1).unwrap();
        }
        let w: Vec<&str> = "room_box c a|b pick=A!".split_whitespace().collect();
        assert!(RoomRules::line(&mut rooms, Some(&st), &w, 3).is_err());
    }
    use crate::art::stage::StageSrc;

    /// `#+ room_react` places a piece at each cell as `room_at` does
    /// (mirrored with its cell), outside the stage, which is painted once;
    /// it shows its cycle by the game's frame, the same for the same frame,
    /// and a `#+ room_react_when` piece on the frames its tests hold, the
    /// first that holds winning. Lines it can't read are refused.
    #[test]
    fn a_reacting_piece_cycles_and_shows_what_the_counters_say() {
        let (st, r) = rules(&[
            "room s",
            "room_cells door tile=3",
            "room_react door 2 -4 wall:2 top:3",
            "room_react_when end lives == 0",
            "room_react_when door lost >= 1 lives < 9",
        ]);
        let Lay::React(react, off) = &r.places[0].lay else {
            panic!("{:?}", r.places[0].lay)
        };
        assert_eq!(*off, [2, -4]);
        assert_eq!(react.pieces(), ["wall", "top", "end", "door"]);
        assert_eq!(r.places[0].pieces(), react.pieces());
        let mut sc = screen();
        sc.cells[16].flip[0] = true;
        let room = build(&st, &r, &sc);
        // The stage draws nothing of it; one for each of the class's cells.
        assert!(room.stage.draws.is_empty());
        assert_eq!(room.cells, [2]);
        assert_eq!(room.live.len(), 2);
        let none = |_: &str, _: &str, _: i64| false;
        let cycle: Vec<usize> = (0..12).map(|f| react.shown(f, none)).collect();
        assert_eq!(cycle, [0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0]);
        let lives = |n: i64| {
            move |c: &str, cmp: &str, v: i64| match (c, cmp) {
                ("lives", "==") => n == v,
                ("lives", "<") => n < v,
                ("lost", ">=") => 1 >= v,
                _ => false,
            }
        };
        // No lives: the first `when` line, on every frame.
        assert!((0..12).all(|f| react.shown(f, lives(0)) == 2));
        // Lives, and one lost: the second.
        assert!((0..12).all(|f| react.shown(f, lives(3)) == 3));
        // Each piece at its own size (the stage's 2 to a screen pixel),
        // 2, -4 screen pixels from the door's cell at row 1; the mirrored
        // cell's (row 2) mirrored about its middle.
        let (a, b) = (&room.live[0], &room.live[1]);
        let d = a.draw(2, none).unwrap();
        assert_eq!(
            (d.src, d.x, d.y, d.w, d.h, d.flip_x),
            (StageSrc::Piece(1), 4, 32 - 8, 32, 16, false)
        );
        let d = b.draw(2, none).unwrap();
        assert_eq!((d.x, d.y, d.flip_x), (32 - 4 - 32, 64 - 8, true));
        assert_eq!(a.draw(0, none).unwrap().src, StageSrc::Piece(0));
        assert_eq!(a.draw(0, lives(0)).unwrap().src, StageSrc::Piece(2));
        // Refused: a when line with no react before it, a step of no
        // frames or none at all, a bad comparison, pick=.
        let mut rooms = Vec::new();
        for l in ["room s", "room_cells c", "room_at c wall 0 0"] {
            let w: Vec<&str> = l.split_whitespace().collect();
            RoomRules::line(&mut rooms, Some(&st), &w, 1).unwrap();
        }
        for bad in [
            "room_react_when end lives == 0",
            "room_react c 0 0 wall:0",
            "room_react c 0 0 wall",
            "room_react c 0 0",
            "room_react c 0 0 wall:3 pick=x",
        ] {
            let w: Vec<&str> = bad.split_whitespace().collect();
            assert_ne!(
                RoomRules::line(&mut rooms, Some(&st), &w, 4),
                Ok(true),
                "{bad}"
            );
        }
        let w: Vec<&str> = "room_react c 0 0 wall:3".split_whitespace().collect();
        RoomRules::line(&mut rooms, Some(&st), &w, 4).unwrap();
        for bad in [
            "room_react_when end lives = 0",
            "room_react_when end lives ==",
            "room_react_when end lives == x",
        ] {
            let w: Vec<&str> = bad.split_whitespace().collect();
            assert_ne!(
                RoomRules::line(&mut rooms, Some(&st), &w, 5),
                Ok(true),
                "{bad}"
            );
        }
    }

    /// `#+ room_react_drift` moves its piece by the game's frame from where
    /// `room_react` places it, the other way where the cell is mirrored,
    /// out by one edge of the screen and back in by the other, cut to the
    /// screen; it keeps the cycle's pictures. Lines it can't read are
    /// refused.
    #[test]
    fn a_drifting_piece_crosses_the_screen_and_wraps_round() {
        let (st, r) = rules(&[
            "room s",
            "room_cells door tile=3",
            "room_react door 0 0 top:4 end:4",
            "room_react_drift 1 0 2",
        ]);
        let Lay::React(react, _) = &r.places[0].lay else {
            panic!("{:?}", r.places[0].lay)
        };
        assert_eq!(react.drift, Some(([1, 0], 2)));
        let mut sc = screen();
        sc.cells[16].flip[0] = true;
        let room = build(&st, &r, &sc);
        let (a, b) = (&room.live[0], &room.live[1]);
        let none = |_: &str, _: &str, _: i64| false;
        // One screen pixel each 2 frames is one stage pixel a frame at
        // scale 2; the screen is 8 cells of 32 stage pixels, 256 wide; the
        // top is 32 wide.
        let at = |l: &Live, f: u64| {
            let d = l.draw(f, none).unwrap();
            (d.x, d.y)
        };
        assert_eq!(at(a, 0), (0, 32));
        assert_eq!(at(a, 3), (3, 32));
        // Its last pixel on the right edge, then just off the left, then
        // coming in by it.
        assert_eq!(at(a, 255), (255, 32));
        assert_eq!(at(a, 256), (-32, 32));
        assert_eq!(at(a, 257), (-31, 32));
        // Round in 256 + 32 frames, back where it started.
        assert_eq!(at(a, 288), (0, 32));
        assert_eq!(at(a, 290), at(a, 2));
        // The cycle runs as it would standing still: end on frame 4.
        assert_eq!(a.draw(4, none).unwrap().src, StageSrc::Piece(2));
        assert_eq!(a.draw(4, none).unwrap().x, 4);
        // Cut to the screen, in stage pixels.
        assert_eq!(a.draw(257, none).unwrap().clip, Some([0, 0, 256, 192]));
        // The mirrored cell's goes left.
        assert_eq!(at(b, 0), (0, 64));
        assert_eq!(at(b, 10), (-10, 64));
        assert_eq!(at(b, 33), (256 - 1, 64));
        // Up the screen (192 high), the top 16 high: off the top, in at
        // the bottom.
        let (st, r) = rules(&[
            "room s",
            "room_cells door tile=3",
            "room_react door 0 0 top:1",
            "room_react_drift 0 -2 1",
        ]);
        let room = build(&st, &r, &screen());
        let l = &room.live[0];
        assert_eq!(at(l, 4), (0, 32 - 16));
        assert_eq!(at(l, 12), (0, 32 - 48));
        // At -20 it would be 4 past just off the top (-16), so it stands 4
        // above just off the bottom (192).
        assert_eq!(at(l, 13), (0, 192 - 4));
        // Without a drift, where room_react places it, and not cut.
        let (st, r) = rules(&[
            "room s",
            "room_cells door tile=3",
            "room_react door 0 0 top:1",
        ]);
        let room = build(&st, &r, &screen());
        let d = room.live[0].draw(500, none).unwrap();
        assert_eq!((d.x, d.y, d.clip), (0, 32, None));
        // Refused: no react before it, a second drift, no frames, no
        // motion, a word that isn't a number, too fast.
        let mut rooms = Vec::new();
        for l in ["room s", "room_cells c", "room_at c wall 0 0"] {
            let w: Vec<&str> = l.split_whitespace().collect();
            RoomRules::line(&mut rooms, Some(&st), &w, 1).unwrap();
        }
        let w: Vec<&str> = "room_react_drift 1 0 1".split_whitespace().collect();
        assert_eq!(RoomRules::line(&mut rooms, Some(&st), &w, 4), Err(()));
        for l in ["room_react c 0 0 wall:3", "room_react_drift 1 0 1"] {
            let w: Vec<&str> = l.split_whitespace().collect();
            RoomRules::line(&mut rooms, Some(&st), &w, 5).unwrap();
        }
        for bad in [
            "room_react_drift 1 0 1",
            "room_react_drift 1 0 0",
            "room_react_drift 0 0 3",
            "room_react_drift x 0 3",
            "room_react_drift 99 0 3",
            "room_react_drift 1 0",
        ] {
            let w: Vec<&str> = bad.split_whitespace().collect();
            assert_ne!(
                RoomRules::line(&mut rooms, Some(&st), &w, 6),
                Ok(true),
                "{bad}"
            );
        }
    }

    /// Reads `#+` room lines (without the `#+`) after a stage.
    fn rules(lines: &[&str]) -> (StageRules, RoomRules) {
        let mut st = None;
        for l in [
            "stage 2 10 10 10 20 20 20",
            "stage_piece wall pieces/wall.png 64 64",
            "stage_piece top pieces/top.png 32 16",
            "stage_piece end pieces/end.png 16 32",
            "stage_piece door pieces/door.png 40 80",
            "stage_piece cell pieces/cell.png 32 32",
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
            flip: [false; 2],
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
            "room_at bar door 1",
            "room_at bar door x 0",
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
    fn a_piece_at_each_anchor_cell_keeps_its_size_and_offset() {
        let (st, r) = rules(&["room s", "room_cells door tile=3", "room_at door door -4 2"]);
        assert_eq!(r.places[0].lay, Lay::At("door".into(), [-4, 2]));
        let room = build(&st, &r, &screen());
        assert_eq!(room.cells, [2]);
        let at: Vec<[i64; 4]> = room
            .stage
            .draws
            .iter()
            .map(|d| [d.x, d.y, d.w, d.h])
            .collect();
        // Cells (0, 1) and (0, 2) at scale 2, moved -8, 4 stage pixels;
        // the door's own 40 by 80 each time, though the cells touch.
        assert_eq!(at, [[-8, 36, 40, 80], [-8, 68, 40, 80]]);
        // The same door mirrored: its right edge 8 past the cell's right.
        let mut cells = screen();
        cells.cells[8].flip[0] = true;
        let room = build(&st, &r, &cells);
        let d = &room.stage.draws[0];
        assert_eq!([d.x, d.y, d.w, d.h], [32 + 8 - 40, 36, 40, 80]);
        assert!(d.flip_x && !room.stage.draws[1].flip_x);
    }

    #[test]
    fn a_tile_sized_fill_goes_cell_by_cell_and_mirrors_with_the_board() {
        let mut st = None;
        for l in [
            "stage 2 10 10 10 20 20 20",
            "stage_piece t pieces/t.png 32 32",
        ] {
            let w: Vec<&str> = l.split_whitespace().collect();
            StageRules::line(&mut st, &w).unwrap();
        }
        let st = st.unwrap();
        let mut rooms = Vec::new();
        for l in ["room s", "room_cells bar tile=2", "room_fill bar t"] {
            let w: Vec<&str> = l.split_whitespace().collect();
            RoomRules::line(&mut rooms, Some(&st), &w, 1).unwrap();
        }
        let mut cells = screen();
        cells.cells[3 * 8 + 4].flip[0] = true;
        let room = build(&st, &rooms[0], &cells);
        let d = &room.stage.draws;
        assert_eq!(d.len(), 5);
        assert!(
            d.iter()
                .all(|d| d.w == 32 && d.h == 32 && d.y == 96 && d.clip.is_none())
        );
        assert_eq!(
            d.iter().map(|d| d.x).collect::<Vec<_>>(),
            [64, 96, 128, 160, 192]
        );
        assert_eq!(
            d.iter().map(|d| d.flip_x).collect::<Vec<_>>(),
            [false, false, true, false, false]
        );
    }

    #[test]
    fn a_class_that_matches_nothing_counts_none() {
        let (st, r) = rules(&["room s", "room_cells sky b>r+100", "room_fill sky wall"]);
        let room = build(&st, &r, &screen());
        assert_eq!(room.cells, [0]);
        assert!(room.stage.draws.is_empty());
    }
}
