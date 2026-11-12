//! `gp`: a thin client over the library crates. It holds only the commands
//! the new world uses so far; each is one call into a library, with no
//! engine here.

mod affected;
mod agent_cmd;
mod arcade_rec;
mod assets;
mod attest;
mod author;
mod behave;
mod capture;
mod cli_doc;
mod clip;
mod clip_presses;
mod clip_score;
mod clip_screen;
mod clip_session;
mod clipset;
mod code;
mod code_z80;
mod corpus_import;
mod corpus_score;
mod coverage;
mod cycles;
mod dataset;
mod dataset_tables;
mod derive;
mod diagnose;
mod draws_find;
mod draws_find_z80;
mod element;
mod element_z80;
mod explore;
mod farm;
mod from_trace;
mod game;
mod game_new;
mod gaps;
mod gaps_arcade;
mod gate;
mod gate_append;
mod gate_copy;
mod gate_queue;
mod harvest;
mod host;
mod hosted;
mod interp;
mod interp_args;
mod interp_draws;
mod labels;
mod labels_propose;
mod leads;
mod lockstep;
mod mcp;
mod mechanic;
mod mechanics;
mod model;
mod model_blind;
mod model_static;
mod module;
mod moments;
mod movie;
mod odds;
mod open;
mod original;
mod package;
mod picture;
mod pinned;
mod plan;
mod pool;
mod publish;
mod reach;
mod reach_window;
mod recindex;
mod recordings;
mod refine;
mod registry;
mod rom_dataset;
mod rom_extract;
mod romclean;
mod romdiff;
mod rompatch;
mod roms;
mod roms_cmd;
mod rule_refine;
mod run;
mod run_input;
mod scenario;
mod scenario_refine;
mod scenarios_cmd;
mod screen;
mod sealed;
mod search;
mod serve;
mod served;
mod slots;
mod solve;
mod sound;
mod sound_ay;
mod sprites;
mod static_grade;
mod store;
mod sweep;
mod tape;
mod template;
mod template_build;
mod template_cites;
mod template_original;
mod tool_gate;
mod track;
mod translate;
mod validate;
mod watch;
mod watch_serve;
mod worker;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::Value;

/// What a command says: its text, its JSON, and its exit code (0 ok, 1 a
/// check failed).
pub struct Outcome {
    pub json: Value,
    pub text: String,
    pub exit: u8,
}

impl Outcome {
    pub fn ok(json: Value, text: String) -> Self {
        Self {
            json,
            text,
            exit: 0,
        }
    }
}

