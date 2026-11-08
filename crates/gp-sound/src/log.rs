//! The APU register log (docs/formats/apu-log.md): every write a recording
//! made to the APU's registers, with the frame and the CPU cycle within it.
//! Capture writes it from a recording's event stream or a replay with the
//! event log on; it's observe-only, so nothing about the replay changes.
//! A log is game content: it lives with the recording's derived data under
//! `$GP_HOME/work/sound/`, never in the repo.

use std::fmt::Write as _;

use crate::apu::{Apu, ChannelState, Region, Sample};

/// The format's version.
pub const VERSION: u32 = 1;

/// One register write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Write {
    pub frame: u64,
    /// CPU cycles since the frame started (at the start of vertical
    /// blank, as gp-nes frames end).
    pub cycle: u32,
    /// The register's offset from $4000: 0x00-0x13, 0x15 or 0x17.
    pub reg: u8,
    pub value: u8,
}

/// A log: a recording's APU writes over a range of frames.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApuLog {
    pub region: Region,
    /// The headerless SHA-256 of the ROM, when the log came from one.
    pub source: Option<String>,
    /// The recording's ID.
    pub recording: Option<String>,
    /// The first frame and the frame after the last.
    pub first: u64,
    pub end: u64,
    /// Whether `cycle` is measured (false: the writes' order within each
    /// frame is known but not when they came, and each is at cycle 0).
    pub timed: bool,
    pub writes: Vec<Write>,
    /// DMC sample bytes read from the console's memory, by address.
    pub samples: Vec<Sample>,
}

/// The CPU cycle within a frame of a write the PPU saw at `scanline`,
/// `dot`: frames start at scanline 241, dot 1.
pub fn cycle_in_frame(region: Region, scanline: u16, dot: u16) -> u32 {
    let lines = region.frame_dots() / 341;
    let line = (u64::from(scanline) + lines - 241) % lines;
    let dots = (line * 341 + u64::from(dot)).saturating_sub(1);
    region.dots_to_cycles(dots) as u32
}

