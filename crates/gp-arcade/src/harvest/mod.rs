//! The static harvest of a Z80 program (docs/plan-extraction-process.md,
//! stage 0, on an arcade board): what the code settles without running it,
//! and the rest as open facts naming the routine and bytes that decide
//! them. The port of gp-nes's `harvest` to the Z80: the value domain, the
//! guards, the activations and the passes map over; banks, the PPU and the
//! platformer rules (probes, moves, camera, level) stay the NES's; IM 2
//! through the CTC, 16-bit pairs, IX/IY record tables, the alternate
//! registers and the R register are new.
//!
//! [`absint`] reads every routine from the reset and the interrupt
//! handlers with each call followed in its caller's state; [`cond`] turns
//! branch outcomes into conditions on places; [`rules`] turns both into
//! routines and their roles, dispatch, state machines, record tables,
//! counters, timers, the random routine and its callers, constants and
//! model facts, each with the instruction it comes from. The same ROM gives
//! the same output.
//!
//! The output has the NES harvest's shape (gp-cli turns it into
//! gp_nes::harvest::Harvest, so `gp model static`'s JSON and text are the
//! same for both); gp-arcade has no serde, so values are [`Value`].

pub mod absint;
pub mod board;
pub mod cond;
mod rules;
pub mod sem;

use std::collections::{BTreeMap, BTreeSet};

use absint::{Loc, Reader};
pub use board::Board;
use sem::Sem;

/// The shape of the output; the same as gp-nes's, whose fields it fills.
pub const SCHEMA: &str = "gp model static 1";

/// A fact's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Str(String),
    Bool(bool),
    List(Vec<Value>),
}

#[derive(Debug, Clone)]
pub struct Evidence {
    pub routine: String,
    pub pc: String,
    pub insn: String,
}

#[derive(Debug, Clone)]
pub struct Fact {
    pub path: String,
    pub value: Value,
    pub rule: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone)]
pub struct Open {
    pub what: String,
    pub routine: String,
    pub pc: String,
    pub bytes: Vec<String>,
    pub why: String,
}

