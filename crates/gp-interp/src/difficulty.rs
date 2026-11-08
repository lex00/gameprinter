//! Difficulty by search: a run through a level found by the player's own
//! moves, and for each input of it the frames it can come earlier or later
//! and still clear (research spike, docs/research/difficulty.md).
//!
//! The search is [`crate::reach::playthrough`]'s, best first by how far
//! right the player stands instead of breadth first, with the whole world
//! in it: enemies, timers and the game's rules. Each move of the run it
//! finds is one input with up to three edges: the jump pressed, the jump
//! let go, the direction let go. For each edge, [`score`] moves it one
//! frame at a time, earlier and later, on the run's own timeline: the
//! variant starts from the world before the first frame it changes, plays
//! its move, waits for the player to stand, then plays the run's next move
//! as the search found it. It passes when the player is then standing no
//! more than a tile behind where the run stood. The frames that pass are
//! the edge's window.
//!
//! A window turns into a chance of making the input by a player whose
//! timing is off by a normal error with a deviation of 2 frames (a
//! placeholder, to be fitted to labels), and the level's score is the sum
//! of `-log2` of those chances in thousandths of a bit: 1000 is one
//! doubling of the expected attempts. All of it is integer arithmetic on
//! the deterministic simulation, so the same model gives the same score
//! on every machine.
//!
//! Limits, on purpose for a spike: the run is the first one the search
//! finds, not the easiest, so a level's score is an upper bound on what
//! its easiest route costs; a variant is judged with the next move as
//! found, so a window is a lower bound; the moves are [`crate::reach`]'s
//! sample, which takes no exits and can't ride lifts or swim.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use gp_sim::spec::EventKind;
use gp_sim::{Exts, Fx, Spec, World, step_with_ext};

use crate::reach::{self, Dir, End, LIMIT, Move, Pad};

/// What [`score`] searches and measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Stop at the first place the player stands at or past this x, in
    /// pixels, instead of the goal.
    pub to_x: Option<i64>,
    /// The most places the search keeps before it gives up.
    pub limit: usize,
    /// The most frames an edge is moved each way.
    pub reach: i64,
    /// How many of the run's later moves a variant must still play.
    pub horizon: usize,
    /// The width of a section of the level map, in pixels.
    pub section: i64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            to_x: None,
            limit: 6000,
            reach: 8,
            horizon: 1,
            section: 256,
        }
    }
}

/// One edge of an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    PressJump,
    LetGoJump,
    LetGoDirection,
}

impl EdgeKind {
    pub fn words(self) -> &'static str {
        match self {
            EdgeKind::PressJump => "press jump",
            EdgeKind::LetGoJump => "let go of jump",
            EdgeKind::LetGoDirection => "let go of the direction",
        }
    }
}

/// An edge and its window: moved by any of `early..=late` frames (`early`
/// at most 0, `late` at least 0) the run still clears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub kind: EdgeKind,
    /// The frame of the move it's on, from the move's first.
    pub at: usize,
    pub early: i64,
    pub late: i64,
    /// Earlier would be before power-on, so it wasn't tried.
    pub early_open: bool,
}

impl Edge {
    /// Frames in the window, with an untried early side counted as `reach`
    /// frames.
    pub fn width(&self, reach: i64) -> i64 {
        let early = if self.early_open { -reach } else { self.early };
        self.late - early + 1
    }
}

/// One move of the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// Its first frame, counted from power-on.
    pub frame: usize,
    /// Where the player stood before it, whole pixels.
    pub x: i64,
    pub y: i64,
    /// What the move is, in words.
    pub what: String,
    /// The model's parts it used: the options it held and the events it
    /// caused, as `run.run`, `jump.bands[1]`, `enemy.stomped goomba`.
    pub parts: Vec<String>,
    pub edges: Vec<Edge>,
    /// The narrowest edge's width, and its cost in thousandths of a bit.
    pub window: Option<i64>,
    pub millibits: u32,
}

/// A stretch of the level, `x0..x1` pixels, and the inputs started in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub x0: i64,
    pub x1: i64,
    pub inputs: usize,
    pub millibits: u32,
    pub tightest: Option<i64>,
}

