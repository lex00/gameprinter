//! Two small programs that check when the PPU raises NMI.
//!
//! On the NES, the PPU's NMI output is the vblank flag AND the NMI enable bit
//! (bit 7 of $2000), and the CPU takes an NMI when that output goes from low to
//! high (nesdev wiki, "NMI" and "PPU registers"). So during vblank, with the
//! flag still set:
//!
//! - writing $2000 with bit 7 set while it is already set changes nothing and
//!   fires no NMI;
//! - changing bit 7 from 0 to 1 fires one NMI right away.
//!
//! [`build_rewrite`] checks the first: after the warmup it rewrites $80 to
//! $2000 in a tight loop, a few hundred times per vblank, and its handler
//! counts NMIs at [`addr::NMIS`]. Expect exactly one per frame.
//!
//! [`build_toggle`] checks the second. It runs the same loop, and its handler,
//! on the vblank NMI, turns bit 7 off and on again once. That 0-to-1 change
//! fires a second NMI, which nests inside the first and is counted at
//! [`addr::TOGGLED`]. Expect two NMIs per frame, one of them toggled.

use super::assemble;

/// Where the programs' RAM variables live.
pub mod addr {
    /// Every NMI taken, both programs.
    pub const NMIS: usize = 0x00;
    /// NMIs fired by the toggle in [`build_toggle`](super::build_toggle).
    pub const TOGGLED: usize = 0x01;
    /// 1 while [`build_toggle`](super::build_toggle)'s handler is toggling.
    pub const PHASE: usize = 0x02;
}

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
    0xA9, 0x80,       // $C00F loop: LDA #$80
    0x8D, 0x00, 0x20, //       STA $2000    NMI on, rewritten on every pass
    0x4C, 0x0F, 0xC0, //       JMP loop
];

#[rustfmt::skip]
const NMI_COUNT: &[u8] = &[
    0xE6, 0x00,       // $C017 INC $00      (A is untouched, so the loop always writes $80)
    0x40,             //       RTI
];

#[rustfmt::skip]
const NMI_TOGGLE: &[u8] = &[
    0x48,             // $C017 PHA          the loop's A must survive
    0xE6, 0x00,       //       INC $00
    0xA5, 0x02,       //       LDA $02
    0xD0, 0x16,       //       BNE nested   this NMI came from the toggle below
    0xE6, 0x02,       //       INC $02      phase 1: toggling
    0xA9, 0x00,       //       LDA #$00
    0x8D, 0x00, 0x20, //       STA $2000    NMI enable 1 -> 0, vblank flag still set
    0xA9, 0x80,       //       LDA #$80
    0x8D, 0x00, 0x20, //       STA $2000    0 -> 1 during vblank: one more NMI, now
    0xEA,             //       NOP          it is taken within an instruction or two,
    0xEA,             //       NOP          while the phase is still 1
    0xEA,             //       NOP
    0xEA,             //       NOP
    0xA9, 0x00,       //       LDA #$00
    0x85, 0x02,       //       STA $02      phase 0
    0x68,             //       PLA
    0x40,             //       RTI
    0xE6, 0x01,       // nested: INC $01
    0x68,             //       PLA
    0x40,             //       RTI
];

/// NMI enabled and $2000 rewritten with the same value in a tight loop.
pub fn build_rewrite() -> Vec<u8> {
    assemble(RESET, NMI_COUNT)
}

/// [`build_rewrite`], plus one 0-to-1 change of the NMI enable bit in every
/// vblank.
pub fn build_toggle() -> Vec<u8> {
    assemble(RESET, NMI_TOGGLE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Nes;
    use gp_evidence::input_log::InputFrame;
    use gp_evidence::manifest::PowerOn;

    /// Runs `frames` frames and returns, for each frame after the warmup,
    /// how much (NMIS, TOGGLED) grew during it.
    fn per_frame(rom: &[u8], frames: usize) -> Vec<(u8, u8)> {
        let mut nes = Nes::power_on(rom, PowerOn::default()).unwrap();
        let read = |nes: &Nes| (nes.wram()[addr::NMIS], nes.wram()[addr::TOGGLED]);
        let mut counts = Vec::new();
        for _ in 0..frames {
            nes.step(InputFrame::default()).unwrap();
            counts.push(read(&nes));
        }
        // The first frames wait for the PPU and enable NMI part way through.
        counts
            .windows(2)
            .skip(5)
            .map(|w| (w[1].0.wrapping_sub(w[0].0), w[1].1.wrapping_sub(w[0].1)))
            .collect()
    }

    #[test]
    fn layout_matches_the_listing() {
        assert_eq!(RESET.len(), 0x17, "NMI must start at $C017");
        assert_eq!(&RESET[0x0F..0x11], &[0xA9, 0x80], "loop: is at $C00F");
        let bne = 5;
        let nested = bne + 2 + NMI_TOGGLE[bne + 1] as usize;
        assert_eq!(&NMI_TOGGLE[nested..nested + 2], &[0xE6, 0x01]);
    }

    #[test]
    fn rewriting_an_enabled_nmi_fires_no_extra_nmi() {
        let deltas = per_frame(&build_rewrite(), 120);
        assert!(deltas.iter().all(|&(nmis, _)| nmis == 1), "{deltas:?}");
    }

    #[test]
    fn enabling_nmi_during_vblank_fires_once() {
        let deltas = per_frame(&build_toggle(), 120);
        assert!(deltas.iter().all(|&d| d == (2, 1)), "{deltas:?}");
    }

    #[test]
    fn a_handler_that_changes_a_makes_the_loop_toggle_nmi() {
        // The loop's LDA #$80 / STA $2000 with a handler that leaves its
        // count in A. An NMI taken between the two makes the STA write the
        // count, which turns NMI off while bit 7 of the count is clear; the
        // next pass turns it on again during vblank, a 0-to-1 change that
        // correctly fires a second NMI. So some frames count two NMIs until
        // the count reaches 128. That is what the first test program saw.
        #[rustfmt::skip]
        const NMI_CLOBBER: &[u8] = &[
            0xE6, 0x00, // $C017 INC $00
            0xA5, 0x00, //       LDA $00      A is now the count, not $80
            0x40,       //       RTI
        ];
        let rom = assemble(RESET, NMI_CLOBBER);
        let deltas: Vec<u8> = per_frame(&rom, 200).into_iter().map(|d| d.0).collect();
        assert!(deltas.iter().all(|&d| d == 1 || d == 2), "{deltas:?}");
        assert!(deltas[..80].contains(&2), "{deltas:?}");
        assert!(deltas[140..].iter().all(|&d| d == 1), "{deltas:?}");
    }
}
