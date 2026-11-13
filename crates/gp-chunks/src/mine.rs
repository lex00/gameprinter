//! Mining a game's levels into chunk kinds, and each kind's function
//! (docs/chunks.md).
//!
//! Two capture layers, plus the groupings above them:
//!
//! - objects: the format's own object vocabulary, when its levels are object
//!   lists. Each kind's footprint comes from the cells each object drew,
//!   so the builder that expands it is the game's own.
//! - groups, gaps and patterns, mined from the decoded grid of any format:
//!   connected pieces of the level's structure by shape, holes in the floor
//!   by width, and frequent windows (the overlapping patterns of Gumin's
//!   WaveFunctionCollapse) and one-cell slices (the vertical slices of
//!   Dahlskog, Togelius and Nelson's n-gram levels).
//! - compounds: two pieces, or a piece and an entity, that sit together in
//!   the same way at least `min_count` times (a pipe with a piranha, a row
//!   of blocks over a gap).

use std::collections::{BTreeMap, BTreeSet};

use gp_sim::spec::Tile;

use crate::format::{
    Adjacency, Chunk, Coverage, Demand, Example, Fit, Footprint, Function, Layer, Layers,
    LevelInfo, Near, Param,
};
use crate::level::{LevelGrid, class_name};

/// How the miner runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub min_count: usize,
    pub windows: Vec<[usize; 2]>,
    pub slices: bool,
    pub max_patterns: usize,
    pub max_group: [usize; 2],
    pub reach_tests: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            min_count: 2,
            windows: vec![[2, 2], [3, 3], [4, 4]],
            slices: true,
            max_patterns: 200,
            max_group: [16, 16],
            reach_tests: 6,
        }
    }
}

/// Runs a reachability search on a made-up level holding one occurrence
/// (crate::demand).
pub trait Tester {
    /// `cells` are the occurrence's cells in its level, `(x, y, class)`;
    /// `gap` the columns of a floor gap to cut, first and last; `ground`
    /// whether it rests on something solid.
    fn test(
        &self,
        grid: &LevelGrid,
        cells: &[(usize, usize, Tile)],
        gap: Option<(usize, usize)>,
        ground: bool,
    ) -> Option<Demand>;
}

/// An interactive element kind and the four tile numbers its cell shows
/// before it changes (docs/formats/interactive.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interactive {
    pub kind: String,
    pub before: [u8; 4],
}

/// What the miner found.
#[derive(Debug, Clone, PartialEq)]
pub struct Mined {
    pub chunks: Vec<Chunk>,
    pub levels: Vec<LevelInfo>,
    pub layers: Layers,
    pub controls: BTreeMap<String, usize>,
    pub notes: Vec<String>,
}

/// One occurrence: its level, its bounding box (inclusive) and its cells.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Inst {
    level: usize,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    /// Cells with the value the occurrence gives each.
    cells: Vec<(usize, u32)>,
    /// Cells still holding that value in the finished level.
    kept: Vec<usize>,
    params: Vec<(String, i64)>,
}

impl Inst {
    fn of_cells(level: usize, g: &LevelGrid, cells: Vec<(usize, u32)>) -> Option<Self> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for &(i, _) in &cells {
            let (x, y) = (i % g.width, i / g.width);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        if cells.is_empty() {
            return None;
        }
        let kept = cells
            .iter()
            .filter(|(i, v)| g.ids[*i] == *v)
            .map(|(i, _)| *i)
            .collect();
        Some(Self {
            level,
            x0,
            y0,
            x1,
            y1,
            cells,
            kept,
            params: Vec::new(),
        })
    }

    fn size(&self) -> [usize; 2] {
        [self.x1 - self.x0 + 1, self.y1 - self.y0 + 1]
    }

    /// An empty grid the size of the bounding box.
    fn shape(&self) -> Vec<Vec<Option<u32>>> {
        let [w, h] = self.size();
        vec![vec![None; w]; h]
    }
}

struct Kind {
    id: String,
    layer: Layer,
    name: String,
    insts: Vec<Inst>,
    params: Vec<Param>,
    /// The example occurrence's index in `insts`.
    example: usize,
    relation: Option<String>,
    parts: Vec<String>,
}

/// What each level is made of, worked out once.
struct Prep {
    fg: Vec<bool>,
    terrain: Vec<bool>,
}

