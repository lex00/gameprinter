//! `gp`: a thin client over the library crates. It holds only the commands
//! the new world uses so far; each is one call into a library, with no
//! engine here.

mod assets;
mod game;
mod model;
mod module;
mod package;
mod registry;
mod roms;
mod run;
mod served;
mod template;

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

#[derive(Parser)]
#[command(name = "gp", version, about = "Game Printer")]
struct Cli {
    /// Print machine-readable JSON on stdout.
    #[arg(long, global = true)]
    json: bool,
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
    /// Fetch a game by its exact id and lift it from a person's ROMs.
    Game {
        #[command(subcommand)]
        cmd: GameCmd,
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
    /// Fetch NAME@VERSION by id into a folder, checking every signature and
    /// hash, and lift every ROM entry it finds in the ROM folder.
    Fetch {
        name: String,
        #[arg(long)]
        id: String,
        /// The registry: a folder, s3://BUCKET or an https address.
        #[arg(long)]
        from: String,
        /// A file holding the registry's root public key.
        #[arg(long)]
        root: PathBuf,
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
        root_key: PathBuf,
        #[arg(long)]
        signing_key: PathBuf,
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
        key: PathBuf,
    },
    /// Re-sign the list files from these name@version entries.
    Reindex {
        target: String,
        versions: Vec<String>,
        #[arg(long)]
        key: PathBuf,
    },
    /// Test a registry from outside against its pinned root.
    Check {
        url: String,
        /// A file holding the root's public key.
        #[arg(long)]
        root: PathBuf,
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
        Cmd::Model { cmd } => match cmd {
            ModelCmd::Check { dir, assets } => model::check(dir, assets.as_deref()),
            ModelCmd::Fmt { dir, .. } => model::fmt_check(dir),
        },
        Cmd::Baseline {
            cmd: BaselineCmd::Check { files, rom },
        } => run::baseline_check(files, rom),
        Cmd::Interp {
            cmd:
                InterpCmd::Compare {
                    dir,
                    rom,
                    baseline,
                    assets,
                },
        } => run::compare(dir, rom, baseline, assets),
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
        } => game::fetch(name, id, from, root, into, rom_dir),
        Cmd::Game {
            cmd: GameCmd::Kit { out },
        } => module::kit(out),
        Cmd::Game {
            cmd: GameCmd::Hash { file },
        } => module::hash(file),
        Cmd::Game {
            cmd:
                GameCmd::Build {
                    dir,
                    kit,
                    out,
                    update_lock,
                },
        } => module::build(dir, kit, out, *update_lock),
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
            } => registry::init_cmd(target, name, root_key, signing_key, namespace),
            RegistryCmd::Publish {
                target,
                dir,
                name,
                version,
                kind,
                title,
                for_games,
                key,
            } => registry::publish_cmd(target, dir, (name, version), (kind, title), for_games, key),
            RegistryCmd::Reindex {
                target,
                versions,
                key,
            } => registry::reindex_cmd(target, versions, key),
            RegistryCmd::Check { url, root } => registry::check_cmd(url, root),
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
