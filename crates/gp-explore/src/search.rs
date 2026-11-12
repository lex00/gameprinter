//! One seeded search: candidates are press sequences mutated from the best
//! kept so far, each run from a start with gp-events watching, ranked by a
//! score. The same ROM, start, score and seed give the same candidates in
//! the same order, on any machine.

use gp_events::{Bits, Levels, RunError};

use crate::graph::CallGraph;
use crate::presses::{Rng, mutate};
use crate::start::{Start, run_from};

/// How the search runs.
#[derive(Debug, Clone)]
pub struct Opts {
    pub seed: u64,
    /// Candidates to run, at most.
    pub budget: usize,
    pub min_frames: usize,
    pub max_frames: usize,
}

/// What a score says of a run: `None` to drop it, or its rank (lower is
/// better), compared as a tuple.
pub trait Score {
    fn rank(&mut self, run: &Levels) -> Option<(u32, i64)>;
    /// Whether the search can stop.
    fn done(&self) -> bool {
        false
    }
}

/// A kept candidate.
#[derive(Debug, Clone)]
pub struct Found {
    /// Which of the search's starts it runs from.
    pub start: usize,
    pub pads: Vec<u8>,
    pub rank: (u32, i64),
    pub levels: Levels,
}

/// What a search plugs into [`drive`]: how it draws parents, how it judges
/// a run, and what it does between batches. [`drive`] owns the loop, the
/// seeded generator's order and the batches, so every search in gp (this
/// crate's, `gp capture dense`, `gp capture steer` and `gp clips matrix`'s)
/// is the same loop with its own plan.
pub trait Plan {
    /// What running one candidate gives.
    type Out;
    /// Whether there is anything to draw parents from yet.
    fn has_parents(&self) -> bool;
    /// Before a batch of new candidates is drawn (a sort, say).
    fn prepare(&mut self) {}
    /// A parent to mutate and, for a splice, a mate: the start the child
    /// runs from, the parent's presses and the mate's. Draws from `rng` in
    /// a fixed order.
    fn parents(&mut self, rng: &mut Rng) -> (usize, Vec<u8>, Option<Vec<u8>>);
    /// Judges one run, in the batch's order; `true` ends the search.
    fn judge(&mut self, start: usize, pads: Vec<u8>, out: Self::Out) -> bool;
    /// After each batch, with the candidates run so far and the budget.
    fn after_batch(&mut self, _ran: usize, _budget: usize) {}
}

/// What [`drive`] runs a batch with: the plan as it stands and the batch,
/// giving one result a candidate, in order.
pub type Runner<'a, P> = dyn FnMut(&P, &[(usize, Vec<u8>)]) -> Vec<Option<<P as Plan>::Out>> + 'a;

/// How [`drive`] runs.
#[derive(Debug, Clone, Copy)]
pub struct Drive {
    /// Candidates run, at most (the seeds count, and are all run).
    pub budget: usize,
    /// Candidates drawn per batch. A batch's results are judged in its
    /// order, so the batch size is part of what a seed gives.
    pub batch: usize,
    pub min_frames: usize,
    pub max_frames: usize,
}

/// The search loop: run `seeds`, then, while the budget lasts and the plan
/// has parents, draw a batch of mutations from the plan's parents, run
/// them with `run` (given the plan as it stands; it may run a batch on
/// threads, but returns results in its order, `None` for a candidate that
/// couldn't run), and judge each
/// in order. Returns the candidates run. The same plan, seeds and `rng`
/// give the same candidates in the same order on any machine.
pub fn drive<P: Plan>(
    plan: &mut P,
    rng: &mut Rng,
    seeds: Vec<(usize, Vec<u8>)>,
    d: &Drive,
    run: &mut Runner<'_, P>,
) -> usize {
    let mut todo = seeds;
    let budget = d.budget.max(todo.len());
    let mut ran = 0usize;
    loop {
        if todo.is_empty() {
            if ran >= budget || !plan.has_parents() {
                return ran;
            }
            plan.prepare();
            for _ in 0..d.batch.min(budget - ran) {
                let (s, p, m) = plan.parents(rng);
                todo.push((s, mutate(rng, &p, m.as_deref(), d.min_frames, d.max_frames)));
            }
        }
        let batch = std::mem::take(&mut todo);
        let out = run(plan, &batch);
        ran += batch.len();
        for ((s, p), o) in batch.into_iter().zip(out) {
            let Some(o) = o else { continue };
            if plan.judge(s, p, o) {
                return ran;
            }
        }
        plan.after_batch(ran, budget);
    }
}

