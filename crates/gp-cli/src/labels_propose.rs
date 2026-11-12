//! `gp labels propose`: a labels file (docs/formats/labels.md) drafted from
//! a run of an arcade original, for a person or an agent to read, rename
//! and accept (port wave 6b). Nothing in it is the game's: names are made
//! from addresses (or the state mapping's paths), descriptions from counts.
//!
//! The run is the input log from power-on on gp-arcade, watched with the
//! levels and the access log on the main Z80:
//!
//! - `[[routine]]`: each routine of the code map that ran (a call's,
//!   restart's or interrupt's target, or a `jp (..)` target), named
//!   `sub_XXXX` or `irq_XXXX`, with how often it was entered, its callers
//!   and the RAM it wrote.
//! - `[[label]]`: work RAM (E000-E7FF) written in the run, adjacent bytes
//!   written by the same routines taken as one variable, named after the
//!   state mapping's field when one reads it (`entities.player[0].x` gives
//!   `player_x`), else `ram_XXXX`; with its writers, its readers and on how
//!   many frames it changed. A byte that went up by one on most frames is
//!   said to be a counter.
//! - `[[data]]`: ROM the program read as data, by reading instruction, as
//!   one table per reader's range, `data_XXXX`.
//!
//! A drafted file is a proposal: it goes to standard output or `--out`
//! (outside any git work tree), never over a game's labels.toml.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_arcade::levels::{EdgeKind, Watcher};
use gp_arcade::mcr::AccessKind;
use gp_arcade::{Controls, Romset, Tapper};
use gp_evidence::input_log::InputFrame;
use serde_json::{Value, json};

const RAM: std::ops::Range<u16> = 0xe000..0xe800;

/// A mapping's names for RAM bytes: `entities.player[0].x` as `player_x`.
pub(crate) fn mapping_names(text: &str) -> BTreeMap<u16, String> {
    let mut out = BTreeMap::new();
    let Ok(v) = text.parse::<toml::Table>() else {
        return out;
    };
    for f in v
        .get("field")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(path) = f.get("path").and_then(toml::Value::as_str) else {
            continue;
        };
        let name: String = path
            .trim_start_matches("entities.")
            .replace("[0]", "")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .split('_')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("_");
        for a in f
            .get("ram")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(a) = a.as_integer().and_then(|a| u16::try_from(a).ok()) {
                out.entry(a).or_insert_with(|| name.clone());
            }
        }
    }
    out
}

fn list(m: &BTreeMap<u16, u64>, max: usize) -> String {
    let mut v: Vec<(&u16, &u64)> = m.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut s: Vec<String> = v
        .iter()
        .take(max)
        .map(|(a, n)| format!("${a:04X} ({n})"))
        .collect();
    if v.len() > max {
        s.push(format!("{} more", v.len() - max));
    }
    s.join(", ")
}

