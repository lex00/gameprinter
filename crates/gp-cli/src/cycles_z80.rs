//! `gp cycles` on an arcade romset: the main Z80's per-frame work, read
//! from gp-arcade's instruction trace (each instruction's cycle count since
//! power-on) and its access log (each memory access with the instruction
//! that made it), as cycles.rs reads the 6502's from gp-nes.
//!
//! What maps from the NES:
//!
//! - A call is `call nn` (taken, when conditional) or `rst p` to the
//!   routine; it ends at the first instruction after it whose stack pointer
//!   is back at or above the caller's (a `ret`, or a routine that drops its
//!   return address and returns to its caller's caller). Its cycles run
//!   from the call's start to that instruction's, less any interrupt the
//!   board took inside it (Tapper's game loop runs with interrupts on, so
//!   a call there can have the CTC's handler in the middle); that
//!   handler's accesses aren't the call's either.
//! - The frame's handler is the first interrupt the main CPU takes in the
//!   frame (the CTC's, [`gp_arcade::mcr::Trace::interrupt`]), from its
//!   acknowledge to the first instruction after it whose stack pointer is
//!   back above the return address the interrupt pushed. The board raises
//!   it near the frame's start, so a handler may end in the next frame,
//!   which `split` runs to finish it.
//! - RAM is the main CPU's work RAM, E000-E7FF (2048 bytes, as the NES's
//!   2 KB); the stack's accesses are left out by the instruction that made
//!   them (push, pop, call, ret), not by address.
//! - `check`'s cost is gp-arcade's own Z80 (MAME's, cycle for cycle) run
//!   alone from the call, on the ROM and the RAM at the call (the frame's
//!   start with the frame's writes before the call applied), the call's
//!   registers from the trace (the refresh register too, which `ld a,r`
//!   reads), the second register set as the frame
//!   started, the input ports as the frame read them, and no interrupt:
//!   gp-cycles' cost-only CPU is the 6502's.
//!
//! `chain` stays the NES's: it predicts each NMI from the PPU's vblank
//! grid, and the MCR board's interrupt comes from its CTC counting the
//! video timing, which nothing here models outside gp-arcade's board.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_arcade::dasm::{self, Addr, Flow};
use gp_arcade::mcr::{Access, AccessKind, Trace};
use gp_arcade::z80::{Z80, Z80Bus};
use gp_arcade::{Controls, Romset, Tapper};
use gp_evidence::{Hash, InputLog};
use serde_json::json;

use crate::Outcome;
use crate::cycles::hex as routine_hex;

/// Work RAM on the main CPU.
const RAM: u16 = 0xe000;
const RAM_LEN: usize = 0x800;

/// The romset in a zip's bytes.
pub(crate) fn romset(bytes: &[u8]) -> Result<Romset> {
    Romset::from_zip(bytes).map_err(|e| anyhow::anyhow!("{e}"))
}

fn controls(f: &gp_evidence::input_log::InputFrame) -> Result<Controls> {
    let bits = f.buttons_word();
    if bits > 0xff || f.reset {
        bail!("an arcade input frame holds bits 0 to 7 and no reset; this one holds {bits:#x}");
    }
    Ok(Controls::from_bits(bits))
}

/// One frame of the main CPU as the trace and access log saw it.
struct Frame {
    /// The frame's number from power-on.
    n: u64,
    trace: Vec<Trace>,
    /// Each trace entry's accesses (an interrupt entry has none).
    acc: Vec<Vec<Access>>,
    /// Every byte the main CPU could read at the frame's start
    /// ([`Flat::of`]).
    start: Box<Flat>,
    /// The main CPU as the frame started, for what the trace doesn't hold
    /// (the second register set).
    main: Z80,
}

impl Frame {
    /// The work RAM at `time` in the frame: the start with the frame's
    /// writes before it.
    fn ram_at(&self, time: gp_arcade::attotime::Attotime) -> Flat {
        let mut m = (*self.start).clone();
        for a in self.acc.iter().flatten() {
            if a.kind == AccessKind::Write && a.time < time {
                m.write(a.addr, a.value);
            }
        }
        m
    }

