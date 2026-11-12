// license:BSD-3-Clause
// copyright-holders:Aaron Giles
//
// Tapper's boards as MAME 0.276 has them: the 91490 CPU board
// (src/mame/midway/mcr.cpp: cpu_91490_map, cpu_91490_portmap, mcr_91490,
// mcr_m.cpp: mcr_interrupt), the 91464 video board (mcr_v.cpp, in
// video.rs), and the 90913 Super Sound I/O board (src/mame/midway/
// midway.cpp, midway_ssio_device). The screen and watchdog timing follow
// src/emu/screen.cpp, src/devices/machine/timer.cpp and watchdog.cpp; the
// AY-8910 register file follows src/devices/sound/ay8910.cpp (copyright-
// holders Couriersud). All of these are BSD-3-Clause.

//! The whole machine, one 30 Hz frame at a time.

use std::sync::Arc;

use crate::attotime::{ATTOSECONDS_PER_SECOND, Attotime};
use crate::ctc::{Ctc, CtcTimer, DAISY_IEO, DAISY_INT};
use crate::irq::IrqLine;
use crate::romset::Romset;
use crate::sched::{Exec, Scheduler};
use crate::sound::Sound;
use crate::state::{Reader, Writer};
use crate::video::{self, Gfx, HEIGHT, WIDTH};
use crate::z80::{Z80, Z80Bus};

/// The main board's Z80 and the CTC: 5 MHz (mcr_91490).
pub const MAIN_CLOCK: u32 = 5_000_000;
/// The SSIO board: a 16 MHz crystal; its Z80 runs at a eighth of it.
pub const SSIO_CLOCK: u32 = 16_000_000;
pub const SSIO_CPU_CLOCK: u32 = SSIO_CLOCK / 8;

const MAIN: usize = 0;
const SSIO: usize = 1;

/// The screen: 30 Hz, 480 lines; the "VBLANK" lasts 2500 us.
const FRAME_PERIOD: i64 = ATTOSECONDS_PER_SECOND / 30;
const SCANTIME: i64 = FRAME_PERIOD / HEIGHT as i64;
const PIXELTIME: i64 = FRAME_PERIOD / (HEIGHT as i64 * WIDTH as i64);
const VBLANK_PERIOD: i64 = 2500 * 1_000_000_000_000;

/// The watchdog fires after 16 frames without a write to port E0.
const WATCHDOG_FRAMES: i32 = 16;

/// Bytes in one oracle RAM record (docs/oracle-mame.md).
pub const RAM_RECORD: usize = 2048 + 512 + 2048 + 128 + 1024;

/// What each timer does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    /// The sound system's 50 Hz update. It changes nothing here, but MAME
    /// has it, and every timer ends a timeslice.
    SoundUpdate,
    /// The scanline timer that drives the CTC's triggers.
    ScanTimer,
    VblankBegin,
    VblankEnd,
    /// The screen's start-of-frame tick.
    Scanline0,
    Ctc(CtcTimer),
    /// The SSIO board's 14024 counter, clocked at 50 kHz.
    SsioPeriodic,
    /// A latch written by the main CPU reaching the SSIO board.
    SsioWrite,
    /// Queued IRQ line changes for a CPU.
    MainIrq,
    SsioIrq,
    /// running_machine's soft reset, set by the watchdog.
    SoftReset,
}

impl What {
    fn code(self) -> (u8, u8) {
        match self {
            Self::SoundUpdate => (0, 0),
            Self::ScanTimer => (1, 0),
            Self::VblankBegin => (2, 0),
            Self::VblankEnd => (3, 0),
            Self::Scanline0 => (4, 0),
            Self::Ctc(CtcTimer::Count(c)) => (5, c),
            Self::Ctc(CtcTimer::ZcEnd(c)) => (6, c),
            Self::SsioPeriodic => (7, 0),
            Self::SsioWrite => (8, 0),
            Self::MainIrq => (9, 0),
            Self::SsioIrq => (10, 0),
            Self::SoftReset => (11, 0),
        }
    }

    fn from_code(tag: u8, arg: u8) -> Result<Self, String> {
        Ok(match tag {
            0 => Self::SoundUpdate,
            1 => Self::ScanTimer,
            2 => Self::VblankBegin,
            3 => Self::VblankEnd,
            4 => Self::Scanline0,
            5 if arg < 4 => Self::Ctc(CtcTimer::Count(arg)),
            6 if arg < 4 => Self::Ctc(CtcTimer::ZcEnd(arg)),
            7 => Self::SsioPeriodic,
            8 => Self::SsioWrite,
            9 => Self::MainIrq,
            10 => Self::SsioIrq,
            11 => Self::SoftReset,
            _ => return Err(format!("unknown timer {tag}/{arg} in snapshot")),
        })
    }
}

/// One frame's controls, as the game reads them through the SSIO board.
/// Each is true when held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Controls {
    pub coin: bool,
    pub start: bool,
    pub right: bool,
    pub left: bool,
    pub down: bool,
    pub up: bool,
    pub tap: bool,
    /// The cabinet's tilt switch. Only the oracle's logs name it (column
    /// `X`, after the seven of `CSRLDUT`); the game hangs after a tilt and
    /// the watchdog resets the board.
    pub tilt: bool,
}