/// The draft for `set` run through `inputs`.
pub(crate) fn propose(
    set: &Romset,
    inputs: &[InputFrame],
    names: &BTreeMap<u16, String>,
) -> Result<(String, Value)> {
    let mut t = Tapper::power_on(set);
    let mut w = Watcher::new(false);
    // By RAM address: writers and readers by PC; and each byte's frames
    // changed and frames up by one.
    let mut writers: BTreeMap<u16, BTreeMap<u16, u64>> = BTreeMap::new();
    let mut readers: BTreeMap<u16, BTreeMap<u16, u64>> = BTreeMap::new();
    let mut rom_reads: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    let mut changed = vec![0u64; RAM.len()];
    let mut up_one = vec![0u64; RAM.len()];
    let mut prev: Vec<u8> = (RAM).map(|a| t.peek(a)).collect();
    for f in inputs {
        let bits = f.buttons_word();
        if bits > 0xff || f.reset {
            bail!("an arcade input frame holds bits 0 to 7 and no reset; this one holds {bits:#x}");
        }
        let watched = w.step_quiet(&mut t, Controls::from_bits(bits));
        for a in watched.accesses.iter().filter(|a| a.cpu == 0) {
            match a.kind {
                AccessKind::Write if RAM.contains(&a.addr) => {
                    *writers.entry(a.addr).or_default().entry(a.pc).or_default() += 1;
                }
                AccessKind::Read if RAM.contains(&a.addr) => {
                    *readers.entry(a.addr).or_default().entry(a.pc).or_default() += 1;
                }
                AccessKind::Read if a.addr < 0xe000 => {
                    rom_reads.entry(a.pc).or_default().insert(a.addr);
                }
                _ => {}
            }
        }
        for (i, a) in RAM.enumerate() {
            let v = t.peek(a);
            if v != prev[i] {
                changed[i] += 1;
                up_one[i] += u64::from(v.wrapping_sub(prev[i]) == 1);
            }
            prev[i] = v;
        }
    }
    let frames = inputs.len() as u64;
    let levels = w.finish();
    let main = &levels.cpus[0];
    let map = crate::code_z80::CodeMap::of(set, 0, Some(main));
    let routine_of = |pc: u16| map.routines.range(..=pc).next_back().map(|(a, _)| *a);
    let rname = |at: u16| match map.routines.get(&at) {
        Some(&"interrupt handler") => format!("irq_{at:04x}"),
        Some(&"reset") => "reset".to_owned(),
        _ => format!("sub_{at:04x}"),
    };
    // Writes by routine, for both the RAM and the routines.
    let by_routine = |m: &BTreeMap<u16, u64>| -> BTreeMap<u16, u64> {
        let mut out = BTreeMap::new();
        for (pc, n) in m {
            if let Some(r) = routine_of(*pc) {
                *out.entry(r).or_default() += n;
            }
        }
        out
    };
    // RAM variables: runs of written bytes with the same writing routines.
    let mut vars: Vec<(u16, u16, BTreeSet<u16>)> = Vec::new();
    for (&a, m) in &writers {
        let rs: BTreeSet<u16> = by_routine(m).into_keys().collect();
        match vars.last_mut() {
            Some((lo, hi, set))
                if *hi + 1 == a
                    && *set == rs
                    && !names.contains_key(&a)
                    && !names.contains_key(lo) =>
            {
                *hi = a;
            }
            _ => vars.push((a, a, rs)),
        }
    }
    let mut text = format!(
        "# A labels draft made by `gp labels propose` from {frames} frames of {}: names\n\
         # from addresses and the state mapping, descriptions from counts. Read,\n\
         # rename and keep what holds (docs/formats/labels.md).\n\
         schema = 0\ngame = \"{}\"\n",
        set.name, set.name
    );
    let mut labels = Vec::new();
    for (lo, hi, rs) in &vars {
        let size = hi - lo + 1;
        let i = usize::from(lo - RAM.start);
        let name = names
            .get(lo)
            .cloned()
            .unwrap_or_else(|| format!("ram_{lo:04x}"));
        let writes: BTreeMap<u16, u64> =
            (*lo..=*hi)
                .filter_map(|a| writers.get(&a))
                .fold(BTreeMap::new(), |mut acc, m| {
                    for (r, n) in by_routine(m) {
                        *acc.entry(r).or_default() += n;
                    }
                    acc
                });
        let reads: BTreeMap<u16, u64> =
            (*lo..=*hi)
                .filter_map(|a| readers.get(&a))
                .fold(BTreeMap::new(), |mut acc, m| {
                    for (r, n) in by_routine(m) {
                        *acc.entry(r).or_default() += n;
                    }
                    acc
                });
        let counter = size == 1 && up_one[i] * 2 > frames;
        let description = format!(
            "{}written by {}; read by {}; changed on {} of {frames} frames",
            if counter {
                "a counter, up by one on most frames; "
            } else {
                ""
            },
            list(&writes, 3),
            if reads.is_empty() {
                "nothing in the run".to_owned()
            } else {
                list(&reads, 3)
            },
            changed[i],
        );
        let _ = write!(
            text,
            "\n[[label]]\nname = \"{name}\"\naddr = 0x{lo:04X}\nsize = {size}\ndescription = \"{description}\"\n"
        );
        labels.push(json!({"name": name, "addr": lo, "size": size, "writers": rs}));
    }
    let mut routines = Vec::new();
    let starts: Vec<u16> = map.routines.keys().copied().collect();
    for (k, &at) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().map_or(0xe000, u32::from);
        if !main
            .starts
            .range(at..)
            .next()
            .is_some_and(|(pc, _)| u32::from(*pc) < end)
        {
            continue;
        }
        let mut callers: BTreeMap<u16, u64> = BTreeMap::new();
        for ((from, to, kind), s) in &main.edges {
            if *to == at && matches!(kind, EdgeKind::Call | EdgeKind::Rst | EdgeKind::Indirect) {
                *callers.entry(*from).or_default() += s.count;
            }
        }
        let entered: u64 = main
            .edges
            .iter()
            .filter(|((_, to, k), _)| {
                *to == at
                    && matches!(
                        k,
                        EdgeKind::Call | EdgeKind::Rst | EdgeKind::Interrupt | EdgeKind::Indirect
                    )
            })
            .map(|(_, s)| s.count)
            .sum();
        let wrote: BTreeSet<String> = vars
            .iter()
            .filter(|(_, _, rs)| rs.contains(&at))
            .map(|(lo, _, _)| {
                names
                    .get(lo)
                    .cloned()
                    .unwrap_or_else(|| format!("ram_{lo:04x}"))
            })
            .collect();
        let name = rname(at);
        let description = format!(
            "{}; entered {entered} times{}{}",
            map.routines.get(&at).copied().unwrap_or("entry"),
            if callers.is_empty() {
                String::new()
            } else {
                format!(", from {}", list(&callers, 3))
            },
            if wrote.is_empty() {
                String::new()
            } else {
                let v: Vec<String> = wrote.into_iter().collect();
                let more = v.len().saturating_sub(6);
                format!(
                    "; writes {}{}",
                    v[..v.len().min(6)].join(", "),
                    if more > 0 {
                        format!(" and {more} more")
                    } else {
                        String::new()
                    }
                )
            }
        );
        let _ = write!(
            text,
            "\n[[routine]]\nname = \"{name}\"\nbank = 0\naddr = 0x{at:04X}\nprg = {at}\nsize = {}\ndescription = \"{description}\"\n",
            end - u32::from(at)
        );
        routines.push(json!({"name": name, "addr": at, "entered": entered}));
    }
    // ROM tables: each reader's range, overlapping ranges taken as one.
    let mut ranges: Vec<(u16, u16, BTreeSet<u16>, usize)> = rom_reads
        .iter()
        .map(|(pc, addrs)| {
            let (lo, hi) = (*addrs.first().expect("read"), *addrs.last().expect("read"));
            (lo, hi, BTreeSet::from([*pc]), addrs.len())
        })
        .collect();
    ranges.sort_by_key(|r| (r.0, r.1));
    let mut tables: Vec<(u16, u16, BTreeSet<u16>, usize)> = Vec::new();
    // A reader that read more than a few KB (a ROM check, a copy loop)
    // stands alone, so it doesn't join every table it passes over.
    let (wide, ranges): (Vec<_>, Vec<_>) = ranges.into_iter().partition(|r| r.3 > 1024);
    tables.extend(wide);
    for r in ranges {
        match tables.last_mut().filter(|t| t.3 <= 1024) {
            Some(t) if r.0 <= t.1 => {
                t.1 = t.1.max(r.1);
                t.2.extend(r.2);
                t.3 += r.3;
            }
            _ => tables.push(r),
        }
    }
    let mut data = Vec::new();
    for (lo, hi, pcs, n) in &tables {
        let name = format!("data_{lo:04x}");
        let by: BTreeSet<String> = pcs
            .iter()
            .filter_map(|pc| routine_of(*pc))
            .map(rname)
            .collect();
        let by: Vec<String> = by.into_iter().collect();
        let _ = write!(
            text,
            "\n[[data]]\nname = \"{name}\"\nbank = 0\naddr = 0x{lo:04X}\nprg = {lo}\nsize = {}\ndescription = \"read by {} instructions in {}\"\n",
            u32::from(*hi) - u32::from(*lo) + 1,
            pcs.len(),
            if by.len() > 4 {
                format!("{} and {} more", by[..4].join(", "), by.len() - 4)
            } else {
                by.join(", ")
            },
        );
        let _ = n;
        data.push(json!({"name": name, "addr": lo, "readers": pcs}));
    }
    // It reads back as a labels file.
    crate::labels::parse(&text).context("the draft doesn't read as a labels file")?;
    Ok((
        text,
        json!({"frames": frames, "labels": labels, "routines": routines, "data": data}),
    ))
}

