//! `gp rule lane`: the rule for a player who moves between lanes, as
//! Tapper's bartender moves between the bars. Fitted from the clips and
//! checked against the original frame by frame, as `gp rule jump` is.
//!
//! The player's bytes are the lane (0 the lane whose number is lowest:
//! Tapper's bottom bar, its y counting up the screen), x and y. They come from
//! the flags or from the state mapping: `entities.player[0].x` and `.y`,
//! and `entities.player[0].lane`; each lane's tap, where it has one, from
//! `entities.tap[K].x` (or `--tap`, one per lane, lane 0 first).
//!
//! What is fitted, each from the frames that show it:
//!
//! | Number | From |
//! |---|---|
//! | lanes, row_top, row_height | the y the player rests at in each lane |
//! | bob | how far y strays from the row while the player walks |
//! | switch_frames | frames from up or down to the lane changing |
//! | switch_busy | frames after a move's press in which another press does nothing, counted from the press |
//! | wrap | whether a move past the first or last lane comes round to the other end |
//! | up_step | which way up changes the lane's number (+1 or -1) |
//! | press_to_tap | whether a press of the button puts the player at the tap on the same frame |
//! | land | where x is when the lane changes: so far left of the new lane's tap (stand), or one x for each lane |
//! | walk_step, walk_lag | x's change a frame with left or right held, and the frames before it starts |
//! | send_step | the step back from the tap's place when the button is let go after a pour |
//! | pour_frames, pour_least | the fewest frames held before a release that sent, and the most before one that didn't, plus one: the pour is between them |
//!
//! The check plays the rule from the original's state at each stretch of
//! play (the player resting on a lane's row) with the clip's buttons, and
//! compares lane, x and y (y to within the bob) on every frame. At a
//! difference it says which, takes the original's state and goes on, so
//! one miss doesn't cost the rest of the clip.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use serde_json::{Value, json};

use super::{
    Trace, WINDOW, corpus_traces, fitted_from_words, held_from, load_traces, status, whole_log,
};
use crate::Outcome;

#[derive(Args, Debug)]
pub(crate) struct LaneArgs {
    /// The game's package folder (games/tapper).
    pub game: PathBuf,
    /// The mechanic sessions whose clips to use (default: every one).
    #[arg(long = "session")]
    pub sessions: Vec<String>,
    /// What the rule is fitted on: the game's corpus ("corpus", the
    /// default), your mechanic session clips ("yours") or the clips
    /// harvested from a recorded run ("harvest"). The others are the
    /// held-out test.
    #[arg(long, default_value = "corpus", value_parser = ["corpus", "yours", "harvest"])]
    pub fit_on: String,
    /// An input log from power-on to test on as well, whole; held out,
    /// never fitted on.
    #[arg(long = "test-log")]
    pub test_logs: Vec<PathBuf>,
    /// The ROM; by default the one in $GP_ROM_DIR the package names.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// Where to write the result (default the lane session's refine.json).
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// The player's lane byte (default: the mapping's
    /// entities.player[0].lane).
    #[arg(long, value_parser = parse_addr)]
    pub lane: Option<u16>,
    /// The player's x byte (default: the mapping's entities.player[0].x).
    #[arg(long, value_parser = parse_addr)]
    pub x: Option<u16>,
    /// The player's y byte (default: the mapping's entities.player[0].y).
    #[arg(long, value_parser = parse_addr)]
    pub y: Option<u16>,
    /// Each lane's tap x byte, lane 0 first (default: the mapping's
    /// entities.tap[K].x). Without them, a lane's landing x is fitted as
    /// one number for each lane.
    #[arg(long = "tap", value_parser = parse_addr)]
    pub taps: Vec<u16>,
}

/// An address as 0xE6F9, $E6F9 or decimal.
fn parse_addr(s: &str) -> Result<u16, String> {
    let t = s.trim();
    let hex = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .or_else(|| t.strip_prefix('$'));
    match hex {
        Some(h) => u16::from_str_radix(h, 16),
        None => t.parse(),
    }
    .map_err(|e| format!("{s}: {e}"))
}

/// The first `ram` address of a mapping field, by its path.
fn mapped(game_dir: &Path, path: &str) -> Result<Option<u16>> {
    let file = game_dir.join(gp_interp::mapping::MAPPING_FILE);
    let Ok(text) = fs::read_to_string(&file) else {
        return Ok(None);
    };
    let m: toml::Table = toml::from_str(&text).with_context(|| file.display().to_string())?;
    Ok(m.get("field")
        .and_then(|f| f.as_array())
        .and_then(|fields| {
            fields.iter().find_map(|f| {
                (f.get("path")?.as_str()? == path)
                    .then(|| f.get("ram")?.as_array()?.first()?.as_integer())
                    .flatten()
            })
        })
        .map(|a| a as u16))
}

