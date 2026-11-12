//! `gp interp draws --find`: a game's random register and the instructions
//! whose reads of it decide a branch, found from a run of the original with
//! no model (docs/formats/draws.md, "Finding draw sites").
//!
//! Two runs of the same input in gp-nes. The first finds the register: RAM
//! bytes stepped by a shift (`ROR $18`) or by a store of a value made by a
//! shift, an exclusive or or an add, on most frames, whose value at the end
//! of a frame takes many values and changes by no one amount on most frames
//! (that rules out counters and timers) and by more than 8 either way
//! on most frames (that rules out positions). Adjacent bytes stepped alike are
//! one register. The second run follows each read of the register: the
//! value read is traced through the registers, the flags and RAM for a few
//! dozen instructions, and a read whose value reaches a conditional branch
//! (or the index of a table read) is a draw site. A read
//! followed by a write to the register at the same stack depth is the
//! register's own step, not a draw.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde_json::{Value as Json, json};
use tetanes_core::bus::Bus;
use tetanes_core::cpu::instr::{AddrMode, Instr};
use tetanes_core::cpu::{Cpu, Status};
use tetanes_core::debug::FrameKind;

use gp_events::footprint::{reads_memory, target, writes_memory};
use gp_evidence::input_log::InputFrame;
use gp_interp::mapping::DrawSite;

/// Internal RAM, mirrored through $1FFF.
const RAM: usize = 0x800;
/// How many instructions a read's value is followed for.
const WINDOW: u32 = 64;
/// How many instructions after a read a write to the register, at the same
/// stack depth, makes the read part of the register's own step.
const STEP_WINDOW: u32 = 48;
/// The most reads followed at once.
const MAX_TRACKERS: usize = 16;

/// How a byte is stepped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    /// A read-modify-write shift or rotate of the byte (`ROR $18`).
    #[default]
    Shift,
    /// A store of a value made by an exclusive or, an add or a shift.
    Mixed,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Shift => "shift",
            Kind::Mixed => "mixed",
        }
    }
}

/// One RAM byte's writes and end-of-frame values over the first run.
#[derive(Clone)]
struct ByteStats {
    shift_writes: u64,
    mixed_writes: u64,
    main_writes: u64,
    /// The frame (counted from 1) of the first shift or mixed write.
    first: Option<u64>,
    /// Frames with at least one shift or mixed write.
    stepped_frames: u64,
    this_frame: u64,
    min_per_frame: u64,
    max_per_frame: u64,
    writers: BTreeMap<u16, u64>,
    routines: BTreeMap<u16, u64>,
    seen: [u64; 4],
    prev: u8,
    deltas: Vec<u32>,
}

impl Default for ByteStats {
    fn default() -> Self {
        ByteStats {
            shift_writes: 0,
            mixed_writes: 0,
            main_writes: 0,
            first: None,
            stepped_frames: 0,
            this_frame: 0,
            min_per_frame: u64::MAX,
            max_per_frame: 0,
            writers: BTreeMap::new(),
            routines: BTreeMap::new(),
            seen: [0; 4],
            prev: 0,
            deltas: vec![0; 256],
        }
    }
}

impl ByteStats {
    fn distinct(&self) -> u32 {
        self.seen.iter().map(|w| w.count_ones()).sum()
    }
    /// The share of frames since the first step whose change is the most
    /// common change: 1 for a counter, small for a random byte.
    fn modal_delta(&self) -> f64 {
        let total: u64 = self.deltas.iter().map(|&n| u64::from(n)).sum();
        if total == 0 {
            return 1.0;
        }
        f64::from(*self.deltas.iter().max().unwrap_or(&0)) / total as f64
    }
    /// The share of frames since the first step whose change is 8 or less
    /// either way: near 1 for a position or a timer, which move a little a
    /// frame, and about 7% for a random byte.
    fn small_delta(&self) -> f64 {
        let total: u64 = self.deltas.iter().map(|&n| u64::from(n)).sum();
        if total == 0 {
            return 1.0;
        }
        let small: u64 = self.deltas[..=8]
            .iter()
            .chain(&self.deltas[248..])
            .map(|&n| u64::from(n))
            .sum();
        small as f64 / total as f64
    }
}

/// The call stack's routines, for naming where an instruction runs: the
/// target of each `JSR` and each interrupt's handler.
#[derive(Default)]
struct Calls {
    stack: Vec<u16>,
    /// Interrupts entered and not returned from.
    depth: u32,
}

impl Calls {
    /// Before an instruction: an interrupt taken after the last one, then
    /// the instruction's own call or return. Returns whether an interrupt
    /// was entered.
    fn before(&mut self, bus: &mut Bus, instr: Instr, pc: u16) -> bool {
        let interrupted = matches!(
            bus.interrupt_taken.take(),
            Some(FrameKind::Nmi | FrameKind::Irq)
        );
        if interrupted {
            self.depth += 1;
            self.push(pc);
        }
        match instr {
            Instr::JSR => self.push(u16::from_le_bytes([
                bus.peek(pc.wrapping_add(1)),
                bus.peek(pc.wrapping_add(2)),
            ])),
            Instr::RTS => {
                self.stack.pop();
            }
            Instr::RTI => {
                self.stack.pop();
                self.depth = self.depth.saturating_sub(1);
            }
            _ => {}
        }
        interrupted
    }
    fn push(&mut self, at: u16) {
        // A game that calls through pushed addresses can leave the stack
        // deep; keep the innermost calls.
        if self.stack.len() >= 64 {
            self.stack.remove(0);
        }
        self.stack.push(at);
    }
    fn routine(&self) -> Option<u16> {
        self.stack.last().copied()
    }
}

