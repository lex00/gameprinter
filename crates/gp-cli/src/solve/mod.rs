//! Deterministic solvers (docs/research/deterministic-inference.md,
//! section 4): detectors that read a clip's scored outcome, the original's
//! and the model's series per measure, and name the number that would
//! close the gap, with no search and no LLM.
//!
//! | Solver | Reads | Proposes |
//! |---|---|---|
//! | `arith` ([`arith`]) | the diff from the first wrong frame: a constant, a linear drift, a lag, a sign flip, every N, a 256 wrap | a speed, acceleration or offset off by the diff's exact fraction; `every` and `phase`; a `_wrap` |
//! | `timers` ([`timers`]) | a count's change points in both games, their period and phase, the value a reload sets, the frame count modulo N | a timer's period, reload or start; `align`, `every` and `phase` |
//! | `bodies`, `states` ([`missing`], `gp solve --missing`) | a body one game has alone; a value the original takes that the model never does in the run; a split on a state's entry | nothing: they say what is missing, for the question |
//!
//! Both are pure functions of the outcome. A proposal names its edits, the
//! exact values and the evidence in a sentence. `gp solve` prints them;
//! the refine loop tries them after its early-stop detectors
//! (`refine_lockstep.rs`) with the same check a fitted value passes, and
//! a number cited to the game's code is asked about, never written.
//!
//! Ported from game-printer whole. `gp solve` reads a game's corpus
//! scored into a run (`gp corpus score --run`) instead of an old capture
//! run ([`cli`]), and [`code_cited`] is kept here, since the refinement
//! loop it came from is ported apart.

pub mod arith;
pub(crate) mod cli;

pub use cli::SolveArgs;
pub(crate) use cli::run;
pub mod missing;
pub mod params;
pub mod timers;

use serde::Serialize;

use gp_interp::mechanics::Catalog;
use gp_interp::scenario::Outcome;
use gp_model::Model;

use arith::{ONE, fraction};
pub use params::Edit;

/// One measure's two series, raw 16.16, and where it is wrong.
#[derive(Debug, Clone)]
pub struct Measure {
    pub path: String,
    pub owners: Vec<String>,
    pub original: Vec<Option<i64>>,
    pub model: Vec<Option<i64>>,
    /// The first index whose distance is past the tolerance.
    pub first: Option<usize>,
}

/// A clip's outcome as the solvers read it.
#[derive(Debug, Clone)]
pub struct Clip {
    pub id: String,
    pub name: String,
    /// The original frame of index 0.
    pub frame0: u64,
    pub measures: Vec<Measure>,
}

fn raw(s: &str) -> Option<i64> {
    let f: f64 = s.trim().parse().ok()?;
    Some((f * ONE as f64).round() as i64)
}

impl Clip {
    /// From a scored outcome, whose measures carry both series.
    pub fn of(o: &Outcome, name: &str) -> Self {
        let measures = o
            .measures
            .iter()
            .map(|m| {
                let tol = gp_model::Fixed::parse(&m.tolerance).map_or(0, |f| f.raw());
                let first = m
                    .distance
                    .iter()
                    .position(|d| gp_model::Fixed::parse(d).is_ok_and(|d| d.raw() > tol));
                Measure {
                    path: m.path.clone(),
                    owners: m.owners.clone(),
                    original: m.original.iter().map(|s| raw(s)).collect(),
                    model: m.model.iter().map(|s| raw(s)).collect(),
                    first,
                }
            })
            .collect();
        Self {
            id: o.id.clone(),
            name: name.to_owned(),
            frame0: o.original_from,
            measures,
        }
    }
}

/// What a detector read on one measure.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// `arith` or `timers`.
    pub solver: &'static str,
    pub class: &'static str,
    pub measure: String,
    /// The clip's frame it is first wrong on.
    pub at: usize,
    /// The class's numbers.
    pub read: serde_json::Value,
    pub says: String,
    #[serde(skip)]
    kind: Kind,
}

#[derive(Debug, Clone)]
enum Kind {
    Arith(arith::Class),
    Timer(timers::Class),
}

fn px(raw: i64) -> String {
    fraction(raw)
}

fn whole(raw: i64) -> String {
    (raw / ONE).to_string()
}

