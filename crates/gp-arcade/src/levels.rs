//! What a run of the board does, kept as levels, as gp-events keeps them
//! for the NES (docs/research/rom-event-model.md), for both Z80s:
//!
//! | Level | What |
//! |---|---|
//! | L0, footprints | Instruction starts with their counts and first frame, the bytes executed, and the RAM bytes written, by address |
//! | L1, edges | Every transfer of control with its kind, count and first frame; for `jp (hl)`, `jp (ix)` and `jp (iy)` each target is an edge of its own |
//! | L2, frame events | Per frame: the board's events, the instructions first seen, and the RAM bytes that changed |
//!
//! A [`Watcher`] steps a [`Tapper`] with its instruction trace, access log
//! and event log on, and adds each frame to the levels. None of them is
//! part of the state, so a watched run reaches the states an unwatched one
//! does.

use std::collections::BTreeMap;

use crate::dasm::{self, Flow};
use crate::events::BoardEvent;
use crate::mcr::{AccessKind, Trace};
use crate::{Controls, Tapper};

/// How control went from one instruction to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeKind {
    /// `jp nn`, `jr e`, and a conditional one taken.
    Jump,
    /// A conditional jump not taken: on to the next instruction.
    Fall,
    Call,
    Rst,
    Return,
    /// `jp (hl)`, `jp (ix)`, `jp (iy)`.
    Indirect,
    /// An interrupt, from the instruction it came after to its handler.
    Interrupt,
    /// `djnz`, `ldir` and the others that go back to themselves.
    Loop,
}

impl EdgeKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Jump => "jump",
            Self::Fall => "fall",
            Self::Call => "call",
            Self::Rst => "rst",
            Self::Return => "return",
            Self::Indirect => "indirect",
            Self::Interrupt => "interrupt",
            Self::Loop => "loop",
        }
    }
}

/// A count and the first frame it was seen on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Seen {
    pub count: u64,
    pub first: u64,
}

impl Seen {
    fn add(&mut self, frame: u64) {
        if self.count == 0 {
            self.first = frame;
        }
        self.count += 1;
    }
}

/// One CPU's levels.
#[derive(Debug, Clone, Default)]
pub struct CpuLevels {
    /// L0: each instruction start.
    pub starts: BTreeMap<u16, Seen>,
    /// L0: each instruction start's length, as the disassembler reads it.
    pub lengths: BTreeMap<u16, u8>,
    /// L0: each byte written (memory, not ports), and by which
    /// instructions.
    pub written: BTreeMap<u16, Seen>,
    pub writers: BTreeMap<u16, BTreeMap<u16, u64>>,
    /// L0: each port read or written.
    pub ports: BTreeMap<(u16, bool), Seen>,
    /// L1: each transfer of control, from, to and kind.
    pub edges: BTreeMap<(u16, u16, EdgeKind), Seen>,
}

impl CpuLevels {
    /// Whether the byte at `addr` was executed: inside some instruction
    /// that started.
    pub fn executed(&self, addr: u16) -> bool {
        self.lengths
            .range(addr.saturating_sub(3)..=addr)
            .any(|(&pc, &len)| addr - pc < u16::from(len))
    }

    /// The bytes executed, as sorted address ranges (end excluded).
    pub fn executed_ranges(&self) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = Vec::new();
        for (&pc, &len) in &self.lengths {
            let (a, b) = (u32::from(pc), u32::from(pc) + u32::from(len));
            match out.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => out.push((a, b)),
            }
        }
        out
    }

    /// Where `jp (hl)` and friends at `pc` went, with counts.
    pub fn indirect_targets(&self, pc: u16) -> BTreeMap<u16, u64> {
        self.edges
            .range((pc, 0, EdgeKind::Jump)..=(pc, u16::MAX, EdgeKind::Loop))
            .filter(|((_, _, k), _)| *k == EdgeKind::Indirect)
            .map(|((_, to, _), s)| (*to, s.count))
            .collect()
    }
}

/// L2: one frame.
#[derive(Debug, Clone, Default)]
pub struct FrameLevels {
    pub frame: u64,
    pub events: Vec<BoardEvent>,
    /// Instructions first run this frame, by CPU and address.
    pub new_code: Vec<(u8, u16)>,
    /// Main-CPU RAM bytes (work, sprite, video, palette) and sound RAM
    /// bytes (as the sound CPU addresses them, 8000 on) whose value
    /// changed this frame.
    pub ram_changed: Vec<(u8, u16)>,
}

/// The levels of a run.
#[derive(Debug, Clone, Default)]
pub struct Levels {
    pub cpus: [CpuLevels; 2],
    /// L2, for each frame watched with frames kept.
    pub frames: Vec<FrameLevels>,
    /// Frames watched.
    pub watched: u64,
}