/// `gp labels propose`.
pub(crate) fn run(
    rom: &Path,
    input: &Path,
    mapping: Option<&Path>,
    out: Option<&Path>,
) -> Result<crate::Outcome> {
    let bytes = std::fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    if !gp_arcade::romset::looks_like_zip(&bytes) {
        bail!(
            "gp labels propose drafts from gp-arcade's access log, so it takes an arcade romset; an NES game's labels come from its disassembly"
        );
    }
    let set = Romset::from_zip(&bytes)?;
    let log = gp_evidence::InputLog::parse(
        &std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?,
    )?;
    let names = match mapping {
        Some(m) => mapping_names(
            &std::fs::read_to_string(m).with_context(|| format!("reading {}", m.display()))?,
        ),
        None => BTreeMap::new(),
    };
    let (text, json) = propose(&set, &log.frames, &names)?;
    let summary = format!(
        "{} RAM labels, {} routines and {} data tables from {} frames",
        json["labels"].as_array().map_or(0, Vec::len),
        json["routines"].as_array().map_or(0, Vec::len),
        json["data"].as_array().map_or(0, Vec::len),
        json["frames"]
    );
    match out {
        Some(p) => {
            if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(p) {
                bail!(
                    "--out {}: inside the git work tree {}; a draft goes outside a repo until a person accepts it",
                    p.display(),
                    repo.display()
                );
            }
            std::fs::write(p, &text).with_context(|| format!("writing {}", p.display()))?;
            Ok(crate::Outcome::ok(
                json,
                format!("{summary}\nwritten to {}", p.display()),
            ))
        }
        None => Ok(crate::Outcome::ok(json, text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The busy program's draft names the frame counter at e001 a counter
    /// written by the reset routine, its table jump targets as routines,
    /// its table at 0040 as data, and it reads back as a labels file.
    #[test]
    fn drafts_the_busy_programs_labels() {
        let set = gp_arcade::testrom::busy();
        let names = mapping_names("[[field]]\npath = \"entities.player[0].x\"\nram = [0xE000]\n");
        let (text, json) = propose(&set, &[InputFrame::default(); 6], &names).unwrap();
        let file = crate::labels::parse(&text).unwrap();
        assert!(
            file.label
                .iter()
                .any(|l| l.addr == 0xe000 && l.name == "player_x"),
            "{text}"
        );
        let e001 = file.label.iter().find(|l| l.addr == 0xe001).unwrap();
        assert_eq!(e001.name, "ram_e001");
        assert!(
            e001.description.contains("written by $0000"),
            "{}",
            e001.description
        );
        assert!(file.routine.iter().any(|r| r.addr == 0x0050), "{text}");
        assert!(file.data.iter().any(|d| d.addr == 0x0040), "{text}");
        assert!(json["frames"] == 6);
    }
}
