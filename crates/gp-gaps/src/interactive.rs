//! The interactive-element cataloger (`gp gaps interactive`, docs/gaps.md,
//! docs/formats/interactive.md). Every background cell that changes during
//! play is an interaction, and so is every frame a handler routine fires
//! (one that fires with cell changes far more than by chance) without one.
//! Each gets the context around it: where the player was and how it moved
//! relative to the cell, what the counters did, what appeared nearby,
//! which sound started, which routines fired, what the cell became and
//! whether it changed back. Interactions cluster into kinds by what the
//! cell was and became (or by handler, for those with no cell change).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::changes::{CellEvent, runtime_cells};
use crate::{FrameFacts, NO_ROM, Run, r2, sig, top, trigger};

/// Frames around a cell change searched for its causes and effects.
const BEFORE: usize = 2;
const AFTER: usize = 12;
/// A handler fires on at most this share of play frames.
const RARE: f64 = 0.05;
/// A routine is a handler when this share of the frames it fires on (or
/// the two after) have a cell change.
const HANDLER: f64 = 0.3;
/// Changes of one cell this many frames apart or less are repeats.
const REPEAT_WITHIN: u64 = 1200;
/// A cell that changes back within this many frames was changed for a
/// moment (a bump).
const REVERT_WITHIN: u64 = 60;

/// One interaction.
#[derive(Debug, Clone, Serialize)]
pub struct Interaction {
    pub frame: u64,
    /// World pixels of the cell's top left, or the player's when no cell
    /// changed.
    pub at: [i32; 2],
    pub before: Option<String>,
    pub after: Option<String>,
    /// `below`, `above`, `left`, `right`, `inside` or `away`.
    pub relation: String,
    /// The player's speed over the two frames before, pixels a frame.
    pub player_v: [i32; 2],
    pub counters: BTreeMap<String, i64>,
    pub spawned: Vec<String>,
    pub sound: Vec<String>,
    /// Sound effects requested within two frames before to three after.
    pub requests: Vec<String>,
    /// What found it: `cell`, `sound` (a request with no cell change) or
    /// `handler` (a handler routine with neither).
    pub found_by: String,
    pub routines: Vec<String>,
    pub primary: Option<String>,
    /// The code that asked for its sound effect (the routine label around
    /// the call or the queue write), for one found by its sound.
    #[serde(default)]
    pub requester: Option<String>,
    /// Rarely changing RAM bytes that changed with it.
    pub ram: Vec<String>,
    pub reverted_after: Option<u64>,
}

