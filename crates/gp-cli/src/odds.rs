//! `gp interp odds`: the random-choice behaviour check (docs/benchmarks.md,
//! "Random choices"). A clip's score hands the rebuild the original's
//! draws; this check holds the rule itself.
//!
//! The rule is read from the model, not written down: at each draw the
//! model makes in a free run (its own generator, the mapping's
//! `[[draws]]` site names), the frame is run again from the state before
//! it with the draw set to each of the 256 byte values, and the bytes are
//! grouped by the state they lead to. Each group is a branch the rule can
//! take there, and its share of the 256 bytes is the rule's odds for it
//! (a ladder's `byte & 0x88 == 0` is 64 of 256, 1 in 4). The draw's own
//! branch is the group whose state the free run reached.
//!
//! Decisions with the same groups are pooled, and the model's count of
//! each branch is tested against the rule's odds with a chi-square test:
//! a pool fails when its p-value is below [`P_FAIL`]. A pool with an
//! expected count under [`MIN_EXPECTED`] is reported but not tested.
//! With `--rom`, the original's draws over each clip (from its start) are
//! put in the same groups and tested the same way, and the rebuild, given
//! them in a resync lockstep, must take each one: a desync at a site is a
//! draw the original made where the rule makes none, or the reverse.
//!
//! Ported from game-printer's odds.rs. Changed by gameprinter's layout: the
//! model runs through [`Source`], its hooks in gp or in the game's module
//! (`--module`, whose `set-draw-script`, `draw-calls` and `world-hash` calls
//! the free run uses); the original starts through [`OriginalRuntime`];
//! `--clips` takes a game's package folder (its corpus) as well as a clip
//! set file; the clips run on the shared pool (`gp --jobs`), not a `--jobs`
//! of their own; and there is no run recorder.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_evidence::{InputLog, Paths};
use gp_runtime::{InputFrame, Runtime};
use gp_sim::DrawScript;
use gp_sim::draws::Draw;
use serde_json::{Value, json};

use crate::Outcome;
use crate::hosted::{Model, Source};
use crate::original::OriginalRuntime;

/// A pool fails when the chance of counts this far from the rule's odds,
/// were the model's draws the rule's, is below this.
pub(crate) const P_FAIL: f64 = 0.001;

/// The least expected count of every branch for a chi-square test.
pub(crate) const MIN_EXPECTED: f64 = 5.0;

#[derive(clap::Args)]
pub struct OddsArgs {
    /// A model directory holding model.toml and mapping.toml.
    pub model: PathBuf,
    /// Clip sets (clips.toml), or games' package folders (their corpus),
    /// whose clips the model plays, each from power-on.
    #[arg(long, value_name = "FILE|DIR", required = true)]
    pub clips: Vec<PathBuf>,
    /// Only these clips, by ID (comma-separated).
    #[arg(long, value_delimiter = ',')]
    pub only: Vec<String>,
    /// The original game's ROM: the model then plays each clip's frames as
    /// the state mapping lines them up, and the original's draws are
    /// tested too.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The model's lifted assets (default: $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
}

/// The 256 byte values split into the branches they lead to, each a
/// 256-bit set, in the order of their lowest byte.
type Groups = Vec<[u64; 4]>;

fn has(set: &[u64; 4], v: u8) -> bool {
    set[usize::from(v >> 6)] >> (v & 63) & 1 == 1
}

fn size(set: &[u64; 4]) -> u32 {
    set.iter().map(|w| w.count_ones()).sum()
}

/// The bits a byte's branch depends on, and each branch's values of them:
/// `& 0x88: 0x00 | 0x08, 0x80, 0x88`.
pub(crate) fn describe(groups: &Groups) -> (u8, Vec<Vec<u8>>) {
    let of = |v: u8| groups.iter().position(|g| has(g, v));
    let mask = (0..8u8)
        .map(|b| 1u8 << b)
        .filter(|&bit| (0..=255u8).any(|v| of(v) != of(v ^ bit)))
        .fold(0u8, |m, b| m | b);
    let values = groups
        .iter()
        .map(|g| {
            let mut vs: Vec<u8> = (0..=255u8)
                .filter(|&v| has(g, v))
                .map(|v| v & mask)
                .collect();
            vs.sort_unstable();
            vs.dedup();
            vs
        })
        .collect();
    (mask, values)
}

