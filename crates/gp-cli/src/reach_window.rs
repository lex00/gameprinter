//! `gp reach --window` and `--diff-*`: what the code did over a window of
//! frames or a set of clips, by routine of the code map, and what one side
//! ran that the other didn't (docs/code-reading.md).
//!
//! A side is clips (the sets given, `--only` picking some) and a window of
//! each, counted instruction by instruction (`gp_events::profile`), or a
//! `gp reach --write` output (`--diff-reach`), whose instruction starts and
//! branch edges say what ran without how often. Both sides are named by
//! one code map, the static harvest's from the ROM alone, so routines
//! compare as names. On an arcade romset the same report reads both Z80s'
//! code maps (crate::reach_window_z80).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_events::profile::{InsnStat, Profile};
use gp_evidence::Paths;
use gp_nes::codemap::{Mode, Va, va_cpu, va_hex};
use serde_json::{Value, json};

use crate::Outcome;
use crate::code::{CodeMap, Window, count, event_store, parse_window, select};
use crate::reach::ReachArgs;

/// One side: what ran, by banked address, and whether instructions were
/// counted (a reach output's side knows only that they ran).
struct Side {
    label: String,
    ran: BTreeMap<Va, InsnStat>,
    counted: bool,
    profile: Option<Profile>,
}

/// Whether the arguments ask for this report.
pub(crate) fn wanted(a: &ReachArgs) -> bool {
    a.window.is_some()
        || !a.diff_clips.is_empty()
        || !a.diff_only.is_empty()
        || a.diff_window.is_some()
        || a.diff_reach.is_some()
        || !a.cites.is_empty()
}

/// A side of clips: the sets, the clips picked and the window.
fn clip_side(
    rom: &[u8],
    sets: &[PathBuf],
    only: &[String],
    window: Window,
    a: &ReachArgs,
    paths: &Paths,
) -> Result<(String, Profile)> {
    // Stage 0's starts only when the side names no clip set.
    let clips = if sets.is_empty() {
        a.starts.with(&[], paths)?
    } else {
        crate::reach::clip_inputs(sets, paths)?
    };
    let clips = select(clips, only)?;
    let store = event_store(&a.store, a.no_store, paths);
    let c = count(&paths.home, rom, &clips, window, store.as_ref())?;
    let names: Vec<&str> = c.clips.iter().map(|(id, _)| id.as_str()).collect();
    let shown = if names.len() <= 4 {
        names.join(", ")
    } else {
        format!("{}, ... ({} in all)", names[..3].join(", "), names.len())
    };
    let label = format!(
        "{} clips ({shown}), {}, {} frames ({} counted now, {} from the store, {:.1} s)",
        c.clips.len(),
        window.text(),
        c.profile.frames,
        c.ran,
        c.clips.len() - c.ran,
        c.seconds
    );
    Ok((label, c.profile))
}

/// A side from a `gp reach --write` output: the instruction starts it
/// ran, and its branches' taken and fall edges.
fn reach_side(path: &Path, rom: &[u8], map: &CodeMap) -> Result<Side> {
    let v: Value = serde_json::from_str(
        &fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("{} isn't a gp reach output", path.display()))?;
    let sha = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(rom));
    if v["rom_sha256"].as_str() != Some(sha.as_str()) {
        bail!("{} is a reach of another ROM", path.display());
    }
    let mut ran: BTreeMap<Va, InsnStat> = BTreeMap::new();
    let ranges = v["footprint"]["starts"]["ranges"]
        .as_array()
        .context("no footprint.starts.ranges")?;
    for r in ranges {
        let (a, b) = (r[0].as_u64().unwrap_or(0), r[1].as_u64().unwrap_or(0));
        for o in a..b {
            if let Some(va) = va_of_offset(map, o as u32) {
                ran.insert(
                    va,
                    InsnStat {
                        cpu: va_cpu(va),
                        ..InsnStat::default()
                    },
                );
            }
        }
    }
    for e in v["edges"]["list"].as_array().into_iter().flatten() {
        let kind = e["kind"].as_str().unwrap_or_default();
        if kind != "branch" && kind != "fall" {
            continue;
        }
        let Some(from) = e["from"].as_str().and_then(gp_nes::codemap::va_parse) else {
            continue;
        };
        let n = e["count"].as_u64().unwrap_or(0);
        let s = ran.entry(from).or_insert_with(|| InsnStat {
            cpu: va_cpu(from),
            ..InsnStat::default()
        });
        if kind == "branch" {
            s.taken += n;
        } else {
            s.fell += n;
        }
    }
    let clips = v["clips"].as_array().map_or(0, Vec::len);
    let frames: u64 = v["clips"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["frames"].as_u64())
        .sum();
    Ok(Side {
        label: format!(
            "{} (a gp reach of {clips} clips, {frames} frames, whole clips)",
            path.display()
        ),
        ran,
        counted: false,
        profile: None,
    })
}

