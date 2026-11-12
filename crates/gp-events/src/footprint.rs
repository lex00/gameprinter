//! L0, footprints: which bytes of PRG ROM a run executes and which bytes of
//! RAM it writes, instruction by instruction.
//!
//! [`Footprint::before`] reads the console before each instruction through
//! [`gp_nes::Nes::step_with`], the hook instruction traces use, and changes
//! nothing but `Bus::interrupt_taken`, which only it and the tracer read. So
//! a recording's coverage needs no change to the runtime, and replay with
//! the observer reaches the same frame hashes as replay without it.
//!
//! Executed bytes are every byte of each instruction that runs from PRG ROM:
//! the opcode and its operands, by offset into PRG ROM, so two banks that
//! share a CPU address count apart. RAM writes are worked out from the
//! instruction about to run, with the registers and memory it will see: the
//! address a store or read-modify-write reaches, and the stack bytes `PHA`,
//! `PHP`, `JSR` and `BRK` push. An NMI or IRQ shows up as
//! `Bus::interrupt_taken` after the instruction it followed; its three
//! pushed bytes sit just above the stack pointer at that point.

use tetanes_core::bus::Bus;
use tetanes_core::cpu::Cpu;
use tetanes_core::cpu::instr::{AddrMode, Instr};
use tetanes_core::debug::FrameKind;

use crate::bits::Bits;
use crate::layout::{Layout, PAGE, RAM_BYTES};

/// What the CPU did over the frames it watched.
#[derive(Debug, Clone)]
pub struct Footprint {
    layout: Layout,
    /// Every byte of every instruction run from PRG ROM.
    pub executed: Bits,
    /// The first byte of each of them.
    pub starts: Bits,
    /// For each 1KB page of PRG ROM, the CPU pages (bit `n` for
    /// `$n*0x400`) it ran at.
    pub windows: Vec<u64>,
    pub ram_written: Bits,
    pub prg_ram_written: Bits,
    pub instructions: u64,
    /// Instructions run from anywhere but PRG ROM (RAM, usually).
    pub outside_rom: u64,
}

impl Footprint {
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            executed: Bits::new(layout.prg_rom),
            starts: Bits::new(layout.prg_rom),
            windows: vec![0; layout.prg_rom.div_ceil(PAGE)],
            ram_written: Bits::new(RAM_BYTES),
            prg_ram_written: Bits::new(layout.prg_ram),
            instructions: 0,
            outside_rom: 0,
        }
    }

    /// Call before each instruction, and once more after each frame for
    /// the interrupt that followed its last instruction. Start each run of
    /// frames with [`Footprint::start`].
    #[inline]
    pub fn before(&mut self, bus: &mut Bus) {
        self.interrupt(bus);
        self.instruction(bus);
    }

    /// Clears a stale `interrupt_taken` before the first frame: the record
    /// stays until the next interrupt.
    pub fn start(&mut self, bus: &mut Bus) {
        bus.interrupt_taken = None;
    }

    /// Records the pushes of an NMI or IRQ taken after the last instruction.
    #[inline]
    pub fn interrupt(&mut self, bus: &mut Bus) {
        if let Some(FrameKind::Nmi | FrameKind::Irq) = bus.interrupt_taken.take() {
            self.interrupted(bus);
        }
    }

    /// The three bytes an NMI or IRQ just pushed, for a caller that took
    /// `Bus::interrupt_taken` itself.
    #[inline]
    pub fn interrupted(&mut self, bus: &Bus) {
        let sp = bus.cpu.sp;
        for i in 1..=3u8 {
            self.ram_written
                .set(0x100 | usize::from(sp.wrapping_add(i)));
        }
    }

    /// Records the instruction about to run; returns its PRG offset, if it
    /// runs from PRG ROM, and its opcode.
    #[inline]
    pub fn instruction(&mut self, bus: &Bus) -> (Option<usize>, u8) {
        let pc = bus.cpu.pc;
        let opcode = bus.peek(pc);
        let r = Cpu::INSTR_REF[usize::from(opcode)];
        let len = 1 + r.addr_mode.operand_len();
        self.instructions += 1;
        let mut here = None;
        for i in 0..len {
            let addr = pc.wrapping_add(u16::from(i));
            match self.layout.rom_offset(bus, addr) {
                Some(offset) => {
                    self.executed.set(offset);
                    if i == 0 {
                        here = Some(offset);
                        self.starts.set(offset);
                        self.windows[offset / PAGE] |= 1 << (addr >> 10);
                    }
                }
                None if i == 0 => self.outside_rom += 1,
                None => {}
            }
        }
        let sp = bus.cpu.sp;
        let pushes = match r.instr {
            Instr::PHA | Instr::PHP => 1,
            Instr::JSR => 2,
            Instr::BRK => 3,
            _ => 0,
        };
        for i in 0..pushes {
            self.ram_written
                .set(0x100 | usize::from(sp.wrapping_sub(i)));
        }
        if writes_memory(r.instr, r.addr_mode)
            && let Some(addr) = target(bus, pc, r.instr, r.addr_mode)
        {
            if addr < 0x2000 {
                self.ram_written.set(usize::from(addr) & (RAM_BYTES - 1));
            } else if let Some(offset) = self.layout.ram_offset(bus, addr) {
                self.prg_ram_written.set(offset);
            }
        }
        (here, opcode)
    }

    /// L0 as the shim marked it since it was attached.
    pub fn from_shim(&mut self, s: &tetanes_core::shim::Shim) {
        self.executed.words_mut().copy_from_slice(&s.executed);
        self.starts.words_mut().copy_from_slice(&s.starts);
        self.windows.copy_from_slice(&s.windows);
        self.ram_written.words_mut().copy_from_slice(&s.ram_written);
        self.prg_ram_written
            .words_mut()
            .copy_from_slice(&s.prg_ram_written);
        self.instructions = s.instructions;
        self.outside_rom = s.outside_rom;
    }

    /// Adds what `other` watched. Both must have watched the same cartridge.
    pub fn merge(&mut self, other: &Footprint) -> Result<(), String> {
        if self.layout != other.layout {
            return Err("can't merge coverage of two different cartridges".into());
        }
        self.executed.union(&other.executed)?;
        self.starts.union(&other.starts)?;
        for (a, b) in self.windows.iter_mut().zip(&other.windows) {
            *a |= b;
        }
        self.ram_written.union(&other.ram_written)?;
        self.prg_ram_written.union(&other.prg_ram_written)?;
        self.instructions += other.instructions;
        self.outside_rom += other.outside_rom;
        Ok(())
    }
}