fn groups_text(groups: &Groups) -> String {
    let (mask, values) = describe(groups);
    let branches: Vec<String> = values
        .iter()
        .zip(groups)
        .map(|(vs, g)| {
            let v: Vec<String> = vs.iter().map(|v| format!("{v:#04x}")).collect();
            format!("{} ({}/256)", v.join(","), size(g))
        })
        .collect();
    format!("byte & {mask:#04x}: {}", branches.join(" | "))
}

/// One pool: the decisions at a site with the same branches, and how
/// many took each.
#[derive(Debug, Clone, Default)]
pub(crate) struct Pool {
    pub decisions: u64,
    pub counts: Vec<u64>,
}

/// The chi-square test of `counts` against the shares `of_256`: the
/// statistic, its degrees of freedom and the p-value, or None when an
/// expected count is under [`MIN_EXPECTED`].
#[cfg(test)]
pub(crate) fn chi_square(counts: &[u64], of_256: &[u32]) -> Option<(f64, usize, f64)> {
    let odds: Vec<f64> = of_256.iter().map(|&s| f64::from(s) / 256.0).collect();
    chi_square_odds(counts, &odds)
}

/// [`chi_square`] against each branch's chance, the chances summing to 1.
pub(crate) fn chi_square_odds(counts: &[u64], odds: &[f64]) -> Option<(f64, usize, f64)> {
    let n: u64 = counts.iter().sum();
    let expected: Vec<f64> = odds.iter().map(|&o| n as f64 * o).collect();
    if counts.len() < 2 || expected.iter().any(|&e| e < MIN_EXPECTED) {
        return None;
    }
    let stat: f64 = counts
        .iter()
        .zip(&expected)
        .map(|(&c, &e)| (c as f64 - e).powi(2) / e)
        .sum();
    let dof = counts.len() - 1;
    Some((stat, dof, gamma_q(dof as f64 / 2.0, stat / 2.0)))
}

/// Whether two sets of counts over the same branches come from the same
/// odds (chi-square on the 2 by k table): the statistic, its degrees of
/// freedom and the p-value, or None when an expected count is under
/// [`MIN_EXPECTED`]. Branches neither set takes are left out.
pub(crate) fn chi_square_same(a: &[u64], b: &[u64]) -> Option<(f64, usize, f64)> {
    let cols: Vec<(u64, u64)> = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| (x, y))
        .filter(|(x, y)| x + y > 0)
        .collect();
    let (na, nb): (u64, u64) = (
        cols.iter().map(|c| c.0).sum(),
        cols.iter().map(|c| c.1).sum(),
    );
    let n = (na + nb) as f64;
    if cols.len() < 2 || na == 0 || nb == 0 {
        return None;
    }
    let mut stat = 0.0;
    for (x, y) in &cols {
        let col = (x + y) as f64;
        for (obs, row) in [(*x, na), (*y, nb)] {
            let e = row as f64 * col / n;
            if e < MIN_EXPECTED {
                return None;
            }
            stat += (obs as f64 - e).powi(2) / e;
        }
    }
    let dof = cols.len() - 1;
    Some((stat, dof, gamma_q(dof as f64 / 2.0, stat / 2.0)))
}

/// Each branch's chance under a byte distribution: the share of `weights`
/// (a count for each byte value) its bytes hold, or its share of the 256
/// values when there are no weights.
pub(crate) fn odds_of(groups: &Groups, weights: Option<&[u64; 256]>) -> Vec<f64> {
    match weights.filter(|w| w.iter().any(|&n| n > 0)) {
        None => groups.iter().map(|g| f64::from(size(g)) / 256.0).collect(),
        Some(w) => {
            let total: u64 = w.iter().sum();
            groups
                .iter()
                .map(|g| {
                    let n: u64 = (0..=255u8)
                        .filter(|&v| has(g, v))
                        .map(|v| w[usize::from(v)])
                        .sum();
                    n as f64 / total as f64
                })
                .collect()
        }
    }
}

