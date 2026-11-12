//! Gaps: what a recording of the original game shows that the model
//! doesn't account for (docs/gaps.md). Detect and catalog only; nothing
//! here changes a model.
//!
//! Three detectors read the same per-frame facts ([`Run`]), which
//! `gp gaps` builds from a recording's frame trace, its event stream, its
//! replay with the code observer (`gp_coverage::activity`) and the model's
//! state mapping:
//!
//! - [`changes`]: state that changes during play and that nothing in the
//!   model names, in five places: RAM, sprites, the background, sound and
//!   code.
//! - [`interactive`]: every background cell that changes during play (and
//!   every handler routine that fires without one), with the event context
//!   around it, clustered into interactive element kinds.
//! - [`drawn`]: the model's sprite boxes, placed as the presentation draws
//!   them, against the original's sprites for the same objects.
//!
//! [`focus`] keeps what the three found in a window a person marked, and
//! [`score`] holds their findings to a game's labelled reference set.
//!
//! Nothing here knows a game. Addresses, labels, entity kinds and counters
//! come in as data.

pub mod align;
pub mod changes;
pub mod code;
pub mod drawn;
pub mod focus;
pub mod interactive;
pub mod score;
pub mod sound;

use std::collections::BTreeSet;

pub use gp_track::sprites::Cluster;
use serde::{Deserialize, Serialize};

/// The format version of the reports and the catalog (docs/gaps.md,
/// docs/formats/interactive.md).
pub const SCHEMA: u32 = 0;

/// A PRG offset for code outside PRG ROM, or no routine.
pub const NO_ROM: u32 = u32::MAX;

/// One mapped object in a frame: the player or a slot's occupant, at its
/// model position (world pixels, from the state mapping).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ent {
    /// Index into [`Run::kinds`], or `None` for a slot whose ID no model
    /// entity lists.
    pub kind: Option<u16>,
    pub player: bool,
    pub slot: Option<u8>,
    pub id: Option<u8>,
    pub x: i32,
    pub y: i32,
}

/// A background cell that differs from the frame before: a 2x2 tile block
/// of physical nametable `nt`, with its tiles before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellChange {
    pub nt: u8,
    pub cx: u8,
    pub cy: u8,
    pub before: [u8; 4],
    pub after: [u8; 4],
    /// Routines that wrote the cell's tiles through $2007 this frame.
    pub writers: Vec<u32>,
}

/// One attributed RAM write that changed a byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamWrite {
    pub addr: u16,
    pub value: u8,
    /// Made by the sound engine (a routine that writes the APU on most of
    /// its calls, or code under one).
    pub engine: bool,
    pub pc: u32,
    pub routine: u32,
}

/// What the code did in a frame (from the replay's observer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Code {
    /// Routines entered, by PRG offset, with how many times.
    pub entered: Vec<(u32, u32)>,
    pub ram: Vec<RamWrite>,
    /// Routines that wrote the APU this frame.
    pub apu: Vec<u32>,
    /// The PRG offsets of the instructions that did.
    pub apu_pc: Vec<u32>,
    /// Calls to watched routines: target, caller PRG offset, A register,
    /// and whether the caller is the sound engine.
    pub calls: Vec<(u32, u32, u8, bool)>,
}

/// A named constant, and the names the source uses within three lines
/// after it (`lda #Sfx_Bump` / `sta Square1SoundQueue` ties the effect to
/// its queue).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Constant {
    pub name: String,
    pub value: i64,
    pub near: Vec<String>,
}

/// What every call to one routine looked like over the run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallStat {
    pub target: u32,
    pub calls: u64,
    pub callers: usize,
    pub distinct_a: u32,
    pub reaches_apu: bool,
    /// Most of its calls write the APU.
    pub engine: bool,
}

/// One sprite object on screen, with the sprite table entries it's made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub c: Cluster,
    pub sprites: Vec<u8>,
}

