//! `gp picture GAME --run RUN`: the picture-gap detector. Every clip of a
//! game's corpus is made again into the run (as `gp corpus score` makes
//! them) and played in lockstep, and on each compared frame the screen
//! measure's boxes (gp_interp::screen) are put down to a cause by what is on
//! screen there: the model's bodies and their looks, the original's sprite
//! table, the level's cells and the HUD. The causes are grouped, and each
//! group says its clips, frames and pixels and an example clip and frame.
//!
//! It also gives the corpus's fidelity as `gp corpus score` counts it (each
//! clip's frames together), with every clip scored again by this build, the
//! screen measure's frames the same, and each clip's behaviour, so a change
//! to the picture can be checked to leave behaviour alone. It never writes
//! the corpus or the model.
//!
//! Everything is whole numbers and the clips are taken in ID order, so the
//! same corpus, model and build give the same table.
//!
//! Ported from game-printer's picture.rs. What gameprinter's layout
//! changed: the clips are a game's corpus made into a `gp corpus score`
//! run folder (`$GP_HOME/capture/RUN`), not an old capture run; the model
//! runs through [`crate::hosted::Source`]; the original through
//! [`OriginalRuntime`]. The old fidelity score's parts (behaviour,
//! coverage, picture) aren't in gameprinter, so the fidelity is the corpus
//! score's. An arcade original has no sprite table the tools read yet, so
//! its boxes are put down to causes without one (milestone 3's sprite
//! mask comes later).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::Result;
use clap::Args;
use serde::Serialize;
use serde_json::{Value, json};

use gp_interp::lockstep::Screens;
use gp_interp::scenario::{Outcome as Ran, Scenario};
use gp_interp::screen::{self, Options, Region};
use gp_present::raster::Frame;
use gp_sim::spec::Tile;

use crate::Outcome;
use crate::hosted::{Model, Source};
use crate::original::OriginalRuntime;
use crate::screen::{Art, Probe};

#[derive(Args)]
pub struct PictureArgs {
    /// The game's package folder (games/donkey-kong), with its corpus plan.
    pub game: std::path::PathBuf,
    /// The run's name, as `gp corpus score --run` takes it: its starts are
    /// kept under $GP_HOME/capture/RUN.
    #[arg(long)]
    pub run: String,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<std::path::PathBuf>,
    /// The model to play; by default the game's own.
    #[arg(long)]
    pub model: Option<std::path::PathBuf>,
    /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE,
    /// lifted there first if missing).
    #[arg(long)]
    pub assets: Option<std::path::PathBuf>,
    /// The ROM; by default the one in $GP_ROM_DIR the package names.
    #[arg(long)]
    pub rom: Option<std::path::PathBuf>,
    /// Only this clip, by ID.
    #[arg(long)]
    pub clip: Option<String>,
    /// Details printed per group.
    #[arg(long, default_value_t = 4)]
    pub top: usize,
    /// With `--clip`: write these compared frames (by the frame numbers
    /// the report gives, comma-separated, `A-B` for a range) as PNGs into
    /// `--shots-dir`, the original, the model and the pixels that differ
    /// side by side, each at 2x.
    #[arg(long, requires = "clip", requires = "shots_dir")]
    pub shots: Option<String>,
    /// The folder `--shots` writes, outside any git work tree.
    #[arg(long)]
    pub shots_dir: Option<std::path::PathBuf>,
}

/// The frame numbers `--shots` names: `12,40-45`.
fn shot_frames(list: &str) -> Result<BTreeSet<u64>> {
    let mut out = BTreeSet::new();
    for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => out.extend(a.trim().parse::<u64>()?..=b.trim().parse::<u64>()?),
            None => {
                out.insert(part.parse::<u64>()?);
            }
        }
    }
    Ok(out)
}

/// The original, the model and the pixels that differ (white), side by
/// side at 2x, as a PNG at `to`.
fn write_shot(to: &std::path::Path, o: &Frame, m: &Frame) -> Result<()> {
    let (w, h) = (o.w.min(m.w) as usize, o.h.min(m.h) as usize);
    let (sw, sh) = (w * 3 * 2 + 8, h * 2);
    let mut px = vec![48u8; sw * sh * 3];
    let mut put = |x: usize, y: usize, c: [u8; 3]| {
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let i = ((y * 2 + dy) * sw + x * 2 + dx) * 3;
            px[i..i + 3].copy_from_slice(&c);
        }
    };
    for y in 0..h {
        for x in 0..w {
            let a = o.rgb[y * o.w as usize + x];
            let b = m.rgb[y * m.w as usize + x];
            put(x, y, a);
            put(x + w + 2, y, b);
            put(
                x + 2 * w + 4,
                y,
                if a == b { [0, 0, 0] } else { [255, 255, 255] },
            );
        }
    }
    let mut enc = png::Encoder::new(std::fs::File::create(to)?, sw as u32, sh as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&px)?;
    Ok(())
}

/// The causes a box is put down to, in the order they are tried.
pub(crate) const GROUPS: [&str; 11] = [
    "fallback box (no sprite)",
    "animation frame or flip",
    "palette or color step",
    "HUD",
    "background tiles",
    "draw order and priority",
    "scroll offset",
    "drawn in one game only",
    "flicker",
    "placed elsewhere",
    "other",
];

