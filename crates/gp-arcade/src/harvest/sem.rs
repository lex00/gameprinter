//! What each Z80 instruction does, for the static reader ([`super::absint`]):
//! the registers, memory, flags and stack it reads and writes, as one
//! [`Sem`] per instruction. Decoded from the opcode's bits as
//! [`crate::dasm`] decodes them (x = 7-6, y = 5-3, z = 2-0, p = y >> 1,
//! q = y & 1, from Zilog's manual and "Decoding Z80 opcodes"); dasm still
//! gives the length, the text and the flow.
//!
//! A DD or FD prefix turns HL into IX or IY, `(hl)` into `(ix+d)`, and H
//! and L into the index register's halves, except in an instruction that
//! also uses `(ix+d)`, and never in `ex de,hl` or an ED opcode.

/// An 8-bit register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum R8 {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
    Ixh,
    Ixl,
    Iyh,
    Iyl,
    I,
    R,
}

/// A register pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum R16 {
    Bc,
    De,
    Hl,
    Sp,
    Ix,
    Iy,
    Af,
}

impl R16 {
    /// The pair's high and low registers (AF's low is the flags: `None`).
    pub fn halves(self) -> Option<(R8, Option<R8>)> {
        Some(match self {
            Self::Bc => (R8::B, Some(R8::C)),
            Self::De => (R8::D, Some(R8::E)),
            Self::Hl => (R8::H, Some(R8::L)),
            Self::Ix => (R8::Ixh, Some(R8::Ixl)),
            Self::Iy => (R8::Iyh, Some(R8::Iyl)),
            Self::Af => (R8::A, None),
            Self::Sp => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Bc => "bc",
            Self::De => "de",
            Self::Hl => "hl",
            Self::Sp => "sp",
            Self::Ix => "ix",
            Self::Iy => "iy",
            Self::Af => "af",
        }
    }
}

impl R8 {
    /// The pair it is half of, and whether it is the high half.
    pub fn pair(self) -> Option<(R16, bool)> {
        Some(match self {
            Self::B => (R16::Bc, true),
            Self::C => (R16::Bc, false),
            Self::D => (R16::De, true),
            Self::E => (R16::De, false),
            Self::H => (R16::Hl, true),
            Self::L => (R16::Hl, false),
            Self::Ixh => (R16::Ix, true),
            Self::Ixl => (R16::Ix, false),
            Self::Iyh => (R16::Iy, true),
            Self::Iyl => (R16::Iy, false),
            Self::A | Self::I | Self::R => return None,
        })
    }
}

/// A memory operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mem {
    /// `(nn)`.
    Abs(u16),
    /// `(bc)`, `(de)`, `(hl)`.
    Pair(R16),
    /// `(ix+d)`, `(iy+d)`.
    Idx(R16, i8),
}

/// An 8-bit operand read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Src {
    R(R8),
    N(u8),
    M(Mem),
}

/// An 8-bit operand written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dst {
    R(R8),
    M(Mem),
}

