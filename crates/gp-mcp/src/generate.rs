//! `assets_generate` (docs/stubs.md): make an asset for a slot from the
//! slot catalog's constraints, then place it with `assets_place`'s checks.
//!
//! A generator is anything that implements [`Generator`]: given a checked
//! [`Target`] (the slot's shape, the entity and its role, what the slot
//! holds now) it returns a [`Candidate`] of exactly that shape. The ones
//! here are deterministic and make no network calls, so the same arguments
//! give the same bytes (the shapes use only IEEE arithmetic that every
//! platform rounds alike):
//!
//! | Generator | Tiles | Palettes |
//! |---|---|---|
//! | `stub` | Each tile shows its number, as `gp assets stub` draws it | One hue per palette, by the role that draws with it |
//! | `shapes` | A shape per role at the sprite's size (a body with eyes for a player, a spiked dome for an enemy, and so on); a whole sheet is stub tiles with every sprite drawn in | None |
//! | `recolor` | The color indices 1 to 3 permuted | Every hue turned round the NES color wheel |
//!
//! An image model would be one more implementation, its candidate held to
//! the same shape and the same placement checks.

use base64::Engine as _;
use gp_assets::stub;
use gp_model::model::AssetKind;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::catalog::{self, Origin, Status};
use crate::critic;
use crate::place::{self, Content, Current, Provenance, Target};
use crate::{Config, Reply};

/// The lowest critic score `enforce` and `progressive` place.
pub const DEFAULT_MIN_SCORE: f64 = 0.7;

/// What a generator is asked for.
pub struct Request<'a> {
    /// The slot, checked: its shape, the entity, what it holds now.
    pub target: &'a Target,
    /// Varies the result; the same seed gives the same bytes.
    pub seed: u64,
    /// For `recolor`: how many steps round the color wheel (of 12).
    pub hue_shift: Option<i32>,
    /// What the person asked for. The deterministic generators only record
    /// it; an image model would read it.
    pub prompt: Option<&'a str>,
}

/// A generated asset in the slot's shape.
pub enum Candidate {
    /// Color indices 0 to 3, row by row, of the target's `image_size`.
    Pixels(Vec<u8>),
    /// Palettes of four NES color numbers: every palette of the slot, or
    /// the entity's one.
    Palettes(Vec<[u8; 4]>),
}

/// A way of making assets for slots.
pub trait Generator {
    fn name(&self) -> &'static str;
    /// True when the result is made from what the slot holds now, so it
    /// keeps that content's license (a recolor of original art is still
    /// the original's). Otherwise the result holds no one's work and is
    /// `CC0-1.0`, as stubs are.
    fn derives_from_current(&self) -> bool;
    fn generate(&self, req: &Request) -> Result<Candidate, String>;
}

