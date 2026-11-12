//! The consistency critic for generated assets (docs/host.md, "Critic").
//!
//! A [`Critic`] reviews a candidate asset in the context of the working
//! copy: the palettes, the other sprites drawn from the same sheet, the
//! level's backdrop. The one built in, [`Metrics`], is deterministic and
//! offline: it measures, with fixed thresholds, and the same candidate in
//! the same copy always gets the same review. An image model would be
//! another `Critic`; it gets the same [`Subject`] (with the images as PNG on
//! request) and returns the same [`Review`].
//!
//! The checks:
//!
//! | Check | Looks at | Fails when |
//! |---|---|---|
//! | `shape.ink` | The share of a sprite's pixels that are drawn | Blank or solid; warns outside the role's usual range |
//! | `shape.bbox` | How much of the sprite's box the drawing spans | Warns under 50% on either axis |
//! | `size.siblings` | Ink density against other sprites of the sheet | Warns when the difference is over 0.35 |
//! | `palette.distinct` | The sprite's palette | Two of colors 1 to 3 are equal, or one is the backdrop |
//! | `palette.hues` | Every palette of the game | Warns over 8 distinct hues |
//! | `palette.ramp` | One palette's light-to-dark spread | Warns under 0.15 relative luminance |
//! | `palette.harmony` | A palette's brightness against the rest | Warns over 1.5 steps from their mean |
//! | `contrast.backdrop` | Sprite colors against the level's backdrop | Fails under 1.5, warns under 3.0 (WCAG ratio) |
//! | `style.similarity` | Ink, outline and color mix against other art | Warns over 0.35 |
//!
//! A check with nothing to compare against is `skipped`, not passed.

use std::fs;

use gp_assets::{Asset, chr};
use gp_model::model::AssetKind;
use serde::Serialize;
use serde_json::{Value, json};

use crate::generate::Candidate;
use crate::place::{self, Current, Target};
use crate::{Config, catalog};

/// How one check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub verdict: Verdict,
    /// What was measured, when it is a number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// The limit it was held to, in words.
    pub limit: String,
    pub message: String,
}

/// A critic's answer.
#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub critic: &'static str,
    /// 0 to 1: passes count 1, warnings 0.5, failures 0, over the checks
    /// that weren't skipped.
    pub score: f64,
    /// No check failed.
    pub pass: bool,
    pub checks: Vec<Check>,
}

impl Review {
    pub fn from_checks(critic: &'static str, checks: Vec<Check>) -> Self {
        let counted: Vec<&Check> = checks
            .iter()
            .filter(|c| c.verdict != Verdict::Skipped)
            .collect();
        let points: f64 = counted
            .iter()
            .map(|c| match c.verdict {
                Verdict::Pass => 1.0,
                Verdict::Warn => 0.5,
                _ => 0.0,
            })
            .sum();
        let score = if counted.is_empty() {
            1.0
        } else {
            (points / counted.len() as f64 * 1000.0).round() / 1000.0
        };
        Self {
            critic,
            score,
            pass: checks.iter().all(|c| c.verdict != Verdict::Fail),
            checks,
        }
    }

    pub fn json(&self) -> Value {
        serde_json::to_value(self).expect("a review serializes")
    }

    pub fn problems(&self) -> Vec<&Check> {
        self.checks
            .iter()
            .filter(|c| matches!(c.verdict, Verdict::Warn | Verdict::Fail))
            .collect()
    }
}

/// An image of color indices 0 to 3, row by row.
#[derive(Debug, Clone)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Image {
    pub fn to_png(&self) -> Vec<u8> {
        chr::write_png(self.w, self.h, &self.px)
    }

    fn ink(&self) -> usize {
        self.px.iter().filter(|&&p| p != 0).count()
    }

    /// The share of pixels drawn.
    pub fn density(&self) -> f64 {
        if self.px.is_empty() {
            0.0
        } else {
            self.ink() as f64 / self.px.len() as f64
        }
    }

    /// The drawing's bounding box as shares of the image: (width, height).
    fn span(&self) -> (f64, f64) {
        let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0, u32::MAX, 0);
        for (i, &p) in self.px.iter().enumerate() {
            if p == 0 {
                continue;
            }
            let (x, y) = (i as u32 % self.w, i as u32 / self.w);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        if x0 == u32::MAX {
            return (0.0, 0.0);
        }
        (
            f64::from(x1 - x0 + 1) / f64::from(self.w),
            f64::from(y1 - y0 + 1) / f64::from(self.h),
        )
    }

    /// Ink, outline share, and the shares of indices 2 and 3 among the ink:
    /// a small vector that stands for how the art is drawn.
    fn style(&self) -> [f64; 4] {
        let ink = self.ink().max(1) as f64;
        let count = |v: u8| self.px.iter().filter(|&&p| p == v).count() as f64;
        [
            self.density(),
            count(1) / ink,
            count(2) / ink,
            count(3) / ink,
        ]
    }
}

