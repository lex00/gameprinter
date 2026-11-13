//! Stage 0 on an arcade romset: the ROM dataset's rows (`gp dataset rom`,
//! docs/formats/rom-dataset.md) and the routine entries a run passes, for
//! `gp mechanic harvest` and `gp mechanic leads`.
//!
//! The romset is run under gp-explore's arcade starts
//! ([`gp_explore::arcade::start_set`]: idle, a coin and Start, then seeded
//! stick and tap runs) and read through the main Z80's code map
//! ([`crate::code_z80::CodeMap`]): its routines (a call's, restart's or
//! interrupt's target, or an entry), the conditional branches in them with
//! the ways the runs took each, and the RAM bytes the runs wrote with the
//! routines that wrote them. The tables are the NES dataset's, so datasets
//! of both platforms union. A column with no arcade meaning is null where
//! the schema allows it; the `rom` row's `mapper` (an iNES number, never
//! null) is 0 and its `board` names the arcade board. The NES's other
//! readers (elements, state machines, constants, mechanics, facts, links,
//! estimates) read 6502 code, so those tables are empty and one `open` row
//! says so. The sound Z80's code isn't in the tables: its routines would
//! share addresses with the main CPU's.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use gp_arcade::Romset;
use gp_arcade::dasm::Flow;
use gp_arcade::levels::{EdgeKind, Levels};
use serde_json::json;

use crate::code_z80::CodeMap;
use crate::rom_dataset::{Dataset, tool};

/// The work RAM the main CPU's game state is in (Midway MCR: E000-E7FF),
/// the window of RAM the harvest and the leads read, as the NES's 2 KB.
pub(crate) const RAM_BASE: u16 = 0xe000;
pub(crate) const RAM_END: u16 = 0xe800;

/// The routine an address is in: the nearest entry at or below it.
pub(crate) fn routine_of(map: &CodeMap, pc: u16) -> Option<u16> {
    map.routines.range(..=pc).next_back().map(|(a, _)| *a)
}

/// The routines the starts reach, read from the code with what they ran.
pub(crate) fn code_map(set: &Romset, levels: Option<&Levels>) -> CodeMap {
    CodeMap::of(set, 0, levels.map(|l| &l.cpus[0]))
}

/// A run's name and kind, as the NES dataset's `runs` table has them.
fn kind_of(id: &str, seed: u64) -> (&'static str, Option<i64>) {
    match id {
        "idle" => ("idle", None),
        "start" => ("start", None),
        r if r.starts_with("rand") => (
            "random",
            r[4..].parse::<i64>().ok().map(|k| seed as i64 + k - 1),
        ),
        _ => ("clip", None),
    }
}

