//! The runtime protocol (`gp_runtime::Runtime`) for the original game: the
//! time, state and observe operations over a [`Nes`].
//!
//! There is no game model and no state mapping yet, so selectors name raw
//! console state: the canonical state's fields (docs/formats/recording.md),
//! such as `cpu.pc` or `ppu.scanline`, and its memories by index, such as
//! `ram[0x0770]`, `oam[0..4]` or `palette[*]`. A name with no field of its
//! own selects every integer field under it, so `cpu` reads all the CPU's
//! registers. `bus[addr]` reads the CPU's address space as the CPU would see
//! it, without a read's side effects, and `frame` is the frame number.
//! docs/runtime-protocol.md has the full list.
//!
//! Each frame stepped gives the event stream's events (writes to PPU and
//! APU registers, interrupts, sprite 0 hits, controller reads and writes,
//! bank switches) in the order they happened, then one `frame` event with a
//! `polled` flag that is false on lag frames. Kinds are named as the stream
//! names them ([`EventKind::name`], such as `ppu-write`), so `gp events`
//! and the protocol share one naming; docs/runtime-protocol.md has the
//! mapping.

use gp_evidence::Hash;
use gp_evidence::input_log::InputFrame as LogFrame;
use gp_evidence::manifest::PowerOn;
use gp_runtime::select::{Condition, Index, Selector, hex};
use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, PatchResult, Rect, Runtime, Until, Value, input_at,
};

use gp_evidence::events::EventKind;

use crate::canonical::{self, Width};
use crate::{Nes, NesError, runtime_ref};

/// Leads every protocol snapshot. Version 1: then the runtime version
/// string (u32 length, bytes), the frame (u64), the four controller bytes,
/// and `tetanes-core`'s save state. All integers little-endian.
const SNAPSHOT_MAGIC: &[u8; 4] = b"GPRS";
const SNAPSHOT_VERSION: u32 = 1;

/// A [`Nes`] behind the runtime protocol.
#[derive(Clone)]
pub struct NesRuntime {
    nes: Nes,
    /// Whether `ram[...]` reads the RAM as the CPU takes the next NMI
    /// ([`Nes::wram_at_next_nmi`]) instead of at the frame's end: set by
    /// `set("read_at_nmi", true)`, for a state mapping's `read_at = "nmi"`.
    at_nmi: bool,
    /// The console as the CPU takes the next NMI after the frame stepped
    /// last, found on the first read that needs it (`read_at_nmi`'s RAM
    /// and the `nmi.` selectors).
    settled: std::sync::OnceLock<Option<Box<Nes>>>,
}

/// A data access an instruction makes: the address, how
/// ([`crate::coverage::READ`], [`crate::coverage::WRITE`]) and the byte
/// there before it runs.
pub type Access = (u16, u8, u8);

impl NesRuntime {
    /// Powers on with the ROM loaded. Refuses a cartridge whose canonical
    /// state isn't defined yet (any mapper but those in
    /// [`canonical::MAPPERS`]), since every state operation needs it.
    pub fn load(rom: &[u8], power_on: PowerOn) -> Result<Self, NesError> {
        Self::new(Nes::power_on(rom, power_on)?)
    }

    /// Wraps a console that's already running, such as one restored from a
    /// recording's checkpoint. Turns its event log on, since `step` returns
    /// the events; the log changes nothing the console does.
    pub fn new(mut nes: Nes) -> Result<Self, NesError> {
        nes.canonical_state()?;
        nes.set_events(true);
        nes.take_events();
        Ok(Self {
            nes,
            at_nmi: false,
            settled: std::sync::OnceLock::new(),
        })
    }

    /// Turns video rendering on or off ([`Nes::set_video`]), for a caller
    /// that reads the frames' pictures, such as the screen measure.
    pub fn set_video(&mut self, on: bool) {
        self.nes.set_video(on);
    }

    pub fn nes(&self) -> &Nes {
        &self.nes
    }

    pub fn into_nes(self) -> Nes {
        self.nes
    }

    /// Runs one frame as [`Runtime::step`] does, adding the code it runs
    /// to `map` ([`crate::coverage`]), and returns whether the game polled
    /// the controllers. Its events are dropped. The frame ends in the same
    /// state as an uncovered one.
    pub fn step_covered(
        &mut self,
        input: &InputFrame,
        map: &mut crate::coverage::CodeMap,
    ) -> Result<bool, String> {
        self.settled = std::sync::OnceLock::new();
        let pads = input.buttons.to_le_bytes();
        if !input.axes.is_empty() || pads[2] != 0 || pads[3] != 0 {
            return Err("gp-nes applies controllers 1 and 2 only, with no axes".into());
        }
        let mut watch = crate::coverage::Watch::default();
        let out = self
            .nes
            .step_with(
                LogFrame {
                    polled: false,
                    pads,
                    reset: input.reset,
                },
                |bus| watch.before(bus, map),
            )
            .map_err(|e| format!("frame {}: {e}", self.nes.frame() + 1))?;
        self.nes.take_events();
        Ok(out.polled)
    }

    /// One frame as [`Self::step_covered`], and before each instruction
    /// `seen` hears its address, the X register, the data address it reads
    /// or writes ([`crate::coverage::access`]), how, and the byte there
    /// before it runs (RAM and ROM; 0 for a register).
    /// One frame as [`Self::step_accesses`], and `seen` hears every
    /// instruction before it runs: its address, its opcode, and the data
    /// access it makes, if any, with the byte there (RAM and ROM; 0 for a
    /// register). A branch, which makes none, is heard too, so its
    /// direction is where the next instruction is.
    pub fn step_each(
        &mut self,
        input: &InputFrame,
        map: &mut crate::coverage::CodeMap,
        seen: &mut dyn FnMut(u16, u8, Option<Access>),
    ) -> Result<bool, String> {
        self.step_regs(input, map, &mut |pc, op, access, _| seen(pc, op, access))
    }

