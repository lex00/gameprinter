//! A frame drawn on the CPU from the presentation's pictures: the level at
//! the camera, sprites, particles and the screen overlay. For runtimes with
//! no GPU (themed videos of the interpreter, the editor's theme preview);
//! the web preview and the Bevy build draw the same layers in their own
//! renderers.

use crate::particles::Dot;
use crate::pix::Rgba;

/// A sprite at its top left corner, in level pixels.
#[derive(Debug, Clone, Copy)]
pub struct Placed<'a> {
    pub image: &'a Rgba,
    pub x: i64,
    pub y: i64,
    pub flip: bool,
}

/// What a frame shows.
#[derive(Debug, Clone)]
pub struct Scene<'a> {
    pub view: [u32; 2],
    pub clear: [u8; 3],
    pub level: Option<&'a Rgba>,
    /// The view's top left corner, in level pixels.
    pub camera: [i64; 2],
    pub sprites: Vec<Placed<'a>>,
    /// Particles, in view pixels.
    pub dots: Vec<Dot>,
    pub overlay: Option<&'a Rgba>,
}

fn blend(dst: &mut [u8; 4], src: [u8; 4], alpha: f32) {
    let a = f32::from(src[3]) / 255.0 * alpha.clamp(0.0, 1.0);
    for i in 0..3 {
        dst[i] = (f32::from(src[i]) * a + f32::from(dst[i]) * (1.0 - a)).round() as u8;
    }
    dst[3] = 255;
}

/// Draws the scene, opaque, at the view's size.
pub fn frame(s: &Scene<'_>) -> Rgba {
    let [w, h] = s.view;
    let [r, g, b] = s.clear;
    let mut out = Rgba {
        width: w,
        height: h,
        pixels: vec![[r, g, b, 255]; (w * h) as usize],
    };
    let [cx, cy] = s.camera;
    let draw = |img: &Rgba, ox: i64, oy: i64, flip: bool, out: &mut Rgba| {
        for y in 0..img.height {
            let vy = oy + i64::from(y);
            if vy < 0 || vy >= i64::from(h) {
                continue;
            }
            for x in 0..img.width {
                let vx = ox + i64::from(x);
                if vx < 0 || vx >= i64::from(w) {
                    continue;
                }
                let sx = if flip { img.width - 1 - x } else { x };
                let p = img.get(sx, y);
                if p[3] == 0 {
                    continue;
                }
                let i = (vy as u32 * w + vx as u32) as usize;
                blend(&mut out.pixels[i], p, 1.0);
            }
        }
    };
    if let Some(l) = s.level {
        draw(l, -cx, -cy, false, &mut out);
    }
    for sp in &s.sprites {
        draw(sp.image, sp.x - cx, sp.y - cy, sp.flip, &mut out);
    }
    for d in &s.dots {
        let [r, g, b] = d.color;
        for yy in 0..d.h.max(1.0) as i64 {
            for xx in 0..d.w.max(1.0) as i64 {
                let (x, y) = (d.x.floor() as i64 + xx, d.y.floor() as i64 + yy);
                if x >= 0 && y >= 0 && x < i64::from(w) && y < i64::from(h) {
                    let i = (y as u32 * w + x as u32) as usize;
                    blend(&mut out.pixels[i], [r, g, b, 255], d.alpha);
                }
            }
        }
    }
    if let Some(o) = s.overlay {
        draw(o, 0, 0, false, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_draw_in_order() {
        let mut level = Rgba::new(8, 4);
        level.set(5, 0, [0, 255, 0, 255]);
        let mut spr = Rgba::new(2, 1);
        spr.set(0, 0, [255, 0, 0, 255]);
        let s = Scene {
            view: [4, 2],
            clear: [0, 0, 255],
            level: Some(&level),
            camera: [4, 0],
            sprites: vec![Placed {
                image: &spr,
                x: 6,
                y: 1,
                flip: true,
            }],
            dots: vec![],
            overlay: None,
        };
        let f = frame(&s);
        assert_eq!(f.get(1, 0), [0, 255, 0, 255], "the level, scrolled");
        assert_eq!(f.get(0, 0), [0, 0, 255, 255], "the clear color under it");
        assert_eq!(f.get(3, 1), [255, 0, 0, 255], "the sprite, mirrored");
        assert_eq!(f.get(2, 1), [0, 0, 255, 255]);
    }
}
