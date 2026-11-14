//! Behaviour read from runs of a ROM, with no model: the part of
//! game-printer's `gp behave` that `gp watch` reports from, the detectors'
//! run built from clips (`build`) and its finds as open facts
//! (`open_facts`). `gp behave` itself (its report, `--grade`) is group B
//! of the port and isn't here.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use gp_events::activity::{Activity, FrameActivity};
use gp_events::{Bits, Collector, Layout};
use gp_evidence::events::EventKind;
use gp_evidence::input_log::InputFrame;
use gp_gaps::{CellChange, Code, FrameFacts, RamWrite, Run};
use gp_nes::Nes;
use tetanes_core::memory::Src;

use crate::gaps::vram_cell;
use crate::reach::ClipInput;
use crate::solve::arith::ONE;
use crate::solve::timers::steps;

/// One clip's frames, and what its run left for the run's totals.
struct Clip {
    frames: Vec<FrameFacts>,
    calls: BTreeMap<u32, gp_events::activity::CallStats>,
    starts: Bits,
    roles: Option<gp_nes::ramrole::RamTrace>,
    dma: BTreeMap<u8, u64>,
    /// Each RAM byte's value at the end of every frame, by address.
    series: Vec<Vec<u8>>,
}

/// The cell under a VRAM address with `mirroring`, as gp gaps reads it.
fn cells_of(prev: &[u8], now: &[u8], fa: &FrameActivity, mirroring: u8) -> Vec<CellChange> {
    let tables = now.len() / 1024;
    let mut writers: BTreeMap<(u8, u8, u8), BTreeSet<u32>> = BTreeMap::new();
    for w in &fa.vram {
        if let Some(cell) = vram_cell(w.addr, mirroring, tables) {
            writers.entry(cell).or_default().insert(w.routine);
        }
    }
    let mut cells = Vec::new();
    if prev.len() != now.len() {
        return cells;
    }
    for p in 0..tables {
        let base = p * 1024;
        for cy in 0..15usize {
            for cx in 0..16usize {
                let at = |r: usize, c: usize| base + (cy * 2 + r) * 32 + cx * 2 + c;
                let idx = [at(0, 0), at(0, 1), at(1, 0), at(1, 1)];
                if idx.iter().all(|&i| now[i] == prev[i]) {
                    continue;
                }
                let key = (p as u8, cx as u8, cy as u8);
                cells.push(CellChange {
                    nt: p as u8,
                    cx: cx as u8,
                    cy: cy as u8,
                    before: idx.map(|i| prev[i]),
                    after: idx.map(|i| now[i]),
                    writers: writers
                        .remove(&key)
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
                });
            }
        }
    }
    cells
}

fn code_of(fa: &FrameActivity) -> Code {
    let mut apu: Vec<u32> = fa.apu.iter().map(|w| w.routine).collect();
    apu.sort_unstable();
    apu.dedup();
    let mut apu_pc: Vec<u32> = fa.apu.iter().map(|w| w.pc).collect();
    apu_pc.sort_unstable();
    apu_pc.dedup();
    Code {
        entered: fa.entered.clone(),
        ram: fa
            .ram
            .iter()
            .map(|w| RamWrite {
                addr: w.addr,
                value: w.value,
                engine: w.engine,
                pc: w.pc,
                routine: w.routine,
            })
            .collect(),
        apu,
        apu_pc,
        calls: fa.calls.clone(),
    }
}

