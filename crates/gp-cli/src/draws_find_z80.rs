//! `gp interp draws --find` on an arcade romset (port wave 6): the same
//! two runs as [`crate::draws_find`] does on gp-nes, on gp-arcade's main
//! Z80, read through its instruction trace, its access log (each read and
//! write with the instruction that made it) and the Z80 disassembler
//! ([`gp_arcade::dasm`]).
//!
//! The first run finds the register among the main board's work RAM
//! (`e000-e7ff`): bytes stepped by a shift in place (`rlc (hl)`,
//! `rr (ix+d)`) or by a store of A (`ld (nn),a`, `ld (hl),a`) when A was
//! last made by an exclusive or, an add, a subtract or a rotate, on most
//! frames, with the same tests of their end-of-frame values as on the NES.
//! One Z80 difference: a byte stored from a value made from the refresh
//! register (`ld a,r`, which counts instructions run) is a generator the
//! game calls when it needs a number, so it needs only many values over
//! the frames it was stepped, not a step on most frames. Tapper's is one
//! (docs/formats/draws.md).
//! The second run follows each read of the register through the
//! registers (as 8-bit units; `exx` and `ex af,af'` move the second set as
//! one), the flags and memory (by the addresses the access log saw), for
//! [`WINDOW`] instructions: a read whose value reaches a conditional jump,
//! call or return (or `djnz` through B), an address register of a memory
//! operand (a table index), or `jp (hl)`/`jp (ix)`/`jp (iy)`, is a draw
//! site. A read followed by a write to the register with the same SP is
//! the register's own step. Reads in an interrupt handler are followed
//! inside it; the ones it interrupted wait for its `reti`.
//!
//! The output is [`crate::draws_find`]'s: the same registers, sites, text,
//! JSON and proposed `[[draws]]`, with Z80 text in place of 6502 text.

use std::collections::BTreeMap;

use anyhow::Result;
use gp_arcade::dasm::{self, Addr, Alu, Flow, Insn, Op, Ptr, reg};
use gp_arcade::mcr::{Access, AccessKind, Trace};
use gp_arcade::{Controls, Romset, Tapper};
use gp_evidence::input_log::InputFrame;

use crate::draws_find::{ByteStats, Found, Kind, Register, Site};

/// The main board's work RAM, where a game keeps its variables.
const RAM: u16 = 0xe000;
const RAM_LEN: usize = 0x800;
/// How many instructions a read's value is followed for.
const WINDOW: u32 = 64;
/// How many instructions after a read a write to the register, with the
/// same SP, makes the read part of the register's own step.
const STEP_WINDOW: u32 = 48;
const MAX_TRACKERS: usize = 16;

fn ram_index(a: u16) -> Option<usize> {
    a.checked_sub(RAM).map(usize::from).filter(|&i| i < RAM_LEN)
}

fn controls(f: &InputFrame) -> Controls {
    Controls::from_bits(f.buttons_word() & 0xff)
}

/// One main-CPU instruction with the accesses it made.
struct Step<'a> {
    e: &'a Trace,
    insn: Insn,
    accesses: &'a [Access],
    /// The next instruction's address, if any came in the frame.
    next: Option<u16>,
}

/// Runs one frame with the trace and access log on, and gives the main
/// CPU's instructions in order, each with its accesses, and interrupts as
/// entries of their own (`insn` unread).
pub(crate) fn frame(t: &mut Tapper, c: Controls) -> (Vec<Trace>, Vec<Access>) {
    t.set_trace(true);
    t.set_accesses(true);
    t.step(c);
    let mut trace: Vec<Trace> = t.take_trace().into_iter().filter(|e| e.cpu == 0).collect();
    trace.sort_by_key(|e| e.time);
    let accesses: Vec<Access> = t
        .take_accesses()
        .into_iter()
        .filter(|a| a.cpu == 0)
        .collect();
    t.set_trace(false);
    t.set_accesses(false);
    (trace, accesses)
}