// Arcade button bits (gp-arcade's order, the oracle log's columns) and the
// NES pad's: the rule reads the stick and one button.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pad {
    pub right: u8,
    pub left: u8,
    pub down: u8,
    pub up: u8,
    pub button: u8,
}

pub(crate) const ARCADE_PAD: Pad = Pad {
    right: 0x04,
    left: 0x08,
    down: 0x10,
    up: 0x20,
    button: 0x40,
};

pub(crate) const NES_PAD: Pad = Pad {
    right: 0x80,
    left: 0x40,
    down: 0x20,
    up: 0x10,
    button: 0x01,
};

/// The player's bytes, as offsets into a trace's RAM window.
#[derive(Clone, Debug)]
pub(crate) struct Bytes {
    pub lane: usize,
    pub x: usize,
    pub y: usize,
    pub taps: Vec<usize>,
}

/// One frame as the rule sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    pub lane: i32,
    pub x: i32,
    pub y: i32,
}

/// A trace cut to what the rule reads: each frame's state, the taps' x and
/// the buttons held.
#[derive(Clone, Debug)]
pub(crate) struct LaneTrace {
    pub name: String,
    pub start: usize,
    pub seen: Vec<Seen>,
    pub taps: Vec<Vec<i32>>,
    pub pads: Vec<u8>,
}

fn cut(t: &Trace, b: &Bytes) -> LaneTrace {
    LaneTrace {
        name: if t.clip > 0 {
            format!("{} clip {}", t.session, t.clip)
        } else {
            t.session.clone()
        },
        start: t.start,
        seen: t
            .ram
            .iter()
            .map(|r| Seen {
                lane: i32::from(r[b.lane]),
                x: i32::from(r[b.x]),
                y: i32::from(r[b.y]),
            })
            .collect(),
        taps: t
            .ram
            .iter()
            .map(|r| b.taps.iter().map(|&a| i32::from(r[a])).collect())
            .collect(),
        pads: t.pads.clone(),
    }
}

/// The fitted rule.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rule {
    pub lanes: i32,
    pub row_top: i32,
    pub row_height: i32,
    pub bob: i32,
    pub switch_frames: usize,
    pub switch_busy: usize,
    /// A move past the last lane comes round to lane 0 (and back from
    /// lane 0 to the last).
    pub wrap: bool,
    /// The lane a press of up moves to, from the lane's number: 1 or -1
    /// (down moves the other way).
    pub up_step: i32,
    /// Where x is when the lane changes: so far left of the new lane's tap
    /// (a tap on the lane's right end; mirrored on the left), or one x for
    /// each lane.
    pub land: Land,
    pub walk_step: i32,
    /// Frames from a direction's first held frame to x's first change.
    pub walk_lag: usize,
    /// A press of the button puts the player at the lane's tap (where a
    /// move lands) on the same frame.
    pub press_to_tap: bool,
    /// How far from the tap's place x steps back on the frame the button is
    /// let go after a pour; none when no clip shows it.
    pub send_step: Option<i32>,
    /// The fewest frames the button was held before a release that sent;
    /// the rule plays this.
    pub pour_frames: usize,
    /// The most frames the button was held before a release that didn't
    /// send, plus one: the pour's frames are between this and
    /// `pour_frames`.
    pub pour_least: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Land {
    Stand(i32),
    Fixed(BTreeMap<i32, i32>),
}

fn mode<T: Ord + Copy>(v: impl IntoIterator<Item = T>) -> Option<T> {
    let mut n: BTreeMap<T, usize> = BTreeMap::new();
    for x in v {
        *n.entry(x).or_default() += 1;
    }
    n.into_iter()
        .max_by_key(|&(k, c)| (c, std::cmp::Reverse(k)))
        .map(|(k, _)| k)
}

/// The tap's side on a lane: true for one on the lane's right end (the
/// lane's middle is 128 in the game's units).
fn tap_right(tap_x: i32) -> bool {
    tap_x >= 128
}

/// Where the player lands on `lane` by the rule, from the taps' x.
fn land_x(rule: &Rule, lane: i32, taps: &[i32]) -> Option<i32> {
    match &rule.land {
        Land::Stand(d) => {
            let t = *taps.get(usize::try_from(lane).ok()?)?;
            Some(if tap_right(t) { t - d } else { t + d })
        }
        Land::Fixed(m) => m.get(&lane).copied(),
    }
}

/// The stick's direction this frame: -1 up, 1 down, 0 neither.
fn vertical(p: u8, pad: &Pad) -> i32 {
    if p & pad.up != 0 {
        -1
    } else if p & pad.down != 0 {
        1
    } else {
        0
    }
}

fn horizontal(p: u8, pad: &Pad) -> i32 {
    if p & pad.right != 0 {
        1
    } else if p & pad.left != 0 {
        -1
    } else {
        0
    }
}

