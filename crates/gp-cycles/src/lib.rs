//! The cycle cost of one of an original's routines (docs/cycles.md).
//!
//! A game that steps its random generator once per pass of an idle loop
//! (Donkey Kong: JSR $F4ED at $C7E1) steps it as many times as the frame's
//! spare CPU time allows, so a model that steps it as the game does (the
//! retro profile) needs the frame's work in cycles. That work is a function
//! of the RAM the frame's routines read (`gp cycles readset` shows it), and
//! its branches compare that RAM, so the cost is the cycles of the path the
//! routine's own code takes. [`cost`] runs a routine's code, from a slice of
//! the user's ROM made by `gp cycles slice` under $GP_HOME/work, on a
//! scratch copy of RAM the caller fills from model state, and returns the
//! cycles it took. Nothing it computes leaves it but the count: the model's
//! state is never written.
//!
//! The 6502 here is the NMOS one's official instructions with their cycle
//! counts, the extra cycle of an indexed read that crosses a page, and a
//! branch's extra cycles when taken and when it lands on another page
//! (MOS Technology, MCS6500 Microcomputer Family Programming Manual, 1976,
//! appendix A). Writes to hardware registers are dropped, and reads of them
//! ask the caller's `io` ([`cost_io`]): a controller's bits, for a routine
//! that branches on a button.

mod walk;
pub use walk::{Answer, Ask, Call, Path, WalkError, walk};

/// The code and data a routine reads from the cartridge, by CPU address:
/// `prg` mapped from `base` up, mirrored to the end of the address space,
/// as a 16 KB NROM image is at $8000 and $C000.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    pub base: u16,
    pub prg: Vec<u8>,
}

/// The registers a routine is called with; `s` is the stack pointer inside
/// it, below the caller's return address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Regs {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub p: u8,
    pub s: u8,
}

/// Why a cost couldn't be measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CostError {
    /// An opcode this measures no cost for (an unofficial one, or data run
    /// as code), at its address.
    Opcode(u16, u8),
    /// The routine ran past `limit` cycles without returning.
    Limit(u64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Imp,
    Acc,
    Imm,
    Zp,
    ZpX,
    ZpY,
    Abs,
    AbsX,
    AbsY,
    Ind,
    IndX,
    IndY,
    Rel,
}

struct Cpu<'a> {
    ram: [u8; 0x800],
    slice: &'a Slice,
    io: &'a mut dyn FnMut(u16, u64) -> u8,
    r: Regs,
    pc: u16,
    cycles: u64,
    /// The stack pointer as the routine was entered, the caller's return
    /// address just above it.
    s0: u8,
}

const C: u8 = 0x01;
const Z: u8 = 0x02;
const I: u8 = 0x04;
const D: u8 = 0x08;
const V: u8 = 0x40;
const N: u8 = 0x80;