impl Dst {
    pub fn src(self) -> Src {
        match self {
            Self::R(r) => Src::R(r),
            Self::M(m) => Src::M(m),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Alu {
    Add,
    Adc,
    Sub,
    Sbc,
    And,
    Xor,
    Or,
    Cp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rot {
    Rlc,
    Rrc,
    Rl,
    Rr,
    Sla,
    Sra,
    Sll,
    Srl,
}

/// A branch condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cc {
    Nz,
    Z,
    Nc,
    C,
    Po,
    Pe,
    P,
    M,
    /// `djnz`: B not zero after the decrement.
    B,
}

impl Cc {
    pub fn name(self) -> &'static str {
        ["nz", "z", "nc", "c", "po", "pe", "p", "m", "djnz"][self as usize]
    }
}

const CCS: [Cc; 8] = [Cc::Nz, Cc::Z, Cc::Nc, Cc::C, Cc::Po, Cc::Pe, Cc::P, Cc::M];
const ALUS: [Alu; 8] = [
    Alu::Add,
    Alu::Adc,
    Alu::Sub,
    Alu::Sbc,
    Alu::And,
    Alu::Xor,
    Alu::Or,
    Alu::Cp,
];
const ROTS: [Rot; 8] = [
    Rot::Rlc,
    Rot::Rrc,
    Rot::Rl,
    Rot::Rr,
    Rot::Sla,
    Rot::Sra,
    Rot::Sll,
    Rot::Srl,
];

/// The block instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    /// `ldi`, `ldd`, `ldir`, `lddr`: (DE) = (HL), BC times when repeating.
    Ld { up: bool, repeat: bool },
    /// `cpi`, `cpd`, `cpir`, `cpdr`.
    Cp { up: bool, repeat: bool },
    /// `ini`, `outi` and the rest.
    Io,
}

/// What one instruction does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sem {
    Nop,
    Ld(Dst, Src),
    /// `ld rr,nn`.
    LdImm16(R16, u16),
    /// `ld rr,(nn)`.
    LdMem16(R16, u16),
    /// `ld (nn),rr`.
    StMem16(u16, R16),
    /// `ld sp,hl` (or ix, iy).
    LdSp(R16),
    Alu(Alu, Src),
    /// `inc` (+1) or `dec` (-1) of an 8-bit place.
    Step(Dst, i8),
    /// The same of a pair; sets no flags.
    Step16(R16, i8),
    /// `add hl,rr` (`None`), `adc hl,rr` (`Some(false)`), `sbc hl,rr`
    /// (`Some(true)`).
    Add16(R16, R16, Option<bool>),
    /// A shift or rotate; the accumulator forms (`rlca`) say so in `acc`;
    /// `copy` is the undocumented DDCB copy to a register.
    Rot {
        op: Rot,
        dst: Dst,
        copy: Option<R8>,
        acc: bool,
    },
    Bit(u8, Src),
    /// `res` (false) or `set` (true) of bit n.
    SetBit(u8, Dst, bool, Option<R8>),
    ExDeHl,
    ExAf,
    Exx,
    /// `ex (sp),hl` (or ix, iy).
    ExSp(R16),
    Push(R16),
    Pop(R16),
    /// `in a,(n)` (`Some(n)`), `in r,(c)` (`None`); `in (c)` sets only the
    /// flags (no register).
    In(Option<R8>, Option<u8>),
    Out(Option<u8>, Src),
    Block(Block),
    Daa,
    Cpl,
    Neg,
    Scf,
    Ccf,
    Di,
    Ei,
    Im(u8),
    Halt,
    /// `rld`, `rrd`: A and (HL) trade nibbles.
    Rxd,
    Jp(Option<Cc>, u16),
    Djnz(u16),
    Call(Option<Cc>, u16),
    Ret(Option<Cc>),
    /// `reti`, `retn`.
    Reti,
    Rst(u16),
    /// `jp (hl)`, `jp (ix)`, `jp (iy)`.
    JpInd(R16),
}

struct D<'a> {
    b: &'a [u8],
    pc: u16,
    idx: Option<R16>,
    at: usize,
}

impl D<'_> {
    fn byte(&self, i: usize) -> u8 {
        self.b.get(i).copied().unwrap_or(0)
    }
    fn word(&self, i: usize) -> u16 {
        u16::from_le_bytes([self.byte(i), self.byte(i + 1)])
    }
    fn hl(&self) -> R16 {
        self.idx.unwrap_or(R16::Hl)
    }
    fn disp(&self) -> i8 {
        self.byte(self.at + 1) as i8
    }
    /// Register `i` of the r table; `(hl)` or `(ix+d)` as memory. With
    /// `keep`, H and L stay themselves (the instruction uses `(ix+d)`).
    fn r(&self, i: usize, keep: bool) -> Dst {
        let r = match i {
            0 => R8::B,
            1 => R8::C,
            2 => R8::D,
            3 => R8::E,
            4 => match (self.idx, keep) {
                (Some(R16::Ix), false) => R8::Ixh,
                (Some(R16::Iy), false) => R8::Iyh,
                _ => R8::H,
            },
            5 => match (self.idx, keep) {
                (Some(R16::Ix), false) => R8::Ixl,
                (Some(R16::Iy), false) => R8::Iyl,
                _ => R8::L,
            },
            6 => {
                return Dst::M(match self.idx {
                    None => Mem::Pair(R16::Hl),
                    Some(p) => Mem::Idx(p, self.disp()),
                });
            }
            _ => R8::A,
        };
        Dst::R(r)
    }
    fn rp(&self, p: usize) -> R16 {
        [R16::Bc, R16::De, self.hl(), R16::Sp][p]
    }
    fn rp2(&self, p: usize) -> R16 {
        [R16::Bc, R16::De, self.hl(), R16::Af][p]
    }
    /// The target of a relative jump whose instruction is `len` bytes.
    fn rel(&self, len: u16) -> u16 {
        let e = self.byte(self.at + 1) as i8;
        self.pc.wrapping_add(len).wrapping_add(e as u16)
    }
}