/// What [`score`] found in one level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difficulty {
    pub level: String,
    /// Whether a run was found to the goal (or `to_x`).
    pub cleared: bool,
    /// The run's buttons from power-on.
    pub buttons: Vec<u8>,
    pub inputs: Vec<Input>,
    pub sections: Vec<Section>,
    /// The level's score: the sum of its inputs'.
    pub millibits: u32,
    /// Places searched, and frames simulated in all.
    pub searched: usize,
    pub frames: u64,
}

/// `-log2` of the chance that a normal timing error with a deviation of 2
/// frames falls in a window of `w` frames (`erf(w / (2 * sqrt(2) * 2))`),
/// in thousandths of a bit, for `w` from 1 to 17.
const MILLIBITS: [u32; 17] = [
    2341, 1385, 871, 551, 342, 207, 120, 67, 36, 18, 9, 4, 2, 1, 0, 0, 0,
];

/// The cost of a window of `w` frames.
pub fn millibits(w: i64) -> u32 {
    if w < 1 {
        return MILLIBITS[0];
    }
    MILLIBITS.get(w as usize - 1).copied().unwrap_or(0)
}

/// A place in the search: the world with the player standing still, the
/// move that got there and its buttons, and the place it came from.
struct Node {
    world: World,
    from: Option<usize>,
    mv: Option<Move>,
    buttons: Vec<u8>,
}

fn key(w: &World, p: usize) -> [i64; 2] {
    [w.bodies[p].x.floor(), w.bodies[p].y.floor()]
}

/// The direction bits a move holds: its direction, and run when it runs.
fn dir_bits(pad: &Pad, mv: &Move) -> (u8, u8) {
    let dir = match mv.dir {
        Some(Dir::Left) => pad.left,
        Some(Dir::Right) => pad.right,
        None => 0,
    };
    (dir, dir | if mv.run && dir != 0 { pad.run } else { 0 })
}

/// The run's buttons `b` with one edge moved by `d` frames: the jump's
/// press at `at` (with its hold, so the span moves), the jump's release at
/// `at`, or the direction's release at `at` (with run). `None` when that
/// would put it before power-on or join it to another press.
fn moved(b: &[u8], pad: &Pad, bits: u8, kind: EdgeKind, at: usize, d: i64) -> Option<Vec<u8>> {
    let mut v = b.to_vec();
    let has =
        |v: &[u8], i: i64, bit: u8| i >= 0 && (i as usize) < v.len() && v[i as usize] & bit != 0;
    let j = pad.jump;
    match kind {
        EdgeKind::PressJump => {
            let rel = (at..b.len()).find(|&i| b[i] & j == 0).unwrap_or(b.len());
            let (a, r) = (at as i64 + d, rel as i64 + d);
            if a < 0 {
                return None;
            }
            for x in &mut v[at..rel] {
                *x &= !j;
            }
            // Pressed earlier, into a jump still held from before: that
            // hold is let go a frame before, as a player must to press
            // again.
            if d < 0 && (has(&v, a - 1, j) || (a..at as i64).any(|i| has(&v, i, j))) {
                let mut i = (at as i64 - 1).min(v.len() as i64 - 1);
                while i >= a - 1 && i >= 0 {
                    v[i as usize] &= !j;
                    i -= 1;
                }
                if !has(&v, a - 2, j) {
                    return None;
                }
            }
            if has(&v, a - 1, j) || has(&v, r, j) || (a..r).any(|i| has(&v, i, j)) {
                return None;
            }
            v.resize(v.len().max(r as usize), 0);
            for x in &mut v[a as usize..r as usize] {
                *x |= j;
            }
        }
        EdgeKind::LetGoJump => {
            let press = (0..at).rev().find(|&i| b[i] & j == 0).map_or(0, |i| i + 1);
            let r = at as i64 + d;
            if r <= press as i64 {
                return None;
            }
            if d > 0 {
                if has(&v, r, j) || (at as i64..r).any(|i| has(&v, i, j)) {
                    return None;
                }
                v.resize(v.len().max(r as usize), 0);
                for x in &mut v[at..r as usize] {
                    *x |= j;
                }
            } else {
                for x in &mut v[r as usize..at] {
                    *x &= !j;
                }
            }
        }
        EdgeKind::LetGoDirection => {
            let dir = bits & !pad.run;
            let first = (0..at)
                .rev()
                .find(|&i| b[i] & dir == 0)
                .map_or(0, |i| i + 1);
            let e = at as i64 + d;
            if dir == 0 || e <= first as i64 {
                return None;
            }
            if d > 0 {
                v.resize(v.len().max(e as usize), 0);
                for x in &mut v[at..e as usize] {
                    *x |= bits;
                }
            } else {
                for x in &mut v[e as usize..at] {
                    *x &= !bits;
                }
            }
        }
    }
    Some(v)
}

