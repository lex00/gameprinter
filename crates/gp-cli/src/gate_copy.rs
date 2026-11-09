//! A working copy with no git of its own (docs: packaging 6.1): the gate
//! keeps a private git of it in its home, with no remote and the
//! ROM-blocking hook installed. The folder's edits since the last landed
//! commit are the patch; a landing is a commit in the private git, and
//! publishing is a separate step. The tree has the shape of a checkout of
//! the games repo, with the package under `game/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result};
use serde_json::json;

use crate::Outcome;
use crate::gate::{Patch, apply, commit, git, hook};

const HOOK: &str = include_str!("../../../.githooks/pre-commit");
const ALLOWLIST: &str = include_str!("../../../.githooks/binary-allowlist");
const GUARDED: &str = include_str!("../../../.githooks/guarded-roms.toml");
const EXCERPTS: &str = include_str!("../../../.githooks/rom-excerpt-allowlist");
const ROM_TEXT: &str = include_str!("../../../scripts/romguard/rom_text.py");
/// git's empty tree, what a first look at a folder is measured against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

pub(crate) enum Opened {
    /// Nothing more for the gate to do: a first look, or no edits.
    Done(Outcome),
    Gate {
        repo: PathBuf,
        tree: PathBuf,
        diff: PathBuf,
    },
}

