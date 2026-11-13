//! `gp env`, `gp noop` and `gp input-log check/generate`, as game-printer's
//! main.rs had them (wave 9 of the port). `input-log generate` also writes
//! an arcade log (`--arcade SYSTEM --columns COLUMNS`, the layout a game's
//! presses files name), and `input-log check` reads one, so both work for
//! Tapper as for an NES game.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;
use gp_evidence::input_log::{ArcadeLayout, InputLog};
use gp_evidence::{Hash, Paths, Store};
use serde_json::json;

use crate::{Outcome, runrec};

#[derive(Subcommand)]
pub enum InputLogCmd {
    /// Parse a log and report its frame and lag counts.
    Check {
        /// The input log file.
        file: PathBuf,
    },
    /// Write an input log from button spans, for scripted recordings.
    Generate {
        /// Number of frames.
        #[arg(long, required_unless_present = "baseline")]
        frames: Option<usize>,
        /// Buttons held over a frame range, e.g. `start@120-124` or `right+b@130-`
        /// (to the end). Repeat for more spans. Buttons: a, b, select, start, up,
        /// down, left, right; for an arcade log, its layout's (coin, start1,
        /// right, left, down, up, tap).
        #[arg(long = "hold")]
        holds: Vec<String>,
        /// Take the frames and button spans from a baseline file instead
        /// (baselines/), so a recording can replay a baseline's input.
        #[arg(long, conflicts_with_all = ["frames", "holds"])]
        baseline: Option<PathBuf>,
        /// Write an arcade log for this MAME set (`tapper`), with --columns.
        #[arg(long, requires = "columns", conflicts_with = "baseline")]
        arcade: Option<String>,
        /// The arcade log's columns, one letter a button (`CSRLDUT` for
        /// Tapper), as the game's presses files name them.
        #[arg(long, requires = "arcade")]
        columns: Option<String>,
        /// Write here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

pub(crate) fn env(paths: &Paths) -> Result<Outcome> {
    let store = paths
        .store
        .exists()
        .then(|| Store::open(&paths.store).and_then(|s| s.stats()))
        .transpose()?;
    let git = gp_metrics::run::git_info();
    let v = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "paths": paths,
        "roms_dir_exists": paths.roms.exists(),
        "store": store,
        "git": git,
        "host": gp_metrics::run::Host::current(),
    });
    let text = format!(
        "gp {}\nGP_HOME     {}\nGP_STORE    {}{}\nGP_ROM_DIR  {}{}\nmetrics     {}",
        env!("CARGO_PKG_VERSION"),
        paths.home.display(),
        paths.store.display(),
        store
            .map(|s| format!("  ({} objects, {} bytes)", s.objects, s.bytes))
            .unwrap_or_else(|| "  (not created yet)".into()),
        paths.roms.display(),
        if paths.roms.exists() {
            ""
        } else {
            "  (not created yet)"
        },
        paths.metrics_db().display(),
    );
    Ok(Outcome::ok(v, text))
}

/// `gp noop`: nothing, but its run is recorded (startup cost).
pub(crate) fn noop() -> Outcome {
    Outcome::ok(json!({}), "ok".into())
}

pub(crate) fn input_log(cmd: &InputLogCmd) -> Result<Outcome> {
    match cmd {
        InputLogCmd::Check { file } => check(file),
        InputLogCmd::Generate {
            frames,
            holds,
            baseline,
            arcade,
            columns,
            out,
        } => match baseline {
            Some(file) => {
                let baseline = gp_evidence::baseline::Baseline::load(file)?;
                generate(baseline.frames, &baseline.holds, None, out.as_deref())
            }
            None => generate(
                frames.expect("clap requires --frames without --baseline"),
                holds,
                arcade
                    .as_ref()
                    .zip(columns.as_ref())
                    .map(|(s, c)| ArcadeLayout {
                        system: s.clone(),
                        columns: c.clone(),
                    }),
                out.as_deref(),
            ),
        },
    }
}

fn check(file: &Path) -> Result<Outcome> {
    let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let log = InputLog::parse(&text)?;
    runrec::metric("input_log.frames", log.frames.len() as f64, "count");
    runrec::metric("input_log.lag_frames", log.lag_frames() as f64, "count");
    Ok(Outcome::ok(
        json!({ "frames": log.frames.len(), "lag_frames": log.lag_frames(), "controllers": log.controllers }),
        log.summary(),
    ))
}

fn generate(
    frames: usize,
    holds: &[String],
    arcade: Option<ArcadeLayout>,
    out: Option<&Path>,
) -> Result<Outcome> {
    let spans: Vec<&str> = holds.iter().map(String::as_str).collect();
    let log = match &arcade {
        Some(a) => gp_evidence::input_log::generate_arcade(frames, a, &spans)?,
        None => gp_evidence::input_log::generate(frames, &spans)?,
    };
    let text = log.to_text();
    runrec::output(
        "input log",
        format!("blake3:{}", Hash::of(text.as_bytes())),
        Some(text.len() as u64),
    );
    match out {
        Some(path) => {
            fs::write(path, &text).with_context(|| format!("writing {}", path.display()))?;
            Ok(Outcome::ok(
                json!({ "frames": frames, "out": path }),
                format!("wrote {frames} frames to {}", path.display()),
            ))
        }
        None => {
            print!("{text}");
            Ok(Outcome::ok(json!({ "frames": frames }), String::new()))
        }
    }
}
