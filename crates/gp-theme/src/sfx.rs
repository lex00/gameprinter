//! Sound effects described by parameters, in the manner of sfxr (Tomas
//! Pettersson's sound effect generator, MIT licensed,
//! https://drpetter.se/project_sfxr.html): a wave, a start pitch and how it
//! slides, an envelope with a punch, a duty sweep, vibrato, an arpeggio
//! jump, a repeat and two filters. Our own code, written from that
//! program's list of parameters, in plain units: notes or hertz,
//! semitones a second, frames at 60 a second.
//!
//! A `.tune` holds one on a line (docs/formats/theme.md, "Sound
//! replacements"):
//!
//! ```text
//! sfx zap 0.5 wave=square freq=C6 slide=-36 decay=12 duty=0.3 duty_sweep=-0.5
//! ```

use crate::tune::{hz, note_number};

/// An effect's wave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SfxWave {
    Square,
    Saw,
    Sine,
    Noise,
}

/// One effect.
#[derive(Debug, Clone, PartialEq)]
pub struct Sfx {
    pub name: String,
    pub volume: f32,
    pub wave: SfxWave,
    /// The start pitch, Hz.
    pub freq: f64,
    /// Pitch change, semitones a second, and its change a second.
    pub slide: f64,
    pub dslide: f64,
    /// The lowest pitch, Hz: the effect stops below it.
    pub min: f64,
    /// Envelope, frames: a rise, a hold and a fall.
    pub attack: f64,
    pub sustain: f64,
    pub decay: f64,
    /// Extra loudness at the hold's start, 0 to 1, fading through it.
    pub punch: f64,
    /// A square's high share, 0 to 1, and its change a second.
    pub duty: f64,
    pub duty_sweep: f64,
    /// Vibrato depth, semitones, and rate, Hz.
    pub vibrato: f64,
    pub vibrato_rate: f64,
    /// A jump of this many semitones `arp_at` frames in.
    pub arp: f64,
    pub arp_at: f64,
    /// Pitch, slide and arpeggio start again every this many frames (0:
    /// never).
    pub repeat: f64,
    /// A one-pole low pass, 0 to 1 (1: open), and high pass (0: off).
    pub lowpass: f64,
    pub highpass: f64,
    /// When it starts, in the tune's steps.
    pub at: u32,
}

impl Default for Sfx {
    fn default() -> Self {
        Self {
            name: String::new(),
            volume: 0.5,
            wave: SfxWave::Square,
            freq: 440.0,
            slide: 0.0,
            dslide: 0.0,
            min: 20.0,
            attack: 0.0,
            sustain: 6.0,
            decay: 10.0,
            punch: 0.0,
            duty: 0.5,
            duty_sweep: 0.0,
            vibrato: 0.0,
            vibrato_rate: 6.0,
            arp: 0.0,
            arp_at: 0.0,
            repeat: 0.0,
            lowpass: 1.0,
            highpass: 0.0,
            at: 0,
        }
    }
}

/// The keys an `sfx` line takes.
pub const KEYS: &[&str] = &[
    "wave",
    "freq",
    "slide",
    "dslide",
    "min",
    "attack",
    "sustain",
    "decay",
    "punch",
    "duty",
    "duty_sweep",
    "vibrato",
    "vibrato_rate",
    "arp",
    "arp_at",
    "repeat",
    "lowpass",
    "highpass",
    "at",
];

fn pitch(v: &str) -> Option<f64> {
    note_number(v).map(|n| hz(f64::from(n))).or_else(|| {
        v.parse::<f64>()
            .ok()
            .filter(|f| (20.0..=12_000.0).contains(f))
    })
}

impl Sfx {
    /// An `sfx NAME VOLUME key=value ...` line's fields after `sfx`.
    pub fn parse(fields: &[&str]) -> Result<Self, String> {
        let [name, vol, rest @ ..] = fields else {
            return Err("an effect is `sfx NAME VOLUME key=value ...`".into());
        };
        let mut s = Sfx {
            name: (*name).to_owned(),
            volume: vol
                .parse()
                .ok()
                .filter(|v: &f32| (0.0..=1.0).contains(v))
                .ok_or("a volume is 0 to 1")?,
            ..Sfx::default()
        };
        for kv in rest {
            let (k, v) = kv
                .split_once('=')
                .ok_or(format!("`{kv}` isn't key=value"))?;
            let num = |lo: f64, hi: f64| -> Result<f64, String> {
                v.parse::<f64>()
                    .ok()
                    .filter(|x| (lo..=hi).contains(x))
                    .ok_or(format!("{k} is {lo} to {hi}"))
            };
            match k {
                "wave" => {
                    s.wave = match v {
                        "square" => SfxWave::Square,
                        "saw" => SfxWave::Saw,
                        "sine" => SfxWave::Sine,
                        "noise" => SfxWave::Noise,
                        _ => return Err(format!("`{v}` isn't square, saw, sine or noise")),
                    }
                }
                "freq" => {
                    s.freq = pitch(v).ok_or(format!("`{v}` isn't a note or 20 to 12000 Hz"))?
                }
                "min" => s.min = pitch(v).ok_or(format!("`{v}` isn't a note or 20 to 12000 Hz"))?,
                "slide" => s.slide = num(-240.0, 240.0)?,
                "dslide" => s.dslide = num(-960.0, 960.0)?,
                "attack" => s.attack = num(0.0, 600.0)?,
                "sustain" => s.sustain = num(0.0, 600.0)?,
                "decay" => s.decay = num(0.0, 600.0)?,
                "punch" => s.punch = num(0.0, 1.0)?,
                "duty" => s.duty = num(0.0, 1.0)?,
                "duty_sweep" => s.duty_sweep = num(-4.0, 4.0)?,
                "vibrato" => s.vibrato = num(0.0, 12.0)?,
                "vibrato_rate" => s.vibrato_rate = num(0.0, 60.0)?,
                "arp" => s.arp = num(-48.0, 48.0)?,
                "arp_at" => s.arp_at = num(0.0, 600.0)?,
                "repeat" => s.repeat = num(0.0, 600.0)?,
                "lowpass" => s.lowpass = num(0.0, 1.0)?,
                "highpass" => s.highpass = num(0.0, 1.0)?,
                "at" => {
                    s.at = v
                        .parse()
                        .ok()
                        .filter(|x| *x <= 3600)
                        .ok_or("at is 0 to 3600 steps")?;
                }
                _ => return Err(format!("`{k}` isn't one of {}", KEYS.join(", "))),
            }
        }
        if s.attack + s.sustain + s.decay <= 0.0 {
            return Err("an effect lasts more than 0 frames".into());
        }
        Ok(s)
    }

