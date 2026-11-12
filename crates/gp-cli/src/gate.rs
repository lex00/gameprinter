//! `gp model gate`: a patch to a game in the `games` repo, held to the
//! corpus. A worktree of the repo in the gate's home is put at the branch
//! tip and scored; the patch is applied there, the game's module built for
//! its hooks, the package scanned for what it must not hold, and the corpus
//! scored again on the patched game through the patched module. A patch that
//! leaves any clip worse fails, and what passes lands on the branch. Gates
//! waiting on one home are taken into one batch (`gate_queue`), settled by
//! halves when it fails, and a patch made on an older tip is merged onto it
//! three ways (docs/gate.md). The old gate's build of gp per commit is
//! gone: gp is the running binary, and a patch only ever reaches the game's
//! files.
//!
//! On a working copy with no git of its own (a folder from `gp game fetch`)
//! the gate keeps a private git of it in its home (`gate_copy`): the edits
//! since the last landed commit are the patch, and a landing is a commit
//! there. A tool patch is the other half (`tool_gate`), and shares the
//! apply, hook, tree and landing code here.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::clip_score::rom_of;
use crate::corpus_score::{CorpusCmd, lift_into};

/// Tries before giving up on a tip that keeps moving.
const TIP_TRIES: usize = 5;

#[derive(Args)]
pub struct GateArgs {
    /// The game's package folder in a checkout of the games repo. For a new
    /// game it may be a folder the tip doesn't have yet, which the patch
    /// creates with its package.toml.
    pub game: PathBuf,
    /// The patch: `git diff` output, or a `git format-patch` mail, which
    /// keeps its own message and author. It may only change the game's files.
    /// Left out for a working copy with no git of its own (the gate takes
    /// the edits since the last landed commit) and for --score-only.
    #[arg(long, conflicts_with = "import")]
    pub patch: Option<PathBuf>,
    /// Gate the clips the mechanic sessions have that the corpus hasn't, in
    /// place of a patch: `gp corpus import GAME --write` on the tip, with
    /// the person's sessions ($GP_HOME), gated as a patch. Nothing to
    /// import lands nothing.
    #[arg(long, conflicts_with = "score_only")]
    pub import: bool,
    /// The run in the gate's home the corpus's clips are made in.
    #[arg(long, default_value = "gate")]
    pub run: String,
    /// The branch to land on (default: the one checked out where GAME is).
    #[arg(long)]
    pub onto: Option<String>,
    /// The gate's own home (default: $GP_GATE_HOME, else ~/.gameprinter/gate).
    /// Its runs, scores, modules and worktree are kept there.
    #[arg(long)]
    pub home: Option<PathBuf>,
    /// The commit message, for a patch that isn't a format-patch mail.
    #[arg(long, short)]
    pub message: Option<String>,
    /// Say PASS or FAIL and land nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// A clip the patch removes from the corpus on purpose: its ID, or a
    /// pattern with `*`. A patch that removes a clip it doesn't name fails,
    /// since no score compares a clip that is gone.
    #[arg(long = "removes", value_name = "ID")]
    pub removes: Vec<String>,
    /// Score the tip and stop: no patch, nothing lands. The tool gate's
    /// canary runs this through the patched gp and compares the rows.
    #[arg(long, conflicts_with = "patch")]
    pub score_only: bool,
    /// Apply the patch only as it is. By default a patch that no longer
    /// applies on the tip is merged onto it three ways and the merge is
    /// gated. A gate with --no-rebase or --dry-run batches with no other.
    #[arg(long)]
    pub no_rebase: bool,
}

/// What every try reads.
struct Ctx {
    repo: PathBuf,
    /// The game's folder inside the repo.
    rel: PathBuf,
    /// A working copy the private git follows, for what a landing writes
    /// back to it.
    copy: Option<PathBuf>,
    /// The gate's worktree.
    tree: PathBuf,
    home: PathBuf,
    onto: String,
    run: String,
    /// Paths with `home` the gate's, so its runs stay out of the person's.
    paths: Paths,
    /// `gp game kit` output for this run.
    kit: PathBuf,
    kit_hash: String,
}

/// `Err` with git's own words.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !o.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// `gp` itself as a command, with the gate's home as its $GP_HOME.
fn self_cmd(c: &Path) -> Result<Command> {
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.env("GP_HOME", c);
    Ok(cmd)
}

