//! `gp model blind`: a model and a state mapping from the ROM alone, by
//! playing it. Blind discovery (crates/gp-cli/src/blind.rs) finds play
//! start, the player's x and y bytes, the buttons for jump and run and
//! the forward direction. This file measures more from there, with the
//! game running and nothing read about it: the byte that carries x's
//! high part, the walk and run speeds, the jump's takeoff speed and
//! gravity, the fastest fall, and the player's size on screen. It writes
//! them as model facts with evidence (`observed`, each with the frames it
//! came from) and the player's position as mapping fields, so a capture
//! run can compare the rebuilt game with the original.
//!
//! Nothing here names a game, an address or a constant of one: every
//! number comes from a run of the ROM it is given.
//!
//! Ported from game-printer whole. Blind discovery and the Laya search are
//! wave 7's (crate::farm::blind), and the jump rule's fit is
//! crate::rule_refine::fit_traces. It plays gp-nes and reads the NES's RAM
//! and sprite table, so an arcade romset is refused in words.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use gp_evidence::input_log::InputFrame;
use serde_json::{Value, json};

use crate::Outcome;

/// A model fact the level gives: its path, value, the reason and whether
/// the asset map alone gives it (false: the contacts decide).
type LevelFact = (String, Value, String, bool);

#[derive(Args, Debug)]
pub struct BlindArgs {
    /// The model directory to write into: its model.toml gets the facts
    /// (the header it has is kept) and mapping.toml is written.
    pub model: PathBuf,
    /// The ROM to play.
    #[arg(long)]
    pub rom: PathBuf,
    /// Rounds for the Laya search over the first level (0 skips it). The
    /// search stops at the level's end or after this many rounds, never on
    /// a clock, so the facts don't depend on how busy the machine is.
    #[arg(long, default_value_t = 600)]
    pub play_rounds: usize,
    /// The decision server for the search.
    #[arg(long, default_value = "http://127.0.0.1:7720")]
    pub decide: String,
    /// The asset map, for the level: its grid and tileset are lifted from
    /// the ROM and tested against the measured contacts.
    #[arg(long)]
    pub assets: Option<PathBuf>,
}

/// One frame of a measuring run: the player's bytes and the sprites.
struct Seen {
    wram: Vec<u8>,
    oam: [u8; 256],
    pad: u8,
}

/// A start the measures all begin from: the console at play start, the
/// frames that led there, and what blind discovery learned.
struct Start {
    nes: gp_nes::Nes,
    snap: Vec<u8>,
    frame: u64,
    held: [u8; 4],
    x: usize,
    y: usize,
    jump: u8,
    run: u8,
    forward: u8,
    box_dy: i64,
}

impl Start {
    /// Runs forward with run held, jumping for 24 frames when x hasn't moved
    /// for 4 frames, for `frames` frames.
    fn forward_run(&mut self, frames: usize) -> Result<Vec<Seen>> {
        self.nes.restore(&self.snap, self.frame, self.held)?;
        let mut out: Vec<Seen> = Vec::new();
        let mut jump_left = 0usize;
        for _ in 0..frames {
            let n = out.len();
            if jump_left == 0
                && n >= 4
                && (n - 4..n).all(|k| out[k].wram[self.x] == out[n - 1].wram[self.x])
            {
                jump_left = 24;
            }
            let mut pad = self.forward | self.run;
            if jump_left > 0 {
                pad |= self.jump;
                jump_left -= 1;
            } else if n >= 1
                && out[n - 1].wram[self.y]
                    != out.get(n.wrapping_sub(2)).map_or(0, |s| s.wram[self.y])
            {
                // Let go of jump in the air so the next press is a new one.
            }
            self.nes.step(InputFrame {
                polled: true,
                pads: [pad, 0, 0, 0],
                reset: false,
            })?;
            out.push(Seen {
                wram: self.nes.wram()[..0x800].to_vec(),
                oam: self.nes.oam(),
                pad,
            });
        }
        Ok(out)
    }

    /// Plays `pads` (a button set held for so many frames, in turn) from
    /// the start and returns each frame.
    fn play(&mut self, pads: &[(u8, usize)]) -> Result<Vec<Seen>> {
        self.nes.restore(&self.snap, self.frame, self.held)?;
        let mut out = Vec::new();
        for &(pad, n) in pads {
            for _ in 0..n {
                self.nes.step(InputFrame {
                    polled: true,
                    pads: [pad, 0, 0, 0],
                    reset: false,
                })?;
                out.push(Seen {
                    wram: self.nes.wram()[..0x800].to_vec(),
                    oam: self.nes.oam(),
                    pad,
                });
            }
        }
        Ok(out)
    }
}

fn button_name(bit: u8) -> &'static str {
    match bit {
        0x01 => "a",
        0x02 => "b",
        0x04 => "select",
        0x08 => "start",
        0x10 => "up",
        0x20 => "down",
        0x40 => "left",
        _ => "right",
    }
}

/// The byte that steps by one each time x's low byte wraps, and holds
/// still between: x's high part. None when x never wraps in the run.
fn high_byte(seen: &[Seen], lo: usize, other: usize) -> Option<usize> {
    let wraps: Vec<(usize, i64)> = seen
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| {
            let (a, b) = (w[0].wram[lo], w[1].wram[lo]);
            let step = ((i64::from(b) - i64::from(a) + 384) % 256) - 128;
            if step.abs() > 32 {
                None
            } else if a >= 0xC0 && b < 0x40 {
                Some((i + 1, 1))
            } else if a < 0x40 && b >= 0xC0 {
                Some((i + 1, -1))
            } else {
                None
            }
        })
        .collect();
    if wraps.is_empty() {
        return None;
    }
    let mut found = Vec::new();
    for a in 0..0x800usize {
        if a == lo {
            continue;
        }
        let mut ok = true;
        for (i, w) in seen.windows(2).enumerate() {
            // A jump of the low byte that isn't a wrap (a respawn, a new
            // room) can move the high byte any way: those frames don't count.
            let jump = |b: usize| {
                let (p, q) = (i64::from(w[0].wram[b]), i64::from(w[1].wram[b]));
                (((q - p + 384) % 256) - 128).abs() > 32
            };
            if jump(lo) || jump(other) {
                continue;
            }
            let d = i64::from(w[1].wram[a]) - i64::from(w[0].wram[a]);
            let want = wraps
                .iter()
                .find(|(f, _)| *f == i + 1)
                .map_or(0, |(_, s)| *s);
            if d != want {
                ok = false;
                break;
            }
        }
        if ok {
            found.push(a);
        }
    }
    (found.len() == 1).then(|| found[0])
}

/// The byte holding the vertical speed's whole part: over the frames in
/// the air, its value (signed) is within a pixel of how far y moves next
/// on 9 frames in 10 (a landing or a bump cuts a move short), and it takes
/// several values. None unless exactly one byte does.
fn speed_byte(runs: &[&[Seen]], y: usize) -> Option<usize> {
    let mut found = Vec::new();
    for b in 0..0x800usize {
        if b == y {
            continue;
        }
        let (mut n, mut near, mut values) = (0usize, 0usize, std::collections::BTreeSet::new());
        for seen in runs {
            for w in seen.windows(2) {
                let dy = i64::from(w[1].wram[y]) - i64::from(w[0].wram[y]);
                if dy == 0 || dy.abs() > 16 {
                    continue;
                }
                let v = i64::from(w[0].wram[b] as i8);
                n += 1;
                values.insert(v);
                near += usize::from((v - dy).abs() <= 1);
            }
        }
        if n >= 20 && near * 10 >= n * 9 && values.len() >= 4 {
            found.push(b);
        }
    }
    (found.len() == 1).then(|| found[0])
}

/// The frames where the player stops dead against something while moving
/// `dir` with it held: (frame, x). With `y`, only stops made standing (y
/// still from 3 frames before to 2 after), so a death or a stop in the air
/// isn't one.
fn stops_on(
    seen: &[Seen],
    x: usize,
    hi: Option<usize>,
    dir: u8,
    y: Option<usize>,
) -> Vec<(usize, i64)> {
    let pos = |s: &Seen| i64::from(s.wram[x]) + hi.map_or(0, |h| 256 * i64::from(s.wram[h]));
    let sign = if dir == 0x80 { 1 } else { -1 };
    let mut out = Vec::new();
    let mut i = 5;
    while i + 4 < seen.len() {
        let d = |k: usize| (pos(&seen[k]) - pos(&seen[k - 1])) * sign;
        let before: i64 = (i - 3..=i).map(d).sum();
        let ahead_ok = (i + 1..=i + 4).all(|k| (-1..=0).contains(&d(k)) && seen[k].pad & dir != 0);
        let standing =
            y.is_none_or(|y| (i - 3..=i + 2).all(|k| seen[k].wram[y] == seen[i].wram[y]));
        if before >= 2 && (i - 3..=i).all(|k| d(k) >= 0) && ahead_ok && standing {
            out.push((i, pos(&seen[i])));
            i += 30;
            continue;
        }
        i += 1;
    }
    out
}

