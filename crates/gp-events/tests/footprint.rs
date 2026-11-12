//! The instruction-level pass on a small program that writes RAM in every
//! addressing mode, pushes and pops, calls a subroutine and takes NMIs: the
//! bytes it reports executed and written are exactly the ones the program
//! runs and writes, and no RAM byte changes without being reported.

use gp_events::{Footprint as Observer, Layout, RAM_BYTES};
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
    0xA9, 0x11,       // C009 LDA #$11      loop
    0x85, 0x10,       // C00B STA $10
    0xA2, 0x03,       // C00D LDX #$03
    0x95, 0x20,       // C00F STA $20,X     $23
    0x8D, 0x34, 0x03, // C011 STA $0334
    0x9D, 0x40, 0x03, // C014 STA $0340,X   $0343
    0xA0, 0x05,       // C017 LDY #$05
    0x99, 0x50, 0x03, // C019 STA $0350,Y   $0355
    0xA9, 0x00,       // C01C LDA #$00
    0x85, 0x30,       // C01E STA $30
    0xA9, 0x04,       // C020 LDA #$04
    0x85, 0x31,       // C022 STA $31       pointer $30 = $0400
    0x91, 0x30,       // C024 STA ($30),Y   $0405
    0x81, 0x2D,       // C026 STA ($2D,X)   $0400
    0xE6, 0x40,       // C028 INC $40
    0x0E, 0x00, 0x05, // C02A ASL $0500
    0x20, 0x40, 0xC0, // C02D JSR $C040
    0x8D, 0x00, 0x60, // C030 STA $6000     PRG RAM
    0x87, 0x50,       // C033 SAX $50       (unofficial)
    0xC7, 0x51,       // C035 DCP $51       (unofficial)
    0x4C, 0x09, 0xC0, // C037 JMP $C009
];

#[rustfmt::skip]
const SUB: &[u8] = &[
    0x48, // C040 PHA
    0x08, // C041 PHP
    0x28, // C042 PLP
    0x68, // C043 PLA
    0x60, // C044 RTS
];

#[rustfmt::skip]
const NMI: &[u8] = &[
    0xE6, 0x60, // C050 INC $60
    0x40,       // C052 RTI
];

fn rom() -> Vec<u8> {
    let mut prg = vec![0xEAu8; 0x4000];
    prg[..MAIN.len()].copy_from_slice(MAIN);
    prg[0x40..0x40 + SUB.len()].copy_from_slice(SUB);
    prg[0x50..0x50 + NMI.len()].copy_from_slice(NMI);
    prg[0x3FFA..0x3FFC].copy_from_slice(&0xC050u16.to_le_bytes());
    prg[0x3FFC..0x3FFE].copy_from_slice(&ORIGIN.to_le_bytes());
    prg[0x3FFE..0x4000].copy_from_slice(&0xC052u16.to_le_bytes());
    let mut rom = b"NES\x1a\x01\x00\x00\x00".to_vec();
    rom.resize(16, 0);
    rom.extend(prg);
    rom
}

#[test]
fn reports_exactly_what_runs_and_what_is_written() {
    let mut nes = Nes::power_on(&rom(), PowerOn::default()).unwrap();
    let layout = Layout::of(nes.bus()).unwrap();
    assert_eq!(layout.prg_rom, 0x4000);
    let mut obs = Observer::new(layout);
    obs.start(nes.bus_mut_for_observers());
    let mut before = nes.wram().to_vec();
    let mut missed = Vec::new();
    for _ in 0..12 {
        nes.step_with(InputFrame::default(), |bus| {
            obs.before(bus);
            // Everything the last instruction (and an interrupt after it)
            // changed must be reported by now.
            let now = &bus.wram[0..RAM_BYTES];
            if before[..] != *now {
                for (i, (old, new)) in before.iter().zip(now).enumerate() {
                    if old != new && !obs.ram_written.contains(i) {
                        missed.push(i);
                    }
                }
                before.copy_from_slice(now);
            }
        })
        .unwrap();
        obs.interrupt(nes.bus_mut_for_observers());
    }
    assert!(missed.is_empty(), "writes not reported: {missed:x?}");

    let offsets = |r: std::ops::RangeInclusive<u16>| r.map(|a| usize::from(a - ORIGIN));
    let executed: Vec<usize> = offsets(0xC000..=0xC039)
        .chain(offsets(0xC040..=0xC044))
        .chain(offsets(0xC050..=0xC052))
        .collect();
    let got: Vec<usize> = (0..0x4000).filter(|&i| obs.executed.contains(i)).collect();
    assert_eq!(got, executed);
    assert!(obs.starts.contains(0x37) && !obs.starts.contains(0x38));
    assert_eq!(obs.windows[0], 1 << (0xC000 >> 10), "ran at $C000 only");
    assert_eq!(obs.outside_rom, 0);

    let mut want: Vec<usize> = vec![
        0x10, 0x23, 0x30, 0x31, 0x40, 0x50, 0x51, 0x60, 0x334, 0x343, 0x355, 0x400, 0x405, 0x500,
    ];
    let written: Vec<usize> = (0..RAM_BYTES)
        .filter(|&i| obs.ram_written.contains(i))
        .collect();
    let stack: Vec<usize> = written
        .iter()
        .copied()
        .filter(|&i| (0x100..0x200).contains(&i))
        .collect();
    // JSR, PHA and PHP push $01FF to $01FC; an NMI pushes three bytes below
    // wherever the stack is when it arrives.
    assert!(stack.iter().all(|&i| i >= 0x1F9), "{stack:x?}");
    for i in 0x1FC..=0x1FF {
        assert!(stack.contains(&i), "{stack:x?}");
    }
    want.extend(&stack);
    want.sort_unstable();
    assert_eq!(written, want);
    if layout.prg_ram > 0 {
        assert_eq!(obs.prg_ram_written.count(), 1);
        assert!(obs.prg_ram_written.contains(0));
    }
}
