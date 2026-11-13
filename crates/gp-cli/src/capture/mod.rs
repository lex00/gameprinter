//! `gp capture`: clips the original's code chooses, made from a game's
//! package (docs/research/dense-clips.md, docs/research/code-steered-clips.md).
//!
//! In game-printer these were runs of `gp serve`'s capture loop, reading
//! a capture run (`$GP_HOME/capture/RUN`: the ROM, the model, starts with
//! snapshots, recordings, the generated clips and their scores). gameprinter
//! has no capture runs yet: wave 7 of the port brings the loop back. Until
//! then a command here reads a game's package folder instead, through
//! [`Inputs`], the one place a capture run will plug in: its starts, its
//! mapping and its ROM. Each writes a clip set (`clips.toml` with its
//! `.presses` files) into a scratch folder, which `gp corpus` and `gp clip`
//! read as any other.

pub(crate) mod carry;
pub(crate) mod dense;
pub(crate) mod run;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_interp::mapping::Mapping;

use crate::Outcome;

#[derive(Subcommand)]
pub enum CaptureCmd {
    /// Dense clips (docs/research/dense-clips.md) are a few short clips that
    /// each run much more of the original's code. Candidates from a
    /// coverage-guided search of presses at the game's clip starts, valued
    /// by the code, branches and measure values they reach on the original,
    /// chosen by greedy set cover. They are written as a clip set into a
    /// scratch folder (`--into`); the game read from is unchanged. The same
    /// seed, ROM and game give the same clips.
    Dense(Box<dense::DenseArgs>),
    /// What the game's clip starts can't carry into the rebuilt game
    /// (gp_interp::carry), for each start and, with `--every`, for frames
    /// sampled through the starts' presses. It lists every loaded object a
    /// start there can't carry and why, grouped by cause, with the starts
    /// each gap alone holds back.
    Carry(carry::CarryArgs),
    /// List the runs: every game of the games checkout, with the round and
    /// state of `gp serve`'s background work on it.
    List {
        #[command(flatten)]
        games: run::Games,
    },
    /// Print a run's events after a sequence number, one JSON object a
    /// line, from the running `gp serve`.
    Events {
        run: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[command(flatten)]
        server: run::Server,
    },
    /// Print the event format's JSON Schema (`schema/capture-events.json`).
    Schema,
    /// The run's background queue (docs/show.md, "The background work").
    /// It shows the queue's state, or scores clips here in the foreground.
    /// A job scores the next active clip not scored on the model as it
    /// stands. While the queue is on, `gp serve` does this work itself, and
    /// refines the working copy on the clip that drifts apart earliest.
    Background(run::BackgroundArgs),
    /// The run's fidelity: `gp corpus score` on the run's game, every clip
    /// made again from the ROM and played through the model, with its
    /// fidelity line and record (game-printer's `capture fidelity`).
    Fidelity(run::ScoreArgs),
    /// Make a game's kept clip set again from the ROM and check each clip's
    /// curve against the set's: `gp corpus score TEMPLATE --run RUN`
    /// (game-printer's `capture regenerate`). Exits 1 when a curve differs
    /// or a clip can't be made.
    Regenerate {
        /// The game's folder, holding its corpus.
        #[arg(long)]
        template: PathBuf,
        /// The run to make them into: its starts and caches under $GP_HOME.
        #[arg(long)]
        run: String,
        /// The game's built module (default: built from its hook code).
        #[arg(long)]
        module: Option<PathBuf>,
        /// The ROM; by default the one in $GP_ROM_DIR the package names.
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// What a change since an earlier score reaches (docs/gate.md,
    /// "Affected clips"), and how many of the run's clips it would score
    /// again by what each read. Scores none: `gp corpus score GAME --run
    /// RUN --reuse FILE --impact`.
    Impact {
        #[command(flatten)]
        score: run::ScoreArgs,
        /// An earlier `gp --json corpus score` output of the run's game.
        #[arg(long)]
        reuse: PathBuf,
    },
    /// Make every clip again from the ROM and check every curve hash:
    /// `gp corpus verify GAME --run RUN`. Exits 1 on any difference.
    Verify(run::ScoreArgs),
}

pub fn run(cmd: &CaptureCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        CaptureCmd::Dense(a) => dense::cli(paths, a),
        CaptureCmd::Carry(a) => carry::cli(paths, a),
        CaptureCmd::List { games } => run::list(paths, games),
        CaptureCmd::Events { run, after, server } => run::events(run, *after, server),
        CaptureCmd::Schema => Ok(run::schema()),
        CaptureCmd::Background(a) => run::background(paths, a),
        CaptureCmd::Fidelity(a) => {
            let game = run::game_of(paths, &a.games, &a.run)?;
            run::corpus_score(paths, &game, &a.run, a, None, false)
        }
        CaptureCmd::Regenerate {
            template,
            run: r,
            module,
            rom,
        } => run::corpus_score(
            paths,
            template,
            r,
            &run::ScoreArgs {
                run: r.clone(),
                games: run::Games::default(),
                module: module.clone(),
                rom: rom.clone(),
            },
            None,
            false,
        ),
        CaptureCmd::Impact { score, reuse } => {
            let game = run::game_of(paths, &score.games, &score.run)?;
            run::corpus_score(paths, &game, &score.run, score, Some(reuse.clone()), true)
        }
        CaptureCmd::Verify(a) => {
            let game = run::game_of(paths, &a.games, &a.run)?;
            run::corpus_verify(paths, &game, a)
        }
    }
}