/// Fits the rule from the traces. Fails in words when the traces don't show
/// a part of it (no move between lanes, no walk).
pub(crate) fn fit(traces: &[LaneTrace], pad: &Pad) -> Result<Rule> {
    // Rows: the commonest y in each lane, over every frame.
    let mut ys: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    for t in traces {
        for s in &t.seen[t.start..] {
            ys.entry(s.lane).or_default().push(s.y);
        }
    }
    let rows: BTreeMap<i32, i32> = ys
        .iter()
        .filter(|(_, v)| v.len() >= 10)
        .filter_map(|(&l, v)| Some((l, mode(v.iter().copied())?)))
        .collect();
    // The lanes: 0 up, each a row at one height more than the one before.
    let mut lanes = 0;
    let mut row_height = 0;
    let row_top = *rows.get(&0).context("no frames in lane 0")?;
    while let Some(&r) = rows.get(&(lanes + 1)) {
        let h = r - rows[&lanes];
        if h <= 0 || (row_height != 0 && h != row_height) {
            break;
        }
        row_height = h;
        lanes += 1;
    }
    lanes += 1;
    if lanes < 2 {
        bail!("the clips show the player in one lane only: no move between lanes to fit");
    }
    let row = |l: i32| row_top + l * row_height;
    let on_row =
        |s: &Seen, bob: i32| (0..lanes).contains(&s.lane) && (s.y - row(s.lane)).abs() <= bob;
    // The bob: how far y strays from its row while the lane holds and y
    // stays near it (a quarter of a row's height at most).
    let mut strays: BTreeMap<i32, usize> = BTreeMap::new();
    for t in traces {
        for (i, w) in t.seen.windows(2).enumerate().skip(t.start) {
            if horizontal(t.pads[i + 1], pad) != 0
                && w[0].lane == w[1].lane
                && on_row(&w[1], row_height / 4)
            {
                *strays.entry((w[1].y - row(w[1].lane)).abs()).or_default() += 1;
            }
        }
    }
    // On frames with left or right held. A stray seen on fewer than 1 in
    // 200 of them is a life lost or a level's end passing through, not the
    // walk.
    let near: usize = strays.values().sum();
    let bob = strays
        .iter()
        .filter(|&(_, &n)| n * 200 >= near)
        .map(|(&d, _)| d)
        .max()
        .unwrap_or(0);

    // Moves between lanes: a press of up or down on the stick (not held the
    // frame before) with the player on a row, and the lane changing later.
    struct Move {
        t: usize,
        change: usize,
        from: i32,
        to: i32,
        dir: i32,
    }
    // The fresh presses of up or down with the player on a row: (trace,
    // frame, direction).
    let mut presses: Vec<(usize, usize, i32)> = Vec::new();
    for (ti, t) in traces.iter().enumerate() {
        for i in t.start.max(1)..t.seen.len() {
            let v = vertical(t.pads[i], pad);
            if v != 0 && vertical(t.pads[i - 1], pad) != v && on_row(&t.seen[i - 1], bob) {
                presses.push((ti, i, v));
            }
        }
    }
    // A move's frames: the commonest gap from a press to the lane's next
    // change, over presses with no other press before the change.
    let next_change = |ti: usize, i: usize| -> Option<usize> {
        let t = &traces[ti];
        let from = t.seen[i - 1].lane;
        (i..t.seen.len().min(i + 30)).find(|&k| t.seen[k].lane != from)
    };
    let gaps = presses.iter().enumerate().filter_map(|(n, &(ti, i, _))| {
        let k = next_change(ti, i)?;
        let alone = presses
            .get(n + 1)
            .is_none_or(|&(tj, j, _)| tj != ti || j > k);
        alone.then_some(k - i)
    });
    let switch_frames = mode(gaps).context("no move between lanes in the clips")?;
    // Each press either moved (the lane changes after the move's frames,
    // onto a row) or didn't; the frames since the last press that moved
    // give the busy time.
    let mut moves: Vec<Move> = Vec::new();
    let mut idle_presses: Vec<usize> = Vec::new();
    let mut honoured_gaps: Vec<usize> = Vec::new();
    let mut last: Option<(usize, usize)> = None;
    for &(ti, i, v) in &presses {
        let t = &traces[ti];
        let k = i + switch_frames;
        let since = last.filter(|&(tl, _)| tl == ti).map(|(_, p)| i - p);
        let moved = k < t.seen.len()
            && t.seen[k].lane != t.seen[k - 1].lane
            && t.seen[k - 1].lane == t.seen[i - 1].lane
            && on_row(&t.seen[k], bob);
        if moved {
            honoured_gaps.extend(since);
            last = Some((ti, i));
            moves.push(Move {
                t: ti,
                change: k,
                from: t.seen[i - 1].lane,
                to: t.seen[k].lane,
                dir: v,
            });
        } else if let Some(g) = since
            && g < 30
        {
            idle_presses.push(g);
        }
    }
    let wrap = moves.iter().any(|m| (m.to - m.from).abs() > 1);
    // Up's lane change, from every move: a move up from lane 2 to 3 is +1,
    // and one down from 0 to 3 (wrapped) is +1 for up too.
    let up_step = mode(moves.iter().filter_map(|m| {
        let d = (m.to - m.from).rem_euclid(lanes);
        let step = if d == 1 {
            1
        } else if d == lanes - 1 {
            -1
        } else {
            return None;
        };
        Some(if m.dir < 0 { step } else { -step })
    }))
    .unwrap_or(-1);
    // Busy: the most frames after a move's press at which a press still
    // did nothing, plus one; never past the fewest frames after which a
    // press moved again.
    let first_free = honoured_gaps.iter().copied().min().unwrap_or(usize::MAX);
    let switch_busy = idle_presses
        .iter()
        .copied()
        .filter(|&g| g < first_free)
        .max()
        .map_or(switch_frames, |g| g + 1)
        .max(switch_frames);

    // Landing: x on the frame the lane changes, against the new lane's tap.
    let mut stands = Vec::new();
    let mut fixed: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    for m in &moves {
        let t = &traces[m.t];
        let s = t.seen[m.change];
        fixed.entry(m.to).or_default().push(s.x);
        if let Some(&tap) = t.taps[m.change].get(usize::try_from(m.to).unwrap_or(usize::MAX)) {
            stands.push(if tap_right(tap) { tap - s.x } else { s.x - tap });
        }
    }
    let land = match mode(stands.iter().copied()) {
        Some(d) if stands.iter().filter(|&&v| v == d).count() * 10 >= stands.len() * 9 => {
            Land::Stand(d)
        }
        _ => Land::Fixed(
            fixed
                .into_iter()
                .filter_map(|(l, v)| Some((l, mode(v)?)))
                .collect(),
        ),
    };

    // The walk: x's change on frames where one direction has been held a
    // while, the lane held and the button not pressed.
    let mut steps = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    for t in traces {
        let n = t.seen.len();
        let mut i = t.start.max(1);
        while i < n {
            let h = horizontal(t.pads[i], pad);
            if h == 0 || horizontal(t.pads[i - 1], pad) == h || t.pads[i] & pad.button != 0 {
                i += 1;
                continue;
            }
            // A walk from frame i: held from here, x moves in its direction.
            let mut k = i;
            while k < n
                && horizontal(t.pads[k], pad) == h
                && t.pads[k] & (pad.up | pad.down | pad.button) == 0
                && t.seen[k].lane == t.seen[i - 1].lane
                && on_row(&t.seen[k], bob)
            {
                k += 1;
            }
            if k - i >= 4 {
                let first = (i..k).find(|&f| t.seen[f].x != t.seen[f - 1].x);
                if let Some(f) = first {
                    starts.push(f - i);
                    for g in f..k {
                        let d = (t.seen[g].x - t.seen[g - 1].x) * h;
                        if d > 0 {
                            steps.push(d);
                        }
                    }
                }
            }
            i = k.max(i + 1);
        }
    }
    let walk_step = mode(steps.iter().copied()).context("no walk along a lane in the clips")?;
    let walk_lag = mode(starts.iter().copied()).unwrap_or(0);

    // The tap: a fresh press of the button, the player off the tap's
    // place, puts him there on the same frame or doesn't; and a release at
    // the tap's place after a hold steps him back from it or doesn't.
    let partial = Rule {
        lanes,
        row_top,
        row_height,
        bob,
        switch_frames,
        switch_busy,
        wrap,
        up_step,
        land: land.clone(),
        walk_step,
        walk_lag,
        press_to_tap: false,
        send_step: None,
        pour_frames: 0,
        pour_least: 0,
    };
    let (mut snapped, mut stayed) = (0usize, 0usize);
    let mut sends = Vec::new();
    let mut sent_holds = Vec::new();
    let mut kept_holds = Vec::new();
    for t in traces {
        for i in t.start.max(1)..t.seen.len() {
            let (s0, s1) = (t.seen[i - 1], t.seen[i]);
            if s0.lane != s1.lane
                || !on_row(&s1, bob)
                || horizontal(t.pads[i], pad) != 0
                || vertical(t.pads[i], pad) != 0
            {
                continue;
            }
            let Some(at) = land_x(&partial, s1.lane, &t.taps[i]) else {
                continue;
            };
            let (down, was) = (t.pads[i] & pad.button != 0, t.pads[i - 1] & pad.button != 0);
            if down && !was && s0.x != at {
                if s1.x == at {
                    snapped += 1;
                } else if s1.x == s0.x {
                    stayed += 1;
                }
            }
            if !down && was && s0.x == at {
                let mut h = 0;
                while i > h + 1 && t.pads[i - 1 - h] & pad.button != 0 {
                    h += 1;
                }
                let tap = usize::try_from(s1.lane)
                    .ok()
                    .and_then(|l| t.taps[i].get(l).copied());
                let away = match tap {
                    Some(tp) if !tap_right(tp) => s1.x - at,
                    _ => at - s1.x,
                };
                if away > 0 {
                    sends.push(away);
                    sent_holds.push(h);
                } else if s1.x == at {
                    kept_holds.push(h);
                }
            }
        }
    }
    let press_to_tap = snapped > stayed;
    let send_step = mode(sends.iter().copied());
    let pour_frames = sent_holds.iter().copied().min().unwrap_or(0);
    let pour_least = kept_holds
        .iter()
        .copied()
        .filter(|&h| h < pour_frames)
        .max()
        .map_or(pour_frames, |h| h + 1);
    Ok(Rule {
        lanes,
        row_top,
        row_height,
        bob,
        switch_frames,
        switch_busy,
        wrap,
        up_step,
        land,
        walk_step,
        walk_lag,
        press_to_tap,
        send_step,
        pour_frames,
        pour_least,
    })
}

