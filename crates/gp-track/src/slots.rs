//! Object slots from bound tracks (docs/formats/tracks.md, "Slots"): the
//! player, groups of slots at one stride with their active flag, page, y
//! and ID bytes, and the frame counter.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bind::{Bound, Camera, Seen, camera_at, edge};
use crate::{Frame, Options, RAM, Source};

/// A RAM byte the tracker proposes, and how sure it is (0 to 1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Field {
    pub ram: u16,
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Player {
    pub x: Field,
    pub page: Option<Field>,
    pub y: Option<Field>,
    /// The share of frames its x moved the way the controller said.
    pub input: f64,
    pub tracks: usize,
    pub frames: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IdField {
    pub ram: u16,
    pub score: f64,
    /// The IDs seen, in order.
    pub values: Vec<u8>,
}

/// Slots at `x.ram + i * stride`, for i below `count`; the other fields
/// are slot 0's, at the same stride.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Group {
    pub count: usize,
    pub stride: usize,
    pub x: Field,
    pub x_page: Option<Field>,
    pub y: Option<Field>,
    pub active: Option<Field>,
    pub id: Option<IdField>,
    pub tracks: usize,
    pub frames: usize,
}

impl Group {
    pub fn slot(&self, addr: u16) -> Option<usize> {
        let a = usize::from(addr);
        let x = usize::from(self.x.ram);
        (a >= x && (a - x) % self.stride == 0 && (a - x) / self.stride < self.count)
            .then(|| (a - x) / self.stride)
    }
}

/// The byte that goes up by one on most frames.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Clock {
    pub ram: u16,
    pub score: f64,
}

pub struct Found {
    pub player: Option<Player>,
    pub player_tracks: BTreeSet<usize>,
    pub groups: Vec<Group>,
    pub clock: Option<Clock>,
    pub slot_of: Vec<Option<(usize, usize)>>,
    pub ids: Vec<Option<u8>>,
    pub positions: Vec<Vec<[i64; 3]>>,
}

/// Frames an address's tracks must cover before it can be a slot.
const MIN_SUPPORT: usize = 32;
/// The player's x must move the way the controller says on this share of
/// the frames the controller says anything, and on [`MIN_AGREE`] of the
/// frames it moves while the controller is held.
const MIN_INPUT: f64 = 0.4;
const MIN_AGREE: f64 = 0.9;
/// A flag must be set on this share of the sampled frames its slot's
/// object is drawn in.
const FLAG_SET: f64 = 0.98;
/// The least share of the undrawn frames an active flag is clear on.
const MIN_FLAG: f64 = 0.4;
/// Flag scores this close are the same.
const FLAG_NOISE: f64 = 0.02;
/// Largest stride searched between slots.
const MAX_STRIDE: usize = 16;

