//! The runtime protocol (`gp_runtime::Runtime`) for an arcade original:
//! the time, state and observe operations over a [`Tapper`].
//!
//! An `InputFrame`'s `buttons` low byte holds the controls, bits 0 to 6
//! coin, start, right, left, down, up and tap ([`Controls::from_bits`]);
//! the higher bytes, axes and the reset button are refused, since the
//! board has none of them.
//!
//! Selectors (docs/runtime-protocol.md has the grammar):
//!
//! | Selector | What it reads |
//! |---|---|
//! | `frame` | Frames run since power-on |
//! | `ram[A]` | The main CPU's address space at `A` (0 to 0xFFFF) as the CPU reads it, without side effects: work RAM at 0xE000, sprite RAM at 0xE800, video RAM at 0xF000, the program below |
//! | `sound_ram[A]` | The SSIO board's 1 KB of RAM, by offset (its Z80 sees it at 0x8000) |
//! | `palette[A]` | The 128 bytes of palette RAM, which the main CPU only writes (at 0xF800) |
//! | `ports[N]` | The five input ports as the game read them in the last frame |
//! | `cpu.pc`, `sound_cpu.pc` | The two Z80s' program counters |
//!
//! The canonical state is the machine's snapshot ([`Tapper::snapshot`]):
//! both CPUs, the timers, every RAM and the last frame's picture. Its hash
//! is [`Tapper::state_hash`]. Each frame stepped gives one `frame` event
//! with a `polled` flag, false on a frame where the game didn't read its
//! inputs. With `set("ay_events", true)` it also gives an `ay-write` event
//! for each AY register the sound CPU wrote in the frame, in order, with
//! `chip`, `reg` and `value`: the original's side of a sound measure, as
//! gp-nes's `apu-write` events are for the NES. A model has no AY hook
//! yet, so nothing compares them.

use gp_runtime::select::{Condition, Index, Selector, hex};
use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, LoadNote, PatchResult, Rect, Runtime, Until, Value,
    input_at,
};

use crate::{Controls, Romset, Tapper};

/// The kind of the events that carry the AY writes.
pub const AY_KIND: &str = "ay-write";

/// Tapper behind the runtime protocol.
#[derive(Clone)]
pub struct ArcadeRuntime {
    tapper: Tapper,
    /// Whether a step gives `ay-write` events.
    ay_events: bool,
}

impl ArcadeRuntime {
    /// Powers the set on.
    pub fn load(rom: &Romset) -> Self {
        Self::new(Tapper::power_on(rom))
    }

    /// Powers on the set in a MAME zip's bytes.
    pub fn load_zip(zip: &[u8]) -> Result<Self, String> {
        Ok(Self::load(
            &Romset::from_zip(zip).map_err(|e| e.to_string())?,
        ))
    }

    /// Wraps a machine already running, such as one restored from a start.
    pub fn new(tapper: Tapper) -> Self {
        Self {
            tapper,
            ay_events: false,
        }
    }

    pub fn tapper(&self) -> &Tapper {
        &self.tapper
    }

    pub fn into_tapper(self) -> Tapper {
        self.tapper
    }

    /// Runs one frame and returns whether the game read its inputs.
    pub fn advance(&mut self, input: &InputFrame) -> Result<bool, String> {
        if !input.axes.is_empty() {
            return Err("the arcade board has no analog input; axes must be empty".into());
        }
        if input.reset {
            return Err("the arcade board's reset isn't modelled".into());
        }
        if input.buttons > 0x7f {
            return Err(format!(
                "buttons {:#x}: the arcade board reads bits 0 to 6 only (coin, start, right, left, down, up, tap)",
                input.buttons
            ));
        }
        self.tapper.step(Controls::from_bits(input.buttons));
        Ok(self.tapper.polled())
    }

    fn step_frame(&mut self, input: &InputFrame) -> Result<Vec<Event>, String> {
        let polled = self.advance(input)?;
        let frame = self.tapper.frame();
        let mut events = vec![Event {
            frame,
            kind: "frame".into(),
            entity: None,
            data: vec![Entry {
                path: "polled".into(),
                value: Value::Flag(polled),
            }],
        }];
        if self.ay_events {
            let int = |path: &str, v: u8| Entry {
                path: path.into(),
                value: Value::Int(i64::from(v)),
            };
            events.extend(
                self.tapper
                    .ay_registers_written()
                    .iter()
                    .map(|&(chip, reg, value)| Event {
                        frame,
                        kind: AY_KIND.into(),
                        entity: None,
                        data: vec![int("chip", chip), int("reg", reg), int("value", value)],
                    }),
            );
        }
        Ok(events)
    }