/// The ROM dataset of an arcade romset: `runs` seeded runs from `seed`.
pub(crate) fn extract(set: &Romset, runs: usize, seed: u64) -> Result<Dataset> {
    let sha = set.identity.clone();
    let tool = tool();
    let starts = crate::reach::Starts {
        runs,
        seed,
        start_frames: gp_explore::arcade::START_FRAMES,
        no_starts: false,
    };
    let clips = starts.with_platform(&[], &gp_evidence::Paths::from_env()?, true)?;
    let mut merged = Levels::default();
    let mut each = Vec::new();
    for c in &clips {
        let l = crate::code_z80::run_input(set, &c.inputs)?;
        crate::code_z80::merge(&mut merged, l.clone());
        each.push((c.id.clone(), c.key()?, c.inputs.len(), l));
    }
    let map = code_map(set, Some(&merged));
    let lv = &merged.cpus[0];
    let mut d = Dataset::default();
    let frames: usize = each.iter().map(|e| e.2).sum();
    let main_bytes: usize = crate::code_z80::chips(set, 0)
        .iter()
        .map(|c| (c.2 - c.1) as usize)
        .sum();
    d.push(
        "rom",
        json!({"rom": sha, "mapper": 0, "board": format!("midway-mcr ({})", set.name),
               "prg_bytes": main_bytes, "chr_bytes": set.gfx1.len() + set.gfx2.len(),
               "asset_map": null, "runs": each.len(), "frames": frames,
               "format": crate::rom_dataset::FORMAT, "tool": tool}),
    )?;

    // Routines: extent, callers and callees from the code, `ran` from the
    // runs' instruction starts.
    let name = |a: u16| format!("{a:04X}");
    let mut callees: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    for (&pc, i) in &map.insns {
        let Some(r) = routine_of(&map, pc) else {
            continue;
        };
        // A call or restart, or a jump into another routine.
        if let Some(t) = i.flow.target()
            && map.routines.contains_key(&t)
            && match i.flow {
                Flow::Call(_) | Flow::CallIf(_) | Flow::Rst(_) => true,
                Flow::Jump(_) | Flow::JumpIf(_) => t != r,
                _ => false,
            }
        {
            callees.entry(r).or_default().insert(t);
        }
        if matches!(i.flow, Flow::Indirect(_)) {
            for to in lv.indirect_targets(pc).keys() {
                if map.routines.contains_key(to) {
                    callees.entry(r).or_default().insert(*to);
                }
            }
        }
    }
    let mut callers: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    for (from, tos) in &callees {
        for to in tos {
            callers.entry(*to).or_default().insert(*from);
        }
    }
    let entries: Vec<u16> = map.routines.keys().copied().collect();
    for (k, &at) in entries.iter().enumerate() {
        let end = entries.get(k + 1).map_or(0x1_0000, |e| u32::from(*e));
        let last = map
            .insns
            .range(at..)
            .take_while(|(pc, _)| u32::from(**pc) < end)
            .last()
            .map_or(u32::from(at), |(pc, i)| u32::from(*pc) + u32::from(i.len));
        let ran = lv.starts.contains_key(&at);
        let list = |s: Option<&BTreeSet<u16>>| -> Result<String> {
            Ok(serde_json::to_string(
                &s.into_iter()
                    .flatten()
                    .map(|a| name(*a))
                    .collect::<Vec<_>>(),
            )?)
        };
        d.push(
            "routines",
            json!({"rom": sha, "routine": name(at), "bank": null, "addr": at,
                   "size": last - u32::from(at), "element": null, "read": true, "ran": ran,
                   "callers": list(callers.get(&at))?, "callees": list(callees.get(&at))?,
                   "src": format!("read:{}", name(at)),
                   "rule": format!("the main Z80's code map: {}", map.routines[&at]), "tool": tool}),
        )?;
    }

    // Branches: each conditional instruction, its compare the `cp n`
    // before it, and the ways the runs took it.
    let mut prev: Option<&gp_arcade::dasm::Insn> = None;
    for (&pc, i) in &map.insns {
        if i.flow.conditional() {
            let (mut taken, mut fell) = (0u64, 0u64);
            for ((_, _, k), s) in lv
                .edges
                .range((pc, 0, EdgeKind::Jump)..=(pc, u16::MAX, EdgeKind::Loop))
            {
                if *k == EdgeKind::Fall {
                    fell += s.count;
                } else {
                    taken += s.count;
                }
            }
            let reached = lv.starts.contains_key(&pc);
            // The mnemonic and its condition: `jr nz`, `djnz`, `ret c`.
            let op = i
                .text
                .split(',')
                .next()
                .unwrap_or(&i.text)
                .split_whitespace()
                .filter(|w| !w.starts_with('$'))
                .collect::<Vec<_>>()
                .join(" ");
            let compare = prev.filter(|p| p.text.starts_with("cp $")).and_then(|p| {
                let v = p.text.trim_start_matches("cp ").trim_start_matches('$');
                i64::from_str_radix(v, 16).ok()
            });
            d.push(
                "branches",
                json!({"rom": sha, "pc": name(pc), "routine": routine_of(&map, pc).map(name),
                       "op": op, "tests": "[]", "compare": compare,
                       "taken": reached.then_some(taken), "not_taken": reached.then_some(fell),
                       "src": format!("read:{}", name(pc)),
                       "rule": "a conditional jump, call or return in the main Z80's code map; its compare from a `cp n` before it", "tool": tool}),
            )?;
        }
        prev = Some(i);
    }

    // RAM: every work RAM byte (E000-E7FF) the runs wrote, with its
    // writers' routines. Sprite, video and palette RAM are the picture's.
    for (&a, _) in lv.written.range(RAM_BASE..RAM_END) {
        let writers: BTreeSet<String> = lv
            .writers
            .get(&a)
            .into_iter()
            .flatten()
            .filter_map(|(pc, _)| routine_of(&map, *pc).map(name))
            .collect();
        d.push(
            "ram",
            json!({"rom": sha, "addr": a, "written": true, "readers": "[]",
                   "writers": serde_json::to_string(&writers)?, "src": "run",
                   "rule": "written by a run (the main Z80's access log); readers aren't kept", "tool": tool}),
        )?;
        d.push(
            "open",
            json!({"rom": sha, "what": format!("ram:${a:04X}"), "routine": writers.iter().next(),
                   "pc": "", "bytes": serde_json::to_string(&[format!("${a:04X}")])?,
                   "why": "no rule gives its role or its element", "settle_by": "trace",
                   "src": "run", "rule": "every written byte has a row", "tool": tool}),
        )?;
    }
    d.push(
        "open",
        json!({"rom": sha, "what": "arcade: elements, state machines, constants, mechanics, facts",
               "pc": "", "bytes": "[]",
               "why": "stage 0's readers of these read 6502 code; the main Z80's code map gives routines, branches, RAM writers and runs only",
               "settle_by": "reader", "src": "run", "rule": "what the arcade dataset leaves out", "tool": tool}),
    )?;

    // Runs.
    for (id, key, n, l) in &each {
        let c = &l.cpus[0];
        let (kind, s) = kind_of(id, seed);
        let entered = map
            .routines
            .keys()
            .filter(|a| c.starts.contains_key(a))
            .count();
        let executed: u32 = c.executed_ranges().iter().map(|(a, b)| b - a).sum();
        d.push(
            "runs",
            json!({"rom": sha, "run": key, "kind": kind, "seed": s, "frames": n,
                   "routines": entered, "prg_bytes": executed,
                   "ram_written": c.written.range(RAM_BASE..RAM_END).count(),
                   "rule": "stage 0's starts", "tool": tool}),
        )?;
    }
    Ok(d)
}

