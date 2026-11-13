//! A theme kit's gameplay geometry (docs/theme-kit.md, "Gameplay
//! geometry"): what a theme's art must keep so the game plays as fairly
//! themed as it does with its own art.
//!
//! The shareable kit gets, from the model alone, each sprite slot's boxes
//! in its picture's pixels (the box that meets the hero, the one that
//! stands on floors, the hero's ladder grab box, and the boxes the game's
//! `themes/kit/geometry.toml` names among its hooks' numbers), the frames
//! that harm, the row the feet stand on, the smallest visible body and
//! what it must stand apart from, and each stage's floor and ladder rules.
//! The local part gets what only the lifted art and level say: each
//! frame's visible body in the original, and each stage's floor lines and
//! ladders.
//!
//! `gp theme check` holds a themed sprite to the original's body within
//! [`EDGE`] pixels of place and [`SIZE`] of size (the feet within
//! [`FEET`]), and a harmful box to being drawn on; and a themed stage's
//! floors to the original's within [`FLOOR`] pixels. A flag the original's
//! own art has too is not raised.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use gp_model::Model;
use gp_model::model::Entity;
use gp_model::platformer::Touch;
use serde::{Deserialize, Serialize};

/// The game's extra boxes, beside words.toml.
pub const GEOMETRY_FILE: &str = "geometry.toml";
/// A themed body's middle may sit this many pixels off the original's.
pub const EDGE: i64 = 2;
/// A themed body's width and height may differ this many pixels from the
/// original's.
pub const SIZE: i64 = 4;
/// A body that stands on the floor keeps its bottom row within this many
/// pixels of the original's.
pub const FEET: i64 = 1;
/// A stage's floor line keeps within this many pixels of the original's.
pub const FLOOR: i64 = 1;
/// The share of a harmful box a frame's visible body must cover.
pub const COVER: f64 = 0.5;
/// The smallest side of a visible body, in pixels.
pub const MIN_SIDE: i64 = 6;
/// The share of a floor's columns that may miss its line before it is
/// flagged.
pub const FLOOR_MISS: f64 = 0.1;

/// A box on a slot's picture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoBox {
    /// `touch` (meets the hero: harms it, or is taken), `solid` (stands on
    /// floors, stops at walls), `grab` (takes a ladder), `hit` (smashes
    /// what it meets) or `struck` (where a hit lands on it).
    pub kind: String,
    /// What it does, in plain words.
    pub what: String,
    /// `[x, y, w, h]` in the picture's pixels from its top left. It may
    /// reach past the picture.
    pub rect: [i64; 4],
    /// The frames it holds in, by name; all when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<String>,
}

/// A sprite slot's geometry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boxes: Vec<GeoBox>,
    /// The frames that harm the hero (or, for a tool, smash), by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub danger: Vec<String>,
    /// What the danger is, in plain words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub danger_what: String,
    /// The row under the feet, from the picture's top, when it stands on
    /// the floor: the bottom of its touch box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feet: Option<i64>,
    /// The smallest visible body, width and height.
    pub min_size: [i64; 2],
    /// What it must stand apart from.
    pub contrast: String,
}

/// The tolerances the check holds art to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    pub edge: i64,
    pub size: i64,
    pub feet: i64,
    pub floor: i64,
    pub cover: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            edge: EDGE,
            size: SIZE,
            feet: FEET,
            floor: FLOOR,
            cover: COVER,
        }
    }
}

/// A level's surfaces and climbable zones, as rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    pub level: String,
    /// The picture slot its stage is drawn as, when it has one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub picture: String,
    /// Where a body stands.
    pub floor: String,
    /// The rows inside an 8 px cell a surface tile's top edge sits at.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tile_tops: Vec<u8>,
    /// What can be climbed, and how.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub climb: String,
    /// Where the hero falls out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub below: String,
}

/// A stage's floor lines and ladders, from its lifted level (the local
/// part of a kit).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub level: String,
    /// Each floor's pieces, `[x0, row, x1]`, the row a body stands on;
    /// bottom floor first.
    pub floors: Vec<Vec<[i64; 3]>>,
    /// The step heights between a floor's pieces, and how many of each.
    pub steps: BTreeMap<i64, usize>,
    pub ladders: Vec<LadderZone>,
}

/// A ladder's box and what it joins.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LadderZone {
    /// `[x0, y0, x1, y1]`, edges included.
    pub rect: [i64; 4],
    /// The floors it stands on and leads to, bottom first.
    pub lower: Option<usize>,
    pub upper: Option<usize>,
    /// The upper floor's row at its middle.
    pub top: Option<i64>,
    /// It stops short of the floor above: a stub the hero can't climb.
    pub broken: bool,
}

/// The game's `themes/kit/geometry.toml`: boxes and danger frames its
/// hooks' numbers (`ext_params`) say, which the model's components don't.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeoFile {
    pub format: String,
    #[serde(default, rename = "box")]
    pub boxes: Vec<BoxRef>,
    #[serde(default)]
    pub slot: Vec<SlotRef>,
}

/// A box named by four of an entity's `ext_params`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoxRef {
    /// The kit's slot id.
    pub slot: String,
    pub kind: String,
    pub what: String,
    /// The left, top, right and bottom, edges included, as param names.
    #[serde(default)]
    pub params: Vec<String>,
    /// Or the box itself, `[x, y, w, h]` in the picture's pixels, for a box
    /// the model holds only folded into another (a touch test the model
    /// puts on the hero's column); `source` then says where it was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<[i64; 4]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// The entity whose params they are; the slot's own by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params_of: Option<String>,
    /// Two params giving the slot's place from the box's origin, when the
    /// box is measured from another body (a held tool's from the hero's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<[String; 2]>,
    #[serde(default)]
    pub frames: Vec<String>,
}

/// A slot's danger frames, where the model's contact doesn't say.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotRef {
    pub id: String,
    #[serde(default)]
    pub danger: Vec<String>,
    #[serde(default)]
    pub danger_what: String,
}

/// A template's geometry file, when it has one.
pub fn load(template: &Path) -> Result<Option<GeoFile>> {
    let f = template.join("themes").join("kit").join(GEOMETRY_FILE);
    if !f.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?;
    Ok(Some(
        toml::from_str(&text).with_context(|| format!("{}", f.display()))?,
    ))
}

fn int(f: gp_model::Fixed) -> i64 {
    f.raw() >> 16
}

/// Where a sprite's picture sits on its body: the picture's top left from
/// the body's place.
pub fn picture_offset(e: &Entity, pic: [u32; 2]) -> [i64; 2] {
    match e.core.sprite.as_ref().and_then(|s| s.offset) {
        Some([x, y]) => [i64::from(x), i64::from(y)],
        None => {
            let [w, h] = e.core.size.map(int);
            [(w - i64::from(pic[0])) / 2, h - i64::from(pic[1])]
        }
    }
}

