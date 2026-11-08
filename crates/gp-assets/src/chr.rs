//! NES pattern tiles (2 bits per pixel) to and from an indexed PNG.
//!
//! A tile is 16 bytes: eight bytes of bit plane 0, one per row with the
//! leftmost pixel in bit 7, then eight of bit plane 1. A pixel's color index
//! is plane 0's bit plus twice plane 1's. The PNG lays tiles out left to
//! right, `tiles_per_row` to a row, and stores each pixel's index (0 to 3)
//! with four gray preview colors. Pixels past the last tile are 0.

use std::io::Cursor;

/// Preview colors for indices 0 to 3, darkest first. Only the indices are
/// data; the game's palettes decide the real colors.
const GRAYS: [u8; 12] = [
    0x00, 0x00, 0x00, 0x55, 0x55, 0x55, 0xAA, 0xAA, 0xAA, 0xFF, 0xFF, 0xFF,
];

/// The image size, in pixels, for `tiles` tiles laid out `per_row` wide.
pub fn image_size(tiles: u32, per_row: u32) -> (u32, u32) {
    let cols = per_row.min(tiles);
    let rows = tiles.div_ceil(per_row);
    (cols * 8, rows * 8)
}

/// One color index per pixel, row by row.
pub fn decode(bytes: &[u8], per_row: u32) -> (u32, u32, Vec<u8>) {
    let tiles = (bytes.len() / 16) as u32;
    let (w, h) = image_size(tiles, per_row);
    let mut pixels = vec![0u8; (w * h) as usize];
    for (t, tile) in bytes.as_chunks::<16>().0.iter().enumerate() {
        let t = t as u32;
        let (tx, ty) = ((t % per_row) * 8, (t / per_row) * 8);
        for y in 0..8u32 {
            let (p0, p1) = (tile[y as usize], tile[y as usize + 8]);
            for x in 0..8u32 {
                let bit = 7 - x;
                let index = ((p0 >> bit) & 1) | (((p1 >> bit) & 1) << 1);
                pixels[((ty + y) * w + tx + x) as usize] = index;
            }
        }
    }
    (w, h, pixels)
}

/// The tile bytes for `tiles` tiles from one color index per pixel.
pub fn encode(pixels: &[u8], width: u32, tiles: u32, per_row: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(tiles as usize * 16);
    for t in 0..tiles {
        let (tx, ty) = ((t % per_row) * 8, (t / per_row) * 8);
        let mut planes = [0u8; 16];
        for y in 0..8u32 {
            for x in 0..8u32 {
                let index = pixels[((ty + y) * width + tx + x) as usize];
                let bit = 7 - x;
                planes[y as usize] |= (index & 1) << bit;
                planes[y as usize + 8] |= ((index >> 1) & 1) << bit;
            }
        }
        out.extend_from_slice(&planes);
    }
    out
}

/// An indexed PNG, 2 bits per pixel, with the gray preview palette.
pub fn write_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut packed = Vec::with_capacity((width.div_ceil(4) * height) as usize);
    for row in pixels.chunks_exact(width as usize) {
        for four in row.chunks(4) {
            let mut byte = 0u8;
            for (i, p) in four.iter().enumerate() {
                byte |= (p & 3) << (6 - 2 * i);
            }
            packed.push(byte);
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Two);
    enc.set_palette(&GRAYS[..]);
    enc.set_compression(png::Compression::Balanced);
    let mut writer = enc.write_header().expect("PNG header into memory");
    writer
        .write_image_data(&packed)
        .expect("PNG data into memory");
    writer.finish().expect("PNG end into memory");
    out
}

/// Reads an indexed PNG of any bit depth back to one index per pixel.
/// Image editors often save 8-bit indexed images, so any depth is taken;
/// whether every index is 0 to 3 is the caller's check.
/// The image must be `want` (width, height) pixels; that's checked from
/// the header, before any pixel data is read.
pub fn read_png(bytes: &[u8], want: (u32, u32)) -> Result<Vec<u8>, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    if info.color_type != png::ColorType::Indexed {
        return Err(format!(
            "it's a {:?} image; tiles must stay an indexed (palette) PNG, one index 0 to 3 per pixel",
            info.color_type
        ));
    }
    if info.is_animated() {
        return Err("it's an animated PNG; tiles are one still image".into());
    }
    let (width, height) = (info.width, info.height);
    if (width, height) != want {
        return Err(format!(
            "the image is {width}x{height}; the map's tiles make {}x{}",
            want.0, want.1
        ));
    }
    let depth = info.bit_depth as u32;
    let size = reader
        .output_buffer_size()
        .ok_or("the image is too large to read")?;
    let mut buf = vec![0u8; size];
    let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let stride = frame.line_size;
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height as usize {
        let line = &buf[y * stride..(y + 1) * stride];
        for x in 0..width as usize {
            let bit = x as u32 * depth;
            let byte = line[(bit / 8) as usize];
            let shift = 8 - depth - (bit % 8);
            pixels.push((byte >> shift) & ((1u16 << depth) - 1) as u8);
        }
    }
    Ok(pixels)
}
