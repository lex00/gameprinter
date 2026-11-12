//! `gp solve GAME --run RUN`: each non-exact clip of a game's corpus
//! scored into a run, its detector class and its proposals ([`super`]).
//! With `--check`, each clip's best proposal is scored the same way:
//! better on the clip, and no other clip worse. It never writes the model.
//!
//! Ported from game-printer, where `--run` named an old capture run and its
//! background queue. In gameprinter a scored run is `gp corpus score GAME
//! --run RUN`: the corpus's clips are made again from the ROM into the
//! run's folder (`$GP_HOME/capture/RUN`, the starts kept there) and scored
//! with the scores cached, so after a corpus score of the same model the
//! clips aren't run again. With no queue to say which clips weren't
//! exact, every clip of the corpus is scored, and the ones not exact are
//! solved. `--threads` sets the scoring threads (default: `--jobs`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clap::Args;
use serde_json::{Value, json};

use gp_interp::scenario::{Bar, Outcome as Ran, Scenario};

use crate::Outcome;
use crate::clipset as cs;
use crate::scenario::{self as sc, Started};

#[derive(Args)]
pub struct SolveArgs {
    /// The game's package folder (games/donkey-kong), with its corpus.
    pub game: PathBuf,
    /// The run's name, as `gp corpus score --run` names it: its starts and
    /// score caches are kept under $GP_HOME.
    #[arg(long)]
    pub run: String,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The model to solve for; by default the game's own.
    #[arg(long)]
    pub model: Option<PathBuf>,
    /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE,
    /// lifted there first if missing).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// The ROM; by default the one in $GP_ROM_DIR the package names.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// Only this clip, by ID.
    #[arg(long)]
    pub clip: Option<String>,
    /// Score each clip's best proposal on the run's clips: the clips it
    /// makes better and worse. Code-cited numbers are scored too, and
    /// reported apart; nothing is written.
    #[arg(long)]
    pub check: bool,
    /// Clips scored at once (default: --jobs).
    #[arg(long)]
    pub threads: Option<usize>,
    /// Also run the body and state detectors (`bodies`, `states`): a body
    /// one game has alone, a value the original takes that the model
    /// never does in any of the run's clips, a split on a state's entry.
    #[arg(long)]
    pub missing: bool,
}

/// A clip's score as the check ranks it: its first wrong frame (later is
/// better), then how far from exact.
fn rank(r: &Ran) -> (std::cmp::Reverse<u64>, u64) {
    (
        std::cmp::Reverse(r.first.as_ref().map_or(u64::MAX, |f| f.frame)),
        r.score,
    )
}

/// The bench a model edited to `m` is scored on.
struct Benches<'a> {
    base: &'a sc::Bench,
    module: Option<&'a Path>,
}

impl Benches<'_> {
    fn of(&self, m: &gp_model::Model) -> Result<sc::Bench> {
        self.base.with_model(m, self.module)
    }
}

fn score_one(b: &sc::Bench, m: &gp_model::Model, clip: &Scenario) -> Result<Ran> {
    let text = toml::to_string(&m.to_tree())?;
    let mut r = sc::score(
        b,
        m,
        &text,
        &Started::none(),
        std::slice::from_ref(clip),
        true,
        &mut |_, _, _| {},
    )?;
    Ok(r.remove(0))
}

/// Scores `clips` on `m` (on the bench compiled for it) on the helper
/// threads, in their order; a clip that can't be scored is None.
fn score_all(b: &sc::Bench, m: &gp_model::Model, clips: &[Scenario]) -> Vec<Option<Ran>> {
    let Ok(text) = toml::to_string(&m.to_tree()) else {
        return vec![None; clips.len()];
    };
    match sc::score_each(
        b,
        m,
        &text,
        &Started::none(),
        clips,
        true,
        &mut |_, _, _| {},
    ) {
        Ok(rs) => rs.into_iter().map(Result::ok).collect(),
        Err(_) => vec![None; clips.len()],
    }
}

