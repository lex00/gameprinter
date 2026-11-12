//! L1, edges: every transfer of control a run makes, by PRG offset (so by
//! bank), with how many times it was taken and the first frame it was.
//!
//! A transfer is read from the instruction that made it and the one that
//! runs next, so nothing is inferred: a `jmp (ptr)` records the pointer's
//! address and the value it held, an `rts` that doesn't return to just
//! after the `jsr` that made its return address is a jump (a jump engine,
//! or a pushed address), and a branch records the way it went. An NMI or
//! IRQ is an edge from the instruction it followed to its handler.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tetanes_core::bus::Bus;

/// A PRG offset for code that isn't in PRG ROM (RAM, usually).
pub const NO_ROM: u32 = u32::MAX;

/// How control moved.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    Jsr,
    Rts,
    /// An `rts` to somewhere other than just after the `jsr` that pushed
    /// its return address: a jump through a pushed address.
    RtsJump,
    Rti,
    Jmp,
    JmpIndirect,
    /// A conditional branch taken.
    Branch,
    /// A conditional branch not taken.
    Fall,
    /// An NMI or IRQ, from the instruction it followed.
    Interrupt,
    Brk,
    /// A soft reset: from wherever the CPU was to the reset handler.
    Reset,
}

/// Every kind, in declaration order (`kind as u8` indexes it).
const KINDS: [EdgeKind; 11] = [
    EdgeKind::Jsr,
    EdgeKind::Rts,
    EdgeKind::RtsJump,
    EdgeKind::Rti,
    EdgeKind::Jmp,
    EdgeKind::JmpIndirect,
    EdgeKind::Branch,
    EdgeKind::Fall,
    EdgeKind::Interrupt,
    EdgeKind::Brk,
    EdgeKind::Reset,
];

impl EdgeKind {
    /// Whether the target starts code a caller enters (a routine, a
    /// handler, a table's entry), rather than a place within one.
    pub fn enters(self) -> bool {
        matches!(
            self,
            EdgeKind::Jsr
                | EdgeKind::RtsJump
                | EdgeKind::JmpIndirect
                | EdgeKind::Interrupt
                | EdgeKind::Brk
                | EdgeKind::Reset
        )
    }
}

/// One edge's record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeStat {
    pub count: u64,
    /// The first frame it was taken in (frame 1 is the first frame run).
    pub first_frame: u64,
    /// The CPU addresses of its two ends where they first ran.
    pub from_cpu: u16,
    pub to_cpu: u16,
    /// For `jmp (ptr)`: each (pointer address, value) seen.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub pointers: BTreeSet<(u16, u16)>,
}

/// An edge's key: its kind and its two ends' PRG offsets.
pub type EdgeKey = (EdgeKind, u32, u32);

#[derive(Debug, Clone, Copy)]
struct Prev {
    opcode: u8,
    offset: u32,
    pc: u16,
    sp: u8,
    /// For `jmp (ptr)`: the pointer's address and value.
    pointer: Option<(u16, u16)>,
}

/// The edge table of a run, and what it needs to read the next transfer.
#[derive(Debug, Clone)]
pub struct Edges {
    pub table: BTreeMap<EdgeKey, EdgeStat>,
    /// Frames finished.
    pub frames: u64,
    prev: Option<Prev>,
    /// The last instruction run: its PRG offset and CPU address.
    last: (u32, u16),
    /// For each stack position a `jsr` wrote its return address's high
    /// byte to, the CPU address its `rts` returns to.
    returns: [Option<u16>; 256],
    /// Edges counted this frame before they reach [`Edges::table`]: a
    /// direct-mapped cache, so the transfers a loop repeats cost an array
    /// lookup, not a tree insert. Flushed at each frame's end.
    hot: Vec<Hot>,
    /// Keys first added to the table since [`Edges::take_fresh`] was last
    /// called, when tracking is on.
    fresh: Option<Vec<EdgeKey>>,
}

/// A cache slot: an edge counted this frame. `count` 0 is an empty slot.
#[derive(Debug, Clone, Copy, Default)]
struct Hot {
    key: (u8, u32, u32),
    count: u32,
    from_cpu: u16,
    to_cpu: u16,
}

/// Slots in the cache; a power of two.
const HOT: usize = 4096;

fn slot(k: &EdgeKey) -> usize {
    let h = (u64::from(k.1) ^ u64::from(k.2).rotate_left(21) ^ (k.0 as u64).rotate_left(42))
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h >> 52) as usize & (HOT - 1)
}

impl Default for Edges {
    fn default() -> Self {
        Self {
            table: BTreeMap::new(),
            frames: 0,
            prev: None,
            last: (NO_ROM, 0),
            returns: [None; 256],
            hot: vec![Hot::default(); HOT],
            fresh: None,
        }
    }
}