/// Whether the instruction writes the memory its operand names. Stores,
/// read-modify-writes, and the unofficial opcodes that do either.
pub fn writes_memory(instr: Instr, mode: AddrMode) -> bool {
    use Instr::*;
    match instr {
        STA | STX | STY | SAX | AHX | SXA | SYA | TAS | SHAA | SHAZ | DCP | ISB | SLO | RLA
        | SRE | RRA | INC | DEC => true,
        ASL | LSR | ROL | ROR => mode != AddrMode::ACC,
        _ => false,
    }
}

/// Whether the instruction reads the memory its operand names: loads,
/// compares, the arithmetic and logic that take a memory operand, bit tests
/// and read-modify-writes.
pub fn reads_memory(instr: Instr, mode: AddrMode) -> bool {
    use Instr::*;
    if matches!(
        mode,
        AddrMode::IMM | AddrMode::IMP | AddrMode::ACC | AddrMode::REL
    ) {
        return false;
    }
    matches!(
        instr,
        LDA | LDX
            | LDY
            | CMP
            | CPX
            | CPY
            | ADC
            | SBC
            | AND
            | ORA
            | EOR
            | BIT
            | LAX
            | INC
            | DEC
            | ASL
            | LSR
            | ROL
            | ROR
            | DCP
            | ISB
            | SLO
            | RLA
            | SRE
            | RRA
    )
}

/// The address the instruction at `pc` will reach, with the registers and
/// memory as they are before it runs.
pub fn target(bus: &Bus, pc: u16, instr: Instr, mode: AddrMode) -> Option<u16> {
    let cpu = &bus.cpu;
    let b1 = bus.peek(pc.wrapping_add(1));
    let abs = || u16::from_le_bytes([b1, bus.peek(pc.wrapping_add(2))]);
    // A pointer in zero page wraps within it.
    let pointer = |zp: u8| {
        u16::from_le_bytes([
            bus.peek(u16::from(zp)),
            bus.peek(u16::from(zp.wrapping_add(1))),
        ])
    };
    Some(match mode {
        AddrMode::ZP0 => u16::from(b1),
        AddrMode::ZPX => u16::from(b1.wrapping_add(cpu.x)),
        AddrMode::ZPY => u16::from(b1.wrapping_add(cpu.y)),
        AddrMode::ABS => abs(),
        AddrMode::ABX | AddrMode::ABXW => abs().wrapping_add(cpu.x.into()),
        AddrMode::ABY | AddrMode::ABYW => abs().wrapping_add(cpu.y.into()),
        AddrMode::IDX => pointer(b1.wrapping_add(cpu.x)),
        AddrMode::IDY | AddrMode::IDYW => pointer(b1).wrapping_add(cpu.y.into()),
        // The unofficial stores whose page cross is unstable land on the
        // indexed address when it doesn't cross, which is what this gives.
        AddrMode::OTH => match instr {
            Instr::SYA => abs().wrapping_add(cpu.x.into()),
            Instr::JSR => return None,
            _ => abs().wrapping_add(cpu.y.into()),
        },
        AddrMode::ACC | AddrMode::IMP | AddrMode::IMM | AddrMode::REL | AddrMode::IND => {
            return None;
        }
    })
}
