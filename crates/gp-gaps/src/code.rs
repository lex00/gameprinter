//! Reading a routine's code statically, for the cross-check of what a
//! recording exercised against what the code handles: the branch and call
//! targets in a routine's bytes that never ran, and the constants it
//! compares with (docs/gaps.md, "Cross-check with the code").

use serde::Serialize;

use crate::{RomLabel, Run};

/// The length of the 6502 instruction with this opcode, official and
/// unofficial alike, by the opcode's addressing mode bits.
pub fn length(op: u8) -> u32 {
    let (a, b, c) = (op >> 5, (op >> 2) & 7, op & 3);
    match (c, b) {
        (0, 0) => match a {
            1 => 3,     // JSR abs
            2 | 3 => 1, // RTI, RTS
            0 => 1,     // BRK (its padding byte isn't an instruction)
            _ => 2,     // NOP #, LDY #, CPY #, CPX #
        },
        (0, 2) | (0, 6) | (2, 6) | (2, 2) => 1,
        (2, 0) => 2,
        (2, 4) => 1,
        (_, 1) | (_, 5) | (_, 4) => 2,
        (_, 0) | (_, 2) => 2,
        (_, 3) | (_, 6) | (_, 7) => 3,
        _ => 1,
    }
}

fn is_branch(op: u8) -> bool {
    op & 0x1F == 0x10
}

/// What a routine's bytes say.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Static {
    pub routine: String,
    /// Branch, jump and call targets in or from it that no frame ran, by
    /// name (a label, or `Routine+n`).
    pub unexercised: Vec<String>,
    /// Targets that ran.
    pub exercised: usize,
    /// Immediate values it compares a register with (CMP, CPX, CPY #).
    pub compares: Vec<u8>,
}

/// Reads routine `l`'s code from the run's PRG ROM by control flow from
/// its entry ([`body`]), or the label's first 512 bytes when the walk finds
/// nothing. A target is followed to a PRG offset when it's in the same 8KB
/// window as the routine, or anywhere on a board with 32KB of PRG or less,
/// which maps all of it at once.
pub fn scan(run: &Run, l: &RomLabel) -> Option<Static> {
    if run.prg.is_empty() || run.executed.is_empty() {
        return None;
    }
    let prg = &run.prg;
    let fixed = prg.len() <= 0x8000;
    let to_prg = |t: u16| -> Option<u32> {
        if t < 0x8000 {
            return None;
        }
        if !fixed && (t & !0x1FFF) != (l.addr & !0x1FFF) {
            return None;
        }
        let o = i64::from(l.prg) + i64::from(t) - i64::from(l.addr);
        (0..prg.len() as i64).contains(&o).then_some(o as u32)
    };
    let mut out = Static {
        routine: l.name.clone(),
        ..Static::default()
    };
    // The routine's code by control flow from its entry, across the labels
    // a disassembly puts at every branch target; the label's own extent
    // when that finds less (data the walk can't enter).
    let entries: std::collections::BTreeSet<u32> = run.calls.iter().map(|c| c.target).collect();
    let mut at: Vec<u32> = body(run, l, &entries).into_iter().collect();
    if at.len() <= 1 {
        let end = l.prg + l.size.clamp(1, 512);
        let mut o = l.prg;
        at.clear();
        while o < end && (o as usize) < prg.len() {
            at.push(o);
            o += length(prg[o as usize]);
        }
    }
    let mut targets = Vec::new();
    for o in at {
        let op = prg[o as usize];
        let byte = |i: u32| prg.get((o + i) as usize).copied().unwrap_or(0);
        let pc = l.addr.wrapping_add(o.wrapping_sub(l.prg) as u16);
        if is_branch(op) {
            let t = pc.wrapping_add(2).wrapping_add(byte(1) as i8 as u16);
            targets.extend(to_prg(t));
        } else if op == 0x20 || op == 0x4C {
            let t = u16::from_le_bytes([byte(1), byte(2)]);
            targets.extend(to_prg(t));
        } else if matches!(op, 0xC9 | 0xE0 | 0xC0) && !out.compares.contains(&byte(1)) {
            out.compares.push(byte(1));
        }
    }
    targets.sort_unstable();
    targets.dedup();
    for t in targets {
        if run.ran(t) {
            out.exercised += 1;
        } else {
            out.unexercised.push(run.known.routine_name(t));
        }
    }
    out.compares.sort_unstable();
    Some(out)
}

/// The most bytes of code [`body`] reads from one entry.
pub const BODY_MAX: usize = 4096;

/// Whether the routine at PRG offset `o` pulls its caller's return address
/// within its first instructions (two `PLA` before any return): the bytes
/// after a `JSR` to it are a jump table, not code.
fn pulls_return(prg: &[u8], o: u32) -> bool {
    let mut at = o as usize;
    let mut pulls = 0;
    for _ in 0..8 {
        let Some(&op) = prg.get(at) else {
            return false;
        };
        match op {
            0x68 => {
                pulls += 1;
                if pulls == 2 {
                    return true;
                }
            }
            0x60 | 0x40 | 0x4C | 0x6C => return false,
            _ => {}
        }
        at += length(op) as usize;
    }
    false
}

