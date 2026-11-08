//! Text tunes (`.tune`), a theme's sound replacements written as notes, so
//! a template's theme holds only text files and its sounds review and diff
//! in git like its `.pix` pictures (docs/formats/theme.md, "Sound
//! replacements"). A file is:
//!
//! ```text
//! tune 1
//! # a comment
//! step 6
//! loop
//! voice lead pluck 0.5
//! notes C5:2 E5:2 G5:4 -:2 C6>G5:4
//! voice bass soft 0.4
//! notes C3:4 G2:4
//! ```
//!
//! `step` is the frames (at 60 a second) one step lasts. `loop` marks a
//! tune that repeats while its role holds. Each `voice` names itself, a
//! wave and a volume (0 to 1), and its `notes` lines follow: a note name
//! with its octave (`C4`, `F#3`, `Bb5`), `-` for a rest, or a glide
//! `FROM>TO`, each with `:steps` (default 1). The waves are `square`,
//! `pulse` (a narrow square), `triangle`, `soft` (a sine), `pluck` (a sine
//! that dies away), `noise` and `thud` (low noise that dies away).
//!
//! An `sfx NAME VOLUME key=value ...` line adds an effect described by
//! parameters, sfxr's way ([`crate::sfx`]), starting at step `at`.
//!
//! [`render`] makes 16-bit mono samples, the same bytes on every machine:
//! the waves are worked out from integer phase steps and a fixed noise
//! generator, so a tune renders the same everywhere.

/// The sample rate a tune renders at.
pub const RATE: u32 = 22_050;

/// A wave a voice plays with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    Square,
    Pulse,
    Triangle,
    Soft,
    Pluck,
    Noise,
    Thud,
}

impl Wave {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "square" => Self::Square,
            "pulse" => Self::Pulse,
            "triangle" => Self::Triangle,
            "soft" => Self::Soft,
            "pluck" => Self::Pluck,
            "noise" => Self::Noise,
            "thud" => Self::Thud,
            _ => return None,
        })
    }
}

