//! `gp movie diagnose`: where a replayed movie stops fitting the game.
//!
//! A tool-assisted run doesn't waste input. Every change of buttons the
//! author made was meant to be read by the game, so after recording, when
//! the runtime has replaced the polled flags with the frames the game really
//! read, an input change that the game never sees marks the point where our
//! replay and the author's emulator part ways. That point is often well
//! before the visible symptom, a death or a stuck screen (docs/movies.md).
//!
//! The report reads only the stored input log. Frames are numbered as
//! everywhere else: input log entry `n` is the input frame `n + 1` ran with.

use gp_evidence::InputLog;
use serde::Serialize;

/// How many frames of each kind the report lists.
pub const LISTED: usize = 20;

/// One input change the game didn't read as given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// The frame that ran with the new input.
    pub frame: u64,
    /// The buttons held on controllers 1 and 2 after the change.
    pub pads: [u8; 2],
    /// The next frame the game read the controllers in, if any.
    pub next_polled: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnosis {
    pub frames: u64,
    pub lag_frames: u64,
    /// Frames whose input differs from the frame before's.
    pub changes: u64,
    /// Changes on lag frames whose buttons are gone by the next frame the
    /// game reads: input the game never saw.
    pub wasted: Vec<Change>,
    pub wasted_count: u64,
    /// Changes on a lag frame that follows another lag frame, whether or not
    /// the buttons survive to the next read.
    pub in_lag_runs: Vec<Change>,
    pub in_lag_runs_count: u64,
}

/// Diagnoses an input log whose polled flags came from recording it.
pub fn diagnose(log: &InputLog) -> Diagnosis {
    let pads = |i: usize| -> [u8; 2] {
        let p = log.frames[i].pads;
        [p[0], p[1]]
    };
    let n = log.frames.len();
    // The next polled entry at or after each entry.
    let mut next_polled = vec![None; n + 1];
    for (i, frame) in log.frames.iter().enumerate().rev() {
        next_polled[i] = if frame.polled {
            Some(i)
        } else {
            next_polled[i + 1]
        };
    }
    let mut d = Diagnosis {
        frames: n as u64,
        lag_frames: log.lag_frames() as u64,
        changes: 0,
        wasted: Vec::new(),
        wasted_count: 0,
        in_lag_runs: Vec::new(),
        in_lag_runs_count: 0,
    };
    for (i, &next) in next_polled.iter().enumerate().take(n).skip(1) {
        if pads(i) == pads(i - 1) {
            continue;
        }
        d.changes += 1;
        if log.frames[i].polled {
            continue;
        }
        let change = Change {
            frame: i as u64 + 1,
            pads: pads(i),
            next_polled: next.map(|g| g as u64 + 1),
        };
        if next.is_none_or(|g| pads(g) != pads(i)) {
            d.wasted_count += 1;
            if d.wasted.len() < LISTED {
                d.wasted.push(change.clone());
            }
        }
        if !log.frames[i - 1].polled {
            d.in_lag_runs_count += 1;
            if d.in_lag_runs.len() < LISTED {
                d.in_lag_runs.push(change);
            }
        }
    }
    d
}

/// The report as text, one line per listed change.
pub fn text(d: &Diagnosis) -> String {
    let mut s = format!(
        "{} frames, {} lag, {} input changes\nwasted inputs (changed on a lag frame, gone by the next read): {}\nchanges inside a lag run: {}\n",
        d.frames, d.lag_frames, d.changes, d.wasted_count, d.in_lag_runs_count
    );
    let line = |c: &Change| {
        format!(
            "  frame {:>7}  pads {:02x} {:02x}  next read {}\n",
            c.frame,
            c.pads[0],
            c.pads[1],
            c.next_polled
                .map_or("never".to_owned(), |f| format!("at frame {f}"))
        )
    };
    if !d.wasted.is_empty() {
        s.push_str("first wasted inputs:\n");
        d.wasted.iter().for_each(|c| s.push_str(&line(c)));
    }
    if !d.in_lag_runs.is_empty() {
        s.push_str("first changes inside lag runs:\n");
        d.in_lag_runs.iter().for_each(|c| s.push_str(&line(c)));
    }
    s.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_evidence::InputFrame;

    fn log(frames: &[(bool, u8)]) -> InputLog {
        let mut log = InputLog::new(1);
        log.frames = frames
            .iter()
            .map(|&(polled, pad)| InputFrame {
                polled,
                pads: [pad, 0, 0, 0],
                reset: false,
            })
            .collect();
        log
    }

    #[test]
    fn a_press_released_before_the_next_read_is_wasted() {
        // Entry 2 presses A on a lag frame and entry 3 lets go before the
        // game reads again at entry 4.
        let d = diagnose(&log(&[
            (true, 0),
            (true, 0),
            (false, 1),
            (false, 0),
            (true, 0),
        ]));
        assert_eq!(d.changes, 2);
        // The press never reaches the game; the release does, since the
        // next read sees no buttons.
        assert_eq!(d.wasted_count, 1);
        assert_eq!(d.wasted[0].frame, 3);
        assert_eq!(d.wasted[0].next_polled, Some(5));
        assert_eq!(d.in_lag_runs_count, 1);
        assert_eq!(d.in_lag_runs[0].frame, 4);
    }

    #[test]
    fn a_press_held_into_the_next_read_isnt_wasted() {
        // Pressed on a lag frame, still held when the game reads: an early
        // press, which a TAS can make.
        let d = diagnose(&log(&[(true, 0), (false, 1), (true, 1), (true, 0)]));
        assert_eq!(d.changes, 2);
        assert_eq!(d.wasted_count, 0);
        assert_eq!(d.in_lag_runs_count, 0);
        // A change on the last frames with no read after it is wasted.
        let d = diagnose(&log(&[(true, 0), (false, 8)]));
        assert_eq!(d.wasted_count, 1);
        assert_eq!(d.wasted[0].next_polled, None);
        assert!(text(&d).contains("next read never"));
    }
}
