//! The collector through the cartridge shim against the host observer, on a
//! small program with every kind of transfer (calls and returns, a jump
//! through a pointer, a return to a pushed address, taken and untaken
//! branches, `brk`, NMIs) and a reset: the same L0 and L1, and the same
//! console state as a run nobody watches.

use gp_events::{Collector, Layout, Levels};
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_nes::Nes;

const ORIGIN: u16 = 0xC000;

#[rustfmt::skip]
const MAIN: &[u8] = &[
    0x78,             // C000 SEI
    0xA2, 0xFF,       // C001 LDX #$FF
    0x9A,             // C003 TXS
    0xA9, 0x80,       // C004 LDA #$80
    0x8D, 0x00, 0x20, // C006 STA $2000     NMI on
    0xA9, 0x60,       // C009 LDA #$60      loop: pointer $30 = $C060
    0x85, 0x30,       // C00B STA $30
    0xA9, 0xC0,       // C00D LDA #$C0
    0x85, 0x31,       // C00F STA $31
    0x20, 0x40, 0xC0, // C011 JSR $C040
    0xE6, 0x10,       // C014 INC $10
    0xA5, 0x10,       // C016 LDA $10
    0x29, 0x01,       // C018 AND #$01
    0xF0, 0x03,       // C01A BEQ +3        every other pass
    0x6C, 0x30, 0x00, // C01C JMP ($0030)   to C060
    0xA9, 0xC0,       // C01F LDA #$C0      push $C06F, rts to C070
    0x48,             // C021 PHA
    0xA9, 0x6F,       // C022 LDA #$6F
    0x48,             // C024 PHA
    0x60,             // C025 RTS
];

#[rustfmt::skip]
const SUB: &[u8] = &[
    0x85, 0x20,       // C040 STA $20
    0x8D, 0x00, 0x60, // C042 STA $6000
    0x60,             // C045 RTS
];

#[rustfmt::skip]
const NMI: &[u8] = &[
    0xE6, 0x60, // C050 INC $60
    0x40,       // C052 RTI
];

#[rustfmt::skip]
const TAIL: &[u8] = &[
    0x00, 0xEA,       // C060 BRK (to the IRQ vector, C052: RTI back to C062)
    0x4C, 0x09, 0xC0, // C062 JMP $C009
];

#[rustfmt::skip]
const PUSHED: &[u8] = &[
    0x4C, 0x09, 0xC0, // C070 JMP $C009
];

fn rom() -> Vec<u8> {
    let mut prg = vec![0xEAu8; 0x4000];
    prg[..MAIN.len()].copy_from_slice(MAIN);
    prg[0x40..0x40 + SUB.len()].copy_from_slice(SUB);
    prg[0x50..0x50 + NMI.len()].copy_from_slice(NMI);
    prg[0x60..0x60 + TAIL.len()].copy_from_slice(TAIL);
    prg[0x70..0x70 + PUSHED.len()].copy_from_slice(PUSHED);
    prg[0x3FFA..0x3FFC].copy_from_slice(&0xC050u16.to_le_bytes());
    prg[0x3FFC..0x3FFE].copy_from_slice(&ORIGIN.to_le_bytes());
    prg[0x3FFE..0x4000].copy_from_slice(&0xC052u16.to_le_bytes());
    let mut rom = b"NES\x1a\x01\x00\x00\x00".to_vec();
    rom.resize(16, 0);
    rom.extend(prg);
    rom
}

/// 120 frames with a reset at frame 50, watched one way or the other (or
/// not at all): the levels and the console's state at the end.
fn run(watch: Option<bool>) -> (Option<Levels>, Vec<u8>) {
    let mut nes = Nes::power_on(&rom(), PowerOn::default()).unwrap();
    let mut c = Collector::new(Layout::of(nes.bus()).unwrap(), true);
    c.start(nes.bus_mut_for_observers());
    if watch == Some(true) {
        c.attach(nes.bus_mut_for_observers());
    }
    for f in 0..120 {
        let input = InputFrame {
            polled: true,
            pads: [0; 4],
            reset: f == 50,
        };
        match watch {
            Some(_) => {
                c.step(&mut nes, input).unwrap();
            }
            None => {
                nes.step(input).unwrap();
            }
        }
    }
    (watch.map(|_| Levels::of(&c, 120)), nes.snapshot().unwrap())
}

#[test]
fn the_shim_keeps_the_levels_the_host_observer_does() {
    let (host, host_state) = run(Some(false));
    let (shim, shim_state) = run(Some(true));
    let (_, plain_state) = run(None);
    let (host, shim) = (host.unwrap(), shim.unwrap());
    assert_eq!(host, shim);
    assert_eq!(host_state, plain_state);
    assert_eq!(shim_state, plain_state);
    // Every kind the program makes is there.
    let kinds: std::collections::BTreeSet<_> = shim.edges.iter().map(|e| e.kind).collect();
    use gp_events::EdgeKind::*;
    for k in [
        Jsr,
        Rts,
        RtsJump,
        Rti,
        Jmp,
        JmpIndirect,
        Branch,
        Fall,
        Interrupt,
        Brk,
        Reset,
    ] {
        assert!(kinds.contains(&k), "no {k:?} edge: {kinds:?}");
    }
    assert!(shim.footprint.prg_ram_written.count() > 0);
}
