//! A Z80 disassembler: each instruction's length, its text, where control
//! goes after it, and what it reads and writes (registers, memory, ports),
//! for the tools that read an arcade board's code (`gp code`, `gp reach`,
//! `gp coverage`, `gp interp draws --find`, `gp element inputs`).
//!
//! Written from Zilog's Z80 CPU User Manual (UM0080) and the opcode
//! decomposition in Cristian Dinu's "Decoding Z80 opcodes" (an opcode's
//! bits as x = 7-6, y = 5-3, z = 2-0, p = y >> 1, q = y & 1, with the
//! tables r, rp, rp2, cc, alu and rot), not from MAME's z80dasm. The
//! undocumented instructions are decoded as the Z80 runs them (MAME's core
//! here, `crate::z80`, runs them too): `sll`, the IXH/IXL/IYH/IYL halves,
//! `ddcb` shifts that also copy to a register, `in (c)`, `out (c),0`, the
//! ED opcodes that act as NOPs (`noni`), and a DD or FD prefix followed by
//! another prefix, which does nothing on its own (`nop*`, one byte).
//!
//! The text is lower-case Zilog syntax with `$` hex: `ld a,($e000)`,
//! `jr nz,$0105`, `ld (ix+$05),a`, `rst $38`. Relative jumps show their
//! target.

use std::fmt::Write as _;

/// Where control goes after an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// The next instruction.
    Next,
    /// `jp nn` and `jr e`.
    Jump(u16),
    /// `jp cc,nn` and `jr cc,e`: the target or the next instruction.
    JumpIf(u16),
    /// `djnz e`: the target while B isn't zero.
    Djnz(u16),
    /// `call nn`.
    Call(u16),
    /// `call cc,nn`.
    CallIf(u16),
    /// `rst p`.
    Rst(u16),
    /// `ret`, `reti`, `retn`.
    Ret,
    /// `ret cc`.
    RetIf,
    /// `jp (hl)`, `jp (ix)`, `jp (iy)`: wherever the register points.
    Indirect(Ptr),
    /// `halt`: the same instruction until an interrupt.
    Halt,
}

impl Flow {
    /// The target a jump, call or restart names in its operand.
    pub fn target(self) -> Option<u16> {
        match self {
            Self::Jump(t)
            | Self::JumpIf(t)
            | Self::Djnz(t)
            | Self::Call(t)
            | Self::CallIf(t)
            | Self::Rst(t) => Some(t),
            _ => None,
        }
    }

    /// Whether the next instruction can run after this one.
    pub fn falls_through(self) -> bool {
        !matches!(
            self,
            Self::Jump(_) | Self::Ret | Self::Indirect(_) | Self::Halt
        )
    }

    /// Whether it goes one of two ways by a flag (or B for `djnz`).
    pub fn conditional(self) -> bool {
        matches!(
            self,
            Self::JumpIf(_) | Self::Djnz(_) | Self::CallIf(_) | Self::RetIf
        )
    }
}

/// The register `jp (..)` and an indexed operand use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ptr {
    Hl,
    Ix,
    Iy,
}

impl Ptr {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hl => "hl",
            Self::Ix => "ix",
            Self::Iy => "iy",
        }
    }
}

/// Registers as 8-bit units, a bit each ([`Regs`]).
pub mod reg {
    pub const A: u32 = 1 << 0;
    pub const F: u32 = 1 << 1;
    pub const B: u32 = 1 << 2;
    pub const C: u32 = 1 << 3;
    pub const D: u32 = 1 << 4;
    pub const E: u32 = 1 << 5;
    pub const H: u32 = 1 << 6;
    pub const L: u32 = 1 << 7;
    pub const IXH: u32 = 1 << 8;
    pub const IXL: u32 = 1 << 9;
    pub const IYH: u32 = 1 << 10;
    pub const IYL: u32 = 1 << 11;
    pub const SP: u32 = 1 << 12;
    pub const I: u32 = 1 << 13;
    pub const R: u32 = 1 << 14;
    /// The second register set, as one unit (`exx`, `ex af,af'`).
    pub const ALT: u32 = 1 << 15;
    pub const BC: u32 = B | C;
    pub const DE: u32 = D | E;
    pub const HL: u32 = H | L;
    pub const IX: u32 = IXH | IXL;
    pub const IY: u32 = IYH | IYL;
    pub const AF: u32 = A | F;
}

/// A set of [`reg`] bits.
pub type Regs = u32;

/// How an operand in memory is addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addr {
    /// `(nn)`.
    Abs(u16),
    /// `(bc)`, `(de)`, `(hl)`: the pair's bits in [`reg`].
    Pair(Regs),
    /// `(ix+d)`, `(iy+d)`.
    Index(Ptr, i8),
    /// The stack: `push`, `pop`, `call`, `ret`, `ex (sp),..`.
    Stack,
}

/// An instruction's memory operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mem {
    pub addr: Addr,
    pub read: bool,
    pub write: bool,
    /// Two bytes (`ld hl,(nn)`, `push`) rather than one.
    pub word: bool,
}

