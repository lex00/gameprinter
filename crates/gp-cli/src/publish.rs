//! The published gp: `$GP_HOME/bin/gp`, a link to `$GP_HOME/bin/gp-<commit>`,
//! which the tool gate moves as it lands patches (docs/gate.md, "Published
//! builds"), and `gp self path`, which names the file behind the link.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use serde_json::json;

use crate::Outcome;
use crate::gate::git;

/// How many earlier builds are kept beside the linked one.
const KEEP: usize = 8;

#[derive(Subcommand)]
pub enum SelfCmd {
    /// Print the newest published build: the file $GP_HOME/bin/gp points
    /// at, which the tool gate keeps at the newest commit it built gp for.
    /// A worker that isn't changing Rust runs this build instead of
    /// building gp. Exits 1 when none is published yet.
    Path,
    /// Publish this binary as the build of the commit checked out here, as
    /// the tool gate does after a landing. For a first setup, before any
    /// gate has run: `just publish`. Refuses a tree with uncommitted changes.
    Publish,
    /// Write the command list, CLI.md, from the commands gp has. With
    /// --check, fail if the file differs.
    Docs {
        /// The file to write or check (default CLI.md here).
        #[arg(default_value = "CLI.md")]
        file: PathBuf,
        #[arg(long)]
        check: bool,
    },
    /// Write every format's JSON Schema into DIR, one file per format with
    /// no version in its name, as schema/README.md says. `just docs` runs
    /// it on the schema/ folder.
    Schemas {
        /// The folder to write or check (default schema here).
        #[arg(default_value = "schema")]
        dir: PathBuf,
        /// Write nothing; exit 1 if a file is stale or missing, or a JSON
        /// file in DIR is no format's.
        #[arg(long)]
        check: bool,
    },
    /// Print the named features this build knows as Markdown (the source
    /// of docs/features.md, which `just docs` writes).
    Features,
    /// Print every command this gp has, from its own command tree: each
    /// one's words, help, and arguments and flags with their kinds, for
    /// `gp mcp`'s gp_commands and gp_run (docs/mcp.md, "Every command").
    Commands,
    /// Write the runtime shell's generated files (docs/shell.md): the
    /// storage interface's bindings from wit/shell/storage.wit (a Rust
    /// trait in gp-present and the JavaScript host's check) and the web
    /// preview's copy of the key table (scripts/web/page/shell-keys.js).
    /// Run in the repo; `just docs` runs it.
    Shell {
        /// Write nothing; exit 1 if a file is stale.
        #[arg(long)]
        check: bool,
    },
}

pub(crate) fn run(cmd: &SelfCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        SelfCmd::Path => self_path(paths),
        SelfCmd::Publish => self_publish(paths),
        SelfCmd::Docs { file, check } => crate::cli_doc::run(file, *check),
        SelfCmd::Schemas { dir, check } => crate::schemas::run(dir, *check),
        SelfCmd::Commands => crate::cli_tree::run(),
        SelfCmd::Features => {
            let md = crate::schemas::features_markdown();
            Ok(Outcome::ok(
                serde_json::json!({ "markdown": md }),
                md.trim_end().to_owned(),
            ))
        }
        SelfCmd::Shell { check } => {
            let here = std::env::current_dir()?;
            let Some(repo) = here.ancestors().find(|d| d.join("wit/shell").is_dir()) else {
                anyhow::bail!(
                    "run gp self shell inside the gameprinter repo (it reads wit/shell/)"
                );
            };
            crate::docs_shell::run(repo, *check)
        }
    }
}

/// What `$GP_HOME/bin/gp` points at, with its commit.
fn self_path(paths: &Paths) -> Result<Outcome> {
    let link = paths.home.join("bin").join("gp");
    let real = fs::canonicalize(&link).with_context(|| {
        format!(
            "no build is published at {}: the tool gate publishes one when it lands, or run `just publish`",
            link.display()
        )
    });
    let real = match real {
        Ok(r) => r,
        Err(e) => {
            return Ok(Outcome {
                json: json!({"path": null, "link": link, "error": format!("{e:#}")}),
                text: format!("{e:#}"),
                exit: 1,
            });
        }
    };
    let commit = commit_of(&real);
    Ok(Outcome::ok(
        json!({"path": real, "link": link, "commit": commit}),
        real.display().to_string(),
    ))
}

fn commit_of(real: &Path) -> Option<String> {
    real.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("gp-"))
        .map(str::to_owned)
}

fn self_publish(paths: &Paths) -> Result<Outcome> {
    let cwd = std::env::current_dir()?;
    let top = PathBuf::from(git(&cwd, &["rev-parse", "--show-toplevel"])?);
    if !git(&top, &["status", "--porcelain"])?.is_empty() {
        bail!(
            "{} has uncommitted changes, so a build of it has no commit to be named by",
            top.display()
        );
    }
    let commit = git(&top, &["rev-parse", "HEAD"])?;
    let line = publish(
        &paths.home,
        &std::env::current_exe()?,
        &commit,
        &top,
        &mut Vec::new(),
    )?;
    Ok(Outcome::ok(json!({"published": line.clone()}), line))
}