    /// Reads a parsed selector.
    pub fn read(&self, sel: &Selector) -> Result<Vec<Entry>, String> {
        let name = sel.name();
        let index = sel.last_index()?;
        let int = |path: String, v: i64| Entry {
            path,
            value: Value::Int(v),
        };
        let t = &self.tapper;
        match (name.as_str(), index) {
            ("frame", None) => Ok(vec![int(name, t.frame() as i64)]),
            ("ram", Some(index)) => Ok(range(&name, index, 0x1_0000)?
                .map(|a| int(format!("ram[{}]", hex(a as i64)), t.peek(a as u16).into()))
                .collect()),
            ("sound_ram", Some(index)) => indexed(&name, index, t.sound_ram()),
            ("palette", Some(index)) => indexed(&name, index, t.palette_ram()),
            ("ports", Some(index)) => indexed(&name, index, &t.ports()),
            ("cpu.pc", None) => Ok(vec![int(name, t.main.pc.into())]),
            ("sound_cpu.pc", None) => Ok(vec![int(name, t.ssio.pc.into())]),
            ("ram" | "sound_ram" | "palette" | "ports", None) => Err(format!(
                "{name} takes an index: one ({name}[0x10]), a range ({name}[0..16]) or all ({name}[*])"
            )),
            _ => Err(format!(
                "no selector {sel} on the arcade board: frame, ram[...], sound_ram[...], palette[...], ports[...], cpu.pc and sound_cpu.pc"
            )),
        }
    }

    fn read_one(&self, sel: &Selector) -> Result<i64, String> {
        match self.read(sel)?.as_slice() {
            [Entry { value, .. }] => value
                .as_int()
                .ok_or_else(|| format!("{sel} isn't a number")),
            many => Err(format!(
                "{sel} names {} values; a condition needs exactly one",
                many.len()
            )),
        }
    }

    fn holds(&self, cond: &Condition) -> Result<bool, String> {
        cond.eval(&mut |sel| self.read_one(sel))
    }
}

/// Answers `sel` from a RAM record (the oracle's layout, [`Tapper::ram_record`])
/// alone, as [`ArcadeRuntime::read`] would on the machine that made it:
/// `ram[...]` from 0xE000 up, `sound_ram[...]` and `palette[...]`. `None`
/// for a selector the record can't answer (the program below 0xE000, the
/// ports, the CPUs), which a score tape keeps by selector instead.
pub fn read_record(record: &[u8], sel: &Selector) -> Option<Result<Vec<Entry>, String>> {
    if record.len() != crate::mcr::RAM_RECORD {
        return None;
    }
    let index = match sel.last_index() {
        Err(e) => return Some(Err(e)),
        Ok(None) => return None,
        Ok(Some(i)) => i,
    };
    match sel.name().as_str() {
        "sound_ram" => Some(indexed("sound_ram", index, &record[4736..])),
        "palette" => Some(indexed("palette", index, &record[4608..4736])),
        "ram" => {
            let r = match range("ram", index, 0x1_0000) {
                Ok(r) => r,
                Err(e) => return Some(Err(e)),
            };
            if r.start < 0xe000 {
                return None;
            }
            // Board::main_read for 0xE000 up.
            let byte = |a: usize| match a {
                0xe000..=0xe7ff => record[a & 0x7ff],
                0xe800..=0xebff => record[2048 + (a & 0x1ff)],
                0xf000..=0xf7ff => record[2560 + (a & 0x7ff)],
                _ => 0xff,
            };
            Some(Ok(r
                .map(|a| Entry {
                    path: format!("ram[{}]", hex(a as i64)),
                    value: Value::Int(byte(a).into()),
                })
                .collect()))
        }
        _ => None,
    }
}

