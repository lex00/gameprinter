//! `gp localize`: find play that no run has made yet, say where in a level
//! it could happen, and send the search there (docs/localize.md).
//!
//! 1. One-way branches: every clip and run of a game that isn't a recording
//!    is replayed while watching each conditional branch; a branch that ran
//!    in the level but only ever went one way is a lead.
//! 2. Guards: for each, the frame it first ran is replayed with the writer
//!    slicer (gp_nes::slice), and the instruction that set the branch's flag
//!    is sliced back to the RAM bytes and constants it was computed from.
//!    When the flag came from a comparison of one RAM byte with a constant,
//!    the values that would send the branch the other way are worked out.
//! 3. Localize: a guard that read the level grid (the template mapping's
//!    [cells]) is checked against the grid as the runs saw it: a cell that
//!    holds a value the other way needs is a place, and where the cell sat
//!    from the player each time the guard ran says how to meet it.
//! 4. Probe: Go-Explore from a savestate before the place, with the moves
//!    ordered by the decision model, at random, or not aimed at all, scored
//!    by the distance to meeting the cell as the guard does; it stops when
//!    the branch goes the other way.
//!
//! Nothing here knows a game: the bytes come from the ROM dataset and the
//! template's observe.toml and mapping.toml. `--grade` is the grader's
//! part, kept apart: it replays a recording (a TAS) to see whether and where
//! it sends the same branch the other way, and watches one byte it is told.
//!
//! Ported from game-printer's localize.rs. What gameprinter's layout
//! changed: the game is its package in the games checkout (its
//! observe.toml [level] and mapping.toml [cells], and the ROM its
//! roms.toml names), where game-printer read templates/GAME; blind
//! discovery and the blind run are wave 7's (crate::farm::blind); clips are
//! read through crate::mechanic::paths. It reads the NES's 6502: each
//! conditional branch's direction from tetanes-core's bus and the writer
//! slice (gp_nes::slice). An arcade game is refused in words; it needs the
//! same over the main Z80 (each conditional jump's direction is in
//! gp-arcade's instruction trace, as crate::covered reads it, and
//! crate::element_z80 has a writer slice back from a write, which would
//! have to slice back from a jump's flags instead), and a level grid in
//! the mapping, which Tapper's hasn't.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use clap::Args;
use serde_json::{Value, json};

use crate::Outcome;
use crate::farm::{self, Move, Rng};
use gp_evidence::Paths;
use gp_evidence::input_log::{InputFrame, InputLog};
use gp_nes::slice::{self, From, Loc, Recorder};

#[derive(Args, Debug, Clone)]
pub(crate) struct LocalizeArgs {
    /// The game, as its package in the games checkout is named. Without one,
    /// give --rom: everything is then learned from the ROM and its play.
    #[arg(required_unless_present = "rom")]
    pub game: Option<String>,
    /// A ROM file to read instead of the game's: with no game,
    /// the player's bytes come from blind discovery, the whole run counts
    /// as one level, sessions are keyed by the ROM's hash, and a blind run
    /// is made first when no clip or run of the ROM exists.
    #[arg(long)]
    pub rom: Option<std::path::PathBuf>,
    /// The level to look in, by its observe.toml [level] name (game-printer
    /// defaulted to Super Mario Bros.' w1_1; gameprinter to Donkey Kong's).
    #[arg(long, default_value = "main")]
    pub level: String,
    /// The decision server for the probes.
    #[arg(long, default_value = "http://127.0.0.1:7720")]
    pub decide: String,
    /// How many of the best-placed leads to probe.
    #[arg(long, default_value_t = 3)]
    pub probes: usize,
    /// Moves simulated for each probe and each way of ordering them.
    #[arg(long, default_value_t = 400)]
    pub budget: usize,
    /// Also run the RAM-poke sweep: put the player next to each grid cell
    /// of each screen along the furthest run (below rising, above falling,
    /// beside moving toward it) for a few frames, and note what flips.
    #[arg(long)]
    pub poke: bool,
    /// Leave out the probes: list, slice and localize only.
    #[arg(long)]
    pub no_probe: bool,
    /// The grader's part: a store recording to replay for the same
    /// branch going the other way. Not read by steps 1 to 4.
    #[arg(long)]
    pub grade: Option<String>,
    /// Collect what came out of a place: from this run's state at
    /// `--from-frame`, search for the player touching the new sprite group
    /// that appears by the cell at `--at`, until a counter-like byte goes
    /// up at the touch (a byte that held for a second goes up by one and
    /// stays). Runs uniform, undirected and with the decision model.
    #[arg(long, requires_all = ["from_frame", "at"])]
    pub collect: Option<std::path::PathBuf>,
    /// With --collect: the frame of the run to start from.
    #[arg(long)]
    pub from_frame: Option<usize>,
    /// With --collect: the cell, as level x,y.
    #[arg(long, value_parser = parse_xy)]
    pub at: Option<(i64, i64)>,
    /// The grader's check of one run: replay this input log from power-on
    /// (then 240 frames of nothing) and report when `--branch` goes the
    /// way it names, the watched byte, and the sprites near the player;
    /// with --grade, the same branch in that recording. Steps 1 to 4
    /// don't run.
    #[arg(long, requires = "branch")]
    pub check_log: Option<std::path::PathBuf>,
    /// With --check-log: the branch (hex) and the way that counts: taken
    /// or fell.
    #[arg(long, value_parser = parse_branch)]
    pub branch: Option<(u16, bool)>,
    /// With --grade, a RAM byte to report before and after (hex).
    #[arg(long, value_parser = parse_hex)]
    pub watch: Option<u16>,
}

fn parse_hex(s: &str) -> std::result::Result<u16, String> {
    u16::from_str_radix(s.trim_start_matches("0x").trim_start_matches('$'), 16)
        .map_err(|e| e.to_string())
}

fn parse_xy(s: &str) -> std::result::Result<(i64, i64), String> {
    let (x, y) = s.split_once(',').ok_or("X,Y")?;
    Ok((
        x.trim().parse().map_err(|e| format!("{e}"))?,
        y.trim().parse().map_err(|e| format!("{e}"))?,
    ))
}

fn parse_branch(s: &str) -> std::result::Result<(u16, bool), String> {
    let (b, w) = s.split_once(':').ok_or("BRANCH:taken or BRANCH:fell")?;
    Ok((
        parse_hex(b)?,
        match w {
            "taken" => true,
            "fell" => false,
            _ => return Err("the way is taken or fell".into()),
        },
    ))
}

/// A replayable input log from power-on.
struct Src {
    name: String,
    frames: Vec<InputFrame>,
}

/// Every clip of the game's mechanic sessions except those cut from a
/// recording (harvest-*) or generated (jumpathon), and the blind runs of
/// this ROM.
fn sources(paths: &Paths, game: &str, sha: &str) -> Result<Vec<Src>> {
    let mut out = Vec::new();
    let root = paths.home.join("mechanics").join(game);
    let mut dirs: Vec<_> = fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("clips").is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let name = d
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        if name.starts_with("harvest") || name.starts_with("jumpathon") {
            continue;
        }
        let mut logs: Vec<_> = fs::read_dir(d.join("clips"))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "log"))
            .collect();
        logs.sort();
        for l in logs {
            // The full log from power-on, also for a clip saved as a path,
            // a frame on it and its own buttons.
            let n: u64 = l
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let Ok((text, _)) = crate::mechanic::paths::full_log(&paths.home, &d, n) else {
                continue;
            };
            let Ok(log) = InputLog::parse(&text) else {
                continue;
            };
            out.push(Src {
                name: format!(
                    "{name}/{}",
                    l.file_stem().and_then(|s| s.to_str()).unwrap_or("")
                ),
                frames: log.frames,
            });
        }
    }
    // Blind runs whose result names this ROM.
    for g in fs::read_dir(paths.home.join("work/farm/blind"))
        .into_iter()
        .flatten()
        .flatten()
    {
        for r in fs::read_dir(g.path()).into_iter().flatten().flatten() {
            let res = r.path().join("result.json");
            let Ok(v) = fs::read_to_string(&res).map(|t| serde_json::from_str::<Value>(&t)) else {
                continue;
            };
            let Ok(v) = v else { continue };
            if v["sha256"].as_str() != Some(sha) {
                continue;
            }
            let Ok(text) = fs::read_to_string(r.path().join("path.log")) else {
                continue;
            };
            let Ok(log) = InputLog::parse(&text) else {
                continue;
            };
            if log.frames.len() > 600 {
                out.push(Src {
                    name: format!("blind/{}", r.file_name().to_string_lossy()),
                    frames: log.frames,
                });
            }
        }
    }
    Ok(out)
}

/// The level key: observe.toml [level] (as gp farm reads it).
struct LevelKey {
    ram: Vec<u16>,
    when: Option<gp_runtime::select::Condition>,
    names: BTreeMap<Vec<u8>, String>,
}

impl LevelKey {
    fn load(game_dir: &Path) -> Result<LevelKey> {
        let t: toml::Value = toml::from_str(&fs::read_to_string(game_dir.join("observe.toml"))?)?;
        let l = t
            .get("level")
            .context("the game's observe.toml has no [level]")?;
        let ram = l
            .get("ram")
            .and_then(|v| v.as_array())
            .context("[level] ram")?
            .iter()
            .filter_map(|v| v.as_integer().map(|n| n as u16))
            .collect();
        let when = l
            .get("when")
            .and_then(|w| w.as_str())
            .and_then(|w| gp_runtime::select::Condition::parse(w).ok());
        let mut names = BTreeMap::new();
        for n in l
            .get("names")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(a) = n.as_array() else { continue };
            let (Some(k), Some(v)) = (
                a.first().and_then(|k| k.as_str()),
                a.get(1).and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let key: Vec<u8> = k
                .split_whitespace()
                .filter_map(|h| u8::from_str_radix(h.trim_start_matches("0x"), 16).ok())
                .collect();
            names.insert(key, v.to_owned());
        }
        Ok(LevelKey { ram, when, names })
    }

    /// No level key (a ROM with no template): every frame is in "all".
    fn any() -> LevelKey {
        LevelKey {
            ram: Vec::new(),
            when: None,
            names: BTreeMap::from([(Vec::new(), "all".to_owned())]),
        }
    }

    fn of(&self, w: &[u8]) -> Option<String> {
        if let Some(c) = &self.when {
            let ok = c
                .eval(&mut |sel| match (sel.name().as_str(), sel.last_index()?) {
                    ("ram", Some(gp_runtime::select::Index::At(n))) if (0..0x800).contains(&n) => {
                        Ok(i64::from(w[n as usize]))
                    }
                    _ => Err("only ram[N]".into()),
                })
                .unwrap_or(false);
            if !ok {
                return None;
            }
        }
        let key: Vec<u8> = self.ram.iter().map(|a| w[usize::from(*a)]).collect();
        Some(self.names.get(&key).cloned().unwrap_or_else(|| {
            key.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join("")
        }))
    }
}

/// The level grid: mapping.toml [cells].
struct Grid {
    pages: Vec<u16>,
    cols: usize,
    rows: usize,
    top: usize,
}

impl Grid {
    fn load(game_dir: &Path) -> Option<Grid> {
        let t: toml::Value =
            toml::from_str(&fs::read_to_string(game_dir.join("mapping.toml")).ok()?).ok()?;
        let c = t.get("cells")?;
        Some(Grid {
            pages: c
                .get("pages")?
                .as_array()?
                .iter()
                .filter_map(|v| v.as_integer().map(|n| n as u16))
                .collect(),
            cols: c.get("page_columns")?.as_integer()? as usize,
            rows: c.get("rows")?.as_integer()? as usize,
            top: c.get("top").and_then(|v| v.as_integer()).unwrap_or(0) as usize,
        })
    }

    /// The (page, row, column in the page) of a grid byte.
    fn place(&self, a: u16) -> Option<(usize, usize, usize)> {
        let size = self.cols * self.rows;
        for (p, &base) in self.pages.iter().enumerate() {
            if a >= base && usize::from(a - base) < size {
                let i = usize::from(a - base);
                return Some((p, i / self.cols, i % self.cols));
            }
        }
        None
    }