/// What kind of operation, for the tools that ask how a value was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// A copy: `ld`, `push`, `pop`, `ex`, `in`, `out`.
    Load,
    /// `add`, `adc`, `sub`, `sbc`, `and`, `xor`, `or`, `cp` (8 bits), `neg`,
    /// `daa`, `cpl`, and the 16-bit `add`, `adc`, `sbc`.
    Alu(Alu),
    /// `inc`, `dec`.
    Count,
    /// A rotate or shift: `rlca` and friends, the CB shifts, `rld`, `rrd`.
    Shift,
    /// `bit`, `res`, `set`.
    Bit,
    /// `ldi`, `cpir` and the other block instructions.
    Block,
    /// Jumps, calls, returns, and everything else (`nop`, `di`, `im`...).
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alu {
    Add,
    Adc,
    Sub,
    Sbc,
    And,
    Xor,
    Or,
    Cp,
    /// `neg`, `daa`, `cpl`, `scf`, `ccf`.
    Other,
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insn {
    pub len: u8,
    pub text: String,
    pub flow: Flow,
    pub op: Op,
    /// The registers it reads and writes, flags as `F`.
    pub reads: Regs,
    pub writes: Regs,
    pub mem: Option<Mem>,
    /// A port it reads or writes: `Some(n)` for `in a,(n)`/`out (n),a`
    /// (the port's low byte; A is the high), `None` within for `(c)`.
    pub io: Option<Io>,
    /// The result doesn't depend on what it reads: `xor a`, `sub a`,
    /// `ld r,n`, `ld rr,nn`.
    pub constant: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Io {
    pub port: Option<u8>,
    pub write: bool,
}

const R8: [&str; 8] = ["b", "c", "d", "e", "h", "l", "(hl)", "a"];
const R8_BITS: [Regs; 8] = [reg::B, reg::C, reg::D, reg::E, reg::H, reg::L, 0, reg::A];
const CC: [&str; 8] = ["nz", "z", "nc", "c", "po", "pe", "p", "m"];
const ALU: [(&str, Alu); 8] = [
    ("add a,", Alu::Add),
    ("adc a,", Alu::Adc),
    ("sub ", Alu::Sub),
    ("sbc a,", Alu::Sbc),
    ("and ", Alu::And),
    ("xor ", Alu::Xor),
    ("or ", Alu::Or),
    ("cp ", Alu::Cp),
];
const ROT: [&str; 8] = ["rlc", "rrc", "rl", "rr", "sla", "sra", "sll", "srl"];

fn hex8(v: u8) -> String {
    format!("${v:02x}")
}

fn hex16(v: u16) -> String {
    format!("${v:04x}")
}

/// The decoder's state for one instruction.
struct Dec<'a> {
    b: &'a [u8],
    pc: u16,
    /// The DD or FD prefix in force.
    idx: Option<Ptr>,
    /// Bytes before the opcode: the prefixes.
    at: usize,
    reads: Regs,
    writes: Regs,
    mem: Option<Mem>,
}

impl Dec<'_> {
    fn byte(&self, i: usize) -> u8 {
        self.b.get(i).copied().unwrap_or(0)
    }

    fn word(&self, i: usize) -> u16 {
        u16::from_le_bytes([self.byte(i), self.byte(i + 1)])
    }

    /// HL, or IX/IY under a prefix.
    fn hl(&self) -> (&'static str, Regs) {
        match self.idx {
            None => ("hl", reg::HL),
            Some(Ptr::Ix) => ("ix", reg::IX),
            Some(Ptr::Iy) => ("iy", reg::IY),
            Some(Ptr::Hl) => unreachable!("never a prefix"),
        }
    }

    /// The index register's displacement (the byte after the opcode, or
    /// for DDCB the one before it).
    fn disp(&self) -> i8 {
        self.byte(self.at + 1) as i8
    }

    /// Register `i` of the r table, as text and bits; `(hl)` (or
    /// `(ix+d)`) also records the memory operand. `keep_hl` is set when
    /// the same instruction uses `(ix+d)`, so H and L stay themselves.
    fn r8(&mut self, i: usize, keep_hl: bool, read: bool, write: bool) -> (String, Regs) {
        if i == 6 {
            let (text, addr, bits) = match self.idx {
                None => ("(hl)".to_owned(), Addr::Pair(reg::HL), reg::HL),
                Some(p) => {
                    let d = self.disp();
                    let sign = if d < 0 { '-' } else { '+' };
                    (
                        format!("({}{sign}{})", p.name(), hex8(d.unsigned_abs())),
                        Addr::Index(p, d),
                        if p == Ptr::Ix { reg::IX } else { reg::IY },
                    )
                }
            };
            self.reads |= bits;
            self.mem = Some(Mem {
                addr,
                read,
                write,
                word: false,
            });
            return (text, 0);
        }
        if !keep_hl && (i == 4 || i == 5) {
            match self.idx {
                Some(Ptr::Ix) => {
                    return if i == 4 {
                        ("ixh".into(), reg::IXH)
                    } else {
                        ("ixl".into(), reg::IXL)
                    };
                }
                Some(Ptr::Iy) => {
                    return if i == 4 {
                        ("iyh".into(), reg::IYH)
                    } else {
                        ("iyl".into(), reg::IYL)
                    };
                }
                _ => {}
            }
        }
        (R8[i].to_owned(), R8_BITS[i])
    }

    /// Register pair `p` of the rp table (BC, DE, HL, SP).
    fn rp(&self, p: usize) -> (&'static str, Regs) {
        match p {
            0 => ("bc", reg::BC),
            1 => ("de", reg::DE),
            2 => self.hl(),
            _ => ("sp", reg::SP),
        }
    }

    /// Register pair `p` of the rp2 table (BC, DE, HL, AF).
    fn rp2(&self, p: usize) -> (&'static str, Regs) {
        if p == 3 { ("af", reg::AF) } else { self.rp(p) }
    }

    fn stack(&mut self, read: bool, write: bool) {
        self.reads |= reg::SP;
        self.writes |= reg::SP;
        self.mem = Some(Mem {
            addr: Addr::Stack,
            read,
            write,
            word: true,
        });
    }

    fn mem_word(&mut self, addr: Addr, read: bool, write: bool) {
        self.mem = Some(Mem {
            addr,
            read,
            write,
            word: true,
        });
    }

    fn mem_byte(&mut self, addr: Addr, read: bool, write: bool) {
        self.mem = Some(Mem {
            addr,
            read,
            write,
            word: false,
        });
    }
}

