//! The ROM's code map: its routines and the call, jump and branch graph
//! between them, found from the code alone (no labels, no source), shared
//! by `gp clips matrix --labels rom`, `gp capture steer` and the static
//! harvest ([`crate::harvest`], `gp model static`).
//!
//! [`Routines::from_rom`] walks the code from the reset, NMI and IRQ
//! vectors. [`decode`] names each official 6502 opcode and its addressing
//! mode. [`RomCode::of`] gives every instruction the walk reached, with the
//! routine it is in, and is cached by the image's BLAKE3 hash, so tools that ask for
//! the same ROM twice in one process walk it once.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use crate::coverage::CodeMap;

/// The game's routines: each a name, its first address and its size.
#[derive(Debug, Clone, Default)]
pub struct Routines {
    pub names: Vec<String>,
    pub addr: Vec<u16>,
    pub size: Vec<u32>,
    /// For each CPU address, its routine's index plus one, or 0.
    of: Vec<u32>,
}

impl Routines {
    /// From `(name, addr, size)`; a size of 0 runs to the next routine.
    pub fn new(mut list: Vec<(String, u16, u32)>) -> Self {
        list.retain(|(_, a, _)| *a >= 0x8000);
        list.sort_by_key(|(_, a, _)| *a);
        list.dedup_by_key(|(_, a, _)| *a);
        let mut r = Routines {
            of: vec![0; 0x1_0000],
            ..Default::default()
        };
        for i in 0..list.len() {
            let (name, a, s) = &list[i];
            let next = list.get(i + 1).map_or(0x1_0000, |n| u32::from(n.1));
            let size = if *s == 0 {
                next - u32::from(*a)
            } else {
                (*s).min(next - u32::from(*a))
            };
            for x in u32::from(*a)..u32::from(*a) + size {
                r.of[x as usize] = i as u32 + 1;
            }
            r.names.push(name.clone());
            r.addr.push(*a);
            r.size.push(size);
        }
        r
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// The routine an address is in.
    pub fn at(&self, addr: u16) -> Option<usize> {
        match self.of[addr as usize] {
            0 => None,
            i => Some(i as usize - 1),
        }
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    /// The routines with an instruction run in `map`.
    pub fn run_in(&self, map: &CodeMap) -> BTreeSet<usize> {
        (0x8000u32..0x1_0000)
            .filter(|&a| map.get(a))
            .filter_map(|a| self.at(a as u16))
            .collect()
    }
}

/// What a walk of the code from one start reaches: the instructions, the
/// jumps (from, to), the calls, and whether it returns.
type Body = (BTreeSet<u16>, Vec<(u16, u16)>, Vec<u16>, bool);

impl Routines {
    /// The routines of a ROM with no labels, from its code alone: a walk
    /// from the reset, NMI and IRQ vectors that follows branches, calls
    /// and jumps and stops at returns and indirect jumps. A routine starts
    /// at each vector and each call's target, and at each jump's target
    /// that the code from the jump's own routine start doesn't reach by
    /// falling through or branching (a tail call into another routine),
    /// or that a later jump goes back to when the code from there never
    /// returns (a main loop the reset code falls into). Each is named `sub_XXXX` by its address. Data the walk never
    /// reaches is in no routine.
    pub fn from_rom(read: &dyn Fn(u16) -> u8) -> Self {
        Self::from_rom_with(read, &[])
    }

    /// [`Routines::from_rom`] with more starts: the entries of jump tables
    /// found by a reader that follows indirect jumps
    /// ([`crate::harvest`]).
    pub fn from_rom_with(read: &dyn Fn(u16) -> u8, extra: &[u16]) -> Self {
        Self::from_rom_jumps(read, extra, &[])
    }

    /// [`Routines::from_rom_with`] with the entries of jump tables, as
    /// `(site, entry)`: each entry starts a routine. A reader that sees
    /// the code reached from a site never return goes on into it as the
    /// jumping routine's own path ([`crate::harvest::absint`]).
    pub fn from_rom_jumps(read: &dyn Fn(u16) -> u8, extra: &[u16], jumps: &[(u16, u16)]) -> Self {
        let vec = |a: u16| u16::from_le_bytes([read(a), read(a.wrapping_add(1))]);
        let starts: BTreeSet<u16> = [vec(0xFFFC), vec(0xFFFA), vec(0xFFFE)]
            .into_iter()
            .chain(extra.iter().copied())
            .chain(jumps.iter().map(|&(_, to)| to))
            .filter(|&a| a >= 0x8000)
            .collect();
        let mut starts = starts;
        // The code each start reaches by falling through and branching,
        // and the jumps and calls it makes.
        let body = |start: u16| -> Body {
            let (mut seen, mut jumps, mut calls) = (BTreeSet::new(), Vec::new(), Vec::new());
            let mut returns = false;
            let mut todo = vec![start];
            while let Some(pc) = todo.pop() {
                if pc < 0x8000 || !seen.insert(pc) {
                    continue;
                }
                let op = read(pc);
                let next = pc.wrapping_add(op_len(op));
                let w = vec(pc.wrapping_add(1));
                match op {
                    0x40 | 0x60 => returns = true,
                    0x00 | 0x6C => {}
                    0x4C => jumps.push((pc, w)),
                    0x20 => {
                        calls.push(w);
                        // A routine that pulls its return address doesn't
                        // come back: what follows the call is its data.
                        if !pulls_return(&|a| read(a as u16), u32::from(w)) {
                            todo.push(next);
                        }
                    }
                    _ if op & 0x1F == 0x10 => {
                        let off = read(pc.wrapping_add(1)) as i8;
                        todo.push(next.wrapping_add(off as u16));
                        todo.push(next);
                    }
                    _ => todo.push(next),
                }
            }
            // A jump out of the code reached is a tail call, which returns
            // for it.
            returns |= jumps.iter().any(|(_, to)| !seen.contains(to));
            (seen, jumps, calls, returns)
        };
        let mut done: BTreeSet<u16> = BTreeSet::new();
        loop {
            let todo: Vec<u16> = starts.difference(&done).copied().collect();
            if todo.is_empty() {
                break;
            }
            for st in todo {
                done.insert(st);
                let (seen, jumps, calls, _) = body(st);
                starts.extend(calls.into_iter().filter(|&a| a >= 0x8000));
                for (from, to) in jumps {
                    if to < 0x8000 {
                        continue;
                    }
                    let loops_back = to > st && to < from && !body(to).3;
                    if !seen.contains(&to) || loops_back {
                        starts.insert(to);
                    }
                }
            }
        }
        // A routine runs from its start to the next, within the code the
        // walk reached.
        let mut code: BTreeSet<u16> = BTreeSet::new();
        for &st in &starts {
            code.extend(body(st).0);
        }
        let list: Vec<(String, u16, u32)> = starts
            .iter()
            .map(|&a| {
                let mut end = u32::from(a);
                while end < 0x1_0000
                    && (end == u32::from(a) || !starts.contains(&(end as u16)))
                    && code.contains(&(end as u16))
                {
                    end += u32::from(op_len(read(end as u16)));
                }
                (format!("sub_{a:04X}"), a, end - u32::from(a))
            })
            .collect();
        Self::new(list)
    }
}

/// Whether the routine at `start` is a jump engine: in straight-line code
/// from its start (no branch, call or return) it pulls its caller's return
/// address (two bytes more than it pushed) and jumps through a pointer
/// (`JMP (ptr)`). It reads the table that follows the call
/// (`jsr engine` then `.word a, b, c`), so the call never comes back to
/// the next byte. A routine that pulls the address to read data after the
/// call and returns past it (a print routine's inline string) branches or
/// returns first, and isn't one.
pub fn pulls_return(read: &dyn Fn(Va) -> u8, start: Va) -> bool {
    let mut pc = start;
    let mut depth = 0i32;
    for _ in 0..16 {
        let op = read(pc);
        match op {
            0x48 | 0x08 => depth += 1,
            0x68 | 0x28 => depth -= 1,
            0x6C => return depth <= -2,
            0x00 | 0x20 | 0x40 | 0x4C | 0x60 => return false,
            _ if op & 0x1F == 0x10 => return false,
            _ => {}
        }
        if decode(op).is_none() {
            return false;
        }
        pc = va_add(pc, op_len(op));
    }
    false
}

/// Bytes of a 6502 instruction by its opcode.
fn op_len(op: u8) -> u16 {
    match op {
        0x20 => 3,
        _ if matches!(op & 0x0F, 0x0C..=0x0E) => 3,
        _ if op & 0x1F == 0x19 => 3,
        0x00 | 0x40 | 0x60 => 1,
        _ if op & 0x0F == 0x08 || op & 0x0F == 0x0A && op < 0x80 => 1,
        0x8A | 0x9A | 0xAA | 0xBA | 0xCA | 0xEA => 1,
        _ => 2,
    }
}

/// The call, jump and branch graph between routines, decoded from the
/// code in memory: each routine's successors.
#[allow(clippy::needless_range_loop)]
pub fn graph(read: &dyn Fn(u16) -> u8, r: &Routines) -> Vec<BTreeSet<usize>> {
    let mut out = vec![BTreeSet::new(); r.len()];
    let start_of: BTreeSet<u16> = r.addr.iter().copied().collect();
    for i in 0..r.len() {
        let (a0, end) = (u32::from(r.addr[i]), u32::from(r.addr[i]) + r.size[i]);
        let mut pc = a0;
        let mut ended = false;
        while pc < end {
            let op = read(pc as u16);
            let len = u32::from(op_len(op));
            let w = |at: u32| u16::from_le_bytes([read(at as u16), read((at + 1) as u16)]);
            let mut add = |t: u16| {
                if let Some(j) = r.at(t)
                    && j != i
                {
                    out[i].insert(j);
                }
            };
            match op {
                0x20 => {
                    add(w(pc + 1));
                    // An inline jump table after the call (a jump engine):
                    // two or more words that each start a routine.
                    let mut k = pc + 3;
                    let mut n = 0;
                    while k + 1 < 0x1_0000 && start_of.contains(&w(k)) {
                        add(w(k));
                        k += 2;
                        n += 1;
                    }
                    if n >= 2 {
                        ended = true;
                        break;
                    }
                }
                0x4C => {
                    add(w(pc + 1));
                    ended = true;
                    break;
                }
                0x6C | 0x60 | 0x40 | 0x00 => {
                    ended = true;
                    break;
                }
                _ if op & 0x1F == 0x10 => {
                    let off = read((pc + 1) as u16) as i8;
                    add((pc as i32 + 2 + i32::from(off)) as u16);
                }
                _ => {}
            }
            pc += len;
        }
        if !ended
            && let Some(j) = r.at(end.min(0xFFFF) as u16)
            && end < 0x1_0000
        {
            out[i].insert(j);
        }
    }
    out
}

/// Each routine's distance to `target` in the graph: the fewest calls,
/// jumps, branches or fall-throughs, or `None` when nothing leads there.
pub fn distances(g: &[BTreeSet<usize>], target: usize) -> Vec<Option<u32>> {
    let mut pred = vec![Vec::new(); g.len()];
    for (i, s) in g.iter().enumerate() {
        for &j in s {
            pred[j].push(i);
        }
    }
    let mut d = vec![None; g.len()];
    d[target] = Some(0);
    let mut q = VecDeque::from([target]);
    while let Some(x) = q.pop_front() {
        let dx = d[x].unwrap_or(0);
        for &p in &pred[x] {
            if d[p].is_none() {
                d[p] = Some(dx + 1);
                q.push_back(p);
            }
        }
    }
    d
}

/// A 6502 instruction's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mn {
    Adc,
    And,
    Asl,
    Bcc,
    Bcs,
    Beq,
    Bit,
    Bmi,
    Bne,
    Bpl,
    Brk,
    Bvc,
    Bvs,
    Clc,
    Cld,
    Cli,
    Clv,
    Cmp,
    Cpx,
    Cpy,
    Dec,
    Dex,
    Dey,
    Eor,
    Inc,
    Inx,
    Iny,
    Jmp,
    Jsr,
    Lda,
    Ldx,
    Ldy,
    Lsr,
    Nop,
    Ora,
    Pha,
    Php,
    Pla,
    Plp,
    Rol,
    Ror,
    Rti,
    Rts,
    Sbc,
    Sec,
    Sed,
    Sei,
    Sta,
    Stx,
    Sty,
    Tax,
    Tay,
    Tsx,
    Txa,
    Txs,
    Tya,
}

impl Mn {
    /// The mnemonic in lower case, as ca65 writes it.
    pub fn name(self) -> &'static str {
        match self {
            Mn::Adc => "adc",
            Mn::And => "and",
            Mn::Asl => "asl",
            Mn::Bcc => "bcc",
            Mn::Bcs => "bcs",
            Mn::Beq => "beq",
            Mn::Bit => "bit",
            Mn::Bmi => "bmi",
            Mn::Bne => "bne",
            Mn::Bpl => "bpl",
            Mn::Brk => "brk",
            Mn::Bvc => "bvc",
            Mn::Bvs => "bvs",
            Mn::Clc => "clc",
            Mn::Cld => "cld",
            Mn::Cli => "cli",
            Mn::Clv => "clv",
            Mn::Cmp => "cmp",
            Mn::Cpx => "cpx",
            Mn::Cpy => "cpy",
            Mn::Dec => "dec",
            Mn::Dex => "dex",
            Mn::Dey => "dey",
            Mn::Eor => "eor",
            Mn::Inc => "inc",
            Mn::Inx => "inx",
            Mn::Iny => "iny",
            Mn::Jmp => "jmp",
            Mn::Jsr => "jsr",
            Mn::Lda => "lda",
            Mn::Ldx => "ldx",
            Mn::Ldy => "ldy",
            Mn::Lsr => "lsr",
            Mn::Nop => "nop",
            Mn::Ora => "ora",
            Mn::Pha => "pha",
            Mn::Php => "php",
            Mn::Pla => "pla",
            Mn::Plp => "plp",
            Mn::Rol => "rol",
            Mn::Ror => "ror",
            Mn::Rti => "rti",
            Mn::Rts => "rts",
            Mn::Sbc => "sbc",
            Mn::Sec => "sec",
            Mn::Sed => "sed",
            Mn::Sei => "sei",
            Mn::Sta => "sta",
            Mn::Stx => "stx",
            Mn::Sty => "sty",
            Mn::Tax => "tax",
            Mn::Tay => "tay",
            Mn::Tsx => "tsx",
            Mn::Txa => "txa",
            Mn::Txs => "txs",
            Mn::Tya => "tya",
        }
    }
}

