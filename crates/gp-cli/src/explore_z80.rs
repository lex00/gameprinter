//! `gp explore`'s search on an arcade romset: presses from gp-explore's
//! arcade palette (the stick and the tap), run on gp-arcade from the end
//! of each start clip with its levels watched (gp_arcade::levels), ranked
//! as the NES search ranks them (crate::explore): with targets, by the
//! fewest calls from a routine a run ran to a target the runs haven't
//! reached, then by new code; with none, by new code alone. New code is
//! the instruction starts of either Z80 and the conditional branches'
//! directions no run had before.
//!
//! The routines and calls are the main Z80's code map
//! ([`crate::code_z80::CodeMap`]), read from the ROM and the start clips'
//! runs, since a run is what finds a `jp (hl)`'s targets. A routine is
//! named `sub_XXXX` by its entry.
//!
//! A `--target` may also be an address inside a routine (`0BC3`, a branch
//! way): it is reached when the main CPU starts the instruction there,
//! which the watched run already records (its instruction starts), so
//! aiming at one costs no speed. A run nearer it ranks first: inside its
//! routine, by the fewest conditional branch ways from an instruction the
//! run started to the address; from elsewhere, by the calls to its routine
//! plus the branch ways from the routine's entry. `--aim unrun` aims at the routines no
//! start clip ran; `--aim stops` at the routines a `jp (hl)`, `jp (ix)`
//! or `jp (iy)` the start clips never took sits in (the Z80's counterpart
//! of the 6502 reader's stops: places the reading of the code alone can't
//! go past). `--aim unread` is the 6502 static reader's report, which has
//! no Z80 counterpart: the code map reads every routine it reaches, so
//! there is nothing it leaves unread but what `stops` aims at.
//!
//! The search runs candidates in batches of eight on threads and judges
//! them in order, so the same seed gives the same clips on any machine.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anyhow::{Context as _, Result, bail};
use gp_arcade::dasm::Flow;
use gp_arcade::levels::{EdgeKind, Levels, Watcher};
use gp_arcade::{Controls, Romset, Tapper};
use gp_explore::presses::Rng;
use gp_explore::{Drive, Plan};

use crate::code_z80::CodeMap;
use crate::explore::ExploreArgs;
use crate::reach::ClipInput;

/// Candidates run together, judged in order.
const BATCH: usize = 8;
/// Kept candidates parents are drawn from, as gp-explore's search does.
const PARENTS: usize = 8;

/// The main Z80's routines and the calls and jumps between them.
#[derive(Debug, Clone)]
pub(crate) struct Graph {
    pub entries: Vec<u16>,
    /// Routines each one transfers to (a call, a restart, a jump).
    pub to: Vec<BTreeSet<usize>>,
    /// Each `jp (..)`'s address, by routine.
    pub indirect: Vec<Vec<u16>>,
}

impl Graph {
    pub fn of(map: &CodeMap) -> Graph {
        let entries: Vec<u16> = map.routines.keys().copied().collect();
        let mut g = Graph {
            to: vec![BTreeSet::new(); entries.len()],
            indirect: vec![Vec::new(); entries.len()],
            entries,
        };
        for (pc, i) in &map.insns {
            let Some(r) = g.at(*pc) else { continue };
            match i.flow {
                Flow::Call(t)
                | Flow::CallIf(t)
                | Flow::Rst(t)
                | Flow::Jump(t)
                | Flow::JumpIf(t)
                | Flow::Djnz(t) => {
                    if let Some(to) = g.at(t).filter(|&to| to != r) {
                        g.to[r].insert(to);
                    }
                }
                Flow::Indirect(_) => g.indirect[r].push(*pc),
                _ => {}
            }
        }
        g
    }

    /// The routine an address is in: the nearest entry at or below it.
    pub fn at(&self, pc: u16) -> Option<usize> {
        self.entries.partition_point(|&e| e <= pc).checked_sub(1)
    }

    pub fn name(&self, i: usize) -> String {
        format!("sub_{:04X}", self.entries[i])
    }

