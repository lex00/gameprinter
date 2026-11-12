//! L2, frame events: what each frame of a run did, in a form small enough
//! to keep for every run and to send to a watcher live.
//!
//! | Part | From |
//! |---|---|
//! | `rom` | The console's own event log (interrupts, sprite 0, mapper writes, controller reads and writes, PPU and APU writes), as recordings store it |
//! | `new_code` | PRG bytes first executed in this frame, as ranges of offsets |
//! | `new_edges` | Transfers first taken in this frame (L1 keys) |
//! | `ram` | Internal RAM bytes whose value differs from the frame before, with both values |
//! | `watch` | Each write to a watched address: the value written and the writing instruction |

use std::collections::BTreeSet;

use gp_evidence::events::Event;
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_nes::Nes;
use serde::Serialize;
use tetanes_core::cpu::Cpu;
use tetanes_core::cpu::instr::Instr;

use crate::bits::Bits;
use crate::collector::Collector;
use crate::edges::{EdgeKey, NO_ROM};
use crate::layout::{Layout, RAM_BYTES};
use crate::run::{Run, RunError};

/// A write to a watched address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Watched {
    pub addr: u16,
    pub value: u8,
    /// The writing instruction's PRG offset, or [`NO_ROM`], and its CPU
    /// address.
    pub offset: u32,
    pub cpu: u16,
}

/// A hooked instruction about to run, with the registers it sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hooked {
    /// Its PRG offset and CPU address.
    pub offset: u32,
    pub cpu: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub p: u8,
}

/// One frame of L2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrameEvents {
    pub frame: u64,
    pub polled: bool,
    #[serde(serialize_with = "rom_events")]
    pub rom: Vec<Event>,
    /// PRG offset ranges `[start, end)` first executed this frame.
    pub new_code: Vec<(u32, u32)>,
    pub new_edges: Vec<EdgeKey>,
    /// (address, value before, value after) for each RAM byte that changed.
    pub ram: Vec<(u16, u8, u8)>,
    pub watch: Vec<Watched>,
    /// Each hooked instruction run this frame (when hooks were asked for).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<Hooked>,
}

/// The console's events as `gp events --json` prints them.
fn rom_events<S: serde::Serializer>(v: &[Event], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(v.len()))?;
    for e in v {
        seq.serialize_element(&serde_json::json!({
            "addr": e.addr,
            "dot": e.dot,
            "frame": e.frame,
            "kind": e.kind.name(),
            "scanline": e.scanline,
            "value": e.value,
        }))?;
    }
    seq.end()
}

/// An address as a watch names it: internal RAM's mirrors fold to
/// $0000-$07FF and the PPU registers' to $2000-$2007.
pub fn fold(a: u16) -> u16 {
    match a {
        0..0x2000 => a & (RAM_BYTES as u16 - 1),
        0x2000..0x4000 => 0x2000 | (a & 7),
        _ => a,
    }
}

/// Every CPU address that folds to `a`, as [`fold`] folds them.
fn mirrors(a: u16) -> Vec<u16> {
    match a {
        0..0x800 => (0..4).map(|k| a + k * 0x800).collect(),
        0x2000..0x2008 => (0..1024).map(|k| a + k * 8).collect(),
        _ => vec![a],
    }
}

/// The watched writes among the shim's: each write an instruction's operand
/// names (not a push), the last of an instruction's writes to one address
/// (a read-modify-write puts the old value back first), and a
/// read-modify-write or an unofficial store kept only in RAM, where its
/// value is what reads back.
fn watched(writes: &[tetanes_core::shim::ShimWrite]) -> Vec<Watched> {
    let mut out: Vec<(u64, Watched)> = Vec::new();
    for w in writes.iter().filter(|w| !w.push) {
        let addr = fold(w.addr);
        let plain = matches!(
            Cpu::INSTR_REF[usize::from(w.opcode)].instr,
            Instr::STA | Instr::STX | Instr::STY | Instr::SAX
        );
        if !plain && addr >= 0x2000 {
            continue;
        }
        let hit = Watched {
            addr,
            value: w.value,
            offset: w.offset.map_or(NO_ROM, |o| o as u32),
            cpu: w.pc,
        };
        match out.last_mut() {
            Some((n, last)) if *n == w.instruction && last.addr == addr => *last = hit,
            _ => out.push((w.instruction, hit)),
        }
    }
    out.into_iter().map(|(_, w)| w).collect()
}

