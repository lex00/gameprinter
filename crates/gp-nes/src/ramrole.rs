//! Each RAM byte's role, from a trace of the original (docs/progress.md,
//! "The memory map's ignore list"): which bytes hold the game's state and
//! which hold something a model doesn't need, such as the controller's
//! bits, a frame counter, a scratch value or the sound driver's own state.
//!
//! [`Tracer`] watches the CPU between instructions through
//! [`crate::Nes::step_with`], as [`crate::coverage`] does, and changes
//! nothing the console does. It decodes each instruction's operand to the
//! RAM byte it reads or writes, keeps a call stack from `JSR`, the
//! interrupts and the stack pointer, and gives each access to
//! [`RamTrace`]. A test gives [`RamTrace`] a made-up trace directly.
//!
//! [`RamTrace::classify`] gives every byte one [`Role`], by these rules in
//! this order:
//!
//! | Role | Rule |
//! |---|---|
//! | `sound` | Only the sound routines touch it, or (flagged for review) only they read it. A sound routine is one that writes an APU register, directly or through its calls, and never a PPU register; a routine only sound routines call is one too |
//! | `input` | Written by a routine that reads the controller and writes no PPU or APU register, or its value is a mask of such a byte on nearly every frame it is written |
//! | `counter` | Nearly every change is +1, or nearly every one is -1, it changes on at least half the frames it runs (a pause stops one), and it wraps around at least twice |
//! | `scratch` | Never live across frames: no frame reads it before it writes it, or (flagged) only a few do, or what they read is always one value the frame before left. A byte written and never read is scratch too, flagged |
//! | `pointer` | Scratch that the code uses as a pointer (`($nn),Y`, `($nn,X)`, `JMP ($nnnn)`) or loads into X or Y to index with |
//! | `state` | The rest: live across frames and read by the game's own code |
//!
//! A rule that holds on nearly every frame but not all says so with
//! `review`, for a person to check.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tetanes_core::bus::Bus;
use tetanes_core::cpu::Cpu;
use tetanes_core::cpu::instr::{AddrMode, Instr};

/// RAM's size: $0000-$07FF, mirrored up to $1FFF.
pub const RAM: usize = 0x800;

/// A routine's key: its first address, or [`ROOT`] for code outside any
/// call (the reset code's main loop).
pub type Routine = u32;
pub const ROOT: Routine = 0x1_0000;

/// What a byte holds, by how the code uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Input,
    Counter,
    Scratch,
    Sound,
    Pointer,
    State,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Input => "input",
            Role::Counter => "counter",
            Role::Scratch => "scratch",
            Role::Sound => "sound",
            Role::Pointer => "pointer",
            Role::State => "state",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Role::Input,
            Role::Counter,
            Role::Scratch,
            Role::Sound,
            Role::Pointer,
            Role::State,
        ]
        .into_iter()
        .find(|r| r.name() == s)
    }
}

/// One byte's role and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RamRole {
    pub addr: u16,
    pub role: Role,
    pub why: String,
    /// The rule held on nearly every frame but not all: a person checks it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub review: bool,
}

/// What the classifier found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classified {
    /// Every RAM byte the trace touched, in address order.
    pub roles: Vec<RamRole>,
    /// The sound routines that no other sound routine calls: the sound
    /// driver's entry points.
    pub sound_entries: Vec<Routine>,
    /// The routines that read the controller.
    pub input_routines: Vec<Routine>,
}

/// The input rule's mask holds on at least this share of the frames a
/// byte is written, in percent, with `review` below all of them.
const NEARLY: u64 = 99;
/// A scratch byte may be read before it is written on at most this share
/// of the frames it is read, in percent (a lag frame splits a frame's work
/// in two), and is flagged when it is.
const SCRATCH_SLACK: u64 = 1;
/// A counter changes on at least this share of the frames between its
/// first change and its last, in percent (a pause stops one), and at
/// least [`COUNTER_STEPS`] percent of its changes are a step of one.
const COUNTER_EVERY: u64 = 50;
const COUNTER_STEPS: u64 = 99;
/// And it wraps around at least this often: a place or a timer that steps
/// by one stops short of that.
const COUNTER_WRAPS: u32 = 2;
/// And at least this many times.
const COUNTER_MIN: u32 = 100;
/// A pointer's uses as a pointer or index, as a share of its reads, in
/// percent.
const POINTER_USES: u64 = 25;

