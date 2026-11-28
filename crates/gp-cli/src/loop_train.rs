//! `gp loop hand-in` and `gp loop train` (issue 58, docs/loop.md "The
//! train"): the rows marked for the train build their whole work, test it
//! and hand their patch in; the person runs the train by hand, which stacks
//! the handed-in patches in the order they came on a scratch worktree of the
//! tip, drops any that does not apply (its row goes back to next with why),
//! runs the checks that fail fast, and gates the stack once: one tool gate
//! for the tools' patches, one model gate for each game's. A failing stack
//! is tried once more, then split in halves to find the patch at fault,
//! whose row goes back to next with the gate's words while the rest land.
//!
//! The hand-ins are kept under `$GP_HOME/work/loop/train/`, one
//! `ID.json` and `ID.patch` each, the ID starting with the time so the
//! order they came in is the order of the names. Rows are written through
//! the quest log's own locked writer (crate::mechanic::session::update_queue).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use crate::Outcome;
use crate::gate::git;
#[cfg(test)]
use crate::mechanic::session::write_queue;
use crate::mechanic::session::{queue_file, read_queue};

/// Where the hand-ins are kept.
pub(crate) fn train_dir(home: &Path) -> PathBuf {
    home.join("work").join("loop").join("train")
}

/// One handed-in patch.
#[derive(Clone, Debug)]
pub(crate) struct HandIn {
    /// The file stem under [`train_dir`]: the time, the game and the row.
    pub id: String,
    pub game: String,
    pub row: String,
    /// "tools" (gameprinter) or "games" (the game's repo).
    pub repo: String,
    /// The commit the patch was made on, when its tree said. Kept on file
    /// for whoever reads a hand-in; the train stacks on the tip as it is.
    #[cfg_attr(not(test), allow(dead_code))]
    pub tip: String,
    pub message: String,
    pub removes_test: Vec<String>,
    pub at_ms: u64,
    pub patch: PathBuf,
}

impl HandIn {
    fn name(&self) -> String {
        format!("{}/{}", self.game, self.row)
    }

    fn from_json(dir: &Path, v: &Value) -> Option<HandIn> {
        let id = v["id"].as_str()?.to_owned();
        Some(HandIn {
            patch: dir.join(format!("{id}.patch")),
            id,
            game: v["game"].as_str()?.to_owned(),
            row: v["row"].as_str()?.to_owned(),
            repo: v["repo"].as_str().unwrap_or("games").to_owned(),
            tip: v["tip"].as_str().unwrap_or("").to_owned(),
            message: v["message"].as_str().unwrap_or("").to_owned(),
            removes_test: v["removes_test"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            at_ms: v["at_ms"].as_u64().unwrap_or(0),
        })
    }
}

/// The handed-in patches, in the order they came.
pub(crate) fn pending(home: &Path) -> Vec<HandIn> {
    let dir = train_dir(home);
    let mut out: Vec<HandIn> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| fs::read_to_string(&p).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .filter_map(|v| HandIn::from_json(&dir, &v))
        .filter(|h| h.patch.is_file())
        .collect();
    out.sort_by(|a, b| (a.at_ms, &a.id).cmp(&(b.at_ms, &b.id)));
    out
}

/// Sets row `row` of `game`'s quest log through `f`; Err when the log has
/// no such row.
fn set_row(
    paths: &gp_evidence::Paths,
    game: &str,
    row: &str,
    f: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> Result<()> {
    let qf = queue_file(paths, game)?;
    crate::mechanic::session::update_queue(&qf, |q| {
        let Some(o) = q
            .iter_mut()
            .find(|r| r["mechanic"] == json!(row))
            .and_then(Value::as_object_mut)
        else {
            bail!("{game}'s quest log has no row {row}");
        };
        f(o);
        Ok(())
    })
}

/// `gp loop hand-in`: copies the patch and its message under
/// [`train_dir`] with the row, its repo, the tip it was made on and the
/// time, and sets the row in line, handed in. A row handed in again
/// replaces its earlier patch and goes to the end of the order.
pub(crate) fn hand_in(
    paths: &gp_evidence::Paths,
    game: &str,
    row: &str,
    patch: &Path,
    message: &str,
    removes_test: &[String],
) -> Result<Value> {
    let qf = queue_file(paths, game)?;
    let q = read_queue(&qf);
    let Some(r) = q.iter().find(|r| r["mechanic"] == json!(row)) else {
        bail!("{game}'s quest log has no row {row}");
    };
    let repo = if r["repo"] == json!("tools") {
        "tools"
    } else {
        "games"
    };
    let text = fs::read_to_string(patch).with_context(|| format!("reading {}", patch.display()))?;
    if text.trim().is_empty() {
        bail!(
            "the patch {} is empty: git diff > FILE in your worktree",
            patch.display()
        );
    }
    if message.trim().is_empty() {
        bail!("a hand-in says what it changes (--message)");
    }
    // The tip the patch was made on: its worktree's HEAD.
    let tip = patch
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .and_then(|d| git(&d, &["rev-parse", "HEAD"]).ok())
        .unwrap_or_default();
    let dir = train_dir(&paths.home);
    fs::create_dir_all(&dir)?;
    for old in pending(&paths.home)
        .into_iter()
        .filter(|h| h.game == game && h.row == row)
    {
        let _ = fs::remove_file(&old.patch);
        let _ = fs::remove_file(dir.join(format!("{}.json", old.id)));
    }
    let at = gp_mcp::asks::now_ms();
    let id = format!("{at:013}-{game}-{row}");
    fs::write(dir.join(format!("{id}.patch")), &text)?;
    let v = json!({
        "id": id,
        "game": game,
        "row": row,
        "repo": repo,
        "tip": tip,
        "message": message,
        "removes_test": removes_test,
        "at_ms": at,
    });
    fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_string_pretty(&v)?,
    )?;
    set_row(paths, game, row, |o| {
        o.insert("status".into(), json!("in line"));
        o.insert(
            "note".into(),
            json!(format!(
                "Handed in to the train: {}. gp loop train gates it with the other rows' patches.",
                first_line(message)
            )),
        );
        o.insert("train".into(), json!({"id": id, "at_ms": at}));
        o.remove("worker");
    })?;
    Ok(v)
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

/// Which gate a hand-in goes through: the tool gate, or the model gate of
/// its game.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Group {
    Tools,
    Game(String),
}

impl Group {
    fn of(h: &HandIn) -> Group {
        if h.repo == "tools" {
            Group::Tools
        } else {
            Group::Game(h.game.clone())
        }
    }