/// Another sprite of the same sheet, as it is now.
#[derive(Debug, Clone)]
pub struct Sibling {
    pub entity: String,
    pub role: String,
    pub image: Image,
}

/// Everything a critic may look at.
#[derive(Debug, Clone)]
pub struct Subject {
    pub slot: String,
    pub kind: AssetKind,
    pub entity: Option<String>,
    pub role: Option<String>,
    /// The candidate's picture: the sprite's or the whole sheet's. None for
    /// a palettes candidate.
    pub image: Option<Image>,
    /// Whether `image` is one entity's sprite.
    pub is_sprite: bool,
    /// The palette the art draws with (the candidate's, for a palettes
    /// candidate that is for one entity).
    pub palette: Option<[u8; 4]>,
    /// Every palette of the game as it would be with the candidate placed.
    pub palettes: Vec<[u8; 4]>,
    /// The level's backdrop: color 0 of background palette 0.
    pub backdrop: Option<u8>,
    /// Other sprites of the same sheet.
    pub siblings: Vec<Sibling>,
    /// Pictures of other tiles slots, for style.
    pub other_art: Vec<Image>,
    /// The prompt the person gave, for a model that reads one.
    pub prompt: Option<String>,
}

/// A way of judging a candidate.
pub trait Critic {
    fn name(&self) -> &'static str;
    fn review(&self, subject: &Subject) -> Result<Review, String>;
}

/// The critic used unless one is named.
pub const DEFAULT: &str = "metrics";

/// Every critic, by name.
pub fn critics() -> Vec<Box<dyn Critic>> {
    vec![Box::new(Metrics)]
}

