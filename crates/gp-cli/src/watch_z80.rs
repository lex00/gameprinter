//! `gp watch` on an arcade romset: the same session folder and event
//! stream as the NES's (crate::watch, gp_events::watch), from gp-arcade's
//! runs and the main Z80's code map (crate::code_z80) where the NES's come
//! from gp-events and the 6502 static reader.
//!
//! What each event means on the board:
//!
//! | Event | On the arcade board |
//! |---|---|
//! | `watch.start` | `mapper` 0 (no banks), `prg_bytes` the main CPU's ROM |
//! | `snapshot` | `static-0.json` (the code map from the ROM alone) and `static.json` (with the runs): its routines, all read, and its stops: each `jp (hl)`, `jp (ix)` and `jp (iy)`, where reading the code alone can't go on |
//! | `new-code` | Main-CPU instructions first run, as address ranges (an address is the offset: the main CPU's ROM isn't banked) |
//! | `edge` | A transfer first taken, gp-events' kinds: `call` and `rst` as `jsr`, `ret` as `rts`, a jump as `jmp` (`branch` when conditional), a fall-through as `fall`, a `jp (..)` as `jmp_indirect`, a `djnz` back as `branch` |
//! | `stop.reached` | A `jp (..)` first taken, to the routine it went to |
//! | `fact` | A routine the code map has with the runs that it didn't before (a `jp (..)` target found) |
//! | `clip.kept` | As on the NES; `end_hash` is the machine's state hash in hex (`OriginalRuntime::state_hex`) |
//! | `watch` (rerun) | A write to a watched address, from gp-arcade's access log, its writer `XXXX` |
//! | `hook` (rerun) | A main-CPU instruction about to run, `at` its address `XXXX`, with `cpu` `z80` and the Z80's register pairs (`af`, `bc`, `de`, `hl`, `ix`, `iy`, `sp`), from gp-arcade's instruction trace |
//!
//! Not made on the board: `bank` (no banks), `pointer` (a `jp (hl)` has no
//! pointer in memory) and `behaviour` (gp behave's detectors read
//! gp-events' NES runs).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use gp_arcade::levels::{EdgeKind as ZEdge, Levels, Watcher};
use gp_arcade::mcr::AccessKind;
use gp_arcade::{Controls, Romset};
use gp_events::EdgeKind;
use gp_events::watch::{ExploreOpts, HookRegs, RegsZ80, StartClip, Stop, WatchBody};
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::code_z80::CodeMap;
use crate::explore::ExploreArgs;
use crate::explore_z80::{self as ez, Graph, Progress};
use crate::reach::ClipInput;
use crate::watch::Writer;

/// The code map as a session file: routines and stops.
fn map_json(map: &CodeMap) -> Value {
    let g = Graph::of(map);
    json!({
        "cpu": "main",
        "instructions": map.insns.len(),
        "routines": map.routines.iter().map(|(at, why)| json!({"name": format!("sub_{at:04X}"), "at": at, "why": why})).collect::<Vec<_>>(),
        "stops": g.indirect.iter().flatten().map(|pc| format!("{pc:04X}")).collect::<Vec<_>>(),
    })
}

fn stops(map: &CodeMap) -> Vec<Stop> {
    Graph::of(map)
        .indirect
        .iter()
        .flatten()
        .map(|pc| Stop {
            id: format!("jp (..)@{pc:04X}"),
            pc: format!("{pc:04X}"),
            target: None,
            behind: 0,
        })
        .collect()
}

/// Writes a code map as `static{tag}.json`, with its hash.
fn write_map(dir: &Path, tag: &str, map: &CodeMap) -> Result<BTreeMap<String, String>> {
    let name = format!("static{tag}.json");
    let text = serde_json::to_string_pretty(&map_json(map))? + "\n";
    std::fs::write(dir.join(&name), &text)?;
    Ok(BTreeMap::from([(
        name,
        gp_evidence::Hash::of(text.as_bytes()).to_string(),
    )]))
}