/// The module for the game in folder `game`, built once for its hook code
/// and the kit; `None` for a game with no hooks.
fn module_at(c: &Ctx, game: &Path, lines: &mut Vec<String>) -> Result<Option<PathBuf>> {
    let Some(ext) = crate::package::hook_code(game)? else {
        return Ok(None);
    };
    let key = blake3::hash(format!("{ext}\n{}", c.kit_hash).as_bytes()).to_hex();
    let out = c.home.join("modules").join(format!("{key}.wasm"));
    if out.is_file() {
        lines.push(format!("module for hook code {} reused", &ext[..12]));
        return Ok(Some(out));
    }
    fs::create_dir_all(out.parent().expect("a parent"))?;
    let started = std::time::Instant::now();
    let tmp = out.with_extension("tmp");
    let o = self_cmd(&c.home)?
        .args(["game", "build"])
        .arg(game)
        .arg("--kit")
        .arg(&c.kit)
        .arg("-o")
        .arg(&tmp)
        .output()?;
    if !o.status.success() {
        bail!(
            "gp game build: {}{}",
            String::from_utf8_lossy(&o.stdout).trim(),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    fs::rename(&tmp, &out)?;
    lines.push(format!(
        "module built for hook code {} in {} s",
        &ext[..12],
        started.elapsed().as_secs()
    ));
    Ok(Some(out))
}

/// The lifted assets for the game in `game`, by its asset map: a patch that
/// changes the map changes the lift.
fn assets_for(c: &Ctx, game: &Path, rom: &[u8]) -> Result<PathBuf> {
    let (_, map) = crate::corpus_score::asset_map(game, rom)?;
    let out = c
        .home
        .join("assets")
        .join(blake3::hash(map.as_bytes()).to_hex().as_str());
    lift_into(game, rom, &out)?;
    Ok(out)
}

/// A score's rows, in the corpus's order, and the file holding its whole
/// JSON.
fn score(
    c: &Ctx,
    game: &Path,
    module: Option<&Path>,
    assets: &Path,
    reuse: Option<&Path>,
    remeasure: bool,
    name: &str,
) -> Result<(Rows, PathBuf)> {
    let cmd = CorpusCmd::Score {
        game: game.to_owned(),
        run: c.run.clone(),
        module: module.map(Path::to_owned),
        model: None,
        assets: Some(assets.to_owned()),
        rom: None,
        only: Vec::new(),
        reuse: reuse.map(Path::to_owned),
        write: None,
        remeasure: remeasure.then(|| game.to_owned()),
    };
    let o = crate::corpus_score::run(&cmd, &c.paths)?;
    let dir = c.home.join("scores");
    fs::create_dir_all(&dir)?;
    let f = dir.join(format!("{name}-{}.json", c.run));
    fs::write(&f, serde_json::to_vec(&o.json)?)?;
    let clips = o.json["record"]["predicate"]["clips"]
        .as_array()
        .context("gp corpus score's record has no clips")?;
    let rows = clips
        .iter()
        .map(|v| (v["id"].as_str().unwrap_or("").to_owned(), Row::of(v)))
        .collect();
    Ok((rows, f))
}

/// The files of the package scan's needs-rewrite items, by count.
fn rewrites(v: &Value) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for part in v["parts"].as_array().into_iter().flatten() {
        for f in part["files"].as_array().into_iter().flatten() {
            let n = f["rewrite"].as_u64().unwrap_or(0);
            if n > 0 {
                out.insert(f["file"].as_str().unwrap_or("").to_owned(), n);
            }
        }
    }
    out
}

/// The tree at `commit`, clean.
pub(crate) fn checkout(repo: &Path, tree: &Path, commit: &str) -> Result<()> {
    if tree.join(".git").exists() {
        let _ = git(tree, &["am", "--abort"]);
        let _ = git(tree, &["reset", "-q", "--hard"]);
        git(tree, &["checkout", "--detach", "--force", commit])?;
        // The gate's own tree: what an earlier patch added goes, in the
        // template or in gp's code.
        git(tree, &["clean", "-fdq"])?;
    } else {
        git(repo, &["worktree", "prune"])?;
        git(
            repo,
            &[
                "worktree",
                "add",
                "--detach",
                &tree.to_string_lossy(),
                commit,
            ],
        )?;
    }
    Ok(())
}

/// The tree back to its last commit, so a refused patch's files don't stay
/// on disk until the next gate.
pub(crate) fn clean(tree: &Path) {
    let _ = git(tree, &["reset", "-q", "--hard"]);
    let _ = git(tree, &["clean", "-fdq"]);
}

/// A failure's reason and the phase it failed in.
#[derive(Clone)]
pub(crate) struct Failed {
    pub why: String,
    pub phase: &'static str,
}

/// A patch file, read: the diff, and what a `git format-patch` mail
/// carries of its message and author.
pub(crate) struct Patch {
    pub diff: PathBuf,
    pub message: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
}

/// Reads a patch or a one-commit mail, whose diff goes to `work`. A mail
/// with several commits is refused: each lands through a gate of its own.
pub(crate) fn read_patch(file: &Path, work: &Path) -> Result<std::result::Result<Patch, String>> {
    let text = fs::read(file).with_context(|| file.display().to_string())?;
    let plain = Patch {
        diff: std::path::absolute(file)?,
        message: None,
        author: None,
        date: None,
    };
    if !text.starts_with(b"From ") {
        return Ok(Ok(plain));
    }
    let commits = String::from_utf8_lossy(&text)
        .lines()
        .filter(|l| l.starts_with("From ") && l.ends_with("Mon Sep 17 00:00:00 2001"))
        .count();
    if commits > 1 {
        return Ok(Err(format!(
            "the mail holds {commits} commits; land them one at a time"
        )));
    }
    fs::create_dir_all(work)?;
    let (msg, diff) = (work.join("mail-message"), work.join("mail-patch"));
    let o = Command::new("git")
        .arg("mailinfo")
        .arg(&msg)
        .arg(&diff)
        .stdin(fs::File::open(file)?)
        .output()?;
    if !o.status.success() {
        return Ok(Err(format!(
            "git mailinfo: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )));
    }
    let info = String::from_utf8_lossy(&o.stdout).into_owned();
    let field = |k: &str| {
        info.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}: ")).map(str::to_owned))
    };
    let body = fs::read_to_string(&msg).unwrap_or_default();
    let subject = field("Subject").unwrap_or_default();
    Ok(Ok(Patch {
        diff,
        message: Some(format!("{subject}\n\n{body}").trim().to_owned()),
        author: field("Author").map(|a| format!("{a} <{}>", field("Email").unwrap_or_default())),
        date: field("Date"),
    }))
}

/// Applies the diff on the clean tree, staged: as it is, else merged three
/// ways. A commit comes only after the checks pass.
pub(crate) fn apply(tree: &Path, diff: &Path) -> std::result::Result<(), String> {
    apply_how(tree, diff, true).map(|_| ())
}

/// [`apply`], saying how: `None` when the diff applied as it is, else how a
/// three-way merge went. With `rebase` false (`--no-rebase`) the diff must
/// apply as it is. A merge's conflicts are resolved where a file has a rule
/// (docs/gate.md, "Patch"): a generated file takes the tip's side, to be
/// written again; an append-only file keeps both sides' added rows; a
/// game's model.toml or fidelity.toml merges through its canonical text.
/// A conflict anywhere else fails, naming the files, with the tree left
/// clean.
pub(crate) fn apply_how(
    tree: &Path,
    diff: &Path,
    rebase: bool,
) -> std::result::Result<Option<String>, String> {
    let p = diff.to_string_lossy();
    if git(tree, &["apply", "--check", &p]).is_ok() || !rebase {
        git(tree, &["apply", "--index", &p]).map_err(|e| {
            format!(
                "the patch doesn't apply on the tip as it is: {}",
                format!("{e:#}").trim_start_matches("git apply --index ")
            )
        })?;
        return Ok(None);
    }
    if git(tree, &["apply", "--3way", &p]).is_ok() {
        return Ok(Some(
            "the patch didn't apply as it is on the tip; merged three ways cleanly".into(),
        ));
    }
    let unmerged = git(tree, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
    let left = resolve_conflicts(tree);
    if left.is_empty() && !unmerged.is_empty() {
        return Ok(Some(format!(
            "the patch didn't apply as it is on the tip; merged three ways, {}",
            merged_how(&unmerged)
        )));
    }
    let _ = git(tree, &["reset", "-q", "--hard"]);
    if left.is_empty() {
        Err("the patch doesn't apply on the tip, even three ways".into())
    } else {
        Err(format!(
            "a three-way merge conflicts in {}",
            left.join(", ")
        ))
    }
}

/// How a three-way merge's conflicts in `unmerged` (git's list) were
/// resolved, for the gate's line.
fn merged_how(unmerged: &str) -> String {
    let mut how = Vec::new();
    for f in unmerged.lines() {
        if crate::gate_append::APPEND_ONLY.contains(&f) {
            how.push(format!("{f} by its added rows"));
        } else if generated_file(f) {
            how.push(format!("{f} written again"));
        } else if canonical_file(f) {
            how.push(format!("{f} through its canonical form"));
        }
    }
    how.join(", ")
}

/// The files gp writes of itself, which a check holds to the code: the
/// tool repo's CLI.md (`gp self docs`).
pub(crate) fn generated_file(f: &str) -> bool {
    f == "CLI.md"
}

/// Writes CLI.md in a tool tree with `gp`, the build of that tree, after a
/// merge took the tip's side of it. Returns whether it changed; a tree with
/// no CLI.md has nothing to write. What it writes is staged.
pub(crate) fn regenerate(tree: &Path, gp: &Path) -> Result<bool> {
    if !tree.join("CLI.md").is_file() {
        return Ok(false);
    }
    let o = Command::new(gp)
        .current_dir(tree)
        .args(["self", "docs", "CLI.md"])
        .output()
        .with_context(|| format!("running {}", gp.display()))?;
    if !o.status.success() {
        bail!(
            "gp self docs fails: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    git(tree, &["add", "--", "CLI.md"])?;
    Ok(git(
        tree,
        &["diff", "--cached", "--quiet", "HEAD", "--", "CLI.md"],
    )
    .is_err())
}

/// After a three-way merge that conflicts: each conflicted file with a rule
/// resolved and staged. Returns the files still in conflict.
fn resolve_conflicts(tree: &Path) -> Vec<String> {
    let mut left: Vec<String> = git(tree, &["diff", "--name-only", "--diff-filter=U"])
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    // A generated file is written again from the merged code (the tool
    // gate's `regenerate`), so either side will do.
    left.retain(|f| {
        !(generated_file(f)
            && git(tree, &["checkout", "--ours", "--", f]).is_ok()
            && git(tree, &["add", "--", f]).is_ok())
    });
    left.retain(|f| {
        !(crate::gate_append::APPEND_ONLY.contains(&f.as_str()) && resolve_append_only(tree, f))
    });
    left.retain(|f| !(canonical_file(f) && resolve_canonical(tree, f)));
    left
}

/// Whether `f` is a game's model.toml or fidelity.toml (`GAME/model.toml`
/// in the games repo, `templates/GAME/model.toml` in the tool's), which
/// have one canonical text (`gp model fmt`).
pub(crate) fn canonical_file(f: &str) -> bool {
    let p: Vec<&str> = f.split('/').collect();
    let name = p.last().copied().unwrap_or("");
    (name == gp_model::MODEL_FILE || name == gp_contract::CONTRACT_FILE)
        && (p.len() == 2 || (p.len() == 3 && p[0] == "templates"))
}

/// The canonical text of a model.toml or fidelity.toml, or None when it
/// doesn't parse.
fn canonical_text(f: &str, text: &str) -> Option<String> {
    if f.ends_with(gp_model::MODEL_FILE) {
        gp_model::Model::parse(text).ok().map(|m| m.to_toml())
    } else {
        gp_contract::Contract::parse(text).ok().map(|c| c.to_toml())
    }
}

/// `text` written to the object store; its id.
fn blob(tree: &Path, text: &str) -> Option<String> {
    use std::io::Write as _;
    let mut c = Command::new("git")
        .arg("-C")
        .arg(tree)
        .args(["hash-object", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    c.stdin.take()?.write_all(text.as_bytes()).ok()?;
    let o = c.wait_with_output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// Merges a model.toml or fidelity.toml from its three stages through
/// their canonical texts, and stages the canonical result. A patch made on
/// another layout of the same model (a hand edit, or an older layout) then
/// merges by the facts it changes, one line each. False when a side doesn't
/// parse or the canonical texts conflict.
fn resolve_canonical(tree: &Path, f: &str) -> bool {
    let stage = |n: u8| git(tree, &["show", &format!(":{n}:{f}")]).ok();
    let (Some(base), Some(ours), Some(theirs)) = (stage(1), stage(2), stage(3)) else {
        return false;
    };
    let (Some(base), Some(ours), Some(theirs)) = (
        canonical_text(f, &base),
        canonical_text(f, &ours),
        canonical_text(f, &theirs),
    ) else {
        return false;
    };
    let (Some(b), Some(o), Some(t)) = (blob(tree, &base), blob(tree, &ours), blob(tree, &theirs))
    else {
        return false;
    };
    // merge-file exits with the number of conflicts: 0 is a clean merge.
    let Ok(out) = Command::new("git")
        .arg("-C")
        .arg(tree)
        .args(["merge-file", "-p", "--object-id", &o, &b, &t])
        .output()
    else {
        return false;
    };
    if out.status.code() != Some(0) {
        return false;
    }
    let Some(merged) = canonical_text(f, &String::from_utf8_lossy(&out.stdout)) else {
        return false;
    };
    fs::write(tree.join(f), merged).is_ok() && git(tree, &["add", "--", f]).is_ok()
}

/// Merges the append-only file `f` (docs/gate.md, "Append-only files") from
/// its three stages, keeping both sides' added rows in date order, and
/// stages it. False when a side changed or dropped a row the base has.
fn resolve_append_only(tree: &Path, f: &str) -> bool {
    let stages = git(tree, &["ls-files", "-u", "--", f]).unwrap_or_default();
    let id = |n: &str| {
        stages.lines().find_map(|l| {
            let mut w = l.split_whitespace();
            let (_, id, stage) = (w.next()?, w.next()?, w.next()?);
            (stage == n).then(|| id.to_owned())
        })
    };
    let (Some(base), Some(ours), Some(theirs)) = (id("1"), id("2"), id("3")) else {
        return false;
    };
    // merge-file exits with the number of conflicts, so its status says
    // nothing here; its output does.
    let Ok(o) = Command::new("git")
        .arg("-C")
        .arg(tree)
        .args([
            "merge-file",
            "-p",
            "--diff3",
            &format!("--marker-size={}", crate::gate_append::MARKER),
            "--object-id",
            &ours,
            &base,
            &theirs,
        ])
        .output()
    else {
        return false;
    };
    let Some(merged) = crate::gate_append::resolve(&String::from_utf8_lossy(&o.stdout)) else {
        return false;
    };
    fs::write(tree.join(f), merged).is_ok() && git(tree, &["add", "--", f]).is_ok()
}

/// The ROM-blocking hook over what is staged in `tree`, before any build,
/// so a patch with a ROM's bytes is refused while nothing is committed. A
/// tip's hook: the patch can't change it. Its package scan runs `gp`.
pub(crate) fn hook(tree: &Path, gp: &Path) -> std::result::Result<(), String> {
    let o = Command::new(tree.join(".githooks/pre-commit"))
        .current_dir(tree)
        .env("GP", gp)
        .output()
        .map_err(|e| format!("running the hook: {e}"))?;
    if o.status.success() {
        return Ok(());
    }
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr),
    );
    Err(format!("{err}{out}").trim().to_owned())
}

/// Commits what is staged. Commits only run the hook, never skip it.
pub(crate) fn commit(tree: &Path, message: &str, patch: &Patch, gp: &Path) -> Result<()> {
    let msg = patch.message.as_deref().unwrap_or(message);
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(tree)
        .args(["commit", "-q", "-m", msg])
        .env("GP", gp);
    if let Some(a) = &patch.author {
        cmd.arg("--author").arg(a);
    }
    if let Some(d) = &patch.date {
        cmd.arg("--date").arg(d);
    }
    let o = cmd.output()?;
    if !o.status.success() {
        bail!(
            "git commit: {}{}",
            String::from_utf8_lossy(&o.stdout).trim(),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(())
}

/// One patch of a gate's batch: its own, or one a waiting gate asked for
/// that this gate took into its batch (`gate_queue`).
pub(crate) struct Item {
    pub patch: Patch,
    /// The patch as its asker named it, for the lines.
    pub shown: String,
    pub message: Option<String>,
    /// The clips it removes on purpose (`--removes`).
    pub removes: Vec<String>,
    /// The files it changes, relative to the repo.
    pub files: Vec<String>,
}

/// How much of the gate a patch needs, by the files it changes
/// (docs/gate.md, "Lanes").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Lane {
    /// Files no score reads: Markdown. The hook, then it lands.
    Free,
    /// Presentation: model.toml (its `present` sections, told apart once
    /// applied) and themes/. The picture measure and the corpus.
    Picture,
    /// Anything that can change behaviour: the no-regression rule.
    Behaviour,
}

/// A patch's lane from its files, relative to the repo; `rel` is the
/// game's folder.
pub(crate) fn lane_of(files: &[String], rel: &str) -> Lane {
    let free = |f: &String| f.ends_with(".md");
    if files.iter().all(free) {
        Lane::Free
    } else if files.iter().all(|f| {
        free(f) || f == &format!("{rel}/model.toml") || f.starts_with(&format!("{rel}/themes/"))
    }) {
        Lane::Picture
    } else {
        Lane::Behaviour
    }
}

/// A state of the gate's tree that passed: the tip, or the tip with the
/// patches a batch has passed so far, each committed.
#[derive(Clone)]
struct State {
    commit: String,
    rows: Rows,
    /// The whole score of `rows`, which the next score reuses.
    json: PathBuf,
    /// The patches on top of the tip, by index into the batch, with their
    /// commits.
    commits: Vec<(usize, String)>,
    /// Whether a commit on top of the tip holds curves measured again.
    remeasured: bool,
}

/// How one try of a set of patches went.
enum Tried {
    Pass(State),
    /// The patch at this index fails on its own (doesn't apply, the hook
    /// refuses it, it removes a clip it doesn't name): it leaves the set.
    Item(usize, Failed),
    /// The set fails together; halving finds which patch.
    Fail(Failed),
}

/// A patch for people, in a batch's lines: "patch 2 (fix.patch): ".
fn who(items: &[Item], i: usize) -> String {
    if items.len() == 1 {
        String::new()
    } else {
        format!("patch {} ({}): ", i + 1, items[i].shown)
    }
}

/// `set`'s patches applied and committed on the clean tree in order, each
/// through the hook before its commit, so a ROM's bytes never become a
/// commit. Returns their commits and files, or the patch that fails.
fn apply_items(
    c: &Ctx,
    a: &GateArgs,
    items: &[Item],
    set: &[usize],
    lines: &mut Vec<String>,
) -> Result<Applied> {
    let tree = &c.tree;
    let rel = c.rel.to_string_lossy().into_owned();
    let gp = std::env::current_exe()?;
    let mut commits = Vec::new();
    let mut all = Vec::new();
    let refuse = |i: usize, why: String, phase: &'static str| Ok(Err((i, Failed { why, phase })));
    for &i in set {
        let item = &items[i];
        match apply_how(tree, &item.patch.diff, !a.no_rebase) {
            Ok(Some(how)) => lines.push(format!("{}{how}", who(items, i))),
            Ok(None) => {}
            Err(why) => return refuse(i, why, "patch"),
        }
        let files: Vec<String> = git(tree, &["diff", "--cached", "--name-only", "HEAD"])?
            .lines()
            .map(str::to_owned)
            .collect();
        if files.is_empty() {
            return refuse(
                i,
                "the patch changes nothing on the tip: it is there already".into(),
                "patch",
            );
        }
        if let Some(f) = files.iter().find(|f| !f.starts_with(&format!("{rel}/"))) {
            return refuse(
                i,
                format!("the patch changes {f}, outside the game"),
                "patch",
            );
        }
        if let Err(why) = hook(tree, &gp) {
            return refuse(
                i,
                format!("the ROM-blocking hook refuses it: {why}"),
                "hook",
            );
        }
        let removed = removed_clips(tree, &files, |f| {
            git(tree, &["show", &format!("HEAD:{f}")]).ok()
        });
        if let Some(why) = unnamed_removals(&removed, &item.removes) {
            return refuse(i, why, "removed clips");
        }
        let msg = item
            .message
            .clone()
            .unwrap_or_else(|| "A patch, by gp model gate".into());
        if let Err(e) = commit(tree, &msg, &item.patch, &gp) {
            return refuse(i, format!("{e:#}"), "commit");
        }
        commits.push((i, git(tree, &["rev-parse", "HEAD"])?));
        all.extend(files);
    }
    Ok(Ok((commits, all)))
}

/// Each patch's commit, by index into the batch, and the files they
/// change; or the patch that fails on its own.
type Applied = std::result::Result<(Vec<(usize, String)>, Vec<String>), (usize, Failed)>;

/// One try of a set of patches on a state.
type TrySet<'a> = dyn FnMut(&State, &[usize], &mut Vec<String>) -> Result<Tried> + 'a;

/// What a try reads of the tip's own checks.
struct AtTip<'a> {
    scan_before: &'a (BTreeMap<String, u64>, u8),
    rom: &'a [u8],
}

/// A try of patches no score reads: applied and committed through the
/// hook, and nothing else; the corpus keeps its rows.
fn free_trial(
    c: &Ctx,
    a: &GateArgs,
    items: &[Item],
    base: &State,
    set: &[usize],
    lines: &mut Vec<String>,
) -> Result<Tried> {
    checkout(&c.repo, &c.tree, &base.commit)?;
    let (commits, _) = match apply_items(c, a, items, set, lines)? {
        Ok(x) => x,
        Err((i, f)) => return Ok(Tried::Item(i, f)),
    };
    lines.push(format!(
        "{}: no score reads them; the hook passes",
        what_of(set)
    ));
    let mut all = base.commits.clone();
    all.extend(commits);
    Ok(Tried::Pass(State {
        commit: git(&c.tree, &["rev-parse", "HEAD"])?,
        commits: all,
        ..base.clone()
    }))
}

/// A try of a set of patches on `base`: each applied and committed through
/// the hook, then the game as they leave it: its module built, the package
/// scanned, the model checked and in canonical form, no play-only ROM named
/// as a source, its themes, its hook code's tests when they changed, and
/// the corpus scored against `base`'s rows.
fn trial(
    c: &Ctx,
    a: &GateArgs,
    items: &[Item],
    base: &State,
    set: &[usize],
    at: &AtTip,
    lines: &mut Vec<String>,
) -> Result<Tried> {
    let tree = &c.tree;
    let game = tree.join(&c.rel);
    let rel = c.rel.to_string_lossy().into_owned();
    let started = std::time::Instant::now();
    // No score to compare with: the patch adds the game, and its score is
    // the game's first baseline.
    let first = base.json.as_os_str().is_empty();
    checkout(&c.repo, tree, &base.commit)?;
    let base_model = fs::read_to_string(game.join(gp_model::MODEL_FILE)).unwrap_or_default();
    let (commits, files) = match apply_items(c, a, items, set, lines)? {
        Ok(x) => x,
        Err((i, f)) => return Ok(Tried::Item(i, f)),
    };
    let fail = |why: String, phase: &'static str| Ok(Tried::Fail(Failed { why, phase }));
    let module = match module_at(c, &game, lines) {
        Ok(m) => m,
        Err(e) => return fail(format!("{e:#}"), "module"),
    };
    let wasm: Vec<PathBuf> = module.iter().cloned().collect();
    let after = crate::package::scan::run(&game, &[], &wasm, &c.paths)?;
    let now = rewrites(&after.json);
    let new: Vec<String> = now
        .iter()
        .filter(|(f, n)| at.scan_before.0.get(*f).is_none_or(|b| *b < **n))
        .map(|(f, n)| format!("{f} ({n})"))
        .collect();
    if !new.is_empty() || (after.exit != 0 && at.scan_before.1 == 0) {
        let found: Vec<&str> = after
            .text
            .lines()
            .filter(|l| l.contains("NEEDS REWRITE"))
            .collect();
        return fail(
            format!(
                "gp package scan: new needs-rewrite items: {}{}",
                new.join("; "),
                found.join("; ")
            ),
            "package scan",
        );
    }
    let assets = assets_for(c, &game, at.rom)?;
    let check = match crate::model::check(&game, Some(&assets)) {
        Ok(c) => c,
        Err(e) => return fail(format!("gp model check: {e:#}"), "model check"),
    };
    if check.exit != 0 {
        return fail(
            format!("gp model check: {}", check.text.trim()),
            "model check",
        );
    }
    // The models guard: model.toml and fidelity.toml in canonical form.
    match crate::model::fmt_check(&game) {
        Ok(o) if o.exit == 0 => {}
        Ok(o) => return fail(format!("gp model fmt --check: {}", o.text.trim()), "models"),
        Err(e) => return fail(format!("gp model fmt --check: {e:#}"), "models"),
    }
    let loads = loads_problems(&game);
    if !loads.is_empty() {
        return fail(loads.join("; "), "loads");
    }
    let themes = theme_problems(&game);
    if !themes.is_empty() {
        return fail(
            format!("the game's themes: {}", themes.join("; ")),
            "themes",
        );
    }
    // The data guard, for a game: its hook code's own tests, when it changed.
    if let Ok(Some(_)) = crate::package::hook_code(&game)
        && let Some(krate) = hook_crate(&game)
        && files
            .iter()
            .any(|f| f.starts_with(&format!("{rel}/{krate}/")) || f == &format!("{rel}/Cargo.lock"))
    {
        match crate::module::test_hooks(&game, &c.kit) {
            Ok(o) if o.exit == 0 => lines.push(o.text),
            Ok(o) => return fail(o.text, "hook tests"),
            Err(e) => return fail(format!("{e:#}"), "hook tests"),
        }
    }
    let rows = match score(
        c,
        &game,
        module.as_deref(),
        &assets,
        (!first).then_some(base.json.as_path()),
        true,
        "patched",
    ) {
        Ok(x) => x,
        Err(e) => return fail(format!("gp corpus score: {e:#}"), "corpus"),
    };
    let (rows, json) = rows;
    let text_now = fs::read_to_string(game.join(gp_model::MODEL_FILE)).unwrap_or_default();
    let parts = changed_parts(&base_model, &text_now);
    // A patch to the model's presentation alone is held to the picture too.
    let picture = files
        .iter()
        .all(|f| f == &format!("{rel}/model.toml") || f.starts_with(&format!("{rel}/themes/")))
        && parts.as_ref().is_some_and(|p| {
            p.iter()
                .all(|x| x == "present" || x.starts_with("present."))
        });
    let baseline = if first {
        match first_baseline(&rows) {
            Ok(line) => Some(line),
            Err(w) => return fail(w, "corpus"),
        }
    } else {
        None
    };
    if picture && let Some(w) = picture_worse(&base.rows, &rows) {
        return fail(w, "picture");
    }
    if let Some(w) = worse(&base.rows, &rows) {
        return fail(w, "corpus");
    }
    // The curves measured again under a mapping the patch changed: the
    // corpus's new curves, version and record land with it, in a commit of
    // their own.
    let remeasured = !git(tree, &["status", "--porcelain", "--", &rel])?.is_empty();
    if remeasured {
        let corpus = fs::read_to_string(game.join("corpus.toml")).unwrap_or_default();
        let version = corpus
            .lines()
            .find_map(|l| l.trim().strip_prefix("version = "))
            .unwrap_or("")
            .trim_matches('"');
        let msg = format!(
            "{rel} corpus {version}: the curves measured again under the mapping as it now is (gp model gate)"
        );
        git(tree, &["add", "-A", "--", &rel])?;
        let plain = Patch {
            diff: PathBuf::new(),
            message: None,
            author: None,
            date: None,
        };
        commit(tree, &msg, &plain, &std::env::current_exe()?)?;
        lines.push(msg);
    }
    // Kept under its commit, since the next try reuses it as its base.
    let commit_now = git(tree, &["rev-parse", "HEAD"])?;
    let kept = c
        .home
        .join("scores")
        .join(format!("state-{}-{}.json", &commit_now[..12], c.run));
    fs::copy(&json, &kept)?;
    match baseline {
        Some(line) => {
            lines.push(format!(
                "{}: model check valid, package scan clean, every clip makes, in {} s",
                what_of(set),
                started.elapsed().as_secs()
            ));
            lines.push(format!("{rel}'s {line}, recorded at {}", kept.display()));
        }
        None => lines.push(format!(
            "{}: model check valid, package scan clean, corpus no worse ({} clips held) in {} s",
            what_of(set),
            rows.len(),
            started.elapsed().as_secs()
        )),
    }
    let mut all = base.commits.clone();
    all.extend(commits);
    Ok(Tried::Pass(State {
        commit: commit_now,
        rows,
        json: kept,
        commits: all,
        remeasured: base.remeasured || remeasured,
    }))
}

/// The hook crate's folder a package names (`[hooks] crate`).
fn hook_crate(game: &Path) -> Option<String> {
    let text = fs::read_to_string(game.join("package.toml")).ok()?;
    let t: toml::Table = toml::from_str(&text).ok()?;
    Some(t.get("hooks")?.get("crate")?.as_str()?.to_owned())
}

/// The loads guard: a ROM the package loads only to play (a `[[rom.entry]]`
/// with a role other than `scored`, or a template.toml `[[loads]]`) named
/// as a source the corpus is scored on: template.toml's `[source]` id or
/// `accepts`, or corpus.toml's `[target]` game or rom. No tool may score,
/// gate or derive from one (the old scripts/check/ju_rom.py).
pub(crate) fn loads_problems(game: &Path) -> Vec<String> {
    let read = |f: &str| -> toml::Table {
        fs::read_to_string(game.join(f))
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    };
    let (package, template, corpus) = (
        read("package.toml"),
        read("template.toml"),
        read("corpus.toml"),
    );
    let s = |v: Option<&toml::Value>| v.and_then(toml::Value::as_str).map(str::to_lowercase);
    let list = |v: Option<&toml::Value>| -> Vec<String> {
        v.and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(str::to_lowercase))
            .collect()
    };
    let mut play: BTreeSet<String> = BTreeSet::new();
    let entries = package
        .get("rom")
        .and_then(|r| r.get("entry"))
        .and_then(toml::Value::as_array);
    for e in entries.into_iter().flatten() {
        if e.get("role").and_then(toml::Value::as_str) != Some("scored") {
            play.extend(s(e.get("id")));
            play.extend(s(e.get("sha256")));
        }
    }
    for l in template
        .get("loads")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        play.extend(s(l.get("source")));
        play.extend(list(l.get("accepts")));
    }
    let mut named = Vec::new();
    if let Some(src) = template.get("source") {
        named.extend(s(src.get("id")).map(|v| ("template.toml [source] id", v)));
        named.extend(
            list(src.get("accepts"))
                .into_iter()
                .map(|v| ("template.toml [source] accepts", v)),
        );
    }
    if let Some(t) = corpus.get("target") {
        named.extend(s(t.get("game")).map(|v| ("corpus.toml [target] game", v)));
        named.extend(s(t.get("rom")).map(|v| ("corpus.toml [target] rom", v)));
    }
    named
        .into_iter()
        .filter(|(_, v)| play.contains(v))
        .map(|(w, v)| format!("{w} names {v}, a ROM the package loads only to play"))
        .collect()
}

/// Patches for people: "patch 4", "patches 1-3".
fn what_of(set: &[usize]) -> String {
    let span = match set {
        [a] => (a + 1).to_string(),
        [a, .., b] => format!("{}-{}", a + 1, b + 1),
        [] => String::new(),
    };
    if set.len() == 1 {
        format!("patch {span}")
    } else {
        format!("patches {span}")
    }
}

/// Settles `set` on `base`: tried together, and when the set fails, split
/// in halves, the first settled, then the second on what the first left
/// passing. A first half that all passed means the failure is in the
/// second, so its verdict is carried there rather than tried again. Each
/// patch's verdict goes into `verdicts`; returns the state the passing
/// patches reach.
fn settle(
    base: &State,
    set: &[usize],
    known: Option<Failed>,
    verdicts: &mut [Option<std::result::Result<(), Failed>>],
    lines: &mut Vec<String>,
    try_set: &mut TrySet,
) -> Result<State> {
    if set.is_empty() {
        return Ok(base.clone());
    }
    let why = match known {
        Some(w) => w,
        None => match try_set(base, set, lines)? {
            Tried::Pass(s) => {
                for &i in set {
                    verdicts[i] = Some(Ok(()));
                }
                return Ok(s);
            }
            Tried::Item(i, f) => {
                lines.push(format!("{}FAIL ({}): {}", what_of(&[i]), f.phase, f.why));
                verdicts[i] = Some(Err(f));
                let rest: Vec<usize> = set.iter().copied().filter(|k| *k != i).collect();
                return settle(base, &rest, None, verdicts, lines, try_set);
            }
            Tried::Fail(f) => f,
        },
    };
    if let [only] = set {
        verdicts[*only] = Some(Err(why));
        return Ok(base.clone());
    }
    let (first, second) = set.split_at(set.len() / 2);
    lines.push(format!(
        "split: {} apart from {} ({}: {})",
        what_of(first),
        what_of(second),
        why.phase,
        why.why.lines().next().unwrap_or("")
    ));
    let mid = settle(base, first, None, verdicts, lines, try_set)?;
    let whole = first.iter().all(|i| matches!(verdicts[*i], Some(Ok(()))));
    settle(&mid, second, whole.then_some(why), verdicts, lines, try_set)
}

/// The gate's batch on the tip of the branch: each patch first alone on
/// the tip (applies, the hook), then the Markdown-only ones on their own
/// checks, then the rest on top of them, settled by halves. Lands what
/// passed, all at once. A tip that moves before the landing runs the batch
/// again on the new tip. One outcome a patch, in order.
fn gate_batch(c: &Ctx, a: &GateArgs, items: &[Item], rom: &[u8]) -> Result<Vec<Outcome>> {
    let gp = std::env::current_exe()?;
    let rel = c.rel.to_string_lossy().into_owned();
    let n = items.len();
    let mut said: Vec<String> = Vec::new();
    for attempt in 1..=TIP_TRIES {
        let tip = git(&c.repo, &["rev-parse", &c.onto])?;
        let mut lines = vec![if n == 1 {
            format!("gate: {} onto {} at {}", items[0].shown, c.onto, &tip[..12])
        } else {
            format!(
                "gate: a batch of {n} patches onto {} at {}: {}",
                c.onto,
                &tip[..12],
                items
                    .iter()
                    .map(|i| i.shown.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }];
        let mut verdicts: Vec<Option<std::result::Result<(), Failed>>> = vec![None; n];
        // Before any scoring: a patch that doesn't apply on the tip, or
        // that the hook refuses, stops here in seconds.
        for (i, item) in items.iter().enumerate() {
            checkout(&c.repo, &c.tree, &tip)?;
            let early = apply_how(&c.tree, &item.patch.diff, !a.no_rebase)
                .map_err(|why| Failed {
                    why,
                    phase: "patch",
                })
                .and_then(|_| {
                    hook(&c.tree, &gp).map_err(|why| Failed {
                        why: format!("the ROM-blocking hook refuses it: {why}"),
                        phase: "hook",
                    })
                });
            clean(&c.tree);
            if let Err(f) = early {
                lines.push(format!("{}FAIL ({}): {}", who(items, i), f.phase, f.why));
                verdicts[i] = Some(Err(f));
            }
        }
        let live: Vec<usize> = (0..n).filter(|i| verdicts[*i].is_none()).collect();
        let (free, scored): (Vec<usize>, Vec<usize>) = live
            .iter()
            .partition(|i| lane_of(&items[**i].files, &rel) == Lane::Free);
        let start = State {
            commit: tip.clone(),
            rows: Rows::new(),
            json: PathBuf::new(),
            commits: Vec::new(),
            remeasured: false,
        };
        let mut free_try =
            |b: &State, set: &[usize], l: &mut Vec<String>| free_trial(c, a, items, b, set, l);
        let mid = settle(
            &start,
            &free,
            None,
            &mut verdicts,
            &mut lines,
            &mut free_try,
        )?;
        let mut tip_json = None;
        let mut first = false;
        // The tip's rows, for the status file a landing writes.
        let mut tip_rows = Rows::new();
        let end = if scored.is_empty() {
            mid
        } else {
            // The tip scored, with what the free patches add (no score
            // reads it), as the base of the rest.
            checkout(&c.repo, &c.tree, &mid.commit)?;
            let started = std::time::Instant::now();
            let tree_game = c.tree.join(&c.rel);
            let (base, scan_before) = if !tree_game.join("package.toml").is_file() {
                // A new game: the tip has no score and no scan to keep to,
                // so the patch's score is the first baseline (`trial`).
                first = true;
                lines.push(format!(
                    "tip {}: no {rel} yet; the patch adds it, so its score is the game's first baseline",
                    &tip[..8]
                ));
                (mid, (BTreeMap::new(), 0))
            } else {
                let module = match module_at(c, &tree_game, &mut lines) {
                    Ok(m) => m,
                    Err(e) => bail!("the tip's module doesn't build: {e:#}"),
                };
                let assets = assets_for(c, &tree_game, rom)?;
                let (rows, json) = score(
                    c,
                    &tree_game,
                    module.as_deref(),
                    &assets,
                    None,
                    false,
                    "tip",
                )?;
                lines.push(format!(
                    "tip {}: {} clips scored in {} s",
                    &tip[..8],
                    rows.len(),
                    started.elapsed().as_secs()
                ));
                let wasm: Vec<PathBuf> = module.iter().cloned().collect();
                let scan = crate::package::scan::run(&tree_game, &[], &wasm, &c.paths)?;
                tip_json = Some(json.clone());
                tip_rows = rows.clone();
                (
                    State { rows, json, ..mid },
                    (rewrites(&scan.json), scan.exit),
                )
            };
            let at = AtTip {
                scan_before: &scan_before,
                rom,
            };
            let mut try_set =
                |b: &State, set: &[usize], l: &mut Vec<String>| trial(c, a, items, b, set, &at, l);
            settle(
                &base,
                &scored,
                None,
                &mut verdicts,
                &mut lines,
                &mut try_set,
            )?
        };
        clean(&c.tree);
        let landed: std::result::Result<(), (String, bool)> = if a.dry_run || end.commits.is_empty()
        {
            Ok(())
        } else {
            land(&c.repo, &c.onto, &tip, &end.commit)
        };
        match &landed {
            Err((why, true)) if attempt < TIP_TRIES => {
                said.extend(lines);
                said.push(format!("the tip moved: {why}; again (try {})", attempt + 1));
                continue;
            }
            Err((why, false)) => bail!("couldn't land {}: {why}", end.commit),
            _ => {}
        }
        if landed.is_ok()
            && !a.dry_run
            && end.remeasured
            && let Some(dir) = &c.copy
        {
            crate::gate_copy::write_back(&c.repo, &end.commit, dir)?;
            lines.push(format!(
                "the curves measured again are written to {}",
                dir.display()
            ));
        }
        if landed.is_ok() && !a.dry_run && !end.commits.is_empty() && !tip_rows.is_empty() {
            lines.push(status_line(c, &end.commit, &tip_rows, &end.rows));
        }
        said.extend(lines);
        let outcomes = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let mine = end.commits.iter().find(|(k, _)| *k == i).map(|(_, k)| k);
                let pre = if n == 1 {
                    String::new()
                } else {
                    format!("{}: ", item.shown)
                };
                let (line, json, exit) = match (&verdicts[i], mine, &landed) {
                    (Some(Err(f)), _, _) => (
                        format!("{pre}FAIL ({}): {}", f.phase, f.why),
                        json!({"verdict": "FAIL", "phase": f.phase, "why": f.why}),
                        1,
                    ),
                    (_, Some(k), _) if a.dry_run => (
                        format!("{pre}--dry-run: PASS, nothing lands"),
                        json!({"verdict": "PASS", "commit": k}),
                        0,
                    ),
                    (_, Some(k), Ok(())) => (
                        format!("{pre}PASS: landed {k} on {}", c.onto),
                        json!({"verdict": "PASS", "commit": k, "landed": end.commit}),
                        0,
                    ),
                    (_, Some(k), Err((why, _))) => (
                        format!("{pre}PASS, but not landed: {why}. Gate it again on the new tip"),
                        json!({"verdict": "PASS", "commit": k, "landed": false}),
                        2,
                    ),
                    (_, None, _) => (
                        format!("{pre}FAIL (gate): no verdict"),
                        json!({"verdict": "FAIL", "phase": "gate", "why": "no verdict"}),
                        1,
                    ),
                };
                let mut json = json;
                json["tip"] = json!(tip);
                json["tip_score"] = json!(tip_json);
                if first && mine.is_some() {
                    json["first_baseline"] = json!(end.json);
                }
                json["batch"] = json!(n);
                let mut all = said.clone();
                all.push(line);
                json["lines"] = json!(all);
                Outcome {
                    json,
                    text: all.join("\n"),
                    exit,
                }
            })
            .collect();
        return Ok(outcomes);
    }
    unreachable!("the last try returns")
}

/// The branch to land on: `--onto`, else the one checked out in `repo`. A
/// detached checkout has none, so it must name one (a score-only run lands
/// nothing and doesn't care).
pub(crate) fn onto_branch(onto: Option<&str>, repo: &Path, any: bool) -> Result<String> {
    if let Some(b) = onto {
        return Ok(b.to_owned());
    }
    let b = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if b == "HEAD" && !any {
        bail!("this checkout is detached, so name the branch to land on with --onto");
    }
    Ok(b)
}

/// The gate's home, from `--home`, $GP_GATE_HOME, else ~/.gameprinter/gate,
/// made if missing and held outside any git work tree.
pub(crate) fn gate_home(home: Option<&Path>) -> Result<PathBuf> {
    let h = match home {
        Some(h) => h.to_owned(),
        None => match std::env::var_os("GP_GATE_HOME") {
            Some(h) => PathBuf::from(h),
            None => dirs_home()?.join(".gameprinter").join("gate"),
        },
    };
    let h = std::path::absolute(&h)?;
    fs::create_dir_all(&h)?;
    if let Some(r) = gp_evidence::paths::enclosing_git_work_tree(&h) {
        bail!(
            "the gate's home is inside the git work tree at {}",
            r.display()
        );
    }
    Ok(h)
}

pub(crate) fn run(a: &GateArgs, paths: &Paths) -> Result<Outcome> {
    // A new game's folder may not be here yet: the patch creates it.
    let game = gp_evidence::paths::real_path(&a.game);
    let home = gate_home(a.home.as_deref())?;
    let gp = std::env::current_exe()?;
    let private = gp_evidence::paths::enclosing_git_work_tree(&game).is_none();
    if !game.is_dir() && (private || a.score_only) {
        bail!("{}: No such folder", a.game.display());
    }
    let near = game
        .ancestors()
        .find(|d| d.is_dir())
        .context("no folder above the game")?
        .to_owned();
    // One gate at a time on a home: they share the worktree. A gate that
    // may share a batch takes a ticket first, so the gate holding the lock
    // can take its patch in (gate_queue).
    let lock = fs::File::create(home.join("gate.lock"))?;
    let mut copy = None;
    let mut ticket = None;
    // An --import's commit message, unless -m gives one.
    let mut import_msg = None;
    let (repo, rel, onto, tree, patch_file) = if private {
        if a.patch.is_some() {
            bail!(
                "{} has no git of its own: the gate takes your edits from the folder, so leave out --patch",
                game.display()
            );
        }
        lock.lock()?;
        match crate::gate_copy::open(&game, &home, &gp, a.message.as_deref())? {
            crate::gate_copy::Opened::Done(outcome) => return Ok(outcome),
            crate::gate_copy::Opened::Gate { repo, tree, diff } => {
                copy = Some(game.clone());
                (repo, PathBuf::from("game"), "main".to_owned(), tree, diff)
            }
        }
    } else {
        let repo = PathBuf::from(git(&near, &["rev-parse", "--show-toplevel"])?);
        let rel = game.strip_prefix(&repo)?.to_owned();
        let onto = onto_branch(a.onto.as_deref(), &repo, a.score_only)?;
        // A score-only run reads the checkout as it stands: it's made by
        // the tool gate for this one job and never changes.
        let tree = if a.score_only {
            repo.clone()
        } else {
            home.join("tree")
        };
        let file = if a.score_only {
            PathBuf::new()
        } else if a.import {
            match import_patch(&repo, &rel, &onto, &home, paths, &a.removes)? {
                Imported::Nothing(o) => return Ok(o),
                Imported::Patch(f, msg) => {
                    import_msg = Some(msg);
                    f
                }
            }
        } else {
            a.patch.clone().context(
                "--patch is needed: give the diff to gate, or point the gate at a working copy with no git",
            )?
        };
        if !a.score_only && !a.dry_run && !a.no_rebase {
            let common = git(
                &repo,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            )?;
            let req = crate::gate_queue::Request {
                patch: PathBuf::new(),
                shown: file.display().to_string(),
                message: a.message.clone().or(import_msg.clone()),
                repo: PathBuf::from(common),
                game: rel.to_string_lossy().into_owned(),
                onto: onto.clone(),
                run: a.run.clone(),
                removes: a.removes.clone(),
            };
            ticket = Some(crate::gate_queue::Ticket::enter(&home, req, &file)?);
        }
        lock.lock()?;
        if let Some(t) = &ticket
            && let Some(o) = t.result()
        {
            return Ok(o);
        }
        (repo, rel, onto, tree, file)
    };

    let kit = home.join("kit");
    let o = self_cmd(&home)?
        .args(["--json", "game", "kit", "-o"])
        .arg(&kit)
        .output()?;
    if !o.status.success() {
        bail!("gp game kit: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    let kit_hash = serde_json::from_slice::<Value>(&o.stdout)
        .context("gp game kit gave no JSON")?["hash"]
        .as_str()
        .context("gp game kit gave no hash")?
        .to_owned();
    let c = Ctx {
        repo,
        rel,
        copy,
        tree,
        paths: Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: paths.roms.clone(),
        },
        home,
        onto,
        run: a.run.clone(),
        kit,
        kit_hash,
    };
    let rom = if c.copy.is_none() && !a.score_only && !on_tip(&c.repo, &c.onto, &c.rel) {
        match new_game_rom(&c, a, &patch_file)? {
            Ok(rom) => rom,
            Err(why) => {
                return Ok(Outcome {
                    json: json!({"verdict": "FAIL", "phase": "patch", "why": why}),
                    text: format!("FAIL (patch): {why}"),
                    exit: 1,
                });
            }
        }
    } else {
        rom_of(&c.paths, &game, None)?
    };
    if a.score_only {
        return score_only(&c, &rom);
    }

    // The batch: this gate's patch, then the waiting ones it takes in.
    let claimed = ticket.as_ref().map(|t| t.claim()).unwrap_or_default();
    let mut items = Vec::new();
    let mut asked: Vec<Option<String>> = Vec::new();
    let mut early: Vec<(Option<String>, Outcome)> = Vec::new();
    let own = (
        None,
        patch_file.clone(),
        patch_file.display().to_string(),
        a.message.clone().or(import_msg),
        a.removes.clone(),
    );
    let others = claimed
        .into_iter()
        .map(|(name, r)| (Some(name), r.patch, r.shown, r.message, r.removes));
    for (k, (name, file, shown, message, removes)) in std::iter::once(own).chain(others).enumerate()
    {
        let work = c.home.join("batch").join(k.to_string());
        let patch = match read_patch(&file, &work)? {
            Ok(p) => p,
            Err(why) => {
                early.push((
                    name,
                    Outcome {
                        json: json!({"verdict": "FAIL", "phase": "patch", "why": why}),
                        text: format!("FAIL (patch): {why}"),
                        exit: 1,
                    },
                ));
                continue;
            }
        };
        let files: Vec<String> = git(
            &c.repo,
            &["apply", "--numstat", &patch.diff.to_string_lossy()],
        )
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split('\t').nth(2).map(str::to_owned))
        .collect();
        items.push(Item {
            patch,
            shown,
            message,
            removes,
            files,
        });
        asked.push(name);
    }
    let outcomes = if items.is_empty() {
        Vec::new()
    } else {
        gate_batch(&c, a, &items, &rom)?
    };
    let mut mine = None;
    for (name, o) in asked.into_iter().zip(outcomes).chain(early) {
        match (&name, &ticket) {
            (Some(n), Some(t)) => t.answer(n, &o),
            _ => mine = Some(o),
        }
    }
    mine.context("the gate reached no outcome for its own patch")
}

/// Whether the tip of `onto` has the game in `rel` (its package.toml).
fn on_tip(repo: &Path, onto: &str, rel: &Path) -> bool {
    git(
        repo,
        &[
            "cat-file",
            "-e",
            &format!("{onto}:{}/package.toml", rel.to_string_lossy()),
        ],
    )
    .is_ok()
}

/// Why a patch can't add the game in `rel`, the tip having none: it must
/// create the game's package.toml. `files` are the patch's, relative to the
/// repo; that they are all in the game's folder is checked as for any patch.
pub(crate) fn new_game_problem(files: &[String], rel: &str) -> Option<String> {
    let package = format!("{rel}/package.toml");
    (!files.contains(&package)).then(|| {
        format!(
            "{rel} isn't on the tip, and the patch doesn't create {package}: a patch to a game that isn't there yet must add its package"
        )
    })
}

/// A new game's ROM, read from its package.toml as the patch writes it on
/// the tip; `Err` says in words why the patch can't add the game.
fn new_game_rom(
    c: &Ctx,
    a: &GateArgs,
    file: &Path,
) -> Result<std::result::Result<Vec<u8>, String>> {
    let patch = match read_patch(file, &c.home.join("batch").join("new"))? {
        Ok(p) => p,
        Err(why) => return Ok(Err(why)),
    };
    let files: Vec<String> = git(
        &c.repo,
        &["apply", "--numstat", &patch.diff.to_string_lossy()],
    )
    .unwrap_or_default()
    .lines()
    .filter_map(|l| l.split('\t').nth(2).map(str::to_owned))
    .collect();
    if let Some(why) = new_game_problem(&files, &c.rel.to_string_lossy()) {
        return Ok(Err(why));
    }
    let tip = git(&c.repo, &["rev-parse", &c.onto])?;
    checkout(&c.repo, &c.tree, &tip)?;
    let rom = match apply_how(&c.tree, &patch.diff, !a.no_rebase) {
        Ok(_) => rom_of(&c.paths, &c.tree.join(&c.rel), None),
        Err(why) => {
            clean(&c.tree);
            return Ok(Err(why));
        }
    };
    clean(&c.tree);
    rom.map(Ok)
}

/// A new game's first score, held to the one rule a game with no score on
/// the tip has: every clip makes, whatever its score. The line that gives it
/// as the game's first baseline, or the clips that don't make.
pub(crate) fn first_baseline(rows: &Rows) -> std::result::Result<String, String> {
    if rows.is_empty() {
        return Err("the corpus has no clips, so there is no first baseline".into());
    }
    let broken: Vec<String> = rows
        .iter()
        .filter_map(|(id, r)| r.error.as_ref().map(|e| format!("{id}: {e}")))
        .collect();
    if !broken.is_empty() {
        return Err(format!(
            "{} of {} clips don't make: {}",
            broken.len(),
            rows.len(),
            broken.join("; ")
        ));
    }
    let full = rows.iter().filter(|(_, r)| r.full()).count();
    let together: u64 = rows.iter().map(|(_, r)| r.together).sum();
    let frames: u64 = rows.iter().map(|(_, r)| r.frames).sum();
    Ok(format!(
        "first baseline: {} clips, {full} at 100%, {together}/{frames} frames together",
        rows.len()
    ))
}

/// `--score-only`: the tip scored, and nothing lands. The tool gate's
/// canary runs this through the patched gp and compares the rows.
fn score_only(c: &Ctx, rom: &[u8]) -> Result<Outcome> {
    let tip = git(&c.repo, &["rev-parse", &c.onto])?;
    let started = std::time::Instant::now();
    let tree_game = c.tree.join(&c.rel);
    let mut lines = Vec::new();
    let module = match module_at(c, &tree_game, &mut lines) {
        Ok(m) => m,
        Err(e) => bail!("the tip's module doesn't build: {e:#}"),
    };
    let assets = assets_for(c, &tree_game, rom)?;
    let (tip_rows, tip_json) = score(
        c,
        &tree_game,
        module.as_deref(),
        &assets,
        None,
        false,
        "tip",
    )?;
    lines.push(format!(
        "tip {}: {} clips scored in {} s",
        &tip[..8],
        tip_rows.len(),
        started.elapsed().as_secs()
    ));
    let clips: Vec<Value> = tip_rows.iter().map(|(_, r)| r.raw.clone()).collect();
    Ok(Outcome::ok(
        json!({"verdict": "SCORED", "tip": tip, "tip_score": tip_json, "clips": clips}),
        lines.join("\n"),
    ))
}

fn dirs_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("no home directory; give --home")
}

/// A corpus score's rows by clip, in the corpus's order.
pub(crate) type Rows = Vec<(String, Row)>;

#[derive(Clone, Default)]
pub(crate) struct Row {
    frames: u64,
    together: u64,
    error: Option<String>,
    bar_together: Option<bool>,
    bar_longest: Option<bool>,
    longest: Option<u64>,
    first: Option<(u64, u64, String)>,
    /// The row as gp corpus score gave it, for the status file.
    raw: Value,
}

impl Row {
    pub(crate) fn of(v: &Value) -> Self {
        let bar = |k: &str| v["bars"][k].as_bool();
        Self {
            frames: v["frames"].as_u64().unwrap_or(0),
            together: v["frames_together"].as_u64().unwrap_or(0),
            error: v["error"].as_str().map(str::to_owned),
            bar_together: bar("together"),
            bar_longest: bar("longest_stretch"),
            longest: v["stretches"]["longest_any"].as_u64(),
            first: v["first"].as_object().map(|f| {
                (
                    f.get("frame").and_then(Value::as_u64).unwrap_or(0),
                    f.get("original_frame").and_then(Value::as_u64).unwrap_or(0),
                    f.get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                )
            }),
            raw: v.clone(),
        }
    }

    fn full(&self) -> bool {
        self.error.is_none() && self.frames > 0 && self.together == self.frames
    }
}

fn changed_parts(a: &str, b: &str) -> Option<Vec<String>> {
    let a: toml::Value = toml::from_str(a).ok()?;
    let b: toml::Value = toml::from_str(b).ok()?;
    let mut out = Vec::new();
    fn walk(
        a: Option<&toml::Value>,
        b: Option<&toml::Value>,
        at: &str,
        depth: usize,
        out: &mut Vec<String>,
    ) {
        if a == b {
            return;
        }
        match (a, b) {
            (Some(toml::Value::Table(x)), Some(toml::Value::Table(y))) if depth < 3 => {
                let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
                for k in keys {
                    let p = if at.is_empty() {
                        k.clone()
                    } else {
                        format!("{at}.{k}")
                    };
                    walk(x.get(k), y.get(k), &p, depth + 1, out);
                }
            }
            _ => out.push(at.to_owned()),
        }
    }
    walk(Some(&a), Some(&b), "", 0, &mut out);
    Some(out)
}

fn pairing_of(r: &Row) -> String {
    serde_json::from_value::<gp_interp::lockstep::Pairing>(r.raw["pairing"].clone())
        .map(|p| format!("; {}", p.says()))
        .unwrap_or_default()
}

pub(crate) fn worse(base: &Rows, new: &Rows) -> Option<String> {
    for (id, b) in base {
        let Some((_, n)) = new.iter().find(|(i, _)| i == id) else {
            continue;
        };
        let at = |r: &Row| match &r.first {
            Some((f, of, p)) => format!(
                ", first wrong at frame {f} of the clip (the original's {of}), {p}{}",
                pairing_of(r)
            ),
            None => String::new(),
        };
        if b.error.is_none()
            && let Some(e) = &n.error
        {
            return Some(format!("{id}: scored before, now can't be: {e}"));
        }
        if b.full() && !n.full() && n.raw["stopped"] == true {
            return Some(format!(
                "{id}: was 100% ({0}/{0} frames together), now splits{1}; its run stopped there",
                b.frames,
                at(n)
            ));
        }
        if b.full() && !n.full() {
            return Some(format!(
                "{id}: was 100% ({0}/{0} frames together), now {1}/{2}{3}",
                b.frames,
                n.together,
                n.frames,
                at(n)
            ));
        }
        if b.bar_together == Some(true) && n.bar_together == Some(false) {
            return Some(format!(
                "{id}: met the contract's together bar, now {}/{} frames{}",
                n.together,
                n.frames,
                at(n)
            ));
        }
        if b.bar_longest == Some(true) && n.bar_longest != Some(true) {
            return Some(format!(
                "{id}: met the contract's longest stretch ({} frames), now {} frames",
                b.longest.unwrap_or(0),
                n.longest.unwrap_or(0)
            ));
        }
    }
    None
}

fn picture_worse(base: &Rows, new: &Rows) -> Option<String> {
    for (id, b) in base {
        let Some((_, n)) = new.iter().find(|(i, _)| i == id) else {
            continue;
        };
        for layer in ["sprites", "background"] {
            let (bs, bt) = (
                &b.raw["screen"][layer]["same"],
                &b.raw["screen"][layer]["total"],
            );
            let (ns, nt) = (
                &n.raw["screen"][layer]["same"],
                &n.raw["screen"][layer]["total"],
            );
            if let (Some(bs), Some(ns)) = (bs.as_u64(), ns.as_u64())
                && bt == nt
                && ns < bs
            {
                return Some(format!(
                    "{id}: the picture's {layer} is the same on {ns} of {} frames, {bs} before",
                    nt.as_u64().unwrap_or(0)
                ));
            }
        }
    }
    None
}

/// The clip IDs a `corpus.toml` names: a plan's `[[clip]]` table, or an
/// older file's `[recipes]`.
pub(crate) fn recipe_ids(text: &str) -> BTreeSet<String> {
    let Ok(v) = toml::from_str::<toml::Value>(text) else {
        return BTreeSet::new();
    };
    if let Some(clips) = v.get("clip").and_then(|c| c.as_array()) {
        return clips
            .iter()
            .filter_map(|c| c.get("id")?.as_str().map(str::to_owned))
            .collect();
    }
    v.get("recipes")
        .and_then(|r| r.as_table())
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

pub(crate) fn matches(pattern: &str, id: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == id;
    }
    let mut rest = id;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(p) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            let Some(at) = rest.find(p) else {
                return false;
            };
            rest = &rest[at + p.len()..];
        }
    }
    true
}

pub(crate) fn removed_clips(
    tree: &Path,
    files: &[String],
    old: impl Fn(&str) -> Option<String>,
) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let corpora: BTreeSet<&String> = files
        .iter()
        .filter(|f| f.ends_with("/corpus.toml") || f.as_str() == "corpus.toml")
        .collect();
    for f in corpora {
        let Some(before) = old(f) else {
            continue;
        };
        let after = recipe_ids(&fs::read_to_string(tree.join(f)).unwrap_or_default());
        let gone: Vec<String> = recipe_ids(&before)
            .into_iter()
            .filter(|id| !after.contains(id))
            .collect();
        if !gone.is_empty() {
            out.push((f.clone(), gone));
        }
    }
    out
}

pub(crate) fn unnamed_removals(
    removed: &[(String, Vec<String>)],
    allowed: &[String],
) -> Option<String> {
    let mut each = Vec::new();
    let mut n = 0;
    for (file, ids) in removed {
        let left: Vec<&String> = ids
            .iter()
            .filter(|id| !allowed.iter().any(|p| matches(p, id)))
            .collect();
        if left.is_empty() {
            continue;
        }
        n += left.len();
        let shown: Vec<&str> = left.iter().take(8).map(|s| s.as_str()).collect();
        let more = if left.len() > 8 {
            format!(" and {} more", left.len() - 8)
        } else {
            String::new()
        };
        each.push(format!(
            "{} from {file}: {}{more}",
            left.len(),
            shown.join(", ")
        ));
    }
    if each.is_empty() {
        return None;
    }
    Some(format!(
        "the patch removes {n} clip{} from the corpus without naming {}: {}. No score compares a clip that is gone, so a patch that means to remove clips names them: `--removes ID` (repeated, or a pattern such as `dk.climb-sweep.*`)",
        if n == 1 { "" } else { "s" },
        if n == 1 { "it" } else { "them" },
        each.join("; ")
    ))
}

/// Moves the branch from `tip` to `commit`: a fast-forward where it is
/// checked out, so that checkout follows; else the ref, if still at `tip`.
pub(crate) fn land(
    repo: &Path,
    onto: &str,
    tip: &str,
    commit: &str,
) -> std::result::Result<(), (String, bool)> {
    let porcelain =
        git(repo, &["worktree", "list", "--porcelain"]).map_err(|e| (format!("{e:#}"), false))?;
    let mut at: Option<PathBuf> = None;
    let mut cur: Option<PathBuf> = None;
    for l in porcelain.lines() {
        if let Some(p) = l.strip_prefix("worktree ") {
            cur = Some(PathBuf::from(p));
        } else if l == format!("branch refs/heads/{onto}") {
            at = cur.clone();
        }
    }
    let landed = match &at {
        Some(w) => git(w, &["merge", "--ff-only", "-q", commit]),
        None => git(
            repo,
            &["update-ref", &format!("refs/heads/{onto}"), commit, tip],
        ),
    };
    landed.map(|_| ()).map_err(|e| {
        let now = git(repo, &["rev-parse", onto]).unwrap_or_default();
        (format!("{e:#}"), now != tip)
    })
}

/// What's wrong with a game's themes, its open version's too: each file
/// against the SHA-256 its `theme.toml` records, and the rest of the theme's
/// own check. The player refuses a theme that fails these.
fn theme_problems(game: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for themes in [game.join("themes"), game.join("open/themes")] {
        let Ok(entries) = fs::read_dir(&themes) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join(gp_theme::format::THEME_FILE).is_file())
            .collect();
        dirs.sort();
        for dir in dirs {
            match gp_theme::format::Theme::load(&dir) {
                Err(e) => problems.push(e),
                Ok(theme) => problems.extend(
                    theme
                        .check(&dir)
                        .into_iter()
                        .map(|(at, what)| format!("{}: {at}: {what}", dir.display())),
                ),
            }
        }
    }
    problems
}