/// What the instruction at `pc` does; `bytes` starts there (four are
/// enough; missing ones read as zero).
pub fn decode(bytes: &[u8], pc: u16) -> Sem {
    let mut d = D {
        b: bytes,
        pc,
        idx: None,
        at: 0,
    };
    match d.byte(0) {
        0xdd | 0xfd => {
            d.idx = Some(if d.byte(0) == 0xdd { R16::Ix } else { R16::Iy });
            d.at = 1;
            match d.byte(1) {
                0xdd | 0xfd | 0xed => Sem::Nop,
                0xcb => cb(&d),
                _ => main(&d),
            }
        }
        0xcb => cb(&d),
        0xed => ed(&d),
        _ => main(&d),
    }
}

fn main(d: &D<'_>) -> Sem {
    let op = d.byte(d.at);
    let (x, y, z) = (op >> 6, usize::from((op >> 3) & 7), usize::from(op & 7));
    let (p, q) = (y >> 1, y & 1);
    // The instruction's length up to its operands: prefix and opcode.
    let base = d.at as u16 + 1;
    match (x, z) {
        (0, 0) => match y {
            0 => Sem::Nop,
            1 => Sem::ExAf,
            2 => Sem::Djnz(d.rel(base + 1)),
            3 => Sem::Jp(None, d.rel(base + 1)),
            _ => Sem::Jp(Some(CCS[y - 4]), d.rel(base + 1)),
        },
        (0, 1) => {
            if q == 0 {
                Sem::LdImm16(d.rp(p), d.word(d.at + 1))
            } else {
                Sem::Add16(d.hl(), d.rp(p), None)
            }
        }
        (0, 2) => {
            let nn = d.word(d.at + 1);
            match (q, p) {
                (0, 0) => Sem::Ld(Dst::M(Mem::Pair(R16::Bc)), Src::R(R8::A)),
                (0, 1) => Sem::Ld(Dst::M(Mem::Pair(R16::De)), Src::R(R8::A)),
                (0, 2) => Sem::StMem16(nn, d.hl()),
                (0, _) => Sem::Ld(Dst::M(Mem::Abs(nn)), Src::R(R8::A)),
                (_, 0) => Sem::Ld(Dst::R(R8::A), Src::M(Mem::Pair(R16::Bc))),
                (_, 1) => Sem::Ld(Dst::R(R8::A), Src::M(Mem::Pair(R16::De))),
                (_, 2) => Sem::LdMem16(d.hl(), nn),
                (_, _) => Sem::Ld(Dst::R(R8::A), Src::M(Mem::Abs(nn))),
            }
        }
        (0, 3) => Sem::Step16(d.rp(p), if q == 0 { 1 } else { -1 }),
        (0, 4) => Sem::Step(d.r(y, false), 1),
        (0, 5) => Sem::Step(d.r(y, false), -1),
        (0, 6) => {
            let dst = d.r(y, false);
            // `ld r,n`: n follows the opcode, after the displacement for
            // `(ix+d)`.
            let at = d.at + 1 + usize::from(y == 6 && d.idx.is_some());
            Sem::Ld(dst, Src::N(d.byte(at)))
        }
        (0, _) => match y {
            0 => Sem::Rot {
                op: Rot::Rlc,
                dst: Dst::R(R8::A),
                copy: None,
                acc: true,
            },
            1 => Sem::Rot {
                op: Rot::Rrc,
                dst: Dst::R(R8::A),
                copy: None,
                acc: true,
            },
            2 => Sem::Rot {
                op: Rot::Rl,
                dst: Dst::R(R8::A),
                copy: None,
                acc: true,
            },
            3 => Sem::Rot {
                op: Rot::Rr,
                dst: Dst::R(R8::A),
                copy: None,
                acc: true,
            },
            4 => Sem::Daa,
            5 => Sem::Cpl,
            6 => Sem::Scf,
            _ => Sem::Ccf,
        },
        (1, _) => {
            if y == 6 && z == 6 {
                return Sem::Halt;
            }
            let uses_m = y == 6 || z == 6;
            Sem::Ld(d.r(y, uses_m), d.r(z, uses_m).src())
        }
        (2, _) => Sem::Alu(ALUS[y], d.r(z, false).src()),
        (3, 0) => Sem::Ret(Some(CCS[y])),
        (3, 1) => match (q, p) {
            (0, _) => Sem::Pop(d.rp2(p)),
            (_, 0) => Sem::Ret(None),
            (_, 1) => Sem::Exx,
            (_, 2) => Sem::JpInd(d.hl()),
            (_, _) => Sem::LdSp(d.hl()),
        },
        (3, 2) => Sem::Jp(Some(CCS[y]), d.word(d.at + 1)),
        (3, 3) => match y {
            0 => Sem::Jp(None, d.word(d.at + 1)),
            2 => Sem::Out(Some(d.byte(d.at + 1)), Src::R(R8::A)),
            3 => Sem::In(Some(R8::A), Some(d.byte(d.at + 1))),
            4 => Sem::ExSp(d.hl()),
            5 => Sem::ExDeHl,
            6 => Sem::Di,
            7 => Sem::Ei,
            // 1 is CB, handled before.
            _ => Sem::Nop,
        },
        (3, 4) => Sem::Call(Some(CCS[y]), d.word(d.at + 1)),
        (3, 5) => match (q, p) {
            (0, _) => Sem::Push(d.rp2(p)),
            (_, 0) => Sem::Call(None, d.word(d.at + 1)),
            _ => Sem::Nop,
        },
        (3, 6) => Sem::Alu(ALUS[y], Src::N(d.byte(d.at + 1))),
        _ => Sem::Rst((y as u16) * 8),
    }
}