    /// A routine by name: `sub_5FAC`, `5FAC`, `$5FAC` or `0x5FAC`, an entry
    /// of the code map.
    pub fn index(&self, name: &str) -> Option<usize> {
        let hex = name
            .trim_start_matches("sub_")
            .trim_start_matches("0x")
            .trim_start_matches('$');
        let a = u16::from_str_radix(hex, 16).ok()?;
        self.entries.binary_search(&a).ok()
    }

    /// Each routine's fewest transfers to one of `targets`.
    pub fn distances_to(&self, targets: &BTreeSet<usize>) -> Vec<Option<u32>> {
        let mut from: Vec<Vec<usize>> = vec![Vec::new(); self.entries.len()];
        for (r, tos) in self.to.iter().enumerate() {
            for &t in tos {
                from[t].push(r);
            }
        }
        let mut dist = vec![None; self.entries.len()];
        let mut q = VecDeque::new();
        for &t in targets {
            dist[t] = Some(0);
            q.push_back(t);
        }
        while let Some(r) = q.pop_front() {
            let d = dist[r].unwrap_or(0) + 1;
            for &c in &from[r] {
                if dist[c].is_none() {
                    dist[c] = Some(d);
                    q.push_back(c);
                }
            }
        }
        dist
    }
}

/// A `--target` inside a routine: the main CPU's instruction at `pc`.
#[derive(Debug, Clone)]
pub(crate) struct Point {
    pub pc: u16,
    /// The routine it is in.
    pub routine: usize,
    /// Each instruction's fewest conditional branch ways to `pc` without a
    /// call or a return, by address.
    pub local: BTreeMap<u16, u32>,
    /// Each routine's fewest calls to `routine`.
    pub dist: Vec<Option<u32>>,
}

impl Point {
    pub fn of(map: &CodeMap, graph: &Graph, pc: u16) -> Option<Point> {
        map.insns.get(&pc)?;
        let routine = graph.at(pc)?;
        Some(Point {
            pc,
            routine,
            local: branch_ways_to(map, pc),
            dist: graph.distances_to(&BTreeSet::from([routine])),
        })
    }

    /// `$0BC3 in sub_0B63`.
    pub fn name(&self, g: &Graph) -> String {
        format!("${:04X} in {}", self.pc, g.name(self.routine))
    }

    pub fn reached(&self, run: &Levels) -> bool {
        run.cpus[0].starts.contains_key(&self.pc)
    }

    /// How far `run` got from it: 0 when it ran it; else the fewest branch
    /// ways from an instruction the run started (at least 1), or the calls
    /// from a routine it ran to this one's plus the branch ways from its
    /// entry.
    pub fn distance(&self, g: &Graph, run: &Levels) -> u32 {
        if self.reached(run) {
            return 0;
        }
        let starts = &run.cpus[0].starts;
        let near = starts
            .keys()
            .filter_map(|s| self.local.get(s))
            .min()
            .map(|&d| d.max(1));
        let entry = self
            .local
            .get(&g.entries[self.routine])
            .copied()
            .unwrap_or(0);
        let far = ran(g, run)
            .into_iter()
            .filter(|&r| r != self.routine)
            .filter_map(|r| self.dist[r])
            .min()
            .map(|d| d + entry.max(1));
        near.into_iter().chain(far).min().unwrap_or(u32::MAX)
    }
}

