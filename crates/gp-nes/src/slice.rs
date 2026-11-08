//! Writer slices (docs/research/deterministic-inference.md, A2): which
//! instruction wrote a byte on a frame, and the bytes and branch conditions
//! its value depends on, from one frame's run of the original.
//!
//! [`Recorder`] watches the CPU between instructions through
//! [`crate::Nes::step_with`], as [`crate::coverage`] does, and changes
//! nothing the console does. For each instruction it keeps the registers,
//! the operand's effective address (through indexing and pointers, as the
//! CPU resolved it on that frame) and the byte there, and the call it runs
//! in, from `JSR`, the interrupts and the stack pointer.
//!
//! [`slice`] walks that frame backward from one write (a backward dynamic
//! slice: Weiser, ICSE 1981; Korel and Laski, Information Processing
//! Letters, 1988). Each instruction's effect is a list of the places it
//! defines (a register, a flag, a byte) and, for each, the places that
//! value is computed from. A place is live while some instruction of the
//! slice still needs its value, and the latest instruction before that
//! defines it joins the slice. The region is the writer's call and the
//! calls it makes; a live place defined outside it, or not at all on the
//! frame, is one of the region's inputs. The slice is exact for the
//! instructions it covers: it reads the addresses the CPU used, not the
//! base labels.
//!
//! It gives up, and says so in [`Slice::gaps`], where the trace can't
//! say what a value came from:
//!
//! | Gap | Why |
//! |---|---|
//! | Code run from RAM | Code that may have been written, or may change itself; its bytes are taken as they were when it ran |
//! | An opcode it doesn't model | The unofficial opcodes whose result depends on the bus (`XAA`, `LAS`, `TAS`, `AHX`, `SYA`, `SXA`, `SHAA`, `ATX`) and `BRK`; what they define is left out |
//! | A register read | A byte from the PPU, the APU or a mapper: the slice stops at the register. The controllers ($4016, $4017) are inputs, not a gap |
//!
//! The stack pointer is taken as known: a value pushed and pulled is
//! followed through the stack byte, and `TSX` gives a constant.

use std::collections::BTreeMap;

use tetanes_core::bus::Bus;
use tetanes_core::cpu::Cpu;
use tetanes_core::cpu::instr::{AddrMode, Instr};

/// A place a value lives: a register, a status flag, or a byte at a CPU
/// address (RAM's mirrors folded to $0000-$07FF).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Loc {
    A,
    X,
    Y,
    N,
    Z,
    C,
    V,
    Mem(u16),
}

impl Loc {
    pub fn name(self) -> String {
        match self {
            Loc::A => "a".into(),
            Loc::X => "x".into(),
            Loc::Y => "y".into(),
            Loc::N => "flag n".into(),
            Loc::Z => "flag z".into(),
            Loc::C => "flag c".into(),
            Loc::V => "flag v".into(),
            Loc::Mem(a) => format!("${a:04X}"),
        }
    }
}

/// RAM's mirrors folded, and the PPU registers' mirrors.
pub fn fold(addr: u16) -> u16 {
    match addr {
        0..=0x1FFF => addr & 0x7FF,
        0x2000..=0x3FFF => 0x2000 | (addr & 7),
        _ => addr,
    }
}

/// Where a byte lives, for how far a slice can follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    /// Internal RAM or cartridge RAM: followed back to its writer.
    Ram,
    /// A register of the PPU, the APU or the controllers.
    Io,
    /// The cartridge's ROM (and anything else at $8000 and up): a constant.
    Rom,
}

pub fn space(addr: u16) -> Space {
    match addr {
        0..=0x1FFF | 0x6000..=0x7FFF => Space::Ram,
        0x2000..=0x5FFF => Space::Io,
        _ => Space::Rom,
    }
}

/// One instruction as it ran: the state before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub pc: u16,
    pub op: u8,
    /// The bytes after the opcode, low first, whether it uses them or not.
    pub operand: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub p: u8,
    pub sp: u8,
    /// The address its operand reached, folded ([`fold`]).
    pub addr: Option<u16>,
    /// The zero page pointer's first byte (`($nn,X)`, `($nn),Y`) or
    /// `JMP ($nnnn)`'s pointer.
    pub ptr: Option<u16>,
    /// The byte at `addr` before it ran, read without side effects.
    pub value: u8,
    /// The pointer's two bytes, for `($nn,X)` and `($nn),Y`.
    pub pointer: [u8; 2],
    /// The byte a pull would take: the one above the stack pointer.
    pub pulled: u8,
    /// The call it runs in: an index into [`FrameTrace::calls`].
    pub call: u32,
    /// An interrupt was taken just before it: it is the handler's first.
    pub interrupt: bool,
}

impl Step {
    pub fn instr(&self) -> Instr {
        Cpu::INSTR_REF[usize::from(self.op)].instr
    }

    pub fn mode(&self) -> AddrMode {
        Cpu::INSTR_REF[usize::from(self.op)].addr_mode
    }

