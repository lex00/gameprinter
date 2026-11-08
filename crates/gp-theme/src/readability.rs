//! The readability check: players, enemies, hazards and text must stay
//! distinguishable from the background and from each other with a theme
//! on (ARCHITECTURE.md, "Themes"). It measures the themed pictures by role,
//! deterministically, and refuses or warns.
//!
//! The background is every pixel of the `world` group, plus the view's
//! clear color. Its dominant colors are the fewest, most common colors
//! that cover 95% of it. For each foreground group:
//!
//! | Measure | What it is |
//! |---|---|
//! | `distinct` | The share of the group's pixels at least [`DISTINCT_DE`] (CIE76) from every dominant background color |
//! | `contrast` | The WCAG contrast of the group's median-luminance color with the most common background color |
//!
//! A group is refused under [`REFUSE_DISTINCT`] distinct, and warned under
//! [`WARN_DISTINCT`], or a contrast under [`WARN_CONTRAST`] with fewer than
//! [`WARN_CONTRAST_DISTINCT`] distinct. Two groups
//! whose mean colors are within [`WARN_PAIR_DE`] of each other are warned
//! (the player and the enemies look alike).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::color::{Rgb, contrast, delta_e};
use crate::stack::Resolved;

pub const DISTINCT_DE: f64 = 20.0;
pub const REFUSE_DISTINCT: f64 = 0.25;
pub const WARN_DISTINCT: f64 = 0.5;
pub const WARN_CONTRAST: f64 = 1.5;
/// A low contrast is warned only while fewer than this share of pixels
/// differ by color, since hue alone can set a sprite apart.
pub const WARN_CONTRAST_DISTINCT: f64 = 0.75;
pub const WARN_PAIR_DE: f64 = 10.0;

/// The groups measured against the background, with their roles.
pub const FOREGROUND: &[(&str, &[&str])] = &[
    ("player", &["player"]),
    ("enemies", &["enemy", "boss"]),
    ("hazards", &["hazard", "projectile"]),
    ("text", &["hud", "font", "menu"]),
];

/// The roles behind the foreground.
pub const BACKGROUND: &[&str] = &["sky", "background", "terrain", "water", "platform"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Ok,
    Warn,
    Refuse,
}

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub group: String,
    pub pixels: usize,
    pub distinct: f64,
    pub contrast: f64,
    pub verdict: Verdict,
    pub why: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pair {
    pub a: String,
    pub b: String,
    pub delta_e: f64,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub verdict: Verdict,
    pub background: Vec<String>,
    pub groups: Vec<Group>,
    pub pairs: Vec<Pair>,
    /// Groups the model has nothing of (Moss Hop has no text).
    pub absent: Vec<String>,
}

/// Pixels by role, from themed pictures.
#[derive(Debug, Clone, Default)]
pub struct Samples {
    pub by_role: BTreeMap<String, Vec<Rgb>>,
}

impl Samples {
    /// Adds a themed picture's opaque pixels by their roles (`roles`
    /// indexes [`crate::ROLES`]).
    pub fn add(&mut self, image: &crate::pix::Rgba, roles: &[u8]) {
        for (p, &r) in image.pixels.iter().zip(roles) {
            if p[3] < 128 {
                continue;
            }
            if let Some(role) = crate::ROLES.get(usize::from(r)) {
                self.by_role
                    .entry((*role).to_owned())
                    .or_default()
                    .push(Rgb([p[0], p[1], p[2]]));
            }
        }
    }

    /// Adds `n` pixels of one color with a role, such as the clear color as
    /// sky.
    pub fn add_color(&mut self, role: &str, c: Rgb, n: usize) {
        self.by_role
            .entry(role.into())
            .or_default()
            .extend(std::iter::repeat_n(c, n));
    }
}

/// The fewest most common colors covering `share` of `px`, at most 16.
fn dominant(px: &[Rgb], share: f64) -> Vec<(Rgb, usize)> {
    let mut count: BTreeMap<Rgb, usize> = BTreeMap::new();
    for c in px {
        *count.entry(*c).or_default() += 1;
    }
    let mut v: Vec<(Rgb, usize)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let need = (px.len() as f64 * share).ceil() as usize;
    let mut out = Vec::new();
    let mut got = 0;
    for (c, n) in v {
        if got >= need || out.len() == 16 {
            break;
        }
        got += n;
        out.push((c, n));
    }
    out
}

fn mean_rgb(px: &[Rgb]) -> Rgb {
    // The mean in sRGB is close enough to compare two groups' looks.
    let n = px.len().max(1) as f64;
    let mut s = [0.0f64; 3];
    for c in px {
        for (acc, v) in s.iter_mut().zip(c.0) {
            *acc += f64::from(v);
        }
    }
    Rgb(s.map(|v| crate::color::byte(v / n)))
}