/// Each instruction's fewest conditional branch ways to `pc` along the
/// code map's jumps and fall-throughs (a call counts as falling through to
/// the instruction after it): a 0-1 search backwards from `pc`.
fn branch_ways_to(map: &CodeMap, pc: u16) -> BTreeMap<u16, u32> {
    let mut from: BTreeMap<u16, Vec<(u16, u32)>> = BTreeMap::new();
    for (&at, i) in &map.insns {
        let w = u32::from(i.flow.conditional());
        let next = at.wrapping_add(u16::from(i.len));
        if i.flow.falls_through() {
            from.entry(next).or_default().push((at, w));
        }
        if let Flow::Jump(t) | Flow::JumpIf(t) | Flow::Djnz(t) = i.flow {
            from.entry(t).or_default().push((at, w));
        }
    }
    let mut dist: BTreeMap<u16, u32> = BTreeMap::from([(pc, 0)]);
    let mut q = VecDeque::from([pc]);
    while let Some(x) = q.pop_front() {
        let dx = dist[&x];
        for &(p, w) in from.get(&x).map_or(&[][..], |v| v.as_slice()) {
            let d = dx + w;
            if dist.get(&p).is_none_or(|&old| d < old) {
                dist.insert(p, d);
                if w == 0 {
                    q.push_front(p);
                } else {
                    q.push_back(p);
                }
            }
        }
    }
    dist
}

/// A `--target`: a routine by name or entry, else an instruction inside
/// one by its address.
pub(crate) enum Aim {
    Routine(usize),
    Point(Point),
}

pub(crate) fn aim(map: &CodeMap, g: &Graph, t: &str) -> Result<Aim> {
    if let Some(r) = g.index(t) {
        return Ok(Aim::Routine(r));
    }
    let hex = t
        .trim_start_matches("sub_")
        .trim_start_matches("0x")
        .trim_start_matches('$');
    let pc = u16::from_str_radix(hex, 16)
        .ok()
        .with_context(|| format!("--target {t}: a routine (sub_5FAC) or an address (0BC3)"))?;
    Point::of(map, g, pc).map(Aim::Point).with_context(|| {
        format!(
            "--target {t}: the main Z80's code map has no routine there and no instruction starts at ${pc:04X}"
        )
    })
}

/// What runs have reached: both CPUs' instruction starts, and the
/// directions the main CPU's conditional branches went.
#[derive(Debug, Clone, Default)]
pub(crate) struct Seen {
    pub starts: BTreeSet<(u8, u16)>,
    pub dirs: BTreeSet<(u16, bool)>,
    /// The main CPU's conditional instructions, by address.
    pub conditional: std::sync::Arc<BTreeSet<u16>>,
}

impl Seen {
    pub fn of(map: &CodeMap, known: &Levels) -> Seen {
        let mut s = Seen {
            conditional: std::sync::Arc::new(
                map.insns
                    .iter()
                    .filter(|(_, i)| i.flow.conditional())
                    .map(|(pc, _)| *pc)
                    .collect(),
            ),
            ..Seen::default()
        };
        s.fresh(known);
        s
    }

    /// What `run` reached that none before did, added.
    pub fn fresh(&mut self, run: &Levels) -> u64 {
        let mut n = 0;
        for (c, lv) in run.cpus.iter().enumerate() {
            for pc in lv.starts.keys() {
                n += u64::from(self.starts.insert((c as u8, *pc)));
            }
        }
        for (from, _, kind) in run.cpus[0].edges.keys() {
            if !self.conditional.contains(from) {
                continue;
            }
            let went = !matches!(kind, EdgeKind::Fall);
            n += u64::from(self.dirs.insert((*from, went)));
        }
        n
    }
}

/// The routines a run's main CPU ran.
fn ran(g: &Graph, run: &Levels) -> BTreeSet<usize> {
    run.cpus[0]
        .starts
        .keys()
        .filter_map(|&pc| g.at(pc))
        .collect()
}

/// How a run ranks: `None` drops it; lower is better.
pub(crate) enum Score {
    /// New code only.
    New(Seen),
    /// Toward targets not reached yet.
    Toward {
        graph: Graph,
        targets: BTreeSet<usize>,
        /// Targets inside routines not reached yet.
        points: Vec<Point>,
        reached: Vec<usize>,
        dist: Vec<Option<u32>>,
        best: u32,
        seen: Seen,
    },
}