    fn words(&self) -> String {
        match self {
            Group::Tools => "the tool gate".into(),
            Group::Game(g) => format!("{g}'s model gate"),
        }
    }

    fn slug(&self) -> String {
        match self {
            Group::Tools => "tools".into(),
            Group::Game(g) => format!("game-{g}"),
        }
    }
}

/// The files of the page and its checks, the paths that make GitHub run
/// `just check-web` (.github/workflows/check-web.yml's paths, which a test
/// holds to this list): a tools patch that changes one makes the train run
/// `just check-web` on its stack (row train-runs-check-web-for-page-patches).
/// A pattern's `**` is any path under it, its `*` any part of one name.
pub(crate) const PAGE_PATHS: &[&str] = &[
    "scripts/web/**",
    "scripts/inspector/**",
    "editor/**",
    "mechanic/**",
    "crates/gp-play/**",
    "crates/gp-web/**",
    "crates/gp-web-*/**",
    "crates/gp-arcade-play/**",
    "crates/gp-cli/src/serve/**",
    "crates/gp-cli/src/mechanic/**",
    "justfile",
    ".github/workflows/check-web.yml",
];

/// One name of a path against one name of a pattern, `*` any run of
/// letters.
fn name_matches(pat: &str, name: &str) -> bool {
    match pat.split_once('*') {
        None => pat == name,
        Some((head, tail)) => {
            name.len() >= head.len() + tail.len() && name.starts_with(head) && name.ends_with(tail)
        }
    }
}

/// Whether `path` is under pattern `pat` of [`PAGE_PATHS`].
fn path_matches(pat: &str, path: &str) -> bool {
    let (mut p, mut f) = (pat.split('/'), path.split('/'));
    loop {
        match (p.next(), f.next()) {
            (Some("**"), Some(_)) => return true,
            (Some(a), Some(b)) if name_matches(a, b) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// The files a patch changes, from its `diff --git a/X b/Y` lines (both
/// names, so a file moved out of the page counts too).
fn patch_files(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        let Some(rest) = l.strip_prefix("diff --git a/") else {
            continue;
        };
        let (a, b) = rest.split_once(" b/").unwrap_or((rest, rest));
        for f in [a, b] {
            if !out.iter().any(|o| o == f) {
                out.push(f.to_owned());
            }
        }
    }
    out
}

/// The page's files (by [`PAGE_PATHS`]) that `items`' patches change.
fn page_files(items: &[HandIn]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in items {
        let text = fs::read_to_string(&h.patch).unwrap_or_default();
        for f in patch_files(&text) {
            if PAGE_PATHS.iter().any(|p| path_matches(p, &f)) && !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// Up to three of `files`, and how many more.
fn some_files(files: &[String]) -> String {
    let mut s = files.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if files.len() > 3 {
        s.push_str(&format!(" and {} more", files.len() - 3));
    }
    s
}

/// One call of the gate: the stack as one patch.
#[derive(Clone)]
pub(crate) struct Batch<'a> {
    pub group: &'a Group,
    /// The repo the gate lands in.
    pub repo: &'a Path,
    pub patch: &'a Path,
    pub message: &'a str,
    pub removes_test: &'a [String],
    pub rows: Vec<String>,
    /// A dry run, to find the patch at fault: lands nothing.
    pub dry_run: bool,
}

/// The gate: Ok with the commit it landed (or a pass, on a dry run), Err
/// with its words.
pub(crate) type Gate<'a> = dyn FnMut(&Batch) -> std::result::Result<String, String> + 'a;

/// `just check-web` on a stack's scratch worktree: Err with its words.
pub(crate) type WebCheck<'a> = dyn FnMut(&Path) -> std::result::Result<(), String> + 'a;

/// What the train runs on a stack: the gate, and `just check-web` after
/// the tool gate's dry pass when the tools' patches change the page
/// (unless `check_web` is off: `--no-check-web`).
pub(crate) struct Checks<'a> {
    pub gate: &'a mut Gate<'a>,
    pub web: &'a mut WebCheck<'a>,
    pub check_web: bool,
}

/// What the train did to each row, in its order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fate {
    Landed(String),
    Back(String),
    /// A dry run: it applies and would be gated by this gate.
    WouldGate(String),
}

/// A stack on a scratch worktree of the tip.
struct Stack {
    tree: PathBuf,
    tip: String,
    kept: Vec<HandIn>,
    dropped: Vec<(HandIn, String)>,
}

/// `git` in `dir` with its exit, stdout and stderr.
fn git_try(dir: &Path, args: &[&str]) -> (bool, String, String) {
    match Command::new("git").arg("-C").arg(dir).args(args).output() {
        Ok(o) => (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).trim().to_owned(),
        ),
        Err(e) => (false, String::new(), e.to_string()),
    }
}

/// The stack as it stands, kept as the index's tree (no commit is made on
/// the scratch worktree).
fn snapshot(tree: &Path) -> Result<String> {
    git(tree, &["add", "-A"])?;
    git(tree, &["write-tree"])
}

/// The scratch worktree back to snapshot `snap`.
fn reset(tree: &Path, snap: &str) {
    let _ = git_try(tree, &["read-tree", "--reset", "-u", snap]);
    let _ = git_try(tree, &["clean", "-fdq"]);
}

/// Removes the scratch worktree `tree` of `repo`, if it is there.
fn remove_tree(repo: &Path, tree: &Path) {
    if tree.exists() {
        let _ = git_try(
            repo,
            &["worktree", "remove", "--force", &tree.to_string_lossy()],
        );
        let _ = fs::remove_dir_all(tree);
    }
    let _ = git_try(repo, &["worktree", "prune"]);
}

/// Whether a version of a file only adds lines to `base`.
fn only_adds(base: &Path, other: &Path) -> bool {
    let o = Command::new("git")
        .args(["diff", "--no-index", "--numstat"])
        .arg(base)
        .arg(other)
        .output();
    let Ok(o) = o else { return false };
    String::from_utf8_lossy(&o.stdout).lines().all(|l| {
        l.split('\t')
            .nth(1)
            .is_some_and(|d| d.trim().parse::<u64>() == Ok(0))
    })
}

/// The conflicted files of a 3-way apply kept with both sides' lines, when
/// each side only added lines to the base there. Err with why not.
fn keep_both(tree: &Path, scratch: &Path) -> std::result::Result<(), String> {
    let (_, out, _) = git_try(tree, &["diff", "--name-only", "--diff-filter=U"]);
    let files: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
    if files.is_empty() {
        return Err("no conflicted file to merge".into());
    }
    fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    for f in &files {
        let stage = |n: u8| {
            let (ok, text, _) = git_try(tree, &["show", &format!(":{n}:{f}")]);
            if ok { text } else { String::new() }
        };
        let (base, ours, theirs) = (
            scratch.join("base"),
            scratch.join("ours"),
            scratch.join("theirs"),
        );
        for (p, t) in [(&base, stage(1)), (&ours, stage(2)), (&theirs, stage(3))] {
            fs::write(p, t).map_err(|e| e.to_string())?;
        }
        if !only_adds(&base, &ours) || !only_adds(&base, &theirs) {
            return Err(format!(
                "it changes the same lines of {f} as a patch before it"
            ));
        }
        let o = Command::new("git")
            .args(["merge-file", "-p", "--union"])
            .arg(&ours)
            .arg(&base)
            .arg(&theirs)
            .output()
            .map_err(|e| e.to_string())?;
        fs::write(tree.join(f), &o.stdout).map_err(|e| e.to_string())?;
        let (ok, _, err) = git_try(tree, &["add", "--", f]);
        if !ok {
            return Err(err);
        }
    }
    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

/// The name check (scripts/check/no-game-names.sh) on the stack, when the
/// tree has it: Err with its words.
fn names_check(tree: &Path) -> std::result::Result<(), String> {
    let script = tree.join("scripts/check/no-game-names.sh");
    if !script.is_file() {
        return Ok(());
    }
    match Command::new("bash").arg(&script).current_dir(tree).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "the name check fails: {}",
            last_lines(
                &format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                ),
                6
            )
        )),
        Err(e) => Err(format!("the name check didn't run: {e}")),
    }
}

