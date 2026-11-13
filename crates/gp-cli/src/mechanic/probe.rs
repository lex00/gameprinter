//! `gp mechanic probe`: probe recipes, a start and presses that reproduce an
//! evidence clip, and the measurements taken on it, kept in the game's
//! package folder as `probes.toml` so they run again with no agent.
//!
//! A probe starts from a base (power-on, a stored path, or a mechanic
//! clip's inputs from power-on) at a frame, then plays its steps:
//!
//! | Step | What it does |
//! |---|---|
//! | `PADS N` | Holds the buttons (letters of `RLDUTSBA` on the NES, of the board's columns, `CSRLDUT`, on an arcade board, or `.` for none) for N frames; on the NES `P1\|P2` holds controller 1's and controller 2's (`R\|L`, `.\|A`) |
//! | `walk_to NAME V [max N]` | Holds Right while RAM NAME is below V and Left while above, until it equals V |
//! | `until COND PADS [max N]` | Holds PADS until the condition holds (checked before each frame) |
//! | `repeat K: STEP, STEP` | The steps, K times |
//! | `replay_until COND [max N]` | Plays the base's own inputs on until the condition holds |
//! | `mark` | Measures start here, not at the probe's start |
//!
//! A sweep names variables and their values, and `rows` names an asset
//! whose rows (lifted from the ROM by the template's asset map) give more:
//! each combination is a cell, its steps and measures with `{VAR}`,
//! `{VAR+N}` and `{VAR-N}` replaced. Measures read RAM at the
//! probe's start and after each of its frames, by the count of frames run
//! from power-on (as `gp trace frame` numbers them):
//!
//! | Measure | Value |
//! |---|---|
//! | `first COND` | The first frame the condition holds, or none |
//! | `last COND` | The last such frame, or none |
//! | `any COND` | Whether it holds on any frame: true or false |
//! | `count COND` | How many frames it holds on |
//! | `edges COND` | Each frame it starts to hold |
//! | `gaps COND` | The frames between those starts |
//! | `between COND1 then COND2` | Frames from the first COND1 to the first COND2 at or after it |
//! | `pairs COND1 then COND2` | For each start of COND1, the frames to the next COND2 |
//! | `changes NAME` | Each frame RAM NAME differs from the frame before |
//! | `intervals NAME` | The frames between those changes |
//! | `values NAME at changes OTHER` | NAME on each frame OTHER changes |
//! | `delta NAME` | Its last value less its first |
//! | `value NAME at F` | RAM NAME after frame F (`+K` counts from the probe's start) |
//! | `min NAME`, `max NAME` | Its least and greatest value |
//! | `range NAME` | The greatest less the least |
//!
//! Any measure may end in `while COND`: only the frames the condition
//! holds on count (a change counts when it and the frame before both do).
//! A condition is `NAME OP V` with OP one of `== != < <= > >=`, or several
//! joined by `and`. Names come
//! from the file's `[watch]` table (a RAM address each, the game's facts)
//! or are written as an address (`0x96`). Nothing here knows a game.
//!
//! Addresses are the platform's own: the NES's 2 KB of work RAM (0 to
//! 0x7FF), or an arcade board's main CPU address space (0 to 0xFFFF, as
//! gp-arcade's `ram[A]` reads it, work RAM at 0xE000 on Tapper's board).
//!
//! Ported from game-printer's probe.rs. What gameprinter's layout changed:
//! the original runs through [`OriginalRuntime`] (gp-nes or gp-arcade by
//! the ROM), so a probe runs on an arcade game too; the probes file is the
//! game's package folder's (`games/GAME/probes.toml`), its `rows` assets
//! are lifted with the package's asset map, and saved clips go into the
//! session as the mechanic page saves them. Sealed sessions aren't refused
//! here: gameprinter has no sealed runs yet.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_evidence::input_log::{ArcadeLayout, InputFrame, InputLog};
use gp_runtime::Runtime as _;
use serde::Deserialize;
use serde_json::{Value, json};

use super::paths::{clip_fields, full_log, original_at, put_path, read_path};
use super::session::Session;
use crate::Outcome;
use crate::original::{OriginalRuntime, Platform};

/// The NES's buttons, controller 1's bit 7 first.
const ORDER: &[u8; 8] = b"RLDUTSBA";
/// An arcade board's buttons, bit 0 first: the MAME oracle's columns.
const ARCADE_ORDER: &[u8; 7] = b"CSRLDUT";

/// What a probe's presses and RAM names mean on the game's platform.
#[derive(Clone, Debug)]
pub(crate) struct Board {
    /// Each button's letter and its bit in controller 1's byte.
    pub(crate) keys: Vec<(u8, u8)>,
    /// Whether a step may press controller 2 (`P1|P2`).
    two: bool,
    /// RAM addresses run 0 to this, less one.
    space: u32,
    /// An arcade log's set and columns, for the logs saved; none on the NES.
    pub(crate) arcade: Option<ArcadeLayout>,
}

impl Board {
    pub(crate) fn nes() -> Self {
        Self {
            keys: ORDER
                .iter()
                .enumerate()
                .map(|(i, &c)| (c, 0x80 >> i))
                .collect(),
            two: true,
            space: 0x800,
            arcade: None,
        }
    }

    /// An arcade board whose romset is `system`.
    pub(crate) fn arcade(system: &str) -> Self {
        Self {
            keys: ARCADE_ORDER
                .iter()
                .enumerate()
                .map(|(i, &c)| (c, 1 << i))
                .collect(),
            two: false,
            space: 0x1_0000,
            arcade: Some(ArcadeLayout {
                system: system.to_owned(),
                columns: String::from_utf8_lossy(ARCADE_ORDER).into_owned(),
            }),
        }
    }

    /// The board the ROM is for.
    pub(crate) fn of_rom(rom: &[u8]) -> Result<Self> {
        Ok(match Platform::of_file(rom) {
            Platform::Nes => Self::nes(),
            Platform::Arcade => Self::arcade(
                gp_arcade::romset::Romset::from_zip(rom)
                    .map_err(|e| anyhow::anyhow!("{e}"))?
                    .name,
            ),
        })
    }

    fn bit(&self, c: u8) -> Option<u8> {
        self.keys.iter().find(|k| k.0 == c).map(|k| k.1)
    }

    fn right(&self) -> u16 {
        u16::from(self.bit(b'R').unwrap_or(0))
    }

    fn left(&self) -> u16 {
        u16::from(self.bit(b'L').unwrap_or(0))
    }

    /// An empty log of this board's spelling.
    pub(crate) fn log(&self, controllers: u8) -> InputLog {
        let mut l = InputLog::new(controllers);
        l.arcade = self.arcade.clone();
        if l.arcade.is_some() {
            l.controllers = 1;
        }
        l
    }
}

/// The watch names and the platform's address space they're checked in.
pub(crate) struct Names {
    pub watch: BTreeMap<String, u16>,
    pub space: u32,
}

