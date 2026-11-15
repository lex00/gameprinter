//! The run record's git state without a git process when the work tree is
//! as the last run saw it.
//!
//! `git status` costs about 7 ms, and every `gp` command paid it. What it
//! answers is fixed by HEAD's commit, the index, and the tracked files'
//! contents. This reads HEAD and the index itself, and compares each
//! tracked file's `lstat` with what the index holds for it, as git does
//! before it reads a file. When HEAD, the index file and every tracked
//! file's stat are what they were the last time git was asked in this
//! work tree, the answer is the one git gave then
//! (`$GP_HOME/runs/git-state.json`). Anything else asks git again: a
//! changed file, a new commit, a racily clean entry (written in the same
//! instant as the index), a layout this doesn't read (a split or sparse
//! index, submodules, a merge in progress, GIT_DIR and the like, refs in a
//! reftable, SHA-256), or no unix stat.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::run::{GitInfo, git_info_in};

/// The most work trees the cache file keeps.
const KEEP: usize = 32;

/// The most tracked files that can differ from the index for an answer to
/// be kept.
const MAX_CHANGED: usize = 64;

/// One work tree as git was last asked about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Seen {
    state: State,
    dirty: bool,
    /// Seconds since the epoch, for keeping the most recent.
    used: u64,
}

/// What git's answer depends on, read without git.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    commit: String,
    branch: Option<String>,
    /// The index file's mtime (seconds, nanoseconds), size and inode.
    index: [u64; 4],
    /// The tracked files whose stat isn't the one the index holds, with
    /// the stat they have (all ones for a missing file).
    changed: Vec<(String, [u64; 9])>,
}

/// [`crate::run::git_info`] of `dir`, answered from `cache` when the
/// work tree is as git last saw it, and kept there when git is asked.
pub fn git_info_cached(dir: &Path, cache: &Path) -> Option<GitInfo> {
    let git_info = || git_info_in(Some(dir));
    let repo = match Repo::find(dir) {
        Found::No => return None,
        Found::Unread => return git_info(),
        Found::Yes(r) => r,
    };
    let Some(state) = repo.state() else {
        return git_info();
    };
    let key = repo.gitdir.to_string_lossy().into_owned();
    let mut seen = read_cache(cache);
    if let Some(s) = seen.get(&key)
        && s.state == state
    {
        return Some(GitInfo {
            commit: state.commit,
            dirty: s.dirty,
            branch: state.branch,
        });
    }
    let info = git_info()?;
    // Kept only when git saw what was read, before and after it ran.
    if info.commit == state.commit
        && info.branch == state.branch
        && state.changed.len() <= MAX_CHANGED
        && repo.state().as_ref() == Some(&state)
    {
        let used = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        seen.insert(
            key.clone(),
            Seen {
                state,
                dirty: info.dirty,
                used,
            },
        );
        while seen.len() > KEEP {
            let oldest = seen
                .iter()
                .filter(|(k, _)| **k != key)
                .min_by_key(|(_, s)| s.used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => seen.remove(&k),
                None => break,
            };
        }
        write_cache(cache, &seen);
    }
    Some(info)
}

