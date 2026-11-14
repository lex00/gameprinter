//! The input log: controller state for every frame of a recording.
//!
//! Text, one line per frame, so logs diff cleanly. Format version 0:
//!
//! ```text
//! gp-input-log 0
//! controllers 2
//! P R..U...A ........
//! - R..U...A ........
//! ```
//!
//! Each frame line starts with `P` if the game read the controllers during the
//! frame or `-` for a lag frame, then one field per controller. A field lists
//! the buttons in the order Right, Left, Down, Up, sTart, Select, B, A, with
//! the letter `RLDUTSBA` when held and `.` when not (the FCEUX convention).
//! Lag frames keep whatever was held, since importers and replays need it.
//!
//! Format version 1 adds one thing: a `!` after the `P` or `-` (`P!`, `-!`)
//! presses the console's reset button at the start of that frame, before
//! the frame runs. Logs without a reset are still written as version 0, so
//! their bytes and hashes don't change.
//!
//! # An arcade log
//!
//! An arcade game's log is spelt as the MAME oracle's input log
//! (docs/oracle-mame.md), so a capture's log and a clip's are one format
//! and the oracle replays a clip as it is:
//!
//! ```text
//! gp-arcade-input 0
//! system rbtapper
//! columns CSRLDUT
//! 600 .......
//! 6 C......
//! ```
//!
//! `system` names the MAME set and `columns` the buttons, one letter each
//! from bit 0. Each line is a count of frames and the buttons held through
//! them, the column's letter when held and `.` when not; `#` starts a
//! comment. The bits go in controller 1's byte ([`InputFrame::pads`]), as
//! the runtime protocol's `buttons` has them. The log keeps no lag marks
//! or resets: every frame reads as polled. [`InputLog::arcade`] says which
//! spelling a log has, and [`InputLog::to_text`] writes it back the same
//! way.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// The newest format version: 1, which adds resets. A log without a reset
/// is written as [`FORMAT_VERSION_NO_RESET`].
pub const FORMAT_VERSION: u32 = 1;
/// The version a log without any reset is written as.
pub const FORMAT_VERSION_NO_RESET: u32 = 0;
const ORDER: &[u8; 8] = b"RLDUTSBA";

/// Button bits within one controller byte, in the order the NES reads them
/// from its shift register (A first).
pub mod button {
    pub const A: u8 = 1 << 0;
    pub const B: u8 = 1 << 1;
    pub const SELECT: u8 = 1 << 2;
    pub const START: u8 = 1 << 3;
    pub const UP: u8 = 1 << 4;
    pub const DOWN: u8 = 1 << 5;
    pub const LEFT: u8 = 1 << 6;
    pub const RIGHT: u8 = 1 << 7;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InputFrame {
    /// Whether the game read the controllers during this frame.
    pub polled: bool,
    /// One byte per controller port, bits as in [`button`].
    pub pads: [u8; 4],
    /// Whether the reset button is pressed at the start of this frame,
    /// before it runs (a soft reset). Format version 1 only.
    #[serde(default)]
    pub reset: bool,
}

impl InputFrame {
    /// A polled frame with `pad` held on controller 1 and nothing on the
    /// others.
    pub fn held(pad: u8) -> Self {
        Self {
            polled: true,
            pads: [pad, 0, 0, 0],
            reset: false,
        }
    }

    /// The runtime protocol's `input-frame.buttons`: port 1 in the low byte.
    pub fn buttons_word(&self) -> u32 {
        u32::from_le_bytes(self.pads)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputLog {
    pub controllers: u8,
    pub frames: Vec<InputFrame>,
    /// An arcade log's set and button columns; none for the NES.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arcade: Option<ArcadeLayout>,
}

/// What an arcade log names: the MAME set and its button columns, a
/// letter each from bit 0 (Tapper's `CSRLDUT`: coin, start, right, left,
/// down, up, tap).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArcadeLayout {
    pub system: String,
    pub columns: String,
}

/// An arcade log's first line.
pub const ARCADE_HEADER: &str = "gp-arcade-input 0";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("input log line {line}: {message}")]
pub struct InputLogError {
    pub line: usize,
    pub message: String,
}

fn err(line: usize, message: impl Into<String>) -> InputLogError {
    InputLogError {
        line,
        message: message.into(),
    }
}

impl InputLog {
    pub fn new(controllers: u8) -> Self {
        assert!((1..=4).contains(&controllers), "1 to 4 controllers");
        Self {
            controllers,
            frames: Vec::new(),
            arcade: None,
        }
    }

