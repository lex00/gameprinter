//! A routine's cycles along one path, read statically from its code.
//!
//! [`crate::cost`] runs a routine on RAM. A model has no RAM: it has its
//! own state, and knows from it which way each of the routine's branches
//! goes (a counter as the frame starts, a body's state, a slot's flag). So
//! the model names the routine by its address and answers, at each
//! conditional branch, whether it is taken, and at an indexed read that
//! can cross a page, the index; [`walk`] reads the instructions from the
//! code the caller gives (the lifted program, the recipe's bytes from the
//! person's ROM) and adds their cycles: each instruction's own, a taken
//! branch's extra cycle and one more when it lands on another page, and an
//! indexed read's extra cycle when the address it forms crosses a page
//! (MOS Technology, MCS6500 Microcomputer Family Programming Manual, 1976,
//! appendix A). Nothing it adds is a number in the caller's code.

use crate::{Mode, decode};

/// What the walk asks the path at an instruction whose cost or next
/// address the code alone doesn't give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// A conditional branch at `pc`: is it taken?
    Branch { pc: u16 },
    /// An indexed read at `pc` (abs,X, abs,Y or (zp),Y) from `base`: the
    /// index, X or Y. For (zp),Y, `base` is the zero-page pointer's
    /// address and the path gives the pointer too ([`Path::pointer`]).
    Index { pc: u16, base: u16, indirect: bool },
    /// A `JSR` at `pc` to `target`.
    Call { pc: u16, target: u16 },
    /// A `JMP` at `pc` to `target`: a tail call when the path answers it
    /// as one ([`Call::Cost`], the target's cycles to the RTS that then
    /// returns from the routine that jumped).
    Jump { pc: u16, target: u16 },
}

/// How a call is costed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    /// Walk the callee's code too, to its return.
    Into,
    /// The callee's cycles from its first instruction to its return, its
    /// RTS included, as the path knows them (another walk's sum, or a
    /// measured fact). The JSR's own 6 are added by the walk.
    Cost(u64),
}

/// The path's answer to an [`Ask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// To a branch: taken or not.
    Taken(bool),
    /// To an indexed read: the index register, and for (zp),Y the pointer
    /// with it.
    Index(u8),
    Pointer(u16, u8),
    /// To a call.
    Call(Call),
    /// The path doesn't say. A branch then stops the walk
    /// ([`WalkError::Unknown`]); an indexed read whose base is at a page's
    /// start can't cross one and costs its own cycles, any other stops the
    /// walk; a call is walked into.
    Unknown,
}

/// The model's side of a walk: the answers from its own state. Any
/// `FnMut(Ask) -> Answer` is one.
pub trait Path {
    fn ask(&mut self, a: Ask) -> Answer;
}

impl<F: FnMut(Ask) -> Answer> Path for F {
    fn ask(&mut self, a: Ask) -> Answer {
        self(a)
    }
}

/// Why a walk stopped without a cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkError {
    /// An opcode this has no cost for (unofficial, BRK, or data read as
    /// code: code the lift doesn't have reads as 0, BRK), at its address.
    Opcode(u16, u8),
    /// A `JMP ($nnnn)`, whose target is data, at its address.
    Indirect(u16),
    /// More than the limit's instructions without returning.
    Limit,
    /// A branch, or an indexed read that may cross a page, the path
    /// gave no answer for, at its address.
    Unknown(u16),
}

