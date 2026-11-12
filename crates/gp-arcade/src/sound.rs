// license:BSD-3-Clause
// copyright-holders:Couriersud, Aaron Giles
//
// Tapper's sound as MAME 0.276 makes it: two AY-8910s on the Super Sound
// I/O board. Ported from src/devices/sound/ay8910.cpp and ay8910.h
// (copyright-holders Couriersud: the register file, the tone, noise and
// envelope generators, the legacy output tables), src/mame/midway/
// midway.cpp (copyright-holders Aaron Giles: the SSIO's duty-cycle
// volumes from its 82S123 PROM, the mute bit, the mixing), and the parts
// of src/emu/sound.cpp and speaker.cpp (copyright-holders Aaron Giles)
// that decide what reaches the speakers: the 50 Hz stream updates, the
// default resampler from the AYs' 250 kHz to 48 kHz, and the final 16-bit
// mix. All BSD-3-Clause.

//! The sound: what the AYs' registers say, sample by sample, and what
//! reaches the two speakers (the left is AY 0, the right AY 1).
//!
//! # How it follows MAME
//!
//! - Each AY makes one sample per 8 clocks of its 2 MHz: 250,000 a
//!   second. A register write lands at the first sample at or after its
//!   time (MAME updates the stream to the write's time, rounding the end
//!   up), so the time of every write inside a frame matters, and the board
//!   gives it.
//! - The samples are made in the sound manager's 50 Hz updates, as MAME
//!   makes them: each update resamples 20 ms of each AY channel to 48 kHz
//!   with MAME's default resampler (it sums each output period's energy,
//!   24 us late) and applies the channel's gain as it stands at the update.
//!   The SSIO's volume latches and mute therefore act on whole 20 ms
//!   pieces, as in MAME.
//! - A channel's gain is the SSIO's duty-cycle volume (from the PROM, a
//!   whole percent), or 0 when muted; then 0.33 into the board's mixer;
//!   then the speaker. The 16-bit sample is the float times 32767,
//!   truncated, as MAME writes its WAV.
//!
//! None of it changes RAM or the picture. The generators' state is part
//! of the snapshot, so a restored machine sounds the same.
//!
//! # Quirks a model can switch
//!
//! These are the board's (or MAME's) ways that a model's sound would copy
//! in its Retro profile and may leave in Normal:
//!
//! - `ay_dc_offset`: MAME's legacy AY output table puts silence at -0.125,
//!   not 0, so every channel sits at a DC offset that changes with the
//!   volume.
//! - `ssio_duty_volume`: the SSIO sets each voice's loudness by gating it
//!   with a duty cycle read from its PROM, in 16 coarse steps, over the
//!   AY's own volume.
//! - `ssio_gain_steps`: the duty-cycle volumes and the mute take effect a
//!   20 ms piece at a time (MAME's stream updates), not at the write.
//! - `ay_stereo_split`: each AY has a speaker of its own, so the music and
//!   effects are split left and right by chip.

use crate::attotime::{ATTOSECONDS_PER_SECOND, Attotime};
use crate::state::{Reader, Writer};

/// Samples a second each AY makes: its 2 MHz clock over 8.
pub const AY_RATE: i64 = 250_000;
/// Samples a second at the speakers: MAME's default.
pub const OUT_RATE: u32 = 48_000;
const AY_ATTOS: i64 = ATTOSECONDS_PER_SECOND / AY_RATE;
const OUT_ATTOS: i64 = ATTOSECONDS_PER_SECOND / OUT_RATE as i64;
/// The 50 Hz stream update's period.
pub const UPDATE_HZ: u32 = 50;
/// Raw samples kept from one update for the next one's resampler.
const TAIL: usize = 16;

/// AY register numbers.
const AY_ENABLE: usize = 7;
const AY_EASHAPE: usize = 13;
const AY_PORTA: usize = 14;
const AY_PORTB: usize = 15;

/// The AY-8910's register file as the sound CPU sees it: what it reads
/// back, and the port writes the SSIO turns into volumes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ay8910 {
    pub active: bool,
    pub latch: u8,
    pub regs: [u8; 16],
    pub last_enable: u8,
}

/// A write to one of an AY's I/O ports: port A (0) or B (1), and the value.
pub type PortWrite = (usize, u8);

