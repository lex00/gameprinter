//! Press sequences: one controller byte a frame, the seeded generator the
//! search draws them from, the mutations it makes, and the spans clips
//! write them as. Moved here from `gp capture dense`, which with `gp
//! capture steer` and `gp clips matrix` uses them; the same seed gives the
//! same sequences in every one of them.

use gp_evidence::input_log::button;
use gp_runtime::InputFrame;

/// The buttons the search presses: no Start or Select, which pause or do
/// nothing in a level.
pub const CHORDS: &[u8] = &[
    0,
    button::RIGHT,
    button::RIGHT | button::B,
    button::RIGHT | button::A,
    button::RIGHT | button::A | button::B,
    button::LEFT,
    button::LEFT | button::B,
    button::LEFT | button::A,
    button::LEFT | button::A | button::B,
    button::A,
    button::B,
    button::A | button::B,
    button::DOWN,
    button::DOWN | button::B,
    button::UP,
];
pub const PLAY: &[u8] = &[
    button::A,
    button::B,
    button::UP,
    button::DOWN,
    button::LEFT,
    button::RIGHT,
];

/// SplitMix64 (Steele, Lea and Flood, OOPSLA 2014): small, fixed and the
/// same on every machine.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    // A generator's next number, not an iterator's next item.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    /// A number in `lo..=hi`.
    pub fn within(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi.saturating_sub(lo) + 1)
    }
}

/// Drops the second of two opposite directions held together, which a
/// pad can't press.
pub fn legal(p: u8) -> u8 {
    let mut p = p;
    if p & button::LEFT != 0 && p & button::RIGHT != 0 {
        p &= !button::LEFT;
    }
    if p & button::UP != 0 && p & button::DOWN != 0 {
        p &= !button::UP;
    }
    p & !(button::START | button::SELECT)
}

/// One mutation of a press sequence, `min..=max` frames long. `other` is
/// another kept sequence from the same start, for a splice.
pub fn mutate(rng: &mut Rng, pads: &[u8], other: Option<&[u8]>, min: usize, max: usize) -> Vec<u8> {
    let mut p = pads.to_vec();
    let rounds = 1 + rng.below(4);
    for _ in 0..rounds {
        let n = p.len().max(1);
        let at = rng.below(n);
        let len = rng.within(1, 40).min(n - at);
        match rng.below(6) {
            // Toggle one button over a span.
            0 => {
                let b = PLAY[rng.below(PLAY.len())];
                for x in &mut p[at..at + len] {
                    *x ^= b;
                }
            }
            // A chord held over a span.
            1 | 2 => {
                let c = CHORDS[rng.below(CHORDS.len())];
                for x in &mut p[at..at + len] {
                    *x = c;
                }
            }
            // Frames of nothing put in, or taken out.
            3 => {
                if rng.below(2) == 0 {
                    p.splice(at..at, std::iter::repeat_n(0, len));
                } else {
                    p.drain(at..at + len);
                }
            }
            // Longer or shorter.
            4 => {
                let to = rng.within(min, max);
                let last = p.last().copied().unwrap_or(0);
                p.resize(to, last);
            }
            // This one's start and the other's end.
            _ => {
                if let Some(o) = other.filter(|o| !o.is_empty()) {
                    let cut = rng.below(o.len());
                    p.truncate(at);
                    p.extend_from_slice(&o[cut.min(o.len())..]);
                }
            }
        }
    }
    p.truncate(max);
    while p.len() < min {
        p.push(p.last().copied().unwrap_or(0));
    }
    p.iter().map(|&x| legal(x)).collect()
}

/// How long each run of a start set is: a minute at 60 frames a second.
pub const START_FRAMES: usize = 3600;

