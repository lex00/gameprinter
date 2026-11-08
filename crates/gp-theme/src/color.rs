//! Colors for themes: `#rrggbb` text, sRGB to linear and CIELAB, WCAG
//! relative luminance and contrast, and the HSL edits palette rules make.
//! Presentation only, so floats throughout.

use std::fmt;

/// An sRGB color, 8 bits a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    /// Parses `#rrggbb` (the `#` is optional).
    pub fn parse(text: &str) -> Result<Self, String> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("`{text}` isn't a color; write #rrggbb"));
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("checked hex");
        Ok(Self([byte(0), byte(2), byte(4)]))
    }

    /// WCAG 2 relative luminance, 0 to 1.
    pub fn luminance(self) -> f64 {
        let [r, g, b] = self.0.map(to_linear);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// CIELAB (D65), for color differences.
    pub fn lab(self) -> [f64; 3] {
        let [r, g, b] = self.0.map(to_linear);
        let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
        let f = |t: f64| {
            if t > 216.0 / 24389.0 {
                t.cbrt()
            } else {
                (24389.0 / 27.0 * t + 16.0) / 116.0
            }
        };
        let (fx, fy, fz) = (f(x), f(y), f(z));
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    /// Hue in degrees, saturation and lightness, 0 to 1.
    pub fn hsl(self) -> [f64; 3] {
        let [r, g, b] = self.0.map(|c| f64::from(c) / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let d = max - min;
        if d == 0.0 {
            return [0.0, 0.0, l];
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == r {
            60.0 * (((g - b) / d).rem_euclid(6.0))
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        [h, s, l]
    }

    pub fn from_hsl([h, s, l]: [f64; 3]) -> Self {
        let s = s.clamp(0.0, 1.0);
        let l = l.clamp(0.0, 1.0);
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let h = h.rem_euclid(360.0) / 60.0;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        Self([r, g, b].map(|v| byte((v + m) * 255.0)))
    }

    /// The color `t` of the way from `self` to `other`, in sRGB.
    pub fn mix(self, other: Rgb, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let mut out = [0u8; 3];
        for (i, o) in out.iter_mut().enumerate() {
            let a = f64::from(self.0[i]);
            let b = f64::from(other.0[i]);
            *o = byte(a + (b - a) * t);
        }
        Rgb(out)
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0;
        write!(f, "#{r:02x}{g:02x}{b:02x}")
    }
}

fn to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A float channel value as a byte, rounded and clamped.
pub fn byte(v: f64) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// WCAG 2 contrast ratio of two colors, 1 to 21.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// CIE76 color difference: the distance in CIELAB. About 2.3 is the
/// smallest difference people notice; past 20 colors read as different.
pub fn delta_e(a: Rgb, b: Rgb) -> f64 {
    let (p, q) = (a.lab(), b.lab());
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints() {
        let c = Rgb::parse("#1a2B3c").unwrap();
        assert_eq!(c.0, [0x1a, 0x2b, 0x3c]);
        assert_eq!(c.to_string(), "#1a2b3c");
        assert!(Rgb::parse("#12345").is_err());
        assert!(Rgb::parse("red").is_err());
    }

    #[test]
    fn contrast_of_black_and_white_is_21() {
        let r = contrast(Rgb([0, 0, 0]), Rgb([255, 255, 255]));
        assert!((r - 21.0).abs() < 0.01, "{r}");
        assert!((contrast(Rgb([9, 9, 9]), Rgb([9, 9, 9])) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn hsl_round_trips() {
        for c in [[200, 40, 10], [10, 200, 40], [10, 40, 200], [128, 128, 128]] {
            let back = Rgb::from_hsl(Rgb(c).hsl());
            for i in 0..3 {
                assert!(back.0[i].abs_diff(c[i]) <= 1, "{c:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn delta_e_orders_differences() {
        let red = Rgb([220, 30, 30]);
        assert!(delta_e(red, Rgb([215, 35, 30])) < 5.0);
        assert!(delta_e(red, Rgb([30, 30, 220])) > 50.0);
    }
}