impl Ay8910 {
    /// ay8910_reset_ym, on the register side: every register from 0 to 13
    /// written with 0, which writes 0xff to both ports (enable goes from
    /// 0xc0 to 0).
    pub fn reset(&mut self) -> Vec<PortWrite> {
        self.active = false;
        self.latch = 0;
        self.last_enable = 0xc0;
        let mut ports = Vec::new();
        for r in 0..AY_PORTA {
            ports.extend(self.write_reg(r, 0));
        }
        ports
    }

    /// ay8910_write_reg, as far as the bus sees it: the register, and the
    /// port writes it makes.
    fn write_reg(&mut self, r: usize, v: u8) -> Vec<PortWrite> {
        self.regs[r] = v;
        let mut ports = Vec::new();
        match r {
            AY_ENABLE => {
                let enable = v & 0x40;
                if enable != self.last_enable & 0x40 {
                    ports.push((
                        0,
                        if enable != 0 {
                            self.regs[AY_PORTA]
                        } else {
                            0xff
                        },
                    ));
                }
                let enable = v & 0x80;
                if enable != self.last_enable & 0x80 {
                    ports.push((
                        1,
                        if enable != 0 {
                            self.regs[AY_PORTB]
                        } else {
                            0xff
                        },
                    ));
                }
                self.last_enable = v;
            }
            AY_PORTA if self.regs[AY_ENABLE] & 0x40 != 0 => ports.push((0, v)),
            AY_PORTB if self.regs[AY_ENABLE] & 0x80 != 0 => ports.push((1, v)),
            _ => {}
        }
        ports
    }

    pub fn address_w(&mut self, data: u8) {
        self.active = data >> 4 == 0;
        if self.active {
            self.latch = data & 0x0f;
        }
    }

    /// A data write: the register written, if the chip is selected, and
    /// the port writes it makes.
    pub fn data_w(&mut self, data: u8) -> Option<(usize, Vec<PortWrite>)> {
        if !self.active {
            return None;
        }
        let r = usize::from(self.latch);
        Some((r, self.write_reg(r, data)))
    }

    pub fn data_r(&self) -> u8 {
        if !self.active {
            return 0xff;
        }
        const MASK: [u8; 16] = [
            0xff, 0x0f, 0xff, 0x0f, 0xff, 0x0f, 0x1f, 0xff, 0x1f, 0x1f, 0x1f, 0xff, 0xff, 0x0f,
            0xff, 0xff,
        ];
        let r = usize::from(self.latch);
        self.regs[r] & MASK[r]
    }
}

/// ay8910.cpp's ay8910_param: the AY-8910's output resistances (Matthew
/// Westcott's measurements, as MAME models them).
const R_UP: f64 = 800_000.0;
const R_DOWN: f64 = 8_000_000.0;
const RES: [f64; 16] = [
    15950.0, 15350.0, 15090.0, 14760.0, 14275.0, 13620.0, 12890.0, 11370.0, 10600.0, 8590.0,
    7190.0, 5985.0, 4820.0, 3945.0, 3017.0, 2345.0,
];
/// The load on each output, MAME's default.
const R_LOAD: f64 = 1000.0;

/// build_single_table with AY8910_LEGACY_OUTPUT (normalized): the output
/// for each of the 16 levels.
fn level_table(zero_is_off: bool) -> [f32; 16] {
    let mut temp = [0f64; 16];
    let (mut min, mut max) = (10.0f64, 0.0f64);
    for (j, t) in temp.iter_mut().enumerate() {
        let mut rt = 1.0 / R_DOWN + 1.0 / R_LOAD;
        let mut rw = 1.0 / RES[j];
        rt += 1.0 / RES[j];
        if !(zero_is_off && j == 0) {
            rw += 1.0 / R_UP;
            rt += 1.0 / R_UP;
        }
        *t = rw / rt;
        min = min.min(*t);
        max = max.max(*t);
    }
    temp.map(|t| ((((t - min) / (max - min)) - 0.25) * 0.5) as f32)
}

fn tables() -> &'static ([f32; 16], [f32; 16]) {
    static T: std::sync::OnceLock<([f32; 16], [f32; 16])> = std::sync::OnceLock::new();
    T.get_or_init(|| (level_table(true), level_table(false)))
}

