//! Comparing two recordings frame by frame, field by field.
//!
//! The diff compares every field the two field tables share by name and
//! width, frame by frame, and stops at a divergence: a frame where a field
//! differs. Each divergence gets classes, which are what triage names
//! ([`crate::triage`]): the name of each field that differs, or
//! `frame-shift` when one side's state matches the other's next or previous
//! frame (a frame was skipped or repeated), or a boundary class when the
//! two frames ended a few cycles apart ([`boundary_class`]). Then it
//! resyncs and goes on to the next divergence:
//!
//! - A frame shift moves the alignment by one frame.
//! - With side A running live in `gp-nes` (it needs the ROM), every
//!   differing field [`gp_nes::poke`] can write is copied from B's state
//!   into the console before the next frame, so the next frame starts
//!   where B's did. A field copied [`MAX_COPIES`] frames in a row that
//!   still differs is set aside instead; the count starts again once the
//!   two agree on it.
//! - The rest, and every field when A comes from its trace, are set aside
//!   with what their difference looks like: an integer's difference, or a
//!   memory's differing bytes. A set-aside field is reported again when
//!   its difference changes, or when a byte outside the set starts to
//!   differ, and compared normally once the two agree.

use std::collections::{BTreeMap, BTreeSet};

use gp_evidence::InputLog;
use gp_evidence::events::{Event, EventKind};
use gp_evidence::manifest::PowerOn;
use gp_evidence::trace::{FieldTable, Width};
use gp_nes::{Nes, canonical, poke};
use serde::Serialize;

use crate::states::States;

/// How many frames in a row a field is copied before it's set aside.
pub const MAX_COPIES: u32 = 3;

/// The class of a divergence where one side skipped or repeated a frame.
pub const FRAME_SHIFT: &str = "frame-shift";
/// The class of a frame that ended on one side of an NMI entry on A and
/// the other side on B: one state is at the instruction before the NMI
/// sequence, the other at the handler's first instruction.
/// [`boundary_class`] says which fields it explains.
pub const NMI_BOUNDARY: &str = "nmi-boundary";
/// The class of a frame that ended on different instructions on A and B,
/// with no NMI entry between them ([`boundary_class`]).
pub const BOUNDARY_INSTRUCTION: &str = "boundary-instruction";
/// The class of a frame whose two ends are at the same CPU cycle but whose
/// PPU positions differ by the run's alignment ([`diff`] measures it at
/// frame 1): the two runtimes' PPUs run that many dots apart.
pub const PPU_ALIGNMENT: &str = "ppu-alignment";
/// The class of frames the two sides disagree are lag frames.
pub const LAG: &str = "lag";

/// Bits of `cpu.p` the comparison ignores: 4 (B) and 5 (unused) aren't
/// flags in the register, only in the copy pushed to the stack, and the two
/// runtimes hold them differently.
const P_IGNORED: u8 = 0x30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldDiff {
    pub field: String,
    pub a: String,
    pub b: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Divergence {
    /// B's frame; A's is `a_frame`, the same unless frames have shifted.
    pub frame: u64,
    pub a_frame: u64,
    pub classes: Vec<String>,
    pub fields: Vec<FieldDiff>,
    /// What the resync did: which fields it copied and which it set aside.
    pub resync: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassSummary {
    pub class: String,
    pub first_frame: u64,
    pub divergences: usize,
    /// B's frame of each divergence of the class, which triage scopes
    /// check ([`crate::triage`]).
    pub frames: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventKindSummary {
    pub kind: String,
    pub a_events: usize,
    pub b_events: usize,
    /// The frame (B's) of the first event that differs in address or
    /// value, or that one side has and the other doesn't.
    pub first_frame: Option<u64>,
    /// Events that match but fell in another frame (a frame boundary on
    /// the other side of them).
    pub other_frame: usize,
    /// Over events that match in the same frame, B's position minus A's
    /// in dots, and how many events had that offset, most common first (at
    /// most 5).
    pub offsets: Vec<(i64, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// `live` (A rerun in gp-nes, with resync by copying) or `trace`.
    pub mode: &'static str,
    pub frames: u64,
    pub compared: Vec<String>,
    pub only_a: Vec<String>,
    pub only_b: Vec<String>,
    pub divergences: Vec<Divergence>,
    /// Whether the diff stopped at the divergence limit.
    pub truncated: bool,
    pub classes: Vec<ClassSummary>,
    /// Frames the two input logs disagree on being lag frames.
    pub lag_frames: Vec<u64>,
    /// Per event kind both record, when both recordings have events.
    pub events: Vec<EventKindSummary>,
}

impl Report {
    pub fn first(&self) -> Option<&Divergence> {
        self.divergences.first()
    }

    /// Every class found, the lag and event ones included.
    pub fn class_names(&self) -> Vec<String> {
        self.classes.iter().map(|c| c.class.clone()).collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error(transparent)]
    Chunk(#[from] gp_evidence::chunk::ChunkError),
    #[error(transparent)]
    Nes(#[from] gp_nes::NesError),
    #[error("{0}")]
    Invalid(String),
}

/// Side A of a diff when it runs live: a `gp-nes` console on the
/// recording's ROM, power-on state and input.
pub struct LiveA<'a> {
    pub rom: &'a [u8],
    pub power_on: PowerOn,
    pub input: &'a InputLog,
}

struct Live<'a> {
    nes: Nes,
    input: &'a InputLog,
    state: Vec<u8>,
    prev: Option<Vec<u8>>,
    pending: Vec<(String, Vec<u8>)>,
}

impl<'a> Live<'a> {
    fn new(a: &LiveA<'a>) -> Result<Self, DiffError> {
        let nes = Nes::power_on(a.rom, a.power_on)?;
        let state = nes.canonical_state()?;
        Ok(Self {
            nes,
            input: a.input,
            state,
            prev: None,
            pending: Vec::new(),
        })
    }

    fn step(
        nes: &mut Nes,
        input: &InputLog,
        pending: &[(String, Vec<u8>)],
    ) -> Result<(), DiffError> {
        let frame = input
            .frames
            .get(nes.frame() as usize)
            .copied()
            .ok_or_else(|| DiffError::Invalid("ran past the input log".into()))?;
        let mut done = pending.is_empty();
        let mut failed = None;
        nes.step_with(frame, |bus| {
            if !done {
                for (name, value) in pending {
                    if let Err(e) = poke::write_field(bus, name, value) {
                        failed = Some(e.to_string());
                    }
                }
                done = true;
            }
        })?;
        match failed {
            Some(e) => Err(DiffError::Invalid(e)),
            None => Ok(()),
        }
    }

    /// Runs forward to `frame`.
    fn at(&mut self, frame: u64) -> Result<&[u8], DiffError> {
        while self.nes.frame() < frame {
            let pending = std::mem::take(&mut self.pending);
            Self::step(&mut self.nes, self.input, &pending)?;
            self.prev = Some(std::mem::replace(
                &mut self.state,
                self.nes.canonical_state()?,
            ));
        }
        if self.nes.frame() != frame {
            return Err(DiffError::Invalid(format!(
                "the live console is at frame {}, past {frame}",
                self.nes.frame()
            )));
        }
        Ok(&self.state)
    }

    /// The state one frame on, without moving.
    fn peek_next(&self) -> Result<Option<Vec<u8>>, DiffError> {
        if self.nes.frame() as usize >= self.input.frames.len() {
            return Ok(None);
        }
        let mut copy = self.nes.clone();
        Self::step(&mut copy, self.input, &self.pending)?;
        Ok(Some(copy.canonical_state()?))
    }
}

enum Side<'a> {
    Trace(&'a States),
    Live(Box<Live<'a>>),
}

impl Side<'_> {
    fn frames(&self) -> u64 {
        match self {
            Side::Trace(s) => s.states.len() as u64 - 1,
            Side::Live(l) => l.input.frames.len() as u64,
        }
    }

    fn state(&mut self, frame: u64) -> Result<Vec<u8>, DiffError> {
        match self {
            Side::Trace(s) => Ok(s.states[frame as usize].clone()),
            Side::Live(l) => Ok(l.at(frame)?.to_vec()),
        }
    }

    fn next(&mut self, frame: u64) -> Result<Option<Vec<u8>>, DiffError> {
        match self {
            Side::Trace(s) => Ok(s.states.get(frame as usize + 1).cloned()),
            Side::Live(l) => l.peek_next(),
        }
    }

    fn prev(&mut self, frame: u64) -> Option<Vec<u8>> {
        match self {
            Side::Trace(s) => frame
                .checked_sub(1)
                .and_then(|f| s.states.get(f as usize).cloned()),
            Side::Live(l) => l.prev.clone(),
        }
    }
}

/// Field names both tables have with the same width, in A's order, and
/// those only one has (or has with another width).
fn shared(a: &FieldTable, b: &FieldTable) -> (Vec<String>, Vec<String>, Vec<String>) {
    let bw: BTreeMap<&str, Width> = b
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.width))
        .collect();
    let aw: BTreeMap<&str, Width> = a
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.width))
        .collect();
    let both: Vec<String> = a
        .fields
        .iter()
        .filter(|f| bw.get(f.name.as_str()) == Some(&f.width))
        .map(|f| f.name.clone())
        .collect();
    let only_a = a
        .fields
        .iter()
        .filter(|f| bw.get(f.name.as_str()) != Some(&f.width))
        .map(|f| f.name.clone())
        .collect();
    let only_b = b
        .fields
        .iter()
        .filter(|f| aw.get(f.name.as_str()) != Some(&f.width))
        .map(|f| f.name.clone())
        .collect();
    (both, only_a, only_b)
}

