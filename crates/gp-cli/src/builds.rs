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
//!
//! Each gp binary makes its own module kit in work/module-kit, so every
//! worker's dev build of gp left one of 63 MB: 21 GB of them on 2026-11-25
//! (trouble-the-home-s-disk-has-gb-free-under-gb-gp-7e400a). Those no gp
//! used in a day go the same way ([`idle_module_kits`]).

use std::collections::{BTreeMap, BTreeSet};
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

pub(crate) fn step_id_ok(id: &str) -> bool {
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

/// An ended worker's worktree is kept this long after its worker's last
/// word, for a look at what it did.
const TREE_IDLE: Duration = CACHE_AGE;

/// The worktrees (work/loop/trees/ID, crate::serve::runner's worktree) of
/// workers that ended over [`TREE_IDLE`] ago: their record's state isn't a
/// live one, their step isn't in `live`, and no card open, holding a yes or
/// running names their step (a card's patch file is in its worker's tree).
/// None was ever removed: on 2026-11-26 there were 725 of them, 22 GB, with
/// the disk at 11.9 GB free (trouble-the-home-s-disk-has-gb-free-under-gb-
/// gp-8546ab). A tree with no worker record is left alone; its branch stays.
pub(crate) fn ended_worktrees(home: &Path, live: &BTreeSet<String>) -> Vec<PathBuf> {
    let now = gp_mcp::asks::now_ms();
    let idle_ms = TREE_IDLE.as_millis() as u64;
    let cards: Vec<String> = gp_mcp::asks::all(home)
        .iter()
        .filter(|a| {
            gp_mcp::asks::is_open(a, now)
                || gp_mcp::asks::is_yes(a, now)
                || a["state"] == serde_json::json!("running")
        })
        .map(|a| a.to_string())
        .collect();
    let trees = gp_mcp::steps::loop_dir(home).join("trees");
    let mut out: Vec<PathBuf> = gp_mcp::steps::workers(home)
        .iter()
        .filter(|w| !gp_mcp::steps::LIVE.contains(&w["state"].as_str().unwrap_or("")))
        .filter(|w| {
            let last = w["last_ms"]
                .as_u64()
                .or(w["started_ms"].as_u64())
                .unwrap_or(now);
            now.saturating_sub(last) >= idle_ms
        })
        .filter_map(|w| w["step"]["id"].as_str())
        .filter(|id| step_id_ok(id) && !live.contains(*id))
        .filter(|id| !cards.iter().any(|c| c.contains(*id)))
        .map(|id| trees.join(id))
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The git folder a worktree's `.git` file names, as `gitdir: DIR`.
fn worktree_gitdir(tree: &Path) -> Option<PathBuf> {
    let t = fs::read_to_string(tree.join(".git")).ok()?;
    let d = t.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    Some(tree.join(d))
}

/// Removes worktree `tree` with `git worktree remove --force` in the
/// checkout it was made from, else removes the folder and prunes the
/// checkout's record of it; its branch stays. The bytes it held.
fn remove_tree(tree: &Path) -> u64 {
    // DIR is CHECKOUT/.git/worktrees/NAME.
    let common = worktree_gitdir(tree)
        .and_then(|d| Some(d.parent()?.parent()?.to_owned()))
        .filter(|c| c.is_dir());
    let n = size(tree);
    let git = |c: &Path, args: &[&std::ffi::OsStr]| {
        let _ = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(c)
            .args(args)
            .output();
    };
    if let Some(c) = &common {
        git(
            c,
            &[
                "worktree".as_ref(),
                "remove".as_ref(),
                "--force".as_ref(),
                tree.as_os_str(),
            ],
        );
    }
    if fs::symlink_metadata(tree).is_ok() {
        if fs::remove_dir_all(tree).is_err() {
            return 0;
        }
        if let Some(c) = &common {
            git(c, &["worktree".as_ref(), "prune".as_ref()]);
        }
    }
    n
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
/// than [`CACHE_AGE`], the module kits no gp used in a day go
/// ([`idle_module_kits`]), and so do the worktrees of workers that ended a
/// day ago ([`ended_worktrees`]). The bytes freed.
pub(crate) fn clear_old_caches(home: &Path, gate_cache: Option<&Path>) -> u64 {
    let caches: u64 = regenerate_roots(home, gate_cache)
        .iter()
        .flat_map(|r| regenerate_folders(r).1)
        .map(|d| remove_old_cache(&d))
        .sum();
    caches
        + idle_module_kits(home)
            .iter()
            .map(|k| remove(k))
            .sum::<u64>()
        + ended_worktrees(home, &BTreeSet::new())
            .iter()
            .map(|t| remove_tree(t))
            .sum::<u64>()
}

/// A module kit (crate::module::own_kit) no gp has used this long belongs
/// to a gp no longer run.
const KIT_IDLE: Duration = CACHE_AGE;

/// The module kits of work/module-kit no gp has used in [`KIT_IDLE`]. Each
/// gp binary makes its own (it is named by the binary's hash), so every
/// worker's dev build of gp made one of 63 MB and none was ever removed: on
/// 2026-11-25 they were 21 GB of 330 kits, with the disk under 20 GB free.
/// A kit's Cargo.lock is touched each time a gp uses it; a kit with none
/// (a `.tmpPID` one a stopped gp left half made) goes by its folder's time.
/// The running gp's own kit is always kept; a gp whose kit went makes it
/// again.
pub(crate) fn idle_module_kits(home: &Path) -> Vec<PathBuf> {
    let Some(before) = SystemTime::now().checked_sub(KIT_IDLE) else {
        return Vec::new();
    };
    let own = crate::module::own_kit_dir(home).ok();
    let used = |p: &Path| {
        let lock = p.join("Cargo.lock");
        let m = if lock.is_file() {
            fs::metadata(&lock)
        } else {
            fs::metadata(p)
        };
        m.and_then(|m| m.modified()).ok()
    };
    let mut out: Vec<PathBuf> = fs::read_dir(home.join("work").join("module-kit"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && own.as_deref() != Some(p.as_path()))
        .filter(|p| used(p).is_some_and(|t| t < before))
        .collect();
    out.sort();
    out
}

/// The tool gate's build folders under its home (crate::gate::gate_home):
/// they build again on its next run.
fn gate_builds(gate_home: &Path) -> Vec<PathBuf> {
    let tool = gate_home.join("tool");
    vec![tool.join("target"), tool.join("target-tests")]
}

/// The build folder inside the tree a tool gate of a gp from before the two
/// locks worked in (`tool/tree/target`: the scripts' page, preview and
/// wasm builds and a debug build, 7.1 GB on 2026-11-25). A tool gate of
/// this gp works in `tool/tree-tool-lock` ([`crate::tool_gate::TREE`]), so
/// nothing builds there unless such an older gate runs, and gp loop clean
/// --gate clears it whenever none does, a tool gate of this gp running or
/// not (trouble-the-home-s-disk-has-gb-free-under-gb-gp-8546ab: the tool
/// gate ran, so the gate's builds were skipped, with 15.7 GB free).
fn old_tree_build(gate_home: &Path) -> PathBuf {
    gate_home.join("tool").join("tree").join("target")
}

/// The build folder inside the tool gate's own tree, where the scripts
/// build the page and the wasm modules and some steps run cargo in place:
/// its incremental caches are pruned with the gate's targets.
fn tree_build(gate_home: &Path) -> PathBuf {
    gate_home
        .join("tool")
        .join(crate::tool_gate::TREE)
        .join("target")
}

/// A crate's incremental folder in the gate's builds untouched this long
/// belongs to a build no gate makes now (other features, an older rustc).
const INCREMENTAL_IDLE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The tool gate's prune of its own build folders' incremental caches, run
/// at its start while it holds tool.lock, so no cargo builds in them. The
/// gate is busy most of the day, so gp loop clean --gate, which clears them
/// only while the lock is free, seldom could: they reached 45 GB, 38 GB of
/// it `incremental` (docs/gate.md, 2026-11-24). Each
/// crate's folder there keeps only its newest finished session (what the
/// next build starts from) and that session's lock; older sessions, the
/// `-working` ones a stopped lane left, a crate folder whose newest
/// session is older than [`INCREMENTAL_IDLE`], and one a newer build of
/// the same crate superseded ([`INCREMENTAL_SUPERSEDED`]: gp of an older
/// version) go. So do the build products in `deps` of the crates that
/// folder names, superseded the same way ([`prune_deps`]). The bytes freed.
pub(crate) fn prune_gate_incremental(gate_home: &Path) -> u64 {
    let now = SystemTime::now();
    let mut freed = 0;
    let mut targets = gate_builds(gate_home);
    targets.push(tree_build(gate_home));
    for target in targets {
        // TARGET/PROFILE, and TARGET/TRIPLE/PROFILE.
        for d in fs::read_dir(&target).into_iter().flatten().flatten() {
            let d = d.path();
            freed += prune_profile(&d, now);
            for p in fs::read_dir(&d).into_iter().flatten().flatten() {
                freed += prune_profile(&p.path(), now);
            }
        }
    }
    freed
}

/// One profile folder of a gate's build: its `incremental` pruned, and its
/// `deps` of the crates `incremental` names (read before the prune).
fn prune_profile(profile: &Path, now: SystemTime) -> u64 {
    let inc = profile.join("incremental");
    let names: BTreeSet<String> = fs::read_dir(&inc)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| crate_name(&e.file_name().to_string_lossy()).to_owned())
        .filter(|n| n != "build_script_build")
        .collect();
    prune_incremental(&inc, now) + prune_deps(&profile.join("deps"), &names)
}

/// A `deps` entry's crate name and cargo's 16-digit hash:
/// `libgp_cli-HASH.rlib`, `gp_cli-HASH.d` and the test binary `gp-HASH`
/// each give theirs; a name not in `names` gives none.
fn deps_entry<'a>(file: &'a str, names: &BTreeSet<String>) -> Option<(&'a str, &'a str)> {
    let stem = file.split('.').next()?;
    let (name, hash) = stem.rsplit_once('-')?;
    if hash.len() != 16 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if names.contains(name) {
        return Some((name, hash));
    }
    let name = name.strip_prefix("lib")?;
    names.contains(name).then_some((name, hash))
}

/// [`prune_gate_incremental`] on one `deps` folder: for each of the
/// workspace's crates (`names`, which `incremental` holds; no dependency
/// from crates.io builds there), the files of one hash whose newest is
/// [`INCREMENTAL_SUPERSEDED`] older than the newest of another hash of the
/// same name go. Cargo never removes them: each release's new gp-cli
/// version gives gp, libgp_cli and each of gp-cli's test binaries a new
/// hash, and the old ones stayed in the gate's two build folders, 15.4 GB
/// with 17.9 GB free on 2026-11-27, after the incremental prune
/// (trouble-the-home-s-disk-has-gb-free-under-gb-gp-18851b).
fn prune_deps(deps: &Path, names: &BTreeSet<String>) -> u64 {
    if names.is_empty() {
        return 0;
    }
    // (name, hash) -> its newest time and its entries.
    let mut builds: BTreeMap<(String, String), (Option<SystemTime>, Vec<PathBuf>)> =
        BTreeMap::new();
    for e in fs::read_dir(deps).into_iter().flatten().flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let Some((name, hash)) = deps_entry(&file, names) else {
            continue;
        };
        let at = e.metadata().and_then(|m| m.modified()).ok();
        let b = builds
            .entry((name.to_owned(), hash.to_owned()))
            .or_default();
        b.0 = b.0.max(at);
        b.1.push(e.path());
    }
    let mut newest: BTreeMap<&str, SystemTime> = BTreeMap::new();
    for ((name, _), (at, _)) in &builds {
        if let Some(at) = *at {
            let e = newest.entry(name.as_str()).or_insert(at);
            *e = (*e).max(at);
        }
    }
    let mut freed = 0;
    for ((name, _), (at, paths)) in &builds {
        let superseded = at.zip(newest.get(name.as_str())).is_some_and(|(at, top)| {
            top.duration_since(at)
                .is_ok_and(|d| d >= INCREMENTAL_SUPERSEDED)
        });
        if !superseded {
            continue;
        }
        for p in paths {
            if p.is_dir() {
                freed += remove(p);
            } else {
                let b = size(p);
                if fs::remove_file(p).is_ok() {
                    freed += b;
                }
            }
        }
    }
    freed
}

/// A crate's incremental folder whose newest session is this much older
/// than the newest of another folder of the same crate name belongs to a
/// build the gate no longer makes. Each release stamps gp-cli's version
/// into crates/gp-cli/Cargo.toml, which gives gp and every one of its
/// test binaries a new hash: on 2026-11-24 the gate's target-tests held
/// seven builds of gp (0.1.1 to 0.1.7), 9 GB of its 12 GB incremental,
/// all newer than [`INCREMENTAL_IDLE`]. A crate's lib, bin and tests
/// build together, minutes apart.
const INCREMENTAL_SUPERSEDED: Duration = Duration::from_secs(24 * 60 * 60);

/// A crate folder's name, `NAME-HASH`, without its hash.
fn crate_name(folder: &str) -> &str {
    folder.rsplit_once('-').map_or(folder, |(n, _)| n)
}

/// [`prune_gate_incremental`] on one `incremental` folder: a crate folder
/// superseded by a newer one of its name ([`INCREMENTAL_SUPERSEDED`]) goes
/// whole; every other is pruned by [`prune_crate`]. Build scripts all
/// share one name, `build_script_build`, whatever their package, so they
/// are never superseded.
fn prune_incremental(dir: &Path, now: SystemTime) -> u64 {
    let crates: Vec<(PathBuf, String, Option<SystemTime>)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| {
            let name = crate_name(&p.file_name().unwrap_or_default().to_string_lossy()).to_owned();
            let at = newest_session(&p).map(|(t, _)| t);
            (p, name, at)
        })
        .collect();
    let mut newest: BTreeMap<&str, SystemTime> = BTreeMap::new();
    for (_, name, at) in &crates {
        if let Some(at) = *at {
            let e = newest.entry(name.as_str()).or_insert(at);
            *e = (*e).max(at);
        }
    }
    crates
        .iter()
        .map(|(krate, name, at)| {
            let superseded = name != "build_script_build"
                && at.zip(newest.get(name.as_str())).is_some_and(|(at, top)| {
                    top.duration_since(at)
                        .is_ok_and(|d| d >= INCREMENTAL_SUPERSEDED)
                });
            if superseded {
                remove(krate)
            } else {
                prune_crate(krate, now)
            }
        })
        .sum()
}