/// One tone generator (ay8910.h's tone_t).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tone {
    pub period: u32,
    pub volume: u8,
    pub count: i32,
    pub duty_cycle: u8,
    pub output: u8,
}

/// The envelope generator (ay8910.h's envelope_t). The AY-8910 has one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Envelope {
    pub period: u32,
    pub count: u32,
    pub step: i8,
    pub volume: u32,
    pub hold: u8,
    pub alternate: u8,
    pub attack: u8,
    pub holding: u8,
}

/// The AY-8910 for an AY (not a YM): 16 envelope steps, each held for two
/// samples.
const ENV_MASK: u8 = 0x0f;
const ENV_STEP: u32 = 2;

impl Envelope {
    fn set_shape(&mut self, shape: u8) {
        self.attack = if shape & 0x04 != 0 { ENV_MASK } else { 0 };
        if shape & 0x08 == 0 {
            self.hold = 1;
            self.alternate = self.attack;
        } else {
            self.hold = shape & 0x01;
            self.alternate = shape & 0x02;
        }
        self.step = ENV_MASK as i8;
        self.holding = 0;
        self.volume = u32::from((self.step as u8) ^ self.attack);
    }
}

/// An AY's sound generators, with the registers as they stood at the last
/// sample made (the bus's file runs ahead of it within an update).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AyGen {
    pub regs: [u8; 16],
    pub tone: [Tone; 3],
    pub env: Envelope,
    pub prescale_noise: u8,
    pub count_noise: i16,
    pub rng: u32,
}

impl AyGen {
    /// ay8910_reset_ym, on the generators' side.
    fn reset(&mut self) {
        self.rng = 1;
        self.tone = [Tone::default(); 3];
        self.env = Envelope::default();
        self.count_noise = 0;
        self.prescale_noise = 0;
        for r in 0..AY_PORTA {
            self.write_reg(r, 0);
        }
    }

    fn write_reg(&mut self, r: usize, v: u8) {
        self.regs[r] = v;
        let period = |fine: u8, coarse: u8| u32::from(fine) | (u32::from(coarse & 0x0f) << 8);
        match r {
            0 | 1 => self.tone[0].period = period(self.regs[0], self.regs[1]),
            2 | 3 => self.tone[1].period = period(self.regs[2], self.regs[3]),
            4 | 5 => self.tone[2].period = period(self.regs[4], self.regs[5]),
            8..=10 => self.tone[r - 8].volume = v,
            11 | 12 => self.env.period = u32::from(self.regs[11]) | (u32::from(self.regs[12]) << 8),
            AY_EASHAPE => self.env.set_shape(v),
            _ => {}
        }
    }

    /// One sample of each channel (sound_stream_update's loop body, for an
    /// AY-8910 with three outputs).
    fn sample(&mut self) -> [f32; 3] {
        for t in &mut self.tone {
            let period = t.period.max(1) as i32;
            t.count += 1;
            while t.count >= period {
                t.duty_cycle = t.duty_cycle.wrapping_sub(1) & 0x1f;
                t.output = t.duty_cycle & 1;
                t.count -= period;
            }
        }
        self.count_noise += 1;
        if self.count_noise >= i16::from(self.regs[6] & 0x1f) {
            self.count_noise = 0;
            self.prescale_noise ^= 1;
            if self.prescale_noise == 0 {
                self.rng = (self.rng >> 1) | (((self.rng & 1) ^ ((self.rng >> 3) & 1)) << 16);
            }
        }
        let enable = self.regs[AY_ENABLE];
        let noise = (self.rng & 1) as u8;
        let on =
            |c: usize, out: u8| (out | (enable >> c) & 1) & (noise | (enable >> (3 + c)) & 1) != 0;
        let e = &mut self.env;
        if e.holding == 0 {
            e.count += 1;
            if e.count >= e.period * ENV_STEP {
                e.count = 0;
                e.step -= 1;
                if e.step < 0 {
                    if e.hold != 0 {
                        if e.alternate != 0 {
                            e.attack ^= ENV_MASK;
                        }
                        e.holding = 1;
                        e.step = 0;
                    } else {
                        if e.alternate != 0 && (e.step as u8) & (ENV_MASK + 1) != 0 {
                            e.attack ^= ENV_MASK;
                        }
                        e.step = ((e.step as u8) & ENV_MASK) as i8;
                    }
                }
            }
        }
        e.volume = u32::from((e.step as u8) ^ e.attack);
        let (vol, env) = tables();
        let mut out = [0f32; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let t = self.tone[c];
            let enabled = on(c, t.output);
            *o = if (t.volume >> 4) & 1 != 0 {
                env[if enabled { self.env.volume as usize } else { 0 }]
            } else {
                vol[if enabled {
                    usize::from(t.volume & 0x0f)
                } else {
                    0
                }]
            };
        }
        out
    }
}

