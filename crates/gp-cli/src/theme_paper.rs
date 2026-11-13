//! The paper painter for a game drawn from tiles, and `gp theme fit`
//! (docs/theme-kit.md). Both fill a theme from a kit: each sprite slot's
//! frames cut into the game's tiles (`theme_kit::Banks`), each picture slot
//! drawn whole, palette rules that color the tiles' tones, and with
//! `paint`, an original tune for each sound role.
//!
//! The painter draws from the kit alone: an entity's role, size, frame and
//! description words, a level's ground and ladder boxes from the model,
//! and the words of a layout that the game's font can read. It never looks
//! at the game's pictures.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use gp_model::Model;
use gp_theme::color::Rgb;
use gp_theme::format::{PaletteRule, Restyle, SoundReplace, sha256_hex};
use gp_theme::pix::Rgba;

use crate::theme_kit::{ArtSlot, Banks, Kit, Local, tone_index};

/// Every paper color the painter uses, for the theme's style guide.
pub fn paper_palette() -> Vec<String> {
    let mut out = Vec::new();
    for r in [
        "player", "enemy", "npc", "hazard", "power-up", "hud", "other",
    ] {
        for c in paper_tones(r) {
            out.push(Rgb(c).to_string());
        }
    }
    for c in [CREAM, CREAM2, SHADOW, INK] {
        out.push(Rgb(c).to_string());
    }
    out
}

/// Paper tones by role: dark, middle, light.
pub(crate) fn paper_tones(role: &str) -> [[u8; 3]; 3] {
    match role {
        "player" => [[0x1f, 0x35, 0x58], [0x3d, 0x6f, 0xb6], [0x9c, 0xc0, 0xe8]],
        "enemy" | "boss" => [[0x5e, 0x1a, 0x10], [0xc4, 0x46, 0x2c], [0xf0, 0x8c, 0x5e]],
        "npc" => [[0x12, 0x4a, 0x44], [0x2a, 0xa0, 0x8c], [0x9c, 0xe8, 0xd8]],
        "hazard" | "projectile" => [[0x2b, 0x33, 0x3b], [0x5b, 0x6b, 0x7a], [0xb0, 0xc0, 0xcc]],
        "power-up" | "collectible" => [[0x6e, 0x52, 0x10], [0xd8, 0xa8, 0x28], [0xf8, 0xe0, 0x90]],
        "hud" | "font" | "menu" => [[0x2a, 0x22, 0x1c], [0x6a, 0x5a, 0x4a], [0xc8, 0xb8, 0x98]],
        _ => [[0x3a, 0x6a, 0x3a], [0x6c, 0xa8, 0x5a], [0xb8, 0xdc, 0x9a]],
    }
}

const CREAM: [u8; 3] = [0xef, 0xe6, 0xcf];
/// Kraft paper: dark, middle, light.
const KRAFT: [[u8; 3]; 3] = [[0x6e, 0x4a, 0x2a], [0xb7, 0x83, 0x4f], [0xd9, 0xab, 0x78]];
const CREAM2: [u8; 3] = [0xe6, 0xdb, 0xc0];
const SHADOW: [u8; 3] = [0xc9, 0xbb, 0x9c];
const INK: [u8; 3] = [0x2a, 0x22, 0x1c];

/// A frame of tones, 0 clear, 1 dark, 2 middle, 3 light.
struct Tones {
    w: i32,
    h: i32,
    t: Vec<u8>,
}

impl Tones {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w: w as i32,
            h: h as i32,
            t: vec![0; (w * h) as usize],
        }
    }
    fn set(&mut self, x: i32, y: i32, v: u8) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.t[(y * self.w + x) as usize] = v;
        }
    }
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, v: u8) {
        for y in 0..self.h {
            for x in 0..self.w {
                let dx = (x as f32 + 0.5 - cx) / rx.max(0.5);
                let dy = (y as f32 + 0.5 - cy) / ry.max(0.5);
                if dx * dx + dy * dy <= 1.0 {
                    self.set(x, y, v);
                }
            }
        }
    }
    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, v: u8) {
        for y in y0.round() as i32..y1.round() as i32 {
            for x in x0.round() as i32..x1.round() as i32 {
                self.set(x, y, v);
            }
        }
    }
    /// The paper finish: an edge where a cut shape meets the clear is
    /// dark, its top and left inner edge light, the rest the middle tone.
    fn finish(&mut self) {
        let src = self.t.clone();
        let at = |x: i32, y: i32| -> u8 {
            if x >= 0 && y >= 0 && x < self.w && y < self.h {
                src[(y * self.w + x) as usize]
            } else {
                0
            }
        };
        for y in 0..self.h {
            for x in 0..self.w {
                let v = at(x, y);
                if v == 0 {
                    continue;
                }
                let edge = at(x + 1, y) == 0
                    || at(x, y + 1) == 0
                    || at(x - 1, y) == 0
                    || at(x, y - 1) == 0;
                let lit = at(x - 1, y - 1) == 0 && !edge;
                let out = if v == 3 {
                    3
                } else if v == 1 || edge && (at(x + 1, y) == 0 || at(x, y + 1) == 0) {
                    1
                } else if lit || edge {
                    3
                } else {
                    2
                };
                self.t[(y * self.w + x) as usize] = out;
            }
        }
    }
}

/// Which shape a sprite slot gets, from its role and description words.
fn shape_of(a: &ArtSlot) -> &'static str {
    let d = a.description.to_lowercase();
    let big = a.size[0] * a.size[1] >= 1024;
    match a.role.as_str() {
        "player" => "figure",
        // Original Edition's 100m ending: the heart between the hero and
        // the friend.
        _ if d.contains("heart") => "heart",
        // A platform the hero rides (Donkey Kong's elevators) is a plank,
        // not a character.
        "npc" if d.contains("platform") => "plank",
        // Original Edition's turning belt ends, cells far apart in one
        // body, are a small wheel in each 8 by 8 cell.
        "npc" if d.contains("end caps") => "wheels",
        // A ladder that moves (Original Edition's 50m) is a ladder.
        "npc" if d.contains("ladder") => "rungs",
        "npc" => "robot",
        "hazard" if d.contains("drum") || d.contains("barrel") => "drum",
        "hazard" => "spikes",
        "power-up" if d.contains("hammer") => "mallet",
        "power-up" | "collectible" => "gem",
        "effect" => "star",
        "menu" => "arrow",
        _ if d.contains("fire") || d.contains("flame") => "flame",
        "enemy" if d.contains("spring") => "spring",
        "enemy" | "boss" if big => "brute",
        _ => "roll",
    }
}

/// A frame as its tiles slot and its places (tile, column, row and flips),
/// sorted: two frames with the same key show the same picture.
#[allow(clippy::type_complexity)]
fn frame_key(c: &crate::theme_kit::Cut) -> (String, Vec<(u32, u32, u32, bool, bool)>) {
    let mut p: Vec<(u32, u32, u32, bool, bool)> = c
        .places
        .iter()
        .map(|p| (p.tile, p.col, p.row, p.flip_x, p.flip_y))
        .collect();
    p.sort_unstable();
    (c.slot.clone(), p)
}

/// The art slot of `ids` whose frames (`cuts`) hold every frame of slot
/// `id` (the same tiles at the same places, flipped the same way), when
/// there is one: the one with the most frames, or with as many, the first.
/// None when `id` is that slot itself.
fn same_frames_as<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    cuts: &BTreeMap<String, Vec<crate::theme_kit::Cut>>,
    id: &str,
) -> Option<&'a str> {
    let key = frame_key;
    let mine: Vec<_> = cuts.get(id)?.iter().map(key).collect();
    let mut best: Option<(&str, usize)> = None;
    for b in ids {
        let Some(theirs) = cuts.get(b) else {
            continue;
        };
        if theirs.is_empty() {
            continue;
        }
        let theirs: Vec<_> = theirs.iter().map(key).collect();
        if !mine.iter().all(|m| theirs.contains(m)) {
            continue;
        }
        if best.is_none_or(|(_, n)| theirs.len() > n) {
            best = Some((b, theirs.len()));
        }
    }
    best.map(|(b, _)| b).filter(|b| *b != id)
}

/// The pose pins kept with a painted theme (`poses.toml` beside its
/// theme.toml): for each art slot, the pose number each group of frames
/// is painted with, by the tiles its first frame shows. The number used
/// to be the group's first frame's place in the slot's frame list, so a
/// frame the base gained (Original Edition's walk off a girder's end and
/// its mirrored walk, 2026-11-07) moved every pose after it: the hero
/// walked and climbed in other poses with nothing about him changed. A
/// group the pins name keeps its number; one they don't takes its place
/// and is pinned at it, so the next paint keeps it too.
#[derive(Debug, Default)]
struct Pins {
    by: BTreeMap<(String, Vec<u32>), Pin>,
}

/// A group's pinned picture: its pose number, the box of the original's
/// bodies it is painted in and whether it is painted the same both ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pin {
    pose: u32,
    frame_box: Option<crate::theme_geometry::Body>,
    mirrored: bool,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PinsFile {
    #[serde(default)]
    pose: Vec<PinRow>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PinRow {
    art: String,
    tiles: Vec<u32>,
    pose: u32,
    #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
    frame_box: Option<crate::theme_geometry::Body>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    mirrored: bool,
}

impl Pins {
    const FILE: &'static str = "poses.toml";

    fn read(path: &Path) -> Result<Pins> {
        let mut pins = Pins::default();
        if let Ok(text) = fs::read_to_string(path) {
            let f: PinsFile =
                toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
            for r in f.pose {
                let pin = Pin {
                    pose: r.pose,
                    frame_box: r.frame_box,
                    mirrored: r.mirrored,
                };
                pins.by.insert((r.art, r.tiles), pin);
            }
        }
        Ok(pins)
    }

    fn write(&self, path: &Path) -> Result<()> {
        let f = PinsFile {
            pose: self
                .by
                .iter()
                .map(|((art, tiles), pin)| PinRow {
                    art: art.clone(),
                    tiles: tiles.clone(),
                    pose: pin.pose,
                    frame_box: pin.frame_box,
                    mirrored: pin.mirrored,
                })
                .collect(),
        };
        let head = "# The pose each group of a slot's frames is painted with, by the tiles\n\
                    # its first frame shows (gp theme paint keeps it: docs/theme-kit.md,\n\
                    # \"Pose pins\"). Change a number to repaint that group in another pose.\n\n";
        fs::write(path, format!("{head}{}", toml::to_string(&f)?))
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// The tiles a frame shows, but the blank, in order, once each.
    fn key(c: &crate::theme_kit::Cut, blank: &[u32]) -> Vec<u32> {
        let set: BTreeSet<u32> = c
            .places
            .iter()
            .map(|p| p.tile)
            .filter(|t| !blank.contains(t))
            .collect();
        set.into_iter().collect()
    }

    /// The pin of the group of `members` (frame indices, first first):
    /// the first member's pin, else any member's in order (a box it
    /// lacks taken from `now`), else `now`, which is then pinned to the
    /// first member's tiles.
    fn pin(
        &mut self,
        art: &str,
        cuts: &[crate::theme_kit::Cut],
        members: &[usize],
        blank: &[u32],
        now: Pin,
    ) -> Pin {
        let keys: Vec<Vec<u32>> = members
            .iter()
            .map(|&j| Pins::key(&cuts[j], blank))
            .collect();
        if let Some(key) = keys
            .iter()
            .find(|k| self.by.contains_key(&(art.to_owned(), (*k).clone())))
        {
            let pin = self
                .by
                .get_mut(&(art.to_owned(), key.clone()))
                .expect("found");
            if pin.frame_box.is_none() {
                pin.frame_box = now.frame_box;
            }
            return *pin;
        }
        if let Some(k) = keys.first() {
            self.by.insert((art.to_owned(), k.clone()), now);
        }
        now
    }
}

/// The character a slot is drawn as, when it is another slot's, a large
/// foe ([`shape_of`] "brute"), whatever the slot's own role (the kit
/// calls these "a character the hero doesn't fight"): a large character
/// of the foe's very size that isn't a foe itself (Original Edition's
/// thrower falling and fallen in 100m's ending), or a figure the game
/// draws as rows of sprites ([`stack_rows`]: the thrower going up with
/// Pauline, five rows of six) as big as a large character and as wide as
/// the foe. It takes the foe's shape and tones, so the thrower going up
/// is the red thrower, not a teal robot (2026-11-07). The first such foe
/// in the kit, or none.
fn look_of<'a>(kit: &'a Kit, local: &Local, a: &ArtSlot) -> Option<&'a ArtSlot> {
    if a.kind != "sprite" {
        return None;
    }
    let brute = |b: &ArtSlot| b.id != a.id && b.kind == "sprite" && shape_of(b) == "brute";
    if shape_of(a) != "brute"
        && a.size[0] * a.size[1] >= 1024
        && let Some(b) = kit.art.iter().find(|b| brute(b) && b.size == a.size)
    {
        return Some(b);
    }
    let cuts = local.cuts.get(&a.id)?;
    let (stacked, _, parts) = stack_rows(cuts, None, true);
    let tall = stacked
        .iter()
        .zip(&parts)
        .filter(|(_, p)| p.len() >= 2)
        .map(|(c, _)| {
            let w = c.places.iter().map(|p| p.col + 1).max().unwrap_or(1) * 8;
            let h = c.places.iter().map(|p| p.row + 1).max().unwrap_or(1) * 8;
            (w, h)
        })
        .max_by_key(|&(w, h)| w * h)?;
    if tall.0 * tall.1 < 1024 {
        return None;
    }
    kit.art.iter().find(|b| brute(b) && b.size[0] == tall.0)
}

/// Frames that are rows of one figure, each a row of tiles: frame j is
/// the row below frame i when it has the same tiles one higher at the same
/// places (Original Edition's thrower going up with Pauline, five bodies
/// of six sprites, the k-th row of a pose from tile $40 + k and on in
/// steps of 5). Painted a row at a time, each row was a figure of its own,
/// a stack of streaks (2026-11-07). Returns the frames with each chain of
/// two or more rows put together as one frame where its top row was, the
/// bodies the same way (a row's moved down its place), and for each frame
/// the original frames it holds with their row.
#[allow(clippy::type_complexity)]
fn stack_rows(
    cuts: &[crate::theme_kit::Cut],
    bodies: Option<&Vec<Option<crate::theme_geometry::Body>>>,
    sprite: bool,
) -> (
    Vec<crate::theme_kit::Cut>,
    Option<Vec<Option<crate::theme_geometry::Body>>>,
    Vec<Vec<(usize, u32)>>,
) {
    let one_row =
        |c: &crate::theme_kit::Cut| c.places.len() >= 2 && c.places.iter().all(|p| p.row == 0);
    let below = |i: usize, j: usize| {
        let (a, b) = (&cuts[i], &cuts[j]);
        i != j
            && one_row(a)
            && one_row(b)
            && a.slot == b.slot
            && a.places.len() == b.places.len()
            && a.places.iter().all(|p| {
                b.places.iter().any(|q| {
                    q.col == p.col
                        && q.tile == p.tile + 1
                        && q.flip_x == p.flip_x
                        && q.flip_y == p.flip_y
                })
            })
    };
    let n = cuts.len();
    let next: Vec<Option<usize>> = (0..n)
        .map(|i| {
            if sprite {
                (0..n).find(|&j| below(i, j))
            } else {
                None
            }
        })
        .collect();
    let has_above: Vec<bool> = (0..n).map(|j| next.contains(&Some(j))).collect();
    let mut chain_of: Vec<Option<Vec<usize>>> = vec![None; n];
    let mut taken = vec![false; n];
    for i in 0..n {
        if has_above[i] || next[i].is_none() {
            continue;
        }
        let mut chain = vec![i];
        let mut at = i;
        while let Some(j) = next[at] {
            if chain.contains(&j) {
                break;
            }
            chain.push(j);
            at = j;
        }
        for &k in &chain {
            taken[k] = true;
        }
        chain_of[i] = Some(chain);
    }
    let mut out = Vec::new();
    let mut out_bodies = Vec::new();
    let mut parts = Vec::new();
    for i in 0..n {
        let body = |k: usize| bodies.and_then(|b| b.get(k).copied().flatten());
        if let Some(chain) = &chain_of[i] {
            let mut c = cuts[i].clone();
            c.places.clear();
            let mut bb: Option<crate::theme_geometry::Body> = None;
            for (r, &k) in chain.iter().enumerate() {
                let r = r as u32;
                c.places
                    .extend(cuts[k].places.iter().map(|p| crate::theme_kit::Place {
                        row: p.row + r,
                        ..p.clone()
                    }));
                if let Some(b) = body(k) {
                    let b = [b[0], b[1] + i64::from(r) * 8, b[2], b[3] + i64::from(r) * 8];
                    bb = Some(bb.map_or(b, |a| {
                        [
                            a[0].min(b[0]),
                            a[1].min(b[1]),
                            a[2].max(b[2]),
                            a[3].max(b[3]),
                        ]
                    }));
                }
            }
            out.push(c);
            out_bodies.push(bb);
            parts.push(
                chain
                    .iter()
                    .enumerate()
                    .map(|(r, &k)| (k, r as u32))
                    .collect(),
            );
        } else if !taken[i] {
            out.push(cuts[i].clone());
            out_bodies.push(body(i));
            parts.push(vec![(i, 0)]);
        }
    }
    (out, bodies.map(|_| out_bodies), parts)
}