#[derive(Debug, Clone, Default)]
struct Byte {
    reads: u64,
    writes: u64,
    /// The last frame it was touched, read and written (frame + 1, so 0
    /// is never).
    touched: u32,
    read_in: u32,
    written_in: u32,
    frames_read: u32,
    frames_written: u32,
    /// Frames whose first touch was a read of a value an earlier frame
    /// wrote, and the values those reads saw (a bit each).
    carried: u32,
    carried_values: [u64; 4],
    uses: u64,
    /// End-of-frame changes, and those of +1 and of -1.
    changes: u32,
    up: u32,
    down: u32,
    /// Steps from $FF to $00 or from $00 to $FF.
    wraps: u32,
    first_change: u32,
    last_change: u32,
    /// The last routine to read it and to write it, to skip the set.
    last_reader: Option<Routine>,
    last_writer: Option<Routine>,
}

/// The accesses of a trace, byte by byte, and the routines behind them.
#[derive(Debug, Clone)]
pub struct RamTrace {
    bytes: Vec<Byte>,
    /// (byte, routine, write) for each routine that touched a byte.
    touch: HashSet<(u16, Routine, bool)>,
    /// The call stack, innermost last.
    stack: Vec<Routine>,
    /// Callee -> callers.
    callers: HashMap<Routine, BTreeSet<Routine>>,
    /// Routines on the stack when an APU register was written, and when a
    /// PPU register was.
    apu: HashSet<Routine>,
    other_io: HashSet<Routine>,
    /// Routines that read the controller (the innermost at the read).
    input: BTreeSet<Routine>,
    /// The frame being traced, from 0.
    frame: u32,
    /// Bytes written this frame.
    written: Vec<u16>,
    /// The value at the end of the frame before.
    last: Vec<u8>,
    /// Per byte written by an input routine this frame.
    by_input: Vec<bool>,
    /// The bytes an input routine writes.
    input_bytes: BTreeSet<u16>,
    /// For the input rule: each byte's end values on the frames it is
    /// written (a bit for each value seen), and against each input byte's
    /// value then, how often each pair of values came.
    seen: Vec<[u64; 4]>,
    pairs: HashMap<(u16, u16), HashMap<(u8, u8), u32>>,
    frames: u32,
}

impl Default for RamTrace {
    fn default() -> Self {
        Self::new()
    }
}

impl RamTrace {
    pub fn new() -> Self {
        Self {
            bytes: vec![Byte::default(); RAM],
            touch: HashSet::new(),
            stack: Vec::new(),
            callers: HashMap::new(),
            apu: HashSet::new(),
            other_io: HashSet::new(),
            input: BTreeSet::new(),
            frame: 0,
            written: Vec::new(),
            last: vec![0; RAM],
            by_input: vec![false; RAM],
            input_bytes: BTreeSet::new(),
            seen: vec![[0; 4]; RAM],
            pairs: HashMap::new(),
            frames: 0,
        }
    }

    /// The routine running now.
    pub fn routine(&self) -> Routine {
        self.stack.last().copied().unwrap_or(ROOT)
    }

    /// A frame starts.
    pub fn frame_start(&mut self) {}

    /// A call to `target` (a `JSR`, or an interrupt's handler).
    pub fn call(&mut self, target: Routine) {
        let from = self.routine();
        self.callers.entry(target).or_default().insert(from);
        self.stack.push(target);
    }

    /// The innermost call returns.
    pub fn ret(&mut self) {
        self.stack.pop();
    }

    /// A jump through a pointer (`JMP ($nnnn)`): the innermost call goes
    /// on as `target`, as a jump table dispatches.
    pub fn jump(&mut self, target: Routine) {
        let from = self
            .stack
            .len()
            .checked_sub(2)
            .map_or(ROOT, |i| self.stack[i]);
        self.callers.entry(target).or_default().insert(from);
        match self.stack.last_mut() {
            Some(t) => *t = target,
            None => self.stack.push(target),
        }
    }