    /// [`Self::step_each`], with the A, X and Y registers before each
    /// instruction too: what a compare compares, for a branch's distance
    /// from going the other way.
    pub fn step_regs(
        &mut self,
        input: &InputFrame,
        map: &mut crate::coverage::CodeMap,
        seen: &mut dyn FnMut(u16, u8, Option<Access>, [u8; 3]),
    ) -> Result<bool, String> {
        self.settled = std::sync::OnceLock::new();
        let pads = input.buttons.to_le_bytes();
        if !input.axes.is_empty() || pads[2] != 0 || pads[3] != 0 {
            return Err("gp-nes applies controllers 1 and 2 only, with no axes".into());
        }
        let mut watch = crate::coverage::Watch::default();
        let out = self
            .nes
            .step_with(
                LogFrame {
                    polled: false,
                    pads,
                    reset: input.reset,
                },
                |bus| {
                    watch.before(bus, map);
                    let pc = bus.cpu.pc;
                    let access = crate::coverage::access(bus).map(|(a, rw)| {
                        let v = if !(0x2000..0x8000).contains(&a) {
                            bus.peek(a)
                        } else {
                            0
                        };
                        (a, rw, v)
                    });
                    let regs = [bus.cpu.acc, bus.cpu.x, bus.cpu.y];
                    seen(pc, bus.peek(pc), access, regs);
                },
            )
            .map_err(|e| format!("frame {}: {e}", self.nes.frame() + 1))?;
        self.nes.take_events();
        Ok(out.polled)
    }

    pub fn step_accesses(
        &mut self,
        input: &InputFrame,
        map: &mut crate::coverage::CodeMap,
        seen: &mut dyn FnMut(u16, u8, u16, u8, u8),
    ) -> Result<bool, String> {
        self.settled = std::sync::OnceLock::new();
        let pads = input.buttons.to_le_bytes();
        if !input.axes.is_empty() || pads[2] != 0 || pads[3] != 0 {
            return Err("gp-nes applies controllers 1 and 2 only, with no axes".into());
        }
        let mut watch = crate::coverage::Watch::default();
        let out = self
            .nes
            .step_with(
                LogFrame {
                    polled: false,
                    pads,
                    reset: input.reset,
                },
                |bus| {
                    watch.before(bus, map);
                    if let Some((a, rw)) = crate::coverage::access(bus) {
                        // The byte there before the instruction runs, from
                        // RAM or ROM only: a register's read has effects.
                        let v = if !(0x2000..0x8000).contains(&a) {
                            bus.peek(a)
                        } else {
                            0
                        };
                        seen(bus.cpu.pc, bus.cpu.x, a, rw, v);
                    }
                },
            )
            .map_err(|e| format!("frame {}: {e}", self.nes.frame() + 1))?;
        self.nes.take_events();
        Ok(out.polled)
    }

    /// [`NesRuntime::step_covered`], also giving `tracer` each instruction
    /// ([`crate::ramrole`]), for the RAM bytes' roles.
    pub fn step_traced(
        &mut self,
        input: &InputFrame,
        map: &mut crate::coverage::CodeMap,
        tracer: &mut crate::ramrole::Tracer,
    ) -> Result<bool, String> {
        self.settled = std::sync::OnceLock::new();
        let pads = input.buttons.to_le_bytes();
        if !input.axes.is_empty() || pads[2] != 0 || pads[3] != 0 {
            return Err("gp-nes applies controllers 1 and 2 only, with no axes".into());
        }
        tracer.frame_start(self.nes.bus_mut_for_observers());
        let mut watch = crate::coverage::Watch::default();
        let out = self
            .nes
            .step_with(
                LogFrame {
                    polled: false,
                    pads,
                    reset: input.reset,
                },
                |bus| {
                    watch.before(bus, map);
                    tracer.before(bus);
                },
            )
            .map_err(|e| format!("frame {}: {e}", self.nes.frame() + 1))?;
        self.nes.take_events();
        tracer.frame_end(self.nes.bus_mut_for_observers());
        Ok(out.polled)
    }

    /// Runs until `condition` holds, as [`Runtime::run_until`] does, but
    /// keeps no events: the event log is off while it runs, so memory stays
    /// flat however many frames it takes. Returns whether the condition was
    /// met. For callers that only want the frame where it stops, such as
    /// `gp run-until`.
    pub fn run_until_frames(
        &mut self,
        condition: &str,
        inputs: &[InputFrame],
        max_frames: u32,
    ) -> Result<bool, String> {
        let cond = Condition::parse(condition)?;
        self.nes.set_events(false);
        let met = (|| {
            if self.holds(&cond)? {
                return Ok(true);
            }
            for i in 0..max_frames as usize {
                self.advance(&input_at(inputs, i))?;
                if self.holds(&cond)? {
                    return Ok(true);
                }
            }
            Ok(false)
        })();
        self.nes.set_events(true);
        met
    }

    /// The internal RAM `ram[...]` reads: at the frame's end, or as the CPU
    /// takes the next NMI when `read_at_nmi` is set and the two differ.
    fn ram(&self) -> &[u8] {
        if !self.at_nmi {
            return self.nes.wram();
        }
        self.at_nmi().wram()
    }

