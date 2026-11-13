//! `gp code listing`, `gp code tables`, `gp reach` and `gp coverage` on an
//! arcade romset (port wave 6): the main and sound Z80s' code, read with
//! gp-arcade's disassembler ([`gp_arcade::dasm`]) and what runs of it did
//! ([`gp_arcade::levels`]), a listing and a code map per CPU, by ROM chip.
//!
//! The code map is a recursive descent from each CPU's reset at 0000 (a
//! restart vector or the NMI's at 66 is code only once something reaches
//! it: an `rst`, or a run's interrupt) and from every instruction a run
//! started, every interrupt handler and every target a
//! `jp (hl)`, `jp (ix)` or `jp (iy)` went to in a run, which no reading of
//! the code alone can find. A routine is a call's or restart's target, an
//! interrupt handler, or an entry; each instruction belongs to the nearest
//! routine at or below it.
//!
//! Tapper's main CPU sees 56 KB of ROM at 0000-DFFF with no banking (the
//! board has no bank switch: mcr.rs maps it whole), so an address is a
//! place in one chip; the sound CPU's 16 KB is at 0000-3FFF.
//!
//! Nothing here goes in the repo: a listing is ROM-derived, so `--write`
//! refuses a git work tree, as for the NES.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use gp_arcade::dasm::{self, Flow, Insn};
use gp_arcade::levels::{CpuLevels, EdgeKind, Levels, Watcher};
use gp_arcade::romset::{self, Region};
use gp_arcade::{Controls, Romset, Tapper};
use gp_evidence::input_log::InputFrame;
use serde_json::{Value, json};

use crate::reach::ClipInput;

/// The two CPUs, by the names the output uses.
pub(crate) const CPUS: [&str; 2] = ["main", "sound"];

/// The ROM a CPU runs, and where its ROM ends in its address space.
pub(crate) fn program(set: &Romset, cpu: usize) -> (&[u8], u32) {
    if cpu == 0 {
        (&set.maincpu, 0xe000)
    } else {
        (&set.ssiocpu, 0x4000)
    }
}

/// A CPU's chips: name, first address, end (excluded).
pub(crate) fn chips(set: &Romset, cpu: usize) -> Vec<(&'static str, u32, u32)> {
    let region = if cpu == 0 {
        Region::MainCpu
    } else {
        Region::SsioCpu
    };
    romset::set(set.name)
        .map(|s| {
            s.chips
                .iter()
                .filter(|c| c.region == region)
                .map(|c| (c.name, c.offset as u32, (c.offset + c.size) as u32))
                .collect()
        })
        .unwrap_or_else(|| vec![("program", 0, program(set, cpu).1)])
}

