//! `gp clip feasible`: whether a person could make a corpus clip's presses.
//!
//! A clip proves the model matches the original for its inputs; it says
//! nothing about whether a person can play that way. A script presses for
//! exactly one frame, or eight, with two frames between, and no person
//! does. This measures each clip's own presses (the frames from its start,
//! not the path to it) against a human floor ([`FLOOR`], one table with its
//! source), at the machine's own frame rate (the model's `frame_rate`):
//! the shortest press, the shortest gap between two presses of one
//! control, the most presses one control makes in a second, and how many
//! presses fall under the floor. A clip a person could play has none; one
//! they could not names its first press under it. It reads and labels:
//! no clip is changed.
//!
//! A control is one button or one direction (up, down, left, right each
//! their own), so a lever pushed up and at once down isn't a gap. A press
//! held into the clip's first frame or out past its last is cut by the
//! clip's edge: its length isn't known, so it is not held to the floor's
//! press length, though its gaps and its count are.

use std::path::Path;

use anyhow::{Context as _, Result};
use gp_evidence::InputLog;
use gp_evidence::input_log::{arcade_button, button};
use serde_json::{Value, json};

use crate::Outcome;
use crate::clipset;

/// The human floor a clip's presses are held to.
pub(crate) struct Floor {
    /// The shortest press a person makes, in milliseconds.
    pub press_ms: u64,
    /// The shortest time a person lets go between two presses of one
    /// control, in milliseconds.
    pub gap_ms: u64,
    /// The most presses a person makes on one control in a second.
    pub per_second: u64,
    /// Where the numbers are from.
    pub source: &'static str,
}

/// The floor: every number of it, and where it is from. Change it here.
pub(crate) const FLOOR: Floor = Floor {
    press_ms: 50,
    gap_ms: 50,
    per_second: 8,
    source: "the owner's starting floor of 2026-11-22, docs/formats/scoring-corpus.md, \"Could a person play it\"",
};

/// One press of one control: its first and last frame from the clip's
/// start, and whether the clip's edge cuts it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Press {
    control: String,
    from: u64,
    to: u64,
    cut: bool,
}

/// The first press under the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fail {
    pub frame: u64,
    pub control: String,
    pub why: String,
}

/// A clip's presses measured against the floor. Lengths are in frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Measure {
    pub fps_milli: u64,
    pub presses: usize,
    /// The shortest press the clip's edge doesn't cut.
    pub shortest_press: Option<u64>,
    pub shortest_gap: Option<u64>,
    /// The most presses one control starts within a second.
    pub fastest_per_second: usize,
    /// Presses under the floor by any of its rules.
    pub under_floor: usize,
    pub first: Option<Fail>,
}

impl Measure {
    pub(crate) fn could(&self) -> bool {
        self.first.is_none()
    }

    pub(crate) fn verdict(&self) -> &'static str {
        if self.could() {
            "a person could"
        } else {
            "a person could not"
        }
    }
}

/// `frames` frames in milliseconds at `fps_milli`, rounded.
pub(crate) fn ms(frames: u64, fps_milli: u64) -> u64 {
    (frames * 1_000_000 + fps_milli / 2) / fps_milli.max(1)
}

/// Whether `frames` frames last at least `floor_ms` at `fps_milli`.
fn lasts(frames: u64, floor_ms: u64, fps_milli: u64) -> bool {
    frames * 1_000_000 >= floor_ms * fps_milli
}

/// The log's controls: a name and where its bit is (port, bit).
fn controls(log: &InputLog) -> Vec<(String, usize, u8)> {
    match &log.arcade {
        Some(a) => a
            .columns
            .chars()
            .enumerate()
            .take(8)
            .map(|(k, c)| {
                let name = arcade_button(c).map_or_else(|| format!("column {c}"), str::to_owned);
                (name, 0, 1u8 << k)
            })
            .collect(),
        None => {
            const NES: [(u8, &str); 8] = [
                (button::RIGHT, "right"),
                (button::LEFT, "left"),
                (button::DOWN, "down"),
                (button::UP, "up"),
                (button::START, "start"),
                (button::SELECT, "select"),
                (button::B, "b"),
                (button::A, "a"),
            ];
            let mut out = Vec::new();
            for (port, prefix) in [(0usize, ""), (1, gp_evidence::input_log::P2)] {
                for (bit, name) in NES {
                    out.push((format!("{prefix}{name}"), port, bit));
                }
            }
            out
        }
    }
}

