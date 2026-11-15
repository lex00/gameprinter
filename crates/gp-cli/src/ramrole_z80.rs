//! Each RAM byte's role on an arcade board, from a trace of the main Z80:
//! gp_nes::ramrole's classifier ([`RamTrace`], the same rules in the same
//! order) fed from gp-arcade's instruction trace and access log, where
//! gp_nes::ramrole::Tracer feeds it from the 6502's bus.
//!
//! What the classifier is told, frame by frame:
//!
//! | Event | On the MCR board's main Z80 |
//! |---|---|
//! | A call | `call`, `call cc` taken, `rst`, and an interrupt's handler; it has returned once the stack pointer is back at or above where it was before the call (as the 6502 tracer pops) |
//! | A jump through a pointer | `jp (hl)`, `jp (ix)`, `jp (iy)`: the routine goes on as its target |
//! | A read or a write | Each access the access log has in work RAM (E000-E7FF), by its index from E000; the stack's own (`push`, `pop`, `call`, `ret`, `ex (sp),..`, an interrupt's push) aren't the game's bytes |
//! | A byte used as a pointer or an index | A byte loaded into a register that then addresses memory (`ld hl,(nn)` then `ld a,(hl)`, `ld e,(ix+2)` then `ld a,(de)`) |
//! | A sound write | An `out` to the sound board's latches, ports 1C to 1F |
//! | A picture write | A write to sprite, video or palette RAM (E800 on) |
//! | A controller read | An `in` from input ports 00 to 02 (the coin and start, the players' sticks; the board mirrors them at 08 and up), not the dip switches at 03 and 04 |
//!
//! Each byte's [`gp_nes::ramrole::RamRole::addr`] is its index from E000,
//! as the measure's other counts are.

use gp_arcade::dasm::{self, Addr, Flow, Insn, Op, reg};
use gp_arcade::mcr::{Access, AccessKind, Trace};
use gp_arcade::{Controls, Tapper};
use gp_nes::ramrole::{Classified, RamTrace};

/// The main board's work RAM.
const RAM: u16 = 0xe000;
const RAM_END: u16 = 0xe800;

/// The classifier and what the Z80 feeding it keeps between instructions.
#[derive(Debug, Clone)]
pub(crate) struct Z80Roles {
    pub(crate) trace: RamTrace,
    /// Each open call's stack pointer from before it.
    sp: Vec<u16>,
    /// The work RAM byte each eight-bit register was loaded from, by its
    /// bit in [`reg`].
    src: [Option<u16>; 16],
    /// The frame's last instruction: one the frame's end cut into makes
    /// its last accesses at the next frame's start.
    last: Option<Insn>,
}

impl Default for Z80Roles {
    fn default() -> Self {
        Self::new()
    }
}

/// The bit index of each eight-bit register a pair or index names.
fn bits(r: u32) -> impl Iterator<Item = usize> {
    (0..16).filter(move |b| r & (1 << b) != 0 && 1 << b != reg::F)
}

impl Z80Roles {
    pub(crate) fn new() -> Self {
        Self {
            trace: RamTrace::new(),
            sp: Vec::new(),
            src: [None; 16],
            last: None,
        }
    }

    /// One frame's main-CPU instructions (`trace`, the main CPU's only or
    /// both, in time order) with their accesses.
    pub(crate) fn feed(&mut self, trace: &[Trace], accesses: &[Access]) {
        self.trace.frame_start();
        let main: Vec<&Trace> = trace.iter().filter(|e| e.cpu == 0).collect();
        let acc: Vec<&Access> = accesses.iter().filter(|a| a.cpu == 0).collect();
        let mut j = 0;
        // Accesses before the frame's first instruction are the last
        // one's of the frame before, which the frame's end cut into.
        if let Some(first) = main.first() {
            while j < acc.len() && acc[j].time < first.time {
                j += 1;
            }
            if let Some(insn) = self.last.take() {
                self.accesses(&insn, &acc[..j], None);
            }
        }
        for (i, e) in main.iter().enumerate() {
            // Calls the stack pointer says have returned.
            while self.sp.last().is_some_and(|&s| e.sp >= s) {
                self.sp.pop();
                self.trace.ret();
            }
            let next = main.get(i + 1);
            let end = next.map(|n| n.time);
            let from = j;
            while j < acc.len() && end.is_none_or(|t| acc[j].time < t) {
                j += 1;
            }
            let mine = &acc[from..j];
            if e.interrupt.is_some() {
                // The handler is called with the stack pointer as it was
                // before the push of the address it returns to.
                self.sp.push(e.sp.wrapping_add(2));
                self.trace.call(u32::from(e.pc));
                continue;
            }
            let insn = dasm::decode(&e.bytes, e.pc);
            // An interrupt after this instruction pushed its return
            // address: those writes are the stack's.
            let pushed = next
                .filter(|n| n.interrupt.is_some())
                .map(|n| [n.sp, n.sp.wrapping_add(1)]);
            self.accesses(&insn, mine, pushed);
            self.registers(&insn, mine);
            if next.is_none() {
                self.last = Some(insn.clone());
            }
            let fall = e.pc.wrapping_add(u16::from(insn.len));
            let went = next.filter(|n| n.interrupt.is_none()).map(|n| n.pc);
            match insn.flow {
                Flow::Call(t) | Flow::Rst(t) => {
                    self.sp.push(e.sp);
                    self.trace.call(u32::from(t));
                }
                Flow::CallIf(t) if went.is_some_and(|w| w != fall) => {
                    self.sp.push(e.sp);
                    self.trace.call(u32::from(t));
                }
                Flow::Indirect(_) => {
                    if let Some(w) = went {
                        self.trace.jump(u32::from(w));
                    }
                }
                _ => {}
            }
        }
    }

