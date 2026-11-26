//! A new tools worker's build folder starts warm (row
//! workers-start-from-a-warm-build, 2026-11-22). A worker's target folder
//! (crate::worker, `ROOT/loop-ID/HASH`) used to start empty, so its first
//! `cargo build` compiled everything: 560 s measured on 2026-11-22. gp
//! serve's runner now makes it, when it makes the worker's worktree, as a
//! copy-on-write clone of the tool gate's own target (`<gate home>/tool/
//! target`, built from main's tip, crate::tool_gate): `cp -c` (clonefile)
//! on macOS, `cp --reflink=always` on Linux, so the clone costs no disk
//! until a build changes a file. Where neither works, or a tool gate is
//! building in its target now, the folder starts empty as before, and the
//! worker's record says why.
//!
//! A clone alone isn't enough: cargo rebuilds a workspace crate whose
//! source files are newer than its last build, and a new worktree's files
//! are all written now. So each file of the worktree that is byte for byte
//! the gate tree's takes the gate tree's file's modified time: cargo then
//! compares exactly what it compares in the gate's tree, and finds fresh
//! what the gate built from those bytes. A file that differs keeps its own
//! (new) time and is built again, so the clone never gives a gp built from
//! other sources than the worktree's.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::{Value, json};

/// The marker a warm start leaves in the folder it made, read by `gp worker
/// env` to say so.
pub(crate) const MARKER: &str = ".gp-warm.json";

/// Clones the folder `src` to `dst` (which must not exist) copy-on-write,
/// keeping files' times. `Err` says why not; nothing is left at `dst` then.
pub(crate) fn clone_dir(src: &Path, dst: &Path) -> Result<(), String> {
    let mut cmd = Command::new("cp");
    if cfg!(target_os = "macos") {
        // -c: clonefile(2), which fails where the disk can't clone (not APFS).
        cmd.args(["-c", "-R", "-p"]);
    } else if cfg!(target_os = "linux") {
        // always, not auto: auto falls back to a whole copy, 7 GB a worker.
        cmd.args(["-a", "--reflink=always"]);
    } else {
        return Err("no copy-on-write clone on this system".into());
    }
    let o = cmd
        .arg(src)
        .arg(dst)
        .output()
        .map_err(|e| format!("running cp: {e}"))?;
    if o.status.success() {
        return Ok(());
    }
    let _ = fs::remove_dir_all(dst);
    let why = String::from_utf8_lossy(&o.stderr);
    Err(format!(
        "cp couldn't clone it: {}",
        why.lines().next().unwrap_or("").trim()
    ))
}