/// The oracle record's RAMs as each CPU addresses them.
fn ram_addresses() -> impl Iterator<Item = (u8, u16)> {
    let main = (0xe000..0xe800u16)
        .chain(0xe800..0xea00)
        .chain(0xf000..0xf800)
        .chain(0xf800..0xf880);
    main.map(|a| (0u8, a))
        .chain((0x8000..0x8400u16).map(|a| (1u8, a)))
}

/// A CPU's counts while a run is watched: dense, by address, made into
/// [`CpuLevels`] when the run ends.
struct Dense {
    starts: Vec<Seen>,
    /// 0 for an address never decoded; else the length.
    len: Vec<u8>,
    flow: Vec<Flow>,
    written: Vec<Seen>,
    writers: BTreeMap<(u16, u16), u64>,
    ports: BTreeMap<(u16, bool), Seen>,
    edges: std::collections::HashMap<(u16, u16, EdgeKind), Seen>,
    /// The last instruction of the frame before, for an interrupt taken
    /// first thing in a frame.
    last_pc: Option<u16>,
}

impl Dense {
    fn new() -> Self {
        Self {
            starts: vec![Seen::default(); 0x1_0000],
            len: vec![0; 0x1_0000],
            flow: vec![Flow::Next; 0x1_0000],
            written: vec![Seen::default(); 0x1_0000],
            writers: BTreeMap::new(),
            ports: BTreeMap::new(),
            edges: std::collections::HashMap::new(),
            last_pc: None,
        }
    }

    fn levels(&self) -> CpuLevels {
        let mut lv = CpuLevels::default();
        for (a, s) in self.starts.iter().enumerate() {
            if s.count > 0 {
                lv.starts.insert(a as u16, *s);
                lv.lengths.insert(a as u16, self.len[a]);
            }
        }
        for (a, s) in self.written.iter().enumerate() {
            if s.count > 0 {
                lv.written.insert(a as u16, *s);
            }
        }
        for (&(addr, pc), &n) in &self.writers {
            lv.writers.entry(addr).or_default().insert(pc, n);
        }
        lv.ports = self.ports.clone();
        lv.edges = self.edges.iter().map(|(k, v)| (*k, *v)).collect();
        lv
    }
}

/// Steps a machine with everything the levels need logged.
pub struct Watcher {
    pub levels: Levels,
    /// Whether to keep L2 for each frame.
    pub keep_frames: bool,
    dense: [Dense; 2],
}

impl Watcher {
    pub fn new(keep_frames: bool) -> Self {
        Self {
            levels: Levels::default(),
            keep_frames,
            dense: [Dense::new(), Dense::new()],
        }
    }

    /// The levels so far ([`Watcher::levels`] holds them after each step;
    /// this ends the watch without copying them again).
    pub fn into_levels(self) -> Levels {
        self.levels
    }

    /// Runs one frame of `t` with `controls`, watched. Returns the frame's
    /// trace and accesses for a caller that wants them too.
    pub fn step(&mut self, t: &mut Tapper, controls: Controls) -> Watched {
        let w = self.step_quiet(t, controls);
        for c in 0..2 {
            self.levels.cpus[c] = self.dense[c].levels();
        }
        w
    }