/// The banked address of a PRG offset: the highest CPU address the board
/// can show it at (code at $C000 on a 16 KB board mirrored at $8000).
fn va_of_offset(map: &CodeMap, o: u32) -> Option<Va> {
    (0..4u32).rev().find_map(|k| {
        let cpu = 0x8000 + (o % 0x2000) + k * 0x2000;
        map.namer.va(o, cpu as u16)
    })
}

/// Per routine of the map: the instructions run, and the branches by the
/// ways they went.
#[derive(Default)]
struct Row {
    run: usize,
    insns: usize,
    both: usize,
    one_way: Vec<String>,
    not_run: usize,
}

fn rows(map: &CodeMap, ran: &BTreeMap<Va, InsnStat>) -> (BTreeMap<usize, Row>, Vec<Va>) {
    let mut out: BTreeMap<usize, Row> = BTreeMap::new();
    let mut in_map = BTreeSet::new();
    for i in 0..map.routines.len() {
        let body = map.body(i);
        in_map.extend(body.iter().map(|(p, _)| *p));
        if !body.iter().any(|(p, _)| ran.contains_key(p)) {
            continue;
        }
        let mut r = Row::default();
        for (p, ins) in &body {
            let Some(ins) = ins else { continue };
            r.insns += 1;
            let s = ran.get(p);
            r.run += usize::from(s.is_some());
            if ins.mode == Mode::Rel {
                match s {
                    Some(s) if s.taken > 0 && s.fell > 0 => r.both += 1,
                    Some(s) => r.one_way.push(format!(
                        "{} {}",
                        va_hex(*p),
                        if s.taken > 0 { "taken" } else { "fell" }
                    )),
                    None => r.not_run += 1,
                }
            }
        }
        out.insert(i, r);
    }
    let outside = ran
        .keys()
        .filter(|p| !in_map.contains(p))
        .copied()
        .collect();
    (out, outside)
}