/// The chords a seeded run of a start set picks from, Select and Start
/// among them (each a five-frame tap).
const START_CHORDS: &[u8] = &[
    button::RIGHT,
    button::RIGHT | button::B,
    button::RIGHT | button::A,
    button::RIGHT | button::A | button::B,
    button::LEFT,
    button::LEFT | button::A,
    button::A,
    button::B,
    button::DOWN,
    button::UP,
    button::UP | button::A,
    button::DOWN | button::B,
    button::START,
    button::SELECT,
];

/// Xorshift64 (Marsaglia, "Xorshift RNGs", 2003; shifts 13, 7, 17), the
/// start set's own generator, seeded by a run's number through the golden
/// ratio constant SplitMix64 uses.
struct Xorshift(u64);

impl Xorshift {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Holds `b` over frames `a..=b_end` (inclusive, as clip spans are).
fn hold(pads: &mut [u8], a: usize, b_end: usize, b: u8) {
    let end = (b_end + 1).min(pads.len());
    if a < end {
        pads[a..end].fill(b);
    }
}

/// One seeded run of random presses, `frames` long: Start tapped at frames
/// 60 and 200 (past a title screen), then from frame 260 segments of 8 to
/// 60 frames. Each segment is left empty one time in five, else holds a
/// chord of [`START_CHORDS`]; a Select or Start pick stays one a third of
/// the time, as a five-frame tap, and is Right otherwise.
fn random_run(seed: u64, frames: usize) -> Vec<u8> {
    let mut pads = vec![0; frames];
    hold(&mut pads, 60, 64, button::START);
    hold(&mut pads, 200, 204, button::START);
    let mut r = Xorshift::new(seed);
    let mut f = 260;
    while f + 8 < frames {
        let len = 8 + (r.next() % 53) as usize;
        let end = (f + len).min(frames - 1);
        if r.next() % 20 >= 4 {
            let mut c = START_CHORDS[(r.next() % START_CHORDS.len() as u64) as usize];
            let tap = c == button::START || c == button::SELECT;
            if tap && !r.next().is_multiple_of(3) {
                c = button::RIGHT;
            }
            if c == button::START || c == button::SELECT {
                hold(&mut pads, f, (f + 4).min(end), c);
            } else {
                hold(&mut pads, f, end, c);
            }
        }
        f = end + 1;
    }
    pads
}

/// A seeded run of random presses for a start already in play, `frames`
/// long: from frame 0, segments of 8 to 60 frames, each left empty one
/// time in five and otherwise holding a chord of [`START_CHORDS`] other
/// than Start and Select, which would pause or leave play.
pub fn play_run(seed: u64, frames: usize) -> Vec<u8> {
    let mut pads = vec![0; frames];
    let chords: Vec<u8> = START_CHORDS
        .iter()
        .copied()
        .filter(|&c| c != button::START && c != button::SELECT)
        .collect();
    let mut r = Xorshift::new(seed);
    let mut f = 0;
    while f + 8 < frames {
        let len = 8 + (r.next() % 53) as usize;
        let end = (f + len).min(frames - 1);
        if r.next() % 20 >= 4 {
            let c = chords[(r.next() % chords.len() as u64) as usize];
            hold(&mut pads, f, end, c);
        }
        f = end + 1;
    }
    pads
}

/// Stage 0's own starts (docs/plan-extraction-process.md, stage 0): runs
/// from power-on that need nothing but the ROM, each `frames` long, as one
/// controller byte a frame with its name. `idle` presses nothing; `start`
/// taps Start at frames 120, 300, 480 and 660 to get past title and menu
/// screens; `rand1` to `randN` are [`random_run`]s seeded `seed` to
/// `seed + N - 1`, so the first N are the same whatever `runs` is.
pub fn start_set(runs: usize, seed: u64, frames: usize) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![("idle".to_owned(), vec![0; frames])];
    let mut start = vec![0; frames];
    for k in 0..4 {
        let at = 120 + 180 * k;
        hold(&mut start, at, at + 4, button::START);
    }
    out.push(("start".to_owned(), start));
    for k in 0..runs as u64 {
        out.push((
            format!("rand{}", k + 1),
            random_run(seed.wrapping_add(k), frames),
        ));
    }
    out
}