/// Reads what the critic needs from a working copy, with the candidate
/// substituted for what it replaces. Writes nothing.
pub fn subject(
    config: &Config,
    t: &Target,
    candidate: &Candidate,
    prompt: Option<&str>,
) -> Result<Subject, String> {
    let (at, map) = place::current_assets(config, &t.dir, &t.model)?;
    let assets = catalog::slot_assets(&t.model, &map)?;

    // The palettes of the game now.
    let mut palettes: Vec<[u8; 4]> = Vec::new();
    for (name, slot) in &t.model.assets {
        if slot.kind != AssetKind::Palettes {
            continue;
        }
        if let Some(Asset::Palette { .. }) = assets.get(name.as_str()).copied() {
            let a = assets[name.as_str()];
            let text = fs::read_to_string(at.join(a.file()))
                .map_err(|e| format!("reading {}: {e}", a.file()))?;
            palettes = place::read_palettes(&text)?;
            break;
        }
    }
    let mut palette = match (&t.sprite, palettes.is_empty()) {
        (Some(sp), false) => palettes.get(sp.palette as usize).copied(),
        _ => None,
    };
    let mut image = None;
    let mut is_sprite = false;
    match candidate {
        Candidate::Pixels(px) => {
            let (w, h) = t
                .image_size()
                .ok_or("a tiles candidate for a palettes slot")?;
            image = Some(Image {
                w,
                h,
                px: px.clone(),
            });
            is_sprite = t.sprite.is_some();
        }
        Candidate::Palettes(p) => match &t.sprite {
            Some(sp) => {
                if let Some(slot) = palettes.get_mut(sp.palette as usize) {
                    *slot = p[0];
                }
                palette = p.first().copied();
            }
            None => {
                for (i, c) in p.iter().enumerate() {
                    if let Some(slot) = palettes.get_mut(i) {
                        *slot = *c;
                    }
                }
            }
        },
    }
    let backdrop = palettes.first().map(|p| p[0]);

    // Siblings: other entities with a sprite in the same tiles slot, as
    // the sheet holds them (or as the candidate would draw them, for a
    // whole-sheet candidate).
    let mut siblings = Vec::new();
    if let Current::Tiles {
        per_row,
        size,
        pixels,
        ..
    } = &t.current
    {
        let sheet: &[u8] = match (candidate, t.sprite.is_none()) {
            (Candidate::Pixels(px), true) => px,
            _ => pixels,
        };
        for (name, e) in &t.model.entities {
            let Some(sp) = e.core.sprite.as_ref().filter(|s| s.slot == t.slot) else {
                continue;
            };
            if t.entity.as_deref() == Some(name.as_str()) && t.sprite.is_some() {
                continue;
            }
            let w = sp.width.max(1);
            let (iw, ih) = (w * 8, (sp.tiles.len() as u32).div_ceil(w) * 8);
            let mut px = vec![0u8; (iw * ih) as usize];
            let mut ok = true;
            for (k, &tile) in sp.tiles.iter().enumerate() {
                let (sx, sy) = ((k as u32 % w) * 8, (k as u32 / w) * 8);
                let (tx, ty) = ((tile % per_row) * 8, (tile / per_row) * 8);
                for y in 0..8 {
                    for x in 0..8 {
                        let from = ((ty + y) * size.0 + tx + x) as usize;
                        let Some(&v) = sheet.get(from) else {
                            ok = false;
                            continue;
                        };
                        px[((sy + y) * iw + sx + x) as usize] = v;
                    }
                }
            }
            if ok {
                siblings.push(Sibling {
                    entity: name.clone(),
                    role: e.core.role.clone(),
                    image: Image { w: iw, h: ih, px },
                });
            }
        }
    }

    // Other tiles slots' sheets, when they are filled from other assets.
    let mut other_art = Vec::new();
    for (name, slot) in &t.model.assets {
        if slot.kind != AssetKind::Tiles || *name == t.slot {
            continue;
        }
        let Some(Asset::Chr {
            tiles,
            tiles_per_row,
            ..
        }) = assets.get(name.as_str()).copied()
        else {
            continue;
        };
        if assets[name.as_str()].file() == t.asset.file() {
            continue;
        }
        let size = chr::image_size(*tiles, *tiles_per_row);
        if let Ok(bytes) = fs::read(at.join(assets[name.as_str()].file()))
            && let Ok(px) = chr::read_png(&bytes, size)
        {
            other_art.push(Image {
                w: size.0,
                h: size.1,
                px,
            });
        }
    }

    Ok(Subject {
        slot: t.slot.clone(),
        kind: t.kind,
        entity: t.entity.clone(),
        role: t.role().map(str::to_owned),
        image,
        is_sprite,
        palette,
        palettes,
        backdrop,
        siblings,
        other_art,
        prompt: prompt.map(str::to_owned),
    })
}

// ---- NES colors ----

/// An RGB approximation of an NES color number, from its hue (low nibble)
/// and brightness (high nibble). It is a fixed table of twelve hues
/// shaded by row, not any one emulator's palette: close enough for contrast
/// ratios, and the same on every machine.
pub fn nes_rgb(c: u8) -> [u8; 3] {
    const HUES: [[u16; 3]; 12] = [
        [0, 70, 200],
        [60, 40, 220],
        [130, 40, 200],
        [190, 40, 160],
        [210, 40, 90],
        [210, 50, 30],
        [190, 90, 10],
        [160, 120, 0],
        [90, 150, 0],
        [20, 160, 20],
        [10, 160, 80],
        [10, 140, 160],
    ];
    let (row, hue) = (u16::from((c >> 4) & 3), u16::from(c & 0x0F));
    let gray = |g: u16| [g as u8; 3];
    match hue {
        0 => gray([84, 168, 236, 255][row as usize]),
        13 => gray([0, 0, 110, 160][row as usize]),
        14 | 15 => gray(0),
        h => {
            let base = HUES[(h - 1) as usize];
            // Percent of the hue; the last row is pastel (mixed with white).
            let shade = [45u16, 80, 100][row.min(2) as usize];
            let mut rgb = [0u8; 3];
            for i in 0..3 {
                let mut v = base[i] * shade / 100;
                if row == 3 {
                    v = (base[i] + 2 * 255) / 3;
                }
                rgb[i] = v.min(255) as u8;
            }
            rgb
        }
    }
}