/// Each frame's group: the first frame it shares a tile with, directly or
/// through others (one group for frames whose tiles chain together). The
/// game's blank tile (`blank`) is never painted, so it joins no frames:
/// before 2026-11-05 it did, and Donkey Kong's big foe's two side poses,
/// which share no other tile, were painted in the box of all seven
/// frames, 2.5 px off their own.
/// A frame borrowed from (and in [`tile_groups`] apart from) the
/// others is in its own group.
fn tile_groups(cuts: &[crate::theme_kit::Cut], blank: &[u32]) -> Vec<usize> {
    tile_groups_but(cuts, blank, &borrowed_frames(cuts, blank))
}

/// The frames that only show other frames' tiles at other places
/// (`true` for each): every tile, but the blank, is away from its home,
/// the place (column and row, a mirrored frame's column read unmirrored)
/// most frames that show the tile put it (the first such, on a tie).
/// Original Edition draws Mario, as he walks off a girder's end, from
/// tiles 2, 4, 3 and 5: the right half of one walking pose and the left
/// half of the standing one. Painted as a frame of its own it chained the
/// walking poses into one group, a figure in the box of them all
/// (2026-11-07); it is the others' tiles, so it is left to them.
#[allow(clippy::type_complexity)]
fn borrowed_frames(cuts: &[crate::theme_kit::Cut], blank: &[u32]) -> Vec<bool> {
    let place = |c: &crate::theme_kit::Cut, p: &crate::theme_kit::Place| {
        let w = c.places.iter().map(|q| q.col + 1).max().unwrap_or(1);
        let col = if p.flip_x { w - 1 - p.col } else { p.col };
        ((c.slot.clone(), p.tile), (col, p.row))
    };
    let mut seen: BTreeMap<(String, u32), Vec<((u32, u32), usize)>> = BTreeMap::new();
    for c in cuts {
        let mine: BTreeSet<_> = c
            .places
            .iter()
            .filter(|p| !blank.contains(&p.tile))
            .map(|p| place(c, p))
            .collect();
        for (key, at) in mine {
            let v = seen.entry(key).or_default();
            match v.iter_mut().find(|(a, _)| *a == at) {
                Some((_, n)) => *n += 1,
                None => v.push((at, 1)),
            }
        }
    }
    let home: BTreeMap<(String, u32), (u32, u32)> = seen
        .into_iter()
        .map(|(key, v)| {
            let mut best = v[0];
            for &(at, n) in &v[1..] {
                if n > best.1 {
                    best = (at, n);
                }
            }
            (key, best.0)
        })
        .collect();
    cuts.iter()
        .map(|c| {
            let mut ps = c
                .places
                .iter()
                .filter(|p| !blank.contains(&p.tile))
                .peekable();
            ps.peek().is_some()
                && ps.all(|p| {
                    let (key, at) = place(c, p);
                    home.get(&key) != Some(&at)
                })
        })
        .collect()
}

/// [`tile_groups`] with the frames `skip` marks joining nothing.
fn tile_groups_but(cuts: &[crate::theme_kit::Cut], blank: &[u32], skip: &[bool]) -> Vec<usize> {
    let mut g: Vec<usize> = (0..cuts.len()).collect();
    fn root(g: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while g[r] != r {
            r = g[r];
        }
        g[i] = r;
        r
    }
    let skipped = |i: usize| skip.get(i).copied().unwrap_or(false);
    for i in 0..cuts.len() {
        for j in 0..i {
            let shares = !skipped(i)
                && !skipped(j)
                && cuts[i].slot == cuts[j].slot
                && cuts[i]
                    .places
                    .iter()
                    .filter(|p| !blank.contains(&p.tile))
                    .any(|p| cuts[j].places.iter().any(|q| q.tile == p.tile));
            if shares {
                let (a, b) = (root(&mut g, i), root(&mut g, j));
                g[a.max(b)] = a.min(b);
            }
        }
    }
    (0..cuts.len()).map(|i| root(&mut g, i)).collect()
}

/// A cowboy hat in the dark tone, drawn before the finish: a crown
/// `crown` wide from the picture's top to `brim_y`, pinched at the top
/// middle, and a brim `brim` wide from there, `thick` rows deep, its tips
/// turned up a row. Centered on `cx`.
fn hat(t: &mut Tones, cx: f32, crown: f32, brim: f32, brim_y: f32, thick: f32) {
    let y = brim_y.round();
    t.rect(cx - crown / 2.0, 0.0, cx + crown / 2.0, y, 1);
    t.rect(cx - brim / 2.0, y, cx + brim / 2.0, y + thick, 1);
    let (x0, x1) = (
        (cx - brim / 2.0).round() as i32,
        (cx + brim / 2.0).round() as i32 - 1,
    );
    t.set(x0, y as i32 - 1, 1);
    t.set(x1, y as i32 - 1, 1);
    // The pinch: the crown's top middle left clear when it is wide enough.
    let w = crown.round() as i32;
    if w >= 4 {
        let mid = cx.round() as i32;
        for x in mid - (w / 4).max(1)..mid + (w / 4).max(1) {
            if x > (cx - crown / 2.0).round() as i32 && x < (cx + crown / 2.0).round() as i32 - 1 {
                t.set(x, 0, 0);
            }
        }
    }
}

/// The hat's band, after the finish: a light row across the crown just
/// over the brim.
fn hat_band(t: &mut Tones, cx: f32, crown: f32, brim_y: f32) {
    let y = brim_y.round() - 1.0;
    if y >= 2.0 {
        t.rect(cx - crown / 2.0, y, cx + crown / 2.0, y + 1.0, 3);
    }
}

/// A big foe's hand: a round mitt, its middle and radius, on the right
/// side or the left, `y` down the picture as a share of its height. Its
/// size goes by the height alone, which a slot's frames share, so a hand
/// is as big in a narrow pose as in a wide one; its outer edge is where
/// the old oval's was, a twenty-fifth of the box in from its side. Before
/// 2026-11-04 a hand was an oval a tenth of the box's width across, so in
/// the narrow poses it was a sliver or a flat stub against the body (the
/// user: "sometimes half of his hands disappear").
fn brute_hand(fw: f32, fh: f32, right: bool, y: f32) -> (f32, f32, f32) {
    let r = (fh * 0.17).min(fw * 0.16);
    let cx = if right { fw * 0.96 - r } else { fw * 0.04 + r };
    // Inside the picture top and bottom too.
    let cy = (fh * y).clamp(r, fh - r);
    (cx, cy, r)
}

/// A hand: its middle and radius, in the shape's pixels.
type Hand = (f32, f32, f32);

/// The pixels of a hand's disc, as `Tones::ellipse` draws it.
fn disc(&(cx, cy, r): &Hand) -> Vec<(i32, i32)> {
    let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
    let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
    let mut out = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = (x as f32 + 0.5 - cx) / r.max(0.5);
            let dy = (y as f32 + 0.5 - cy) / r.max(0.5);
            if dx * dx + dy * dy <= 1.0 {
                out.push((x, y));
            }
        }
    }
    out
}

/// A hand moved, and made smaller only when it must, to the nearest place
/// `ok` takes. `ok` gives a cost for a place it takes (the number of
/// frames that must hold the same hand there, in a tile group), and the
/// place of least cost is taken at each step. The steps, in turn: up or
/// down with its outer edge kept, a pixel smaller each time down to six
/// tenths of its size; then moved in as well, at those sizes; then
/// smaller, to 2 px. Where no place is taken, there is no hand. Until
/// 2026-11-05 a hand moved in before it moved up or down or got smaller,
/// so in Donkey Kong's side poses, whose cells by the box's side are open
/// only low down, it went into the body and the figure drew 38 px of the
/// original's 43.
fn hand_place(
    (cx, cy, r): Hand,
    fh: f32,
    right: bool,
    ok: &mut dyn FnMut(Hand) -> Option<usize>,
) -> Option<Hand> {
    let inward = if right { -1.0 } else { 1.0 };
    let least = (r * 0.6).max(2.0);
    let sizes = |lo: f32, hi: f32| {
        let mut v = Vec::new();
        let mut rr = r;
        while rr >= 2.0 {
            if rr >= lo && rr <= hi {
                v.push(rr);
            }
            rr -= 1.0;
        }
        v
    };
    let mut steps: Vec<(f32, Vec<i32>)> = Vec::new();
    steps.extend(sizes(least, r).into_iter().map(|rr| (rr, vec![0])));
    steps.extend(
        sizes(least, r)
            .into_iter()
            .map(|rr| (rr, (1..=r as i32).collect())),
    );
    steps.extend(
        sizes(2.0, least - 0.001)
            .into_iter()
            .map(|rr| (rr, (0..=r as i32).collect())),
    );
    for (rr, dxs) in steps {
        let mut best: Option<(usize, Hand)> = None;
        'step: for dx in dxs {
            for d in 0..(fh as i32) {
                for dy in [d as f32, -(d as f32)] {
                    let y = cy + dy;
                    if y - rr < 0.0 || y + rr > fh {
                        continue;
                    }
                    // A smaller hand keeps its outer edge where it was
                    // (before 2026-11-05 its middle moved in by what it
                    // lost, so its edge moved in twice that).
                    let x = cx + inward * dx as f32 - inward * (r - rr);
                    if let Some(cost) = ok((x, y, rr))
                        && best.is_none_or(|b| cost < b.0)
                    {
                        best = Some((cost, (x, y, rr)));
                        if cost <= 1 {
                            break 'step;
                        }
                    }
                }
            }
        }
        if let Some((_, h)) = best {
            return Some(h);
        }
    }
    None
}

