// license:BSD-3-Clause
// copyright-holders:Wilbert Pol
//
// A port of MAME 0.276's Z80 CTC (src/devices/machine/z80ctc.cpp, based on
// the original version (c) 1997, Tatsuyuki Satoh) and the parts of its
// daisy chain interface (src/devices/machine/z80daisy.cpp, copyright-holders
// Aaron Giles) the MCR boards use. Channel 0's zero count drives channel
// 1's trigger, as mcr.cpp wires it.

//! The Z80 CTC: four counter/timer channels and their interrupts.

use crate::attotime::Attotime;
use crate::irq::IrqLine;
use crate::sched::Scheduler;

const INTERRUPT: u16 = 0x80;
const INTERRUPT_ON: u16 = 0x80;
const INTERRUPT_OFF: u16 = 0x00;
const MODE: u16 = 0x40;
const MODE_TIMER: u16 = 0x00;
const MODE_COUNTER: u16 = 0x40;
const PRESCALER: u16 = 0x20;
const PRESCALER_16: u16 = 0x00;
const EDGE: u16 = 0x10;
const EDGE_FALLING: u16 = 0x00;
const EDGE_RISING: u16 = 0x10;
const TRIGGER: u16 = 0x08;
const TRIGGER_AUTO: u16 = 0x00;
const CONSTANT: u16 = 0x04;
const CONSTANT_LOAD: u16 = 0x04;
const RESET: u16 = 0x02;
const RESET_ACTIVE: u16 = 0x02;
const CONTROL: u16 = 0x01;
const CONTROL_VECTOR: u16 = 0x00;
const CONTROL_WORD: u16 = 0x01;
const WAITING_FOR_TRIG: u16 = 0x100;

pub const DAISY_INT: u8 = 0x01;
pub const DAISY_IEO: u8 = 0x02;

