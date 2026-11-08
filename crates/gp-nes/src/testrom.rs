//! A tiny NES program of our own, assembled by hand, so the runner can be
//! tested in the open tier with no commercial ROM.
//!
//! On reset it waits two vertical blanks for the PPU to warm up, enables NMI
//! once, and idles. On every NMI it increments a frame counter at $00 and, on
//! even counts only, strobes both controllers and reads controller 1 into $01
//! and controller 2 into $03 (A ends up in bit 7 of each). It then stores
//! $00 + $01 at $02. So input changes RAM, and every other frame is a lag
//! frame with no controller read.
//!
//! [`build_port2_alone`] is the same program with one byte changed: on odd
//! counts that are 3 more than a multiple of 4 it reads controller 2 alone, so
//! a frame can be polled through $4017 only. The [`nmi`] module holds two more
//! programs, for how the PPU raises NMIs.

pub mod nmi;

/// Where the program's RAM variables live.
pub mod addr {
    pub const FRAMES: usize = 0x00;
    pub const BUTTONS: usize = 0x01;
    pub const MIX: usize = 0x02;
    /// Controller 2, bits in the same order as [`BUTTONS`].
    pub const BUTTONS2: usize = 0x03;
}

const PRG_SIZE: usize = 16 * 1024;
const ORIGIN: u16 = 0xC000;

// NMI is enabled once, after the warmup wait. Rewriting $2000 with the same
// value would be harmless (tests in the nmi module check that), but a handler
// that changes A can make a rewrite loop write a value with NMI off, and the
// next pass's 0-to-1 change then fires a second NMI in the same vblank.
#[rustfmt::skip]
const RESET: &[u8] = &[
    0x78,             // $C000 SEI
    0xD8,             //       CLD
    0xA2, 0xFF,       //       LDX #$FF
    0x9A,             //       TXS
    0x2C, 0x02, 0x20, // $C005 vw1: BIT $2002
    0x10, 0xFB,       //       BPL vw1      wait for the first vblank
    0x2C, 0x02, 0x20, // $C00A vw2: BIT $2002
    0x10, 0xFB,       //       BPL vw2      and the second
    0xA9, 0x80,       // $C00F LDA #$80
    0x8D, 0x00, 0x20, //       STA $2000    enable NMI, once
    0x4C, 0x14, 0xC0, // $C014 idle: JMP idle
];

/// Offset in [`NMI`] of the mask that picks the odd counts that read
/// controller 2 alone. It is 0 in [`build`], so every odd count lags.
const PORT2_ALONE_MASK: usize = 40;

#[rustfmt::skip]
const NMI: &[u8] = &[
    0xE6, 0x00,       // $C017 INC $00
    0xA5, 0x00,       //       LDA $00
    0x29, 0x01,       //       AND #$01
    0xD0, 0x1D,       //       BNE odd      odd counts skip controller 1
    0xA9, 0x01,       //       LDA #1
    0x8D, 0x16, 0x40, //       STA $4016    strobe on (both controllers)
    0xA9, 0x00,       //       LDA #0
    0x8D, 0x16, 0x40, //       STA $4016    strobe off
    0xA2, 0x08,       //       LDX #8
    0xAD, 0x16, 0x40, // rd:   LDA $4016
    0x4A,             //       LSR A
    0x26, 0x01,       //       ROL $01
    0xAD, 0x17, 0x40, //       LDA $4017
    0x4A,             //       LSR A
    0x26, 0x03,       //       ROL $03
    0xCA,             //       DEX
    0xD0, 0xF1,       //       BNE rd
    0xF0, 0x1B,       //       BEQ mix      always taken: X is 0
    0xA5, 0x00,       // odd:  LDA $00
    0x29, 0x00,       //       AND #mask    0 in build(), 2 in build_port2_alone()
    0xF0, 0x15,       //       BEQ mix      lag frame: no controller read
    0xA9, 0x01,       //       LDA #1
    0x8D, 0x16, 0x40, //       STA $4016    strobe on
    0xA9, 0x00,       //       LDA #0
    0x8D, 0x16, 0x40, //       STA $4016    strobe off
    0xA2, 0x08,       //       LDX #8
    0xAD, 0x17, 0x40, // rd2:  LDA $4017    controller 2 alone
    0x4A,             //       LSR A
    0x26, 0x03,       //       ROL $03
    0xCA,             //       DEX
    0xD0, 0xF7,       //       BNE rd2
    0xA5, 0x00,       // mix:  LDA $00
    0x18,             //       CLC
    0x65, 0x01,       //       ADC $01
    0x85, 0x02,       //       STA $02
    0x40,             //       RTI
];