/// The SSIO's volume table from its 82S123 PROM
/// (compute_ay8910_modulation): each duty-cycle value's loudness, as a
/// whole percent.
pub fn duty_volumes(prom: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for volval in 0..16usize {
        let mut remaining = volval;
        let mut prev = 1u8;
        let mut clock = 0usize;
        while clock < 160 && remaining > 0 {
            let cur = prom.get(clock / 8).copied().unwrap_or(0) & (0x80 >> (clock % 8));
            if cur == 0 && prev != 0 {
                remaining -= 1;
            }
            prev = cur;
            clock += 1;
        }
        out[15 - volval] = (clock * 100 / 160) as u8;
    }
    out
}

/// A [`Pending`] register that resets the chip's generators instead.
pub const RESET_REG: u8 = 0xff;

/// A register write waiting for the generators: the sample it lands at,
/// the chip, the register and the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub at: u64,
    pub chip: u8,
    pub reg: u8,
    pub value: u8,
}

/// The two AYs, the SSIO's volumes and the path to the speakers.
#[derive(Clone)]
pub struct Sound {
    pub bus: [Ay8910; 2],
    pub gens: [AyGen; 2],
    /// Raw samples made so far by each AY, from power-on.
    pub made: u64,
    pub pending: Vec<Pending>,
    /// The last [`TAIL`] raw samples made, oldest first: each chip's three
    /// channels.
    pub tail: Vec<[[f32; 3]; 2]>,
    /// The end of the last stream update.
    pub last_update: Attotime,
    /// The next 48 kHz sample to make, as (second, index in the second).
    pub next_out: (i32, i64),
    /// The last speaker sample made, which reaches the speakers one sample
    /// late ([`Sound::update`]).
    pub held: [f32; 2],
    /// The SSIO's latches from the AYs' ports: each voice's duty cycle,
    /// the overall volume (unused by the sound, kept as MAME keeps it), and
    /// the mute bit.
    pub duty: [[u8; 3]; 2],
    pub overall: [u8; 2],
    pub mute: u8,
    /// From the PROM; not state.
    pub volumes: [u8; 16],
    /// Whether the PROM was there: without it every volume is 0 and the
    /// board is silent (MAME wouldn't run the set at all).
    pub have_prom: bool,
    /// When on, the speakers' samples made since it was last taken. Not
    /// state.
    pub out: Option<Vec<[f32; 2]>>,
    /// The registers written since it was last cleared: chip, register,
    /// value. Not state.
    pub written: Vec<(u8, u8, u8)>,
    /// Set by a reset until the update after it: the volumes before the
    /// reset and the speaker sample they last until.
    pub split: Option<GainSplit>,
}

/// MAME brings a sound device's streams up to the moment of a reset before
/// resetting it (device_sound_interface::interface_pre_reset), so the
/// speaker samples before the reset are made with the volumes as they were;
/// the rest of that 20 ms piece gets the volumes after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GainSplit {
    /// The first speaker sample with the new volumes, as (second, index in
    /// the second). MAME's stream update to the reset's time ends at the
    /// sample index it rounds down to (time_to_buffer_index without
    /// rounding up), so this is that index.
    pub at: (i32, i64),
    pub duty: [[u8; 3]; 2],
    pub mute: u8,
}

impl Sound {
    /// The board at power-on: both AYs reset at time zero.
    pub fn new(prom: Option<&[u8]>) -> Self {
        let mut s = Self {
            bus: [Ay8910::default(), Ay8910::default()],
            gens: [AyGen::default(), AyGen::default()],
            made: 0,
            pending: Vec::new(),
            tail: Vec::new(),
            last_update: Attotime::ZERO,
            next_out: (0, 0),
            held: [0.0; 2],
            duty: [[0; 3]; 2],
            overall: [0; 2],
            mute: 0,
            volumes: prom.map(duty_volumes).unwrap_or([0; 16]),
            have_prom: prom.is_some(),
            out: None,
            written: Vec::new(),
            split: None,
        };
        for chip in 0..2 {
            s.gens[chip].reset();
            let ports = s.bus[chip].reset();
            for p in ports {
                s.port_write(chip, p);
            }
        }
        s
    }

