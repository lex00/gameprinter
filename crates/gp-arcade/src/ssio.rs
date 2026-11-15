// license:BSD-3-Clause
// copyright-holders:Aaron Giles
//
// The 90913 Super Sound I/O board on its own, as MAME 0.276 runs it inside
// Tapper (src/mame/midway/midway.cpp, midway_ssio_device: the sound CPU's
// map, its 14024 interrupt counter and the four latches from the main
// board), with the scheduling of src/emu/schedule.cpp. BSD-3-Clause.

//! The sound board alone: its Z80, its RAM, the 14024 counter that
//! interrupts it, the four latches the main board writes and the two
//! AY-8910s ([`crate::sound`]), with no main board.
//!
//! [`crate::Tapper`] runs this board beside the main CPU. A player whose
//! game is a model rather than the original's main CPU (the public page,
//! where a Rust model plays the game) has no main CPU to write the
//! latches, so it runs this instead and writes them itself: the board's
//! own sound program, from the player's own romset, makes the sound. Fed
//! the main CPU's latch writes at their times, it makes the same AY writes
//! and the same speaker samples as the whole machine (tests/ssio_alone.rs,
//! with the romset on the machine).
//!
//! The SSIO's answers to the main board (its status latch, which Tapper's
//! boot waits on) are kept but go nowhere.

use std::sync::Arc;

use crate::attotime::{ATTOSECONDS_PER_SECOND, Attotime};
use crate::irq::IrqLine;
use crate::romset::Romset;
use crate::sched::{Exec, Scheduler};
use crate::sound::Sound;
use crate::z80::{Z80, Z80Bus};

/// One of Tapper's 30 Hz video frames, the length [`SoundBoard::frame`]
/// runs.
pub const FRAME: Attotime = Attotime {
    seconds: 0,
    attoseconds: ATTOSECONDS_PER_SECOND / 30,
};

/// What each of the board's timers does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum What {
    /// The sound manager's 50 Hz stream update.
    SoundUpdate,
    /// The 14024 counter, clocked at 50 kHz.
    Periodic,
    /// A latch write from the main board reaching the board: latch in bits
    /// 8 and 9, the byte below.
    Write,
    /// Queued IRQ line changes.
    Irq,
    /// The end of a [`SoundBoard::run_to`].
    Stop,
}

/// The sound board's state besides its CPU.
#[derive(Clone)]
struct Bus {
    sched: Scheduler<What>,
    rom: Arc<[u8]>,
    ram: [u8; 0x400],
    data: [u8; 4],
    status: u8,
    c14024: u8,
    irq: IrqLine<What>,
    sound: Sound,
    ay_writes: Vec<(u8, u8, u8)>,
    stopped: bool,
}

/// The sound board, run alone from latch writes.
#[derive(Clone)]
pub struct SoundBoard {
    cpu: Z80,
    bus: Bus,
}

impl SoundBoard {
    /// The board at power-on, from the romset's sound program and PROM, as
    /// it is at [`crate::Tapper::power_on`].
    pub fn power_on(rom: &Romset) -> Self {
        let mut sched = Scheduler::new(
            vec![Exec::new(crate::mcr::SSIO_CPU_CLOCK)],
            Attotime::from_hz(60),
        );
        // In Tapper's order: the sound manager's timer first, then the
        // counter's.
        let t_sound = sched.alloc(What::SoundUpdate);
        let fifty = Attotime::from_hz(50);
        sched.adjust(t_sound, fifty, 0, fifty);
        let t_periodic = sched.alloc(What::Periodic);
        let mut bus = Bus {
            sched,
            rom: rom.ssiocpu.clone().into(),
            ram: [0; 0x400],
            data: [0; 4],
            status: 0,
            c14024: 0,
            irq: IrqLine::new(What::Irq, 0xff),
            sound: Sound::new(rom.ssio_prom.as_deref()),
            ay_writes: Vec::new(),
            stopped: false,
        };
        // reset_devices' share of the SSIO.
        for e in &mut bus.sched.execs {
            e.totalcycles = 0;
        }
        let mut cpu = Z80::new();
        cpu.reset();
        bus.irq.reset(0xff);
        let period = Attotime::from_hz(crate::mcr::SSIO_CLOCK / (2 * 16 * 10));
        bus.sched.adjust(t_periodic, period, 0, period);
        Self { cpu, bus }
    }

    /// The emulated time.
    pub fn time(&self) -> Attotime {
        self.bus.sched.basetime
    }

    /// The main board writes `value` to latch `latch` (0 to 3) at `at`,
    /// which must not be before [`SoundBoard::time`] (one before it lands
    /// now).
    pub fn latch(&mut self, at: Attotime, latch: u8, value: u8) {
        let now = self.time();
        let delay = if at > now { at - now } else { Attotime::ZERO };
        let param = (i32::from(latch & 3) << 8) | i32::from(value);
        self.bus.sched.timer_set(delay, What::Write, param);
    }

