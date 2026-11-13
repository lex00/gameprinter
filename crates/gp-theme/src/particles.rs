//! Particles, as a pure function of the frame number, so every runtime
//! draws the same ones without any state of their own. This file depends
//! on nothing outside `core`; the web preview has a copy in JavaScript
//! (`scripts/web/page/preview.js`).
//!
//! Presentation only: floats are fine, and nothing here reads or writes the
//! simulation. A runtime passes the frame, the view and the boxes of the
//! bodies an emitter follows.

/// A full turn in radians.
const TURN: f32 = core::f32::consts::TAU;

/// How particles move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// Slow drifting dots that blink, over the screen or around a body.
    Fireflies,
    /// Falling dots that sway.
    Snow,
    /// Fast slanted streaks.
    Rain,
    /// Short-lived dots thrown up from a body's feet.
    Sparks,
}

/// One emitter, from a theme's `particles` effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Emitter {
    pub motion: Motion,
    pub color: [u8; 3],
    pub count: u32,
    pub seed: u32,
    /// True: around each body box passed in. False: over the screen.
    pub follows: bool,
}

/// A particle to draw this frame, in view pixels from the top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    /// Width and height in pixels (a rain streak is taller than wide).
    pub w: f32,
    pub h: f32,
    pub color: [u8; 3],
    pub alpha: f32,
}

/// A hash of three numbers to 0..1: the same in every runtime, from 32-bit
/// integer arithmetic only.
pub fn unit(seed: u32, i: u32, salt: u32) -> f32 {
    let mut x = seed
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(i.wrapping_mul(0x85EB_CA6B))
        .wrapping_add(salt.wrapping_mul(0xC2B2_AE35));
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / 16_777_216.0
}

/// The dots of one emitter at `frame`. `view` is the view's size and
/// `bodies` the boxes (x, y, w, h, in view pixels) it follows.
pub fn dots(e: &Emitter, frame: u64, view: [f32; 2], bodies: &[[f32; 4]]) -> Vec<Dot> {
    let t = (frame % 1_000_000) as f32;
    let mut out = Vec::new();
    let screen = [[0.0, 0.0, view[0], view[1]]];
    let boxes: &[[f32; 4]] = if e.follows { bodies } else { &screen };
    for (b, &[bx, by, bw, bh]) in boxes.iter().enumerate() {
        for i in 0..e.count {
            let k = i + (b as u32) * 1000;
            let (u, v, w) = (unit(e.seed, k, 1), unit(e.seed, k, 2), unit(e.seed, k, 3));
            let d = match e.motion {
                Motion::Fireflies => {
                    let r = if e.follows { 12.0 } else { 0.0 };
                    let x = bx - r
                        + (u * (bw + 2.0 * r) + 6.0 * (t * 0.02 + w * TURN).sin())
                            .rem_euclid(bw + 2.0 * r);
                    let y = by - r
                        + (v * (bh + 2.0 * r) + 4.0 * (t * 0.017 + u * TURN).cos())
                            .rem_euclid(bh + 2.0 * r);
                    let a = 0.5 + 0.5 * (t * 0.05 + w * TURN).sin();
                    Dot {
                        x,
                        y,
                        w: 1.0,
                        h: 1.0,
                        color: e.color,
                        alpha: a,
                    }
                }
                Motion::Snow => {
                    let x = bx + (u * bw + 3.0 * (t * 0.03 + w * TURN).sin()).rem_euclid(bw);
                    let y = by + (v * bh + t * (0.3 + 0.4 * w)).rem_euclid(bh);
                    Dot {
                        x,
                        y,
                        w: 1.0,
                        h: 1.0,
                        color: e.color,
                        alpha: 0.9,
                    }
                }
                Motion::Rain => {
                    let x = bx + (u * bw - t * 1.0).rem_euclid(bw);
                    let y = by + (v * bh + t * (3.0 + 2.0 * w)).rem_euclid(bh);
                    Dot {
                        x,
                        y,
                        w: 1.0,
                        h: 4.0,
                        color: e.color,
                        alpha: 0.6,
                    }
                }
                Motion::Sparks => {
                    let life = 30.0;
                    let age = (t + w * life).rem_euclid(life);
                    let x = bx + u * bw + (v - 0.5) * age * 0.6;
                    let y = by + bh - age * (0.8 + v) + 0.02 * age * age;
                    Dot {
                        x,
                        y,
                        w: 1.0,
                        h: 1.0,
                        color: e.color,
                        alpha: 1.0 - age / life,
                    }
                }
            };
            out.push(d);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_frame_gives_the_same_dots() {
        let e = Emitter {
            motion: Motion::Fireflies,
            color: [255, 220, 90],
            count: 8,
            seed: 3,
            follows: false,
        };
        let a = dots(&e, 120, [256.0, 240.0], &[]);
        assert_eq!(a, dots(&e, 120, [256.0, 240.0], &[]));
        assert_ne!(a, dots(&e, 121, [256.0, 240.0], &[]));
        assert_eq!(a.len(), 8);
        assert!(
            a.iter()
                .all(|d| (0.0..256.0).contains(&d.x) && (0.0..240.0).contains(&d.y))
        );
    }

    #[test]
    fn a_following_emitter_draws_per_body() {
        let e = Emitter {
            motion: Motion::Sparks,
            color: [255; 3],
            count: 4,
            seed: 1,
            follows: true,
        };
        assert_eq!(
            dots(
                &e,
                5,
                [256.0, 240.0],
                &[[10.0, 10.0, 16.0, 16.0], [50.0, 10.0, 16.0, 16.0]]
            )
            .len(),
            8
        );
        assert!(dots(&e, 5, [256.0, 240.0], &[]).is_empty());
    }

    #[test]
    fn unit_is_in_range() {
        for i in 0..1000 {
            let u = unit(7, i, 1);
            assert!((0.0..1.0).contains(&u));
        }
    }
}