fn ram_index(t: u16) -> Option<usize> {
    let a = usize::from(t);
    // The stack page holds return addresses, never a random byte.
    (a < 0x2000 && !(0x100..0x200).contains(&(a & (RAM - 1)))).then_some(a & (RAM - 1))
}

fn is_shift(instr: Instr) -> bool {
    matches!(instr, Instr::ASL | Instr::LSR | Instr::ROL | Instr::ROR)
}

/// The first run: every RAM byte's writes and values.
struct Scan {
    bytes: Vec<ByteStats>,
    calls: Calls,
    /// Whether A was last made by an exclusive or, an add or a shift.
    a_mixed: bool,
    frame: u64,
    started: bool,
}

impl Scan {
    fn new() -> Self {
        Scan {
            bytes: vec![ByteStats::default(); RAM],
            calls: Calls::default(),
            a_mixed: false,
            frame: 0,
            started: false,
        }
    }

    fn before(&mut self, bus: &mut Bus) {
        if !self.started {
            bus.interrupt_taken = None;
            self.started = true;
        }
        let pc = bus.cpu.pc;
        let r = Cpu::INSTR_REF[usize::from(bus.peek(pc))];
        self.calls.before(bus, r.instr, pc);
        if writes_memory(r.instr, r.addr_mode)
            && let Some(a) = target(bus, pc, r.instr, r.addr_mode).and_then(ram_index)
        {
            let kind = if is_shift(r.instr) {
                Some(Kind::Shift)
            } else if r.instr == Instr::STA && self.a_mixed {
                Some(Kind::Mixed)
            } else {
                None
            };
            if let Some(kind) = kind {
                let b = &mut self.bytes[a];
                match kind {
                    Kind::Shift => b.shift_writes += 1,
                    Kind::Mixed => b.mixed_writes += 1,
                }
                if self.calls.depth == 0 {
                    b.main_writes += 1;
                }
                b.first.get_or_insert(self.frame);
                b.this_frame += 1;
                *b.writers.entry(pc).or_default() += 1;
                if let Some(at) = self.calls.routine() {
                    *b.routines.entry(at).or_default() += 1;
                }
            }
        }
        match r.instr {
            Instr::EOR | Instr::ADC | Instr::SBC => self.a_mixed = true,
            i if is_shift(i) && r.addr_mode == AddrMode::ACC => self.a_mixed = true,
            Instr::LDA | Instr::TXA | Instr::TYA | Instr::PLA | Instr::LAX => self.a_mixed = false,
            _ => {}
        }
    }

    /// After a frame: each byte's value, and its writes that frame.
    fn frame_end(&mut self, ram: impl Fn(u16) -> u8) {
        for (a, b) in self.bytes.iter_mut().enumerate() {
            let v = ram(a as u16);
            b.seen[usize::from(v >> 6)] |= 1 << (v & 63);
            if b.first.is_some() {
                b.deltas[usize::from(v.wrapping_sub(b.prev))] += 1;
            }
            b.prev = v;
            if b.this_frame > 0 {
                b.stepped_frames += 1;
                b.min_per_frame = b.min_per_frame.min(b.this_frame);
                b.max_per_frame = b.max_per_frame.max(b.this_frame);
                b.this_frame = 0;
            }
        }
        self.frame += 1;
    }

    /// Whether byte `a` looks random, with how it's stepped.
    fn random(&self, a: usize) -> Option<Kind> {
        let b = &self.bytes[a];
        let first = b.first?;
        let span = self.frame.saturating_sub(first);
        // A generator runs all game: stepped on at least half of all the
        // frames run, not only of those after its first step.
        if span < 16 || b.stepped_frames * 2 < self.frame {
            return None;
        }
        if u64::from(b.distinct()) < 64.min(span / 4).max(8)
            || b.modal_delta() > 0.5
            || b.small_delta() > 0.5
        {
            return None;
        }
        Some(if b.shift_writes >= b.mixed_writes {
            Kind::Shift
        } else {
            Kind::Mixed
        })
    }

    /// The random registers: runs of adjacent random bytes stepped alike.
    fn registers(&self) -> Vec<Register> {
        let mut out: Vec<Register> = Vec::new();
        for a in 0..RAM {
            let Some(kind) = self.random(a) else { continue };
            let b = &self.bytes[a];
            let writes = b.shift_writes + b.mixed_writes;
            if let Some(last) = out.last_mut()
                && usize::from(last.hi) + 1 == a
                && last.kind == kind
                && {
                    let w = self.bytes[usize::from(last.hi)].shift_writes
                        + self.bytes[usize::from(last.hi)].mixed_writes;
                    writes <= w * 2 && w <= writes * 2
                }
            {
                last.hi = a as u16;
                last.add(b);
                continue;
            }
            let mut r = Register {
                lo: a as u16,
                hi: a as u16,
                kind,
                ..Register::default()
            };
            r.add(b);
            out.push(r);
        }
        for r in &mut out {
            r.finish(self.frame);
        }
        out
    }
}

/// A random register found by the first run.
#[derive(Clone, Debug, Default)]
pub(crate) struct Register {
    pub lo: u16,
    pub hi: u16,
    pub kind: Kind,
    /// Writes of its bytes, in the main loop and in all.
    pub writes: u64,
    pub main_writes: u64,
    /// The bytes' frames stepped, the fewest and most writes of one byte in
    /// a frame it was stepped, and the frames run.
    pub stepped_frames: u64,
    pub min_per_frame: u64,
    pub max_per_frame: u64,
    pub frames: u64,
    pub bytes: u64,
    pub writers: BTreeMap<u16, u64>,
    pub routines: BTreeMap<u16, u64>,
}