impl Controls {
    /// The oracle log's columns, CSRLDUT and then X (tilt), one bit each
    /// from bit 0.
    pub fn from_bits(bits: u32) -> Self {
        let b = |n: u32| bits & (1 << n) != 0;
        Self {
            coin: b(0),
            start: b(1),
            right: b(2),
            left: b(3),
            down: b(4),
            up: b(5),
            tap: b(6),
            tilt: b(7),
        }
    }

    pub fn bits(self) -> u32 {
        [
            self.coin, self.start, self.right, self.left, self.down, self.up, self.tap, self.tilt,
        ]
        .iter()
        .enumerate()
        .map(|(i, &on)| u32::from(on) << i)
        .sum()
    }

    /// The five SSIO input ports, active low, from mcr.cpp's
    /// INPUT_PORTS_START( tapper ) with the DIP switches at their defaults
    /// (demo sounds on, upright, one coin meter).
    ///
    /// MAME's 4-way joystick filter isn't modelled: only one direction at
    /// a time is meaningful.
    pub fn ports(self) -> [u8; 5] {
        let clear = |on: bool, mask: u8| if on { mask } else { 0 };
        let ip0 = !(clear(self.coin, 0x01) | clear(self.start, 0x04) | clear(self.tilt, 0x20));
        let ip1 = !(clear(self.right, 0x01)
            | clear(self.left, 0x02)
            | clear(self.down, 0x04)
            | clear(self.up, 0x08)
            | clear(self.tap, 0x10));
        [ip0, ip1, 0xff, 0xfb, 0xff]
    }
}

/// Everything but the two CPUs.
#[derive(Clone)]
pub struct Board {
    pub sched: Scheduler<What>,
    pub main_rom: Arc<[u8]>,
    pub ssio_rom: Arc<[u8]>,
    /// e000-e7ff, battery backed; MAME starts it at 0xff.
    pub nvram: [u8; 0x800],
    /// e800-e9ff, mirrored at ea00.
    pub spriteram: [u8; 0x200],
    /// f000-f7ff.
    pub videoram: [u8; 0x800],
    /// f800-f87f, write only.
    pub paletteram: [u8; 0x80],
    /// The 64 colours as 0xRRGGBB. The ninth bit comes from which byte of
    /// the pair was written last, so this isn't a function of paletteram.
    pub palette: [u32; 64],
    pub cocktail_flip: bool,
    pub ctc: Ctc,
    pub main_irq: IrqLine<What>,
    pub ssio_irq: IrqLine<What>,
    pub watchdog_counter: i32,
    /// Set when the watchdog has run out since power-on: the board reset
    /// itself, as MAME's does.
    pub watchdog_fired: bool,
    // The screen.
    pub vblank_start: Attotime,
    pub vblank_end: Attotime,
    pub frame_number: u64,
    pub t_vblank_begin: usize,
    pub t_vblank_end: usize,
    pub t_scanline0: usize,
    pub t_scantimer: usize,
    pub scantimer_first: bool,
    pub t_sound: usize,
    // The SSIO board.
    pub ssio_ram: [u8; 0x400],
    pub ssio_data: [u8; 4],
    pub ssio_status: u8,
    pub ssio_14024: u8,
    pub t_ssio_periodic: usize,
    /// The two AY-8910s, the SSIO's volumes and the speakers.
    pub sound: Sound,
    /// The sound CPU's writes to the AYs in the frame being run: the chip,
    /// the latch (0 address, 2 data) and the value, as the oracle's
    /// `ay.tsv` has them. Cleared when a frame starts; not part of the
    /// state.
    pub ay_writes: Vec<(u8, u8, u8)>,
    pub ports: [u8; 5],
    /// Set by the VBLANK timer: the frame is over.
    pub frame_done: bool,
    /// Whether the main CPU read an input port (0 to 4) in the frame being
    /// run: a frame without is a lag frame. Cleared when a frame starts;
    /// not part of the state.
    pub polled: bool,
    /// When on, every instruction both CPUs start, for debugging. Not
    /// part of the state.
    pub trace: Option<Vec<Trace>>,
}

/// One instruction start: which CPU, the start of its timeslice, the PC
/// and the main registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trace {
    pub cpu: u8,
    pub time: Attotime,
    pub pc: u16,
    pub af: u16,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
}

impl Board {
    fn now(&self) -> Attotime {
        self.sched.time()
    }

    /// screen_device::vpos.
    fn vpos(&self) -> i64 {
        let delta = (self.now() - self.vblank_start).as_attoseconds() + PIXELTIME / 2;
        (HEIGHT as i64 + delta / SCANTIME) % HEIGHT as i64
    }

    /// screen_device::time_until_pos, for the start of a line.
    fn time_until_pos(&self, vpos: i64) -> Attotime {
        let vpos = vpos % HEIGHT as i64;
        let mut target = vpos * SCANTIME;
        let cur = (self.now() - self.vblank_start).as_attoseconds();
        if target <= cur + PIXELTIME / 2 {
            target += FRAME_PERIOD;
        }
        while target <= cur {
            target += FRAME_PERIOD;
        }
        Attotime::new(0, target - cur)
    }

    fn time_until_vblank_end(&self) -> Attotime {
        let now = self.now();
        let mut target = self.vblank_end;
        if now >= self.vblank_end {
            target += Attotime::new(0, FRAME_PERIOD);
        }
        target - now
    }

    fn main_read(&self, addr: u16) -> u8 {
        let a = usize::from(addr);
        match addr {
            0x0000..=0xdfff => self.main_rom[a],
            0xe000..=0xe7ff => self.nvram[a & 0x7ff],
            0xe800..=0xebff => self.spriteram[a & 0x1ff],
            0xf000..=0xf7ff => self.videoram[a & 0x7ff],
            _ => 0xff,
        }
    }