/// The edges a move has, as frames of the move: the jump's press and
/// release, and the direction's release.
fn edges_of(buttons: &[u8], pad: &Pad, mv: &Move) -> Vec<(EdgeKind, usize)> {
    let mut out = Vec::new();
    let after = |bit: u8, from: usize| {
        (from..buttons.len())
            .find(|&i| buttons[i] & bit == 0)
            .unwrap_or(buttons.len())
    };
    if mv.jump.is_some()
        && let Some(at) = buttons.iter().position(|b| b & pad.jump != 0)
    {
        out.push((EdgeKind::PressJump, at));
        out.push((EdgeKind::LetGoJump, after(pad.jump, at)));
    }
    let (dir, _) = dir_bits(pad, mv);
    if dir != 0
        && let Some(at) = buttons.iter().position(|b| b & dir != 0)
    {
        out.push((EdgeKind::LetGoDirection, after(dir, at)));
    }
    out
}

/// A move in words.
fn words(mv: &Move) -> String {
    let way = match mv.dir {
        Some(Dir::Left) => " left",
        Some(Dir::Right) => " right",
        None => "",
    };
    let gait = if mv.run { "run" } else { "walk" };
    match (mv.jump, mv.dir) {
        (None, _) => format!("{gait}{way}"),
        (Some(h), None) => {
            if h >= LIMIT {
                "jump in place, held".into()
            } else {
                format!("jump in place, held {h} frames")
            }
        }
        (Some(h), Some(_)) => {
            let held = if h >= LIMIT {
                "held".to_owned()
            } else {
                format!("held {h} frames")
            };
            let up = if mv.runup > 0 {
                format!(" after {} frames", mv.runup)
            } else {
                String::new()
            };
            format!("{gait}{way}, jump{up}, {held}")
        }
    }
}

