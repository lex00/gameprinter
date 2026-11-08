//! A scene drawn into pixels on the CPU, for the screen measure
//! (docs/interpreter.md, "The screen measure"). It draws what the
//! interpreter preview draws (`scripts/web/page/preview.js`, `draw`): the
//! clear color, the level picture under the camera and its cycling colors,
//! the HUD's background part (its layer, its cycling colors and the tiles
//! of its fields in background palettes), each body's picture, then the
//! HUD's sprites. Themes' particles and overlay aren't drawn: the measure compares
//! the model's own pictures.
//!
//! The picture is the screen's whole frame (`Screen::size`), with only the
//! visible part (`Screen::visible`) drawn, where the preview's canvas sits
//! over the original's frame in the game view. A pixel counts as drawn
//! when its picture's alpha is at least half. Whole numbers only.

use crate::{Present, Scene};

/// A decoded picture, RGBA, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<[u8; 4]>,
}

impl Image {
    fn at(&self, x: i64, y: i64, flip_x: bool, flip_y: bool) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x >= i64::from(self.w) || y >= i64::from(self.h) {
            return None;
        }
        let sx = if flip_x { i64::from(self.w) - 1 - x } else { x };
        let sy = if flip_y { i64::from(self.h) - 1 - y } else { y };
        let c = self.px[(sy as u32 * self.w + sx as u32) as usize];
        (c[3] >= 128).then_some(c)
    }
}

/// Something drawn, by name, and its box on the screen: a body's picture
/// or a HUD field's text, for naming the places two pictures differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thing {
    pub name: String,
    /// Left, top, width, height in screen pixels.
    pub rect: [i64; 4],
    pub sprite: bool,
}

/// One frame drawn: every pixel's color and whether a sprite (a body's
/// picture) shows there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<[u8; 3]>,
    pub sprite: Vec<bool>,
    /// The part drawn: left, top, width, height.
    pub visible: [u32; 4],
    pub things: Vec<Thing>,
}