    /// The level column of a page column nearest a level x.
    fn level_col(&self, page: usize, col: usize, near_x: i64) -> i64 {
        let span = (self.cols * self.pages.len()) as i64;
        let base = (page * self.cols + col) as i64;
        let near = near_x.div_euclid(16);
        let k = (near - base).div_euclid(span);
        // The nearest of the two candidates around `near`.
        let a = base + k * span;
        let b = a + span;
        if (near - a).abs() <= (b - near).abs() {
            a
        } else {
            b
        }
    }

    /// A cell's top y on the screen.
    fn row_y(&self, row: usize) -> i64 {
        ((row + self.top) * 16) as i64
    }
}

/// The player's bytes, from the ROM dataset.
struct Player {
    x_hi: Option<u16>,
    x_lo: u16,
    y: u16,
}

fn player_of(dir: &Path) -> Result<Player> {
    let d = crate::rom_dataset::read(dir)
        .with_context(|| format!("no ROM dataset at {}", dir.display()))?;
    let (mut hi, mut lo, mut y) = (None, None, None);
    for r in d.rows("ram") {
        let s = |k: &str| match r.get(k) {
            Some(crate::rom_dataset::Cell::S(Some(v))) => v.clone(),
            _ => String::new(),
        };
        let Some(crate::rom_dataset::Cell::I(a)) = r.get("addr") else {
            continue;
        };
        if s("element") != "player" {
            continue;
        }
        match (s("quantity").as_str(), s("part").as_str()) {
            ("x", "hi") => hi = Some(*a as u16),
            ("x", _) => lo = Some(*a as u16),
            ("y", _) => y = Some(*a as u16),
            _ => {}
        }
    }
    Ok(Player {
        x_hi: hi,
        x_lo: lo.context("the dataset has no player x byte")?,
        y: y.context("the dataset has no player y byte")?,
    })
}

/// The player's bytes a package's state mapping states
/// (`entities.player[0].x` and `.y`, with an `x_hi` field when it has one).
fn mapped_player(game_dir: &Path) -> Option<Player> {
    let m: toml::Table =
        toml::from_str(&fs::read_to_string(game_dir.join("mapping.toml")).ok()?).ok()?;
    let fields = m.get("field")?.as_array()?;
    let ram = |path: &str, n: usize| -> Option<u16> {
        fields.iter().find_map(|f| {
            (f.get("path")?.as_str()? == path)
                .then(|| f.get("ram")?.as_array()?.get(n)?.as_integer())
                .flatten()
                .map(|a| a as u16)
        })
    };
    let x = ram("entities.player[0].x", 0)?;
    // A two-byte x lists its low byte first.
    let hi = ram("entities.player[0].x", 1);
    Some(Player {
        x_hi: hi,
        x_lo: x,
        y: ram("entities.player[0].y", 0)?,
    })
}

impl Player {
    fn x(&self, w: &[u8]) -> i64 {
        self.x_hi.map_or(0, |h| i64::from(w[usize::from(h)])) * 256
            + i64::from(w[usize::from(self.x_lo)])
    }
    fn y(&self, w: &[u8]) -> i64 {
        i64::from(w[usize::from(self.y)])
    }
}

/// Whether a branch at `p` (opcode `op`) went to `next`: Some(taken).
fn went(bus: &tetanes_core::bus::Bus, p: u16, next: u16) -> Option<bool> {
    let fall = p.wrapping_add(2);
    let to = fall.wrapping_add(bus.peek(p.wrapping_add(1)) as i8 as u16);
    if to == fall {
        None
    } else if next == to {
        Some(true)
    } else if next == fall {
        Some(false)
    } else {
        None
    }
}

/// One branch's record over the runs.
#[derive(Default, Clone)]
struct Seen {
    taken: u64,
    fell: u64,
    /// (source, frame) of the first runs, up to a few.
    at: Vec<(usize, usize)>,
}

/// A comparison guarding a branch: the RAM byte compared, the constant,
/// and the values of the byte that send the branch the way no run went.
#[derive(Clone, Debug)]
struct Guard {
    op: u8,
    /// The way no run went: true = taken.
    want_taken: bool,
    /// The instruction that set the flag.
    setter: u16,
    constant: Option<u8>,
    /// The RAM byte the compared register came from, and its value then.
    source: Option<(u16, u8)>,
    /// Through a pointer (a grid read).
    indirect: bool,
    /// The compared register's value when the flag was set.
    reg_value: u8,
    /// RAM bytes the compared value was copied through, latest first
    /// (loads, stores and register transfers, across calls).
    chain: Vec<u16>,
    /// Values of the source byte that would send the branch the other way.
    flips: Vec<u8>,
    reads: Vec<(u16, u8)>,
}

/// The flags a compare-like instruction leaves for value `v`, as
/// (Z, C, N), or None if it isn't one this can work out.
fn flags_for(setter_op: u8, constant: Option<u8>, v: u8) -> Option<(bool, bool, bool)> {
    use tetanes_core::cpu::instr::{AddrMode, Instr};
    let r = tetanes_core::cpu::Cpu::INSTR_REF[usize::from(setter_op)];
    let imm = r.addr_mode == AddrMode::IMM;
    match r.instr {
        Instr::CMP | Instr::CPX | Instr::CPY if imm => {
            let k = constant?;
            let d = v.wrapping_sub(k);
            Some((v == k, v >= k, d & 0x80 != 0))
        }
        Instr::AND if imm => {
            let d = v & constant?;
            Some((d == 0, false, d & 0x80 != 0))
        }
        Instr::LDA | Instr::LDX | Instr::LDY => Some((v == 0, false, v & 0x80 != 0)),
        _ => None,
    }
}

/// Whether a branch opcode is taken given (Z, C, N); V isn't modelled.
fn taken_by(op: u8, (z, c, n): (bool, bool, bool)) -> Option<bool> {
    Some(match op {
        0x10 => !n,
        0x30 => n,
        0x90 => !c,
        0xB0 => c,
        0xD0 => !z,
        0xF0 => z,
        _ => return None,
    })
}

/// Replays `src` to `frame` and records that frame's instructions.
fn record_frame(rom: &[u8], src: &Src, frame: usize) -> Result<slice::FrameTrace> {
    let mut nes = gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
    nes.set_video(false);
    for f in &src.frames[..frame] {
        nes.step(*f)?;
    }
    let mut rec = Recorder::new();
    rec.start(nes.bus_mut_for_observers());
    nes.step_with(src.frames[frame], |bus| rec.before(bus))?;
    Ok(rec.finish())
}

/// The guard of branch `pc` from one recorded frame.
fn guard_of(t: &slice::FrameTrace, pc: u16, op: u8, want_taken: bool) -> Option<Guard> {
    let bi = t.steps.iter().position(|s| s.pc == pc)?;
    guard_at(t, bi, op, want_taken)
}

/// The guard of the branch run at step `bi`.
fn guard_at(t: &slice::FrameTrace, bi: usize, op: u8, want_taken: bool) -> Option<Guard> {
    let flag = t.steps[bi].branch_flag()?;
    // The latest step before it that defines the flag.
    let si = (0..bi).rev().find(|&j| {
        slice::effects(&t.steps[j]).is_some_and(|e| e.iter().any(|(l, _)| *l == flag))
    })?;
    let s = &t.steps[si];
    let sl = slice::slice(t, si, flag, slice::Options { branches: false });
    let reads: Vec<(u16, u8)> = sl
        .reads
        .iter()
        .filter(|r| r.addr < 0x800 && !matches!(r.from, From::Rom | From::Io))
        .map(|r| (r.addr, r.value))
        .collect();
    use tetanes_core::cpu::instr::AddrMode;
    let constant = (s.mode() == AddrMode::IMM).then_some((s.operand & 0xFF) as u8);
    // The compared register's source: for a compare, the latest RAM read
    // that loaded it; for a load, its own operand.
    let (source, indirect) = if s.addr.is_some_and(|a| a < 0x800)
        && matches!(
            s.instr(),
            tetanes_core::cpu::instr::Instr::LDA
                | tetanes_core::cpu::instr::Instr::LDX
                | tetanes_core::cpu::instr::Instr::LDY
        ) {
        (s.addr.map(|a| (a, s.value)), s.indirect())
    } else {
        let reg = match s.instr() {
            tetanes_core::cpu::instr::Instr::CPX => Loc::X,
            tetanes_core::cpu::instr::Instr::CPY => Loc::Y,
            _ => Loc::A,
        };
        // The latest load into that register before the setter.
        let li = (0..si).rev().find(|&j| {
            slice::effects(&t.steps[j]).is_some_and(|e| e.iter().any(|(l, _)| *l == reg))
        });
        match li.map(|j| &t.steps[j]) {
            Some(l) if l.addr.is_some_and(|a| a < 0x800) => {
                (l.addr.map(|a| (a, l.value)), l.indirect())
            }
            _ => (None, false),
        }
    };
    // Follow the compared value back further: slice the register's own
    // definition, and the last write of its source byte, and keep the RAM
    // bytes those read (a tile read through a pointer, kept in a scratch
    // byte, and compared in a routine called later).
    let mut reads = reads;
    let reg = match s.instr() {
        tetanes_core::cpu::instr::Instr::CPX | tetanes_core::cpu::instr::Instr::LDX => Loc::X,
        tetanes_core::cpu::instr::Instr::CPY | tetanes_core::cpu::instr::Instr::LDY => Loc::Y,
        _ => Loc::A,
    };
    let keep = |sl: &slice::Slice, out: &mut Vec<(u16, u8)>| {
        for r in &sl.reads {
            if r.addr < 0x800
                && !matches!(r.from, From::Rom | From::Io)
                && !out.contains(&(r.addr, r.value))
            {
                out.push((r.addr, r.value));
            }
        }
    };
    if let Some(j) = (0..si)
        .rev()
        .find(|&j| slice::effects(&t.steps[j]).is_some_and(|e| e.iter().any(|(l, _)| *l == reg)))
    {
        keep(
            &slice::slice(t, j, reg, slice::Options { branches: false }),
            &mut reads,
        );
        if let Some(a) = t.steps[j].addr.filter(|a| *a < 0x800)
            && let Some(w) = t.writes_of(a).into_iter().rfind(|w| *w < j)
        {
            keep(
                &slice::slice(
                    t,
                    w,
                    Loc::Mem(slice::fold(a)),
                    slice::Options { branches: false },
                ),
                &mut reads,
            );
        }
    }
    if let Some((a, _)) = source
        && let Some(w) = t.writes_of(a).into_iter().rfind(|w| *w < si)
    {
        keep(
            &slice::slice(
                t,
                w,
                Loc::Mem(slice::fold(a)),
                slice::Options { branches: false },
            ),
            &mut reads,
        );
    }
    let reg_value = match s.instr() {
        tetanes_core::cpu::instr::Instr::LDA
        | tetanes_core::cpu::instr::Instr::LDX
        | tetanes_core::cpu::instr::Instr::LDY => s.value,
        tetanes_core::cpu::instr::Instr::CPX => s.x,
        tetanes_core::cpu::instr::Instr::CPY => s.y,
        _ => s.a,
    };
    let mut flips = Vec::new();
    for v in 0..=255u8 {
        if let Some(f) = flags_for(s.op, constant, v)
            && taken_by(op, f) == Some(want_taken)
        {
            flips.push(v);
        }
    }
    Some(Guard {
        op,
        want_taken,
        setter: s.pc,
        constant,
        source,
        indirect,
        reg_value,
        chain: copy_chain(t, si, reg),
        flips,
        reads,
    })
}

