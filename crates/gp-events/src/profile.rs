//! L3's counts: how often each instruction ran in a window of a run, and
//! for each conditional branch how often it was taken and how often it fell
//! through. Like L3 it is made again by running the recipe; unlike L3 it is
//! small (one row per instruction run), so the store keeps it
//! ([`crate::store::Store::profile`]).
//!
//! `gp code listing` prints a routine's instructions with these counts, and
//! `gp reach --window` reports and compares windows by them.
//!
//! The way a branch went is read from the instruction that runs next: the
//! one just after the branch is a fall, any other a take. An NMI or IRQ
//! between them is seen through: the address it pushed is where the branch
//! went.

use std::collections::BTreeMap;

use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_nes::Nes;
use serde::{Deserialize, Serialize};
use tetanes_core::debug::FrameKind;

use crate::layout::Layout;
use crate::run::RunError;

/// The version of what a profile holds. A change bumps it, and the store's
/// profiles are made again.
pub const PROFILE_VERSION: u32 = 1;

/// One instruction's counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsnStat {
    /// The CPU address it first ran at.
    pub cpu: u16,
    pub count: u64,
    /// For a conditional branch: times taken, and times it fell through.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub taken: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fell: u64,
    /// For a conditional branch right after a compare (`CMP`, `CPX`,
    /// `CPY`), counted only by [`run_segments`]: the compare's class each
    /// time the branch went the other way from its last run (a flip), a
    /// bit for each way and class. Bit `5 * way + class`, way 0 taken and
    /// 1 fell, class [`compare_class`]'s.
    #[serde(default, skip_serializing_if = "is_zero16")]
    pub flips: u16,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn is_zero16(n: &u16) -> bool {
    *n == 0
}

/// The classes of a compare: the register minus the operand, as numbers
/// 0 to 255, is below -1, -1, 0, +1 or above +1. The three middle ones are
/// a value one off its boundary or on it.
pub const COMPARE_CLASSES: [&str; 5] = ["<-1", "-1", "0", "+1", ">+1"];

/// The class of a compare (`CMP`, `CPX`, `CPY` in any mode) about to run
/// at `pc`, from the registers and memory as they are; `None` for any
/// other instruction. Memory is peeked, so nothing changes.
pub fn compare_class(bus: &mut tetanes_core::bus::Bus, op: u8, pc: u16) -> Option<u8> {
    let (a, x, y) = (bus.cpu.acc, bus.cpu.x, bus.cpu.y);
    if !matches!(
        op,
        0xC9 | 0xC5
            | 0xD5
            | 0xCD
            | 0xDD
            | 0xD9
            | 0xC1
            | 0xD1
            | 0xE0
            | 0xE4
            | 0xEC
            | 0xC0
            | 0xC4
            | 0xCC
    ) {
        return None;
    }
    let b = bus.peek(pc.wrapping_add(1));
    let w = u16::from_le_bytes([b, bus.peek(pc.wrapping_add(2))]);
    let zp_word = |z: u8| {
        u16::from_le_bytes([
            bus.peek(u16::from(z)),
            bus.peek(u16::from(z.wrapping_add(1))),
        ])
    };
    let at = match op {
        0xC9 | 0xE0 | 0xC0 => None,
        0xC5 | 0xE4 | 0xC4 => Some(u16::from(b)),
        0xD5 => Some(u16::from(b.wrapping_add(x))),
        0xCD | 0xEC | 0xCC => Some(w),
        0xDD => Some(w.wrapping_add(u16::from(x))),
        0xD9 => Some(w.wrapping_add(u16::from(y))),
        0xC1 => Some(zp_word(b.wrapping_add(x))),
        _ => Some(zp_word(b).wrapping_add(u16::from(y))),
    };
    let m = match at {
        Some(at) => bus.peek(at),
        None => b,
    };
    let reg = match op {
        0xE0 | 0xE4 | 0xEC => x,
        0xC0 | 0xC4 | 0xCC => y,
        _ => a,
    };
    Some(match i16::from(reg) - i16::from(m) {
        ..=-2 => 0,
        -1 => 1,
        0 => 2,
        1 => 3,
        _ => 4,
    })
}

/// The counts of a window of one run, or of several merged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Frames in the window(s).
    pub frames: u64,
    /// Instructions run, in PRG ROM or not.
    pub instructions: u64,
    /// Instructions run from outside PRG ROM (RAM).
    pub outside_rom: u64,
    /// By PRG offset, each instruction run.
    pub insns: BTreeMap<u32, InsnStat>,
}

/// Whether an opcode is a conditional branch.
pub fn is_branch(op: u8) -> bool {
    op & 0x1F == 0x10
}

