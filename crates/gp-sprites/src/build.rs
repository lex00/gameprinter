//! A complete sprite set per entity from what the tables define and what
//! the recordings showed: each frame once, the clips of each model state
//! (frames in order, their timing, held clips and blinks), the palettes
//! each level showed and a state's own colors, and coverage: which states
//! have frames, which table frames were never seen, which seen frames no
//! table holds.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use gp_assets::sprite_set::{Colors, PaletteSet, Piece, SetFrame, SpriteSet};
use gp_model::Model;
use gp_model::model::{Clip, ClipFrame, Clock, When};
use serde::Serialize;

use crate::chr::TileBank;
use crate::config::Config;
use crate::observe::{CONDITIONS, Cond, Layout, Obs};
use crate::tables::{TableFrame, Tables};

/// One entity's sprites.
#[derive(Debug, Clone, Default)]
pub struct EntitySprites {
    /// By model state ("" for the entity's own sprite): the frame shown
    /// when no clip holds, and the clips.
    pub states: BTreeMap<String, (u32, Vec<Clip>)>,
    /// Observations behind each state's clips, in the clips' order (0 for
    /// a clip from a table).
    pub seen: BTreeMap<String, Vec<usize>>,
    /// Some frame was seen mirrored: the entity faces both ways.
    pub faces_both_ways: bool,
    /// A blink found: its condition and its period.
    pub blink: Option<(When, u32)>,
    pub coverage: Coverage,
}

/// What capture found of one entity, for the report and the scorecard.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Coverage {
    /// The model's states (or `default` for one with no machine).
    pub states: Vec<String>,
    /// Those with a frame.
    pub covered: Vec<String>,
    /// States the rules named that the model doesn't have.
    pub unmodelled: Vec<String>,
    /// Frames in the set for it.
    pub frames: usize,
    /// Table frames, as `<table> <index>`, never seen in a recording.
    pub unobserved: Vec<String>,
    /// Frames seen that no table of it holds (when it has tables).
    pub observed_only: usize,
    pub observations: usize,
    /// Distinct palette colors seen per palette, a sign of variants.
    pub variants: Vec<String>,
    /// Clip timings measured, as `<state>: <frames> every <n>`.
    pub timings: Vec<String>,
}

/// The whole capture.
#[derive(Debug, Clone, Default)]
pub struct Capture {
    pub set: SpriteSet,
    pub entities: BTreeMap<String, EntitySprites>,
    /// Table pieces whose tile no recording showed, so it couldn't be
    /// matched to a lifted tile.
    pub unresolved_pieces: usize,
}

/// A frame key: its layout and the palettes it brings.
type FrameKey = (Layout, Vec<Colors>);

struct SetBuilder {
    frames: Vec<SetFrame>,
    index: HashMap<FrameKey, u32>,
}

impl SetBuilder {
    fn add(
        &mut self,
        entity: &str,
        state: &str,
        from: &str,
        layout: &Layout,
        colors: &[Colors],
    ) -> u32 {
        let key = (layout.clone(), colors.to_vec());
        if let Some(&i) = self.index.get(&key) {
            let f = &mut self.frames[i as usize];
            if !f.from.iter().any(|x| x == from) {
                f.from.push(from.to_owned());
            }
            return i;
        }
        let i = self.frames.len() as u32;
        self.frames.push(SetFrame {
            entity: entity.to_owned(),
            state: state.to_owned(),
            from: vec![from.to_owned()],
            pieces: layout.clone(),
            colors: colors.to_vec(),
        });
        self.index.insert(key, i);
        i
    }
}

/// The tiles a layout shows and where, from its top left corner, without
/// flips or palettes: what a table frame and a seen frame share when the
/// game sets flips and palettes in code.
fn tiles_of(l: &Layout) -> Vec<(Option<String>, u32, i32, i32)> {
    let x0 = l.iter().map(|p| p.x).min().unwrap_or(0);
    let y0 = l.iter().map(|p| p.y).min().unwrap_or(0);
    let mut v: Vec<_> = l
        .iter()
        .map(|p| (p.slot.clone(), p.tile, p.x - x0, p.y - y0))
        .collect();
    v.sort();
    v
}

