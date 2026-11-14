//! Scoring a set of clips on several threads.
//!
//! Each clip's score reads the clip, the model and the bench and nothing
//! else, so the clips of a set can run on any threads in any order. [`map`]
//! hands the items of a list to helper threads and gives the results back
//! by index, so the output is the same for any number of threads.
//!
//! The helpers are shared by the whole process: at most `jobs()` of them
//! run at once, whichever call asked for them. A call that finds none free
//! runs its items on its own thread, one after another, as before. A call
//! made on a helper thread runs its items on that thread.
//!
//! The helpers are shared by the machine too. Every gp process with
//! helpers working leaves a locked file in `~/.gameprinter/work/helpers`
//! (from `$HOME`, not `$GP_HOME`, so the gate's canary homes count as well;
//! the kernel drops the lock when a process dies). Each process lets at
//! most every core less two, divided by the processes there, of its
//! helpers run an item at once, and looks again about once a second: three
//! scorers on 18 cores run 5 helpers each, not 16 each. A helper over the
//! share waits before its next item. `--jobs` and `GP_JOBS` stay the cap
//! for one process.
//!
//! | Setting | What |
//! |---|---|
//! | `--jobs N` | Any command: up to N helper threads (1 runs everything on the command's own thread) |
//! | `GP_JOBS=N` | The same, for a command or `gp serve` started without `--jobs` |
//! | Neither | Every core less two, at least 1 |

use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// The helper threads running now.
static USED: Mutex<usize> = Mutex::new(0);

/// The helpers running an item now: at most [`limit`] of them.
static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// The setting `--jobs` gave, if any (0: none).
static SET: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Whether this thread is one of [`map`]'s helpers.
    static HELPER: Cell<bool> = const { Cell::new(false) };
}

/// Sets how many helper threads score clips, for `--jobs`.
pub(crate) fn set_jobs(n: usize) {
    let n = n.clamp(1, 256);
    SET.store(n, Ordering::Relaxed);
}

/// How many helper threads score clips: `--jobs`, else `GP_JOBS`, else
/// every core less two, at least one.
pub(crate) fn jobs() -> usize {
    explicit().unwrap_or_else(default_jobs)
}

/// The number `--jobs` or `GP_JOBS` gave, if either did.
pub(crate) fn explicit() -> Option<usize> {
    match SET.load(Ordering::Relaxed) {
        0 => std::env::var("GP_JOBS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
            .map(|n| n.min(256)),
        n => Some(n),
    }
}

fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .saturating_sub(2)
        .max(1)
}

/// This process's share of the machine's helpers: every core less two
/// divided by the gp processes with helpers working (this one among
/// them), at least one.
fn share(processes: usize) -> usize {
    (default_jobs() / processes.max(1)).max(1)
}

/// How many helpers may run an item at once now: `jobs()`, and at most
/// this process's share of the machine. At least one.
fn limit() -> usize {
    jobs().min(share(machine::processes())).max(1)
}

/// Takes up to `want` helpers; gives back how many it got.
fn take(want: usize) -> usize {
    let mut used = USED.lock().unwrap_or_else(|e| e.into_inner());
    let limit = match jobs() {
        0 | 1 => 0,
        n => n,
    };
    let got = want.min(limit.saturating_sub(*used));
    *used += got;
    got
}

/// Helpers taken, given back when dropped.
struct Taken(usize);

impl Drop for Taken {
    fn drop(&mut self) {
        let mut used = USED.lock().unwrap_or_else(|e| e.into_inner());
        *used = used.saturating_sub(self.0);
    }
}

/// A helper's turn to run one item, counted in [`RUNNING`] until dropped.
struct Turn;