/// A big foe's hands in each frame of one tile group, and notes on the
/// frames whose hands had to be smaller or held in common. Frames whose
/// tiles are the same at every place are one view. A tile two views show
/// holds one picture, so a hand may take ink only in a view's own tiles,
/// or in a shared tile where every view that shows it holds the same hand
/// (`at` is the shape's top left in the frame; the shape is the group's,
/// so the same hand gives the same pixels in a tile shown at the same
/// place). A hand is whole: every pixel of its disc in a cell the view
/// draws from a tile other than the blank one. Its reach is its disc and a
/// pixel round it, which the paper finish's edges change. Each view takes
/// its pose's hands where they fit with the fewest other views holding
/// them; a view another's hand reached holds that hand. 2026-11-05: Donkey
/// Kong's big foe's seven frames share tiles, and before this every frame
/// was painted with the first's hands, so two of them lost half a hand
/// where they draw the blank tile (the user: "vanishing hands").
fn group_hands(
    cuts: &[crate::theme_kit::Cut],
    members: &[usize],
    at: [i32; 2],
    (fw, fh): (f32, f32),
    blank: &[u32],
    mirrored: bool,
) -> (BTreeMap<usize, [Option<Hand>; 2]>, Vec<String>) {
    use crate::theme_kit::Place;
    let same = |a: &[Place], b: &[Place]| {
        a.len() == b.len()
            && a.iter().zip(b).all(|(p, q)| {
                (p.tile, p.col, p.row, p.flip_x, p.flip_y)
                    == (q.tile, q.col, q.row, q.flip_x, q.flip_y)
            })
    };
    let mut views: Vec<Vec<usize>> = Vec::new();
    for &k in members {
        match views
            .iter_mut()
            .find(|v| same(&cuts[v[0]].places, &cuts[k].places))
        {
            Some(v) => v.push(k),
            None => views.push(vec![k]),
        }
    }
    let cell = |v: usize, x: i32, y: i32| -> Option<&Place> {
        let (fx, fy) = (x + at[0], y + at[1]);
        if fx < 0 || fy < 0 || x < 0 || y < 0 || x >= fw as i32 || y >= fh as i32 {
            return None;
        }
        cuts[views[v][0]]
            .places
            .iter()
            .find(|p| p.col == fx as u32 / 8 && p.row == fy as u32 / 8 && !blank.contains(&p.tile))
    };
    let mirror = |px: Vec<(i32, i32)>| -> Vec<(i32, i32)> {
        if !mirrored {
            return px;
        }
        let w = fw as i32;
        let m: Vec<(i32, i32)> = px.iter().map(|&(x, y)| (w - 1 - x, y)).collect();
        px.into_iter().chain(m).collect()
    };
    let mut hands: Vec<[Option<Option<Hand>>; 2]> = vec![[None, None]; views.len()];
    let mut notes = Vec::new();
    let frames = |vs: &[usize]| -> String {
        let mut ks: Vec<usize> = vs.iter().flat_map(|&v| views[v].iter().copied()).collect();
        ks.sort_unstable();
        let ks: Vec<String> = ks.iter().map(|k| k.to_string()).collect();
        match ks.len() {
            1 => format!("frame {}", ks[0]),
            n => format!("frames {} and {}", ks[..n - 1].join(", "), ks[n - 1]),
        }
    };
    for v in 0..views.len() {
        let pose = views[v][0] as u32 % 6;
        let (ly, ry) = brute_pose(pose);
        for (s, (right, y)) in [(false, ly), (true, ry)].into_iter().enumerate() {
            if hands[v][s].is_some() {
                continue;
            }
            let want = brute_hand(fw, fh, right, y);
            // The views that must hold `h`, or None where it can't be
            // whole in one of them or another already holds another hand.
            let held = |h: Hand, hands: &[[Option<Option<Hand>>; 2]]| -> Option<Vec<usize>> {
                let body = mirror(disc(&h));
                let mut reach: BTreeSet<(i32, i32)> = BTreeSet::new();
                for &(x, y) in &body {
                    for (dx, dy) in [
                        (-1, -1),
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (0, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                    ] {
                        reach.insert((x + dx, y + dy));
                    }
                }
                let mut set = vec![v];
                let mut i = 0;
                while i < set.len() {
                    let u = set[i];
                    i += 1;
                    if hands[u][s].is_some_and(|o| o != Some(h)) {
                        return None;
                    }
                    if body.iter().any(|&(x, y)| cell(u, x, y).is_none()) {
                        return None;
                    }
                    for &(x, y) in &reach {
                        let Some(p) = cell(u, x, y) else { continue };
                        for o in 0..views.len() {
                            if set.contains(&o) {
                                continue;
                            }
                            let shows =
                                cuts[views[o][0]].places.iter().filter(|q| q.tile == p.tile);
                            let mut any = false;
                            for q in shows {
                                if (q.col, q.row, q.flip_x, q.flip_y)
                                    != (p.col, p.row, p.flip_x, p.flip_y)
                                {
                                    return None;
                                }
                                any = true;
                            }
                            if any {
                                set.push(o);
                            }
                        }
                    }
                }
                Some(set)
            };
            let mut ok = |h: Hand| held(h, &hands).map(|set| set.len());
            let got = hand_place(want, fh, right, &mut ok);
            let side = if right { "right" } else { "left" };
            match got {
                Some(h) => {
                    let set = held(h, &hands).unwrap_or_else(|| vec![v]);
                    for &u in &set {
                        hands[u][s] = Some(Some(h));
                    }
                    if set.len() > 1 {
                        notes.push(format!(
                            "{} hold one {side} hand, at {:.0}, {:.0}, through tiles they share",
                            frames(&set),
                            h.0,
                            h.1
                        ));
                    }
                    if h.2 < want.2 {
                        let outer = if right { h.0 + h.2 } else { h.0 - h.2 };
                        let kept = (outer
                            - if right {
                                want.0 + want.2
                            } else {
                                want.0 - want.2
                            })
                        .abs()
                            < 0.01;
                        notes.push(format!(
                            "{}: the {side} hand is {:.1} px across, of {:.1}: no bigger disc is whole {} in cells the frame alone draws, or shares only with frames that can hold the same hand",
                            frames(&[v]),
                            2.0 * h.2,
                            2.0 * want.2,
                            if kept { "at the box's side" } else { "anywhere" }
                        ));
                    }
                }
                None => {
                    hands[v][s] = Some(None);
                    notes.push(format!(
                        "{}: no {side} hand: no disc of 4 px or more is whole in cells the frame alone draws, or shares only with frames that can hold the same hand",
                        frames(&[v])
                    ));
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    for (v, ks) in views.iter().enumerate() {
        for &k in ks {
            out.insert(k, hands[v].map(Option::flatten));
        }
    }
    (out, notes)
}

/// A big foe's pose: where its left and right hands are, down the picture
/// as a share of its height.
fn brute_pose(pose: u32) -> (f32, f32) {
    match pose % 6 {
        1 => (0.3, 0.7),
        2 => (0.25, 0.25),
        3 => (0.7, 0.3),
        4 => (0.45, 0.45),
        5 => (0.35, 0.55),
        _ => (0.7, 0.7),
    }
}

/// Paints frame `k` of a sprite slot, `w` by `h`, in tones.
fn paint_frame(shape: &str, w: u32, h: u32, k: u32) -> Tones {
    paint_frame_in(shape, w, h, k, None)
}

/// [`paint_frame`] with a big foe's hands where `hands` puts them (left,
/// then right; None for no hand), from [`group_hands`]. Without `hands`
/// they are where the pose puts them.
fn paint_frame_in(shape: &str, w: u32, h: u32, k: u32, hands: Option<[Option<Hand>; 2]>) -> Tones {
    let mut t = Tones::new(w, h);
    let (fw, fh) = (w as f32, h as f32);
    match shape {
        "figure" => {
            // A cowboy hat, a round head, a coat, two legs and two arms,
            // in turns. The legs step a pixel apart and back, and the
            // arms lift to the shoulders, never over the head, so every
            // pose keeps a hat, a head, two arms and two legs apart.
            t.ellipse(fw * 0.5, fh * 0.32, fw * 0.22, fh * 0.12, 2);
            t.rect(fw * 0.28, fh * 0.38, fw * 0.72, fh * 0.7, 2);
            hat(&mut t, fw * 0.5, fw * 0.5, fw * 0.92, fh * 0.16, 1.0);
            let spread = [0.0, 1.0, 0.0, 0.0][(k % 4) as usize];
            t.rect(fw * 0.3 - spread, fh * 0.68, fw * 0.46 - spread, fh, 2);
            t.rect(fw * 0.54 + spread, fh * 0.68, fw * 0.7 + spread, fh, 2);
            let up = k % 3 == 1;
            let ay = if up { fh * 0.3 } else { fh * 0.42 };
            t.rect(fw * 0.06, ay, fw * 0.26, ay + fh * 0.2, 2);
            t.rect(fw * 0.74, ay, fw * 0.94, ay + fh * 0.2, 2);
            t.finish();
            hat_band(&mut t, fw * 0.5, fw * 0.5, fh * 0.16);
            // A paper band across the coat.
            t.rect(fw * 0.3, fh * 0.5, fw * 0.7, fh * 0.56, 3);
        }
        "robot" => {
            // A small paper robot: ball on an antenna, a square head with
            // two eyes, a boxed body, stub legs. One arm waves, then the
            // other, and the ball and the feet shift with it.
            let odd = k % 2 == 1;
            let bob = if odd { 1.0 } else { 0.0 };
            t.rect(fw * 0.44, fh * 0.1, fw * 0.56, fh * 0.2, 2);
            t.ellipse(fw * 0.5, fh * 0.07 + bob * 0.5, fw * 0.12, fh * 0.07, 2);
            t.rect(fw * 0.19, fh * 0.2, fw * 0.81, fh * 0.46, 2);
            t.rect(fw * 0.4, fh * 0.46, fw * 0.6, fh * 0.52, 2);
            t.rect(fw * 0.125, fh * 0.5, fw * 0.875, fh * 0.58, 2);
            t.rect(fw * 0.19, fh * 0.5, fw * 0.81, fh * 0.8, 2);
            let (lu, ru) = (!odd, odd);
            let (ly, ry) = (
                if lu { fh * 0.24 } else { fh * 0.5 },
                if ru { fh * 0.24 } else { fh * 0.5 },
            );
            // The arms a pixel in from the picture's sides, so they aren't
            // cut by its edge.
            t.rect(1.0, ly, 1.0 + fw * 0.125, ly + fh * 0.34, 2);
            t.rect(fw * 0.875 - 1.0, ry, fw - 1.0, ry + fh * 0.34, 2);
            t.rect(fw * 0.19 + bob, fh * 0.8, fw * 0.44 + bob, fh, 2);
            t.rect(fw * 0.56 + bob, fh * 0.8, fw * 0.81 + bob, fh, 2);
            t.finish();
            // Eyes, a chest panel and its two lamps.
            t.rect(fw * 0.31, fh * 0.28, fw * 0.44, fh * 0.35, 3);
            t.rect(fw * 0.56, fh * 0.28, fw * 0.69, fh * 0.35, 3);
            t.rect(fw * 0.31, fh * 0.56, fw * 0.69, fh * 0.72, 3);
            t.rect(fw * 0.38, fh * 0.61, fw * 0.5, fh * 0.65, 1);
            t.set((fw * 0.6) as i32, (fh * 0.63) as i32, 1);
        }
        "brute" => {
            // A big paper bear-like brute in a cowboy hat, arms in one of six
            // poses.
            t.ellipse(fw * 0.5, fh * 0.64, fw * 0.3, fh * 0.34, 2);
            t.ellipse(fw * 0.5, fh * 0.33, fw * 0.16, fh * 0.18, 2);
            hat(
                &mut t,
                fw * 0.5,
                fw * 0.24,
                fw * 0.5,
                fh * 0.17,
                (fh * 0.07).round().max(1.0),
            );
            let (ly, ry) = brute_pose(k);
            let hands = hands.unwrap_or([
                Some(brute_hand(fw, fh, false, ly)),
                Some(brute_hand(fw, fh, true, ry)),
            ]);
            for (cx, cy, r) in hands.into_iter().flatten() {
                t.ellipse(cx, cy, r, r, 2);
            }
            t.ellipse(fw * 0.36, fh * 0.94, fw * 0.1, fh * 0.07, 2);
            t.ellipse(fw * 0.64, fh * 0.94, fw * 0.1, fh * 0.07, 2);
            t.finish();
            hat_band(&mut t, fw * 0.5, fw * 0.24, fh * 0.17);
            t.ellipse(fw * 0.5, fh * 0.66, fw * 0.16, fh * 0.18, 3);
            t.set((fw * 0.44) as i32, (fh * 0.34) as i32, 1);
            t.set((fw * 0.56) as i32, (fh * 0.34) as i32, 1);
        }
        "plank" => {
            // A strip of card, its ends a shade darker: a platform keeps
            // its whole box, as the hero stands on its top.
            t.rect(0.0, 0.0, fw, fh, 2);
            t.finish();
            t.rect(1.0, fh * 0.5, 2.0, fh * 0.5 + 1.0, 1);
            t.rect(fw - 2.0, fh * 0.5, fw - 1.0, fh * 0.5 + 1.0, 1);
        }
        "wheels" => {
            // The same wheel in every 8 by 8 cell, so a tile two frames
            // show at other cells holds one picture: a ring and its hub.
            let mut cy = 0.0;
            while cy + 8.0 <= fh {
                let mut cx = 0.0;
                while cx + 8.0 <= fw {
                    t.ellipse(cx + 4.0, cy + 4.0, 3.5, 3.5, 2);
                    cx += 8.0;
                }
                cy += 8.0;
            }
            t.finish();
            let mut cy = 0.0;
            while cy + 8.0 <= fh {
                let mut cx = 0.0;
                while cx + 8.0 <= fw {
                    t.ellipse(cx + 4.0, cy + 4.0, 1.0, 1.0, 3);
                    cx += 8.0;
                }
                cy += 8.0;
            }
        }
        "rungs" => {
            // Two paper rails and a rung every four rows, filling its box:
            // the hero climbs it, so it keeps the original's place.
            let rail = (fw * 0.15).round().max(2.0);
            t.rect(fw * 0.12, 0.0, fw * 0.12 + rail, fh, 2);
            t.rect(fw * 0.88 - rail, 0.0, fw * 0.88, fh, 2);
            let mut y = 1.0;
            while y + 2.0 <= fh {
                t.rect(fw * 0.12, y, fw * 0.88, y + 2.0, 2);
                y += 4.0;
            }
            t.finish();
            t.rect(fw * 0.12 + 1.0, 0.0, fw * 0.12 + 2.0, fh, 3);
        }
        "spring" => {
            // A paper coil between two plates, as tall as its box: a
            // pressed frame has a lower box, so its coil is squat.
            let plate = (fh * 0.2).round().max(2.0);
            t.rect(0.0, 0.0, fw, plate, 2);
            t.rect(0.0, fh - plate, fw, fh, 2);
            let (top, bottom) = (plate, fh - plate);
            let turns = 3.0;
            for y in top as i32..bottom as i32 {
                let u = (y as f32 + 0.5 - top) / (bottom - top).max(1.0) * turns;
                let tri = 1.0 - 2.0 * (u.fract() - 0.5).abs();
                let x = fw * 0.2 + tri * fw * 0.6;
                t.rect(x - 1.5, y as f32, x + 1.5, y as f32 + 1.0, 2);
            }
            t.finish();
            t.rect(1.0, 1.0, fw - 1.0, 2.0, 3);
        }
        "roll" => {
            // A rolled paper tube seen end on, its curl turning.
            t.ellipse(fw * 0.5, fh * 0.5, fw * 0.46, fh * 0.46, 2);
            t.finish();
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            for r in 1..(fw.min(fh) * 0.42) as i32 {
                let x = fw * 0.5 + a.cos() * r as f32;
                let y = fh * 0.5 + a.sin() * r as f32;
                t.set(x as i32, y as i32, 1);
            }
            t.ellipse(fw * 0.5, fh * 0.5, 1.5, 1.5, 3);
        }
        "flame" => {
            let sway = if k.is_multiple_of(2) { -1.0 } else { 1.0 };
            // Filling its box, as the hazard it draws is that big.
            t.ellipse(fw * 0.5, fh * 0.69, fw * 0.47, fh * 0.31, 2);
            for y in 0..(fh * 0.7) as i32 {
                let k2 = y as f32 / (fh * 0.7);
                let half = fw * 0.45 * k2;
                let cx = fw * 0.5 + sway * (1.0 - k2) * 1.5;
                for x in (cx - half).round() as i32..(cx + half).round() as i32 {
                    t.set(x, y, 2);
                }
            }
            t.finish();
            t.ellipse(fw * 0.5, fh * 0.74, fw * 0.12, fh * 0.12, 3);
        }
        "drum" => {
            t.rect(fw * 0.1, fh * 0.15, fw * 0.9, fh, 2);
            t.ellipse(fw * 0.5, fh * 0.15, fw * 0.4, fh * 0.12, 2);
            t.finish();
            t.rect(fw * 0.1, fh * 0.45, fw * 0.9, fh * 0.45 + 1.0, 1);
            t.rect(fw * 0.1, fh * 0.75, fw * 0.9, fh * 0.75 + 1.0, 1);
            if k % 2 == 1 {
                t.rect(fw * 0.3, fh * 0.55, fw * 0.7, fh * 0.65, 3);
            }
        }
        "mallet" if w > h => {
            // Swung forward: the handle across, the head at the far end.
            t.rect(0.0, fh * 0.35, fw * 0.62, fh * 0.65, 2);
            t.rect(fw * 0.6, 0.0, fw, fh, 2);
            t.finish();
        }
        "mallet" => {
            t.rect(fw * 0.0, fh * 0.05, fw, fh * 0.4, 2);
            t.rect(fw * 0.35, fh * 0.4, fw * 0.65, fh, 2);
            t.finish();
        }
        "heart" => {
            // A paper heart: two round lobes over a point, filling its
            // box, a light fold down the left lobe.
            let r = fw * 0.25;
            t.ellipse(fw * 0.27, fh * 0.34, r, fh * 0.25, 2);
            t.ellipse(fw * 0.73, fh * 0.34, r, fh * 0.25, 2);
            for y in (fh * 0.36) as i32..h as i32 {
                let k2 = (y as f32 + 0.5 - fh * 0.36) / (fh * 0.62);
                let half = fw * 0.49 * (1.0 - k2);
                for x in (fw * 0.5 - half).round() as i32..(fw * 0.5 + half).round() as i32 {
                    t.set(x, y, 2);
                }
            }
            t.finish();
            t.ellipse(fw * 0.25, fh * 0.3, fw * 0.08, fh * 0.1, 3);
        }
        "gem" => {
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let dx = (x as f32 + 0.5 - fw / 2.0).abs() / (fw / 2.0);
                    let dy = (y as f32 + 0.5 - fh / 2.0).abs() / (fh / 2.0);
                    if dx + dy <= 0.95 {
                        t.set(x, y, 2);
                    }
                }
            }
            t.finish();
        }
        "star" => {
            let (cx, cy) = (fw / 2.0, fh / 2.0);
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let dx = (x as f32 + 0.5 - cx).abs();
                    let dy = (y as f32 + 0.5 - cy).abs();
                    if dx * dy < 3.0 && dx.max(dy) < cx.min(cy) {
                        t.set(x, y, 2);
                    }
                }
            }
            t.finish();
        }
        "arrow" => {
            for y in 0..h as i32 {
                let half = (fh / 2.0 - (y as f32 + 0.5 - fh / 2.0).abs()) as i32;
                for x in 1..(1 + half * 2).min(w as i32) {
                    t.set(x, y, 2);
                }
            }
            t.finish();
        }
        _ => {
            for y in (fh * 0.4) as i32..h as i32 {
                for x in 0..w as i32 {
                    if (x / 3 + y) % 2 == 0 {
                        t.set(x, y, 2);
                    }
                }
            }
            t.finish();
        }
    }
    t
}

/// The theme's own pixel font, drawn here for Game Printer (CC0-1.0, as
/// the theme is): a narrow face, most glyphs 4 dots wide (M, V, W, X and
/// Y take 5) and 7 tall, in rows of 5 so every glyph has one box. It
/// replaced, on 2026-11-03, a 5 by 7 face of the common LCD kind that
/// Rivet Rodeo's own pictures (scripts/rivet-rodeo/art.py) also drew,
/// so `gp theme check --original` refused Rivet Rodeo's paper font at
/// 0.967 against its own art. The digits score 0.38 against Donkey Kong's
/// lifted font and 0.49 against Rivet Rodeo's (refused at 0.75).
pub(crate) fn glyph(c: char) -> Option<[&'static str; 7]> {
    Some(match c {
        '0' => [
            ".##..", "#..#.", "#.##.", "##.#.", "#..#.", "#..#.", ".##..",
        ],
        '1' => [
            ".#...", "##...", ".#...", ".#...", ".#...", ".#...", "###..",
        ],
        '2' => [
            ".##..", "#..#.", "...#.", "..#..", ".#...", "#....", "####.",
        ],
        '3' => [
            "###..", "...#.", "...#.", ".##..", "...#.", "...#.", "###..",
        ],
        '4' => [
            "#..#.", "#..#.", "#..#.", "####.", "...#.", "...#.", "...#.",
        ],
        '5' => [
            "####.", "#....", "###..", "...#.", "...#.", "#..#.", ".##..",
        ],
        '6' => [
            ".##..", "#....", "#....", "###..", "#..#.", "#..#.", ".##..",
        ],
        '7' => [
            "####.", "...#.", "...#.", "..#..", "..#..", ".#...", ".#...",
        ],
        '8' => [
            ".##..", "#..#.", "#..#.", ".##..", "#..#.", "#..#.", ".##..",
        ],
        '9' => [
            ".##..", "#..#.", "#..#.", ".###.", "...#.", "...#.", ".##..",
        ],
        'A' => [
            ".##..", "#..#.", "#..#.", "####.", "#..#.", "#..#.", "#..#.",
        ],
        'B' => [
            "###..", "#..#.", "#..#.", "###..", "#..#.", "#..#.", "###..",
        ],
        'C' => [
            ".##..", "#..#.", "#....", "#....", "#....", "#..#.", ".##..",
        ],
        'D' => [
            "###..", "#..#.", "#..#.", "#..#.", "#..#.", "#..#.", "###..",
        ],
        'E' => [
            "####.", "#....", "#....", "###..", "#....", "#....", "####.",
        ],
        'F' => [
            "####.", "#....", "#....", "###..", "#....", "#....", "#....",
        ],
        'G' => [
            ".##..", "#..#.", "#....", "#.##.", "#..#.", "#..#.", ".###.",
        ],
        'H' => [
            "#..#.", "#..#.", "#..#.", "####.", "#..#.", "#..#.", "#..#.",
        ],
        'I' => [
            "###..", ".#...", ".#...", ".#...", ".#...", ".#...", "###..",
        ],
        'J' => [
            "..##.", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..",
        ],
        'K' => [
            "#..#.", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#..#.",
        ],
        'L' => [
            "#....", "#....", "#....", "#....", "#....", "#....", "####.",
        ],
        'M' => [
            "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#",
        ],
        'N' => [
            "#..#.", "##.#.", "##.#.", "#.##.", "#.##.", "#..#.", "#..#.",
        ],
        'O' => [
            ".##..", "#..#.", "#..#.", "#..#.", "#..#.", "#..#.", ".##..",
        ],
        'P' => [
            "###..", "#..#.", "#..#.", "###..", "#....", "#....", "#....",
        ],
        'Q' => [
            ".##..", "#..#.", "#..#.", "#..#.", "#..#.", "#.#..", ".#.#.",
        ],
        'R' => [
            "###..", "#..#.", "#..#.", "###..", "#.#..", "#..#.", "#..#.",
        ],
        'S' => [
            ".###.", "#....", "#....", ".##..", "...#.", "...#.", "###..",
        ],
        'T' => [
            "###..", ".#...", ".#...", ".#...", ".#...", ".#...", ".#...",
        ],
        'U' => [
            "#..#.", "#..#.", "#..#.", "#..#.", "#..#.", "#..#.", ".##..",
        ],
        'V' => [
            "#...#", "#...#", "#...#", "#...#", ".#.#.", ".#.#.", "..#..",
        ],
        'W' => [
            "#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#",
        ],
        'X' => [
            "#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#",
        ],
        'Y' => [
            "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..",
        ],
        'Z' => [
            "####.", "...#.", "...#.", "..#..", ".#...", "#....", "####.",
        ],
        '-' => [
            ".....", ".....", ".....", "###..", ".....", ".....", ".....",
        ],
        '=' => [
            ".....", ".....", "###..", ".....", "###..", ".....", ".....",
        ],
        '.' => [
            ".....", ".....", ".....", ".....", ".....", "##...", "##...",
        ],
        _ => return None,
    })
}

/// A glyph's width in dots: 3 for a space, else to its last inked column.
pub(crate) fn glyph_width(c: char) -> i32 {
    glyph(c).map_or(3, |g| {
        g.iter()
            .filter_map(|r| r.rfind('#'))
            .max()
            .map_or(0, |x| x as i32 + 1)
    })
}

/// A line's width in dots in the theme's font: each glyph's width and a
/// dot between glyphs.
pub(crate) fn text_width(text: &str) -> i32 {
    let w: i32 = text.chars().map(|c| glyph_width(c) + 1).sum();
    (w - 1).max(0)
}

/// A glyph in a cell's tones, the ink centered across the cell.
fn glyph_tones(c: char) -> Tones {
    let mut t = Tones::new(8, 8);
    if let Some(g) = glyph(c) {
        let x0 = (8 - glyph_width(c)) / 2;
        for (y, row) in g.iter().enumerate() {
            for (x, b) in row.bytes().enumerate() {
                if b == b'#' {
                    t.set(x as i32 + x0, y as i32, 1);
                }
            }
        }
    }
    t
}

/// A metatile of a level built from objects, 16 by 16, in tones by its
/// item role: ground as a kraft block, a platform as a strip, a block the
/// hero hits as a card with a mark, a pickup as a disc, a goal as a post,
/// water as waves, and scenery as a few paper specks, so no shape of the
/// game's scenery is kept. `k` is the metatile's number, for the grain.
fn object_tones(role: &str, k: u32) -> Tones {
    let mut t = Tones::new(16, 16);
    for y in 0..16i32 {
        for x in 0..16i32 {
            let v = match role {
                "terrain" | "solid" => {
                    if y == 0 || x == 0 {
                        3
                    } else if y == 15 || x == 15 || (x + y) % 8 == 0 {
                        1
                    } else {
                        2
                    }
                }
                "platform" => match y {
                    0 => 3,
                    1..=5 => 2,
                    6 => 1,
                    _ => 0,
                },
                "interactive" => {
                    let edge = y == 0 || x == 0 || y == 15 || x == 15;
                    let mark = (x - 8).abs() + (y - 8).abs() <= 3;
                    if edge {
                        1
                    } else if mark {
                        2
                    } else {
                        3
                    }
                }
                "collectible" | "power-up" => {
                    let d = (x * 2 - 15).pow(2) + (y * 2 - 15).pow(2);
                    if d <= 100 {
                        if x < 7 && y < 7 { 3 } else { 2 }
                    } else if d <= 144 {
                        1
                    } else {
                        0
                    }
                }
                "goal" => match x {
                    6 | 9 => 1,
                    7 | 8 => 3,
                    _ => 0,
                },
                "water" => {
                    let wave = ((x + (k as i32) * 3) % 8 - 4).abs() / 2;
                    if y % 6 == wave { 3 } else { 0 }
                }
                "background"
                    if hash(x as u32 / 2, y as u32 / 2, k.wrapping_mul(31)).is_multiple_of(23) =>
                {
                    3
                }
                _ => 0,
            };
            t.set(x, y, v);
        }
    }
    t
}

/// Draws text in the theme's font into a picture, `scale` pixels a dot,
/// with a paper shadow.
fn text_into(
    img: &mut Rgba,
    x0: i32,
    y0: i32,
    text: &str,
    scale: i32,
    fill: [u8; 3],
    shadow: Option<[u8; 3]>,
) {
    let mut next = x0;
    for c in text.chars() {
        let gx = next;
        next += (glyph_width(c) + 1) * scale;
        let Some(g) = glyph(c) else { continue };
        for pass in 0..2 {
            let (off, col) = match (pass, shadow) {
                (0, Some(s)) => (scale.max(1), s),
                (0, None) => continue,
                _ => (0, fill),
            };
            for (y, row) in g.iter().enumerate() {
                for (x, b) in row.bytes().enumerate() {
                    if b != b'#' {
                        continue;
                    }
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = gx + x as i32 * scale + dx + off;
                            let py = y0 + y as i32 * scale + dy + off;
                            if px >= 0 && py >= 0 {
                                img.set(px as u32, py as u32, [col[0], col[1], col[2], 255]);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn hash(a: u32, b: u32, seed: u32) -> u32 {
    let mut h =
        a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

fn rgb(c: [u8; 3]) -> [u8; 4] {
    [c[0], c[1], c[2], 255]
}

/// A stage on paper: cream stock, the ground's boxes as folded kraft
/// strips and the ladders as paper rungs, from the compiled level, and the
/// other cells the game draws as paper chips.
fn level_picture(
    size: [u32; 2],
    level: Option<&gp_sim::spec::Level>,
    cells: Option<&(Vec<String>, Vec<String>)>,
    seed: u32,
) -> Rgba {
    let mut img = Rgba::new(size[0], size[1]);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let c = if hash(x / 2, y / 2, seed).is_multiple_of(7) {
                CREAM2
            } else {
                CREAM
            };
            img.set(x, y, rgb(c));
        }
    }
    let mut covered = vec![false; (size[0] * size[1]) as usize];
    // The ground's own rows (its 8 rows from the floor line, not the
    // shadow): a cell they reach at all is a girder cell.
    let mut ground = vec![false; (size[0] * size[1]) as usize];
    let kraft = [[0x6e, 0x4a, 0x2a], [0xb7, 0x83, 0x4f], [0xd9, 0xab, 0x78]];
    let rung = [[0x3e, 0x5c, 0x70], [0x7a, 0xa0, 0xb8], [0xb4, 0xd0, 0xe2]];
    if let Some(l) = level {
        for b in &l.ladders {
            let [x0, y0, x1, y1] = b.map(|v| v as i32);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    if x < 0 || y < 0 || x >= size[0] as i32 || y >= size[1] as i32 {
                        continue;
                    }
                    let rail = x <= x0 + 1 || x >= x1 - 1;
                    let step = (y - y0) % 4 == 2;
                    let c = if rail {
                        if x == x0 || x == x1 { rung[0] } else { rung[1] }
                    } else if step {
                        rung[2]
                    } else {
                        continue;
                    };
                    img.set(x as u32, y as u32, rgb(c));
                    covered[(y as u32 * size[0] + x as u32) as usize] = true;
                }
            }
        }
        for b in &l.ground {
            let [x0, y0, x1, _] = b.map(|v| v as i32);
            for x in x0..=x1 {
                for dy in 0..9 {
                    let y = y0 + dy;
                    if x < 0 || y < 0 || x >= size[0] as i32 || y >= size[1] as i32 {
                        continue;
                    }
                    let c = match dy {
                        0 => kraft[2],
                        7 => kraft[0],
                        8 => SHADOW,
                        _ if (x + dy) % 8 == 0 => kraft[0],
                        _ => kraft[1],
                    };
                    img.set(x as u32, y as u32, rgb(c));
                    let i = (y as u32 * size[0] + x as u32) as usize;
                    covered[i] = true;
                    ground[i] |= dy < 8;
                }
            }
        }
    }
    // The other cells the game draws, by their shape: a block of two by
    // two in a palette none of the boxes' cells use is a thing on its
    // own (Donkey Kong's prizes on 50m, 75m and 100m), a paper gem; a
    // block two by two or more is a paper card (a stack of things, a
    // drum, or a thing in the ground's colors), a row of three
    // or more a strip of the ground, a column of two or more a ladder (a
    // broken one, or one to where play doesn't reach). A lone cell is left
    // out: beside the ground or a ladder it is a piece the boxes leave out
    // (a slope's step, a ladder's end), and on its own a stray chip.
    if let Some((text, palettes)) = cells {
        let rows = text.len();
        let cols = text.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let cov = |of: &[bool], c: usize, r: usize| -> usize {
            let (x0, y0) = (c as u32 * 8, r as u32 * 8);
            (0..64u32)
                .filter(|i| {
                    of.get(((y0 + i / 8) * size[0] + x0 + i % 8) as usize)
                        .copied()
                        .unwrap_or(true)
                })
                .count()
        };
        let drawn: Vec<Vec<bool>> = text
            .iter()
            .enumerate()
            .map(|(r, l)| {
                let mut v: Vec<bool> = l.chars().map(|ch| ch != ' ').collect();
                v.resize(cols, false);
                for (c, d) in v.iter_mut().enumerate() {
                    // A cell the boxes draw over is theirs. A girder
                    // cell is, however little of it the girder reaches:
                    // a sloped girder whose floor line is the last row of
                    // its cell shows one row there, and drawn as a strip
                    // that cell stood a girder 7 px over the floor, a
                    // ledge the hero walked through (Rivet Rodeo's paper,
                    // 2026-11-04).
                    if *d && (cov(&covered, c, r) > 8 || cov(&ground, c, r) > 0) {
                        *d = false;
                    }
                }
                v
            })
            .collect();
        // The palettes of the cells the boxes draw.
        let palette = |c: usize, r: usize| palettes.get(r).and_then(|l| l.chars().nth(c));
        let boxes_palettes: BTreeSet<char> = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (c, r)))
            .filter(|&(c, r)| cov(&covered, c, r) > 8 || cov(&ground, c, r) > 0)
            .filter_map(|(c, r)| palette(c, r))
            .collect();
        let mut left = drawn;
        // A thing on its own: the drawn cells of a palette block (two by
        // two cells, aligned, as a console's attributes give a palette)
        // in a palette none of the boxes' cells use, two or more of them,
        // with no drawn cell of that palette just outside it. Donkey Kong's
        // prizes on 50m, 75m and 100m; a ladder in such a palette runs on
        // out of its block.
        let gem = [[0x6e, 0x52, 0x10], [0xd8, 0xa8, 0x28], [0xf8, 0xe0, 0x90]];
        for r0 in (0..rows).step_by(2) {
            for c0 in (0..cols).step_by(2) {
                let inside =
                    |c: usize, r: usize| (c0..c0 + 2).contains(&c) && (r0..r0 + 2).contains(&r);
                let Some(p) = palette(c0, r0) else { continue };
                if boxes_palettes.contains(&p) {
                    continue;
                }
                let mine: Vec<(usize, usize)> = (r0..(r0 + 2).min(rows))
                    .flat_map(|r| (c0..(c0 + 2).min(cols)).map(move |c| (c, r)))
                    .filter(|&(c, r)| left[r][c] && palette(c, r) == Some(p))
                    .collect();
                if mine.len() < 2 {
                    continue;
                }
                let runs_on = (r0.saturating_sub(1)..(r0 + 3).min(rows))
                    .flat_map(|r| (c0.saturating_sub(1)..(c0 + 3).min(cols)).map(move |c| (c, r)))
                    .any(|(c, r)| !inside(c, r) && left[r][c] && palette(c, r) == Some(p));
                if runs_on {
                    continue;
                }
                for &(c, r) in &mine {
                    left[r][c] = false;
                }
                // A gem standing on the block's bottom row.
                let (x0, y0) = (c0 as u32 * 8, r0 as u32 * 8);
                let (cx, cy, rad) = (x0 as f32 + 8.0, y0 as f32 + 9.0, 6.5f32);
                for y in y0..y0 + 16 {
                    for x in x0..x0 + 16 {
                        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                        let d = dx.abs() + dy.abs() * 1.15;
                        if d > rad {
                            continue;
                        }
                        let c = if d > rad - 1.2 {
                            gem[0]
                        } else if dx < 0.0 && dy < 0.0 && dx.abs() + dy.abs() < rad - 2.5 {
                            gem[2]
                        } else {
                            gem[1]
                        };
                        img.set(x, y, rgb(c));
                    }
                }
            }
        }
        let take = |c0: usize, r0: usize, w: usize, h: usize, left: &mut Vec<Vec<bool>>| {
            for row in left.iter_mut().skip(r0).take(h) {
                for cell in row.iter_mut().skip(c0).take(w) {
                    *cell = false;
                }
            }
        };
        let card = [[0x8a, 0x76, 0x58], [0xd8, 0xc7, 0xa0], [0xec, 0xdf, 0xc0]];
        // Blocks first: each widest run of drawn cells, as deep as every
        // cell under it is drawn, at least two by two.
        for r0 in 0..rows {
            for c0 in 0..cols {
                if !left[r0][c0] {
                    continue;
                }
                let w = (c0..cols).take_while(|&c| left[r0][c]).count();
                let h = (r0..rows)
                    .take_while(|&r| (c0..c0 + w).all(|c| left[r][c]))
                    .count();
                if w < 2 || h < 2 {
                    continue;
                }
                take(c0, r0, w, h, &mut left);
                let (x0, y0) = (c0 as u32 * 8, r0 as u32 * 8);
                let (x1, y1) = (x0 + w as u32 * 8 - 1, y0 + h as u32 * 8 - 1);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let c = if x == x1 || y == y1 {
                            SHADOW
                        } else if x == x0 || y == y0 || x + 1 == x1 || y + 1 == y1 {
                            card[0]
                        } else if (x + y) % 6 == 0 {
                            card[2]
                        } else {
                            card[1]
                        };
                        img.set(x, y, rgb(c));
                    }
                }
            }
        }
        // Then rows of three or more: strips of the ground.
        for r0 in 0..rows {
            let mut c0 = 0;
            while c0 < cols {
                let w = (c0..cols).take_while(|&c| left[r0][c]).count();
                if w >= 3 {
                    take(c0, r0, w, 1, &mut left);
                    let (x0, y0) = (c0 as u32 * 8, r0 as u32 * 8);
                    for x in x0..x0 + w as u32 * 8 {
                        for dy in 0..8u32 {
                            let c = match dy {
                                0 => kraft[2],
                                7 => kraft[0],
                                _ if (x + dy) % 8 == 0 => kraft[0],
                                _ => kraft[1],
                            };
                            img.set(x, y0 + dy, rgb(c));
                        }
                    }
                }
                c0 += w.max(1);
            }
        }
        // Then columns of two or more: ladders.
        for c0 in 0..cols {
            let mut r0 = 0;
            while r0 < rows {
                let h = (r0..rows).take_while(|&r| left[r][c0]).count();
                if h >= 2 {
                    take(c0, r0, 1, h, &mut left);
                    let (x0, y0) = (c0 as u32 * 8, r0 as u32 * 8);
                    let (x1, y1) = (x0 + 7, y0 + h as u32 * 8 - 1);
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let c = if x <= x0 + 1 || x >= x1 - 1 {
                                if x == x0 || x == x1 { rung[0] } else { rung[1] }
                            } else if (y - y0) % 4 == 2 {
                                rung[2]
                            } else {
                                continue;
                            };
                            img.set(x, y, rgb(c));
                        }
                    }
                }
                r0 += h.max(1);
            }
        }
        // What is left (a lone cell or two) is a piece of the ground or a
        // ladder the boxes leave out, or a stray: left out.
    }
    img
}