impl Profile {
    /// Adds another window's counts (of the same ROM).
    pub fn merge(&mut self, o: &Profile) {
        self.frames += o.frames;
        self.instructions += o.instructions;
        self.outside_rom += o.outside_rom;
        for (&k, s) in &o.insns {
            let e = self.insns.entry(k).or_insert_with(|| InsnStat {
                cpu: s.cpu,
                ..InsnStat::default()
            });
            e.count += s.count;
            e.taken += s.taken;
            e.fell += s.fell;
            e.flips |= s.flips;
        }
    }
}

/// What the counter keeps while a run goes on: dense by PRG offset.
struct Counter {
    layout: Layout,
    count: Vec<u64>,
    cpu: Vec<u16>,
    taken: Vec<u64>,
    fell: Vec<u64>,
    instructions: u64,
    outside_rom: u64,
    /// The last instruction, when it was a conditional branch in PRG ROM:
    /// its offset and CPU address.
    branch: Option<(usize, u16)>,
    /// Whether to read compares' classes at branch flips (`flips`).
    values: bool,
    flips: Vec<u16>,
    /// Each branch's last way: 0 none yet, 1 taken, 2 fell.
    last_way: Vec<u8>,
    /// The class of the last instruction, when it was a compare.
    cmp_class: Option<u8>,
    /// The class of the compare just before the pending branch.
    branch_class: Option<u8>,
}

impl Counter {
    fn new(layout: Layout, values: bool) -> Counter {
        let n = layout.prg_rom;
        Counter {
            layout,
            count: vec![0; n],
            cpu: vec![0; n],
            taken: vec![0; n],
            fell: vec![0; n],
            instructions: 0,
            outside_rom: 0,
            branch: None,
            values,
            flips: if values { vec![0; n] } else { Vec::new() },
            last_way: if values { vec![0; n] } else { Vec::new() },
            cmp_class: None,
            branch_class: None,
        }
    }

    #[inline]
    fn before(&mut self, bus: &mut tetanes_core::bus::Bus) {
        let pc = bus.cpu.pc;
        // Where the last instruction went: here, or, after an interrupt,
        // the address the interrupt pushed.
        let went = match bus.interrupt_taken.take() {
            Some(FrameKind::Nmi | FrameKind::Irq) => {
                let sp = bus.cpu.sp;
                u16::from_le_bytes([
                    bus.peek(0x100 | u16::from(sp.wrapping_add(2))),
                    bus.peek(0x100 | u16::from(sp.wrapping_add(3))),
                ])
            }
            _ => pc,
        };
        if let Some((o, at)) = self.branch.take() {
            let fell = went == at.wrapping_add(2);
            if fell {
                self.fell[o] += 1;
            } else {
                self.taken[o] += 1;
            }
            if self.values {
                let way = if fell { 2 } else { 1 };
                let last = self.last_way[o];
                if last != 0
                    && last != way
                    && let Some(c) = self.branch_class
                {
                    self.flips[o] |= 1 << (5 * (way - 1) + c);
                }
                self.last_way[o] = way;
            }
        }
        let class_before = self.cmp_class.take();
        self.instructions += 1;
        match self.layout.rom_offset(bus, pc) {
            Some(o) => {
                if self.count[o] == 0 {
                    self.cpu[o] = pc;
                }
                self.count[o] += 1;
                let op = bus.peek(pc);
                if is_branch(op) {
                    self.branch = Some((o, pc));
                    self.branch_class = class_before;
                } else if self.values {
                    self.cmp_class = compare_class(bus, op, pc);
                }
            }
            None => self.outside_rom += 1,
        }
    }

    /// The counts so far as a profile of `frames` frames, and the counts
    /// set to zero for the next window; each branch's last way is kept.
    fn cut(&mut self, frames: u64) -> Profile {
        let mut insns = BTreeMap::new();
        for o in 0..self.count.len() {
            let n = self.count[o];
            if n == 0 {
                continue;
            }
            insns.insert(
                o as u32,
                InsnStat {
                    cpu: self.cpu[o],
                    count: n,
                    taken: self.taken[o],
                    fell: self.fell[o],
                    flips: self.flips.get(o).copied().unwrap_or(0),
                },
            );
            self.count[o] = 0;
            self.taken[o] = 0;
            self.fell[o] = 0;
            if let Some(f) = self.flips.get_mut(o) {
                *f = 0;
            }
        }
        let p = Profile {
            frames,
            instructions: self.instructions,
            outside_rom: self.outside_rom,
            insns,
        };
        self.instructions = 0;
        self.outside_rom = 0;
        p
    }