/// Which RAM addresses each played frame keeps: all the NES's work RAM, or
/// on an arcade board the addresses the probe's measures name.
#[derive(Clone, Debug)]
pub(crate) struct Layout {
    addrs: Vec<u16>,
    dense: bool,
}

impl Layout {
    /// Every address from 0 to `n`, less one.
    pub(crate) fn dense(n: u16) -> Self {
        Self {
            addrs: (0..n).collect(),
            dense: true,
        }
    }

    pub(crate) fn sparse(set: BTreeSet<u16>) -> Self {
        Self {
            addrs: set.into_iter().collect(),
            dense: false,
        }
    }

    /// The byte at `a` in a kept frame.
    fn get(&self, row: &[u8], a: u16) -> u8 {
        if self.dense {
            return row[usize::from(a)];
        }
        let i = self
            .addrs
            .binary_search(&a)
            .unwrap_or_else(|_| panic!("{a:#x} isn't among the addresses kept"));
        row[i]
    }

    /// The kept bytes of the original now.
    fn row(&self, o: &OriginalRuntime) -> Result<Vec<u8>> {
        if self.dense
            && let Some(n) = o.nes()
        {
            return Ok(n.nes().wram()[..self.addrs.len()].to_vec());
        }
        self.addrs.iter().map(|&a| peek(o, a)).collect()
    }
}

/// The byte at RAM address `a` of the original now.
pub(crate) fn peek(o: &OriginalRuntime, a: u16) -> Result<u8> {
    if let Some(n) = o.nes() {
        return Ok(n.nes().wram()[usize::from(a)]);
    }
    if let OriginalRuntime::Arcade(r) = o {
        return Ok(r.tapper().peek(a));
    }
    let sel = format!("ram[{a:#06x}]");
    o.query(&sel)
        .map_err(anyhow::Error::msg)?
        .first()
        .and_then(|e| e.value.as_int())
        .map(|v| v as u8)
        .with_context(|| format!("{sel} isn't a byte"))
}