/// The hitbox [x, y, w, h] from contacts on a tile grid: each stop against
/// a wall moving forward puts the box's front edge on a tile edge, each
/// stop moving back puts its back edge on one, a head hit puts its top on
/// one and standing puts its bottom on one. The grid is the largest of 32,
/// 16 and 8 on which each kind of contact agrees (most of them, at least
/// two, with no tie). The height is the bottom's place nearest the
/// sprites' height (`sz`).
// Each argument is one input the caller already holds; the command's own
// plumbing, with no group of them that belongs together.
#[allow(clippy::too_many_arguments)]
fn hitbox_of(
    runs: &[&[Seen]],
    x: usize,
    hi: Option<usize>,
    y: usize,
    fwd: u8,
    back: u8,
    sz: [i64; 2],
    box_dy: i64,
) -> std::result::Result<([i64; 4], i64), String> {
    let (mut f, mut b, mut stand, mut heads) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for r in runs {
        // A head hit: rising, then not, by 3 px a frame at once.
        for i in 2..r.len().saturating_sub(1) {
            let d0 = i64::from(r[i].wram[y]) - i64::from(r[i - 1].wram[y]);
            let d1 = i64::from(r[i + 1].wram[y]) - i64::from(r[i].wram[y]);
            if d0 < 0 && d1 >= 0 && d1 - d0 >= 3 {
                heads.push(i64::from(r[i].wram[y]));
            }
        }
        f.extend(stops_on(r, x, hi, fwd, Some(y)).into_iter().map(|s| s.1));
        b.extend(stops_on(r, x, hi, back, Some(y)).into_iter().map(|s| s.1));
        // Standing: y still while x moves (walking on the ground, not a
        // jump's top or a frozen death).
        for w in r.windows(8) {
            if w.iter().all(|s| s.wram[y] == w[0].wram[y]) && w[7].wram[x] != w[0].wram[x] {
                stand.push(i64::from(w[0].wram[y]));
            }
        }
    }
    f.sort_unstable();
    f.dedup();
    b.sort_unstable();
    b.dedup();
    stand.sort_unstable();
    stand.dedup();
    heads.sort_unstable();
    heads.dedup();
    if f.len() < 2 || b.len() < 2 || heads.is_empty() {
        return Err(format!(
            "{} forward and {} backward wall stops, {} head hits: not enough to place the box on the grid",
            f.len(),
            b.len(),
            heads.len()
        ));
    }
    if std::env::var_os("GP_TRACE_BLIND").is_some() {
        eprintln!("stops forward {f:?} back {b:?} stand {stand:?} heads {heads:?}");
    }
    // Each kind of contact's place on a grid of `t`: the residue most of
    // its contacts share, held by at least two of them and at least half,
    // with no other residue as common (a tie is a disagreement).
    // A stop at x 0 is the level's edge, not a wall, and is left out.
    let f: Vec<i64> = f.into_iter().filter(|v| *v > 0).collect();
    let b: Vec<i64> = b.into_iter().filter(|v| *v > 0).collect();
    let mode = |v: &[i64], t: i64| -> Option<i64> {
        let mut n: std::collections::BTreeMap<i64, usize> = Default::default();
        for a in v {
            *n.entry(a.rem_euclid(t)).or_default() += 1;
        }
        let top = n.values().copied().max()?;
        let mut best = n.iter().filter(|(_, c)| **c == top);
        let (r, c) = best.next()?;
        (best.next().is_none() && *c >= 2 && *c * 2 >= v.len()).then_some(*r)
    };
    let Some((t, rf, rb, rs, rh)) = [32i64, 16, 8].into_iter().find_map(|t| {
        Some((
            t,
            mode(&f, t)?,
            mode(&b, t)?,
            mode(&stand, t).or_else(|| (stand.len() == 1).then(|| stand[0].rem_euclid(t)))?,
            mode(&heads, t)?,
        ))
    }) else {
        return Err(format!(
            "the contacts don't agree on a tile grid (forward {f:?}, back {b:?}, standing {stand:?}, heads {heads:?})"
        ));
    };
    // Moving right, the box's right edge (x + hx + w) meets a tile edge;
    // moving back, its left edge (x + hx) does: the other way round when
    // forward is left.
    let (front, rear) = if fwd == 0x80 { (rf, rb) } else { (rb, rf) };
    let hx = (-rear).rem_euclid(t);
    let mut w = (-front).rem_euclid(t) - hx;
    while w <= 0 {
        w += t;
    }
    // A head hit puts the top (y + hy) on a tile edge, standing puts the
    // bottom (y + hy + h) on one.
    let _ = box_dy;
    let hy = {
        let r = (-rh).rem_euclid(t);
        if r > t / 2 { r - t } else { r }
    };
    let bottom = (-(rs + hy)).rem_euclid(t);
    let mut h = bottom;
    while h < sz[1] - t / 2 {
        h += t;
    }
    if w > sz[0] || h > sz[1] + t / 2 {
        return Err(format!(
            "the box from contacts ({w} by {h}) is bigger than the sprites"
        ));
    }
    Ok(([hx, hy, w, h], t))
}