    /// An empty arcade log for the set `system` with these button columns.
    pub fn new_arcade(system: &str, columns: &str) -> Self {
        Self {
            controllers: 1,
            frames: Vec::new(),
            arcade: Some(ArcadeLayout {
                system: system.to_owned(),
                columns: columns.to_owned(),
            }),
        }
    }

    pub fn parse(text: &str) -> Result<Self, InputLogError> {
        if text.lines().next().map(str::trim_end) == Some(ARCADE_HEADER) {
            return Self::parse_arcade(text);
        }
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l));

        let (n, header) = lines.next().ok_or_else(|| err(1, "empty input log"))?;
        let version = match header.strip_prefix("gp-input-log ") {
            Some("0") => 0,
            Some("1") => 1,
            Some(v) => return Err(err(n, format!("unsupported format version {v}"))),
            None => return Err(err(n, "expected `gp-input-log 0` or `gp-input-log 1`")),
        };

        let (n, line) = lines
            .next()
            .ok_or_else(|| err(2, "missing `controllers` line"))?;
        let controllers: u8 = line
            .strip_prefix("controllers ")
            .and_then(|c| c.parse().ok())
            .filter(|c| (1..=4).contains(c))
            .ok_or_else(|| err(n, "expected `controllers N` with N from 1 to 4"))?;

