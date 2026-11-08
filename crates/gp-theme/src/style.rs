//! Style checks measured against the active theme: with a theme on, the
//! contract's Style level holds art to the theme's style guide, not to the
//! original art (ARCHITECTURE.md, "Themes").
//!
//! Measures go through [`StyleMeasure`], so an embedding model can sit
//! beside the deterministic metrics later. [`Deterministic`] has three:
//!
//! | Metric | Per | What it is | Passes when |
//! |---|---|---|---|
//! | `palette_distance` | Sprite | The mean CIE76 distance from each opaque pixel to the nearest color of the guide's palette | At most `max_palette_distance` (default 12) |
//! | `edge_density` | Sprite | The share of opaque pixels with a 4-neighbour of another color or transparent | Within `edge_density`, when the guide gives it |
//! | `size` | Sprite | How far the drawn box (the opaque pixels' bounds) is from the entity's size, as a share of it | At most `size_tolerance` (default 0.25) |
//! | `size_consistency` | Role | The spread (max minus min) of `size` across the role's sprites | At most `size_tolerance` |

use std::collections::BTreeMap;

use serde::Serialize;

use crate::color::{Rgb, delta_e};
use crate::format::StyleGuide;
use crate::pix::Rgba;
use crate::stack::Resolved;

/// A piece of art a Style check covers.
#[derive(Debug, Clone)]
pub struct Subject {
    /// The model path it draws, such as `entities.player.sprite`.
    pub part: String,
    pub role: String,
    /// The themed picture.
    pub image: Rgba,
    /// The entity's size, for the size metrics.
    pub size: [u32; 2],
}

/// What art is measured against: the highest layer's style guide.
#[derive(Debug, Clone)]
pub struct Reference {
    pub theme: String,
    pub guide: StyleGuide,
    pub palette: Vec<Rgb>,
}

impl Reference {
    /// The style guide of the highest layer that has one.
    pub fn of(theme: &Resolved) -> Option<Self> {
        theme.layers.iter().rev().find_map(|l| {
            l.theme.style.as_ref().map(|g| Reference {
                theme: l.theme.id.clone(),
                guide: g.clone(),
                palette: g
                    .palette
                    .iter()
                    .filter_map(|c| Rgb::parse(c).ok())
                    .collect(),
            })
        })
    }
}

/// One measurement.
#[derive(Debug, Clone, Serialize)]
pub struct Measure {
    pub part: String,
    pub metric: String,
    pub value: f64,
    pub bound: String,
    pub pass: bool,
}

/// A way of measuring style: the deterministic metrics now, an embedding
/// model's distance later.
pub trait StyleMeasure {
    fn name(&self) -> &'static str;
    fn measure(&self, subjects: &[Subject], reference: &Reference) -> Vec<Measure>;
}

/// The deterministic metrics.
pub struct Deterministic;

/// Mean distance from each opaque pixel to the palette.
pub fn palette_distance(img: &Rgba, palette: &[Rgb]) -> Option<f64> {
    let px: Vec<Rgb> = img
        .pixels
        .iter()
        .filter(|p| p[3] >= 128)
        .map(|p| Rgb([p[0], p[1], p[2]]))
        .collect();
    if px.is_empty() || palette.is_empty() {
        return None;
    }
    let sum: f64 = px
        .iter()
        .map(|c| {
            palette
                .iter()
                .map(|p| delta_e(*c, *p))
                .fold(f64::INFINITY, f64::min)
        })
        .sum();
    Some(sum / px.len() as f64)
}

/// The share of opaque pixels on an edge.
pub fn edge_density(img: &Rgba) -> Option<f64> {
    let (w, h) = (img.width as i64, img.height as i64);
    let at = |x: i64, y: i64| -> [u8; 4] {
        if x < 0 || y < 0 || x >= w || y >= h {
            [0; 4]
        } else {
            img.get(x as u32, y as u32)
        }
    };
    let (mut opaque, mut edge) = (0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            let p = at(x, y);
            if p[3] < 128 {
                continue;
            }
            opaque += 1;
            if [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| at(x + dx, y + dy) != p)
            {
                edge += 1;
            }
        }
    }
    (opaque > 0).then(|| edge as f64 / opaque as f64)
}