/// The level's facts from the asset map: its grid and tileset assets (kind,
/// role, source), and the level's grid, tileset, size, tile size and solid
/// property. The tile size and solid property must agree with the contacts:
/// the grid's cells are `t` px, and the property must be set under the
/// player's box everywhere it stood and clear in the cells its box covered.
#[allow(clippy::too_many_arguments)]
fn level_of(
    rom: &[u8],
    map_path: &std::path::Path,
    probes: &[Vec<Seen>],
    path: &[Seen],
    x: usize,
    hi: Option<usize>,
    y: usize,
    hb: [i64; 4],
    t: i64,
    speed: Option<(usize, usize, usize)>,
) -> Result<(Vec<LevelFact>, Value)> {
    let text = fs::read_to_string(map_path)?;
    let map = gp_assets::AssetMap::parse(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let (files, _) = gp_assets::lift(rom, &map, &text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let raw: toml::Value = toml::from_str(&text)?;
    let source = raw
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let assets = raw
        .get("asset")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let by_format = |f: &str| {
        assets
            .iter()
            .find(|a| a.get("format").and_then(|v| v.as_str()) == Some(f))
            .cloned()
    };
    let grid = by_format("level-grid").context("the asset map has no level grid")?;
    let tileset = by_format("metatiles").context("the asset map has no metatiles")?;
    let name = |a: &toml::Value| {
        a.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned()
    };
    let (gname, tname) = (name(&grid), name(&tileset));
    let file = |n: &str| -> Result<toml::Value> {
        let f = files
            .iter()
            .find(|f| f.name == format!("{n}.toml"))
            .with_context(|| format!("no lifted {n}"))?;
        Ok(toml::from_str(std::str::from_utf8(&f.bytes)?)?)
    };
    let (lv, mt) = (file(&gname)?, file(&tname)?);
    let legend: Vec<char> = grid
        .get("legend")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .collect();
    let rows: Vec<Vec<usize>> = lv
        .get("rows")
        .and_then(|v| v.as_array())
        .context("the lifted grid has no rows")?
        .iter()
        .map(|r| {
            r.as_str()
                .unwrap_or("")
                .chars()
                .map(|c| legend.iter().position(|l| *l == c).unwrap_or(0))
                .collect()
        })
        .collect();
    let (w, h) = (rows.first().map_or(0, Vec::len), rows.len());
    let metas = mt
        .get("metatile")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let props: Vec<String> = tileset
        .get("property")
        .and_then(|v| v.as_array())
        .map(|ps| {
            ps.iter()
                .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let cell = |cx: i64, cy: i64| -> Option<usize> {
        if cx < 0 || cy < 0 || cx as usize >= w || cy as usize >= h {
            return None;
        }
        Some(rows[cy as usize][cx as usize])
    };
    let prop = |m: usize, p: &str| {
        metas
            .get(m)
            .and_then(|v| v.get(p))
            .and_then(|v| v.as_integer())
            .unwrap_or(0)
    };
    // Standing frames: y still and x moving, over 8 frames.
    let pos = |s: &Seen| i64::from(s.wram[x]) + hi.map_or(0, |hh| 256 * i64::from(s.wram[hh]));
    let mut under = Vec::new();
    let mut inside = Vec::new();
    for run in probes
        .iter()
        .map(|v| v.as_slice())
        .chain(std::iter::once(path))
    {
        for wdw in run.windows(8) {
            if !(wdw.iter().all(|s| s.wram[y] == wdw[0].wram[y]) && pos(&wdw[7]) != pos(&wdw[0])) {
                continue;
            }
            let (px, py) = (pos(&wdw[4]), i64::from(wdw[4].wram[y]));
            let (l, r) = (px + hb[0], px + hb[0] + hb[2] - 1);
            let (top, bottom) = (py + hb[1], py + hb[1] + hb[3]);
            under.push([l.div_euclid(t), r.div_euclid(t), bottom.div_euclid(t)]);
            for cy in top.div_euclid(t)..=(bottom - 1).div_euclid(t) {
                inside.push([l.div_euclid(t), r.div_euclid(t), cy]);
            }
        }
    }
    if under.len() < 20 {
        bail!("{} standing frames: too few to test the tiles", under.len());
    }
    // Under the feet, at least one of the cells the box spans is solid;
    // inside the box, none is.
    let fits = |p: &str| {
        let ok_under = under
            .iter()
            .filter(|u| (u[0]..=u[1]).any(|cx| cell(cx, u[2]).is_some_and(|m| prop(m, p) != 0)))
            .count();
        let ok_inside = inside
            .iter()
            .filter(|u| (u[0]..=u[1]).all(|cx| cell(cx, u[2]).is_none_or(|m| prop(m, p) == 0)))
            .count();
        (ok_under, ok_inside)
    };
    let scores: Vec<(String, (usize, usize))> =
        props.iter().map(|p| (p.clone(), fits(p))).collect();
    let good: Vec<&String> = scores
        .iter()
        .filter(|(_, (u, i))| *u * 100 >= under.len() * 98 && *i * 100 >= inside.len() * 98)
        .map(|(p, _)| p)
        .collect();
    // Each fact, with whether it comes from the asset map (true) or from
    // the contacts alone (false).
    let mut out: Vec<LevelFact> = Vec::new();
    let why = |s: &str| s.to_owned();
    for (n, kind) in [(&gname, "level-grid"), (&tname, "metatiles")] {
        out.push((
            format!("assets.{n}.kind"),
            json!(kind),
            why("the asset map's format for it"),
            true,
        ));
        out.push((
            format!("assets.{n}.from.source"),
            json!(source),
            why("the asset map's source"),
            true,
        ));
        out.push((
            format!("assets.{n}.from.asset"),
            json!(n),
            why("the asset map's asset"),
            true,
        ));
    }
    let cells_agree = under.len();
    out.push((
        "levels.main.grid".into(),
        json!(gname),
        why("the asset map's level grid"),
        true,
    ));
    out.push((
        "levels.main.tileset".into(),
        json!(tname),
        why("the asset map's metatiles"),
        true,
    ));
    out.push((
        "levels.main.size".into(),
        json!([w, h]),
        why("the lifted grid's width and height"),
        true,
    ));
    out.push((
        "levels.main.tile_size".into(),
        json!(t),
        format!("the contacts' grid ({cells_agree} standing frames)"),
        false,
    ));
    if good.len() == 1 {
        out.push((
            "levels.main.solid_property".into(),
            json!(good[0]),
            format!(
                "the asset map's property set under the player on {} standing frames, clear inside its box",
                under.len()
            ),
            true,
        ));
        // The grid the contacts confirm is what the level is laid out from,
        // and the metatiles are its tiles: the roles the model's level
        // reads them in (docs/model.md, "Asset slots").
        out.push((
            format!("assets.{gname}.role"),
            json!("layout"),
            why("the grid the contacts confirm the level is laid out from"),
            true,
        ));
        out.push((
            format!("assets.{tname}.role"),
            json!("tileset"),
            why("the tiles of the grid the contacts confirm"),
            true,
        ));
    }
    // Head hits: the frames a rise is cut to a new speed at once, not by
    // gravity, read from the game's speed bytes. The cell that cut it is the
    // one under the head point; the point is the offset that puts every hit
    // in a cell that stops heads and every other rising frame in none.
    let mut head_note = Value::Null;
    if let Some((vb, vf, yf)) = speed {
        let vy = |s: &Seen| i64::from(s.wram[vb] as i8) * 256 + i64::from(s.wram[vf]);
        let yy = |s: &Seen| i64::from(s.wram[y]) * 256 + i64::from(s.wram[yf]);
        // Rising frames, and the hits among them: rising before, then a
        // speed that isn't the gravity step from the one before.
        let mut rising: Vec<(i64, i64, bool, i64, usize, usize)> = Vec::new(); // (x, y whole, hit, speed after, run, frame)
        let runs: Vec<&[Seen]> = probes
            .iter()
            .map(|v| v.as_slice())
            .chain(std::iter::once(path))
            .collect();
        let mut grav: std::collections::BTreeMap<i64, usize> = Default::default();
        for r in &runs {
            for w in r.windows(3) {
                if vy(&w[0]) < 0 && vy(&w[1]) < 0 && vy(&w[2]) < 0 {
                    *grav.entry(vy(&w[1]) - vy(&w[0])).or_default() += 1;
                }
            }
        }
        let g = grav
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map_or(0, |(g, _)| g);
        for (ri, r) in runs.iter().enumerate() {
            for i in 1..r.len() {
                let (v0, v1) = (vy(&r[i - 1]), vy(&r[i]));
                if v0 >= 0 {
                    continue;
                }
                let hit = v1 > v0 + g;
                rising.push((pos(&r[i]), yy(&r[i]) >> 8, hit && v1 >= 0, v1, ri, i));
            }
        }
        let hits: Vec<&(i64, i64, bool, i64, usize, usize)> =
            rising.iter().filter(|h| h.2).collect();
        // Cells that stop a head: solid by the solid property or with a
        // nonzero value of another property.
        let other: Vec<&String> = props
            .iter()
            .filter(|p| good.first().is_none_or(|g| *g != *p))
            .collect();
        let stopper = |m: usize| {
            good.first().is_some_and(|sp| prop(m, sp) != 0) || other.iter().any(|p| prop(m, p) != 0)
        };
        let mut points = Vec::new();
        if hits.len() >= 3 {
            for px in 0..t {
                for py in -4..=4i64 {
                    let ok_hits = hits.iter().all(|h| {
                        cell((h.0 + px).div_euclid(t), (h.1 + py).div_euclid(t))
                            .is_some_and(stopper)
                    });
                    let ok_rest = rising.iter().filter(|h| !h.2).all(|h| {
                        cell((h.0 + px).div_euclid(t), (h.1 + py).div_euclid(t))
                            .is_none_or(|m| !stopper(m))
                    });
                    if ok_hits && ok_rest {
                        points.push([px, py]);
                    }
                }
            }
        }
        // The speed after a hit, the same for all.
        let after: std::collections::BTreeSet<i64> = hits.iter().map(|h| h.3).collect();
        head_note = json!({"hits": hits.len(), "rising": rising.len(), "points": points, "speeds_after": after, "gravity": g});
        // A point is stated only when one offset fits; the hit's speed only
        // when every hit leaves the same one.
        if points.len() == 1 && after.len() == 1 {
            let sp = *after.iter().next().expect("one");
            out.push(("entities.player.jump.head.point".into(), json!(points[0]), format!("the one offset that puts all {} head hits in a stopping cell and no other rising frame", hits.len()), false));
            // The cells that stopped heads without being solid: their kind,
            // named from what the hit does, and the property that marks it.
            let kinds: std::collections::BTreeSet<usize> = hits
                .iter()
                .filter_map(|h| {
                    cell(
                        (h.0 + points[0][0]).div_euclid(t),
                        (h.1 + points[0][1]).div_euclid(t),
                    )
                })
                .filter(|m| good.first().is_none_or(|sp| prop(*m, sp) == 0))
                .collect();
            if let (Some(&m), 1) = (kinds.iter().next(), kinds.len()) {
                let marks: Vec<&&String> = other.iter().filter(|p| prop(m, p) != 0).collect();
                if marks.len() == 1 {
                    let kp = marks[0].as_str();
                    let val = prop(m, kp);
                    // What the block becomes: the metatile whose value the
                    // RAM grid byte takes at the hit is read in the next step;
                    // until then, only the hit's speed is stated.
                    out.push((
                        "levels.main.kind_property".into(),
                        json!(kp),
                        "the property set on the cells that stop heads without being solid".into(),
                        true,
                    ));
                    out.push((
                        format!("levels.main.tile_kinds.{val}"),
                        json!("coin_block"),
                        "reading: named from outcome (a head hit from below stops the rise there)"
                            .into(),
                        true,
                    ));
                    out.push((
                        "entities.player.jump.head.hits".into(),
                        json!([{"tile": "coin_block", "speed": sp / 256}]),
                        format!("every head hit leaves speed {sp}/256"),
                        false,
                    ));
                }
            }
        }
    }
    Ok((
        out,
        json!({"head": head_note, "standing": under.len(), "inside": inside.len(), "properties": scores.iter().map(|(p, (u, i))| json!({"name": p, "under": u, "inside": i})).collect::<Vec<_>>()}),
    ))
}

/// One traced run: each frame's writes and reads by routine, its RAM and
/// the PPU's scroll.
struct Traced {
    writes: Vec<Vec<gp_events::activity::Write>>,
    reads: Vec<Vec<(u16, u32)>>,
    ram: Vec<Vec<u8>>,
    scroll: Vec<u16>,
}

fn trace_run(bytes: &[u8], power_on: gp_evidence::manifest::PowerOn, text: &str) -> Result<Traced> {
    let log = gp_evidence::input_log::InputLog::parse(text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut nes = gp_nes::Nes::power_on(bytes, power_on)?;
    let layout = gp_events::Layout::of(nes.bus())?;
    let mut act = gp_events::activity::Activity::watching(
        layout,
        &gp_events::Bits::new(0),
        std::collections::BTreeSet::new(),
    );
    act.reads = true;
    let mut col = gp_events::Collector::with_activity(layout, false, act);
    col.start(nes.bus_mut_for_observers());
    let mut t = Traced {
        writes: Vec::new(),
        reads: Vec::new(),
        ram: Vec::new(),
        scroll: Vec::new(),
    };
    for f in &log.frames {
        if f.reset {
            col.will_reset();
        }
        nes.step_with(*f, |bus| col.before(bus))?;
        let fa = col.end_frame_activity(nes.bus()).context("activity")?;
        t.writes.push(fa.ram);
        t.reads.push(fa.reads);
        t.ram.push(nes.wram()[..0x800].to_vec());
        let sc = &nes.bus().ppu.scroll;
        t.scroll
            .push((sc.t & 31) * 8 + u16::from(sc.fine_x as u8 & 7) + ((sc.t >> 10) & 1) * 256);
    }
    Ok(t)
}

/// The order the game's routines run in during play, over traced runs from
/// `from` on (frames where the player's bytes change count as in play),
/// with each routine's systems by what it touches:
///
/// - player: writes the player's position;
/// - enemies: writes another mover (a byte, not the player's, that moves
///   by at most 4 a frame on many frames);
/// - camera: writes a byte equal to the scroll's low byte (that frame or
///   the next) on 9 frames in 10;
/// - contact: reads the player's position and a mover, and writes neither;
/// - counters: writes a byte that only ever steps by one the same way;
/// - rare: writes a byte that changes on only a few frames (rules,
///   entity states, spawn and exits, which writes alone can't tell apart).
fn system_trace(runs: &[Traced], from: usize, player: &[usize]) -> Value {
    let in_play =
        |t: &Traced, i: usize| i > 0 && player.iter().any(|&b| t.ram[i][b] != t.ram[i - 1][b]);
    let frames: Vec<(usize, usize)> = runs
        .iter()
        .enumerate()
        .flat_map(|(r, t)| {
            (from.min(t.ram.len())..t.ram.len())
                .filter(move |&i| in_play(t, i))
                .map(move |i| (r, i))
        })
        .collect();
    if frames.len() < 60 {
        return json!({"error": format!("{} frames in play: too few", frames.len())});
    }
    let total = frames.len();
    let delta = |t: &Traced, i: usize, b: usize| {
        ((i32::from(t.ram[i][b]) - i32::from(t.ram[i - 1][b]) + 384) % 256) - 128
    };
    let byte_set =
        |keep: &dyn Fn(usize) -> bool| -> Vec<usize> { (0..0x800).filter(|&b| keep(b)).collect() };
    let changes = |b: usize| {
        frames
            .iter()
            .filter(|&&(r, i)| runs[r].ram[i][b] != runs[r].ram[i - 1][b])
            .count()
    };
    let lag = |l: usize| {
        byte_set(&|b| {
            let eq = frames
                .iter()
                .filter(|&&(r, i)| {
                    let t = &runs[r];
                    i + l < t.ram.len() && u16::from(t.ram[i][b]) == t.scroll[i + l] & 0xFF
                })
                .count();
            eq * 10 >= total * 9 && changes(b) > 20
        })
    };
    let (camera0, camera1) = (lag(0), lag(1));
    let camera: Vec<usize> = camera0.iter().chain(&camera1).copied().collect();
    let counter = byte_set(&|b| {
        let d: Vec<i32> = frames
            .iter()
            .map(|&(r, i)| delta(&runs[r], i, b))
            .filter(|d| *d != 0)
            .collect();
        d.len() >= 20 && (d.iter().all(|v| *v == 1) || d.iter().all(|v| *v == -1))
    });
    let movers = byte_set(&|b| {
        if player.contains(&b)
            || camera.contains(&b)
            || counter.contains(&b)
            || (0x100..0x300).contains(&b)
        {
            return false;
        }
        let d: Vec<i32> = frames.iter().map(|&(r, i)| delta(&runs[r], i, b)).collect();
        let moving = d.iter().filter(|v| **v != 0).count();
        moving * 4 >= total && d.iter().filter(|v| v.abs() > 4).count() * 20 <= moving
    });
    let rare = byte_set(&|b| {
        let c = changes(b);
        (1..=8).contains(&c) && !(0x100..0x300).contains(&b)
    });
    #[derive(Default)]
    struct Row {
        ranks: Vec<usize>,
        writes: std::collections::BTreeSet<&'static str>,
        reads_player: bool,
        reads_mover: bool,
    }
    let mut by: std::collections::BTreeMap<u32, Row> = Default::default();
    for &(r, i) in &frames {
        let t = &runs[r];
        let mut seen: Vec<u32> = Vec::new();
        for w in &t.writes[i] {
            if w.engine || (0x100..0x300).contains(&usize::from(w.addr)) {
                continue;
            }
            if !seen.contains(&w.routine) {
                seen.push(w.routine);
            }
            let a = usize::from(w.addr);
            let e = by.entry(w.routine).or_default();
            for (set, name) in [
                (&player.to_vec(), "player"),
                (&movers, "enemies"),
                (&camera, "camera"),
                (&counter, "counters"),
                (&rare, "rare"),
            ] {
                if set.contains(&a) {
                    e.writes.insert(name);
                }
            }
        }
        for (rank, rt) in seen.iter().enumerate() {
            by.entry(*rt).or_default().ranks.push(rank);
        }
        for &(a, rt) in &t.reads[i] {
            let a = usize::from(a);
            let e = by.entry(rt).or_default();
            e.reads_player |= player.contains(&a);
            e.reads_mover |= movers.contains(&a);
        }
    }
    let mut rows: Vec<Value> = by
        .into_iter()
        .filter(|(_, row)| !row.ranks.is_empty())
        .map(|(rt, mut row)| {
            row.ranks.sort_unstable();
            let mut sys: Vec<&str> = row.writes.iter().copied().filter(|s| *s != "rare").collect();
            if row.reads_player && row.reads_mover && !row.writes.contains("player") && !row.writes.contains("enemies") {
                sys.push("contact");
            }
            if row.writes.contains("rare") {
                sys.push("rare state");
            }
            json!({"routine": format!("{rt:#06x}"), "frames": row.ranks.len(), "median_rank": row.ranks[row.ranks.len() / 2],
                   "systems": sys, "reads_player": row.reads_player, "reads_mover": row.reads_mover})
        })
        .collect();
    rows.sort_by_key(|v| v["median_rank"].as_u64().unwrap_or(0));
    let hex = |v: &[usize]| v.iter().map(|a| format!("${a:04X}")).collect::<Vec<_>>();
    json!({"frames": total, "camera_lag0": hex(&camera0), "camera_lag1": hex(&camera1), "counters": hex(&counter),
           "movers": hex(&movers), "rare": hex(&rare), "routines": rows})
}

/// Probes from standing places along a path: from every 8th frame where
/// the player has stood still for 4 frames, each of `pads` held 150 frames.
fn path_probes(
    bytes: &[u8],
    power_on: gp_evidence::manifest::PowerOn,
    text: &str,
    y: usize,
    pads: &[u8],
) -> Result<Vec<Vec<Seen>>> {
    let log = gp_evidence::input_log::InputLog::parse(text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut nes = gp_nes::Nes::power_on(bytes, power_on)?;
    nes.set_video(false);
    let mut ys: Vec<u8> = Vec::new();
    let mut starts = Vec::new();
    for (i, f) in log.frames.iter().enumerate() {
        nes.step(*f)?;
        ys.push(nes.wram()[y]);
        let n = ys.len();
        if i % 8 == 0 && n >= 4 && ys[n - 4..].iter().all(|v| *v == ys[n - 1]) {
            starts.push((nes.snapshot()?, nes.frame(), nes.held()));
        }
    }
    let mut out = Vec::new();
    for (snap, frame, held) in starts.iter().take(300) {
        for &pad in pads {
            nes.restore(snap, *frame, *held)?;
            let mut v = Vec::new();
            for _ in 0..150 {
                nes.step(InputFrame {
                    polled: true,
                    pads: [pad, 0, 0, 0],
                    reset: false,
                })?;
                v.push(Seen {
                    wram: nes.wram()[..0x800].to_vec(),
                    oam: nes.oam(),
                    pad,
                });
            }
            out.push(v);
        }
    }
    Ok(out)
}

/// The byte that says the player stands: nonzero on frames it walks on the
/// ground (y still over 5 frames while x moves) and zero in the air (y moving
/// on the frames around it), on 99 frames in 100 of each. None unless
/// exactly one byte does.
fn on_ground_byte(runs: &[&[Seen]], y: usize, x: usize) -> Option<usize> {
    let (mut stand, mut air) = (Vec::new(), Vec::new());
    for r in runs {
        for i in 2..r.len().saturating_sub(2) {
            let ys: Vec<u8> = (i - 2..=i + 2).map(|k| r[k].wram[y]).collect();
            // Standing is walking on the ground: y still while x moves, so a
            // frozen screen (a death, the clear) isn't counted.
            if ys.iter().all(|v| *v == ys[0]) && r[i + 2].wram[x] != r[i - 2].wram[x] {
                stand.push((*r, i));
            } else if ys.windows(2).all(|w| w[0] != w[1]) {
                air.push((*r, i));
            }
        }
    }
    if stand.len() < 50 || air.len() < 50 {
        return None;
    }
    if std::env::var_os("GP_TRACE_BLIND").is_some() {
        let mut sc: Vec<(usize, usize, usize)> = (0..0x800usize)
            .map(|b| {
                let s = stand.iter().filter(|(r, i)| r[*i].wram[b] != 0).count();
                let a = air.iter().filter(|(r, i)| r[*i].wram[b] == 0).count();
                (
                    s * 1000 / stand.len() + a * 1000 / air.len(),
                    b,
                    s * 1000 / stand.len(),
                )
            })
            .collect();
        sc.sort_unstable();
        eprintln!(
            "on_ground candidates (score, byte, stand per mille): {:?} of {} stand, {} air",
            &sc[sc.len() - 5..],
            stand.len(),
            air.len()
        );
    }
    let found: Vec<usize> = (0..0x800usize)
        .filter(|&b| b != y && !(0x100..0x300).contains(&b))
        .filter(|&b| {
            let s = stand.iter().filter(|(r, i)| r[*i].wram[b] != 0).count();
            let a = air.iter().filter(|(r, i)| r[*i].wram[b] == 0).count();
            s * 100 >= stand.len() * 99 && a * 100 >= air.len() * 99
        })
        .collect();
    (found.len() == 1).then(|| found[0])
}

/// A whole or a quarter, eighth or sixteenth as the model writes a fixed
/// point number: `1`, `-5`, `"0.25"`.
fn fixed(v: f64) -> Value {
    let q = (v * 16.0).round() / 16.0;
    if q.fract() == 0.0 {
        json!(q as i64)
    } else {
        json!(format!("{q}"))
    }
}

/// The steady speed across while `pad` is held: the most common move a
/// frame over the run's second half, ignoring frames with no move (a wall).
fn speed_across(seen: &[Seen], x: usize, hi: Option<usize>) -> Option<f64> {
    let pos = |s: &Seen| f64::from(s.wram[x]) + hi.map_or(0.0, |h| 256.0 * f64::from(s.wram[h]));
    let half = &seen[8.min(seen.len())..];
    let moves: Vec<f64> = half
        .windows(2)
        .map(|w| {
            let d = pos(&w[1]) - pos(&w[0]);
            if hi.is_none() {
                (d + 384.0) % 256.0 - 128.0
            } else {
                d
            }
        })
        .filter(|d| *d != 0.0)
        .collect();
    if moves.len() < 8 {
        return None;
    }
    // Speeds with fractions alternate (1, 2, 1, 2): the mean over the
    // moves is the speed.
    Some(moves.iter().sum::<f64>() / moves.len() as f64)
}

/// A jump from standing: its takeoff speed and its gravity, by a least
/// squares fit of y over the frames in the air (y = y0 + v t + g t(t-1)/2),
/// and the fastest fall as the largest move down held 3 frames running.
fn jump_of(seen: &[Seen], y: usize) -> Option<(f64, f64, Option<i64>, usize, usize)> {
    let ys: Vec<f64> = seen.iter().map(|s| f64::from(s.wram[y])).collect();
    let y0 = ys[0];
    let up = ys.iter().position(|v| *v < y0)?;
    let back = up
        + ys[up..]
            .iter()
            .position(|v| *v >= y0)
            .unwrap_or(ys.len() - up);
    let t0 = up - 1;
    let pts: Vec<(f64, f64)> = (t0..back)
        .map(|i| ((i - t0) as f64, ys[i] - ys[t0]))
        .collect();
    if pts.len() < 6 {
        return None;
    }
    // Two unknowns, v and g, over terms t and t(t-1)/2.
    let (mut s11, mut s12, mut s22, mut b1, mut b2) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for &(t, d) in &pts {
        let (p, q) = (t, t * (t - 1.0) / 2.0);
        s11 += p * p;
        s12 += p * q;
        s22 += q * q;
        b1 += p * d;
        b2 += q * d;
    }
    let det = s11 * s22 - s12 * s12;
    if det.abs() < 1e-9 {
        return None;
    }
    let v0 = (b1 * s22 - b2 * s12) / det;
    let g0 = (s11 * b2 - s12 * b1) / det;
    // The fit is near; the game keeps a fraction and drops it on screen,
    // and adds gravity before or after it moves. Each takeoff speed and
    // gravity near the fit, in 16ths and 64ths, with each order and each
    // starting fraction, is played forward and kept if it gives every
    // whole y of the jump's rise; the coarsest that does wins.
    let rise_end = (t0..back)
        .find(|&i| i + 1 < back && ys[i + 1] >= ys[i])
        .unwrap_or(back);
    let target: Vec<f64> = (t0..=rise_end).map(|i| ys[i] - ys[t0]).collect();
    if std::env::var_os("GP_TRACE_BLIND").is_some() {
        eprintln!("jump ys: {:?}\nrise target: {:?}", &ys[t0..back], target);
    }
    let mut best: Option<(u32, f64, f64)> = None;
    for vi in -24..=24 {
        let v = (v0 * 16.0).round() / 16.0 + f64::from(vi) / 16.0;
        for gi in -16..=16 {
            let g = (g0 * 64.0).round() / 64.0 + f64::from(gi) / 64.0;
            if g <= 0.0 {
                continue;
            }
            let fits = (0..2).any(|order| {
                (0..16).any(|f| {
                    let mut pos = f64::from(f) / 16.0;
                    let mut vel = v;
                    target.iter().enumerate().all(|(t, want)| {
                        if t > 0 {
                            if order == 0 {
                                pos += vel;
                                vel += g;
                            } else {
                                vel += g;
                                pos += vel;
                            }
                        }
                        (pos.floor() - (f64::from(f) / 16.0).floor()) == *want
                    })
                })
            });
            if fits {
                // Coarseness: the smallest power of two denominator of both.
                let den = |x: f64| {
                    (0..=6)
                        .find(|k| (x * f64::from(1u32 << k)).fract() == 0.0)
                        .unwrap_or(7)
                };
                let c = den(v).max(den(g));
                if best.is_none_or(|(bc, _, _)| c < bc) {
                    best = Some((c, v, g));
                }
            }
        }
    }
    let (v, g) = best.map_or((v0, g0), |(_, v, g)| (v, g));
    let downs: Vec<i64> = ys[t0..back]
        .windows(2)
        .map(|w| (w[1] - w[0]) as i64)
        .collect();
    let max_fall = downs
        .windows(3)
        .filter(|w| w[0] > 0 && w[0] == w[1] && w[1] == w[2])
        .map(|w| w[0])
        .max();
    Some((v, g, max_fall, t0, back))
}

/// The box the sprites at the player's place make: its width and height.
fn size_of(s: &Seen, y: usize, box_dy: i64) -> Option<[i64; 2]> {
    let top = i64::from(s.wram[y]) + box_dy;
    let mut cells: Vec<[i64; 4]> = s
        .oam
        .chunks(4)
        .filter(|e| e[0] < 0xEF)
        .map(|e| [i64::from(e[3]), i64::from(e[0]) + 1, 8, 8])
        .collect();
    // Merge touching cells, then keep the box whose top is the player's.
    loop {
        let mut merged = false;
        'o: for i in 0..cells.len() {
            for j in i + 1..cells.len() {
                let (a, c) = (cells[i], cells[j]);
                if a[0] <= c[0] + c[2]
                    && c[0] <= a[0] + a[2]
                    && a[1] <= c[1] + c[3]
                    && c[1] <= a[1] + a[3]
                {
                    let x0 = a[0].min(c[0]);
                    let y0 = a[1].min(c[1]);
                    cells[i] = [
                        x0,
                        y0,
                        (a[0] + a[2]).max(c[0] + c[2]) - x0,
                        (a[1] + a[3]).max(c[1] + c[3]) - y0,
                    ];
                    cells.swap_remove(j);
                    merged = true;
                    break 'o;
                }
            }
        }
        if !merged {
            break;
        }
    }
    cells
        .iter()
        .find(|b| (b[1] - top).abs() <= 1)
        .map(|b| [b[2], b[3]])
}

pub fn run(a: &BlindArgs, paths: &Paths) -> Result<Outcome> {
    let bytes = fs::read(&a.rom).with_context(|| format!("no ROM at {}", a.rom.display()))?;
    // Blind discovery and every measure here play gp-nes and read the NES's
    // RAM and sprite table; an arcade board's counterparts aren't wired yet.
    crate::original::nes_only(
        crate::original::Platform::of_file(&bytes),
        "gp model blind (discovery plays gp-nes and reads its RAM and sprite table)",
    )?;
    let img = gp_evidence::source::identify_nes(&bytes)?;
    let rom_ref = format!("rom:{}", &img.sha256[..12]);
    let d = crate::farm::blind::discover(&bytes, paths)?;
    let snap = d.nes.snapshot()?;
    let mut st = Start {
        frame: d.nes.frame(),
        held: d.nes.held(),
        nes: d.nes,
        snap,
        x: usize::from(d.x),
        y: usize::from(d.y),
        jump: d.jump,
        run: d.run,
        forward: if d.right { 0x80 } else { 0x40 },
        box_dy: d.box_dy,
    };
    let at = d.log.len();
    let start_x = i64::from(st.nes.wram()[st.x]);
    let start_y = i64::from(st.nes.wram()[st.y]);
    let power_on = d.power_on;

    // Each measure, with the frames it was taken over, for its evidence.
    // (path, value, evidence, how it was found)
    let mut facts: Vec<(String, Value, String, &str)> = Vec::new();
    const PROBE: &str = "scripted probe";
    const LAYA: &str = "Laya play";
    let mut x_hi_by = PROBE;
    let mut y_hi_by = PROBE;
    const FITTED: &str = "fitted";
    // Read from the bench's asset map (the level's grid and tileset), not
    // found blind.
    const MAP: &str = "from asset map";
    let note = |what: &str, from: usize, to: usize| {
        format!("{rom_ref} frames {}-{}: {what}", at + from, at + to)
    };
    facts.push((
        "input.device".into(),
        json!("nes-pad"),
        note("the game reads a standard pad", 0, 0),
        PROBE,
    ));
    facts.push((
        "input.actions.jump".into(),
        json!(button_name(st.jump)),
        note("this button lifts the player", 0, 0),
        PROBE,
    ));
    facts.push((
        "input.actions.run".into(),
        json!(button_name(st.run)),
        note("this button speeds the player up", 0, 0),
        PROBE,
    ));
    facts.push((
        "input.actions.right".into(),
        json!("right"),
        note("the d-pad moves the player across", 0, 0),
        PROBE,
    ));
    facts.push((
        "input.actions.left".into(),
        json!("left"),
        note("the d-pad moves the player across", 0, 0),
        PROBE,
    ));
    facts.push((
        "entities.player.role".into(),
        json!("player"),
        note("the sprite that follows the pad", 0, 0),
        PROBE,
    ));

    // A long run forward that jumps whenever the player stops across (a
    // wall) and tries a later or earlier takeoff after a death: x's high
    // byte, if it has one, steps when the low byte wraps.
    let long = st.forward_run(1200)?;
    let mut hi = high_byte(&long, st.x, st.y);
    let mut y_hi = high_byte(&long, st.y, st.x);
    // The Laya search over the level: a path far enough for x to wrap,
    // and what the run learned on the way (walls, pits, the level end).
    let mut played = Value::Null;
    let mut path_seen: Vec<Seen> = Vec::new();
    let mut path_text = String::new();
    if a.play_rounds > 0 {
        let out_dir = a.model.with_extension("play");
        let o = crate::farm::blind::Opts {
            decide: a.decide.clone(),
            rounds: a.play_rounds,
            top: 3,
            unsure: 0.4,
            frontier: 256,
            seed: 1,
            // No clock: rounds alone end the search.
            seconds: f64::INFINITY,
            levels: 1,
            session: None,
        };
        match crate::farm::blind::farm(&a.rom, &o, paths, &out_dir) {
            Ok(v) => {
                played = v;
                if let Ok(text) = fs::read_to_string(out_dir.join("path.log")) {
                    let path = replay(&bytes, power_on, &text)?;
                    if hi.is_none() {
                        hi = high_byte(&path, st.x, st.y);
                        x_hi_by = LAYA;
                    }
                    if y_hi.is_none() {
                        y_hi = high_byte(&path, st.y, st.x);
                        y_hi_by = LAYA;
                    }
                    path_seen = path;
                    path_text = text;
                }
            }
            // Without the search's path x's high byte, the level and the
            // contacts go unfound, and the mapping would state x as its
            // low byte alone: a wrong fact, where a failure is the truth.
            Err(e) => bail!(
                "the search over the level failed: {e:#}. `--decide uniform` searches without a decision server; `--play-rounds 0` skips the search"
            ),
        }
    }

    let walk = st.play(&[(st.forward, 120)])?;
    if std::env::var_os("GP_TRACE_BLIND").is_some() {
        let left = st.play(&[(if st.forward == 0x80 { 0x40 } else { 0x80 }, 120)])?;
        let ls2: Vec<String> = left.iter().map(|s| s.wram[st.x].to_string()).collect();
        eprintln!("left x: {}", ls2.join(" "));
        let xs: Vec<String> = walk.iter().map(|s| s.wram[st.x].to_string()).collect();
        let ls: Vec<String> = long
            .iter()
            .map(|s| format!("{}/{}", s.wram[st.x], s.wram[st.y]))
            .collect();
        eprintln!("walk x: {}\nlong x: {}", xs.join(" "), ls.join(" "));
    }
    if let Some(v) = speed_across(&walk, st.x, hi) {
        facts.push((
            "entities.player.run.walk_speed".into(),
            fixed(v.abs()),
            note("speed across with only the d-pad held", 60, 120),
            PROBE,
        ));
    }
    let runs = st.play(&[(st.forward | st.run, 120)])?;
    if let Some(v) = speed_across(&runs, st.x, hi) {
        facts.push((
            "entities.player.run.run_speed".into(),
            fixed(v.abs()),
            note("speed across with run held", 60, 120),
            PROBE,
        ));
        facts.push((
            "entities.player.run.run_action".into(),
            json!("run"),
            note("run held", 0, 120),
            PROBE,
        ));
    }
    let jumped = st.play(&[(st.jump, 50), (0, 100)])?;
    let tapped = st.play(&[(st.jump, 4), (0, 100)])?;
    let vyb = speed_byte(&[&jumped, &tapped], st.y);

    let mut rule = Value::Null;
    if let Some(vb) = vyb {
        let tr = |v: &[Seen]| {
            (
                v.iter().map(|s| s.wram.clone()).collect::<Vec<_>>(),
                v.iter().map(|s| s.pad).collect::<Vec<_>>(),
            )
        };
        let mut traces = vec![tr(&jumped), tr(&tapped), tr(&long)];
        if !path_seen.is_empty() {
            traces.push(tr(&path_seen));
        }
        rule = crate::rule_refine::fit_traces(traces, st.y, vb);
        if std::env::var_os("GP_TRACE_BLIND").is_some() {
            eprintln!("vy byte {vb:#04x}; rule {rule}");
        }
    }
    // The jump, from the game's own speed bytes (`gp rule jump`'s fitter):
    // the takeoff speed in the rebuild's order, and gravity. One gravity
    // is stated only when holding the button makes no difference (a held
    // and a tapped jump rise the same).
    let same_rise = {
        let rise = |v: &[Seen]| -> Vec<u8> {
            let ys: Vec<u8> = v.iter().map(|s| s.wram[st.y]).collect();
            let top = ys
                .iter()
                .enumerate()
                .min_by_key(|(_, y)| **y)
                .map_or(0, |(i, _)| i);
            ys[..=top].to_vec()
        };
        rise(&jumped) == rise(&tapped)
    };
    if let Some((_, _, max_fall, t0, t1)) = jump_of(&jumped, st.y) {
        facts.push((
            "entities.player.jump.action".into(),
            json!("jump"),
            note("jump pressed", t0, t1),
            PROBE,
        ));
        let take = rule["takeoff"].as_object().and_then(|m| {
            (m.len() == 1)
                .then(|| m.values().next().and_then(Value::as_i64))
                .flatten()
        });
        let grav = rule["gravity"].as_object().and_then(|m| {
            let vals: std::collections::BTreeSet<i64> =
                m.values().filter_map(Value::as_i64).collect();
            (vals.len() == 1).then(|| vals.into_iter().next()).flatten()
        });
        if let (Some(v0), Some(g)) = (take, grav)
            && same_rise
            && rule["speed_fraction"].is_number()
        {
            let speed = if rule["gravity_first"] == json!(true) {
                v0 - g
            } else {
                v0
            };
            facts.push((
                "entities.player.jump.speed".into(),
                fixed(speed as f64 / 256.0),
                note(
                    "takeoff speed, from the game's speed bytes in the rebuild's order",
                    t0,
                    t1,
                ),
                FITTED,
            ));
            facts.push((
                "entities.player.jump.gravity".into(),
                fixed(g as f64 / 256.0),
                note("gravity, from the game's speed bytes", t0, t1),
                FITTED,
            ));
        }
        if let Some(m) = max_fall {
            facts.push((
                "entities.player.jump.max_fall".into(),
                json!(m),
                note("the fastest fall, held 3 frames", t0, t1),
                FITTED,
            ));
        }
    }
    let mut systems_trace = Value::Null;
    if !path_text.is_empty() {
        let mut player = vec![st.x, st.y];
        player.extend(hi);
        player.extend(y_hi);
        // Laya's path, and probe searches toward a touch and a head hit,
        // so contacts, deaths and state changes happen in play.
        let mut texts = vec![path_text.clone()];
        let pdir = a.model.with_extension("probes");
        let pf = pdir.join("probes.json");
        fs::create_dir_all(&pdir)?;
        fs::write(
            &pf,
            // No "seconds": rounds alone end each probe, never a clock.
            r#"[{"kind": "touch", "id": "touch"}, {"kind": "head_hit", "id": "head"}]"#,
        )?;
        let o = crate::farm::blind::Opts {
            decide: a.decide.clone(),
            rounds: a.play_rounds / 2,
            top: 3,
            unsure: 0.4,
            frontier: 256,
            seed: 1,
            seconds: f64::INFINITY,
            levels: 1,
            session: None,
        };
        if a.play_rounds > 0
            && let Ok(v) = crate::farm::blind::probes(&a.rom, &o, paths, &pdir, &pf)
            && let Some(dir) = v["run_dir"].as_str()
        {
            for id in ["touch", "head"] {
                let d = Path::new(dir).join("probes").join(id);
                if let Ok(t) = fs::read_to_string(d.join("path.log"))
                    .or_else(|_| fs::read_to_string(d.join("clip.log")))
                {
                    texts.push(t);
                }
            }
        }
        let runs: Vec<Traced> = texts
            .iter()
            .filter_map(|t| trace_run(&bytes, power_on, t).ok())
            .collect();
        systems_trace = system_trace(&runs, at, &player);
        systems_trace["runs"] = json!(runs.len());
    }
    let mut contacts_left: Vec<Vec<Seen>> = Vec::new();
    let mut hitbox_open: Option<String> = None;
    let mut hitbox: Option<([i64; 4], i64)> = None;
    if let Some(sz) = walk.first().and_then(|s| size_of(s, st.y, st.box_dy)) {
        facts.push((
            "entities.player.size".into(),
            json!(sz),
            note("the player's sprite box", 0, 0),
            PROBE,
        ));
        // The box it collides with, from contacts: where it stops against
        // walls from each side and where its head meets a ceiling, on the
        // level's tile grid.
        let mut contacts: Vec<&[Seen]> = vec![&long];
        if !path_seen.is_empty() {
            contacts.push(&path_seen);
        }
        let back = if st.forward == 0x80 { 0x40 } else { 0x80 };
        // More contacts: from places along the Laya path where the player
        // stands, walk back, walk forward and jump straight up.
        if !path_text.is_empty() {
            contacts_left = path_probes(
                &bytes,
                power_on,
                &path_text,
                st.y,
                &[
                    back,
                    back | st.run,
                    st.forward,
                    st.jump,
                    st.jump | st.forward,
                    st.jump | st.forward | st.run,
                ],
            )?;
        }
        for p in &contacts_left {
            contacts.push(p);
        }
        match hitbox_of(&contacts, st.x, hi, st.y, st.forward, back, sz, st.box_dy) {
            Ok((hb, grid_t)) => {
                hitbox = Some((hb, grid_t));
                facts.push((
                    "entities.player.tile_collision".into(),
                    json!(true),
                    note("stops against walls and stands on the level's tiles", 0, 0),
                    PROBE,
                ));
                for (k, v) in [("x", hb[0]), ("y", hb[1]), ("w", hb[2]), ("h", hb[3])] {
                    facts.push((format!("entities.player.hitbox.{k}"), json!(v), note("from wall stops on both sides and the standing height, on the tile grid", 0, 0), PROBE));
                }
            }
            Err(why) => hitbox_open = Some(why),
        }
    }
    // Air control: a jump straight up, then the d-pad held in the air.
    {
        let up = st.play(&[(st.jump, 4), (0, 24)])?;
        let steer = st.play(&[(st.jump, 4), (0, 4), (st.forward, 20)])?;
        let in_air = (8..24).all(|i| up[i].wram[st.y] != up[0].wram[st.y]);
        if in_air {
            let moved = steer[23].wram[st.x] != up[23].wram[st.x];
            facts.push((
                "entities.player.run.air_control".into(),
                json!(moved),
                note("the d-pad held during a jump straight up", 8, 24),
                PROBE,
            ));
        }
    }

    // The level, from the asset map's grid and tileset, tested against the
    // contacts: the property that is set under every place the player
    // stood and clear where its box was is the solid one.
    let mut level_note = Value::Null;
    if let (Some(map_path), Some((hb, grid_t))) = (&a.assets, hitbox) {
        match level_of(
            &bytes,
            map_path,
            &contacts_left,
            &path_seen,
            st.x,
            hi,
            st.y,
            hb,
            grid_t,
            match (
                vyb,
                rule["speed_fraction"].as_u64(),
                rule["y_fraction"].as_u64(),
            ) {
                (Some(v), Some(vf), Some(yf)) => Some((v, vf as usize, yf as usize)),
                _ => None,
            },
        ) {
            Ok((fs_, note_)) => {
                for (p, v, why, by_map) in fs_ {
                    facts.push((p, v, why, if by_map { MAP } else { PROBE }));
                }
                level_note = note_;
            }
            Err(e) => level_note = json!({"error": format!("{e:#}")}),
        }
        // The level's end: where Laya's path stops with the game frozen (the
        // player's x the same for its last 60 frames, after moving).
        if path_seen.len() > 120 {
            let pos =
                |s: &Seen| i64::from(s.wram[st.x]) + hi.map_or(0, |h| 256 * i64::from(s.wram[h]));
            let n = path_seen.len();
            let end = pos(&path_seen[n - 1]);
            if path_seen[n - 60..].iter().all(|s| pos(s) == end) && pos(&path_seen[n - 120]) != end
            {
                facts.push((
                    "levels.main.goal.x".into(),
                    json!(end),
                    note("where Laya's path ends, the game frozen by the clear", 0, 0),
                    LAYA,
                ));
            }
        }
        let spawn_note = note("where play starts", 0, 0);
        // One spawn, the player's: a list of one table, so the level's
        // spawns stay a list.
        facts.push((
            "levels.main.spawn".into(),
            json!([{"entity": "player", "x": start_x, "y": start_y}]),
            spawn_note,
            PROBE,
        ));
    }

    // model.toml: the header it has, the facts, and their evidence.
    fs::create_dir_all(&a.model)?;
    let mf = a.model.join(gp_model::MODEL_FILE);
    let mut doc: toml::Value = match fs::read_to_string(&mf) {
        Ok(t) => toml::from_str(&t)?,
        Err(_) => toml::Value::Table(Default::default()),
    };
    let model_name = doc
        .get("model")
        .and_then(|m| m.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("blind")
        .to_owned();
    let source = doc
        .get("model")
        .and_then(|m| m.get("source"))
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_owned();
    set(
        &mut doc,
        &format!("model.extensions.{}", gp_model::platformer::NAME),
        json!(gp_model::platformer::VERSION),
    )?;
    for (path, v, _, _) in &facts {
        set(&mut doc, path, v.clone())?;
    }
    // The order the systems run in. Measured (the routine trace): the
    // player's movement before the other movers', the camera last. The
    // rest is unobservable in the current clips and follows the order
    // docs/model.md lists the systems in. The header has no evidence entry
    // (docs/model.md), so how it was found is in the output only.
    let docs_order = [
        "rules",
        "entity_states",
        "counters",
        "spawn",
        "platformer.player",
        "platformer.enemies",
        "platformer.contact",
        "exits",
        "camera",
    ];
    let mut order: Vec<&str> = docs_order.to_vec();
    let rank_of = |sys: &str| -> Option<u64> {
        systems_trace["routines"]
            .as_array()?
            .iter()
            .filter(|r| {
                r["systems"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v == sys))
            })
            .filter_map(|r| r["median_rank"].as_u64())
            .min()
    };
    let measured_player_first =
        matches!((rank_of("player"), rank_of("enemies")), (Some(p), Some(e)) if p < e);
    let measured_camera_last = !systems_trace["camera_lag0"]
        .as_array()
        .is_none_or(Vec::is_empty)
        || !systems_trace["camera_lag1"]
            .as_array()
            .is_none_or(Vec::is_empty);
    if measured_camera_last {
        order.retain(|s| *s != "camera");
        order.push("camera");
    }
    set(&mut doc, "model.systems", json!(order))?;
    let systems_by = format!(
        "partly measured (player before the other movers: {measured_player_first}; camera last: {measured_camera_last}); the rest unobservable in current clips, in docs/model.md's order"
    );
    let ev = doc
        .as_table_mut()
        .context("model.toml is a table")?
        .entry("evidence")
        .or_insert(toml::Value::Table(Default::default()));
    let ev = ev.as_table_mut().context("evidence is a table")?;
    for (path, _, why, how) in &facts {
        let mut e = toml::map::Map::new();
        e.insert("source".into(), "observed".into());
        e.insert("confidence".into(), toml::Value::Integer(80));
        let mut r = toml::map::Map::new();
        r.insert("at".into(), why.clone().into());
        e.insert(
            "refs".into(),
            toml::Value::Array(vec![toml::Value::Table(r)]),
        );
        e.insert("note".into(), format!("found by: {how}").into());
        ev.insert(path.clone(), toml::Value::Table(e));
    }
    fs::write(&mf, toml::to_string(&doc)?)?;

    // mapping.toml: the player's place.
    let hex = |v: usize| format!("0x{v:02X}");
    let x_ram = match hi {
        Some(h) => format!("[{}, {}]", hex(st.x), hex(h)),
        None => format!("[{}]", hex(st.x)),
    };
    let y_ram = match y_hi {
        Some(h) => format!("[{}, {}]", hex(st.y), hex(h)),
        None => format!("[{}]", hex(st.y)),
    };
    // y's fraction and the vertical speed, from the jump fitter: the bytes
    // that make y move by the speed each frame.
    let y_frac = rule["y_fraction"]
        .as_u64()
        .map_or(String::new(), |f| format!("frac = {}\n", hex(f as usize)));
    let ground = {
        let mut runs: Vec<&[Seen]> = vec![&long, &jumped, &tapped];
        if !path_seen.is_empty() {
            runs.push(&path_seen);
        }
        on_ground_byte(&runs, st.y, st.x)
    };
    let ground_field = ground.map_or(String::new(), |g| {
        format!(
            "\n[[field]]\npath = \"entities.player[0].on_ground\"\nram = [{}]\nflag = true\nnote = \"set while it stands, clear in the air\"\n",
            hex(g)
        )
    });
    let vy_field = match (vyb, rule["speed_fraction"].as_u64()) {
        (Some(v), Some(f)) => format!(
            "\n[[field]]\npath = \"entities.player[0].vy\"\nram = [{}]\nfrac = {}\nsigned = true\nnote = \"y moves by it each frame\"\n",
            hex(v),
            hex(f as usize)
        ),
        _ => String::new(),
    };
    let mapping = format!(
        "# Written by `gp model blind` from a run of the ROM ({rom_ref}).\n\
         schema = {}\nmodel = \"{model_name}\"\nsource = \"{source}\"\nalign = \"polled\"\n\n\
         [[field]]\npath = \"entities.player[0].x\"\nram = {x_ram}\nnote = \"follows the d-pad across\"\n\n\
         [[field]]\npath = \"entities.player[0].y\"\nram = {y_ram}\n{y_frac}note = \"lifts when jump is pressed\"\n{vy_field}{ground_field}",
        gp_interp::mapping::SCHEMA
    );
    fs::write(a.model.join("mapping.toml"), mapping)?;

    let v = json!({
        "model": a.model.display().to_string(),
        "x": hex(st.x), "x_high": hi.map(hex), "y": hex(st.y), "y_high": y_hi.map(hex), "x_high_found_by": hi.map(|_| x_hi_by), "y_high_found_by": y_hi.map(|_| y_hi_by),
        "facts": [],
        "written": facts.iter().map(|(p, v, w, h)| json!({"path": p, "value": v, "evidence": w, "found_by": h})).collect::<Vec<_>>(),
        "played": played,
        "vy": vyb.map(hex),
        "on_ground": ground.map(hex),
        "hitbox_open": hitbox_open,
        "systems_trace": systems_trace,
        "level": level_note,
        "rule": rule,
        "systems": {"order": order, "found_by": systems_by},
    });
    let text = facts
        .iter()
        .map(|(p, v, _, h)| format!("{p} = {v}  ({h})"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Outcome::ok(
        v,
        format!("{text}\nmapping: x {x_ram}, y {y_ram}"),
    ))
}

/// Plays an input log from power-on and keeps every frame.
fn replay(bytes: &[u8], power_on: gp_evidence::manifest::PowerOn, text: &str) -> Result<Vec<Seen>> {
    let log = gp_evidence::input_log::InputLog::parse(text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut nes = gp_nes::Nes::power_on(bytes, power_on)?;
    nes.set_video(false);
    let mut out = Vec::with_capacity(log.frames.len());
    for f in &log.frames {
        nes.step(*f)?;
        out.push(Seen {
            wram: nes.wram()[..0x800].to_vec(),
            oam: nes.oam(),
            pad: f.pads[0],
        });
    }
    Ok(out)
}

/// Sets a dotted path in a TOML document, making tables on the way.
fn set(doc: &mut toml::Value, path: &str, v: Value) -> Result<()> {
    let t = toml::Value::try_from(v)?;
    let mut cur = doc;
    let parts: Vec<&str> = path.split('.').collect();
    for (i, p) in parts.iter().enumerate() {
        let tab = cur
            .as_table_mut()
            .context("a path through a value that isn't a table")?;
        if i + 1 == parts.len() {
            tab.insert((*p).into(), t);
            return Ok(());
        }
        cur = tab
            .entry(*p)
            .or_insert(toml::Value::Table(Default::default()));
    }
    bail!("an empty path")
}
