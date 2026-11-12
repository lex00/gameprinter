//! `gp dataset rom` and `gp dataset gaps`: stage 0's ROM dataset
//! (docs/formats/rom-dataset.md) written from one ROM, and the open rows of
//! several read back. Ported from game-printer's dataset.rs; its `export`,
//! `grade` and `catalogue` are wave 9 of the port.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;
use gp_evidence::Paths;

use crate::Outcome;

#[derive(Subcommand)]
pub enum DatasetCmd {
    /// Extract one ROM into a ROM dataset (docs/formats/rom-dataset.md), from
    /// the ROM alone. The dataset is Parquet tables with a card and Croissant
    /// metadata. Its tables hold routines and branches, RAM and elements, and
    /// state machines and constants. Further tables hold mechanics and model
    /// facts, and open questions and runs. Two runs on one ROM write identical
    /// bytes. Fails if a written file holds bytes of the ROM.
    Rom {
        /// The iNES ROM.
        rom: PathBuf,
        /// The directory to write; made if missing.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Write every table with no rows: the null tool the grader is
        /// checked with.
        #[arg(long)]
        empty: bool,
        /// Runs of seeded random presses, besides power-on idle and Start
        /// presses (each 3600 frames).
        #[arg(long, default_value_t = 4)]
        runs: usize,
        /// The seed of the random presses.
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
    /// What ROM datasets still leave open, ranked: open rows by kind and
    /// by what would settle them, RAM bytes by the role the runs suggest,
    /// and constants with no unit, per dataset and over all of them. Reads
    /// the datasets only, never a key: the list the next rule is picked
    /// from (docs/research/extraction-ledger.md).
    Gaps {
        /// ROM dataset folders.
        #[arg(required = true)]
        dirs: Vec<PathBuf>,
    },
}

pub(crate) fn run(cmd: &DatasetCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        DatasetCmd::Gaps { dirs } => crate::rom_dataset::gaps(dirs),
        DatasetCmd::Rom {
            rom,
            out,
            empty,
            runs,
            seed,
        } => crate::rom_dataset::cli(rom, out, *empty, *runs, *seed, paths),
    }
}
