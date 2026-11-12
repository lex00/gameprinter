//! A run's L0 and L1 as data: what the store keeps and what runs of one ROM
//! merge into, with no console attached.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::bits::Bits;
use crate::collector::Collector;
use crate::edges::{EdgeKey, EdgeKind, EdgeStat};

/// L0, as data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FootprintLevel {
    pub executed: Bits,
    pub starts: Bits,
    /// For each 1KB page of PRG ROM, the CPU pages it ran at.
    pub windows: Vec<u64>,
    pub ram_written: Bits,
    pub prg_ram_written: Bits,
    pub instructions: u64,
    pub outside_rom: u64,
}

/// One edge, as data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeRow {
    pub kind: EdgeKind,
    pub from: u32,
    pub to: u32,
    #[serde(flatten)]
    pub stat: EdgeStat,
}

/// A run's (or several runs') L0 and L1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Levels {
    pub frames: u64,
    pub footprint: FootprintLevel,
    pub edges: Vec<EdgeRow>,
}

impl Levels {
    pub fn of(c: &Collector, frames: u64) -> Levels {
        let f = &c.footprint;
        Levels {
            frames,
            footprint: FootprintLevel {
                executed: f.executed.clone(),
                starts: f.starts.clone(),
                windows: f.windows.clone(),
                ram_written: f.ram_written.clone(),
                prg_ram_written: f.prg_ram_written.clone(),
                instructions: f.instructions,
                outside_rom: f.outside_rom,
            },
            edges: c
                .edges
                .as_ref()
                .map(|e| {
                    e.table
                        .iter()
                        .map(|(k, s)| EdgeRow {
                            kind: k.0,
                            from: k.1,
                            to: k.2,
                            stat: s.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The edges as a table.
    pub fn table(&self) -> BTreeMap<EdgeKey, EdgeStat> {
        self.edges
            .iter()
            .map(|r| ((r.kind, r.from, r.to), r.stat.clone()))
            .collect()
    }

    /// Adds another run of the same ROM: footprints join, edge counts add,
    /// first frames are this one's where it has the edge.
    pub fn merge(&mut self, o: &Levels) -> Result<(), String> {
        let (a, b) = (&mut self.footprint, &o.footprint);
        a.executed.union(&b.executed)?;
        a.starts.union(&b.starts)?;
        for (x, y) in a.windows.iter_mut().zip(&b.windows) {
            *x |= y;
        }
        a.ram_written.union(&b.ram_written)?;
        a.prg_ram_written.union(&b.prg_ram_written)?;
        a.instructions += b.instructions;
        a.outside_rom += b.outside_rom;
        self.frames += o.frames;
        let mut t = self.table();
        for r in &o.edges {
            match t.get_mut(&(r.kind, r.from, r.to)) {
                Some(e) => {
                    e.count += r.stat.count;
                    e.pointers.extend(r.stat.pointers.iter().copied());
                }
                None => {
                    t.insert((r.kind, r.from, r.to), r.stat.clone());
                }
            }
        }
        self.edges = t
            .into_iter()
            .map(|(k, stat)| EdgeRow {
                kind: k.0,
                from: k.1,
                to: k.2,
                stat,
            })
            .collect();
        Ok(())
    }

    /// The PRG offsets control entered ([`EdgeKind::enters`]).
    pub fn entries(&self) -> std::collections::BTreeSet<u32> {
        self.edges
            .iter()
            .filter(|r| r.kind.enters() && r.to != crate::edges::NO_ROM)
            .map(|r| r.to)
            .collect()
    }
}
