//! `gp theme sheet` (docs/theme-kit.md, "Sheets"): starting grids and
//! prompts for an image model, from a game's kit and a style card.
//!
//! An image model asked for "a sprite sheet" gives no steady grid, frames
//! of differing sizes and no background to key out (the research note
//! `asset-generation.md`, 2(a)). So we draw the grid: per entity or group
//! of small slots, a picture of bordered cells on a key color, each with a
//! label strip above it (the cell code, the slot's neutral label, the
//! frame and its pose words), a faint box at the slot's size, the line it
//! stands on, and a neutral pose figure drawn from the pose words alone
//! (a stick or a block shape, never anything of the original). The model
//! only paints inside it. With the grids come a prompt file per sheet for
//! each model, the style board and turnaround prompts to make first, a
//! prompt per whole picture (a stage, the score display, a title), and
//! `sheets.json`, which maps each cell back to its slot, frames and anchor
//! so `gp theme fit --sheets` reads the results back.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use gp_theme::pix::Rgba;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Outcome;
use crate::theme_kit::{ArtSlot, Local};
use crate::theme_style::Card;

/// The manifest's format line.
pub const SHEETS_FORMAT: &str = "gp-sheets 1";
/// The manifest's file.
pub const SHEETS_FILE: &str = "sheets.json";
/// The key color: the cells' ground and the gutters.
pub const KEY: [u8; 4] = [255, 0, 255, 255];
/// The most cells a sheet holds.
pub const MAX_CELLS: usize = 16;
/// Gen-4's prompt limit, in characters.
pub const GEN4_LIMIT: usize = 1000;

const GUTTER: u32 = 16;
const BORDER: u32 = 2;
const BOX: [u8; 4] = [208, 0, 208, 255];
const LINE: [u8; 4] = [176, 0, 176, 255];
const FIGURE: [u8; 4] = [168, 168, 168, 255];
const INK: [u8; 4] = [20, 20, 28, 255];
const PAPER: [u8; 4] = [255, 255, 255, 255];
/// The dashed keep box: key-like, so fit clears it with the ground.
const KEEP: [u8; 4] = [160, 0, 160, 255];

/// A dashed rectangle outline, one pixel wide.
fn dashed(img: &mut Rgba, r: [u32; 4], c: [u8; 4]) {
    let [x, y, w, h] = r;
    for i in 0..w {
        if i % 6 < 3 {
            img.set(x + i, y, c);
            img.set(x + i, y + h.saturating_sub(1), c);
        }
    }
    for i in 0..h {
        if i % 6 < 3 {
            img.set(x, y + i, c);
            img.set(x + w.saturating_sub(1), y + i, c);
        }
    }
}

/// The union of the original's bodies in a drawing's frames, in the
/// slot's pixels.
fn keep_box(local: Option<&Local>, id: &str, frames: &[u32]) -> Option<[i64; 4]> {
    let bodies = local?.bodies.get(id)?;
    frames
        .iter()
        .filter_map(|f| bodies.get(*f as usize).copied().flatten())
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
}

/// `sheets.json`: everything a runner needs to make the pictures, and
/// everything `gp theme fit --sheets` needs to read them back. Paths are
/// relative to the manifest's folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// `gp-sheets 1`.
    pub format: String,
    pub game: String,
    pub style: Option<StyleRef>,
    /// The key color, `#FF00FF`.
    pub key: String,
    /// Pictures to make first, in order: the style board, then a
    /// turnaround per main subject.
    pub refs: Vec<RefJob>,
    pub sheets: Vec<Sheet>,
    /// Whole pictures: a stage, the score display, a title screen.
    pub pictures: Vec<PictureJob>,
    /// Slots too small for a model, drawn in code (the font's digits, an
    /// 8 by 8 cursor).
    pub code: Vec<CodeSlot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleRef {
    pub name: String,
    /// The card file, absolute.
    pub card: String,
}

/// A reference picture to make before the sheets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefJob {
    /// `style-board`, or `turn-<subject>`. A later job names it in `refs`
    /// and its prompts tag it `@<id with - as _>`.
    pub id: String,
    /// `style-board` or `turnaround`.
    pub kind: String,
    /// A turnaround's subject: the slots' neutral label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default)]
    pub slots: Vec<String>,
    /// The model to call and the picture's ratio, `W:H`.
    pub model: String,
    pub ratio: String,
    /// The prompt, a text file.
    pub prompt_file: String,
    /// Where the runner writes the picture.
    pub out: String,
    /// A picture the style card already has for it (absolute); when set,
    /// the runner uses it and makes nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Ids of the refs to pass with it.
    #[serde(default)]
    pub refs: Vec<String>,
}

/// One grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sheet {
    /// `sheet-01`, ...
    pub id: String,
    /// The starting grid, a PNG of `size`.
    pub image: String,
    pub size: [u32; 2],
    /// The cells' pixels per slot pixel.
    pub scale: u32,
    /// The model to call first; `prompts` has a file for each model.
    pub model: String,
    pub prompts: BTreeMap<String, String>,
    /// Where the runner writes the filled grid. `fit` scales it to `size`
    /// before cutting the cells.
    pub out: String,
    /// Ids of the refs to pass with the grid.
    pub refs: Vec<String>,
    pub cells: Vec<SheetCell>,
}

/// One cell of a grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetCell {
    /// `A01`: the sheet's letter and the cell's number. Printed on it.
    pub cell: String,
    pub slot: String,
    /// The slot's neutral label.
    pub label: String,
    /// The kit frames this drawing is (frames whose tiles are cut the same
    /// share one drawing), by index and by name.
    pub frames: Vec<u32>,
    pub frame_names: Vec<String>,
    /// The cell's inside, `[x, y, w, h]` in the grid: what fit cuts.
    pub rect: [u32; 4],
    /// The faint box at the slot's size times `scale`, `[x, y, w, h]`.
    pub body: [u32; 4],
    /// The slot's size, pixels.
    pub size: [u32; 2],
    /// The kit's anchor words.
    pub anchor: String,
    /// The row the figure stands on, in the grid (the box's bottom).
    pub baseline: u32,
    pub pose: String,
    /// The dashed box the visible body keeps inside, `[x, y, w, h]` in the
    /// grid: the original's body in these frames, from the local kit
    /// (docs/theme-kit.md, "Gameplay geometry"). Absent without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<[u32; 4]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PictureJob {
    pub id: String,
    pub slot: String,
    pub size: [u32; 2],
    pub model: String,
    pub ratio: String,
    pub prompt_file: String,
    pub out: String,
    /// The geometry guide to pass (ground orange, ladders blue), when the
    /// local kit has one and it may be sent (below).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<String>,
    /// The guide is made from the person's ROM (the lifted level of a
    /// template that ports a commercial game): written only with
    /// `--rom-guides`, and `gp theme generate` sends it only with its own
    /// `--rom-guides`. A guide from an original template's own files is
    /// not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rom_derived: bool,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSlot {
    pub slot: String,
    pub why: String,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Self> {
        let f = dir.join(SHEETS_FILE);
        let m: Self = serde_json::from_str(
            &fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?,
        )
        .with_context(|| format!("{}", f.display()))?;
        if m.format != SHEETS_FORMAT {
            bail!(
                "{}: format `{}`, not `{SHEETS_FORMAT}`",
                f.display(),
                m.format
            );
        }
        Ok(m)
    }
}

