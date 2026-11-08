//! A painter for generated placeholder art in a theme's style, from roles
//! and sizes only: any entity role at any size, and any item role as a 16
//! by 16 cell. It knows nothing about a particular game. The same role,
//! size, scheme and seed give the same pixels every run, and what it makes
//! is CC0-1.0.
//!
//! The look is paper cutout: flat shapes in a role's colors with a dark
//! edge and a one-pixel drop shadow, and a faint grain on large areas.

use crate::color::Rgb;
use crate::particles::unit;
use crate::pix::Rgba;

/// A role's three colors: base, light and dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shades {
    pub base: Rgb,
    pub light: Rgb,
    pub dark: Rgb,
}

/// The colors a painting uses, by role group: `(role, shades)`, with
/// `default` for a role the list doesn't name.
#[derive(Debug, Clone)]
pub struct Scheme {
    pub roles: Vec<(String, Shades)>,
    pub default: Shades,
    pub ink: Rgb,
}

impl Scheme {
    pub fn shades(&self, role: &str) -> Shades {
        self.roles
            .iter()
            .find(|(r, _)| r == role)
            .map(|(_, s)| *s)
            .unwrap_or(self.default)
    }

    /// Every color the scheme can paint, for a style guide's palette.
    pub fn colors(&self) -> Vec<Rgb> {
        let mut v = vec![self.ink];
        for s in self.roles.iter().map(|(_, s)| s).chain([&self.default]) {
            for c in [s.base, s.light, s.dark] {
                if !v.contains(&c) {
                    v.push(c);
                }
            }
        }
        v
    }
}

fn px(c: Rgb) -> [u8; 4] {
    [c.0[0], c.0[1], c.0[2], 255]
}

/// The outline of a shape: what to fill at (x, y) of a w by h box.
fn inside(role: &str, x: f64, y: f64, w: f64, h: f64) -> bool {
    let (u, v) = (x / w, y / h);
    match role {
        // Spikes along the bottom.
        "hazard" => {
            let teeth = (w / 8.0).max(1.0);
            let t = (u * teeth).fract();
            v > 0.3 && v >= 1.0 - (1.0 - (2.0 * t - 1.0).abs()) * 0.7 - 0.0
        }
        // A diamond.
        "collectible" => (u - 0.5).abs() + (v - 0.5).abs() <= 0.42,
        // A round badge.
        "power-up" | "projectile" => (u - 0.5).powi(2) + (v - 0.5).powi(2) <= 0.16,
        // A framed box.
        "interactive" => (0.1..0.9).contains(&u) && (0.1..0.9).contains(&v),
        // An arch.
        "goal" => v > 0.2 && ((u - 0.5).powi(2) * 4.0 + (v - 0.9).powi(2) * 1.3 <= 1.0),
        // Creatures: a low blob for enemies, a rounder one otherwise.
        "enemy" | "boss" => {
            let cy = 0.68;
            ((u - 0.5) / 0.48).powi(2) + ((v - cy) / 0.32).powi(2) <= 1.0
        }
        _ => ((u - 0.5) / 0.44).powi(2) + ((v - 0.55) / 0.44).powi(2) <= 1.0,
    }
}

