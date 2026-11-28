//! What `gp theme look cells` answers of a room beside its grid. A room
//! rule places a piece by a class of cells, and a class is chosen by tile,
//! so a theme maker has to know a room's tiles by place: which tiles stand
//! directly left or right of a class's cells (`--beside`), which tiles one
//! box of the screen has and the rest of it lacks, so two like parts can be
//! ruled apart (`--only-in`), and which tiles none of some classes take
//! (`--not`). A theme maker answered these with scripts of their own
//! before (site docs, themes.md, "To see what your tests match").

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Result, bail};
use gp_present::art::room::Cells;
use serde_json::{Value, json};

/// A cell's place on the screen, and whether the board draws it mirrored
/// left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Place {
    pub row: usize,
    pub col: usize,
    pub mirrored: bool,
}

impl Place {
    fn text(&self) -> String {
        format!(
            "r{}c{}{}",
            self.row,
            self.col,
            if self.mirrored { "<" } else { "" }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Side {
    Left,
    Right,
}

impl Side {
    fn word(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
    fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// A tile that stands directly left or right of a class's cell in the
/// same row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Beside {
    pub tile: u16,
    pub bank: u8,
    /// Which side of the tile the class's cell is on in the tile's own
    /// picture, as the board draws it unmirrored: a mirrored cell with the
    /// class on its right on the screen has it on its own left.
    pub own_side: Side,
    pub places: Vec<Place>,
}

/// The tiles beside the cells `class` marks, each tile, bank and own side
/// once, in tile order.
pub(crate) fn beside(cells: &Cells, class: &[bool]) -> Vec<Beside> {
    let mut found: BTreeMap<(u16, u8, Side), Vec<Place>> = BTreeMap::new();
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            let i = row * cells.cols + col;
            if class[i] {
                continue;
            }
            let c = &cells.cells[i];
            for (side, next) in [(Side::Right, col + 1), (Side::Left, col.wrapping_sub(1))] {
                if next >= cells.cols || !class[row * cells.cols + next] {
                    continue;
                }
                let own_side = if c.flip[0] { side.other() } else { side };
                found
                    .entry((c.tile, c.bank, own_side))
                    .or_default()
                    .push(Place {
                        row,
                        col,
                        mirrored: c.flip[0],
                    });
            }
        }
    }
    found
        .into_iter()
        .map(|((tile, bank, own_side), places)| Beside {
            tile,
            bank,
            own_side,
            places,
        })
        .collect()
}

/// A tile that stands inside a box of the screen and nowhere outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnlyIn {
    pub tile: u16,
    /// The banks it is drawn in there.
    pub banks: Vec<u8>,
    /// Its places from the box's top left cell (`row` and `col` counted
    /// from it).
    pub places: Vec<Place>,
}

/// A box of cells, `C0,R0,C1,R1` from its top left cell to its bottom
/// right one, both inside it, as `#+ room_front` gives one.
pub(crate) fn parse_box(s: &str, cells: &Cells) -> Result<[usize; 4]> {
    let n: Vec<usize> = s
        .split([',', ' '])
        .filter(|p| !p.is_empty())
        .map(|p| p.trim().parse::<usize>())
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("--only-in {s}: give C0,R0,C1,R1, the box's top left cell's column and row and its bottom right one's (4,4,12,8)"))?;
    let [c0, r0, c1, r1] = n[..] else {
        bail!(
            "--only-in {s}: give four numbers, C0,R0,C1,R1, the box's top left cell's column and row and its bottom right one's (4,4,12,8)"
        );
    };
    if c0 > c1 || r0 > r1 || c1 >= cells.cols || r1 >= cells.rows {
        bail!(
            "--only-in {s}: the box's top left cell comes first and both are on the screen, columns 0 to {} and rows 0 to {}",
            cells.cols.saturating_sub(1),
            cells.rows.saturating_sub(1)
        );
    }
    Ok([c0, r0, c1, r1])
}

/// The tiles that stand in `bx` and nowhere else on the screen, in tile
/// order: what a class needs to take that box's part and not its likes.
pub(crate) fn only_in(cells: &Cells, bx: [usize; 4]) -> Vec<OnlyIn> {
    let [c0, r0, c1, r1] = bx;
    let inside = |row: usize, col: usize| (r0..=r1).contains(&row) && (c0..=c1).contains(&col);
    let mut outside: BTreeSet<u16> = BTreeSet::new();
    let mut found: BTreeMap<u16, (BTreeSet<u8>, Vec<Place>)> = BTreeMap::new();
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            let c = &cells.cells[row * cells.cols + col];
            if inside(row, col) {
                let e = found.entry(c.tile).or_default();
                e.0.insert(c.bank);
                e.1.push(Place {
                    row: row - r0,
                    col: col - c0,
                    mirrored: c.flip[0],
                });
            } else {
                outside.insert(c.tile);
            }
        }
    }
    found
        .into_iter()
        .filter(|(t, _)| !outside.contains(t))
        .map(|(tile, (banks, places))| OnlyIn {
            tile,
            banks: banks.into_iter().collect(),
            places,
        })
        .collect()
}

