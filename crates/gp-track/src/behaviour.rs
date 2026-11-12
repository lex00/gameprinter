//! Behaviour patterns fitted to tracks (docs/catalog-patterns.md, "The
//! finders"). Each track's whole-pixel positions are tested against the
//! pattern library's motion signatures, and the best fit is named with the
//! numbers that gave it. It reads nothing but `[frame, x, y]` rows, so it
//! works for any game `gp track` can follow.
//!
//! The tests are simple statistics over runs of consecutive frames:
//!
//! | Pattern | Signature |
//! |---|---|
//! | `still` | Neither coordinate changes |
//! | `fly` | x changes by a steady speed one way, y doesn't change |
//! | `walk` or `shuttle` | y doesn't change and x turns back: `shuttle` when it turns at the same two places, `walk` otherwise |
//! | `fall` | y only goes down, speeding up |
//! | `hop` | y rises suddenly from rest and comes back to the same height, again and again |
//! | `leap` | y rises suddenly from the same start again and again, with waits or gaps between |
//! | `oscillate` | a coordinate swings around a center: its speed changes by a steady amount and turns at the center, not at the ends |
//! | `orbit` | the position stays a steady distance from a point while the angle turns |
//! | `pursue` | the speed across follows the side the player is on |

use serde::Serialize;

/// Changes of more than this many pixels between frames are wraps or
/// respawns.
const JUMP: i64 = 64;

/// A track's best-fitting pattern.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fit {
    pub pattern: &'static str,
    /// How well it fits, 0 to 1.
    pub score: f64,
    /// The numbers measured, by name: `speed`, `gravity`, `period`,
    /// `center`, `radius`, `rise`.
    pub params: Vec<(&'static str, f64)>,
}

/// Runs of consecutive frames: (dx, dy) pairs and the positions, with jumps
/// cut out.
struct Steps {
    dx: Vec<i64>,
    dy: Vec<i64>,
    xs: Vec<i64>,
    ys: Vec<i64>,
    frames: Vec<i64>,
}

