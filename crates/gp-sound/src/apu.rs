//! A model of the NES APU, the 2A03's sound: two pulse channels, a
//! triangle, a noise channel and the DMC, with their length counters,
//! envelopes, sweeps, linear counter and the frame counter that clocks
//! them. It follows the NESdev wiki's APU pages (APU, APU Pulse, APU
//! Triangle, APU Noise, APU DMC, APU Frame Counter, APU Mixer).
//!
//! Two things use it. Note capture applies a register log and samples each
//! channel's state at the end of every frame ([`Apu::state`]); the
//! synthesizer ([`crate::synth`]) runs it and averages each channel's
//! output over each sample. It is presentation: no simulation reads it,
//! and nothing in it reads a clock.

use serde::{Deserialize, Serialize};

/// Length counter loads, by the top five bits of a channel's fourth
/// register.
pub const LENGTHS: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

const DUTY: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

const TRIANGLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

/// Noise timer periods in CPU cycles, by the period index.
pub const NOISE_NTSC: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];
pub const NOISE_PAL: [u16; 16] = [
    4, 8, 14, 30, 60, 88, 118, 148, 188, 236, 354, 472, 708, 944, 1890, 3778,
];
/// DMC rates in CPU cycles per output bit, by the rate index.
pub const DMC_NTSC: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];
pub const DMC_PAL: [u16; 16] = [
    398, 354, 316, 298, 276, 236, 210, 198, 176, 148, 132, 118, 98, 78, 66, 50,
];

/// The console's region: its clock rate and frame length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    #[default]
    Ntsc,
    Pal,
}

impl Region {
    /// The CPU clock in Hz (the NESdev wiki's Cycle reference chart).
    pub fn cpu_hz(self) -> u64 {
        match self {
            Self::Ntsc => 1_789_773,
            Self::Pal => 1_662_607,
        }
    }

    /// PPU dots in a frame: 262 or 312 lines of 341 dots.
    pub fn frame_dots(self) -> u64 {
        match self {
            Self::Ntsc => 262 * 341,
            Self::Pal => 312 * 341,
        }
    }

    /// CPU cycles from dots: 3 dots a cycle on NTSC, 3.2 on PAL.
    pub fn dots_to_cycles(self, dots: u64) -> u64 {
        match self {
            Self::Ntsc => dots / 3,
            Self::Pal => dots * 5 / 16,
        }
    }

    /// The CPU cycle frame `n` starts at, counting from frame 0.
    pub fn frame_start(self, n: u64) -> u64 {
        self.dots_to_cycles(n * self.frame_dots())
    }

    /// Frames a second, as a ratio (60.0988 or 50.007).
    pub fn frame_rate(self) -> f64 {
        self.cpu_hz() as f64 / (self.frame_start(1000) as f64 / 1000.0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Ntsc => "ntsc",
            Self::Pal => "pal",
        }
    }

    /// The frame counter's steps in CPU cycles, and the 4- and 5-step
    /// sequences' lengths.
    fn steps(self) -> ([u32; 5], u32, u32) {
        match self {
            Self::Ntsc => ([7457, 14913, 22371, 29829, 37281], 29830, 37282),
            Self::Pal => ([8313, 16627, 24939, 33252, 41565], 33254, 41566),
        }
    }

    fn noise(self) -> &'static [u16; 16] {
        match self {
            Self::Ntsc => &NOISE_NTSC,
            Self::Pal => &NOISE_PAL,
        }
    }

    fn dmc(self) -> &'static [u16; 16] {
        match self {
            Self::Ntsc => &DMC_NTSC,
            Self::Pal => &DMC_PAL,
        }
    }
}

/// The five channels, in register order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Pulse1,
    Pulse2,
    Triangle,
    Noise,
    Dmc,
}

impl Channel {
    pub const ALL: [Channel; 5] = [
        Self::Pulse1,
        Self::Pulse2,
        Self::Triangle,
        Self::Noise,
        Self::Dmc,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Pulse1 => "pulse1",
            Self::Pulse2 => "pulse2",
            Self::Triangle => "triangle",
            Self::Noise => "noise",
            Self::Dmc => "dmc",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Default)]
struct Envelope {
    start: bool,
    divider: u8,
    decay: u8,
    looping: bool,
    constant: bool,
    period: u8,
}