/// A crate folder's newest finished session: its time and folder name.
fn newest_session(krate: &Path) -> Option<(SystemTime, String)> {
    fs::read_dir(krate)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            if !n.starts_with("s-") || n.ends_with("-working") {
                return None;
            }
            Some((e.metadata().ok()?.modified().ok()?, n))
        })
        .max()
}

/// A session's stem, `s-TIME-RANDOM`, of its folder `s-TIME-RANDOM-SVH`
/// (or `-working`) or its lock `s-TIME-RANDOM.lock`.
fn session_stem(name: &str) -> String {
    let name = name.strip_suffix(".lock").unwrap_or(name);
    name.splitn(4, '-').take(3).collect::<Vec<_>>().join("-")
}

/// One crate's incremental folder: its newest finished session and that
/// session's lock kept, the rest removed; the whole folder when it has no
/// finished session or its newest is older than [`INCREMENTAL_IDLE`].
fn prune_crate(krate: &Path, now: SystemTime) -> u64 {
    let Some((at, keep)) = newest_session(krate) else {
        return remove(krate);
    };
    if now.duration_since(at).is_ok_and(|d| d >= INCREMENTAL_IDLE) {
        return remove(krate);
    }
    let stem = session_stem(&keep);
    let mut freed = 0;
    for e in fs::read_dir(krate).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let p = e.path();
        if n == keep || (n.ends_with(".lock") && session_stem(&n) == stem) {
            continue;
        }
        if p.is_dir() {
            freed += remove(&p);
        } else {
            let b = size(&p);
            if fs::remove_file(&p).is_ok() {
                freed += b;
            }
        }
    }
    freed
}