/// Every generator, by name.
pub fn generators() -> Vec<Box<dyn Generator>> {
    vec![Box::new(Stub), Box::new(Shapes), Box::new(Recolor)]
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerateArgs {
    pub copy: String,
    /// A tiles or palettes slot, as in assets_place. Not needed with
    /// `progressive`.
    #[serde(default)]
    pub slot: Option<String>,
    /// Make only what this entity's sprite uses: its tiles, or its palette.
    #[serde(default)]
    pub entity: Option<String>,
    /// `stub`, `shapes` or `recolor`. Default: `shapes` for tiles, `stub`
    /// for palettes.
    #[serde(default)]
    pub generator: Option<String>,
    /// Varies the result; the same arguments give the same bytes.
    #[serde(default)]
    pub seed: u64,
    /// For `recolor`: steps round the NES color wheel, 1 to 11 (default
    /// from the seed).
    #[serde(default)]
    pub hue_shift: Option<i32>,
    /// What the person asked for, recorded in the slot's note.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Place the candidate (default true). With false, only return it.
    #[serde(default = "yes")]
    pub place: bool,
    /// Who asked for it.
    #[serde(default)]
    pub author: Option<String>,
    /// The consistency critic (docs/host.md): `off`, `advise` (default:
    /// review the candidate and return the review) or `enforce` (place
    /// only a candidate whose review passes with `min_score`).
    #[serde(default)]
    pub critic: Option<String>,
    /// For `enforce` and `progressive`: the lowest review score placed
    /// (default 0.7).
    #[serde(default)]
    pub min_score: Option<f64>,
    /// Fill every presentation slot in the catalog's order: palettes, then
    /// each tiles slot's base sheet, then its sprites by role (player,
    /// enemies, bosses, characters, pickups, projectiles, hazards). Each
    /// step tries up to `tries` seeds from `seed` and places the best
    /// reviewed. `slot` and `entity` are ignored.
    #[serde(default)]
    pub progressive: bool,
    /// With `progressive`: return the order without generating anything.
    #[serde(default)]
    pub plan_only: bool,
    /// With `progressive`: seeds to try per step (default 4, at most 16).
    #[serde(default)]
    pub tries: Option<u32>,
}

fn yes() -> bool {
    true
}

pub fn generate(config: &Config, a: GenerateArgs) -> Result<Reply, String> {
    if a.progressive {
        return crate::progressive::run(config, a);
    }
    let slot = a
        .slot
        .clone()
        .ok_or("give a slot, or progressive: true to fill them all")?;
    let t = place::target(config, &a.copy, &slot, a.entity.as_deref())?;
    let name = a.generator.clone().unwrap_or_else(|| {
        match t.kind {
            AssetKind::Tiles => "shapes",
            _ => "stub",
        }
        .to_owned()
    });
    let g = generators()
        .into_iter()
        .find(|g| g.name() == name)
        .ok_or(format!(
            "there's no generator `{name}`; there are {:?}",
            generators().iter().map(|g| g.name()).collect::<Vec<_>>()
        ))?;
    let req = Request {
        target: &t,
        seed: a.seed,
        hue_shift: a.hue_shift,
        prompt: a.prompt.as_deref(),
    };
    let candidate = g.generate(&req)?;
    let label = format!(
        "gp-mcp {} {name} seed {}{}",
        env!("CARGO_PKG_VERSION"),
        a.seed,
        a.hue_shift
            .map_or(String::new(), |h| format!(" hue_shift {h}"))
    );
    let (shown, content) = match &candidate {
        Candidate::Pixels(px) => {
            let (w, h) = t
                .image_size()
                .ok_or("a tiles candidate for a palettes slot")?;
            let png = gp_assets::chr::write_png(w, h, px);
            (
                json!({
                    "image_size": [w, h],
                    "png_base64": base64::engine::general_purpose::STANDARD.encode(&png),
                }),
                Content::Pixels(px.clone()),
            )
        }
        Candidate::Palettes(p) => (
            json!({ "palettes": p }),
            Content::Palettes(p.iter().map(|c| c.to_vec()).collect()),
        ),
    };
    let mode = a.critic.as_deref().unwrap_or("advise");
    if !matches!(mode, "off" | "advise" | "enforce") {
        return Err(format!("critic `{mode}` isn't off, advise or enforce"));
    }
    let min_score = a.min_score.unwrap_or(DEFAULT_MIN_SCORE);
    let review = if mode == "off" {
        None
    } else {
        Some(critic::review(
            config,
            &t,
            &candidate,
            a.prompt.as_deref(),
            critic::DEFAULT,
        )?)
    };
    let mut out = json!({
        "generator": { "name": name, "seed": a.seed, "hue_shift": a.hue_shift, "label": label },
        "candidate": shown,
        "critic": review.as_ref().map(critic::Review::json),
    });
    if mode == "off" {
        out["critic_note"] = json!(
            "the critic was off: the candidate is held to the slot's shape and the placement checks only"
        );
    }
    if let Some(r) = &review
        && mode == "enforce"
        && (!r.pass || r.score < min_score)
    {
        out["ok"] = json!(false);
        out["written"] = json!(false);
        out["stage"] = json!("critic");
        out["min_score"] = json!(min_score);
        out["problems"] = json!(
            r.problems()
                .iter()
                .map(|c| format!("{}: {}", c.name, c.message))
                .collect::<Vec<_>>()
        );
        return Ok(Reply::ok(out));
    }
    if !a.place {
        out["ok"] = json!(true);
        out["written"] = json!(false);
        return Ok(Reply::ok(out));
    }
    let whole_stub = name == "stub" && t.entity.is_none();
    let p = Provenance {
        status: if whole_stub {
            Status::Stub
        } else {
            Status::Replaced
        },
        origin: if whole_stub {
            Origin::Stub
        } else {
            Origin::Generated
        },
        license: if g.derives_from_current() {
            t.state.license.clone()
        } else {
            catalog::STUB_LICENSE.to_owned()
        },
        generator: Some(label),
        author: a.author.clone(),
        note: a.prompt.clone(),
    };
    let placed = place::place_content(config, &t, &p, content)?;
    if let (Some(o), Some(r)) = (out.as_object_mut(), placed.json.as_object()) {
        o.extend(r.clone());
    }
    Ok(Reply {
        json: out,
        is_error: placed.is_error,
    })
}

/// A small, fixed mixer, so a seed picks variations the same way on every
/// machine.
fn mix(seed: u64, salt: u64) -> u64 {
    let mut z = seed
        .wrapping_add(salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Numbered tiles and role colors, as `gp assets stub` makes them.
pub struct Stub;

/// Stub tiles laid out as a sheet `per_row` wide, for the tile numbers
/// given.
fn stub_tiles(numbers: &[u32], per_row: u32, size: (u32, u32)) -> Vec<u8> {
    let mut px = vec![0u8; (size.0 * size.1) as usize];
    for (k, &t) in numbers.iter().enumerate() {
        let (tx, ty) = ((k as u32 % per_row) * 8, (k as u32 / per_row) * 8);
        for (i, p) in stub::tile_pixels(t).iter().enumerate() {
            let (x, y) = (i as u32 % 8, i as u32 / 8);
            px[((ty + y) * size.0 + tx + x) as usize] = *p;
        }
    }
    px
}

impl Generator for Stub {
    fn name(&self) -> &'static str {
        "stub"
    }
    fn derives_from_current(&self) -> bool {
        false
    }
    fn generate(&self, req: &Request) -> Result<Candidate, String> {
        let t = req.target;
        match (&t.current, &t.sprite) {
            (Current::Tiles { tiles, per_row, .. }, None) => {
                let all: Vec<u32> = (0..*tiles).collect();
                let size = t.image_size().expect("tiles");
                Ok(Candidate::Pixels(stub_tiles(&all, *per_row, size)))
            }
            (Current::Tiles { .. }, Some(sp)) => {
                let size = t.image_size().expect("tiles");
                Ok(Candidate::Pixels(stub_tiles(
                    &sp.tiles,
                    sp.width.max(1),
                    size,
                )))
            }
            (Current::Palettes(_), Some(_)) => {
                Ok(Candidate::Palettes(vec![stub::palette(t.role())]))
            }
            (Current::Palettes(_), None) => Ok(Candidate::Palettes(
                t.palette_roles()
                    .iter()
                    .map(|r| stub::palette(r.as_deref()))
                    .collect(),
            )),
        }
    }
}

/// A shape per role, drawn at a sprite's size.
pub struct Shapes;

/// Draws a role's shape into a `w` by `h` image of color indices: 1 the
/// outline, 2 the body, 3 the eyes and highlights, 0 transparent. Facing
/// right, since sprites are mirrored when they face left.
pub fn shape(role: &str, w: u32, h: u32, seed: u64) -> Vec<u8> {
    let (wf, hf) = (w as f32, h as f32);
    let (cx, cy) = (wf / 2.0, hf / 2.0);
    let jitter = (mix(seed, 1) % 3) as f32 - 1.0;
    let mut px = vec![0u8; (w * h) as usize];
    let inside = |x: f32, y: f32| -> bool {
        let (dx, dy) = ((x - cx) / (wf / 2.0 - 0.5), (y - cy) / (hf / 2.0 - 0.5));
        match role {
            "player" | "npc" => dx * dx + dy * dy <= 1.0,
            "enemy" | "boss" => {
                let top = (dy + 0.4).min(0.0);
                dy >= -1.0 && dx * dx + top * top * 2.5 <= 1.0
            }
            "collectible" | "power-up" | "goal" => dx.abs() + dy.abs() <= 1.0,
            "projectile" => dx * dx + dy * dy <= 0.3,
            "hazard" => {
                // A row of spikes, each a quarter of the width.
                let k = ((x / wf) * 4.0).fract();
                (y / hf) >= 1.0 - 2.0 * (0.5 - (k - 0.5).abs()) * 0.9
            }
            _ => dx.abs() <= 0.9 && dy.abs() <= 0.9,
        }
    };
    for y in 0..h {
        for x in 0..w {
            let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
            if !inside(xf, yf) {
                continue;
            }
            let edge = [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)]
                .iter()
                .any(|(ox, oy)| {
                    let (nx, ny) = (xf + ox, yf + oy);
                    nx < 0.0 || ny < 0.0 || nx > wf || ny > hf || !inside(nx, ny)
                });
            px[(y * w + x) as usize] = if edge { 1 } else { 2 };
        }
    }
    let mut dot = |x: f32, y: f32| {
        let (x, y) = (x.round() as i64, y.round() as i64);
        for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (px_, py) = (x + ox, y + oy);
            if px_ >= 0 && py >= 0 && (px_ as u32) < w && (py as u32) < h {
                let i = (py as u32 * w + px_ as u32) as usize;
                if px[i] == 2 {
                    px[i] = 3;
                }
            }
        }
    };
    match role {
        "player" | "npc" => {
            dot(wf * 0.55 + jitter, hf * 0.3);
            dot(wf * 0.75 + jitter, hf * 0.3);
        }
        "enemy" | "boss" => {
            dot(wf * 0.65 + jitter, hf * 0.5);
            if role == "boss" {
                // A crown: highlights along the top.
                for i in 0..4 {
                    dot(wf * (0.25 + 0.15 * i as f32), hf * 0.32);
                }
            }
        }
        "hazard" | "projectile" => {}
        _ => dot(cx - 1.0 + jitter, cy - 1.0),
    }
    px
}

