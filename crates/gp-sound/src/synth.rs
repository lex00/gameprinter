//! The synthesizer: the APU model run at the CPU's clock, each channel's
//! output averaged over each sample, mixed with the NES mixer's formulas
//! and filtered as the console's output stage does (a high pass that takes
//! out the DC offset). Every runtime plays sound through it, so the Bevy
//! build, the web preview and the fidelity check hear the same samples
//! from the same writes. Floats appear only here, in presentation: the
//! operations are IEEE additions, multiplications and divisions, so every
//! platform makes the same samples.

use crate::apu::{Apu, Region, Sample, mix};

/// The sample rate everything renders at.
pub const SAMPLE_RATE: u32 = 44_100;

/// Per-channel gains (1.0 is as the console mixes it, 0.0 mutes), in
/// channel order.
pub type Gains = [f64; 5];

pub const UNITY: Gains = [1.0; 5];

/// A synthesizer fed a frame at a time.
#[derive(Debug, Clone)]
pub struct Synth {
    apu: Apu,
    rate: u64,
    /// CPU cycles run and samples made since the start.
    cycle: u64,
    sample: u64,
    frame: u64,
    hp_x: f64,
    hp_y: f64,
    pub gains: Gains,
}

impl Synth {
    pub fn new(region: Region, rate: u32, samples: Vec<Sample>) -> Self {
        Self {
            apu: Apu::new(region).with_samples(samples),
            rate: u64::from(rate),
            cycle: 0,
            sample: 0,
            frame: 0,
            hp_x: 0.0,
            hp_y: 0.0,
            gains: UNITY,
        }
    }

    fn hz(&self) -> u64 {
        self.apu.region().cpu_hz()
    }

    /// Makes samples until `cycle`, appending them to `out`.
    fn run_to(&mut self, cycle: u64, out: &mut Vec<i16>) {
        loop {
            let next = (self.sample + 1) * self.hz() / self.rate;
            if next > cycle {
                if cycle > self.cycle {
                    self.apu.run((cycle - self.cycle) as u32);
                    self.cycle = cycle;
                }
                return;
            }
            self.apu.run((next - self.cycle) as u32);
            self.cycle = next;
            self.sample += 1;
            let span = (next - (self.sample - 1) * self.hz() / self.rate).max(1);
            let s = self.apu.take_sums();
            let lvl = |i: usize| s[i] as f64 / span as f64 * self.gains[i];
            let x = mix(lvl(0), lvl(1), lvl(2), lvl(3), lvl(4));
            // A first-order high pass near 37 Hz.
            let y = 0.995 * (self.hp_y + x - self.hp_x);
            self.hp_x = x;
            self.hp_y = y;
            out.push((y * 30_000.0).round().clamp(-32_768.0, 32_767.0) as i16);
        }
    }

    /// Runs one frame with its writes (CPU cycle within the frame,
    /// register, value) and returns its samples.
    pub fn frame(&mut self, writes: &[(u32, u8, u8)]) -> Vec<i16> {
        let region = self.apu.region();
        let start = region.frame_start(self.frame);
        let end = region.frame_start(self.frame + 1);
        let mut out = Vec::with_capacity(800);
        for &(c, reg, v) in writes {
            self.run_to((start + u64::from(c)).min(end), &mut out);
            self.apu.write(reg, v);
        }
        self.run_to(end, &mut out);
        self.frame += 1;
        out
    }
}

/// Renders a timeline of writes at absolute CPU cycles to `end_cycle`.
pub fn render(
    region: Region,
    timeline: &[(u64, u8, u8)],
    end_cycle: u64,
    samples: &[Sample],
    gains: Gains,
) -> Vec<i16> {
    let mut s = Synth::new(region, SAMPLE_RATE, samples.to_vec());
    s.gains = gains;
    let mut out = Vec::new();
    for &(c, reg, v) in timeline {
        s.run_to(c.min(end_cycle), &mut out);
        s.apu.write(reg, v);
    }
    s.run_to(end_cycle, &mut out);
    out
}

/// A mono 16-bit PCM WAV file.
pub fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// The RMS level of each block of `block` samples: a loudness curve, for
/// comparing two renders.
pub fn envelope(samples: &[i16], block: usize) -> Vec<f64> {
    samples
        .chunks(block.max(1))
        .map(|c| {
            let sum: f64 = c.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
            (sum / c.len() as f64).sqrt()
        })
        .collect()
}