    /// Both AYs' device_reset (ay8910_reset_ym) at a soft reset, at `now`.
    /// MAME first brings the streams up to `now` ([`GainSplit`]), so the
    /// generators take the reset at the first sample at or after it, as a
    /// write would, and the speakers keep the old volumes up to it. The
    /// registers and the ports (so the volumes and the mute) change at once.
    pub fn reset(&mut self, now: Attotime) {
        let at = ay_index_up(now);
        if self.split.is_none() {
            self.split = Some(GainSplit {
                at: {
                    let i = now.attoseconds / OUT_ATTOS;
                    if i >= i64::from(OUT_RATE) {
                        (now.seconds + 1, i - i64::from(OUT_RATE))
                    } else {
                        (now.seconds, i)
                    }
                },
                duty: self.duty,
                mute: self.mute,
            });
        }
        for chip in 0..2 {
            let ports = self.bus[chip].reset();
            for p in ports {
                self.port_write(chip, p);
            }
            self.pending.push(Pending {
                at,
                chip: chip as u8,
                reg: RESET_REG,
                value: 0,
            });
        }
    }

    /// The SSIO's port handlers (porta0_w and the rest).
    fn port_write(&mut self, chip: usize, (port, data): PortWrite) {
        match port {
            0 => {
                self.duty[chip][0] = data & 15;
                self.duty[chip][1] = data >> 4;
            }
            _ => {
                self.duty[chip][2] = data & 15;
                self.overall[chip] = (data >> 4) & 7;
                if chip == 1 {
                    self.mute = data & 0x80;
                }
            }
        }
    }

    /// A channel's gain as update_volumes sets it: the duty cycle's
    /// volume over 100, or nothing when muted.
    fn gain(&self, chip: usize, chan: usize) -> f32 {
        self.gain_of(&self.duty, self.mute, chip, chan)
    }

    fn gain_of(&self, duty: &[[u8; 3]; 2], mute: u8, chip: usize, chan: usize) -> f32 {
        if mute != 0 {
            0.0
        } else {
            (f64::from(self.volumes[usize::from(duty[chip][chan])]) / 100.0) as f32
        }
    }

    /// A write from the sound CPU at `now` to chip `chip`'s address (port
    /// 0) or data (port 2) latch.
    pub fn write(&mut self, chip: usize, port: u8, data: u8, now: Attotime) {
        if port == 0 {
            self.bus[chip].address_w(data);
            return;
        }
        let Some((reg, ports)) = self.bus[chip].data_w(data) else {
            return;
        };
        self.written.push((chip as u8, reg as u8, data));
        self.pending.push(Pending {
            at: ay_index_up(now),
            chip: chip as u8,
            reg: reg as u8,
            value: data,
        });
        for p in ports {
            self.port_write(chip, p);
        }
    }

    /// Makes raw samples up to (not including) `end`, the registers
    /// written landing at their samples.
    fn make_until(&mut self, end: u64) -> Vec<[[f32; 3]; 2]> {
        let mut out = Vec::with_capacity(end.saturating_sub(self.made) as usize);
        let mut k = 0;
        while self.made < end {
            while let Some(p) = self.pending.get(k).filter(|p| p.at <= self.made) {
                let g = &mut self.gens[usize::from(p.chip)];
                if p.reg == RESET_REG {
                    g.reset();
                } else {
                    g.write_reg(usize::from(p.reg), p.value);
                }
                k += 1;
            }
            out.push([self.gens[0].sample(), self.gens[1].sample()]);
            self.made += 1;
        }
        // Writes that land at `end` or later wait for the next update.
        self.pending.drain(..k);
        out
    }