/// Applies one hand-in on the stack at snapshot `snap` and moves `snap` on;
/// Err with why it doesn't apply, the tree as it was.
fn apply_one(tree: &Path, h: &HandIn, snap: &mut String) -> std::result::Result<(), String> {
    let patch = h.patch.to_string_lossy().into_owned();
    let (ok, _, err) = git_try(
        tree,
        &["apply", "--3way", "--whitespace=nowarn", patch.as_str()],
    );
    if !ok {
        let scratch = tree.with_extension("merge");
        if let Err(why) = keep_both(tree, &scratch) {
            reset(tree, snap);
            let _ = fs::remove_dir_all(&scratch);
            let why = if why == "no conflicted file to merge" {
                format!("it doesn't apply on the tip: {}", last_lines(&err, 4))
            } else {
                format!("it doesn't apply on the patches before it: {why}")
            };
            return Err(why);
        }
    }
    if let Err(why) = names_check(tree) {
        reset(tree, snap);
        return Err(why);
    }
    match snapshot(tree) {
        Ok(s) => {
            *snap = s;
            Ok(())
        }
        Err(e) => {
            reset(tree, snap);
            Err(e.to_string())
        }
    }
}

/// Stacks `items` in order on a fresh scratch worktree `tree` of `repo`'s
/// tip.
fn stack(repo: &Path, tree: &Path, items: &[HandIn]) -> Result<Stack> {
    remove_tree(repo, tree);
    let tip = git(repo, &["rev-parse", "HEAD"])?;
    if let Some(p) = tree.parent() {
        fs::create_dir_all(p)?;
    }
    git(
        repo,
        &["worktree", "add", "--detach", &tree.to_string_lossy(), &tip],
    )?;
    let mut snap = snapshot(tree)?;
    let mut s = Stack {
        tree: tree.to_path_buf(),
        tip,
        kept: Vec::new(),
        dropped: Vec::new(),
    };
    for h in items {
        match apply_one(tree, h, &mut snap) {
            Ok(()) => s.kept.push(h.clone()),
            Err(why) => s.dropped.push((h.clone(), why)),
        }
    }
    Ok(s)
}

/// cargo fmt over a stack of gameprinter's (a tree with a Cargo.toml):
/// formatting is fixed on the stack, not sent back.
fn fmt_stack(tree: &Path) {
    if !tree.join("Cargo.toml").is_file() {
        return;
    }
    let ok = Command::new("cargo")
        .args(["fmt", "--all", "--check"])
        .current_dir(tree)
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok
        && Command::new("cargo")
            .args(["fmt", "--all"])
            .current_dir(tree)
            .status()
            .is_ok_and(|s| s.success())
    {
        let _ = snapshot(tree);
    }
}

/// The stack as one patch and its message, written beside the hand-ins.
fn as_batch_files(dir: &Path, group: &Group, s: &Stack) -> Result<(PathBuf, String)> {
    git(&s.tree, &["add", "-A"])?;
    let diff = git(&s.tree, &["diff", "--binary", "--cached", &s.tip])?;
    let file = dir.join(format!("stack-{}.patch", group.slug()));
    fs::write(&file, format!("{diff}\n"))?;
    let message = if s.kept.len() == 1 {
        s.kept[0].message.clone()
    } else {
        let names: Vec<String> = s.kept.iter().map(HandIn::name).collect();
        let mut m = format!(
            "{} rows' work gated as one patch by gp loop train: {}.",
            s.kept.len(),
            names.join(", ")
        );
        for h in &s.kept {
            m.push_str(&format!("\n\n{}: {}", h.name(), h.message.trim()));
        }
        m
    };
    Ok((file, message))
}

fn removes_of(items: &[HandIn]) -> Vec<String> {
    let mut r: Vec<String> = items
        .iter()
        .flat_map(|h| h.removes_test.iter().cloned())
        .collect();
    r.sort();
    r.dedup();
    r
}