#[derive(Parser)]
#[command(name = "gp", version, about = "Game Printer")]
pub struct Cli {
    /// Print machine-readable JSON on stdout.
    #[arg(long, global = true)]
    json: bool,
    /// Helper threads for scoring clips (default: GP_JOBS, else every core
    /// less two, at least one).
    #[arg(long, global = true)]
    jobs: Option<usize>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// The work record of gp serve's clip agents and the driver: notes,
    /// attempts scored from a worktree, keep (game-printer's gp agent).
    Agent(agent_cmd::AgentArgs),
    /// Lift a ROM's assets into files and pack them back.
    Assets {
        #[command(subcommand)]
        cmd: AssetsCmd,
    },
    /// A game's scoring corpus.
    Corpus {
        #[command(subcommand)]
        cmd: corpus_score::CorpusCmd,
    },
    /// Corpus clips: their input logs and their scores on a model.
    Clip {
        #[command(subcommand)]
        cmd: clip_score::ClipCmd,
    },
    /// Check a model and its fidelity contract.
    Model {
        #[command(subcommand)]
        cmd: ModelCmd,
    },
    /// Committed baselines: scripted runs of a ROM with expected results.
    Baseline {
        #[command(subcommand)]
        cmd: BaselineCmd,
    },
    /// Run a model beside the original and compare them.
    Interp {
        #[command(subcommand)]
        cmd: InterpCmd,
    },
    /// The searches and the stage measure of a level, run through the game's
    /// module.
    Level {
        #[command(subcommand)]
        cmd: search::LevelCmd,
    },
    /// Find a recording's objects from its frame trace without a hand-written
    /// mapping: sprites followed from frame to frame, the RAM bytes that hold
    /// their positions, and a proposed `[slots]` section (docs/formats/tracks.md).
    Track(track::TrackArgs),
    /// Hold a model to its fidelity contract against recordings of the
    /// original (docs/validation.md): the contract's rules compiled into
    /// checks, each recording's input run on the original (read through the
    /// state mapping) and on the model, and compared. Exits 1 when a check
    /// fails, 2 when no recording could be run.
    Validate(validate::ValidateArgs),
    /// Report what recordings exercised, given recordings or the whole
    /// store (docs/coverage.md).
    Coverage(coverage::CoverageArgs),
    /// Find what a recording of the original shows that the model doesn't
    /// account for: unexplained changes, interactive elements and drawn
    /// box offsets. Detect and catalog only (docs/gaps.md).
    Gaps {
        #[command(subcommand)]
        cmd: gaps::GapsCmd,
    },
    /// Music and sound effects as model data: capture a recording's APU
    /// register log into songs, effects and instruments, check fidelity,
    /// and compare register logs (docs/sound.md).
    Sound {
        #[command(subcommand)]
        cmd: sound::SoundCmd,
    },
    /// Stage 0's ROM dataset: one ROM's routines, RAM, elements, state
    /// machines, constants, mechanics and open questions as Parquet tables
    /// (docs/formats/rom-dataset.md).
    Dataset {
        #[command(subcommand)]
        cmd: dataset::DatasetCmd,
    },
    /// Sprites: capture every frame of every entity from the sprite tables
    /// and recordings (docs/capture.md).
    Sprites {
        #[command(subcommand)]
        cmd: sprites::SpritesCmd,
    },
    /// Fetch a game by its exact id and lift it from a person's ROMs.
    Game {
        #[command(subcommand)]
        cmd: GameCmd,
    },
    /// One mechanic of a game, worked on by a person and their agent: the
    /// page to play and record clips in, and the agent's side of the talk.
    Mechanic {
        #[command(subcommand)]
        cmd: mechanic::MechanicCmd,
    },
    /// A game's labels: names for its RAM, routines and data tables
    /// (docs/formats/labels.md).
    Labels {
        #[command(subcommand)]
        cmd: labels::LabelsCmd,
    },
    /// The rule behind a mechanic, found from the clips and checked frame by
    /// frame against the original; the mechanic page shows it at /refine.
    Rule {
        #[command(subcommand)]
        cmd: rule_refine::RuleCmd,
    },
    /// Game Printer's page, at http://127.0.0.1:7710/: a game's tabs CLIPS
    /// (record clips of the original, hand them to your agent, review them
    /// on the model), REFINE (the background work scoring and refining the
    /// rebuilt game), PLAY (the rebuild) and THEMES, with your agent's log
    /// beside them. gp mechanic start and watch open it.
    Serve {
        #[command(flatten)]
        opts: serve::ServeOpts,
    },
    /// The ROMs you own, kept in $GP_ROM_DIR: add, list and clean them.
    Roms {
        #[command(subcommand)]
        cmd: roms_cmd::RomsCmd,
    },
    /// Sealed runs: held-out play scored by the coordinator alone
    /// (docs/sealed.md).
    Sealed {
        #[command(subcommand)]
        cmd: sealed::SealedCmd,
    },
    /// Make, publish to and test registries.
    Registry {
        #[command(subcommand)]
        cmd: RegistryCmd,
    },
    /// Ask about a package folder.
    Package {
        #[command(subcommand)]
        cmd: PackageCmd,
    },
    /// Check a template's pins against a ROM.
    Template {
        #[command(subcommand)]
        cmd: TemplateCmd,
    },
    /// A worker's setup on a shared machine.
    Worker {
        #[command(subcommand)]
        cmd: worker::WorkerCmd,
    },
    /// Land a patch to the tool itself (the gameprinter repo).
    Tool {
        #[command(subcommand)]
        cmd: ToolCmd,
    },
    /// Search for presses that run code no run has run yet, with no model
    /// (gp-explore, docs/research/rom-event-model.md). It starts at the end of
    /// each of stage 0's own starts and `--start`'s clips, aims at the routines
    /// the code map has that no start clip ran (or those named, or the targets
    /// of the places the reader stopped), ranks each candidate by the fewest
    /// calls from a routine it ran to a target, and writes what it keeps as
    /// clips from power-on. Its output depends only on the ROM, the start and
    /// the seed
    Explore(explore::ExploreArgs),
    /// Watch stage 0 on a ROM as it runs: a session whose events (the code,
    /// banks, edges and pointer values each run shows first, the places the
    /// static reader stopped that a run went past, what the reader reads
    /// after the runs, the search's progress and kept clips) are written to
    /// $GP_HOME/watch/NAME/events.ndjson as they come, for `gp watch
    /// events --follow` and `gp mechanic watch`'s page to follow
    Watch {
        #[command(subcommand)]
        cmd: watch::WatchCmd,
    },
    /// Clips the original's code chooses, written as a clip set from a
    /// game's package: dense clips found by a coverage-guided search of
    /// presses (docs/research/dense-clips.md).
    Capture {
        #[command(subcommand)]
        cmd: capture::CaptureCmd,
    },
    /// Run a ROM from power-on through an input log and store a recording
    /// (docs/recording.md).
    Record(recordings::RecordArgs),
    /// Replay a recording and check every frame against it.
    Replay(recordings::ReplayArgs),
    /// Read raw console state such as `ram[0x0770]` or `cpu.pc` at a frame of
    /// a recording. It reads the frame from the recording's trace if it has
    /// one. For `bus[...]` or a recording without a trace it restores the
    /// nearest checkpoint and replays, and exits 1 if the state it reaches
    /// isn't the one recorded.
    Query(recordings::QueryArgs),
    /// Run a recording until a condition over raw state holds, such as
    /// `ram[0x0770] == 1`, and print the frame where it does. A recording
    /// with a trace needs no replay up to its last frame. It exits 1 if the
    /// condition never holds, or if the state it stops at isn't the one
    /// recorded.
    RunUntil(recordings::RunUntilArgs),
    /// Traces of a recording: its full state at any frame, and CPU
    /// instructions over a window of frames.
    Trace {
        #[command(subcommand)]
        cmd: recordings::TraceCmd,
    },
    /// List a recording's events (interrupts, sprite 0 hits, bank switches,
    /// controller reads, register writes). Read from its event stream, or by
    /// replay when it has none.
    Events(recordings::EventsArgs),
    /// Stored recordings.
    Recordings {
        #[command(subcommand)]
        cmd: recordings::RecordingsCmd,
    },
    /// FCEUX and BizHawk movies, imported as input logs (docs/movies.md).
    Movie {
        #[command(subcommand)]
        cmd: movie::MovieCmd,
    },
    /// Read a ROM's code with what runs of it did: listings with each
    /// instruction's count and each branch's ways, and the tables a RAM
    /// byte indexes (docs/code-reading.md).
    Code {
        #[command(subcommand)]
        cmd: code::CodeCmd,
    },
    /// Run a ROM under clips and say what its code did. It lists the PRG bytes
    /// the code executed and every transfer of control by bank, with the
    /// pointer values at each `jmp (ptr)`. gp-events names these levels L0 and
    /// L1 (docs/research/rom-event-model.md). Each clip runs from power-on: its
    /// start's presses (or a recording's input) and then its own. Addresses are
    /// named as `gp model static` names them; the same ROM and clips give the
    /// same output. With --window or --diff-*, it reports by routine of the
    /// code map instead, and can compare two sides (docs/code-reading.md)
    Reach(reach::ReachArgs),
    /// Measure an original's per-frame CPU work, for a model of a random
    /// generator stepped in an idle loop (docs/cycles.md). Analysis only.
    Cycles {
        #[command(subcommand)]
        cmd: cycles::CyclesCmd,
    },
    /// Read one game element's code from the original's run: the routine
    /// that wrote a measure's byte on a clip's frame and every input it
    /// read (the element-extraction skill, steps 2 and 3).
    Element {
        #[command(subcommand)]
        cmd: element::ElementCmd,
    },
    /// The picture-gap detector: a scored run's clips played again, and the
    /// screen measure's boxes on each compared frame put down to a cause.
    Picture(picture::PictureArgs),
    /// Each non-exact clip of a scored run, its detector class and its
    /// proposals for the model's numbers. Never writes the model.
    Solve(solve::SolveArgs),
    /// A prototype of the authoring API: a proposer edits a base
    /// template's mechanic parameters from a short request, gp's checks
    /// verify each proposal (schema, mechanics, the public tier, reach, a
    /// playthrough, difficulty against a target, the asked-for feel), the
    /// failures feed the next proposal, and one log row is written
    /// (docs/author.md). The proposer is scripted; only `--dry-run` is
    /// built.
    Author(author::AuthorArgs),
    /// A decision model plays a level, by search from savestates: it
    /// orders the moves, the emulator says which work
    /// (docs/research/laya-level-play.md).
    Farm(farm::FarmArgs),
    /// Blind mode for a top-down game (docs/research/topdown-extension.md). It
    /// plays from power-on through a class select and reads the player's bytes
    /// on both axes and the health that drains. Its search moves in eight ways,
    /// in an order a decision model gives.
    FarmTopdown(farm::topdown::TopdownArgs),
    /// The local runtime host: interpreter sessions, parallel scenario
    /// runs, background builds, playable links on localhost, and the speed
    /// targets (docs/host.md).
    Host {
        #[command(subcommand)]
        cmd: host::HostCmd,
    },
    /// Serve the agent tools over MCP on stdin and stdout, for an agent to
    /// edit a working copy of a template by model path and check it
    /// (docs/mcp.md).
    Mcp(mcp::McpArgs),
    /// The content-addressed evidence store.
    Store {
        #[command(subcommand)]
        cmd: store::StoreCmd,
    },
    /// gp about itself: its published build and its command list.
    #[command(name = "self")]
    Itself {
        #[command(subcommand)]
        cmd: publish::SelfCmd,
    },
}

