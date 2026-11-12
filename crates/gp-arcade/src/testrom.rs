//! Made-up sets for tests: no game's bytes, only the programs a test
//! writes. They run on the MCR board like any set, so the tools that read
//! an arcade run can be tested without a romset on the machine.

use crate::Romset;

/// A set whose main and sound programs are `main` and `ssio`, from address
/// 0, with zeros after them and blank graphics.
pub fn romset(main: &[u8], ssio: &[u8]) -> Romset {
    let mut maincpu = vec![0u8; 0x10000];
    maincpu[..main.len()].copy_from_slice(main);
    let mut ssiocpu = vec![0u8; 0x10000];
    ssiocpu[..ssio.len()].copy_from_slice(ssio);
    Romset {
        name: "test",
        identity: "0".repeat(64),
        maincpu,
        ssiocpu,
        gfx1: vec![0; 0x8000],
        gfx2: vec![0; 0x20000],
        extras: Vec::new(),
        ssio_prom: None,
    }
}

/// Both programs loop at 0 (`JR $`).
pub fn looping() -> Romset {
    romset(&[0x18, 0xfe], &[0x18, 0xfe])
}

/// A main program that does a little of everything a tool reads, every
/// frame, with no interrupts: it reads input port 1 into work RAM at
/// `e000`, counts frames at `e001`, steps a shift register at `e002` (a
/// made-up random generator: `ld a,(e002) / rlca / xor 1Ch` when the
/// carry is set; an even constant keeps it from ever reaching zero),
/// writes the sound latch and feeds the watchdog, then
/// jumps through a table with `jp (hl)`. The sound program writes AY-8910
/// register 7 with whatever is in latch 0.
pub fn busy() -> Romset {
    let main: &[u8] = &[
        // 0000: ld sp,e7f0
        0x31, 0xf0, 0xe7, //
        // 0003: in a,(01) ; ld (e000),a
        0xdb, 0x01, 0x32, 0x00, 0xe0, //
        // 0008: ld hl,e001 ; inc (hl)
        0x21, 0x01, 0xe0, 0x34, //
        // 000c: ld a,(e002) ; rlca ; jr nc,+2 ; xor 1c ; ld (e002),a
        0x3a, 0x02, 0xe0, 0x07, 0x30, 0x02, 0xee, 0x1c, 0x32, 0x02, 0xe0, //
        // 0017: out (1c),a ; out (e0),a
        0xd3, 0x1c, 0xd3, 0xe0, //
        // 001b: and 1 ; add a,a ; ld e,a ; ld d,0 ; ld hl,0040 ; add hl,de
        0xe6, 0x01, 0x87, 0x5f, 0x16, 0x00, 0x21, 0x40, 0x00, 0x19, //
        // 0025: ld a,(hl) ; inc hl ; ld h,(hl) ; ld l,a ; jp (hl)
        0x7e, 0x23, 0x66, 0x6f, 0xe9, //
        // 002a..003f: nops, then the table at 0040: 0050, 0058
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
        0x50, 0x00, 0x58, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
        // 0050: ld a,1 ; ld (e003),a ; jr 0003 (back to the loop)
        0x3e, 0x01, 0x32, 0x03, 0xe0, 0x18, 0xac, 0, //
        // 0058: ld a,2 ; ld (e003),a ; jr 0003
        0x3e, 0x02, 0x32, 0x03, 0xe0, 0x18, 0xa4,
    ];
    let ssio: &[u8] = &[
        // 0000: ld a,07 ; ld (a000),a  (select register 7 of the first AY)
        0x3e, 0x07, 0x32, 0x00, 0xa0, //
        // 0005: ld a,(9000) ; ld (a002),a ; jr 0005
        0x3a, 0x00, 0x90, 0x32, 0x02, 0xa0, 0x18, 0xf8,
    ];
    romset(main, ssio)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Controls, Tapper};

    /// The busy program runs as its comments say.
    #[test]
    fn the_busy_program_runs() {
        let mut t = Tapper::power_on(&busy());
        t.step(Controls::default());
        t.step(Controls {
            tap: true,
            ..Controls::default()
        });
        assert_eq!(t.peek(0xe000), 0xef, "port 1 with tap held");
        assert!(t.peek(0xe001) > 2, "the loop ran many times a frame");
        assert!(matches!(t.peek(0xe003), 1 | 2), "the table jump landed");
    }

    /// With the event log on, a frame of the busy program logs each kind
    /// it does, in time order, and the log changes no state.
    #[test]
    fn the_busy_program_logs_its_events() {
        use crate::events::Happened;
        let mut quiet = Tapper::power_on(&busy());
        let mut t = Tapper::power_on(&busy());
        t.set_events(true);
        for _ in 0..3 {
            quiet.step(Controls::default());
            t.take_events();
            t.step(Controls::default());
        }
        assert_eq!(quiet.state_hash(), t.state_hash());
        let ev = t.take_events();
        for what in [
            Happened::PortRead,
            Happened::SoundLatch,
            Happened::Watchdog,
            Happened::AyWrite,
        ] {
            assert!(ev.iter().any(|e| e.what == what), "{what:?}");
        }
        let ay = ev.iter().find(|e| e.what == Happened::AyWrite).unwrap();
        assert_eq!(ay.addr, 7, "register 7 of the first chip");
        let lines: Vec<_> = ev
            .iter()
            .filter(|e| e.what == Happened::PortRead)
            .map(|e| (e.line, e.dot))
            .collect();
        assert!(lines.windows(2).all(|w| w[0] <= w[1]), "in time order");
        assert!(t.take_events().is_empty(), "taken");
    }
}