/// Gates `items` stacked on `repo`'s tip. When the tools' patches change
/// the page ([`page_files`]) and `just check-web` is on, the tool gate
/// runs dry first, then `just check-web` on the stack, then (unless
/// `dry_run`) the tool gate lands it: a failure of either is the gate's.
fn gate_items(
    dir: &Path,
    group: &Group,
    repo: &Path,
    tree: &Path,
    items: &[HandIn],
    dry_run: bool,
    checks: &mut Checks,
) -> Result<(Stack, std::result::Result<String, String>)> {
    let s = stack(repo, tree, items)?;
    if s.kept.is_empty() {
        return Ok((s, Err("nothing applies".into())));
    }
    fmt_stack(&s.tree);
    let (file, message) = as_batch_files(dir, group, &s)?;
    let removes = removes_of(&s.kept);
    let b = Batch {
        group,
        repo,
        patch: &file,
        message: &message,
        removes_test: &removes,
        rows: s.kept.iter().map(HandIn::name).collect(),
        dry_run,
    };
    let page = if *group == Group::Tools {
        page_files(&s.kept)
    } else {
        Vec::new()
    };
    if page.is_empty() {
        let r = (checks.gate)(&b);
        return Ok((s, r));
    }
    if !checks.check_web {
        eprintln!(
            "gp loop train: just check-web is not run (--no-check-web), though the stack changes {}",
            some_files(&page)
        );
        let r = (checks.gate)(&b);
        return Ok((s, r));
    }
    let dry = Batch {
        dry_run: true,
        ..b.clone()
    };
    let r = (checks.gate)(&dry).and_then(|pass| {
        eprintln!(
            "gp loop train: just check-web for {} (the stack changes {})",
            b.rows.join(", "),
            some_files(&page)
        );
        (checks.web)(&s.tree).map_err(|why| {
            format!(
                "just check-web fails on the stack (it changes {}): {why}",
                some_files(&page)
            )
        })?;
        if dry_run { Ok(pass) } else { (checks.gate)(&b) }
    });
    Ok((s, r))
}

/// The train for one gate's hand-ins: each row's fate, in their order.
fn run_group(
    dir: &Path,
    group: &Group,
    repo: &Path,
    items: &[HandIn],
    dry_run: bool,
    checks: &mut Checks,
) -> Result<Vec<(HandIn, Fate)>> {
    let tree = dir.join(format!("stack-{}", group.slug()));
    let mut fates: Vec<(HandIn, Fate)> = Vec::new();
    let mut todo = items.to_vec();
    if dry_run {
        let s = stack(repo, &tree, &todo)?;
        for (h, why) in s.dropped {
            fates.push((h, Fate::Back(why)));
        }
        for h in s.kept {
            let page = if *group == Group::Tools {
                page_files(std::slice::from_ref(&h))
            } else {
                Vec::new()
            };
            let words = match (page.is_empty(), checks.check_web) {
                (true, _) => group.words(),
                (false, true) => format!(
                    "{}, then just check-web (it changes {})",
                    group.words(),
                    some_files(&page)
                ),
                (false, false) => format!(
                    "{} (it changes {}; just check-web is not run: --no-check-web)",
                    group.words(),
                    some_files(&page)
                ),
            };
            fates.push((h, Fate::WouldGate(words)));
        }
        remove_tree(repo, &tree);
        return Ok(fates);
    }
    let mut tried_again = false;
    loop {
        let (s, mut r) = gate_items(dir, group, repo, &tree, &todo, false, checks)?;
        for (h, why) in s.dropped {
            fates.push((h, Fate::Back(why)));
        }
        todo = s.kept;
        if todo.is_empty() {
            break;
        }
        // A failure is tried once more, as a flaky check may pass.
        if r.is_err() && !tried_again {
            tried_again = true;
            r = gate_items(dir, group, repo, &tree, &todo, false, checks)?.1;
        }
        match r {
            Ok(c) => {
                for h in todo.drain(..) {
                    fates.push((h, Fate::Landed(c.clone())));
                }
                break;
            }
            Err(words) if todo.len() == 1 => {
                fates.push((todo.remove(0), Fate::Back(words)));
                break;
            }
            Err(words) => {
                // Split in halves: todo[..good] passes, todo[..end] fails.
                let (mut good, mut end, mut why) = (0, todo.len(), words);
                while end - good > 1 {
                    let mid = good + (end - good) / 2;
                    match gate_items(dir, group, repo, &tree, &todo[..mid], true, checks)?.1 {
                        Ok(_) => good = mid,
                        Err(w) => {
                            end = mid;
                            why = w;
                        }
                    }
                }
                fates.push((todo.remove(good), Fate::Back(why)));
            }
        }
    }
    remove_tree(repo, &tree);
    Ok(fates)
}

/// Where each gate's patches land.
pub(crate) struct Repos {
    /// The gameprinter checkout.
    pub tools: Option<PathBuf>,
    /// The games checkout.
    pub games: Option<PathBuf>,
}

/// `gp loop train`: every hand-in's fate, the rows set to match (unless
/// `dry_run`), in the order they came.
pub(crate) fn train(
    paths: &gp_evidence::Paths,
    repos: &Repos,
    dry_run: bool,
    checks: &mut Checks,
) -> Result<Vec<(HandIn, Fate)>> {
    let dir = train_dir(&paths.home);
    let all = pending(&paths.home);
    let mut groups: Vec<Group> = all.iter().map(Group::of).collect();
    groups.sort();
    groups.dedup();
    let mut fates = Vec::new();
    for g in &groups {
        let items: Vec<HandIn> = all.iter().filter(|h| &Group::of(h) == g).cloned().collect();
        let repo = match g {
            Group::Tools => repos.tools.as_deref(),
            Group::Game(_) => repos.games.as_deref(),
        };
        let Some(repo) = repo else {
            for h in items {
                fates.push((h, Fate::Back(format!("no checkout for {}", g.words()))));
            }
            continue;
        };
        fates.extend(run_group(&dir, g, repo, &items, dry_run, checks)?);
    }
    // In the order they came.
    fates.sort_by(|a, b| (a.0.at_ms, &a.0.id).cmp(&(b.0.at_ms, &b.0.id)));
    if !dry_run {
        for (h, f) in &fates {
            settle(paths, &dir, h, f)?;
        }
    }
    Ok(fates)
}

