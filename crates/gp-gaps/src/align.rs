//! Finding the same moment in two runs whose paths diverge (docs/gaps.md,
//! "Aligning by position"). The model's player leaves the original's path
//! within seconds of the same input, so the same frame number is soon a
//! different place. A moment is matched instead by where the player is:
//! the same level, the nearest position, and the nearest frame among equals.
//! Detect only; nothing here changes either run.

use serde::Serialize;

/// Where the player was on one frame of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pos {
    pub frame: u64,
    /// The level's name, or empty when the run doesn't say.
    pub level: String,
    /// World pixels.
    pub x: i32,
    pub y: i32,
}

/// The moment of a track that best matches a position, and how far the
/// two are apart.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Match {
    /// The matched frame of the track.
    pub frame: u64,
    /// Pixels between the matched position and the one asked for (x plus
    /// half of y, as levels run sideways more than up).
    pub distance: i64,
    /// The asked frame minus the matched one: negative when the track gets
    /// there later (it runs behind), positive when it got there earlier.
    pub frames_apart: i64,
    pub same_level: bool,
}

/// The weighted distance between two positions.
pub fn distance(a: &Pos, b: &Pos) -> i64 {
    i64::from((a.x - b.x).abs()) + i64::from((a.y - b.y).abs()) / 2
}

/// The frame of `track` nearest `at` by position: on `at`'s level when the
/// track was ever there (levels compare when both name one), then the
/// smallest [`distance`], then the frame nearest `at.frame`.
pub fn nearest(track: &[Pos], at: &Pos) -> Option<Match> {
    let same = |p: &Pos| at.level.is_empty() || p.level.is_empty() || p.level == at.level;
    let on_level = track.iter().any(same);
    track
        .iter()
        .filter(|p| !on_level || same(p))
        .min_by_key(|p| (distance(p, at), p.frame.abs_diff(at.frame)))
        .map(|p| Match {
            frame: p.frame,
            distance: distance(p, at),
            frames_apart: at.frame as i64 - p.frame as i64,
            same_level: same(p),
        })
}

/// How far two runs have drifted at one frame: the distance between the
/// players on that frame, and the frame of `other` that best matches `at`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Drift {
    pub same_frame_distance: Option<i64>,
    pub same_frame_level: Option<bool>,
    pub matched: Option<Match>,
}

pub fn drift(at: &Pos, other: &[Pos]) -> Drift {
    let same = other.iter().find(|p| p.frame == at.frame);
    Drift {
        same_frame_distance: same.map(|p| distance(p, at)),
        same_frame_level: same.map(|p| p.level == at.level),
        matched: nearest(other, at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(frame: u64, level: &str, x: i32) -> Pos {
        Pos {
            frame,
            level: level.into(),
            x,
            y: 160,
        }
    }

    #[test]
    fn the_matching_moment_is_by_level_and_position_not_frame() {
        // The other run lags: it reaches x 300 forty frames later.
        let other: Vec<Pos> = (0..200).map(|f| pos(f, "1-1", f as i32 * 2)).collect();
        let at = pos(110, "1-1", 300);
        let d = drift(&at, &other);
        assert_eq!(d.same_frame_distance, Some(80));
        let m = d.matched.unwrap();
        assert_eq!((m.frame, m.distance, m.frames_apart), (150, 0, -40));
        // A position on another level matches only on that level.
        let mut two = other.clone();
        two.push(pos(5, "1-2", 300));
        let m = nearest(&two, &pos(110, "1-2", 290)).unwrap();
        assert_eq!((m.frame, m.same_level), (5, true));
    }
}