#[derive(Subcommand)]
enum ToolCmd {
    /// Hold a patch to gameprinter to the tool's checks and a Donkey Kong
    /// canary, and land it when it passes (docs/gate.md).
    Gate(tool_gate::ToolGateArgs),
}

#[derive(Subcommand)]
enum AssetsCmd {
    /// Decode a ROM's assets into files: tiles as indexed PNG, palettes,
    /// metatiles, levels, tables, text and pointer tables as TOML, object
    /// lists and music as directories of TOML files, plus the asset map as
    /// assets.toml. The map is the one for the ROM's hash in $GP_ASSETS, or
    /// in the tool repo's and each game's assets/, unless one is given.
    Lift {
        /// The ROM file.
        rom: PathBuf,
        /// The directory to write, outside any git work tree (default
        /// $GP_HOME/work/assets/<source-id>). A directory that already
        /// holds a lift (an assets.toml) is refused unless `--force`, since
        /// its files may have been edited.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// The asset map, instead of finding one by the ROM's hash.
        #[arg(long)]
        map: Option<PathBuf>,
        /// Lift into a directory that already holds a lift, replacing the
        /// files of the names it writes (nothing else in it is touched).
        #[arg(long)]
        force: bool,
    },
    /// Encode a lifted directory back into ROM bytes, then compare them
    /// with a ROM (the round-trip check, which exits 1 naming each asset
    /// and offset that differs), or write a copy of a base ROM with the
    /// encoded assets in place and list the bytes that changed.
    #[command(group(clap::ArgGroup::new("mode").required(true).args(["check", "base"])))]
    Pack {
        /// A directory written by gp assets lift (its assets.toml says how
        /// to pack it).
        dir: PathBuf,
        /// The ROM to compare with: the round-trip check.
        #[arg(long, value_name = "ROM")]
        check: Option<PathBuf>,
        /// The ROM to write the assets into.
        #[arg(long, value_name = "ROM", requires = "out")]
        base: Option<PathBuf>,
        /// The ROM file to write, outside any git work tree.
        #[arg(short, long, requires = "base")]
        out: Option<PathBuf>,
        /// Replace an existing output file.
        #[arg(long, requires = "out")]
        force: bool,
        /// With --check, give the bytes compared and the differences in
        /// the JSON as PREFIX.bytes_covered and PREFIX.differences.
        #[arg(long, value_name = "PREFIX")]
        metrics: Option<String>,
    },
}

