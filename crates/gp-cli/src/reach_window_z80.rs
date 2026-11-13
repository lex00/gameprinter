//! `gp reach --window` and `--diff-*` on an arcade romset: what both Z80s
//! did over a window of frames or a set of clips, by routine of the code
//! map (crate::code_z80::CodeMap), and what one side ran that the other
//! didn't, in reach_window.rs's report.
//!
//! What maps from the NES: a side's clips run from power-on on gp-arcade
//! with its levels watching the window's frames (crate::code_z80::run_clips;
//! no store of counted windows, so every window is counted now); one code
//! map per CPU, from both sides' runs, names the routines both sides are
//! read by; a routine is named `main:5FAC` or `sound:0276` and its element
//! column says why it is one (called, restart, interrupt handler, jp (..)
//! target, reset). A conditional's ways are its edges: a jump, call, return
//! or `djnz` loop taken, or a fall.
//!
//! `--diff-reach` and `--cites` stay the NES's: an arcade `gp reach
//! --write` holds counts by chip, not instruction starts, and the
//! templates' cite index reads addresses from $8000 up (the NES's program),
//! where Tapper's code is 0000-DFFF.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Result, bail};
use gp_arcade::Romset;
use gp_arcade::levels::{EdgeKind, Levels};
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::code::{Window, parse_window, select};
use crate::code_z80::{CPUS, CodeMap};
use crate::reach::ReachArgs;

/// An instruction: its CPU (0 main, 1 sound) and address.
type At = (usize, u16);

/// What an instruction did on a side.
#[derive(Default, Clone, Copy)]
struct Stat {
    taken: u64,
    fell: u64,
}

struct Side {
    label: String,
    frames: u64,
    ran: BTreeMap<At, Stat>,
}

fn name(at: At) -> String {
    format!("{}:{:04X}", CPUS[at.0], at.1)
}

/// What the levels say each instruction did.
fn ran_of(levels: &Levels) -> BTreeMap<At, Stat> {
    let mut out = BTreeMap::new();
    for (c, lv) in levels.cpus.iter().enumerate() {
        for pc in lv.starts.keys() {
            out.insert((c, *pc), Stat::default());
        }
        for (from, _, kind) in lv.edges.keys() {
            let s = out.entry((c, *from)).or_insert_with(Stat::default);
            match kind {
                EdgeKind::Fall => s.fell += 1,
                EdgeKind::Jump | EdgeKind::Loop | EdgeKind::Call | EdgeKind::Return => {
                    s.taken += 1;
                }
                _ => {}
            }
        }
    }
    out
}

/// A side of clips: the sets, the clips picked and the window.
fn clip_side(
    set: &Romset,
    sets: &[PathBuf],
    only: &[String],
    w: Window,
    a: &ReachArgs,
    paths: &Paths,
) -> Result<(Side, Levels)> {
    let t0 = Instant::now();
    let clips = if sets.is_empty() {
        a.starts.with_platform(&[], paths, true)?
    } else {
        crate::reach::clip_inputs(sets, paths)?
    };
    let clips = select(clips, only)?;
    let levels = crate::code_z80::run_clips(set, &clips, (w.first, w.last))?;
    let names: Vec<&str> = clips.iter().map(|c| c.id.as_str()).collect();
    let shown = if names.len() <= 4 {
        names.join(", ")
    } else {
        format!("{}, ... ({} in all)", names[..3].join(", "), names.len())
    };
    let label = format!(
        "{} clips ({shown}), {}, {} frames ({} counted now, 0 from the store, {:.1} s)",
        clips.len(),
        w.text(),
        levels.watched,
        clips.len(),
        t0.elapsed().as_secs_f64()
    );
    Ok((
        Side {
            label,
            frames: levels.watched,
            ran: ran_of(&levels),
        },
        levels,
    ))
}

/// Both CPUs' code maps, from every side's runs.
struct Maps {
    maps: Vec<CodeMap>,
}

impl Maps {
    fn of(set: &Romset, sides: &[&Levels]) -> Self {
        let mut merged = Levels::default();
        for l in sides {
            for (into, from) in merged.cpus.iter_mut().zip(&l.cpus) {
                for (k, v) in &from.starts {
                    into.starts.entry(*k).or_insert(*v);
                }
                for (k, v) in &from.edges {
                    into.edges.entry(*k).or_insert(*v);
                }
            }
        }
        Self {
            maps: (0..2)
                .map(|c| CodeMap::of(set, c, Some(&merged.cpus[c])))
                .collect(),
        }
    }

