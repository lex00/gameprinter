// license:BSD-3-Clause
// copyright-holders:Aaron Giles
//
// A port of the 91490 CPU board's video from MAME 0.276: the graphics
// layouts in src/mame/midway/mcr_m.cpp and mcr.cpp (gfx_mcr), and the
// tilemap, palette and 91464 sprite drawing in mcr_v.cpp.

//! The picture: a 32x30 map of 16x16 background tiles, 32x32 sprites on
//! top, and a 64-colour palette held in RAM. 512x480 pixels.

pub const WIDTH: usize = 512;
pub const HEIGHT: usize = 480;

/// The graphics ROMs decoded to one byte a pixel, as MAME's gfx_element.
#[derive(Clone)]
pub struct Gfx {
    /// 1024 background tiles of 8x8 (MAME draws them doubled to 16x16).
    pub tiles: Vec<[u8; 64]>,
    /// 256 sprites of 32x32.
    pub sprites: Vec<[u8; 1024]>,
}

/// MAME's readbit: bits are numbered from the top of each byte.
fn bit(region: &[u8], n: usize) -> bool {
    region[n / 8] & (0x80 >> (n % 8)) != 0
}

impl Gfx {
    /// Decodes gfx1 with mcr_bg_layout and gfx2 with mcr_sprite_layout.
    /// The first plane is the most significant bit of the pixel.
    pub fn decode(gfx1: &[u8], gfx2: &[u8]) -> Self {
        // mcr_bg_layout: 8x8, RGN_FRAC(1,2), 4 planes at
        // { half+0, half+1, 0, 1 }, x step 2 bits, y step 16, 128 bits a tile.
        let half = gfx1.len() * 8 / 2;
        let planes = [half, half + 1, 0, 1];
        let count = gfx1.len() * 8 / 2 / 128;
        let tiles = (0..count)
            .map(|code| {
                let mut t = [0u8; 64];
                for y in 0..8 {
                    for x in 0..8 {
                        let mut v = 0;
                        for (p, &po) in planes.iter().enumerate() {
                            if bit(gfx1, code * 128 + po + y * 16 + x * 2) {
                                v |= 1 << (3 - p);
                            }
                        }
                        t[y * 8 + x] = v;
                    }
                }
                t
            })
            .collect();

        // mcr_sprite_layout: 32x32, RGN_FRAC(1,4), planes 0..3 next to each
        // other; each run of 8 pixels takes 2 from each quarter of the
        // region in turn; 32 bits a row, 1024 bits a sprite.
        let quarter = gfx2.len() * 8 / 4;
        let mut xoffs = [0usize; 32];
        for group in 0..4 {
            for q in 0..4 {
                for k in 0..2 {
                    xoffs[group * 8 + q * 2 + k] = q * quarter + group * 8 + k * 4;
                }
            }
        }
        let count = quarter / 1024;
        let sprites = (0..count)
            .map(|code| {
                let mut s = [0u8; 1024];
                for y in 0..32 {
                    for (x, &xo) in xoffs.iter().enumerate() {
                        let mut v = 0;
                        for p in 0..4 {
                            if bit(gfx2, code * 1024 + p + xo + y * 32) {
                                v |= 1 << (3 - p);
                            }
                        }
                        s[y * 32 + x] = v;
                    }
                }
                s
            })
            .collect();
        Self { tiles, sprites }
    }
}

/// mcr_set_color: 9 bits of RGB, 3 each (red the top 3, then blue, then
/// green at the bottom), as 0xRRGGBB through MAME's pal3bit.
pub fn color_9bit(data: u16) -> u32 {
    let pal3 = |v: u16| -> u32 {
        let v = u32::from(v & 7);
        (v << 5) | (v << 2) | (v >> 1)
    };
    let r = pal3(data >> 6);
    let g = pal3(data);
    let b = pal3(data >> 3);
    (r << 16) | (g << 8) | b
}