impl Cpu<'_> {
    fn read(&mut self, a: u16) -> u8 {
        match a {
            0..0x2000 => self.ram[usize::from(a & 0x7FF)],
            0x2000..0x4020 => (self.io)(a, self.cycles),
            _ if a >= self.slice.base && !self.slice.prg.is_empty() => {
                self.slice.prg[usize::from(a - self.slice.base) % self.slice.prg.len()]
            }
            _ => 0,
        }
    }

    fn write(&mut self, a: u16, v: u8) {
        if a < 0x2000 {
            self.ram[usize::from(a & 0x7FF)] = v;
        }
    }

    fn word(&mut self, a: u16) -> u16 {
        u16::from(self.read(a)) | u16::from(self.read(a.wrapping_add(1))) << 8
    }

    /// A zero-page pointer, its high byte read from the same page.
    fn zword(&mut self, z: u8) -> u16 {
        u16::from(self.read(u16::from(z))) | u16::from(self.read(u16::from(z.wrapping_add(1)))) << 8
    }

    fn push(&mut self, v: u8) {
        self.write(0x100 | u16::from(self.r.s), v);
        self.r.s = self.r.s.wrapping_sub(1);
    }

    fn pull(&mut self) -> u8 {
        self.r.s = self.r.s.wrapping_add(1);
        self.read(0x100 | u16::from(self.r.s))
    }

    fn nz(&mut self, v: u8) {
        self.r.p = (self.r.p & !(N | Z)) | (v & N) | if v == 0 { Z } else { 0 };
    }

    fn flag(&mut self, f: u8, on: bool) {
        if on {
            self.r.p |= f;
        } else {
            self.r.p &= !f;
        }
    }

    /// The operand's address for `mode`, and whether forming it crossed a
    /// page (an indexed read's extra cycle).
    fn addr(&mut self, mode: Mode) -> (u16, bool) {
        let pc = self.pc;
        let b1 = self.read(pc);
        match mode {
            Mode::Zp => {
                self.pc = pc.wrapping_add(1);
                (u16::from(b1), false)
            }
            Mode::ZpX => {
                self.pc = pc.wrapping_add(1);
                (u16::from(b1.wrapping_add(self.r.x)), false)
            }
            Mode::ZpY => {
                self.pc = pc.wrapping_add(1);
                (u16::from(b1.wrapping_add(self.r.y)), false)
            }
            Mode::Abs => {
                self.pc = pc.wrapping_add(2);
                (self.word(pc), false)
            }
            Mode::AbsX | Mode::AbsY => {
                self.pc = pc.wrapping_add(2);
                let base = self.word(pc);
                let i = if mode == Mode::AbsX {
                    self.r.x
                } else {
                    self.r.y
                };
                let a = base.wrapping_add(u16::from(i));
                (a, a & 0xFF00 != base & 0xFF00)
            }
            Mode::Ind => {
                self.pc = pc.wrapping_add(2);
                let p = self.word(pc);
                // The NMOS 6502 reads the high byte from the same page.
                let hi = (p & 0xFF00) | (p.wrapping_add(1) & 0x00FF);
                (
                    u16::from(self.read(p)) | u16::from(self.read(hi)) << 8,
                    false,
                )
            }
            Mode::IndX => {
                self.pc = pc.wrapping_add(1);
                (self.zword(b1.wrapping_add(self.r.x)), false)
            }
            Mode::IndY => {
                self.pc = pc.wrapping_add(1);
                let base = self.zword(b1);
                let a = base.wrapping_add(u16::from(self.r.y));
                (a, a & 0xFF00 != base & 0xFF00)
            }
            Mode::Imm => {
                self.pc = pc.wrapping_add(1);
                (pc, false)
            }
            Mode::Imp | Mode::Acc | Mode::Rel => (0, false),
        }
    }

    fn adc(&mut self, v: u8) {
        let a = self.r.a;
        let s = u16::from(a) + u16::from(v) + u16::from(self.r.p & C);
        let r = s as u8;
        self.flag(C, s > 0xFF);
        self.flag(V, (a ^ r) & (v ^ r) & 0x80 != 0);
        self.r.a = r;
        self.nz(r);
    }

    fn cmp(&mut self, reg: u8, v: u8) {
        self.flag(C, reg >= v);
        self.nz(reg.wrapping_sub(v));
    }

    fn branch(&mut self, take: bool) {
        let off = self.read(self.pc) as i8;
        self.pc = self.pc.wrapping_add(1);
        if take {
            let to = self.pc.wrapping_add_signed(i16::from(off));
            self.cycles += if to & 0xFF00 != self.pc & 0xFF00 {
                2
            } else {
                1
            };
            self.pc = to;
        }
    }

    /// Runs one instruction; false when it was the routine's last RTS.
    fn step(&mut self, depth: &mut i32) -> Result<bool, CostError> {
        let at = self.pc;
        let op = self.read(at);
        let Some((name, mode, base, cross)) = decode(op) else {
            return Err(CostError::Opcode(at, op));
        };
        self.pc = at.wrapping_add(1);
        self.cycles += u64::from(base);
        let (a, crossed) = self.addr(mode);
        if cross && crossed {
            self.cycles += 1;
        }
        let val = |s: &mut Self| s.read(a);
        let rmw = |s: &mut Self, f: &dyn Fn(&mut Self, u8) -> u8| {
            if mode == Mode::Acc {
                let v = s.r.a;
                s.r.a = f(s, v);
            } else {
                let v = s.read(a);
                let r = f(s, v);
                s.write(a, r);
            }
        };
        match name {
            "LDA" => {
                self.r.a = val(self);
                self.nz(self.r.a);
            }
            "LDX" => {
                self.r.x = val(self);
                self.nz(self.r.x);
            }
            "LDY" => {
                self.r.y = val(self);
                self.nz(self.r.y);
            }
            "STA" => self.write(a, self.r.a),
            "STX" => self.write(a, self.r.x),
            "STY" => self.write(a, self.r.y),
            "TAX" => {
                self.r.x = self.r.a;
                self.nz(self.r.x);
            }
            "TAY" => {
                self.r.y = self.r.a;
                self.nz(self.r.y);
            }
            "TXA" => {
                self.r.a = self.r.x;
                self.nz(self.r.a);
            }
            "TYA" => {
                self.r.a = self.r.y;
                self.nz(self.r.a);
            }
            "TSX" => {
                self.r.x = self.r.s;
                self.nz(self.r.x);
            }
            "TXS" => self.r.s = self.r.x,
            "PHA" => self.push(self.r.a),
            "PHP" => self.push(self.r.p | 0x30),
            "PLA" => {
                self.r.a = self.pull();
                self.nz(self.r.a);
            }
            "PLP" => self.r.p = self.pull(),
            "AND" => {
                self.r.a &= val(self);
                self.nz(self.r.a);
            }
            "ORA" => {
                self.r.a |= val(self);
                self.nz(self.r.a);
            }
            "EOR" => {
                self.r.a ^= val(self);
                self.nz(self.r.a);
            }
            "ADC" => {
                let v = val(self);
                self.adc(v);
            }
            "SBC" => {
                let v = val(self);
                self.adc(!v);
            }
            "CMP" => {
                let v = val(self);
                self.cmp(self.r.a, v);
            }
            "CPX" => {
                let v = val(self);
                self.cmp(self.r.x, v);
            }
            "CPY" => {
                let v = val(self);
                self.cmp(self.r.y, v);
            }
            "BIT" => {
                let v = val(self);
                self.flag(Z, self.r.a & v == 0);
                self.r.p = (self.r.p & !(N | V)) | (v & (N | V));
            }
            "INC" => rmw(self, &|s, v| {
                let r = v.wrapping_add(1);
                s.nz(r);
                r
            }),
            "DEC" => rmw(self, &|s, v| {
                let r = v.wrapping_sub(1);
                s.nz(r);
                r
            }),
            "INX" => {
                self.r.x = self.r.x.wrapping_add(1);
                self.nz(self.r.x);
            }
            "INY" => {
                self.r.y = self.r.y.wrapping_add(1);
                self.nz(self.r.y);
            }
            "DEX" => {
                self.r.x = self.r.x.wrapping_sub(1);
                self.nz(self.r.x);
            }
            "DEY" => {
                self.r.y = self.r.y.wrapping_sub(1);
                self.nz(self.r.y);
            }
            "ASL" => rmw(self, &|s, v| {
                s.flag(C, v & 0x80 != 0);
                let r = v << 1;
                s.nz(r);
                r
            }),
            "LSR" => rmw(self, &|s, v| {
                s.flag(C, v & 1 != 0);
                let r = v >> 1;
                s.nz(r);
                r
            }),
            "ROL" => rmw(self, &|s, v| {
                let c = s.r.p & C;
                s.flag(C, v & 0x80 != 0);
                let r = (v << 1) | c;
                s.nz(r);
                r
            }),
            "ROR" => rmw(self, &|s, v| {
                let c = s.r.p & C;
                s.flag(C, v & 1 != 0);
                let r = (v >> 1) | (c << 7);
                s.nz(r);
                r
            }),
            "JMP" => self.pc = a,
            "JSR" => {
                let ret = self.pc.wrapping_sub(1);
                self.push((ret >> 8) as u8);
                self.push(ret as u8);
                self.pc = a;
                *depth += 1;
            }
            "RTS" => {
                // The routine's own return: its return address is on top of
                // the stack it was called with (a callee may pop its caller's
                // address, as $E107's PLA PLA does, so depth alone can't say).
                if self.r.s == self.s0 {
                    return Ok(false);
                }
                let lo = self.pull();
                let hi = self.pull();
                self.pc = (u16::from(lo) | u16::from(hi) << 8).wrapping_add(1);
                *depth -= 1;
            }
            "RTI" => {
                self.r.p = self.pull();
                let lo = self.pull();
                let hi = self.pull();
                self.pc = u16::from(lo) | u16::from(hi) << 8;
            }
            "BCC" => self.branch(self.r.p & C == 0),
            "BCS" => self.branch(self.r.p & C != 0),
            "BEQ" => self.branch(self.r.p & Z != 0),
            "BNE" => self.branch(self.r.p & Z == 0),
            "BMI" => self.branch(self.r.p & N != 0),
            "BPL" => self.branch(self.r.p & N == 0),
            "BVC" => self.branch(self.r.p & V == 0),
            "BVS" => self.branch(self.r.p & V != 0),
            "CLC" => self.flag(C, false),
            "SEC" => self.flag(C, true),
            "CLI" => self.flag(I, false),
            "SEI" => self.flag(I, true),
            "CLD" => self.flag(D, false),
            "SED" => self.flag(D, true),
            "CLV" => self.flag(V, false),
            "NOP" => {}
            _ => return Err(CostError::Opcode(at, op)),
        }
        Ok(true)
    }
}