/// One note of a voice: MIDI note numbers (a glide's start and end, the
/// same for a held note), `None` for a rest, and its length in steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    pub from: Option<u8>,
    pub to: Option<u8>,
    pub steps: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Voice {
    pub name: String,
    pub wave: Wave,
    pub volume: f32,
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tune {
    /// Frames a step lasts, at 60 frames a second.
    pub step: u32,
    pub looped: bool,
    pub voices: Vec<Voice>,
    /// Effects by parameters (`sfx` lines, [`crate::sfx`]).
    pub effects: Vec<crate::sfx::Sfx>,
}

/// A note name and octave as a MIDI number: `C4` is 60.
pub fn note_number(s: &str) -> Option<u8> {
    let mut chars = s.chars();
    let base = match chars.next()? {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let (shift, oct) = if let Some(r) = rest.strip_prefix('#') {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('b') {
        (-1, r)
    } else {
        (0, rest.as_str())
    };
    let oct: i32 = oct.parse().ok()?;
    let n = (oct + 1) * 12 + base + shift;
    u8::try_from(n).ok().filter(|n| *n < 128)
}

impl Tune {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text
            .lines()
            .map(str::trim)
            .enumerate()
            .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'));
        match lines.next() {
            Some((_, "tune 1")) => {}
            _ => return Err("a tune starts with `tune 1`".into()),
        }
        let mut t = Tune {
            step: 6,
            looped: false,
            voices: Vec::new(),
            effects: Vec::new(),
        };
        for (i, line) in lines {
            let n = i + 1;
            let f: Vec<&str> = line.split_whitespace().collect();
            match f.as_slice() {
                ["step", s] => {
                    t.step = s
                        .parse()
                        .ok()
                        .filter(|v| (1..=120).contains(v))
                        .ok_or(format!("line {n}: a step is 1 to 120 frames"))?;
                }
                ["loop"] => t.looped = true,
                ["voice", name, wave, vol] => {
                    let wave = Wave::parse(wave).ok_or(format!(
                        "line {n}: `{wave}` isn't a wave (square, pulse, triangle, soft, pluck, noise, thud)"
                    ))?;
                    let volume: f32 = vol
                        .parse()
                        .ok()
                        .filter(|v: &f32| (0.0..=1.0).contains(v))
                        .ok_or(format!("line {n}: a volume is 0 to 1"))?;
                    t.voices.push(Voice {
                        name: (*name).to_owned(),
                        wave,
                        volume,
                        notes: Vec::new(),
                    });
                }
                ["sfx", rest @ ..] => {
                    t.effects
                        .push(crate::sfx::Sfx::parse(rest).map_err(|e| format!("line {n}: {e}"))?);
                }
                ["notes", rest @ ..] => {
                    let v = t
                        .voices
                        .last_mut()
                        .ok_or(format!("line {n}: notes before any voice"))?;
                    for w in rest {
                        v.notes
                            .push(parse_note(w).map_err(|e| format!("line {n}: {e}"))?);
                    }
                }
                _ => return Err(format!("line {n}: `{line}`")),
            }
        }
        if t.voices.is_empty() && t.effects.is_empty() {
            return Err("a tune has a voice or an effect".into());
        }
        if t.frames() > 60 * 60 {
            return Err("a tune lasts at most a minute".into());
        }
        Ok(t)
    }

    /// Its length in frames: the longest voice's or effect's.
    pub fn frames(&self) -> u32 {
        self.voices
            .iter()
            .map(|v| v.notes.iter().map(|n| n.steps).sum::<u32>() * self.step)
            .chain(self.effects.iter().map(|e| e.at * self.step + e.frames()))
            .max()
            .unwrap_or(0)
    }

    /// The tune's notes in time order, as (start frame, MIDI note) for the
    /// voice with the most notes: what the originality check compares.
    pub fn melody(&self) -> Vec<(u32, u8)> {
        let pitched = |v: &&Voice| !matches!(v.wave, Wave::Noise | Wave::Thud);
        let Some(v) = self
            .voices
            .iter()
            .filter(pitched)
            .max_by_key(|v| v.notes.iter().filter(|n| n.from.is_some()).count())
        else {
            return Vec::new();
        };
        let mut at = 0;
        let mut out = Vec::new();
        for n in &v.notes {
            if let Some(p) = n.from {
                out.push((at, p));
            }
            at += n.steps * self.step;
        }
        out
    }
}

fn parse_note(w: &str) -> Result<Note, String> {
    let (pitch, steps) = match w.split_once(':') {
        Some((p, s)) => (
            p,
            s.parse::<u32>()
                .ok()
                .filter(|s| (1..=256).contains(s))
                .ok_or(format!("`{w}`: a length is 1 to 256 steps"))?,
        ),
        None => (w, 1),
    };
    if pitch == "-" {
        return Ok(Note {
            from: None,
            to: None,
            steps,
        });
    }
    let (a, b) = pitch.split_once('>').unwrap_or((pitch, pitch));
    let a = note_number(a).ok_or(format!("`{w}` isn't a note"))?;
    let b = note_number(b).ok_or(format!("`{w}` isn't a note"))?;
    Ok(Note {
        from: Some(a),
        to: Some(b),
        steps,
    })
}

/// A MIDI note's frequency in Hz.
pub fn hz(n: f64) -> f64 {
    440.0 * 2f64.powf((n - 69.0) / 12.0)
}

/// The tune as 16-bit mono samples at [`RATE`], one pass.
pub fn render(t: &Tune) -> Vec<i16> {
    let per_frame = f64::from(RATE) / 60.0;
    let total = (f64::from(t.frames()) * per_frame).round() as usize;
    let mut mix = vec![0f32; total];
    for v in &t.voices {
        let mut at = 0usize;
        let mut phase = 0f64;
        let mut noise: u32 = 0x1234_5678;
        for n in &v.notes {
            let len = (f64::from(n.steps * t.step) * per_frame).round() as usize;
            if let (Some(a), Some(b)) = (n.from, n.to) {
                for i in 0..len {
                    let k = i as f64 / len.max(1) as f64;
                    let f = hz(f64::from(a) + (f64::from(b) - f64::from(a)) * k);
                    phase = (phase + f / f64::from(RATE)).fract();
                    noise ^= noise << 13;
                    noise ^= noise >> 17;
                    noise ^= noise << 5;
                    let white = (noise >> 8) as f64 / f64::from(1u32 << 24) * 2.0 - 1.0;
                    let s = match v.wave {
                        Wave::Square => {
                            if phase < 0.5 {
                                0.6
                            } else {
                                -0.6
                            }
                        }
                        Wave::Pulse => {
                            if phase < 0.125 {
                                0.6
                            } else {
                                -0.15
                            }
                        }
                        Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                        Wave::Soft | Wave::Pluck => (phase * std::f64::consts::TAU).sin(),
                        Wave::Noise | Wave::Thud => white,
                    };
                    // A short attack and release, so no note clicks; a
                    // pluck and a thud die away.
                    let attack = (i as f64 / 60.0).min(1.0);
                    let release = ((len - i) as f64 / 120.0).min(1.0);
                    let decay = match v.wave {
                        Wave::Pluck => (-4.0 * i as f64 / f64::from(RATE)).exp(),
                        Wave::Thud => (-12.0 * i as f64 / f64::from(RATE)).exp(),
                        _ => 1.0,
                    };
                    let s = if v.wave == Wave::Thud {
                        // Low noise: the white noise smoothed toward the
                        // note's own sine.
                        0.5 * white * 0.3 + 0.7 * (phase * std::f64::consts::TAU).sin()
                    } else {
                        s
                    };
                    if let Some(m) = mix.get_mut(at + i) {
                        *m += (s * attack * release * decay) as f32 * v.volume;
                    }
                }
            }
            at += len;
        }
    }
    for e in &t.effects {
        let at = (f64::from(e.at * t.step) * per_frame).round() as usize;
        for (i, v) in e.render(RATE).into_iter().enumerate() {
            if let Some(m) = mix.get_mut(at + i) {
                *m += v;
            }
        }
    }
    mix.iter()
        .map(|s| (s.clamp(-1.0, 1.0) * 28_000.0) as i16)
        .collect()
}

/// Samples as a 16-bit mono WAV file.
pub fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// A 16-bit PCM WAV file's samples, mixed to mono, and its rate.
pub fn read_wav(bytes: &[u8]) -> Result<(Vec<i16>, u32), String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF WAVE file".into());
    }
    let mut at = 12;
    let (mut channels, mut rate, mut bits) = (0u16, 0u32, 0u16);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap_or([0; 4])) as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        }
        if id == b"data" {
            if bits != 16 || channels == 0 {
                return Err(format!(
                    "a {bits}-bit WAV with {channels} channels; give 16-bit PCM"
                ));
            }
            let c = usize::from(channels);
            let frames = body.len() / 2 / c;
            let mut out = Vec::with_capacity(frames);
            for i in 0..frames {
                let mut sum = 0i32;
                for k in 0..c {
                    let o = (i * c + k) * 2;
                    sum += i32::from(i16::from_le_bytes([body[o], body[o + 1]]));
                }
                out.push((sum / c as i32) as i16);
            }
            return Ok((out, rate));
        }
        at += 8 + len + (len & 1);
    }
    Err("the WAV file has no data".into())
}

