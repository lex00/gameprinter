//! The net under gp_run's writes: a write may add to the person's games
//! checkout at once, but a change to a file already there is shown to the
//! person before it stays. Before a write runs, gp_run notes the
//! checkout's tracked files that differ from git's index (`git status`)
//! and copies them aside; after it, every tracked file the call removed,
//! or changed in more than lines added (each line the file had is still
//! there, in order), is put back as it was, and the call's version is held
//! here until the person says yes on the ask that shows it. A yes puts the
//! held version in place without running the command again; a file that
//! changed again in the meantime is refused. Files the call made and lines
//! it only added stay.
//!
//! | File | What |
//! |---|---|
//! | `$GP_HOME/work/held/STEM/held.json` | `{top, command, files: [{path, kind, before_sha, after_sha, added, removed}]}`; kind is `changed` or `removed` |
//! | `$GP_HOME/work/held/STEM/before/PATH`, `after/PATH` | The file as it was before the call (put back) and as the call left it (held) |
//! | `$GP_HOME/work/held/STEM/diff.txt` | `git diff --no-index` of the two, for the person |
//!
//! A clean file is put back from git's index (`git checkout-index`, so
//! a Git LFS file comes back whole); one that already had changes, from
//! the copy made before the call. Something else writing the checkout
//! while the call runs is seen as the call's: both versions are kept, so
//! nothing is lost, and the refusal names the file. Plans first is better
//! where a command has one (crate::policy::SELF_GUARDED); this net is for
//! every other write, which can't be audited one by one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use sha2::Digest as _;

/// The most a snapshot copies of files that already had changes. Past it
/// the write is refused rather than run without the net.
pub const MAX_BACKUP_BYTES: u64 = 256 * 1024 * 1024;
/// The most of the diff a reply and a card carry.
const DIFF_SHOWN: usize = 6 * 1024;

