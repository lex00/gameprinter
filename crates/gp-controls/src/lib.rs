//! A controller's raw readings turned into the game's buttons
//! (docs/controls.md).
//!
//! A page that plays a game can only do one thing no module can: read the
//! browser's gamepads and keys. Everything after that is here, so the page
//! a person tests on and the page the public plays read a hand the same
//! way: which button is which ([`Layout`]), when a stick counts as pushed,
//! when a lever is pulled and let go ([`StickPull`]), and which buttons
//! each game frame reads from the changes since the last one
//! ([`Timeline`]). [`Controls`] holds the three together.
//!
//! Ported from mechanic/input-timeline.js and mechanic/screen.js
//! (`gamepadBits`, `secondStick`), which the owner tuned by hand on
//! 2026-11-22; the public page had its own rules until 2026-11-25 (a lever
//! held a fixed 700 ms: "the joystick pour is really slow in the browser
//! .... it was ok in testing locally").

use std::collections::BTreeMap;

/// How far a stick must be pushed along an axis to count as a direction.
pub const STICK_ON: f32 = 0.5;

/// A lever below this far from the middle is at rest.
pub const LEVER_REST: f32 = 0.12;

/// An axis must have been seen this near the middle once to count as a
/// stick: a trigger some pads report as an axis resting at -1 never is.
pub const AXIS_RESTED: f32 = 0.2;

/// The settings of the controls' timing, a person's own since controllers
/// differ.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// How far past a frame's own time its reading may look, up to now:
    /// 0 reads exactly at the frame's time.
    pub lead_ms: f64,
    /// How long a press is held on after it is let go.
    pub assist_ms: f64,
    /// How far a lever must move, as a percent of its reach, to count as
    /// a pull beginning or ending.
    pub stick_travel: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            lead_ms: 8.0,
            assist_ms: 0.0,
            stick_travel: 6.0,
        }
    }
}

impl Options {
    /// The settings kept inside their ranges; a value that isn't a number
    /// is the default.
    #[must_use]
    pub fn clamped(self) -> Self {
        let d = Self::default();
        let num =
            |v: f64, d: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { d };
        Self {
            lead_ms: num(self.lead_ms, d.lead_ms, 0.0, 50.0),
            assist_ms: num(self.assist_ms, d.assist_ms, 0.0, 100.0),
            stick_travel: num(self.stick_travel, d.stick_travel, 1.0, 50.0),
        }
    }
}

/// A lever read as a pull by which way it is travelling. It is pulled once
/// it has gone out `travel` of its reach from the lowest it has been since
/// the last pull, past a small rest zone; it is let go once it has come
/// back `travel` from the furthest it got. A lever read at a fixed halfway
/// point is let go only when it is most of the way home.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickPull {
    travel: f32,
    pulled: bool,
    /// While pulled, the furthest out; while not, the lowest since.
    mark: f32,
}

impl StickPull {
    #[must_use]
    pub fn new(travel_percent: f64) -> Self {
        Self {
            travel: (travel_percent / 100.0).clamp(0.01, 0.5) as f32,
            pulled: false,
            mark: 0.0,
        }
    }

    /// The lever is `m` of its reach from the middle (0 to 1): pulled or
    /// not.
    pub fn read(&mut self, m: f32) -> bool {
        if self.pulled {
            if m > self.mark {
                self.mark = m;
            }
            if m <= self.mark - self.travel || m < LEVER_REST {
                self.pulled = false;
                self.mark = m;
            }
        } else {
            if m < self.mark {
                self.mark = m;
            }
            if m >= self.mark + self.travel && m >= LEVER_REST {
                self.pulled = true;
                self.mark = m;
            }
        }
        self.pulled
    }
}

/// Which machine's buttons a gamepad gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// A, B, Select, Start, up, down, left, right: bits 0 to 7.
    #[default]
    Nes,
    /// An arcade cabinet's, in an arcade input log's column order: coin,
    /// start, right, left, down, up, the button (bits 0 to 6). A second
    /// stick is the button's lever, as a drink tap's handle was.
    Arcade,
}