fn describe(field: &str, a: &[u8], b: &[u8]) -> (String, String) {
    let int = |v: &[u8]| {
        let mut n = 0u64;
        for (i, byte) in v.iter().enumerate() {
            n |= u64::from(*byte) << (8 * i);
        }
        format!("0x{n:0w$x} ({n})", w = v.len() * 2)
    };
    if a.len() == b.len() && a.len() <= 8 {
        return (int(a), int(b));
    }
    if a.len() != b.len() {
        return (format!("{} bytes", a.len()), format!("{} bytes", b.len()));
    }
    let differing: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
    let shown: Vec<String> = differing
        .iter()
        .take(4)
        .map(|&i| {
            if field == "ram" {
                format!("${i:04x}")
            } else {
                format!("+{i}")
            }
        })
        .collect();
    let at = |v: &[u8]| {
        let bytes: Vec<String> = differing
            .iter()
            .take(4)
            .map(|&i| format!("{:02x}", v[i]))
            .collect();
        format!(
            "{} of {} bytes differ, at {}: {}",
            differing.len(),
            a.len(),
            shown.join(" "),
            bytes.join(" ")
        )
    };
    (at(a), at(b))
}

/// A field that differs: its name, then A's and B's bytes.
type Differing = (String, Vec<u8>, Vec<u8>);

/// A state split into its fields, by name.
type Fields<'s> = BTreeMap<&'s str, &'s [u8]>;

struct Compare<'t> {
    a: &'t FieldTable,
    b: &'t FieldTable,
    fields: BTreeSet<String>,
}

impl Compare<'_> {
    /// Every compared field that differs, in A's order.
    fn diffs(&self, sa: &[u8], sb: &[u8]) -> Result<Vec<Differing>, DiffError> {
        let fb: Fields = self.b.split(sb)?.into_iter().collect();
        let mut out = Vec::new();
        for (name, va) in self.a.split(sa)? {
            if !self.fields.contains(name) {
                continue;
            }
            let vb = fb[name];
            if !same(name, va, vb) {
                out.push((name.to_owned(), va.to_vec(), vb.to_vec()));
            }
        }
        Ok(out)
    }
}

fn same(name: &str, va: &[u8], vb: &[u8]) -> bool {
    if name == "cpu.p" && va.len() == 1 && vb.len() == 1 {
        va[0] & !P_IGNORED == vb[0] & !P_IGNORED
    } else {
        va == vb
    }
}

/// B's cycle count minus A's. Both are 64-bit counts (gp-nes field table
/// 3, Mesen table 2), so no count wraps in practice.
fn cycles_apart(a: i64, b: i64) -> i64 {
    b.wrapping_sub(a)
}

fn le(v: &[u8]) -> u64 {
    v.iter()
        .enumerate()
        .fold(0, |n, (i, b)| n | u64::from(*b) << (8 * i))
}

/// What a set-aside difference looks like, so the diff can tell the same
/// difference lasting from one that changes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Signature {
    /// An integer field: B's value minus A's (with `cpu.p`'s ignored bits
    /// cleared). The same difference lasts while this stays the same.
    Delta(i128),
    /// A memory: the byte offsets that differ. A byte outside the set that
    /// starts to differ is a new difference.
    Bytes(BTreeSet<usize>),
    /// A region of another size on each side.
    Sizes(usize, usize),
}

/// The fields that place a state in time. A live diff copies them only
/// from a recording of the same runtime.
const TIMING: [&str; 4] = ["cpu.cycles", "ppu.scanline", "ppu.dot", "ppu.odd_frame"];

/// Fields that are memories, compared byte by byte; every other field is
/// an integer.
const MEMORIES: [&str; 6] = ["ram", "oam", "palette", "nametables", "chr_ram", "prg_ram"];

fn signature(name: &str, va: &[u8], vb: &[u8]) -> Signature {
    if va.len() != vb.len() {
        return Signature::Sizes(va.len(), vb.len());
    }
    if !MEMORIES.contains(&name) && va.len() <= 8 {
        let mask = if name == "cpu.p" { !P_IGNORED } else { 0xFF };
        let v = |x: &[u8]| {
            let mut x = x.to_vec();
            if name == "cpu.p" {
                x[0] &= mask;
            }
            i128::from(le(&x))
        };
        if name == "cpu.cycles" {
            return Signature::Delta(i128::from(cycles_apart(v(va) as i64, v(vb) as i64)));
        }
        return Signature::Delta(v(vb) - v(va));
    }
    Signature::Bytes((0..va.len()).filter(|&i| va[i] != vb[i]).collect())
}