/// What a decode builds before the length is known.
struct Out {
    text: String,
    flow: Flow,
    op: Op,
    io: Option<Io>,
    constant: bool,
}

impl Out {
    fn new(text: impl Into<String>, op: Op) -> Self {
        Self {
            text: text.into(),
            flow: Flow::Next,
            op,
            io: None,
            constant: false,
        }
    }
}

/// Decodes the instruction at `pc`, whose bytes start `bytes` (four are
/// enough for any instruction; missing ones read as zero).
pub fn decode(bytes: &[u8], pc: u16) -> Insn {
    let mut d = Dec {
        b: bytes,
        pc,
        idx: None,
        at: 0,
        reads: 0,
        writes: 0,
        mem: None,
    };
    let first = d.byte(0);
    let (out, len) = match first {
        0xdd | 0xfd => {
            d.idx = Some(if first == 0xdd { Ptr::Ix } else { Ptr::Iy });
            d.at = 1;
            match d.byte(1) {
                // A prefix followed by another prefix does nothing itself.
                0xdd | 0xfd | 0xed => (Out::new("nop*", Op::Other), 1),
                0xcb => {
                    let (o, len) = cb(&mut d);
                    (o, len)
                }
                _ => main(&mut d),
            }
        }
        0xcb => cb(&mut d),
        0xed => ed(&mut d),
        _ => main(&mut d),
    };
    Insn {
        len,
        text: out.text,
        flow: out.flow,
        op: out.op,
        reads: d.reads,
        writes: d.writes,
        mem: d.mem,
        io: out.io,
        constant: out.constant,
    }
}

