//! `gp mcp`: the agent tools as an MCP server on stdin and stdout
//! (Milestone 3; docs/mcp.md). The tools are in `gp-mcp`;
//! this module finds the repo and the working directory and starts it.
//! Checks and validation run this same `gp` binary as a child process.
//! Brought from game-printer's mcp.rs; `--json` is refused in main.rs,
//! where the global flag is.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use serde_json::json;

use gp_evidence::Paths;

use crate::Outcome;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct McpArgs {
    #[command(subcommand)]
    pub(crate) cmd: Option<McpCmd>,
    /// The repo whose templates/ and baselines/ the tools read, and that
    /// evidence refs are read against (default: the git work tree of the
    /// current directory).
    #[arg(long, value_name = "DIR")]
    repo: Option<PathBuf>,
    /// Where working copies go, outside any git work tree (default:
    /// $GP_HOME/work/mcp).
    #[arg(long, value_name = "DIR")]
    work: Option<PathBuf>,
    /// The original's ROM, for validation, when it isn't found in
    /// $GP_ROM_DIR by hash (Moss Hop's is target/mosshop/mosshop.nes).
    #[arg(long)]
    rom: Option<PathBuf>,
    /// The lifted assets previews and validation read (default:
    /// $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    assets: Option<PathBuf>,
    /// The games checkout, whose game folders are templates the tools see
    /// beside the repo's templates/ (default: $GP_GAMES_DIR, else a `games`
    /// folder at or beside the current folder or one above it).
    #[arg(long, value_name = "DIR")]
    games: Option<PathBuf>,
    /// A loop worker's own worktree of the games checkout: gp_run runs its
    /// commands there, all but the gates, which land on --games.
    #[arg(long, value_name = "DIR")]
    worker_games: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum McpCmd {
    /// How to connect your agent (Claude Code, Codex, Cursor, Gemini CLI,
    /// VS Code) to gp mcp: what this machine has of each, the line that
    /// connects it, and with --write the connection made after printing
    /// what it writes.
    Setup(crate::mcp_setup::SetupArgs),
}

fn absolute(p: &Path) -> Result<PathBuf> {
    std::path::absolute(p).with_context(|| p.display().to_string())
}

/// What `gp mcp` says when it finds no templates/ to serve: one line, with
/// what to install.
pub(crate) const NO_REPO: &str = "gp mcp found no templates/ to serve: this gp carries none \
     and the checkout it was built from is gone; install a released gp \
     (https://github.com/lex00/gameprinter, the install page), or run it in a gameprinter \
     checkout or give one with --repo";

/// The repo `gp mcp` serves from `cwd`: `given`, else the git work tree
/// of `cwd` when it has templates/, else the files a released gp carries,
/// else the checkout gp was built from (a source build, the published
/// ones too). So `gp mcp` started outside any repo, as an agent app
/// starts a plugin's server, works on what the binary has.
pub(crate) fn repo_for(given: Option<&Path>, cwd: &Path) -> Result<PathBuf> {
    let has = |d: &Path| d.join("templates").is_dir();
    let repo = match given {
        Some(r) => absolute(r)?,
        None => gp_evidence::paths::enclosing_git_work_tree(cwd)
            .filter(|t| has(t))
            .or_else(|| crate::bundle::root().filter(|r| has(r)))
            .or_else(|| {
                crate::repo::built()
                    .filter(|b| has(b))
                    .map(|b| b.canonicalize().unwrap_or(b))
            })
            .ok_or_else(|| anyhow!(NO_REPO))?,
    };
    if !has(&repo) {
        bail!("{} has no templates/", repo.display());
    }
    Ok(repo)
}

/// Why `gp mcp` with no options, started in `cwd`, would not start, or
/// None when it would: the session-start hook says so (crate::loop_hook),
/// since an agent app that saw the server fail keeps the failure a while.
pub(crate) fn wont_start(cwd: &Path, paths: &Paths) -> Option<String> {
    let check = || -> Result<()> {
        repo_for(None, cwd)?;
        work_for(None, paths)?;
        Ok(())
    };
    check().err().map(|e| format!("{e:#}"))
}

/// The working copies' folder, made, refused inside a git work tree.
fn work_for(given: Option<&Path>, paths: &Paths) -> Result<PathBuf> {
    let work = match given {
        Some(w) => absolute(w)?,
        None => paths.home.join("work").join("mcp"),
    };
    // Checked before anything is created, so a refused --work leaves no
    // directory behind in a work tree.
    let existing = work
        .ancestors()
        .find(|p| p.exists())
        .unwrap_or(Path::new("/"));
    if let Some(tree) = gp_evidence::paths::enclosing_git_work_tree(existing) {
        bail!(
            "{} is inside the git work tree {}; working copies go outside it",
            work.display(),
            tree.display()
        );
    }
    std::fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
    Ok(work)
}

pub(crate) fn run(args: &McpArgs, paths: &Paths) -> Result<Outcome> {
    if let Some(McpCmd::Setup(s)) = &args.cmd {
        return crate::mcp_setup::run(s);
    }
    let repo = repo_for(args.repo.as_deref(), &std::env::current_dir()?)?;
    let work = work_for(args.work.as_deref(), paths)?;
    let config = gp_mcp::Config {
        gp: std::env::current_exe().context("finding the gp binary")?,
        repo,
        work,
        home: paths.home.clone(),
        rom: args.rom.as_deref().map(absolute).transpose()?,
        assets: args.assets.as_deref().map(absolute).transpose()?,
        games: crate::roms_cmd::games_dir(args.games.as_deref())
            .map(|g| absolute(&g))
            .transpose()?,
        worker_games: args.worker_games.as_deref().map(absolute).transpose()?,
    };
    let shown = json!({ "repo": config.repo, "work": config.work, "games": config.games });
    // The tools' `gp query` children ask this process, which keeps the
    // recordings open between them (crate::session).
    let _sessions = crate::session::serve_in_background(paths);
    // The server says gp's own version, which a plugin pins (docs/mcp.md,
    // "Keeping up"), not gp-mcp's crate version.
    gp_mcp::server::serve_stdio(config, env!("CARGO_PKG_VERSION")).map_err(anyhow::Error::msg)?;
    // stdout was the MCP channel, so nothing more is printed there.
    Ok(Outcome::ok(shown, String::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Started in a folder of no repo, as an agent app starts the plugin's
    /// server, a source build serves the checkout it was built from.
    #[test]
    fn gp_mcp_outside_a_repo_serves_the_checkout_it_was_built_from() {
        let t = tempfile::tempdir().unwrap();
        let repo = repo_for(None, t.path()).unwrap();
        assert!(repo.join("templates").is_dir(), "{}", repo.display());
        let built = crate::repo::built().unwrap().canonicalize().unwrap();
        assert_eq!(repo.canonicalize().unwrap(), built);
        let home = t.path().join("home");
        let paths = Paths {
            store: home.join("store"),
            roms: home.join("roms"),
            home,
        };
        assert_eq!(wont_start(t.path(), &paths), None);
        // A git work tree without templates/ (a theme's repo) too.
        std::fs::create_dir(t.path().join(".git")).unwrap();
        assert_eq!(
            repo_for(None, t.path()).unwrap().canonicalize().unwrap(),
            built
        );
        // A --repo without templates/ is still refused, in one line.
        let e = repo_for(Some(t.path()), t.path()).unwrap_err().to_string();
        assert!(e.ends_with("has no templates/"), "{e}");
        assert!(!NO_REPO.contains('\n'));
        // Working copies inside a git work tree: it says why it won't start.
        let why = wont_start(t.path(), &paths).unwrap();
        assert!(why.contains("working copies go outside it"), "{why}");
    }
}