#[cfg(test)]
mod theme_tests {
    use super::*;

    /// A game holding one theme with one art file, its pin right.
    fn game(at: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(at);
        fs::create_dir_all(&dir).unwrap();
        let art = b"pix 1\nsize 1 1\na #000000\na\n";
        fs::write(dir.join("a.pix"), art).unwrap();
        fs::write(
            dir.join(gp_theme::format::THEME_FILE),
            format!(
                "format = \"0.1.3\"\nid = \"paper\"\ntitle = \"Paper\"\nlicense = \"CC0-1.0\"\nfor = [\"m\"]\n\n[[restyle]]\nslot = \"s\"\nentry = \"e\"\nfile = \"a.pix\"\norigin = \"generated\"\nlicense = \"CC0-1.0\"\nsha256 = \"{}\"\n",
                gp_theme::format::sha256_hex(art)
            ),
        )
        .unwrap();
        d
    }

    #[test]
    fn a_theme_whose_files_match_their_pins_passes() {
        for at in ["themes/paper", "open/themes/paper"] {
            let d = game(at);
            assert_eq!(theme_problems(d.path()), Vec::<String>::new());
        }
    }

    #[test]
    fn a_file_changed_without_its_pin_is_named() {
        for at in ["themes/paper", "open/themes/paper"] {
            let d = game(at);
            fs::write(d.path().join(at).join("a.pix"), b"changed").unwrap();
            let found = theme_problems(d.path());
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].contains("a.pix has SHA-256"), "{found:?}");
        }
    }

    #[test]
    fn a_game_with_no_themes_has_no_problems() {
        let d = tempfile::tempdir().unwrap();
        assert!(theme_problems(d.path()).is_empty());
    }
}