/// The natural log of the gamma function (Lanczos, g = 7).
fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let t = x + 7.5;
    let a = C[1..]
        .iter()
        .enumerate()
        .fold(C[0], |a, (i, c)| a + c / (x + i as f64 + 1.0));
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// The regularized upper incomplete gamma function Q(a, x): the
/// chi-square test's p-value for `a` = dof / 2 and `x` = statistic / 2.
fn gamma_q(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    let ln_front = -x + a * x.ln() - ln_gamma(a);
    if x < a + 1.0 {
        // The series for P, then 1 - P.
        let (mut sum, mut term, mut n) = (1.0 / a, 1.0 / a, a);
        for _ in 0..500 {
            n += 1.0;
            term *= x / n;
            sum += term;
            if term.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        (1.0 - sum * ln_front.exp()).max(0.0)
    } else {
        // The continued fraction for Q (modified Lentz).
        let tiny = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / tiny;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..500 {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < tiny {
                d = tiny;
            }
            c = b + an / c;
            if c.abs() < tiny {
                c = tiny;
            }
            d = 1.0 / d;
            let delta = d * c;
            h *= delta;
            if (delta - 1.0).abs() < 1e-15 {
                break;
            }
        }
        (ln_front.exp() * h).min(1.0)
    }
}

/// What a clip's free run of the model drew.
#[derive(Default)]
struct ClipRun {
    /// (site, groups, branch taken) for each decision.
    decisions: Vec<(String, Groups, usize)>,
    /// Draws not tested, by site, and why.
    untested: BTreeMap<(String, &'static str), u64>,
}

/// Runs the model on `frames` with its own generator and, at each draw,
/// reads the rule's branches there by setting the draw to every byte.
fn free_run(start: Model, frames: &[InputFrame]) -> Result<ClipRun> {
    let mut out = ClipRun::default();
    let mut rt = start.with_draw_script(DrawScript {
        calls: Some(Vec::new()),
        ..DrawScript::default()
    })?;
    let mut seen = 0;
    for input in frames {
        let before = rt.fork();
        rt.step_unhurried(std::slice::from_ref(input))?;
        let calls = rt.draw_calls()?;
        if calls.len() == seen {
            continue;
        }
        let made: Vec<(u64, String)> = calls[seen..].to_vec();
        seen = calls.len();
        let reached = rt.world_hash()?;
        for (k, (frame, site)) in made.iter().enumerate() {
            // A forcing script sets a site's first draw of the frame.
            if made[..k].iter().any(|(_, s)| s == site) {
                *out.untested
                    .entry((site.clone(), "a second draw at the site in its frame"))
                    .or_default() += 1;
                continue;
            }
            let mut by_state: Vec<([u8; 32], [u64; 4])> = Vec::new();
            for v in 0..=255u8 {
                let mut f = before.fork().with_draw_script(DrawScript {
                    draws: vec![Draw {
                        frame: *frame,
                        site: site.clone(),
                        value: v,
                    }],
                    origin: Some(0),
                    lenient: true,
                    ..DrawScript::default()
                })?;
                f.step_unhurried(std::slice::from_ref(input))?;
                let h = f.world_hash()?;
                let i = match by_state.iter().position(|(s, _)| *s == h) {
                    Some(i) => i,
                    None => {
                        by_state.push((h, [0; 4]));
                        by_state.len() - 1
                    }
                };
                by_state[i].1[usize::from(v >> 6)] |= 1 << (v & 63);
            }
            match by_state.iter().position(|(s, _)| *s == reached) {
                Some(i) => {
                    out.decisions
                        .push((site.clone(), by_state.iter().map(|g| g.1).collect(), i))
                }
                None => {
                    *out.untested
                        .entry((
                            site.clone(),
                            "the free run reached none of the set draws' states",
                        ))
                        .or_default() += 1;
                }
            }
        }
    }
    Ok(out)
}

/// The model's frames for an input of the original game: the comparison
/// `gp interp compare` makes, through the mapping's start and alignment,
/// gives each model frame the original frame it stands for, and the model
/// frame's input is the input that led to that original frame.
fn aligned_frames(
    rom: &[u8],
    mapping: &gp_interp::mapping::Mapping,
    source: &Source,
    log: &InputLog,
) -> Result<Vec<InputFrame>> {
    let inputs = crate::run::frames(log);
    let mut original = OriginalRuntime::of_rom(rom)?;
    let mut model = source.open()?;
    let report = gp_interp::compare::compare(&mut original, &mut model, mapping, &inputs)
        .map_err(anyhow::Error::msg)?;
    let mut out = Vec::new();
    for row in report.rows.iter().skip(1) {
        let i = row
            .original_frame
            .checked_sub(1)
            .context("a model frame stands for the original's power-on")? as usize;
        out.push(pads_of(&inputs[i])?);
    }
    if out.is_empty() {
        bail!(
            "the model never started: the mapping's start didn't hold, or the input ended first (is it the model's ROM revision?)"
        );
    }
    if out.len() as u64 != report.model_frames {
        bail!(
            "the alignment gave {} model frames, but the comparison ran {}",
            out.len(),
            report.model_frames
        );
    }
    Ok(out)
}

/// An input a model reads: controllers 1 and 2 and the reset.
fn pads_of(f: &InputFrame) -> Result<InputFrame> {
    if f.buttons > 0xFFFF {
        bail!("a model reads controllers 1 and 2 only");
    }
    Ok(InputFrame {
        buttons: f.buttons,
        axes: Vec::new(),
        reset: f.reset,
    })
}

/// The clips' inputs from power-on, each with its start frame. A folder
/// is a game's package folder, read as its corpus; a file is a clip set.
fn clips_of(a: &OddsArgs) -> Result<Vec<(String, InputLog, u64)>> {
    let mut out = Vec::new();
    for f in &a.clips {
        let (set, dir) = if f.is_dir() {
            (crate::clipset::read_game(f)?, f.to_owned())
        } else {
            (
                crate::clipset::read(f)?,
                f.parent().unwrap_or(Path::new(".")).to_owned(),
            )
        };
        for c in &set.clips {
            if !a.only.is_empty() && !a.only.contains(&c.id) {
                continue;
            }
            let Some((log, _)) = crate::clipset::recipe_log(&dir, c)? else {
                bail!(
                    "{}: clip {} starts from a recording or its presses file is missing",
                    f.display(),
                    c.id
                );
            };
            out.push((c.id.clone(), log, c.start.frame));
        }
    }
    if out.is_empty() {
        bail!("no clip to run");
    }
    Ok(out)
}

pub(crate) fn run(a: &OddsArgs, paths: &Paths) -> Result<Outcome> {
    let (source, _) =
        crate::interp_args::source(&a.model, a.assets.as_deref(), a.module.as_deref(), paths)?;
    let model_dir = crate::interp_args::model_dir(&a.model);
    let (_, mapping) = crate::interp_args::load_mapping(&a.model, None)?;
    if mapping.draws.is_empty() {
        bail!("the state mapping names no `[[draws]]` sites");
    }
    let rom = match &a.rom {
        Some(r) => Some(fs::read(r).with_context(|| format!("reading {}", r.display()))?),
        None => None,
    };
    let clips = clips_of(a)?;
    // Each clip on the pool's threads, each with a run of the model of its own.
    type Run = (ClipRun, Vec<Draw>, Value);
    let results: Vec<Result<Run>> = crate::pool::map(
        &clips,
        || (),
        |_, _, (_, log, from)| {
            let frames = match &rom {
                Some(rom) => aligned_frames(rom, &mapping, &source, log)?,
                None => crate::run::frames(log)
                    .iter()
                    .map(pads_of)
                    .collect::<Result<_>>()?,
            };
            let run = free_run(source.open()?, &frames)?;
            let (orig, taken) = match &rom {
                Some(rom) => {
                    let d = crate::lockstep::record_draws(rom, &mapping, &log.frames, *from)?;
                    let (_, j, _) = crate::lockstep::run_resumed(
                        &source,
                        &model_dir,
                        &mapping,
                        rom,
                        &crate::run::frames(log),
                        gp_interp::lockstep::Mode::Resync,
                        Some(*from),
                        true,
                        None,
                        Some(d.clone()),
                        None,
                        None,
                    )?;
                    (d.draws, j["draws"].clone())
                }
                None => (Vec::new(), Value::Null),
            };
            Ok((run, orig, taken))
        },
        &mut |_| {},
    );

    // Pools by site and branches.
    let mut pools: BTreeMap<(String, Groups), Pool> = BTreeMap::new();
    let mut no_choice: BTreeMap<String, u64> = BTreeMap::new();
    let mut untested: BTreeMap<(String, &'static str), u64> = BTreeMap::new();
    let mut original: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut desyncs: BTreeMap<String, Vec<(String, u64, String)>> = BTreeMap::new();
    let mut scripts = (0u64, 0u64);
    let mut errors = Vec::new();
    for ((id, _, _), r) in clips.iter().zip(results) {
        let (run, orig, taken) = match r {
            Ok(x) => x,
            Err(e) => {
                errors.push(format!("{id}: {e:#}"));
                continue;
            }
        };
        for (site, groups, i) in run.decisions {
            if groups.len() < 2 {
                *no_choice.entry(site).or_default() += 1;
                continue;
            }
            let p = pools.entry((site, groups.clone())).or_default();
            p.counts.resize(groups.len(), 0);
            p.decisions += 1;
            p.counts[i] += 1;
        }
        for (k, n) in run.untested {
            *untested.entry(k).or_default() += n;
        }
        for d in orig {
            original.entry(d.site).or_default().push(d.value);
        }
        if let Some(t) = taken.as_object() {
            scripts.0 += t.get("taken").and_then(Value::as_u64).unwrap_or(0);
            scripts.1 += t.get("script").and_then(Value::as_u64).unwrap_or(0);
            for d in t
                .get("desyncs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let site = d["site"].as_str().unwrap_or("").to_owned();
                desyncs.entry(site).or_default().push((
                    id.clone(),
                    d["original_frame"].as_u64().unwrap_or(0),
                    d["why"].as_str().unwrap_or("").to_owned(),
                ));
            }
        }
    }

    let mut lines = Vec::new();
    let mut sites_j = Vec::new();
    let mut failed = 0usize;
    for site in mapping.draws.iter().map(|d| d.site.as_str()) {
        let mine: Vec<(&Groups, &Pool)> = pools
            .iter()
            .filter(|((s, _), _)| s == site)
            .map(|((_, g), p)| (g, p))
            .collect();
        let decisions: u64 = mine.iter().map(|(_, p)| p.decisions).sum();
        let none = no_choice.get(site).copied().unwrap_or(0);
        lines.push(format!(
            "{site}: {decisions} decisions with a choice, {none} with one branch only"
        ));
        let mut pools_j = Vec::new();
        // The generator's real output at the site: the bytes the original
        // drew there. Donkey Kong's register ($18-$1F, stepped by the idle
        // loop) doesn't give the 256 values evenly, so even the original's
        // own draws failed against an even split (roll_off p 0.0005,
        // fire_way p 0.0004).
        let weights: Option<[u64; 256]> = original.get(site).map(|bytes| {
            let mut w = [0u64; 256];
            for &v in bytes {
                w[usize::from(v)] += 1;
            }
            w
        });
        for (g, p) in &mine {
            let shares: Vec<u32> = g.iter().map(size).collect();
            let odds = odds_of(g, weights.as_ref());
            let test = chi_square_odds(&p.counts, &odds);
            let verdict = match test {
                Some((_, _, pv)) if pv < P_FAIL => {
                    failed += 1;
                    "FAIL"
                }
                Some(_) => "ok",
                None => "too few to test",
            };
            let expect: Vec<String> = odds
                .iter()
                .map(|&o| format!("{:.1}", p.decisions as f64 * o))
                .collect();
            let counts: Vec<String> = p.counts.iter().map(u64::to_string).collect();
            lines.push(format!(
                "  {}: {} decisions, taken {} where the rule expects {}{}{}: {verdict}",
                groups_text(g),
                p.decisions,
                counts.join("/"),
                expect.join("/"),
                if weights.is_some() {
                    " under the original's bytes"
                } else {
                    " (an even byte: no ROM)"
                },
                test.map_or(String::new(), |(c, d, pv)| format!(
                    " (chi-square {c:.2}, {d} dof, p {pv:.4})"
                )),
            ));
            pools_j.push(json!({
                "branches": groups_text(g),
                "shares_of_256": shares,
                "odds": odds,
                "decisions": p.decisions,
                "counts": p.counts,
                "chi_square": test.map(|t| t.0),
                "dof": test.map(|t| t.1),
                "p": test.map(|t| t.2),
                "verdict": verdict,
            }));
        }
        for ((s, why), n) in &untested {
            if s == site {
                lines.push(format!("  {n} draws not tested: {why}"));
            }
        }
        // The original's draws in the site's most common branches.
        let mut orig_j = Value::Null;
        if let Some(bytes) = original.get(site)
            && let Some((g, _)) = mine.iter().max_by_key(|(_, p)| p.decisions)
        {
            let counts: Vec<u64> = g
                .iter()
                .map(|set| bytes.iter().filter(|&&v| has(set, v)).count() as u64)
                .collect();
            // The original's branches against the model's own in the same
            // pool: whether the two games choose with the same odds. Its
            // bytes give the odds above, so they can't be tested by them.
            let model_counts = &mine
                .iter()
                .max_by_key(|(_, p)| p.decisions)
                .expect("a pool")
                .1
                .counts;
            let test = chi_square_same(&counts, model_counts);
            let verdict = match test {
                Some((_, _, pv)) if pv < P_FAIL => {
                    failed += 1;
                    "FAIL"
                }
                Some(_) => "ok",
                None => "too few to test",
            };
            let shown: Vec<String> = counts.iter().map(u64::to_string).collect();
            lines.push(format!(
                "  the original: {} draws, {} in those branches, against the model's {}{}: {verdict}",
                bytes.len(),
                shown.join("/"),
                model_counts.iter().map(u64::to_string).collect::<Vec<_>>().join("/"),
                test.map_or(String::new(), |(c, d, pv)| format!(
                    " (chi-square {c:.2}, {d} dof, p {pv:.4})"
                )),
            ));
            orig_j = json!({"draws": bytes.len(), "counts": counts, "p": test.map(|t| t.2), "verdict": verdict});
        }
        let ds = desyncs.get(site).cloned().unwrap_or_default();
        if rom.is_some() {
            if ds.is_empty() {
                lines.push("  the rebuild took each of the original's draws here".into());
            } else {
                failed += 1;
                let (id, f, why) = &ds[0];
                lines.push(format!(
                    "  {} desyncs: draws the original made where the rule makes none, or the reverse; first {id} at the original's frame {f}: {why}: FAIL",
                    ds.len()
                ));
            }
        }
        sites_j.push(json!({
            "site": site,
            "decisions": decisions,
            "one_branch": none,
            "pools": pools_j,
            "original": orig_j,
            "desyncs": ds.iter().map(|(id, f, why)| json!({"clip": id, "original_frame": f, "why": why})).collect::<Vec<_>>(),
        }));
    }
    if rom.is_some() {
        lines.push(format!(
            "the rebuild took {} of the original's {} draws in resync locksteps",
            scripts.0, scripts.1
        ));
    }
    for e in &errors {
        lines.push(format!("not run: {e}"));
    }
    lines.push(format!(
        "{} clips; {} test{} failed (p below {P_FAIL}, or a desync)",
        clips.len(),
        failed,
        if failed == 1 { "" } else { "s" }
    ));
    Ok(Outcome {
        json: json!({
            "clips": clips.len(),
            "p_fail": P_FAIL,
            "min_expected": MIN_EXPECTED,
            "sites": sites_j,
            "draws_taken": scripts.0,
            "draws_in_scripts": scripts.1,
            "failed": failed,
            "errors": errors,
        }),
        text: lines.join("\n"),
        exit: u8::from(failed > 0 || !errors.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups_of(f: impl Fn(u8) -> usize, n: usize) -> Groups {
        let mut g = vec![[0u64; 4]; n];
        for v in 0..=255u8 {
            g[f(v)][usize::from(v >> 6)] |= 1 << (v & 63);
        }
        g
    }

    #[test]
    fn branches_are_named_by_the_bits_they_read() {
        let ladder = groups_of(|v| usize::from(v & 0x88 != 0), 2);
        assert_eq!(
            groups_text(&ladder),
            "byte & 0x88: 0x00 (64/256) | 0x08,0x80,0x88 (192/256)"
        );
        let fire = groups_of(
            |v| match v & 7 {
                3 | 7 => 1,
                _ => 0,
            },
            2,
        );
        // 3 and 7 differ only in bit 2, which no branch reads.
        assert_eq!(describe(&fire), (0x03, vec![vec![0, 1, 2], vec![3]]));
    }

    /// The chi-square p-value against known values: 3.84 on 1 dof is p
    /// 0.05, 9.21 on 2 is 0.01, and counts at the odds give 1.
    #[test]
    fn the_chi_square_test_gives_known_p_values() {
        assert!((gamma_q(0.5, 3.841 / 2.0) - 0.05).abs() < 1e-3);
        assert!((gamma_q(1.0, 9.210 / 2.0) - 0.01).abs() < 1e-4);
        assert!((gamma_q(2.5, 20.515 / 2.0) - 0.001).abs() < 1e-4);
        let (c, d, p) = chi_square(&[25, 75], &[64, 192]).unwrap();
        assert_eq!((c, d), (0.0, 1));
        assert!((p - 1.0).abs() < 1e-9);
        // 1 in 4 drawn as 1 in 2 over 200 decisions fails.
        let (_, _, p) = chi_square(&[100, 100], &[64, 192]).unwrap();
        assert!(p < P_FAIL);
        // Too few to test.
        assert!(chi_square(&[1, 3], &[64, 192]).is_none());
    }

    /// A branch's odds come from the bytes the generator gives, not an even
    /// 256: a generator that gives the low half three times as often as the
    /// high makes `byte & 0x80 == 0` 3 in 4, and counts at 3 in 4 pass.
    #[test]
    fn odds_follow_the_generators_bytes() {
        let half = groups_of(|v| usize::from(v & 0x80 != 0), 2);
        assert_eq!(odds_of(&half, None), vec![0.5, 0.5]);
        let mut w = [0u64; 256];
        w[..128].fill(3);
        w[128..].fill(1);
        assert_eq!(odds_of(&half, Some(&w)), vec![0.75, 0.25]);
        let (_, _, p) = chi_square_odds(&[150, 50], &odds_of(&half, Some(&w))).unwrap();
        assert!(p > 0.99);
        assert!(chi_square(&[150, 50], &[128, 128]).unwrap().2 < P_FAIL);
        // Two games choosing with the same odds, and with others.
        assert!(chi_square_same(&[150, 50], &[300, 100]).unwrap().2 > 0.99);
        assert!(chi_square_same(&[150, 50], &[100, 100]).unwrap().2 < P_FAIL);
        assert!(chi_square_same(&[1, 0], &[1, 0]).is_none());
    }
}
