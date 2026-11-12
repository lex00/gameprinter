//! Code activity, frame by frame: which routines ran and which code wrote
//! what (docs/gaps.md, "Code signals"), for `gp gaps`. One of the
//! collector's observers ([`crate::Collector::with_activity`]): it reads
//! each instruction through the collector's one look at it.
//!
//! A routine is a PRG offset. Every `JSR` target is one, and so is the
//! handler an NMI or IRQ enters; a caller can add more (a label file's
//! routines, so code reached by a jump table or by falling through counts
//! too). Each write is attributed to the instruction that made it (its PRG
//! offset) and to the routine on top of a shadow call stack: `JSR` and
//! interrupts push, and `RTS` and `RTI` pop every entry whose stack pointer
//! the return leaves behind, so code that pulls its return address and
//! jumps (a jump table) doesn't leave the stack out of step.

use tetanes_core::bus::Bus;
use tetanes_core::cpu::Cpu;
use tetanes_core::cpu::instr::Instr;

use crate::bits::Bits;
use crate::footprint::{reads_memory, target, writes_memory};
use crate::layout::{Layout, RAM_BYTES};

/// A PRG offset for code that isn't in PRG ROM (RAM, usually), or no
/// routine at all.
pub const NO_ROM: u32 = u32::MAX;

/// The shadow call stack's depth limit; the oldest entry goes first.
const MAX_DEPTH: usize = 64;

/// One write, attributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Write {
    /// RAM address ($0000-$07FF), VRAM address for a $2007 write, or the
    /// APU register.
    pub addr: u16,
    pub value: u8,
    /// The writing instruction's PRG offset, or [`NO_ROM`].
    pub pc: u32,
    /// The routine it ran in, or [`NO_ROM`].
    pub routine: u32,
    /// Made under a routine that writes the APU on most of its calls: the
    /// sound engine's own work.
    pub engine: bool,
}

/// What the code did in one frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameActivity {
    /// Routines entered, by PRG offset, with how many times, sorted.
    pub entered: Vec<(u32, u32)>,
    /// Every write that changed a RAM byte's value, in order.
    pub ram: Vec<Write>,
    /// The last write to each VRAM address through $2007.
    pub vram: Vec<Write>,
    /// Every APU register write ($4000-$4013, $4015, $4017).
    pub apu: Vec<Write>,
    /// Calls to the watched routines: target, the caller's PRG offset, the
    /// A register at the call, and whether the caller is the sound engine.
    pub calls: Vec<(u32, u32, u8, bool)>,
    /// With [`Activity::reads`]: each RAM byte a routine read this frame, in
    /// the order first read, as (address, routine), once per pair.
    pub reads: Vec<(u16, u32)>,
}

/// What every `JSR` to one routine looked like over a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallStats {
    pub calls: u64,
    /// Distinct calling instructions, by PRG offset, at most 64.
    pub callers: std::collections::BTreeSet<u32>,
    /// The A register values seen at a call, as a 256-bit set.
    pub a_values: [u64; 4],
    /// Whether the APU was written while it was on the call stack.
    pub reaches_apu: bool,
    /// Calls during which the APU was written (back to the interrupt that
    /// entered it, if any).
    pub apu_calls: u64,
}

impl CallStats {
    /// Whether most of its calls write the APU: a routine of the sound
    /// engine.
    pub fn is_engine(&self) -> bool {
        self.calls >= 8 && self.apu_calls * 2 >= self.calls
    }

    pub fn distinct_a(&self) -> u32 {
        self.a_values.iter().map(|w| w.count_ones()).sum()
    }

    pub fn merge(&mut self, o: &CallStats) {
        self.calls += o.calls;
        for c in &o.callers {
            if self.callers.len() < 64 {
                self.callers.insert(*c);
            }
        }
        for (a, b) in self.a_values.iter_mut().zip(o.a_values) {
            *a |= b;
        }
        self.reaches_apu |= o.reaches_apu;
        self.apu_calls += o.apu_calls;
    }
}

#[derive(Debug, Clone)]
struct Pending {
    addr: u16,
    old: u8,
    pc: u32,
    routine: u32,
    engine: bool,
}

/// The observer. Feed it each instruction with [`Activity::observe`] and
/// end each frame with [`Activity::end_frame`].
#[derive(Debug, Clone)]
pub struct Activity {
    layout: Layout,
    marks: Bits,
    counts: Vec<u32>,
    touched: Vec<u32>,
    /// Routine, the stack pointer it runs at, whether an interrupt entered
    /// it, and whether this call has written the APU.
    stack: Vec<(u32, u8, bool, bool)>,
    pending: Option<Pending>,
    vram_last: std::collections::HashMap<u16, usize>,
    frame: FrameActivity,
    pub calls: std::collections::HashMap<u32, CallStats>,
    watch: std::collections::BTreeSet<u32>,
    /// Keep only [`FrameActivity::calls`].
    pub calls_only: bool,
    /// Also list RAM reads in [`FrameActivity::reads`] (off by default: a
    /// read is far more common than a write).
    pub reads: bool,
}