fn git(top: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(top)
        .args(args)
        .output()
        .map_err(|e| format!("running git: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    Ok(o.stdout)
}

fn sha(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The top of the git work tree `dir` is in, if it is in one.
pub fn top_of(dir: &Path) -> Option<PathBuf> {
    let out = git(dir, &["rev-parse", "--show-toplevel"]).ok()?;
    let top = String::from_utf8(out).ok()?;
    let top = top.trim();
    (!top.is_empty()).then(|| PathBuf::from(top))
}

/// The tracked files that differ from the index or HEAD, by path from the
/// top.
fn dirty(top: &Path) -> Result<BTreeSet<String>, String> {
    let out = git(
        top,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=no",
            "--no-renames",
            "--ignore-submodules",
        ],
    )?;
    Ok(out
        .split(|b| *b == 0)
        .filter(|e| e.len() > 3)
        .map(|e| String::from_utf8_lossy(&e[3..]).into_owned())
        .collect())
}

/// The checkout before a call: each tracked file that already had changes,
/// copied aside (None: it was missing).
pub struct Snap {
    pub top: PathBuf,
    dir: PathBuf,
    before: BTreeMap<String, Option<String>>,
}

/// Notes the checkout at `top` before a call, copying its changed tracked
/// files into `dir`.
pub fn snapshot(top: &Path, dir: &Path) -> Result<Snap, String> {
    let mut before = BTreeMap::new();
    let mut total = 0u64;
    for p in dirty(top)? {
        let f = top.join(&p);
        match std::fs::read(&f) {
            Ok(bytes) => {
                total += bytes.len() as u64;
                if total > MAX_BACKUP_BYTES {
                    return Err(format!(
                        "the games checkout has more than {} MB of changes not committed, more than gp_run copies aside to watch a write; commit them, or ask the person to run the command in a terminal",
                        MAX_BACKUP_BYTES / 1024 / 1024
                    ));
                }
                let to = dir.join("before").join(&p);
                if let Some(d) = to.parent() {
                    std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
                }
                std::fs::write(&to, &bytes).map_err(|e| format!("{}: {e}", to.display()))?;
                before.insert(p, Some(sha(&bytes)));
            }
            Err(_) => {
                before.insert(p, None);
            }
        }
    }
    Ok(Snap {
        top: top.to_owned(),
        dir: dir.to_owned(),
        before,
    })
}

/// What a call did to a file already in the checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Changed,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub kind: Kind,
    /// It already had changes before the call (put back from the copy).
    from_copy: bool,
}

/// Whether every line of `old` is in `new`, in order: lines added only.
pub fn only_added(old: &[u8], new: &[u8]) -> bool {
    let (Ok(old), Ok(new)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
        return false;
    };
    let mut rest = new.lines();
    old.lines().all(|l| rest.any(|n| n == l))
}

/// The tracked files the call removed, or changed in more than lines
/// added, since `snap`.
pub fn changes(snap: &Snap) -> Result<Vec<Change>, String> {
    let now = dirty(&snap.top)?;
    let paths: BTreeSet<&String> = now.iter().chain(snap.before.keys()).collect();
    let mut out = Vec::new();
    for p in paths {
        let file = snap.top.join(p);
        let current = std::fs::read(&file).ok();
        let (old, from_copy) = match snap.before.get(p) {
            // Missing before: anything there now was added.
            Some(None) => continue,
            Some(Some(was)) => {
                if current.as_deref().map(sha).as_deref() == Some(was.as_str()) {
                    continue;
                }
                let copy = std::fs::read(snap.dir.join("before").join(p))
                    .map_err(|e| format!("reading the copy of {p}: {e}"))?;
                (copy, true)
            }
            // Clean before: it was the index's.
            None => (git(&snap.top, &["show", &format!(":{p}")])?, false),
        };
        match current {
            None => out.push(Change {
                path: p.clone(),
                kind: Kind::Removed,
                from_copy,
            }),
            Some(new) if !only_added(&old, &new) => out.push(Change {
                path: p.clone(),
                kind: Kind::Changed,
                from_copy,
            }),
            Some(_) => {}
        }
    }
    Ok(out)
}

fn numstat(a: &Path, b: &Path) -> (Value, Value) {
    let o = Command::new("git")
        .args(["diff", "--no-index", "--numstat", "--"])
        .arg(a)
        .arg(b)
        .output();
    let text = o
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let mut it = text.split('\t');
    let n = |s: Option<&str>| {
        s.and_then(|s| s.trim().parse::<u64>().ok())
            .map_or(Value::Null, |n| json!(n))
    };
    (n(it.next()), n(it.next()))
}

/// Puts each changed file back as it was before the call and keeps the
/// call's version in `held` (a folder of its own), with a diff; returns
/// the record ([`apply`] reads it).
pub fn hold(snap: &Snap, list: &[Change], command: &str, held: &Path) -> Result<Value, String> {
    let mut files = Vec::new();
    let mut diff = String::new();
    for c in list {
        let file = snap.top.join(&c.path);
        let after = held.join("after").join(&c.path);
        let before = held.join("before").join(&c.path);
        for d in [after.parent(), before.parent()].into_iter().flatten() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let after_sha = match std::fs::read(&file) {
            Ok(bytes) => {
                std::fs::write(&after, &bytes).map_err(|e| format!("{}: {e}", after.display()))?;
                Some(sha(&bytes))
            }
            Err(_) => None,
        };
        if c.from_copy {
            let copy = snap.dir.join("before").join(&c.path);
            std::fs::copy(&copy, &file).map_err(|e| format!("putting back {}: {e}", c.path))?;
        } else {
            git(&snap.top, &["checkout-index", "-f", "--", &c.path])
                .map_err(|e| format!("putting back {}: {e}", c.path))?;
        }
        let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        std::fs::write(&before, &bytes).map_err(|e| format!("{}: {e}", before.display()))?;
        let (added, removed) = if after_sha.is_some() {
            numstat(&before, &after)
        } else {
            (json!(0), Value::Null)
        };
        if diff.len() < DIFF_SHOWN * 4 {
            let other = if after_sha.is_some() {
                after.clone()
            } else {
                PathBuf::from("/dev/null")
            };
            if let Ok(o) = Command::new("git")
                .args(["diff", "--no-index", "--no-color", "--unified=1", "--"])
                .arg(&before)
                .arg(&other)
                .output()
            {
                let text =
                    String::from_utf8_lossy(&o.stdout).replace(&format!("{}/", held.display()), "");
                diff.push_str(&format!("# {}\n{text}", c.path));
            }
        }
        files.push(json!({
            "path": c.path,
            "kind": match c.kind { Kind::Changed => "changed", Kind::Removed => "removed" },
            "before_sha": sha(&bytes),
            "after_sha": after_sha,
            "added": added,
            "removed": removed,
        }));
    }
    std::fs::write(held.join("diff.txt"), &diff).map_err(|e| format!("writing the diff: {e}"))?;
    let v = json!({
        "dir": held,
        "top": snap.top,
        "command": command,
        "files": files,
    });
    std::fs::write(
        held.join("held.json"),
        serde_json::to_string_pretty(&v).expect("JSON"),
    )
    .map_err(|e| format!("writing the held record: {e}"))?;
    Ok(v)
}

/// The diff of a held change, cut to what a reply carries.
pub fn diff_shown(held: &Value) -> String {
    let Some(dir) = held["dir"].as_str() else {
        return String::new();
    };
    let text = std::fs::read_to_string(Path::new(dir).join("diff.txt")).unwrap_or_default();
    if text.len() <= DIFF_SHOWN {
        return text;
    }
    let mut n = DIFF_SHOWN;
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    format!(
        "{}\n... (cut; the whole diff is in {dir}/diff.txt)",
        &text[..n]
    )
}

/// Each held file in words: `rooftop/corpus.toml (3 lines removed or
/// changed, 40 added)`.
pub fn each_in_words(held: &Value) -> Vec<String> {
    held["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let path = f["path"].as_str().unwrap_or("?");
            if f["kind"] == json!("removed") {
                return format!("{path} removed");
            }
            match (f["removed"].as_u64(), f["added"].as_u64()) {
                (Some(r), Some(a)) => format!(
                    "{path} ({r} line{} removed or changed, {a} added)",
                    if r == 1 { "" } else { "s" }
                ),
                _ => format!("{path} changed (not text)"),
            }
        })
        .collect()
}