    fn main_write(&mut self, addr: u16, v: u8) {
        let a = usize::from(addr);
        match addr {
            0xe000..=0xe7ff => self.nvram[a & 0x7ff] = v,
            0xe800..=0xebff => self.spriteram[a & 0x1ff] = v,
            0xf000..=0xf7ff => self.videoram[a & 0x7ff] = v,
            0xf800..=0xffff => {
                // mcr_paletteram9_w
                let off = a & 0x7f;
                self.paletteram[off] = v;
                self.palette[off / 2] = video::color_9bit(u16::from(v) | (((off & 1) as u16) << 8));
            }
            _ => {}
        }
    }

    fn main_io_read(&mut self, port: u16) -> u8 {
        let p = port & 0xff;
        let m = p & !0x18;
        if m <= 4 {
            self.polled = true;
            return self.ports[usize::from(m)];
        }
        if m == 7 {
            return self.ssio_status;
        }
        if (0xf0..=0xf3).contains(&p) {
            return self.ctc.read(usize::from(p & 3), &self.sched);
        }
        0xff
    }

    fn main_io_write(&mut self, port: u16, v: u8) {
        let p = port & 0xff;
        match p {
            // The SSIO's output ports; port 0's bits go to mcr_control_port_w.
            0x00..=0x03 => self.cocktail_flip = (v >> 6) & 1 != 0,
            0x04..=0x07 => {}
            0x1c..=0x1f => self
                .sched
                .synchronize(What::SsioWrite, (i32::from(p & 3) << 8) | i32::from(v)),
            0xe0 => self.watchdog_counter = WATCHDOG_FRAMES,
            0xf0..=0xf3 => {
                self.ctc
                    .write(usize::from(p & 3), v, &mut self.sched, &mut self.main_irq)
            }
            _ => {}
        }
    }

    fn ssio_read(&mut self, addr: u16, side_effects: bool) -> u8 {
        let a = usize::from(addr);
        match addr {
            0x0000..=0x3fff => self.ssio_rom[a],
            0x8000..=0x8fff => self.ssio_ram[a & 0x3ff],
            0x9000..=0x9fff => self.ssio_data[a & 3],
            0xa000..=0xafff if a & 3 == 1 => self.sound.bus[0].data_r(),
            0xb000..=0xbfff if a & 3 == 1 => self.sound.bus[1].data_r(),
            0xe000..=0xefff => {
                // irq_clear: resets the 14024 count and clears /SINT.
                if side_effects {
                    self.ssio_14024 = 0;
                    self.ssio_irq.set(&mut self.sched, false);
                }
                0xff
            }
            _ => 0xff,
        }
    }

    fn ssio_write(&mut self, addr: u16, v: u8) {
        let a = usize::from(addr);
        match addr {
            0x8000..=0x8fff => self.ssio_ram[a & 0x3ff] = v,
            0xa000..=0xbfff if a & 1 == 0 => {
                let chip = usize::from(addr >= 0xb000);
                let latch = (a & 3) as u8;
                self.ay_writes.push((chip as u8, latch, v));
                let now = self.now();
                self.sound.write(chip, latch, v, now);
            }
            0xc000..=0xcfff => self.ssio_status = v,
            _ => {}
        }
    }
}

struct MainBus<'a>(&'a mut Board);
struct SsioBus<'a>(&'a mut Board);

impl Z80Bus for MainBus<'_> {
    fn read(&mut self, addr: u16, _icount: &mut i32) -> u8 {
        self.0.main_read(addr)
    }
    fn write(&mut self, addr: u16, value: u8, _icount: &mut i32) {
        self.0.main_write(addr, value);
    }
    fn fetch(&mut self, addr: u16) -> u8 {
        self.0.main_read(addr)
    }
    fn io_read(&mut self, port: u16, icount: &mut i32) -> u8 {
        self.0.sched.icount = *icount;
        let v = self.0.main_io_read(port);
        *icount = self.0.sched.icount;
        v
    }
    fn io_write(&mut self, port: u16, value: u8, icount: &mut i32) {
        self.0.sched.icount = *icount;
        self.0.main_io_write(port, value);
        *icount = self.0.sched.icount;
    }
    fn irq_ack(&mut self, icount: &mut i32) -> i32 {
        let b = &mut *self.0;
        b.sched.icount = *icount;
        let v = if b.ctc.irq_state() & DAISY_INT != 0 {
            b.ctc.irq_ack(&mut b.sched, &mut b.main_irq)
        } else {
            b.main_irq.ack_vector()
        };
        *icount = b.sched.icount;
        v
    }
    fn instruction(&mut self, pc: u16, af: u16, bc: u16, de: u16, hl: u16) {
        if let Some(t) = &mut self.0.trace {
            t.push(Trace {
                cpu: MAIN as u8,
                time: self.0.sched.execs[MAIN].localtime,
                pc,
                af,
                bc,
                de,
                hl,
            });
        }
    }
    fn reti(&mut self, icount: &mut i32) {
        let b = &mut *self.0;
        b.sched.icount = *icount;
        if b.ctc.irq_state() & DAISY_IEO != 0 {
            b.ctc.decode(0xed, &mut b.sched, &mut b.main_irq);
            b.ctc.decode(0x4d, &mut b.sched, &mut b.main_irq);
        }
        *icount = b.sched.icount;
    }
}

