//! Instruction traces: one line per CPU instruction over a window of
//! frames, in the layout of nestest.log so they read side by side with other
//! emulators' traces.
//!
//! ```text
//! ; frame 70, pads 80 00
//! C017  E6 00     INC $00 = 42                    A:43 X:00 Y:00 P:A4 SP:FC PPU:241, 30 CYC:2052489
//! C019  A5 00     LDA $00 = 43                    A:43 X:00 Y:00 P:24 SP:FC PPU:241, 44 CYC:2052494
//! ...
//! C014  4C 14 C0  JMP $C014                       A:44 X:00 Y:00 P:A4 SP:FF PPU:241,  0 CYC:2082260
//! ; NMI
//! ```
//!
//! Each line is the state before the instruction runs: its address, its
//! bytes, the instruction (unofficial opcodes marked `*`), what its operand
//! comes to (`@` the address it reaches, `=` the value there, read without
//! side effects), then the registers, with bit 5 of P set as nestest.log
//! shows it, the PPU's scanline and dot and the CPU cycle count. nestest
//! runs through this tracer in `gp testroms` and matches all 8,991 lines of
//! its log. `; NMI` and `; IRQ` mark an interrupt taken after the
//! instruction above, so the next line is the handler's first. Lines starting
//! with `;` are comments. An interrupt taken by the instruction that ended
//! the frame before the window isn't marked: the window then starts at the
//! handler's first instruction, as in the example, which is our test program
//! (`gp_nes::testrom`).
//!
//! Tracing reads the console between instructions through
//! [`crate::Nes::step_with`] and changes nothing, so a traced frame ends in the
//! same state as an untraced one.

use std::fmt::Write as _;
use std::io::{self, Write};

use tetanes_core::bus::Bus;
use tetanes_core::cpu::Resolved;
use tetanes_core::cpu::instr::AddrMode;
use tetanes_core::debug::FrameKind;

use gp_evidence::input_log::InputFrame;

use crate::Nes;
use crate::seek::{Cursor, SeekError};

