//! `gp model static`: stage 0 of docs/plan-extraction-process.md, the
//! static harvest. Everything the ROM's code settles without running it
//! (`gp_nes::harvest`), from the ROM and the asset map only: elements and
//! their routines, dispatch, state machines, constants, model facts with
//! the instruction each comes from, mapping rows, and the open facts with
//! the routine and bytes that decide them.
//!
//! `--model DIR` also writes the facts as a model (model.toml) a later
//! stage starts from; `--grade` grades the harvest against the game's
//! answer key (crate::static_grade). Deterministic: the same ROM and asset
//! map give the same bytes.
//!
//! Brought from game-printer with its flags and outputs. A template
//! directory is a game's package folder now (its template.toml keeps the
//! old [source] accepts and [recipe] asset_map). No run recorder; an arcade
//! romset is refused in words. The helpers `gp explore` and `gp watch` read
//! the code map with (wave 3a) are these.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Args;
use serde_json::Value;
use toml::Value as Toml;

use crate::Outcome;
use gp_evidence::Paths;
use gp_nes::harvest::{self, Harvest, Input, Table};

#[derive(Args, Debug, Clone)]
pub struct StaticArgs {
    /// An iNES ROM, or a template directory (its ROM is found in
    /// $GP_ROM_DIR by the hashes it accepts, or given with --rom).
    pub target: PathBuf,
    /// The ROM, for a template.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// The asset map (default: the template's recipe.asset_map).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// Write static.json (the whole harvest) into this directory.
    #[arg(long)]
    pub write: Option<PathBuf>,
    /// Write the facts as a model directory: model.toml with the
    /// harvest's own names.
    #[arg(long)]
    pub model: Option<PathBuf>,
    /// With --model: write the facts in the real model's names, by an
    /// answer key (the bench's start; the harvest itself never reads it).
    #[arg(long, requires = "model")]
    pub names: Option<PathBuf>,
    /// Grade against a game's answer key (games/GAME/static-key.toml).
    #[arg(long)]
    pub grade: Option<PathBuf>,
    /// Report where the reader stopped: each routine the code map has that
    /// no path read, traced to the instruction where the reader didn't go
    /// on (a bank not known, a pointer not known, a site not reached),
    /// ranked by how many routines are behind it. With --write, also
    /// unread.json.
    #[arg(long)]
    pub unread: bool,
    /// Clip sets (clips.toml) to run before reading, after stage 0's own
    /// starts: each clip runs from power-on in gp-nes, and the code map
    /// starts from every routine the runs entered and every jump through a
    /// pointer or a pushed address they took, as well as from the vectors
    /// (stage 0, "the ROM, run or read"; `gp reach` runs the same clips).
    #[arg(long = "clips", value_name = "CLIPS")]
    pub clips: Vec<PathBuf>,
    #[command(flatten)]
    pub starts: crate::reach::Starts,
    /// Read the ROM without running it: no starts and no clips.
    #[arg(long, conflicts_with_all = ["clips", "runs", "no_starts"])]
    pub no_run: bool,
    /// The store of runs reads and fills (default $GP_HOME/work/events).
    #[arg(long)]
    pub store: Option<PathBuf>,
}

/// The ROM and asset map a target names.
pub(crate) fn resolve(a: &StaticArgs, paths: &Paths) -> Result<(PathBuf, Option<PathBuf>)> {
    if a.target.is_dir() {
        let t: Toml = toml::from_str(&fs::read_to_string(a.target.join("template.toml"))?)?;
        let map = a.assets.clone().or_else(|| {
            t.get("recipe")?
                .get("asset_map")?
                .as_str()
                .map(|m| a.target.join(m))
        });
        if let Some(r) = &a.rom {
            return Ok((r.clone(), map));
        }
        return Ok((template_rom(&a.target, &t, paths)?, map));
    }
    Ok((a.target.clone(), a.assets.clone()))
}

