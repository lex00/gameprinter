//! A CP/M harness on this crate's Z80, for the Z80 instruction suites
//! (ZEXDOC and ZEXALL, `gp testroms`, docs/test-roms.md). Those are CP/M
//! programs: they are loaded at 0100, print their result through the
//! BDOS at 0005 and end by jumping to 0000, the warm boot.
//!
//! The machine is 64 KB of RAM and the Z80, nothing else. 0005 holds a
//! `RET`, and the harness carries out the BDOS call just before that
//! `RET` runs: call 2 prints the character in E, call 9 the string at DE up
//! to a `$`, and call 0 ends the run as a jump to 0000 does. Any other call
//! ends the run as unsupported, so a program never goes on with a call
//! the harness didn't make. 0006 holds the top of memory a program may use
//! (FE00), which ZEXDOC and ZEXALL load into SP. The program starts with SP
//! at FDFE and 0000 on the stack, so a `RET` from the program ends it too.
//!
//! No interrupt is ever raised, and the I/O ports read FF.

use crate::z80::{Z80, Z80Bus};

/// Where a CP/M program is loaded and starts.
pub const LOAD: u16 = 0x0100;
/// The BDOS entry.
const BDOS: u16 = 0x0005;
/// The top of the memory a program may use, as 0006 gives it.
const TOP: u16 = 0xfe00;
/// The cycles run between checks for the end.
const SLICE: i32 = 1_000_000;

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The program jumped to 0000 (or called BDOS 0): CP/M's warm boot.
    WarmBoot,
    /// The CPU halted, which with no interrupts is for good. `pc` as the
    /// Z80 has it then.
    Halted { pc: u16 },
    /// A BDOS call the harness doesn't have.
    Unsupported { call: u8, pc: u16 },
    /// The cycle limit ran out first.
    OutOfCycles,
}

/// What a CP/M program did.
#[derive(Debug, Clone)]
pub struct Run {
    /// Everything it printed, as bytes (CP/M's output is 8-bit; ZEXDOC
    /// ends its lines with CR LF).
    pub output: Vec<u8>,
    pub ended: Ended,
    /// The Z80 cycles it ran.
    pub cycles: u64,
}