/// How an instruction names its operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
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

impl Mode {
    /// Bytes of the instruction, opcode included.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(self) -> u16 {
        match self {
            Mode::Imp | Mode::Acc => 1,
            Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind => 3,
            _ => 2,
        }
    }
}

/// An official opcode's name and mode; `None` for the unofficial ones.
pub fn decode(op: u8) -> Option<(Mn, Mode)> {
    TABLE[op as usize]
}

const TABLE: [Option<(Mn, Mode)>; 256] = [
    Some((Mn::Brk, Mode::Imp)),
    Some((Mn::Ora, Mode::IndX)),
    None,
    None,
    None,
    Some((Mn::Ora, Mode::Zp)),
    Some((Mn::Asl, Mode::Zp)),
    None,
    Some((Mn::Php, Mode::Imp)),
    Some((Mn::Ora, Mode::Imm)),
    Some((Mn::Asl, Mode::Acc)),
    None,
    None,
    Some((Mn::Ora, Mode::Abs)),
    Some((Mn::Asl, Mode::Abs)),
    None,
    Some((Mn::Bpl, Mode::Rel)),
    Some((Mn::Ora, Mode::IndY)),
    None,
    None,
    None,
    Some((Mn::Ora, Mode::ZpX)),
    Some((Mn::Asl, Mode::ZpX)),
    None,
    Some((Mn::Clc, Mode::Imp)),
    Some((Mn::Ora, Mode::AbsY)),
    None,
    None,
    None,
    Some((Mn::Ora, Mode::AbsX)),
    Some((Mn::Asl, Mode::AbsX)),
    None,
    Some((Mn::Jsr, Mode::Abs)),
    Some((Mn::And, Mode::IndX)),
    None,
    None,
    Some((Mn::Bit, Mode::Zp)),
    Some((Mn::And, Mode::Zp)),
    Some((Mn::Rol, Mode::Zp)),
    None,
    Some((Mn::Plp, Mode::Imp)),
    Some((Mn::And, Mode::Imm)),
    Some((Mn::Rol, Mode::Acc)),
    None,
    Some((Mn::Bit, Mode::Abs)),
    Some((Mn::And, Mode::Abs)),
    Some((Mn::Rol, Mode::Abs)),
    None,
    Some((Mn::Bmi, Mode::Rel)),
    Some((Mn::And, Mode::IndY)),
    None,
    None,
    None,
    Some((Mn::And, Mode::ZpX)),
    Some((Mn::Rol, Mode::ZpX)),
    None,
    Some((Mn::Sec, Mode::Imp)),
    Some((Mn::And, Mode::AbsY)),
    None,
    None,
    None,
    Some((Mn::And, Mode::AbsX)),
    Some((Mn::Rol, Mode::AbsX)),
    None,
    Some((Mn::Rti, Mode::Imp)),
    Some((Mn::Eor, Mode::IndX)),
    None,
    None,
    None,
    Some((Mn::Eor, Mode::Zp)),
    Some((Mn::Lsr, Mode::Zp)),
    None,
    Some((Mn::Pha, Mode::Imp)),
    Some((Mn::Eor, Mode::Imm)),
    Some((Mn::Lsr, Mode::Acc)),
    None,
    Some((Mn::Jmp, Mode::Abs)),
    Some((Mn::Eor, Mode::Abs)),
    Some((Mn::Lsr, Mode::Abs)),
    None,
    Some((Mn::Bvc, Mode::Rel)),
    Some((Mn::Eor, Mode::IndY)),
    None,
    None,
    None,
    Some((Mn::Eor, Mode::ZpX)),
    Some((Mn::Lsr, Mode::ZpX)),
    None,
    Some((Mn::Cli, Mode::Imp)),
    Some((Mn::Eor, Mode::AbsY)),
    None,
    None,
    None,
    Some((Mn::Eor, Mode::AbsX)),
    Some((Mn::Lsr, Mode::AbsX)),
    None,
    Some((Mn::Rts, Mode::Imp)),
    Some((Mn::Adc, Mode::IndX)),
    None,
    None,
    None,
    Some((Mn::Adc, Mode::Zp)),
    Some((Mn::Ror, Mode::Zp)),
    None,
    Some((Mn::Pla, Mode::Imp)),
    Some((Mn::Adc, Mode::Imm)),
    Some((Mn::Ror, Mode::Acc)),
    None,
    Some((Mn::Jmp, Mode::Ind)),
    Some((Mn::Adc, Mode::Abs)),
    Some((Mn::Ror, Mode::Abs)),
    None,
    Some((Mn::Bvs, Mode::Rel)),
    Some((Mn::Adc, Mode::IndY)),
    None,
    None,
    None,
    Some((Mn::Adc, Mode::ZpX)),
    Some((Mn::Ror, Mode::ZpX)),
    None,
    Some((Mn::Sei, Mode::Imp)),
    Some((Mn::Adc, Mode::AbsY)),
    None,
    None,
    None,
    Some((Mn::Adc, Mode::AbsX)),
    Some((Mn::Ror, Mode::AbsX)),
    None,
    None,
    Some((Mn::Sta, Mode::IndX)),
    None,
    None,
    Some((Mn::Sty, Mode::Zp)),
    Some((Mn::Sta, Mode::Zp)),
    Some((Mn::Stx, Mode::Zp)),
    None,
    Some((Mn::Dey, Mode::Imp)),
    None,
    Some((Mn::Txa, Mode::Imp)),
    None,
    Some((Mn::Sty, Mode::Abs)),
    Some((Mn::Sta, Mode::Abs)),
    Some((Mn::Stx, Mode::Abs)),
    None,
    Some((Mn::Bcc, Mode::Rel)),
    Some((Mn::Sta, Mode::IndY)),
    None,
    None,
    Some((Mn::Sty, Mode::ZpX)),
    Some((Mn::Sta, Mode::ZpX)),
    Some((Mn::Stx, Mode::ZpY)),
    None,
    Some((Mn::Tya, Mode::Imp)),
    Some((Mn::Sta, Mode::AbsY)),
    Some((Mn::Txs, Mode::Imp)),
    None,
    None,
    Some((Mn::Sta, Mode::AbsX)),
    None,
    None,
    Some((Mn::Ldy, Mode::Imm)),
    Some((Mn::Lda, Mode::IndX)),
    Some((Mn::Ldx, Mode::Imm)),
    None,
    Some((Mn::Ldy, Mode::Zp)),
    Some((Mn::Lda, Mode::Zp)),
    Some((Mn::Ldx, Mode::Zp)),
    None,
    Some((Mn::Tay, Mode::Imp)),
    Some((Mn::Lda, Mode::Imm)),
    Some((Mn::Tax, Mode::Imp)),
    None,
    Some((Mn::Ldy, Mode::Abs)),
    Some((Mn::Lda, Mode::Abs)),
    Some((Mn::Ldx, Mode::Abs)),
    None,
    Some((Mn::Bcs, Mode::Rel)),
    Some((Mn::Lda, Mode::IndY)),
    None,
    None,
    Some((Mn::Ldy, Mode::ZpX)),
    Some((Mn::Lda, Mode::ZpX)),
    Some((Mn::Ldx, Mode::ZpY)),
    None,
    Some((Mn::Clv, Mode::Imp)),
    Some((Mn::Lda, Mode::AbsY)),
    Some((Mn::Tsx, Mode::Imp)),
    None,
    Some((Mn::Ldy, Mode::AbsX)),
    Some((Mn::Lda, Mode::AbsX)),
    Some((Mn::Ldx, Mode::AbsY)),
    None,
    Some((Mn::Cpy, Mode::Imm)),
    Some((Mn::Cmp, Mode::IndX)),
    None,
    None,
    Some((Mn::Cpy, Mode::Zp)),
    Some((Mn::Cmp, Mode::Zp)),
    Some((Mn::Dec, Mode::Zp)),
    None,
    Some((Mn::Iny, Mode::Imp)),
    Some((Mn::Cmp, Mode::Imm)),
    Some((Mn::Dex, Mode::Imp)),
    None,
    Some((Mn::Cpy, Mode::Abs)),
    Some((Mn::Cmp, Mode::Abs)),
    Some((Mn::Dec, Mode::Abs)),
    None,
    Some((Mn::Bne, Mode::Rel)),
    Some((Mn::Cmp, Mode::IndY)),
    None,
    None,
    None,
    Some((Mn::Cmp, Mode::ZpX)),
    Some((Mn::Dec, Mode::ZpX)),
    None,
    Some((Mn::Cld, Mode::Imp)),
    Some((Mn::Cmp, Mode::AbsY)),
    None,
    None,
    None,
    Some((Mn::Cmp, Mode::AbsX)),
    Some((Mn::Dec, Mode::AbsX)),
    None,
    Some((Mn::Cpx, Mode::Imm)),
    Some((Mn::Sbc, Mode::IndX)),
    None,
    None,
    Some((Mn::Cpx, Mode::Zp)),
    Some((Mn::Sbc, Mode::Zp)),
    Some((Mn::Inc, Mode::Zp)),
    None,
    Some((Mn::Inx, Mode::Imp)),
    Some((Mn::Sbc, Mode::Imm)),
    Some((Mn::Nop, Mode::Imp)),
    None,
    Some((Mn::Cpx, Mode::Abs)),
    Some((Mn::Sbc, Mode::Abs)),
    Some((Mn::Inc, Mode::Abs)),
    None,
    Some((Mn::Beq, Mode::Rel)),
    Some((Mn::Sbc, Mode::IndY)),
    None,
    None,
    None,
    Some((Mn::Sbc, Mode::ZpX)),
    Some((Mn::Inc, Mode::ZpX)),
    None,
    Some((Mn::Sed, Mode::Imp)),
    Some((Mn::Sbc, Mode::AbsY)),
    None,
    None,
    None,
    Some((Mn::Sbc, Mode::AbsX)),
    Some((Mn::Inc, Mode::AbsX)),
    None,
];

