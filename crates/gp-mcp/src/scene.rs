//! An edited sprite seen where the recordings show it (docs/sprite-editing.md):
//! the source's baselines replayed in the interpreter, drawn on the CPU
//! from the presentation the generator compiles (`gp_look::present`,
//! the pictures `gp-present` names), once with the copy's assets as they
//! are and once with the edit. Presentation never moves the simulation,
//! so one replay serves both: the world is the same frame for frame and
//! only the pictures differ.
//!
//! Two things come out:
//!
//! - Shots: for each pose the contexts list (`crate::contexts`), a crop
//!   around the entity at its first frame in that pose and at its first
//!   moment, before beside after, as one PNG.
//! - Readability against the backgrounds the sprite was drawn over on
//!   every frame it was seen: the level picture's colors under its drawn
//!   pixels, weighted by how often, against the sprite's own colors before
//!   and after, by WCAG contrast (`gp_theme::color`).
//!
//! Level color cycles (tints) and the HUD aren't drawn. Floats only in the
//! readability numbers, which are presentation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use gp_interp::{Interp, Lifted};
use gp_present::{Animator, Present};
use gp_theme::color::{Rgb, contrast, delta_e};
use serde_json::{Value, json};

use crate::place::Target;

/// The crop around the entity, in world pixels, before scaling.
pub const CROP: u32 = 64;
/// How many times bigger a shot draws each pixel.
pub const SCALE: u32 = 2;
/// The most shots one preview holds.
pub const MAX_SHOTS: usize = 12;
/// A color the sprite draws with takes part in readability when it's at
/// least this share of its drawn pixels.
pub const MIN_INK_SHARE: f64 = 0.1;
/// The WCAG contrast at which a sprite color reads against a background
/// color (the ratio WCAG 2 asks of large text).
pub const READABLE: f64 = 3.0;

/// RGBA pictures by their number in the presentation.
struct Art {
    present: Present,
    pics: Vec<Option<Rgba>>,
}

#[derive(Clone)]
struct Rgba {
    w: u32,
    h: u32,
    px: Vec<[u8; 4]>,
}

fn decode(png: &[u8]) -> Result<Rgba, String> {
    let mut d = png::Decoder::new(std::io::Cursor::new(png));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![
        0;
        r.output_buffer_size()
            .ok_or("a picture too big to decode")?
    ];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let n = info.color_type.samples();
    let px = buf[..info.buffer_size()]
        .chunks(n)
        .map(|c| match n {
            1 => [c[0], c[0], c[0], 255],
            2 => [c[0], c[0], c[0], c[1]],
            3 => [c[0], c[1], c[2], 255],
            _ => [c[0], c[1], c[2], c[3]],
        })
        .collect();
    Ok(Rgba {
        w: info.width,
        h: info.height,
        px,
    })
}

fn load_art(t: &Target, assets: &Path, spec: &gp_sim::Spec) -> Result<Art, String> {
    let art = gp_look::present::art(&t.model, spec, assets)?;
    let by_file: BTreeMap<&str, &[u8]> = art
        .pictures
        .iter()
        .map(|p| (p.file.as_str(), p.png.as_slice()))
        .collect();
    let pics = art
        .present
        .pictures
        .iter()
        .map(|p| by_file.get(p.file.as_str()).map(|b| decode(b)).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Art {
        present: art.present,
        pics,
    })
}

/// A picture's pixel at (x, y) of where it's drawn, flips applied.
fn pic_at(p: &Rgba, x: i64, y: i64, flip_x: bool, flip_y: bool) -> Option<[u8; 4]> {
    if x < 0 || y < 0 || x >= i64::from(p.w) || y >= i64::from(p.h) {
        return None;
    }
    let sx = if flip_x { i64::from(p.w) - 1 - x } else { x };
    let sy = if flip_y { i64::from(p.h) - 1 - y } else { y };
    Some(p.px[(sy as u32 * p.w + sx as u32) as usize])
}

/// The background at a world pixel: the level picture, or the clear color.
fn background(art: &Art, s: &gp_present::Scene, x: i64, y: i64) -> [u8; 3] {
    if let Some(l) = s.level.and_then(|l| art.pics.get(l as usize)?.as_ref())
        && let Some(c) = pic_at(l, x, y, false, false)
        && c[3] >= 128
    {
        return [c[0], c[1], c[2]];
    }
    s.clear
}