/// An unprefixed opcode, or one after DD or FD.
fn main(d: &mut Dec<'_>) -> (Out, u8) {
    let at = d.at;
    let op = d.byte(at);
    let (x, y, z) = (op >> 6, usize::from((op >> 3) & 7), usize::from(op & 7));
    let (p, q) = (y >> 1, y & 1);
    // Bytes so far: prefixes and the opcode; a (ix+d) adds one.
    let base = at as u8 + 1;
    let indexed_mem = |d: &Dec<'_>, uses_m: bool| u8::from(uses_m && d.idx.is_some());
    let rel = |d: &Dec<'_>, len: u8| {
        let e = d.byte(at + 1) as i8;
        d.pc.wrapping_add(u16::from(len)).wrapping_add(e as u16)
    };
    let mut o;
    let len;
    match (x, z) {
        (0, 0) => match y {
            0 => {
                o = Out::new("nop", Op::Other);
                len = base;
            }
            1 => {
                o = Out::new("ex af,af'", Op::Load);
                d.reads |= reg::AF | reg::ALT;
                d.writes |= reg::AF | reg::ALT;
                len = base;
            }
            2 => {
                len = base + 1;
                let t = rel(d, len);
                o = Out::new(format!("djnz {}", hex16(t)), Op::Other);
                o.flow = Flow::Djnz(t);
                d.reads |= reg::B;
                d.writes |= reg::B;
            }
            3 => {
                len = base + 1;
                let t = rel(d, len);
                o = Out::new(format!("jr {}", hex16(t)), Op::Other);
                o.flow = Flow::Jump(t);
            }
            _ => {
                len = base + 1;
                let t = rel(d, len);
                o = Out::new(format!("jr {},{}", CC[y - 4], hex16(t)), Op::Other);
                o.flow = Flow::JumpIf(t);
                d.reads |= reg::F;
            }
        },
        (0, 1) => {
            let (name, bits) = d.rp(p);
            if q == 0 {
                let nn = d.word(at + 1);
                o = Out::new(format!("ld {name},{}", hex16(nn)), Op::Load);
                o.constant = true;
                d.writes |= bits;
                len = base + 2;
            } else {
                let (hl, hb) = d.hl();
                o = Out::new(format!("add {hl},{name}"), Op::Alu(Alu::Add));
                d.reads |= hb | bits;
                d.writes |= hb | reg::F;
                len = base;
            }
        }
        (0, 2) => {
            let (hl, hb) = d.hl();
            match (q, p) {
                (0, 0) | (0, 1) => {
                    let (pr, pb) = if p == 0 {
                        ("bc", reg::BC)
                    } else {
                        ("de", reg::DE)
                    };
                    o = Out::new(format!("ld ({pr}),a"), Op::Load);
                    d.reads |= pb | reg::A;
                    d.mem_byte(Addr::Pair(pb), false, true);
                    len = base;
                }
                (0, 2) => {
                    let nn = d.word(at + 1);
                    o = Out::new(format!("ld ({}),{hl}", hex16(nn)), Op::Load);
                    d.reads |= hb;
                    d.mem_word(Addr::Abs(nn), false, true);
                    len = base + 2;
                }
                (0, _) => {
                    let nn = d.word(at + 1);
                    o = Out::new(format!("ld ({}),a", hex16(nn)), Op::Load);
                    d.reads |= reg::A;
                    d.mem_byte(Addr::Abs(nn), false, true);
                    len = base + 2;
                }
                (_, 0) | (_, 1) => {
                    let (pr, pb) = if p == 0 {
                        ("bc", reg::BC)
                    } else {
                        ("de", reg::DE)
                    };
                    o = Out::new(format!("ld a,({pr})"), Op::Load);
                    d.reads |= pb;
                    d.writes |= reg::A;
                    d.mem_byte(Addr::Pair(pb), true, false);
                    len = base;
                }
                (_, 2) => {
                    let nn = d.word(at + 1);
                    o = Out::new(format!("ld {hl},({})", hex16(nn)), Op::Load);
                    d.writes |= hb;
                    d.mem_word(Addr::Abs(nn), true, false);
                    len = base + 2;
                }
                (_, _) => {
                    let nn = d.word(at + 1);
                    o = Out::new(format!("ld a,({})", hex16(nn)), Op::Load);
                    d.writes |= reg::A;
                    d.mem_byte(Addr::Abs(nn), true, false);
                    len = base + 2;
                }
            }
        }
        (0, 3) => {
            let (name, bits) = d.rp(p);
            o = Out::new(
                format!("{} {name}", if q == 0 { "inc" } else { "dec" }),
                Op::Count,
            );
            d.reads |= bits;
            d.writes |= bits;
            len = base;
        }
        (0, 4) | (0, 5) => {
            let (name, bits) = d.r8(y, false, true, true);
            o = Out::new(
                format!("{} {name}", if z == 4 { "inc" } else { "dec" }),
                Op::Count,
            );
            d.reads |= bits;
            d.writes |= bits | reg::F;
            len = base + indexed_mem(d, y == 6);
        }
        (0, 6) => {
            let (name, bits) = d.r8(y, false, false, true);
            let n_at = at + 1 + usize::from(indexed_mem(d, y == 6));
            let n = d.byte(n_at);
            o = Out::new(format!("ld {name},{}", hex8(n)), Op::Load);
            o.constant = true;
            d.writes |= bits;
            len = base + 1 + indexed_mem(d, y == 6);
        }
        (0, _) => {
            const NAMES: [&str; 8] = ["rlca", "rrca", "rla", "rra", "daa", "cpl", "scf", "ccf"];
            let op = if y < 4 {
                Op::Shift
            } else {
                Op::Alu(Alu::Other)
            };
            o = Out::new(NAMES[y], op);
            match y {
                0..=4 => {
                    d.reads |= reg::A
                        | if y == 2 || y == 3 || y == 4 {
                            reg::F
                        } else {
                            0
                        };
                    d.writes |= reg::A | reg::F;
                }
                5 => {
                    d.reads |= reg::A;
                    d.writes |= reg::A | reg::F;
                }
                _ => {
                    d.reads |= reg::F;
                    d.writes |= reg::F;
                }
            }
            len = base;
        }
        (1, _) if y == 6 && z == 6 => {
            o = Out::new("halt", Op::Other);
            o.flow = Flow::Halt;
            len = base;
        }
        (1, _) => {
            let uses_m = y == 6 || z == 6;
            let (dst, db) = d.r8(y, uses_m, false, true);
            let (src, sb) = d.r8(z, uses_m, true, false);
            o = Out::new(format!("ld {dst},{src}"), Op::Load);
            d.reads |= sb;
            d.writes |= db;
            len = base + indexed_mem(d, uses_m);
        }
        (2, _) => {
            let (name, bits) = d.r8(z, false, true, false);
            let (text, alu) = ALU[y];
            o = Out::new(format!("{text}{name}"), Op::Alu(alu));
            d.reads |= bits
                | reg::A
                | if matches!(alu, Alu::Adc | Alu::Sbc) {
                    reg::F
                } else {
                    0
                };
            d.writes |= reg::F | if alu == Alu::Cp { 0 } else { reg::A };
            o.constant = z == 7 && matches!(alu, Alu::Sub | Alu::Xor);
            len = base + indexed_mem(d, z == 6);
        }
        (_, 0) => {
            o = Out::new(format!("ret {}", CC[y]), Op::Other);
            o.flow = Flow::RetIf;
            d.reads |= reg::F;
            d.stack(true, false);
            len = base;
        }
        (_, 1) => {
            if q == 0 {
                let (name, bits) = d.rp2(p);
                o = Out::new(format!("pop {name}"), Op::Load);
                d.writes |= bits;
                d.stack(true, false);
            } else {
                match p {
                    0 => {
                        o = Out::new("ret", Op::Other);
                        o.flow = Flow::Ret;
                        d.stack(true, false);
                    }
                    1 => {
                        o = Out::new("exx", Op::Load);
                        d.reads |= reg::BC | reg::DE | reg::HL | reg::ALT;
                        d.writes |= reg::BC | reg::DE | reg::HL | reg::ALT;
                    }
                    2 => {
                        let (hl, hb) = d.hl();
                        o = Out::new(format!("jp ({hl})"), Op::Other);
                        o.flow = Flow::Indirect(d.idx.unwrap_or(Ptr::Hl));
                        d.reads |= hb;
                    }
                    _ => {
                        let (hl, hb) = d.hl();
                        o = Out::new(format!("ld sp,{hl}"), Op::Load);
                        d.reads |= hb;
                        d.writes |= reg::SP;
                    }
                }
            }
            len = base;
        }
        (_, 2) => {
            let nn = d.word(at + 1);
            o = Out::new(format!("jp {},{}", CC[y], hex16(nn)), Op::Other);
            o.flow = Flow::JumpIf(nn);
            d.reads |= reg::F;
            len = base + 2;
        }
        (_, 3) => match y {
            0 => {
                let nn = d.word(at + 1);
                o = Out::new(format!("jp {}", hex16(nn)), Op::Other);
                o.flow = Flow::Jump(nn);
                len = base + 2;
            }
            1 => unreachable!("CB is decoded before main"),
            2 => {
                let n = d.byte(at + 1);
                o = Out::new(format!("out ({}),a", hex8(n)), Op::Load);
                o.io = Some(Io {
                    port: Some(n),
                    write: true,
                });
                d.reads |= reg::A;
                len = base + 1;
            }
            3 => {
                let n = d.byte(at + 1);
                o = Out::new(format!("in a,({})", hex8(n)), Op::Load);
                o.io = Some(Io {
                    port: Some(n),
                    write: false,
                });
                d.reads |= reg::A;
                d.writes |= reg::A;
                len = base + 1;
            }
            4 => {
                let (hl, hb) = d.hl();
                o = Out::new(format!("ex (sp),{hl}"), Op::Load);
                d.reads |= hb;
                d.writes |= hb;
                d.stack(true, true);
                len = base;
            }
            5 => {
                o = Out::new("ex de,hl", Op::Load);
                d.reads |= reg::DE | reg::HL;
                d.writes |= reg::DE | reg::HL;
                len = base;
            }
            6 => {
                o = Out::new("di", Op::Other);
                len = base;
            }
            _ => {
                o = Out::new("ei", Op::Other);
                len = base;
            }
        },
        (_, 4) => {
            let nn = d.word(at + 1);
            o = Out::new(format!("call {},{}", CC[y], hex16(nn)), Op::Other);
            o.flow = Flow::CallIf(nn);
            d.reads |= reg::F;
            d.stack(false, true);
            len = base + 2;
        }
        (_, 5) => {
            if q == 0 {
                let (name, bits) = d.rp2(p);
                o = Out::new(format!("push {name}"), Op::Load);
                d.reads |= bits;
                d.stack(false, true);
                len = base;
            } else {
                // p is 0 here: DD, ED and FD are prefixes, decoded before.
                let nn = d.word(at + 1);
                o = Out::new(format!("call {}", hex16(nn)), Op::Other);
                o.flow = Flow::Call(nn);
                d.stack(false, true);
                len = base + 2;
            }
        }
        (_, 6) => {
            let n = d.byte(at + 1);
            let (text, alu) = ALU[y];
            o = Out::new(format!("{text}{}", hex8(n)), Op::Alu(alu));
            d.reads |= reg::A
                | if matches!(alu, Alu::Adc | Alu::Sbc) {
                    reg::F
                } else {
                    0
                };
            d.writes |= reg::F | if alu == Alu::Cp { 0 } else { reg::A };
            len = base + 1;
        }
        _ => {
            let t = (y as u16) * 8;
            o = Out::new(format!("rst {}", hex8(t as u8)), Op::Other);
            o.flow = Flow::Rst(t);
            d.stack(false, true);
            len = base;
        }
    }
    (o, len)
}

