//! The original's side of a clip, kept. A clip's score runs the original in gp-nes and the
//! rebuilt game in the interpreter side by side, and the original's run
//! reads nothing of the model: the same ROM, start and presses give the
//! same frames whatever the model is. So the first score of a clip keeps
//! what the lockstep read from the original, frame by frame, in a tape:
//!
//! | Part | What |
//! |---|---|
//! | RAM | The 2 KB of internal RAM on each frame something was read, so any `ram[...]` selector answers from it, whatever the mapping reads. For an arcade original, the board's RAM record (gp-arcade's 5,760 bytes: work, sprite, video and palette RAM and the sound board's), which answers `ram[...]` from 0xE000 up, `sound_ram[...]` and `palette[...]` |
//! | Other reads | The answer to any other selector, by frame and selector |
//! | Steps | Each frame's step: the frame after it and its events |
//! | Pictures | The original's picture on each frame the screen measure took one: each pixel's colour index, with the sprite bit. An arcade picture is its pens (0 to 63) with the sprite bit, after the 64 colours it was drawn with |
//! | Read at NMI | Whether `ram[...]` read the RAM as the CPU takes the next NMI (a mapping's `read_at = "nmi"`), so the RAM kept is what those reads saw. A replay is read the same way or not at all |
//!
//! A later score plays the tape in place of the emulator ([`Original`] is a
//! [`Runtime`] either way), so it runs only the rebuilt game and the
//! comparison. Anything the tape doesn't hold (a frame, a selector, a
//! picture, a call it can't answer) marks the replay missed, and the clip
//! is run on the emulator again, which writes a new tape. A tape is keyed
//! by the ROM (for an arcade romset, its identity, so any zip of the same
//! chips shares tapes), the clip's start and presses, the input played to
//! the start and the sources of gp-nes, gp-arcade and what they build on
//! ([`key_parts`]), so a change to either emulator never reads an old one. The mapping isn't in the
//! key: RAM answers any selector, and a selector the tape never saw is a
//! miss.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use crate::original::OriginalRuntime;
use base64::Engine as _;
use gp_present::raster::Frame;
use gp_runtime::select::Selector;
use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, LoadNote, PatchResult, Rect, Runtime, Until, Value,
};
use serde::{Deserialize, Serialize};

fn pack(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(lz4_flex::compress_prepend_size(bytes))
}

fn unpack(s: &str) -> Option<Vec<u8>> {
    let b = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
    lz4_flex::decompress_size_prepended(&b).ok()
}

/// The tape format's version: a change to what a tape holds or how it is
/// read bumps it.
/// Tape 3 adds the arcade's RAM record and pictures.
pub(crate) const TAPE_VERSION: &str = "tape 3";

/// What the lockstep read from the original.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Tape {
    /// The original's frame at the clip's start.
    start: u64,
    /// RAM by frame, compressed, in base64.
    ram: BTreeMap<u64, String>,
    /// Other selectors' answers by frame.
    other: BTreeMap<u64, BTreeMap<String, Answer>>,
    /// Each step by the frame it starts from: the frame after and the
    /// events.
    steps: BTreeMap<u64, (u64, Vec<Ev>)>,
    /// Pictures by frame: a colour index a pixel, the sprite bit as 0x80,
    /// compressed, in base64.
    pictures: BTreeMap<u64, String>,
    /// Whether the RAM was read as the CPU takes the next NMI
    /// (`set("read_at_nmi", true)`).
    #[serde(default)]
    read_at_nmi: bool,
    /// Whether it is an arcade original's: `ram` then holds RAM records
    /// and `pictures` pens after their palette.
    #[serde(default)]
    arcade: bool,
}

#[derive(Clone, Serialize, Deserialize)]
enum V {
    I(i64),
    F(i64),
    B(bool),
    T(String),
    E(u64),
    P(i64, i64),
}

impl From<&Value> for V {
    fn from(v: &Value) -> Self {
        match v {
            Value::Int(x) => Self::I(*x),
            Value::Fixed(x) => Self::F(*x),
            Value::Flag(x) => Self::B(*x),
            Value::Text(x) => Self::T(x.clone()),
            Value::Entity(x) => Self::E(*x),
            Value::Vec2(x, y) => Self::P(*x, *y),
        }
    }
}