/// The ROM in $GP_ROM_DIR a template accepts, by the hashes its
/// template.toml (`t`) lists.
pub(crate) fn template_rom(dir: &Path, t: &Toml, paths: &Paths) -> Result<PathBuf> {
    let accepts: Vec<String> = t
        .get("source")
        .and_then(|s| s.get("accepts"))
        .and_then(|v| v.as_array())
        .map(|v| {
            v.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let mut files = Vec::new();
    if paths.roms.is_dir() {
        walk(&paths.roms, &mut files)?;
    }
    files.sort();
    for p in files {
        let Ok(b) = fs::read(&p) else { continue };
        if let Ok(img) = gp_evidence::source::identify_nes(&b)
            && accepts.contains(&img.sha256)
        {
            return Ok(p);
        }
        // An arcade package accepts a romset by its identity.
        if gp_arcade::romset::looks_like_zip(&b)
            && let Ok(set) = gp_arcade::Romset::from_zip(&b)
            && accepts.contains(&set.identity)
        {
            return Ok(p);
        }
    }
    bail!(
        "no ROM in {} that {} accepts; give it with --rom",
        paths.roms.display(),
        dir.display()
    );
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// The asset map's PRG tables, the level's width and the metatiles' names
/// and kinds.
pub(crate) fn input_of(map_text: Option<&str>, prg: &[u8]) -> Result<Input> {
    let mut inp = Input {
        prg_len: prg.len(),
        ..Input::default()
    };
    let Some(text) = map_text else { return Ok(inp) };
    let m: Toml = toml::from_str(text).context("reading the asset map")?;
    let int = |v: Option<&Toml>| v.and_then(Toml::as_integer).map(|x| x as usize);
    for a in m
        .get("asset")
        .and_then(Toml::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let name = a
            .get("name")
            .and_then(Toml::as_str)
            .unwrap_or("")
            .to_owned();
        let format = a
            .get("format")
            .and_then(Toml::as_str)
            .unwrap_or("")
            .to_owned();
        let prg_region = |v: &Toml| v.get("region").and_then(Toml::as_str) == Some("prg");
        let count = int(a.get("count")).unwrap_or(0);
        if prg_region(&a)
            && let Some(off) = int(a.get("offset"))
        {
            let len = match format.as_str() {
                "palette" => count * 4,
                "metatiles" => count * 4,
                "level-grid" => {
                    int(a.get("width")).unwrap_or(0) * int(a.get("height")).unwrap_or(0)
                }
                _ => count,
            };
            inp.tables.push(Table {
                asset: name.clone(),
                column: name.clone(),
                asset_kind: format.clone(),
                offset: off,
                len,
            });
        }
        if format == "level-grid" {
            inp.level_width_px = (int(a.get("width")).unwrap_or(0) * 16) as u32;
        }
        if format == "metatiles" {
            inp.metatile_names = a
                .get("names")
                .and_then(Toml::as_array)
                .map(|v| {
                    v.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
        }
        for p in a
            .get("property")
            .and_then(Toml::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let (Some(pn), Some(off)) =
                (p.get("name").and_then(Toml::as_str), int(p.get("offset")))
            else {
                continue;
            };
            inp.tables.push(Table {
                asset: name.clone(),
                column: pn.to_owned(),
                asset_kind: format.clone(),
                offset: off,
                len: count,
            });
            if pn == "solid" {
                inp.metatile_solid = prg
                    .get(off..off + count)
                    .map(<[u8]>::to_vec)
                    .unwrap_or_default();
            }
            if pn == "kind" {
                inp.metatile_kind = prg
                    .get(off..off + count)
                    .map(<[u8]>::to_vec)
                    .unwrap_or_default();
            }
        }
        for c in a
            .get("column")
            .and_then(Toml::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let Some(cn) = c.get("name").and_then(Toml::as_str) else {
                continue;
            };
            for key in ["offset", "hi_offset"] {
                if let Some(off) = int(c.get(key)) {
                    inp.tables.push(Table {
                        asset: name.clone(),
                        column: cn.to_owned(),
                        asset_kind: format.clone(),
                        offset: off,
                        len: count,
                    });
                }
            }
        }
    }
    inp.tables
        .sort_by(|a, b| (a.offset, &a.column).cmp(&(b.offset, &b.column)));
    Ok(inp)
}

/// The harvest of a ROM with its asset map; with `runs`, the code map
/// also starts from what runs of the ROM did.
pub(crate) fn harvest_seeded(
    rom: &Path,
    map: Option<&Path>,
    unread: bool,
    runs: Option<&gp_events::Levels>,
) -> Result<(Harvest, Option<harvest::unread::Unread>)> {
    let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    let (prg, mapper) = harvest::ines_prg(&bytes).context("not an iNES file")?;
    let map_text = match map {
        Some(p) => Some(fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?),
        None => None,
    };
    let mut inp = input_of(map_text.as_deref(), prg)?;
    let mut board = gp_nes::codemap::Board::of(prg, mapper);
    let harvest_once = |board: &gp_nes::codemap::Board, inp: &Input| {
        if unread {
            let (h, u) = harvest::harvest_board_unread(board, inp);
            (h, Some(u))
        } else {
            (harvest::harvest_board(board, inp), None)
        }
    };
    let (mut h, u) = match runs {
        None => harvest_once(&board, &inp),
        Some(l) => {
            // A region the runs ran in that the board's usual layout fixes
            // is switched (MMC1 in 32 KB mode).
            board.switched =
                gp_events::names::Namer::observed_switched(&board, &l.footprint.windows);
            let namer = gp_events::names::Namer::of_board(board.clone());
            for e in &l.edges {
                // An entry, or a jump or call from code in RAM into ROM. A
                // return or a branch back from RAM lands inside a routine,
                // not at one's start.
                let from_ram = e.from == gp_events::NO_ROM
                    && matches!(
                        e.kind,
                        gp_events::EdgeKind::Jsr
                            | gp_events::EdgeKind::Jmp
                            | gp_events::EdgeKind::JmpIndirect
                            | gp_events::EdgeKind::RtsJump
                    );
                if !(e.kind.enters() && e.from != gp_events::NO_ROM) && !from_ram {
                    continue;
                }
                let Some(to) = namer.va(e.to, e.stat.to_cpu) else {
                    continue;
                };
                inp.seed_starts.push(to);
                if matches!(
                    e.kind,
                    gp_events::EdgeKind::JmpIndirect | gp_events::EdgeKind::RtsJump
                ) && let Some(from) = namer.va(e.from, e.stat.from_cpu)
                {
                    inp.seed_jumps.push((from, to));
                }
            }
            // Executed code no routine owns (a loop's head before its
            // routine's start, code a walk didn't follow on to) starts a
            // routine at its first instruction, and the harvest runs again.
            let mut out = harvest_once(&board, &inp);
            for _ in 0..4 {
                let mut starts = uncovered_starts(&board, &out.0, l);
                // A routine of the map whose first instruction a run ran was
                // entered, by a call or not (a `jmp`, a fall-through): the
                // reader starts there too (it skips what a path read).
                for rt in &out.0.routines {
                    let Some(v) = gp_nes::codemap::va_parse(&rt.addr) else {
                        continue;
                    };
                    if let Some(o) = board.prg_offset(v)
                        && l.footprint.starts.contains(o)
                        && !inp.seed_starts.contains(&v)
                    {
                        starts.push((o as u32, gp_nes::codemap::va_cpu(v)));
                    }
                }
                let new: Vec<u32> = starts
                    .into_iter()
                    .filter_map(|(o, cpu)| namer.va(o, cpu))
                    .filter(|v| !inp.seed_starts.contains(v))
                    .collect();
                if new.is_empty() {
                    break;
                }
                inp.seed_starts.extend(new);
                out = harvest_once(&board, &inp);
            }
            out
        }
    };
    // UxROM, MMC1 and MMC3 banks are followed; another board's switched
    // code isn't read.
    if gp_nes::codemap::banked(prg) && !matches!(mapper, 1 | 2 | 4) {
        h.open.insert(
            0,
            harvest::Open {
                what: "code in switched banks".into(),
                routine: String::new(),
                pc: String::new(),
                bytes: Vec::new(),
                why: format!(
                    "mapper {mapper}, {} KB of PRG: only the first and last 16 KB are read",
                    prg.len() / 1024
                ),
            },
        );
    }
    Ok((h, u))
}

/// The facts as a model.toml: each path set to its value, with the
/// harvest's names, and the evidence of each.
pub(crate) fn model_text(facts: &[(String, Value)], name: &str) -> Result<String> {
    let mut doc = toml::Table::new();
    let mut head = toml::Table::new();
    head.insert("name".into(), Toml::String(name.into()));
    head.insert("source".into(), Toml::String("gp model static".into()));
    doc.insert("model".into(), Toml::Table(head));
    for (p, v) in facts {
        set_path(&mut doc, p, to_toml(v));
    }
    Ok(toml::to_string(&doc)?)
}

fn to_toml(v: &Value) -> Toml {
    match v {
        Value::Bool(b) => Toml::Boolean(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Toml::Integer(i),
            None => {
                let x = n.as_f64().unwrap_or(0.0);
                if x.fract() == 0.0 {
                    Toml::Integer(x as i64)
                } else {
                    Toml::String(format!("{x}"))
                }
            }
        },
        Value::String(s) => Toml::String(s.clone()),
        Value::Array(a) => Toml::Array(a.iter().map(to_toml).collect()),
        Value::Object(o) => Toml::Table(o.iter().map(|(k, v)| (k.clone(), to_toml(v))).collect()),
        Value::Null => Toml::String(String::new()),
    }
}

/// Sets `a.b[i].c` (a list element by position) or `a.b[key].c` (one the
/// harvest names, kept as a table key `key`).
fn set_path(doc: &mut toml::Table, path: &str, v: Toml) {
    enum Seg {
        Key(String),
        Index(usize),
    }
    let mut segs = Vec::new();
    for part in path.split('.') {
        let (name, rest) = part
            .split_once('[')
            .map_or((part, None), |(n, r)| (n, Some(r)));
        segs.push(Seg::Key(name.to_owned()));
        if let Some(r) = rest {
            let inner = r.trim_end_matches(']');
            match inner.parse::<usize>() {
                Ok(i) => segs.push(Seg::Index(i)),
                Err(_) => segs.push(Seg::Key(inner.to_owned())),
            }
        }
    }
    fn put(cur: &mut Toml, segs: &[Seg], v: Toml) {
        let Some((first, rest)) = segs.split_first() else {
            *cur = v;
            return;
        };
        match first {
            Seg::Key(k) => {
                if !cur.is_table() {
                    *cur = Toml::Table(toml::Table::new());
                }
                if let Toml::Table(t) = cur {
                    let next = t
                        .entry(k.clone())
                        .or_insert(Toml::Table(toml::Table::new()));
                    put(next, rest, v);
                }
            }
            Seg::Index(i) => {
                if !cur.is_array() {
                    *cur = Toml::Array(Vec::new());
                }
                if let Toml::Array(a) = cur {
                    while a.len() <= *i {
                        a.push(Toml::Table(toml::Table::new()));
                    }
                    put(&mut a[*i], rest, v);
                }
            }
        }
    }
    let mut root = Toml::Table(std::mem::take(doc));
    put(&mut root, &segs, v);
    if let Toml::Table(t) = root {
        *doc = t;
    }
}

pub(crate) fn run(a: &StaticArgs, paths: &Paths) -> Result<Outcome> {
    let t0 = Instant::now();
    let (rom, map) = resolve(a, paths)?;
    let bytes = fs::read(&rom).with_context(|| format!("reading {}", rom.display()))?;
    // A romset: the Z80 harvest (crate::model_static_z80).
    if crate::original::Platform::of_file(&bytes) == crate::original::Platform::Arcade {
        return crate::model_static_z80::run(a, &rom, &bytes, t0);
    }
    drop(bytes);
    let runs = if a.no_run {
        None
    } else {
        let bytes = fs::read(&rom).with_context(|| format!("reading {}", rom.display()))?;
        let clips = a.starts.with(&a.clips, paths)?;
        let store = gp_events::store::Store::at(
            a.store
                .clone()
                .unwrap_or_else(|| paths.home.join("work").join("events")),
        );
        let keep = crate::reach::Keep {
            store: Some(&store),
            ..Default::default()
        };
        Some((
            crate::reach::run_clips(&bytes, &clips, &keep)?,
            bytes,
            clips,
        ))
    };
    let (mut h, unread) = harvest_seeded(
        &rom,
        map.as_deref(),
        a.unread,
        runs.as_ref().map(|r| &r.0.levels),
    )?;
    // What the runs did that no rule names yet, as open facts: the behaviour
    // detectors' finds, each with its bytes or routine.
    if let Some((_, bytes, clips)) = &runs {
        let b = crate::behave::build(bytes, clips)?;
        h.open.extend(crate::behave::open_facts(&b));
        h.open.sort_by(|a, b| a.what.cmp(&b.what));
    }
    let runs = runs.map(|r| r.0);
    let secs = t0.elapsed().as_secs_f64();
    let doc = serde_json::to_value(&h)?;
    let text = serde_json::to_string_pretty(&doc)? + "\n";
    if let Some(d) = &a.write {
        fs::create_dir_all(d)?;
        fs::write(d.join("static.json"), &text)?;
        if let Some(u) = &unread {
            fs::write(
                d.join("unread.json"),
                serde_json::to_string_pretty(u)? + "\n",
            )?;
        }
    }
    if let Some(d) = &a.model {
        fs::create_dir_all(d)?;
        let facts: Vec<(String, Value)> = match &a.names {
            Some(k) => crate::static_grade::in_model_names(&h, k)?
                .into_iter()
                .map(|(p, v, _)| (p, v))
                .collect(),
            None => h
                .facts
                .iter()
                .map(|f| (f.path.clone(), f.value.clone()))
                .collect(),
        };
        fs::write(d.join(gp_model::MODEL_FILE), model_text(&facts, "static")?)?;
    }
    let grade = match &a.grade {
        Some(k) => Some(crate::static_grade::grade(&h, k)?),
        None => None,
    };
    let mut out = doc;
    if let Some(g) = &grade {
        out["grade"] = g.clone();
    }
    let mut text = summary(&h, &rom, secs, grade.as_ref());
    if let Some(r) = &runs {
        let v = runs_against_map(&rom, &h, r)?;
        text += &format!(
            "\nRuns: {} clips, {} frames; {} PRG bytes executed, {} of them outside the code map ({} instruction starts); {} entries ({} jumps through a pointer or a pushed address)\n",
            v["clips"],
            v["frames"],
            v["executed_bytes"],
            v["outside_code_map"],
            v["starts_outside_code_map"],
            v["entries"],
            v["seed_jumps"]
        );
        out["runs"] = v;
    }
    if let Some(u) = &unread {
        out["unread"] = serde_json::to_value(u)?;
        text += &unread_text(u);
    }
    Ok(Outcome::ok(out, text))
}

/// Where the reader stopped, as text: counts, then the stops that hide the
/// most routines.
fn unread_text(u: &harvest::unread::Unread) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "\nUnread: {} of {} routines read, {} not; the code map covers {} of {} PRG bytes",
        u.read, u.routines, u.unread, u.code_bytes, u.prg_bytes
    );
    for (reason, (stops, behind)) in &u.by_reason {
        let _ = writeln!(
            s,
            "  {reason}: {stops} stops, {behind} routines behind them"
        );
    }
    for st in u.stops.iter().take(20) {
        let bytes = if st.bytes.is_empty() {
            String::new()
        } else {
            format!(" [{}]", st.bytes.join(" "))
        };
        let target = if st.target.is_empty() {
            String::new()
        } else {
            format!(" -> {}", st.target)
        };
        let _ = writeln!(
            s,
            "  {:>4} behind  {} in {}{target}{bytes}",
            st.behind, st.id, st.routine
        );
    }
    s
}

/// The harvest as text.
pub(crate) fn summary(h: &Harvest, rom: &Path, secs: f64, grade: Option<&Value>) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "gp model static {}: {} routines, {} elements, {} dispatch sites, {} state machines, {} facts, {} open, {} constants ({:.2} s)",
        rom.display(),
        h.routines.len(),
        h.elements.len(),
        h.dispatch.len(),
        h.machines.len(),
        h.facts.len(),
        h.open.len(),
        h.constants.len(),
        secs
    );
    let _ = writeln!(s, "\nElements:");
    for e in &h.elements {
        let _ = writeln!(s, "  {} ({}): {}", e.name, e.kind, e.routines.join(", "));
    }
    let _ = writeln!(s, "\nDispatch:");
    for d in &h.dispatch {
        let _ = writeln!(s, "  {} at {} on {}", d.kind, d.at, d.subject);
        for (k, v) in &d.cases {
            let _ = writeln!(s, "    {k}: {}", v.join(", "));
        }
    }
    let _ = writeln!(s, "\nState machines:");
    for m in &h.machines {
        let st: Vec<String> = m.states.iter().map(|x| x.name.clone()).collect();
        let _ = writeln!(s, "  {} ({}): {}", m.name, m.byte, st.join(" "));
        for t in &m.transitions {
            let after = t.after.map(|a| format!(" after {a}")).unwrap_or_default();
            let guards = if t.guards.is_empty() {
                String::new()
            } else {
                format!(" if {}", t.guards.join(", "))
            };
            let _ = writeln!(
                s,
                "    {} -> {}{} in {} at {}{}",
                t.from, t.to, after, t.routine, t.at, guards
            );
        }
    }
    let _ = writeln!(s, "\nFacts:");
    for f in &h.facts {
        let ev: Vec<String> = f
            .evidence
            .iter()
            .map(|e| format!("{} {}", e.pc, e.insn))
            .collect();
        let _ = writeln!(s, "  {} = {} ({})", f.path, f.value, ev.join("; "));
    }
    let _ = writeln!(s, "\nOpen:");
    for o in &h.open {
        let _ = writeln!(
            s,
            "  {}: {} {} [{}] {}",
            o.what,
            o.routine,
            o.pc,
            o.bytes.join(" "),
            o.why
        );
    }
    if let Some(g) = grade {
        let _ = writeln!(s, "\n{}", crate::static_grade::table(g));
    }
    s
}

/// What the runs did against the code map: executed PRG bytes, and those
/// outside every routine the harvest's code map has.
fn runs_against_map(rom: &Path, h: &Harvest, r: &crate::reach::Reached) -> Result<Value> {
    let bytes = fs::read(rom)?;
    let (prg, mapper) = harvest::ines_prg(&bytes).context("not an iNES file")?;
    let mut board = gp_nes::codemap::Board::of(prg, mapper);
    board.switched =
        gp_events::names::Namer::observed_switched(&board, &r.levels.footprint.windows);
    let map = routine_bytes(&board, h);
    let f = &r.levels.footprint;
    let executed = f.executed.count();
    let outside = (0..prg.len())
        .filter(|&i| f.executed.contains(i) && !map[i])
        .count();
    // A brk's padding byte is fetched, so executed, but starts nothing.
    let starts_outside = (0..prg.len())
        .filter(|&i| f.starts.contains(i) && !map[i])
        .count();
    let jumps = r
        .levels
        .edges
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                gp_events::EdgeKind::JmpIndirect | gp_events::EdgeKind::RtsJump
            )
        })
        .count();
    Ok(serde_json::json!({
        "clips": r.clips.len(),
        "frames": r.clips.iter().map(|c| c.2).sum::<u64>(),
        "executed_bytes": executed,
        "outside_code_map": outside,
        "starts_outside_code_map": starts_outside,
        "code_map_bytes": map.iter().filter(|&&b| b).count(),
        "entries": r.levels.entries().len(),
        "seed_jumps": jumps,
    }))
}

