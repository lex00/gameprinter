//! Feel parameters measured from a recording's tracks (docs/mechanics.md,
//! "Measuring feel"). The player's track is the one `gp track` found
//! moving the way the controller says; its whole-pixel positions and the
//! controller give:
//!
//! - walk and run speed: on the ground with one direction held, the run
//!   button held or not, the mean speed over the last half of each run of
//!   at least [`MIN_RUN`] frames;
//! - acceleration: from a standing start, the distance a run falls short
//!   of moving at top speed from its first frame, `v^2 / (2 (D + v/2))` for
//!   a speed gained `a` a frame;
//! - the jump: each press of the jump action while standing, as an arc of
//!   heights; a quadratic fitted to the arc gives gravity (twice the square
//!   term) and the takeoff speed, the part while the action is held and
//!   rising giving the held gravity, and the frames falling at the arc's
//!   steepest give the fall limit. Each jump also gives its rise for the
//!   frames the action was held;
//! - enemies: each bound track's step (the most common move), period (the
//!   most common frames between moves) and speed (pixels over frames).
//!
//! Positions are whole pixels, so speeds are means over many frames and
//! every measured value is rounded to the grid the original likely used
//! ([`grid`]) before it is proposed.

use std::collections::BTreeMap;

use gp_model::Model;
use gp_track::TrackReport;
use serde::Serialize;

/// Frames a run on the ground needs before its speed is measured.
pub const MIN_RUN: usize = 16;
/// The longest jump arc followed, in frames.
const MAX_ARC: usize = 300;

/// The controller bits the player's actions use.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pad {
    pub left: u8,
    pub right: u8,
    pub run: u8,
    pub jump: u8,
}

impl Pad {
    /// The bits of the model's player actions: its `run`'s `run_action`
    /// and its `jump`'s `action`, with `left` and `right` by name.
    pub fn of(model: &Model, player: &str) -> Self {
        let Some(input) = &model.input else {
            return Self::default();
        };
        // The bit by the model's device, so an arcade board's buttons read too.
        let b = |name: &str| {
            input
                .actions
                .get(name)
                .and_then(|&x| input.device.bit(x))
                .unwrap_or(0)
        };
        let e = model.entities.get(player);
        let run = e
            .and_then(|e| e.platformer.run.as_ref())
            .and_then(|r| r.run_action.as_deref())
            .map_or(0, b);
        let jump = e
            .and_then(|e| e.platformer.jump.as_ref())
            .map_or(0, |j| b(&j.action));
        Self {
            left: b("left"),
            right: b("right"),
            run,
            jump,
        }
    }
}

/// One run on the ground in one direction.
#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub frame: u64,
    pub frames: usize,
    pub running: bool,
    /// Pixels a frame over its last half.
    pub speed: f64,
    /// From a standing start: the speed gained a frame, or `None` when it
    /// reached its speed at once.
    pub acceleration: Option<f64>,
    pub from_standing: bool,
}

/// One jump.
#[derive(Debug, Clone, Serialize)]
pub struct Jump {
    /// The frame the action was pressed on.
    pub frame: u64,
    /// Frames the action was held from the press.
    pub hold: u32,
    /// Whole pixels from the takeoff height to the top.
    pub rise: i64,
    pub airtime: u32,
    /// Pixels a frame across in the four frames before the press.
    pub takeoff_vx: f64,
    /// Fitted to the frames after the held phase (or all of them), before
    /// the fall limit: gravity, and the takeoff speed when the whole arc is
    /// one phase.
    pub gravity: Option<f64>,
    pub speed: Option<f64>,
    /// Fitted to the held, rising frames.
    pub gravity_held: Option<f64>,
    pub speed_held: Option<f64>,
    /// The mean drop a frame where the arc falls at its steepest for three
    /// frames or more.
    pub max_fall: Option<f64>,
}

/// One enemy track's motion.
#[derive(Debug, Clone, Serialize)]
pub struct Enemy {
    pub track: usize,
    pub id: Option<u8>,
    pub palette: u8,
    pub frames: usize,
    pub step: Option<i64>,
    pub period: Option<u32>,
    pub speed: f64,
}