/// What the rule plays from a state: the hidden counters a frame of the
/// original doesn't show in the player's bytes.
#[derive(Clone, Debug, Default)]
struct Hidden {
    /// Frames since a move's press, while a move is on.
    moving: Option<(usize, i32)>,
    busy: usize,
    held_h: usize,
    held_button: usize,
}

/// One frame of the rule: the state after a frame with `p` held.
fn step(rule: &Rule, pad: &Pad, s: Seen, h: &mut Hidden, p: u8, prev: u8, taps: &[i32]) -> Seen {
    let mut s = s;
    let row = |l: i32| rule.row_top + l * rule.row_height;
    if let Some((k, dir)) = h.moving {
        let k = k + 1;
        if k >= rule.switch_frames {
            let mut l = s.lane + dir;
            if rule.wrap {
                l = l.rem_euclid(rule.lanes);
            }
            if (0..rule.lanes).contains(&l) {
                s.lane = l;
                s.y = row(l);
                if let Some(x) = land_x(rule, l, taps) {
                    s.x = x;
                }
            }
            h.moving = None;
        } else {
            h.moving = Some((k, dir));
        }
    }
    h.busy = h.busy.saturating_sub(1);
    let v = vertical(p, pad);
    if h.moving.is_none() && h.busy == 0 && v != 0 && vertical(prev, pad) != v {
        h.busy = rule.switch_busy;
        h.moving = Some((0, if v < 0 { rule.up_step } else { -rule.up_step }));
        h.held_h = 0;
        return s;
    }
    if h.moving.is_some() {
        return s;
    }
    let dir = horizontal(p, pad);
    if dir != 0 && p & pad.button == 0 {
        h.held_h += 1;
        if h.held_h > rule.walk_lag {
            s.x += dir * rule.walk_step;
        }
    } else {
        h.held_h = 0;
    }
    let at = land_x(rule, s.lane, taps);
    if p & pad.button != 0 {
        if h.held_button == 0
            && prev & pad.button == 0
            && rule.press_to_tap
            && let Some(at) = at
        {
            s.x = at;
        }
        h.held_button += 1;
    } else {
        if h.held_button >= rule.pour_frames.max(1)
            && let Some(d) = rule.send_step
        {
            let tap = usize::try_from(s.lane)
                .ok()
                .and_then(|l| taps.get(l).copied());
            let back = match tap {
                Some(t) if !tap_right(t) => d,
                _ => -d,
            };
            s.x = at.unwrap_or(s.x) + back;
        }
        h.held_button = 0;
    }
    s
}