/// Searches `level` for a run to its goal (or `opts.to_x`) and measures
/// the window of every input of it.
pub fn score(spec: &Spec, level: usize, exts: &Exts, opts: Options) -> Result<Difficulty, String> {
    let mut spec = spec.clone();
    spec.start_level = level;
    let kind = reach::player_kind(&spec, level)?;
    let pad = reach::pad(&spec, kind)?;
    let tries = reach::moves(&spec, kind);
    let name = spec.levels[level].name.clone();
    let mut w = World::power_on(&spec);
    let p = w
        .bodies
        .iter()
        .position(|b| b.kind == kind)
        .ok_or("no player at power-on")?;
    let settle = Move {
        dir: None,
        run: false,
        runup: 0,
        jump: None,
        steer: Some(0),
    };
    let mut first = Vec::new();
    if !matches!(
        reach::play_recorded(&spec, exts, &pad, p, &mut w, settle, &mut first),
        End::Stand(..)
    ) {
        return Err(format!(
            "level {name}: the player doesn't come to stand at its spawn"
        ));
    }
    let mut frames = first.len() as u64;
    let done = |w: &World| opts.to_x.is_some_and(|x| w.bodies[p].x.floor() >= x);

    // Best first: the place furthest right, then the one found first.
    let mut nodes = vec![Node {
        world: w.clone(),
        from: None,
        mv: None,
        buttons: first,
    }];
    let mut seen: BTreeMap<[i64; 2], usize> = BTreeMap::new();
    seen.insert(key(&w, p), 0);
    let mut heap = BinaryHeap::from([(key(&w, p)[0], Reverse(0usize))]);
    let mut end = None;
    'search: while let Some((_, Reverse(i))) = heap.pop() {
        if done(&nodes[i].world) {
            end = Some(i);
            break;
        }
        for m in &tries {
            let mut w = nodes[i].world.clone();
            let mut buttons = Vec::new();
            let e = reach::play_recorded(&spec, exts, &pad, p, &mut w, *m, &mut buttons);
            frames += buttons.len() as u64;
            match e {
                End::Goal => {
                    nodes.push(Node {
                        world: w,
                        from: Some(i),
                        mv: Some(*m),
                        buttons,
                    });
                    end = Some(nodes.len() - 1);
                    break 'search;
                }
                End::Stand(..) => {
                    let k = key(&w, p);
                    if seen.contains_key(&k) || nodes.len() >= opts.limit {
                        continue;
                    }
                    seen.insert(k, nodes.len());
                    heap.push((k[0], Reverse(nodes.len())));
                    nodes.push(Node {
                        world: w,
                        from: Some(i),
                        mv: Some(*m),
                        buttons,
                    });
                }
                End::Died | End::TimedOut => {}
            }
        }
    }
    let searched = nodes.len();
    let width = spec.levels[level].size[0] * spec.levels[level].tile_size;
    let Some(end) = end else {
        return Ok(Difficulty {
            level: name,
            cleared: false,
            buttons: Vec::new(),
            inputs: Vec::new(),
            sections: sections(&[], width, opts.section),
            millibits: 0,
            searched,
            frames,
        });
    };
    let mut path = vec![end];
    while let Some(f) = nodes[*path.last().unwrap()].from {
        path.push(f);
    }
    path.reverse();
    let mut buttons: Vec<u8> = Vec::new();
    for &i in &path {
        buttons.extend(&nodes[i].buttons);
    }

    // What each move used: replay the run and note the events and bands.
    let mut starts = Vec::new();
    let mut t = nodes[path[0]].buttons.len();
    for &i in &path[1..] {
        starts.push(t);
        t += nodes[i].buttons.len();
    }
    let mut parts: Vec<Vec<String>> = vec![Vec::new(); starts.len()];
    {
        let mut w = World::power_on(&spec);
        for (f, &b) in buttons.iter().enumerate() {
            let events = step_with_ext(&spec, &mut w, b, exts);
            let Some(j) = starts.iter().rposition(|&s| s <= f) else {
                continue;
            };
            let mut add = |s: String| {
                if !parts[j].contains(&s) {
                    parts[j].push(s);
                }
            };
            let band = w.bodies[p].band;
            if band > 0
                && spec.kinds[kind]
                    .jump
                    .as_ref()
                    .is_some_and(|j| !j.bands.is_empty())
            {
                add(format!("jump.bands[{}]", band - 1));
            }
            for e in &events {
                if matches!(e.kind, EventKind::LEVEL_STARTED | EventKind::GOAL_REACHED) {
                    continue;
                }
                // Events of an entity only: the game's own (a palette
                // cycle, a timer) say nothing about the input.
                let ev = spec.events.get(e.kind.0).cloned().unwrap_or_default();
                match e.entity {
                    Some(o) if o != p => add(format!("{ev} {}", spec.kinds[w.bodies[o].kind].name)),
                    Some(_) => add(ev),
                    None => {}
                }
            }
        }
    }

    // The windows, on the run's whole timeline: the world before every
    // frame, so a variant starts where it first differs and the frames
    // around it stay as they were (enemies keep their timing).
    let mut snaps = Vec::with_capacity(buttons.len() + 1);
    {
        let mut w = World::power_on(&spec);
        for &b in &buttons {
            snaps.push(w.clone());
            step_with_ext(&spec, &mut w, b, exts);
        }
        snaps.push(w);
    }
    let ends: Vec<usize> = starts
        .iter()
        .zip(&path[1..])
        .map(|(&s, &i)| s + nodes[i].buttons.len())
        .collect();
    let stand_after: Vec<[i64; 2]> = path.iter().map(|&i| key(&nodes[i].world, p)).collect();
    let tile = spec.levels[level].tile_size;
    let mut inputs = Vec::new();
    for j in 0..starts.len() {
        let node = &nodes[path[j + 1]];
        let mv = node.mv.expect("a move");
        let (_, bits) = dir_bits(&pad, &mv);
        let last = (j + opts.horizon).min(starts.len() - 1);
        let ends_run = last == starts.len() - 1;
        let want = stand_after[last + 1];
        // A variant plays its own frames open loop from the first frame it
        // changes, then waits for the player to stand still, then plays the
        // run's next moves as the search found them, each from a stand, as
        // a player who waits for the landing does.
        let mut passes = |v: &[u8], from: usize, until: usize| -> bool {
            let mut w = snaps[from].clone();
            let mut f = from;
            let mut idle = 0;
            loop {
                let b = if f < until {
                    v.get(f).copied().unwrap_or(0)
                } else {
                    0
                };
                if f >= until {
                    let me = w.bodies[p];
                    if me.on_ground && me.vx == Fx::ZERO {
                        break;
                    }
                    idle += 1;
                    if idle > LIMIT {
                        return false;
                    }
                }
                let events = step_with_ext(&spec, &mut w, b, exts);
                frames += 1;
                f += 1;
                for e in &events {
                    if e.entity == Some(p) && e.kind == EventKind::GOAL_REACHED {
                        return true;
                    }
                    if e.entity == Some(p) && e.kind == EventKind::PLAYER_DIED {
                        return false;
                    }
                }
                if reach::game_frozen(&spec, &w) || !w.bodies[p].active() {
                    return false;
                }
            }
            if done(&w) {
                return true;
            }
            for &k in &path[j + 2..=last + 1] {
                let mut b = Vec::new();
                let e = reach::play_recorded(
                    &spec,
                    exts,
                    &pad,
                    p,
                    &mut w,
                    nodes[k].mv.unwrap(),
                    &mut b,
                );
                frames += b.len() as u64;
                match e {
                    End::Goal => return true,
                    End::Stand(..) if done(&w) => return true,
                    End::Stand(..) => {}
                    End::Died | End::TimedOut => return false,
                }
            }
            if ends_run {
                // Stopped short of the goal: the last move again, as a
                // player who stops early walks on.
                let mut b = Vec::new();
                let e = reach::play_recorded(
                    &spec,
                    exts,
                    &pad,
                    p,
                    &mut w,
                    nodes[path[last + 1]].mv.unwrap(),
                    &mut b,
                );
                frames += b.len() as u64;
                return matches!(e, End::Goal) || done(&w);
            }
            // Kept its progress: standing at least as far on, less a tile.
            key(&w, p)[0] >= want[0] - tile
        };
        let mut edges = Vec::new();
        for (kind, at) in edges_of(&node.buttons, &pad, &mv) {
            let g = starts[j] + at;
            let mut try_d = |d: i64| -> Option<bool> {
                // The run up to this move's end: the moves after it are
                // played again from the landing.
                let v = moved(&buttons[..ends[j]], &pad, bits, kind, g, d)?;
                let from = (g as i64 + d.min(0)).max(0) as usize;
                let until = ends[j] + d.max(0) as usize;
                Some(passes(&v, from, until))
            };
            let mut late = 0;
            while late < opts.reach && try_d(late + 1) == Some(true) {
                late += 1;
            }
            let mut early = 0;
            let mut early_open = false;
            while early > -opts.reach {
                match try_d(early - 1) {
                    Some(true) => early -= 1,
                    Some(false) => break,
                    None => {
                        early_open = g as i64 + early - 1 < 0;
                        break;
                    }
                }
            }
            edges.push(Edge {
                kind,
                at,
                early,
                late,
                early_open,
            });
        }
        let window = edges.iter().map(|e| e.width(opts.reach)).min();
        let mut used = Vec::new();
        if mv.run {
            used.push("run.run".to_owned());
        }
        if let Some(h) = mv.jump
            && h < LIMIT
        {
            used.push(format!("jump.gravity_held ({h} frames)"));
        }
        used.extend(parts[j].iter().cloned());
        let [x, y] = stand_after[j];
        inputs.push(Input {
            frame: starts[j],
            x,
            y,
            what: words(&mv),
            parts: used,
            edges,
            window,
            millibits: window.map_or(0, millibits),
        });
    }
    let sections = sections(&inputs, width, opts.section);
    Ok(Difficulty {
        level: name,
        cleared: true,
        buttons,
        millibits: inputs.iter().map(|i| i.millibits).sum(),
        inputs,
        sections,
        searched,
        frames,
    })
}

