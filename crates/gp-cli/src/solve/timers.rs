//! Detector 2, change points and frame-counter modulo on timers
//! (docs/research/deterministic-inference.md, A6 and A10): a count-like
//! measure (a timer, a hold, a phase, a counter's ticks) is cut where its
//! value changes, in both games. The changes give each game's period and
//! phase, the value a reload sets, and whether the original's changes all
//! fall on one residue of the frame count modulo N.
//!
//! | Class | What differs | Usually |
//! |---|---|---|
//! | `start` | the value on the first frame | a start field the mapping doesn't carry |
//! | `period` | the frames from one step to the next | a counter's `every` |
//! | `reload` | the value a reload sets | a timer's reload number |
//! | `aligned` | the original steps only on frames N divides (plus a phase) | `align`, or `every` with `phase` |
//! | `shift` | the same series, some frames apart | a start or a phase a few frames off |
//! | `stops` | one side stops counting, the other goes on | a bound (`min`, `max`) or a hold the model lacks |
//!
//! Values are raw 16.16, as in [`super::arith`]; a count is a whole
//! number.

use serde::Serialize;

use super::arith::ONE;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum Class {
    /// They differ on the clip's first frame.
    Start { original: i64, model: i64 },
    /// One step every `original` frames in the original, `model` in the
    /// model.
    Period { original: u64, model: u64 },
    /// A reload sets `original` in the original and `model` in the model.
    Reload { original: i64, model: i64 },
    /// The original's steps all fall on frames whose number is `phase`
    /// modulo `every`, the model's don't.
    Aligned { every: u64, phase: u64 },
    /// The model's series is the original's `frames` frames later
    /// (negative: earlier).
    Shift { frames: i64 },
    /// One side's count stays at `value` while the other's goes on to
    /// `other`: the rebuilt game's when `rebuilt`.
    Stops {
        rebuilt: bool,
        value: i64,
        other: i64,
    },
}

impl Class {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Start { .. } => "start",
            Self::Period { .. } => "period",
            Self::Reload { .. } => "reload",
            Self::Aligned { .. } => "aligned",
            Self::Shift { .. } => "shift",
            Self::Stops { .. } => "stops",
        }
    }
}

/// What the changes of one series are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Steps {
    /// The indices where the value changes.
    pub changes: Vec<usize>,
    /// The usual change: +1 or -1 counts, in whole units.
    pub step: i64,
    /// The indices of the other changes (reloads), and the value each
    /// sets.
    pub reloads: Vec<(usize, i64)>,
    /// The frames between steps, when they are all the same.
    pub period: Option<u64>,
}

/// One series' change points.
pub fn steps(v: &[i64]) -> Steps {
    let changes: Vec<usize> = (1..v.len()).filter(|&i| v[i] != v[i - 1]).collect();
    let deltas: Vec<i64> = changes.iter().map(|&i| v[i] - v[i - 1]).collect();
    let up = deltas.iter().filter(|d| **d == ONE).count();
    let down = deltas.iter().filter(|d| **d == -ONE).count();
    let step = if up >= down { ONE } else { -ONE };
    let reloads: Vec<(usize, i64)> = changes
        .iter()
        .zip(&deltas)
        .filter(|(_, d)| **d != step)
        .map(|(&i, _)| (i, v[i]))
        .collect();
    // The gaps between steps that have no reload between them.
    let regular: Vec<usize> = changes
        .iter()
        .zip(&deltas)
        .filter(|(_, d)| **d == step)
        .map(|(&i, _)| i)
        .collect();
    let mut gaps: Vec<u64> = Vec::new();
    for w in regular.windows(2) {
        if !reloads.iter().any(|(r, _)| *r > w[0] && *r <= w[1]) {
            gaps.push((w[1] - w[0]) as u64);
        }
    }
    let period = (gaps.len() >= 2 && gaps.windows(2).all(|g| g[0] == g[1])).then(|| gaps[0]);
    Steps {
        changes,
        step,
        reloads,
        period,
    }
}

