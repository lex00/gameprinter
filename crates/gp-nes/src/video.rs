//! The picture a frame produced, for `gp video`. Reading it changes nothing
//! about the console, so it needs no `BEHAVIOUR_VERSION` bump. The PPU
//! draws only while video rendering is on ([`Nes::set_video`]).

use tetanes_core::ppu::{self, Ppu};

use crate::Nes;

/// Visible pixels per scanline.
pub const WIDTH: usize = ppu::size::WIDTH as usize;
/// Visible scanlines.
pub const HEIGHT: usize = ppu::size::HEIGHT as usize;

impl Nes {
    /// The visible picture of the last frame as RGB, 3 bytes a pixel, row by
    /// row, in `tetanes-core`'s NTSC palette with no filter. A frame ends at
    /// the start of vertical blank, so all 240 visible scanlines are drawn.
    /// Black while video rendering is off.
    pub fn frame_rgb(&self, out: &mut [u8; WIDTH * HEIGHT * 3]) {
        let palette = Ppu::NTSC_PALETTE;
        for (pixel, rgb) in self
            .deck
            .frame_buffer_raw()
            .iter()
            .zip(out.as_chunks_mut::<3>().0.iter_mut())
        {
            // Six bits of colour and three of emphasis: 512 entries.
            let i = usize::from(*pixel & 0x1ff) * 3;
            rgb.copy_from_slice(&palette[i..i + 3]);
        }
    }
}

impl Nes {
    /// The last frame's colors as NES color numbers (0 to 63, without
    /// emphasis), row by row: for a caller that draws them in its own
    /// palette, such as the screen measure, which compares them with a
    /// model drawn in the generator's.
    pub fn frame_colors(&self) -> Vec<u8> {
        self.deck
            .frame_buffer_raw()
            .iter()
            .map(|p| (*p & 0x3f) as u8)
            .collect()
    }

    /// The sprite table (OAM) as it stands: 64 sprites of 4 bytes, `y`,
    /// tile, attributes, `x`. Reading it changes nothing.
    pub fn oam(&self) -> [u8; 256] {
        let ppu = &self.deck.bus().ppu;
        std::array::from_fn(|i| ppu.oamdata[i])
    }

    /// The backdrop's color as it stands: palette RAM's first entry, an NES
    /// color number (0 to 63). Reading it changes nothing.
    pub fn backdrop(&self) -> u8 {
        self.deck.bus().ppu_bus_peek(0x3F00) & 0x3f
    }

    /// Whether sprites are 8 by 16.
    pub fn tall_sprites(&self) -> bool {
        self.deck.bus().ppu.ctrl_spr_height == 16
    }

    /// Which pixels of the last frame the sprites cover, row by row: a
    /// pixel is set where a sprite in OAM draws an opaque pixel (its
    /// pattern isn't 0 there), with the PPU's limit of eight sprites a
    /// scanline and its left-edge clipping. A sprite behind the background
    /// still counts, since its pixels are the sprite layer's. The OAM and
    /// pattern tables are read as they stand at the end of the frame,
    /// which is what the frame was drawn with in a game that writes OAM in
    /// vertical blank. For the screen measure's sprite layer
    /// (docs/interpreter.md, "The screen measure"). Reading it changes
    /// nothing.
    pub fn sprite_mask(&self) -> Vec<bool> {
        let mut out = vec![false; WIDTH * HEIGHT];
        let bus = self.deck.bus();
        let ppu = &bus.ppu;
        if !ppu.show_spr() {
            return out;
        }
        let tall = ppu.ctrl_spr_height == 16;
        let height = if tall { 16 } else { 8 };
        let left = if ppu.show_left_spr() { 0 } else { 8 };
        let mut on_line = [0u8; HEIGHT];
        for s in 0..64 {
            let o = &ppu.oamdata[s * 4..s * 4 + 4];
            let (y, tile, attr, x) = (usize::from(o[0]) + 1, o[1], o[2], usize::from(o[3]));
            for row in 0..height {
                let line = y + row;
                if line >= HEIGHT {
                    break;
                }
                if on_line[line] >= 8 {
                    continue;
                }
                on_line[line] += 1;
                let r = if attr & 0x80 != 0 {
                    height - 1 - row
                } else {
                    row
                };
                let (table, t) = if tall {
                    (
                        u16::from(tile & 1) * 0x1000,
                        u16::from(tile & 0xfe) + (r / 8) as u16,
                    )
                } else {
                    (ppu.ctrl_spr_select, u16::from(tile))
                };
                let addr = table + t * 16 + (r % 8) as u16;
                let lo = bus.ppu_bus_peek(addr);
                let hi = bus.ppu_bus_peek(addr + 8);
                for col in 0..8 {
                    let px = x + col;
                    if px >= WIDTH || px < left {
                        continue;
                    }
                    let bit = if attr & 0x40 != 0 { col } else { 7 - col };
                    if (lo >> bit) & 1 != 0 || (hi >> bit) & 1 != 0 {
                        out[line * WIDTH + px] = true;
                    }
                }
            }
        }
        out
    }
}

/// The 64 NES colors as RGB, without emphasis: the first 64 entries of the
/// palette [`Nes::frame_rgb`] draws with, so a tile drawn from palette
/// numbers matches the picture `gp video` writes.
pub fn colors() -> [[u8; 3]; 64] {
    let mut out = [[0u8; 3]; 64];
    for (c, rgb) in out
        .iter_mut()
        .zip(Ppu::NTSC_PALETTE.as_chunks::<3>().0.iter())
    {
        *c = *rgb;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrom;
    use gp_evidence::input_log::InputFrame;
    use gp_evidence::manifest::PowerOn;

    #[test]
    fn reading_the_picture_changes_nothing() {
        let rom = testrom::build();
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        let mut other = Nes::power_on(&rom, PowerOn::default()).unwrap();
        nes.set_video(true);
        let mut rgb = Box::new([0u8; WIDTH * HEIGHT * 3]);
        for _ in 0..12 {
            nes.step(InputFrame::default()).unwrap();
            other.step(InputFrame::default()).unwrap();
            nes.frame_rgb(&mut rgb);
        }
        assert_eq!(nes.state_hash().unwrap(), other.state_hash().unwrap());
        // The test program leaves rendering off, so the PPU draws its
        // backdrop colour everywhere, and it's a palette entry.
        let first = [rgb[0], rgb[1], rgb[2]];
        assert!(rgb.as_chunks::<3>().0.iter().all(|p| *p == first));
        assert!(Ppu::NTSC_PALETTE.as_chunks::<3>().0.contains(&first));
    }

    /// The original and the rebuild draw a color number in the same RGB:
    /// the emulator's table without emphasis is the one every runtime that
    /// draws a model reads (`gp_present::palette::NES`), and a frame's
    /// pixels are those colors.
    #[test]
    fn the_original_draws_the_rebuilds_colors() {
        assert_eq!(colors(), gp_present::palette::NES);
        let rom = testrom::build();
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        nes.set_video(true);
        let mut rgb = Box::new([0u8; WIDTH * HEIGHT * 3]);
        for _ in 0..12 {
            nes.step(InputFrame::default()).unwrap();
        }
        nes.frame_rgb(&mut rgb);
        for (c, px) in nes.frame_colors().iter().zip(rgb.as_chunks::<3>().0) {
            assert_eq!(gp_present::palette::NES[usize::from(*c)], *px);
        }
    }
}
