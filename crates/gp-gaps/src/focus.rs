//! A window of frames a person marked (docs/show.md): what the original
//! did in it that the model doesn't account for. The detectors run on the
//! whole recording, so "rare" and "unexplained" mean what they mean
//! there; this keeps what falls in the window, and ranks routines and RAM
//! by how much likelier they are in the window than elsewhere in play.
//! Detect only, like the rest of the crate.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::changes::runtime_cells;
use crate::interactive::{Catalog, Interaction, kind_key};
use crate::sound::Request;
use crate::{Run, r2, sig, top};

/// Frames after the window that its effects are read to: counters settle
/// (the interactive cataloger reads them 12 frames on).
pub const SETTLE: u64 = 12;

/// How close, in pixels, a sprite object is to a mapped object for the
/// mapped object to explain it.
const NEAR: i32 = 24;

/// The most items kept per place.
const KEEP: usize = 12;

/// A routine or RAM group that's active far more often in the window than
/// in the rest of play.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Rare {
    pub name: String,
    /// Frames of the window it was active on.
    pub frames: u64,
    /// The share of the rest of play's frames it was active on.
    pub elsewhere: f64,
}

/// An interaction in the window, with the catalog kind it belongs to.
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub kind: String,
    /// How many interactions of the kind the whole recording has.
    pub kind_count: usize,
    pub interaction: Interaction,
}

/// Background cells in the window that changed one way.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Cells {
    pub before: String,
    pub after: String,
    pub count: u64,
    /// `[frame, world x, world y]`.
    pub samples: Vec<[i64; 3]>,
}

/// Sprite objects in the window near no mapped object, grouped by palette
/// and first tiles.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Sprites {
    pub name: String,
    pub frames: u64,
    pub first: u64,
}

/// A counter's value before the window and once it settled.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Counter {
    pub name: String,
    pub before: i64,
    pub after: i64,
}

/// What the original did in frames `from` to `to` (both included).
#[derive(Debug, Clone, Serialize)]
pub struct Focus {
    pub from: u64,
    pub to: u64,
    /// Frames of the window in play.
    pub play_frames: usize,
    pub interactions: Vec<Hit>,
    pub cells: Vec<Cells>,
    pub requests: Vec<Request>,
    pub counters: Vec<Counter>,
    pub sprites: Vec<Sprites>,
    /// Routines the model cites no name of that ran in the window and are
    /// rare elsewhere.
    pub routines: Vec<Rare>,
    /// RAM the model doesn't name that changed in the window and rarely
    /// elsewhere.
    pub ram: Vec<Rare>,
    pub has_code: bool,
}

/// Everything the whole-recording detectors found that a window reads.
pub struct Found<'a> {
    pub catalog: &'a Catalog,
    pub interactions: &'a [Interaction],
    pub requests: &'a [Request],
}

/// The share of frames outside the window, in play, a predicate holds on;
/// `in_window` counts the window's.
fn rarity(run: &Run, win: (usize, usize), active: impl Fn(usize) -> bool) -> (u64, f64) {
    let (mut inside, mut outside, mut out_play) = (0u64, 0u64, 0u64);
    for (i, f) in run.frames.iter().enumerate() {
        let w = i >= win.0 && i <= win.1;
        if w {
            inside += u64::from(active(i));
        } else if f.play {
            out_play += 1;
            outside += u64::from(active(i));
        }
    }
    let share = if out_play == 0 {
        0.0
    } else {
        outside as f64 / out_play as f64
    };
    (inside, share)
}

/// Keeps the items active on a window frame, rarest elsewhere first. An
/// item is kept when it's at least four times likelier on a window frame
/// than on another play frame, or never active elsewhere.
fn rare_ranked(mut v: Vec<Rare>, window_frames: usize) -> Vec<Rare> {
    let w = window_frames.max(1) as f64;
    v.retain(|r| r.frames > 0 && (r.elsewhere == 0.0 || r.frames as f64 / w >= 4.0 * r.elsewhere));
    v.sort_by(|a, b| {
        a.elsewhere
            .total_cmp(&b.elsewhere)
            .then(b.frames.cmp(&a.frames))
            .then(a.name.cmp(&b.name))
    });
    v.truncate(KEEP);
    v
}