impl Score {
    /// Toward routines and addresses inside routines.
    pub fn toward(
        graph: Graph,
        targets: BTreeSet<usize>,
        points: Vec<Point>,
        seen: Seen,
        known: &Levels,
    ) -> Score {
        let dist = graph.distances_to(&targets);
        let best = ran(&graph, known)
            .into_iter()
            .filter_map(|r| dist[r])
            .chain(points.iter().map(|p| p.distance(&graph, known)))
            .min()
            .unwrap_or(u32::MAX);
        Score::Toward {
            graph,
            targets,
            points,
            reached: Vec::new(),
            dist,
            best,
            seen,
        }
    }

    fn rank(&mut self, run: &Levels) -> Option<(u32, i64)> {
        match self {
            Score::New(seen) => {
                let n = seen.fresh(run);
                (n > 0).then_some((u32::MAX, -(n as i64)))
            }
            Score::Toward {
                graph,
                targets,
                points,
                reached,
                dist,
                best,
                seen,
            } => {
                let r = ran(graph, run);
                let hit: Vec<usize> = targets.iter().copied().filter(|t| r.contains(t)).collect();
                let d = r
                    .iter()
                    .filter_map(|&x| dist[x])
                    .chain(points.iter().map(|p| p.distance(graph, run)))
                    .min()
                    .unwrap_or(u32::MAX);
                let n = seen.fresh(run);
                let points_hit = points.iter().any(|p| p.reached(run));
                if !hit.is_empty() || points_hit {
                    for t in &hit {
                        targets.remove(t);
                    }
                    reached.extend(hit);
                    points.retain(|p| !p.reached(run));
                    *dist = graph.distances_to(targets);
                    *best = u32::MAX;
                    return Some((0, -(n as i64)));
                }
                if d < *best || n > 0 {
                    *best = (*best).min(d);
                    Some((d, -(n as i64)))
                } else {
                    None
                }
            }
        }
    }

    fn done(&self) -> bool {
        matches!(self, Score::Toward { targets, points, .. } if targets.is_empty() && points.is_empty())
    }
}

/// A kept candidate.
#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub start: usize,
    pub pads: Vec<u8>,
    pub rank: (u32, i64),
    pub levels: Levels,
}

/// What the search tells a watcher as it goes.
pub(crate) enum Progress<'a> {
    Ran {
        ran: usize,
        budget: usize,
        best: Option<(u32, i64)>,
        kept: usize,
    },
    Kept {
        found: &'a Found,
    },
}

/// What a search found.
pub(crate) struct Explored {
    pub start_clips: Vec<ClipInput>,
    pub targets: Vec<String>,
    pub reached: Vec<String>,
    pub kept: Vec<Found>,
}

fn controls(f: &gp_evidence::input_log::InputFrame) -> Result<Controls> {
    let bits = f.buttons_word();
    if bits > 0xff || f.reset {
        bail!("an arcade input frame holds bits 0 to 7 and no reset");
    }
    Ok(Controls::from_bits(bits))
}

/// The machine at the end of a clip's input.
pub(crate) fn run_to_end(set: &Romset, clip: &ClipInput) -> Result<Tapper> {
    let mut t = Tapper::power_on(set);
    for (i, f) in clip.inputs.iter().enumerate() {
        t.step(controls(f).with_context(|| format!("clip {}, frame {}", clip.id, i + 1))?);
    }
    Ok(t)
}

/// A candidate's run from a start, watched.
pub(crate) fn run_from(start: &Tapper, pads: &[u8]) -> Levels {
    let mut t = start.clone();
    let mut w = Watcher::new(false);
    for &p in pads {
        w.step_quiet(&mut t, Controls::from_bits(u32::from(p)));
    }
    w.finish()
}

struct Ranked<'a> {
    score: Score,
    kept: Vec<Found>,
    empty: Vec<u8>,
    starts: usize,
    on: &'a mut dyn FnMut(Progress),
}