/// How far the opaque pixels' bounds are from `size`, as a share.
pub fn size_error(img: &Rgba, size: [u32; 2]) -> Option<f64> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..img.height {
        for x in 0..img.width {
            if img.get(x, y)[3] >= 128 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == u32::MAX || size[0] == 0 || size[1] == 0 {
        return None;
    }
    let ew = (f64::from(x1 - x0) - f64::from(size[0])).abs() / f64::from(size[0]);
    let eh = (f64::from(y1 - y0) - f64::from(size[1])).abs() / f64::from(size[1]);
    Some(ew.max(eh))
}

impl StyleMeasure for Deterministic {
    fn name(&self) -> &'static str {
        "deterministic"
    }

    fn measure(&self, subjects: &[Subject], r: &Reference) -> Vec<Measure> {
        let max_d = r.guide.max_palette_distance.unwrap_or(12.0);
        let tol = r.guide.size_tolerance.unwrap_or(0.25);
        let mut out = Vec::new();
        let mut by_role: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        for s in subjects {
            if let Some(d) = palette_distance(&s.image, &r.palette) {
                out.push(Measure {
                    part: s.part.clone(),
                    metric: "palette_distance".into(),
                    value: d,
                    bound: format!("<= {max_d}"),
                    pass: d <= max_d,
                });
            }
            if let (Some(e), Some([lo, hi])) = (edge_density(&s.image), r.guide.edge_density) {
                out.push(Measure {
                    part: s.part.clone(),
                    metric: "edge_density".into(),
                    value: e,
                    bound: format!("{lo} to {hi}"),
                    pass: (lo..=hi).contains(&e),
                });
            }
            if let Some(e) = size_error(&s.image, s.size) {
                by_role.entry(&s.role).or_default().push(e);
                out.push(Measure {
                    part: s.part.clone(),
                    metric: "size".into(),
                    value: e,
                    bound: format!("<= {tol}"),
                    pass: e <= tol,
                });
            }
        }
        for (role, v) in by_role {
            if v.len() < 2 {
                continue;
            }
            let spread = v.iter().copied().fold(f64::MIN, f64::max)
                - v.iter().copied().fold(f64::MAX, f64::min);
            out.push(Measure {
                part: format!("entities[role={role}]"),
                metric: "size_consistency".into(),
                value: spread,
                bound: format!("<= {tol}"),
                pass: spread <= tol,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(c: [u8; 4], n: u32, of: u32) -> Rgba {
        let mut img = Rgba::new(of, of);
        for y in 0..n {
            for x in 0..n {
                img.set(x, y, c);
            }
        }
        img
    }

    #[test]
    fn metrics_on_made_up_art() {
        let img = square([200, 40, 40, 255], 4, 8);
        assert_eq!(palette_distance(&img, &[Rgb([200, 40, 40])]), Some(0.0));
        // A 4x4 square: 12 of its 16 pixels are on its edge.
        assert_eq!(edge_density(&img), Some(0.75));
        assert_eq!(size_error(&img, [4, 4]), Some(0.0));
        assert_eq!(size_error(&img, [8, 8]), Some(0.5));
        assert_eq!(edge_density(&Rgba::new(2, 2)), None);
    }

    #[test]
    fn measures_against_the_guide() {
        let r = Reference {
            theme: "t".into(),
            guide: StyleGuide {
                palette: vec![],
                max_palette_distance: Some(5.0),
                edge_density: Some([0.0, 0.5]),
                size_tolerance: None,
            },
            palette: vec![Rgb([0, 0, 255])],
        };
        let s = [
            Subject {
                part: "entities.a.sprite".into(),
                role: "enemy".into(),
                image: square([200, 40, 40, 255], 4, 8),
                size: [4, 4],
            },
            Subject {
                part: "entities.b.sprite".into(),
                role: "enemy".into(),
                image: square([0, 0, 255, 255], 8, 8),
                size: [4, 4],
            },
        ];
        let m = Deterministic.measure(&s, &r);
        let get = |p: &str, k: &str| m.iter().find(|x| x.part == p && x.metric == k).unwrap();
        assert!(!get("entities.a.sprite", "palette_distance").pass);
        assert!(get("entities.b.sprite", "palette_distance").pass);
        assert!(!get("entities.a.sprite", "edge_density").pass);
        assert!(!get("entities[role=enemy]", "size_consistency").pass);
    }
}
