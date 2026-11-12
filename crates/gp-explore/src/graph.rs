//! The static harvest's routines and the calls between them, over PRG
//! offsets, so a search can ask how far the code a run reached is from a
//! routine it hasn't.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use gp_nes::codemap::{Board, va_parse};
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