    /// The console as the CPU takes the next NMI, or this one when it is
    /// at the NMI's entry already or takes none within a frame.
    fn at_nmi(&self) -> &Nes {
        self.settled
            .get_or_init(|| self.nes.at_next_nmi().map(Box::new))
            .as_deref()
            .unwrap_or(&self.nes)
    }

    /// Feature `field.machine`'s `nmi.` selectors: the console as the CPU
    /// takes the next NMI, whatever `read_at_nmi` says. `nmi.` and any
    /// canonical state field (`nmi.cpu.cycles`, `nmi.ppu.odd_frame`,
    /// `nmi.ram[0x18]`), and three the NMI's entry gives: `nmi.return_pc`
    /// and `nmi.status`, the return address and status it pushed (where
    /// the code it stopped goes on, and its carry), and `nmi.vblank_dots`,
    /// the PPU dots from the vblank (scanline 241, dot 1) to the entry.
    fn read_nmi(&self, sel: &Selector, rest: &str) -> Result<Vec<Entry>, String> {
        let nes = self.at_nmi();
        let bus = nes.bus();
        let int = |v: i64| {
            Ok(vec![Entry {
                path: format!("{sel}"),
                value: Value::Int(v),
            }])
        };
        let stack = |k: u8| i64::from(bus.peek(0x100 + u16::from(bus.cpu.sp.wrapping_add(k))));
        match rest {
            "return_pc" => int(stack(2) | stack(3) << 8),
            "status" => int(stack(1)),
            "vblank_dots" => {
                let line = i64::from(bus.ppu.scanline);
                let line = if line < 241 { line + 262 } else { line };
                int((line - 241) * 341 + i64::from(bus.ppu.cycle) - 1)
            }
            _ => {
                let sub = Selector::parse(rest)?;
                let state = nes.canonical_state().map_err(|e| e.to_string())?;
                let mut out = read_state(&state, nes.frame(), &sub)?;
                for e in &mut out {
                    e.path = format!("nmi.{}", e.path);
                }
                Ok(out)
            }
        }
    }

    /// Runs one frame and returns whether the game polled the controllers.
    fn advance(&mut self, input: &InputFrame) -> Result<bool, String> {
        self.settled = std::sync::OnceLock::new();
        if !input.axes.is_empty() {
            return Err("the NES has no analog input; axes must be empty".into());
        }
        let pads = input.buttons.to_le_bytes();
        if pads[2] != 0 || pads[3] != 0 {
            return Err("gp-nes applies controllers 1 and 2 only".into());
        }
        let out = self
            .nes
            .step(LogFrame {
                polled: false,
                pads,
                reset: input.reset,
            })
            .map_err(|e| format!("frame {}: {e}", self.nes.frame() + 1))?;
        Ok(out.polled)
    }

    /// Runs one frame and returns its events.
    fn step_frame(&mut self, input: &InputFrame) -> Result<Vec<Event>, String> {
        let polled = self.advance(input)?;
        let mut events: Vec<Event> = self.nes.take_events().iter().map(stream_event).collect();
        events.push(Event {
            frame: self.nes.frame(),
            kind: "frame".into(),
            entity: None,
            data: vec![Entry {
                path: "polled".into(),
                value: Value::Flag(polled),
            }],
        });
        Ok(events)
    }

    /// Reads a parsed selector.
    pub fn read(&self, sel: &Selector) -> Result<Vec<Entry>, String> {
        let name = sel.name();
        if let Some(rest) = format!("{sel}").strip_prefix("nmi.") {
            return self.read_nmi(sel, rest);
        }
        let index = sel.last_index()?;
        let int = |path: String, v: u64| Entry {
            path,
            value: Value::Int(v as i64),
        };
        match (name.as_str(), index) {
            ("frame", None) => return Ok(vec![int(name, self.nes.frame())]),
            ("ram", Some(index)) => {
                return indexed(&name, index, self.ram());
            }
            // OAM and the name tables as the canonical state has them,
            // without encoding the whole state for one byte (a state
            // mapping's `field.oam` and `field.nametable` read them a byte
            // at a time).
            ("oam", Some(index)) => {
                return indexed(&name, index, &self.nes.oam());
            }
            ("nametables", Some(index)) => {
                return indexed(&name, index, self.nes.nametables());
            }
            ("bus", Some(index)) => {
                let bus = self.nes.bus();
                return range(&name, index, 0x1_0000)?
                    .map(|addr| {
                        Ok(int(
                            format!("bus[{}]", hex(addr as i64)),
                            bus.peek(addr as u16).into(),
                        ))
                    })
                    .collect();
            }
            ("bus", None) => return Err(needs_index("bus", 0x1_0000)),
            _ => {}
        }
        let state = self.nes.canonical_state().map_err(|e| e.to_string())?;
        read_state(&state, self.nes.frame(), sel)
    }

