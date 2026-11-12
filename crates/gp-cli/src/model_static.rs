//! The static harvest of a ROM's code (gp_nes::harvest), seeded from what
//! runs did: the helpers of game-printer's `gp model static` that `gp
//! explore` and `gp watch` read the code map with, under their old names.
//! Wave 3b brings `gp model static` itself on top of them.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use gp_nes::harvest::{self, Harvest, Input, Table};
use toml::Value as Toml;

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
