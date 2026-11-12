//! Detector 1, exact arithmetic recovery (docs/research/deterministic-inference.md,
//! A8): the rebuilt measure less the original's, from the first wrong
//! frame on, read as one of a few shapes the NES's byte arithmetic gives.
//!
//! | Class | The diff | What it usually is |
//! |---|---|---|
//! | `constant` | the same every frame | an anchor or spawn offset; a fraction not carried |
//! | `linear` | grows by the same amount each frame | a per-frame rate off by a fraction (1/64, 1/2) |
//! | `lag` | the model's value is the original's a frame earlier or later | a rule run a frame apart, the system order |
//! | `sign_flip` | the model's value is the original's negated | a bounce or a turn the other way |
//! | `every_n` | changes on one frame in N only | a mod-N gate (`every`, `phase`) |
//! | `wrap` | a multiple of 256 px, or the low bytes agree | a compare on a low byte, a wrapped position |
//! | `stops` | one side's value stays while the other's moves | a rule the model lacks, or one it runs too long |
//! | `start` | wrong from the first frames, none of the above | a start field the translation doesn't carry |
//!
//! Every value is a raw 16.16 number, as `gp_sim::Fx` keeps it, so a
//! fraction is exact: a rate is found as a whole number of 65536ths and
//! its denominator is the smallest power of two that explains it. No fit:
//! each class reads its number off the diff.

use serde::Serialize;

/// One pixel in raw 16.16.
pub const ONE: i64 = 1 << 16;

/// The most frames read past the first wrong one.
pub const SPAN: usize = 64;

/// What the diff is, with the number it gives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum Class {
    /// The rebuilt value is `by` more than the original's on every frame;
    /// `from_start` when it is so from the clip's first or second frame.
    Constant { by: i64, from_start: bool },
    /// The diff grows by `rate` a frame; `denominator` is the smallest
    /// power of two `rate` is a whole number of.
    Linear { rate: i64, denominator: i64 },
    /// The rebuilt value is the original's from `frames` frames before
    /// (1: a frame late; -1: a frame early).
    Lag { frames: i64 },
    /// The rebuilt value is the original's with its sign turned.
    SignFlip { original: i64, model: i64 },
    /// The diff changes by `step` on the frames whose number, counted
    /// from the first compared frame's original frame, is `phase` modulo
    /// `every`, and on no other.
    EveryN { every: u64, phase: u64, step: i64 },
    /// The diff is a whole multiple of 256 px, or the two agree modulo
    /// 256 px.
    Wrap { by: i64 },
    /// One side's value stays at `value` while the other's goes on
    /// changing: the rebuilt game's when `rebuilt`.
    Stops { rebuilt: bool, value: i64 },
    /// Wrong from the clip's first frames with none of the shapes above:
    /// a start field the translation doesn't carry.
    Start { original: i64, model: i64 },
}

impl Class {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Constant { .. } => "constant",
            Self::Linear { .. } => "linear",
            Self::Lag { .. } => "lag",
            Self::SignFlip { .. } => "sign_flip",
            Self::EveryN { .. } => "every_n",
            Self::Wrap { .. } => "wrap",
            Self::Stops { .. } => "stops",
            Self::Start { .. } => "start",
        }
    }
}

/// A class and the frames it holds for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Read {
    #[serde(flatten)]
    pub class: Class,
    /// The index of the first wrong frame.
    pub at: usize,
    /// How many frames from it the class holds.
    pub holds: usize,
}

/// A raw number as a fraction with a power-of-two denominator, in words:
/// `1/64`, `-3/2`, `4`.
pub fn fraction(raw: i64) -> String {
    let den = denominator(raw);
    let num = raw / (ONE / den);
    if den == 1 {
        num.to_string()
    } else {
        format!("{num}/{den}")
    }
}

/// The smallest power of two `raw` (16.16) is a whole number of.
pub fn denominator(raw: i64) -> i64 {
    if raw == 0 {
        return 1;
    }
    let tz = raw.unsigned_abs().trailing_zeros().min(16);
    1 << (16 - tz)
}

/// What a measure is, for the classes that apply to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A place or another number.
    Place,
    /// A speed (`vx`, `vy`): a sign flip applies.
    Speed,
    /// A whole count read by [`super::timers`] first: no rate read through
    /// whole steps.
    Count,
    /// A flag (0 or 1): only a lag applies.
    Flag,
}