/// The readability rule a role is held to (gp_theme::readability).
pub fn contrast_rule(role: &str) -> String {
    use gp_theme::readability as r;
    let group = r::FOREGROUND
        .iter()
        .find(|(_, roles)| roles.contains(&role))
        .map(|(g, _)| *g);
    let base = format!(
        "stand apart from the stage's {}: at least {:.0}% of its pixels {} or more (CIE76) from each of the stage's main colors, and a contrast of {} or more with the most common one",
        r::BACKGROUND.join(", "),
        r::WARN_DISTINCT * 100.0,
        r::DISTINCT_DE,
        r::WARN_CONTRAST
    );
    match group {
        Some(g) => format!(
            "{base}; measured as the `{g}` group, refused under {:.0}%",
            r::REFUSE_DISTINCT * 100.0
        ),
        None => format!("{base} (not measured for its role)"),
    }
}

/// An entity slot's geometry from the model and the game's geometry file.
pub fn of_entity(
    m: &Model,
    e: &Entity,
    id: &str,
    pic: [u32; 2],
    frame_names: &[String],
    file: Option<&GeoFile>,
) -> Geometry {
    let off = picture_offset(e, pic);
    let on_pic = |r: [i64; 4]| [r[0] - off[0], r[1] - off[1], r[2], r[3]];
    let hb = &e.core.hitbox;
    let hit = [int(hb.x), int(hb.y), int(hb.w), int(hb.h)];
    let contact = e.platformer.contact.as_ref().map(|c| &c.otherwise);
    let mut g = Geometry {
        contrast: contrast_rule(&e.core.role),
        ..Geometry::default()
    };
    let player = e.core.role == "player";
    if hit[2] * hit[3] > 1 || player && hit[3] > 1 {
        let what = match contact {
            Some(Touch::KillPlayer) => "harms the hero where the hero's box meets it",
            Some(Touch::HurtPlayer) => "hurts the hero where the hero's box meets it",
            Some(Touch::Collect) => "the hero takes it where the hero's box meets it",
            Some(Touch::Kick) => "the hero kicks it where the hero's box meets it",
            Some(Touch::Defeat) => "is defeated where the hero's box meets it",
            _ if matches!(e.core.role.as_str(), "power-up" | "collectible") => {
                "the hero takes it where the hero's box meets it"
            }
            _ if player => "the hero's own box: a foe harms the hero where it meets this",
            _ => "meets the hero here",
        };
        g.boxes.push(GeoBox {
            kind: "touch".into(),
            what: what.into(),
            rect: on_pic(hit),
            frames: Vec::new(),
        });
        let feet = hit[1] + hit[3] - off[1];
        let stands = player || matches!(contact, Some(Touch::KillPlayer | Touch::HurtPlayer));
        if stands && feet > 0 && feet <= i64::from(pic[1]) {
            g.feet = Some(feet);
        }
    }
    if e.core.tile_collision {
        let t = e.core.tile_hitbox.as_ref().unwrap_or(hb);
        g.boxes.push(GeoBox {
            kind: "solid".into(),
            what: "stands on floors and stops at walls".into(),
            rect: on_pic([int(t.x), int(t.y), int(t.w), int(t.h)]),
            frames: Vec::new(),
        });
    }
    if let Some(c) = &e.platformer.climb
        && let Some([x0, y0, x1, y1]) = c.grab
    {
        g.boxes.push(GeoBox {
            kind: "grab".into(),
            what: "takes a ladder where this box meets the ladder's box".into(),
            rect: on_pic([
                i64::from(x0),
                i64::from(y0),
                i64::from(x1 - x0 + 1),
                i64::from(y1 - y0 + 1),
            ]),
            frames: Vec::new(),
        });
    }
    if matches!(contact, Some(Touch::KillPlayer | Touch::HurtPlayer)) {
        g.danger = frame_names.to_vec();
        g.danger_what = "harms the hero on touch".into();
    }
    if let Some(f) = file {
        for b in f.boxes.iter().filter(|b| b.slot == id) {
            let of = b
                .params_of
                .as_deref()
                .and_then(|n| m.entities.get(n))
                .unwrap_or(e);
            let p = |n: &str| of.core.ext_params.get(n).map_or(0, |v| int(*v));
            let rect = match (b.rect, b.params.as_slice()) {
                (Some(r), _) => r,
                (None, [a, b0, c, d]) => {
                    let [x0, y0, x1, y1] = [a, b0, c, d].map(|n| p(n));
                    let o = b.origin.as_ref().map_or([0, 0], |[x, y]| [p(x), p(y)]);
                    on_pic([x0 - o[0], y0 - o[1], x1 - x0 + 1, y1 - y0 + 1])
                }
                _ => continue,
            };
            // A touch box the file gives is the true one: it takes the
            // model's place.
            if b.kind == "touch" {
                g.boxes.retain(|x| x.kind != "touch");
            }
            let what = if b.source.is_empty() {
                b.what.clone()
            } else {
                format!("{} ({})", b.what, b.source)
            };
            g.boxes.push(GeoBox {
                kind: b.kind.clone(),
                what,
                rect,
                frames: b.frames.clone(),
            });
        }
        for s in f.slot.iter().filter(|s| s.id == id) {
            g.danger = s.danger.clone();
            g.danger_what = s.danger_what.clone();
        }
    }
    let touch = g.boxes.iter().find(|b| b.kind == "touch").map(|b| b.rect);
    let side = |v: i64, max: u32| v.max(MIN_SIDE).min(i64::from(max));
    g.min_size = match touch {
        Some(r) => [side(r[2], pic[0]), side(r[3], pic[1])],
        None => [side(0, pic[0]), side(0, pic[1])],
    };
    g
}

/// Each level's surfaces as rules, from the model.
pub fn surfaces(m: &Model) -> Vec<Surface> {
    let mut out = Vec::new();
    for (name, l) in &m.levels {
        let p = &l.platformer;
        if p.ground.is_none() && p.ladders.is_none() && l.core.tile_surfaces.is_empty() {
            continue;
        }
        let mut tops: Vec<u8> = l.core.tile_surfaces.values().copied().collect();
        tops.sort_unstable();
        tops.dedup();
        let mut floor = "a body stands on the top edge of each ground box: draw each platform with its top edge on the floor line and nothing standing above it where the hero walks".to_owned();
        if !tops.is_empty() {
            floor.push_str(&format!(
                "; a surface tile's top edge sits {} px down its {} px cell, by the tile",
                tops.iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                l.core.tile_size
            ));
        }
        let climb = match &p.ladders {
            Some(_) => "a ladder is a box the hero climbs where its grab box meets it; draw its rungs over the whole box, and a broken ladder (one that stops short of the floor above) as a stub that ends where its box ends".to_owned(),
            None => String::new(),
        };
        let below = p
            .kill_below
            .map(|k| format!("a body below row {} is out", int(k)))
            .unwrap_or_default();
        out.push(Surface {
            level: name.clone(),
            picture: l
                .core
                .layout
                .as_ref()
                .map(|lay| format!("picture.{}.{}", lay.slot, lay.entry))
                .unwrap_or_default(),
            floor,
            tile_tops: tops,
            climb,
            below,
        });
    }
    out
}

