//! Reachability: which standing places a level's player can get to, and
//! whether the goal is one of them, found by running the player's own
//! movement mechanics in the simulation (research spike,
//! docs/research/mechanics.md).
//!
//! A generated level is playable under a model's mechanics when the goal
//! can be reached from the player's spawn. This module answers that by
//! search: from each place the player stands, it tries a fixed set of
//! moves (walk a step, jump with and without running, held for a short or
//! a long time, steered for part or all of the jump), each one a run of
//! `gp_sim::step_with_ext` with the player alone in the level, and follows every
//! landing it hasn't seen. The moves are the model's, so a change to the
//! jump's numbers changes what's reachable with no code changed here.
//!
//! [`envelope`] measures the jump itself on a flat floor: how high it
//! rises and how far it carries walking and running. That is the
//! mechanic's reachable envelope, the facts a level generator needs.
//!
//! The model's `ext` hooks (`exts`, from crates/gp-exts in gp) run as they
//! do in the interpreter, so a hooked player moves as it plays. Limits, all
//! on purpose for a spike: enemies, exits and state machines other than the
//! game's own are left out (only the player's spawn is kept); a landing is keyed by its whole pixel and
//! restarts with no speed, so momentum carried from one move into the
//! next isn't searched; and the moves are a sample, so "unreachable"
//! means no sampled move gets there.

use std::collections::{BTreeMap, VecDeque};

use gp_sim::spec::{EventKind, Role};
use gp_sim::world::ground_under;
use gp_sim::{Exts, Fx, Spec, World, step_with_ext};

/// The most frames one move may take before it's given up.
pub(crate) const LIMIT: u32 = 600;

/// What [`reach`] found in one level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    pub level: String,
    /// Every place the player stood still, as whole pixels `[x, y]`, in
    /// the order they were found. The first is the spawn.
    pub stands: Vec<[i64; 2]>,
    /// The places from the spawn to the one the goal was reached from,
    /// when it was.
    pub goal_path: Option<Vec<[i64; 2]>>,
    /// Moves that ended with the player dead.
    pub deaths: u64,
    /// Moves simulated, and the frames they took.
    pub moves: u64,
    pub frames: u64,
}

/// How far a jump carries, measured on a flat floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Envelope {
    /// Pixels from the takeoff to the highest point, whole pixels, with
    /// the jump held to its top.
    pub rise: i64,
    /// Frames from takeoff to landing.
    pub airtime: u32,
    /// Pixels carried while holding a direction, walking and (when the
    /// player can run) running. Speed is built up before the takeoff.
    pub walk_distance: i64,
    pub run_distance: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir {
    Left,
    Right,
}

/// One move: hold a direction (with run, perhaps) for `runup` frames,
/// then, for a jump, press it and hold it `hold` frames, holding the
/// direction for `steer` frames after the press (`None`: until landing).
/// A move without a jump holds the direction for `steer` frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Move {
    pub(crate) dir: Option<Dir>,
    pub(crate) run: bool,
    pub(crate) runup: u32,
    pub(crate) jump: Option<u32>,
    pub(crate) steer: Option<u32>,
}

pub(crate) enum End {
    Stand(Fx, Fx),
    Died,
    Goal,
    TimedOut,
}

/// The player's buttons, by the model's actions.
pub(crate) struct Pad {
    pub(crate) left: u8,
    pub(crate) right: u8,
    pub(crate) run: u8,
    pub(crate) jump: u8,
}

pub(crate) fn player_kind(spec: &Spec, level: usize) -> Result<usize, String> {
    spec.levels[level]
        .spawns
        .iter()
        .map(|s| s.kind)
        .find(|k| spec.kinds[*k].role == Role::Player)
        .ok_or(format!(
            "level {}: no player spawn",
            spec.levels[level].name
        ))
}

pub(crate) fn pad(spec: &Spec, kind: usize) -> Result<Pad, String> {
    let k = &spec.kinds[kind];
    let run = k
        .run
        .as_ref()
        .ok_or(format!("{}: the player has no `run`", k.name))?;
    let jump = k
        .jump
        .as_ref()
        .ok_or(format!("{}: the player has no `jump`", k.name))?;
    Ok(Pad {
        left: spec.actions[run.left].mask,
        right: spec.actions[run.right].mask,
        run: run.run.map_or(0, |(_, a)| spec.actions[a].mask),
        jump: spec.actions[jump.action].mask,
    })
}