/// What a recording's tracks show.
#[derive(Debug, Clone, Serialize)]
pub struct Feel {
    pub runs: Vec<Run>,
    pub jumps: Vec<Jump>,
    pub enemies: Vec<Enemy>,
    /// Frames the player's track covers.
    pub player_frames: usize,
}

/// Measures the player's and the enemies' tracks. `pads[i]` is the
/// controller for frame `first + i`. `move_first` is the model's jump
/// order, which decides how the takeoff speed is read from the fit.
pub fn measure(
    tracks: &[TrackReport],
    first: u64,
    pads: &[u8],
    pad: Pad,
    move_first: bool,
    held_gravity: bool,
) -> Feel {
    let pad_at = |f: u64| -> u8 {
        f.checked_sub(first)
            .and_then(|i| pads.get(i as usize))
            .copied()
            .unwrap_or(0)
    };
    // The player's positions by frame, from every track that is the player.
    let mut pos: BTreeMap<u64, (i64, i64)> = BTreeMap::new();
    for t in tracks.iter().filter(|t| t.player) {
        for p in &t.positions {
            pos.entry(p[0] as u64).or_insert((p[1], p[2]));
        }
    }
    let at = |f: u64| pos.get(&f).copied();
    let still_y = |f: u64| match (at(f.wrapping_sub(1)), at(f)) {
        (Some(a), Some(b)) => a.1 == b.1,
        _ => false,
    };

    // Jumps first, so runs can leave their frames out.
    let mut in_air = vec![];
    let mut jumps = Vec::new();
    let frames: Vec<u64> = pos.keys().copied().collect();
    for &f in &frames {
        if pad.jump == 0 || pad_at(f) & pad.jump == 0 || pad_at(f - 1) & pad.jump != 0 {
            continue;
        }
        if !still_y(f - 1) || in_air.iter().any(|&(a, b)| f >= a && f <= b) {
            continue;
        }
        let Some((x0, y0)) = at(f - 1) else { continue };
        // The arc: heights from the frame before the press until it stops
        // falling.
        let mut ys = vec![0i64];
        let mut end = None;
        for k in 1..MAX_ARC as u64 {
            let Some((_, y)) = at(f - 1 + k) else { break };
            ys.push(y - y0);
            let n = ys.len();
            if n >= 3 && ys[n - 1] == ys[n - 2] && ys[n - 2] - ys[n - 3] > 0 {
                end = Some(n - 2);
                break;
            }
        }
        let Some(end) = end else { continue };
        if ys[1..=end.min(3)].iter().all(|&y| y >= 0) {
            continue;
        }
        let full = &ys[..=end];
        in_air.push((f, f - 1 + end as u64));
        let mut hold = 0u32;
        while pad_at(f + hold as u64) & pad.jump != 0 && (hold as usize) < end {
            hold += 1;
        }
        let rise = -full.iter().copied().min().unwrap_or(0);
        // Fit only the part one gravity explains: up to a landing on an
        // enemy or a block, a bounce, or a bump on a block overhead.
        let (ys, bumped) = free_flight(full);
        let takeoff_vx = match at(f.saturating_sub(5)) {
            Some((xb, _)) => (x0 - xb).abs() as f64 / 4.0,
            None => 0.0,
        };
        // The steepest fall, held for three frames or more.
        let dys: Vec<i64> = ys.windows(2).map(|w| w[1] - w[0]).collect();
        let steepest = dys.iter().copied().max().unwrap_or(0);
        let cap_from = dys
            .windows(3)
            .position(|w| w.iter().all(|&d| d == steepest) && steepest > 0);
        let max_fall = cap_from.map(|c| {
            let run: Vec<i64> = dys[c..]
                .iter()
                .copied()
                .take_while(|&d| d == steepest)
                .collect();
            run.iter().sum::<i64>() as f64 / run.len() as f64
        });
        // The fit stops where the fall levels out: from the first frame
        // the drop is within a pixel of the steepest and stays there for
        // three frames or more, since a cap with a fraction below it
        // drops 4 and 5 pixels in turn and never three of either.
        let plateau = (steepest > 1)
            .then(|| dys.iter().position(|&d| d >= steepest - 1))
            .flatten()
            .filter(|&k| dys[k..].iter().all(|&d| d >= steepest - 1) && dys.len() - k >= 3);
        // A level stretch's first drops are still speeding up through the
        // last whole pixel, so the fit keeps two frames of it (on made-up
        // capped falls this halves the error of stopping at its start).
        let pre_cap = cap_from
            .map(|c| c + 1)
            .into_iter()
            .chain(plateau.map(|k| k + 3))
            .min()
            .map_or(ys.len(), |c| c.min(ys.len()));
        // The top: the last frame at the highest height of the part fitted,
        // where the fall begins (frames level at the top still rise by a
        // fraction under held gravity).
        let high = ys.iter().copied().min().unwrap_or(0);
        let top = ys.iter().rposition(|&y| y == high).unwrap_or(0);
        let speed_of = |c1: f64, c2: f64| if move_first { c1 + c2 } else { c1 - c2 };
        let (mut gravity, mut speed, mut gravity_held, mut speed_held) = (None, None, None, None);
        if held_gravity {
            // Held while rising: from the takeoff to the release or the top.
            let held_end = (hold as usize).min(top).min(pre_cap);
            if let Some((_, c1, c2)) = fit(ys, 0, held_end + 1) {
                gravity_held = Some(2.0 * c2);
                speed_held = Some(speed_of(c1, c2));
            }
            // Released: from there to the fall limit. After a bump the
            // fall starts from a speed the block set, which still fits.
            if let Some((_, _, c2)) = fit(ys, held_end.max(top), pre_cap) {
                gravity = Some(2.0 * c2);
            }
            let _ = bumped;
        } else if let Some((c0, c1, c2)) = fit(ys, 0, pre_cap)
            // One gravity explains the arc only if the parabola meets every
            // height within a pixel: a head that bumps a block slowly
            // (Moss Hop's ? block stops a rise of a pixel a frame) bends
            // the arc in a way free_flight can't see by its drops.
            && ys[..pre_cap.min(ys.len())].iter().enumerate().all(|(k, &y)| {
                let k = k as f64;
                (c0 + c1 * k + c2 * k * k - y as f64).abs() <= 1.0
            })
            // and if the rise and the fall, fitted apart, meet at the top
            // at one speed: a bump sets the speed to 0 while the rise
            // still had some left, so the two disagree there. An arc that
            // ends too soon after its top to fit the fall can't show it,
            // and isn't measured.
            && match (fit(ys, 0, top + 1), fit(ys, top, pre_cap)) {
                (Some((_, r1, r2)), Some((_, f1, f2))) => {
                    let k = top as f64;
                    ((r1 + 2.0 * r2 * k) - (f1 + 2.0 * f2 * k)).abs() <= 0.75
                }
                _ => false,
            }
        {
            gravity = Some(2.0 * c2);
            speed = Some(speed_of(c1, c2));
        }
        jumps.push(Jump {
            frame: f,
            hold,
            rise,
            airtime: end as u32,
            takeoff_vx,
            gravity,
            speed,
            gravity_held,
            speed_held,
            max_fall,
        });
    }
    let airborne = |f: u64| in_air.iter().any(|&(a, b)| f >= a && f <= b + 1);

    // Runs on the ground.
    let dir = |p: u8| match (p & pad.left != 0, p & pad.right != 0) {
        (true, false) => -1i8,
        (false, true) => 1,
        _ => 0,
    };
    let mut runs = Vec::new();
    let mut i = 0;
    while i < frames.len() {
        let f = frames[i];
        let d = dir(pad_at(f));
        let running = pad.run != 0 && pad_at(f) & pad.run != 0;
        if d == 0 || airborne(f) || !still_y(f) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j + 1 < frames.len() {
            let g = frames[j + 1];
            let r = pad.run != 0 && pad_at(g) & pad.run != 0;
            if g != frames[j] + 1
                || dir(pad_at(g)) != d
                || r != running
                || airborne(g)
                || !still_y(g)
            {
                break;
            }
            j += 1;
        }
        let n = j - i + 1;
        if n >= MIN_RUN {
            let half = n / 2;
            let (xe, _) = pos[&frames[j]];
            let (xm, _) = pos[&frames[j - half]];
            let speed = (xe - xm).abs() as f64 / half as f64;
            let from_standing = dir(pad_at(f - 1)) == 0
                && matches!((at(f - 2), at(f - 1)), (Some(a), Some(b)) if a.0 == b.0);
            let acceleration = if from_standing && speed > 0.0 {
                let (xs, _) = pos[&(f - 1)];
                let short = speed * n as f64 - (xe - xs).abs() as f64;
                (short > speed / 2.0 + 0.75).then(|| speed * speed / (2.0 * (short + speed / 2.0)))
            } else {
                None
            };
            runs.push(Run {
                frame: f,
                frames: n,
                running,
                speed,
                acceleration,
                from_standing,
            });
        }
        i = j + 1;
    }

    // Enemies: tracks bound to RAM that aren't the player.
    let mut enemies = Vec::new();
    for t in tracks.iter().filter(|t| !t.player && t.x.is_some()) {
        let mut moves: Vec<(i64, i64)> = Vec::new();
        let (mut sum, mut pairs) = (0i64, 0usize);
        for w in t.positions.windows(2) {
            if w[1][0] != w[0][0] + 1 {
                continue;
            }
            let dx = w[1][1] - w[0][1];
            if dx.abs() > 8 {
                continue;
            }
            sum += dx.abs();
            pairs += 1;
            if dx != 0 {
                moves.push((w[1][0], dx.abs()));
            }
        }
        if moves.len() < 8 || pairs < 16 {
            continue;
        }
        let step = mode(moves.iter().map(|m| m.1));
        let period = mode(
            moves
                .windows(2)
                .map(|w| w[1].0 - w[0].0)
                .filter(|&d| (1..=16).contains(&d)),
        )
        .map(|p| p as u32);
        enemies.push(Enemy {
            track: t.track,
            id: t.id,
            palette: t.palette,
            frames: pairs,
            step,
            period,
            speed: sum as f64 / pairs as f64,
        });
    }
    Feel {
        runs,
        jumps,
        enemies,
        player_frames: pos.len(),
    }
}

