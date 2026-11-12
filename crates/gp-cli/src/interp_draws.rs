//! `gp interp draws`: the original's random draws recorded at the state
//! mapping's `[[draws]]` sites, or with `--find` the draw sites found from a
//! run with no model ([`crate::draws_find`]). Brought from game-printer's
//! interp.rs with its flags and output.
//!
//! Changed from game-printer by gameprinter's layout: the input comes from
//! [`crate::run_input`], with no sealed-run guard until wave 1 brings it
//! back. On an arcade romset (port wave 6) `--find` reads the main Z80's
//! code ([`crate::draws_find_z80`]) and the draws at a mapping's sites are
//! recorded on gp-arcade (crate::lockstep::record_draws_arcade).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_interp::mapping::{MAPPING_FILE, Mapping};
use serde_json::json;

use crate::Outcome;
use crate::run_input::InputArgs;

#[derive(clap::Args)]
pub struct DrawsArgs {
    /// A model directory holding model.toml and mapping.toml.
    #[arg(required_unless_present = "find")]
    pub model: Option<PathBuf>,
    /// The original game's ROM.
    #[arg(long, required_unless_present = "find", conflicts_with = "find")]
    pub rom: Option<PathBuf>,
    /// Find the random register and its draw sites on this ROM: bytes
    /// stepped by a shift, an exclusive or or an add on most frames,
    /// and the reads of them whose value decides a branch. Prints the
    /// proposed `[[draws]]` entries (`--out` writes them).
    #[arg(long, value_name = "ROM")]
    pub find: Option<PathBuf>,
    #[command(flatten)]
    pub input: InputArgs,
    /// The state mapping, instead of the model directory's mapping.toml.
    #[arg(long, value_name = "FILE")]
    pub mapping: Option<PathBuf>,
    /// Record only draws after this frame of the original, the frame a
    /// lockstep starts from.
    #[arg(long, value_name = "FRAME")]
    pub from: Option<u64>,
    /// Where to write the script, or with `--find` the proposed
    /// `[[draws]]` entries (default: standard output).
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

/// The state mapping: the given file, or the model directory's (a model
/// argument may name its model.toml).
fn load_mapping(model: &Path, mapping: Option<&Path>) -> Result<Mapping> {
    let file = match mapping {
        Some(f) => f.to_owned(),
        None if model.is_dir() => model.join(MAPPING_FILE),
        None => model.parent().unwrap_or(Path::new(".")).join(MAPPING_FILE),
    };
    Mapping::parse(
        &fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))
}

fn read_rom(rom: &Path) -> Result<Vec<u8>> {
    let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    crate::original::nes_only(
        crate::original::Platform::of_file(&bytes),
        "a random draw (`gp interp draws`)",
    )?;
    Ok(bytes)
}

pub(crate) fn run(a: &DrawsArgs, paths: &Paths) -> Result<Outcome> {
    let from = a.from.unwrap_or(0);
    if let Some(rom) = &a.find {
        let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
        let rom = if crate::original::Platform::of_file(&bytes) == crate::original::Platform::Arcade
        {
            bytes
        } else {
            read_rom(rom)?
        };
        let log = a.input.read(paths)?;
        let hand = match (&a.model, &a.mapping) {
            (None, None) => None,
            (Some(m), map) => Some(load_mapping(m, map.as_deref())?),
            (None, Some(map)) => Some(load_mapping(map, Some(map))?),
        };
        return Ok(find(&rom, &log.frames, from, hand.as_ref(), a.out.as_deref())?.0);
    }
    let (Some(model), Some(rom)) = (&a.model, &a.rom) else {
        bail!("give a model and --rom, or --find ROM");
    };
    let rom = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    let log = a.input.read(paths)?;
    let mapping = load_mapping(model, a.mapping.as_deref())?;
    let script = crate::lockstep::record_draws(&rom, &mapping, &log.frames, from)?;
    let mut counts = BTreeMap::<&str, usize>::new();
    for d in &script.draws {
        *counts.entry(&d.site).or_default() += 1;
    }
    let summary = format!(
        "{} random draws after frame {from}: {}",
        script.draws.len(),
        counts
            .iter()
            .map(|(s, n)| format!("{s} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let text = match &a.out {
        Some(p) => {
            fs::write(p, script.text()).with_context(|| format!("writing {}", p.display()))?;
            format!("{summary}\nwrote {}", p.display())
        }
        None => format!("{}{summary}", script.text()),
    };
    Ok(Outcome {
        json: json!({"draws": script.draws.len(), "sites": counts}),
        text,
        exit: 0,
    })
}

/// `--find` on an input from power-on: the output, and the proposed
/// `[[draws]]` entries (empty when none is proposed).
pub(crate) fn find(
    rom: &[u8],
    inputs: &[gp_evidence::input_log::InputFrame],
    from: u64,
    hand: Option<&Mapping>,
    out: Option<&Path>,
) -> Result<(Outcome, String)> {
    let found = if crate::original::Platform::of_file(rom) == crate::original::Platform::Arcade {
        let set = gp_arcade::Romset::from_zip(rom)?;
        crate::draws_find_z80::find(&set, inputs, from)?
    } else {
        crate::draws_find::find(rom, inputs, from)?
    };
    let hand_sites = hand.map_or(&[][..], |m| &m.draws[..]);
    let vs = hand.map(|_| crate::draws_find::against(&found, hand_sites));
    let toml = crate::draws_find::proposal(&found, hand_sites);
    let mut text = crate::draws_find::text(&found, vs.as_ref());
    let any = found.proposed().next().is_some();
    match out {
        Some(p) => {
            fs::write(p, &toml).with_context(|| format!("writing {}", p.display()))?;
            text += &format!("\nwrote the proposed [[draws]] to {}", p.display());
        }
        None if any => text += &format!("\n\n{toml}"),
        None => {}
    }
    let outcome = Outcome {
        json: crate::draws_find::to_json(&found, vs.as_ref()),
        text,
        exit: 0,
    };
    Ok((outcome, if any { toml } else { String::new() }))
}
