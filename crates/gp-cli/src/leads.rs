//! `gp mechanic leads`: new mechanics to look at, from what the clips of a
//! game's mechanic sessions have not shown yet. Every clip is replayed with
//! the event collector; the routines it entered and the RAM bytes it wrote
//! are set against the ROM dataset's routines (those stage 0's runs
//! reached) and RAM bytes (those its runs wrote). What the runs reached and
//! no clip did is grouped by the routine the clips do run that calls into
//! it: a branch some play the clips haven't made. Each queue row's open
//! items are leads too. Nothing here knows one game.
//!
//! On an arcade romset the clips replay on gp-arcade ([`replay_arcade`])
//! against the arcade dataset (crate::arcade_stage0): routines by address,
//! RAM in the work RAM from E000.
//!
//! Ported from game-printer's leads.rs. A clip saved from a path (the
//! snapshot form) is replayed from power-on through its path
//! (crate::mechanic::paths::full_log), as one saved the old way is.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::rom_dataset::Cell;

/// What the clips covered: PRG offsets of routines entered, RAM bytes written.
#[derive(Default)]
struct Covered {
    routines: BTreeSet<u32>,
    ram: BTreeSet<u16>,
    clips: usize,
    frames: usize,
}

fn replay(rom: &[u8], log: &str, cov: &mut Covered, known: &BTreeSet<u16>) -> Result<()> {
    let parsed =
        gp_evidence::input_log::InputLog::parse(log).map_err(|e| anyhow::anyhow!("{e}"))?;
    if crate::original::Platform::of_file(rom) == crate::original::Platform::Arcade {
        return replay_arcade(rom, &parsed, cov, known);
    }
    let mut nes = gp_nes::Nes::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
    let layout = gp_events::Layout::of(nes.bus())?;
    let act =
        gp_events::activity::Activity::watching(layout, &gp_events::Bits::new(0), BTreeSet::new());
    let mut col = gp_events::Collector::with_activity(layout, false, act);
    col.start(nes.bus_mut_for_observers());
    for f in &parsed.frames {
        if f.reset {
            col.will_reset();
        }
        nes.step_with(*f, |bus| col.before(bus))?;
        let fa = col.end_frame_activity(nes.bus()).context("watching")?;
        cov.routines.extend(fa.entered.iter().map(|(o, _)| *o));
        cov.ram
            .extend(fa.ram.iter().filter(|w| w.addr < 0x800).map(|w| w.addr));
    }
    cov.clips += 1;
    cov.frames += parsed.frames.len();
    Ok(())
}

/// [`replay`] on gp-arcade: the routine entries the main Z80 ran
/// ([`crate::arcade_stage0::entries`], by address, where the NES counts PRG offsets), and
/// the work RAM bytes it wrote, moved to the 2 KB window from E000 as the
/// NES's RAM is counted from 0.
fn replay_arcade(
    rom: &[u8],
    log: &gp_evidence::InputLog,
    cov: &mut Covered,
    known: &BTreeSet<u16>,
) -> Result<()> {
    use crate::arcade_stage0::{RAM_BASE, RAM_END};
    let set = gp_arcade::Romset::from_zip(rom)?;
    let mut t = gp_arcade::Tapper::power_on(&set);
    for (i, f) in log.frames.iter().enumerate() {
        let bits = f.buttons_word();
        anyhow::ensure!(
            bits <= 0xff && !f.reset,
            "frame {}: an arcade input holds bits 0 to 7 and no reset",
            i + 1
        );
        t.set_trace(true);
        t.set_accesses(true);
        t.step(gp_arcade::Controls::from_bits(bits));
        let trace = t.take_trace();
        let accesses = t.take_accesses();
        t.set_trace(false);
        t.set_accesses(false);
        let mut trace = trace;
        trace.sort_by_key(|e| e.time);
        cov.routines.extend(
            crate::arcade_stage0::entries(&trace, known)
                .into_iter()
                .map(u32::from),
        );
        cov.ram.extend(
            accesses
                .iter()
                .filter(|a| {
                    a.cpu == 0
                        && a.kind == gp_arcade::mcr::AccessKind::Write
                        && (RAM_BASE..RAM_END).contains(&a.addr)
                })
                .map(|a| a.addr - RAM_BASE),
        );
    }
    cov.clips += 1;
    cov.frames += log.frames.len();
    Ok(())
}

fn s(r: &crate::rom_dataset::Row, k: &str) -> String {
    match r.get(k) {
        Some(Cell::S(Some(v))) => v.clone(),
        _ => String::new(),
    }
}

