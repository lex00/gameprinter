//! The static harvest's routines and the calls between them, over PRG
//! offsets, so a search can ask how far the code a run reached is from a
//! routine it hasn't, or from an instruction inside one ([`Point`]).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use gp_events::Levels;
use gp_nes::codemap::{Board, Mn, Mode, Va, decode, va_add, va_cpu, va_hex, va_parse};
use gp_nes::harvest::Harvest;

/// Routines as PRG offset spans, and who calls whom.
#[derive(Debug, Clone, Default)]
pub struct CallGraph {
    /// Each routine's name, first PRG offset and size, sorted by offset.
    pub routines: Vec<(String, usize, usize)>,
    /// For each routine, the routines it calls.
    pub calls: Vec<BTreeSet<usize>>,
}

impl CallGraph {
    /// The graph of a harvest of `board`.
    pub fn of(board: &Board, h: &Harvest) -> CallGraph {
        let mut routines: Vec<(String, usize, usize)> = h
            .routines
            .iter()
            .filter_map(|r| {
                let o = board.prg_offset(va_parse(&r.addr)?)?;
                Some((r.name.clone(), o, r.size as usize))
            })
            .collect();
        routines.sort_by_key(|r| (r.1, r.0.clone()));
        routines.dedup_by_key(|r| r.1);
        let index: BTreeMap<&str, usize> = routines
            .iter()
            .enumerate()
            .map(|(i, r)| (r.0.as_str(), i))
            .collect();
        let mut calls = vec![BTreeSet::new(); routines.len()];
        for r in &h.routines {
            let Some(&i) = index.get(r.name.as_str()) else {
                continue;
            };
            for c in &r.calls {
                if let Some(&j) = index.get(c.as_str()) {
                    calls[i].insert(j);
                }
            }
        }
        CallGraph { routines, calls }
    }

    /// The routine whose span holds PRG offset `o`.
    pub fn at(&self, o: usize) -> Option<usize> {
        let i = self.routines.partition_point(|r| r.1 <= o).checked_sub(1)?;
        (o < self.routines[i].1 + self.routines[i].2).then_some(i)
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.routines.iter().position(|r| r.0 == name)
    }

    /// The routines `run` ran an instruction of.
    pub fn ran(&self, run: &Levels) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for (w, &word) in run.footprint.starts.words().iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let o = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if let Some(r) = self.at(o) {
                    out.insert(r);
                }
            }
        }
        out
    }

    /// For each routine, the fewest calls from it to one of `targets`.
    pub fn distances_to(&self, targets: &BTreeSet<usize>) -> Vec<Option<u32>> {
        let mut callers = vec![Vec::new(); self.routines.len()];
        for (i, cs) in self.calls.iter().enumerate() {
            for &j in cs {
                callers[j].push(i);
            }
        }
        let mut d = vec![None; self.routines.len()];
        let mut q = VecDeque::new();
        for &t in targets {
            d[t] = Some(0);
            q.push_back(t);
        }
        while let Some(x) = q.pop_front() {
            let dx = d[x].expect("set");
            for &c in &callers[x] {
                if d[c].is_none() {
                    d[c] = Some(dx + 1);
                    q.push_back(c);
                }
            }
        }
        d
    }
}

/// A target inside a routine: the instruction at a banked address, reached
/// when a run starts it (its PRG offset among the run's instruction
/// starts).
#[derive(Debug, Clone)]
pub struct Point {
    pub at: Va,
    /// Its PRG offset.
    pub o: usize,
    /// The routine it is in.
    pub routine: usize,
    /// The routine's entry, as a PRG offset.
    pub entry: usize,
    /// Each instruction of the routine (by PRG offset): its fewest
    /// conditional branch ways to the point without a call or a return.
    pub local: BTreeMap<usize, u32>,
    /// Each routine's fewest calls to `routine`.
    pub dist: Vec<Option<u32>>,
}