/// The RAM bytes a register's value was copied through before step `at`:
/// back through transfers, pulls (to the push of the same value), loads
/// and the stores that filled the bytes loaded, whatever call they ran in.
fn copy_chain(t: &slice::FrameTrace, at: usize, reg: Loc) -> Vec<u16> {
    use tetanes_core::cpu::instr::Instr as I;
    let defines = |j: usize, l: Loc| {
        slice::effects(&t.steps[j]).is_some_and(|e| e.iter().any(|(x, _)| *x == l))
    };
    let mut out = Vec::new();
    let (mut j, mut loc) = (at, reg);
    for _ in 0..16 {
        let Some(k) = (0..j).rev().find(|&k| defines(k, loc)) else {
            break;
        };
        let st = &t.steps[k];
        match (loc, st.instr()) {
            (Loc::A, I::TXA) => loc = Loc::X,
            (Loc::A, I::TYA) => loc = Loc::Y,
            (Loc::X, I::TAX) | (Loc::Y, I::TAY) => loc = Loc::A,
            (Loc::A, I::PLA) => {
                // The push of the value pulled.
                let v = st.pulled;
                let Some(p) = (0..k)
                    .rev()
                    .find(|&p| t.steps[p].instr() == I::PHA && t.steps[p].a == v)
                else {
                    break;
                };
                j = p;
                loc = Loc::A;
                continue;
            }
            (_, I::LDA | I::LDX | I::LDY) => {
                let Some(ad) = st.addr.filter(|a| *a < 0x800) else {
                    break;
                };
                out.push(ad);
                // The store that filled it, if this frame made one.
                let Some(w) = t.writes_of(ad).into_iter().rfind(|w| *w < k) else {
                    break;
                };
                let ws = &t.steps[w];
                loc = match ws.instr() {
                    I::STA => Loc::A,
                    I::STX => Loc::X,
                    I::STY => Loc::Y,
                    _ => break,
                };
                j = w;
                continue;
            }
            _ => break,
        }
        j = k;
    }
    out
}

/// The grid byte a guard's compared value came from: its source if that
/// is on the grid, else a grid byte the slice read holding the same value
/// (a tile loaded, kept in a scratch byte, then compared).
fn grid_source(g: &Guard, grid: &Grid) -> Option<u16> {
    if let Some((ad, _)) = g.source
        && grid.place(ad).is_some()
    {
        return Some(ad);
    }
    if let Some(ad) = g.chain.iter().find(|ad| grid.place(**ad).is_some()) {
        return Some(*ad);
    }
    g.reads
        .iter()
        .find(|(ad, v)| *v == g.reg_value && grid.place(*ad).is_some())
        .map(|(ad, _)| *ad)
}