impl Register {
    fn add(&mut self, b: &ByteStats) {
        self.bytes += 1;
        self.writes += b.shift_writes + b.mixed_writes;
        self.main_writes += b.main_writes;
        self.stepped_frames = self.stepped_frames.max(b.stepped_frames);
        self.min_per_frame = if self.bytes == 1 {
            b.min_per_frame
        } else {
            self.min_per_frame.min(b.min_per_frame)
        };
        self.max_per_frame = self.max_per_frame.max(b.max_per_frame);
        for (k, n) in &b.writers {
            *self.writers.entry(*k).or_default() += n;
        }
        for (k, n) in &b.routines {
            *self.routines.entry(*k).or_default() += n;
        }
    }
    fn finish(&mut self, frames: u64) {
        self.frames = frames;
    }
    pub fn contains(&self, a: u16) -> bool {
        (self.lo..=self.hi).contains(&a)
    }
    /// Writes of one byte per frame stepped.
    fn per_frame(&self) -> f64 {
        self.writes as f64 / self.bytes.max(1) as f64 / self.stepped_frames.max(1) as f64
    }
    fn routine(&self) -> Option<u16> {
        self.routines
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(k, _)| *k)
    }
    pub fn range(&self) -> String {
        if self.lo == self.hi {
            format!("${:04X}", self.lo)
        } else {
            format!("${:04X}-${:04X}", self.lo, self.hi)
        }
    }
    fn line(&self) -> String {
        let place = if self.main_writes * 2 > self.writes {
            "in the main loop (the loop the CPU waits for the next frame in)"
        } else {
            "in an interrupt handler"
        };
        let by = match self.kind {
            Kind::Shift => "shifted in place",
            Kind::Mixed => "stored from an exclusive or, add or shift",
        };
        let routine = self
            .routine()
            .map(|r| format!(" by the routine at ${r:04X}"))
            .unwrap_or_default();
        format!(
            "random register {} ({} byte{}): {by}{routine} {place}, {:.1} times a frame ({} to {}), on {} of {} frames",
            self.range(),
            self.bytes,
            if self.bytes == 1 { "" } else { "s" },
            self.per_frame(),
            self.min_per_frame,
            self.max_per_frame,
            self.stepped_frames,
            self.frames
        )
    }
}

/// One read being followed: what its value has reached.
#[derive(Clone)]
struct Tracker {
    site: u16,
    age: u32,
    a: bool,
    x: bool,
    y: bool,
    /// The flags, as one: any flag set from the value.
    p: bool,
    mem: Vec<u16>,
    mask_found: bool,
    decided: bool,
}

impl Tracker {
    fn alive(&self) -> bool {
        self.age < WINDOW && (self.a || self.x || self.y || self.p || !self.mem.is_empty())
    }
    fn set_mem(&mut self, t: u16, on: bool) {
        let has = self.mem.contains(&t);
        if on && !has {
            self.mem.push(t);
        } else if !on && has {
            self.mem.retain(|&m| m != t);
        }
    }
}

/// A read site of the register, from the second run.
#[derive(Clone, Debug, Default)]
pub(crate) struct Site {
    pub pc: u16,
    pub text: String,
    pub reads: u64,
    pub frames: u64,
    last_frame: Option<u64>,
    /// Reads followed by a write to the register at the same stack depth.
    pub steps: u64,
    /// Reads whose value reached a branch or a table index.
    pub decided: u64,
    /// RAM the value was stored in, outside the register: a timer or a
    /// speed set at random decides later, past what a read is followed for.
    pub stored: BTreeMap<u16, u64>,
    pub addrs: BTreeMap<u16, u64>,
    pub base: Option<u16>,
    pub masks: BTreeMap<u8, u64>,
    /// Each branch the value decided: (taken, not taken).
    pub branches: BTreeMap<u16, (u64, u64)>,
    /// Each table read the value indexed, and the indexes seen.
    pub indexes: BTreeMap<u16, BTreeSet<u8>>,
    pub routines: BTreeMap<u16, u64>,
}