fn snapshot(w: &mut Writer, dir: &Path, tag: &str, map: &CodeMap) -> Result<()> {
    let files = write_map(dir, tag, map)?;
    w.emit(
        None,
        None,
        None,
        WatchBody::Snapshot {
            files,
            routines: map.routines.len(),
            read: map.routines.len(),
            stops: stops(map),
        },
    )
}

/// gp-events' kind for a Z80 edge, and whether a conditional went.
fn kind_of(k: ZEdge, conditional: bool) -> EdgeKind {
    match k {
        ZEdge::Jump if conditional => EdgeKind::Branch,
        ZEdge::Jump => EdgeKind::Jmp,
        ZEdge::Fall => EdgeKind::Fall,
        ZEdge::Call | ZEdge::Rst => EdgeKind::Jsr,
        ZEdge::Return => EdgeKind::Rts,
        ZEdge::Indirect => EdgeKind::JmpIndirect,
        ZEdge::Interrupt => EdgeKind::Interrupt,
        ZEdge::Loop => EdgeKind::Branch,
    }
}

/// What the session has seen: the main CPU's code and edges, and the
/// `jp (..)`s not taken yet.
struct Seen {
    set: Romset,
    code: BTreeSet<u16>,
    edges: BTreeSet<(u16, u16, ZEdge)>,
    stops: BTreeSet<u16>,
    graph: Graph,
}

impl Seen {
    fn of(set: &Romset, map: &CodeMap, known: Option<&Levels>) -> Seen {
        let graph = Graph::of(map);
        let mut s = Seen {
            set: set.clone(),
            code: BTreeSet::new(),
            edges: BTreeSet::new(),
            stops: graph.indirect.iter().flatten().copied().collect(),
            graph,
        };
        if let Some(k) = known {
            s.code.extend(k.cpus[0].starts.keys());
            s.edges.extend(k.cpus[0].edges.keys());
            for (f, _, kind) in k.cpus[0].edges.keys() {
                if *kind == ZEdge::Indirect {
                    s.stops.remove(f);
                }
            }
        }
        s
    }

    fn name(&self, pc: u16) -> String {
        format!("{pc:04X}")
    }

    fn routine(&self, pc: u16) -> String {
        self.graph
            .at(pc)
            .map_or_else(|| format!("sub_{pc:04X}"), |r| self.graph.name(r))
    }

    /// Runs `clip` from power-on frame by frame, writing what each frame
    /// did that the session hadn't seen; returns its levels.
    fn run(&mut self, w: &mut Writer, clip: &ClipInput) -> Result<Levels> {
        let recipe = clip.key()?;
        w.recipe_arcade(clip, &recipe, &ez::layout(&self.set))?;
        let mut t = gp_arcade::Mcr::power_on(&self.set);
        let mut watcher = Watcher::new(false);
        for f in &clip.inputs {
            let frame = t.frame() + 1;
            let ran = watcher.step(&mut t, Controls::from_bits(f.buttons_word()));
            let lv = &watcher.levels.cpus[0];
            // New code, as ranges of instructions.
            let mut ranges: Vec<(u32, u32)> = Vec::new();
            let mut pcs: Vec<u16> = ran
                .trace
                .iter()
                .filter(|e| e.cpu == 0 && e.interrupt.is_none())
                .map(|e| e.pc)
                .filter(|pc| !self.code.contains(pc))
                .collect();
            pcs.sort_unstable();
            pcs.dedup();
            for pc in pcs {
                self.code.insert(pc);
                let len = lv.lengths.get(&pc).copied().unwrap_or(1).max(1);
                let (a, b) = (u32::from(pc), u32::from(pc) + u32::from(len));
                match ranges.last_mut() {
                    Some(r) if r.1 == a => r.1 = b,
                    _ => ranges.push((a, b)),
                }
            }
            if !ranges.is_empty() {
                let at = ranges.iter().map(|r| self.name(r.0 as u16)).collect();
                w.emit(
                    Some(&recipe),
                    Some(frame),
                    Some(2),
                    WatchBody::NewCode { ranges, at },
                )?;
            }
            let fresh: Vec<(u16, u16, ZEdge)> = lv
                .edges
                .keys()
                .filter(|e| !self.edges.contains(e))
                .copied()
                .collect();
            for (from, to, kind) in fresh {
                self.edges.insert((from, to, kind));
                let conditional = crate::code_z80::insn_at(&self.set, 0, from)
                    .is_some_and(|i| i.flow.conditional());
                w.emit(
                    Some(&recipe),
                    Some(frame),
                    Some(2),
                    WatchBody::Edge {
                        edge: kind_of(kind, conditional),
                        from: self.name(from),
                        to: self.name(to),
                    },
                )?;
                if kind == ZEdge::Indirect && self.stops.remove(&from) {
                    w.emit(
                        Some(&recipe),
                        Some(frame),
                        Some(2),
                        WatchBody::StopReached {
                            stop: format!("jp (..)@{from:04X}"),
                            routine: self.routine(to),
                        },
                    )?;
                }
            }
        }
        Ok(watcher.into_levels())
    }
}