/// Which PRG bytes the harvest's routines span.
fn routine_bytes(board: &gp_nes::codemap::Board, h: &Harvest) -> Vec<bool> {
    let mut map = vec![false; board.prg.len()];
    for rt in &h.routines {
        let Some(v) = gp_nes::codemap::va_parse(&rt.addr) else {
            continue;
        };
        let Some(o) = board.prg_offset(v) else {
            continue;
        };
        for b in map.iter_mut().skip(o).take(rt.size as usize) {
            *b = true;
        }
    }
    map
}

/// The first instruction start of each run of executed PRG bytes no
/// routine spans, with the CPU address it ran at (the lowest, from L0's
/// windows).
fn uncovered_starts(
    board: &gp_nes::codemap::Board,
    h: &Harvest,
    l: &gp_events::Levels,
) -> Vec<(u32, u16)> {
    let map = routine_bytes(board, h);
    let f = &l.footprint;
    let mut out = Vec::new();
    let mut inside = false;
    for (o, &mapped) in map.iter().enumerate() {
        let open = f.executed.contains(o) && !mapped;
        if open && !inside && f.starts.contains(o) {
            let cpus = f.windows.get(o / 0x400).copied().unwrap_or(0);
            if let Some(cp) = (32..64u32).find(|cp| cpus & (1u64 << cp) != 0) {
                let cpu = (cp * 0x400) as u16 | (o % 0x400) as u16;
                out.push((o as u32, cpu));
            }
        }
        inside = open && (inside || f.starts.contains(o));
    }
    out
}