/// A clip's check: frames compared, matched, and each difference.
#[derive(Debug, Default)]
pub(crate) struct Checked {
    pub frames: usize,
    pub matched: usize,
    pub first: Option<Value>,
    pub misses: BTreeMap<&'static str, usize>,
}

/// Plays the rule against a trace frame by frame from its start.
pub(crate) fn check(rule: &Rule, pad: &Pad, t: &LaneTrace) -> Checked {
    let mut out = Checked::default();
    let row = |l: i32| rule.row_top + l * rule.row_height;
    let on_row =
        |s: &Seen| (0..rule.lanes).contains(&s.lane) && (s.y - row(s.lane)).abs() <= rule.bob;
    let mut sim: Option<(Seen, Hidden)> = None;
    for i in t.start.max(1)..t.seen.len() {
        let real = t.seen[i];
        let Some((s, mut h)) = sim.take() else {
            // Out of play: the rule starts again from the next frame on a row.
            if on_row(&real) {
                sim = Some((real, Hidden::default()));
            }
            continue;
        };
        let next = step(rule, pad, s, &mut h, t.pads[i], t.pads[i - 1], &t.taps[i]);
        if !on_row(&real) && h.moving.is_none() {
            // The original left play (a life lost, a level's end).
            continue;
        }
        out.frames += 1;
        let what = if next.lane != real.lane {
            Some("lane")
        } else if next.x != real.x {
            Some("x")
        } else if (next.y - real.y).abs() > rule.bob {
            Some("y")
        } else {
            None
        };
        match what {
            None => {
                out.matched += 1;
                sim = Some((Seen { y: real.y, ..next }, h));
            }
            Some(w) => {
                *out.misses.entry(w).or_default() += 1;
                if out.first.is_none() {
                    out.first = Some(json!({
                        "frame": i - t.start,
                        "what": w,
                        "rule": {"lane": next.lane, "x": next.x, "y": next.y},
                        "original": {"lane": real.lane, "x": real.x, "y": real.y},
                        "buttons": format!("{:#04x}", t.pads[i]),
                    }));
                }
                // Take the original's state and go on; the hidden counters
                // the rule kept stay.
                sim = Some((real, h));
            }
        }
    }
    out
}