/// The part of a difference that a set-aside signature doesn't cover, or
/// `None` if it covers all of it.
fn uncovered(now: &Signature, accepted: &Signature) -> Option<Signature> {
    match (now, accepted) {
        (Signature::Bytes(n), Signature::Bytes(a)) => {
            let new: BTreeSet<usize> = n.difference(a).copied().collect();
            (!new.is_empty()).then_some(Signature::Bytes(new))
        }
        (n, a) if n == a => None,
        (n, _) => Some(n.clone()),
    }
}

/// Fields the APU frame counter clocks (its step, and the quarter- and
/// half-frame units). A frame counter tick can fall between two frame ends
/// a few cycles apart, so a boundary class explains these when they're at
/// most 1 apart ([`boundary_class`]).
const FRAME_COUNTER_CLOCKED: [&str; 19] = [
    "apu.frame_counter.step",
    "apu.frame_counter.irq_pending",
    "apu.pulse1.length.counter",
    "apu.pulse1.envelope.start",
    "apu.pulse1.envelope.divider",
    "apu.pulse1.envelope.decay",
    "apu.pulse1.sweep.reload",
    "apu.pulse1.sweep.divider",
    "apu.pulse2.length.counter",
    "apu.pulse2.envelope.start",
    "apu.pulse2.envelope.divider",
    "apu.pulse2.envelope.decay",
    "apu.pulse2.sweep.reload",
    "apu.pulse2.sweep.divider",
    "apu.triangle.length.counter",
    "apu.triangle.linear.counter",
    "apu.triangle.linear.reload",
    "apu.noise.length.counter",
    "apu.noise.envelope.decay",
];

/// Whether two values of a frame-counter-clocked field are one tick apart:
/// 1 apart, or for the frame counter's step, the wrap from its last step
/// (3, or 4 in 5-step mode) back to 0.
fn one_tick(field: &str, x: i64, y: i64) -> bool {
    (x - y).abs() <= 1
        || (field == "apu.frame_counter.step" && x.min(y) == 0 && (3..=4).contains(&x.max(y)))
}

/// The most CPU cycles two frame ends can be apart and still be the same
/// boundary reached one instruction apart: the longest instruction.
pub const MAX_INSTRUCTION_CYCLES: i64 = 7;
/// The cycles of an NMI entry.
pub const NMI_ENTRY_CYCLES: i64 = 7;
/// How far, in dots, the PPU positions of two frame ends can stray from 3
/// dots per CPU cycle between them plus the run's alignment: a position is
/// read partway into a cycle.
pub const POSITION_SLACK: i64 = 2;
/// How many cycles earlier one side can take an NMI because of the
/// alignment: its PPU reaches vblank up to 2 cycles (5 dots) sooner.
pub const ALIGNMENT_CYCLES: i64 = 2;

/// What a boundary class accepts of a field.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Accept {
    /// The field's whole difference.
    Whole,
    /// These byte offsets of a memory.
    Bytes(BTreeSet<usize>),
}

/// Whether the two frames ended at different instructions near the same
/// point, and what of the difference that explains. Both need `cpu.cycles`
/// to differ; equal cycle counts with another PC are a real split in
/// control flow, not a boundary.
///
/// - [`NMI_BOUNDARY`]: `cpu.s` is 3 lower on one side, whose stack holds
///   the other side's PC (or the next instruction's, up to 3 bytes on)
///   with its P in the three bytes above S, whose I flag is set, and whose
///   cycle count is 7 to 14 ahead (the entry, and at most one instruction).
///   It explains `cpu.s`, `cpu.pc`, `cpu.cycles`, `cpu.p` if only I (and
///   bits 4-5) differ, and the three pushed bytes of `ram`.
/// - [`BOUNDARY_INSTRUCTION`]: otherwise, the cycle counts are at most
///   [`MAX_INSTRUCTION_CYCLES`] apart. It explains `cpu.pc`, `cpu.cycles`
///   and the registers an instruction or two can change (`cpu.a`, `cpu.x`,
///   `cpu.y`, `cpu.p`); memory is compared as always.
///
/// Either also explains the PPU position when it moved 3 dots per cycle
/// (within [`POSITION_SLACK`]), and what the APU frame counter clocks
/// ([`FRAME_COUNTER_CLOCKED`]) when it's at most 1 apart.
fn boundary_class(
    fa: &Fields,
    fb: &Fields,
    all: &[Differing],
    align: i64,
) -> Option<(&'static str, BTreeMap<&'static str, Accept>)> {
    let differs = |name: &str| all.iter().any(|(n, _, _)| n == name);
    let num = |f: &Fields, name: &str| f.get(name).map(|v| le(v) as i64);
    let dc = cycles_apart(num(fa, "cpu.cycles")?, num(fb, "cpu.cycles")?);
    let pos = |f: &Fields| Some(num(f, "ppu.scanline")? * 341 + num(f, "ppu.dot")?);
    let position_fits = match (pos(fa), pos(fb)) {
        (Some(pa), Some(pb)) => ((pb - pa) - 3 * dc - align).abs() <= POSITION_SLACK,
        _ => false,
    };
    let mut out = BTreeMap::new();
    if dc == 0 {
        // The same cycle count: only the PPU alignment can explain a
        // difference, and only in the position.
        if align != 0 && position_fits && !differs("cpu.pc") {
            out.insert("ppu.scanline", Accept::Whole);
            out.insert("ppu.dot", Accept::Whole);
            return Some((PPU_ALIGNMENT, out));
        }
        return None;
    }
    let (sa, sb) = (num(fa, "cpu.s")?, num(fb, "cpu.s")?);
    let class = if (sa - sb).abs() == 3 {
        // The side with the lower S took the NMI entry.
        let (pre, post, steps) = if sb < sa { (fa, fb, dc) } else { (fb, fa, -dc) };
        if !(NMI_ENTRY_CYCLES - ALIGNMENT_CYCLES..=NMI_ENTRY_CYCLES + MAX_INSTRUCTION_CYCLES)
            .contains(&steps)
        {
            return None;
        }
        let post_s = num(post, "cpu.s")? as usize;
        let ram = post.get("ram")?;
        let byte = |i: usize| ram.get(0x100 + ((post_s + i) & 0xFF)).copied();
        let pushed_p = byte(1)?;
        let pushed_pc = u16::from(byte(2)?) | u16::from(byte(3)?) << 8;
        let pre_pc = num(pre, "cpu.pc")? as u16;
        if pushed_pc.wrapping_sub(pre_pc) > 3 {
            return None;
        }
        let post_p = num(post, "cpu.p")? as u8;
        if post_p & 0x04 == 0 {
            return None;
        }
        let pre_p = num(pre, "cpu.p")? as u8;
        if pushed_pc == pre_pc && (pushed_p ^ pre_p) & !P_IGNORED != 0 {
            return None;
        }
        out.insert("cpu.s", Accept::Whole);
        if (pre_p ^ post_p) & !(P_IGNORED | 0x04) == 0 {
            out.insert("cpu.p", Accept::Whole);
        }
        let pushed = (1..=3).map(|i| 0x100 + ((post_s + i) & 0xFF)).collect();
        out.insert("ram", Accept::Bytes(pushed));
        NMI_BOUNDARY
    } else {
        if dc.abs() > MAX_INSTRUCTION_CYCLES || sa != sb {
            return None;
        }
        for r in ["cpu.a", "cpu.x", "cpu.y", "cpu.p"] {
            out.insert(r, Accept::Whole);
        }
        BOUNDARY_INSTRUCTION
    };
    out.insert("cpu.pc", Accept::Whole);
    out.insert("cpu.cycles", Accept::Whole);
    if position_fits {
        out.insert("ppu.scanline", Accept::Whole);
        out.insert("ppu.dot", Accept::Whole);
    }
    for f in FRAME_COUNTER_CLOCKED {
        if differs(f)
            && let (Some(x), Some(y)) = (num(fa, f), num(fb, f))
            && one_tick(f, x, y)
        {
            out.insert(f, Accept::Whole);
        }
    }
    Some((class, out))
}