    /// The register that indexed its operand, if one did.
    pub fn index(&self) -> Option<Loc> {
        match self.mode() {
            AddrMode::ZPX | AddrMode::ABX | AddrMode::ABXW | AddrMode::IDX => Some(Loc::X),
            AddrMode::ZPY | AddrMode::ABY | AddrMode::ABYW | AddrMode::IDY | AddrMode::IDYW => {
                Some(Loc::Y)
            }
            _ => None,
        }
    }

    /// The operand's address as the code names it, before indexing: the
    /// base a label names (`Enemy_Y_Speed` of `sta Enemy_Y_Speed,x`), or
    /// the pointer's zero page byte for `($nn),Y` and `($nn,X)`.
    pub fn base(&self) -> Option<u16> {
        match self.mode() {
            AddrMode::ZP0
            | AddrMode::ZPX
            | AddrMode::ZPY
            | AddrMode::IDX
            | AddrMode::IDY
            | AddrMode::IDYW => Some(self.operand & 0xFF),
            AddrMode::ABS | AddrMode::ABX | AddrMode::ABXW | AddrMode::ABY | AddrMode::ABYW => {
                Some(fold(self.operand))
            }
            _ => None,
        }
    }

    /// Whether it went through a pointer: `($nn),Y` or `($nn,X)`.
    pub fn indirect(&self) -> bool {
        matches!(self.mode(), AddrMode::IDX | AddrMode::IDY | AddrMode::IDYW)
    }

    /// One of the eight conditional branches.
    pub fn is_branch(&self) -> bool {
        crate::coverage::is_branch(self.op)
    }

    /// The flag a conditional branch tests.
    pub fn branch_flag(&self) -> Option<Loc> {
        Some(match self.instr() {
            Instr::BPL | Instr::BMI => Loc::N,
            Instr::BVC | Instr::BVS => Loc::V,
            Instr::BCC | Instr::BCS => Loc::C,
            Instr::BNE | Instr::BEQ => Loc::Z,
            _ => return None,
        })
    }

    /// The byte a store writes: the register it stores, for `STA`, `STX`
    /// and `STY`.
    pub fn stored(&self) -> Option<u8> {
        match self.instr() {
            Instr::STA => Some(self.a),
            Instr::STX => Some(self.x),
            Instr::STY => Some(self.y),
            _ => None,
        }
    }

    /// Whether it ran from RAM, where code can be written.
    pub fn from_ram(&self) -> bool {
        space(self.pc) == Space::Ram
    }
}

/// One call: a `JSR`, or an interrupt, and the steps it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Call {
    /// The call it was made from (the root's own index for the root).
    pub parent: u32,
    /// Where it went: the `JSR`'s target or the handler's first address.
    /// 0 for the root, the code the frame started in.
    pub entry: u16,
    /// Its first step.
    pub first: usize,
    pub interrupt: bool,
}

/// One frame's instructions, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameTrace {
    pub steps: Vec<Step>,
    /// Call 0 is the root.
    pub calls: Vec<Call>,
}

/// Watches one frame's instructions: call [`Recorder::start`] before the
/// frame and give it the bus before each instruction.
#[derive(Debug)]
pub struct Recorder {
    trace: FrameTrace,
    /// The open calls: the stack pointer at which each has returned, and
    /// its index.
    open: Vec<(u8, u32)>,
    current: u32,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            trace: FrameTrace {
                steps: Vec::new(),
                calls: vec![Call {
                    parent: 0,
                    entry: 0,
                    first: 0,
                    interrupt: false,
                }],
            },
            open: Vec::new(),
            current: 0,
        }
    }

    /// Clears the record of an interrupt taken before the frame, which is
    /// stale until the next one.
    pub fn start(&mut self, bus: &mut Bus) {
        bus.interrupt_taken = None;
    }

    fn open_call(&mut self, entry: u16, back_at: u8, interrupt: bool) {
        let id = self.trace.calls.len() as u32;
        self.trace.calls.push(Call {
            parent: self.current,
            entry,
            first: self.trace.steps.len(),
            interrupt,
        });
        self.open.push((back_at, id));
        self.current = id;
    }

    /// Notes the instruction about to run.
    #[inline]
    pub fn before(&mut self, bus: &mut Bus) {
        let cpu = &bus.cpu;
        let (pc, sp) = (cpu.pc, cpu.sp);
        // Calls that have returned: RTS, RTI, or a routine that pulled its
        // own return address, as a jump table does.
        while let Some(&(back, _)) = self.open.last() {
            if back <= sp {
                self.open.pop();
                self.current = self.open.last().map_or(0, |&(_, id)| id);
            } else {
                break;
            }
        }
        let interrupt = bus.interrupt_taken.take().is_some();
        if interrupt {
            self.open_call(pc, sp.wrapping_add(3), true);
        }
        let cpu = &bus.cpu;
        let op = bus.peek(pc);
        let r = Cpu::INSTR_REF[usize::from(op)];
        let b1 = bus.peek(pc.wrapping_add(1));
        let b2 = bus.peek(pc.wrapping_add(2));
        let abs = u16::from(b1) | u16::from(b2) << 8;
        let (x, y) = (cpu.x, cpu.y);
        let word = |lo: u16, hi: u16| u16::from(bus.peek(lo)) | u16::from(bus.peek(hi)) << 8;
        let (addr, ptr) = match r.addr_mode {
            AddrMode::ZP0 => (Some(u16::from(b1)), None),
            AddrMode::ZPX => (Some(u16::from(b1.wrapping_add(x))), None),
            AddrMode::ZPY => (Some(u16::from(b1.wrapping_add(y))), None),
            AddrMode::ABS if r.instr != Instr::JMP => (Some(abs), None),
            AddrMode::ABX | AddrMode::ABXW => (Some(abs.wrapping_add(x.into())), None),
            AddrMode::ABY | AddrMode::ABYW => (Some(abs.wrapping_add(y.into())), None),
            AddrMode::IDX => {
                let p = b1.wrapping_add(x);
                (
                    Some(word(p.into(), p.wrapping_add(1).into())),
                    Some(u16::from(p)),
                )
            }
            AddrMode::IDY | AddrMode::IDYW => {
                let base = word(b1.into(), b1.wrapping_add(1).into());
                (Some(base.wrapping_add(y.into())), Some(u16::from(b1)))
            }
            AddrMode::IND => (None, Some(abs)),
            _ => (None, None),
        };
        let addr = addr.map(fold);
        let step = Step {
            pc,
            op,
            operand: abs,
            a: cpu.acc,
            x,
            y,
            p: cpu.status.bits(),
            sp,
            addr,
            ptr,
            value: addr.map_or(0, |a| bus.peek(a)),
            pointer: match (r.addr_mode, ptr) {
                (AddrMode::IDX | AddrMode::IDY | AddrMode::IDYW, Some(p)) => {
                    [bus.peek(p), bus.peek((p + 1) & 0xFF)]
                }
                _ => [0, 0],
            },
            pulled: bus.peek(0x100 | u16::from(sp.wrapping_add(1))),
            call: self.current,
            interrupt,
        };
        self.trace.steps.push(step);
        if r.instr == Instr::JSR {
            self.open_call(abs, sp, false);
        }
    }

    pub fn finish(self) -> FrameTrace {
        self.trace
    }
}

