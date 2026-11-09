//! The memory an instruction will read, worked out from the instruction
//! about to run with the registers and memory it will see. A random draw
//! site's value is read this way: the byte an instruction like `AND $19,X`
//! reads from its indexed operand.

use tetanes_core::bus::Bus;
use tetanes_core::cpu::Cpu;
use tetanes_core::cpu::instr::{AddrMode, Instr};

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

/// The address the instruction at `pc` reads from memory, if it reads one.
pub fn operand_read(bus: &Bus, pc: u16) -> Option<u16> {
    let r = Cpu::INSTR_REF[usize::from(bus.peek(pc))];
    target(bus, pc, r.instr, r.addr_mode).filter(|_| reads_memory(r.instr, r.addr_mode))
}
