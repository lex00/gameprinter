//! The loop's build folders and the home's free disk (owner 2026-11-20,
//! loop-cleans-its-builds: 51 GB of worker targets were left behind, one
//! 3-5 GB cargo target per step, and the disk ran down to 9 GB).
//!
//! A worker of the runner builds in `ROOT/loop-ID/HASH` (crate::worker:
//! ROOT is $GP_TARGET_DIR, else $GP_HOME/work/target; `loop-ID` is its
//! branch `loop/ID`). Nothing needs that folder once the worker ended: its
//! card's gate builds in the gate's own tree from the patch file. So the
//! runner removes it when the worker's step ends, whatever it ended as,
//! and on start removes those of workers that ended while no gp serve ran.
//!
//! Workers don't share one target folder: cargo keys its fingerprints by
//! package, not by source folder, so worktrees sharing one got a gp mixed
//! from their checkouts (crate::worker, 2026-11-03). They share compiled
//! dependencies through the locked cargo's compile cache instead.
//!
//! `gp loop health` says PROBLEM when the home's disk has under
//! [`LOW_DISK`] free, with the biggest folders `gp loop clean` can clear.
//!
//! A score cache in work/regenerate (crate::scenario, one JSON per clip
//! score, in the home's and in the gate's home) that a run still touches
//! is never idle, so it was never cleared: the gate's own, work/regenerate/
//! gate, reached 47 GB (loop-clean-gate-score-cache, 2026-11-21). Its
//! files older than [`CACHE_AGE`] go instead, the folder kept: `gp loop
//! clean` clears them, and gp serve's runner every hour. A cleared score
//! is only scored again.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Under this much free disk, gp loop health says PROBLEM.
pub(crate) const LOW_DISK: u64 = 20 * 1024 * 1024 * 1024;
/// A folder of work/regenerate untouched this long belongs to no run.
const REGENERATE_IDLE: Duration = Duration::from_secs(6 * 60 * 60);
/// A cache file of a work/regenerate folder older than this is cleared.
const CACHE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// One thing gp loop clean clears: a folder whole, or with `old_files` a
/// cache folder's files older than [`CACHE_AGE`], the folder kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Clear {
    pub path: PathBuf,
    pub bytes: u64,
    pub old_files: bool,
}

impl Clear {
    fn folder(path: PathBuf) -> Clear {
        let bytes = size(&path);
        Clear {
            path,
            bytes,
            old_files: false,
        }
    }

    /// `PATH SIZE`, with what of it goes for a cache folder.
    pub(crate) fn line(&self) -> String {
        format!(
            "{} {}{}",
            self.path.display(),
            gb(self.bytes),
            if self.old_files {
                " (its files older than a day)"
            } else {
                ""
            }
        )
    }
}