fn read_cache(path: &Path) -> BTreeMap<String, Seen> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Written whole and renamed into place, so a reader never sees half of
/// it; when two runs write at once, the last one stands.
fn write_cache(path: &Path, seen: &BTreeMap<String, Seen>) {
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = dir.join(format!(".git-state.{}.json", std::process::id()));
    let Ok(json) = serde_json::to_vec(seen) else {
        return;
    };
    if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

enum Found {
    /// No `.git` from here up: git finds no work tree either.
    No,
    /// One this doesn't read; git is asked.
    Unread,
    Yes(Repo),
}

struct Repo {
    work: PathBuf,
    /// The work tree's own git folder (a linked worktree's under the main
    /// one's `worktrees/`): HEAD and the index.
    gitdir: PathBuf,
    /// Where refs and the config are: `commondir`'s, or the git folder.
    common: PathBuf,
}

impl Repo {
    fn find(from: &Path) -> Found {
        const ENV: [&str; 6] = [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_CEILING_DIRECTORIES",
        ];
        if !cfg!(unix) || ENV.iter().any(|v| std::env::var_os(v).is_some()) {
            return Found::Unread;
        }
        for dir in from.ancestors() {
            let dot = dir.join(".git");
            let Ok(meta) = std::fs::symlink_metadata(&dot) else {
                continue;
            };
            let gitdir = if meta.is_dir() {
                dot
            } else if meta.is_file() {
                let Ok(text) = std::fs::read_to_string(&dot) else {
                    return Found::Unread;
                };
                let Some(to) = text.trim_end().strip_prefix("gitdir: ") else {
                    return Found::Unread;
                };
                dir.join(to)
            } else {
                return Found::Unread;
            };
            let common = match std::fs::read_to_string(gitdir.join("commondir")) {
                Ok(c) => gitdir.join(c.trim_end()),
                Err(_) => gitdir.clone(),
            };
            return Found::Yes(Repo {
                work: dir.to_path_buf(),
                gitdir,
                common,
            });
        }
        Found::No
    }

    /// What git's answer depends on, or None for anything this doesn't
    /// read as git would.
    fn state(&self) -> Option<State> {
        // Settings that move the work tree, the object format, the refs,
        // or what the index holds; and includes, which could set them.
        let config = std::fs::read_to_string(self.common.join("config"))
            .ok()?
            .to_ascii_lowercase();
        const UNREAD: [&str; 7] = [
            "worktree",
            "bare = true",
            "objectformat",
            "refstorage",
            "sparse",
            "splitindex",
            "include",
        ];
        if UNREAD.iter().any(|k| config.contains(k)) {
            return None;
        }
        // A merge, a rebase, a cherry-pick in progress: git says more.
        for busy in ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
            if self.gitdir.join(busy).exists() {
                return None;
            }
        }
        let (commit, branch) = self.head()?;
        let index_path = self.gitdir.join("index");
        let bytes = std::fs::read(&index_path).ok()?;
        let meta = std::fs::metadata(&index_path).ok()?;
        let index = stat_of(&meta);
        let changed = self.changed(&bytes, [index[2], index[3]])?;
        Some(State {
            commit,
            branch,
            index: [index[2], index[3], index[7], index[4]],
            changed,
        })
    }

    /// HEAD's commit and branch, as `git status --branch` names them.
    fn head(&self) -> Option<(String, Option<String>)> {
        let head = std::fs::read_to_string(self.gitdir.join("HEAD")).ok()?;
        let head = head.trim_end();
        let Some(name) = head.strip_prefix("ref: ") else {
            return is_oid(head).then(|| (head.to_owned(), None));
        };
        let branch = name.strip_prefix("refs/heads/")?;
        let oid = match std::fs::read_to_string(self.common.join(name)) {
            Ok(text) => text.trim_end().to_owned(),
            Err(_) => {
                let packed = std::fs::read_to_string(self.common.join("packed-refs")).ok()?;
                packed
                    .lines()
                    .filter(|l| !l.starts_with('#') && !l.starts_with('^'))
                    .find_map(|l| {
                        let (oid, r) = l.split_once(' ')?;
                        (r == name).then(|| oid.to_owned())
                    })?
            }
        };
        is_oid(&oid).then(|| (oid, Some(branch.to_owned())))
    }

    /// The tracked files whose stat isn't the index's for them, from the
    /// index's bytes (versions 2 and 3). None for an entry or extension
    /// this doesn't read, or a racily clean entry: one whose mtime isn't
    /// before the index's own (`index_mtime`), which git reads again.
    fn changed(&self, b: &[u8], index_mtime: [u64; 2]) -> Option<Vec<(String, [u64; 9])>> {
        let u32_at = |o: usize| -> Option<u32> {
            Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
        };
        let u16_at = |o: usize| -> Option<u16> {
            Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
        };
        if b.get(..4)? != b"DIRC" || b.len() < 12 + 20 {
            return None;
        }
        let version = u32_at(4)?;
        if !(2..=3).contains(&version) {
            return None;
        }
        let count = u32_at(8)?;
        let mut off = 12;
        let mut changed = Vec::new();
        for _ in 0..count {
            let mut f = [0u32; 10];
            for (i, v) in f.iter_mut().enumerate() {
                *v = u32_at(off + 4 * i)?;
            }
            let flags = u16_at(off + 60)?;
            let mut name_at = off + 62;
            if flags & 0x4000 != 0 {
                if version < 3 || u16_at(name_at)? != 0 {
                    return None;
                }
                name_at += 2;
            }
            let len = usize::from(flags & 0xfff);
            // Assume-valid, a conflict's stage, a name past 4095 bytes.
            if flags & 0x8000 != 0 || flags & 0x3000 != 0 || len == 0xfff {
                return None;
            }
            let name = std::str::from_utf8(b.get(name_at..name_at + len)?).ok()?;
            off += (name_at - off + len + 8) & !7;
            let mode = f[6];
            let kind = mode >> 12;
            // Regular files and symlinks; a gitlink is a submodule.
            if kind != 0b1000 && kind != 0b1010 {
                return None;
            }
            if [u64::from(f[2]), u64::from(f[3])] >= index_mtime {
                return None;
            }
            let held = [f[0], f[1], f[2], f[3], f[5], f[7], f[8], f[9], mode].map(u64::from);
            match std::fs::symlink_metadata(self.work.join(name)) {
                Ok(m) => {
                    let s = stat_of(&m);
                    let now = [s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], mode_of(&m)];
                    if now != held {
                        changed.push((name.to_owned(), now));
                    }
                }
                Err(_) => changed.push((name.to_owned(), [u64::MAX; 9])),
            }
        }
        // The extensions: an optional one (a capital first letter) changes
        // nothing here; a required one (`link`, `sdir`) does.
        while off + 8 <= b.len() - 20 {
            let sig = b.get(off..off + 4)?;
            if !sig[0].is_ascii_uppercase() {
                return None;
            }
            off += 8 + usize::try_from(u32_at(off + 4)?).ok()?;
        }
        Some(changed)
    }
}