fn sections(inputs: &[Input], width: i64, step: i64) -> Vec<Section> {
    let step = step.max(1);
    let n = ((width + step - 1) / step).max(1);
    (0..n)
        .map(|s| {
            let (x0, x1) = (s * step, (s + 1) * step);
            let here: Vec<&Input> = inputs.iter().filter(|i| i.x >= x0 && i.x < x1).collect();
            Section {
                x0,
                x1,
                inputs: here.len(),
                millibits: here.iter().map(|i| i.millibits).sum(),
                tightest: here.iter().filter_map(|i| i.window).min(),
            }
        })
        .collect()
}

/// One character a section, darker for harder: a heat strip of the level.
pub fn strip(sections: &[Section]) -> String {
    sections
        .iter()
        .map(|s| match s.millibits {
            0 if s.inputs == 0 => ' ',
            0..100 => '.',
            100..500 => ':',
            500..1000 => '-',
            1000..2000 => '=',
            2000..4000 => '+',
            4000..8000 => '*',
            _ => '#',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Moss Hop's model (templates/mosshop, our own game) on the made-up
    //! levels of reach's tests: none of them is the game's level.

    use super::*;
    use crate::reach::tests::{level, spec};

    fn run(gap: usize) -> Difficulty {
        let d = score(&spec(level(gap, &[])), 0, &Exts::new(), Options::default()).unwrap();
        assert!(d.cleared, "gap {gap}: {d:?}");
        d
    }

    /// Moss Hop's running jump carries 80 pixels, so a gap of five columns
    /// (80 pixels) is cleared only from close to its edge, and a gap of one
    /// from anywhere near it: the wide gap's jump has the narrower window
    /// and the higher score.
    #[test]
    fn a_wider_gap_needs_a_tighter_jump() {
        let easy = run(1);
        let hard = run(5);
        let tightest = |d: &Difficulty| d.inputs.iter().filter_map(|i| i.window).min().unwrap();
        assert!(
            tightest(&hard) < tightest(&easy),
            "{:?}\n{:?}",
            hard.inputs,
            easy.inputs
        );
        assert!(
            hard.millibits > easy.millibits,
            "{} {}",
            hard.millibits,
            easy.millibits
        );
        // The tight input is a jump, and it is in the gap's section.
        let t = hard
            .inputs
            .iter()
            .min_by_key(|i| i.window.unwrap_or(i64::MAX))
            .unwrap();
        assert!(
            t.edges.iter().any(|e| e.kind == EdgeKind::PressJump),
            "{t:?}"
        );
        assert!(t.x < 12 * 16, "{t:?}");
    }

    /// The search is the simulation: its buttons replay to the goal, and
    /// two scores of the same level are the same.
    #[test]
    fn the_run_replays_and_the_score_repeats() {
        let s = spec(level(3, &[]));
        let a = score(&s, 0, &Exts::new(), Options::default()).unwrap();
        let b = score(&s, 0, &Exts::new(), Options::default()).unwrap();
        assert_eq!(a, b);
        let mut w = World::power_on(&s);
        let mut goal = false;
        for &b in &a.buttons {
            goal |= step_with_ext(&s, &mut w, b, &Exts::new())
                .iter()
                .any(|e| e.kind == EventKind::GOAL_REACHED);
        }
        assert!(goal);
        assert_eq!(strip(&a.sections).chars().count(), a.sections.len());
    }

    #[test]
    fn a_window_costs_less_as_it_widens() {
        assert_eq!(millibits(1), 2341);
        assert!((1..17).all(|w| millibits(w) >= millibits(w + 1)));
        assert_eq!(millibits(40), 0);
    }
}
