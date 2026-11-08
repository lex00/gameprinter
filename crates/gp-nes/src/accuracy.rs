//! Runs the standard NES test ROMs headless and reads their results, for
//! the accuracy baseline (docs/test-roms.md).
//!
//! Three ways a test ROM reports:
//!
//! - nestest, in its automation mode, runs from $C000 with no PPU and is
//!   checked instruction by instruction against a log from a known-good
//!   emulator ([`run_nestest`]).
//! - blargg's newer tests write a status byte at $6000 and text from $6004,
//!   once $6001-$6003 hold the signature DE B0 61 ([`run_blargg`]).
//! - blargg's 2005 tests (sprite hit, sprite overflow) keep their result in
//!   zero page at $F8: 1 means passed and 2 or more is the number of the
//!   failing check ([`run_blargg_2005`]).
//! - Tests built with blargg's older shell on a board without PRG RAM
//!   (read_joy3, on CNROM) only print their result, so it's read off the
//!   screen ([`run_screen`]). So do the 2005 tests' own banners ("PASSED",
//!   "FAILED #2"), which some of them print without writing $F8.
//! - blargg's 2005 PPU and APU tests print only their result code, as
//!   `$01` on screen, with no text around it ([`run_screen_code`]).
//!
//! None of this needs the canonical state, so it runs any mapper
//! `tetanes-core` supports, not just NROM. Timeouts count frames, never
//! wall time, so a result can't depend on how fast the machine is.

use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;

use crate::{Nes, NesError, instructions};

/// Where a blargg test's status byte is.
const STATUS: u16 = 0x6000;
/// What $6001-$6003 hold while the status at $6000 is valid.
const SIGNATURE: [u8; 3] = [0xDE, 0xB0, 0x61];
/// Status while the test runs.
const RUNNING: u8 = 0x80;
/// Status when the test wants the reset button pressed, at least 100 ms later.
const NEEDS_RESET: u8 = 0x81;
/// Frames to wait before pressing reset: 100 ms is 6 NTSC frames; 10 is safe.
const RESET_DELAY_FRAMES: u64 = 10;
/// Where blargg's 2005 tests keep their result code.
const RESULT_2005: u16 = 0x00F8;

/// How a test ROM run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    Failed,
    /// The frame budget ran out before the ROM reported a result.
    TimedOut,
}

/// The result of one test ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    pub verdict: Verdict,
    /// The ROM's result code, if it gave one: 0 is passed for the $6000
    /// protocol, 1 for the 2005 tests.
    pub code: Option<u8>,
    /// What the ROM said, or what went wrong.
    pub message: String,
    /// Frames run.
    pub frames: u64,
}

fn idle() -> InputFrame {
    InputFrame {
        polled: false,
        pads: [0; 4],
        reset: false,
    }
}