/// A table frame as a layout: its tiles matched to lifted tiles, blank
/// ones left out, and a frame not drawn from the position stood on the
/// entity's bottom edge.
pub fn table_layout(
    f: &TableFrame,
    bank: &TileBank,
    pattern: &HashMap<u32, (String, u32)>,
    height: i32,
    palette: Option<u8>,
    unresolved: &mut usize,
) -> Layout {
    let mut out = Vec::new();
    let mut bottom = 0;
    for p in &f.pieces {
        bottom = bottom.max(p.y + 8);
    }
    for p in &f.pieces {
        let (slot, tile) = match &p.slot {
            Some(s) => (s.clone(), p.tile),
            None => match pattern.get(&p.tile) {
                Some((s, t)) => (s.clone(), *t),
                None => {
                    *unresolved += 1;
                    continue;
                }
            },
        };
        let blank = bank
            .assets
            .iter()
            .position(|a| a.slot.as_deref() == Some(&slot) || a.name == slot)
            .and_then(|a| bank.assets[a].tiles.get(tile as usize))
            .is_some_and(|px| px.iter().all(|&c| c == 0));
        if blank {
            continue;
        }
        let dy = if f.anchored { 0 } else { height - bottom };
        out.push(Piece {
            tile,
            slot: Some(slot),
            x: p.x,
            y: p.y + dy,
            palette: if f.anchored {
                p.palette
            } else {
                palette.unwrap_or(p.palette)
            },
            flip_x: p.flip_x,
            flip_y: p.flip_y,
        });
    }
    out.sort();
    out
}

/// The sprite palettes most seen in each level, as the set's palette
/// sets: levels with the same colors share one, and the most seen of all
/// is also the default.
pub fn level_sets(level_colors: &BTreeMap<String, BTreeMap<Vec<u8>, u32>>) -> Vec<PaletteSet> {
    let mut by_colors: BTreeMap<Vec<u8>, (Vec<String>, u32)> = BTreeMap::new();
    for (level, counts) in level_colors {
        if let Some((colors, n)) = counts
            .iter()
            .max_by_key(|(c, n)| (**n, std::cmp::Reverse((*c).clone())))
        {
            let e = by_colors.entry(colors.clone()).or_default();
            e.0.push(level.clone());
            e.1 += n;
        }
    }
    let mut sets: Vec<PaletteSet> = by_colors
        .iter()
        .map(|(c, (levels, _))| PaletteSet {
            levels: levels.iter().filter(|l| !l.is_empty()).cloned().collect(),
            colors: c.clone(),
        })
        .collect();
    if let Some((c, _)) = by_colors.iter().max_by_key(|(_, (_, n))| *n) {
        sets.push(PaletteSet {
            levels: Vec::new(),
            colors: c.clone(),
        });
    }
    sets.retain(|s| s.colors.len() == 16);
    sets
}

/// The middle value once sorted (the upper of the two middles), or None
/// when there are none.
pub fn median(v: &mut [u32]) -> Option<u32> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// Whether a group blinks: the frames that show nothing are at least a
/// quarter of it and come in runs of at most 4 frames between frames that
/// show something. The period is their usual run.
fn blink_of(obs: &[&Obs], layouts: &[Layout]) -> Option<u32> {
    let empty = obs
        .iter()
        .filter(|o| layouts[o.layout as usize].is_empty())
        .count();
    if empty * 4 < obs.len() || empty == obs.len() {
        return None;
    }
    let mut sorted: Vec<&&Obs> = obs.iter().collect();
    sorted.sort_by_key(|o| (o.instance, o.frame));
    let mut runs: Vec<u32> = Vec::new();
    let mut cur = 0u32;
    let mut prev: Option<(u32, u64)> = None;
    for o in sorted {
        let next = prev.is_some_and(|(i, f)| i == o.instance && o.frame == f + 1);
        if !next && cur > 0 {
            cur = 0;
        }
        if layouts[o.layout as usize].is_empty() {
            cur += 1;
        } else if cur > 0 {
            if next {
                runs.push(cur);
            }
            cur = 0;
        }
        prev = Some((o.instance, o.frame));
    }
    let every = median(&mut runs)?;
    (every <= 4).then_some(every)
}

/// One clip's worth of observations, measured.
struct Group {
    when: Cond,
    frames: Vec<u32>,
    every: Option<u32>,
    game_clock: bool,
    hold: bool,
    seen: usize,
}