fn is_oid(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

/// A file's stat as the index keeps it, each field cut to 32 bits: ctime
/// and mtime (seconds, nanoseconds), inode, uid, gid, size.
#[cfg(unix)]
fn stat_of(m: &std::fs::Metadata) -> [u64; 8] {
    use std::os::unix::fs::MetadataExt as _;
    [
        m.ctime() as u32,
        m.ctime_nsec() as u32,
        m.mtime() as u32,
        m.mtime_nsec() as u32,
        m.ino() as u32,
        m.uid(),
        m.gid(),
        m.size() as u32,
    ]
    .map(u64::from)
}

#[cfg(not(unix))]
fn stat_of(_: &std::fs::Metadata) -> [u64; 8] {
    [u64::MAX; 8]
}

/// The mode git keeps for a file: a symlink, or a regular file with or
/// without its owner's execute bit (0 for anything else).
#[cfg(unix)]
fn mode_of(m: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    if m.file_type().is_symlink() {
        0o120000
    } else if m.is_file() {
        if m.mode() & 0o100 != 0 {
            0o100755
        } else {
            0o100644
        }
    } else {
        0
    }
}

#[cfg(not(unix))]
fn mode_of(_: &std::fs::Metadata) -> u64 {
    0
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn state(dir: &Path) -> Option<State> {
        match Repo::find(dir) {
            Found::Yes(r) => r.state(),
            _ => None,
        }
    }

    /// HEAD and the index read as git reads them: the commit, the branch,
    /// no changed file in a clean tree, and the one file a change touches;
    /// a file written as the index was isn't trusted.
    #[test]
    fn reads_head_and_the_index_as_git_does() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        git(d, &["init", "-q", "-b", "trunk"]);
        std::fs::create_dir(d.join("sub")).unwrap();
        for (f, text) in [
            ("a", "one"),
            ("sub/b", "two"),
            ("x".repeat(40).as_str(), "3"),
        ] {
            std::fs::write(d.join(f), text).unwrap();
        }
        git(d, &["add", "."]);
        git(d, &["commit", "-q", "-m", "one"]);
        // Past the racy instant: the index written after the files' mtimes.
        std::thread::sleep(std::time::Duration::from_millis(20));
        git(d, &["update-index", "--really-refresh"]);
        let head = git(d, &["rev-parse", "HEAD"]);
        let s = state(d).unwrap();
        assert_eq!(s.commit, head);
        assert_eq!(s.branch.as_deref(), Some("trunk"));
        assert!(s.changed.is_empty(), "{:?}", s.changed);
        // From a folder inside the tree, the same.
        assert_eq!(state(&d.join("sub")), Some(s.clone()));

        std::fs::write(d.join("sub/b"), "changed").unwrap();
        let s2 = state(d).unwrap();
        assert_eq!(
            s2.changed.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            ["sub/b"]
        );
        std::fs::remove_file(d.join("a")).unwrap();
        assert_eq!(state(d).unwrap().changed.len(), 2);

        // Detached, and a branch only in packed-refs.
        git(d, &["checkout", "-q", "--detach"]);
        assert_eq!(state(d).unwrap().branch, None);
        git(d, &["checkout", "-q", "trunk"]);
        git(d, &["pack-refs", "--all"]);
        assert_eq!(state(d).unwrap().commit, head);

        // A linked worktree: its own HEAD and index, the main one's refs.
        let linked = tmp.path().join("linked");
        git(
            d,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                linked.to_str().unwrap(),
            ],
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        git(&linked, &["update-index", "--really-refresh"]);
        let s = state(&linked).unwrap();
        assert_eq!(
            (s.commit.as_str(), s.branch.as_deref()),
            (head.as_str(), Some("side"))
        );
        assert!(s.changed.is_empty(), "{:?}", s.changed);
    }

    /// The cache answers as git does: clean, then dirty after a change,
    /// clean again once it is undone and committed; and the second ask of
    /// an unchanged tree comes from the cache file.
    #[test]
    fn the_cached_answer_is_gits() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("w");
        std::fs::create_dir(&d).unwrap();
        let cache = tmp.path().join("cache/git-state.json");
        git(&d, &["init", "-q", "-b", "main"]);
        std::fs::write(d.join("a"), "one").unwrap();
        git(&d, &["add", "a"]);
        git(&d, &["commit", "-q", "-m", "one"]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        git(&d, &["update-index", "--really-refresh"]);
        let ask = |d: &Path| git_info_cached(d, &cache);
        let first = ask(&d).unwrap();
        assert!(!first.dirty);
        assert!(read_cache(&cache).values().any(|s| !s.dirty));
        assert_eq!(ask(&d), Some(first.clone()));
        std::fs::write(d.join("a"), "two!").unwrap();
        assert!(ask(&d).unwrap().dirty);
        assert!(ask(&d).unwrap().dirty);
        git(&d, &["commit", "-q", "-am", "two"]);
        let after = ask(&d).unwrap();
        assert!(!after.dirty);
        assert_ne!(after.commit, first.commit);
        // An answer in the cache is what an unchanged tree gets, without
        // git: a made-up one shows it. A change to a file asks git again.
        let mut seen = read_cache(&cache);
        for s in seen.values_mut() {
            s.dirty = true;
        }
        write_cache(&cache, &seen);
        assert!(ask(&d).unwrap().dirty);
        std::fs::write(d.join("a"), "three").unwrap();
        git(&d, &["commit", "-q", "-am", "three"]);
        assert!(!ask(&d).unwrap().dirty);
    }
}
