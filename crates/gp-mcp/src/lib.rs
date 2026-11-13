//! The agent tools as an MCP server (Milestone 3, docs/mcp.md). An agent
//! copies a template ([`fork`]), reads the copy's surface, edits the model and contract by model path, previews a run in
//! the interpreter and validates against the original's recordings.
//!
//! - [`tools`]: each tool's arguments, its JSON Schema and what it does.
//!   They're plain functions from JSON to JSON, so they test without MCP.
//! - [`edit`]: path-addressed edits to a model tree and contract
//!   operations.
//! - [`server`]: the MCP server over stdio, with the `rmcp` SDK.
//!
//! Edits run in-process on `gp-model` and `gp-contract`; previews on
//! `gp-interp`. Checking and validation run `gp --json model check` and
//! `gp --json validate` as child processes, so the server gives the same
//! answers and exit codes as the CLI and records the same runs.

pub mod asset_edit;
pub mod capture;
pub mod catalog;
pub mod contexts;
pub mod critic;
pub mod edit;
pub mod edit_log;
pub mod editor_trip;
pub mod exchange;
pub mod fork;
pub mod generate;
pub mod moments;
pub mod place;
pub mod progressive;
pub mod publish;
pub mod ready;
pub mod refine;
pub mod registry;
pub mod requests;
pub mod resize;
pub mod scenario;
pub mod scene;
pub mod search;
pub mod server;
pub mod sound;
pub mod sprite_ops;
pub mod sprite_set;

pub mod show;
pub mod testgame;
pub mod tile_edit;
pub mod tools;
pub mod watch;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

use serde_json::{Value, json};

/// Where the server finds things and keeps its working copies.
#[derive(Debug, Clone)]
pub struct Config {
    /// The `gp` binary checks and validation run.
    pub gp: PathBuf,
    /// The repo: `templates/`, `baselines/`, and the root evidence refs are
    /// read against.
    pub repo: PathBuf,
    /// Working copies, one directory each, and the validation store.
    pub work: PathBuf,
    /// `$GP_HOME`, for the default lifted assets.
    pub home: PathBuf,
    /// The original's ROM for validation, when it isn't in `$GP_ROM_DIR`.
    pub rom: Option<PathBuf>,
    /// Lifted assets, instead of `$GP_HOME/work/assets/<model.source>`.
    pub assets: Option<PathBuf>,
    /// The games checkout, whose game folders are templates too (`gp`
    /// finds it as its other commands do: $GP_GAMES_DIR, else a `games`
    /// folder at, above or beside the current folder). None without one.
    pub games: Option<PathBuf>,
}

impl Config {
    /// The directory of a working copy. Its name must be a model path name,
    /// so it can't reach outside `work`.
    pub fn copy_dir(&self, name: &str) -> Result<PathBuf, String> {
        if !gp_model::path::is_name(name) {
            return Err(format!(
                "`{name}` isn't a copy name: letters, digits and `_`, not starting with a digit"
            ));
        }
        Ok(self.work.join(name))
    }

    /// An existing working copy.
    pub fn existing_copy(&self, name: &str) -> Result<PathBuf, String> {
        let dir = self.copy_dir(name)?;
        if !dir.join(gp_model::MODEL_FILE).is_file() {
            return Err(format!(
                "there's no copy `{name}`; make one with template_fork"
            ));
        }
        Ok(dir)
    }

    /// The store validation keeps its results in.
    pub fn store(&self) -> PathBuf {
        self.work.join(".store")
    }

    /// The assets a working copy runs on: its own (`<copy>/assets/`, made
    /// by the first `assets_place`), or else the lifted assets for its
    /// source.
    pub fn assets_of(&self, dir: &Path, source: Option<&str>) -> Option<PathBuf> {
        let own = dir.join(place::COPY_ASSETS);
        if own.join("assets.toml").is_file() {
            return Some(own);
        }
        self.assets_for(source)
    }

    /// The lifted assets for a model's source.
    pub fn assets_for(&self, source: Option<&str>) -> Option<PathBuf> {
        let source = source.filter(|s| is_source_name(s));
        match (&self.assets, source) {
            (Some(a), _) => Some(a.clone()),
            (None, Some(s)) => Some(self.home.join("work").join("assets").join(s)),
            (None, None) => None,
        }
    }
}