/// The detectors on every wrong measure, the earliest first.
pub fn findings(clip: &Clip) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut order: Vec<&Measure> = clip.measures.iter().filter(|m| m.first.is_some()).collect();
    order.sort_by_key(|m| m.first);
    for m in order {
        let at = m.first.unwrap_or(0);
        let words = gp_interp::lockstep::words(&m.path);
        let field = m.path.rsplit('.').next().unwrap_or("");
        let o: Vec<i64> = m.original.iter().map(|v| v.unwrap_or(0)).collect();
        let mv: Vec<i64> = m.model.iter().map(|v| v.unwrap_or(0)).collect();
        let numbers = m.original.iter().chain(&m.model).all(Option::is_some);
        let flag01 = o.iter().chain(&mv).all(|v| *v == 0 || *v == ONE);
        if numbers
            && !flag01
            && timers::count_like(&m.path, &o, &mv)
            && let Some(c) = timers::classify(&o, &mv, at, clip.frame0)
        {
            let says = match &c {
                timers::Class::Start { original, model } => format!(
                    "{words}: wrong on the clip's first frame (original {}, rebuilt {}): a start field",
                    whole(*original),
                    whole(*model)
                ),
                timers::Class::Period { original, model } => format!(
                    "{words}: steps every {original} frames in the original and every {model} in the rebuilt game"
                ),
                timers::Class::Reload { original, model } => format!(
                    "{words}: a reload sets {} in the original and {} in the rebuilt game",
                    whole(*original),
                    whole(*model)
                ),
                timers::Class::Aligned { every, phase } => format!(
                    "{words}: the original steps only on frames whose count is {phase} modulo {every}, the rebuilt game on others"
                ),
                timers::Class::Stops {
                    rebuilt,
                    value,
                    other,
                } => format!(
                    "{words}: the {} count stops at {} from frame {at} while the other goes on to {}",
                    if *rebuilt { "rebuilt" } else { "original" },
                    whole(*value),
                    whole(*other)
                ),
                timers::Class::Shift { frames } => format!(
                    "{words}: the rebuilt count is the original's {} frame{} {} from frame {at}",
                    frames.abs(),
                    if frames.abs() == 1 { "" } else { "s" },
                    if *frames > 0 { "late" } else { "early" }
                ),
            };
            out.push(Finding {
                solver: "timers",
                class: c.name(),
                measure: m.path.clone(),
                at,
                read: serde_json::to_value(&c).unwrap_or_default(),
                says,
                kind: Kind::Timer(c),
            });
            continue;
        }
        let flag = m
            .original
            .iter()
            .chain(&m.model)
            .all(|v| v.is_none_or(|v| v == 0 || v == ONE));
        let kind = if flag {
            arith::Kind::Flag
        } else if numbers && timers::count_like(&m.path, &o, &mv) {
            arith::Kind::Count
        } else if field.starts_with('v') {
            arith::Kind::Speed
        } else {
            arith::Kind::Place
        };
        let Some(r) = arith::classify(&m.original, &m.model, at, clip.frame0, kind) else {
            continue;
        };
        let says = match &r.class {
            arith::Class::Constant { by, from_start } => format!(
                "{words}: the rebuilt value is {} off on every frame from frame {at}{}",
                px(*by),
                if *from_start {
                    ", the clip's start"
                } else {
                    ""
                }
            ),
            arith::Class::Linear { rate, denominator } => format!(
                "{words}: the diff grows by {} a frame from frame {at} (a rate off by {}, denominator {denominator})",
                px(*rate),
                fraction(*rate)
            ),
            arith::Class::Lag { frames } => format!(
                "{words}: the rebuilt value is the original's from a frame {} from frame {at}",
                if *frames > 0 {
                    "before (a frame late)"
                } else {
                    "after (a frame early)"
                }
            ),
            arith::Class::SignFlip { original, model } => format!(
                "{words}: the rebuilt value is the original's turned the other way from frame {at} (original {}, rebuilt {})",
                fraction(*original),
                fraction(*model)
            ),
            arith::Class::EveryN { every, phase, step } => format!(
                "{words}: the diff steps by {} on one frame in {every} only (phase {phase}) from frame {at}: a gate modulo {every}",
                px(*step)
            ),
            arith::Class::Stops { rebuilt, value } => format!(
                "{words}: the {} value stays at {} from frame {at} while the {}'s goes on",
                if *rebuilt { "rebuilt" } else { "original" },
                fraction(*value),
                if *rebuilt { "original" } else { "rebuilt game" }
            ),
            arith::Class::Start { original, model } => format!(
                "{words}: wrong from the clip's start (original {}, rebuilt {}) with no shape: a start field the translation doesn't carry",
                fraction(*original),
                fraction(*model)
            ),
            arith::Class::Wrap { by } => format!(
                "{words}: the rebuilt value is {} off, a multiple of 256: a low byte compared or a wrapped place",
                px(*by)
            ),
        };
        out.push(Finding {
            solver: "arith",
            class: r.class.name(),
            measure: m.path.clone(),
            at,
            read: serde_json::to_value(&r).unwrap_or_default(),
            says,
            kind: Kind::Arith(r.class),
        });
    }
    out
}