impl From<&V> for Value {
    fn from(v: &V) -> Self {
        match v {
            V::I(x) => Self::Int(*x),
            V::F(x) => Self::Fixed(*x),
            V::B(x) => Self::Flag(*x),
            V::T(x) => Self::Text(x.clone()),
            V::E(x) => Self::Entity(*x),
            V::P(x, y) => Self::Vec2(*x, *y),
        }
    }
}

type Answer = Result<Vec<(String, V)>, String>;

fn to_answer(r: &Result<Vec<Entry>, String>) -> Answer {
    r.as_ref()
        .map(|es| {
            es.iter()
                .map(|e| (e.path.clone(), V::from(&e.value)))
                .collect()
        })
        .map_err(Clone::clone)
}

fn from_answer(a: &Answer) -> Result<Vec<Entry>, String> {
    a.as_ref()
        .map(|es| {
            es.iter()
                .map(|(p, v)| Entry {
                    path: p.clone(),
                    value: v.into(),
                })
                .collect()
        })
        .map_err(Clone::clone)
}

#[derive(Clone, Serialize, Deserialize)]
struct Ev {
    frame: u64,
    kind: String,
    entity: Option<u64>,
    data: Vec<(String, V)>,
}

impl From<&Event> for Ev {
    fn from(e: &Event) -> Self {
        Self {
            frame: e.frame,
            kind: e.kind.clone(),
            entity: e.entity,
            data: e
                .data
                .iter()
                .map(|d| (d.path.clone(), V::from(&d.value)))
                .collect(),
        }
    }
}

impl From<&Ev> for Event {
    fn from(e: &Ev) -> Self {
        Self {
            frame: e.frame,
            kind: e.kind.clone(),
            entity: e.entity,
            data: e
                .data
                .iter()
                .map(|(p, v)| Entry {
                    path: p.clone(),
                    value: v.into(),
                })
                .collect(),
        }
    }
}

const MAGIC: &[u8] = b"gp tape 1\n";

impl Tape {
    /// The tape's bytes: a magic line and its JSON, compressed.
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(lz4_flex::compress_prepend_size(
            &serde_json::to_vec(self).unwrap_or_default(),
        ));
        out
    }

    fn from_bytes(b: &[u8]) -> Option<Self> {
        let body = b.strip_prefix(MAGIC)?;
        serde_json::from_slice(&lz4_flex::decompress_size_prepended(body).ok()?).ok()
    }
}

/// Reads a tape, none when it isn't there or can't be read.
pub(crate) fn load(file: &Path) -> Option<Tape> {
    Tape::from_bytes(&std::fs::read(file).ok()?)
}

/// Writes a tape through a temporary file of the thread's own, as the
/// scenario cache does. A tape that can't be written is only slower next
/// time.
pub(crate) fn save(file: &Path, t: &Tape) {
    if let Some(p) = file.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let tmp = file.with_extension(
        format!("tape.tmp-{:?}", std::thread::current().id()).replace(['(', ')'], ""),
    );
    if std::fs::write(&tmp, t.to_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, file);
    }
}

/// The parts of a tape's key besides the clip: its version and the hash of
/// what the original's side depends on, the sources of gp-nes, the runtime
/// protocol, gp-evidence, the vendored core, `Cargo.lock` and the files
/// here, hashed by this crate's build script. Any change to those changes
/// the key, so a tape is reused only for the same emulator. A model or
/// hook change leaves it: the original's frames don't read the model.
pub(crate) fn key_parts() -> Vec<u8> {
    let mut k = TAPE_VERSION.as_bytes().to_vec();
    k.extend(env!("GP_ORIGINAL_SOURCES").as_bytes());
    k
}

/// Whether an arcade original's RAM record answers `sel`.
fn arcade_record(sel: &Selector) -> bool {
    gp_arcade::protocol::read_record(&[0; gp_arcade::mcr::RAM_RECORD], sel).is_some()
}