    fn finish(mut self, frames: u64) -> Profile {
        self.cut(frames)
    }
}

/// Runs `rom` from power-on through `inputs` and counts frames `first` to
/// `last` (frame 1 is the first frame run; `last` past the input is its
/// end). Frames before the window run unwatched. A branch that is the
/// window's last instruction is counted as run, but neither taken nor
/// fallen.
pub fn run_window(
    rom: &[u8],
    power_on: PowerOn,
    inputs: &[InputFrame],
    first: u64,
    last: u64,
) -> Result<Profile, RunError> {
    run_window_on(Nes::power_on(rom, power_on)?, 0, inputs, first, last)
}

/// The frames of `inputs` from `at` to the window's first, run unwatched
/// on `nes`, which stands after the first `at` of them (powered on for 0,
/// or restored from a snapshot there).
fn catch_up(nes: &mut Nes, at: usize, inputs: &[InputFrame], first: u64) -> Result<(), RunError> {
    let before = usize::try_from(first.max(1) - 1)
        .unwrap_or(usize::MAX)
        .min(inputs.len());
    if at > before {
        return Err(RunError::Start { at, first });
    }
    for i in &inputs[at..before] {
        nes.step(feed(i))?;
    }
    Ok(())
}

/// What the emulator is fed for an input frame: every frame polled, as a
/// replay of a log does.
fn feed(i: &InputFrame) -> InputFrame {
    InputFrame {
        polled: true,
        pads: i.pads,
        reset: i.reset,
    }
}

/// [`run_window`] on a console that stands after the first `at` frames
/// of `inputs` (a start snapshot restored), `at` no later than the
/// window's first frame less one. The same profile as from power-on.
pub fn run_window_on(
    mut nes: Nes,
    at: usize,
    inputs: &[InputFrame],
    first: u64,
    last: u64,
) -> Result<Profile, RunError> {
    let layout = Layout::of(nes.bus())?;
    let n = inputs.len() as u64;
    let first = first.max(1);
    let last = last.min(n);
    catch_up(&mut nes, at, inputs, first)?;
    let mut c = Counter::new(layout, false);
    nes.bus_mut_for_observers().interrupt_taken = None;
    let mut frames = 0;
    for f in first..=last {
        let i = feed(&inputs[usize::try_from(f - 1).unwrap_or(usize::MAX)]);
        if i.reset {
            c.branch = None;
        }
        nes.step_with(i, |bus| c.before(bus))?;
        frames += 1;
    }
    Ok(c.finish(frames))
}

/// The version of what [`run_segments`] gives, for the store.
pub const SEGMENTS_VERSION: u32 = 1;

/// One stretch of a window over which a RAM byte held one value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// The byte's value as each frame of the stretch began.
    pub value: u8,
    pub first: u64,
    pub last: u64,
    pub profile: Profile,
}

/// Counts frames `first` to `last` of a run as [`run_window`] does, cut
/// into stretches where RAM byte `byte` changes: a frame belongs to the
/// value the byte holds as it begins. Each branch's flips (see
/// [`InsnStat::flips`]) are counted too, the last way carried across
/// the cuts.
pub fn run_segments(
    rom: &[u8],
    power_on: PowerOn,
    inputs: &[InputFrame],
    first: u64,
    last: u64,
    byte: u16,
) -> Result<Vec<Segment>, RunError> {
    run_segments_on(Nes::power_on(rom, power_on)?, 0, inputs, first, last, byte)
}

