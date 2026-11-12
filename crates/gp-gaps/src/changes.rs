//! The unexplained-change detector (`gp gaps changes`, docs/gaps.md): state
//! that changes during play and that the model doesn't account for, in
//! five places (RAM, sprites, background, sound, code), each ranked.

use std::collections::{BTreeMap, BTreeSet};

use gp_track::sprites::Tracker;
use serde::Serialize;

use crate::{NO_ROM, Run, cell_on_screen, r2, sig, top, trigger};

/// More visible cells than this changing in one frame is a redraw (a new
/// screen or level), not something happening in it.
pub const REDRAW_CELLS: usize = 40;

/// This many changed cells in one column or row of a frame is the game
/// streaming in new background as it scrolls.
pub const STREAM_CELLS: usize = 6;

/// A background cell that changed at run time: visible on screen, during
/// play, not part of a redraw or a scroll stream, and not in the rows the
/// model's HUD draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellEvent {
    /// Index into [`Run::frames`].
    pub at: usize,
    pub frame: u64,
    pub nt: u8,
    pub cx: u8,
    pub cy: u8,
    /// Screen position, pixels.
    pub sx: i32,
    pub sy: i32,
    /// World position: the camera plus the screen position.
    pub wx: i32,
    pub wy: i32,
    pub before: [u8; 4],
    pub after: [u8; 4],
    pub writers: Vec<u32>,
}