    /// An instruction's accesses, given to the classifier.
    fn accesses(&mut self, insn: &Insn, mine: &[&Access], pushed: Option<[u16; 2]>) {
        let stack = insn.mem.is_some_and(|m| m.addr == Addr::Stack);
        let a_regs = crate::draws_find_z80::address_regs(insn) & !reg::SP;
        for a in mine {
            match a.kind {
                AccessKind::Read | AccessKind::Write if stack => {}
                AccessKind::Write if pushed.is_some_and(|p| p.contains(&a.addr)) => {}
                AccessKind::Read if (RAM..RAM_END).contains(&a.addr) => {
                    self.trace.read(a.addr - RAM);
                }
                AccessKind::Write if (RAM..RAM_END).contains(&a.addr) => {
                    self.trace.write(a.addr - RAM);
                }
                AccessKind::Write if a.addr >= RAM_END => self.trace.other_io_write(),
                AccessKind::Out if (0x1c..=0x1f).contains(&(a.addr & 0xff)) => {
                    self.trace.apu_write();
                }
                // Ports 0 to 2 (and their mirrors, as the board decodes
                // them): the coin and start, and the players' sticks;
                // 3 and 4 are the dip switches.
                AccessKind::In if (a.addr & 0xff & !0x18) <= 0x02 => self.trace.controller_read(),
                _ => {}
            }
        }
        // The bytes the address registers were loaded from are used as a
        // pointer or an index.
        if a_regs != 0 && !stack {
            let used: Vec<u16> = bits(a_regs).filter_map(|b| self.src[b]).collect();
            for u in used {
                self.trace.used(u);
            }
        }
    }

    /// Where each register's value came from, after the instruction.
    fn registers(&mut self, insn: &Insn, mine: &[&Access]) {
        let w = insn.writes & !reg::F;
        if w == 0 {
            return;
        }
        let loaded = |k: usize| -> Option<u16> {
            mine.iter()
                .filter(|a| a.kind == AccessKind::Read && (RAM..RAM_END).contains(&a.addr))
                .nth(k)
                .map(|a| a.addr - RAM)
        };
        let copy = insn.op == Op::Load
            && insn.mem.is_none()
            && insn.io.is_none()
            && !insn.constant
            && !insn.text.starts_with("ex");
        let from_mem = insn.op == Op::Load
            && insn
                .mem
                .is_some_and(|m| m.read && !m.write && m.addr != Addr::Stack);
        let dst: Vec<usize> = bits(w).collect();
        let srcs: Vec<Option<u16>> = if from_mem {
            // A word load fills the low register from the first byte.
            let mut order = dst.clone();
            order.sort_by_key(|b| {
                // L, E, C, IXL, IYL before H, D, B, IXH, IYH.
                matches!(1u32 << b, reg::H | reg::D | reg::B | reg::IXH | reg::IYH)
            });
            let mut out = vec![None; dst.len()];
            for (k, b) in order.iter().enumerate() {
                let at = dst.iter().position(|x| x == b).expect("a register");
                out[at] = loaded(k);
            }
            out
        } else if copy {
            let from: Vec<usize> = bits(insn.reads).collect();
            dst.iter()
                .enumerate()
                .map(|(k, _)| from.get(k).and_then(|&b| self.src[b]))
                .collect()
        } else {
            vec![None; dst.len()]
        };
        for (b, s) in dst.into_iter().zip(srcs) {
            self.src[b] = s;
        }
    }