/// The cycles a call of the routine at `entry` takes, its JSR and final RTS
/// included, run on `ram` (a copy: the caller's is untouched) with `regs`.
/// Stops with [`CostError::Limit`] past `limit` cycles.
pub fn cost(
    slice: &Slice,
    entry: u16,
    ram: &[u8; 0x800],
    regs: Regs,
    limit: u64,
) -> Result<u64, CostError> {
    cost_io(slice, entry, ram, regs, limit, &mut |_, _| 0)
}

/// [`cost`], with `io` giving what a read of a hardware register ($2000 to
/// $401F) returns, given the address and the cycle the instruction began
/// on (a PPU status read depends on the time: its vblank flag).
pub fn cost_io(
    slice: &Slice,
    entry: u16,
    ram: &[u8; 0x800],
    regs: Regs,
    limit: u64,
    io: &mut dyn FnMut(u16, u64) -> u8,
) -> Result<u64, CostError> {
    let mut cpu = Cpu {
        ram: *ram,
        slice,
        io,
        r: regs,
        pc: entry,
        // The caller's JSR; the routine's last RTS counts as it runs.
        cycles: 6,
        s0: regs.s,
    };
    let mut depth = 0;
    while cpu.step(&mut depth)? {
        if cpu.cycles > limit {
            return Err(CostError::Limit(cpu.cycles));
        }
    }
    Ok(cpu.cycles)
}

