//! Coverage (docs/research/clip-corpus.md, section 5): which clips move
//! each number and enter each mechanic state, the pairs of numbers no clip
//! tells apart, the minimal set, the gaps, and the background queue's
//! order.
//!
//! A clip excites a number when changing the number changes the model's
//! mapped measures in that clip, measured by a perturbation (the number
//! doubled and halved) from the clip's start. Two clips are independent
//! when their patterns differ or their contexts do: two holds of 16 and 32
//! frames from the same start move a walking speed the same way, so a
//! number that one fools fools the other.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{Cell, Clip, Context, Status};

/// One number and the clips that move it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ParamCover {
    pub path: String,
    pub mechanic: String,
    pub clips: Vec<String>,
    /// The most clips among them that are pairwise independent.
    pub independent: usize,
    /// Moved by at least two independent clips.
    pub covered: bool,
}

/// One mechanic state and the clips that enter it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StateCover {
    pub state: String,
    pub clips: Vec<String>,
}

/// Two numbers of a mechanic some clip moves together, and the clips that
/// tell them apart: one that moves one without the other, or two that
/// move both in a different ratio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PairCover {
    pub a: String,
    pub b: String,
    /// Clips that move both.
    pub together: usize,
    /// Up to three clips that move one and not the other, or else two
    /// that move both in ratios more than a quarter apart.
    pub separated_by: Vec<String>,
}

/// What the corpus doesn't cover, in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Gap {
    /// `param`, `pair`, `context`, `state` or `probe`.
    pub kind: String,
    pub what: String,
    pub says: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Coverage {
    pub params: Vec<ParamCover>,
    pub states: Vec<StateCover>,
    pub pairs: Vec<PairCover>,
    /// The minimal set's clips, in the order they were chosen.
    pub minimal: Vec<String>,
    pub gaps: Vec<Gap>,
    /// "26 of 30 numbers moved by two independent clips; minimal set 24
    /// clips; 5 gaps".
    pub says: String,
}

/// What a clip is, for independence: its pattern and its context.
fn family(cells: &BTreeMap<&str, &Cell>, clip: &Clip) -> (String, String) {
    cells
        .get(clip.cell.as_str())
        .map(|c| (c.pattern.name.clone(), c.context.clone()))
        .unwrap_or_default()
}

/// The most pairwise independent clips among `ids`, greedily in order.
fn independent(fam: &BTreeMap<String, (String, String)>, ids: &[String]) -> usize {
    let mut kept: Vec<&(String, String)> = Vec::new();
    for id in ids {
        let Some(f) = fam.get(id) else { continue };
        if kept.iter().all(|k| k.0 != f.0 || k.1 != f.1) {
            kept.push(f);
        }
    }
    kept.len()
}

/// Whether clip `c` counts as a new independent exciter of a number whose
/// chosen exciters are `chosen`.
fn adds(fam: &BTreeMap<String, (String, String)>, chosen: &[String], c: &str) -> bool {
    let Some(f) = fam.get(c) else { return false };
    chosen
        .iter()
        .filter_map(|x| fam.get(x))
        .all(|k| k.0 != f.0 || k.1 != f.1)
}