    /// Reads a selector that must name one integer, for conditions.
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

/// Whether reading `sel` needs a running console rather than a stored
/// state: `bus[...]` reads through the cartridge's mapper.
pub fn reads_console(sel: &Selector) -> bool {
    sel.name() == "bus"
}

/// Reads a selector from a canonical state alone, such as a frame of a
/// recording's trace, as [`NesRuntime`] reads it from the console; `frame`
/// answers the `frame` selector. Refuses `bus`, which needs the console
/// ([`reads_console`]), and a state of another field table version.
pub fn read_state(state: &[u8], frame: u64, sel: &Selector) -> Result<Vec<Entry>, String> {
    let name = sel.name();
    let index = sel.last_index()?;
    let int = |path: String, v: u64| Entry {
        path,
        value: Value::Int(v as i64),
    };
    if reads_console(sel) {
        return match index {
            Some(_) => Err(format!(
                "{sel} reads through the cartridge's mapper, so it needs the console, not a stored state"
            )),
            None => Err(needs_index("bus", 0x1_0000)),
        };
    }
    if (name.as_str(), index) == ("frame", None) {
        return Ok(vec![int(name, frame)]);
    }
    let fields = canonical::decode(state).map_err(|e| e.to_string())?;
    if let Some(field) = canonical::FIELDS.iter().find(|f| f.name == name) {
        let bytes = fields
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| *b)
            .expect("decode gives every field");
        return match (field.width, index) {
            (Width::Fixed(n), None) if n <= 8 => Ok(vec![int(name, le(bytes))]),
            (_, Some(index)) if is_memory(field.width) => indexed(&name, index, bytes),
            (_, None) => Err(needs_index(&name, bytes.len())),
            (_, Some(_)) => Err(format!("{name} is a number, so it takes no index")),
        };
    }
    if index.is_none() {
        let prefix = format!("{name}.");
        let under: Vec<Entry> = canonical::FIELDS
            .iter()
            .zip(&fields)
            .filter(|(f, _)| f.name.starts_with(&prefix))
            .filter(|(f, _)| !is_memory(f.width))
            .map(|(f, (_, bytes))| int(f.name.to_owned(), le(bytes)))
            .collect();
        if !under.is_empty() {
            return Ok(under);
        }
    }
    Err(format!(
        "no field named {name}; docs/runtime-protocol.md lists the selectors"
    ))
}

/// Whether `cond` holds in a canonical state, read as [`read_state`] reads.
pub fn holds_in_state(state: &[u8], frame: u64, cond: &Condition) -> Result<bool, String> {
    cond.eval(&mut |sel| match read_state(state, frame, sel)?.as_slice() {
        [Entry { value, .. }] => value
            .as_int()
            .ok_or_else(|| format!("{sel} isn't a number")),
        many => Err(format!(
            "{sel} names {} values; a condition needs exactly one",
            many.len()
        )),
    })
}

/// The kind of the event that ends each frame. No stream kind has this name.
pub const FRAME_KIND: &str = "frame";

/// An event from the event stream as a protocol event: its kind's stream
/// name, then `scanline` and `dot`, then its own fields. An interrupt has
/// the handler's `addr`, a sprite 0 hit nothing more, and the rest `addr`
/// and `value`, as `gp events` prints them.
pub fn stream_event(e: &gp_evidence::events::Event) -> Event {
    let int = |path: &str, v: i64| Entry {
        path: path.into(),
        value: Value::Int(v),
    };
    let mut data = vec![int("scanline", e.scanline.into()), int("dot", e.dot.into())];
    match e.kind {
        EventKind::Sprite0Hit => {}
        EventKind::Nmi | EventKind::Irq => data.push(int("addr", e.addr.into())),
        _ => {
            data.push(int("addr", e.addr.into()));
            data.push(int("value", e.value.into()));
        }
    }
    Event {
        frame: e.frame,
        kind: e.kind.name().into(),
        entity: None,
        data,
    }
}

/// The next `n` bytes of a snapshot being read.
fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
    if rest.len() < n {
        return Err("the snapshot is truncated".into());
    }
    let (head, tail) = rest.split_at(n);
    *rest = tail;
    Ok(head)
}

/// Whether a field is a memory, read by index, rather than a number.
fn is_memory(width: Width) -> bool {
    match width {
        Width::Fixed(n) => n > 8,
        Width::Region => true,
    }
}

fn needs_index(name: &str, len: usize) -> String {
    format!(
        "{name} is {len} bytes; select one ({name}[0x10]), a range ({name}[0..16]) or all ({name}[*])"
    )
}

/// A little-endian integer.
fn le(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .rev()
        .fold(0, |acc, b| (acc << 8) | u64::from(*b))
}

