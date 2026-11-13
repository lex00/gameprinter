//! The miner on made-up grids and object lists.

use std::cell::Cell;
use std::collections::BTreeMap;

use gp_sim::spec::Tile;

use crate::format::{Demand, Layer};
use crate::level::{LevelGrid, Object, Placed};
use crate::mine::{Interactive, Options, Tester, mine};

/// A level from rows of characters: `#` floor (value 1), `B` a block
/// (value 2), `c` a coin (value 3, empty to collision with a tile kind),
/// `.` sky (value 0).
fn grid(name: &str, rows: &[&str]) -> LevelGrid {
    let height = rows.len();
    let width = rows[0].len();
    let mut ids = Vec::new();
    let mut class = Vec::new();
    for r in rows {
        for ch in r.chars() {
            let (id, t) = match ch {
                '#' => (1, Tile::Solid),
                'B' => (2, Tile::Solid),
                'c' => (3, Tile::Empty),
                _ => (0, Tile::Empty),
            };
            ids.push(id);
            class.push(t);
        }
    }
    let mut names = BTreeMap::new();
    names.insert(1, "floor".to_owned());
    names.insert(2, "block".to_owned());
    names.insert(3, "coin".to_owned());
    let mut kinds = BTreeMap::new();
    kinds.insert(3, vec!["coin".to_owned()]);
    let mut tiles = BTreeMap::new();
    tiles.insert(0, [0, 0, 0, 0]);
    tiles.insert(2, [1, 2, 3, 4]);
    LevelGrid {
        name: name.into(),
        levels: vec![name.into()],
        layout: format!("grid/{name}"),
        width,
        height,
        tile_size: 16,
        ids,
        class,
        names,
        kinds,
        tiles,
        pits: true,
        ..LevelGrid::default()
    }
}

const ONE: &[&str] = &[
    "....................",
    "...BB.......BB......",
    "....................",
    "......c.............",
    "##########..########",
    "##########..########",
];

fn chunk<'a>(m: &'a crate::Mined, id: &str) -> &'a crate::Chunk {
    m.chunks.iter().find(|c| c.id == id).unwrap_or_else(|| {
        panic!(
            "no {id} in {:?}",
            m.chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
        )
    })
}

#[test]
fn repeated_pieces_become_groups_and_the_floor_stays_terrain() {
    let mut g = grid("one", ONE);
    g.spawns.push(Placed {
        entity: "walker".into(),
        x: 3 * 16,
        y: 0,
    });
    let m = mine(&[g], &Options::default(), &[], None);
    let groups: Vec<_> = m
        .chunks
        .iter()
        .filter(|c| c.layer == Layer::Groups)
        .collect();
    // The two pairs of blocks are one kind; the floor is terrain, and the
    // lone coin is seen once, under the minimum.
    assert_eq!(groups.len(), 1, "{groups:?}");
    let b = groups[0];
    assert_eq!(b.count, 2);
    assert_eq!(b.size, [2, 1]);
    assert_eq!(b.shape, vec![vec![Some(2), Some(2)]]);
    assert_eq!(b.name, "block x2");
    assert_eq!(b.function.classes.get("solid"), Some(&2));
    assert_eq!(b.function.placement.air, 2);
    // The walker stands over the first pair only.
    assert_eq!(b.function.enemies[0].entity, "walker");
    assert_eq!(b.function.enemies[0].share, 0.5);
    // With a minimum of one, the coin is a group too, with its tile kind.
    let m1 = mine(
        &[grid("one", ONE)],
        &Options {
            min_count: 1,
            ..Options::default()
        },
        &[],
        None,
    );
    let coin = m1
        .chunks
        .iter()
        .find(|c| c.layer == Layer::Groups && c.name == "coin")
        .expect("the coin");
    assert_eq!(coin.function.tile_kinds.get("coin"), Some(&1));
}

