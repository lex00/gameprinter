//! Scoring a set of clips on several threads.
//!
//! Each clip's score reads the clip, the model and the bench and nothing
//! else, so the clips of a set can run on any threads in any order. [`map`]
//! hands the items of a list to helper threads and gives the results back
//! by index, so the output is the same for any number of threads.
//!
//! The helpers are shared by the whole process: at most `jobs()` of them
//! run at once, whichever call asked for them. A call that finds none free
//! runs its items on its own thread, one after another, as before.
//!
//! | Setting | What |
//! |---|---|
//! | `--jobs N` | Any command: up to N helper threads (1 runs everything on the command's own thread) |
//! | `GP_JOBS=N` | The same, for a command or `gp serve` started without `--jobs` |
//! | Neither | Every core less two, at least 1 |

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The helper threads running now.
static USED: Mutex<usize> = Mutex::new(0);

/// The setting `--jobs` gave, if any (0: none).
static SET: AtomicUsize = AtomicUsize::new(0);

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

/// `f` on every item, on as many helper threads as are free and the list
/// needs, each with its own state from `init`; the results in the list's
/// order. `started` hears each item's index as it starts, on the calling
/// thread (a little after, when helpers run it). With one item, or no
/// helper free, it all runs on the calling thread in order.
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
    let n = if items.len() > 1 {
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
    let next = AtomicUsize::new(0);
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Option<R>)>();
    let mut out: Vec<Option<R>> = Vec::new();
    out.resize_with(items.len(), || None);
    std::thread::scope(|scope| {
        for _ in 0..taken.0 {
            let tx = tx.clone();
            let (next, init, f) = (&next, &init, &f);
            scope.spawn(move || {
                let mut s = init();
                loop {
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
    drop(taken);
    out.into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
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
    }
}