/// Every press in `frames`, control by control, each in frame order.
fn presses(log: &InputLog, frames: &[gp_evidence::InputFrame]) -> Vec<Vec<Press>> {
    let n = frames.len();
    controls(log)
        .into_iter()
        .map(|(name, port, bit)| {
            let mut out = Vec::new();
            let mut i = 0;
            while i < n {
                if frames[i].pads[port] & bit == 0 {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j + 1 < n && frames[j + 1].pads[port] & bit != 0 {
                    j += 1;
                }
                out.push(Press {
                    control: name.clone(),
                    from: i as u64,
                    to: j as u64,
                    cut: i == 0 || j + 1 == n,
                });
                i = j + 1;
            }
            out
        })
        .filter(|p: &Vec<Press>| !p.is_empty())
        .collect()
}

/// A clip's own frames measured against `floor` at `fps_milli`.
pub(crate) fn measure(
    log: &InputLog,
    frames: &[gp_evidence::InputFrame],
    fps_milli: u64,
    floor: &Floor,
) -> Measure {
    let mut m = Measure {
        fps_milli,
        presses: 0,
        shortest_press: None,
        shortest_gap: None,
        fastest_per_second: 0,
        under_floor: 0,
        first: None,
    };
    let mut fails: Vec<Fail> = Vec::new();
    for control in presses(log, frames) {
        for (k, p) in control.iter().enumerate() {
            m.presses += 1;
            let mut why = Vec::new();
            let held = p.to - p.from + 1;
            if !p.cut {
                m.shortest_press = Some(m.shortest_press.map_or(held, |s| s.min(held)));
                if !lasts(held, floor.press_ms, fps_milli) {
                    why.push(format!(
                        "held {held} frame{} ({} ms), under {} ms",
                        if held == 1 { "" } else { "s" },
                        ms(held, fps_milli),
                        floor.press_ms
                    ));
                }
            }
            if k > 0 {
                let gap = p.from - control[k - 1].to - 1;
                m.shortest_gap = Some(m.shortest_gap.map_or(gap, |s| s.min(gap)));
                if !lasts(gap, floor.gap_ms, fps_milli) {
                    why.push(format!(
                        "let go {gap} frame{} ({} ms) before it, under {} ms",
                        if gap == 1 { "" } else { "s" },
                        ms(gap, fps_milli),
                        floor.gap_ms
                    ));
                }
            }
            // The presses of this control that start within the second
            // ending at this one's start.
            let in_second = control[..=k]
                .iter()
                .filter(|q| (p.from - q.from) * 1_000_000 < fps_milli * 1000)
                .count();
            m.fastest_per_second = m.fastest_per_second.max(in_second);
            if in_second as u64 > floor.per_second {
                why.push(format!(
                    "press {in_second} of {} in a second, over {}",
                    p.control, floor.per_second
                ));
            }
            if !why.is_empty() {
                m.under_floor += 1;
                fails.push(Fail {
                    frame: p.from,
                    control: p.control.clone(),
                    why: why.join("; "),
                });
            }
        }
    }
    m.first = fails
        .into_iter()
        .min_by(|a, b| (a.frame, &a.control).cmp(&(b.frame, &b.control)));
    m
}

/// The game's frame rate, frames a second times 1000, from its model's
/// `frame_rate`: the machine's own, never a guess.
pub(crate) fn fps_milli(game: &Path) -> Result<u64> {
    let file = game.join(gp_model::MODEL_FILE);
    let text = std::fs::read_to_string(&file).with_context(|| {
        format!(
            "reading {} for the game's frame rate (a clip's presses are measured at the machine's own)",
            file.display()
        )
    })?;
    let m =
        gp_model::Model::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    Ok(m.model.frame_rate.milli())
}

/// One corpus clip measured.
pub(crate) struct Clip {
    pub id: String,
    pub source: String,
    /// None for a clip whose presses can't be read (it starts from a
    /// recording, or its presses file is missing), with why.
    pub measure: std::result::Result<Measure, String>,
}

/// The game's corpus clips (every one, or those `ids` name) measured.
pub(crate) fn clips(game: &Path, ids: &[String]) -> Result<(u64, Vec<Clip>)> {
    let fps = fps_milli(game)?;
    let set = clipset::read_game(game)?;
    for id in ids {
        if !set.clips.iter().any(|r| &r.id == id) {
            anyhow::bail!("{} has no corpus clip {id}", game.display());
        }
    }
    let out = set
        .clips
        .iter()
        .filter(|r| ids.is_empty() || ids.contains(&r.id))
        .map(|r| {
            let measure = match clipset::recipe_log(game, r) {
                Ok(Some((log, start))) => Ok(measure(&log, &log.frames[start..], fps, &FLOOR)),
                Ok(None) => Err("it starts from a recording or its presses file is missing".into()),
                Err(e) => Err(format!("{e:#}")),
            };
            Clip {
                id: r.id.clone(),
                source: r.source.clone(),
                measure,
            }
        })
        .collect();
    Ok((fps, out))
}

/// How many clips a person could play, of those measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Count {
    pub could: usize,
    pub measured: usize,
    pub clips: usize,
}