/// One decoded instruction the walk reached, at a banked address ([`Va`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insn {
    pub pc: Va,
    pub mn: Mn,
    pub mode: Mode,
    /// The operand: an immediate or zero-page byte, or an address. A
    /// branch's, jump's or call's target is a banked address: in the same
    /// bank when it is in the same switched region as the instruction,
    /// else as the CPU sees it at power-on (bank 0). Other addresses are
    /// CPU addresses.
    pub arg: Va,
}

impl Insn {
    pub fn next(&self) -> Va {
        va_add(self.pc, self.mode.len())
    }

    /// The instruction as ca65 writes it, with addresses in hex.
    pub fn text(&self) -> String {
        let m = self.mn.name();
        match self.mode {
            Mode::Imp => m.to_owned(),
            Mode::Acc => format!("{m} a"),
            Mode::Imm => format!("{m} #${:02X}", self.arg),
            Mode::Zp => format!("{m} ${:02X}", self.arg),
            Mode::ZpX => format!("{m} ${:02X},x", self.arg),
            Mode::ZpY => format!("{m} ${:02X},y", self.arg),
            Mode::Abs | Mode::Rel => format!("{m} ${:04X}", va_cpu(self.arg)),
            Mode::AbsX => format!("{m} ${:04X},x", self.arg),
            Mode::AbsY => format!("{m} ${:04X},y", self.arg),
            Mode::Ind => format!("{m} (${:04X})", self.arg),
            Mode::IndX => format!("{m} (${:02X},x)", self.arg),
            Mode::IndY => format!("{m} (${:02X}),y", self.arg),
        }
    }
}