impl Plan for Ranked<'_> {
    type Out = Levels;

    fn has_parents(&self) -> bool {
        true
    }

    fn parents(&mut self, rng: &mut Rng) -> (usize, Vec<u8>, Option<Vec<u8>>) {
        if self.kept.is_empty() {
            let s = if self.starts > 1 {
                rng.below(self.starts)
            } else {
                0
            };
            return (s, self.empty.clone(), None);
        }
        let top = self.kept.len().min(PARENTS);
        let p = rng.below(top);
        let m = rng.below(top);
        (
            self.kept[p].start,
            self.kept[p].pads.clone(),
            Some(self.kept[m].pads.clone()),
        )
    }

    fn judge(&mut self, start: usize, pads: Vec<u8>, levels: Levels) -> bool {
        match self.score.rank(&levels) {
            Some(rank) => {
                let at = self.kept.partition_point(|k| k.rank <= rank);
                self.kept.insert(
                    at,
                    Found {
                        start,
                        pads,
                        rank,
                        levels,
                    },
                );
                (self.on)(Progress::Kept {
                    found: &self.kept[at],
                });
                self.score.done()
            }
            None => false,
        }
    }

    fn after_batch(&mut self, ran: usize, budget: usize) {
        (self.on)(Progress::Ran {
            ran,
            budget,
            best: self.kept.first().map(|k| k.rank),
            kept: self.kept.len(),
        });
    }
}

/// The routines `--aim stops` aims at: those holding a `jp (..)` no known
/// run took.
fn stop_routines(g: &Graph, known: &Levels) -> BTreeSet<usize> {
    let taken: BTreeSet<u16> = known.cpus[0]
        .edges
        .keys()
        .filter(|(_, _, k)| *k == EdgeKind::Indirect)
        .map(|(f, _, _)| *f)
        .collect();
    (0..g.entries.len())
        .filter(|&r| g.indirect[r].iter().any(|pc| !taken.contains(pc)))
        .collect()
}

/// The search `a` asks for on `set`, from `clips` whose merged runs are
/// `known`, telling `on` as it goes.
pub(crate) fn explore_with(
    a: &ExploreArgs,
    set: &Romset,
    clips: Vec<ClipInput>,
    known: &Levels,
    on: &mut dyn FnMut(Progress),
) -> Result<Explored> {
    let start_clips: Vec<ClipInput> = if a.from.is_empty() {
        clips
    } else {
        let mut v = Vec::new();
        for id in &a.from {
            let i = clips
                .iter()
                .position(|c| &c.id == id)
                .with_context(|| format!("no start clip {id}"))?;
            v.push(clips[i].clone());
        }
        v
    };
    anyhow::ensure!(!start_clips.is_empty(), "the start set has no clip");
    let map = CodeMap::of(set, 0, Some(&known.cpus[0]));
    let graph = Graph::of(&map);
    let mut points: Vec<Point> = Vec::new();
    let targets: BTreeSet<usize> = if a.new_code {
        BTreeSet::new()
    } else if !a.targets.is_empty() {
        let mut routines = BTreeSet::new();
        for t in &a.targets {
            match aim(&map, &graph, t)? {
                Aim::Routine(r) => {
                    routines.insert(r);
                }
                Aim::Point(p) => points.push(p),
            }
        }
        routines
    } else {
        let ran_known = ran(&graph, known);
        match a.aim.as_str() {
            "unrun" => (0..graph.entries.len())
                .filter(|r| !ran_known.contains(r))
                .collect(),
            "stops" => stop_routines(&graph, known),
            "unread" => bail!(
                "--aim unread aims at what the 6502 static reader left unread; on an arcade romset the Z80 code map reads every routine it reaches, so aim at `unrun` or `stops` (the routines holding a jp (..) no run took)"
            ),
            _ => bail!("--aim is unrun, unread or stops"),
        }
    };
    let target_names: Vec<String> = targets
        .iter()
        .map(|&i| graph.name(i))
        .chain(points.iter().map(|p| p.name(&graph)))
        .collect();
    let starts: Vec<Tapper> = crate::pool::map(
        &start_clips,
        || (),
        |_, _, c| run_to_end(set, c),
        &mut |_| {},
    )
    .into_iter()
    .collect::<Result<_>>()?;
    let seen = Seen::of(&map, known);
    let score = if targets.is_empty() && points.is_empty() {
        Score::New(seen)
    } else {
        Score::toward(graph.clone(), targets.clone(), points.clone(), seen, known)
    };
    let mut plan = Ranked {
        score,
        kept: Vec::new(),
        empty: vec![0u8; a.min_frames.max(1)],
        starts: starts.len(),
        on,
    };
    let d = Drive {
        budget: a.budget,
        batch: BATCH,
        min_frames: a.min_frames,
        max_frames: a.max_frames,
    };
    let mut rng = Rng::new(a.seed);
    gp_explore::drive_with(
        &mut plan,
        &mut rng,
        Vec::new(),
        &d,
        &gp_explore::arcade::PALETTE,
        &mut |_, batch| {
            crate::pool::map(
                batch,
                || (),
                |_, _, (s, p)| Some(run_from(&starts[*s], p)),
                &mut |_| {},
            )
        },
    );
    let mut found = plan.kept;
    found.truncate(a.keep);
    let reached: Vec<String> = {
        let mut r = BTreeSet::new();
        for f in &found {
            let hit = ran(&graph, &f.levels);
            for &t in &targets {
                if hit.contains(&t) {
                    r.insert(graph.name(t));
                }
            }
            for p in points.iter().filter(|p| p.reached(&f.levels)) {
                r.insert(p.name(&graph));
            }
        }
        r.into_iter().collect()
    };
    Ok(Explored {
        start_clips,
        targets: target_names,
        reached,
        kept: found,
    })
}