/// [`cost`], with the address of each instruction run, in order: the path
/// the routine took, to compare with a trace of the original.
pub fn path(
    slice: &Slice,
    entry: u16,
    ram: &[u8; 0x800],
    regs: Regs,
    limit: u64,
) -> Result<(u64, Vec<u16>), CostError> {
    let mut io = |_, _| 0;
    let mut cpu = Cpu {
        ram: *ram,
        slice,
        io: &mut io,
        r: regs,
        pc: entry,
        cycles: 6,
        s0: regs.s,
    };
    let mut depth = 0;
    let mut out = vec![];
    loop {
        out.push(cpu.pc);
        if !cpu.step(&mut depth)? {
            break;
        }
        if cpu.cycles > limit {
            return Err(CostError::Limit(cpu.cycles));
        }
    }
    Ok((cpu.cycles, out))
}

/// A free-running 6502 on a slice and a scratch RAM, for the loop around a
/// game's routines: a model steps it instruction by instruction to the
/// next interrupt and counts the passes of its idle loop. Its writes stay
/// in its own RAM; hardware writes go to `on_write` with the value and the
/// cycle after the store, which returns the cycles they stall the CPU (an
/// OAM DMA's 513 or 514). Only absolute stores reach it, as games write
/// their PPU and APU registers.
pub struct Machine<'a> {
    cpu: Cpu<'a>,
    on_write: &'a mut dyn FnMut(u16, u8, u64) -> u64,
}

impl<'a> Machine<'a> {
    pub fn new(
        slice: &'a Slice,
        ram: &[u8; 0x800],
        regs: Regs,
        pc: u16,
        cycle: u64,
        io: &'a mut dyn FnMut(u16, u64) -> u8,
        on_write: &'a mut dyn FnMut(u16, u8, u64) -> u64,
    ) -> Self {
        Machine {
            cpu: Cpu {
                ram: *ram,
                slice,
                io,
                r: regs,
                pc,
                cycles: cycle,
                s0: 0,
            },
            on_write,
        }
    }