/// The old gate's tests (game-printer's gate.rs), for what the game gate
/// and the tool gate keep of it: the no-regression rule and its failure
/// line, the picture rule, lanes, batches settled by halves, and three-way
/// merges with their rules for generated, append-only and canonical files.
#[cfg(test)]
mod tests {
    use super::*;

    fn row(frames: u64, together: u64) -> Row {
        Row {
            frames,
            together,
            ..Row::default()
        }
    }

    #[test]
    fn a_clip_that_leaves_100_or_loses_a_bar_fails_and_others_may_move() {
        let base: Rows = vec![("a".into(), row(10, 10)), ("b".into(), row(10, 5))];
        // b gets worse but was never at 100 and holds no bar: a pass.
        let ok: Rows = vec![("a".into(), row(10, 10)), ("b".into(), row(10, 3))];
        assert_eq!(worse(&base, &ok), None);
        let bad: Rows = vec![("a".into(), row(10, 9)), ("b".into(), row(10, 5))];
        assert!(worse(&base, &bad).unwrap().starts_with("a: was 100%"));
        let mut barred = base.clone();
        barred[1].1.bar_longest = Some(true);
        let mut lost = base.clone();
        lost[1].1.bar_longest = Some(false);
        assert!(worse(&barred, &lost).unwrap().contains("longest stretch"));
    }