/// A tile and bank none of some classes take, with its places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Untaken {
    pub tile: u16,
    pub bank: u8,
    pub places: Vec<Place>,
}

/// The tiles and banks of cells that none of `classes` marks, the most
/// often first (then by tile).
pub(crate) fn untaken(cells: &Cells, classes: &[&[bool]]) -> Vec<Untaken> {
    let mut found: BTreeMap<(u16, u8), Vec<Place>> = BTreeMap::new();
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            let i = row * cells.cols + col;
            if classes.iter().any(|c| c[i]) {
                continue;
            }
            let c = &cells.cells[i];
            found.entry((c.tile, c.bank)).or_default().push(Place {
                row,
                col,
                mirrored: c.flip[0],
            });
        }
    }
    let mut v: Vec<Untaken> = found
        .into_iter()
        .map(|((tile, bank), places)| Untaken { tile, bank, places })
        .collect();
    v.sort_by(|a, b| {
        b.places
            .len()
            .cmp(&a.places.len())
            .then((a.tile, a.bank).cmp(&(b.tile, b.bank)))
    });
    v
}

/// Places as text, at most `max` of them, with how many more.
fn places_text(p: &[Place], max: usize) -> String {
    let mut s: Vec<String> = p.iter().take(max).map(Place::text).collect();
    if p.len() > max {
        s.push(format!("and {} more", p.len() - max));
    }
    s.join(" ")
}

fn places_json(p: &[Place]) -> Value {
    p.iter()
        .map(|p| json!({"row": p.row, "col": p.col, "mirrored": p.mirrored}))
        .collect()
}

pub(crate) fn beside_text(class: &str, v: &[Beside]) -> String {
    let mut t = format!(
        "\nBeside {class}: each tile directly left or right of its cells in the same row, TILE.BANK, the side the class is on in the tile's own picture (as the board draws it unmirrored), then its places (rROWcCOL, < where the board mirrors it)\n"
    );
    if v.is_empty() {
        let _ = writeln!(t, "  none");
    }
    for b in v {
        let _ = writeln!(
            t,
            "  {:03x}.{} {} on its {} ({} cells): {}",
            b.tile,
            b.bank,
            class,
            b.own_side.word(),
            b.places.len(),
            places_text(&b.places, 12)
        );
    }
    t
}

pub(crate) fn beside_json(v: &[Beside]) -> Value {
    v.iter()
        .map(|b| {
            json!({"tile": format!("{:03x}", b.tile), "bank": b.bank, "own_side": b.own_side.word(),
                "places": places_json(&b.places)})
        })
        .collect()
}

pub(crate) fn only_in_text(bx: [usize; 4], v: &[OnlyIn]) -> String {
    let mut t = format!(
        "\nOnly in columns {} to {}, rows {} to {}: each tile that stands there and nowhere else on the screen, its banks, then its places from the box's top left cell (rROWcCOL, < where the board mirrors it)\n",
        bx[0], bx[2], bx[1], bx[3]
    );
    if v.is_empty() {
        let _ = writeln!(t, "  none: every tile in the box stands outside it too");
    }
    for o in v {
        let banks: Vec<String> = o.banks.iter().map(u8::to_string).collect();
        let _ = writeln!(
            t,
            "  {:03x} bank {} ({} cells): {}",
            o.tile,
            banks.join(","),
            o.places.len(),
            places_text(&o.places, 12)
        );
    }
    t
}

pub(crate) fn only_in_json(v: &[OnlyIn]) -> Value {
    v.iter()
        .map(|o| {
            json!({"tile": format!("{:03x}", o.tile), "banks": o.banks, "places": places_json(&o.places)})
        })
        .collect()
}

pub(crate) fn untaken_text(classes: &[String], v: &[Untaken]) -> String {
    let mut t = format!(
        "\nIn none of {}: each TILE.BANK, how many cells, then its places (rROWcCOL, < where the board mirrors it), the most often first\n",
        classes.join(", ")
    );
    if v.is_empty() {
        let _ = writeln!(t, "  none: those classes take every cell");
    }
    for u in v {
        let _ = writeln!(
            t,
            "  {:03x}.{} ({} cells): {}",
            u.tile,
            u.bank,
            u.places.len(),
            places_text(&u.places, 8)
        );
    }
    t
}

