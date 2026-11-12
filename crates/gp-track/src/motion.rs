//! Speed and period of tracks, and tracks grouped into kinds (docs/formats/
//! tracks.md, "Motion").

use serde::Serialize;

use crate::TrackReport;

/// Changes of more than this many pixels between two frames are a wrap or a
/// respawn, not motion.
const JUMP: i64 = 128;

/// How one track moved, from its positions.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Motion {
    /// The mean absolute change of x a frame, over the pairs of consecutive
    /// frames it was drawn in, jumps left out. Frames it stood still count.
    pub speed_x: Option<f64>,
    pub speed_y: Option<f64>,
    /// Times it turned back along x.
    pub turns: usize,
    /// Frames between every second turn back along x, the mean.
    pub period: Option<f64>,
}

/// `[frame, x, y]` rows in frame order.
pub fn measure(positions: &[[i64; 3]]) -> Motion {
    let (mut sx, mut sy, mut n) = (0i64, 0i64, 0usize);
    let mut dir = 0i64;
    let mut turn_frames: Vec<i64> = Vec::new();
    for w in positions.windows(2) {
        if w[1][0] != w[0][0] + 1 {
            continue;
        }
        let (dx, dy) = (w[1][1] - w[0][1], w[1][2] - w[0][2]);
        if dx.abs() > JUMP || dy.abs() > JUMP {
            continue;
        }
        sx += dx.abs();
        sy += dy.abs();
        n += 1;
        if dx != 0 {
            let d = dx.signum();
            if dir != 0 && d != dir {
                turn_frames.push(w[1][0]);
            }
            dir = d;
        }
    }
    let period = (turn_frames.len() >= 3).then(|| {
        let spans: Vec<i64> = turn_frames.windows(3).map(|t| t[2] - t[0]).collect();
        spans.iter().sum::<i64>() as f64 / spans.len() as f64
    });
    Motion {
        speed_x: (n > 0).then(|| sx as f64 / n as f64),
        speed_y: (n > 0).then(|| sy as f64 / n as f64),
        turns: turn_frames.len(),
        period,
    }
}

/// Tracks of one look, and their motion together.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Kind {
    /// The slot ID the tracks share, when they have one.
    pub id: Option<u8>,
    pub palette: u8,
    pub tiles: Vec<u8>,
    pub tracks: usize,
    pub frames: usize,
    /// The frames-weighted mean of its tracks' speeds.
    pub speed_x: Option<f64>,
    pub speed_y: Option<f64>,
    pub period: Option<f64>,
}

/// Groups tracks by slot ID (palette alone for tracks with none), skipping
/// the player. The tiles are every tile any of them was drawn with.
pub fn kinds(tracks: &[TrackReport]) -> Vec<Kind> {
    let mut out: Vec<(Kind, f64, f64, usize, f64, usize)> = Vec::new();
    for t in tracks.iter().filter(|t| !t.player) {
        let k = out
            .iter_mut()
            .find(|(k, ..)| k.id == t.id && k.palette == t.palette);
        let slot = match k {
            Some(k) => k,
            None => {
                out.push((
                    Kind {
                        id: t.id,
                        palette: t.palette,
                        tiles: Vec::new(),
                        tracks: 0,
                        frames: 0,
                        speed_x: None,
                        speed_y: None,
                        period: None,
                    },
                    0.0,
                    0.0,
                    0,
                    0.0,
                    0,
                ));
                out.last_mut().unwrap()
            }
        };
        let w = t.seen as f64;
        slot.0.tracks += 1;
        slot.0.frames += t.seen;
        for tile in &t.tiles {
            if let Err(i) = slot.0.tiles.binary_search(tile) {
                slot.0.tiles.insert(i, *tile);
            }
        }
        if let Some(s) = t.motion.speed_x {
            slot.1 += s * w;
            slot.3 += t.seen;
        }
        if let Some(s) = t.motion.speed_y {
            slot.2 += s * w;
        }
        if let Some(p) = t.motion.period {
            slot.4 += p;
            slot.5 += 1;
        }
    }
    let mut kinds: Vec<Kind> = out
        .into_iter()
        .map(|(mut k, sx, sy, w, p, np)| {
            if w > 0 {
                k.speed_x = Some(sx / w as f64);
                k.speed_y = Some(sy / w as f64);
            }
            if np > 0 {
                k.period = Some(p / np as f64);
            }
            k
        })
        .collect();
    kinds.sort_by(|a, b| b.frames.cmp(&a.frames).then(a.id.cmp(&b.id)));
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_and_period_of_a_pacing_object() {
        // Half a pixel a frame, turning every 40 frames: a period of 80.
        let mut x = 100i64;
        let mut dx = 1i64;
        let mut rows = Vec::new();
        for f in 0..400i64 {
            if f % 2 == 0 {
                x += dx;
            }
            if f % 40 == 39 {
                dx = -dx;
            }
            rows.push([f, x, 50]);
        }
        let m = measure(&rows);
        assert!((m.speed_x.unwrap() - 0.5).abs() < 0.01);
        assert_eq!(m.speed_y, Some(0.0));
        assert!((m.period.unwrap() - 80.0).abs() < 2.1, "{:?}", m.period);
    }

    #[test]
    fn jumps_and_gaps_are_not_motion() {
        let m = measure(&[[1, 10, 0], [2, 11, 0], [3, 250, 0], [5, 400, 0]]);
        assert_eq!(m.speed_x, Some(1.0));
    }
}