/// The moves to try from each place, from the player's mechanics: holds
/// only matter for a jump with `gravity_held`, run-ups only for a run with
/// acceleration, and running only when the player can run.
pub(crate) fn moves(spec: &Spec, kind: usize) -> Vec<Move> {
    let k = &spec.kinds[kind];
    let run = k.run.as_ref();
    let jump = k.jump.as_ref();
    let held = jump.is_some_and(|j| {
        j.gravity_held.is_some() || j.bands.iter().any(|b| b.gravity_held.is_some())
    });
    let holds: &[u32] = if held { &[1, 8, LIMIT] } else { &[LIMIT] };
    let runs: &[bool] = if run.is_some_and(|r| r.run.is_some()) {
        &[false, true]
    } else {
        &[false]
    };
    let runups: &[u32] = if run.is_some_and(|r| r.acceleration.is_some()) {
        &[0, 32]
    } else {
        &[0]
    };
    let mut out = Vec::new();
    for &hold in holds {
        out.push(Move {
            dir: None,
            run: false,
            runup: 0,
            jump: Some(hold),
            steer: Some(0),
        });
    }
    for dir in [Dir::Left, Dir::Right] {
        for &run in runs {
            out.push(Move {
                dir: Some(dir),
                run,
                runup: 0,
                jump: None,
                steer: Some(16),
            });
            for &runup in runups {
                for &hold in holds {
                    for steer in [Some(8), Some(16), None] {
                        out.push(Move {
                            dir: Some(dir),
                            run,
                            runup,
                            jump: Some(hold),
                            steer,
                        });
                    }
                }
            }
        }
    }
    out
}

/// Runs one move from the world `w` (the player standing still) and says
/// how it ended. Counts the frames into `frames`.
fn play(
    spec: &Spec,
    exts: &Exts,
    pad: &Pad,
    p: usize,
    mut w: World,
    m: Move,
    frames: &mut u64,
) -> End {
    let mut took_off = false;
    for t in 0..LIMIT {
        let since = t.checked_sub(m.runup);
        let landed = took_off && w.bodies[p].on_ground;
        let dir_on = match (since, m.jump, m.steer) {
            (None, _, _) => true,
            (Some(s), _, Some(n)) => s < n,
            (Some(_), _, None) => !landed,
        };
        let jump_on = matches!((since, m.jump), (Some(s), Some(h)) if s < h && !landed);
        let mut buttons = 0;
        if dir_on {
            buttons |= match m.dir {
                Some(Dir::Left) => pad.left,
                Some(Dir::Right) => pad.right,
                None => 0,
            };
            if m.run && m.dir.is_some() {
                buttons |= pad.run;
            }
        }
        if jump_on {
            buttons |= pad.jump;
        }
        let events = step_with_ext(spec, &mut w, buttons, exts);
        *frames += 1;
        for e in &events {
            if e.entity == Some(p) && e.kind == EventKind::PLAYER_DIED {
                return End::Died;
            }
            if e.entity == Some(p) && e.kind == EventKind::GOAL_REACHED {
                return End::Goal;
            }
        }
        let b = w.bodies[p];
        if m.jump.is_some() && since.is_some() && !b.on_ground {
            took_off = true;
        }
        let done_jumping = m.jump.is_none() || took_off || since.is_some_and(|s| s > 2);
        if since.is_some() && !dir_on && !jump_on && done_jumping && b.on_ground && b.vx == Fx::ZERO
        {
            return End::Stand(b.x, b.y);
        }
    }
    End::TimedOut
}

/// The level alone with its player, at power-on, and the player's index.
fn alone(spec: &Spec, level: usize) -> Result<(Spec, usize, usize), String> {
    let kind = player_kind(spec, level)?;
    let mut s = spec.clone();
    s.start_level = level;
    s.levels[level].spawns.retain(|sp| sp.kind == kind);
    s.levels[level].spawns.truncate(1);
    // The search asks what the player's own moves reach, so a game machine
    // that starts in a state holding the player still (a title screen)
    // starts in the first of its states that lets the player move.
    for m in &mut s.machines {
        let holds = |st: &gp_sim::spec::State| {
            st.freeze || st.part.stops.contains(&gp_sim::spec::System::Player)
        };
        if m.states.get(m.initial).is_some_and(holds)
            && let Some(i) = m.states.iter().position(|st| !holds(st))
        {
            m.initial = i;
        }
    }
    Ok((s, kind, 0))
}