/// One body as the model drew it on a frame.
#[derive(Debug, Clone)]
struct Drawn {
    kind: String,
    /// Left, top, width, height on the screen.
    rect: [i64; 4],
    /// Its look is a plain box: the kind (in its state) has no sprite.
    fallback: bool,
    /// Drawn behind the level's picture.
    behind: bool,
}

/// What the model showed on one frame, beside its picture.
#[derive(Debug, Clone, Default)]
struct Shown {
    /// The model's frame from the clip's start.
    frame: u64,
    bodies: Vec<Drawn>,
    camera: [i64; 2],
    /// The level picture's masks of cycling colors.
    tints: Vec<u32>,
    level: String,
    clear: [u8; 3],
    /// The original's backdrop color on the frame.
    backdrop: [u8; 3],
    /// Cells in view of a kind a game changes (blocks, bricks, coins), or
    /// changed: screen rect, the level's kind, the kind now.
    cells: Vec<([i64; 4], Tile, Tile)>,
}

/// One box put down to a cause.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Found {
    pub group: &'static str,
    pub detail: String,
    pub frame: u64,
    pub layer: &'static str,
    pub rect: [i64; 4],
    pub pixels: u64,
}

/// What the detector needs of the model, the same for every clip.
#[derive(Debug, Clone, Default)]
pub(crate) struct Looks {
    /// By the spec's kinds, then the state: whether the look is a box.
    fallback: Vec<Vec<bool>>,
    /// The HUD's bottom edge on the screen, in pixels.
    hud_bottom: i64,
}

impl Looks {
    pub(crate) fn of(model: &gp_model::Model, spec: &gp_sim::Spec) -> Self {
        let fallback = spec
            .kinds
            .iter()
            .map(|k| {
                let Some(e) = model.entities.get(&k.name) else {
                    return vec![true];
                };
                match &k.machine {
                    Some(m) => m
                        .states
                        .iter()
                        .map(|s| {
                            e.in_state(&s.name)
                                .map_or(e.core.sprite.is_none(), |e| e.core.sprite.is_none())
                        })
                        .collect(),
                    None => vec![e.core.sprite.is_none()],
                }
            })
            .collect();
        let hud_bottom = model
            .present
            .as_ref()
            .and_then(|p| p.hud.as_ref())
            .and_then(|h| h.size)
            .map_or(0, |s| i64::from(s[1]) * 8);
        Looks {
            fallback,
            hud_bottom,
        }
    }
}

/// The screen measure's screens for a lockstep, watched: each compared
/// frame's boxes put down to causes.
pub(crate) struct Watch {
    probe: Probe,
    looks: Arc<Looks>,
    lag: usize,
    shown: VecDeque<Shown>,
    steps: u64,
    /// The original's sprite pixels on the frame before.
    before: Option<Vec<bool>>,
    pub(crate) found: Vec<Found>,
    /// `--shots`: the frames to write and the folder.
    shots: Option<(BTreeSet<u64>, std::path::PathBuf)>,
}

impl Watch {
    pub(crate) fn new(art: Arc<Art>, looks: Arc<Looks>) -> Self {
        let opts = crate::screen::nes_options();
        Watch {
            probe: Probe::new(art, opts),
            looks,
            lag: usize::from(opts.lag),
            shown: VecDeque::new(),
            steps: 0,
            before: None,
            found: Vec::new(),
            shots: None,
        }
    }

    fn shown_now(&self, model: &Model) -> Shown {
        let art = self.probe.art();
        let scene = self.probe.scene(model);
        let spec = model.spec();
        let w = model.world();
        let [cx, cy] = scene.camera;
        let mut bodies = Vec::new();
        for (i, d) in scene.bodies.iter().enumerate() {
            let (Some(d), Some(b)) = (d, w.bodies.get(i)) else {
                continue;
            };
            let Some(p) = art.present.pictures.get(d.pic as usize) else {
                continue;
            };
            let fallback = self
                .looks
                .fallback
                .get(b.kind)
                .and_then(|s| s.get(b.state).or_else(|| s.first()))
                .copied()
                .unwrap_or(true);
            bodies.push(Drawn {
                kind: spec
                    .kinds
                    .get(b.kind)
                    .map_or_else(String::new, |k| k.name.clone()),
                rect: [
                    d.x - cx,
                    d.y - cy,
                    i64::from(p.size[0]),
                    i64::from(p.size[1]),
                ],
                fallback,
                behind: scene.behind.get(i).copied().unwrap_or(false),
            });
        }
        let mut cells = Vec::new();
        if let Some(level) = spec.levels.get(w.level) {
            let ts = level.tile_size.max(1);
            let tiles = w.tiles(spec);
            let [sw, sh] = art.present.screen.size.map(i64::from);
            let (c0, r0) = (cx.div_euclid(ts), cy.div_euclid(ts));
            for r in r0..=(cy + sh).div_euclid(ts) {
                for c in c0..=(cx + sw).div_euclid(ts) {
                    let (px, py) = (c * ts, r * ts);
                    let Some(k) = level.cell(px, py) else {
                        continue;
                    };
                    let was = level.tiles[k];
                    let now = tiles.at(px, py);
                    let block = |t: Tile| {
                        matches!(
                            t,
                            Tile::Coin | Tile::CoinBlock | Tile::ItemBlock | Tile::Brick
                        )
                    };
                    if was != now || block(was) {
                        cells.push(([px - cx, py - cy, ts, ts], was, now));
                    }
                }
            }
        }
        Shown {
            frame: self.steps,
            bodies,
            camera: [cx, cy],
            tints: scene.tints.iter().map(|(m, _)| *m).collect(),
            level: spec
                .levels
                .get(w.level)
                .map_or_else(String::new, |l| l.name.clone()),
            clear: scene.clear,
            backdrop: [0; 3],
            cells,
        }
    }
}