/// The working copies a call is using now. The server runs each request as
/// its own task, so two calls on one copy would otherwise interleave their
/// reads and writes, and one's rollback could undo the other's write.
static BUSY: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
static FREED: Condvar = Condvar::new();

/// Holds a working copy for one call; the next call on it waits until this
/// is dropped. Calls on other copies don't wait.
pub struct CopyLock(String);

impl CopyLock {
    pub fn take(copy: &str) -> Self {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        while busy.contains(copy) {
            busy = FREED.wait(busy).unwrap_or_else(|e| e.into_inner());
        }
        busy.insert(copy.to_owned());
        CopyLock(copy.to_owned())
    }
}

impl Drop for CopyLock {
    fn drop(&mut self) {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        busy.remove(&self.0);
        FREED.notify_all();
    }
}

/// True for a source ID that's safe to join into a path: a path name, with
/// `-` allowed (`smb1-ntsc-prg0`).
/// Where an original template keeps its model baselines: its own
/// `baselines/` directory (gp_interp::baseline), for a source with no ROM
/// baselines under the repo's `baselines/`.
pub fn model_baselines_dir(config: &Config, source: &str) -> PathBuf {
    config
        .repo
        .join("templates")
        .join(source)
        .join(gp_interp::baseline::DIR)
}

pub fn is_source_name(s: &str) -> bool {
    gp_model::path::is_name(&s.replace('-', "_"))
}

/// What a tool returns: structured content, and whether it's an error the
/// agent should see and correct (an edit refused, a run that couldn't
/// start). A check that ran and failed isn't an error.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub json: Value,
    pub is_error: bool,
}

impl Reply {
    pub fn ok(json: Value) -> Self {
        Self {
            json,
            is_error: false,
        }
    }

    /// The tool refused or couldn't finish; `json` says why.
    pub fn refused(json: Value) -> Self {
        Self {
            json,
            is_error: true,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::refused(json!({ "error": message.into() }))
    }
}

/// Runs `gp --json ARGS` with the server's store, and returns its exit code
/// and JSON. Exit 2, or output that isn't JSON, is an error carrying gp's
/// message. Its stdin is closed: the server's stdin is the MCP channel.
pub fn run_gp(config: &Config, args: &[&std::ffi::OsStr]) -> Result<(i32, Value), String> {
    let out = std::process::Command::new(&config.gp)
        .arg("--json")
        .args(args)
        .env("GP_STORE", config.store())
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("running {}: {e}", config.gp.display()))?;
    let code = out.status.code().unwrap_or(2);
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if code == 2 {
        return Err(if stderr.is_empty() {
            "gp exited 2 (couldn't run)".to_owned()
        } else {
            stderr
        });
    }
    let json = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("gp printed no JSON ({e}): {stderr}"))?;
    Ok((code, json))
}

/// Runs `gp model check` on a model directory, with evidence refs read
/// against the repo and slots against the lifted assets when they're there.
/// Returns whether it passed and its report.
pub fn model_check(
    config: &Config,
    dir: &Path,
    source: Option<&str>,
) -> Result<(bool, Value), String> {
    // A copy's evidence refs are the template's: relative to the repo for
    // a template under its templates/ (`--root REPO`, as game-printer
    // passed), and to the game's own folder, which the copy holds whole,
    // for a game folder (one with package.toml), as gp template extract
    // reads them.
    let root = if dir.join("package.toml").is_file() {
        dir
    } else {
        config.repo.as_path()
    };
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "model".as_ref(),
        "check".as_ref(),
        dir.as_os_str(),
        "--root".as_ref(),
        root.as_os_str(),
    ];
    let assets = config.assets_of(dir, source);
    if let Some(a) = assets.as_ref().filter(|a| a.join("assets.toml").is_file()) {
        args.push("--assets".as_ref());
        args.push(a.as_os_str());
    }
    let (code, json) = run_gp(config, &args)?;
    Ok((code == 0, json))
}

#[cfg(test)]
mod asset_edit_tests;
#[cfg(test)]
mod place_tests;
#[cfg(test)]
mod sound_tests;
#[cfg(test)]
mod tests;