impl Site {
    /// The register's own step.
    pub fn is_step(&self) -> bool {
        self.steps * 2 > self.reads
    }
    /// The value read reached a conditional branch or a table's index. A
    /// rare draw may go one way only in a run (Donkey Kong's roll-off at
    /// $DCB3 is read once in run 5658S), so one way is enough.
    pub fn decides(&self) -> bool {
        !self.branches.is_empty() || !self.indexes.is_empty()
    }
    pub fn proposed(&self) -> bool {
        !self.is_step() && self.decides()
    }
    pub fn mask(&self) -> Option<u8> {
        self.masks.iter().max_by_key(|(_, n)| **n).map(|(k, _)| *k)
    }
    /// The random byte for the mapping's `ram`: the operand's address when
    /// it lies in the register, else the byte read most.
    pub fn ram(&self) -> u16 {
        self.base.unwrap_or_else(|| {
            self.addrs
                .iter()
                .max_by_key(|(_, n)| **n)
                .map_or(0, |(k, _)| *k)
        })
    }
    pub fn name(&self) -> String {
        format!("draw_{:04x}", self.pc)
    }
    fn routine(&self) -> Option<u16> {
        self.routines
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(k, _)| *k)
    }
    /// What the site does, for the proposed entry's note.
    pub fn note(&self) -> String {
        let mut parts = vec![self.text.clone()];
        if let Some(r) = self.routine() {
            parts.push(format!("in ${r:04X}"));
        }
        if let Some(m) = self.mask() {
            parts.push(format!("mask ${m:02X}"));
        }
        // The branches and tables decided most often first, three at most.
        let mut decides: Vec<(u64, String)> = self
            .branches
            .iter()
            .map(|(b, (t, n))| (t + n, format!("${b:04X} taken {t} of {}", t + n)))
            .collect();
        decides.extend(self.indexes.iter().map(|(i, s)| {
            let ways = if s.len() == 1 { "way" } else { "ways" };
            (
                if s.len() > 1 { u64::MAX } else { 0 },
                format!("${i:04X} indexed {} {ways}", s.len()),
            )
        }));
        decides.sort_by_key(|a| std::cmp::Reverse(a.0));
        if !decides.is_empty() {
            let more = decides.len().saturating_sub(3);
            let mut list: Vec<String> = decides.into_iter().take(3).map(|(_, d)| d).collect();
            if more > 0 {
                list.push(format!("{more} more"));
            }
            parts.push(format!("decides {}", list.join(", ")));
        }
        parts.push(format!("{} reads on {} frames", self.reads, self.frames));
        parts.join("; ")
    }
}

/// An instruction as assembly, for people.
fn asm(bus: &Bus, pc: u16) -> String {
    let r = Cpu::INSTR_REF[usize::from(bus.peek(pc))];
    let b1 = bus.peek(pc.wrapping_add(1));
    let w = u16::from_le_bytes([b1, bus.peek(pc.wrapping_add(2))]);
    let op = match r.addr_mode {
        AddrMode::ZP0 => format!(" ${b1:02X}"),
        AddrMode::ZPX => format!(" ${b1:02X},X"),
        AddrMode::ZPY => format!(" ${b1:02X},Y"),
        AddrMode::ABS | AddrMode::OTH => format!(" ${w:04X}"),
        AddrMode::ABX | AddrMode::ABXW => format!(" ${w:04X},X"),
        AddrMode::ABY | AddrMode::ABYW => format!(" ${w:04X},Y"),
        AddrMode::IDX => format!(" (${b1:02X},X)"),
        AddrMode::IDY | AddrMode::IDYW => format!(" (${b1:02X}),Y"),
        AddrMode::IMM => format!(" #${b1:02X}"),
        AddrMode::IND => format!(" (${w:04X})"),
        AddrMode::ACC | AddrMode::IMP | AddrMode::REL => String::new(),
    };
    format!("{:?}{op}", r.instr)
}

/// The operand's address before any index, for the zero page and absolute
/// modes.
fn operand_base(bus: &Bus, pc: u16, mode: AddrMode) -> Option<u16> {
    let b1 = bus.peek(pc.wrapping_add(1));
    let w = u16::from_le_bytes([b1, bus.peek(pc.wrapping_add(2))]);
    match mode {
        AddrMode::ZP0 | AddrMode::ZPX | AddrMode::ZPY => Some(u16::from(b1)),
        AddrMode::ABS | AddrMode::ABX | AddrMode::ABXW | AddrMode::ABY | AddrMode::ABYW => Some(w),
        _ => None,
    }
}

fn branch_taken(instr: Instr, p: Status) -> Option<bool> {
    Some(match instr {
        Instr::BEQ => p.contains(Status::Z),
        Instr::BNE => !p.contains(Status::Z),
        Instr::BMI => p.contains(Status::N),
        Instr::BPL => !p.contains(Status::N),
        Instr::BCS => p.contains(Status::C),
        Instr::BCC => !p.contains(Status::C),
        Instr::BVS => p.contains(Status::V),
        Instr::BVC => !p.contains(Status::V),
        _ => return None,
    })
}

/// The second run: every read of the registers, followed.
struct Readers {
    register: Vec<bool>,
    sites: BTreeMap<u16, Site>,
    calls: Calls,
    active: Vec<Tracker>,
    suspended: Vec<Vec<Tracker>>,
    /// Reads waiting to see whether the register is written at their depth.
    pending: Vec<(u16, u8, u32)>,
    frame: u64,
    started: bool,
}

impl Readers {
    fn new(registers: &[Register]) -> Self {
        let mut register = vec![false; RAM];
        for r in registers {
            for a in r.lo..=r.hi {
                register[usize::from(a)] = true;
            }
        }
        Readers {
            register,
            sites: BTreeMap::new(),
            calls: Calls::default(),
            active: Vec::new(),
            suspended: Vec::new(),
            pending: Vec::new(),
            frame: 0,
            started: false,
        }
    }

    fn in_register(&self, t: Option<u16>) -> Option<u16> {
        let t = t?;
        ram_index(t).filter(|&a| self.register[a]).map(|a| a as u16)
    }

