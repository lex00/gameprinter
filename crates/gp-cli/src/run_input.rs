//! The input a code-reading command plays on the original: game-printer's
//! `interp::InputArgs` (`--input`, `--baseline`, `--recording`), for `gp
//! cycles` and `gp interp draws`.
//!
//! Changed from game-printer by gameprinter's layout: a recording's input
//! log is read from the store directly (the old code went through
//! gp-oracle), and there is no sealed-run guard here yet; wave 1 brings the
//! guard back. Waves 1b and 2 bring their own copy of these flags for `gp
//! interp`; once all three have landed they become one.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result, anyhow, bail};
use gp_evidence::{InputLog, Paths, Store};

#[derive(clap::Args, Debug, Clone)]
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
    /// The input log.
    pub(crate) fn read(&self, paths: &Paths) -> Result<InputLog> {
        match (&self.input, &self.baseline, &self.recording) {
            (Some(p), _, _) => InputLog::parse(
                &fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
            )
            .map_err(|e| anyhow!("{}: {e}", p.display())),
            (None, Some(b), _) => gp_evidence::baseline::Baseline::load(b)
                .with_context(|| format!("reading {}", b.display()))?
                .input()
                .map_err(|e| anyhow!("{}: {e}", b.display())),
            (None, None, Some(id)) => {
                let store = Store::open(&paths.store)?;
                let entry = gp_evidence::recordings::resolve(&store, id)?;
                let m = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
                let text = String::from_utf8(store.get(&m.input_log)?)
                    .context("the recording's input log isn't text")?;
                Ok(InputLog::parse(&text)?)
            }
            (None, None, None) => bail!("give --input, --baseline or --recording"),
        }
    }
}