    /// The stack's depth, for the tracer to pop to.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    pub fn read(&mut self, addr: u16) {
        let a = usize::from(addr) & (RAM - 1);
        let f = self.frame + 1;
        let r = self.routine();
        let b = &mut self.bytes[a];
        b.reads += 1;
        if b.touched != f {
            b.touched = f;
            if b.writes > 0 {
                b.carried += 1;
                // Nothing wrote it this frame: it holds last frame's end.
                let v = self.last[a];
                b.carried_values[usize::from(v >> 6)] |= 1 << (v & 63);
            }
        }
        if b.read_in != f {
            b.read_in = f;
            b.frames_read += 1;
        }
        if b.last_reader != Some(r) {
            b.last_reader = Some(r);
            self.touch.insert((a as u16, r, false));
        }
    }

    pub fn write(&mut self, addr: u16) {
        let a = usize::from(addr) & (RAM - 1);
        let f = self.frame + 1;
        let r = self.routine();
        let b = &mut self.bytes[a];
        b.writes += 1;
        b.touched = f;
        if b.written_in != f {
            b.written_in = f;
            b.frames_written += 1;
            self.written.push(a as u16);
        }
        if b.last_writer != Some(r) {
            b.last_writer = Some(r);
            self.touch.insert((a as u16, r, true));
        }
        if self.input.contains(&r) && !self.other_io.contains(&r) && !self.apu.contains(&r) {
            self.by_input[a] = true;
        }
    }

    /// The byte's value is used as a pointer or an index.
    pub fn used(&mut self, addr: u16) {
        self.bytes[usize::from(addr) & (RAM - 1)].uses += 1;
    }

    /// An APU register is written.
    pub fn apu_write(&mut self) {
        self.apu.insert(ROOT);
        self.apu.extend(self.stack.iter().copied());
    }

    /// A PPU register is written (or OAM DMA started).
    pub fn other_io_write(&mut self) {
        self.other_io.insert(ROOT);
        self.other_io.extend(self.stack.iter().copied());
    }

    /// The controller is read.
    pub fn controller_read(&mut self) {
        let r = self.routine();
        self.input.insert(r);
    }

    /// The frame ends with RAM as `ram`.
    pub fn frame_end(&mut self, ram: &[u8]) {
        let f = self.frame;
        for &a in &self.written {
            let i = usize::from(a);
            if self.by_input[i] {
                self.input_bytes.insert(a);
                self.by_input[i] = false;
            }
        }
        for a in self.written.drain(..) {
            let i = usize::from(a);
            let v = ram[i];
            self.seen[i][usize::from(v >> 6)] |= 1 << (v & 63);
            if self.input_bytes.contains(&a) {
                continue;
            }
            for &k in &self.input_bytes {
                *self
                    .pairs
                    .entry((a, k))
                    .or_default()
                    .entry((ram[usize::from(k)], v))
                    .or_default() += 1;
            }
        }
        for (i, (&now, was)) in ram.iter().zip(self.last.iter_mut()).enumerate().take(RAM) {
            if now != *was {
                let b = &mut self.bytes[i];
                if b.changes == 0 {
                    b.first_change = f;
                }
                b.changes += 1;
                b.last_change = f;
                match now.wrapping_sub(*was) {
                    1 => b.up += 1,
                    0xFF => b.down += 1,
                    _ => {}
                }
                if (*was == 0xFF && now == 0) || (*was == 0 && now == 0xFF) {
                    b.wraps += 1;
                }
                *was = now;
            }
        }
        self.frame += 1;
        self.frames += 1;
    }

    /// Each byte's role.
    pub fn classify(&self) -> Classified {
        let sound = self.sound_routines();
        let mut roles = Vec::new();
        let mut readers: BTreeMap<u16, Vec<Routine>> = BTreeMap::new();
        let mut writers: BTreeMap<u16, Vec<Routine>> = BTreeMap::new();
        for &(a, r, w) in &self.touch {
            if w {
                writers.entry(a).or_default().push(r);
            } else {
                readers.entry(a).or_default().push(r);
            }
        }
        for (i, b) in self.bytes.iter().enumerate() {
            if b.reads == 0 && b.writes == 0 {
                continue;
            }
            let addr = i as u16;
            let rd = readers.get(&addr).map_or(&[][..], Vec::as_slice);
            let wr = writers.get(&addr).map_or(&[][..], Vec::as_slice);
            let role = |role, why: String, review| RamRole {
                addr,
                role,
                why,
                review,
            };
            // Sound.
            let all_sound = |rs: &[Routine]| !rs.is_empty() && rs.iter().all(|r| sound.contains(r));
            if !sound.is_empty() && all_sound(rd) && (wr.is_empty() || all_sound(wr)) {
                roles.push(role(
                    Role::Sound,
                    "only the sound routines touch it".into(),
                    false,
                ));
                continue;
            }
            if !sound.is_empty() && all_sound(rd) && !wr.is_empty() {
                roles.push(role(
                    Role::Sound,
                    "the game writes it and only the sound routines read it: a sound request"
                        .into(),
                    true,
                ));
                continue;
            }
            // Input.
            if self.input_bytes.contains(&addr) {
                roles.push(role(
                    Role::Input,
                    "written by the routine that reads the controller".into(),
                    false,
                ));
                continue;
            }
            if let Some((share, from)) = self.mask_of_input(i) {
                roles.push(role(
                    Role::Input,
                    format!("its value is the controller's bits from ${from:04X}, masked, on {share}% of the frames it is written"),
                    share < 100,
                ));
                continue;
            }
            // A counter.
            let span = u64::from(b.last_change - b.first_change + 1);
            let steady = u64::from(b.changes) * 100 >= span * COUNTER_EVERY;
            let steps = u64::from(b.up.max(b.down));
            if b.changes >= COUNTER_MIN
                && steady
                && b.wraps >= COUNTER_WRAPS
                && steps * 100 >= u64::from(b.changes) * COUNTER_STEPS
            {
                roles.push(role(
                    Role::Counter,
                    format!(
                        "counts {} by one on {steps} of its {} changes, over {span} frames",
                        if b.up >= b.down { "up" } else { "down" },
                        b.changes
                    ),
                    steps < u64::from(b.changes),
                ));
                continue;
            }
            // Scratch and pointers.
            if b.reads == 0 {
                roles.push(role(
                    Role::Scratch,
                    "written but never read in the trace".into(),
                    true,
                ));
                continue;
            }
            let carried = u64::from(b.carried);
            let kept: u32 = b.carried_values.iter().map(|w| w.count_ones()).sum();
            let few = carried * 100 <= u64::from(b.frames_read) * SCRATCH_SLACK;
            if few || kept == 1 {
                let why = if carried == 0 {
                    "no frame reads it before writing it".to_owned()
                } else if few {
                    format!(
                        "read before it is written on {carried} of the {} frames it is read",
                        b.frames_read
                    )
                } else {
                    let v = (0..=255u8)
                        .find(|v| b.carried_values[usize::from(v >> 6)] >> (v & 63) & 1 == 1)
                        .unwrap_or(0);
                    format!(
                        "what a frame reads before writing it is always ${v:02X}, the value the frame before left"
                    )
                };
                let pointer = b.uses * 100 >= b.reads * POINTER_USES;
                roles.push(if pointer {
                    role(
                        Role::Pointer,
                        format!("used as a pointer or an index; {why}"),
                        carried > 0,
                    )
                } else {
                    role(Role::Scratch, why, carried > 0)
                });
                continue;
            }
            roles.push(role(
                Role::State,
                format!(
                    "live across frames: read before it is written on {carried} of the {} frames it is read",
                    b.frames_read
                ),
                false,
            ));
        }
        let entries = sound
            .iter()
            .copied()
            .filter(|r| {
                self.callers
                    .get(r)
                    .is_none_or(|c| c.iter().all(|c| !sound.contains(c)))
            })
            .collect();
        Classified {
            roles,
            sound_entries: entries,
            input_routines: self.input.iter().copied().collect(),
        }
    }

    /// When byte `i`'s value is an input byte's masked, on at least
    /// [`NEARLY`] percent of the frames it is written: the share and that
    /// input byte. The mask is every bit the byte is ever seen with.
    fn mask_of_input(&self, i: usize) -> Option<(u64, u16)> {
        let seen = &self.seen[i];
        let distinct: u32 = seen.iter().map(|w| w.count_ones()).sum();
        let written = u64::from(self.bytes[i].frames_written);
        if distinct < 2 || written < 10 {
            return None;
        }
        let mask = (0..=255u8)
            .filter(|v| seen[usize::from(v >> 6)] >> (v & 63) & 1 == 1)
            .fold(0u8, |m, v| m | v);
        let mut best: Option<(u64, u16)> = None;
        for &k in &self.input_bytes {
            let Some(p) = self.pairs.get(&(i as u16, k)) else {
                continue;
            };
            // A copy of the controller follows it: it is written at least a
            // quarter as often as the byte it copies. A byte written a few
            // times (a count of stomps) can agree with a held button by
            // chance.
            let of_k = self
                .bytes
                .get(usize::from(k))
                .map_or(0, |b| u64::from(b.frames_written));
            if written * 4 < of_k {
                continue;
            }
            let all: u64 = p.values().map(|&n| u64::from(n)).sum();
            let agree: u64 = p
                .iter()
                .filter(|((x, v), _)| x & mask == *v)
                .map(|(_, &n)| u64::from(n))
                .sum();
            // The controller's bits must vary for the mask to say anything.
            let varied: BTreeSet<u8> = p
                .keys()
                .filter(|(x, v)| x & mask == *v)
                .map(|(_, v)| *v)
                .collect();
            if all == 0 || all * 2 < written || varied.len() < 2 {
                continue;
            }
            let share = agree * 100 / all;
            if share >= NEARLY && best.is_none_or(|b| share > b.0) {
                best = Some((share, k));
            }
        }
        best
    }

    /// The sound routines: those that write an APU register (directly or
    /// in a call) and never a PPU register, and the routines
    /// only sound routines call.
    pub fn sound_routines(&self) -> BTreeSet<Routine> {
        let mut s: BTreeSet<Routine> = self
            .apu
            .iter()
            .copied()
            .filter(|r| !self.other_io.contains(r))
            .collect();
        if s.is_empty() {
            return s;
        }
        loop {
            let more: Vec<Routine> = self
                .callers
                .iter()
                .filter(|(callee, from)| {
                    !s.contains(*callee)
                        && !self.other_io.contains(*callee)
                        && from.iter().all(|c| s.contains(c))
                })
                .map(|(c, _)| *c)
                .collect();
            if more.is_empty() {
                return s;
            }
            s.extend(more);
        }
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }
}