pub(crate) fn count(clips: &[Clip]) -> Count {
    let measured: Vec<&Measure> = clips
        .iter()
        .filter_map(|c| c.measure.as_ref().ok())
        .collect();
    Count {
        could: measured.iter().filter(|m| m.could()).count(),
        measured: measured.len(),
        clips: clips.len(),
    }
}

/// The floor in words, at `fps_milli`.
pub(crate) fn floor_words(fps_milli: u64) -> String {
    format!(
        "a press of at least {} ms, at least {} ms let go between two presses of one control, at most {} presses a second on one control; frames at {}.{:03} a second, the model's frame_rate ({})",
        FLOOR.press_ms,
        FLOOR.gap_ms,
        FLOOR.per_second,
        fps_milli / 1000,
        fps_milli % 1000,
        FLOOR.source
    )
}

fn frames_ms(frames: Option<u64>, fps_milli: u64) -> String {
    match frames {
        Some(f) => format!(
            "{f} frame{} ({} ms)",
            if f == 1 { "" } else { "s" },
            ms(f, fps_milli)
        ),
        None => "none".into(),
    }
}

fn frames_json(frames: Option<u64>, fps_milli: u64) -> Value {
    frames.map_or(
        Value::Null,
        |f| json!({"frames": f, "ms": ms(f, fps_milli)}),
    )
}

pub(crate) fn clip_json(c: &Clip) -> Value {
    match &c.measure {
        Ok(m) => json!({
            "id": c.id,
            "source": c.source,
            "verdict": m.verdict(),
            "could": m.could(),
            "presses": m.presses,
            "shortest_press": frames_json(m.shortest_press, m.fps_milli),
            "shortest_gap": frames_json(m.shortest_gap, m.fps_milli),
            "fastest_per_second": m.fastest_per_second,
            "under_floor": m.under_floor,
            "first": m.first.as_ref().map(|f| json!({"frame": f.frame, "control": f.control, "why": f.why})),
        }),
        Err(e) => json!({"id": c.id, "source": c.source, "verdict": null, "error": e}),
    }
}

