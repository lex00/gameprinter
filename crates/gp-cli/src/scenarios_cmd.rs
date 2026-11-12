//! `gp interp scenarios`: the scenario ladder (gp_interp::scenario) run on
//! the original and the model from the original's state at one frame of an
//! input, scored, with each scenario's state (locked, open, failing,
//! passed), the next one to work on and a score history across model
//! changes. Results are cached by everything they read, under
//! `$GP_HOME/work/scenarios`.
//!
//! Ported from game-printer's scenario.rs (`cmd`). What gameprinter's
//! layout changed: the model runs through its game's module (`--module`)
//! or the built-in interpreter, the original through `OriginalRuntime`;
//! `--recording` takes an input log or a baseline file, since gameprinter
//! has no recordings store yet; there is no run recorder, so the ladder's
//! metrics aren't recorded; and `--refine` and `--auto` run gp model
//! refine --lockstep's loop (crate::scenario_refine).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::{InputLog, Paths};
use gp_interp::scenario::Bar;
use serde_json::Value as Json;

use crate::Outcome;
use crate::scenario::{self as sc, Bench};

#[derive(clap::Args)]
pub struct ScenariosArgs {
    /// A model directory holding model.toml and mapping.toml.
    pub model: PathBuf,
    /// The original game's ROM.
    #[arg(long)]
    pub rom: PathBuf,
    /// A recording in the store (an ID prefix), a baseline file or an
    /// input log file: the input played to the start. gameprinter has no
    /// recordings store yet, so a file.
    #[arg(long, value_name = "ID|FILE")]
    pub recording: String,
    /// The original's frame to start at (default: a quiet frame).
    #[arg(long, value_name = "FRAME")]
    pub at: Option<u64>,
    /// Only tiers up to this one (1 to 3).
    #[arg(long)]
    pub tier: Option<u8>,
    /// When a scenario passes: `exact` (every measure, every frame) or
    /// a share of frames within tolerance, such as `99%`.
    #[arg(long, default_value = "exact")]
    pub bar: String,
    /// A played scenario (its JSON) to run with the ladder; repeat for
    /// more.
    #[arg(long, value_name = "FILE")]
    pub played: Vec<PathBuf>,
    /// Refine the working copy on this scenario: its start and presses,
    /// its mechanic's numbers only, and only fits that make no other
    /// scenario worse.
    #[arg(long, value_name = "ID")]
    pub refine: Option<String>,
    /// Work through the open scenarios: refine the next one until it
    /// passes, then the next, until one is stuck or all pass.
    #[arg(long, conflicts_with = "refine")]
    pub auto: bool,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The model's lifted assets (default:
    /// $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
}

/// The input a start names: a baseline file (`.toml`) or an input log
/// file, and the words for it.
fn input_of(input: &str) -> Result<(InputLog, String)> {
    let file = Path::new(input);
    if !file.is_file() {
        bail!(
            "{input} isn't a file; gameprinter has no recordings store yet (`gp record` isn't ported), so give a baseline or an input log file"
        );
    }
    if file.extension().is_some_and(|e| e == "toml") {
        let b = gp_evidence::baseline::Baseline::load(file)?;
        return Ok((b.input()?, format!("baseline {input}")));
    }
    let text = fs::read_to_string(file).with_context(|| format!("reading {input}"))?;
    Ok((InputLog::parse(&text)?, format!("input {input}")))
}