/// The original as the lockstep sees it: the emulator, writing what it is
/// asked into a tape, or a tape played back. Forks share the tape and the
/// missed mark.
pub(crate) struct Original {
    live: Option<OriginalRuntime>,
    /// Whether a live run writes the tape.
    record: bool,
    /// A replay's frame.
    frame: u64,
    tape: Rc<RefCell<Tape>>,
    missed: Rc<Cell<bool>>,
    /// A replay's `read_at_nmi`, which must be the tape's for its RAM.
    at_nmi: bool,
    /// A replay's RAM on the frame it read last, unpacked once: a
    /// comparison reads RAM many times a frame.
    ram_at: RefCell<Option<(u64, Rc<Vec<u8>>)>>,
}

impl Original {
    /// The emulator at the clip's start, writing a new tape.
    pub(crate) fn recording(nes: OriginalRuntime) -> Self {
        let tape = Tape {
            start: nes.frame(),
            arcade: nes.platform() == crate::original::Platform::Arcade,
            ..Tape::default()
        };
        Self {
            live: Some(nes),
            record: true,
            frame: 0,
            tape: Rc::new(RefCell::new(tape)),
            missed: Rc::new(Cell::new(false)),
            at_nmi: false,
            ram_at: RefCell::new(None),
        }
    }

    /// A tape played back from the clip's start.
    pub(crate) fn replay(tape: Tape) -> Self {
        Self {
            live: None,
            record: false,
            frame: tape.start,
            tape: Rc::new(RefCell::new(tape)),
            missed: Rc::new(Cell::new(false)),
            at_nmi: false,
            ram_at: RefCell::new(None),
        }
    }

    /// Whether a replay was asked something its tape doesn't hold, or a
    /// recording was asked something a tape can't keep.
    pub(crate) fn missed(&self) -> bool {
        self.missed.get()
    }

    /// The tape as it stands.
    pub(crate) fn into_tape(self) -> Option<Tape> {
        drop(self.live);
        Rc::try_unwrap(self.tape).ok().map(RefCell::into_inner)
    }

    fn miss<T>(&self, what: &str) -> Result<T, String> {
        self.missed.set(true);
        Err(format!("the original's tape has no {what}"))
    }

    /// Notes a call a tape can't keep: the live run goes on, but writes no
    /// tape.
    fn unkept(&self) {
        if self.record {
            self.missed.set(true);
        }
    }

    /// The original's picture as it stands: the screen measure's.
    pub(crate) fn picture(&self) -> Frame {
        use gp_nes::video::{HEIGHT, WIDTH};
        if let Some(live) = &self.live {
            let Some(nes) = live.nes() else {
                if self.record
                    && let OriginalRuntime::Arcade(a) = live
                {
                    let (pens, palette) = a.tapper().pens();
                    let mut px: Vec<u8> = palette
                        .iter()
                        .flat_map(|c| [(c >> 16) as u8, (c >> 8) as u8, *c as u8])
                        .collect();
                    px.extend_from_slice(pens);
                    self.tape.borrow_mut().pictures.insert(a.frame(), pack(&px));
                }
                return crate::screen::Shows::shown(live);
            };
            let f = crate::screen::original_frame(nes);
            if self.record {
                let colors = nes.nes().frame_colors();
                let mask = nes.nes().sprite_mask();
                if colors.iter().any(|c| *c >= 0x80) {
                    self.missed.set(true);
                } else {
                    let px: Vec<u8> = colors
                        .iter()
                        .zip(&mask)
                        .map(|(c, s)| c | if *s { 0x80 } else { 0 })
                        .collect();
                    self.tape
                        .borrow_mut()
                        .pictures
                        .insert(nes.frame(), pack(&px));
                }
            }
            return f;
        }
        let t = self.tape.borrow();
        let px = t.pictures.get(&self.frame).and_then(|b| unpack(b));
        if t.arcade {
            use gp_arcade::video::{HEIGHT, WIDTH};
            let Some(px) = px.filter(|p| p.len() == 64 * 3 + WIDTH * HEIGHT) else {
                drop(t);
                self.missed.set(true);
                return crate::screen::blank_frame(WIDTH, HEIGHT);
            };
            let (palette, pens) = px.split_at(64 * 3);
            return crate::screen::arcade_pens_frame(pens, palette.as_chunks::<3>().0);
        }
        let Some(px) = px.filter(|p| p.len() == WIDTH * HEIGHT) else {
            drop(t);
            self.missed.set(true);
            return Frame {
                w: WIDTH as u32,
                h: HEIGHT as u32,
                rgb: vec![[0; 3]; WIDTH * HEIGHT],
                sprite: vec![false; WIDTH * HEIGHT],
                visible: [0, 0, WIDTH as u32, HEIGHT as u32],
                things: Vec::new(),
            };
        };
        let colors = crate::screen::nes_palette();
        Frame {
            w: WIDTH as u32,
            h: HEIGHT as u32,
            rgb: px.iter().map(|c| colors[usize::from(c & 0x3f)]).collect(),
            sprite: px.iter().map(|c| c & 0x80 != 0).collect(),
            visible: [0, 0, WIDTH as u32, HEIGHT as u32],
            things: Vec::new(),
        }
    }