    /// The input ports as the frame read them, by port's low byte.
    fn ports(&self) -> BTreeMap<u8, u8> {
        let mut p = BTreeMap::new();
        for a in self.acc.iter().flatten() {
            if a.kind == AccessKind::In {
                p.entry((a.addr & 0xff) as u8).or_insert(a.value);
            }
        }
        p
    }
}

/// Runs a log's frames on the board, handing frames FROM to TO (numbered
/// from 1) to `each` with their trace and accesses.
fn frames(
    set: &Romset,
    log: &InputLog,
    (from, to): (u64, u64),
    each: &mut dyn FnMut(&Frame, &Tapper) -> Result<()>,
) -> Result<()> {
    let mut t = Tapper::power_on(set);
    for (i, fr) in log.frames.iter().enumerate().take(to as usize) {
        let n = i as u64 + 1;
        let c = controls(fr).with_context(|| format!("frame {n}"))?;
        if n < from {
            t.step(c);
            continue;
        }
        let start = Box::new(Flat::of(&t));
        let main = t.main.clone();
        t.set_trace(true);
        t.set_accesses(true);
        t.step(c);
        let mut trace: Vec<Trace> = t.take_trace().into_iter().filter(|e| e.cpu == 0).collect();
        let mut accesses: Vec<Access> = t
            .take_accesses()
            .into_iter()
            .filter(|a| a.cpu == 0)
            .collect();
        t.set_trace(false);
        t.set_accesses(false);
        trace.sort_by_key(|e| e.time);
        accesses.sort_by_key(|a| a.time);
        // Each access goes to the last instruction at or before it that
        // has its PC (an interrupt's pushes are put on the instruction it
        // came after).
        let mut acc = vec![Vec::new(); trace.len()];
        let mut k = 0usize;
        for a in accesses {
            while k + 1 < trace.len() && trace[k + 1].time <= a.time {
                k += 1;
            }
            let mut j = k;
            while j > 0 && (trace[j].pc != a.pc || trace[j].interrupt.is_some()) {
                j -= 1;
            }
            if let Some(v) = acc.get_mut(j) {
                v.push(a);
            }
        }
        let f = Frame {
            n,
            trace,
            acc,
            start,
            main,
        };
        each(&f, &t)?;
    }
    Ok(())
}

/// The routine a call or restart at `e` goes to, when `next` (the main
/// CPU's next instruction) shows it was taken.
fn called(e: &Trace, next: Option<&Trace>) -> Option<u16> {
    let i = dasm::decode(&e.bytes, e.pc);
    match i.flow {
        Flow::Call(t) | Flow::Rst(t) => Some(t),
        Flow::CallIf(t) if next.is_some_and(|n| n.pc == t) => Some(t),
        _ => None,
    }
}

/// The next instruction (not an interrupt entry) after `i`.
fn next_insn(trace: &[Trace], i: usize) -> Option<&Trace> {
    trace[i + 1..].iter().find(|e| e.interrupt.is_none())
}

/// Whether an access by `e` is the stack's (push, pop, call, ret, an
/// interrupt's push).
fn stack_access(e: &Trace) -> bool {
    let i = dasm::decode(&e.bytes, e.pc);
    matches!(i.mem.map(|m| m.addr), Some(Addr::Stack))
        || matches!(
            i.flow,
            Flow::Call(_) | Flow::CallIf(_) | Flow::Rst(_) | Flow::Ret | Flow::RetIf
        )
}

/// What the main CPU reads, flat: the ROM below E000, work RAM, sprite
/// RAM and video RAM at their canonical addresses (mirrors folded as the
/// board folds them), 0xff elsewhere.
#[derive(Clone)]
pub(crate) struct Flat {
    mem: Vec<u8>,
}

fn fold(a: u16) -> Option<usize> {
    let a = usize::from(a);
    match a {
        0x0000..=0xdfff => Some(a),
        0xe000..=0xe7ff => Some(a),
        0xe800..=0xebff => Some(0xe800 + (a & 0x1ff)),
        0xf000..=0xf7ff => Some(a),
        _ => None,
    }
}