/// How a value reaches the slice's byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum How {
    /// Its value is part of the value written.
    Value,
    /// It picks which byte is read or written: an index register's source.
    Index,
    /// It is a pointer's byte.
    Pointer,
    /// It decides a branch taken on the way to the write.
    Branch,
}

impl How {
    pub fn name(self) -> &'static str {
        match self {
            How::Value => "value",
            How::Index => "index",
            How::Pointer => "pointer",
            How::Branch => "branch",
        }
    }

    /// The way a use reaches the slice through a value that reaches it
    /// `self`'s way.
    fn then(self, by: How) -> How {
        match self {
            How::Value => by,
            other => other.max(by),
        }
    }
}

/// A place an instruction's result is computed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Use {
    pub loc: Loc,
    pub how: How,
}

/// What an instruction defines, each place with the places it is computed
/// from. `None` for an instruction [`slice`] doesn't model.
pub fn effects(s: &Step) -> Option<Vec<(Loc, Vec<Use>)>> {
    use Instr::*;
    use Loc::*;
    let r = Cpu::INSTR_REF[usize::from(s.op)];
    let v = |l: Loc| Use {
        loc: l,
        how: How::Value,
    };
    // The address's own inputs: the index register and the pointer bytes.
    let mut at: Vec<Use> = Vec::new();
    if let Some(i) = s.index() {
        at.push(Use {
            loc: i,
            how: How::Index,
        });
    }
    if s.indirect()
        && let Some(p) = s.ptr
    {
        for b in [p, (p + 1) & 0xFF] {
            at.push(Use {
                loc: Mem(b),
                how: How::Pointer,
            });
        }
    }
    // The operand: the byte it reads, with the address's inputs, or nothing
    // for an immediate.
    let m = s.addr.map(Mem);
    let operand = || -> Vec<Use> {
        match m {
            Some(l) => std::iter::once(v(l)).chain(at.iter().copied()).collect(),
            None => Vec::new(),
        }
    };
    let with = |regs: &[Loc], more: Vec<Use>| -> Vec<Use> {
        regs.iter().map(|&l| v(l)).chain(more).collect()
    };
    let stack = |off: u8| Mem(0x100 | u16::from(s.sp.wrapping_add(off)));
    let each = |locs: &[Loc], uses: Vec<Use>| -> Vec<(Loc, Vec<Use>)> {
        locs.iter().map(|&l| (l, uses.clone())).collect()
    };
    let acc = r.addr_mode == AddrMode::ACC;
    let mem = || m.expect("a memory operand");
    let shift = |carry: bool| -> Vec<(Loc, Vec<Use>)> {
        let c = if carry { vec![v(C)] } else { Vec::new() };
        if acc {
            each(&[A, N, Z, C], with(&[A], c))
        } else {
            each(&[mem(), N, Z, C], [operand(), c].concat())
        }
    };
    Some(match r.instr {
        LDA => each(&[A, N, Z], operand()),
        LDX => each(&[X, N, Z], operand()),
        LDY => each(&[Y, N, Z], operand()),
        LAX => each(&[A, X, N, Z], operand()),
        STA => vec![(mem(), with(&[A], at.clone()))],
        STX => vec![(mem(), with(&[X], at.clone()))],
        STY => vec![(mem(), with(&[Y], at.clone()))],
        SAX => vec![(mem(), with(&[A, X], at.clone()))],
        ADC | SBC => each(&[A, N, Z, C, V], with(&[A, C], operand())),
        AND | ORA | EOR => each(&[A, N, Z], with(&[A], operand())),
        CMP => each(&[N, Z, C], with(&[A], operand())),
        CPX => each(&[N, Z, C], with(&[X], operand())),
        CPY => each(&[N, Z, C], with(&[Y], operand())),
        BIT => [each(&[N, V], operand()), each(&[Z], with(&[A], operand()))].concat(),
        ASL | LSR => shift(false),
        ROL | ROR => shift(true),
        INC | DEC => each(&[mem(), N, Z], operand()),
        INX | DEX => each(&[X, N, Z], vec![v(X)]),
        INY | DEY => each(&[Y, N, Z], vec![v(Y)]),
        TAX => each(&[X, N, Z], vec![v(A)]),
        TAY => each(&[Y, N, Z], vec![v(A)]),
        TXA => each(&[A, N, Z], vec![v(X)]),
        TYA => each(&[A, N, Z], vec![v(Y)]),
        TSX => each(&[X, N, Z], Vec::new()),
        PHA => vec![(stack(0), vec![v(A)])],
        PHP => vec![(stack(0), vec![v(N), v(Z), v(C), v(V)])],
        PLA => each(&[A, N, Z], vec![v(stack(1))]),
        PLP | RTI => each(&[N, Z, C, V], vec![v(stack(1))]),
        // The return address is a constant.
        JSR => vec![(stack(0), Vec::new()), (stack(0xFF), Vec::new())],
        CLC | SEC => vec![(C, Vec::new())],
        CLV => vec![(V, Vec::new())],
        TXS | RTS | JMP | NOP | CLD | SED | CLI | SEI | BPL | BMI | BVC | BVS | BCC | BCS | BNE
        | BEQ => Vec::new(),
        DCP => [
            vec![(mem(), operand())],
            each(&[N, Z, C], with(&[A], operand())),
        ]
        .concat(),
        ISB => [
            vec![(mem(), operand())],
            each(&[A, N, Z, C, V], with(&[A, C], operand())),
        ]
        .concat(),
        SLO | SRE => [
            each(&[mem(), C], operand()),
            each(&[A, N, Z], with(&[A], operand())),
        ]
        .concat(),
        RLA => [
            each(&[mem(), C], with(&[C], operand())),
            each(&[A, N, Z], with(&[A, C], operand())),
        ]
        .concat(),
        RRA => [
            vec![(mem(), with(&[C], operand()))],
            each(&[A, N, Z, C, V], with(&[A, C], operand())),
        ]
        .concat(),
        ANC | ALR => each(&[A, N, Z, C], vec![v(A)]),
        ARR => each(&[A, N, Z, C, V], vec![v(A), v(C)]),
        AXS => each(&[X, N, Z, C], vec![v(A), v(X)]),
        _ => return None,
    })
}