impl Activity {
    /// `marks` are extra routine entries by PRG offset (a label file's), as
    /// many bits as the PRG ROM has bytes, or empty.
    pub fn new(layout: Layout, marks: &Bits) -> Self {
        Self::watching(layout, marks, Default::default())
    }

    /// [`Activity::new`], also listing each call to a routine in `watch`
    /// (PRG offsets) in [`FrameActivity::calls`].
    pub fn watching(layout: Layout, marks: &Bits, watch: std::collections::BTreeSet<u32>) -> Self {
        let marks = if marks.len() == layout.prg_rom {
            marks.clone()
        } else {
            Bits::new(layout.prg_rom)
        };
        Self {
            layout,
            marks,
            counts: vec![0; layout.prg_rom],
            touched: Vec::new(),
            stack: Vec::new(),
            pending: None,
            vram_last: Default::default(),
            frame: FrameActivity::default(),
            calls: Default::default(),
            watch,
            calls_only: false,
            reads: false,
        }
    }

    fn enter(&mut self, offset: u32) {
        let Some(c) = self.counts.get_mut(offset as usize) else {
            return;
        };
        if *c == 0 {
            self.touched.push(offset);
        }
        *c += 1;
    }

    fn top(&self) -> u32 {
        self.stack.last().map_or(NO_ROM, |e| e.0)
    }

    fn push(&mut self, routine: u32, sp: u8) {
        self.push_as(routine, sp, false);
    }

    fn push_as(&mut self, routine: u32, sp: u8, interrupt: bool) {
        if self.stack.len() == MAX_DEPTH {
            self.stack.remove(0);
        }
        self.stack.push((routine, sp, interrupt, false));
    }

    /// Pops the entries a return to stack pointer `sp` leaves behind.
    fn unwind(&mut self, sp: u8) {
        while self.stack.last().is_some_and(|e| e.1 < sp) {
            if let Some((r, _, _, true)) = self.stack.pop()
                && let Some(st) = self.calls.get_mut(&r)
            {
                st.apu_calls += 1;
            }
        }
    }

    /// Whether the code running now is under a sound engine routine.
    fn in_engine(&self) -> bool {
        for &(r, _, interrupt, _) in self.stack.iter().rev() {
            if self.calls.get(&r).is_some_and(CallStats::is_engine) {
                return true;
            }
            if interrupt {
                break;
            }
        }
        false
    }

    fn settle(&mut self, bus: &Bus) {
        if let Some(p) = self.pending.take() {
            let now = bus.peek(p.addr);
            if now != p.old {
                self.frame.ram.push(Write {
                    addr: p.addr,
                    value: now,
                    pc: p.pc,
                    routine: p.routine,
                    engine: p.engine,
                });
            }
        }
    }