#[derive(Debug, Clone)]
pub struct RoutineRow {
    pub name: String,
    pub addr: String,
    pub size: u32,
    pub element: String,
    pub why: String,
    pub calls: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Dispatch {
    pub kind: String,
    pub at: String,
    pub subject: String,
    pub cases: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct MapRow {
    pub byte: String,
    pub role: String,
    pub read_by: Vec<String>,
    pub written_by: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub kind: String,
    pub routines: Vec<String>,
    pub mapping: Vec<MapRow>,
}

#[derive(Debug, Clone)]
pub struct State {
    pub value: u8,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Transition {
    pub from: String,
    pub to: String,
    pub at: String,
    pub routine: String,
    pub guards: Vec<String>,
    pub after: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Machine {
    pub byte: String,
    pub name: String,
    pub initial: Option<String>,
    pub path: String,
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
}

#[derive(Debug, Clone)]
pub struct Constant {
    pub routine: String,
    pub pc: String,
    pub insn: String,
    pub value: u8,
    pub what: String,
    pub then: String,
    pub cited: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Harvest {
    pub schema: String,
    pub routines: Vec<RoutineRow>,
    pub dispatch: Vec<Dispatch>,
    pub elements: Vec<Element>,
    pub machines: Vec<Machine>,
    pub facts: Vec<Fact>,
    pub open: Vec<Open>,
    pub constants: Vec<Constant>,
}

/// The code map: a recursive descent from the routine starts, through
/// jumps, branches and calls, and through the jump-table edges the reader
/// found. A routine is the reset, an interrupt handler, a call's or
/// restart's target, or a table's entry; each instruction belongs to the
/// nearest routine at or below it.
#[derive(Debug, Clone, Default)]
pub struct CodeMap {
    /// Each instruction's length.
    pub insns: BTreeMap<u16, u8>,
    /// Routine starts and why each is one.
    pub routines: BTreeMap<u16, &'static str>,
}

impl CodeMap {
    pub fn walk(
        board: &Board,
        handlers: &[u16],
        tables: &BTreeSet<(u16, u16)>,
        called: &BTreeSet<u16>,
    ) -> CodeMap {
        let mut m = CodeMap::default();
        m.routines.insert(0, "reset");
        for &h in handlers {
            m.routines.insert(h, "interrupt handler");
        }
        for &(_, t) in tables {
            m.routines.entry(t).or_insert("jump table entry");
        }
        for &c in called {
            m.routines.entry(c).or_insert("called");
        }
        let mut todo: Vec<u16> = m.routines.keys().copied().collect();
        let mut seen = BTreeSet::new();
        while let Some(pc) = todo.pop() {
            if !board.is_rom(pc) || !seen.insert(pc) {
                continue;
            }
            let b = board.bytes(pc);
            let d = crate::dasm::decode(&b, pc);
            m.insns.insert(pc, d.len);
            let next = pc.wrapping_add(u16::from(d.len));
            match sem::decode(&b, pc) {
                Sem::Call(_, t) | Sem::Rst(t) => {
                    if board.is_rom(t) {
                        m.routines.entry(t).or_insert("called");
                        todo.push(t);
                    }
                }
                Sem::JpInd(_) => {
                    for &(at, t) in tables {
                        if at == pc {
                            todo.push(t);
                        }
                    }
                }
                Sem::Ret(None) | Sem::Reti => {
                    for &(at, t) in tables {
                        if at == pc {
                            todo.push(t);
                        }
                    }
                }
                _ => {}
            }
            if let Some(t) = d.flow.target() {
                todo.push(t);
            }
            if d.flow.falls_through() {
                todo.push(next);
            }
        }
        m
    }

    /// The routine an address is in.
    pub fn routine_of(&self, pc: u16) -> Option<u16> {
        self.routines.range(..=pc).next_back().map(|(a, _)| *a)
    }

    /// A routine's size: up to the end of its last instruction before the
    /// next routine.
    pub fn size(&self, start: u16) -> u32 {
        let end = self
            .routines
            .range(start.saturating_add(1)..)
            .next()
            .map_or(0x1_0000, |(a, _)| u32::from(*a));
        self.insns
            .range(start..)
            .take_while(|(pc, _)| u32::from(**pc) < end)
            .map(|(pc, len)| u32::from(*pc) + u32::from(*len))
            .max()
            .map_or(0, |e| e - u32::from(start))
    }
}

/// Passes before the harvest stops even if a pass still found something
/// new: each can add the handlers or the table entries the one before
/// reached.
const PASSES: usize = 6;

/// The harvest of a board's program.
pub fn harvest(board: &Board) -> Harvest {
    let mut handlers: Vec<(u16, u8)> = Vec::new();
    let mut tables: BTreeSet<(u16, u16)> = BTreeSet::new();
    let mut called: BTreeSet<u16> = BTreeSet::new();
    let mut index: BTreeMap<Loc, BTreeSet<u8>> = BTreeMap::new();
    let mut hints: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    for pass in 0..PASSES {
        let hs: Vec<u16> = handlers.iter().map(|h| h.0).collect();
        let map = CodeMap::walk(board, &hs, &tables, &called);
        let mut r = Reader::new(board);
        r.index_values = index.clone();
        r.hints = hints.clone();
        r.starts = map.routines.keys().copied().collect();
        // The handlers first: what they write, an interrupt can change
        // under the rest of the code.
        r.run_interrupts(&hs);
        r.volatile = r.writes_so_far();
        r.run_reset();
        let starts: Vec<u16> = map.routines.keys().copied().collect();
        r.run_entries(&starts);
        if std::env::var_os("GP_HARVEST_TRACE").is_some() {
            eprintln!(
                "pass {pass}: {} activations, {} routines, {} table edges",
                r.acts.len(),
                map.routines.len(),
                r.tables.len()
            );
        }
        let next_handlers = r.im2_handlers();
        let next_tables = r.tables.clone();
        let mut next_called = called.clone();
        next_called.extend(r.called.iter().copied());
        let next_index = r.index_values_now();
        let next_hints = r.hints.clone();
        if (next_handlers == handlers
            && next_tables == tables
            && next_called == called
            && next_index == index
            && next_hints == hints)
            || pass == PASSES - 1
        {
            let map = CodeMap::walk(board, &hs, &tables, &called);
            let mut h = rules::H::new(board, &map, &r, &handlers);
            rules::run(&mut h);
            let mut out = h.out;
            out.schema = SCHEMA.into();
            return out;
        }
        handlers = next_handlers;
        tables = next_tables;
        called = next_called;
        index = next_index;
        hints = next_hints;
    }
    unreachable!("the last pass returns")
}

#[cfg(test)]
mod tests;

/// Every activation of one pass from the reset and the handlers, as text,
/// for reading by hand while the rules are written.
pub fn dump(board: &Board, handlers: &[u16]) -> String {
    let mut r = Reader::new(board);
    r.run_interrupts(handlers);
    r.volatile = r.writes_so_far();
    r.run_reset();
    let mut out = format!(
        "im {:?} i {:?} vectors {:02X?} handlers {:04X?}\n",
        r.im,
        r.i_reg,
        r.ctc_vectors,
        r.im2_handlers()
    );
    for (i, a) in r.acts.iter().enumerate() {
        out += &format!(
            "act {i} {:04X} general={} returns={} events={} callers={:?}\n",
            a.start,
            a.general,
            a.returns,
            a.events.len(),
            a.callers.iter().map(|c| (c.0, c.1)).collect::<Vec<_>>()
        );
        for e in &a.events {
            if matches!(e, absint::Ev::Indirect { .. } | absint::Ev::Walk { .. }) {
                out += &format!("  {e:?}\n");
            }
        }
    }
    out
}