/// One frame on the original.
pub(crate) fn step(o: &mut OriginalRuntime, f: &InputFrame) -> Result<()> {
    o.step(&[gp_runtime::InputFrame {
        buttons: f.buttons_word(),
        axes: Vec::new(),
        reset: f.reset,
    }])
    .map_err(anyhow::Error::msg)?;
    Ok(())
}
/// The most frames a `walk_to` or `until` step holds when it names no max.
const STEP_MAX: usize = 600;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct File {
    /// Names for RAM addresses.
    #[serde(default)]
    pub watch: BTreeMap<String, u16>,
    /// Named starts: steps from power-on, which a probe's base can name.
    #[serde(default)]
    pub start: BTreeMap<String, Start>,
    #[serde(default)]
    pub probe: Vec<Probe>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Start {
    pub steps: Vec<String>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub id: String,
    /// The mechanic session its clips belong to.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub note: String,
    /// The session's clips this probe reproduces, one a sweep value.
    #[serde(default)]
    pub clips: Vec<u64>,
    #[serde(default)]
    pub base: Base,
    #[serde(default)]
    pub steps: Vec<String>,
    /// The probe's length from `at`; steps shorter than it are padded with
    /// nothing pressed.
    #[serde(default)]
    pub frames: Option<usize>,
    /// Variables and their values: a cell for each combination.
    #[serde(default)]
    pub sweep: BTreeMap<String, Vec<i64>>,
    /// A cell for each row of an asset too, its columns named by `vars`.
    #[serde(default)]
    pub rows: Option<Rows>,
    #[serde(default)]
    pub check: Vec<Check>,
}

/// The rows of a `byte-rows` asset, lifted from the ROM by the template's
/// asset map: game facts read where they live, never typed out.
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rows {
    pub asset: String,
    pub vars: Vec<String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Base {
    /// A start of the file's `[start]` table.
    #[serde(default)]
    pub start: Option<String>,
    /// A stored path, by its sha256.
    #[serde(default)]
    pub path: Option<String>,
    /// Or a mechanic clip's inputs from power-on: session and clip.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub clip: Option<u64>,
    /// The frame the steps start at: by default the path's end, or the
    /// clip's start; 0 from power-on.
    #[serde(default)]
    pub at: Option<usize>,
    /// Play the base's own inputs on from `at` to this frame, before the
    /// steps.
    #[serde(default)]
    pub to: Option<usize>,
    /// Play them to the base's end (a clip's last frame).
    #[serde(default)]
    pub replay: bool,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Check {
    pub measure: String,
    /// The value it must have: a number, "none", or a list.
    #[serde(default)]
    pub expect: Option<toml::Value>,
    /// Per sweep value, in order.
    #[serde(default)]
    pub expect_each: Option<Vec<toml::Value>>,
    #[serde(default)]
    pub note: String,
}

/// One controller's buttons: the board's letters (`RLDUTSBA` on the NES),
/// or `.` for none.
fn pad(b: &Board, s: &str) -> Result<u8> {
    let mut p = 0u8;
    for c in s.bytes() {
        if c == b'.' {
            continue;
        }
        let bit = b.bit(c).with_context(|| {
            let letters: String = b.keys.iter().map(|k| k.0 as char).collect();
            format!("{s}: buttons are letters of {letters} or .")
        })?;
        p |= bit;
    }
    Ok(p)
}

/// Both controllers' buttons, controller 1 in the low byte: `P1`, or
/// `P1|P2` for a press on controller 2 as well (the NES only).
pub(crate) fn pads(b: &Board, s: &str) -> Result<u16> {
    match s.split_once('|') {
        Some(_) if !b.two => bail!("{s}: this board has one set of controls"),
        Some((x, y)) => Ok(u16::from(pad(b, x)?) | u16::from(pad(b, y)?) << 8),
        None => Ok(u16::from(pad(b, s)?)),
    }
}

pub(crate) fn press(p: u16) -> InputFrame {
    let [p1, p2] = p.to_le_bytes();
    InputFrame {
        polled: true,
        pads: [p1, p2, 0, 0],
        reset: false,
    }
}

/// The text of one controller, as steps write it: its letters, or `.`.
fn pad_text(b: &Board, p: u8) -> String {
    let s: String = b
        .keys
        .iter()
        .filter(|(_, bit)| p & bit != 0)
        .map(|(c, _)| *c as char)
        .collect();
    if s.is_empty() { ".".into() } else { s }
}

/// The text of both controllers: controller 2's after a `|` when it holds
/// any button.
fn pads_text(b: &Board, p: u16) -> String {
    let [p1, p2] = p.to_le_bytes();
    if p2 == 0 {
        pad_text(b, p1)
    } else {
        format!("{}|{}", pad_text(b, p1), pad_text(b, p2))
    }
}

/// Inputs as steps: one `PADS N` per run of the same buttons.
pub(crate) fn steps_of(b: &Board, frames: &[InputFrame]) -> Vec<String> {
    let mut out: Vec<(u16, usize)> = Vec::new();
    for f in frames {
        let p = u16::from_le_bytes([f.pads[0], f.pads[1]]);
        match out.last_mut() {
            Some((q, n)) if *q == p => *n += 1,
            _ => out.push((p, 1)),
        }
    }
    out.into_iter()
        .map(|(p, n)| format!("{} {n}", pads_text(b, p)))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Atom {
    pub(crate) addr: u16,
    op: Op,
    v: i64,
}

/// Tests joined by `and`.
#[derive(Clone, Debug)]
pub(crate) struct Cond(pub(crate) Vec<Atom>);

impl Cond {
    /// Whether it holds on a kept frame.
    fn holds(&self, l: &Layout, ram: &[u8]) -> bool {
        self.holds_by(|a| l.get(ram, a))
    }

    /// Whether it holds on the original now.
    pub(crate) fn holds_now(&self, o: &OriginalRuntime) -> Result<bool> {
        let mut err = None;
        let r = self.holds_by(|a| {
            peek(o, a).unwrap_or_else(|e| {
                err = Some(e);
                0
            })
        });
        match err {
            Some(e) => Err(e),
            None => Ok(r),
        }
    }

    pub(crate) fn holds_by(&self, mut byte: impl FnMut(u16) -> u8) -> bool {
        self.0.iter().all(|a| {
            let x = i64::from(byte(a.addr));
            match a.op {
                Op::Eq => x == a.v,
                Op::Ne => x != a.v,
                Op::Lt => x < a.v,
                Op::Le => x <= a.v,
                Op::Gt => x > a.v,
                Op::Ge => x >= a.v,
            }
        })
    }
}

pub(crate) fn num(s: &str) -> Result<i64> {
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix('$')) {
        return i64::from_str_radix(h, 16).with_context(|| format!("not a number: {s}"));
    }
    s.parse().with_context(|| format!("not a number: {s}"))
}

fn addr(names: &Names, s: &str) -> Result<u16> {
    if let Some(a) = names.watch.get(s) {
        return Ok(*a);
    }
    let a = num(s).with_context(|| format!("{s} is in no [watch] row and isn't an address"))?;
    if !(0..i64::from(names.space)).contains(&a) {
        bail!("{s}: RAM addresses run 0 to {:#X}", names.space - 1);
    }
    Ok(a as u16)
}

pub(crate) fn cond(names: &Names, t: &[&str]) -> Result<Cond> {
    let mut atoms = Vec::new();
    for part in t.split(|w| *w == "and") {
        let [name, op, v] = part else {
            bail!("a condition is NAME OP V, joined by and: {}", t.join(" "));
        };
        let op = match *op {
            "==" => Op::Eq,
            "!=" => Op::Ne,
            "<" => Op::Lt,
            "<=" => Op::Le,
            ">" => Op::Gt,
            ">=" => Op::Ge,
            o => bail!("{o}: a condition's test is one of == != < <= > >="),
        };
        atoms.push(Atom {
            addr: addr(names, name)?,
            op,
            v: num(v)?,
        });
    }
    Ok(Cond(atoms))
}

/// A cell: each variable's value.
type Cell = BTreeMap<String, i64>;

/// Replaces `{VAR}`, `{VAR+N}` and `{VAR-N}` with the cell's values; other
/// braces are left as they are.
fn fill(s: &str, cell: &Cell) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else {
            break;
        };
        let inner = &rest[i + 1..i + j];
        let (name, add) = match inner.find(['+', '-']) {
            Some(k) => {
                let n: Option<i64> = inner[k + 1..].trim().parse().ok();
                let sign = if inner.as_bytes()[k] == b'-' { -1 } else { 1 };
                (inner[..k].trim(), n.map(|n| sign * n))
            }
            None => (inner.trim(), Some(0)),
        };
        match (cell.get(name), add) {
            (Some(v), Some(d)) => out.push_str(&(v + d).to_string()),
            _ => out.push_str(&rest[i..=i + j]),
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

/// The cells of a probe: every combination of its sweep's values, times
/// its asset's rows; one empty cell when it has neither.
fn cells_of(p: &Probe, rows: &[Vec<i64>]) -> Result<Vec<Cell>> {
    let mut cells = vec![Cell::new()];
    for (k, vs) in &p.sweep {
        cells = cells
            .into_iter()
            .flat_map(|c| {
                vs.iter().map(move |v| {
                    let mut c = c.clone();
                    c.insert(k.clone(), *v);
                    c
                })
            })
            .collect();
    }
    if let Some(r) = &p.rows {
        let mut out = Vec::new();
        for c in &cells {
            for row in rows {
                if row.len() < r.vars.len() {
                    bail!(
                        "{}: a row has {} columns, not {}",
                        r.asset,
                        row.len(),
                        r.vars.len()
                    );
                }
                let mut c = c.clone();
                for (k, v) in r.vars.iter().zip(row) {
                    c.insert(k.clone(), *v);
                }
                out.push(c);
            }
        }
        cells = out;
    }
    Ok(cells)
}

/// A `byte-rows` asset's rows, lifted from the ROM by the asset map the
/// game's package names for it (its `[[rom.entry]]`'s `asset_map`).
fn asset_rows(game_dir: &Path, rom: &[u8], name: &str) -> Result<Vec<Vec<i64>>> {
    let (map_path, map_text) = crate::corpus_score::asset_map(game_dir, rom)?;
    let map =
        gp_assets::AssetMap::parse(&map_text).with_context(|| map_path.display().to_string())?;
    let (files, lifted) = gp_assets::lift(rom, &map, &map_text)?;
    let l = lifted
        .iter()
        .find(|l| l.name == name)
        .with_context(|| format!("no asset {name} in {}", map_path.display()))?;
    let f = files
        .iter()
        .find(|f| f.name == l.file)
        .with_context(|| format!("asset {name} lifted no file"))?;
    let v: toml::Table = toml::from_str(&String::from_utf8_lossy(&f.bytes))
        .with_context(|| format!("asset {name}"))?;
    let rows = v
        .get("rows")
        .and_then(toml::Value::as_array)
        .with_context(|| format!("asset {name} has no rows: it isn't byte-rows"))?;
    rows.iter()
        .map(|r| {
            r.as_array()
                .with_context(|| format!("asset {name}: a row isn't a list of numbers"))?
                .iter()
                .map(|x| {
                    x.as_integer()
                        .with_context(|| format!("asset {name}: not a number"))
                })
                .collect()
        })
        .collect()
}

/// Where a probe starts: the inputs from power-on to `at`, `at`, the
/// base's own inputs it plays on from there (`to`, `replay`), and the
/// base's inputs after those, which `replay_until` plays. A clip's inputs
/// are looked up in GAME's sessions.
fn resolve_base(
    home: &Path,
    game: &str,
    rom: &[u8],
    board: &Board,
    f: &File,
    b: &Base,
) -> Result<Based> {
    if let Some(name) = &b.start {
        if b.path.is_some() || b.session.is_some() {
            bail!("a base is a start, a path, or a session and a clip");
        }
        let st = f
            .start
            .get(name)
            .with_context(|| format!("no [start.{name}] in the probes file"))?;
        let o = OriginalRuntime::of_rom(rom)?;
        let names = names_of(board, f);
        let none = Layout::sparse(BTreeSet::new());
        let played = play(o, board, &names, &none, &[], &[], &st.steps, None)?;
        if let Some(u) = played.unreached {
            bail!("start {name}: the step `{u}` ran out of frames");
        }
        let frames = played.inputs;
        let at = b.at.unwrap_or(frames.len());
        if at > frames.len() {
            bail!("start {name} has {} frames, not {at}", frames.len());
        }
        return Ok(Based {
            base: frames[..at].to_vec(),
            at,
            tail: Vec::new(),
            follow: Vec::new(),
        });
    }
    let (frames, default_at, end) = match (&b.path, &b.session, b.clip) {
        (Some(sha), None, None) => {
            let l = read_path(home, sha)?;
            let n = l.frames.len();
            (l.frames, n, n)
        }
        (None, Some(s), Some(n)) => {
            let dir = home.join("mechanics").join(game).join(s);
            let (text, start) =
                full_log(home, &dir, n).with_context(|| format!("the base clip {s} {n}"))?;
            let log = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("{s} clip {n}: {e}"))?;
            let t: toml::Table =
                toml::from_str(&fs::read_to_string(dir.join(format!("clips/{n:03}.toml")))?)?;
            let end = t
                .get("frames")
                .and_then(toml::Value::as_integer)
                .map_or(log.frames.len(), |f| {
                    (start + f as usize).min(log.frames.len())
                });
            (log.frames, start, end)
        }
        (None, None, None) => (Vec::new(), 0, 0),
        _ => bail!("a base is a path, or a session and a clip, or nothing (power-on)"),
    };
    let at = b.at.unwrap_or(default_at);
    let to = match (b.to, b.replay) {
        (Some(t), _) => t,
        (None, true) => end,
        (None, false) => at,
    };
    if at > frames.len() || to > frames.len() || to < at {
        bail!(
            "the base has {} frames: at {at} and to {to} don't fit",
            frames.len()
        );
    }
    Ok(Based {
        base: frames[..at].to_vec(),
        at,
        tail: frames[at..to].to_vec(),
        follow: frames[to..].to_vec(),
    })
}

struct Based {
    base: Vec<InputFrame>,
    at: usize,
    tail: Vec<InputFrame>,
    follow: Vec<InputFrame>,
}

/// The original at the base: from the start cache when the base is a
/// stored path, else replayed from power-on.
fn console_at(
    home: &Path,
    rom: &[u8],
    b: &Base,
    frames: &[InputFrame],
    at: usize,
) -> Result<OriginalRuntime> {
    if let (Some(sha), None) = (&b.path, &b.session) {
        return original_at(home, rom, sha, at);
    }
    let mut o = OriginalRuntime::of_rom(rom)?;
    for f in frames {
        step(&mut o, f)?;
    }
    Ok(o)
}

/// The file's watch names on the board.
fn names_of(board: &Board, f: &File) -> Names {
    Names {
        watch: f.watch.clone(),
        space: board.space,
    }
}

/// The addresses a measure reads on its frames, as [`measure`] parses it.
fn measure_addrs(names: &Names, m: &str) -> Result<BTreeSet<u16>> {
    let all: Vec<&str> = m.split_whitespace().collect();
    let mut out = BTreeSet::new();
    let add_cond = |t: &[&str], out: &mut BTreeSet<u16>| -> Result<()> {
        out.extend(cond(names, t)?.0.iter().map(|a| a.addr));
        Ok(())
    };
    let t = match all.iter().position(|w| *w == "while") {
        Some(k) => {
            add_cond(&all[k + 1..], &mut out)?;
            &all[..k]
        }
        None => &all[..],
    };
    match t.first().copied() {
        Some("first" | "last" | "any" | "count" | "edges" | "gaps") => add_cond(&t[1..], &mut out)?,
        Some("between" | "pairs") => {
            if let Some(k) = t.iter().position(|w| *w == "then") {
                add_cond(&t[1..k], &mut out)?;
                add_cond(&t[k + 1..], &mut out)?;
            }
        }
        Some("value") => {
            if let Some(n) = t.get(1) {
                out.insert(addr(names, n)?);
            }
        }
        Some("values") => {
            for n in [t.get(1), t.get(4)].into_iter().flatten() {
                out.insert(addr(names, n)?);
            }
        }
        Some("changes" | "intervals" | "delta" | "range" | "min" | "max") => {
            if let Some(n) = t.get(1) {
                out.insert(addr(names, n)?);
            }
        }
        _ => {}
    }
    Ok(out)
}

/// The addresses a probe's played frames keep: the NES's whole work RAM,
/// else every watch name's and every address the measures read.
fn layout_of(board: &Board, names: &Names, measures: &[String]) -> Result<Layout> {
    if board.arcade.is_none() {
        return Ok(Layout::dense(board.space as u16));
    }
    let mut set: BTreeSet<u16> = names.watch.values().copied().collect();
    for m in measures {
        set.extend(measure_addrs(names, m)?);
    }
    Ok(Layout::sparse(set))
}

/// Steps with each `repeat K: STEP, STEP` written out K times.
fn expand(steps: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for s in steps {
        match s.trim().strip_prefix("repeat ") {
            Some(r) => {
                let (k, body) = r
                    .split_once(':')
                    .with_context(|| format!("{s}: repeat K: STEP, STEP"))?;
                let k: usize = k.trim().parse().with_context(|| format!("{s}: K"))?;
                let body: Vec<String> = body.split(',').map(|b| b.trim().to_owned()).collect();
                for _ in 0..k {
                    out.extend(body.iter().cloned());
                }
            }
            None => out.push(s.clone()),
        }
    }
    Ok(out)
}

/// A cell as the run keeps it.
struct Ran {
    inputs: Vec<InputFrame>,
    unreached: Option<String>,
    /// Each measure, filled for the cell, and its value.
    values: Vec<(String, Value)>,
}

/// One played cell: its inputs from `at` and RAM after each of its frames.
pub(crate) struct Played {
    pub inputs: Vec<InputFrame>,
    pub ram: Vec<Vec<u8>>,
    /// A `walk_to` or `until` that ran out of frames: the step's text.
    pub unreached: Option<String>,
    /// Where the measures start: the last `mark` step's frame (from `at`).
    pub mark: usize,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn play(
    mut o: OriginalRuntime,
    board: &Board,
    names: &Names,
    layout: &Layout,
    tail: &[InputFrame],
    follow: &[InputFrame],
    steps: &[String],
    frames: Option<usize>,
) -> Result<Played> {
    let mut follow = follow.iter();
    let mut mark = 0;
    let mut inputs = Vec::new();
    // RAM at the start, then after each frame: `ram[i]` is frame `at + i`,
    // as `gp trace frame` numbers them.
    let mut ram = vec![layout.row(&o)?];
    let mut unreached = None;
    let run = |o: &mut OriginalRuntime,
               f: InputFrame,
               inputs: &mut Vec<InputFrame>,
               ram: &mut Vec<Vec<u8>>|
     -> Result<()> {
        step(o, &f)?;
        inputs.push(f);
        ram.push(layout.row(o)?);
        Ok(())
    };
    for f in tail {
        run(&mut o, *f, &mut inputs, &mut ram)?;
    }
    let steps = expand(steps)?;
    'steps: for s in &steps {
        let t: Vec<&str> = s.split_whitespace().collect();
        let (most, t) = match t.iter().position(|w| *w == "max") {
            Some(i) => (
                t.get(i + 1)
                    .with_context(|| format!("{s}: max needs a number"))?
                    .parse::<usize>()?,
                &t[..i],
            ),
            None => (STEP_MAX, &t[..]),
        };
        match t.first().copied() {
            Some("walk_to") => {
                let [_, name, v] = t else {
                    bail!("{s}: walk_to NAME V [max N]");
                };
                let (a, v) = (addr(names, name)?, num(v)?);
                let mut k = 0;
                loop {
                    let x = i64::from(peek(&o, a)?);
                    if x == v {
                        break;
                    }
                    if k == most {
                        unreached = Some(s.clone());
                        break 'steps;
                    }
                    let p = if x < v { board.right() } else { board.left() };
                    run(&mut o, press(p), &mut inputs, &mut ram)?;
                    k += 1;
                }
            }
            Some("mark") => mark = ram.len() - 1,
            Some("replay_until") => {
                let c = cond(names, &t[1..])?;
                let mut k = 0;
                while !c.holds_now(&o)? {
                    let next = follow.next();
                    match next {
                        Some(f) if k < most => {
                            run(&mut o, *f, &mut inputs, &mut ram)?;
                            k += 1;
                        }
                        _ => {
                            unreached = Some(s.clone());
                            break 'steps;
                        }
                    }
                }
            }
            Some("until") => {
                if t.len() < 5 {
                    bail!("{s}: until COND PADS [max N]");
                }
                let c = cond(names, &t[1..t.len() - 1])?;
                let p = pads(board, t[t.len() - 1])?;
                let mut k = 0;
                while !c.holds_now(&o)? {
                    if k == most {
                        unreached = Some(s.clone());
                        break 'steps;
                    }
                    run(&mut o, press(p), &mut inputs, &mut ram)?;
                    k += 1;
                }
            }
            Some(p) => {
                let [_, n] = t else {
                    bail!("{s}: a step is PADS N, walk_to, until, replay_until or repeat");
                };
                let p = pads(board, p)?;
                for _ in 0..n
                    .parse::<usize>()
                    .with_context(|| format!("{s}: N frames"))?
                {
                    run(&mut o, press(p), &mut inputs, &mut ram)?;
                }
            }
            None => {}
        }
    }
    if unreached.is_none()
        && let Some(n) = frames
    {
        while inputs.len() < n {
            run(&mut o, press(0), &mut inputs, &mut ram)?;
        }
    }
    Ok(Played {
        inputs,
        ram,
        unreached,
        mark,
    })
}

/// A measure's value over a played cell whose first frame is `at`: RAM
/// `ram[i]` is frame `at + i`. A measure may end in `while COND`, which
/// keeps only the frames the condition holds on.
fn measure(names: &Names, l: &Layout, m: &str, at: usize, ram: &[Vec<u8>]) -> Result<Value> {
    let all: Vec<&str> = m.split_whitespace().collect();
    let (t, keep): (&[&str], Vec<bool>) = match all.iter().position(|w| *w == "while") {
        Some(k) => {
            let c = cond(names, &all[k + 1..])?;
            (&all[..k], ram.iter().map(|r| c.holds(l, r)).collect())
        }
        None => (&all[..], vec![true; ram.len()]),
    };
    let frame = |i: usize| json!(at + i);
    let hold = |c: &Cond, i: usize| keep[i] && c.holds(l, &ram[i]);
    let first_from = |c: &Cond, from: usize| (from..ram.len()).find(|&i| hold(c, i));
    let edges = |c: &Cond| -> Vec<usize> {
        (0..ram.len())
            .filter(|&i| hold(c, i) && (i == 0 || !hold(c, i - 1)))
            .collect()
    };
    let byte = |name: &str| -> Result<u16> { addr(names, name) };
    let changes = |a: u16| -> Vec<usize> {
        (1..ram.len())
            .filter(|&i| keep[i] && keep[i - 1] && l.get(&ram[i], a) != l.get(&ram[i - 1], a))
            .collect()
    };
    let kept = || (0..ram.len()).filter(|&i| keep[i]);
    let one = |what: &str| -> Result<u16> {
        let [_, name] = t[..] else {
            bail!("{m}: {what} NAME");
        };
        byte(name)
    };
    Ok(match t.first().copied() {
        Some("first") => first_from(&cond(names, &t[1..])?, 0).map_or(json!("none"), frame),
        Some("last") => {
            let c = cond(names, &t[1..])?;
            (0..ram.len())
                .rev()
                .find(|&i| hold(&c, i))
                .map_or(json!("none"), frame)
        }
        Some("any") => {
            let c = cond(names, &t[1..])?;
            json!((0..ram.len()).any(|i| hold(&c, i)))
        }
        Some("count") => {
            let c = cond(names, &t[1..])?;
            json!((0..ram.len()).filter(|&i| hold(&c, i)).count())
        }
        Some("edges") => {
            json!(
                edges(&cond(names, &t[1..])?)
                    .into_iter()
                    .map(|i| at + i)
                    .collect::<Vec<_>>()
            )
        }
        Some("gaps") => {
            let e = edges(&cond(names, &t[1..])?);
            json!(e.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>())
        }
        Some(w @ ("between" | "pairs")) => {
            let k = t
                .iter()
                .position(|w| *w == "then")
                .with_context(|| format!("{m}: {w} COND1 then COND2"))?;
            let (a, b) = (cond(names, &t[1..k])?, cond(names, &t[k + 1..])?);
            if w == "between" {
                match first_from(&a, 0) {
                    Some(i) => first_from(&b, i).map_or(json!("none"), |j| json!(j - i)),
                    None => json!("none"),
                }
            } else {
                json!(
                    edges(&a)
                        .into_iter()
                        .filter_map(|i| first_from(&b, i).map(|j| j - i))
                        .collect::<Vec<_>>()
                )
            }
        }
        Some("value") => {
            let [_, name, "at", f] = t[..] else {
                bail!("{m}: value NAME at F");
            };
            let a = byte(name)?;
            let i = match f.strip_prefix('+') {
                Some(k) => k.parse::<usize>()?,
                // A frame number from power-on.
                None => num(f)?
                    .checked_sub(at as i64)
                    .and_then(|k| usize::try_from(k).ok())
                    .with_context(|| format!("{m}: frame {f} is before the probe's start {at}"))?,
            };
            ram.get(i).map_or(json!("none"), |r| json!(l.get(r, a)))
        }
        Some("values") => {
            let [_, name, "at", "changes", other] = t[..] else {
                bail!("{m}: values NAME at changes OTHER");
            };
            let (a, o) = (byte(name)?, byte(other)?);
            json!(
                changes(o)
                    .into_iter()
                    .map(|i| l.get(&ram[i], a))
                    .collect::<Vec<_>>()
            )
        }
        Some("changes") => {
            json!(
                changes(one("changes")?)
                    .into_iter()
                    .map(|i| at + i)
                    .collect::<Vec<_>>()
            )
        }
        Some("intervals") => {
            let c = changes(one("intervals")?);
            json!(c.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>())
        }
        Some("delta") => {
            let a = one("delta")?;
            match (kept().next(), kept().next_back()) {
                (Some(i), Some(j)) => {
                    json!(i64::from(l.get(&ram[j], a)) - i64::from(l.get(&ram[i], a)))
                }
                _ => json!("none"),
            }
        }
        Some("range") => {
            let a = one("range")?;
            let (lo, hi) = (
                kept().map(|i| l.get(&ram[i], a)).min(),
                kept().map(|i| l.get(&ram[i], a)).max(),
            );
            match (lo, hi) {
                (Some(lo), Some(hi)) => json!(hi - lo),
                _ => json!("none"),
            }
        }
        Some(w @ ("min" | "max")) => {
            let a = one(w)?;
            let it = kept().map(|i| l.get(&ram[i], a));
            let v = if w == "min" { it.min() } else { it.max() };
            v.map_or(json!("none"), |v| json!(v))
        }
        _ => bail!(
            "{m}: a measure is first, last, any, count, edges, gaps, between, pairs, value, values, changes, intervals, delta, range, min or max"
        ),
    })
}

fn expect_json(v: &toml::Value) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// The game's probes file: its package folder's `probes.toml`, or `file`.
pub(crate) fn load(game_dir: &Path, file: Option<&Path>) -> Result<(PathBuf, File)> {
    let f = file.map_or_else(|| game_dir.join("probes.toml"), Path::to_path_buf);
    let text = fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?;
    let parsed: File = toml::from_str(&text).with_context(|| format!("{}", f.display()))?;
    let mut seen = std::collections::BTreeSet::new();
    for p in &parsed.probe {
        if !seen.insert(p.id.clone()) {
            bail!("{}: two probes named {}", f.display(), p.id);
        }
    }
    Ok((f, parsed))
}

pub(crate) struct Args<'a> {
    pub game: &'a str,
    pub ids: &'a [String],
    pub file: Option<&'a Path>,
    pub rom: Option<&'a Path>,
    pub games: Option<&'a Path>,
    pub save: bool,
}

/// Compares a cell's inputs from power-on with a saved clip's, over the
/// clip's frames: None when they agree, else the first frame they don't.
fn differs(
    home: &Path,
    dir: &Path,
    n: u64,
    base: &[InputFrame],
    inputs: &[InputFrame],
) -> Result<Option<String>> {
    let (text, start) = full_log(home, dir, n)?;
    let clip = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("clip {n}: {e}"))?;
    let t: toml::Table =
        toml::from_str(&fs::read_to_string(dir.join(format!("clips/{n:03}.toml")))?)?;
    let len = t
        .get("frames")
        .and_then(toml::Value::as_integer)
        .map_or(clip.frames.len() - start, |f| f as usize);
    let end = (start + len).min(clip.frames.len());
    let ours: Vec<&InputFrame> = base.iter().chain(inputs).collect();
    if ours.len() < end {
        return Ok(Some(format!(
            "the probe runs to frame {}, the clip to {end}",
            ours.len()
        )));
    }
    if let Some(i) = (0..end).find(|&i| ours[i].pads[0] != clip.frames[i].pads[0]) {
        return Ok(Some(format!("inputs differ at frame {i}")));
    }
    Ok(None)
}

/// The game's package folder and its ROM: `--rom`, else the one its
/// roms.toml names, found in the ROM folder as the mechanic page finds it.
fn game_and_rom(paths: &Paths, a: &Args) -> Result<(PathBuf, Vec<u8>)> {
    let games = crate::roms_cmd::games_dir(a.games)
        .context("no games checkout: pass --games or set GP_GAMES_DIR")?;
    let rom = match a.rom {
        Some(r) => fs::read(r).with_context(|| format!("reading {}", r.display()))?,
        None => super::server::game_rom(paths, a.games, a.game)?.0.bytes,
    };
    Ok((games.join(a.game), rom))
}

pub(crate) fn run(paths: &Paths, a: &Args) -> Result<Outcome> {
    let (game_dir, rom) = game_and_rom(paths, a)?;
    let (file, f) = load(&game_dir, a.file)?;
    let board = Board::of_rom(&rom)?;
    let names = names_of(&board, &f);
    let rom_sha = crate::roms::sha256_of(&rom)?;
    let chosen: Vec<&Probe> = f
        .probe
        .iter()
        .filter(|p| a.ids.is_empty() || a.ids.contains(&p.id))
        .collect();
    if chosen.is_empty() {
        bail!("{}: no probe named {}", file.display(), a.ids.join(", "));
    }
    let mut text = String::new();
    let mut out = Vec::new();
    let (mut checks, mut failed) = (0usize, 0usize);
    for p in chosen {
        let Based {
            base,
            at,
            tail,
            follow,
        } = resolve_base(&paths.home, a.game, &rom, &board, &f, &p.base)?;
        let start = console_at(&paths.home, &rom, &p.base, &base, at)?;
        let rows = match &p.rows {
            Some(r) => asset_rows(&game_dir, &rom, &r.asset)?,
            None => Vec::new(),
        };
        let all = cells_of(p, &rows)?;
        text.push_str(&format!("{} (from frame {at}): {}\n", p.id, p.note));
        // The cells play on the pool's threads; each keeps only its inputs
        // and its measures' values, not its RAM.
        let ran = crate::pool::map(
            &all,
            || (),
            |_, _, kv| -> std::result::Result<Ran, String> {
                let go = || -> Result<Ran> {
                    let steps: Vec<String> = p.steps.iter().map(|s| fill(s, kv)).collect();
                    let ms: Vec<String> = p.check.iter().map(|c| fill(&c.measure, kv)).collect();
                    let layout = layout_of(&board, &names, &ms)?;
                    let played = play(
                        start.clone(),
                        &board,
                        &names,
                        &layout,
                        &tail,
                        &follow,
                        &steps,
                        p.frames,
                    )?;
                    let mut values = Vec::new();
                    for m in ms {
                        let v = measure(
                            &names,
                            &layout,
                            &m,
                            at + played.mark,
                            &played.ram[played.mark..],
                        )?;
                        values.push((m, v));
                    }
                    Ok(Ran {
                        inputs: played.inputs,
                        unreached: played.unreached,
                        values,
                    })
                };
                go().map_err(|e| format!("{e:#}"))
            },
            &mut |_| {},
        );
        let mut cells = Vec::new();
        for (ci, (kv, played)) in all.iter().zip(ran).enumerate() {
            let played = played.map_err(|e| anyhow::anyhow!("probe {}: {e}", p.id))?;
            let label = if kv.is_empty() {
                String::new()
            } else {
                let parts: Vec<String> = kv.iter().map(|(k, v)| format!("{k} = {v}")).collect();
                format!("{}: ", parts.join(", "))
            };
            let mut ms = Vec::new();
            for (c, (m, got)) in p.check.iter().zip(played.values) {
                let want = c
                    .expect_each
                    .as_ref()
                    .and_then(|e| e.get(ci))
                    .or(c.expect.as_ref())
                    .map(expect_json);
                let ok = want.as_ref().is_none_or(|w| *w == got);
                if want.is_some() {
                    checks += 1;
                    failed += usize::from(!ok);
                }
                text.push_str(&format!(
                    "  {label}{m}: {got}{}\n",
                    match (&want, ok) {
                        (None, _) => String::new(),
                        (Some(_), true) => " (as expected)".into(),
                        (Some(w), false) => format!(" (EXPECTED {w})"),
                    }
                ));
                ms.push(
                    json!({"measure": m, "value": got, "expect": want, "ok": ok, "note": c.note}),
                );
            }
            if let Some(u) = &played.unreached {
                text.push_str(&format!("  {label}the step `{u}` ran out of frames\n"));
            }
            // The saved clip it reproduces, if any.
            let mut clip_json = Value::Null;
            let saving = Saving {
                paths,
                game: a.game,
                rom: &rom,
                rom_sha: &rom_sha,
                board: &board,
            };
            if let (Some(s), Some(&n)) = (&p.session, p.clips.get(ci)) {
                let dir = paths.home.join("mechanics").join(a.game).join(s);
                if dir.join(format!("clips/{n:03}.toml")).is_file() {
                    let d = differs(&paths.home, &dir, n, &base, &played.inputs)?;
                    text.push_str(&format!(
                        "  {label}{s} clip {n}: {}\n",
                        d.as_deref().unwrap_or("the same inputs from power-on")
                    ));
                    clip_json = json!({"session": s, "clip": n, "same": d.is_none(), "differs": d});
                } else if a.save {
                    let n = saving.save(s, &base, &played.inputs, Some(n), &p.note)?;
                    text.push_str(&format!("  {label}saved as {s} clip {n}\n"));
                    clip_json = json!({"session": s, "clip": n, "saved": true});
                }
            } else if a.save
                && let Some(s) = &p.session
            {
                let note = format!("probe {}: {label}{}", p.id, p.note);
                let n = saving.save(s, &base, &played.inputs, None, &note)?;
                text.push_str(&format!("  {label}saved as {s} clip {n}\n"));
                clip_json = json!({"session": s, "clip": n, "saved": true});
            }
            cells.push(json!({
                "cell": kv, "frames": played.inputs.len(), "unreached": played.unreached,
                "measures": ms, "clip": clip_json,
            }));
        }
        out.push(json!({"id": p.id, "at": at, "cells": cells}));
    }
    text.push_str(&format!(
        "{} of {checks} checks as expected\n",
        checks - failed
    ));
    let mut o = Outcome::ok(
        json!({"file": file.display().to_string(), "probes": out, "checks": checks, "failed": failed}),
        text,
    );
    if failed > 0 {
        o.exit = 1;
    }
    Ok(o)
}

/// What saving a cell as a clip needs.
struct Saving<'a> {
    paths: &'a Paths,
    game: &'a str,
    rom: &'a [u8],
    rom_sha: &'a str,
    board: &'a Board,
}

