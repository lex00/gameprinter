//! `gp code`: a ROM's code read with what runs of it did, for a planner
//! working out what an element's routines do (.claude/skills/
//! element-extraction/SKILL.md).
//!
//! `gp code listing` prints the routines of the static harvest's code map
//! (`gp model static --no-run`'s 249 routines on Donkey Kong) with how
//! often each instruction ran and how often each branch was taken and
//! fell through, over chosen clips and frames (`gp_events::profile`).
//! `gp code tables` finds the ROM tables the code reads with one RAM byte
//! (a stage counter) as the index, with each table's entry size and the
//! entry each value of the byte reads, as asset map entries to name.
//!
//! Nothing here is kept in the repo: a listing goes to standard output, or
//! with `--write` to a file outside any git work tree.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use gp_events::names::Namer;
use gp_events::profile::{InsnStat, Profile, run_window_on};
use gp_events::store::Store as EventStore;
use gp_evidence::Paths;
use gp_nes::codemap::{Board, Insn, Mn, Mode, Va, decode, va_add, va_cpu, va_hex, va_parse};
use serde_json::json;

use crate::Outcome;
use crate::reach::ClipInput;

#[derive(Subcommand, Debug)]
pub enum CodeCmd {
    /// Print the ROM's routines (the static harvest's code map) with how
    /// often each instruction ran and each branch's ways (taken, fell),
    /// over the clips and frames chosen. Without clips, the code alone.
    /// Writes nothing in the repo: the listing goes to standard output, or
    /// to --write's file outside any git work tree.
    Listing(ListingArgs),
    /// Find the ROM tables the code reads with a RAM byte as the index
    /// (`lda $53; asl a; tax; lda table,x`). Each is listed with its rows
    /// and their readers. The row each value of the byte reads is named,
    /// with where a pointer row points. An asset map entry follows for
    /// each table.
    Tables(TablesArgs),
}

pub fn run(cmd: &CodeCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        CodeCmd::Listing(a) => listing(a, paths),
        CodeCmd::Tables(a) => tables(a),
    }
}

/// Which runs to count: clip sets, the clips chosen of them, and a window
/// of frames.
#[derive(Args, Debug, Clone, Default)]
pub struct Runs {
    /// Clip sets (clips.toml) whose clips run, each from power-on.
    #[arg(long = "clips", visible_alias = "clip", value_name = "SET")]
    pub clips: Vec<PathBuf>,
    /// Only these clips: an ID, or a prefix ending in `*`. Repeat for more.
    #[arg(long, value_name = "ID")]
    pub only: Vec<String>,
    /// Count only frames A to B of each clip, numbered from power-on as
    /// `gp reach --detail` numbers them: `A..B` without B, `A..=B` with it,
    /// `A..` to the clip's end, `..` the whole clip (the default).
    #[arg(long, value_name = "A..B")]
    pub frames: Option<String>,
    /// The store of counted windows (default $GP_HOME/work/events).
    #[arg(long)]
    pub store: Option<PathBuf>,
    /// Count every window again, reading nothing from the store.
    #[arg(long)]
    pub no_store: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ListingArgs {
    /// The iNES ROM, or an arcade romset (a MAME zip): both Z80s' code,
    /// by chip.
    pub rom: PathBuf,
    #[command(flatten)]
    pub runs: Runs,
    /// Only the routines holding these addresses (`D2FD`, `$E7AE`,
    /// `03:8123`). Repeat for more.
    #[arg(long, value_name = "ADDR")]
    pub routine: Vec<String>,
    /// List every routine, not only those that ran.
    #[arg(long)]
    pub all: bool,
    /// Instead of the code, every instruction the runs saw write this RAM
    /// byte (`E70F`, `$0770`), block clears and copies (`ldir`) and pushes
    /// included, with its count, its routine and where the runs called
    /// that routine from (on an arcade romset). The first frame is the
    /// earliest over the clips. Needs --clips. Repeat for more.
    #[arg(long, value_name = "ADDR", requires = "clips")]
    pub writes: Vec<String>,
    /// Write the listing to this file (outside any git work tree) instead
    /// of standard output.
    #[arg(long)]
    pub write: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct TablesArgs {
    /// The iNES ROM, or an arcade romset (a MAME zip), whose tables are
    /// found from a run: give --input.
    pub rom: PathBuf,
    /// For an arcade romset: the input log whose run reads the tables
    /// (the index byte's reads are followed through it).
    #[arg(long)]
    pub input: Option<PathBuf>,
    /// The RAM byte used as the index: `$53` (Donkey Kong's stage).
    #[arg(long)]
    pub index: String,
    /// The values the byte takes: `1..=4`, `1..5` or `1,2,4`. Default: from
    /// the code, the smallest to the largest constant it compares the byte
    /// with or stores in it.
    #[arg(long)]
    pub values: Option<String>,
    /// The asset map names' prefix.
    #[arg(long, default_value = "stage")]
    pub name: String,
}

/// A window of frames: the first and the last (`u64::MAX`: the clip's end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub first: u64,
    pub last: u64,
}

impl Window {
    pub const WHOLE: Window = Window {
        first: 1,
        last: u64::MAX,
    };