    /// Turns the emulator's drawing on; a replay's pictures are drawn.
    pub(crate) fn draw(&mut self) {
        if let Some(nes) = &mut self.live {
            nes.set_video(true);
        }
    }
}

impl Runtime for Original {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SNAPSHOT.union(Capabilities::FORK)
    }

    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
        if let Some(nes) = &mut self.live {
            let from = nes.frame();
            let ev = nes.step(inputs)?;
            if self.record {
                if inputs.len() == 1 {
                    self.tape
                        .borrow_mut()
                        .steps
                        .insert(from, (nes.frame(), ev.iter().map(Ev::from).collect()));
                } else {
                    self.missed.set(true);
                }
            }
            return Ok(ev);
        }
        if inputs.len() != 1 {
            return self.miss("step of several frames");
        }
        let got = self.tape.borrow().steps.get(&self.frame).cloned();
        let Some((next, ev)) = got else {
            return self.miss(&format!("step from frame {}", self.frame));
        };
        self.frame = next;
        Ok(ev.iter().map(Event::from).collect())
    }

    fn run_until(
        &mut self,
        condition: &str,
        inputs: &[InputFrame],
        max_frames: u32,
    ) -> Result<Until, String> {
        self.unkept();
        match &mut self.live {
            Some(nes) => nes.run_until(condition, inputs, max_frames),
            None => self.miss("run_until"),
        }
    }

    fn frame(&self) -> u64 {
        self.live
            .as_ref()
            .map_or(self.frame, OriginalRuntime::frame)
    }

    fn snapshot(&self) -> Vec<u8> {
        self.unkept();
        match &self.live {
            Some(nes) => nes.snapshot(),
            None => {
                self.missed.set(true);
                Vec::new()
            }
        }
    }

    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        self.unkept();
        match &mut self.live {
            Some(nes) => nes.restore(snap),
            None => self.miss("restore"),
        }
    }

    fn fork(&self) -> Self {
        Self {
            live: self.live.as_ref().map(OriginalRuntime::fork),
            record: self.record,
            frame: self.frame,
            tape: Rc::clone(&self.tape),
            missed: Rc::clone(&self.missed),
            at_nmi: self.at_nmi,
            ram_at: RefCell::new(None),
        }
    }

    fn canonical_state(&self) -> Vec<u8> {
        self.unkept();
        match &self.live {
            Some(nes) => nes.canonical_state(),
            None => {
                self.missed.set(true);
                Vec::new()
            }
        }
    }

    fn state_hash(&self) -> u64 {
        self.unkept();
        match &self.live {
            Some(nes) => nes.state_hash(),
            None => {
                self.missed.set(true);
                0
            }
        }
    }

    fn load_state(&mut self, state: &[u8]) -> Result<Vec<LoadNote>, String> {
        self.unkept();
        match &mut self.live {
            Some(nes) => nes.load_state(state),
            None => self.miss("load_state"),
        }
    }

    fn query(&self, selector: &str) -> Result<Vec<Entry>, String> {
        let parsed = Selector::parse(selector);
        if let Some(nes) = &self.live {
            let r = nes.query(selector);
            if self.record
                && let OriginalRuntime::Arcade(a) = nes
            {
                let f = nes.frame();
                let ram = parsed.as_ref().ok().is_some_and(arcade_record);
                let mut t = self.tape.borrow_mut();
                if ram {
                    t.ram
                        .entry(f)
                        .or_insert_with(|| pack(&a.tapper().ram_record()));
                } else {
                    t.other
                        .entry(f)
                        .or_default()
                        .insert(selector.to_owned(), to_answer(&r));
                }
                return r;
            }
            if self.record && nes.nes().is_some() {
                let f = nes.frame();
                let ram = parsed
                    .as_ref()
                    .ok()
                    .is_some_and(|s| gp_nes::protocol::read_ram(&[], s).is_some());
                // What `ram[...]` reads: at the frame's end, or as the CPU
                // takes the next NMI.
                let kept = ram && !self.tape.borrow().ram.contains_key(&f);
                let bytes = kept.then(|| nes.query("ram[0..0x800]"));
                if let Some(read) = &bytes
                    && !read.as_ref().is_ok_and(|all| all.len() == 0x800)
                {
                    self.missed.set(true);
                    return r;
                }
                let mut t = self.tape.borrow_mut();
                if ram {
                    if let Some(Ok(all)) = bytes {
                        let b: Vec<u8> = all
                            .iter()
                            .map(|x| x.value.as_int().unwrap_or(0) as u8)
                            .collect();
                        t.ram.insert(f, pack(&b));
                    }
                } else {
                    t.other
                        .entry(f)
                        .or_default()
                        .insert(selector.to_owned(), to_answer(&r));
                }
            }
            return r;
        }
        let t = self.tape.borrow();
        if t.arcade
            && let Ok(sel) = &parsed
            && arcade_record(sel)
        {
            let Some(rec) = t.ram.get(&self.frame).and_then(|b| unpack(b)) else {
                drop(t);
                return self.miss(&format!("RAM at frame {}", self.frame));
            };
            return gp_arcade::protocol::read_record(&rec, sel)
                .unwrap_or_else(|| self.miss("RAM read"));
        }
        if !t.arcade
            && let Ok(sel) = &parsed
            && gp_nes::protocol::read_ram(&[], sel).is_some()
        {
            if t.read_at_nmi != self.at_nmi {
                drop(t);
                return self.miss("RAM read the same way");
            }
            let cached = self
                .ram_at
                .borrow()
                .as_ref()
                .filter(|(f, _)| *f == self.frame)
                .map(|(_, r)| Rc::clone(r));
            let ram = match cached {
                Some(r) => r,
                None => {
                    let Some(r) = t.ram.get(&self.frame).and_then(|b| unpack(b)) else {
                        drop(t);
                        return self.miss(&format!("RAM at frame {}", self.frame));
                    };
                    let r = Rc::new(r);
                    *self.ram_at.borrow_mut() = Some((self.frame, Rc::clone(&r)));
                    r
                }
            };
            return gp_nes::protocol::read_ram(&ram, sel).unwrap_or_else(|| self.miss("RAM read"));
        }
        let got = t
            .other
            .get(&self.frame)
            .and_then(|m| m.get(selector))
            .map(from_answer);
        drop(t);
        got.unwrap_or_else(|| self.miss(&format!("{selector} at frame {}", self.frame)))
    }

    fn sample(
        &mut self,
        selectors: &[String],
        inputs: &[InputFrame],
        frames: u32,
    ) -> Result<Vec<Vec<Entry>>, String> {
        self.unkept();
        match &mut self.live {
            Some(nes) => nes.sample(selectors, inputs, frames),
            None => self.miss("sample"),
        }
    }

    fn entities_at(&self, area: Rect) -> Vec<u64> {
        self.live
            .as_ref()
            .map_or_else(Vec::new, |nes| nes.entities_at(area))
    }

    fn patch(&mut self, model_diff: &str) -> PatchResult {
        match &mut self.live {
            Some(nes) => nes.patch(model_diff),
            None => PatchResult::Rejected("the original game has no model to patch".into()),
        }
    }

    fn set(&mut self, p: &str, v: Value) -> Result<(), String> {
        // `read_at_nmi` is kept: the RAM a tape holds is what the reads
        // saw, so a replay must read the same way.
        if p == "read_at_nmi"
            && let Value::Flag(on) = v
        {
            if let Some(nes) = &mut self.live {
                nes.set(p, v)?;
                if self.record {
                    let mut t = self.tape.borrow_mut();
                    if !t.ram.is_empty() && t.read_at_nmi != on {
                        drop(t);
                        self.missed.set(true);
                    } else {
                        t.read_at_nmi = on;
                    }
                }
            }
            self.at_nmi = on;
            return Ok(());
        }
        self.unkept();
        match &mut self.live {
            Some(nes) => nes.set(p, v),
            None => self.miss("set"),
        }
    }

    fn explain(&self, p: &str) -> Vec<String> {
        self.live
            .as_ref()
            .map_or_else(Vec::new, |nes| nes.explain(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_evidence::manifest::PowerOn;

    fn nes() -> OriginalRuntime {
        OriginalRuntime::Nes(
            gp_nes::protocol::NesRuntime::load(&gp_nes::testrom::build(), PowerOn::default())
                .unwrap(),
        )
    }

    /// Steps 40 frames with a press now and then, reading all of RAM on
    /// each: the answers.
    fn play(o: &mut Original) -> Vec<Vec<Entry>> {
        let mut out = Vec::new();
        for k in 0..40u8 {
            o.step(&[InputFrame::pads([
                if k % 7 < 3 { 0x80 } else { 0 },
                0,
                0,
                0,
            ])])
            .unwrap();
            out.push(o.query("ram[0..0x800]").unwrap());
        }
        out
    }

    /// A tape recorded with `read_at_nmi` keeps the RAM those reads saw:
    /// its replay, read the same way, answers as the emulator did, and a
    /// replay read the other way misses rather than answer wrong.
    #[test]
    fn a_tape_read_at_the_nmi_replays_the_same_reads_and_only_that_way() {
        for at_nmi in [false, true] {
            let mut live = Original::recording(nes());
            live.set("read_at_nmi", Value::Flag(at_nmi)).unwrap();
            let seen = play(&mut live);
            assert!(!live.missed());
            let tape = live.into_tape().unwrap();
            let tape = Tape::from_bytes(&tape.to_bytes()).unwrap();
            assert_eq!(tape.read_at_nmi, at_nmi);

            let mut again = Original::replay(Tape::from_bytes(&tape.to_bytes()).unwrap());
            again.set("read_at_nmi", Value::Flag(at_nmi)).unwrap();
            assert_eq!(play(&mut again), seen);
            assert!(!again.missed());

            let mut other = Original::replay(tape);
            other.set("read_at_nmi", Value::Flag(!at_nmi)).unwrap();
            other.step(&[InputFrame::pads([0; 4])]).unwrap();
            assert!(other.query("ram[0..0x800]").is_err());
            assert!(other.missed());
        }
        // The two ways read differently on the test ROM, so the check
        // above is not empty.
        let mut a = Original::recording(nes());
        let mut b = Original::recording(nes());
        b.set("read_at_nmi", Value::Flag(true)).unwrap();
        assert_ne!(play(&mut a), play(&mut b));
    }

    /// An arcade original's tape: its RAM record answers the RAM
    /// selectors, the other selectors are kept as asked, and its pictures
    /// come back as they were drawn, sprite mask included. Needs the
    /// owner's rbtapper romset; skipped without it.
    #[test]
    fn an_arcade_tape_replays_its_reads_and_pictures() {
        let Some(zip) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let rom = std::fs::read(zip).unwrap();
        let o = OriginalRuntime::of_rom(&rom).unwrap();
        let sels = [
            "ram[0xe000..0xe800]",
            "ram[0xe800..0xea00]",
            "sound_ram[*]",
            "palette[0..4]",
            "ports[*]",
            "ram[0x100]",
        ];
        let run = |o: &mut Original| {
            let mut out = Vec::new();
            for k in 0..120u32 {
                let b = if (60..66).contains(&k) { 1 } else { 0 };
                o.step(&[InputFrame::pads([b, 0, 0, 0])]).unwrap();
                for s in sels {
                    out.push(format!("{:?}", o.query(s)));
                }
                if k % 30 == 0 {
                    let f = o.picture();
                    out.push(format!(
                        "{:?} {:?}",
                        blake3::hash(&f.rgb.concat()),
                        f.sprite.iter().filter(|x| **x).count()
                    ));
                }
            }
            out
        };
        let mut live = Original::recording(o);
        let seen = run(&mut live);
        assert!(!live.missed());
        let tape = Tape::from_bytes(&live.into_tape().unwrap().to_bytes()).unwrap();
        assert!(tape.arcade);
        let mut again = Original::replay(tape);
        assert_eq!(run(&mut again), seen);
        assert!(!again.missed());
    }
}
