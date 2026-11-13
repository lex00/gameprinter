//! What a chunk demands of the player, found by `gp_interp::reach` on a
//! made-up level that holds it: a floor two cells deep the height of the
//! chunk's own level, the chunk's cells where they were (moved down onto
//! the floor when it rests on something), the player on the left and the
//! goal on the right. The model's own movement runs, so the answer is the
//! model's.

use gp_interp::reach::{self, Envelope};
use gp_interp::variant::Sim;
use gp_sim::spec::{Role, Tile};
use gp_sim::{Fx, Spec};

use crate::format::Demand;
use crate::level::LevelGrid;
use crate::mine::Tester;

/// Cells of open floor before and after the chunk.
const RUN_UP: usize = 8;

/// Runs reachability searches with a model's compiled spec, stepped by
/// `sim` (the hooks in this process, or the game's module).
pub struct Reach<'a> {
    pub spec: &'a Spec,
    pub sim: &'a dyn Sim,
    pub envelope: Option<Envelope>,
    /// Why the envelope couldn't be measured, when it couldn't.
    pub why_not: Option<String>,
}

impl<'a> Reach<'a> {
    /// The player's envelope is measured once, on the first level with a
    /// player spawn.
    pub fn new(spec: &'a Spec, sim: &'a dyn Sim) -> Self {
        let mut why_not = None;
        let envelope = (0..spec.levels.len()).find_map(|l| match reach::envelope(spec, l, sim) {
            Ok(e) => Some(e),
            Err(e) => {
                why_not.get_or_insert(e);
                None
            }
        });
        Self {
            spec,
            sim,
            why_not: if envelope.is_none() { why_not } else { None },
            envelope,
        }
    }
}

/// A made-up level: `width` by `height` cells of `tile_size` pixels, a
/// floor in the bottom two rows but for the `gap` columns, and `cells`
/// placed on it.
pub fn mini_level(
    width: usize,
    height: usize,
    cells: &[(usize, usize, Tile)],
    gap: Option<(usize, usize)>,
) -> Vec<Tile> {
    let mut tiles = vec![Tile::Empty; width * height];
    for y in height.saturating_sub(2)..height {
        for x in 0..width {
            if gap.is_none_or(|(a, b)| x < a || x > b) {
                tiles[y * width + x] = Tile::Solid;
            }
        }
    }
    for &(x, y, t) in cells {
        if x < width && y < height {
            tiles[y * width + x] = t;
        }
    }
    tiles
}

impl Tester for Reach<'_> {
    fn test(
        &self,
        grid: &LevelGrid,
        cells: &[(usize, usize, Tile)],
        gap: Option<(usize, usize)>,
        ground: bool,
    ) -> Option<Demand> {
        let level = grid.spec_level?;
        let src = self.spec.levels.get(level)?;
        let player = src
            .spawns
            .iter()
            .position(|s| self.spec.kinds[s.kind].role == Role::Player)?;
        let pk = &self.spec.kinds[src.spawns[player].kind];
        let ts = src.tile_size;
        let height = grid.height.max(4);
        let floor = height - 2;
        let x0 = cells.iter().map(|c| c.0).min()?;
        let x1 = cells.iter().map(|c| c.0).max()?;
        let y1 = cells.iter().map(|c| c.1).max()?;
        // Down onto the floor, when it rests on something above it.
        let dy = if ground && y1 + 1 < floor {
            floor - 1 - y1
        } else {
            0
        };
        let w = x1 - x0 + 1;
        let width = RUN_UP + w + RUN_UP;
        let placed: Vec<(usize, usize, Tile)> = cells
            .iter()
            .map(|&(x, y, t)| (x - x0 + RUN_UP, y + dy, t))
            .collect();
        let gap = gap.map(|(a, b)| (a - x0 + RUN_UP, b - x0 + RUN_UP));
        let tiles = mini_level(width, height, &placed, gap);

        let mut spec = self.spec.clone();
        {
            let l = &mut spec.levels[level];
            l.size = [width as i64, height as i64];
            l.tiles = tiles.clone();
            l.exits.clear();
            l.next = None;
            l.kill_below = Some(Fx::from_int(height as i64 * ts));
            l.goal_x = Some(Fx::from_int((width as i64 - 3) * ts));
            let mut sp = l.spawns[player].clone();
            sp.x = Fx::from_int(2 * ts);
            sp.y = Fx::from_int(floor as i64 * ts - pk.size[1].floor() - 1);
            l.spawns = vec![sp];
            l.origin = [Fx::ZERO; 2];
        }
        let r = reach::reach(&spec, level, self.sim).ok()?;

        // The chunk's shape: the highest wall rising from the floor, and
        // whether it stands in the player's way at the floor.
        let body = (pk.size[1].floor() + ts - 1) / ts;
        let solid = |x: usize, y: usize| {
            let t = tiles[y * width + x];
            t.solid() || matches!(t, Tile::Top | Tile::Ice)
        };
        let mut wall = 0;
        let mut blocks = false;
        for x in RUN_UP..RUN_UP + w {
            let mut h = 0;
            while h < floor && solid(x, floor - 1 - h) {
                h += 1;
            }
            wall = wall.max(h);
            blocks |=
                (1..=body as usize).any(|k| floor >= k && tiles[(floor - k) * width + x].solid());
        }
        // The widest run of columns with nothing at the bottom: a gap cut
        // for a gap, or one the chunk's own empty cells make (a hole).
        let (mut run, mut widest) = (0, 0);
        for x in 0..width {
            if tiles[(height - 1) * width + x] == Tile::Empty {
                run += 1;
                widest = widest.max(run);
            } else {
                run = 0;
            }
        }
        let gap_px = widest as i64 * ts;
        let floor_px = floor as i64 * ts;
        let stands_on_top = r.stands.iter().any(|s| {
            let cx = (s[0] + pk.size[0].floor() / 2) / ts;
            cx >= RUN_UP as i64
                && cx < (RUN_UP + w) as i64
                && s[1] + pk.size[1].floor() < floor_px - 1
        });
        let reachable = r.goal_path.is_some();
        let needs = if !reachable {
            "blocked"
        } else if gap_px > 0 {
            match self.envelope {
                Some(e) if gap_px > e.walk_distance => "running jump",
                _ => "jump",
            }
        } else if blocks {
            "jump"
        } else {
            "walk"
        };
        Some(Demand {
            reachable,
            needs: needs.into(),
            height_px: wall as i64 * ts,
            gap_px,
            stands_on_top,
            moves: r.moves,
        })
    }
}