/// The private git for `copy`, made if missing, the hook kept current, and
/// the folder's edits since the last landed commit written to `edits.patch`.
pub(crate) fn open(copy: &Path, home: &Path, gp: &Path, message: Option<&str>) -> Result<Opened> {
    let name = copy
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "copy".into());
    let key = blake3::hash(copy.as_os_str().as_encoded_bytes()).to_hex();
    let root = home.join("private").join(format!("{name}-{}", &key[..12]));
    let repo = root.join("repo");
    let plain = Patch {
        diff: PathBuf::new(),
        message: None,
        author: None,
        date: None,
    };
    if !repo.join(".git").exists() {
        fs::create_dir_all(&repo)?;
        git(&repo, &["init", "-q", "-b", "main"])?;
        git(&repo, &["config", "core.hooksPath", ".githooks"])?;
        // Commits here need no person's identity: the person's edits and
        // the gate's own housekeeping are told apart by the messages.
        git(&repo, &["config", "user.name", "gp model gate"])?;
        git(
            &repo,
            &["config", "user.email", "gate@game-printer.invalid"],
        )?;
    }
    fs::create_dir_all(repo.join(".git/info"))?;
    fs::write(repo.join(".git/info/exclude"), "lifted/\ntarget/\n")?;
    write_hooks(&repo)?;
    git(&repo, &["add", ".githooks", "scripts"])?;
    if !git(&repo, &["diff", "--cached", "--name-only"])?.is_empty() {
        commit(
            &repo,
            "The gate's ROM-blocking hook, from this gp",
            &plain,
            gp,
        )?;
    }

    let head = git(&repo, &["rev-parse", "HEAD"])?;
    let first = git(&repo, &["rev-parse", "--verify", "-q", "HEAD:game"]).is_err();
    let base = if first {
        EMPTY_TREE.to_owned()
    } else {
        git(&repo, &["rev-parse", "HEAD:game"])?
    };
    let now = tree_of(&repo, copy, &root, (!first).then_some(&base))?;
    if now == base {
        let text = format!(
            "nothing has changed since the last landed commit {}",
            &head[..12]
        );
        return Ok(Opened::Done(Outcome::ok(
            json!({"verdict": "UNCHANGED", "commit": head, "lines": [text.clone()]}),
            text,
        )));
    }
    let diff = root.join("edits.patch");
    let o = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args([
            "diff",
            "--binary",
            "--src-prefix=a/game/",
            "--dst-prefix=b/game/",
            &base,
            &now,
        ])
        .output()?;
    if !o.status.success() {
        anyhow::bail!("git diff: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    fs::write(&diff, &o.stdout)?;
    if first {
        return Ok(Opened::Done(baseline(&repo, &diff, gp, &plain, message)?));
    }
    Ok(Opened::Gate {
        tree: root.join("tree"),
        repo,
        diff,
    })
}

/// The gate's copy of the ROM-blocking hook and what it reads, in the
/// private repo's tree. `.githooks` beside it, as in the games repo.
fn write_hooks(repo: &Path) -> Result<()> {
    let hooks = repo.join(".githooks");
    fs::create_dir_all(&hooks)?;
    let exec = |path: PathBuf, text: &str| -> Result<()> {
        fs::write(&path, text)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        Ok(())
    };
    exec(hooks.join("pre-commit"), HOOK)?;
    fs::write(hooks.join("binary-allowlist"), ALLOWLIST)?;
    fs::write(hooks.join("guarded-roms.toml"), GUARDED)?;
    fs::write(hooks.join("rom-excerpt-allowlist"), EXCERPTS)?;
    let romguard = repo.join("scripts/romguard");
    fs::create_dir_all(&romguard)?;
    exec(romguard.join("rom_text.py"), ROM_TEXT)?;
    Ok(())
}

/// The tree of `copy`'s files as git sees them (ignored files left out),
/// built in a private index so the private git's real index isn't touched.
fn tree_of(repo: &Path, copy: &Path, root: &Path, base: Option<&String>) -> Result<String> {
    let index = root.join("index-working-copy");
    let _ = fs::remove_file(&index);
    let env = |c: &mut Command| {
        c.env("GIT_DIR", repo.join(".git"))
            .env("GIT_WORK_TREE", copy)
            .env("GIT_INDEX_FILE", &index)
            .current_dir(copy);
    };
    let git_here = |args: &[&str]| -> Result<String> {
        let mut c = Command::new("git");
        env(&mut c);
        let o = c.args(args).output()?;
        if !o.status.success() {
            anyhow::bail!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    match base {
        Some(b) => git_here(&["read-tree", b])?,
        None => git_here(&["read-tree", "--empty"])?,
    };
    git_here(&["add", "-A", "--", "."])?;
    git_here(&["write-tree"])
}

/// A first look: the folder's files, through the hook, as the private
/// git's first commit of the game. Nothing to score it against yet.
fn baseline(
    repo: &Path,
    diff: &Path,
    gp: &Path,
    plain: &Patch,
    message: Option<&str>,
) -> Result<Outcome> {
    let refuse = |why: String| {
        let _ = git(repo, &["reset", "-q", "--hard"]);
        let _ = git(repo, &["clean", "-fdq", "--", "game"]);
        Outcome {
            json: json!({"verdict": "FAIL", "phase": "hook", "why": why}),
            text: format!("FAIL (hook): the ROM-blocking hook refuses this folder: {why}"),
            exit: 1,
        }
    };
    if let Err(why) = apply(repo, diff) {
        return Ok(refuse(why));
    }
    if let Err(why) = hook(repo, gp) {
        return Ok(refuse(why));
    }
    let msg = message.unwrap_or("The working copy, as the gate first found it");
    commit(repo, msg, plain, gp)?;
    let head = git(repo, &["rev-parse", "HEAD"])?;
    let text =
        format!("BASELINE: {head} holds the folder as it stands. Edit it and run the gate again.");
    Ok(Outcome::ok(
        json!({"verdict": "BASELINE", "commit": head, "lines": [text.clone()]}),
        text,
    ))
}

/// A landing's measured-again corpus files, written back into the folder.
pub(crate) fn write_back(repo: &Path, commit: &str, copy: &Path) -> Result<()> {
    let changed = git(
        repo,
        &[
            "diff",
            "--name-only",
            &format!("{commit}~1"),
            commit,
            "--",
            "game",
        ],
    )?;
    for f in changed.lines() {
        let Some(rel) = f.strip_prefix("game/") else {
            continue;
        };
        let body = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["show", &format!("{commit}:{f}")])
            .output()?;
        if !body.status.success() {
            continue;
        }
        let dest = copy.join(rel);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(&dest, &body.stdout).with_context(|| dest.display().to_string())?;
    }
    Ok(())
}
