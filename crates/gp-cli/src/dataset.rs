//! `gp dataset rom` and `gp dataset gaps`: stage 0's ROM dataset
//! (docs/formats/rom-dataset.md) written from one ROM, and the open rows of
//! several read back. `gp dataset grade`: a dataset graded against its
//! game's key. `gp dataset export` and `gp dataset catalogue`: a game's
//! corpus, scores and data model as data sets (docs/research/datasets.md,
//! docs/research/hf-datasets.md). Ported from game-printer's dataset.rs.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Subcommand;
use gp_evidence::Paths;

use crate::Outcome;

#[derive(Subcommand)]
pub enum DatasetCmd {
    /// Score every clip of a game's corpus on its model and write the clips,
    /// per-frame measures, body events and first splits as tables, with a
    /// dataset card (README.md), Croissant metadata (croissant.json) and a
    /// score record of every input by hash (record.json). The clips are made
    /// again from the ROM into run RUN, as `gp corpus score` makes them.
    /// Writes only to `--out`; nothing of the game goes in, and nothing is
    /// published. A research prototype.
    ///
    /// With `--hf` DIR `--template` T it writes the package's data model as a
    /// Hugging Face style dataset repo instead. Its configs are the facts and
    /// mechanics and the mapping, plus the clips with their scores and the
    /// measures from `--run` (for a commercial game only with `--measures`),
    /// with a card, Croissant metadata and `gp-dataset.json`
    /// (docs/research/hf-datasets.md). The written files are checked against
    /// the ROM when it is in $GP_ROM_DIR, and the export fails if they hold
    /// any of it.
    Export {
        /// The run the clips are made into (its starts and caches are kept
        /// under $GP_HOME, as for `gp corpus score`).
        #[arg(long)]
        run: Option<String>,
        /// The game's package folder (games/donkey-kong) whose corpus is
        /// scored, with `--run` and `--out`.
        #[arg(long, value_name = "DIR")]
        game: Option<PathBuf>,
        /// The directory to write; made if missing.
        #[arg(long, value_name = "DIR", required_unless_present = "hf")]
        out: Option<PathBuf>,
        /// parquet or csv.
        #[arg(long, default_value = "parquet")]
        format: String,
        /// Write the Hugging Face layout of `--template` to DIR.
        #[arg(
            long,
            value_name = "DIR",
            requires = "template",
            conflicts_with = "out"
        )]
        hf: Option<PathBuf>,
        /// The template or game package, with `--hf`.
        #[arg(long, value_name = "DIR")]
        template: Option<PathBuf>,
        /// With `--hf` and `--run`: include the per-frame measures for a
        /// commercial game, which are held back by default.
        #[arg(long)]
        measures: bool,
        /// The game's built module, for a model with hooks (as for `gp
        /// corpus score`).
        #[arg(long)]
        module: Option<PathBuf>,
        /// The model to score; by default the game's own.
        #[arg(long)]
        model: Option<PathBuf>,
        /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE).
        #[arg(long)]
        assets: Option<PathBuf>,
        /// The ROM; by default the one in $GP_ROM_DIR the package names.
        #[arg(long)]
        rom: Option<PathBuf>,
    },
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
    /// Union the facts and mechanics of several templates, with a game
    /// column, and a taxonomy of mechanic parts: which games use each and
    /// with which options. Writes a Hugging Face style dataset repo to
    /// `--out` (docs/research/hf-datasets.md).
    Catalogue {
        /// The directory to write; made if missing.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// The templates or game packages (any directory with a model.toml).
        #[arg(required = true)]
        templates: Vec<PathBuf>,
    },
}

pub(crate) fn run(cmd: &DatasetCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        DatasetCmd::Export {
            run,
            game,
            out,
            format,
            hf,
            template,
            measures,
            module,
            model,
            assets,
            rom,
        } => {
            let how = crate::clip_score::Open {
                module: module.as_deref(),
                model: model.as_deref(),
                assets: assets.as_deref(),
                rom: rom.as_deref(),
            };
            match (hf, template, out) {
                (Some(hf), Some(t), _) => {
                    crate::dataset_hf::export(paths, t, hf, run.as_deref(), *measures, &how)
                }
                (None, _, Some(out)) => {
                    let run = run.as_deref().context("--run is needed without --hf")?;
                    let game = game.as_deref().context(
                        "--game is needed with --run: the package whose corpus is scored",
                    )?;
                    crate::dataset_export::cli(paths, game, run, &how, out, format)
                }
                _ => anyhow::bail!("give --run, --game and --out, or --hf and --template"),
            }
        }
        DatasetCmd::Grade {
            dataset,
            key,
            rom,
            counts,
        } => crate::rom_grade::cli(dataset, key.as_deref(), rom.as_deref(), *counts, paths),
        DatasetCmd::Catalogue { out, templates } => {
            crate::dataset_hf::catalogue(paths, templates, out)
        }
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