    pub fn text(&self) -> String {
        match (self.first, self.last) {
            (1, u64::MAX) => "whole clips".into(),
            (a, u64::MAX) => format!("frames {a}.."),
            (a, b) => format!("frames {a}..={b}"),
        }
    }
}

/// `A..B`, `A..=B`, `A..` or `..`.
pub(crate) fn parse_window(s: &str) -> Result<Window> {
    let (a, b) = s
        .split_once("..")
        .with_context(|| format!("{s:?}: a window is A..B, A..=B, A.. or .."))?;
    let first: u64 = if a.is_empty() { 1 } else { a.parse()? };
    let last = match b {
        "" => u64::MAX,
        _ => match b.strip_prefix('=') {
            Some(b) => b.parse()?,
            None => b
                .parse::<u64>()?
                .checked_sub(1)
                .context("an empty window")?,
        },
    };
    if last < first {
        bail!("the window {s} is empty");
    }
    Ok(Window { first, last })
}

/// The clips `only` names, in their order; all when it is empty. A pattern
/// that names no clip is an error.
pub(crate) fn select(clips: Vec<ClipInput>, only: &[String]) -> Result<Vec<ClipInput>> {
    if only.is_empty() {
        return Ok(clips);
    }
    let hit = |p: &str, id: &str| match p.strip_suffix('*') {
        Some(pre) => id.starts_with(pre),
        None => id == p,
    };
    for p in only {
        if !clips.iter().any(|c| hit(p, &c.id)) {
            bail!("--only {p}: no clip of the sets given has that ID");
        }
    }
    Ok(clips
        .into_iter()
        .filter(|c| only.iter().any(|p| hit(p, &c.id)))
        .collect())
}

/// The counts of the clips' windows, merged.
pub(crate) struct Counted {
    pub profile: Profile,
    /// Each clip's ID and the frames counted of it.
    pub clips: Vec<(String, u64)>,
    /// Windows counted now; the rest came from the store.
    pub ran: usize,
    pub seconds: f64,
}

/// Counts window `w` of each clip, each distinct run once, on the free
/// helper threads; a window the store has isn't run again.
pub(crate) fn count(
    home: &Path,
    rom: &[u8],
    clips: &[ClipInput],
    w: Window,
    store: Option<&EventStore>,
) -> Result<Counted> {
    let t0 = Instant::now();
    let rom_sha = crate::template::sha256_hex(rom);
    let mut keys = Vec::new();
    for c in clips {
        let n = c.inputs.len() as u64;
        let (first, last) = (w.first.max(1), w.last.min(n));
        keys.push((c.key()?, first, last));
    }
    let mut first_of: BTreeMap<&(String, u64, u64), usize> = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        first_of.entry(k).or_insert(i);
    }
    let runs: Vec<usize> = first_of.values().copied().collect();
    let done = crate::pool::map(
        &runs,
        || (),
        |_, _, &i| -> std::result::Result<(Profile, bool), String> {
            let (input, first, last) = &keys[i];
            if first > last {
                return Ok((Profile::default(), false));
            }
            if let Some(p) = store.and_then(|s| s.profile(&rom_sha, input, *first, *last)) {
                return Ok((p, false));
            }
            let c = &clips[i];
            let (nes, at) = console_before(home, rom, c, *first)
                .map_err(|e| format!("clip {}: {e:#}", c.id))?;
            let p = run_window_on(nes, at, &c.inputs, *first, *last)
                .map_err(|e| format!("clip {}: {e}", c.id))?;
            if let Some(s) = store {
                s.put_profile(&rom_sha, input, *first, *last, &p)
                    .map_err(|e| e.to_string())?;
            }
            Ok((p, true))
        },
        &mut |_| {},
    );
    let mut by_key: BTreeMap<&(String, u64, u64), Profile> = BTreeMap::new();
    let mut ran = 0;
    for (&i, r) in runs.iter().zip(done) {
        let (p, fresh) = r.map_err(anyhow::Error::msg)?;
        ran += usize::from(fresh);
        by_key.insert(&keys[i], p);
    }
    let mut profile = Profile::default();
    let mut listed = Vec::new();
    for (i, c) in clips.iter().enumerate() {
        let p = &by_key[&keys[i]];
        listed.push((c.id.clone(), p.frames));
        profile.merge(p);
    }
    Ok(Counted {
        profile,
        clips: listed,
        ran,
        seconds: t0.elapsed().as_secs_f64(),
    })
}

/// The event store a command's `--store` and `--no-store` name.
pub(crate) fn event_store(
    store: &Option<PathBuf>,
    no_store: bool,
    paths: &Paths,
) -> Option<EventStore> {
    (!no_store).then(|| {
        EventStore::at(
            store
                .clone()
                .unwrap_or_else(|| paths.home.join("work").join("events")),
        )
    })
}

/// One routine of the code map.
#[derive(Debug, Clone)]
pub(crate) struct Routine {
    pub name: String,
    pub at: Va,
    pub size: u32,
    pub element: String,
}

/// The ROM's code map, as `gp model static --no-run` reads it (no asset
/// map, no runs), so every tool and every side of a comparison names the
/// same routines.
pub(crate) struct CodeMap {
    pub board: Board,
    pub namer: Namer,
    pub routines: Vec<Routine>,
}

impl CodeMap {
    /// The code map of an iNES ROM. `ran` are counts of runs of it, whose
    /// addresses show the regions a board switches beyond its usual layout.
    pub fn of(rom: &[u8], ran: &[&Profile]) -> Result<CodeMap> {
        let (prg, mapper) = gp_nes::harvest::ines_prg(rom).context("not an iNES ROM")?;
        let mut board = Board::of(prg, mapper);
        let mut windows = vec![0u64; prg.len().div_ceil(0x400)];
        for p in ran {
            for (&o, s) in &p.insns {
                if let Some(w) = windows.get_mut(o as usize / 0x400) {
                    *w |= 1u64 << (s.cpu >> 10);
                }
            }
        }
        board.switched = Namer::observed_switched(&board, &windows);
        let inp = crate::model_static::input_of(None, prg)?;
        let h = gp_nes::harvest::harvest_board(&board, &inp);
        let mut routines: Vec<Routine> = h
            .routines
            .iter()
            .filter_map(|r| {
                Some(Routine {
                    name: r.name.clone(),
                    at: va_parse(&r.addr)?,
                    size: r.size,
                    element: r.element.clone(),
                })
            })
            .collect();
        routines.sort_by_key(|r| r.at);
        Ok(CodeMap {
            namer: Namer::of_board(board.clone()),
            board,
            routines,
        })
    }

    /// The routine an address is in.
    pub fn routine_of(&self, v: Va) -> Option<usize> {
        let i = self
            .routines
            .partition_point(|r| r.at <= v)
            .checked_sub(1)?;
        let r = &self.routines[i];
        (v < r.at + r.size).then_some(i)
    }

    /// The instruction at a banked address, as the code map decodes it.
    pub fn insn_at(&self, p: Va) -> Option<Insn> {
        let b = &self.board;
        let (mn, mode) = decode(b.read(p))?;
        let byte = |d: u16| b.read(va_add(p, d));
        let word = u16::from_le_bytes([byte(1), byte(2)]);
        let arg = match mode {
            Mode::Imp | Mode::Acc => 0,
            Mode::Rel => b.target(
                p,
                va_cpu(p).wrapping_add(2).wrapping_add(byte(1) as i8 as u16),
            ),
            Mode::Abs if matches!(mn, Mn::Jmp | Mn::Jsr) => b.target(p, word),
            m if m.len() == 3 => u32::from(word),
            _ => u32::from(byte(1)),
        };
        Some(Insn {
            pc: p,
            mn,
            mode,
            arg,
        })
    }

    /// A routine's instructions in order; a byte that decodes to none is
    /// `None` and the walk goes on at the next byte.
    pub fn body(&self, i: usize) -> Vec<(Va, Option<Insn>)> {
        let r = &self.routines[i];
        let end = u32::from(va_cpu(r.at)) + r.size;
        let mut out = Vec::new();
        let mut p = r.at;
        while u32::from(va_cpu(p)) < end && u32::from(va_cpu(p)) >= 0x8000 {
            let ins = self.insn_at(p);
            let len = ins.map_or(1, |i| i.mode.len());
            out.push((p, ins));
            if u32::from(va_cpu(p)) + u32::from(len) > 0xFFFF {
                break;
            }
            p = va_add(p, len);
        }
        out
    }