fn prep(g: &LevelGrid) -> Prep {
    let n = g.ids.len();
    let fg: Vec<bool> = (0..n).map(|i| g.foreground(i)).collect();
    // Terrain: the values that make up the floor, those seen in the stack
    // of structure rising from the bottom (from the sides, for a level that
    // runs down) in at least half the columns (rows), and the commonest
    // structure value; their cells that join an edge of the level through
    // cells of those values.
    let mut count: BTreeMap<u32, usize> = BTreeMap::new();
    for (i, _) in fg.iter().enumerate().filter(|(_, f)| **f) {
        *count.entry(g.ids[i]).or_default() += 1;
    }
    let mut ids: BTreeSet<u32> = count
        .iter()
        .max_by_key(|(id, c)| (**c, std::cmp::Reverse(**id)))
        .map(|(id, _)| *id)
        .into_iter()
        .collect();
    let mut stacks: BTreeMap<u32, usize> = BTreeMap::new();
    let lines = if g.across() { g.width } else { g.height };
    for line in 0..lines {
        let mut seen = BTreeSet::new();
        let walks: Vec<Vec<usize>> = if g.across() {
            vec![(0..g.height).rev().map(|y| g.at(line, y)).collect()]
        } else {
            vec![
                (0..g.width).map(|x| g.at(x, line)).collect(),
                (0..g.width).rev().map(|x| g.at(x, line)).collect(),
            ]
        };
        for walk in walks {
            for i in walk {
                if !fg[i] {
                    break;
                }
                seen.insert(g.ids[i]);
            }
        }
        for v in seen {
            *stacks.entry(v).or_default() += 1;
        }
    }
    ids.extend(
        stacks
            .into_iter()
            .filter(|(_, c)| *c * 2 >= lines)
            .map(|(id, _)| id),
    );
    let mut terrain = vec![false; n];
    let is_t = |i: usize| fg[i] && ids.contains(&g.ids[i]);
    let mut stack: Vec<usize> = (0..n)
        .filter(|&i| {
            let (x, y) = (i % g.width, i / g.width);
            (x == 0 || y == 0 || x + 1 == g.width || y + 1 == g.height) && is_t(i)
        })
        .collect();
    while let Some(i) = stack.pop() {
        if terrain[i] {
            continue;
        }
        terrain[i] = true;
        for j in neighbours(g, i) {
            if !terrain[j] && is_t(j) {
                stack.push(j);
            }
        }
    }
    Prep { fg, terrain }
}

fn neighbours(g: &LevelGrid, i: usize) -> impl Iterator<Item = usize> {
    let (w, h) = (g.width, g.height);
    let (x, y) = (i % w, i / w);
    let mut out = Vec::with_capacity(4);
    if x > 0 {
        out.push(i - 1);
    }
    if x + 1 < w {
        out.push(i + 1);
    }
    if y > 0 {
        out.push(i - w);
    }
    if y + 1 < h {
        out.push(i + w);
    }
    out.into_iter()
}