/// Runs one clip with the collector watching, `frame0` the number its
/// first frame takes in the run, and `watch` the routines whose calls to
/// list. With `full`, the frames' facts; else only the watched calls.
fn run_clip(
    rom: &[u8],
    c: &ClipInput,
    frame0: u64,
    watch: &BTreeSet<u32>,
    full: bool,
    trace: Option<gp_nes::ramrole::RamTrace>,
) -> Result<Clip> {
    let mut nes = Nes::power_on(rom, c.power_on)?;
    nes.set_events(full);
    let layout = Layout::of(nes.bus())?;
    let mut a = Activity::watching(layout, &Bits::new(0), watch.clone());
    a.calls_only = !full;
    let mut col = Collector::with_activity(layout, false, a);
    if full {
        // One RAM trace across the clips: each clip's tracer adds to it.
        let mut t = gp_nes::ramrole::Tracer::new();
        if let Some(tr) = trace {
            t.trace = tr;
        }
        col.ramrole = Some(t);
    }
    col.start(nes.bus_mut_for_observers());
    let mut ram_prev = nes.wram()[..0x800].to_vec();
    let mut nt_prev = nes.bus().memory.region_ref(Src::CiRam).to_vec();
    let mut frames = Vec::new();
    let mut dma = BTreeMap::new();
    let mut series: Vec<Vec<u8>> = if full {
        (0..0x800)
            .map(|_| Vec::with_capacity(c.inputs.len()))
            .collect()
    } else {
        Vec::new()
    };
    for (i, f) in c.inputs.iter().enumerate() {
        let input = InputFrame {
            polled: true,
            pads: f.pads,
            reset: f.reset,
        };
        if input.reset {
            col.will_reset();
        }
        if let Some(t) = &mut col.ramrole {
            t.frame_start(nes.bus_mut_for_observers());
        }
        let out = nes.step_with(input, |bus| col.before(bus))?;
        let fa = col.end_frame_activity(nes.bus()).expect("watching");
        if let Some(t) = &mut col.ramrole {
            t.frame_end(nes.bus_mut_for_observers());
        }
        if !full {
            frames.push(FrameFacts {
                frame: frame0 + i as u64 + 1,
                code: Some(Code {
                    calls: fa.calls,
                    ..Code::default()
                }),
                ..FrameFacts::default()
            });
            continue;
        }
        let mut apu = Vec::new();
        for e in nes.take_events() {
            match e.kind {
                EventKind::ApuWrite => apu.push((e.addr, e.value)),
                EventKind::PpuWrite if e.addr == 0x4014 => *dma.entry(e.value).or_default() += 1,
                _ => {}
            }
        }
        let bus = nes.bus();
        let ram_now = &nes.wram()[..0x800];
        let ram_changed: Vec<u16> = (0..0x800u16)
            .filter(|&a| ram_now[usize::from(a)] != ram_prev[usize::from(a)])
            .collect();
        ram_prev.copy_from_slice(ram_now);
        for (a, s) in series.iter_mut().enumerate() {
            s.push(ram_now[a]);
        }
        let nt_now = bus.memory.region_ref(Src::CiRam).to_vec();
        let mirroring = bus.mapper.mirroring() as u8;
        let cells = cells_of(&nt_prev, &nt_now, &fa, mirroring);
        nt_prev = nt_now;
        let t = bus.ppu.scroll.t;
        let fine = u16::from(bus.ppu.scroll.fine_x as u8 & 7);
        let scroll = [
            (t & 31) * 8 + fine + ((t >> 10) & 1) * 256,
            ((t >> 5) & 31) * 8 + ((t >> 12) & 7) + ((t >> 11) & 1) * 240,
        ];
        frames.push(FrameFacts {
            frame: frame0 + i as u64 + 1,
            // No model says when the player has control: the game reading
            // its controller is play.
            play: out.polled,
            pad: f.pads[0],
            scroll,
            mirroring,
            ram_changed,
            cells,
            apu,
            code: Some(code_of(&fa)),
            ..FrameFacts::default()
        });
    }
    let a = col.activity.take().expect("watching");
    Ok(Clip {
        frames,
        calls: a.calls.into_iter().collect(),
        starts: col.footprint.starts,
        roles: col.ramrole.take().map(|t| t.trace),
        dma,
        series,
    })
}

/// The detectors' run, built from clips, and the RAM roles.
pub(crate) struct Built {
    pub run: Run,
    pub roles: gp_nes::ramrole::Classified,
    /// Bytes that count: their step, period and reloads, merged over clips.
    pub timers: Vec<Timer>,
}

/// A RAM byte that counts by one, as the timer solver reads a series
/// (`solve::timers::steps`): its usual step, the frames between steps when
/// they are all the same in a clip, and the values it is set to other than
/// by a step.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Timer {
    pub addr: u16,
    pub step: i64,
    pub periods: BTreeSet<u64>,
    pub reloads: BTreeSet<i64>,
    pub steps: usize,
    /// The values it took.
    pub values: BTreeSet<u8>,
}