fn steps(positions: &[[i64; 3]]) -> Steps {
    let mut s = Steps {
        dx: Vec::new(),
        dy: Vec::new(),
        xs: Vec::new(),
        ys: Vec::new(),
        frames: Vec::new(),
    };
    for w in positions.windows(2) {
        if w[1][0] != w[0][0] + 1 {
            continue;
        }
        let (dx, dy) = (w[1][1] - w[0][1], w[1][2] - w[0][2]);
        if dx.abs() > JUMP || dy.abs() > JUMP {
            continue;
        }
        s.dx.push(dx);
        s.dy.push(dy);
        s.xs.push(w[1][1]);
        s.ys.push(w[1][2]);
        s.frames.push(w[1][0]);
    }
    s
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// The speed over windows of `n` frames, so whole-pixel steps of a
/// fractional speed average out.
fn smoothed(d: &[i64], n: usize) -> Vec<f64> {
    d.chunks(n)
        .filter(|c| c.len() == n)
        .map(|c| c.iter().sum::<i64>() as f64 / n as f64)
        .collect()
}

/// Upward starts: indices where dy goes negative after a frame of dy >= 0.
fn takeoffs(dy: &[i64]) -> Vec<usize> {
    (1..dy.len())
        .filter(|&i| dy[i] < 0 && dy[i - 1] >= 0)
        .collect()
}

/// Sign changes of a smoothed speed.
fn reversals(v: &[f64]) -> usize {
    let signs: Vec<f64> = v
        .iter()
        .filter(|x| x.abs() > 1e-9)
        .map(|x| x.signum())
        .collect();
    signs.windows(2).filter(|w| w[0] != w[1]).count()
}

/// Fits `positions` (rows of `[frame, x, y]`) to the pattern library.
/// `player` is the player's positions in the same rows, for `pursue`.
pub fn classify(positions: &[[i64; 3]], player: Option<&[[i64; 3]]>) -> Option<Fit> {
    let s = steps(positions);
    if s.dx.len() < 16 {
        return None;
    }
    let n = s.dx.len() as f64;
    let moved_x = s.dx.iter().filter(|d| **d != 0).count() as f64 / n;
    let moved_y = s.dy.iter().filter(|d| **d != 0).count() as f64 / n;
    let speed_x = s.dx.iter().map(|d| d.abs()).sum::<i64>() as f64 / n;
    let speed_y = s.dy.iter().map(|d| d.abs()).sum::<i64>() as f64 / n;
    if moved_x == 0.0 && moved_y == 0.0 {
        return Some(Fit {
            pattern: "still",
            score: 1.0,
            params: vec![],
        });
    }
    let mut fits: Vec<Fit> = Vec::new();

    // Orbit: a steady distance from the mean point, the angle turning.
    if moved_x > 0.15 && moved_y > 0.15 {
        let cx = mean(&s.xs.iter().map(|v| *v as f64).collect::<Vec<_>>());
        let cy = mean(&s.ys.iter().map(|v| *v as f64).collect::<Vec<_>>());
        let r: Vec<f64> =
            s.xs.iter()
                .zip(&s.ys)
                .map(|(x, y)| ((*x as f64 - cx).powi(2) + (*y as f64 - cy).powi(2)).sqrt())
                .collect();
        let rm = mean(&r);
        let spread = mean(&r.iter().map(|v| (v - rm).abs()).collect::<Vec<_>>());
        if rm >= 4.0 && spread / rm < 0.15 {
            let angles: Vec<f64> =
                s.xs.iter()
                    .zip(&s.ys)
                    .map(|(x, y)| (*y as f64 - cy).atan2(*x as f64 - cx))
                    .collect();
            let mut turn = Vec::new();
            for w in angles.windows(2) {
                let mut d = w[1] - w[0];
                while d > std::f64::consts::PI {
                    d -= 2.0 * std::f64::consts::PI;
                }
                while d < -std::f64::consts::PI {
                    d += 2.0 * std::f64::consts::PI;
                }
                turn.push(d);
            }
            let t = mean(&turn);
            let same = turn.iter().filter(|d| d.signum() == t.signum()).count() as f64
                / turn.len().max(1) as f64;
            if same > 0.8 && t.abs() > 1e-3 {
                fits.push(Fit {
                    pattern: "orbit",
                    score: (1.0 - spread / rm) * same,
                    params: vec![("radius", rm), ("spin", t / (2.0 * std::f64::consts::PI))],
                });
            }
        }
    }

    // Vertical patterns: hops, leaps, falls and swings.
    if moved_y > 0.1 {
        let ups = takeoffs(&s.dy);
        // A takeoff is sudden when the rise starts at its fastest and
        // slows (a launch against gravity); a swing's rise speeds up first.
        let sudden: Vec<usize> = ups
            .iter()
            .copied()
            .filter(|&i| {
                if i < 4 || i + 8 > s.dy.len() {
                    return false;
                }
                // Not rising before: resting or falling.
                if s.dy[i - 4..i].iter().sum::<i64>() < 0 {
                    return false;
                }
                let a: i64 = s.dy[i..i + 4].iter().sum();
                let b: i64 = s.dy[i + 4..i + 8].iter().sum();
                // At least a pixel a frame up over the first four frames:
                // a swing turns slowly at its ends.
                a < b && a <= -4
            })
            .collect();
        let start_ys: Vec<i64> = sudden.iter().map(|&i| s.ys[i] - s.dy[i]).collect();
        let same_start = |ys: &[i64]| -> f64 {
            if ys.is_empty() {
                return 0.0;
            }
            let mut sorted = ys.to_vec();
            sorted.sort_unstable();
            let mid = sorted[sorted.len() / 2];
            ys.iter().filter(|y| (**y - mid).abs() <= 2).count() as f64 / ys.len() as f64
        };
        // Gravity: the mean second difference of y inside the arcs.
        let mut acc = Vec::new();
        for i in 1..s.dy.len() {
            if s.frames[i] == s.frames[i - 1] + 1 && s.dy[i - 1] < 0 || s.dy[i] > 0 {
                acc.push((s.dy[i] - s.dy[i - 1]) as f64);
            }
        }
        let gravity = mean(
            &acc.iter()
                .copied()
                .filter(|a| a.abs() <= 4.0)
                .collect::<Vec<_>>(),
        );
        if sudden.len() >= 2 {
            let start = same_start(&start_ys);
            // Gaps between takeoffs: a hop takes off as it lands; a leap waits.
            let gaps: Vec<f64> = sudden
                .windows(2)
                .map(|w| (s.frames[w[1]] - s.frames[w[0]]) as f64)
                .collect();
            let still_between = sudden
                .windows(2)
                .map(|w| {
                    let span = &s.dy[w[0]..w[1]];
                    span.iter().filter(|d| **d == 0).count() as f64 / span.len().max(1) as f64
                })
                .collect::<Vec<_>>();
            let rest = mean(&still_between);
            let rise = mean(
                &sudden
                    .iter()
                    .map(|&i| {
                        let mut y = s.ys[i] - s.dy[i];
                        let top = s.ys[i..].iter().take(120).copied().min().unwrap_or(y);
                        y -= top;
                        y as f64
                    })
                    .collect::<Vec<_>>(),
            );
            let params = vec![
                ("period", mean(&gaps)),
                ("rise", rise),
                ("gravity", gravity.max(0.0)),
            ];
            if rest > 0.4 || s.frames.len() < positions.len() / 2 {
                fits.push(Fit {
                    pattern: "leap",
                    score: start * 0.9,
                    params: params.clone(),
                });
            }
            fits.push(Fit {
                pattern: "hop",
                score: start * if rest > 0.4 { 0.7 } else { 0.95 },
                params,
            });
        }
        // Falling: y never rises, and its speed grows.
        let rises = s.dy.iter().filter(|d| **d < 0).count();
        if rises == 0 {
            let v = smoothed(&s.dy, 4);
            let growing = v.windows(2).filter(|w| w[1] >= w[0]).count() as f64
                / v.len().saturating_sub(1).max(1) as f64;
            fits.push(Fit {
                pattern: "fall",
                score: 0.6 + 0.3 * growing,
                params: vec![("speed", speed_y)],
            });
        }
        // Swinging on y: smooth speed, reversing, turning without sudden
        // takeoffs.
        let v = smoothed(&s.dy, 8);
        let rev = reversals(&v);
        // The turns must come at the same two heights: a swing's ends.
        let nonzero: Vec<(usize, f64)> = v
            .iter()
            .enumerate()
            .filter(|(_, x)| x.abs() > 1e-9)
            .map(|(k, x)| (k, x.signum()))
            .collect();
        let turn_ys: Vec<i64> = nonzero
            .windows(2)
            .filter(|w| w[0].1 != w[1].1)
            .filter_map(|w| s.ys.get(w[1].0 * 8).copied())
            .collect();
        // Split the turns at their mean: each side must sit at one height.
        let tm = mean(&turn_ys.iter().map(|y| *y as f64).collect::<Vec<_>>());
        let side = |low: bool| -> Vec<i64> {
            let mut v: Vec<i64> = turn_ys
                .iter()
                .copied()
                .filter(|y| ((*y as f64) < tm) == low)
                .collect();
            v.sort_unstable();
            v
        };
        let (lows, highs) = (side(true), side(false));
        let near = |v: &[i64]| {
            v.get(v.len() / 2)
                .map_or(0, |m| v.iter().filter(|y| (**y - m).abs() <= 4).count())
        };
        let at_ends = (near(&lows) + near(&highs)) as f64 / turn_ys.len().max(1) as f64;
        let ymin = lows.get(lows.len() / 2).copied().unwrap_or(0);
        let ymax = highs.get(highs.len() / 2).copied().unwrap_or(0);
        if rev >= 2 && sudden.len() <= rev / 2 && at_ends >= 0.8 && ymax - ymin >= 8 {
            let (ymin, ymax) = (ymin as f64, ymax as f64);
            let center = (ymin + ymax) / 2.0;
            fits.push(Fit {
                pattern: "oscillate",
                score: 0.85 * at_ends,
                params: vec![
                    ("axis_y", 1.0),
                    ("center", center),
                    ("swing", ymax - ymin),
                    ("speed", speed_y),
                ],
            });
        }
    }

    // Horizontal patterns.
    if moved_x > 0.1 && moved_y < 0.2 {
        let v = smoothed(&s.dx, 8);
        let rev = reversals(&v);
        if rev == 0 {
            let steady = {
                let m = mean(&v);
                1.0 - mean(&v.iter().map(|x| (x - m).abs()).collect::<Vec<_>>()) / m.abs().max(1e-9)
            };
            fits.push(Fit {
                pattern: "fly",
                score: 0.6 + 0.3 * steady.clamp(0.0, 1.0),
                params: vec![("speed", speed_x)],
            });
        } else {
            // Turning points: the x at each reversal.
            let mut turns = Vec::new();
            let mut dir = 0;
            for (i, d) in s.dx.iter().enumerate() {
                if *d != 0 {
                    if dir != 0 && d.signum() != dir {
                        turns.push(s.xs[i]);
                    }
                    dir = d.signum();
                }
            }
            let ends = |sign: bool| {
                let side: Vec<i64> = turns
                    .iter()
                    .copied()
                    .filter(|t| {
                        (*t as f64 > mean(&turns.iter().map(|v| *v as f64).collect::<Vec<_>>()))
                            == sign
                    })
                    .collect();
                if side.len() < 2 {
                    return 0.0;
                }
                let m = mean(&side.iter().map(|v| *v as f64).collect::<Vec<_>>());
                side.iter()
                    .filter(|t| (**t as f64 - m).abs() <= 2.0)
                    .count() as f64
                    / side.len() as f64
            };
            let fixed = (ends(true) + ends(false)) / 2.0;
            let pattern = if fixed > 0.7 { "shuttle" } else { "walk" };
            fits.push(Fit {
                pattern,
                score: 0.7
                    + 0.2
                        * if pattern == "shuttle" {
                            fixed
                        } else {
                            1.0 - fixed
                        },
                params: vec![("speed", speed_x), ("turns", turns.len() as f64)],
            });
        }
    }

    // Pursuit: the sign of dx follows the player's side.
    if let Some(p) = player {
        let mut agree = 0usize;
        let mut total = 0usize;
        let mut riding = 0usize;
        for (i, f) in s.frames.iter().enumerate() {
            if s.dx[i] == 0 {
                continue;
            }
            if let Some(pp) = p.iter().find(|r| r[0] == *f) {
                // The player moving with it, as on a lift, isn't pursuit.
                if let Some(prev) = p.iter().find(|r| r[0] == *f - 1)
                    && pp[1] - prev[1] == s.dx[i]
                {
                    riding += 1;
                }
                let side = (pp[1] - s.xs[i]).signum();
                if side != 0 {
                    total += 1;
                    if side == s.dx[i].signum() {
                        agree += 1;
                    }
                }
            }
        }
        if total >= 16 {
            let share = agree as f64 / total as f64;
            // Moving toward the player more than a walker turning at walls
            // would by chance, and turning more than once.
            if share > 0.85
                && (riding as f64) < 0.5 * total as f64
                && reversals(&smoothed(&s.dx, 8)) >= 1
            {
                fits.push(Fit {
                    pattern: "pursue",
                    // Below a shuttle with fixed ends, which a player
                    // walking along also seems to be chased by.
                    score: share * 0.85,
                    params: vec![("speed", speed_x)],
                });
            }
        }
    }

    fits.into_iter().max_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(f: impl Fn(i64) -> (i64, i64), n: i64) -> Vec<[i64; 3]> {
        (0..n)
            .map(|t| {
                let (x, y) = f(t);
                [t, x, y]
            })
            .collect()
    }

    /// Made-up motions, one per pattern.
    #[test]
    fn each_made_up_motion_gets_its_pattern() {
        let still = rows(|_| (10, 10), 60);
        assert_eq!(classify(&still, None).unwrap().pattern, "still");
        let fly = rows(|t| (200 - 2 * t, 50), 60);
        assert_eq!(classify(&fly, None).unwrap().pattern, "fly");
        // Back and forth between 100 and 140, a pixel a frame.
        let shuttle = rows(
            |t| {
                let p = t % 80;
                (if p < 40 { 100 + p } else { 180 - p }, 50)
            },
            320,
        );
        assert_eq!(classify(&shuttle, None).unwrap().pattern, "shuttle");
        // A hop of speed -4 and gravity 0.25, again on landing at y 100.
        let mut hop = Vec::new();
        let (mut y, mut vy) = (100.0f64, -4.0f64);
        for t in 0..300 {
            vy += 0.25;
            y += vy;
            if y >= 100.0 {
                y = 100.0;
                vy = -4.0;
            }
            hop.push([t, 50, y.floor() as i64]);
        }
        assert_eq!(classify(&hop, None).unwrap().pattern, "hop");
        // A swing around y 80 by a force of 1/16, at most 1 a frame.
        let mut osc = Vec::new();
        let (mut y, mut vy) = (64.0f64, 0.0f64);
        for t in 0..400 {
            vy = (vy + if y < 80.0 { 0.0625 } else { -0.0625 }).clamp(-1.0, 1.0);
            y += vy;
            osc.push([t, 200, y.floor() as i64]);
        }
        assert_eq!(classify(&osc, None).unwrap().pattern, "oscillate");
        // A part 24 pixels from a pivot, turning a 32nd of a turn every 4
        // frames.
        let orbit = rows(
            |t| {
                let a = (t / 4) as f64 * std::f64::consts::PI / 16.0;
                (
                    100 + (24.0 * a.cos()).round() as i64,
                    100 + (24.0 * a.sin()).round() as i64,
                )
            },
            256,
        );
        assert_eq!(classify(&orbit, None).unwrap().pattern, "orbit");
    }
}