/// The cycles from `entry` to the RTS that returns from it (or the RTI),
/// that last instruction included and the caller's JSR not. `code` gives
/// the program's byte at a CPU address. A routine that pulls its caller's
/// return address (two PLAs with nothing pushed) returns from its caller
/// at its RTS, as the 6502 does. Stops after `limit` instructions.
pub fn walk(
    code: &dyn Fn(u16) -> u8,
    entry: u16,
    path: &mut dyn Path,
    limit: usize,
) -> Result<u64, WalkError> {
    // Each call's pushes not yet pulled and where it returns to (none for
    // the walk's own entry); a pull with none pulls a byte of the return
    // address.
    let mut frames: Vec<(i32, Option<u16>)> = vec![(0, None)];
    let mut pc = entry;
    let mut cycles = 0u64;
    let word = |a: u16| u16::from(code(a)) | u16::from(code(a.wrapping_add(1))) << 8;
    for _ in 0..limit {
        let op = code(pc);
        let Some((name, mode, base, cross)) = decode(op) else {
            return Err(WalkError::Opcode(pc, op));
        };
        cycles += u64::from(base);
        let len: u16 = match mode {
            Mode::Imp | Mode::Acc => 1,
            Mode::Imm | Mode::Zp | Mode::ZpX | Mode::ZpY | Mode::IndX | Mode::IndY | Mode::Rel => 2,
            Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind => 3,
        };
        let next = pc.wrapping_add(len);
        if cross && matches!(mode, Mode::AbsX | Mode::AbsY | Mode::IndY) {
            let operand = if mode == Mode::IndY {
                u16::from(code(pc.wrapping_add(1)))
            } else {
                word(pc.wrapping_add(1))
            };
            let indirect = mode == Mode::IndY;
            let (b, i) = match path.ask(Ask::Index {
                pc,
                base: operand,
                indirect,
            }) {
                Answer::Index(i) if mode != Mode::IndY => (operand, i),
                Answer::Pointer(p, i) => (p, i),
                Answer::Unknown if mode != Mode::IndY && operand & 0xFF == 0 => (operand, 0),
                _ => return Err(WalkError::Unknown(pc)),
            };
            let a = b.wrapping_add(u16::from(i));
            cycles += u64::from(a & 0xFF00 != b & 0xFF00);
        }
        match name {
            "BPL" | "BMI" | "BVC" | "BVS" | "BCC" | "BCS" | "BNE" | "BEQ" => {
                let taken = match path.ask(Ask::Branch { pc }) {
                    Answer::Taken(t) => t,
                    _ => return Err(WalkError::Unknown(pc)),
                };
                if taken {
                    let d = code(pc.wrapping_add(1)) as i8;
                    let to = next.wrapping_add(d as u16);
                    cycles += 1 + u64::from(to & 0xFF00 != next & 0xFF00);
                    pc = to;
                } else {
                    pc = next;
                }
            }
            "JMP" if mode == Mode::Abs => {
                let target = word(pc.wrapping_add(1));
                match path.ask(Ask::Jump { pc, target }) {
                    Answer::Call(Call::Cost(c)) => {
                        cycles += c;
                        // The target's RTS returns from this frame.
                        let (_, back) = frames.pop().unwrap_or((0, None));
                        match back {
                            Some(r) if !frames.is_empty() => pc = r,
                            _ => return Ok(cycles),
                        }
                    }
                    _ => pc = target,
                }
            }
            "JMP" => return Err(WalkError::Indirect(pc)),
            "JSR" => {
                let target = word(pc.wrapping_add(1));
                let how = match path.ask(Ask::Call { pc, target }) {
                    Answer::Call(c) => c,
                    _ => Call::Into,
                };
                match how {
                    Call::Into => {
                        frames.push((0, Some(next)));
                        pc = target;
                    }
                    Call::Cost(c) => {
                        cycles += c;
                        pc = next;
                    }
                }
            }
            "PHA" | "PHP" => {
                if let Some(f) = frames.last_mut() {
                    f.0 += 1;
                }
                pc = next;
            }
            "PLA" | "PLP" => {
                if let Some(f) = frames.last_mut() {
                    f.0 -= 1;
                }
                pc = next;
            }
            "RTS" | "RTI" => {
                // Each two bytes pulled past the frame's own pushes are a
                // caller's return address: return from that one too.
                let (own, mut back) = frames.pop().unwrap_or((0, None));
                for _ in 0..(-own).max(0) / 2 {
                    back = frames.pop().and_then(|f| f.1);
                }
                match back {
                    Some(r) if name == "RTS" && !frames.is_empty() => pc = r,
                    _ => return Ok(cycles),
                }
            }
            _ => pc = next,
        }
    }
    Err(WalkError::Limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Regs, Slice, path};

    /// Hand-made code at $8000 (not any game's), NOPs past it.
    fn prog(code: &[u8]) -> Vec<u8> {
        let mut prg = vec![0xEA; 0x4000];
        prg[..code.len()].copy_from_slice(code);
        prg
    }

    /// A path that answers from a run's instruction trace: each branch
    /// taken when the next address run isn't the one after it.
    struct Traced {
        pcs: Vec<u16>,
        at: usize,
        x: u8,
    }

    impl Path for Traced {
        fn ask(&mut self, a: Ask) -> Answer {
            match a {
                Ask::Branch { pc } => {
                    let i = (self.at..self.pcs.len())
                        .find(|&i| self.pcs[i] == pc)
                        .unwrap();
                    self.at = i + 1;
                    Answer::Taken(self.pcs.get(i + 1) != Some(&pc.wrapping_add(2)))
                }
                Ask::Index { .. } => Answer::Index(self.x),
                Ask::Call { .. } | Ask::Jump { .. } => Answer::Unknown,
            }
        }
    }

    fn both(code: &[u8], ram: &[u8; 0x800], x: u8) -> (u64, u64) {
        let prg = prog(code);
        let s = Slice {
            base: 0x8000,
            prg: prg.clone(),
        };
        let regs = Regs {
            x,
            s: 0xFD,
            ..Regs::default()
        };
        let (run, pcs) = path(&s, 0x8000, ram, regs, 100_000).unwrap();
        let read = |a: u16| prg[usize::from(a.wrapping_sub(0x8000)) % prg.len()];
        let mut t = Traced { pcs, at: 0, x };
        // `path` counts the caller's JSR; the walk doesn't.
        (run - 6, walk(&read, 0x8000, &mut t, 10_000).unwrap())
    }

    #[test]
    fn a_walk_costs_what_a_run_on_the_same_path_does() {
        // LDX #3; DEX; BNE -3; RTS: a loop's branches.
        let (run, w) = both(&[0xA2, 0x03, 0xCA, 0xD0, 0xFD, 0x60], &[0; 0x800], 0);
        assert_eq!((run, w), (2 + 6 + 6 + 2 + 6, 2 + 6 + 6 + 2 + 6));
        // LDA $10; BEQ +2; INC $11; JSR $8010; RTS, and at $8010 INC $11;
        // RTS: a branch either way and a call walked into.
        let mut code = vec![0xA5, 0x10, 0xF0, 0x02, 0xE6, 0x11, 0x20, 0x10, 0x80, 0x60];
        code.resize(0x10, 0xEA);
        code.extend([0xE6, 0x11, 0x60]);
        for v in [0, 1] {
            let mut ram = [0u8; 0x800];
            ram[0x10] = v;
            let (run, w) = both(&code, &ram, 0);
            assert_eq!(run, w, "with $10 = {v}");
        }
    }

    #[test]
    fn a_walk_adds_a_page_crossing_and_a_far_branch() {
        // LDA $00FF,X: X = 1 crosses a page.
        let (run, w) = both(&[0xBD, 0xFF, 0x00, 0x60], &[0; 0x800], 1);
        assert_eq!((run, w), (5 + 6, 5 + 6));
        let (run, w) = both(&[0xBD, 0xFF, 0x00, 0x60], &[0; 0x800], 0);
        assert_eq!((run, w), (4 + 6, 4 + 6));
        // LDA #0, then a BEQ at $80FC taken to $8100 on the next page
        // from $80FE: 4 cycles.
        let mut code = vec![0xA9, 0x00, 0x4C, 0xFC, 0x80];
        code.resize(0xFC, 0xEA);
        code.extend([0xF0, 0x02, 0xEA, 0xEA, 0x60]);
        let (run, w) = both(&code, &[0; 0x800], 0);
        assert_eq!(run, w);
        assert_eq!(w, 2 + 3 + 4 + 6);
    }

    #[test]
    fn a_routine_that_pulls_its_return_address_returns_from_its_caller() {
        // $8000: JSR $8010; LDA #0; RTS. $8010: JSR $8020; RTS (not run).
        // $8020: PLA; PLA; RTS, which returns to $8003.
        let mut code = vec![0x20, 0x10, 0x80, 0xA9, 0x00, 0x60];
        code.resize(0x10, 0xEA);
        code.extend([0x20, 0x20, 0x80, 0x60]);
        code.resize(0x20, 0xEA);
        code.extend([0x68, 0x68, 0x60]);
        let (run, w) = both(&code, &[0; 0x800], 0);
        assert_eq!(run, w);
        assert_eq!(w, 6 + 6 + 4 + 4 + 6 + 2 + 6);
    }

    #[test]
    fn a_call_can_be_costed_by_the_path() {
        let mut fixed = |a: Ask| match a {
            Ask::Call { .. } => Answer::Call(Call::Cost(100)),
            _ => Answer::Unknown,
        };
        let prg = prog(&[0x20, 0x00, 0x90, 0x60]);
        let read = |a: u16| prg[usize::from(a.wrapping_sub(0x8000)) % prg.len()];
        assert_eq!(walk(&read, 0x8000, &mut fixed, 100), Ok(6 + 100 + 6));
        // Code the lift doesn't have reads as BRK.
        let none = |_: u16| 0u8;
        assert_eq!(
            walk(&none, 0x8000, &mut fixed, 100),
            Err(WalkError::Opcode(0x8000, 0))
        );
        // A branch the path doesn't decide stops the walk.
        let prg = prog(&[0xF0, 0x00, 0x60]);
        let read = |a: u16| prg[usize::from(a.wrapping_sub(0x8000)) % prg.len()];
        assert_eq!(
            walk(&read, 0x8000, &mut fixed, 100),
            Err(WalkError::Unknown(0x8000))
        );
    }
}