#[test]
fn a_hole_in_the_floor_is_a_gap_by_width() {
    let m = mine(&[grid("one", ONE)], &Options::default(), &[], None);
    let gap = chunk(&m, "gap:2");
    assert_eq!(gap.count, 1);
    assert_eq!(gap.size, [2, 2]);
    assert_eq!(gap.examples[0].x, 10);
    // A level where falling doesn't kill has no gaps.
    let mut g = grid("one", ONE);
    g.pits = false;
    let m = mine(&[g], &Options::default(), &[], None);
    assert!(m.chunks.iter().all(|c| c.layer != Layer::Gaps));
}

#[test]
fn windows_and_slices_are_counted_across_levels() {
    let m = mine(
        &[grid("one", ONE), grid("two", ONE)],
        &Options::default(),
        &[],
        None,
    );
    // The 2x2 window of floor is everywhere in both levels.
    let floor = m
        .chunks
        .iter()
        .find(|c| c.layer == Layer::Patterns && c.shape == vec![vec![Some(1); 2]; 2])
        .expect("a 2x2 floor window");
    assert_eq!(floor.by_level.len(), 2);
    assert_eq!(floor.count, 2 * 16);
    // A slice is a whole column.
    assert!(
        m.chunks
            .iter()
            .any(|c| c.id.starts_with("slice:") && c.size == [1, 6])
    );
    // Every structure cell of both levels is in some pattern.
    for l in &m.levels {
        assert_eq!(l.coverage.patterns, 1.0, "{l:?}");
        assert_eq!(l.coverage.objects, 0.0);
    }
    // Patterns of mixed values come before plain floor.
    let first = m
        .chunks
        .iter()
        .find(|c| c.layer == Layer::Patterns)
        .unwrap();
    let vals: Vec<_> = first.shape.iter().flatten().collect();
    assert!(vals.iter().any(|v| **v != *vals[0]), "{first:?}");
}

/// A list of three rows of blocks, lengths 1, 2 and 3, as a format's
/// objects with the cells each drew.
fn with_objects() -> LevelGrid {
    let mut g = grid(
        "rows",
        &[
            "..................",
            ".B...BB...BBB.....",
            "..................",
            "##################",
        ],
    );
    let w = g.width;
    let row = |x: usize, n: usize| Object {
        kind: "row of blocks".into(),
        params: vec![("length".into(), n as i64)],
        cells: (x..x + n).map(|x| (w + x, 2)).collect(),
    };
    g.objects = vec![
        Object {
            kind: "header".into(),
            params: vec![],
            cells: (0..w).map(|x| (3 * w + x, 1)).collect(),
        },
        row(1, 1),
        row(5, 2),
        row(10, 3),
        Object {
            kind: "page jump".into(),
            params: vec![],
            cells: vec![],
        },
    ];
    g
}

#[test]
fn object_kinds_have_footprints_as_a_function_of_their_parameters() {
    let m = mine(&[with_objects()], &Options::default(), &[], None);
    let row = chunk(&m, "object:row of blocks");
    assert_eq!(row.count, 3);
    let p = &row.params[0];
    assert_eq!(p.name, "length");
    assert_eq!(p.values, vec![1, 2, 3]);
    assert_eq!(p.footprint[2].size, [3, 1]);
    let fit = p.fit.as_ref().expect("a line through the footprints");
    assert_eq!((fit.axis.as_str(), fit.base, fit.per), ("width", 0, 1));
    // An object that draws nothing is a control, not a chunk.
    assert_eq!(m.controls.get("page jump"), Some(&1));
    assert!(m.chunks.iter().all(|c| c.id != "object:page jump"));
    // The objects explain every structure cell.
    assert_eq!(m.levels[0].coverage.objects, 1.0);
}