impl Envelope {
    fn write(&mut self, v: u8) {
        self.looping = v & 0x20 != 0;
        self.constant = v & 0x10 != 0;
        self.period = v & 0x0F;
    }

    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.period;
        } else if self.divider == 0 {
            self.divider = self.period;
            if self.decay > 0 {
                self.decay -= 1;
            } else if self.looping {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.constant {
            self.period
        } else {
            self.decay
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Pulse {
    second: bool,
    enabled: bool,
    duty: u8,
    step: u8,
    period: u16,
    /// CPU cycles left until the sequencer steps.
    timer: u32,
    length: u8,
    env: Envelope,
    sweep_on: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_reload: bool,
    sweep_divider: u8,
    acc: u64,
}

impl Pulse {
    fn target(&self) -> i32 {
        let p = i32::from(self.period);
        let change = p >> self.sweep_shift;
        if self.sweep_negate {
            p - change - i32::from(!self.second)
        } else {
            p + change
        }
    }

    fn muted(&self) -> bool {
        self.period < 8 || (!self.sweep_negate && self.target() > 0x7FF)
    }

    fn write(&mut self, r: u8, v: u8) {
        match r {
            0 => {
                self.duty = v >> 6;
                self.env.write(v);
            }
            1 => {
                self.sweep_on = v & 0x80 != 0;
                self.sweep_period = (v >> 4) & 7;
                self.sweep_negate = v & 0x08 != 0;
                self.sweep_shift = v & 7;
                self.sweep_reload = true;
            }
            2 => self.period = (self.period & 0x700) | u16::from(v),
            _ => {
                self.period = (self.period & 0xFF) | (u16::from(v & 7) << 8);
                if self.enabled {
                    self.length = LENGTHS[usize::from(v >> 3)];
                }
                self.step = 0;
                self.env.start = true;
            }
        }
    }

    fn half(&mut self) {
        if !self.env.looping && self.length > 0 {
            self.length -= 1;
        }
        if self.sweep_divider == 0 && self.sweep_on && self.sweep_shift > 0 && !self.muted() {
            self.period = self.target().clamp(0, 0x7FF) as u16;
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.muted()
            || self.length == 0
            || DUTY[usize::from(self.duty)][usize::from(self.step)] == 0
        {
            0
        } else {
            self.env.output()
        }
    }

    /// The volume it plays at, whatever the duty step: 0 when silent.
    fn volume(&self) -> u8 {
        if self.muted() || self.length == 0 {
            0
        } else {
            self.env.output()
        }
    }

    fn run(&mut self, mut n: u32) {
        let period = 2 * (u32::from(self.period) + 1);
        while n > 0 {
            if self.timer == 0 {
                self.timer = period;
            }
            let k = n.min(self.timer);
            self.acc += u64::from(self.output()) * u64::from(k);
            self.timer -= k;
            n -= k;
            if self.timer == 0 {
                self.step = (self.step + 1) & 7;
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Triangle {
    enabled: bool,
    control: bool,
    reload_value: u8,
    linear: u8,
    reload: bool,
    period: u16,
    timer: u32,
    step: u8,
    length: u8,
    acc: u64,
}

impl Triangle {
    fn write(&mut self, r: u8, v: u8) {
        match r {
            0 => {
                self.control = v & 0x80 != 0;
                self.reload_value = v & 0x7F;
            }
            1 => {}
            2 => self.period = (self.period & 0x700) | u16::from(v),
            _ => {
                self.period = (self.period & 0xFF) | (u16::from(v & 7) << 8);
                if self.enabled {
                    self.length = LENGTHS[usize::from(v >> 3)];
                }
                self.reload = true;
            }
        }
    }

    fn quarter(&mut self) {
        if self.reload {
            self.linear = self.reload_value;
        } else if self.linear > 0 {
            self.linear -= 1;
        }
        if !self.control {
            self.reload = false;
        }
    }

    fn half(&mut self) {
        if !self.control && self.length > 0 {
            self.length -= 1;
        }
    }

    fn audible(&self) -> bool {
        self.length > 0 && self.linear > 0 && self.period >= 2
    }

    fn run(&mut self, mut n: u32) {
        let period = u32::from(self.period) + 1;
        let moving = self.audible();
        while n > 0 {
            if self.timer == 0 {
                self.timer = period;
            }
            let k = n.min(self.timer);
            self.acc += u64::from(TRIANGLE[usize::from(self.step)]) * u64::from(k);
            self.timer -= k;
            n -= k;
            if self.timer == 0 && moving {
                self.step = (self.step + 1) & 31;
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Noise {
    enabled: bool,
    env: Envelope,
    mode: bool,
    period: u8,
    lfsr: u16,
    timer: u32,
    length: u8,
    acc: u64,
}

impl Default for Noise {
    fn default() -> Self {
        Self {
            enabled: false,
            env: Envelope::default(),
            mode: false,
            period: 0,
            lfsr: 1,
            timer: 0,
            length: 0,
            acc: 0,
        }
    }
}

impl Noise {
    fn write(&mut self, r: u8, v: u8) {
        match r {
            0 => self.env.write(v),
            1 => {}
            2 => {
                self.mode = v & 0x80 != 0;
                self.period = v & 0x0F;
            }
            _ => {
                if self.enabled {
                    self.length = LENGTHS[usize::from(v >> 3)];
                }
                self.env.start = true;
            }
        }
    }

    fn half(&mut self) {
        if !self.env.looping && self.length > 0 {
            self.length -= 1;
        }
    }

    fn volume(&self) -> u8 {
        if self.length == 0 {
            0
        } else {
            self.env.output()
        }
    }

    fn output(&self) -> u8 {
        if self.lfsr & 1 == 1 { 0 } else { self.volume() }
    }

    fn run(&mut self, mut n: u32, table: &[u16; 16]) {
        let period = u32::from(table[usize::from(self.period)]);
        while n > 0 {
            if self.timer == 0 {
                self.timer = period;
            }
            let k = n.min(self.timer);
            self.acc += u64::from(self.output()) * u64::from(k);
            self.timer -= k;
            n -= k;
            if self.timer == 0 {
                let other = if self.mode { 6 } else { 1 };
                let feedback = (self.lfsr ^ (self.lfsr >> other)) & 1;
                self.lfsr = (self.lfsr >> 1) | (feedback << 14);
            }
        }
    }
}

/// A DMC sample: the bytes the DMC reads from `addr` on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    /// The CPU address ($C000 + 64 * the $4012 value).
    pub addr: u16,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
struct Dmc {
    looping: bool,
    rate: u8,
    level: u8,
    start: u16,
    len: u16,
    /// Bytes of the current sample read so far, and left.
    pos: u16,
    left: u16,
    shift: u8,
    bits: u8,
    timer: u32,
    /// The current sample's bytes, if the synthesizer was given them.
    bytes: Option<Vec<u8>>,
    /// Started since the state was last sampled.
    started: bool,
    acc: u64,
}

impl Dmc {
    fn write(&mut self, r: u8, v: u8) {
        match r {
            0 => {
                self.looping = v & 0x40 != 0;
                self.rate = v & 0x0F;
            }
            1 => self.level = v & 0x7F,
            2 => self.start = 0xC000 | (u16::from(v) << 6),
            _ => self.len = (u16::from(v) << 4) + 1,
        }
    }

    fn restart(&mut self, samples: &[Sample]) {
        self.pos = 0;
        self.left = self.len;
        self.bytes = samples
            .iter()
            .find(|s| s.addr == self.start)
            .map(|s| s.bytes.clone());
        self.started = true;
    }

    fn run(&mut self, mut n: u32, table: &[u16; 16], samples: &[Sample]) {
        let period = u32::from(table[usize::from(self.rate)]);
        while n > 0 {
            if self.timer == 0 {
                self.timer = period;
            }
            let k = n.min(self.timer);
            self.acc += u64::from(self.level) * u64::from(k);
            self.timer -= k;
            n -= k;
            if self.timer == 0 {
                if self.bits == 0 {
                    // Fetch the next byte, if there is one and we have it.
                    self.bits = 8;
                    self.shift = 0;
                    if self.left > 0 {
                        self.shift = self
                            .bytes
                            .as_ref()
                            .and_then(|b| b.get(usize::from(self.pos)).copied())
                            .unwrap_or(0xAA);
                        self.pos += 1;
                        self.left -= 1;
                        if self.left == 0 && self.looping {
                            self.restart(samples);
                        }
                    } else {
                        // Silence: the level holds.
                        self.bits = 0;
                        continue;
                    }
                }
                if self.bytes.is_some() {
                    if self.shift & 1 == 1 {
                        if self.level <= 125 {
                            self.level += 2;
                        }
                    } else if self.level >= 2 {
                        self.level -= 2;
                    }
                }
                self.shift >>= 1;
                self.bits -= 1;
            }
        }
    }
}

/// One channel's state at the end of a frame: what capture reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelState {
    /// The timer period (pulse, triangle), the period index (noise) or the
    /// rate index (DMC).
    pub period: u16,
    /// The volume it plays at now, 0 when silent: 0-15, or for the
    /// triangle 15 while it sounds; for the DMC 15 while a sample plays.
    pub volume: u8,
    /// The pulse duty (0-3), or the noise mode (1 for the short loop).
    pub duty: u8,
    /// The channel's fourth register was written this frame (a pulse or
    /// noise envelope restart, a triangle reload), or a DMC sample started.
    pub trigger: bool,
    /// The DMC sample's address and length, when one started.
    pub sample: Option<(u16, u16)>,
}

/// The APU.
#[derive(Debug, Clone)]
pub struct Apu {
    region: Region,
    pulse: [Pulse; 2],
    triangle: Triangle,
    noise: Noise,
    dmc: Dmc,
    five_step: bool,
    /// CPU cycles since the frame counter's sequence started.
    fc: u32,
    fc_step: usize,
    triggers: [bool; 5],
    samples: Vec<Sample>,
}

impl Apu {
    pub fn new(region: Region) -> Self {
        let mut pulse = [Pulse::default(), Pulse::default()];
        pulse[1].second = true;
        Self {
            region,
            pulse,
            triangle: Triangle::default(),
            noise: Noise::default(),
            dmc: Dmc::default(),
            five_step: false,
            fc: 0,
            fc_step: 0,
            triggers: [false; 5],
            samples: Vec::new(),
        }
    }

    /// Gives the DMC its samples' bytes.
    pub fn with_samples(mut self, samples: Vec<Sample>) -> Self {
        self.samples = samples;
        self
    }

    pub fn region(&self) -> Region {
        self.region
    }

    /// Writes a register: `reg` is the offset from $4000 (0x00-0x13, 0x15,
    /// 0x17). Other offsets are ignored.
    pub fn write(&mut self, reg: u8, v: u8) {
        match reg {
            0x00..=0x03 => {
                self.pulse[0].write(reg, v);
                self.triggers[0] |= reg == 3;
            }
            0x04..=0x07 => {
                self.pulse[1].write(reg - 4, v);
                self.triggers[1] |= reg == 7;
            }
            0x08..=0x0B => {
                self.triangle.write(reg - 8, v);
                self.triggers[2] |= reg == 0x0B;
            }
            0x0C..=0x0F => {
                self.noise.write(reg - 0x0C, v);
                self.triggers[3] |= reg == 0x0F;
            }
            0x10..=0x13 => self.dmc.write(reg - 0x10, v),
            0x15 => {
                let on = |b: u8| v & b != 0;
                self.pulse[0].enabled = on(1);
                self.pulse[1].enabled = on(2);
                self.triangle.enabled = on(4);
                self.noise.enabled = on(8);
                for (i, e) in [on(1), on(2), on(4), on(8)].into_iter().enumerate() {
                    if !e {
                        match i {
                            0 => self.pulse[0].length = 0,
                            1 => self.pulse[1].length = 0,
                            2 => self.triangle.length = 0,
                            _ => self.noise.length = 0,
                        }
                    }
                }
                if on(0x10) {
                    if self.dmc.left == 0 {
                        let samples = std::mem::take(&mut self.samples);
                        self.dmc.restart(&samples);
                        self.samples = samples;
                        self.triggers[4] = true;
                    }
                } else {
                    self.dmc.left = 0;
                }
            }
            0x17 => {
                self.five_step = v & 0x80 != 0;
                self.fc = 0;
                self.fc_step = 0;
                if self.five_step {
                    self.quarter();
                    self.half();
                }
            }
            _ => {}
        }
    }

    fn quarter(&mut self) {
        self.pulse[0].env.clock();
        self.pulse[1].env.clock();
        self.noise.env.clock();
        self.triangle.quarter();
    }

    fn half(&mut self) {
        self.pulse[0].half();
        self.pulse[1].half();
        self.triangle.half();
        self.noise.half();
    }

    /// Runs `n` CPU cycles.
    pub fn run(&mut self, mut n: u32) {
        let (steps, four, five) = self.region.steps();
        while n > 0 {
            let next = if self.fc_step < 5 {
                steps[self.fc_step]
            } else {
                u32::MAX
            };
            let k = n.min(next.saturating_sub(self.fc).max(1));
            self.pulse[0].run(k);
            self.pulse[1].run(k);
            self.triangle.run(k);
            self.noise.run(k, self.region.noise());
            self.dmc.run(k, self.region.dmc(), &self.samples);
            self.fc += k;
            n -= k;
            if self.fc >= next {
                let s = self.fc_step;
                self.fc_step += 1;
                match (self.five_step, s) {
                    (false, 0) | (false, 2) | (true, 0) | (true, 2) => self.quarter(),
                    (false, 1) | (false, 3) | (true, 1) | (true, 4) => {
                        self.quarter();
                        self.half();
                    }
                    _ => {}
                }
                let end = if self.five_step { five } else { four };
                if (self.five_step && s == 4) || (!self.five_step && s == 3) {
                    self.fc = self.fc.saturating_sub(end);
                    self.fc_step = 0;
                }
            }
        }
    }

    /// Each channel's output summed over the cycles run since the last
    /// call, then cleared: pulse and noise 0-15 a cycle, triangle 0-15,
    /// DMC 0-127.
    pub fn take_sums(&mut self) -> [u64; 5] {
        let out = [
            self.pulse[0].acc,
            self.pulse[1].acc,
            self.triangle.acc,
            self.noise.acc,
            self.dmc.acc,
        ];
        self.pulse[0].acc = 0;
        self.pulse[1].acc = 0;
        self.triangle.acc = 0;
        self.noise.acc = 0;
        self.dmc.acc = 0;
        out
    }

    /// Every channel's state now, and whether each was triggered since the
    /// last call (which clears the triggers).
    pub fn state(&mut self) -> [ChannelState; 5] {
        let t = std::mem::take(&mut self.triggers);
        let p = |c: &Pulse, trig| ChannelState {
            period: c.period,
            volume: c.volume(),
            duty: c.duty,
            trigger: trig,
            sample: None,
        };
        let dmc_started = std::mem::take(&mut self.dmc.started);
        [
            p(&self.pulse[0], t[0]),
            p(&self.pulse[1], t[1]),
            ChannelState {
                period: self.triangle.period,
                volume: if self.triangle.audible() { 15 } else { 0 },
                duty: 0,
                trigger: t[2],
                sample: None,
            },
            ChannelState {
                period: u16::from(self.noise.period),
                volume: self.noise.volume(),
                duty: u8::from(self.noise.mode),
                trigger: t[3],
                sample: None,
            },
            ChannelState {
                period: u16::from(self.dmc.rate),
                volume: if self.dmc.left > 0 || dmc_started {
                    15
                } else {
                    0
                },
                duty: 0,
                trigger: t[4] || dmc_started,
                sample: dmc_started.then_some((self.dmc.start, self.dmc.len)),
            },
        ]
    }
}

/// The mixer (the NESdev wiki's APU Mixer formulas): channel levels in,
/// a level from 0.0 to about 1.0 out.
pub fn mix(p1: f64, p2: f64, t: f64, n: f64, d: f64) -> f64 {
    let pulse = if p1 + p2 > 0.0 {
        95.88 / (8128.0 / (p1 + p2) + 100.0)
    } else {
        0.0
    };
    let x = t / 8227.0 + n / 12241.0 + d / 22638.0;
    let tnd = if x > 0.0 {
        159.79 / (1.0 / x + 100.0)
    } else {
        0.0
    };
    pulse + tnd
}

/// A pulse or triangle timer period's pitch as a MIDI note number with a
/// fraction (69.0 is A440). `None` for a period that makes no tone.
pub fn pitch(channel: Channel, period: u16, region: Region) -> Option<f64> {
    let div = match channel {
        Channel::Pulse1 | Channel::Pulse2 if period >= 8 => 16.0,
        Channel::Triangle if period >= 2 => 32.0,
        _ => return None,
    };
    let f = region.cpu_hz() as f64 / (div * (f64::from(period) + 1.0));
    Some(69.0 + 12.0 * (f / 440.0).log2())
}

/// The timer period nearest a MIDI note on a pulse or triangle channel.
pub fn period_of(channel: Channel, note: f64, region: Region) -> u16 {
    let div = if channel == Channel::Triangle {
        32.0
    } else {
        16.0
    };
    let f = 440.0 * 2f64.powf((note - 69.0) / 12.0);
    let p = region.cpu_hz() as f64 / (div * f) - 1.0;
    p.round().clamp(0.0, 2047.0) as u16
}