/// The background cells that change at run time, in frame order.
pub fn runtime_cells(run: &Run) -> Vec<CellEvent> {
    let mut out = Vec::new();
    for (at, f) in run.frames.iter().enumerate() {
        if !f.play || f.cells.is_empty() {
            continue;
        }
        let visible: Vec<(&crate::CellChange, (i32, i32))> = f
            .cells
            .iter()
            .filter_map(|c| cell_on_screen(c.nt, c.cx, c.cy, f.mirroring, f.scroll).map(|p| (c, p)))
            .collect();
        if visible.len() > REDRAW_CELLS {
            continue;
        }
        let mut cols: BTreeMap<(u8, u8), usize> = BTreeMap::new();
        let mut rows: BTreeMap<(u8, u8), usize> = BTreeMap::new();
        for c in &f.cells {
            *cols.entry((c.nt, c.cx)).or_default() += 1;
            *rows.entry((c.nt, c.cy)).or_default() += 1;
        }
        for (c, (sx, sy)) in visible {
            if c.cy * 2 < run.known.hud_rows {
                continue;
            }
            if cols[&(c.nt, c.cx)] >= STREAM_CELLS || rows[&(c.nt, c.cy)] >= STREAM_CELLS {
                continue;
            }
            out.push(CellEvent {
                at,
                frame: f.frame,
                nt: c.nt,
                cx: c.cx,
                cy: c.cy,
                sx,
                sy,
                wx: f.camera[0] + sx,
                wy: f.camera[1] + sy,
                before: c.before,
                after: c.after,
                writers: c.writers.clone(),
            });
        }
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct RamItem {
    /// The label (or `Label+n`, or `$xxx` for a 16-byte block with none).
    pub name: String,
    pub addrs: Vec<u16>,
    /// Frames on which one of the bytes changed during play.
    pub play_changes: u64,
    pub changes: u64,
    /// Share of the changes made during play.
    pub play_share: f64,
    /// How much likelier a change is within two frames of a controller
    /// change than on any frame (1 is no correlation).
    pub input_lift: f64,
    /// Whether the label exists (but the model doesn't cite it).
    pub labelled: bool,
    /// What the place ranks by: how much likelier the byte starts changing
    /// near a game event than anywhere in play, held down for bytes that
    /// change on most frames and for the sound engine's own state.
    pub score: f64,
    /// Starts of a run of changes (a change after a frame without one)
    /// within a frame before to two after an event, against chance.
    pub event_lift: f64,
    /// Share of play frames it changed on.
    pub per_frame: f64,
    /// Share of its writes made by the sound engine.
    pub engine_share: f64,
    pub writers: Vec<(String, u64)>,
    pub samples: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OamItem {
    /// `pal<p> tiles <t..>`, or the slot ID the sprites belong to.
    pub name: String,
    pub palette: u8,
    pub tiles: Vec<u8>,
    pub tracks: usize,
    pub frames: u64,
    pub mean_life: f64,
    /// Typical width and height, pixels.
    pub size: [i16; 2],
    /// Share of its tracks that start within 8 frames and 32 pixels of a
    /// background cell change.
    pub near_cell_change: f64,
    /// IDs of mapped slots nearby that no model entity lists.
    pub slot_ids: Vec<u8>,
    pub writers: Vec<(String, u64)>,
    pub samples: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CellItem {
    pub before: String,
    pub after: String,
    pub count: u64,
    /// Distinct world cells.
    pub cells: usize,
    pub writers: Vec<(String, u64)>,
    /// `[frame, world x, world y]`.
    pub samples: Vec<[i64; 3]>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SoundItem {
    pub channel: String,
    pub writes: u64,
    pub triggers: u64,
    pub play_triggers: u64,
    /// Game events a note starts with, by how much likelier a note is
    /// within three frames after one than in any three frames.
    pub with: Vec<(String, f64, u64)>,
    pub writers: Vec<(String, u64)>,
}

/// One sound effect the game asks for that the model has no counterpart
/// for (no fact, mechanic or hook cites its name).
#[derive(Debug, Clone, Serialize)]
pub struct EffectItem {
    pub name: String,
    pub via: String,
    pub id: u8,
    /// `music`, `effect` or `command` (an engine command such as a fade),
    /// as told from what the request does; empty when it can't be told.
    pub class: String,
    pub count: u64,
    pub play_count: u64,
    pub requesters: Vec<(String, u64)>,
    pub samples: Vec<u64>,
    /// Counter changes within 12 frames after: each counter's most common
    /// change and the share of requests it followed.
    pub counters: Vec<(String, i64, f64)>,
    /// Mapped objects that appeared within 4 frames, and the share.
    pub spawns: Vec<(String, f64)>,
    /// Share of requests within 3 frames of a background cell change.
    pub cell_change: f64,
    /// The strongest of those, as a hint at the behaviour it marks.
    pub suggests: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodeItem {
    pub name: String,
    pub prg: u32,
    /// Play frames it ran in.
    pub frames: u64,
    /// Unexplained RAM changes it made.
    pub caused: u64,
    pub samples: Vec<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub schema: u32,
    pub frames: usize,
    pub play_frames: usize,
    pub has_code: bool,
    pub ram: Vec<RamItem>,
    pub oam: Vec<OamItem>,
    pub background: Vec<CellItem>,
    /// Sound effects requested, when the game's sound-request interface
    /// was found (it needs the code signals).
    pub sound: Vec<EffectItem>,
    pub sound_interfaces: Vec<crate::sound::Interface>,
    /// APU activity by channel, and the game events notes start with.
    pub apu: Vec<SoundItem>,
    pub code: Vec<CodeItem>,
    /// What the model has that changes cells or makes sound.
    pub model_tile_mechanics: Vec<String>,
    pub model_sound: bool,
    /// Counters whose values are reported scaled to the units the player
    /// sees, with the factor (`Run::counter_scale`).
    pub counter_scale: BTreeMap<String, i64>,
}

impl Report {
    /// Each place's item count, for the `gaps.<template>.<place>.count`
    /// metrics.
    pub fn counts(&self) -> Vec<(&'static str, usize)> {
        vec![
            ("ram", self.ram.len()),
            ("oam", self.oam.len()),
            ("background", self.background.len()),
            (
                "sound",
                if self.sound_interfaces.is_empty() {
                    self.apu.len()
                } else {
                    self.sound.len()
                },
            ),
            ("code", self.code.len()),
        ]
    }
}

fn samples_spread(frames: &[u64], n: usize) -> Vec<u64> {
    if frames.len() <= n {
        return frames.to_vec();
    }
    (0..n)
        .map(|i| frames[i * (frames.len() - 1) / (n - 1).max(1)])
        .collect()
}

/// A counter that changes on more than this share of play frames is a
/// clock (a timer ticking), not an event.
const CLOCK: f64 = 0.05;

/// Frames either side of a scene change (a redraw, play starting or
/// stopping) a change counts as part of it.
const SCENE: usize = 10;

/// A RAM byte written by more distinct routines than this is scratch
/// space (temporaries, pointers), ranked down in proportion.
const SCRATCH: f64 = 6.0;

/// Kinds of game event, each with the frames a frame before to two after
/// one: each sound effect requested, each counter's changes (of a counter
/// that isn't a clock), background cells changing at run time, and mapped
/// slots filling.
pub fn event_frames(
    run: &Run,
    cells: &[CellEvent],
    reqs: &[crate::sound::Request],
) -> Vec<(String, Vec<bool>)> {
    let n = run.frames.len();
    let play = run.play_frames().max(1) as f64;
    let mut kinds: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    kinds
        .entry("cell change".into())
        .or_default()
        .extend(cells.iter().map(|c| c.at));
    for r in reqs {
        kinds
            .entry(format!("sound {}", r.name))
            .or_default()
            .push(r.at);
    }
    for (c, name) in run.counters.iter().enumerate() {
        let changes: Vec<usize> = (1..n)
            .filter(|&i| {
                run.frames[i].play
                    && run.frames[i].counters.get(c) != run.frames[i - 1].counters.get(c)
            })
            .collect();
        if (changes.len() as f64) <= CLOCK * play {
            kinds
                .entry(format!("counter {name}"))
                .or_default()
                .extend(changes);
        }
    }
    for i in 1..n {
        let had: BTreeSet<u8> = run.frames[i - 1]
            .ents
            .iter()
            .filter_map(|e| e.slot)
            .collect();
        if run.frames[i]
            .ents
            .iter()
            .any(|e| e.slot.is_some_and(|s| !had.contains(&s)))
        {
            kinds.entry("slot fills".into()).or_default().push(i);
        }
    }
    kinds
        .into_iter()
        .filter(|(_, at)| at.len() >= 3)
        .map(|(name, at)| {
            let mut out = vec![false; n];
            for i in at {
                let hi = (i + 2).min(n.saturating_sub(1));
                out[i.saturating_sub(1)..=hi].fill(true);
            }
            (name, out)
        })
        .collect()
}

/// Runs the detector.
pub fn detect(run: &Run) -> Report {
    let k = &run.known;
    let frames = run.frames.len();
    let play_frames = run.play_frames();
    let mut rep = Report {
        schema: crate::SCHEMA,
        frames,
        play_frames,
        has_code: run.has_code(),
        model_tile_mechanics: k.tile_mechanics.clone(),
        model_sound: k.sound,
        counter_scale: run
            .counters
            .iter()
            .zip(&run.counter_scale)
            .filter(|(_, k)| **k != 1)
            .map(|(n, k)| (n.clone(), *k))
            .collect(),
        ..Report::default()
    };

    let cells = runtime_cells(run);
    let (ifaces, reqs) = crate::sound::detect(run);
    let engine = crate::sound::Engine::of(run);
    let events = event_frames(run, &cells, &reqs);
    let event_share: Vec<f64> = events
        .iter()
        .map(|(_, e)| {
            (0..frames).filter(|&i| run.frames[i].play && e[i]).count() as f64
                / play_frames.max(1) as f64
        })
        .collect();
    let at_event: Vec<Vec<usize>> = (0..frames)
        .map(|i| (0..events.len()).filter(|&t| events[t].1[i]).collect())
        .collect();

    // RAM: change counts per byte, and who wrote them.
    let mut onsets = vec![0u64; 0x800];
    let mut onset_hits: Vec<Vec<u64>> = vec![vec![0; events.len()]; 0x800];
    let mut on_input = vec![0u64; 0x800];
    let mut on_scene = vec![0u64; 0x800];
    // Frames near a scene change: a redraw of the background, or play
    // starting or stopping. A byte that changes only then is set up with
    // a level, not changed by what happens in it.
    let mut scene = vec![false; frames];
    for i in 0..frames {
        let f = &run.frames[i];
        let start = i > 0 && run.frames[i - 1].play != f.play;
        if f.cells.len() > REDRAW_CELLS || start {
            scene[i.saturating_sub(SCENE)..(i + SCENE + 1).min(frames)].fill(true);
        }
    }
    let mut engine_writes = vec![0u64; 0x800];
    let mut writer_routines: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); 0x800];
    // The routines that write the background through $2007: a byte they
    // write is the renderer's (an update buffer it empties, a count it
    // clears), not game state.
    let renderers: BTreeSet<u32> = run
        .frames
        .iter()
        .flat_map(|f| f.cells.iter().flat_map(|c| c.writers.iter().copied()))
        .filter(|&r| r != NO_ROM)
        .collect();
    let mut renderer_writes = vec![0u64; 0x800];
    let mut all_writes = vec![0u64; 0x800];
    let mut changes = vec![0u64; 0x800];
    let mut play_changes = vec![0u64; 0x800];
    let mut near_input = vec![0u64; 0x800];
    let mut first: Vec<Vec<u64>> = vec![Vec::new(); 0x800];
    let mut writers: Vec<BTreeMap<String, u64>> = vec![BTreeMap::new(); 0x800];
    let mut input_frames = 0u64;
    let pad_changed: Vec<bool> = (0..frames)
        .map(|i| {
            (i.saturating_sub(2)..=i).any(|j| j > 0 && run.frames[j].pad != run.frames[j - 1].pad)
        })
        .collect();
    let oam_lo = u16::from(k.oam_page) << 8;
    let skip = |a: u16| (0x100..0x200).contains(&a) || (oam_lo..oam_lo + 0x100).contains(&a);
    let mut changed_before = vec![false; 0x800];
    for (i, f) in run.frames.iter().enumerate() {
        if f.play && pad_changed[i] {
            input_frames += 1;
        }
        let mut now = vec![false; 0x800];
        for &a in &f.ram_changed {
            let a = a & 0x7FF;
            now[usize::from(a)] = true;
            changes[usize::from(a)] += 1;
            if f.play && !changed_before[usize::from(a)] {
                onsets[usize::from(a)] += 1;
                for &t in &at_event[i] {
                    onset_hits[usize::from(a)][t] += 1;
                }
                on_scene[usize::from(a)] += u64::from(scene[i]);
                // A change on the frame the controller changed, or the
                // one after: a copy of the controller.
                if (i.saturating_sub(1)..=i)
                    .any(|j| j > 0 && run.frames[j].pad != run.frames[j - 1].pad)
                {
                    on_input[usize::from(a)] += 1;
                }
            }
            if f.play {
                play_changes[usize::from(a)] += 1;
                if pad_changed[i] {
                    near_input[usize::from(a)] += 1;
                }
                let s = &mut first[usize::from(a)];
                if s.len() < 64 {
                    s.push(f.frame);
                }
            }
        }
        changed_before = now;
        if let Some(c) = &f.code {
            for w in &c.ram {
                let a = usize::from(w.addr & 0x7FF);
                *writers[a]
                    .entry(k.writer_name(w.pc, w.routine))
                    .or_default() += 1;
                all_writes[a] += 1;
                engine_writes[a] += u64::from(w.engine || engine.has(w.pc));
                if writer_routines[a].len() < 64 {
                    writer_routines[a].insert(w.routine);
                }
                renderer_writes[a] += u64::from(renderers.contains(&w.routine));
            }
        }
    }
    // How often a byte starts changing on a frame the controller changed
    // (or the one after) by chance.
    let pad_share = (1..frames)
        .filter(|&i| {
            run.frames[i].play
                && (i - 1..=i).any(|j| j > 0 && run.frames[j].pad != run.frames[j - 1].pad)
        })
        .count() as f64
        / play_frames.max(1) as f64;
    // A byte's rank: how much likelier it starts changing near one kind
    // of game event than by chance (the best kind, smoothed by a count and
    // weighed by how many starts there are), times how rarely it changes,
    // times the share not written by the sound engine; held down for a
    // scratch byte many routines write and for a copy of the controller.
    let rank = |a: usize| {
        let n = onsets[a] as f64;
        let (lift, _) = (0..events.len())
            .map(|t| {
                let hits = onset_hits[a][t] as f64;
                let l = (hits + 1.0) / (n * event_share[t] + 1.0);
                (l * hits / (hits + 3.0), t)
            })
            .fold((0.0f64, 0usize), |b, x| if x.0 > b.0 { x } else { b });
        let per_frame = play_changes[a] as f64 / play_frames.max(1) as f64;
        let engine_share = if all_writes[a] == 0 {
            0.0
        } else {
            engine_writes[a] as f64 / all_writes[a] as f64
        };
        let scratch = 1.0 / (1.0 + (writer_routines[a].len() as f64 - SCRATCH).max(0.0) / SCRATCH);
        let input = if n >= 3.0 && on_input[a] as f64 >= (pad_share + 0.3).clamp(0.9, 0.99) * n {
            0.1
        } else {
            1.0
        };
        let setup = if n >= 1.0 && on_scene[a] as f64 >= 0.8 * n {
            0.2
        } else {
            1.0
        };
        let score = lift * (1.0 - per_frame) * (1.0 - engine_share) * scratch * input * setup;
        (score, lift, per_frame, engine_share)
    };
    let lift = |a: usize| {
        if play_changes[a] == 0 || input_frames == 0 || play_frames == 0 {
            return 0.0;
        }
        let p_near = near_input[a] as f64 / input_frames as f64;
        let p_any = play_changes[a] as f64 / play_frames as f64;
        p_near / p_any
    };
    let mut groups: BTreeMap<String, Vec<u16>> = BTreeMap::new();
    let mut labelled: BTreeMap<String, bool> = BTreeMap::new();
    for a in 0..0x800u16 {
        if skip(a) || play_changes[usize::from(a)] == 0 || k.explains_ram(a) {
            continue;
        }
        let (name, label) = k.ram_name(a);
        // Bytes past a label's own size group under its base name.
        let base = match name.split_once('+') {
            Some((b, _)) => b.to_owned(),
            None => name,
        };
        labelled.insert(base.clone(), label.is_some());
        groups.entry(base).or_default().push(a);
    }
    for (name, addrs) in groups {
        let pc: u64 = addrs
            .iter()
            .map(|&a| play_changes[usize::from(a)])
            .max()
            .unwrap_or(0);
        let all: u64 = addrs
            .iter()
            .map(|&a| changes[usize::from(a)])
            .max()
            .unwrap_or(0);
        let mut w: BTreeMap<String, u64> = BTreeMap::new();
        for &a in &addrs {
            for (n, c) in &writers[usize::from(a)] {
                *w.entry(n.clone()).or_default() += c;
            }
        }
        let best = addrs
            .iter()
            .copied()
            .max_by_key(|&a| play_changes[usize::from(a)])
            .unwrap_or(0);
        // The label's own bytes rank it; the unlabelled bytes after it
        // only when its own never changed.
        let own: Vec<u16> = addrs
            .iter()
            .copied()
            .filter(|&a| {
                k.ram
                    .iter()
                    .any(|l| l.name == name && l.addr <= a && a < l.addr + l.size.max(1))
            })
            .collect();
        let rankers = if own.is_empty() { &addrs } else { &own };
        let (score, event_lift, per_frame, engine_share) = rankers
            .iter()
            .map(|&a| rank(usize::from(a)))
            .max_by(|x, y| x.0.total_cmp(&y.0))
            .unwrap_or_default();
        let mut s: Vec<u64> = first[usize::from(best)].clone();
        s.dedup();
        rep.ram.push(RamItem {
            labelled: labelled[&name],
            score: r2(score * 100.0) / 100.0,
            event_lift: r2(event_lift),
            per_frame: r2(per_frame),
            engine_share: r2(engine_share),
            name,
            play_changes: pc,
            changes: all,
            play_share: r2(if all == 0 {
                0.0
            } else {
                pc as f64 / all as f64
            }),
            input_lift: r2(lift(usize::from(best))),
            writers: top(&w, 3),
            samples: samples_spread(&s, 5),
            addrs,
        });
    }
    rep.ram.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.play_changes.cmp(&a.play_changes))
            .then(a.name.cmp(&b.name))
    });

    // Background.
    let mut by_kind: BTreeMap<(String, String), Vec<&CellEvent>> = BTreeMap::new();
    for c in &cells {
        by_kind
            .entry((sig(&c.before), sig(&c.after)))
            .or_default()
            .push(c);
    }
    for ((before, after), evs) in by_kind {
        let mut w: BTreeMap<String, u64> = BTreeMap::new();
        for e in &evs {
            for r in &e.writers {
                *w.entry(k.routine_name(*r)).or_default() += 1;
            }
        }
        let distinct: BTreeSet<(i32, i32)> = evs.iter().map(|e| (e.wx >> 4, e.wy >> 4)).collect();
        let frames: Vec<u64> = evs.iter().map(|e| e.frame).collect();
        let picks = samples_spread(&frames, 5);
        rep.background.push(CellItem {
            before,
            after,
            count: evs.len() as u64,
            cells: distinct.len(),
            writers: top(&w, 3),
            samples: picks
                .iter()
                .filter_map(|f| evs.iter().find(|e| e.frame == *f))
                .map(|e| [e.frame as i64, i64::from(e.wx), i64::from(e.wy)])
                .collect(),
        });
    }
    rep.background
        .sort_by(|a, b| b.count.cmp(&a.count).then(a.before.cmp(&b.before)));

    // Sprites: tracks not near any mapped model entity.
    let mut tracker = Tracker::new(4);
    for f in &run.frames {
        tracker.push(f.frame, f.objects.iter().map(|o| o.c.clone()).collect());
    }
    let tracks = tracker.finish();
    let index: BTreeMap<u64, usize> = run
        .frames
        .iter()
        .enumerate()
        .map(|(i, f)| (f.frame, i))
        .collect();
    let mut oam: BTreeMap<String, (u8, Vec<u8>, Vec<&gp_track::sprites::Track>)> = BTreeMap::new();
    let mut slot_ids: BTreeMap<String, BTreeSet<u8>> = BTreeMap::new();
    for t in &tracks {
        if t.obs.len() < 2 {
            continue;
        }
        let mut near = 0usize;
        let mut play = 0usize;
        let mut ids = BTreeSet::new();
        for o in &t.obs {
            let Some(&i) = index.get(&o.frame) else {
                continue;
            };
            let f = &run.frames[i];
            play += usize::from(f.play);
            let (cx, cy) = (
                (i32::from(o.x0) + i32::from(o.x1)) / 2,
                (i32::from(o.y0) + i32::from(o.y1)) / 2,
            );
            let mut hit = false;
            for e in &f.ents {
                let (ex, ey) = (e.x - f.camera[0], e.y - f.camera[1]);
                let dx = (ex - cx).abs().min((ex + 8 - cx).abs());
                let dy = (ey - cy).abs().min((ey + 8 - cy).abs());
                if dx <= 16 && dy <= 24 {
                    if e.kind.is_some() {
                        hit = true;
                    } else if let Some(id) = e.id {
                        ids.insert(id);
                    }
                }
            }
            near += usize::from(hit);
        }
        if play * 2 < t.obs.len() || near * 2 >= t.obs.len() {
            continue;
        }
        let tiles: Vec<u8> = t.tiles.iter().copied().take(4).collect();
        let key = format!(
            "pal{} tiles {}",
            t.palette,
            tiles
                .iter()
                .map(|t| format!("{t:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        slot_ids.entry(key.clone()).or_default().extend(ids);
        oam.entry(key)
            .or_insert_with(|| (t.palette, tiles, Vec::new()))
            .2
            .push(t);
    }
    for (name, (palette, tiles, ts)) in oam {
        let frames: u64 = ts.iter().map(|t| t.obs.len() as u64).sum();
        let mut ws = Vec::new();
        let mut hs = Vec::new();
        let mut near = 0usize;
        let mut w: BTreeMap<String, u64> = BTreeMap::new();
        for t in &ts {
            let o = t.obs[0];
            ws.push(i64::from(o.x1 - o.x0));
            hs.push(i64::from(o.y1 - o.y0));
            let (sx, sy) = (i32::from(o.x0), i32::from(o.y0));
            if cells.iter().any(|c| {
                c.frame + 8 >= o.frame
                    && c.frame <= o.frame + 8
                    && (c.sx - sx).abs() <= 32
                    && (c.sy - sy).abs() <= 32
            }) {
                near += 1;
            }
            if let Some(&i) = index.get(&o.frame) {
                let f = &run.frames[i];
                let sprites: Vec<u8> = f
                    .objects
                    .iter()
                    .find(|ob| ob.c.x0 == o.x0 && ob.c.y0 == o.y0)
                    .map(|ob| ob.sprites.clone())
                    .unwrap_or_default();
                if let Some(c) = &f.code {
                    for wr in &c.ram {
                        let off = wr.addr.wrapping_sub(oam_lo);
                        if off < 0x100 && sprites.contains(&((off / 4) as u8)) {
                            *w.entry(k.writer_name(wr.pc, wr.routine)).or_default() += 1;
                        }
                    }
                }
            }
        }
        ws.sort_unstable();
        hs.sort_unstable();
        let starts: Vec<u64> = ts.iter().map(|t| t.first()).collect();
        rep.oam.push(OamItem {
            name: name.clone(),
            palette,
            tiles,
            tracks: ts.len(),
            frames,
            mean_life: r2(frames as f64 / ts.len() as f64),
            size: [ws[ws.len() / 2] as i16, hs[hs.len() / 2] as i16],
            near_cell_change: r2(near as f64 / ts.len() as f64),
            slot_ids: slot_ids
                .remove(&name)
                .unwrap_or_default()
                .into_iter()
                .collect(),
            writers: top(&w, 3),
            samples: samples_spread(&starts, 5),
        });
    }
    rep.oam
        .sort_by(|a, b| b.tracks.cmp(&a.tracks).then(b.frames.cmp(&a.frames)));

    // Sound: per channel, and which game events notes start with.
    let mut events: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for c in &cells {
        events
            .entry("background cell change".into())
            .or_default()
            .push(c.at);
    }
    for (i, f) in run.frames.iter().enumerate() {
        if i == 0 || !f.play {
            continue;
        }
        let p = &run.frames[i - 1];
        for (n, name) in run.counters.iter().enumerate() {
            if let (Some(a), Some(b)) = (p.counters.get(n), f.counters.get(n)) {
                if b > a {
                    events.entry(format!("{name} up")).or_default().push(i);
                } else if b < a {
                    events.entry(format!("{name} down")).or_default().push(i);
                }
            }
        }
        let had: BTreeSet<u8> = p.ents.iter().filter_map(|e| e.slot).collect();
        if f.ents
            .iter()
            .any(|e| e.slot.is_some_and(|s| !had.contains(&s)))
        {
            events.entry("slot spawn".into()).or_default().push(i);
        }
        if i >= 2 {
            let y = |fr: &crate::FrameFacts| fr.ents.iter().find(|e| e.player).map(|e| e.y);
            if let (Some(a), Some(b), Some(c)) = (y(&run.frames[i - 2]), y(p), y(f))
                && b - a >= 0
                && c - b <= -2
            {
                events.entry("player rises".into()).or_default().push(i);
            }
        }
    }
    for t in &tracks {
        if let Some(&i) = index.get(&t.first()) {
            events
                .entry("sprite object appears".into())
                .or_default()
                .push(i);
        }
    }
    let mut writes: BTreeMap<&str, u64> = BTreeMap::new();
    let mut trig: BTreeMap<&str, Vec<bool>> = BTreeMap::new();
    let mut trig_n: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    let mut sw: BTreeMap<String, u64> = BTreeMap::new();
    for (i, f) in run.frames.iter().enumerate() {
        for &(reg, v) in &f.apu {
            *writes.entry(crate::channel(reg)).or_default() += 1;
            if let Some(ch) = trigger(reg, v) {
                let t = trig.entry(ch).or_insert_with(|| vec![false; frames]);
                if !t[i] {
                    t[i] = true;
                    let e = trig_n.entry(ch).or_default();
                    e.0 += 1;
                    e.1 += u64::from(f.play);
                }
            }
        }
        if let Some(c) = &f.code {
            for r in &c.apu {
                *sw.entry(k.routine_name(*r)).or_default() += 1;
            }
        }
    }
    let sound_writers = top(&sw, 3);
    for ch in crate::CHANNELS {
        let Some(t) = trig.get(ch) else {
            continue;
        };
        let window = |i: usize| (i..(i + 3).min(frames)).any(|j| t[j]);
        let base = (0..frames)
            .filter(|&i| run.frames[i].play && window(i))
            .count() as f64
            / play_frames.max(1) as f64;
        let mut with = Vec::new();
        for (name, at) in &events {
            if at.len() < 3 || base == 0.0 {
                continue;
            }
            let hits = at.iter().filter(|&&i| window(i)).count();
            let lift = hits as f64 / at.len() as f64 / base;
            if lift >= 2.0 && hits >= 3 {
                with.push((name.clone(), r2(lift), hits as u64));
            }
        }
        with.sort_by(|a, b| b.1.total_cmp(&a.1));
        with.truncate(5);
        let (n, p) = trig_n[ch];
        rep.apu.push(SoundItem {
            channel: ch.into(),
            writes: writes.get(ch).copied().unwrap_or(0),
            triggers: n,
            play_triggers: p,
            with,
            writers: sound_writers.clone(),
        });
    }
    if k.sound {
        rep.apu.clear();
    }
    rep.apu.sort_by_key(|a| std::cmp::Reverse(a.play_triggers));

    // Sound requests: each effect the model doesn't cite, with what
    // happens around it.
    rep.sound_interfaces = ifaces;
    let cell_at: BTreeSet<usize> = cells.iter().map(|c| c.at).collect();
    let classes = crate::sound::classes(run, &reqs);
    let mut effects: BTreeMap<(String, u8), Vec<&crate::sound::Request>> = BTreeMap::new();
    for r in &reqs {
        effects.entry((r.via.clone(), r.id)).or_default().push(r);
    }
    // How often each counter changes in any such window, so a hint beats
    // chance (a timer ticks in half of them anyway).
    let counter_base: Vec<f64> = (0..run.counters.len())
        .map(|c| {
            let mut changed = 0usize;
            let mut total = 0usize;
            for i in (1..frames.saturating_sub(12)).step_by(7) {
                if !run.frames[i].play {
                    continue;
                }
                total += 1;
                if run.frames[i - 1].counters.get(c) != run.frames[i + 12].counters.get(c) {
                    changed += 1;
                }
            }
            changed as f64 / total.max(1) as f64
        })
        .collect();
    for ((via, id), rs) in effects {
        let class = classes.get(&(via.clone(), id)).cloned().unwrap_or_default();
        let name = rs[0].name.clone();
        if !k.sound && name.split('/').any(|n| k.cited.contains(n)) {
            continue;
        }
        let n = rs.len() as f64;
        let mut req: BTreeMap<String, u64> = BTreeMap::new();
        let mut cnt: BTreeMap<String, BTreeMap<i64, u64>> = BTreeMap::new();
        let mut spawn: BTreeMap<String, u64> = BTreeMap::new();
        let mut near_cell = 0u64;
        for r in &rs {
            *req.entry(r.requester.clone()).or_default() += 1;
            let lo = r.at.saturating_sub(1);
            let hi = (r.at + 12).min(frames - 1);
            for (c, cname) in run.counters.iter().enumerate() {
                if let (Some(a), Some(b)) = (
                    run.frames[lo].counters.get(c),
                    run.frames[hi].counters.get(c),
                ) && a != b
                {
                    *cnt.entry(cname.clone())
                        .or_default()
                        .entry(b - a)
                        .or_default() += 1;
                }
            }
            let had: BTreeSet<u8> = run.frames[lo].ents.iter().filter_map(|e| e.slot).collect();
            let mut seen = BTreeSet::new();
            for g in &run.frames[lo..=(r.at + 4).min(frames - 1)] {
                for e in &g.ents {
                    if let Some(slot) = e.slot
                        && !had.contains(&slot)
                    {
                        seen.insert(match e.kind {
                            Some(kd) => run.kinds[usize::from(kd)].clone(),
                            None => format!("slot id {:#04x}", e.id.unwrap_or(0)),
                        });
                    }
                }
            }
            for x in seen {
                *spawn.entry(x).or_default() += 1;
            }
            if (r.at.saturating_sub(3)..=r.at + 3).any(|j| cell_at.contains(&j)) {
                near_cell += 1;
            }
        }
        let counters: Vec<(String, i64, f64)> = cnt
            .into_iter()
            .map(|(c, ds)| {
                let (d, m) = top(&ds, 1).into_iter().next().unwrap_or_default();
                (c, d, r2(m as f64 / n))
            })
            .collect();
        let spawns: Vec<(String, f64)> = top(&spawn, 3)
            .into_iter()
            .map(|(x, m)| (x, r2(m as f64 / n)))
            .collect();
        let cell_change = r2(near_cell as f64 / n);
        let mut hints: Vec<(f64, String)> = counters
            .iter()
            .filter(|(c, _, sh)| {
                let b = run
                    .counters
                    .iter()
                    .position(|x| x == c)
                    .map_or(0.0, |i| counter_base[i]);
                *sh >= 2.0 * b
            })
            .map(|(c, d, sh)| (*sh, format!("{c} {d:+}")))
            .chain(spawns.iter().map(|(x, sh)| (*sh, format!("{x} appears"))))
            .chain(std::iter::once((
                cell_change,
                "a background cell changes".to_owned(),
            )))
            .filter(|(sh, _)| *sh >= 0.3)
            .collect();
        hints.sort_by(|a, b| b.0.total_cmp(&a.0));
        let frames_of: Vec<u64> = rs.iter().map(|r| r.frame).collect();
        rep.sound.push(EffectItem {
            name,
            via,
            id,
            class,
            count: rs.len() as u64,
            play_count: rs.iter().filter(|r| run.frames[r.at].play).count() as u64,
            requesters: top(&req, 3),
            samples: samples_spread(&frames_of, 5),
            counters,
            spawns,
            cell_change,
            suggests: hints
                .first()
                .map_or_else(String::new, |(sh, h)| format!("{h} ({:.0}%)", sh * 100.0)),
        });
    }
    rep.sound
        .sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));

    // Code: routines that run during play that nothing in the model cites.
    let mut active: BTreeMap<u32, (u64, Vec<u64>)> = BTreeMap::new();
    let mut caused: BTreeMap<u32, u64> = BTreeMap::new();
    for f in &run.frames {
        let Some(c) = &f.code else { continue };
        if !f.play {
            continue;
        }
        for &(r, _) in &c.entered {
            let e = active.entry(r).or_default();
            e.0 += 1;
            if e.1.len() < 64 {
                e.1.push(f.frame);
            }
        }
        for w in &c.ram {
            if !skip(w.addr) && !k.explains_ram(w.addr) {
                let at = k.routine(w.pc).map_or(w.routine, |l| l.prg);
                *caused.entry(at).or_default() += 1;
            }
        }
    }
    for (prg, (n, s)) in active {
        if prg == NO_ROM || k.cites_routine(prg) {
            continue;
        }
        rep.code.push(CodeItem {
            name: k.routine_name(prg),
            prg,
            frames: n,
            caused: caused.get(&prg).copied().unwrap_or(0),
            samples: samples_spread(&s, 5),
        });
    }
    rep.code
        .sort_by(|a, b| b.frames.cmp(&a.frames).then(b.caused.cmp(&a.caused)));
    rep
}