pub fn find(
    source: &mut dyn Source,
    opts: &Options,
    seen: &Seen,
    bound: &Bound,
) -> Result<Found, String> {
    // Frames each x address's tracks cover, and the controller agreement.
    // (frames, tracks, input weighted by frames, frames with an input
    // share, agreement weighted by frames, frames with an agreement)
    let mut support: BTreeMap<u16, (usize, usize, f64, usize, f64, usize)> = BTreeMap::new();
    for (t, b) in bound.x.iter().enumerate() {
        let Some(b) = b else { continue };
        let e = support.entry(b.ram).or_insert((0, 0, 0.0, 0, 0.0, 0));
        let w = seen.tracks[t].obs.len();
        e.0 += w;
        e.1 += 1;
        if let Some(input) = b.input {
            e.2 += input * w as f64;
            e.3 += w;
        }
        if let Some(agree) = b.input_agree {
            e.4 += agree * w as f64;
            e.5 += w;
        }
    }
    // The player's x answers the controller: it moves the way the pad says
    // on most frames the pad says anything, and nearly always when it moves
    // while the pad is held.
    let input_of = |s: &(usize, usize, f64, usize, f64, usize)| {
        (s.3 > 0 && s.5 > 0).then(|| (s.2 / s.3 as f64, s.4 / s.5 as f64))
    };
    let player_addr = support
        .iter()
        .filter(|(_, s)| s.0 >= MIN_SUPPORT)
        .filter_map(|(a, s)| {
            let (input, agree) = input_of(s)?;
            (input >= MIN_INPUT && agree >= MIN_AGREE).then_some((*a, input, s.1, s.0))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1).then(a.3.cmp(&b.3)));
    let addrs: Vec<u16> = support
        .iter()
        .filter(|(a, s)| s.0 >= MIN_SUPPORT && Some(**a) != player_addr.map(|p| p.0))
        .map(|(a, _)| *a)
        .collect();

    // Which bound addresses are drawn in each sampled frame.
    let drawn: Vec<BTreeSet<u16>> = bound
        .samples
        .iter()
        .map(|(i, _)| {
            seen.by_frame[*i]
                .iter()
                .filter_map(|(t, _)| bound.x[*t as usize].as_ref().map(|b| b.ram))
                .collect()
        })
        .collect();
    let mut taken: BTreeSet<usize> = addrs.iter().map(|a| usize::from(*a)).collect();
    if let Some(p) = player_addr {
        taken.insert(usize::from(p.0));
    }
    let flags: BTreeMap<u16, FlagTable> = addrs
        .iter()
        .map(|a| (*a, flag_table(*a, bound, &drawn, opts, &taken)))
        .collect();

    // Runs of addresses at one stride that share a flag offset.
    let mut left: BTreeSet<u16> = addrs.iter().copied().collect();
    let mut runs: Vec<(Vec<u16>, usize)> = Vec::new();
    loop {
        let mut best: Option<(Vec<u16>, usize)> = None;
        for stride in 1..=MAX_STRIDE {
            for &start in &left {
                let mut run = vec![start];
                let mut shared: BTreeSet<i32> = flags[&start].top.clone();
                loop {
                    let next = usize::from(*run.last().unwrap()) + stride;
                    let Ok(next) = u16::try_from(next) else { break };
                    if !left.contains(&next) {
                        break;
                    }
                    let both: BTreeSet<i32> =
                        shared.intersection(&flags[&next].top).copied().collect();
                    if both.is_empty() {
                        break;
                    }
                    shared = both;
                    run.push(next);
                }
                let better = match &best {
                    None => run.len() >= 2,
                    Some((b, _)) => run.len() > b.len(),
                };
                if better {
                    best = Some((run, stride));
                }
            }
        }
        let Some((mut run, stride)) = best else { break };
        // Past the slots of one kind, an array of a game's objects holds
        // others, which fit a weaker flag: keep the longest prefix whose
        // flag is nearly as clear as the clearest prefix's.
        let scores: Vec<(usize, f64)> = (2..=run.len())
            .filter_map(|n| best_flag(&run[..n], &flags).map(|(_, s)| (n, s)))
            .collect();
        let top = scores.iter().map(|s| s.1).fold(f64::MIN, f64::max);
        let keep = scores
            .iter()
            .rfind(|s| s.1 >= top - FLAG_NOISE)
            .map_or(2, |s| s.0);
        run.truncate(keep);
        // Those others are also seen on far fewer frames (fireballs after
        // enemies): trailing slots drawn on under a tenth of the median are
        // left out.
        let mut drawn: Vec<u32> = run.iter().map(|a| flags[a].drawn).collect();
        let full = drawn.clone();
        drawn.sort_unstable();
        let median = drawn[drawn.len() / 2];
        while run.len() > 2 && full[run.len() - 1] * 10 < median {
            run.pop();
        }
        for a in &run {
            left.remove(a);
        }
        runs.push((run, stride));
    }

    let mut groups = Vec::new();
    for (run, stride) in &runs {
        groups.push(group(run, *stride, seen, bound, &flags, &taken));
    }
    // The group with the most tracked frames first.
    groups.sort_by(|a, b| b.frames.cmp(&a.frames).then(a.x.ram.cmp(&b.x.ram)));

    // Pages: a full pass over the bytes that change with each slot's x.
    let mut watch: Vec<u16> = groups
        .iter()
        .flat_map(|g| (0..g.count).map(move |i| g.x.ram + (i * g.stride) as u16))
        .collect();
    if let Some(p) = player_addr {
        watch.push(p.0);
    }
    let mut taken_now = taken.clone();
    for g in &groups {
        for i in 0..g.count {
            for f in [g.active.as_ref(), g.y.as_ref()].into_iter().flatten() {
                taken_now.insert(usize::from(f.ram) + i * g.stride);
            }
        }
    }
    let pages = page_pass(source, opts, seen, bound, &watch, &taken_now)?;
    for g in &mut groups {
        let addrs: Vec<u16> = (0..g.count)
            .map(|i| g.x.ram + (i * g.stride) as u16)
            .collect();
        g.x_page = pick_page(&addrs, &pages, bound, opts, &taken_now);
    }
    let player = player_addr.map(|(a, input, tracks, frames)| {
        let player_tracks: Vec<usize> = (0..seen.tracks.len())
            .filter(|t| bound.x[*t].as_ref().is_some_and(|b| b.ram == a))
            .collect();
        let score = mean(
            player_tracks
                .iter()
                .map(|t| bound.x[*t].as_ref().unwrap().score),
        );
        Player {
            x: Field { ram: a, score },
            page: pick_page(&[a], &pages, bound, opts, &taken_now),
            y: y_of(&[a], 1, seen, bound),
            input,
            tracks,
            frames,
        }
    });

    // Each track's slot and ID.
    let mut slot_of = vec![None; seen.tracks.len()];
    let mut ids = vec![None; seen.tracks.len()];
    let mut player_tracks = BTreeSet::new();
    for (t, b) in bound.x.iter().enumerate() {
        let Some(b) = b else { continue };
        if player.as_ref().is_some_and(|p| p.x.ram == b.ram) {
            player_tracks.insert(t);
        }
        for (gi, g) in groups.iter().enumerate() {
            if let Some(i) = g.slot(b.ram) {
                slot_of[t] = Some((gi, i));
                if let Some(id) = &g.id {
                    ids[t] = constant(&bound.track_samples[t], usize::from(id.ram) + i * g.stride);
                }
            }
        }
    }

    let positions = positions_pass(source, seen, bound, &groups, player.as_ref(), &slot_of)?;

    let clock = (0..RAM)
        .filter(|a| !opts.excluded(*a))
        .max_by_key(|a| (seen.plus_one[*a], std::cmp::Reverse(*a)))
        .map(|a| Clock {
            ram: a as u16,
            score: f64::from(seen.plus_one[a]) / (seen.frames.max(2) - 1) as f64,
        })
        .filter(|c| c.score >= 0.5);

    Ok(Found {
        player,
        player_tracks,
        groups,
        clock,
        slot_of,
        ids,
        positions,
    })
}