/// The part of an arc (heights from the takeoff) one gravity explains, and
/// whether it ended at a bump overhead. Gravity only ever adds to the
/// drop a frame (`dy`), by at most a pixel or two in whole pixels, so the
/// arc ends where `dy` falls by two or more (a bounce off an enemy or a
/// spring), where it stops falling (a landing, on a block or an enemy, at
/// any height), or, while rising, where it jumps by more than two (a
/// block overhead stopped the rise). The arc keeps the frame before.
pub fn free_flight(ys: &[i64]) -> (&[i64], bool) {
    let dys: Vec<i64> = ys.windows(2).map(|w| w[1] - w[0]).collect();
    if let Some(k) = dys.iter().position(|d| d.abs() > 12) {
        // A jump no arc makes: a wrap off the screen or a warp.
        return free_flight(&ys[..=k]);
    }
    for k in 1..dys.len() {
        let (d0, d1) = (dys[k - 1], dys[k]);
        if d1 < d0 - 1 || (d0 > 0 && d1 <= 0) {
            return (&ys[..=k], false);
        }
        if d0 < 0 && d1 - d0 > 2 {
            return (&ys[..=k], true);
        }
    }
    (ys, false)
}

/// The most common value, the smallest on a tie.
pub fn mode(values: impl Iterator<Item = i64>) -> Option<i64> {
    let mut count: BTreeMap<i64, usize> = BTreeMap::new();
    for v in values {
        *count.entry(v).or_insert(0) += 1;
    }
    let best = count.values().copied().max()?;
    count.into_iter().find(|(_, n)| *n == best).map(|(v, _)| v)
}