fn chip_of(chips: &[(&'static str, u32, u32)], a: u16) -> &'static str {
    chips
        .iter()
        .find(|c| (c.1..c.2).contains(&u32::from(a)))
        .map_or("-", |c| c.0)
}

/// The instruction at `pc` of a CPU's ROM.
pub(crate) fn insn_at(set: &Romset, cpu: usize, pc: u16) -> Option<Insn> {
    let (rom, end) = program(set, cpu);
    if u32::from(pc) >= end {
        return None;
    }
    let at = usize::from(pc);
    let bytes: Vec<u8> = (0..4)
        .map(|i| rom.get(at + i).copied().unwrap_or(0))
        .collect();
    Some(dasm::decode(&bytes, pc))
}

/// Runs each clip from power-on, watching the frames in `window` (frame
/// numbers counted from 1 within the clip), and merges their levels.
pub(crate) fn run_clips(set: &Romset, clips: &[ClipInput], window: (u64, u64)) -> Result<Levels> {
    let mut levels = Levels::default();
    for clip in clips {
        let mut t = Tapper::power_on(set);
        let mut w = Watcher::new(false);
        for (i, f) in clip.inputs.iter().enumerate() {
            let c = controls(f).with_context(|| format!("clip {}, frame {}", clip.id, i + 1))?;
            let frame = i as u64 + 1;
            if frame < window.0 || frame > window.1 {
                t.step(c);
            } else {
                w.step_quiet(&mut t, c);
            }
        }
        merge(&mut levels, w.finish());
    }
    Ok(levels)
}

/// Runs one input from power-on, watching every frame.
pub(crate) fn run_input(set: &Romset, inputs: &[InputFrame]) -> Result<Levels> {
    let mut t = Tapper::power_on(set);
    let mut w = Watcher::new(false);
    for f in inputs {
        w.step_quiet(&mut t, controls(f)?);
    }
    Ok(w.finish())
}

fn controls(f: &InputFrame) -> Result<Controls> {
    let bits = f.buttons_word();
    if bits > 0xff || f.reset {
        bail!(
            "an arcade input frame holds bits 0 to 7 (coin, start, right, left, down, up, tap, tilt) and no reset; this one holds {bits:#x}{}",
            if f.reset { " and a reset" } else { "" }
        );
    }
    Ok(Controls::from_bits(bits))
}

pub(crate) fn merge(into: &mut Levels, from: Levels) {
    for (a, b) in into.cpus.iter_mut().zip(from.cpus) {
        for (k, v) in b.starts {
            let e = a.starts.entry(k).or_default();
            e.first = if e.count == 0 {
                v.first
            } else {
                e.first.min(v.first)
            };
            e.count += v.count;
        }
        a.lengths.extend(b.lengths);
        for (k, v) in b.written {
            let e = a.written.entry(k).or_default();
            e.count += v.count;
        }
        for (k, v) in b.writers {
            let e = a.writers.entry(k).or_default();
            for (pc, n) in v {
                *e.entry(pc).or_default() += n;
            }
        }
        for (k, v) in b.ports {
            a.ports.entry(k).or_default().count += v.count;
        }
        for (k, v) in b.edges {
            let e = a.edges.entry(k).or_default();
            e.first = if e.count == 0 {
                v.first
            } else {
                e.first.min(v.first)
            };
            e.count += v.count;
        }
    }
    into.watched += from.watched;
}

/// One CPU's code map.
pub(crate) struct CodeMap {
    pub cpu: usize,
    pub insns: BTreeMap<u16, Insn>,
    /// Routine entries and why each is one.
    pub routines: BTreeMap<u16, &'static str>,
    pub chips: Vec<(&'static str, u32, u32)>,
    /// The CPU's program, for the listing's bytes.
    pub rom: Vec<u8>,
}

impl CodeMap {
    /// The descent from the entries and what `ran` saw.
    pub fn of(set: &Romset, cpu: usize, ran: Option<&CpuLevels>) -> Self {
        let end = program(set, cpu).1;
        let mut routines: BTreeMap<u16, &'static str> = BTreeMap::new();
        routines.insert(0, "reset");
        let mut todo: Vec<u16> = vec![0];
        if let Some(r) = ran {
            todo.extend(r.starts.keys());
            for (_, to, kind) in r.edges.keys() {
                match kind {
                    EdgeKind::Interrupt => {
                        routines.entry(*to).or_insert("interrupt handler");
                    }
                    EdgeKind::Indirect => {
                        routines.entry(*to).or_insert("jp (..) target");
                    }
                    _ => {}
                }
                todo.push(*to);
            }
        }
        let mut insns = BTreeMap::new();
        let mut seen = BTreeSet::new();
        while let Some(pc) = todo.pop() {
            if u32::from(pc) >= end || !seen.insert(pc) {
                continue;
            }
            let Some(i) = insn_at(set, cpu, pc) else {
                continue;
            };
            let next = pc.wrapping_add(u16::from(i.len));
            match i.flow {
                Flow::Call(t) | Flow::CallIf(t) => {
                    routines.entry(t).or_insert("called");
                    todo.push(t);
                }
                Flow::Rst(t) => {
                    routines.entry(t).or_insert("restart");
                    todo.push(t);
                }
                Flow::Jump(t) | Flow::JumpIf(t) | Flow::Djnz(t) => todo.push(t),
                _ => {}
            }
            if i.flow.falls_through() {
                todo.push(next);
            }
            insns.insert(pc, i);
        }
        routines.retain(|at, _| insns.contains_key(at));
        Self {
            cpu,
            insns,
            routines,
            chips: chips(set, cpu),
            rom: program(set, cpu).0.to_vec(),
        }
    }

    /// The listing of one routine's instructions, with what `ran` did.
    pub fn routine_text(&self, at: u16, ran: Option<&CpuLevels>, out: &mut String) {
        let end = self
            .routines
            .range(at.saturating_add(1)..)
            .next()
            .map_or(0x1_0000, |(a, _)| u32::from(*a));
        let why = self.routines.get(&at).copied().unwrap_or("entry");
        let calls = ran.map_or(0, |r| {
            r.edges
                .iter()
                .filter(|((_, to, k), _)| {
                    *to == at && matches!(k, EdgeKind::Call | EdgeKind::Rst | EdgeKind::Interrupt)
                })
                .map(|(_, s)| s.count)
                .sum::<u64>()
        });
        let _ = writeln!(
            out,
            "; {} ${at:04X}, {why}, chip {}{}",
            CPUS[self.cpu],
            chip_of(&self.chips, at),
            if ran.is_some() {
                format!(", entered {calls} times")
            } else {
                String::new()
            }
        );
        for (&pc, i) in self.insns.range(at..) {
            if u32::from(pc) >= end {
                break;
            }
            let bytes = (0..i.len)
                .map(|k| {
                    let b = self.rom.get(usize::from(pc) + usize::from(k));
                    format!("{:02X}", b.copied().unwrap_or(0))
                })
                .collect::<Vec<_>>()
                .join(" ");
            let mut note = String::new();
            if let Some(r) = ran {
                let n = r.starts.get(&pc).map_or(0, |s| s.count);
                let _ = write!(note, "ran {n}");
                if i.flow.conditional() {
                    let (mut t, mut f) = (0, 0);
                    for ((_, _, k), s) in r
                        .edges
                        .range((pc, 0, EdgeKind::Jump)..=(pc, u16::MAX, EdgeKind::Loop))
                    {
                        match k {
                            EdgeKind::Fall => f += s.count,
                            _ => t += s.count,
                        }
                    }
                    let _ = write!(note, ", taken {t}, fell {f}");
                }
                if matches!(i.flow, Flow::Indirect(_)) {
                    let targets = r.indirect_targets(pc);
                    let list: Vec<String> = targets
                        .iter()
                        .map(|(to, n)| format!("${to:04X} {n}"))
                        .collect();
                    let _ = write!(note, ", to {}", list.join(", "));
                }
            }
            let _ = writeln!(
                out,
                "{pc:04X}  {bytes:<12} {:<24}{}",
                i.text,
                if note.is_empty() {
                    String::new()
                } else {
                    format!("; {note}")
                }
            );
        }
    }
}

/// `gp code listing` on a romset: every routine (or those chosen) of
/// both CPUs.
pub(crate) fn listing(
    set: &Romset,
    ran: Option<&Levels>,
    wanted: &[u16],
    cpu: Option<usize>,
) -> (String, Value) {
    let mut text = String::new();
    let mut maps = Vec::new();
    #[allow(clippy::needless_range_loop)] // c picks the CPU's ROM too
    for c in 0..2 {
        if cpu.is_some_and(|w| w != c) {
            continue;
        }
        let lv = ran.map(|r| &r.cpus[c]);
        let map = CodeMap::of(set, c, lv);
        for &at in map.routines.keys() {
            if !wanted.is_empty() && !wanted.contains(&at) {
                continue;
            }
            if lv.is_some_and(|l| !routine_ran(&map, l, at)) && wanted.is_empty() {
                continue;
            }
            map.routine_text(at, lv, &mut text);
            text.push('\n');
        }
        maps.push(json!({
            "cpu": CPUS[c],
            "instructions": map.insns.len(),
            "routines": map.routines.len(),
            "ran": lv.map(|l| l.starts.len()),
        }));
    }
    (text, json!({ "set": set.name, "cpus": maps }))
}

fn routine_ran(map: &CodeMap, lv: &CpuLevels, at: u16) -> bool {
    let end = map
        .routines
        .range(at.saturating_add(1)..)
        .next()
        .map_or(0x1_0000, |(a, _)| u32::from(*a));
    lv.starts
        .range(at..)
        .next()
        .is_some_and(|(pc, _)| u32::from(*pc) < end)
}

/// `gp reach` on a romset: what the runs' code did, per CPU and chip.
pub(crate) fn reach(set: &Romset, levels: &Levels) -> (String, Value) {
    let mut text = String::new();
    let mut cpus = Vec::new();
    #[allow(clippy::needless_range_loop)] // c picks the CPU's ROM too
    for c in 0..2 {
        let lv = &levels.cpus[c];
        let map = CodeMap::of(set, c, Some(lv));
        let ranges = lv.executed_ranges();
        let executed = |lo: u32, hi: u32| -> u32 {
            ranges
                .iter()
                .map(|(a, b)| (*b).min(hi).saturating_sub((*a).max(lo)))
                .sum()
        };
        let mut chip_rows = Vec::new();
        for (name, lo, hi) in chips(set, c) {
            let n = executed(lo, hi);
            chip_rows.push(
                json!({"chip": name, "from": lo, "to": hi, "bytes_executed": n, "bytes": hi - lo}),
            );
            let _ = writeln!(
                text,
                "{} chip {name} ({lo:04X}-{:04X}): {n} of {} bytes executed",
                CPUS[c],
                hi - 1,
                hi - lo
            );
        }
        let mut kinds: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
        for ((_, _, k), s) in &lv.edges {
            let e = kinds.entry(k.name()).or_default();
            e.0 += 1;
            e.1 += s.count;
        }
        let indirect: Vec<Value> = lv
            .edges
            .keys()
            .filter(|(_, _, k)| *k == EdgeKind::Indirect)
            .map(|(f, _, _)| *f)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|pc| {
                let t = lv.indirect_targets(pc);
                let _ = writeln!(
                    text,
                    "{} {} at ${pc:04X} went to {}",
                    CPUS[c],
                    insn_at(set, c, pc).map_or_else(|| "?".into(), |i| i.text),
                    t.iter()
                        .map(|(to, n)| format!("${to:04X} ({n})"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                json!({"at": pc, "targets": t.iter().map(|(to, n)| json!([to, n])).collect::<Vec<_>>()})
            })
            .collect();
        let _ = writeln!(
            text,
            "{}: {} instructions ran of {} in the code map, {} routines; edges {}",
            CPUS[c],
            lv.starts.len(),
            map.insns.len(),
            map.routines.len(),
            kinds
                .iter()
                .map(|(k, (n, c))| format!("{k} {n} ({c} times)"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        cpus.push(json!({
            "cpu": CPUS[c],
            "instructions_ran": lv.starts.len(),
            "instructions_mapped": map.insns.len(),
            "routines": map.routines.len(),
            "chips": chip_rows,
            "edges": kinds.iter().map(|(k, (n, c))| json!({"kind": k, "edges": n, "count": c})).collect::<Vec<_>>(),
            "indirect": indirect,
            "ports": lv.ports.iter().map(|((p, w), s)| json!({"port": p, "write": w, "count": s.count})).collect::<Vec<_>>(),
            "ram_written": lv.written.len(),
        }));
    }
    (
        text.trim_end().to_owned(),
        json!({"schema": "gp-arcade reach 0.1.0", "set": set.name, "frames": levels.watched, "cpus": cpus}),
    )
}

/// Coverage of a run on a romset: of the code map, the instructions run,
/// and of its conditional branches, those that went both ways.
pub(crate) fn coverage(set: &Romset, levels: &Levels) -> Value {
    let mut cpus = Vec::new();
    #[allow(clippy::needless_range_loop)] // c picks the CPU's ROM too
    for c in 0..2 {
        let lv = &levels.cpus[c];
        let map = CodeMap::of(set, c, Some(lv));
        let branches: Vec<u16> = map
            .insns
            .iter()
            .filter(|(_, i)| i.flow.conditional())
            .map(|(pc, _)| *pc)
            .collect();
        let ways = |pc: u16| {
            let mut kinds = BTreeSet::new();
            for ((_, _, k), _) in lv
                .edges
                .range((pc, 0, EdgeKind::Jump)..=(pc, u16::MAX, EdgeKind::Loop))
            {
                kinds.insert(*k == EdgeKind::Fall);
            }
            kinds.len()
        };
        let both = branches.iter().filter(|&&pc| ways(pc) == 2).count();
        let one = branches.iter().filter(|&&pc| ways(pc) == 1).count();
        cpus.push(json!({
            "cpu": CPUS[c],
            "instructions": map.insns.len(),
            "instructions_run": map.insns.keys().filter(|pc| lv.starts.contains_key(pc)).count(),
            "branches": branches.len(),
            "branches_both_ways": both,
            "branches_one_way": one,
        }));
    }
    json!({"set": set.name, "frames": levels.watched, "cpus": cpus})
}

/// `gp code tables` on a romset: ROM reads whose address came from the
/// index byte's value, found by following its reads through a run, as
/// tables with their entry size and the entry each value read.
pub(crate) fn tables(set: &Romset, inputs: &[InputFrame], index: u16) -> Result<(String, Value)> {
    use gp_arcade::dasm::{Addr, Ptr, reg};
    use gp_arcade::mcr::AccessKind;
    let mut t = Tapper::power_on(set);
    // (reader pc) -> index value -> addresses read
    let mut reads: BTreeMap<u16, BTreeMap<u8, BTreeSet<u16>>> = BTreeMap::new();
    for f in inputs {
        t.set_trace(true);
        t.set_accesses(true);
        t.step(controls(f)?);
        let mut trace: Vec<_> = t.take_trace().into_iter().filter(|e| e.cpu == 0).collect();
        trace.sort_by_key(|e| e.time);
        let acc: Vec<_> = t
            .take_accesses()
            .into_iter()
            .filter(|a| a.cpu == 0)
            .collect();
        t.set_trace(false);
        t.set_accesses(false);
        // Each read of the index byte, followed for 32 instructions through
        // the registers: (taint, value, age).
        let mut active: Vec<(u32, u8, u32)> = Vec::new();
        let mut j = 0;
        for (n, e) in trace.iter().enumerate() {
            let end = trace.get(n + 1).map(|x| x.time);
            let from = j;
            while j < acc.len() && end.is_none_or(|end| acc[j].time < end) {
                j += 1;
            }
            if e.interrupt.is_some() {
                continue;
            }
            let i = dasm::decode(&e.bytes, e.pc);
            let mine = &acc[from..j];
            let addr_regs = match i.mem.map(|m| m.addr) {
                Some(Addr::Pair(b)) => b,
                Some(Addr::Index(Ptr::Ix, _)) => reg::IX,
                Some(Addr::Index(Ptr::Iy, _)) => reg::IY,
                _ => 0,
            };
            for (taint, value, age) in &mut active {
                *age += 1;
                if addr_regs & *taint != 0 {
                    for a in mine
                        .iter()
                        .filter(|a| a.kind == AccessKind::Read && a.addr < 0xe000)
                    {
                        reads
                            .entry(e.pc)
                            .or_default()
                            .entry(*value)
                            .or_default()
                            .insert(a.addr);
                    }
                }
                let src = !i.constant && *taint & (i.reads & !addr_regs) != 0;
                *taint = (*taint & !i.writes) | if src { i.writes } else { 0 };
            }
            active.retain(|(taint, _, age)| *taint != 0 && *age < 32);
            if let Some(a) = mine
                .iter()
                .find(|a| a.kind == AccessKind::Read && a.addr == index)
                && i.writes != 0
            {
                active.push((i.writes & !reg::F, a.value, 0));
            }
        }
    }
    let mut text = String::new();
    let mut out = Vec::new();
    for (pc, by_value) in &reads {
        let all: BTreeSet<u16> = by_value.values().flatten().copied().collect();
        let (lo, hi) = (*all.first().expect("read"), *all.last().expect("read"));
        // The entry size: the smallest step between the first addresses
        // two values read.
        let firsts: BTreeSet<u16> = by_value
            .values()
            .filter_map(|s| s.first().copied())
            .collect();
        let size = firsts
            .iter()
            .zip(firsts.iter().skip(1))
            .map(|(a, b)| b - a)
            .min()
            .unwrap_or(1);
        let text_at = insn_at(set, 0, *pc).map_or_else(|| "?".into(), |i| i.text);
        let _ = writeln!(
            text,
            "table ${lo:04X}-${hi:04X}, entries of {size} bytes, read by ${pc:04X} `{text_at}` with ${index:04X} as the index:"
        );
        for (v, addrs) in by_value {
            let _ = writeln!(
                text,
                "  ${index:04X} = {v:3} reads {}",
                addrs
                    .iter()
                    .map(|a| format!("${a:04X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        let _ = writeln!(
            text,
            "  [[assets]] name = \"table_{lo:04x}\" region = \"maincpu\" offset = 0x{lo:04x} size = {}",
            u32::from(hi) - u32::from(lo) + 1
        );
        out.push(json!({
            "reader": pc,
            "text": text_at,
            "from": lo,
            "to": hi,
            "entry_size": size,
            "rows": by_value.iter().map(|(v, a)| json!({"value": v, "reads": a})).collect::<Vec<_>>(),
        }));
    }
    if out.is_empty() {
        text = format!(
            "no ROM table was read with ${index:04X} as the index in {} frames",
            inputs.len()
        );
    }
    Ok((
        text.trim_end().to_owned(),
        json!({"index": index, "tables": out}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn busy_levels(frames: usize) -> (Romset, Levels) {
        let set = gp_arcade::testrom::busy();
        let inputs = vec![InputFrame::default(); frames];
        let lv = run_input(&set, &inputs).unwrap();
        (set, lv)
    }

    /// The busy program's map: the descent finds its loop, the run adds
    /// the table targets only `jp (hl)` reaches, and the listing counts.
    #[test]
    fn the_map_takes_jp_hl_targets_from_the_run() {
        let (set, lv) = busy_levels(4);
        let bare = CodeMap::of(&set, 0, None);
        assert!(bare.insns.contains_key(&0x0029));
        assert!(!bare.insns.contains_key(&0x0050), "only a run finds it");
        let map = CodeMap::of(&set, 0, Some(&lv.cpus[0]));
        assert!(map.insns.contains_key(&0x0050) && map.insns.contains_key(&0x0058));
        assert_eq!(map.routines.get(&0x0050), Some(&"jp (..) target"));
        let (text, json) = listing(&set, Some(&lv), &[0x0000], Some(0));
        assert!(
            text.starts_with("; main $0000, reset, chip program"),
            "{text}"
        );
        assert!(text.contains("0029  E9           jp (hl)"), "{text}");
        assert!(text.contains(", to $0050"), "{text}");
        assert!(
            text.contains("0010  30 02        jr nc,$0014             ; ran 3"),
            "{text}"
        );
        assert!(text.contains(", taken "), "{text}");
        assert_eq!(json["cpus"][0]["cpu"], "main");
    }

    #[test]
    fn reach_and_coverage_count_the_run() {
        let (set, lv) = busy_levels(3);
        let (text, json) = reach(&set, &lv);
        assert!(text.contains("jp (hl) at $0029 went to $0050"), "{text}");
        assert!(json["cpus"][0]["ports"].as_array().unwrap().len() >= 3);
        let cov = coverage(&set, &lv);
        assert!(cov["cpus"][0]["branches"].as_u64().unwrap() >= 1);
    }

    /// The busy program indexes its table at 0040 with e002's low bit.
    #[test]
    fn tables_follow_the_index_byte() {
        let set = gp_arcade::testrom::busy();
        let (text, json) = tables(&set, &[InputFrame::default(); 3], 0xe002).unwrap();
        assert!(text.contains("read by $0025"), "{text}");
        let t = &json["tables"][0];
        assert_eq!(t["from"], 0x40);
        assert_eq!(t["entry_size"], 2);
    }
}
