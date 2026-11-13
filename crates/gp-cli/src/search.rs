//! `gp interp reach`, `gp interp baselines`, `gp level measure` and `gp level difficulty`:
//! the searches and the stage measure of a level, run through the game's
//! module (`--module`) or the built-in interpreter for a model with no hooks.
//!
//! The searches hold the world on the host and step it through the module
//! (`gp_interp::variant`, [`crate::hosted::Source`]'s [`Sim`] implementation),
//! with the model edited to each variant as text and loaded.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_interp::baseline::{self, Expect, ModelBaseline};
use gp_interp::stage::{self, Measures};
use gp_model::Model;
use gp_sim::Spec;
use serde_json::{Value, json};

use crate::Outcome;
use crate::hosted::Source;

/// Frames a recorded run goes on after the goal, so the level's end state
/// is in the baseline.
const TAIL: usize = 30;

fn model_dir(model: &Path) -> PathBuf {
    if model.is_dir() {
        model.to_owned()
    } else {
        model.parent().map(Path::to_owned).unwrap_or_default()
    }
}

fn source(
    model: &Path,
    assets: Option<&Path>,
    paths: &Paths,
    module: Option<&Path>,
) -> Result<Source> {
    Source::open_dir(&model_dir(model), assets, paths, module).map(|(s, _)| s)
}

/// A template named on the command line: a folder holding model.toml, else
/// a name under templates/ (Moss Hop), else a game's package folder in the
/// games checkout (donkey-kong), as game-printer took a name under its
/// templates/.
fn template_dir(t: &Path) -> PathBuf {
    if t.exists() || t.components().count() != 1 {
        return t.to_owned();
    }
    let tool = Path::new("templates").join(t);
    if tool.is_dir() {
        return tool;
    }
    match crate::roms_cmd::games_dir(None).map(|g| g.join(t)) {
        Some(g) if g.is_dir() => g,
        _ => t.to_owned(),
    }
}

/// A variant's failure, as the search's `Err` carries it: the search gave a
/// result for a level that can't be searched, so this is the caller's to fail
/// on, as [`gp_interp::reach::envelope`]'s other errors aren't.
fn variant_failure(e: &str) -> bool {
    e.contains("doesn't compile to the edited spec") || e.contains("module")
}

pub fn reach_cmd(
    model: &Path,
    assets: Option<&Path>,
    levels: &[String],
    record: bool,
    module: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let src = source(model, assets, paths, module)?;
    let m = src.parse_model()?;
    let spec = src.spec.clone();
    let names: Vec<String> = if levels.is_empty() {
        spec.levels.iter().map(|l| l.name.clone()).collect()
    } else {
        levels.to_vec()
    };
    let mut text = String::new();
    let mut out = Vec::new();
    let mut ok = true;
    for name in &names {
        let i = spec
            .levels
            .iter()
            .position(|l| &l.name == name)
            .with_context(|| format!("the model has no level `{name}`"))?;
        let r = gp_interp::reach::reach(&spec, i, &src).map_err(|e| anyhow!("{e}"))?;
        let env = match gp_interp::reach::envelope(&spec, i, &src) {
            Ok(e) => Some(e),
            Err(e) if variant_failure(&e) => bail!("{e}"),
            Err(_) => None,
        };
        let reached = r.goal_path.is_some();
        ok &= reached;
        text.push_str(&format!(
            "{name}: goal {} by the player alone, {} places to stand, {} moves",
            if reached {
                "reachable"
            } else {
                "NOT reachable"
            },
            r.stands.len(),
            r.moves
        ));
        if let Some(e) = env {
            text.push_str(&format!(
                "; jump rises {} px over {} frames, carries {} px walking{}",
                e.rise,
                e.airtime,
                e.walk_distance,
                e.run_distance
                    .map_or(String::new(), |d| format!(", {d} running"))
            ));
        }
        text.push('\n');
        let mut entry = json!({
            "level": name,
            "reachable": reached,
            "stands": r.stands.len(),
            "goal_path": r.goal_path,
            "moves": r.moves,
            "envelope": env.map(|e| json!({
                "rise": e.rise, "airtime": e.airtime,
                "walk_distance": e.walk_distance, "run_distance": e.run_distance,
            })),
        });
        if record {
            match gp_interp::reach::playthrough(&spec, i, &src).map_err(|e| anyhow!("{e}"))? {
                None => {
                    ok = false;
                    text.push_str(&format!(
                        "  no run through {name} with its enemies found in {} places\n",
                        gp_interp::reach::PLAYTHROUGH_LIMIT
                    ));
                    entry["recorded"] = Value::Null;
                }
                Some(p) => {
                    let path = write_recorded(model, &m, name, &p, &src, module)?;
                    text.push_str(&format!(
                        "  recorded a run of {} frames through {} places ({} searched) -> {}\n",
                        p.buttons.len(),
                        p.stands.len(),
                        p.searched,
                        path.display()
                    ));
                    entry["recorded"] = json!(path);
                }
            }
        }
        out.push(entry);
    }
    Ok(Outcome {
        json: json!({ "ok": ok, "model": m.model.name, "levels": out }),
        text: text.trim_end().to_owned(),
        exit: if ok { 0 } else { 1 },
    })
}