    /// Every routine: its CPU, its address and why it is one.
    fn routines(&self) -> Vec<(usize, u16, &'static str)> {
        self.maps
            .iter()
            .enumerate()
            .flat_map(|(c, m)| m.routines.iter().map(move |(a, why)| (c, *a, *why)))
            .collect()
    }

    /// A routine's instructions, up to the next routine.
    fn body(&self, c: usize, at: u16) -> Vec<(At, bool)> {
        let m = &self.maps[c];
        let end = m
            .routines
            .range(at.saturating_add(1)..)
            .next()
            .map_or(0x1_0000, |(a, _)| u32::from(*a));
        m.insns
            .range(at..)
            .take_while(|(pc, _)| u32::from(**pc) < end)
            .map(|(pc, i)| ((c, *pc), i.flow.conditional()))
            .collect()
    }

    fn mapped(&self, at: At) -> bool {
        self.maps[at.0].insns.contains_key(&at.1)
    }

    fn next(&self, at: At) -> Option<At> {
        let i = self.maps[at.0].insns.get(&at.1)?;
        Some((at.0, at.1.wrapping_add(u16::from(i.len))))
    }
}

/// Addresses as ranges of consecutive instructions: `main:5FAC-5FC0`.
fn spans(maps: &Maps, list: &[At]) -> String {
    let mut out: Vec<(At, At)> = Vec::new();
    for &p in list {
        match out.last_mut() {
            Some((_, end)) if maps.next(*end) == Some(p) => *end = p,
            _ => out.push((p, p)),
        }
    }
    out.iter()
        .map(|(a, b)| {
            if a == b {
                name(*a)
            } else {
                format!("{}-{:04X}", name(*a), b.1)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Default)]
struct Row {
    run: usize,
    insns: usize,
    both: usize,
    one_way: Vec<String>,
    not_run: usize,
}

type Rows = BTreeMap<(usize, u16), (Row, &'static str)>;

fn rows(maps: &Maps, ran: &BTreeMap<At, Stat>) -> (Rows, Vec<At>) {
    let mut out = Rows::new();
    for (c, at, why) in maps.routines() {
        let body = maps.body(c, at);
        if !body.iter().any(|(p, _)| ran.contains_key(p)) {
            continue;
        }
        let mut r = Row::default();
        for (p, branch) in &body {
            r.insns += 1;
            let s = ran.get(p);
            r.run += usize::from(s.is_some());
            if *branch {
                match s {
                    Some(s) if s.taken > 0 && s.fell > 0 => r.both += 1,
                    Some(s) => r.one_way.push(format!(
                        "{} {}",
                        name(*p),
                        if s.taken > 0 { "taken" } else { "fell" }
                    )),
                    None => r.not_run += 1,
                }
            }
        }
        out.insert((c, at), (r, why));
    }
    let outside = ran.keys().filter(|p| !maps.mapped(**p)).copied().collect();
    (out, outside)
}

fn summary(s: &Side, rows: &Rows, outside: &[At]) -> Value {
    let both: usize = rows.values().map(|r| r.0.both).sum();
    let one: usize = rows.values().map(|r| r.0.one_way.len()).sum();
    let none: usize = rows.values().map(|r| r.0.not_run).sum();
    json!({
        "label": s.label,
        "counted": true,
        "frames": s.frames,
        "routines_run": rows.len(),
        "instructions_run": s.ran.len(),
        "branches": {"both": both, "one_way": one, "not_run": none},
        "outside_code_map": outside.len(),
    })
}

fn summary_line(v: &Value) -> String {
    let mut s = format!(
        "{} routines ran, {} instructions; branches in them: both ways {}, one way {}, not run {}",
        v["routines_run"],
        v["instructions_run"],
        v["branches"]["both"],
        v["branches"]["one_way"],
        v["branches"]["not_run"],
    );
    if v["outside_code_map"].as_u64().unwrap_or(0) > 0 {
        let _ = write!(
            s,
            "; {} instructions outside the code map",
            v["outside_code_map"]
        );
    }
    s
}

fn report(maps: &Maps, s: &Side) -> (String, Value) {
    let (rows, outside) = rows(maps, &s.ran);
    let sum = summary(s, &rows, &outside);
    let mut text = format!("{}\n{}\n", s.label, summary_line(&sum));
    let _ = writeln!(
        text,
        "\n{:<12} {:<18} {:>9} {:>5} {:>7}  branches one way",
        "routine", "element", "run/insns", "both", "not run",
    );
    let mut list = Vec::new();
    for ((c, at), (r, why)) in &rows {
        let n = name((*c, *at));
        let _ = writeln!(
            text,
            "{:<12} {:<18} {:>9} {:>5} {:>7}  {}",
            n,
            why,
            format!("{}/{}", r.run, r.insns),
            r.both,
            r.not_run,
            r.one_way.join(", ")
        );
        list.push(json!({
            "routine": n, "element": why, "run": r.run, "instructions": r.insns,
            "both": r.both, "one_way": r.one_way, "not_run": r.not_run, "cited": 0,
        }));
    }
    if !outside.is_empty() {
        let _ = writeln!(text, "\noutside the code map: {}", spans(maps, &outside));
    }
    (
        text.trim_end().to_owned(),
        json!({"a": sum, "routines": list}),
    )
}

fn diff_report(maps: &Maps, a: &Side, b: &Side) -> (String, Value) {
    let (ra, oa) = rows(maps, &a.ran);
    let (rb, ob) = rows(maps, &b.ran);
    let sa = summary(a, &ra, &oa);
    let sb = summary(b, &rb, &ob);
    let mut text = format!(
        "A: {}\n   {}\nB: {}\n   {}\n",
        a.label,
        summary_line(&sa),
        b.label,
        summary_line(&sb)
    );
    let mut doc = json!({"a": sa, "b": sb});
    for (key, x, y, rx, ry) in [("a_not_b", a, b, &ra, &rb), ("b_not_a", b, a, &rb, &ra)] {
        let (xs, ys) = if key == "a_not_b" {
            ("A", "B")
        } else {
            ("B", "A")
        };
        let only: Vec<(usize, u16)> = rx.keys().filter(|i| !ry.contains_key(i)).copied().collect();
        let mut paths: Vec<((usize, u16), Vec<At>)> = Vec::new();
        let mut ways: Vec<String> = Vec::new();
        let mut same = 0;
        for &(c, at) in rx.keys().filter(|i| ry.contains_key(i)) {
            let body = maps.body(c, at);
            let new: Vec<At> = body
                .iter()
                .map(|(p, _)| *p)
                .filter(|p| x.ran.contains_key(p) && !y.ran.contains_key(p))
                .collect();
            for (p, branch) in &body {
                if !branch {
                    continue;
                }
                let (Some(sx), Some(sy)) = (x.ran.get(p), y.ran.get(p)) else {
                    continue;
                };
                if sx.taken > 0 && sy.taken == 0 {
                    ways.push(format!("{} taken ({})", name(*p), name((c, at))));
                }
                if sx.fell > 0 && sy.fell == 0 {
                    ways.push(format!("{} fell ({})", name(*p), name((c, at))));
                }
            }
            if new.is_empty() {
                same += 1;
            } else {
                paths.push(((c, at), new));
            }
        }
        let n_new: usize = paths.iter().map(|(_, v)| v.len()).sum();
        let _ = writeln!(
            text,
            "\n{xs} ran {} routines: {} that {ys} also ran with no instruction {ys} didn't run, {} with instructions only {xs} ran ({n_new} instructions), {} that {ys} didn't run at all",
            rx.len(),
            same,
            paths.len(),
            only.len()
        );
        if !only.is_empty() {
            let _ = writeln!(text, "  routines only {xs} ran:");
            for i in &only {
                let (r, why) = &rx[i];
                let _ = writeln!(
                    text,
                    "    {} ({why}) {}/{} instructions",
                    name(*i),
                    r.run,
                    r.insns
                );
            }
        }
        if !paths.is_empty() {
            let _ = writeln!(text, "  instructions only {xs} ran, in routines both ran:");
            for (i, v) in &paths {
                let _ = writeln!(
                    text,
                    "    {} ({}): {} instructions, {}",
                    name(*i),
                    rx[i].1,
                    v.len(),
                    spans(maps, v)
                );
            }
        }
        if !ways.is_empty() {
            let _ = writeln!(
                text,
                "  branch ways {xs} took that {ys} didn't, in instructions both ran: {}",
                ways.len()
            );
            for w in &ways {
                let _ = writeln!(text, "    {w}");
            }
        }
        let outside_new: Vec<At> = x
            .ran
            .keys()
            .filter(|p| !y.ran.contains_key(p) && !maps.mapped(**p))
            .copied()
            .collect();
        if !outside_new.is_empty() {
            let _ = writeln!(
                text,
                "  outside the code map, only {xs}: {}",
                spans(maps, &outside_new)
            );
        }
        doc[key] = json!({
            "routines_only": only.iter().map(|i| name(*i)).collect::<Vec<_>>(),
            "routines_same": same,
            "routines_with_new_paths": paths.iter().map(|(i, v)| json!({
                "routine": name(*i),
                "instructions": v.len(),
                "at": spans(maps, v),
            })).collect::<Vec<_>>(),
            "new_instructions": n_new,
            "branch_ways": ways,
            "outside_code_map": outside_new.len(),
        });
    }
    (text.trim_end().to_owned(), doc)
}

/// `gp reach --window`/`--diff-*` on a romset's bytes.
pub(crate) fn run(rom: &[u8], a: &ReachArgs, paths: &Paths) -> Result<Outcome> {
    if a.diff_reach.is_some() {
        bail!(
            "--diff-reach reads an NES gp reach's instruction starts; an arcade gp reach --write holds counts by chip, so diff two sets of clips with --diff-clip or --diff-window"
        );
    }
    if !a.cites.is_empty() {
        bail!(
            "--cites indexes the templates' addresses from $8000 up, the NES's program; an arcade CPU's code (Tapper's at 0000-DFFF) isn't in that index"
        );
    }
    let set = Romset::from_zip(rom).map_err(|e| anyhow::anyhow!("{e}"))?;
    let w_a = match &a.window {
        Some(s) => parse_window(s)?,
        None => Window::WHOLE,
    };
    let (side_a, lv_a) = clip_side(&set, &a.clips, &a.only, w_a, a, paths)?;
    let diff = !a.diff_clips.is_empty() || !a.diff_only.is_empty() || a.diff_window.is_some();
    let b = if diff {
        let own = !a.diff_clips.is_empty();
        let sets = if own { &a.diff_clips } else { &a.clips };
        let only = if own || !a.diff_only.is_empty() {
            &a.diff_only
        } else {
            &a.only
        };
        let w = match &a.diff_window {
            Some(s) => parse_window(s)?,
            None if own => Window::WHOLE,
            None => w_a,
        };
        Some(clip_side(&set, sets, only, w, a, paths)?)
    } else {
        None
    };
    let all: Vec<&Levels> = std::iter::once(&lv_a)
        .chain(b.as_ref().map(|(_, l)| l))
        .collect();
    let maps = Maps::of(&set, &all);
    let (text, doc) = match &b {
        None => report(&maps, &side_a),
        Some((side_b, _)) => diff_report(&maps, &side_a, side_b),
    };
    Ok(Outcome::ok(doc, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reach::ClipInput;

    /// With the rbtapper romset: an idle run's later frames, by routine,
    /// hold the interrupt handler, and against its first frames the diff
    /// names what only one side ran. Skipped without it.
    #[test]
    fn a_window_reads_by_routine_of_both_cpus() {
        let Some(zip) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let set = Romset::from_zip(&std::fs::read(zip).unwrap()).unwrap();
        let clip = ClipInput {
            set: "t".into(),
            id: "idle".into(),
            power_on: Default::default(),
            inputs: vec![Default::default(); 240],
        };
        let late =
            crate::code_z80::run_clips(&set, std::slice::from_ref(&clip), (200, 240)).unwrap();
        let early = crate::code_z80::run_clips(&set, &[clip], (1, 20)).unwrap();
        assert_eq!(late.watched, 41);
        let side = |l: &Levels| Side {
            label: String::new(),
            frames: l.watched,
            ran: ran_of(l),
        };
        let maps = Maps::of(&set, &[&late, &early]);
        let (rows, outside) = rows(&maps, &side(&late).ran);
        assert!(outside.is_empty());
        assert!(rows.values().any(|(_, why)| *why == "interrupt handler"));
        assert!(
            rows.keys().any(|(c, _)| *c == 1),
            "the sound CPU's routines"
        );
        let (text, doc) = diff_report(&maps, &side(&late), &side(&early));
        assert!(text.starts_with("A: "), "{text}");
        assert!(
            !doc["a_not_b"]["routines_only"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