/// Sets a row as the train left it and files its hand-in away.
fn settle(paths: &gp_evidence::Paths, dir: &Path, h: &HandIn, f: &Fate) -> Result<()> {
    let set = set_row(paths, &h.game, &h.row, |o| {
        o.remove("train");
        match f {
            Fate::Landed(c) => {
                o.insert("status".into(), json!("done"));
                o.insert("commit".into(), json!(c));
                o.insert(
                    "note".into(),
                    json!(format!(
                        "Landed by gp loop train in {c}: {}",
                        first_line(&h.message)
                    )),
                );
            }
            Fate::Back(why) => {
                o.insert("status".into(), json!("next"));
                o.insert(
                    "note".into(),
                    json!(format!(
                        "gp loop train sent it back: {why}. Make the patch again on the tip, test it, and hand it in again."
                    )),
                );
            }
            Fate::WouldGate(_) => {}
        }
    });
    if let Err(e) = set {
        eprintln!("gp loop train: {}: {e}", h.name());
    }
    let done = dir.join("done");
    fs::create_dir_all(&done)?;
    for ext in ["json", "patch"] {
        let from = dir.join(format!("{}.{ext}", h.id));
        let _ = fs::rename(&from, done.join(format!("{}.{ext}", h.id)));
    }
    Ok(())
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join(" / ")
}