/// Where a byte the slice reads got its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum From {
    /// Not written on the frame before the read: what the frame before
    /// left.
    Before,
    /// Written by this step, inside the slice's region.
    Inside(usize),
    /// Written by this step, outside the region (earlier in the frame).
    Outside(usize),
    /// The cartridge's ROM: a constant.
    Rom,
    /// A hardware register.
    Io,
}

/// One byte a slice step read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Read {
    /// The step that read it.
    pub step: usize,
    pub addr: u16,
    pub value: u8,
    pub how: How,
    pub from: From,
}

/// A register or flag the region took from the code before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegIn {
    pub loc: Loc,
    pub how: How,
    /// The step that set it, or `None` if nothing did on the frame.
    pub from: Option<usize>,
}

/// A conditional branch on the way to the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Branch {
    pub step: usize,
    pub taken: bool,
}

/// Where a slice gives up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// A step of the region ran from RAM.
    CodeInRam { step: usize },
    /// A step of the region whose effect isn't modelled.
    Unmodelled { step: usize },
    /// A slice step read a hardware register.
    Register { step: usize, addr: u16 },
}

/// The slice of one write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slice {
    /// The step that wrote the byte.
    pub writer: usize,
    pub loc: Option<Loc>,
    /// The steps whose results reach the write, the writer last.
    pub steps: Vec<usize>,
    /// The bytes those steps read, latest first.
    pub reads: Vec<Read>,
    pub regs_in: Vec<RegIn>,
    /// The branches the writer's own call took before the write, latest
    /// first.
    pub branches: Vec<Branch>,
    pub gaps: Vec<Gap>,
}

impl FrameTrace {
    /// Whether call `c` is `root` or one it made, directly or not.
    pub fn within(&self, c: u32, root: u32) -> bool {
        let mut c = c;
        loop {
            if c == root {
                return true;
            }
            let p = self.calls[c as usize].parent;
            if p == c {
                return false;
            }
            c = p;
        }
    }