impl Screens<OriginalRuntime, Model> for Watch {
    fn model_changed(&mut self, model: &Model, loaded: bool) {
        if !loaded {
            self.steps += 1;
        }
        Screens::<OriginalRuntime, Model>::model_changed(&mut self.probe, model, loaded);
    }

    fn model_events(&mut self, model: &Model, events: &[gp_runtime::Event]) {
        Screens::<OriginalRuntime, Model>::model_events(&mut self.probe, model, events);
    }

    fn shots(
        &mut self,
        original: &OriginalRuntime,
        model: &Model,
    ) -> Result<Option<(Frame, Frame)>, String> {
        self.shown.push_back(self.shown_now(model));
        let shots = self.probe.shots(original, model)?;
        if self.shown.len() <= self.lag {
            return Ok(shots);
        }
        while self.shown.len() > self.lag + 1 {
            self.shown.pop_front();
        }
        let mut shown = self.shown.pop_front().expect("a frame was just kept");
        let Some((o, m)) = shots else {
            return Ok(None);
        };
        // An arcade original's sprites aren't read yet: no sprite table,
        // and its picture's first pixel as the backdrop.
        let (oam, tall) = match original.nes() {
            Some(n) => {
                let nes = n.nes();
                shown.backdrop = crate::screen::nes_palette()[usize::from(nes.backdrop() & 0x3f)];
                (nes.oam(), nes.tall_sprites())
            }
            None => {
                shown.backdrop = o.rgb.first().copied().unwrap_or_default();
                ([0xff; 256], false)
            }
        };
        let found = classify(
            &o,
            &m,
            &shown,
            &oam,
            tall,
            &self.looks,
            self.before.as_deref(),
            self.probe.art(),
        );
        self.found.extend(found);
        self.before = Some(o.sprite.clone());
        if let Some((frames, dir)) = &self.shots
            && frames.contains(&shown.frame)
        {
            write_shot(&dir.join(format!("frame-{:05}.png", shown.frame)), &o, &m)
                .map_err(|e| format!("--shots: {e}"))?;
        }
        Ok(Some((o, m)))
    }

    fn options(&self) -> Options {
        Screens::<OriginalRuntime, Model>::options(&self.probe)
    }

    fn original_stepping(&mut self, original: &mut OriginalRuntime) {
        self.probe.original_stepping(original);
    }

    fn animation(&mut self, model: &Model, values: &[(String, i64)]) {
        Screens::<OriginalRuntime, Model>::animation(&mut self.probe, model, values);
    }
}

fn overlap(a: [i64; 4], b: [i64; 4]) -> i64 {
    let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    if w > 0 && h > 0 { w * h } else { 0 }
}

fn rect_of(r: &Region) -> [i64; 4] {
    [r.x, r.y, r.w, r.h]
}

/// An original's sprite: its rect on the screen, tile and attributes.
struct Oam {
    rect: [i64; 4],
    tile: u8,
    attr: u8,
}

fn sprites_of(oam: &[u8; 256], tall: bool) -> Vec<Oam> {
    let h = if tall { 16 } else { 8 };
    oam.chunks(4)
        .filter(|s| s[0] < 0xEF)
        .map(|s| Oam {
            rect: [i64::from(s[3]), i64::from(s[0]) + 1, 8, h],
            tile: s[1],
            attr: s[2],
        })
        .collect()
}

/// Shapes compared over `rect` (original's space): the original's sprite
/// pixels against the model's at an offset `d`, mirrored or not. The share
/// of pixels that disagree, in thousandths, of those either covers.
fn shape_apart(o: &Frame, m: &Frame, rect: [i64; 4], d: [i64; 2], mirror: bool) -> u64 {
    let (mut n, mut apart) = (0u64, 0u64);
    for y in rect[1]..rect[1] + rect[3] {
        for x in rect[0]..rect[0] + rect[2] {
            if x < 0 || y < 0 || x >= i64::from(o.w) || y >= i64::from(o.h) {
                continue;
            }
            let mx = if mirror {
                rect[0] + rect[2] - 1 - (x - rect[0])
            } else {
                x
            } + d[0];
            let my = y + d[1];
            let ours = mx >= 0
                && my >= 0
                && mx < i64::from(m.w)
                && my < i64::from(m.h)
                && m.sprite[(my * i64::from(m.w) + mx) as usize];
            let theirs = o.sprite[(y * i64::from(o.w) + x) as usize];
            if ours || theirs {
                n += 1;
                apart += u64::from(ours != theirs);
            }
        }
    }
    (apart * 1000).checked_div(n).unwrap_or(0)
}

fn sprite_pixels(f: &Frame, rect: [i64; 4]) -> u64 {
    let mut n = 0;
    for y in rect[1].max(0)..(rect[1] + rect[3]).min(i64::from(f.h)) {
        for x in rect[0].max(0)..(rect[0] + rect[2]).min(i64::from(f.w)) {
            n += u64::from(f.sprite[(y * i64::from(f.w) + x) as usize]);
        }
    }
    n
}