/// A tag for a ref id in a prompt.
pub fn tag(id: &str) -> String {
    format!("@{}", id.replace('-', "_"))
}

/// A subject's short key, for its turnaround's id (`turn-hero`): by role,
/// `hero`, `ally` or `foe`, with `2`, `3` ... when taken. Runway's tags
/// are 3 to 16 characters, so ids stay short whatever the labels are.
fn subject_key(a: &ArtSlot, taken: &[String]) -> String {
    let base = match a.role.as_str() {
        "player" => "hero",
        "npc" => "ally",
        _ => "foe",
    };
    let mut k = base.to_owned();
    let mut n = 2;
    while taken.contains(&format!("turn-{k}")) {
        k = format!("{base}{n}");
        n += 1;
    }
    k
}

/// The tags a prompt may use: Runway takes 3 to 16 characters.
pub const TAG_LEN: std::ops::RangeInclusive<usize> = 3..=16;

/// Whether a manifest's ref tags are all of a length Runway takes; the
/// tags too long or short, else none.
pub fn bad_tags(m: &Manifest) -> Vec<String> {
    let mut ids: Vec<&str> = vec!["grid", "guide"];
    ids.extend(m.refs.iter().map(|r| r.id.as_str()));
    for s in &m.sheets {
        ids.extend(s.refs.iter().map(String::as_str));
    }
    for p in &m.pictures {
        ids.extend(p.refs.iter().map(String::as_str));
    }
    let mut bad: Vec<String> = ids
        .into_iter()
        .map(|i| i.replace('-', "_"))
        .filter(|t| !TAG_LEN.contains(&t.len()))
        .collect();
    bad.sort();
    bad.dedup();
    bad
}

/// Whether a slot is a main subject: it gets sheets of its own and a
/// turnaround.
fn is_subject(a: &ArtSlot) -> bool {
    let big = a.size[0] * a.size[1] >= 1024;
    match a.role.as_str() {
        "player" | "npc" | "boss" => true,
        "enemy" => big,
        _ => false,
    }
}

/// Each slot's drawings: frames whose tiles are cut the same (from the
/// local kit) share one. Without it, a drawing a frame.
fn drawings(a: &ArtSlot, local: Option<&Local>) -> Vec<Vec<u32>> {
    let cuts = local.and_then(|l| l.cuts.get(&a.id));
    let mut out: Vec<(String, Vec<u32>)> = Vec::new();
    for k in 0..a.frames {
        let key = match cuts.and_then(|c| c.get(k as usize)) {
            Some(c) => serde_json::to_string(&c.places).unwrap_or_default(),
            None => format!("frame {k}"),
        };
        match out.iter_mut().find(|(x, _)| *x == key) {
            Some((_, v)) => v.push(k),
            None => out.push((key, vec![k])),
        }
    }
    out.into_iter().map(|(_, v)| v).collect()
}

/// A pose phrase: the words file's, else the frame's name.
fn pose_of(a: &ArtSlot, k: u32) -> String {
    a.poses.get(k as usize).cloned().unwrap_or_else(|| {
        format!(
            "frame {}",
            a.frame_names
                .get(k as usize)
                .cloned()
                .unwrap_or_else(|| k.to_string())
        )
    })
}

// Drawing.

fn fill(img: &mut Rgba, x: u32, y: u32, w: u32, h: u32, c: [u8; 4]) {
    for yy in y..y + h {
        for xx in x..x + w {
            img.set(xx, yy, c);
        }
    }
}

fn frame_rect(img: &mut Rgba, x: u32, y: u32, w: u32, h: u32, t: u32, c: [u8; 4]) {
    fill(img, x, y, w, t, c);
    fill(img, x, y + h - t, w, t, c);
    fill(img, x, y, t, h, c);
    fill(img, x + w - t, y, t, h, c);
}

fn dot(img: &mut Rgba, x: f32, y: f32, r: f32, c: [u8; 4]) {
    let (x0, x1) = ((x - r).floor() as i32, (x + r).ceil() as i32);
    let (y0, y1) = ((y - r).floor() as i32, (y + r).ceil() as i32);
    for yy in y0..=y1 {
        for xx in x0..=x1 {
            let (dx, dy) = (xx as f32 + 0.5 - x, yy as f32 + 0.5 - y);
            if dx * dx + dy * dy <= r * r && xx >= 0 && yy >= 0 {
                img.set(xx as u32, yy as u32, c);
            }
        }
    }
}

fn line(img: &mut Rgba, a: (f32, f32), b: (f32, f32), r: f32, c: [u8; 4]) {
    let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) * 2.0)
        .ceil()
        .max(1.0) as i32;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        dot(img, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, r, c);
    }
}

/// Text in the 5 by 7 font, `z` pixels a font pixel; letters it lacks are
/// spaces. Returns the width drawn.
fn text(img: &mut Rgba, x: u32, y: u32, s: &str, z: u32, c: [u8; 4], max_w: u32) {
    let mut cx = x;
    for ch in s.chars() {
        if cx + 5 * z > x + max_w {
            break;
        }
        if let Some(g) = crate::theme_paper::glyph(ch.to_ascii_uppercase()) {
            for (r, row) in g.iter().enumerate() {
                for (k, b) in row.bytes().enumerate() {
                    if b != b'.' && b != b' ' {
                        fill(img, cx + k as u32 * z, y + r as u32 * z, z, z, c);
                    }
                }
            }
        }
        cx += 6 * z;
    }
}