pub(crate) fn run(a: &LocalizeArgs, paths: &Paths) -> Result<Outcome> {
    let t0 = std::time::Instant::now();
    if let Some(g) = &a.game
        && !crate::mechanic::name_ok(g)
    {
        bail!("a game name is lower-case letters, digits and dashes: {g}");
    }
    // The game's package in the games checkout, when it is one.
    let game_dir = a.game.as_ref().and_then(|g| {
        crate::roms_cmd::games_dir(None).and_then(|d| crate::labels::game_folder(&d, g))
    });
    let rom = match (&a.rom, &game_dir, &a.game) {
        (Some(r), _, _) => fs::read(r).with_context(|| format!("reading {}", r.display()))?,
        (None, Some(d), _) => crate::clip_score::rom_of(paths, d, None)?,
        (None, None, Some(g)) => bail!("no game {g} in the games checkout: give --rom"),
        (None, None, None) => bail!("give a game or --rom"),
    };
    if crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade {
        bail!(
            "gp localize reads the NES's 6502: each branch's direction and the writer slice back from it. An arcade romset needs the same over the main Z80 (the conditional jumps' directions are in gp-arcade's instruction trace; a slice back from a jump's flags, as gp element inputs slices back from a write) and a level grid in the mapping"
        );
    }
    let sha = gp_evidence::source::identify_nes(&rom)?.sha256;
    // A game with observe.toml, or the ROM alone (keyed by its hash).
    let templated = game_dir
        .as_ref()
        .filter(|d| d.join("observe.toml").is_file());
    let name = a
        .game
        .clone()
        .unwrap_or_else(|| format!("rom-{}", &sha[..12]));
    let a = &LocalizeArgs {
        game: Some(name.clone()),
        level: if templated.is_some() {
            a.level.clone()
        } else {
            "all".to_owned()
        },
        ..a.clone()
    };
    let level = match templated {
        Some(d) => LevelKey::load(d)?,
        None => LevelKey::any(),
    };
    let grid = templated.and_then(|d| Grid::load(d));
    let dataset = paths.home.join("work/datasets").join(&name);
    let mut srcs = sources(paths, &name, &sha)?;
    if srcs.is_empty() && templated.is_none() {
        // Nothing has played this ROM: a blind run first.
        let o = crate::farm::blind::Opts {
            decide: a.decide.clone(),
            rounds: 100_000,
            top: 3,
            unsure: 0.4,
            frontier: 256,
            seed: 1,
            seconds: 120.0,
            levels: 1,
            session: None,
        };
        let dir = paths.home.join("work/farm/blind").join(&name);
        // The blind run reads a ROM file: the bytes found, in a file for it.
        let rom_file = tempfile::Builder::new().suffix(".nes").tempfile()?;
        fs::write(rom_file.path(), &rom)?;
        crate::farm::blind::farm_run(rom_file.path(), &o, paths, &dir)?;
        srcs = sources(paths, &name, &sha)?;
    }
    let player = if dataset.join("data").is_dir() {
        player_of(&dataset)?
    } else if let Some(p) = game_dir.as_deref().and_then(mapped_player) {
        p
    } else {
        blind_player(&rom, paths, &srcs)?
    };
    if let (Some(f), Some(from), Some(at)) = (&a.collect, a.from_frame, a.at) {
        let log = InputLog::parse(&fs::read_to_string(f)?).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut rows = Vec::new();
        let mut text = String::new();
        for way in ["uniform", "undirected", "laya"] {
            let r = collect(
                &rom,
                &log.frames,
                from,
                at,
                &player,
                &level,
                &a.level,
                way,
                a.budget,
                &a.decide,
            )?;
            text.push_str(&format!(
                "{way}: {} in {} moves, {} frames, {} calls, {:.1} s{}\n",
                if r["gained"] == json!(true) {
                    "GAINED"
                } else {
                    "not gained"
                },
                r["moves"],
                r["frames"],
                r["calls"],
                r["seconds"].as_f64().unwrap_or(0.0),
                r["log"]
                    .as_str()
                    .map(|l| format!(", run {l}"))
                    .unwrap_or_default()
            ));
            rows.push(r);
        }
        return Ok(Outcome::ok(json!({"collect": rows}), text));
    }
    if let (Some(f), Some((b, want))) = (&a.check_log, a.branch) {
        let log = InputLog::parse(&fs::read_to_string(f)?).map_err(|e| anyhow::anyhow!("{e}"))?;
        let v = check_run(
            &rom,
            &log.frames,
            gp_evidence::manifest::PowerOn::default(),
            b,
            want,
            a.watch,
            &player,
            &level,
            &a.level,
            240,
        )?;
        let tas = match &a.grade {
            Some(rec) => {
                let store = gp_evidence::store::Store::open(&paths.store)?;
                let entry = gp_evidence::recordings::resolve(&store, rec)?;
                let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
                let tl = gp_oracle::states::input_log(&store, &manifest)?;
                check_run(
                    &rom,
                    &tl.frames,
                    manifest.power_on,
                    b,
                    want,
                    a.watch,
                    &player,
                    &level,
                    &a.level,
                    0,
                )?
            }
            None => Value::Null,
        };
        let out = json!({"run": v, "recording": tas});
        return Ok(Outcome::ok(
            out.clone(),
            serde_json::to_string_pretty(&out)?,
        ));
    }
    if srcs.is_empty() {
        bail!("no clips or runs of {name} to read");
    }

    // 1. Branches, both ways, in the level.
    let mut seen: HashMap<u16, Seen> = HashMap::new();
    let mut frames_in = 0usize;
    // Grid values seen in the level: (address) -> {value -> (source, frame, player x)}.
    let mut grid_seen: HashMap<u16, BTreeMap<u8, (usize, usize, i64)>> = HashMap::new();
    let mut src_max_x: Vec<i64> = Vec::new();
    // Code that ran in the level, one bit an address.
    let mut exec = vec![0u64; 1024];
    for (si, s) in srcs.iter().enumerate() {
        let mut nes = gp_nes::Nes::power_on(&rom, gp_evidence::manifest::PowerOn::default())?;
        nes.set_video(false);
        let mut in_level = false;
        let mut max_x = 0;
        for (fi, f) in s.frames.iter().enumerate() {
            let mut prev: Option<u16> = None;
            let mut hits: Vec<(u16, bool)> = Vec::new();
            let here = in_level;
            nes.step_with(*f, |bus| {
                let pc = bus.cpu.pc;
                if here {
                    exec[usize::from(pc >> 6)] |= 1 << (pc & 63);
                }
                if let Some(p) = prev
                    && gp_nes::coverage::is_branch(bus.peek(p))
                    && let Some(t) = went(bus, p, pc)
                {
                    hits.push((p, t));
                }
                prev = Some(pc);
            })?;
            if in_level {
                frames_in += 1;
                for (p, t) in hits {
                    let e = seen.entry(p).or_default();
                    if t {
                        e.taken += 1;
                    } else {
                        e.fell += 1;
                    }
                    if e.at.len() < 12 && e.at.last().is_none_or(|l| l.1 + 30 < fi || l.0 != si) {
                        e.at.push((si, fi));
                    }
                }
            }
            let w = &nes.wram()[..0x800];
            in_level = level.of(w).is_some_and(|l| l == a.level);
            if in_level {
                let px = player.x(w);
                max_x = max_x.max(px);
                if let Some(g) = &grid
                    && fi % 8 == 0
                {
                    for &base in &g.pages {
                        for i in 0..(g.cols * g.rows) {
                            let ad = base + i as u16;
                            grid_seen
                                .entry(ad)
                                .or_default()
                                .entry(w[usize::from(ad)])
                                .or_insert((si, fi, px));
                        }
                    }
                }
            }
        }
        src_max_x.push(max_x);
    }
    let one_way: Vec<(u16, Seen)> = {
        let mut v: Vec<(u16, Seen)> = seen
            .iter()
            .filter(|(_, s)| (s.taken == 0) != (s.fell == 0))
            .map(|(p, s)| (*p, s.clone()))
            .collect();
        v.sort_by_key(|(p, _)| *p);
        v
    };

    // 2. Guards, from each one-way branch's first run.
    let peek_nes = gp_nes::Nes::power_on(&rom, gp_evidence::manifest::PowerOn::default())?;
    let mut by_frame: BTreeMap<(usize, usize), Vec<u16>> = BTreeMap::new();
    for (p, s) in &one_way {
        if let Some(&at) = s.at.first() {
            by_frame.entry(at).or_default().push(*p);
        }
    }
    let mut guards: HashMap<u16, Guard> = HashMap::new();
    for ((si, fi), pcs) in &by_frame {
        let t = record_frame(&rom, &srcs[*si], *fi)?;
        for pc in pcs {
            let s = &seen[pc];
            if let Some(g) = guard_of(&t, *pc, peek_nes.peek(*pc), s.taken == 0) {
                guards.insert(*pc, g);
            }
        }
    }

    // The code behind each one-way branch's unrun side: instructions no
    // run executed, reached from there (calls followed, stopping at code
    // that ran). More code behind it, a bigger thing the runs haven't seen.
    let ran = |pc: u16| exec[usize::from(pc >> 6)] >> (pc & 63) & 1 == 1;
    let behind: HashMap<u16, usize> = one_way
        .iter()
        .map(|(p, s)| {
            let fall = p.wrapping_add(2);
            let to = fall.wrapping_add(peek_nes.peek(p.wrapping_add(1)) as i8 as u16);
            let start = if s.taken == 0 { to } else { fall };
            (*p, unrun_behind(&peek_nes, start, &ran))
        })
        .collect();
    let mut by_code: Vec<(u16, usize)> = behind.iter().map(|(p, n)| (*p, *n)).collect();
    by_code.sort_by_key(|(p, n)| (std::cmp::Reverse(*n), *p));

    // 3. Localize: guards on a grid byte, against the grid as the runs saw it.
    let mut leads: Vec<Value> = Vec::new();
    let mut places: Vec<(f64, u16, Value)> = Vec::new();
    for (pc, s) in &one_way {
        let g = guards.get(pc);
        let mut lead = json!({
            "branch": format!("{pc:04X}"),
            "unrun_behind": behind.get(pc).copied().unwrap_or(0),
            "went": if s.taken > 0 { "only taken" } else { "only fell through" },
            "runs": s.taken + s.fell,
        });
        let Some(g) = g else {
            lead["guard"] = json!("not read");
            leads.push(lead);
            continue;
        };
        lead["guard"] = json!({
            "set_by": format!("{:04X}", g.setter),
            "constant": g.constant,
            "source": g.source.map(|(ad, v)| format!("{ad:04X}={v}")),
            "grid_source": grid.as_ref().and_then(|gr| grid_source(g, gr)).map(|ad| format!("{ad:04X}")),
            "value": g.reg_value,
            "through_pointer": g.indirect,
            "flips_on": g.flips.len(),
            "reads": g.reads.iter().take(6).map(|(ad, v)| format!("{ad:04X}={v}")).collect::<Vec<_>>(),
        });
        let on_grid = grid.as_ref().and_then(|gr| grid_source(g, gr));
        if let (Some(gr), Some(_)) = (&grid, on_grid)
            && !g.flips.is_empty()
            && g.flips.len() < 256
        {
            // Cells that held a flipping value while the level was played.
            let flips: BTreeSet<u8> = g.flips.iter().copied().collect();
            let mut cells: Vec<Value> = Vec::new();
            for (ad, vals) in &grid_seen {
                for (v, (si, fi, px)) in vals {
                    if flips.contains(v)
                        && let Some((page, row, col)) = gr.place(*ad)
                    {
                        let lc = gr.level_col(page, col, *px + 128);
                        cells.push(
                            json!({"addr": format!("{ad:04X}"), "value": v, "level_x": lc * 16,
                            "y": gr.row_y(row), "first": [srcs[*si].name, fi]}),
                        );
                    }
                }
            }
            // How the guard met its cell each time it ran: where that cell
            // sat from the player, and whether the player was rising.
            let mut offs: Vec<(i64, i64, i64)> = Vec::new();
            let mut need: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
            for &(si, fi) in &s.at {
                need.entry(si).or_default().insert(fi);
            }
            for (si, frames) in &need {
                let last = *frames.iter().max().expect("a frame");
                let mut nes =
                    gp_nes::Nes::power_on(&rom, gp_evidence::manifest::PowerOn::default())?;
                nes.set_video(false);
                let mut ys: Vec<i64> = Vec::new();
                for (k, f) in srcs[*si].frames[..=last].iter().enumerate() {
                    if !frames.contains(&k) {
                        nes.step(*f)?;
                        ys.push(player.y(&nes.wram()[..0x800]));
                        continue;
                    }
                    let mut rec = Recorder::new();
                    rec.start(nes.bus_mut_for_observers());
                    nes.step_with(*f, |bus| rec.before(bus))?;
                    let t = rec.finish();
                    let w = &nes.wram()[..0x800];
                    let (px, py) = (player.x(w), player.y(w));
                    ys.push(py);
                    let vy = py - ys.get(k.saturating_sub(2)).copied().unwrap_or(py);
                    // Each time the branch ran this frame: the cell it
                    // compared, taking the one nearest the player.
                    let mut best: Option<(i64, i64)> = None;
                    for (bi, st) in t.steps.iter().enumerate() {
                        if st.pc != *pc {
                            continue;
                        }
                        let Some(gx) = guard_at(&t, bi, g.op, g.want_taken) else {
                            continue;
                        };
                        let Some(ad) = grid_source(&gx, gr) else {
                            continue;
                        };
                        let Some((page, row, col)) = gr.place(ad) else {
                            continue;
                        };
                        let lc = gr.level_col(page, col, px);
                        let o = (lc * 16 - px, gr.row_y(row) - py);
                        if best.is_none_or(|b| o.0.abs() + o.1.abs() < b.0.abs() + b.1.abs()) {
                            best = Some(o);
                        }
                    }
                    if let Some(o) = best {
                        offs.push((o.0, o.1, vy));
                    }
                }
            }
            // The samples nearest the player: the guard may also run for
            // other things (enemies) in the same code.
            offs.sort_by_key(|o| o.0.abs() + o.1.abs());
            offs.truncate(offs.len().div_ceil(2).max(1));
            let med = |mut v: Vec<i64>| {
                v.sort_unstable();
                v.get(v.len() / 2).copied().unwrap_or(0)
            };
            let (ox, oy, vy) = (
                med(offs.iter().map(|o| o.0).collect()),
                med(offs.iter().map(|o| o.1).collect()),
                med(offs.iter().map(|o| o.2).collect()),
            );
            lead["meets"] = json!({"cell_from_player_x": ox, "cell_from_player_y": oy,
                "player_moving_y": vy, "samples": offs.len()});
            lead["cells"] = json!(cells);
            if !cells.is_empty() {
                // Fewer flipping values and fewer cells: a more specific lead.
                let score = 1.0 / (g.flips.len() as f64 * cells.len() as f64);
                places.push((score, *pc, lead.clone()));
            }
        }
        leads.push(lead);
    }
    places.sort_by(|x, y| y.0.total_cmp(&x.0));
    let ranked: Vec<Value> = places.iter().map(|p| p.2.clone()).collect();

    // 4. Probes.
    let mut probes: Vec<Value> = Vec::new();
    if !a.no_probe {
        let best_src = (0..srcs.len())
            .max_by_key(|&i| src_max_x[i])
            .context("a source")?;
        for (_, pc, lead) in places.iter().take(a.probes) {
            let g = &guards[pc];
            for cell in lead["cells"].as_array().into_iter().flatten().take(2) {
                let goal = Goal {
                    branch: *pc,
                    want_taken: g.want_taken,
                    also: Vec::new(),
                    cell: cell["addr"]
                        .as_str()
                        .and_then(|a| u16::from_str_radix(a, 16).ok())
                        .zip(cell["value"].as_u64().map(|v| v as u8)),
                    x: cell["level_x"].as_i64().unwrap_or(0),
                    y: cell["y"].as_i64().unwrap_or(0),
                    ox: lead["meets"]["cell_from_player_x"].as_i64().unwrap_or(0),
                    oy: lead["meets"]["cell_from_player_y"].as_i64().unwrap_or(0),
                    rising: lead["meets"]["player_moving_y"].as_i64().unwrap_or(0) < 0,
                };
                // The runs met the guard's cell at the player's feet or
                // level with them, which a cell up in the air can't be
                // from the ground: meet it from below instead, the
                // player's top just under it, rising.
                let goal = if goal.oy >= 0 {
                    Goal {
                        ox: 0,
                        oy: -16,
                        rising: true,
                        ..goal
                    }
                } else {
                    goal
                };
                let start = start_before(
                    &rom,
                    &srcs[best_src],
                    &player,
                    &level,
                    &a.level,
                    goal.x - goal.ox,
                )?;
                let Some(start) = start else {
                    probes.push(json!({"branch": format!("{pc:04X}"), "cell": cell, "error": "no run reaches the place"}));
                    continue;
                };
                for way in ["laya", "uniform", "undirected"] {
                    let r = probe(
                        &rom, &start, &goal, &player, &level, &a.level, way, a.budget, &a.decide,
                    )?;
                    let mut r = r;
                    r["branch"] = json!(format!("{pc:04X}"));
                    r["cell"] = cell.clone();
                    probes.push(r);
                }
            }
        }
    }

    // 4b. The RAM-poke sweep.
    let mut pokes = Value::Null;
    if a.poke {
        let best_src = (0..srcs.len())
            .max_by_key(|&i| src_max_x[i])
            .context("a source")?;
        let flips: HashMap<u16, bool> = one_way.iter().map(|(p, s)| (*p, s.taken == 0)).collect();
        let placed: BTreeSet<(i64, i64)> = places
            .iter()
            .flat_map(|p| p.2["cells"].as_array().cloned().unwrap_or_default())
            .map(|c| {
                (
                    c["level_x"].as_i64().unwrap_or(0),
                    c["y"].as_i64().unwrap_or(0),
                )
            })
            .collect();
        pokes = poke_sweep(
            &rom,
            &srcs[best_src],
            &player,
            &level,
            &a.level,
            grid.as_ref(),
            &flips,
            &exec,
            a,
            &guards,
            &placed,
            &behind,
        )?;
    }

    // 5. The grader's part.
    let mut graded = Value::Null;
    if let Some(rec) = &a.grade {
        graded = grade(
            paths, &rom, rec, &places, &guards, &player, &level, &a.level, a.watch, &probes,
        )?;
    }

    let v = json!({
        "game": a.game, "level": a.level, "sources": srcs.len(), "frames_in_level": frames_in,
        "branches_seen": seen.len(), "one_way": one_way.len(), "guards_read": guards.len(),
        "placed": ranked.len(), "ranked": ranked,
        "by_code": by_code.iter().take(20).map(|(p, n)| json!({"branch": format!("{p:04X}"), "unrun_behind": n})).collect::<Vec<_>>(), "probes": probes, "pokes": pokes, "graded": graded,
        "seconds": (t0.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    });
    let out = paths.home.join("work/localize").join(&name);
    fs::create_dir_all(&out)?;
    fs::write(
        out.join(format!("{}.json", a.level)),
        serde_json::to_string_pretty(&json!({"summary": v, "leads": leads}))?,
    )?;
    let mut text = format!(
        "{} sources, {} frames in {}: {} branches, {} one way, {} guards read, {} placed on the grid ({:.0} s)",
        srcs.len(),
        frames_in,
        a.level,
        seen.len(),
        one_way.len(),
        guards.len(),
        ranked.len(),
        t0.elapsed().as_secs_f64()
    );
    text.push_str("\nby code behind the unrun side:");
    for (i, (p, n)) in by_code.iter().take(8).enumerate() {
        text.push_str(&format!(
            "\n  {}. branch {p:04X}: {n} instructions no run executed",
            i + 1
        ));
    }
    for (i, r) in ranked.iter().take(8).enumerate() {
        text.push_str(&format!(
            "\n{}. branch {} {}: {} on {} ({} values flip it), {} cell(s), first at level x {}; meets it {} px across, {} px down, moving {}",
            i + 1, r["branch"].as_str().unwrap_or(""), r["went"].as_str().unwrap_or(""),
            r["guard"]["set_by"].as_str().unwrap_or(""), r["guard"]["source"].as_str().unwrap_or(""),
            r["guard"]["flips_on"], r["cells"].as_array().map_or(0, Vec::len),
            r["cells"][0]["level_x"], r["meets"]["cell_from_player_x"], r["meets"]["cell_from_player_y"],
            r["meets"]["player_moving_y"],
        ));
    }
    for p in &probes {
        text.push_str(&format!(
            "\nprobe {} {}: {} in {} moves, {} frames, {} calls, {:.1} s",
            p["branch"].as_str().unwrap_or(""),
            p["way"].as_str().unwrap_or(""),
            if p["triggered"] == json!(true) {
                "TRIGGERED"
            } else {
                "not triggered"
            },
            p["moves"],
            p["frames"],
            p["calls"],
            p["seconds"].as_f64().unwrap_or(0.0),
        ));
    }
    if !graded.is_null() {
        text.push_str(&format!("\ngrade: {graded}"));
    }
    Ok(Outcome::ok(v, text))
}

/// A probe's goal: meet grid cell (x, y) as the guard of `branch` does.
struct Goal {
    branch: u16,
    want_taken: bool,
    /// Other branches that count as meeting it, each with its way.
    also: Vec<(u16, bool)>,
    x: i64,
    y: i64,
    ox: i64,
    oy: i64,
    rising: bool,
    /// The cell's grid byte and its value: when set, the goal is met when
    /// that byte changes (a lasting change at the place), not when the
    /// branch flips.
    cell: Option<(u16, u8)>,
}

/// A savestate to start probing from.
struct Start {
    snap: Vec<u8>,
    frame: u64,
    held: [u8; 4],
    log: Vec<InputFrame>,
}

/// The last frame of `src` in the level, standing, with the player's x
/// below `x` (and at least 32 px before it).
fn start_before(
    rom: &[u8],
    src: &Src,
    player: &Player,
    level: &LevelKey,
    name: &str,
    x: i64,
) -> Result<Option<Start>> {
    let mut nes = gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
    nes.set_video(false);
    let mut best: Option<Start> = None;
    let mut py = -1;
    for (i, f) in src.frames.iter().enumerate() {
        nes.step(*f)?;
        let w = &nes.wram()[..0x800];
        if level.of(w).is_none_or(|l| l != name) {
            continue;
        }
        let (px, y) = (player.x(w), player.y(w));
        if px < x - 32 && px > x - 160 && y == py {
            best = Some(Start {
                snap: nes.snapshot()?,
                frame: nes.frame(),
                held: nes.held(),
                log: src.frames[..=i].to_vec(),
            });
        }
        if px >= x {
            break;
        }
        py = y;
    }
    Ok(best)
}

struct Arch {
    snap: Vec<u8>,
    frame: u64,
    held: [u8; 4],
    log: Vec<InputFrame>,
    d: i64,
    visits: u32,
    x: i64,
    y: i64,
    ground: bool,
}

/// The probe's moves: gp farm's, plus small steps and jumps in place.
fn probe_moves() -> Vec<Move> {
    let mut v = farm::moves(3, 14, 30);
    let mv = |name, about, press: Vec<(u8, usize)>, then, settle| Move {
        name,
        about,
        press,
        then,
        settle,
    };
    v.push(mv(
        "nudge_right",
        "step right a few pixels",
        vec![(farm::RIGHT, 4)],
        0,
        0,
    ));
    v.push(mv(
        "nudge_left",
        "step left a few pixels",
        vec![(farm::LEFT, 4)],
        0,
        0,
    ));
    v.push(mv(
        "hop_up",
        "a small hop straight up",
        vec![(0, 1), (farm::A, 4)],
        0,
        90,
    ));
    v.push(mv(
        "mid_up",
        "a medium jump straight up",
        vec![(0, 1), (farm::A, 14)],
        0,
        90,
    ));
    v
}

#[allow(clippy::too_many_arguments)]
fn probe(
    rom: &[u8],
    start: &Start,
    goal: &Goal,
    player: &Player,
    level: &LevelKey,
    name: &str,
    way: &str,
    budget: usize,
    decide: &str,
) -> Result<Value> {
    let t0 = std::time::Instant::now();
    let moves = probe_moves();
    let mut nes = gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
    nes.set_video(false);
    nes.restore(&start.snap, start.frame, start.held)?;
    let dist = |x: i64, y: i64, vy: i64| -> i64 {
        let d = ((goal.x - x) - goal.ox).abs() + ((goal.y - y) - goal.oy).abs();
        d + if goal.rising && vy >= 0 { 8 } else { 0 }
    };
    let w0 = nes.wram()[..0x800].to_vec();
    let (x0, y0) = (player.x(&w0), player.y(&w0));
    let mut arch: BTreeMap<(i64, i64, bool), Arch> = BTreeMap::new();
    arch.insert(
        (x0 / 8, y0 / 8, true),
        Arch {
            snap: start.snap.clone(),
            frame: start.frame,
            held: start.held,
            log: start.log.clone(),
            d: dist(x0, y0, 0),
            visits: 0,
            x: x0,
            y: y0,
            ground: true,
        },
    );
    let mut rng = Rng(0x9E37_79B9);
    let (mut sims, mut frames, mut calls) = (0usize, 0u64, 0usize);
    let mut flips_seen = 0usize;
    let criteria: serde_json::Map<String, Value> = moves
        .iter()
        .map(|mv| (mv.name.to_owned(), json!(mv.about)))
        .collect();
    while sims < budget {
        // Pick: the nearest few to the goal, less visited first; undirected
        // takes the least visited of all.
        let keys: Vec<(i64, i64, bool)> = arch.keys().copied().collect();
        let key = if way == "undirected" {
            *keys
                .iter()
                .min_by_key(|k| (arch[k].visits, rng.next() % 1000))
                .context("a cell")?
        } else {
            let mut ks = keys.clone();
            ks.sort_by_key(|k| (arch[k].d, arch[k].visits));
            let n = ks.len().min(6);
            let weights: Vec<f64> = ks[..n]
                .iter()
                .map(|k| 1.0 / (1.0 + f64::from(arch[k].visits)))
                .collect();
            let total: f64 = weights.iter().sum();
            let mut r = rng.unit() * total;
            let mut pick = ks[0];
            for (k, wt) in ks[..n].iter().zip(&weights) {
                if r < *wt {
                    pick = *k;
                    break;
                }
                r -= wt;
            }
            pick
        };
        let (snap, frame, held, log, cx, cy, cg) = {
            let c = arch.get_mut(&key).expect("a cell");
            c.visits += 1;
            (
                c.snap.clone(),
                c.frame,
                c.held,
                c.log.clone(),
                c.x,
                c.y,
                c.ground,
            )
        };
        let order: Vec<&'static str> = if way == "laya" {
            let st = json!({
                "target": format!("The player's head should meet the spot {} px ahead and {} px up", (goal.x - cx) - goal.ox, -((goal.y - cy) - goal.oy)),
                "target_ahead_px": (goal.x - cx) - goal.ox,
                "target_up_px": -((goal.y - cy) - goal.oy),
                "moving_up_when_meeting": goal.rising,
                "on_ground": cg,
            });
            let ask = json!({"model": "laya", "state": st, "questions": {"move": {"type": "choice",
                "instructions": "Which move brings the player to meet the target spot the way described?", "criteria": criteria}}});
            let ans = farm::post(
                &format!("{}/v1/systemone", decide.trim_end_matches('/')),
                &ask,
            )?;
            calls += 1;
            let pr = &ans["answers"]["move"]["probabilities"];
            let mut o: Vec<(&'static str, f64)> = moves
                .iter()
                .map(|mv| (mv.name, pr[mv.name].as_f64().unwrap_or(0.0)))
                .collect();
            o.sort_by(|a, b| b.1.total_cmp(&a.1));
            o.into_iter().map(|(n, _)| n).collect()
        } else {
            let mut o: Vec<&'static str> = moves.iter().map(|mv| mv.name).collect();
            for i in (1..o.len()).rev() {
                o.swap(i, (rng.next() % (i as u64 + 1)) as usize);
            }
            o
        };
        for name_mv in order.iter().take(4) {
            if sims >= budget {
                break;
            }
            let mv = moves.iter().find(|m| m.name == *name_mv).expect("a move");
            nes.restore(&snap, frame, held)?;
            sims += 1;
            let mut l = log.clone();
            let r = play(&mut nes, mv, &mut l, player, level, name, goal)?;
            if r.flipped {
                flips_seen += 1;
            }
            frames += r.frames;
            if r.triggered {
                let w = nes.wram()[..0x800].to_vec();
                if way != "undirected" || TRIGGER_LOG.with(|t| t.borrow().is_none()) {
                    TRIGGER_LOG.with(|t| *t.borrow_mut() = Some(l.clone()));
                }
                // The run that did it, from power-on, for the clip.
                let mut log = InputLog::new(1);
                log.frames = l.clone();
                let dir = std::env::var_os("GP_HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                            .join(".gameprinter")
                    })
                    .join("work/localize/runs");
                let _ = fs::create_dir_all(&dir);
                let f = dir.join(format!("{:04X}-{way}-{}.log", goal.branch, l.len()));
                let _ = fs::write(&f, log.to_text());
                return Ok(
                    json!({"way": way, "triggered": true, "branch_flips": flips_seen, "moves": sims, "frames": frames, "calls": calls,
                    "seconds": t0.elapsed().as_secs_f64(), "at_frame": l.len(),
                    "player_x": player.x(&w), "player_y": player.y(&w), "log": f.display().to_string(),
                    "start_frame": start.log.len()}),
                );
            }
            if r.dead {
                continue;
            }
            let k = (r.x / 8, r.y / 8, r.ground);
            let d = r.best_d;
            let better = arch
                .get(&k)
                .is_none_or(|c| d < c.d || l.len() < c.log.len());
            if better {
                arch.insert(
                    k,
                    Arch {
                        snap: nes.snapshot()?,
                        frame: nes.frame(),
                        held: nes.held(),
                        log: l,
                        d: dist(r.x, r.y, 0).min(d),
                        visits: 0,
                        x: r.x,
                        y: r.y,
                        ground: r.ground,
                    },
                );
            }
        }
    }
    let best = arch.values().map(|c| c.d).min().unwrap_or(i64::MAX);
    Ok(
        json!({"way": way, "triggered": false, "branch_flips": flips_seen, "moves": sims, "frames": frames, "calls": calls,
        "seconds": t0.elapsed().as_secs_f64(), "closest_px": best, "cells": arch.len()}),
    )
}

