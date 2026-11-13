//! `gp dataset grade`: grades a ROM dataset (docs/formats/rom-dataset.md)
//! against its game's key (docs/formats/rom-dataset-key.md) and prints the
//! stage 0 scorecard's column for that ROM.
//!
//! The coverage rows' denominators come from running the ROM here, with
//! stage 0's own starts through gp-events (as `gp reach` runs them), never
//! from the dataset's rows. The keyed rows match the dataset's elements to
//! the key's by shared RAM, never by name. `--counts` prints counts only:
//! nothing of the key (no address, value, name or path) reaches its output.
//!
//! On an arcade romset (`gp dataset rom` reads the main Z80's code map, in
//! crate::arcade_stage0) the coverage rows run the romset with stage 0's
//! arcade starts on gp-arcade's levels: the main CPU's routines entered,
//! the work RAM bytes (E000-E7FF) whose value changed, and the `cp n` and
//! `ld r,n` instructions run, named as the arcade dataset names them
//! (`5FAC`). The dataset is found by its romset identity. A key's ROM
//! check reads 6502 code, so a key for a romset is refused for now; no
//! arcade game has a key yet.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use gp_events::NO_ROM;
use gp_events::names::Namer;
use gp_events::store::Store as EventStore;
use gp_evidence::Paths;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::rom_dataset::{Cell, Dataset, Row};

/// The key format this grader reads.
pub(crate) const KEY_FORMAT: &str = "0.1.0";

/// The key's file in a key folder.
pub(crate) const KEY_FILE: &str = "key.toml";