/// Words wrapped to lines of at most `n` characters.
fn wrap(s: &str, n: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in s.split_whitespace() {
        match out.last_mut() {
            Some(l) if l.len() + 1 + w.len() <= n => {
                l.push(' ');
                l.push_str(w);
            }
            _ => out.push(w.chars().take(n).collect()),
        }
    }
    out
}

/// A pose's words, lowercase, one space between, a space at each end, so
/// a word is found by its start (`ring` isn't in `flickering`).
fn spaced(pose: &str) -> String {
    let w: Vec<String> = pose
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    format!(" {} ", w.join(" "))
}

/// What the figure for a pose is: a stick figure, or a block shape.
fn figure_kind(a: &ArtSlot, pose: &str) -> &'static str {
    let p = spaced(pose);
    let has = |ws: &[&str]| ws.iter().any(|w| p.contains(&format!(" {w}")));
    if has(&["draw nothing", "empty"]) {
        return "none";
    }
    let body = matches!(a.role.as_str(), "player" | "npc" | "enemy" | "boss");
    if body
        && has(&[
            "standing", "walking", "climbing", "jumping", "spinning", "running", "stepping",
            "stride", "landing", "swimming", "crouch", "arm", "hand", "legs", "viewer", "bending",
            "holding", "fist", "calling", "riding", "throwing", "turned",
        ])
    {
        return "stick";
    }
    if has(&["number", "points"]) {
        return "number";
    }
    if has(&["ring"]) {
        return "ring";
    }
    if has(&["flame", "ember", "fire", "flicker"]) {
        return "flame";
    }
    if has(&["platform", "lift", "spring", "bar"]) {
        return "bar";
    }
    if has(&["upright", "tool", "swung", "held"]) {
        return "tool";
    }
    if has(&["spark", "burst", "rays"]) {
        return "burst";
    }
    "blob"
}

/// Draws the neutral pose figure for a pose in a box: from the pose's
/// words alone.
fn draw_figure(img: &mut Rgba, a: &ArtSlot, pose: &str, b: [u32; 4]) {
    let (x, y, w, h) = (b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32);
    let p = spaced(pose);
    let has = |ws: &[&str]| ws.iter().any(|w| p.contains(&format!(" {w}")));
    let r = (w.min(h) / 22.0).max(1.0);
    let c = FIGURE;
    match figure_kind(a, pose) {
        "none" => {}
        "stick" => {
            // In a unit box: head, hip, feet, hands. Then turned as the
            // pose says.
            let wide = a.size[0] > a.size[1];
            let sx = if wide { 0.32 } else { 0.22 };
            let crouch = has(&["crouch", "landing", "bending"]);
            let head = (0.5, if crouch { 0.3 } else { 0.14 });
            let neck = (0.5, head.1 + 0.12);
            let hip = (0.5, if crouch { 0.66 } else { 0.6 });
            let apart = has(&[
                "apart", "stride", "walking", "running", "stepping", "spread",
            ]);
            let tucked = has(&["tucked", "jumping"]);
            let (lf, rf) = if tucked {
                ((0.5 - sx * 0.6, 0.8), (0.5 + sx * 0.8, 0.86))
            } else if apart {
                ((0.5 - sx, 1.0), (0.5 + sx, 1.0))
            } else {
                ((0.5 - sx * 0.35, 1.0), (0.5 + sx * 0.35, 1.0))
            };
            let up = (0.5 - sx * 1.1, 0.0);
            let up2 = (0.5 + sx * 1.1, 0.0);
            let down = (0.5 - sx * 1.1, 0.62);
            let down2 = (0.5 + sx * 1.1, 0.62);
            let fwd = (0.5 + sx * 1.8, 0.5);
            let (lh, rh) = if has(&["overhead", "both arms raised", "arms out"]) {
                (up, up2)
            } else if has(&["forward"]) {
                ((0.5 + sx * 1.6, 0.48), fwd)
            } else if has(&[
                "left hand up",
                "right fist",
                "one arm",
                "one fist",
                "waving",
            ]) {
                (up, down2)
            } else if has(&["right hand up", "left fist", "hand up"]) {
                (down, up2)
            } else if has(&["spread", "legs spread"]) {
                ((0.5 - sx * 1.6, 0.3), (0.5 + sx * 1.6, 0.3))
            } else if has(&["holding", "low in front"]) {
                ((0.5 + sx * 0.6, 0.55), (0.5 + sx * 1.2, 0.55))
            } else {
                (down, down2)
            };
            let turn: f32 = if has(&["upside down"]) {
                180.0
            } else if has(&["head to the left"]) {
                -90.0
            } else if has(&["head to the right", "lying forward", "over a ledge"]) {
                90.0
            } else {
                0.0
            };
            let (s, co) = (turn.to_radians().sin(), turn.to_radians().cos());
            let at = |q: (f32, f32)| -> (f32, f32) {
                let (dx, dy) = (q.0 - 0.5, q.1 - 0.5);
                let (rx, ry) = (dx * co - dy * s, dx * s + dy * co);
                (x + (rx + 0.5) * w, y + (ry + 0.5) * h)
            };
            dot(img, at(head).0, at(head).1, (w.min(h) * 0.11).max(1.5), c);
            line(img, at(neck), at(hip), r, c);
            line(img, at(neck), at(lh), r, c);
            line(img, at(neck), at(rh), r, c);
            line(img, at(hip), at(lf), r, c);
            line(img, at(hip), at(rf), r, c);
        }
        "number" => {
            for k in 0..3 {
                let cx = x + w * (0.25 + 0.25 * k as f32);
                frame_rect(
                    img,
                    (cx - w * 0.08) as u32,
                    (y + h * 0.2) as u32,
                    (w * 0.16).max(3.0) as u32,
                    (h * 0.6).max(3.0) as u32,
                    r.max(1.0) as u32,
                    c,
                );
            }
        }
        "ring" => {
            let rad = if has(&["small", "tiny"]) { 0.22 } else { 0.4 };
            let cx = (x + w / 2.0, y + h / 2.0);
            for k in 0..48 {
                let t = k as f32 / 48.0 * std::f32::consts::TAU;
                dot(
                    img,
                    cx.0 + t.cos() * w * rad,
                    cx.1 + t.sin() * h * rad,
                    r,
                    c,
                );
            }
        }
        "flame" => {
            for k in 0..=12 {
                let t = k as f32 / 12.0;
                let half = (1.0 - t) * 0.35 * w;
                line(
                    img,
                    (x + w / 2.0 - half, y + h * (0.95 - t * 0.85)),
                    (x + w / 2.0 + half, y + h * (0.95 - t * 0.85)),
                    r,
                    c,
                );
            }
        }
        "bar" => fill(
            img,
            (x + w * 0.05) as u32,
            (y + h * 0.35) as u32,
            (w * 0.9) as u32,
            (h * 0.4).max(2.0) as u32,
            c,
        ),
        "tool" => {
            let side = has(&["forward", "pointing right", "quarter"]);
            if side {
                line(
                    img,
                    (x + w * 0.05, y + h * 0.5),
                    (x + w * 0.7, y + h * 0.5),
                    r,
                    c,
                );
                fill(
                    img,
                    (x + w * 0.6) as u32,
                    (y + h * 0.3) as u32,
                    (w * 0.35) as u32,
                    (h * 0.4) as u32,
                    c,
                );
            } else {
                line(
                    img,
                    (x + w * 0.5, y + h * 0.95),
                    (x + w * 0.5, y + h * 0.3),
                    r,
                    c,
                );
                fill(
                    img,
                    (x + w * 0.15) as u32,
                    (y + h * 0.05) as u32,
                    (w * 0.7) as u32,
                    (h * 0.3) as u32,
                    c,
                );
            }
        }
        "burst" => {
            let cx = (x + w / 2.0, y + h / 2.0);
            for k in 0..8 {
                let t = k as f32 / 8.0 * std::f32::consts::TAU;
                line(
                    img,
                    (cx.0 + t.cos() * w * 0.2, cx.1 + t.sin() * h * 0.2),
                    (cx.0 + t.cos() * w * 0.42, cx.1 + t.sin() * h * 0.42),
                    r,
                    c,
                );
            }
        }
        _ => {
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let (rx, ry) = (w * 0.42, h * 0.42);
            for yy in b[1]..b[1] + b[3] {
                for xx in b[0]..b[0] + b[2] {
                    let dx = (xx as f32 + 0.5 - cx) / rx;
                    let dy = (yy as f32 + 0.5 - cy) / ry;
                    if dx * dx + dy * dy <= 1.0 {
                        img.set(xx, yy, c);
                    }
                }
            }
        }
    }
}