/// Everything the detectors read about one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameFacts {
    pub frame: u64,
    /// The player is in control (the mapping's `start` condition holds, or
    /// the game read its controller when the mapping has none).
    pub play: bool,
    pub pad: u8,
    /// The model camera's top left, world pixels.
    pub camera: [i32; 2],
    /// The PPU scroll the background was drawn with: x 0-511, y 0-479.
    pub scroll: [u16; 2],
    /// Nametable mirroring, as the canonical state's `mapper.mirroring`:
    /// 0 vertical, 1 horizontal, 2 and 3 one screen, 4 four screens.
    pub mirroring: u8,
    /// RAM bytes that differ from the frame before.
    pub ram_changed: Vec<u16>,
    /// Counter values, by [`Run::counters`].
    pub counters: Vec<i64>,
    pub ents: Vec<Ent>,
    /// The sprites drawn from this frame's RAM.
    pub objects: Vec<Object>,
    pub cells: Vec<CellChange>,
    /// APU register writes this frame, in order.
    pub apu: Vec<(u16, u8)>,
    pub code: Option<Code>,
}

/// A RAM label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RamLabel {
    pub name: String,
    pub addr: u16,
    pub size: u16,
}

/// A label in PRG ROM, by offset; `addr` is the CPU address it runs at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RomLabel {
    pub name: String,
    pub prg: u32,
    pub addr: u16,
    pub size: u32,
}

/// What the model already accounts for, and the names to report with.
#[derive(Debug, Clone, Default)]
pub struct Known {
    /// RAM bytes the state mapping reads (fields and slots).
    pub mapped: BTreeSet<u16>,
    /// Names the model cites: every identifier in its facts' evidence, its
    /// mapping, its mechanic records and its hooks' source.
    pub cited: BTreeSet<String>,
    pub ram: Vec<RamLabel>,
    /// Sorted by `prg`.
    pub rom: Vec<RomLabel>,
    /// Data labels in PRG ROM (tables, music and sound data), for naming
    /// what a pointer table points at.
    pub data: Vec<RomLabel>,
    /// The RAM page the sprite table is copied from.
    pub oam_page: u8,
    /// Mechanisms of the model that change level cells at run time.
    pub tile_mechanics: Vec<String>,
    /// Whether the model has any sound.
    pub sound: bool,
    /// Nametable tile rows from the top that the model's HUD draws
    /// (`present.hud`): cell changes there are the HUD's counters, which
    /// the model shows.
    pub hud_rows: u8,
}

impl Known {
    /// The RAM label covering `addr`, or the nearest one at most 8 bytes
    /// before it, as `Name` or `Name+n`.
    pub fn ram_name(&self, addr: u16) -> (String, Option<&RamLabel>) {
        let covering = self
            .ram
            .iter()
            .filter(|l| l.addr <= addr && addr < l.addr + l.size.max(1))
            .min_by_key(|l| l.size);
        if let Some(l) = covering {
            return (l.name.clone(), Some(l));
        }
        let near = self
            .ram
            .iter()
            .filter(|l| l.addr <= addr && addr - l.addr <= 8)
            .max_by_key(|l| l.addr);
        match near {
            Some(l) => (format!("{}+{}", l.name, addr - l.addr), Some(l)),
            None => (format!("${:03X}x", addr >> 4), None),
        }
    }

    /// Whether the model names the byte: mapped, or labelled by a name the
    /// model cites.
    pub fn explains_ram(&self, addr: u16) -> bool {
        if self.mapped.contains(&addr) {
            return true;
        }
        self.ram.iter().any(|l| {
            l.addr <= addr && addr < l.addr + l.size.max(1) && self.cited.contains(&l.name)
        })
    }

    /// The ROM label covering PRG offset `prg`.
    pub fn routine(&self, prg: u32) -> Option<&RomLabel> {
        if prg == NO_ROM {
            return None;
        }
        let i = self.rom.partition_point(|l| l.prg <= prg);
        let l = self.rom.get(i.checked_sub(1)?)?;
        (prg < l.prg + l.size.max(1)).then_some(l)
    }

    /// A routine's name: its label, or `sub_<offset>` for an unlabelled
    /// entry (a `JSR` target or interrupt handler), or `ram` for code run
    /// from RAM.
    pub fn routine_name(&self, prg: u32) -> String {
        if prg == NO_ROM {
            return "ram".into();
        }
        match self.routine(prg) {
            Some(l) if l.prg == prg => l.name.clone(),
            Some(l) => format!("{}+{}", l.name, prg - l.prg),
            None => format!("sub_{prg:05X}"),
        }
    }