    /// The steps that wrote `addr` (folded), in order.
    pub fn writes_of(&self, addr: u16) -> Vec<usize> {
        let want = Loc::Mem(fold(addr));
        self.steps
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.addr.is_some_and(|a| a == want_addr(want))
                    && effects(s).is_some_and(|e| e.iter().any(|(l, _)| *l == want))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The last step that wrote `addr`.
    pub fn last_write(&self, addr: u16) -> Option<usize> {
        self.writes_of(addr).last().copied()
    }
}

fn want_addr(l: Loc) -> u16 {
    match l {
        Loc::Mem(a) => a,
        _ => u16::MAX,
    }
}

/// Options for [`slice`].
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Follow the branches the writer's call took before the write.
    pub branches: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { branches: true }
    }
}

/// The backward slice of `loc` as step `writer` defines it, over the steps
/// of `trace` before it. The region is the writer's call and the calls it
/// made.
pub fn slice(trace: &FrameTrace, writer: usize, loc: Loc, opts: Options) -> Slice {
    let steps = &trace.steps;
    let home = steps[writer].call;
    let region = |j: usize| trace.within(steps[j].call, home);
    let mut out = Slice {
        writer,
        loc: Some(loc),
        ..Slice::default()
    };
    // Each live place: the ways it reaches the write, and the reads (by
    // index into `out.reads`) waiting for its writer.
    let mut live: BTreeMap<Loc, (How, Vec<usize>)> = BTreeMap::new();
    let mut seen: BTreeMap<(usize, u16), usize> = BTreeMap::new();
    let mut in_slice = vec![writer];

    // Takes in step `j`'s uses for a place it defines that reaches the
    // write `how`.
    let mut take = |j: usize,
                    how: How,
                    uses: &[Use],
                    live: &mut BTreeMap<Loc, (How, Vec<usize>)>,
                    out: &mut Slice| {
        let s = &steps[j];
        for u in uses {
            let h = how.then(u.how);
            let Loc::Mem(a) = u.loc else {
                let e = live.entry(u.loc).or_insert((h, Vec::new()));
                e.0 = e.0.min(h);
                continue;
            };
            let read = match seen.get(&(j, a)) {
                Some(&k) => {
                    out.reads[k].how = out.reads[k].how.min(h);
                    k
                }
                None => {
                    let from = match space(a) {
                        Space::Ram => From::Before,
                        Space::Rom => From::Rom,
                        // The controllers are inputs, not a gap.
                        Space::Io if a == 0x4016 || a == 0x4017 => From::Io,
                        Space::Io => {
                            if !out
                                .gaps
                                .iter()
                                .any(|g| matches!(g, Gap::Register { addr, .. } if *addr == a))
                            {
                                out.gaps.push(Gap::Register { step: j, addr: a });
                            }
                            From::Io
                        }
                    };
                    let value = byte_seen(s, a);
                    out.reads.push(Read {
                        step: j,
                        addr: a,
                        value,
                        how: h,
                        from,
                    });
                    seen.insert((j, a), out.reads.len() - 1);
                    out.reads.len() - 1
                }
            };
            if space(a) == Space::Ram {
                let e = live.entry(u.loc).or_insert((h, Vec::new()));
                e.0 = e.0.min(h);
                if !e.1.contains(&read) {
                    e.1.push(read);
                }
            }
        }
    };

    // The writer's own uses for `loc`.
    match effects(&steps[writer]) {
        Some(eff) => {
            for (d, uses) in eff {
                if d == loc {
                    take(writer, How::Value, &uses, &mut live, &mut out);
                }
            }
        }
        None => out.gaps.push(Gap::Unmodelled { step: writer }),
    }
    if steps[writer].from_ram() {
        out.gaps.push(Gap::CodeInRam { step: writer });
    }

    for j in (0..writer).rev() {
        let s = &steps[j];
        let inside = region(j);
        if opts.branches && s.call == home && s.is_branch() {
            let fall = s.pc.wrapping_add(2);
            let taken = steps.get(j + 1).is_some_and(|n| n.pc != fall);
            out.branches.push(Branch { step: j, taken });
            if let Some(f) = s.branch_flag() {
                take(
                    j,
                    How::Branch,
                    &[Use {
                        loc: f,
                        how: How::Branch,
                    }],
                    &mut live,
                    &mut out,
                );
            }
        }
        if live.is_empty() {
            continue;
        }
        let Some(eff) = effects(s) else {
            if inside {
                out.gaps.push(Gap::Unmodelled { step: j });
            }
            continue;
        };
        let mut joined = false;
        for (d, uses) in eff {
            let Some((how, reads)) = live.remove(&d) else {
                continue;
            };
            for &k in &reads {
                out.reads[k].from = if inside {
                    From::Inside(j)
                } else {
                    From::Outside(j)
                };
            }
            if inside {
                joined = true;
                take(j, how, &uses, &mut live, &mut out);
            } else if !matches!(d, Loc::Mem(_)) {
                out.regs_in.push(RegIn {
                    loc: d,
                    how,
                    from: Some(j),
                });
            }
        }
        if joined {
            in_slice.push(j);
            if s.from_ram() {
                out.gaps.push(Gap::CodeInRam { step: j });
            }
        }
    }
    for (loc, (how, _)) in live {
        if !matches!(loc, Loc::Mem(_)) {
            out.regs_in.push(RegIn {
                loc,
                how,
                from: None,
            });
        }
    }
    in_slice.sort_unstable();
    in_slice.dedup();
    out.steps = in_slice;
    out.gaps.sort_by_key(|g| match g {
        Gap::CodeInRam { step } | Gap::Unmodelled { step } | Gap::Register { step, .. } => *step,
    });
    out.gaps.dedup();
    out
}