fn write_recorded(
    model: &Path,
    m: &Model,
    level: &str,
    p: &gp_interp::reach::Playthrough,
    src: &Source,
    module: Option<&Path>,
) -> Result<PathBuf> {
    let frames = p.buttons.len() + TAIL;
    let mut buttons = p.buttons.clone();
    buttons.resize(frames, 0);
    let holds = baseline::holds_from_buttons(&buttons);
    let mut b = ModelBaseline {
        format: baseline::FORMAT.into(),
        model: m.model.name.clone(),
        level: level.to_owned(),
        description: Some(format!(
            "Plays {level} from its start to its goal; found by gp interp reach --record"
        )),
        recorded: "search".into(),
        frames,
        holds,
        digest: String::new(),
        expects: Vec::new(),
    };
    let input = b.input().map_err(|e| anyhow!("{e}"))?;
    let at = baseline::at_level(m, level).map_err(|e| anyhow!("{e}"))?;
    let mut rt = src.with_model(&at, module)?.open()?;
    let run = baseline::run_on(&mut rt, &input, &[]).map_err(|e| anyhow!("{e}"))?;
    b.digest = run.digest.clone();
    b.expects = end_expects(frames, &run);
    let dir = model_dir(model).join(baseline::DIR);
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{level}.toml"));
    let comment = format!(
        "A model baseline (docs/originals.md): the interpreter plays {level} of {}\n\
         from its start to its goal on this input, with every enemy in place.\n\
         Recorded by `gp interp reach --record`; `gp interp baselines` checks it.",
        m.model.name
    );
    fs::write(&path, b.to_toml(&comment)).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// The level, the game's states and its counters after the last frame:
/// what a recorded run expects.
fn end_expects(frames: usize, run: &baseline::Run) -> Vec<Expect> {
    run.last
        .iter()
        .filter(|(k, _)| {
            k == "level"
                || (k.starts_with("rules.") && k.ends_with(".state"))
                || k.starts_with("counters.")
        })
        .map(|(k, v)| Expect {
            frame: frames,
            path: k.clone(),
            equals: v.clone(),
        })
        .collect()
}

/// A file's leading comment, without its `# ` marks.
fn leading_comment(text: &str) -> String {
    text.lines()
        .take_while(|l| l.starts_with('#'))
        .map(|l| l.trim_start_matches('#').trim_start())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `gp interp baselines`: replays every model baseline in
/// MODEL/baselines/, each from its level, through the game's module or the
/// built-in interpreter, and compares its digest and expectations.
pub fn baselines_cmd(
    model: &Path,
    assets: Option<&Path>,
    write: bool,
    module: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let src = source(model, assets, paths, module)?;
    let m = src.parse_model()?;
    let dir = model_dir(model).join(baseline::DIR);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("{} holds no model baselines", dir.display());
    }
    let mut text = String::new();
    let mut out = Vec::new();
    let mut ok = true;
    for f in &files {
        let mut b = ModelBaseline::load(f).map_err(|e| anyhow!("{e}"))?;
        let mut problems = Vec::new();
        if b.model != m.model.name {
            problems.push(format!(
                "it's a baseline of {}, not {}",
                b.model, m.model.name
            ));
        }
        let input = b.input().map_err(|e| anyhow!("{e}"))?;
        let at = baseline::at_level(&m, &b.level).map_err(|e| anyhow!("{e}"))?;
        let mut rt = src.with_model(&at, module)?.open()?;
        let run = baseline::run_on(&mut rt, &input, &b.expects).map_err(|e| anyhow!("{e}"))?;
        if run.digest != b.digest {
            problems.push(format!("digest {} (expected {})", run.digest, b.digest));
        }
        for (e, got) in b.expects.iter().zip(&run.got) {
            if got.as_deref() != Some(e.equals.as_str()) {
                problems.push(format!(
                    "frame {}: {} is {} (expected {})",
                    e.frame,
                    e.path,
                    got.as_deref().unwrap_or("not there"),
                    e.equals
                ));
            }
        }
        let name = f.file_stem().unwrap_or_default().to_string_lossy();
        if write && !problems.is_empty() {
            // Record what the run gives now: its digest, and the state at
            // the end in place of the expectations at the last frame.
            let old = fs::read_to_string(f)?;
            b.digest = run.digest.clone();
            let mut expects: Vec<Expect> = b
                .expects
                .iter()
                .filter(|e| e.frame != b.frames)
                .cloned()
                .collect();
            expects.extend(end_expects(b.frames, &run));
            b.expects = expects;
            fs::write(f, b.to_toml(&leading_comment(&old)))
                .with_context(|| format!("writing {}", f.display()))?;
            let _ = writeln!(
                text,
                "{name}: rewritten ({} frames from {}): {}",
                b.frames,
                b.level,
                problems.join("; ")
            );
            out.push(json!({ "baseline": f, "level": b.level, "ok": true, "rewritten": true, "problems": problems }));
            continue;
        }
        ok &= problems.is_empty();
        let _ = writeln!(
            text,
            "{name}: {} ({} frames from {}, {} expectations)",
            if problems.is_empty() { "ok" } else { "FAILED" },
            b.frames,
            b.level,
            b.expects.len()
        );
        for p in &problems {
            let _ = writeln!(text, "  {p}");
        }
        out.push(json!({ "baseline": f, "level": b.level, "ok": problems.is_empty(), "problems": problems }));
    }
    Ok(Outcome {
        json: json!({ "ok": ok, "model": m.model.name, "baselines": out }),
        text: text.trim_end().to_owned(),
        exit: if ok { 0 } else { 1 },
    })
}

#[derive(Subcommand)]
pub enum LevelCmd {
    /// Search a level for a run to its goal (or to `--to`) with the player's
    /// own moves, measure how many frames earlier or later each input can come
    /// and still clear, and score the level from those windows, by section.
    /// This is a research spike, so the run is the first one found and a
    /// window is a lower bound.
    Difficulty {
        /// The template: a directory holding model.toml, a name under
        /// templates/ (mosshop) or a game's folder in the games checkout
        /// (donkey-kong).
        #[arg(long)]
        template: PathBuf,
        /// The level, by its name in the model.
        #[arg(long)]
        level: String,
        /// Stop at the first place the player stands at or past this x,
        /// in pixels, instead of the goal.
        #[arg(long, value_name = "X")]
        to: Option<i64>,
        /// The most places the search keeps.
        #[arg(long, default_value_t = 6000)]
        limit: usize,
        /// The model's lifted assets (default:
        /// $GP_HOME/work/assets/<model.source>).
        #[arg(long, value_name = "DIR")]
        assets: Option<PathBuf>,
        /// The game's built module. Needed for a model with hooks.
        #[arg(long, value_name = "FILE")]
        module: Option<PathBuf>,
    },
    /// Measure a stage of sloped girders and ladders on the model
    /// (gp_interp::stage). It gives the stage's girders, ladders and things. It
    /// also gives each ladder's climb and the direct route to the top. For the
    /// rollers (barrels) it gives their routes, as the girders they roll on,
    /// the ladders they take and the frames from the throw to the end, and the
    /// jump window over each roller that meets the player on each girder.
    Measure {
        /// The template: a directory holding model.toml, a name under
        /// templates/ (mosshop) or a game's folder in the games checkout
        /// (donkey-kong).
        #[arg(long)]
        template: PathBuf,
        /// The level, by its name in the model.
        #[arg(long, default_value = "main")]
        level: String,
        /// The entity whose routes are measured.
        #[arg(long, default_value = "barrel")]
        roller: String,
        /// Runs of the game for the rollers' routes, each Start pressed 37
        /// frames later.
        #[arg(long, default_value_t = 6)]
        runs: u32,
        /// Frames each run lasts.
        #[arg(long, default_value_t = 3000)]
        frames: u32,
        /// Also play the direct route this many times with the enemies in,
        /// from power-on, waiting before each walk until it is lived
        /// through, and print the holds as a model baseline takes them.
        #[arg(long, value_name = "N")]
        clears: Option<u32>,
        /// The model's lifted assets (default:
        /// $GP_HOME/work/assets/<model.source>).
        #[arg(long, value_name = "DIR")]
        assets: Option<PathBuf>,
        /// The game's built module. Needed for a model with hooks.
        #[arg(long, value_name = "FILE")]
        module: Option<PathBuf>,
    },
    /// Write a game's level kit (docs/level-kit.md): `level-kit/kit.toml`
    /// and `level-kit/level-kit.md`, what a designer needs to make a stage
    /// by hand. The levels' sizes, the hero's moves and the goal come from
    /// the model and the asset map, and so do the things' boxes and the data
    /// slots. The hand-written `level-kit/rules.toml` gives the rest. That
    /// file says what's data and what's fixed in code, and the limits and the
    /// play measures to land near. It also gives how to verify, a worked
    /// example and what is still open.
    Kit {
        /// A template directory, a template id under ./templates or a game in
        /// the games checkout.
        game: PathBuf,
        /// Where to write (default: the template's level-kit/).
        #[arg(short, long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
}

pub fn level_run(cmd: &LevelCmd, paths: &Paths) -> Result<Outcome> {
    if let LevelCmd::Kit { game, out } = cmd {
        return crate::level_kit::kit_cmd(game, out.as_deref());
    }
    if let LevelCmd::Measure {
        template,
        level,
        roller,
        runs,
        frames,
        clears,
        assets,
        module,
    } = cmd
    {
        let src = source(
            &template_dir(template),
            assets.as_deref(),
            paths,
            module.as_deref(),
        )?;
        let m = src.parse_model()?;
        return measure(&m, &src, level, roller, (*runs, *frames, *clears));
    }
    let LevelCmd::Difficulty {
        template,
        level,
        to,
        limit,
        assets,
        module,
    } = cmd
    else {
        unreachable!("the measure and the kit are above")
    };
    let src = source(
        &template_dir(template),
        assets.as_deref(),
        paths,
        module.as_deref(),
    )?;
    let spec = &src.spec;
    let i = spec
        .levels
        .iter()
        .position(|l| &l.name == level)
        .with_context(|| format!("the model has no level `{level}`"))?;
    let opts = gp_interp::difficulty::Options {
        to_x: *to,
        limit: *limit,
        ..Default::default()
    };
    let d = gp_interp::difficulty::score(spec, i, &src, opts).map_err(|e| anyhow!("{e}"))?;
    let mut text = String::new();
    if !d.cleared {
        text.push_str(&format!(
            "{level}: no run found in {} places ({} frames simulated)\n",
            d.searched, d.frames
        ));
    } else {
        text.push_str(&format!(
            "{level}: a run of {} frames, {} inputs, score {}.{:03} bits ({} places searched, {} frames simulated)\n",
            d.buttons.len(),
            d.inputs.len(),
            d.millibits / 1000,
            d.millibits % 1000,
            d.searched,
            d.frames
        ));
        text.push_str(&format!(
            "map, {} px a section: [{}]\n",
            opts.section,
            gp_interp::difficulty::strip(&d.sections)
        ));
        text.push_str("frame  x     y    window  bits   input; edges early..late; parts\n");
        for inp in &d.inputs {
            let edges: Vec<String> = inp
                .edges
                .iter()
                .map(|e| {
                    let early = if e.early_open {
                        "open".to_owned()
                    } else {
                        e.early.to_string()
                    };
                    format!("{} {}..+{}", e.kind.words(), early, e.late)
                })
                .collect();
            text.push_str(&format!(
                "{:<6} {:<5} {:<4} {:<7} {:<6} {}; {}; {}\n",
                inp.frame,
                inp.x,
                inp.y,
                inp.window.map_or("-".into(), |w| w.to_string()),
                format!("{}.{:03}", inp.millibits / 1000, inp.millibits % 1000),
                inp.what,
                edges.join(", "),
                if inp.parts.is_empty() {
                    "-".into()
                } else {
                    inp.parts.join(", ")
                }
            ));
        }
    }
    let json = json!({
        "ok": d.cleared,
        "model": spec.name,
        "level": level,
        "cleared": d.cleared,
        "frames": d.buttons.len(),
        "millibits": d.millibits,
        "searched": d.searched,
        "simulated": d.frames,
        "strip": gp_interp::difficulty::strip(&d.sections),
        "sections": d.sections.iter().map(|s| json!({
            "x0": s.x0, "x1": s.x1, "inputs": s.inputs,
            "millibits": s.millibits, "tightest": s.tightest,
        })).collect::<Vec<_>>(),
        "inputs": d.inputs.iter().map(|i| json!({
            "frame": i.frame, "x": i.x, "y": i.y, "what": i.what,
            "window": i.window, "millibits": i.millibits, "parts": i.parts,
            "edges": i.edges.iter().map(|e| json!({
                "edge": e.kind.words(), "at": e.at, "early": e.early,
                "late": e.late, "early_open": e.early_open,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    Ok(Outcome {
        json,
        text: text.trim_end().to_owned(),
        exit: if d.cleared { 0 } else { 1 },
    })
}

fn median(v: &mut [u32]) -> Option<u32> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

fn share(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

/// The measures as numbers by name, the ones the stage is tuned to.
pub(crate) fn summary(m: &Measures) -> BTreeMap<String, Value> {
    let lay = m.layout.as_ref().expect("measured");
    let mut s = BTreeMap::new();
    let n = lay.girders.len();
    let main: Vec<usize> = (0..n).filter(|&i| lay.girders[i].width() >= 64).collect();
    s.insert("girders".into(), json!(main.len()));
    s.insert("platforms".into(), json!(n - main.len()));
    let slopes: Vec<f64> = main
        .iter()
        .map(|&i| {
            let g = &lay.girders[i];
            let (a, b) = g.ends();
            (b - a) as f64 * 24.0 / g.width() as f64
        })
        .collect();
    s.insert(
        "slope_px_per_24".into(),
        json!(
            slopes
                .iter()
                .map(|x| (x * 100.0).round() / 100.0)
                .collect::<Vec<_>>()
        ),
    );
    let gaps: Vec<i64> = (1..n)
        .map(|i| lay.girders[i - 1].mid() - lay.girders[i].mid())
        .collect();
    s.insert("gaps".into(), json!(gaps));
    let ends: Vec<(i64, i64)> = main
        .iter()
        .map(|&i| (lay.girders[i].x0(), lay.girders[i].x1()))
        .collect();
    s.insert("girder_ends".into(), json!(ends));
    let ladders = lay.ladders.len();
    let broken: Vec<usize> = m
        .climbs
        .iter()
        .filter(|c| c.frames.is_none())
        .map(|c| c.ladder)
        .collect();
    s.insert("ladders".into(), json!(ladders));
    s.insert("broken".into(), json!(broken.len()));
    s.insert("broken_share".into(), json!(share(broken.len(), ladders)));
    // Where each sits across its lower girder, 0 left end to 1 right.
    let across: Vec<Value> = lay
        .ladders
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let g = l.lower.map(|g| &lay.girders[g]);
            let at = g.map(|g| (l.cx() - g.x0()) as f64 / g.width() as f64);
            json!({"ladder": i, "lower": l.lower, "upper": l.upper, "x": l.cx(),
                   "across": at.map(|a| (a * 100.0).round() / 100.0),
                   "broken": broken.contains(&i),
                   "climb": m.climbs.get(i).and_then(|c| c.frames)})
        })
        .collect();
    s.insert("ladder_places".into(), json!(across));
    let per_girder: Vec<usize> = (0..n)
        .map(|g| lay.ladders.iter().filter(|l| l.lower == Some(g)).count())
        .collect();
    s.insert("ladders_above_each_girder".into(), json!(per_girder));
    let bottom = lay.girders[0].mid();
    // Things: their height over the bottom girder, and how far each is
    // from the nearest climbable ladder that meets its girder.
    let mut things = Vec::new();
    for t in &lay.things {
        let near = t.girder.and_then(|g| {
            lay.ladders
                .iter()
                .enumerate()
                .filter(|(i, l)| (l.lower == Some(g) || l.upper == Some(g)) && !broken.contains(i))
                .map(|(_, l)| (l.cx() - (t.x + t.w / 2)).abs())
                .min()
        });
        things.push(
            json!({"kind": t.kind, "x": t.x, "y": t.y, "girder": t.girder,
                           "height": bottom - (t.y + t.h), "to_ladder": near}),
        );
    }
    s.insert("things".into(), json!(things));
    s.insert(
        "goal_height".into(),
        json!(lay.goal_under.map(|g| bottom - g)),
    );
    s.insert("walk_px_per_96".into(), json!(m.walk_px));
    let mut climbs: Vec<u32> = m.climbs.iter().filter_map(|c| c.frames).collect();
    climbs.sort_unstable();
    s.insert("climb_frames_min".into(), json!(climbs.first()));
    s.insert(
        "climb_frames_median".into(),
        json!(median(&mut climbs.clone())),
    );
    s.insert("route_ladders".into(), json!(m.route.len()));
    s.insert("route_plan_frames".into(), json!(m.route_plan));
    s.insert("route_frames".into(), json!(m.route_frames));
    let used: Vec<Value> = (0..n)
        .map(|g| {
            let w = m.route_walk.get(&g).copied().unwrap_or(0);
            json!((share(w as usize, lay.girders[g].width() as usize) * 100.0).round() / 100.0)
        })
        .collect();
    s.insert("route_walk_share".into(), json!(used));
    // Rollers.
    let thrown: Vec<&stage::Life> = m
        .lives
        .iter()
        .filter(|l| !l.open && !l.girders.is_empty())
        .collect();
    s.insert("rollers".into(), json!(thrown.len()));
    let drum = lay.things.iter().find(|t| t.kind == "oil_drum");
    let at_drum = |l: &stage::Life| {
        drum.is_some_and(|d| {
            l.girders.last().is_some_and(|g| g.0 == 0) && (l.last.0 - (d.x + d.w / 2)).abs() <= 24
        })
    };
    let mut to_drum: Vec<u32> = thrown
        .iter()
        .filter(|l| at_drum(l))
        .map(|l| (l.end - l.start) as u32)
        .collect();
    s.insert("to_drum".into(), json!(to_drum.len()));
    s.insert(
        "to_drum_share".into(),
        json!(share(to_drum.len(), thrown.len())),
    );
    s.insert("throw_to_drum_median".into(), json!(median(&mut to_drum)));
    let mut leaves: BTreeMap<String, usize> = BTreeMap::new();
    for l in &thrown {
        for h in &l.leaves {
            let k = if h.starts_with("ladder") {
                "ladder"
            } else {
                h.as_str()
            };
            *leaves.entry(k.to_owned()).or_default() += 1;
        }
    }
    let total: usize = leaves.values().sum();
    s.insert("leaves".into(), json!(leaves));
    s.insert(
        "leave_by_ladder_share".into(),
        json!(share(leaves.get("ladder").copied().unwrap_or(0), total)),
    );
    // Each ladder: lives that rolled past its top, and those that took it.
    let mut odds = Vec::new();
    for (i, d) in lay.ladders.iter().enumerate() {
        let Some(up) = d.upper else { continue };
        let reached = thrown
            .iter()
            .filter(|l| {
                l.girders
                    .iter()
                    .any(|g| g.0 == up && g.1 - 2 <= d.cx() && d.cx() <= g.2 + 2)
            })
            .count();
        let took = thrown
            .iter()
            .filter(|l| l.leaves.iter().any(|h| h == &format!("ladder {i}")))
            .count();
        if reached > 0 {
            odds.push(json!({"ladder": i, "reached": reached, "took": took,
                             "odds": (share(took, reached) * 100.0).round() / 100.0}));
        }
    }
    s.insert("ladder_odds".into(), json!(odds));
    let girder_use: Vec<Value> = (0..n)
        .map(|g| {
            json!(
                (share(
                    thrown
                        .iter()
                        .filter(|l| l.girders.iter().any(|x| x.0 == g))
                        .count(),
                    thrown.len()
                ) * 100.0)
                    .round()
                    / 100.0
            )
        })
        .collect();
    s.insert("roller_girder_share".into(), json!(girder_use));
    // Jump windows by girder.
    let mut by: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    for w in &m.windows {
        by.entry(w.girder).or_default().push(w.passes.len() as u32);
    }
    let windows: Vec<Value> = by
        .iter_mut()
        .map(|(g, v)| {
            let met = v.len();
            let zero = v.iter().filter(|&&x| x == 0).count();
            json!({"girder": g, "met": met, "median": median(v), "min": v.iter().min(), "max": v.iter().max(), "none": zero})
        })
        .collect();
    s.insert("jump_windows".into(), json!(windows));
    let mut all: Vec<u32> = m.windows.iter().map(|w| w.passes.len() as u32).collect();
    s.insert("jump_window_median".into(), json!(median(&mut all)));
    s
}

fn measure(
    model: &Model,
    src: &Source,
    level: &str,
    roller: &str,
    (runs, frames, clears): (u32, u32, Option<u32>),
) -> Result<Outcome> {
    let spec: &Spec = &src.spec;
    let i = spec
        .levels
        .iter()
        .position(|l| l.name == level)
        .with_context(|| format!("the model has no level `{level}`"))?;
    let kind = |name: &str| spec.kinds.iter().position(|k| k.name == name);
    let enemies = model
        .entities
        .iter()
        .filter(|(_, e)| e.core.role == "enemy")
        .filter_map(|(n, _)| kind(n))
        .collect();
    let playing = spec
        .machines
        .iter()
        .position(|m| m.name == "game")
        .and_then(|mi| {
            spec.machines[mi]
                .states
                .iter()
                .position(|s| s.name == "playing")
                .map(|si| (mi, si))
        });
    let o = stage::Options {
        enemies,
        roller: kind(roller),
        playing,
        run_frames: frames,
        runs,
    };
    let m = stage::measure(src, spec, i, &o).map_err(|e| anyhow!("{e}"))?;
    let s = summary(&m);
    let mut text = String::new();
    for (k, v) in &s {
        if matches!(
            k.as_str(),
            "ladder_places" | "things" | "ladder_odds" | "jump_windows"
        ) {
            let _ = writeln!(text, "{k}:");
            for x in v.as_array().into_iter().flatten() {
                let _ = writeln!(text, "  {x}");
            }
        } else {
            let _ = writeln!(text, "{k}: {v}");
        }
    }
    for n in &m.notes {
        let _ = writeln!(text, "note: {n}");
    }
    let holds: Vec<String> = m
        .route_holds
        .iter()
        .map(|(b, a, z)| format!("{}@{a}-{z}", button_name(*b)))
        .collect();
    let _ = writeln!(
        text,
        "route holds from frame {}: {}",
        m.ready,
        holds.join(", ")
    );
    let mut run = Value::Null;
    if let Some(n) = clears {
        let (h, cleared, next) = stage::clears(src, spec, &o, &m, n).map_err(|e| anyhow!("{e}"))?;
        let h: Vec<String> = h
            .iter()
            .map(|(b, a, z)| format!("\"{}@{a}-{z}\"", button_name(*b)))
            .collect();
        let _ = writeln!(
            text,
            "clears at frames {cleared:?}; the next stage plays from {next}"
        );
        let _ = writeln!(text, "holds = [{}]", h.join(", "));
        run = json!({"holds": h, "cleared": cleared, "next": next});
    }
    Ok(Outcome {
        json: json!({"ok": true, "model": spec.name, "level": level, "ready": m.ready,
                     "measures": s, "route_holds": holds, "notes": m.notes, "clears": run}),
        text: text.trim_end().to_owned(),
        exit: 0,
    })
}

fn button_name(b: u8) -> String {
    let names = ["a", "b", "select", "start", "up", "down", "left", "right"];
    let mut out: Vec<&str> = (0..8)
        .filter(|k| b >> k & 1 != 0)
        .map(|k| names[k])
        .collect();
    // Directions first, as the baselines write them.
    out.sort_by_key(|n| n.len() == 1);
    out.join("+")
}