    /// sound_manager::update at `now`: 20 ms of samples made and resampled
    /// to the speakers, with the gains as they stand.
    pub fn update(&mut self, now: Attotime) {
        let period = (now - self.last_update).as_attoseconds();
        let n = period / OUT_ATTOS;
        let endtime = self.last_update + Attotime::new(0, n * OUT_ATTOS);
        let first = self.made;
        let raw = self.make_until(ay_index_up(endtime));
        // The raw samples from `base` on: the tail kept, then the new.
        let base = first - self.tail.len() as u64;
        let mut all = std::mem::take(&mut self.tail);
        all.extend(raw);

        let mut speakers = vec![[0f32; 2]; 0];
        let (sec, idx) = self.next_out;
        let start = Attotime::new(sec, idx * OUT_ATTOS);
        let end_idx = out_index_up(endtime);
        let count = (end_idx.0 - sec) as i64 * i64::from(OUT_RATE) + end_idx.1 - idx;
        if count > 0 {
            let gains: [[f32; 3]; 2] =
                std::array::from_fn(|c| std::array::from_fn(|ch| self.gain(c, ch)));
            let count = count as usize;
            match self.split.take() {
                Some(split) => {
                    // The samples before the reset with the volumes before it.
                    let to = split.at;
                    let k = (i64::from(to.0 - sec) * i64::from(OUT_RATE) + to.1 - idx)
                        .clamp(0, count as i64) as usize;
                    let old: [[f32; 3]; 2] = std::array::from_fn(|c| {
                        std::array::from_fn(|ch| self.gain_of(&split.duty, split.mute, c, ch))
                    });
                    speakers = resample(&all, base, start, k, &old);
                    let (s2, i2) = (
                        sec + ((idx + k as i64) / i64::from(OUT_RATE)) as i32,
                        (idx + k as i64) % i64::from(OUT_RATE),
                    );
                    let start2 = Attotime::new(s2, i2 * OUT_ATTOS);
                    speakers.extend(resample(&all, base, start2, count - k, &gains));
                    if (to.0, to.1) > end_idx {
                        self.split = Some(split);
                    }
                }
                None => speakers = resample(&all, base, start, count, &gains),
            }
        }
        // MAME's speakers get each sample one 48 kHz sample after the
        // resampler makes it: found by comparing with its WAV, where every
        // change of volume lands one sample later than the resampler puts
        // it. So each piece starts with the last sample of the one before.
        if let Some(last) = speakers.last().copied() {
            speakers.rotate_right(1);
            speakers[0] = std::mem::replace(&mut self.held, last);
        }
        self.next_out = end_idx;
        self.last_update = endtime;
        let keep = all.len().saturating_sub(TAIL);
        self.tail = all.split_off(keep);
        if let Some(out) = &mut self.out {
            out.extend(speakers);
        }
    }

    pub fn save(&self, w: &mut Writer) {
        for ay in &self.bus {
            w.bool(ay.active);
            w.u8(ay.latch);
            w.bytes(&ay.regs);
            w.u8(ay.last_enable);
        }
        for g in &self.gens {
            w.bytes(&g.regs);
            for t in &g.tone {
                w.u32(t.period);
                w.u8(t.volume);
                w.i32(t.count);
                w.u8(t.duty_cycle);
                w.u8(t.output);
            }
            let e = &g.env;
            w.u32(e.period);
            w.u32(e.count);
            w.u8(e.step as u8);
            w.u32(e.volume);
            w.bytes(&[e.hold, e.alternate, e.attack, e.holding]);
            w.u8(g.prescale_noise);
            w.i32(i32::from(g.count_noise));
            w.u32(g.rng);
        }
        w.u64(self.made);
        w.u32(self.pending.len() as u32);
        for p in &self.pending {
            w.u64(p.at);
            w.bytes(&[p.chip, p.reg, p.value]);
        }
        w.u32(self.tail.len() as u32);
        for s in &self.tail {
            for v in s.iter().flatten() {
                w.u32(v.to_bits());
            }
        }
        w.time(self.last_update);
        w.i32(self.next_out.0);
        w.i64(self.next_out.1);
        w.u32(self.held[0].to_bits());
        w.u32(self.held[1].to_bits());
        for d in &self.duty {
            w.bytes(d);
        }
        w.bytes(&self.overall);
        w.u8(self.mute);
    }