/// Text a blargg test wrote from $6004, up to its terminating zero.
fn status_text(nes: &Nes) -> String {
    (0x6004..0x8000u16)
        .map(|addr| nes.peek(addr))
        .take_while(|&b| b != 0)
        .map(char::from)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn status(nes: &Nes) -> Option<u8> {
    let signature = [nes.peek(0x6001), nes.peek(0x6002), nes.peek(0x6003)];
    (signature == SIGNATURE).then(|| nes.peek(STATUS))
}

/// Runs a test that reports at $6000 for at most `max_frames` frames,
/// pressing reset when it asks.
pub fn run_blargg(rom: &[u8], power_on: PowerOn, max_frames: u64) -> Result<TestResult, NesError> {
    let mut nes = Nes::power_on(rom, power_on)?;
    // The frame at which the test asked for reset, until it's pressed.
    let mut reset_asked: Option<u64> = None;
    while nes.frame() < max_frames {
        if let Err(e) = nes.step(idle()) {
            return Ok(TestResult {
                verdict: Verdict::Failed,
                code: None,
                message: format!("the emulator stopped: {e}"),
                frames: nes.frame(),
            });
        }
        match status(&nes) {
            Some(NEEDS_RESET) => {
                let asked = *reset_asked.get_or_insert(nes.frame());
                if nes.frame() >= asked + RESET_DELAY_FRAMES {
                    nes.reset();
                    // The test writes RUNNING again once it restarts.
                    reset_asked = None;
                    // Don't press it twice while the old status is still up.
                    while status(&nes) == Some(NEEDS_RESET) && nes.frame() < max_frames {
                        nes.step(idle())?;
                    }
                }
            }
            Some(code) if code < RUNNING => {
                return Ok(TestResult {
                    verdict: if code == 0 {
                        Verdict::Passed
                    } else {
                        Verdict::Failed
                    },
                    code: Some(code),
                    message: status_text(&nes),
                    frames: nes.frame(),
                });
            }
            _ => {}
        }
    }
    let message = match status(&nes) {
        Some(_) => {
            let text = status_text(&nes);
            if text.is_empty() {
                "still running".to_owned()
            } else {
                format!("still running: {text}")
            }
        }
        None => "never wrote the $6000 signature".to_owned(),
    };
    Ok(TestResult {
        verdict: Verdict::TimedOut,
        code: None,
        message,
        frames: nes.frame(),
    })
}

/// Runs one of blargg's 2005 tests for `frames` frames, or until it reports
/// a pass, and reads its result code from $F8. They write the number of each
/// check to $F8 before running it, and 1 once all pass, so a failing test
/// leaves the number of the check that failed. These tests don't say when
/// they've finished failing, so a failure always runs the whole budget.
/// $F8 still at 0 or at its power-on value (such as $FF with RAM all ones)
/// means the test never wrote a result, which is a timeout.
pub fn run_blargg_2005(rom: &[u8], power_on: PowerOn, frames: u64) -> Result<TestResult, NesError> {
    let mut nes = Nes::power_on(rom, power_on)?;
    let at_power_on = nes.peek(RESULT_2005);
    while nes.frame() < frames {
        if let Err(e) = nes.step(idle()) {
            return Ok(TestResult {
                verdict: Verdict::Failed,
                code: None,
                message: format!("the emulator stopped: {e}"),
                frames: nes.frame(),
            });
        }
        if nes.peek(RESULT_2005) == 1 {
            break;
        }
    }
    let code = nes.peek(RESULT_2005);
    let (verdict, message) = match code {
        1 => (Verdict::Passed, "passed".to_owned()),
        0 => (Verdict::TimedOut, "no result at $F8".to_owned()),
        n if n == at_power_on => (
            Verdict::TimedOut,
            format!("no result at $F8: it still holds its power-on value {n:02X}h"),
        ),
        n => (Verdict::Failed, format!("failed check #{n}")),
    };
    Ok(TestResult {
        verdict,
        code: Some(code),
        message,
        frames: nes.frame(),
    })
}

/// The text on the first nametable, one line per tile row, as blargg's
/// console prints it: tile numbers are ASCII codes. Anything that isn't
/// printable reads as a space.
fn screen_text(nes: &Nes) -> String {
    let bus = nes.deck.bus();
    (0..30u16)
        .map(|row| {
            (0..32u16)
                .map(|col| match bus.ppu_bus_peek(0x2000 + row * 32 + col) {
                    b @ 0x20..=0x7E => char::from(b),
                    _ => ' ',
                })
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs a test that reports only on screen (blargg's older shell, on a
/// board with no PRG RAM for the $6000 protocol) until its console shows
/// "Passed", "Failed" or "Error" (or the 2005 shell's "PASSED" and
/// "FAILED"), or `max_frames` frames have run.
pub fn run_screen(rom: &[u8], power_on: PowerOn, max_frames: u64) -> Result<TestResult, NesError> {
    let mut nes = Nes::power_on(rom, power_on)?;
    while nes.frame() < max_frames {
        if let Err(e) = nes.step(idle()) {
            return Ok(TestResult {
                verdict: Verdict::Failed,
                code: None,
                message: format!("the emulator stopped: {e}"),
                frames: nes.frame(),
            });
        }
        // The console is drawn during vblank; checking every 10 frames is
        // plenty and keeps the cost down.
        if nes.frame() % 10 != 0 {
            continue;
        }
        let text = screen_text(&nes);
        let verdict = if text
            .lines()
            .any(|l| matches!(l.trim(), "Passed" | "PASSED"))
        {
            Verdict::Passed
        } else if text
            .lines()
            .map(str::trim)
            .any(|l| l == "Failed" || l.starts_with("Error ") || l.starts_with("FAILED"))
        {
            Verdict::Failed
        } else {
            continue;
        };
        return Ok(TestResult {
            verdict,
            code: None,
            message: text,
            frames: nes.frame(),
        });
    }
    Ok(TestResult {
        verdict: Verdict::TimedOut,
        code: None,
        message: format!("the screen shows no result: {:?}", screen_text(&nes)),
        frames: nes.frame(),
    })
}

/// Runs one of blargg's 2005 PPU or APU tests, which print nothing but
/// their result code, as `$01`, once they finish. 1 is passed; 2 or more is
/// the number of the check that failed (their readme.txt lists them). Stops
/// at the first frame whose screen holds a line that is just `$` and two hex
/// digits, or after `max_frames` frames.
pub fn run_screen_code(
    rom: &[u8],
    power_on: PowerOn,
    max_frames: u64,
) -> Result<TestResult, NesError> {
    let mut nes = Nes::power_on(rom, power_on)?;
    while nes.frame() < max_frames {
        if let Err(e) = nes.step(idle()) {
            return Ok(TestResult {
                verdict: Verdict::Failed,
                code: None,
                message: format!("the emulator stopped: {e}"),
                frames: nes.frame(),
            });
        }
        if nes.frame() % 10 != 0 {
            continue;
        }
        let text = screen_text(&nes);
        let Some(code) = text.lines().find_map(|l| {
            let hex = l.trim().strip_prefix('$')?;
            (hex.len() == 2).then(|| u8::from_str_radix(hex, 16).ok())?
        }) else {
            continue;
        };
        let (verdict, message) = match code {
            1 => (Verdict::Passed, "passed".to_owned()),
            n => (Verdict::Failed, format!("failed check #{n}")),
        };
        return Ok(TestResult {
            verdict,
            code: Some(code),
            message,
            frames: nes.frame(),
        });
    }
    Ok(TestResult {
        verdict: Verdict::TimedOut,
        code: None,
        message: format!("the screen shows no result code: {:?}", screen_text(&nes)),
        frames: nes.frame(),
    })
}

/// The CPU state nestest.log records before each instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuLine {
    pc: u16,
    a: u8,
    x: u8,
    y: u8,
    p: u8,
    s: u8,
    scanline: u16,
    dot: u16,
    cycles: u64,
}

impl std::fmt::Display for CpuLine {
    /// The register columns, from ` A:` on, as nestest.log writes them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            " A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} PPU:{:3},{:3} CYC:{}",
            self.a, self.x, self.y, self.p, self.s, self.scanline, self.dot, self.cycles
        )
    }
}

/// Where the register columns start in a nestest.log line. Before them are
/// the address, the instruction's bytes and its disassembly.
const REGISTERS: usize = 47;

/// A line's columns before the registers, without trailing spaces.
fn disassembly(line: &str) -> &str {
    line.get(..REGISTERS).unwrap_or(line).trim_end()
}

/// The part of a line's disassembly two emulators should agree on. For an
/// operand in the PPU and APU registers ($2000-$401F) that leaves out the
/// value after the last `=`: it's what each emulator's debugger shows for a
/// register read without side effects, not what the CPU does. nestest.log
/// shows FF for the write-only APU registers, where `tetanes-core` shows 00.
fn comparable(line: &str) -> &str {
    let d = disassembly(line);
    let register = d.split([' ', '(', ')', ',', '$']).any(|word| {
        word.len() == 4
            && u16::from_str_radix(word, 16).is_ok_and(|a| (0x2000..=0x401F).contains(&a))
    });
    match d.rfind(" = ") {
        Some(at) if register => &d[..at],
        _ => d,
    }
}

/// One line of nestest.log, and whether its opcode is an unofficial one
/// (the log marks those with `*` before the mnemonic).
fn parse_log_line(line: &str) -> Option<(CpuLine, bool)> {
    let hex8 = |key: &str| -> Option<u8> {
        let at = line.find(key)? + key.len();
        u8::from_str_radix(line.get(at..at + 2)?, 16).ok()
    };
    let pc = u16::from_str_radix(line.get(0..4)?, 16).ok()?;
    let unofficial = line.get(15..16)? == "*";
    let ppu = line.get(line.find("PPU:")? + 4..)?;
    let (scanline, rest) = ppu.split_once(',')?;
    let dot = rest.get(..3)?;
    let cycles = line.get(line.find("CYC:")? + 4..)?.trim();
    Some((
        CpuLine {
            pc,
            a: hex8(" A:")?,
            x: hex8(" X:")?,
            y: hex8(" Y:")?,
            p: hex8(" P:")?,
            s: hex8(" SP:")?,
            scanline: scanline.trim().parse().ok()?,
            dot: dot.trim().parse().ok()?,
            cycles: cycles.parse().ok()?,
        },
        unofficial,
    ))
}

/// NTSC dots in a frame with rendering off, which nestest's is: 262
/// scanlines of 341, with no short odd frames.
const DOTS_PER_FRAME: u32 = 262 * 341;

/// The PPU position as dots since the start of the frame.
fn dot_index(line: &CpuLine) -> u32 {
    u32::from(line.scanline) * 341 + u32::from(line.dot)
}

/// How far nestest matched its log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestestResult {
    /// Log lines up to the first unofficial opcode.
    pub official_lines: usize,
    /// Every line in the log.
    pub total_lines: usize,
    /// Lines matched before the first mismatch (all of them if none).
    pub matched: usize,
    /// The first mismatch: its 1-based line number, the log's line and ours.
    pub mismatch: Option<(usize, String, String)>,
    /// nestest's own result bytes after the run: $02 for official opcodes and
    /// $03 for unofficial ones, 0 when passed.
    pub codes: [u8; 2],
}

impl NestestResult {
    pub fn official_passed(&self) -> bool {
        self.matched >= self.official_lines && self.codes[0] == 0
    }

    pub fn all_passed(&self) -> bool {
        self.matched == self.total_lines && self.codes == [0, 0]
    }
}

/// Runs nestest in automation mode: from power-on with `power_on`, sets PC to $C000, then
/// runs one instruction per log line. Before each one it writes the
/// instruction trace's line for it ([`instructions::line`], the same as
/// `gp trace instructions`) and compares it with the log's: the address,
/// bytes and disassembly as text, then A, X, Y, P, SP, the PPU position and
/// the CPU cycle count. The log's cycle count starts at 7 (the reset
/// sequence); ours is lined up to it on the first line.
pub fn run_nestest(rom: &[u8], log: &str, power_on: PowerOn) -> Result<NestestResult, NesError> {
    let lines: Vec<(&str, CpuLine, bool)> = log
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| {
            let (cpu, unofficial) = parse_log_line(l).ok_or_else(|| {
                NesError::Unsupported(format!("nestest.log line {} can't be read: {l}", i + 1))
            })?;
            Ok((l, cpu, unofficial))
        })
        .collect::<Result<_, NesError>>()?;
    let official_lines = lines.iter().take_while(|(_, _, u)| !u).count();
    let mut nes = Nes::power_on(rom, power_on)?;
    nes.deck.bus_mut().cpu.pc = 0xC000;
    let traced = |nes: &Nes| {
        let mut text = String::new();
        instructions::line(nes.deck.bus(), &mut text);
        let (cpu, _) = parse_log_line(&text).expect("the tracer writes nestest.log's layout");
        (text, cpu)
    };
    // The log starts 7 cycles in, after the reset sequence, and its PPU is
    // aligned to the CPU one dot differently (tetanes-core fixes the
    // CPU-PPU offset at 1). Line both up on the first line; a drift after
    // that is a mismatch.
    let (_, start) = traced(&nes);
    let (cycle_offset, dot_offset) = lines.first().map_or((0, 0), |(_, l, _)| {
        (
            l.cycles.wrapping_sub(start.cycles),
            dot_index(l) as i64 - dot_index(&start) as i64,
        )
    });
    let mut mismatch = None;
    let mut matched = 0;
    for (i, (want_text, want, _)) in lines.iter().enumerate() {
        let (text, mut got) = traced(&nes);
        got.cycles = got.cycles.wrapping_add(cycle_offset);
        let dot = (dot_index(&got) as i64 + dot_offset).rem_euclid(DOTS_PER_FRAME as i64) as u32;
        (got.scanline, got.dot) = ((dot / 341) as u16, (dot % 341) as u16);
        if got != *want || comparable(&text) != comparable(want_text) {
            mismatch = Some((
                i + 1,
                want_text.trim_end().to_owned(),
                format!("{:<REGISTERS$}{got}", disassembly(&text)),
            ));
            break;
        }
        matched += 1;
        if let Err(e) = nes.deck.clock_instr() {
            mismatch = Some((i + 2, "the next instruction".into(), format!("{e}")));
            break;
        }
    }
    Ok(NestestResult {
        official_lines,
        total_lines: lines.len(),
        matched,
        mismatch,
        codes: [nes.peek(0x0002), nes.peek(0x0003)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrom;

    #[test]
    fn reads_a_nestest_log_line() {
        let (line, unofficial) = parse_log_line(
            "C000  4C F5 C5  JMP $C5F5                       A:00 X:00 Y:00 P:24 SP:FD PPU:  0, 21 CYC:7",
        )
        .unwrap();
        assert!(!unofficial);
        assert_eq!(
            line,
            CpuLine {
                pc: 0xC000,
                a: 0,
                x: 0,
                y: 0,
                p: 0x24,
                s: 0xFD,
                scanline: 0,
                dot: 21,
                cycles: 7
            }
        );
        let (line, unofficial) = parse_log_line(
            "C6BD  04 A9    *NOP $A9 = 00                    A:AA X:97 Y:4E P:EF SP:F9 PPU:128, 89 CYC:14579\r",
        )
        .unwrap();
        assert!(unofficial);
        assert_eq!((line.scanline, line.dot, line.cycles), (128, 89, 14579));
    }

    #[test]
    fn register_values_are_left_out_of_the_comparison() {
        let log = "C68B  8D 15 40  STA $4015 = FF                  A:02 X:FF Y:15 P:25 SP:FB PPU:233,107 CYC:26520";
        let ours = log.replace("= FF", "= 00");
        assert_eq!(comparable(log), "C68B  8D 15 40  STA $4015");
        assert_eq!(comparable(log), comparable(&ours));
        let ram = "D959  B1 FF     LDA ($FF),Y = 0146 @ 0245 = 12  A:01 X:65 Y:FF P:E5 SP:FA PPU: 77,215 CYC:8824";
        assert_eq!(
            comparable(ram),
            "D959  B1 FF     LDA ($FF),Y = 0146 @ 0245 = 12"
        );
    }

    /// Our own test program as a stand-in for a blargg ROM: it never writes
    /// the signature, so the run times out and says why.
    #[test]
    fn a_rom_without_the_signature_times_out() {
        let r = run_blargg(&testrom::build(), PowerOn::default(), 30).unwrap();
        assert_eq!(r.verdict, Verdict::TimedOut);
        assert_eq!(r.frames, 30);
        assert_eq!(r.message, "never wrote the $6000 signature");
    }

    #[test]
    fn a_2005_test_with_nothing_at_f8_times_out() {
        let r = run_blargg_2005(&testrom::build(), PowerOn::default(), 20).unwrap();
        assert_eq!(r.verdict, Verdict::TimedOut);
        assert_eq!(r.code, Some(0));
    }

    /// With RAM all ones, $F8 starts at $FF; a test that never writes it
    /// has timed out, not failed check #255.
    #[test]
    fn a_2005_test_with_power_on_ones_at_f8_times_out() {
        let power_on = PowerOn {
            ram_init: gp_evidence::manifest::RamInit::Ones,
            ..PowerOn::default()
        };
        let r = run_blargg_2005(&testrom::build(), power_on, 20).unwrap();
        assert_eq!(r.verdict, Verdict::TimedOut, "{}", r.message);
        assert_eq!(r.code, Some(0xFF));
    }

    #[test]
    fn a_screen_without_a_result_code_times_out() {
        let r = run_screen_code(&testrom::build(), PowerOn::default(), 20).unwrap();
        assert_eq!(r.verdict, Verdict::TimedOut);
        assert_eq!(r.frames, 20);
    }

    /// The test program's log of its first instructions, written by hand from
    /// its source (testrom.rs): from $C000, SEI, CLD, LDX #$FF, TXS.
    #[test]
    fn nestest_matching_reports_the_first_mismatch() {
        let log = "\
C000  78        SEI                             A:00 X:00 Y:00 P:24 SP:FD PPU:  0, 21 CYC:7
C001  D8        CLD                             A:00 X:00 Y:00 P:24 SP:FD PPU:  0, 27 CYC:9
C002  A2 FF     LDX #$FF                        A:00 X:00 Y:00 P:24 SP:FD PPU:  0, 33 CYC:11
C004  9A        TXS                             A:00 X:FF Y:00 P:A4 SP:FD PPU:  0, 39 CYC:13
C005  2C 02 20  BIT $2002 = 00                  A:00 X:FF Y:00 P:A4 SP:FF PPU:  0, 45 CYC:15
";
        let r = run_nestest(&testrom::build(), log, PowerOn::default()).unwrap();
        assert_eq!((r.total_lines, r.official_lines), (5, 5));
        let wrong = log.replace("P:A4 SP:FF", "P:A4 SP:FE");
        let r = run_nestest(&testrom::build(), &wrong, PowerOn::default()).unwrap();
        assert_eq!(r.matched, 4);
        let (line, want, got) = r.mismatch.unwrap();
        assert_eq!(line, 5);
        assert!(want.contains("SP:FE") && got.contains("SP:FF"), "{got}");
    }
}