/// A sheet's layout: cells of one size, near square. Returns the grid's
/// size, the scale and each cell's (tile x, tile y).
struct Layout {
    size: [u32; 2],
    scale: u32,
    cell: [u32; 2],
    label_h: u32,
    cols: u32,
}

/// The sizes a grid is made at: the image models' square, wide and tall
/// pictures (gpt_image_2 1024:1024, 1536:1024 and 1024:1536).
const TARGETS: [[u32; 2]; 3] = [[1024, 1024], [1536, 1024], [1024, 1536]];

fn layout(slots: &[&ArtSlot], n: usize) -> Layout {
    let mw = slots.iter().map(|a| a.size[0]).max().unwrap_or(16);
    let mh = slots.iter().map(|a| a.size[1]).max().unwrap_or(16);
    let n = n.max(1) as u32;
    let mut best: Option<Layout> = None;
    for t in TARGETS {
        for cols in 1..=n {
            let rows = n.div_ceil(cols);
            for scale in (2..=16u32).rev() {
                let cw = (mw * scale + 2 * scale).max(120);
                let label_h = 8 * 3 + 6;
                let ch = mh * scale + 4 * scale;
                let tw = cw + 2 * BORDER;
                let th = label_h + ch + 3 * BORDER;
                let w = cols * tw + (cols + 1) * GUTTER;
                let h = rows * th + (rows + 1) * GUTTER;
                if w <= t[0] && h <= t[1] {
                    // The largest cells win; at the same scale, the
                    // smaller picture.
                    let better = match &best {
                        None => true,
                        Some(b) => {
                            scale > b.scale
                                || scale == b.scale && t[0] * t[1] < b.size[0] * b.size[1]
                        }
                    };
                    if better {
                        best = Some(Layout {
                            size: t,
                            scale,
                            cell: [cw, ch],
                            label_h,
                            cols,
                        });
                    }
                    break;
                }
            }
        }
    }
    best.unwrap_or(Layout {
        size: [1024, 1024],
        scale: 2,
        cell: [mw * 2 + 4, mh * 2 + 8],
        label_h: 30,
        cols: n.min(4),
    })
}

/// One cell to draw: its slot, its frames and its pose.
struct Planned<'a> {
    slot: &'a ArtSlot,
    frames: Vec<u32>,
}

/// The style card's lines for a prompt.
fn style_lines(card: Option<&Card>) -> String {
    let Some(c) = card else {
        return "Style: clean original pixel art, flat colors, a dark outline.\n".into();
    };
    let mut s = format!("Style: {}\n", c.look.trim());
    if !c.palette.is_empty() {
        s.push_str(&format!("Palette hints: {}\n", c.palette.join(", ")));
    }
    if !c.cast.trim().is_empty() {
        s.push_str(&format!("Cast, in the maker's words: {}\n", c.cast.trim()));
    }
    s
}