pub(crate) fn clip_line(c: &Clip) -> String {
    match &c.measure {
        Ok(m) => {
            let mut s = format!(
                "{} ({}): {}: {} presses, shortest {}, shortest gap {}, at most {} a second on one control, {} under the floor",
                c.id,
                c.source,
                m.verdict(),
                m.presses,
                frames_ms(m.shortest_press, m.fps_milli),
                frames_ms(m.shortest_gap, m.fps_milli),
                m.fastest_per_second,
                share(m.under_floor, m.presses),
            );
            if let Some(f) = &m.first {
                s.push_str(&format!(
                    "; first at frame {}: {} {}",
                    f.frame, f.control, f.why
                ));
            }
            s
        }
        Err(e) => format!("{} ({}): not measured: {e}", c.id, c.source),
    }
}

fn share(n: usize, of: usize) -> String {
    if of == 0 {
        return "0".into();
    }
    format!("{n} of {of} ({}%)", n * 100 / of)
}

/// The count in words: `N of M clips a person could play`.
pub(crate) fn count_words(c: Count) -> String {
    let mut s = format!(
        "{} of {} clips are presses a person could make",
        c.could, c.measured
    );
    if c.measured < c.clips {
        s.push_str(&format!(" ({} not measured)", c.clips - c.measured));
    }
    s
}

/// `gp clip feasible GAME [CLIP...]`.
pub(crate) fn run(game: &Path, ids: &[String]) -> Result<Outcome> {
    let (fps, clips) = clips(game, ids)?;
    let n = count(&clips);
    let mut text = vec![format!("The floor: {}.", floor_words(fps))];
    text.extend(clips.iter().map(clip_line));
    let mut by_source: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
    for c in &clips {
        if let Ok(m) = &c.measure {
            let e = by_source.entry(c.source.as_str()).or_default();
            e.1 += 1;
            if m.could() {
                e.0 += 1;
            }
        }
    }
    let sources: Vec<String> = by_source
        .iter()
        .map(|(s, (could, of))| format!("{could} of the {of} marked {s}"))
        .collect();
    text.push(format!("{}: {}.", count_words(n), sources.join(", ")));
    Ok(Outcome::ok(
        json!({
            "floor": {"press_ms": FLOOR.press_ms, "gap_ms": FLOOR.gap_ms,
                "per_second": FLOOR.per_second, "source": FLOOR.source},
            "fps_milli": fps,
            "could": n.could,
            "measured": n.measured,
            "clips": clips.iter().map(clip_json).collect::<Vec<_>>(),
            "by_source": by_source.iter().map(|(s, (could, of))| (s.to_string(), json!({"could": could, "measured": of}))).collect::<serde_json::Map<_, _>>(),
        }),
        text.join("\n"),
    ))
}

/// The game's count for the scoreboard, kept while its corpus plan, its
/// inputs folder and its model don't change: the board is asked for every
/// few seconds and the count reads every clip's presses.
pub(crate) fn kept_count(game: &Path) -> Option<Count> {
    let stamp = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .map(|m| (m.modified().ok(), m.len()))
    };
    let key = (
        stamp(&game.join(crate::corpus_score::FILE))?,
        stamp(&game.join("inputs")),
        stamp(&game.join(gp_model::MODEL_FILE)),
    );
    {
        let kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((k, c)) = kept.get(game)
            && *k == key
        {
            return *c;
        }
    }
    let c = clips(game, &[]).ok().map(|(_, c)| count(&c));
    KEPT.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(game.to_owned(), (key, c));
    c
}

type Stamp = Option<(Option<std::time::SystemTime>, u64)>;
type Key = ((Option<std::time::SystemTime>, u64), Stamp, Stamp);
/// Each game folder's count, with the key it was made under.
type Kept = std::collections::HashMap<std::path::PathBuf, (Key, Option<Count>)>;

static KEPT: std::sync::LazyLock<std::sync::Mutex<Kept>> =
    std::sync::LazyLock::new(Default::default);

#[cfg(test)]
mod tests {
    use super::*;

    fn arcade(spans: &[&str], frames: usize) -> InputLog {
        let layout = gp_evidence::input_log::ArcadeLayout {
            system: "rooftop".into(),
            columns: "CSRLDUT".into(),
        };
        gp_evidence::input_log::generate_arcade(frames, &layout, spans).unwrap()
    }

