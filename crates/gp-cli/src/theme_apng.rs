//! An animated PNG written frame by frame, each frame as the rectangle
//! that changed since the one before: `gp theme video`'s output.
//!
//! Brought from game-printer's crates/gp-cli/src/video.rs (`Apng` and
//! `changed`) without the rest of `gp video`, which gameprinter has no
//! command for.

use anyhow::Result;

/// The smallest rectangle holding every pixel that differs between two
/// images of the same size, as x, y, width, height.
fn changed(a: &[u8], b: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let row = w * 3;
    let rows: Vec<usize> = (0..h)
        .filter(|y| a[y * row..(y + 1) * row] != b[y * row..(y + 1) * row])
        .collect();
    let (&top, &bottom) = (rows.first()?, rows.last()?);
    let (mut left, mut right) = (w, 0);
    for y in top..=bottom {
        let (ra, rb) = (&a[y * row..(y + 1) * row], &b[y * row..(y + 1) * row]);
        for x in 0..w {
            if ra[x * 3..x * 3 + 3] != rb[x * 3..x * 3 + 3] {
                left = left.min(x);
                right = right.max(x);
            }
        }
    }
    Some((left, top, right - left + 1, bottom - top + 1))
}

/// Writes frames to an animated PNG, each as the rectangle that changed
/// since the frame before.
pub(crate) struct Apng<W: std::io::Write> {
    writer: png::Writer<W>,
    w: usize,
    h: usize,
    previous: Option<Vec<u8>>,
    rect: Vec<u8>,
}

impl<W: std::io::Write> Apng<W> {
    pub(crate) fn new(out: W, w: usize, h: usize, frames: u32, delay: (u16, u16)) -> Result<Self> {
        let mut enc = png::Encoder::new(out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Balanced);
        enc.set_animated(frames, 1)?;
        enc.set_frame_delay(delay.0, delay.1)?;
        enc.set_blend_op(png::BlendOp::Source)?;
        enc.set_dispose_op(png::DisposeOp::None)?;
        Ok(Self {
            writer: enc.write_header()?,
            w,
            h,
            previous: None,
            rect: Vec::new(),
        })
    }

    pub(crate) fn frame(&mut self, rgb: &[u8]) -> Result<()> {
        let Some(previous) = &self.previous else {
            self.writer.write_image_data(rgb)?;
            self.previous = Some(rgb.to_vec());
            return Ok(());
        };
        // An unchanged frame is still a frame: one pixel, as it was.
        let (x, y, w, h) = changed(previous, rgb, self.w, self.h).unwrap_or((0, 0, 1, 1));
        self.rect.clear();
        for row in y..y + h {
            let start = (row * self.w + x) * 3;
            self.rect.extend_from_slice(&rgb[start..start + w * 3]);
        }
        // Each setter checks the rectangle against the image with the
        // other's current value, so go through the origin.
        self.writer.set_frame_position(0, 0)?;
        self.writer.set_frame_dimension(w as u32, h as u32)?;
        self.writer.set_frame_position(x as u32, y as u32)?;
        self.writer.write_image_data(&self.rect)?;
        if let Some(p) = &mut self.previous {
            p.copy_from_slice(rgb);
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<()> {
        self.writer.finish()?;
        Ok(())
    }
}