/// A prompt cut to `limit` characters at a line's end.
fn within(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_owned();
    }
    let mut out = String::new();
    for l in s.lines() {
        if out.len() + l.len() + 1 > limit {
            break;
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// Retry prompts for some cells of filled grids: per sheet holding any of
/// `cells`, `retry-<sheet>.gpt_image_2.txt`, which asks the model to paint
/// those cells again on the filled grid (its `out`) and keep the rest.
pub fn retry_cmd(out: &Path, cells: &[String]) -> Result<Outcome> {
    let m = Manifest::load(out)?;
    let mut wrote = Vec::new();
    let mut found = Vec::new();
    for s in &m.sheets {
        let mine: Vec<&SheetCell> = s.cells.iter().filter(|c| cells.contains(&c.cell)).collect();
        if mine.is_empty() {
            continue;
        }
        if !out.join(&s.out).is_file() {
            bail!("{} has no filled grid ({}) to retry on", s.id, s.out);
        }
        let mut p = format!(
            "The attached grid ({}x{}, @grid) is filled. Paint again only the cells listed below, in the same style and with the same design and colors as the other cells and {}; leave every other cell, the magenta background, the borders and the label strips exactly as they are. Inside each listed cell's box, standing on its line; flat colors, a dark outline, at most three tones; no text.\nCells to paint again:\n",
            s.size[0],
            s.size[1],
            s.refs.iter().map(|r| tag(r)).collect::<Vec<_>>().join(", ")
        );
        for c in &mine {
            p.push_str(&format!("{} {}: {}\n", c.cell, c.label, c.pose));
            found.push(c.cell.clone());
        }
        let f = format!("retry-{}.gpt_image_2.txt", s.id);
        fs::write(out.join(&f), p)?;
        wrote.push(json!({ "sheet": s.id, "grid": s.out, "prompt": f, "cells": mine.iter().map(|c| c.cell.clone()).collect::<Vec<_>>() }));
    }
    let unknown: Vec<&String> = cells.iter().filter(|c| !found.contains(c)).collect();
    if !unknown.is_empty() {
        bail!(
            "no such cells: {}",
            unknown
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(Outcome::ok(
        json!({ "retries": wrote }),
        format!(
            "wrote {} retry prompts in {}: send each with its sheet's filled grid as @grid, write the result over the grid's out file, and fit again",
            wrote.len(),
            out.display()
        ),
    ))
}

/// Writes the sheets, prompts and manifest for a game into `out`.
#[allow(clippy::too_many_lines)]
pub fn sheet_cmd(
    game: &Path,
    style: Option<&str>,
    out: &Path,
    logo: &str,
    force: bool,
    rom_guides: bool,
    paths: &Paths,
) -> Result<Outcome> {
    let tdir = crate::theme_kit::template_dir(game)?;
    let (m, _) = crate::theme::read_model(&tdir)?;
    let (id, _, _) = crate::theme_kit::manifest(&tdir);
    let (kit, problems) = crate::theme_kit::kit_for(&m, &id, &tdir)?;
    if !problems.is_empty() {
        bail!(
            "the words file doesn't fit the kit:\n  {}",
            problems.join("\n  ")
        );
    }
    let deny = crate::theme_words::load(&tdir)?
        .map(|w| w.deny)
        .unwrap_or_default();
    let card = match style {
        Some(s) => Some(crate::theme_style::find(s, paths)?),
        None => None,
    };
    let ldir = crate::theme_kit::local_dir(paths, &kit.game);
    let local: Option<Local> = fs::read_to_string(ldir.join("local.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    if out.join(SHEETS_FILE).exists() && !force {
        bail!(
            "{} has sheets; pass --force to write them again",
            out.display()
        );
    }
    if gp_evidence::paths::enclosing_git_work_tree(out).is_some() {
        bail!(
            "{} is inside a git work tree; sheets stay outside",
            out.display()
        );
    }
    fs::create_dir_all(out.join("refs"))?;
    fs::create_dir_all(out.join("pictures"))?;
    let c = card.as_ref().map(|(c, _)| c);
    let style_text = style_lines(c);
    let mut prompts: Vec<(String, String)> = Vec::new();

    // Plan: the code slots, the subjects' sheets, then props.
    let mut code = Vec::new();
    let mut groups: Vec<(String, Vec<Planned>)> = Vec::new();
    let mut props: Vec<Planned> = Vec::new();
    for a in &kit.art {
        if a.kind == "picture" {
            continue;
        }
        if a.kind == "font" || a.size[0] * a.size[1] <= 64 {
            code.push(CodeSlot {
                slot: a.id.clone(),
                why: format!(
                    "{} by {}: a model's picture downscaled this far is mush; draw it in code",
                    a.size[0], a.size[1]
                ),
            });
            continue;
        }
        let ds = drawings(a, local.as_ref());
        let planned: Vec<Planned> = ds
            .into_iter()
            .map(|f| Planned { slot: a, frames: f })
            .collect();
        if is_subject(a) || planned.len() >= 8 {
            let parts = planned.len().div_ceil(MAX_CELLS).max(1);
            let per = planned.len().div_ceil(parts);
            let mut it = planned.into_iter().peekable();
            while it.peek().is_some() {
                groups.push((a.label.clone(), it.by_ref().take(per).collect()));
            }
        } else {
            props.extend(planned);
        }
    }
    while !props.is_empty() {
        let rest = props.split_off(props.len().min(MAX_CELLS));
        groups.push(("props".into(), props));
        props = rest;
    }

    // Refs: the style board, then a turnaround a subject.
    let mut refs = Vec::new();
    let board_image = card.as_ref().and_then(|(c, f)| {
        c.board
            .as_ref()
            .map(|b| c.resolve(f.parent().unwrap_or(Path::new(".")), b))
    });
    let board = format!(
        "A style board for an original 2D side-view video game. No text, no letters, no logos.\n{style_text}Show, on a plain light background: a row of the palette's color swatches, the hero in three poses (standing, walking, jumping), two foes, a ground tile, a platform and a ladder. Everything original; no character from any existing game.\n"
    );
    refs.push(RefJob {
        id: "style-board".into(),
        kind: "style-board".into(),
        subject: None,
        slots: Vec::new(),
        model: "gen4_image".into(),
        ratio: "1920:1080".into(),
        prompt_file: "refs/style-board.gen4_image.txt".into(),
        out: "refs/style-board.png".into(),
        image: board_image.map(|p| p.display().to_string()),
        refs: Vec::new(),
    });
    prompts.push((
        "refs/style-board.gen4_image.txt".into(),
        within(&board, GEN4_LIMIT),
    ));
    let mut subjects: Vec<&ArtSlot> = Vec::new();
    for a in kit
        .art
        .iter()
        .filter(|a| a.kind == "sprite" && is_subject(a))
    {
        if subjects.iter().any(|s| s.label == a.label) {
            continue;
        }
        subjects.push(a);
    }
    let mut keys: Vec<String> = Vec::new();
    for a in &subjects {
        let rid = format!("turn-{}", subject_key(a, &keys));
        keys.push(rid.clone());
        let have = card.as_ref().and_then(|(c, f)| {
            c.turnaround.iter().find(|t| t.subject == a.label).map(|t| {
                c.resolve(f.parent().unwrap_or(Path::new(".")), &t.image)
                    .display()
                    .to_string()
            })
        });
        let p = format!(
            "A turnaround of one original character, in the style of @style_board. No text.\nThe character: {} ({}).\n{style_text}Three full-body views side by side, the same size and colors: front, side facing right, back. Standing on one line, on a flat magenta (#FF00FF) background.\n",
            a.label, a.description
        );
        let file = format!("refs/{rid}.gen4_image.txt");
        prompts.push((file.clone(), within(&p, GEN4_LIMIT)));
        refs.push(RefJob {
            id: rid.clone(),
            kind: "turnaround".into(),
            subject: Some(a.label.clone()),
            slots: kit
                .art
                .iter()
                .filter(|x| x.label == a.label)
                .map(|x| x.id.clone())
                .collect(),
            model: "gen4_image".into(),
            ratio: "1440:1080".into(),
            prompt_file: file,
            out: format!("refs/{rid}.png"),
            image: have,
            refs: vec!["style-board".into()],
        });
    }

    // The sheets.
    let mut sheets = Vec::new();
    for (si, (_, cells)) in groups.iter().enumerate() {
        let letter = char::from(b'A' + (si % 26) as u8);
        let sid = format!("sheet-{:02}", si + 1);
        let slots: Vec<&ArtSlot> = cells.iter().map(|p| p.slot).collect();
        let lay = layout(&slots, cells.len());
        let mut img = Rgba::new(lay.size[0], lay.size[1]);
        fill(&mut img, 0, 0, lay.size[0], lay.size[1], KEY);
        let tw = lay.cell[0] + 2 * BORDER;
        let th = lay.label_h + lay.cell[1] + 3 * BORDER;
        let mut out_cells = Vec::new();
        for (k, p) in cells.iter().enumerate() {
            let a = p.slot;
            let (col, row) = (k as u32 % lay.cols, k as u32 / lay.cols);
            let x0 = GUTTER + col * (tw + GUTTER);
            let y0 = GUTTER + row * (th + GUTTER);
            frame_rect(&mut img, x0, y0, tw, th, BORDER, INK);
            fill(
                &mut img,
                x0 + BORDER,
                y0 + BORDER,
                lay.cell[0],
                lay.label_h,
                PAPER,
            );
            fill(&mut img, x0, y0 + BORDER + lay.label_h, tw, BORDER, INK);
            let code_id = format!("{letter}{:02}", k + 1);
            let pose = pose_of(a, p.frames[0]);
            let first = a
                .frame_names
                .get(p.frames[0] as usize)
                .cloned()
                .unwrap_or_default();
            let chars = (lay.cell[0] as usize - 4) / 6;
            let mut lines = vec![format!("{code_id} {}", a.label)];
            lines.extend(wrap(&pose, chars).into_iter().take(2));
            for (i, l) in lines.iter().enumerate() {
                text(
                    &mut img,
                    x0 + BORDER + 2,
                    y0 + BORDER + 2 + i as u32 * 10,
                    l,
                    1,
                    INK,
                    lay.cell[0] - 4,
                );
            }
            let rect = [
                x0 + BORDER,
                y0 + 2 * BORDER + lay.label_h,
                lay.cell[0],
                lay.cell[1],
            ];
            let (bw, bh) = (a.size[0] * lay.scale, a.size[1] * lay.scale);
            let bx = rect[0] + (rect[2] - bw) / 2;
            let by = rect[1] + rect[3] - bh - 2 * lay.scale;
            frame_rect(&mut img, bx, by, bw, bh, 1, BOX);
            fill(
                &mut img,
                rect[0] + 2,
                by + bh,
                rect[2] - 4,
                1.max(lay.scale / 4),
                LINE,
            );
            let keep = keep_box(local.as_ref(), &a.id, &p.frames).map(|k| {
                let s = i64::from(lay.scale);
                [
                    (i64::from(bx) + k[0] * s).max(0) as u32,
                    (i64::from(by) + k[1] * s).max(0) as u32,
                    ((k[2] - k[0]) * s).max(1) as u32,
                    ((k[3] - k[1]) * s).max(1) as u32,
                ]
            });
            // The stand-in figure fills the keep box when there is one.
            draw_figure(&mut img, a, &pose, keep.unwrap_or([bx, by, bw, bh]));
            if let Some(r) = keep {
                dashed(&mut img, r, KEEP);
            }
            out_cells.push(SheetCell {
                cell: code_id,
                slot: a.id.clone(),
                label: a.label.clone(),
                frames: p.frames.clone(),
                frame_names: p
                    .frames
                    .iter()
                    .map(|f| {
                        a.frame_names
                            .get(*f as usize)
                            .cloned()
                            .unwrap_or_else(|| f.to_string())
                    })
                    .collect(),
                rect,
                body: [bx, by, bw, bh],
                size: a.size,
                anchor: a.anchor.clone(),
                baseline: by + bh,
                pose,
                keep,
            });
            let _ = first;
        }
        let image = format!("{sid}.png");
        fs::write(out.join(&image), img.png())?;
        // Refs for this sheet: the board and the turnarounds of its slots.
        let mut srefs = vec!["style-board".to_owned()];
        for r in &refs {
            if r.kind == "turnaround" && cells.iter().any(|p| r.slots.contains(&p.slot.id)) {
                srefs.push(r.id.clone());
            }
        }
        let mut subj: Vec<&ArtSlot> = Vec::new();
        for p in cells {
            if !subj.iter().any(|s| s.id == p.slot.id) {
                subj.push(p.slot);
            }
        }
        let tags: Vec<String> = srefs.iter().map(|r| tag(r)).collect();
        let mut gpt = format!(
            "Fill each bordered cell of the attached grid ({}x{}) with one original game sprite, in the style of {}{}.\n{style_text}Rules: paint each figure inside its cell's thin box and standing on the line under it. Where a cell has a dashed box, keep the figure's whole visible body inside it and fill it: the game tests touches and floors there, so a body past it or short of it plays unfairly. The gray stick or block shape in a cell shows the pose and size only: paint over it, never keep it. Flat colors, a dark outline, at most three tones a figure. A subject keeps the same design and colors in every cell. Leave the magenta background, the gutters, the black borders and the white label strips exactly as they are. Add no text, letters or logos. Nothing from any existing game.\nSubjects:\n",
            lay.size[0],
            lay.size[1],
            tags[0],
            if tags.len() > 1 {
                format!(" and the turnaround {}", tags[1..].join(", "))
            } else {
                String::new()
            }
        );
        for s in &subj {
            gpt.push_str(&format!("- {}: {}", s.label, s.description));
            if let Some(g) = &s.geometry
                && !g.danger.is_empty()
            {
                gpt.push_str(&format!(
                    ". It {} in {} of its poses: draw its harmful part solid and plain to see, at least {} by {} pixels of the slot",
                    g.danger_what,
                    if g.danger.len() == s.frames as usize { "all".to_owned() } else { g.danger.len().to_string() },
                    g.min_size[0],
                    g.min_size[1]
                ));
            }
            gpt.push('\n');
        }
        gpt.push_str("Cells:\n");
        for oc in &out_cells {
            gpt.push_str(&format!("{} {}: {}\n", oc.cell, oc.label, oc.pose));
        }
        let mut gen4 = format!(
            "Fill each bordered cell of @grid with {} in the style of @style_board. {} Paint over the gray pose shapes, inside each thin box (and its dashed box), on its line; flat colors, dark outline, same colors every cell. Keep the magenta background, borders and label strips; add no text.\n",
            subj.iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            c.map(|c| c.look.trim().to_owned())
                .unwrap_or_else(|| "Clean original pixel art.".into()),
        );
        for oc in &out_cells {
            let l = format!("{} {}: {}\n", oc.cell, oc.label, oc.pose);
            if gen4.len() + l.len() + 30 > GEN4_LIMIT {
                gen4.push_str("Other cells: as labeled.\n");
                break;
            }
            gen4.push_str(&l);
        }
        let gpt_file = format!("{sid}.gpt_image_2.txt");
        let gen4_file = format!("{sid}.gen4_image.txt");
        prompts.push((gpt_file.clone(), gpt));
        prompts.push((gen4_file.clone(), within(&gen4, GEN4_LIMIT)));
        sheets.push(Sheet {
            id: sid.clone(),
            image,
            size: lay.size,
            scale: lay.scale,
            model: "gpt_image_2".into(),
            prompts: [
                ("gpt_image_2".to_owned(), gpt_file),
                ("gen4_image".to_owned(), gen4_file),
            ]
            .into_iter()
            .collect(),
            out: format!("{sid}.out.png"),
            refs: srefs,
            cells: out_cells,
        });
    }

    // Whole pictures.
    let mut pictures = Vec::new();
    // A stage's guide is the lifted level's ground and ladders: from the
    // person's ROM unless the template is an original (its own files).
    let authored = fs::read_to_string(tdir.join("template.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|v| {
            v.get("source")?
                .get("how")?
                .as_str()
                .map(|h| h == "authored")
        })
        .unwrap_or(false);
    let mut warnings = Vec::new();
    if rom_guides && !authored {
        warnings.push("WARNING: --rom-guides writes stage guides made from your ROM (the original's platforms and ladders). They are marked rom_derived in sheets.json, and gp theme generate sends them to a service only with its own --rom-guides. Never share them.".to_owned());
    }
    for a in kit.art.iter().filter(|a| a.kind == "picture") {
        let pid = a.id.replace('.', "-");
        let mut prefs = vec!["style-board".to_owned()];
        let guide_src =
            a.id.strip_prefix("picture.")
                .map(|r| ldir.join(format!("guide-picture.{r}.png")));
        let guide = match guide_src {
            Some(g) if g.is_file() && (authored || rom_guides) => {
                let to = format!("pictures/{pid}.guide.png");
                fs::copy(&g, out.join(&to))?;
                prefs.push("guide".into());
                Some(to)
            }
            _ => None,
        };
        let p = match a.role.as_str() {
            "menu" => format!(
                "A title screen for an original video game, {} by {}, in the style of @style_board. The only words: the title \"{logo}\" and a short menu, START and OPTIONS. No other text, no logo of any existing game.\n{style_text}",
                a.size[0], a.size[1]
            ),
            "hud" => format!(
                "A score display strip, {} by {}, in the style of @style_board: a plain dark ground with a few small labeled boxes; leave the boxes empty, the numbers are drawn later. No other text.\n{style_text}",
                a.size[0], a.size[1]
            ),
            _ => format!(
                "A side-view game stage, {} by {}, in the style of @style_board: {}. {}{}Nothing else on the platforms; no characters; no text.\n{style_text}",
                a.size[0],
                a.size[1],
                a.description,
                if guide.is_some() {
                    "Put the platforms where @guide is orange and the ladders where it is blue, in the same places and sizes; the top edge of each orange strip is the floor line. "
                } else {
                    ""
                },
                if kit.surfaces.iter().any(|s| s.picture == a.id) {
                    "The top edge of each platform is the floor the characters stand on: keep it flat, sharp and unbroken, with nothing above it; draw each ladder's rungs over its whole height. "
                } else {
                    ""
                }
            ),
        };
        let file = format!("pictures/{pid}.gen4_image.txt");
        prompts.push((file.clone(), within(&p, GEN4_LIMIT)));
        pictures.push(PictureJob {
            id: pid.clone(),
            slot: a.id.clone(),
            size: a.size,
            model: "gen4_image".into(),
            ratio: "1440:1080".into(),
            prompt_file: file,
            out: format!("pictures/{pid}.out.png"),
            rom_derived: guide.is_some() && !authored,
            guide,
            refs: prefs,
        });
    }

    // No prompt may hold a word the game denies.
    let mut bad = Vec::new();
    for (f, t) in &prompts {
        let d = crate::theme_words::denied(t, &deny);
        if !d.is_empty() {
            bad.push(format!("{f}: {}", d.join(", ")));
        }
    }
    if !bad.is_empty() {
        bail!(
            "a prompt holds words the game's words file denies (the style card's or the logo's?):\n  {}",
            bad.join("\n  ")
        );
    }
    for (f, t) in &prompts {
        fs::write(out.join(f), t)?;
    }
    let manifest = Manifest {
        format: SHEETS_FORMAT.into(),
        game: kit.game.clone(),
        style: card.as_ref().map(|(c, f)| StyleRef {
            name: c.name.clone(),
            card: fs::canonicalize(f)
                .unwrap_or_else(|_| f.clone())
                .display()
                .to_string(),
        }),
        key: "#FF00FF".into(),
        refs,
        sheets,
        pictures,
        code,
    };
    let bad = bad_tags(&manifest);
    if !bad.is_empty() {
        bail!(
            "tags Runway won't take (3 to 16 characters): {}",
            bad.join(", ")
        );
    }
    fs::write(
        out.join(SHEETS_FILE),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    let n_cells: usize = manifest.sheets.iter().map(|s| s.cells.len()).sum();
    let gen4_calls =
        manifest.refs.iter().filter(|r| r.image.is_none()).count() + manifest.pictures.len();
    let skipped_guides = manifest
        .pictures
        .iter()
        .filter(|p| p.guide.is_none())
        .count();
    let mut text = warnings.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    if !authored && !rom_guides && local.is_some() && skipped_guides > 0 {
        text.push_str("no stage guides: a guide made from your ROM goes only with --rom-guides\n");
    }
    let text = text
        + &format!(
            "wrote {} sheets ({} cells), {} refs to make first and {} pictures into {}; {} slots to draw in code\ncalls: {} on gen4_image (1080p, 8 credits each) and {} on gpt_image_2 (about 20 credits each at medium quality): about {} credits, ${:.2}",
            manifest.sheets.len(),
            n_cells,
            manifest.refs.len(),
            manifest.pictures.len(),
            out.display(),
            manifest.code.len(),
            gen4_calls,
            manifest.sheets.len(),
            gen4_calls * 8 + manifest.sheets.len() * 20,
            (gen4_calls * 8 + manifest.sheets.len() * 20) as f64 / 100.0
        );
    Ok(Outcome::ok(
        json!({
            "sheets": manifest.sheets.iter().map(|s| json!({"id": s.id, "cells": s.cells.len(), "size": s.size, "scale": s.scale})).collect::<Vec<_>>(),
            "cells": n_cells,
            "refs": manifest.refs.len(),
            "pictures": manifest.pictures.len(),
            "manifest": out.join(SHEETS_FILE),
        }),
        text,
    ))
}

/// The grid a runner returned, at the sheet's size: its `out` file scaled
/// (area average) to `size` when the model gave another.
pub fn filled(dir: &Path, s: &Sheet) -> Result<Option<Rgba>> {
    let f = dir.join(&s.out);
    if !f.is_file() {
        return Ok(None);
    }
    let img = Rgba::load(&f).map_err(anyhow::Error::msg)?;
    Ok(Some(if [img.width, img.height] == s.size {
        img
    } else {
        crate::theme_fit::area_scale(&img, s.size[0], s.size[1])
    }))
}

/// A cell's inside from a filled grid.
pub fn crop(img: &Rgba, r: [u32; 4]) -> Rgba {
    let mut out = Rgba::new(r[2], r[3]);
    for y in 0..r[3] {
        for x in 0..r[2] {
            out.set(x, y, img.get(r[0] + x, r[1] + y));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pose_picks_a_figure_from_its_words() {
        let a = ArtSlot {
            id: "entity.player".into(),
            kind: "sprite".into(),
            role: "player".into(),
            size: [16, 16],
            frames: 1,
            frame_names: vec!["base".into()],
            timing: String::new(),
            colors: String::new(),
            anchor: String::new(),
            description: String::new(),
            label: "hero".into(),
            poses: vec![],
            geometry: None,
        };
        assert_eq!(figure_kind(&a, "walking, legs apart"), "stick");
        assert_eq!(figure_kind(&a, "empty: draw nothing"), "none");
        let mut b = a.clone();
        b.role = "enemy".into();
        assert_eq!(
            figure_kind(&b, "breaking apart: a large hollow ring"),
            "ring"
        );
        assert_eq!(figure_kind(&b, "flickering, tall tips"), "flame");
        let mut img = Rgba::new(64, 64);
        draw_figure(
            &mut img,
            &a,
            "standing, both arms raised overhead",
            [0, 0, 64, 64],
        );
        assert!(img.pixels.contains(&FIGURE));
    }

    #[test]
    fn gen4_prompts_stay_within_the_limit() {
        let long = "a line of words\n".repeat(200);
        assert!(within(&long, GEN4_LIMIT).len() <= GEN4_LIMIT);
    }

    /// One style card makes sheets for gameprinter's own templates (Moss
    /// Hop; game-printer's test had Donkey Kong, Moss Hop and Signal
    /// Failure, and the games checkout's Donkey Kong words file is older
    /// than its model), its look in every prompt, Gen-4's prompts within
    /// the limit, and the manifest reads back.
    #[test]
    fn one_card_makes_sheets_for_three_games() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let card = root.join("themes/styles/riso-site.toml");
        let look = crate::theme_style::load(&card).unwrap().look;
        let home = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: home.path().to_owned(),
            store: home.path().join("store"),
            roms: home.path().join("roms"),
        };
        for dir in crate::theme_template::test_own_templates() {
            let game = dir.display().to_string();
            let out = tempfile::tempdir().unwrap();
            sheet_cmd(
                &dir,
                Some(card.to_str().unwrap()),
                out.path(),
                "SITE CLIMB",
                false,
                false,
                &paths,
            )
            .unwrap_or_else(|e| panic!("{game}: {e:#}"));
            let m = Manifest::load(out.path()).unwrap();
            assert!(!m.sheets.is_empty(), "{game}");
            assert_eq!(m.refs[0].id, "style-board");
            assert!(bad_tags(&m).is_empty(), "{game}: {:?}", bad_tags(&m));
            assert!(
                m.pictures.iter().all(|p| !p.rom_derived),
                "{game}: no ROM guides without the flag"
            );
            for s in &m.sheets {
                assert!(out.path().join(&s.image).is_file());
                let gpt = fs::read_to_string(out.path().join(&s.prompts["gpt_image_2"])).unwrap();
                let gen4 = fs::read_to_string(out.path().join(&s.prompts["gen4_image"])).unwrap();
                assert!(gpt.contains(&look), "{game} {}", s.id);
                assert!(gen4.len() <= GEN4_LIMIT, "{game} {}", s.id);
                for c in &s.cells {
                    assert!(
                        c.rect[0] + c.rect[2] <= s.size[0] && c.rect[1] + c.rect[3] <= s.size[1]
                    );
                    assert!(gpt.contains(&format!("{} {}: {}", c.cell, c.label, c.pose)));
                }
            }
            for r in &m.refs {
                let p = fs::read_to_string(out.path().join(&r.prompt_file)).unwrap();
                assert!(
                    p.len() <= GEN4_LIMIT && p.contains(&look[..20]),
                    "{game} {}",
                    r.id
                );
            }
        }
    }
}