/// For one x address: over the sampled frames, how often each byte is
/// set while the address's object is drawn and clear while it isn't.
struct FlagTable {
    drawn: u32,
    undrawn: u32,
    set_drawn: Vec<u32>,
    clear_undrawn: Vec<u32>,
    /// Flag offsets (flag address minus x address) of the bytes that fit:
    /// set whenever the object is drawn, and clear on some frame it isn't.
    top: BTreeSet<i32>,
}

fn flag_table(
    addr: u16,
    bound: &Bound,
    drawn: &[BTreeSet<u16>],
    opts: &Options,
    taken: &BTreeSet<usize>,
) -> FlagTable {
    let mut t = FlagTable {
        drawn: 0,
        undrawn: 0,
        set_drawn: vec![0; RAM],
        clear_undrawn: vec![0; RAM],
        top: BTreeSet::new(),
    };
    for ((_, row), d) in bound.samples.iter().zip(drawn) {
        if d.contains(&addr) {
            t.drawn += 1;
            for (n, v) in t.set_drawn.iter_mut().zip(row) {
                *n += u32::from(*v != 0);
            }
        } else {
            t.undrawn += 1;
            for (n, v) in t.clear_undrawn.iter_mut().zip(row) {
                *n += u32::from(*v == 0);
            }
        }
    }
    t.top = (0..RAM)
        .filter(|f| flag_score(&t, *f, opts, taken).is_some())
        .map(|f| f as i32 - i32::from(addr))
        .collect();
    t
}