/// Each instruction with its accesses: those from its start to the next
/// entry's.
fn steps<'a>(trace: &'a [Trace], accesses: &'a [Access]) -> Vec<Step<'a>> {
    let mut out = Vec::with_capacity(trace.len());
    let mut j = 0;
    for (i, e) in trace.iter().enumerate() {
        let end = trace.get(i + 1).map(|n| n.time);
        let from = j;
        while j < accesses.len() && end.is_none_or(|end| accesses[j].time < end) {
            j += 1;
        }
        let next = trace[i + 1..]
            .iter()
            .find(|n| n.interrupt.is_none())
            .map(|n| n.pc);
        out.push(Step {
            e,
            insn: dasm::decode(&e.bytes, e.pc),
            accesses: &accesses[from..j],
            next,
        });
    }
    out
}

/// One routine on the call stack.
#[derive(Clone)]
struct Frame {
    /// The call's target, or the interrupt's handler.
    at: u16,
    irq: bool,
    /// The call instruction and its text, for a routine that was called.
    from: Option<(u16, String)>,
    /// The register byte the routine wrote, if it wrote one (the readers'
    /// pass).
    stepped: Option<u16>,
    /// A read of the register that started in this routine (the site and
    /// the byte), for a routine that passes a drawn number on in A
    /// (Tapper's $5FAC, which brings $5FBD's number into a range).
    drew: Option<(u16, u16)>,
}

/// The call stack's routines: each call's target and each interrupt's
/// handler.
#[derive(Default)]
struct Calls {
    stack: Vec<Frame>,
    depth: u32,
}

impl Calls {
    fn interrupt(&mut self, handler: u16) {
        self.depth += 1;
        self.push(handler, true, None);
    }

    /// After an instruction: its call or return, by where control went.
    /// Returns the routine it returned from, if it returned.
    fn after(&mut self, s: &Step<'_>) -> Option<Frame> {
        let fall = s.e.pc.wrapping_add(u16::from(s.insn.len));
        let taken = s.next.is_some_and(|n| n != fall);
        match s.insn.flow {
            Flow::Call(t) | Flow::Rst(t) => {
                self.push(t, false, Some((s.e.pc, s.insn.text.clone())))
            }
            Flow::CallIf(t) if taken => self.push(t, false, Some((s.e.pc, s.insn.text.clone()))),
            Flow::Ret => return self.pop(),
            Flow::RetIf if taken => return self.pop(),
            _ => {}
        }
        None
    }

    fn push(&mut self, at: u16, irq: bool, from: Option<(u16, String)>) {
        if self.stack.len() >= 64 {
            self.stack.remove(0);
        }
        self.stack.push(Frame {
            at,
            irq,
            from,
            stepped: None,
            drew: None,
        });
    }

    fn pop(&mut self) -> Option<Frame> {
        let f = self.stack.pop();
        if f.as_ref().is_some_and(|f| f.irq) {
            self.depth = self.depth.saturating_sub(1);
        }
        f
    }

    fn routine(&self) -> Option<u16> {
        self.stack.last().map(|e| e.at)
    }
}

/// The first run: every work RAM byte's writes and values.
struct Scan {
    bytes: Vec<ByteStats>,
    calls: Calls,
    a_mixed: bool,
    /// Where each register's value came from, for the refresh rule.
    prov: Prov,
    /// Each byte's mixed writes of a value made from the refresh register.
    refresh: Vec<u64>,
    frame: u64,
}