/// [`run_segments`] on a console that stands after the first `at` frames
/// of `inputs`, as [`run_window_on`].
pub fn run_segments_on(
    mut nes: Nes,
    at: usize,
    inputs: &[InputFrame],
    first: u64,
    last: u64,
    byte: u16,
) -> Result<Vec<Segment>, RunError> {
    let layout = Layout::of(nes.bus())?;
    let n = inputs.len() as u64;
    let first = first.max(1);
    let last = last.min(n);
    catch_up(&mut nes, at, inputs, first)?;
    let mut c = Counter::new(layout, true);
    nes.bus_mut_for_observers().interrupt_taken = None;
    let mut out = Vec::new();
    let mut open: Option<(u8, u64)> = None;
    for f in first..=last {
        let v = nes.peek(byte);
        match open {
            Some((w, from)) if w != v => {
                out.push(Segment {
                    value: w,
                    first: from,
                    last: f - 1,
                    profile: c.cut(f - from),
                });
                open = Some((v, f));
            }
            None => open = Some((v, f)),
            _ => {}
        }
        let i = feed(&inputs[usize::try_from(f - 1).unwrap_or(usize::MAX)]);
        if i.reset {
            c.branch = None;
        }
        nes.step_with(i, |bus| c.before(bus))?;
    }
    if let Some((w, from)) = open {
        out.push(Segment {
            value: w,
            first: from,
            last,
            profile: c.cut(last + 1 - from),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up NROM program: the reset loops forever counting X down from
    /// 3 with a BNE (taken twice, then falling once), over and over; the
    /// NMI handler returns at once. The vectors point at both.
    fn rom() -> Vec<u8> {
        let mut prg = vec![0xEAu8; 0x4000];
        // C000: LDX #$03; C002: DEX; C003: BNE $C002; C005: JMP $C000
        prg[..8].copy_from_slice(&[0xA2, 0x03, 0xCA, 0xD0, 0xFD, 0x4C, 0x00, 0xC0]);
        // C010: RTI
        prg[0x10] = 0x40;
        // NMI $C010, reset $C000, IRQ $C010.
        prg[0x3FFA..].copy_from_slice(&[0x10, 0xC0, 0x00, 0xC0, 0x10, 0xC0]);
        let mut rom = b"NES\x1A\x01\x01\x00\x00".to_vec();
        rom.resize(16, 0);
        rom.extend(prg);
        rom.extend(vec![0u8; 0x2000]);
        rom
    }

    #[test]
    fn a_loop_counts_its_branch_both_ways() {
        let rom = rom();
        let inputs = vec![InputFrame::default(); 3];
        let p = run_window(&rom, PowerOn::default(), &inputs, 1, 3).unwrap();
        assert_eq!(p.frames, 3);
        let ldx = &p.insns[&0];
        let dex = &p.insns[&2];
        let bne = &p.insns[&3];
        assert_eq!(ldx.cpu, 0xC000);
        // Each pass of the loop: LDX once, DEX and BNE three times each,
        // BNE taken twice and falling once into the JMP. The window can
        // end inside a pass.
        let jmp = &p.insns[&5];
        assert!(dex.count.abs_diff(3 * ldx.count) <= 3, "{dex:?} {ldx:?}");
        assert!(bne.count.abs_diff(dex.count) <= 1);
        let done = bne.taken + bne.fell;
        assert!(done + 1 >= bne.count && done <= bne.count, "{bne:?}");
        assert!(bne.fell.abs_diff(jmp.count) <= 1, "{bne:?} {jmp:?}");
        assert!(bne.taken.abs_diff(2 * bne.fell) <= 2, "{bne:?}");
    }

    #[test]
    fn a_window_counts_only_its_frames_and_windows_add_up() {
        let rom = rom();
        let inputs = vec![InputFrame::default(); 4];
        let all = run_window(&rom, PowerOn::default(), &inputs, 1, 4).unwrap();
        let mut parts = run_window(&rom, PowerOn::default(), &inputs, 1, 2).unwrap();
        parts.merge(&run_window(&rom, PowerOn::default(), &inputs, 3, 4).unwrap());
        assert_eq!(parts.frames, 4);
        assert_eq!(parts.instructions, all.instructions);
        assert_eq!(parts.insns[&2].count, all.insns[&2].count);
        // A branch at the window's end goes uncounted either way, so the
        // two halves agree with the whole within that one branch.
        let (a, b) = (&parts.insns[&3], &all.insns[&3]);
        assert!(a.taken + a.fell + 1 >= b.taken + b.fell);
    }

    /// A window counted on a console restored from a snapshot taken
    /// before it is the window counted from power-on; a console past the
    /// window's start is refused.
    #[test]
    fn a_window_on_a_restored_console_is_the_same() {
        let rom = rom();
        let inputs = vec![InputFrame::default(); 9];
        let plain = run_window(&rom, PowerOn::default(), &inputs, 6, 9).unwrap();
        let segs = run_segments(&rom, PowerOn::default(), &inputs, 6, 9, 0x00).unwrap();
        for at in [0usize, 3, 5] {
            let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
            for i in &inputs[..at] {
                nes.step(feed(i)).unwrap();
            }
            let (snap, held) = (nes.snapshot().unwrap(), nes.held());
            let restored = || {
                let mut n = Nes::power_on(&rom, PowerOn::default()).unwrap();
                n.restore(&snap, at as u64, held).unwrap();
                n
            };
            assert_eq!(run_window_on(restored(), at, &inputs, 6, 9).unwrap(), plain);
            assert_eq!(
                run_segments_on(restored(), at, &inputs, 6, 9, 0x00).unwrap(),
                segs
            );
        }
        let nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        assert!(run_window_on(nes, 7, &inputs, 6, 9).is_err());
    }
}