/// Why a kept clip was kept, in words.
pub(crate) fn why(f: &Found) -> String {
    if f.rank.0 == u32::MAX {
        format!(
            "runs {} instruction starts or branch ways no run had",
            -f.rank.1
        )
    } else if f.rank.0 == 0 {
        "runs a target".to_owned()
    } else {
        format!(
            "{} calls or branch ways from a target, {} new instruction starts or branch ways",
            f.rank.0, -f.rank.1
        )
    }
}

/// The start clips' runs, merged: what the search counts as known.
pub(crate) fn known(set: &Romset, clips: &[ClipInput]) -> Result<Levels> {
    crate::code_z80::run_clips(set, clips, (1, u64::MAX))
}

/// The romset's layout for a `.presses` file.
pub(crate) fn layout(set: &Romset) -> gp_evidence::input_log::ArcadeLayout {
    gp_evidence::input_log::ArcadeLayout {
        system: set.name.to_owned(),
        columns: "CSRLDUT".to_owned(),
    }
}

/// Writes what `e` kept into `d` as a clip set from power-on: clips.toml
/// and each start's explore-start-K.presses, in the arcade layout.
pub(crate) fn write_clips(
    d: &std::path::Path,
    a: &ExploreArgs,
    set: &Romset,
    e: &Explored,
) -> Result<()> {
    std::fs::create_dir_all(d)?;
    let out = d.join("clips.toml");
    if out.exists()
        && let Some(st) = &a.start
        && std::fs::canonicalize(&out)? == std::fs::canonicalize(st)?
    {
        bail!(
            "--write {} would replace the start set; write into another folder",
            d.display()
        );
    }
    let spans = gp_explore::arcade::spans;
    for (k, c) in e.start_clips.iter().enumerate() {
        let pads: Vec<u8> = c.inputs.iter().map(|i| i.pads[0]).collect();
        std::fs::write(
            d.join(format!("explore-start-{k}.presses")),
            crate::clipset::presses_text(&crate::clipset::Presses {
                frames: pads.len() as u64,
                spans: spans(&pads),
                arcade: Some(layout(set)),
            }),
        )?;
    }
    let mut text = String::from("[clipset]\nformat = \"0.3.0\"\ngenerator = \"gp explore\"\n");
    for (i, f) in e.kept.iter().enumerate() {
        let why = why(f);
        let presses = spans(&f.pads)
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!(
            "\n[[clips]]\nid = \"explore.s{}.{i}\"\nname = \"Explored from {}: {}\"\nsource = \"explore\"\npresses = [{presses}]\nframes = {}\nwhy = \"{why}\"\n\n[clips.start]\npresses_from_power_on = \"explore-start-{}.presses\"\nframe = {}\n",
            a.seed,
            e.start_clips[f.start].id,
            why,
            f.pads.len(),
            f.start,
            e.start_clips[f.start].inputs.len(),
        ));
    }
    std::fs::write(out, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routines_are_named_by_entry_and_distances_follow_calls() {
        let g = Graph {
            entries: vec![0x0000, 0x0100, 0x0200],
            to: vec![BTreeSet::from([1]), BTreeSet::from([2]), BTreeSet::new()],
            indirect: vec![vec![], vec![], vec![]],
        };
        assert_eq!(g.at(0x01ff), Some(1));
        assert_eq!(g.index("sub_0200"), Some(2));
        assert_eq!(g.index("$0100"), Some(1));
        assert_eq!(g.index("0150"), None);
        assert_eq!(
            g.distances_to(&BTreeSet::from([2])),
            vec![Some(2), Some(1), Some(0)]
        );
        assert_eq!(g.name(1), "sub_0100");
    }

    /// A made-up program (no game's bytes): `xor a / or a / jr nz,0006 /
    /// jr 0000`, so the `ld (e010),a` at 0006 inside the reset routine never
    /// runs, and the `jr 0000` at 0004 runs every time round. An address
    /// inside a routine is a target of its own, reached only when its
    /// instruction starts.
    #[test]
    fn a_target_inside_a_routine_is_reached_when_its_instruction_runs() {
        let set = gp_arcade::testrom::romset(
            &[
                0xaf, // 0000: xor a
                0xb7, // 0001: or a
                0x20, 0x02, // 0002: jr nz,0006
                0x18, 0xfa, // 0004: jr 0000
                0x32, 0x10, 0xe0, // 0006: ld (e010),a
                0x18, 0xf5, // 0009: jr 0000
            ],
            &[0x18, 0xfe],
        );
        let start = Tapper::power_on(&set);
        let run = run_from(&start, &[0; 3]);
        let map = CodeMap::of(&set, 0, Some(&run.cpus[0]));
        let g = Graph::of(&map);
        assert!(matches!(
            aim(&map, &g, "sub_0000").unwrap(),
            Aim::Routine(0)
        ));
        let Aim::Point(never) = aim(&map, &g, "0006").unwrap() else {
            panic!("0006 is inside the reset routine")
        };
        assert_eq!(never.name(&g), "$0006 in sub_0000");
        // One branch way, `jr nz` taken, from the loop the run ran.
        assert_eq!(never.local.get(&0x0002), Some(&1));
        assert_eq!(never.local.get(&0x0000), Some(&1));
        assert!(!never.reached(&run));
        assert_eq!(never.distance(&g, &run), 1);
        let Aim::Point(every) = aim(&map, &g, "$0004").unwrap() else {
            panic!("0004 is inside the reset routine")
        };
        assert_eq!(every.distance(&g, &run), 0);
        // Not an instruction start: the middle of `jr 0000`.
        assert!(aim(&map, &g, "0005").is_err());

        // A run that starts 0004 reaches that target and ends the search;
        // one aimed at 0006 ranks by its branch way and goes on.
        let seen = Seen::of(&map, &Levels::default());
        let mut score = Score::toward(
            g.clone(),
            BTreeSet::new(),
            vec![every],
            seen.clone(),
            &Levels::default(),
        );
        assert_eq!(score.rank(&run).map(|r| r.0), Some(0));
        assert!(score.done());
        let mut score = Score::toward(g, BTreeSet::new(), vec![never], seen, &Levels::default());
        assert_eq!(score.rank(&run).map(|r| r.0), Some(1));
        assert!(!score.done());
    }
}