thread_local! {
    /// The input log of the last probe that triggered, for the grader.
    static TRIGGER_LOG: std::cell::RefCell<Option<Vec<InputFrame>>> = const { std::cell::RefCell::new(None) };
}

struct Played {
    flipped: bool,
    frames: u64,
    triggered: bool,
    dead: bool,
    x: i64,
    y: i64,
    ground: bool,
    best_d: i64,
}

/// Plays one move, watching the goal's branch; `log` gets the inputs.
fn play(
    nes: &mut gp_nes::Nes,
    mv: &Move,
    log: &mut Vec<InputFrame>,
    player: &Player,
    level: &LevelKey,
    name: &str,
    goal: &Goal,
) -> Result<Played> {
    let mut frames = 0u64;
    let mut triggered = false;
    let mut flipped = false;
    let w = nes.wram()[..0x800].to_vec();
    let (mut px, mut py) = (player.x(&w), player.y(&w));
    let mut best_d = i64::MAX;
    let mut still = 0;
    let mut left = false;
    let mut steps: Vec<u8> = Vec::new();
    for &(pad, n) in &mv.press {
        steps.extend(std::iter::repeat_n(pad, n));
    }
    let settle = mv.settle;
    let total = steps.len() + settle;
    for i in 0..total {
        let pad = steps.get(i).copied().unwrap_or(mv.then);
        let f = InputFrame {
            polled: true,
            pads: [pad, 0, 0, 0],
            reset: false,
        };
        let mut prev: Option<u16> = None;
        nes.step_with(f, |bus| {
            let pc = bus.cpu.pc;
            if let Some(p) = prev
                && (p == goal.branch && went(bus, p, pc) == Some(goal.want_taken)
                    || goal
                        .also
                        .iter()
                        .any(|(b, w)| *b == p && went(bus, p, pc) == Some(*w)))
            {
                flipped = true;
            }
            prev = Some(pc);
        })?;
        log.push(f);
        frames += 1;
        let w = &nes.wram()[..0x800];
        match goal.cell {
            Some((ad, v)) => {
                if w[usize::from(ad)] != v {
                    triggered = true;
                }
            }
            // No cell byte to watch: the flip counts near the pose only.
            None => {
                let (x, y) = (player.x(w), player.y(w));
                let near = ((goal.x - x) - goal.ox).abs() + ((goal.y - y) - goal.oy).abs() <= 24;
                triggered |= flipped && near;
                flipped = false;
            }
        }
        if level.of(w).is_none_or(|l| l != name) {
            return Ok(Played {
                flipped,
                frames,
                triggered,
                dead: !triggered,
                x: px,
                y: py,
                ground: false,
                best_d,
            });
        }
        let (x, y) = (player.x(w), player.y(w));
        if y > 232 {
            return Ok(Played {
                flipped,
                frames,
                triggered,
                dead: !triggered,
                x,
                y,
                ground: false,
                best_d,
            });
        }
        let vy = y - py;
        let d = ((goal.x - x) - goal.ox).abs()
            + ((goal.y - y) - goal.oy).abs()
            + if goal.rising && vy >= 0 { 8 } else { 0 };
        best_d = best_d.min(d);
        still = if y == py { still + 1 } else { 0 };
        if still == 0 {
            left = true;
        }
        (px, py) = (x, y);
        if triggered {
            break;
        }
        if i >= steps.len() && left && still >= 2 {
            break;
        }
    }
    Ok(Played {
        flipped,
        frames,
        triggered,
        dead: false,
        x: px,
        y: py,
        ground: still >= 1,
        best_d,
    })
}