    /// The counts by banked address. An instruction run where the board's
    /// view has another byte (a board the code map doesn't follow) has
    /// none.
    pub fn by_address(&self, p: &Profile) -> BTreeMap<Va, InsnStat> {
        let mut out = BTreeMap::new();
        for (&o, s) in &p.insns {
            if let Some(v) = self.namer.va(o, s.cpu) {
                out.insert(v, s.clone());
            }
        }
        out
    }
}

/// How a branch went, in a word.
pub(crate) fn ways(s: &InsnStat) -> &'static str {
    match (s.taken > 0, s.fell > 0) {
        (true, true) => "both",
        (true, false) => "taken only",
        (false, true) => "fell only",
        (false, false) => "neither",
    }
}

/// The console to count a window of `c` from frame `first` on: from the
/// start cache ([`crate::clip::cache::start_state`]) a few frames before
/// it, else powered on; and the frames of the input it stands after. The
/// counts are the same as from power-on.
pub(crate) fn console_before(
    home: &Path,
    rom: &[u8],
    c: &ClipInput,
    first: u64,
) -> Result<(gp_nes::Nes, usize)> {
    use crate::clip::cache;
    let before = usize::try_from(first.max(1) - 1).unwrap_or(usize::MAX);
    let at = before.saturating_sub(cache::START_LEAD).min(c.inputs.len());
    if at == 0 || !cache::starts_on() {
        return Ok((gp_nes::Nes::power_on(rom, c.power_on)?, 0));
    }
    Ok((start_state(home, rom, c.power_on, &c.inputs, at)?, at))
}

/// The console after the first `n` frames of `frames` from power-on, from
/// the start cache when it holds them, else run there and kept. (The old
/// tool reached it through its checkpoint chain; gameprinter's start cache
/// keeps starts, not checkpoints, so the frames run from power-on once.)
pub(crate) fn start_state(
    home: &Path,
    rom: &[u8],
    power_on: gp_evidence::manifest::PowerOn,
    frames: &[gp_evidence::input_log::InputFrame],
    n: usize,
) -> Result<gp_nes::Nes> {
    use crate::clip::cache;
    let n = n.min(frames.len());
    let run = || -> Result<gp_nes::Nes> {
        let mut nes = gp_nes::Nes::power_on(rom, power_on)?;
        nes.set_video(false);
        for f in &frames[..n] {
            nes.step(*f)?;
        }
        Ok(nes)
    };
    if !cache::starts_on() {
        return run();
    }
    let key = cache::start_key(rom, &power_on, frames, n);
    if let Some(s) = cache::read_start(home, &key, rom, power_on) {
        return s.nes(rom, power_on);
    }
    let saved = cache::write_start(home, &key, &run()?)?;
    saved.nes(rom, power_on)
}

/// The clips a command's runs name, from power-on.
fn clips_of(runs: &Runs, paths: &Paths) -> Result<Vec<ClipInput>> {
    let all = crate::reach::clip_inputs(&runs.clips, paths)?;
    select(all, &runs.only)
}

fn listing(a: &ListingArgs, paths: &Paths) -> Result<Outcome> {
    use std::fmt::Write as _;
    if let Some(w) = &a.write
        && let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(w)
    {
        bail!(
            "--write {}: inside the git work tree {}; a listing of a ROM's code stays outside the repo (under $GP_HOME/work)",
            w.display(),
            repo.display()
        );
    }
    let rom = fs::read(&a.rom).with_context(|| format!("reading {}", a.rom.display()))?;
    if crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade {
        return listing_z80(a, paths, &rom);
    }
    let window = match &a.runs.frames {
        Some(s) => parse_window(s)?,
        None => Window::WHOLE,
    };
    if !a.writes.is_empty() {
        let addrs = write_addrs(&a.writes)?;
        let clips = clips_of(&a.runs, paths)?;
        let (text, mut doc) = writes_nes(&paths.home, &rom, &clips, window, &addrs)?;
        doc["window"] = json!(window.text());
        return Ok(Outcome::ok(doc, text.trim_end().to_owned()));
    }
    let counted = if a.runs.clips.is_empty() {
        if !a.runs.only.is_empty() || a.runs.frames.is_some() {
            bail!("--only and --frames choose clips of --clips sets; none given");
        }
        None
    } else {
        let clips = clips_of(&a.runs, paths)?;
        let store = event_store(&a.runs.store, a.runs.no_store, paths);
        Some(count(&paths.home, &rom, &clips, window, store.as_ref())?)
    };
    let map = CodeMap::of(
        &rom,
        &counted.iter().map(|c| &c.profile).collect::<Vec<_>>(),
    )?;
    let ran = counted
        .as_ref()
        .map(|c| map.by_address(&c.profile))
        .unwrap_or_default();
    let wanted: Option<BTreeSet<usize>> = if a.routine.is_empty() {
        None
    } else {
        let mut s = BTreeSet::new();
        for r in &a.routine {
            let t = r.trim().trim_start_matches('$');
            let v = va_parse(t).with_context(|| format!("--routine {r}: not an address"))?;
            let i = map
                .routine_of(v)
                .with_context(|| format!("--routine {r}: in no routine of the code map"))?;
            s.insert(i);
        }
        Some(s)
    };
    let mut text = String::new();
    if let Some(c) = &counted {
        let _ = writeln!(
            text,
            "; {} clips, {}, {} frames counted ({} counted now, {} from the store, {:.1} s)",
            c.clips.len(),
            window.text(),
            c.profile.frames,
            c.ran,
            c.clips.len() - c.ran,
            c.seconds
        );
    }
    let mut rows = Vec::new();
    let (mut listed, mut run_routines, mut insns_total, mut insns_run) = (0, 0, 0, 0);
    let (mut both, mut one_way, mut not_run) = (0, 0, 0);
    let mut in_map: BTreeSet<Va> = BTreeSet::new();
    for (i, r) in map.routines.iter().enumerate() {
        let body = map.body(i);
        in_map.extend(body.iter().map(|(p, _)| *p));
        if let Some(w) = &wanted
            && !w.contains(&i)
        {
            continue;
        }
        let n_run = body.iter().filter(|(p, _)| ran.contains_key(p)).count();
        if counted.is_some() && n_run == 0 && !a.all && wanted.is_none() {
            continue;
        }
        listed += 1;
        run_routines += usize::from(n_run > 0);
        let n_insns = body.iter().filter(|(_, i)| i.is_some()).count();
        insns_total += n_insns;
        insns_run += n_run;
        let entered = ran.get(&r.at).map_or(0, |s| s.count);
        let _ = write!(
            text,
            "\n; {} ({}) {}-{}, {n_insns} instructions",
            r.name,
            r.element,
            va_hex(r.at),
            va_hex(va_add(r.at, (r.size.max(1) - 1) as u16)),
        );
        if counted.is_some() {
            let _ = write!(text, ", {n_run} run; first instruction run {entered} times");
        }
        text.push('\n');
        let mut lines = Vec::new();
        for (p, ins) in &body {
            let stat = ran.get(p);
            let Some(ins) = ins else {
                let _ = writeln!(text, "{}  .byte ${:02X}", va_hex(*p), map.board.read(*p));
                continue;
            };
            let mut line = format!("{}  {:<16}", va_hex(*p), ins.text());
            let mut row = json!({"pc": va_hex(*p), "insn": ins.text()});
            match stat {
                Some(s) => {
                    let _ = write!(line, " {:>8}", s.count);
                    row["count"] = json!(s.count);
                    if ins.mode == Mode::Rel {
                        let _ = write!(line, "  taken {}, fell {} ({})", s.taken, s.fell, ways(s));
                        row["taken"] = json!(s.taken);
                        row["fell"] = json!(s.fell);
                        if s.taken > 0 && s.fell > 0 {
                            both += 1;
                        } else {
                            one_way += 1;
                        }
                    }
                }
                None if counted.is_some() => {
                    line.push_str("        -");
                    if ins.mode == Mode::Rel {
                        not_run += 1;
                    }
                }
                None => {}
            }
            text.push_str(line.trim_end());
            text.push('\n');
            lines.push(row);
        }
        rows.push(json!({
            "routine": r.name, "addr": va_hex(r.at), "size": r.size,
            "element": r.element, "instructions": n_insns, "run": n_run,
            "entered": entered, "lines": lines,
        }));
    }
    // What ran outside every routine of the map.
    let outside: Vec<(&Va, &InsnStat)> = ran.iter().filter(|(p, _)| !in_map.contains(p)).collect();
    if !outside.is_empty() && wanted.is_none() {
        let _ = writeln!(
            text,
            "\n; outside the code map: {} instructions run",
            outside.len()
        );
        for (p, s) in &outside {
            let t = map.insn_at(**p).map(|i| i.text()).unwrap_or_default();
            let _ = writeln!(text, "{}  {:<16} {:>8}", va_hex(**p), t, s.count);
        }
    }
    let _ = writeln!(
        text,
        "\n; {listed} routines listed{}, {insns_run} of {insns_total} instructions run; branches run both ways {both}, one way {one_way}, not run {not_run}",
        if counted.is_some() {
            format!(" ({run_routines} ran)")
        } else {
            String::new()
        }
    );
    let doc = json!({
        "rom_sha256": crate::template::sha256_hex(&rom),
        "window": window.text(),
        "clips": counted.as_ref().map(|c| c.clips.iter().map(|(id, f)| json!({"id": id, "frames": f})).collect::<Vec<_>>()),
        "routines_listed": listed,
        "routines_run": run_routines,
        "instructions": insns_total,
        "instructions_run": insns_run,
        "branches": {"both": both, "one_way": one_way, "not_run": not_run},
        "outside_code_map": outside.len(),
        "listing": rows,
    });
    if let Some(w) = &a.write {
        if let Some(d) = w.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(w, &text)?;
        let summary = text.lines().last().unwrap_or_default().to_owned();
        return Ok(Outcome::ok(
            doc,
            format!("{summary}\nwritten to {}", w.display()),
        ));
    }
    Ok(Outcome::ok(doc, text.trim_end().to_owned()))
}