/// One kind of interactive element, as the catalog record says it
/// (docs/formats/interactive.md).
#[derive(Debug, Clone, Serialize)]
pub struct ElementKind {
    pub kind: String,
    pub cell: Option<CellSig>,
    pub trigger: Trigger,
    pub effects: Effects,
    pub result: Result_,
    pub limits: Limits,
    pub evidence: Vec<u64>,
    pub count: usize,
    pub code: Option<crate::code::Static>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CellSig {
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Trigger {
    /// The most common relation and its share.
    pub relation: String,
    pub share: f64,
    pub relations: BTreeMap<String, u64>,
    /// Mean player speed at the trigger.
    pub player_v: [f64; 2],
    /// The handler most of its interactions fired, and how many did.
    pub routine: Option<String>,
    pub routine_share: f64,
    /// The sound effect most of its interactions requested, and how many
    /// did.
    pub request: Option<String>,
    pub request_share: f64,
    /// The code that asked for the sound most often, for a kind found by
    /// its sound, and how many did.
    #[serde(default)]
    pub requester: Option<String>,
    #[serde(default)]
    pub requester_share: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Effects {
    /// Each counter's most common change and how often it happened.
    pub counters: BTreeMap<String, (i64, u64)>,
    pub spawned: Vec<(String, u64)>,
    pub sound: Vec<(String, u64)>,
    /// Sound effects the game requested at the trigger.
    pub requests: Vec<(String, u64)>,
    pub ram: Vec<(String, u64)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename = "Result")]
pub struct Result_ {
    pub after: Option<String>,
    /// How many changed back to what they were, and after how long on
    /// average (frames).
    pub reverted: u64,
    pub revert_frames: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Limits {
    /// Distinct cells.
    pub cells: usize,
    /// The most changes one cell had, and the mean frames between them.
    pub max_hits: u64,
    pub interval: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Catalog {
    pub schema: u32,
    pub frames: usize,
    pub play_frames: usize,
    pub has_code: bool,
    pub interactions: usize,
    pub kinds: Vec<ElementKind>,
    /// Routines found as handlers: fires, fires with a cell change.
    pub handlers: Vec<(String, u64, u64)>,
    /// Cell kinds nothing touched: the player away from the cell on 90% of
    /// them, no sound asked for and no counter moved with them on most.
    /// Scenery changing (an animation, a scroll seam, a menu or a screen
    /// of text), kept apart from the kinds.
    #[serde(default)]
    pub scenery: Vec<ElementKind>,
    /// The code that asked for sound effects in play, with how many times:
    /// the handlers of interactions that change no cell.
    #[serde(default)]
    pub requesters: Vec<(String, u64)>,
}

fn player(f: &FrameFacts) -> Option<&crate::Ent> {
    f.ents.iter().find(|e| e.player)
}

/// The player's box on screen in frame `f`: the sprite object nearest its
/// mapped position, else a 16 pixel square there.
fn player_box(f: &FrameFacts) -> Option<[i32; 4]> {
    let p = player(f)?;
    let (px, py) = (p.x - f.camera[0], p.y - f.camera[1]);
    let near = f
        .objects
        .iter()
        .map(|o| {
            let c = &o.c;
            let d = (i32::from(c.x0) - px)
                .abs()
                .min((i32::from(c.x1) - px).abs())
                + (i32::from(c.y0) - py)
                    .abs()
                    .min((i32::from(c.y1) - py).abs());
            (
                d,
                [
                    i32::from(c.x0),
                    i32::from(c.y0),
                    i32::from(c.x1),
                    i32::from(c.y1),
                ],
            )
        })
        .filter(|(d, _)| *d <= 24)
        .min_by_key(|(d, _)| *d);
    Some(near.map_or([px, py, px + 16, py + 16], |(_, b)| b))
}

/// Where the player is relative to a 16 pixel cell at screen `(sx, sy)`.
pub fn relation(b: [i32; 4], sx: i32, sy: i32) -> &'static str {
    let [x0, y0, x1, y1] = b;
    let (cx0, cy0, cx1, cy1) = (sx, sy, sx + 16, sy + 16);
    let h = x0 < cx1 + 2 && cx0 - 2 < x1;
    let v = y0 < cy1 + 2 && cy0 - 2 < y1;
    if h && (y0 - cy1).abs() <= 8 && y0 >= cy0 + 8 {
        "below"
    } else if h && (y1 - cy0).abs() <= 8 && y1 <= cy1 - 8 {
        "above"
    } else if v && (x1 - cx0).abs() <= 6 {
        "left"
    } else if v && (x0 - cx1).abs() <= 6 {
        "right"
    } else if h && v {
        "inside"
    } else {
        "away"
    }
}

struct Stats {
    /// Play frames each routine fired in.
    fires: BTreeMap<u32, u64>,
    /// Of those, frames with a cell change in it or the two after.
    with_cell: BTreeMap<u32, u64>,
    /// Play frames each RAM byte changed in.
    ram: Vec<u64>,
}

/// Whether the player is by the cell (not `away`) on the frame before its
/// change; true when no player is mapped.
fn touched(run: &Run, c: &CellEvent) -> bool {
    let prev = &run.frames[c.at.saturating_sub(1)];
    match player_box(prev).or_else(|| player_box(&run.frames[c.at])) {
        Some(b) => relation(b, c.sx, c.sy) != "away",
        None => true,
    }
}

fn stats(run: &Run, cells: &[CellEvent]) -> Stats {
    // A handler is tied to cells the player is by: a menu's or a scroll's
    // cells don't make their writers handlers.
    let change_at: BTreeSet<usize> = cells
        .iter()
        .filter(|c| touched(run, c))
        .map(|c| c.at)
        .collect();
    let mut s = Stats {
        fires: BTreeMap::new(),
        with_cell: BTreeMap::new(),
        ram: vec![0; 0x800],
    };
    for (i, f) in run.frames.iter().enumerate() {
        if !f.play {
            continue;
        }
        for &a in &f.ram_changed {
            s.ram[usize::from(a & 0x7FF)] += 1;
        }
        let Some(c) = &f.code else { continue };
        let near = (i..=i + 2).any(|j| change_at.contains(&j));
        for &(r, _) in &c.entered {
            *s.fires.entry(r).or_default() += 1;
            if near {
                *s.with_cell.entry(r).or_default() += 1;
            }
        }
    }
    s
}

fn handlers(s: &Stats, play: usize) -> BTreeSet<u32> {
    s.fires
        .iter()
        .filter(|(r, n)| {
            **r != NO_ROM
                && (**n as f64) <= RARE * play as f64
                && **n >= 2
                && s.with_cell.get(r).copied().unwrap_or(0) as f64 >= HANDLER * **n as f64
        })
        .map(|(r, _)| *r)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn context(
    run: &Run,
    s: &Stats,
    hs: &BTreeSet<u32>,
    reqs: &BTreeMap<usize, Vec<String>>,
    at: usize,
    sx: i32,
    sy: i32,
    world: [i32; 2],
) -> Interaction {
    let k = &run.known;
    let frames = &run.frames;
    let f = &frames[at];
    let prev = &frames[at.saturating_sub(1)];
    let pb = player_box(prev).or_else(|| player_box(f));
    let relation = pb.map_or("away", |b| relation(b, sx, sy)).to_owned();
    let pv = match (player(&frames[at.saturating_sub(3)]), player(prev)) {
        (Some(a), Some(b)) => [(b.x - a.x) / 2, (b.y - a.y) / 2],
        _ => [0, 0],
    };
    let lo = at.saturating_sub(BEFORE);
    let hi = (at + AFTER).min(frames.len() - 1);
    let mut counters = BTreeMap::new();
    for (n, name) in run.counters.iter().enumerate() {
        if let (Some(a), Some(b)) = (frames[lo].counters.get(n), frames[hi].counters.get(n))
            && a != b
        {
            counters.insert(name.clone(), b - a);
        }
    }
    let mut spawned = BTreeSet::new();
    let had: BTreeSet<u8> = frames[lo].ents.iter().filter_map(|e| e.slot).collect();
    for g in &frames[lo..=(at + 4).min(frames.len() - 1)] {
        for e in &g.ents {
            if let Some(slot) = e.slot
                && !had.contains(&slot)
                && (e.x - world[0]).abs() <= 48
                && (e.y - world[1]).abs() <= 48
            {
                spawned.insert(match e.kind {
                    Some(kind) => run.kinds[usize::from(kind)].clone(),
                    None => format!("slot id {:#04x}", e.id.unwrap_or(0)),
                });
            }
        }
    }
    // Sprite objects that appear near the cell and aren't a mapped entity.
    let before_objs = &frames[lo].objects;
    for g in &frames[at..=(at + 4).min(frames.len() - 1)] {
        for o in &g.objects {
            let (ox, oy) = (i32::from(o.c.x0), i32::from(o.c.y0));
            if (ox - sx).abs() > 24 || (oy - sy).abs() > 32 {
                continue;
            }
            let old = before_objs.iter().any(|b| {
                b.c.palette == o.c.palette
                    && (i32::from(b.c.x0) - ox).abs() <= 8
                    && (i32::from(b.c.y0) - oy).abs() <= 8
            });
            let mapped = g.ents.iter().any(|e| {
                (e.x - g.camera[0] - ox).abs() <= 16 && (e.y - g.camera[1] - oy).abs() <= 16
            });
            if !old && !mapped {
                spawned.insert(format!(
                    "sprite pal{} tiles {}",
                    o.c.palette,
                    o.c.tiles
                        .iter()
                        .take(4)
                        .map(|t| format!("{t:02x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        }
    }
    let mut sound = BTreeSet::new();
    for g in &frames[at..=(at + 3).min(frames.len() - 1)] {
        for &(r, v) in &g.apu {
            if let Some(ch) = trigger(r, v) {
                sound.insert(ch.to_owned());
            }
        }
    }
    let mut routines = BTreeSet::new();
    let mut primary: Option<(f64, u32)> = None;
    for g in &frames[at.saturating_sub(2)..=at] {
        if let Some(c) = &g.code {
            for &(r, _) in &c.entered {
                if hs.contains(&r) {
                    routines.insert(k.routine_name(r));
                    let n = s.fires[&r] as f64;
                    let assoc = s.with_cell.get(&r).copied().unwrap_or(0) as f64 / n;
                    // The most specific handler: most tied to cell changes,
                    // then the rarest.
                    let score = assoc * 1e6 - n;
                    if primary.is_none_or(|(b, _)| score > b) {
                        primary = Some((score, r));
                    }
                }
            }
        }
    }
    let play = run.play_frames().max(1) as f64;
    let mut ram = BTreeSet::new();
    for g in &frames[at.saturating_sub(1)..=at] {
        for &a in &g.ram_changed {
            let a = a & 0x7FF;
            if (s.ram[usize::from(a)] as f64) < 0.02 * play && !(0x100..0x200).contains(&a) {
                ram.insert(k.ram_name(a).0);
            }
        }
    }
    Interaction {
        frame: f.frame,
        at: world,
        before: None,
        after: None,
        relation,
        player_v: pv,
        counters,
        spawned: spawned.into_iter().collect(),
        sound: sound.into_iter().collect(),
        requests: (at.saturating_sub(2)..=at + 3)
            .filter_map(|j| reqs.get(&j))
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        found_by: "cell".into(),
        routines: routines.into_iter().collect(),
        primary: primary.map(|(_, r)| k.routine_name(r)),
        requester: None,
        ram: ram.into_iter().take(12).collect(),
        reverted_after: None,
    }
}

/// A cell kind nothing touched (`Catalog::scenery`).
pub fn is_scenery(k: &ElementKind) -> bool {
    let n = k.count.max(1) as f64;
    let away = k.trigger.relations.get("away").copied().unwrap_or(0) as f64 / n;
    let counters = k
        .effects
        .counters
        .values()
        .map(|(_, c)| *c as f64 / n)
        .fold(0.0, f64::max);
    k.cell.is_some() && away >= 0.9 && k.trigger.request_share < 0.3 && counters < 0.5
}

/// An interaction with the cell signature it changed (before, after), if
/// any, and its world cell.
type Found = (Option<(String, String)>, (i32, i32), Interaction);

/// The kind an interaction clusters into: the cell's tiles before and
/// after, or for one with no cell change the effect it asked for or the
/// handler that fired. A catalog record's `kind` is this key.
pub fn kind_key(it: &Interaction) -> String {
    match (&it.before, &it.after) {
        (Some(b), Some(a)) => format!("cell {b} -> {a}"),
        _ if it.found_by == "sound" => format!(
            "sound {} (no cell change)",
            it.requests.first().cloned().unwrap_or_default()
        ),
        _ => format!(
            "handler {} (no cell change)",
            it.primary.clone().unwrap_or_else(|| "?".into())
        ),
    }
}

/// Runs the cataloger.
pub fn catalog(run: &Run) -> (Catalog, Vec<Interaction>) {
    let cells = runtime_cells(run);
    let s = stats(run, &cells);
    let play = run.play_frames();
    let hs = handlers(&s, play);
    let (_, requests) = crate::sound::detect(run);
    let mut reqs: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut asker: BTreeMap<usize, String> = BTreeMap::new();
    let mut requesters: BTreeMap<String, u64> = BTreeMap::new();
    for r in &requests {
        reqs.entry(r.at).or_default().push(r.name.clone());
        asker.entry(r.at).or_insert_with(|| r.requester.clone());
        if run.frames[r.at].play {
            *requesters.entry(r.requester.clone()).or_default() += 1;
        }
    }
    let mut found: Vec<Found> = Vec::new();
    let mut seen_at: BTreeSet<usize> = BTreeSet::new();
    for (i, c) in cells.iter().enumerate() {
        let mut it = context(run, &s, &hs, &reqs, c.at, c.sx, c.sy, [c.wx, c.wy]);
        it.before = Some(sig(&c.before));
        it.after = Some(sig(&c.after));
        // Changed back within a moment: a later change of the same cell to
        // what it was.
        it.reverted_after = cells[i + 1..]
            .iter()
            .take_while(|d| d.frame <= c.frame + REVERT_WITHIN)
            .find(|d| (d.nt, d.cx, d.cy) == (c.nt, c.cx, c.cy) && d.after == c.before)
            .map(|d| d.frame - c.frame);
        seen_at.insert(c.at);
        found.push((
            Some((sig(&c.before), sig(&c.after))),
            (c.wx >> 4, c.wy >> 4),
            it,
        ));
    }
    // Sound requests with no cell change near: interactions too, such as
    // a bump on a block that doesn't change or a kick.
    let near =
        |set: &BTreeSet<usize>, i: usize| (i.saturating_sub(2)..=i + 2).any(|j| set.contains(&j));
    let mut sound_at: BTreeSet<usize> = BTreeSet::new();
    for (&i, names) in &reqs {
        let f = &run.frames[i];
        if !f.play || near(&seen_at, i) {
            continue;
        }
        let Some(p) = player(f) else { continue };
        let (sx, sy) = (p.x - f.camera[0], p.y - f.camera[1]);
        let mut it = context(run, &s, &hs, &reqs, i, sx, sy, [p.x, p.y]);
        it.relation = "none".into();
        it.found_by = "sound".into();
        it.requests = names.clone();
        it.requester = asker.get(&i).cloned();
        sound_at.insert(i);
        found.push((None, (p.x >> 4, p.y >> 4), it));
    }
    // Handlers firing with neither: interactions too.
    for (i, f) in run.frames.iter().enumerate() {
        if !f.play || near(&seen_at, i) || near(&sound_at, i) {
            continue;
        }
        let Some(c) = &f.code else { continue };
        if !c.entered.iter().any(|(r, _)| hs.contains(r)) {
            continue;
        }
        // One interaction per burst: skip a frame whose previous frame
        // fired a handler too.
        if i > 0
            && run.frames[i - 1]
                .code
                .as_ref()
                .is_some_and(|p| p.entered.iter().any(|(r, _)| hs.contains(r)))
        {
            continue;
        }
        let Some(p) = player(f) else { continue };
        let (sx, sy) = (p.x - f.camera[0], p.y - f.camera[1]);
        let mut it = context(run, &s, &hs, &reqs, i, sx, sy, [p.x, p.y]);
        it.relation = "none".into();
        it.found_by = "handler".into();
        found.push((None, (p.x >> 4, p.y >> 4), it));
    }

    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (n, (_, _, it)) in found.iter().enumerate() {
        groups.entry(kind_key(it)).or_default().push(n);
    }
    let mut kinds = Vec::new();
    for (key, members) in groups {
        let its: Vec<&Interaction> = members.iter().map(|&n| &found[n].2).collect();
        let n = its.len() as f64;
        let mut rel: BTreeMap<String, u64> = BTreeMap::new();
        let mut prim: BTreeMap<String, u64> = BTreeMap::new();
        let mut cnt: BTreeMap<String, BTreeMap<i64, u64>> = BTreeMap::new();
        let mut sp: BTreeMap<String, u64> = BTreeMap::new();
        let mut so: BTreeMap<String, u64> = BTreeMap::new();
        let mut rq: BTreeMap<String, u64> = BTreeMap::new();
        let mut ram: BTreeMap<String, u64> = BTreeMap::new();
        let mut askers: BTreeMap<String, u64> = BTreeMap::new();
        let (mut vx, mut vy) = (0.0, 0.0);
        let mut reverted = Vec::new();
        for it in &its {
            *rel.entry(it.relation.clone()).or_default() += 1;
            if let Some(p) = &it.primary {
                *prim.entry(p.clone()).or_default() += 1;
            }
            if let Some(a) = &it.requester {
                *askers.entry(a.clone()).or_default() += 1;
            }
            for (c, d) in &it.counters {
                *cnt.entry(c.clone()).or_default().entry(*d).or_default() += 1;
            }
            for x in &it.spawned {
                *sp.entry(x.clone()).or_default() += 1;
            }
            for x in &it.requests {
                *rq.entry(x.clone()).or_default() += 1;
            }
            for x in &it.sound {
                *so.entry(x.clone()).or_default() += 1;
            }
            for x in &it.ram {
                *ram.entry(x.clone()).or_default() += 1;
            }
            vx += f64::from(it.player_v[0]);
            vy += f64::from(it.player_v[1]);
            if let Some(r) = it.reverted_after {
                reverted.push(r as f64);
            }
        }
        let best_rel = top(&rel, 1).into_iter().next().unwrap_or_default();
        let best_prim = top(&prim, 1).into_iter().next();
        // Repeats: changes of one world cell close in time.
        let mut per_cell: BTreeMap<(i32, i32), Vec<u64>> = BTreeMap::new();
        for &m in &members {
            per_cell
                .entry(found[m].1)
                .or_default()
                .push(found[m].2.frame);
        }
        let mut max_hits = 0u64;
        let mut gaps = Vec::new();
        for frames in per_cell.values() {
            let mut run_len = 1u64;
            for w in frames.windows(2) {
                if w[1] - w[0] <= REPEAT_WITHIN {
                    run_len += 1;
                    gaps.push((w[1] - w[0]) as f64);
                } else {
                    run_len = 1;
                }
                max_hits = max_hits.max(run_len);
            }
            max_hits = max_hits.max(run_len);
        }
        let cell = found[members[0]].0.clone().map(|(b, a)| CellSig {
            before: b,
            after: a,
        });
        // The code to cross-check: the primary handler, else the code that
        // asked for the sound.
        let primary_label = best_prim
            .as_ref()
            .map(|(name, _)| name.clone())
            .or_else(|| top(&askers, 1).into_iter().next().map(|p| p.0))
            .and_then(|name| run.known.rom.iter().find(|l| l.name == name).cloned());
        let evidence: Vec<u64> = {
            let fr: Vec<u64> = its.iter().map(|i| i.frame).collect();
            if fr.len() <= 5 {
                fr
            } else {
                (0..5).map(|i| fr[i * (fr.len() - 1) / 4]).collect()
            }
        };
        kinds.push(ElementKind {
            kind: key,
            result: Result_ {
                after: cell.as_ref().map(|c| c.after.clone()),
                reverted: reverted.len() as u64,
                revert_frames: r2(crate::mean_sd(&reverted).0),
            },
            cell,
            trigger: Trigger {
                relation: best_rel.0,
                share: r2(best_rel.1 as f64 / n),
                relations: rel,
                player_v: [r2(vx / n), r2(vy / n)],
                routine_share: r2(best_prim.as_ref().map_or(0.0, |p| p.1 as f64 / n)),
                routine: best_prim.map(|p| p.0),
                request_share: r2(top(&rq, 1).first().map_or(0.0, |p| p.1 as f64 / n)),
                request: top(&rq, 1).into_iter().next().map(|p| p.0),
                requester_share: r2(top(&askers, 1).first().map_or(0.0, |p| p.1 as f64 / n)),
                requester: top(&askers, 1).into_iter().next().map(|p| p.0),
            },
            effects: Effects {
                counters: cnt
                    .into_iter()
                    .map(|(c, ds)| {
                        let (d, k) = top(&ds, 1).into_iter().next().unwrap_or_default();
                        (c, (d, k))
                    })
                    .collect(),
                spawned: top(&sp, 4),
                sound: top(&so, 3),
                requests: top(&rq, 4),
                ram: top(&ram, 6),
            },
            limits: Limits {
                cells: per_cell.len(),
                max_hits,
                interval: r2(crate::mean_sd(&gaps).0),
            },
            evidence,
            count: its.len(),
            code: primary_label.and_then(|l| crate::code::scan(run, &l)),
        });
    }
    kinds.sort_by(|a, b| b.count.cmp(&a.count).then(a.kind.cmp(&b.kind)));
    let (scenery, kinds): (Vec<ElementKind>, Vec<ElementKind>) =
        kinds.into_iter().partition(is_scenery);
    let mut hl: Vec<(String, u64, u64)> = hs
        .iter()
        .map(|r| {
            (
                run.known.routine_name(*r),
                s.fires[r],
                s.with_cell.get(r).copied().unwrap_or(0),
            )
        })
        .collect();
    hl.sort_by_key(|a| std::cmp::Reverse(a.2));
    let cat = Catalog {
        scenery,
        requesters: top(&requesters, usize::MAX),
        schema: crate::SCHEMA,
        frames: run.frames.len(),
        play_frames: play,
        has_code: run.has_code(),
        interactions: found.len(),
        kinds,
        handlers: hl,
    };
    (cat, found.into_iter().map(|(_, _, i)| i).collect())
}
