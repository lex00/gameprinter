//! `gp gaps` on an arcade recording (port wave 6b): gp-gaps' per-frame
//! facts built from gp-arcade instead of the NES's sprite table,
//! nametables, APU and 6502 code.
//!
//! | Fact | NES | Arcade (Tapper's MCR board) |
//! |---|---|---|
//! | RAM | internal RAM, $0000-$07FF | the main CPU's address space, so a mapping's addresses (E000 on) read as they are; `ram_changed` is the work RAM, E000-E7FF |
//! | objects | OAM clusters | the sprite list (`gp_arcade::video::sprite_list`) clustered as gp-track clusters them, in the game's half-pixel units |
//! | cells | 16x16 nametable cells | video RAM's 32x30 tiles of 16x16 (`nt` 0), each tile's two bytes as `before`/`after` [low, high, 0, 0]; writers are the routines that wrote its video RAM bytes |
//! | sound | APU writes | the AY-8910 writes (`ay-write` events), as chip x 256 + register |
//! | code | the code observer's replay | a replay with the trace and access log on the main Z80: routines entered (call, restart and interrupt targets), RAM writes with their instruction and routine, and the routines that wrote the sound board's latch (the "engine") |
//!
//! Addresses of code are the main CPU's (there is no banking), used as
//! the "PRG offset" gp-gaps keys code by.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result, bail};
use gp_arcade::Tapper;
use gp_arcade::dasm::{self, Flow};
use gp_arcade::mcr::AccessKind;
use gp_evidence::Paths;
use gp_gaps::{CellChange, Code, Ent, FrameFacts, Object, RamWrite, Run};
use gp_runtime::select::Condition;

use crate::gaps::{Built, GapsArgs, Template, eval, hud_digits, known, labels, read};
use crate::open::Opened;
use crate::recordings::parse_frames;

/// The main CPU's address space as a state's fields give it: work, sprite
/// and video RAM in place, zeros elsewhere.
fn space(fields: &[(&str, &[u8])]) -> Vec<u8> {
    let mut m = vec![0u8; 0x1_0000];
    let get = |n: &str| fields.iter().find(|(f, _)| *f == n).map(|(_, b)| *b);
    for (name, at) in [
        ("ram", 0xe000),
        ("sprite_ram", 0xe800),
        ("video_ram", 0xf000),
    ] {
        if let Some(b) = get(name) {
            m[at..at + b.len()].copy_from_slice(b);
        }
    }
    m
}

/// One frame's code facts from a replay of it.
fn code_of(w: &gp_arcade::levels::Watched, routines: &BTreeSet<u16>) -> (Code, Vec<(u16, u32)>) {
    let routine_of = |pc: u16| -> u32 {
        routines
            .range(..=pc)
            .next_back()
            .map_or(u32::from(pc), |r| u32::from(*r))
    };
    let mut entered: BTreeMap<u32, u32> = BTreeMap::new();
    let mut prev: Option<(u16, Flow, u16)> = None;
    for e in w.trace.iter().filter(|e| e.cpu == 0) {
        if e.interrupt.is_some() {
            *entered.entry(u32::from(e.pc)).or_default() += 1;
            prev = None;
            continue;
        }
        if let Some((pc, flow, fall)) = prev
            && matches!(flow, Flow::Call(_) | Flow::CallIf(_) | Flow::Rst(_))
            && e.pc != fall
        {
            let _ = pc;
            *entered.entry(u32::from(e.pc)).or_default() += 1;
        }
        let i = dasm::decode(&e.bytes, e.pc);
        prev = Some((e.pc, i.flow, e.pc.wrapping_add(u16::from(i.len))));
    }
    let mut ram = Vec::new();
    let mut engine: BTreeSet<u32> = BTreeSet::new();
    let mut engine_pc: BTreeSet<u32> = BTreeSet::new();
    let mut vram = Vec::new();
    for a in w.accesses.iter().filter(|a| a.cpu == 0) {
        match a.kind {
            AccessKind::Write if (0xe000..0xe800).contains(&a.addr) => ram.push(RamWrite {
                addr: a.addr,
                value: a.value,
                engine: false,
                pc: u32::from(a.pc),
                routine: routine_of(a.pc),
            }),
            AccessKind::Write if (0xf000..0xf800).contains(&a.addr) => {
                vram.push((a.addr, routine_of(a.pc)));
            }
            AccessKind::Out if (0x1c..=0x1f).contains(&(a.addr & 0xff)) => {
                engine.insert(routine_of(a.pc));
                engine_pc.insert(u32::from(a.pc));
            }
            _ => {}
        }
    }
    for wr in &mut ram {
        wr.engine = engine.contains(&wr.routine);
    }
    (
        Code {
            entered: entered.into_iter().collect(),
            ram,
            apu: engine.into_iter().collect(),
            apu_pc: engine_pc.into_iter().collect(),
            calls: Vec::new(),
        },
        vram,
    )
}