/// A sound replacement's samples at [`RATE`]: a tune rendered, or a WAV
/// file resampled. An Ogg file isn't decoded here.
pub fn samples_of(file: &str, bytes: &[u8]) -> Result<Vec<i16>, String> {
    let lower = file.to_ascii_lowercase();
    if lower.ends_with(".tune") {
        let text = std::str::from_utf8(bytes).map_err(|_| format!("{file} isn't UTF-8 text"))?;
        return Ok(render(
            &Tune::parse(text).map_err(|e| format!("{file}: {e}"))?,
        ));
    }
    if lower.ends_with(".wav") {
        let (s, rate) = read_wav(bytes).map_err(|e| format!("{file}: {e}"))?;
        return Ok(resample(&s, rate, RATE));
    }
    Err(format!(
        "{file}: only .tune and .wav files are decoded here"
    ))
}

/// Linear resampling.
pub fn resample(s: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || from == 0 || s.is_empty() {
        return s.to_vec();
    }
    let n = (s.len() as u64 * u64::from(to) / u64::from(from)) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * f64::from(from) / f64::from(to);
            let a = x.floor() as usize;
            let b = (a + 1).min(s.len() - 1);
            let k = x - a as f64;
            (f64::from(s[a.min(s.len() - 1)]) * (1.0 - k) + f64::from(s[b]) * k) as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tune_parses_renders_and_round_trips_through_wav() {
        let t = Tune::parse(
            "tune 1\n# a jump\nstep 2\nvoice lead pluck 0.5\nnotes C5:2 E5 G5>C6:4 -:2\nvoice drum noise 0.2\nnotes C4:1\n",
        )
        .unwrap();
        assert_eq!(t.frames(), 18);
        assert_eq!(note_number("C4"), Some(60));
        assert_eq!(note_number("F#3"), Some(54));
        assert_eq!(note_number("Bb5"), Some(82));
        assert_eq!(t.melody(), vec![(0, 72), (4, 76), (6, 79)]);
        let s = render(&t);
        assert_eq!(s.len(), (18.0 * f64::from(RATE) / 60.0).round() as usize);
        assert_eq!(render(&t), s, "the same samples every time");
        assert!(s.iter().any(|v| v.unsigned_abs() > 1000));
        let w = wav(&s, RATE);
        assert_eq!(read_wav(&w).unwrap(), (s.clone(), RATE));
        assert_eq!(samples_of("x.wav", &w).unwrap(), s);
        assert!(Tune::parse("tune 1\nnotes C4\n").is_err());
        assert!(Tune::parse("tune 1\nvoice a kazoo 0.5\n").is_err());
        assert!(Tune::parse("tune 1\nvoice a soft 0.5\nnotes H4\n").is_err());
    }
}