/// screen_update_mcr for the 91490 board with the 91464 sprite board:
/// each pixel's pen (0 to 63) into `pens`.
pub fn draw(gfx: &Gfx, videoram: &[u8], spriteram: &[u8], flip: bool, pens: &mut [u8]) {
    let mut pri = vec![0u8; WIDTH * HEIGHT];

    // The tilemap, 32x30 tiles of 16x16, each pixel opaque. MAME draws it
    // once per category with the category as the priority; the 91490's
    // sprites ignore that priority, so one pass draws the same.
    for row in 0..30 {
        for col in 0..32 {
            let i = row * 32 + col;
            let data = u16::from(videoram[i * 2]) | (u16::from(videoram[i * 2 + 1]) << 8);
            let code = usize::from(data & 0x3ff);
            let color = ((data >> 12) & 3) as u8;
            let flipx = (data >> 10) & 1 != 0;
            let flipy = (data >> 11) & 1 != 0;
            let category = ((data >> 14) & 3) as u8;
            let tile = &gfx.tiles[code];
            for y in 0..16 {
                for x in 0..16 {
                    let tx = if flipx { 15 - x } else { x };
                    let ty = if flipy { 15 - y } else { y };
                    let pix = tile[(ty / 2) * 8 + tx / 2];
                    let (mut sx, mut sy) = (col * 16 + x, row * 16 + y);
                    if flip {
                        sx = WIDTH - 1 - sx;
                        sy = HEIGHT - 1 - sy;
                    }
                    pens[sy * WIDTH + sx] = color * 16 + pix;
                    pri[sy * WIDTH + sx] = category << 4;
                }
            }
        }
    }

    // render_sprites_91464(primask 0x00, sprmask 0x30, colormask 0x00),
    // from the topmost (last) sprite down; a sprite pixel claims its spot
    // even when its low 3 bits are clear and it draws nothing.
    let mut offs = spriteram.len();
    while offs >= 4 {
        offs -= 4;
        let s = &spriteram[offs..offs + 4];
        let code = (usize::from(s[2]) + 256 * usize::from((s[1] >> 3) & 1)) % gfx.sprites.len();
        let color = ((!s[1] & 3) << 4) & 0x30;
        let mut hflip = if s[1] & 0x10 != 0 { 31 } else { 0 };
        let mut vflip = if s[1] & 0x20 != 0 { 31 } else { 0 };
        let mut sx = (i32::from(s[3]) - 3) * 2;
        let mut sy = (241 - i32::from(s[0])) * 2;
        if flip {
            hflip ^= 31;
            vflip ^= 31;
            sx = 480 - sx;
            sy = 452 - sy;
        }
        let sx = (sx & 0x1ff) as usize;
        let mut sy = (sy & 0x1ff) as usize;
        let sprite = &gfx.sprites[code];
        for y in 0..32usize {
            if (2..HEIGHT).contains(&sy) {
                let src = &sprite[(y ^ vflip) * 32..(y ^ vflip) * 32 + 32];
                for x in 0..32usize {
                    let tx = (sx + x) & 0x1ff;
                    // The bitmap is 512 wide, so every tx is on it.
                    let at = sy * WIDTH + tx;
                    let p = pri[at];
                    if p != 0xff {
                        let pix = color | src[x ^ hflip];
                        if pix & 0x0f != 0 {
                            pri[at] = 0xff;
                            if pix & 0x07 != 0 {
                                pens[at] = pix;
                            }
                        }
                    }
                }
            }
            sy = (sy + 1) & 0x1ff;
        }
    }
}