/// Draws a crop of the world with its top left at (ox, oy).
fn crop(art: &Art, s: &gp_present::Scene, ox: i64, oy: i64) -> Vec<[u8; 3]> {
    let n = CROP as i64;
    let mut out = vec![[0u8; 3]; (CROP * CROP) as usize];
    for y in 0..n {
        for x in 0..n {
            out[(y * n + x) as usize] = background(art, s, ox + x, oy + y);
        }
    }
    for d in s.bodies.iter().flatten() {
        let Some(p) = art.pics.get(d.pic as usize).and_then(Option::as_ref) else {
            continue;
        };
        for y in 0..i64::from(p.h) {
            for x in 0..i64::from(p.w) {
                let (wx, wy) = (d.x + x - ox, d.y + y - oy);
                if wx < 0 || wy < 0 || wx >= n || wy >= n {
                    continue;
                }
                if let Some(c) = pic_at(p, x, y, d.flip_x, d.flip_y)
                    && c[3] >= 128
                {
                    out[(wy * n + wx) as usize] = [c[0], c[1], c[2]];
                }
            }
        }
    }
    out
}

/// Colors counted: the background under the sprite's drawn pixels, and
/// the sprite's own colors.
#[derive(Default)]
struct Seen {
    behind: BTreeMap<[u8; 3], u64>,
    ink: BTreeMap<[u8; 3], u64>,
}

fn count(art: &Art, s: &gp_present::Scene, body: usize, seen: &mut Seen) {
    let Some(Some(d)) = s.bodies.get(body) else {
        return;
    };
    let Some(p) = art.pics.get(d.pic as usize).and_then(Option::as_ref) else {
        return;
    };
    for y in 0..i64::from(p.h) {
        for x in 0..i64::from(p.w) {
            if let Some(c) = pic_at(p, x, y, d.flip_x, d.flip_y)
                && c[3] >= 128
            {
                *seen.ink.entry([c[0], c[1], c[2]]).or_default() += 1;
                *seen
                    .behind
                    .entry(background(art, s, d.x + x, d.y + y))
                    .or_default() += 1;
            }
        }
    }
}

/// How well a sprite's colors read against the backgrounds behind it.
fn readability(seen: &Seen) -> Value {
    let ink_total: u64 = seen.ink.values().sum();
    let behind_total: u64 = seen.behind.values().sum();
    if ink_total == 0 || behind_total == 0 {
        return json!({ "frames": 0 });
    }
    let inks: Vec<Rgb> = seen
        .ink
        .iter()
        .filter(|&(_, &n)| n as f64 / ink_total as f64 >= MIN_INK_SHARE)
        .map(|(c, _)| Rgb(*c))
        .collect();
    let (mut readable, mut sum_contrast, mut min_de) = (0u64, 0.0, f64::MAX);
    let mut backgrounds = Vec::new();
    for (c, &n) in &seen.behind {
        let bg = Rgb(*c);
        let best = inks.iter().map(|&i| contrast(i, bg)).fold(1.0, f64::max);
        let de = inks
            .iter()
            .map(|&i| delta_e(i, bg))
            .fold(f64::MAX, f64::min);
        if best >= READABLE {
            readable += n;
        }
        sum_contrast += best * n as f64;
        min_de = min_de.min(de);
        backgrounds.push((n, *c, best));
    }
    backgrounds.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
    json!({
        "share_readable": r3(readable as f64 / behind_total as f64),
        "mean_contrast": r3(sum_contrast / behind_total as f64),
        "min_delta_e": r3(min_de),
        "inks": inks.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "backgrounds": backgrounds.iter().take(4).map(|(n, c, best)| json!({
            "color": Rgb(*c).to_string(),
            "share": r3(*n as f64 / behind_total as f64),
            "best_contrast": r3(*best),
        })).collect::<Vec<_>>(),
    })
}

/// The readability check: a warning when the edit makes the sprite read
/// worse against what it was drawn over (5 points less of the background
/// readable), or when under half of it is readable after.
pub fn readability_check(before: &Value, after: &Value) -> Value {
    let (b, a) = (
        before["share_readable"].as_f64(),
        after["share_readable"].as_f64(),
    );
    let (verdict, message) = match (b, a) {
        (Some(b), Some(a)) if a + 0.05 < b || a < 0.5 => (
            "warn",
            format!(
                "against the backgrounds it was seen on, {:.0}% reads at contrast {READABLE} or more after the edit ({:.0}% before)",
                a * 100.0,
                b * 100.0
            ),
        ),
        (Some(b), Some(a)) => (
            "pass",
            format!(
                "{:.0}% of the backgrounds it was seen on read at contrast {READABLE} or more ({:.0}% before)",
                a * 100.0,
                b * 100.0
            ),
        ),
        _ => (
            "pass",
            "the sprite wasn't drawn on any recorded frame, so readability wasn't measured"
                .to_owned(),
        ),
    };
    json!({ "name": "context.readability", "verdict": verdict, "message": message, "before": before, "after": after })
}