/// `--writes`' addresses.
fn write_addrs(given: &[String]) -> Result<Vec<u16>> {
    given
        .iter()
        .map(|r| {
            u16::from_str_radix(r.trim_start_matches('$').trim_start_matches("0x"), 16)
                .with_context(|| format!("--writes {r} isn't an address"))
        })
        .collect()
}

/// `gp code listing --writes` on an iNES ROM: each clip's window run with
/// every instruction watched, and each instruction that wrote one of
/// `addrs` (a store, a read-modify-write, or a push, `jsr` or `brk` onto
/// the stack) counted with the first frame it did. Internal RAM is matched
/// through its mirrors. An interrupt's own pushes have no instruction and
/// aren't counted.
fn writes_nes(
    home: &Path,
    rom: &[u8],
    clips: &[ClipInput],
    window: Window,
    addrs: &[u16],
) -> Result<(String, serde_json::Value)> {
    use std::fmt::Write as _;
    use tetanes_core::cpu::Cpu;
    use tetanes_core::cpu::instr::Instr;
    let norm = |a: u16| if a < 0x2000 { a & 0x7ff } else { a };
    let want: BTreeSet<u16> = addrs.iter().map(|&a| norm(a)).collect();
    // By address written, then the instruction's PRG offset and CPU
    // address: how many bytes, and the first frame.
    let mut found: BTreeMap<(u16, u32, u16), (u64, u64)> = BTreeMap::new();
    let feed = |i: &gp_evidence::input_log::InputFrame| gp_evidence::input_log::InputFrame {
        polled: true,
        pads: i.pads,
        reset: i.reset,
    };
    for c in clips {
        let n = c.inputs.len() as u64;
        let (first, last) = (window.first.max(1), window.last.min(n));
        if first > last {
            continue;
        }
        let (mut nes, at) =
            console_before(home, rom, c, first).with_context(|| format!("clip {}", c.id))?;
        nes.set_video(false);
        let layout = gp_events::Layout::of(nes.bus())?;
        for i in &c.inputs[at..(first - 1) as usize] {
            nes.step(feed(i))?;
        }
        for f in first..=last {
            nes.step_with(feed(&c.inputs[(f - 1) as usize]), |bus| {
                let pc = bus.cpu.pc;
                let r = Cpu::INSTR_REF[usize::from(bus.peek(pc))];
                let pushes = match r.instr {
                    Instr::PHA | Instr::PHP => 1,
                    Instr::JSR => 2,
                    Instr::BRK => 3,
                    _ => 0,
                };
                let mut hit = |a: u16| {
                    if want.contains(&norm(a)) {
                        let o = layout
                            .rom_offset(bus, pc)
                            .map_or(gp_events::NO_ROM, |o| o as u32);
                        let e = found.entry((norm(a), o, pc)).or_insert((0, f));
                        e.0 += 1;
                    }
                };
                for i in 0..pushes {
                    hit(0x100 | u16::from(bus.cpu.sp.wrapping_sub(i)));
                }
                if gp_events::footprint::writes_memory(r.instr, r.addr_mode)
                    && let Some(a) = gp_events::footprint::target(bus, pc, r.instr, r.addr_mode)
                {
                    hit(a);
                }
            })?;
        }
    }
    let map = CodeMap::of(rom, &[])?;
    let mut text = String::new();
    let mut rows = Vec::new();
    for &addr in &want {
        let mine: Vec<_> = found
            .range((addr, 0, 0)..=(addr, u32::MAX, u16::MAX))
            .collect();
        if mine.is_empty() {
            let _ = writeln!(text, "; ${addr:04X}: no run wrote it");
            rows.push(json!({"addr": format!("{addr:04X}"), "writes": 0, "writers": []}));
            continue;
        }
        let total: u64 = mine.iter().map(|(_, v)| v.0).sum();
        let first = mine.iter().map(|(_, v)| v.1).min();
        let _ = writeln!(
            text,
            "; ${addr:04X}: {total} writes by {} instructions{}",
            mine.len(),
            first.map_or(String::new(), |f| format!(", first on frame {f}")),
        );
        let mut writers = Vec::new();
        for (&(_, o, pc), &(count, first)) in mine {
            let va = map.namer.va(o, pc);
            let name = va.map_or(format!("{pc:04X}"), va_hex);
            let insn = va
                .and_then(|v| map.insn_at(v))
                .map(|i| i.text())
                .unwrap_or_default();
            let routine = va
                .and_then(|v| map.routine_of(v))
                .map(|i| map.routines[i].name.clone());
            let mut line = format!("{name}  {insn:<16} {count:>8}  first on frame {first}");
            if let Some(r) = &routine {
                let _ = write!(line, "  in {r}");
            }
            let _ = writeln!(text, "{line}");
            writers.push(json!({
                "pc": name, "insn": insn, "writes": count, "first_frame": first,
                "routine": routine,
            }));
        }
        text.push('\n');
        rows.push(json!({
            "addr": format!("{addr:04X}"),
            "writes": total,
            "first_frame": first,
            "writers": writers,
        }));
    }
    Ok((text, json!({ "writes": rows })))
}