/// A CB opcode, or DD CB d op / FD CB d op.
fn cb(d: &mut Dec<'_>) -> (Out, u8) {
    let indexed = d.idx.is_some();
    // DD CB d op: the displacement comes before the opcode.
    let op_at = if indexed { 3 } else { 1 };
    if indexed {
        // `disp()` reads the byte after `at`, which is the CB byte's
        // position for DDCB: the displacement.
        d.at = 1;
    }
    let op = d.byte(op_at);
    let (x, y, z) = (op >> 6, usize::from((op >> 3) & 7), usize::from(op & 7));
    let len = if indexed { 4 } else { 2 };
    // On (ix+d), a register other than (hl) in z also gets the result
    // (undocumented); `bit` never writes.
    let target = if indexed { 6 } else { z };
    let write = x != 1;
    let (name, bits) = d.r8(target, true, true, write);
    let copy = if indexed && z != 6 && x != 1 {
        let (r, rb) = (R8[z], R8_BITS[z]);
        d.writes |= rb;
        format!(",{r}")
    } else {
        String::new()
    };
    let o = match x {
        0 => {
            d.reads |= bits | if y == 2 || y == 3 { reg::F } else { 0 };
            d.writes |= bits | reg::F;
            Out::new(format!("{} {name}{copy}", ROT[y]), Op::Shift)
        }
        1 => {
            d.reads |= bits;
            d.writes |= reg::F;
            Out::new(format!("bit {y},{name}"), Op::Bit)
        }
        _ => {
            d.reads |= bits;
            d.writes |= bits;
            Out::new(
                format!("{} {y},{name}{copy}", if x == 2 { "res" } else { "set" }),
                Op::Bit,
            )
        }
    };
    (o, len)
}

