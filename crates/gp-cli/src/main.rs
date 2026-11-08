//! `gp`: a thin client over the library crates. It holds only the commands
//! the new world uses so far; each is one call into a library, with no
//! engine here.

mod assets;
mod model;
mod registry;
mod run;
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
    /// Make, publish to and test registries.
    Registry {
        #[command(subcommand)]
        cmd: RegistryCmd,
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
enum RegistryCmd {
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
enum TemplateCmd {
    /// Check a ROM against the template: accepted hash, map pin, lift pin.
    Check {
        dir: PathBuf,
        #[arg(long)]
        rom: PathBuf,
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
        Cmd::Registry { cmd } => match cmd {
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
            cmd: TemplateCmd::Check { dir, rom },
        } => template::check(dir, rom),
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