impl Saving<'_> {
    /// Saves a cell as a clip of a session, as the mechanic page saves
    /// one: the base stored as a path, the cell's inputs as the clip, and
    /// the state's hash at its start.
    fn save(
        &self,
        session: &str,
        base: &[InputFrame],
        inputs: &[InputFrame],
        as_clip: Option<u64>,
        note: &str,
    ) -> Result<u64> {
        if inputs.is_empty() {
            bail!("the clip has no frames");
        }
        let home = &self.paths.home;
        let two = |fs: &[InputFrame]| {
            if fs.iter().any(|f| f.pads[1] != 0) {
                2
            } else {
                1
            }
        };
        let mut prefix = self.board.log(two(base));
        prefix.frames = base.to_vec();
        let (sha, _) = put_path(home, &prefix.to_text())?;
        let mut own = self.board.log(two(inputs));
        own.frames = inputs.to_vec();
        let (mut t, log) = clip_fields(&sha, base.len(), &own);
        t.insert(
            "state".into(),
            original_at(home, self.rom, &sha, base.len())?
                .state_hex()?
                .into(),
        );
        let s = Session::create(self.paths, self.game, session, None, self.rom_sha)?;
        match as_clip {
            Some(n) => s.put_clip(n, t, &log, note, "agent"),
            None => s.add_clip_meta(t, &log, note, "agent"),
        }
    }
}