/// A stage's floors and ladders, from its compiled level.
pub fn stage_of(spec: &gp_sim::Spec, level: usize) -> Stage {
    let lay = gp_interp::stage::layout(spec, level);
    let mut steps = BTreeMap::new();
    for g in &lay.girders {
        for w in g.pieces.windows(2) {
            let d = (w[1][1] - w[0][1]).abs();
            if d > 0 {
                *steps.entry(d).or_default() += 1;
            }
        }
    }
    let ladders = lay
        .ladders
        .iter()
        .map(|l| {
            let top = l.upper.and_then(|u| lay.girders[u].surface(l.cx()));
            LadderZone {
                rect: l.bx,
                lower: l.lower,
                upper: l.upper,
                top,
                broken: top.is_some_and(|t| l.bx[1] > t + 4) || l.upper.is_none(),
            }
        })
        .collect();
    Stage {
        level: spec.levels[level].name.clone(),
        floors: lay.girders.iter().map(|g| g.pieces.clone()).collect(),
        steps,
        ladders,
    }
}

// --- The check ----------------------------------------------------------------

/// A visible body: `[x0, y0, x1, y1]`, the right and bottom past it.
pub type Body = [i64; 4];

/// The visible body of a frame of tones (0 clear), `w` wide.
pub fn body_of(t: &[u8], w: u32) -> Option<Body> {
    let w = w.max(1) as usize;
    let mut b: Option<Body> = None;
    for (i, v) in t.iter().enumerate() {
        if *v == 0 {
            continue;
        }
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        b = Some(match b {
            None => [x, y, x + 1, y + 1],
            Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)],
        });
    }
    b
}

/// What the check found wrong with one frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Flag {
    pub slot: String,
    /// The frame's name, or the floor or ladder.
    pub at: String,
    pub what: String,
    /// `warn` or `refuse`.
    pub verdict: String,
}

/// A themed frame's body against the original's and the slot's geometry:
/// what's wrong, in words.
pub fn frame_problems(
    g: &Geometry,
    frame: &str,
    orig: Option<Body>,
    themed: Option<Body>,
    tol: &Tolerance,
) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let danger = g.danger.iter().any(|d| d == frame);
    match (orig, themed) {
        (None, Some(t)) => out.push((
            format!(
                "draws {} by {} where the original draws nothing",
                t[2] - t[0],
                t[3] - t[1]
            ),
            false,
        )),
        (Some(_), None) => out.push(("draws nothing where the original draws".into(), danger)),
        (Some(o), Some(t)) => {
            let (ow, oh, tw, th) = (o[2] - o[0], o[3] - o[1], t[2] - t[0], t[3] - t[1]);
            if tw > ow + tol.size || th > oh + tol.size {
                out.push((
                    format!("too big: {tw} by {th}, the original's body {ow} by {oh}"),
                    false,
                ));
            }
            if tw < ow - tol.size || th < oh - tol.size {
                out.push((
                    format!("too small: {tw} by {th}, the original's body {ow} by {oh}"),
                    false,
                ));
            }
            // Middles in half pixels.
            let dx = (t[0] + t[2]) - (o[0] + o[2]);
            let dy = (t[1] + t[3]) - (o[1] + o[3]);
            if dx.abs() > 2 * tol.edge || dy.abs() > 2 * tol.edge {
                out.push((
                    format!(
                        "off its box: its middle {:+.1}, {:+.1} px from the original's",
                        dx as f64 / 2.0,
                        dy as f64 / 2.0
                    ),
                    false,
                ));
            }
            let stands = g.feet.is_some_and(|f| (o[3] - f).abs() <= tol.feet);
            if stands && (t[3] - o[3]).abs() > tol.feet {
                out.push((
                    format!(
                        "stands {} px {} the original's feet",
                        (t[3] - o[3]).abs(),
                        if t[3] > o[3] { "below" } else { "above" }
                    ),
                    false,
                ));
            }
        }
        (None, None) => {}
    }
    let sized = out.iter().any(|(w, _)| w.starts_with("too "));
    if let (Some(_), Some(t), false) = (orig, themed, sized) {
        let (tw, th) = (t[2] - t[0], t[3] - t[1]);
        if tw < g.min_size[0] || th < g.min_size[1] {
            out.push((
                format!(
                    "under the smallest body, {} by {}: {tw} by {th}",
                    g.min_size[0], g.min_size[1]
                ),
                false,
            ));
        }
    }
    if danger {
        for b in g.boxes.iter().filter(|b| {
            matches!(b.kind.as_str(), "touch" | "hit")
                && (b.frames.is_empty() || b.frames.iter().any(|f| f == frame))
        }) {
            let c = covered(b.rect, themed);
            if c < tol.cover {
                out.push((
                    format!(
                        "its {} box is {:.0}% drawn on, under {:.0}%",
                        b.kind,
                        c * 100.0,
                        tol.cover * 100.0
                    ),
                    c == 0.0,
                ));
            }
        }
    }
    out
}

/// The picture's edges a frame's ink runs into (3 or more inked pixels on
/// the edge's row or column) where the original's body doesn't reach that
/// edge: a shape cut off by the picture.
pub fn clipped(t: &[u8], w: u32, h: u32, orig: Option<Body>) -> Vec<&'static str> {
    let (w, h) = (w as i64, h as i64);
    let ink = |x: i64, y: i64| t[(y * w + x) as usize] != 0;
    let count = |pts: &mut dyn Iterator<Item = (i64, i64)>| pts.filter(|&(x, y)| ink(x, y)).count();
    let o = orig.unwrap_or([w, h, 0, 0]);
    let mut out = Vec::new();
    if o[0] > 0 && count(&mut (0..h).map(|y| (0, y))) >= 3 {
        out.push("left");
    }
    if o[2] < w && count(&mut (0..h).map(|y| (w - 1, y))) >= 3 {
        out.push("right");
    }
    if o[1] > 0 && count(&mut (0..w).map(|x| (x, 0))) >= 3 {
        out.push("top");
    }
    if o[3] < h && count(&mut (0..w).map(|x| (x, h - 1))) >= 3 {
        out.push("bottom");
    }
    out
}

/// The rows (or columns) a part's side must run straight along a tile's
/// edge to be flagged as cut.
pub const CUT_RUN: usize = 6;