/// Whether a measure is count-like: whole numbers, mostly changed by one,
/// with a name that says so.
pub fn count_like(path: &str, o: &[i64], m: &[i64]) -> bool {
    if !o
        .iter()
        .chain(m)
        .all(|v| v % ONE == 0 && v.abs() < 65536 * ONE)
    {
        return false;
    }
    let named = [
        "ticks", "counters", "hold", "timer", "phase", "time", "count",
    ]
    .iter()
    .any(|w| path.contains(w));
    // Counts step by one; a score's tens and hundreds are no timer.
    let (so, sm) = (steps(o), steps(m));
    let changes = so.changes.len() + sm.changes.len();
    let ones = changes - so.reloads.len() - sm.reloads.len();
    let steps_by_one = so.changes.len() + sm.changes.len() == 0 || ones * 2 >= changes;
    // By name only: a place that moves a pixel at a time steps by one too.
    steps_by_one && named
}

/// The class of a count-like measure first wrong at `at`, `frame0` the
/// original frame of index 0.
pub fn classify(o: &[i64], m: &[i64], at: usize, frame0: u64) -> Option<Class> {
    let n = o.len().min(m.len());
    if at >= n {
        return None;
    }
    if at == 0 {
        return Some(Class::Start {
            original: o[0],
            model: m[0],
        });
    }
    let so = steps(&o[..n]);
    let sm = steps(&m[..n]);
    if let (Some(a), Some(b)) = (so.period, sm.period)
        && a != b
    {
        return Some(Class::Period {
            original: a,
            model: b,
        });
    }
    // The first reload at or after the first wrong frame, either side.
    let near = |s: &Steps| s.reloads.iter().find(|(i, _)| *i + 2 >= at).map(|r| r.1);
    if let (Some(a), Some(b)) = (near(&so), near(&sm))
        && a != b
    {
        return Some(Class::Reload {
            original: a,
            model: b,
        });
    }
    // One side stops counting while the other goes on.
    let still = |v: &[i64]| (at..n).take_while(|&i| v[i] == v[at]).count();
    let moved = |v: &[i64], k: usize| (at..at + k).filter(|&i| v[i] != v[i - 1]).count();
    let (so_still, sm_still) = (still(o), still(m));
    if sm_still >= 3 && moved(o, sm_still) >= 2 {
        return Some(Class::Stops {
            rebuilt: true,
            value: m[at],
            other: o[at + sm_still - 1],
        });
    }
    if so_still >= 3 && moved(m, so_still) >= 2 {
        return Some(Class::Stops {
            rebuilt: false,
            value: o[at],
            other: m[at + so_still - 1],
        });
    }
    // Aligned: every step of the original on one residue of the frame
    // count, with at least three of them.
    let regular = |s: &Steps| -> Vec<u64> {
        s.changes
            .iter()
            .filter(|i| !s.reloads.iter().any(|(r, _)| r == *i))
            .map(|&i| frame0 + i as u64)
            .collect()
    };
    let ro = regular(&so);
    let rm = regular(&sm);
    for every in [2u64, 4, 8, 16, 32] {
        if ro.len() < 3 {
            break;
        }
        let phase = ro[0] % every;
        if ro.iter().all(|f| f % every == phase) && !rm.iter().all(|f| f % every == phase) {
            // The largest such N, as a counter modulo 8 is modulo 4 too.
            let mut best = every;
            let mut e = every * 2;
            while e <= 256 && ro.iter().all(|f| f % e == ro[0] % e) {
                best = e;
                e *= 2;
            }
            return Some(Class::Aligned {
                every: best,
                phase: ro[0] % best,
            });
        }
    }
    // The same series some frames apart.
    let mut best: Option<(usize, i64)> = None;
    for k in 1..=8i64 {
        for shift in [k, -k] {
            let hold = (at..n)
                .take_while(|&i| {
                    let j = i as i64 - shift;
                    j >= 0 && (j as usize) < n && m[i] == o[j as usize]
                })
                .count();
            if hold >= 3 && best.is_none_or(|(h, _)| hold > h) {
                best = Some((hold, shift));
            }
        }
    }
    best.map(|(_, frames)| Class::Shift { frames })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole(v: &[i64]) -> Vec<i64> {
        v.iter().map(|x| x * ONE).collect()
    }

    #[test]
    fn a_timer_counting_down_at_another_rate_has_another_period() {
        // Made up: down one every 4 frames in the original, every 3 in the
        // model.
        let o: Vec<i64> = (0..40).map(|i| (20 - i / 4) * ONE).collect();
        let m: Vec<i64> = (0..40).map(|i| (20 - i / 3) * ONE).collect();
        let at = (0..40).find(|&i| o[i] != m[i]).unwrap();
        assert_eq!(
            classify(&o, &m, at, 0),
            Some(Class::Period {
                original: 4,
                model: 3
            })
        );
    }

    #[test]
    fn a_hold_reloaded_to_another_number_is_a_reload() {
        let o = whole(&[0, 0, 3, 2, 1, 0, 0, 3, 2, 1, 0]);
        let m = whole(&[0, 0, 4, 3, 2, 1, 0, 4, 3, 2, 1]);
        assert_eq!(
            classify(&o, &m, 2, 0),
            Some(Class::Reload {
                original: 3 * ONE,
                model: 4 * ONE
            })
        );
    }

    #[test]
    fn steps_only_on_frames_eight_divides_are_aligned() {
        // The original steps on frames 8, 16, 24, ... of the frame count;
        // the model every 8 frames from its own start, 3 frames off.
        let o: Vec<i64> = (0..48u64).map(|f| ((100 + f) / 8) as i64 * ONE).collect();
        let m: Vec<i64> = (0..48u64).map(|f| ((103 + f) / 8) as i64 * ONE).collect();
        let at = (0..48).find(|&i| o[i] != m[i]).unwrap();
        assert_eq!(
            classify(&o, &m, at, 100),
            Some(Class::Aligned { every: 8, phase: 0 })
        );
    }

    #[test]
    fn the_same_count_a_frame_ahead_is_a_shift() {
        let o = whole(&[5, 5, 5, 4, 4, 4, 3, 3, 3, 2, 2, 2, 7, 7]);
        let m = whole(&[5, 5, 4, 4, 4, 3, 3, 3, 2, 2, 2, 7, 7, 7]);
        // Both periods are 3, no reload differs, and with only three
        // steps on the frame count's residues it isn't aligned.
        assert_eq!(classify(&o, &m, 2, 1), Some(Class::Shift { frames: -1 }));
    }

    #[test]
    fn a_hold_the_model_stops_counting_stops() {
        let o = whole(&[7, 6, 5, 4, 3, 2, 1, 0, 0]);
        let m = whole(&[7, 6, 5, 4, 3, 2, 2, 2, 2]);
        assert_eq!(
            classify(&o, &m, 6, 0),
            Some(Class::Stops {
                rebuilt: true,
                value: 2 * ONE,
                other: 0
            })
        );
    }

    #[test]
    fn a_wrong_first_frame_is_the_start() {
        let o = whole(&[3, 2, 1]);
        let m = whole(&[0, 0, 0]);
        assert_eq!(
            classify(&o, &m, 0, 0),
            Some(Class::Start {
                original: 3 * ONE,
                model: 0
            })
        );
    }

    #[test]
    fn counts_are_told_from_positions() {
        assert!(count_like(
            "cameras.main.hold",
            &whole(&[0, 1]),
            &whole(&[0, 2])
        ));
        assert!(!count_like(
            "counters.score",
            &whole(&[100, 120, 220]),
            &whole(&[100, 100, 200])
        ));
        assert!(!count_like(
            "entities.mario[0].x",
            &[ONE / 2, ONE],
            &[ONE, ONE]
        ));
    }
}