/// Runs every clip and builds the detectors' facts, as `gp gaps` builds
/// them from a recording, with no model: the pass the sound detector needs
/// for the calls to its play-sound candidates is a second run of each clip.
pub(crate) fn build(rom: &[u8], clips: &[ClipInput]) -> Result<Built> {
    let (prg, _) = gp_nes::harvest::ines_prg(rom).context("not an iNES ROM")?;
    let mut run = Run {
        prg: prg.to_vec(),
        ..Run::default()
    };
    let mut calls: BTreeMap<u32, gp_events::activity::CallStats> = BTreeMap::new();
    let mut starts = Bits::new(prg.len());
    let mut dma: BTreeMap<u8, u64> = BTreeMap::new();
    let mut trace: Option<gp_nes::ramrole::RamTrace> = None;
    let mut timers: BTreeMap<u16, Timer> = BTreeMap::new();
    // Each clip's first frame in the run and its RAM, frame by frame.
    let mut clip_ram: Vec<(usize, Vec<Vec<u8>>)> = Vec::new();
    let mut offsets = Vec::new();
    let mut frame0 = 0u64;
    for c in clips {
        let r = run_clip(rom, c, frame0, &BTreeSet::new(), true, trace.take())?;
        offsets.push(frame0);
        frame0 += c.inputs.len() as u64;
        for (k, v) in r.calls {
            calls.entry(k).or_default().merge(&v);
        }
        starts.union(&r.starts).map_err(anyhow::Error::msg)?;
        for (p, n) in r.dma {
            *dma.entry(p).or_default() += n;
        }
        trace = r.roles;
        for (a, v) in r.series.iter().enumerate() {
            let one = ONE;
            let vals: Vec<i64> = v.iter().map(|&x| i64::from(x) * one).collect();
            let st = steps(&vals);
            let regular = st.changes.len() - st.reloads.len();
            // Counting: most changes are the one step.
            if regular < 3 || regular * 2 < st.changes.len() {
                continue;
            }
            let t = timers.entry(a as u16).or_insert_with(|| Timer {
                addr: a as u16,
                step: st.step / one,
                periods: BTreeSet::new(),
                reloads: BTreeSet::new(),
                steps: 0,
                values: BTreeSet::new(),
            });
            if t.step != st.step / one {
                continue;
            }
            t.steps += regular;
            t.values.extend(v.iter().copied());
            t.periods.extend(st.period);
            t.reloads.extend(st.reloads.iter().map(|(_, v)| v / one));
        }
        clip_ram.push((run.frames.len(), r.series));
        run.frames.extend(r.frames);
    }
    if let Some((&page, _)) = dma.iter().max_by_key(|(_, n)| **n) {
        run.known.oam_page = page;
        place_player(&mut run, &clip_ram, page);
    }
    run.calls = calls
        .iter()
        .map(|(t, st)| gp_gaps::CallStat {
            target: *t,
            calls: st.calls,
            callers: st.callers.len(),
            distinct_a: st.distinct_a(),
            reaches_apu: st.reaches_apu,
            engine: st.is_engine(),
        })
        .collect();
    run.executed = (0..prg.len().div_ceil(64))
        .map(|w| {
            (0..64).fold(0u64, |acc, b| {
                let o = w * 64 + b;
                acc | (u64::from(o < prg.len() && starts.contains(o)) << b)
            })
        })
        .collect();
    // The play-sound candidates, and a second pass that lists their calls.
    run.watched = gp_gaps::sound::candidates(&run);
    if !run.watched.is_empty() {
        let watch: BTreeSet<u32> = run.watched.iter().copied().collect();
        let mut i = 0usize;
        for (c, &f0) in clips.iter().zip(&offsets) {
            let r = run_clip(rom, c, f0, &watch, false, None)?;
            for f in r.frames {
                if let (Some(dst), Some(src)) = (run.frames[i].code.as_mut(), f.code) {
                    dst.calls.extend(src.calls);
                }
                i += 1;
            }
        }
    }
    // The RAM roles over every clip's trace.
    let roles = trace.map(|t| t.classify()).unwrap_or_default();
    // A timer or a counter, not a position or a temporary: not in the
    // sprite table's page nor a byte the RAM roles call scratch or a
    // pointer, set other than by its step to at most 4 values, and given a
    // period only once it has stepped regularly 8 times.
    let not_counting: BTreeSet<u16> = roles
        .roles
        .iter()
        .filter(|r| {
            matches!(
                r.role,
                gp_nes::ramrole::Role::Scratch | gp_nes::ramrole::Role::Pointer
            )
        })
        .map(|r| r.addr)
        .collect();
    let oam = u16::from(run.known.oam_page) << 8;
    let timers: Vec<Timer> = timers
        .into_values()
        .filter(|t| !(oam..oam + 0x100).contains(&t.addr) || run.known.oam_page == 0)
        .filter(|t| !not_counting.contains(&t.addr) && t.reloads.len() <= 4)
        .map(|mut t| {
            if t.steps < 8 {
                t.periods.clear();
            }
            t
        })
        .collect();
    Ok(Built { run, roles, timers })
}