/// A flag the same as the original's some frames apart, with the
/// original's flag changing in that stretch: "gone a frame late".
fn flag_lag(o: &[i64], m: &[i64], at: usize, end: usize) -> Option<Read> {
    let mut best: Option<(usize, i64)> = None;
    for k in 1..=8i64 {
        for lag in [k, -k] {
            let hold = (at..end)
                .take_while(|&i| {
                    let j = i as i64 - lag;
                    j >= 0 && (j as usize) < end && m[i] == o[j as usize]
                })
                .count();
            // Both flags change in the stretch: the same change, frames
            // apart.
            let lo = (at as i64 - k).max(1) as usize;
            let moves = (lo..(at + hold).min(end)).any(|i| o[i] != o[i - 1]);
            let model_moves = (at.max(1)..(at + hold).min(end)).any(|i| m[i] != m[i - 1]);
            if hold >= 2 && moves && model_moves && best.is_none_or(|(h, _)| hold > h) {
                best = Some((hold, lag));
            }
        }
    }
    best.map(|(holds, frames)| Read {
        class: Class::Lag { frames },
        at,
        holds,
    })
}

/// The diff's class from `at` on, or `None` when no class holds long
/// enough. `o` and `m` are the original's and the rebuilt series (`None`
/// where a value isn't a number), `frame0` the original frame of index
/// 0, `kind` what the measure is.
pub fn classify(
    o: &[Option<i64>],
    m: &[Option<i64>],
    at: usize,
    frame0: u64,
    kind: Kind,
) -> Option<Read> {
    let speed = kind == Kind::Speed;
    // The frames both have a number for, from `at`.
    let mut end = at;
    while end < o.len().min(m.len()) && end - at < SPAN && o[end].is_some() && m[end].is_some() {
        end += 1;
    }
    if end <= at {
        return None;
    }
    let ov: Vec<i64> = o[..end].iter().map(|v| v.unwrap_or(0)).collect();
    let mv: Vec<i64> = m[..end].iter().map(|v| v.unwrap_or(0)).collect();
    let d: Vec<i64> = (0..end).map(|i| mv[i] - ov[i]).collect();
    if d[at] == 0 {
        return None;
    }
    if kind == Kind::Flag {
        return flag_lag(&ov, &mv, at, end);
    }
    let ends_here = end == o.len().min(m.len());
    let whole = ov[..end].iter().chain(&mv[..end]).all(|v| v % ONE == 0);
    let mut found: Vec<(usize, u8, Class)> = Vec::new();
    // Constant, and the 256 wrap.
    let c = d[at];
    let hold_c = (at..end).take_while(|&i| d[i] == c).count();
    let wraps = |i: usize| d[i] != 0 && d[i] % (256 * ONE) == 0;
    let hold_w = (at..end).take_while(|&i| wraps(i)).count();
    if hold_w >= 1 {
        found.push((hold_w, 0, Class::Wrap { by: c }));
    }
    if hold_c >= 3 || (ends_here && hold_c == end - at && hold_c >= 1) {
        found.push((
            hold_c,
            3,
            Class::Constant {
                by: c,
                from_start: at <= 1,
            },
        ));
    }
    // A few frames late or early: the model's value is the original's
    // from frames apart, while the original's moves, from the first wrong
    // frame or the one after it (a frame that differs as the lag starts).
    for k in 1..=3i64 {
        for lag in [k, -k] {
            let src = |i: usize| -> Option<i64> {
                let j = i as i64 - lag;
                (j >= 0 && (j as usize) < end)
                    .then(|| o[j as usize])
                    .flatten()
            };
            for skip in 0..=1usize {
                let from = at + skip;
                let hold = (from..end).take_while(|&i| src(i) == Some(mv[i])).count();
                let moves = (from.max(1)..from + hold)
                    .filter(|&i| ov[i] != ov[i - 1])
                    .count();
                let steps: Vec<i64> = (from.max(1)..from + hold)
                    .map(|i| ov[i] - ov[i - 1])
                    .collect();
                let even = steps.windows(2).all(|w| w[0] == w[1]);
                if hold >= 3 && moves >= 2 {
                    // A steady speed a frame late is a constant offset too;
                    // the lag reads it only when the steps vary.
                    let rank = if even { 4 } else { 1 } + u8::from(k > 1) + u8::from(skip > 0);
                    found.push((hold + skip, rank, Class::Lag { frames: lag }));
                    break;
                }
            }
        }
    }
    // One side stops while the other goes on.
    let still = |v: &[i64]| (at..end).take_while(|&i| v[i] == v[at]).count();
    let moved = |v: &[i64], n: usize| (at.max(1)..at + n).filter(|&i| v[i] != v[i - 1]).count();
    let (so, sm) = (still(&ov), still(&mv));
    if sm >= 3 && moved(&ov, sm) >= 2 {
        found.push((
            sm,
            2,
            Class::Stops {
                rebuilt: true,
                value: mv[at],
            },
        ));
    } else if so >= 3 && moved(&mv, so) >= 2 {
        found.push((
            so,
            2,
            Class::Stops {
                rebuilt: false,
                value: ov[at],
            },
        ));
    }
    // Sign flip.
    if speed && ov[at] != 0 && mv[at] == -ov[at] {
        let hold = (at..end)
            .take_while(|&i| ov[i] != 0 && mv[i] == -ov[i])
            .count();
        found.push((
            hold.max(3),
            2,
            Class::SignFlip {
                original: ov[at],
                model: mv[at],
            },
        ));
    }
    // Exactly linear: the same step every frame.
    if end - at >= 4 {
        let r = d[at + 1] - d[at];
        if r != 0 {
            let hold = 1 + (at + 1..end).take_while(|&i| d[i] - d[i - 1] == r).count();
            if hold >= 4 {
                found.push((
                    hold,
                    3,
                    Class::Linear {
                        rate: r,
                        denominator: denominator(r),
                    },
                ));
            }
        }
    }
    // Every N: the diff steps on one residue only, by the same amount,
    // on every frame of that residue.
    let prev = if at > 0 { d[at - 1] } else { 0 };
    let step = |i: usize| {
        if i == at {
            d[at] - prev
        } else {
            d[i] - d[i - 1]
        }
    };
    'n: for n in 2..=8u64 {
        let changes: Vec<usize> = (at..end).filter(|&i| step(i) != 0).collect();
        let Some(&c0) = changes.first() else { break };
        let r = (frame0 + c0 as u64) % n;
        let s = step(c0);
        let mut hold = 0usize;
        let mut count = 0usize;
        for i in at..end {
            let on = (frame0 + i as u64) % n == r;
            let st = step(i);
            if (on && st != s) || (!on && st != 0) {
                if count < 3 {
                    continue 'n;
                }
                break;
            }
            if on {
                count += 1;
            }
            hold += 1;
        }
        if count >= 3 {
            found.push((
                hold,
                2,
                Class::EveryN {
                    every: n,
                    phase: r,
                    step: s,
                },
            ));
            break;
        }
    }
    // Linear on whole pixels: a rate under a pixel shows as whole steps,
    // so the diff stays within a pixel of a straight line.
    if whole
        && kind != Kind::Count
        && end - at >= 6
        && !found.iter().any(|f| matches!(f.2, Class::Linear { .. }))
    {
        'len: for len in (6..=end - at).rev() {
            let last = at + len - 1;
            for q in 0..=8u32 {
                let den = 1i64 << q;
                // The rate in 1/den px, rounded from the line's ends.
                let span = (last - at) as i64;
                let num = ((d[last] - d[at]) * den + span * ONE / 2 * (d[last] - d[at]).signum())
                    / (span * ONE);
                // A drift under two pixels over the line is no rate to read.
                if num == 0 || (d[last] - d[at]).abs() < 2 * ONE {
                    continue;
                }
                let fits = (at..=last).all(|i| {
                    let want = d[at] * den + num * ONE * (i - at) as i64;
                    (d[i] * den - want).abs() < ONE * den
                });
                if fits {
                    let rate = num * ONE / den;
                    found.push((
                        len,
                        5,
                        Class::Linear {
                            rate,
                            denominator: denominator(rate),
                        },
                    ));
                    break 'len;
                }
            }
        }
    }
    // Wrong from the start with no shape: a start field.
    if found.is_empty() && at <= 1 {
        found.push((
            1,
            9,
            Class::Start {
                original: ov[at],
                model: mv[at],
            },
        ));
    }
    // The longest hold wins; between equals the lower rank.
    found.sort_by_key(|(hold, rank, _)| (std::cmp::Reverse(*hold), *rank));
    found
        .into_iter()
        .next()
        .map(|(holds, _, class)| Read { class, at, holds })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(v: f64) -> Option<i64> {
        Some((v * ONE as f64).round() as i64)
    }

    fn series(v: &[f64]) -> Vec<Option<i64>> {
        v.iter().map(|x| px(*x)).collect()
    }

    #[test]
    fn a_constant_offset_from_the_start_is_an_anchor() {
        let o = series(&[100.0, 101.0, 102.0, 103.0, 104.0, 105.0]);
        let m = series(&[100.0, 109.0, 110.0, 111.0, 112.0, 113.0]);
        let r = classify(&o, &m, 1, 0, Kind::Place).unwrap();
        assert_eq!(
            r.class,
            Class::Constant {
                by: 8 * ONE,
                from_start: true
            }
        );
    }

    #[test]
    fn a_rate_off_by_a_64th_grows_linearly_and_names_its_denominator() {
        // A made-up body falling at 1/2 px a frame in the original and
        // 33/64 in the model.
        let o: Vec<Option<i64>> = (0..20).map(|i| Some(i * ONE / 2)).collect();
        let m: Vec<Option<i64>> = (0..20).map(|i| Some(i * ONE / 2 + i * ONE / 64)).collect();
        let r = classify(&o, &m, 1, 0, Kind::Place).unwrap();
        assert_eq!(
            r.class,
            Class::Linear {
                rate: ONE / 64,
                denominator: 64
            }
        );
        assert_eq!(fraction(ONE / 64), "1/64");
        assert_eq!(fraction(-3 * ONE / 2), "-3/2");
    }

    #[test]
    fn a_half_pixel_rate_on_whole_pixels_still_reads_as_a_half() {
        // Whole pixels only: the model moves 1 px a frame, the original
        // 1 px every other frame, read through floor.
        let o: Vec<Option<i64>> = (0..24).map(|i| Some((i / 2) * ONE)).collect();
        let m: Vec<Option<i64>> = (0..24).map(|i| Some(i * ONE)).collect();
        let r = classify(&o, &m, 1, 0, Kind::Place).unwrap();
        // The diff steps on odd frames only: every 2 reads it exactly.
        assert_eq!(
            r.class,
            Class::EveryN {
                every: 2,
                phase: 1,
                step: ONE
            }
        );
    }

    #[test]
    fn a_value_a_frame_late_is_a_lag() {
        let o = series(&[0.0, 0.0, 1.0, 3.0, 6.0, 10.0, 15.0, 21.0]);
        let m = series(&[0.0, 0.0, 0.0, 1.0, 3.0, 6.0, 10.0, 15.0]);
        let r = classify(&o, &m, 2, 0, Kind::Place).unwrap();
        assert_eq!(r.class, Class::Lag { frames: 1 });
    }

    #[test]
    fn a_speed_turned_the_other_way_is_a_sign_flip() {
        let o = series(&[1.0, 1.0, -2.0, -2.0, -2.0]);
        let m = series(&[1.0, 1.0, 2.0, 2.0, 2.0]);
        let r = classify(&o, &m, 2, 0, Kind::Speed).unwrap();
        assert_eq!(
            r.class,
            Class::SignFlip {
                original: -2 * ONE,
                model: 2 * ONE
            }
        );
    }

    #[test]
    fn a_diff_that_steps_every_fourth_frame_is_every_n() {
        // The original moves 1 px on the frames whose number isn't a
        // multiple of 4; the model moves every frame.
        let o: Vec<Option<i64>> = (0..24)
            .map(|i| Some((1..=i).filter(|f| f % 4 != 0).count() as i64 * ONE))
            .collect();
        let m: Vec<Option<i64>> = (0..24).map(|i| Some(i * ONE)).collect();
        let first = (0..24).find(|&i| o[i] != m[i]).unwrap();
        let r = classify(&o, &m, first, 0, Kind::Place).unwrap();
        assert_eq!(
            r.class,
            Class::EveryN {
                every: 4,
                phase: 0,
                step: ONE
            }
        );
    }

    #[test]
    fn a_far_body_read_as_near_is_a_256_wrap() {
        let o = series(&[6.0, 6.0, 6.0, 6.0]);
        let m = series(&[262.0, 262.0, 262.0, 262.0]);
        let r = classify(&o, &m, 0, 0, Kind::Place).unwrap();
        assert_eq!(r.class, Class::Wrap { by: 256 * ONE });
    }

    #[test]
    fn a_flag_that_drops_a_frame_late_is_a_lag_and_nothing_else() {
        let o = series(&[1.0, 1.0, 1.0, 0.0, 0.0, 0.0]);
        let m = series(&[1.0, 1.0, 1.0, 1.0, 0.0, 0.0]);
        let r = classify(&o, &m, 3, 0, Kind::Flag).unwrap();
        assert_eq!(r.class, Class::Lag { frames: 1 });
        // A flag the model never drops is no lag.
        let m = series(&[1.0; 6]);
        assert_eq!(classify(&o, &m, 3, 0, Kind::Flag), None);
    }

    #[test]
    fn a_body_the_model_leaves_standing_stops() {
        let o = series(&[120.0, 120.0, 121.0, 121.0, 122.0, 124.0, 125.0, 127.0]);
        let m = series(&[120.0; 8]);
        let r = classify(&o, &m, 2, 0, Kind::Place).unwrap();
        assert_eq!(
            r.class,
            Class::Stops {
                rebuilt: true,
                value: 120 * ONE
            }
        );
    }

    #[test]
    fn a_lag_may_start_a_frame_after_the_first_wrong_frame() {
        let o = series(&[0.04, 0.05, 0.025, 0.0, 0.037, 0.012, 0.049]);
        let m = series(&[0.04, 0.014, 0.05, 0.025, 0.0, 0.037, 0.012]);
        let r = classify(&o, &m, 1, 0, Kind::Place).unwrap();
        assert_eq!(r.class, Class::Lag { frames: 1 });
    }

    #[test]
    fn agreeing_series_have_no_class() {
        let o = series(&[1.0, 2.0, 3.0]);
        assert_eq!(classify(&o, &o, 0, 0, Kind::Place), None);
    }
}