/// Whether an opcode is a conditional branch.
fn is_branch(op: u8) -> bool {
    op & 0x1F == 0x10
}

/// Whether an opcode can move control anywhere but the next instruction.
pub(crate) fn transfers(op: u8) -> bool {
    is_branch(op) || matches!(op, 0x00 | 0x20 | 0x40 | 0x4C | 0x60 | 0x6C)
}

impl Edges {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(
        &mut self,
        kind: EdgeKind,
        from: (u32, u16),
        to: (u32, u16),
        pointer: Option<(u16, u16)>,
    ) {
        let frame = self.frames + 1;
        let key = (kind, from.0, to.0);
        let fresh = || EdgeStat {
            count: 1,
            first_frame: frame,
            from_cpu: from.1,
            to_cpu: to.1,
            pointers: Default::default(),
        };
        if let Some(p) = pointer {
            // Rare enough to go straight to the table, with its pointer.
            let mut e = fresh();
            e.pointers.insert(p);
            self.put(key, e);
            return;
        }
        let i = slot(&key);
        let packed = (kind as u8, from.0, to.0);
        let h = &mut self.hot[i];
        if h.count > 0 && h.key == packed {
            h.count += 1;
            return;
        }
        let old = std::mem::replace(
            h,
            Hot {
                key: packed,
                count: 1,
                from_cpu: from.1,
                to_cpu: to.1,
            },
        );
        if old.count > 0 {
            self.put_hot(old);
        }
    }

    fn put_hot(&mut self, h: Hot) {
        let k = (KINDS[usize::from(h.key.0)], h.key.1, h.key.2);
        let e = EdgeStat {
            count: u64::from(h.count),
            first_frame: self.frames + 1,
            from_cpu: h.from_cpu,
            to_cpu: h.to_cpu,
            pointers: Default::default(),
        };
        self.put(k, e);
    }

    /// Adds `e` to the table under `k`: counts add, the first frame is the
    /// earlier, pointers join.
    fn put(&mut self, k: EdgeKey, e: EdgeStat) {
        match self.table.get_mut(&k) {
            Some(t) => {
                t.count += e.count;
                t.first_frame = t.first_frame.min(e.first_frame);
                t.pointers.extend(e.pointers);
            }
            None => {
                self.table.insert(k, e);
                if let Some(f) = &mut self.fresh {
                    f.push(k);
                }
            }
        }
    }

    /// Turns on tracking of new edges for [`Edges::take_fresh`].
    pub fn track_fresh(&mut self) {
        self.fresh.get_or_insert_with(Vec::new);
    }

    /// The edges first taken since the last call, in key order.
    pub fn take_fresh(&mut self) -> Vec<EdgeKey> {
        let mut v = self.fresh.as_mut().map(std::mem::take).unwrap_or_default();
        v.sort_unstable();
        v
    }

    /// Moves the cache's counts into the table.
    fn flush(&mut self) {
        for i in 0..HOT {
            let h = std::mem::take(&mut self.hot[i]);
            if h.count > 0 {
                self.put_hot(h);
            }
        }
    }

    /// An NMI or IRQ was taken after the last instruction. `here` is the
    /// handler's first instruction, about to run; `resumed` is where the
    /// interrupted code goes on (the address the interrupt pushed) and its
    /// PRG offset, so a transfer the interrupt fell between keeps its edge.
    pub fn interrupt(&mut self, bus: &Bus, here: Option<usize>, resumed: (Option<usize>, u16)) {
        let to = (here.map_or(NO_ROM, |o| o as u32), bus.cpu.pc);
        let from = self.last;
        if let Some(p) = self.prev.take() {
            self.finish(p, (resumed.0.map_or(NO_ROM, |o| o as u32), resumed.1));
        }
        self.add(EdgeKind::Interrupt, from, to, None);
    }

    /// Records the transfer `p` made, now that its target `to` is known.
    fn finish(&mut self, p: Prev, to: (u32, u16)) {
        let from = (p.offset, p.pc);
        let pc = to.1;
        let kind = match p.opcode {
            0x20 => Some(EdgeKind::Jsr),
            0x40 => Some(EdgeKind::Rti),
            0x4C => Some(EdgeKind::Jmp),
            0x6C => Some(EdgeKind::JmpIndirect),
            0x00 => Some(EdgeKind::Brk),
            0x60 => {
                // The return address's high byte sits two above the
                // stack pointer the `rts` saw.
                let slot = usize::from(p.sp.wrapping_add(2));
                let expected = self.returns[slot].take();
                Some(if expected == Some(pc) {
                    EdgeKind::Rts
                } else {
                    EdgeKind::RtsJump
                })
            }
            op if is_branch(op) => Some(if pc == p.pc.wrapping_add(2) {
                EdgeKind::Fall
            } else {
                EdgeKind::Branch
            }),
            _ => None,
        };
        if let Some(k) = kind {
            self.add(k, from, to, p.pointer);
        }
    }