/// A probe block for a saved clip: its start as the base, its own inputs as
/// steps. For turning a clip an agent made by hand into a recipe.
pub(crate) fn import(paths: &Paths, game: &str, session: &str, n: u64) -> Result<Outcome> {
    let dir = paths.home.join("mechanics").join(game).join(session);
    let t: toml::Table =
        toml::from_str(&fs::read_to_string(dir.join(format!("clips/{n:03}.toml")))?)?;
    let (text, start) = full_log(&paths.home, &dir, n)?;
    let log = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("clip {n}: {e}"))?;
    let len = t
        .get("frames")
        .and_then(toml::Value::as_integer)
        .map_or(log.frames.len() - start, |f| f as usize);
    let own = &log.frames[start..(start + len).min(log.frames.len())];
    // The board from the log's own spelling: an arcade log names its set.
    let board = match &log.arcade {
        Some(l) => Board::arcade(&l.system),
        None => Board::nes(),
    };
    let mut prefix = board.log(1);
    prefix.frames = log.frames[..start].to_vec();
    let (sha, _) = put_path(&paths.home, &prefix.to_text())?;
    let note = t.get("note").and_then(toml::Value::as_str).unwrap_or("");
    let steps = steps_of(&board, own);
    let mut s = String::new();
    s.push_str("[[probe]]\n");
    s.push_str(&format!("id = \"{session}-{n:03}\"\n"));
    s.push_str(&format!("session = \"{session}\"\nclips = [{n}]\n"));
    s.push_str(&format!("note = {}\n", toml::Value::String(note.into())));
    s.push_str(&format!("base = {{ path = \"{sha}\", at = {start} }}\n"));
    s.push_str(&format!("frames = {}\n", own.len()));
    s.push_str("steps = [\n");
    for st in &steps {
        s.push_str(&format!("  \"{st}\",\n"));
    }
    s.push_str("]\n");
    Ok(Outcome::ok(
        json!({"session": session, "clip": n, "path": sha, "at": start, "frames": own.len(), "steps": steps}),
        s,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w() -> Names {
        Names {
            watch: BTreeMap::from([("state".to_string(), 0x10u16), ("x".to_string(), 0x11u16)]),
            space: 0x800,
        }
    }

    #[test]
    fn steps_round_trip_through_run_lengths() {
        let f: Vec<InputFrame> = [0x40, 0x40, 0x41, 0, 0, 0]
            .iter()
            .map(|&p| press(p))
            .collect();
        let b = Board::nes();
        assert_eq!(steps_of(&b, &f), vec!["L 2", "LA 1", ". 3"]);
        assert_eq!(pads(&b, "LA").unwrap(), 0x41);
        assert_eq!(pads(&b, ".").unwrap(), 0);
        assert!(pads(&b, "X").is_err());
        assert_eq!(pads(&b, "R|L").unwrap(), 0x4080);
        assert_eq!(pads(&b, ".|A").unwrap(), 0x0100);
        let two: Vec<InputFrame> = [0x4080, 0x4080, 0x0001].iter().map(|&p| press(p)).collect();
        assert_eq!(two[0].pads, [0x80, 0x40, 0, 0]);
        assert_eq!(steps_of(&b, &two), vec!["R|L 2", "A 1"]);
    }

    #[test]
    fn measures_read_ram_by_frame() {
        // ram[i] is frame 100 + i.
        let mut ram = vec![vec![0u8; 0x800]; 6];
        for (i, v) in [1u8, 1, 255, 255, 1, 255].iter().enumerate() {
            ram[i][0x10] = *v;
            ram[i][0x11] = 20 + i as u8;
        }
        let l = Layout::dense(0x800);
        let m = |s: &str| measure(&w(), &l, s, 100, &ram).unwrap();
        assert_eq!(m("first state == 255"), json!(102));
        assert_eq!(m("last state == 255"), json!(105));
        assert_eq!(m("count state == 255"), json!(3));
        assert_eq!(m("any state == 255 and x == 21"), json!(false));
        assert_eq!(m("edges state == 255"), json!([102, 105]));
        assert_eq!(m("gaps state == 255"), json!([3]));
        assert_eq!(m("between state == 255 then state == 1"), json!(2));
        assert_eq!(m("value x at 103"), json!(23));
        assert_eq!(m("value x at +0"), json!(20));
        assert_eq!(m("max x"), json!(25));
        assert_eq!(m("first 0x11 >= 0x18"), json!(104));
        assert_eq!(m("first state == 7"), json!("none"));
        assert_eq!(m("first state == 255 and x >= 25"), json!(105));
        assert_eq!(m("changes state"), json!([102, 104, 105]));
        assert_eq!(m("intervals state"), json!([2, 1]));
        assert_eq!(m("pairs state == 255 then state == 1"), json!([2]));
        assert_eq!(m("values x at changes state"), json!([22, 24, 25]));
        assert_eq!(m("delta x while state == 255"), json!(3));
        assert_eq!(m("count state == 255 while x <= 23"), json!(2));
        assert_eq!(m("max x while state == 1"), json!(24));
        assert_eq!(m("range x while state == 255"), json!(3));
    }

    #[test]
    fn repeat_writes_its_steps_out() {
        let s = expand(&["repeat 2: R 2, U 3".into(), ". 1".into()]).unwrap();
        assert_eq!(s, vec!["R 2", "U 3", "R 2", "U 3", ". 1"]);
    }

    #[test]
    fn a_sweep_value_fills_steps_and_measures() {
        let c = Cell::from([("X".to_string(), 28), ("LY".to_string(), 128)]);
        assert_eq!(fill("walk_to x {X}", &c), "walk_to x 28");
        assert_eq!(fill("walk_to x {X+1} y {LY-12}", &c), "walk_to x 29 y 116");
        assert_eq!(fill("walk_to x {Q}", &c), "walk_to x {Q}");
        assert_eq!(fill("walk_to x {X}", &Cell::new()), "walk_to x {X}");
    }

    /// An arcade board's buttons are its columns, bit 0 first, with one set
    /// of controls; its addresses run to 0xFFFF; the measures keep only the
    /// addresses they read.
    #[test]
    fn an_arcade_board_reads_its_columns_and_its_address_space() {
        let b = Board::arcade("rbtapper");
        assert_eq!(pads(&b, "C").unwrap(), 0x01);
        assert_eq!(pads(&b, "RT").unwrap(), 0x44);
        assert!(pads(&b, "A").is_err());
        assert!(pads(&b, "C|S").is_err());
        assert_eq!((b.right(), b.left()), (0x04, 0x08));
        let f: Vec<InputFrame> = [1u16, 1, 0x40].iter().map(|&p| press(p)).collect();
        assert_eq!(steps_of(&b, &f), vec!["C 2", "T 1"]);
        let names = Names {
            watch: BTreeMap::from([("lives".to_string(), 0xe0f0u16)]),
            space: b.space,
        };
        let l = layout_of(
            &b,
            &names,
            &[
                "count 0xe100 == 3 while 0xe200 > 0".into(),
                "max lives".into(),
            ],
        )
        .unwrap();
        assert_eq!(l.addrs, vec![0xe0f0, 0xe100, 0xe200]);
        assert!(addr(&w(), "0x900").is_err());
    }

    /// With the owner's rbtapper romset on this machine: a made-up probe
    /// plays presses on gp-arcade from power-on and reads its RAM by Z80
    /// address. Skipped without it.
    #[test]
    fn a_probe_runs_on_an_arcade_board() {
        let Some(zip) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let rom = fs::read(zip).unwrap();
        let board = Board::of_rom(&rom).unwrap();
        assert_eq!(board.arcade.as_ref().unwrap().system, "rbtapper");
        let f: File = toml::from_str(
            r#"
[watch]
w = 0xe000

[[probe]]
id = "coin"
steps = [". 30", "C 4", ". 30", "mark", "S 4", ". 20"]
check = [{ measure = "min w" }, { measure = "value w at +0" }]
"#,
        )
        .unwrap();
        let names = names_of(&board, &f);
        let p = &f.probe[0];
        let ms: Vec<String> = p.check.iter().map(|c| c.measure.clone()).collect();
        let layout = layout_of(&board, &names, &ms).unwrap();
        let o = OriginalRuntime::of_rom(&rom).unwrap();
        let start = peek(&o, 0xe000).unwrap();
        let played = play(o, &board, &names, &layout, &[], &[], &p.steps, None).unwrap();
        assert_eq!(played.inputs.len(), 88);
        assert_eq!(played.mark, 64);
        assert_eq!(played.inputs[30].pads[0], 0x01);
        assert_eq!(played.ram.len(), 89);
        assert_eq!(layout.get(&played.ram[0], 0xe000), start);
        for m in &ms {
            let v = measure(&names, &layout, m, played.mark, &played.ram[played.mark..]).unwrap();
            assert!(v.is_u64(), "{m}: {v}");
        }
    }
}