/// Press spans of a sequence, as clips write them: `right+b@0-44`.
pub fn spans(pads: &[u8]) -> Vec<String> {
    let name = |p: u8| -> String {
        let mut v = Vec::new();
        for (b, n) in [
            (button::A, "a"),
            (button::B, "b"),
            (button::SELECT, "select"),
            (button::START, "start"),
            (button::UP, "up"),
            (button::DOWN, "down"),
            (button::LEFT, "left"),
            (button::RIGHT, "right"),
        ] {
            if p & b != 0 {
                v.push(n);
            }
        }
        v.join("+")
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

/// A sequence as the runtime protocol's input frames.
pub fn inputs_of(pads: &[u8]) -> Vec<InputFrame> {
    pads.iter()
        .map(|&p| InputFrame::pads([p, 0, 0, 0]))
        .collect()
}

/// A sequence as an input log's frames, as gp-events runs take them.
pub fn log_frames(pads: &[u8]) -> Vec<gp_evidence::input_log::InputFrame> {
    pads.iter()
        .map(|&p| gp_evidence::input_log::InputFrame {
            polled: true,
            pads: [p, 0, 0, 0],
            reset: false,
        })
        .collect()
}

#[cfg(test)]
mod play_tests {
    use super::*;

    #[test]
    fn a_play_run_never_presses_start_or_select_and_repeats_by_seed() {
        let a = play_run(5, 2000);
        assert!(a.iter().all(|&p| p & (button::START | button::SELECT) == 0));
        assert!(a.iter().any(|&p| p != 0));
        assert_eq!(a, play_run(5, 2000));
        assert_ne!(a, play_run(6, 2000));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_mutants() {
        let base: Vec<u8> = vec![button::RIGHT; 90];
        let run = |seed| {
            let mut r = Rng::new(seed);
            (0..20)
                .map(|_| mutate(&mut r, &base, Some(&base), 60, 180))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
        for p in run(7) {
            assert!((60..=180).contains(&p.len()));
            assert!(p.iter().all(|&x| legal(x) == x));
        }
    }

    #[test]
    fn a_start_set_is_the_same_for_its_seed_and_keeps_its_first_runs() {
        let a = start_set(4, 1, START_FRAMES);
        assert_eq!(a, start_set(4, 1, START_FRAMES));
        assert_ne!(a, start_set(4, 2, START_FRAMES));
        assert_eq!(a[..], start_set(6, 1, START_FRAMES)[..6]);
        let names: Vec<&str> = a.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["idle", "start", "rand1", "rand2", "rand3", "rand4"]);
        assert!(a.iter().all(|(_, p)| p.len() == START_FRAMES));
        assert!(a[0].1.iter().all(|&p| p == 0));
        assert_eq!(spans(&a[1].1)[0], "start@120-124");
        // The set stage 0's numbers were first measured with (2026-11-01).
        assert_eq!(
            spans(&a[2].1)[..6],
            [
                "start@60-64",
                "start@200-204",
                "right@260-318",
                "b+down@319-356",
                "b+right@357-373",
                "down@402-447"
            ]
        );
        assert_eq!(spans(&a[3].1)[2], "a@260-268");
    }

    #[test]
    fn presses_read_back_as_the_spans_they_were() {
        let pads = [
            0,
            0,
            button::RIGHT,
            button::RIGHT,
            button::RIGHT | button::A,
            0,
        ];
        let s = spans(&pads);
        assert_eq!(s, vec!["right@2-3", "a+right@4-4"]);
        let refs: Vec<&str> = s.iter().map(String::as_str).collect();
        let log = gp_evidence::input_log::generate(pads.len(), &refs).unwrap();
        let back: Vec<u8> = log.frames.iter().map(|f| f.pads[0]).collect();
        assert_eq!(back, pads);
    }
}
