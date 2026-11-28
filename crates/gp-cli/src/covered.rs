//! A frame of the original with its code coverage, on either platform:
//! the feature map gp-nes's covered step fills ([`gp_nes::coverage`]),
//! filled on an arcade romset from gp-arcade's instruction trace.
//!
//! On the arcade board the main Z80's features use the map's own ranges
//! (an instruction ran at an address, a conditional went to its target or
//! fell through), since the main CPU's address space is 16 bits as the
//! NES's is; the sound Z80's go into the caller's range
//! ([`gp_nes::coverage::EXTRA`]), hashed with a tag of their own, so the
//! two CPUs' code at one address count apart. A branch's direction is
//! where the same CPU's next instruction starts; one an interrupt cut
//! into counts no direction. `djnz` and the conditional calls and returns
//! count as branches, as the 6502's `bcc` and the others do.

use gp_arcade::dasm::{self, Flow};
use gp_arcade::mcr::Trace;
use gp_arcade::{Controls, Mcr};
use gp_nes::coverage::{CodeMap, EXEC, FELL, TAKEN};
use gp_runtime::InputFrame;

use crate::original::OriginalRuntime;

/// The tag the sound CPU's features are hashed with.
const SOUND: u64 = 0x5_0000_0000;

/// One frame of `o` on `input`, adding the code it runs to `map`; whether
/// the game read its inputs. The frame ends in the state an uncovered one
/// does.
pub(crate) fn step_covered(
    o: &mut OriginalRuntime,
    input: &InputFrame,
    map: &mut CodeMap,
) -> Result<bool, String> {
    match o {
        OriginalRuntime::Nes(n) => n.step_covered(input, map),
        OriginalRuntime::Arcade(a) => {
            // advance()'s checks, then the frame with the trace on.
            if input.reset || !input.axes.is_empty() || input.buttons > 0x7f {
                return a.advance(input);
            }
            Ok(frame_covered(
                a.mcr_mut(),
                Controls::from_bits(input.buttons),
                map,
            ))
        }
    }
}

/// One frame of the machine with its trace on, its features added to
/// `map`; whether the game read its inputs.
pub(crate) fn frame_covered(t: &mut Mcr, c: Controls, map: &mut CodeMap) -> bool {
    let trace = frame_trace(t, c);
    add_trace(&trace, map);
    t.polled()
}

/// One frame with the instruction trace on: both CPUs' instructions in
/// time order.
pub(crate) fn frame_trace(t: &mut Mcr, c: Controls) -> Vec<Trace> {
    t.set_trace(true);
    t.step(c);
    let mut trace = t.take_trace();
    t.set_trace(false);
    trace.sort_by_key(|e| e.time);
    trace
}

/// The features of a frame's trace.
pub(crate) fn add_trace(trace: &[Trace], map: &mut CodeMap) {
    for cpu in 0..2u8 {
        let mut prev: Option<(u16, Flow, u16)> = None;
        for e in trace.iter().filter(|e| e.cpu == cpu) {
            if e.interrupt.is_some() {
                prev = None;
                continue;
            }
            if let Some((pc, flow, fall)) = prev
                && flow.conditional()
            {
                let went = e.pc != fall;
                if cpu == 0 {
                    map.set(if went { TAKEN } else { FELL } + u32::from(pc));
                } else {
                    let kind = if went { 1u64 } else { 2 };
                    map.extra(SOUND | kind << 16 | u64::from(pc));
                }
            }
            if cpu == 0 {
                map.set(EXEC + u32::from(e.pc));
            } else {
                map.extra(SOUND | u64::from(e.pc));
            }
            let insn = dasm::decode(&e.bytes, e.pc);
            prev = Some((e.pc, insn.flow, e.pc.wrapping_add(u16::from(insn.len))));
        }
    }
}