    pub fn load(&mut self, r: &mut Reader) -> Result<(), String> {
        for ay in &mut self.bus {
            ay.active = r.bool()?;
            ay.latch = r.u8()?;
            r.fill(&mut ay.regs)?;
            ay.last_enable = r.u8()?;
        }
        for g in &mut self.gens {
            r.fill(&mut g.regs)?;
            for t in &mut g.tone {
                t.period = r.u32()?;
                t.volume = r.u8()?;
                t.count = r.i32()?;
                t.duty_cycle = r.u8()?;
                t.output = r.u8()?;
            }
            let e = &mut g.env;
            e.period = r.u32()?;
            e.count = r.u32()?;
            e.step = r.u8()? as i8;
            e.volume = r.u32()?;
            let mut f = [0u8; 4];
            r.fill(&mut f)?;
            [e.hold, e.alternate, e.attack, e.holding] = f;
            g.prescale_noise = r.u8()?;
            g.count_noise = i16::try_from(r.i32()?).map_err(|e| e.to_string())?;
            g.rng = r.u32()?;
        }
        self.made = r.u64()?;
        let n = r.u32()? as usize;
        self.pending.clear();
        for _ in 0..n {
            let at = r.u64()?;
            let mut b = [0u8; 3];
            r.fill(&mut b)?;
            self.pending.push(Pending {
                at,
                chip: b[0],
                reg: b[1],
                value: b[2],
            });
        }
        let n = r.u32()? as usize;
        self.tail.clear();
        for _ in 0..n {
            let mut s = [[0f32; 3]; 2];
            for v in s.iter_mut().flatten() {
                *v = f32::from_bits(r.u32()?);
            }
            self.tail.push(s);
        }
        self.last_update = r.time()?;
        self.next_out = (r.i32()?, r.i64()?);
        self.held = [f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)];
        for d in &mut self.duty {
            r.fill(d)?;
        }
        r.fill(&mut self.overall)?;
        self.mute = r.u8()?;
        Ok(())
    }
}

/// The first AY sample at or after `t`: a stream updated to `t` has made
/// every sample before it (time_to_buffer_index, rounding up).
fn ay_index_up(t: Attotime) -> u64 {
    let within = (t.attoseconds + AY_ATTOS - 1) / AY_ATTOS;
    t.seconds.max(0) as u64 * AY_RATE as u64 + within as u64
}

/// The 48 kHz sample at or after `t`, as (second, index in the second).
/// MAME counts a stream's samples within each second, so the last sample
/// of a second is a little longer than the rest.
fn out_index_up(t: Attotime) -> (i32, i64) {
    let i = (t.attoseconds + OUT_ATTOS - 1) / OUT_ATTOS;
    if i >= i64::from(OUT_RATE) {
        (t.seconds + 1, i - i64::from(OUT_RATE))
    } else {
        (t.seconds, i)
    }
}

/// default_resampler_stream::resampler_sound_update for each of the six
/// channels from `start`, `count` output samples, the input being `raw`
/// (raw sample `base` first) times each channel's gain; then each chip's
/// three channels summed at 0.33 into its speaker.
fn resample(
    raw: &[[[f32; 3]; 2]],
    base: u64,
    start: Attotime,
    count: usize,
    gains: &[[f32; 3]; 2],
) -> Vec<[f32; 2]> {
    let step = AY_RATE as f32 / OUT_RATE as f32;
    let stepinv = (1.0 / f64::from(step)) as f32;
    let latency_samples = 1 + step as i64;
    let latency = Attotime::new(0, latency_samples * AY_ATTOS);
    let mut out = vec![[0f32; 2]; count];
    // Before the latency has passed, the resampler gives silence.
    let mut first = 0;
    let mut out_start = start;
    while latency > out_start && first < count {
        first += 1;
        out_start += Attotime::new(0, OUT_ATTOS);
    }
    if first >= count {
        return out;
    }
    let from = out_start - latency;
    let rebased = from.seconds as u64 * AY_RATE as u64 + (from.attoseconds / AY_ATTOS) as u64;
    let rebased_time = Attotime::new(
        (rebased / AY_RATE as u64) as i32,
        (rebased % AY_RATE as u64) as i64 * AY_ATTOS,
    );
    let delta = out_start - (rebased_time + latency);
    let srcpos0 = (delta.attoseconds as f64 / AY_ATTOS as f64) as f32;
    let get = |i: u64, chip: usize, ch: usize| -> f32 {
        let at = i.checked_sub(base).map(|k| k as usize);
        at.and_then(|k| raw.get(k))
            .map_or(0.0, |s| s[chip][ch] * gains[chip][ch])
    };
    for chip in 0..2 {
        for ch in 0..3 {
            let mut srcpos = srcpos0;
            let mut src = rebased;
            let mut cur = get(src, chip, ch);
            src += 1;
            for o in out.iter_mut().skip(first) {
                let scale = 1.0 - srcpos;
                let mut sample = cur * scale;
                let mut remaining = step - scale;
                while remaining >= 1.0 {
                    sample += get(src, chip, ch);
                    src += 1;
                    remaining -= 1.0;
                }
                cur = get(src, chip, ch);
                src += 1;
                sample += cur * remaining;
                o[chip] += sample * stepinv * 0.33;
                srcpos = remaining;
            }
        }
    }
    out
}