/// Draws a sprite's shape into a sheet, tile by tile as the sprite lists
/// its tiles.
fn draw_sprite(
    sheet: &mut [u8],
    sheet_w: u32,
    per_row: u32,
    sp: &gp_model::model::Sprite,
    image: &[u8],
) {
    let sw = sp.width.max(1);
    let iw = sw * 8;
    for (k, &t) in sp.tiles.iter().enumerate() {
        let (sx, sy) = ((k as u32 % sw) * 8, (k as u32 / sw) * 8);
        let (tx, ty) = ((t % per_row) * 8, (t / per_row) * 8);
        for y in 0..8 {
            for x in 0..8 {
                sheet[((ty + y) * sheet_w + tx + x) as usize] =
                    image[((sy + y) * iw + sx + x) as usize];
            }
        }
    }
}

impl Generator for Shapes {
    fn name(&self) -> &'static str {
        "shapes"
    }
    fn derives_from_current(&self) -> bool {
        false
    }
    fn generate(&self, req: &Request) -> Result<Candidate, String> {
        let t = req.target;
        let Current::Tiles {
            tiles,
            per_row,
            size,
            ..
        } = &t.current
        else {
            return Err("shapes draws tiles; for a palettes slot use stub or recolor".into());
        };
        if t.sprite.is_some() {
            let (w, h) = t.image_size().expect("tiles");
            return Ok(Candidate::Pixels(shape(
                t.role().unwrap_or(""),
                w,
                h,
                req.seed,
            )));
        }
        // The whole sheet: stub tiles, with every sprite in this slot drawn
        // in, so nothing of the original is left in it.
        let all: Vec<u32> = (0..*tiles).collect();
        let mut sheet = stub_tiles(&all, *per_row, *size);
        for e in t.model.entities.values() {
            let Some(sp) = e.core.sprite.as_ref().filter(|s| s.slot == t.slot) else {
                continue;
            };
            if sp.tiles.iter().any(|&n| n >= *tiles) {
                continue;
            }
            let sw = sp.width.max(1);
            let (w, h) = (sw * 8, (sp.tiles.len() as u32).div_ceil(sw) * 8);
            let image = shape(&e.core.role, w, h, req.seed);
            draw_sprite(&mut sheet, size.0, *per_row, sp, &image);
        }
        Ok(Candidate::Pixels(sheet))
    }
}