/// `gp code listing` on an arcade romset (crate::code_z80).
fn listing_z80(a: &ListingArgs, paths: &Paths, rom: &[u8]) -> Result<Outcome> {
    let set = gp_arcade::Romset::from_zip(rom)?;
    let window = match &a.runs.frames {
        Some(s) => parse_window(s)?,
        None => Window::WHOLE,
    };
    let ran = if a.runs.clips.is_empty() {
        None
    } else {
        let clips = clips_of(&a.runs, paths)?;
        Some(crate::code_z80::run_clips(
            &set,
            &clips,
            (window.first, window.last),
        )?)
    };
    if !a.writes.is_empty() {
        let addrs = write_addrs(&a.writes)?;
        let ran = ran.context("--writes needs --clips: the runs whose writes are listed")?;
        let (text, mut doc) = crate::code_z80::writes(&set, &ran, &addrs);
        doc["window"] = json!(window.text());
        return Ok(Outcome::ok(doc, text.trim_end().to_owned()));
    }
    let wanted = a
        .routine
        .iter()
        .map(|r| {
            u16::from_str_radix(r.trim_start_matches('$').trim_start_matches("0x"), 16)
                .with_context(|| format!("--routine {r} isn't an address"))
        })
        .collect::<Result<Vec<_>>>()?;
    let wanted: Vec<u16> = if a.all && wanted.is_empty() {
        let mut all = Vec::new();
        for c in 0..2 {
            let lv = ran.as_ref().map(|r| &r.cpus[c]);
            all.extend(crate::code_z80::CodeMap::of(&set, c, lv).routines.keys());
        }
        all
    } else {
        wanted
    };
    let (text, mut doc) = crate::code_z80::listing(&set, ran.as_ref(), &wanted, None);
    doc["window"] = json!(window.text());
    if let Some(w) = &a.write {
        if let Some(d) = w.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(w, &text)?;
        return Ok(Outcome::ok(
            doc,
            format!("{} lines written to {}", text.lines().count(), w.display()),
        ));
    }
    Ok(Outcome::ok(doc, text.trim_end().to_owned()))
}

/// An index register's value as a line in the index byte: `m * byte + b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Lin {
    m: i32,
    b: i32,
}

/// What a path knows: the registers as lines in the index byte, and the
/// carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Regs {
    a: Option<Lin>,
    x: Option<Lin>,
    y: Option<Lin>,
    c: Option<bool>,
}

/// A table read at an index made from the byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TableRead {
    base: u16,
    lin: Lin,
    pc: Va,
    /// Where the path that read it loaded the byte.
    from: Va,
}

/// Paths followed from one load of the byte before the reader gives up.
const STEPS: usize = 400;
/// Calls deep a path follows a register into.
const CALLS: u8 = 3;