/// The routines `after` has that `before` didn't, as `fact` events.
fn facts(w: &mut Writer, before: &CodeMap, after: &CodeMap) -> Result<()> {
    for at in after.routines.keys() {
        if !before.routines.contains_key(at) {
            w.emit(
                None,
                None,
                None,
                WatchBody::Fact {
                    what: "routine read".into(),
                    routine: format!("sub_{at:04X}"),
                },
            )?;
        }
    }
    Ok(())
}

/// The machine's state hash after a recipe.
fn end_hash(rom: &[u8], clip: &ClipInput) -> Result<String> {
    let mut o = crate::original::OriginalRuntime::of_rom(rom)?;
    let inputs: Vec<gp_runtime::InputFrame> = clip
        .inputs
        .iter()
        .map(|f| gp_runtime::InputFrame::pads(f.pads))
        .collect();
    gp_runtime::Runtime::step(&mut o, &inputs).map_err(anyhow::Error::msg)?;
    o.state_hex()
}

/// `gp watch start` on a romset, the session's folder and writer made.
pub(crate) fn start(
    name: &str,
    e: &ExploreArgs,
    w: &mut Writer,
    dir: &Path,
    rom: &[u8],
    paths: &Paths,
    t0: Instant,
) -> Result<Outcome> {
    let set = Romset::from_zip(rom)?;
    let sets: Vec<std::path::PathBuf> = e.start.iter().cloned().collect();
    let clips = e.starts().with_platform(&sets, paths, true)?;
    let aim = if e.new_code {
        "new-code"
    } else if !e.targets.is_empty() {
        "named"
    } else {
        e.aim.as_str()
    };
    let starts: Vec<StartClip> = clips
        .iter()
        .map(|c| {
            Ok(StartClip {
                id: c.id.clone(),
                recipe: c.key()?,
                frames: c.inputs.len() as u64,
            })
        })
        .collect::<Result<_>>()?;
    let rom_sha = crate::roms::sha256_of(rom)?;
    std::fs::write(
        dir.join("session.json"),
        serde_json::to_string_pretty(&json!({
            "name": name,
            "rom": e.rom,
            "rom_sha256": rom_sha,
            "platform": "arcade",
            "set": set.name,
            "starts": starts,
            "explore": {"aim": aim, "targets": e.targets, "budget": e.budget, "seed": e.seed,
                        "start_runs": e.start_runs, "start_seed": e.start_seed},
        }))? + "\n",
    )?;
    w.emit(
        None,
        None,
        None,
        WatchBody::Start {
            rom_sha256: rom_sha,
            mapper: 0,
            prg_bytes: set.maincpu.len(),
            starts,
            explore: ExploreOpts {
                aim: aim.to_owned(),
                targets: e.targets.clone(),
                budget: e.budget,
                seed: e.seed,
            },
        },
    )?;
    let map0 = CodeMap::of(&set, 0, None);
    snapshot(w, dir, "-0", &map0)?;
    let mut seen = Seen::of(&set, &map0, None);
    for c in &clips {
        seen.run(w, c)?;
    }
    let known = crate::code_z80::run_clips(&set, &clips, (1, u64::MAX))?;
    let map1 = CodeMap::of(&set, 0, Some(&known.cpus[0]));
    facts(w, &map0, &map1)?;
    snapshot(w, dir, "", &map1)?;
    let (ex, map2) = search_into(w, &mut seen, e, &set, rom, dir, clips, &known, &map1)?;
    let secs = t0.elapsed().as_secs_f64();
    Ok(Outcome::ok(
        json!({"name": name, "dir": dir, "events": w.seq(), "kept": ex.kept.len(),
               "reached": ex.reached, "read": map2.routines.len(), "routines": map2.routines.len()}),
        format!(
            "session {name}: {} events in {}; {} routines in the main Z80's code map; kept {} clips, reached {} ({}); {:.1} s",
            w.seq(),
            dir.join("events.ndjson").display(),
            map2.routines.len(),
            ex.kept.len(),
            ex.reached.len(),
            ex.reached.join(", "),
            secs
        ),
    ))
}