impl Flat {
    fn of(t: &Tapper) -> Self {
        let mut mem = vec![0xff; 0x1_0000];
        for (a, b) in mem.iter_mut().enumerate() {
            if fold(a as u16) == Some(a) {
                *b = t.peek(a as u16);
            }
        }
        Self { mem }
    }

    fn read(&self, a: u16) -> u8 {
        fold(a).map_or(0xff, |i| self.mem[i])
    }

    fn write(&mut self, a: u16, v: u8) {
        if a >= RAM
            && let Some(i) = fold(a)
        {
            self.mem[i] = v;
        }
    }

    fn ram(&self) -> &[u8] {
        &self.mem[usize::from(RAM)..usize::from(RAM) + RAM_LEN]
    }
}

/// The flat machine a call's cost runs on: no interrupts, the ports as
/// the frame read them. Before each instruction it looks for the call's
/// end: the stack pointer back at `sp`.
struct Alone<'a> {
    m: Flat,
    ports: &'a BTreeMap<u8, u8>,
    sp: u16,
    seen: u32,
    /// The cycles left when the call ended.
    ended: Option<i32>,
}

impl Alone<'_> {
    fn stop(&self, icount: &mut i32) {
        if self.ended.is_some() {
            *icount = 0;
        }
    }
}

impl Z80Bus for Alone<'_> {
    fn read(&mut self, addr: u16, icount: &mut i32) -> u8 {
        self.stop(icount);
        self.m.read(addr)
    }
    fn write(&mut self, addr: u16, value: u8, icount: &mut i32) {
        self.stop(icount);
        self.m.write(addr, value);
    }
    fn fetch(&mut self, addr: u16) -> u8 {
        self.m.read(addr)
    }
    fn io_read(&mut self, port: u16, icount: &mut i32) -> u8 {
        self.stop(icount);
        self.ports
            .get(&((port & 0xff) as u8))
            .copied()
            .unwrap_or(0xff)
    }
    fn io_write(&mut self, _port: u16, _value: u8, icount: &mut i32) {
        self.stop(icount);
    }
    fn irq_ack(&mut self, _icount: &mut i32) -> i32 {
        0xff
    }
    fn reti(&mut self, _icount: &mut i32) {}
    fn instruction(&mut self, cpu: &Z80) {
        if self.ended.is_none() && self.seen > 0 && cpu.sp >= self.sp {
            self.ended = Some(cpu.icount);
        }
        self.seen += 1;
    }
}

/// Cycles a call may take run alone before it counts as not coming back.
const BUDGET: i32 = 200_000;

/// The cycles a call at `e` takes run alone from the frame's state at it,
/// up to the first instruction with the stack pointer back at `e`'s;
/// `None` when it doesn't come back within [`BUDGET`] cycles.
fn cost_alone(f: &Frame, e: &Trace, main: &Z80) -> Option<u64> {
    let ports = f.ports();
    let mut bus = Alone {
        m: f.ram_at(e.time),
        ports: &ports,
        sp: e.sp,
        seen: 0,
        ended: None,
    };
    let mut cpu = main.clone();
    cpu.pc = e.pc;
    cpu.sp = e.sp;
    cpu.af = e.af;
    cpu.bc = e.bc;
    cpu.de = e.de;
    cpu.hl = e.hl;
    cpu.ix = e.ix;
    cpu.iy = e.iy;
    cpu.r = e.r;
    cpu.r2 = e.r2;
    cpu.iff1 = 0;
    cpu.iff2 = 0;
    cpu.irq_state = 0;
    cpu.nmi_pending = false;
    cpu.halt = 0;
    cpu.m_ref = 0xffff00;
    cpu.icount = BUDGET;
    cpu.execute_run(&mut bus);
    bus.ended.map(|left| (BUDGET - left) as u64)
}

/// The cycles MAME's NMOS Z80 takes to acknowledge an interrupt in mode 2
/// (Tapper's: the CTC's vector) and mode 1, before the handler's first
/// instruction.
const ACK_IM2: u64 = 19;
const ACK_IM1: u64 = 13;

/// A call being followed.
struct Open {
    routine: u16,
    cycles: u64,
    /// The stack pointer at the call: it is back there once the call ends.
    sp: u16,
    reads: Vec<(u16, u8)>,
    seen: BTreeSet<u16>,
    predicted: Option<u64>,
}