        let mut frames = Vec::new();
        for (n, line) in lines {
            let mut fields = line.split(' ');
            let (polled, reset) = match fields.next() {
                Some("P") => (true, false),
                Some("-") => (false, false),
                Some("P!") if version >= 1 => (true, true),
                Some("-!") if version >= 1 => (false, true),
                Some("P!" | "-!") => {
                    return Err(err(n, "a reset (`!`) needs format version 1"));
                }
                _ => return Err(err(n, "frame line must start with `P` or `-`")),
            };
            let mut pads = [0u8; 4];
            for (port, pad) in pads.iter_mut().enumerate().take(controllers as usize) {
                let field = fields
                    .next()
                    .ok_or_else(|| err(n, format!("missing controller {}", port + 1)))?;
                *pad = parse_pad(field).ok_or_else(|| {
                    err(
                        n,
                        format!(
                            "controller {} field {field:?} is not 8 of `RLDUTSBA` or `.`",
                            port + 1
                        ),
                    )
                })?;
            }
            if fields.next().is_some() {
                return Err(err(n, "more controller fields than declared"));
            }
            frames.push(InputFrame {
                polled,
                pads,
                reset,
            });
        }
        Ok(Self {
            controllers,
            frames,
            arcade: None,
        })
    }

    /// Reads an arcade log (the module docs' "An arcade log").
    fn parse_arcade(text: &str) -> Result<Self, InputLogError> {
        let mut system = None;
        let mut columns: Option<String> = None;
        let mut frames = Vec::new();
        for (i, raw) in text.lines().enumerate().skip(1) {
            let n = i + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(v) = line.strip_prefix("system ") {
                system = Some(v.trim().to_owned());
                continue;
            }
            if let Some(v) = line.strip_prefix("columns ") {
                let v = v.trim();
                if v.is_empty() || v.chars().count() > 8 || v.contains('.') {
                    return Err(err(n, "`columns` is 1 to 8 letters, one a button"));
                }
                columns = Some(v.to_owned());
                continue;
            }
            let cols = columns
                .as_deref()
                .ok_or_else(|| err(n, "frames before `columns`"))?;
            let mut parts = line.split_whitespace();
            let count: usize = parts
                .next()
                .and_then(|c| c.parse().ok())
                .ok_or_else(|| err(n, "expected `<frames> <buttons>`"))?;
            let held = parts.next().ok_or_else(|| err(n, "no buttons"))?;
            if parts.next().is_some() || held.chars().count() != cols.chars().count() {
                return Err(err(
                    n,
                    format!("the buttons are one field of {cols}'s letters or `.`"),
                ));
            }
            let mut bits = 0u8;
            for (k, (b, c)) in held.chars().zip(cols.chars()).enumerate() {
                if b == c {
                    bits |= 1 << k;
                } else if b != '.' {
                    return Err(err(
                        n,
                        format!("column {} is `{b}`, not `.` or `{c}`", k + 1),
                    ));
                }
            }
            frames.extend(std::iter::repeat_n(InputFrame::held(bits), count));
        }
        Ok(Self {
            controllers: 1,
            frames,
            arcade: Some(ArcadeLayout {
                system: system.ok_or_else(|| err(1, "no `system` line"))?,
                columns: columns.ok_or_else(|| err(1, "no `columns` line"))?,
            }),
        })
    }

    /// An arcade log's text: a line for each run of frames holding the same
    /// buttons.
    fn arcade_text(&self, a: &ArcadeLayout) -> String {
        let mut out = format!(
            "{ARCADE_HEADER}\nsystem {}\ncolumns {}\n",
            a.system, a.columns
        );
        let held = |bits: u8| -> String {
            a.columns
                .chars()
                .enumerate()
                .map(|(k, c)| if bits & (1 << k) != 0 { c } else { '.' })
                .collect()
        };
        let mut i = 0;
        while i < self.frames.len() {
            let bits = self.frames[i].pads[0];
            let mut j = i;
            while j < self.frames.len() && self.frames[j].pads[0] == bits {
                j += 1;
            }
            let _ = writeln!(out, "{} {}", j - i, held(bits));
            i = j;
        }
        out
    }

    pub fn to_text(&self) -> String {
        if let Some(a) = &self.arcade {
            return self.arcade_text(a);
        }
        let version = if self.has_resets() {
            FORMAT_VERSION
        } else {
            FORMAT_VERSION_NO_RESET
        };
        let mut out = format!("gp-input-log {version}\ncontrollers {}\n", self.controllers);
        for frame in &self.frames {
            out.push(if frame.polled { 'P' } else { '-' });
            if frame.reset {
                out.push('!');
            }
            for pad in &frame.pads[..self.controllers as usize] {
                out.push(' ');
                for (i, letter) in ORDER.iter().enumerate() {
                    let bit = 1u8 << (7 - i);
                    out.push(if pad & bit != 0 { *letter as char } else { '.' });
                }
            }
            out.push('\n');
        }
        out
    }

    /// Whether any frame presses reset, which needs format version 1.
    pub fn has_resets(&self) -> bool {
        self.frames.iter().any(|f| f.reset)
    }

    pub fn lag_frames(&self) -> usize {
        self.frames.iter().filter(|f| !f.polled).count()
    }

    /// A one-line summary, for logs and run manifests.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        let _ = write!(
            s,
            "{} frames, {} lag, {} controllers",
            self.frames.len(),
            self.lag_frames(),
            self.controllers
        );
        s
    }
}

/// The prefix of a controller 2 span ([`generate`], [`spans_of`]).
pub const P2: &str = "p2:";