/// Draws `scene`, the scene of `world`, with `pics` (by the presentation's picture numbers; a
/// missing one draws nothing). `names` names each body's kind for
/// [`Frame::things`], by body; `fields` names each HUD field.
pub fn draw(
    p: &Present,
    world: &gp_sim::World,
    pics: &[Option<Image>],
    s: &Scene,
    names: &[String],
    fields: &[String],
) -> Frame {
    let [w, h] = p.screen.size;
    let visible = p.screen.visible();
    let [vl, vt, vw, vh] = visible.map(i64::from);
    let mut f = Frame {
        w,
        h,
        rgb: vec![[0; 3]; (w * h) as usize],
        sprite: vec![false; (w * h) as usize],
        visible,
        things: Vec::new(),
    };
    let pic = |n: u32| pics.get(n as usize).and_then(Option::as_ref);
    let inside = |x: i64, y: i64| x >= vl && y >= vt && x < vl + vw && y < vt + vh;
    let idx = |x: i64, y: i64| (y as u32 * w + x as u32) as usize;
    let [cx, cy] = s.camera;
    for y in vt..vt + vh {
        for x in vl..vl + vw {
            let i = idx(x, y);
            let mut c = s.clear;
            if let Some(l) = s.level.and_then(pic)
                && let Some(px) = l.at(x + cx, y + cy, false, false)
            {
                c = [px[0], px[1], px[2]];
            }
            for (m, color) in &s.tints {
                if pic(*m)
                    .and_then(|m| m.at(x + cx, y + cy, false, false))
                    .is_some()
                {
                    c = *color;
                }
            }
            f.rgb[i] = c;
        }
    }
    // The cells the game changed, over the level and its tints.
    for d in &s.cells {
        let Some(img) = pic(d.pic) else { continue };
        for y in 0..i64::from(img.h) {
            for x in 0..i64::from(img.w) {
                let (sx, sy) = (d.x - cx + x, d.y - cy + y);
                if inside(sx, sy)
                    && let Some(c) = img.at(x, y, d.flip_x, d.flip_y)
                {
                    f.rgb[idx(sx, sy)] = [c[0], c[1], c[2]];
                }
            }
        }
    }
    // The HUD's background part: its layer and the fields in background
    // palettes, all but its last `hud_sprites` draws. It is the
    // background's, as the console draws it, so every body that isn't
    // behind it goes over it: the hero over the status row.
    let sprites_from = s.hud.len().saturating_sub(s.hud_sprites);
    let hud_bg = &s.hud[..sprites_from];
    for (k, d) in hud_bg.iter().enumerate() {
        let Some(img) = pic(d.pic) else { continue };
        for y in 0..i64::from(img.h) {
            for x in 0..i64::from(img.w) {
                let (sx, sy) = (d.x + x, d.y + y);
                if inside(sx, sy)
                    && let Some(c) = img.at(x, y, d.flip_x, d.flip_y)
                {
                    f.rgb[idx(sx, sy)] = [c[0], c[1], c[2]];
                }
            }
        }
        if k == 0 {
            for (m, color) in &s.hud_tints {
                let Some(m) = pic(*m) else { continue };
                for y in vt..vt + vh {
                    for x in vl..vl + vw {
                        if m.at(x, y, false, false).is_some() {
                            f.rgb[idx(x, y)] = *color;
                        }
                    }
                }
            }
        }
    }
    // Whether the background draws a pixel of its own at a screen pixel
    // (the level, a changed cell or the HUD's background part): a sprite
    // behind it shows only where it doesn't. A changed cell's picture
    // counts as the level's.
    let level_at = |x: i64, y: i64| {
        s.level
            .and_then(pic)
            .is_some_and(|l| l.at(x + cx, y + cy, false, false).is_some())
            || s.cells.iter().any(|d| {
                let (wx, wy) = (x + cx - d.x, y + cy - d.y);
                pic(d.pic).is_some_and(|i| i.at(wx, wy, d.flip_x, d.flip_y).is_some())
            })
            || hud_bg.iter().any(|d| {
                pic(d.pic).is_some_and(|i| i.at(x - d.x, y - d.y, d.flip_x, d.flip_y).is_some())
            })
    };
    let put = |f: &mut Frame,
               img: &Image,
               x0: i64,
               y0: i64,
               fx: bool,
               fy: bool,
               sprite: bool,
               behind: bool,
               cut: &[[i64; 4]],
               part: Option<([i64; 4], bool)>| {
        for y in 0..i64::from(img.h) {
            for x in 0..i64::from(img.w) {
                let (mut sx, sy) = (x0 + x, y0 + y);
                // Core `present.sprite_x_wraps`: a hardware sprite whose
                // left column is left of the screen shows 256 pixels on.
                if sprite && p.screen.x_wraps && x0 + x.div_euclid(8) * 8 < 0 {
                    sx += 256;
                }
                // A part the console's sprite limit drops (`Scene::cut`).
                let dropped = cut
                    .iter()
                    .any(|c| x >= c[0] && x < c[0] + c[2] && y >= c[1] && y < c[1] + c[3]);
                // Core `sprite.split`: only the kept part, or only the rest.
                let away = part.is_some_and(|(k, keep)| {
                    let within = x >= k[0] && x < k[0] + k[2] && y >= k[1] && y < k[1] + k[3];
                    within != keep
                });
                if !inside(sx, sy) || dropped || away {
                    continue;
                }
                if let Some(c) = img.at(x, y, fx, fy) {
                    let i = idx(sx, sy);
                    // A sprite behind the level is still a sprite's pixel,
                    // as the NES's sprite layer counts it.
                    if !(behind && level_at(sx, sy)) {
                        f.rgb[i] = [c[0], c[1], c[2]];
                    }
                    f.sprite[i] = sprite;
                }
            }
        }
    };
    // The bodies behind the level first, then the name table's, then the
    // rest by the sprite table, then those in front of every body
    // (`Scene::paint_order`).
    let behind = |b: usize| s.behind.get(b).copied().unwrap_or(false);
    for b in s.paint_order() {
        let Some(d) = &s.bodies[b] else { continue };
        let Some(img) = pic(d.pic) else { continue };
        let (x, y) = (d.x - cx, d.y - cy);
        let sprite = !s.name_table.get(b).copied().unwrap_or(false);
        let cut = s.cut.get(b).map_or(&[][..], Vec::as_slice);
        // Core `sprite.split`: the rest moved, then the kept part over it
        // where the picture is.
        match s.split.get(b).copied().flatten() {
            Some((keep, dx, dy)) => {
                let rest = Some((keep, false));
                put(
                    &mut f,
                    img,
                    x + dx,
                    y + dy,
                    d.flip_x,
                    d.flip_y,
                    sprite,
                    behind(b),
                    cut,
                    rest,
                );
                let kept = Some((keep, true));
                put(
                    &mut f,
                    img,
                    x,
                    y,
                    d.flip_x,
                    d.flip_y,
                    sprite,
                    behind(b),
                    cut,
                    kept,
                );
            }
            None => put(
                &mut f,
                img,
                x,
                y,
                d.flip_x,
                d.flip_y,
                sprite,
                behind(b),
                cut,
                None,
            ),
        }
        f.things.push(Thing {
            name: names.get(b).cloned().unwrap_or_default(),
            rect: [x, y, i64::from(img.w), i64::from(img.h)],
            sprite,
        });
    }
    // The HUD's sprites (fields in sprite palettes and its own sprites),
    // over the bodies.
    for d in &s.hud[sprites_from..] {
        let Some(img) = pic(d.pic) else { continue };
        put(
            &mut f,
            img,
            d.x,
            d.y,
            d.flip_x,
            d.flip_y,
            true,
            false,
            &[],
            None,
        );
    }
    // Each HUD field's text as one box, by the tiles it was drawn in.
    // A HUD hidden (core `present.hidden`'s `hud`) draws no field.
    if let Some(hud) = p.hud.as_ref().filter(|_| !s.hud.is_empty()) {
        for (i, fl) in hud.fields.iter().enumerate() {
            let Some(name) = fields.get(i) else { continue };
            let len = p
                .levels
                .get(world.level)
                .map_or(0, |l| fl.text(world, l).chars().count());
            if len == 0 {
                continue;
            }
            f.things.push(Thing {
                name: name.clone(),
                rect: [
                    i64::from(fl.at[0]) * 8,
                    i64::from(fl.at[1]) * 8,
                    len as i64 * 8,
                    8,
                ],
                sprite: false,
            });
        }
    }
    // Core `present.screens_from_line`: the picture turned off from a
    // line down shows the backdrop alone there, no sprite.
    if let Some((line, color)) = s.off_from {
        for y in line.max(vt)..vt + vh {
            for x in vl..vl + vw {
                let i = idx(x, y);
                f.rgb[i] = color;
                f.sprite[i] = false;
            }
        }
    }
    f
}