    /// [`Watcher::step`] without bringing [`Watcher::levels`]'s per-CPU
    /// levels up to date: for a long run, call [`Watcher::finish`] at its
    /// end.
    pub fn step_quiet(&mut self, t: &mut Tapper, controls: Controls) -> Watched {
        t.set_trace(true);
        t.set_accesses(true);
        t.set_events(true);
        let before = self.keep_frames.then(|| t.ram_record());
        t.step(controls);
        let mut trace = t.take_trace();
        trace.sort_by_key(|e| e.time);
        let accesses = t.take_accesses();
        let events = t.take_events();
        t.set_trace(false);
        t.set_accesses(false);
        t.set_events(false);
        let frame = t.frame();
        let mut new_code = Vec::new();
        for cpu in 0..2u8 {
            let d = &mut self.dense[usize::from(cpu)];
            let mut prev: Option<&Trace> = None;
            let mine = trace.iter().filter(|e| e.cpu == cpu);
            let mut last: Option<&Trace> = None;
            for e in mine {
                // The edge from the instruction before, now that its next is known.
                if e.interrupt.is_some() {
                    if let Some(from) = last.map(|p| p.pc).or(d.last_pc) {
                        d.edges
                            .entry((from, e.pc, EdgeKind::Interrupt))
                            .or_default()
                            .add(frame);
                    }
                } else if let Some(p) = last
                    && let Some(pi) = prev.filter(|x| std::ptr::eq(*x, p))
                {
                    let at = usize::from(pi.pc);
                    let fall = pi.pc.wrapping_add(u16::from(d.len[at]));
                    let next = e.pc;
                    let kind = match d.flow[at] {
                        Flow::Next if next == pi.pc => Some(EdgeKind::Loop),
                        Flow::Next | Flow::Halt => None,
                        Flow::Jump(_) => Some(EdgeKind::Jump),
                        Flow::JumpIf(_) => Some(if next == fall {
                            EdgeKind::Fall
                        } else {
                            EdgeKind::Jump
                        }),
                        Flow::Djnz(_) => Some(if next == fall {
                            EdgeKind::Fall
                        } else {
                            EdgeKind::Loop
                        }),
                        Flow::Call(_) => Some(EdgeKind::Call),
                        Flow::CallIf(_) => Some(if next == fall {
                            EdgeKind::Fall
                        } else {
                            EdgeKind::Call
                        }),
                        Flow::Rst(_) => Some(EdgeKind::Rst),
                        Flow::Ret => Some(EdgeKind::Return),
                        Flow::RetIf => Some(if next == fall {
                            EdgeKind::Fall
                        } else {
                            EdgeKind::Return
                        }),
                        Flow::Indirect(_) => Some(EdgeKind::Indirect),
                    };
                    if let Some(kind) = kind {
                        d.edges.entry((pi.pc, next, kind)).or_default().add(frame);
                    }
                }
                last = Some(e);
                if e.interrupt.is_some() {
                    prev = None;
                    continue;
                }
                let at = usize::from(e.pc);
                if d.starts[at].count == 0 {
                    new_code.push((cpu, e.pc));
                }
                d.starts[at].add(frame);
                if d.len[at] == 0 {
                    let insn = dasm::decode(&e.bytes, e.pc);
                    d.len[at] = insn.len;
                    d.flow[at] = insn.flow;
                }
                prev = Some(e);
            }
            if let Some(p) = prev {
                d.last_pc = Some(p.pc);
            }
        }
        for a in &accesses {
            let d = &mut self.dense[usize::from(a.cpu)];
            match a.kind {
                AccessKind::Write => {
                    d.written[usize::from(a.addr)].add(frame);
                    *d.writers.entry((a.addr, a.pc)).or_default() += 1;
                }
                AccessKind::In | AccessKind::Out => {
                    d.ports
                        .entry((a.addr & 0xff, a.kind == AccessKind::Out))
                        .or_default()
                        .add(frame);
                }
                AccessKind::Read => {}
            }
        }
        if let Some(before) = before {
            let after = t.ram_record();
            let ram_changed = ram_addresses()
                .zip(before.iter().zip(&after))
                .filter(|(_, (x, y))| x != y)
                .map(|(a, _)| a)
                .collect();
            self.levels.frames.push(FrameLevels {
                frame,
                events: events.clone(),
                new_code,
                ram_changed,
            });
        }
        self.levels.watched += 1;
        Watched {
            trace,
            accesses,
            events,
        }
    }

    /// Brings [`Watcher::levels`] up to date after [`Watcher::step_quiet`]s.
    pub fn finish(mut self) -> Levels {
        for c in 0..2 {
            self.levels.cpus[c] = self.dense[c].levels();
        }
        self.levels
    }
}

/// One watched frame's raw logs, in time order for the trace.
pub struct Watched {
    pub trace: Vec<Trace>,
    pub accesses: Vec<crate::mcr::Access>,
    pub events: Vec<BoardEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Happened;

    /// The busy program's levels: its loop and table targets, the bytes it
    /// writes, its ports; and a watched run's states are an unwatched
    /// one's.
    #[test]
    fn the_busy_program_has_its_levels() {
        let set = crate::testrom::busy();
        let mut plain = Tapper::power_on(&set);
        let mut t = Tapper::power_on(&set);
        let mut w = Watcher::new(true);
        for i in 0..6 {
            let c = Controls {
                tap: i % 2 == 0,
                ..Controls::default()
            };
            plain.step(c);
            w.step(&mut t, c);
        }
        assert_eq!(plain.state_hash(), t.state_hash());
        let main = &w.levels.cpus[0];
        assert!(main.starts.contains_key(&0x0003));
        assert!(main.executed(0x0004), "inside `in a,($01)`");
        assert!(!main.executed(0x0030), "the nops between are never run");
        let targets = main.indirect_targets(0x0029);
        assert!(targets.contains_key(&0x0050) || targets.contains_key(&0x0058));
        assert!(main.written.contains_key(&0xe002));
        assert!(main.writers[&0xe002].contains_key(&0x0014));
        assert!(main.ports.contains_key(&(0x01, false)));
        assert!(main.ports.contains_key(&(0x1c, true)));
        assert!(
            main.edges
                .keys()
                .any(|(f, _, k)| *f == 0x0055 && *k == EdgeKind::Jump)
        );
        let first = &w.levels.frames[0];
        assert!(first.new_code.contains(&(0, 0x0000)));
        assert!(first.ram_changed.contains(&(0, 0xe001)));
        assert!(first.events.iter().any(|e| e.what == Happened::AyWrite));
        assert!(w.levels.frames[1].new_code.len() < first.new_code.len());
        let sound = &w.levels.cpus[1];
        assert!(sound.written.contains_key(&0xa002));
    }
}
