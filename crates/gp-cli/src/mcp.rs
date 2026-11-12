//! `gp mcp`: the agent tools as an MCP server on stdin and stdout
//! (Milestone 3; docs/mcp.md). The tools are in `gp-mcp`;
//! this module finds the repo and the working directory and starts it.
//! Checks and validation run this same `gp` binary as a child process.
//! Brought from game-printer's mcp.rs; `--json` is refused in main.rs,
//! where the global flag is.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use serde_json::json;

use gp_evidence::Paths;

use crate::Outcome;

#[derive(Args)]
pub struct McpArgs {
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
}

fn absolute(p: &Path) -> Result<PathBuf> {
    std::path::absolute(p).with_context(|| p.display().to_string())
}

pub(crate) fn run(args: &McpArgs, paths: &Paths) -> Result<Outcome> {
    let cwd = std::env::current_dir()?;
    let repo = match &args.repo {
        Some(r) => absolute(r)?,
        None => gp_evidence::paths::enclosing_git_work_tree(&cwd)
            .ok_or_else(|| anyhow!("not in a git work tree; give the repo with --repo"))?,
    };
    if !repo.join("templates").is_dir() {
        bail!("{} has no templates/", repo.display());
    }
    let work = match &args.work {
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
    let config = gp_mcp::Config {
        gp: std::env::current_exe().context("finding the gp binary")?,
        repo,
        work,
        home: paths.home.clone(),
        rom: args.rom.as_deref().map(absolute).transpose()?,
        assets: args.assets.as_deref().map(absolute).transpose()?,
    };
    let shown = json!({ "repo": config.repo, "work": config.work });
    gp_mcp::server::serve_stdio(config).map_err(anyhow::Error::msg)?;
    // stdout was the MCP channel, so nothing more is printed there.
    Ok(Outcome::ok(shown, String::new()))
}