fn cb(d: &D<'_>) -> Sem {
    // DDCB d op: the displacement comes before the opcode.
    let (op, dst) = match d.idx {
        Some(p) => (d.byte(d.at + 2), Dst::M(Mem::Idx(p, d.disp()))),
        None => (d.byte(1), Dst::R(R8::A)),
    };
    let (x, y, z) = (op >> 6, (op >> 3) & 7, usize::from(op & 7));
    let plain = D {
        b: d.b,
        pc: d.pc,
        idx: None,
        at: 0,
    };
    // Unprefixed: the operand is register z. Prefixed: (ix+d), and a z
    // other than 6 also gets the result (undocumented).
    let (dst, copy) = match d.idx {
        None => (plain.r(z, true), None),
        Some(_) => (
            dst,
            match plain.r(z, true) {
                Dst::R(r) if z != 6 => Some(r),
                _ => None,
            },
        ),
    };
    match x {
        0 => Sem::Rot {
            op: ROTS[usize::from(y)],
            dst,
            copy,
            acc: false,
        },
        1 => Sem::Bit(y, dst.src()),
        2 => Sem::SetBit(y, dst, false, copy),
        _ => Sem::SetBit(y, dst, true, copy),
    }
}

fn ed(d: &D<'_>) -> Sem {
    let op = d.byte(1);
    let (x, y, z) = (op >> 6, usize::from((op >> 3) & 7), op & 7);
    let (p, q) = (y >> 1, y & 1);
    let plain = D {
        b: d.b,
        pc: d.pc,
        idx: None,
        at: 1,
    };
    match (x, z) {
        (1, 0) => Sem::In(
            match plain.r(y, true) {
                Dst::R(r) if y != 6 => Some(r),
                _ => None,
            },
            None,
        ),
        (1, 1) => Sem::Out(
            None,
            match plain.r(y, true) {
                Dst::R(r) if y != 6 => Src::R(r),
                _ => Src::N(0),
            },
        ),
        (1, 2) => Sem::Add16(R16::Hl, plain.rp(p), Some(q == 0)),
        (1, 3) => {
            let nn = d.word(2);
            if q == 0 {
                Sem::StMem16(nn, plain.rp(p))
            } else {
                Sem::LdMem16(plain.rp(p), nn)
            }
        }
        (1, 4) => Sem::Neg,
        (1, 5) => Sem::Reti,
        (1, 6) => Sem::Im(match y & 3 {
            2 => 1,
            3 => 2,
            _ => 0,
        }),
        (1, 7) => match y {
            0 => Sem::Ld(Dst::R(R8::I), Src::R(R8::A)),
            1 => Sem::Ld(Dst::R(R8::R), Src::R(R8::A)),
            2 => Sem::Ld(Dst::R(R8::A), Src::R(R8::I)),
            3 => Sem::Ld(Dst::R(R8::A), Src::R(R8::R)),
            4 | 5 => Sem::Rxd,
            _ => Sem::Nop,
        },
        (2, _) if y >= 4 && z <= 3 => {
            let up = y & 1 == 0;
            let repeat = y >= 6;
            match z {
                0 => Sem::Block(Block::Ld { up, repeat }),
                1 => Sem::Block(Block::Cp { up, repeat }),
                _ => Sem::Block(Block::Io),
            }
        }
        _ => Sem::Nop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_indexed_operands() {
        assert_eq!(
            decode(&[0x3a, 0x97, 0xe3], 0),
            Sem::Ld(Dst::R(R8::A), Src::M(Mem::Abs(0xe397)))
        );
        assert_eq!(
            decode(&[0xfd, 0x7e, 0x00], 0),
            Sem::Ld(Dst::R(R8::A), Src::M(Mem::Idx(R16::Iy, 0)))
        );
        assert_eq!(
            decode(&[0xfd, 0x36, 0x00, 0x05], 0),
            Sem::Ld(Dst::M(Mem::Idx(R16::Iy, 0)), Src::N(5))
        );
        assert_eq!(
            decode(&[0xdd, 0x66, 0x02], 0),
            Sem::Ld(Dst::R(R8::H), Src::M(Mem::Idx(R16::Ix, 2)))
        );
        assert_eq!(
            decode(&[0xdd, 0x7c], 0),
            Sem::Ld(Dst::R(R8::A), Src::R(R8::Ixh))
        );
        assert_eq!(
            decode(&[0xfd, 0x35, 0x04], 0),
            Sem::Step(Dst::M(Mem::Idx(R16::Iy, 4)), -1)
        );
        assert_eq!(decode(&[0xdd, 0x19], 0), Sem::Add16(R16::Ix, R16::De, None));
        assert_eq!(decode(&[0xeb], 0), Sem::ExDeHl);
        assert_eq!(decode(&[0xdd, 0xe9], 0), Sem::JpInd(R16::Ix));
    }

    #[test]
    fn prefixed_bit_operations_and_ed() {
        assert_eq!(
            decode(&[0xfd, 0xcb, 0x0d, 0x46], 0),
            Sem::Bit(0, Src::M(Mem::Idx(R16::Iy, 0x0d)))
        );
        assert_eq!(
            decode(&[0xcb, 0x3f], 0),
            Sem::Rot {
                op: Rot::Srl,
                dst: Dst::R(R8::A),
                copy: None,
                acc: false
            }
        );
        assert_eq!(
            decode(&[0xed, 0x5f], 0),
            Sem::Ld(Dst::R(R8::A), Src::R(R8::R))
        );
        assert_eq!(
            decode(&[0xed, 0x47], 0),
            Sem::Ld(Dst::R(R8::I), Src::R(R8::A))
        );
        assert_eq!(decode(&[0xed, 0x5e], 0), Sem::Im(2));
        assert_eq!(
            decode(&[0xed, 0xb0], 0),
            Sem::Block(Block::Ld {
                up: true,
                repeat: true
            })
        );
        assert_eq!(
            decode(&[0xed, 0x4b, 0x9e, 0xe3], 0),
            Sem::LdMem16(R16::Bc, 0xe39e)
        );
    }

    #[test]
    fn flow_targets() {
        assert_eq!(decode(&[0x20, 0xfb], 0x000a), Sem::Jp(Some(Cc::Nz), 0x0007));
        assert_eq!(decode(&[0x10, 0xeb], 0x0b60), Sem::Djnz(0x0b4d));
        assert_eq!(decode(&[0xcd, 0x0a, 0x10], 0), Sem::Call(None, 0x100a));
        assert_eq!(decode(&[0xd0], 0), Sem::Ret(Some(Cc::Nc)));
        assert_eq!(decode(&[0xff], 0), Sem::Rst(0x38));
    }
}