// ---------------------------------------------------------------------------
// The key as written.

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub(crate) enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T: Clone> OneOrMany<T> {
    fn list(&self) -> Vec<T> {
        match self {
            OneOrMany::One(x) => vec![x.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub(crate) enum Number {
    N(f64),
    S(String),
}

impl Number {
    fn value(&self) -> Option<f64> {
        match self {
            Number::N(x) => Some(*x),
            Number::S(s) => s.trim().parse().ok(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Fields for people, kept so a key reads strictly.
pub(crate) struct KeyHead {
    pub format: String,
    pub game: String,
    pub rom: String,
    #[serde(default)]
    pub rom_file: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mapping: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Fields for people, kept so a key reads strictly.
pub(crate) struct KElement {
    pub name: String,
    pub kind: Option<String>,
    pub slots: Option<i64>,
    pub model: Option<String>,
    pub class: String,
    pub note: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Fields for people, kept so a key reads strictly.
pub(crate) struct KRam {
    pub addr: u32,
    pub len: Option<u32>,
    pub slots: Option<u32>,
    pub slot: Option<i64>,
    pub element: Option<String>,
    pub field: Option<String>,
    pub quantity: Option<OneOrMany<String>>,
    pub part: Option<OneOrMany<String>>,
    pub role: Option<OneOrMany<String>>,
    pub class: String,
    pub src: Option<String>,
    pub note: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Fields for people, kept so a key reads strictly.
pub(crate) struct KConst {
    pub path: String,
    pub value: Option<Number>,
    pub number: Option<f64>,
    pub raw: Option<i64>,
    pub format: Option<String>,
    pub quantity: Option<String>,
    pub mechanic: Option<String>,
    pub parameter: Option<String>,
    pub element: Option<String>,
    #[serde(default)]
    pub pc: Vec<String>,
    pub entry: Option<i64>,
    #[serde(default)]
    pub part_pc: Vec<String>,
    pub class: String,
    pub src: Option<String>,
    pub note: Option<String>,
}

impl KConst {
    /// What a dataset's `number` should be.
    fn expected(&self) -> Option<f64> {
        self.number
            .or_else(|| self.value.as_ref().and_then(Number::value))
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Fields for people, kept so a key reads strictly.
pub(crate) struct KMachine {
    pub byte: u32,
    #[serde(default)]
    pub bytes: Vec<u32>,
    pub element: Option<String>,
    pub values: Vec<i64>,
    pub initial: Option<i64>,
    #[serde(default)]
    pub complete: bool,
    pub class: String,
    pub note: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Fields for people, kept so a key reads strictly.
pub(crate) struct KTrans {
    pub byte: u32,
    pub from: OneOrMany<i64>,
    pub to: i64,
    pub pc: Option<String>,
    pub class: String,
    pub src: Option<String>,
    pub note: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyFile {
    pub key: KeyHead,
    #[serde(default)]
    pub element: Vec<KElement>,
    #[serde(default)]
    pub ram: Vec<KRam>,
    #[serde(default)]
    pub constant: Vec<KConst>,
    #[serde(default)]
    pub machine: Vec<KMachine>,
    #[serde(default)]
    pub transition: Vec<KTrans>,
}

// ---------------------------------------------------------------------------
// The key, expanded: one fact a byte.

/// What the key says of one RAM byte.
#[derive(Debug, Clone)]
pub(crate) struct RamFact {
    pub element: Option<String>,
    pub slot: Option<i64>,
    pub field: Option<String>,
    pub quantity: Vec<String>,
    pub part: Vec<String>,
    pub role: Vec<String>,
    pub class: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Key {
    pub head: KeyHead,
    pub elements: Vec<KElement>,
    pub ram: BTreeMap<u32, RamFact>,
    pub constants: Vec<KConst>,
    pub machines: Vec<KMachine>,
    /// Each machine byte (every byte of a slot array) to its machine's.
    pub machine_of: BTreeMap<u32, u32>,
    /// (byte, from, to), changes to the same value left out.
    pub transitions: BTreeSet<(u32, i64, i64)>,
    pub trans_class: BTreeMap<(u32, i64, i64), String>,
}

const CLASSES: [&str; 3] = ["rom", "trace", "name"];
const ROLES: [&str; 8] = [
    "state", "counter", "timer", "pointer", "input", "sound", "scratch", "level",
];
const QUANTITIES: [&str; 14] = [
    "x", "y", "speed_x", "speed_y", "state", "timer", "facing", "kind", "flag", "count", "pointer",
    "buttons", "cell", "other",
];
const PARTS: [&str; 4] = ["whole", "lo", "hi", "fraction"];
const KINDS: [&str; 9] = [
    "player", "slots", "camera", "level", "hud", "game", "sound", "input", "other",
];
const CQUANTITIES: [&str; 15] = [
    "gravity",
    "jump_speed",
    "max_speed",
    "accel",
    "decel",
    "threshold",
    "period",
    "duration",
    "count",
    "offset",
    "size",
    "limit",
    "kind",
    "mask",
    "other",
];

impl Key {
    pub(crate) fn read(dir: &Path) -> Result<Key> {
        let p = if dir.is_dir() {
            dir.join(KEY_FILE)
        } else {
            dir.to_path_buf()
        };
        let text = fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        Key::parse(&text).with_context(|| format!("in {}", p.display()))
    }

    pub(crate) fn parse(text: &str) -> Result<Key> {
        let f: KeyFile = toml::from_str(text)?;
        if f.key.format != KEY_FORMAT {
            bail!(
                "key format {} (this grader reads {KEY_FORMAT})",
                f.key.format
            );
        }
        let class_ok = |c: &str, what: &str| -> Result<()> {
            if !CLASSES.contains(&c) {
                bail!("{what}: class {c:?} isn't rom, trace or name");
            }
            Ok(())
        };
        let in_vocab = |v: &[String], vocab: &[&str], what: &str| -> Result<()> {
            for x in v {
                if !vocab.contains(&x.as_str()) {
                    bail!("{what}: {x:?} isn't in the vocabulary");
                }
            }
            Ok(())
        };
        let names: BTreeSet<&str> = f.element.iter().map(|e| e.name.as_str()).collect();
        if names.len() != f.element.len() {
            bail!("two elements have the same name");
        }
        for e in &f.element {
            class_ok(&e.class, &e.name)?;
            if let Some(k) = &e.kind {
                in_vocab(std::slice::from_ref(k), &KINDS, &e.name)?;
            }
        }
        let el_ok = |e: &Option<String>, what: &str| -> Result<()> {
            if let Some(e) = e
                && !names.contains(e.as_str())
            {
                bail!("{what}: no element {e:?}");
            }
            Ok(())
        };
        let mut ram = BTreeMap::new();
        for r in &f.ram {
            let what = format!("ram ${:04X}", r.addr);
            class_ok(&r.class, &what)?;
            el_ok(&r.element, &what)?;
            let quantity = r.quantity.as_ref().map(OneOrMany::list).unwrap_or_default();
            let part = r.part.as_ref().map(OneOrMany::list).unwrap_or_default();
            let role = r.role.as_ref().map(OneOrMany::list).unwrap_or_default();
            in_vocab(&quantity, &QUANTITIES, &what)?;
            in_vocab(&part, &PARTS, &what)?;
            in_vocab(&role, &ROLES, &what)?;
            if r.element.is_none()
                && r.slot.is_none()
                && quantity.is_empty()
                && part.is_empty()
                && role.is_empty()
            {
                bail!("{what}: states nothing to grade");
            }
            if r.len.is_some() && r.slots.is_some() {
                bail!("{what}: len and slots both given");
            }
            let n = r.len.or(r.slots).unwrap_or(1);
            for i in 0..n {
                let a = r.addr + i;
                let fact = RamFact {
                    element: r.element.clone(),
                    slot: if r.slots.is_some() {
                        Some(i64::from(i))
                    } else {
                        r.slot
                    },
                    field: r.field.clone(),
                    quantity: quantity.clone(),
                    part: part.clone(),
                    role: role.clone(),
                    class: r.class.clone(),
                };
                if ram.insert(a, fact).is_some() {
                    bail!("ram ${a:04X} is keyed twice");
                }
            }
        }
        for c in &f.constant {
            class_ok(&c.class, &c.path)?;
            el_ok(&c.element, &c.path)?;
            if let Some(q) = &c.quantity {
                in_vocab(std::slice::from_ref(q), &CQUANTITIES, &c.path)?;
            }
            if c.class == "rom" && c.expected().is_none() {
                bail!("{}: a rom constant needs a number", c.path);
            }
            for pc in c.pc.iter().chain(&c.part_pc) {
                if gp_nes::codemap::va_parse(pc).is_none() {
                    bail!("{}: {pc:?} isn't an instruction's name", c.path);
                }
            }
        }
        for e in &f.element {
            if !ram
                .values()
                .any(|r: &RamFact| r.element.as_deref() == Some(e.name.as_str()))
            {
                bail!("element {}: no RAM names it", e.name);
            }
        }
        let mut machine_of = BTreeMap::new();
        for m in &f.machine {
            let what = format!("machine ${:04X}", m.byte);
            class_ok(&m.class, &what)?;
            el_ok(&m.element, &what)?;
            machine_of.insert(m.byte, m.byte);
            for &b in &m.bytes {
                machine_of.insert(b, m.byte);
            }
        }
        let mut transitions = BTreeSet::new();
        let mut trans_class = BTreeMap::new();
        for t in &f.transition {
            let what = format!("transition ${:04X}", t.byte);
            class_ok(&t.class, &what)?;
            let Some(m) = f.machine.iter().find(|m| m.byte == t.byte) else {
                bail!("{what}: no machine has that byte");
            };
            for from in t.from.list() {
                for v in [from, t.to] {
                    if !m.values.contains(&v) {
                        bail!("{what}: {v} isn't one of the machine's values");
                    }
                }
                if from != t.to {
                    transitions.insert((t.byte, from, t.to));
                    trans_class.insert((t.byte, from, t.to), t.class.clone());
                }
            }
        }
        Ok(Key {
            head: f.key,
            elements: f.element,
            ram,
            constants: f.constant,
            machines: f.machine,
            machine_of,
            transitions,
            trans_class,
        })
    }

    /// Each element's RAM.
    pub(crate) fn element_ram(&self) -> BTreeMap<String, BTreeSet<u32>> {
        let mut out: BTreeMap<String, BTreeSet<u32>> = self
            .elements
            .iter()
            .map(|e| (e.name.clone(), BTreeSet::new()))
            .collect();
        for (&a, f) in &self.ram {
            if let Some(e) = &f.element {
                out.entry(e.clone()).or_default().insert(a);
            }
        }
        out
    }

    /// The mapping fields: each field's element and bytes.
    pub(crate) fn fields(&self) -> BTreeMap<String, (Option<String>, Vec<u32>)> {
        let mut out: BTreeMap<String, (Option<String>, Vec<u32>)> = BTreeMap::new();
        for (&a, f) in &self.ram {
            if let Some(field) = &f.field {
                let e = out
                    .entry(field.clone())
                    .or_insert_with(|| (f.element.clone(), Vec::new()));
                e.1.push(a);
            }
        }
        out
    }

    /// Keyed facts by class: RAM bytes, elements, constants, machines and
    /// transitions.
    pub(crate) fn classes(&self) -> BTreeMap<&'static str, usize> {
        let mut out: BTreeMap<&'static str, usize> = CLASSES.iter().map(|c| (*c, 0)).collect();
        let mut add = |c: &str| {
            if let Some(k) = CLASSES.iter().find(|k| **k == c) {
                *out.get_mut(k).expect("every class") += 1;
            }
        };
        self.ram.values().for_each(|f| add(&f.class));
        self.elements.iter().for_each(|e| add(&e.class));
        self.constants.iter().for_each(|c| add(&c.class));
        self.machines.iter().for_each(|m| add(&m.class));
        self.trans_class.values().for_each(|c| add(c));
        out
    }
}

// ---------------------------------------------------------------------------
// Reading a dataset's cells.

fn s<'a>(r: &'a Row, c: &str) -> Option<&'a str> {
    match r.get(c) {
        Some(Cell::S(Some(v))) => Some(v.as_str()),
        _ => None,
    }
}

fn i(r: &Row, c: &str) -> Option<i64> {
    match r.get(c) {
        Some(Cell::I(v)) => Some(*v),
        Some(Cell::OI(v)) => *v,
        _ => None,
    }
}

fn n(r: &Row, c: &str) -> Option<f64> {
    match r.get(c) {
        Some(Cell::N(v)) => *v,
        _ => None,
    }
}

fn b(r: &Row, c: &str) -> bool {
    matches!(r.get(c), Some(Cell::B(true)))
}

/// An `open` row's `what` as a table and key: `ram:$0507` to ("ram",
/// "$0507").
fn open_what(d: &Dataset) -> BTreeSet<(String, String)> {
    d.rows("open")
        .iter()
        .filter_map(|r| s(r, "what"))
        .filter_map(|w| w.split_once(':'))
        .map(|(t, k)| (t.to_owned(), k.to_owned()))
        .collect()
}

/// A RAM address as text (`$05`, `$0507`, `0x507`, `1287`).
fn parse_ram(s: &str) -> Option<u32> {
    let t = s.trim();
    if let Some(h) = t.strip_prefix('$').or_else(|| t.strip_prefix("0x")) {
        u32::from_str_radix(h, 16).ok()
    } else {
        t.parse().ok()
    }
}

/// Two instruction names are the same instruction: case and leading zeros
/// don't matter.
fn same_pc(a: &str, b: &str) -> bool {
    match (
        gp_nes::codemap::va_parse(&a.to_ascii_uppercase()),
        gp_nes::codemap::va_parse(&b.to_ascii_uppercase()),
    ) {
        (Some(x), Some(y)) => x == y,
        _ => a.eq_ignore_ascii_case(b),
    }
}

fn norm_pc(a: &str) -> String {
    gp_nes::codemap::va_parse(&a.to_ascii_uppercase())
        .map_or_else(|| a.to_ascii_uppercase(), gp_nes::codemap::va_hex)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

// ---------------------------------------------------------------------------
// Coverage: the ROM run here.

/// What the ROM did under stage 0's starts.
#[derive(Debug, Default, Clone)]
pub(crate) struct Ran {
    /// Routines entered, by name.
    pub routines: BTreeSet<String>,
    /// RAM ($0000-$07FF) and PRG RAM ($6000-) bytes written.
    pub ram: BTreeSet<u32>,
    /// Executed compare and load instructions with an immediate operand.
    pub immediates: BTreeSet<String>,
    pub frames: u64,
    pub seconds: f64,
}

/// `cmp`, `cpx`, `cpy`, `lda`, `ldx`, `ldy` with an immediate operand.
const IMMEDIATES: [u8; 6] = [0xC9, 0xE0, 0xC0, 0xA9, 0xA2, 0xA0];

pub(crate) fn run_rom(rom: &[u8], paths: &Paths) -> Result<Ran> {
    let t0 = Instant::now();
    let starts = crate::reach::Starts {
        runs: 4,
        seed: 1,
        start_frames: gp_explore::presses::START_FRAMES,
        no_starts: false,
    };
    let clips = starts.with(&[], paths)?;
    let store = EventStore::at(paths.home.join("work").join("events"));
    let keep = crate::reach::Keep {
        store: Some(&store),
        ..Default::default()
    };
    let r = crate::reach::run_clips(rom, &clips, &keep)?;
    let (prg, mapper) = gp_nes::harvest::ines_prg(rom).context("not an iNES ROM")?;
    let mut board = gp_nes::codemap::Board::of(prg, mapper);
    board.switched = Namer::observed_switched(&board, &r.levels.footprint.windows);
    let namer = Namer::of_board(board);
    let f = &r.levels.footprint;
    let mut out = Ran {
        frames: r.clips.iter().map(|c| c.2).sum(),
        ..Default::default()
    };
    for e in &r.levels.edges {
        if e.kind.enters() && e.to != NO_ROM {
            out.routines.insert(namer.name(e.to, e.stat.to_cpu));
        }
    }
    // A byte counts once its value changes: a reset that clears all of
    // RAM writes every byte, and most keep the value they had.
    let changed = changed_ram(rom, &clips)?;
    for k in 0..f.ram_written.len() {
        if f.ram_written.contains(k) && changed.contains(&(k as u32)) {
            out.ram.insert(k as u32);
        }
    }
    for k in 0..f.prg_ram_written.len() {
        if f.prg_ram_written.contains(k) && changed.contains(&(0x6000 + k as u32)) {
            out.ram.insert(0x6000 + k as u32);
        }
    }
    for (off, op) in prg.iter().enumerate().take(f.starts.len()) {
        if !f.starts.contains(off) || !IMMEDIATES.contains(op) {
            continue;
        }
        let page = f.windows.get(off / 0x400).copied().unwrap_or(0);
        let cpus: Vec<u16> = (0..64u32)
            .filter(|cp| page & (1u64 << cp) != 0)
            .map(|cp| (cp * 0x400 + (off as u32 % 0x400)) as u16)
            .collect();
        let name = cpus
            .iter()
            .find(|&&c| namer.va(off as u32, c).is_some())
            .or(cpus.first())
            .map(|&c| namer.name(off as u32, c));
        if let Some(name) = name {
            out.immediates.insert(name);
        }
    }
    out.seconds = t0.elapsed().as_secs_f64();
    Ok(out)
}

/// [`run_rom`] on an arcade romset: stage 0's arcade starts
/// (crate::arcade_stage0's), each from power-on on gp-arcade with its
/// levels watching. Routines are the main Z80's code map entries a run
/// started, RAM the work RAM bytes whose value changed from one frame to
/// the next, and immediates the `cp n` and `ld r,n` instructions run.
pub(crate) fn run_romset(set: &gp_arcade::Romset, paths: &Paths) -> Result<Ran> {
    use crate::arcade_stage0::{RAM_BASE, RAM_END};
    /// `ld b..a,n` and `cp n`.
    const Z80_IMMEDIATES: [u8; 8] = [0x06, 0x0E, 0x16, 0x1E, 0x26, 0x2E, 0x3E, 0xFE];
    let t0 = Instant::now();
    let starts = crate::reach::Starts {
        runs: 4,
        seed: 1,
        start_frames: gp_explore::arcade::START_FRAMES,
        no_starts: false,
    };
    let clips = starts.with_platform(&[], paths, true)?;
    let mut merged = gp_arcade::levels::Levels::default();
    let mut changed: BTreeSet<u32> = BTreeSet::new();
    let mut frames = 0u64;
    for c in &clips {
        let mut t = gp_arcade::Tapper::power_on(set);
        let mut w = gp_arcade::levels::Watcher::new(false);
        let ram =
            |t: &gp_arcade::Tapper| -> Vec<u8> { (RAM_BASE..RAM_END).map(|a| t.peek(a)).collect() };
        let mut before = ram(&t);
        for f in &c.inputs {
            let bits = f.buttons_word();
            if bits > 0xff || f.reset {
                bail!(
                    "clip {}: an arcade input frame holds bits 0 to 7 and no reset",
                    c.id
                );
            }
            w.step_quiet(&mut t, gp_arcade::Controls::from_bits(bits));
            let now = ram(&t);
            for (k, (x, y)) in before.iter().zip(&now).enumerate() {
                if x != y {
                    changed.insert(u32::from(RAM_BASE) + k as u32);
                }
            }
            before = now;
            frames += 1;
        }
        crate::code_z80::merge(&mut merged, w.finish());
    }
    let map = crate::arcade_stage0::code_map(set, Some(&merged));
    let lv = &merged.cpus[0];
    let name = |a: u16| format!("{a:04X}");
    let mut out = Ran {
        frames,
        ..Default::default()
    };
    for at in map.routines.keys() {
        if lv.starts.contains_key(at) {
            out.routines.insert(name(*at));
        }
    }
    for (&a, _) in lv.written.range(RAM_BASE..RAM_END) {
        if changed.contains(&u32::from(a)) {
            out.ram.insert(u32::from(a));
        }
    }
    let (prog, end) = crate::code_z80::program(set, 0);
    for pc in lv.starts.keys() {
        if u32::from(*pc) < end
            && prog
                .get(usize::from(*pc))
                .is_some_and(|op| Z80_IMMEDIATES.contains(op))
        {
            out.immediates.insert(name(*pc));
        }
    }
    out.seconds = t0.elapsed().as_secs_f64();
    Ok(out)
}

/// The RAM bytes whose value changes from one frame to the next in any of
/// the runs, each run on a thread of its own: internal RAM every frame, and
/// PRG RAM ($6000-$7FFF, read without side effects) every 15 frames.
fn changed_ram(rom: &[u8], clips: &[crate::reach::ClipInput]) -> Result<BTreeSet<u32>> {
    const PRG_RAM_EVERY: usize = 15;
    let sets: Vec<Result<BTreeSet<u32>>> = std::thread::scope(|sc| {
        let hs: Vec<_> = clips
            .iter()
            .map(|c| {
                sc.spawn(move || -> Result<BTreeSet<u32>> {
                    let mut nes = gp_nes::Nes::power_on(rom, c.power_on)?;
                    let prg_ram = |nes: &gp_nes::Nes| -> Vec<u8> {
                        (0x6000..=0x7FFFu16).map(|a| nes.peek(a)).collect()
                    };
                    let mut out = BTreeSet::new();
                    let mut ram = nes.wram()[..gp_events::RAM_BYTES].to_vec();
                    let mut pram = prg_ram(&nes);
                    for (k, f) in c.inputs.iter().enumerate() {
                        let mut f = *f;
                        f.polled = true;
                        nes.step(f)?;
                        let now = &nes.wram()[..gp_events::RAM_BYTES];
                        for (a, (x, y)) in ram.iter().zip(now).enumerate() {
                            if x != y {
                                out.insert(a as u32);
                            }
                        }
                        ram.copy_from_slice(now);
                        if k % PRG_RAM_EVERY == PRG_RAM_EVERY - 1 {
                            let now = prg_ram(&nes);
                            for (a, (x, y)) in pram.iter().zip(&now).enumerate() {
                                if x != y {
                                    out.insert(0x6000 + a as u32);
                                }
                            }
                            pram = now;
                        }
                    }
                    Ok(out)
                })
            })
            .collect();
        hs.into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("a run panicked")))
            })
            .collect()
    });
    let mut out = BTreeSet::new();
    for s in sets {
        out.extend(s?);
    }
    Ok(out)
}

/// One coverage row: placed (a row that says it), open (an open row), of
/// all.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cover {
    pub placed: usize,
    pub open: usize,
    pub of: usize,
}

pub(crate) struct Coverage {
    pub routines: Cover,
    pub ram: Cover,
    pub constants: Cover,
}

pub(crate) fn coverage(d: &Dataset, ran: &Ran) -> Coverage {
    let open = open_what(d);
    let open_keys = |table: &str| -> BTreeSet<String> {
        open.iter()
            .filter(|(t, _)| t == table)
            .map(|(_, k)| k.clone())
            .collect()
    };
    // Routines.
    let placed_r: BTreeSet<String> = d
        .rows("routines")
        .iter()
        .filter(|r| s(r, "element").is_some())
        .filter_map(|r| s(r, "routine").map(norm_pc))
        .collect();
    let open_r: BTreeSet<String> = open_keys("routines").iter().map(|k| norm_pc(k)).collect();
    let mut routines = Cover {
        of: ran.routines.len(),
        ..Default::default()
    };
    for name in &ran.routines {
        let k = norm_pc(name);
        if placed_r.contains(&k) {
            routines.placed += 1;
        } else if open_r.contains(&k) {
            routines.open += 1;
        }
    }
    // RAM.
    let placed_m: BTreeSet<u32> = d
        .rows("ram")
        .iter()
        .filter(|r| s(r, "role").is_some())
        .filter_map(|r| i(r, "addr").map(|a| a as u32))
        .collect();
    let open_m: BTreeSet<u32> = open_keys("ram")
        .iter()
        .filter_map(|k| parse_ram(k))
        .collect();
    let mut ram = Cover {
        of: ran.ram.len(),
        ..Default::default()
    };
    for a in &ran.ram {
        if placed_m.contains(a) {
            ram.placed += 1;
        } else if open_m.contains(a) {
            ram.open += 1;
        }
    }
    // Constants.
    let placed_c: BTreeSet<String> = d
        .rows("constants")
        .iter()
        .filter_map(|r| s(r, "pc").map(norm_pc))
        .collect();
    let open_c: BTreeSet<String> = open_keys("constants").iter().map(|k| norm_pc(k)).collect();
    let mut constants = Cover {
        of: ran.immediates.len(),
        ..Default::default()
    };
    for name in &ran.immediates {
        let k = norm_pc(name);
        if placed_c.contains(&k) {
            constants.placed += 1;
        } else if open_c.contains(&k) {
            constants.open += 1;
        }
    }
    Coverage {
        routines,
        ram,
        constants,
    }
}

// ---------------------------------------------------------------------------
// Keyed rows.

/// How one keyed fact came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Mark {
    Right,
    Wrong,
    Open,
    Missing,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tally {
    pub right: usize,
    pub wrong: usize,
    pub open: usize,
    pub missing: usize,
}

impl Tally {
    fn add(&mut self, m: Mark) {
        match m {
            Mark::Right => self.right += 1,
            Mark::Wrong => self.wrong += 1,
            Mark::Open => self.open += 1,
            Mark::Missing => self.missing += 1,
        }
    }
    pub(crate) fn of(&self) -> usize {
        self.right + self.wrong + self.open + self.missing
    }
}

/// The `estimates` table against the key's numbers.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Estimates {
    pub close: usize,
    pub far: usize,
    pub unplaced: usize,
}

/// Whether an estimate is close to a key number: within 10% of it, within
/// 2 of a number of frames, or within the row's own spread if that is
/// larger.
fn estimate_close(x: f64, want: f64, frames: bool, spread: Option<f64>) -> bool {
    let mut tol = 0.1 * want.abs();
    if frames {
        tol = tol.max(2.0);
    }
    if let Some(sp) = spread {
        tol = tol.max(sp.abs());
    }
    (x - want).abs() <= tol + 1e-9
}

/// Wrong over every row stated, for one table.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stated {
    pub stated: usize,
    pub wrong: usize,
    pub unplaced: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Keyed {
    pub fields: Tally,
    pub elements_found: usize,
    pub elements_of: usize,
    pub numbers: Tally,
    pub transitions_found: usize,
    pub transitions_of: usize,
    pub transitions_extra: usize,
    /// Per table: ram, elements, constants, states, transitions.
    pub stated: BTreeMap<&'static str, Stated>,
    /// Rows of tables no key states (routines, branches, ...).
    pub not_keyed: BTreeMap<&'static str, usize>,
    /// Per `table.column`: rows graded on it and rows wrong on it. Only
    /// rows that state the column (non-null) and land on a fact that
    /// states it are graded on it.
    pub columns: BTreeMap<String, (usize, usize)>,
    /// Estimates (measured numbers, never graded wrong): close to a key
    /// number, far from every one, and those on nothing the key states.
    pub estimates: Estimates,
    /// What went wrong or is missing, for the full report (never printed
    /// with `--counts`).
    pub details: Vec<String>,
}

impl Keyed {
    pub(crate) fn wrong_total(&self) -> Stated {
        let mut t = Stated::default();
        for s in self.stated.values() {
            t.stated += s.stated;
            t.wrong += s.wrong;
            t.unplaced += s.unplaced;
        }
        t
    }
}

/// The dataset's elements matched to the key's by shared RAM, largest
/// overlap first, one to one: key element -> dataset element.
pub(crate) fn match_elements(key: &Key, d: &Dataset) -> BTreeMap<String, String> {
    let mut ds: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for r in d.rows("ram") {
        if let (Some(e), Some(a)) = (s(r, "element"), i(r, "addr")) {
            ds.entry(e.to_owned()).or_default().insert(a as u32);
        }
    }
    let kr = key.element_ram();
    let mut pairs = Vec::new();
    for (ke, ks) in &kr {
        for (de, dset) in &ds {
            let shared = ks.intersection(dset).count();
            if shared > 0 {
                pairs.push((std::cmp::Reverse(shared), ke.clone(), de.clone()));
            }
        }
    }
    pairs.sort();
    let mut out = BTreeMap::new();
    let mut taken = BTreeSet::new();
    for (_, ke, de) in pairs {
        if out.contains_key(&ke) || taken.contains(&de) {
            continue;
        }
        taken.insert(de.clone());
        out.insert(ke, de);
    }
    out
}

/// Counts each graded column of one row: `seen` were graded, `wrong` of
/// them were wrong.
fn tick(k: &mut Keyed, table: &str, seen: &[&str], wrong: &[&str]) {
    for c in seen {
        let e = k.columns.entry(format!("{table}.{c}")).or_default();
        e.0 += 1;
        if wrong.contains(c) {
            e.1 += 1;
        }
    }
}

pub(crate) fn grade_keyed(key: &Key, d: &Dataset) -> Keyed {
    let mut k = Keyed::default();
    let matched = match_elements(key, d);
    let back: BTreeMap<&str, &str> = matched
        .iter()
        .map(|(a, b)| (b.as_str(), a.as_str()))
        .collect();
    let open = open_what(d);
    let open_ram: BTreeSet<u32> = open
        .iter()
        .filter(|(t, _)| t == "ram")
        .filter_map(|(_, w)| parse_ram(w))
        .collect();
    let open_const: BTreeSet<String> = open
        .iter()
        .filter(|(t, _)| t == "constants")
        .map(|(_, w)| norm_pc(w))
        .collect();
    let ram_rows: BTreeMap<u32, &Row> = d
        .rows("ram")
        .iter()
        .filter_map(|r| i(r, "addr").map(|a| (a as u32, r)))
        .collect();

    // Mapping fields.
    for (field, (el, bytes)) in key.fields() {
        let want = el.as_ref().and_then(|e| matched.get(e));
        let marks: Vec<Mark> = bytes
            .iter()
            .map(|a| {
                match (
                    ram_rows.get(a).and_then(|r| s(r, "element")),
                    ram_rows.get(a),
                ) {
                    (Some(e), _) if Some(&e.to_owned()) == want => Mark::Right,
                    (Some(_), _) => Mark::Wrong,
                    (None, Some(_)) => Mark::Open,
                    (None, None) if open_ram.contains(a) => Mark::Open,
                    (None, None) => Mark::Missing,
                }
            })
            .collect();
        let m = if marks.contains(&Mark::Wrong) {
            Mark::Wrong
        } else if marks.iter().all(|m| *m == Mark::Right) {
            Mark::Right
        } else if marks.iter().all(|m| *m == Mark::Missing) {
            Mark::Missing
        } else {
            Mark::Open
        };
        if m != Mark::Right {
            k.details.push(format!("field {field}: {m:?}"));
        }
        k.fields.add(m);
    }

    // Elements.
    k.elements_of = key.elements.len();
    k.elements_found = matched.len();
    for e in &key.elements {
        if !matched.contains_key(&e.name) {
            k.details.push(format!("element {}: not found", e.name));
        }
    }

    // Numbers, over the key's rom constants.
    let consts = d.rows("constants");
    let at = |r: &Row, pcs: &[String], entry: i64| -> bool {
        s(r, "pc").is_some_and(|p| pcs.iter().any(|q| same_pc(p, q)))
            && i(r, "entry").unwrap_or(0) == entry
    };
    let by_role = |r: &Row, c: &KConst| -> bool {
        c.pc.is_empty()
            && c.mechanic.is_some()
            && c.parameter.is_some()
            && s(r, "mechanic") == c.mechanic.as_deref()
            && s(r, "parameter") == c.parameter.as_deref()
            && c.element
                .as_ref()
                .and_then(|e| matched.get(e))
                .map(String::as_str)
                == s(r, "element")
    };
    // The key constants a row lands on: all of them at its pc (one load
    // can be two model facts), else those its role names.
    let lands = |r: &Row| -> Vec<&KConst> {
        let entry = i(r, "entry").unwrap_or(0);
        let mut v: Vec<&KConst> = key
            .constants
            .iter()
            .filter(|c| at(r, &c.pc, c.entry.unwrap_or(0)))
            .collect();
        if v.is_empty() {
            v = key.constants.iter().filter(|c| by_role(r, c)).collect();
        }
        if v.is_empty() && entry == 0 {
            v = key
                .constants
                .iter()
                .filter(|c| s(r, "pc").is_some_and(|p| c.part_pc.iter().any(|q| same_pc(p, q))))
                .collect();
        }
        v
    };
    let whole = |r: &Row, c: &KConst| at(r, &c.pc, c.entry.unwrap_or(0)) || by_role(r, c);
    for c in key.constants.iter().filter(|c| c.class == "rom") {
        let want = c.expected().expect("checked on reading");
        let mut marks = Vec::new();
        for r in consts.iter().filter(|r| whole(r, c)) {
            match n(r, "number") {
                Some(x) if close(x, want) => marks.push(Mark::Right),
                // Another fact's number at the same place is that fact's.
                Some(x)
                    if lands(r)
                        .iter()
                        .any(|o| whole(r, o) && o.expected().is_some_and(|w| close(x, w))) => {}
                Some(_) => marks.push(Mark::Wrong),
                None => marks.push(Mark::Open),
            }
        }
        let m = if marks.contains(&Mark::Wrong) {
            Mark::Wrong
        } else if marks.contains(&Mark::Right) {
            Mark::Right
        } else if !marks.is_empty() || c.pc.iter().any(|p| open_const.contains(&norm_pc(p))) {
            Mark::Open
        } else {
            Mark::Missing
        };
        if m != Mark::Right {
            k.details.push(format!("number {}: {m:?}", c.path));
        }
        k.numbers.add(m);
    }

    // States and transitions, as (byte, value).
    let mut state_of: BTreeMap<(String, String), (u32, i64)> = BTreeMap::new();
    for r in d.rows("states") {
        if let (Some(m), Some(st), Some(byte), Some(v)) =
            (s(r, "machine"), s(r, "state"), i(r, "byte"), i(r, "value"))
        {
            state_of.insert((m.to_owned(), st.to_owned()), (byte as u32, v));
        }
    }
    let mut ds_trans: BTreeSet<(u32, i64, i64)> = BTreeSet::new();
    let mut st_t = Stated::default();
    for r in d.rows("transitions") {
        let m = s(r, "machine").unwrap_or_default().to_owned();
        let from = s(r, "from_state").and_then(|x| state_of.get(&(m.clone(), x.to_owned())));
        let to = s(r, "to_state").and_then(|x| state_of.get(&(m.clone(), x.to_owned())));
        let (Some(&(fb, fv)), Some(&(tb, tv))) = (from, to) else {
            st_t.unplaced += 1;
            continue;
        };
        let (Some(&mf), Some(&mt)) = (key.machine_of.get(&fb), key.machine_of.get(&tb)) else {
            st_t.unplaced += 1;
            continue;
        };
        if mf != mt || fv == tv {
            st_t.unplaced += 1;
            continue;
        }
        let t = (mf, fv, tv);
        let complete = key.machines.iter().any(|x| x.byte == mf && x.complete);
        if key.transitions.contains(&t) {
            st_t.stated += 1;
            tick(&mut k, "transitions", &["change"], &[]);
        } else if complete {
            st_t.stated += 1;
            st_t.wrong += 1;
            tick(&mut k, "transitions", &["change"], &["change"]);
            k.details
                .push(format!("transition ${mf:04X} {fv} -> {tv}: not in the key"));
        } else {
            st_t.unplaced += 1;
        }
        ds_trans.insert(t);
    }
    k.transitions_of = key.transitions.len();
    k.transitions_found = key.transitions.intersection(&ds_trans).count();
    k.transitions_extra = ds_trans.difference(&key.transitions).count();
    for t in key.transitions.difference(&ds_trans) {
        k.details.push(format!(
            "transition ${:04X} {} -> {}: missing",
            t.0, t.1, t.2
        ));
    }

    // Wrong over every row stated.
    let mut st_ram = Stated::default();
    for r in d.rows("ram") {
        let Some(a) = i(r, "addr") else {
            st_ram.unplaced += 1;
            continue;
        };
        let Some(f) = key.ram.get(&(a as u32)) else {
            st_ram.unplaced += 1;
            continue;
        };
        let mut seen = Vec::new();
        let mut wrong = Vec::new();
        if let (Some(ke), Some(de)) = (&f.element, s(r, "element")) {
            seen.push("element");
            if matched.get(ke).map(String::as_str) != Some(de) {
                wrong.push("element");
            }
        }
        if let (Some(ks), Some(ds)) = (f.slot, i(r, "slot")) {
            seen.push("slot");
            if ks != ds {
                wrong.push("slot");
            }
        }
        for (col, vals) in [
            ("quantity", &f.quantity),
            ("part", &f.part),
            ("role", &f.role),
        ] {
            if let (false, Some(v)) = (vals.is_empty(), s(r, col)) {
                seen.push(col);
                if !vals.iter().any(|x| x == v) {
                    wrong.push(col);
                }
            }
        }
        tick(&mut k, "ram", &seen, &wrong);
        if seen.is_empty() {
            st_ram.unplaced += 1;
        } else {
            st_ram.stated += 1;
            if !wrong.is_empty() {
                st_ram.wrong += 1;
                k.details
                    .push(format!("ram ${a:04X}: wrong {}", wrong.join(", ")));
            }
        }
    }
    let mut st_el = Stated::default();
    for r in d.rows("elements") {
        let Some(name) = s(r, "element") else {
            st_el.unplaced += 1;
            continue;
        };
        let Some(ke) = back.get(name) else {
            st_el.unplaced += 1;
            continue;
        };
        let e = key
            .elements
            .iter()
            .find(|e| e.name == *ke)
            .expect("matched");
        st_el.stated += 1;
        let mut seen = vec!["match"];
        let mut wrong = Vec::new();
        if let (Some(kk), Some(dk)) = (&e.kind, s(r, "kind")) {
            seen.push("kind");
            if kk != dk {
                wrong.push("kind");
            }
        }
        if let (Some(ks), Some(dsl)) = (e.slots, i(r, "slots")) {
            seen.push("slots");
            if ks != dsl {
                wrong.push("slots");
            }
        }
        tick(&mut k, "elements", &seen, &wrong);
        if !wrong.is_empty() {
            st_el.wrong += 1;
            k.details
                .push(format!("element {}: wrong {}", e.name, wrong.join(", ")));
        }
    }
    let mut st_c = Stated::default();
    for r in consts {
        let cands = lands(r);
        if cands.is_empty() {
            st_c.unplaced += 1;
            continue;
        }
        // Graded against the fact it fits best.
        let grade = |c: &KConst| -> (Vec<&'static str>, Vec<&'static str>) {
            let mut graded = Vec::new();
            let mut wrong = Vec::new();
            if whole(r, c)
                && let (Some(x), Some(want)) = (n(r, "number"), c.expected())
            {
                graded.push("number");
                if !close(x, want) {
                    wrong.push("number");
                }
            }
            for (col, want) in [
                ("quantity", &c.quantity),
                ("mechanic", &c.mechanic),
                ("parameter", &c.parameter),
            ] {
                if let (Some(w), Some(v)) = (want, s(r, col)) {
                    graded.push(col);
                    if w != v {
                        wrong.push(col);
                    }
                }
            }
            if let (Some(ke), Some(de)) = (&c.element, s(r, "element")) {
                graded.push("element");
                if matched.get(ke).map(String::as_str) != Some(de) {
                    wrong.push("element");
                }
            }
            (graded, wrong)
        };
        let (c, (graded, wrong)) = cands
            .iter()
            .map(|c| (*c, grade(c)))
            .min_by_key(|(_, (_, w))| w.len())
            .expect("not empty");
        tick(&mut k, "constants", &graded, &wrong);
        if graded.is_empty() {
            st_c.unplaced += 1;
        } else {
            st_c.stated += 1;
            if !wrong.is_empty() {
                st_c.wrong += 1;
                k.details
                    .push(format!("constant {}: wrong {}", c.path, wrong.join(", ")));
            }
        }
    }
    let mut st_s = Stated::default();
    for r in d.rows("states") {
        let (Some(byte), Some(v)) = (i(r, "byte"), i(r, "value")) else {
            st_s.unplaced += 1;
            continue;
        };
        let Some(m) = key
            .machine_of
            .get(&(byte as u32))
            .and_then(|mb| key.machines.iter().find(|m| m.byte == *mb))
        else {
            st_s.unplaced += 1;
            continue;
        };
        st_s.stated += 1;
        let mut seen = vec!["value"];
        let mut wrong = Vec::new();
        if !m.values.contains(&v) {
            wrong.push("value");
        }
        if let Some(init) = m.initial
            && b(r, "initial")
        {
            seen.push("initial");
            if init != v {
                wrong.push("initial");
            }
        }
        if let (Some(ke), Some(de)) = (&m.element, s(r, "element")) {
            seen.push("element");
            if matched.get(ke).map(String::as_str) != Some(de) {
                wrong.push("element");
            }
        }
        tick(&mut k, "states", &seen, &wrong);
        if !wrong.is_empty() {
            st_s.wrong += 1;
            k.details.push(format!(
                "state ${:04X} = {v}: wrong {}",
                m.byte,
                wrong.join(", ")
            ));
        }
    }
    // Estimates: matched by element (through shared RAM), mechanic and
    // parameter. A row is close when it is close to any key number of that
    // place; none ever counts as wrong.
    for r in d.rows("estimates") {
        let cands: Vec<&KConst> = key
            .constants
            .iter()
            .filter(|c| {
                c.mechanic.is_some()
                    && c.parameter.is_some()
                    && s(r, "mechanic") == c.mechanic.as_deref()
                    && s(r, "parameter") == c.parameter.as_deref()
                    && c.element
                        .as_ref()
                        .and_then(|e| matched.get(e))
                        .map(String::as_str)
                        == s(r, "element")
            })
            .filter(|c| c.expected().is_some())
            .collect();
        let Some(x) = n(r, "value") else {
            k.estimates.unplaced += 1;
            continue;
        };
        if cands.is_empty() {
            k.estimates.unplaced += 1;
            continue;
        }
        let spread = n(r, "spread");
        let row_frames = s(r, "unit").is_some_and(|u| u.starts_with("frame"));
        let close = cands.iter().any(|c| {
            let frames = row_frames || matches!(c.quantity.as_deref(), Some("duration" | "period"));
            estimate_close(x, c.expected().expect("filtered"), frames, spread)
        });
        if close {
            k.estimates.close += 1;
        } else {
            k.estimates.far += 1;
            k.details.push(format!(
                "estimate {}: far",
                cands
                    .iter()
                    .map(|c| c.path.as_str())
                    .collect::<Vec<_>>()
                    .join(" or ")
            ));
        }
    }
    k.stated.insert("ram", st_ram);
    k.stated.insert("elements", st_el);
    k.stated.insert("constants", st_c);
    k.stated.insert("states", st_s);
    k.stated.insert("transitions", st_t);
    for t in [
        "routines",
        "branches",
        "mechanics",
        "facts",
        "open",
        "runs",
        "estimates",
    ] {
        k.not_keyed.insert(t, d.rows(t).len());
    }
    k
}

// ---------------------------------------------------------------------------
// The key checked against the ROM.

/// Opcodes with an immediate operand.
const IMMEDIATE_OPS: [u8; 11] = [
    0x09, 0x29, 0x49, 0x69, 0xA0, 0xA2, 0xA9, 0xC0, 0xC9, 0xE0, 0xE9,
];

/// Checks each constant's `pc` against the ROM: an instruction with an
/// immediate operand must hold `raw` as its operand, any other place is a
/// table whose byte at `entry` (two bytes, low first, for a `raw` over
/// 255) must be `raw`. Returns how many were checked and each that wasn't
/// (its path and pc).
pub(crate) fn check_key(key: &Key, rom: &[u8]) -> Result<(usize, Vec<String>)> {
    let (prg, mapper) = gp_nes::harvest::ines_prg(rom).context("not an iNES ROM")?;
    let board = gp_nes::codemap::Board::of(prg, mapper);
    let mut checked = 0;
    let mut bad = Vec::new();
    for c in &key.constants {
        let Some(raw) = c.raw else { continue };
        for pc in &c.pc {
            checked += 1;
            let off = gp_nes::codemap::va_parse(&pc.to_ascii_uppercase())
                .and_then(|v| board.prg_offset(v));
            let byte = |k: usize| off.and_then(|o| prg.get(o + k)).copied();
            let ok = match (byte(0), c.entry) {
                (Some(op), None) if IMMEDIATE_OPS.contains(&op) => {
                    byte(1).map(i64::from) == Some(raw & 0xFF)
                }
                (Some(_), e) => {
                    let e = usize::try_from(e.unwrap_or(0)).unwrap_or(usize::MAX / 2);
                    if raw > 0xFF {
                        match (byte(e), byte(e + 1)) {
                            (Some(lo), Some(hi)) => i64::from(lo) | (i64::from(hi) << 8) == raw,
                            _ => false,
                        }
                    } else {
                        byte(e).map(i64::from) == Some(raw)
                    }
                }
                (None, _) => false,
            };
            if !ok {
                bad.push(format!("{} at {pc}", c.path));
            }
        }
    }
    Ok((checked, bad))
}

// ---------------------------------------------------------------------------
// The command.

/// The dataset's ROM hash, from `dataset.json`.
fn dataset_rom(dir: &Path) -> Result<String> {
    let p = dir.join("dataset.json");
    let v: Value = serde_json::from_str(
        &fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?,
    )?;
    v["rom"]
        .as_str()
        .map(str::to_owned)
        .context("dataset.json has no rom")
}

/// The headerless SHA-256 of an iNES file, or a romset's identity, or
/// None.
fn headerless_sha(bytes: &[u8]) -> Option<String> {
    if gp_arcade::romset::looks_like_zip(bytes) {
        return gp_arcade::Romset::from_zip(bytes).ok().map(|s| s.identity);
    }
    gp_evidence::source::nes_banks(bytes)
        .ok()
        .map(|(img, _, _)| img.sha256)
}

fn search(dir: &Path, sha: &str, depth: usize) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in &entries {
        if p.is_file()
            && p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("nes") || e.eq_ignore_ascii_case("zip"))
            && fs::read(p).ok().and_then(|b| headerless_sha(&b)).as_deref() == Some(sha)
        {
            return Some(p.clone());
        }
    }
    if depth == 0 {
        return None;
    }
    entries
        .iter()
        .filter(|p| p.is_dir())
        .find_map(|p| search(p, sha, depth - 1))
}

/// The ROM whose headerless hash is `sha`: the key's `rom_file` under the
/// repo, $GP_ROM_DIR or $GP_HOME, else any `.nes` with that hash there.
fn find_rom(sha: &str, hint: Option<&str>, paths: &Paths) -> Result<PathBuf> {
    let rom_dir = std::env::var_os("GP_ROM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join("roms"));
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let roots = [
        PathBuf::from("."),
        repo.clone(),
        rom_dir.clone(),
        paths.home.clone(),
    ];
    if let Some(h) = hint {
        for r in &roots {
            let p = r.join(h);
            if fs::read(&p)
                .ok()
                .and_then(|b| headerless_sha(&b))
                .as_deref()
                == Some(sha)
            {
                return Ok(p);
            }
        }
    }
    for (dir, depth) in [(rom_dir, 3), (paths.home.join("work").join("sealed"), 2)] {
        if let Some(p) = search(&dir, sha, depth) {
            return Ok(p);
        }
    }
    bail!("no ROM with headerless SHA-256 (or romset identity) {sha} found; give it with --rom")
}

/// The scorecard's column for one ROM, as JSON.
pub(crate) fn scorecard(
    cov: &Coverage,
    keyed: Option<(&Key, &Keyed)>,
    ran: &Ran,
    rows: &BTreeMap<String, usize>,
) -> Value {
    let c = |x: &Cover| json!({"placed": x.placed, "open": x.open, "of": x.of});
    let mut v = json!({
        "routines": c(&cov.routines),
        "ram": c(&cov.ram),
        "constants": c(&cov.constants),
        "run": {"frames": ran.frames, "seconds": ran.seconds},
        "rows": rows,
    });
    if let Some((key, k)) = keyed {
        let t = |x: &Tally| json!({"right": x.right, "wrong": x.wrong, "open": x.open, "missing": x.missing, "of": x.of()});
        let w = k.wrong_total();
        v["keyed"] = json!({
            "facts": key.classes(),
            "mapping_fields": t(&k.fields),
            "elements": {"found": k.elements_found, "of": k.elements_of},
            "numbers": t(&k.numbers),
            "transitions": {"found": k.transitions_found, "of": k.transitions_of, "extra": k.transitions_extra},
            "wrong_over_stated": {"wrong": w.wrong, "stated": w.stated, "unplaced": w.unplaced},
            "by_table": k.stated.iter().map(|(t, s)| (t.to_string(), json!({"stated": s.stated, "wrong": s.wrong, "unplaced": s.unplaced}))).collect::<BTreeMap<_, _>>(),
            "not_keyed": k.not_keyed,
            "estimates": {"close": k.estimates.close, "far": k.estimates.far, "of": k.estimates.close + k.estimates.far, "unplaced": k.estimates.unplaced},
            "columns": k.columns.iter().map(|(c, (g, w))| (c.clone(), json!({"graded": g, "wrong": w}))).collect::<BTreeMap<_, _>>(),
        });
    }
    v
}

pub(crate) fn text(v: &Value, title: &str) -> String {
    let mut out = format!("{title}\n");
    let cov = |name: &str, x: &Value| {
        format!(
            "{name}: {} with a row, {} open, of {}\n",
            x["placed"], x["open"], x["of"]
        )
    };
    out += &cov("Executed routines with an element row", &v["routines"]);
    out += &cov("RAM bytes written with a role row", &v["ram"]);
    out += &cov(
        "Compare and load-immediate sites with a constants row",
        &v["constants"],
    );
    if let Some(k) = v.get("keyed") {
        let t = &k["mapping_fields"];
        out += &format!(
            "Key's mapping fields in the right element: right {}, wrong {} (open {}, missing {}, of {})\n",
            t["right"], t["wrong"], t["open"], t["missing"], t["of"]
        );
        out += &format!(
            "Key's elements matched: found {} of {}\n",
            k["elements"]["found"], k["elements"]["of"]
        );
        let t = &k["numbers"];
        out += &format!(
            "Key's numbers the ROM settles: right {}, wrong {}, open {}, missing {} (of {})\n",
            t["right"], t["wrong"], t["open"], t["missing"], t["of"]
        );
        out += &format!(
            "Key's transitions: found {} of {}, extra {}\n",
            k["transitions"]["found"], k["transitions"]["of"], k["transitions"]["extra"]
        );
        let e = &k["estimates"];
        out += &format!(
            "Key's numbers estimated: close {}, far {}, of {} the key has an estimate for (unplaced {})\n",
            e["close"], e["far"], e["of"], e["unplaced"]
        );
        let w = &k["wrong_over_stated"];
        out += &format!(
            "Wrong, over every row stated: {} of {} (unplaced {})\n",
            w["wrong"], w["stated"], w["unplaced"]
        );
        let mut line = String::new();
        if let Some(cols) = k["columns"].as_object() {
            for (c, x) in cols {
                line += &format!("  {c} wrong {} of {}\n", x["wrong"], x["graded"]);
            }
        }
        if let Some(tabs) = k["by_table"].as_object() {
            for (t, x) in tabs {
                line += &format!("  {t} unplaced {}\n", x["unplaced"]);
            }
        }
        if !line.is_empty() {
            out += "By table and column (rows graded on it):\n";
            out += &line;
        }
        let f = &k["facts"];
        out += &format!(
            "Keyed facts: rom {}, trace {}, name {}\n",
            f["rom"], f["trace"], f["name"]
        );
    } else {
        out += "No key: the keyed rows don't apply\n";
    }
    out += &format!(
        "Run: {} frames in {:.1} s\n",
        v["run"]["frames"],
        v["run"]["seconds"].as_f64().unwrap_or(0.0)
    );
    out
}

/// `gp dataset grade`.
pub(crate) fn cli(
    dataset: &Path,
    key: Option<&Path>,
    rom: Option<&Path>,
    counts: bool,
    paths: &Paths,
) -> Result<crate::Outcome> {
    let sha = dataset_rom(dataset)?;
    let key = key.map(Key::read).transpose()?;
    if let Some(k) = &key
        && k.head.rom != sha
    {
        bail!("the dataset is of another ROM than the key's");
    }
    let d = crate::rom_dataset::read(dataset)?;
    for r in d.rows("rom") {
        if s(r, "rom") != Some(sha.as_str()) {
            bail!("the dataset's rom table names another ROM than dataset.json");
        }
    }
    let rom_path = match rom {
        Some(p) => p.to_path_buf(),
        None => find_rom(
            &sha,
            key.as_ref().and_then(|k| k.head.rom_file.as_deref()),
            paths,
        )?,
    };
    let bytes = fs::read(&rom_path).with_context(|| format!("reading {}", rom_path.display()))?;
    if headerless_sha(&bytes).as_deref() != Some(sha.as_str()) {
        bail!("the ROM given isn't the dataset's");
    }
    let arcade = crate::original::Platform::of_file(&bytes) == crate::original::Platform::Arcade;
    if arcade && key.is_some() {
        bail!(
            "a key is checked against 6502 code; a romset's key needs a Z80 reading of its constants, which this gp hasn't got yet: grade {} without the key",
            dataset.display()
        );
    }
    let checked = key.as_ref().map(|k| check_key(k, &bytes)).transpose()?;
    let ran = if arcade {
        run_romset(&gp_arcade::Romset::from_zip(&bytes)?, paths)?
    } else {
        run_rom(&bytes, paths)?
    };
    let cov = coverage(&d, &ran);
    let keyed = key.as_ref().map(|k| grade_keyed(k, &d));
    let rows: BTreeMap<String, usize> = crate::rom_dataset::TABLES
        .iter()
        .map(|t| (t.name.to_owned(), d.rows(t.name).len()))
        .collect();
    let mut v = scorecard(&cov, key.as_ref().zip(keyed.as_ref()), &ran, &rows);
    let title = if counts {
        format!("Scorecard, counts only ({})", &sha[..12])
    } else {
        format!(
            "Scorecard: {} ({})",
            key.as_ref().map_or("no key", |k| k.head.game.as_str()),
            &sha[..12]
        )
    };
    if let Some((n, bad)) = &checked {
        v["key_check"] = json!({"checked": n, "mismatched": bad.len()});
    }
    let mut t = text(&v, &title);
    if let Some((n, bad)) = &checked {
        t += &format!(
            "Key checked against the ROM: {n} pcs, {} mismatched\n",
            bad.len()
        );
        if !counts {
            for b in bad {
                t += &format!("  key mismatch: {b}\n");
            }
        }
    }
    if !counts {
        v["rom_file"] = json!(rom_path.display().to_string());
        if let (Some(k), Some(g)) = (&key, &keyed) {
            v["game"] = json!(k.head.game);
            v["details"] = json!(g.details);
            if !g.details.is_empty() {
                t += "\nDetails:\n";
                for line in &g.details {
                    t += &format!("  {line}\n");
                }
            }
        }
    }
    Ok(crate::Outcome::ok(v, t.trim_end().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gp-cli's folder: its tests/keys hold game-printer's Moss Hop key, a
    /// key of that tool's own homebrew game, as the grader's fixture.
    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
    }

    fn keys() -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = fs::read_dir(root().join("tests/keys"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join(KEY_FILE).exists())
            .collect();
        out.sort();
        out
    }

    /// A dataset that says exactly what the key says: the oracle tool.
    /// Element names are made up, so matching has to go by RAM.
    pub(crate) fn oracle(key: &Key) -> Dataset {
        let mut d = Dataset::default();
        let rom = key.head.rom.as_str();
        let ds_name = |e: &str| {
            let k = key.elements.iter().position(|x| x.name == e).unwrap();
            format!("ds_{k}")
        };
        d.push("rom", json!({"rom": rom})).unwrap();
        for e in &key.elements {
            d.push(
                "elements",
                json!({"rom": rom, "element": ds_name(&e.name), "kind": e.kind, "slots": e.slots}),
            )
            .unwrap();
        }
        for (&a, f) in &key.ram {
            d.push(
                "ram",
                json!({"rom": rom, "addr": a, "element": f.element.as_deref().map(ds_name),
                       "slot": f.slot, "quantity": f.quantity.first(), "part": f.part.first(),
                       "role": f.role.first(), "written": true}),
            )
            .unwrap();
        }
        for c in &key.constants {
            let want = c.expected();
            for pc in &c.pc {
                d.push(
                    "constants",
                    json!({"rom": rom, "pc": pc, "entry": c.entry.unwrap_or(0), "raw": c.raw.unwrap_or(0),
                           "format": c.format, "number": want, "quantity": c.quantity,
                           "element": c.element.as_deref().map(ds_name), "mechanic": c.mechanic,
                           "parameter": c.parameter}),
                )
                .unwrap();
            }
            if c.pc.is_empty() && c.mechanic.is_some() && c.parameter.is_some() {
                d.push(
                    "constants",
                    json!({"rom": rom, "pc": format!("FF{:02X}", d.rows("constants").len() % 256), "entry": 0,
                           "number": want, "element": c.element.as_deref().map(ds_name),
                           "mechanic": c.mechanic, "parameter": c.parameter}),
                )
                .unwrap();
            }
        }
        for m in &key.machines {
            let mname = format!("state_{:02X}", m.byte);
            for v in &m.values {
                d.push(
                    "states",
                    json!({"rom": rom, "machine": mname, "byte": m.byte, "value": v,
                           "state": format!("s{v}"), "initial": m.initial == Some(*v),
                           "element": m.element.as_deref().map(ds_name)}),
                )
                .unwrap();
            }
        }
        for (b, f, t) in &key.transitions {
            d.push(
                "transitions",
                json!({"rom": rom, "machine": format!("state_{b:02X}"), "from_state": format!("s{f}"),
                       "to_state": format!("s{t}"), "pc": "C000"}),
            )
            .unwrap();
        }
        d
    }

    /// Every key whose ROM is on this machine holds the ROM's numbers at
    /// its pcs. Moss Hop's is built by `just mosshop`; the others are in
    /// $GP_ROM_DIR or $GP_HOME (the ROM tier).
    #[test]
    fn every_key_found_on_this_machine_matches_its_rom() {
        let paths = Paths::from_env().unwrap();
        for k in keys() {
            let key = Key::read(&k).unwrap();
            let Ok(p) = find_rom(&key.head.rom, key.head.rom_file.as_deref(), &paths) else {
                eprintln!("{}: no ROM here, skipped", k.display());
                continue;
            };
            let (n, bad) = check_key(&key, &fs::read(p).unwrap()).unwrap();
            assert!(bad.is_empty(), "{}: {bad:?}", k.display());
            eprintln!("{}: {n} pcs match", k.display());
        }
    }

    #[test]
    fn every_key_reads() {
        let ks = keys();
        assert!(!ks.is_empty());
        for k in ks {
            let key = Key::read(&k).unwrap_or_else(|e| panic!("{}: {e:#}", k.display()));
            assert!(!key.ram.is_empty(), "{}", k.display());
        }
    }

    #[test]
    fn the_null_tool_scores_zero_on_every_keyed_row() {
        for k in keys() {
            let key = Key::read(&k).unwrap();
            let d = Dataset::default();
            let g = grade_keyed(&key, &d);
            assert_eq!(g.fields.right, 0);
            assert_eq!(g.fields.wrong, 0);
            assert_eq!(g.elements_found, 0);
            assert_eq!(g.numbers.right, 0);
            assert_eq!(g.numbers.wrong, 0);
            assert_eq!(g.transitions_found, 0);
            assert_eq!(g.wrong_total(), Stated::default());
            let cov = coverage(
                &d,
                &Ran {
                    routines: ["C000".to_owned()].into(),
                    ram: [5].into(),
                    immediates: ["C001".to_owned()].into(),
                    ..Default::default()
                },
            );
            for c in [cov.routines, cov.ram, cov.constants] {
                assert_eq!((c.placed, c.open, c.of), (0, 0, 1));
            }
        }
    }

    #[test]
    fn the_oracle_scores_every_keyed_row_right() {
        for k in keys() {
            let key = Key::read(&k).unwrap();
            let d = oracle(&key);
            let g = grade_keyed(&key, &d);
            let who = k.display().to_string();
            assert_eq!(g.fields.right, g.fields.of(), "{who}: {:?}", g.details);
            assert_eq!(g.elements_found, g.elements_of, "{who}: {:?}", g.details);
            assert_eq!(g.numbers.right, g.numbers.of(), "{who}: {:?}", g.details);
            assert_eq!(g.transitions_found, g.transitions_of, "{who}");
            assert_eq!(g.transitions_extra, 0, "{who}");
            let w = g.wrong_total();
            assert_eq!(w.wrong, 0, "{who}: {:?}", g.details);
            assert_eq!(w.unplaced, 0, "{who}: {:?}", g.stated);
        }
    }

    #[test]
    fn every_change_to_the_oracle_is_graded_wrong() {
        for k in keys() {
            let key = Key::read(&k).unwrap();
            let who = k.display().to_string();
            // Each number changed.
            let mut d = oracle(&key);
            let changed = d
                .tables
                .get_mut("constants")
                .unwrap()
                .iter_mut()
                .map(|r| {
                    if let Some(Cell::N(Some(x))) = r.get_mut("number") {
                        *x += 1.0;
                    }
                })
                .count();
            let g = grade_keyed(&key, &d);
            assert_eq!(g.numbers.right, 0, "{who}");
            assert_eq!(g.numbers.wrong, g.numbers.of(), "{who}");
            assert_eq!(g.stated["constants"].wrong, changed, "{who}");
            // Each RAM byte's role and quantity changed.
            let mut d = oracle(&key);
            let mut n_changed = 0;
            for r in d.tables.get_mut("ram").unwrap() {
                let a = i(r, "addr").unwrap() as u32;
                let f = &key.ram[&a];
                if !f.role.is_empty() || !f.quantity.is_empty() {
                    let other_role = ROLES
                        .iter()
                        .find(|x| !f.role.iter().any(|y| y == *x))
                        .unwrap();
                    let other_q = QUANTITIES
                        .iter()
                        .find(|x| !f.quantity.iter().any(|y| y == *x))
                        .unwrap();
                    if !f.role.is_empty() {
                        r.insert("role".into(), Cell::S(Some((*other_role).into())));
                    }
                    if !f.quantity.is_empty() {
                        r.insert("quantity".into(), Cell::S(Some((*other_q).into())));
                    }
                    n_changed += 1;
                }
            }
            let g = grade_keyed(&key, &d);
            assert_eq!(g.stated["ram"].wrong, n_changed, "{who}");
            // Every element's bytes merged into one: one key element
            // matches it, and every other one's fields are wrong.
            let mut d = oracle(&key);
            for r in d.tables.get_mut("ram").unwrap() {
                if s(r, "element").is_some() {
                    r.insert("element".into(), Cell::S(Some("all".into())));
                }
            }
            let m = match_elements(&key, &d);
            assert_eq!(m.len(), 1, "{who}");
            let kept = m.keys().next().unwrap();
            let others = key
                .fields()
                .values()
                .filter(|(e, _)| e.as_ref() != Some(kept))
                .count();
            let g = grade_keyed(&key, &d);
            assert_eq!(g.fields.wrong, others, "{who}");
            assert_eq!(g.fields.right, g.fields.of() - others, "{who}");
            // Every transition reversed (a value to itself left out).
            let mut d = oracle(&key);
            for r in d.tables.get_mut("transitions").unwrap() {
                let f = r["from_state"].clone();
                let t = r["to_state"].clone();
                r.insert("from_state".into(), t);
                r.insert("to_state".into(), f);
            }
            let g = grade_keyed(&key, &d);
            let reversed: BTreeSet<_> =
                key.transitions.iter().map(|&(b, f, t)| (b, t, f)).collect();
            let still = key.transitions.intersection(&reversed).count();
            assert_eq!(g.transitions_found, still, "{who}");
            let complete_wrong = reversed
                .difference(&key.transitions)
                .filter(|(b, _, _)| key.machines.iter().any(|m| m.byte == *b && m.complete))
                .count();
            assert_eq!(g.stated["transitions"].wrong, complete_wrong, "{who}");
        }
    }

    #[test]
    fn rows_the_key_doesnt_state_are_unplaced_not_dropped() {
        let key = Key::read(&root().join("tests/keys/mosshop")).unwrap();
        let mut d = oracle(&key);
        d.push(
            "ram",
            json!({"rom": key.head.rom, "addr": 0x7FF, "role": "state"}),
        )
        .unwrap();
        d.push(
            "constants",
            json!({"rom": key.head.rom, "pc": "C001", "number": 3.0}),
        )
        .unwrap();
        let g = grade_keyed(&key, &d);
        assert_eq!(g.stated["ram"].unplaced, 1);
        assert_eq!(g.stated["constants"].unplaced, 1);
        assert_eq!(g.wrong_total().wrong, 0);
    }

    #[test]
    fn coverage_counts_rows_and_open_rows() {
        let mut d = Dataset::default();
        d.push(
            "routines",
            json!({"rom": "x", "routine": "C000", "element": "player"}),
        )
        .unwrap();
        d.push("routines", json!({"rom": "x", "routine": "C100"}))
            .unwrap();
        d.push("open", json!({"rom": "x", "what": "routines:C100"}))
            .unwrap();
        d.push("ram", json!({"rom": "x", "addr": 5, "role": "state"}))
            .unwrap();
        d.push("open", json!({"rom": "x", "what": "ram:$0006"}))
            .unwrap();
        d.push("constants", json!({"rom": "x", "pc": "c010"}))
            .unwrap();
        let ran = Ran {
            routines: ["C000", "C100", "C200"].map(String::from).into(),
            ram: [5, 6, 7].into(),
            immediates: ["C010", "C011"].map(String::from).into(),
            ..Default::default()
        };
        let c = coverage(&d, &ran);
        assert_eq!(
            c.routines,
            Cover {
                placed: 1,
                open: 1,
                of: 3
            }
        );
        assert_eq!(
            c.ram,
            Cover {
                placed: 1,
                open: 1,
                of: 3
            }
        );
        assert_eq!(
            c.constants,
            Cover {
                placed: 1,
                open: 0,
                of: 2
            }
        );
    }

    /// A RAM row is graded only on the columns it states: a null role with
    /// an element set is graded on the element alone, and the reverse.
    #[test]
    fn a_ram_row_is_graded_only_on_what_it_states() {
        let key = Key::read(&root().join("tests/keys/mosshop")).unwrap();
        let mut d = oracle(&key);
        let rows = d.tables.get_mut("ram").unwrap();
        // $10 (the player's x): role null, element kept.
        let r = rows
            .iter_mut()
            .find(|r| i(r, "addr") == Some(0x10))
            .unwrap();
        for c in ["role", "quantity", "part"] {
            r.insert(c.into(), Cell::S(None));
        }
        r.insert("slot".into(), Cell::OI(None));
        // $11: element null, role wrong.
        let r = rows
            .iter_mut()
            .find(|r| i(r, "addr") == Some(0x11))
            .unwrap();
        for c in ["element", "quantity", "part"] {
            r.insert(c.into(), Cell::S(None));
        }
        r.insert("slot".into(), Cell::OI(None));
        r.insert("role".into(), Cell::S(Some("sound".into())));
        // Every other row says nothing.
        for r in rows.iter_mut() {
            let a = i(r, "addr").unwrap();
            if a != 0x10 && a != 0x11 {
                for c in ["element", "role", "quantity", "part"] {
                    r.insert(c.into(), Cell::S(None));
                }
                r.insert("slot".into(), Cell::OI(None));
            }
        }
        let g = grade_keyed(&key, &d);
        assert_eq!(g.columns.get("ram.element"), Some(&(1, 0)));
        assert_eq!(g.columns.get("ram.role"), Some(&(1, 1)));
        assert!(!g.columns.contains_key("ram.quantity"));
        assert!(!g.columns.contains_key("ram.part"));
        assert_eq!(g.stated["ram"].stated, 2);
        assert_eq!(g.stated["ram"].wrong, 1);
    }

    /// Estimates are their own rows: close or far, never wrong, and one on
    /// nothing the key states is unplaced.
    #[test]
    fn an_estimate_never_adds_to_wrong() {
        for k in keys() {
            let key = Key::read(&k).unwrap();
            let base = grade_keyed(&key, &oracle(&key));
            let mut d = oracle(&key);
            let ds_name = |e: &str| {
                let k = key.elements.iter().position(|x| x.name == e).unwrap();
                format!("ds_{k}")
            };
            let mut places = BTreeSet::new();
            let (mut near, mut far) = (0, 0);
            for c in &key.constants {
                let (Some(m), Some(p), Some(e), Some(w)) =
                    (&c.mechanic, &c.parameter, &c.element, c.expected())
                else {
                    continue;
                };
                if !places.insert((e.clone(), m.clone(), p.clone())) {
                    continue;
                }
                // Every other place is far off; the rest within 5%.
                let x = if places.len() % 2 == 0 {
                    far += 1;
                    w * 3.0 + 100.0
                } else {
                    near += 1;
                    w * 1.05
                };
                d.push(
                    "estimates",
                    json!({"rom": key.head.rom, "element": ds_name(e), "mechanic": m,
                           "parameter": p, "value": x}),
                )
                .unwrap();
            }
            d.push(
                "estimates",
                json!({"rom": key.head.rom, "element": "nowhere", "mechanic": "platformer.jump",
                       "parameter": "gravity", "value": 1.0}),
            )
            .unwrap();
            let g = grade_keyed(&key, &d);
            let who = k.display().to_string();
            assert_eq!(g.wrong_total(), base.wrong_total(), "{who}");
            assert_eq!(g.estimates.unplaced, 1, "{who}");
            assert_eq!(g.estimates.close + g.estimates.far, near + far, "{who}");
            assert!(g.estimates.far <= far, "{who}");
            assert!(g.estimates.close >= near, "{who}");
        }
    }

    #[test]
    fn close_is_ten_percent_two_frames_or_the_spread() {
        assert!(estimate_close(1.09, 1.0, false, None));
        assert!(!estimate_close(1.2, 1.0, false, None));
        assert!(estimate_close(12.0, 10.0, true, None));
        assert!(!estimate_close(13.0, 10.0, true, None));
        assert!(estimate_close(1.5, 1.0, false, Some(0.5)));
    }

    /// The breakdown by table and column, with every row wrong, holds only
    /// the schema's table and column names and counts.
    #[test]
    fn the_breakdown_holds_nothing_of_the_key() {
        for k in keys() {
            let key = Key::read(&k).unwrap();
            let mut d = oracle(&key);
            for r in d.tables.get_mut("constants").unwrap() {
                if let Some(Cell::N(Some(x))) = r.get_mut("number") {
                    *x += 1.0;
                }
                r.insert("quantity".into(), Cell::S(Some("other".into())));
            }
            for r in d.tables.get_mut("ram").unwrap() {
                r.insert("role".into(), Cell::S(Some("sound".into())));
            }
            let g = grade_keyed(&key, &d);
            assert!(g.columns["constants.number"].1 > 0);
            let v = scorecard(
                &coverage(&d, &Ran::default()),
                Some((&key, &g)),
                &Ran::default(),
                &BTreeMap::new(),
            );
            let t = text(&v, "Scorecard, counts only");
            let start = t.find("By table and column").unwrap();
            let names: BTreeSet<String> = crate::rom_dataset::TABLES
                .iter()
                .flat_map(|t| t.cols.iter().map(move |c| format!("{}.{}", t.name, c.name)))
                .chain(["elements.match", "transitions.change"].map(String::from))
                .collect();
            for line in t[start..]
                .lines()
                .skip(1)
                .take_while(|l| l.starts_with("  "))
            {
                let w: Vec<&str> = line.split_whitespace().collect();
                match w.as_slice() {
                    [col, "wrong", a, "of", b] => {
                        assert!(names.contains(*col), "{line}");
                        assert!(
                            a.parse::<usize>().is_ok() && b.parse::<usize>().is_ok(),
                            "{line}"
                        );
                    }
                    [table, "unplaced", a] => {
                        assert!(
                            crate::rom_dataset::TABLES.iter().any(|t| t.name == *table),
                            "{line}"
                        );
                        assert!(a.parse::<usize>().is_ok(), "{line}");
                    }
                    _ => panic!("{}: {line}", k.display()),
                }
            }
            for c in &key.constants {
                assert!(!t.contains(&c.path));
            }
        }
    }

    /// `--counts` prints no key detail: nothing but numbers and the
    /// scorecard's own words.
    #[test]
    fn counts_print_nothing_of_the_key() {
        let key = Key::read(&root().join("tests/keys/mosshop")).unwrap();
        let d = oracle(&key);
        let g = grade_keyed(&key, &d);
        let cov = coverage(&d, &Ran::default());
        let v = scorecard(&cov, Some((&key, &g)), &Ran::default(), &BTreeMap::new());
        let t = text(&v, "Scorecard, counts only");
        for c in &key.constants {
            assert!(!t.contains(&c.path));
            for pc in &c.pc {
                assert!(!t.contains(pc.as_str()), "{pc}");
            }
        }
        for e in &key.elements {
            assert!(!t.contains(&format!(" {} ", e.name)), "{}", e.name);
        }
        assert!(!t.contains('$'));
        assert!(v.get("details").is_none());
    }
}