    /// A new run from power-on: the calls left open by the last one end,
    /// and the classifier goes on adding to what it has.
    pub(crate) fn next_run(&mut self) {
        while self.trace.depth() > 0 {
            self.trace.ret();
        }
        self.sp.clear();
        self.src = [None; 16];
        self.last = None;
    }

    /// The frame ends with the work RAM as `ram` (2 KB from E000).
    pub(crate) fn frame_end(&mut self, ram: &[u8]) {
        self.trace.frame_end(ram);
    }

    pub(crate) fn classify(&self) -> Classified {
        self.trace.classify()
    }
}

/// One frame of `t` with its trace and access log on, fed to `roles`: both
/// CPUs' instructions, in time order, for a caller that wants the trace
/// too (crate::covered's code features).
pub(crate) fn step(t: &mut Tapper, c: Controls, roles: &mut Z80Roles) -> Vec<Trace> {
    t.set_trace(true);
    t.set_accesses(true);
    t.step(c);
    let mut trace = t.take_trace();
    let accesses = t.take_accesses();
    t.set_trace(false);
    t.set_accesses(false);
    trace.sort_by_key(|e| e.time);
    roles.feed(&trace, &accesses);
    let ram: Vec<u8> = (RAM..RAM_END).map(|a| t.peek(a)).collect();
    roles.frame_end(&ram);
    trace
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gp_nes::ramrole::Role;

    /// A made-up main loop: a routine at 0040 reads the controller into
    /// E000; one at 0050 steps E010 and writes it to the sound latch; the
    /// loop keeps a pointer in E020 and reads through it, steps E030 from
    /// its own last value and writes the picture.
    pub(crate) fn program() -> gp_arcade::Romset {
        let mut main = vec![
            0x31, 0xf0, 0xe7, // 0000: ld sp,e7f0
            0xcd, 0x40, 0x00, // 0003: call 0040
            0xcd, 0x50, 0x00, // 0006: call 0050
            0x21, 0x00, 0xe1, // 0009: ld hl,e100
            0x22, 0x20, 0xe0, // 000c: ld (e020),hl
            0x2a, 0x20, 0xe0, // 000f: ld hl,(e020)
            0x7e, // 0012: ld a,(hl)
            0x3a, 0x30, 0xe0, // 0013: ld a,(e030)
            0x3c, // 0016: inc a
            0x32, 0x30, 0xe0, // 0017: ld (e030),a
            0x32, 0x00, 0xf0, // 001a: ld (f000),a
            0x18, 0xe4, // 001d: jr 0003
        ];
        main.resize(0x40, 0);
        main.extend([0xdb, 0x01, 0x32, 0x00, 0xe0, 0xc9]); // 0040: in a,(01) ; ld (e000),a ; ret
        main.resize(0x50, 0);
        // 0050: ld a,(e010) ; inc a ; ld (e010),a ; out (1c),a ; ret
        main.extend([0x3a, 0x10, 0xe0, 0x3c, 0x32, 0x10, 0xe0, 0xd3, 0x1c, 0xc9]);
        gp_arcade::testrom::romset(&main, &[0x18, 0xfe])
    }

    #[test]
    fn a_z80_trace_gives_each_bytes_role() {
        let set = program();
        let mut t = Tapper::power_on(&set);
        let mut r = Z80Roles::new();
        for n in 0..12 {
            step(
                &mut t,
                Controls {
                    tap: n % 3 == 0,
                    ..Controls::default()
                },
                &mut r,
            );
        }
        let c = r.classify();
        let role = |a: u16| c.roles.iter().find(|x| x.addr == a).map(|x| x.role);
        assert_eq!(role(0x000), Some(Role::Input), "{c:?}");
        assert_eq!(role(0x010), Some(Role::Sound), "{c:?}");
        assert_eq!(role(0x020), Some(Role::Pointer), "{c:?}");
        assert_eq!(role(0x030), Some(Role::State), "{c:?}");
        assert_eq!(c.input_routines, vec![0x40]);
        assert_eq!(c.sound_entries, vec![0x50]);
        // The stack (E7EE, E7EF for the calls' return addresses) isn't
        // the game's.
        assert!(role(0x7ee).is_none() && role(0x7ef).is_none(), "{c:?}");
        // A traced frame ends where an untraced one does.
        let mut plain = Tapper::power_on(&set);
        for n in 0..12 {
            plain.step(Controls {
                tap: n % 3 == 0,
                ..Controls::default()
            });
        }
        assert_eq!(plain.state_hash(), t.state_hash());
    }
}