    /// The instruction about to run, at PRG offset `here`, with its opcode.
    #[inline]
    pub fn instruction(&mut self, bus: &Bus, here: Option<usize>, opcode: u8) {
        let pc = bus.cpu.pc;
        self.arrive(here, pc);
        if transfers(opcode) {
            self.begin(bus, opcode, here, pc, bus.cpu.sp);
        }
    }

    /// Control is at `pc` (PRG offset `here`): the target of a transfer
    /// that waits for one.
    #[inline]
    pub fn arrive(&mut self, here: Option<usize>, pc: u16) {
        let off = here.map_or(NO_ROM, |o| o as u32);
        if let Some(p) = self.prev.take() {
            self.finish(p, (off, pc));
        }
        self.last = (off, pc);
    }

    /// A transfer at `pc` (PRG offset `here`) with the stack pointer `sp`
    /// it ran with; its target is the next instruction. Its operand and a
    /// `jmp (ptr)`'s pointer are read now: the transfer changes neither.
    pub fn begin(&mut self, bus: &Bus, opcode: u8, here: Option<usize>, pc: u16, sp: u8) {
        let off = here.map_or(NO_ROM, |o| o as u32);
        let pointer = (opcode == 0x6C).then(|| {
            let lo = bus.peek(pc.wrapping_add(1));
            let hi = bus.peek(pc.wrapping_add(2));
            let ptr = u16::from_le_bytes([lo, hi]);
            // The 6502 reads the high byte from the same page.
            let next = (ptr & 0xFF00) | (ptr.wrapping_add(1) & 0x00FF);
            (ptr, u16::from_le_bytes([bus.peek(ptr), bus.peek(next)]))
        });
        if opcode == 0x20 {
            self.returns[usize::from(sp)] = Some(pc.wrapping_add(3));
        }
        self.prev = Some(Prev {
            opcode,
            offset: off,
            pc,
            sp,
            pointer,
        });
    }

    /// Whether a transfer waits for the next instruction, its target.
    pub fn pending(&self) -> bool {
        self.prev.is_some()
    }

    /// The last instruction run, for a caller that doesn't show it every
    /// instruction (the shim): its PRG offset, if in PRG ROM, and CPU
    /// address.
    pub fn set_last(&mut self, offset: Option<usize>, pc: u16) {
        self.last = (offset.map_or(NO_ROM, |o| o as u32), pc);
    }

    /// A soft reset, with the reset handler's first instruction about to
    /// run at `here`: one edge, and nothing pending or returning carries
    /// across it.
    pub fn reset(&mut self, bus: &Bus, here: Option<usize>) {
        let to = (here.map_or(NO_ROM, |o| o as u32), bus.cpu.pc);
        let from = self.last;
        self.prev = None;
        self.returns = [None; 256];
        self.add(EdgeKind::Reset, from, to, None);
    }

    /// For each stack slot, the address a pending `jsr` returns to: what a
    /// run from a snapshot needs to tell a return from a jump.
    pub fn returns(&self) -> [Option<u16>; 256] {
        self.returns
    }

    /// Starts with the pending returns of a run up to a snapshot.
    pub fn set_returns(&mut self, r: [Option<u16>; 256]) {
        self.returns = r;
    }

    /// Ends a frame.
    pub fn end_frame(&mut self) {
        self.flush();
        self.frames += 1;
    }

    /// Adds another run's edges (of the same ROM), its cache's counts
    /// included. First frames are kept from the run that has the edge first
    /// in `self`.
    pub fn merge(&mut self, other: &Edges) {
        self.flush();
        let mut theirs = other.table.clone();
        for h in other.hot.iter().filter(|h| h.count > 0) {
            let k = (KINDS[usize::from(h.key.0)], h.key.1, h.key.2);
            let e = theirs.entry(k).or_insert_with(|| EdgeStat {
                first_frame: other.frames + 1,
                from_cpu: h.from_cpu,
                to_cpu: h.to_cpu,
                ..EdgeStat::default()
            });
            e.count += u64::from(h.count);
        }
        for (k, v) in theirs {
            match self.table.get_mut(&k) {
                Some(e) => {
                    e.count += v.count;
                    e.pointers.extend(v.pointers.iter().copied());
                }
                None => {
                    self.table.insert(k, v);
                }
            }
        }
    }

    /// The PRG offsets control entered: the targets of edges that enter
    /// code ([`EdgeKind::enters`]).
    pub fn entries(&self) -> BTreeSet<u32> {
        self.table
            .keys()
            .filter(|(k, _, to)| k.enters() && *to != NO_ROM)
            .map(|(_, _, to)| *to)
            .collect()
    }
}