/// The grader's part: does `rec` send the placed branches the other way,
/// where, and what does the watched byte do; and does the probe's run
/// replay to the same from power-on.
#[allow(clippy::too_many_arguments)]
fn grade(
    paths: &Paths,
    rom: &[u8],
    rec: &str,
    places: &[(f64, u16, Value)],
    guards: &HashMap<u16, Guard>,
    player: &Player,
    level: &LevelKey,
    name: &str,
    watch: Option<u16>,
    probes: &[Value],
) -> Result<Value> {
    let store = gp_evidence::store::Store::open(&paths.store)?;
    let entry = gp_evidence::recordings::resolve(&store, rec)?;
    let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
    let log = gp_oracle::states::input_log(&store, &manifest)?;
    let mut nes = gp_nes::Nes::power_on(rom, manifest.power_on)?;
    nes.set_video(false);
    let targets: BTreeMap<u16, bool> = places
        .iter()
        .map(|p| (p.1, guards[&p.1].want_taken))
        .collect();
    let mut first: BTreeMap<u16, (usize, i64, i64)> = BTreeMap::new();
    let mut watched: Vec<(usize, u8)> = Vec::new();
    let mut last_w: Option<u8> = None;
    let mut seen_level = false;
    for (fi, f) in log.frames.iter().enumerate() {
        let mut prev: Option<u16> = None;
        let mut hit: Vec<u16> = Vec::new();
        nes.step_with(*f, |bus| {
            let pc = bus.cpu.pc;
            if let Some(p) = prev
                && let Some(want) = targets.get(&p)
                && went(bus, p, pc) == Some(*want)
            {
                hit.push(p);
            }
            prev = Some(pc);
        })?;
        let w = &nes.wram()[..0x800];
        let here = level.of(w).is_some_and(|l| l == name);
        if here {
            seen_level = true;
        } else if seen_level && level.of(w).is_some() {
            break;
        }
        for p in hit {
            first.entry(p).or_insert((fi + 1, player.x(w), player.y(w)));
        }
        if let Some(a) = watch {
            let v = w[usize::from(a)];
            if last_w != Some(v) {
                watched.push((fi + 1, v));
                last_w = Some(v);
            }
        }
    }
    // The probe's run from power-on: the watched byte before and after.
    let probe_check = TRIGGER_LOG.with(|t| t.borrow().clone()).map(|l| -> Result<Value> {
        let mut nes = gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
        nes.set_video(false);
        let mut changes: Vec<(usize, u8)> = Vec::new();
        let mut last: Option<u8> = None;
        let mut fired = None;
        for (fi, f) in l.iter().chain(std::iter::repeat_n(&InputFrame { polled: true, pads: [0; 4], reset: false }, 240)).enumerate() {
            let mut prev: Option<u16> = None;
            let mut hit = false;
            nes.step_with(*f, |bus| {
                let pc = bus.cpu.pc;
                if let Some(p) = prev
                    && let Some(want) = targets.get(&p)
                    && went(bus, p, pc) == Some(*want)
                {
                    hit = true;
                }
                prev = Some(pc);
            })?;
            let w = &nes.wram()[..0x800];
            if hit && fired.is_none() {
                fired = Some((fi + 1, player.x(w), player.y(w)));
            }
            if let Some(a) = watch {
                let v = w[usize::from(a)];
                if last != Some(v) {
                    changes.push((fi + 1, v));
                    last = Some(v);
                }
            }
        }
        Ok(json!({"replays_from_power_on": fired.is_some(), "fired_at": fired, "watched": changes.iter().rev().take(4).collect::<Vec<_>>()}))
    }).transpose()?;
    let _ = probes;
    Ok(json!({
        "recording": rec,
        "first_other_way": first.iter().map(|(p, (f, x, y))| json!({"branch": format!("{p:04X}"), "frame": f, "player_x": x, "player_y": y})).collect::<Vec<_>>(),
        "watched": watched.iter().filter(|(f, _)| *f > 0).take(12).collect::<Vec<_>>(),
        "probe_run": probe_check,
    }))
}

/// A byte that tracks a position's change: the one whose sign (as i8)
/// agrees with the change on the most frames where both are non-zero.
fn speed_byte(ws: &[Vec<u8>], pos: &dyn Fn(&[u8]) -> i64) -> Option<(u16, i8, i8)> {
    let mut best: Option<(u16, f64)> = None;
    for a in 0..0x800usize {
        let (mut agree, mut n) = (0usize, 0usize);
        for k in 1..ws.len() {
            let d = pos(&ws[k]) - pos(&ws[k - 1]);
            let v = ws[k - 1][a] as i8;
            if d == 0 || v == 0 || d.abs() > 8 {
                continue;
            }
            n += 1;
            if (d > 0) == (v > 0) {
                agree += 1;
            }
        }
        if n >= 30 {
            let r = agree as f64 / n as f64;
            if r > 0.9 && best.is_none_or(|b| r > b.1) {
                best = Some((a as u16, r));
            }
        }
    }
    let (a, _) = best?;
    let vals: Vec<i8> = ws.iter().map(|w| w[usize::from(a)] as i8).collect();
    Some((a, *vals.iter().min()?, *vals.iter().max()?))
}

#[allow(clippy::too_many_arguments)]
fn poke_sweep(
    rom: &[u8],
    src: &Src,
    player: &Player,
    level: &LevelKey,
    name: &str,
    grid: Option<&Grid>,
    flips: &HashMap<u16, bool>,
    exec: &[u64],
    a: &LocalizeArgs,
    guards: &HashMap<u16, Guard>,
    placed: &BTreeSet<(i64, i64)>,
    behind: &HashMap<u16, usize>,
) -> Result<Value> {
    let t0 = std::time::Instant::now();
    // With no level grid (a ROM alone), every 16 px cell of the screen.
    let lattice = Grid {
        pages: Vec::new(),
        cols: 16,
        rows: 15,
        top: 0,
    };
    let gr = grid.unwrap_or(&lattice);
    // The path, with a savestate a screen (256 px of x), standing.
    let mut nes = gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
    nes.set_video(false);
    let mut ws: Vec<Vec<u8>> = Vec::new();
    // (player x, savestate, frame, held, path frame)
    #[allow(clippy::type_complexity)]
    let mut screens: Vec<(i64, Vec<u8>, u64, [u8; 4], usize)> = Vec::new();
    let mut py = -1;
    for (i, f) in src.frames.iter().enumerate() {
        nes.step(*f)?;
        let w = nes.wram()[..0x800].to_vec();
        if level.of(&w).is_some_and(|l| l == name) {
            let (x, y) = (player.x(&w), player.y(&w));
            if y == py && screens.last().is_none_or(|s| x >= s.0 + 256) {
                screens.push((x, nes.snapshot()?, nes.frame(), nes.held(), i));
            }
            py = y;
            ws.push(w);
        }
    }
    let ys = speed_byte(&ws, &|w| player.y(w));
    let xs = speed_byte(&ws, &|w| player.x(w));
    let Some((vy_b, vy_min, vy_max)) = ys else {
        return Ok(json!({"error": "no byte tracks the player's vertical change"}));
    };
    let mut emulated = 0u64;
    let mut tries = 0usize;
    let mut hits: BTreeMap<(i64, i64, u16), Value> = BTreeMap::new();
    let ran = |pc: u16| exec[usize::from(pc >> 6)] >> (pc & 63) & 1 == 1;
    for (sx, snap, frame, held, _) in &screens {
        nes.restore(snap, *frame, *held)?;
        let w0 = nes.wram()[..0x800].to_vec();
        let col0 = sx / 16;
        for col in (col0 - 4)..(col0 + 12) {
            for row in 0..gr.rows {
                // The cell's grid byte, when there's a grid: an empty cell
                // is skipped.
                let ad = (!gr.pages.is_empty()).then(|| {
                    let page =
                        (col.rem_euclid((gr.cols * gr.pages.len()) as i64) as usize) / gr.cols;
                    usize::from(gr.pages[page])
                        + row * gr.cols
                        + (col.rem_euclid(gr.cols as i64) as usize)
                });
                if ad.is_some_and(|ad| w0[ad] == 0) {
                    continue;
                }
                let (cx, cy) = (col * 16, gr.row_y(row));
                // (direction, x, y, vertical speed, horizontal speed)
                let mut poses: Vec<(&str, i64, i64, i8, Option<i8>)> = Vec::new();
                for dx in [0, 6] {
                    for dy in [16, 24] {
                        poses.push(("below", cx + dx, cy + dy, vy_min, None));
                    }
                    poses.push(("above", cx + dx, cy - 40, vy_max, None));
                }
                if let Some((_, _, xmax)) = xs {
                    poses.push(("side", cx - 16, cy - 8, 0, Some(xmax)));
                }
                for (dir, x, y, vy, vx) in poses {
                    if !(0..256).contains(&y) || x < 0 {
                        continue;
                    }
                    nes.restore(snap, *frame, *held)?;
                    {
                        let wm = nes.bus_mut_for_observers().wram_mut();
                        if let Some(h) = player.x_hi {
                            wm[usize::from(h)] = (x >> 8) as u8;
                        }
                        wm[usize::from(player.x_lo)] = x as u8;
                        wm[usize::from(player.y)] = y as u8;
                        wm[usize::from(vy_b)] = vy as u8;
                        if let (Some((xb, _, _)), Some(v)) = (xs, vx) {
                            wm[usize::from(xb)] = v as u8;
                        }
                    }
                    tries += 1;
                    let mut flipped: BTreeSet<u16> = BTreeSet::new();
                    let mut new_code: BTreeSet<u16> = BTreeSet::new();
                    let oam_before = nes.oam().chunks(4).filter(|e| e[0] < 0xEF).count() as i64;
                    let mut sprites_after = oam_before;
                    for _ in 0..24 {
                        let mut prev: Option<u16> = None;
                        let pad = if dir == "side" { farm::RIGHT } else { 0 };
                        nes.step_with(
                            InputFrame {
                                polled: true,
                                pads: [pad, 0, 0, 0],
                                reset: false,
                            },
                            |bus| {
                                let pc = bus.cpu.pc;
                                if !ran(pc) {
                                    new_code.insert(pc);
                                }
                                if let Some(p) = prev
                                    && let Some(want) = flips.get(&p)
                                    && went(bus, p, pc) == Some(*want)
                                {
                                    flipped.insert(p);
                                }
                                prev = Some(pc);
                            },
                        )?;
                        emulated += 1;
                        sprites_after = sprites_after
                            .max(nes.oam().chunks(4).filter(|e| e[0] < 0xEF).count() as i64);
                    }
                    if flipped.is_empty() && new_code.is_empty() {
                        continue;
                    }
                    for b in flipped
                        .iter()
                        .copied()
                        .chain(std::iter::once(0))
                        .take(flipped.len().max(1))
                    {
                        let key = (cx, cy, b);
                        hits.entry(key).or_insert_with(|| json!({
                            "cell_x": cx, "cell_y": cy, "tile": ad.map(|ad| w0[ad]), "addr": ad.map(|ad| format!("{ad:04X}")), "direction": dir,
                            "player_x": x, "player_y": y,
                            "flipped": flipped.iter().map(|p| format!("{p:04X}")).collect::<Vec<_>>(),
                            "new_code": new_code.len(),
                            "new_sprites": sprites_after - oam_before,
                            "branch": if b == 0 { Value::Null } else { json!(format!("{b:04X}")) },
                        }));
                    }
                }
            }
        }
    }
    let found = hits.len();
    let secs = t0.elapsed().as_secs_f64();
    // Confirm: a hit counts once real inputs from power-on reach it: the
    // directed search to that cell, as the poke posed it.
    let mut confirmed: Vec<Value> = Vec::new();
    let mut tried = 0;
    let mut rank: Vec<&Value> = hits.values().filter(|h| !h["branch"].is_null()).collect();
    // Hits on a cell the localizer placed first, then the most specific
    // (fewest branches flipped together: poking the player into the
    // ground flips many), then the most new sprites.
    rank.sort_by_key(|h| {
        let on_placed = placed.contains(&(
            h["cell_x"].as_i64().unwrap_or(0),
            h["cell_y"].as_i64().unwrap_or(0),
        ));
        // The most unrun code behind a branch it flips.
        let most = h["flipped"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| u16::from_str_radix(b.as_str()?, 16).ok())
            .filter_map(|b| behind.get(&b).copied())
            .max()
            .unwrap_or(0);
        (
            !on_placed,
            std::cmp::Reverse(most),
            h["flipped"].as_array().map_or(0, Vec::len),
            -h["new_code"].as_i64().unwrap_or(0),
            -h["new_sprites"].as_i64().unwrap_or(0),
        )
    });
    // One confirmation a cell.
    let mut cells_done: BTreeSet<(i64, i64)> = BTreeSet::new();
    rank.retain(|h| {
        cells_done.insert((
            h["cell_x"].as_i64().unwrap_or(0),
            h["cell_y"].as_i64().unwrap_or(0),
        ))
    });
    for h in rank.into_iter().take(a.probes) {
        tried += 1;
        // The branch it flips with the most unrun code behind it.
        let pc = h["flipped"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| u16::from_str_radix(b.as_str()?, 16).ok())
            .max_by_key(|b| behind.get(b).copied().unwrap_or(0))
            .unwrap_or(0);
        let want = flips.get(&pc).copied().unwrap_or(true);
        let goal = Goal {
            branch: pc,
            want_taken: want,
            also: h["flipped"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| u16::from_str_radix(b.as_str()?, 16).ok())
                .map(|b| (b, flips.get(&b).copied().unwrap_or(true)))
                .collect(),
            cell: h["addr"]
                .as_str()
                .and_then(|a| u16::from_str_radix(a, 16).ok())
                .zip(h["tile"].as_u64().map(|v| v as u8)),
            x: h["cell_x"].as_i64().unwrap_or(0),
            y: h["cell_y"].as_i64().unwrap_or(0),
            ox: h["cell_x"].as_i64().unwrap_or(0) - h["player_x"].as_i64().unwrap_or(0),
            oy: h["cell_y"].as_i64().unwrap_or(0) - h["player_y"].as_i64().unwrap_or(0),
            rising: h["direction"] == json!("below"),
        };
        let Some(start) = start_before(rom, src, player, level, name, goal.x - goal.ox)? else {
            confirmed
                .push(json!({"hit": h, "confirmed": false, "why": "no run reaches the place"}));
            continue;
        };
        let r = probe(
            rom, &start, &goal, player, level, name, "laya", a.budget, &a.decide,
        )?;
        confirmed.push(
            json!({"branch": h["branch"], "cell_x": h["cell_x"], "cell_y": h["cell_y"],
            "direction": h["direction"], "confirmed": r["triggered"], "search": r}),
        );
    }
    let _ = guards;
    Ok(json!({
        "tag": "poke",
        "speed_y_byte": format!("{vy_b:04X}"), "speed_x_byte": xs.map(|x| format!("{:04X}", x.0)),
        "screens": screens.len(), "tries": tries, "emulated_frames": emulated, "seconds": (secs * 10.0).round() / 10.0,
        "found": found, "hits": hits.values().collect::<Vec<_>>(), "confirm_tried": tried, "confirmed": confirmed,
    }))
}