    /// Call before each instruction, with whether an NMI or IRQ was taken
    /// after the last one (the collector reads `Bus::interrupt_taken` once
    /// for every observer).
    #[inline]
    pub fn observe(&mut self, bus: &Bus, interrupted: bool) {
        self.settle(bus);
        if interrupted {
            let pc = bus.cpu.pc;
            let at = self.layout.rom_offset(bus, pc).map_or(NO_ROM, |o| o as u32);
            self.enter(at);
            self.push_as(at, bus.cpu.sp, true);
        }
        let pc = bus.cpu.pc;
        let opcode = bus.peek(pc);
        let r = Cpu::INSTR_REF[usize::from(opcode)];
        let here = self.layout.rom_offset(bus, pc);
        if let Some(o) = here
            && self.marks.contains(o)
        {
            self.enter(o as u32);
        }
        let pc_off = here.map_or(NO_ROM, |o| o as u32);
        let sp = bus.cpu.sp;
        match r.instr {
            Instr::JSR => {
                let t = u16::from_le_bytes([
                    bus.peek(pc.wrapping_add(1)),
                    bus.peek(pc.wrapping_add(2)),
                ]);
                let at = self.layout.rom_offset(bus, t).map_or(NO_ROM, |o| o as u32);
                // The target's own mark counts it when it runs; count a
                // JSR target that isn't marked here.
                if at == NO_ROM || !self.marks.contains(at as usize) {
                    self.enter(at);
                }
                let a = bus.cpu.acc;
                let st = self.calls.entry(at).or_default();
                st.calls += 1;
                if st.callers.len() < 64 {
                    st.callers.insert(pc_off);
                }
                st.a_values[usize::from(a >> 6)] |= 1 << (a & 63);
                if self.watch.contains(&at) {
                    let engine = self.in_engine();
                    self.frame.calls.push((at, pc_off, a, engine));
                }
                self.push(at, sp.wrapping_sub(2));
            }
            Instr::RTS => self.unwind(sp.wrapping_add(2)),
            Instr::RTI => self.unwind(sp.wrapping_add(3)),
            _ => {}
        }
        if self.reads
            && reads_memory(r.instr, r.addr_mode)
            && let Some(addr) = target(bus, pc, r.instr, r.addr_mode)
            && addr < 0x2000
        {
            let pair = (addr & (RAM_BYTES as u16 - 1), self.top());
            if !self.frame.reads.contains(&pair) {
                self.frame.reads.push(pair);
            }
        }
        if writes_memory(r.instr, r.addr_mode)
            && let Some(addr) = target(bus, pc, r.instr, r.addr_mode)
        {
            let routine = self.top();
            if addr < 0x2000 {
                let a = addr & (RAM_BYTES as u16 - 1);
                self.pending = Some(Pending {
                    addr: a,
                    old: bus.peek(a),
                    pc: pc_off,
                    routine,
                    engine: self.in_engine(),
                });
            } else if addr & 0xE007 == 0x2007 && addr < 0x4000 {
                let value = match r.instr {
                    Instr::STA => bus.cpu.acc,
                    Instr::STX => bus.cpu.x,
                    Instr::STY => bus.cpu.y,
                    _ => 0,
                };
                let w = Write {
                    addr: bus.ppu.scroll.v & 0x3FFF,
                    value,
                    pc: pc_off,
                    routine,
                    engine: false,
                };
                match self.vram_last.get(&w.addr) {
                    Some(&i) => self.frame.vram[i] = w,
                    None => {
                        self.vram_last.insert(w.addr, self.frame.vram.len());
                        self.frame.vram.push(w);
                    }
                }
            } else if (0x4000..=0x4017).contains(&addr) && addr != 0x4014 && addr != 0x4016 {
                let value = match r.instr {
                    Instr::STA => bus.cpu.acc,
                    Instr::STX => bus.cpu.x,
                    Instr::STY => bus.cpu.y,
                    _ => 0,
                };
                self.frame.apu.push(Write {
                    addr,
                    value,
                    pc: pc_off,
                    routine,
                    engine: true,
                });
                // The routines that led here, back to the interrupt (if any)
                // that entered them: what an interrupt interrupted didn't.
                for e in self.stack.iter_mut().rev() {
                    e.3 = true;
                    if let Some(st) = self.calls.get_mut(&e.0) {
                        st.reaches_apu = true;
                    }
                    if e.2 {
                        break;
                    }
                }
            }
        }
    }

    /// Ends a frame and returns what it saw.
    pub fn end_frame(&mut self, bus: &Bus) -> FrameActivity {
        // An interrupt taken after the frame's last instruction belongs to
        // the next frame, whose first `before` sees it.
        self.settle(bus);
        let mut f = std::mem::take(&mut self.frame);
        if self.calls_only {
            for &o in &self.touched {
                self.counts[o as usize] = 0;
            }
            self.touched.clear();
            self.vram_last.clear();
            return FrameActivity {
                calls: f.calls,
                ..FrameActivity::default()
            };
        }
        self.touched.sort_unstable();
        for &o in &self.touched {
            f.entered.push((o, self.counts[o as usize]));
            self.counts[o as usize] = 0;
        }
        self.touched.clear();
        self.vram_last.clear();
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;

    fn layout() -> Layout {
        Layout::for_tests(64)
    }

    #[test]
    fn unwinding_drops_frames_a_return_leaves() {
        let mut a = Activity::new(layout(), &Bits::new(0));
        a.push(1, 0xF0);
        a.push(2, 0xEE);
        a.push(3, 0xEC);
        // A routine that pulled its caller's return address returns two
        // levels up at once.
        a.unwind(0xF0);
        assert_eq!(a.top(), 1);
        a.unwind(0xF2);
        assert_eq!(a.top(), NO_ROM);
    }

    #[test]
    fn entries_are_counted_per_frame() {
        let mut a = Activity::new(layout(), &Bits::new(0));
        a.enter(5);
        a.enter(5);
        a.enter(2);
        a.touched.sort_unstable();
        let mut f = std::mem::take(&mut a.frame);
        for &o in &a.touched {
            f.entered.push((o, a.counts[o as usize]));
        }
        assert_eq!(f.entered, vec![(2, 1), (5, 2)]);
    }
}
