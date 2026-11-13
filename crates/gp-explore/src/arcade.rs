//! The arcade board's chord palette (port wave 6): the presses a search,
//! stage 0's starts and `gp mechanic demo` pick from on Tapper's MCR
//! board, as [`crate::presses`] has them for the NES pad.
//!
//! A frame's controls are one byte, as an arcade input log's columns
//! `CSRLDUT` give them, a bit each from bit 0: coin, start, right, left,
//! down, up and tap (`gp_arcade::Controls::from_bits`'s order). The span
//! names are the corpus's (`coin`, `start1`, `right`, `left`, `down`, `up`,
//! `tap`), so a sequence writes as a `.presses` file's spans.

use crate::presses::{Palette, Rng};

pub const COIN: u8 = 1 << 0;
pub const START: u8 = 1 << 1;
pub const RIGHT: u8 = 1 << 2;
pub const LEFT: u8 = 1 << 3;
pub const DOWN: u8 = 1 << 4;
pub const UP: u8 = 1 << 5;
pub const TAP: u8 = 1 << 6;

/// The buttons a search presses in play: the stick each way and the tap,
/// alone and together; no coin or start, which leave play or do nothing.
pub const CHORDS: &[u8] = &[
    0,
    UP,
    DOWN,
    LEFT,
    RIGHT,
    TAP,
    UP | TAP,
    DOWN | TAP,
    LEFT | TAP,
    RIGHT | TAP,
];

/// One button each, for a search's single presses.
pub const PLAY: &[u8] = &[UP, DOWN, LEFT, RIGHT, TAP];

/// A press the board can take in play: the stick one way on each axis
/// (right over left, down over up, as the NES's rule keeps one), no coin
/// and no start.
pub fn legal(p: u8) -> u8 {
    let mut p = p;
    if p & LEFT != 0 && p & RIGHT != 0 {
        p &= !LEFT;
    }
    if p & UP != 0 && p & DOWN != 0 {
        p &= !UP;
    }
    p & !(COIN | START)
}

/// The palette a search mutates arcade presses with
/// ([`crate::presses::mutate_with`]).
pub const PALETTE: Palette = Palette {
    chords: CHORDS,
    play: PLAY,
    legal,
};

/// Each button's name in a span, by bit.
pub const NAMES: [(u8, &str); 7] = [
    (COIN, "coin"),
    (START, "start1"),
    (RIGHT, "right"),
    (LEFT, "left"),
    (DOWN, "down"),
    (UP, "up"),
    (TAP, "tap"),
];

/// How long each run of a start set is: a minute at 30 frames a second.
pub const START_FRAMES: usize = 1800;

fn hold(pads: &mut [u8], a: usize, b_end: usize, b: u8) {
    let end = (b_end + 1).min(pads.len());
    if a < end {
        pads[a..end].fill(b);
    }
}

/// A coin at frame `at` and Start 60 frames later, each held 6 frames (a
/// coin switch needs a few frames closed; Tapper takes it in attract).
fn coin_and_start(pads: &mut [u8], at: usize) {
    hold(pads, at, at + 5, COIN);
    hold(pads, at + 60, at + 65, START);
}

/// Stage 0's starts on the arcade board: `idle` presses nothing; `start`
/// drops a coin at frame 120 and presses Start at 180, again at 600 and
/// 660 should the first be missed; `rand1` to `randN` do the same, then
/// from frame 720 hold [`CHORDS`] for 8 to 60 frames at a time (one
/// segment in five left empty), seeded `seed` on.
pub fn start_set(runs: usize, seed: u64, frames: usize) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![("idle".to_owned(), vec![0; frames])];
    let mut start = vec![0; frames];
    coin_and_start(&mut start, 120);
    coin_and_start(&mut start, 600);
    out.push(("start".to_owned(), start.clone()));
    for k in 0..runs as u64 {
        let mut pads = start.clone();
        let mut r = Rng::new(seed.wrapping_add(k));
        let mut f = 720;
        while f + 8 < frames {
            let end = (f + r.within(8, 60)).min(frames - 1);
            if r.below(5) != 0 {
                hold(&mut pads, f, end, CHORDS[1 + r.below(CHORDS.len() - 1)]);
            }
            f = end + 1;
        }
        out.push((format!("rand{}", k + 1), pads));
    }
    out
}

/// Press spans of a sequence, as an arcade clip writes them:
/// `up+tap@0-44`.
pub fn spans(pads: &[u8]) -> Vec<String> {
    let name = |p: u8| {
        NAMES
            .iter()
            .filter(|(b, _)| p & b != 0)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join("+")
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < pads.len() {
        let mut j = i;
        while j + 1 < pads.len() && pads[j + 1] == pads[i] {
            j += 1;
        }
        if pads[i] != 0 {
            out.push(format!("{}@{i}-{j}", name(pads[i])));
        }
        i = j + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_coin_then_play_the_stick_and_tap() {
        let set = start_set(2, 7, 1800);
        assert_eq!(set.len(), 4);
        assert!(set[0].1.iter().all(|&p| p == 0));
        let start = &set[1].1;
        assert_eq!((start[120], start[180], start[600]), (COIN, START, COIN));
        let r = &set[2].1;
        assert_eq!(&r[..720], &start[..720]);
        assert!(r[720..].iter().all(|&p| p & (COIN | START) == 0));
        assert!(r[720..].iter().any(|&p| p & TAP != 0));
        assert_eq!(start_set(2, 7, 1800)[3].1, set[3].1, "seeded");
        assert_ne!(set[2].1, set[3].1);
        assert_eq!(spans(&start[..200]), ["coin@120-125", "start1@180-185"]);
        assert_eq!(spans(&[UP | TAP, UP | TAP, 0]), ["up+tap@0-1"]);
    }
}