    /// The name of the code at PRG offset `pc`, for attributing a write:
    /// the label covering it, or else the routine it ran in.
    pub fn writer_name(&self, pc: u32, routine: u32) -> String {
        match self.routine(pc) {
            Some(l) => l.name.clone(),
            None => self.routine_name(routine),
        }
    }

    /// Whether the model cites the routine at `prg` by name.
    pub fn cites_routine(&self, prg: u32) -> bool {
        self.routine(prg)
            .is_some_and(|l| self.cited.contains(&l.name))
    }
}

/// A run's frames and what's known about them.
#[derive(Debug, Clone, Default)]
pub struct Run {
    pub frames: Vec<FrameFacts>,
    /// Model entity names, by [`Ent::kind`].
    pub kinds: Vec<String>,
    /// Counter names, by [`FrameFacts::counters`].
    pub counters: Vec<String>,
    /// What each counter's mapped value was multiplied by to give the
    /// units the player sees (a score kept as its displayed digits, with a
    /// fixed 0 after them, is 10); empty or 1 for none.
    pub counter_scale: Vec<i64>,
    pub known: Known,
    /// Every instruction start the replay ran, by PRG offset (empty without
    /// a replay).
    pub executed: Vec<u64>,
    /// The PRG ROM, for reading code statically (empty without the ROM).
    pub prg: Vec<u8>,
    /// Calls by `JSR` target, from the replay.
    pub calls: Vec<CallStat>,
    /// Named constants of the disassembly (a listing's equates), for
    /// naming sound effect IDs.
    pub constants: Vec<Constant>,
    /// Routines whose calls were watched (play-sound candidates).
    pub watched: Vec<u32>,
}

impl Run {
    pub fn has_code(&self) -> bool {
        self.frames.iter().any(|f| f.code.is_some())
    }

    /// Whether the instruction at PRG offset `o` ran.
    pub fn ran(&self, o: u32) -> bool {
        let o = o as usize;
        self.executed
            .get(o / 64)
            .is_some_and(|w| w & (1 << (o % 64)) != 0)
    }

    pub fn play_frames(&self) -> usize {
        self.frames.iter().filter(|f| f.play).count()
    }

    /// Takes out of play the frames a title screen, a menu or a screen of
    /// text the mapping's condition holds on: frames on which the player
    /// hasn't moved for [`STILL`] frames either way, and, with the code
    /// signals, frames more than [`MOVER_GAP`] frames from any run of the code
    /// that moves the player. That code is the routines that make 80% of
    /// the play frames' writes to the player's position bytes
    /// (`player_ram`), the busiest first; other code that borrows those
    /// bytes (a title screen's) writes them far less. Returns how many
    /// frames it took out. A run with no mapped player is left as it is.
    pub fn refine_play(&mut self, player_ram: &[u16]) -> usize {
        let n = self.frames.len();
        let before = self.play_frames();
        if self.has_code() && !player_ram.is_empty() {
            let mut by: std::collections::BTreeMap<u32, u64> = Default::default();
            for f in self.frames.iter().filter(|f| f.play) {
                let Some(c) = &f.code else { continue };
                for w in &c.ram {
                    if player_ram.contains(&w.addr) {
                        *by.entry(w.routine).or_default() += 1;
                    }
                }
            }
            let total: u64 = by.values().sum();
            let mut ranked: Vec<(u32, u64)> = by.into_iter().collect();
            ranked.sort_by_key(|x| std::cmp::Reverse(x.1));
            let mut movers = BTreeSet::new();
            let mut sum = 0;
            for (r, c) in ranked {
                if sum * 5 >= total * 4 {
                    break;
                }
                sum += c;
                movers.insert(r);
            }
            if !movers.is_empty() {
                let ran: Vec<bool> = self
                    .frames
                    .iter()
                    .map(|f| {
                        f.code
                            .as_ref()
                            .is_some_and(|c| c.entered.iter().any(|e| movers.contains(&e.0)))
                    })
                    .collect();
                let near = within(&ran, MOVER_GAP);
                for (f, ok) in self.frames.iter_mut().zip(near) {
                    if !ok {
                        f.play = false;
                    }
                }
            }
        }
        let pos: Vec<Option<(i32, i32)>> = self
            .frames
            .iter()
            .map(|f| f.ents.iter().find(|e| e.player).map(|e| (e.x, e.y)))
            .collect();
        if pos.iter().all(Option::is_none) {
            return 0;
        }
        let mut moved = vec![false; n];
        for i in 1..n {
            if pos[i].is_some() && pos[i - 1].is_some() && pos[i] != pos[i - 1] {
                moved[i] = true;
            }
        }
        let near = within(&moved, STILL);
        for (f, ok) in self.frames.iter_mut().zip(near) {
            if !ok {
                f.play = false;
            }
        }
        before - self.play_frames()
    }
}