/// The source name baselines use for this program (`source = "gp-testrom"`),
/// which `gp baseline check` runs without any ROM files.
pub const SOURCE_ID: &str = "gp-testrom";

/// The program as an iNES image: mapper 0, 16KB PRG, 8KB CHR RAM.
pub fn build() -> Vec<u8> {
    assemble(RESET, NMI)
}

/// [`build`], but counts of 3, 7, 11 and so on read controller 2 alone. Port
/// 1 is still read on even counts only; a quarter of frames are lag frames.
pub fn build_port2_alone() -> Vec<u8> {
    let mut nmi = NMI.to_vec();
    nmi[PORT2_ALONE_MASK] = 0x02;
    assemble(RESET, &nmi)
}

/// Places `reset` at $C000 with the NMI handler right after it and an IRQ
/// handler (a lone RTI) after that, and wraps it as a mapper 0 iNES image.
fn assemble(reset: &[u8], nmi: &[u8]) -> Vec<u8> {
    let mut prg = vec![0xEAu8; PRG_SIZE]; // NOP fill
    prg[..reset.len()].copy_from_slice(reset);
    let nmi_at = reset.len();
    prg[nmi_at..nmi_at + nmi.len()].copy_from_slice(nmi);
    let irq_at = nmi_at + nmi.len();
    prg[irq_at] = 0x40; // RTI

    let vector = |offset: usize| (ORIGIN + offset as u16).to_le_bytes();
    prg[0x3FFA..0x3FFC].copy_from_slice(&vector(nmi_at));
    prg[0x3FFC..0x3FFE].copy_from_slice(&vector(0));
    prg[0x3FFE..0x4000].copy_from_slice(&vector(irq_at));

    let mut rom = b"NES\x1a\x01\x00\x00\x00".to_vec();
    rom.resize(16, 0);
    rom.extend(prg);
    rom
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Nes;
    use gp_evidence::input_log::{InputFrame, button};
    use gp_evidence::manifest::PowerOn;

    fn pads(p1: u8, p2: u8) -> InputFrame {
        InputFrame {
            polled: true,
            pads: [p1, p2, 0, 0],
            reset: false,
        }
    }

    /// Powers on `rom` and runs `frames` frames holding `p1` and `p2`.
    fn held(rom: &[u8], frames: usize, p1: u8, p2: u8) -> Nes {
        let mut nes = Nes::power_on(rom, PowerOn::default()).unwrap();
        for _ in 0..frames {
            nes.step(pads(p1, p2)).unwrap();
        }
        nes
    }

    #[test]
    fn layout_matches_the_listing() {
        assert_eq!(
            RESET.len(),
            0x17,
            "NMI must start at $C017 as the listing says"
        );
        let rom = build();
        assert_eq!(rom.len(), 16 + PRG_SIZE);
        let prg = &rom[16..];
        assert_eq!(&prg[0x3FFA..0x3FFE], &[0x17, 0xC0, 0x00, 0xC0]);
        // The idle loop jumps to itself.
        assert_eq!(&RESET[0x14..0x17], &[0x4C, 0x14, 0xC0]);
        // Every branch lands where the listing's labels say.
        let target = |at: usize| (at as isize + 2 + NMI[at + 1] as i8 as isize) as usize;
        let (rd, odd, rd2, mix) = (20, 37, 55, 64);
        assert_eq!(&NMI[rd..rd + 3], &[0xAD, 0x16, 0x40]);
        assert_eq!(&NMI[rd2..rd2 + 3], &[0xAD, 0x17, 0x40]);
        assert_eq!(&NMI[mix..mix + 2], &[0xA5, 0x00]);
        assert_eq!(target(6), odd);
        assert_eq!(target(33), rd);
        assert_eq!(target(35), mix);
        assert_eq!(target(41), mix);
        assert_eq!(target(62), rd2);
        assert_eq!(NMI[PORT2_ALONE_MASK - 1], 0x29, "the mask is AND's operand");
        let alone = build_port2_alone();
        let diff: Vec<usize> = (0..rom.len()).filter(|&i| rom[i] != alone[i]).collect();
        assert_eq!(diff, [16 + RESET.len() + PORT2_ALONE_MASK]);
        let img = gp_evidence::source::identify_nes(&rom).unwrap();
        assert_eq!(img.header.mapper, 0);
        assert_eq!(img.header.chr_rom_bytes, 0);
    }

    const ORDER: [u8; 8] = [
        button::A,
        button::B,
        button::SELECT,
        button::START,
        button::UP,
        button::DOWN,
        button::LEFT,
        button::RIGHT,
    ];

    #[test]
    fn each_port_2_button_reaches_its_own_bit() {
        // $4017 returns buttons in the same order as $4016, A first.
        for rom in [build(), build_port2_alone()] {
            for (i, b) in ORDER.into_iter().enumerate() {
                let nes = held(&rom, 10, 0, b);
                assert_eq!(nes.wram()[addr::BUTTONS2], 0x80 >> i, "button {b:#04x}");
            }
        }
    }

    #[test]
    fn ports_do_not_bleed_into_each_other() {
        for rom in [build(), build_port2_alone()] {
            let one = held(&rom, 20, button::A | button::UP, 0);
            assert_eq!(one.wram()[addr::BUTTONS], 0b1000_1000);
            assert_eq!(one.wram()[addr::BUTTONS2], 0);
            let two = held(&rom, 20, 0, button::B | button::RIGHT);
            assert_eq!(two.wram()[addr::BUTTONS], 0);
            assert_eq!(two.wram()[addr::BUTTONS2], 0b0100_0001);
            let both = held(&rom, 20, button::START, button::SELECT);
            assert_eq!(both.wram()[addr::BUTTONS], 0b0001_0000);
            assert_eq!(both.wram()[addr::BUTTONS2], 0b0010_0000);
        }
    }

    #[test]
    fn a_frame_that_reads_only_port_2_is_polled() {
        let mut nes = Nes::power_on(&build_port2_alone(), PowerOn::default()).unwrap();
        let mut seen = [0usize; 4];
        for f in 0..80 {
            // Press a different button on each port every frame, so each
            // frame shows which ports it read.
            let (p1, p2) = (ORDER[f % 8], ORDER[(f + 3) % 8]);
            let polled = nes.step(pads(p1, p2)).unwrap().polled;
            let w = nes.wram();
            let count = w[addr::FRAMES];
            if count < 2 {
                continue; // NMI isn't on yet
            }
            // A read this frame leaves this frame's button; A is bit 7.
            let read1 = w[addr::BUTTONS] == 0x80 >> (f % 8);
            let read2 = w[addr::BUTTONS2] == 0x80 >> ((f + 3) % 8);
            let expected = match count % 4 {
                0 | 2 => (true, true),
                3 => (false, true),
                _ => (false, false),
            };
            assert_eq!(
                (read1, read2, polled),
                (expected.0, expected.1, expected.0 || expected.1),
                "frame {f}, count {count}"
            );
            seen[count as usize % 4] += 1;
        }
        assert!(seen.iter().all(|&n| n > 10), "{seen:?}");
    }
}