/// A banked address: the CPU address in the low 16 bits, and above them 0
/// for what the CPU sees at power-on (the only view of an unbanked board),
/// or 1 + the bank of the board's switched region the address is in.
pub type Va = u32;

/// The CPU address of a banked address.
pub fn va_cpu(v: Va) -> u16 {
    v as u16
}

/// The bank part of a banked address: 0 at power-on, else 1 + the bank.
pub fn va_tag(v: Va) -> u32 {
    v >> 16
}

/// A banked address `d` bytes on, in the same bank.
pub fn va_add(v: Va, d: u16) -> Va {
    (v & 0xFFFF_0000) | u32::from((v as u16).wrapping_add(d))
}

/// The bank part of an address in a switched region whose bank isn't
/// known: it reads as 0.
pub const UNKNOWN_BANK: u32 = 0x7FFF;

/// A banked address as hex: `C123` at power-on, `03:8123` in bank 3.
pub fn va_hex(v: Va) -> String {
    match va_tag(v) {
        0 => format!("{:04X}", va_cpu(v)),
        UNKNOWN_BANK => format!("??:{:04X}", va_cpu(v)),
        t => format!("{:02X}:{:04X}", t - 1, va_cpu(v)),
    }
}

/// A banked address from [`va_hex`]'s text: `C123` or `03:8123`.
pub fn va_parse(s: &str) -> Option<Va> {
    match s.split_once(':') {
        None => u16::from_str_radix(s, 16).ok().map(u32::from),
        Some(("??", a)) => Some((UNKNOWN_BANK << 16) | u32::from(u16::from_str_radix(a, 16).ok()?)),
        Some((b, a)) => {
            let b = u32::from(u8::from_str_radix(b, 16).ok()?);
            Some(((b + 1) << 16) | u32::from(u16::from_str_radix(a, 16).ok()?))
        }
    }
}