/// How an instruction touches memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Touch {
    None,
    Read,
    Write,
    Modify,
}

#[derive(Debug, Clone, Copy)]
struct Op {
    instr: Instr,
    mode: AddrMode,
    touch: Touch,
}

fn ops() -> [Op; 256] {
    std::array::from_fn(|i| {
        let r = Cpu::INSTR_REF[i];
        use Instr::*;
        let touch = match r.addr_mode {
            AddrMode::IMM | AddrMode::REL | AddrMode::IMP | AddrMode::ACC | AddrMode::OTH => {
                Touch::None
            }
            _ => match r.instr {
                ADC | AND | BIT | CMP | CPX | CPY | EOR | LDA | LDX | LDY | ORA | SBC | LAX
                | LAS => Touch::Read,
                STA | STX | STY | SAX | AHX => Touch::Write,
                ASL | LSR | ROL | ROR | INC | DEC | SLO | RLA | SRE | RRA | DCP | ISB => {
                    Touch::Modify
                }
                _ => Touch::None,
            },
        };
        Op {
            instr: r.instr,
            mode: r.addr_mode,
            touch,
        }
    })
}

/// Where a register's value came from: the RAM byte it was loaded from.
type Src = Option<u16>;

/// Watches one run's instructions and gives [`RamTrace`] each access. Give
/// it the bus before each instruction, and call [`Tracer::frame_start`]
/// and [`Tracer::frame_end`] around each frame.
#[derive(Debug, Clone)]
pub struct Tracer {
    pub trace: RamTrace,
    ops: [Op; 256],
    /// Each open call's stack pointer from before it: the call has
    /// returned once the stack pointer is back at or above it.
    sp: Vec<u8>,
    a: Src,
    x: Src,
    y: Src,
    started: bool,
}

