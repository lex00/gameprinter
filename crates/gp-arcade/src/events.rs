//! What happened inside a frame, for recordings and the tools that read
//! them: the main CPU's input port reads, its writes to the sound board's
//! latch and to the watchdog, the sound CPU's AY-8910 register writes, and
//! both CPUs' interrupts (the main CPU's come from its CTC).
//!
//! Off by default ([`crate::Tapper::set_events`]), and never part of the
//! state: a run that logs them reaches the same states as one that doesn't.
//!
//! A position is where the screen's beam was when it happened, counted from
//! the start of the frame (the VBLANK that ended the frame before): `line`
//! is 69.4 us of emulated time (one of the 480 lines of a 30 Hz frame) and
//! `dot` one 512th of a line. A CPU can run a few cycles past the frame's
//! end before the scheduler stops it, so the last line is 480.

use crate::attotime::Attotime;

/// What kind of thing happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Happened {
    /// The main CPU read an input port (0 to 4, through the SSIO board);
    /// `addr` is the port as the CPU addressed it, `value` what it read.
    PortRead,
    /// The main CPU wrote one of the SSIO board's four latches; `addr` is
    /// the latch, `value` the byte.
    SoundLatch,
    /// The main CPU fed the watchdog (port E0); `addr` is the port.
    Watchdog,
    /// The sound CPU wrote an AY-8910 register; `addr` is the chip (0 or 1)
    /// times 256 plus the register, `value` the byte.
    AyWrite,
    /// The main CPU took an interrupt from its CTC; `addr` is the
    /// handler's first instruction, `value` the vector the CTC gave (its
    /// channel is bits 1 and 2: channel 2 counts the lines, channel 3 the
    /// frames).
    Irq,
    /// The sound CPU took its interrupt (the SSIO's 14024 counter); `addr`
    /// is the handler's first instruction.
    SoundIrq,
}

/// One thing that happened, where in the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardEvent {
    pub what: Happened,
    pub line: u16,
    pub dot: u16,
    pub addr: u16,
    pub value: u8,
}

/// Lines in a frame's positions, the last one for a CPU's overshoot.
pub const LINES: u16 = 481;
/// Dots in a line.
pub const DOTS: u16 = 512;

/// Where `at` falls in a frame that started at `start`.
pub fn position(start: Attotime, at: Attotime) -> (u16, u16) {
    let scantime = crate::mcr::SCANTIME;
    let offset = if at < start {
        0
    } else {
        (at - start).as_attoseconds()
    };
    let last = i64::from(LINES) - 1;
    if offset >= last * scantime + scantime {
        // Far past the frame's end: the overshoot, kept at its last dot.
        return (LINES - 1, DOTS - 1);
    }
    let line = offset / scantime;
    let dot = (offset % scantime) * i64::from(DOTS) / scantime;
    (line as u16, dot as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_from_the_frame_start() {
        let start = Attotime::new(1, 0);
        assert_eq!(position(start, start), (0, 0));
        let line = Attotime::new(0, crate::mcr::SCANTIME);
        assert_eq!(position(start, start + line), (1, 0));
        let half = Attotime::new(0, crate::mcr::SCANTIME / 2);
        assert_eq!(position(start, start + line + half), (1, 256));
        // Before the start (a CPU behind the scheduler's base time) is 0.
        assert_eq!(position(start, Attotime::new(0, 5)), (0, 0));
        // Far past the end stays in the last line.
        assert_eq!(position(start, Attotime::new(3, 0)).0, LINES - 1);
    }
}
