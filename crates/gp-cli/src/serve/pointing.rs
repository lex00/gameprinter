//! Pause and point (docs/point-and-tell.md): what's under a rectangle of
//! the screen at one frame, in both games, named in plain words.
//!
//! The original's side reads the frame's state (from the recording's trace
//! when the run has a stored recording, else by replaying its input log on
//! the ROM) and answers as box selection does (`serve::select`): the
//! sprites in OAM and the mapped entities whose hitboxes the box touches.
//! It adds the background cells under the box, read from the nametables
//! at the frame's scroll and named by matching their tiles to the model's
//! lifted metatile tables, and the HUD fields the box covers
//! (`present.hud`). The model's side runs the model in the interpreter on
//! the same input's polled frames to the matching model frame and answers
//! the runtime protocol's `entities_at`, the level cells and the HUD from
//! its own state.
//!
//! Ported from game-printer's point.rs (wave 8), as a part of gp serve.
//! What gameprinter's layout changed: a run is a game ([`RunGame::of_run`]),
//! with the recordings the game view plays (super::lockstep::recordings),
//! its model the run's working copy or the game's own, run through its
//! module as the lockstep runs it (crate::hosted) rather than the built-in
//! interpreter alone; a ROM is read by the server, not from a file.
//!
//! Nothing here knows a game. Entity and counter names are the model's,
//! cell names are the asset map's names for its metatiles, and roles come
//! from the model's `item_roles`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use gp_evidence::input_log::InputLog;
use gp_runtime::{Rect, Runtime as _};
use serde_json::{Value, json};

use super::labels::Loaded;
use super::select::{mapping_for_dir, selection};

/// The picture's size.
pub const SCREEN: (i64, i64) = (256, 240);

/// Opens a recording in `store` read-only.
fn open_in(store: &Path, id: &str) -> Result<crate::open::Opened> {
    let store = gp_evidence::Store::open_existing(store)?;
    let entry = gp_evidence::recordings::resolve(&store, id)?;
    let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
    Ok(crate::open::Opened {
        store,
        entry,
        manifest,
    })
}

/// A run's recording, by the name the game view shows.
#[derive(Debug, Clone)]
pub struct RunRecording {
    pub name: String,
    /// Its ID in [`RunGame::store`], once recorded.
    pub id: Option<String>,
    /// The input log the run keeps for it.
    pub log: Option<PathBuf>,
    /// Its input log as text, when the server read it already.
    pub text: Option<String>,
}

/// What pointing needs of a capture run or a play session.
#[derive(Clone)]
pub struct RunGame {
    pub run: String,
    /// `capture` or `play`.
    pub kind: &'static str,
    /// The folder with the model and its state mapping.
    pub model_dir: PathBuf,
    /// Where the run's recordings are.
    pub store: PathBuf,
    pub rom: Option<PathBuf>,
    /// The ROM's bytes, when the server read it already.
    pub rom_bytes: Option<std::sync::Arc<Vec<u8>>>,
    /// The model's source, opened with its module (crate::hosted).
    pub source: Option<std::sync::Arc<(crate::hosted::Source, gp_interp::mapping::Mapping)>>,
    pub recordings: Vec<RunRecording>,
}

impl RunGame {
    /// A game of gp serve's games checkout, with the recording `name` the
    /// page points at, its input log read by the server.
    pub(crate) fn of_run(
        ctx: &super::Ctx,
        run: &super::game::Run,
        name: &str,
    ) -> Result<Self, super::Fail> {
        let text = super::lockstep::input_of(ctx, run, name)?;
        let rom = super::game::rom_of(ctx, run)?;
        let model_dir = run.model_dir();
        let mut rb = run.rebuild(ctx);
        rb.game = model_dir.clone();
        let source = crate::mechanic::live::opened(&ctx.paths, &rb, &rom).ok();
        Ok(Self {
            run: run.name.clone(),
            kind: "capture",
            model_dir,
            store: ctx.paths.store.clone(),
            rom: None,
            rom_bytes: Some(rom),
            source,
            recordings: vec![RunRecording {
                name: name.to_owned(),
                id: None,
                log: None,
                text: Some(String::from_utf8_lossy(&text).into_owned()),
            }],
        })
    }