/// The coverage of `clips` over `params` (path, mechanic).
pub fn compute(
    params: &[(String, String)],
    cells: &[Cell],
    clips: &[Clip],
    contexts: &[Context],
) -> Coverage {
    let by_cell: BTreeMap<&str, &Cell> = cells.iter().map(|c| (c.id.as_str(), c)).collect();
    // Clips that can be fitted from: active and checks, not probes.
    let usable: Vec<&Clip> = clips
        .iter()
        .filter(|c| matches!(c.status, Status::Active | Status::Check))
        .collect();
    let fam: BTreeMap<String, (String, String)> = usable
        .iter()
        .map(|c| (c.id.clone(), family(&by_cell, c)))
        .collect();
    let exciters = |p: &str| -> Vec<String> {
        usable
            .iter()
            .filter(|c| c.excites.iter().any(|e| e == p))
            .map(|c| c.id.clone())
            .collect()
    };
    let mut out = Coverage::default();
    for (p, m) in params {
        let ids = exciters(p);
        let ind = independent(&fam, &ids);
        out.params.push(ParamCover {
            path: p.clone(),
            mechanic: m.clone(),
            clips: ids,
            independent: ind,
            covered: ind >= 2,
        });
    }
    let mut states: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in &usable {
        for s in &c.enters {
            states.entry(s.clone()).or_default().push(c.id.clone());
        }
    }
    out.states = states
        .into_iter()
        .map(|(state, clips)| StateCover { state, clips })
        .collect();
    // Pairs of a mechanic's numbers some clip moves together.
    for (i, (a, ma)) in params.iter().enumerate() {
        for (b, mb) in &params[i + 1..] {
            if ma != mb {
                continue;
            }
            let both = usable
                .iter()
                .filter(|c| c.excites.contains(a) && c.excites.contains(b))
                .count();
            if both == 0 {
                continue;
            }
            let mut sep: Vec<String> = usable
                .iter()
                .filter(|c| c.excites.contains(a) != c.excites.contains(b))
                .map(|c| c.id.clone())
                .take(3)
                .collect();
            if sep.is_empty() {
                'find: for (i, c) in usable.iter().enumerate() {
                    for d in &usable[i + 1..] {
                        if apart(c, d, a, b) {
                            sep = vec![c.id.clone(), d.id.clone()];
                            break 'find;
                        }
                    }
                }
            }
            out.pairs.push(PairCover {
                a: a.clone(),
                b: b.clone(),
                together: both,
                separated_by: sep,
            });
        }
    }
    out.minimal = minimal(params, &usable, &fam, &out.pairs);
    // Gaps.
    for p in &out.params {
        if p.clips.is_empty() {
            out.gaps.push(Gap {
                kind: "param".into(),
                what: p.path.clone(),
                says: format!(
                    "no clip moves {}: changing it changes nothing in any clip",
                    p.path
                ),
            });
        } else if !p.covered {
            out.gaps.push(Gap {
                kind: "param".into(),
                what: p.path.clone(),
                says: format!(
                    "only one independent clip moves {} ({}): a wrong value can hide behind another number",
                    p.path,
                    p.clips.len()
                ),
            });
        }
    }
    for pc in &out.pairs {
        if pc.separated_by.is_empty() {
            out.gaps.push(Gap {
                kind: "pair".into(),
                what: format!("{} {}", pc.a, pc.b),
                says: format!(
                    "no clip moves {} without {}, or the two in another ratio: they can make up for each other",
                    pc.a, pc.b
                ),
            });
        }
    }
    for c in contexts {
        if let Some(why) = &c.missing {
            out.gaps.push(Gap {
                kind: "context".into(),
                what: c.id.clone(),
                says: format!("no start {}: {why}", c.words),
            });
        }
    }
    for cell in cells.iter().filter(|c| c.status == Status::Probe) {
        if !clips.iter().any(|c| c.cell == cell.id) {
            continue;
        }
        out.gaps.push(Gap {
            kind: "probe".into(),
            what: cell.id.clone(),
            says: format!("probe: {}", cell.name),
        });
    }
    let covered = out.params.iter().filter(|p| p.covered).count();
    let n = |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
    out.says = format!(
        "{covered} of {} moved by two independent clips; minimal set {}; {}",
        n(out.params.len(), "number", "numbers"),
        n(out.minimal.len(), "clip", "clips"),
        n(
            out.gaps.iter().filter(|g| g.kind != "probe").count(),
            "gap",
            "gaps"
        )
    );
    out
}

/// Whether two clips that both move `a` and `b` move them in ratios more
/// than a quarter apart.
fn apart(c: &Clip, d: &Clip, a: &str, b: &str) -> bool {
    let (Some(ca), Some(cb), Some(da), Some(db)) = (
        c.effect_of(a),
        c.effect_of(b),
        d.effect_of(a),
        d.effect_of(b),
    ) else {
        return false;
    };
    // ca/cb against da/db, without dividing.
    let (x, y) = (ca * db, cb * da);
    let (hi, lo) = (x.max(y), x.min(y));
    4 * hi > 5 * lo
}

/// Whether clip `c` tells `pc` apart, alone or with one of `chosen`.
fn tells(c: &Clip, pc: &PairCover, chosen: &[&Clip]) -> bool {
    let (ia, ib) = (c.excites.contains(&pc.a), c.excites.contains(&pc.b));
    if ia != ib {
        return true;
    }
    ia && chosen.iter().any(|d| apart(c, d, &pc.a, &pc.b))
}