/// How a board maps its PRG ROM into $8000 to $FFFF: the power-on image,
/// and the regions a bank register switches.
#[derive(Debug, Clone)]
pub struct Board {
    pub mapper: u16,
    pub prg: Vec<u8>,
    /// $8000 to $FFFF at power-on ([`cpu_image`]).
    pub image: Vec<u8>,
    /// Regions a run of the ROM showed switched beyond the mapper's usual
    /// layout, as (first address, size): MMC1 in 32 KB mode switches
    /// $C000 too. Empty unless runs say so.
    pub switched: Vec<(u16, u32)>,
}

impl Board {
    /// A board with nothing switched: the image is all there is.
    pub fn flat(image: &[u8]) -> Board {
        let mut img = vec![0u8; 0x8000];
        let n = image.len().min(0x8000);
        img[..n].copy_from_slice(&image[..n]);
        Board {
            mapper: 0,
            prg: img.clone(),
            image: img,
            switched: Vec::new(),
        }
    }

    /// A board from its iNES PRG and mapper.
    pub fn of(prg: &[u8], mapper: u16) -> Board {
        Board {
            mapper,
            prg: prg.to_vec(),
            image: cpu_image(prg),
            switched: Vec::new(),
        }
    }

    /// The switched region an address is in, as (first address, size):
    /// $8000 to $BFFF on UxROM (2) and MMC1 (1), $8000 and $A000 (8 KB
    /// each) on MMC3 (4), none on a board whose PRG fits the CPU's view.
    pub fn region(&self, a: u16) -> Option<(u16, u32)> {
        if !banked(&self.prg) {
            return None;
        }
        if let Some(&(s, size)) = self
            .switched
            .iter()
            .find(|(s, size)| (u32::from(*s)..u32::from(*s) + size).contains(&u32::from(a)))
        {
            return Some((s, size));
        }
        match self.mapper {
            1 | 2 if (0x8000..0xC000).contains(&a) => Some((0x8000, 0x4000)),
            4 if (0x8000..0xA000).contains(&a) => Some((0x8000, 0x2000)),
            4 if (0xA000..0xC000).contains(&a) => Some((0xA000, 0x2000)),
            _ => None,
        }
    }