/// FNV-1a over the values, as ten hex digits: a short, stable name for a
/// shape.
fn hash(vals: impl IntoIterator<Item = u64>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in vals {
        for b in v.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{:010x}", h >> 24)
}

fn shape_hash(shape: &[Vec<Option<u32>>]) -> String {
    let w = shape.first().map_or(0, Vec::len) as u64;
    hash(
        std::iter::once(w).chain(
            shape
                .iter()
                .flatten()
                .map(|c| c.map_or(u64::MAX, u64::from)),
        ),
    )
}

/// An occurrence's shape: its cells' values in its bounding box.
fn shape_of(g: &LevelGrid, inst: &Inst) -> Vec<Vec<Option<u32>>> {
    let mut s = inst.shape();
    for &(i, v) in &inst.cells {
        let (x, y) = (i % g.width, i / g.width);
        s[y - inst.y0][x - inst.x0] = Some(v);
    }
    s
}

/// A name from the values' names, commonest first.
fn cells_name(g: &LevelGrid, vals: impl IntoIterator<Item = u32>, bg: u32) -> String {
    let mut n: BTreeMap<String, usize> = BTreeMap::new();
    for v in vals {
        if v == bg {
            continue;
        }
        let name = g
            .names
            .get(&v)
            .cloned()
            .unwrap_or_else(|| format!("${v:02X}"));
        *n.entry(name).or_default() += 1;
    }
    let mut v: Vec<(String, usize)> = n.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let parts: Vec<String> = v
        .iter()
        .take(3)
        .map(|(name, c)| {
            if *c > 1 {
                format!("{name} x{c}")
            } else {
                name.clone()
            }
        })
        .collect();
    if parts.is_empty() {
        "empty".into()
    } else if v.len() > 3 {
        format!("{} and more", parts.join(", "))
    } else {
        parts.join(", ")
    }
}

fn params_of(insts: &[Inst]) -> Vec<Param> {
    let mut names: Vec<String> = Vec::new();
    for i in insts {
        for (n, _) in &i.params {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
    }
    names
        .into_iter()
        .map(|name| {
            let mut by: BTreeMap<i64, BTreeMap<[usize; 2], usize>> = BTreeMap::new();
            for i in insts {
                if let Some((_, v)) = i.params.iter().find(|(n, _)| *n == name) {
                    *by.entry(*v).or_default().entry(i.size()).or_default() += 1;
                }
            }
            let footprint: Vec<Footprint> = by
                .iter()
                .map(|(v, sizes)| {
                    let (size, _) = sizes
                        .iter()
                        .max_by_key(|(s, c)| (**c, std::cmp::Reverse(**s)))
                        .expect("a value seen has a size");
                    Footprint {
                        value: *v,
                        size: *size,
                        count: sizes.values().sum(),
                    }
                })
                .collect();
            let fit = fit(&footprint);
            Param {
                name,
                values: by.keys().copied().collect(),
                footprint,
                fit,
            }
        })
        .collect()
}

/// A line `axis = base + per * value` through every footprint, when there
/// is one with a nonzero slope (width tried first).
fn fit(f: &[Footprint]) -> Option<Fit> {
    if f.len() < 2 {
        return None;
    }
    for (axis, k) in [("width", 0), ("height", 1)] {
        let (a, b) = (&f[0], &f[f.len() - 1]);
        let dv = b.value - a.value;
        let ds = b.size[k] as i64 - a.size[k] as i64;
        if dv == 0 || ds == 0 || ds % dv != 0 {
            continue;
        }
        let per = ds / dv;
        let base = a.size[k] as i64 - per * a.value;
        if f.iter().all(|p| p.size[k] as i64 == base + per * p.value) {
            return Some(Fit {
                axis: axis.into(),
                base,
                per,
            });
        }
    }
    None
}

/// Mines every level into chunk kinds and works out what each does.
pub fn mine(
    grids: &[LevelGrid],
    opts: &Options,
    interactive: &[Interactive],
    tester: Option<&dyn Tester>,
) -> Mined {
    let preps: Vec<Prep> = grids.iter().map(prep).collect();
    let bgs: Vec<u32> = grids.iter().map(LevelGrid::background).collect();
    let mut kinds: Vec<Kind> = Vec::new();
    let mut controls: BTreeMap<String, usize> = BTreeMap::new();
    let mut notes = Vec::new();

    // Layer a: the format's objects.
    let mut by_kind: BTreeMap<String, Vec<Inst>> = BTreeMap::new();
    for (l, g) in grids.iter().enumerate() {
        for o in &g.objects {
            match Inst::of_cells(l, g, o.cells.clone()) {
                Some(mut inst) => {
                    inst.params = o.params.clone();
                    by_kind.entry(o.kind.clone()).or_default().push(inst);
                }
                None => *controls.entry(o.kind.clone()).or_default() += 1,
            }
        }
    }
    for (name, insts) in by_kind {
        // The example: the first occurrence of the commonest parameters.
        let mut n: BTreeMap<&Vec<(String, i64)>, usize> = BTreeMap::new();
        for i in &insts {
            *n.entry(&i.params).or_default() += 1;
        }
        let common = n
            .iter()
            .max_by_key(|(p, c)| (**c, std::cmp::Reverse(**p)))
            .map(|(p, _)| (*p).clone())
            .unwrap_or_default();
        let example = insts.iter().position(|i| i.params == common).unwrap_or(0);
        kinds.push(Kind {
            id: format!("object:{name}"),
            layer: Layer::Objects,
            params: params_of(&insts),
            name,
            insts,
            example,
            relation: None,
            parts: vec![],
        });
    }

    // Layer b: groups, the connected pieces of the structure.
    let mut groups: BTreeMap<Vec<Vec<Option<u32>>>, Vec<Inst>> = BTreeMap::new();
    let mut masses = 0;
    for (l, g) in grids.iter().enumerate() {
        let p = &preps[l];
        let mut seen = vec![false; g.ids.len()];
        for start in 0..g.ids.len() {
            if seen[start] || !p.fg[start] || p.terrain[start] {
                continue;
            }
            let mut cells = Vec::new();
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(i) = stack.pop() {
                cells.push((i, g.ids[i]));
                for j in neighbours(g, i) {
                    if !seen[j] && p.fg[j] && !p.terrain[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            cells.sort_unstable();
            let inst = Inst::of_cells(l, g, cells).expect("a piece has a cell");
            let [w, h] = inst.size();
            if w > opts.max_group[0] || h > opts.max_group[1] {
                masses += 1;
                continue;
            }
            groups.entry(shape_of(g, &inst)).or_default().push(inst);
        }
    }
    if masses > 0 {
        notes.push(format!(
            "{masses} connected pieces larger than {} by {} cells were left to terrain",
            opts.max_group[0], opts.max_group[1]
        ));
    }
    for (shape, insts) in groups {
        if insts.len() < opts.min_count {
            continue;
        }
        let g = &grids[insts[0].level];
        let name = cells_name(
            g,
            shape.iter().flatten().flatten().copied(),
            bgs[insts[0].level],
        );
        kinds.push(Kind {
            id: format!("group:{}", shape_hash(&shape)),
            layer: Layer::Groups,
            name,
            insts,
            params: vec![],
            example: 0,
            relation: None,
            parts: vec![],
        });
    }

    // Layer b: gaps in the floor, by width.
    let mut gaps: BTreeMap<usize, Vec<Inst>> = BTreeMap::new();
    for (l, g) in grids.iter().enumerate() {
        if !g.pits || !g.across() || g.height == 0 {
            continue;
        }
        let y = g.height - 1;
        // A column with anything but empty space at the bottom (a floor, a
        // ladder going on down, water) is no pit.
        let floor: Vec<bool> = (0..g.width)
            .map(|x| g.class[g.at(x, y)] != Tile::Empty)
            .collect();
        if floor.iter().filter(|f| **f).count() * 4 < g.width {
            continue;
        }
        let mut x = 0;
        while x < g.width {
            if floor[x] {
                x += 1;
                continue;
            }
            let a = x;
            while x < g.width && !floor[x] {
                x += 1;
            }
            if a == 0 || x == g.width {
                continue;
            }
            // The floor's top beside the gap, on its left.
            let mut top = y;
            while top > 0 && g.standable(g.at(a - 1, top - 1)) {
                top -= 1;
            }
            let mut cells = Vec::new();
            for cy in top..=y {
                for cx in a..x {
                    cells.push((g.at(cx, cy), g.ids[g.at(cx, cy)]));
                }
            }
            let inst = Inst::of_cells(l, g, cells).expect("a gap has a cell");
            gaps.entry(x - a).or_default().push(inst);
        }
    }
    for (w, insts) in gaps {
        kinds.push(Kind {
            id: format!("gap:{w}"),
            layer: Layer::Gaps,
            name: format!("gap {w} wide"),
            insts,
            params: vec![],
            example: 0,
            relation: None,
            parts: vec![],
        });
    }

    // Layer b: windows and slices.
    type Key = ([usize; 2], Vec<u32>);
    let mut windows: BTreeMap<Key, Vec<(usize, usize, usize)>> = BTreeMap::new();
    for (l, g) in grids.iter().enumerate() {
        let p = &preps[l];
        let mut sizes: Vec<[usize; 2]> = opts.windows.clone();
        if opts.slices {
            sizes.push(if g.across() {
                [1, g.height]
            } else {
                [g.width, 1]
            });
        }
        for [w, h] in sizes {
            if w > g.width || h > g.height {
                continue;
            }
            for y in 0..=(g.height - h) {
                for x in 0..=(g.width - w) {
                    let mut vals = Vec::with_capacity(w * h);
                    let mut any = false;
                    for dy in 0..h {
                        for dx in 0..w {
                            let i = g.at(x + dx, y + dy);
                            any |= p.fg[i];
                            vals.push(g.ids[i]);
                        }
                    }
                    if any {
                        windows.entry(([w, h], vals)).or_default().push((l, x, y));
                    }
                }
            }
        }
    }
    type Found = (Key, Vec<(usize, usize, usize)>);
    let mut by_size: BTreeMap<[usize; 2], Vec<Found>> = BTreeMap::new();
    for (k, at) in windows {
        if at.len() >= opts.min_count {
            by_size.entry(k.0).or_default().push((k, at));
        }
    }
    for (size, mut list) in by_size {
        let slice = !opts.windows.contains(&size);
        // Mixed patterns first, then the commonest.
        list.sort_by(|a, b| {
            let mixed = |k: &Key| k.1.iter().any(|v| *v != k.1[0]);
            mixed(&b.0)
                .cmp(&mixed(&a.0))
                .then(b.1.len().cmp(&a.1.len()))
                .then(a.0.cmp(&b.0))
        });
        list.truncate(opts.max_patterns);
        for ((_, vals), at) in list {
            let [w, h] = size;
            let insts: Vec<Inst> = at
                .iter()
                .map(|&(l, x, y)| {
                    let g = &grids[l];
                    let mut cells = Vec::with_capacity(w * h);
                    for dy in 0..h {
                        for dx in 0..w {
                            let i = g.at(x + dx, y + dy);
                            cells.push((i, g.ids[i]));
                        }
                    }
                    Inst::of_cells(l, g, cells).expect("a window has cells")
                })
                .collect();
            let shape: Vec<Vec<Option<u32>>> = vals
                .chunks(w)
                .map(|r| r.iter().map(|v| Some(*v)).collect())
                .collect();
            let l = insts[0].level;
            let id = if slice {
                format!("slice:{}", shape_hash(&shape))
            } else {
                format!("window:{w}x{h}:{}", shape_hash(&shape))
            };
            kinds.push(Kind {
                id,
                layer: Layer::Patterns,
                name: format!(
                    "{} {w}x{h}: {}",
                    if slice { "slice" } else { "window" },
                    cells_name(&grids[l], vals.iter().copied(), bgs[l])
                ),
                insts,
                params: vec![],
                example: 0,
                relation: None,
                parts: vec![],
            });
        }
    }

    // The pieces each level is made of, for compounds and adjacency: its
    // objects (or its groups, for a format without objects) and its gaps.
    // Objects wider than two screens set the level's terrain and aren't
    // pieces.
    let mut units: Vec<Vec<(usize, usize)>> = vec![Vec::new(); grids.len()];
    for (k, kind) in kinds.iter().enumerate() {
        for (n, inst) in kind.insts.iter().enumerate() {
            let has_objects = !grids[inst.level].objects.is_empty();
            let piece = match kind.layer {
                Layer::Objects => inst.size()[0] <= 32,
                Layer::Groups => !has_objects,
                Layer::Gaps => true,
                _ => false,
            };
            if piece {
                units[inst.level].push((k, n));
            }
        }
    }

    // Compounds.
    let mut pairs: BTreeMap<(String, String, String), Vec<Inst>> = BTreeMap::new();
    for (l, g) in grids.iter().enumerate() {
        let us = &units[l];
        for &(ka, na) in us {
            let a = &kinds[ka].insts[na];
            for s in &g.spawns {
                if near(g, a, s.x, s.y) {
                    let mut i = a.clone();
                    i.params.clear();
                    pairs
                        .entry((
                            "with".into(),
                            kinds[ka].id.clone(),
                            format!("enemy:{}", s.entity),
                        ))
                        .or_default()
                        .push(i);
                }
            }
            for &(kb, nb) in us {
                if (ka, na) == (kb, nb) {
                    continue;
                }
                let b = &kinds[kb].insts[nb];
                let over_x = a.x0 <= b.x1 && b.x0 <= a.x1;
                let over_y = a.y0 <= b.y1 && b.y0 <= a.y1;
                let rel = if kinds[kb].layer == Layer::Gaps && kinds[ka].layer != Layer::Gaps {
                    (over_x && a.y1 < b.y0).then_some("over")
                } else if kinds[ka].layer == Layer::Gaps || kinds[kb].layer == Layer::Gaps {
                    None
                } else if over_x && b.y0 > a.y1 && b.y0 - a.y1 <= 4 {
                    Some("above")
                } else if over_y && b.x0 > a.x1 && b.x0 - a.x1 <= 2 {
                    Some("left-of")
                } else {
                    None
                };
                if let Some(rel) = rel {
                    let mut cells = a.cells.clone();
                    cells.extend(b.cells.iter().copied());
                    cells.sort_unstable();
                    cells.dedup_by_key(|c| c.0);
                    let inst = Inst::of_cells(l, g, cells).expect("a pair has cells");
                    pairs
                        .entry((rel.into(), kinds[ka].id.clone(), kinds[kb].id.clone()))
                        .or_default()
                        .push(inst);
                }
            }
        }
    }
    let names: BTreeMap<String, String> = kinds
        .iter()
        .map(|k| (k.id.clone(), k.name.clone()))
        .collect();
    for ((rel, a, b), insts) in pairs {
        if insts.len() < opts.min_count {
            continue;
        }
        let name_of = |id: &str| {
            names
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.trim_start_matches("enemy:").to_owned())
        };
        let word = match rel.as_str() {
            "with" => "with",
            "over" => "over",
            "above" => "above",
            _ => "left of",
        };
        kinds.push(Kind {
            id: format!("compound:{rel}:{a}|{b}"),
            layer: Layer::Compounds,
            name: format!("{} {word} {}", name_of(&a), name_of(&b)),
            insts,
            params: vec![],
            example: 0,
            relation: Some(rel),
            parts: vec![a, b],
        });
    }

    // Coverage.
    let mut covered: Vec<[BTreeSet<usize>; 3]> = vec![Default::default(); grids.len()];
    for kind in &kinds {
        let layer = match kind.layer {
            Layer::Objects => 0,
            Layer::Groups => 1,
            Layer::Patterns => 2,
            _ => continue,
        };
        for inst in &kind.insts {
            let p = &preps[inst.level];
            let c = &mut covered[inst.level][layer];
            let cells: Box<dyn Iterator<Item = usize>> = if kind.layer == Layer::Objects {
                Box::new(inst.kept.iter().copied())
            } else {
                Box::new(inst.cells.iter().map(|c| c.0))
            };
            c.extend(cells.filter(|i| p.fg[*i]));
        }
    }
    let levels: Vec<LevelInfo> = grids
        .iter()
        .enumerate()
        .map(|(l, g)| {
            let fg = preps[l].fg.iter().filter(|f| **f).count();
            let terrain = preps[l].terrain.iter().filter(|t| **t).count();
            let of = |n: usize, d: usize| {
                if d == 0 {
                    0.0
                } else {
                    round(n as f64 / d as f64)
                }
            };
            let share = |n: usize| of(n, fg);
            let [o, gr, pa] = &covered[l];
            let any: BTreeSet<usize> = o.iter().chain(gr).chain(pa).copied().collect();
            LevelInfo {
                name: g.name.clone(),
                levels: g.levels.clone(),
                layout: g.layout.clone(),
                size: [g.width, g.height],
                tile_size: g.tile_size,
                cells: fg,
                terrain,
                objects: g.objects.len(),
                spawns: g.spawns.len(),
                coverage: Coverage {
                    objects: share(o.len()),
                    groups: of(gr.len(), fg - terrain),
                    patterns: share(pa.len()),
                    any: share(any.len()),
                },
            }
        })
        .collect();

    // Order: by layer, then commonest.
    // `units` index into `kinds`, so the order is kept apart.
    let mut order: Vec<usize> = (0..kinds.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&kinds[a], &kinds[b]);
        // Patterns of one value (plain floor) after the mixed ones.
        let plain = |k: &Kind| {
            k.layer == Layer::Patterns && {
                let mut v = k.insts[0].cells.iter().map(|c| c.1);
                let first = v.next();
                v.all(|x| Some(x) == first)
            }
        };
        a.layer
            .cmp(&b.layer)
            .then(plain(a).cmp(&plain(b)))
            .then(b.insts.len().cmp(&a.insts.len()))
            .then(a.id.cmp(&b.id))
    });
    let mut tested: BTreeMap<Layer, usize> = BTreeMap::new();
    let mut layers = Layers::default();
    let mut chunks = Vec::with_capacity(kinds.len());
    for kind in order.iter().map(|&k| &kinds[k]) {
        match kind.layer {
            Layer::Objects => layers.objects += 1,
            Layer::Groups => layers.groups += 1,
            Layer::Gaps => layers.gaps += 1,
            Layer::Patterns => layers.patterns += 1,
            Layer::Compounds => layers.compounds += 1,
        }
        let ex = &kind.insts[kind.example];
        let g = &grids[ex.level];
        let mut function = function_of(kind, grids, &preps, interactive, &units, &kinds);
        // Pieces only: an object wider than two screens sets the terrain.
        if kind.layer != Layer::Patterns
            && ex.size()[0] <= 32
            && let Some(t) = tester
        {
            let n = tested.entry(kind.layer).or_default();
            if *n < opts.reach_tests {
                *n += 1;
                let cells: Vec<(usize, usize, Tile)> = ex
                    .cells
                    .iter()
                    .map(|&(i, _)| (i % g.width, i / g.width, g.class[i]))
                    .collect();
                let gap = if kind.layer == Layer::Gaps {
                    Some((ex.x0, ex.x1))
                } else {
                    kind.parts
                        .iter()
                        .find(|p| p.starts_with("gap:"))
                        .and_then(|_| {
                            // The gap's columns: the compound's cells empty at
                            // the bottom row.
                            let y = g.height - 1;
                            let xs: Vec<usize> = ex
                                .cells
                                .iter()
                                .filter(|(i, _)| i / g.width == y && !g.standable(*i))
                                .map(|(i, _)| i % g.width)
                                .collect();
                            Some((*xs.iter().min()?, *xs.iter().max()?))
                        })
                };
                function.demand = t.test(g, &cells, gap, ground(g, &preps[ex.level], ex));
            }
        }
        let mut by_level: BTreeMap<String, usize> = BTreeMap::new();
        for i in &kind.insts {
            *by_level.entry(grids[i.level].name.clone()).or_default() += 1;
        }
        chunks.push(Chunk {
            id: kind.id.clone(),
            layer: kind.layer,
            name: kind.name.clone(),
            size: ex.size(),
            shape: if kind.layer == Layer::Compounds {
                // Both parts' cells, and what lies between them.
                let mut s = shape_of(g, ex);
                for (dy, row) in s.iter_mut().enumerate() {
                    for (dx, c) in row.iter_mut().enumerate() {
                        let i = g.at(ex.x0 + dx, ex.y0 + dy);
                        if c.is_none() && preps[ex.level].fg[i] {
                            *c = Some(g.ids[i]);
                        }
                    }
                }
                s
            } else {
                shape_of(g, ex)
            },
            params: kind.params.clone(),
            count: kind.insts.len(),
            by_level,
            relation: kind.relation.clone(),
            parts: kind.parts.clone(),
            function,
            examples: kind
                .insts
                .iter()
                .take(5)
                .map(|i| Example {
                    level: grids[i.level].name.clone(),
                    x: i.x0,
                    y: i.y0,
                })
                .collect(),
        });
    }
    Mined {
        chunks,
        levels,
        layers,
        controls,
        notes,
    }
}

fn round(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// Whether an entity placed at `x`, `y` (pixels) is on or near an
/// occurrence: within a cell of it on either side, up to three cells above
/// or one below.
fn near(g: &LevelGrid, a: &Inst, x: i64, y: i64) -> bool {
    let ts = g.tile_size.max(1);
    let (cx, cy) = (x.div_euclid(ts), y.div_euclid(ts));
    cx >= a.x0 as i64 - 1 && cx <= a.x1 as i64 + 1 && cy >= a.y0 as i64 - 3 && cy <= a.y1 as i64 + 1
}

/// Whether an occurrence rests on something: a standable cell under its
/// bottom row that isn't its own, or the level's bottom edge.
fn ground(g: &LevelGrid, p: &Prep, a: &Inst) -> bool {
    if a.y1 + 1 >= g.height {
        return true;
    }
    let own: BTreeSet<usize> = a.cells.iter().map(|c| c.0).collect();
    (a.x0..=a.x1).any(|x| {
        let i = g.at(x, a.y1 + 1);
        !own.contains(&i) && p.fg[i] && g.standable(i)
    })
}

fn third(v: usize, n: usize) -> usize {
    (v * 3).checked_div(n).map_or(0, |t| t.min(2))
}

fn function_of(
    kind: &Kind,
    grids: &[LevelGrid],
    preps: &[Prep],
    interactive: &[Interactive],
    units: &[Vec<(usize, usize)>],
    kinds: &[Kind],
) -> Function {
    let ex = &kind.insts[kind.example];
    let g = &grids[ex.level];
    let mut f = Function::default();
    let bg = g.background();
    for &(i, v) in &ex.cells {
        *f.classes
            .entry(class_name(g.class[i]).to_owned())
            .or_default() += 1;
        for k in g.kinds.get(&v).into_iter().flatten() {
            *f.tile_kinds.entry(k.clone()).or_default() += 1;
        }
        for k in g.cell_kinds.get(&i).into_iter().flatten() {
            *f.tile_kinds.entry(k.clone()).or_default() += 1;
        }
        // Empty space's tiles would match every kind that appears in it.
        if let Some(t) = g.tiles.get(&v)
            && g.tiles.get(&bg) != Some(t)
        {
            for it in interactive.iter().filter(|it| it.before == *t) {
                *f.interactive.entry(it.kind.clone()).or_default() += 1;
            }
        }
    }
    let mut near_count: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for inst in &kind.insts {
        let g = &grids[inst.level];
        let p = &preps[inst.level];
        let mut here: BTreeMap<&str, usize> = BTreeMap::new();
        for s in &g.spawns {
            if near(g, inst, s.x, s.y) {
                *here.entry(&s.entity).or_default() += 1;
            }
        }
        for (e, n) in here {
            let c = near_count.entry(e.to_owned()).or_default();
            c.0 += n;
            c.1 += 1;
        }
        if ground(g, p, inst) {
            f.placement.ground += 1;
        } else {
            f.placement.air += 1;
        }
        let (mx, my) = ((inst.x0 + inst.x1) / 2, (inst.y0 + inst.y1) / 2);
        f.placement.rows[third(my, g.height)] += 1;
        let along = if g.across() {
            third(mx, g.width)
        } else {
            third(my, g.height)
        };
        f.placement.segment[along] += 1;
        f.placement.screen[third(mx % 16, 16)] += 1;
        if matches!(kind.layer, Layer::Objects | Layer::Groups | Layer::Gaps) {
            adjacency(
                &mut f.adjacency,
                g,
                p,
                inst,
                &units[inst.level],
                kinds,
                &kind.id,
            );
        }
    }
    let n = kind.insts.len().max(1) as f64;
    let mut enemies: Vec<Near> = near_count
        .into_iter()
        .map(|(entity, (count, with))| Near {
            entity,
            count,
            share: round(with as f64 / n),
        })
        .collect();
    enemies.sort_by(|a, b| b.count.cmp(&a.count).then(a.entity.cmp(&b.entity)));
    f.enemies = enemies;
    f
}

/// The nearest piece on each side of an occurrence: within eight cells
/// left or right with rows in common, or six above or below with columns
/// in common; else the terrain or the level's edge when one touches it,
/// else `none`.
fn adjacency(
    out: &mut Adjacency,
    g: &LevelGrid,
    p: &Prep,
    a: &Inst,
    units: &[(usize, usize)],
    kinds: &[Kind],
    own: &str,
) {
    let mut best: [Option<(usize, &str)>; 4] = [None; 4];
    for &(k, n) in units {
        let b = &kinds[k].insts[n];
        if kinds[k].id == own && b == a {
            continue;
        }
        let over_x = a.x0 <= b.x1 && b.x0 <= a.x1;
        let over_y = a.y0 <= b.y1 && b.y0 <= a.y1;
        let sides = [
            (over_y && b.x1 < a.x0).then(|| a.x0 - b.x1),
            (over_y && b.x0 > a.x1).then(|| b.x0 - a.x1),
            (over_x && b.y1 < a.y0).then(|| a.y0 - b.y1),
            (over_x && b.y0 > a.y1).then(|| b.y0 - a.y1),
        ];
        for (s, d) in sides.into_iter().enumerate() {
            let limit = if s < 2 { 8 } else { 6 };
            if let Some(d) = d
                && d <= limit
                && best[s].is_none_or(|(bd, bid)| (d, kinds[k].id.as_str()) < (bd, bid))
            {
                best[s] = Some((d, kinds[k].id.as_str()));
            }
        }
    }
    let edge = |s: usize| -> String {
        let touches = |i: usize| p.terrain[i];
        let (cells, at_edge): (Vec<usize>, bool) = match s {
            0 if a.x0 == 0 => (vec![], true),
            0 => ((a.y0..=a.y1).map(|y| g.at(a.x0 - 1, y)).collect(), false),
            1 if a.x1 + 1 == g.width => (vec![], true),
            1 => ((a.y0..=a.y1).map(|y| g.at(a.x1 + 1, y)).collect(), false),
            2 if a.y0 == 0 => (vec![], true),
            2 => ((a.x0..=a.x1).map(|x| g.at(x, a.y0 - 1)).collect(), false),
            _ if a.y1 + 1 == g.height => (vec![], true),
            _ => ((a.x0..=a.x1).map(|x| g.at(x, a.y1 + 1)).collect(), false),
        };
        if at_edge {
            "edge".into()
        } else if cells.into_iter().any(touches) {
            "terrain".into()
        } else {
            "none".into()
        }
    };
    let maps = [
        &mut out.left,
        &mut out.right,
        &mut out.above,
        &mut out.below,
    ];
    for (s, m) in maps.into_iter().enumerate() {
        let key = match best[s] {
            Some((_, id)) => id.to_owned(),
            None => edge(s),
        };
        *m.entry(key).or_default() += 1;
    }
}