/// A call that ended: the routine, its cycles, the RAM bytes it read
/// before writing them, and its cost run alone.
type Call = (u16, u64, Vec<(u16, u8)>, Option<u64>);

/// `readset` and `check`: each call of the routines in `routines` in
/// frames FROM to TO, with its cycles, the RAM bytes it read before
/// writing them, and with `predict` its cost run alone.
fn calls(
    set: &Romset,
    log: &InputLog,
    window: (u64, u64),
    routines: &[u16],
    predict: bool,
) -> Result<Vec<Call>> {
    let mut open: Vec<Open> = Vec::new();
    let mut done = Vec::new();
    // An interrupt taken inside a call: its start and the stack pointer
    // above which it has returned. Its cycles and accesses aren't the
    // call's.
    let mut irq: Option<(u64, u16)> = None;
    frames(set, log, window, &mut |f, _| {
        for (i, e) in f.trace.iter().enumerate() {
            if e.interrupt.is_some() {
                if irq.is_none() && !open.is_empty() {
                    // The entry is at the handler's first instruction; the
                    // acknowledge before it (the vector read and the
                    // pushes) is the interrupt's too.
                    let ack = if f.main.im == 2 { ACK_IM2 } else { ACK_IM1 };
                    irq = Some((e.cycles - ack, e.sp.wrapping_add(2)));
                }
                continue;
            }
            if let Some((c0, sp)) = irq {
                if e.sp < sp {
                    continue;
                }
                irq = None;
                for o in &mut open {
                    o.cycles += e.cycles - c0;
                }
            }
            while let Some(o) = open.last()
                && e.sp >= o.sp
            {
                let o = open.pop().expect("an open call");
                done.push((o.routine, e.cycles - o.cycles, o.reads, o.predicted));
            }
            if !stack_access(e) {
                for a in &f.acc[i] {
                    if !(RAM..RAM + RAM_LEN as u16).contains(&a.addr) {
                        continue;
                    }
                    for o in &mut open {
                        match a.kind {
                            AccessKind::Read if o.seen.insert(a.addr) => {
                                o.reads.push((a.addr, a.value));
                            }
                            AccessKind::Write => {
                                o.seen.insert(a.addr);
                            }
                            _ => {}
                        }
                    }
                }
            }
            if let Some(r) = called(e, next_insn(&f.trace, i))
                && routines.contains(&r)
            {
                open.push(Open {
                    routine: r,
                    cycles: e.cycles,
                    sp: e.sp,
                    reads: Vec::new(),
                    seen: BTreeSet::new(),
                    predicted: if predict {
                        cost_alone(f, e, &f.main)
                    } else {
                        None
                    },
                });
            }
        }
        Ok(())
    })?;
    Ok(done)
}

/// `gp cycles slice` on a romset: the main CPU's ROM.
pub(crate) fn slice(bytes: &[u8], home: &Path) -> Result<Outcome> {
    let set = romset(bytes)?;
    let hash = Hash::of(bytes).to_string();
    let dir = home.join("work").join("cycles").join(&hash);
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    fs::write(dir.join("prg.bin"), &set.maincpu)?;
    let meta = json!({"rom_blake3": hash, "base": 0, "prg_len": set.maincpu.len(), "set": set.name, "cpu": "main"});
    fs::write(dir.join("slice.json"), serde_json::to_string_pretty(&meta)?)?;
    Ok(Outcome {
        json: meta,
        text: format!("wrote {}", dir.display()),
        exit: 0,
    })
}