/// The window's findings. `from` and `to` are frame numbers; frames
/// outside the recording are left out.
pub fn window(run: &Run, found: &Found<'_>, from: u64, to: u64) -> Focus {
    let at = |frame: u64| run.frames.partition_point(|f| f.frame < frame);
    let (a, b) = (at(from), at(to + 1));
    let frames = &run.frames[a..b];
    let win = (a, b.saturating_sub(1));
    let k = &run.known;

    // Interactions and requests the whole-run detectors placed in it.
    let kinds: BTreeMap<&str, usize> = found
        .catalog
        .kinds
        .iter()
        .map(|kd| (kd.kind.as_str(), kd.count))
        .collect();
    let interactions = found
        .interactions
        .iter()
        .filter(|it| it.frame >= from && it.frame <= to)
        .map(|it| {
            let kind = kind_key(it);
            Hit {
                kind_count: kinds.get(kind.as_str()).copied().unwrap_or(1),
                kind,
                interaction: it.clone(),
            }
        })
        .collect();
    let requests = found
        .requests
        .iter()
        .filter(|r| r.frame >= from && r.frame <= to)
        .cloned()
        .collect();

    // Cell changes.
    let mut cells: BTreeMap<(String, String), Cells> = BTreeMap::new();
    for c in runtime_cells(run)
        .into_iter()
        .filter(|c| c.frame >= from && c.frame <= to)
    {
        let e = cells
            .entry((sig(&c.before), sig(&c.after)))
            .or_insert_with(|| Cells {
                before: sig(&c.before),
                after: sig(&c.after),
                count: 0,
                samples: Vec::new(),
            });
        e.count += 1;
        if e.samples.len() < 5 {
            e.samples
                .push([c.frame as i64, i64::from(c.wx), i64::from(c.wy)]);
        }
    }
    let mut cells: Vec<Cells> = cells.into_values().collect();
    cells.sort_by(|x, y| y.count.cmp(&x.count).then(x.before.cmp(&y.before)));

    // Counters, from the frame before the window to SETTLE frames after.
    let before = run.frames.get(a.saturating_sub(1));
    let after = run
        .frames
        .get((at(to + SETTLE + 1)).saturating_sub(1).max(a));
    let counters = match (before, after) {
        (Some(x), Some(y)) => run
            .counters
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                let (p, q) = (*x.counters.get(i)?, *y.counters.get(i)?);
                (p != q).then(|| Counter {
                    name: name.clone(),
                    before: p,
                    after: q,
                })
            })
            .collect(),
        _ => Vec::new(),
    };

    // Sprite objects no mapped object is near.
    let mut sprites: BTreeMap<String, Sprites> = BTreeMap::new();
    for f in frames.iter().filter(|f| f.play) {
        for o in &f.objects {
            let (cx, cy) = (
                (i32::from(o.c.x0) + i32::from(o.c.x1)) / 2,
                (i32::from(o.c.y0) + i32::from(o.c.y1)) / 2,
            );
            let near = f.ents.iter().any(|e| {
                let (ex, ey) = (e.x - f.camera[0], e.y - f.camera[1]);
                (ex - cx).abs() + (ey - cy).abs() <= NEAR * 2
            });
            if near {
                continue;
            }
            let tiles: Vec<String> =
                o.c.tiles
                    .iter()
                    .take(2)
                    .map(|t| format!("{t:02x}"))
                    .collect();
            let name = format!("palette {} tiles {}", o.c.palette, tiles.join(" "));
            let e = sprites.entry(name.clone()).or_insert(Sprites {
                name,
                frames: 0,
                first: f.frame,
            });
            e.frames += 1;
        }
    }
    let mut sprites: Vec<Sprites> = sprites.into_values().collect();
    sprites.sort_by(|x, y| y.frames.cmp(&x.frames).then(x.name.cmp(&y.name)));
    sprites.truncate(KEEP);

    // Routines and RAM, by rarity outside the window.
    let mut routine_ids: BTreeMap<u32, ()> = BTreeMap::new();
    let mut ram_groups: BTreeMap<String, Vec<u16>> = BTreeMap::new();
    let oam_lo = u16::from(k.oam_page) << 8;
    for f in frames {
        if let Some(c) = &f.code {
            for (r, _) in &c.entered {
                if !k.cites_routine(*r) {
                    routine_ids.insert(*r, ());
                }
            }
        }
        for &addr in &f.ram_changed {
            let addr = addr & 0x7FF;
            if (0x100..0x200).contains(&addr)
                || (oam_lo..oam_lo + 0x100).contains(&addr)
                || k.explains_ram(addr)
            {
                continue;
            }
            let (name, _) = k.ram_name(addr);
            let base = name
                .split_once('+')
                .map_or(name.clone(), |(b, _)| b.to_owned());
            let g = ram_groups.entry(base).or_default();
            if !g.contains(&addr) {
                g.push(addr);
            }
        }
    }
    let routines = routine_ids
        .keys()
        .map(|&r| {
            let (frames, elsewhere) = rarity(run, win, |i| {
                run.frames[i]
                    .code
                    .as_ref()
                    .is_some_and(|c| c.entered.iter().any(|(e, _)| *e == r))
            });
            Rare {
                name: k.routine_name(r),
                frames,
                elsewhere: r2(elsewhere * 1000.0) / 1000.0,
            }
        })
        .collect();
    let ram = ram_groups
        .into_iter()
        .map(|(name, addrs)| {
            let (frames, elsewhere) = rarity(run, win, |i| {
                run.frames[i]
                    .ram_changed
                    .iter()
                    .any(|a| addrs.contains(&(a & 0x7FF)))
            });
            Rare {
                name,
                frames,
                elsewhere: r2(elsewhere * 1000.0) / 1000.0,
            }
        })
        .collect();

    Focus {
        from,
        to,
        play_frames: frames.iter().filter(|f| f.play).count(),
        interactions,
        cells,
        requests,
        counters,
        sprites,
        routines: rare_ranked(routines, frames.len()),
        ram: rare_ranked(ram, frames.len()),
        has_code: frames.iter().any(|f| f.code.is_some()),
    }
}