    /// Runs the board to `end`.
    pub fn run_to(&mut self, end: Attotime) {
        if end <= self.time() {
            return;
        }
        self.bus.sched.timer_set(end - self.time(), What::Stop, 0);
        loop {
            while let Some((handle, what, param, enabled)) = self.bus.sched.pop_due() {
                if enabled {
                    self.fire(what, param);
                }
                self.bus.sched.finish_due(handle);
                if self.bus.stopped {
                    self.bus.stopped = false;
                    return;
                }
            }
            self.timeslice();
        }
    }

    /// One 30 Hz frame from now, with the main board's latch writes in it:
    /// each the time from the frame's start, the latch and the byte.
    pub fn frame(&mut self, writes: &[(Attotime, u8, u8)]) {
        let start = self.time();
        self.bus.ay_writes.clear();
        self.bus.sound.written.clear();
        for &(at, latch, value) in writes {
            self.latch(start + at, latch, value);
        }
        self.run_to(start + FRAME);
    }

    /// Clears the frame's write lists, for a caller running the board with
    /// [`SoundBoard::run_to`] instead of [`SoundBoard::frame`].
    pub fn clear_writes(&mut self) {
        self.bus.ay_writes.clear();
        self.bus.sound.written.clear();
    }

    /// The sound CPU's writes to the AYs since the last frame started:
    /// chip, latch (0 address, 2 data) and value, as
    /// [`crate::Tapper::ay_writes`] gives them.
    pub fn ay_writes(&self) -> &[(u8, u8, u8)] {
        &self.bus.ay_writes
    }

    /// The AY registers written since the last frame started: chip,
    /// register, value.
    pub fn ay_registers_written(&self) -> &[(u8, u8, u8)] {
        &self.bus.sound.written
    }

    /// Keeps the speakers' samples from now on, or stops keeping them.
    pub fn set_audio(&mut self, on: bool) {
        self.bus.sound.out = on.then(Vec::new);
    }

    /// The speakers' samples made since the last call, left (AY 0) and
    /// right (AY 1), at [`crate::sound::OUT_RATE`], as
    /// [`crate::Tapper::take_audio`] gives them.
    pub fn take_audio(&mut self) -> Vec<[f32; 2]> {
        self.bus
            .sound
            .out
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// The board's answer to the main board (its status latch).
    pub fn status(&self) -> u8 {
        self.bus.status
    }

    /// The sound CPU's RAM.
    pub fn ram(&self) -> &[u8] {
        &self.bus.ram
    }

    fn timeslice(&mut self) {
        let quantum = Attotime::new(0, self.bus.sched.quantum);
        while self.bus.sched.basetime < self.bus.sched.next_expire() {
            let base = self.bus.sched.basetime;
            let mut target = base + quantum;
            let next = self.bus.sched.next_expire();
            if next < target {
                target = next;
            }
            if let Some(cycles) = self.bus.sched.cycles_to_run(0, target) {
                {
                    let e = &mut self.bus.sched.execs[0];
                    e.cycles_running = cycles;
                    e.cycles_stolen = 0;
                }
                self.bus.sched.executing = Some(0);
                self.cpu.icount = cycles;
                self.cpu.execute_run(&mut self.bus);
                let left = self.cpu.icount;
                self.bus.sched.executing = None;
                let local = self.bus.sched.account(0, cycles, left);
                if local < target {
                    target = local.max(base);
                }
            }
            self.bus.sched.basetime = target;
        }
    }

    fn fire(&mut self, what: What, param: i32) {
        let b = &mut self.bus;
        match what {
            What::SoundUpdate => {
                let now = b.sched.time();
                b.sound.update(now);
            }
            What::Periodic => {
                // clock_14024: bit 6 of the 7-bit count is /SINT.
                b.c14024 = (b.c14024 + 1) & 0x7f;
                if b.c14024 & 0x3f == 0 {
                    let on = b.c14024 & 0x40 != 0;
                    b.irq.set(&mut b.sched, on);
                }
            }
            What::Write => b.data[((param >> 8) & 3) as usize] = param as u8,
            What::Irq => {
                for state in b.irq.take() {
                    self.cpu.set_irq_line(state, false);
                }
            }
            What::Stop => b.stopped = true,
        }
    }
}

impl Bus {
    fn read(&mut self, addr: u16, side_effects: bool) -> u8 {
        let a = usize::from(addr);
        match addr {
            0x0000..=0x3fff => self.rom.get(a).copied().unwrap_or(0xff),
            0x8000..=0x8fff => self.ram[a & 0x3ff],
            0x9000..=0x9fff => self.data[a & 3],
            0xa000..=0xafff if a & 3 == 1 => self.sound.bus[0].data_r(),
            0xb000..=0xbfff if a & 3 == 1 => self.sound.bus[1].data_r(),
            0xe000..=0xefff => {
                // irq_clear: resets the 14024 count and clears /SINT.
                if side_effects {
                    self.c14024 = 0;
                    self.irq.set(&mut self.sched, false);
                }
                0xff
            }
            _ => 0xff,
        }
    }