    fn before(&mut self, bus: &mut Bus) {
        if !self.started {
            bus.interrupt_taken = None;
            self.started = true;
        }
        let pc = bus.cpu.pc;
        let r = Cpu::INSTR_REF[usize::from(bus.peek(pc))];
        let (instr, mode) = (r.instr, r.addr_mode);
        if self.calls.before(bus, instr, pc) {
            self.suspended.push(std::mem::take(&mut self.active));
        }
        if instr == Instr::RTI {
            self.active = self.suspended.pop().unwrap_or_default();
        }
        let t = target(bus, pc, instr, mode);
        let sp = bus.cpu.sp;

        // A write to the register at a pending read's depth: that read was
        // the register's own step.
        for p in &mut self.pending {
            p.2 += 1;
        }
        if writes_memory(instr, mode) && self.in_register(t).is_some() {
            for &(site, _, _) in self.pending.iter().filter(|p| p.1 == sp) {
                if let Some(s) = self.sites.get_mut(&site) {
                    s.steps += 1;
                }
            }
            self.pending.retain(|p| p.1 != sp);
        }
        self.pending.retain(|p| p.2 <= STEP_WINDOW);

        // The reads already followed see this instruction.
        let taken = branch_taken(instr, bus.cpu.status);
        let (x, y) = (bus.cpu.x, bus.cpu.y);
        for tr in &mut self.active {
            tr.age += 1;
            let src = reads_memory(instr, mode) && t.is_some_and(|t| tr.mem.contains(&t));
            let by_x = matches!(
                mode,
                AddrMode::ZPX | AddrMode::ABX | AddrMode::ABXW | AddrMode::IDX
            ) && tr.x;
            let by_y = matches!(
                mode,
                AddrMode::ZPY | AddrMode::ABY | AddrMode::ABYW | AddrMode::IDY | AddrMode::IDYW
            ) && tr.y;
            let site = self
                .sites
                .get_mut(&tr.site)
                .expect("a followed read has a site");
            if by_x || by_y {
                site.indexes
                    .entry(pc)
                    .or_default()
                    .insert(if by_x { x } else { y });
                tr.decided = true;
            }
            let imm = mode == AddrMode::IMM;
            match instr {
                Instr::LDA => (tr.a, tr.p) = (src, src),
                Instr::LDX => (tr.x, tr.p) = (src, src),
                Instr::LDY => (tr.y, tr.p) = (src, src),
                Instr::LAX => (tr.a, tr.x, tr.p) = (src, src, src),
                Instr::AND | Instr::ORA | Instr::EOR | Instr::ADC | Instr::SBC => {
                    if instr == Instr::AND && imm && tr.a && !tr.mask_found {
                        tr.mask_found = true;
                        *site.masks.entry(bus.peek(pc.wrapping_add(1))).or_default() += 1;
                    }
                    tr.a = tr.a || src;
                    tr.p = tr.a;
                }
                Instr::CMP => tr.p = tr.a || src,
                Instr::CPX => tr.p = tr.x || src,
                Instr::CPY => tr.p = tr.y || src,
                Instr::BIT => tr.p = tr.a || src,
                Instr::ASL | Instr::LSR | Instr::ROL | Instr::ROR => {
                    if mode == AddrMode::ACC {
                        tr.p = tr.a;
                    } else if let Some(t) = t {
                        let on = src || (tr.p && matches!(instr, Instr::ROL | Instr::ROR));
                        tr.set_mem(t, on);
                        tr.p = on;
                    }
                }
                Instr::INC | Instr::DEC => {
                    if let Some(t) = t {
                        tr.set_mem(t, src);
                    }
                    tr.p = src;
                }
                Instr::INX | Instr::DEX => tr.p = tr.x,
                Instr::INY | Instr::DEY => tr.p = tr.y,
                Instr::TAX => (tr.x, tr.p) = (tr.a, tr.a),
                Instr::TAY => (tr.y, tr.p) = (tr.a, tr.a),
                Instr::TXA => (tr.a, tr.p) = (tr.x, tr.x),
                Instr::TYA => (tr.a, tr.p) = (tr.y, tr.y),
                Instr::TSX => (tr.x, tr.p) = (false, false),
                Instr::STA | Instr::STX | Instr::STY => {
                    if let Some(t) = t {
                        let on = match instr {
                            Instr::STA => tr.a,
                            Instr::STX => tr.x,
                            _ => tr.y,
                        };
                        tr.set_mem(t, on);
                        if on && ram_index(t).is_some_and(|a| !self.register[a]) {
                            *site.stored.entry(t).or_default() += 1;
                        }
                    }
                }
                Instr::PLA => (tr.a, tr.p) = (false, false),
                Instr::PLP => tr.p = false,
                _ => {
                    if let Some(went) = taken
                        && tr.p
                    {
                        let e = site.branches.entry(pc).or_default();
                        if went {
                            e.0 += 1;
                        } else {
                            e.1 += 1;
                        }
                        tr.decided = true;
                    }
                }
            }
        }
        // A read that ends here counts once.
        let mut ended = Vec::new();
        self.active.retain(|tr| {
            let keep = tr.alive();
            if !keep {
                ended.push((tr.site, tr.decided));
            }
            keep
        });
        for (site, decided) in ended {
            if decided && let Some(s) = self.sites.get_mut(&site) {
                s.decided += 1;
            }
        }

        // A read of the register starts a new one.
        if reads_memory(instr, mode)
            && let Some(at) = self.in_register(t)
        {
            let routine = self.calls.routine();
            let base = operand_base(bus, pc, mode)
                .and_then(|b| ram_index(b).filter(|&a| self.register[a]).map(|a| a as u16));
            let frame = self.frame;
            let s = self.sites.entry(pc).or_insert_with(|| Site {
                pc,
                text: asm(bus, pc),
                base,
                ..Site::default()
            });
            s.reads += 1;
            if s.last_frame != Some(frame) {
                s.frames += 1;
                s.last_frame = Some(frame);
            }
            *s.addrs.entry(at).or_default() += 1;
            if let Some(r) = routine {
                *s.routines.entry(r).or_default() += 1;
            }
            let mut tr = Tracker {
                site: pc,
                age: 0,
                a: false,
                x: false,
                y: false,
                p: true,
                mem: Vec::new(),
                mask_found: false,
                decided: false,
            };
            match instr {
                Instr::LDA | Instr::ORA | Instr::EOR | Instr::ADC | Instr::SBC => tr.a = true,
                Instr::AND => {
                    tr.a = true;
                    tr.mask_found = true;
                    *s.masks.entry(bus.cpu.acc).or_default() += 1;
                }
                Instr::LDX => tr.x = true,
                Instr::LDY => tr.y = true,
                Instr::LAX => (tr.a, tr.x) = (true, true),
                Instr::INC
                | Instr::DEC
                | Instr::ASL
                | Instr::LSR
                | Instr::ROL
                | Instr::ROR
                | Instr::DCP
                | Instr::ISB
                | Instr::SLO
                | Instr::RLA
                | Instr::SRE
                | Instr::RRA => {
                    if let Some(t) = t {
                        tr.mem.push(t);
                    }
                }
                _ => {}
            }
            if self.active.len() >= MAX_TRACKERS {
                self.active.remove(0);
            }
            self.active.push(tr);
            self.pending.push((pc, sp, 0));
        }
    }