impl Scan {
    fn instruction(&mut self, s: &Step<'_>) {
        let i = &s.insn;
        let stores_a = i.op == Op::Load && i.reads & reg::A != 0;
        for a in s.accesses.iter().filter(|a| a.kind == AccessKind::Write) {
            let Some(at) = ram_index(a.addr) else {
                continue;
            };
            let kind = if i.op == Op::Shift {
                Some(Kind::Shift)
            } else if stores_a && self.a_mixed {
                Some(Kind::Mixed)
            } else {
                None
            };
            let Some(kind) = kind else { continue };
            if kind == Kind::Mixed && self.prov.refresh_feeds(a.addr) {
                self.refresh[at] += 1;
            }
            let b = &mut self.bytes[at];
            match kind {
                Kind::Shift => b.shift_writes += 1,
                Kind::Mixed => b.mixed_writes += 1,
            }
            if self.calls.depth == 0 {
                b.main_writes += 1;
            }
            b.first.get_or_insert(self.frame);
            b.this_frame += 1;
            *b.writers.entry(s.e.pc).or_default() += 1;
            if let Some(r) = self.calls.routine() {
                *b.routines.entry(r).or_default() += 1;
            }
        }
        if i.writes & reg::A != 0 {
            // `ld a,r` reads the refresh register, which counts the
            // instructions run: the Z80's own source of noise.
            let refresh = i.reads & reg::R != 0;
            let mixes = matches!(
                i.op,
                Op::Alu(Alu::Xor | Alu::Add | Alu::Adc | Alu::Sub | Alu::Sbc) | Op::Shift
            );
            self.a_mixed = !i.constant && (mixes || refresh);
        }
        self.prov.instruction(s);
        self.calls.after(s);
    }