#[test]
fn pieces_that_sit_together_are_compounds() {
    let mut g = with_objects();
    for x in [5, 10] {
        g.spawns.push(Placed {
            entity: "flyer".into(),
            x: x * 16,
            y: 0,
        });
    }
    let m = mine(&[g], &Options::default(), &[], None);
    let with = chunk(&m, "compound:with:object:row of blocks|enemy:flyer");
    assert_eq!(with.count, 2);
    assert_eq!(with.name, "row of blocks with flyer");
    assert_eq!(with.relation.as_deref(), Some("with"));
    // Rows next to each other within two cells.
    let rows = &m.chunks.iter().find(|c| c.layer == Layer::Objects).unwrap();
    assert!(
        rows.function
            .adjacency
            .right
            .contains_key("object:row of blocks"),
        "{:?}",
        rows.function.adjacency
    );
}

#[test]
fn interactive_kinds_are_matched_by_the_cells_tiles() {
    let json = serde_json::json!({
        "kinds": [
            { "kind": "cell 01020304 -> 00000000", "cell": { "before": "01020304", "after": "00000000" } },
            { "kind": "cell 00000000 -> 01020304", "cell": { "before": "00000000", "after": "01020304" } },
            { "kind": "sound bump (no cell change)", "cell": null },
        ]
    });
    let kinds = crate::interactive_kinds(&json);
    assert_eq!(
        kinds,
        vec![
            Interactive {
                kind: "cell 01020304 -> 00000000".into(),
                before: [1, 2, 3, 4]
            },
            Interactive {
                kind: "cell 00000000 -> 01020304".into(),
                before: [0, 0, 0, 0]
            },
        ]
    );
    let m = mine(&[grid("one", ONE)], &Options::default(), &kinds, None);
    let b = m.chunks.iter().find(|c| c.layer == Layer::Groups).unwrap();
    // The block's tiles match the first; empty space's would match the
    // second everywhere, so it isn't counted.
    assert_eq!(
        b.function.interactive.keys().collect::<Vec<_>>(),
        vec!["cell 01020304 -> 00000000"]
    );
}

struct Count(Cell<usize>);

impl Tester for Count {
    fn test(
        &self,
        _: &LevelGrid,
        cells: &[(usize, usize, Tile)],
        gap: Option<(usize, usize)>,
        ground: bool,
    ) -> Option<Demand> {
        self.0.set(self.0.get() + 1);
        Some(Demand {
            reachable: true,
            needs: if gap.is_some() { "jump" } else { "walk" }.into(),
            height_px: cells.len() as i64,
            gap_px: 0,
            stands_on_top: ground,
            moves: 0,
        })
    }
}

#[test]
fn a_few_kinds_per_layer_get_a_reachability_test() {
    let t = Count(Cell::new(0));
    let opts = Options {
        reach_tests: 1,
        ..Options::default()
    };
    let m = mine(&[grid("one", ONE)], &opts, &[], Some(&t));
    // One group and one gap; patterns never.
    assert_eq!(t.0.get(), 2);
    assert_eq!(
        chunk(&m, "gap:2")
            .function
            .demand
            .as_ref()
            .map(|d| d.needs.as_str()),
        Some("jump")
    );
    assert!(
        m.chunks
            .iter()
            .filter(|c| c.layer == Layer::Patterns)
            .all(|c| c.function.demand.is_none())
    );
}

#[test]
fn mini_levels_have_a_floor_with_the_gap_cut() {
    let t = crate::demand::mini_level(6, 4, &[(2, 1, Tile::Solid)], Some((3, 4)));
    let row = |y: usize| -> String {
        (0..6)
            .map(|x| {
                if t[y * 6 + x] == Tile::Solid {
                    '#'
                } else {
                    '.'
                }
            })
            .collect()
    };
    assert_eq!(row(0), "......");
    assert_eq!(row(1), "..#...");
    assert_eq!(row(2), "###..#");
    assert_eq!(row(3), "###..#");
}

#[test]
fn the_schema_file_is_current() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema/chunks.json");
    let committed = std::fs::read_to_string(path).unwrap_or_default();
    assert_eq!(
        committed,
        crate::format::schema(),
        "schema/chunks.json is stale: run `gp chunks --schema > schema/chunks.json` (just docs does)"
    );
}