/// What gp loop clean can clear, each with its bytes, biggest first:
/// ended workers' builds, the worktrees of workers that ended a day ago,
/// module kits no gp used in a day, idle
/// regenerate folders and the cache files
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
        .chain(ended_worktrees(home, live))
        .chain(idle_module_kits(home))
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
                .chain([old_tree_build(g)])
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
    let trees = gp_mcp::steps::loop_dir(home).join("trees");
    for c in clearable(home, live, None, gate_cache) {
        if c.old_files {
            let bytes = remove_old_cache(&c.path);
            done.push(Clear { bytes, ..c });
            continue;
        }
        let bytes = if c.path.parent() == Some(trees.as_path()) {
            remove_tree(&c.path)
        } else {
            remove(&c.path)
        };
        if c.path.exists() {
            skipped.push(format!("{}: couldn't be removed", c.path.display()));
        } else {
            done.push(Clear { bytes, ..c });
        }
    }
    if let Some(g) = gate {
        let older = crate::gate_hold::older_tool_gate_runs(g);
        let old = old_tree_build(g);
        if old.is_dir() {
            if older {
                skipped.push(format!(
                    "{}: an older gp's tool gate runs now",
                    old.display()
                ));
            } else {
                let bytes = remove(&old);
                if old.exists() {
                    skipped.push(format!("{}: couldn't be removed", old.display()));
                } else {
                    done.push(Clear {
                        path: old,
                        bytes,
                        old_files: false,
                    });
                }
            }
        }
        // The tool gate holds its lock while it runs (crate::tool_gate), and
        // a tool gate of a gp from before the two locks held gate.lock.
        let free = crate::gate_hold::open_lock(g, crate::gate_hold::Lane::Tool)
            .ok()
            .filter(|f| f.try_lock().is_ok());
        match free {
            Some(_held) if !older => {
                for p in gate_builds(g).into_iter().filter(|p| p.is_dir()) {
                    let bytes = remove(&p);
                    done.push(Clear {
                        path: p,
                        bytes,
                        old_files: false,
                    });
                }
            }
            _ => skipped.push("the gate's build folders: a tool gate runs now".into()),
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
        "The home's disk has {} free, under {}. gp loop clean clears ended workers' builds, old regenerate folders, and module kits, score cache files and ended workers' worktrees older than a day (--gate also the gate's builds){}",
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
    fn ended_workers_worktrees_clear_a_day_on_unless_a_card_waits() {
        if std::env::var_os("GP_TARGET_DIR").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let home = tmp("trees");
        let repo = home.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let o = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .output()
                .unwrap();
            assert!(o.status.success(), "{args:?}: {o:?}");
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        git(&repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join("a.txt"), "a").unwrap();
        git(&repo, &["add", "a.txt"]);
        git(&repo, &["commit", "-q", "-m", "a"]);
        let trees = gp_mcp::steps::loop_dir(&home).join("trees");
        let workers = gp_mcp::steps::workers_dir(&home);
        fs::create_dir_all(&workers).unwrap();
        let now = gp_mcp::asks::now_ms();
        let day = 24 * 60 * 60 * 1000;
        let old = "s0000000000b1"; // ended two days ago: goes
        let fresh = "s0000000000b2"; // ended an hour ago: kept
        let carded = "s0000000000b3"; // its card waits: kept
        let working = "s0000000000b4"; // still working: kept
        for (id, state, last) in [
            (old, "done", now - 2 * day),
            (fresh, "done", now - 60 * 60 * 1000),
            (carded, "done", now - 2 * day),
            (working, "working", now - 2 * day),
        ] {
            let t = trees.join(id);
            git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    &format!("loop/{id}"),
                    &t.to_string_lossy(),
                    "main",
                ],
            );
            fs::write(t.join("p.patch"), vec![0u8; 500]).unwrap();
            let w = serde_json::json!({"state": state, "last_ms": last, "step": {"id": id}});
            fs::write(workers.join(format!("{id}.json")), w.to_string()).unwrap();
        }
        // No record: left alone.
        fs::create_dir_all(trees.join("s0000000000b5")).unwrap();
        let asks = gp_mcp::asks::dir(&home);
        fs::create_dir_all(&asks).unwrap();
        let card = serde_json::json!({"id": "c1", "state": "open", "made_ms": now,
            "argv": ["tool", "gate", "--patch", trees.join(carded).join("p.patch")]});
        fs::write(asks.join("c1.json"), card.to_string()).unwrap();
        assert_eq!(
            ended_worktrees(&home, &BTreeSet::new()),
            vec![trees.join(old)]
        );
        let (done, skipped) = clean(&home, &BTreeSet::new(), None, None);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(done.len(), 1, "{done:?}");
        assert_eq!(done[0].path, trees.join(old));
        assert!(done[0].bytes >= 500, "{done:?}");
        assert!(!trees.join(old).exists());
        for id in [fresh, carded, working, "s0000000000b5"] {
            assert!(trees.join(id).exists(), "{id} kept");
        }
        // The checkout forgot the worktree; its branch stays.
        let list = git(&repo, &["worktree", "list", "--porcelain"]);
        assert!(!list.contains(old), "{list}");
        assert!(list.contains(fresh), "{list}");
        git(
            &repo,
            &["rev-parse", "--verify", &format!("refs/heads/loop/{old}")],
        );
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

    /// The build in an older gp's tool tree goes while a tool gate of this
    /// gp runs, its own builds kept; the incremental caches of its own
    /// tree's build are pruned with its targets.
    #[test]
    fn the_old_tool_trees_build_clears_while_a_tool_gate_runs() {
        if std::env::var_os("GP_TARGET_DIR").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let root = tmp("old-tree");
        let g = root.join("gate");
        let t = g.join("tool").join("target");
        fs::create_dir_all(&t).unwrap();
        fs::write(t.join("f"), b"12345").unwrap();
        let old = old_tree_build(&g);
        fs::create_dir_all(old.join("debug")).unwrap();
        fs::write(old.join("debug").join("gp"), b"1234567").unwrap();
        let mine = g.join("tool/tree-tool-lock/target");
        let k = mine.join("debug/incremental/gp_web-1x");
        let hour = Duration::from_secs(60 * 60);
        file_aged(&k.join("s-old-aaa-svh1/query-cache.bin"), 11, hour);
        file_aged(
            &k.join("s-new-bbb-svh2/query-cache.bin"),
            3,
            Duration::from_secs(60),
        );
        fs::File::open(k.join("s-old-aaa-svh1"))
            .unwrap()
            .set_modified(SystemTime::now() - hour)
            .unwrap();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        assert_eq!(
            clearable(&home, &BTreeSet::new(), Some(&g), None),
            vec![Clear::of(old.clone(), 7), Clear::of(t.clone(), 5)]
        );
        let lock = crate::gate_hold::open_lock(&g, crate::gate_hold::Lane::Tool).unwrap();
        lock.try_lock().unwrap();
        let (done, skipped) = clean(&home, &BTreeSet::new(), Some(&g), None);
        assert_eq!(done, vec![Clear::of(old.clone(), 7)]);
        assert_eq!(
            skipped,
            vec!["the gate's build folders: a tool gate runs now"]
        );
        assert!(!old.exists());
        assert!(t.join("f").exists());
        assert_eq!(prune_gate_incremental(&g), 11);
        assert!(k.join("s-new-bbb-svh2/query-cache.bin").exists());
        drop(lock);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_gate_keeps_each_crates_newest_incremental_session_only() {
        let root = tmp("incr");
        let g = root.join("gate");
        let week = Duration::from_secs(8 * 24 * 60 * 60);
        let hour = Duration::from_secs(60 * 60);
        let new = Duration::from_secs(60);
        let inc = g.join("tool/target-tests/fastdev/incremental");
        // A crate with two finished sessions and one a stopped lane left.
        let k = inc.join("gp_runtime-03w7r280oi2yh");
        file_aged(&k.join("s-old-aaa-svh1/query-cache.bin"), 100, hour);
        file_aged(&k.join("s-old-aaa.lock"), 0, hour);
        file_aged(&k.join("s-new-bbb-svh2/query-cache.bin"), 30, new);
        file_aged(&k.join("s-new-bbb.lock"), 0, new);
        file_aged(&k.join("s-cut-ccc-working/x.o"), 50, new);
        file_aged(&k.join("s-cut-ccc.lock"), 0, new);
        for (s, ago) in [("s-old-aaa-svh1", hour), ("s-new-bbb-svh2", new)] {
            fs::File::open(k.join(s))
                .unwrap()
                .set_modified(SystemTime::now() - ago)
                .unwrap();
        }
        // A crate hash no build has made in a week.
        let stale = inc.join("gp_runtime-02e00mjautffv");
        file_aged(&stale.join("s-a-b-c/dep-graph.bin"), 7, week);
        fs::File::open(stale.join("s-a-b-c"))
            .unwrap()
            .set_modified(SystemTime::now() - week)
            .unwrap();
        // The other target, under a triple: only a working session.
        let w = g.join("tool/target/wasm32-unknown-unknown/fastdev/incremental/play-1x");
        file_aged(&w.join("s-t-r-working/y.o"), 9, new);
        assert_eq!(prune_gate_incremental(&g), 100 + 50 + 7 + 9);
        assert!(k.join("s-new-bbb-svh2/query-cache.bin").exists());
        assert!(k.join("s-new-bbb.lock").exists());
        assert!(!k.join("s-old-aaa-svh1").exists());
        assert!(!k.join("s-old-aaa.lock").exists());
        assert!(!k.join("s-cut-ccc-working").exists());
        assert!(!k.join("s-cut-ccc.lock").exists());
        assert!(!stale.exists());
        assert!(!w.exists());
        assert_eq!(prune_gate_incremental(&g), 0, "nothing left to prune");
        assert_eq!(
            session_stem("s-hoe1t7j0u8-0jvpybx.lock"),
            "s-hoe1t7j0u8-0jvpybx"
        );
        assert_eq!(
            session_stem("s-hoe1t7j0u8-0jvpybx-85r3edlsn4i0sp7njs9vo7nh8"),
            "s-hoe1t7j0u8-0jvpybx"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_gate_drops_a_crates_builds_a_newer_build_of_it_superseded() {
        let root = tmp("superseded");
        let g = root.join("gate");
        let inc = g.join("tool/target-tests/fastdev/incremental");
        let days = Duration::from_secs(2 * 24 * 60 * 60);
        let minutes = Duration::from_secs(10 * 60);
        let new = Duration::from_secs(60);
        let session = |krate: &str, bytes: usize, ago: Duration| {
            let s = inc.join(krate).join("s-t-r-svh");
            file_aged(&s.join("query-cache.bin"), bytes, ago);
            file_aged(&inc.join(krate).join("s-t-r.lock"), 0, ago);
            fs::File::open(&s)
                .unwrap()
                .set_modified(SystemTime::now() - ago)
                .unwrap();
        };
        // gp of an older version, two days older than the current one.
        session("gp-old0version", 40, days);
        session("mcp-old0version", 3, days);
        // The current gp: its lib and its bin, minutes apart.
        session("gp-lib0current", 20, new);
        session("gp-bin0current", 20, minutes);
        session("mcp-new0version", 2, new);
        // Two packages' build scripts, one built long ago: both stay.
        session("build_script_build-0aaa", 5, days);
        session("build_script_build-0bbb", 5, new);
        assert_eq!(prune_gate_incremental(&g), 40 + 3);
        assert!(!inc.join("gp-old0version").exists());
        assert!(!inc.join("mcp-old0version").exists());
        for k in [
            "gp-lib0current",
            "gp-bin0current",
            "mcp-new0version",
            "build_script_build-0aaa",
            "build_script_build-0bbb",
        ] {
            assert!(
                inc.join(k).join("s-t-r-svh/query-cache.bin").exists(),
                "{k}"
            );
        }
        assert_eq!(crate_name("gp_sound_bank-0yl8m1fq2a1tb"), "gp_sound_bank");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_gate_drops_an_older_versions_build_products_from_deps() {
        let root = tmp("deps");
        let g = root.join("gate");
        let days = Duration::from_secs(2 * 24 * 60 * 60);
        let minutes = Duration::from_secs(10 * 60);
        let new = Duration::from_secs(60);
        let profile = g.join("tool/target-tests/fastdev");
        let inc = profile.join("incremental");
        let deps = profile.join("deps");
        // incremental names the workspace's crates: gp-cli's lib, its bin
        // and a test, and a build script.
        for k in [
            "gp_cli-1abc",
            "gp-2abc",
            "author-3abc",
            "build_script_build-4abc",
        ] {
            let s = inc.join(k).join("s-t-r-svh");
            file_aged(&s.join("query-cache.bin"), 0, new);
            fs::File::open(&s)
                .unwrap()
                .set_modified(SystemTime::now() - new)
                .unwrap();
        }
        let old = "0123456789abcdef";
        let cur = "fedcba9876543210";
        let other = "00112233aabbccdd";
        // gp of an older version, two days before the current one.
        for f in [
            format!("libgp_cli-{old}.rlib"),
            format!("libgp_cli-{old}.rmeta"),
            format!("gp_cli-{old}.d"),
            format!("gp-{old}"),
            format!("author-{old}"),
        ] {
            file_aged(&deps.join(f), 100, days);
        }
        // An older version's bin with a dSYM folder beside it.
        file_aged(&deps.join(format!("gp-{old}.dSYM/Contents/x")), 7, days);
        fs::File::open(deps.join(format!("gp-{old}.dSYM")))
            .unwrap()
            .set_modified(SystemTime::now() - days)
            .unwrap();
        // The current gp: lib and bin minutes apart, its test.
        for (f, ago) in [
            (format!("libgp_cli-{cur}.rlib"), new),
            (format!("gp_cli-{cur}.d"), new),
            (format!("gp-{cur}"), minutes),
            (format!("gp_cli-{other}"), minutes),
            (format!("author-{cur}"), new),
        ] {
            file_aged(&deps.join(f), 10, ago);
        }
        // A dependency from crates.io, two builds days apart (two feature
        // sets): not the workspace's, so both stay; and a file with no hash.
        file_aged(&deps.join(format!("libserde-{old}.rlib")), 5, days);
        file_aged(&deps.join(format!("libserde-{cur}.rlib")), 5, new);
        file_aged(&deps.join("gp_cli.d"), 3, days);
        assert_eq!(prune_gate_incremental(&g), 5 * 100 + 7);
        for f in [
            format!("libgp_cli-{old}.rlib"),
            format!("libgp_cli-{old}.rmeta"),
            format!("gp_cli-{old}.d"),
            format!("gp-{old}"),
            format!("author-{old}"),
            format!("gp-{old}.dSYM"),
        ] {
            assert!(!deps.join(&f).exists(), "{f}");
        }
        for f in [
            format!("libgp_cli-{cur}.rlib"),
            format!("gp_cli-{cur}.d"),
            format!("gp-{cur}"),
            format!("gp_cli-{other}"),
            format!("author-{cur}"),
            format!("libserde-{old}.rlib"),
            format!("libserde-{cur}.rlib"),
            "gp_cli.d".to_owned(),
        ] {
            assert!(deps.join(&f).exists(), "{f}");
        }
        assert_eq!(prune_gate_incremental(&g), 0, "nothing left to prune");
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
        let home_run = home.join("work").join("regenerate").join("rooftop");
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

    #[test]
    fn module_kits_no_gp_used_in_a_day_are_cleared() {
        if std::env::var_os("GP_TARGET_DIR").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let root = tmp("kits");
        let home = root.join("home");
        let kits = home.join("work").join("module-kit");
        let day = Duration::from_secs(25 * 60 * 60);
        let new = Duration::from_secs(60);
        // An old gp's kit, last used two days ago.
        file_aged(&kits.join("0000old").join("Cargo.lock"), 30, day);
        file_aged(&kits.join("0000old").join("src").join("lib.rs"), 70, day);
        // A kit used a minute ago, though made long ago.
        file_aged(&kits.join("0000new").join("Cargo.lock"), 5, new);
        file_aged(&kits.join("0000new").join("src").join("lib.rs"), 9, day);
        // The running gp's own kit, its time old: it is kept anyway.
        let own = crate::module::own_kit_dir(&home).unwrap();
        file_aged(&own.join("Cargo.lock"), 4, day);
        // A half-made kit a stopped gp left, with no Cargo.lock.
        let half = kits.join("0000new.tmp123");
        file_aged(&half.join("src").join("lib.rs"), 8, day);
        fs::File::open(&half)
            .unwrap()
            .set_modified(SystemTime::now() - day)
            .unwrap();
        assert_eq!(
            idle_module_kits(&home),
            vec![kits.join("0000new.tmp123"), kits.join("0000old")]
        );
        let none = BTreeSet::new();
        let list = clearable(&home, &none, None, None);
        assert_eq!(
            list,
            vec![
                Clear::of(kits.join("0000old"), 100),
                Clear::of(kits.join("0000new.tmp123"), 8),
            ]
        );
        assert_eq!(clear_old_caches(&home, None), 108);
        assert!(!kits.join("0000old").exists());
        assert!(!half.exists());
        assert!(kits.join("0000new").join("src").join("lib.rs").exists());
        assert!(own.join("Cargo.lock").exists());
        assert!(idle_module_kits(&home).is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