/// Kept candidates parents are drawn from.
const PARENTS: usize = 8;

/// What a search tells a watcher as it goes ([`search_watched`]). It only
/// hears: the same seed gives the same candidates watched or not.
#[derive(Debug)]
pub enum Progress<'a> {
    /// A candidate ran: how many have, of the budget, the best rank kept
    /// and how many are kept.
    Ran {
        ran: usize,
        budget: usize,
        best: Option<(u32, i64)>,
        kept: usize,
    },
    /// A candidate was kept, at this place in rank order.
    Kept { place: usize, found: &'a Found },
}

/// `gp explore`'s plan: one start, candidates kept by a [`Score`] in rank
/// order, parents drawn from the best few, one candidate a batch.
struct Ranked<'a> {
    score: &'a mut dyn Score,
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
            // With one start, nothing to draw: the same seed gives what it
            // gave before there were several.
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
                    place: at,
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

/// Runs the search from `starts` (each candidate from one of them);
/// returns what it kept, best first.
pub fn search(
    rom: &[u8],
    starts: &[Start],
    score: &mut dyn Score,
    o: &Opts,
) -> Result<Vec<Found>, RunError> {
    search_watched(rom, starts, score, o, &mut |_| {})
}

/// [`search`], telling `on` of each candidate run and each kept.
pub fn search_watched(
    rom: &[u8],
    starts: &[Start],
    score: &mut dyn Score,
    o: &Opts,
    on: &mut dyn FnMut(Progress),
) -> Result<Vec<Found>, RunError> {
    let mut rng = Rng::new(o.seed);
    let mut plan = Ranked {
        score,
        kept: Vec::new(),
        empty: vec![0u8; o.min_frames.max(1)],
        starts: starts.len(),
        on,
    };
    let d = Drive {
        budget: o.budget,
        batch: 1,
        min_frames: o.min_frames,
        max_frames: o.max_frames,
    };
    let mut err = None;
    drive(&mut plan, &mut rng, Vec::new(), &d, &mut |_, batch| {
        batch
            .iter()
            .map(|(s, p)| match run_from(rom, &starts[*s], p) {
                Ok(l) => Some(l),
                Err(e) => {
                    err.get_or_insert(e);
                    None
                }
            })
            .collect()
    });
    match err {
        Some(e) => Err(e),
        None => Ok(plan.kept),
    }
}

/// Code no run reached before: a run is kept for the instruction starts it
/// runs that none kept before did.
#[derive(Debug, Clone)]
pub struct NewCode {
    pub seen: Seen,
}

impl NewCode {
    /// Starting from what `known` (the start's own runs, say) reached.
    pub fn new(known: &Levels) -> NewCode {
        NewCode {
            seen: Seen::of(known),
        }
    }
}

/// What runs reached so far: instruction starts and branch directions.
#[derive(Debug, Clone)]
pub struct Seen {
    pub starts: Bits,
    pub dirs: std::collections::BTreeSet<(u32, bool)>,
}

impl Seen {
    pub fn of(known: &Levels) -> Seen {
        let mut s = Seen {
            starts: known.footprint.starts.clone(),
            dirs: Default::default(),
        };
        s.add_dirs(known);
        s
    }