/// One gamepad as the browser's standard layout reports it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RawPad {
    /// The browser's index for the pad: its lever's state is kept by it.
    pub index: u32,
    /// The pressed buttons, bit `i` for `buttons[i]`.
    pub buttons: u32,
    /// The first four axes: the left stick's x and y, the right stick's.
    /// An axis the pad hasn't got is NaN.
    pub axes: [f32; 4],
}

/// A pad's lever: the axes seen at rest and the pull.
#[derive(Debug, Clone)]
struct Lever {
    rested: [bool; 2],
    pull: StickPull,
}

/// Gamepads read as the machine's buttons. It keeps each pad's lever.
#[derive(Debug, Clone, Default)]
pub struct Pads {
    levers: BTreeMap<u32, Lever>,
}

impl Pads {
    /// Whether the pad's second stick is pulled any way.
    fn lever(&mut self, p: &RawPad, travel: f64) -> bool {
        let l = self.levers.entry(p.index).or_insert_with(|| Lever {
            rested: [false; 2],
            pull: StickPull::new(travel),
        });
        let want = StickPull::new(travel).travel;
        if (l.pull.travel - want).abs() > f32::EPSILON {
            l.pull = StickPull::new(travel);
        }
        // How far out the stick is, over the axes seen at rest.
        let mut m = 0.0f32;
        for (k, v) in p.axes[2..4].iter().enumerate() {
            if v.is_nan() {
                continue;
            }
            if v.abs() < AXIS_RESTED {
                l.rested[k] = true;
            }
            if l.rested[k] {
                m = m.max(v.abs());
            }
        }
        l.pull.read(m.min(1.0))
    }

    /// One pad's buttons in `layout`.
    fn one(&mut self, layout: Layout, p: &RawPad, travel: f64) -> u32 {
        let b = |i: u32| p.buttons & (1 << i) != 0;
        let axis = |i: usize| if p.axes[i].is_nan() { 0.0 } else { p.axes[i] };
        let (ax, ay) = (axis(0), axis(1));
        let (up, down) = (b(12) || ay < -STICK_ON, b(13) || ay > STICK_ON);
        let (left, right) = (b(14) || ax < -STICK_ON, b(15) || ax > STICK_ON);
        let bits: [bool; 8] = match layout {
            Layout::Arcade => {
                let lever = self.lever(p, travel);
                let button = b(0) || b(1) || b(2) || b(3) || lever;
                [b(8), b(9), right, left, down, up, button, false]
            }
            // A and B as the shell's table has them (gp_present::shell::
            // keys, STANDARD_BUTTONS: A on the bottom button, B on the left
            // and the right), which the public page has always played by.
            Layout::Nes => [b(0), b(2) || b(1), b(8), b(9), up, down, left, right],
        };
        bits.iter()
            .enumerate()
            .fold(0, |acc, (i, on)| acc | (u32::from(*on) << i))
    }

    /// The pads' buttons: the first connected pad is controller 1, in the
    /// low byte, the second controller 2, in the next (a cabinet has one
    /// set of controls: its second pad is not read). The pads are given
    /// in the browser's order, the connected ones only.
    pub fn bits(&mut self, layout: Layout, pads: &[RawPad], travel: f64) -> u32 {
        let mut bits = 0;
        let most = if layout == Layout::Arcade { 1 } else { 2 };
        for (n, p) in pads.iter().take(most).enumerate() {
            bits |= self.one(layout, p, travel) << (8 * n);
        }
        bits
    }
}