    #[test]
    fn a_failure_line_gives_the_clips_frame_pairing() {
        let mut r = row(10, 9);
        r.first = Some((4, 1536, "entities.player[0].x".into()));
        assert_eq!(pairing_of(&r), "");
        r.raw = json!({"pairing": {"model": 1530, "original": 1533, "offset": 3,
            "lag": [], "last_offset": 3}});
        let base: Rows = vec![("a".into(), row(10, 10))];
        let w = worse(&base, &vec![("a".into(), r)]).unwrap();
        assert!(
            w.ends_with("the original's 1536), entities.player[0].x; model frame n is the original's n+3 (model 1530 = original 1533); no lag frames skipped"),
            "{w}"
        );
    }

    #[test]
    fn a_picture_is_worse_when_fewer_frames_are_the_same() {
        let with = |sprites: u64| {
            let mut r = row(10, 10);
            r.raw = json!({"screen": {"sprites": {"same": sprites, "total": 10},
                "background": {"same": 10, "total": 10}}});
            vec![("a".to_owned(), r)]
        };
        assert_eq!(picture_worse(&with(8), &with(9)), None);
        assert!(
            picture_worse(&with(8), &with(7))
                .unwrap()
                .contains("sprites")
        );
        // A clip with no picture measured on one side isn't compared.
        assert_eq!(
            picture_worse(&with(8), &vec![("a".into(), row(10, 10))]),
            None
        );
    }