    /// A byte at a banked address; RAM and registers read as 0.
    pub fn read(&self, v: Va) -> u8 {
        let a = va_cpu(v);
        if a < 0x8000 || va_tag(v) == UNKNOWN_BANK {
            return 0;
        }
        match (va_tag(v), self.region(a)) {
            (t, Some((s, size))) if t > 0 => {
                let o = (t as usize - 1) * size as usize + usize::from(a - s);
                self.prg[o % self.prg.len()]
            }
            _ => self.image[usize::from(a - 0x8000)],
        }
    }

    /// Where a banked address is in the PRG ROM: in its bank, or where the
    /// power-on view puts it (a mirrored 16 KB, the first 16 KB at $8000
    /// and the last at $C000 on a larger board).
    pub fn prg_offset(&self, v: Va) -> Option<usize> {
        let a = va_cpu(v);
        if a < 0x8000 || va_tag(v) == UNKNOWN_BANK || self.prg.is_empty() {
            return None;
        }
        let a = usize::from(a);
        Some(match (va_tag(v), self.region(a as u16)) {
            (t, Some((s, size))) if t > 0 => {
                ((t as usize - 1) * size as usize + (a - usize::from(s))) % self.prg.len()
            }
            _ if banked(&self.prg) && a >= 0xC000 => self.prg.len() - 0x4000 + (a - 0xC000),
            _ => (a - 0x8000) % self.prg.len(),
        })
    }

    /// Where a branch, jump or call at `pc` to CPU address `t` goes: the
    /// same bank when both are in one switched region, else power-on.
    pub fn target(&self, pc: Va, t: u16) -> Va {
        match self.region(t) {
            Some(r) if va_tag(pc) > 0 && self.region(va_cpu(pc)) == Some(r) => {
                (pc & 0xFFFF_0000) | u32::from(t)
            }
            _ => u32::from(t),
        }
    }
}

/// The CPU's view of PRG ROM, $8000 to $FFFF, as mapper 0 shows it: 16KB
/// mirrored, or 32KB. A larger PRG (a banked board) gives its first 16KB at
/// $8000 and its last at $C000, which is what UxROM and most boards with a
/// fixed last bank show at power-on; code in the other banks isn't in the
/// image ([`banked`] says when).
pub fn cpu_image(prg: &[u8]) -> Vec<u8> {
    let mut img = vec![0u8; 0x8000];
    if prg.is_empty() {
        return img;
    }
    if prg.len() <= 0x4000 {
        for i in 0..0x8000 {
            img[i] = prg[i % prg.len()];
        }
    } else if prg.len() == 0x8000 {
        img.copy_from_slice(prg);
    } else {
        img[..0x4000].copy_from_slice(&prg[..0x4000]);
        img[0x4000..].copy_from_slice(&prg[prg.len() - 0x4000..]);
    }
    img
}

/// Whether an image from this PRG leaves banks out.
pub fn banked(prg: &[u8]) -> bool {
    prg.len() > 0x8000
}

/// The routines of a banked code map: each a name, its first banked
/// address and its size in bytes.
#[derive(Debug, Clone, Default)]
pub struct BankedRoutines {
    pub names: Vec<String>,
    pub addr: Vec<Va>,
    pub size: Vec<u32>,
}