/// The gate the train runs: this gp's tool gate or model gate on the
/// stack's patch, from the repo the gate lands in.
pub(crate) fn real_gate(b: &Batch) -> std::result::Result<String, String> {
    let gp = crate::gate::this_gp().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(gp);
    match b.group {
        Group::Tools => {
            cmd.args(["tool", "gate", "--patch"]).arg(b.patch);
            for t in b.removes_test {
                cmd.args(["--removes-test", t]);
            }
        }
        Group::Game(g) => {
            cmd.args(["model", "gate"])
                .arg(b.repo.join(g))
                .arg("--patch")
                .arg(b.patch);
        }
    }
    cmd.args(["--message", b.message]);
    if b.dry_run {
        cmd.arg("--dry-run");
    }
    eprintln!(
        "gp loop train: {}{} for {}",
        b.group.words(),
        if b.dry_run { " (dry run)" } else { "" },
        b.rows.join(", ")
    );
    let o = cmd
        .current_dir(b.repo)
        .output()
        .map_err(|e| e.to_string())?;
    let said = format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    if !o.status.success() {
        let fail = said
            .lines()
            .find(|l| l.trim_start().starts_with("FAIL"))
            .map(str::trim)
            .map(str::to_owned);
        return Err(fail.unwrap_or_else(|| last_lines(&said, 6)));
    }
    if b.dry_run {
        return Ok("passed".into());
    }
    let branch = git(b.repo, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or("HEAD".into());
    git(b.repo, &["rev-parse", "--short", &branch]).map_err(|e| e.to_string())
}

/// `just check-web` on the scratch worktree `tree`, with the train's own
/// cargo target folder under `dir` (kept from one train to the next, so it
/// stays built) and its whole output in `dir/check-web.log`. Err with the
/// failed steps (step.sh's `check-step: NAME S s (failed)` lines) or the
/// last lines, and the log's place.
pub(crate) fn real_check_web(dir: &Path, tree: &Path) -> std::result::Result<(), String> {
    let log = dir.join("check-web.log");
    let o = Command::new("just")
        .arg("check-web")
        .current_dir(tree)
        .env("CARGO_TARGET_DIR", dir.join("target-web"))
        .env_remove("GP_STEP")
        .env_remove("GP_WORKER")
        .output()
        .map_err(|e| format!("just check-web didn't run: {e}"))?;
    let said = format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let _ = fs::write(&log, &said);
    if o.status.success() {
        return Ok(());
    }
    Err(format!(
        "{} (its log: {})",
        web_failure(&said),
        log.display()
    ))
}

/// What a failed `just check-web` said: its failed steps, else its last
/// lines.
fn web_failure(said: &str) -> String {
    let failed: Vec<&str> = said
        .lines()
        .filter_map(|l| l.trim().strip_prefix("check-step: "))
        .filter(|l| l.ends_with("(failed)"))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    if failed.is_empty() {
        last_lines(said, 6)
    } else {
        format!("step {} failed", failed.join(", "))
    }
}

/// One line a row.
fn fate_line(h: &HandIn, f: &Fate) -> String {
    match f {
        Fate::Landed(c) => format!("{}  landed in {c}", h.name()),
        Fate::Back(why) => format!("{}  back to next: {why}", h.name()),
        Fate::WouldGate(g) => format!("{}  applies; {g} would gate it", h.name()),
    }
}

/// `gp loop train` as a command.
pub(crate) fn run_train(
    paths: &gp_evidence::Paths,
    repo: Option<&Path>,
    dry_run: bool,
    check_web: bool,
) -> Result<Outcome> {
    let tools = repo
        .map(Path::to_path_buf)
        .or_else(|| crate::repo::built().filter(|r| r.join(".git").exists()))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .filter(|d| d.join("crates/gp-cli").is_dir())
        });
    let repos = Repos {
        tools,
        games: crate::roms_cmd::games_dir(None),
    };
    let pending = pending(&paths.home);
    if pending.is_empty() {
        return Ok(Outcome::ok(
            json!({"rows": []}),
            "No patch is handed in: nothing to gate.".into(),
        ));
    }
    let dir = train_dir(&paths.home);
    let mut web = |tree: &Path| real_check_web(&dir, tree);
    let fates = train(
        paths,
        &repos,
        dry_run,
        &mut Checks {
            gate: &mut real_gate,
            web: &mut web,
            check_web,
        },
    )?;
    let mut lines: Vec<String> = fates.iter().map(|(h, f)| fate_line(h, f)).collect();
    if !check_web {
        lines.push(
            "just check-web is not run (--no-check-web): a tools patch that changes the page lands on the tool gate alone".into(),
        );
    }
    let rows: Vec<Value> = fates
        .iter()
        .map(|(h, f)| {
            let (fate, words) = match f {
                Fate::Landed(c) => ("landed", c),
                Fate::Back(w) => ("back", w),
                Fate::WouldGate(g) => ("would gate", g),
            };
            json!({"game": h.game, "row": h.row, "repo": h.repo, "fate": fate, "words": words})
        })
        .collect();
    let head = if dry_run {
        format!(
            "{} handed-in patches, in the order they came (a dry run: nothing is gated):",
            fates.len()
        )
    } else {
        format!("{} handed-in patches:", fates.len())
    };
    Ok(Outcome::ok(
        json!({"rows": rows, "dry_run": dry_run, "check_web": check_web}),
        format!("{head}\n{}", lines.join("\n")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up home with game rooftop's quest log, and a made-up repo
    /// whose file `list.txt` has the lines a..f.
    fn rig(d: &Path) -> (gp_evidence::Paths, PathBuf) {
        let home = d.join("home");
        let paths = gp_evidence::Paths {
            store: home.join("store"),
            roms: home.join("roms"),
            home,
        };
        let q: Vec<Value> = ["one", "two", "three", "four"]
            .iter()
            .map(|r| json!({"mechanic": r, "status": "next", "repo": "tools", "note": "work"}))
            .collect();
        write_queue(&queue_file(&paths, "rooftop").unwrap(), &q).unwrap();
        let repo = d.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]).unwrap();
        fs::write(repo.join("list.txt"), "a\nb\nc\nd\ne\nf\n").unwrap();
        git(&repo, &["add", "-A"]).unwrap();
        git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@localhost",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        )
        .unwrap();
        (paths, repo)
    }

    /// A patch made in a worktree of the repo's tip, as a worker makes
    /// it: `list.txt` as `body` and `other` files as given.
    fn patch_of(d: &Path, repo: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let wt = d.join(format!("wt-{name}"));
        git(
            repo,
            &["worktree", "add", "-q", "--detach", &wt.to_string_lossy()],
        )
        .unwrap();
        for (f, body) in files {
            if let Some(dir) = wt.join(f).parent() {
                fs::create_dir_all(dir).unwrap();
            }
            fs::write(wt.join(f), body).unwrap();
        }
        git(&wt, &["add", "-A"]).unwrap();
        let diff = git(&wt, &["diff", "--cached"]).unwrap();
        let p = wt.join("work.patch");
        fs::write(&p, format!("{diff}\n")).unwrap();
        p
    }

    fn hand(paths: &gp_evidence::Paths, row: &str, p: &Path) {
        hand_in(paths, "rooftop", row, p, &format!("{row}'s work"), &[]).unwrap();
        // Distinct times, so the order is the order of the calls.
        std::thread::sleep(std::time::Duration::from_millis(3));
    }

    fn row(paths: &gp_evidence::Paths, r: &str) -> Value {
        read_queue(&queue_file(paths, "rooftop").unwrap())
            .into_iter()
            .find(|x| x["mechanic"] == json!(r))
            .unwrap()
    }

    fn repos(repo: &Path) -> Repos {
        Repos {
            tools: Some(repo.to_path_buf()),
            games: None,
        }
    }

    /// The train with `gate` and a just check-web that passes.
    fn train_t(
        paths: &gp_evidence::Paths,
        repos: &Repos,
        dry_run: bool,
        gate: &mut Gate,
    ) -> Result<Vec<(HandIn, Fate)>> {
        let mut web = |_: &Path| Ok(());
        train(
            paths,
            repos,
            dry_run,
            &mut Checks {
                gate,
                web: &mut web,
                check_web: true,
            },
        )
    }

    /// The page's paths are the ones GitHub's check-web runs on, push and
    /// pull request alike, so the two lists can't drift apart.
    #[test]
    fn the_page_paths_are_check_web_workflow_s() {
        let wf = include_str!("../../../.github/workflows/check-web.yml");
        let mut lists: Vec<Vec<&str>> = Vec::new();
        let mut cur: Option<Vec<&str>> = None;
        for l in wf.lines() {
            let t = l.trim();
            if t == "paths:" {
                cur = Some(Vec::new());
            } else if let Some(c) = cur.as_mut() {
                match t.strip_prefix("- ") {
                    Some(p) => c.push(p.trim_matches('"')),
                    None => lists.extend(cur.take()),
                }
            }
        }
        lists.extend(cur);
        assert_eq!(lists.len(), 2, "push and pull_request each list paths");
        for l in &lists {
            assert_eq!(l.as_slice(), PAGE_PATHS);
        }
    }

    #[test]
    fn page_paths_match_by_folder_and_name() {
        let page = |f: &str| PAGE_PATHS.iter().any(|p| path_matches(p, f));
        assert!(page("scripts/web/check-page.mjs"));
        assert!(page("crates/gp-web-player/src/lib.rs"));
        assert!(page("crates/gp-cli/src/serve/page.rs"));
        assert!(page("justfile"));
        assert!(!page("crates/gp-cli/src/loop_train.rs"));
        assert!(!page("crates/gp-webby"));
        assert!(!page("scripts/web"));
        assert!(!page("docs/web.md"));
        let p = "diff --git a/scripts/web/a.mjs b/scripts/web/a.mjs\n--- a/x\n+++ b/x\ndiff --git a/old.txt b/editor/new.txt\n";
        assert_eq!(
            patch_files(p),
            ["scripts/web/a.mjs", "old.txt", "editor/new.txt"]
        );
    }

    /// A failed just check-web is named by its failed steps.
    #[test]
    fn a_failed_check_web_names_its_steps() {
        let said = "check-step: editor 3.0 s\nboom\ncheck-step: play-web 9.1 s (failed)\nerror: Recipe `check-web` failed\n";
        assert_eq!(web_failure(said), "step play-web failed");
        assert_eq!(web_failure("a\nb\n"), "a / b");
    }

    /// Row train-runs-check-web-for-page-patches: a tools patch that
    /// changes the page makes the train run just check-web on the stack,
    /// after the tool gate's dry pass and before it lands; a stack without
    /// one runs none. A check-web failure is the gate's: tried once more,
    /// then the patch at fault found by halves, its row back to next with
    /// check-web's words while the rest land. --dry-run names the patches
    /// that would run it, and --no-check-web runs none.
    #[test]
    fn a_page_patch_runs_check_web_before_it_lands() {
        let d = tempfile::tempdir().unwrap();
        let (paths, repo) = rig(d.path());
        for (r, f) in [
            ("one", "one.txt"),
            ("two", "scripts/web/two.mjs"),
            ("three", "three.txt"),
        ] {
            let p = patch_of(d.path(), &repo, r, &[(f, r)]);
            hand(&paths, r, &p);
        }
        let mut gate_ok = |_: &Batch| Ok("abc1234".to_owned());
        let mut web_ok = |_: &Path| Ok(());

        // The dry run names the page patch and runs nothing.
        let mut web_runs = 0;
        let mut web = |_: &Path| {
            web_runs += 1;
            Ok(())
        };
        let fates = train(
            &paths,
            &repos(&repo),
            true,
            &mut Checks {
                gate: &mut gate_ok,
                web: &mut web,
                check_web: true,
            },
        )
        .unwrap();
        assert_eq!(web_runs, 0);
        assert_eq!(fates[0].1, Fate::WouldGate("the tool gate".into()));
        assert_eq!(
            fates[1].1,
            Fate::WouldGate(
                "the tool gate, then just check-web (it changes scripts/web/two.mjs)".into()
            )
        );
        let fates = train(
            &paths,
            &repos(&repo),
            true,
            &mut Checks {
                gate: &mut gate_ok,
                web: &mut web_ok,
                check_web: false,
            },
        )
        .unwrap();
        assert!(
            matches!(&fates[1].1, Fate::WouldGate(w) if w.contains("just check-web is not run: --no-check-web")),
            "{fates:?}"
        );

        // A stack check-web fails because of two: the order of the calls,
        // and two goes back with check-web's words.
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let mut gate = |b: &Batch| {
            calls
                .borrow_mut()
                .push(format!("gate {} dry={}", b.rows.len(), b.dry_run));
            Ok("feed123".to_owned())
        };
        let mut web = |tree: &Path| {
            let has_two = tree.join("scripts/web/two.mjs").is_file();
            calls.borrow_mut().push(format!("web two={has_two}"));
            if has_two {
                Err("step play-web failed".to_owned())
            } else {
                Ok(())
            }
        };
        let fates = train(
            &paths,
            &repos(&repo),
            false,
            &mut Checks {
                gate: &mut gate,
                web: &mut web,
                check_web: true,
            },
        )
        .unwrap();
        let calls = calls.into_inner();
        // The stack: dry pass, check-web fails, tried once more the same.
        assert_eq!(
            calls[..4],
            [
                "gate 3 dry=true",
                "web two=true",
                "gate 3 dry=true",
                "web two=true"
            ]
        );
        // The halves run as dry runs; a half without two runs no check-web.
        assert!(calls.contains(&"gate 1 dry=true".to_owned()), "{calls:?}");
        // The rest, no page patch among them, land in one gate with no
        // check-web.
        assert_eq!(calls.last().map(String::as_str), Some("gate 2 dry=false"));
        assert!(!calls.iter().any(|c| c == "web two=false"), "{calls:?}");
        let two = &fates.iter().find(|(h, _)| h.row == "two").unwrap().1;
        assert!(
            matches!(two, Fate::Back(w) if w.contains("just check-web fails on the stack (it changes scripts/web/two.mjs): step play-web failed")),
            "{two:?}"
        );
        for r in ["one", "three"] {
            assert_eq!(row(&paths, r)["status"], "done");
        }
        assert_eq!(row(&paths, "two")["status"], "next");

        // Handed in again with check-web passing: dry pass, check-web,
        // then the landing.
        let p = patch_of(
            d.path(),
            &repo,
            "two-again",
            &[("scripts/web/two.mjs", "2")],
        );
        hand(&paths, "two", &p);
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let mut gate = |b: &Batch| {
            calls.borrow_mut().push(format!("gate dry={}", b.dry_run));
            Ok("cafe456".to_owned())
        };
        let mut web = |_: &Path| {
            calls.borrow_mut().push("web".to_owned());
            Ok(())
        };
        train(
            &paths,
            &repos(&repo),
            false,
            &mut Checks {
                gate: &mut gate,
                web: &mut web,
                check_web: true,
            },
        )
        .unwrap();
        assert_eq!(
            calls.into_inner(),
            ["gate dry=true", "web", "gate dry=false"]
        );
        assert_eq!(row(&paths, "two")["status"], "done");

        // --no-check-web: the gate alone.
        let p = patch_of(
            d.path(),
            &repo,
            "two-third",
            &[("scripts/web/two.mjs", "3")],
        );
        hand(&paths, "two", &p);
        let mut web_never = |_: &Path| -> std::result::Result<(), String> {
            panic!("check-web ran with --no-check-web")
        };
        let mut gates = 0;
        let mut gate = |_: &Batch| {
            gates += 1;
            Ok("beef789".to_owned())
        };
        train(
            &paths,
            &repos(&repo),
            false,
            &mut Checks {
                gate: &mut gate,
                web: &mut web_never,
                check_web: false,
            },
        )
        .unwrap();
        assert_eq!(gates, 1);
    }

    /// A hand-in keeps the patch, its message, repo and tip under the
    /// train's folder and sets the row in line, handed in.
    #[test]
    fn a_hand_in_keeps_the_patch_and_sets_the_row_in_line() {
        let d = tempfile::tempdir().unwrap();
        let (paths, repo) = rig(d.path());
        let p = patch_of(d.path(), &repo, "one", &[("new.txt", "x\n")]);
        hand(&paths, "one", &p);
        let h = pending(&paths.home);
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].row.as_str(), h[0].repo.as_str()), ("one", "tools"));
        assert_eq!(h[0].tip, git(&repo, &["rev-parse", "HEAD"]).unwrap());
        assert!(fs::read_to_string(&h[0].patch).unwrap().contains("new.txt"));
        let r = row(&paths, "one");
        assert_eq!(r["status"], "in line");
        assert!(
            r["note"]
                .as_str()
                .unwrap()
                .contains("Handed in to the train")
        );
        // A second hand-in of the row replaces the first.
        hand(&paths, "one", &p);
        assert_eq!(pending(&paths.home).len(), 1);
        assert!(hand_in(&paths, "rooftop", "nope", &p, "m", &[]).is_err());
    }

    /// Three handed-in patches stack in the order they came and gate once,
    /// as one patch with every row's message; each row is done with the
    /// commit, and the hand-ins are filed away.
    #[test]
    fn three_patches_stack_in_order_and_gate_once() {
        let d = tempfile::tempdir().unwrap();
        let (paths, repo) = rig(d.path());
        for (r, f) in [("one", "1.txt"), ("two", "2.txt"), ("three", "3.txt")] {
            let p = patch_of(d.path(), &repo, r, &[(f, r)]);
            hand(&paths, r, &p);
        }
        let mut calls: Vec<(Vec<String>, String, bool)> = Vec::new();
        let mut gate = |b: &Batch| {
            let diff = fs::read_to_string(b.patch).unwrap();
            assert!(["1.txt", "2.txt", "3.txt"].iter().all(|f| diff.contains(f)));
            calls.push((b.rows.clone(), b.message.to_owned(), b.dry_run));
            Ok("c0ffee1".to_owned())
        };
        let fates = train_t(&paths, &repos(&repo), false, &mut gate).unwrap();
        assert_eq!(calls.len(), 1, "gated once");
        assert_eq!(
            calls[0].0,
            ["rooftop/one", "rooftop/two", "rooftop/three"],
            "in the order they came"
        );
        assert!(calls[0].1.contains("rooftop/two: two's work"));
        assert!(!calls[0].2);
        assert!(
            fates
                .iter()
                .all(|(_, f)| f == &Fate::Landed("c0ffee1".into()))
        );
        for r in ["one", "two", "three"] {
            let r = row(&paths, r);
            assert_eq!(
                (r["status"].as_str(), r["commit"].as_str()),
                (Some("done"), Some("c0ffee1"))
            );
        }
        assert!(pending(&paths.home).is_empty());
        // The scratch worktree is gone.
        assert!(!train_dir(&paths.home).join("stack-tools").exists());
    }

    /// A patch that does not apply on the tip is dropped and its row goes
    /// back to next with why; the rest gate. A dry run says the same and
    /// changes nothing.
    #[test]
    fn a_patch_that_does_not_apply_goes_back_to_next() {
        let d = tempfile::tempdir().unwrap();
        let (paths, repo) = rig(d.path());
        let p = patch_of(d.path(), &repo, "one", &[("1.txt", "one")]);
        hand(&paths, "one", &p);
        // A patch on a file the tip doesn't have.
        let bad = d.path().join("bad.patch");
        fs::write(
            &bad,
            "diff --git a/gone.txt b/gone.txt\n--- a/gone.txt\n+++ b/gone.txt\n@@ -1 +1 @@\n-was\n+now\n",
        )
        .unwrap();
        hand(&paths, "two", &bad);
        let mut dry_calls = 0;
        let fates = train_t(&paths, &repos(&repo), true, &mut |_: &Batch| {
            dry_calls += 1;
            Ok("x".into())
        })
        .unwrap();
        assert_eq!(dry_calls, 0, "a dry run gates nothing");
        assert_eq!(fates[0].1, Fate::WouldGate("the tool gate".into()));
        assert!(matches!(&fates[1].1, Fate::Back(w) if w.contains("doesn't apply")));
        assert_eq!(row(&paths, "two")["status"], "in line");
        assert_eq!(pending(&paths.home).len(), 2);

        let mut rows = Vec::new();
        let fates = train_t(&paths, &repos(&repo), false, &mut |b: &Batch| {
            rows.push(b.rows.clone());
            Ok("abc1234".into())
        })
        .unwrap();
        assert_eq!(rows, [["rooftop/one"]]);
        assert_eq!(fates[0].1, Fate::Landed("abc1234".into()));
        let two = row(&paths, "two");
        assert_eq!(two["status"], "next");
        assert!(
            two["note"].as_str().unwrap().contains("doesn't apply"),
            "{two}"
        );
    }

    /// Two patches that only added lines at the same place of a file keep
    /// both additions; two that change the same line don't, and the later
    /// one goes back.
    #[test]
    fn two_additions_at_one_place_keep_both() {
        let d = tempfile::tempdir().unwrap();
        let (paths, repo) = rig(d.path());
        let one = patch_of(
            d.path(),
            &repo,
            "one",
            &[("list.txt", "a\nb\nc\nONE\nd\ne\nf\n")],
        );
        let two = patch_of(
            d.path(),
            &repo,
            "two",
            &[("list.txt", "a\nb\nc\nTWO\nd\ne\nf\n")],
        );
        let three = patch_of(
            d.path(),
            &repo,
            "three",
            &[("list.txt", "a\nB\nc\nd\ne\nf\n")],
        );
        let four = patch_of(
            d.path(),
            &repo,
            "four",
            &[("list.txt", "a\nBEE\nc\nd\ne\nf\n")],
        );
        for (r, p) in [
            ("one", &one),
            ("two", &two),
            ("three", &three),
            ("four", &four),
        ] {
            hand(&paths, r, p);
        }
        let mut got = String::new();
        let fates = train_t(&paths, &repos(&repo), false, &mut |b: &Batch| {
            got = fs::read_to_string(b.patch).unwrap();
            Ok("feed123".into())
        })
        .unwrap();
        assert!(got.contains("+ONE") && got.contains("+TWO"), "{got}");
        assert!(got.contains("+B\n"), "{got}");
        let f: Vec<&Fate> = fates.iter().map(|(_, f)| f).collect();
        assert_eq!(f[0], &Fate::Landed("feed123".into()));
        assert_eq!(f[1], &Fate::Landed("feed123".into()));
        assert_eq!(f[2], &Fate::Landed("feed123".into()));
        assert!(
            matches!(f[3], Fate::Back(w) if w.contains("same lines of list.txt")),
            "{f:?}"
        );
    }

    /// A stack the gate fails is tried once more, then split in halves to
    /// find the patch at fault: its row goes back to next with the gate's
    /// words, and the rest land in one gate.
    #[test]
    fn a_failing_patch_is_found_by_halves_and_the_rest_land() {
        let d = tempfile::tempdir().unwrap();
        let (paths, repo) = rig(d.path());
        for r in ["one", "two", "three", "four"] {
            let p = patch_of(d.path(), &repo, r, &[(&format!("{r}.txt"), r)]);
            hand(&paths, r, &p);
        }
        let mut calls: Vec<(usize, bool)> = Vec::new();
        let fates = train_t(&paths, &repos(&repo), false, &mut |b: &Batch| {
            calls.push((b.rows.len(), b.dry_run));
            if b.rows.iter().any(|r| r == "rooftop/three") {
                Err("FAIL test: three broke it".into())
            } else {
                Ok("1a2b3c4".into())
            }
        })
        .unwrap();
        // The whole stack twice, its halves as dry runs, then the rest.
        assert_eq!(calls[..2], [(4, false), (4, false)]);
        assert!(calls[2..calls.len() - 1].iter().all(|(_, dry)| *dry));
        assert_eq!(calls.last(), Some(&(3, false)));
        let three = &fates.iter().find(|(h, _)| h.row == "three").unwrap().1;
        assert_eq!(three, &Fate::Back("FAIL test: three broke it".into()));
        for r in ["one", "two", "four"] {
            assert_eq!(row(&paths, r)["status"], "done");
        }
        let t = row(&paths, "three");
        assert_eq!(t["status"], "next");
        assert!(t["note"].as_str().unwrap().contains("three broke it"));
    }
}