/// A speaker sample as MAME writes it to a WAV: clamped, times 32767,
/// truncated.
pub fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ay_reads_back_masked_registers() {
        let mut ay = Ay8910::default();
        ay.reset();
        assert_eq!(ay.data_r(), 0xff); // no register selected yet
        ay.address_w(1);
        ay.data_w(0xff);
        assert_eq!(ay.data_r(), 0x0f);
        ay.address_w(0x21); // a wrong upper nibble deselects the chip
        assert_eq!(ay.data_r(), 0xff);
    }

    /// The legacy tables run from -0.125 to 0.375, silence at the bottom.
    #[test]
    fn the_level_tables_are_mames_legacy_ones() {
        let (vol, env) = tables();
        assert_eq!(vol[0], -0.125);
        assert_eq!(vol[15], 0.375);
        assert!(vol.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(env[15], 0.375);
    }

    /// A made-up PROM whose bits are all set gives no high-to-low edges,
    /// so every duty value but 15 counts all 160 clocks: full volume.
    #[test]
    fn the_duty_volumes_count_prom_edges() {
        let v = duty_volumes(&[0xff; 32]);
        assert_eq!(v[15], 0);
        assert_eq!(v[0], 100);
        // Alternating bits: an edge every two clocks.
        let v = duty_volumes(&[0xaa; 32]);
        assert_eq!(v[14], 1); // one edge: clock 2 of 160
        assert_eq!(v[0], (30 * 100 / 160) as u8);
    }

    /// At reset the AYs write 0xff to both ports: every voice at the
    /// lowest duty volume and the board muted, as MAME starts.
    #[test]
    fn reset_mutes_the_board() {
        let s = Sound::new(Some(&[0xaa; 32]));
        assert_eq!(s.mute, 0x80);
        assert_eq!(s.duty, [[15; 3]; 2]);
        assert_eq!(s.gain(0, 0), 0.0);
    }

    /// A tone at period 1 flips every sample; a square wave at full volume
    /// alternates between the table's ends.
    #[test]
    fn a_tone_toggles_at_its_period() {
        let mut g = AyGen::default();
        g.reset();
        g.write_reg(0, 2);
        g.write_reg(AY_ENABLE, 0b0011_1110); // tone A on, noise off
        g.write_reg(8, 15);
        let s: Vec<f32> = (0..8).map(|_| g.sample()[0]).collect();
        assert_eq!(
            s,
            [-0.125, 0.375, 0.375, -0.125, -0.125, 0.375, 0.375, -0.125]
        );
    }

    /// Writes land at the first sample at or after their time, and the
    /// sound's state comes back from a snapshot.
    #[test]
    fn writes_land_at_their_sample_and_the_state_restores() {
        let mut s = Sound::new(Some(&[0xaa; 32]));
        let t = |us: i64| Attotime::new(0, us * 1_000_000_000_000);
        s.write(0, 0, 8, t(100));
        s.write(0, 2, 15, t(101));
        assert_eq!(s.pending[0].at, 26); // 101 us is 25.25 samples
        s.out = Some(Vec::new());
        s.update(Attotime::from_hz(UPDATE_HZ));
        assert_eq!(s.out.as_ref().unwrap().len(), 960);
        let mut w = Writer::default();
        s.save(&mut w);
        let mut back = Sound::new(Some(&[0xaa; 32]));
        back.load(&mut Reader::new(&w.out)).unwrap();
        let mut w2 = Writer::default();
        back.save(&mut w2);
        assert_eq!(w.out, w2.out);
        assert_eq!(back.gens[0].tone[0].volume, 15);
    }
}