impl BankedRoutines {
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The routine an address is in.
    pub fn at(&self, v: Va) -> Option<usize> {
        let i = self.addr.partition_point(|&a| a <= v).checked_sub(1)?;
        (v < self.addr[i] + self.size[i]).then_some(i)
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
}

/// A routine's name: `sub_` and its address, with its bank when it has one
/// (`sub_C123`, `sub_03_8123`).
pub fn routine_name(v: Va) -> String {
    match va_tag(v) {
        0 => format!("sub_{:04X}", va_cpu(v)),
        t => format!("sub_{:02X}_{:04X}", t - 1, va_cpu(v)),
    }
}

/// [`Routines::from_rom_jumps`] on a banked board: the walk from the
/// vectors, the `extra` starts (calls a reader resolved into a bank) and
/// the jump tables' entries, at banked addresses ([`Board::target`]).
fn walk(board: &Board, extra: &[Va], jumps: &[(Va, Va)]) -> BankedRoutines {
    let read = |v: Va| board.read(v);
    let word = |v: Va| u16::from_le_bytes([read(v), read(va_add(v, 1))]);
    let mut starts: BTreeSet<Va> = [0xFFFC, 0xFFFA, 0xFFFE]
        .into_iter()
        .map(|v| u32::from(word(v)))
        .chain(extra.iter().copied())
        .chain(jumps.iter().map(|&(_, to)| to))
        .filter(|&a| va_cpu(a) >= 0x8000)
        .collect();
    type BodyV = (BTreeSet<Va>, Vec<(Va, Va)>, Vec<Va>, bool);
    let body = |start: Va| -> BodyV {
        let (mut seen, mut jumps, mut calls) = (BTreeSet::new(), Vec::new(), Vec::new());
        let mut returns = false;
        let mut todo = vec![start];
        while let Some(pc) = todo.pop() {
            if va_cpu(pc) < 0x8000 || !seen.insert(pc) {
                continue;
            }
            let op = read(pc);
            let next = va_add(pc, op_len(op));
            let w = board.target(pc, word(va_add(pc, 1)));
            match op {
                0x40 | 0x60 => returns = true,
                0x00 | 0x6C => {}
                0x4C => jumps.push((pc, w)),
                0x20 => {
                    calls.push(w);
                    // A routine that pulls its return address doesn't come
                    // back: what follows the call is its data.
                    if !pulls_return(&read, w) {
                        todo.push(next);
                    }
                }
                _ if op & 0x1F == 0x10 => {
                    let off = read(va_add(pc, 1)) as i8;
                    let t = va_cpu(next).wrapping_add(off as u16);
                    todo.push(board.target(pc, t));
                    todo.push(next);
                }
                _ => todo.push(next),
            }
        }
        // A jump out of the code reached is a tail call, which returns for it.
        returns |= jumps.iter().any(|(_, to)| !seen.contains(to));
        (seen, jumps, calls, returns)
    };
    let mut done: BTreeSet<Va> = BTreeSet::new();
    loop {
        let todo: Vec<Va> = starts.difference(&done).copied().collect();
        if todo.is_empty() {
            break;
        }
        for st in todo {
            done.insert(st);
            let (seen, jumps, calls, _) = body(st);
            starts.extend(calls.into_iter().filter(|&a| va_cpu(a) >= 0x8000));
            for (from, to) in jumps {
                if va_cpu(to) < 0x8000 {
                    continue;
                }
                let loops_back = va_tag(to) == va_tag(st) && to > st && to < from && !body(to).3;
                if !seen.contains(&to) || loops_back {
                    starts.insert(to);
                }
            }
        }
    }
    // A routine runs from its start to the next, within the code the walk
    // reached.
    let mut code: BTreeSet<Va> = BTreeSet::new();
    for &st in &starts {
        code.extend(body(st).0);
    }
    let mut out = BankedRoutines::default();
    let list: Vec<Va> = starts.iter().copied().collect();
    for (k, &a) in list.iter().enumerate() {
        let mut cpu = u32::from(va_cpu(a));
        let tag = a & 0xFFFF_0000;
        while cpu < 0x1_0000
            && (cpu == u32::from(va_cpu(a)) || !starts.contains(&(tag | cpu)))
            && code.contains(&(tag | cpu))
        {
            cpu += u32::from(op_len(read(tag | cpu)));
        }
        // An instruction that runs over the next routine's start ends at it.
        let mut size = cpu - u32::from(va_cpu(a));
        if let Some(&n) = list.get(k + 1)
            && va_tag(n) == va_tag(a)
        {
            size = size.min(n - a);
        }
        out.names.push(routine_name(a));
        out.addr.push(a);
        out.size.push(size);
    }
    out
}

/// The code map of one ROM: its routines (by a walk of the code from the
/// vectors, [`walk`]) and every instruction the walk reached, at banked
/// addresses. Shared by the static harvest ([`crate::harvest`]).
#[derive(Debug)]
pub struct RomCode {
    pub board: Board,
    pub routines: BankedRoutines,
    pub insns: BTreeMap<Va, Insn>,
    /// Each indirect jump's table entries (site -> entries), as given to
    /// [`RomCode::of_board`].
    pub jumps: BTreeMap<Va, Vec<Va>>,
}

impl RomCode {
    /// A byte at a banked address; RAM and registers read as 0.
    pub fn read(&self, a: Va) -> u8 {
        self.board.read(a)
    }

    pub fn word(&self, a: Va) -> u16 {
        u16::from_le_bytes([self.read(a), self.read(va_add(a, 1))])
    }

    /// The code map of a CPU image (from [`cpu_image`]) with nothing
    /// switched, cached by its hash for the life of the process.
    pub fn of(image: &[u8]) -> Arc<RomCode> {
        Self::of_board(&Board::flat(image), &[], &[])
    }

    /// [`RomCode::of`] with more routine starts (jump-table entries).
    pub fn of_with(image: &[u8], extra: &[u16]) -> Arc<RomCode> {
        let extra: Vec<Va> = extra.iter().map(|&a| u32::from(a)).collect();
        Self::of_board(&Board::flat(image), &extra, &[])
    }