    fn frame_end(&mut self, t: &Tapper) {
        for (a, b) in self.bytes.iter_mut().enumerate() {
            let v = t.peek(RAM + a as u16);
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

    /// As [`crate::draws_find`]'s: stepped on half the frames run, many
    /// values, no common change, and not mostly small changes.
    fn random(&self, a: usize) -> Option<Kind> {
        let b = &self.bytes[a];
        let first = b.first?;
        // A byte stored from the refresh register is a generator a game
        // calls when it needs a number, not every frame: its many values
        // are enough.
        if self.refresh[a] * 2 >= b.mixed_writes && b.mixed_writes > 0 {
            let enough = 64.min(b.stepped_frames / 2).max(8);
            return (b.stepped_frames >= 8 && u64::from(b.distinct()) >= enough)
                .then_some(Kind::Mixed);
        }
        let span = self.frame.saturating_sub(first);
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

    fn registers(&self) -> Vec<Register> {
        let mut out: Vec<Register> = Vec::new();
        for a in 0..RAM_LEN {
            let Some(kind) = self.random(a) else { continue };
            let b = &self.bytes[a];
            let writes = b.shift_writes + b.mixed_writes;
            let addr = RAM + a as u16;
            if let Some(last) = out.last_mut()
                && last.hi + 1 == addr
                && last.kind == kind
                && {
                    let h = &self.bytes[usize::from(last.hi - RAM)];
                    let w = h.shift_writes + h.mixed_writes;
                    writes <= w * 2 && w <= writes * 2
                }
            {
                last.hi = addr;
                last.add(b);
                continue;
            }
            let mut r = Register {
                lo: addr,
                hi: addr,
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

/// Where each register's value came from: the RAM bytes read on the way
/// and whether the refresh register went in. For the refresh rule: a
/// generator's store feeds back the byte it read (`ld a,($e390)` ...
/// `ld a,r` / `xor c` / `ld ($e390),a`), where a byte merely set from a
/// random number doesn't.
#[derive(Default)]
struct Prov {
    regs: [(Vec<u16>, bool); 16],
}

impl Prov {
    fn instruction(&mut self, s: &Step<'_>) {
        let i = &s.insn;
        let data = i.reads & !address_regs(i);
        let mut addrs: Vec<u16> = Vec::new();
        let mut refresh = i.reads & reg::R != 0;
        if !i.constant {
            for (n, (a, r)) in self.regs.iter().enumerate() {
                if data & (1 << n) != 0 {
                    addrs.extend(a);
                    refresh |= r;
                }
            }
            addrs.extend(
                s.accesses
                    .iter()
                    .filter(|a| a.kind == AccessKind::Read && ram_index(a.addr).is_some())
                    .map(|a| a.addr),
            );
        } else {
            refresh = false;
        }
        addrs.sort_unstable();
        addrs.dedup();
        addrs.truncate(8);
        for (n, slot) in self.regs.iter_mut().enumerate() {
            if i.writes & (1 << n) != 0 {
                *slot = (addrs.clone(), refresh);
            }
        }
    }

    /// Whether the value being stored at `addr` (by the instruction just
    /// seen, before [`Prov::instruction`] moves on) came from `addr`
    /// itself and the refresh register.
    fn refresh_feeds(&self, addr: u16) -> bool {
        // Called before `instruction` for this step: the stored value is
        // A's (a store of A is the only kind the refresh rule counts).
        let (a, r) = &self.regs[0];
        *r && a.contains(&addr)
    }
}

/// One read being followed: the registers and memory its value reached.
#[derive(Clone)]
struct Tracker {
    site: u16,
    age: u32,
    regs: u32,
    mem: Vec<u16>,
    mask_found: bool,
    decided: bool,
}

impl Tracker {
    fn alive(&self) -> bool {
        self.age < WINDOW && (self.regs != 0 || !self.mem.is_empty())
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

/// The registers a memory operand's address comes from.
pub(crate) fn address_regs(i: &Insn) -> u32 {
    match i.mem.map(|m| m.addr) {
        Some(Addr::Pair(bits)) => bits,
        Some(Addr::Index(Ptr::Ix, _)) => reg::IX,
        Some(Addr::Index(Ptr::Iy, _)) => reg::IY,
        Some(Addr::Index(Ptr::Hl, _)) => reg::HL,
        Some(Addr::Stack) => reg::SP,
        Some(Addr::Abs(_)) | None => 0,
    }
}

/// The low byte of the index a tainted address register holds.
fn index_value(e: &Trace, regs: u32) -> u8 {
    if regs & reg::IX != 0 {
        e.ix as u8
    } else if regs & reg::IY != 0 {
        e.iy as u8
    } else if regs & reg::HL != 0 {
        e.hl as u8
    } else if regs & reg::DE != 0 {
        e.de as u8
    } else {
        e.bc as u8
    }
}

/// The second run: every read of the registers, followed.
struct Readers {
    register: Vec<bool>,
    sites: BTreeMap<u16, Site>,
    calls: Calls,
    active: Vec<Tracker>,
    suspended: Vec<Vec<Tracker>>,
    pending: Vec<(u16, u16, u32)>,
    frame: u64,
}

impl Readers {
    fn new(registers: &[Register]) -> Self {
        let mut register = vec![false; RAM_LEN];
        for r in registers {
            for a in r.lo..=r.hi {
                register[usize::from(a - RAM)] = true;
            }
        }
        Self {
            register,
            sites: BTreeMap::new(),
            calls: Calls::default(),
            active: Vec::new(),
            suspended: Vec::new(),
            pending: Vec::new(),
            frame: 0,
        }
    }

    fn in_register(&self, a: u16) -> bool {
        ram_index(a).is_some_and(|i| self.register[i])
    }

    fn interrupt(&mut self, handler: u16) {
        self.calls.interrupt(handler);
        self.suspended.push(std::mem::take(&mut self.active));
    }

    fn instruction(&mut self, s: &Step<'_>) {
        let (e, i) = (s.e, &s.insn);
        let pc = e.pc;
        let reads: Vec<u16> = s
            .accesses
            .iter()
            .filter(|a| a.kind == AccessKind::Read)
            .map(|a| a.addr)
            .collect();
        let writes: Vec<u16> = s
            .accesses
            .iter()
            .filter(|a| a.kind == AccessKind::Write)
            .map(|a| a.addr)
            .collect();

        // A write to the register with a pending read's SP: that read was
        // the register's own step.
        for p in &mut self.pending {
            p.2 += 1;
        }
        if let Some(&w) = writes.iter().find(|&&a| self.in_register(a)) {
            if let Some(f) = self.calls.stack.last_mut() {
                f.stepped = Some(w);
            }
            for &(site, _, _) in self.pending.iter().filter(|p| p.1 == e.sp) {
                if let Some(site) = self.sites.get_mut(&site) {
                    site.steps += 1;
                }
            }
            self.pending.retain(|p| p.1 != e.sp);
        }
        self.pending.retain(|p| p.2 <= STEP_WINDOW);

        let fall = pc.wrapping_add(u16::from(i.len));
        let taken = s.next.map(|n| n != fall);
        let addr_regs = address_regs(i);
        let data_regs = i.reads & !addr_regs;
        for tr in &mut self.active {
            tr.age += 1;
            let site = self
                .sites
                .get_mut(&tr.site)
                .expect("a followed read has a site");
            // A value that picks the address of a memory operand, or
            // where jp (hl) goes, is a table's index.
            let indexes = addr_regs & tr.regs & !reg::SP != 0
                || matches!(i.flow, Flow::Indirect(_)) && i.reads & tr.regs != 0;
            if indexes {
                let v = if matches!(i.flow, Flow::Indirect(_)) {
                    s.next.unwrap_or(0) as u8
                } else {
                    index_value(e, addr_regs & tr.regs)
                };
                site.indexes.entry(pc).or_default().insert(v);
                tr.decided = true;
            }
            if i.flow.conditional() {
                let cond = if matches!(i.flow, Flow::Djnz(_)) {
                    reg::B
                } else {
                    reg::F
                };
                if tr.regs & cond != 0
                    && let Some(went) = taken
                {
                    let b = site.branches.entry(pc).or_default();
                    if went {
                        b.0 += 1;
                    } else {
                        b.1 += 1;
                    }
                    tr.decided = true;
                }
            }
            if i.op == Op::Alu(Alu::And)
                && i.mem.is_none()
                && data_regs == reg::A
                && tr.regs & reg::A != 0
                && !tr.mask_found
                && e.bytes[0] == 0xe6
            {
                tr.mask_found = true;
                *site.masks.entry(e.bytes[1]).or_default() += 1;
            }
            let src = !i.constant
                && (tr.regs & data_regs != 0 || reads.iter().any(|a| tr.mem.contains(a)));
            tr.regs = (tr.regs & !i.writes) | if src { i.writes } else { 0 };
            for &a in &writes {
                tr.set_mem(a, src);
                if src && ram_index(a).is_some() && !self.register[ram_index(a).expect("ram")] {
                    *site.stored.entry(a).or_default() += 1;
                }
            }
        }
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

        // A read of the register starts a new one. A block copy or search
        // that passes over it isn't one.
        if let Some(&at) = reads.iter().find(|&&a| self.in_register(a))
            && !matches!(i.mem.map(|m| m.addr), Some(Addr::Stack))
            && i.op != Op::Block
        {
            let base = match i.mem.map(|m| m.addr) {
                Some(Addr::Abs(a)) if self.in_register(a) => Some(a),
                _ => None,
            };
            let mask = (i.op == Op::Alu(Alu::And)).then_some((e.af >> 8) as u8);
            self.start(pc, Some(at), base, i.writes, writes.clone(), mask);
            if let Some(site) = self.sites.get_mut(&pc)
                && site.text.is_empty()
            {
                site.text = i.text.clone();
            }
            self.pending.push((pc, e.sp, 0));
        }
        let leaving_irq =
            matches!(i.flow, Flow::Ret) && self.calls.stack.last().is_some_and(|f| f.irq);
        let returned = self.calls.after(s);
        if leaving_irq {
            self.active = self.suspended.pop().unwrap_or_default();
        }
        // A routine that stepped the register returns its number in A, as
        // a Z80 game's generator does (Tapper's at $5FBD): its call is a
        // read of the register, followed from the instruction after it.
        // And a routine that took a number so and returns it on in A (A
        // still made from it) makes its own call a read too, so the draw
        // sites are where the game asks for its number.
        let Some(Frame {
            from: Some((call, text)),
            stepped,
            drew,
            irq: false,
            ..
        }) = returned
        else {
            return;
        };
        let passes = drew.and_then(|(site, byte)| {
            self.active
                .iter()
                .any(|t| t.site == site && t.regs & reg::A != 0)
                .then_some(byte)
        });
        if let Some(byte) = stepped.or(passes) {
            self.start(call, Some(byte), None, reg::A, Vec::new(), None);
            if let Some(site) = self.sites.get_mut(&call)
                && site.text.is_empty()
            {
                site.text = format!("{text}, the number it returns");
            }
        }
    }

    /// A new read of the register at `pc`.
    fn start(
        &mut self,
        pc: u16,
        at: Option<u16>,
        base: Option<u16>,
        regs: u32,
        mem: Vec<u16>,
        mask: Option<u8>,
    ) {
        let frame = self.frame;
        let routine = self.calls.routine();
        if let Some(f) = self.calls.stack.last_mut()
            && f.drew.is_none()
        {
            f.drew = Some((pc, at.unwrap_or(0)));
        }
        let site = self.sites.entry(pc).or_insert_with(|| Site {
            pc,
            text: String::new(),
            base,
            ..Site::default()
        });
        site.reads += 1;
        if site.last_frame != Some(frame) {
            site.frames += 1;
            site.last_frame = Some(frame);
        }
        if let Some(at) = at {
            *site.addrs.entry(at).or_default() += 1;
        }
        if let Some(r) = routine {
            *site.routines.entry(r).or_default() += 1;
        }
        let mut tr = Tracker {
            site: pc,
            age: 0,
            regs,
            mem,
            mask_found: false,
            decided: false,
        };
        if let Some(m) = mask {
            tr.mask_found = true;
            *site.masks.entry(m).or_default() += 1;
        }
        if self.active.len() >= MAX_TRACKERS {
            self.active.remove(0);
        }
        self.active.push(tr);
    }
}

/// Runs `inputs` twice from power-on on the main Z80: once to find the
/// random registers, once to follow their reads. Frames up to `from` run
/// unwatched.
pub(crate) fn find(set: &Romset, inputs: &[InputFrame], from: u64) -> Result<Found> {
    let mut t = Tapper::power_on(set);
    let mut scan = Scan {
        bytes: vec![ByteStats::default(); RAM_LEN],
        calls: Calls::default(),
        a_mixed: false,
        prov: Prov::default(),
        refresh: vec![0; RAM_LEN],
        frame: 0,
    };
    for (n, input) in inputs.iter().enumerate() {
        if (n as u64) < from {
            t.step(controls(input));
            continue;
        }
        let (trace, accesses) = frame(&mut t, controls(input));
        for s in steps(&trace, &accesses) {
            if s.e.interrupt.is_some() {
                scan.calls.interrupt(s.e.pc);
            } else {
                scan.instruction(&s);
            }
        }
        scan.frame_end(&t);
    }
    let registers = scan.registers();
    let mut sites = Vec::new();
    if !registers.is_empty() {
        let mut t = Tapper::power_on(set);
        let mut readers = Readers::new(&registers);
        for (n, input) in inputs.iter().enumerate() {
            if (n as u64) < from {
                t.step(controls(input));
                continue;
            }
            let (trace, accesses) = frame(&mut t, controls(input));
            for s in steps(&trace, &accesses) {
                if s.e.interrupt.is_some() {
                    readers.interrupt(s.e.pc);
                } else {
                    readers.instruction(&s);
                }
            }
            readers.frame += 1;
        }
        sites = readers.sites.into_values().collect();
    }
    Ok(Found {
        frames: scan.frame,
        registers,
        sites,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The made-up busy program steps a shift register at e002 every loop
    /// (`rlca`, `xor $1c`, `ld ($e002),a`) and jumps through a table by its
    /// low bit (`and $01` ... `jp (hl)`): the first run finds e002 as a
    /// register stored from an exclusive or, and the second finds the read
    /// at 000c as its step (the store with the same SP follows) and the
    /// table's index.
    #[test]
    fn finds_the_busy_programs_register() {
        let set = gp_arcade::testrom::busy();
        let inputs = vec![InputFrame::default(); 40];
        let found = find(&set, &inputs, 0).unwrap();
        assert_eq!(found.frames, 40);
        let r = found
            .registers
            .iter()
            .find(|r| r.contains(0xe002))
            .unwrap_or_else(|| panic!("{:?}", found.registers));
        assert_eq!(r.kind, Kind::Mixed);
        assert!(r.writers.contains_key(&0x0014));
        let site = found.sites.iter().find(|s| s.pc == 0x000c).unwrap();
        assert_eq!(site.text, "ld a,($e002)");
        assert!(site.is_step(), "the read is the register's own step");
    }

    /// A program that draws: it steps e002 as the busy one does, and in
    /// a routine one call deeper reads it, masks it and branches on it.
    #[test]
    fn finds_a_draw_site() {
        let main: &[u8] = &[
            // 0000: ld sp,e7f0 ; call 0010 (step) ; call 0030 ; jr 0003
            0x31, 0xf0, 0xe7, 0xcd, 0x10, 0x00, 0xcd, 0x30, 0x00, 0x18, 0xf8, 0, 0, 0, 0, 0,
            // 0010: ld a,(e002) ; rlca ; jr nc,+2 ; xor 1c ; ld (e002),a ; ret
            0x3a, 0x02, 0xe0, 0x07, 0x30, 0x02, 0xee, 0x1c, 0x32, 0x02, 0xe0, 0xc9, 0, 0, 0,
            0, //
            // 0020: ld a,(e002) ; and 03 ; jr z,+4 ; ld hl,e010 ; inc (hl) ; ret
            0x3a, 0x02, 0xe0, 0xe6, 0x03, 0x28, 0x04, 0x21, 0x10, 0xe0, 0x34, 0xc9, 0, 0, 0,
            0, //
            // 0030: call 0020 ; ret
            0xcd, 0x20, 0x00, 0xc9,
        ];
        let set = gp_arcade::testrom::romset(main, &[0x18, 0xfe]);
        let found = find(&set, &[InputFrame::default(); 40], 0).unwrap();
        assert!(found.registers.iter().any(|r| r.contains(0xe002)));
        let draw = found.sites.iter().find(|s| s.pc == 0x0020).unwrap();
        assert!(draw.proposed(), "{}", draw.note());
        assert_eq!(draw.mask(), Some(0x03));
        let (taken, not) = draw.branches[&0x0025];
        assert!(taken > 0 && not > 0, "both ways: {taken} {not}");
        let step = found.sites.iter().find(|s| s.pc == 0x0010).unwrap();
        assert!(step.is_step());
    }

    /// A generator routine that returns its number in A, as Tapper's does:
    /// the call is the draw site, and the caller's branch on A decides.
    #[test]
    fn finds_a_draw_through_a_routines_return() {
        let main: &[u8] = &[
            // 0000: ld sp,e7f0 ; call 0010 ; and 01 ; jr z,+3 ; ld (e010),a ; jr 0003
            0x31, 0xf0, 0xe7, 0xcd, 0x10, 0x00, 0xe6, 0x01, 0x28, 0x03, 0x32, 0x10, 0xe0, 0x18,
            0xf4, 0, //
            // 0010: ld a,(e002) ; rlca ; jr nc,+2 ; xor 1c ; ld c,a ; ld a,r ; xor c
            0x3a, 0x02, 0xe0, 0x07, 0x30, 0x02, 0xee, 0x1c, 0x4f, 0xed, 0x5f, 0xa9,
            // 001c: ld (e002),a ; ret
            0x32, 0x02, 0xe0, 0xc9,
        ];
        let set = gp_arcade::testrom::romset(main, &[0x18, 0xfe]);
        let found = find(&set, &[InputFrame::default(); 30], 0).unwrap();
        assert!(found.registers.iter().any(|r| r.contains(0xe002)));
        let call = found.sites.iter().find(|s| s.pc == 0x0003).unwrap();
        assert_eq!(call.text, "call $0010, the number it returns");
        assert!(call.proposed(), "{}", call.note());
        assert_eq!(call.ram(), 0xe002);
        assert!(call.branches.contains_key(&0x0008));
    }
}