/// Every change of the buttons with the time it happened, and the buttons
/// each game frame reads, taken at that frame's own time.
///
/// A page that reads the controls once for each game frame it steps reads
/// them at whatever moment the browser ran the step. A late browser frame
/// steps several game frames in one go, and all of them read the same
/// buttons: a short press counts as several frames, or a release is seen
/// that many frames late. A press and release between two steps is never
/// seen.
#[derive(Debug, Clone)]
pub struct Timeline {
    pub opts: Options,
    /// Changes, oldest first: the buttons from `t` on.
    changes: Vec<(f64, u32)>,
    /// The time of the last frame taken.
    taken: f64,
    /// The buttons the last frame taken read.
    prev: u32,
    /// Each button's press still counted, by its bit: held now, or let go
    /// after the last frame's reading.
    open: BTreeMap<u32, Press>,
    /// The newest press let go, for the page's own measure.
    last: Option<LastPress>,
}

/// A press being counted: when it went down, when it came up, and the
/// frames that have read it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    down: f64,
    up: Option<f64>,
    frames: u32,
}

/// The newest press let go: its button, how long it was held and the
/// frames that read it; `counting` while its [`Press`] is still open, so a
/// frame that reads it after the release still counts.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LastPress {
    bit: u32,
    ms: f64,
    frames: u32,
    counting: bool,
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new(Options::default())
    }
}

impl Timeline {
    #[must_use]
    pub fn new(opts: Options) -> Self {
        Self {
            opts: opts.clamped(),
            changes: vec![(f64::NEG_INFINITY, 0)],
            taken: f64::NEG_INFINITY,
            prev: 0,
            open: BTreeMap::new(),
            last: None,
        }
    }

    /// The buttons are `bits` from time `t` on.
    pub fn set(&mut self, bits: u32, t: f64) {
        let (last_t, last) = self.changes[self.changes.len() - 1];
        if last == bits {
            return;
        }
        // A change stamped before the newest one goes just after it: times
        // from two clocks (an event's, a poll's) never reorder what
        // happened.
        self.changes.push((t.max(last_t), bits));
        for i in 0..32 {
            let b = 1u32 << i;
            if bits & b != 0 && last & b == 0 {
                // A new press of the newest one's button: that one's count
                // is done.
                if let Some(l) = self.last.as_mut().filter(|l| l.bit == b) {
                    l.counting = false;
                }
                self.open.insert(
                    b,
                    Press {
                        down: t,
                        up: None,
                        frames: 0,
                    },
                );
            }
            if bits & b == 0
                && last & b != 0
                && let Some(o) = self.open.get_mut(&b)
            {
                o.up = Some(t);
                self.last = Some(LastPress {
                    bit: b,
                    ms: t - o.down,
                    frames: o.frames,
                    counting: true,
                });
            }
        }
    }

    /// The newest press let go: the ms it was held and the frames that
    /// read it. None before the first.
    #[must_use]
    pub fn last_press(&self) -> Option<(f64, u32)> {
        self.last.map(|l| (l.ms, l.frames))
    }

    /// The buttons at time `t`, with each release put off by the assist.
    #[must_use]
    pub fn at(&self, t: f64) -> u32 {
        let mut bits = 0;
        for &(ct, b) in &self.changes {
            if ct > t {
                break;
            }
            bits = b;
        }
        let hold = self.opts.assist_ms;
        if hold > 0.0 {
            // A button let go less than the assist before t is still down.
            for i in (1..self.changes.len()).rev() {
                let (ct, b) = self.changes[i];
                if ct > t {
                    continue;
                }
                if ct <= t - hold {
                    break;
                }
                bits |= self.changes[i - 1].1 & !b;
            }
        }
        bits
    }