    /// The code map of a board, with more routine starts (`extra`, calls a
    /// reader resolved into a bank) and the entries of jump tables a
    /// reader found, as `(site, entry)`. Cached by their hash.
    pub fn of_board(board: &Board, extra: &[Va], jumps: &[(Va, Va)]) -> Arc<RomCode> {
        static CACHE: OnceLock<Mutex<BTreeMap<[u8; 32], Arc<RomCode>>>> = OnceLock::new();
        let mut hasher = blake3::Hasher::new();
        hasher.update(&board.mapper.to_le_bytes());
        hasher.update(&board.prg);
        hasher.update(&board.image);
        for &(s, size) in &board.switched {
            hasher.update(&s.to_le_bytes());
            hasher.update(&size.to_le_bytes());
        }
        for &e in extra {
            hasher.update(&e.to_le_bytes());
        }
        hasher.update(b"jumps");
        for &(a, b) in jumps {
            hasher.update(&a.to_le_bytes());
            hasher.update(&b.to_le_bytes());
        }
        let key = *hasher.finalize().as_bytes();
        let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
        if let Some(c) = cache.lock().ok().and_then(|m| m.get(&key).cloned()) {
            return c;
        }
        let c = Arc::new(Self::build(board, extra, jumps));
        if let Ok(mut m) = cache.lock() {
            m.insert(key, c.clone());
        }
        c
    }

    fn build(board: &Board, extra: &[Va], jumps: &[(Va, Va)]) -> RomCode {
        let routines = walk(board, extra, jumps);
        let read = |v: Va| board.read(v);
        let mut table: BTreeMap<Va, Vec<Va>> = BTreeMap::new();
        for &(site, to) in jumps {
            if read(site) != 0x6C {
                continue;
            }
            let e = table.entry(site).or_default();
            if !e.contains(&to) {
                e.push(to);
            }
        }
        let mut insns = BTreeMap::new();
        for i in 0..routines.len() {
            let a0 = routines.addr[i];
            let tag = a0 & 0xFFFF_0000;
            let (mut cpu, end) = (
                u32::from(va_cpu(a0)),
                u32::from(va_cpu(a0)) + routines.size[i],
            );
            while cpu < end && cpu < 0x1_0000 {
                let p = tag | cpu;
                let op = read(p);
                let Some((mn, mode)) = decode(op) else {
                    cpu += 1;
                    continue;
                };
                let byte = |d: u16| read(va_add(p, d));
                let arg = match mode {
                    Mode::Imp | Mode::Acc => 0,
                    Mode::Rel => {
                        let off = byte(1) as i8;
                        board.target(p, (cpu as u16).wrapping_add(2).wrapping_add(off as u16))
                    }
                    Mode::Abs if matches!(mn, Mn::Jmp | Mn::Jsr) => {
                        board.target(p, u16::from_le_bytes([byte(1), byte(2)]))
                    }
                    m if m.len() == 3 => u32::from(u16::from_le_bytes([byte(1), byte(2)])),
                    _ => u32::from(byte(1)),
                };
                insns.insert(
                    p,
                    Insn {
                        pc: p,
                        mn,
                        mode,
                        arg,
                    },
                );
                cpu += u32::from(mode.len());
            }
        }
        RomCode {
            board: board.clone(),
            routines,
            insns,
            jumps: table,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routines_cover_their_addresses_up_to_the_next() {
        let r = Routines::new(vec![
            ("b".into(), 0x8010, 0),
            ("a".into(), 0x8000, 0),
            ("ram".into(), 0x0300, 4),
        ]);
        assert_eq!(r.len(), 2);
        assert_eq!(r.at(0x8000), Some(0));
        assert_eq!(r.at(0x800F), Some(0));
        assert_eq!(r.at(0x8010), Some(1));
        assert_eq!(r.at(0x0300), None);
    }

    #[test]
    fn the_graph_follows_calls_jumps_branches_and_jump_tables() {
        // 8000: JSR 8010; BNE +3 (to 8008); RTS. 8008: JMP 8020.
        // 8010: JSR 8030 then a table of 8000 and 8020.
        // 8020: RTS. 8030: RTS.
        let mut mem = vec![0xEAu8; 0x1_0000];
        mem[0x8000..0x8008].copy_from_slice(&[0x20, 0x10, 0x80, 0xD0, 0x03, 0x60, 0xEA, 0xEA]);
        mem[0x8008..0x800B].copy_from_slice(&[0x4C, 0x20, 0x80]);
        mem[0x8010..0x8017].copy_from_slice(&[0x20, 0x30, 0x80, 0x00, 0x80, 0x20, 0x80]);
        mem[0x8020] = 0x60;
        mem[0x8030] = 0x60;
        let r = Routines::new(vec![
            ("main".into(), 0x8000, 0),
            ("tail".into(), 0x8008, 0),
            ("engine_user".into(), 0x8010, 0),
            ("done".into(), 0x8020, 0),
            ("engine".into(), 0x8030, 0),
        ]);
        let g = graph(&|a| mem[a as usize], &r);
        assert!(g[0].contains(&2), "the call");
        assert!(g[0].contains(&1), "the branch into the next routine");
        assert!(g[1].contains(&3), "the jump");
        assert!(
            g[2].contains(&4) && g[2].contains(&0) && g[2].contains(&3),
            "the table"
        );
        let d = distances(&g, 3);
        assert_eq!(d[3], Some(0));
        assert_eq!(d[1], Some(1));
        assert_eq!(d[0], Some(2));
        assert_eq!(d[4], None);
    }

    #[test]
    fn opcode_lengths() {
        for (op, n) in [
            (0xA9, 2),
            (0xAD, 3),
            (0xBD, 3),
            (0xB9, 3),
            (0x20, 3),
            (0x60, 1),
            (0x0A, 1),
            (0xE8, 1),
            (0xD0, 2),
            (0xB1, 2),
            (0x6C, 3),
            (0x8A, 1),
            (0xAA, 1),
        ] {
            assert_eq!(op_len(op), n, "{op:02X}");
        }
    }

    #[test]
    fn the_walk_and_the_decoder_agree_on_every_official_length() {
        for op in 0..=255u8 {
            if let Some((_, mode)) = decode(op) {
                assert_eq!(op_len(op), mode.len(), "{op:02X}");
            }
        }
    }
}