    pub fn pc(&self) -> u16 {
        self.cpu.pc
    }

    /// The CPU cycle the next instruction starts on.
    pub fn cycle(&self) -> u64 {
        self.cpu.cycles
    }

    pub fn ram(&self) -> &[u8; 0x800] {
        &self.cpu.ram
    }

    pub fn regs(&self) -> Regs {
        self.cpu.r
    }

    /// Runs one instruction.
    pub fn step(&mut self) -> Result<(), CostError> {
        let at = self.cpu.pc;
        let op = self.cpu.read(at);
        let mut depth = 1;
        // A store to a hardware register: its stall, by the cycle it lands.
        let store = matches!(op, 0x8D | 0x8E | 0x8C | 0x9D | 0x99);
        let target = if store {
            self.cpu.word(at.wrapping_add(1))
        } else {
            0
        };
        let value = match op {
            0x8E => self.cpu.r.x,
            0x8C => self.cpu.r.y,
            _ => self.cpu.r.a,
        };
        self.cpu.s0 = self.cpu.r.s.wrapping_add(1);
        self.cpu.step(&mut depth)?;
        if store && (0x2000..0x4020).contains(&target) {
            let extra = (self.on_write)(target, value, self.cpu.cycles);
            self.cpu.cycles += extra;
        }
        Ok(())
    }

    /// Takes an NMI: pushes PC and P and jumps to `vector`, 7 cycles.
    pub fn nmi(&mut self, vector: u16) {
        let pc = self.cpu.pc;
        self.cpu.push((pc >> 8) as u8);
        self.cpu.push(pc as u8);
        self.cpu.push(self.cpu.r.p & !0x10 | 0x20);
        self.cpu.r.p |= I;
        self.cpu.pc = vector;
        self.cpu.cycles += 7;
    }
}