/// The search, written into the session as `gp watch start` and `gp watch
/// explore` write it; returns what it found and the code map after.
#[allow(clippy::too_many_arguments)]
fn search_into(
    w: &mut Writer,
    seen: &mut Seen,
    e: &ExploreArgs,
    set: &Romset,
    rom: &[u8],
    dir: &Path,
    clips: Vec<ClipInput>,
    known: &Levels,
    before: &CodeMap,
) -> Result<(ez::Explored, CodeMap)> {
    let start_clips = clips.clone();
    let mut err: Option<anyhow::Error> = None;
    let mut last_kept = 0usize;
    let mut kept_n = 0usize;
    let explored = {
        let w = &mut *w;
        let seen = &mut *seen;
        let mut on = |p: Progress| {
            if err.is_some() {
                return;
            }
            let r = (|| -> Result<()> {
                match p {
                    Progress::Ran {
                        ran,
                        budget,
                        best,
                        kept,
                    } => {
                        if ran % 10 == 0 || kept != last_kept || ran >= budget {
                            last_kept = kept;
                            w.emit(
                                None,
                                None,
                                None,
                                WatchBody::ExploreProgress {
                                    ran,
                                    budget,
                                    kept,
                                    best,
                                },
                            )?;
                        }
                    }
                    Progress::Kept { found } => {
                        let from = &start_clips[found.start];
                        let mut clip = from.clone();
                        clip.inputs
                            .extend(gp_explore::presses::log_frames(&found.pads));
                        clip.id = format!("explore.s{}.k{kept_n}", e.seed);
                        kept_n += 1;
                        let recipe = clip.key()?;
                        w.emit(
                            Some(&recipe),
                            None,
                            None,
                            WatchBody::ClipKept {
                                id: clip.id.clone(),
                                start: from.id.clone(),
                                presses: gp_explore::arcade::spans(&found.pads),
                                frames: found.pads.len(),
                                recipe: recipe.clone(),
                                why: ez::why(found),
                                end_hash: end_hash(rom, &clip)?,
                            },
                        )?;
                        seen.run(w, &clip)?;
                    }
                }
                Ok(())
            })();
            if let Err(x) = r {
                err = Some(x);
            }
        };
        ez::explore_with(e, set, clips, known, &mut on)?
    };
    if let Some(x) = err {
        return Err(x);
    }
    ez::write_clips(&dir.join("clips"), e, set, &explored)?;
    let mut all = explored.start_clips.clone();
    for (i, f) in explored.kept.iter().enumerate() {
        let mut c = explored.start_clips[f.start].clone();
        c.inputs.extend(gp_explore::presses::log_frames(&f.pads));
        c.id = format!("explore.s{}.{i}", e.seed);
        all.push(c);
    }
    let after_levels = crate::code_z80::run_clips(set, &all, (1, u64::MAX))?;
    let after = CodeMap::of(set, 0, Some(&after_levels.cpus[0]));
    facts(w, before, &after)?;
    w.emit(
        None,
        None,
        None,
        WatchBody::End {
            ran: e.budget,
            kept: explored.kept.len(),
            reached: explored.reached.clone(),
            read: after.routines.len(),
            routines: after.routines.len(),
        },
    )?;
    Ok((explored, after))
}