impl Z80Bus for SsioBus<'_> {
    fn read(&mut self, addr: u16, icount: &mut i32) -> u8 {
        self.0.sched.icount = *icount;
        let v = self.0.ssio_read(addr, true);
        *icount = self.0.sched.icount;
        v
    }
    fn write(&mut self, addr: u16, value: u8, icount: &mut i32) {
        // The AYs' writes land at the CPU's time.
        self.0.sched.icount = *icount;
        self.0.ssio_write(addr, value);
    }
    fn fetch(&mut self, addr: u16) -> u8 {
        self.0.ssio_read(addr, false)
    }
    fn io_read(&mut self, _port: u16, _icount: &mut i32) -> u8 {
        // The SSIO board maps no I/O space.
        0xff
    }
    fn io_write(&mut self, _port: u16, _value: u8, _icount: &mut i32) {}
    fn irq_ack(&mut self, _icount: &mut i32) -> i32 {
        self.0.ssio_irq.ack_vector()
    }
    fn instruction(&mut self, pc: u16, af: u16, bc: u16, de: u16, hl: u16) {
        if let Some(t) = &mut self.0.trace {
            t.push(Trace {
                cpu: SSIO as u8,
                time: self.0.sched.execs[SSIO].localtime,
                pc,
                af,
                bc,
                de,
                hl,
            });
        }
    }
    fn reti(&mut self, _icount: &mut i32) {}
}

/// Tapper: both boards, the screen and the frame counter.
#[derive(Clone)]
pub struct Tapper {
    pub main: Z80,
    pub ssio: Z80,
    pub board: Board,
    gfx: Arc<Gfx>,
    /// The last frame's picture, one pen (0 to 63) a pixel, with
    /// video::SPRITE_BIT where a sprite drew it.
    pens: Vec<u8>,
    /// The palette as it was when the picture was drawn.
    pens_palette: [u32; 64],
    frame: u64,
}

impl Tapper {
    /// The machine at power-on (the oracle's frame 0): MAME's device start
    /// and reset, in MAME's order, at time zero.
    pub fn power_on(rom: &Romset) -> Self {
        let mut sched = Scheduler::new(
            vec![Exec::new(MAIN_CLOCK), Exec::new(SSIO_CPU_CLOCK)],
            Attotime::from_hz(60),
        );
        // The sound manager starts before the devices.
        let t_sound = sched.alloc(What::SoundUpdate);
        let fifty = Attotime::from_hz(50);
        sched.adjust(t_sound, fifty, 0, fifty);
        // device_start, in device order.
        let t_scantimer = sched.alloc(What::ScanTimer);
        let ctc = Ctc::new(MAIN_CLOCK, &mut sched, What::Ctc);
        let t_vblank_begin = sched.alloc(What::VblankBegin);
        let t_vblank_end = sched.alloc(What::VblankEnd);
        let t_scanline0 = sched.alloc(What::Scanline0);
        let t_ssio_periodic = sched.alloc(What::SsioPeriodic);
        let mut board = Board {
            sched,
            main_rom: rom.maincpu.clone().into(),
            ssio_rom: rom.ssiocpu.clone().into(),
            nvram: [0xff; 0x800],
            spriteram: [0; 0x200],
            videoram: [0; 0x800],
            paletteram: [0; 0x80],
            palette: [0; 64],
            cocktail_flip: false,
            ctc,
            main_irq: IrqLine::new(What::MainIrq, 0xff),
            ssio_irq: IrqLine::new(What::SsioIrq, 0xff),
            watchdog_counter: 0,
            watchdog_fired: false,
            vblank_start: Attotime::ZERO,
            vblank_end: Attotime::ZERO,
            frame_number: 0,
            t_vblank_begin,
            t_vblank_end,
            t_scanline0,
            t_scantimer,
            scantimer_first: true,
            t_sound,
            ssio_ram: [0; 0x400],
            ssio_data: [0; 4],
            ssio_status: 0,
            ssio_14024: 0,
            t_ssio_periodic,
            sound: Sound::new(rom.ssio_prom.as_deref()),
            ay_writes: Vec::new(),
            ports: Controls::default().ports(),
            frame_done: false,
            polled: false,
            trace: None,
        };
        // The screen's configure(): VBLANK starts a frame from now, and
        // since this is line 0, reset_partial_updates sets the line-0 tick.
        let until = board.time_until_pos(HEIGHT as i64);
        board
            .sched
            .adjust(t_vblank_begin, until, 0, Attotime::NEVER);
        let until = board.time_until_pos(0);
        board.sched.adjust(t_scanline0, until, 0, Attotime::NEVER);
        board.vblank_start = Attotime::ZERO;
        board.vblank_end = Attotime::new(0, VBLANK_PERIOD);

        // The AYs reset in Sound::new; the rest in reset_devices.
        let mut tapper = Self {
            main: Z80::new(),
            ssio: Z80::new(),
            board,
            gfx: Arc::new(Gfx::decode(&rom.gfx1, &rom.gfx2)),
            pens: vec![0; WIDTH * HEIGHT],
            pens_palette: [0; 64],
            frame: 0,
        };
        tapper.reset_devices();
        tapper
    }

