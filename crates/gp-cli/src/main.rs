//! `gp`: a thin client over the library crates. It holds only the commands
//! the new world uses so far; each is one call into a library, with no
//! engine here.

mod affected;
mod assets;
mod attest;
mod cli_doc;
mod clip;
mod clip_score;
mod clipset;
mod corpus_score;
mod game;
mod game_new;
mod gate;
mod gate_copy;
mod hosted;
mod lockstep;
mod mechanic;
mod model;
mod module;
mod package;
mod plan;
mod pool;
mod publish;
mod registry;
mod romclean;
mod roms;
mod roms_cmd;
mod run;
mod scenario;
mod screen;
mod search;
mod served;
mod tape;
mod template;
mod tool_gate;
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
    /// The ROMs you own, kept in $GP_ROM_DIR: add, list and clean them.
    Roms {
        #[command(subcommand)]
        cmd: roms_cmd::RomsCmd,
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
    /// Decode a ROM's assets into a directory outside any git work tree.
    Lift {
        rom: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long)]
        map: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Pack a lifted directory and compare it with the ROM, byte for byte.
    Pack {
        dir: PathBuf,
        #[arg(long)]
        check: PathBuf,
    },
}

#[derive(Subcommand)]
enum ModelCmd {
    /// Check a model, its profiles, its contract and its asset slots.
    Check {
        dir: PathBuf,
        #[arg(long)]
        assets: Option<PathBuf>,
    },
    /// Hold a patch to a game in the games repo to its corpus: PASS or FAIL,
    /// and what passes lands.
    Gate(gate::GateArgs),
    /// Fail if the model or contract isn't in canonical form.
    Fmt {
        dir: PathBuf,
        #[arg(long, required = true)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum BaselineCmd {
    /// Run baselines and compare them with their expected results.
    Check {
        files: Vec<PathBuf>,
        #[arg(long)]
        rom: PathBuf,
    },
}

#[derive(Subcommand)]
enum InterpCmd {
    /// Run a model through its game's module on an input log and write the
    /// state hash at power-on and after every frame, one in hex a line.
    Run {
        dir: PathBuf,
        /// The game's built module.
        #[arg(long)]
        module: PathBuf,
        #[arg(long)]
        assets: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        hashes: PathBuf,
    },
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
    /// Compare the model with the original on a baseline's input.
    Compare {
        dir: PathBuf,
        #[arg(long)]
        rom: PathBuf,
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        assets: PathBuf,
    },
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
        /// The genre: platformer, topdown or other
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
    /// compared. Exits 1 on any needs-rewrite item or a failed rebuild.
    Scan {
        dir: PathBuf,
        /// A ROM file to compare with, instead of finding it by hash.
        #[arg(long)]
        rom: Vec<PathBuf>,
        /// A built module to check too: each data segment and custom
        /// section against the ROMs. Any .wasm in the folder is always read.
        #[arg(long)]
        wasm: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum TemplateCmd {
    /// Build an authored template's lifted files and hold them to its pin.
    Build {
        dir: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Check a ROM against the template: accepted hash, map pin, lift pin.
    Check {
        dir: PathBuf,
        #[arg(long)]
        rom: PathBuf,
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
        Cmd::Assets { cmd } => match cmd {
            AssetsCmd::Lift {
                rom,
                out,
                map,
                force,
            } => assets::lift(rom, out, map, *force),
            AssetsCmd::Pack { dir, check } => assets::check_dir(dir, check),
        },
        Cmd::Worker { cmd } => worker::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Tool {
            cmd: ToolCmd::Gate(a),
        } => tool_gate::run(a, &gp_evidence::Paths::from_env()?),
        Cmd::Itself { cmd } => publish::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Clip { cmd } => clip_score::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Corpus { cmd } => corpus_score::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Model { cmd } => match cmd {
            ModelCmd::Check { dir, assets } => model::check(dir, assets.as_deref()),
            ModelCmd::Fmt { dir, .. } => model::fmt_check(dir),
            ModelCmd::Gate(a) => gate::run(a, &gp_evidence::Paths::from_env()?),
        },
        Cmd::Baseline {
            cmd: BaselineCmd::Check { files, rom },
        } => run::baseline_check(files, rom),
        Cmd::Interp {
            cmd:
                InterpCmd::Run {
                    dir,
                    module,
                    assets,
                    input,
                    hashes,
                },
        } => run::module_run(dir, module, assets, input, hashes),
        Cmd::Interp {
            cmd:
                InterpCmd::Compare {
                    dir,
                    rom,
                    baseline,
                    assets,
                },
        } => run::compare(dir, rom, baseline, assets),
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
        Cmd::Level { cmd } => search::level_run(cmd, &gp_evidence::Paths::from_env()?),
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
                },
        } => game_new::run(
            &game_new::New {
                game,
                rom,
                genre,
                title: title.as_deref(),
                namespace: namespace.as_deref(),
                games: games.as_deref(),
                date: &game_new::today(),
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
        Cmd::Roms { cmd } => roms_cmd::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Mechanic { cmd } => mechanic::run(cmd, &gp_evidence::Paths::from_env()?),
        Cmd::Package { cmd } => match cmd {
            PackageCmd::Hooks { dir } => package::hooks(dir),
            PackageCmd::Scan { dir, rom, wasm } => {
                package::scan::run(dir, rom, wasm, &gp_evidence::Paths::from_env()?)
            }
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
        Cmd::Template {
            cmd: TemplateCmd::Build { dir, out },
        } => template::build(dir, out),
        Cmd::Template {
            cmd: TemplateCmd::Check { dir, rom },
        } => template::check(dir, rom),
        Cmd::Template {
            cmd: TemplateCmd::Served { dir, folder, rom },
        } => served::served(dir, folder, rom, &gp_evidence::Paths::from_env()?),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(n) = cli.jobs {
        pool::set_jobs(n);
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