/// A shot: its label, baseline and frame, and the crop before and after.
type Shot = (String, String, u64, [Vec<[u8; 3]>; 2]);

/// Where a shot is taken: a baseline and the frame after which it shows.
fn jump_points(contexts: &Value) -> Vec<(String, String, u64)> {
    let mut out: Vec<(String, String, u64)> = Vec::new();
    for p in contexts["poses"].as_array().into_iter().flatten() {
        let pose = p["pose"].as_str().unwrap_or("").to_owned();
        let mut add = |label: String, b: &Value, f: &Value| {
            if let (Some(b), Some(f)) = (b.as_str(), f.as_u64())
                && !out.iter().any(|(_, ob, of)| ob == b && *of == f)
            {
                out.push((label, b.to_owned(), f));
            }
        };
        add(pose.clone(), &p["first"]["baseline"], &p["first"]["frame"]);
        if let Some(m) = p["moments"].as_array().and_then(|m| m.first()) {
            let ev = m["events"]
                .as_array()
                .and_then(|e| e.first())
                .and_then(Value::as_str)
                .unwrap_or("");
            add(format!("{pose} at {ev}"), &m["baseline"], &m["frame"]);
        }
    }
    out.truncate(MAX_SHOTS);
    out
}

/// The shots and readability of an edit, from the copy's assets as they
/// are (`before`) and with the edit (`after`). Returns the PNG (or None
/// when the entity was never seen) and the JSON.
pub fn in_context(
    t: &Target,
    before: &Path,
    after: &Path,
    entity: &str,
    contexts: &Value,
) -> Result<(Option<Vec<u8>>, Value), String> {
    let lifted = Lifted::read_dir(before, &t.model)?;
    let probe = Interp::load(&t.model, &lifted)?;
    let kind = probe
        .spec()
        .kinds
        .iter()
        .position(|k| k.name == entity)
        .ok_or(format!("the simulation has no kind `{entity}`"))?;
    let arts = [
        load_art(t, before, probe.spec())?,
        load_art(t, after, probe.spec())?,
    ];
    let points = jump_points(contexts);
    let mut shots: Vec<Shot> = Vec::new();
    let mut seen = [Seen::default(), Seen::default()];
    let mut frames_seen = 0u64;
    for (name, input) in &t.runs {
        let mut rt = Interp::load(&t.model, &lifted)?;
        let mut anims = [Animator::new(), Animator::new()];
        for f in input {
            rt.advance(f)?;
            let (spec, w) = (rt.spec(), rt.world());
            for (a, art) in anims.iter_mut().zip(&arts) {
                a.step(&art.present, spec, w);
            }
            let Some((body, b)) = w.of_kind(kind).find(|(_, b)| b.active()) else {
                continue;
            };
            frames_seen += 1;
            let scenes = [
                anims[0].scene(&arts[0].present, spec, w),
                anims[1].scene(&arts[1].present, spec, w),
            ];
            for k in 0..2 {
                count(&arts[k], &scenes[k], body, &mut seen[k]);
            }
            for (label, base, frame) in &points {
                if base == name && *frame == w.frame {
                    let size = spec.kinds[kind].size;
                    let (cx, cy) = (
                        b.x.floor() + size[0].floor() / 2,
                        b.y.floor() + size[1].floor() / 2,
                    );
                    let (ox, oy) = (cx - i64::from(CROP / 2), cy - i64::from(CROP / 2));
                    shots.push((
                        label.clone(),
                        base.clone(),
                        *frame,
                        [
                            crop(&arts[0], &scenes[0], ox, oy),
                            crop(&arts[1], &scenes[1], ox, oy),
                        ],
                    ));
                }
            }
        }
    }
    let rb = readability(&seen[0]);
    let ra = readability(&seen[1]);
    let mut check = readability_check(&rb, &ra);
    check["frames"] = json!(frames_seen);
    let list: Vec<Value> = shots
        .iter()
        .enumerate()
        .map(|(row, (label, b, f, _))| json!({ "row": row, "pose": label, "baseline": b, "frame": f }))
        .collect();
    let png = (!shots.is_empty()).then(|| strip(&shots.iter().map(|s| &s.3).collect::<Vec<_>>()));
    Ok((
        png,
        json!({
            "shots": list,
            "layout": "one row a shot: before, then after",
            "crop": CROP,
            "scale": SCALE,
            "readability": check,
        }),
    ))
}