    /// running_machine::reset_all_devices, in device order, without the
    /// AYs: at power-on, and at a soft reset. The main Z80's daisy chain
    /// resets the CTC after the CPU, and the CTC resets again in its own
    /// turn. Each CPU keeps the registers device_reset doesn't set; the
    /// RAM, the palette and the screen's timing are untouched.
    fn reset_devices(&mut self) {
        let board = &mut self.board;
        for e in &mut board.sched.execs {
            // interface_pre_reset
            e.totalcycles = 0;
        }
        self.main.reset();
        board.main_irq.reset(0xff);
        board.ctc.reset(&mut board.sched, &mut board.main_irq);
        board.ctc.reset_daisy();
        board.scantimer_first = true;
        board
            .sched
            .adjust(board.t_scantimer, Attotime::ZERO, 0, Attotime::NEVER);
        board.ctc.reset(&mut board.sched, &mut board.main_irq);
        board.ctc.reset_daisy();
        board.watchdog_counter = WATCHDOG_FRAMES;
        board.ssio_data = [0; 4];
        board.ssio_status = 0;
        board.ssio_14024 = 0;
        self.ssio.reset();
        board.ssio_irq.reset(0xff);
        let period = Attotime::from_hz(SSIO_CLOCK / (2 * 16 * 10));
        board.sched.adjust(board.t_ssio_periodic, period, 0, period);
        // mcr_state::machine_reset, after the children.
        board.cocktail_flip = false;
    }

    /// Frames run since power-on.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// The emulated time.
    pub fn time(&self) -> Attotime {
        self.board.sched.basetime
    }

    /// The input ports during the last frame run.
    pub fn ports(&self) -> [u8; 5] {
        self.board.ports
    }

    /// Runs one frame with these controls held: up to the start of the
    /// next VBLANK, where MAME ends a frame.
    pub fn step(&mut self, controls: Controls) {
        self.board.ports = controls.ports();
        self.board.polled = false;
        self.board.ay_writes.clear();
        self.board.sound.written.clear();
        loop {
            while let Some((handle, what, param, enabled)) = self.board.sched.pop_due() {
                if enabled {
                    self.fire(what, param);
                }
                self.board.sched.finish_due(handle);
                if self.board.frame_done {
                    self.board.frame_done = false;
                    self.frame += 1;
                    return;
                }
            }
            self.timeslice();
        }
    }

    /// device_scheduler::timeslice, up to the next timer.
    fn timeslice(&mut self) {
        let quantum = Attotime::new(0, self.board.sched.quantum);
        while self.board.sched.basetime < self.board.sched.next_expire() {
            let base = self.board.sched.basetime;
            let mut target = base + quantum;
            let next = self.board.sched.next_expire();
            if next < target {
                target = next;
            }
            for cpu in [MAIN, SSIO] {
                let Some(cycles) = self.board.sched.cycles_to_run(cpu, target) else {
                    continue;
                };
                {
                    let e = &mut self.board.sched.execs[cpu];
                    e.cycles_running = cycles;
                    e.cycles_stolen = 0;
                }
                self.board.sched.executing = Some(cpu);
                let left = if cpu == MAIN {
                    self.main.icount = cycles;
                    self.main.execute_run(&mut MainBus(&mut self.board));
                    self.main.icount
                } else {
                    self.ssio.icount = cycles;
                    self.ssio.execute_run(&mut SsioBus(&mut self.board));
                    self.ssio.icount
                };
                self.board.sched.executing = None;
                let local = self.board.sched.account(cpu, cycles, left);
                if local < target {
                    target = local.max(base);
                }
            }
            self.board.sched.basetime = target;
        }
    }

    fn fire(&mut self, what: What, param: i32) {
        let b = &mut self.board;
        match what {
            What::SoundUpdate => {
                let now = b.now();
                b.sound.update(now);
            }
            What::ScanTimer => {
                // timer_device::scanline_tick with mcr_interrupt.
                let mut next = 0;
                if !b.scantimer_first {
                    let vpos = b.vpos();
                    if vpos == 0 || vpos == 240 {
                        b.ctc.trigger(2, true, &mut b.sched, &mut b.main_irq);
                        b.ctc.trigger(2, false, &mut b.sched, &mut b.main_irq);
                    }
                    if vpos == 0 {
                        b.ctc.trigger(3, true, &mut b.sched, &mut b.main_irq);
                        b.ctc.trigger(3, false, &mut b.sched, &mut b.main_irq);
                    }
                    if vpos + 1 < HEIGHT as i64 {
                        next = vpos + 1;
                    }
                }
                b.scantimer_first = false;
                let until = b.time_until_pos(next);
                b.sched.adjust(b.t_scantimer, until, 0, Attotime::NEVER);
            }
            What::VblankBegin => {
                let now = b.now();
                b.vblank_start = now;
                b.vblank_end = now + Attotime::new(0, VBLANK_PERIOD);
                // frame_update: the whole picture, drawn now.
                video::draw(
                    &self.gfx,
                    &b.videoram,
                    &b.spriteram,
                    b.cocktail_flip,
                    &mut self.pens,
                );
                self.pens_palette = b.palette;
                // The watchdog's VBLANK callback, after the frame's
                // picture: when it runs out, schedule_soft_reset sets the
                // machine's reset timer for now, so it fires after the
                // timers already due now.
                b.watchdog_counter -= 1;
                if b.watchdog_counter == 0 {
                    b.watchdog_fired = true;
                    b.sched.timer_set(Attotime::ZERO, What::SoftReset, 0);
                }
                let until = b.time_until_pos(HEIGHT as i64);
                b.sched.adjust(b.t_vblank_begin, until, 0, Attotime::NEVER);
                let until = b.time_until_vblank_end();
                b.sched.adjust(b.t_vblank_end, until, 0, Attotime::NEVER);
                b.frame_done = true;
            }
            What::VblankEnd => b.frame_number += 1,
            What::Scanline0 => {
                let until = b.time_until_pos(0);
                b.sched.adjust(b.t_scanline0, until, 0, Attotime::NEVER);
            }
            What::Ctc(t) => b.ctc.fire(t, &mut b.sched, &mut b.main_irq),
            What::SsioPeriodic => {
                // clock_14024: bit 6 of the 7-bit count is /SINT.
                b.ssio_14024 = (b.ssio_14024 + 1) & 0x7f;
                if b.ssio_14024 & 0x3f == 0 {
                    let on = b.ssio_14024 & 0x40 != 0;
                    b.ssio_irq.set(&mut b.sched, on);
                }
            }
            What::SsioWrite => b.ssio_data[((param >> 8) & 3) as usize] = param as u8,
            What::MainIrq => {
                for state in b.main_irq.take() {
                    let chain = b.ctc.irq_state() & DAISY_INT != 0;
                    self.main.set_irq_line(state, chain);
                }
            }
            What::SsioIrq => {
                for state in b.ssio_irq.take() {
                    self.ssio.set_irq_line(state, false);
                }
            }
            What::SoftReset => {
                // running_machine::soft_reset: every device's reset, the
                // AYs (the SSIO's children, after its CPU) included.
                // sound_manager::reset resets the sound devices first, each
                // brought up to now; the second reset changes nothing more.
                let now = self.board.now();
                self.board.sound.reset(now);
                self.reset_devices();
            }
        }
    }

