//! The text file `scripts/oracle/capture.lua` writes from inside Mesen.
//!
//! ```text
//! gp-mesen-capture 1
//! mesen-region Ntsc
//! frame <n> polled <0|1>
//! s <key> <value>            a value from Mesen's emu.getState()
//! m <name> <hex>             a memory region
//! e <kind> <scanline> <dot> <addr> <value>
//! end <frames>
//! ```
//!
//! Frames start at 0 (power-on) and count up by one. A frame's `e` lines are
//! the events that happened while it ran, as Mesen timed them.

use std::collections::BTreeMap;

pub const HEADER: &str = "gp-mesen-capture 1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvent {
    pub kind: String,
    /// Mesen's scanline, -1 for the pre-render line.
    pub scanline: i32,
    /// Mesen's PPU cycle within the scanline when its callback ran.
    pub dot: u32,
    pub addr: u16,
    pub value: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frame {
    pub frame: u64,
    /// Whether the game read $4016 or $4017 while this frame ran.
    pub polled: bool,
    pub values: BTreeMap<String, String>,
    pub memories: BTreeMap<String, Vec<u8>>,
    pub events: Vec<RawEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// The region Mesen ran, as it names it (`Ntsc`, `Pal`).
    pub region: String,
    pub frames: Vec<Frame>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CaptureError {
    #[error("not a Mesen capture: the first line isn't {HEADER:?}")]
    Header,
    #[error("capture line {line}: {message}")]
    Line { line: usize, message: String },
    #[error("the capture ends at frame {got}, before its last frame {want}; Mesen stopped early")]
    Truncated { got: u64, want: u64 },
}

impl Capture {
    /// Parses a capture that should hold frames 0 to `frames`.
    pub fn parse(text: &str, frames: u64) -> Result<Self, CaptureError> {
        let mut lines = text.lines().enumerate();
        if lines.next().map(|(_, l)| l) != Some(HEADER) {
            return Err(CaptureError::Header);
        }
        let mut region = String::new();
        let mut out: Vec<Frame> = Vec::new();
        let mut ended = false;
        for (i, line) in lines {
            let n = i + 1;
            let bad = |message: String| CaptureError::Line { line: n, message };
            let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
            match tag {
                "mesen-region" => region = rest.to_owned(),
                "frame" => {
                    let parts: Vec<&str> = rest.split(' ').collect();
                    let [number, "polled", polled] = parts[..] else {
                        return Err(bad(format!("a frame line is {line:?}")));
                    };
                    let frame: u64 = number
                        .parse()
                        .map_err(|_| bad(format!("frame number {number:?}")))?;
                    if frame != out.len() as u64 {
                        return Err(bad(format!(
                            "frame {frame} follows frame {}",
                            out.len() as i64 - 1
                        )));
                    }
                    out.push(Frame {
                        frame,
                        polled: polled == "1",
                        ..Frame::default()
                    });
                }
                "s" | "m" | "e" => {
                    let current = out
                        .last_mut()
                        .ok_or_else(|| bad("a value before the first frame".into()))?;
                    match tag {
                        "s" => {
                            let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
                            current.values.insert(key.to_owned(), value.to_owned());
                        }
                        "m" => {
                            let (name, hex) = rest.split_once(' ').unwrap_or((rest, ""));
                            let bytes = hex_bytes(hex)
                                .ok_or_else(|| bad(format!("memory {name} isn't hex")))?;
                            current.memories.insert(name.to_owned(), bytes);
                        }
                        _ => current.events.push(
                            parse_event(rest)
                                .ok_or_else(|| bad(format!("an event line is {line:?}")))?,
                        ),
                    }
                }
                "end" => ended = true,
                "" => {}
                other => return Err(bad(format!("unknown record {other:?}"))),
            }
        }
        let last = out.len() as u64;
        if !ended || last != frames + 1 {
            return Err(CaptureError::Truncated {
                got: last.saturating_sub(1),
                want: frames,
            });
        }
        Ok(Self {
            region,
            frames: out,
        })
    }
}

fn hex_bytes(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

fn parse_event(rest: &str) -> Option<RawEvent> {
    let mut it = rest.split(' ');
    let kind = it.next()?.to_owned();
    let scanline = it.next()?.parse().ok()?;
    let dot = it.next()?.parse().ok()?;
    let addr = it.next()?.parse().ok()?;
    let value = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some(RawEvent {
        kind,
        scanline,
        dot,
        addr,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "gp-mesen-capture 1\nmesen-region Ntsc\nframe 0 polled 0\ns cpu.a 0\nm ram 00ff\nframe 1 polled 1\ns cpu.a 144\nm ram 0102\ne controller-read 241 30 16406 65\nend 1\n";

    #[test]
    fn parses_frames_values_memories_and_events() {
        let c = Capture::parse(SAMPLE, 1).unwrap();
        assert_eq!(c.region, "Ntsc");
        assert_eq!(c.frames.len(), 2);
        assert_eq!(c.frames[1].values["cpu.a"], "144");
        assert_eq!(c.frames[0].memories["ram"], vec![0, 255]);
        assert!(c.frames[1].polled);
        assert_eq!(
            c.frames[1].events,
            vec![RawEvent {
                kind: "controller-read".into(),
                scanline: 241,
                dot: 30,
                addr: 0x4016,
                value: 65
            }]
        );
    }

    #[test]
    fn refuses_a_short_or_foreign_capture() {
        assert_eq!(
            Capture::parse(SAMPLE, 2),
            Err(CaptureError::Truncated { got: 1, want: 2 })
        );
        let unfinished = SAMPLE.replace("end 1\n", "");
        assert!(matches!(
            Capture::parse(&unfinished, 1),
            Err(CaptureError::Truncated { .. })
        ));
        assert_eq!(Capture::parse("hello", 1), Err(CaptureError::Header));
        let odd = SAMPLE.replace("m ram 00ff", "m ram 0f0");
        assert!(matches!(
            Capture::parse(&odd, 1),
            Err(CaptureError::Line { line: 5, .. })
        ));
    }
}