    /// The old test read the quest log's session from a clip ID too; the
    /// gate writes no status file now (the mechanic page's quest log has
    /// its own), so only the changed parts are left.
    #[test]
    fn changed_parts_are_read_from_the_texts() {
        let a = "[entities.player.jump]\nland_hold = 4\n[entities.barrel]\nsize = [8, 8]\n";
        let b = "[entities.player.jump]\nland_hold = 5\n[entities.barrel]\nsize = [8, 8]\n";
        assert_eq!(
            changed_parts(a, b).unwrap(),
            vec!["entities.player.jump".to_owned()]
        );
    }

    #[test]
    fn a_patch_takes_the_lane_its_files_need() {
        let g = "donkey-kong";
        let lane = |fs: &[&str]| lane_of(&fs.iter().map(|f| f.to_string()).collect::<Vec<_>>(), g);
        assert_eq!(lane(&["donkey-kong/NOTES.md", "README.md"]), Lane::Free);
        assert_eq!(
            lane(&["donkey-kong/themes/kit/a.pix", "donkey-kong/x.md"]),
            Lane::Picture
        );
        assert_eq!(lane(&["donkey-kong/model.toml"]), Lane::Picture);
        assert_eq!(lane(&["donkey-kong/mapping.toml"]), Lane::Behaviour);
        assert_eq!(
            lane(&["donkey-kong/ext/src/lib.rs", "donkey-kong/x.md"]),
            Lane::Behaviour
        );
        assert_eq!(lane(&["other/model.toml"]), Lane::Behaviour);
    }

    /// A batch settled by a stand-in for the gate's try: patches in `bad`
    /// fail whenever they're in the tree, and a passing try commits its
    /// set. Returns each patch's verdict and the sets tried.
    fn settle_with(n: usize, bad: &[usize], unapplied: &[usize]) -> (Vec<bool>, Vec<Vec<usize>>) {
        let base = State {
            commit: "tip".into(),
            rows: Rows::new(),
            json: PathBuf::new(),
            commits: Vec::new(),
            remeasured: false,
        };
        let mut tried: Vec<Vec<usize>> = Vec::new();
        let mut try_set = |b: &State, set: &[usize], _: &mut Vec<String>| {
            tried.push(set.to_vec());
            if let Some(&i) = set.iter().find(|i| unapplied.contains(i)) {
                return Ok(Tried::Item(
                    i,
                    Failed {
                        why: "doesn't apply".into(),
                        phase: "patch",
                    },
                ));
            }
            if b.commits
                .iter()
                .map(|c| c.0)
                .chain(set.iter().copied())
                .any(|i| bad.contains(&i))
            {
                return Ok(Tried::Fail(Failed {
                    why: "worse".into(),
                    phase: "corpus",
                }));
            }
            let mut s = b.clone();
            s.commits.extend(set.iter().map(|&i| (i, format!("c{i}"))));
            Ok(Tried::Pass(s))
        };
        let mut verdicts = vec![None; n];
        let all: Vec<usize> = (0..n).collect();
        let end = settle(
            &base,
            &all,
            None,
            &mut verdicts,
            &mut Vec::new(),
            &mut try_set,
        )
        .unwrap();
        let passed: Vec<usize> = end.commits.iter().map(|c| c.0).collect();
        let pass: Vec<bool> = verdicts.iter().map(|v| matches!(v, Some(Ok(())))).collect();
        for (i, p) in pass.iter().enumerate() {
            assert_eq!(*p, passed.contains(&i), "patch {i}");
        }
        (pass, tried)
    }

    #[test]
    fn a_batch_lands_in_one_try_and_a_failure_is_found_by_halves() {
        // Six that pass: one try.
        let (pass, tried) = settle_with(6, &[], &[]);
        assert!(pass.iter().all(|p| *p));
        assert_eq!(tried, vec![vec![0, 1, 2, 3, 4, 5]]);
        // The fifth fails: the whole set, the first half, then the second
        // half is known to fail on top of it and isn't tried whole.
        let (pass, tried) = settle_with(6, &[4], &[]);
        assert_eq!(pass, [true, true, true, true, false, true]);
        assert_eq!(
            tried,
            vec![
                vec![0, 1, 2, 3, 4, 5],
                vec![0, 1, 2],
                vec![3],
                vec![4],
                vec![5]
            ]
        );
        // One that doesn't apply drops out without a split.
        let (pass, tried) = settle_with(3, &[], &[1]);
        assert_eq!(pass, [true, false, true]);
        assert_eq!(tried, vec![vec![0, 1, 2], vec![0, 2]]);
        // Alone, a failing patch is tried once.
        let (pass, tried) = settle_with(1, &[0], &[]);
        assert_eq!((pass, tried.len()), (vec![false], 1));
    }

    /// A repository with `model.toml` of ten lines, committed, and a
    /// plain diff that changes its second line.
    fn repo_with_patch() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let run = |args: &[&str]| git(d, args).unwrap();
        run(&["init", "-q"]);
        run(&["config", "user.email", "gate@test"]);
        run(&["config", "user.name", "gate test"]);
        run(&["config", "commit.gpgsign", "false"]);
        run(&["config", "core.hooksPath", "/dev/null"]);
        let lines: Vec<String> = (0..10).map(|i| format!("a{i} = {i}")).collect();
        fs::write(d.join("model.toml"), lines.join("\n") + "\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "base"]);
        let mut changed = lines.clone();
        changed[1] = "a1 = 100".into();
        fs::write(d.join("model.toml"), changed.join("\n") + "\n").unwrap();
        let diff = run(&["diff"]) + "\n";
        run(&["checkout", "-q", "--", "model.toml"]);
        let patch = d.join("fix.patch");
        fs::write(&patch, diff).unwrap();
        (t, patch)
    }

