//! The workers' branches, and the count of what the workers left (issue
//! 52). Each worker gets a git worktree, `$GP_HOME/work/loop/trees/ID` on
//! branch `loop/ID`, in the games checkout or gameprinter's. Its worktree
//! goes a day after it ended (crate::builds::ended_worktrees, by `gp loop
//! clean` and gp serve's hourly sweep), and its branch stays. This removes
//! the branch once its tree is gone, and counts both for `gp loop health`:
//! on 2026-11-26 the two repos held several hundred `loop/` branches.
//!
//! A branch goes only when no tree of its step is left, its worker isn't
//! at work, and no card of its step is open, holding a yes or running. A
//! worker's log, brief and record are elsewhere (`files/ID`,
//! `workers/ID.json`) and are kept.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::builds::step_id_ok;

/// Where the workers' worktrees are.
pub(crate) fn trees_dir(home: &Path) -> PathBuf {
    gp_mcp::steps::loop_dir(home).join("trees")
}

/// The branch of the worker on step `id`.
pub(crate) fn branch_of(id: &str) -> String {
    format!("loop/{id}")
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    crate::serve::work::git_ok(dir, args).map_err(|e| format!("{e:#}"))
}

/// How many worktrees the workers have left under `home`.
pub(crate) fn trees(home: &Path) -> usize {
    fs::read_dir(trees_dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| step_id_ok(&e.file_name().to_string_lossy()) && e.path().is_dir())
        .count()
}

/// The workers' branches (`loop/ID`) in checkout `repo`, by step id. A
/// branch under `loop/` that names no step is no worker's.
pub(crate) fn branches(repo: &Path) -> Vec<String> {
    git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads/loop/",
        ],
    )
    .unwrap_or_default()
    .lines()
    .filter_map(|b| b.strip_prefix("loop/"))
    .filter(|id| step_id_ok(id))
    .map(str::to_owned)
    .collect()
}

/// Deletes, or with `dry_run` lists, each `loop/ID` branch in `repos`
/// whose tree is gone, unless its worker is at work or a card of its step
/// still waits (open, holding a yes or running). Gives each as `{"repo",
/// "id"}`.
pub(crate) fn prune_branches(
    home: &Path,
    repos: &[PathBuf],
    now: u64,
    dry_run: bool,
) -> Vec<Value> {
    let workers = gp_mcp::steps::workers(home);
    let asks = gp_mcp::asks::all(home);
    let kept = |id: &str| {
        trees_dir(home).join(id).exists()
            || workers
                .iter()
                .any(|w| w["step"]["id"] == json!(id) && gp_mcp::steps::holds(w))
            || asks.iter().any(|a| {
                a["step"] == json!(id)
                    && (gp_mcp::asks::is_open(a, now)
                        || gp_mcp::asks::is_yes(a, now)
                        || a["state"] == json!("running"))
            })
    };
    let mut gone = Vec::new();
    for repo in repos {
        for id in branches(repo) {
            if kept(&id) {
                continue;
            }
            if dry_run || git(repo, &["branch", "-D", &branch_of(&id)]).is_ok() {
                gone.push(json!({"repo": repo, "id": id}));
            }
        }
    }
    gone
}

/// [`prune_branches`]' outcome as a line a person reads; None when no
/// branch went.
pub(crate) fn words(gone: &[Value], dry_run: bool) -> Option<String> {
    (!gone.is_empty()).then(|| {
        format!(
            "{} {} worker branch(es) (loop/STEP) whose worktree is gone",
            if dry_run { "Would delete" } else { "Deleted" },
            gone.len()
        )
    })
}

/// Past this many trees or branches left, gp loop health says PROBLEM.
pub(crate) const MANY: usize = 200;

/// gp loop health's line on the trees and branches left: how many of each
/// (branches counted over `repos`), a problem past [`MANY`] of either.
pub(crate) fn health_line(home: &Path, repos: &[PathBuf]) -> Result<String, String> {
    let branches: usize = repos.iter().map(|r| branches(r).len()).sum();
    health_words(trees(home), branches)
}