/// The positions an index selects in a memory of `len` bytes. Every
/// selection must be inside the memory and hold at least one byte.
fn range(name: &str, index: Index, len: usize) -> Result<std::ops::Range<usize>, String> {
    if len == 0 {
        return Err(format!("{name} is empty on this cartridge"));
    }
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

/// Reads `sel` from internal RAM's bytes `wram` as [`NesRuntime`]'s
/// `query` does, when it names `ram[...]`; `None` for any other selector.
/// A kept copy of a frame's RAM answers the same as the console did.
pub fn read_ram(wram: &[u8], sel: &Selector) -> Option<Result<Vec<Entry>, String>> {
    if sel.name() != "ram" {
        return None;
    }
    match sel.last_index() {
        Err(e) => Some(Err(e)),
        Ok(Some(index)) => Some(indexed("ram", index, wram)),
        Ok(None) => None,
    }
}

fn indexed(name: &str, index: Index, bytes: &[u8]) -> Result<Vec<Entry>, String> {
    let range = range(name, index, bytes.len())?;
    Ok(bytes[range.clone()]
        .iter()
        .zip(range)
        .map(|(b, i)| Entry {
            path: format!("{name}[{}]", hex(i as i64)),
            value: Value::Int((*b).into()),
        })
        .collect())
}

impl Runtime for NesRuntime {
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
        self.nes.frame()
    }

    fn snapshot(&self) -> Vec<u8> {
        let state = self
            .nes
            .snapshot()
            .expect("tetanes-core serializes into a buffer of its own size");
        let version = runtime_ref().version;
        let mut out = Vec::with_capacity(4 + 4 + 4 + version.len() + 8 + 4 + state.len());
        out.extend_from_slice(SNAPSHOT_MAGIC);
        out.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
        out.extend_from_slice(&(version.len() as u32).to_le_bytes());
        out.extend_from_slice(version.as_bytes());
        out.extend_from_slice(&self.nes.frame().to_le_bytes());
        out.extend_from_slice(&self.nes.held());
        out.extend_from_slice(&state);
        out
    }

    /// Refused: the console has no model state to load.
    fn load_state(&mut self, _state: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
        Err("gp-nes runs the original, which has no model state to load; restore takes its snapshots".into())
    }

    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        let mut rest = snap;
        let mut take = |n| take(&mut rest, n);
        if take(4)? != SNAPSHOT_MAGIC {
            return Err("not a gp-nes snapshot".into());
        }
        let version = u32::from_le_bytes(take(4)?.try_into().unwrap());
        if version != SNAPSHOT_VERSION {
            return Err(format!("snapshot version {version} isn't supported"));
        }
        let len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let made_by = String::from_utf8_lossy(take(len)?).into_owned();
        let current = runtime_ref().version;
        if made_by != current {
            return Err(format!(
                "the snapshot was made by gp-nes {made_by}, and this is gp-nes {current}"
            ));
        }
        let frame = u64::from_le_bytes(take(8)?.try_into().unwrap());
        let held: [u8; 4] = take(4)?.try_into().unwrap();
        self.settled = std::sync::OnceLock::new();
        self.nes
            .restore(rest, frame, held)
            .map_err(|e| e.to_string())
    }

    fn fork(&self) -> Self {
        self.clone()
    }

    fn canonical_state(&self) -> Vec<u8> {
        self.nes
            .canonical_state()
            .expect("NesRuntime::new checked the canonical state is defined")
    }

    /// The first 8 bytes, little-endian, of the BLAKE3 hash that recordings
    /// store for each frame.
    fn state_hash(&self) -> u64 {
        let hash = Hash::of(&self.canonical_state());
        u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
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

    /// The original runtime has no entities until a state mapping exists.
    fn entities_at(&self, _area: Rect) -> Vec<u64> {
        Vec::new()
    }

    fn patch(&mut self, _model_diff: &str) -> PatchResult {
        PatchResult::Rejected("the original game has no model to patch".into())
    }

    /// Sets one byte of internal RAM, `ram[addr]`, for test setup, or
    /// `read_at_nmi`: whether `ram[...]` reads the RAM as the CPU takes the
    /// next NMI, so a frame whose NMI handler runs past the frame's end is
    /// read whole ([`Nes::wram_at_next_nmi`]). Nothing else is settable.
    fn set(&mut self, p: &str, v: Value) -> Result<(), String> {
        if p == "read_at_nmi" {
            let Value::Flag(on) = v else {
                return Err(format!("{p}: {v} isn't a flag"));
            };
            self.at_nmi = on;
            return Ok(());
        }
        self.settled = std::sync::OnceLock::new();
        let sel = Selector::parse(p)?;
        let (Some(Index::At(addr)), "ram") = (sel.last_index()?, sel.name().as_str()) else {
            return Err(format!("{p}: only ram[addr] can be set"));
        };
        let byte = match v.as_int() {
            Some(n @ 0..=255) => n as u8,
            _ => return Err(format!("{p}: {v} isn't a byte")),
        };
        let addr = range("ram", Index::At(addr), 0x800)?.start;
        self.nes.deck.wram_mut()[addr] = byte;
        Ok(())
    }

    /// Nothing yet: the original routine behind a value needs the state
    /// mapping and the disassembly labels.
    fn explain(&self, _p: &str) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrom;
    use gp_evidence::input_log::button;

    fn runtime() -> NesRuntime {
        NesRuntime::load(&testrom::build(), PowerOn::default()).unwrap()
    }

    fn press(pad: u8) -> InputFrame {
        InputFrame::pads([pad, 0, 0, 0])
    }

    fn one(rt: &NesRuntime, sel: &str) -> i64 {
        match rt.query(sel).unwrap().as_slice() {
            [e] => e.value.as_int().unwrap(),
            other => panic!("{sel}: {other:?}"),
        }
    }

    #[test]
    fn a_covered_frame_ends_where_an_uncovered_one_does() {
        let (mut a, mut b) = (runtime(), runtime());
        let mut map = crate::coverage::CodeMap::new();
        for k in 0..40u8 {
            let i = press(if k % 7 == 0 { button::A } else { 0 });
            a.step(std::slice::from_ref(&i)).unwrap();
            b.step_covered(&i, &mut map).unwrap();
        }
        assert_eq!(a.state_hash(), b.state_hash());
        assert_eq!(a.frame(), b.frame());
        // The reset vector's code ran, and some branch went each way.
        assert!(map.count_in(0x8000..0x1_0000) > 10);
        assert!(map.count_in(crate::coverage::TAKEN..crate::coverage::FELL) > 0);
        assert!(map.count_in(crate::coverage::FELL..crate::coverage::EXTRA) > 0);
    }

    #[test]
    fn steps_one_frame_per_input_with_events() {
        let mut rt = runtime();
        let all = rt.step(&vec![press(0); 40]).unwrap();
        assert_eq!(rt.frame(), 40);
        let events: Vec<&Event> = all.iter().filter(|e| e.kind == FRAME_KIND).collect();
        assert_eq!(events.len(), 40);
        assert_eq!(
            events.iter().map(|e| e.frame).collect::<Vec<_>>(),
            (1..=40).collect::<Vec<_>>()
        );
        // Each frame's stream events come before its `frame` event, in
        // frame order.
        let mut last = (0, false);
        for e in &all {
            assert!(e.frame >= last.0 && !(e.frame == last.0 && last.1), "{e:?}");
            last = (e.frame, e.kind == FRAME_KIND);
        }
        assert!(all.iter().any(|e| e.kind == "nmi"));
        assert!(all.iter().any(|e| e.kind == "controller-read"));
        let polled: Vec<bool> = events
            .iter()
            .map(|e| e.data[0].value == Value::Flag(true))
            .collect();
        // The test ROM polls every other frame.
        assert!(polled[20..].windows(2).all(|w| w[0] != w[1]));
    }

    /// The protocol returns the same events a recording stores, frame by
    /// frame, under the stream's kind names.
    #[test]
    fn step_returns_the_event_stream() {
        let mut rt = runtime();
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        nes.set_events(true);
        for f in 0..90u64 {
            let pad = if f % 7 == 0 { button::A } else { 0 };
            let got = rt.step(&[press(pad)]).unwrap();
            nes.step(LogFrame {
                polled: false,
                pads: [pad, 0, 0, 0],
                reset: false,
            })
            .unwrap();
            let want: Vec<Event> = nes.take_events().iter().map(stream_event).collect();
            assert_eq!(got[..got.len() - 1], want[..], "frame {}", f + 1);
            assert_eq!(got.last().unwrap().kind, FRAME_KIND);
        }
        let nmi = stream_event(&gp_evidence::events::Event {
            frame: 3,
            kind: EventKind::Nmi,
            scanline: 241,
            dot: 3,
            addr: 0x8082,
            value: 0,
        });
        assert_eq!(nmi.kind, "nmi");
        assert_eq!(
            nmi.data.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            ["scanline", "dot", "addr"]
        );
    }

    /// Reading a stored state gives what the console gives, for every
    /// selector but `bus`.
    #[test]
    fn reads_a_stored_state_like_the_console() {
        let mut rt = runtime();
        rt.step(&vec![press(button::A); 50]).unwrap();
        let state = rt.canonical_state();
        for sel in [
            "frame",
            "cpu",
            "cpu.pc",
            "ppu.scanline",
            "ram[0x00..0x20]",
            "oam[*]",
            "palette[3]",
            "apu.pulse1",
            "nothing",
            "cpu.pc[1]",
            "ram",
        ] {
            let parsed = Selector::parse(sel).unwrap();
            assert_eq!(
                read_state(&state, rt.frame(), &parsed),
                rt.query(sel),
                "{sel}"
            );
        }
        let bus = Selector::parse("bus[0x8000]").unwrap();
        assert!(reads_console(&bus) && read_state(&state, 50, &bus).is_err());
        let cond = Condition::parse("frame == 50 && cpu.s > 0").unwrap();
        assert!(holds_in_state(&state, 50, &cond).unwrap());
        let pc = Selector::parse("cpu.pc").unwrap();
        assert!(read_state(&state[..20], 50, &pc).is_err());
    }

    /// The WIT's `event` record lists the kinds this runtime returns, so the
    /// interface and the Rust side name them the same way.
    #[test]
    fn the_wit_names_every_event_kind() {
        let wit = include_str!("../../../wit/runtime/runtime.wit");
        let start = wit.find("record event {").unwrap();
        let comment = &wit[wit[..start].rfind("\n\n").unwrap()..start];
        for kind in EventKind::NES
            .map(EventKind::name)
            .into_iter()
            .chain([FRAME_KIND])
        {
            assert!(
                comment.contains(&format!("`{kind}`")),
                "{kind} isn't in the WIT"
            );
        }
        assert!(EventKind::from_name(FRAME_KIND).is_none());
    }

    #[test]
    fn a_reset_reaches_the_runtime_once() {
        // The protocol presses reset where the input frame says, like
        // Nes::step, and doesn't repeat it when holding the last input.
        let mut rt = runtime();
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        let inputs: Vec<InputFrame> = (0..60u64)
            .map(|f| InputFrame {
                reset: f == 30 || f == 59,
                ..press(if f % 5 == 0 { button::A } else { 0 })
            })
            .collect();
        rt.step(&inputs).unwrap();
        for i in &inputs {
            nes.step(LogFrame {
                polled: false,
                pads: i.buttons.to_le_bytes(),
                reset: i.reset,
            })
            .unwrap();
        }
        assert_eq!(rt.canonical_state(), nes.canonical_state().unwrap());
        assert!(input_at(&inputs, 59).reset && !input_at(&inputs, 60).reset);
    }

    #[test]
    fn matches_the_plain_runtime() {
        let mut rt = runtime();
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        for f in 0..120u64 {
            let pad = if f % 5 == 0 { button::A } else { 0 };
            rt.step(&[press(pad)]).unwrap();
            nes.step(LogFrame {
                polled: false,
                pads: [pad, 0, 0, 0],
                reset: false,
            })
            .unwrap();
            assert_eq!(rt.canonical_state(), nes.canonical_state().unwrap());
        }
        let full = nes.state_hash().unwrap();
        assert_eq!(
            rt.state_hash(),
            u64::from_le_bytes(full.as_bytes()[..8].try_into().unwrap())
        );
    }

    #[test]
    fn queries_raw_state() {
        let mut rt = runtime();
        rt.step(&vec![press(button::A | button::RIGHT); 30])
            .unwrap();
        let buttons = testrom::addr::BUTTONS as i64;
        assert_eq!(one(&rt, &format!("ram[{buttons}]")), 0b1000_0001);
        assert_eq!(one(&rt, "frame"), 30);
        assert_eq!(one(&rt, "ppu.scanline"), 241);
        assert_eq!(one(&rt, "cpu.pc"), i64::from(rt.nes().bus().cpu.pc));
        // The bus reads RAM through its mirrors.
        assert_eq!(one(&rt, &format!("bus[{}]", buttons + 0x800)), 0b1000_0001);
        let oam = rt.query("oam[4..8]").unwrap();
        assert_eq!(oam.len(), 4);
        assert_eq!(oam[0].path, "oam[0x0004]");
        assert_eq!(rt.query("palette[*]").unwrap().len(), 32);
        assert_eq!(rt.query("ram[$7ff]").unwrap()[0].path, "ram[0x07ff]");
        let cpu = rt.query("cpu").unwrap();
        let names: Vec<&str> = cpu.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(&names[..3], ["cpu.a", "cpu.x", "cpu.y"]);
        assert!(rt.query("apu.pulse1").unwrap().len() > 10);
        for bad in [
            "ram",
            "ram[0x800]",
            "ram[0..0x801]",
            "ram[0x7fffffffffffffff]",
            "ram[-1]",
            "ram[0x10..0x10]",
            "ram[0x7ff..0x7fffffffffffffff]",
            "bus[0x10000]",
            "bus[0x7fffffffffffffff]",
            "cpu.pc[0]",
            "nope",
            "bus",
            "ram[",
            "entities.enemies[*].pos",
        ] {
            assert!(rt.query(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn runs_until_a_condition_holds() {
        let mut rt = runtime();
        let frames = format!("ram[{}]", testrom::addr::FRAMES);
        let until = rt.run_until(&format!("{frames} >= 10"), &[], 1000).unwrap();
        assert!(until.met);
        assert_eq!(one(&rt, &frames), 10);
        let frames_run = until.events.iter().filter(|e| e.kind == FRAME_KIND).count();
        assert_eq!(frames_run as u64, rt.frame());
        // Already true: no frames run.
        let again = rt.run_until(&format!("{frames} == 10"), &[], 5).unwrap();
        assert!(again.met && again.events.is_empty());
        // Not reached within the limit.
        let start = rt.frame();
        let never = rt.run_until(&format!("{frames} == 0x1ff"), &[], 7).unwrap();
        assert!(!never.met);
        assert_eq!(rt.frame(), start + 7);
        assert!(rt.run_until("ram[0..2] == 1", &[], 1).is_err());
        assert!(rt.run_until("ram[0] ==", &[], 1).is_err());
    }

    #[test]
    fn samples_one_row_per_frame() {
        let mut rt = runtime();
        let selectors = vec!["frame".to_owned(), "oam[0..2]".to_owned()];
        let rows = rt.sample(&selectors, &[press(0)], 5).unwrap();
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| r.len() == 3));
        assert_eq!(rows[4][0].value, Value::Int(5));
    }

    #[test]
    fn snapshot_restores_frame_buttons_and_state() {
        let mut rt = runtime();
        rt.step(&vec![press(button::B); 50]).unwrap();
        let snap = rt.snapshot();
        let (state, frame) = (rt.canonical_state(), rt.frame());
        let ahead: Vec<u64> = (0..30)
            .map(|_| {
                rt.step(&[press(button::LEFT)]).unwrap();
                rt.state_hash()
            })
            .collect();
        let mut other = runtime();
        other.restore(&snap).unwrap();
        assert_eq!((other.canonical_state(), other.frame()), (state, frame));
        assert_eq!(other.nes().held(), [button::B, 0, 0, 0]);
        assert_eq!(other.nes().snapshot().unwrap(), rt_snapshot_state(&snap));
        let again: Vec<u64> = (0..30)
            .map(|_| {
                other.step(&[press(button::LEFT)]).unwrap();
                other.state_hash()
            })
            .collect();
        assert_eq!(again, ahead);

        assert!(other.restore(&snap[..20]).is_err());
        assert!(other.restore(b"nope").is_err());
        let mut foreign = snap.clone();
        foreign[12] ^= 1; // a byte of the runtime version
        assert!(other.restore(&foreign).unwrap_err().contains("made by"));
    }

    /// The `tetanes-core` save state inside a protocol snapshot.
    fn rt_snapshot_state(snap: &[u8]) -> Vec<u8> {
        let len = u32::from_le_bytes(snap[8..12].try_into().unwrap()) as usize;
        snap[12 + len + 8 + 4..].to_vec()
    }

    #[test]
    fn ram_read_at_the_next_nmi_leaves_the_console_where_it_was() {
        let mut rt = runtime();
        let mut plain = runtime();
        rt.set("read_at_nmi", Value::Flag(true)).unwrap();
        assert!(rt.set("read_at_nmi", Value::Int(1)).is_err());
        for _ in 0..30 {
            rt.step(&[press(0)]).unwrap();
            plain.step(&[press(0)]).unwrap();
            // A read runs a copy on; the console itself stays at the
            // frame's end, as one that reads at the end does.
            let at_nmi = rt.query("ram[0..0x800]").unwrap();
            assert_eq!(at_nmi.len(), 0x800);
            assert_eq!(rt.state_hash(), plain.state_hash());
            // The RAM at the next NMI is the frame's end's run on: the
            // copy's own answer, the same however often it's asked.
            assert_eq!(rt.query("ram[0..0x800]").unwrap(), at_nmi);
            match rt.nes.wram_at_next_nmi() {
                Some(ram) => {
                    let ints: Vec<i64> = ram.iter().map(|b| i64::from(*b)).collect();
                    let read: Vec<i64> = at_nmi.iter().map(|e| e.value.as_int().unwrap()).collect();
                    assert_eq!(read, ints);
                }
                None => assert_eq!(one(&rt, "ram[0x10]"), i64::from(rt.nes.wram()[0x10])),
            }
        }
    }

    /// The `nmi.` selectors read the console as the CPU takes the next
    /// NMI: at its vector, on the vblank's scanline a few dots in, with the
    /// return address it pushed in the program, and leave the console
    /// where it was.
    #[test]
    fn nmi_selectors_read_the_console_at_the_next_nmi() {
        let mut rt = runtime();
        let mut plain = runtime();
        let vector = one(&rt, "bus[0xfffa]") | one(&rt, "bus[0xfffb]") << 8;
        let mut seen = 0;
        for _ in 0..30 {
            rt.step(&[press(0)]).unwrap();
            plain.step(&[press(0)]).unwrap();
            if rt.nes.at_next_nmi().is_none() {
                continue;
            }
            seen += 1;
            assert_eq!(one(&rt, "nmi.cpu.pc"), vector);
            assert_eq!(one(&rt, "nmi.ppu.scanline"), 241);
            let dots = one(&rt, "nmi.vblank_dots");
            assert!((0..60).contains(&dots), "{dots}");
            assert!(one(&rt, "nmi.return_pc") >= 0x8000);
            assert!((0..256).contains(&one(&rt, "nmi.status")));
            assert!(one(&rt, "nmi.cpu.cycles") > 0);
            assert_eq!(rt.state_hash(), plain.state_hash());
        }
        assert!(seen > 0);
    }

    #[test]
    fn forks_are_independent() {
        let mut rt = runtime();
        rt.step(&vec![press(0); 20]).unwrap();
        let mut fork = rt.fork();
        assert_eq!(fork.state_hash(), rt.state_hash());
        fork.step(&vec![press(button::START); 10]).unwrap();
        rt.step(&vec![press(0); 10]).unwrap();
        assert_eq!(fork.frame(), rt.frame());
        assert_ne!(fork.state_hash(), rt.state_hash());
        // The original, stepped the same way again, matches a fresh run.
        let mut fresh = runtime();
        fresh.step(&vec![press(0); 30]).unwrap();
        assert_eq!(fresh.state_hash(), rt.state_hash());
    }

    #[test]
    fn sets_ram_and_refuses_the_rest() {
        let mut rt = runtime();
        rt.set("ram[0x0700]", Value::Int(0x5a)).unwrap();
        assert_eq!(one(&rt, "ram[0x700]"), 0x5a);
        assert!(rt.set("ram[0x0700]", Value::Int(256)).is_err());
        assert!(rt.set("cpu.pc", Value::Int(0)).is_err());
        assert!(rt.set("ram[0x800]", Value::Int(0)).is_err());
        assert_eq!(rt.capabilities().names(), ["snapshot", "fork"]);
        assert!(matches!(rt.patch(""), PatchResult::Rejected(_)));
        assert!(rt.entities_at(Rect::default()).is_empty());
        assert!(rt.explain("ram[0]").is_empty());
    }

    #[test]
    fn refuses_input_it_cant_apply() {
        let mut rt = runtime();
        assert!(rt.step(&[InputFrame::pads([0, 0, 1, 0])]).is_err());
        let analog = InputFrame {
            reset: false,
            buttons: 0,
            axes: vec![3],
        };
        assert!(rt.step(&[analog]).is_err());
        assert_eq!(rt.frame(), 0);
    }

    #[test]
    fn an_empty_memory_selects_nothing() {
        for index in [Index::All, Index::At(0), Index::Range(0, 1)] {
            let err = range("chr_ram", index, 0).unwrap_err();
            assert!(err.contains("empty"), "{err}");
        }
        assert_eq!(range("ram", Index::Range(1, 3), 4), Ok(1..3));
        assert!(range("ram", Index::At(i64::MAX), 4).is_err());
        assert!(range("ram", Index::Range(i64::MIN, 2), 4).is_err());
    }

    /// Selectors and conditions built from the grammar's tokens, with
    /// indexes at the edges of every memory and of i64, must give an answer
    /// or an error, never a panic.
    #[test]
    fn fuzzed_selectors_and_conditions_never_panic() {
        const NAMES: &[&str] = &[
            "ram",
            "bus",
            "oam",
            "palette",
            "chr_ram",
            "prg_ram",
            "nametables",
            "cpu",
            "cpu.pc",
            "frame",
            "apu.pulse1",
            "nope",
        ];
        const NUMBERS: &[&str] = &[
            "0",
            "1",
            "0x7ff",
            "0x800",
            "0xffff",
            "0x10000",
            "-1",
            "$1f",
            "0x7fffffffffffffff",
            "-0x7fffffffffffffff",
            "9223372036854775807",
            "18446744073709551616",
        ];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let rt = runtime();
        let mut answered = 0;
        for _ in 0..3000 {
            let name = NAMES[next(NAMES.len())];
            let (a, b) = (NUMBERS[next(NUMBERS.len())], NUMBERS[next(NUMBERS.len())]);
            let sel = match next(4) {
                0 => name.to_owned(),
                1 => format!("{name}[{a}]"),
                2 => format!("{name}[{a}..{b}]"),
                _ => format!("{name}[*]"),
            };
            if let Ok(entries) = rt.query(&sel) {
                assert!(!entries.is_empty(), "{sel}");
                answered += 1;
            }
            let op = ["==", "<", "&", "!=", ""][next(5)];
            let cond = format!("!({sel} {op} {b}) || {a}");
            let _ = rt.clone().run_until(&cond, &[], 1);
        }
        assert!(answered > 100, "{answered}");
    }
}