/// A sprite for an entity of `role`, `w` by `h` pixels.
pub fn sprite(role: &str, w: u32, h: u32, scheme: &Scheme, seed: u32) -> Rgba {
    let s = scheme.shades(role);
    let mut img = Rgba::new(w, h);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let fill = |x: u32, y: u32| {
        inside(
            role,
            f64::from(x) + 0.5,
            f64::from(y) + 0.5,
            fw - 1.0,
            fh - 1.0,
        )
    };
    // Shadow, one pixel down and right, then the shape over it.
    for y in 0..h {
        for x in 0..w {
            if x > 0 && y > 0 && fill(x - 1, y - 1) && !fill(x, y) {
                img.set(x, y, px(s.dark));
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            if !fill(x, y) {
                continue;
            }
            let edge = x == 0
                || y == 0
                || !fill(x - 1, y)
                || !fill(x + 1, y)
                || !fill(x, y - 1)
                || !fill(x, y + 1);
            let c = if edge {
                scheme.ink
            } else if f64::from(y) < fh * 0.4 && unit(seed, x + y * w, 9) < 0.5 {
                s.light
            } else {
                s.base
            };
            img.set(x, y, px(c));
        }
    }
    // Creatures get an eye, facing right, a third of the way down.
    if matches!(role, "player" | "enemy" | "boss" | "npc") && w >= 6 && h >= 6 {
        let (ex, ey) = (w * 2 / 3, h / 3 + if role == "player" { 0 } else { h / 6 });
        if fill(ex, ey) {
            img.set(ex, ey, px(scheme.ink));
            img.set(ex.saturating_sub(1), ey, px(s.light));
        }
    }
    img
}

/// A 16 by 16 cell for a level item of `role`.
pub fn cell(role: &str, scheme: &Scheme, seed: u32) -> Rgba {
    let s = scheme.shades(role);
    let mut img = Rgba::new(16, 16);
    for y in 0..16u32 {
        for x in 0..16u32 {
            let grain = unit(seed, x + y * 16, 5);
            let c = match role {
                "sky" | "background" => {
                    if grain < 0.04 {
                        s.light
                    } else {
                        s.base
                    }
                }
                "water" => {
                    let wave =
                        ((f64::from(x) / 16.0 * std::f64::consts::TAU).sin() * 1.5 + 2.0) as u32;
                    if y == wave {
                        s.light
                    } else if y < wave {
                        s.dark
                    } else {
                        s.base
                    }
                }
                "goal" | "interactive" | "collectible" | "power-up" => {
                    if x == 0 || y == 0 || x == 15 || y == 15 {
                        scheme.ink
                    } else if (x + y) % 4 == 0 {
                        s.light
                    } else {
                        s.base
                    }
                }
                // Terrain and platforms: a light top strip with a torn
                // edge, a dark seam, and grained body.
                _ => {
                    let tear = 3 + (unit(seed, x, 6) * 2.0) as u32;
                    if y < tear {
                        s.light
                    } else if y == tear || grain < 0.08 {
                        s.dark
                    } else {
                        s.base
                    }
                }
            };
            img.set(x, y, px(c));
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheme() -> Scheme {
        let sh = |a, b, c| Shades {
            base: Rgb(a),
            light: Rgb(b),
            dark: Rgb(c),
        };
        Scheme {
            roles: vec![(
                "player".into(),
                sh([90, 160, 70], [150, 210, 120], [40, 80, 30]),
            )],
            default: sh([200, 190, 170], [230, 225, 210], [120, 110, 95]),
            ink: Rgb([40, 30, 25]),
        }
    }

    #[test]
    fn the_same_inputs_paint_the_same_pixels() {
        let a = sprite("player", 16, 16, &scheme(), 1);
        assert_eq!(a, sprite("player", 16, 16, &scheme(), 1));
        assert_ne!(a, sprite("enemy", 16, 16, &scheme(), 1));
        assert!(
            a.pixels.iter().any(|p| p[3] == 0),
            "a sprite has a transparent margin"
        );
        assert!(a.pixels.iter().any(|p| p[..3] == [90, 160, 70]));
        for role in [
            "hazard",
            "collectible",
            "goal",
            "interactive",
            "power-up",
            "boss",
        ] {
            let s = sprite(role, 16, 24, &scheme(), 2);
            assert!(
                s.pixels.iter().any(|p| p[3] == 255),
                "{role} paints something"
            );
        }
    }

    #[test]
    fn cells_are_opaque() {
        for role in ["terrain", "sky", "water", "goal", "platform"] {
            let c = cell(role, &scheme(), 3);
            assert!(c.pixels.iter().all(|p| p[3] == 255), "{role}");
        }
    }
}
