//! Code coverage of the original (docs/research/dense-clips.md): which CPU
//! addresses a run executes, and which way each conditional branch went,
//! read between instructions through [`crate::Nes::step_with`]. It changes
//! nothing the console does, so a covered frame ends in the same state as
//! any other.
//!
//! A map is a set of feature numbers, one bit each:
//!
//! | Range | Feature |
//! |---|---|
//! | `0x0_0000..0x1_0000` | An instruction at this CPU address ran |
//! | `0x1_0000..0x2_0000` | The branch at this address was taken |
//! | `0x2_0000..0x3_0000` | The branch at this address fell through |
//! | `0x3_0000..0x4_0000` | A caller's own feature, such as a value of a mapped measure ([`CodeMap::extra`]) |
//!
//! Addresses are the CPU's, so on a board that switches banks two banks'
//! code at one address count once. Super Mario Bros. (mapper 0) has no
//! banks. A bank-aware key would read the event stream's bank switches.

use tetanes_core::bus::Bus;

/// The first feature of each range.
pub const EXEC: u32 = 0;
pub const TAKEN: u32 = 0x1_0000;
pub const FELL: u32 = 0x2_0000;
pub const EXTRA: u32 = 0x3_0000;
/// One past the last feature.
pub const SIZE: u32 = 0x4_0000;

const WORDS: usize = (SIZE / 64) as usize;

/// A set of features.
#[derive(Clone, PartialEq, Eq)]
pub struct CodeMap {
    bits: Vec<u64>,
}

impl Default for CodeMap {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CodeMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CodeMap({} features)", self.count())
    }
}

impl CodeMap {
    pub fn new() -> Self {
        Self {
            bits: vec![0; WORDS],
        }
    }

    pub fn clear(&mut self) {
        self.bits.fill(0);
    }

    /// Sets feature `id`; whether it was new.
    #[inline]
    pub fn set(&mut self, id: u32) -> bool {
        let (w, b) = ((id / 64) as usize, id % 64);
        let was = self.bits[w] >> b & 1;
        self.bits[w] |= 1 << b;
        was == 0
    }

    #[inline]
    pub fn get(&self, id: u32) -> bool {
        self.bits[(id / 64) as usize] >> (id % 64) & 1 == 1
    }

    /// Sets a caller's feature, `key` hashed into the extra range.
    pub fn extra(&mut self, key: u64) -> bool {
        // SplitMix64's finalizer: a fixed mix, the same on every machine.
        let mut z = key.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        self.set(EXTRA + (z as u32 & 0xFFFF))
    }

    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Features in `range`.
    pub fn count_in(&self, range: std::ops::Range<u32>) -> usize {
        range.filter(|&i| self.get(i)).count()
    }

    /// Adds every feature of `other`.
    pub fn union(&mut self, other: &CodeMap) {
        for (a, b) in self.bits.iter_mut().zip(&other.bits) {
            *a |= b;
        }
    }

    /// Features here that `base` lacks.
    pub fn new_over(&self, base: &CodeMap) -> usize {
        self.bits
            .iter()
            .zip(&base.bits)
            .map(|(a, b)| (a & !b).count_ones() as usize)
            .sum()
    }

    /// Every feature, in order.
    pub fn ids(&self) -> Vec<u32> {
        let mut out = Vec::new();
        for (w, &bits) in self.bits.iter().enumerate() {
            let mut x = bits;
            while x != 0 {
                let b = x.trailing_zeros();
                out.push(w as u32 * 64 + b);
                x &= x - 1;
            }
        }
        out
    }

    pub fn from_ids(ids: &[u32]) -> Self {
        let mut m = Self::new();
        for &i in ids {
            m.set(i);
        }
        m
    }
}

/// Whether `op` is one of the eight conditional branches (BPL, BMI, BVC,
/// BVS, BCC, BCS, BNE, BEQ): `xxy10000`.
pub fn is_branch(op: u8) -> bool {
    op & 0x1F == 0x10
}

/// Watches one run's instructions: give it the bus before each one.
#[derive(Debug, Default, Clone, Copy)]
pub struct Watch {
    prev: Option<u16>,
}

impl Watch {
    /// Forgets the instruction before, at a frame's start or a restore.
    pub fn reset(&mut self) {
        self.prev = None;
    }