/// The positions an index selects in a memory of `len` bytes.
fn range(name: &str, index: Index, len: usize) -> Result<std::ops::Range<usize>, String> {
    let outside = || {
        format!(
            "{name} has {len} bytes, indexed 0 to {}",
            hex(len as i64 - 1)
        )
    };
    let (start, end) = match index {
        Index::At(i) => (i, i.checked_add(1).ok_or_else(outside)?),
        Index::Range(a, b) => (a, b),
        Index::All => (0, len as i64),
    };
    if start >= end {
        return Err(format!(
            "{name}[{}..{}] selects nothing",
            hex(start),
            hex(end)
        ));
    }
    match (usize::try_from(start), usize::try_from(end)) {
        (Ok(start), Ok(end)) if end <= len => Ok(start..end),
        _ => Err(outside()),
    }
}

fn indexed(name: &str, index: Index, bytes: &[u8]) -> Result<Vec<Entry>, String> {
    let r = range(name, index, bytes.len())?;
    Ok(bytes[r.clone()]
        .iter()
        .zip(r)
        .map(|(b, i)| Entry {
            path: format!("{name}[{}]", hex(i as i64)),
            value: Value::Int((*b).into()),
        })
        .collect())
}

impl Runtime for ArcadeRuntime {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SNAPSHOT.union(Capabilities::FORK)
    }

    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
        let mut events = Vec::with_capacity(inputs.len());
        for input in inputs {
            events.extend(self.step_frame(input)?);
        }
        Ok(events)
    }

    fn run_until(
        &mut self,
        condition: &str,
        inputs: &[InputFrame],
        max_frames: u32,
    ) -> Result<Until, String> {
        let cond = Condition::parse(condition)?;
        let mut events = Vec::new();
        if self.holds(&cond)? {
            return Ok(Until { met: true, events });
        }
        for i in 0..max_frames as usize {
            events.extend(self.step_frame(&input_at(inputs, i))?);
            if self.holds(&cond)? {
                return Ok(Until { met: true, events });
            }
        }
        Ok(Until { met: false, events })
    }

    fn frame(&self) -> u64 {
        self.tapper.frame()
    }

    fn snapshot(&self) -> Vec<u8> {
        self.tapper.snapshot()
    }

    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        self.tapper.restore(snap)
    }

    fn fork(&self) -> Self {
        self.clone()
    }

    fn canonical_state(&self) -> Vec<u8> {
        self.tapper.snapshot()
    }

    fn state_hash(&self) -> u64 {
        self.tapper.state_hash()
    }

    /// Refused: the board has no model state to load.
    fn load_state(&mut self, _state: &[u8]) -> Result<Vec<LoadNote>, String> {
        Err("gp-arcade runs the original, which has no model state to load; restore takes its snapshots".into())
    }

    fn query(&self, selector: &str) -> Result<Vec<Entry>, String> {
        self.read(&Selector::parse(selector)?)
    }

    fn sample(
        &mut self,
        selectors: &[String],
        inputs: &[InputFrame],
        frames: u32,
    ) -> Result<Vec<Vec<Entry>>, String> {
        let parsed = selectors
            .iter()
            .map(|s| Selector::parse(s))
            .collect::<Result<Vec<_>, _>>()?;
        let mut rows = Vec::with_capacity(frames as usize);
        for i in 0..frames as usize {
            self.step_frame(&input_at(inputs, i))?;
            let mut row = Vec::new();
            for sel in &parsed {
                row.extend(self.read(sel)?);
            }
            rows.push(row);
        }
        Ok(rows)
    }

    /// The original has no entities: a state mapping names them.
    fn entities_at(&self, _area: Rect) -> Vec<u64> {
        Vec::new()
    }

    fn patch(&mut self, _model_diff: &str) -> PatchResult {
        PatchResult::Rejected("the original game can't be patched".into())
    }

    fn set(&mut self, p: &str, v: Value) -> Result<(), String> {
        match (p, v) {
            ("ay_events", Value::Flag(on)) => {
                self.ay_events = on;
                Ok(())
            }
            _ => Err(format!(
                "{p}: gp-arcade sets only ay_events (on or off); the original runs as it is"
            )),
        }
    }

    fn explain(&self, _p: &str) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up set: a main program that loops at 0 (`JR $`), nothing
    /// else. No game's bytes.
    fn looping() -> Romset {
        let mut maincpu = vec![0u8; 0x10000];
        maincpu[..2].copy_from_slice(&[0x18, 0xfe]);
        let mut ssiocpu = vec![0u8; 0x10000];
        ssiocpu[..2].copy_from_slice(&[0x18, 0xfe]);
        Romset {
            name: "test",
            identity: String::new(),
            maincpu,
            ssiocpu,
            gfx1: vec![0; 0x8000],
            gfx2: vec![0; 0x20000],
            extras: Vec::new(),
            ssio_prom: None,
        }
    }

    #[test]
    fn steps_reads_and_snapshots() {
        let mut rt = ArcadeRuntime::load(&looping());
        let ev = rt
            .step(&[InputFrame::default(), InputFrame::default()])
            .unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[1].frame, 2);
        assert_eq!(ev[1].data[0].value, Value::Flag(false));
        assert_eq!(rt.frame(), 2);
        assert_eq!(rt.query("frame").unwrap()[0].value, Value::Int(2));
        // The program's first byte, through the main CPU's address space.
        assert_eq!(rt.query("ram[0]").unwrap()[0].value, Value::Int(0x18));
        assert_eq!(rt.query("ram[0xe000..0xe004]").unwrap().len(), 4);
        assert_eq!(rt.query("sound_ram[*]").unwrap().len(), 0x400);
        assert!(rt.query("ram").is_err());
        assert!(rt.query("oam[0]").is_err());
        let snap = rt.snapshot();
        let h = rt.state_hash();
        let mut other = rt.fork();
        other.step(&[InputFrame::default()]).unwrap();
        assert_ne!(other.state_hash(), h);
        other.restore(&snap).unwrap();
        assert_eq!(other.state_hash(), h);
        assert_eq!(other.canonical_state(), rt.canonical_state());
        let until = rt.run_until("frame >= 5", &[], 10).unwrap();
        assert!(until.met);
        assert_eq!(rt.frame(), 5);
    }

    #[test]
    fn refuses_what_the_board_has_not() {
        let mut rt = ArcadeRuntime::load(&looping());
        let pad2 = InputFrame::pads([0, 1, 0, 0]);
        assert!(rt.step(&[pad2]).is_err());
        let reset = InputFrame {
            reset: true,
            ..InputFrame::default()
        };
        assert!(rt.step(&[reset]).is_err());
        assert!(rt.load_state(&[]).is_err());
    }

    /// `ay_events` turns the AY write events on; with the owner's romset,
    /// the sound board's start-up writes come out as events. Skipped
    /// without it.
    #[test]
    fn ay_events_carry_the_sound_cpus_writes() {
        let mut rt = ArcadeRuntime::load(&looping());
        assert!(rt.set("ay_events", Value::Flag(true)).is_ok());
        assert!(rt.set("volume", Value::Int(3)).is_err());
        let Some(zip) = crate::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let mut rt = ArcadeRuntime::load_zip(&std::fs::read(zip).unwrap()).unwrap();
        rt.set("ay_events", Value::Flag(true)).unwrap();
        let ev = rt.step(&vec![InputFrame::default(); 30]).unwrap();
        let ay: Vec<&Event> = ev.iter().filter(|e| e.kind == AY_KIND).collect();
        assert!(!ay.is_empty());
        assert_eq!(ay[0].data[0].path, "chip");
    }

    /// A RAM record answers what the machine answers, for the parts it
    /// holds, and nothing else.
    #[test]
    fn a_ram_record_answers_as_the_machine() {
        let mut rt = ArcadeRuntime::load(&looping());
        rt.step(&vec![InputFrame::default(); 3]).unwrap();
        let rec = rt.tapper().ram_record();
        for sel in [
            "ram[0xe000..0xe010]",
            "ram[0xe9f0..0xec10]",
            "ram[0xf7f0..0xf810]",
            "sound_ram[0..8]",
            "palette[*]",
        ] {
            let s = Selector::parse(sel).unwrap();
            assert_eq!(read_record(&rec, &s).unwrap(), rt.read(&s), "{sel}");
        }
        for sel in ["ram[0x100]", "ports[0]", "cpu.pc", "frame"] {
            assert!(
                read_record(&rec, &Selector::parse(sel).unwrap()).is_none(),
                "{sel}"
            );
        }
    }
}