pub(crate) fn run(a: &ScenariosArgs, paths: &Paths) -> Result<Outcome> {
    let bar = Bar::parse(&a.bar).map_err(anyhow::Error::msg)?;
    let rom = crate::interp_args::read_rom(&a.rom)?;
    let (log, _) = input_of(&a.recording)?;
    let model_dir = crate::interp_args::model_dir(&a.model);
    let assets = match &a.assets {
        Some(x) => Some(x.clone()),
        None if model_dir.join("../assets").is_dir() => Some(model_dir.join("../assets")),
        None => None,
    };
    let (source, assets) =
        crate::hosted::Source::open_dir(&model_dir, assets.as_deref(), paths, a.module.as_deref())?;
    let cache = paths.home.join("work").join("scenarios");
    let b = Bench::new(
        &model_dir,
        &assets,
        rom,
        log.to_text(),
        &a.recording,
        &cache,
        std::sync::Arc::new(source),
    )?;
    let mut played = sc::read_played(&a.played)?;
    sc::resolve_logs(&mut played, &paths.home);
    let args = crate::scenario_refine::RefineArgs {
        at: a.at,
        tier: a.tier,
        played: &played,
        bar,
        pool_only: false,
    };
    if a.auto {
        let j = crate::scenario_refine::auto(&b, &args, &mut |_| {})?;
        let mut o = Outcome::ok(j.clone(), j["says"].as_str().unwrap_or("").to_owned());
        o.exit = u8::from(j["state"] != "done");
        return Ok(o);
    }
    if let Some(id) = &a.refine {
        let j = crate::scenario_refine::refine_one(&b, &args, id, &mut |_| {})?;
        let mut lines = vec![j["says"].as_str().unwrap_or("").to_owned()];
        for r in j["refine"]["rounds"].as_array().into_iter().flatten() {
            lines.push(format!(
                "  {} {} -> {}",
                r["param"].as_str().unwrap_or(""),
                r["old"].as_str().unwrap_or(""),
                r["new"].as_str().unwrap_or("")
            ));
        }
        let mut o = Outcome::ok(j.clone(), lines.join("\n"));
        o.exit = u8::from(j["passes"] != true);
        return Ok(o);
    }
    let (model, text) = b.model()?;
    let started = b.started(&model, &text, a.at)?;
    let list = sc::scenarios(&model, &b.hosted.exts, &played, a.tier);
    let results = sc::score(&b, &model, &text, &started, &list, true, &mut |_, _, _| {})?;
    let history = sc::record_history(&b, &started, &text, "first run", &results);
    let j = sc::ladder_json(
        &model,
        &started,
        &list,
        &results,
        bar,
        &[],
        &history,
        &a.recording,
    );
    let mut lines = vec![
        format!("start: {}", j["start"]["says"].as_str().unwrap_or("")),
        format!(
            "  the original's frame {} as the model's: {} loaded, {} approximated, {} missing",
            started.original_frame, started.loaded, started.approximated, started.missing
        ),
    ];
    let mut tier_now = None;
    for row in j["scenarios"].as_array().into_iter().flatten() {
        let t = row["scenario"]["tier"].as_u64();
        if t != tier_now {
            tier_now = t;
            let tj = j["tiers"]
                .as_array()
                .and_then(|ts| ts.iter().find(|x| x["tier"].as_u64() == t));
            lines.push(format!(
                "tier {}: {} of {} exact",
                t.unwrap_or(0),
                tj.map_or(0, |x| x["exact"].as_u64().unwrap_or(0)),
                tj.map_or(0, |x| x["total"].as_u64().unwrap_or(0))
            ));
        }
        let mark = match row["state"].as_str() {
            Some("passed") => "pass",
            Some("locked") => "lock",
            Some("open") => "open",
            _ => "FAIL",
        };
        lines.push(format!(
            "  {mark}  {}  [{}]{}",
            row["says"].as_str().unwrap_or(""),
            row["result"]["headline"]["says"].as_str().unwrap_or(""),
            row["trend"]
                .as_str()
                .map(|t| format!("  {t}"))
                .unwrap_or_default()
        ));
        for l in row["result"]["screen"]["says"]
            .as_array()
            .into_iter()
            .flatten()
        {
            lines.push(format!("        screen: {}", l.as_str().unwrap_or("")));
        }
    }
    lines.push(j["says"].as_str().unwrap_or("").to_owned());
    let mut o = Outcome::ok(j.clone(), lines.join("\n"));
    o.exit = u8::from(j["next"] != Json::Null);
    Ok(o)
}