    /// The buttons the game frame at time `t` reads, at `now`. Frames are
    /// taken in order. A press that began and ended since the last frame
    /// taken is read once, so a quick tap isn't lost.
    pub fn take(&mut self, t: f64, now: f64) -> u32 {
        let read = self.taken.max(now.min(t + self.opts.lead_ms));
        let mut bits = self.at(read);
        // Buttons that went down after the last frame's reading and are up
        // again by this one: read once, unless the last frame had them
        // down.
        let mut tapped = 0;
        for &(ct, b) in &self.changes {
            if ct > read {
                break;
            }
            if ct > self.taken {
                tapped |= b;
            }
        }
        bits |= tapped & !self.prev;
        let last = &mut self.last;
        self.open.retain(|&b, o| {
            if bits & b != 0 {
                o.frames += 1;
                if let Some(l) = last.as_mut().filter(|l| l.bit == b && l.counting) {
                    l.frames = o.frames;
                }
            }
            let done = o.up.is_some_and(|up| up <= read);
            if done && let Some(l) = last.as_mut().filter(|l| l.bit == b) {
                l.counting = false;
            }
            !done
        });
        self.taken = read;
        self.prev = bits;
        // Changes older than the assist's reach behind this frame are done.
        let keep = read - self.opts.assist_ms - 1.0;
        while self.changes.len() > 2 && self.changes[1].0 <= keep {
            self.changes.remove(0);
        }
        bits
    }
}

/// A page's controls: its gamepads and keys in, each game frame's buttons
/// out.
#[derive(Debug, Clone, Default)]
pub struct Controls {
    pub layout: Layout,
    pads: Pads,
    timeline: Timeline,
    /// The pads handed in since the last [`Controls::set`].
    polled: Vec<RawPad>,
}