fn mask_at(art: &Art, pic: u32, x: i64, y: i64) -> bool {
    let Some(Some(img)) = art.pics.get(pic as usize) else {
        return false;
    };
    if x < 0 || y < 0 || x >= i64::from(img.w) || y >= i64::from(img.h) {
        return false;
    }
    img.px[(y as u32 * img.w + x as u32) as usize][3] >= 128
}

fn tile_words(t: Tile) -> &'static str {
    match t {
        Tile::Coin => "coin",
        Tile::CoinBlock | Tile::ItemBlock => "? block",
        Tile::Brick => "brick",
        Tile::Empty => "empty",
        Tile::Solid => "solid block",
        _ => "tile",
    }
}

/// A shape this far apart, in thousandths, or less, is the same shape.
const SAME_SHAPE: u64 = 50;

#[allow(clippy::too_many_arguments)]
fn classify(
    o: &Frame,
    m: &Frame,
    shown: &Shown,
    oam: &[u8; 256],
    tall: bool,
    looks: &Looks,
    before: Option<&[bool]>,
    art: &Art,
) -> Vec<Found> {
    let (bg, sp) = screen::measure(o, m, 0);
    let mut out = Vec::new();
    let mut push = |group: &'static str, detail: String, layer: &'static str, r: &Region| {
        out.push(Found {
            group,
            detail,
            frame: shown.frame,
            layer,
            rect: rect_of(r),
            pixels: r.pixels,
        });
    };
    let sprites = sprites_of(oam, tall);
    // The sprites' boxes are in the model's frame moved by the
    // background's shift when that was the nearer.
    let d = match (bg.shift, sp.shifted) {
        (Some(s), Some(n)) if n < sp.pixels => s,
        _ => [0, 0],
    };
    let hud = looks.hud_bottom;
    for r in &sp.regions {
        let rr = rect_of(r);
        if hud > 0 && rr[1] + rr[3] <= hud {
            push("HUD", "a sprite in the HUD".into(), "sprites", r);
            continue;
        }
        // The model's bodies over the box, in the original's space, the
        // most overlap first.
        let mut over: Vec<(&Drawn, [i64; 4], i64)> = shown
            .bodies
            .iter()
            .map(|b| {
                let br = [b.rect[0] - d[0], b.rect[1] - d[1], b.rect[2], b.rect[3]];
                (b, br, overlap(br, rr))
            })
            .filter(|x| x.2 > 0)
            .collect();
        over.sort_by_key(|x| std::cmp::Reverse(x.2));
        let theirs: Vec<&Oam> = sprites.iter().filter(|s| overlap(s.rect, rr) > 0).collect();
        if let Some((b, _, _)) = over.iter().find(|x| x.0.fallback) {
            push("fallback box (no sprite)", b.kind.clone(), "sprites", r);
            continue;
        }
        let Some(&(b, br, _)) = over.first() else {
            if theirs.is_empty() {
                push(
                    "other",
                    "the model draws a sprite no body owns".into(),
                    "sprites",
                    r,
                );
                continue;
            }
            // A sprite over a block cell is the block bumped.
            let block = shown.cells.iter().find(|c| {
                overlap([c.0[0] - d[0], c.0[1] - d[1], c.0[2], c.0[3]], rr) > 0
                    && matches!(c.1, Tile::CoinBlock | Tile::ItemBlock | Tile::Brick)
            });
            if let Some(c) = block {
                push(
                    "background tiles",
                    format!("a {} bumped (a sprite in the original)", tile_words(c.1)),
                    "sprites",
                    r,
                );
                continue;
            }
            let tiles: BTreeSet<u8> = theirs.iter().map(|s| s.tile).collect();
            let t: Vec<String> = tiles.iter().take(4).map(|t| format!("{t:02x}")).collect();
            push(
                "drawn in one game only",
                format!("original only: tiles {}", t.join(" ")),
                "sprites",
                r,
            );
            continue;
        };
        if sprite_pixels(o, br) == 0 {
            // Past the eight sprites a line, or shown on the frame before.
            let mut lines = BTreeMap::new();
            for s in &sprites {
                for y in s.rect[1]..s.rect[1] + s.rect[3] {
                    *lines.entry(y).or_insert(0u32) += 1;
                }
            }
            let full = (br[1]..br[1] + br[3]).any(|y| lines.get(&y).is_some_and(|n| *n > 8));
            let was = before.is_some_and(|bf| {
                let f = Frame {
                    w: o.w,
                    h: o.h,
                    rgb: Vec::new(),
                    sprite: bf.to_vec(),
                    visible: o.visible,
                    things: Vec::new(),
                };
                sprite_pixels(&f, br) > 0
            });
            if full || was {
                push("flicker", b.kind.clone(), "sprites", r);
            } else {
                push(
                    "drawn in one game only",
                    format!("rebuilt only: {}", b.kind),
                    "sprites",
                    r,
                );
            }
            continue;
        }
        let theirs_behind = theirs.iter().any(|s| s.attr & 0x20 != 0);
        if theirs_behind != b.behind {
            let way = if theirs_behind {
                "behind the background in the original only"
            } else {
                "behind the background in the rebuilt game only"
            };
            push(
                "draw order and priority",
                format!("{}: {way}", b.kind),
                "sprites",
                r,
            );
            continue;
        }
        if over.len() > 1 && overlap(over[0].1, over[1].1) > 0 {
            let mut k = vec![over[0].0.kind.clone(), over[1].0.kind.clone()];
            k.sort();
            k.dedup();
            push(
                "draw order and priority",
                format!("{} overlap", k.join(" and ")),
                "sprites",
                r,
            );
            continue;
        }
        let rect = [br[0], br[1], br[2], br[3]];
        if shape_apart(o, m, rect, d, false) <= SAME_SHAPE {
            push("palette or color step", b.kind.clone(), "sprites", r);
            continue;
        }
        if shape_apart(o, m, rect, d, true) <= SAME_SHAPE {
            push(
                "animation frame or flip",
                format!("{}: flip", b.kind),
                "sprites",
                r,
            );
            continue;
        }
        // The nearest offset that lines the shapes up, the smallest first.
        let mut offs: Vec<(i64, i64)> = (-4..=4i64)
            .flat_map(|dy| (-4..=4i64).map(move |dx| (dx, dy)))
            .filter(|&(dx, dy)| (dx, dy) != (0, 0))
            .collect();
        offs.sort_by_key(|&(dx, dy)| (dx.abs() + dy.abs(), dy, dx));
        let near = offs
            .into_iter()
            .find(|&(dx, dy)| shape_apart(o, m, rect, [d[0] + dx, d[1] + dy], false) <= SAME_SHAPE);
        match near {
            // The model's picture at (x + dx, y + dy) lines up with the
            // original's at (x, y): it is drawn dx right and dy down of it.
            Some((dx, dy)) => push(
                "placed elsewhere",
                format!(
                    "{}: {}",
                    b.kind,
                    screen::shift_words([dx, dy]).replace(" in the rebuilt game", "")
                ),
                "sprites",
                r,
            ),
            None => push(
                "animation frame or flip",
                format!("{}: frame", b.kind),
                "sprites",
                r,
            ),
        }
    }
    if let Some(s) = bg.shift {
        let all = Region {
            x: i64::from(o.visible[0]),
            y: hud,
            w: i64::from(o.visible[2]),
            h: i64::from(o.visible[3]) - hud,
            pixels: bg.pixels,
            what: Vec::new(),
        };
        push("scroll offset", screen::shift_words(s), "background", &all);
        return out;
    }
    let [cx, cy] = shown.camera;
    for r in &bg.regions {
        let rr = rect_of(r);
        // Most differing pixels on the model's backdrop: the sky's color.
        // and the original's backdrop there: the two backdrops differ.
        let (mut sky, mut n) = (0u64, 0u64);
        for y in rr[1]..rr[1] + rr[3] {
            for x in rr[0]..rr[0] + rr[2] {
                let i = (y * i64::from(o.w) + x) as usize;
                if o.sprite[i] || m.sprite[i] || o.rgb[i] == m.rgb[i] {
                    continue;
                }
                n += 1;
                sky += u64::from(m.rgb[i] == shown.clear && o.rgb[i] == shown.backdrop);
            }
        }
        if n > 0 && sky * 2 > n {
            push(
                "palette or color step",
                format!("the backdrop ({})", shown.level),
                "background",
                r,
            );
            continue;
        }
        if hud > 0 && rr[1] < hud {
            let what = if r.what.is_empty() {
                "the HUD's fixed tiles".to_owned()
            } else {
                r.what.join(", ")
            };
            push("HUD", what, "background", r);
            continue;
        }
        // Differing pixels on a cycling color's mask.
        let (mut on, mut n) = (0u64, 0u64);
        for y in rr[1]..rr[1] + rr[3] {
            for x in rr[0]..rr[0] + rr[2] {
                let i = (y * i64::from(o.w) + x) as usize;
                if o.sprite[i] || m.sprite[i] || o.rgb[i] == m.rgb[i] {
                    continue;
                }
                n += 1;
                on += u64::from(shown.tints.iter().any(|t| mask_at(art, *t, x + cx, y + cy)));
            }
        }
        if n > 0 && on * 2 > n {
            push(
                "palette or color step",
                "a level color step".into(),
                "background",
                r,
            );
            continue;
        }
        let cells: Vec<&([i64; 4], Tile, Tile)> = shown
            .cells
            .iter()
            .filter(|c| overlap(c.0, rr) > 0)
            .collect();
        if let Some(c) = cells.iter().find(|c| c.1 != c.2) {
            let words = match (c.1, c.2) {
                (Tile::Coin, _) => "a coin taken in the rebuilt game".to_owned(),
                (Tile::Brick, Tile::Empty) => "a brick broken in the rebuilt game".to_owned(),
                (a, b) => format!(
                    "a {} that became a {} in the rebuilt game",
                    tile_words(a),
                    tile_words(b)
                ),
            };
            push("background tiles", words, "background", r);
            continue;
        }
        if let Some(c) = cells.first() {
            push(
                "background tiles",
                format!("a {} the original changed", tile_words(c.1)),
                "background",
                r,
            );
            continue;
        }
        push(
            "other",
            format!("the level picture ({})", shown.level),
            "background",
            r,
        );
    }
    out
}