impl Turn {
    /// Waits until fewer than [`limit`] helpers run an item, then takes a
    /// turn. `None` once `next` is past the list: nothing is left to run.
    fn wait(next: &AtomicUsize, len: usize) -> Option<Turn> {
        loop {
            if next.load(Ordering::Relaxed) >= len {
                return None;
            }
            let running = RUNNING.load(Ordering::Relaxed);
            if running < limit()
                && RUNNING
                    .compare_exchange(running, running + 1, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
            {
                return Some(Turn);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::AcqRel);
    }
}

/// `f` on every item, on as many helper threads as are free and the list
/// needs, each with its own state from `init`; the results in the list's
/// order. `started` hears each item's index as it starts, on the calling
/// thread (a little after, when helpers run it). With one item, no helper
/// free, or called on a helper, it all runs on the calling thread in order.
pub(crate) fn map<T, S, R>(
    items: &[T],
    init: impl Fn() -> S + Sync,
    f: impl Fn(&mut S, usize, &T) -> R + Sync,
    started: &mut dyn FnMut(usize),
) -> Vec<R>
where
    T: Sync,
    R: Send,
{
    // A helper waiting on helpers of its own would hold a turn they need.
    let n = if items.len() > 1 && !HELPER.with(Cell::get) {
        take(items.len())
    } else {
        0
    };
    let taken = Taken(n);
    // One helper is no faster than the calling thread.
    if n <= 1 {
        drop(taken);
        let mut s = init();
        return items
            .iter()
            .enumerate()
            .map(|(i, x)| {
                started(i);
                f(&mut s, i, x)
            })
            .collect();
    }
    let here = machine::enter();
    let next = AtomicUsize::new(0);
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Option<R>)>();
    let mut out: Vec<Option<R>> = Vec::new();
    out.resize_with(items.len(), || None);
    std::thread::scope(|scope| {
        for _ in 0..taken.0 {
            let tx = tx.clone();
            let (next, init, f) = (&next, &init, &f);
            scope.spawn(move || {
                HELPER.with(|h| h.set(true));
                let mut s = init();
                while let Some(_turn) = Turn::wait(next, items.len()) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(x) = items.get(i) else { break };
                    if tx.send((i, None)).is_err() {
                        break;
                    }
                    let r = f(&mut s, i, x);
                    if tx.send((i, Some(r))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        // Results are kept by index, never by when they finished.
        for (i, r) in rx {
            match r {
                None => started(i),
                Some(r) => out[i] = Some(r),
            }
        }
    });
    drop(here);
    drop(taken);
    out.into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
}

/// The gp processes with helpers working on this machine, by the locked
/// files they leave in `~/.gameprinter/work/helpers`.
mod machine {
    use std::fs::{self, File, TryLockError};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// This process's file while it has helpers working, with how many
    /// calls of `map` are using it.
    static MARK: Mutex<(usize, Option<(PathBuf, File)>)> = Mutex::new((0, None));

    /// The last count and when it was taken.
    static SEEN: Mutex<Option<(Instant, usize)>> = Mutex::new(None);

    /// How long a count is used before the folder is read again.
    const FRESH: Duration = Duration::from_secs(1);

    fn dir() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join(".gameprinter")
                .join("work")
                .join("helpers"),
        )
    }

    /// Marks this process as one with helpers working until the guard is
    /// dropped. Without a folder to mark in, the process counts alone.
    pub(super) fn enter() -> Here {
        let marked = {
            let mut m = MARK.lock().unwrap_or_else(|e| e.into_inner());
            m.0 += 1;
            let new = m.1.is_none();
            if new {
                m.1 = dir().and_then(|d| mark(&d));
            }
            new
        };
        // Counted again with this process in. (MARK is let go first:
        // `processes` takes SEEN, then MARK.)
        if marked {
            *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        Here
    }

    pub(super) struct Here;

    impl Drop for Here {
        fn drop(&mut self) {
            let mut m = MARK.lock().unwrap_or_else(|e| e.into_inner());
            m.0 = m.0.saturating_sub(1);
            if m.0 == 0
                && let Some((p, f)) = m.1.take()
            {
                let _ = fs::remove_file(&p);
                drop(f);
            }
        }
    }

    /// A new locked file in `dir`, named for this process and the time:
    /// locked under a temporary name first, so every `.lock` file seen in
    /// the folder is held by a live process or by none.
    pub(super) fn mark(dir: &Path) -> Option<(PathBuf, File)> {
        fs::create_dir_all(dir).ok()?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let name = format!("{}-{nanos}", std::process::id());
        let tmp = dir.join(format!("{name}.tmp"));
        let f = File::create(&tmp).ok()?;
        f.try_lock().ok()?;
        let p = dir.join(format!("{name}.lock"));
        fs::rename(&tmp, &p).ok()?;
        Some((p, f))
    }

    /// The processes with helpers working, this one counted: read again
    /// once [`FRESH`] has passed.
    pub(super) fn processes() -> usize {
        let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, n)) = *seen
            && at.elapsed() < FRESH
        {
            return n;
        }
        let own = MARK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .1
            .as_ref()
            .map(|(p, _)| p.clone());
        let n = match (dir(), &own) {
            (Some(d), Some(own)) => count(&d, own).max(1),
            _ => 1,
        };
        *seen = Some((Instant::now(), n));
        n
    }

    /// The `.lock` files in `dir` a live process holds, `own` among them.
    /// A file no process holds is left by one that died, and is removed.
    pub(super) fn count(dir: &Path, own: &Path) -> usize {
        let Ok(rd) = fs::read_dir(dir) else { return 1 };
        let mut n = 0;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "lock") {
                continue;
            }
            if p == own {
                n += 1;
                continue;
            }
            let Ok(f) = File::open(&p) else { continue };
            match f.try_lock_shared() {
                Ok(()) => {
                    let _ = f.unlock();
                    let _ = fs::remove_file(&p);
                }
                Err(TryLockError::WouldBlock) => n += 1,
                Err(TryLockError::Error(_)) => {}
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_back_in_the_lists_order_for_any_number_of_threads() {
        let items: Vec<u64> = (0..200).collect();
        let work = |_: &mut (), _: usize, x: &u64| {
            // Uneven work, so the threads finish out of order.
            let mut h = *x;
            for _ in 0..(x % 7) * 2000 {
                h = h.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            }
            (*x, h)
        };
        set_jobs(1);
        let one = map(&items, || (), work, &mut |_| {});
        set_jobs(8);
        let mut seen = Vec::new();
        let eight = map(&items, || (), work, &mut |i| seen.push(i));
        assert_eq!(one, eight);
        seen.sort_unstable();
        assert_eq!(seen, items.iter().map(|x| *x as usize).collect::<Vec<_>>());
        // A map inside a helper runs on that helper, with the same results.
        let nested = map(
            &items[..4],
            || (),
            |_, _, _| map(&items, || (), work, &mut |_| {}),
            &mut |_| {},
        );
        assert!(nested.iter().all(|r| *r == one));
    }

    /// A file another process (here another handle) holds counts; one
    /// nobody holds is a dead process's, and is removed.
    #[test]
    fn the_machine_count_is_the_held_files() {
        let d = tempfile::tempdir().unwrap();
        let (own, _f) = machine::mark(d.path()).unwrap();
        assert_eq!(machine::count(d.path(), &own), 1);
        let other = machine::mark(d.path()).unwrap();
        assert_eq!(machine::count(d.path(), &own), 2);
        let gone = other.0.clone();
        drop(other);
        assert_eq!(machine::count(d.path(), &own), 1);
        assert!(!gone.exists());
        assert_eq!(share(1), default_jobs());
        assert!(share(1000) >= 1);
    }
}