/// The flag offset the slots of `run` share with the largest share of their
/// undrawn frames clear, and that share.
fn best_flag(run: &[u16], flags: &BTreeMap<u16, FlagTable>) -> Option<(i32, f64)> {
    let shared: BTreeSet<i32> = run
        .iter()
        .map(|a| flags[a].top.clone())
        .reduce(|a, b| a.intersection(&b).copied().collect())
        .unwrap_or_default();
    shared
        .iter()
        .map(|off| {
            let (mut clear, mut undrawn) = (0u32, 0u32);
            for a in run {
                let t = &flags[a];
                clear += t.clear_undrawn[(i32::from(*a) + off) as usize];
                undrawn += t.undrawn;
            }
            (*off, f64::from(clear) / f64::from(undrawn.max(1)))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
}

fn flag_score(t: &FlagTable, f: usize, opts: &Options, taken: &BTreeSet<usize>) -> Option<f64> {
    if t.drawn == 0 || t.undrawn == 0 || opts.excluded(f) || taken.contains(&f) {
        return None;
    }
    if f64::from(t.set_drawn[f]) / f64::from(t.drawn) < FLAG_SET || t.clear_undrawn[f] == 0 {
        return None;
    }
    Some(f64::from(t.clear_undrawn[f]) / f64::from(t.undrawn))
}

fn group(
    run: &[u16],
    stride: usize,
    seen: &Seen,
    bound: &Bound,
    flags: &BTreeMap<u16, FlagTable>,
    taken: &BTreeSet<usize>,
) -> Group {
    let base = run[0];
    let members: Vec<usize> = (0..seen.tracks.len())
        .filter(|t| bound.x[*t].as_ref().is_some_and(|b| run.contains(&b.ram)))
        .collect();
    let x_score = mean(members.iter().map(|t| bound.x[*t].as_ref().unwrap().score));
    let frames = members.iter().map(|t| seen.tracks[*t].obs.len()).sum();

    // A flag that is clear on fewer than half the frames the slots aren't
    // drawn explains too little to be their active flag: a scratch byte
    // that happens to be 0 then.
    let active = best_flag(run, flags)
        .filter(|(_, score)| *score >= MIN_FLAG)
        .map(|(off, score)| Field {
            ram: (i32::from(base) + off) as u16,
            score,
        });

    let y = y_of(run, stride, seen, bound);
    let mut taken = taken.clone();
    for i in 0..run.len() {
        for f in [active.as_ref(), y.as_ref()].into_iter().flatten() {
            taken.insert(usize::from(f.ram) + i * stride);
        }
    }
    let id = id_of(run, stride, &members, seen, bound, &taken);
    Group {
        count: run.len(),
        stride,
        x: Field {
            ram: base,
            score: x_score,
        },
        x_page: None,
        y,
        active,
        id,
        tracks: members.len(),
        frames,
    }
}

/// Slot 0's y byte: the offset from x to y most of the slots' tracks have.
fn y_of(run: &[u16], stride: usize, seen: &Seen, bound: &Bound) -> Option<Field> {
    let mut offsets: BTreeMap<i32, (usize, f64)> = BTreeMap::new();
    let mut total = 0usize;
    for t in 0..seen.tracks.len() {
        let (Some(x), Some(y)) = (&bound.x[t], &bound.y[t]) else {
            continue;
        };
        let Some(i) = run.iter().position(|a| *a == x.ram) else {
            continue;
        };
        let w = seen.tracks[t].obs.len();
        total += w;
        let e = offsets
            .entry(i32::from(y.ram) - i32::from(run[0]) - (i * stride) as i32)
            .or_insert((0, 0.0));
        e.0 += w;
        e.1 += y.score * w as f64;
    }
    let (off, (w, score)) = offsets
        .into_iter()
        .max_by(|a, b| a.1.0.cmp(&b.1.0).then(b.0.cmp(&a.0)))?;
    Some(Field {
        ram: (i32::from(run[0]) + off) as u16,
        score: score / w as f64 * w as f64 / total as f64,
    })
}

/// The value at `addr` if it's the same on at least 11 of a track's
/// sampled frames.
fn constant(rows: &[Vec<u8>], addr: usize) -> Option<u8> {
    if rows.is_empty() || addr >= RAM {
        return None;
    }
    let mut hist = [0u8; 256];
    for r in rows {
        hist[usize::from(r[addr])] += 1;
    }
    let (v, n) = hist
        .iter()
        .enumerate()
        .max_by_key(|(v, n)| (**n, std::cmp::Reverse(*v)))?;
    (usize::from(*n) + 1 >= rows.len()).then_some(v as u8)
}

/// Tracks compared pairwise when scoring an ID byte.
const ID_PAIRS_TRACKS: usize = 300;

/// Slot 0's object ID: a byte that stays the same through each object's
/// life and is the same for two objects exactly when they look alike
/// (palette and most tiles).
fn id_of(
    run: &[u16],
    stride: usize,
    members: &[usize],
    seen: &Seen,
    bound: &Bound,
    taken: &BTreeSet<usize>,
) -> Option<IdField> {
    let lives: Vec<(usize, usize)> = members
        .iter()
        .filter(|t| !bound.track_samples[**t].is_empty())
        .map(|t| {
            let a = bound.x[*t].as_ref().unwrap().ram;
            (*t, run.iter().position(|r| *r == a).unwrap())
        })
        .collect();
    if lives.len() < 2 {
        return None;
    }
    let step = lives.len().div_ceil(ID_PAIRS_TRACKS).max(1);
    let picked: Vec<(usize, usize)> = lives.iter().step_by(step).copied().collect();
    let alike: Vec<Vec<bool>> = picked
        .iter()
        .map(|(a, _)| {
            picked
                .iter()
                .map(|(b, _)| looks_alike(seen, *a, *b))
                .collect()
        })
        .collect();
    if alike.iter().flatten().all(|s| *s) {
        // Every object looks the same: nothing tells an ID apart.
        return None;
    }
    let span = (run.len() - 1) * stride;
    let mut best: Option<(f64, f64, usize, Vec<u8>)> = None;
    for base in 0..RAM.saturating_sub(span) {
        if (0..run.len()).any(|i| taken.contains(&(base + i * stride))) {
            continue;
        }
        let values: Vec<Option<u8>> = picked
            .iter()
            .map(|(t, i)| constant(&bound.track_samples[*t], base + i * stride))
            .collect();
        let steady = values.iter().filter(|v| v.is_some()).count() as f64 / values.len() as f64;
        if steady < 0.9 {
            continue;
        }
        let set: BTreeSet<u8> = values.iter().flatten().copied().collect();
        if set.len() < 2 {
            continue;
        }
        let (mut agree, mut pairs) = (0u32, 0u32);
        for a in 0..picked.len() {
            for b in a + 1..picked.len() {
                let (Some(va), Some(vb)) = (values[a], values[b]) else {
                    continue;
                };
                pairs += 1;
                agree += u32::from((va == vb) == alike[a][b]);
            }
        }
        let score = f64::from(agree) / f64::from(pairs.max(1));
        if score < 0.75 {
            continue;
        }
        let better = best.as_ref().is_none_or(|b| (score, steady) > (b.0, b.1));
        if better {
            best = Some((score, steady, base, set.into_iter().collect()));
        }
    }
    best.map(|(score, steady, base, values)| IdField {
        ram: base as u16,
        score: score * steady,
        values,
    })
}

/// Two tracks look alike when they have one palette and share at least a
/// third of their tiles.
fn looks_alike(seen: &Seen, a: usize, b: usize) -> bool {
    let (ta, tb) = (&seen.tracks[a], &seen.tracks[b]);
    if ta.palette != tb.palette {
        return false;
    }
    let shared = ta
        .tiles
        .iter()
        .filter(|t| tb.tiles.binary_search(t).is_ok())
        .count();
    let union = ta.tiles.len() + tb.tiles.len() - shared;
    union > 0 && shared * 3 >= union
}

/// Evidence for each byte being an x address's page (high byte).
struct PageTable {
    wraps: u32,
    wrap_ok: Vec<u32>,
    steps: u32,
    step_changed: Vec<u32>,
    edges: u32,
    edge_ok: Vec<u32>,
}

fn page_pass(
    source: &mut dyn Source,
    opts: &Options,
    seen: &Seen,
    bound: &Bound,
    watch: &[u16],
    taken: &BTreeSet<usize>,
) -> Result<BTreeMap<u16, PageTable>, String> {
    let mut tables: BTreeMap<u16, PageTable> = watch
        .iter()
        .map(|a| {
            (
                *a,
                PageTable {
                    wraps: 0,
                    wrap_ok: vec![0; RAM],
                    steps: 0,
                    step_changed: vec![0; RAM],
                    edges: 0,
                    edge_ok: vec![0; RAM],
                },
            )
        })
        .collect();
    let mut prev: Vec<u8> = Vec::new();
    let mut i = 0usize;
    let _ = (opts, taken);
    source.scan(&mut |f: &Frame<'_>| {
        let ram = &f.ram[..RAM];
        for &(t, o) in &seen.by_frame[i] {
            let Some(b) = &bound.x[t as usize] else {
                continue;
            };
            let Some(table) = tables.get_mut(&b.ram) else {
                continue;
            };
            let track = &seen.tracks[t as usize];
            let obs = &track.obs[o as usize];
            let a = usize::from(b.ram);
            let lo = i32::from(ram[a]);
            if bound.camera != Camera::None {
                // The page's low bit is the 256s bit of the object's x,
                // which the scroll's nametable bit gives on screen.
                let base = lo - edge(obs, b.anchor) - camera_at(bound.camera, obs.frame, seen);
                table.edges += 1;
                for (n, p) in table.edge_ok.iter_mut().zip(ram) {
                    let v = (i32::from(*p) * 256 + base).rem_euclid(512);
                    *n += u32::from(!(128..384).contains(&v));
                }
            }
            let follows = o > 0 && track.obs[o as usize - 1].frame + 1 == obs.frame;
            if follows && !prev.is_empty() {
                let d = lo - i32::from(prev[a]);
                if d.abs() > 128 {
                    let want: u8 = if d < 0 { 1 } else { 255 };
                    table.wraps += 1;
                    for (n, (p, q)) in table.wrap_ok.iter_mut().zip(ram.iter().zip(&prev)) {
                        *n += u32::from(p.wrapping_sub(*q) == want);
                    }
                } else {
                    table.steps += 1;
                    for (n, (p, q)) in table.step_changed.iter_mut().zip(ram.iter().zip(&prev)) {
                        *n += u32::from(p != q);
                    }
                }
            }
        }
        prev.clear();
        prev.extend_from_slice(ram);
        i += 1;
    })?;
    Ok(tables)
}

/// A page byte's fit: wraps it follows, distinct values, the share of
/// frames the scroll puts the slots on screen by it, its address, and the
/// frames that share is over.
type PageFit = (u32, usize, f64, usize, u32);

/// Slot 0's page byte for the slots at `addrs`: one offset from x for all
/// of them that changes only when x wraps, and by one in the direction it
/// wraps. With no wrap seen there is no page byte to name.
fn pick_page(
    addrs: &[u16],
    tables: &BTreeMap<u16, PageTable>,
    bound: &Bound,
    opts: &Options,
    taken: &BTreeSet<usize>,
) -> Option<Field> {
    let stride = if addrs.len() > 1 {
        usize::from(addrs[1] - addrs[0])
    } else {
        1
    };
    let span = (addrs.len() - 1) * stride;
    let mut fits: Vec<PageFit> = Vec::new();
    for base in 0..RAM.saturating_sub(span) {
        let pages: Vec<usize> = (0..addrs.len()).map(|i| base + i * stride).collect();
        if pages.iter().any(|p| opts.excluded(*p) || taken.contains(p)) {
            continue;
        }
        let (mut wraps, mut ok, mut steps, mut changed, mut edges, mut edge_ok) =
            (0, 0, 0, 0, 0, 0);
        for (a, p) in addrs.iter().zip(&pages) {
            let t = &tables[a];
            wraps += t.wraps;
            ok += t.wrap_ok[*p];
            steps += t.steps;
            changed += t.step_changed[*p];
            edges += t.edges;
            edge_ok += t.edge_ok[*p];
        }
        if ok < wraps || f64::from(changed) > 0.02 * f64::from(steps) {
            continue;
        }
        let edge_score = if edges > 0 {
            f64::from(edge_ok) / f64::from(edges)
        } else {
            1.0
        };
        if edge_score < 0.98 {
            continue;
        }
        let distinct: BTreeSet<u8> = bound
            .samples
            .iter()
            .flat_map(|(_, row)| pages.iter().map(|p| row[*p]))
            .collect();
        fits.push((ok, distinct.len(), edge_score, base, edges));
    }
    fits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.3.cmp(&b.3)));
    let first = fits.first()?;
    // A page byte that never changes can't be told from any other constant
    // byte, so one wrap of x is the least evidence that names it; or, with
    // no wrap, slots in different pages that the scroll places on screen
    // (each slot's own value puts it on the right half of the 512 pixels
    // the nametables span) on enough frames, and one candidate alone.
    if first.0 >= 1 {
        return Some(Field {
            ram: first.3 as u16,
            score: first.2,
        });
    }
    // With no wrap, each slot's page is one small value on nearly every
    // frame (the frames before the level is set up aside), the slots'
    // values differ, and the scroll places each slot on screen by it.
    let constant = |base: usize| -> bool {
        let n = bound.samples.len().max(1);
        let mut modes = BTreeSet::new();
        for i in 0..addrs.len() {
            let p = base + i * stride;
            let mut count: BTreeMap<u8, usize> = BTreeMap::new();
            for (_, row) in &bound.samples {
                *count.entry(row[p]).or_default() += 1;
            }
            let Some((v, c)) = count
                .into_iter()
                .max_by_key(|(v, c)| (*c, std::cmp::Reverse(*v)))
            else {
                return false;
            };
            if v >= 8 || c * 100 < n * 95 {
                return false;
            }
            modes.insert(v);
        }
        modes.len() >= 2
    };
    let by_edges: Vec<&PageFit> = fits
        .iter()
        .filter(|f| {
            f.0 == 0 && f.4 >= MIN_SUPPORT as u32 && !(0x100..0x200).contains(&f.3) && constant(f.3)
        })
        .collect();
    // Of those, the one nearest the x bytes, as a slot table keeps an
    // object's fields together; none when two are as near.
    let x0 = usize::from(addrs[0]);
    let mut near: Vec<(usize, &PageFit)> = by_edges
        .into_iter()
        .map(|f| (f.3.abs_diff(x0), f))
        .collect();
    near.sort_by_key(|(d, f)| (*d, f.3));
    match near.as_slice() {
        [(d, f), rest @ ..] if rest.first().is_none_or(|(e, _)| e > d) => Some(Field {
            ram: f.3 as u16,
            score: f.2,
        }),
        _ => None,
    }
}

