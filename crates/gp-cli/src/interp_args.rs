//! What the `gp interp` commands ported from game-printer share: the input
//! they play (`--input`, `--baseline`, `--recording`), the model they run
//! (through its game's module, `--module`, or the built-in interpreter for
//! a model with no hooks) and its state mapping.
//!
//! Changed from game-printer's interp.rs by gameprinter's layout: the model
//! runs through [`crate::hosted::Source`], so a model with hooks needs
//! `--module`; the original is started through
//! [`crate::original::OriginalRuntime`]; and `--recording` is refused until
//! recordings come back (`gp record`, wave 3a).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::{InputLog, Paths};
use gp_interp::mapping::{MAPPING_FILE, Mapping};

use crate::hosted::Source;

#[derive(clap::Args)]
#[group(required = true, multiple = false)]
pub struct InputArgs {
    /// The input log to play.
    #[arg(long)]
    pub input: Option<PathBuf>,
    /// A baseline whose holds give the input.
    #[arg(long)]
    pub baseline: Option<PathBuf>,
    /// A recording in the store (an ID prefix) whose input log to play.
    /// Not in gameprinter yet: recordings come back with `gp record`.
    #[arg(long, value_name = "ID")]
    pub recording: Option<String>,
}

impl InputArgs {
    pub(crate) fn read(&self) -> Result<InputLog> {
        read_input(
            self.input.as_deref(),
            self.baseline.as_deref(),
            self.recording.as_deref(),
        )
    }
}

/// The input log from a file, a baseline's holds, or (not yet) a recording.
pub(crate) fn read_input(
    input: Option<&Path>,
    baseline: Option<&Path>,
    recording: Option<&str>,
) -> Result<InputLog> {
    match (input, baseline, recording) {
        (Some(p), _, _) => InputLog::parse(
            &fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
        )
        .map_err(|e| anyhow::anyhow!("{}: {e}", p.display())),
        (None, Some(b), _) => Ok(gp_evidence::baseline::Baseline::load(b)
            .with_context(|| format!("reading {}", b.display()))?
            .input()
            .map_err(|e| anyhow::anyhow!("{}: {e}", b.display()))?),
        (None, None, Some(_)) => bail!(
            "gameprinter has no recordings store yet (`gp record` isn't ported): give the input with --input or --baseline"
        ),
        (None, None, None) => bail!("give --input, --baseline or --recording"),
    }
}

/// The model directory a model argument names: DIR, or the folder of a
/// model.toml file.
pub(crate) fn model_dir(model: &Path) -> PathBuf {
    if model.is_dir() {
        model.to_owned()
    } else {
        model
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_owned()
    }
}

/// The state mapping: the given file, or the model directory's.
pub(crate) fn load_mapping(model: &Path, mapping: Option<&Path>) -> Result<(PathBuf, Mapping)> {
    let file = match mapping {
        Some(f) => f.to_owned(),
        None => model_dir(model).join(MAPPING_FILE),
    };
    let mapping = Mapping::parse(
        &fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    Ok((file, mapping))
}

/// The model as the tools run it, from its directory, its lifted assets
/// (default `$GP_HOME/work/assets/<source>`) and its game's module.
pub(crate) fn source(
    model: &Path,
    assets: Option<&Path>,
    module: Option<&Path>,
    paths: &Paths,
) -> Result<(Source, PathBuf)> {
    Source::open_dir(&model_dir(model), assets, paths, module)
}

/// The original's ROM file.
pub(crate) fn read_rom(rom: &Path) -> Result<Vec<u8>> {
    fs::read(rom).with_context(|| format!("reading {}", rom.display()))
}

/// Refuses a mapping written for another model.
pub(crate) fn same_model(file: &Path, mapping: &Mapping, model: &str) -> Result<()> {
    if mapping.model != model {
        bail!(
            "{} is the state mapping of `{}`, not of `{model}`",
            file.display(),
            mapping.model
        );
    }
    Ok(())
}