pub(crate) fn build(args: &GapsArgs, t: &Template, paths: &Paths, opened: Opened) -> Result<Built> {
    let manifest = &opened.manifest;
    if manifest.trace.is_none() {
        bail!(
            "{} has no frame trace; gp gaps reads states from it",
            opened.short()
        );
    }
    let range = parse_frames(&args.frames, manifest.frames)?;
    let range = range.start.max(1)..range.end.min(manifest.frames + 1);
    if range.is_empty() {
        bail!("--frames {} leaves no frame", args.frames);
    }
    let mut run = Run {
        known: known(t),
        ram_base: 0xe000,
        ..Run::default()
    };
    let mut known_ = std::mem::take(&mut run.known);
    for n in labels(args, t, paths, 0xe000, &mut known_, &mut run.constants) {
        eprintln!("gp: {n}");
    }
    run.known = known_;

    // The replay with the code facts, unless --no-code or no romset.
    let mut no_code_why = String::new();
    let set = if args.no_code {
        no_code_why = "--no-code".into();
        None
    } else {
        match opened.romset(None, paths) {
            Ok(s) => Some(s),
            Err(e) => {
                no_code_why = format!("{e:#}");
                None
            }
        }
    };
    let log = gp_evidence::InputLog::parse(
        &String::from_utf8(opened.store.get(&manifest.input_log)?)
            .context("the recording's input log isn't text")?,
    )?;
    let mut code_by_frame: BTreeMap<u64, (Code, Vec<(u16, u32)>)> = BTreeMap::new();
    if let Some(set) = &set {
        run.prg = set.maincpu[..0xe000].to_vec();
        // The routines: the code map of a watched run of these frames.
        let mut t0 = Tapper::power_on(set);
        let mut w = gp_arcade::levels::Watcher::new(false);
        let mut kept = Vec::new();
        for (i, f) in log.frames.iter().enumerate().take((range.end - 1) as usize) {
            let c = gp_arcade::Controls::from_bits(f.buttons_word() & 0xff);
            let frame = i as u64 + 1;
            if frame < range.start {
                t0.step(c);
            } else {
                kept.push((frame, w.step_quiet(&mut t0, c)));
            }
        }
        let levels = w.finish();
        let map = crate::code_z80::CodeMap::of(set, 0, Some(&levels.cpus[0]));
        let routines: BTreeSet<u16> = map.routines.keys().copied().collect();
        run.executed = (0..0xe000usize.div_ceil(64))
            .map(|wd| {
                (0..64).fold(0u64, |acc, b| {
                    let o = (wd * 64 + b) as u16;
                    acc | (u64::from(levels.cpus[0].starts.contains_key(&o)) << b)
                })
            })
            .collect();
        for (frame, watched) in kept {
            code_by_frame.insert(frame, code_of(&watched, &routines));
        }
    }

    let m = &t.mapping;
    run.kinds = t.model.entities.keys().cloned().collect();
    let kind_of = |name: &str| run.kinds.iter().position(|k| k == name).map(|i| i as u16);
    let player_name = m
        .fields
        .iter()
        .find_map(|f| {
            let rest = f.path.strip_prefix("entities.")?;
            let (name, field) = rest.split_once("[0].")?;
            (field == "x").then(|| name.to_owned())
        })
        .unwrap_or_default();
    let field = |path: &str| m.fields.iter().find(|f| f.path == path);
    let px = field(&format!("entities.{player_name}[0].x"));
    let py = field(&format!("entities.{player_name}[0].y"));
    let counters: Vec<_> = m
        .fields
        .iter()
        .filter(|f| f.path.starts_with("counters."))
        .collect();
    run.counters = counters
        .iter()
        .map(|f| f.path.trim_start_matches("counters.").to_owned())
        .collect();
    run.counter_scale = counters
        .iter()
        .map(|f| {
            let _ = hud_digits(&t.model, f.path.trim_start_matches("counters."));
            1
        })
        .collect();
    let start = m
        .start
        .as_deref()
        .map(Condition::parse)
        .transpose()
        .map_err(anyhow::Error::msg)?;
    let player_kind = kind_of(&player_name);

    // The AY writes, from the events.
    let mut sound: BTreeMap<u64, Vec<(u16, u8)>> = BTreeMap::new();
    if let Some(ev) = crate::arcade_rec::stored_events(&opened.store, manifest, range.clone())? {
        for e in ev
            .iter()
            .filter(|e| e.kind == gp_evidence::events::EventKind::AyWrite)
        {
            sound.entry(e.frame).or_default().push((e.addr, e.value));
        }
    }

    let table = crate::arcade_rec::field_table_of(&opened.store, manifest)?;
    let mut prev: Option<Vec<u8>> = None;
    let mut failed = None;
    let ok = gp_nes::recording::scan_trace(
        &opened.store,
        manifest,
        (range.start - 1)..=(range.end - 1),
        |frame, state| {
            let fields = match table.split(state) {
                Ok(f) => f,
                Err(e) => {
                    failed = Some(e.to_string());
                    return false;
                }
            };
            let mem = space(&fields);
            let Some(pm) = prev.replace(mem.clone()) else {
                return true;
            };
            let f = frame;
            let input = &log.frames[(f - 1) as usize];
            // gaps' read and eval index the NES's 2 KB by an address's low
            // 11 bits: here that is the work RAM, E000-E7FF, where a
            // mapping's addresses are.
            let work = &mem[0xe000..0xe800];
            let play = start.as_ref().is_none_or(|c| eval(c, work));
            let ram_changed: Vec<u16> = (0xe000..0xe800u16)
                .filter(|&a| mem[usize::from(a)] != pm[usize::from(a)])
                .collect();
            let mut ents = Vec::new();
            if let (Some(x), Some(y)) = (px, py)
                && let (Some(x), Some(y)) = (read(x, work, 0), read(y, work, 0))
            {
                ents.push(Ent {
                    kind: player_kind,
                    player: true,
                    slot: None,
                    id: None,
                    x: x as i32,
                    y: y as i32,
                });
            }
            let counters = counters
                .iter()
                .map(|c| read(c, work, 0).unwrap_or(0))
                .collect();
            let sprites = crate::track::arcade_sprites(&mem[0xe800..0xea00]);
            let list = gp_arcade::video::sprite_list(&mem[0xe800..0xea00]);
            let objects = gp_track::sprites::group(&sprites)
                .into_iter()
                .map(|c| {
                    let inside: Vec<u8> = list
                        .iter()
                        .filter(|s| {
                            s.x >= c.x0 && s.x + 16 <= c.x1 && s.y >= c.y0 && s.y + 16 <= c.y1
                        })
                        .map(|s| s.slot)
                        .collect();
                    Object { c, sprites: inside }
                })
                .collect();
            let (code, vram) = code_by_frame
                .remove(&f)
                .map_or((None, Vec::new()), |(c, v)| (Some(c), v));
            let mut writers: BTreeMap<(u8, u8), BTreeSet<u32>> = BTreeMap::new();
            for (addr, r) in vram {
                let tile = usize::from(addr - 0xf000) / 2;
                if tile < 960 {
                    writers
                        .entry(((tile % 32) as u8, (tile / 32) as u8))
                        .or_default()
                        .insert(r);
                }
            }
            let mut cells = Vec::new();
            for tile in 0..960usize {
                let at = 0xf000 + tile * 2;
                let (b, a) = ([pm[at], pm[at + 1]], [mem[at], mem[at + 1]]);
                if a == b {
                    continue;
                }
                let key = ((tile % 32) as u8, (tile / 32) as u8);
                cells.push(CellChange {
                    nt: 0,
                    cx: key.0,
                    cy: key.1,
                    before: [b[0], b[1], 0, 0],
                    after: [a[0], a[1], 0, 0],
                    writers: writers
                        .remove(&key)
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
                });
            }
            run.frames.push(FrameFacts {
                frame: f,
                play,
                pad: (input.buttons_word() & 0xff) as u8,
                camera: [0, 0],
                scroll: [0, 0],
                mirroring: 0,
                ram_changed,
                counters,
                ents,
                objects,
                cells,
                apu: sound.remove(&f).unwrap_or_default(),
                code,
            });
            true
        },
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(e) = failed {
        bail!("{e}");
    }
    if !ok {
        bail!("the recording has no frame trace");
    }
    Ok(Built {
        run,
        range,
        short: opened.short(),
        id: opened.entry.id.to_hex(),
        no_code_why,
    })
}

/// How the model draws each entity kind (by `kinds`) on an arcade game,
/// for `gp gaps drawn`: from the arcade lift's map and the sprite-set
/// `gp sprites capture` wrote there, since the NES box reader
/// (gp_look::present) reads CHR tiles and an NES map. Each kind's sprite is
/// the one of the state its lives start in (its machine's `initial`), else
/// its own: a frame of the set is laid out from the position, each piece a
/// whole MCR sprite, 16 by 16 in the game's units, so its box is the
/// pieces' bounds with their top left as the origin; tiles of a pictures
/// slot are `width` to a row of 16 by 16, stood on the body's bottom edge.
/// A kind with no sprite isn't drawn by the model and isn't checked.
pub(crate) fn model_boxes(
    args: &GapsArgs,
    t: &Template,
    kinds: &[String],
    paths: &Paths,
) -> Result<Vec<Option<gp_gaps::drawn::ModelBox>>> {
    let dir = match &args.assets {
        Some(a) => a.clone(),
        None => paths.home.join("work/assets").join(&t.source),
    };
    boxes_from(&dir, &t.model, kinds)
}

/// [`model_boxes`] from a lifted folder.
fn boxes_from(
    dir: &std::path::Path,
    model: &gp_model::Model,
    kinds: &[String],
) -> Result<Vec<Option<gp_gaps::drawn::ModelBox>>> {
    const SIDE: i32 = 16;
    let map_file = dir.join("assets.toml");
    let text = std::fs::read_to_string(&map_file).with_context(|| {
        format!(
            "{} (lift the model's assets with gp assets lift, or name them with --assets)",
            map_file.display()
        )
    })?;
    if !gp_assets::arcade::is_arcade(&text) {
        bail!("{} isn't an arcade lift", map_file.display());
    }
    let map = gp_assets::arcade::ArcadeMap::parse(&text)
        .with_context(|| map_file.display().to_string())?;
    let asset_of = |slot: &str| -> Option<&gp_assets::arcade::ArcadeAsset> {
        let from = model.assets.get(slot)?.from.as_ref()?;
        map.assets.iter().find(|a| a.name == from.asset)
    };
    let mut sets: BTreeMap<String, gp_assets::sprite_set::SpriteSet> = BTreeMap::new();
    let mut out = Vec::new();
    for name in kinds {
        let Some(e) = model.entities.get(name) else {
            out.push(None);
            continue;
        };
        let e = e
            .core
            .machine
            .as_ref()
            .and_then(|m| e.in_state(&m.initial).ok())
            .unwrap_or_else(|| e.clone());
        let body = [
            (e.core.size[0].raw() >> 16) as i32,
            (e.core.size[1].raw() >> 16) as i32,
        ];
        let Some(s) = &e.core.sprite else {
            out.push(None);
            continue;
        };
        let b = if let Some(f) = &s.frame {
            let a = asset_of(&f.slot).with_context(|| {
                format!(
                    "{name}'s sprite frame names slot `{}`, which no asset of the arcade map fills",
                    f.slot
                )
            })?;
            if a.format != gp_assets::arcade::Format::SpriteSet {
                bail!("{name}'s sprite frame names `{}`, not a sprite-set", a.name);
            }
            if !sets.contains_key(&a.name) {
                let file = dir.join(a.file());
                let st = std::fs::read_to_string(&file).with_context(|| {
                    format!("{} (write it with gp sprites capture)", file.display())
                })?;
                let set = gp_assets::sprite_set::SpriteSet::parse(&st)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
                sets.insert(a.name.clone(), set);
            }
            let Some(fr) = sets[&a.name].frames.get(f.row as usize) else {
                bail!("{name}'s sprite frame is row {}, past {}", f.row, a.name);
            };
            if fr.pieces.is_empty() {
                out.push(None);
                continue;
            }
            let x0 = fr.pieces.iter().map(|p| p.x).min().unwrap_or(0);
            let y0 = fr.pieces.iter().map(|p| p.y).min().unwrap_or(0);
            let x1 = fr.pieces.iter().map(|p| p.x + SIDE).max().unwrap_or(0);
            let y1 = fr.pieces.iter().map(|p| p.y + SIDE).max().unwrap_or(0);
            gp_gaps::drawn::ModelBox {
                body,
                picture: [x1 - x0, y1 - y0],
                origin: Some([x0, y0]),
            }
        } else {
            if s.tiles.is_empty() {
                out.push(None);
                continue;
            }
            let w = s.width.max(1) as i32;
            let rows = (s.tiles.len() as i32 + w - 1) / w;
            gp_gaps::drawn::ModelBox {
                body,
                picture: [w * SIDE, rows * SIDE],
                origin: None,
            }
        };
        out.push(Some(b));
    }
    Ok(out)
}

/// The run with each mapped object's position where the MCR board would
/// draw a sprite with those bytes, `(x - 3, 241 - y)`: the sprite list's
/// units, which the original's objects are in. A mapping reads a position
/// in the sprite RAM's own units, as the game's object slots hold it.
pub(crate) fn on_screen(run: &Run) -> Run {
    let mut r = run.clone();
    for f in &mut r.frames {
        for e in &mut f.ents {
            e.x -= 3;
            e.y = 241 - e.y;
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up arcade model and lift: a hero drawn from a captured
    /// frame of two whole sprites, a block drawn from two tiles of the
    /// pictures slot, and a kind with no sprite. Each box is in the game's
    /// units, a sprite 16 by 16; positions land where the board draws
    /// their bytes.
    #[test]
    fn arcade_boxes_come_from_the_sprite_set_and_the_map() {
        let dir = tempfile::tempdir().unwrap();
        let map = format!(
            "{}\n[[asset]]\nname = \"set\"\nformat = \"sprite-set\"\n",
            gp_assets::arcade::mcr_91490_map("made-up", "Made Up", &["madeup"])
        );
        std::fs::write(dir.path().join("assets.toml"), map).unwrap();
        std::fs::write(
            dir.path().join("set.toml"),
            "schema = 1\nasset = \"set\"\n\n[[frame]]\n\n[[frame.pieces]]\ntile = 3\nslot = \"sprites\"\nx = -4\ny = -13\npalette = 4\n\n[[frame.pieces]]\ntile = 4\nslot = \"sprites\"\nx = 12\ny = 0\npalette = 4\n",
        )
        .unwrap();
        let model = gp_model::Model::parse(
            r#"
[model]
schema = "0.4.2"
name = "madeup"
title = "Made Up"
source = "made-up"
view = "side"
frame_rate = "30"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]

[model.extensions]
platformer = "0.4"

[input]
device = "arcade"

[input.actions]
left = "left"

[assets.set]
kind = "sprite-set"
role = "graphics"
from = { source = "made-up", asset = "set" }

[assets.sprites]
kind = "tiles"
role = "graphics"
from = { source = "made-up", asset = "sprites" }

[entities.hero]
role = "player"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }
sprite = { slot = "sprites", width = 1, palette = 4, frame = { slot = "set", row = 0 } }

[entities.block]
role = "npc"
size = [32, 16]
hitbox = { x = 0, y = 0, w = 32, h = 16 }
sprite = { slot = "sprites", tiles = [1, 2], width = 2, palette = 4 }

[entities.ghost]
role = "npc"
size = [8, 8]
hitbox = { x = 0, y = 0, w = 8, h = 8 }

[levels.main]
size = [32, 30]
tile_size = 16
"#,
        )
        .unwrap();
        let kinds: Vec<String> = ["hero", "block", "ghost"].map(String::from).to_vec();
        let b = boxes_from(dir.path(), &model, &kinds).unwrap();
        let hero = b[0].unwrap();
        assert_eq!(hero.picture, [32, 29]);
        assert_eq!(hero.top_left(), [-4, -13]);
        let block = b[1].unwrap();
        assert_eq!(block.picture, [32, 16]);
        assert_eq!(block.top_left(), [0, 0]);
        assert!(b[2].is_none());

        let mut run = Run::default();
        run.frames.push(FrameFacts {
            ents: vec![Ent {
                kind: Some(0),
                player: true,
                slot: None,
                id: None,
                x: 100,
                y: 48,
            }],
            ..FrameFacts::default()
        });
        let e = &on_screen(&run).frames[0].ents[0];
        assert_eq!((e.x, e.y), (97, 193));
    }
}