/// `gp cycles readset` on a romset.
pub(crate) fn readset(
    bytes: &[u8],
    log: &InputLog,
    window: (u64, u64),
    routine: &str,
) -> Result<Outcome> {
    let set = romset(bytes)?;
    let r = routine_hex(routine)?;
    let mut costs: BTreeMap<Vec<(u16, u8)>, BTreeSet<u64>> = BTreeMap::new();
    let mut addrs: BTreeSet<u16> = BTreeSet::new();
    let found = calls(&set, log, window, &[r], false)?;
    let n = found.len() as u64;
    for (_, cyc, reads, _) in found {
        addrs.extend(reads.iter().map(|(a, _)| *a));
        costs.entry(reads).or_default().insert(cyc);
    }
    let ambiguous = costs.values().filter(|v| v.len() > 1).count();
    let list: Vec<String> = addrs.iter().map(|a| format!("{a:04x}")).collect();
    let cyc: BTreeSet<u64> = costs.values().flatten().copied().collect();
    Ok(Outcome {
        json: json!({"calls": n, "read_states": costs.len(), "ambiguous": ambiguous, "bytes": list,
                     "cycles": cyc.iter().collect::<Vec<_>>()}),
        text: format!(
            "{n} calls, {} distinct read-states, {ambiguous} ambiguous; {} bytes read: {}\ncycles a call: {}",
            costs.len(),
            list.len(),
            list.join(" "),
            cyc.iter().map(u64::to_string).collect::<Vec<_>>().join(" ")
        ),
        exit: 0,
    })
}

/// `gp cycles check` on a romset.
pub(crate) fn check(
    bytes: &[u8],
    log: &InputLog,
    window: (u64, u64),
    routines: &[String],
) -> Result<Outcome> {
    let set = romset(bytes)?;
    let rs: Vec<u16> = routines
        .iter()
        .map(|s| routine_hex(s))
        .collect::<Result<_>>()?;
    let mut stats: BTreeMap<u16, (u64, u64)> = BTreeMap::new();
    for (t, real, _, pred) in calls(&set, log, window, &rs, true)? {
        let e = stats.entry(t).or_default();
        e.0 += 1;
        e.1 += u64::from(pred == Some(real));
    }
    let lines: Vec<String> = stats
        .iter()
        .map(|(t, (n, ok))| format!("{t:04x}: {ok} of {n} calls exact"))
        .collect();
    let j: Vec<_> = stats
        .iter()
        .map(|(t, (n, ok))| json!({"routine": format!("{t:04x}"), "calls": n, "exact": ok}))
        .collect();
    Ok(Outcome {
        json: json!({"routines": j}),
        text: lines.join("\n"),
        exit: 0,
    })
}

/// The handler being followed in `split`.
struct Handler {
    frame: u64,
    cycles: u64,
    /// Back above this, the handler has returned.
    sp: u16,
    open: Vec<(u16, u64, u16)>,
    by: BTreeMap<u16, u64>,
    hist: BTreeMap<u16, (u32, u64)>,
    last: Option<(u16, u64)>,
}

/// A frame's handler: its cycles, each routine's, and each address's runs
/// and cycles.
type Row = (u64, BTreeMap<u16, u64>, BTreeMap<u16, (u32, u64)>);