fn when_of(c: &Cond) -> When {
    When {
        ground: c[0],
        moving: c[1],
        rising: c[2],
        jumped: c[3],
        skidding: c[4],
        climbing: c[5],
        invulnerable: c[6],
        ..When::default()
    }
}

/// Measures a group of observations (one state, one value of the
/// conditions): its frames in the order they follow each other, how long
/// each shows, whether changes keep to the game's clock, and whether it
/// never changes within a run (a held clip).
fn measure(
    obs: &[&Obs],
    frame_of: &dyn Fn(&Obs) -> u32,
    min_share: f64,
    min_seen: u32,
) -> (Vec<u32>, Option<u32>, bool, bool) {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for o in obs {
        *counts.entry(frame_of(o)).or_default() += 1;
    }
    let total = obs.len().max(1);
    let keep: BTreeSet<u32> = counts
        .iter()
        .filter(|(_, n)| {
            **n as f64 >= min_share * total as f64 && **n as u32 >= min_seen.min(total as u32)
        })
        .map(|(f, _)| *f)
        .collect();
    // Runs: one instance on consecutive frames.
    let mut sorted: Vec<&&Obs> = obs.iter().collect();
    sorted.sort_by_key(|o| (o.instance, o.frame));
    let mut durations = Vec::new();
    let mut follows: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut changes = 0usize;
    let mut on_clock: BTreeMap<u32, BTreeMap<u32, usize>> = BTreeMap::new();
    let mut change_clocks: Vec<u8> = Vec::new();
    let mut i = 0;
    let mut runs = 0usize;
    while i < sorted.len() {
        let mut j = i + 1;
        while j < sorted.len()
            && sorted[j].instance == sorted[i].instance
            && sorted[j].frame == sorted[j - 1].frame + 1
        {
            j += 1;
        }
        runs += 1;
        // Segments of one frame within the run.
        let run = &sorted[i..j];
        let mut segs: Vec<(u32, u32, Option<u8>)> = Vec::new();
        for o in run {
            let f = frame_of(o);
            match segs.last_mut() {
                Some((g, n, _)) if *g == f => *n += 1,
                _ => segs.push((f, 1, o.clock)),
            }
        }
        for w in segs.windows(2) {
            if keep.contains(&w[0].0) && keep.contains(&w[1].0) {
                *follows.entry((w[0].0, w[1].0)).or_default() += 1;
                changes += 1;
                if let Some(c) = w[1].2 {
                    change_clocks.push(c);
                }
            }
        }
        if segs.len() > 2 {
            for s in &segs[1..segs.len() - 1] {
                if keep.contains(&s.0) {
                    durations.push(s.1);
                }
            }
        }
        i = j;
    }
    let _ = &mut on_clock;
    // Order: start from the most seen, then the most usual next frame.
    let mut order: Vec<u32> = Vec::new();
    let mut by_count: Vec<(u32, usize)> = counts
        .iter()
        .filter(|(f, _)| keep.contains(f))
        .map(|(f, n)| (*f, *n))
        .collect();
    by_count.sort_by_key(|(f, n)| (std::cmp::Reverse(*n), *f));
    if let Some(&(first, _)) = by_count.first() {
        order.push(first);
        loop {
            let cur = *order.last().unwrap();
            let next = follows
                .iter()
                .filter(|((a, b), _)| *a == cur && !order.contains(b))
                .max_by_key(|(k, n)| (**n, std::cmp::Reverse(k.1)))
                .map(|((_, b), _)| *b);
            match next {
                Some(b) => order.push(b),
                None => break,
            }
        }
        for (f, _) in &by_count {
            if !order.contains(f) {
                order.push(*f);
            }
        }
    }
    let every = median(&mut durations);
    let hold = order.len() > 1 && changes == 0 && runs > 1;
    let game_clock = match every {
        Some(e) if e > 0 && change_clocks.len() >= 4 => {
            let mut res: BTreeMap<u32, usize> = BTreeMap::new();
            for c in &change_clocks {
                *res.entry(u32::from(*c) % e).or_default() += 1;
            }
            let top = res.values().max().copied().unwrap_or(0);
            top * 10 >= change_clocks.len() * 8
        }
        _ => false,
    };
    (order, every, game_clock, hold)
}