    /// Its length in frames.
    pub fn frames(&self) -> u32 {
        (self.attack + self.sustain + self.decay).ceil() as u32
    }

    /// Its samples at `rate`, -1 to 1 times its volume.
    pub fn render(&self, rate: u32) -> Vec<f32> {
        let rate = f64::from(rate);
        let per_frame = rate / 60.0;
        let (a, su, d) = (
            self.attack * per_frame,
            self.sustain * per_frame,
            self.decay * per_frame,
        );
        let total = (a + su + d).round() as usize;
        let mut out = Vec::with_capacity(total);
        let mut phase = 0.0f64;
        let mut noise: u32 = 0x9E37_79B9;
        let mut noise_v = 0.0f64;
        let (mut lp_y, mut hp_y, mut prev_x) = (0.0f64, 0.0f64, 0.0f64);
        let lp_k = self.lowpass * self.lowpass;
        let hp_k = 1.0 - self.highpass * 0.98;
        for i in 0..total {
            let t_all = i as f64 / rate;
            let frame = i as f64 / per_frame;
            let local = if self.repeat > 0.0 {
                frame % self.repeat
            } else {
                frame
            };
            let t = local / 60.0;
            let mut semi = self.slide * t + 0.5 * self.dslide * t * t;
            if self.vibrato > 0.0 {
                semi += self.vibrato * (std::f64::consts::TAU * self.vibrato_rate * t_all).sin();
            }
            if self.arp != 0.0 && local >= self.arp_at {
                semi += self.arp;
            }
            let f = self.freq * 2f64.powf(semi / 12.0);
            if f < self.min {
                break;
            }
            let before = phase;
            phase = (phase + f / rate).fract();
            if phase < before || i == 0 {
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                noise_v = f64::from(noise >> 8) / f64::from(1u32 << 24) * 2.0 - 1.0;
            }
            let duty = (self.duty + self.duty_sweep * t_all).clamp(0.05, 0.95);
            let x = match self.wave {
                SfxWave::Square => {
                    if phase < duty {
                        0.6
                    } else {
                        -0.6
                    }
                }
                SfxWave::Saw => 2.0 * phase - 1.0,
                SfxWave::Sine => (phase * std::f64::consts::TAU).sin(),
                SfxWave::Noise => noise_v,
            };
            let at = i as f64;
            let env = if at < a {
                at / a.max(1.0)
            } else if at < a + su {
                1.0 + self.punch * (1.0 - (at - a) / su.max(1.0))
            } else {
                (1.0 - (at - a - su) / d.max(1.0)).max(0.0)
            };
            lp_y += lp_k * (x - lp_y);
            let y = if self.lowpass < 1.0 { lp_y } else { x };
            hp_y = hp_k * (hp_y + y - prev_x);
            prev_x = y;
            let y = if self.highpass > 0.0 { hp_y } else { y };
            out.push((y * env) as f32 * self.volume);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_effect_parses_and_renders_the_same_each_time() {
        let s = Sfx::parse(&[
            "zap",
            "0.5",
            "wave=square",
            "freq=C6",
            "slide=-36",
            "decay=12",
            "punch=0.4",
            "duty=0.3",
            "duty_sweep=-0.5",
            "arp=12",
            "arp_at=3",
            "lowpass=0.8",
        ])
        .unwrap();
        assert_eq!(s.frames(), 18);
        let a = s.render(22_050);
        assert_eq!(a, s.render(22_050));
        assert!(a.iter().any(|v| v.abs() > 0.2));
        assert!(Sfx::parse(&["x", "0.5", "wave=kazoo"]).is_err());
        assert!(Sfx::parse(&["x", "0.5", "bogus=1"]).is_err());
        // A falling pitch that hits its floor stops early.
        let f = Sfx::parse(&[
            "fall",
            "0.5",
            "freq=C5",
            "slide=-120",
            "min=C3",
            "sustain=60",
        ])
        .unwrap();
        assert!(f.render(22_050).len() < (66.0 * 22_050.0 / 60.0) as usize);
    }
}