impl Controls {
    #[must_use]
    pub fn new(layout: Layout, opts: Options) -> Self {
        Self {
            layout,
            timeline: Timeline::new(opts),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn options(&self) -> Options {
        self.timeline.opts
    }

    /// Takes new settings; the changes kept and the levers stay.
    pub fn set_options(&mut self, opts: Options) {
        self.timeline.opts = opts.clamped();
    }

    /// A connected gamepad as it reads now. Call it for each one, in the
    /// browser's order, then [`Controls::set`].
    pub fn pad(&mut self, pad: RawPad) {
        self.polled.push(pad);
    }

    /// The controls at time `t`: the pads handed in since the last call
    /// with `keys`, the keyboard's buttons in the layout's bits
    /// (controller 1 in the low byte, 2 in the next). The buttons they
    /// give, as kept for the frames to take.
    pub fn set(&mut self, keys: u32, t: f64) -> u32 {
        let pads = std::mem::take(&mut self.polled);
        let travel = self.timeline.opts.stick_travel;
        let bits = keys | self.pads.bits(self.layout, &pads, travel);
        self.timeline.set(bits, t);
        bits
    }

    /// The buttons the game frame at time `t` reads, at `now`.
    pub fn take(&mut self, t: f64, now: f64) -> u32 {
        self.timeline.take(t, now)
    }

    /// The newest press let go: the ms it was held and the frames that
    /// read it.
    #[must_use]
    pub fn last_press(&self) -> Option<(f64, u32)> {
        self.timeline.last_press()
    }
}

/// The exports to JavaScript.
#[cfg(target_arch = "wasm32")]
pub mod abi;

#[cfg(test)]
mod tests {
    use super::*;

    const NAN: f32 = f32::NAN;

    fn pad(buttons: u32, axes: [f32; 4]) -> RawPad {
        RawPad {
            index: 0,
            buttons,
            axes,
        }
    }

    /// A lever is pulled on the first of its travel out and let go on the
    /// first of its travel home, not at a halfway point.
    #[test]
    fn a_lever_is_let_go_on_the_first_of_its_way_home() {
        let mut s = StickPull::new(6.0);
        assert!(!s.read(0.05));
        assert!(!s.read(0.10), "inside the rest zone");
        assert!(s.read(0.20), "out past the rest zone and its travel");
        assert!(s.read(1.0));
        assert!(s.read(0.95), "less than its travel back");
        assert!(!s.read(0.93), "its travel back from the furthest");
        // On its way home it isn't pulled again.
        assert!(!s.read(0.5));
        assert!(!s.read(0.2));
        // A new pull from where it stopped, without going back to rest.
        assert!(s.read(0.3));
    }

    /// The arcade layout: coin, start, right, left, down, up and the
    /// button, from the buttons, the d-pad and the left stick; the second
    /// stick, pulled any way, is the button.
    #[test]
    fn an_arcade_pad_gives_the_cabinets_buttons() {
        let mut p = Pads::default();
        let bits = |p: &mut Pads, r: RawPad| p.bits(Layout::Arcade, &[r], 6.0);
        assert_eq!(bits(&mut p, pad(1 << 8, [0.0; 4])), 1, "Select is the coin");
        assert_eq!(bits(&mut p, pad(1 << 9, [0.0; 4])), 2, "Start starts");
        assert_eq!(bits(&mut p, pad(1 << 15, [0.0; 4])), 4);
        assert_eq!(bits(&mut p, pad(0, [-0.6, 0.0, 0.0, 0.0])), 8);
        assert_eq!(bits(&mut p, pad(0, [0.0, 0.6, 0.0, 0.0])), 16);
        assert_eq!(bits(&mut p, pad(1 << 12, [0.0; 4])), 32);
        assert_eq!(bits(&mut p, pad(1 << 2, [0.0; 4])), 64, "any face button");
        // The second stick up, then left: the button either way, and let
        // go as it starts home.
        assert_eq!(bits(&mut p, pad(0, [0.0, 0.0, 0.0, -0.9])), 64);
        assert_eq!(bits(&mut p, pad(0, [0.0, 0.0, 0.0, -0.8])), 0);
        assert_eq!(bits(&mut p, pad(0, [0.0, 0.0, 0.0, 0.0])), 0);
        assert_eq!(bits(&mut p, pad(0, [0.0, 0.0, -0.9, 0.0])), 64);
        // The left stick at half way is no direction.
        assert_eq!(bits(&mut p, pad(0, [0.5, -0.5, 0.0, 0.0])), 0);
    }

    /// An axis that has never been near the middle (a trigger at -1) is no
    /// lever, and a pad with no second stick has none.
    #[test]
    fn an_axis_never_at_rest_is_no_lever() {
        let mut p = Pads::default();
        assert_eq!(
            p.bits(Layout::Arcade, &[pad(0, [0.0, 0.0, -1.0, NAN])], 6.0),
            0
        );
        assert_eq!(
            p.bits(Layout::Arcade, &[pad(0, [0.0, 0.0, NAN, NAN])], 6.0),
            0
        );
    }

    /// The NES layout, and a second pad in the next byte.
    #[test]
    fn two_nes_pads_are_two_controllers() {
        let mut p = Pads::default();
        let one = pad(1 | 1 << 9, [0.9, 0.0, 0.0, 0.0]);
        let two = RawPad {
            index: 1,
            buttons: 1 << 2,
            axes: [0.0, -0.9, 0.9, 0.9],
        };
        let bits = p.bits(Layout::Nes, &[one, two], 6.0);
        assert_eq!(bits & 0xff, 1 | 8 | 128, "A, Start, right");
        assert_eq!(bits >> 8, 2 | 16, "B, up: a second stick is nothing here");
        // The right button is B too, and the top one nothing.
        assert_eq!(p.bits(Layout::Nes, &[pad(1 << 1, [0.0; 4])], 6.0), 2);
        assert_eq!(p.bits(Layout::Nes, &[pad(1 << 3, [0.0; 4])], 6.0), 0);
        // A cabinet reads one pad.
        assert_eq!(p.bits(Layout::Arcade, &[pad(0, [0.0; 4]), two], 6.0), 0);
    }

    /// A late browser frame that steps three game frames gives each its
    /// own buttons, and a press and release between two frames is read
    /// once.
    #[test]
    fn each_frame_reads_the_buttons_at_its_own_time() {
        let mut t = Timeline::new(Options {
            lead_ms: 0.0,
            ..Options::default()
        });
        // Down at 10 ms, up at 40: frames at 0, 33 and 66, all taken late.
        t.set(64, 10.0);
        t.set(0, 40.0);
        assert_eq!(t.take(0.0, 100.0), 0);
        assert_eq!(t.take(33.0, 100.0), 64);
        assert_eq!(t.take(66.0, 100.0), 0);
        // A tap wholly between two frames.
        t.set(64, 70.0);
        t.set(0, 75.0);
        assert_eq!(t.take(99.0, 100.0), 64, "read once");
        assert_eq!(t.take(132.0, 140.0), 0);
    }

    /// The lead reads up to that far past the frame's time, never past
    /// now; the assist holds a press on after it is let go.
    #[test]
    fn the_lead_and_the_assist() {
        let mut t = Timeline::new(Options {
            lead_ms: 8.0,
            assist_ms: 0.0,
            ..Options::default()
        });
        t.set(1, 5.0);
        assert_eq!(t.take(0.0, 3.0), 0, "now is before the press");
        let mut t = Timeline::new(Options::default());
        t.set(1, 5.0);
        assert_eq!(t.take(0.0, 20.0), 1, "inside the lead");
        let mut t = Timeline::new(Options {
            lead_ms: 0.0,
            assist_ms: 20.0,
            ..Options::default()
        });
        t.set(1, 0.0);
        t.set(0, 10.0);
        assert_eq!(t.at(25.0), 1, "let go 15 ms before");
        assert_eq!(t.at(31.0), 0);
    }

    /// The newest press: the ms it was held and the frames that read it,
    /// counting a frame that reads it after it was let go.
    #[test]
    fn the_newest_press_is_its_ms_and_its_frames() {
        let mut t = Timeline::new(Options {
            lead_ms: 0.0,
            ..Options::default()
        });
        assert_eq!(t.last_press(), None);
        // Held from 10 to 80 ms: read by the frames at 33 and 66.
        t.set(64, 10.0);
        assert_eq!(t.take(0.0, 5.0), 0);
        assert_eq!(t.take(33.0, 40.0), 64);
        assert_eq!(t.take(66.0, 70.0), 64);
        t.set(0, 80.0);
        assert_eq!(t.last_press(), Some((70.0, 2)));
        assert_eq!(t.take(99.0, 100.0), 0);
        assert_eq!(t.last_press(), Some((70.0, 2)));
        // A tap between two frames, let go before the next reads it: that
        // frame still counts it.
        t.set(1, 105.0);
        t.set(0, 110.0);
        assert_eq!(t.last_press(), Some((5.0, 0)));
        assert_eq!(t.take(132.0, 133.0), 1);
        assert_eq!(t.last_press(), Some((5.0, 1)));
        assert_eq!(t.take(165.0, 166.0), 0);
        assert_eq!(t.last_press(), Some((5.0, 1)));
    }

    /// The whole of it: pads and keys in, frames' buttons out.
    #[test]
    fn controls_join_the_keys_and_the_pads() {
        let mut c = Controls::new(Layout::Arcade, Options::default());
        // The lever is seen at rest first: an axis never at rest is none.
        c.pad(pad(0, [0.0; 4]));
        assert_eq!(c.set(0, -1.0), 0);
        c.pad(pad(0, [0.0, 0.0, 0.0, 0.9]));
        assert_eq!(c.set(4, 0.0), 4 | 64, "the key's right and the lever");
        assert_eq!(c.take(0.0, 1.0), 4 | 64);
        // No pad handed in: the keys alone.
        assert_eq!(c.set(0, 20.0), 0);
        assert_eq!(c.take(33.0, 34.0), 0);
        // Settings out of range are brought inside it.
        c.set_options(Options {
            lead_ms: 500.0,
            assist_ms: f64::NAN,
            stick_travel: 0.0,
        });
        assert_eq!(
            c.options(),
            Options {
                lead_ms: 50.0,
                assist_ms: 0.0,
                stick_travel: 1.0
            }
        );
    }
}