/// Places the player of `w` at `x`, `y`, still, standing if there's ground
/// under it.
fn place(spec: &Spec, w: &mut World, p: usize, x: Fx, y: Fx) {
    let level = &spec.levels[w.level];
    let mut b = w.bodies[p];
    b.x = x;
    b.y = y;
    b.vx = Fx::ZERO;
    b.vy = Fx::ZERO;
    b.band = 0;
    b.buffer = 0;
    b.on_ground = ground_under(level, spec.kind(&b), &b);
    b.air = 0;
    w.bodies[p] = b;
    w.prev = 0;
    w.held = 0;
}

/// Searches `level` (an index into `spec.levels`) for every place its
/// player can stand, from its spawn, by the player's own moves, with the
/// model's hooks `exts` (an empty [`Exts`] for a model without any).
pub fn reach(spec: &Spec, level: usize, exts: &Exts) -> Result<Reach, String> {
    let (spec, kind, p) = alone(spec, level)?;
    let pad = pad(&spec, kind)?;
    let tries = moves(&spec, kind);
    let mut base = World::power_on(&spec);
    let mut frames = 0;
    // Let the player settle on the ground.
    let settle = Move {
        dir: None,
        run: false,
        runup: 0,
        jump: None,
        steer: Some(0),
    };
    let (sx, sy) = match play(&spec, exts, &pad, p, base.clone(), settle, &mut frames) {
        End::Stand(x, y) => (x, y),
        _ => {
            return Err(format!(
                "level {}: the player doesn't come to stand at its spawn",
                spec.levels[level].name
            ));
        }
    };
    place(&spec, &mut base, p, sx, sy);

    let key = |x: Fx, y: Fx| [x.floor(), y.floor()];
    let mut stands = vec![key(sx, sy)];
    let mut at: BTreeMap<[i64; 2], (Fx, Fx, Option<usize>)> = BTreeMap::new();
    at.insert(key(sx, sy), (sx, sy, None));
    let mut queue = VecDeque::from([key(sx, sy)]);
    let (mut deaths, mut count) = (0, 0);
    let mut goal_from = None;
    while let Some(here) = queue.pop_front() {
        let (x, y, _) = at[&here];
        for m in &tries {
            let mut w = base.clone();
            place(&spec, &mut w, p, x, y);
            count += 1;
            match play(&spec, exts, &pad, p, w, *m, &mut frames) {
                End::Stand(nx, ny) => {
                    let k = key(nx, ny);
                    if let std::collections::btree_map::Entry::Vacant(e) = at.entry(k) {
                        let from = stands.iter().position(|s| *s == here);
                        e.insert((nx, ny, from));
                        stands.push(k);
                        queue.push_back(k);
                    }
                }
                End::Died => deaths += 1,
                End::Goal => {
                    if goal_from.is_none() {
                        goal_from = Some(here);
                    }
                }
                End::TimedOut => {}
            }
        }
        if goal_from.is_some() {
            break;
        }
    }
    let goal_path = goal_from.map(|mut k| {
        let mut path = vec![k];
        while let Some(i) = at[&k].2 {
            k = stands[i];
            path.push(k);
        }
        path.reverse();
        path
    });
    Ok(Reach {
        level: spec.levels[level].name.clone(),
        stands,
        goal_path,
        deaths,
        moves: count,
        frames,
    })
}