/// The first two controllers of `frames` as button spans, the inverse of
/// [`generate`]: one span for each run of frames holding the same buttons,
/// none for frames holding nothing, controller 1's first and then
/// controller 2's with the `p2:` prefix. `None` when a frame presses reset
/// or a third or fourth controller, which spans can't say.
pub fn spans_of(frames: &[InputFrame]) -> Option<Vec<String>> {
    const NAMES: [(u8, &str); 8] = [
        (button::RIGHT, "right"),
        (button::LEFT, "left"),
        (button::DOWN, "down"),
        (button::UP, "up"),
        (button::START, "start"),
        (button::SELECT, "select"),
        (button::B, "b"),
        (button::A, "a"),
    ];
    if frames
        .iter()
        .any(|f| f.reset || f.pads[2..].iter().any(|&p| p != 0))
    {
        return None;
    }
    let mut out = Vec::new();
    for (port, prefix) in [(0, ""), (1, P2)] {
        let mut i = 0;
        while i < frames.len() {
            let bits = frames[i].pads[port];
            let mut j = i;
            while j + 1 < frames.len() && frames[j + 1].pads[port] == bits {
                j += 1;
            }
            if bits != 0 {
                let names: Vec<&str> = NAMES
                    .iter()
                    .filter(|(b, _)| bits & b != 0)
                    .map(|(_, n)| *n)
                    .collect();
                out.push(format!("{prefix}{}@{i}-{j}", names.join("+")));
            }
            i = j + 1;
        }
    }
    Some(out)
}

/// Builds an input log from button spans, for scripted recordings:
/// `start@120-124` holds Start on frames 120 to 124, `right+b@130-` holds
/// Right and B from frame 130 to the end. A span with the `p2:` prefix is
/// controller 2's (`p2:start@60-60`), and the log then has two controllers.
/// Frames count from 0; every frame is marked polled, since the runtime
/// records what the game actually read.
pub fn generate(frames: usize, spans: &[&str]) -> Result<InputLog, InputLogError> {
    let mut log = InputLog::new(1);
    log.frames = vec![
        InputFrame {
            polled: true,
            pads: [0; 4],
            reset: false,
        };
        frames
    ];
    for (i, spec) in spans.iter().enumerate() {
        let bad = |message: String| err(i + 1, format!("span {spec:?}: {message}"));
        let (port, body) = match spec.strip_prefix(P2) {
            Some(rest) => {
                log.controllers = 2;
                (1, rest)
            }
            None => (0, *spec),
        };
        let (buttons, range) = body
            .split_once('@')
            .ok_or_else(|| bad("expected BUTTONS@FROM-TO, e.g. a+right@10-20".into()))?;
        let mut bits = 0u8;
        for name in buttons.split('+') {
            bits |= match name.to_ascii_lowercase().as_str() {
                "a" => button::A,
                "b" => button::B,
                "select" => button::SELECT,
                "start" => button::START,
                "up" => button::UP,
                "down" => button::DOWN,
                "left" => button::LEFT,
                "right" => button::RIGHT,
                other => return Err(bad(format!("unknown button {other:?}"))),
            };
        }
        let (from, to) = span_range(range, frames).map_err(bad)?;
        for f in &mut log.frames[from..=to] {
            f.pads[port] |= bits;
        }
    }
    Ok(log)
}

/// An arcade button's name in a span, by its column letter in the log
/// (`columns`): coin, start1, right, left, down, up, tap. `None` for a
/// letter no arcade set here uses.
pub fn arcade_button(letter: char) -> Option<&'static str> {
    Some(match letter {
        'C' => "coin",
        'S' => "start1",
        'R' => "right",
        'L' => "left",
        'D' => "down",
        'U' => "up",
        'T' => "tap",
        _ => return None,
    })
}

