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
//! null) is 0 and its `board` names the arcade board. Each RAM byte's
//! readers come from the runs' access log, and the role the main CPU's
//! trace suggests (crate::ramrole_z80, gp_nes::ramrole's rules) goes into
//! its open row, unstated. The elements, state machines, constants, facts
//! and mechanics come from the main Z80's static harvest
//! ([`crate::model_static_z80::harvest_of`], the NES harvest's shape)
//! through the NES dataset's own rows code
//! ([`crate::rom_extract::harvest_claims`] and
//! [`crate::rom_extract::harvest_rows`]), and the RAM bytes the harvest
//! places carry their element, slot and quantity as on the NES. What the
//! NES reads from its runs' tracks (the player and slot groups the screen
//! shows, links, estimates, a byte's unit) isn't read on the board, and
//! the Z80 harvest names no mechanic part: `open` rows say so. The sound
//! Z80's code isn't in the tables: its routines would share addresses with
//! the main CPU's.

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
    // One RAM trace across the runs, as the NES's behave keeps one, and
    // each work RAM byte's readers by instruction.
    let mut roles = crate::ramrole_z80::Z80Roles::new();
    let mut read_by: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    for c in &clips {
        roles.next_run();
        let l = crate::code_z80::run_input_with(set, &c.inputs, &mut |w, t| {
            roles.feed(&w.trace, &w.accesses);
            let ram: Vec<u8> = (RAM_BASE..RAM_END).map(|a| t.peek(a)).collect();
            roles.frame_end(&ram);
            for a in w.accesses.iter().filter(|a| {
                a.cpu == 0
                    && a.kind == gp_arcade::mcr::AccessKind::Read
                    && (RAM_BASE..RAM_END).contains(&a.addr)
            }) {
                read_by.entry(a.addr).or_default().insert(a.pc);
            }
        })?;
        crate::code_z80::merge(&mut merged, l.clone());
        each.push((c.id.clone(), c.key()?, c.inputs.len(), l));
    }
    let class: BTreeMap<u16, gp_nes::ramrole::RamRole> = roles
        .classify()
        .roles
        .into_iter()
        .map(|r| (r.addr + RAM_BASE, r))
        .collect();
    let map = code_map(set, Some(&merged));
    let lv = &merged.cpus[0];
    // The main Z80's static harvest and the RAM bytes it places, by the
    // NES's rules 1 to 3 (crate::rom_extract::harvest_claims); a byte two
    // elements claim is left out, as on the NES.
    let h = crate::model_static_z80::harvest_of(set);
    let mut owner = crate::rom_extract::Owner::new();
    let mut ambiguous = BTreeSet::new();
    crate::rom_extract::harvest_claims(&h, &mut owner, &mut ambiguous);
    for a in &ambiguous {
        owner.remove(a);
    }
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
    // Each routine's element, as the harvest groups them.
    let element_of: BTreeMap<String, String> = h
        .routines
        .iter()
        .filter(|r| !r.element.is_empty())
        .map(|r| (r.addr.clone(), r.element.clone()))
        .collect();
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
                   "size": last - u32::from(at), "element": element_of.get(&name(at)),
                   "read": true, "ran": ran,
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

    // RAM: every work RAM byte (E000-E7FF) the runs wrote or the harvest
    // places, with its writers' and readers' routines, the element, slot
    // and quantity the harvest's rules give it (as on the NES), and the
    // role the main Z80's trace suggests (crate::ramrole_z80,
    // gp_nes::ramrole's rules) in its open row. The NES states the
    // classifier's input role, graded exact on its games; on the board it
    // isn't: stage 0's idle and Start runs give Tapper 383 bytes by the
    // mask rule, most of them filled by the power-on RAM test, so no role
    // is stated. Sprite, video and palette RAM are the picture's.
    let mut bytes: BTreeSet<u16> = lv
        .written
        .range(RAM_BASE..RAM_END)
        .map(|(a, _)| *a)
        .collect();
    bytes.extend(owner.range(RAM_BASE..RAM_END).map(|(a, _)| *a));
    for &a in &bytes {
        let written = lv.written.contains_key(&a);
        let writers: BTreeSet<String> = lv
            .writers
            .get(&a)
            .into_iter()
            .flatten()
            .filter_map(|(pc, _)| routine_of(&map, *pc).map(name))
            .collect();
        let readers: BTreeSet<String> = read_by
            .get(&a)
            .into_iter()
            .flatten()
            .filter_map(|pc| routine_of(&map, *pc).map(name))
            .collect();
        let placed = owner.get(&a);
        let mut src: Vec<String> = placed
            .map(|o| o.3.clone())
            .filter(|s| !s.is_empty())
            .into_iter()
            .collect();
        if written {
            src.push("run".to_owned());
        }
        let rule = match placed {
            Some(o) => o.4.clone(),
            None => "written and read by a run (the main Z80's access log)".to_owned(),
        };
        d.push(
            "ram",
            json!({"rom": sha, "addr": a, "written": written,
                   "element": placed.map(|o| &o.0), "slot": placed.and_then(|o| o.1),
                   "quantity": placed.and_then(|o| o.2.as_ref()),
                   "readers": serde_json::to_string(&readers)?,
                   "writers": serde_json::to_string(&writers)?,
                   "src": if src.len() == 1 { json!(src[0]) } else { json!(serde_json::to_string(&src)?) },
                   "rule": rule, "tool": tool}),
        )?;
        let what = if placed.is_some() {
            "no rule gives its role"
        } else {
            "no rule gives its role or its element"
        };
        let why = match class.get(&a) {
            Some(c) => format!(
                "{what}; the runs suggest {} ({}), a class not exact enough to state",
                c.role.name(),
                c.why
            ),
            None => what.to_owned(),
        };
        d.push(
            "open",
            json!({"rom": sha, "what": format!("ram:${a:04X}"), "element": placed.map(|o| &o.0),
                   "routine": writers.iter().next(),
                   "pc": "", "bytes": serde_json::to_string(&[format!("${a:04X}")])?,
                   "why": why, "settle_by": "trace",
                   "src": "run", "rule": "every written byte has a row", "tool": tool}),
        )?;
    }
    // Elements, states, transitions, constants, facts and mechanics: the
    // NES dataset's rows of a harvest. The board's runs aren't tracked, so
    // nothing but the harvest places a byte or gives one a unit.
    crate::rom_extract::harvest_rows(
        &mut d,
        &sha,
        &h,
        &crate::rom_extract::Placed {
            owner: &owner,
            part_of: &BTreeMap::new(),
            speed_scale: &BTreeMap::new(),
        },
    )?;
    // What the NES reads from its runs' tracks and the board's dataset
    // doesn't, in words.
    d.push(
        "open",
        json!({"rom": sha, "what": "arcade: elements the screen shows, links, estimates, units",
               "pc": "", "bytes": "[]",
               "why": "the NES dataset finds the player and the groups of slots the screen shows, the routines that draw and update them (links), what the runs measure (estimates) and the unit of a byte a constant is stored in or compared with, by following the runs' sprites (gp-track on the NES's sprite table and pad); stage 0 on the board doesn't track its sprites, so only the main Z80's harvest places bytes here",
               "settle_by": "trace", "src": "run", "rule": "what the arcade dataset leaves out", "tool": tool}),
    )?;
    if d.rows("mechanics").is_empty() {
        d.push(
            "open",
            json!({"rom": sha, "what": "arcade: mechanics",
                   "pc": "", "bytes": "[]",
                   "why": "a mechanics row is a harvest fact on a part of the catalogue (docs/mechanics.md), and the catalogue's parts are the platformer's; the main Z80's harvest has the rules an arcade game shares with an NES one (record tables, state bytes, counters, timers, input, random), not the platformer's, so none of its facts names a part",
                   "settle_by": "name", "src": "run", "rule": "what the arcade dataset leaves out", "tool": tool}),
        )?;
    }

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

    /// The made-up records program (gp_arcade::testrom::RECORDS) read
    /// through the NES's rows of a harvest: its record tables are elements
    /// of slots, the state byte of the 32-byte records a machine with its
    /// transitions, each slot's state byte a RAM row of its element, the
    /// compare at 0629 a constant, and the reset's constants named by its
    /// address. What the board's dataset lacks is said in open rows.
    #[test]
    fn an_arcade_dataset_has_the_harvests_rows() {
        use crate::rom_dataset::Cell;
        let d = extract(&gp_arcade::testrom::records(), 0, 1).unwrap();
        let find = |t: &str, col: &str, v: Cell| {
            d.rows(t)
                .iter()
                .find(|r| r[col] == v)
                .cloned()
                .unwrap_or_else(|| panic!("no {t} row with {col} {v:?}"))
        };
        let s = |v: &str| Cell::S(Some(v.into()));
        let el = find("elements", "element", s("slots_E200"));
        assert_eq!(el["kind"], s("slots"));
        assert_eq!(el["slots"], Cell::OI(Some(4)));
        assert_eq!(el["ram"], Cell::I(4), "the state byte of each slot");
        let st = find("states", "state", s("s1"));
        assert_eq!(st["byte"], Cell::I(0xe200));
        assert_eq!(st["element"], s("slots_E200"));
        let moves: Vec<(String, String, String)> = d
            .rows("transitions")
            .iter()
            .map(|r| {
                let t = |c: &str| match &r[c] {
                    Cell::S(Some(v)) => v.clone(),
                    _ => String::new(),
                };
                (t("from_state"), t("to_state"), t("routine"))
            })
            .collect();
        assert!(
            moves.contains(&("s0".into(), "s1".into(), "0600".into())),
            "{moves:?}"
        );
        let ram = find("ram", "addr", Cell::I(0xe220));
        assert_eq!(ram["element"], s("slots_E200"));
        assert_eq!(ram["slot"], Cell::OI(Some(1)));
        assert_eq!(ram["quantity"], s("state"));
        assert_eq!(
            ram["role"],
            Cell::S(None),
            "a role stays the runs' suggestion"
        );
        let c = find("constants", "pc", s("0629"));
        assert_eq!(
            (c["op"].clone(), c["raw"].clone()),
            (s("cp"), Cell::I(0x80))
        );
        assert_eq!(c["routine"], s("0620"));
        assert_eq!(find("constants", "pc", s("0004"))["routine"], s("0000"));
        assert!(!d.rows("facts").is_empty());
        for what in [
            "arcade: mechanics",
            "arcade: elements the screen shows, links, estimates, units",
        ] {
            find("open", "what", s(what));
        }
    }

    /// A made-up program (crate::ramrole_z80's test's): the byte the
    /// routine at 0040 writes from the controller has its writer, and the
    /// runs' suggestion of input in its open row; the main loop's byte its
    /// reader and the suggestion of state. No role is stated.
    #[test]
    fn an_arcade_datasets_ram_rows_have_roles_and_readers() {
        let set = crate::ramrole_z80::tests::program();
        let d = extract(&set, 0, 1).unwrap();
        let ram = |a: i64| {
            d.rows("ram")
                .iter()
                .find(|r| r["addr"] == crate::rom_dataset::Cell::I(a))
                .cloned()
                .unwrap_or_else(|| panic!("no row for {a:04X}"))
        };
        let input = ram(0xe000);
        assert_eq!(input["role"], crate::rom_dataset::Cell::S(None));
        assert_eq!(
            input["writers"],
            crate::rom_dataset::Cell::S(Some("[\"0040\"]".into()))
        );
        let state = ram(0xe030);
        assert_eq!(state["role"], crate::rom_dataset::Cell::S(None));
        assert_eq!(
            state["readers"],
            crate::rom_dataset::Cell::S(Some("[\"0000\"]".into()))
        );
        let open = d
            .rows("open")
            .iter()
            .find(|r| r["what"] == crate::rom_dataset::Cell::S(Some("ram:$E030".into())))
            .cloned()
            .unwrap();
        let crate::rom_dataset::Cell::S(Some(why)) = &open["why"] else {
            panic!("{open:?}")
        };
        assert!(why.contains("the runs suggest state"), "{why}");
    }
}