/// The value step `s` saw at `a`: its operand's byte, a pointer byte or
/// the byte it pulled.
fn byte_seen(s: &Step, a: u16) -> u8 {
    if s.addr == Some(a) {
        return s.value;
    }
    if let Some(p) = s.ptr.filter(|_| s.indirect()) {
        if a == p {
            return s.pointer[0];
        }
        if a == (p + 1) & 0xFF {
            return s.pointer[1];
        }
    }
    s.pulled
}

/// A slice, and the slices of the bytes and registers its region took from
/// earlier in the frame, `depth` levels on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Followed {
    pub slice: Slice,
    /// For each earlier write followed: the read or register it fed, and
    /// its slice.
    pub feeds: Vec<(Loc, Followed)>,
}

/// [`slice`], then the same for each value the region took from a write
/// earlier in the frame outside it, `depth` levels on, with the branches
/// of each writer's own call. Only values, not indexes or branches, are
/// followed to their writers.
pub fn follow(trace: &FrameTrace, writer: usize, loc: Loc, opts: Options, depth: u32) -> Followed {
    let s = slice(trace, writer, loc, opts);
    let mut feeds = Vec::new();
    if depth > 0 {
        let mut done: Vec<(usize, Loc)> = Vec::new();
        let mut from: Vec<(usize, Loc)> = Vec::new();
        for r in &s.reads {
            if let (From::Outside(j), How::Value) = (r.from, r.how) {
                from.push((j, Loc::Mem(r.addr)));
            }
        }
        for r in &s.regs_in {
            if let (Some(j), How::Value) = (r.from, r.how) {
                from.push((j, r.loc));
            }
        }
        for (j, l) in from {
            if done.contains(&(j, l)) {
                continue;
            }
            done.push((j, l));
            let sub = follow(trace, j, l, opts, depth - 1);
            feeds.push((l, sub));
        }
    }
    Followed { slice: s, feeds }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Nes;
    use gp_evidence::input_log::InputFrame;
    use gp_evidence::manifest::PowerOn;

    /// A tiny assembler: bytes at $C000 on, with labels patched after.
    struct Asm {
        code: Vec<u8>,
        labels: BTreeMap<&'static str, u16>,
        /// (offset, label, relative)
        fixups: Vec<(usize, &'static str, bool)>,
    }

    impl Asm {
        fn new() -> Self {
            Self {
                code: Vec::new(),
                labels: BTreeMap::new(),
                fixups: Vec::new(),
            }
        }
        fn here(&self) -> u16 {
            0xC000 + self.code.len() as u16
        }
        fn label(&mut self, name: &'static str) -> &mut Self {
            let at = self.here();
            self.labels.insert(name, at);
            self
        }
        fn b(&mut self, bytes: &[u8]) -> &mut Self {
            self.code.extend_from_slice(bytes);
            self
        }
        /// An instruction with a 16-bit operand that is a label.
        fn abs(&mut self, op: u8, to: &'static str) -> &mut Self {
            self.code.push(op);
            self.fixups.push((self.code.len(), to, false));
            self.code.extend_from_slice(&[0, 0]);
            self
        }
        fn rel(&mut self, op: u8, to: &'static str) -> &mut Self {
            self.code.push(op);
            self.fixups.push((self.code.len(), to, true));
            self.code.push(0);
            self
        }
        fn rom(mut self) -> (Vec<u8>, BTreeMap<&'static str, u16>) {
            for &(off, to, relative) in &self.fixups {
                let target = self.labels[to];
                if relative {
                    let next = 0xC000 + off as i32 + 1;
                    let d = i32::from(target) - next;
                    assert!((-128..128).contains(&d), "{to} out of reach");
                    self.code[off] = d as i8 as u8;
                } else {
                    self.code[off..off + 2].copy_from_slice(&target.to_le_bytes());
                }
            }
            let mut prg = vec![0xEAu8; 0x4000];
            prg[..self.code.len()].copy_from_slice(&self.code);
            let nmi = self.labels["nmi"].to_le_bytes();
            let reset = self.labels["reset"].to_le_bytes();
            prg[0x3FFA..0x3FFC].copy_from_slice(&nmi);
            prg[0x3FFC..0x3FFE].copy_from_slice(&reset);
            prg[0x3FFE..0x4000].copy_from_slice(&nmi);
            let mut rom = b"NES\x1a\x01\x00\x00\x00".to_vec();
            rom.resize(16, 0);
            rom.extend(prg);
            (rom, self.labels)
        }
    }

    // RAM of the made-up game.
    const SCRATCH: u8 = 0x00;
    const PTR: u8 = 0x02; // and $03: points at TILES
    const TIMER: u8 = 0x11;
    const SLOT: u8 = 0x40;
    const RANDOM: u8 = 0x50;
    const SPEED: u8 = 0x20; // one a slot
    const RAMCODE_OUT: u8 = 0x60;
    const TILES: u16 = 0x0300;
    const RAMCODE: u16 = 0x0400;

    /// The made-up game: on reset it sets a tile, points `PTR` at it and
    /// copies a three-instruction routine into RAM. Each NMI steps the
    /// random byte, sets the slot, calls `mover` and the routine in RAM.
    /// `mover` writes `SPEED+x` from the random byte, masked, through the
    /// scratch byte, plus the tile its helper reads through the pointer,
    /// unless the timer is set.
    fn game() -> (Vec<u8>, BTreeMap<&'static str, u16>) {
        let mut a = Asm::new();
        a.label("reset")
            .b(&[0x78, 0xD8, 0xA2, 0xFF, 0x9A]) // SEI CLD LDX #$FF TXS
            .label("vw1")
            .b(&[0x2C, 0x02, 0x20]) // BIT $2002
            .rel(0x10, "vw1") // BPL vw1
            .label("vw2")
            .b(&[0x2C, 0x02, 0x20])
            .rel(0x10, "vw2")
            .b(&[0xA9, 0x05, 0x8D, 0x00, 0x03]) // LDA #5 STA TILES
            .b(&[0xA9, 0x00, 0x85, PTR, 0xA9, 0x03, 0x85, PTR + 1]) // PTR = $0300
            // Copy LDA #7 / STA $60 / RTS to $0400.
            .b(&[0xA9, 0xA9, 0x8D, 0x00, 0x04, 0xA9, 0x07, 0x8D, 0x01, 0x04])
            .b(&[
                0xA9,
                0x85,
                0x8D,
                0x02,
                0x04,
                0xA9,
                RAMCODE_OUT,
                0x8D,
                0x03,
                0x04,
            ])
            .b(&[0xA9, 0x60, 0x8D, 0x04, 0x04])
            .b(&[0xA9, 0x80, 0x8D, 0x00, 0x20]) // enable NMI
            .label("idle")
            .abs(0x4C, "idle");
        a.label("nmi")
            .b(&[0xE6, RANDOM]) // INC RANDOM
            .b(&[0xA2, 0x01, 0x86, SLOT]) // LDX #1 STX SLOT
            .abs(0x20, "mover") // JSR mover
            .b(&[0x20, 0x00, 0x04]) // JSR $0400
            .b(&[0x40]); // RTI
        a.label("mover")
            .b(&[0xA6, SLOT]) // LDX SLOT
            .b(&[0xA5, TIMER]) // LDA TIMER
            .rel(0xD0, "skip") // BNE skip
            .label("read_random")
            .b(&[0xB5, RANDOM - 1]) // LDA RANDOM-1,X
            .b(&[0x29, 0x0F, 0x85, SCRATCH]) // AND #$0F STA SCRATCH
            .abs(0x20, "helper") // JSR helper
            .label("write")
            .b(&[0x95, SPEED]) // STA SPEED,X
            .label("skip")
            .b(&[0x60]); // RTS
        a.label("helper")
            .b(&[0xA0, 0x00]) // LDY #0
            .label("read_tile")
            .b(&[0xB1, PTR]) // LDA (PTR),Y
            .b(&[0x18]) // CLC
            .label("add")
            .b(&[0x65, SCRATCH]) // ADC SCRATCH
            .b(&[0x60]); // RTS
        a.rom()
    }

    fn pads() -> InputFrame {
        InputFrame {
            polled: false,
            pads: [0; 4],
            reset: false,
        }
    }

    /// The game run to `frames`, then one frame recorded.
    fn recorded(rom: &[u8], frames: usize) -> (Nes, FrameTrace) {
        let mut nes = Nes::power_on(rom, PowerOn::default()).unwrap();
        for _ in 0..frames {
            nes.step(pads()).unwrap();
        }
        let mut r = Recorder::new();
        r.start(nes.bus_mut_for_observers());
        nes.step_with(pads(), |bus| r.before(bus)).unwrap();
        (nes, r.finish())
    }

    fn step_at(t: &FrameTrace, pc: u16) -> usize {
        t.steps.iter().position(|s| s.pc == pc).unwrap()
    }

    #[test]
    fn the_writer_and_every_read_it_depends_on() {
        let (rom, at) = game();
        let (nes, t) = recorded(&rom, 10);
        let speed = u16::from(SPEED) + 1;
        let writer = t.last_write(speed).unwrap();
        assert_eq!(t.steps[writer].pc, at["write"]);
        // The write lands where the game says: (random & 15) + the tile.
        let random = nes.wram()[usize::from(RANDOM)];
        assert_eq!(nes.wram()[usize::from(speed)], (random & 0x0F) + 5);

        let s = slice(&t, writer, Loc::Mem(speed), Options::default());
        let inc = step_at(&t, at["nmi"]);
        let stx = step_at(&t, at["nmi"] + 4);
        let sta_scratch = step_at(&t, at["read_random"] + 4);
        let read = |addr: u16, step_pc: u16| {
            s.reads
                .iter()
                .find(|r| r.addr == addr && t.steps[r.step].pc == step_pc)
                .copied()
                .unwrap_or_else(|| panic!("no read of ${addr:04X} at ${step_pc:04X}: {s:#?}"))
        };
        // The random byte, read indexed, set earlier in the frame by NMI.
        let r = read(u16::from(RANDOM), at["read_random"]);
        assert_eq!((r.how, r.from), (How::Value, From::Outside(inc)));
        assert_eq!(r.value, random);
        // The tile through the pointer, from before the frame, and the
        // pointer's two bytes.
        let r = read(TILES, at["read_tile"]);
        assert_eq!((r.how, r.from, r.value), (How::Value, From::Before, 5));
        for b in [u16::from(PTR), u16::from(PTR) + 1] {
            let r = read(b, at["read_tile"]);
            assert_eq!((r.how, r.from), (How::Pointer, From::Before));
        }
        // The scratch byte, written inside the region.
        let r = read(u16::from(SCRATCH), at["add"]);
        assert_eq!((r.how, r.from), (How::Value, From::Inside(sta_scratch)));
        // The slot: the index of both the store and the random read.
        let r = read(u16::from(SLOT), at["mover"]);
        assert_eq!((r.how, r.from), (How::Index, From::Outside(stx)));
        // The timer, tested by the branch, not taken.
        let r = read(u16::from(TIMER), at["mover"] + 2);
        assert_eq!((r.how, r.from), (How::Branch, From::Before));
        assert_eq!(s.branches.len(), 1);
        assert!(!s.branches[0].taken);
        // Nothing else: no register came from the caller, no gap.
        assert_eq!(s.reads.len(), 7, "{:#?}", s.reads);
        assert!(s.regs_in.is_empty(), "{:?}", s.regs_in);
        assert!(s.gaps.is_empty(), "{:?}", s.gaps);
        // The steps: LDX, LDA TIMER (for the branch), LDA RANDOM-1,X, AND,
        // STA SCRATCH, LDY #0 (Y indexes the pointer), LDA (PTR),Y, CLC,
        // ADC, and the write.
        let pcs: Vec<u16> = s.steps.iter().map(|&j| t.steps[j].pc).collect();
        assert_eq!(pcs.len(), 10, "{pcs:04X?}");
        assert_eq!(*pcs.last().unwrap(), at["write"]);
        assert!(pcs.contains(&(at["mover"] + 2)));
    }

    #[test]
    fn follows_the_random_byte_one_level_back() {
        let (rom, at) = game();
        let (_, t) = recorded(&rom, 10);
        let speed = u16::from(SPEED) + 1;
        let writer = t.last_write(speed).unwrap();
        let f = follow(&t, writer, Loc::Mem(speed), Options::default(), 1);
        assert_eq!(f.feeds.len(), 1, "{:#?}", f.feeds);
        let (loc, sub) = &f.feeds[0];
        assert_eq!(*loc, Loc::Mem(u16::from(RANDOM)));
        assert_eq!(t.steps[sub.slice.writer].pc, at["nmi"]);
        // INC reads the byte it writes, as the frame before left it.
        assert_eq!(sub.slice.reads.len(), 1);
        assert_eq!(sub.slice.reads[0].from, From::Before);
        assert!(sub.feeds.is_empty());
        // The same trace gives the same slice.
        assert_eq!(
            f,
            follow(&t, writer, Loc::Mem(speed), Options::default(), 1)
        );
    }

    #[test]
    fn says_where_it_gives_up() {
        let (rom, _) = game();
        let (_, t) = recorded(&rom, 10);
        let out = u16::from(RAMCODE_OUT);
        let writer = t.last_write(out).unwrap();
        assert_eq!(t.steps[writer].pc, RAMCODE + 2);
        let s = slice(&t, writer, Loc::Mem(out), Options::default());
        // The store and the load before it ran from RAM.
        assert_eq!(
            s.gaps,
            vec![
                Gap::CodeInRam { step: writer - 1 },
                Gap::CodeInRam { step: writer }
            ]
        );
    }

    #[test]
    fn recording_changes_nothing() {
        let (rom, _) = game();
        let (traced, _) = recorded(&rom, 10);
        let mut plain = Nes::power_on(&rom, PowerOn::default()).unwrap();
        for _ in 0..11 {
            plain.step(pads()).unwrap();
        }
        assert_eq!(traced.state_hash().unwrap(), plain.state_hash().unwrap());
        // Calls: the root, the NMI, mover, helper and the RAM routine.
        let (_, t) = recorded(&rom, 10);
        let entries: Vec<u16> = t.calls.iter().map(|c| c.entry).collect();
        assert!(entries.contains(&RAMCODE), "{entries:04X?}");
    }
}