/// Replays `frames` from power-on (then `tail` frames of nothing) and
/// reports: the frames branch `b` goes the wanted way and where the player
/// is then, the watched byte's changes, and the sprite count around it.
#[allow(clippy::too_many_arguments)]
fn check_run(
    rom: &[u8],
    frames: &[InputFrame],
    power_on: gp_evidence::manifest::PowerOn,
    b: u16,
    want: bool,
    watch: Option<u16>,
    player: &Player,
    level: &LevelKey,
    name: &str,
    tail: usize,
) -> Result<Value> {
    let mut nes = gp_nes::Nes::power_on(rom, power_on)?;
    nes.set_video(false);
    let idle = InputFrame {
        polled: true,
        pads: [0; 4],
        reset: false,
    };
    let mut fired: Vec<Value> = Vec::new();
    let mut changes: Vec<(usize, u8)> = Vec::new();
    let mut last: Option<u8> = None;
    let mut sprites: Vec<(usize, usize)> = Vec::new();
    let mut seen_level = false;
    for (fi, f) in frames
        .iter()
        .chain(std::iter::repeat_n(&idle, tail))
        .enumerate()
    {
        let mut prev: Option<u16> = None;
        let mut hit = false;
        nes.step_with(*f, |bus| {
            let pc = bus.cpu.pc;
            if prev == Some(b) && went(bus, b, pc) == Some(want) {
                hit = true;
            }
            prev = Some(pc);
        })?;
        let w = nes.wram()[..0x800].to_vec();
        let here = level.of(&w).is_some_and(|l| l == name);
        if here {
            seen_level = true;
        } else if seen_level && tail == 0 && level.of(&w).is_some() {
            break;
        }
        let n = nes.oam().chunks(4).filter(|e| e[0] < 0xEF).count();
        if hit {
            fired.push(json!({"frame": fi + 1, "player_x": player.x(&w), "player_y": player.y(&w), "sprites": n}));
        }
        if !fired.is_empty() && sprites.len() < 120 {
            sprites.push((fi + 1, n));
        }
        if let Some(a) = watch {
            let v = w[usize::from(a)];
            if last != Some(v) {
                changes.push((fi + 1, v));
                last = Some(v);
            }
        }
    }
    let first = fired.first().and_then(|f| f["frame"].as_u64()).unwrap_or(0) as usize;
    let before = sprites.first().map(|s| s.1).unwrap_or(0);
    let most = sprites.iter().map(|s| s.1).max().unwrap_or(0);
    Ok(json!({
        "frames": frames.len(), "fired": fired.len(), "first": fired.first(),
        "sprites_at_fire": before, "most_sprites_after": most,
        "watched": changes.iter().filter(|(f, _)| *f + 300 >= first && first > 0).collect::<Vec<_>>(),
    }))
}

/// The camera's x from the PPU scroll (both nametables), as gp farm reads it.
fn scroll_x(nes: &gp_nes::Nes) -> i64 {
    let sc = &nes.bus().ppu.scroll;
    i64::from((sc.t & 31) * 8 + u16::from(sc.fine_x as u8 & 7) + ((sc.t >> 10) & 1) * 256)
}

/// The sprite boxes on screen and the index of the player's (the box
/// nearest the player's screen position), if any.
fn boxes_of(nes: &gp_nes::Nes, player: &Player) -> (Vec<[i32; 4]>, Option<usize>) {
    let oam = nes.oam();
    let b = crate::mechanic::detect::sprite_boxes(&oam, if nes.tall_sprites() { 16 } else { 8 });
    let w = &nes.wram()[..0x800];
    let sx = (player.x(w) - scroll_x(nes)).rem_euclid(512);
    let sy = player.y(w);
    let p = b
        .iter()
        .enumerate()
        .min_by_key(|(_, r)| (i64::from(r[0]) - sx).abs() + (i64::from(r[1]) - sy).abs())
        .filter(|(_, r)| (i64::from(r[0]) - sx).abs() + (i64::from(r[1]) - sy).abs() < 24)
        .map(|(i, _)| i);
    (b, p)
}

/// One state of the collect search.
struct CState {
    snap: Vec<u8>,
    frame: u64,
    held: [u8; 4],
    log: Vec<InputFrame>,
    /// The tracked box's screen position, once seen.
    item: Option<(i64, i64)>,
    d: i64,
    visits: u32,
}