/// Copies `binary` to `home/bin/gp-<commit>` and points the link at it,
/// unless the link's commit already holds `commit`: the link only moves
/// forward, so a slow gate finishing late can't undo a newer landing.
pub(crate) fn publish(
    home: &Path,
    binary: &Path,
    commit: &str,
    repo: &Path,
    lines: &mut Vec<String>,
) -> Result<String> {
    let bin = home.join("bin");
    fs::create_dir_all(&bin)?;
    let short = &commit[..12];
    let dest = bin.join(format!("gp-{short}"));
    // A new file renamed into place: rewriting a Mach-O in place keeps its
    // inode, and macOS kills its next launch for a stale signature.
    let tmp = bin.join(format!("gp-{short}.tmp{}", std::process::id()));
    fs::copy(binary, &tmp).with_context(|| format!("copying {}", binary.display()))?;
    fs::rename(&tmp, &dest)?;

    let link = bin.join("gp");
    let held = fs::read_link(&link).ok().and_then(|p| commit_of(&p));
    let ahead = match &held {
        None => true,
        // An unknown linked commit (a rebased history) doesn't block.
        Some(old) => {
            git(repo, &["merge-base", "--is-ancestor", old, commit]).is_ok()
                || git(repo, &["cat-file", "-e", old]).is_err()
        }
    };
    let line = if ahead {
        let tmp = bin.join(format!("gp.new{}", std::process::id()));
        std::os::unix::fs::symlink(format!("gp-{short}"), &tmp)?;
        fs::rename(&tmp, &link)?;
        format!("published {short} as {}", link.display())
    } else {
        format!(
            "published {short} at {}; the link stays on the newer {}",
            dest.display(),
            held.unwrap_or_default()
        )
    };
    lines.push(line.clone());
    prune(&bin, &link);
    Ok(line)
}

/// Keeps the newest [`KEEP`] builds and the linked one.
fn prune(bin: &Path, link: &Path) {
    let keep = fs::read_link(link)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_owned()));
    let Ok(rd) = fs::read_dir(bin) else {
        return;
    };
    let mut builds: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name();
            let n = n.to_str()?;
            (n.starts_with("gp-") && !n.contains(".tmp")).then_some(())?;
            Some((e.metadata().ok()?.modified().ok()?, e.path()))
        })
        .collect();
    builds.sort();
    for (_, p) in builds.iter().rev().skip(KEEP) {
        if p.file_name() != keep.as_deref() {
            let _ = fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init(dir: &Path) {
        let run = |args: &[&str]| {
            let o = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(o.status.success(), "{o:?}");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.invalid"]);
        run(&["config", "user.name", "publish test"]);
        run(&["commit", "-q", "--allow-empty", "-m", "one"]);
        run(&["commit", "-q", "--allow-empty", "-m", "two"]);
    }

    /// The link moves forward and never back; a build published out of
    /// order stays beside it and `self path` still names the linked file.
    #[test]
    fn the_link_only_moves_forward() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).unwrap();
        init(&repo);
        let exe = dir.path().join("exe");
        fs::write(&exe, b"binary").unwrap();
        let home = dir.path().join("home");
        let commit = |n: &str| git(&repo, &["rev-parse", n]).unwrap();
        let older = commit("HEAD~1");
        let newer = commit("HEAD");
        let paths = Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: home.join("roms"),
        };
        let none = self_path(&paths).unwrap();
        assert_eq!(none.exit, 1, "{}", none.text);
        let mut lines = Vec::new();
        publish(&home, &exe, &newer, &repo, &mut lines).unwrap();
        let published = home.join("bin").join(format!("gp-{}", &newer[..12]));
        assert!(published.is_file());
        assert_eq!(
            fs::canonicalize(home.join("bin/gp")).unwrap(),
            fs::canonicalize(&published).unwrap()
        );
        let said = publish(&home, &exe, &older, &repo, &mut lines).unwrap();
        assert!(said.contains("stays"), "{said}");
        assert_eq!(
            fs::canonicalize(home.join("bin/gp")).unwrap(),
            fs::canonicalize(&published).unwrap(),
            "an older commit doesn't take the link"
        );
        assert!(
            home.join("bin")
                .join(format!("gp-{}", &older[..12]))
                .is_file()
        );
        assert_eq!(self_path(&paths).unwrap().exit, 0);
    }

    #[test]
    fn prune_keeps_the_linked_build() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        for i in 0..12 {
            let p = bin.join(format!("gp-{i:012}"));
            fs::write(&p, b"x").unwrap();
            let t =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i as u64);
            fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        let link = bin.join("gp");
        std::os::unix::fs::symlink(format!("gp-{:012}", 0), &link).unwrap();
        prune(&bin, &link);
        assert!(
            bin.join(format!("gp-{:012}", 0)).is_file(),
            "the linked one stays"
        );
        assert!(bin.join(format!("gp-{:012}", 11)).is_file());
        assert!(!bin.join(format!("gp-{:012}", 1)).exists());
    }
}