/// A change a solver proposes.
#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    /// `solver:arith` or `solver:timers`, as the ledger records it.
    pub by: String,
    pub class: &'static str,
    pub measure: String,
    pub at: usize,
    pub mechanic: String,
    pub edits: Vec<Edit>,
    /// The finding and the change, in a sentence.
    pub evidence: String,
    /// Where the game's code gives a number it changes: never written,
    /// asked about instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<String>>,
}

/// At most this many proposals a clip.
pub const MOST: usize = 24;

/// Proposals come from this many of the earliest findings.
pub const FIRST: usize = 3;

/// The proposals for `findings`, from the measures' owners' numbers in
/// `model`.
pub fn propose(
    findings: &[Finding],
    model: &Model,
    catalog: &Catalog,
    clip: &Clip,
) -> Vec<Proposal> {
    let tree = model.to_tree();
    let mut out: Vec<Proposal> = Vec::new();
    // The first few measures wrong: the refine loop works on the first
    // wrong frame, and a later measure often follows from it.
    for f in findings.iter().take(FIRST) {
        let Some(m) = clip.measures.iter().find(|m| m.path == f.measure) else {
            continue;
        };
        let part = params::part_of(&m.path);
        let nums = params::numbers(catalog, &tree, &m.owners, &part);
        let field = m.path.rsplit('.').next().unwrap_or("");
        let cands = match &f.kind {
            Kind::Arith(c) => params::for_arith(c, field, &nums),
            Kind::Timer(c) => params::for_timer(c, &nums),
        };
        for edits in cands {
            if out.iter().any(|p| p.edits == edits) {
                continue;
            }
            let mechanic = nums
                .iter()
                .find(|n| n.path == edits[0].path)
                .map_or(String::new(), |n| n.mechanic.clone());
            let change = edits
                .iter()
                .map(|e| {
                    format!(
                        "{} {} -> {}",
                        e.path,
                        e.old.as_deref().unwrap_or("(unset)"),
                        e.new
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let mut code: Vec<String> = Vec::new();
            for e in &edits {
                if e.old.is_some()
                    && let Some(c) = code_cited(model, &e.path)
                {
                    code.extend(c);
                }
            }
            code.dedup();
            out.push(Proposal {
                by: format!("solver:{}", f.solver),
                class: f.class,
                measure: f.measure.clone(),
                at: f.at,
                mechanic,
                evidence: format!("{}; so {change}", f.says),
                edits,
                code: (!code.is_empty()).then_some(code),
            });
            if out.len() >= MOST {
                return out;
            }
        }
    }
    out
}

/// The findings and proposals for one scored clip.
#[derive(Debug, Clone, Serialize)]
pub struct Solved {
    pub clip: String,
    pub name: String,
    pub findings: Vec<Finding>,
    pub proposals: Vec<Proposal>,
}

pub fn solve(o: &Outcome, name: &str, model: &Model, catalog: &Catalog) -> Solved {
    let clip = Clip::of(o, name);
    let findings = findings(&clip);
    let proposals = propose(&findings, model, catalog, &clip);
    Solved {
        clip: clip.id,
        name: clip.name,
        findings,
        proposals,
    }
}

/// The model with `edits` made.
pub fn apply(model: &Model, edits: &[Edit]) -> anyhow::Result<Model> {
    let mut tree = model.to_tree();
    for e in edits {
        let p = gp_model::Path::parse(&e.path).map_err(anyhow::Error::msg)?;
        let (last, parents) = p
            .segments
            .split_last()
            .ok_or_else(|| anyhow::anyhow!("an empty path"))?;
        let mut node = &mut tree;
        for s in parents {
            node = node
                .get_mut(s.name.as_str())
                .ok_or_else(|| anyhow::anyhow!("{}: no {}", e.path, s.name))?;
            if let Some(gp_model::path::Index::At(i)) = s.index {
                node = usize::try_from(i)
                    .ok()
                    .and_then(|i| node.get_mut(i))
                    .ok_or_else(|| anyhow::anyhow!("{}: no item {i}", e.path))?;
            }
        }
        let t = node
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("{}: not in a table", e.path))?;
        let v = match e.new.parse::<i64>() {
            Ok(i) => toml::Value::Integer(i),
            Err(_) => toml::Value::String(e.new.clone()),
        };
        t.insert(last.name.clone(), v);
    }
    let text = toml::to_string(&tree)?;
    Model::parse(&text).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Whether the refine loop tries the solvers (docs/interpreter.md,
/// "Refining from the lockstep"). On unless `GP_REFINE_SOLVE` is `off`.
/// For the refinement loop, which is ported apart.
#[allow(dead_code)]
pub fn on() -> bool {
    std::env::var("GP_REFINE_SOLVE").map_or(true, |v| v != "off")
}

/// The code the model cites for a number at `path` or a part above it:
/// a `stated` evidence entry's refs to lines of the game's code. A fact
/// from the code outranks a fit from measurement, so a proposal on it is a
/// question. (game-printer's `refine_lockstep::code_cited`.)
pub(crate) fn code_cited(model: &Model, path: &str) -> Option<Vec<String>> {
    let mut p = path.to_owned();
    let mut cites: Vec<String> = Vec::new();
    loop {
        if let Some(e) = model.evidence.get(&p)
            && e.source == gp_model::model::Source::Stated
        {
            cites.extend(e.refs.iter().filter_map(code_ref));
        }
        // Up one part: `a.b[1].c` -> `a.b[1]` -> `a.b` -> `a`.
        if let Some(i) = p.rfind('[')
            && p.ends_with(']')
        {
            p.truncate(i);
        } else if let Some((head, _)) = p.rsplit_once('.') {
            p = head.to_owned();
        } else {
            break;
        }
    }
    (!cites.is_empty()).then_some(cites)
}

/// A ref to a line of the game's code, in words: `games/mosshop/mosshop.s:31`,
/// `SMBDIS.ASM:6033 (smbdis@7e8e9777)`.
fn code_ref(r: &gp_model::model::Ref) -> Option<String> {
    let line = r.at.rsplit_once(':').is_some_and(|(f, n)| {
        f.contains('.') && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
    });
    match &r.within {
        Some(w) => Some(format!("{} ({w})", r.at)),
        None if line => Some(r.at.clone()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(path: &str, o: &[i64], m: &[i64], tol: i64) -> Measure {
        let first = o.iter().zip(m).position(|(a, b)| (a - b).abs() > tol);
        Measure {
            path: path.into(),
            owners: Vec::new(),
            original: o.iter().map(|v| Some(*v)).collect(),
            model: m.iter().map(|v| Some(*v)).collect(),
            first,
        }
    }

    #[test]
    fn the_earliest_wrong_measure_comes_first_and_each_gets_its_detector() {
        let x_o: Vec<i64> = (0..20).map(|i| i * ONE).collect();
        let x_m: Vec<i64> = (0..20).map(|i| i * ONE + i * ONE / 2).collect();
        let hold_o: Vec<i64> = [0, 0, 0, 0, 3, 2, 1, 0, 0, 0]
            .iter()
            .map(|v| v * ONE)
            .collect();
        let hold_m: Vec<i64> = [0, 0, 0, 0, 4, 3, 2, 1, 0, 0]
            .iter()
            .map(|v| v * ONE)
            .collect();
        let clip = Clip {
            id: "c".into(),
            name: "c".into(),
            frame0: 0,
            measures: vec![
                measure("cameras.main.hold", &hold_o, &hold_m, 0),
                measure("entities.frog[0].x", &x_o, &x_m, 0),
            ],
        };
        let f = findings(&clip);
        assert_eq!(f[0].measure, "entities.frog[0].x");
        assert_eq!(f[0].class, "linear");
        assert!(f[0].says.contains("grows by 1/2 a frame"), "{}", f[0].says);
        assert_eq!(f[1].solver, "timers");
        assert_eq!(f[1].class, "reload");
    }
}