/// `gp watch explore` on a romset session: another search from its start
/// clips.
pub(crate) fn explore_more(
    w: &mut Writer,
    dir: &Path,
    rom: &[u8],
    e: &ExploreArgs,
    starts: Vec<ClipInput>,
    kept: &[ClipInput],
) -> Result<(ez::Explored, CodeMap)> {
    let set = Romset::from_zip(rom)?;
    let mut all = starts.clone();
    all.extend(kept.iter().cloned());
    let known = crate::code_z80::run_clips(&set, &all, (1, u64::MAX))?;
    let map = CodeMap::of(&set, 0, Some(&known.cpus[0]));
    let mut seen = Seen::of(&set, &map, Some(&known));
    search_into(w, &mut seen, e, &set, rom, dir, starts, &known, &map)
}

/// An instruction to hook on the main Z80, as `gp watch rerun --hook`
/// names it on a romset: its address in hex (`0158`, `$0158`, `0x0158`)
/// or a routine's name (`sub_0158`).
pub(crate) fn parse_hook(s: &str) -> Result<u16> {
    let t = s.trim();
    let h = t
        .strip_prefix("sub_")
        .or_else(|| t.strip_prefix('$'))
        .or_else(|| t.strip_prefix("0x"))
        .unwrap_or(t);
    if h.is_empty() || h.len() > 4 {
        anyhow::bail!("{s:?} isn't a main-CPU address (four hex digits, such as 0158 or sub_0158)");
    }
    u16::from_str_radix(h, 16).map_err(|_| {
        anyhow::anyhow!(
            "{s:?} isn't a main-CPU address (four hex digits, such as 0158 or sub_0158)"
        )
    })
}

/// What a rerun saw and wrote.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rerun {
    pub watch_hits: usize,
    pub hook_hits: usize,
    pub written: usize,
}

/// `gp watch rerun` on a romset session: each write to a watched address,
/// from gp-arcade's access log, with the instruction that made it, and
/// each hooked main-CPU instruction about to run, with the Z80's
/// registers, from its instruction trace (which changes no cycle of the
/// run). At most `cap` events an address or hook a recipe.
pub(crate) fn rerun(
    w: &mut Writer,
    set: &Romset,
    all: &[ClipInput],
    addrs: &BTreeSet<u16>,
    hooks: &BTreeSet<u16>,
    cap: usize,
) -> Result<Rerun> {
    let mut out = Rerun::default();
    for c in all {
        let recipe = c.key()?;
        let mut count: BTreeMap<(bool, u16), usize> = BTreeMap::new();
        let mut t = gp_arcade::Mcr::power_on(set);
        for f in &c.inputs {
            let controls = Controls::from_bits(f.buttons_word());
            t.set_accesses(!addrs.is_empty());
            let trace = if hooks.is_empty() {
                t.step(controls);
                Vec::new()
            } else {
                crate::covered::frame_trace(&mut t, controls)
            };
            let acc = t.take_accesses();
            t.set_accesses(false);
            for a in acc
                .iter()
                .filter(|a| a.cpu == 0 && a.kind == AccessKind::Write && addrs.contains(&a.addr))
            {
                out.watch_hits += 1;
                let n = count.entry((false, a.addr)).or_insert(0);
                *n += 1;
                if *n <= cap {
                    out.written += 1;
                    w.emit(
                        Some(&recipe),
                        Some(t.frame()),
                        Some(2),
                        WatchBody::Watch {
                            addr: a.addr,
                            value: a.value,
                            writer: format!("{:04X}", a.pc),
                        },
                    )?;
                }
            }
            // An interrupt's entry comes just before its handler's first
            // instruction, with the same PC: only the instruction counts.
            for e in trace
                .iter()
                .filter(|e| e.cpu == 0 && e.interrupt.is_none() && hooks.contains(&e.pc))
            {
                out.hook_hits += 1;
                let n = count.entry((true, e.pc)).or_insert(0);
                *n += 1;
                if *n <= cap {
                    out.written += 1;
                    w.emit(
                        Some(&recipe),
                        Some(t.frame()),
                        Some(2),
                        WatchBody::Hook {
                            at: format!("{:04X}", e.pc),
                            regs: HookRegs::Z80(RegsZ80 {
                                af: e.af,
                                bc: e.bc,
                                de: e.de,
                                hl: e.hl,
                                ix: e.ix,
                                iy: e.iy,
                                sp: e.sp,
                            }),
                        },
                    )?;
                }
            }
        }
    }
    Ok(out)
}