pub(crate) fn run(a: &LaneArgs, paths: &Paths) -> Result<Outcome> {
    let game_dir = fs::canonicalize(&a.game).with_context(|| a.game.display().to_string())?;
    let game = game_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_owned();
    let home = paths.home.join("mechanics").join(&game);
    let out_dir = home.join("lane");
    status(&out_dir, "reading the player's bytes");
    let rom = crate::clip_score::rom_of(paths, &game_dir, a.rom.as_deref())?;
    let pick = |flag: Option<u16>, path: &str| -> Result<u16> {
        match flag {
            Some(v) => Ok(v),
            None => mapped(&game_dir, path)?.with_context(|| {
                format!("the state mapping reads no {path} from RAM; give it as a flag")
            }),
        }
    };
    let yb = pick(a.y, "entities.player[0].y")?;
    let xb = pick(a.x, "entities.player[0].x")?;
    let lb = pick(a.lane, "entities.player[0].lane")?;
    let mut taps = a.taps.clone();
    if taps.is_empty() {
        for k in 0.. {
            match mapped(&game_dir, &format!("entities.tap[{k}].x"))? {
                Some(t) => taps.push(t),
                None => break,
            }
        }
    }
    let base = usize::from(yb) & !(WINDOW - 1);
    let off = |b: u16| -> Result<usize> {
        let b = usize::from(b);
        if b < base || b >= base + WINDOW {
            bail!("${b:04X} isn't in the 2 KB page of the player's y (${yb:04X})");
        }
        Ok(b - base)
    };
    let bytes = Bytes {
        lane: off(lb)?,
        x: off(xb)?,
        y: off(yb)?,
        taps: taps.iter().map(|&t| off(t)).collect::<Result<_>>()?,
    };
    let pad = match crate::original::Platform::of_file(&rom) {
        crate::original::Platform::Arcade => ARCADE_PAD,
        crate::original::Platform::Nes => NES_PAD,
    };

    let harvested = |s: &str| s.starts_with("harvest-");
    let only = &a.sessions;
    let fit_on = a.fit_on.as_str();
    let fitted_here = |s: &str| match fit_on {
        "harvest" => harvested(s),
        "yours" => !harvested(s) && (only.is_empty() || only.iter().any(|o| o == s)),
        _ => false,
    };
    status(&out_dir, "replaying the clips");
    let mut traces = load_traces(&rom, base, &home, &|s| fitted_here(s))?;
    if fit_on == "corpus" {
        traces.extend(corpus_traces(&rom, base, &game_dir)?);
    }
    if traces.is_empty() {
        bail!("no clips to fit on ({fit_on})");
    }
    let fit_traces: Vec<LaneTrace> = traces.iter().map(|t| cut(t, &bytes)).collect();
    if let Ok(p) = std::env::var("GP_LANE_DUMP") {
        let v: Vec<Value> = fit_traces
            .iter()
            .map(|t| {
                json!({"name": t.name, "start": t.start,
                    "seen": t.seen.iter().map(|s| [s.lane, s.x, s.y]).collect::<Vec<_>>(),
                    "taps": t.taps, "pads": t.pads})
            })
            .collect();
        fs::write(p, serde_json::to_string(&v)?)?;
    }
    status(&out_dir, "fitting the lanes, the moves and the walk");
    let rule = fit(&fit_traces, &pad)?;

    status(&out_dir, "running the rule against every clip");
    let per = |ts: &[LaneTrace]| -> (Vec<Value>, usize, usize) {
        let mut clips = Vec::new();
        let (mut m, mut n) = (0, 0);
        for t in ts {
            let c = check(&rule, &pad, t);
            m += c.matched;
            n += c.frames;
            clips.push(json!({
                "clip": t.name,
                "frames": c.frames,
                "matched": c.matched,
                "first_difference": c.first,
                "differences": c.misses,
            }));
        }
        (clips, m, n)
    };
    let (clips, matched, total) = per(&fit_traces);

    let mut test = load_traces(&rom, base, &home, &|s| {
        !fitted_here(s) && (only.is_empty() || only.iter().any(|o| o == s) || harvested(s))
    })?;
    if fit_on != "corpus" {
        test.extend(corpus_traces(&rom, base, &game_dir)?);
    }
    for log in &a.test_logs {
        test.push(whole_log(&rom, base, log)?);
    }
    let test: Vec<LaneTrace> = test.iter().map(|t| cut(t, &bytes)).collect();
    let (test_clips, test_matched, test_total) = per(&test);

    let n = fit_traces.len();
    let c = |name: &str, value: Value, unit: &str, from: &str| json!({"name": name, "value": value, "unit": unit, "source": "fitted", "from": from, "clips": n});
    let mut constants = vec![
        c(
            "lanes",
            json!(rule.lanes),
            "lanes",
            "the lane byte's values with a row of their own",
        ),
        c(
            "row_top",
            json!(rule.row_top),
            "px",
            "the commonest y in lane 0",
        ),
        c(
            "row_height",
            json!(rule.row_height),
            "px",
            "the step from one lane's row to the next",
        ),
        c(
            "bob",
            json!(rule.bob),
            "px",
            "the most y strays from its row in a lane",
        ),
        c(
            "switch_frames",
            json!(rule.switch_frames),
            "frames",
            "from up or down pressed to the lane changing",
        ),
        c(
            "switch_busy",
            json!(rule.switch_busy),
            "frames",
            "after a move's press, the frames in which another press does nothing",
        ),
        c(
            "wrap",
            json!(rule.wrap),
            "",
            "a move past the first or last lane comes round to the other end",
        ),
        c(
            "up_step",
            json!(rule.up_step),
            "lanes",
            "the lane number's change on a move with up (down changes it the other way)",
        ),
        c(
            "press_to_tap",
            json!(rule.press_to_tap),
            "",
            "a press of the button puts the player at the lane's tap on the same frame",
        ),
        c(
            "walk_step",
            json!(rule.walk_step),
            "px a frame",
            "x's change with left or right held",
        ),
        c(
            "walk_lag",
            json!(rule.walk_lag),
            "frames",
            "from the first frame held to x's first change",
        ),
    ];
    match &rule.land {
        Land::Stand(d) => constants.push(c(
            "stand",
            json!(d),
            "px",
            "x on the frame the lane changes, this far from the new lane's tap toward the lane's middle",
        )),
        Land::Fixed(m) => constants.push(c(
            "land",
            json!(m.iter().map(|(l, x)| (l.to_string(), json!(x))).collect::<serde_json::Map<_, _>>()),
            "px",
            "x on the frame the lane changes, for each lane (no taps given)",
        )),
    }
    if let Some(s) = rule.send_step {
        constants.push(c(
            "send_step",
            json!(s),
            "px",
            "the step back from the tap when the button is let go",
        ));
        constants.push(c(
            "pour_frames",
            json!(rule.pour_frames),
            "frames",
            "the fewest frames the button was held before a step back",
        ));
        constants.push(c(
            "pour_least",
            json!(rule.pour_least),
            "frames",
            "the most frames the button was held before a release with no step back, plus one: the pour takes between this and pour_frames",
        ));
    }
    let hex = |b: usize| format!("${:04X}", base + b);
    let fitted_from: Vec<String> = fit_traces.iter().map(|t| t.name.clone()).collect();
    let result = json!({
        "mechanic": "lane",
        "game": game,
        "bytes": {
            "lane": hex(bytes.lane),
            "x": hex(bytes.x),
            "y": hex(bytes.y),
            "taps": bytes.taps.iter().map(|&b| hex(b)).collect::<Vec<_>>(),
        },
        "rule": constants,
        "matched": matched,
        "total": total,
        "clips": clips,
        "held_out": {"from": held_from(fit_on), "clips": test.len(), "matched": test_matched, "total": test_total, "per_clip": test_clips},
        "from_clips": fitted_from,
        "fitted_on": fitted_from_words(fit_on),
    });
    let out_path = a.out.clone().unwrap_or_else(|| out_dir.join("refine.json"));
    if let Some(d) = out_path.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(&out_path, serde_json::to_string(&result)?)?;
    let fitted = fitted_from_words(fit_on);
    status(
        &out_dir,
        &format!("done: matches {matched} of {total} frames in play in {fitted}"),
    );
    let mut text = format!(
        "lane rule for {game}: matches {matched} of {total} frames in play in the clips it was fitted on ({fitted})\n\
         held out ({}, {} clips): {test_matched} of {test_total} frames\n",
        held_from(fit_on),
        test.len(),
    );
    for c in result["rule"].as_array().into_iter().flatten() {
        text.push_str(&format!(
            "  {}: {} {}\n",
            c["name"].as_str().unwrap_or(""),
            c["value"],
            c["unit"].as_str().unwrap_or("")
        ));
    }
    for c in result["clips"].as_array().into_iter().flatten() {
        if let Some(f) = c["first_difference"].as_object() {
            text.push_str(&format!(
                "  {}: {} of {} frames; first difference at frame {}: {} (rule {}, original {})\n",
                c["clip"].as_str().unwrap_or(""),
                c["matched"],
                c["frames"],
                f["frame"],
                f["what"].as_str().unwrap_or(""),
                f["rule"],
                f["original"],
            ));
        }
    }
    Ok(Outcome::ok(result, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up game: 4 lanes with rows at 48 + 48 k, taps at 225 - 16 k
    /// on the right, a move landing 16 left of the tap 6 frames after the
    /// press and busy for 8, a walk of 2 a frame from the first frame held,
    /// and a send that steps 6 back after the button is held 5 frames.
    fn truth() -> Rule {
        Rule {
            lanes: 4,
            row_top: 48,
            row_height: 48,
            bob: 0,
            switch_frames: 6,
            switch_busy: 8,
            wrap: true,
            up_step: 1,
            land: Land::Stand(16),
            walk_step: 2,
            walk_lag: 0,
            press_to_tap: true,
            send_step: Some(6),
            pour_frames: 5,
            pour_least: 5,
        }
    }

    fn play(rule: &Rule, pads: &[u8]) -> LaneTrace {
        let taps: Vec<i32> = (0..4).map(|k| 225 - 16 * k).collect();
        let mut s = Seen {
            lane: 2,
            x: 177,
            y: 144,
        };
        let mut h = Hidden::default();
        let mut seen = vec![s];
        for i in 1..pads.len() {
            s = step(rule, &ARCADE_PAD, s, &mut h, pads[i], pads[i - 1], &taps);
            seen.push(s);
        }
        LaneTrace {
            name: "made up".into(),
            start: 0,
            seen,
            taps: vec![taps; pads.len()],
            pads: pads.to_vec(),
        }
    }

    fn script() -> Vec<u8> {
        let p = ARCADE_PAD;
        let mut v = vec![0u8; 40];
        let hold = |b: u8, n: usize, v: &mut Vec<u8>| {
            v.extend(std::iter::repeat_n(b, n));
            v.extend(std::iter::repeat_n(0, 12));
        };
        for _ in 0..4 {
            hold(p.up, 3, &mut v);
            hold(p.up, 1, &mut v);
            hold(p.left, 10, &mut v);
            hold(p.right, 4, &mut v);
            hold(p.button, 5, &mut v);
            hold(p.button, 4, &mut v);
            hold(p.down, 2, &mut v);
            hold(p.button, 9, &mut v);
        }
        // Up again 3 frames after a move's press: busy, nothing happens.
        v.extend([p.up, 0, 0, p.up, 0]);
        v.extend(std::iter::repeat_n(0, 20));
        v.extend([p.up, 0, 0, 0, 0, 0, 0, p.up, 0]);
        v.extend(std::iter::repeat_n(0, 20));
        for _ in 0..5 {
            hold(p.up, 1, &mut v);
        }
        v.extend([p.down, 0, 0, 0, 0, 0, 0, 0, p.down]);
        v.extend(std::iter::repeat_n(0, 20));
        v
    }

    #[test]
    fn the_rule_is_fitted_back_from_its_own_play() {
        let t = truth();
        let trace = play(&t, &script());
        let fitted = fit(std::slice::from_ref(&trace), &ARCADE_PAD).unwrap();
        assert_eq!(fitted, t);
        let c = check(&fitted, &ARCADE_PAD, &trace);
        assert_eq!(c.matched, c.frames);
        assert!(c.first.is_none());
    }

    #[test]
    fn a_difference_is_found_on_its_frame() {
        let t = truth();
        let mut trace = play(&t, &script());
        // The original walks 3 a frame once, at frame 60.
        let at = (1..trace.seen.len())
            .find(|&i| trace.seen[i].x != trace.seen[i - 1].x && i > 50)
            .unwrap();
        for s in &mut trace.seen[at..] {
            s.x += 1;
        }
        let c = check(&t, &ARCADE_PAD, &trace);
        assert!(c.matched < c.frames);
        assert_eq!(c.first.unwrap()["frame"], json!(at));
        assert_eq!(c.misses.keys().copied().collect::<Vec<_>>(), ["x"]);
    }

    #[test]
    fn addresses_read_in_hex_or_decimal() {
        assert_eq!(parse_addr("0xE6F9"), Ok(0xE6F9));
        assert_eq!(parse_addr("$e2e9"), Ok(0xE2E9));
        assert_eq!(parse_addr("10"), Ok(10));
    }
}