fn positions_pass(
    source: &mut dyn Source,
    seen: &Seen,
    bound: &Bound,
    groups: &[Group],
    player: Option<&Player>,
    slot_of: &[Option<(usize, usize)>],
) -> Result<Vec<Vec<[i64; 3]>>, String> {
    let page_of: Vec<Option<usize>> = (0..seen.tracks.len())
        .map(|t| {
            let x = bound.x[t].as_ref()?;
            if let Some((g, i)) = slot_of[t] {
                let g = &groups[g];
                return g.x_page.as_ref().map(|p| usize::from(p.ram) + i * g.stride);
            }
            player
                .filter(|p| p.x.ram == x.ram)
                .and_then(|p| p.page.as_ref())
                .map(|p| usize::from(p.ram))
        })
        .collect();
    let mut out: Vec<Vec<[i64; 3]>> = seen
        .tracks
        .iter()
        .map(|t| Vec::with_capacity(t.obs.len()))
        .collect();
    let mut i = 0usize;
    source.scan(&mut |f: &Frame<'_>| {
        for &(t, o) in &seen.by_frame[i] {
            let t = t as usize;
            let obs = &seen.tracks[t].obs[o as usize];
            let x = match &bound.x[t] {
                Some(b) => {
                    let lo = i64::from(f.ram[usize::from(b.ram)]);
                    lo + page_of[t].map_or(0, |p| i64::from(f.ram[p]) * 256)
                }
                None => i64::from(obs.x0),
            };
            let y = match &bound.y[t] {
                Some(b) => i64::from(f.ram[usize::from(b.ram)]),
                None => i64::from(obs.y0),
            };
            out[t].push([obs.frame as i64, x, y]);
        }
        i += 1;
    })?;
    Ok(out)
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = values.fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    if n == 0 { 0.0 } else { sum / n as f64 }
}