    /// The oracle's RAM record: work RAM, sprite RAM, video RAM, palette
    /// RAM, then the SSIO board's RAM (docs/oracle-mame.md).
    pub fn ram_record(&self) -> Vec<u8> {
        let b = &self.board;
        let mut out = Vec::with_capacity(RAM_RECORD);
        out.extend_from_slice(&b.nvram);
        out.extend_from_slice(&b.spriteram);
        out.extend_from_slice(&b.videoram);
        out.extend_from_slice(&b.paletteram);
        out.extend_from_slice(&b.ssio_ram);
        out
    }

    /// A byte as the main CPU would read it, without side effects.
    pub fn peek(&self, addr: u16) -> u8 {
        self.board.main_read(addr)
    }

    /// The last frame's picture as 0xRRGGBB, rows from the top.
    pub fn picture(&self) -> Vec<u32> {
        self.pens
            .iter()
            .map(|&p| self.pens_palette[usize::from(p & 0x3f)])
            .collect()
    }

    /// The last frame's picture as pens (0 to 63, a pixel, with
    /// video::SPRITE_BIT where a sprite drew it), rows from the top, and
    /// the palette they were drawn with.
    pub fn pens(&self) -> (&[u8], &[u32; 64]) {
        (&self.pens, &self.pens_palette)
    }

    /// Which pixels of the last frame a sprite drew, rows from the top:
    /// the screen measure's sprite mask.
    pub fn sprite_mask(&self) -> Vec<bool> {
        self.pens
            .iter()
            .map(|p| p & video::SPRITE_BIT != 0)
            .collect()
    }

    /// The oracle's hash of the last frame's picture. Frame 0, before any
    /// picture, is black, as MAME's is.
    pub fn picture_hash(&self) -> u64 {
        video::picture_hash(
            self.pens
                .iter()
                .map(|&p| self.pens_palette[usize::from(p & 0x3f)]),
        )
    }

    /// Whether the game read its input ports in the last frame run.
    pub fn polled(&self) -> bool {
        self.board.polled
    }

    /// The SSIO board's 1 KB of RAM, as its Z80 sees it at 0x8000.
    pub fn sound_ram(&self) -> &[u8] {
        &self.board.ssio_ram
    }

    /// The palette RAM's 128 bytes, which the main CPU writes at 0xF800.
    pub fn palette_ram(&self) -> &[u8] {
        &self.board.paletteram
    }

    /// The sound CPU's writes to the AYs in the last frame run: chip, latch
    /// (0 address, 2 data) and value, in order.
    pub fn ay_writes(&self) -> &[(u8, u8, u8)] {
        &self.board.ay_writes
    }

    /// The AY registers the sound CPU wrote in the last frame run: chip,
    /// register, value, in order (the data writes of [`Tapper::ay_writes`]
    /// with the register each landed in).
    pub fn ay_registers_written(&self) -> &[(u8, u8, u8)] {
        &self.board.sound.written
    }

    /// Keeps the speakers' samples from now on ([`Tapper::take_audio`]),
    /// or stops keeping them. They're not part of the state.
    pub fn set_audio(&mut self, on: bool) {
        self.board.sound.out = on.then(Vec::new);
    }