fn b(r: &crate::rom_dataset::Row, k: &str) -> bool {
    matches!(r.get(k), Some(Cell::B(true)))
}

fn i(r: &crate::rom_dataset::Row, k: &str) -> i64 {
    match r.get(k) {
        Some(Cell::I(v)) | Some(Cell::OI(Some(v))) => *v,
        _ => 0,
    }
}

/// The leads for a game, best first: each {lead, kind, weight, ...}.
pub(crate) fn leads(home: &Path, rom: &[u8], game: &str) -> Result<(Vec<Value>, Value)> {
    let root = home.join("mechanics").join(game);
    let mut cov = Covered::default();
    let data = crate::rom_dataset::read(&home.join("work/datasets").join(game))
        .with_context(|| format!("no ROM dataset for {game} under $GP_HOME/work/datasets"))?;
    let known = crate::arcade_stage0::dataset_routines(Some(&data));
    let mut dirs: Vec<_> = fs::read_dir(&root)
        .with_context(|| format!("no mechanics for {game}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("clips").is_dir())
        .collect();
    dirs.sort();
    for d in &dirs {
        let mut logs: Vec<_> = fs::read_dir(d.join("clips"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "log"))
            .collect();
        logs.sort();
        for l in logs {
            let n: Option<u64> = l
                .file_stem()
                .and_then(|x| x.to_str())
                .and_then(|x| x.parse().ok());
            let text = match n {
                Some(n) => crate::mechanic::paths::full_log(home, d, n)?.0,
                None => fs::read_to_string(&l)?,
            };
            replay(rom, &text, &mut cov, &known)?;
        }
    }

    // A routine's key in the clips' coverage: its PRG offset on the NES,
    // its address on the arcade board (the dataset's `addr`, unbanked).
    let arcade = crate::original::Platform::of_file(rom) == crate::original::Platform::Arcade;
    let board = if arcade {
        None
    } else {
        let (prg, mapper) = gp_nes::harvest::ines_prg(rom).context("not an iNES ROM")?;
        Some(gp_nes::codemap::Board::of(prg, mapper))
    };
    let base: u16 = if arcade {
        crate::arcade_stage0::RAM_BASE
    } else {
        0
    };

    // Routines: by name, with offset, size, element, ran (by stage 0),
    // callers, and whether a clip entered it.
    struct R {
        size: i64,
        element: String,
        stage0: bool,
        clip: bool,
        callers: Vec<String>,
    }
    let mut rs: BTreeMap<String, R> = BTreeMap::new();
    for r in data.rows("routines") {
        let name = s(r, "routine");
        let off = match &board {
            Some(board) => gp_nes::codemap::va_parse(&name).and_then(|v| board.prg_offset(v)),
            None => usize::try_from(i(r, "addr")).ok(),
        };
        let callers: Vec<String> = serde_json::from_str(&s(r, "callers")).unwrap_or_default();
        rs.insert(
            name,
            R {
                size: i(r, "size"),
                element: s(r, "element"),
                stage0: b(r, "ran"),
                clip: off.is_some_and(|o| cov.routines.contains(&(o as u32))),
                callers,
            },
        );
    }
    // RAM: bytes stage 0's runs wrote, with their writers and names.
    let mut ram_by_writer: BTreeMap<String, Vec<(u16, String)>> = BTreeMap::new();
    let mut ram_unseen = 0usize;
    let mut ram_written = 0usize;
    for r in data.rows("ram") {
        if !b(r, "written") {
            continue;
        }
        ram_written += 1;
        let a = i(r, "addr") as u16;
        let w = a.wrapping_sub(base);
        if w >= 0x800 || cov.ram.contains(&w) {
            continue;
        }
        ram_unseen += 1;
        let name = [s(r, "element"), s(r, "quantity"), s(r, "role")]
            .into_iter()
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let writers: Vec<String> = serde_json::from_str(&s(r, "writers")).unwrap_or_default();
        for w in writers {
            ram_by_writer.entry(w).or_default().push((a, name.clone()));
        }
    }

    // Group what stage 0 reached and no clip did by its gate: the nearest
    // caller a clip did run (walking up callers that no clip ran either).
    let gate_of = |start: &str| -> Option<String> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![start.to_owned()];
        while let Some(n) = todo.pop() {
            if !seen.insert(n.clone()) {
                continue;
            }
            for c in rs.get(&n).map(|r| r.callers.clone()).unwrap_or_default() {
                match rs.get(&c) {
                    Some(r) if r.clip => return Some(c),
                    Some(_) => todo.push(c),
                    None => {}
                }
            }
        }
        None
    };
    struct G {
        routines: Vec<String>,
        bytes: i64,
        ram: BTreeSet<(u16, String)>,
        elements: BTreeSet<String>,
        stage0: usize,
    }
    let mut groups: BTreeMap<String, G> = BTreeMap::new();
    for (name, r) in &rs {
        if r.clip {
            continue;
        }
        // With no caller a clip ran, the group is its element (or "none").
        let gate = gate_of(name).unwrap_or_else(|| {
            format!(
                "~{}",
                if r.element.is_empty() {
                    "unknown"
                } else {
                    &r.element
                }
            )
        });
        let g = groups.entry(gate).or_insert(G {
            routines: Vec::new(),
            bytes: 0,
            ram: BTreeSet::new(),
            elements: BTreeSet::new(),
            stage0: 0,
        });
        g.routines.push(name.clone());
        g.bytes += r.size;
        g.stage0 += usize::from(r.stage0);
        if !r.element.is_empty() {
            g.elements.insert(r.element.clone());
        }
        for x in ram_by_writer.get(name).into_iter().flatten() {
            g.ram.insert(x.clone());
        }
    }

    let mut out: Vec<Value> = Vec::new();
    for (gate, g) in &groups {
        // Weight: code bytes, plus 8 for each RAM byte it writes that no
        // clip wrote; code stage 0 reached counts double (a run got there,
        // so play can).
        let weight = g.bytes * if g.stage0 > 0 { 2 } else { 1 } + 8 * g.ram.len() as i64;
        let named: Vec<String> = g
            .ram
            .iter()
            .filter(|(_, n)| !n.is_empty())
            .map(|(a, n)| format!("${a:04X} {n}"))
            .take(4)
            .collect();
        let what = if g.elements.is_empty() {
            String::new()
        } else {
            format!(
                " of {}",
                g.elements.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        };
        let (from, show) = match gate.strip_prefix('~') {
            Some(e) => (
                format!("with no caller any clip ran (element: {e})"),
                "A clip reaching a part of the game no clip has (another level, a menu, a game over) would show it".to_owned(),
            ),
            None => (
                format!("branching off routine {gate}, which the clips do run"),
                format!("A clip that makes {gate} take its other path would show it"),
            ),
        };
        let lead = format!(
            "{} routine(s){what}, {} bytes, that no clip ran ({} reached by stage 0's runs), {from}; they write {} RAM byte(s) no clip wrote{}. {show}.",
            g.routines.len(),
            g.bytes,
            g.stage0,
            g.ram.len(),
            if named.is_empty() {
                String::new()
            } else {
                format!(" ({})", named.join(", "))
            }
        );
        out.push(json!({"kind": "code", "gate": gate, "weight": weight, "lead": lead,
                        "routines": g.routines, "ram": g.ram.iter().map(|(a, _)| format!("${a:04X}")).collect::<Vec<_>>()}));
    }
    // Elements no clip ran any routine of.
    for r in data.rows("elements") {
        let e = s(r, "element");
        if e.is_empty() {
            continue;
        }
        let mine: Vec<&R> = rs.values().filter(|x| x.element == e).collect();
        if !mine.is_empty() && mine.iter().all(|x| !x.clip) {
            out.push(json!({"kind": "element", "weight": mine.iter().map(|x| x.size).sum::<i64>() * 2,
                "lead": format!("the element \"{e}\" ({} routine(s)) never ran in a clip: a clip with it on screen would show what it does", mine.len())}));
        }
    }
    // The queue's open items.
    let q: Vec<Value> = fs::read_to_string(root.join("queue.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for row in &q {
        for o in row["open"].as_array().into_iter().flatten() {
            if let Some(o) = o.as_str() {
                out.push(
                    json!({"kind": "open", "weight": 400, "mechanic": row["mechanic"],
                    "lead": format!("{}: {o}", row["mechanic"].as_str().unwrap_or(""))}),
                );
            }
        }
    }
    out.sort_by(|a, b| {
        b["weight"]
            .as_i64()
            .cmp(&a["weight"].as_i64())
            .then(a["lead"].as_str().cmp(&b["lead"].as_str()))
    });
    let total_code: i64 = rs.values().map(|r| r.size).sum();
    let clip_code: i64 = rs.values().filter(|r| r.clip).map(|r| r.size).sum();
    let summary = json!({
        "clips": cov.clips, "frames": cov.frames,
        "routines": rs.len(), "routines_in_clips": rs.values().filter(|r| r.clip).count(),
        "code_bytes": total_code, "code_bytes_in_clips": clip_code,
        "ram_written_by_stage0": ram_written, "ram_no_clip_wrote": ram_unseen,
    });
    Ok((out, summary))
}