/// The files git tracks in `tree`, by their paths from it.
fn tracked(tree: &Path) -> Vec<String> {
    let Ok(o) = Command::new("git")
        .arg("-C")
        .arg(tree)
        .args(["ls-files", "-z"])
        .output()
    else {
        return Vec::new();
    };
    if !o.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&o.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Gives each tracked file of `tree` that is byte for byte the same file
/// of `like` the modified time of `like`'s; the others keep theirs. The
/// files given it.
pub(crate) fn age_like(tree: &Path, like: &Path) -> usize {
    let mut n = 0;
    for rel in tracked(tree) {
        let (mine, theirs) = (tree.join(&rel), like.join(&rel));
        let (Ok(m), Ok(t)) = (fs::symlink_metadata(&mine), fs::symlink_metadata(&theirs)) else {
            continue;
        };
        if !m.is_file() || !t.is_file() || m.len() != t.len() {
            continue;
        }
        let Ok(when) = t.modified() else {
            continue;
        };
        if m.modified().is_ok_and(|w| w == when) {
            n += 1;
            continue;
        }
        if fs::read(&mine).ok() != fs::read(&theirs).ok() {
            continue;
        }
        if fs::File::options()
            .write(true)
            .open(&mine)
            .and_then(|f| f.set_modified(when))
            .is_ok()
        {
            n += 1;
        }
    }
    n
}

/// Starts `dst`, the build folder of a worker in `worktree`, as a clone of
/// the target `src` the tool gate built from its tree `src_tree`, while
/// the tool gate's lock in `gate_home` is free (a gate building there now
/// would give a clone half written). What it did, for the worker's record:
/// `{"warm": true, ...}`, or `{"warm": false, "why": ...}` with the
/// folder left to start empty.
pub(crate) fn start(gate_home: &Path, dst: &Path, worktree: &Path) -> Value {
    let tool = gate_home.join("tool");
    // The tree the gate builds its target from (crate::tool_gate::TREE),
    // not `tool/tree`, an older gp's, whose files and tip may be days old.
    let (src, src_tree) = (tool.join("target"), tool.join(crate::tool_gate::TREE));
    let cold = |why: &str| json!({"warm": false, "why": why, "target": dst});
    if dst.join(MARKER).is_file() || fs::read_dir(dst).is_ok_and(|mut d| d.next().is_some()) {
        return cold("the worker's build folder already has a build");
    }
    if !src.join(crate::worker::PROFILE).is_dir() {
        return cold("the tool gate has no build to start from yet");
    }
    let Ok(lock) = crate::gate_hold::open_lock(gate_home, crate::gate_hold::Lane::Tool) else {
        return cold("the tool gate's lock can't be opened");
    };
    if lock.try_lock().is_err() || crate::gate_hold::older_tool_gate_runs(gate_home) {
        return cold("a tool gate is building in its target now");
    }
    let t = Instant::now();
    let Some(parent) = dst.parent() else {
        return cold("the build folder has no parent");
    };
    if fs::create_dir_all(parent).is_err() {
        return cold("the build folder's parent can't be made");
    }
    // Cloned beside it, then moved in, so no build sees half a clone.
    let part: PathBuf = dst.with_extension("warming");
    let _ = fs::remove_dir_all(&part);
    let _ = fs::remove_dir(dst);
    if let Err(why) = clone_dir(&src, &part) {
        return cold(&why);
    }
    let aged = age_like(worktree, &src_tree);
    let tip = crate::gate::git(&src_tree, &["rev-parse", "HEAD"]).unwrap_or_default();
    let said = json!({
        "warm": true,
        "from": src,
        "tree_tip": tip,
        "target": dst,
        "files_aged": aged,
        "secs": (t.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    });
    let _ = fs::write(part.join(MARKER), said.to_string());
    if let Err(e) = fs::rename(&part, dst) {
        let _ = fs::remove_dir_all(&part);
        return cold(&format!("the clone couldn't be moved in: {e}"));
    }
    drop(lock);
    said
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "gp-warm-{name}-{}-{}",
            std::process::id(),
            gp_mcp::asks::now_ms()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn git(dir: &Path, args: &[&str]) {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }

    fn repo(dir: &Path, files: &[(&str, &str)]) {
        fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q"]);
        for (f, text) in files {
            let p = dir.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        git(dir, &["add", "-A"]);
    }

    fn set_time(p: &Path, t: SystemTime) {
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    fn time(p: &Path) -> SystemTime {
        fs::metadata(p).unwrap().modified().unwrap()
    }

    /// A worktree's file the same as the gate tree's takes its time, so
    /// cargo finds the gate's build of it fresh; one that differs keeps its
    /// own and is built again.
    #[test]
    fn only_files_the_same_as_the_gates_take_its_time() {
        let root = tmp("age");
        let gate = root.join("gate");
        let mine = root.join("mine");
        repo(
            &gate,
            &[("src/a.rs", "fn a() {}"), ("src/b.rs", "fn b() {}")],
        );
        repo(
            &mine,
            &[("src/a.rs", "fn a() {}"), ("src/b.rs", "fn b() { 1; }")],
        );
        let old = SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60);
        set_time(&gate.join("src/a.rs"), old);
        set_time(&gate.join("src/b.rs"), old);
        let b_before = time(&mine.join("src/b.rs"));
        assert_eq!(age_like(&mine, &gate), 1);
        assert_eq!(
            time(&mine.join("src/a.rs")),
            old,
            "the same bytes: the gate's time"
        );
        assert_eq!(
            time(&mine.join("src/b.rs")),
            b_before,
            "changed: its own time"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// The runner's start: the gate's target cloned in whole with its
    /// files' times, the marker written; the worktree's files aged like the
    /// tree the gate builds from, not an older gp's `tool/tree`; a folder
    /// that has a build is left alone, and with no gate build it starts
    /// empty saying why.
    #[test]
    fn a_workers_build_folder_starts_as_a_clone_of_the_gates() {
        let root = tmp("start");
        let gate = root.join("gate");
        let target = gate.join("tool").join("target");
        let dep = target.join("fastdev").join("deps").join("libx.rlib");
        let mine = root.join("mine");
        let gate_tree = gate.join("tool").join(crate::tool_gate::TREE);
        repo(&gate_tree, &[("a.rs", "x"), ("b.rs", "new")]);
        // An older gp's tree, left beside it: b.rs as it was then.
        repo(
            &gate.join("tool").join("tree"),
            &[("a.rs", "x"), ("b.rs", "old")],
        );
        repo(&mine, &[("a.rs", "x"), ("b.rs", "new")]);
        let tree_old = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        set_time(&gate_tree.join("b.rs"), tree_old);
        let dst = root.join("work").join("target").join("loop-s1").join("abc");
        let none = start(&gate, &dst, &mine);
        assert_eq!(none["warm"], json!(false), "{none}");
        assert!(none["why"].as_str().unwrap().contains("no build"));
        fs::create_dir_all(dep.parent().unwrap()).unwrap();
        fs::write(&dep, b"built").unwrap();
        let old = SystemTime::now() - Duration::from_secs(60 * 60);
        set_time(&dep, old);
        let said = start(&gate, &dst, &mine);
        if !(cfg!(target_os = "macos") || cfg!(target_os = "linux")) || said["warm"] != json!(true)
        {
            // A disk that can't clone (tmpfs, ext4): it starts empty, saying why.
            assert!(said["why"].as_str().unwrap().contains("clone"), "{said}");
            assert!(!dst.exists());
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let built = dst.join("fastdev").join("deps").join("libx.rlib");
        assert_eq!(fs::read(&built).unwrap(), b"built");
        assert_eq!(time(&built), old, "its build times kept");
        assert!(dst.join(MARKER).is_file());
        assert_eq!(said["files_aged"], json!(2), "{said}");
        assert_eq!(
            time(&mine.join("b.rs")),
            tree_old,
            "aged like the gate's own tree"
        );
        assert!(!dst.with_extension("warming").exists());
        let again = start(&gate, &dst, &mine);
        assert_eq!(again["warm"], json!(false));
        assert!(again["why"].as_str().unwrap().contains("already"));
        let _ = fs::remove_dir_all(&root);
    }

    /// While a tool gate holds its lock, building in its target, the
    /// worker's folder starts empty.
    #[test]
    fn no_clone_while_a_tool_gate_builds() {
        let root = tmp("busy");
        let gate = root.join("gate");
        fs::create_dir_all(gate.join("tool").join("target").join("fastdev")).unwrap();
        let held = crate::gate_hold::open_lock(&gate, crate::gate_hold::Lane::Tool).unwrap();
        held.lock().unwrap();
        let dst = root.join("t").join("abc");
        let said = start(&gate, &dst, &root);
        assert_eq!(said["warm"], json!(false));
        assert!(said["why"].as_str().unwrap().contains("building"), "{said}");
        drop(held);
        let _ = fs::remove_dir_all(&root);
    }
}