/// [`generate`] for an arcade set: the spans name the layout's buttons
/// ([`arcade_button`]: `coin@10-15`, `up+tap@100-`), and each sets its
/// column's bit. The log is an arcade log of that layout.
pub fn generate_arcade(
    frames: usize,
    layout: &ArcadeLayout,
    spans: &[&str],
) -> Result<InputLog, InputLogError> {
    let mut log = InputLog::new_arcade(&layout.system, &layout.columns);
    log.frames = vec![
        InputFrame {
            polled: true,
            pads: [0; 4],
            reset: false,
        };
        frames
    ];
    for (i, spec) in spans.iter().enumerate() {
        let bad = |message: String| err(i + 1, format!("span {spec:?}: {message}"));
        let (buttons, range) = spec
            .split_once('@')
            .ok_or_else(|| bad("expected BUTTONS@FROM-TO, e.g. up+tap@10-20".into()))?;
        let mut bits = 0u8;
        for name in buttons.split('+') {
            let name = name.to_ascii_lowercase();
            let k = layout
                .columns
                .chars()
                .position(|c| arcade_button(c) == Some(name.as_str()))
                .ok_or_else(|| {
                    let known: Vec<&str> =
                        layout.columns.chars().filter_map(arcade_button).collect();
                    bad(format!(
                        "unknown button {name:?}; {}'s are {}",
                        layout.system,
                        known.join(", ")
                    ))
                })?;
            bits |= 1 << k;
        }
        let (from, to) = span_range(range, frames).map_err(bad)?;
        for f in &mut log.frames[from..=to] {
            f.pads[0] |= bits;
        }
    }
    Ok(log)
}

/// An arcade log's frames as spans, the inverse of [`generate_arcade`]:
/// one span for each run of frames holding the same buttons, none for
/// frames holding nothing. `None` when a frame holds a column with no
/// button name.
pub fn arcade_spans_of(frames: &[InputFrame], layout: &ArcadeLayout) -> Option<Vec<String>> {
    let names: Vec<Option<&str>> = layout.columns.chars().map(arcade_button).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < frames.len() {
        let bits = frames[i].pads[0];
        let mut j = i;
        while j + 1 < frames.len() && frames[j + 1].pads[0] == bits {
            j += 1;
        }
        if bits != 0 {
            let mut held = Vec::new();
            for k in 0..8 {
                if bits & (1 << k) != 0 {
                    held.push((*names.get(k)?)?);
                }
            }
            out.push(format!("{}@{i}-{j}", held.join("+")));
        }
        i = j + 1;
    }
    Some(out)
}

/// A span's `FROM-TO` or `FROM-` within `frames` frames.
fn span_range(range: &str, frames: usize) -> Result<(usize, usize), String> {
    let (from, to) = range
        .split_once('-')
        .ok_or_else(|| "expected a frame range FROM-TO or FROM-".to_string())?;
    let from: usize = from.parse().map_err(|_| format!("bad frame {from:?}"))?;
    let to: usize = if to.is_empty() {
        frames.saturating_sub(1)
    } else {
        to.parse().map_err(|_| format!("bad frame {to:?}"))?
    };
    if from > to || to >= frames {
        return Err(format!(
            "range {from}-{to} isn't within 0-{}",
            frames.saturating_sub(1)
        ));
    }
    Ok((from, to))
}