/// The sides where a frame's ink is cut straight along a tile's edge
/// (a multiple of 8 px) and the original's ink goes on past it: in
/// `CUT_RUN` or more rows in a run (columns, for the top and bottom) the
/// ink ends at that edge while the original's reaches more than `edge` px
/// further. A part drawn half, such as a hand stopped where a tile its
/// frame doesn't draw begins, reads this way. A side the original also
/// ends at isn't flagged, nor a straight side inside a tile, which is a
/// shape's own. `t` and `o` are the themed and original frames' ink, `w`
/// by `h`.
pub fn cut_flat(t: &[u8], o: &[u8], w: u32, h: u32, edge: i64) -> Vec<String> {
    let (w, h) = (w as i64, h as i64);
    let mut out = Vec::new();
    for side in ["left", "right", "top", "bottom"] {
        // Lines across the side (rows for left and right), each read from
        // the side inward: `n` lines of `len` pixels.
        let across = matches!(side, "left" | "right");
        let (n, len) = if across { (h, w) } else { (w, h) };
        let at = |f: &[u8], line: i64, d: i64| -> bool {
            let (x, y) = match side {
                "left" => (d, line),
                "right" => (w - 1 - d, line),
                "top" => (line, d),
                _ => (line, h - 1 - d),
            };
            f[(y * w + x) as usize] != 0
        };
        // The pixel the ink's outermost reaches on this line, counted in
        // from the side.
        let reach = |f: &[u8], line: i64| (0..len).find(|&d| at(f, line, d));
        let mut run: Vec<(i64, i64)> = Vec::new();
        let mut flag = |run: &mut Vec<(i64, i64)>| {
            if run.len() >= CUT_RUN {
                let (a, b, d) = (run[0].0, run[run.len() - 1].0, run[0].1);
                let px = if matches!(side, "right" | "bottom") {
                    len - d
                } else {
                    d
                };
                out.push(format!(
                    "a part cut straight at a tile's edge on its {side} side: {} {a} to {b} stop at {} {px}, where the original reaches further",
                    if across { "rows" } else { "columns" },
                    if across { "x" } else { "y" }
                ));
            }
            run.clear();
        };
        for line in 0..n {
            // On a tile's edge: the outermost inked pixel, counted from
            // the side, is a tile's first (frames are whole tiles).
            let cut = match (reach(t, line), reach(o, line)) {
                (Some(dt), Some(dd)) if dt - dd > edge && dt % 8 == 0 => Some(dt),
                _ => None,
            };
            match (cut, run.last()) {
                (Some(d), Some(&(_, c))) if d == c => run.push((line, d)),
                (Some(d), _) => {
                    flag(&mut run);
                    run.push((line, d));
                }
                (None, _) => flag(&mut run),
            }
        }
        flag(&mut run);
    }
    out
}

/// The share of a box's area, `[x, y, w, h]`, inside a body.
pub fn covered(r: [i64; 4], body: Option<Body>) -> f64 {
    let Some(b) = body else { return 0.0 };
    let area = (r[2] * r[3]).max(1);
    let w = (r[0] + r[2]).min(b[2]) - r[0].max(b[0]);
    let h = (r[1] + r[3]).min(b[3]) - r[1].max(b[1]);
    (w.max(0) * h.max(0)) as f64 / area as f64
}

/// Each frame's flags, less the ones the original's own art has.
pub fn slot_flags(
    id: &str,
    g: &Geometry,
    names: &[String],
    orig: &[Option<Body>],
    themed: &[Option<Body>],
    tol: &Tolerance,
) -> Vec<Flag> {
    let mut out = Vec::new();
    for (k, name) in names.iter().enumerate() {
        let (o, t) = (
            orig.get(k).copied().flatten(),
            themed.get(k).copied().flatten(),
        );
        // A flag's kind: its words up to the first number or colon.
        let kind = |w: &str| {
            w.split(|c: char| c == ':' || c.is_ascii_digit())
                .next()
                .unwrap_or_default()
                .to_owned()
        };
        let own: Vec<String> = frame_problems(g, name, o, o, tol)
            .into_iter()
            .map(|(w, _)| kind(&w))
            .collect();
        for (what, refuse) in frame_problems(g, name, o, t, tol) {
            let head = kind(&what);
            if own.contains(&head) {
                continue;
            }
            out.push(Flag {
                slot: id.into(),
                at: name.clone(),
                what,
                verdict: if refuse { "refuse" } else { "warn" }.into(),
            });
        }
    }
    out
}

/// Each frame of a figure placed on a stage, its drawn feet against the
/// floor it stands on: the floor piece under the body's middle column
/// whose row is nearest the original's feet, from 8 rows under them up
/// to half the body's height over them (a floor the original's legs are
/// drawn through counts). `at` is the picture's
/// top left on the screen (the spawn place and the sprite's offset). A
/// frame is held to the floor when the original stands on it within the
/// feet tolerance, and the base frame (the one drawn at the spawn place)
/// always: a figure whose art, the original's too, sinks into the floor
/// or floats over it is flagged. Before 2026-11-04 the feet were checked
/// only against the original's, and only for a slot with a `feet` row (a
/// hero, or a body that harms on touch), so a figure the hero doesn't
/// fight drawn through its floor passed.
pub fn stand_flags(
    id: &str,
    names: &[String],
    orig: &[Option<Body>],
    themed: &[Option<Body>],
    at: [i64; 2],
    stage: &Stage,
    tol: &Tolerance,
) -> Vec<Flag> {
    let mut out = Vec::new();
    for (k, o) in orig.iter().enumerate() {
        let (Some(o), Some(t)) = (*o, themed.get(k).copied().flatten()) else {
            continue;
        };
        let feet = at[1] + o[3];
        let mid = at[0] + (o[0] + o[2]) / 2;
        let Some(row) = stage
            .floors
            .iter()
            .flatten()
            .filter(|p| {
                mid >= p[0] && mid <= p[2] && p[1] <= feet + 8 && p[1] >= feet - (o[3] - o[1]) / 2
            })
            .map(|p| p[1])
            .min_by_key(|r| (feet - r).abs())
        else {
            continue;
        };
        let stands = (feet - row).abs() <= tol.feet;
        if !stands && k != 0 {
            continue;
        }
        let d = at[1] + t[3] - row;
        if d.abs() <= tol.feet {
            continue;
        }
        let side = |d: i64| if d > 0 { "below" } else { "above" };
        let mut what = format!(
            "its feet {} px {} the floor it stands on (row {row})",
            d.abs(),
            side(d)
        );
        if !stands {
            what.push_str(&format!(
                ", the original's {} px {}",
                (feet - row).abs(),
                side(feet - row)
            ));
        }
        out.push(Flag {
            slot: id.into(),
            at: names.get(k).cloned().unwrap_or_else(|| k.to_string()),
            what,
            verdict: "warn".into(),
        });
    }
    out
}

/// The top inked row near `row` in column `x` of a picture's tones, from
/// 8 rows above to 8 below.
fn top_near(t: &[u8], w: u32, h: u32, x: i64, row: i64) -> Option<i64> {
    (row - 8..=row + 8)
        .filter(|&y| x >= 0 && y >= 0 && x < i64::from(w) && y < i64::from(h))
        .find(|&y| t[(y as u32 * w + x as u32) as usize] != 0)
}