    fn frame_end(&mut self) {
        self.frame += 1;
    }
}

/// What `--find` found on one input.
pub(crate) struct Found {
    pub frames: u64,
    pub registers: Vec<Register>,
    pub sites: Vec<Site>,
}

impl Found {
    pub fn proposed(&self) -> impl Iterator<Item = &Site> {
        self.sites.iter().filter(|s| s.proposed())
    }
}

/// Runs `inputs` twice from power-on: once to find the random registers,
/// once to follow their reads. Frames up to `from` run unwatched.
pub(crate) fn find(rom: &[u8], inputs: &[InputFrame], from: u64) -> Result<Found> {
    let power_on = || {
        gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())
            .map_err(anyhow::Error::msg)
    };
    let step_err = |e: gp_nes::NesError| anyhow::anyhow!("{e}");

    let mut nes = power_on()?;
    let mut scan = Scan::new();
    for (i, input) in inputs.iter().enumerate() {
        if (i as u64) < from {
            nes.step(*input).map_err(step_err)?;
            continue;
        }
        nes.step_with(*input, |bus| scan.before(bus))
            .map_err(step_err)?;
        scan.frame_end(|a| nes.peek(a));
    }
    let registers = scan.registers();

    let mut sites = Vec::new();
    if !registers.is_empty() {
        let mut nes = power_on()?;
        let mut readers = Readers::new(&registers);
        for (i, input) in inputs.iter().enumerate() {
            if (i as u64) < from {
                nes.step(*input).map_err(step_err)?;
                continue;
            }
            nes.step_with(*input, |bus| readers.before(bus))
                .map_err(step_err)?;
            readers.frame_end();
        }
        sites = readers.sites.into_values().collect();
    }
    Ok(Found {
        frames: scan.frame,
        registers,
        sites,
    })
}

/// The found sites held against a mapping's hand-written `[[draws]]`.
pub(crate) struct Against {
    pub hand: usize,
    pub proposed: usize,
    /// Hand sites found, by name and address.
    pub found: Vec<(String, u16)>,
    pub missed: Vec<(String, u16)>,
    /// Proposed sites the mapping doesn't name.
    pub extra: Vec<u16>,
    /// Hand sites whose random byte lies in a found register.
    pub ram_in_register: usize,
}

impl Against {
    pub fn recall(&self) -> f64 {
        if self.hand == 0 {
            return 1.0;
        }
        self.found.len() as f64 / self.hand as f64
    }
    pub fn precision(&self) -> f64 {
        if self.proposed == 0 {
            return 1.0;
        }
        self.found.len() as f64 / self.proposed as f64
    }
}

pub(crate) fn against(found: &Found, hand: &[DrawSite]) -> Against {
    let proposed: BTreeSet<u16> = found.proposed().map(|s| s.pc).collect();
    let pcs: BTreeSet<u16> = hand.iter().map(|d| d.pc).collect();
    let mut a = Against {
        hand: hand.len(),
        proposed: proposed.len(),
        found: Vec::new(),
        missed: Vec::new(),
        extra: proposed.difference(&pcs).copied().collect(),
        ram_in_register: 0,
    };
    for d in hand {
        if proposed.contains(&d.pc) {
            a.found.push((d.site.clone(), d.pc));
        } else {
            a.missed.push((d.site.clone(), d.pc));
        }
        if found.registers.iter().any(|r| r.contains(d.ram)) {
            a.ram_in_register += 1;
        }
    }
    a
}

/// The proposed `[[draws]]` entries, as TOML for a mapping, each named by
/// its address unless the mapping already names it.
pub(crate) fn proposal(found: &Found, hand: &[DrawSite]) -> String {
    let mut out = String::from(
        "# Proposed by `gp interp draws --find`: review each site, name it after\n\
         # what it chooses, and keep the ones the model's hooks draw at.\n\
         score_draws = true\n",
    );
    for s in found.proposed() {
        let named = hand.iter().find(|d| d.pc == s.pc);
        let name = named.map_or_else(|| s.name(), |d| d.site.clone());
        out += &format!(
            "\n[[draws]]\nsite = \"{name}\"\npc = 0x{:04X}\nram = 0x{:02X}\nnote = \"{}\"\n",
            s.pc,
            s.ram(),
            s.note().replace('"', "'")
        );
    }
    out
}