    fn add_dirs(&mut self, run: &Levels) -> u64 {
        let mut n = 0;
        for e in &run.edges {
            let dir = match e.kind {
                gp_events::EdgeKind::Branch => true,
                gp_events::EdgeKind::Fall => false,
                _ => continue,
            };
            if self.dirs.insert((e.from, dir)) {
                n += 1;
            }
        }
        n
    }
}

/// What `run` reached that none before did, adding it to `seen`: new
/// instruction starts and new branch directions (a branch taken or not
/// taken for the first time, the strongest code-only measure found for
/// choosing clips, docs/research/code-steered-clips.md).
fn fresh(seen: &mut Seen, run: &Levels) -> u64 {
    let mut n = 0;
    for (s, r) in seen
        .starts
        .words_mut()
        .iter_mut()
        .zip(run.footprint.starts.words())
    {
        let new = r & !*s;
        n += u64::from(new.count_ones());
        *s |= new;
    }
    n + seen.add_dirs(run)
}

impl Score for NewCode {
    fn rank(&mut self, run: &Levels) -> Option<(u32, i64)> {
        let n = fresh(&mut self.seen, run);
        // No target: the distance is none, `u32::MAX`.
        (n > 0).then_some((u32::MAX, -(n as i64)))
    }
}

/// Toward routines no run reached: a run ranks by the fewest calls from a
/// routine it ran to a target still unreached, then by new code. A target a
/// run reaches is dropped and the distances worked out again; the search
/// is done when every target ran.
#[derive(Debug, Clone)]
pub struct Toward {
    pub graph: CallGraph,
    /// Targets not reached yet.
    pub targets: std::collections::BTreeSet<usize>,
    /// Targets reached, in the order runs reached them.
    pub reached: Vec<usize>,
    dist: Vec<Option<u32>>,
    best: u32,
    seen: Seen,
}

impl Toward {
    pub fn new(
        graph: CallGraph,
        targets: std::collections::BTreeSet<usize>,
        known: &Levels,
    ) -> Toward {
        let dist = graph.distances_to(&targets);
        let mut t = Toward {
            graph,
            targets,
            reached: Vec::new(),
            dist,
            best: u32::MAX,
            seen: Seen::of(known),
        };
        t.best = t.distance(known);
        t
    }

    /// The routines `run` ran.
    fn ran(&self, run: &Levels) -> std::collections::BTreeSet<usize> {
        let mut out = std::collections::BTreeSet::new();
        for (w, &word) in run.footprint.starts.words().iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let o = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if let Some(r) = self.graph.at(o) {
                    out.insert(r);
                }
            }
        }
        out
    }

    /// The fewest calls from a routine `run` ran to a target not reached.
    pub fn distance(&self, run: &Levels) -> u32 {
        self.ran(run)
            .into_iter()
            .filter_map(|r| self.dist[r])
            .min()
            .unwrap_or(u32::MAX)
    }
}

impl Score for Toward {
    fn rank(&mut self, run: &Levels) -> Option<(u32, i64)> {
        let ran = self.ran(run);
        let hit: Vec<usize> = self
            .targets
            .iter()
            .copied()
            .filter(|t| ran.contains(t))
            .collect();
        let d = ran
            .iter()
            .filter_map(|&r| self.dist[r])
            .min()
            .unwrap_or(u32::MAX);
        let n = fresh(&mut self.seen, run);
        if !hit.is_empty() {
            for t in &hit {
                self.targets.remove(t);
            }
            self.reached.extend(hit);
            self.dist = self.graph.distances_to(&self.targets);
            self.best = u32::MAX;
            return Some((0, -(n as i64)));
        }
        if d < self.best || n > 0 {
            self.best = self.best.min(d);
            Some((d, -(n as i64)))
        } else {
            None
        }
    }

    fn done(&self) -> bool {
        self.targets.is_empty()
    }
}