/// An official opcode's mnemonic, addressing mode, base cycles and whether
/// an indexed read across a page costs one more.
fn decode(op: u8) -> Option<(&'static str, Mode, u8, bool)> {
    use Mode::*;
    let r = match op {
        0x69 => ("ADC", Imm, 2, false),
        0x65 => ("ADC", Zp, 3, false),
        0x75 => ("ADC", ZpX, 4, false),
        0x6D => ("ADC", Abs, 4, false),
        0x7D => ("ADC", AbsX, 4, true),
        0x79 => ("ADC", AbsY, 4, true),
        0x61 => ("ADC", IndX, 6, false),
        0x71 => ("ADC", IndY, 5, true),
        0x29 => ("AND", Imm, 2, false),
        0x25 => ("AND", Zp, 3, false),
        0x35 => ("AND", ZpX, 4, false),
        0x2D => ("AND", Abs, 4, false),
        0x3D => ("AND", AbsX, 4, true),
        0x39 => ("AND", AbsY, 4, true),
        0x21 => ("AND", IndX, 6, false),
        0x31 => ("AND", IndY, 5, true),
        0x0A => ("ASL", Acc, 2, false),
        0x06 => ("ASL", Zp, 5, false),
        0x16 => ("ASL", ZpX, 6, false),
        0x0E => ("ASL", Abs, 6, false),
        0x1E => ("ASL", AbsX, 7, false),
        0x90 => ("BCC", Rel, 2, false),
        0xB0 => ("BCS", Rel, 2, false),
        0xF0 => ("BEQ", Rel, 2, false),
        0x30 => ("BMI", Rel, 2, false),
        0xD0 => ("BNE", Rel, 2, false),
        0x10 => ("BPL", Rel, 2, false),
        0x50 => ("BVC", Rel, 2, false),
        0x70 => ("BVS", Rel, 2, false),
        0x24 => ("BIT", Zp, 3, false),
        0x2C => ("BIT", Abs, 4, false),
        0x18 => ("CLC", Imp, 2, false),
        0xD8 => ("CLD", Imp, 2, false),
        0x58 => ("CLI", Imp, 2, false),
        0xB8 => ("CLV", Imp, 2, false),
        0xC9 => ("CMP", Imm, 2, false),
        0xC5 => ("CMP", Zp, 3, false),
        0xD5 => ("CMP", ZpX, 4, false),
        0xCD => ("CMP", Abs, 4, false),
        0xDD => ("CMP", AbsX, 4, true),
        0xD9 => ("CMP", AbsY, 4, true),
        0xC1 => ("CMP", IndX, 6, false),
        0xD1 => ("CMP", IndY, 5, true),
        0xE0 => ("CPX", Imm, 2, false),
        0xE4 => ("CPX", Zp, 3, false),
        0xEC => ("CPX", Abs, 4, false),
        0xC0 => ("CPY", Imm, 2, false),
        0xC4 => ("CPY", Zp, 3, false),
        0xCC => ("CPY", Abs, 4, false),
        0xC6 => ("DEC", Zp, 5, false),
        0xD6 => ("DEC", ZpX, 6, false),
        0xCE => ("DEC", Abs, 6, false),
        0xDE => ("DEC", AbsX, 7, false),
        0xCA => ("DEX", Imp, 2, false),
        0x88 => ("DEY", Imp, 2, false),
        0x49 => ("EOR", Imm, 2, false),
        0x45 => ("EOR", Zp, 3, false),
        0x55 => ("EOR", ZpX, 4, false),
        0x4D => ("EOR", Abs, 4, false),
        0x5D => ("EOR", AbsX, 4, true),
        0x59 => ("EOR", AbsY, 4, true),
        0x41 => ("EOR", IndX, 6, false),
        0x51 => ("EOR", IndY, 5, true),
        0xE6 => ("INC", Zp, 5, false),
        0xF6 => ("INC", ZpX, 6, false),
        0xEE => ("INC", Abs, 6, false),
        0xFE => ("INC", AbsX, 7, false),
        0xE8 => ("INX", Imp, 2, false),
        0xC8 => ("INY", Imp, 2, false),
        0x4C => ("JMP", Abs, 3, false),
        0x6C => ("JMP", Ind, 5, false),
        0x20 => ("JSR", Abs, 6, false),
        0xA9 => ("LDA", Imm, 2, false),
        0xA5 => ("LDA", Zp, 3, false),
        0xB5 => ("LDA", ZpX, 4, false),
        0xAD => ("LDA", Abs, 4, false),
        0xBD => ("LDA", AbsX, 4, true),
        0xB9 => ("LDA", AbsY, 4, true),
        0xA1 => ("LDA", IndX, 6, false),
        0xB1 => ("LDA", IndY, 5, true),
        0xA2 => ("LDX", Imm, 2, false),
        0xA6 => ("LDX", Zp, 3, false),
        0xB6 => ("LDX", ZpY, 4, false),
        0xAE => ("LDX", Abs, 4, false),
        0xBE => ("LDX", AbsY, 4, true),
        0xA0 => ("LDY", Imm, 2, false),
        0xA4 => ("LDY", Zp, 3, false),
        0xB4 => ("LDY", ZpX, 4, false),
        0xAC => ("LDY", Abs, 4, false),
        0xBC => ("LDY", AbsX, 4, true),
        0x4A => ("LSR", Acc, 2, false),
        0x46 => ("LSR", Zp, 5, false),
        0x56 => ("LSR", ZpX, 6, false),
        0x4E => ("LSR", Abs, 6, false),
        0x5E => ("LSR", AbsX, 7, false),
        0xEA => ("NOP", Imp, 2, false),
        0x09 => ("ORA", Imm, 2, false),
        0x05 => ("ORA", Zp, 3, false),
        0x15 => ("ORA", ZpX, 4, false),
        0x0D => ("ORA", Abs, 4, false),
        0x1D => ("ORA", AbsX, 4, true),
        0x19 => ("ORA", AbsY, 4, true),
        0x01 => ("ORA", IndX, 6, false),
        0x11 => ("ORA", IndY, 5, true),
        0x48 => ("PHA", Imp, 3, false),
        0x08 => ("PHP", Imp, 3, false),
        0x68 => ("PLA", Imp, 4, false),
        0x28 => ("PLP", Imp, 4, false),
        0x2A => ("ROL", Acc, 2, false),
        0x26 => ("ROL", Zp, 5, false),
        0x36 => ("ROL", ZpX, 6, false),
        0x2E => ("ROL", Abs, 6, false),
        0x3E => ("ROL", AbsX, 7, false),
        0x6A => ("ROR", Acc, 2, false),
        0x66 => ("ROR", Zp, 5, false),
        0x76 => ("ROR", ZpX, 6, false),
        0x6E => ("ROR", Abs, 6, false),
        0x7E => ("ROR", AbsX, 7, false),
        0x40 => ("RTI", Imp, 6, false),
        0x60 => ("RTS", Imp, 6, false),
        0xE9 => ("SBC", Imm, 2, false),
        0xE5 => ("SBC", Zp, 3, false),
        0xF5 => ("SBC", ZpX, 4, false),
        0xED => ("SBC", Abs, 4, false),
        0xFD => ("SBC", AbsX, 4, true),
        0xF9 => ("SBC", AbsY, 4, true),
        0xE1 => ("SBC", IndX, 6, false),
        0xF1 => ("SBC", IndY, 5, true),
        0x38 => ("SEC", Imp, 2, false),
        0xF8 => ("SED", Imp, 2, false),
        0x78 => ("SEI", Imp, 2, false),
        0x85 => ("STA", Zp, 3, false),
        0x95 => ("STA", ZpX, 4, false),
        0x8D => ("STA", Abs, 4, false),
        0x9D => ("STA", AbsX, 5, false),
        0x99 => ("STA", AbsY, 5, false),
        0x81 => ("STA", IndX, 6, false),
        0x91 => ("STA", IndY, 6, false),
        0x86 => ("STX", Zp, 3, false),
        0x96 => ("STX", ZpY, 4, false),
        0x8E => ("STX", Abs, 4, false),
        0x84 => ("STY", Zp, 3, false),
        0x94 => ("STY", ZpX, 4, false),
        0x8C => ("STY", Abs, 4, false),
        0xAA => ("TAX", Imp, 2, false),
        0xA8 => ("TAY", Imp, 2, false),
        0xBA => ("TSX", Imp, 2, false),
        0x8A => ("TXA", Imp, 2, false),
        0x9A => ("TXS", Imp, 2, false),
        0x98 => ("TYA", Imp, 2, false),
        _ => return None,
    };
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slice of hand-made code at $8000 (not any game's).
    fn slice(code: &[u8]) -> Slice {
        let mut prg = vec![0xEA; 0x4000];
        prg[..code.len()].copy_from_slice(code);
        Slice { base: 0x8000, prg }
    }

    #[test]
    fn a_loop_costs_its_branches_and_its_page_crossings() {
        // LDX #3; DEX; BNE -3; RTS
        let s = slice(&[0xA2, 0x03, 0xCA, 0xD0, 0xFD, 0x60]);
        let c = cost(&s, 0x8000, &[0; 0x800], Regs::default(), 10_000).unwrap();
        // JSR 6 + LDX 2 + 3 * DEX 2 + 2 taken BNE 3 + 1 untaken 2 + RTS 6.
        assert_eq!(c, 6 + 2 + 6 + 6 + 2 + 6);
        // LDA $00FF,X with X = 1 crosses a page: one more cycle.
        let s = slice(&[0xA2, 0x01, 0xBD, 0xFF, 0x00, 0x60]);
        let c = cost(&s, 0x8000, &[0; 0x800], Regs::default(), 10_000).unwrap();
        assert_eq!(c, 6 + 2 + 5 + 6);
    }

    #[test]
    fn the_cost_follows_the_ram_and_leaves_it_untouched() {
        // LDA $10; BEQ +2; INC $11; INC $11; RTS
        let s = slice(&[0xA5, 0x10, 0xF0, 0x02, 0xE6, 0x11, 0xE6, 0x11, 0x60]);
        let mut ram = [0u8; 0x800];
        let zero = cost(&s, 0x8000, &ram, Regs::default(), 10_000).unwrap();
        ram[0x10] = 1;
        let one = cost(&s, 0x8000, &ram, Regs::default(), 10_000).unwrap();
        assert_eq!(one - zero, 4);
        assert_eq!(ram[0x11], 0);
    }

    #[test]
    fn a_routine_that_never_returns_stops_at_the_limit() {
        let s = slice(&[0x4C, 0x00, 0x80]);
        assert!(matches!(
            cost(&s, 0x8000, &[0; 0x800], Regs::default(), 100),
            Err(CostError::Limit(_))
        ));
    }
}