/// A themed stage's floors against the original's: each floor whose top
/// edge misses the original's by more than the tolerance on more than
/// [`FLOOR_MISS`] of its columns, and each ladder the theme leaves mostly
/// undrawn. Columns under a ladder are skipped.
pub fn stage_flags(
    id: &str,
    stage: &Stage,
    orig: &[u8],
    themed: &[u8],
    size: [u32; 2],
    tol: &Tolerance,
) -> Vec<Flag> {
    let [w, h] = size;
    let mut out = Vec::new();
    let under_ladder = |x: i64| {
        stage
            .ladders
            .iter()
            .any(|l| x >= l.rect[0] - 1 && x <= l.rect[2] + 1)
    };
    for (i, f) in stage.floors.iter().enumerate() {
        let (mut cols, mut miss, mut worst) = (0usize, 0usize, 0i64);
        for p in f {
            for x in (p[0]..=p[2]).step_by(2) {
                if under_ladder(x) {
                    continue;
                }
                let Some(o) = top_near(orig, w, h, x, p[1]) else {
                    continue;
                };
                cols += 1;
                let d = match top_near(themed, w, h, x, p[1]) {
                    Some(t) => t - o,
                    None => 99,
                };
                if d.abs() > tol.floor {
                    miss += 1;
                    if d.abs() > worst.abs() {
                        worst = d;
                    }
                }
            }
        }
        if cols > 0 && miss as f64 > FLOOR_MISS * cols as f64 {
            out.push(Flag {
                slot: id.into(),
                at: format!("floor {i}"),
                what: if worst == 99 {
                    format!("top edge missing on {miss} of {cols} columns")
                } else {
                    format!(
                        "top edge off the floor line on {miss} of {cols} columns, by up to {worst:+} px"
                    )
                },
                verdict: "warn".into(),
            });
        }
    }
    for (i, l) in stage.ladders.iter().enumerate() {
        let r = l.rect;
        let rows = |t: &[u8]| {
            (r[1]..=r[3])
                .filter(|&y| {
                    (r[0]..=r[2]).any(|x| {
                        x >= 0
                            && y >= 0
                            && x < i64::from(w)
                            && y < i64::from(h)
                            && t[(y as u32 * w + x as u32) as usize] != 0
                    })
                })
                .count()
        };
        let all = (r[3] - r[1] + 1).max(1) as f64;
        let (o, t) = (rows(orig) as f64 / all, rows(themed) as f64 / all);
        if t < 0.5 && o >= 0.5 {
            out.push(Flag {
                slot: id.into(),
                at: format!("ladder {i}"),
                what: format!(
                    "{}drawn on {:.0}% of its rows (the original {:.0}%)",
                    if l.broken { "broken ladder " } else { "" },
                    t * 100.0,
                    o * 100.0
                ),
                verdict: "warn".into(),
            });
        }
    }
    out
}

/// How far up the drawn top of a raised floor is followed.
pub const RAISE_MAX: i64 = 24;

/// What a column of a stage picture draws at the row the feet stand on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drawn {
    /// The drawn top, within the tolerance of the surface or past it.
    Top(i64),
    /// Nothing within 8 rows under the surface.
    Missing,
}

/// The drawn top in column `x` of a picture's ink over the surface row
/// `s`: the top of the inked run that reaches the surface (within `tol`),
/// followed up to [`RAISE_MAX`] rows over gaps of up to 2; else the first ink up to 8 rows
/// under it.
fn drawn_top(t: &[u8], w: u32, h: u32, x: i64, s: i64, tol: i64) -> Drawn {
    let ink = |y: i64| {
        x >= 0
            && y >= 0
            && x < i64::from(w)
            && y < i64::from(h)
            && t[(y as u32 * w + x as u32) as usize] != 0
    };
    match (s - tol..=s + tol).find(|&y| ink(y)) {
        Some(mut y) => {
            // Up through the ink, over gaps of up to 2 rows (a thing drawn
            // a pixel clear of the floor still stands on it).
            while let Some(up) = (1..=3).find(|&d| y - d >= s - RAISE_MAX && ink(y - d)) {
                y -= up;
            }
            Drawn::Top(y)
        }
        None => (s + tol + 1..=s + 8)
            .find(|&y| ink(y))
            .map_or(Drawn::Missing, Drawn::Top),
    }
}

/// A themed stage's drawn floor against the walkable surface, at every x
/// of every floor piece (none under a ladder): the drawn top of the
/// girder must be the row the feet stand on, within the floor tolerance.
/// A run of columns whose drawn top is higher (a step, an overlap or a
/// stacked tile the hero walks through, which reads as a wall or ledge),
/// lower (the feet stand on air) or missing (no girder drawn under the
/// surface) is one flag, with its x range and rows. With the original's
/// ink, a column the original draws the same way (its top within the
/// tolerance, at that column or one within the tolerance of it) is not
/// flagged: a thing it stands on the floor (a drum) is the
/// original's. Added on 2026-11-04, when the user saw Rivet Rodeo's paper
/// draw girder tiles stacked a cell over its sloped girders; the floor
/// line check ([`stage_flags`]) holds a floor only to the original's, and
/// only when more than [`FLOOR_MISS`] of its columns miss.
pub fn surface_flags(
    id: &str,
    stage: &Stage,
    orig: Option<&[u8]>,
    themed: &[u8],
    size: [u32; 2],
    tol: &Tolerance,
) -> Vec<Flag> {
    let [w, h] = size;
    let under_ladder = |x: i64| {
        stage
            .ladders
            .iter()
            .any(|l| x >= l.rect[0] - 1 && x <= l.rect[2] + 1)
    };
    // A column's fault: the kind (raised, low, missing), its drawn top
    // and the surface.
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Raised,
        Low,
        Missing,
    }
    let mut out = Vec::new();
    for (i, f) in stage.floors.iter().enumerate() {
        // Each column's surface; where two pieces share a column, the
        // higher.
        let mut surf: BTreeMap<i64, i64> = BTreeMap::new();
        for p in f {
            for x in p[0]..=p[2] {
                let s = surf.entry(x).or_insert(p[1]);
                *s = (*s).min(p[1]);
            }
        }
        let mut cols: Vec<(i64, Kind, i64, i64)> = Vec::new();
        {
            for (&x, &s) in &surf {
                if under_ladder(x) || x < 0 || x >= i64::from(w) {
                    continue;
                }
                let d = drawn_top(themed, w, h, x, s, tol.floor);
                let fault = match d {
                    Drawn::Top(t) if (t - s).abs() <= tol.floor => None,
                    Drawn::Top(t) if t < s => Some((Kind::Raised, t)),
                    Drawn::Top(t) => Some((Kind::Low, t)),
                    Drawn::Missing => Some((Kind::Missing, s)),
                };
                let Some((k, t)) = fault else { continue };
                // The original's column, or one within the tolerance of
                // it (a thing's edge a pixel over), drawn the same way.
                let shared = orig.is_some_and(|o| {
                    (x - tol.floor..=x + tol.floor).any(|ox| {
                        match (drawn_top(o, w, h, ox, s, tol.floor), d) {
                            (Drawn::Top(a), Drawn::Top(b)) => (a - b).abs() <= tol.floor,
                            (Drawn::Missing, Drawn::Missing) => true,
                            _ => false,
                        }
                    })
                });
                if !shared {
                    cols.push((x, k, t, s));
                }
            }
        }
        // Runs of neighbouring columns of one kind.
        let mut k = 0;
        while k < cols.len() {
            let mut e = k;
            while e + 1 < cols.len() && cols[e + 1].0 == cols[e].0 + 1 && cols[e + 1].1 == cols[k].1
            {
                e += 1;
            }
            let run = &cols[k..=e];
            let (x0, x1) = (run[0].0, run[run.len() - 1].0);
            let rows = |v: &mut dyn Iterator<Item = i64>| {
                let v: Vec<i64> = v.collect();
                let (a, b) = (*v.iter().min().unwrap_or(&0), *v.iter().max().unwrap_or(&0));
                if a == b {
                    format!("row {a}")
                } else {
                    format!("rows {a} to {b}")
                }
            };
            let surf = rows(&mut run.iter().map(|c| c.3));
            let top = rows(&mut run.iter().map(|c| c.2));
            let what = match run[0].1 {
                Kind::Raised => {
                    let most = run.iter().map(|c| c.3 - c.2).max().unwrap_or(0);
                    format!(
                        "drawn top at {top}, up to {most} px over the surface the feet stand on ({surf}): a step or stacked tile the hero walks through"
                    )
                }
                Kind::Low => {
                    let most = run.iter().map(|c| c.2 - c.3).max().unwrap_or(0);
                    format!(
                        "drawn top at {top}, up to {most} px under the surface the feet stand on ({surf}): the feet stand on air"
                    )
                }
                Kind::Missing => format!("no girder drawn under the surface ({surf})"),
            };
            out.push(Flag {
                slot: id.into(),
                at: format!("floor {i} x {x0} to {x1}"),
                what,
                verdict: "warn".into(),
            });
            k = e + 1;
        }
    }
    out
}