/// The report in words.
pub(crate) fn text(found: &Found, vs: Option<&Against>) -> String {
    let mut lines = Vec::new();
    if found.registers.is_empty() {
        lines.push(format!(
            "no random register found in {} frames: no RAM byte is stepped by a shift, an exclusive or or an add on most frames with many values",
            found.frames
        ));
    }
    for r in &found.registers {
        lines.push(r.line());
    }
    let proposed: Vec<&Site> = found.proposed().collect();
    if !found.registers.is_empty() {
        lines.push(format!(
            "{} draw site{} (reads whose value decides a branch or a table index):",
            proposed.len(),
            if proposed.len() == 1 { "" } else { "s" }
        ));
        for s in &proposed {
            lines.push(format!("  ${:04X} {}", s.pc, s.note()));
        }
        let steps: Vec<String> = found
            .sites
            .iter()
            .filter(|s| s.is_step())
            .map(|s| format!("${:04X} {}", s.pc, s.text))
            .collect();
        if !steps.is_empty() {
            lines.push(format!("the register's own step: {}", steps.join(", ")));
        }
        let stored: Vec<String> = found
            .sites
            .iter()
            .filter(|s| !s.is_step() && !s.decides() && !s.stored.is_empty())
            .map(|s| {
                let to: Vec<String> = s.stored.keys().map(|t| format!("${t:04X}")).collect();
                format!(
                    "${:04X} {} into {} ({} reads)",
                    s.pc,
                    s.text,
                    to.join(" "),
                    s.reads
                )
            })
            .collect();
        if !stored.is_empty() {
            lines.push(format!(
                "reads stored for later, to review (a timer or speed set at random): {}",
                stored.join(", ")
            ));
        }
        let idle: Vec<String> = found
            .sites
            .iter()
            .filter(|s| !s.is_step() && !s.decides() && s.stored.is_empty())
            .map(|s| format!("${:04X} {} ({} reads)", s.pc, s.text, s.reads))
            .collect();
        if !idle.is_empty() {
            lines.push(format!("reads that decide nothing: {}", idle.join(", ")));
        }
    }
    if let Some(v) = vs {
        lines.push(format!(
            "against the mapping's {} [[draws]] site{}: recall {}/{} ({:.0}%), precision {}/{} ({:.0}%); {} of {} random bytes in a found register",
            v.hand,
            if v.hand == 1 { "" } else { "s" },
            v.found.len(),
            v.hand,
            100.0 * v.recall(),
            v.found.len(),
            v.proposed,
            100.0 * v.precision(),
            v.ram_in_register,
            v.hand
        ));
        if !v.missed.is_empty() {
            lines.push(format!(
                "  missed: {}",
                v.missed
                    .iter()
                    .map(|(n, pc)| format!("{n} ${pc:04X}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !v.extra.is_empty() {
            lines.push(format!(
                "  not in the mapping: {}",
                v.extra
                    .iter()
                    .map(|pc| format!("${pc:04X}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    lines.join("\n")
}

pub(crate) fn to_json(found: &Found, vs: Option<&Against>) -> Json {
    json!({
        "frames": found.frames,
        "registers": found.registers.iter().map(|r| json!({
            "lo": r.lo,
            "hi": r.hi,
            "kind": r.kind.word(),
            "routine": r.routine(),
            "per_frame": r.per_frame(),
            "min_per_frame": r.min_per_frame,
            "max_per_frame": r.max_per_frame,
            "main_loop": r.main_writes * 2 > r.writes,
            "stepped_frames": r.stepped_frames,
        })).collect::<Vec<_>>(),
        "sites": found.sites.iter().map(|s| json!({
            "pc": s.pc,
            "text": s.text,
            "ram": s.ram(),
            "reads": s.reads,
            "frames": s.frames,
            "step": s.is_step(),
            "decides": s.decides(),
            "proposed": s.proposed(),
            "stored": s.stored.keys().collect::<Vec<_>>(),
            "mask": s.mask(),
            "branches": s.branches.iter().map(|(b, (t, n))| json!({"pc": b, "taken": t, "not_taken": n})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "against": vs.map(|v| json!({
            "hand": v.hand,
            "proposed": v.proposed,
            "found": v.found.iter().map(|(n, pc)| json!({"site": n, "pc": pc})).collect::<Vec<_>>(),
            "missed": v.missed.iter().map(|(n, pc)| json!({"site": n, "pc": pc})).collect::<Vec<_>>(),
            "extra": v.extra,
            "recall": v.recall(),
            "precision": v.precision(),
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An NROM-128 cartridge: `code` at $C000, the NMI handler at `nmi`.
    fn nrom(code: &[u8], nmi: u16) -> Vec<u8> {
        let mut rom = vec![0u8; 16 + 0x4000 + 0x2000];
        rom[..4].copy_from_slice(b"NES\x1A");
        rom[4] = 1;
        rom[5] = 1;
        let prg = &mut rom[16..16 + 0x4000];
        prg[..code.len()].copy_from_slice(code);
        prg[0x3FFA..0x3FFC].copy_from_slice(&nmi.to_le_bytes());
        prg[0x3FFC..0x3FFE].copy_from_slice(&0xC000u16.to_le_bytes());
        prg[0x3FFE..0x4000].copy_from_slice(&0xC000u16.to_le_bytes());
        rom
    }

    /// A made-up game: the main loop steps a 16-bit shift register at
    /// $10-$11 as often as it can, as Donkey Kong's idle loop does; the NMI
    /// handler draws from it ($C030 decides a branch), copies it without
    /// deciding anything ($C038), and counts frames with an add ($30).
    fn game() -> Vec<u8> {
        #[rustfmt::skip]
        let code: &[u8] = &[
            0x78,             // C000 SEI
            0xD8,             // C001 CLD
            0xA2, 0xFF,       // C002 LDX #$FF
            0x9A,             // C004 TXS
            0xA9, 0x01,       // C005 LDA #$01
            0x85, 0x10,       // C007 STA $10
            0xA9, 0x80,       // C009 LDA #$80
            0x8D, 0x00, 0x20, // C00B STA $2000
            0x20, 0x14, 0xC0, // C00E JSR $C014
            0x4C, 0x0E, 0xC0, // C011 JMP $C00E
            0x46, 0x11,       // C014 LSR $11      the step: a Galois
            0x66, 0x10,       // C016 ROR $10      shift register
            0x90, 0x06,       // C018 BCC $C020
            0xA5, 0x11,       // C01A LDA $11
            0x49, 0xB4,       // C01C EOR #$B4
            0x85, 0x11,       // C01E STA $11
            0x60,             // C020 RTS
            0xEA, 0xEA,       // C021 NOP NOP
            0x48,             // C023 PHA          the NMI
            0x20, 0x30, 0xC0, // C024 JSR $C030
            0xA5, 0x30,       // C027 LDA $30
            0x18,             // C029 CLC
            0x69, 0x01,       // C02A ADC #$01
            0x85, 0x30,       // C02C STA $30
            0x68,             // C02E PLA
            0x40,             // C02F RTI
            0xA5, 0x11,       // C030 LDA $11      the draw
            0x29, 0x01,       // C032 AND #$01
            0xF0, 0x02,       // C034 BEQ $C038
            0xE6, 0x20,       // C036 INC $20
            0xA5, 0x10,       // C038 LDA $10      a copy
            0x85, 0x40,       // C03A STA $40
            0x60,             // C03C RTS
        ];
        nrom(code, 0xC023)
    }

    #[test]
    fn finds_the_register_and_the_one_read_that_decides() {
        let inputs = vec![InputFrame::default(); 400];
        let found = find(&game(), &inputs, 0).unwrap();
        let regs: Vec<(u16, u16, Kind)> = found
            .registers
            .iter()
            .map(|r| (r.lo, r.hi, r.kind))
            .collect();
        assert_eq!(
            regs,
            vec![(0x10, 0x11, Kind::Shift)],
            "{}",
            text(&found, None)
        );
        let r = &found.registers[0];
        assert!(r.main_writes * 2 > r.writes, "stepped in the main loop");
        assert!(r.per_frame() > 10.0, "many passes a frame");
        assert_eq!(r.routine(), Some(0xC014));

        let proposed: Vec<u16> = found.proposed().map(|s| s.pc).collect();
        assert_eq!(proposed, vec![0xC030], "{}", text(&found, None));
        let draw = found.sites.iter().find(|s| s.pc == 0xC030).unwrap();
        assert_eq!(draw.mask(), Some(0x01));
        assert_eq!(draw.ram(), 0x11);
        assert_eq!(draw.routine(), Some(0xC030));
        assert!(draw.branches.contains_key(&0xC034));
        for step in [0xC014, 0xC016, 0xC01A] {
            assert!(found.sites.iter().find(|s| s.pc == step).unwrap().is_step());
        }
        let copy = found.sites.iter().find(|s| s.pc == 0xC038).unwrap();
        assert!(!copy.is_step() && !copy.decides());
        assert_eq!(copy.stored.keys().collect::<Vec<_>>(), vec![&0x40]);
    }

    #[test]
    fn scores_the_found_sites_against_a_mapping() {
        let inputs = vec![InputFrame::default(); 400];
        let found = find(&game(), &inputs, 0).unwrap();
        let site = |name: &str, pc, ram| DrawSite {
            site: name.to_owned(),
            pc,
            ram,
            note: None,
            soft: false,
        };
        let hand = [site("choice", 0xC030, 0x11), site("other", 0xC100, 0x10)];
        let v = against(&found, &hand);
        assert_eq!(v.found, vec![("choice".to_owned(), 0xC030)]);
        assert_eq!(v.missed, vec![("other".to_owned(), 0xC100)]);
        assert_eq!(v.ram_in_register, 2);
        assert!((v.recall() - 0.5).abs() < 1e-9 && (v.precision() - 1.0).abs() < 1e-9);
        let toml = proposal(&found, &hand);
        assert!(
            toml.contains("site = \"choice\"\npc = 0xC030\nram = 0x11"),
            "{toml}"
        );
        assert!(toml.contains("score_draws = true"));
    }

    #[test]
    fn a_game_with_only_counters_has_no_register() {
        #[rustfmt::skip]
        let code: &[u8] = &[
            0x78, 0xD8, 0xA2, 0xFF, 0x9A,
            0xA9, 0x80, 0x8D, 0x00, 0x20, // NMI on
            0x4C, 0x0A, 0xC0,             // C00A JMP $C00A
            0xA5, 0x30, 0x18, 0x69, 0x03, 0x85, 0x30, // C00D the NMI: $30 += 3
            0x40,
        ];
        let found = find(&nrom(code, 0xC00D), &vec![InputFrame::default(); 300], 0).unwrap();
        assert!(found.registers.is_empty(), "{}", text(&found, None));
        assert!(found.proposed().next().is_none());
    }
}