/// A number the HUD draws over its fixed part: its first cell, its width
/// in cells and its counter's name.
pub(crate) struct HudNumber {
    pub col: u32,
    pub row: u32,
    pub width: u32,
    pub counter: String,
}

/// The HUD's numbers, from the model's `present.hud` fields, one per cell
/// ([`one_per_cell`]).
pub(crate) fn hud_numbers(m: &Model) -> Vec<HudNumber> {
    m.present
        .as_ref()
        .and_then(|p| p.hud.as_ref())
        .map(|h| {
            one_per_cell(
                h.field
                    .iter()
                    .filter_map(|f| {
                        Some((
                            HudNumber {
                                col: f.at[0],
                                row: f.at[1],
                                width: f.digits?,
                                counter: f.counter.clone()?,
                            },
                            !f.when.is_empty(),
                        ))
                    })
                    .collect(),
            )
        })
        .unwrap_or_default()
}

/// The numbers (each with whether its field is drawn only under a `when`)
/// with one per first cell: a field drawn always there before one drawn
/// only sometimes, else the first. Original Edition draws its bonus at
/// [23, 5] as `bonus`, and as `hud_bonus` while `hud_stale` is 1: both
/// labelled, the second's "H" was painted over the first's "BONUS".
fn one_per_cell(fields: Vec<(HudNumber, bool)>) -> Vec<HudNumber> {
    let mut keep: Vec<(HudNumber, bool)> = Vec::new();
    for (n, when) in fields {
        match keep
            .iter_mut()
            .find(|(k, _)| k.col == n.col && k.row == n.row)
        {
            Some(k) if k.1 && !when => *k = (n, when),
            Some(_) => {}
            None => keep.push((n, when)),
        }
    }
    keep.into_iter().map(|(n, _)| n).collect()
}