/// A start a clip runs from: the original's snapshot there.
#[derive(Debug, Clone)]
pub(crate) struct StartPoint {
    /// `FILE@FRAME`: the start's presses file and its frame.
    pub id: String,
    /// The presses from power-on, as the clip set names them.
    pub presses_file: String,
    pub frame: u64,
    pub snapshot: Vec<u8>,
}

/// What a capture command reads: the ROM, the state mapping, the starts
/// the clips run from, and the game's own clips (the pool the new ones
/// are compared with). From a game's package folder now; wave 7's capture
/// runs give the same.
pub(crate) struct Inputs {
    /// The game's folder, which the clip set's presses files are under.
    pub dir: PathBuf,
    pub game: String,
    pub rom: Vec<u8>,
    pub mapping: Mapping,
    pub starts: Vec<StartPoint>,
    /// The game's clips: the start each runs from and its presses.
    pub pool: Vec<(String, usize, Vec<u8>)>,
}

impl Inputs {
    /// A game's package: the scored ROM (`--rom` or `$GP_ROM_DIR` by
    /// hash), mapping.toml, and each distinct start of its clip set (its
    /// corpus plan, else clips.toml), the original run to it once.
    pub(crate) fn of_game(paths: &Paths, dir: &Path, rom: Option<&Path>) -> Result<Inputs> {
        let rom = crate::clip_score::rom_of(paths, dir, rom)?;
        let mfile = dir.join(gp_interp::mapping::MAPPING_FILE);
        let mapping = Mapping::parse(
            &fs::read_to_string(&mfile).with_context(|| format!("reading {}", mfile.display()))?,
        )
        .map_err(|e| anyhow::anyhow!("{}: {e}", mfile.display()))?;
        let set = crate::clipset::read_game(dir)?;
        let mut by_start: BTreeMap<(String, u64), usize> = BTreeMap::new();
        let mut want: Vec<(String, u64)> = Vec::new();
        let mut pool = Vec::new();
        for c in &set.clips {
            let Some(p) = &c.start.presses_from_power_on else {
                continue;
            };
            let key = (p.clone(), c.start.frame);
            let k = *by_start.entry(key.clone()).or_insert_with(|| {
                want.push(key);
                want.len() - 1
            });
            let spans = match &c.presses_file {
                Some(f) => crate::clipset::parse_presses(&fs::read_to_string(dir.join(f))?)?.spans,
                None => c.presses.clone(),
            };
            // An arcade clip's spans name the board's buttons, in its start's
            // layout.
            let arcade = crate::clipset::parse_presses(&fs::read_to_string(dir.join(p))?)?.arcade;
            let log = gp_evidence::InputLog::parse(&crate::clipset::log_of(
                c.frames,
                &spans,
                arcade.as_ref(),
            )?)?;
            pool.push((
                c.id.clone(),
                k,
                log.frames.iter().map(|f| f.pads[0]).collect(),
            ));
        }
        anyhow::ensure!(
            !want.is_empty(),
            "{}'s clips start from no presses file, so there is no start to run from",
            dir.display()
        );
        let snaps = crate::pool::map(
            &want,
            || (),
            |_, _, (file, frame)| -> Result<Vec<u8>> {
                let p = crate::clipset::parse_presses(&fs::read_to_string(dir.join(file))?)?;
                let log = gp_evidence::InputLog::parse(&crate::clipset::log_of_presses(&p)?)?;
                let upto = usize::try_from(*frame)?.min(log.frames.len());
                let mut o = crate::original::OriginalRuntime::of_rom(&rom)
                    .map_err(|e| anyhow::anyhow!("the ROM: {e}"))?;
                let inputs: Vec<gp_runtime::InputFrame> = log.frames[..upto]
                    .iter()
                    .map(|f| gp_runtime::InputFrame {
                        reset: f.reset,
                        ..gp_runtime::InputFrame::pads(f.pads)
                    })
                    .collect();
                gp_runtime::Runtime::step(&mut o, &inputs).map_err(anyhow::Error::msg)?;
                Ok(gp_runtime::Runtime::snapshot(&o))
            },
            &mut |_| {},
        );
        let mut starts = Vec::new();
        for ((file, frame), snap) in want.into_iter().zip(snaps) {
            starts.push(StartPoint {
                id: format!("{file}@{frame}"),
                presses_file: file,
                frame,
                snapshot: snap?,
            });
        }
        let game = if set.clipset.game.is_empty() {
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            set.clipset.game.clone()
        };
        Ok(Inputs {
            dir: dir.to_path_buf(),
            game,
            rom,
            mapping,
            starts,
            pool,
        })
    }
}