    pub fn recording(&self, name: &str) -> Result<&RunRecording> {
        self.recordings
            .iter()
            .find(|r| r.name == name)
            .with_context(|| {
                format!(
                    "no recording {name:?} in the run (it has {})",
                    self.recordings
                        .iter()
                        .map(|r| r.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }

    /// The input log of a recording: the run's copy, else the stored one.
    pub fn input_log(&self, r: &RunRecording) -> Result<InputLog> {
        if let Some(t) = &r.text {
            return Ok(InputLog::parse(t)?);
        }
        if let Some(p) = &r.log {
            if let Ok(t) = std::fs::read_to_string(p) {
                return Ok(InputLog::parse(&t)?);
            }
            // A session still being played: the frames sent so far.
            if let Some(part) = p.parent().map(|d| d.join("frames.part"))
                && let Ok(lines) = std::fs::read_to_string(part)
            {
                return Ok(InputLog::parse(&format!(
                    "gp-input-log 1\ncontrollers 1\n{lines}"
                ))?);
            }
        }
        let id = r.id.as_deref().context("the recording has no input log")?;
        let o = open_in(&self.store, id)?;
        Ok(InputLog::parse(&String::from_utf8_lossy(
            &o.store.get(&o.manifest.input_log)?,
        ))?)
    }
}

/// The rectangle `x,y,w,h` of the picture, checked and clipped to it.
pub fn parse_rect(text: &str) -> Result<Rect, String> {
    let n: Vec<i64> = text
        .split(',')
        .map(|p| p.trim().parse::<i64>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("rect is x,y,w,h in whole pixels, not {text:?}"))?;
    let [x, y, w, h] = n[..] else {
        return Err(format!("rect is x,y,w,h, four numbers, not {text:?}"));
    };
    if !(0..SCREEN.0).contains(&x) || !(0..SCREEN.1).contains(&y) || w < 1 || h < 1 {
        return Err(format!(
            "rect {text:?} must start on the {}x{} picture and be at least 1x1",
            SCREEN.0, SCREEN.1
        ));
    }
    Ok(Rect {
        x,
        y,
        w: w.min(SCREEN.0 - x) as u64,
        h: h.min(SCREEN.1 - y) as u64,
    })
}

/// One metatile of the model's lifted tables: its tiles, the asset map's
/// name for it and its role.
#[derive(Debug, Clone, PartialEq)]
pub struct Metatile {
    pub asset: String,
    pub index: usize,
    pub tiles: [u8; 4],
    pub name: String,
    pub role: Option<String>,
}

/// Reads a lifted metatiles file (`gp assets lift` writes each entry as
/// `# <index>: <name>[, level character "c"]` over `[[metatile]]` and its
/// `tiles`), keeping each entry's tiles and name.
pub fn read_metatiles(asset: &str, text: &str, roles: &[String]) -> Vec<Metatile> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(c) = l.strip_prefix("# ")
            && let Some((i, n)) = c.split_once(": ")
            && i.chars().all(|ch| ch.is_ascii_digit())
        {
            let n = n.split(", level character").next().unwrap_or(n);
            name = Some(n.trim().to_owned());
        } else if let Some(t) = l.strip_prefix("tiles = [") {
            let nums: Vec<u8> = t
                .trim_end_matches(']')
                .split(',')
                .filter_map(|x| {
                    let x = x.trim();
                    x.strip_prefix("0x")
                        .map_or_else(|| x.parse().ok(), |h| u8::from_str_radix(h, 16).ok())
                })
                .collect();
            if let [a, b, c, d] = nums[..] {
                let index = out.len();
                out.push(Metatile {
                    asset: asset.into(),
                    index,
                    tiles: [a, b, c, d],
                    name: name.take().unwrap_or_else(|| format!("{asset} {index}")),
                    role: roles.get(index).cloned(),
                });
            }
        }
    }
    out
}

/// Every metatile of the model's metatile slots, read from its lifted
/// assets. Empty when the assets aren't lifted.
pub fn model_metatiles(model: &gp_model::Model, assets: &Path) -> Vec<Metatile> {
    let mut out = Vec::new();
    for (name, slot) in &model.assets {
        if slot.kind != gp_model::model::AssetKind::Metatiles {
            continue;
        }
        let file = slot
            .from
            .as_ref()
            .map_or(name.as_str(), |f| f.asset.as_str());
        if let Ok(t) = std::fs::read_to_string(assets.join(format!("{file}.toml"))) {
            out.extend(read_metatiles(name, &t, &slot.item_roles));
        }
    }
    out
}

/// The four tiles of the 16x16 cell holding screen pixel `x`, `y`, from
/// the nametables at `scroll` under `mirroring` (as the canonical state's
/// `mapper.mirroring`), with the cell's top left in screen pixels.
pub fn cell_at(
    nt: &[u8],
    mirroring: u8,
    scroll: [i64; 2],
    x: i64,
    y: i64,
) -> Option<([u8; 4], [i64; 2])> {
    let lx = (x + scroll[0]).rem_euclid(512);
    let ly = (y + scroll[1]).rem_euclid(480);
    let logical = (lx / 256 + 2 * (ly / 240)) as u8;
    let phys = match mirroring {
        0 => logical & 1,
        1 => logical >> 1,
        2 => 0,
        3 => 1,
        _ => logical,
    };
    let (col, row) = (((lx % 256) / 16) * 2, ((ly % 240) / 16) * 2);
    let base = usize::from(phys) * 1024;
    let at = |r: i64, c: i64| nt.get(base + (r.min(29) * 32 + c) as usize).copied();
    let tiles = [
        at(row, col)?,
        at(row, col + 1)?,
        at(row + 1, col)?,
        at(row + 1, col + 1)?,
    ];
    // The cell's top left on screen.
    let sx = x - (lx % 16);
    let sy = y - ((ly % 240) % 16);
    Some((tiles, [sx, sy]))
}

/// The screen cells (16x16, aligned to the scroll) a rectangle touches,
/// as their top-left screen pixels' sample points.
fn cell_points(r: Rect, scroll: [i64; 2]) -> Vec<(i64, i64)> {
    let (ox, oy) = (scroll[0].rem_euclid(16), scroll[1].rem_euclid(16));
    let mut out = Vec::new();
    let mut y = r.y - (r.y + oy).rem_euclid(16);
    while y < r.y + r.h as i64 {
        let mut x = r.x - (r.x + ox).rem_euclid(16);
        while x < r.x + r.w as i64 {
            out.push((x.max(0).max(r.x), y.max(0).max(r.y)));
            x += 16;
        }
        y += 16;
    }
    // A whole screen is 17 by 16 cells at most.
    out.truncate(300);
    out
}

/// "a brick", "an empty cell": the name with its article.
pub fn a(name: &str) -> String {
    let n = name.trim();
    let lower = n.to_ascii_lowercase();
    // "a used block", "a unit": a u that sounds like "you".
    let you = ["use", "uni", "usu", "eu", "one"]
        .iter()
        .any(|p| lower.starts_with(p));
    let vowel = !you && lower.chars().next().is_some_and(|c| "aeiou".contains(c));
    format!("{} {n}", if vowel { "an" } else { "a" })
}

/// "the goomba", "Mario": an entity kind as a person would say it. The
/// player's kind is called by its own name when it has one.
pub fn entity_label(kind: &str, role: Option<&str>) -> String {
    let words = kind.replace('_', " ");
    if role == Some("player") && kind != "player" {
        let mut c = words.chars();
        return c
            .next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default();
    }
    format!("the {words}")
}

/// The HUD fields of the model that a screen rectangle covers, each with
/// its counter (or `level` for the level's label) and its box.
pub fn hud_under(model: &gp_model::Model, r: Rect) -> Vec<(String, [i64; 4])> {
    let Some(h) = model.present.as_ref().and_then(|p| p.hud.as_ref()) else {
        return Vec::new();
    };
    h.field
        .iter()
        .filter_map(|f| {
            let what = match (&f.counter, f.level_label, &f.text) {
                (Some(c), _, _) => c.clone(),
                (None, true, _) => "level".into(),
                _ => return None,
            };
            let chars = f.digits.unwrap_or(match what.as_str() {
                "level" => 3,
                _ => 2,
            });
            let b = [
                i64::from(f.at[0]) * 8,
                i64::from(f.at[1]) * 8,
                i64::from(chars) * 8,
                8,
            ];
            let overlaps = b[0] < r.x + r.w as i64
                && r.x < b[0] + b[2]
                && b[1] < r.y + r.h as i64
                && r.y < b[1] + b[3];
            overlaps.then_some((what, b))
        })
        .collect()
}

fn hud_label(what: &str) -> String {
    if what == "level" {
        "the level's name".into()
    } else {
        format!("the {}", what.replace('_', " "))
    }
}

/// The original's state at `frame`: the stored recording's (trace or
/// replay), or the ROM replayed on the input log.
fn original_state(
    game: &RunGame,
    r: &RunRecording,
    log: &InputLog,
    frame: u64,
    paths: &Paths,
) -> Result<(BTreeMap<String, Vec<u8>>, &'static str)> {
    let fields = |table: &gp_evidence::trace::FieldTable,
                  state: &[u8]|
     -> Result<BTreeMap<String, Vec<u8>>> {
        Ok(table
            .split(state)
            .map_err(anyhow::Error::from)?
            .into_iter()
            .map(|(n, b)| (n.to_owned(), b.to_vec()))
            .collect())
    };
    if let Some(id) = &r.id
        && let Ok(o) = open_in(&game.store, id)
    {
        let (state, from) = o.state(frame, game.rom.as_deref(), paths)?;
        let table = crate::recordings::field_table_of(&o.store, &o.manifest)?;
        return Ok((fields(&table, &state)?, from));
    }
    let rom = match (&game.rom_bytes, &game.rom) {
        (Some(b), _) => b.to_vec(),
        (None, Some(f)) => std::fs::read(f).with_context(|| f.display().to_string())?,
        (None, None) => {
            bail!("the run has no ROM and no stored recording to read the original from")
        }
    };
    let mut nes = gp_nes::Nes::power_on(&rom, Default::default())?;
    for f in log.frames.iter().take(frame as usize) {
        nes.step(*f)?;
    }
    let state = nes.canonical_state()?;
    Ok((fields(&gp_nes::canonical::field_table(), &state)?, "replay"))
}

/// The model frame shown at original frame `frame`: how many of the log's
/// frames up to it the game polled the controller in.
pub fn model_frame(log: &InputLog, frame: u64) -> u64 {
    log.frames
        .iter()
        .take(frame as usize)
        .filter(|f| f.polled)
        .count() as u64
}

/// Model frames past the shown one the model's track is followed, to find
/// where it reaches the original's position when it runs behind.
const ALIGN_AHEAD: u64 = 1800;

/// The model run to `frame` (a model frame) on the log's polled frames:
/// its side under `rect`, and in `track` the player's position on each
/// frame up to `ahead` frames past it.
#[allow(clippy::too_many_arguments)]
fn model_at(
    game: &RunGame,
    log: &InputLog,
    frame: u64,
    ahead: u64,
    track: &mut Vec<gp_gaps::align::Pos>,
    model: &gp_model::Model,
    rect: Rect,
    paths: &Paths,
) -> Result<Value> {
    let _ = paths;
    let source = game
        .source
        .as_ref()
        .context("the model can't be opened with its module, so its side isn't shown")?;
    let mut rt = source.0.open()?;
    let player = {
        let spec = rt.spec();
        spec.kinds.iter().position(|k| {
            model
                .entities
                .get(&k.name)
                .is_some_and(|e| e.core.role == "player")
        })
    };
    let push = |rt: &crate::hosted::Model, track: &mut Vec<gp_gaps::align::Pos>| {
        let w = rt.world();
        if let Some(b) = player.and_then(|k| w.bodies.iter().find(|b| b.kind == k)) {
            track.push(gp_gaps::align::Pos {
                frame: w.frame,
                level: rt
                    .spec()
                    .levels
                    .get(w.level)
                    .map(|l| l.name.clone())
                    .unwrap_or_default(),
                x: b.x.floor() as i32,
                y: b.y.floor() as i32,
            });
        }
    };
    let mut side = None;
    let polled = log.frames.iter().filter(|f| f.polled);
    for (n, f) in polled.take((frame + ahead) as usize).enumerate() {
        if n as u64 == frame {
            side = Some(model_side(&rt, model, rect));
        }
        rt.step(&[gp_runtime::InputFrame {
            buttons: f.buttons_word(),
            axes: Vec::new(),
            reset: f.reset,
        }])
        .map_err(anyhow::Error::msg)?;
        push(&rt, track);
    }
    Ok(side.unwrap_or_else(|| model_side(&rt, model, rect)))
}

/// The original's player at `frame`, world pixels, through the mapping.
fn original_player(ram: &[u8], m: &gp_interp::mapping::Mapping) -> Option<(i32, i32)> {
    let field = |end: &str| {
        m.fields.iter().find(|f| {
            f.path
                .strip_prefix("entities.")
                .and_then(|r| r.split_once("[0]."))
                .is_some_and(|(_, field)| field == end)
        })
    };
    let (fx, fy) = (field("x")?, field("y")?);
    let x = crate::gaps::read(fx, ram, 0)?;
    let y = crate::gaps::read(fy, ram, 0)?;
    Some((x as i32, y as i32))
}

/// The model's side: entities under the box, level cells, HUD values.
fn model_side(rt: &crate::hosted::Model, model: &gp_model::Model, rect: Rect) -> Value {
    let spec = rt.spec();
    let w = rt.world();
    let cam = w
        .cameras
        .first()
        .map_or([0, 0], |c| [c[0].floor(), c[1].floor()]);
    let world = Rect {
        x: rect.x + cam[0],
        y: rect.y + cam[1],
        ..rect
    };
    let role = |k: &str| model.entities.get(k).map(|e| e.core.role.clone());
    let entities: Vec<Value> = rt
        .entities_at(world)
        .into_iter()
        .filter_map(|i| {
            let b = w.bodies.get(i as usize)?;
            let kind = &spec.kinds[b.kind];
            let n = w.of_kind(b.kind).position(|(j, _)| j == i as usize)?;
            let h = spec.kind(b).hitbox;
            let (x, y) = (b.x.add(h.x).floor() - cam[0], b.y.add(h.y).floor() - cam[1]);
            Some(json!({
                "path": format!("entities.{}[{n}]", kind.name),
                "kind": kind.name,
                "role": role(&kind.name),
                "label": entity_label(&kind.name, role(&kind.name).as_deref()),
                "box": {"x": x, "y": y, "w": h.w.floor(), "h": h.h.floor()},
            }))
        })
        .collect();
    let level = spec.levels.get(w.level);
    let mut cells = Vec::new();
    if let Some(l) = level {
        let tiles = gp_sim::world::Tiles {
            level: l,
            changed: &w.tiles,
            bodies: &[],
        };
        let ts = l.tile_size.max(1);
        let mut seen = std::collections::BTreeSet::new();
        let mut y = world.y.div_euclid(ts) * ts;
        while y < world.y + world.h as i64 {
            let mut x = world.x.div_euclid(ts) * ts;
            while x < world.x + world.w as i64 {
                if let Some(c) = l.cell(x, y)
                    && seen.insert(c)
                {
                    let changed = w.tiles.iter().any(|(i, _)| *i as usize == c);
                    cells.push(json!({
                        "cell": [x / ts, y / ts],
                        "tile": format!("{:?}", tiles.at(x, y)).to_ascii_lowercase(),
                        "changed": changed,
                    }));
                }
                x += ts;
            }
            y += ts;
        }
        cells.truncate(64);
    }
    let hud: Vec<Value> = hud_under(model, rect)
        .into_iter()
        .map(|(what, b)| {
            let value = spec
                .counters
                .iter()
                .position(|c| c.name == what)
                .and_then(|i| w.counters.get(i).copied());
            json!({"what": what, "label": hud_label(&what), "box": b, "value": value})
        })
        .collect();
    json!({
        "frame": w.frame,
        "camera": cam,
        "world": {"x": world.x, "y": world.y, "w": world.w, "h": world.h},
        "level": level.map(|l| l.name.clone()),
        "entities": entities,
        "cells": cells,
        "hud": hud,
    })
}

/// What's under `rect` at original frame `frame` of the run's recording
/// `name`, in both games, with the things named.
pub fn resolve(paths: &Paths, game: &RunGame, name: &str, frame: u64, rect: Rect) -> Result<Value> {
    let r = game.recording(name)?;
    let log = game.input_log(r)?;
    if frame == 0 || frame > log.frames.len() as u64 {
        bail!(
            "frame {frame} is past the recording's {} frames",
            log.frames.len()
        );
    }
    let model = crate::mechanics::load_model(&game.model_dir)?;
    let source = model.model.source.clone().unwrap_or_default();
    let assets = paths.home.join("work/assets").join(&source);
    let mapped = mapping_for_dir(&game.model_dir).map_err(anyhow::Error::msg)?;

    // The original.
    let mut player_at = None;
    let original = (|| -> Result<Value> {
        let (fields, from) = original_state(game, r, &log, frame, paths)?;
        let get = |n: &str| fields.get(n).map(Vec::as_slice).unwrap_or(&[]);
        if let Some((m, _)) = &mapped {
            player_at = original_player(get("ram"), m);
        }
        let tall = get("ppu.ctrl").first().is_some_and(|c| c & 0x20 != 0);
        let labels = Loaded {
            game: Some(source.clone()),
            file: None,
            missing: None,
            labels: Vec::new(),
        };
        let mut out = selection(
            get("ram"),
            get("oam"),
            tall,
            rect,
            mapped.as_ref().map(|(m, h)| (m, h)),
            &labels,
        );
        let t = get("ppu.t");
        let t_reg = if t.len() >= 2 {
            u16::from_le_bytes([t[0], t[1]])
        } else {
            0
        };
        let fine = get("ppu.x").first().map_or(0, |b| u16::from(b & 7));
        let scroll = [
            i64::from((t_reg & 31) * 8 + fine + ((t_reg >> 10) & 1) * 256),
            i64::from(((t_reg >> 5) & 31) * 8 + ((t_reg >> 12) & 7) + ((t_reg >> 11) & 1) * 240),
        ];
        let mirroring = get("mapper.mirroring").first().copied().unwrap_or(1);
        let nt = get("nametables");
        let metatiles = model_metatiles(&model, &assets);
        let hud = hud_under(&model, rect);
        let in_hud = |p: [i64; 2]| hud.iter().any(|(_, b)| p[1] >= b[1] && p[1] < b[1] + 8);
        let hud_rows = model
            .present
            .as_ref()
            .and_then(|p| p.hud.as_ref())
            .and_then(|h| h.size.map(|s| i64::from(s[1]) * 8))
            .unwrap_or(0);
        let mut cells = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (x, y) in cell_points(rect, scroll) {
            if y < hud_rows || in_hud([x, y]) {
                continue;
            }
            let Some((tiles, at)) = cell_at(nt, mirroring, scroll, x, y) else {
                continue;
            };
            if !seen.insert(at) {
                continue;
            }
            let m = metatiles.iter().find(|m| m.tiles == tiles);
            cells.push(json!({
                "screen": at,
                "tiles": gp_gaps::sig(&tiles),
                "name": m.map(|m| m.name.clone()),
                "role": m.and_then(|m| m.role.clone()),
                "asset": m.map(|m| format!("{}[{}]", m.asset, m.index)),
            }));
        }
        let o = out.as_object_mut().expect("an object");
        o.insert("from".into(), json!(from));
        o.insert("scroll".into(), json!(scroll));
        o.insert("cells".into(), json!(cells));
        o.insert(
            "hud".into(),
            json!(
                hud.iter()
                    .map(|(w, b)| json!({"what": w, "label": hud_label(w), "box": b}))
                    .collect::<Vec<_>>()
            ),
        );
        o.remove("ram");
        Ok(out)
    })()
    .unwrap_or_else(|e| json!({"error": format!("{e:#}")}));

    // The model.
    let mframe = model_frame(&log, frame);
    let mut track = Vec::new();
    let model_json = model_at(
        game,
        &log,
        mframe,
        ALIGN_AHEAD,
        &mut track,
        &model,
        rect,
        paths,
    )
    .unwrap_or_else(|e| json!({"error": format!("{e:#}")}));
    // The same moment by position: the model's player leaves the
    // original's path, so the frame number alone soon compares two places.
    // The model's track is matched to where the original's player is.
    let alignment = player_at
        .map(|(x, y)| {
            // The original's level isn't read from its RAM yet, so the
            // match is on position (an empty level matches any).
            let at = gp_gaps::align::Pos {
                frame: mframe,
                level: String::new(),
                x,
                y,
            };
            json!(gp_gaps::align::drift(&at, &track))
        })
        .unwrap_or(Value::Null);

    let role = |k: &str| model.entities.get(k).map(|e| e.core.role.clone());
    let things = things(&role, &original, &model_json);
    let label = things
        .first()
        .and_then(|t| t["label"].as_str())
        .unwrap_or("this part of the screen")
        .to_owned();
    Ok(json!({
        "run": game.run,
        "kind": game.kind,
        "recording": name,
        "recording_id": r.id,
        "frame": frame,
        "model_frame": mframe,
        "rect": {"x": rect.x, "y": rect.y, "w": rect.w, "h": rect.h},
        "label": label,
        "things": things,
        "original": original,
        "model": model_json,
        "alignment": alignment,
    }))
}

/// The things under the box, both games together, most specific first:
/// entities, HUD fields, named background cells, then sprites no entity
/// explains. Each says which games have it.
pub fn things(role: &dyn Fn(&str) -> Option<String>, original: &Value, m: &Value) -> Vec<Value> {
    let arr = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let mut out: Vec<Value> = Vec::new();
    // Entities, by kind.
    let mut kinds: BTreeMap<String, (u64, u64, Vec<String>)> = BTreeMap::new();
    for e in arr(&original["entities"]) {
        let k = e["kind"].as_str().unwrap_or_default().to_owned();
        let x = kinds.entry(k).or_default();
        x.0 += 1;
        if let Some(p) = e["path"].as_str() {
            x.2.push(p.to_owned());
        }
    }
    for e in arr(&m["entities"]) {
        let k = e["kind"].as_str().unwrap_or_default().to_owned();
        kinds.entry(k).or_default().1 += 1;
    }
    // The player last among entities: a box usually means the other thing.
    let mut ents: Vec<_> = kinds.into_iter().collect();
    ents.sort_by_key(|(k, _)| role(k).as_deref() == Some("player"));
    for (k, (o, mm, paths)) in ents {
        out.push(json!({
            "what": "entity",
            "kind": k,
            "role": role(&k),
            "label": entity_label(&k, role(&k).as_deref()),
            "original": o,
            "model": mm,
            "paths": paths,
        }));
    }
    // HUD fields.
    for h in arr(&original["hud"]).into_iter().chain(arr(&m["hud"])) {
        let what = h["what"].as_str().unwrap_or_default();
        if out
            .iter()
            .any(|t| t["what"] == "hud" && t["counter"] == what)
        {
            continue;
        }
        let value = arr(&m["hud"])
            .iter()
            .find(|x| x["what"] == what)
            .map(|x| x["value"].clone());
        out.push(json!({
            "what": "hud",
            "counter": what,
            "label": hud_label(what),
            "model_value": value,
        }));
    }
    // Background cells, by name; empty sky isn't worth naming unless it's
    // all there is.
    let mut names: BTreeMap<String, (u64, Option<String>, Vec<String>)> = BTreeMap::new();
    for c in arr(&original["cells"]) {
        let Some(n) = c["name"].as_str() else {
            continue;
        };
        let e = names.entry(n.to_owned()).or_default();
        e.0 += 1;
        e.1 = c["role"].as_str().map(str::to_owned);
        e.2.push(c["tiles"].as_str().unwrap_or_default().to_owned());
    }
    let quiet = |r: &Option<String>| matches!(r.as_deref(), Some("sky" | "background"));
    let mut cells: Vec<_> = names.into_iter().collect();
    cells.sort_by_key(|(_, (n, r, _))| (quiet(r), std::cmp::Reverse(*n)));
    let model_tiles: Vec<String> = arr(&m["cells"])
        .iter()
        .filter_map(|c| c["tile"].as_str().map(str::to_owned))
        .collect();
    let any_loud = cells.iter().any(|(_, (_, r, _))| !quiet(r));
    for (name, (n, r, tiles)) in cells {
        if any_loud && quiet(&r) {
            continue;
        }
        out.push(json!({
            "what": "cell",
            "name": name,
            "label": a(&name),
            "role": r,
            "cells": n,
            "tiles": tiles,
            "model_tiles": model_tiles,
        }));
    }
    // Sprites no mapped entity accounts for.
    let ent_boxes: Vec<[i64; 4]> = arr(&original["entities"])
        .iter()
        .map(|e| {
            let b = &e["box"];
            [
                b["x"].as_i64().unwrap_or(0),
                b["y"].as_i64().unwrap_or(0),
                b["w"].as_i64().unwrap_or(0),
                b["h"].as_i64().unwrap_or(SCREEN.1),
            ]
        })
        .collect();
    let loose: Vec<Value> = arr(&original["sprites"])
        .into_iter()
        .filter(|s| {
            let (x, y) = (
                s["box"]["x"].as_i64().unwrap_or(0),
                s["box"]["y"].as_i64().unwrap_or(0),
            );
            !ent_boxes.iter().any(|b| {
                x + 8 > b[0] - 8 && x < b[0] + b[2] + 8 && y + 8 > b[1] - 8 && y < b[1] + b[3] + 8
            })
        })
        .collect();
    if !loose.is_empty() {
        out.push(json!({
            "what": "sprite",
            "label": "something drawn there that the model has no object for",
            "sprites": loose.len(),
            "tiles": loose.iter().map(|s| s["tile"].clone()).collect::<Vec<_>>(),
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metatile_files_give_names_and_roles() {
        let text = "# Metatiles\nschema = 1\nasset = \"metatiles\"\n\n# 0: sky, level character \".\"\n[[metatile]]\ntiles = [0x00, 0x00, 0x00, 0x00]\nsolid = 0\n\n# 1: brick with line\n[[metatile]]\ntiles = [0x45, 0x45, 0x47, 0x47]\n";
        let m = read_metatiles("blocks", text, &["sky".into(), "interactive".into()]);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].name, "sky");
        assert_eq!(m[1].tiles, [0x45, 0x45, 0x47, 0x47]);
        assert_eq!(m[1].name, "brick with line");
        assert_eq!(m[1].role.as_deref(), Some("interactive"));
        assert_eq!(a(&m[1].name), "a brick with line");
        assert_eq!(a("empty cell"), "an empty cell");
        assert_eq!(a("used block"), "a used block");
    }

    #[test]
    fn cells_read_through_scroll_and_mirroring() {
        // Two nametables, vertical mirroring: the second is the right
        // screen. A cell of tiles 1 2 / 3 4 at the second table's column 2
        // and row 4 (tile coordinates), so world x 256 + 16, y 32.
        let mut nt = vec![0u8; 2048];
        let put = |nt: &mut Vec<u8>, r: usize, c: usize, v: u8| nt[1024 + r * 32 + c] = v;
        put(&mut nt, 4, 2, 1);
        put(&mut nt, 4, 3, 2);
        put(&mut nt, 5, 2, 3);
        put(&mut nt, 5, 3, 4);
        // Scrolled 200 right: the cell is at screen x 72..88.
        let (tiles, at) = cell_at(&nt, 0, [200, 0], 75, 40).unwrap();
        assert_eq!(tiles, [1, 2, 3, 4]);
        assert_eq!(at, [72, 32]);
        // Nothing there on the first screen.
        assert_eq!(cell_at(&nt, 0, [0, 0], 20, 40).unwrap().0, [0, 0, 0, 0]);
        let pts = cell_points(
            Rect {
                x: 70,
                y: 30,
                w: 20,
                h: 4,
            },
            [200, 0],
        );
        // 70..90 crosses three cells (the scroll puts their edges at 72
        // and 88), and 30..34 two rows.
        assert_eq!(pts.len(), 6, "{pts:?}");
        let one = cell_points(
            Rect {
                x: 72,
                y: 32,
                w: 16,
                h: 16,
            },
            [200, 0],
        );
        assert_eq!(one, [(72, 32)]);
    }

    #[test]
    fn labels_are_plain() {
        assert_eq!(entity_label("goomba", Some("enemy")), "the goomba");
        assert_eq!(entity_label("mario", Some("player")), "Mario");
        assert_eq!(entity_label("player", Some("player")), "the player");
        assert_eq!(entity_label("koopa_red", Some("enemy")), "the koopa red");
        assert_eq!(hud_label("score"), "the score");
        assert_eq!(parse_rect("250,230,20,20").unwrap().w, 6);
        assert!(parse_rect("0,240,1,1").is_err());
    }

    #[test]
    fn things_put_entities_first_and_skip_sky_beside_a_brick() {
        let role = |k: &str| match k {
            "hero" => Some("player".to_owned()),
            "blob" => Some("enemy".to_owned()),
            _ => None,
        };
        let original = json!({
            "entities": [{"kind": "hero", "path": "entities.hero[0]", "box": {"x": 0, "y": 0, "w": 8, "h": 8}},
                         {"kind": "blob", "path": "entities.blob[0]", "box": {"x": 50, "y": 50, "w": 8, "h": 8}}],
            "cells": [{"name": "sky", "role": "sky", "tiles": "24242424"},
                      {"name": "brick", "role": "interactive", "tiles": "45454747"}],
            "sprites": [{"box": {"x": 120, "y": 120}, "tile": 9}],
            "hud": [],
        });
        let m = json!({"entities": [{"kind": "blob"}], "cells": [{"tile": "solid"}], "hud": []});
        let t = things(&role, &original, &m);
        let labels: Vec<&str> = t.iter().map(|x| x["label"].as_str().unwrap()).collect();
        assert_eq!(labels[0], "the blob");
        assert_eq!(labels[1], "Hero");
        assert_eq!(labels[2], "a brick");
        assert!(labels[3].starts_with("something drawn"));
        assert_eq!(t[0]["model"], 1);
        assert_eq!(t[1]["model"], 0);
    }
}