/// Addresses as ranges of consecutive instructions: `EE84-EEB4`.
fn spans(map: &CodeMap, list: &[Va]) -> String {
    let mut out: Vec<(Va, Va)> = Vec::new();
    for &p in list {
        match out.last_mut() {
            Some((_, end)) if map.insn_at(*end).is_some_and(|i| i.next() == p) => *end = p,
            _ => out.push((p, p)),
        }
    }
    out.iter()
        .map(|(a, b)| {
            if a == b {
                va_hex(*a)
            } else {
                format!("{}-{}", va_hex(*a), va_hex(*b))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn run(a: &ReachArgs, paths: &Paths) -> Result<Outcome> {
    let rom = fs::read(&a.rom).with_context(|| format!("reading {}", a.rom.display()))?;
    if crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade {
        return crate::reach_window_z80::run(&rom, a, paths);
    }
    let w_a = match &a.window {
        Some(s) => parse_window(s)?,
        None => Window::WHOLE,
    };
    let (label_a, prof_a) = clip_side(&rom, &a.clips, &a.only, w_a, a, paths)?;
    let diff = !a.diff_clips.is_empty()
        || !a.diff_only.is_empty()
        || a.diff_window.is_some()
        || a.diff_reach.is_some();
    // The other side: its own sets, or A's with its own pick and window.
    let prof_b = if diff && a.diff_reach.is_none() {
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
        Some(clip_side(&rom, sets, only, w, a, paths)?)
    } else {
        None
    };
    let profs: Vec<&Profile> = std::iter::once(&prof_a)
        .chain(prof_b.as_ref().map(|p| &p.1))
        .collect();
    let map = CodeMap::of(&rom, &profs)?;
    let side_a = Side {
        label: label_a,
        ran: map.by_address(&prof_a),
        counted: true,
        profile: Some(prof_a.clone()),
    };
    let side_b = match (&a.diff_reach, prof_b) {
        (Some(p), _) => Some(reach_side(p, &rom, &map)?),
        (None, Some((label, p))) => Some(Side {
            label,
            ran: map.by_address(&p),
            counted: true,
            profile: Some(p),
        }),
        (None, None) => None,
    };
    let cites = if a.cites.is_empty() {
        None
    } else {
        let (idx, _) = crate::template_cites::index(&a.cites)?;
        Some(crate::template_cites::by_routine(&idx, &map).0)
    };
    let (text, doc) = match &side_b {
        None => report(&map, &side_a, cites.as_ref()),
        Some(b) => diff_report(&map, &side_a, b, cites.as_ref()),
    };
    Ok(Outcome::ok(doc, text))
}

/// The routines that ran on one side, a line each.
fn report(map: &CodeMap, s: &Side, cites: Option<&BTreeMap<usize, Vec<u16>>>) -> (String, Value) {
    let (rows, outside) = rows(map, &s.ran);
    let summary = side_summary(s, &rows, &outside, cites);
    let mut text = format!("{}\n{}\n", s.label, summary_line(&summary));
    let _ = writeln!(
        text,
        "\n{:<12} {:<10} {:>9} {:>5} {:>7}{}  branches one way",
        "routine",
        "element",
        "run/insns",
        "both",
        "not run",
        if cites.is_some() { "  cited" } else { "" }
    );
    let mut list = Vec::new();
    for (i, r) in &rows {
        let rt = &map.routines[*i];
        let c = cites.and_then(|c| c.get(i)).map_or(0, Vec::len);
        let _ = writeln!(
            text,
            "{:<12} {:<10} {:>9} {:>5} {:>7}{}  {}",
            rt.name,
            rt.element,
            format!("{}/{}", r.run, r.insns),
            r.both,
            r.not_run,
            if cites.is_some() {
                format!("  {c:>5}")
            } else {
                String::new()
            },
            r.one_way.join(", ")
        );
        list.push(json!({
            "routine": rt.name, "element": rt.element, "run": r.run, "instructions": r.insns,
            "both": r.both, "one_way": r.one_way, "not_run": r.not_run, "cited": c,
        }));
    }
    if !outside.is_empty() {
        let _ = writeln!(text, "\noutside the code map: {}", spans(map, &outside));
    }
    (
        text.trim_end().to_owned(),
        json!({"a": summary, "routines": list}),
    )
}

fn side_summary(
    s: &Side,
    rows: &BTreeMap<usize, Row>,
    outside: &[Va],
    cites: Option<&BTreeMap<usize, Vec<u16>>>,
) -> Value {
    let both: usize = rows.values().map(|r| r.both).sum();
    let one: usize = rows.values().map(|r| r.one_way.len()).sum();
    let none: usize = rows.values().map(|r| r.not_run).sum();
    let mut v = json!({
        "label": s.label,
        "counted": s.counted,
        "frames": s.profile.as_ref().map(|p| p.frames),
        "routines_run": rows.len(),
        "instructions_run": s.ran.len(),
        "branches": {"both": both, "one_way": one, "not_run": none},
        "outside_code_map": outside.len(),
    });
    if let Some(c) = cites {
        v["routines_cited"] = json!(rows.keys().filter(|i| c.contains_key(i)).count());
    }
    v
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
    if let Some(c) = v.get("routines_cited") {
        let _ = write!(
            s,
            "; {c} of the {} hold an address the templates cite",
            v["routines_run"]
        );
    }
    if v["outside_code_map"].as_u64().unwrap_or(0) > 0 {
        let _ = write!(
            s,
            "; {} instructions outside the code map",
            v["outside_code_map"]
        );
    }
    s
}

/// What A ran that B didn't, and the reverse, by routine.
fn diff_report(
    map: &CodeMap,
    a: &Side,
    b: &Side,
    cites: Option<&BTreeMap<usize, Vec<u16>>>,
) -> (String, Value) {
    let (ra, oa) = rows(map, &a.ran);
    let (rb, ob) = rows(map, &b.ran);
    let sa = side_summary(a, &ra, &oa, cites);
    let sb = side_summary(b, &rb, &ob, cites);
    let mut text = format!(
        "A: {}\n   {}\nB: {}\n   {}\n",
        a.label,
        summary_line(&sa),
        b.label,
        summary_line(&sb)
    );
    let mut doc = json!({"a": sa, "b": sb});
    for (name, x, y, rx, ry) in [("a_not_b", a, b, &ra, &rb), ("b_not_a", b, a, &rb, &ra)] {
        let (xs, ys) = if name == "a_not_b" {
            ("A", "B")
        } else {
            ("B", "A")
        };
        // Routines only x ran; routines both ran with instructions only x
        // ran; branch ways x took that y didn't.
        let only: Vec<usize> = rx.keys().filter(|i| !ry.contains_key(i)).copied().collect();
        let mut paths: Vec<(usize, Vec<Va>)> = Vec::new();
        let mut ways_list: Vec<String> = Vec::new();
        let mut shared_same = 0;
        for i in rx.keys().filter(|i| ry.contains_key(i)) {
            let body = map.body(*i);
            let new: Vec<Va> = body
                .iter()
                .map(|(p, _)| *p)
                .filter(|p| x.ran.contains_key(p) && !y.ran.contains_key(p))
                .collect();
            for (p, ins) in &body {
                if ins.is_none_or(|i| i.mode != Mode::Rel) {
                    continue;
                }
                let (Some(sx), Some(sy)) = (x.ran.get(p), y.ran.get(p)) else {
                    continue;
                };
                if sx.taken > 0 && sy.taken == 0 {
                    ways_list.push(format!("{} taken ({})", va_hex(*p), map.routines[*i].name));
                }
                if sx.fell > 0 && sy.fell == 0 {
                    ways_list.push(format!("{} fell ({})", va_hex(*p), map.routines[*i].name));
                }
            }
            if new.is_empty() {
                shared_same += 1;
            } else {
                paths.push((*i, new));
            }
        }
        let n_new: usize = paths.iter().map(|(_, v)| v.len()).sum();
        let _ = writeln!(
            text,
            "\n{xs} ran {} routines: {} that {ys} also ran with no instruction {ys} didn't run, {} with instructions only {xs} ran ({n_new} instructions), {} that {ys} didn't run at all",
            rx.len(),
            shared_same,
            paths.len(),
            only.len()
        );
        if !only.is_empty() {
            let _ = writeln!(text, "  routines only {xs} ran:");
            for i in &only {
                let r = &map.routines[*i];
                let _ = writeln!(
                    text,
                    "    {} ({}) {}/{} instructions",
                    r.name, r.element, rx[i].run, rx[i].insns
                );
            }
        }
        if !paths.is_empty() {
            let _ = writeln!(text, "  instructions only {xs} ran, in routines both ran:");
            for (i, v) in &paths {
                let _ = writeln!(
                    text,
                    "    {} ({}): {} instructions, {}",
                    map.routines[*i].name,
                    map.routines[*i].element,
                    v.len(),
                    spans(map, v)
                );
            }
        }
        if !ways_list.is_empty() {
            let _ = writeln!(
                text,
                "  branch ways {xs} took that {ys} didn't, in instructions both ran: {}",
                ways_list.len()
            );
            for w in &ways_list {
                let _ = writeln!(text, "    {w}");
            }
        }
        let outside_new: Vec<Va> = x
            .ran
            .keys()
            .filter(|p| !y.ran.contains_key(p))
            .filter(|p| map.routine_of(**p).is_none())
            .copied()
            .collect();
        if !outside_new.is_empty() {
            let _ = writeln!(
                text,
                "  outside the code map, only {xs}: {}",
                spans(map, &outside_new)
            );
        }
        doc[name] = json!({
            "routines_only": only.iter().map(|i| map.routines[*i].name.clone()).collect::<Vec<_>>(),
            "routines_same": shared_same,
            "routines_with_new_paths": paths.iter().map(|(i, v)| json!({
                "routine": map.routines[*i].name,
                "instructions": v.len(),
                "at": spans(map, v),
            })).collect::<Vec<_>>(),
            "new_instructions": n_new,
            "branch_ways": ways_list,
            "outside_code_map": outside_new.len(),
        });
    }
    (text.trim_end().to_owned(), doc)
}