impl Run {
    /// The printed text, with any byte past ASCII as U+FFFD.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

/// 64 KB of RAM and the BDOS.
struct Machine {
    mem: Vec<u8>,
    output: Vec<u8>,
    ended: Option<Ended>,
    /// The cycles left in the slice when the run ended.
    left: Option<i32>,
}

impl Machine {
    fn bdos(&mut self, cpu: &Z80) -> Option<Ended> {
        let call = cpu.bc as u8;
        match call {
            0 => return Some(Ended::WarmBoot),
            2 => self.output.push(cpu.de as u8),
            9 => {
                let mut at = cpu.de;
                // A string with no `$` in all of memory stops where it
                // wraps back to its start.
                for _ in 0..0x10000 {
                    let b = self.mem[usize::from(at)];
                    if b == b'$' {
                        break;
                    }
                    self.output.push(b);
                    at = at.wrapping_add(1);
                }
            }
            _ => {
                return Some(Ended::Unsupported { call, pc: cpu.pc });
            }
        }
        None
    }
}

impl Z80Bus for Machine {
    fn read(&mut self, addr: u16, _: &mut i32) -> u8 {
        self.mem[usize::from(addr)]
    }
    fn write(&mut self, addr: u16, value: u8, _: &mut i32) {
        self.mem[usize::from(addr)] = value;
    }
    fn fetch(&mut self, addr: u16) -> u8 {
        self.mem[usize::from(addr)]
    }
    fn io_read(&mut self, _: u16, _: &mut i32) -> u8 {
        0xff
    }
    fn io_write(&mut self, _: u16, _: u8, _: &mut i32) {}
    fn irq_ack(&mut self, _: &mut i32) -> i32 {
        0xff
    }
    fn reti(&mut self, _: &mut i32) {}
    fn instruction(&mut self, cpu: &mut Z80) {
        if self.ended.is_some() {
            cpu.icount = 0;
            return;
        }
        let ended = match cpu.pc {
            0x0000 => Some(Ended::WarmBoot),
            BDOS => self.bdos(cpu),
            _ => None,
        };
        if ended.is_some() {
            self.ended = ended;
            // Ends the slice, keeping what was left of it for the count:
            // the fetch that follows is the last step run.
            self.left = Some(cpu.icount);
            cpu.icount = 0;
        }
    }
}

/// Runs the CP/M program `program` (a .COM file's bytes) for at most
/// `max_cycles` Z80 cycles. Fails only for a program too big to load
/// below the top of memory.
pub fn run(program: &[u8], max_cycles: u64) -> Result<Run, String> {
    let start = usize::from(LOAD);
    if program.len() > usize::from(TOP) - start - 2 {
        return Err(format!(
            "a CP/M program of {} bytes doesn't fit between {LOAD:04X} and {TOP:04X}",
            program.len()
        ));
    }
    let mut mem = vec![0u8; 0x10000];
    mem[start..start + program.len()].copy_from_slice(program);
    mem[0x0000] = 0x76; // HALT, never reached: the jump to 0000 ends the run
    mem[usize::from(BDOS)] = 0xc9; // RET
    mem[0x0006..0x0008].copy_from_slice(&TOP.to_le_bytes());
    let mut bus = Machine {
        mem,
        output: Vec::new(),
        ended: None,
        left: None,
    };
    let mut cpu = Z80::new();
    cpu.reset();
    cpu.pc = LOAD;
    cpu.wz = LOAD;
    // 0000 is on the stack (the RAM is zeros), for a program that returns.
    cpu.sp = TOP - 2;
    let mut cycles = 0u64;
    let ended = loop {
        if let Some(e) = bus.ended.take() {
            break e;
        }
        if cpu.halt != 0 {
            break Ended::Halted { pc: cpu.pc };
        }
        if cycles >= max_cycles {
            break Ended::OutOfCycles;
        }
        let slice = SLICE.min(i32::try_from(max_cycles - cycles).unwrap_or(SLICE));
        cpu.icount = slice;
        cpu.execute_run(&mut bus);
        let left = bus.left.take().unwrap_or(cpu.icount);
        cycles += u64::try_from(i64::from(slice) - i64::from(left)).unwrap_or(0);
    };
    Ok(Run {
        output: bus.output,
        ended,
        cycles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up CP/M program: prints `H` through BDOS call 2 and `i there`
    /// through call 9, then jumps to 0000.
    const HELLO: &[u8] = &[
        0x0e, 0x02, // 0100: ld c,2
        0x1e, b'H', // 0102: ld e,'H'
        0xcd, 0x05, 0x00, // 0104: call 0005
        0x0e, 0x09, // 0107: ld c,9
        0x11, 0x12, 0x01, // 0109: ld de,0112
        0xcd, 0x05, 0x00, // 010C: call 0005
        0xc3, 0x00, 0x00, // 010F: jp 0000
        b'i', b' ', b't', b'h', b'e', b'r', b'e', b'$', // 0112
    ];

    #[test]
    fn a_program_prints_through_bdos_2_and_9_and_ends_at_0000() {
        let r = run(HELLO, 10_000).unwrap();
        assert_eq!(r.ended, Ended::WarmBoot);
        assert_eq!(r.text(), "Hi there");
        // Two rounds of LD C, LD E (or DE), CALL and the RET at 0005, then
        // the JP: at least 7+7+17+10 and 7+10+17+10 and 10 cycles.
        assert!(r.cycles >= 95 && r.cycles < 200, "{}", r.cycles);
    }

    #[test]
    fn a_program_that_reads_the_top_of_memory_and_returns_ends_too() {
        // Prints the top of memory's high byte through call 2, then RETs
        // to the 0000 on the stack.
        let program = [
            0x2a, 0x06, 0x00, // ld hl,(0006)
            0x5c, // ld e,h
            0x0e, 0x02, // ld c,2
            0xcd, 0x05, 0x00, // call 0005
            0xc9, // ret
        ];
        let r = run(&program, 10_000).unwrap();
        assert_eq!(r.ended, Ended::WarmBoot);
        assert_eq!(r.output, vec![0xfe]);
    }

    #[test]
    fn halts_unsupported_calls_and_endless_loops_end_the_run() {
        let r = run(&[0x00, 0x76], 1000).unwrap(); // nop; halt
        assert!(matches!(r.ended, Ended::Halted { .. }), "{:?}", r.ended);
        let r = run(&[0x0e, 0x01, 0xcd, 0x05, 0x00], 1000).unwrap(); // BDOS 1
        assert_eq!(
            r.ended,
            Ended::Unsupported {
                call: 1,
                pc: 0x0005
            }
        );
        let r = run(&[0x18, 0xfe], 5000).unwrap(); // jr $
        assert_eq!(r.ended, Ended::OutOfCycles);
        assert!(r.cycles >= 5000);
        assert!(run(&vec![0; 0xff00], 10).is_err());
    }
}