#[allow(clippy::too_many_arguments)]
fn collect(
    rom: &[u8],
    run: &[InputFrame],
    from: usize,
    at: (i64, i64),
    player: &Player,
    level: &LevelKey,
    name: &str,
    way: &str,
    budget: usize,
    decide: &str,
) -> Result<Value> {
    let t0 = std::time::Instant::now();
    let mut nes = gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
    nes.set_video(false);
    // RAM for the second before the start, to tell a held byte.
    let mut hist: std::collections::VecDeque<Vec<u8>> = std::collections::VecDeque::new();
    for f in &run[..from] {
        nes.step(*f)?;
        hist.push_back(nes.wram()[..0x800].to_vec());
        if hist.len() > 60 {
            hist.pop_front();
        }
    }
    let mut moves = probe_moves();
    moves.push(Move {
        name: "stop",
        about: "brake to a stop and stand",
        press: vec![(0, 24)],
        then: 0,
        settle: 0,
    });
    // A test hook: GP_COLLECT_SEQ="L12 A30 N120" is the only move.
    if let Ok(seq) = std::env::var("GP_COLLECT_SEQ") {
        let press: Vec<(u8, usize)> = seq
            .split_whitespace()
            .filter_map(|t| {
                let (k, n) = t.split_at(1);
                let pad = match k {
                    "L" => farm::LEFT,
                    "R" => farm::RIGHT,
                    "A" => farm::A,
                    _ => 0,
                };
                n.parse().ok().map(|n| (pad, n))
            })
            .collect();
        moves = vec![Move {
            name: "seq",
            about: "a given sequence",
            press,
            then: 0,
            settle: 0,
        }];
    }

    let criteria: serde_json::Map<String, Value> = moves
        .iter()
        .map(|mv| (mv.name.to_owned(), json!(mv.about)))
        .collect();
    let mut states: Vec<CState> = vec![CState {
        snap: nes.snapshot()?,
        frame: nes.frame(),
        held: nes.held(),
        log: run[..from].to_vec(),
        item: None,
        d: i64::MAX,
        visits: 0,
    }];
    let base_hist: Vec<Vec<u8>> = hist.iter().cloned().collect();
    let mut rng = Rng(0x2545_F491);
    let mut ordinary = vec![false; 0x800];
    let (mut touches, mut seen_item, mut closest) = (0usize, 0usize, i64::MAX);
    let (mut sims, mut frames, mut calls) = (0usize, 0u64, 0usize);
    let idle = InputFrame {
        polled: true,
        pads: [0; 4],
        reset: false,
    };
    while sims < budget {
        let pick = if way == "undirected" {
            (0..states.len())
                .min_by_key(|&i| (states[i].visits, rng.next() % 1000))
                .context("a state")?
        } else {
            let mut idx: Vec<usize> = (0..states.len()).collect();
            idx.sort_by_key(|&i| (states[i].d, states[i].visits));
            let n = idx.len().min(6);
            idx[(rng.next() % n as u64) as usize]
        };
        states[pick].visits += 1;
        let (snap, frame, held, log, item0) = {
            let c = &states[pick];
            (c.snap.clone(), c.frame, c.held, c.log.clone(), c.item)
        };
        let order: Vec<&'static str> = if way == "laya" {
            nes.restore(&snap, frame, held)?;
            let w = nes.wram()[..0x800].to_vec();
            let sx = (player.x(&w) - scroll_x(&nes)).rem_euclid(512);
            let (dx, dy) = item0
                .map_or(((at.0 - player.x(&w)), at.1 - player.y(&w)), |(ix, iy)| {
                    (ix - sx, iy - player.y(&w))
                });
            let st = json!({
                "target": format!("touch the thing that came out of the block: {dx} px ahead, {} px up", -dy),
                "target_ahead_px": dx, "target_up_px": -dy, "seen_yet": item0.is_some(),
            });
            let ask = json!({"model": "laya", "state": st, "questions": {"move": {"type": "choice",
                "instructions": "Which move brings the player to touch the target?", "criteria": criteria}}});
            let ans = farm::post(
                &format!("{}/v1/systemone", decide.trim_end_matches('/')),
                &ask,
            )?;
            calls += 1;
            let pr = &ans["answers"]["move"]["probabilities"];
            let mut o: Vec<(&'static str, f64)> = moves
                .iter()
                .map(|mv| (mv.name, pr[mv.name].as_f64().unwrap_or(0.0)))
                .collect();
            o.sort_by(|a, b| b.1.total_cmp(&a.1));
            o.into_iter().map(|(n, _)| n).collect()
        } else {
            let mut o: Vec<&'static str> = moves.iter().map(|mv| mv.name).collect();
            for i in (1..o.len()).rev() {
                o.swap(i, (rng.next() % (i as u64 + 1)) as usize);
            }
            o
        };
        for nm in order.iter().take(4) {
            if sims >= budget {
                break;
            }
            let mv = moves.iter().find(|m| m.name == *nm).expect("a move");
            nes.restore(&snap, frame, held)?;
            sims += 1;
            let mut l = log.clone();
            let mut item = item0;
            let mut best = i64::MAX;
            let mut end_d = i64::MAX / 2;
            let mut ram: Vec<Vec<u8>> = Vec::new();
            let mut contacts: Vec<usize> = Vec::new();
            let mut stop_dx: i64 = 0;
            // The RAM page the game copies to OAM: sprite positions, not gains.
            let mut oam_page = [0usize; 8];
            let mut prev_oam = nes.oam();
            let mut pads: Vec<u8> = Vec::new();
            for &(pad, n) in &mv.press {
                pads.extend(std::iter::repeat_n(pad, n));
            }
            let total = pads.len() + mv.settle;
            let mut dead = false;
            let mut k = 0usize;
            loop {
                // "stop": hold against the way the player moves until it
                // stands still, then nothing.
                let pad = if mv.name == "stop" {
                    if stop_dx > 0 {
                        farm::LEFT
                    } else if stop_dx < 0 {
                        farm::RIGHT
                    } else {
                        0
                    }
                } else if k < total {
                    pads.get(k).copied().unwrap_or(mv.then)
                } else {
                    0
                };
                let f = if pad == 0 && k >= total {
                    idle
                } else {
                    InputFrame {
                        polled: true,
                        pads: [pad, 0, 0, 0],
                        reset: false,
                    }
                };
                let x_before = player.x(&nes.wram()[..0x800]);
                nes.step(f)?;
                l.push(f);
                frames += 1;
                k += 1;
                stop_dx = player.x(&nes.wram()[..0x800]) - x_before;
                let w = nes.wram()[..0x800].to_vec();
                ram.push(w.clone());
                if level.of(&w).is_none_or(|lv| lv != name) || player.y(&w) > 232 {
                    dead = contacts.is_empty();
                    break;
                }
                // The page the game copies to OAM (this frame's, or the
                // one copied at the next interrupt).
                let oam = nes.oam();
                for (pg, n) in oam_page.iter_mut().enumerate() {
                    let page = &w[pg * 256..pg * 256 + 256];
                    if page == &oam[..] || page == &prev_oam[..] {
                        *n += 1;
                    }
                }
                prev_oam = oam;
                let (b, p) = boxes_of(&nes, player);
                let cam = scroll_x(&nes);
                // The item: a box that isn't the player's, near the cell at
                // first, then the box nearest where it was.
                let near = |r: &[i32; 4], (x, y): (i64, i64)| {
                    (i64::from(r[0]) - x).abs() + (i64::from(r[1]) - y).abs()
                };
                let cell_s = ((at.0 - cam).rem_euclid(512), at.1);
                let cand: Vec<(usize, &[i32; 4])> = b
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| Some(*i) != p)
                    .collect();
                // Follow it from where it was; if it's gone (behind the
                // block a few frames as it comes out), pick it up again by
                // the cell.
                let by_last = item.and_then(|it| {
                    cand.iter()
                        .copied()
                        .min_by_key(|(_, r)| near(r, it))
                        .filter(|(_, r)| near(r, it) < 32)
                });
                let found = by_last.or_else(|| {
                    cand.iter()
                        .copied()
                        .min_by_key(|(_, r)| near(r, cell_s))
                        .filter(|(_, r)| near(r, cell_s) < 40)
                });
                // Gone next to the player: its sprites joined the player's
                // box (touching boxes are merged) or it was taken.
                if found.is_none()
                    && contacts.last().is_none_or(|c| ram.len() > c + 20)
                    && let (Some(it), Some(pi)) = (item, p)
                {
                    let q = b[pi];
                    let gap = (it.0 - i64::from(q[0]))
                        .abs()
                        .max((it.1 - i64::from(q[1])).abs());
                    if gap <= 36 {
                        contacts.push(ram.len() - 1);
                    }
                }
                if let Some((_, r)) = found {
                    item = Some((i64::from(r[0]), i64::from(r[1])));
                    if let Some(pi) = p {
                        let q = b[pi];
                        let dxy = (i64::from(q[0]) - i64::from(r[0])).abs()
                            + (i64::from(q[1]) - i64::from(r[1])).abs();
                        best = best.min(dxy);
                        end_d = dxy;
                        let overlap = q[0] < r[0] + r[2]
                            && r[0] < q[0] + q[2]
                            && q[1] < r[1] + r[3]
                            && r[1] < q[1] + q[3];
                        // OAM is a frame behind, and a taken item is gone the
                        // frame it's touched: near enough counts.
                        if (overlap || dxy <= 24)
                            && contacts.last().is_none_or(|c| ram.len() > c + 20)
                        {
                            contacts.push(ram.len() - 1);
                        }
                    }
                }
                match contacts.last().copied() {
                    Some(c) if ram.len() >= c + 40 && k >= total => break,
                    _ if k >= total + 400 => break,
                    None if k > total => break,
                    _ => {}
                }
            }
            if item.is_some() {
                seen_item += 1;
            }
            closest = closest.min(best);
            if dead {
                continue;
            }
            if contacts.is_empty() && ram.len() > 1 {
                // Bytes that move in ordinary play, touching nothing: not
                // what a touch gains.
                for a in 0..0x800usize {
                    if ram.iter().any(|w| w[a] != ram[0][a]) {
                        ordinary[a] = true;
                    }
                }
            }
            for &c in &contacts {
                touches += 1;
                // A byte that held through the second before the touch and
                // went up by one within 16 frames of it, and stayed.
                let before: Vec<&Vec<u8>> = base_hist
                    .iter()
                    .chain(ram[..c].iter())
                    .rev()
                    .take(60)
                    .collect();
                let last = &ram[ram.len() - 1];
                let at_c = &ram[c.saturating_sub(1)];
                // A page where many bytes change in the move (a sprite
                // buffer) holds positions, not gains.
                let churn: Vec<usize> = (0..8)
                    .map(|pg| {
                        (pg * 256..pg * 256 + 256)
                            .filter(|&a| ram.iter().any(|w| w[a] != ram[0][a]))
                            .count()
                    })
                    .collect();
                let mut gained = Vec::new();
                for a in 0..0x800usize {
                    let mine = [Some(player.x_lo), player.x_hi, Some(player.y)];
                    let sprite_page = oam_page[a / 256] * 2 > ram.len() || churn[a / 256] > 64;
                    if ordinary[a] || sprite_page || mine.contains(&Some(a as u16)) {
                        continue;
                    }
                    let v0 = at_c[a];
                    if before.iter().any(|w| w[a] != v0) {
                        continue;
                    }
                    let up =
                        (c..(c + 16).min(ram.len())).find(|&j| ram[j][a] == v0.wrapping_add(1));
                    if let Some(j) = up
                        && ram[j..].iter().all(|w| w[a] == v0.wrapping_add(1))
                        && last[a] == v0.wrapping_add(1)
                    {
                        gained.push(format!("{a:04X} {v0}->{}", v0.wrapping_add(1)));
                    }
                }
                if !gained.is_empty() {
                    let mut out = InputLog::new(1);
                    out.frames = l.clone();
                    let dir = paths_home().join("work/localize/runs");
                    let _ = fs::create_dir_all(&dir);
                    let f = dir.join(format!("collect-{way}-{}.log", l.len()));
                    fs::write(&f, out.to_text())?;
                    return Ok(
                        json!({"way": way, "gained": true, "bytes": gained, "contact_frame": log.len() + c + 1,
                        "moves": sims, "frames": frames, "calls": calls, "seconds": t0.elapsed().as_secs_f64(),
                        "log": f.display().to_string(), "start_frame": from}),
                    );
                }
            }
            // Scored by where it ends, not the closest it came: the item
            // moves, so a state that was close once can be far now.
            let d = if item.is_some() { end_d } else { i64::MAX / 2 };
            states.push(CState {
                snap: nes.snapshot()?,
                frame: nes.frame(),
                held: nes.held(),
                log: l,
                item,
                d,
                visits: 0,
            });
        }
    }
    Ok(
        json!({"way": way, "gained": false, "touches": touches, "item_seen_moves": seen_item,
        "closest_px": closest, "moves": sims, "frames": frames, "calls": calls,
        "seconds": t0.elapsed().as_secs_f64(), "start_frame": from}),
    )
}

fn paths_home() -> std::path::PathBuf {
    std::env::var_os("GP_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".gameprinter")
        })
}

/// The player's bytes with no dataset: blind discovery's x and y, and an x
/// high byte learned from the runs (the byte that steps by one each time x
/// wraps, and hardly ever otherwise).
fn blind_player(rom: &[u8], paths: &Paths, srcs: &[Src]) -> Result<Player> {
    let d = crate::farm::blind::discover(rom, paths)?;
    let mut player = Player {
        x_hi: None,
        x_lo: d.x,
        y: d.y,
    };
    let Some(src) = srcs.iter().max_by_key(|s| s.frames.len()) else {
        return Ok(player);
    };
    let mut nes = gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
    nes.set_video(false);
    let mut prev: Option<Vec<u8>> = None;
    // Per byte: (steps at a wrap, changes not at a wrap).
    let mut tally = vec![(0usize, 0usize); 0x800];
    for f in &src.frames {
        nes.step(*f)?;
        let w = nes.wram()[..0x800].to_vec();
        if let Some(p) = &prev {
            let (x0, x1) = (p[usize::from(d.x)], w[usize::from(d.x)]);
            let wrap = (x0 >= 0xC0 && x1 < 0x40) || (x0 < 0x40 && x1 >= 0xC0);
            for a in 0..0x800 {
                if a == usize::from(d.x) || a == usize::from(d.y) {
                    continue;
                }
                let step = w[a].wrapping_sub(p[a]);
                if wrap && (step == 1 || step == 0xFF) {
                    tally[a].0 += 1;
                } else if w[a] != p[a] {
                    tally[a].1 += 1;
                }
            }
        }
        prev = Some(w);
    }
    player.x_hi = (0..0x800)
        .filter(|&a| tally[a].0 >= 1 && tally[a].1 <= tally[a].0)
        .max_by_key(|&a| (tally[a].0, std::cmp::Reverse(tally[a].1)))
        .map(|a| a as u16);
    Ok(player)
}

/// Instructions reachable from `start` that no run executed: the walk
/// follows falls, branches, jumps and calls, and stops at code that ran,
/// a return, an indirect jump or a break.
fn unrun_behind(nes: &gp_nes::Nes, start: u16, ran: &dyn Fn(u16) -> bool) -> usize {
    let mut seen: BTreeSet<u16> = BTreeSet::new();
    let mut todo = vec![start];
    while let Some(pc) = todo.pop() {
        if seen.len() >= 4096 || ran(pc) || pc < 0x8000 || !seen.insert(pc) {
            continue;
        }
        let op = nes.peek(pc);
        let r = tetanes_core::cpu::Cpu::INSTR_REF[usize::from(op)];
        let next = pc.wrapping_add(1 + u16::from(r.addr_mode.operand_len()));
        let word = u16::from_le_bytes([nes.peek(pc.wrapping_add(1)), nes.peek(pc.wrapping_add(2))]);
        match op {
            0x00 | 0x40 | 0x60 | 0x6C => {}
            0x4C => todo.push(word),
            0x20 => {
                todo.push(word);
                todo.push(next);
            }
            _ if gp_nes::coverage::is_branch(op) => {
                todo.push(next);
                todo.push(next.wrapping_add(nes.peek(pc.wrapping_add(1)) as i8 as u16));
            }
            _ => todo.push(next),
        }
    }
    seen.len()
}
