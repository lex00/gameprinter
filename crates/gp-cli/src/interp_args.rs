//! What the `gp interp` commands ported from game-printer share: the input
//! they play (`--input`, `--baseline`, `--recording`), the model they run
//! (through its game's module, `--module`, or the built-in interpreter for
//! a model with no hooks) and its state mapping.
//!
//! Changed from game-printer's interp.rs by gameprinter's layout: the model
//! runs through [`crate::hosted::Source`], so a model with hooks needs
//! `--module`; the original is started through
//! [`crate::original::OriginalRuntime`]; and `--recording` reads a store
//! recording's input log, NES or arcade, as `gp interp run` does.

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
    #[arg(long, value_name = "ID")]
    pub recording: Option<String>,
}

impl InputArgs {
    pub(crate) fn read(&self, paths: &Paths) -> Result<InputLog> {
        read_input(
            self.input.as_deref(),
            self.baseline.as_deref(),
            self.recording.as_deref(),
            paths,
        )
    }
}

/// The input log from a file, a baseline's holds, or a store recording's.
pub(crate) fn read_input(
    input: Option<&Path>,
    baseline: Option<&Path>,
    recording: Option<&str>,
    paths: &Paths,
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
        (None, None, Some(id)) => {
            let o = crate::open::Opened::new(paths, id)?;
            let bytes = o.store.get(&o.manifest.input_log)?;
            InputLog::parse(&String::from_utf8_lossy(&bytes))
                .map_err(|e| anyhow::anyhow!("recording {}: {e}", o.short()))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `--recording` reads a store recording's input log: a made-up arcade
    /// program's (gp_arcade::testrom::busy, no game's bytes), where it was
    /// refused as if there were no store.
    #[test]
    fn a_recording_s_input_is_read_from_the_store() {
        let home = tempfile::tempdir().unwrap();
        let paths = crate::test_paths(home.path());
        let store = gp_evidence::Store::open(&paths.store).unwrap();
        let set = gp_arcade::testrom::busy();
        let input = InputLog::parse(
            "gp-arcade-input 0\nsystem test\ncolumns CSRLDUT\n5 .......\n4 ......T\n1 C......\n",
        )
        .unwrap();
        let rec = crate::arcade_rec::record(
            &store,
            crate::arcade_rec::Request {
                set: &set,
                source_id: None,
                input: &input,
                checkpoint_interval: 5,
                capture: gp_nes::recording::Capture::ALL,
                provenance: gp_evidence::manifest::Provenance::Scripted,
                name: "busy".into(),
                created_at: "2026-11-14T00:00:00Z".into(),
                notes: None,
            },
        )
        .unwrap();
        let id = rec.id.to_hex();
        let log = read_input(None, None, Some(&id[..8]), &paths).unwrap();
        assert_eq!(log.frames, input.frames);
    }
}