    fn write(&mut self, addr: u16, v: u8) {
        let a = usize::from(addr);
        match addr {
            0x8000..=0x8fff => self.ram[a & 0x3ff] = v,
            0xa000..=0xbfff if a & 1 == 0 => {
                let chip = usize::from(addr >= 0xb000);
                let latch = (a & 3) as u8;
                self.ay_writes.push((chip as u8, latch, v));
                let now = self.sched.time();
                self.sound.write(chip, latch, v, now);
            }
            0xc000..=0xcfff => self.status = v,
            _ => {}
        }
    }
}

impl Z80Bus for Bus {
    fn read(&mut self, addr: u16, icount: &mut i32) -> u8 {
        self.sched.icount = *icount;
        let v = Bus::read(self, addr, true);
        *icount = self.sched.icount;
        v
    }
    fn write(&mut self, addr: u16, value: u8, icount: &mut i32) {
        // The AYs' writes land at the CPU's time.
        self.sched.icount = *icount;
        Bus::write(self, addr, value);
    }
    fn fetch(&mut self, addr: u16) -> u8 {
        Bus::read(self, addr, false)
    }
    fn io_read(&mut self, _port: u16, _icount: &mut i32) -> u8 {
        // The SSIO board maps no I/O space.
        0xff
    }
    fn io_write(&mut self, _port: u16, _value: u8, _icount: &mut i32) {}
    fn irq_ack(&mut self, _icount: &mut i32) -> i32 {
        self.irq.ack_vector()
    }
    fn instruction(&mut self, _cpu: &Z80) {}
    fn reti(&mut self, _icount: &mut i32) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Controls, Tapper};

    fn made_up() -> Romset {
        crate::testrom::sounding()
    }

    #[test]
    fn a_latch_write_reaches_the_sound_program() {
        let mut b = SoundBoard::power_on(&made_up());
        b.frame(&[]);
        assert!(b.ay_registers_written().contains(&(0, 7, 0x7e)));
        let half = Attotime::new(0, FRAME.attoseconds / 2);
        b.frame(&[(half, 0, 0x1c)]);
        let w = b.ay_registers_written();
        assert_eq!(w.first(), Some(&(0, 8, 0)));
        assert_eq!(w.last(), Some(&(0, 8, 0x0c)));
    }

    /// Fed the main CPU's latch writes at their times, the board alone
    /// makes the whole machine's AY writes and speaker samples, frame by
    /// frame.
    #[test]
    fn alone_it_sounds_as_it_does_in_the_whole_machine() {
        let rom = made_up();
        let mut whole = Tapper::power_on(&rom);
        whole.set_audio(true);
        whole.set_latch_log(true);
        let mut alone = SoundBoard::power_on(&rom);
        alone.set_audio(true);
        let mut writes = 0;
        let mut loud = 0;
        for f in 0..90 {
            whole.step(Controls::default());
            alone.clear_writes();
            for (at, latch, value) in whole.take_latch_log() {
                alone.latch(at, latch, value);
                writes += 1;
            }
            alone.run_to(whole.time());
            let (x, y) = (alone.ay_writes(), whole.ay_writes());
            let first = x.iter().zip(y).position(|(p, q)| p != q);
            assert!(
                x == y,
                "frame {f}'s AY writes: {} alone, {} whole, first differing at {first:?}: {:?} against {:?}",
                x.len(),
                y.len(),
                first.map(|i| &x[i.saturating_sub(2)..(i + 3).min(x.len())]),
                first.map(|i| &y[i.saturating_sub(2)..(i + 3).min(y.len())])
            );
            let (a, w) = (alone.take_audio(), whole.take_audio());
            loud += w.iter().filter(|s| s[0].abs() > 0.01).count();
            assert_eq!(a, w, "frame {f}'s samples");
        }
        assert!(writes > 1000, "{writes} latch writes");
        assert!(loud > 0, "the made-up program made no sound");
    }

    #[test]
    fn the_board_alone_is_deterministic_and_clones_exactly() {
        let mut a = SoundBoard::power_on(&made_up());
        a.set_audio(true);
        a.frame(&[(Attotime::ZERO, 0, 0x0f)]);
        let mut b = a.clone();
        for k in 0..10u8 {
            let w = [(Attotime::new(0, i64::from(k) * 1_000_000_000_000), 0, k)];
            a.frame(&w);
            b.frame(&w);
            assert_eq!(a.take_audio(), b.take_audio());
            assert_eq!(a.ay_writes(), b.ay_writes());
        }
        assert_eq!(a.time(), b.time());
    }
}