/// Relative luminance as WCAG defines it, 0 to 1.
pub fn luminance(rgb: [u8; 3]) -> f64 {
    let lin = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

/// WCAG contrast ratio of two colors, 1 to 21.
pub fn contrast(a: u8, b: u8) -> f64 {
    let (la, lb) = (luminance(nes_rgb(a)), luminance(nes_rgb(b)));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn chromatic_hue(c: u8) -> Option<u8> {
    let h = c & 0x0F;
    (1..=12).contains(&h).then_some(h)
}

// ---- The metric critic ----

/// The deterministic critic.
pub struct Metrics;

/// The share of a sprite's pixels drawn, by role: the range outside which
/// the check warns.
fn ink_range(role: &str) -> (f64, f64) {
    match role {
        "player" | "npc" | "enemy" | "boss" => (0.30, 0.95),
        "collectible" | "power-up" | "goal" => (0.15, 0.90),
        "projectile" => (0.05, 0.70),
        "hazard" => (0.20, 0.95),
        _ => (0.10, 0.98),
    }
}

fn check(
    name: &'static str,
    verdict: Verdict,
    value: Option<f64>,
    limit: impl Into<String>,
    message: impl Into<String>,
) -> Check {
    Check {
        name,
        verdict,
        value: value.map(|v| (v * 1000.0).round() / 1000.0),
        limit: limit.into(),
        message: message.into(),
    }
}

fn skipped(name: &'static str, why: &str) -> Check {
    check(name, Verdict::Skipped, None, "", why)
}

fn style_distance(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f64>() / 4.0
}

impl Metrics {
    fn shape(s: &Subject, out: &mut Vec<Check>) {
        let Some(img) = s.image.as_ref().filter(|_| s.is_sprite) else {
            for n in ["shape.ink", "shape.bbox", "size.siblings"] {
                out.push(skipped(n, "not a single sprite"));
            }
            return;
        };
        let role = s.role.as_deref().unwrap_or("");
        let (lo, hi) = ink_range(role);
        let d = img.density();
        let limit = format!("{lo} to {hi} for role `{role}`; blank and solid fail");
        out.push(if img.ink() == 0 {
            check(
                "shape.ink",
                Verdict::Fail,
                Some(d),
                limit,
                "the sprite is blank",
            )
        } else if img.px.iter().all(|&p| p != 0) {
            check(
                "shape.ink",
                Verdict::Fail,
                Some(d),
                limit,
                "the sprite is solid, with no silhouette",
            )
        } else if d < lo || d > hi {
            check(
                "shape.ink",
                Verdict::Warn,
                Some(d),
                limit,
                format!(
                    "{:.0}% of the sprite is drawn, unusual for a {role}",
                    d * 100.0
                ),
            )
        } else {
            check(
                "shape.ink",
                Verdict::Pass,
                Some(d),
                limit,
                "the silhouette fills a usual share of the box",
            )
        });
        let (sw, sh) = img.span();
        let m = sw.min(sh);
        out.push(if m < 0.5 {
            check(
                "shape.bbox",
                Verdict::Warn,
                Some(m),
                "at least 0.5 on both axes",
                "the drawing is small in its box, so it will look smaller than its hitbox",
            )
        } else {
            check(
                "shape.bbox",
                Verdict::Pass,
                Some(m),
                "at least 0.5 on both axes",
                "the drawing spans its box",
            )
        });
        let same: Vec<f64> = s
            .siblings
            .iter()
            .filter(|b| b.role == role && b.image.ink() > 0)
            .map(|b| b.image.density())
            .collect();
        let any: Vec<f64> = s
            .siblings
            .iter()
            .filter(|b| b.image.ink() > 0)
            .map(|b| b.image.density())
            .collect();
        let pool = if same.is_empty() { any } else { same };
        out.push(if pool.is_empty() {
            skipped(
                "size.siblings",
                "no other sprite on the sheet has art to compare",
            )
        } else {
            let mut p = pool;
            p.sort_by(f64::total_cmp);
            let median = p[p.len() / 2];
            let diff = (d - median).abs();
            if diff > 0.35 {
                check(
                    "size.siblings",
                    Verdict::Warn,
                    Some(diff),
                    "at most 0.35 from the median",
                    format!("ink density {d:.2} against {median:.2} for the other sprites"),
                )
            } else {
                check(
                    "size.siblings",
                    Verdict::Pass,
                    Some(diff),
                    "at most 0.35 from the median",
                    "as dense as the other sprites",
                )
            }
        });
    }

    fn palette(s: &Subject, out: &mut Vec<Check>) {
        match s.palette {
            Some(p) => {
                let (a, b, c) = (p[1], p[2], p[3]);
                let backdrop = s.backdrop;
                let dup = a == b || b == c || a == c;
                let on_backdrop = backdrop.is_some_and(|bd| [a, b, c].contains(&bd));
                out.push(if dup {
                    check(
                        "palette.distinct",
                        Verdict::Fail,
                        None,
                        "colors 1 to 3 differ",
                        "two of the sprite palette's colors are the same, so its shading is lost",
                    )
                } else if on_backdrop {
                    check(
                        "palette.distinct",
                        Verdict::Fail,
                        None,
                        "colors 1 to 3 differ from the backdrop",
                        "a sprite color equals the backdrop, so part of it vanishes",
                    )
                } else {
                    check(
                        "palette.distinct",
                        Verdict::Pass,
                        None,
                        "colors 1 to 3 differ",
                        "three distinct colors, none the backdrop",
                    )
                });
                let ls: Vec<f64> = [a, b, c].iter().map(|&c| luminance(nes_rgb(c))).collect();
                let span =
                    ls.iter().cloned().fold(0.0, f64::max) - ls.iter().cloned().fold(1.0, f64::min);
                out.push(if span < 0.15 {
                    check("palette.ramp", Verdict::Warn, Some(span), "at least 0.15", "the palette's colors are nearly as bright as each other, so the sprite will look flat")
                } else {
                    check("palette.ramp", Verdict::Pass, Some(span), "at least 0.15", "the palette has a light-to-dark spread")
                });
            }
            None => {
                out.push(skipped(
                    "palette.distinct",
                    "no palette is known for this art",
                ));
                out.push(skipped("palette.ramp", "no palette is known for this art"));
            }
        }
        if s.palettes.is_empty() {
            out.push(skipped("palette.hues", "the game has no palettes slot"));
            out.push(skipped("palette.harmony", "the game has no palettes slot"));
            return;
        }
        let mut hues: Vec<u8> = s
            .palettes
            .iter()
            .flat_map(|p| p.iter().filter_map(|&c| chromatic_hue(c)))
            .collect();
        hues.sort_unstable();
        hues.dedup();
        out.push(if hues.len() > 8 {
            check(
                "palette.hues",
                Verdict::Warn,
                Some(hues.len() as f64),
                "at most 8 distinct hues",
                "the palettes use many hues, which reads as unrelated art",
            )
        } else {
            check(
                "palette.hues",
                Verdict::Pass,
                Some(hues.len() as f64),
                "at most 8 distinct hues",
                "the hues are few enough to belong together",
            )
        });
        // Brightness steps (0 to 3) of a palette's chromatic and gray
        // colors, averaged; the candidate palette against the others.
        let level =
            |p: &[u8; 4]| p[1..].iter().map(|&c| f64::from((c >> 4) & 3)).sum::<f64>() / 3.0;
        match s.palette {
            Some(me) if s.palettes.len() > 1 => {
                let others: Vec<f64> = s.palettes.iter().filter(|p| **p != me).map(level).collect();
                if others.is_empty() {
                    out.push(skipped("palette.harmony", "every palette is this one"));
                } else {
                    let mean = others.iter().sum::<f64>() / others.len() as f64;
                    let d = (level(&me) - mean).abs();
                    out.push(if d > 1.5 {
                        check(
                            "palette.harmony",
                            Verdict::Warn,
                            Some(d),
                            "at most 1.5 brightness steps from the other palettes",
                            "much brighter or darker than the rest",
                        )
                    } else {
                        check(
                            "palette.harmony",
                            Verdict::Pass,
                            Some(d),
                            "at most 1.5 brightness steps from the other palettes",
                            "in line with the other palettes",
                        )
                    });
                }
            }
            _ => out.push(skipped("palette.harmony", "no single palette to compare")),
        }
    }

    fn contrast(s: &Subject, out: &mut Vec<Check>) {
        let (Some(p), Some(bd)) = (s.palette, s.backdrop) else {
            out.push(skipped(
                "contrast.backdrop",
                "needs the sprite's palette and the level backdrop",
            ));
            return;
        };
        if s.kind == AssetKind::Tiles && s.image.is_none() {
            out.push(skipped("contrast.backdrop", "no picture"));
            return;
        }
        // Indices the sprite actually draws with.
        let used: Vec<usize> = match s.image.as_ref().filter(|_| s.is_sprite) {
            Some(img) => (1..=3).filter(|i| img.px.contains(&(*i as u8))).collect(),
            None => vec![1, 2, 3],
        };
        if used.is_empty() {
            out.push(skipped("contrast.backdrop", "the sprite draws nothing"));
            return;
        }
        // The best any drawn color does against the backdrop; the outline
        // (index 1) can carry a sprite whose body is close to the backdrop.
        let best = used.iter().map(|&i| contrast(p[i], bd)).fold(0.0, f64::max);
        let limit = "at least 3.0 (fails under 1.5)";
        out.push(if best < 1.5 {
            check(
                "contrast.backdrop",
                Verdict::Fail,
                Some(best),
                limit,
                "no color of the sprite can be told from the backdrop",
            )
        } else if best < 3.0 {
            check(
                "contrast.backdrop",
                Verdict::Warn,
                Some(best),
                limit,
                "the sprite is hard to read against the backdrop",
            )
        } else {
            check(
                "contrast.backdrop",
                Verdict::Pass,
                Some(best),
                limit,
                "the sprite stands out from the backdrop",
            )
        });
    }

    fn style(s: &Subject, out: &mut Vec<Check>) {
        let Some(img) = s.image.as_ref().filter(|i| i.ink() > 0) else {
            out.push(skipped("style.similarity", "no picture"));
            return;
        };
        let mut refs: Vec<[f64; 4]> = s
            .siblings
            .iter()
            .filter(|b| b.image.ink() > 0)
            .map(|b| b.image.style())
            .collect();
        refs.extend(s.other_art.iter().filter(|i| i.ink() > 0).map(Image::style));
        if refs.is_empty() || !s.is_sprite {
            out.push(skipped(
                "style.similarity",
                "no other art to compare, or the candidate is a whole sheet",
            ));
            return;
        }
        let mut mean = [0.0; 4];
        for r in &refs {
            for i in 0..4 {
                mean[i] += r[i] / refs.len() as f64;
            }
        }
        let d = style_distance(&img.style(), &mean);
        out.push(if d > 0.35 {
            check(
                "style.similarity",
                Verdict::Warn,
                Some(d),
                "at most 0.35",
                "drawn differently from the other art (ink, outline and color mix)",
            )
        } else {
            check(
                "style.similarity",
                Verdict::Pass,
                Some(d),
                "at most 0.35",
                "drawn like the other art",
            )
        });
    }
}

impl Critic for Metrics {
    fn name(&self) -> &'static str {
        "metrics"
    }

    fn review(&self, s: &Subject) -> Result<Review, String> {
        let mut checks = Vec::new();
        Self::shape(s, &mut checks);
        Self::palette(s, &mut checks);
        Self::contrast(s, &mut checks);
        Self::style(s, &mut checks);
        Ok(Review::from_checks(self.name(), checks))
    }
}

/// The named critic's review of a candidate in a working copy, as JSON.
pub fn review(
    config: &Config,
    t: &Target,
    candidate: &Candidate,
    prompt: Option<&str>,
    critic: &str,
) -> Result<Review, String> {
    let c = critics()
        .into_iter()
        .find(|c| c.name() == critic)
        .ok_or(format!(
            "there's no critic `{critic}`; there are {:?}",
            critics().iter().map(|c| c.name()).collect::<Vec<_>>()
        ))?;
    c.review(&subject(config, t, candidate, prompt)?)
}

/// A one-line summary for a log.
pub fn summary(r: &Review) -> Value {
    json!({
        "critic": r.critic,
        "score": r.score,
        "pass": r.pass,
        "problems": r.problems().iter().map(|c| format!("{}: {}", c.name, c.message)).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::shape;

    fn subject_with(img: Image, palette: [u8; 4], backdrop: u8) -> Subject {
        Subject {
            slot: "tiles".into(),
            kind: AssetKind::Tiles,
            entity: Some("player".into()),
            role: Some("player".into()),
            image: Some(img),
            is_sprite: true,
            palette: Some(palette),
            palettes: vec![[backdrop, 0x0F, 0x16, 0x30], palette],
            backdrop: Some(backdrop),
            siblings: Vec::new(),
            other_art: Vec::new(),
            prompt: None,
        }
    }

    fn get<'a>(r: &'a Review, name: &str) -> &'a Check {
        r.checks.iter().find(|c| c.name == name).expect(name)
    }

    #[test]
    fn a_shaped_sprite_with_a_readable_palette_passes() {
        let img = Image {
            w: 16,
            h: 16,
            px: shape("player", 16, 16, 1),
        };
        let r = Metrics
            .review(&subject_with(img, [0x22, 0x0F, 0x16, 0x30], 0x22))
            .unwrap();
        assert!(r.pass, "{:?}", r.problems());
        assert_eq!(get(&r, "shape.ink").verdict, Verdict::Pass);
        assert_eq!(get(&r, "contrast.backdrop").verdict, Verdict::Pass);
        assert_eq!(get(&r, "size.siblings").verdict, Verdict::Skipped);
    }

    #[test]
    fn blank_solid_and_invisible_art_fail() {
        let blank = Image {
            w: 8,
            h: 8,
            px: vec![0; 64],
        };
        let r = Metrics
            .review(&subject_with(blank, [0x22, 0x0F, 0x16, 0x30], 0x22))
            .unwrap();
        assert_eq!(get(&r, "shape.ink").verdict, Verdict::Fail);
        assert!(!r.pass);

        let solid = Image {
            w: 8,
            h: 8,
            px: vec![2; 64],
        };
        let r = Metrics
            .review(&subject_with(solid, [0x22, 0x0F, 0x16, 0x30], 0x22))
            .unwrap();
        assert_eq!(get(&r, "shape.ink").verdict, Verdict::Fail);

        // A body the color of the sky.
        let img = Image {
            w: 16,
            h: 16,
            px: shape("player", 16, 16, 1),
        };
        let r = Metrics
            .review(&subject_with(img, [0x22, 0x22, 0x22, 0x22], 0x22))
            .unwrap();
        assert_eq!(get(&r, "palette.distinct").verdict, Verdict::Fail);
        assert_eq!(get(&r, "contrast.backdrop").verdict, Verdict::Fail);
    }

    #[test]
    fn contrast_is_a_ratio_of_luminances() {
        assert!(contrast(0x0F, 0x30) > 15.0, "black on white");
        assert!((contrast(0x21, 0x21) - 1.0).abs() < 1e-9);
        assert!(contrast(0x16, 0x26) < contrast(0x0F, 0x30));
    }

    #[test]
    fn the_review_is_the_same_every_time() {
        let img = Image {
            w: 16,
            h: 16,
            px: shape("enemy", 16, 16, 3),
        };
        let mut s = subject_with(img, [0x22, 0x0F, 0x16, 0x30], 0x22);
        s.role = Some("enemy".into());
        let a = Metrics.review(&s).unwrap().json();
        assert_eq!(a, Metrics.review(&s).unwrap().json());
    }

    #[test]
    fn a_sprite_unlike_its_siblings_is_flagged() {
        let img = Image {
            w: 16,
            h: 16,
            px: shape("player", 16, 16, 1),
        };
        let mut s = subject_with(img, [0x22, 0x0F, 0x16, 0x30], 0x22);
        let mut sparse = vec![0u8; 256];
        sparse[0] = 1;
        s.siblings.push(Sibling {
            entity: "slug".into(),
            role: "player".into(),
            image: Image {
                w: 16,
                h: 16,
                px: sparse,
            },
        });
        let r = Metrics.review(&s).unwrap();
        assert_eq!(get(&r, "size.siblings").verdict, Verdict::Warn);
        assert_eq!(get(&r, "style.similarity").verdict, Verdict::Warn);
        assert!(r.pass, "warnings don't fail a review");
        assert!(r.score < 1.0);
    }
}