    /// A made-up game with no ROM, Moss Pond, that the tip of `games`
    /// doesn't have yet: a patch that creates its package adds it, and its
    /// first score passes when every clip makes; a patch that only touches
    /// the missing game fails in words.
    #[test]
    fn a_patch_may_add_a_game_the_tip_doesnt_have() {
        let (t, _) = repo_with_patch();
        let d = t.path();
        let g = |args: &[&str]| git(d, args).unwrap();
        let rel = Path::new("moss-pond");
        assert!(!on_tip(d, "HEAD", rel));
        fs::create_dir_all(d.join(rel)).unwrap();
        fs::write(
            d.join("moss-pond/package.toml"),
            "[package]\nid = \"moss-pond\"\n",
        )
        .unwrap();
        fs::write(d.join("moss-pond/corpus.toml"), "[[clip]]\nid = \"walk\"\n").unwrap();
        g(&["add", "-N", "moss-pond"]);
        let adds = d.join("adds.patch");
        fs::write(&adds, g(&["diff"]) + "\n").unwrap();
        g(&["rm", "-q", "-r", "--cached", "moss-pond"]);
        fs::remove_dir_all(d.join(rel)).unwrap();
        let files: Vec<String> = git(d, &["apply", "--numstat", &adds.to_string_lossy()])
            .unwrap()
            .lines()
            .filter_map(|l| l.split('\t').nth(2).map(str::to_owned))
            .collect();
        assert_eq!(new_game_problem(&files, "moss-pond"), None);
        assert_eq!(apply_how(d, &adds, true).unwrap(), None);
        g(&["commit", "-q", "-m", "Moss Pond"]);
        assert!(on_tip(d, "HEAD", rel));
        // Only a note in the game's folder: no package, so no game.
        let note = vec!["moss-pond/NOTES.md".to_owned()];
        let why = new_game_problem(&note, "moss-pond").unwrap();
        assert!(
            why.contains("doesn't create moss-pond/package.toml"),
            "{why}"
        );
        // The first score: every clip makes, whatever its score.
        let mut walk = row(100, 40);
        walk.raw = json!({"id": "walk"});
        let mut rows: Rows = vec![("walk".into(), walk), ("jump".into(), row(50, 50))];
        let line = first_baseline(&rows).unwrap();
        assert_eq!(
            line,
            "first baseline: 2 clips, 1 at 100%, 90/150 frames together"
        );
        // And later gates hold to it as usual.
        assert!(worse(&rows, &rows).is_none());
        rows[1].1.error = Some("the recording ended early".into());
        let why = first_baseline(&rows).unwrap_err();
        assert_eq!(
            why,
            "1 of 2 clips don't make: jump: the recording ended early"
        );
        assert!(first_baseline(&Rows::new()).is_err());
    }

    /// Moves the repository's tip: line `n` of model.toml becomes `text`.
    fn move_tip(d: &Path, n: usize, text: &str) {
        let body = fs::read_to_string(d.join("model.toml")).unwrap();
        let mut lines: Vec<&str> = body.lines().collect();
        lines[n] = text;
        fs::write(d.join("model.toml"), lines.join("\n") + "\n").unwrap();
        git(d, &["commit", "-q", "-am", "moved"]).unwrap();
    }

    #[test]
    fn a_patch_on_an_older_tip_is_merged_three_ways_or_fails_with_its_conflicts() {
        // As it is on its own tip.
        let (t, patch) = repo_with_patch();
        assert_eq!(apply_how(t.path(), &patch, true).unwrap(), None);
        // The tip moved far from the patch's lines: it still applies as
        // it is, with an offset.
        let (t, patch) = repo_with_patch();
        move_tip(t.path(), 8, "a8 = 80");
        assert_eq!(apply_how(t.path(), &patch, true).unwrap(), None);
        // The tip moved inside the hunk's context, so it only goes on by
        // a three-way merge.
        let (t, patch) = repo_with_patch();
        move_tip(t.path(), 4, "a4 = 40");
        assert!(apply_how(t.path(), &patch, false).is_err());
        git(t.path(), &["reset", "-q", "--hard"]).unwrap();
        let note = apply_how(t.path(), &patch, true).unwrap().unwrap();
        assert!(note.contains("merged three ways"), "{note}");
        let body = fs::read_to_string(t.path().join("model.toml")).unwrap();
        assert!(
            body.contains("a1 = 100\na2 = 2\na3 = 3\na4 = 40\n"),
            "{body}"
        );
        // The tip changed the same line: a conflict, by file, and the
        // tree is left clean.
        let (t, patch) = repo_with_patch();
        move_tip(t.path(), 1, "a1 = 7");
        let e = apply_how(t.path(), &patch, true).unwrap_err();
        assert!(e.contains("conflicts in model.toml"), "{e}");
        assert_eq!(
            git(t.path(), &["status", "--porcelain", "--untracked-files=no"]).unwrap(),
            ""
        );
    }

    /// Two patches that each change CLI.md conflict there; the merge takes
    /// the tip's side, which the tool gate writes again with the merged gp.
    #[test]
    fn a_conflict_in_a_generated_file_is_no_conflict() {
        let (t, _) = repo_with_patch();
        let d = t.path();
        let g = |args: &[&str]| git(d, args).unwrap();
        fs::write(d.join("CLI.md"), "a\n").unwrap();
        g(&["add", "CLI.md"]);
        g(&["commit", "-q", "-m", "page"]);
        fs::write(d.join("CLI.md"), "a\nc\n").unwrap();
        let diff = g(&["diff"]) + "\n";
        g(&["checkout", "-q", "--", "CLI.md"]);
        let patch = d.join("c.patch");
        fs::write(&patch, diff).unwrap();
        fs::write(d.join("CLI.md"), "a\nb\n").unwrap();
        g(&["commit", "-q", "-am", "b landed"]);
        let note = apply_how(d, &patch, true).unwrap().unwrap();
        assert!(note.contains("CLI.md written again"), "{note}");
        assert_eq!(fs::read_to_string(d.join("CLI.md")).unwrap(), "a\nb\n");
    }

    /// CLI.md is written again with the tree's gp and staged; a second
    /// time there is nothing to write.
    #[test]
    fn the_generated_files_are_written_again_with_the_trees_gp() {
        let (t, _) = repo_with_patch();
        let d = t.path();
        let g = |args: &[&str]| git(d, args).unwrap();
        fs::write(d.join("CLI.md"), "cli\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "patch one"]);
        // A gp whose commands are a and b, as the merged code has them.
        let bin = tempfile::tempdir().unwrap();
        let gp = bin.path().join("gp");
        fs::write(&gp, "#!/bin/sh\nprintf 'a\\nb\\n' > \"$3\"\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&gp, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(regenerate(d, &gp).unwrap());
        assert_eq!(fs::read_to_string(d.join("CLI.md")).unwrap(), "a\nb\n");
        assert_eq!(g(&["diff", "--cached", "--name-only"]), "CLI.md");
        g(&["commit", "-q", "-m", "written"]);
        assert!(!regenerate(d, &gp).unwrap());
        assert!(generated_file("CLI.md"));
        assert!(!generated_file("docs/gate.md"));
    }

    /// Two patches made on one tip each append a dated row to the notes
    /// log (an append-only file) and change model.toml apart; the second,
    /// on the tip the first moved, merges with both rows kept in date
    /// order. A patch that changes a row the base has still conflicts.
    #[test]
    fn two_patches_appending_rows_to_an_append_only_file_keep_both() {
        let (t, _) = repo_with_patch();
        let d = t.path();
        let run = |args: &[&str]| git(d, args).unwrap();
        let log = d.join(crate::gate_append::APPEND_ONLY[0]);
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        let head = "## 5. Notes log\n\n| Date | Branch | Finding |\n|---|---|---|\n";
        let row = |date: &str, what: &str| format!("| {date} | lane | {what} |\n");
        let old = row("2026-11-02", "old");
        fs::write(&log, format!("{head}{old}")).unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "log"]);
        let patch_of = |name: &str, line: usize, rows: &str| -> PathBuf {
            let body = fs::read_to_string(d.join("model.toml")).unwrap();
            let mut lines: Vec<String> = body.lines().map(str::to_owned).collect();
            lines[line] = format!("a{line} = {}", line * 100);
            fs::write(d.join("model.toml"), lines.join("\n") + "\n").unwrap();
            fs::write(&log, format!("{head}{old}{rows}")).unwrap();
            let diff = run(&["diff"]) + "\n";
            run(&["checkout", "-q", "--", "."]);
            let p = d.join(name);
            fs::write(&p, diff).unwrap();
            p
        };
        let (first_row, second_row) = (row("2026-11-04", "first"), row("2026-11-03", "second"));
        let first = patch_of("first.patch", 1, &first_row);
        let second = patch_of("second.patch", 8, &second_row);
        fs::write(&log, format!("{head}{}", row("2026-11-02", "changed"))).unwrap();
        let changes = run(&["diff"]) + "\n";
        run(&["checkout", "-q", "--", "."]);
        assert_eq!(apply_how(d, &first, true).unwrap(), None);
        run(&["commit", "-q", "-m", "first"]);
        let said = apply_how(d, &second, true).unwrap().unwrap();
        assert!(said.contains("by its added rows"), "{said}");
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            format!("{head}{old}{second_row}{first_row}")
        );
        let model = fs::read_to_string(d.join("model.toml")).unwrap();
        assert!(
            model.contains("a1 = 100\n") && model.contains("a8 = 800\n"),
            "{model}"
        );
        assert_eq!(run(&["diff", "--name-only", "--diff-filter=U"]), "");
        run(&["commit", "-q", "-m", "second"]);
        // A patch that changes the base's row conflicts, and the tree is
        // left clean.
        let p = d.join("changes.patch");
        fs::write(&p, changes).unwrap();
        let e = apply_how(d, &p, true).unwrap_err();
        assert!(e.contains("conflicts in docs/research"), "{e}");
        assert_eq!(run(&["status", "--porcelain", "--untracked-files=no"]), "");
    }

    /// A patch made on a model's hand-laid text (comments, another table
    /// order) merges onto a tip that made the text canonical: the three
    /// stages are merged through their canonical texts, so the patch lands
    /// as the facts it changes and the result is canonical. A patch whose
    /// fact clashes with the tip's still conflicts.
    #[test]
    fn a_patch_on_a_models_old_layout_merges_onto_its_canonical_text() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let run = |args: &[&str]| git(d, args).unwrap();
        run(&["init", "-q"]);
        run(&["config", "user.email", "gate@test"]);
        run(&["config", "user.name", "gate test"]);
        run(&["config", "commit.gpgsign", "false"]);
        run(&["config", "core.hooksPath", "/dev/null"]);
        let f = "g/model.toml";
        fs::create_dir_all(d.join("g")).unwrap();
        let old = "# A hand-laid model: a comment, and the counters before the header.\n\
                   [counters.lives]\nstart = 3\n\n[counters.score]\nstart = 2\n\n\
                   [model]\nschema = \"0.4\"\nname = \"g\"\ntitle = \"G\"\nsource = \"g\"\n\
                   view = \"side\"\nframe_rate = \"60\"\nsystems = [\"rules\"]\n";
        fs::write(d.join(f), old).unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "base"]);
        let patch_of = |name: &str, from: &str, to: &str| -> PathBuf {
            fs::write(d.join(f), old.replace(from, to)).unwrap();
            let diff = run(&["diff"]) + "\n";
            run(&["checkout", "-q", "--", "."]);
            let p = d.join(name);
            fs::write(&p, diff).unwrap();
            p
        };
        let lives = patch_of("lives.patch", "start = 3", "start = 5");
        let clash = patch_of("clash.patch", "start = 2", "start = 7");
        // The tip: canonical, and the score's start changed.
        let canon = |text: &str| canonical_text(f, text).unwrap();
        let tip = canon(&old.replace("start = 2", "start = 1"));
        assert_ne!(tip, old);
        fs::write(d.join(f), &tip).unwrap();
        run(&["commit", "-q", "-am", "canonical"]);
        assert!(git(d, &["apply", "--check", &lives.to_string_lossy()]).is_err());
        let said = apply_how(d, &lives, true).unwrap().unwrap();
        assert!(
            said.contains("model.toml through its canonical form"),
            "{said}"
        );
        let merged = fs::read_to_string(d.join(f)).unwrap();
        assert_eq!(
            merged,
            canon(
                &old.replace("start = 3", "start = 5")
                    .replace("start = 2", "start = 1")
            )
        );
        assert_eq!(run(&["diff", "--name-only", "--diff-filter=U"]), "");
        run(&["commit", "-q", "-m", "lives"]);
        let e = apply_how(d, &clash, true).unwrap_err();
        assert!(e.ends_with("conflicts in g/model.toml"), "{e}");
        assert_eq!(run(&["status", "--porcelain", "--untracked-files=no"]), "");
        assert!(canonical_file("templates/mosshop/model.toml"));
        assert!(canonical_file("donkey-kong/fidelity.toml"));
        assert!(!canonical_file("donkey-kong/themes/model.toml"));
    }

    /// The loads guard: a ROM the package loads only to play, named as the
    /// corpus's target or the template's source, is refused; Donkey Kong's
    /// own layout (OE scored, JU played) passes.
    #[test]
    fn a_rom_loaded_only_to_play_is_never_a_source() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let (oe, ju) = ("a".repeat(64), "b".repeat(64));
        fs::write(
            d.join("package.toml"),
            format!(
                "[[rom.entry]]\nid = \"g-oe\"\nrole = \"scored\"\nsha256 = \"{oe}\"\n\n[[rom.entry]]\nid = \"g-ju\"\nrole = \"play\"\nsha256 = \"{ju}\"\n"
            ),
        )
        .unwrap();
        fs::write(
            d.join("template.toml"),
            format!("[source]\nid = \"g-oe\"\naccepts = [\"{oe}\"]\n\n[[loads]]\nsource = \"g-ju\"\naccepts = [\"{ju}\"]\n"),
        )
        .unwrap();
        fs::write(
            d.join("corpus.toml"),
            format!("[target]\ngame = \"g-oe\"\nrom = \"{oe}\"\n"),
        )
        .unwrap();
        assert!(loads_problems(d).is_empty());
        fs::write(
            d.join("corpus.toml"),
            format!("[target]\ngame = \"g-oe\"\nrom = \"{ju}\"\n"),
        )
        .unwrap();
        let p = loads_problems(d);
        assert_eq!(p.len(), 1);
        assert!(
            p[0].starts_with("corpus.toml [target] rom names bbbb"),
            "{p:?}"
        );
        fs::write(d.join("template.toml"), "[source]\nid = \"g-ju\"\n").unwrap();
        assert_eq!(loads_problems(d).len(), 2);
    }

    /// Donkey Kong in the games checkout beside this repo, when it's there,
    /// passes the loads guard.
    #[test]
    fn donkey_kong_names_no_play_only_rom_as_a_source() {
        let dk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/donkey-kong");
        if !dk.join("package.toml").is_file() {
            eprintln!("no games checkout beside this repo; skipped");
            return;
        }
        assert_eq!(loads_problems(&dk), Vec::<String>::new());
    }
}