/// `gp watch detail` on a romset session: each instruction both Z80s ran
/// in frames `a` to `last` of a recipe, as `gp trace instructions` prints
/// them.
pub(crate) fn detail(rom: &[u8], clip: &ClipInput, a: u64, last: u64) -> Result<(Value, String)> {
    let set = Romset::from_zip(rom)?;
    let mut t = gp_arcade::Mcr::power_on(&set);
    let mut text = String::new();
    let mut n = 0usize;
    for (i, f) in clip.inputs.iter().enumerate() {
        let frame = i as u64 + 1;
        if frame > last {
            break;
        }
        let c = Controls::from_bits(f.buttons_word());
        if frame < a {
            t.step(c);
            continue;
        }
        let trace = crate::covered::frame_trace(&mut t, c);
        let begun = t.frame_start();
        text.push_str(&format!("; frame {frame}\n"));
        for e in &trace {
            n += 1;
            text.push_str(&crate::arcade_rec::trace_line(e, begun));
            text.push('\n');
        }
    }
    Ok((
        json!({"clip": clip.id, "from": a, "to": last, "instructions": n}),
        text,
    ))
}

/// `gp watch point` on a romset session: the sprites under a box at a
/// frame (gp-arcade's sprite list, in the game's half-pixel units) and
/// the instruction that last wrote each one's bytes in sprite RAM.
pub(crate) fn point(
    rom: &[u8],
    clip: &ClipInput,
    hash: &str,
    frame: u64,
    rect: (u16, u16, u16, u16),
) -> Result<Value> {
    let set = Romset::from_zip(rom)?;
    let n = usize::try_from(frame)?.min(clip.inputs.len());
    let mut t = gp_arcade::Mcr::power_on(&set);
    let mut writers: BTreeMap<u16, (u16, u64, u8)> = BTreeMap::new();
    for f in &clip.inputs[..n] {
        t.set_accesses(true);
        t.step(Controls::from_bits(f.buttons_word()));
        for a in t.take_accesses() {
            if a.cpu == 0 && a.kind == AccessKind::Write && (0xe800..0xea00).contains(&a.addr) {
                writers.insert(a.addr, (a.pc, t.frame(), a.value));
            }
        }
        t.set_accesses(false);
    }
    let ram: Vec<u8> = (0xe800u16..0xea00).map(|a| t.peek(a)).collect();
    let (bx, by, bw, bh) = (
        i32::from(rect.0),
        i32::from(rect.1),
        i32::from(rect.2),
        i32::from(rect.3),
    );
    let map = CodeMap::of(&set, 0, None);
    let g = Graph::of(&map);
    let sprites: Vec<Value> = gp_arcade::video::sprite_list(&ram)
        .into_iter()
        .filter(|s| {
            let (x, y) = (i32::from(s.x), i32::from(s.y));
            x < bx + bw && bx < x + 16 && y < by + bh && by < y + 16
        })
        .map(|s| {
            let bytes: Vec<Value> = (0..4u16)
                .filter_map(|k| {
                    let addr = 0xe800 + 4 * u16::from(s.slot) + k;
                    let (pc, at, value) = writers.get(&addr)?;
                    Some(
                        json!({"addr": addr, "value": value, "writer": format!("{pc:04X}"),
                                "routine": g.at(*pc).map(|r| g.name(r)), "frame": at}),
                    )
                })
                .collect();
            json!({"index": s.slot, "x": s.x, "y": s.y, "code": s.code, "color": s.color,
                   "flip_x": s.flip_x, "flip_y": s.flip_y, "bytes": bytes})
        })
        .collect();
    let words: Vec<String> = sprites
        .iter()
        .map(|s| {
            let by = s["bytes"]
                .as_array()
                .and_then(|v| v.first())
                .map(|w| {
                    format!(
                        "written by {} ({}) at frame {}",
                        w["routine"].as_str().unwrap_or("no routine"),
                        w["writer"].as_str().unwrap_or(""),
                        w["frame"]
                    )
                })
                .unwrap_or_else(|| "with no write seen".into());
            format!("sprite {}, {by}", s["index"])
        })
        .collect();
    Ok(json!({
        "recipe": hash, "frame": n, "rect": [rect.0, rect.1, rect.2, rect.3],
        "sprite_ram": 0xe800, "sprites": sprites, "words": words,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hook_names_a_main_cpu_address() {
        assert_eq!(parse_hook("0158").unwrap(), 0x0158);
        assert_eq!(parse_hook("sub_0158").unwrap(), 0x0158);
        assert_eq!(parse_hook("$C27D").unwrap(), 0xc27d);
        assert_eq!(parse_hook("0x3").unwrap(), 3);
        assert!(parse_hook("01:8000").is_err());
        assert!(parse_hook("12345").is_err());
        assert!(parse_hook("zz").is_err());
    }

    /// On a made-up set (gp_arcade::testrom::busy), a hook on the main
    /// loop's `in a,(01)` comes back with the Z80's registers: the stack
    /// pointer the reset set, and HL as the last pass left it; a watch on
    /// the frame counter sees its writes. Each is capped a recipe.
    #[test]
    fn a_rerun_hooks_the_main_z80() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("session.json"), "{}").unwrap();
        std::fs::write(dir.path().join("events.ndjson"), "").unwrap();
        let clip = ClipInput {
            set: "t".into(),
            id: "idle".into(),
            power_on: Default::default(),
            inputs: vec![Default::default(); 3],
        };
        let set = gp_arcade::testrom::busy();
        let r = {
            let mut w = Writer::append(dir.path()).unwrap();
            rerun(
                &mut w,
                &set,
                &[clip],
                &BTreeSet::from([0xe001]),
                &BTreeSet::from([0x0003]),
                5,
            )
            .unwrap()
        };
        assert!(r.hook_hits > 5 && r.watch_hits > 5, "{r:?}");
        assert_eq!(r.written, 10);
        let text = std::fs::read_to_string(dir.path().join("events.ndjson")).unwrap();
        let events: Vec<gp_events::watch::WatchEvent> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let hooks: Vec<&RegsZ80> = events
            .iter()
            .filter_map(|e| match &e.body {
                WatchBody::Hook {
                    at,
                    regs: HookRegs::Z80(r),
                } if at == "0003" => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(hooks.len(), 5);
        assert!(hooks.iter().all(|r| r.sp == 0xe7f0), "{hooks:?}");
        assert!(text.lines().all(|l| l.contains(r#""v":2"#)));
        assert!(text.contains(r#""cpu":"z80""#), "{text}");
    }
}