/// Builds the capture.
#[allow(clippy::too_many_arguments)]
pub fn build(
    model: &Model,
    config: &Config,
    tables: &Tables,
    bank: &TileBank,
    layouts: &[Layout],
    obs: &[Obs],
    level_colors: &BTreeMap<String, BTreeMap<Vec<u8>, u32>>,
    pattern: &HashMap<u32, (String, u32)>,
) -> Capture {
    let mut cap = Capture::default();
    // Each frame placed where it was most often seen from the position.
    let mut anchors: Vec<BTreeMap<[i32; 2], usize>> = vec![BTreeMap::new(); layouts.len()];
    for o in obs {
        if let Some(a) = anchors.get_mut(o.layout as usize) {
            *a.entry(o.anchor).or_default() += 1;
        }
    }
    let placed: Vec<Layout> = layouts
        .iter()
        .zip(&anchors)
        .map(|(l, a)| {
            let [x, y] = a
                .iter()
                .max_by_key(|(k, n)| (**n, std::cmp::Reverse(**k)))
                .map_or([0, 0], |(k, _)| *k);
            let mut l = l.clone();
            for p in &mut l {
                p.x += x;
                p.y += y;
            }
            l.sort();
            l
        })
        .collect();
    let layouts = &placed;
    let sets = level_sets(level_colors);
    let set_colors = |level: &str| -> Option<&Vec<u8>> {
        sets.iter()
            .find(|s| s.levels.iter().any(|l| l == level))
            .or_else(|| sets.iter().find(|s| s.levels.is_empty()))
            .map(|s| &s.colors)
    };
    let mut sb = SetBuilder {
        frames: Vec::new(),
        index: HashMap::new(),
    };
    let mut by_entity: BTreeMap<&str, Vec<&Obs>> = BTreeMap::new();
    for o in obs {
        by_entity.entry(o.entity.as_str()).or_default().push(o);
    }
    let mut names: BTreeSet<String> = by_entity.keys().map(|s| (*s).to_owned()).collect();
    names.extend(tables.of_entity.keys().cloned());
    names.retain(|n| model.entities.contains_key(n));

    for name in &names {
        let entity = &model.entities[name];
        let height = (entity.core.size[1].raw() >> 16) as i32;
        let model_states: Vec<String> = match &entity.core.machine {
            Some(m) => m.states.keys().cloned().collect(),
            None => vec!["default".into()],
        };
        let initial = entity.core.machine.as_ref().map(|m| m.initial.clone());
        let mut es = EntitySprites::default();
        let eobs: Vec<&Obs> = by_entity.get(name.as_str()).cloned().unwrap_or_default();
        es.coverage.observations = eobs.len();
        es.coverage.states = model_states.clone();

        // A slot's raw state byte, where no rule names its states: the
        // most seen value is the entity's own sprite, the rest are
        // reported for rules to name.
        // The raw state an instance is in when first seen is where it
        // starts: its own sprite.
        let mut raw: BTreeMap<&str, usize> = BTreeMap::new();
        let mut lives: Vec<&Obs> = eobs
            .iter()
            .copied()
            .filter(|o| o.state.starts_with("raw "))
            .collect();
        lives.sort_by_key(|o| (o.instance, o.frame));
        let mut prev: Option<(u32, u64)> = None;
        for o in lives {
            if prev.is_none_or(|(i, f)| i != o.instance || o.frame > f + 1) {
                *raw.entry(o.state.as_str()).or_default() += 1;
            }
            prev = Some((o.instance, o.frame));
        }
        let base_raw = raw
            .iter()
            .max_by_key(|(s, n)| (**n, std::cmp::Reverse(**s)))
            .map(|(s, _)| (*s).to_owned());
        let state_of = |o: &Obs| -> String {
            if o.state.is_empty()
                || Some(&o.state) == initial.as_ref()
                || Some(&o.state) == base_raw.as_ref()
            {
                String::new()
            } else {
                o.state.clone()
            }
        };
        let mut unmodelled: BTreeMap<String, usize> = BTreeMap::new();
        for o in &eobs {
            let s = state_of(o);
            if !s.is_empty() && !model_states.contains(&s) {
                *unmodelled.entry(s).or_default() += 1;
            }
        }
        es.coverage.unmodelled = unmodelled
            .iter()
            .map(|(s, n)| format!("{s} ({n} frames seen)"))
            .collect();

        // A state's own colors: a palette that differs from its level's
        // set in most of the state's frames.
        let mut state_colors: BTreeMap<String, Vec<Colors>> = BTreeMap::new();
        let mut per_state: BTreeMap<String, Vec<&Obs>> = BTreeMap::new();
        for o in &eobs {
            per_state.entry(state_of(o)).or_default().push(o);
        }
        let mut variants: BTreeMap<u8, BTreeSet<[u8; 4]>> = BTreeMap::new();
        for (state, list) in &per_state {
            let mut by_pal: BTreeMap<u8, (usize, BTreeMap<[u8; 4], usize>)> = BTreeMap::new();
            for o in list {
                for (p, c) in &o.colors {
                    variants.entry(*p).or_default().insert(*c);
                    let e = by_pal.entry(*p).or_default();
                    *e.1.entry(*c).or_default() += 1;
                    let level = set_colors(&o.level);
                    let at = usize::from(p - 4) * 4;
                    if level.is_some_and(|l| l.get(at + 1..at + 4) != Some(&c[1..])) {
                        e.0 += 1;
                    }
                }
            }
            let mut own = Vec::new();
            for (p, (differ, counts)) in by_pal {
                let total: usize = counts.values().sum();
                if differ * 10 >= total * 8
                    && let Some((c, _)) = counts
                        .iter()
                        .max_by_key(|(c, n)| (**n, std::cmp::Reverse(**c)))
                {
                    own.push(Colors {
                        palette: p,
                        colors: *c,
                    });
                }
            }
            if !own.is_empty() {
                state_colors.insert(state.clone(), own);
            }
        }
        es.coverage.variants = variants
            .iter()
            .filter(|(_, v)| v.len() > 1)
            .map(|(p, v)| format!("palette {p}: {} colors seen", v.len()))
            .collect();

        // Table frames of the entity and which observed frames match them.
        let mut unresolved = 0;
        let table_frames: Vec<((String, u32), Layout)> = tables
            .of_entity
            .get(name)
            .into_iter()
            .flatten()
            .filter_map(|k| {
                let f = tables.frames.get(k)?;
                Some((
                    k.clone(),
                    table_layout(
                        f,
                        bank,
                        pattern,
                        height,
                        tables.palette_of.get(name).copied(),
                        &mut unresolved,
                    ),
                ))
            })
            .collect();
        cap.unresolved_pieces += unresolved;
        let mut seen_layouts: BTreeMap<u32, usize> = BTreeMap::new();
        for o in &eobs {
            *seen_layouts.entry(o.layout).or_default() += 1;
        }
        let mut matched: BTreeMap<(String, u32), Vec<u32>> = BTreeMap::new();
        let mut observed_only = 0;
        for (l, n) in &seen_layouts {
            if (*n as u32) < config.min_seen || layouts[*l as usize].is_empty() {
                continue;
            }
            let t = tiles_of(&layouts[*l as usize]);
            let mut any = false;
            for (k, tl) in &table_frames {
                if !tl.is_empty() && tiles_of(tl) == t {
                    matched.entry(k.clone()).or_default().push(*l);
                    any = true;
                }
            }
            if !any && !table_frames.is_empty() {
                observed_only += 1;
            }
        }
        es.coverage.observed_only = observed_only;

        // Clips per state from the observations.
        let frame_id = |sb: &mut SetBuilder, state: &str, l: u32| -> u32 {
            let colors = state_colors.get(state).cloned().unwrap_or_default();
            let layout = &layouts[l as usize];
            let table = table_frames
                .iter()
                .find(|(k, _)| matched.get(k).is_some_and(|v| v.contains(&l)))
                .map(|((t, i), _)| format!("table {t} {i}"));
            let id = sb.add(name, state, "observed", layout, &colors);
            if let Some(t) = table {
                let f = &mut sb.frames[id as usize];
                if !f.from.contains(&t) {
                    f.from.push(t);
                }
            }
            id
        };
        let known: Vec<bool> = (0..CONDITIONS.len())
            .map(|i| eobs.iter().filter(|o| o.cond[i].is_some()).count() * 2 >= eobs.len().max(1))
            .collect();
        for (state, list) in &per_state {
            let mut groups: BTreeMap<Cond, Vec<&Obs>> = BTreeMap::new();
            for o in list {
                let mut c = o.cond;
                for (i, k) in known.iter().enumerate() {
                    if !k {
                        c[i] = None;
                    }
                }
                groups.entry(c).or_default().push(o);
            }
            let mut measured: Vec<Group> = Vec::new();
            for (when, g) in &groups {
                // A blink: frames that show nothing, in short runs between
                // frames that show something. It becomes the sprite's
                // blink, and the rest of the group its clip.
                let blinked = blink_of(g, layouts);
                let kept: Vec<&Obs> = match blinked {
                    Some(every) => {
                        let bw = if when[CONDITIONS.len() - 1] == Some(true) {
                            let mut c: Cond = [None; CONDITIONS.len()];
                            c[CONDITIONS.len() - 1] = Some(true);
                            c
                        } else {
                            *when
                        };
                        if es.blink.is_none() {
                            es.blink = Some((when_of(&bw), every));
                            es.coverage
                                .timings
                                .push(format!("blink: hidden every other {every} frames"));
                        }
                        g.iter()
                            .copied()
                            .filter(|o| !layouts[o.layout as usize].is_empty())
                            .collect()
                    }
                    // Not a blink: a frame that shows nothing is the entity
                    // not drawn (off the screen, or a sprite table a frame
                    // behind as a level starts over), not a frame of it.
                    None => g
                        .iter()
                        .copied()
                        .filter(|o| !layouts[o.layout as usize].is_empty())
                        .collect(),
                };
                let g = &kept;
                let (order, every, game_clock, hold) =
                    measure(g, &|o: &Obs| o.layout, 0.05, config.min_seen);
                if order.is_empty() {
                    continue;
                }
                let frames: Vec<u32> = order.iter().map(|&l| frame_id(&mut sb, state, l)).collect();
                measured.push(Group {
                    when: *when,
                    frames,
                    every,
                    game_clock,
                    hold,
                    seen: g.len(),
                });
            }
            // Groups that differ in one condition and show the same clip
            // are one clip without that condition.
            loop {
                let mut merged = false;
                'outer: for a in 0..measured.len() {
                    for b in a + 1..measured.len() {
                        let (x, y) = (&measured[a], &measured[b]);
                        let diff: Vec<usize> = (0..CONDITIONS.len())
                            .filter(|&i| x.when[i] != y.when[i])
                            .collect();
                        let same = x.frames.iter().collect::<BTreeSet<_>>()
                            == y.frames.iter().collect::<BTreeSet<_>>()
                            && x.hold == y.hold;
                        if diff.len() == 1 && same {
                            let (d, seen) = (diff[0], x.seen + y.seen);
                            let (keep, drop) = if x.seen >= y.seen { (a, b) } else { (b, a) };
                            let mut g = measured.remove(keep.max(drop));
                            let other = measured.remove(keep.min(drop));
                            if keep < drop {
                                g = other;
                            }
                            g.when[d] = None;
                            g.seen = seen;
                            measured.push(g);
                            merged = true;
                            break 'outer;
                        }
                    }
                }
                if !merged {
                    break;
                }
            }
            // The base: the most seen frame of the state.
            let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
            for o in list {
                *counts.entry(o.layout).or_default() += 1;
            }
            let Some((&base_layout, _)) = counts
                .iter()
                .filter(|(l, _)| !layouts[**l as usize].is_empty())
                .max_by_key(|(l, n)| (**n, std::cmp::Reverse(**l)))
            else {
                continue;
            };
            let base = frame_id(&mut sb, state, base_layout);
            // More conditions first, so the first that holds is the most
            // specific; one frame that is the base needs no clip.
            measured.sort_by_key(|g| {
                (
                    std::cmp::Reverse(g.when[CONDITIONS.len() - 1] == Some(true)),
                    std::cmp::Reverse(g.when.iter().filter(|c| c.is_some()).count()),
                    std::cmp::Reverse(g.seen),
                )
            });
            let mut clips = Vec::new();
            let mut seen = Vec::new();
            for g in measured {
                if g.frames == [base] {
                    continue;
                }
                // Pictures that change every frame are a transition the
                // model can't choose yet (a power-up's growth, a hit's
                // shrink), not a clip.
                if g.every == Some(1) && g.frames.len() > 1 && !g.hold {
                    es.coverage.timings.push(format!(
                        "{}: {} frames changing every frame, left out",
                        if state.is_empty() { "default" } else { state },
                        g.frames.len()
                    ));
                    continue;
                }
                let multi = g.frames.len() > 1 && !g.hold;
                let every = if multi {
                    Some(g.every.unwrap_or(8).max(1))
                } else {
                    None
                };
                if g.frames.len() > 1 || g.hold {
                    es.coverage.timings.push(format!(
                        "{}: {} frames{}{}{}",
                        if state.is_empty() { "default" } else { state },
                        g.frames.len(),
                        every.map(|e| format!(" every {e}")).unwrap_or_default(),
                        if g.game_clock && multi {
                            " on the game clock"
                        } else {
                            ""
                        },
                        if g.hold { ", held" } else { "" },
                    ));
                }
                seen.push(g.seen);
                clips.push(Clip {
                    when: when_of(&g.when),
                    frames: g
                        .frames
                        .iter()
                        .map(|&f| ClipFrame {
                            row: Some(f),
                            ..ClipFrame::default()
                        })
                        .collect(),
                    every,
                    by_speed: Vec::new(),
                    speed_before: false,
                    clock: if g.game_clock && multi {
                        Clock::Game
                    } else {
                        Clock::Own
                    },
                    while_frozen: false,
                    hold: g.hold,
                });
            }
            // Clips that only repeat the base with no condition say nothing.
            if clips.len() == 1 && clips[0].when.is_always() && clips[0].frames.len() == 1 {
                clips.clear();
                seen.clear();
            }
            es.seen.insert(state.clone(), seen);
            es.states.insert(state.clone(), (base, clips));
        }
        es.faces_both_ways =
            eobs.iter().any(|o| o.facing_left) && eobs.iter().any(|o| !o.facing_left);

        // Table frames never seen, placed as the seen ones were: by the
        // most usual move from a table frame's corner to its seen copy's.
        let corner = |l: &Layout| {
            [
                l.iter().map(|p| p.x).min().unwrap_or(0),
                l.iter().map(|p| p.y).min().unwrap_or(0),
            ]
        };
        let mut moves: BTreeMap<[i32; 2], usize> = BTreeMap::new();
        for (k, tl) in &table_frames {
            for &l in matched.get(k).into_iter().flatten() {
                let (a, b) = (corner(tl), corner(&layouts[l as usize]));
                *moves.entry([b[0] - a[0], b[1] - a[1]]).or_default() += 1;
            }
        }
        let shift = moves
            .iter()
            .max_by_key(|(k, n)| (**n, std::cmp::Reverse(**k)))
            .map_or([0, 0], |(k, _)| *k);
        let table_frames: Vec<((String, u32), Layout)> = table_frames
            .into_iter()
            .map(|(k, mut tl)| {
                for p in &mut tl {
                    p.x += shift[0];
                    p.y += shift[1];
                }
                (k, tl)
            })
            .collect();
        for (k, tl) in &table_frames {
            if tl.is_empty() {
                continue;
            }
            if !matched.contains_key(k) {
                es.coverage.unobserved.push(format!("{} {}", k.0, k.1));
                sb.add(name, "", &format!("table {} {}", k.0, k.1), tl, &[]);
            }
        }
        if !es.states.contains_key("") {
            let table_id = |sb: &mut SetBuilder, k: &(String, u32)| -> Option<u32> {
                let (_, tl) = table_frames.iter().find(|(kk, _)| kk == k)?;
                if tl.is_empty() {
                    return None;
                }
                Some(sb.add(name, "", &format!("table {} {}", k.0, k.1), tl, &[]))
            };
            let seq = tables
                .sequences_of
                .get(name)
                .and_then(|v| v.first())
                .map(|&i| &tables.sequences[i]);
            if let Some(s) = seq {
                let mut frames = Vec::new();
                for &st in &s.steps {
                    if let Some(id) = table_id(&mut sb, &(s.frames.clone(), st))
                        && frames.last() != Some(&id)
                    {
                        frames.push(id);
                    }
                }
                if let Some(&first) = frames.first() {
                    let clips = if frames.len() > 1 {
                        vec![Clip {
                            when: When::default(),
                            frames: frames
                                .iter()
                                .map(|&f| ClipFrame {
                                    row: Some(f),
                                    ..ClipFrame::default()
                                })
                                .collect(),
                            every: Some(s.every.max(1)),
                            by_speed: Vec::new(),
                            speed_before: false,
                            clock: Clock::Own,
                            while_frozen: false,
                            hold: false,
                        }]
                    } else {
                        Vec::new()
                    };
                    es.coverage.timings.push(format!(
                        "default: {} frames every {} (table {})",
                        frames.len(),
                        s.every,
                        s.table
                    ));
                    es.states.insert(String::new(), (first, clips));
                }
            } else if let Some((k, _)) = table_frames.iter().find(|(_, tl)| !tl.is_empty())
                && let Some(id) = table_id(&mut sb, k)
            {
                es.states.insert(String::new(), (id, Vec::new()));
            }
        }
        let mut covered: Vec<String> = Vec::new();
        for s in &model_states {
            let key = if Some(s) == initial.as_ref() || s == "default" {
                ""
            } else {
                s.as_str()
            };
            if es.states.contains_key(key) {
                covered.push(s.clone());
            }
        }
        es.coverage.covered = covered;
        es.coverage.frames = sb.frames.iter().filter(|f| f.entity == *name).count();
        cap.entities.insert(name.clone(), es);
    }
    cap.set = SpriteSet {
        schema: gp_assets::sprite_set::SCHEMA,
        asset: config.asset.clone(),
        sets,
        frames: sb.frames,
    };
    cap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(instance: u32, frame: u64, layout: u32, clock: u8) -> Obs {
        Obs {
            frame,
            instance,
            entity: "e".into(),
            state: String::new(),
            cond: [None; CONDITIONS.len()],
            layout,
            anchor: [0, 0],
            facing_left: false,
            colors: Vec::new(),
            level: String::new(),
            clock: Some(clock),
        }
    }

    #[test]
    fn a_walk_cycle_is_ordered_and_timed_on_the_game_clock() {
        // Frames 0, 1, 2 in turn, each for 4 frames, changing when the
        // clock is a multiple of 4; two instances.
        let mut all = Vec::new();
        for inst in 0..2 {
            for f in 0..60u64 {
                let c = f as u8 + inst as u8 * 4;
                all.push(obs(inst, f, ((u32::from(c) / 4) % 3 + 7) % 3, c));
            }
        }
        let refs: Vec<&Obs> = all.iter().collect();
        let (order, every, game, hold) = measure(&refs, &|o| o.layout, 0.05, 2);
        assert_eq!(every, Some(4));
        assert!(game);
        assert!(!hold);
        assert_eq!(order.len(), 3);
        // The order follows the cycle from wherever it starts.
        let start = order[0];
        assert_eq!(order, [start, (start + 1) % 3, (start + 2) % 3]);
    }

    #[test]
    fn frames_that_never_change_within_a_run_are_held() {
        let mut all = Vec::new();
        // Three separate runs, each showing one frame throughout.
        for (k, l) in [0u32, 1, 2].iter().enumerate() {
            for f in 0..10u64 {
                all.push(obs(0, k as u64 * 100 + f, *l, 0));
            }
        }
        let refs: Vec<&Obs> = all.iter().collect();
        let (order, every, _, hold) = measure(&refs, &|o| o.layout, 0.05, 2);
        assert_eq!(order.len(), 3);
        assert_eq!(every, None);
        assert!(hold);
    }

    #[test]
    fn level_sets_share_colors_and_name_a_default() {
        let mut lc: BTreeMap<String, BTreeMap<Vec<u8>, u32>> = BTreeMap::new();
        let a: Vec<u8> = (0..16).collect();
        let b: Vec<u8> = (16..32).collect();
        lc.entry("one".into()).or_default().insert(a.clone(), 10);
        lc.entry("two".into()).or_default().insert(a.clone(), 5);
        lc.entry("three".into()).or_default().insert(b.clone(), 3);
        let sets = level_sets(&lc);
        assert_eq!(sets.len(), 3);
        assert_eq!(sets[0].levels, ["one", "two"]);
        assert_eq!(sets[2].levels.len(), 0);
        assert_eq!(sets[2].colors, a);
    }
}