/// Where workers' cargo targets go: $GP_TARGET_DIR, else
/// $GP_HOME/work/target.
pub(crate) fn target_root(home: &Path) -> PathBuf {
    std::env::var_os("GP_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("work").join("target"))
}

/// The build folder of step `id`'s worker (its branch `loop/ID`).
pub(crate) fn loop_build(root: &Path, id: &str) -> PathBuf {
    root.join(crate::worker::branch_folder(&format!("loop/{id}")))
}

fn step_id_ok(id: &str) -> bool {
    id.len() == 13 && id.starts_with('s') && id[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// The bytes under `p`, links not followed.
pub(crate) fn size(p: &Path) -> u64 {
    let Ok(m) = fs::symlink_metadata(p) else {
        return 0;
    };
    if !m.is_dir() {
        return m.len();
    }
    fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| size(&e.path()))
        .sum()
}

/// Removes `p` and returns the bytes it held; 0 when it wasn't there or
/// couldn't be removed.
fn remove(p: &Path) -> u64 {
    if fs::symlink_metadata(p).is_err() {
        return 0;
    }
    let n = size(p);
    match fs::remove_dir_all(p) {
        Ok(()) => n,
        Err(_) => 0,
    }
}

/// Removes step `id`'s worker build folder, once its worker ended.
pub(crate) fn remove_loop_build(home: &Path, id: &str) -> u64 {
    if !step_id_ok(id) {
        return 0;
    }
    remove(&loop_build(&target_root(home), id))
}

/// The build folders of workers that ended: each `loop-ID` under the
/// target root whose step no worker holds (`live`, else a worker record
/// in a live state with its process running).
pub(crate) fn ended_loop_builds(home: &Path, live: &BTreeSet<String>) -> Vec<PathBuf> {
    let held: BTreeSet<String> = gp_mcp::steps::workers(home)
        .iter()
        .filter(|w| gp_mcp::steps::holds(w))
        .filter_map(|w| w["step"]["id"].as_str().map(str::to_owned))
        .collect();
    let mut out: Vec<PathBuf> = fs::read_dir(target_root(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_prefix("loop-")
                .is_some_and(|id| step_id_ok(id) && !live.contains(id) && !held.contains(id))
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// The work/regenerate folders: the home's, and the gate's home's when
/// `gate_cache` names it.
fn regenerate_roots(home: &Path, gate_cache: Option<&Path>) -> Vec<PathBuf> {
    let mut v = vec![home.join("work").join("regenerate")];
    if let Some(g) = gate_cache {
        let r = g.join("work").join("regenerate");
        if !v.contains(&r) {
            v.push(r);
        }
    }
    v
}

/// The entries of a work/regenerate folder `root`: those no run has
/// touched in [`REGENERATE_IDLE`], and the folders still in use.
fn regenerate_folders(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let now = SystemTime::now();
    let mut idle = Vec::new();
    let mut kept = Vec::new();
    for e in fs::read_dir(root).into_iter().flatten().flatten() {
        let Ok(m) = e.metadata() else {
            continue;
        };
        let old = m
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|d| d >= REGENERATE_IDLE);
        if old {
            idle.push(e.path());
        } else if m.is_dir() {
            kept.push(e.path());
        }
    }
    idle.sort();
    kept.sort();
    (idle, kept)
}

/// Calls `f` on each file under `dir` last written before `before`, with
/// its bytes; links not followed.
fn each_old_file(dir: &Path, before: SystemTime, f: &mut dyn FnMut(&Path, u64)) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(m) = fs::symlink_metadata(e.path()) else {
            continue;
        };
        if m.is_dir() {
            each_old_file(&e.path(), before, f);
        } else if m.modified().is_ok_and(|t| t < before) {
            f(&e.path(), m.len());
        }
    }
}

fn cache_cutoff() -> SystemTime {
    SystemTime::now()
        .checked_sub(CACHE_AGE)
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// The bytes of a cache folder's files older than [`CACHE_AGE`].
fn old_cache_bytes(dir: &Path) -> u64 {
    let mut n = 0;
    each_old_file(dir, cache_cutoff(), &mut |_, b| n += b);
    n
}

/// Removes a cache folder's files older than [`CACHE_AGE`], keeping the
/// folder, and returns the bytes they held. A run reading one meanwhile
/// only scores that clip again.
fn remove_old_cache(dir: &Path) -> u64 {
    let mut n = 0;
    each_old_file(dir, cache_cutoff(), &mut |p, b| {
        if fs::remove_file(p).is_ok() {
            n += b;
        }
    });
    n
}

/// gp serve runner's hourly sweep: every work/regenerate folder still in
/// use, the home's and the gate's home's, loses its cache files older
/// than [`CACHE_AGE`]. The bytes freed.
pub(crate) fn clear_old_caches(home: &Path, gate_cache: Option<&Path>) -> u64 {
    regenerate_roots(home, gate_cache)
        .iter()
        .flat_map(|r| regenerate_folders(r).1)
        .map(|d| remove_old_cache(&d))
        .sum()
}

/// The tool gate's build folders under its home (crate::gate::gate_home):
/// they build again on its next run.
fn gate_builds(gate_home: &Path) -> Vec<PathBuf> {
    let tool = gate_home.join("tool");
    vec![tool.join("target"), tool.join("target-tests")]
}

/// What gp loop clean can clear, each with its bytes, biggest first:
/// ended workers' builds, idle regenerate folders and the cache files
/// older than a day of those in use (the home's, and with `gate_cache`
/// the gate's home's), and with `gate_builds_of` (the gate's home) the
/// gate's build folders.
pub(crate) fn clearable(
    home: &Path,
    live: &BTreeSet<String>,
    gate_builds_of: Option<&Path>,
    gate_cache: Option<&Path>,
) -> Vec<Clear> {
    let mut out: Vec<Clear> = ended_loop_builds(home, live)
        .into_iter()
        .map(Clear::folder)
        .collect();
    for root in regenerate_roots(home, gate_cache) {
        let (idle, kept) = regenerate_folders(&root);
        out.extend(idle.into_iter().map(Clear::folder));
        out.extend(kept.into_iter().filter_map(|d| {
            let bytes = old_cache_bytes(&d);
            (bytes > 0).then_some(Clear {
                path: d,
                bytes,
                old_files: true,
            })
        }));
    }
    if let Some(g) = gate_builds_of {
        out.extend(
            gate_builds(g)
                .into_iter()
                .filter(|p| p.is_dir())
                .map(Clear::folder),
        );
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.path.cmp(&b.path)));
    out
}

/// Clears what [`clearable`] lists; the gate's build folders only while no
/// gate runs (its lock is free). Each thing cleared with its bytes, and
/// what was skipped with why.
pub(crate) fn clean(
    home: &Path,
    live: &BTreeSet<String>,
    gate: Option<&Path>,
    gate_cache: Option<&Path>,
) -> (Vec<Clear>, Vec<String>) {
    let mut done = Vec::new();
    let mut skipped = Vec::new();
    for c in clearable(home, live, None, gate_cache) {
        if c.old_files {
            let bytes = remove_old_cache(&c.path);
            done.push(Clear { bytes, ..c });
            continue;
        }
        let bytes = remove(&c.path);
        if c.path.exists() {
            skipped.push(format!("{}: couldn't be removed", c.path.display()));
        } else {
            done.push(Clear { bytes, ..c });
        }
    }
    if let Some(g) = gate {
        // The tool gate holds this lock while it runs (crate::tool_gate).
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(g.join("gate.lock"));
        match lock.map(|f| f.try_lock().map(|()| f)) {
            Ok(Ok(_held)) => {
                for p in gate_builds(g).into_iter().filter(|p| p.is_dir()) {
                    let bytes = remove(&p);
                    done.push(Clear {
                        path: p,
                        bytes,
                        old_files: false,
                    });
                }
            }
            _ => skipped.push("the gate's build folders: a gate runs now".into()),
        }
    }
    (done, skipped)
}

/// Free bytes on the disk holding `p`, from `df -Pk`.
pub(crate) fn free_bytes(p: &Path) -> Option<u64> {
    let o = std::process::Command::new("df")
        .arg("-Pk")
        .arg(p)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    parse_df(&String::from_utf8_lossy(&o.stdout))
}

/// The free kilobytes of `df -Pk`'s one line, in bytes.
fn parse_df(text: &str) -> Option<u64> {
    let line = text.lines().nth(1)?;
    let kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb * 1024)
}

/// Bytes in GB, one decimal.
pub(crate) fn gb(n: u64) -> String {
    format!("{:.1} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// gp loop health's line on the disk: Ok(line) or Err(problem).
pub(crate) fn disk_line(home: &Path) -> Option<Result<String, String>> {
    let free = free_bytes(home)?;
    if free >= LOW_DISK {
        return Some(Ok(format!("{} free on the home's disk", gb(free))));
    }
    let gate = crate::gate::gate_home(None).ok();
    let top: Vec<String> = clearable(home, &BTreeSet::new(), gate.as_deref(), gate.as_deref())
        .into_iter()
        .filter(|c| c.bytes > 0)
        .take(5)
        .map(|c| c.line())
        .collect();
    Some(Err(format!(
        "The home's disk has {} free, under {}. gp loop clean clears ended workers' builds, old regenerate folders and score cache files older than a day (--gate also the gate's builds){}",
        gb(free),
        gb(LOW_DISK),
        if top.is_empty() {
            ": none to clear, so free space elsewhere.".to_owned()
        } else {
            format!(": {}.", top.join(", "))
        }
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "gp-builds-{name}-{}-{}",
            std::process::id(),
            gp_mcp::asks::now_ms()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_dfs_free_column() {
        let t = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3s5 971350180 900000000 9437184 99% /System/Volumes/Data\n";
        assert_eq!(parse_df(t), Some(9437184 * 1024));
        assert_eq!(parse_df(""), None);
    }

    #[test]
    fn a_workers_build_is_its_branchs_folder() {
        assert_eq!(
            loop_build(Path::new("/h/work/target"), "s5cf7fc0593f6"),
            Path::new("/h/work/target/loop-s5cf7fc0593f6")
        );
    }

    #[test]
    fn ended_workers_builds_are_cleared_and_live_ones_kept() {
        if std::env::var_os("GP_TARGET_DIR").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let home = tmp("ended");
        let root = home.join("work").join("target");
        let ended = "s000000000001";
        let live = "s000000000002";
        for id in [ended, live] {
            let d = loop_build(&root, id).join("abc").join("fastdev");
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("gp"), vec![0u8; 1000]).unwrap();
        }
        // Not a worker's: left alone.
        fs::create_dir_all(root.join("main").join("x")).unwrap();
        let live_ids: BTreeSet<String> = [live.to_owned()].into();
        let list = clearable(&home, &live_ids, None, None);
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].path, loop_build(&root, ended));
        assert_eq!(list[0].bytes, 1000);
        let (done, skipped) = clean(&home, &live_ids, None, None);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(done, vec![Clear::of(loop_build(&root, ended), 1000)]);
        assert!(!loop_build(&root, ended).exists());
        assert!(
            loop_build(&root, live).exists(),
            "a live worker keeps its build"
        );
        assert!(root.join("main").exists());
        // The runner's removal when a step ends.
        assert_eq!(remove_loop_build(&home, live), 1000);
        assert!(!loop_build(&root, live).exists());
        assert_eq!(remove_loop_build(&home, "../main"), 0, "only a step id");
        assert!(root.join("main").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_gates_builds_clear_only_when_asked() {
        if std::env::var_os("GP_TARGET_DIR").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let root = tmp("gate");
        let g = root.join("gate");
        let t = g.join("tool").join("target");
        fs::create_dir_all(&t).unwrap();
        fs::write(t.join("f"), b"12345").unwrap();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        assert!(clearable(&home, &BTreeSet::new(), None, Some(&g)).is_empty());
        assert_eq!(
            clearable(&home, &BTreeSet::new(), Some(&g), Some(&g)),
            vec![Clear::of(t.clone(), 5)]
        );
        let (done, _) = clean(&home, &BTreeSet::new(), Some(&g), Some(&g));
        assert_eq!(done, vec![Clear::of(t.clone(), 5)]);
        assert!(!t.exists());
        let _ = fs::remove_dir_all(&root);
    }

    impl Clear {
        fn of(path: PathBuf, bytes: u64) -> Clear {
            Clear {
                path,
                bytes,
                old_files: false,
            }
        }
    }

    /// Writes `bytes` bytes to `p`, last written `ago` before now.
    fn file_aged(p: &Path, bytes: usize, ago: Duration) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; bytes]).unwrap();
        let f = fs::File::options().write(true).open(p).unwrap();
        f.set_modified(SystemTime::now() - ago).unwrap();
    }

    #[test]
    fn a_score_cache_in_use_loses_its_files_older_than_a_day() {
        // clean would clear the real ended workers' builds there.
        if std::env::var_os("GP_TARGET_DIR").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let root = tmp("cache");
        let home = root.join("home");
        let g = root.join("gate");
        let day = Duration::from_secs(25 * 60 * 60);
        let new = Duration::from_secs(60);
        // The gate's run "gate": touched by every gate, never idle.
        let gate_run = g.join("work").join("regenerate").join("gate");
        file_aged(&gate_run.join("old-1.json"), 300, day);
        file_aged(&gate_run.join("tapes").join("old-2.json"), 200, day);
        file_aged(&gate_run.join("new.json"), 7, new);
        // The home's own run in use, with one old score.
        let home_run = home.join("work").join("regenerate").join("tapper");
        file_aged(&home_run.join("old.json"), 40, day);
        file_aged(&home_run.join("new.json"), 1, new);
        let none = BTreeSet::new();
        let list = clearable(&home, &none, None, Some(&g));
        assert_eq!(
            list,
            vec![
                Clear {
                    path: gate_run.clone(),
                    bytes: 500,
                    old_files: true
                },
                Clear {
                    path: home_run.clone(),
                    bytes: 40,
                    old_files: true
                },
            ]
        );
        assert!(list[0].line().ends_with("(its files older than a day)"));
        // Without the gate's home only the home's cache.
        assert_eq!(clearable(&home, &none, None, None).len(), 1);
        let (done, skipped) = clean(&home, &none, None, Some(&g));
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(done.iter().map(|c| c.bytes).sum::<u64>(), 540);
        assert!(gate_run.join("new.json").exists(), "a new score is kept");
        assert!(home_run.join("new.json").exists());
        assert!(!gate_run.join("old-1.json").exists());
        assert!(!gate_run.join("tapes").join("old-2.json").exists());
        assert!(!home_run.join("old.json").exists());
        assert!(gate_run.is_dir(), "the folder in use is kept");
        // The runner's hourly sweep does the same.
        file_aged(&gate_run.join("old-3.json"), 9, day);
        assert_eq!(clear_old_caches(&home, Some(&g)), 9);
        assert_eq!(clear_old_caches(&home, Some(&g)), 0);
        assert!(gate_run.join("new.json").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