/// The oracle's picture hash: FNV-1a over 64 bits, a whole 0xRRGGBB pixel
/// at a time, rows from the top (docs/oracle-mame.md).
pub fn picture_hash(rgb: impl IntoIterator<Item = u32>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in rgb {
        h = (h ^ u64::from(p & 0xff_ffff)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_made_up_tile() {
        // gfx1 of 2 tiles a half: tile 0's top row has plane bits giving
        // pixel 0 = 0b1010, pixel 1 = 0b0101.
        let mut gfx1 = vec![0u8; 2 * 2 * 16];
        let half = gfx1.len() / 2;
        // Pixel 0: bits 0 and 1 of each half (x step 2 bits).
        gfx1[half] = 0b1000_0000; // plane 0 (MSB), pixel 0
        gfx1[0] = 0b1000_0000; // plane 2, pixel 0
        gfx1[half] |= 0b0001_0000; // plane 1, pixel 1
        gfx1[0] |= 0b0001_0000; // plane 3, pixel 1
        let gfx = Gfx::decode(&gfx1, &[0u8; 4 * 128]);
        assert_eq!(gfx.tiles.len(), 2);
        assert_eq!(gfx.tiles[0][0], 0b1010);
        assert_eq!(gfx.tiles[0][1], 0b0101);
        assert_eq!(gfx.tiles[0][2], 0);
        assert_eq!(gfx.sprites.len(), 1);
    }

    #[test]
    fn decodes_a_made_up_sprite() {
        // One sprite, regions of 128 bytes. Each byte of a quarter holds two
        // pixels, high nibble first: x = 0 and 1 come from quarter 0's
        // first byte, x = 2 and 3 from quarter 1's, and so on.
        let mut gfx2 = vec![0u8; 4 * 128];
        gfx2[0] = 0x9f; // x 0 = 9, x 1 = 15
        gfx2[128] = 0x30; // x 2 = 3
        gfx2[384 + 1] = 0x07; // quarter 3, second byte: x 15 = 7
        gfx2[4] = 0xa0; // row 1, x 0
        let gfx = Gfx::decode(&[0u8; 2 * 16], &gfx2);
        let s = &gfx.sprites[0];
        assert_eq!(&s[0..4], &[9, 15, 3, 0]);
        assert_eq!(s[15], 7);
        assert_eq!(s[32], 0xa);
    }

    #[test]
    fn colours_spread_three_bits_as_mame() {
        assert_eq!(color_9bit(0), 0);
        assert_eq!(color_9bit(0x1ff), 0xffffff);
        // red 4, blue 2, green 1
        assert_eq!(color_9bit(0b100_010_001), 0x92_24_49);
    }

    #[test]
    fn sprites_cover_tiles_and_hide_lower_sprites() {
        let gfx = Gfx {
            tiles: vec![[0u8; 64]; 1024],
            sprites: (0..256u16)
                .map(|c| {
                    let mut s = [0u8; 1024];
                    s.fill(if c == 1 { 0x8 } else { 0x3 });
                    s
                })
                .collect(),
        };
        let videoram = vec![0u8; 0x800];
        let mut spriteram = vec![0u8; 0x200];
        // Sprite 0 (lowest): code 2, color bits 0 so color 0x30. At y such
        // that sy = (241 - y) * 2 = 100, x such that sx = (x - 3) * 2 = 100.
        spriteram[0..4].copy_from_slice(&[191, 0x00, 2, 53]);
        // Sprite 1 (on top): code 1, all pixels 0x8: claims, draws nothing.
        spriteram[4..8].copy_from_slice(&[191, 0x03, 1, 61]);
        let mut pens = vec![0u8; WIDTH * HEIGHT];
        draw(&gfx, &videoram, &spriteram, false, &mut pens);
        assert_eq!(pens[100 * WIDTH + 100], 0x33);
        assert_eq!(pens[100 * WIDTH + 116], 0); // under sprite 1: background
        assert_eq!(pens[100 * WIDTH + 99], 0);
    }

    #[test]
    fn hash_of_black() {
        assert_eq!(
            picture_hash(std::iter::repeat_n(0, WIDTH * HEIGHT)),
            0xcd3d81a7020d2325
        );
    }
}