/// Follows every path from a load of the index byte at `start` and lists
/// the ROM tables read at an index made from it, with the constants the
/// byte is compared with.
fn follow(
    map: &CodeMap,
    idx: u16,
    start: Va,
    reads: &mut BTreeSet<TableRead>,
    cmps: &mut BTreeSet<u8>,
) {
    let mut seen: BTreeSet<(Va, Regs)> = BTreeSet::new();
    let Some(first) = map.insn_at(start) else {
        return;
    };
    let one = Some(Lin { m: 1, b: 0 });
    let mut regs = Regs {
        a: None,
        x: None,
        y: None,
        c: None,
    };
    match first.mn {
        Mn::Lda => regs.a = one,
        Mn::Ldx => regs.x = one,
        Mn::Ldy => regs.y = one,
        _ => return,
    }
    let mut todo = vec![(first.next(), regs, 0u8)];
    let mut steps = 0;
    while let Some((p, mut r, depth)) = todo.pop() {
        steps += 1;
        if steps > STEPS || va_cpu(p) < 0x8000 {
            continue;
        }
        if r.a.is_none() && r.x.is_none() && r.y.is_none() {
            continue;
        }
        if !seen.insert((p, r)) {
            continue;
        }
        let Some(ins) = map.insn_at(p) else { continue };
        let arg16 = ins.arg as u16;
        let is_idx = matches!(ins.mode, Mode::Zp | Mode::Abs) && arg16 == idx;
        // A table read: an absolute address in ROM indexed by a register
        // the path knows.
        let reg = match ins.mode {
            Mode::AbsX => r.x,
            Mode::AbsY => r.y,
            _ => None,
        };
        let reads_memory = !matches!(ins.mn, Mn::Sta | Mn::Stx | Mn::Sty | Mn::Jmp | Mn::Jsr);
        if let Some(lin) = reg
            && arg16 >= 0x8000
            && reads_memory
            && lin.m != 0
        {
            reads.insert(TableRead {
                base: arg16,
                lin,
                pc: p,
                from: start,
            });
        }
        let add = |l: Option<Lin>, d: i32| l.map(|l| Lin { m: l.m, b: l.b + d });
        let next = ins.next();
        match ins.mn {
            Mn::Lda => {
                r.a = if is_idx {
                    Some(Lin { m: 1, b: 0 })
                } else {
                    None
                }
            }
            Mn::Ldx => {
                r.x = if is_idx {
                    Some(Lin { m: 1, b: 0 })
                } else {
                    None
                }
            }
            Mn::Ldy => {
                r.y = if is_idx {
                    Some(Lin { m: 1, b: 0 })
                } else {
                    None
                }
            }
            Mn::Tax => r.x = r.a,
            Mn::Tay => r.y = r.a,
            Mn::Txa => r.a = r.x,
            Mn::Tya => r.a = r.y,
            Mn::Inx => r.x = add(r.x, 1),
            Mn::Dex => r.x = add(r.x, -1),
            Mn::Iny => r.y = add(r.y, 1),
            Mn::Dey => r.y = add(r.y, -1),
            Mn::Clc => r.c = Some(false),
            Mn::Sec => r.c = Some(true),
            Mn::Asl if ins.mode == Mode::Acc => {
                r.a = r.a.map(|l| Lin {
                    m: 2 * l.m,
                    b: 2 * l.b,
                });
                r.c = None;
            }
            Mn::Adc if ins.mode == Mode::Imm => {
                r.a = match r.c {
                    Some(c) => add(r.a, ins.arg as i32 + i32::from(c)),
                    None => None,
                };
                r.c = None;
            }
            Mn::Sbc if ins.mode == Mode::Imm => {
                r.a = match r.c {
                    Some(c) => add(r.a, -(ins.arg as i32) - i32::from(!c)),
                    None => None,
                };
                r.c = None;
            }
            Mn::Cmp if ins.mode == Mode::Imm => {
                if r.a == Some(Lin { m: 1, b: 0 }) {
                    cmps.insert(ins.arg as u8);
                }
                r.c = None;
            }
            Mn::Cpx | Mn::Cpy if ins.mode == Mode::Imm => {
                let reg = if ins.mn == Mn::Cpx { r.x } else { r.y };
                if reg == Some(Lin { m: 1, b: 0 }) {
                    cmps.insert(ins.arg as u8);
                }
                r.c = None;
            }
            Mn::Cmp | Mn::Cpx | Mn::Cpy | Mn::Bit => r.c = None,
            Mn::Sta
            | Mn::Stx
            | Mn::Sty
            | Mn::Nop
            | Mn::Pha
            | Mn::Php
            | Mn::Cld
            | Mn::Sed
            | Mn::Cli
            | Mn::Sei
            | Mn::Clv => {}
            // A store to the byte itself leaves it what the register held.
            Mn::Inc | Mn::Dec | Mn::Rol | Mn::Ror | Mn::Lsr | Mn::Asl => {
                if ins.mode == Mode::Acc {
                    r.a = None;
                }
                r.c = None;
            }
            Mn::Tsx => r.x = None,
            Mn::Txs => {}
            Mn::Jmp if ins.mode == Mode::Abs => {
                todo.push((ins.arg, r, depth));
                continue;
            }
            Mn::Jsr => {
                if depth < CALLS {
                    todo.push((ins.arg, r, depth + 1));
                }
                continue;
            }
            Mn::Bcc | Mn::Bcs | Mn::Beq | Mn::Bne | Mn::Bmi | Mn::Bpl | Mn::Bvc | Mn::Bvs => {
                let mut taken = r;
                // A carry branch says what the carry is on each way.
                match ins.mn {
                    Mn::Bcc => {
                        taken.c = Some(false);
                        r.c = Some(true);
                    }
                    Mn::Bcs => {
                        taken.c = Some(true);
                        r.c = Some(false);
                    }
                    _ => {}
                }
                todo.push((ins.arg, taken, depth));
            }
            Mn::Rts | Mn::Rti | Mn::Brk | Mn::Jmp => continue,
            // Anything else writes A from something not the byte.
            _ => {
                r.a = None;
                r.c = None;
            }
        }
        todo.push((next, r, depth));
    }
}

/// Values like `1..=4`, `1..5` or `1,2,4`.
fn parse_values(s: &str) -> Result<BTreeSet<u8>> {
    if let Some((a, b)) = s.split_once("..") {
        let a: u8 = a.trim().parse()?;
        let last: u8 = match b.strip_prefix('=') {
            Some(b) => b.trim().parse()?,
            None => b
                .trim()
                .parse::<u8>()?
                .checked_sub(1)
                .context("an empty range")?,
        };
        return Ok((a..=last).collect());
    }
    s.split(',')
        .map(|v| v.trim().parse::<u8>().map_err(Into::into))
        .collect()
}

/// One table found: rows of `stride` bytes, the row value v reads at
/// `zero + stride * v`, the columns of a row the code reads, and who reads
/// it.
struct Found {
    zero: i32,
    stride: i32,
    /// The least index offset a read uses (`2*$53 - 4` gives -4): a value
    /// that makes the index negative isn't read.
    off: i32,
    cols: BTreeSet<i32>,
    readers: BTreeSet<Va>,
    /// The values whose row is inside the table, before the next table's
    /// first row.
    inside: Vec<u8>,
    /// Rows of two bytes, both read, every row inside a ROM address.
    pointers: bool,
}

impl Found {
    fn row(&self, v: u8) -> u16 {
        (self.zero + self.stride * i32::from(v)) as u16
    }

    /// Whether the code can read value v's row: its index is 0 to 255.
    fn indexes(&self, v: u8) -> bool {
        (0..=255).contains(&(self.stride * i32::from(v) + self.off))
    }
}