/// A least-squares fit of `y = c0 + c1 k + c2 k^2` to `ys[from..to]`,
/// with `k` the index. `None` with fewer than four points.
pub fn fit(ys: &[i64], from: usize, to: usize) -> Option<(f64, f64, f64)> {
    let to = to.min(ys.len());
    if to < from + 4 {
        return None;
    }
    // Normal equations for 1, k, k^2.
    let mut s = [0f64; 5];
    let mut t = [0f64; 3];
    for (k, &y) in ys.iter().enumerate().take(to).skip(from) {
        let k = k as f64;
        let y = y as f64;
        let mut p = 1.0;
        for v in &mut s {
            *v += p;
            p *= k;
        }
        t[0] += y;
        t[1] += y * k;
        t[2] += y * k * k;
    }
    let m = [[s[0], s[1], s[2]], [s[1], s[2], s[3]], [s[2], s[3], s[4]]];
    solve3(m, t)
}

fn solve3(mut m: [[f64; 3]; 3], mut v: [f64; 3]) -> Option<(f64, f64, f64)> {
    for c in 0..3 {
        let p = (c..3).max_by(|&a, &b| m[a][c].abs().total_cmp(&m[b][c].abs()))?;
        if m[p][c].abs() < 1e-9 {
            return None;
        }
        m.swap(c, p);
        v.swap(c, p);
        for r in 0..3 {
            if r != c {
                let f = m[r][c] / m[c][c];
                let pivot = m[c];
                for (x, p) in m[r].iter_mut().zip(pivot) {
                    *x -= f * p;
                }
                v[r] -= f * v[c];
            }
        }
    }
    Some((v[0] / m[0][0], v[1] / m[1][1], v[2] / m[2][2]))
}