impl Point {
    /// The point at `at`, when an instruction of the routine holding it
    /// starts there: the routine's span read from its entry one
    /// instruction after another, as `gp code listing` lists it, with each
    /// instruction's jumps, branches and fall-through (a call falls
    /// through) within the span.
    pub fn of(board: &Board, graph: &CallGraph, at: Va) -> Option<Point> {
        let o = board.prg_offset(at)?;
        let routine = graph.at(o)?;
        let (name, ro, n) = &graph.routines[routine];
        let entry_va = va_parse(name.trim_start_matches("sub_").replace('_', ":").as_str())?;
        let span = *ro..ro + n;
        // Each instruction's ways in: (from, conditional).
        let mut into: BTreeMap<usize, Vec<(usize, u32)>> = BTreeMap::new();
        let mut seen = BTreeSet::new();
        let mut p = entry_va;
        while let Some(po) = board.prg_offset(p).filter(|x| span.contains(x)) {
            seen.insert(po);
            let Some((mn, mode)) = decode(board.read(p)) else {
                p = va_add(p, 1);
                continue;
            };
            let byte = |d: u16| board.read(va_add(p, d));
            let next = va_add(p, mode.len());
            let mut to = |t: Va, w: u32| {
                if let Some(t_o) = board.prg_offset(t).filter(|x| span.contains(x)) {
                    into.entry(t_o).or_default().push((po, w));
                }
            };
            match (mn, mode) {
                (Mn::Jmp, Mode::Abs) => {
                    to(board.target(p, u16::from_le_bytes([byte(1), byte(2)])), 0);
                }
                (Mn::Jmp, _) | (Mn::Rts | Mn::Rti | Mn::Brk, _) => {}
                (_, Mode::Rel) => {
                    let t = va_cpu(p).wrapping_add(2).wrapping_add(byte(1) as i8 as u16);
                    to(board.target(p, t), 1);
                    to(next, 1);
                }
                _ => to(next, 0),
            }
            if u32::from(va_cpu(p)) + u32::from(mode.len()) > 0xFFFF {
                break;
            }
            p = next;
        }
        if !seen.contains(&o) {
            return None;
        }
        // A 0-1 search backwards from the point.
        let mut local: BTreeMap<usize, u32> = BTreeMap::from([(o, 0)]);
        let mut q = VecDeque::from([o]);
        while let Some(x) = q.pop_front() {
            let dx = local[&x];
            for &(p, w) in into.get(&x).map_or(&[][..], |v| v.as_slice()) {
                let d = dx + w;
                if local.get(&p).is_none_or(|&old| d < old) {
                    local.insert(p, d);
                    if w == 0 {
                        q.push_front(p);
                    } else {
                        q.push_back(p);
                    }
                }
            }
        }
        Some(Point {
            at,
            o,
            routine,
            entry: *ro,
            local,
            dist: graph.distances_to(&BTreeSet::from([routine])),
        })
    }

    /// `$C123 in sub_C100`, `$03:8123 in sub_03_8100`.
    pub fn name(&self, g: &CallGraph) -> String {
        format!("${} in {}", va_hex(self.at), g.routines[self.routine].0)
    }

    pub fn reached(&self, run: &Levels) -> bool {
        run.footprint.starts.contains(self.o)
    }

    /// How far `run` got from it, `ran` being the routines it ran: 0 when
    /// it ran it; else the fewest branch ways from an instruction the run
    /// started (at least 1), or the calls from a routine it ran to this
    /// one's plus the branch ways from its entry.
    pub fn distance(&self, ran: &BTreeSet<usize>, run: &Levels) -> u32 {
        if self.reached(run) {
            return 0;
        }
        let near = self
            .local
            .iter()
            .filter(|(o, _)| run.footprint.starts.contains(**o))
            .map(|(_, &d)| d.max(1))
            .min();
        let entry = self.local.get(&self.entry).copied().unwrap_or(0);
        let far = ran
            .iter()
            .filter(|&&r| r != self.routine)
            .filter_map(|&r| self.dist[r])
            .min()
            .map(|d| d + entry.max(1));
        near.into_iter().chain(far).min().unwrap_or(u32::MAX)
    }
}
