//! `gp interp translate`: the original's state as a model state, through
//! the state mapping read in reverse (gp_interp::translate).
//!
//! Ported from game-printer's lockstep.rs (`start_at`, `translate_cmd`).
//! The original starts through [`OriginalRuntime`], so `--snapshot` is a
//! snapshot of whichever emulator the ROM's platform runs (gp-nes's, or
//! gp-arcade's), and the model runs through [`Source`].

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_interp::lockstep;
use gp_interp::mapping::Mapping;
use gp_runtime::{LoadNote, Runtime};
use serde_json::json;

use crate::hosted::Source;
use crate::lockstep::notes_json;
use crate::original::OriginalRuntime;

/// The original's state at its frame `from` as a model state, the way a
/// lockstep from there starts: the first frame the model stands for at or
/// after it (a lag frame, or a frame before the mapping's start, moves it
/// on), translated through the state mapping.
pub(crate) fn start_at(
    source: &Source,
    mapping: &Mapping,
    rom: &[u8],
    inputs: &[gp_runtime::InputFrame],
    from: u64,
) -> Result<lockstep::Start> {
    let original = OriginalRuntime::of_rom(rom)?;
    let mut interp = source.open()?;
    let spec = source.spec.clone();
    let (_, start) = lockstep::start_at(&original, &mut interp, &spec, mapping, inputs, from)
        .map_err(anyhow::Error::msg)?;
    Ok(start)
}

fn count(notes: &[LoadNote], how: gp_runtime::LoadHow) -> usize {
    notes.iter().filter(|n| n.how == how).count()
}

/// `gp interp translate`: the original's state at a frame of an input, or
/// in a snapshot file, as a model state.
pub(crate) fn translate_cmd(
    source: &Source,
    mapping: &Mapping,
    rom: &[u8],
    inputs: Option<&[gp_runtime::InputFrame]>,
    frame: Option<u64>,
    snapshot: Option<&Path>,
    out: Option<&Path>,
) -> Result<crate::Outcome> {
    let (notes, state, at) = match (inputs, frame, snapshot) {
        (_, _, Some(snap)) => {
            let bytes =
                std::fs::read(snap).with_context(|| format!("reading {}", snap.display()))?;
            let mut original = OriginalRuntime::of_rom(rom)?;
            original.restore(&bytes).map_err(anyhow::Error::msg)?;
            let base = gp_sim::World::power_on(&source.spec);
            let t = gp_interp::translate::from_original(&original, &source.spec, &base, mapping)
                .map_err(anyhow::Error::msg)?;
            (t.notes.clone(), t.state(), original.frame())
        }
        (Some(inputs), Some(f), None) => {
            let s = start_at(source, mapping, rom, inputs, f)?;
            (s.notes, s.state, s.original_frame)
        }
        _ => bail!("give --frame with the input, or --snapshot"),
    };
    if let Some(o) = out {
        std::fs::write(o, &state).with_context(|| format!("writing {}", o.display()))?;
    }
    let mut lines = vec![format!(
        "the original's frame {at} as a model state: {} loaded, {} approximated, {} missing",
        count(&notes, gp_runtime::LoadHow::Loaded),
        count(&notes, gp_runtime::LoadHow::Approximated),
        count(&notes, gp_runtime::LoadHow::Missing),
    )];
    for n in &notes {
        lines.push(format!("{:12}  {}  {}", n.how.name(), n.path, n.why));
    }
    if let Some(o) = out {
        lines.push(format!("wrote {} ({} bytes)", o.display(), state.len()));
    }
    Ok(crate::Outcome::ok(
        json!({
            "original_frame": at,
            "notes": notes_json(&notes),
            "state_bytes": state.len(),
            "out": out,
        }),
        lines.join("\n"),
    ))
}

/// `gp interp translate`'s arguments.
#[derive(clap::Args)]
pub struct TranslateArgs {
    /// A model directory holding model.toml and mapping.toml.
    pub model: std::path::PathBuf,
    /// The original game's ROM.
    #[arg(long)]
    pub rom: std::path::PathBuf,
    /// The input log to play to `--frame`.
    #[arg(long)]
    pub input: Option<std::path::PathBuf>,
    /// A baseline whose holds give the input.
    #[arg(long)]
    pub baseline: Option<std::path::PathBuf>,
    /// A recording in the store (an ID prefix) whose input log to play.
    /// Not in gameprinter yet: recordings come back with `gp record`.
    #[arg(long, value_name = "ID")]
    pub recording: Option<String>,
    /// The original's frame: the first frame the model stands for at or
    /// after it is translated.
    #[arg(long, value_name = "FRAME")]
    pub frame: Option<u64>,
    /// A snapshot of the original (the runtime protocol's snapshot, of
    /// gp-nes or gp-arcade by the ROM's platform) to translate instead of
    /// running an input.
    #[arg(long, value_name = "FILE")]
    pub snapshot: Option<std::path::PathBuf>,
    /// Write the canonical model state here.
    #[arg(long, value_name = "FILE")]
    pub out: Option<std::path::PathBuf>,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<std::path::PathBuf>,
    /// The model's lifted assets (default:
    /// $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<std::path::PathBuf>,
    /// The state mapping, instead of the model directory's mapping.toml.
    #[arg(long, value_name = "FILE")]
    pub mapping: Option<std::path::PathBuf>,
}

/// `gp interp translate` from its arguments.
pub(crate) fn run(a: &TranslateArgs, paths: &gp_evidence::Paths) -> Result<crate::Outcome> {
    use crate::interp_args as ia;
    let log = match (&a.input, &a.baseline, &a.recording) {
        (None, None, None) => None,
        _ => Some(ia::read_input(
            a.input.as_deref(),
            a.baseline.as_deref(),
            a.recording.as_deref(),
        )?),
    };
    let (source, _) = ia::source(&a.model, a.assets.as_deref(), a.module.as_deref(), paths)?;
    let (_, mapping) = ia::load_mapping(&a.model, a.mapping.as_deref())?;
    let rom = ia::read_rom(&a.rom)?;
    let inputs = log.as_ref().map(crate::run::frames);
    translate_cmd(
        &source,
        &mapping,
        &rom,
        inputs.as_deref(),
        a.frame,
        a.snapshot.as_deref(),
        a.out.as_deref(),
    )
}