/// Measures the player's jump on a flat floor, made here: a level of
/// `level`'s tile size, one row of solid cells under a long open row, with
/// no pit and no goal. `exts` are the model's hooks, as for [`reach`].
pub fn envelope(spec: &Spec, level: usize, exts: &Exts) -> Result<Envelope, String> {
    let (mut spec, kind, p) = alone(spec, level)?;
    let pad = pad(&spec, kind)?;
    let ts = spec.levels[level].tile_size;
    let (w, h) = (512, 16);
    let mut tiles = vec![gp_sim::spec::Tile::Empty; (w * h) as usize];
    for c in 0..w {
        tiles[((h - 1) * w + c) as usize] = gp_sim::spec::Tile::Solid;
    }
    {
        let l = &mut spec.levels[level];
        l.size = [w, h];
        l.tiles = tiles;
        l.kill_below = None;
        l.goal_x = None;
        l.spawns[0].x = Fx::from_int(ts * 4);
        l.spawns[0].y = Fx::from_int(0);
    }
    let base = World::power_on(&spec);
    let measure = |run: bool| -> Result<(i64, u32, i64), String> {
        let mut w = base.clone();
        let mut t = 0;
        // Settle, then run up for long enough to reach top speed.
        let mut took = None;
        let (mut x0, mut y0, mut top) = (0, 0, 0);
        while t < 4 * LIMIT {
            let b = w.bodies[p];
            let runup = 64;
            let mut buttons = 0;
            if t >= 16 && took.is_none_or(|_| !b.on_ground) {
                buttons |= pad.right;
                if run {
                    buttons |= pad.run;
                }
            }
            if t >= 16 + runup && (took.is_none() || !b.on_ground) {
                buttons |= pad.jump;
            }
            if took.is_none() && buttons & pad.jump != 0 {
                x0 = b.x.floor();
                y0 = b.y.floor();
                top = y0;
                took = Some(t);
            }
            step_with_ext(&spec, &mut w, buttons, exts);
            t += 1;
            let b = w.bodies[p];
            if let Some(start) = took {
                top = top.min(b.y.floor());
                if b.on_ground && t > start + 1 {
                    return Ok((y0 - top, t - start, b.x.floor() - x0));
                }
            }
        }
        Err("the jump didn't land on the flat floor".into())
    };
    let (rise, airtime, walk_distance) = measure(false)?;
    let run_distance = if pad.run != 0 {
        Some(measure(true)?.2)
    } else {
        None
    };
    Ok(Envelope {
        rise,
        airtime,
        walk_distance,
        run_distance,
    })
}

/// A run through a level with everything in it, found by search: the
/// buttons for each frame from power-on to the goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playthrough {
    pub level: String,
    /// Controller 1's buttons on each frame, from power-on, ending on the
    /// frame the goal is reached.
    pub buttons: Vec<u8>,
    /// The places the player stood on the way, as whole pixels.
    pub stands: Vec<[i64; 2]>,
    /// Moves simulated, frames simulated and places searched.
    pub moves: u64,
    pub frames: u64,
    pub searched: usize,
}

/// The most places [`playthrough`] searches before it gives up.
pub const PLAYTHROUGH_LIMIT: usize = 4000;

/// Whether the game's own state machines freeze the world now: a death,
/// a level end or the game's end.
pub(crate) fn game_frozen(spec: &Spec, w: &World) -> bool {
    w.machines
        .iter()
        .zip(&spec.machines)
        .any(|(m, s)| s.states[m.state].freeze)
}

/// One move played in the whole world, its buttons recorded.
pub(crate) fn play_recorded(
    spec: &Spec,
    exts: &Exts,
    pad: &Pad,
    p: usize,
    w: &mut World,
    m: Move,
    out: &mut Vec<u8>,
) -> End {
    let mut took_off = false;
    for t in 0..LIMIT {
        let since = t.checked_sub(m.runup);
        let landed = took_off && w.bodies[p].on_ground;
        let dir_on = match (since, m.jump, m.steer) {
            (None, _, _) => true,
            (Some(s), _, Some(n)) => s < n,
            (Some(_), _, None) => !landed,
        };
        let jump_on = matches!((since, m.jump), (Some(s), Some(h)) if s < h && !landed);
        let mut buttons = 0;
        if dir_on {
            buttons |= match m.dir {
                Some(Dir::Left) => pad.left,
                Some(Dir::Right) => pad.right,
                None => 0,
            };
            if m.run && m.dir.is_some() {
                buttons |= pad.run;
            }
        }
        if jump_on {
            buttons |= pad.jump;
        }
        let events = step_with_ext(spec, w, buttons, exts);
        out.push(buttons);
        for e in &events {
            if e.entity == Some(p) && e.kind == EventKind::GOAL_REACHED {
                return End::Goal;
            }
            if e.entity == Some(p) && e.kind == EventKind::PLAYER_DIED {
                return End::Died;
            }
        }
        if game_frozen(spec, w) || !w.bodies[p].active() {
            return End::Died;
        }
        let b = w.bodies[p];
        if m.jump.is_some() && since.is_some() && !b.on_ground {
            took_off = true;
        }
        let done_jumping = m.jump.is_none() || took_off || since.is_some_and(|s| s > 2);
        if since.is_some() && !dir_on && !jump_on && done_jumping && b.on_ground && b.vx == Fx::ZERO
        {
            return End::Stand(b.x, b.y);
        }
    }
    End::TimedOut
}

