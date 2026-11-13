//! `gp dataset rom` and `gp dataset gaps`: stage 0's ROM dataset
//! (docs/formats/rom-dataset.md) written from one ROM, and the open rows of
//! several read back. `gp dataset grade`: a dataset graded against its
//! game's key. Ported from game-printer's dataset.rs.

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
    /// Grade a ROM dataset against its game's key
    /// (docs/formats/rom-dataset-key.md) and print the stage 0 scorecard's
    /// column for the ROM. The coverage rows run the ROM with stage 0's
    /// starts, so their denominators never come from the dataset; they
    /// apply with no key too, and on an arcade romset. The ROM is found by
    /// the dataset's hash (the key's `rom_file`, $GP_ROM_DIR,
    /// $GP_HOME/work/sealed) unless given. `--counts` prints counts only,
    /// nothing of the key.
    Grade {
        /// The dataset folder `gp dataset rom` wrote.
        dataset: PathBuf,
        /// The game's key folder; the keyed rows need it.
        key: Option<PathBuf>,
        /// The ROM, when it can't be found by its hash.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Print counts only: no path, address, name or value of the key.
        #[arg(long)]
        counts: bool,
    },
}

pub(crate) fn run(cmd: &DatasetCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        DatasetCmd::Grade {
            dataset,
            key,
            rom,
            counts,
        } => crate::rom_grade::cli(dataset, key.as_deref(), rom.as_deref(), *counts, paths),
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