fn health_words(trees: usize, branches: usize) -> Result<String, String> {
    if trees > MANY || branches > MANY {
        return Err(format!(
            "{trees} worker trees and {branches} loop/ branches are left (over {MANY}): gp loop clean removes the trees of workers that ended a day ago and the branches whose tree is gone, their logs and records kept (--dry-run lists them)."
        ));
    }
    Ok(format!(
        "{trees} worker tree(s) and {branches} loop/ branch(es) left"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git_t(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// Issue 52: a worker's branch goes once its tree is gone, unless its
    /// worker is at work or a card of its step waits; a branch that is no
    /// worker's is never touched, and a dry run deletes nothing.
    #[test]
    fn a_branch_whose_tree_is_gone_goes_unless_a_card_waits() {
        let t = tempfile::tempdir().unwrap();
        let (home, repo) = (t.path().join("home"), t.path().join("repo"));
        fs::create_dir_all(&repo).unwrap();
        git_t(&repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join("a.txt"), "a\n").unwrap();
        git_t(&repo, &["add", "."]);
        git_t(&repo, &["commit", "-qm", "first"]);
        let now = 1_000 * 3_600_000;
        let (gone, tree, waits, live) = (
            "s00000000aa01",
            "s00000000aa02",
            "s00000000aa03",
            "s00000000aa04",
        );
        for id in [gone, tree, waits, live] {
            git_t(&repo, &["branch", &branch_of(id), "main"]);
        }
        git_t(&repo, &["branch", "loop/not-a-step", "main"]);
        // One still has its tree, one a card that waits, one a live worker.
        fs::create_dir_all(trees_dir(&home).join(tree)).unwrap();
        fs::create_dir_all(gp_mcp::asks::dir(&home)).unwrap();
        fs::write(
            gp_mcp::asks::dir(&home).join("k-1.json"),
            json!({"id": "k-1", "step": waits, "state": "open", "made_ms": now - 1000}).to_string(),
        )
        .unwrap();
        fs::create_dir_all(gp_mcp::steps::workers_dir(&home)).unwrap();
        fs::write(
            gp_mcp::steps::workers_dir(&home).join(format!("{live}.json")),
            json!({"step": {"id": live}, "state": "working", "pid": std::process::id()})
                .to_string(),
        )
        .unwrap();
        let repos = std::slice::from_ref(&repo);
        let dry = prune_branches(&home, repos, now, true);
        assert_eq!(dry.len(), 1);
        assert_eq!(branches(&repo).len(), 4, "a dry run deletes nothing");
        let v = prune_branches(&home, repos, now, false);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0]["id"], gone);
        let mut left = branches(&repo);
        left.sort();
        assert_eq!(left, [tree, waits, live]);
        assert!(git(&repo, &["rev-parse", "--verify", "loop/not-a-step"]).is_ok());
        assert_eq!(
            words(&v, false).unwrap(),
            "Deleted 1 worker branch(es) (loop/STEP) whose worktree is gone"
        );
        assert_eq!(words(&[], false), None);
    }

    /// Issue 52: gp loop health counts the trees and branches left, a
    /// problem past the threshold.
    #[test]
    fn health_counts_the_trees_and_branches_left() {
        assert_eq!(
            health_words(3, 5).unwrap(),
            "3 worker tree(s) and 5 loop/ branch(es) left"
        );
        let p = health_words(MANY + 1, 2).unwrap_err();
        assert!(
            p.contains("201 worker trees") && p.contains("gp loop clean"),
            "{p}"
        );
        assert!(health_words(0, MANY + 1).is_err());
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(trees_dir(t.path()).join("s00000000cc01")).unwrap();
        fs::create_dir_all(trees_dir(t.path()).join("not-a-step")).unwrap();
        assert_eq!(
            health_line(t.path(), &[]).unwrap(),
            "1 worker tree(s) and 0 loop/ branch(es) left"
        );
    }
}