/// A label: the column of its number, the number's width, the row and the text.
type HudLabel = (u32, u32, u32, String);

/// The labels a box of numbers in one row gets, set apart as Donkey
/// Kong's status box sets its M, BONUS and L apart: each over its number,
/// with a line between neighbours. A row is such a box when two or more of
/// its numbers each follow a one-letter label on their own row (the labels
/// and numbers then run together, "L2B4900N1"; the letter moves up), or
/// when a number has the game's own marks over all of it (a word drawn as
/// a picture, Donkey Kong's BONUS). A letter is spelled out as its
/// counter's name when the name starts with it, is a plain word and fits
/// over the number. Returns the labels (column of the number, its width,
/// row, text) and the rows that are boxes, and blanks the moved letters
/// in `grid`.
fn hud_labels(grid: &mut [Vec<char>], numbers: &[HudNumber]) -> (Vec<HudLabel>, BTreeSet<u32>) {
    let at = |g: &[Vec<char>], c: i64, r: i64| -> char {
        if c < 0 || r < 0 {
            return ' ';
        }
        g.get(r as usize)
            .and_then(|l| l.get(c as usize))
            .copied()
            .unwrap_or(' ')
    };
    let spell = |letter: char, n: &HudNumber| -> String {
        let name = n.counter.to_ascii_uppercase();
        let fits = text_width(&name) <= n.width as i32 * 8;
        if name.starts_with(letter)
            && name
                .chars()
                .all(|c| c.is_ascii_alphabetic() && glyph(c).is_some())
            && fits
        {
            name
        } else {
            letter.to_string()
        }
    };
    let mut labels = Vec::new();
    let mut boxes = BTreeSet::new();
    let rows: BTreeSet<u32> = numbers.iter().map(|n| n.row).collect();
    for r in rows {
        let row: Vec<&HudNumber> = numbers.iter().filter(|n| n.row == r).collect();
        let letter = |g: &[Vec<char>], n: &HudNumber| {
            let c = at(g, i64::from(n.col) - 1, i64::from(r));
            let before = at(g, i64::from(n.col) - 2, i64::from(r));
            (c.is_ascii_uppercase() && !before.is_ascii_alphanumeric()).then_some(c)
        };
        let run_together = row.iter().filter(|n| letter(grid, n).is_some()).count() >= 2;
        for n in &row {
            if run_together
                && r > 0
                && let Some(c) = letter(grid, n)
            {
                grid[r as usize][n.col as usize - 1] = ' ';
                labels.push((n.col, n.width, r - 1, spell(c, n)));
                boxes.insert(r);
                continue;
            }
            let marks = r > 0
                && (0..n.width).all(|k| at(grid, i64::from(n.col + k), i64::from(r) - 1) == '?');
            if marks {
                labels.push((
                    n.col,
                    n.width,
                    r - 1,
                    spell(
                        n.counter.to_ascii_uppercase().chars().next().unwrap_or(' '),
                        n,
                    ),
                ));
                boxes.insert(r);
            }
        }
    }
    (labels, boxes)
}

/// The score display's fixed part: its words in the theme's font where
/// the game's font reads them. The cells it can't read are the game's own
/// marks: a group of them around other cells (a box the game frames a
/// number with) becomes a thin paper frame, and a label (a group in one
/// row) is left out, since the theme has no words of the game's. A box of
/// numbers gets its labels set apart ([`hud_labels`]).
fn hud_picture(
    size: [u32; 2],
    cells: &(Vec<String>, Vec<String>),
    title: &str,
    numbers: &[HudNumber],
) -> Rgba {
    let mut img = Rgba::new(size[0], size[1]);
    let tape = [[0x8a, 0x76, 0x58]];
    let mut grid: Vec<Vec<char>> = cells.0.iter().map(|l| l.chars().collect()).collect();
    let (labels, boxes) = hud_labels(&mut grid, numbers);
    let rows = grid.len();
    let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    let at = |c: usize, r: usize| grid.get(r).and_then(|l| l.get(c)).copied().unwrap_or(' ');
    let mut seen = vec![vec![false; cols]; rows];
    for r0 in 0..rows {
        for c0 in 0..cols {
            if at(c0, r0) != '?' || seen[r0][c0] {
                continue;
            }
            let mut group = vec![(c0, r0)];
            seen[r0][c0] = true;
            let mut k = 0;
            while k < group.len() {
                let (c, r) = group[k];
                k += 1;
                for (dc, dr) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (cc, rr) = (c as i32 + dc, r as i32 + dr);
                    if cc < 0 || rr < 0 || cc as usize >= cols || rr as usize >= rows {
                        continue;
                    }
                    let (cc, rr) = (cc as usize, rr as usize);
                    if at(cc, rr) == '?' && !seen[rr][cc] {
                        seen[rr][cc] = true;
                        group.push((cc, rr));
                    }
                }
            }
            let cmin = group.iter().map(|g| g.0).min().unwrap_or(0) as u32;
            let cmax = group.iter().map(|g| g.0).max().unwrap_or(0) as u32;
            let rmin = group.iter().map(|g| g.1).min().unwrap_or(0) as u32;
            let rmax = group.iter().map(|g| g.1).max().unwrap_or(0) as u32;
            if rmin == rmax {
                continue;
            }
            // A frame: a line round the group's box, half a cell in.
            let (x0, y0, x1, y1) = (cmin * 8 + 3, rmin * 8 + 3, cmax * 8 + 4, rmax * 8 + 7);
            for x in x0..=x1 {
                img.set(x, y0, rgb(tape[0]));
                img.set(x, (y1).min(size[1] - 1), rgb(tape[0]));
            }
            for y in y0..=y1.min(size[1] - 1) {
                img.set(x0, y, rgb(tape[0]));
                img.set(x1, y, rgb(tape[0]));
            }
        }
    }
    // A word of the game's title is a name (a hero's, on Super Mario
    // Bros.' score line): left out, as the kit leaves names out.
    let names: Vec<String> = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_ascii_uppercase)
        .collect();
    for (r, line) in grid.iter().enumerate() {
        let mut c = 0;
        while c < line.len() {
            let w = line[c..]
                .iter()
                .take_while(|ch| ch.is_ascii_alphanumeric())
                .count();
            if w > 0 {
                let word: String = line[c..c + w].iter().collect();
                if !names.contains(&word) {
                    // Set in the theme's font, centered on its cells.
                    let x = c as i32 * 8 + (w as i32 * 8 - text_width(&word)) / 2;
                    text_into(&mut img, x, r as i32 * 8, &word, 1, INK, None);
                }
                c += w;
            } else {
                let ch = line[c];
                if ch != ' ' && ch != '?' {
                    text_into(
                        &mut img,
                        c as i32 * 8 + (8 - glyph_width(ch)) / 2,
                        r as i32 * 8,
                        &ch.to_string(),
                        1,
                        INK,
                        None,
                    );
                }
                c += 1;
            }
        }
    }
    // The boxes' labels, each centred over its number, a line between
    // neighbours and a frame round the box.
    let line = |img: &mut Rgba, x0: u32, y0: u32, x1: u32, y1: u32| {
        for y in y0..=y1.min(size[1].saturating_sub(1)) {
            for x in x0..=x1.min(size[0].saturating_sub(1)) {
                img.set(x, y, rgb(tape[0]));
            }
        }
    };
    for &r in &boxes {
        let mut row: Vec<&HudNumber> = numbers.iter().filter(|n| n.row == r).collect();
        row.sort_by_key(|n| n.col);
        let (Some(first), Some(last)) = (row.first(), row.last()) else {
            continue;
        };
        let (c0, c1) = (first.col.saturating_sub(1), last.col + last.width);
        let (y0, y1) = ((r - 1) * 8, r * 8 + 7);
        line(&mut img, c0 * 8 + 3, y0, c1 * 8 + 4, y0);
        line(&mut img, c0 * 8 + 3, y1, c1 * 8 + 4, y1);
        line(&mut img, c0 * 8 + 3, y0, c0 * 8 + 3, y1);
        line(&mut img, c1 * 8 + 4, y0, c1 * 8 + 4, y1);
        for w in row.windows(2) {
            let gap = w[0].col + w[0].width;
            if gap < w[1].col {
                let x = (gap * 8 + w[1].col * 8) / 2;
                line(&mut img, x, y0 + 2, x, y1 - 2);
            }
        }
    }
    for (col, width, r, text) in &labels {
        let wide = text_width(text);
        let x = *col as i32 * 8 + (*width as i32 * 8 - wide) / 2;
        // Clear the cells under it first: the game's marks there are the
        // word it draws as a picture.
        for y in *r * 8..*r * 8 + 8 {
            for xx in (*col * 8)..(*col + *width) * 8 {
                if xx < size[0] && y < size[1] {
                    img.set(xx, y, [0, 0, 0, 0]);
                }
            }
        }
        text_into(&mut img, x, *r as i32 * 8 + 1, text, 1, INK, None);
    }
    img
}

/// A whole screen (a title and its menu) on paper: the theme's own title
/// word big where the game's picture was, the menu lines the game's font
/// reads in the theme's font, and no line the font can't read (a logo, a
/// maker's line). The menu is the longest run of read lines evenly
/// spaced; any other line the font reads is the game's own words (a
/// maker's or a country's line) and gives way to the theme's `credit`,
/// on the lowest such line.
fn screen_picture(
    size: [u32; 2],
    cells: &(Vec<String>, Vec<String>),
    logo: &str,
    credit: &str,
    seed: u32,
) -> Rgba {
    let mut img = level_picture(size, None, None, seed);
    // Bunting along the top: paper triangles.
    let flags = [
        paper_tones("player")[1],
        paper_tones("npc")[1],
        paper_tones("power-up")[1],
        paper_tones("enemy")[1],
    ];
    for i in 0..(size[0] / 16) {
        let col = flags[(i % 4) as usize];
        for y in 0..10u32 {
            for x in 0..16u32 {
                let half = 8.0 - y as f32 * 0.8;
                if (x as f32 - 8.0).abs() < half {
                    img.set(i * 16 + x, 6 + y, rgb(col));
                }
            }
        }
    }
    let rows: Vec<&String> = cells.0.iter().collect();
    let unread: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains('?'))
        .map(|(i, _)| i)
        .collect();
    let top: Vec<usize> = unread
        .iter()
        .copied()
        .filter(|r| *r < rows.len() * 3 / 5)
        .collect();
    let (a, b) = (
        top.first().copied().unwrap_or(4),
        top.last().copied().unwrap_or(10),
    );
    // The title in the theme's own font at twice the menu's size,
    // one color with a one-pixel shadow: a bigger scale or a shadow as
    // deep as a dot read as blocks (the user on Rivet Rodeo's page,
    // 2026-11-03).
    let scale = 2;
    let lw = text_width(logo) * scale;
    let words: Vec<&str> = logo.split(' ').collect();
    let lines: Vec<String> = if lw > size[0] as i32 - 16 && words.len() > 1 {
        let half = words.len().div_ceil(2);
        vec![words[..half].join(" "), words[half..].join(" ")]
    } else {
        vec![logo.to_owned()]
    };
    let mid = ((a + b + 1) * 8 / 2) as i32;
    let total = lines.len() as i32 * 8 * scale;
    for (i, l) in lines.iter().enumerate() {
        let w = text_width(l) * scale;
        let x = (size[0] as i32 - w) / 2;
        let y = mid - total / 2 + i as i32 * 8 * scale;
        text_into(&mut img, x + 1, y + 1, l, scale, SHADOW, None);
        text_into(&mut img, x, y, l, scale, INK, None);
    }
    let read: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.contains('?') && !l.trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    // The menu: the longest run of read lines with one spacing.
    let mut menu: (usize, usize) = (0, read.len().min(1));
    let mut i = 0;
    while i < read.len() {
        let mut j = i + 1;
        if j < read.len() {
            let gap = read[j] - read[i];
            while j + 1 < read.len() && read[j + 1] - read[j] == gap {
                j += 1;
            }
            j += 1;
        }
        if j - i > menu.1 - menu.0 {
            menu = (i, j);
        }
        i = j.max(i + 1);
    }
    let in_menu: Vec<usize> = read[menu.0..menu.1].to_vec();
    let others: Vec<usize> = read
        .iter()
        .copied()
        .filter(|r| !in_menu.contains(r))
        .collect();
    if let Some(&r) = others.last()
        && !credit.is_empty()
    {
        let w = text_width(credit);
        let x = ((size[0] as i32 - w) / 2).max(0);
        text_into(&mut img, x, r as i32 * 8, credit, 1, INK, None);
    }
    for (r, line) in rows.iter().enumerate() {
        if !in_menu.contains(&r) {
            continue;
        }
        // The line set in the theme's font, centered on the cells the
        // game's font drew it in.
        let (Some(first), Some(last)) = (line.find(|c| c != ' '), line.rfind(|c| c != ' ')) else {
            continue;
        };
        let text = &line[first..=last];
        let span = (last - first + 1) as i32 * 8;
        let x = first as i32 * 8 + (span - text_width(text)) / 2;
        text_into(&mut img, x, r as i32 * 8, text, 1, INK, None);
    }
    img
}

/// A tune of the theme's own for a sound role, from its kind, name and
/// description words. Melodies come from a pentatonic scale picked by a
/// hash of the role's name, so no two roles share one and none is any
/// game's.
pub fn tune_for(role: &str, music: bool, loops: bool, desc: &str, frames: Option<u32>) -> String {
    let d = format!("{role} {}", desc.to_lowercase());
    let has = |w: &str| d.contains(w);
    let h = role
        .bytes()
        .fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(b)));
    let roots = ["C", "D", "F", "G", "A"];
    let root = roots[(h % 5) as usize];
    let scale = |i: i32, oct: i32| -> String {
        let steps = [0, 2, 4, 7, 9];
        let n = i.rem_euclid(5);
        let o = oct + i.div_euclid(5);
        let base = gp_theme::tune::note_number(&format!("{root}4")).unwrap_or(60) as i32;
        name(base - 60 + 12 * (o - 4) + 60 + steps[n as usize])
    };
    let mut s = format!("tune 1\n# {role}: written by gp theme paint, from the role's words\n");
    if music {
        let len = if loops {
            32
        } else if has("victory") || has("clear") {
            20
        } else if has("title") {
            24
        } else {
            12
        };
        let step = if has("fast") || has("urgent") { 4 } else { 7 };
        s.push_str(&format!("step {step}\n"));
        if loops {
            s.push_str("loop\n");
        }
        s.push_str("voice lead pluck 0.45\nnotes");
        let mut i = (h % 3) as i32;
        let mut x = h;
        for k in 0..len {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let mv = [(-1), 1, 2, -2, 0, 1, 3, -1][((x >> 16) % 8) as usize];
            i = (i + mv).clamp(0, 9);
            if !loops && k == len - 1 {
                i = if has("lost") || has("pause") { 0 } else { 5 };
            }
            let dur = if k % 4 == 3 { 2 } else { 1 };
            s.push_str(&format!(" {}:{dur}", scale(i, 4)));
        }
        s.push_str("\nvoice bass soft 0.35\nnotes");
        for k in 0..(len / 4).max(1) {
            let r = if k % 2 == 0 { 0 } else { 3 };
            s.push_str(&format!(" {}:5", scale(r, 2)));
        }
        s.push('\n');
        return s;
    }
    let f = frames.unwrap_or(12).clamp(2, 90);
    if has("walk") || has("step") || has("tick") {
        s.push_str(&format!(
            "step 2\nvoice tick noise 0.25\nnotes {}:1\n",
            scale(4, 6)
        ));
    } else if has("jump over") || has("score") || has("jingle") {
        s.push_str(&format!(
            "step 3\nvoice chime pluck 0.5\nnotes {}:1 {}:1 {}:2\n",
            scale(2, 5),
            scale(4, 5),
            scale(5, 5)
        ));
    } else if has("jump") {
        s.push_str(&format!(
            "step 2\nvoice hop square 0.35\nnotes {}>{}:4\n",
            scale(0, 4),
            scale(5, 4)
        ));
    } else if has("smash") || has("break") {
        s.push_str(&format!(
            "step 2\nvoice crunch noise 0.5\nnotes {}:3\nvoice knock thud 0.6\nnotes {}:4\n",
            scale(0, 5),
            scale(0, 2)
        ));
    } else if has("lost") || has("death") || has("falling") {
        s.push_str(&format!(
            "step 4\nvoice fall triangle 0.5\nnotes {}>{}:{}\n",
            scale(5, 5),
            scale(0, 3),
            (f / 4).max(4)
        ));
    } else if has("thump") || has("beat") || has("drum") {
        s.push_str(&format!(
            "step 2\nvoice thud thud 0.7\nnotes {}:4\n",
            scale(0, 2)
        ));
    } else {
        s.push_str(&format!(
            "step 2\nvoice blip pulse 0.4\nnotes {}:2 {}:2\n",
            scale(2, 5),
            scale(4, 5)
        ));
    }
    s
}