/// Where the player was, as words: `, the player below it`, or nothing
/// for an interaction with no cell.
pub fn where_player(relation: &str) -> String {
    match relation {
        "below" | "above" | "inside" => format!(", the player {relation} it"),
        "left" | "right" => format!(", the player {relation} of it"),
        "away" => ", the player away from it".into(),
        _ => String::new(),
    }
}

impl Focus {
    /// One line per finding, for a person to read.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for h in &self.interactions {
            let it = &h.interaction;
            out.push(format!(
                "frame {}: {} ({} in the recording){}{}{}",
                it.frame,
                h.kind,
                h.kind_count,
                where_player(&it.relation),
                if it.requests.is_empty() {
                    String::new()
                } else {
                    format!(", asking for {}", it.requests.join(", "))
                },
                it.primary
                    .as_ref()
                    .map(|p| format!(", handled by {p}"))
                    .unwrap_or_default()
            ));
        }
        for c in &self.cells {
            out.push(format!(
                "cells {} -> {} changed {} times",
                c.before, c.after, c.count
            ));
        }
        for r in &self.requests {
            out.push(format!(
                "frame {}: sound {} through {} (from {})",
                r.frame, r.name, r.via, r.requester
            ));
        }
        for c in &self.counters {
            out.push(format!(
                "counter {} {} -> {} ({:+})",
                c.name,
                c.before,
                c.after,
                c.after - c.before
            ));
        }
        for s in &self.sprites {
            out.push(format!(
                "sprite object {} on {} frames from {}, near no mapped object",
                s.name, s.frames, s.first
            ));
        }
        let rare = |v: &[Rare]| -> String {
            v.iter()
                .map(|r| {
                    format!(
                        "{} ({}, {:.1}% elsewhere)",
                        r.name,
                        r.frames,
                        r.elsewhere * 100.0
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !self.routines.is_empty() {
            out.push(format!("routines: {}", rare(&self.routines)));
        }
        if !self.ram.is_empty() {
            out.push(format!("RAM: {}", rare(&self.ram)));
        }
        out
    }
}

/// Counts as `(name, count)`, largest first, for summaries.
pub fn counts<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(String, u64)> {
    let mut m: BTreeMap<String, u64> = BTreeMap::new();
    for n in names {
        *m.entry(n.to_owned()).or_default() += 1;
    }
    top(&m, KEEP)
}

/// A window's interactions split into the answer to a question about one
/// spot at one frame and what happened nearby.
#[derive(Debug, Clone, Default)]
pub struct Attributed<'a> {
    pub answer: Vec<&'a Hit>,
    /// The rest within the margin, nearest the asked frame first.
    pub nearby: Vec<&'a Hit>,
}

/// Which of a window's interactions answer a question about box `region`
/// (world pixels `[x, y, w, h]`) at frame `frame`. The answer is the
/// interactions of the cells inside the box itself, at the frame nearest
/// `frame` (and others within two frames of that one, a block and the coin
/// it gives); with no cell in the box, the cells within `margin` pixels of
/// it; with no cell at all, the interaction nearest in time. Everything
/// else within the margin is nearby, nearest in time first. Detect only:
/// this reorders what the window found.
pub fn attribute<'a>(
    f: &'a Focus,
    region: Option<[i64; 4]>,
    margin: i64,
    frame: u64,
) -> Attributed<'a> {
    let overlaps = |h: &Hit, m: i64| {
        let a = h.interaction.at;
        let (x, y) = (i64::from(a[0]), i64::from(a[1]));
        region.is_none_or(|r| {
            x < r[0] + r[2] + m && r[0] - m < x + 16 && y < r[1] + r[3] + m && r[1] - m < y + 16
        })
    };
    let is_cell = |h: &Hit| h.interaction.found_by == "cell";
    let apart = |h: &Hit| h.interaction.frame.abs_diff(frame);
    let mut pool: Vec<&Hit> = f
        .interactions
        .iter()
        .filter(|h| is_cell(h) && overlaps(h, 0))
        .collect();
    if pool.is_empty() {
        pool = f
            .interactions
            .iter()
            .filter(|h| is_cell(h) && overlaps(h, margin))
            .collect();
    }
    if pool.is_empty() {
        pool = f.interactions.iter().filter(|h| !is_cell(h)).collect();
    }
    let mut answer = Vec::new();
    if let Some(best) = pool.iter().map(|h| apart(h)).min() {
        let at = pool
            .iter()
            .find(|h| apart(h) == best)
            .map_or(frame, |h| h.interaction.frame);
        answer = pool
            .into_iter()
            .filter(|h| h.interaction.frame.abs_diff(at) <= 2)
            .collect();
    }
    let mut nearby: Vec<&Hit> = f
        .interactions
        .iter()
        .filter(|h| !answer.iter().any(|a| std::ptr::eq(*a, *h)))
        .filter(|h| !is_cell(h) || overlaps(h, margin))
        .collect();
    nearby.sort_by_key(|h| (apart(h), !is_cell(h)));
    Attributed { answer, nearby }
}