fn tables(a: &TablesArgs) -> Result<Outcome> {
    use std::fmt::Write as _;
    let rom = fs::read(&a.rom).with_context(|| format!("reading {}", a.rom.display()))?;
    if crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade {
        let set = gp_arcade::Romset::from_zip(&rom)?;
        let Some(input) = &a.input else {
            bail!("an arcade romset's tables are found from a run: give --input LOG");
        };
        let text =
            fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;
        let log = gp_evidence::InputLog::parse(&text)?;
        let index =
            u16::from_str_radix(a.index.trim_start_matches('$').trim_start_matches("0x"), 16)
                .with_context(|| format!("--index {} isn't an address", a.index))?;
        let (text, doc) = crate::code_z80::tables(&set, &log.frames, index)?;
        return Ok(Outcome::ok(doc, text));
    }
    let idx = crate::reach::parse_addr(&a.index)?;
    let byte = format!("${idx:02X}");
    let map = CodeMap::of(&rom, &[])?;
    // Every load of the byte, then the constants stored in it.
    let mut reads = BTreeSet::new();
    let mut cmps = BTreeSet::new();
    let mut stored = BTreeSet::new();
    for i in 0..map.routines.len() {
        let body = map.body(i);
        let mut last_imm: Option<u8> = None;
        for (p, ins) in &body {
            let Some(ins) = ins else { continue };
            let names_idx = matches!(ins.mode, Mode::Zp | Mode::Abs) && ins.arg as u16 == idx;
            if names_idx && matches!(ins.mn, Mn::Lda | Mn::Ldx | Mn::Ldy) {
                follow(&map, idx, *p, &mut reads, &mut cmps);
            }
            if names_idx
                && matches!(ins.mn, Mn::Sta | Mn::Stx | Mn::Sty)
                && let Some(k) = last_imm
            {
                stored.insert(k);
            }
            last_imm = match (ins.mn, ins.mode) {
                (Mn::Lda | Mn::Ldx | Mn::Ldy, Mode::Imm) => Some(ins.arg as u8),
                (Mn::Sta | Mn::Stx | Mn::Sty, _) => last_imm,
                _ => None,
            };
        }
    }
    let (values, values_from) = match &a.values {
        Some(s) => (parse_values(s)?, "--values".to_owned()),
        None => {
            let all: BTreeSet<u8> = cmps.iter().chain(stored.iter()).copied().collect();
            match (all.first(), all.last()) {
                (Some(&lo), Some(&hi)) => (
                    (lo..=hi).collect(),
                    format!(
                        "the code: compared with {}; stored {}",
                        list_hex(&cmps),
                        list_hex(&stored)
                    ),
                ),
                _ => bail!(
                    "the code neither compares {byte} with a constant nor stores one in it; give --values"
                ),
            }
        }
    };
    let Some(&vmin) = values.first() else {
        bail!("no values");
    };
    // Reads of one stride whose rows start within a stride of each other
    // are columns of one table.
    let mut by: BTreeMap<(i32, i32), (i32, BTreeSet<Va>)> = BTreeMap::new();
    for r in &reads {
        let e = by
            .entry((r.lin.m, i32::from(r.base) + r.lin.b))
            .or_insert((r.lin.b, BTreeSet::new()));
        e.0 = e.0.min(r.lin.b);
        e.1.insert(r.pc);
    }
    let mut found: Vec<Found> = Vec::new();
    for ((stride, at), (off, readers)) in by {
        match found.last_mut() {
            Some(f) if f.stride == stride && at - f.zero < stride => {
                f.cols.insert(at - f.zero);
                f.readers.extend(readers);
            }
            _ => found.push(Found {
                zero: at,
                stride,
                off,
                cols: [0].into(),
                readers,
                inside: Vec::new(),
                pointers: false,
            }),
        }
    }
    // A table starts at the row of the least value whose index the code
    // can make, and ends where the next one's first row starts.
    let first_of = |f: &Found| {
        let v = values
            .iter()
            .copied()
            .find(|&v| f.indexes(v))
            .unwrap_or(vmin);
        f.row(v)
    };
    found.sort_by_key(|f| (first_of(f), f.stride));
    let starts: Vec<u16> = found.iter().map(first_of).collect();
    let word = |cpu: u16| {
        u16::from_le_bytes([
            map.board.read(u32::from(cpu)),
            map.board.read(u32::from(cpu.wrapping_add(1))),
        ])
    };
    for (f, &first) in found.iter_mut().zip(&starts) {
        let next = starts.iter().copied().find(|&s| s > first);
        f.inside = values
            .iter()
            .copied()
            .filter(|&v| f.indexes(v) && next.is_none_or(|n| f.row(v) < n))
            .collect();
        f.pointers = f.stride == 2
            && f.cols.contains(&0)
            && f.cols.contains(&1)
            && !f.inside.is_empty()
            && f.inside.iter().all(|&v| word(f.row(v)) >= 0x8000);
    }
    // Where every pointer table's rows point, to bound each span by the
    // next one.
    let mut targets: BTreeSet<u16> = starts.iter().copied().collect();
    for f in found.iter().filter(|f| f.pointers) {
        targets.extend(f.inside.iter().map(|&v| word(f.row(v))));
    }
    let prg_off = |cpu: u16| map.board.prg_offset(u32::from(cpu));
    let mut text = format!(
        "{} tables read with {byte} as the index; values {} (from {values_from})\n",
        found.len(),
        list_dec(&values)
    );
    let mut assets = String::new();
    let mut rows = Vec::new();
    for (f, &first) in found.iter().zip(&starts) {
        let readers: Vec<String> = f
            .readers
            .iter()
            .map(|&p| {
                let r = map
                    .routine_of(p)
                    .map(|i| map.routines[i].name.clone())
                    .unwrap_or_else(|| "outside the code map".into());
                format!("{} {r}", va_hex(p))
            })
            .collect();
        let v0 = values
            .iter()
            .copied()
            .find(|&v| f.indexes(v))
            .unwrap_or(vmin);
        let kind = if f.pointers {
            "2-byte pointers".to_owned()
        } else {
            format!("{}-byte rows", f.stride)
        };
        let _ = writeln!(
            text,
            "\n${first:04X} (PRG {}), {kind}, the row for {byte} = v at ${first:04X} + {}*(v - {v0}){}; read at {}",
            prg_off(first).map_or("?".into(), |o| format!("0x{o:04X}")),
            f.stride,
            if f.stride > 1 && !f.pointers {
                format!(
                    ", columns read {}",
                    f.cols
                        .iter()
                        .map(i32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                String::new()
            },
            readers.join(", ")
        );
        let mut entries = Vec::new();
        for &v in &values {
            let at = f.row(v);
            let inside = f.inside.contains(&v);
            if !f.indexes(v) {
                let _ = writeln!(
                    text,
                    "  {byte} = {v}: not read (the index would be {})",
                    f.stride * i32::from(v) + f.off
                );
                continue;
            }
            let mut line = format!(
                "  {byte} = {v}: row {} at ${at:04X}",
                i32::from(v) - i32::from(v0)
            );
            let mut e = json!({"value": v, "at": format!("{at:04X}"), "inside": inside});
            if !inside {
                line.push_str(", past the table's end (the next table starts there)");
            } else if f.pointers {
                let t = word(at);
                let end = targets.range(t.wrapping_add(1)..).next().copied();
                let len = end
                    .map(|n| u32::from(n) - u32::from(t))
                    .filter(|&n| n <= 0x400);
                let _ = write!(
                    line,
                    " -> ${t:04X}{}",
                    len.map_or(String::new(), |n| format!(
                        ", {n} bytes to the next address listed"
                    ))
                );
                e["points_to"] = json!(format!("{t:04X}"));
                e["len"] = json!(len);
            }
            text.push_str(&line);
            text.push('\n');
            entries.push(e);
        }
        // The rows the values inside read, as one asset.
        let n = f.inside.len() as u32;
        if let Some(o) = prg_off(first)
            && n > 0
        {
            let name = format!("{}_{first:04x}", a.name);
            if f.pointers {
                let _ = write!(
                    assets,
                    "\n[[asset]]\nname = \"{name}\"\nformat = \"pointers\"\nregion = \"prg\"\noffset = 0x{o:04X}\ncount = {n}\ncpu_base = 0x{:04X}\n",
                    u32::from(first) - o as u32
                );
            } else if f.stride == 1 {
                let _ = write!(
                    assets,
                    "\n[[asset]]\nname = \"{name}\"\nformat = \"table\"\ncount = {n}\ncolumn = [\n  {{ name = \"byte\", region = \"prg\", offset = 0x{o:04X} }},\n]\n"
                );
            } else {
                let _ = write!(
                    assets,
                    "\n[[asset]]\nname = \"{name}\"\nformat = \"byte-rows\"\nregion = \"prg\"\noffset = 0x{o:04X}\ncount = {n}\nwidth = {}\n",
                    f.stride
                );
            }
        }
        rows.push(json!({
            "first": format!("{first:04X}"),
            "prg_offset": prg_off(first),
            "row_size": f.stride,
            "columns": f.cols,
            "pointers": f.pointers,
            "values_inside": f.inside,
            "readers": readers,
            "rows": entries,
        }));
    }
    let pointed: BTreeSet<String> = rows
        .iter()
        .flat_map(|r| r["rows"].as_array().cloned().unwrap_or_default())
        .filter_map(|e| e["points_to"].as_str().map(str::to_owned))
        .collect();
    let _ = writeln!(
        text,
        "\n{} tables ({} of pointers, {} distinct places pointed at)\n\nAsset map entries to name:\n{}",
        rows.len(),
        rows.iter().filter(|r| r["pointers"] == json!(true)).count(),
        pointed.len(),
        assets
    );
    Ok(Outcome::ok(
        json!({
            "index": byte,
            "values": values,
            "values_from": values_from,
            "tables": rows,
            "assets": assets,
        }),
        text.trim_end().to_owned(),
    ))
}

fn list_hex(s: &BTreeSet<u8>) -> String {
    if s.is_empty() {
        return "none".into();
    }
    s.iter()
        .map(|v| format!("${v:02X}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn list_dec(s: &BTreeSet<u8>) -> String {
    s.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up NROM-128 ROM. The reset code reads three tables by the
    /// byte $53 (a pointer table at $C100 by `2*$53 - 2`, bytes at $C200
    /// by `$53`, 4-byte rows at $C300 by `4*$53` through a call), compares
    /// $53 with 4 and stores 1 in it, then loops. The pointers point at
    /// $C400, $C410, $C430 and $C460. Every table is made up.
    pub(crate) fn rom() -> Vec<u8> {
        let mut prg = vec![0xEAu8; 0x4000];
        let code: &[u8] = &[
            0xA9, 0x01, 0x85, 0x53, // C000 lda #1; sta $53
            0xA5, 0x53, 0x38, 0xE9, 0x01, 0x0A, 0xAA, // C004 lda $53; sec; sbc #1; asl; tax
            0xBD, 0x00, 0xC1, 0x85, 0x00, // C00B lda $C100,x; sta $00
            0xBD, 0x01, 0xC1, 0x85, 0x01, // C010 lda $C101,x; sta $01
            0xA6, 0x53, 0xBD, 0x00, 0xC2, // C015 ldx $53; lda $C200,x
            0xA5, 0x53, 0xC9, 0x04, 0xF0, 0x03, // C01A lda $53; cmp #4; beq +3
            0x20, 0x30, 0xC0, // C020 jsr $C030
            0x4C, 0x04, 0xC0, // C023 jmp $C004
        ];
        prg[..code.len()].copy_from_slice(code);
        // C030: lda $53; asl; asl; tay; lda $C300,y; rts
        prg[0x30..0x39].copy_from_slice(&[0xA5, 0x53, 0x0A, 0x0A, 0xA8, 0xB9, 0x00, 0xC3, 0x60]);
        // C040: rti
        prg[0x40] = 0x40;
        for (i, t) in [0xC400u16, 0xC410, 0xC430, 0xC460].iter().enumerate() {
            prg[0x100 + 2 * i..0x102 + 2 * i].copy_from_slice(&t.to_le_bytes());
        }
        prg[0x3FFA..].copy_from_slice(&[0x40, 0xC0, 0x00, 0xC0, 0x40, 0xC0]);
        let mut rom = b"NES\x1A\x01\x01\x00\x00".to_vec();
        rom.resize(16, 0);
        rom.extend(prg);
        rom.extend(vec![0u8; 0x2000]);
        rom
    }

    #[test]
    fn windows_parse_as_reach_detail_numbers_frames() {
        assert_eq!(
            parse_window("10..20").unwrap(),
            Window {
                first: 10,
                last: 19
            }
        );
        assert_eq!(
            parse_window("10..=20").unwrap(),
            Window {
                first: 10,
                last: 20
            }
        );
        assert_eq!(
            parse_window("10..").unwrap(),
            Window {
                first: 10,
                last: u64::MAX
            }
        );
        assert_eq!(parse_window("..").unwrap(), Window::WHOLE);
        assert!(parse_window("20..10").is_err());
        assert!(parse_window("5").is_err());
    }

    #[test]
    fn only_picks_clips_by_id_or_prefix_and_refuses_a_miss() {
        let clip = |id: &str| ClipInput {
            set: String::new(),
            id: id.into(),
            power_on: Default::default(),
            inputs: Vec::new(),
        };
        let all = || vec![clip("m50.1"), clip("m50.2"), clip("m75.1")];
        let ids = |v: Vec<ClipInput>| v.into_iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(
            ids(select(all(), &["m50.*".into()]).unwrap()),
            ["m50.1", "m50.2"]
        );
        assert_eq!(ids(select(all(), &["m75.1".into()]).unwrap()), ["m75.1"]);
        assert_eq!(select(all(), &[]).unwrap().len(), 3);
        assert!(select(all(), &["m100*".into()]).is_err());
    }

    #[test]
    fn tables_read_by_the_byte_are_found_with_their_entry_size() {
        let rom = rom();
        let map = CodeMap::of(&rom, &[]).unwrap();
        let (mut reads, mut cmps) = (BTreeSet::new(), BTreeSet::new());
        for i in 0..map.routines.len() {
            for (p, ins) in map.body(i) {
                if let Some(ins) = ins
                    && matches!(ins.mn, Mn::Lda | Mn::Ldx | Mn::Ldy)
                    && ins.mode == Mode::Zp
                    && ins.arg == 0x53
                {
                    follow(&map, 0x53, p, &mut reads, &mut cmps);
                }
            }
        }
        let got: BTreeSet<(u16, i32, i32)> =
            reads.iter().map(|r| (r.base, r.lin.m, r.lin.b)).collect();
        let want: BTreeSet<(u16, i32, i32)> = [
            (0xC100, 2, -2),
            (0xC101, 2, -2),
            (0xC200, 1, 0),
            (0xC300, 4, 0),
        ]
        .into();
        assert_eq!(got, want);
        assert_eq!(cmps, [4u8].into());
    }
}