pub(crate) fn run(args: &SolveArgs, paths: &gp_evidence::Paths) -> Result<Outcome> {
    if let Some(n) = args.threads {
        crate::pool::set_jobs(n);
    }
    let game = args.game.as_path();
    let rom = crate::clip_score::rom_of(paths, game, args.rom.as_deref())?;
    let mdir = args.model.as_deref().unwrap_or(game).to_owned();
    let assets = crate::corpus_score::assets_of(paths, game, &mdir, args.assets.as_deref(), &rom)?;
    let (source, _) =
        crate::hosted::Source::open_dir(&mdir, Some(&assets), paths, args.module.as_deref())?;
    let dir = crate::corpus_score::run_dir(&paths.home, &args.run)?;
    std::fs::create_dir_all(&dir)?;
    let work = cs::Work {
        paths,
        run_dir: &dir,
        run: &args.run,
        mdir: &mdir,
        assets: &assets,
        rom,
        source: Arc::new(source),
    };
    let b = cs::bench_for(&work)?;
    let benches = Benches {
        base: &b,
        module: args.module.as_deref(),
    };
    let (model, _) = b.model()?;
    let catalog = gp_interp::mechanics::catalog(&model, &b.hosted.exts);
    // Every clip of the corpus, made again into the run.
    let set = cs::read_game(game)?;
    let mapping = b.mapping.clone();
    let made = cs::make(&paths.home, &dir, game, &work.rom, &mapping, &set.clips)?;
    let mut all: Vec<Scenario> = made
        .into_iter()
        .filter_map(|m| m.ok().map(|m| m.scenario))
        .collect();
    sc::resolve_logs(&mut all, &paths.home);
    if let Some(id) = &args.clip
        && !all.iter().any(|c| &c.id == id)
    {
        anyhow::bail!("the corpus has no clip {id} that can be made");
    }
    let clips: Vec<Scenario> = all
        .iter()
        .filter(|c| args.clip.as_ref().is_none_or(|id| &c.id == id))
        .cloned()
        .collect();
    let scored = score_all(&b, &model, &clips);
    let mut rows: Vec<Value> = Vec::new();
    let mut text = String::new();
    let mut counts: BTreeMap<String, [usize; 6]> = BTreeMap::new();
    // Every clip's score before, for the check's "none worse".
    let held: Vec<Scenario> = if args.check { all.clone() } else { Vec::new() };
    let before_all = if args.check {
        score_all(&b, &model, &held)
    } else {
        Vec::new()
    };
    // The model's values over every clip of the run, for `states`.
    let mut missing_counts: BTreeMap<String, [usize; 2]> = BTreeMap::new();
    let seen = if args.missing {
        let every: Vec<Scenario> = all.clone();
        let rs = score_all(&b, &model, &every);
        super::missing::Seen::of(rs.iter().flatten())
    } else {
        super::missing::Seen::default()
    };
    for (clip, r) in clips.iter().zip(&scored) {
        let Some(r) = r else {
            let _ = writeln!(text, "{}: can't be scored", clip.id);
            continue;
        };
        if r.exact {
            continue;
        }
        let s = super::solve(r, &clip.name, &model, &catalog);
        let first = s.findings.first();
        let class = first.map_or("none".to_owned(), |f| format!("{} {}", f.solver, f.class));
        for name in ["arith", "timers"] {
            let c = counts.entry(name.to_owned()).or_default();
            if s.findings.iter().any(|f| f.solver == name) {
                c[0] += 1;
            }
            if s.proposals.iter().any(|p| p.by == format!("solver:{name}")) {
                c[1] += 1;
            }
        }
        if s.findings.is_empty() {
            counts.entry("none".to_owned()).or_default()[0] += 1;
        }
        let _ = writeln!(
            text,
            "{}: {class}: {}",
            clip.id,
            first.map_or("no detector reads it", |f| f.says.as_str())
        );
        for p in s.proposals.iter().take(4) {
            let _ = writeln!(
                text,
                "  {} {}: {}{}",
                p.by,
                p.class,
                p.edits
                    .iter()
                    .map(|e| format!(
                        "{} {} -> {}",
                        e.path,
                        e.old.as_deref().unwrap_or("(unset)"),
                        e.new
                    ))
                    .collect::<Vec<_>>()
                    .join(", "),
                if p.code.is_some() {
                    " (the game's code: a question)"
                } else {
                    ""
                }
            );
        }
        let mut row = json!({"clip": clip.id, "name": clip.name, "class": class,
            "findings": s.findings, "proposals": s.proposals});
        if args.missing {
            let mf = super::missing::findings(r, &seen);
            for f in &mf {
                let _ = writeln!(
                    text,
                    "  {} {}{}: {}",
                    f.solver,
                    f.class,
                    if f.first { " (first)" } else { "" },
                    f.says
                );
            }
            let mut classes: Vec<String> = mf
                .iter()
                .map(|f| format!("{} {}", f.solver, f.class))
                .collect();
            classes.sort();
            classes.dedup();
            for c in classes {
                let e = missing_counts.entry(c.clone()).or_default();
                e[0] += 1;
                if mf
                    .iter()
                    .any(|f| format!("{} {}", f.solver, f.class) == c && f.first)
                {
                    e[1] += 1;
                }
            }
            row["missing"] = json!(mf);
        }
        if args.clip.is_some() {
            // One clip: its wrong measures' series too, the evidence the
            // findings read.
            row["measures"] = json!(
                r.measures
                    .iter()
                    .filter(|m| m.first.is_some())
                    .map(
                        |m| json!({"path": m.path, "first": m.first.as_ref().map(|f| f.frame),
                    "owners": m.owners, "original": m.original, "model": m.model})
                    )
                    .collect::<Vec<_>>()
            );
        }
        if args.check && !s.proposals.is_empty() {
            // Each proposal on the clip alone; the best that makes it
            // better is scored on every clip.
            let mut best: Option<(usize, Ran, gp_model::Model, sc::Bench)> = None;
            let mut tried = Vec::new();
            for (i, p) in s.proposals.iter().enumerate() {
                let Ok(mm) = super::apply(&model, &p.edits) else {
                    tried.push(json!({"proposal": i, "error": "doesn't parse"}));
                    continue;
                };
                let Ok(bm) = benches.of(&mm) else {
                    tried.push(json!({"proposal": i, "error": "doesn't compile"}));
                    continue;
                };
                let Ok(a) = score_one(&bm, &mm, clip) else {
                    continue;
                };
                tried.push(json!({"proposal": i, "first": a.first.as_ref().map(|f| f.frame), "score": a.score, "exact": a.exact}));
                if rank(&a) < rank(r) && best.as_ref().is_none_or(|(_, x, _, _)| rank(&a) < rank(x))
                {
                    best = Some((i, a, mm, bm));
                }
            }
            row["tried"] = json!(tried);
            if let Some((i, a, mm, bm)) = best {
                let after_all = score_all(&bm, &mm, &held);
                let (mut fixed, mut better, mut worse) = (Vec::new(), Vec::new(), Vec::new());
                for ((h, x), y) in held.iter().zip(&before_all).zip(&after_all) {
                    if h.id == clip.id {
                        continue;
                    }
                    let (Some(x), Some(y)) = (x, y) else { continue };
                    if y.score > x.score || (Bar::Exact.passes(x) && !Bar::Exact.passes(y)) {
                        worse.push(h.id.clone());
                    } else if !Bar::Exact.passes(x) && Bar::Exact.passes(y) {
                        fixed.push(h.id.clone());
                    } else if y.score < x.score {
                        better.push(h.id.clone());
                    }
                }
                let p = &s.proposals[i];
                let kept = worse.is_empty();
                let c = counts
                    .entry(p.by.trim_start_matches("solver:").to_owned())
                    .or_default();
                c[2] += 1;
                if kept {
                    c[3] += 1;
                }
                c[4] += better.len() + fixed.len() + 1;
                c[5] += worse.len();
                let _ = writeln!(
                    text,
                    "  check: proposal {} makes the clip better (first wrong {} -> {}, {}), {} others better or fixed, {} worse{}",
                    i + 1,
                    r.first
                        .as_ref()
                        .map_or("none".into(), |f| f.frame.to_string()),
                    a.first
                        .as_ref()
                        .map_or("none".into(), |f| f.frame.to_string()),
                    if a.exact { "exact" } else { "not exact" },
                    better.len() + fixed.len(),
                    worse.len(),
                    if p.code.is_some() {
                        "; the game's code, so a question"
                    } else {
                        ""
                    }
                );
                row["check"] = json!({"proposal": i, "exact": a.exact,
                    "first": {"before": r.first.as_ref().map(|f| f.frame), "after": a.first.as_ref().map(|f| f.frame)},
                    "fixed": fixed, "better": better, "worse": worse, "kept": kept, "code": p.code.is_some()});
            } else {
                let _ = writeln!(text, "  check: no proposal makes the clip better");
            }
        }
        rows.push(row);
    }
    let exact = scored.iter().flatten().filter(|r| r.exact).count();
    let _ = writeln!(
        text,
        "{} clips: {} exact on the model as it stands, {} not, {} not scored",
        clips.len(),
        exact,
        rows.len(),
        scored.iter().filter(|r| r.is_none()).count()
    );
    let _ = writeln!(
        text,
        "solver: classified, with a proposal, checked better, kept (none worse), clips better, clips worse"
    );
    for (k, c) in &counts {
        let _ = writeln!(
            text,
            "{k}: {} {} {} {} {} {}",
            c[0], c[1], c[2], c[3], c[4], c[5]
        );
    }
    if args.missing {
        let _ = writeln!(
            text,
            "bodies and states (model values from {} clips): clips flagged, of them first wrong there",
            seen.clips
        );
        for (k, c) in &missing_counts {
            let _ = writeln!(text, "{k}: {} {}", c[0], c[1]);
        }
    }
    let totals: BTreeMap<&String, Value> = counts
        .iter()
        .map(|(k, c)| {
            (k, json!({"classified": c[0], "proposed": c[1], "checked": c[2], "kept": c[3], "better": c[4], "worse": c[5]}))
        })
        .collect();
    Ok(Outcome::ok(
        json!({"run": args.run, "game": game.display().to_string(), "clips": rows, "totals": totals,
            "missing": missing_counts.iter().map(|(k, c)| (k.clone(), json!({"clips": c[0], "first": c[1]}))).collect::<BTreeMap<_, _>>()}),
        text.trim_end().to_owned(),
    ))
}