/// Runs `rom` from power-on through `inputs` with L0, L1 and L2, watching
/// writes to `watch` (folded as [`fold`] says).
pub fn run_frames(
    rom: &[u8],
    power_on: PowerOn,
    inputs: &[InputFrame],
    watch: &BTreeSet<u16>,
) -> Result<(Run, Vec<FrameEvents>), RunError> {
    let mut out = Vec::with_capacity(inputs.len());
    let run = run_frames_with(rom, power_on, inputs, watch, &[], |f, _| out.push(f))?;
    Ok((run, out))
}

/// [`run_frames`], giving each frame's L2 to `on_frame` as the frame ends,
/// with the collector as it stands (its edge table holds each new edge's
/// record), so a watcher sees the run as it goes. The instructions at the
/// PRG offsets in `hooks` are hooked: the shim calls out before each runs,
/// and the frame lists them with their registers.
pub fn run_frames_with(
    rom: &[u8],
    power_on: PowerOn,
    inputs: &[InputFrame],
    watch: &BTreeSet<u16>,
    hooks: &[usize],
    mut on_frame: impl FnMut(FrameEvents, &Collector),
) -> Result<Run, RunError> {
    let mut nes = Nes::power_on(rom, power_on)?;
    nes.set_events(true);
    let layout = Layout::of(nes.bus())?;
    let mut c = Collector::new(layout, true);
    c.edges.as_mut().expect("asked for").track_fresh();
    c.start(nes.bus_mut_for_observers());
    c.attach(nes.bus_mut_for_observers());
    if let Some(sh) = nes.bus_mut_for_observers().shim.as_deref_mut() {
        for &a in watch {
            for m in mirrors(fold(a)) {
                sh.watch_addr(m);
            }
        }
        for &o in hooks {
            sh.hook_offset(o);
        }
    }
    let mut seen = Bits::new(layout.prg_rom);
    let mut ram_before = nes.wram()[..RAM_BYTES].to_vec();
    for i in inputs {
        let input = InputFrame {
            polled: true,
            pads: i.pads,
            reset: i.reset,
        };
        let outcome = c.step(&mut nes, input)?;
        let writes = nes
            .bus_mut_for_observers()
            .shim
            .as_deref_mut()
            .map(|s| std::mem::take(&mut s.writes))
            .unwrap_or_default();
        let frame = nes.frame();
        let new_code = fresh_ranges(&c.footprint.executed, &mut seen);
        let new_edges = c.edges.as_mut().expect("asked for").take_fresh();
        let ram_now = &nes.wram()[..RAM_BYTES];
        let ram = ram_before
            .iter()
            .zip(ram_now)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i as u16, *a, *b))
            .collect();
        ram_before.copy_from_slice(ram_now);
        let f = FrameEvents {
            frame,
            polled: outcome.polled,
            rom: nes.take_events(),
            new_code,
            new_edges,
            ram,
            watch: watched(&writes),
            hooks: std::mem::take(&mut c.hooked),
        };
        on_frame(f, &c);
    }
    Ok(Run {
        collector: c,
        frames: inputs.len() as u64,
    })
}

/// The offsets set in `now` and not in `seen`, as ranges; adds them to
/// `seen`.
fn fresh_ranges(now: &Bits, seen: &mut Bits) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for (wi, (n, s)) in now.words().iter().zip(seen.words_mut()).enumerate() {
        let new = n & !*s;
        if new == 0 {
            continue;
        }
        *s |= new;
        let mut bits = new;
        while bits != 0 {
            let b = bits.trailing_zeros();
            let o = (wi * 64) as u32 + b;
            match out.last_mut() {
                Some(r) if r.1 == o => r.1 += 1,
                _ => out.push((o, o + 1)),
            }
            bits &= bits - 1;
        }
    }
    out
}