/// Searches for a run through `level` from power-on to its goal, with
/// every entity of the level in it, by the player's own moves (the same
/// moves [`reach`] tries). Breadth first over the places the player comes
/// to stand, each kept with the whole world as it is when the player
/// stands there, so enemies, timers and the game's rules play as they do
/// in a run. The first time a place is reached is the one kept. Returns
/// `None` when no run is found within [`PLAYTHROUGH_LIMIT`] places. The
/// buttons replay to the same run in the interpreter, since the search is
/// the simulation.
pub fn playthrough(spec: &Spec, level: usize, exts: &Exts) -> Result<Option<Playthrough>, String> {
    let mut spec = spec.clone();
    spec.start_level = level;
    let kind = player_kind(&spec, level)?;
    let pad = pad(&spec, kind)?;
    let tries = moves(&spec, kind);
    let mut w = World::power_on(&spec);
    let p = w
        .bodies
        .iter()
        .position(|b| b.kind == kind)
        .ok_or("no player at power-on")?;
    let (mut moves_n, mut frames) = (0u64, 0u64);
    let settle = Move {
        dir: None,
        run: false,
        runup: 0,
        jump: None,
        steer: Some(0),
    };
    let mut first = Vec::new();
    match play_recorded(&spec, exts, &pad, p, &mut w, settle, &mut first) {
        End::Stand(..) => {}
        _ => {
            return Err(format!(
                "level {}: the player doesn't come to stand at its spawn",
                spec.levels[level].name
            ));
        }
    }
    frames += first.len() as u64;
    let key = |w: &World| [w.bodies[p].x.floor(), w.bodies[p].y.floor()];
    // Each place: its world, its buttons so far, and where it came from.
    let mut nodes: Vec<(World, Vec<u8>, Option<usize>)> = vec![(w.clone(), first, None)];
    let mut seen: BTreeMap<[i64; 2], usize> = BTreeMap::new();
    seen.insert(key(&w), 0);
    let mut queue = VecDeque::from([0usize]);
    while let Some(i) = queue.pop_front() {
        for m in &tries {
            let mut w = nodes[i].0.clone();
            let mut buttons = Vec::new();
            moves_n += 1;
            let end = play_recorded(&spec, exts, &pad, p, &mut w, *m, &mut buttons);
            frames += buttons.len() as u64;
            match end {
                End::Goal => {
                    let mut all = nodes[i].1.clone();
                    all.extend(buttons);
                    let mut stands = vec![key(&w)];
                    let mut at = Some(i);
                    while let Some(j) = at {
                        stands.push(key(&nodes[j].0));
                        at = nodes[j].2;
                    }
                    stands.reverse();
                    return Ok(Some(Playthrough {
                        level: spec.levels[level].name.clone(),
                        buttons: all,
                        stands,
                        moves: moves_n,
                        frames,
                        searched: nodes.len(),
                    }));
                }
                End::Stand(..) => {
                    let k = key(&w);
                    if seen.contains_key(&k) || nodes.len() >= PLAYTHROUGH_LIMIT {
                        continue;
                    }
                    let mut all = nodes[i].1.clone();
                    all.extend(buttons);
                    seen.insert(k, nodes.len());
                    queue.push_back(nodes.len());
                    nodes.push((w, all, Some(i)));
                }
                End::Died | End::TimedOut => {}
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
pub(crate) mod tests {
    //! Moss Hop's model (templates/mosshop, our own game) on made-up
    //! levels: none of them is the game's level.

    use super::*;
    use crate::Lifted;
    use crate::compile::compile;
    use gp_model::Model;

    const MODEL: &str = include_str!("../../../templates/mosshop/model.toml");
    const MAP: &str = include_str!("../../../assets/mosshop.toml");

    const METATILES: &str = r#"
schema = 1
asset = "metatiles"

[[metatile]]
tiles = [0, 0, 0, 0]
solid = 0
kind = 0

[[metatile]]
tiles = [1, 1, 1, 1]
solid = 1
kind = 0

[[metatile]]
tiles = [2, 2, 2, 2]
solid = 1
kind = 0

[[metatile]]
tiles = [3, 3, 3, 3]
solid = 1
kind = 0

[[metatile]]
tiles = [4, 4, 4, 4]
solid = 0
kind = 0

[[metatile]]
tiles = [5, 5, 5, 5]
solid = 0
kind = 0

[[metatile]]
tiles = [6, 6, 6, 6]
solid = 1
kind = 2

[[metatile]]
tiles = [7, 7, 7, 7]
solid = 1
kind = 0

[[metatile]]
tiles = [8, 8, 8, 8]
solid = 0
kind = 5
"#;

    const SLUGS: &str = r#"
schema = 1
asset = "slugs"

[[row]]
start = 64
end = 96

[[row]]
start = 160
end = 192
"#;

    /// A 32 by 15 level whose rows 0 to 11 are sky but for `extra`
    /// (`(row, column, char)`) and rows 12 to 14 ground, with a gap of
    /// `gap` columns from column 12.
    pub(crate) fn level(gap: usize, extra: &[(usize, usize, char)]) -> String {
        let mut rows: Vec<Vec<char>> = (0..15)
            .map(|r| {
                let c = match r {
                    12 => 'M',
                    13 | 14 => 'S',
                    _ => '.',
                };
                vec![c; 32]
            })
            .collect();
        for row in rows.iter_mut().skip(12) {
            for c in row.iter_mut().skip(12).take(gap) {
                *c = '.';
            }
        }
        for &(r, c, ch) in extra {
            rows[r][c] = ch;
        }
        let rows: Vec<String> = rows
            .into_iter()
            .map(|r| format!("  \"{}\",", r.into_iter().collect::<String>()))
            .collect();
        format!(
            "schema = 1\nasset = \"level\"\nrows = [\n{}\n]\n",
            rows.join("\n")
        )
    }

    pub(crate) fn spec(level_text: String) -> Spec {
        let model = Model::parse(MODEL).unwrap();
        let mut files = BTreeMap::new();
        files.insert("assets.toml".to_owned(), MAP.to_owned());
        files.insert("metatiles.toml".to_owned(), METATILES.to_owned());
        files.insert("slugs.toml".to_owned(), SLUGS.to_owned());
        files.insert("level.toml".to_owned(), level_text);
        compile(&model, &Lifted { files }).unwrap()
    }

    /// Moss Hop names the same state paths as before mapping schema 5 and
    /// platformer 0.4.7: its slugs patrol, with no hook, words or path, so
    /// its state gains no `ext`, `home_x`, `path_back` or `wait`, and its
    /// recorded traces and hashes stay as they were.
    #[test]
    fn moss_hops_state_names_no_mechanic_words_or_path_state() {
        let s = spec(level(0, &[]));
        for k in &s.kinds {
            assert!(!crate::state::keeps_words(k), "{}", k.name);
            assert!(!crate::state::follows_path(k), "{}", k.name);
        }
        let w = gp_sim::World::power_on(&s);
        let names: Vec<String> = crate::state::leaves(&s, &w)
            .iter()
            .map(|(p, _)| gp_model::path::display(p))
            .collect();
        let added = |n: &str| {
            n.contains(".ext[")
                || [".home_x", ".home_y", ".path_back", ".wait"]
                    .iter()
                    .any(|s| n.ends_with(s))
        };
        assert!(!names.iter().any(|n| added(n)), "{names:?}");
        assert!(names.iter().any(|n| n == "entities.slug[0].x"), "{names:?}");
    }

    #[test]
    fn the_jump_envelope_is_the_models_arc() {
        let e = envelope(&spec(level(0, &[])), 0, &Exts::new()).unwrap();
        // Speed -5 and gravity 0.25 rise 47.5 pixels, from y 176 to 128.5,
        // the top the jump-height baseline records as 128 (docs/model.md).
        assert_eq!(e.rise, 48, "{e:?}");
        assert_eq!(e.airtime, 40, "{e:?}");
        // One and two pixels a frame for the whole airtime.
        assert_eq!(e.walk_distance, i64::from(e.airtime));
        assert_eq!(e.run_distance, Some(2 * i64::from(e.airtime)));
    }

    #[test]
    fn a_flat_level_reaches_the_goal() {
        let r = reach(&spec(level(0, &[])), 0, &Exts::new()).unwrap();
        assert!(r.goal_path.is_some(), "{r:?}");
        assert_eq!(r.stands[0], [24, 176]);
    }

    #[test]
    fn five_columns_of_gap_are_cleared_and_six_arent() {
        // A running jump carries 80 pixels, so five columns (80 pixels)
        // are the widest gap it clears and six aren't cleared.
        let near = reach(&spec(level(5, &[])), 0, &Exts::new()).unwrap();
        assert!(near.goal_path.is_some(), "{near:?}");
        assert!(near.deaths > 0, "walking into the gap falls in");
        let far = reach(&spec(level(6, &[])), 0, &Exts::new()).unwrap();
        assert!(far.goal_path.is_none(), "{:?}", far.stands);
        // Nothing past the gap is reached.
        assert!(far.stands.iter().all(|s| s[0] < 12 * 16));
    }

    #[test]
    fn a_wall_three_blocks_high_is_climbed_and_four_isnt() {
        let wall =
            |n: usize| -> Vec<(usize, usize, char)> { (0..n).map(|i| (11 - i, 16, 'B')).collect() };
        // The jump rises 48 pixels, so a wall 48 high is just climbed: the
        // player stands on its top, 48 pixels above the ground's 176.
        let low = reach(&spec(level(0, &wall(3))), 0, &Exts::new()).unwrap();
        assert!(low.goal_path.is_some(), "{low:?}");
        assert!(low.stands.iter().any(|s| s[1] == 176 - 48), "{low:?}");
        let high = reach(&spec(level(0, &wall(4))), 0, &Exts::new()).unwrap();
        assert!(high.goal_path.is_none(), "{:?}", high.stands);
    }

    /// The search plays the level with its slugs in it, and its buttons
    /// replay to the goal from power-on in the same simulation.
    #[test]
    fn a_playthrough_with_the_slugs_replays_to_the_goal() {
        let spec = spec(level(3, &[]));
        let p = playthrough(&spec, 0, &Exts::new()).unwrap().expect("a run");
        let mut w = World::power_on(&spec);
        let mut goal = false;
        for &b in &p.buttons {
            let events = step_with_ext(&spec, &mut w, b, &Exts::new());
            goal |= events.iter().any(|e| e.kind == EventKind::GOAL_REACHED);
        }
        assert!(goal, "{p:?}");
        assert_eq!(p.stands[0], [24, 176]);
    }

    /// A shell walker in Super Mario Bros.' style (walk, fall, a stomp that
    /// leaves a shell, a kick), made of platformer components with made-up
    /// numbers, added to Moss Hop: the composed model compiles and its
    /// reachability is Moss Hop's, since the player's mechanics didn't
    /// change (docs/research/mechanics.md, "Composition example").
    #[test]
    fn a_composed_enemy_leaves_the_players_reach_alone() {
        const SHELL: &str = r#"
[entities.shell_walker]
role = "enemy"
size = [16, 16]
tile_collision = true
hitbox = { x = 2, y = 4, w = 12, h = 12 }
walk = { speed = "0.5" }
fall = { gravity = "0.25", max_fall = 4 }
contact = { stomp_depth = 7, bounce = -3, keep_on_stomp = true, otherwise = "kill-player" }

[entities.shell_walker.machine]
initial = "walking"

[[entities.shell_walker.machine.states.walking.transition]]
on = "enemy.stomped"
to = "still"

[entities.shell_walker.machine.states.still.set]
walk = { speed = 0 }
contact = { otherwise = "kick" }

[[entities.shell_walker.machine.states.still.transition]]
on = "enemy.kicked"
to = "sliding"

[entities.shell_walker.machine.states.sliding.set]
walk = { speed = 3 }
contact = { otherwise = "kill-player" }

[[levels.main.spawn]]
entity = "shell_walker"
x = 320
y = 176
"#;
        let model = Model::parse(&format!("{MODEL}\n{SHELL}")).unwrap();
        let mut files = BTreeMap::new();
        files.insert("assets.toml".to_owned(), MAP.to_owned());
        files.insert("metatiles.toml".to_owned(), METATILES.to_owned());
        files.insert("slugs.toml".to_owned(), SLUGS.to_owned());
        files.insert("level.toml".to_owned(), level(2, &[]));
        let composed = compile(&model, &Lifted { files }).unwrap();
        assert!(composed.kinds.iter().any(|k| k.name == "shell_walker"));
        let alone = reach(&spec(level(2, &[])), 0, &Exts::new()).unwrap();
        let with = reach(&composed, 0, &Exts::new()).unwrap();
        assert_eq!(with.stands, alone.stands);
        assert!(with.goal_path.is_some());
    }
}