impl Default for Tracer {
    fn default() -> Self {
        Self::new()
    }
}

fn ram(addr: u16) -> Option<u16> {
    (addr < 0x2000).then_some(addr & 0x7FF)
}

impl Tracer {
    pub fn new() -> Self {
        Self {
            trace: RamTrace::new(),
            ops: ops(),
            sp: Vec::new(),
            a: None,
            x: None,
            y: None,
            started: false,
        }
    }

    pub fn frame_start(&mut self, bus: &mut Bus) {
        if !self.started {
            // Stale until the next interrupt.
            bus.interrupt_taken = None;
            self.started = true;
        }
        self.trace.frame_start();
    }

    pub fn frame_end(&mut self, bus: &mut Bus) {
        let ram: Vec<u8> = (0..RAM as u16).map(|a| bus.peek(a)).collect();
        self.trace.frame_end(&ram);
    }

    fn word(bus: &Bus, lo: u16, hi: u16) -> u16 {
        u16::from(bus.peek(lo)) | u16::from(bus.peek(hi)) << 8
    }

    /// Notes the instruction about to run.
    #[inline]
    pub fn before(&mut self, bus: &mut Bus) {
        let interrupted = bus.interrupt_taken.take().is_some();
        self.observe(bus, interrupted);
    }

    /// [`Tracer::before`] for a caller that took `Bus::interrupt_taken`
    /// itself (gp-events' collector, which reads it once for every
    /// observer): `interrupted` is whether an interrupt was taken after the
    /// last instruction.
    #[inline]
    pub fn observe(&mut self, bus: &Bus, interrupted: bool) {
        let sp = bus.cpu.sp;
        // Calls that have returned (RTS, RTI, or a routine that pulled its
        // own return address, as a jump table does).
        while let Some(&s) = self.sp.last() {
            if s <= sp {
                self.sp.pop();
                self.trace.ret();
            } else {
                break;
            }
        }
        let pc = bus.cpu.pc;
        if interrupted {
            self.sp.push(sp.wrapping_add(3));
            self.trace.call(Routine::from(pc));
        }
        let op = self.ops[usize::from(bus.peek(pc))];
        let b1 = bus.peek(pc.wrapping_add(1));
        let b2 = bus.peek(pc.wrapping_add(2));
        let abs = u16::from(b1) | u16::from(b2) << 8;
        let (x, y) = (bus.cpu.x, bus.cpu.y);
        // The effective address, and the register that indexed it.
        let (addr, by) = match op.mode {
            AddrMode::ZP0 => (Some(u16::from(b1)), None),
            AddrMode::ZPX => (Some(u16::from(b1.wrapping_add(x))), Some('x')),
            AddrMode::ZPY => (Some(u16::from(b1.wrapping_add(y))), Some('y')),
            AddrMode::ABS => (Some(abs), None),
            AddrMode::ABX | AddrMode::ABXW => (Some(abs.wrapping_add(x.into())), Some('x')),
            AddrMode::ABY | AddrMode::ABYW => (Some(abs.wrapping_add(y.into())), Some('y')),
            AddrMode::IDX => {
                let p = b1.wrapping_add(x);
                let q = p.wrapping_add(1);
                for z in [p, q] {
                    self.trace.read(z.into());
                    self.trace.used(z.into());
                }
                (Some(Self::word(bus, p.into(), q.into())), Some('x'))
            }
            AddrMode::IDY | AddrMode::IDYW => {
                let q = b1.wrapping_add(1);
                for z in [b1, q] {
                    self.trace.read(z.into());
                    self.trace.used(z.into());
                }
                let base = Self::word(bus, b1.into(), q.into());
                (Some(base.wrapping_add(y.into())), Some('y'))
            }
            AddrMode::IND => {
                // JMP ($nnnn), with the 6502's page wrap.
                let hi = (abs & 0xFF00) | (abs.wrapping_add(1) & 0x00FF);
                for z in [abs, hi] {
                    if let Some(r) = ram(z) {
                        self.trace.read(r);
                        self.trace.used(r);
                    }
                }
                self.trace.jump(Routine::from(Self::word(bus, abs, hi)));
                (None, None)
            }
            _ => (None, None),
        };
        // The index register's source is used as an index.
        let src = match by {
            Some('x') => self.x,
            Some('y') => self.y,
            _ => None,
        };
        if let (Some(s), Some(_)) = (src, addr) {
            self.trace.used(s);
        }
        if let Some(a) = addr {
            match ram(a) {
                Some(r) => match op.touch {
                    Touch::Read => self.trace.read(r),
                    Touch::Write => self.trace.write(r),
                    Touch::Modify => {
                        self.trace.read(r);
                        self.trace.write(r);
                    }
                    Touch::None => {}
                },
                None => {
                    let writes = matches!(op.touch, Touch::Write | Touch::Modify);
                    if writes && ((0x4000..=0x4013).contains(&a) || a == 0x4015 || a == 0x4017) {
                        self.trace.apu_write();
                    } else if writes && ((0x2000..0x4000).contains(&a) || a == 0x4014) {
                        self.trace.other_io_write();
                    } else if op.touch == Touch::Read && (a == 0x4016 || a == 0x4017) {
                        self.trace.controller_read();
                    }
                }
            }
        }
        // Where each register's value now comes from.
        let from = addr.and_then(ram);
        use Instr::*;
        match op.instr {
            LDA => self.a = from,
            LDX => self.x = from,
            LDY => self.y = from,
            LAX => (self.a, self.x) = (from, from),
            TAX => self.x = self.a,
            TAY => self.y = self.a,
            TXA => self.a = self.x,
            TYA => self.a = self.y,
            TSX => self.x = None,
            PLA => self.a = None,
            JSR => {
                self.sp.push(sp);
                self.trace.call(Routine::from(abs));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up game: a main routine that reads the controller into $10
    /// and masks it into $11, keeps the player's place in $20 (live across
    /// frames), works in $30 (written then read in one frame), indexes
    /// with $31, counts frames in $40, and a sound routine that keeps $50
    /// to itself and reads the request in $51.
    fn game(frames: u32) -> RamTrace {
        let mut t = RamTrace::new();
        let mut ram = vec![0u8; RAM];
        for f in 0..frames {
            let pad = (f * 7 % 256) as u16;
            t.frame_start();
            t.call(0x8000); // the NMI
            t.other_io_write();
            t.call(0x8100); // reads the pads
            t.controller_read();
            t.write(0x10);
            ram[0x10] = pad as u8;
            t.ret();
            t.call(0x8200); // the game
            t.read(0x10);
            t.write(0x11);
            ram[0x11] = pad as u8 & 0xC0;
            t.read(0x20);
            t.write(0x20);
            ram[0x20] = ram[0x20].wrapping_add(pad as u8 & 1);
            t.write(0x30);
            t.read(0x30);
            t.write(0x31);
            t.read(0x31);
            t.used(0x31);
            t.read(0x40);
            t.write(0x40);
            ram[0x40] = ram[0x40].wrapping_add(1);
            if f % 50 == 0 {
                t.write(0x51);
            }
            t.write(0x60);
            t.ret();
            t.call(0x9000); // the sound driver
            t.read(0x51);
            t.read(0x50);
            t.write(0x50);
            ram[0x50] = (f % 3) as u8;
            t.call(0x9100);
            t.apu_write();
            t.read(0x52);
            t.ret();
            t.ret();
            t.ret();
            t.frame_end(&ram);
        }
        t
    }

    fn role(c: &Classified, a: u16) -> &RamRole {
        c.roles.iter().find(|r| r.addr == a).unwrap()
    }

    #[test]
    fn each_byte_gets_its_role_from_a_made_up_trace() {
        let c = game(600).classify();
        assert_eq!(role(&c, 0x10).role, Role::Input);
        assert_eq!(role(&c, 0x11).role, Role::Input, "{:?}", role(&c, 0x11));
        assert!(!role(&c, 0x11).review);
        assert_eq!(role(&c, 0x20).role, Role::State);
        assert_eq!(role(&c, 0x30).role, Role::Scratch);
        assert!(!role(&c, 0x30).review);
        assert_eq!(role(&c, 0x31).role, Role::Pointer);
        assert_eq!(role(&c, 0x40).role, Role::Counter, "{:?}", role(&c, 0x40));
        assert_eq!(role(&c, 0x50).role, Role::Sound);
        assert!(!role(&c, 0x50).review);
        // Written by the game, read only by the sound driver: flagged.
        assert_eq!(role(&c, 0x51).role, Role::Sound);
        assert!(role(&c, 0x51).review);
        // Read only by a routine only the driver calls.
        assert_eq!(role(&c, 0x52).role, Role::Sound);
        // Written and never read.
        assert_eq!(role(&c, 0x60).role, Role::Scratch);
        assert!(role(&c, 0x60).review);
        assert_eq!(c.sound_entries, vec![0x9000]);
        assert_eq!(c.input_routines, vec![0x8100]);
        // The same trace, the same roles.
        assert_eq!(game(600).classify(), c);
    }

    #[test]
    fn a_byte_read_before_written_on_a_few_frames_is_flagged() {
        let mut t = RamTrace::new();
        let ram = vec![0u8; RAM];
        for f in 0..400 {
            t.frame_start();
            if f % 200 == 7 {
                t.read(0x30);
            }
            t.write(0x30);
            t.read(0x30);
            t.frame_end(&ram);
        }
        let c = t.classify();
        let r = role(&c, 0x30);
        assert_eq!((r.role, r.review), (Role::Scratch, true), "{r:?}");
        // On more than one in a hundred, with values that vary, it is
        // state.
        let mut t = RamTrace::new();
        let mut ram = vec![0u8; RAM];
        for f in 0..400 {
            t.frame_start();
            if f % 10 == 0 {
                t.read(0x30);
            }
            t.write(0x30);
            ram[0x30] = f as u8;
            t.frame_end(&ram);
        }
        assert_eq!(role(&t.classify(), 0x30).role, Role::State);
        // Read first on every frame, but always the one value a loop
        // leaves (an index left at its end): scratch, flagged.
        let mut t = RamTrace::new();
        let mut ram = vec![0u8; RAM];
        for _ in 0..400 {
            t.frame_start();
            t.read(0x08);
            t.write(0x08);
            t.read(0x08);
            ram[0x08] = 5;
            t.frame_end(&ram);
        }
        let r = role(&t.classify(), 0x08).clone();
        assert_eq!((r.role, r.review), (Role::Scratch, true), "{r:?}");
        assert!(r.why.contains("always $05"), "{r:?}");
    }

    #[test]
    fn a_controller_byte_that_never_varies_proves_no_mask() {
        // Pad 2 is never pressed: a byte that is mostly 0 agrees with it.
        let mut t = RamTrace::new();
        let mut ram = vec![0u8; RAM];
        for f in 0..300u32 {
            t.frame_start();
            t.call(0x8100);
            t.controller_read();
            t.write(0x10);
            ram[0x10] = 0;
            t.ret();
            t.call(0x8200);
            t.read(0x20);
            t.write(0x20);
            ram[0x20] = u8::from(f == 100) * 4;
            t.ret();
            t.frame_end(&ram);
        }
        assert_ne!(role(&t.classify(), 0x20).role, Role::Input);
    }

    #[test]
    fn a_frame_counter_that_pauses_is_a_counter() {
        let mut t = RamTrace::new();
        let mut ram = vec![0u8; RAM];
        for f in 0..800u32 {
            t.frame_start();
            t.read(0x09);
            if !(200..300).contains(&f) {
                t.write(0x09);
                ram[0x09] = ram[0x09].wrapping_add(1);
            }
            t.frame_end(&ram);
        }
        let r = role(&t.classify(), 0x09).clone();
        assert_eq!((r.role, r.review), (Role::Counter, false), "{r:?}");
    }

    #[test]
    fn a_timer_that_reloads_is_not_a_counter() {
        let mut t = RamTrace::new();
        let mut ram = vec![0u8; RAM];
        for f in 0..500u32 {
            t.frame_start();
            t.read(0x40);
            t.write(0x40);
            ram[0x40] = (20 - f % 21) as u8;
            t.frame_end(&ram);
        }
        assert_eq!(role(&t.classify(), 0x40).role, Role::State);
    }

    #[test]
    fn without_a_sound_routine_nothing_is_sound() {
        let mut t = RamTrace::new();
        let ram = vec![0u8; RAM];
        t.frame_start();
        t.call(0x8000);
        t.apu_write();
        t.other_io_write();
        t.read(0x50);
        t.ret();
        t.frame_end(&ram);
        assert!(t.sound_routines().is_empty());
        assert_ne!(role(&t.classify(), 0x50).role, Role::Sound);
    }
}