/// Puts a held change in place: every file must still be as it was put
/// back, or nothing is written. Returns the files written.
pub fn apply(held: &Value) -> Result<Vec<String>, String> {
    let (Some(dir), Some(top)) = (held["dir"].as_str(), held["top"].as_str()) else {
        return Err("the ask holds no change".into());
    };
    let (dir, top) = (Path::new(dir), Path::new(top));
    let files = held["files"].as_array().cloned().unwrap_or_default();
    let mut moved = Vec::new();
    for f in &files {
        let path = f["path"].as_str().unwrap_or_default();
        let now = std::fs::read(top.join(path)).ok().map(|b| sha(&b));
        if now.as_deref() != f["before_sha"].as_str() {
            moved.push(path.to_owned());
        }
    }
    if !moved.is_empty() {
        return Err(format!(
            "{} changed since the change was held, so the held change no longer fits; nothing was written. Run the command again for a new one",
            moved.join(", ")
        ));
    }
    let mut done = Vec::new();
    for f in &files {
        let path = f["path"].as_str().unwrap_or_default();
        let to = top.join(path);
        if f["after_sha"].is_null() {
            std::fs::remove_file(&to).map_err(|e| format!("removing {path}: {e}"))?;
        } else {
            std::fs::copy(dir.join("after").join(path), &to)
                .map_err(|e| format!("writing {path}: {e}"))?;
        }
        done.push(path.to_owned());
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(top: &Path, args: &[&str]) {
        let o = Command::new("git")
            .arg("-C")
            .arg(top)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }

    /// A checkout with two committed files and one changed, not committed.
    fn checkout(d: &Path) -> PathBuf {
        let top = d.join("games");
        std::fs::create_dir_all(top.join("g")).unwrap();
        g(&top, &["init", "-q"]);
        g(&top, &["config", "user.email", "t@example.com"]);
        g(&top, &["config", "user.name", "t"]);
        g(&top, &["config", "core.hooksPath", "/dev/null"]);
        std::fs::write(top.join("g/corpus.toml"), "a\nb\nc\n").unwrap();
        std::fs::write(top.join("g/model.toml"), "x = 1\n").unwrap();
        std::fs::write(top.join("g/notes.md"), "one\n").unwrap();
        g(&top, &["add", "-A"]);
        g(&top, &["commit", "-q", "-m", "base"]);
        std::fs::write(top.join("g/notes.md"), "one\ntwo, by hand\n").unwrap();
        top
    }

    #[test]
    fn lines_added_stay_and_a_change_is_put_back_and_held_until_applied() {
        let d = tempfile::tempdir().unwrap();
        let top = checkout(d.path());
        assert_eq!(
            top_of(&top.join("g")).unwrap().canonicalize().unwrap(),
            top.canonicalize().unwrap()
        );
        assert!(top_of(d.path()).is_none());
        let snap = snapshot(&top, &d.path().join("snap")).unwrap();
        // The call: adds lines to one file, makes a new one, shortens
        // corpus.toml, removes model.toml and rewrites the hand edit.
        std::fs::write(top.join("g/corpus.toml"), "a\nc\n").unwrap();
        std::fs::remove_file(top.join("g/model.toml")).unwrap();
        std::fs::write(top.join("g/notes.md"), "one\n").unwrap();
        std::fs::write(top.join("g/new.toml"), "n\n").unwrap();
        let list = changes(&snap).unwrap();
        let shown: Vec<(&str, &Kind)> = list.iter().map(|c| (c.path.as_str(), &c.kind)).collect();
        assert_eq!(
            shown,
            [
                ("g/corpus.toml", &Kind::Changed),
                ("g/model.toml", &Kind::Removed),
                ("g/notes.md", &Kind::Changed)
            ]
        );
        let held = hold(&snap, &list, "gp t", &d.path().join("held")).unwrap();
        // Put back as they were, the hand edit too; the new file stays.
        assert_eq!(
            std::fs::read_to_string(top.join("g/corpus.toml")).unwrap(),
            "a\nb\nc\n"
        );
        assert_eq!(
            std::fs::read_to_string(top.join("g/model.toml")).unwrap(),
            "x = 1\n"
        );
        assert_eq!(
            std::fs::read_to_string(top.join("g/notes.md")).unwrap(),
            "one\ntwo, by hand\n"
        );
        assert!(top.join("g/new.toml").exists());
        let words = each_in_words(&held);
        assert_eq!(
            words[0],
            "g/corpus.toml (1 line removed or changed, 0 added)"
        );
        assert_eq!(words[1], "g/model.toml removed");
        assert!(diff_shown(&held).contains("-b"), "{}", diff_shown(&held));
        // Applied: the call's versions.
        assert_eq!(apply(&held).unwrap().len(), 3);
        assert_eq!(
            std::fs::read_to_string(top.join("g/corpus.toml")).unwrap(),
            "a\nc\n"
        );
        assert!(!top.join("g/model.toml").exists());
        // Applied again, the files no longer are as they were put back.
        let e = apply(&held).unwrap_err();
        assert!(e.contains("changed since"), "{e}");
    }

    #[test]
    fn only_lines_added_is_an_addition() {
        assert!(only_added(b"a\nb\n", b"a\nnew\nb\nmore\n"));
        assert!(!only_added(b"a\nb\n", b"b\na\n"));
        assert!(!only_added(b"a\nb\n", b"a\n"));
        assert!(!only_added(b"version = 1\n", b"version = 2\n"));
        assert!(!only_added(&[0xff, 0], &[0xff, 0, 1]));
    }
}