    #[test]
    fn a_scripts_presses_are_under_the_floor_at_30_a_second() {
        // A script's presses on a 30 Hz board (docs/formats/scoring-corpus.md,
        // "Could a person play it"): a tap held 8 frames, 2 let go, and a
        // direction held 1 frame. At 30 a second a frame is 33 ms.
        let log = arcade(&["tap@10-17", "tap@20-27", "up@40-40"], 100);
        let m = measure(&log, &log.frames, 30_000, &FLOOR);
        assert_eq!(m.presses, 3);
        assert_eq!(m.shortest_press, Some(1));
        assert_eq!(m.shortest_gap, Some(2));
        assert_eq!(m.under_floor, 1);
        assert!(!m.could());
        let f = m.first.unwrap();
        assert_eq!((f.frame, f.control.as_str()), (40, "up"));
        assert!(
            f.why.contains("held 1 frame (33 ms), under 50 ms"),
            "{}",
            f.why
        );
    }

    #[test]
    fn a_persons_presses_pass_and_the_clips_edge_cuts_no_press() {
        // Held from before the clip and past its end: lengths unknown.
        let log = arcade(&["left@0-0", "tap@10-14", "tap@17-21", "right@99-99"], 100);
        let m = measure(&log, &log.frames, 30_000, &FLOOR);
        assert_eq!(m.shortest_press, Some(5));
        assert_eq!(m.shortest_gap, Some(2));
        assert!(m.could(), "{:?}", m.first);
        // One frame let go is 33 ms.
        let log = arcade(&["tap@10-14", "tap@16-21"], 100);
        let m = measure(&log, &log.frames, 30_000, &FLOOR);
        assert!(m.first.unwrap().why.contains("let go 1 frame (33 ms)"));
    }

    #[test]
    fn nine_presses_in_a_second_is_over_the_floor() {
        // Taps held 2 frames, 2 let go: one every 4 frames, 7.5 a second
        // at 30 a second, so 8 start in the second from the first...
        let spans: Vec<String> = (0..8)
            .map(|k| format!("tap@{}-{}", 10 + 4 * k, 11 + 4 * k))
            .collect();
        let refs: Vec<&str> = spans.iter().map(String::as_str).collect();
        let log = arcade(&refs, 100);
        let m = measure(&log, &log.frames, 30_000, &FLOOR);
        assert_eq!(m.fastest_per_second, 8);
        assert!(m.could(), "{:?}", m.first);
        // ...and a ninth within that second is one too many. At this
        // floor's press and gap no ninth fits in 30 frames, so a floor
        // with shorter ones shows it: presses of 1 frame, 1 let go.
        let quick = Floor {
            press_ms: 30,
            gap_ms: 30,
            ..FLOOR
        };
        let spans: Vec<String> = (0..9).map(|k| format!("tap@{0}-{0}", 10 + 2 * k)).collect();
        let refs: Vec<&str> = spans.iter().map(String::as_str).collect();
        let log = arcade(&refs, 100);
        let m = measure(&log, &log.frames, 30_000, &quick);
        assert_eq!(m.fastest_per_second, 9);
        assert_eq!(m.under_floor, 1);
        let f = m.first.unwrap();
        assert_eq!(f.frame, 26);
        assert!(
            f.why.contains("press 9 of tap in a second, over 8"),
            "{}",
            f.why
        );
    }

    #[test]
    fn an_nes_frame_is_its_own_length() {
        // At 60.099 a second, 4 frames are 67 ms and 3 are 49.9, under 50.
        let log = gp_evidence::input_log::generate(100, &["a@10-13", "p2:b@20-22"]).unwrap();
        let m = measure(&log, &log.frames, 60_099, &FLOOR);
        let f = m.first.unwrap();
        assert_eq!((f.frame, f.control.as_str()), (20, "p2:b"));
        assert_eq!(m.under_floor, 1);
    }
}