/// What `--import` made: nothing to gate, or a patch and its message.
enum Imported {
    Nothing(Outcome),
    Patch(PathBuf, String),
}

/// `gp corpus import GAME --write` on the tip of `onto`, in a worktree of
/// its own under the gate's home, from the person's sessions
/// (`paths.home`): the result is a patch for the gate to take like any
/// other. The import removes only the clips `--removes` names; the same
/// names then let the removed-clips check pass. Ported from game-printer's
/// gate.rs; the plan keeps no curves here, so no run measures them.
fn import_patch(
    repo: &Path,
    rel: &Path,
    onto: &str,
    home: &Path,
    paths: &Paths,
    removes: &[String],
) -> Result<Imported> {
    let game = repo
        .join(rel)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let rel_s = rel.to_string_lossy().into_owned();
    let dir = home
        .join("work")
        .join("import")
        .join(format!("{game}-{}", std::process::id()));
    let tip = git(repo, &["rev-parse", onto])?;
    git(
        repo,
        &["worktree", "add", "--detach", &dir.to_string_lossy(), &tip],
    )?;
    let made = (|| -> Result<Imported> {
        let o = crate::corpus_import::run(
            &crate::corpus_import::Args {
                game: &game,
                template: Some(&dir.join(rel)),
                from: Some(&paths.home),
                write: true,
                says: None,
                date: None,
                remove: removes,
                sessions: &[],
            },
            paths,
        )?;
        let v = o.json;
        git(&dir, &["add", "-A", "--", &rel_s])?;
        // Untrimmed: a patch's last context line may be a lone space.
        let diff = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(["diff", "--cached", "--binary"])
                .output()
                .context("running git diff")?
                .stdout,
        )?;
        if diff.trim().is_empty() {
            return Ok(Imported::Nothing(Outcome::ok(
                json!({"verdict": "PASS", "landed": false, "import": v}),
                "PASS: the corpus has every clip of the sessions; nothing to land".to_owned(),
            )));
        }
        let ids = |k: &str| {
            v[k].as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        };
        let mut what = Vec::new();
        for k in ["added", "changed", "removed"] {
            let i = ids(k);
            if !i.is_empty() {
                what.push(format!("{k} {i}"));
            }
        }
        if what.is_empty() {
            what.push("the corpus plan".into());
        }
        let msg = format!(
            "{game} corpus {}: {} (gp corpus import)",
            v["corpus"]["suggests"].as_str().unwrap_or(""),
            what.join("; ")
        );
        let file = home
            .join("work")
            .join("import")
            .join(format!("{game}-{}.patch", std::process::id()));
        fs::write(&file, diff)?;
        Ok(Imported::Patch(file, msg))
    })();
    let _ = git(
        repo,
        &["worktree", "remove", "--force", &dir.to_string_lossy()],
    );
    made
}

/// Writes the status file for a landed patch into the person's
/// `$GP_HOME` (not the gate's), and says where, or why not.
fn status_line(c: &Ctx, commit: &str, base: &Rows, new: &Rows) -> String {
    let tree_game = c.tree.join(&c.rel);
    let game = c.copy.clone().unwrap_or_else(|| c.repo.join(&c.rel));
    let home = match gp_evidence::Paths::from_env() {
        Ok(p) => p.home,
        Err(e) => return format!("status not written: {e:#}"),
    };
    let title = git(&c.tree, &["log", "-1", "--format=%s", commit]).unwrap_or_default();
    let when = git(&c.tree, &["log", "-1", "--format=%cI", commit]).unwrap_or_default();
    let hash = {
        let mut b = fs::read(tree_game.join(gp_model::MODEL_FILE)).unwrap_or_default();
        b.extend(fs::read(tree_game.join(gp_interp::mapping::MAPPING_FILE)).unwrap_or_default());
        gp_evidence::Hash::of(&b).to_string()
    };
    let name = game
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match write_status(&home, &name, commit, (&title, &when), &hash, base, new) {
        Ok(f) => format!("status: {}", f.display()),
        Err(e) => format!("status not written: {e:#}"),
    }
}

/// A clip ID's mechanic session and clip number: `dk.clear-25m.001`.
fn session_of(id: &str) -> (String, Option<u64>) {
    let mut parts = id.rsplitn(2, '.');
    let n = parts.next().and_then(|n| n.parse().ok());
    let rest = parts.next().unwrap_or("");
    let session = rest.split_once('.').map_or(rest, |(_, s)| s);
    (session.to_owned(), n)
}

/// Frames together over frames, straight through, and in resync for the
/// long runs, summed over `rows`.
fn totals<'a>(rows: impl Iterator<Item = &'a Row>) -> (u64, u64, u64, u64) {
    let (mut t, mut f, mut rt, mut rf) = (0, 0, 0, 0);
    for r in rows {
        t += r.together;
        f += r.frames;
        if let (Some(a), Some(b)) = (
            r.raw["stretches"]["together"].as_u64(),
            r.raw["stretches"]["frames"].as_u64(),
        ) {
            rt += a;
            rf += b;
        }
    }
    (t, f, rt, rf)
}

fn pct(a: u64, b: u64) -> Value {
    match (a * 1000).checked_div(b) {
        None => Value::Null,
        Some(q) => json!((q as f64) / 10.0),
    }
}

fn together_json(t: u64, f: u64) -> Value {
    if f == 0 {
        Value::Null
    } else {
        json!({"together": t, "of": f, "pct": pct(t, f)})
    }
}

/// The scoring status the mechanic page and `gp mechanic queue` read:
/// `$GP_HOME/work/refine/<game>/status.json`, written to a temporary file
/// and renamed, with the last five merges. Ported from game-printer's
/// gate.rs.
pub(crate) fn write_status(
    home: &Path,
    game: &str,
    commit: &str,
    (title, at): (&str, &str),
    hash: &str,
    base: &Rows,
    new: &Rows,
) -> Result<PathBuf> {
    let dir = home.join("work").join("refine").join(game);
    fs::create_dir_all(&dir)?;
    let file = dir.join("status.json");
    let old: Value = fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let clip_json = |id: &str, r: &Row| -> Value {
        let (session, n) = session_of(id);
        let start = r.raw["start"].as_u64();
        let st = &r.raw["stretches"];
        json!({
            "id": id,
            "mechanic": session,
            "clip": n,
            "start": start,
            "frames": r.frames,
            "straight": together_json(r.together, r.frames),
            "resync": together_json(st["together"].as_u64().unwrap_or(0), st["frames"].as_u64().unwrap_or(0)),
            "longest_exact": st["longest_any"].as_u64().map(|f| json!({"frames": f, "from": Value::Null})),
            "first_split": r.first.as_ref().map(|(f, of, p)| json!({
                "frame": of, "clip_frame": f, "what": format!("{p} apart"), "paths": [p], "owner": Value::Null})),
            "draws": Value::Null,
            "bars": r.raw["bars"].clone(),
        })
    };
    let mut sessions: Vec<String> = Vec::new();
    for (id, _) in new {
        let s = session_of(id).0;
        if !sessions.contains(&s) {
            sessions.push(s);
        }
    }
    let row_pct = |rows: &Rows, s: &str| {
        let (t, f, _, _) = totals(
            rows.iter()
                .filter(|(i, _)| session_of(i).0 == s)
                .map(|(_, r)| r),
        );
        pct(t, f)
    };
    let rows: Vec<Value> = sessions
        .iter()
        .map(|s| {
            let mine: Vec<(&String, &Row)> = new
                .iter()
                .filter(|(i, _)| &session_of(i).0 == s)
                .map(|(i, r)| (i, r))
                .collect();
            let (t, f, rt, rf) = totals(mine.iter().map(|(_, r)| *r));
            let best = mine
                .iter()
                .filter_map(|(i, r)| {
                    r.raw["stretches"]["longest_any"]
                        .as_u64()
                        .map(|l| (l, session_of(i).1))
                })
                .max_by_key(|(l, _)| *l);
            json!({
                "mechanic": s,
                "straight": together_json(t, f),
                "resync": together_json(rt, rf),
                "longest_exact": best.map(|(l, n)| json!({"frames": l, "clip": n, "from": Value::Null})),
                "clips": mine.iter().map(|(i, _)| session_of(i).1).collect::<Vec<_>>(),
            })
        })
        .collect();
    let (bt, bf, brt, brf) = totals(base.iter().map(|(_, r)| r));
    let (nt, nf, nrt, nrf) = totals(new.iter().map(|(_, r)| r));
    let mut moved = serde_json::Map::new();
    for s in &sessions {
        let (b, n) = (row_pct(base, s), row_pct(new, s));
        if b != n {
            moved.insert(s.clone(), json!({"before": b, "after": n}));
        }
    }
    let mut merges = vec![json!({
        "commit": &commit[..8.min(commit.len())],
        "title": title,
        "at": at,
        "before": {"straight_pct": pct(bt, bf), "resync_pct": pct(brt, brf)},
        "after": {"straight_pct": pct(nt, nf), "resync_pct": pct(nrt, nrf)},
        "rows": moved,
    })];
    if let Some(a) = old["merges"].as_array() {
        merges.extend(a.iter().take(4).cloned());
    }
    let status = json!({
        "game": game,
        "version": old["version"].as_u64().unwrap_or(0) + 1,
        "model": {"commit": &commit[..8.min(commit.len())], "hash": hash},
        "rows": rows,
        "clips": new.iter().map(|(i, r)| clip_json(i, r)).collect::<Vec<_>>(),
        "merges": merges,
    });
    let tmp = dir.join("status.json.tmp");
    fs::write(
        &tmp,
        format!("{}\n", serde_json::to_string_pretty(&status)?),
    )?;
    fs::rename(&tmp, &file)?;
    Ok(file)
}

#[cfg(test)]
mod status_tests {
    use super::*;

    fn row(id: &str, together: u64, frames: u64) -> (String, Row) {
        let first = (together < frames).then(
            || json!({"frame": together, "original_frame": together + 100, "path": "mario.x"}),
        );
        (
            id.to_owned(),
            Row::of(&json!({
                "frames": frames, "frames_together": together, "start": 50,
                "stretches": {"together": together, "frames": frames, "longest_any": together},
                "first": first, "bars": {"together": together == frames},
            })),
        )
    }

    /// The status file a landing writes: a row for each mechanic, each
    /// clip with its first split, the merge with the scores before and
    /// after, and the earlier merges kept; and the quest log reads it.
    #[test]
    fn a_landing_writes_the_status_the_quest_log_reads() {
        let d = tempfile::tempdir().unwrap();
        let base = vec![row("dk.walk.001", 100, 100), row("dk.jump.001", 40, 80)];
        let new = vec![row("dk.walk.001", 100, 100), row("dk.jump.001", 80, 80)];
        let f = write_status(
            d.path(),
            "dk",
            "0123456789ab",
            ("Jump fixed", "2026-11-12"),
            "h",
            &base,
            &new,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v["model"]["commit"], "01234567");
        assert_eq!(v["rows"][1]["mechanic"], "jump");
        assert_eq!(v["rows"][1]["straight"]["pct"], 100.0);
        assert_eq!(v["merges"][0]["rows"]["jump"]["before"], 50.0);
        assert!(v["merges"][0]["rows"].get("walk").is_none());
        let f = write_status(
            d.path(),
            "dk",
            "fedcba98",
            ("Again", "2026-11-12"),
            "h",
            &new,
            &base,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(
            (v["version"].as_u64(), v["merges"].as_array().unwrap().len()),
            (Some(2), 2)
        );
        assert_eq!(v["clips"][1]["first_split"]["what"], "mario.x apart");
    }
}