/// One group's or detail's tally.
#[derive(Debug, Clone, Default, Serialize)]
struct Tally {
    clips: BTreeSet<String>,
    frames: u64,
    /// Frames of a layer where this is the only cause: the frames fixing
    /// it alone would make the same.
    alone: u64,
    pixels: u64,
    /// The first clip by ID, its first frame and box.
    example: Option<(String, u64, &'static str, [i64; 4])>,
}

impl Tally {
    fn add(&mut self, clip: &str, f: &Found, new_frame: bool) {
        self.clips.insert(clip.to_owned());
        self.frames += u64::from(new_frame);
        self.pixels += f.pixels;
        if self.example.is_none() {
            self.example = Some((clip.to_owned(), f.frame, f.layer, f.rect));
        }
    }

    fn json(&self) -> Value {
        json!({
            "clips": self.clips.len(),
            "frames": self.frames,
            "alone": self.alone,
            "pixels": self.pixels,
            "example": self.example.as_ref().map(|(c, f, l, r)| json!({"clip": c, "frame": f, "layer": l, "box": r})),
        })
    }
}

/// The body kinds a clip's lockstep paired across the two games.
fn paired_kinds(r: &Ran) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in r.bodies.iter().filter(|b| b.paired) {
        if !out.contains(&b.kind) {
            out.push(b.kind.clone());
        }
    }
    out
}