/// The minimal set: a greedy cover of every number by two independent
/// clips and of every pair some clip moves together by one clip that
/// moves one of them alone. The clip that covers the most still uncovered
/// takes each turn, ties by ID, so it is the same every time.
fn minimal(
    params: &[(String, String)],
    usable: &[&Clip],
    fam: &BTreeMap<String, (String, String)>,
    pairs: &[PairCover],
) -> Vec<String> {
    let mut chosen: Vec<String> = Vec::new();
    let mut per: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut separated: BTreeSet<(String, String)> = BTreeSet::new();
    let need_pairs: Vec<&PairCover> = pairs
        .iter()
        .filter(|p| !p.separated_by.is_empty())
        .collect();
    let mut ids: Vec<&&Clip> = usable.iter().collect();
    ids.sort_by(|a, b| a.id.cmp(&b.id));
    loop {
        let mut best: Option<(usize, &str)> = None;
        for c in &ids {
            if chosen.contains(&c.id) {
                continue;
            }
            let mut gain = 0;
            for (p, _) in params {
                if !c.excites.contains(p) {
                    continue;
                }
                let have = per.get(p.as_str()).map_or(&[][..], Vec::as_slice);
                if independent(fam, have) < 2 && adds(fam, have, &c.id) {
                    gain += 1;
                }
            }
            let picked: Vec<&Clip> = chosen
                .iter()
                .filter_map(|id| usable.iter().find(|x| x.id == *id).copied())
                .collect();
            for pc in &need_pairs {
                if !separated.contains(&(pc.a.clone(), pc.b.clone())) && tells(c, pc, &picked) {
                    gain += 1;
                }
            }
            if gain > 0 && best.is_none_or(|(g, _)| gain > g) {
                best = Some((gain, c.id.as_str()));
            }
        }
        let Some((_, id)) = best else { break };
        let c = usable.iter().find(|c| c.id == id).expect("a clip");
        for (p, _) in params {
            if c.excites.contains(p) {
                per.entry(p.as_str()).or_default().push(c.id.clone());
            }
        }
        let picked: Vec<&Clip> = chosen
            .iter()
            .filter_map(|id| usable.iter().find(|x| x.id == *id).copied())
            .collect();
        for pc in &need_pairs {
            if tells(c, pc, &picked) {
                separated.insert((pc.a.clone(), pc.b.clone()));
            }
        }
        chosen.push(c.id.clone());
    }
    chosen
}

/// What the queue knows of a clip so far.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Seen {
    pub passing: bool,
    /// The clip's first wrong frame, when it has one.
    pub first_wrong: Option<u64>,
}

/// The order the background queue takes clips in (docs/research/
/// clip-corpus.md, section 7, "Ranking for the background queue"):
/// mapping checks first, then starred clips, then the minimal set, then
/// the rest, probes last; within each, the clips whose numbers the fewest
/// passing clips move, then those not passing yet, then the earliest first
/// wrong frame, then the ID.
/// A skipped clip leaves the list.
pub fn rank(
    clips: &[Clip],
    cells: &[Cell],
    marks: &BTreeMap<String, String>,
    seen: &BTreeMap<String, Seen>,
) -> Vec<String> {
    let by_cell: BTreeMap<&str, &Cell> = cells.iter().map(|c| (c.id.as_str(), c)).collect();
    let passing = |p: &str| {
        clips
            .iter()
            .filter(|c| c.excites.iter().any(|e| e == p))
            .filter(|c| seen.get(&c.id).is_some_and(|s| s.passing))
            .count()
    };
    type Key = (u8, usize, bool, u64);
    let mut keyed: Vec<(Key, &str)> = clips
        .iter()
        .filter(|c| marks.get(&c.id).is_none_or(|m| m != "skipped"))
        .map(|c| {
            let check = by_cell
                .get(c.cell.as_str())
                .is_some_and(|x| x.mechanic == "check.mapping");
            let group = if check {
                0
            } else if c.status == Status::Probe {
                4
            } else if marks.get(&c.id).is_some_and(|m| m == "starred") {
                1
            } else if c.minimal {
                2
            } else {
                3
            };
            let thin = c
                .excites
                .iter()
                .map(|p| passing(p))
                .min()
                .unwrap_or(usize::MAX);
            let s = seen.get(&c.id);
            let first = s.and_then(|s| s.first_wrong).unwrap_or(u64::MAX);
            (
                (group, thin, s.is_some_and(|s| s.passing), first),
                c.id.as_str(),
            )
        })
        .collect();
    keyed.sort();
    keyed.into_iter().map(|(_, id)| id.to_owned()).collect()
}