/// What a CTC timer does when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtcTimer {
    /// A channel's down counter reaching zero.
    Count(u8),
    /// The end of a channel's one-clock zero-count pulse.
    ZcEnd(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub mode: u16,
    pub tconst: u16,
    pub down: u16,
    pub extclk: bool,
    pub int_state: u8,
    pub timer: usize,
    pub zc_timer: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ctc {
    pub vector: u8,
    pub channels: [Channel; 4],
    /// The device's clock, which the timers count.
    pub attoseconds_per_clock: i64,
    pub clock: u32,
    /// The daisy chain's RETI decoder (device_z80daisy_interface).
    pub last_opcode: u8,
}

impl Ctc {
    /// device_start: allocates the channels' timers.
    pub fn new<T: Copy + Eq>(
        clock: u32,
        sched: &mut Scheduler<T>,
        wrap: impl Fn(CtcTimer) -> T,
    ) -> Self {
        let channels = std::array::from_fn(|i| Channel {
            mode: 0,
            tconst: 0,
            down: 0,
            extclk: false,
            int_state: 0,
            timer: sched.alloc(wrap(CtcTimer::Count(i as u8))),
            zc_timer: sched.alloc(wrap(CtcTimer::ZcEnd(i as u8))),
        });
        Self {
            vector: 0,
            channels,
            attoseconds_per_clock: crate::attotime::hz_to_attoseconds(i64::from(clock)),
            clock,
            last_opcode: 0,
        }
    }

    /// The device's clocks_to_attotime.
    fn clocks(&self, n: u64) -> Attotime {
        if n < u64::from(self.clock) {
            Attotime::new(0, n as i64 * self.attoseconds_per_clock)
        } else {
            let q = n / u64::from(self.clock);
            let r = n % u64::from(self.clock);
            Attotime::new(q as i32, r as i64 * self.attoseconds_per_clock)
        }
    }

    /// Device reset: each channel's reset, then interrupt_check.
    pub fn reset<T: Copy + Eq>(&mut self, sched: &mut Scheduler<T>, irq: &mut IrqLine<T>) {
        for ch in 0..4 {
            let c = &mut self.channels[ch];
            c.mode = RESET_ACTIVE;
            c.tconst = 0x100;
            c.int_state = 0;
            let t = c.timer;
            sched.adjust(t, Attotime::NEVER, 0, Attotime::NEVER);
        }
        self.interrupt_check(sched, irq);
    }

    /// device_z80daisy_interface::interface_post_reset.
    pub fn reset_daisy(&mut self) {
        self.last_opcode = 0;
    }

    pub fn irq_state(&self) -> u8 {
        let mut state = 0;
        for c in &self.channels {
            if c.int_state & DAISY_IEO != 0 {
                state |= DAISY_IEO;
                break;
            }
            state |= c.int_state;
        }
        state
    }

    pub fn interrupt_check<T: Copy + Eq>(&self, sched: &mut Scheduler<T>, irq: &mut IrqLine<T>) {
        let state = self.irq_state() & DAISY_INT != 0;
        irq.set(sched, state);
    }

    /// z80daisy_irq_ack.
    pub fn irq_ack<T: Copy + Eq>(&mut self, sched: &mut Scheduler<T>, irq: &mut IrqLine<T>) -> i32 {
        for ch in 0..4 {
            if self.channels[ch].int_state & DAISY_INT != 0 {
                self.channels[ch].int_state = DAISY_IEO;
                self.interrupt_check(sched, irq);
                return i32::from(self.vector) + ch as i32 * 2;
            }
        }
        i32::from(self.vector)
    }

    /// z80daisy_irq_reti.
    fn irq_reti<T: Copy + Eq>(&mut self, sched: &mut Scheduler<T>, irq: &mut IrqLine<T>) {
        for ch in 0..4 {
            if self.channels[ch].int_state & DAISY_IEO != 0 {
                self.channels[ch].int_state &= !DAISY_IEO;
                self.interrupt_check(sched, irq);
                return;
            }
        }
    }

    /// z80daisy_decode, fed ED 4D by the CPU's RETI.
    pub fn decode<T: Copy + Eq>(
        &mut self,
        opcode: u8,
        sched: &mut Scheduler<T>,
        irq: &mut IrqLine<T>,
    ) {
        match self.last_opcode {
            0xed => {
                if opcode == 0x4d {
                    self.irq_reti(sched, irq);
                }
                self.last_opcode = 0;
            }
            0xcb | 0xdd | 0xfd => self.last_opcode = 0,
            _ => self.last_opcode = opcode,
        }
    }

    fn period(&self, ch: usize) -> Attotime {
        let c = &self.channels[ch];
        if c.mode & RESET == RESET_ACTIVE {
            return Attotime::NEVER;
        }
        if c.mode & MODE == MODE_COUNTER {
            // The channel's own clock, which mcr.cpp leaves unset.
            return Attotime::NEVER;
        }
        let prescale = if c.mode & PRESCALER == PRESCALER_16 {
            16
        } else {
            256
        };
        self.clocks(prescale).times(u32::from(c.tconst))
    }

    pub fn read<T: Copy + Eq>(&self, ch: usize, sched: &Scheduler<T>) -> u8 {
        let c = &self.channels[ch];
        let t = sched.timer(c.timer);
        if !t.enabled || c.mode & WAITING_FOR_TRIG != 0 {
            return c.down as u8;
        }
        let period = if c.mode & MODE == MODE_COUNTER {
            Attotime::NEVER
        } else {
            self.clocks(if c.mode & PRESCALER == PRESCALER_16 {
                16
            } else {
                256
            })
        };
        let remaining = sched.remaining(c.timer);
        if !remaining.is_never() {
            let v = remaining.as_double() / period.as_double() + 1.0;
            // C++'s u8(double): through an int, then cut to 8 bits.
            (v as i64) as u8
        } else {
            c.down as u8
        }
    }

    pub fn write<T: Copy + Eq>(
        &mut self,
        ch: usize,
        data: u8,
        sched: &mut Scheduler<T>,
        irq: &mut IrqLine<T>,
    ) {
        let data16 = u16::from(data);
        if self.channels[ch].mode & CONSTANT == CONSTANT_LOAD {
            let c = &mut self.channels[ch];
            c.tconst = if data != 0 { data16 } else { 0x100 };
            c.mode &= !CONSTANT;
            c.mode &= !RESET;
            if c.mode & MODE == MODE_COUNTER || c.mode & TRIGGER == TRIGGER_AUTO {
                let p = self.period(ch);
                let t = self.channels[ch].timer;
                sched.adjust(t, p, 0, p);
            } else {
                c.mode |= WAITING_FOR_TRIG;
                let t = c.timer;
                // The channel's clocks_to_attotime(1): never, with no clock.
                sched.adjust(t, Attotime::NEVER, 0, Attotime::NEVER);
            }
            let c = &mut self.channels[ch];
            c.down = c.tconst;
        } else if data16 & CONTROL == CONTROL_VECTOR && ch == 0 {
            self.vector = data & 0xf8;
        } else if data16 & CONTROL == CONTROL_WORD {
            let t = self.channels[ch].timer;
            if self.channels[ch].mode & MODE == MODE_TIMER
                && data16 & MODE == MODE_COUNTER
                && data16 & RESET == 0
            {
                sched.adjust(t, Attotime::NEVER, 0, Attotime::NEVER);
            }
            if data16 & RESET == RESET_ACTIVE {
                let down = self.read(ch, sched);
                self.channels[ch].down = u16::from(down);
                sched.adjust(t, Attotime::NEVER, 0, Attotime::NEVER);
            }
            self.channels[ch].mode = data16;
            if data16 & INTERRUPT == INTERRUPT_OFF && self.channels[ch].int_state & DAISY_INT != 0 {
                self.channels[ch].int_state &= !DAISY_INT;
                self.interrupt_check(sched, irq);
            }
        }
    }

    /// A trigger input changing (trg0 to trg3).
    pub fn trigger<T: Copy + Eq>(
        &mut self,
        ch: usize,
        state: bool,
        sched: &mut Scheduler<T>,
        irq: &mut IrqLine<T>,
    ) {
        if state == self.channels[ch].extclk {
            return;
        }
        self.channels[ch].extclk = state;
        let mode = self.channels[ch].mode;
        if (mode & EDGE == EDGE_RISING && state) || (mode & EDGE == EDGE_FALLING && !state) {
            if mode & WAITING_FOR_TRIG != 0 && mode & MODE == MODE_TIMER {
                let p = self.period(ch);
                let t = self.channels[ch].timer;
                sched.adjust(t, p, 0, p);
            }
            self.channels[ch].mode &= !WAITING_FOR_TRIG;
            if self.channels[ch].mode & MODE == MODE_COUNTER {
                let c = &mut self.channels[ch];
                c.down = c.down.wrapping_sub(1);
                if c.down == 0 {
                    self.count_done(ch, sched, irq);
                }
            }
        }
    }

    /// The channel timer firing (timer_callback).
    pub fn count_done<T: Copy + Eq>(
        &mut self,
        ch: usize,
        sched: &mut Scheduler<T>,
        irq: &mut IrqLine<T>,
    ) {
        if self.channels[ch].mode & WAITING_FOR_TRIG != 0 {
            let p = self.period(ch);
            let t = self.channels[ch].timer;
            sched.adjust(t, p, 0, p);
            self.channels[ch].mode &= !WAITING_FOR_TRIG;
            return;
        }
        if self.channels[ch].mode & INTERRUPT == INTERRUPT_ON {
            self.channels[ch].int_state |= DAISY_INT;
            self.interrupt_check(sched, irq);
        }
        self.zero_count(ch, true, sched, irq);
        let zc = self.channels[ch].zc_timer;
        let one = self.clocks(1);
        sched.adjust(zc, one, 0, Attotime::NEVER);
        let c = &mut self.channels[ch];
        c.down = c.tconst;
    }

    /// zc_to_callback: the end of the zero-count pulse.
    pub fn zc_end<T: Copy + Eq>(
        &mut self,
        ch: usize,
        sched: &mut Scheduler<T>,
        irq: &mut IrqLine<T>,
    ) {
        self.zero_count(ch, false, sched, irq);
    }

    /// The ZC/TO outputs: mcr.cpp wires channel 0's to channel 1's trigger.
    fn zero_count<T: Copy + Eq>(
        &mut self,
        ch: usize,
        state: bool,
        sched: &mut Scheduler<T>,
        irq: &mut IrqLine<T>,
    ) {
        if ch == 0 {
            self.trigger(1, state, sched, irq);
        }
    }

    /// The timer fired: Count or ZcEnd.
    pub fn fire<T: Copy + Eq>(
        &mut self,
        what: CtcTimer,
        sched: &mut Scheduler<T>,
        irq: &mut IrqLine<T>,
    ) {
        match what {
            CtcTimer::Count(ch) => self.count_done(usize::from(ch), sched, irq),
            CtcTimer::ZcEnd(ch) => self.zc_end(usize::from(ch), sched, irq),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attotime::Attotime;
    use crate::sched::{Exec, Scheduler};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum What {
        Ctc(CtcTimer),
        Irq,
    }

    fn setup() -> (Ctc, Scheduler<What>, IrqLine<What>) {
        let mut sched = Scheduler::new(vec![Exec::new(5_000_000)], Attotime::from_hz(60));
        let mut ctc = Ctc::new(5_000_000, &mut sched, What::Ctc);
        let mut irq = IrqLine::new(What::Irq, 0xff);
        ctc.reset(&mut sched, &mut irq);
        // Fire the line change the reset queued at time zero.
        sched.basetime = Attotime::ZERO;
        while let Some((h, _, _, _)) = sched.pop_due() {
            sched.finish_due(h);
        }
        irq.clear_queue();
        (ctc, sched, irq)
    }

    /// Fires every timer due by `t`; returns how many CTC counts fired.
    fn run_to(
        t: Attotime,
        ctc: &mut Ctc,
        sched: &mut Scheduler<What>,
        irq: &mut IrqLine<What>,
    ) -> usize {
        sched.basetime = t;
        let mut counts = 0;
        while let Some((h, what, _, _)) = sched.pop_due() {
            if let What::Ctc(w) = what {
                if matches!(w, CtcTimer::Count(_)) {
                    counts += 1;
                }
                ctc.fire(w, sched, irq);
            }
            sched.finish_due(h);
        }
        counts
    }

    #[test]
    fn a_timer_channel_counts_the_prescaled_clock() {
        let (mut ctc, mut sched, mut irq) = setup();
        // Channel 3: interrupt on, timer mode, prescaler 16, automatic
        // trigger, time constant follows.
        ctc.write(3, 0x85, &mut sched, &mut irq);
        ctc.write(3, 10, &mut sched, &mut irq);
        // 16 * 10 clocks of 200 ns: 32 us.
        let period = Attotime::new(0, 32_000_000_000_000);
        assert_eq!(sched.next_expire(), period);
        assert_eq!(
            run_to(
                Attotime::new(0, 31_999_999_999_999),
                &mut ctc,
                &mut sched,
                &mut irq
            ),
            0
        );
        assert_eq!(run_to(period, &mut ctc, &mut sched, &mut irq), 1);
        assert_eq!(ctc.channels[3].int_state, DAISY_INT);
        assert_eq!(irq.queued(), vec![true]);
        // Halfway through the next period the counter reads 6: 5 left
        // plus one, as MAME rounds it.
        sched.basetime = period + Attotime::new(0, 16_000_000_000_000);
        assert_eq!(ctc.read(3, &sched), 6);
    }

    #[test]
    fn acknowledge_and_reti_pass_the_interrupt_along_the_chain() {
        let (mut ctc, mut sched, mut irq) = setup();
        ctc.write(0, 0x10, &mut sched, &mut irq); // vector 0x10
        ctc.write(2, 0xc5, &mut sched, &mut irq); // counter mode, interrupt on
        ctc.write(2, 2, &mut sched, &mut irq);
        ctc.trigger(2, true, &mut sched, &mut irq); // falling edge counts
        ctc.trigger(2, false, &mut sched, &mut irq);
        assert_eq!(ctc.channels[2].down, 1);
        ctc.trigger(2, true, &mut sched, &mut irq);
        ctc.trigger(2, false, &mut sched, &mut irq);
        assert_eq!(ctc.irq_state(), DAISY_INT);
        assert_eq!(ctc.irq_ack(&mut sched, &mut irq), 0x14);
        assert_eq!(ctc.irq_state(), DAISY_IEO);
        ctc.decode(0xed, &mut sched, &mut irq);
        ctc.decode(0x4d, &mut sched, &mut irq);
        assert_eq!(ctc.irq_state(), 0);
        assert_eq!(irq.queued(), vec![true, false, false]);
    }

    #[test]
    fn channel_zero_clocks_channel_one() {
        let (mut ctc, mut sched, mut irq) = setup();
        ctc.write(1, 0x45, &mut sched, &mut irq); // counter, falling edge, no interrupt
        ctc.write(1, 3, &mut sched, &mut irq);
        ctc.write(0, 0x05, &mut sched, &mut irq); // timer, prescaler 16
        ctc.write(0, 1, &mut sched, &mut irq);
        // Each channel 0 period ends with a one-clock pulse on ZC/TO 0.
        let mut t = Attotime::ZERO;
        for _ in 0..3 {
            t += Attotime::new(0, 16 * 200_000_000_000);
            run_to(t, &mut ctc, &mut sched, &mut irq);
            t += Attotime::new(0, 200_000_000_000);
            run_to(t, &mut ctc, &mut sched, &mut irq);
            t = t - Attotime::new(0, 200_000_000_000);
        }
        // Three falling edges took channel 1 from 3 to 0 and reloaded it.
        assert_eq!(ctc.channels[1].down, 3);
    }
}