#[derive(Subcommand)]
enum ModelCmd {
    /// Check a model directory: model.toml against the schema (references,
    /// roles, events, extensions, evidence for every fact, each evidence
    /// ref's file and line, in the repo or package or in a pinned download
    /// under $GP_HOME/work, skipped with a notice when that isn't there),
    /// and fidelity.toml, if there is one, against the model. A
    /// profiles.toml beside it is checked too. Prints the facts by source.
    /// Exits 1 on any problem.
    Check {
        /// A directory holding model.toml (and optionally fidelity.toml and
        /// profiles.toml).
        dir: PathBuf,
        /// A directory written by gp assets lift: also check each asset slot
        /// filled from that asset map against the asset it names.
        #[arg(long, value_name = "DIR")]
        assets: Option<PathBuf>,
        /// The directory evidence refs are relative to (default: the
        /// package folder for a game's model, else the git work tree the
        /// model is in, else the model's directory).
        #[arg(long, value_name = "DIR")]
        root: Option<PathBuf>,
    },
    /// Hold a patch to a game in the games repo to its corpus: PASS or FAIL,
    /// and what passes lands.
    Gate(gate::GateArgs),
    /// A model's mechanic catalog: every mechanic it uses, from the record
    /// the simulation's components and the model's hooks declare (system,
    /// parameters with types, units and ranges, reads and writes, events,
    /// assumptions, invariants, measures, the primitives it's built from,
    /// and for a hook the general mechanic it's like), the class of every
    /// model path (mechanic, parameter, content or presentation), the hooks
    /// that are still game-specific code, and the problems found by holding
    /// the model to the records (docs/mechanics.md). Exits 1 when the model
    /// and a record disagree.
    Mechanics(mechanics::MechanicsArgs),
    /// Stage 0, the static harvest, is every part of a model the ROM's code
    /// settles without running it, from the ROM and the asset map only
    /// (docs/plan-extraction-process.md). Elements from the code's dispatch and
    /// what each routine touches; constants, thresholds and periods; state
    /// machines from the bytes the code branches on and writes by constant;
    /// mapping rows; model facts, each with the instruction it comes from; and
    /// open facts with the routine and bytes that decide them. `--grade KEY`
    /// grades it against a game's answer key (games/mosshop/static-key.toml).
    Static(model_static::StaticArgs),
    /// Refine a working copy of a game's model against the original, clip
    /// by clip from its corpus, through the lockstep: the clips checked
    /// against the ROM first, numbers fitted from the original's state at
    /// each clip's start and written as facts with evidence, shape
    /// questions asked in questions.toml. Without --lockstep, the loop
    /// measures store recordings (--recording) as gp model propose does and
    /// runs gp validate between rounds: a measured fact a failing check
    /// backs is written, answered questions are written as stated facts,
    /// the rest asked.
    Refine(refine::RefineArgs),
    /// A model and a state mapping from the ROM alone, by playing it. Blind
    /// discovery finds the player and its buttons. Then runs of the game
    /// measure the player's speeds, jump and size. Facts are `observed`,
    /// each with the frames it came from (crates/gp-cli/src/model_blind.rs).
    Blind(model_blind::BlindArgs),
    /// Derive the model's hand-written facts from what their evidence
    /// cites (docs/derivation.md): the constant, immediate or data row a ref
    /// names in assembly source (evidence), the value the routine's code
    /// computes there by constant propagation (code), the table the line's
    /// label names lifted through the asset map, the asset slots against the
    /// map, and the generator scripts the evidence names rerun over the
    /// lifted assets (data, with `--assets`), and the feel parameters
    /// measured from recordings (recordings, with `--recording`). Each fact
    /// either agrees or differs, or is new or underivable (with the class
    /// that could derive it and why); a question goes into questions.toml
    /// where a derived value differs.
    Derive {
        dir: PathBuf,
        /// Which derivers: evidence, code, data, recordings or all (repeat
        /// or comma-separate).
        #[arg(long, default_value = "all")]
        from: Vec<String>,
        /// A directory gp assets lift wrote, for `--from` data.
        #[arg(long, value_name = "DIR")]
        assets: Option<PathBuf>,
        /// Recordings (IDs with frame traces) for `--from` recordings.
        #[arg(long = "recording", num_args = 1..)]
        recordings: Vec<String>,
        #[arg(long, default_value = "1..")]
        frames: String,
        /// The directory refs are relative to (default: the game's package
        /// folder when DIR is one, else the git work tree DIR is in).
        #[arg(long, value_name = "DIR")]
        root: Option<PathBuf>,
        /// Don't write questions.toml.
        #[arg(long)]
        no_questions: bool,
        /// List every fact, not only those that differ or are new.
        #[arg(long)]
        list: bool,
    },
    /// Measure the model's feel parameters from store recordings. gp track
    /// finds the player and the enemies, and their positions and the
    /// controller give the player's movement (walk and run speed,
    /// acceleration), its jump (speed, gravity, held gravity, fall limit and
    /// rise by hold time) and the enemies' steps, periods and speeds. Each is
    /// proposed as a fact by model path, compared with the model's own, and
    /// where the measurement can't decide, a question goes into the
    /// directory's `questions.toml` (docs/mechanics.md).
    Propose {
        dir: PathBuf,
        /// Recording IDs (or unique prefixes) with frame traces.
        #[arg(long = "recording", required = true, num_args = 1..)]
        recordings: Vec<String>,
        /// Which frames: `N`, `A..B`, `A..=B` or `A..`.
        #[arg(long, default_value = "1..")]
        frames: String,
        /// Don't write questions.toml.
        #[arg(long)]
        no_questions: bool,
    },
    /// List the questions in a working copy's questions.toml, and answer
    /// them: an answer to a number's question is written into the model as
    /// a stated fact.
    Questions {
        dir: PathBuf,
        /// ID=N: choose candidate N (from 1) of question ID. Repeatable.
        #[arg(long = "answer", value_name = "ID=N")]
        answers: Vec<String>,
    },
    /// Fail if the model or contract isn't in canonical form.
    /// Rewrite model.toml and fidelity.toml in their canonical form, the
    /// one text each model has. With `--check`, write nothing and exit 1 if
    /// either isn't in it.
    Fmt {
        dir: PathBuf,
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum BaselineCmd {
    /// Record each baseline into a temporary store and check its frame
    /// hashes, lag frames and RAM assertions. A baseline whose ROM isn't in
    /// $GP_ROM_DIR is skipped; our own test ROM is built in, so its
    /// baselines always run. Exits 1 if any check fails.
    Check {
        /// Baseline files, or directories of them. By default, every .toml
        /// under the nearest baselines/ directory.
        files: Vec<PathBuf>,
        /// Also run each baseline with video rendering on, and fail if its
        /// frame hashes change.
        #[arg(long)]
        video: bool,
        /// The ROM file for every baseline checked, instead of finding each
        /// in $GP_ROM_DIR by hash; for a game built from source, such as the
        /// reference game (`just mosshop`). A baseline that expects another
        /// SHA-256 can't be checked.
        #[arg(long)]
        rom: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum InterpCmd {
    /// Run a model through an input log, through its game's module or the
    /// built-in interpreter, and print its frame count, the digest of its
    /// per-frame state hashes, and its state at the last frame by model path.
    Run(interp::RunArgs),
    /// Check that each level's goal can be reached by the player's own
    /// moves (gp_interp::reach, the player alone) and measure the jump's
    /// envelope, through the game's module. With `--record`, also search each
    /// level with its enemies and rules for a run to the goal and write it as
    /// a model baseline, `MODEL/baselines/<level>.toml`. Exits 1 when a goal
    /// can't be reached or no run is found.
    Reach {
        /// A model directory (holding model.toml) or a model.toml file.
        model: PathBuf,
        /// The game's built module. Needed for a model with hooks.
        #[arg(long)]
        module: Option<PathBuf>,
        /// The model's lifted assets (default:
        /// $GP_HOME/work/assets/<model.source>).
        #[arg(long, value_name = "DIR")]
        assets: Option<PathBuf>,
        /// Only this level (repeat for more; default: every level).
        #[arg(long)]
        level: Vec<String>,
        /// Record a run through each level as a model baseline.
        #[arg(long)]
        record: bool,
    },
    /// Run the original and the model on the same input, and compare them
    /// frame by frame through the model's state mapping. Each mapped model
    /// path gets a line with its agreement and its first difference, and
    /// the lockstep's report follows. Exits 1 if any path disagrees.
    Compare(interp::CompareArgs),
    /// The original's state as a model state, through the state mapping
    /// read in reverse: at a frame of an input, or in a snapshot file of
    /// the original. Prints a note for every part (loaded, approximated or
    /// missing when the mapping doesn't hold it). With `--out` it writes the
    /// canonical model state, which any runtime of the model loads with the
    /// runtime protocol's load-state.
    Translate(translate::TranslateArgs),
    /// The game's own moments in a run of the original (jumps, landings,
    /// deaths, lives lost, counter changes, level changes), read from the
    /// state mapping's measures, and what the model does at each on the same
    /// presses from the original's state half a second before it. An
    /// example line reads "jump 1 at frames 218-250 (3.6 s) rises 66 px; the
    /// model too, on the same frame, 52 px".
    Moments(moments::MomentsArgs),
    /// Measure the model's enemies (their components and `ext` hooks)
    /// against the original's enemy slots, slot life by slot life.
    Slots(slots::SlotsArgs),
    /// The random-choice behaviour check: the model's branches at each
    /// `[[draws]]` site tested against the rule's own odds.
    Odds(odds::OddsArgs),
    /// Run the scenario ladder: short generated presses, on the original
    /// and on the model from the original's state, measured apart.
    Scenarios(scenarios_cmd::ScenariosArgs),
    /// Record the original's random draws on an input, at the draw sites the
    /// state mapping's `[[draws]]` names: each read of a site's random byte by
    /// its instruction, with the frame, in order. The script lets the
    /// lockstep make the rebuild choose as the original did
    /// (docs/formats/draws.md). With `--find ROM` and no model, find the game's
    /// random register and its draw sites from the run instead and propose
    /// `[[draws]]` entries for review, as that page's section "Finding draw
    /// sites" says; a model or `--mapping` given too scores them against its
    /// hand-written sites.
    Draws(interp_draws::DrawsArgs),
}

#[derive(Subcommand)]
enum GameCmd {
    /// Start a new game from a ROM you own that no game names yet: its
    /// package folder in the games checkout, with a model skeleton for the
    /// genre, the ROM's hashes in roms.toml, pins that hold, and no clips.
    /// Writes over nothing
    New {
        /// The game's id: lower-case letters, digits and dashes (qbert-usa)
        game: String,
        /// The ROM, already in $GP_ROM_DIR (`gp roms add` puts it there)
        #[arg(long)]
        rom: PathBuf,
        /// The genre: platformer, topdown, gridhop, events or other
        #[arg(long)]
        genre: String,
        /// The game's title (default: GAME)
        #[arg(long)]
        title: Option<String>,
        /// The package's namespace, NS in NS/GAME (default: the one the other
        /// games in the checkout share)
        #[arg(long)]
        namespace: Option<String>,
        /// The games checkout (default: $GP_GAMES_DIR, else a `games` folder
        /// at or beside the current folder or one above it)
        #[arg(long)]
        games: Option<PathBuf>,
        /// Leave out the search for draw sites on an idle run
        #[arg(long)]
        no_draws: bool,
        /// The source ID to register the ROM under in roms.toml (default:
        /// GAME)
        #[arg(long)]
        source: Option<String>,
    },
    /// Fetch NAME@VERSION by id into a folder, checking every signature (with --root) and
    /// hash, and lift every ROM entry it finds in the ROM folder.
    Fetch {
        name: String,
        #[arg(long)]
        id: String,
        /// The registry: a folder, s3://BUCKET or an https address.
        #[arg(long)]
        from: String,
        /// A file holding the registry's root public key, to check signatures.
        #[arg(long)]
        root: Option<PathBuf>,
        /// The working copy to write, outside any git work tree.
        #[arg(long)]
        into: PathBuf,
        #[arg(long)]
        rom_dir: PathBuf,
    },
    /// Write the module kit: the tool's sources for a game module and the
    /// crates they need, ready for a build with no network.
    Kit {
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Print a built module's BLAKE3 hash.
    Hash { file: PathBuf },
    /// Score a game from its package and the ROM alone, in this $GP_HOME:
    /// every clip and curve derived from the corpus plan and the ROM, played
    /// through the module, and the score record written to FILE. Exits 1
    /// when the corpus has problems.
    Score {
        dir: PathBuf,
        /// The game's built module (default: the package's play/module.wasm).
        #[arg(long)]
        module: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Attest a published version of a game: derivation, module build and
    /// score, as signed statements under attest/<id>/ in the registry. Takes
    /// a clean `gp game score` output of the package.
    Attest {
        /// The package's folder (a working copy of the published version).
        dir: PathBuf,
        /// The `gp game score` output.
        #[arg(long)]
        score: PathBuf,
        /// The module kit the module was built from (`gp game kit -o DIR`).
        #[arg(long)]
        kit: PathBuf,
        /// The module (default: the package's play/module.wasm).
        #[arg(long)]
        module: Option<PathBuf>,
        /// The published package's id.
        #[arg(long)]
        id: String,
        /// The registry: a folder, git:PATH or s3://BUCKET.
        #[arg(long)]
        to: String,
        /// The namespace's signing key file.
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Build a game's module from its package folder and the kit, in a
    /// sandbox that reads nothing else and has no network. Exits 1 when the
    /// hook crate breaks the rules, 2 when the build fails.
    Build {
        dir: PathBuf,
        #[arg(long)]
        kit: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        /// Write the package's Cargo.lock from the kit's, first.
        #[arg(long)]
        update_lock: bool,
    },
    /// Run a game's hook crate tests as `gp game build` builds it: in a
    /// copy of the kit's tree under $GP_HOME/work/module-build, in the
    /// sandbox, offline, with the package's lock, for this machine. Nothing
    /// is written in the package folder (no ext/Cargo.lock, no target).
    /// Exits 1 when a test fails, with cargo's last lines.
    Test {
        dir: PathBuf,
        #[arg(long)]
        kit: PathBuf,
    },
}

#[derive(Subcommand)]
enum RegistryCmd {
    /// Serve a folder registry and a built page on this machine only.
    Serve {
        dir: std::path::PathBuf,
        /// The built page, served beside the registry.
        #[arg(long)]
        page: std::path::PathBuf,
        #[arg(long, default_value_t = 8377)]
        port: u16,
    },
    /// Make a registry in a folder or s3://BUCKET.
    Init {
        target: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        root_key: Option<PathBuf>,
        #[arg(long)]
        signing_key: Option<PathBuf>,
        /// NS=KEY_FILE, repeated.
        #[arg(long)]
        namespace: Vec<String>,
    },
    /// Publish a folder of files as one exact version.
    Publish {
        target: String,
        dir: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: String,
        #[arg(long, default_value = "content")]
        kind: String,
        #[arg(long)]
        title: String,
        #[arg(long = "for")]
        for_games: Vec<String>,
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Rewrite the list files from these name@version entries, signed if
    /// --key is given.
    Reindex {
        target: String,
        versions: Vec<String>,
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Test a registry from outside, against its root if --root is given.
    Check {
        url: String,
        /// A file holding the root's public key.
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum PackageCmd {
    /// Print the hash of a package folder's hook code.
    Hooks { dir: std::path::PathBuf },
    /// Sort every file and every field of a package folder into four
    /// buckets: ours, reference, from the ROM and needs rewrite. Compares
    /// with each ROM of its `[[rom.entry]]` found by hash in $GP_ROM_DIR
    /// and prints no ROM byte. With no ROM it says that nothing was
    /// compared. Exits 1 on any needs-rewrite item or a failed rebuild, or
    /// with --baseline only on a needs-rewrite item the baseline doesn't
    /// list.
    Scan {
        dir: PathBuf,
        /// A ROM file to compare with, instead of finding it by hash.
        #[arg(long)]
        rom: Vec<PathBuf>,
        /// A built module to check too: each data segment and custom
        /// section against the ROMs. Any .wasm in the folder is always read.
        #[arg(long)]
        wasm: Vec<PathBuf>,
        /// An earlier scan's --json output: its needs-rewrite items are
        /// listed as known, and only new ones fail.
        #[arg(long)]
        baseline: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum TemplateCmd {
    /// Build an original template's assets (source.how = "authored") from
    /// its authored.toml: a data image (to $GP_HOME/work/images/), its asset
    /// map, and the lifted directory. Exits 1 when the asset map or
    /// template.toml's pins are stale, unless --write updates them.
    Build {
        /// An original template directory.
        dir: PathBuf,
        /// The lifted directory to write, outside any git work tree
        /// (default $GP_HOME/work/assets/<source>).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Rewrite the asset map and template.toml's pins when they're stale.
        #[arg(long)]
        write: bool,
    },
    /// Check a ROM against the template: accepted hash, map pin, lift pin.
    Check {
        dir: PathBuf,
        /// The source ROM (default: the first file in $GP_ROM_DIR whose hash
        /// the template accepts).
        #[arg(long)]
        rom: Option<PathBuf>,
        /// For a derived template with authored stages: the layout check.
        /// Each level's cells, ladders and things against the base's, lifted
        /// in memory from your ROM of the base, and refused when too close;
        /// and any data line of the template's built files that the base's
        /// lifted files hold.
        #[arg(long)]
        original: bool,
    },
    /// Check that a package's open version holds none of the original's
    /// data: no line of 24 characters or more of its built files may be held
    /// by the original's lifted files, and none of its bytes or lifted files
    /// may be the ROM's. With --folder, the same for every file of a folder
    /// to be served. Exits 1 on any find, and 2 when a ROM isn't found.
    Served {
        /// The package folder (games/donkey-kong).
        dir: PathBuf,
        /// A folder to be served; repeat for more.
        #[arg(long)]
        folder: Vec<PathBuf>,
        /// A ROM file to compare with, instead of finding it by hash.
        #[arg(long)]
        rom: Vec<PathBuf>,
    },
}

fn dispatch(cmd: &Cmd) -> Result<Outcome> {
    match cmd {
        Cmd::Agent(a) => agent_cmd::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Assets { cmd } => match cmd {
            AssetsCmd::Lift {
                rom,
                out,
                map,
                force,
            } => assets::lift(
                rom,
                out.as_deref(),
                map.as_deref(),
                *force,
                &gp_evidence::Paths::from_env()?.home,
            ),
            AssetsCmd::Pack {
                dir,
                check: Some(rom),
                metrics,
                ..
            } => assets::check_dir(dir, rom, metrics.as_deref()),
            AssetsCmd::Pack {
                dir,
                base: Some(base),
                out: Some(out),
                force,
                ..
            } => assets::pack_into(dir, base, out, *force),
            AssetsCmd::Pack { .. } => {
                anyhow::bail!("give --check ROM, or --base ROM with --out FILE")
            }
        },
        Cmd::Worker { cmd } => worker::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Tool {
            cmd: ToolCmd::Gate(a),
        } => tool_gate::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Itself { cmd } => publish::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Clip { cmd } => clip_score::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Rule { cmd } => rule_refine::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Corpus { cmd } => corpus_score::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Model { cmd } => match cmd {
            ModelCmd::Check { dir, assets, root } => model::check_in(
                dir,
                assets.as_deref(),
                root.as_deref(),
                &gp_evidence::Paths::from_env()?.home,
            ),
            ModelCmd::Fmt { dir, check } => model::fmt(dir, *check),
            ModelCmd::Gate(a) => gate::run(a, &gp_evidence::Paths::from_env()?),
            ModelCmd::Mechanics(a) => mechanics::run(a, &gp_evidence::Paths::from_env()?),
            ModelCmd::Static(a) => model_static::run(a, &gp_evidence::Paths::from_env()?),
            ModelCmd::Refine(a) => refine::run(a, &gp_evidence::Paths::from_env()?),
            ModelCmd::Blind(a) => model_blind::run(a, &gp_evidence::Paths::from_env()?),
            ModelCmd::Derive {
                dir,
                from,
                assets,
                recordings,
                frames,
                root,
                no_questions,
                list,
            } => derive::run(
                &derive::DeriveArgs {
                    dir,
                    from,
                    assets: assets.as_deref(),
                    recordings,
                    frames,
                    root: root.as_deref(),
                    questions: !no_questions,
                    list: *list,
                },
                &gp_evidence::Paths::from_env()?,
            ),
            ModelCmd::Propose {
                dir,
                recordings,
                frames,
                no_questions,
            } => refine::recording::propose(
                dir,
                recordings,
                frames,
                !no_questions,
                &gp_evidence::Paths::from_env()?,
            ),
            ModelCmd::Questions { dir, answers } => refine::questions(dir, answers),
        },
        Cmd::Baseline {
            cmd: BaselineCmd::Check { files, video, rom },
        } => run::baseline_check(
            files,
            *video,
            rom.as_deref(),
            &gp_evidence::Paths::from_env()?.roms,
        ),
        Cmd::Interp {
            cmd: InterpCmd::Run(a),
        } => interp::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Interp {
            cmd: InterpCmd::Compare(a),
        } => interp::compare(a, &gp_evidence::Paths::from_env()?),
        Cmd::Interp {
            cmd:
                InterpCmd::Reach {
                    model,
                    module,
                    assets,
                    level,
                    record,
                },
        } => search::reach_cmd(
            model,
            assets.as_deref(),
            level,
            *record,
            module.as_deref(),
            &gp_evidence::Paths::from_env()?,
        ),
        Cmd::Interp {
            cmd: InterpCmd::Translate(a),
        } => translate::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Interp {
            cmd: InterpCmd::Moments(a),
        } => moments::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Interp {
            cmd: InterpCmd::Slots(a),
        } => slots::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Interp {
            cmd: InterpCmd::Odds(a),
        } => odds::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Interp {
            cmd: InterpCmd::Scenarios(a),
        } => scenarios_cmd::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Picture(a) => picture::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Solve(a) => solve::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Level { cmd } => search::level_run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Track(a) => track::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Validate(a) => validate::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Coverage(a) => coverage::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Gaps { cmd } => gaps::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Sound { cmd } => sound::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Sprites { cmd } => sprites::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Dataset { cmd } => dataset::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Game {
            cmd:
                GameCmd::Fetch {
                    name,
                    id,
                    from,
                    root,
                    into,
                    rom_dir,
                },
        } => game::fetch(name, id, from, root.as_deref(), into, rom_dir),
        Cmd::Game {
            cmd:
                GameCmd::New {
                    game,
                    rom,
                    genre,
                    title,
                    namespace,
                    games,
                    no_draws,
                    source,
                },
        } => game_new::run(
            &game_new::New {
                game,
                rom,
                genre,
                title: title.as_deref(),
                namespace: namespace.as_deref(),
                games: games.as_deref(),
                source: source.as_deref(),
                date: &game_new::today(),
                draws: !no_draws,
            },
            &gp_evidence::Paths::from_env()?,
        ),
        Cmd::Game {
            cmd: GameCmd::Kit { out },
        } => module::kit(out),
        Cmd::Game {
            cmd: GameCmd::Hash { file },
        } => module::hash(file),
        Cmd::Game {
            cmd: GameCmd::Score { dir, module, out },
        } => corpus_score::game_score(
            &gp_evidence::Paths::from_env()?,
            dir,
            module.as_deref(),
            out,
        ),
        Cmd::Game {
            cmd:
                GameCmd::Attest {
                    dir,
                    score,
                    kit,
                    module,
                    id,
                    to,
                    key,
                },
        } => attest::run(
            &gp_evidence::Paths::from_env()?,
            &attest::Given {
                dir,
                score,
                kit,
                module: module.as_deref(),
                id,
                to,
                key: key.as_deref(),
            },
        ),
        Cmd::Game {
            cmd:
                GameCmd::Build {
                    dir,
                    kit,
                    out,
                    update_lock,
                },
        } => module::build(dir, kit, out, *update_lock),
        Cmd::Game {
            cmd: GameCmd::Test { dir, kit },
        } => module::test_hooks(dir, kit),
        Cmd::Roms { cmd } => roms_cmd::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Labels { cmd } => labels::run(cmd),
        Cmd::Sealed { cmd } => sealed::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Mechanic { cmd } => mechanic::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Serve { opts } => serve::run(opts, &gp_evidence::Paths::from_env()?),
        Cmd::Package { cmd } => match cmd {
            PackageCmd::Hooks { dir } => package::hooks(dir),
            PackageCmd::Scan {
                dir,
                rom,
                wasm,
                baseline: None,
            } => package::scan::run(dir, rom, wasm, &gp_evidence::Paths::from_env()?),
            PackageCmd::Scan {
                dir,
                rom,
                wasm,
                baseline: Some(b),
            } => package::scan::run_against(dir, rom, wasm, b, &gp_evidence::Paths::from_env()?),
        },
        Cmd::Registry { cmd } => match cmd {
            RegistryCmd::Serve { dir, page, port } => registry::serve(dir, page, *port),
            RegistryCmd::Init {
                target,
                name,
                root_key,
                signing_key,
                namespace,
            } => registry::init_cmd(
                target,
                name,
                root_key.as_deref(),
                signing_key.as_deref(),
                namespace,
            ),
            RegistryCmd::Publish {
                target,
                dir,
                name,
                version,
                kind,
                title,
                for_games,
                key,
            } => registry::publish_cmd(
                target,
                dir,
                (name, version),
                (kind, title),
                for_games,
                key.as_deref(),
            ),
            RegistryCmd::Reindex {
                target,
                versions,
                key,
            } => registry::reindex_cmd(target, versions, key.as_deref()),
            RegistryCmd::Check { url, root } => registry::check_cmd(url, root.as_deref()),
        },
        Cmd::Explore(a) => explore::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Watch { cmd } => watch::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Capture { cmd } => capture::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Record(a) => recordings::record_cmd(a, &gp_evidence::Paths::from_env()?),
        Cmd::Replay(a) => recordings::replay_cmd(a, &gp_evidence::Paths::from_env()?),
        Cmd::Query(a) => recordings::query_cmd(a, &gp_evidence::Paths::from_env()?),
        Cmd::RunUntil(a) => recordings::run_until_cmd(a, &gp_evidence::Paths::from_env()?),
        Cmd::Trace { cmd } => recordings::trace_cmd(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Events(a) => recordings::events_cmd(a, &gp_evidence::Paths::from_env()?),
        Cmd::Recordings { cmd } => {
            recordings::recordings_cmd(cmd, &gp_evidence::Paths::from_env()?)
        }
        Cmd::Movie { cmd } => recordings::movie_cmd(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Code { cmd } => code::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Reach(a) => reach::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Cycles { cmd } => cycles::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Element { cmd } => element::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Interp {
            cmd: InterpCmd::Draws(a),
        } => interp_draws::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Author(a) => author::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Farm(a) => farm::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::FarmTopdown(a) => farm::topdown::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Host { cmd } => host::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Mcp(a) => mcp::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Store { cmd } => store::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Template {
            cmd: TemplateCmd::Build { dir, out, write },
        } => template_build::build_cmd(
            dir,
            out.as_deref(),
            *write,
            &gp_evidence::Paths::from_env()?,
        ),
        Cmd::Template {
            cmd: TemplateCmd::Check { dir, rom, original },
        } => template_original::check_cmd(
            dir,
            rom.as_deref(),
            *original,
            &gp_evidence::Paths::from_env()?,
        ),
        Cmd::Template {
            cmd: TemplateCmd::Served { dir, folder, rom },
        } => served::served(dir, folder, rom, &gp_evidence::Paths::from_env()?),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Every reader of a recording or an input log asks the sealed-run guard.
    if let Ok(p) = gp_evidence::Paths::from_env() {
        sealed::install(&p.home);
    }
    if let Some(n) = cli.jobs {
        pool::set_jobs(n);
    }
    host::JSON_OUT.store(cli.json, std::sync::atomic::Ordering::Relaxed);
    if cli.json && matches!(cli.cmd, Cmd::Mcp(_)) {
        // gp prints its JSON on stdout when a command ends, which for `gp
        // mcp` is the MCP channel.
        eprintln!("gp: gp mcp speaks JSON-RPC on stdout, so it takes no --json");
        return ExitCode::from(2);
    }
    match dispatch(&cli.cmd) {
        Ok(o) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&o.json).unwrap_or_default()
                );
            } else {
                println!("{}", o.text);
            }
            ExitCode::from(o.exit)
        }
        Err(e) => {
            eprintln!("gp: {e:#}");
            ExitCode::from(2)
        }
    }
}