/// The colors of what's there now, turned.
pub struct Recolor;

/// An NES color number with its hue (the low nibble, 1 to 12 for the
/// colored ones) turned `steps` round the wheel. Grays and blacks stay.
pub fn turn_hue(c: u8, steps: i32) -> u8 {
    let hue = (c & 0x0F) as i32;
    if !(1..=12).contains(&hue) {
        return c;
    }
    let turned = (hue - 1 + steps).rem_euclid(12) + 1;
    (c & 0x30) | turned as u8
}

impl Generator for Recolor {
    fn name(&self) -> &'static str {
        "recolor"
    }
    fn derives_from_current(&self) -> bool {
        true
    }
    fn generate(&self, req: &Request) -> Result<Candidate, String> {
        let t = req.target;
        match &t.current {
            Current::Tiles { .. } => {
                // One of the five orders of 1, 2 and 3 other than their own.
                const ORDERS: [[u8; 3]; 5] =
                    [[1, 3, 2], [2, 1, 3], [2, 3, 1], [3, 1, 2], [3, 2, 1]];
                let order = ORDERS[(mix(req.seed, 2) % 5) as usize];
                let px = t
                    .current_pixels()
                    .expect("tiles")
                    .iter()
                    .map(|&p| if p == 0 { 0 } else { order[(p - 1) as usize] })
                    .collect();
                Ok(Candidate::Pixels(px))
            }
            Current::Palettes(p) => {
                let steps = req.hue_shift.unwrap_or(1 + (mix(req.seed, 3) % 11) as i32);
                if steps.rem_euclid(12) == 0 {
                    return Err(format!(
                        "a hue_shift of {steps} turns every color back to itself"
                    ));
                }
                let turn = |pal: &[u8; 4]| pal.map(|c| turn_hue(c, steps));
                Ok(Candidate::Palettes(match &t.sprite {
                    Some(sp) => vec![turn(&p[sp.palette as usize])],
                    None => p.iter().map(turn).collect(),
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_are_the_same_every_run_and_vary_by_seed() {
        for role in [
            "player",
            "enemy",
            "boss",
            "collectible",
            "projectile",
            "hazard",
            "other",
        ] {
            let a = shape(role, 16, 16, 7);
            assert_eq!(a, shape(role, 16, 16, 7), "{role}");
            assert!(a.iter().all(|&p| p <= 3), "{role}");
            assert!(a.contains(&1), "{role} has an outline");
        }
        let seeds: std::collections::BTreeSet<Vec<u8>> =
            (0..8).map(|s| shape("player", 16, 16, s)).collect();
        assert!(seeds.len() > 1, "the seed moves the eyes");
        // Any size a sprite can have.
        assert_eq!(shape("enemy", 8, 24, 0).len(), 8 * 24);
    }

    #[test]
    fn a_hue_turns_round_the_wheel() {
        assert_eq!(turn_hue(0x21, 1), 0x22);
        assert_eq!(turn_hue(0x2C, 1), 0x21);
        assert_eq!(turn_hue(0x01, -1), 0x0C);
        // Grays and blacks stay.
        for c in [0x00, 0x0D, 0x0F, 0x10, 0x20, 0x30, 0x3E] {
            assert_eq!(turn_hue(c, 5), c);
        }
    }

    #[test]
    fn every_generator_has_its_own_name() {
        let names: Vec<_> = generators().iter().map(|g| g.name()).collect();
        assert_eq!(names, ["stub", "shapes", "recolor"]);
    }
}