fn parse_pad(field: &str) -> Option<u8> {
    let bytes = field.as_bytes();
    if bytes.len() != 8 {
        return None;
    }
    let mut pad = 0u8;
    for (i, (&b, &letter)) in bytes.iter().zip(ORDER).enumerate() {
        match b {
            b'.' => {}
            _ if b == letter => pad |= 1 << (7 - i),
            _ => return None,
        }
    }
    Some(pad)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An arcade log reads as the oracle writes it and is written back in
    /// one spelling: runs merged, comments gone.
    #[test]
    fn an_arcade_log_reads_and_writes_the_oracle_spelling() {
        let text = "gp-arcade-input 0\n# a comment\nsystem rbtapper\ncolumns CSRLDUT\n3 .......\n2 C......\n1 C......\n4 .S..D.T\n";
        let log = InputLog::parse(text).unwrap();
        let a = log.arcade.as_ref().unwrap();
        assert_eq!(
            (a.system.as_str(), a.columns.as_str()),
            ("rbtapper", "CSRLDUT")
        );
        assert_eq!(log.frames.len(), 10);
        assert_eq!(log.frames[3].pads[0], 1);
        // start (bit 1), down (bit 4), tap (bit 6)
        assert_eq!(log.frames[9].pads[0], 0b101_0010);
        assert_eq!(log.frames[9].buttons_word(), 0x52);
        let out = log.to_text();
        assert_eq!(
            out,
            "gp-arcade-input 0\nsystem rbtapper\ncolumns CSRLDUT\n3 .......\n3 C......\n4 .S..D.T\n"
        );
        assert_eq!(InputLog::parse(&out).unwrap(), log);
        assert!(InputLog::parse("gp-arcade-input 0\nsystem x\ncolumns AB\n1 BA\n").is_err());
        assert!(InputLog::parse("gp-arcade-input 0\ncolumns AB\n1 A.\n").is_err());
        let mut fresh = InputLog::new_arcade("rbtapper", "CSRLDUT");
        fresh.frames = log.frames.clone();
        assert_eq!(fresh.to_text(), out);
    }

    #[test]
    fn round_trip() {
        let mut log = InputLog::new(2);
        log.frames.push(InputFrame::held(button::RIGHT | button::A));
        log.frames.push(InputFrame {
            polled: false,
            pads: [button::RIGHT | button::A, button::START, 0, 0],
            reset: false,
        });
        let text = log.to_text();
        assert_eq!(
            text,
            "gp-input-log 0\ncontrollers 2\nP R......A ........\n- R......A ....T...\n"
        );
        assert_eq!(InputLog::parse(&text).unwrap(), log);
        assert_eq!(log.lag_frames(), 1);
    }

    #[test]
    fn a_reset_writes_version_1() {
        let mut log = InputLog::new(1);
        log.frames.push(InputFrame::default());
        log.frames.push(InputFrame {
            polled: true,
            pads: [button::START, 0, 0, 0],
            reset: true,
        });
        let text = log.to_text();
        assert_eq!(
            text,
            "gp-input-log 1\ncontrollers 1\n- ........\nP! ....T...\n"
        );
        assert_eq!(InputLog::parse(&text).unwrap(), log);
        // Version 0 has no resets, and a version 1 log may have none.
        let v0 = "gp-input-log 0\ncontrollers 1\nP! ........\n";
        assert!(
            InputLog::parse(v0)
                .unwrap_err()
                .message
                .contains("version 1")
        );
        let none =
            InputLog::parse("gp-input-log 1\ncontrollers 1\n-! ........\nP ........\n").unwrap();
        assert!(none.frames[0].reset && !none.frames[0].polled && !none.frames[1].reset);
    }

    #[test]
    fn buttons_word_puts_port_one_low() {
        let f = InputFrame {
            polled: true,
            pads: [button::A, button::B, 0, 0],
            reset: false,
        };
        assert_eq!(f.buttons_word(), 0x0201);
    }

    #[test]
    fn letters_must_be_in_their_own_column() {
        let text = "gp-input-log 0\ncontrollers 1\nP A.......\n";
        assert_eq!(InputLog::parse(text).unwrap_err().line, 3);
    }

    #[test]
    fn rejects_wrong_version_and_counts() {
        assert_eq!(
            InputLog::parse("gp-input-log 9\ncontrollers 1\n")
                .unwrap_err()
                .line,
            1
        );
        assert_eq!(
            InputLog::parse("gp-input-log 0\ncontrollers 5\n")
                .unwrap_err()
                .line,
            2
        );
        let extra = "gp-input-log 0\ncontrollers 1\nP ........ ........\n";
        assert_eq!(InputLog::parse(extra).unwrap_err().line, 3);
        let missing = "gp-input-log 0\ncontrollers 2\nP ........\n";
        assert_eq!(InputLog::parse(missing).unwrap_err().line, 3);
    }

    #[test]
    fn generates_from_spans() {
        let log = generate(10, &["start@2-3", "right+b@5-"]).unwrap();
        assert_eq!(log.frames.len(), 10);
        assert_eq!(log.frames[1].pads[0], 0);
        assert_eq!(log.frames[2].pads[0], button::START);
        assert_eq!(log.frames[4].pads[0], 0);
        assert_eq!(log.frames[9].pads[0], button::RIGHT | button::B);
        assert!(generate(10, &["jump@1-2"]).is_err());
        assert!(generate(10, &["a@5-20"]).is_err());
        assert!(generate(10, &["a@5"]).is_err());
    }

    #[test]
    fn spans_of_gives_back_the_spans_it_was_made_from() {
        let log = generate(12, &["start@2-3", "right+b@5-8", "a@10-11"]).unwrap();
        let spans = spans_of(&log.frames).unwrap();
        assert_eq!(spans, ["start@2-3", "right+b@5-8", "a@10-11"]);
        let refs: Vec<&str> = spans.iter().map(String::as_str).collect();
        assert_eq!(generate(12, &refs).unwrap().frames, log.frames);
        let mut reset = log.frames.clone();
        reset[0].reset = true;
        assert!(spans_of(&reset).is_none());
    }

    #[test]
    fn a_p2_span_is_controller_twos() {
        let log = generate(8, &["start@1-1", "p2:a+left@2-4"]).unwrap();
        assert_eq!(log.controllers, 2);
        assert_eq!(log.frames[1].pads, [button::START, 0, 0, 0]);
        assert_eq!(log.frames[3].pads, [0, button::A | button::LEFT, 0, 0]);
        assert_eq!(generate(8, &["a@1-1"]).unwrap().controllers, 1);
        let spans = spans_of(&log.frames).unwrap();
        assert_eq!(spans, ["start@1-1", "p2:left+a@2-4"]);
        let refs: Vec<&str> = spans.iter().map(String::as_str).collect();
        assert_eq!(generate(8, &refs).unwrap(), log);
        assert_eq!(InputLog::parse(&log.to_text()).unwrap(), log);
        let mut third = log.frames.clone();
        third[0].pads[2] = button::A;
        assert!(spans_of(&third).is_none());
        assert!(generate(8, &["p3:a@1-1"]).is_err());
    }

    /// Arcade spans name the layout's buttons, make an arcade log, and
    /// come back from it unchanged.
    #[test]
    fn arcade_spans_make_an_arcade_log_and_come_back() {
        let layout = ArcadeLayout {
            system: "tapper".into(),
            columns: "CSRLDUT".into(),
        };
        let spans = ["coin@2-3", "start1@6-6", "up+tap@8-9", "left@10-"];
        let log = generate_arcade(12, &layout, &spans).unwrap();
        assert_eq!(log.arcade.as_ref(), Some(&layout));
        assert_eq!(log.frames[2].pads[0], 0b1);
        assert_eq!(log.frames[6].pads[0], 0b10);
        assert_eq!(log.frames[8].pads[0], 0b110_0000);
        assert_eq!(log.frames[11].pads[0], 0b1000);
        let text = log.to_text();
        assert!(text.starts_with("gp-arcade-input 0\nsystem tapper\ncolumns CSRLDUT\n"));
        let back = arcade_spans_of(&InputLog::parse(&text).unwrap().frames, &layout).unwrap();
        assert_eq!(back, ["coin@2-3", "start1@6-6", "up+tap@8-9", "left@10-11"]);
        let refs: Vec<&str> = back.iter().map(String::as_str).collect();
        assert_eq!(generate_arcade(12, &layout, &refs).unwrap(), log);
        // NES names aren't arcade buttons, and the range is checked.
        assert!(generate_arcade(12, &layout, &["a@1-2"]).is_err());
        assert!(generate_arcade(12, &layout, &["coin@5-20"]).is_err());
        assert!(generate(12, &["coin@1-2"]).is_err());
    }

    #[test]
    fn empty_log_has_no_frames() {
        let log = InputLog::parse("gp-input-log 0\ncontrollers 1\n").unwrap();
        assert!(log.frames.is_empty());
    }
}