/// A clip's behaviour, as the corpus score reads it: what must stay the
/// same when only the picture changes.
fn behaviour(r: &Result<Ran>) -> Value {
    match r {
        Ok(r) => json!({
            "exact": r.exact,
            "permille": r.headline.together_permille,
            "score": r.score,
            "drift": r.drift.as_ref().map(|d| d.frame),
            "drift_path": r.drift.as_ref().map(|d| d.path.clone()),
            "tracks": r.tracks.iter().map(|t| &t.path).collect::<Vec<_>>(),
            "paired": paired_kinds(r),
            "error": false,
        }),
        Err(_) => json!({
            "exact": false,
            "permille": 0,
            "score": u64::MAX,
            "drift": null,
            "drift_path": null,
            "tracks": [],
            "paired": [],
            "error": true,
        }),
    }
}

/// Shares as a corpus score gives them: thousandths, `same` of `total`.
fn ppm(same: u64, total: u64) -> u64 {
    (same * 1_000_000).checked_div(total).unwrap_or(0)
}

pub(crate) fn run(args: &PictureArgs, paths: &gp_evidence::Paths) -> Result<Outcome> {
    let game = &args.game;
    let rom = crate::clip_score::rom_of(paths, game, args.rom.as_deref())?;
    let mdir = args.model.clone().unwrap_or_else(|| game.clone());
    let assets = crate::corpus_score::assets_of(paths, game, &mdir, args.assets.as_deref(), &rom)?;
    let (source, _) = Source::open_dir(&mdir, Some(&assets), paths, args.module.as_deref())?;
    let dir = crate::corpus_score::run_dir(&paths.home, &args.run)?;
    std::fs::create_dir_all(&dir)?;
    let work = crate::clipset::Work {
        paths,
        run_dir: &dir,
        run: &args.run,
        mdir: &mdir,
        assets: &assets,
        rom: rom.clone(),
        source: Arc::new(source),
    };
    let set = crate::clipset::read_game(game)?;
    let mut recipes: Vec<crate::clipset::Recipe> = set
        .clips
        .iter()
        .filter(|r| args.clip.as_ref().is_none_or(|id| &r.id == id))
        .cloned()
        .collect();
    if recipes.is_empty() {
        anyhow::bail!(
            "the corpus has no clip {}",
            args.clip.as_deref().unwrap_or("at all")
        );
    }
    recipes.sort_by(|a, b| a.id.cmp(&b.id));
    let b = crate::clipset::bench_for(&work)?;
    let made = crate::clipset::make(&paths.home, &dir, game, &rom, &b.mapping, &recipes)?;
    // Clips that couldn't be made count as errors, as in the corpus score.
    let mut unmade: BTreeMap<String, String> = BTreeMap::new();
    let mut list: Vec<Scenario> = Vec::new();
    for (r, m) in recipes.iter().zip(made) {
        match m {
            Ok(m) => list.push(m.scenario),
            Err(e) => {
                unmade.insert(r.id.clone(), e);
            }
        }
    }
    crate::scenario::resolve_logs(&mut list, &paths.home);
    let (model, text) = b.model()?;
    let mh = crate::scenario::model_hash(&text);
    let spec = b.hosted.spec.clone();
    let looks = Arc::new(Looks::of(&model, &spec));
    let shots = match (&args.shots, &args.shots_dir) {
        (Some(list), Some(dir)) => {
            std::fs::create_dir_all(dir)?;
            Some((shot_frames(list)?, dir.clone()))
        }
        _ => None,
    };
    let ran = crate::scenario::watch_each(
        &b,
        &model,
        &text,
        &list,
        |art, _| {
            art.map(|a| {
                let mut w = Watch::new(a, looks.clone());
                w.shots = shots.clone();
                w
            })
        },
        |w| w.found,
    )?;

    let mut groups: BTreeMap<&'static str, Tally> = BTreeMap::new();
    let mut details: BTreeMap<(&'static str, String), Tally> = BTreeMap::new();
    let mut clips_json = BTreeMap::new();
    let (mut permille_sum, mut sprites, mut background) = (0u64, (0u64, 0u64), (0u64, 0u64));
    for (id, e) in &unmade {
        clips_json.insert(
            id.clone(),
            json!({"behaviour": behaviour(&Err(anyhow::anyhow!("unmade"))), "error": e, "screen": null, "groups": {}}),
        );
    }
    for (s, r) in list.iter().zip(ran) {
        let (outcome, watch) = match r {
            Ok((o, w)) => (Ok(o), w),
            Err(e) => (Err(e), None),
        };
        if let Ok(o) = &outcome {
            permille_sum += o.headline.together_permille;
            if let Some(x) = &o.screen {
                sprites.0 += x.sprites.same;
                sprites.1 += x.sprites.total;
                background.0 += x.background.same;
                background.1 += x.background.total;
            }
        }
        let mut seen: BTreeSet<(&'static str, u64)> = BTreeSet::new();
        let mut seen_d: BTreeSet<(&'static str, String, u64)> = BTreeSet::new();
        let mut mine: BTreeMap<&'static str, u64> = BTreeMap::new();
        for f in watch.iter().flatten() {
            let g = seen.insert((f.group, f.frame));
            groups.entry(f.group).or_default().add(&s.id, f, g);
            let dn = seen_d.insert((f.group, f.detail.clone(), f.frame));
            details
                .entry((f.group, f.detail.clone()))
                .or_default()
                .add(&s.id, f, dn);
            *mine.entry(f.group).or_default() += f.pixels;
        }
        // The causes of each layer's frame, for the frames one holds alone.
        let mut causes: BTreeMap<(&'static str, u64), BTreeSet<(&'static str, String)>> =
            BTreeMap::new();
        for f in watch.iter().flatten() {
            causes
                .entry((f.layer, f.frame))
                .or_default()
                .insert((f.group, f.detail.clone()));
        }
        for set in causes.values() {
            let gs: BTreeSet<&'static str> = set.iter().map(|x| x.0).collect();
            if gs.len() == 1 {
                let g = *gs.iter().next().expect("one group");
                groups.entry(g).or_default().alone += 1;
            }
            if set.len() == 1 {
                let k = set.iter().next().expect("one cause").clone();
                details.entry(k).or_default().alone += 1;
            }
        }
        clips_json.insert(
            s.id.clone(),
            json!({
                "behaviour": behaviour(&outcome),
                "error": outcome.as_ref().err().map(|e| format!("{e:#}")),
                "screen": outcome.as_ref().ok().and_then(|o| o.screen.as_ref()).map(|x| json!({
                    "sprites": [x.sprites.same, x.sprites.total],
                    "background": [x.background.same, x.background.total],
                })),
                "groups": mine,
            }),
        );
    }
    let n = recipes.len() as u64;
    // Every clip counts: one that can't be scored counts as 0.
    let fidelity = permille_sum.checked_div(n).unwrap_or(0);
    let (sprites_ppm, background_ppm) =
        (ppm(sprites.0, sprites.1), ppm(background.0, background.1));

    let mut text_out = String::new();
    let _ = writeln!(
        text_out,
        "{}: {} clips scored again with this build. Fidelity {}.{}% (frames together), picture: characters {}%, background {}% of frames the same.",
        args.run,
        n,
        fidelity / 10,
        fidelity % 10,
        sprites_ppm / 10_000,
        background_ppm / 10_000,
    );
    let _ = writeln!(
        text_out,
        "\n| Group | Clips | Frames | Frames it alone holds | Pixels | Example |"
    );
    let _ = writeln!(text_out, "|---|---|---|---|---|---|");
    let mut order: Vec<&&'static str> = groups.keys().collect();
    order.sort_by_key(|g| {
        (
            std::cmp::Reverse(groups[**g].clips.len()),
            GROUPS.iter().position(|x| *x == **g),
        )
    });
    for g in &order {
        let t = &groups[**g];
        let ex = t.example.as_ref().map_or(String::new(), |(c, f, l, r)| {
            format!(
                "{c} frame {f} ({l} at {}, {} {}x{})",
                r[0], r[1], r[2], r[3]
            )
        });
        let _ = writeln!(
            text_out,
            "| {g} | {} | {} | {} | {} | {ex} |",
            t.clips.len(),
            t.frames,
            t.alone,
            t.pixels
        );
    }
    for g in &order {
        let mut ds: Vec<(&String, &Tally)> = details
            .iter()
            .filter(|((gg, _), _)| gg == *g)
            .map(|((_, d), t)| (d, t))
            .collect();
        ds.sort_by_key(|(d, t)| {
            (
                std::cmp::Reverse(t.clips.len()),
                std::cmp::Reverse(t.frames),
                (*d).clone(),
            )
        });
        let _ = writeln!(text_out, "\n{g}:");
        for (d, t) in ds.iter().take(args.top) {
            let ex = t
                .example
                .as_ref()
                .map_or(String::new(), |(c, f, _, _)| format!("{c} frame {f}"));
            let _ = writeln!(
                text_out,
                "  {d}: {} clips, {} frames, {} alone (e.g. {ex})",
                t.clips.len(),
                t.frames,
                t.alone
            );
        }
    }
    let v = json!({
        "run": args.run,
        "model": mh,
        "clips_scored": n,
        "fidelity": {
            "permille": fidelity,
            "sprites_ppm": sprites_ppm,
            "background_ppm": background_ppm,
        },
        "groups": order.iter().map(|g| {
            let mut j = groups[**g].json();
            j["group"] = json!(g);
            j["details"] = json!(details.iter().filter(|((gg, _), _)| gg == *g).map(|((_, d), t)| {
                let mut x = t.json();
                x["detail"] = json!(d);
                x
            }).collect::<Vec<_>>());
            j
        }).collect::<Vec<_>>(),
        "clips": clips_json,
    });
    Ok(Outcome::ok(v, text_out.trim_end().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 48;
    const H: u32 = 32;

    /// A made-up 48 by 32 frame: a flat backdrop with a stripe every 5
    /// columns below row 8 (so a shift shows), and each sprite's pixels as
    /// an L shape 8 by 8 at its corner, mirrored when asked.
    fn frame(sprites: &[(i64, i64, bool)], scroll: i64) -> Frame {
        let mut f = Frame {
            w: W,
            h: H,
            rgb: vec![[0, 0, 80]; (W * H) as usize],
            sprite: vec![false; (W * H) as usize],
            visible: [0, 0, W, H],
            things: Vec::new(),
        };
        for y in 8..i64::from(H) {
            for x in 0..i64::from(W) {
                if (x + scroll).rem_euclid(5) == 0 {
                    f.rgb[(y * i64::from(W) + x) as usize] = [0, 160, 0];
                }
            }
        }
        for &(sx, sy, mirror) in sprites {
            for y in 0..8 {
                for x in 0..8 {
                    // An L: the left column and the bottom row.
                    let px = if mirror { 7 - x } else { x };
                    if px == 0 || y == 7 {
                        let i = ((sy + y) * i64::from(W) + sx + x) as usize;
                        f.rgb[i] = [250, 200, 0];
                        f.sprite[i] = true;
                    }
                }
            }
        }
        f
    }

    fn body(kind: &str, x: i64, y: i64, fallback: bool) -> Drawn {
        Drawn {
            kind: kind.into(),
            rect: [x, y, 8, 8],
            fallback,
            behind: false,
        }
    }

    /// The original's sprite table with one 8 by 8 sprite at each place.
    fn oam(at: &[(i64, i64)]) -> [u8; 256] {
        let mut o = [0xf8u8; 256];
        for (k, &(x, y)) in at.iter().enumerate() {
            o[k * 4] = (y - 1) as u8;
            o[k * 4 + 1] = 0x10 + k as u8;
            o[k * 4 + 2] = 0;
            o[k * 4 + 3] = x as u8;
        }
        o
    }

    fn causes(
        o: &Frame,
        m: &Frame,
        bodies: Vec<Drawn>,
        at: &[(i64, i64)],
    ) -> Vec<(String, String)> {
        let shown = Shown {
            bodies,
            ..Shown::default()
        };
        classify(
            o,
            m,
            &shown,
            &oam(at),
            false,
            &Looks::default(),
            None,
            &Art::empty(),
        )
        .into_iter()
        .map(|f| (f.group.to_owned(), f.detail))
        .collect()
    }

    #[test]
    fn the_same_picture_has_no_cause() {
        let f = frame(&[(10, 12, false)], 0);
        assert!(causes(&f, &f, vec![body("hero", 10, 12, false)], &[(10, 12)]).is_empty());
    }

    #[test]
    fn a_body_with_no_sprite_is_a_fallback_box() {
        let o = frame(&[(10, 12, false)], 0);
        let mut m = frame(&[], 0);
        for y in 12..20 {
            for x in 10..18 {
                let i = (y * W + x) as usize;
                m.rgb[i] = [136, 112, 0];
                m.sprite[i] = true;
            }
        }
        let got = causes(&o, &m, vec![body("flame", 10, 12, true)], &[(10, 12)]);
        assert_eq!(
            got,
            vec![("fallback box (no sprite)".into(), "flame".into())]
        );
    }

    #[test]
    fn a_mirrored_picture_is_a_flip_and_a_moved_one_is_placed_elsewhere() {
        let o = frame(&[(10, 12, false)], 0);
        let flipped = frame(&[(10, 12, true)], 0);
        let got = causes(&o, &flipped, vec![body("hero", 10, 12, false)], &[(10, 12)]);
        assert_eq!(
            got,
            vec![("animation frame or flip".into(), "hero: flip".into())]
        );
        let moved = frame(&[(11, 12, false)], 0);
        let got = causes(&o, &moved, vec![body("hero", 11, 12, false)], &[(10, 12)]);
        assert_eq!(
            got,
            vec![("placed elsewhere".into(), "hero: 1 px further right".into())]
        );
    }

    #[test]
    fn a_sprite_one_game_draws_is_drawn_in_one_game_only() {
        let o = frame(&[(10, 12, false)], 0);
        let m = frame(&[], 0);
        let got = causes(&o, &m, Vec::new(), &[(10, 12)]);
        assert_eq!(
            got,
            vec![(
                "drawn in one game only".into(),
                "original only: tiles 10".into()
            )]
        );
        let got = causes(&m, &o, vec![body("hero", 10, 12, false)], &[]);
        assert_eq!(
            got,
            vec![("drawn in one game only".into(), "rebuilt only: hero".into())]
        );
    }

    #[test]
    fn a_shifted_background_is_the_scroll() {
        let o = frame(&[], 0);
        let m = frame(&[], -2);
        let got = causes(&o, &m, Vec::new(), &[]);
        assert_eq!(
            got,
            vec![(
                "scroll offset".into(),
                "2 px further right in the rebuilt game".into()
            )]
        );
    }
}