/// Runs the check on themed samples. The screen tint of `theme` is applied
/// to every color first.
pub fn check(samples: &Samples, theme: &Resolved) -> Report {
    let tinted = |roles: &[&str]| -> Vec<Rgb> {
        roles
            .iter()
            .flat_map(|r| samples.by_role.get(*r).into_iter().flatten())
            .map(|c| theme.screen_color(*c))
            .collect()
    };
    let bg = tinted(BACKGROUND);
    let dom = dominant(&bg, 0.95);
    let top = dom.first().map(|(c, _)| *c).unwrap_or_default();
    let mut groups = Vec::new();
    let mut absent = Vec::new();
    let mut means: Vec<(String, Rgb)> = Vec::new();
    for (name, roles) in FOREGROUND {
        let px = tinted(roles);
        if px.is_empty() {
            absent.push((*name).to_owned());
            continue;
        }
        let distinct = px
            .iter()
            .filter(|c| dom.iter().all(|(b, _)| delta_e(**c, *b) >= DISTINCT_DE))
            .count() as f64
            / px.len() as f64;
        let mut by_l = px.clone();
        by_l.sort_by(|a, b| a.luminance().total_cmp(&b.luminance()));
        let median = by_l[by_l.len() / 2];
        let ratio = contrast(median, top);
        let (verdict, why) = if dom.is_empty() {
            (
                Verdict::Warn,
                Some("no background pixels to measure against".into()),
            )
        } else if distinct < REFUSE_DISTINCT {
            (
                Verdict::Refuse,
                Some(format!(
                    "only {:.0}% of its pixels stand out from the background (at least {:.0}% needed)",
                    distinct * 100.0,
                    REFUSE_DISTINCT * 100.0
                )),
            )
        } else if distinct < WARN_DISTINCT {
            (
                Verdict::Warn,
                Some(format!(
                    "{:.0}% of its pixels stand out from the background",
                    distinct * 100.0
                )),
            )
        } else if ratio < WARN_CONTRAST && distinct < WARN_CONTRAST_DISTINCT {
            (
                Verdict::Warn,
                Some(format!(
                    "contrast {ratio:.2} with the background, under {WARN_CONTRAST}"
                )),
            )
        } else {
            (Verdict::Ok, None)
        };
        means.push(((*name).to_owned(), mean_rgb(&px)));
        groups.push(Group {
            group: (*name).to_owned(),
            pixels: px.len(),
            distinct,
            contrast: ratio,
            verdict,
            why,
        });
    }
    let mut pairs = Vec::new();
    for i in 0..means.len() {
        for j in i + 1..means.len() {
            let d = delta_e(means[i].1, means[j].1);
            pairs.push(Pair {
                a: means[i].0.clone(),
                b: means[j].0.clone(),
                delta_e: d,
                verdict: if d < WARN_PAIR_DE {
                    Verdict::Warn
                } else {
                    Verdict::Ok
                },
            });
        }
    }
    let verdict = groups
        .iter()
        .map(|g| g.verdict)
        .chain(pairs.iter().map(|p| p.verdict))
        .max()
        .unwrap_or(Verdict::Ok);
    Report {
        verdict,
        background: dom.iter().map(|(c, _)| c.to_string()).collect(),
        groups,
        pairs,
        absent,
    }
}

impl Report {
    pub fn text(&self) -> String {
        let mut s = format!(
            "readability: {:?} (background {})\n",
            self.verdict,
            self.background.join(" ")
        )
        .to_lowercase();
        for g in &self.groups {
            s.push_str(&format!(
                "  {:<8} {:>5.1}% distinct, contrast {:>5.2}: {}{}\n",
                g.group,
                g.distinct * 100.0,
                g.contrast,
                format!("{:?}", g.verdict).to_lowercase(),
                g.why.as_ref().map(|w| format!(", {w}")).unwrap_or_default()
            ));
        }
        for p in &self.pairs {
            s.push_str(&format!(
                "  {} vs {}: delta E {:.1}: {}\n",
                p.a,
                p.b,
                p.delta_e,
                format!("{:?}", p.verdict).to_lowercase()
            ));
        }
        if !self.absent.is_empty() {
            s.push_str(&format!(
                "  none in the model: {}\n",
                self.absent.join(", ")
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(player: Rgb, sky: Rgb) -> Samples {
        let mut s = Samples::default();
        s.add_color("sky", sky, 900);
        s.add_color("terrain", Rgb([120, 80, 40]), 300);
        s.add_color("player", player, 100);
        s.add_color("enemy", Rgb([200, 30, 30]), 100);
        s
    }

    #[test]
    fn a_player_like_the_sky_is_refused() {
        let sky = Rgb([60, 160, 250]);
        let r = check(&samples(Rgb([62, 158, 248]), sky), &Resolved::none());
        assert_eq!(r.verdict, Verdict::Refuse);
        assert_eq!(r.groups[0].group, "player");
        assert_eq!(r.groups[0].verdict, Verdict::Refuse);
        assert_eq!(r.absent, ["hazards", "text"]);
    }

    #[test]
    fn a_distinct_player_passes() {
        let r = check(
            &samples(Rgb([20, 200, 40]), Rgb([60, 160, 250])),
            &Resolved::none(),
        );
        assert_eq!(r.verdict, Verdict::Ok, "{}", r.text());
    }

    #[test]
    fn look_alike_groups_are_warned() {
        let mut s = samples(Rgb([200, 30, 30]), Rgb([60, 160, 250]));
        s.by_role.remove("enemy");
        s.add_color("enemy", Rgb([202, 32, 30]), 100);
        let r = check(&s, &Resolved::none());
        assert_eq!(r.pairs[0].verdict, Verdict::Warn);
        assert_eq!(r.verdict, Verdict::Warn);
    }
}