/// The romset's bytes, for the check that a dataset holds none of them.
pub(crate) fn rom_bytes(set: &Romset) -> Vec<u8> {
    let mut raw = set.maincpu.clone();
    raw.extend_from_slice(&set.ssiocpu);
    raw.extend_from_slice(&set.gfx1);
    raw.extend_from_slice(&set.gfx2);
    raw
}

/// The routine entries a frame's main-CPU trace ran, in order, once each:
/// an instruction a call or restart went to, an interrupt handler's first,
/// or an address in `known` (a dataset's routines).
pub(crate) fn entries(trace: &[gp_arcade::mcr::Trace], known: &BTreeSet<u16>) -> Vec<u16> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut via_call = false;
    for e in trace.iter().filter(|e| e.cpu == 0) {
        if e.interrupt.is_some() {
            via_call = true;
            continue;
        }
        if (via_call || known.contains(&e.pc)) && seen.insert(e.pc) {
            out.push(e.pc);
        }
        let i = gp_arcade::dasm::decode(&e.bytes, e.pc);
        via_call = matches!(i.flow, Flow::Call(_) | Flow::CallIf(_) | Flow::Rst(_));
    }
    out
}

/// The routines a ROM dataset lists, by address.
pub(crate) fn dataset_routines(d: Option<&Dataset>) -> BTreeSet<u16> {
    d.into_iter()
        .flat_map(|d| d.rows("routines"))
        .filter_map(|r| match r.get("addr") {
            Some(crate::rom_dataset::Cell::I(a)) => u16::try_from(*a).ok(),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With the rbtapper romset: the idle and Start runs give a dataset
    /// with the board's row, routines that ran, branches both ways and work
    /// RAM rows, written without a byte of the romset. Skipped without it.
    #[test]
    fn an_arcade_dataset_reads_the_main_z80() {
        let Some(zip) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let set = Romset::from_zip(&std::fs::read(zip).unwrap()).unwrap();
        let mut d = extract(&set, 0, 1).unwrap();
        d.sort();
        let rom = &d.rows("rom")[0];
        assert_eq!(
            rom["board"],
            crate::rom_dataset::Cell::S(Some("midway-mcr (rbtapper)".into()))
        );
        assert_eq!(d.rows("runs").len(), 2);
        let ran = d
            .rows("routines")
            .iter()
            .filter(|r| r["ran"] == crate::rom_dataset::Cell::B(true))
            .count();
        assert!(ran > 20, "{ran}");
        assert!(!d.rows("branches").is_empty());
        assert!(d.rows("ram").iter().all(
            |r| matches!(r["addr"], crate::rom_dataset::Cell::I(a) if (0xe000..0xe800).contains(&a))
        ));
        let dir = tempfile::tempdir().unwrap();
        crate::rom_dataset::write(
            dir.path(),
            &d,
            &set.identity,
            "t",
            Some(&rom_bytes(&set)),
            "arcade romset",
        )
        .unwrap();
        // A frame's entries: the reset routine's first, then what it calls.
        let mut t = gp_arcade::Tapper::power_on(&set);
        let trace = crate::covered::frame_trace(&mut t, gp_arcade::Controls::default());
        assert!(!entries(&trace, &dataset_routines(Some(&d))).is_empty());
    }
}