/// An ED opcode.
fn ed(d: &mut Dec<'_>) -> (Out, u8) {
    let op = d.byte(1);
    let (x, y, z) = (op >> 6, usize::from((op >> 3) & 7), usize::from(op & 7));
    let (p, q) = (y >> 1, y & 1);
    let mut len = 2;
    let o = match (x, z) {
        (1, 0) => {
            d.reads |= reg::BC;
            d.writes |= reg::F;
            let mut o = if y == 6 {
                Out::new("in (c)", Op::Load)
            } else {
                d.writes |= R8_BITS[y];
                Out::new(format!("in {},(c)", R8[y]), Op::Load)
            };
            o.io = Some(Io {
                port: None,
                write: false,
            });
            o
        }
        (1, 1) => {
            d.reads |= reg::BC | if y == 6 { 0 } else { R8_BITS[y] };
            let mut o = if y == 6 {
                Out::new("out (c),0", Op::Load)
            } else {
                Out::new(format!("out (c),{}", R8[y]), Op::Load)
            };
            o.io = Some(Io {
                port: None,
                write: true,
            });
            o
        }
        (1, 2) => {
            let (name, bits) = d.rp(p);
            d.reads |= reg::HL | bits | reg::F;
            d.writes |= reg::HL | reg::F;
            if q == 0 {
                Out::new(format!("sbc hl,{name}"), Op::Alu(Alu::Sbc))
            } else {
                Out::new(format!("adc hl,{name}"), Op::Alu(Alu::Adc))
            }
        }
        (1, 3) => {
            let nn = d.word(2);
            let (name, bits) = d.rp(p);
            len = 4;
            if q == 0 {
                d.reads |= bits;
                d.mem_word(Addr::Abs(nn), false, true);
                Out::new(format!("ld ({}),{name}", hex16(nn)), Op::Load)
            } else {
                d.writes |= bits;
                d.mem_word(Addr::Abs(nn), true, false);
                Out::new(format!("ld {name},({})", hex16(nn)), Op::Load)
            }
        }
        (1, 4) => {
            d.reads |= reg::A;
            d.writes |= reg::A | reg::F;
            Out::new("neg", Op::Alu(Alu::Other))
        }
        (1, 5) => {
            let mut o = Out::new(if y == 1 { "reti" } else { "retn" }, Op::Other);
            o.flow = Flow::Ret;
            d.stack(true, false);
            o
        }
        (1, 6) => {
            const IM: [&str; 8] = ["0", "0/1", "1", "2", "0", "0/1", "1", "2"];
            Out::new(format!("im {}", IM[y]), Op::Other)
        }
        (1, _) => match y {
            0 => {
                d.reads |= reg::A;
                d.writes |= reg::I;
                Out::new("ld i,a", Op::Load)
            }
            1 => {
                d.reads |= reg::A;
                d.writes |= reg::R;
                Out::new("ld r,a", Op::Load)
            }
            2 => {
                d.reads |= reg::I;
                d.writes |= reg::A | reg::F;
                Out::new("ld a,i", Op::Load)
            }
            3 => {
                d.reads |= reg::R;
                d.writes |= reg::A | reg::F;
                Out::new("ld a,r", Op::Load)
            }
            4 | 5 => {
                d.reads |= reg::A | reg::HL;
                d.writes |= reg::A | reg::F;
                d.mem_byte(Addr::Pair(reg::HL), true, true);
                Out::new(if y == 4 { "rrd" } else { "rld" }, Op::Shift)
            }
            _ => Out::new("nop*", Op::Other),
        },
        (2, _) if z <= 3 && y >= 4 => {
            const BLOCK: [[&str; 4]; 4] = [
                ["ldi", "cpi", "ini", "outi"],
                ["ldd", "cpd", "ind", "outd"],
                ["ldir", "cpir", "inir", "otir"],
                ["lddr", "cpdr", "indr", "otdr"],
            ];
            let name = BLOCK[y - 4][z];
            d.reads |= reg::BC | reg::HL;
            d.writes |= reg::BC | reg::HL | reg::F;
            match z {
                0 => {
                    d.reads |= reg::DE;
                    d.writes |= reg::DE;
                    d.mem_byte(Addr::Pair(reg::HL), true, true);
                }
                1 => {
                    d.reads |= reg::A;
                    d.mem_byte(Addr::Pair(reg::HL), true, false);
                }
                2 => d.mem_byte(Addr::Pair(reg::HL), false, true),
                _ => d.mem_byte(Addr::Pair(reg::HL), true, false),
            }
            let mut o = Out::new(name, Op::Block);
            if z >= 2 {
                o.io = Some(Io {
                    port: None,
                    write: z == 3,
                });
            }
            o
        }
        _ => Out::new("noni", Op::Other),
    };
    (o, len)
}