pub(crate) fn untaken_json(v: &[Untaken]) -> Value {
    v.iter()
        .map(|u| {
            json!({"tile": format!("{:03x}", u.tile), "bank": u.bank, "cells": u.places.len(),
                "places": places_json(&u.places)})
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_present::art::room::Cell;

    /// A made-up room 6 cells by 3: a bar of tile 0f4 in row 1, columns 2
    /// and 3, with an end cap 0f6 on each side (the left one mirrored), a
    /// lamp 0a0 in the box of columns 0 and 1, rows 0 and 1, and another
    /// lamp 0a1 in that box and at the right edge too; tile 000 elsewhere.
    fn room() -> Cells {
        let mut cells: Vec<Cell> = (0..18).map(|_| Cell::default()).collect();
        let mut put = |row: usize, col: usize, tile: u16, bank: u8, mirrored: bool| {
            cells[row * 6 + col] = Cell {
                tile,
                bank,
                flip: [mirrored, false],
                ..Cell::default()
            };
        };
        put(1, 2, 0xf4, 2, false);
        put(1, 3, 0xf4, 2, false);
        put(1, 1, 0xf6, 2, true);
        put(1, 4, 0xf6, 2, false);
        put(0, 0, 0xa0, 1, false);
        put(0, 1, 0xa1, 1, false);
        put(2, 5, 0xa1, 3, false);
        Cells {
            cols: 6,
            rows: 3,
            size: 16,
            cells,
        }
    }

    fn class_of(cells: &Cells, tiles: &[u16]) -> Vec<bool> {
        cells
            .cells
            .iter()
            .map(|c| tiles.contains(&c.tile))
            .collect()
    }

    #[test]
    fn cells_beside_a_class_give_the_side_in_the_tiles_own_picture() {
        let c = room();
        let bar = class_of(&c, &[0xf4]);
        let v = beside(&c, &bar);
        // Both caps are one tile with the bar on its own left: the right
        // one as drawn, the left one because the board mirrors it.
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!((v[0].tile, v[0].bank, v[0].own_side), (0xf6, 2, Side::Left));
        assert_eq!(
            v[0].places,
            vec![
                Place {
                    row: 1,
                    col: 1,
                    mirrored: true
                },
                Place {
                    row: 1,
                    col: 4,
                    mirrored: false
                }
            ]
        );
        let t = beside_text("bar", &v);
        assert!(
            t.contains("0f6.2 bar on its left (2 cells): r1c1< r1c4"),
            "{t}"
        );
        // Cells of the class itself, and rows above and below, aren't beside it.
        let caps = class_of(&c, &[0xf6]);
        let v = beside(&c, &caps);
        assert!(v.iter().all(|b| b.tile == 0xf4 || b.tile == 0), "{v:?}");
        assert!(
            v.iter().all(|b| b.places.iter().all(|p| p.row == 1)),
            "{v:?}"
        );
    }

    #[test]
    fn cells_only_in_a_box_leave_out_tiles_that_stand_outside_it() {
        let c = room();
        let bx = parse_box("0,0,1,1", &c).unwrap();
        let v = only_in(&c, bx);
        // 0a0 stands only in the box. 0a1 is at the right edge too, the
        // cap 0f6 at column 4 and 000 all round, so none of them is the
        // box's own.
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].tile, 0xa0);
        assert_eq!(v[0].banks, vec![1]);
        assert_eq!(
            v[0].places,
            vec![Place {
                row: 0,
                col: 0,
                mirrored: false
            }]
        );
        assert!(only_in_text(bx, &v).contains("0a0 bank 1 (1 cells): r0c0"));
        // The whole screen holds every tile.
        let all = only_in(&c, parse_box("0,0,5,2", &c).unwrap());
        assert_eq!(all.len(), 5, "{all:?}");
        assert!(parse_box("0,0,6,2", &c).is_err());
        assert!(parse_box("2,0,1,2", &c).is_err());
        assert!(parse_box("0,0,1", &c).is_err());
    }

    #[test]
    fn cells_in_no_named_class_are_listed_most_often_first() {
        let c = room();
        let bar = class_of(&c, &[0xf4]);
        let blank = class_of(&c, &[0]);
        let v = untaken(&c, &[&bar, &blank]);
        let got: Vec<(u16, u8, usize)> =
            v.iter().map(|u| (u.tile, u.bank, u.places.len())).collect();
        assert_eq!(
            got,
            vec![(0xf6, 2, 2), (0xa0, 1, 1), (0xa1, 1, 1), (0xa1, 3, 1)]
        );
        let t = untaken_text(&["bar".into(), "void".into()], &v);
        assert!(t.contains("In none of bar, void"), "{t}");
        assert!(t.contains("0f6.2 (2 cells): r1c1< r1c4"), "{t}");
    }
}