fn name(n: i32) -> String {
    const N: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", N[n.rem_euclid(12) as usize], n.div_euclid(12) - 1)
}

/// An RGBA picture as `.pix` text, its colors given characters in order
/// of first use.
pub fn rgba_pix(img: &Rgba, note: &str) -> Result<String> {
    const CHARS: &str =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789@$%&*+=?!~^<>/|;:,_";
    let mut colors: Vec<[u8; 3]> = Vec::new();
    let mut rows = Vec::new();
    for y in 0..img.height {
        let mut r = String::new();
        for x in 0..img.width {
            let p = img.get(x, y);
            if p[3] < 128 {
                r.push('.');
                continue;
            }
            let c = [p[0], p[1], p[2]];
            let i = match colors.iter().position(|k| *k == c) {
                Some(i) => i,
                None => {
                    colors.push(c);
                    colors.len() - 1
                }
            };
            r.push(
                CHARS
                    .chars()
                    .nth(i)
                    .context("more colors than a .pix holds (78)")?,
            );
        }
        rows.push(r);
    }
    let mut s = format!(
        "pix 1\n# {note}\nsize {} {}\n. transparent\n",
        img.width, img.height
    );
    for (i, c) in colors.iter().enumerate() {
        s.push_str(&format!(
            "{} {}\n",
            CHARS.chars().nth(i).unwrap_or('?'),
            Rgb(*c)
        ));
    }
    for r in rows {
        s.push_str(&r);
        s.push('\n');
    }
    Ok(s)
}

/// A theme being written: its files, restyles, rules and sounds.
pub struct Out {
    pub restyles: Vec<Restyle>,
    pub palette: Vec<PaletteRule>,
    pub sound: Vec<SoundReplace>,
    pub notes: Vec<String>,
    /// Whether the levels are built from objects: their sprites are many
    /// and drawn from tables the kit can't cut whole, so each entity role
    /// also gets a sprite of its own.
    pub objects: bool,
}

pub(crate) fn write_file(dir: &Path, file: &str, text: &str) -> Result<String> {
    fs::write(dir.join(file), text).with_context(|| format!("writing {file}"))?;
    Ok(sha256_hex(text.as_bytes()))
}

pub(crate) fn restyle(
    slot: Option<&str>,
    entry: Option<&str>,
    file: &str,
    sha256: String,
    generator: &str,
    origin: &str,
) -> Restyle {
    Restyle {
        target: None,
        slot: slot.map(str::to_owned),
        entry: entry.map(str::to_owned),
        file: file.into(),
        origin: origin.into(),
        license: "CC0-1.0".into(),
        sha256,
        generator: Some(generator.into()),
        author: None,
        note: None,
    }
}

/// The rules that color the tiles' tones: each console color a role's
/// tone was cut with becomes that role's paper tone, and the backdrop
/// cream.
pub(crate) fn tone_rules(
    kit: &Kit,
    local: &Local,
    tones_of: &dyn Fn(&str) -> [[u8; 3]; 3],
) -> Vec<PaletteRule> {
    let nes = gp_look::present::nes_colors();
    let mut by_role: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    // A slot drawn as another character ([`look_of`]) first: a color it
    // shares with another slot of its role (the thrower's colors, which
    // the stage-clear scene gives the sprites it goes up in) takes the
    // character's tone.
    let mut order: Vec<&ArtSlot> = kit.art.iter().collect();
    order.sort_by_key(|a| look_of(kit, local, a).is_none());
    for a in order {
        let Some(cuts) = local.cuts.get(&a.id) else {
            continue;
        };
        let role = match a.role.as_str() {
            "effect" => "*",
            r => r,
        };
        // A slot drawn as another character ([`look_of`]) takes that
        // character's tones, under its own role's rule.
        let paper = tones_of(look_of(kit, local, a).map_or(role, |b| b.role.as_str()));
        for c in cuts {
            for p in &c.places {
                let idx = tone_index(p.colors);
                for (t, i) in idx.iter().enumerate() {
                    let n = p.colors[usize::from(*i)];
                    let from = Rgb(nes[usize::from(n & 0x3F)]).to_string();
                    by_role
                        .entry(role.to_owned())
                        .or_default()
                        .entry(from)
                        .or_insert_with(|| Rgb(paper[t]).to_string());
                }
            }
        }
    }
    let mut out: Vec<PaletteRule> = by_role
        .into_iter()
        .map(|(role, m)| PaletteRule {
            // A color the cuts never showed (a palette the game swaps in
            // during play) gets the role's tone by its lightness, so no
            // sprite shows the original's colors.
            ramp: if role == "*" {
                Vec::new()
            } else {
                tones_of(&role)
                    .iter()
                    .map(|c| Rgb(*c).to_string())
                    .collect()
            },
            target: role,
            replace: m.into_iter().map(|(a, b)| [a, b]).collect(),
            ..PaletteRule::default()
        })
        .collect();
    // A level built from objects: each item role's colors, in whatever
    // palette an area gives them, by their lightness onto the role's
    // paper (a ramp), where no sprite color of the role is swapped.
    let roles: BTreeSet<&str> = local.objects.iter().map(|(r, _)| r.as_str()).collect();
    for role in roles {
        let paper = match role {
            "sky" => continue,
            "terrain" | "solid" | "platform" => KRAFT,
            "interactive" => [[0x8a, 0x76, 0x58], [0xd8, 0xc7, 0xa0], [0xec, 0xdf, 0xc0]],
            "background" => [CREAM2, CREAM2, [0xfa, 0xf4, 0xe4]],
            r => tones_of(r),
        };
        let ramp: Vec<String> = paper.iter().map(|c| Rgb(*c).to_string()).collect();
        match out.iter_mut().find(|r| r.target == role) {
            Some(r) => r.ramp = ramp,
            None => out.push(PaletteRule {
                target: role.to_owned(),
                ramp,
                ..PaletteRule::default()
            }),
        }
    }
    // The cells a single-tile level changes to ground as it runs, drawn in
    // role `terrain`: the stage picture's kraft, by lightness.
    if !local.kind_cells.is_empty() && !out.iter().any(|r| r.target == "terrain") {
        out.push(PaletteRule {
            target: "terrain".into(),
            ramp: KRAFT.iter().map(|c| Rgb(*c).to_string()).collect(),
            ..PaletteRule::default()
        });
    }
    out.push(PaletteRule {
        target: "sky".into(),
        tint: Some(Rgb(CREAM).to_string()),
        tint_amount: Some(1.0),
        ..PaletteRule::default()
    });
    out
}

/// The paper theme for a game drawn from tiles: banks, pictures, rules
/// and tunes, into `dir`. `spec` gives the levels' boxes.
// Each argument is one input the caller already holds; the command's own
// plumbing, with no group of them that belongs together.
#[allow(clippy::too_many_arguments)]
pub fn paint_tiles(
    m: &Model,
    kit: &Kit,
    local: &Local,
    spec: &gp_sim::Spec,
    dir: &Path,
    generator: &str,
    seed: u32,
    logo: &str,
    credit: &str,
) -> Result<Out> {
    let mut out = Out {
        restyles: Vec::new(),
        palette: Vec::new(),
        sound: Vec::new(),
        notes: Vec::new(),
        objects: !local.objects.is_empty(),
    };
    // Restyling the tiles blanks every tile the painter doesn't paint, so a
    // level drawn from tiles must be one it paints whole: as a picture, or,
    // for a level built from objects, metatile by metatile.
    if let Some(sk) = local.skipped.first()
        && local.objects.is_empty()
    {
        bail!("a level the painter can't draw as a picture ({sk}), so its tiles stay the game's");
    }
    let mut banks = Banks::new(local);
    let mut painted: Vec<(String, usize, crate::theme_kit::Cut, Vec<u8>, u32)> = Vec::new();
    let pins_file = dir.join(Pins::FILE);
    let mut pins = Pins::read(&pins_file)?;
    for (ai, a) in kit.art.iter().enumerate() {
        let Some(cuts) = local.cuts.get(&a.id) else {
            continue;
        };
        // The frames slots listed before this one show: a frame of this
        // slot that is one of them is that slot's picture, left to it.
        // Original Edition's 100m fires, while a hammer is out, show the
        // barrels' tiles 48 to 63 (2026-11-07): painted as flames, they
        // drew over the barrels' and the oil drum's tile 58.
        // (A slot painted at another's, as below, paints none of them.)
        let earlier: BTreeSet<_> = kit.art[..ai]
            .iter()
            .filter(|b| {
                same_frames_as(kit.art.iter().map(|c| c.id.as_str()), &local.cuts, &b.id).is_none()
            })
            .filter_map(|b| local.cuts.get(&b.id))
            .flatten()
            .map(frame_key)
            .collect();
        // A slot whose frames are all another slot's (Donkey Kong's big
        // foe on 75m shows three of the 25m big foe's poses, from the same
        // tiles) takes that slot's paint: painted again in a box of its
        // own frames, it would draw over the other's tiles in other poses.
        if let Some(b) = same_frames_as(kit.art.iter().map(|b| b.id.as_str()), &local.cuts, &a.id) {
            out.notes
                .push(format!("{}: its frames are {b}'s, painted there", a.id));
            continue;
        }
        // A figure the game draws as rows of sprites, a body a row, is
        // painted whole and cut back into its rows ([`stack_rows`]).
        let orig = cuts;
        let (stacked, stacked_bodies, parts) =
            stack_rows(orig, local.bodies.get(&a.id), a.kind == "sprite");
        let cuts = &stacked;
        let bodies = stacked_bodies.as_ref();
        let blank_of =
            |c: &crate::theme_kit::Cut| local.blank.get(&c.slot).cloned().unwrap_or_default();
        let blank0 = cuts.first().map(blank_of).unwrap_or_default();
        let borrowed: Vec<bool> = borrowed_frames(cuts, &blank0)
            .into_iter()
            .zip(cuts.iter())
            .map(|(b, c)| b || earlier.contains(&frame_key(c)))
            .collect();
        let group = tile_groups(cuts, &blank0);
        // The shape: the slot's own, or the look of the character it is
        // ([`look_of`]).
        let shape = look_of(kit, local, a).map_or_else(|| shape_of(a), shape_of);
        // A large character fallen over (Original Edition's thrower lying
        // on its head in 100m's ending) is its look upside down.
        let toppled = shape == "brute" && a.description.to_lowercase().contains("fallen");
        // The box of a group: the original's bodies in its frames.
        let raw_box = |g: usize| {
            let b = bodies?;
            (0..cuts.len())
                .filter(|&j| group[j] == g)
                .filter_map(|j| b.get(j).copied().flatten())
                .reduce(|p, q| {
                    [
                        p[0].min(q[0]),
                        p[1].min(q[1]),
                        p[2].max(q[2]),
                        p[3].max(q[3]),
                    ]
                })
        };
        // A group that shows a tile mirrored in one frame and plain in
        // another needs a shape the same both ways (below).
        let raw_mirrored = |g: usize| {
            (0..cuts.len())
                .filter(|&j| group[j] == g)
                .any(|j| cuts[j].places.iter().any(|p| p.flip_x))
        };
        // Each group's pose number, box and mirroring, from the theme's
        // pins ([`Pins`]): a group the pins don't name takes its first
        // frame's place, its frames' box and their mirroring, and is
        // pinned at them, so a frame the base gains later (another pose
        // of the same tiles, mirrored or not) moves no picture.
        let mut pinned: BTreeMap<usize, Pin> = BTreeMap::new();
        for g in group.iter().copied().collect::<BTreeSet<usize>>() {
            let members: Vec<usize> = (0..cuts.len()).filter(|&j| group[j] == g).collect();
            let now = Pin {
                pose: g as u32,
                frame_box: raw_box(g),
                mirrored: raw_mirrored(g),
            };
            pinned.insert(g, pins.pin(&a.id, cuts, &members, &blank0, now));
        }
        let group_box = |g: usize| pinned.get(&g).and_then(|p| p.frame_box);
        let mirrored = |g: usize| pinned.get(&g).is_some_and(|p| p.mirrored);
        // A big foe's hands, group by group, each whole and in tiles no
        // frame that shows another hand shares (`group_hands`).
        let mut hands: BTreeMap<usize, [Option<Hand>; 2]> = BTreeMap::new();
        if shape == "brute" && a.kind != "font" {
            for g in group.iter().copied().collect::<BTreeSet<usize>>() {
                let Some(r) = group_box(g) else { continue };
                let members: Vec<usize> = (0..cuts.len())
                    .filter(|&j| group[j] == g)
                    .filter(|&j| bodies.and_then(|b| b.get(j).copied().flatten()).is_some())
                    .collect();
                let (got, notes) = group_hands(
                    cuts,
                    &members,
                    [r[0] as i32, r[1] as i32],
                    ((r[2] - r[0]) as f32, (r[3] - r[1]) as f32),
                    &blank_of(&cuts[g]),
                    mirrored(g),
                );
                hands.extend(got);
                out.notes
                    .extend(notes.into_iter().map(|n| format!("{}: {n}", a.id)));
            }
        }
        // A frame drawn mirrored that another frame shows plain (the same
        // tiles, the console mirroring them) is that frame's picture:
        // painted too it would put a mirrored second picture into the
        // same tiles, and its ink would be merged into the plain one's.
        // Original Edition gained mirrored copies of the hero's walking
        // and hammer poses (2026-11-07). Only in a group painted one way
        // (its pin's `mirrored` false): one painted the same both ways
        // paints its mirrored frames as it always has. A big foe's hands
        // differ by frame, so its frames are all painted.
        let unmirrored = |c: &crate::theme_kit::Cut| {
            let w = c.places.iter().map(|p| p.col + 1).max().unwrap_or(1);
            let mut v: Vec<(u32, u32, u32, bool)> = c
                .places
                .iter()
                .map(|p| {
                    (
                        p.tile,
                        if p.flip_x { w - 1 - p.col } else { p.col },
                        p.row,
                        p.flip_y,
                    )
                })
                .collect();
            v.sort_unstable();
            v
        };
        let plain = |c: &crate::theme_kit::Cut| c.places.iter().all(|p| !p.flip_x);
        let mirror_of_plain: Vec<bool> = (0..cuts.len())
            .map(|k| {
                shape != "brute"
                    && !mirrored(group[k])
                    && !plain(&cuts[k])
                    && (0..cuts.len()).any(|j| {
                        !borrowed[j]
                            && plain(&cuts[j])
                            && cuts[j].slot == cuts[k].slot
                            && unmirrored(&cuts[j]) == unmirrored(&cuts[k])
                    })
            })
            .collect();
        for (k, c) in cuts.iter().enumerate() {
            // Another frame's tiles at other places, or a plain frame
            // mirrored: left to that frame.
            if borrowed[k] || mirror_of_plain[k] {
                continue;
            }
            let w = c.places.iter().map(|p| p.col + 1).max().unwrap_or(1) * 8;
            let h = c.places.iter().map(|p| p.row + 1).max().unwrap_or(1) * 8;
            let t = if a.kind == "font" {
                glyph_tones(char::from(b'0' + (k as u8 % 10)))
            } else {
                match bodies {
                    Some(b) => {
                        // Frames that share a tile are painted as one: a
                        // shared tile shows the first frame's art, so two
                        // poses would meet at the tile's edge as halves.
                        // The pose is the group's first frame's, and the
                        // box the original's bodies in the group, so a
                        // figure keeps the original's place and size and
                        // never runs into the picture's edge.
                        // Frames that share a tile are painted as one: a
                        // shared tile shows the first frame's art, so two
                        // poses would meet at the tile's edge as halves.
                        // The shape is the group's first frame's, in the
                        // box of the original's bodies in the group, so a
                        // figure keeps the original's place and size and
                        // never runs into the picture's edge. Only a big
                        // foe's hands differ by frame, each where
                        // `group_hands` found it room.
                        let g = group[k];
                        match (group_box(g), b.get(k).copied().flatten()) {
                            (Some(r), Some(_)) => {
                                let (bw, bh) = ((r[2] - r[0]) as u32, (r[3] - r[1]) as u32);
                                let mut shape = paint_frame_in(
                                    shape,
                                    bw,
                                    bh,
                                    pinned[&g].pose,
                                    hands.get(&k).copied(),
                                );
                                if toppled {
                                    let rows: Vec<Vec<u8>> = shape
                                        .t
                                        .chunks(shape.w as usize)
                                        .rev()
                                        .map(<[u8]>::to_vec)
                                        .collect();
                                    shape.t = rows.concat();
                                }
                                if mirrored(g) {
                                    for y in 0..shape.h {
                                        for x in 0..shape.w {
                                            let i = (y * shape.w + x) as usize;
                                            let m = (y * shape.w + shape.w - 1 - x) as usize;
                                            if shape.t[i] == 0 && shape.t[m] != 0 {
                                                shape.t[i] = shape.t[m];
                                            }
                                        }
                                    }
                                }
                                let mut t = Tones::new(w, h);
                                for y in 0..shape.h {
                                    for x in 0..shape.w {
                                        t.set(
                                            x + r[0] as i32,
                                            y + r[1] as i32,
                                            shape.t[(y * shape.w + x) as usize],
                                        );
                                    }
                                }
                                // A cell the frame draws from the blank tile
                                // (the game's own empty cell) can't take ink:
                                // the shape stops there with an edge of its
                                // own, not a cut.
                                let blank = local.blank.get(&c.slot);
                                let mut trimmed = false;
                                for y in 0..t.h {
                                    for x in 0..t.w {
                                        let i = (y * t.w + x) as usize;
                                        let open = c.places.iter().any(|p| {
                                            p.col == x as u32 / 8
                                                && p.row == y as u32 / 8
                                                && blank.is_none_or(|b| !b.contains(&p.tile))
                                        });
                                        if !open && t.t[i] != 0 {
                                            t.t[i] = 0;
                                            trimmed = true;
                                        }
                                    }
                                }
                                if trimmed {
                                    t.finish();
                                }
                                t
                            }
                            // The original draws nothing in this frame (an
                            // unlit fire): neither does paper, and its tiles
                            // are left to the frames that draw.
                            _ => continue,
                        }
                    }
                    None => paint_frame(shape, w, h, k as u32),
                }
            };
            for &(ok, r) in &parts[k] {
                let oc = &orig[ok];
                let rows = (r * 8 * w) as usize..((r * 8 + 8) * w) as usize;
                let sub = if parts[k].len() == 1 {
                    t.t.clone()
                } else {
                    t.t[rows].to_vec()
                };
                banks.put(&a.id, oc, &sub, w);
                if a.kind == "sprite" {
                    painted.push((a.id.clone(), ok, oc.clone(), sub, w));
                }
            }
        }
    }
    // Pins of a slot the kit no longer has go.
    pins.by
        .retain(|(art, _), _| kit.art.iter().any(|b| b.id == *art));
    pins.write(&pins_file)?;
    // Frames that show, in a tile they share, art painted for another:
    // their ink as drawn from the bank against their ink as painted.
    let mut torn = Vec::new();
    // A frame of a slot whose tiles sit at other places in other frames
    // can show another frame's clear pixels over its shape, a corner cut
    // off: each frame's ink is put into the tiles its slot took, so no
    // frame loses any of its own.
    for (id, _, c, t, w) in &painted {
        banks.merge(id, c, t, *w);
    }
    for (id, k, c, t, _) in &painted {
        let ink: Vec<bool> = t.iter().map(|v| *v != 0).collect();
        let Some((sz, px, _)) = banks.banks.get(&c.slot) else {
            continue;
        };
        let shown = crate::theme_original::frame_of(px, *sz, c);
        let shown: Vec<bool> = shown.t.iter().map(|v| *v != 0).collect();
        let off = shown.iter().zip(&ink).filter(|(a, b)| a != b).count();
        if off > 0 {
            torn.push(format!("{id} frame {k} ({off} px)"));
        }
    }
    if !torn.is_empty() {
        out.notes.push(format!(
            "{} sprite frames show another frame's art in a tile they share: {}",
            torn.len(),
            torn.join(", ")
        ));
    }
    // The font's letters and signs, in the theme's own font.
    for (c, cut) in &local.letters {
        banks.put(&format!("letter.{c}"), cut, &glyph_tones(*c).t, 8);
    }
    // A cell a single-tile level changes to a kind as it runs (100m's
    // ending lays a girder under the fallen foe), drawn over the stage's
    // picture from the level's bank: a strip of the picture's ground, in
    // the tile no glyph or figure took (`Banks::put` leaves those).
    for (kind, cut) in &local.kind_cells {
        banks.put(&format!("cell.{kind}"), cut, &ground_tones(), 8);
    }
    // Levels built from objects: every metatile by its role, the roles
    // the hero meets first, so a tile two roles share takes the first's.
    let order = [
        "collectible",
        "power-up",
        "interactive",
        "terrain",
        "solid",
        "platform",
        "goal",
        "water",
        "background",
        "sky",
    ];
    let mut objects: Vec<(usize, &(String, crate::theme_kit::Cut))> =
        local.objects.iter().enumerate().collect();
    objects.sort_by_key(|(i, (role, _))| {
        (
            order.iter().position(|r| r == role).unwrap_or(order.len()),
            *i,
        )
    });
    for (i, (role, cut)) in objects {
        banks.put(
            &format!("object.{role}"),
            cut,
            &object_tones(role, i as u32).t,
            16,
        );
    }
    for (slot, text) in banks.pix(&format!("Painted by {generator} (CC0-1.0)")) {
        let file = format!("{slot}.pix");
        let sha = write_file(dir, &file, &text)?;
        out.restyles.push(restyle(
            Some(&slot),
            None,
            &file,
            sha,
            generator,
            "generated",
        ));
    }
    out.palette = tone_rules(kit, local, &|r| paper_tones(r));
    // Pictures.
    for a in kit.art.iter().filter(|a| a.kind == "picture") {
        let Some(rest) = a.id.strip_prefix("picture.") else {
            continue;
        };
        let Some((slot, entry)) = rest.split_once('.') else {
            continue;
        };
        let Some(cells) = local.cells.get(&a.id) else {
            out.notes.push(format!(
                "{}: no cells in the local kit, left as the game draws it",
                a.id
            ));
            continue;
        };
        let img = match a.role.as_str() {
            "hud" => hud_picture(a.size, cells, &m.model.title, &hud_numbers(m)),
            "menu" => screen_picture(a.size, cells, logo, credit, seed),
            _ => {
                let level = m
                    .levels
                    .iter()
                    .find(|(_, l)| {
                        l.core
                            .layout
                            .as_ref()
                            .is_some_and(|x| x.slot == slot && x.entry == entry)
                    })
                    .and_then(|(n, _)| spec.levels.iter().find(|l| &l.name == n));
                level_picture(a.size, level, Some(cells), seed)
            }
        };
        let file = format!("{}.pix", a.id.replace('.', "-"));
        let text = rgba_pix(&img, &format!("{} painted by {generator} (CC0-1.0)", a.id))?;
        let sha = write_file(dir, &file, &text)?;
        out.restyles.push(restyle(
            Some(slot),
            Some(entry),
            &file,
            sha,
            generator,
            "generated",
        ));
    }
    out.notes.extend(banks.conflicts.iter().take(0).cloned());
    for snd in &kit.sound {
        let text = tune_for(
            &snd.role,
            snd.kind == "music",
            snd.loops,
            &snd.description,
            snd.frames,
        );
        let file = format!("{}.tune", snd.role);
        let sha = write_file(dir, &file, &text)?;
        out.sound.push(SoundReplace {
            role: snd.role.clone(),
            file,
            origin: "generated".into(),
            license: "CC0-1.0".into(),
            sha256: sha,
            generator: Some(generator.into()),
            author: None,
            note: None,
        });
    }
    Ok(out)
}