/// Where the cells of one level tile kind are, by level name: each cell's
/// top left in world pixels (the level picture is drawn at the world's
/// origin, `gp_present::Scene`).
pub type CellsAt = BTreeMap<String, Vec<[i64; 2]>>;

/// The most shots of a tile kind one baseline gives.
pub const TILE_SHOTS_PER_BASELINE: usize = 3;

/// Counts the sprites' ink drawn over a cell of the kind, and the level
/// picture under it, into `seen`.
fn count_over(art: &Art, s: &gp_present::Scene, cells: &BTreeSet<[i64; 2]>, seen: &mut Seen) {
    for d in s.bodies.iter().flatten() {
        let Some(p) = art.pics.get(d.pic as usize).and_then(Option::as_ref) else {
            continue;
        };
        for y in 0..i64::from(p.h) {
            for x in 0..i64::from(p.w) {
                let (wx, wy) = (d.x + x, d.y + y);
                if !cells.contains(&[wx.div_euclid(16) * 16, wy.div_euclid(16) * 16]) {
                    continue;
                }
                if let Some(c) = pic_at(p, x, y, d.flip_x, d.flip_y)
                    && c[3] >= 128
                {
                    *seen.ink.entry([c[0], c[1], c[2]]).or_default() += 1;
                    *seen.behind.entry(background(art, s, wx, wy)).or_default() += 1;
                }
            }
        }
    }
}

/// A level tile kind seen where the recordings show it: the source's
/// baselines replayed, and on every frame the camera's view holds a cell of
/// the kind, the frame is counted. With `after` (the assets with the edit),
/// the first frames each baseline shows a new cell of the kind are drawn
/// before beside after, a crop around the cell, and the sprites drawn over
/// the kind are measured against it before and after (readability, as a
/// sprite's against its backgrounds). Without `after` nothing is drawn: the
/// count is the evidence an edit's readiness reads.
pub fn tile_in_context(
    t: &Target,
    before: &Path,
    after: Option<&Path>,
    cells: &CellsAt,
) -> Result<(Option<Vec<u8>>, Value), String> {
    let lifted = Lifted::read_dir(before, &t.model)?;
    let probe = Interp::load(&t.model, &lifted)?;
    let arts = match after {
        Some(a) => Some([
            load_art(t, before, probe.spec())?,
            load_art(t, a, probe.spec())?,
        ]),
        None => None,
    };
    let sets: BTreeMap<&str, BTreeSet<[i64; 2]>> = cells
        .iter()
        .map(|(l, c)| (l.as_str(), c.iter().copied().collect()))
        .collect();
    let mut shots: Vec<Shot> = Vec::new();
    let mut shot_cells = Vec::new();
    let mut seen = [Seen::default(), Seen::default()];
    let mut total = 0u64;
    let mut per_run = Vec::new();
    // Each cell is shot once, on the first baseline that shows it.
    let mut shot_here: BTreeSet<(String, [i64; 2])> = BTreeSet::new();
    for (name, input) in &t.runs {
        let mut rt = Interp::load(&t.model, &lifted)?;
        let mut anims = [Animator::new(), Animator::new()];
        let (mut here, mut first, mut taken) = (0u64, None, 0usize);
        for f in input {
            rt.advance(f)?;
            let (spec, w) = (rt.spec(), rt.world());
            if let Some(arts) = &arts {
                for (a, art) in anims.iter_mut().zip(arts) {
                    a.step(&art.present, spec, w);
                }
            }
            let Some(level) = spec.levels.get(w.level) else {
                continue;
            };
            let Some(set) = sets.get(level.name.as_str()) else {
                continue;
            };
            let cam = w
                .cameras
                .first()
                .map_or([0, 0], |c| [c[0].floor(), c[1].floor()]);
            let view = spec.cameras.first().map_or([256, 240], |c| c.size);
            let visible: Vec<[i64; 2]> = set
                .iter()
                .filter(|c| {
                    c[0] + 16 > cam[0]
                        && c[0] < cam[0] + view[0]
                        && c[1] + 16 > cam[1]
                        && c[1] < cam[1] + view[1]
                })
                .copied()
                .collect();
            if visible.is_empty() {
                continue;
            }
            here += 1;
            first.get_or_insert(w.frame);
            let Some(arts) = &arts else {
                continue;
            };
            let scenes = [
                anims[0].scene(&arts[0].present, spec, w),
                anims[1].scene(&arts[1].present, spec, w),
            ];
            for k in 0..2 {
                count_over(&arts[k], &scenes[k], set, &mut seen[k]);
            }
            // A cell of the kind fully in view, not shot yet, gets a shot.
            let new = visible.iter().find(|c| {
                c[0] >= cam[0]
                    && c[0] + 16 <= cam[0] + view[0]
                    && !shot_here.contains(&(level.name.clone(), **c))
            });
            if let Some(c) = new
                && taken < TILE_SHOTS_PER_BASELINE
                && shots.len() < MAX_SHOTS
            {
                shot_here.insert((level.name.clone(), *c));
                taken += 1;
                let (ox, oy) = (
                    c[0] + 8 - i64::from(CROP / 2),
                    c[1] + 8 - i64::from(CROP / 2),
                );
                shots.push((
                    level.name.clone(),
                    name.clone(),
                    w.frame,
                    [
                        crop(&arts[0], &scenes[0], ox, oy),
                        crop(&arts[1], &scenes[1], ox, oy),
                    ],
                ));
                shot_cells.push(*c);
            }
        }
        total += here;
        per_run.push(json!({ "baseline": name, "frames": here, "first": first }));
    }
    let mut out = json!({ "seen": total, "baselines": per_run });
    if arts.is_some() {
        let mut check = readability_check(&readability(&seen[0]), &readability(&seen[1]));
        check["name"] = json!("context.sprites_over_tile");
        check["about"] =
            json!("the sprites drawn over this tile kind on the recorded frames, against it");
        out["shots"] = json!(
            shots
                .iter()
                .zip(&shot_cells)
                .enumerate()
                .map(|(row, ((level, b, f, _), c))| json!({ "row": row, "level": level, "baseline": b, "frame": f, "cell": c }))
                .collect::<Vec<_>>()
        );
        out["layout"] = json!("one row a shot: before, then after");
        out["crop"] = json!(CROP);
        out["scale"] = json!(SCALE);
        out["readability"] = check;
    }
    let png = (!shots.is_empty()).then(|| strip(&shots.iter().map(|s| &s.3).collect::<Vec<_>>()));
    Ok((png, out))
}