/// What a trace covered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Traced {
    /// The first frame traced, and the frame after the last.
    pub start: u64,
    pub end: u64,
    pub instructions: u64,
    pub interrupts: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error(transparent)]
    Seek(#[from] SeekError),
    #[error("writing the trace: {0}")]
    Io(#[from] io::Error),
}

/// Traces the recording's frames after the one `cursor` is at, up to and
/// including frame `last`, writing each frame's lines to `out` as it
/// finishes. Frames are numbered as in the recording format: frame `f` is
/// the one that produces state `f`, run with input log entry `f - 1`, so a
/// cursor at state 99 traces frame 100 first. The event stream labels its
/// events the same way.
pub fn trace(cursor: &mut Cursor, last: u64, out: &mut impl Write) -> Result<Traced, TraceError> {
    let frames = cursor.frames();
    let mut inputs = Vec::new();
    for f in cursor.frame() + 1..=last {
        let Some(i) = cursor.input(f - 1) else {
            return Err(SeekError::PastEnd { frame: f, frames }.into());
        };
        inputs.push(i);
    }
    trace_frames(cursor.nes_mut(), &inputs, out)
}

/// Traces the next `inputs.len()` frames of `nes`, one input each, writing
/// each frame's lines to `out` as it finishes: the one stepping loop every
/// instruction trace goes through, for a recording ([`trace`]) or for a run
/// from a recipe (gp-events' detail level).
pub fn trace_frames(
    nes: &mut Nes,
    inputs: &[InputFrame],
    out: &mut impl Write,
) -> Result<Traced, TraceError> {
    let start = nes.frame() + 1;
    let mut traced = Traced {
        start,
        end: start,
        instructions: 0,
        interrupts: 0,
    };
    // Stale until the next interrupt, so clear it before the first frame.
    nes.deck.bus_mut().interrupt_taken = None;
    let mut text = String::new();
    for &input in inputs {
        let frame = nes.frame() + 1;
        text.clear();
        let _ = writeln!(
            text,
            "; frame {frame}, pads {:02X} {:02X}",
            input.pads[0], input.pads[1]
        );
        let (mut instructions, mut interrupts) = (0, 0);
        nes.step_with(input, |bus| {
            if interrupt(bus, &mut text) {
                interrupts += 1;
            }
            line(bus, &mut text);
            instructions += 1;
        })
        .map_err(SeekError::from)?;
        // The NMI that follows the instruction ending the frame.
        if interrupt(nes.deck.bus_mut(), &mut text) {
            interrupts += 1;
        }
        out.write_all(text.as_bytes())?;
        traced.instructions += instructions;
        traced.interrupts += interrupts;
        traced.end = frame + 1;
    }
    Ok(traced)
}

/// Writes `; NMI` or `; IRQ` if the last instruction was followed by an
/// interrupt, and clears the record of it.
fn interrupt(bus: &mut Bus, out: &mut String) -> bool {
    match bus.interrupt_taken.take() {
        Some(FrameKind::Nmi) => out.push_str("; NMI\n"),
        Some(FrameKind::Irq) => out.push_str("; IRQ\n"),
        _ => return false,
    }
    true
}

/// The trace line for the instruction at the program counter.
pub fn line(bus: &Bus, out: &mut String) {
    let cpu = &bus.cpu;
    let mut pc = cpu.pc;
    let disasm = bus.disassemble(&mut pc);
    let mut bytes = format!("{:02X}", disasm.instr.opcode);
    for b in disasm.operands() {
        let _ = write!(bytes, " {b:02X}");
    }
    // `tetanes-core` writes a one-column marker before the mnemonic: `*` for
    // an unofficial opcode, a space for the rest.
    let name = disasm.instr.to_string();
    let (star, mut asm) = match name.strip_prefix('*') {
        Some(rest) => ('*', rest.to_owned()),
        None => (' ', name.trim_start().to_owned()),
    };
    if !disasm.operand.is_empty() {
        let _ = write!(asm, " {}", disasm.operand);
    } else if disasm.instr.addr_mode == AddrMode::ACC {
        asm.push_str(" A");
    }
    // What the operand comes to, in nestest.log's order: `($80,X)` shows
    // the zero page pointer it reads and the address there, and `($80),Y`
    // the address the pointer holds and then that plus Y.
    if let Some(addr) = disasm.effective {
        match disasm.instr.addr_mode {
            AddrMode::ZPX | AddrMode::ZPY => {
                let _ = write!(asm, " @ {addr:02X}");
            }
            AddrMode::IDX => {
                let pointer = disasm.bytes[0].wrapping_add(cpu.x);
                let _ = write!(asm, " @ {pointer:02X} = {addr:04X}");
            }
            AddrMode::IDY | AddrMode::IDYW => {
                let base = addr.wrapping_sub(cpu.y.into());
                let _ = write!(asm, " = {base:04X} @ {addr:04X}");
            }
            _ => {
                let _ = write!(asm, " @ {addr:04X}");
            }
        }
    }
    match disasm.value {
        Some(Resolved::Byte(b)) => {
            let _ = write!(asm, " = {b:02X}");
        }
        Some(Resolved::Word(w)) => {
            let _ = write!(asm, " = {w:04X}");
        }
        None => {}
    }
    // Bit 5 of P isn't stored in a real 6502: it reads as 1 whenever P is
    // pushed, and nestest.log always shows it set. `tetanes-core` keeps
    // whatever PLP or RTI pulled.
    let _ = writeln!(
        out,
        "{:04X}  {bytes:<8} {star}{asm:<31} A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} PPU:{:>3},{:>3} CYC:{}",
        cpu.pc,
        cpu.acc,
        cpu.x,
        cpu.y,
        cpu.status.bits() | 0x20,
        cpu.sp,
        bus.ppu.scanline,
        bus.ppu.cycle,
        cpu.cycle,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::{Capture, RecordRequest, record_with, stored_events};
    use crate::{Nes, seek, testrom};
    use gp_evidence::Store;
    use gp_evidence::events::EventKind;
    use gp_evidence::input_log::generate;
    use gp_evidence::manifest::{PowerOn, Provenance, RecordingManifest, SourceRef};

    fn recorded(store: &Store, rom: &[u8]) -> RecordingManifest {
        let input = generate(120, &["a@10-20", "b@60-"]).unwrap();
        record_with(
            store,
            RecordRequest {
                rom,
                source: SourceRef::NesRom {
                    sha256: gp_evidence::source::identify_nes(rom).unwrap().sha256,
                    mapper: 0,
                },
                source_id: None,
                power_on: PowerOn::default(),
                input: &input,
                checkpoint_interval: 30,
                provenance: Provenance::Scripted,
                name: "trace".into(),
                created_at: "2026-10-27T00:00:00Z".into(),
                notes: None,
            },
            Capture::ALL,
        )
        .unwrap()
        .manifest
    }

    #[test]
    fn traces_a_window_without_changing_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let rom = testrom::build();
        let manifest = recorded(&store, &rom);
        // At state 69, the next frame is 70, run with input log entry 69.
        let mut cursor = seek::open(&store, &manifest, &rom, 69).unwrap();
        let mut out = Vec::new();
        let traced = trace(&mut cursor, 71, &mut out).unwrap();
        assert_eq!((traced.start, traced.end), (70, 72));
        assert_eq!(cursor.frame(), 71);
        // The traced frames end where the recording did, and go on matching.
        assert!(cursor.matches_recording().unwrap());
        cursor.advance_to(120).unwrap();
        assert!(cursor.matches_recording().unwrap());

        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "; frame 70, pads 02 00");
        let frame71 = lines
            .iter()
            .position(|l| l.starts_with("; frame 71"))
            .unwrap();
        let code: Vec<&&str> = lines.iter().filter(|l| !l.starts_with(';')).collect();
        assert_eq!(code.len() as u64, traced.instructions);
        // One NMI a frame: the test program waits for each.
        assert_eq!(traced.interrupts, 2);
        assert_eq!(lines.iter().filter(|l| **l == "; NMI").count(), 2);
        assert!(frame71 > 2);
        for l in &code {
            assert_eq!(&l[48..50], "A:", "{l}");
            // Two spaces between the bytes and the mnemonic, or a space and `*`.
            assert!(matches!(&l[14..16], "  " | " *"), "{l}");
            assert_ne!(&l[16..17], " ", "{l}");
            assert!(l.contains(" PPU:") && l.contains(" CYC:"), "{l}");
        }
        // Cycles only go forward.
        let cycles: Vec<u64> = code
            .iter()
            .map(|l| l.rsplit("CYC:").next().unwrap().parse().unwrap())
            .collect();
        assert!(cycles.windows(2).all(|w| w[0] < w[1]));
    }

    /// The values a frame's instructions write to $4016, read off its trace
    /// lines: the register each store writes, from the line's own columns.
    fn strobe_writes(text: &str, frame: u64) -> Vec<u8> {
        let header = format!("; frame {frame},");
        text.lines()
            .skip_while(|l| !l.starts_with(&header))
            .skip(1)
            .take_while(|l| !l.starts_with("; frame "))
            .filter(|l| l.get(16..25).is_some_and(|op| op.ends_with(" $4016")))
            .filter_map(|l| {
                let register = match &l[16..19] {
                    "STA" => " A:",
                    "STX" => " X:",
                    "STY" => " Y:",
                    _ => return None,
                };
                let at = l.find(register)? + register.len();
                u8::from_str_radix(&l[at..at + 2], 16).ok()
            })
            .collect()
    }

    /// Frame N in an instruction trace is frame N in the event stream: the
    /// controller strobe writes its instructions make are the
    /// controller-write events recorded for that frame.
    #[test]
    fn trace_frames_are_event_frames() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let rom = testrom::build();
        let manifest = recorded(&store, &rom);
        let mut cursor = seek::open(&store, &manifest, &rom, 39).unwrap();
        let mut out = Vec::new();
        trace(&mut cursor, 44, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let events = stored_events(&store, &manifest, 40..45).unwrap().unwrap();
        let mut polled = 0;
        for frame in 40..45 {
            let recorded: Vec<u8> = events
                .iter()
                .filter(|e| e.frame == frame && e.kind == EventKind::ControllerWrite)
                .map(|e| e.value)
                .collect();
            assert_eq!(strobe_writes(&text, frame), recorded, "frame {frame}");
            polled += usize::from(!recorded.is_empty());
        }
        // The test program polls every other frame, so some frames write
        // the strobe and some don't, and the two can't line up by chance.
        assert!((2..=3).contains(&polled), "{polled}");
    }

    #[test]
    fn formats_like_nestest() {
        let nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        let mut text = String::new();
        line(nes.bus(), &mut text);
        let pc = nes.bus().cpu.pc;
        // The reset vector's first instruction, at power-on: P is $24 and
        // SP $FD after the reset sequence, as in nestest.log.
        assert!(text.starts_with(&format!("{pc:04X}  ")), "{text}");
        assert!(text.contains(" P:24 SP:FD "), "{text}");
        assert_eq!(&text[48..50], "A:", "{text}");
    }
}