/// One cell of the stage picture's ground strip ([`level_picture`]'s rows
/// of three or more), in tones: a light top edge, a dark bottom, a dark
/// brace every eight pixels on the slant, the middle tone between.
fn ground_tones() -> Vec<u8> {
    (0..64u32)
        .map(|i| {
            let (x, dy) = (i % 8, i / 8);
            match dy {
                0 => 3,
                7 => 1,
                _ if (x + dy) % 8 == 0 => 1,
                _ => 2,
            }
        })
        .collect()
}

/// A picture with at most 78 colors, as a `.pix` holds: each channel
/// rounded to fewer levels until it fits.
pub fn few_colors(img: &Rgba) -> Rgba {
    for bits in (1..=8u32).rev() {
        let keep = 0xFFu8 << (8 - bits);
        let mut out = img.clone();
        let mut seen = std::collections::BTreeSet::new();
        for p in &mut out.pixels {
            if p[3] < 128 {
                *p = [0; 4];
                continue;
            }
            let half = if bits < 8 { 1u8 << (7 - bits) } else { 0 };
            for c in p.iter_mut().take(3) {
                *c = (*c & keep) | half;
            }
            p[3] = 255;
            seen.insert([p[0], p[1], p[2]]);
        }
        if seen.len() <= 78 {
            return out;
        }
    }
    img.clone()
}