    /// The speakers' samples made since the last call, left and right, at
    /// [`crate::sound::OUT_RATE`]: a 20 ms piece at each of MAME's stream
    /// updates, so a frame has one or two pieces.
    pub fn take_audio(&mut self) -> Vec<[f32; 2]> {
        self.board
            .sound
            .out
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Whether the watchdog has run out since power-on, which resets the
    /// board as MAME's soft reset does.
    pub fn watchdog_fired(&self) -> bool {
        self.board.watchdog_fired
    }

    /// The whole machine state, without the ROMs.
    pub fn snapshot(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(b"gp-arcade tapper 2");
        w.u64(self.frame);
        save_z80(&mut w, &self.main);
        save_z80(&mut w, &self.ssio);
        let b = &self.board;
        save_sched(&mut w, &b.sched);
        w.bytes(&b.nvram);
        w.bytes(&b.spriteram);
        w.bytes(&b.videoram);
        w.bytes(&b.paletteram);
        for &c in &b.palette {
            w.u32(c);
        }
        w.bool(b.cocktail_flip);
        save_ctc(&mut w, &b.ctc);
        save_irq(&mut w, &b.main_irq);
        save_irq(&mut w, &b.ssio_irq);
        w.i32(b.watchdog_counter);
        w.bool(b.watchdog_fired);
        w.time(b.vblank_start);
        w.time(b.vblank_end);
        w.u64(b.frame_number);
        w.bool(b.scantimer_first);
        w.bytes(&b.ssio_ram);
        w.bytes(&b.ssio_data);
        w.u8(b.ssio_status);
        w.u8(b.ssio_14024);
        b.sound.save(&mut w);
        w.bytes(&b.ports);
        w.bytes(&self.pens);
        for &c in &self.pens_palette {
            w.u32(c);
        }
        // Only between a reset and the sound update after it, so a
        // snapshot (and its hash) without one is as it always was.
        if let Some(sp) = &b.sound.split {
            w.i32(sp.at.0);
            w.i64(sp.at.1);
            for d in &sp.duty {
                w.bytes(d);
            }
            w.u8(sp.mute);
        }
        w.out
    }

    /// Puts back a snapshot of this machine (same ROMs).
    pub fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        let mut r = Reader::new(snap);
        if r.bytes()? != b"gp-arcade tapper 2" {
            return Err("not a gp-arcade Tapper snapshot".into());
        }
        let mut next = self.clone();
        next.frame = r.u64()?;
        load_z80(&mut r, &mut next.main)?;
        load_z80(&mut r, &mut next.ssio)?;
        let b = &mut next.board;
        load_sched(&mut r, &mut b.sched)?;
        r.fill(&mut b.nvram)?;
        r.fill(&mut b.spriteram)?;
        r.fill(&mut b.videoram)?;
        r.fill(&mut b.paletteram)?;
        for c in &mut b.palette {
            *c = r.u32()?;
        }
        b.cocktail_flip = r.bool()?;
        load_ctc(&mut r, &mut b.ctc)?;
        load_irq(&mut r, &mut b.main_irq)?;
        load_irq(&mut r, &mut b.ssio_irq)?;
        b.watchdog_counter = r.i32()?;
        b.watchdog_fired = r.bool()?;
        b.vblank_start = r.time()?;
        b.vblank_end = r.time()?;
        b.frame_number = r.u64()?;
        b.scantimer_first = r.bool()?;
        r.fill(&mut b.ssio_ram)?;
        r.fill(&mut b.ssio_data)?;
        b.ssio_status = r.u8()?;
        b.ssio_14024 = r.u8()?;
        b.sound.load(&mut r)?;
        r.fill(&mut b.ports)?;
        r.fill(&mut next.pens)?;
        for c in &mut next.pens_palette {
            *c = r.u32()?;
        }
        next.board.sound.split = if r.done().is_ok() {
            None
        } else {
            let at = (r.i32()?, r.i64()?);
            let mut duty = [[0u8; 3]; 2];
            for d in &mut duty {
                r.fill(d)?;
            }
            Some(crate::sound::GainSplit {
                at,
                duty,
                mute: r.u8()?,
            })
        };
        r.done()?;
        *self = next;
        Ok(())
    }

    /// A hash of the whole state (the snapshot's), for determinism checks.
    pub fn state_hash(&self) -> u64 {
        let h = blake3::hash(&self.snapshot());
        u64::from_le_bytes(h.as_bytes()[..8].try_into().expect("8 bytes"))
    }
}

fn save_z80(w: &mut Writer, z: &Z80) {
    for v in [
        z.prvpc, z.pc, z.sp, z.af, z.bc, z.de, z.hl, z.ix, z.iy, z.wz, z.af2, z.bc2, z.de2, z.hl2,
    ] {
        w.u16(v);
    }
    for v in [
        z.qt,
        z.q,
        z.r,
        z.r2,
        z.iff1,
        z.iff2,
        z.halt,
        z.im,
        z.i,
        z.nmi_state,
        z.irq_state,
        z.busack_state,
        z.rtemp,
    ] {
        w.u8(v);
    }
    w.bool(z.nmi_pending);
    w.bool(z.after_ei);
    w.bool(z.after_ldair);
    for v in [z.wait_state, z.busrq_state, z.icount, z.tmp_irq_vector] {
        w.i32(v);
    }
    for v in [z.ea, z.tadr, z.tdat, z.tdat2] {
        w.u16(v);
    }
    w.u32(z.m_ref);
}