    /// Notes the instruction about to run, and which way the one before
    /// went if it was a branch. An interrupt between them leaves the
    /// branch unjudged.
    #[inline]
    pub fn before(&mut self, bus: &Bus, map: &mut CodeMap) {
        let pc = bus.cpu.pc;
        map.set(EXEC + u32::from(pc));
        if let Some(p) = self.prev {
            let op = bus.peek(p);
            if is_branch(op) {
                let fall = p.wrapping_add(2);
                let to = fall.wrapping_add(bus.peek(p.wrapping_add(1)) as i8 as u16);
                if pc == to && to != fall {
                    map.set(TAKEN + u32::from(p));
                } else if pc == fall {
                    map.set(FELL + u32::from(p));
                }
            }
        }
        self.prev = Some(pc);
    }
}

/// A data access an instruction makes: read, written, or both.
pub const READ: u8 = 1;
pub const WRITE: u8 = 2;

/// The memory the instruction at the program counter reads or writes, and
/// how, worked out from its addressing mode and the registers before it
/// runs: `None` for an instruction with no data address (immediate,
/// implied, a branch, a jump or call, a stack push or pull). Official
/// opcodes only; an unofficial one is judged by its column, as the
/// official ones around it are.
pub fn access(bus: &Bus) -> Option<(u16, u8)> {
    let cpu = &bus.cpu;
    let pc = cpu.pc;
    let op = bus.peek(pc);
    let b1 = bus.peek(pc.wrapping_add(1));
    let w = || u16::from_le_bytes([b1, bus.peek(pc.wrapping_add(2))]);
    let (x, y) = (u16::from(cpu.x), u16::from(cpu.y));
    let zp = |a: u8| {
        u16::from_le_bytes([
            bus.peek(u16::from(a)),
            bus.peek(u16::from(a.wrapping_add(1))),
        ])
    };
    let (cc, bbb) = (op & 3, (op >> 2) & 7);
    // Jumps, calls, returns, BRK, pushes and pulls, and the immediates of
    // the 00 column.
    if matches!(
        op,
        0x00 | 0x20 | 0x40 | 0x60 | 0x4C | 0x6C | 0x08 | 0x28 | 0x48 | 0x68
    ) || (cc == 0 && matches!(bbb, 0 | 2 | 4 | 6))
    {
        return None;
    }
    let addr = match (cc, bbb) {
        (1, 0) => zp(b1.wrapping_add(cpu.x)),
        (1, 4) => zp(b1).wrapping_add(y),
        (1, 2) | (2, 0) | (2, 2) | (2, 6) => return None,
        (_, 1) => u16::from(b1),
        (_, 3) => w(),
        (1, 6) => w().wrapping_add(y),
        (2, 5) if matches!(op, 0x96 | 0xB6) => u16::from(b1.wrapping_add(cpu.y)),
        (_, 5) => u16::from(b1.wrapping_add(cpu.x)),
        (2, 7) if op == 0xBE => w().wrapping_add(y),
        (_, 7) => w().wrapping_add(x),
        _ => return None,
    };
    // STA, STX, STY write; the shifts, INC and DEC (column 10 but STX,
    // LDX) read and write; the rest read.
    let rw = match (cc, op >> 5) {
        (_, 4) => WRITE,
        (2, 0..=3) | (2, 6) | (2, 7) => READ | WRITE,
        _ => READ,
    };
    Some((addr, rw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_counts_unions_and_says_what_is_new() {
        let mut a = CodeMap::new();
        assert!(a.set(0x8000));
        assert!(!a.set(0x8000));
        a.set(TAKEN + 0x8010);
        let mut b = CodeMap::new();
        b.set(0x8000);
        b.set(FELL + 0x8010);
        assert_eq!(b.new_over(&a), 1);
        a.union(&b);
        assert_eq!(a.count(), 3);
        assert_eq!(a.ids(), vec![0x8000, TAKEN + 0x8010, FELL + 0x8010]);
        assert_eq!(CodeMap::from_ids(&a.ids()), a);
        assert_eq!(a.count_in(0x8000..0x1_0000), 1);
        // The same key, the same feature.
        let (mut x, mut y) = (CodeMap::new(), CodeMap::new());
        x.extra(42);
        y.extra(42);
        assert_eq!(x, y);
        assert_eq!(x.count_in(EXTRA..SIZE), 1);
    }

    #[test]
    fn the_eight_branches_are_known_by_their_opcode() {
        let ops: Vec<u8> = (0..=255u8).filter(|&o| is_branch(o)).collect();
        assert_eq!(ops, vec![0x10, 0x30, 0x50, 0x70, 0x90, 0xB0, 0xD0, 0xF0]);
    }
}