/// A picture scaled to `w` by `h`, nearest pixel.
pub fn scale_to(img: &Rgba, w: u32, h: u32) -> Rgba {
    if (img.width, img.height) == (w, h) {
        return img.clone();
    }
    let mut out = Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.set(
                x,
                y,
                img.get(x * img.width / w.max(1), y * img.height / h.max(1)),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_one_to_rescue_is_a_teal_robot_with_two_poses() {
        // No pink: the red tone of every npc color stays below its green.
        for c in paper_tones("npc") {
            assert!(c[1] > c[0], "{c:?}");
        }
        let (a, b) = (
            paint_frame("robot", 16, 24, 0),
            paint_frame("robot", 16, 24, 1),
        );
        assert_ne!(a.t, b.t);
        // Narrow at the feet and the head, not a skirt widening to the hem.
        let row = |t: &Tones, y: i32| (0..16).filter(|&x| t.t[(y * 16 + x) as usize] != 0).count();
        assert!(row(&a, 23) <= 10, "{}", row(&a, 23));
    }

    #[test]
    fn the_big_foes_hands_are_whole_and_one_size_in_every_box() {
        // Boxes as narrow and as wide as Rivet Rodeo's big foe's poses
        // (34 to 48 by 31): every hand is round, the same size in each,
        // inside the box, its outer edge a twenty-fifth of the box in.
        let sizes: Vec<f32> = [34.0f32, 38.0, 40.0, 42.0, 45.0, 48.0]
            .iter()
            .flat_map(|&fw| {
                [false, true].map(|right| {
                    let (cx, cy, r) = brute_hand(fw, 31.0, right, 0.3);
                    assert!(cx - r >= 0.0 && cx + r <= fw, "{fw}: {cx} {r}");
                    assert!(cy - r >= 0.0 && cy + r <= 31.0, "{fw}: {cy} {r}");
                    let outer = if right { fw - (cx + r) } else { cx - r };
                    assert!(
                        (outer - fw * 0.04).abs() < 0.01,
                        "{fw}: the hand is {outer} px in from the side"
                    );
                    r
                })
            })
            .collect();
        assert!(sizes.windows(2).all(|w| w[0] == w[1]), "{sizes:?}");
        // Painted, a hand in a narrow pose is as wide as it is tall, not
        // a sliver: the raised right hand of pose 3 in a 34 px box.
        let t = paint_frame("brute", 34, 31, 3);
        let ink = |x: i32, y: i32| t.t[(y * 34 + x) as usize] != 0;
        let (_, cy, r) = brute_hand(34.0, 31.0, true, 0.3);
        let row = cy as i32;
        let wide = (24..34).filter(|&x| ink(x, row)).count();
        assert!(wide as f32 >= 2.0 * r - 2.0, "{wide} px across, radius {r}");
        // A frame whose cells over y 24 are the game's blank tile (a made
        // up 48 by 32 box, the hand low on the left): the hand moves down
        // into the open row whole, a smaller disc, not cut at y 24.
        let mut ok = |h: Hand| disc(&h).iter().all(|&(_, y)| y >= 24).then_some(1);
        let left = hand_place(brute_hand(48.0, 32.0, false, 0.7), 32.0, false, &mut ok);
        let (cx, cy, r) = left.unwrap();
        assert!(cy - r > 23.5 && cy + r <= 32.0, "{cx} {cy} {r}");
        let right = brute_hand(48.0, 32.0, true, 0.7);
        let t = paint_frame_in("brute", 48, 32, 0, Some([left, Some(right)]));
        // Its outer edge stays at the box's side, and it is round: its
        // top row (left of the body, x under 9) is narrower than its
        // widest, where a hand cut at y 24 is as wide there as anywhere.
        assert!((cx - r - 48.0 * 0.04).abs() < 0.01, "{cx} {r}");
        let widths: Vec<usize> = (0..32)
            .map(|y| (0..9).filter(|&x| t.t[(y * 48 + x) as usize] != 0).count())
            .filter(|&n| n > 0)
            .collect();
        let widest = widths.iter().copied().max().unwrap_or(0);
        assert!(
            widths[0] + 2 <= widest,
            "the left hand's top row is {} px of {widest}: cut flat",
            widths[0]
        );
    }

    #[test]
    fn frames_that_share_a_tile_are_one_group() {
        // Made-up cuts: frames 0 and 2 share tile 5, frame 2 and 3 tile 9,
        // frame 1 shares nothing. A shared tile shows the first frame's
        // art, so 0, 2 and 3 are painted as one and 1 alone.
        let cut = |tiles: &[u32]| crate::theme_kit::Cut {
            slot: "sprite_tiles".into(),
            places: tiles
                .iter()
                .enumerate()
                .map(|(i, &tile)| crate::theme_kit::Place {
                    tile,
                    col: i as u32 % 2,
                    row: i as u32 / 2,
                    flip_x: false,
                    flip_y: false,
                    colors: [0x0F, 0x00, 0x10, 0x30],
                })
                .collect(),
        };
        let cuts = vec![
            cut(&[1, 2, 5, 6]),
            cut(&[3, 4, 7, 8]),
            cut(&[10, 11, 5, 9]),
            cut(&[12, 13, 9, 14]),
            // The blank tile (36) is never painted, so it joins nothing.
            cut(&[36, 15, 16, 17]),
        ];
        assert_eq!(tile_groups(&cuts, &[36]), vec![0, 1, 0, 0, 4]);
        assert_eq!(
            tile_groups(
                &cuts[..2]
                    .iter()
                    .chain(&cuts[4..])
                    .cloned()
                    .collect::<Vec<_>>(),
                &[]
            ),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn a_slot_showing_only_another_slots_frames_is_painted_there() {
        // Made-up cuts: a big foe of three frames, and a second slot that
        // shows two of them (as Donkey Kong's 75m big foe shows 25m's
        // poses), one listed before the other; a third slot shares a
        // frame and has one of its own, so it is painted itself.
        let cut = |tiles: [u32; 2]| crate::theme_kit::Cut {
            slot: "tiles".into(),
            places: tiles
                .iter()
                .enumerate()
                .map(|(i, &tile)| crate::theme_kit::Place {
                    tile,
                    col: i as u32,
                    row: 0,
                    flip_x: false,
                    flip_y: false,
                    colors: [0x0F, 0x00, 0x10, 0x30],
                })
                .collect(),
        };
        let cuts: BTreeMap<String, Vec<crate::theme_kit::Cut>> = [
            ("a", vec![cut([1, 2]), cut([5, 6])]),
            ("foe", vec![cut([1, 2]), cut([3, 4]), cut([5, 6])]),
            ("c", vec![cut([1, 2]), cut([7, 8])]),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let ids = ["a", "foe", "c"];
        assert_eq!(same_frames_as(ids, &cuts, "a"), Some("foe"));
        assert_eq!(same_frames_as(ids, &cuts, "foe"), None);
        assert_eq!(same_frames_as(ids, &cuts, "c"), None);
    }

    #[test]
    fn a_platform_is_a_plank_and_a_spring_a_coil_its_box_tall() {
        // Each fills its box's width and height, so a platform's top is
        // where the hero stands and a pressed spring is squat.
        for (shape, w, h) in [("plank", 16, 8), ("spring", 16, 15), ("spring", 16, 9)] {
            let t = paint_frame(shape, w, h, 0);
            let ink = |x: i32, y: i32| t.t[(y * t.w + x) as usize] != 0;
            assert!(
                (0..w as i32).all(|x| ink(x, 0) && ink(x, h as i32 - 1)),
                "{shape} {w}x{h}"
            );
        }
    }

    #[test]
    fn a_ladder_that_moves_is_two_rails_and_rungs_its_box_tall() {
        let t = paint_frame("rungs", 16, 16, 0);
        let ink = |x: i32, y: i32| t.t[(y * t.w + x) as usize] != 0;
        // Both rails run the box's height, and the middle has rungs and gaps.
        assert!((0..16).all(|y| ink(2, y) && ink(13, y)));
        let mid: Vec<bool> = (0..16).map(|y| ink(8, y)).collect();
        assert!(mid.contains(&true) && mid.contains(&false));
    }

    /// A made-up big foe of five frames, 6 by 4 tiles, cut as Donkey
    /// Kong's is (the tiles made up): frames 0 and 1 are the same tiles,
    /// whose own tiles are one row on each side; frame 2 shares the tiles
    /// over frame 0's left hand row, and frame 3 those over its right; both
    /// draw the blank tile (0) where a hand of their pose would go, and
    /// frame 4 shares only the blank tile. Every frame's hands are whole and
    /// full size, and every tile two frames show is the same in both, so no
    /// frame shows another's hand.
    #[test]
    fn a_big_foes_hands_take_only_tiles_no_other_pose_shows_otherwise() {
        let grid = |rows: [[u32; 6]; 4]| crate::theme_kit::Cut {
            slot: "tiles".into(),
            places: rows
                .iter()
                .enumerate()
                .flat_map(|(row, r)| {
                    r.iter()
                        .enumerate()
                        .map(move |(col, &tile)| crate::theme_kit::Place {
                            tile,
                            col: col as u32,
                            row: row as u32,
                            flip_x: false,
                            flip_y: false,
                            colors: [0x0F, 0x00, 0x10, 0x30],
                        })
                })
                .collect(),
        };
        let a = grid([
            [0, 10, 11, 12, 13, 0],
            [20, 21, 14, 15, 30, 31],
            [40, 41, 16, 17, 42, 43],
            [18, 19, 50, 51, 22, 23],
        ]);
        let cuts = vec![
            a.clone(),
            a,
            grid([
                [0, 10, 11, 12, 60, 61],
                [20, 21, 14, 15, 62, 63],
                [64, 65, 16, 17, 66, 0],
                [67, 68, 50, 51, 22, 23],
            ]),
            grid([
                [70, 71, 11, 12, 13, 0],
                [72, 73, 14, 15, 30, 31],
                [0, 74, 16, 17, 75, 76],
                [18, 19, 50, 51, 77, 78],
            ]),
            grid([
                [0, 0, 80, 81, 82, 83],
                [0, 0, 84, 85, 86, 87],
                [0, 88, 89, 90, 91, 92],
                [93, 94, 95, 96, 97, 98],
            ]),
        ];
        let blank = [0];
        let group = tile_groups(&cuts, &blank);
        assert_eq!(group, vec![0, 0, 0, 0, 4]);
        let (hands, notes) = group_hands(&cuts, &[0, 1, 2, 3], [0, 0], (48.0, 32.0), &blank, false);
        let full = brute_hand(48.0, 32.0, false, 0.5).2;
        let open = |k: usize, x: i32, y: i32| {
            cuts[k]
                .places
                .iter()
                .any(|p| p.col == x as u32 / 8 && p.row == y as u32 / 8 && p.tile != 0)
        };
        for k in 0..4 {
            for h in hands[&k] {
                let h = h.unwrap_or_else(|| panic!("frame {k} lost a hand: {notes:?}"));
                assert_eq!(h.2, full, "frame {k}: {h:?} {notes:?}");
                assert!(
                    disc(&h).iter().all(|&(x, y)| open(k, x, y)),
                    "frame {k}: {h:?} is cut"
                );
            }
        }
        // Painted, each tile two frames show is the same picture.
        let shapes: Vec<Tones> = (0..4)
            .map(|k| paint_frame_in("brute", 48, 32, 0, Some(hands[&k])))
            .collect();
        let tile_px = |k: usize, col: u32, row: u32| -> Vec<u8> {
            (0..64)
                .map(|i| {
                    let (x, y) = (col * 8 + i % 8, row * 8 + i / 8);
                    shapes[k].t[(y * 48 + x) as usize]
                })
                .collect()
        };
        for k in 0..4 {
            for j in 0..k {
                for p in cuts[k].places.iter().filter(|p| p.tile != 0) {
                    if let Some(q) = cuts[j].places.iter().find(|q| q.tile == p.tile) {
                        assert_eq!(
                            tile_px(k, p.col, p.row),
                            tile_px(j, q.col, q.row),
                            "tile {} differs between frames {j} and {k}",
                            p.tile
                        );
                    }
                }
            }
        }
        // The two frames whose own cells hold no whole hand hold one in
        // common with the frame they share the tiles with.
        assert_eq!(hands[&0][0], hands[&2][0], "{notes:?}");
        assert_eq!(hands[&0][1], hands[&3][1], "{notes:?}");
        assert_ne!(hands[&2][1], hands[&0][1]);
    }

    #[test]
    fn tunes_parse_and_differ_by_role() {
        let a = tune_for("stage_music", true, true, "the stage's loop", None);
        let b = tune_for("hammer_music", true, true, "a fast, urgent loop", None);
        assert_ne!(a, b);
        for (r, m, l, d) in [
            ("jump", false, false, "the hero's jump"),
            ("walk", false, false, "a soft tick on every step"),
            ("smash", false, false, "a hazard smashed"),
            (
                "death",
                false,
                false,
                "the falling note as the hero is lost",
            ),
            ("thump", false, false, "a low thump"),
            ("clear", true, false, "a victory tune"),
            ("x", false, false, "anything"),
        ] {
            let t = tune_for(r, m, l, d, Some(30));
            gp_theme::tune::Tune::parse(&t).unwrap_or_else(|e| panic!("{r}: {e}\n{t}"));
        }
        assert!(gp_theme::tune::Tune::parse(&a).unwrap().looped);
    }

    #[test]
    fn run_together_labels_move_over_their_numbers() {
        // Rivet Rodeo's status row before: a letter before each number.
        let mut grid: Vec<Vec<char>> = vec![
            "                                ".chars().collect(),
            "                    ??????????  ".chars().collect(),
            "                    M B    L    ".chars().collect(),
        ];
        let n = |col, width, counter: &str| HudNumber {
            col,
            row: 2,
            width,
            counter: counter.into(),
        };
        let numbers = [n(21, 1, "lives"), n(23, 4, "bonus"), n(28, 1, "loop_shown")];
        let (labels, boxes) = hud_labels(&mut grid, &numbers);
        let texts: Vec<&str> = labels.iter().map(|l| l.3.as_str()).collect();
        assert_eq!(texts, ["M", "BONUS", "L"]);
        assert!(labels.iter().all(|l| l.2 == 1));
        assert_eq!(boxes.into_iter().collect::<Vec<_>>(), [2]);
        assert_eq!(grid[2].iter().filter(|c| **c != ' ').count(), 0);
    }

    #[test]
    fn a_picture_word_over_a_number_gets_its_name() {
        // Donkey Kong's status box: M and L read, BONUS is the game's
        // picture over the number.
        let mut grid: Vec<Vec<char>> = vec![
            "?M??????L?".chars().collect(),
            "? ?    ? ?".chars().collect(),
        ];
        let n = |col, width, counter: &str| HudNumber {
            col,
            row: 1,
            width,
            counter: counter.into(),
        };
        let numbers = [n(1, 1, "lives"), n(3, 4, "bonus"), n(8, 1, "loop_shown")];
        let (labels, boxes) = hud_labels(&mut grid, &numbers);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].3, "BONUS");
        assert!(boxes.contains(&1));
    }

    #[test]
    fn a_lone_letter_before_a_number_stays() {
        let mut grid: Vec<Vec<char>> = vec!["      ".chars().collect(), " X    ".chars().collect()];
        let numbers = [HudNumber {
            col: 2,
            row: 1,
            width: 2,
            counter: "coins".into(),
        }];
        let (labels, boxes) = hud_labels(&mut grid, &numbers);
        assert!(labels.is_empty() && boxes.is_empty());
        assert_eq!(grid[1][1], 'X');
    }

    #[test]
    fn every_glyph_is_five_by_seven() {
        for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-=.".chars() {
            let g = glyph(c).unwrap();
            assert!(g.iter().all(|r| r.len() == 5), "{c}");
        }
    }

    #[test]
    fn a_picture_round_trips_through_pix() {
        let mut img = Rgba::new(3, 2);
        img.set(0, 0, [1, 2, 3, 255]);
        img.set(2, 1, [9, 9, 9, 255]);
        let text = rgba_pix(&img, "test").unwrap();
        assert_eq!(gp_theme::pix::parse(&text).unwrap(), img);
    }

    /// Every sound role of a template's kit can be a `.tune`: Donkey
    /// Kong's 14 are, in its paper theme, and each parses and renders. An
    /// effect by parameters (`sfx`) is a `.tune` too.
    #[test]
    fn every_sound_role_is_a_tune() {
        // gameprinter: the kit is the games checkout's, and the paper theme,
        // taken out of the game on 2026-11-11, is one `gp theme paint` made
        // ($GP_THEME_PAPER); skipped when either is absent.
        let Some(games) = crate::theme_template::test_games() else {
            return;
        };
        let kit_dir = games.join("donkey-kong/themes");
        let Some(paper) = std::env::var_os("GP_THEME_PAPER").map(std::path::PathBuf::from) else {
            return;
        };
        if !paper.join("theme.toml").is_file() {
            return;
        }
        let kit: Kit =
            toml::from_str(&fs::read_to_string(kit_dir.join("kit/kit.toml")).unwrap()).unwrap();
        let theme = gp_theme::Theme::load(&paper).unwrap();
        assert_eq!(kit.sound.len(), 14);
        for s in &kit.sound {
            let r = theme.sound.iter().find(|x| x.role == s.role).unwrap();
            assert!(r.file.ends_with(".tune"), "{}", s.role);
            let t = gp_theme::tune::Tune::parse(&fs::read_to_string(paper.join(&r.file)).unwrap())
                .unwrap();
            assert!(!gp_theme::tune::render(&t).is_empty(), "{}", s.role);
        }
        let zap = gp_theme::tune::Tune::parse(
            "tune 1\nstep 2\nsfx zap 0.5 wave=square freq=C6 slide=-36 decay=12\nsfx thud 0.6 wave=noise freq=80 decay=8 lowpass=0.3 at=2\n",
        )
        .unwrap();
        assert_eq!(zap.frames(), 18);
        assert!(
            gp_theme::tune::render(&zap)
                .iter()
                .any(|v| v.unsigned_abs() > 1000)
        );
    }

    #[test]
    fn a_hud_cell_drawn_by_two_fields_gets_one_label() {
        // Original Edition's bonus at [23, 5]: `bonus`, and `hud_bonus`
        // while `hud_stale` is 1. The field drawn always keeps the cell,
        // listed first or not, so the label reads BONUS, not H.
        let n = |col: u32, width: u32, counter: &str| HudNumber {
            col,
            row: 5,
            width,
            counter: counter.into(),
        };
        for bonus in [
            vec![(n(23, 4, "bonus"), false), (n(23, 4, "hud_bonus"), true)],
            vec![(n(23, 4, "hud_bonus"), true), (n(23, 4, "bonus"), false)],
        ] {
            let mut fields = vec![(n(21, 1, "lives"), false)];
            fields.extend(bonus);
            let kept = one_per_cell(fields);
            let names: Vec<&str> = kept.iter().map(|k| k.counter.as_str()).collect();
            assert_eq!(names, ["lives", "bonus"]);
            // Under the game's own marks (a word drawn as a picture), the
            // bonus is spelled out once.
            let mut grid = vec![vec![' '; 32]; 6];
            for cell in &mut grid[4][23..27] {
                *cell = '?';
            }
            let (labels, _) = hud_labels(&mut grid, &kept);
            let texts: Vec<&str> = labels.iter().map(|l| l.3.as_str()).collect();
            assert_eq!(texts, ["BONUS"]);
        }
    }

    #[test]
    fn a_frame_drawn_from_other_frames_tiles_joins_no_group() {
        // Original Edition's player: standing from tile 4, walking from 0
        // and 8, and as he walks off a girder's end, from 2 (the right
        // half of 0's pose, the left of 4's) and from 6. Those two are the
        // others' tiles at other places: they join nothing and aren't
        // painted, so the poses stay apart, each in its own box.
        let cut = |tiles: [u32; 4], flip: bool| crate::theme_kit::Cut {
            slot: "sprite_tiles".into(),
            places: tiles
                .iter()
                .enumerate()
                .map(|(i, &tile)| crate::theme_kit::Place {
                    tile,
                    col: if flip { 1 - i as u32 % 2 } else { i as u32 % 2 },
                    row: i as u32 / 2,
                    flip_x: flip,
                    flip_y: false,
                    colors: [0x0F, 0x00, 0x10, 0x30],
                })
                .collect(),
        };
        let cuts = vec![
            cut([4, 6, 5, 7], false),
            cut([2, 4, 3, 5], false),
            cut([6, 8, 7, 9], false),
            // Mirrored, a pose's tiles are at their home read unmirrored.
            cut([4, 6, 5, 7], true),
            cut([0, 2, 1, 3], true),
            cut([8, 10, 9, 11], true),
            cut([0, 2, 1, 3], false),
            cut([8, 10, 9, 11], false),
        ];
        assert_eq!(
            borrowed_frames(&cuts, &[]),
            [false, true, true, false, false, false, false, false]
        );
        assert_eq!(tile_groups(&cuts, &[]), vec![0, 1, 2, 0, 4, 5, 4, 5]);
    }

    #[test]
    fn a_frame_the_base_gains_moves_no_pinned_pose() {
        // A slot of three poses, painted once (pins written), then the
        // base gains a frame before them all (a mirrored copy of the third
        // pose, as Original Edition gained its mirrored walk): each pose
        // keeps its number, box and one-way painting, and the new frame's
        // group takes its pin rather than its new place.
        let cut = |tiles: [u32; 4], flip: bool| crate::theme_kit::Cut {
            slot: "sprite_tiles".into(),
            places: tiles
                .iter()
                .enumerate()
                .map(|(i, &tile)| crate::theme_kit::Place {
                    tile,
                    col: if flip { 1 - i as u32 % 2 } else { i as u32 % 2 },
                    row: i as u32 / 2,
                    flip_x: flip,
                    flip_y: false,
                    colors: [0x0F, 0x00, 0x10, 0x30],
                })
                .collect(),
        };
        let poses = |pins: &mut Pins, cuts: &[crate::theme_kit::Cut]| -> Vec<Pin> {
            let group = tile_groups(cuts, &[]);
            let roots: BTreeSet<usize> = group.iter().copied().collect();
            roots
                .into_iter()
                .map(|g| {
                    let members: Vec<usize> = (0..cuts.len()).filter(|&j| group[j] == g).collect();
                    let now = Pin {
                        pose: g as u32,
                        frame_box: Some([0, 0, 16, 16]),
                        mirrored: members.iter().any(|&j| cuts[j].places[0].flip_x),
                    };
                    pins.pin("entity.player", cuts, &members, &[], now)
                })
                .collect()
        };
        let before = vec![
            cut([0, 2, 1, 3], false),
            cut([4, 6, 5, 7], false),
            cut([8, 10, 9, 11], false),
        ];
        let mut pins = Pins::default();
        let first = poses(&mut pins, &before);
        assert_eq!(first.iter().map(|p| p.pose).collect::<Vec<_>>(), [0, 1, 2]);
        // Written and read back, as the next paint reads them.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(Pins::FILE);
        pins.write(&file).unwrap();
        let mut pins = Pins::read(&file).unwrap();
        let mut after = vec![cut([8, 10, 9, 11], true)];
        after.extend(before.iter().cloned());
        let again = poses(&mut pins, &after);
        // The groups are now [8 mirrored, 8], [0], [4], first by place:
        // the 8 group first. Every pose is the one painted before.
        let by_tiles = |ps: &[Pin], order: [usize; 3]| order.map(|i| ps[i]);
        assert_eq!(by_tiles(&again, [1, 2, 0]), [first[0], first[1], first[2]]);
        assert!(
            !again[0].mirrored,
            "the pin keeps the 8 pose painted one way"
        );
    }
}