/// The instruction starts of routine `l`'s code, by control flow from its
/// entry rather than by label: every branch and the fall-through after it,
/// a `JMP` to code in the routine's 8KB window (or anywhere on a board
/// with 32KB of PRG or less) that isn't another routine's entry (`entries`,
/// the `JSR` targets), and the instruction after a `JSR` unless the callee
/// pulls its return address (a jump table follows). It stops at a return,
/// an indirect jump and `BRK`, and reads at most [`BODY_MAX`] bytes. A
/// disassembly with a label at every branch target (dense labels) still
/// gets one body per routine.
pub fn body(
    run: &Run,
    l: &RomLabel,
    entries: &std::collections::BTreeSet<u32>,
) -> std::collections::BTreeSet<u32> {
    let mut out = std::collections::BTreeSet::new();
    let prg = &run.prg;
    if prg.is_empty() {
        out.insert(l.prg);
        return out;
    }
    let fixed = prg.len() <= 0x8000;
    let to_prg = |t: u16| -> Option<u32> {
        if t < 0x8000 {
            return None;
        }
        if !fixed && (t & !0x1FFF) != (l.addr & !0x1FFF) {
            return None;
        }
        let o = i64::from(l.prg) + i64::from(t) - i64::from(l.addr);
        (0..prg.len() as i64).contains(&o).then_some(o as u32)
    };
    let mut todo = vec![l.prg];
    let mut bytes = 0usize;
    while let Some(start) = todo.pop() {
        let mut o = start;
        loop {
            if (o as usize) >= prg.len() || bytes >= BODY_MAX || !out.insert(o) {
                break;
            }
            let op = prg[o as usize];
            let n = length(op);
            bytes += n as usize;
            let byte = |i: u32| prg.get((o + i) as usize).copied().unwrap_or(0);
            let pc = l.addr.wrapping_add(o.wrapping_sub(l.prg) as u16);
            match op {
                0x60 | 0x40 | 0x00 | 0x6C => break,
                0x4C => {
                    if let Some(t) = to_prg(u16::from_le_bytes([byte(1), byte(2)]))
                        && !entries.contains(&t)
                    {
                        todo.push(t);
                    }
                    break;
                }
                0x20 => {
                    let callee = to_prg(u16::from_le_bytes([byte(1), byte(2)])).or_else(|| {
                        // A call into the fixed bank from a switched one.
                        let t = u16::from_le_bytes([byte(1), byte(2)]);
                        (t >= 0xC000 && prg.len() >= 0x4000)
                            .then(|| (prg.len() - 0x10000 + usize::from(t)) as u32)
                    });
                    if callee.is_some_and(|c| pulls_return(prg, c)) {
                        break;
                    }
                }
                _ if is_branch(op) => {
                    let t = pc.wrapping_add(2).wrapping_add(byte(1) as i8 as u16);
                    if let Some(t) = to_prg(t) {
                        todo.push(t);
                    }
                }
                _ => {}
            }
            o += n;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with(code: &[u8]) -> Run {
        let mut prg = vec![0xEA; 0x8000];
        prg[..code.len()].copy_from_slice(code);
        Run {
            prg,
            ..Run::default()
        }
    }

    #[test]
    fn a_body_follows_branches_across_labels_and_stops_at_returns() {
        // 0: LDA #1; BEQ +3 (to 7); JMP 10; (5) RTS; .. (7) INX; RTS; (10) DEY; RTS
        let code = [
            0xA9, 0x01, 0xF0, 0x03, 0x4C, 0x0A, 0x80, 0xE8, 0x60, 0xEA, 0x88, 0x60,
        ];
        let r = run_with(&code);
        let l = RomLabel {
            name: "R".into(),
            prg: 0,
            addr: 0x8000,
            size: 1,
        };
        let b = body(&r, &l, &Default::default());
        assert_eq!(
            b.into_iter().collect::<Vec<_>>(),
            vec![0, 2, 4, 7, 8, 10, 11]
        );
        // A jump to another routine's entry is a tail call, not this body.
        let b = body(&r, &l, &[10u32].into_iter().collect());
        assert!(!b.contains(&10));
    }

    #[test]
    fn a_body_stops_after_a_call_to_a_jump_table_routine() {
        // 0: JSR $8010; (data follows) ... 16: PLA; PLA; RTS
        let mut code = vec![0x20, 0x10, 0x80, 0x34, 0x12, 0x56, 0x78];
        code.resize(16, 0xEA);
        code.extend([0x68, 0x68, 0x60]);
        let r = run_with(&code);
        let l = RomLabel {
            name: "R".into(),
            prg: 0,
            addr: 0x8000,
            size: 1,
        };
        assert_eq!(body(&r, &l, &Default::default()).len(), 1);
    }

    #[test]
    fn lengths_of_common_opcodes() {
        for (op, n) in [
            (0xA9, 2), // LDA #
            (0xAD, 3), // LDA abs
            (0x85, 2), // STA zp
            (0x9D, 3), // STA abs,x
            (0x91, 2), // STA (zp),y
            (0x20, 3), // JSR
            (0x60, 1), // RTS
            (0x4C, 3), // JMP
            (0x6C, 3), // JMP ()
            (0xD0, 2), // BNE
            (0xE8, 1), // INX
            (0x0A, 1), // ASL A
            (0xA2, 2), // LDX #
            (0xBE, 3), // LDX abs,y
            (0xC0, 2), // CPY #
            (0xEA, 1), // NOP
            (0x18, 1), // CLC
            (0x98, 1), // TYA
        ] {
            assert_eq!(length(op), n, "opcode {op:#04x}");
        }
    }
}