/// The grid a measured quantity is rounded to, as a denominator: the NES
/// games' sub-pixels are sixteenths for speeds and 256ths for gravities
/// and accelerations.
pub fn grid(unit: &str) -> i64 {
    match unit {
        "px/frame^2" => 256,
        "px/frame" => 16,
        _ => 1,
    }
}

/// `v` rounded to the nearest `1/den`.
pub fn round_to(v: f64, den: i64) -> f64 {
    (v * den as f64).round() / den as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fitted_arc_gives_its_gravity_and_speed() {
        // A jump at -5 with gravity 0.25, speed added after gravity, in
        // whole pixels as a track has it.
        let (mut y, mut vy) = (0.0f64, -5.0f64);
        let mut ys = vec![0];
        for _ in 0..30 {
            vy += 0.25;
            y += vy;
            ys.push(y.floor() as i64);
        }
        let (_, c1, c2) = fit(&ys, 0, ys.len()).unwrap();
        assert!((2.0 * c2 - 0.25).abs() < 0.01, "{c2}");
        assert!((c1 - c2 + 5.0).abs() < 0.1, "{c1}");
        assert_eq!(round_to(2.0 * c2, 256), 0.25);
    }

    /// A made-up arc: speed `v`, gravity `g`, whole pixels, `n` frames.
    fn arc(v: f64, g: f64, n: usize) -> Vec<i64> {
        let (mut y, mut vy) = (0.0f64, v);
        let mut ys = vec![0];
        for _ in 0..n {
            vy += g;
            y += vy;
            ys.push(y.floor() as i64);
        }
        ys
    }

    #[test]
    fn an_arc_is_cut_where_it_lands_on_something() {
        // Up at -4, down with 0.375, then a stomp sends it up at -3 again.
        let mut ys = arc(-4.0, 0.375, 24);
        let last = *ys.last().unwrap();
        ys.extend(arc(-3.0, 0.375, 10).iter().skip(1).map(|y| y + last));
        let (cut, bumped) = free_flight(&ys);
        assert!(!bumped);
        assert_eq!(cut.len(), 25, "{ys:?}");
        let (_, _, c2) = fit(cut, 11, cut.len()).unwrap();
        assert!((2.0 * c2 - 0.375).abs() < 0.03, "{c2}");
        // The whole arc, bounce and all, measures far less.
        let (_, _, c2) = fit(&ys, 11, ys.len()).unwrap();
        assert!(2.0 * c2 < 0.3, "{c2}");
        // A bump overhead: rising at -4, then stopped dead.
        let mut ys = arc(-4.0, 0.125, 4);
        let last = *ys.last().unwrap();
        ys.extend([last, last + 1, last + 2]);
        assert!(free_flight(&ys).1);
        // A plain arc is kept whole.
        let ys = arc(-5.0, 0.25, 30);
        assert_eq!(free_flight(&ys).0.len(), ys.len());
    }

    #[test]
    fn mode_picks_the_most_common() {
        assert_eq!(mode([2, 1, 2, 3].into_iter()), Some(2));
        assert_eq!(mode(std::iter::empty()), None);
    }
}