/// The proposed `[slots]` section (docs/formats/state-mapping.md, "Slots")
/// for the first group, with the others after it as comments.
pub fn draft(
    groups: &[Group],
    clock: Option<&Clock>,
    player: Option<&Player>,
    from: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Proposed by gp track from {from}. Check each address against the game's\n# source before using it; scores are the share of frames that fit (0 to 1).\n"
    ));
    if let Some(p) = player {
        out.push_str(&format!(
            "# Player: x at 0x{:04X} (score {:.2}, moves with the controller {:.2})",
            p.x.ram, p.x.score, p.input
        ));
        if let Some(pg) = &p.page {
            out.push_str(&format!(", page 0x{:04X} ({:.2})", pg.ram, pg.score));
        }
        if let Some(y) = &p.y {
            out.push_str(&format!(", y 0x{:04X} ({:.2})", y.ram, y.score));
        }
        out.push('\n');
    }
    for (n, g) in groups.iter().enumerate() {
        let mut s = String::new();
        s.push_str(&format!("\n[slots]\ncount = {}\n", g.count));
        if g.stride != 1 {
            s.push_str(&format!(
                "# The slots are {} bytes apart; [slots] reads them 1 apart.\n",
                g.stride
            ));
        }
        match &g.active {
            Some(a) => s.push_str(&format!(
                "active = 0x{:04X} # score {:.2}\n",
                a.ram, a.score
            )),
            None => s.push_str("# active: no flag found\n"),
        }
        match &g.id {
            Some(id) => s.push_str(&format!(
                "id = 0x{:04X} # score {:.2}, IDs seen: {}\n",
                id.ram,
                id.score,
                id.values
                    .iter()
                    .map(|v| format!("0x{v:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )),
            None => {
                s.push_str("# id: no ID byte found (the objects all look alike, or none fits)\n")
            }
        }
        match clock {
            Some(c) => s.push_str(&format!("clock = 0x{:04X} # score {:.2}\n", c.ram, c.score)),
            None => s.push_str("# clock: no frame counter found\n"),
        }
        s.push_str(&format!(
            "note = \"Proposed by gp track: {} tracks over {} frames\"\n",
            g.tracks, g.frames
        ));
        s.push_str("\n[[slots.field]]\npath = \"x\"\n");
        match &g.x_page {
            Some(p) => s.push_str(&format!(
                "ram = [0x{:04X}, 0x{:04X}] # scores {:.2}, page {:.2}\n",
                g.x.ram, p.ram, g.x.score, p.score
            )),
            None => s.push_str(&format!(
                "ram = [0x{:04X}] # score {:.2}; no page byte found\n",
                g.x.ram, g.x.score
            )),
        }
        if let Some(y) = &g.y {
            s.push_str(&format!(
                "\n[[slots.field]]\npath = \"y\"\nram = [0x{:04X}] # score {:.2}\n",
                y.ram, y.score
            ));
        }
        if n == 0 {
            out.push_str(&s);
        } else {
            out.push_str("\n# Another slot group:\n");
            for line in s.lines() {
                if line.is_empty() {
                    out.push_str("#\n");
                } else {
                    out.push_str(&format!("# {line}\n"));
                }
            }
        }
    }
    out
}