fn load_z80(r: &mut Reader, z: &mut Z80) -> Result<(), String> {
    for v in [
        &mut z.prvpc,
        &mut z.pc,
        &mut z.sp,
        &mut z.af,
        &mut z.bc,
        &mut z.de,
        &mut z.hl,
        &mut z.ix,
        &mut z.iy,
        &mut z.wz,
        &mut z.af2,
        &mut z.bc2,
        &mut z.de2,
        &mut z.hl2,
    ] {
        *v = r.u16()?;
    }
    for v in [
        &mut z.qt,
        &mut z.q,
        &mut z.r,
        &mut z.r2,
        &mut z.iff1,
        &mut z.iff2,
        &mut z.halt,
        &mut z.im,
        &mut z.i,
        &mut z.nmi_state,
        &mut z.irq_state,
        &mut z.busack_state,
        &mut z.rtemp,
    ] {
        *v = r.u8()?;
    }
    z.nmi_pending = r.bool()?;
    z.after_ei = r.bool()?;
    z.after_ldair = r.bool()?;
    for v in [
        &mut z.wait_state,
        &mut z.busrq_state,
        &mut z.icount,
        &mut z.tmp_irq_vector,
    ] {
        *v = r.i32()?;
    }
    for v in [&mut z.ea, &mut z.tadr, &mut z.tdat, &mut z.tdat2] {
        *v = r.u16()?;
    }
    z.m_ref = r.u32()?;
    Ok(())
}

fn save_sched(w: &mut Writer, s: &Scheduler<What>) {
    w.u32(s.timers.len() as u32);
    for t in &s.timers {
        match t {
            None => w.bool(false),
            Some(t) => {
                w.bool(true);
                let (tag, arg) = t.what.code();
                w.u8(tag);
                w.u8(arg);
                w.i32(t.param);
                w.bool(t.enabled);
                w.bool(t.temporary);
                w.time(t.period);
                w.time(t.start);
                w.time(t.expire);
            }
        }
    }
    w.u32(s.list.len() as u32);
    for &h in &s.list {
        w.u32(h as u32);
    }
    w.time(s.basetime);
    w.i64(s.quantum);
    for e in &s.execs {
        w.time(e.localtime);
        w.i32(e.cycles_running);
        w.i32(e.cycles_stolen);
        w.u64(e.totalcycles);
        w.bool(e.suspended);
    }
}

fn load_sched(r: &mut Reader, s: &mut Scheduler<What>) -> Result<(), String> {
    let n = r.u32()? as usize;
    s.timers.clear();
    for _ in 0..n {
        if !r.bool()? {
            s.timers.push(None);
            continue;
        }
        let tag = r.u8()?;
        let arg = r.u8()?;
        s.timers.push(Some(crate::sched::Timer {
            what: What::from_code(tag, arg)?,
            param: r.i32()?,
            enabled: r.bool()?,
            temporary: r.bool()?,
            period: r.time()?,
            start: r.time()?,
            expire: r.time()?,
        }));
    }
    let n = r.u32()? as usize;
    s.list.clear();
    for _ in 0..n {
        let h = r.u32()? as usize;
        if !s.timers.get(h).is_some_and(Option::is_some) {
            return Err(format!("snapshot lists timer {h}, which it doesn't hold"));
        }
        s.list.push(h);
    }
    s.basetime = r.time()?;
    s.quantum = r.i64()?;
    for e in &mut s.execs {
        e.localtime = r.time()?;
        e.cycles_running = r.i32()?;
        e.cycles_stolen = r.i32()?;
        e.totalcycles = r.u64()?;
        e.suspended = r.bool()?;
    }
    s.executing = None;
    s.callback = None;
    Ok(())
}

fn save_ctc(w: &mut Writer, c: &Ctc) {
    w.u8(c.vector);
    w.u8(c.last_opcode);
    for ch in &c.channels {
        w.u16(ch.mode);
        w.u16(ch.tconst);
        w.u16(ch.down);
        w.bool(ch.extclk);
        w.u8(ch.int_state);
    }
}

fn load_ctc(r: &mut Reader, c: &mut Ctc) -> Result<(), String> {
    c.vector = r.u8()?;
    c.last_opcode = r.u8()?;
    for ch in &mut c.channels {
        ch.mode = r.u16()?;
        ch.tconst = r.u16()?;
        ch.down = r.u16()?;
        ch.extclk = r.bool()?;
        ch.int_state = r.u8()?;
    }
    Ok(())
}

fn save_irq(w: &mut Writer, l: &IrqLine<What>) {
    w.u32(l.queue.len() as u32);
    for &(s, v) in &l.queue {
        w.bool(s);
        w.i32(v);
    }
    w.i32(l.stored_vector);
    w.i32(l.curvector);
    w.bool(l.curstate);
}

fn load_irq(r: &mut Reader, l: &mut IrqLine<What>) -> Result<(), String> {
    let n = r.u32()? as usize;
    l.queue.clear();
    for _ in 0..n {
        let s = r.bool()?;
        l.queue.push((s, r.i32()?));
    }
    l.stored_vector = r.i32()?;
    l.curvector = r.i32()?;
    l.curstate = r.bool()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_map_to_active_low_ports() {
        let c = Controls {
            coin: true,
            tap: true,
            up: true,
            ..Controls::default()
        };
        assert_eq!(c.ports(), [0xfe, 0xe7, 0xff, 0xfb, 0xff]);
        assert_eq!(Controls::from_bits(c.bits()), c);
        assert_eq!(Controls::default().ports(), [0xff, 0xff, 0xff, 0xfb, 0xff]);
    }

    #[test]
    fn screen_timing_matches_mame_constants() {
        assert_eq!(FRAME_PERIOD, 33_333_333_333_333_333);
        assert_eq!(SCANTIME, 69_444_444_444_444);
        assert_eq!(PIXELTIME, 135_633_680_555);
    }
}