/// B's PPU position minus A's, less 3 dots per CPU cycle B is ahead: how
/// far apart the two PPUs run at the same CPU cycle. 0 without a position.
fn alignment(cmp: &Compare<'_>, sa: &[u8], sb: &[u8]) -> Result<i64, DiffError> {
    let fa: Fields = cmp.a.split(sa)?.into_iter().collect();
    let fb: Fields = cmp.b.split(sb)?.into_iter().collect();
    let num = |f: &Fields, name: &str| f.get(name).map(|v| le(v) as i64);
    let at = |f: &Fields| {
        Some((
            num(f, "ppu.scanline")? * 341 + num(f, "ppu.dot")?,
            num(f, "cpu.cycles")?,
        ))
    };
    Ok(match (at(&fa), at(&fb)) {
        (Some((pa, ca)), Some((pb, cb))) => (pb - pa) - 3 * cycles_apart(ca, cb),
        _ => 0,
    })
}

/// Which differences in `d` a boundary class explains; for `ram`, only if
/// every differing byte is one it accepts.
fn explained(
    d: &[(String, Signature)],
    accept: &BTreeMap<&'static str, Accept>,
) -> BTreeSet<String> {
    d.iter()
        .filter(|(name, sig)| match (accept.get(name.as_str()), sig) {
            (Some(Accept::Whole), _) => true,
            (Some(Accept::Bytes(ok)), Signature::Bytes(diff)) => diff.is_subset(ok),
            _ => false,
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// Compares A with B. `live` runs A in `gp-nes` (A must then be a
/// `gp-nes` recording, with its ROM) so the diff can resync by copying;
/// without it, A comes from its trace. Stops after `max` divergences.
pub fn diff(
    a: &States,
    b: &States,
    live: Option<LiveA<'_>>,
    max: usize,
) -> Result<Report, DiffError> {
    let (fields, only_a, only_b) = shared(&a.table, &b.table);
    let cmp = Compare {
        a: &a.table,
        b: &b.table,
        fields: fields.iter().cloned().collect(),
    };
    let mut side = match &live {
        Some(l) => {
            let canonical_table = canonical::field_table();
            if a.table != canonical_table {
                return Err(DiffError::Invalid(
                    "a live diff needs A to be a gp-nes recording".into(),
                ));
            }
            Side::Live(Box::new(Live::new(l)?))
        }
        None => Side::Trace(a),
    };
    let nb = b.states.len() as u64 - 1;
    let na = side.frames();
    let same_runtime = a.table == b.table;
    // How many dots B's PPU runs ahead of A's at the same CPU cycle,
    // measured at frame 1 (frame 0 is the power-on, where gp-nes's PPU
    // hasn't settled its alignment yet).
    let mut align: i64 = 0;
    // Differences set aside, by the signature they had then.
    let mut aside: BTreeMap<String, Signature> = BTreeMap::new();
    // Copies of each field in a row, since it last agreed.
    let mut copies: BTreeMap<String, u32> = BTreeMap::new();
    let mut shift: i64 = 0;
    let mut divergences = Vec::new();
    let mut truncated = false;
    let mut compared_frames = 0;
    for fb in 0..=nb {
        let fa = fb as i64 + shift;
        if fa < 0 || fa as u64 > na {
            break;
        }
        let fa = fa as u64;
        let sa = side.state(fa)?;
        let sb = &b.states[fb as usize];
        compared_frames = fb;
        let all = cmp.diffs(&sa, sb)?;
        if fb == 1 {
            align = alignment(&cmp, &sa, sb)?;
        }
        copies.retain(|name, _| all.iter().any(|(n, _, _)| n == name));
        // What's set aside shrinks to what still differs.
        aside.retain(|name, _| all.iter().any(|(n, _, _)| n == name));
        for (name, va, vb) in &all {
            if let (Some(Signature::Bytes(kept)), Signature::Bytes(now)) =
                (aside.get_mut(name), signature(name, va, vb))
            {
                kept.retain(|i| now.contains(i));
            }
        }
        // The differences nothing set aside covers, each with the part that
        // isn't covered.
        let new: Vec<(String, Signature)> = all
            .iter()
            .filter_map(|(name, va, vb)| {
                let now = signature(name, va, vb);
                match aside.get(name) {
                    None => Some((name.clone(), now)),
                    Some(acc) => uncovered(&now, acc).map(|part| (name.clone(), part)),
                }
            })
            .collect();
        if new.is_empty() {
            continue;
        }
        if divergences.len() >= max {
            truncated = true;
            break;
        }
        let fields: Vec<FieldDiff> = all
            .iter()
            .filter(|(n, _, _)| new.iter().any(|(m, _)| m == n))
            .map(|(name, va, vb)| {
                let (a, b) = describe(name, va, vb);
                FieldDiff {
                    field: name.clone(),
                    a,
                    b,
                }
            })
            .collect();
        let matches = |other: &[u8]| -> Result<bool, DiffError> {
            Ok(cmp.diffs(other, sb)?.iter().all(|(name, va, vb)| {
                aside
                    .get(name)
                    .is_some_and(|acc| uncovered(&signature(name, va, vb), acc).is_none())
            }))
        };
        let ahead = match side.next(fa)? {
            Some(next) => matches(&next)?,
            None => false,
        };
        let behind = !ahead
            && match side.prev(fa) {
                Some(prev) => matches(&prev)?,
                None => false,
            };
        if ahead || behind {
            shift += if ahead { 1 } else { -1 };
            divergences.push(Divergence {
                frame: fb,
                a_frame: fa,
                classes: vec![FRAME_SHIFT.into()],
                fields,
                resync: if ahead {
                    format!(
                        "B's frame {fb} is A's frame {}; compared one frame later on A from here",
                        fa + 1
                    )
                } else {
                    format!(
                        "B's frame {fb} is A's frame {}; compared one frame earlier on A from here",
                        fa - 1
                    )
                },
            });
            continue;
        }

        let map_a: Fields = cmp.a.split(&sa)?.into_iter().collect();
        let map_b: Fields = cmp.b.split(sb)?.into_iter().collect();
        let mut classes = Vec::new();
        let mut absorbed = BTreeSet::new();
        if let Some((class, accept)) = boundary_class(&map_a, &map_b, &all, align) {
            absorbed = explained(&new, &accept);
            if !absorbed.is_empty() {
                classes.push(class.to_owned());
            }
        }
        classes.extend(
            new.iter()
                .filter(|(n, _)| !absorbed.contains(n))
                .map(|(n, _)| n.clone()),
        );

        // Resync. Live, at a divergence with a class beyond the boundary
        // classes, every difference that can be copied is (the whole
        // divergent state, so the next frame starts where B's did), unless
        // a field has been copied MAX_COPIES frames in a row and still
        // differs. When B is another runtime, what a boundary class explains
        // stays, and so does the timing ([`TIMING`]): the two CPUs and PPUs
        // are aligned differently, and copying B's position would move A to
        // an alignment its own clocks don't have. The rest is set aside
        // with its current signature.
        let real = classes
            .iter()
            .any(|c| ![BOUNDARY_INSTRUCTION, NMI_BOUNDARY, PPU_ALIGNMENT].contains(&c.as_str()));
        let mut copied = Vec::new();
        let mut set_aside = Vec::new();
        for (name, va, vb) in &all {
            let n = copies.entry(name.clone()).or_default();
            let can_copy = matches!(side, Side::Live(_))
                && real
                && poke::copyable(name)
                && va.len() == vb.len()
                && (same_runtime || !(TIMING.contains(&name.as_str()) || absorbed.contains(name)))
                && *n < MAX_COPIES;
            if can_copy {
                *n += 1;
                if let Side::Live(l) = &mut side {
                    l.pending
                        .push((name.clone(), map_b[name.as_str()].to_vec()));
                }
                if new.iter().any(|(m, _)| m == name) {
                    copied.push(name.clone());
                }
            } else {
                aside.insert(name.clone(), signature(name, va, vb));
                if new.iter().any(|(m, _)| m == name) {
                    set_aside.push(name.clone());
                }
            }
        }
        let mut resync = Vec::new();
        if !copied.is_empty() {
            let rest = if matches!(side, Side::Live(_)) && copied.len() < all.len() {
                " (and every other copyable field that differs)"
            } else {
                ""
            };
            resync.push(format!("copied {} from B{rest}", copied.join(", ")));
        }
        if !set_aside.is_empty() {
            resync.push(format!("set aside {}", set_aside.join(", ")));
        }
        divergences.push(Divergence {
            frame: fb,
            a_frame: fa,
            classes,
            fields,
            resync: resync.join("; "),
        });
    }

    let mut classes: BTreeMap<String, ClassSummary> = BTreeMap::new();
    for d in &divergences {
        for c in &d.classes {
            let s = classes.entry(c.clone()).or_insert(ClassSummary {
                class: c.clone(),
                first_frame: d.frame,
                divergences: 0,
                frames: Vec::new(),
            });
            s.divergences += 1;
            s.frames.push(d.frame);
        }
    }
    Ok(Report {
        mode: if live.is_some() { "live" } else { "trace" },
        frames: compared_frames,
        compared: fields,
        only_a,
        only_b,
        divergences,
        truncated,
        classes: classes.into_values().collect(),
        lag_frames: Vec::new(),
        events: Vec::new(),
    })
}

/// Adds the lag comparison: frames where one input log says the game read
/// the controllers and the other says it didn't.
pub fn add_lag(report: &mut Report, a: &InputLog, b: &InputLog) {
    report.lag_frames = a
        .frames
        .iter()
        .zip(&b.frames)
        .enumerate()
        .filter(|(_, (x, y))| x.polled != y.polled)
        .map(|(i, _)| i as u64 + 1)
        .collect();
    if let Some(&first) = report.lag_frames.first() {
        report.classes.push(ClassSummary {
            class: LAG.into(),
            first_frame: first,
            divergences: report.lag_frames.len(),
            frames: report.lag_frames.clone(),
        });
    }
}

/// The event kinds the Mesen capture records, so the ones compared.
pub const COMPARED_EVENTS: [EventKind; 6] = [
    EventKind::Nmi,
    EventKind::Irq,
    EventKind::ControllerRead,
    EventKind::ControllerWrite,
    EventKind::PpuWrite,
    EventKind::ApuWrite,
];

/// Adds the event comparison: each kind's events over the whole run, in
/// order, compared by address and value, so a frame boundary that falls on
/// the other side of an event doesn't count as a difference (it's counted
/// in `other_frame`). A kind whose sequences differ is a class,
/// `events.<kind>`. Positions are a summary only, since they follow from
/// the state's own timing.
pub fn add_events(report: &mut Report, a: &[Vec<Event>], b: &[Vec<Event>]) {
    let dot = |e: &Event| i64::from(e.scanline) * 341 + i64::from(e.dot);
    for kind in COMPARED_EVENTS {
        let ka: Vec<&Event> = a.iter().flatten().filter(|e| e.kind == kind).collect();
        let kb: Vec<&Event> = b.iter().flatten().filter(|e| e.kind == kind).collect();
        let common = ka.len().min(kb.len());
        let mismatch =
            (0..common).find(|&i| ka[i].addr != kb[i].addr || ka[i].value != kb[i].value);
        // Events one side has past the other's last are a difference unless
        // they're all in the last frame: its end can fall on either side
        // of them, like any frame boundary.
        let last = (a.len().min(b.len()) as u64).saturating_sub(1);
        let extra_in_last_frame = ka[common..]
            .iter()
            .chain(&kb[common..])
            .all(|e| e.frame >= last);
        let first = match mismatch {
            Some(i) => Some(kb[i].frame),
            None if !extra_in_last_frame => kb.get(common).or(ka.get(common)).map(|e| e.frame),
            None => None,
        };
        let mut offsets: BTreeMap<i64, usize> = BTreeMap::new();
        let mut other_frame = 0;
        for (x, y) in ka.iter().zip(&kb).take(mismatch.unwrap_or(common)) {
            if x.frame == y.frame {
                *offsets.entry(dot(y) - dot(x)).or_default() += 1;
            } else {
                other_frame += 1;
            }
        }
        let mut offsets: Vec<(i64, usize)> = offsets.into_iter().collect();
        offsets.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
        offsets.truncate(5);
        if let Some(f) = first {
            report.classes.push(ClassSummary {
                class: format!("events.{}", kind.name()),
                first_frame: f,
                divergences: 1,
                frames: vec![f],
            });
        }
        report.events.push(EventKindSummary {
            kind: kind.name().into(),
            a_events: ka.len(),
            b_events: kb.len(),
            first_frame: first,
            other_frame,
            offsets,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> FieldTable {
        use gp_evidence::trace::FieldDef;
        FieldTable {
            state_magic: *b"TEST",
            state_version: 1,
            fields: vec![
                FieldDef {
                    name: "cpu.p".into(),
                    width: Width::Fixed(1),
                },
                FieldDef {
                    name: "ram".into(),
                    width: Width::Fixed(4),
                },
            ],
        }
    }

    fn state(p: u8, ram: [u8; 4]) -> Vec<u8> {
        let mut s = b"TEST".to_vec();
        s.extend_from_slice(&1u32.to_le_bytes());
        s.push(p);
        s.extend_from_slice(&ram);
        s
    }

    fn states(list: Vec<Vec<u8>>) -> States {
        States {
            table: table(),
            states: list,
        }
    }

    #[test]
    fn equal_runs_have_no_divergence() {
        let a = states((0..5).map(|i| state(0x24, [i, 0, 0, 0])).collect());
        let r = diff(&a, &a.clone(), None, 10).unwrap();
        assert!(r.divergences.is_empty());
        assert_eq!(r.frames, 4);
    }

    #[test]
    fn unused_status_bits_are_ignored() {
        let a = states(vec![state(0x24, [0; 4]), state(0xa4, [1, 0, 0, 0])]);
        let b = states(vec![state(0x04, [0; 4]), state(0x94, [1, 0, 0, 0])]);
        assert!(diff(&a, &b, None, 10).unwrap().divergences.is_empty());
    }

    #[test]
    fn a_trace_diff_reports_each_field_once() {
        let a = states((0..6).map(|i| state(0, [i, 0, 0, 0])).collect());
        let mut b = a.clone();
        for s in &mut b.states[3..] {
            s[9] ^= 0xff;
        }
        b.states[5][8] = 0x80;
        let r = diff(&a, &b, None, 10).unwrap();
        assert_eq!(r.divergences.len(), 2);
        assert_eq!(r.divergences[0].frame, 3);
        assert_eq!(r.divergences[0].classes, vec!["ram"]);
        assert!(r.divergences[0].resync.contains("set aside ram"));
        assert_eq!(r.divergences[1].frame, 5);
        assert_eq!(r.divergences[1].classes, vec!["cpu.p"]);
    }

    #[test]
    fn a_field_that_agrees_again_is_compared_again() {
        let a = states((0..8).map(|i| state(0, [i, 0, 0, 0])).collect());
        let mut b = a.clone();
        b.states[2][10] = 7;
        b.states[5][10] = 7;
        let r = diff(&a, &b, None, 10).unwrap();
        let frames: Vec<u64> = r.divergences.iter().map(|d| d.frame).collect();
        assert_eq!(frames, vec![2, 5]);
    }

    #[test]
    fn a_skipped_frame_is_a_frame_shift() {
        let a = states((0..8).map(|i| state(0, [i, 0, 0, 0])).collect());
        let mut b = a.clone();
        b.states.remove(4);
        let r = diff(&a, &b, None, 10).unwrap();
        assert_eq!(r.divergences.len(), 1, "{:?}", r.divergences);
        let d = &r.divergences[0];
        assert_eq!((d.frame, d.a_frame), (4, 4));
        assert_eq!(d.classes, vec![FRAME_SHIFT]);
    }
}

/// The mutation tests of ROADMAP Milestone 1, step 5: a divergence put in
/// on purpose must be found and classified. They run our own test ROM in
/// `gp-nes`, so they need neither Mesen nor a ROM file.
#[cfg(test)]
mod mutation {
    use super::*;
    use gp_evidence::baseline::Baseline;
    use gp_nes::testrom;

    const FRAMES: usize = 400;

    fn input() -> InputLog {
        let b = Baseline::parse(
            include_str!("../../../baselines/gp-testrom/buttons.toml"),
            std::path::Path::new("buttons.toml"),
        )
        .unwrap();
        let mut input = b.input().unwrap();
        input.frames.truncate(FRAMES);
        input
    }

    /// Runs the test ROM. `flip` is `(frame, address, mask)`: that RAM
    /// byte is XORed with the mask before the frame's first instruction.
    fn run(input: &InputLog, flip: Option<(u64, usize, u8)>) -> States {
        run_flips(input, &flip.into_iter().collect::<Vec<_>>())
    }

    /// Runs the test ROM with each `(frame, address, mask)` flip applied.
    fn run_flips(input: &InputLog, flips: &[(u64, usize, u8)]) -> States {
        let rom = testrom::build();
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        let mut states = vec![nes.canonical_state().unwrap()];
        for f in &input.frames {
            let frame = nes.frame() + 1;
            let mut first = true;
            nes.step_with(*f, |bus| {
                if first {
                    for &(at, addr, mask) in flips {
                        if at == frame {
                            bus.wram[addr] ^= mask;
                        }
                    }
                }
                first = false;
            })
            .unwrap();
            states.push(nes.canonical_state().unwrap());
        }
        States {
            table: canonical::field_table(),
            states,
        }
    }

    fn live<'a>(rom: &'a [u8], input: &'a InputLog) -> LiveA<'a> {
        LiveA {
            rom,
            power_on: PowerOn::default(),
            input,
        }
    }

    #[test]
    fn a_flipped_ram_byte_is_found_and_classified() {
        let input = input();
        let a = run(&input, None);
        // $0700, which the program never reads: only RAM differs, from the
        // flip to the end.
        let b = run(&input, Some((150, 0x700, 0x5a)));
        for r in [
            diff(&a, &b, None, 10).unwrap(),
            diff(
                &a,
                &b,
                Some(LiveA {
                    rom: &testrom::build(),
                    power_on: PowerOn::default(),
                    input: &input,
                }),
                10,
            )
            .unwrap(),
        ] {
            assert_eq!(r.divergences.len(), 1, "{} {:#?}", r.mode, r.divergences);
            let d = &r.divergences[0];
            assert_eq!(d.frame, 150);
            assert_eq!(d.classes, vec!["ram"]);
            assert_eq!(d.fields[0].a, "1 of 2048 bytes differ, at $0700: 00");
            assert_eq!(d.fields[0].b, "1 of 2048 bytes differ, at $0700: 5a");
        }
    }

    #[test]
    fn a_flipped_byte_the_game_reads_is_resynced_by_copying() {
        let input = input();
        let a = run(&input, None);
        // FRAMES ($00), the NMI counter the program reads: flipping its
        // low bit moves which frames it polls the controller on, so B runs
        // other code from frame 150 on. Copying B's whole state at the
        // divergence must leave nothing else to report.
        let b = run(&input, Some((150, 0, 0x01)));
        let rom = testrom::build();
        let r = diff(&a, &b, Some(live(&rom, &input)), 50).unwrap();
        assert_eq!(r.divergences.len(), 1, "{:#?}", r.divergences);
        let d = &r.divergences[0];
        assert_eq!(d.frame, 150);
        assert!(d.classes.contains(&"ram".to_owned()), "{:?}", d.classes);
        assert!(d.resync.contains("copied"), "{}", d.resync);

        // From the traces alone the run keeps diverging; the first is still
        // the flip.
        let t = diff(&a, &b, None, 1000).unwrap();
        assert_eq!(t.divergences[0].frame, 150);
        assert!(t.divergences.len() > 1);
    }

    #[test]
    fn copies_are_counted_in_a_row() {
        let input = input();
        let a = run(&input, None);
        // Four separate flips of the counter, each copied once, then a new
        // difference in a byte the program never reads: it must still be
        // found, since the copy limit counts frames in a row.
        let b = run_flips(
            &input,
            &[
                (100, 0, 1),
                (150, 0, 1),
                (200, 0, 1),
                (250, 0, 1),
                (300, 0x700, 0x5a),
            ],
        );
        let rom = testrom::build();
        let r = diff(&a, &b, Some(live(&rom, &input)), 100).unwrap();
        let frames: Vec<u64> = r.divergences.iter().map(|d| d.frame).collect();
        assert_eq!(
            frames,
            vec![100, 150, 200, 250, 300],
            "{:#?}",
            r.divergences
        );
    }

    #[test]
    fn a_skipped_frame_is_found_and_classified() {
        let input = input();
        let a = run(&input, None);
        let mut b = a.clone();
        b.states.remove(200);
        let rom = testrom::build();
        let live = LiveA {
            rom: &rom,
            power_on: PowerOn::default(),
            input: &input,
        };
        for r in [
            diff(&a, &b, Some(live), 10).unwrap(),
            diff(&a, &b, None, 10).unwrap(),
        ] {
            assert_eq!(r.divergences.len(), 1, "{} {:#?}", r.mode, r.divergences);
            let d = &r.divergences[0];
            assert_eq!((d.frame, d.a_frame), (200, 200));
            assert_eq!(d.classes, vec![FRAME_SHIFT]);
            assert_eq!(r.frames, FRAMES as u64 - 1);
        }
    }
}

/// The classifier and the set-aside rules, on small hand-made states.
#[cfg(test)]
mod classify {
    use super::*;
    use gp_evidence::trace::FieldDef;

    #[derive(Clone)]
    struct S {
        a: u8,
        s: u8,
        p: u8,
        pc: u16,
        cycles: u64,
        line: u16,
        dot: u16,
        step: u8,
        ram: Vec<u8>,
    }

    impl Default for S {
        fn default() -> Self {
            Self {
                a: 0,
                s: 0xff,
                p: 0x20,
                pc: 0x8000,
                cycles: 1000,
                line: 241,
                dot: 10,
                step: 2,
                ram: vec![0; 0x200],
            }
        }
    }

    fn table() -> FieldTable {
        let f = |n: &str, w| FieldDef {
            name: n.into(),
            width: Width::Fixed(w),
        };
        FieldTable {
            state_magic: *b"TEST",
            state_version: 1,
            fields: vec![
                f("cpu.a", 1),
                f("cpu.s", 1),
                f("cpu.p", 1),
                f("cpu.pc", 2),
                f("cpu.cycles", 8),
                f("ppu.scanline", 2),
                f("ppu.dot", 2),
                f("apu.frame_counter.step", 1),
                f("ram", 0x200),
            ],
        }
    }

    impl S {
        fn bytes(&self) -> Vec<u8> {
            let mut v = b"TEST".to_vec();
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&[self.a, self.s, self.p]);
            v.extend_from_slice(&self.pc.to_le_bytes());
            v.extend_from_slice(&self.cycles.to_le_bytes());
            v.extend_from_slice(&self.line.to_le_bytes());
            v.extend_from_slice(&self.dot.to_le_bytes());
            v.push(self.step);
            v.extend_from_slice(&self.ram);
            v
        }

        /// This state after an NMI entry: three bytes pushed, I set, PC at
        /// the handler, 7 cycles and 21 dots on.
        fn after_nmi(&self) -> Self {
            let mut n = self.clone();
            let s = usize::from(self.s);
            n.ram[0x100 + s] = (self.pc >> 8) as u8;
            n.ram[0x100 + ((s + 0xFF) & 0xFF)] = self.pc as u8;
            n.ram[0x100 + ((s + 0xFE) & 0xFF)] = self.p | 0x20;
            n.s = self.s.wrapping_sub(3);
            n.p |= 0x04;
            n.pc = 0xc100;
            n.cycles += 7;
            n.dot += 21;
            n
        }
    }

    fn classify(a: &S, b: &S) -> Option<(&'static str, Vec<String>)> {
        let t = table();
        let (sa, sb) = (a.bytes(), b.bytes());
        let fa: Fields = t.split(&sa).unwrap().into_iter().collect();
        let fb: Fields = t.split(&sb).unwrap().into_iter().collect();
        let cmp = Compare {
            a: &t,
            b: &t,
            fields: t.fields.iter().map(|f| f.name.clone()).collect(),
        };
        let all = cmp.diffs(&sa, &sb).unwrap();
        let (class, accept) = boundary_class(&fa, &fb, &all, 0)?;
        let new: Vec<(String, Signature)> = all
            .iter()
            .map(|(n, va, vb)| (n.clone(), signature(n, va, vb)))
            .collect();
        Some((class, explained(&new, &accept).into_iter().collect()))
    }

    fn states(list: &[S]) -> States {
        States {
            table: table(),
            states: list.iter().map(S::bytes).collect(),
        }
    }

    #[test]
    fn an_nmi_entry_on_one_side_is_nmi_boundary() {
        let pre = S::default();
        let post = pre.after_nmi();
        for (a, b) in [(&pre, &post), (&post, &pre)] {
            let (class, explained) = classify(a, b).unwrap();
            assert_eq!(class, NMI_BOUNDARY);
            assert_eq!(
                explained,
                ["cpu.cycles", "cpu.p", "cpu.pc", "cpu.s", "ppu.dot", "ram"]
            );
        }
    }

    #[test]
    fn an_nmi_lookalike_is_not_one() {
        let pre = S::default();
        // Pushed PC isn't the other side's.
        let mut post = pre.after_nmi();
        post.ram[0x1ff] = 0x91;
        assert_eq!(classify(&pre, &post), None);
        // I flag clear on the side that took it.
        let mut post = pre.after_nmi();
        post.p &= !0x04;
        assert_eq!(classify(&pre, &post), None);
        // Too few cycles apart, and too many.
        let mut post = pre.after_nmi();
        post.cycles = pre.cycles + 4;
        assert_eq!(classify(&pre, &post), None);
        post.cycles = pre.cycles + 15;
        assert_eq!(classify(&pre, &post), None);
        // The same cycle count: not a boundary at all.
        post.cycles = pre.cycles;
        assert_eq!(classify(&pre, &post), None);
        // Pushed P isn't the other side's P.
        let mut post = pre.after_nmi();
        post.ram[0x1fd] = 0xe7;
        assert_eq!(classify(&pre, &post), None);
    }

    #[test]
    fn an_nmi_boundary_explains_only_the_pushed_bytes() {
        let pre = S::default();
        let mut post = pre.after_nmi();
        post.ram[0x10] = 7;
        let (class, explained) = classify(&pre, &post).unwrap();
        assert_eq!(class, NMI_BOUNDARY);
        assert!(!explained.contains(&"ram".to_owned()));
    }

    #[test]
    fn frames_ending_an_instruction_apart_are_boundary_instruction() {
        let a = S::default();
        let mut b = a.clone();
        b.pc = 0x8003;
        b.a = 9;
        b.cycles += 3;
        b.dot += 9;
        b.step = 3;
        let (class, explained) = classify(&a, &b).unwrap();
        assert_eq!(class, BOUNDARY_INSTRUCTION);
        assert_eq!(
            explained,
            [
                "apu.frame_counter.step",
                "cpu.a",
                "cpu.cycles",
                "cpu.pc",
                "ppu.dot"
            ]
        );
        // Memory is never explained.
        let mut c = b.clone();
        c.ram[0x20] = 1;
        let (_, explained) = classify(&a, &c).unwrap();
        assert!(!explained.contains(&"ram".to_owned()));
    }

    #[test]
    fn a_boundary_instruction_has_bounded_differences() {
        let a = S::default();
        let mut b = a.clone();
        b.pc = 0x9234;
        // Same cycle count, other PC: a split in control flow.
        assert_eq!(classify(&a, &b), None);
        // More than one instruction apart.
        b.cycles = a.cycles + 8;
        assert_eq!(classify(&a, &b), None);
        // S differs by other than 3.
        b.cycles = a.cycles + 3;
        b.s = 0xfe;
        assert_eq!(classify(&a, &b), None);
        // A position that didn't move with the cycles, and a frame
        // counter 2 steps apart, aren't explained.
        b.s = a.s;
        b.dot = a.dot + 40;
        b.step = a.step + 2;
        let (class, explained) = classify(&a, &b).unwrap();
        assert_eq!(class, BOUNDARY_INSTRUCTION);
        assert!(!explained.contains(&"ppu.dot".to_owned()));
        assert!(!explained.contains(&"apu.frame_counter.step".to_owned()));
    }

    #[test]
    fn the_frame_counter_may_wrap_between_two_ends() {
        let a = S::default();
        let mut b = a.clone();
        b.cycles += 2;
        b.dot += 6;
        b.step = 0;
        let mut a3 = a.clone();
        a3.step = 3;
        let (_, explained) = classify(&a3, &b).unwrap();
        assert!(explained.contains(&"apu.frame_counter.step".to_owned()));
        let mut a2 = a.clone();
        a2.step = 2;
        let (_, explained) = classify(&a2, &b).unwrap();
        assert!(!explained.contains(&"apu.frame_counter.step".to_owned()));
    }

    #[test]
    fn a_ppu_alignment_is_measured_at_frame_1_and_accepted() {
        // B's PPU runs 4 dots ahead at the same CPU cycle, from frame 1 on.
        let a: Vec<S> = (0..6)
            .map(|i| S {
                cycles: 1000 + 29781 * i,
                ..S::default()
            })
            .collect();
        let b: Vec<S> = a
            .iter()
            .enumerate()
            .map(|(i, s)| S {
                dot: if i == 0 { s.dot } else { s.dot + 4 },
                ..s.clone()
            })
            .collect();
        let r = diff(&states(&a), &states(&b), None, 10).unwrap();
        assert_eq!(r.class_names(), vec![PPU_ALIGNMENT.to_owned()]);
        // Any other offset at the same cycle is a position difference.
        let mut c = b.clone();
        c[4].dot = a[4].dot + 9;
        let r = diff(&states(&a), &states(&c), None, 10).unwrap();
        assert!(
            r.class_names().contains(&"ppu.dot".to_owned()),
            "{:?}",
            r.class_names()
        );
    }

    #[test]
    fn a_drifting_difference_is_reported_again() {
        let a: Vec<S> = (0..20)
            .map(|i| S {
                cycles: 1000 + i * 29781,
                ..S::default()
            })
            .collect();
        let b: Vec<S> = a
            .iter()
            .enumerate()
            .map(|(i, s)| S {
                cycles: s.cycles + i as u64,
                dot: s.dot + 3 * i as u16,
                ..s.clone()
            })
            .collect();
        let r = diff(&states(&a), &states(&b), None, 100).unwrap();
        assert_eq!(r.divergences.len(), 19, "one per frame the drift grows");
        assert_eq!(r.divergences[0].classes, vec![BOUNDARY_INSTRUCTION]);
        // Past one instruction's worth, it's no longer a boundary.
        assert!(r.divergences[10].classes.contains(&"cpu.cycles".to_owned()));
    }

    #[test]
    fn cycle_counts_compare_past_32_bits() {
        assert_eq!(cycles_apart(0xFFFF_FFFE, 0x1_0000_0001), 3);
        assert_eq!(cycles_apart(0x1_0000_0001, 0xFFFF_FFFE), -3);
        let a = S {
            cycles: 0xFFFF_FFFE,
            ..S::default()
        };
        let b = S {
            cycles: 0x1_0000_0001,
            pc: 0x8003,
            dot: a.dot + 9,
            ..S::default()
        };
        assert_eq!(classify(&a, &b).unwrap().0, BOUNDARY_INSTRUCTION);
    }

    #[test]
    fn a_lasting_difference_is_reported_once() {
        let a: Vec<S> = (0..10).map(|_| S::default()).collect();
        let b: Vec<S> = (0..10)
            .map(|i| S {
                a: if i >= 3 { 5 } else { 0 },
                ..S::default()
            })
            .collect();
        let r = diff(&states(&a), &states(&b), None, 100).unwrap();
        assert_eq!(r.divergences.len(), 1);
        assert_eq!(r.divergences[0].frame, 3);
        assert_eq!(r.classes[0].frames, vec![3]);
    }

    #[test]
    fn pushed_stack_bytes_dont_hide_later_ram() {
        let pre = S::default();
        let post = pre.after_nmi();
        let mut later = post.clone();
        later.ram[0x10] = 0x77;
        let a = states(&[S::default(), pre.clone(), pre.clone(), pre.clone()]);
        let b = states(&[S::default(), post.clone(), post, later]);
        let r = diff(&a, &b, None, 100).unwrap();
        let frames: Vec<u64> = r.divergences.iter().map(|d| d.frame).collect();
        assert_eq!(frames, vec![1, 3], "{:#?}", r.divergences);
        assert_eq!(r.divergences[0].classes, vec![NMI_BOUNDARY]);
        assert_eq!(r.divergences[1].classes, vec!["ram"]);
    }

    #[test]
    fn a_repeated_frame_is_a_frame_shift() {
        let a: Vec<S> = (0..8)
            .map(|i| S {
                cycles: 1000 + 29781 * i,
                ..S::default()
            })
            .collect();
        let mut b = a.clone();
        b.insert(4, a[3].clone());
        b.pop();
        let r = diff(&states(&a), &states(&b), None, 10).unwrap();
        assert_eq!(r.divergences.len(), 1, "{:#?}", r.divergences);
        assert_eq!(r.divergences[0].classes, vec![FRAME_SHIFT]);
        assert!(r.divergences[0].resync.contains("earlier"));
    }
}