/// Frames either way of a run of the code that moves the player a frame
/// must be within to be in play ([`Run::refine_play`]): half a second.
pub const MOVER_GAP: usize = 30;

/// Which frames are within `d` frames of one where `at` holds.
fn within(at: &[bool], d: usize) -> Vec<bool> {
    let n = at.len();
    let mut near = vec![false; n];
    let mut last: Option<usize> = None;
    for i in 0..n {
        if at[i] {
            last = Some(i);
        }
        near[i] = last.is_some_and(|l| i - l <= d);
    }
    let mut next: Option<usize> = None;
    for i in (0..n).rev() {
        if at[i] {
            next = Some(i);
        }
        near[i] |= next.is_some_and(|x| x - i <= d);
    }
    near
}

/// Frames either way the player must have moved within for a frame to be
/// in play ([`Run::refine_play`]): five seconds.
pub const STILL: usize = 300;

/// Where a background cell is on screen, if it's wholly visible: its left
/// and top in screen pixels. Physical nametable `nt` shows at the logical
/// places `mirroring` gives it, and the screen starts at `scroll`.
pub fn cell_on_screen(
    nt: u8,
    cx: u8,
    cy: u8,
    mirroring: u8,
    scroll: [u16; 2],
) -> Option<(i32, i32)> {
    let places: Vec<(i32, i32)> = match mirroring {
        // Vertical mirroring (top and bottom are the same table): physical
        // n is column n.
        0 => vec![(i32::from(nt) * 256, 0), (i32::from(nt) * 256, 240)],
        // Horizontal mirroring: physical n is row n.
        1 => vec![(0, i32::from(nt) * 240), (256, i32::from(nt) * 240)],
        4 => vec![(i32::from(nt & 1) * 256, i32::from(nt >> 1) * 240)],
        _ => vec![(0, 0), (256, 0), (0, 240), (256, 240)],
    };
    for (lx, ly) in places {
        let sx = (lx + i32::from(cx) * 16 - i32::from(scroll[0])).rem_euclid(512);
        let sy = (ly + i32::from(cy) * 16 - i32::from(scroll[1])).rem_euclid(480);
        if sx <= 240 && sy <= 224 {
            return Some((sx, sy));
        }
    }
    None
}

/// The APU channel a register belongs to.
pub fn channel(reg: u16) -> &'static str {
    match reg {
        0x4000..=0x4003 => "pulse1",
        0x4004..=0x4007 => "pulse2",
        0x4008..=0x400B => "triangle",
        0x400C..=0x400F => "noise",
        0x4010..=0x4013 => "dmc",
        _ => "control",
    }
}

/// The channel a write starts a note on: a length counter load (the
/// fourth register of a tone channel) or a $4015 write that starts the
/// DMC.
pub fn trigger(reg: u16, value: u8) -> Option<&'static str> {
    match reg {
        0x4003 | 0x4007 | 0x400B | 0x400F => Some(channel(reg)),
        0x4015 if value & 0x10 != 0 => Some("dmc"),
        _ => None,
    }
}

pub const CHANNELS: [&str; 5] = ["pulse1", "pulse2", "triangle", "noise", "dmc"];

/// Signature text of a 2x2 tile block.
pub fn sig(t: &[u8; 4]) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}", t[0], t[1], t[2], t[3])
}

/// Mean and standard deviation.
pub fn mean_sd(v: &[f64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let n = v.len() as f64;
    let m = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n;
    (m, var.sqrt())
}

/// Rounds for reports.
pub fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Counts in a map, as the `n` largest, largest first, ties by key.
pub fn top<K: Ord + Clone>(m: &std::collections::BTreeMap<K, u64>, n: usize) -> Vec<(K, u64)> {
    let mut v: Vec<(K, u64)> = m.iter().map(|(k, c)| (k.clone(), *c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

#[cfg(test)]
mod tests;