/// A picture's ink: 1 where a pixel is opaque and well apart from the
/// backdrop (its most common color, with any color within 12 of it
/// (CIE76), as paper's speckles are), else 0.
pub fn ink_of(img: &gp_theme::pix::Rgba) -> Vec<u8> {
    use gp_theme::color::{Rgb, delta_e};
    let mut count: BTreeMap<[u8; 4], usize> = BTreeMap::new();
    for p in &img.pixels {
        *count.entry(*p).or_default() += 1;
    }
    let Some((bg, _)) = count.iter().max_by_key(|(_, n)| **n) else {
        return Vec::new();
    };
    let bg = Rgb([bg[0], bg[1], bg[2]]);
    let mut near: BTreeMap<[u8; 4], bool> = BTreeMap::new();
    img.pixels
        .iter()
        .map(|p| {
            let far = *near
                .entry(*p)
                .or_insert_with(|| p[3] >= 128 && delta_e(Rgb([p[0], p[1], p[2]]), bg) > 12.0);
            u8::from(far)
        })
        .collect()
}

/// The geometry check of a themed game: each sprite slot's frames against
/// the original's bodies and its boxes, and each stage's floors and
/// ladders (its picture when the theme draws one, else its surface tiles).
#[allow(clippy::too_many_arguments)]
pub fn check(
    m: &Model,
    kit: &crate::theme_kit::Kit,
    local: &crate::theme_kit::Local,
    theme: &gp_theme::Resolved,
    assets: &Path,
    map: &gp_assets::AssetMap,
    spec: &gp_sim::Spec,
) -> Result<(Vec<Flag>, usize)> {
    use crate::theme_original::{frame_of, layout_tones};
    let banks = crate::theme_original::banks_of(m, local, theme, assets, map)?;
    let tol = &kit.tolerance;
    let mut out = Vec::new();
    let mut checked = 0;
    // Each sprite slot's size and its frames' bodies, original and themed.
    type Bodies = ([u32; 2], Vec<Option<Body>>, Vec<Option<Body>>);
    let mut figures: BTreeMap<String, Bodies> = BTreeMap::new();
    for a in kit.art.iter().filter(|a| a.kind == "sprite") {
        let (Some(g), Some(cuts)) = (&a.geometry, local.cuts.get(&a.id)) else {
            continue;
        };
        let (mut o, mut t, mut any) = (Vec::new(), Vec::new(), false);
        for (k, c) in cuts.iter().enumerate() {
            if let Some((orig, Some(th), sz)) = banks.get(&c.slot) {
                let f = frame_of(orig, *sz, c);
                let ob = body_of(&f.t, f.w);
                o.push(ob);
                let of = f;
                let f = frame_of(th, *sz, c);
                t.push(body_of(&f.t, f.w));
                for what in cut_flat(&f.t, &of.t, f.w, f.h, tol.edge) {
                    out.push(Flag {
                        slot: a.id.clone(),
                        at: a
                            .frame_names
                            .get(k)
                            .cloned()
                            .unwrap_or_else(|| k.to_string()),
                        what,
                        verdict: "warn".into(),
                    });
                }
                for edge in clipped(&f.t, f.w, f.h, ob) {
                    out.push(Flag {
                        slot: a.id.clone(),
                        at: a
                            .frame_names
                            .get(k)
                            .cloned()
                            .unwrap_or_else(|| k.to_string()),
                        what: format!("clipped at the picture's {edge} edge"),
                        verdict: "warn".into(),
                    });
                }
                any = true;
            } else {
                o.push(None);
                t.push(None);
            }
        }
        if any {
            checked += 1;
            out.extend(slot_flags(&a.id, g, &a.frame_names, &o, &t, tol));
            figures.insert(a.id.clone(), (a.size, o, t));
        }
    }
    // Each figure's feet on the floor it is placed on, for each level
    // drawn as a stage picture.
    let names = crate::theme_kit::names_of(m);
    for (li, l) in spec.levels.iter().enumerate() {
        let Some(ml) = m.levels.get(&l.name) else {
            continue;
        };
        if ml.core.layout.is_none() {
            continue;
        }
        let stage = stage_of(spec, li);
        for sp in &ml.core.spawn {
            let Some(e) = m.entities.get(&sp.entity) else {
                continue;
            };
            let id = crate::theme_kit::entity_id(&sp.entity, &e.core.role, &names);
            let Some((size, o, t)) = figures.get(&id) else {
                continue;
            };
            // Figures: a hero, a foe, a character. A pickup or a tool
            // hangs where the stage puts it.
            let Some(a) = kit.art.iter().find(|a| {
                a.id == id && matches!(a.role.as_str(), "player" | "enemy" | "boss" | "npc")
            }) else {
                continue;
            };
            let off = picture_offset(e, *size);
            let (x, y) = (sp.x.map(int).unwrap_or(0), sp.y.map(int).unwrap_or(0));
            out.extend(stand_flags(
                &id,
                &a.frame_names,
                o,
                t,
                [x + off[0], y + off[1]],
                &stage,
                tol,
            ));
        }
    }
    for (li, l) in spec.levels.iter().enumerate() {
        let Some(ml) = m.levels.get(&l.name) else {
            continue;
        };
        let Some(lay) = &ml.core.layout else { continue };
        let id = format!("picture.{}.{}", lay.slot, lay.entry);
        let stage = stage_of(spec, li);
        if let Some((_, file, _)) = theme.restyle_entry(&lay.slot, &lay.entry) {
            let size = kit.screen;
            let Some(orig) =
                layout_tones(m, assets, &banks, &lay.slot, &lay.entry, size[0], size[1])
            else {
                continue;
            };
            let img = gp_theme::pix::Rgba::load(&file).map_err(anyhow::Error::msg)?;
            let img = crate::theme_paper::scale_to(&img, size[0], size[1]);
            let themed = ink_of(&img);
            checked += 1;
            out.extend(stage_flags(&id, &stage, &orig.t, &themed, size, tol));
            out.extend(surface_flags(
                &id,
                &stage,
                Some(&orig.t),
                &themed,
                size,
                tol,
            ));
            continue;
        }
        // No stage picture: the surface tiles' top edges.
        let Some(slot) = gp_look::present::level_tiles_slot(m) else {
            continue;
        };
        let Some((orig, Some(th), sz)) = banks.get(&slot) else {
            continue;
        };
        let top = |bank: &[u8], t: u32| {
            let (tx, ty) = ((t % sz[0]) * 8, (t / sz[0]) * 8);
            (0..8i64).find(|&y| {
                (0..8).any(|x| {
                    bank.get(((ty + y as u32) * sz[0] * 8 + tx + x) as usize)
                        .is_some_and(|v| *v != 0)
                })
            })
        };
        checked += 1;
        for (tile, k) in &ml.core.tile_surfaces {
            let Ok(t) = tile.parse::<u32>() else { continue };
            if t >= sz[0] * sz[1] {
                continue;
            }
            let (o, th) = (top(orig, t), top(th, t));
            if o.is_none() {
                continue;
            }
            if th.is_none_or(|v| (v - o.unwrap_or(0)).abs() > tol.floor) {
                out.push(Flag {
                    slot: slot.clone(),
                    at: format!("surface tile {k} px down"),
                    what: match th {
                        Some(v) => format!(
                            "its top edge is {v} px down its cell, the floor line {} (the original's)",
                            o.unwrap_or(0)
                        ),
                        None => "draws nothing where the floor is".into(),
                    },
                    verdict: "warn".into(),
                });
            }
        }
    }
    Ok((out, checked))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up 16 by 16 frame of tones with a block from (x0, y0) to
    /// (x1, y1), right and bottom past it.
    fn block(x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<u8> {
        let mut t = vec![0u8; 256];
        for y in y0..y1 {
            for x in x0..x1 {
                t[y * 16 + x] = 2;
            }
        }
        t
    }

    fn hazard() -> Geometry {
        Geometry {
            boxes: vec![GeoBox {
                kind: "touch".into(),
                what: "harms".into(),
                rect: [2, 9, 12, 4],
                frames: Vec::new(),
            }],
            danger: vec!["base".into()],
            danger_what: "harms".into(),
            feet: Some(13),
            min_size: [12, 6],
            contrast: String::new(),
        }
    }

    #[test]
    fn a_made_up_sprite_off_its_box_too_big_or_too_small_is_flagged() {
        let tol = Tolerance::default();
        let g = hazard();
        let names = vec!["base".to_owned()];
        let orig = vec![body_of(&block(2, 3, 14, 13), 16)];
        // The same body: nothing to say.
        let same = vec![body_of(&block(2, 3, 14, 13), 16)];
        assert!(slot_flags("x", &g, &names, &orig, &same, &tol).is_empty());
        // Within the tolerances: a pixel lower, two wider.
        let near = vec![body_of(&block(1, 4, 15, 14), 16)];
        assert!(slot_flags("x", &g, &names, &orig, &near, &tol).is_empty());
        // Too big.
        let big = slot_flags(
            "x",
            &g,
            &names,
            &orig,
            &[body_of(&block(0, 0, 16, 16), 16)],
            &tol,
        );
        assert!(big.iter().any(|f| f.what.starts_with("too big")), "{big:?}");
        // Too small, and its harmful box hardly drawn on.
        let small = slot_flags(
            "x",
            &g,
            &names,
            &orig,
            &[body_of(&block(6, 2, 10, 8), 16)],
            &tol,
        );
        assert!(
            small.iter().any(|f| f.what.starts_with("too small")),
            "{small:?}"
        );
        assert!(
            small
                .iter()
                .any(|f| f.what.contains("touch box") && f.verdict == "refuse"),
            "{small:?}"
        );
        // Moved: off its box, and standing below the feet.
        let moved = slot_flags(
            "x",
            &g,
            &names,
            &orig,
            &[body_of(&block(2, 6, 14, 16), 16)],
            &tol,
        );
        assert!(
            moved.iter().any(|f| f.what.starts_with("off its box")),
            "{moved:?}"
        );
        assert!(
            moved
                .iter()
                .any(|f| f.what.starts_with("stands 3 px below")),
            "{moved:?}"
        );
        // A frame the original leaves empty, drawn.
        let ghost = slot_flags("x", &g, &names, &[None], &same, &tol);
        assert!(ghost[0].what.contains("draws nothing"), "{ghost:?}");
    }

    #[test]
    fn a_flag_the_original_has_too_is_not_raised() {
        // The original's own body misses its touch box (a box low in the
        // picture): a theme that copies its place isn't flagged for it.
        let tol = Tolerance::default();
        let mut g = hazard();
        g.boxes[0].rect = [2, 14, 12, 2];
        let o = vec![body_of(&block(2, 2, 14, 12), 16)];
        assert!(!frame_problems(&g, "base", o[0], o[0], &tol).is_empty());
        assert!(slot_flags("x", &g, &["base".into()], &o, &o, &tol).is_empty());
    }

    #[test]
    fn a_floor_off_its_line_and_an_undrawn_ladder_are_flagged() {
        let (w, h) = (64u32, 32u32);
        let floor = |row: usize, gap: Option<(usize, usize)>| {
            let mut t = vec![0u8; (w * h) as usize];
            for x in 0..w as usize {
                for y in row..row + 4 {
                    t[y * w as usize + x] = 1;
                }
            }
            if let Some((x0, x1)) = gap {
                for y in 8..row {
                    for x in x0..=x1 {
                        t[y * w as usize + x] = 2;
                    }
                }
            }
            t
        };
        let stage = Stage {
            level: "main".into(),
            floors: vec![vec![[0, 19, 63]]],
            steps: BTreeMap::new(),
            ladders: vec![LadderZone {
                rect: [40, 8, 47, 19],
                lower: Some(0),
                upper: None,
                top: None,
                broken: false,
            }],
        };
        let tol = Tolerance::default();
        let orig = floor(20, Some((40, 47)));
        // The same floor, the ladder kept: nothing.
        assert!(stage_flags("s", &stage, &orig, &orig, [w, h], &tol).is_empty());
        // A floor a pixel low is within the tolerance; three is not.
        let low = floor(21, Some((40, 47)));
        assert!(stage_flags("s", &stage, &orig, &low, [w, h], &tol).is_empty());
        let lower = floor(23, None);
        let f = stage_flags("s", &stage, &orig, &lower, [w, h], &tol);
        assert!(
            f.iter()
                .any(|f| f.at == "floor 0" && f.what.contains("by up to +3")),
            "{f:?}"
        );
        assert!(f.iter().any(|f| f.at == "ladder 0"), "{f:?}");
    }

    #[test]
    fn a_stacked_tile_on_a_sloped_floor_is_flagged() {
        // A made-up 128 by 32 girder sloping up a pixel each 40 columns,
        // drawn 8 rows deep from the row the feet stand on.
        let (w, h) = (128u32, 32u32);
        let stage = Stage {
            level: "main".into(),
            floors: vec![vec![[0, 20, 39], [40, 19, 79], [80, 18, 127]]],
            steps: BTreeMap::new(),
            ladders: Vec::new(),
        };
        let mut good = vec![0u8; (w * h) as usize];
        for p in &stage.floors[0] {
            for x in p[0]..=p[2] {
                for y in p[1]..p[1] + 8 {
                    good[(y * i64::from(w) + x) as usize] = 1;
                }
            }
        }
        let tol = Tolerance::default();
        // Drawn as it stands, 1 px steps and all: nothing to say.
        assert!(surface_flags("s", &stage, None, &good, [w, h], &tol).is_empty());
        // A girder tile stacked a cell up over x 48 to 55, its top 7 px
        // over the surface at row 19, and nothing drawn at x 100 to 103.
        let mut bad = good.clone();
        for x in 48..56 {
            for y in 12..19 {
                bad[y * w as usize + x] = 1;
            }
        }
        for x in 100..104 {
            for y in 0..h as usize {
                bad[y * w as usize + x] = 0;
            }
        }
        // The old floor line check passes it: 6 of 64 columns miss, under
        // a tenth.
        assert!(stage_flags("s", &stage, &good, &bad, [w, h], &tol).is_empty());
        let f = surface_flags("s", &stage, Some(&good), &bad, [w, h], &tol);
        assert_eq!(f.len(), 2, "{f:?}");
        assert_eq!(f[0].at, "floor 0 x 48 to 55");
        assert!(f[0].what.contains("up to 7 px over"), "{f:?}");
        assert!(f[0].what.contains("(row 19)"), "{f:?}");
        assert_eq!(f[1].at, "floor 0 x 100 to 103");
        assert!(f[1].what.starts_with("no girder drawn"), "{f:?}");
        // The floor drawn 3 px low there: the feet stand on air.
        let mut low = good.clone();
        for x in 0..40 {
            for y in 20..23 {
                low[y * w as usize + x] = 0;
            }
        }
        let f = surface_flags("s", &stage, None, &low, [w, h], &tol);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].what.contains("3 px under"), "{f:?}");
        // What the original draws the same way (a thing it stands on the
        // floor) is the original's; without it, flagged.
        assert_eq!(
            surface_flags("s", &stage, Some(&bad), &bad, [w, h], &tol),
            Vec::new()
        );
        assert_eq!(
            surface_flags("s", &stage, None, &bad, [w, h], &tol).len(),
            2
        );
    }

    #[test]
    fn a_shape_cut_off_by_the_pictures_edge_is_flagged() {
        // A made-up body 2 px in from every edge in the original; a theme
        // whose shape runs into the bottom and right edges is clipped
        // there, and one inside the frame isn't.
        let orig = Some([2, 2, 14, 14]);
        assert_eq!(
            clipped(&block(4, 4, 16, 16), 16, 16, orig),
            vec!["right", "bottom"]
        );
        assert!(clipped(&block(3, 3, 13, 13), 16, 16, orig).is_empty());
        // The original reaches the bottom too: not a flag.
        assert_eq!(
            clipped(&block(4, 4, 16, 16), 16, 16, Some([2, 2, 14, 16])),
            vec!["right"]
        );
    }

    #[test]
    fn a_part_cut_at_a_tiles_edge_is_flagged() {
        // A made-up 16 by 16 frame: the original's hand reaches x 13 on
        // rows 2 to 9. A theme whose hand stops at the second tile's
        // edge (its last ink at x 7) on those rows is cut; one whose hand
        // stops straight inside the tile (x 10) is its own shape; and a
        // cut the original makes too isn't flagged.
        let orig = block(2, 2, 14, 10);
        let cut = block(2, 2, 8, 10);
        let flags = cut_flat(&cut, &orig, 16, 16, EDGE);
        assert_eq!(flags.len(), 1, "{flags:?}");
        assert!(
            flags[0].contains("right side: rows 2 to 9 stop at x 8"),
            "{}",
            flags[0]
        );
        assert!(cut_flat(&block(2, 2, 11, 10), &orig, 16, 16, EDGE).is_empty());
        assert!(cut_flat(&cut, &block(2, 2, 9, 10), 16, 16, EDGE).is_empty());
        // Three rows cut is a corner, not a part.
        let mut short = orig.clone();
        for y in 2..5 {
            for x in 8..14 {
                short[y * 16 + x] = 0;
            }
        }
        assert!(cut_flat(&short, &orig, 16, 16, EDGE).is_empty());
        // The top and bottom too: a part cut at y 8.
        let flags = cut_flat(&block(2, 2, 14, 8), &block(2, 2, 14, 13), 16, 16, EDGE);
        assert!(flags.iter().any(|f| f.contains("bottom side")), "{flags:?}");
    }

    #[test]
    fn a_box_is_covered_by_its_share_inside_the_body() {
        assert_eq!(covered([0, 0, 4, 4], Some([0, 0, 2, 4])), 0.5);
        assert_eq!(covered([0, 0, 4, 4], None), 0.0);
        assert_eq!(covered([10, 10, 2, 2], Some([0, 0, 4, 4])), 0.0);
    }

    #[test]
    fn a_figure_drawn_through_its_floor_is_flagged() {
        // A 16 by 24 figure the hero doesn't fight: no touch box, so no
        // feet row, as the rescue figure on Rivet Rodeo's top girder.
        let tol = Tolerance::default();
        let g = Geometry {
            min_size: [6, 6],
            ..Geometry::default()
        };
        let names = vec!["base".to_owned(), "wave".to_owned()];
        let figure = |y0: usize, y1: usize| {
            let mut t = vec![0u8; 16 * 24];
            for y in y0..y1 {
                for x in 4..12 {
                    t[y * 16 + x] = 2;
                }
            }
            body_of(&t, 16)
        };
        let stage = Stage {
            level: "main".into(),
            floors: vec![vec![[96, 40, 152]]],
            steps: BTreeMap::new(),
            ladders: Vec::new(),
        };
        // Placed at row 25, its feet at 49: 9 px through the floor at 40.
        // The old check held it only to the original's body, so the same
        // art passed.
        let sunk = vec![figure(0, 24), figure(0, 24)];
        assert!(slot_flags("npc", &g, &names, &sunk, &sunk, &tol).is_empty());
        let f = stand_flags("npc", &names, &sunk, &sunk, [100, 25], &stage, &tol);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].at, "base");
        assert!(f[0].what.contains("9 px below"), "{f:?}");
        // Placed at row 16 it stands on the floor: nothing to say.
        assert!(stand_flags("npc", &names, &sunk, &sunk, [100, 16], &stage, &tol).is_empty());
        // Art that stands, painted floating 3 px over it, is flagged in
        // every frame; a pixel is within the tolerance.
        let float = vec![figure(0, 21), figure(0, 21)];
        let f = stand_flags("npc", &names, &sunk, &float, [100, 16], &stage, &tol);
        assert_eq!(f.len(), 2, "{f:?}");
        assert!(f[0].what.contains("3 px above"), "{f:?}");
        let near = vec![figure(0, 23), figure(0, 23)];
        assert!(stand_flags("npc", &names, &sunk, &near, [100, 16], &stage, &tol).is_empty());
        // Away from any floor (in the air, or past the floor's ends):
        // not standing, nothing to say.
        assert!(stand_flags("npc", &names, &sunk, &float, [10, 16], &stage, &tol).is_empty());
    }
}