/// Every instruction from `start` in `code` (addressed from `origin`) as
/// listing lines, `address  bytes  text`, while it stays inside `code`.
pub fn listing(code: &[u8], origin: u16, start: u16, count: usize) -> String {
    let mut out = String::new();
    let mut pc = start;
    for _ in 0..count {
        let off = usize::from(pc.wrapping_sub(origin));
        let Some(rest) = code.get(off..) else { break };
        if rest.is_empty() {
            break;
        }
        let i = decode(rest, pc);
        let bytes: Vec<String> = rest[..usize::from(i.len).min(rest.len())]
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect();
        let _ = writeln!(out, "{pc:04X}  {:<12} {}", bytes.join(" "), i.text);
        pc = pc.wrapping_add(u16::from(i.len));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(bytes: &[u8]) -> (u8, String) {
        let i = decode(bytes, 0x1000);
        (i.len, i.text)
    }

    /// Instructions from Zilog's manual, one or more of each form.
    #[test]
    fn decodes_the_manual_examples() {
        let cases: &[(&[u8], u8, &str)] = &[
            (&[0x00], 1, "nop"),
            (&[0x3a, 0x00, 0xe0], 3, "ld a,($e000)"),
            (&[0x32, 0x02, 0xe0], 3, "ld ($e002),a"),
            (&[0x21, 0x40, 0x00], 3, "ld hl,$0040"),
            (&[0x2a, 0x34, 0x12], 3, "ld hl,($1234)"),
            (&[0x22, 0x34, 0x12], 3, "ld ($1234),hl"),
            (&[0x18, 0xfe], 2, "jr $1000"),
            (&[0x20, 0x05], 2, "jr nz,$1007"),
            (&[0x10, 0xfe], 2, "djnz $1000"),
            (&[0xc3, 0x34, 0x12], 3, "jp $1234"),
            (&[0xda, 0x34, 0x12], 3, "jp c,$1234"),
            (&[0xcd, 0x34, 0x12], 3, "call $1234"),
            (&[0xc4, 0x34, 0x12], 3, "call nz,$1234"),
            (&[0xc9], 1, "ret"),
            (&[0xd8], 1, "ret c"),
            (&[0xff], 1, "rst $38"),
            (&[0xe9], 1, "jp (hl)"),
            (&[0xdd, 0xe9], 2, "jp (ix)"),
            (&[0xfd, 0xe9], 2, "jp (iy)"),
            (&[0x7e], 1, "ld a,(hl)"),
            (&[0x77], 1, "ld (hl),a"),
            (&[0x36, 0x12], 2, "ld (hl),$12"),
            (&[0xdd, 0x7e, 0x05], 3, "ld a,(ix+$05)"),
            (&[0xfd, 0x77, 0xfe], 3, "ld (iy-$02),a"),
            (&[0xdd, 0x36, 0x05, 0x12], 4, "ld (ix+$05),$12"),
            (&[0xdd, 0x66, 0x05], 3, "ld h,(ix+$05)"),
            (&[0xdd, 0x74, 0x05], 3, "ld (ix+$05),h"),
            (&[0xdd, 0x26, 0x12], 3, "ld ixh,$12"),
            (&[0xdd, 0x7c], 2, "ld a,ixh"),
            (&[0xdd, 0x21, 0x34, 0x12], 4, "ld ix,$1234"),
            (&[0xdd, 0x09], 2, "add ix,bc"),
            (&[0xdd, 0x34, 0x05], 3, "inc (ix+$05)"),
            (&[0xdd, 0xe5], 2, "push ix"),
            (&[0xdd, 0xe1], 2, "pop ix"),
            (&[0xdd, 0xe3], 2, "ex (sp),ix"),
            (&[0xdd, 0xf9], 2, "ld sp,ix"),
            (&[0xdd, 0x86, 0x05], 3, "add a,(ix+$05)"),
            (&[0xdd, 0xdd, 0x00], 1, "nop*"),
            (&[0xee, 0x1d], 2, "xor $1d"),
            (&[0xaf], 1, "xor a"),
            (&[0x07], 1, "rlca"),
            (&[0x27], 1, "daa"),
            (&[0x76], 1, "halt"),
            (&[0xdb, 0x01], 2, "in a,($01)"),
            (&[0xd3, 0x1c], 2, "out ($1c),a"),
            (&[0x08], 1, "ex af,af'"),
            (&[0xd9], 1, "exx"),
            (&[0xeb], 1, "ex de,hl"),
            (&[0xf3], 1, "di"),
            (&[0xcb, 0x07], 2, "rlc a"),
            (&[0xcb, 0x1e], 2, "rr (hl)"),
            (&[0xcb, 0x36], 2, "sll (hl)"),
            (&[0xcb, 0x7f], 2, "bit 7,a"),
            (&[0xcb, 0xc6], 2, "set 0,(hl)"),
            (&[0xcb, 0x86], 2, "res 0,(hl)"),
            (&[0xdd, 0xcb, 0x05, 0x06], 4, "rlc (ix+$05)"),
            (&[0xfd, 0xcb, 0xfe, 0x7e], 4, "bit 7,(iy-$02)"),
            (&[0xdd, 0xcb, 0x05, 0x00], 4, "rlc (ix+$05),b"),
            (&[0xdd, 0xcb, 0x05, 0xc7], 4, "set 0,(ix+$05),a"),
            (&[0xed, 0x4d], 2, "reti"),
            (&[0xed, 0x45], 2, "retn"),
            (&[0xed, 0x5e], 2, "im 2"),
            (&[0xed, 0x47], 2, "ld i,a"),
            (&[0xed, 0x57], 2, "ld a,i"),
            (&[0xed, 0xb0], 2, "ldir"),
            (&[0xed, 0xb9], 2, "cpdr"),
            (&[0xed, 0xa3], 2, "outi"),
            (&[0xed, 0x44], 2, "neg"),
            (&[0xed, 0x52], 2, "sbc hl,de"),
            (&[0xed, 0x4a], 2, "adc hl,bc"),
            (&[0xed, 0x43, 0x34, 0x12], 4, "ld ($1234),bc"),
            (&[0xed, 0x7b, 0x34, 0x12], 4, "ld sp,($1234)"),
            (&[0xed, 0x78], 2, "in a,(c)"),
            (&[0xed, 0x70], 2, "in (c)"),
            (&[0xed, 0x71], 2, "out (c),0"),
            (&[0xed, 0x6f], 2, "rld"),
            (&[0xed, 0x00], 2, "noni"),
        ];
        for (bytes, len, text) in cases {
            assert_eq!(t(bytes), (*len, (*text).to_owned()), "{bytes:02x?}");
        }
    }

    /// Every opcode of every prefix decodes, to one to four bytes, and the
    /// lengths agree with the instructions' operands.
    #[test]
    fn every_opcode_decodes() {
        for prefix in [
            &[][..],
            &[0xcb],
            &[0xed],
            &[0xdd],
            &[0xfd],
            &[0xdd, 0xcb, 0x05],
            &[0xfd, 0xcb, 0x05],
        ] {
            for op in 0..=255u8 {
                let mut bytes = prefix.to_vec();
                bytes.push(op);
                bytes.extend([0x34, 0x12, 0x56]);
                let i = decode(&bytes, 0);
                assert!((1..=4).contains(&i.len), "{bytes:02x?}: {}", i.len);
                assert!(!i.text.is_empty());
            }
        }
    }

    /// What control does and what each reads and writes.
    #[test]
    fn effects() {
        let i = decode(&[0xe9], 0);
        assert_eq!(i.flow, Flow::Indirect(Ptr::Hl));
        assert!(!i.flow.falls_through());
        let i = decode(&[0xdd, 0xe9], 0);
        assert_eq!(i.flow, Flow::Indirect(Ptr::Ix));
        let i = decode(&[0x20, 0x05], 0x1000);
        assert_eq!(i.flow, Flow::JumpIf(0x1007));
        assert!(i.flow.conditional() && i.reads & reg::F != 0);
        let i = decode(&[0x3a, 0x02, 0xe0], 0);
        assert_eq!(i.writes, reg::A);
        assert_eq!(
            i.mem,
            Some(Mem {
                addr: Addr::Abs(0xe002),
                read: true,
                write: false,
                word: false
            })
        );
        let i = decode(&[0xaf], 0);
        assert!(i.constant && i.op == Op::Alu(Alu::Xor));
        let i = decode(&[0xa7], 0);
        assert!(!i.constant);
        assert_eq!(i.writes, reg::A | reg::F);
        let i = decode(&[0xcb, 0x1e], 0);
        assert_eq!(i.op, Op::Shift);
        assert!(i.mem.unwrap().read && i.mem.unwrap().write);
        let i = decode(&[0xdd, 0x7e, 0xfb], 0);
        assert_eq!(i.mem.unwrap().addr, Addr::Index(Ptr::Ix, -5));
        assert!(i.reads & reg::IX == reg::IX);
        let i = decode(&[0xdb, 0x01], 0);
        assert_eq!(
            i.io,
            Some(Io {
                port: Some(1),
                write: false
            })
        );
        let i = decode(&[0xcd, 0x00, 0x20], 0);
        assert_eq!(i.flow, Flow::Call(0x2000));
        assert_eq!(i.mem.unwrap().addr, Addr::Stack);
    }

    #[test]
    fn lists_a_made_up_program() {
        let code = [0x3e, 0x07, 0x32, 0x00, 0xa0, 0x18, 0xf9];
        assert_eq!(
            listing(&code, 0, 0, 10),
            "0000  3E 07        ld a,$07\n0002  32 00 A0     ld ($a000),a\n0005  18 F9        jr $0000\n"
        );
    }
}