/// The player, with no model: the sprite (an entry of the sprite page the
/// game copies by DMA) whose motion in the world, its screen x change plus
/// the scroll's, goes the way the controller's Right and Left say on at
/// least 80% of the frames it moves while one is held, and on at least 20.
/// Each frame it is drawn on gets a player entity at its screen position,
/// so the cataloger can tell a cell the player is by from one it isn't.
fn place_player(run: &mut Run, clips: &[(usize, Vec<Vec<u8>>)], page: u8) {
    use gp_evidence::input_log::button;
    let base = usize::from(page) << 8;
    let n = run.frames.len();
    // The sprite page's x and y for each slot, over the whole run, and
    // the frames that start a clip (no motion to read across them).
    let mut xs = vec![vec![0u8; n]; 64];
    let mut ys = vec![vec![0xFFu8; n]; 64];
    let mut first = vec![false; n];
    for (f0, ram) in clips {
        let len = ram.first().map_or(0, Vec::len);
        if *f0 < n {
            first[*f0] = true;
        }
        for slot in 0..64 {
            for f in 0..len.min(n.saturating_sub(*f0)) {
                ys[slot][f0 + f] = ram[base + slot * 4][f];
                xs[slot][f0 + f] = ram[base + slot * 4 + 3][f];
            }
        }
    }
    let wrap = |d: i32, m: i32| ((d % m) + m + m / 2) % m - m / 2;
    let mut best: Option<(usize, u64)> = None;
    for slot in 0..64 {
        let (mut agree, mut against) = (0u64, 0u64);
        for f in 1..n {
            if first[f] || ys[slot][f] >= 0xEF || ys[slot][f - 1] >= 0xEF {
                continue;
            }
            let pad = run.frames[f].pad;
            let dir = match (pad & button::RIGHT != 0, pad & button::LEFT != 0) {
                (true, false) => 1,
                (false, true) => -1,
                _ => continue,
            };
            let dx = wrap(i32::from(xs[slot][f]) - i32::from(xs[slot][f - 1]), 256)
                + wrap(
                    i32::from(run.frames[f].scroll[0]) - i32::from(run.frames[f - 1].scroll[0]),
                    512,
                );
            if dx == 0 || dx.abs() > 8 {
                continue;
            }
            if dx.signum() == dir {
                agree += 1;
            } else {
                against += 1;
            }
        }
        if agree >= 20 && agree * 10 >= (agree + against) * 8 && best.is_none_or(|b| agree > b.1) {
            best = Some((slot, agree));
        }
    }
    let Some((slot, _)) = best else { return };
    for (f, frame) in run.frames.iter_mut().enumerate() {
        if ys[slot][f] < 0xEF {
            frame.ents.push(gp_gaps::Ent {
                kind: None,
                player: true,
                slot: None,
                id: None,
                x: i32::from(xs[slot][f]),
                y: i32::from(ys[slot][f]) + 1,
            });
        }
    }
}

/// The finds as open facts for `gp model static --clips`: each says what
/// the runs showed and the bytes or routine it is about, for a later rule or
/// a name to settle.
pub(crate) fn open_facts(b: &Built) -> Vec<gp_nes::harvest::Open> {
    use gp_nes::harvest::Open;
    let run = &b.run;
    let mut out = Vec::new();
    let (ifaces, reqs) = gp_gaps::sound::detect(run);
    for i in &ifaces {
        let n = reqs.iter().filter(|r| r.via == i.name).count();
        out.push(Open {
            what: format!("sound interface {}", i.name),
            routine: String::new(),
            pc: String::new(),
            bytes: Vec::new(),
            why: format!("the runs ask the sound engine for {n} effects through it (gp behave)"),
        });
    }
    for c in &gp_gaps::changes::detect(run).background {
        let writers: Vec<String> = c.writers.iter().map(|w| w.0.clone()).collect();
        out.push(Open {
            what: format!("background cell kind {} to {}", c.before, c.after),
            routine: writers.first().cloned().unwrap_or_default(),
            pc: String::new(),
            bytes: Vec::new(),
            why: format!(
                "the runs change {} cells this way {} times, written by {} (gp behave)",
                c.cells,
                c.count,
                writers.join(", ")
            ),
        });
    }
    for t in &b.timers {
        let kind = if t.values.len() <= 2 {
            "flag"
        } else if t.step < 0 {
            "count down"
        } else {
            "count up"
        };
        let every = match t.periods.iter().collect::<Vec<_>>().as_slice() {
            [p] => format!(" every {p} frames"),
            _ => String::new(),
        };
        let set = if t.reloads.is_empty() {
            String::new()
        } else {
            format!(
                ", set to {}",
                t.reloads
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        out.push(Open {
            what: format!("{kind} at ${:04X}", t.addr),
            routine: String::new(),
            pc: String::new(),
            bytes: vec![format!("${:04X}", t.addr)],
            why: format!(
                "steps by {:+}{every}{set}, over {} steps (gp behave)",
                t.step, t.steps
            ),
        });
    }
    out
}