/// Rows of before and after crops, `SCALE` times over, as an RGB PNG.
fn strip(rows: &[&[Vec<[u8; 3]>; 2]]) -> Vec<u8> {
    let (s, gap) = (SCALE, 4u32);
    let cell = CROP * s;
    let w = 2 * cell + 3 * gap;
    let h = rows.len() as u32 * (cell + gap) + gap;
    let mut px = vec![0x30u8; (w * h * 3) as usize];
    for (r, pair) in rows.iter().enumerate() {
        for (k, img) in pair.iter().enumerate() {
            let (ox, oy) = (gap + k as u32 * (cell + gap), gap + r as u32 * (cell + gap));
            for y in 0..cell {
                for x in 0..cell {
                    let c = img[((y / s) * CROP + x / s) as usize];
                    let i = (((oy + y) * w + ox + x) * 3) as usize;
                    px[i..i + 3].copy_from_slice(&c);
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().expect("PNG header into memory");
    wr.write_image_data(&px).expect("PNG data into memory");
    wr.finish().expect("PNG end into memory");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readability_weighs_backgrounds_by_how_often() {
        let mut seen = Seen::default();
        seen.ink.insert([0, 0, 0], 90);
        seen.ink.insert([250, 250, 250], 5);
        // Mostly drawn over white, sometimes over black.
        seen.behind.insert([255, 255, 255], 75);
        seen.behind.insert([0, 0, 0], 25);
        let r = readability(&seen);
        assert_eq!(r["share_readable"], 0.75, "{r}");
        assert_eq!(
            r["inks"],
            json!(["#000000"]),
            "the light color is under a tenth of the ink"
        );
        let mut after = Seen::default();
        after.ink.insert([40, 40, 40], 95);
        after.behind = seen.behind.clone();
        let c = readability_check(&r, &readability(&after));
        assert_eq!(c["verdict"], "pass", "{c}");
        let mut dark = Seen::default();
        dark.ink.insert([10, 10, 10], 95);
        dark.behind.insert([0, 0, 0], 100);
        assert_eq!(
            readability_check(&r, &readability(&dark))["verdict"],
            "warn"
        );
    }

    #[test]
    fn jump_points_take_each_poses_first_frame_and_moment() {
        let c = json!({ "poses": [
            { "pose": "a", "first": { "baseline": "x", "frame": 3 }, "moments": [{ "baseline": "x", "frame": 9, "events": ["player.died"] }] },
            { "pose": "b", "first": { "baseline": "x", "frame": 3 }, "moments": [] },
        ]});
        let p = jump_points(&c);
        assert_eq!(p.len(), 2, "{p:?}");
        assert_eq!(p[1].0, "a at player.died");
    }
}