/// `gp cycles split` on a romset.
pub(crate) fn split(
    bytes: &[u8],
    log: &InputLog,
    (from, to): (u64, u64),
    out: &Path,
    pcs: Option<&Path>,
    ram_out: Option<&Path>,
) -> Result<Outcome> {
    let set = romset(bytes)?;
    // Each frame's line, and what the handler started in it gives.
    let mut rows: BTreeMap<u64, Row> = BTreeMap::new();
    let mut rams: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    let mut h: Option<Handler> = None;
    // One frame past TO, to finish a handler TO started.
    let last = (to + 1).min(log.frames.len() as u64);
    frames(&set, log, (from, last), &mut |f, _| {
        let mine = f.n <= to;
        if mine {
            rows.entry(f.n).or_default();
        }
        let mut started = false;
        for (i, e) in f.trace.iter().enumerate() {
            if e.interrupt.is_some() {
                if h.is_none() && !started && mine {
                    started = true;
                    if ram_out.is_some() {
                        rams.insert(f.n, f.ram_at(e.time).ram().to_vec());
                    }
                    // The entry has the handler's registers: the return
                    // address is already pushed.
                    h = Some(Handler {
                        frame: f.n,
                        cycles: e.cycles,
                        sp: e.sp.wrapping_add(2),
                        open: Vec::new(),
                        by: BTreeMap::new(),
                        hist: BTreeMap::new(),
                        last: None,
                    });
                }
                continue;
            }
            let Some(x) = &mut h else { continue };
            if let Some((lp, lc)) = x.last.take() {
                let s = x.hist.entry(lp).or_default();
                s.0 += 1;
                s.1 += e.cycles - lc;
            }
            while let Some(&(t, c1, sp)) = x.open.last()
                && e.sp >= sp
            {
                x.open.pop();
                *x.by.entry(t).or_default() += e.cycles - c1;
            }
            if e.sp >= x.sp {
                let row = rows.entry(x.frame).or_default();
                *row = (
                    e.cycles - x.cycles,
                    std::mem::take(&mut x.by),
                    std::mem::take(&mut x.hist),
                );
                h = None;
                continue;
            }
            if let Some(t) = called(e, next_insn(&f.trace, i)) {
                x.open.push((t, e.cycles, e.sp));
            }
            x.last = Some((e.pc, e.cycles));
        }
        Ok(())
    })?;
    let mut lines = String::new();
    let mut pcs_bytes: Vec<u8> = vec![];
    let mut ram_bytes: Vec<u8> = vec![];
    let n = rows.len();
    for (frame, (whole, by, hist)) in &rows {
        let routines: BTreeMap<String, u64> =
            by.iter().map(|(t, c)| (format!("{t:04x}"), *c)).collect();
        lines.push_str(&serde_json::to_string(
            &json!({"frame": frame, "handler": whole, "routines": routines}),
        )?);
        lines.push('\n');
        pcs_bytes.extend_from_slice(&(*frame as u32).to_le_bytes());
        pcs_bytes.extend_from_slice(&(hist.len() as u32).to_le_bytes());
        for (pc, (k, cy)) in hist {
            pcs_bytes.extend_from_slice(&pc.to_le_bytes());
            pcs_bytes.extend_from_slice(&(*k as u16).to_le_bytes());
            pcs_bytes.extend_from_slice(&(*cy as u32).to_le_bytes());
        }
        ram_bytes.extend_from_slice(
            rams.get(frame)
                .map_or(&[0u8; RAM_LEN][..], |r| r.as_slice()),
        );
    }
    fs::write(out, lines).with_context(|| format!("writing {}", out.display()))?;
    if let Some(p) = pcs {
        fs::write(p, &pcs_bytes).with_context(|| format!("writing {}", p.display()))?;
    }
    if let Some(p) = ram_out {
        fs::write(p, &ram_bytes).with_context(|| format!("writing {}", p.display()))?;
    }
    Ok(Outcome {
        json: json!({"frames": n, "out": out.display().to_string()}),
        text: format!("{n} frames written to {}", out.display()),
        exit: 0,
    })
}

/// The original's side of `chain`'s measure: each frame's passes of the
/// idle loop at `idle`.
pub(crate) fn idle_passes(
    bytes: &[u8],
    log: &InputLog,
    window: (u64, u64),
    idle: u16,
) -> Result<Vec<(u64, u32)>> {
    let set = romset(bytes)?;
    let mut out = Vec::new();
    frames(&set, log, window, &mut |f, _| {
        let n = f
            .trace
            .iter()
            .filter(|e| e.interrupt.is_none() && e.pc == idle)
            .count() as u32;
        out.push((f.n, n));
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With the rbtapper romset: in attract mode the handler's routines
    /// are called every frame, each call's cost run alone matches the
    /// board's, and split finds one handler a frame. Skipped without it.
    #[test]
    fn calls_on_the_board_cost_what_they_cost_alone() {
        let Some(zip) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let bytes = fs::read(zip).unwrap();
        let log = InputLog {
            controllers: 1,
            frames: vec![Default::default(); 200],
            arcade: None,
        };
        let set = romset(&bytes).unwrap();
        let found = calls(&set, &log, (150, 200), &[0x0325], true).unwrap();
        assert!(found.len() >= 50, "{}", found.len());
        assert!(found.iter().all(|c| c.3 == Some(c.1)));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("split.ndjson");
        split(&bytes, &log, (150, 160), &out, None, None).unwrap();
        let text = fs::read_to_string(&out).unwrap();
        assert_eq!(text.lines().count(), 11);
        assert!(
            text.lines().all(|l| !l.contains("\"handler\":0,")),
            "{text}"
        );
    }
}