impl ApuLog {
    /// Writes the text form.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "gp-apu-log {VERSION}");
        let _ = writeln!(s, "region {}", self.region.name());
        if let Some(src) = &self.source {
            let _ = writeln!(s, "source {src}");
        }
        if let Some(r) = &self.recording {
            let _ = writeln!(s, "recording {r}");
        }
        let _ = writeln!(s, "frames {} {}", self.first, self.end);
        let _ = writeln!(s, "timed {}", self.timed);
        for smp in &self.samples {
            let mut hex = String::with_capacity(smp.bytes.len() * 2);
            for b in &smp.bytes {
                let _ = write!(hex, "{b:02x}");
            }
            let _ = writeln!(s, "sample {:04x} {hex}", smp.addr);
        }
        let _ = writeln!(s, "# frame cycle register value");
        for w in &self.writes {
            let _ = writeln!(s, "{} {} {:02x} {:02x}", w.frame, w.cycle, w.reg, w.value);
        }
        s
    }

    /// Reads the text form.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut log = ApuLog::default();
        let mut lines = text.lines().enumerate();
        match lines.next() {
            Some((_, l)) if l.trim() == format!("gp-apu-log {VERSION}") => {}
            Some((_, l)) => return Err(format!("not an APU log of version {VERSION}: `{l}`")),
            None => return Err("an empty APU log".into()),
        }
        let mut frames = false;
        for (i, line) in lines {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = |what: &str| format!("line {}: {what}: `{line}`", i + 1);
            let mut f = line.split_whitespace();
            let head = f.next().unwrap_or_default();
            match head {
                "region" => {
                    log.region = match f.next() {
                        Some("ntsc") => Region::Ntsc,
                        Some("pal") => Region::Pal,
                        _ => return Err(bad("region is ntsc or pal")),
                    }
                }
                "source" => log.source = f.next().map(str::to_owned),
                "recording" => log.recording = f.next().map(str::to_owned),
                "frames" => {
                    let a = f.next().and_then(|x| x.parse().ok());
                    let b = f.next().and_then(|x| x.parse().ok());
                    match (a, b) {
                        (Some(a), Some(b)) if a <= b => {
                            log.first = a;
                            log.end = b;
                            frames = true;
                        }
                        _ => return Err(bad("frames needs a first and an end frame")),
                    }
                }
                "timed" => log.timed = f.next() == Some("true"),
                "sample" => {
                    let addr = f
                        .next()
                        .and_then(|a| u16::from_str_radix(a, 16).ok())
                        .ok_or_else(|| bad("a sample needs a hex address"))?;
                    let hex = f.next().unwrap_or_default();
                    if hex.len() % 2 != 0 {
                        return Err(bad("sample bytes are pairs of hex digits"));
                    }
                    let bytes = (0..hex.len())
                        .step_by(2)
                        .map(|k| u8::from_str_radix(&hex[k..k + 2], 16))
                        .collect::<Result<Vec<u8>, _>>()
                        .map_err(|_| bad("sample bytes are hex"))?;
                    log.samples.push(Sample { addr, bytes });
                }
                _ => {
                    let frame: u64 = head.parse().map_err(|_| bad("unknown line"))?;
                    let cycle: u32 = f
                        .next()
                        .and_then(|x| x.parse().ok())
                        .ok_or_else(|| bad("a write needs a cycle"))?;
                    let reg = f
                        .next()
                        .and_then(|x| u8::from_str_radix(x, 16).ok())
                        .ok_or_else(|| bad("a write needs a hex register"))?;
                    let value = f
                        .next()
                        .and_then(|x| u8::from_str_radix(x, 16).ok())
                        .ok_or_else(|| bad("a write needs a hex value"))?;
                    if !(reg <= 0x13 || reg == 0x15 || reg == 0x17) {
                        return Err(bad("the register isn't an APU register"));
                    }
                    if let Some(last) = log.writes.last()
                        && (last.frame, last.cycle) > (frame, cycle)
                    {
                        return Err(bad("writes must be in order"));
                    }
                    log.writes.push(Write {
                        frame,
                        cycle,
                        reg,
                        value,
                    });
                }
            }
        }
        if !frames {
            return Err("the log has no `frames` line".into());
        }
        if log
            .writes
            .iter()
            .any(|w| w.frame < log.first || w.frame >= log.end)
        {
            return Err("a write lies outside the log's frames".into());
        }
        Ok(log)
    }

    /// The frames it covers.
    pub fn frames(&self) -> u64 {
        self.end - self.first
    }

    /// The writes as absolute CPU cycles from the log's first frame.
    pub fn timeline(&self) -> Vec<(u64, u8, u8)> {
        self.writes
            .iter()
            .map(|w| {
                (
                    self.region.frame_start(w.frame - self.first) + u64::from(w.cycle),
                    w.reg,
                    w.value,
                )
            })
            .collect()
    }

    /// Every channel's state at the end of each frame, from the first
    /// frame on: what note capture reads.
    pub fn states(&self) -> Vec<[ChannelState; 5]> {
        states_of(self.region, &self.timeline(), self.frames(), &self.samples)
    }

    /// The writes of frames `range` (absolute frame numbers) as a log of
    /// their own.
    pub fn slice(&self, from: u64, to: u64) -> ApuLog {
        let from = from.max(self.first);
        let to = to.min(self.end).max(from);
        // The registers as the frames before left them, written first.
        let mut last: std::collections::BTreeMap<u8, u8> = std::collections::BTreeMap::new();
        for w in self.writes.iter().take_while(|w| w.frame < from) {
            last.insert(w.reg, w.value);
        }
        let mut writes: Vec<Write> = Vec::new();
        // $4015 and $4017 first, then the channels; a channel's fourth
        // register isn't replayed (it would restart a note).
        for (&reg, &value) in last.iter().filter(|(r, _)| **r >= 0x15).chain(
            last.iter()
                .filter(|(r, _)| **r < 0x15 && !matches!(**r, 3 | 7 | 0x0B | 0x0F)),
        ) {
            writes.push(Write {
                frame: from,
                cycle: 0,
                reg,
                value,
            });
        }
        writes.extend(
            self.writes
                .iter()
                .filter(|w| w.frame >= from && w.frame < to)
                .copied(),
        );
        ApuLog {
            first: from,
            end: to,
            writes,
            ..self.clone()
        }
    }
}

/// Channel states at the end of each of `frames` frames, from a timeline
/// of writes at absolute CPU cycles.
pub fn states_of(
    region: Region,
    timeline: &[(u64, u8, u8)],
    frames: u64,
    samples: &[Sample],
) -> Vec<[ChannelState; 5]> {
    let mut apu = Apu::new(region).with_samples(samples.to_vec());
    let mut out = Vec::with_capacity(frames as usize);
    let mut at = 0u64;
    let mut w = timeline.iter().peekable();
    for f in 0..frames {
        let end = region.frame_start(f + 1);
        while let Some(&&(c, reg, v)) = w.peek() {
            if c >= end {
                break;
            }
            apu.run((c.max(at) - at) as u32);
            at = c.max(at);
            apu.write(reg, v);
            w.next();
        }
        apu.run((end - at) as u32);
        at = end;
        out.push(apu.state());
    }
    out
}
