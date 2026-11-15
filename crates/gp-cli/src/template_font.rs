//! `gp template font`: the font check (docs/templates.md, "The font
//! check"). A template's HUD font is a charset (gp_assets::charset) over
//! a bank of tiles: each character is drawn by one tile of the bank. This
//! checks, for any template whose model has `present.hud.font`:
//!
//! - every character the model writes has a glyph: the HUD's counters
//!   (the ten digits and their pad), its fixed text and level labels, and
//!   each `present.screens` text (a two-player turn screen's PLAYER 1);
//! - no two characters share a tile, and every charset in the asset map
//!   gives a character the font's tile;
//! - a glyph a written character uses has ink in the bank (a space aside);
//! - no other art of the model draws a glyph's tile: a sprite or a body
//!   drawn in name-table tiles, an effect, a level's metatiles. A glyph
//!   may only sit in a tile nothing else draws, and the check lists the
//!   bank's free tiles (drawn by nothing, no glyph, not the blank) for a
//!   generator that needs a new glyph.
//!
//! Rivet Rodeo's re-derive of 2026-11-07 drew the fallen thrower's poses
//! as rows of tile 0, the font's `0`, so the paper theme's paint gave
//! tile 0 the thrower's shape and every 0 of the score became a block.
//!
//! The last two need the template's lifted files: an original's are built
//! in memory from its authored.toml, a ROM template's lifted from its ROM
//! in $GP_ROM_DIR. With no ROM those two are skipped and say so.
//!
//! Ported from game-printer's template_font.rs. The picture layouts that
//! game-printer read through gp-gen-bevy (retired) are read through
//! gp-look's `present::layout_cells`, its successor.
//!
//! An arcade model's font is a `board-font` slot (an `mcr-font` asset of
//! the arcade map: background tiles with a charset), checked by
//! [`check_board`] in the board's terms: the characters the model writes
//! (its screens' text and the digits of `present.board.counters`), the
//! digits those counters draw against the font's, the glyphs' ink, and
//! the tiles no screen, counter or glyph draws.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use gp_assets::AssetMap;
use gp_evidence::Paths;
use gp_model::model::Model;

use crate::template_extract::Template;

/// What the check found.
pub(crate) struct FontReport {
    /// One line a problem, naming what clashes or is missing.
    pub problems: Vec<String>,
    /// The report, for people.
    pub text: String,
    pub json: Value,
}

/// A tile number as the asset maps write it.
fn hex(t: u32) -> String {
    format!("{t:#04x}")
}

/// Runs of tile numbers, `0xf0-0xff, 0x24`.
fn ranges(tiles: &BTreeSet<u32>) -> String {
    let mut out = Vec::new();
    let mut it = tiles.iter().copied().peekable();
    while let Some(a) = it.next() {
        let mut b = a;
        while it.peek() == Some(&(b + 1)) {
            b = it.next().unwrap_or(b);
        }
        out.push(if a == b {
            hex(a)
        } else {
            format!("{}-{}", hex(a), hex(b))
        });
    }
    out.join(", ")
}

fn shown(c: char) -> String {
    if c == ' ' {
        "space".into()
    } else {
        format!("`{c}`")
    }
}

/// The asset a model slot is filled from.
fn asset_of<'a>(m: &'a Model, slot: &str) -> Option<&'a str> {
    m.assets
        .get(slot)
        .and_then(|s| s.from.as_ref())
        .map(|f| f.asset.as_str())
}

/// Every character the model writes in the HUD's font, with where it is
/// written (the first place that writes it).
pub(crate) fn written(m: &Model) -> BTreeMap<char, String> {
    let mut out: BTreeMap<char, String> = BTreeMap::new();
    let mut add = |text: &str, at: String| {
        for c in text.chars() {
            out.entry(c).or_insert_with(|| at.clone());
        }
    };
    let Some(p) = &m.present else { return out };
    if let Some(h) = &p.hud {
        for (i, f) in h.field.iter().enumerate() {
            let at = format!("present.hud.field[{i}]");
            if let Some(c) = &f.counter {
                add("0123456789", format!("{at} (counter {c})"));
                add(f.pad.as_deref().unwrap_or("0"), format!("{at}.pad"));
            } else if f.level_label {
                for (name, l) in &m.levels {
                    if let Some(label) = l.core.present.as_ref().and_then(|p| p.label.as_ref()) {
                        add(label, format!("levels.{name}.present.label"));
                    }
                }
            } else if let Some(t) = &f.text {
                add(t, format!("{at}.text"));
            }
        }
    }
    for (i, s) in p.screens.iter().enumerate() {
        for (k, t) in s.text.iter().enumerate() {
            add(
                &t.text,
                format!(
                    "present.screens[{i}].text[{k}] \"{}\" ({})",
                    t.text,
                    s.states.join(", ")
                ),
            );
        }
    }
    out
}

/// The lifted files of a template, written into a fresh folder: an
/// original's built from its authored.toml, a ROM template's lifted from
/// the ROM it accepts (or `given`). None when there is no ROM.
pub(crate) fn lifted_dir(
    t: &Template,
    given: Option<&Path>,
    paths: &Paths,
) -> Result<Result<tempfile::TempDir, String>> {
    let Some((rom_path, _)) = crate::template_extract::find_rom(t, given, paths)? else {
        return Ok(Err(
            "no lifted files (no ROM the template accepts in $GP_ROM_DIR)".into(),
        ));
    };
    let rom = fs::read(&rom_path).with_context(|| format!("reading {}", rom_path.display()))?;
    let map_path = t.dir.join(t.str("recipe", "asset_map").unwrap_or_default());
    let map_text =
        fs::read_to_string(&map_path).with_context(|| format!("reading {}", map_path.display()))?;
    // An arcade romset lifts through its arcade map, its files all its own.
    if gp_assets::arcade::is_arcade(&map_text) {
        let (files, _) =
            gp_web_lift::arcade::lift_files(&rom, &map_text).map_err(|e| anyhow::anyhow!("{e}"))?;
        let work = paths.home.join("work");
        fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
        let tmp = tempfile::Builder::new()
            .prefix("gp-font-check-")
            .tempdir_in(&work)
            .context("making a folder for the lifted files")?;
        for f in &files {
            let to = tmp.path().join(&f.name);
            if let Some(d) = to.parent() {
                fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
            }
            fs::write(&to, &f.bytes).with_context(|| format!("writing {}", to.display()))?;
        }
        fs::write(tmp.path().join("assets.toml"), &map_text)?;
        return Ok(Ok(tmp));
    }
    let map = AssetMap::parse(&map_text).map_err(anyhow::Error::msg)?;
    let (files, _) = gp_assets::lift(&rom, &map, &map_text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let work = paths.home.join("work");
    fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
    let tmp = tempfile::Builder::new()
        .prefix("gp-font-check-")
        .tempdir_in(&work)
        .context("making a folder for the lifted files")?;
    for f in &files {
        let to = tmp.path().join(&f.name);
        if let Some(d) = to.parent() {
            fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        }
        fs::write(&to, &f.bytes).with_context(|| format!("writing {}", to.display()))?;
    }
    fs::write(tmp.path().join("assets.toml"), &map_text)?;
    // A file the lift doesn't make (a sprite set `gp sprites capture`
    // writes into the lifted folder) is taken from the template's usual
    // lifted folder, where `gp theme kit` reads it too.
    let usual = paths.home.join("work").join("assets").join(&map.source);
    for a in &map.assets {
        let to = tmp.path().join(a.file());
        if !to.exists()
            && let Ok(b) = fs::read(usual.join(a.file()))
        {
            fs::write(&to, b).with_context(|| format!("writing {}", to.display()))?;
        }
    }
    let uncaptured: Vec<&str> = map
        .assets
        .iter()
        .filter(|a| matches!(a, gp_assets::Asset::SpriteSet(_)))
        .filter(|a| !tmp.path().join(a.file()).exists())
        .map(gp_assets::Asset::name)
        .collect();
    if !uncaptured.is_empty() {
        return Ok(Err(format!(
            "the sprite set {} isn't captured in {} (gp sprites capture)",
            uncaptured.join(", "),
            usual.display()
        )));
    }
    Ok(Ok(tmp))
}

/// The font check on a template. Ok(None) when its model has no HUD
/// font.
pub(crate) fn check(t: &Template, paths: &Paths) -> Result<Option<FontReport>> {
    let (m, _) = crate::theme::read_model(&t.dir)?;
    let map_path = t.dir.join(t.str("recipe", "asset_map").unwrap_or_default());
    let map_text =
        fs::read_to_string(&map_path).with_context(|| format!("reading {}", map_path.display()))?;
    if gp_assets::arcade::is_arcade(&map_text) {
        let map = gp_assets::arcade::ArcadeMap::parse(&map_text)
            .map_err(|e| anyhow::anyhow!("{}: {e}", map_path.display()))?;
        let Some(font) = board_font_slot(&m) else {
            return Ok(None);
        };
        let lifted = lifted_dir(t, None, paths)?;
        return Ok(Some(check_board(
            &m,
            &map,
            lifted
                .as_ref()
                .map(tempfile::TempDir::path)
                .map_err(String::as_str),
            &font,
        )));
    }
    let Some(hud) = m.present.as_ref().and_then(|p| p.hud.as_ref()) else {
        return Ok(None);
    };
    let map = AssetMap::parse(&map_text).map_err(anyhow::Error::msg)?;
    let lifted = lifted_dir(t, None, paths)?;
    Ok(Some(check_with(
        &m,
        &map,
        lifted
            .as_ref()
            .map(tempfile::TempDir::path)
            .map_err(String::as_str),
        &hud.font.clone(),
        &hud.tiles.clone(),
        &t.id(),
    )))
}

/// The model's `board-font` slot, the first by name when it has several.
pub(crate) fn board_font_slot(m: &Model) -> Option<String> {
    m.assets
        .iter()
        .find(|(_, s)| s.kind == gp_model::model::AssetKind::BoardFont)
        .map(|(n, _)| n.clone())
}

/// The arcade asset a slot names, when the slot is filled from this map.
fn arcade_asset<'a>(
    m: &Model,
    map: &'a gp_assets::arcade::ArcadeMap,
    slot: &str,
) -> Option<&'a gp_assets::arcade::ArcadeAsset> {
    let f = m.assets.get(slot)?.from.as_ref()?;
    (f.source == map.source)
        .then(|| map.assets.iter().find(|a| a.name == f.asset))
        .flatten()
}

/// A lifted `byte-rows` file's rows.
fn lifted_rows(text: &str) -> Result<Vec<Vec<u8>>, String> {
    #[derive(serde::Deserialize)]
    struct F {
        rows: Vec<Vec<u8>>,
    }
    toml::from_str::<F>(text)
        .map(|f| f.rows)
        .map_err(|e| e.to_string())
}

/// The font check on an arcade model: its `board-font` slot's charset
/// against what the model writes, and with the lifted files the digits
/// its board counters draw, the glyphs' ink and the free tiles of the
/// board's tile region.
pub(crate) fn check_board(
    m: &Model,
    map: &gp_assets::arcade::ArcadeMap,
    lifted: Result<&Path, &str>,
    font_slot: &str,
) -> FontReport {
    let mut problems = Vec::new();
    let mut text = String::new();
    let fail = |problems: Vec<String>| FontReport {
        text: problems.join("\n"),
        json: json!({"font": font_slot, "problems": problems}),
        problems,
    };
    let Some(font) = arcade_asset(m, map, font_slot) else {
        return fail(vec![format!(
            "the board font slot {font_slot} names no asset of the arcade map {}",
            map.source
        )]);
    };
    let Some(charset) = font.charset.as_ref() else {
        return fail(vec![format!(
            "the board font slot {font_slot} takes asset `{}`, an {} with no charset (a board-font slot takes an mcr-font)",
            font.name,
            font.format.name()
        )]);
    };
    // The font by character and by tile of the region.
    let mut by_char: BTreeMap<char, u32> = BTreeMap::new();
    let mut by_tile: BTreeMap<u32, Vec<char>> = BTreeMap::new();
    for c in (32u8..127).map(char::from) {
        if charset.tile(c).is_some()
            && let Some(t) = font.glyph(c)
        {
            by_char.insert(c, t);
            by_tile.entry(t).or_default().push(c);
        }
    }
    // What the model writes: its screens' and HUD's text, and the digits
    // of each counter the board draws.
    let mut wrote = written(m);
    let mut counters = Vec::new();
    for (name, l) in &m.levels {
        let Some(b) = l.core.present.as_ref().and_then(|p| p.board.as_ref()) else {
            continue;
        };
        for (i, c) in b.counters.iter().enumerate() {
            let at = format!("levels.{name}.present.board.counters[{i}]");
            for d in "0123456789".chars() {
                wrote
                    .entry(d)
                    .or_insert_with(|| format!("{at} (counter {})", c.counter));
            }
            counters.push((at, c));
        }
    }
    let mut missing = Vec::new();
    for (c, at) in &wrote {
        if !by_char.contains_key(c) {
            missing.push(*c);
            problems.push(format!(
                "{at} writes {}, which the font ({}) has no glyph for",
                shown(*c),
                font.name
            ));
        }
    }
    let _ = write!(
        text,
        "font       {font_slot} from asset {} (mcr-font, tiles {} to {} of gfx1): {} characters on {} tiles; the model writes {} ({}){}",
        font.name,
        hex(font.first),
        hex(font.first + font.count.saturating_sub(1)),
        by_char.len(),
        by_tile.len(),
        wrote.len(),
        wrote.keys().collect::<String>(),
        if missing.is_empty() {
            ", each with a glyph".to_owned()
        } else {
            format!(", {} with no glyph", missing.len())
        }
    );
    let blank: BTreeSet<u32> = by_char.get(&' ').copied().into_iter().collect();
    let mut json_clash = Vec::new();
    let mut free_json = Value::Null;
    match lifted {
        Err(why) => {
            let _ = write!(
                text,
                "\nskip       the counters' digits, the glyphs' ink and the free tiles: {why}"
            );
        }
        Ok(dir) => {
            let read = |a: &gp_assets::arcade::ArcadeAsset| {
                fs::read_to_string(dir.join(a.file())).map_err(|e| format!("{}: {e}", a.file()))
            };
            let mut rows_of: BTreeMap<String, Result<Vec<Vec<u8>>, String>> = BTreeMap::new();
            let mut byte = |r: &gp_model::model::RowByte, plus: u32| -> Result<u8, String> {
                let rows = rows_of.entry(r.slot.clone()).or_insert_with(|| {
                    let a = arcade_asset(m, map, &r.slot).ok_or(format!(
                        "the rows slot {} names no asset of the map",
                        r.slot
                    ))?;
                    lifted_rows(&read(a)?)
                });
                let rows = rows.as_ref().map_err(String::clone)?;
                rows.get(r.row as usize)
                    .and_then(|row| row.get((r.column + plus) as usize))
                    .copied()
                    .ok_or(format!(
                        "the rows slot {} has no byte {} of row {}",
                        r.slot,
                        r.column + plus,
                        r.row
                    ))
            };
            // Every tile the board draws besides the glyphs: the screens'
            // cells and the counters' and repeats' codes.
            let mut used: BTreeSet<u32> = BTreeSet::new();
            let mut screens = 0;
            for (slot, s) in &m.assets {
                if s.kind != gp_model::model::AssetKind::BoardScreen {
                    continue;
                }
                let Some(a) = arcade_asset(m, map, slot) else {
                    continue;
                };
                match read(a).and_then(|t| gp_assets::arcade::read_screen(&t)) {
                    Ok(cells) => {
                        screens += 1;
                        used.extend(cells.iter().map(|c| u32::from(c & 0x3ff)));
                    }
                    Err(e) => problems.push(format!("the screen {slot}: {e}")),
                }
            }
            // Each counter's digits are the font's.
            for (at, c) in &counters {
                let high = match byte(&c.high, 0) {
                    Ok(h) => u32::from(h & 3) << 8,
                    Err(e) => {
                        problems.push(format!("{at}: {e}"));
                        continue;
                    }
                };
                for d in 0..10u8 {
                    let code = if c.table {
                        byte(&c.tile, u32::from(d))
                    } else {
                        byte(&c.tile, 0).map(|b| b.wrapping_add(d))
                    };
                    let code = match code {
                        Ok(b) => u32::from(b) | high,
                        Err(e) => {
                            problems.push(format!("{at}: {e}"));
                            break;
                        }
                    };
                    let ch = char::from(b'0' + d);
                    if let Some(t) = by_char.get(&ch)
                        && *t != code
                    {
                        problems.push(format!(
                            "{at} (counter {}) draws {} with tile {}, the font with {}",
                            c.counter,
                            shown(ch),
                            hex(code),
                            hex(*t)
                        ));
                        json_clash.push(
                            json!({"tile": code, "chars": ch.to_string(), "by": at, "font": t}),
                        );
                    }
                    used.insert(code);
                }
            }
            for (name, l) in &m.levels {
                let Some(b) = l.core.present.as_ref().and_then(|p| p.board.as_ref()) else {
                    continue;
                };
                for (i, r) in b.repeats.iter().enumerate() {
                    match (byte(&r.tile, 0), byte(&r.high, 0)) {
                        (Ok(t), Ok(h)) => {
                            let code = u32::from(t) | (u32::from(h & 3) << 8);
                            if let Some(cs) = by_tile.get(&code)
                                && !blank.contains(&code)
                            {
                                problems.push(format!(
                                    "tile {} is the font's {} and levels.{name}.present.board.repeats[{i}] (counter {}) draws it too: a glyph's tile must be one nothing else draws",
                                    hex(code),
                                    cs.iter().map(|c| shown(*c)).collect::<Vec<_>>().join(", "),
                                    r.counter
                                ));
                            }
                            used.insert(code);
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            problems.push(format!("levels.{name}.present.board.repeats[{i}]: {e}"))
                        }
                    }
                }
            }
            // Ink: each written character's glyph draws something.
            match fs::read(dir.join(font.file())) {
                Err(e) => problems.push(format!("the font's tiles {}: {e}", font.file())),
                Ok(png) => match gp_assets::chr::read_png(&png, font.image_size()) {
                    Err(e) => problems.push(format!("the font's tiles {}: {e}", font.file())),
                    Ok(px) => {
                        let (w, _) = font.image_size();
                        let per_row = font.per_row.max(1);
                        let ink = |t: u32| {
                            let k = t - font.first;
                            let (tx, ty) = ((k % per_row) * 8, (k / per_row) * 8);
                            (0..8).any(|y| {
                                (0..8).any(|x| {
                                    px.get(((ty + y) * w + tx + x) as usize)
                                        .is_some_and(|v| *v != 0)
                                })
                            })
                        };
                        for c in wrote.keys() {
                            if *c == ' ' {
                                continue;
                            }
                            if let Some(t) = by_char.get(c)
                                && !ink(*t)
                            {
                                problems.push(format!(
                                    "the glyph for {} (tile {}) draws nothing in {}",
                                    shown(*c),
                                    hex(*t),
                                    font.file()
                                ));
                            }
                        }
                    }
                },
            }
            // Free tiles of the board's tile region: no screen, counter or
            // glyph draws them.
            used.extend(by_tile.keys());
            let tiles = map
                .assets
                .iter()
                .filter(|a| a.region == "gfx1")
                .map(|a| a.first + a.count)
                .max()
                .unwrap_or(0);
            let free: BTreeSet<u32> = (0..tiles).filter(|t| !used.contains(t)).collect();
            let _ = write!(
                text,
                "\ncounters   {} drawn by the board, {} of their digits not the font's{}",
                counters.len(),
                json_clash.len(),
                if json_clash.is_empty() {
                    ""
                } else {
                    ", listed above"
                }
            );
            let _ = write!(
                text,
                "\nfree       {} tiles of the board's {tiles} that no screen ({screens} read), no counter and no glyph draws, for a new glyph: {}",
                free.len(),
                if free.is_empty() {
                    "none".to_owned()
                } else {
                    ranges(&free)
                }
            );
            free_json = json!(free);
        }
    }
    let json = json!({
        "font": font_slot,
        "asset": font.name,
        "board": true,
        "characters": by_char.keys().collect::<String>(),
        "written": wrote.keys().collect::<String>(),
        "missing": missing,
        "clashes": json_clash,
        "free": free_json,
        "problems": problems,
    });
    FontReport {
        problems,
        text,
        json,
    }
}

/// How many of the model's slots name an asset the map doesn't have.
fn slots_without_assets(m: &Model, map: &AssetMap) -> usize {
    m.assets
        .values()
        .filter_map(|s| s.from.as_ref())
        .filter(|f| f.source == map.source && !map.assets.iter().any(|a| a.name() == f.asset))
        .count()
}

/// The check on a model, its asset map and its lifted files.
fn check_with(
    m: &Model,
    map: &AssetMap,
    lifted: Result<&Path, &str>,
    font_slot: &str,
    hud_tiles: &str,
    game: &str,
) -> FontReport {
    let mut problems = Vec::new();
    let mut text = String::new();
    let font_asset = asset_of(m, font_slot).unwrap_or_default().to_owned();
    let charset = map
        .assets
        .iter()
        .find(|a| a.name() == font_asset)
        .and_then(|a| a.charset());
    let Some(charset) = charset else {
        problems.push(format!(
            "the HUD's font slot {font_slot} takes asset `{font_asset}`, which has no charset in the asset map"
        ));
        return FontReport {
            text: problems[0].clone(),
            json: json!({"font": font_slot, "problems": problems}),
            problems,
        };
    };
    // The font by character and by tile.
    let mut by_char: BTreeMap<char, u32> = BTreeMap::new();
    let mut by_tile: BTreeMap<u32, Vec<char>> = BTreeMap::new();
    for c in (32u8..127).map(char::from) {
        if let Some(t) = charset.tile(c) {
            by_char.insert(c, u32::from(t));
            by_tile.entry(u32::from(t)).or_default().push(c);
        }
    }
    if let Err(e) = charset.validate(&font_asset) {
        problems.push(e);
    }
    for (tile, cs) in &by_tile {
        if cs.len() > 1 {
            problems.push(format!(
                "the font gives tile {} to {} characters: {}",
                hex(*tile),
                cs.len(),
                cs.iter().map(|c| shown(*c)).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    // Every other charset in the map draws a character with the font's tile.
    for a in &map.assets {
        let Some(cs) = a.charset() else { continue };
        if a.name() == font_asset {
            continue;
        }
        for (c, t) in &by_char {
            if let Some(o) = cs.tile(*c)
                && u32::from(o) != *t
            {
                problems.push(format!(
                    "asset {}'s charset draws {} with tile {}, the font with {}",
                    a.name(),
                    shown(*c),
                    hex(u32::from(o)),
                    hex(*t)
                ));
            }
        }
    }
    // Every character the model writes has a glyph.
    let wrote = written(m);
    let mut missing = Vec::new();
    for (c, at) in &wrote {
        if !by_char.contains_key(c) {
            missing.push(c);
            problems.push(format!(
                "{at} writes {}, which the font ({font_asset}) has no glyph for",
                shown(*c)
            ));
        }
    }
    let _ = write!(
        text,
        "font       {font_slot} from asset {font_asset}: {} characters on {} tiles; the model writes {} ({}){}",
        by_char.len(),
        by_tile.len(),
        wrote.len(),
        wrote.keys().collect::<String>(),
        if missing.is_empty() {
            ", each with a glyph".to_owned()
        } else {
            format!(", {} with no glyph", missing.len())
        }
    );
    let blank: BTreeSet<u32> = by_char.get(&' ').copied().into_iter().collect();
    let mut json_clash = Vec::new();
    let mut free_json = Value::Null;
    match lifted {
        Err(why) => {
            let _ = write!(
                text,
                "\nskip       the glyphs' ink and what else draws their tiles: {why}"
            );
        }
        Ok(_) if slots_without_assets(m, map) > 0 => {
            problems.push(format!(
                "the art can't be read: {} of the model's slots name an asset the asset map doesn't have (gp template check names them)",
                slots_without_assets(m, map)
            ));
        }
        Ok(dir) => {
            let kit = crate::theme_kit::kit_of(m, game);
            match crate::theme_kit::local_of(m, &kit, dir, map) {
                Err(e) => problems.push(format!("reading the art from the lifted files: {e:#}")),
                Ok(local) => {
                    let bank = asset_of(m, hud_tiles).unwrap_or_default().to_owned();
                    let in_bank = |slot: &str| asset_of(m, slot) == Some(bank.as_str());
                    // The model's names for the kit's scrubbed entity ids.
                    let names = crate::theme_kit::names_of(m);
                    let named: BTreeMap<String, String> = m
                        .entities
                        .iter()
                        .map(|(n, e)| {
                            (
                                crate::theme_kit::entity_id(n, &e.core.role, &names),
                                format!("entity {n}"),
                            )
                        })
                        .collect();
                    // Each tile of the font's bank, and the art that draws it.
                    let mut drawn: BTreeMap<u32, BTreeMap<String, Vec<usize>>> = BTreeMap::new();
                    for (id, cuts) in &local.cuts {
                        if id == "font" {
                            continue;
                        }
                        let who = named.get(id).cloned().unwrap_or_else(|| id.clone());
                        for (k, c) in cuts.iter().enumerate() {
                            if !in_bank(&c.slot) {
                                continue;
                            }
                            for p in &c.places {
                                let f = drawn
                                    .entry(p.tile)
                                    .or_default()
                                    .entry(who.clone())
                                    .or_default();
                                if f.last() != Some(&k) {
                                    f.push(k);
                                }
                            }
                        }
                    }
                    for (i, (role, c)) in local.objects.iter().enumerate() {
                        if !in_bank(&c.slot) {
                            continue;
                        }
                        for p in &c.places {
                            drawn
                                .entry(p.tile)
                                .or_default()
                                .entry(format!("metatile {i} ({role})"))
                                .or_default();
                        }
                    }
                    for (tile, cs) in &by_tile {
                        if blank.contains(tile) {
                            continue;
                        }
                        let Some(who) = drawn.get(tile) else { continue };
                        for (w, frames) in who {
                            let fr = if frames.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    " (frame{} {})",
                                    if frames.len() > 1 { "s" } else { "" },
                                    frames
                                        .iter()
                                        .map(usize::to_string)
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            };
                            problems.push(format!(
                                "tile {} is the font's {} and {w} draws it too{fr}: a glyph's tile must be one nothing else draws",
                                hex(*tile),
                                cs.iter().map(|c| shown(*c)).collect::<Vec<_>>().join(", ")
                            ));
                            json_clash.push(json!({"tile": tile, "chars": cs.iter().collect::<String>(), "by": w, "frames": frames}));
                        }
                    }
                    // Ink: each written character's glyph draws something.
                    let size = crate::theme_kit::bank_size(map, m, hud_tiles);
                    let png = map
                        .assets
                        .iter()
                        .find(|a| a.name() == bank)
                        .and_then(|a| fs::read(dir.join(a.file())).ok());
                    match (size, png) {
                        (Some(sz), Some(png)) => {
                            match gp_assets::chr::read_png(&png, (sz[0] * 8, sz[1] * 8)) {
                                Err(e) => problems.push(format!("the font's tiles {bank}: {e}")),
                                Ok(px) => {
                                    let w = (sz[0] * 8) as usize;
                                    let ink = |t: u32| {
                                        let (tx, ty) =
                                            ((t % sz[0]) as usize * 8, (t / sz[0]) as usize * 8);
                                        (0..8).any(|y| {
                                            (0..8).any(|x| {
                                                px.get((ty + y) * w + tx + x)
                                                    .is_some_and(|v| *v != 0)
                                            })
                                        })
                                    };
                                    for c in wrote.keys() {
                                        if *c == ' ' {
                                            continue;
                                        }
                                        if let Some(t) = by_char.get(c)
                                            && !ink(*t)
                                        {
                                            problems.push(format!(
                                            "the glyph for {} (tile {}) draws nothing in {bank}",
                                            shown(*c),
                                            hex(*t)
                                        ));
                                        }
                                    }
                                }
                            }
                        }
                        _ => problems.push(format!(
                            "the font's tiles {bank} aren't in the lifted files"
                        )),
                    }
                    // Free tiles: drawn by no art, no picture, and no glyph.
                    let mut used: BTreeSet<u32> = drawn.keys().copied().collect();
                    used.extend(by_tile.keys());
                    used.extend(&blank);
                    let mut pictures = 0;
                    for a in kit.art.iter().filter(|a| a.kind == "picture") {
                        let Some((slot, entry)) =
                            a.id.strip_prefix("picture.")
                                .and_then(|r| r.split_once('.'))
                        else {
                            continue;
                        };
                        if let Ok((cells, _)) = gp_look::present::layout_cells(m, dir, slot, entry)
                        {
                            pictures += 1;
                            used.extend(cells.iter().flatten().map(|t| u32::from(*t)));
                        }
                    }
                    let tiles = size.map_or(256, |s| s[0] * s[1]);
                    let free: BTreeSet<u32> = (0..tiles).filter(|t| !used.contains(t)).collect();
                    let clashes = json_clash.len();
                    let _ = write!(
                        text,
                        "\nglyphs     {} drawn by other art too{}",
                        clashes,
                        if clashes == 0 {
                            " (none)"
                        } else {
                            ", listed above"
                        }
                    );
                    let _ = write!(
                        text,
                        "\nfree       {} tiles of {bank} that no art, no picture ({pictures} read) and no glyph draws, for a new glyph: {}",
                        free.len(),
                        if free.is_empty() {
                            "none".to_owned()
                        } else {
                            ranges(&free)
                        }
                    );
                    free_json = json!(free);
                }
            }
        }
    }
    let json = json!({
        "font": font_slot,
        "asset": font_asset,
        "characters": by_char.keys().collect::<String>(),
        "written": wrote.keys().collect::<String>(),
        "missing": missing,
        "clashes": json_clash,
        "free": free_json,
        "problems": problems,
    });
    FontReport {
        problems,
        text,
        json,
    }
}

/// `gp template font DIR`.
pub(crate) fn font_cmd(dir: &Path, paths: &Paths) -> Result<crate::Outcome> {
    let t = Template::load(dir)?;
    let id = t.id();
    let Some(r) = check(&t, paths)? else {
        return Ok(crate::Outcome {
            json: json!({"ok": true, "id": id, "font": null}),
            text: format!(
                "{id}: no HUD font (present.hud.font) and no board font (a board-font slot), nothing to check"
            ),
            exit: 0,
        });
    };
    let ok = r.problems.is_empty();
    let mut text = String::new();
    for p in &r.problems {
        let _ = writeln!(text, "FAIL  {id}: {p}");
    }
    let _ = write!(text, "{}", r.text);
    let _ = write!(
        text,
        "\n{}",
        if ok {
            format!("ok: {id}'s font check passed")
        } else {
            format!("FAIL: {id}'s font check: {} problem(s)", r.problems.len())
        }
    );
    let mut json = r.json;
    json["ok"] = json!(ok);
    json["id"] = json!(id);
    Ok(crate::Outcome {
        json,
        text,
        exit: if ok { 0 } else { 1 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up arcade model: a board font, a screen, and two counters
    /// whose digit codes come from a rows slot. No game's bytes.
    const MODEL: &str = r#"
[model]
schema = "0.4"
name = "cab"
title = "Cab"
description = "A made-up arcade game"
source = "cab"
view = "side"
frame_rate = "30"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]
start_level = "main"

[model.extensions]
platformer = "0.4"

[input]
device = "arcade"

[input.actions]
left = "left"

[assets.tiles]
kind = "board-tiles"
role = "graphics"
[assets.tiles.from]
source = "cab"
asset = "tiles"

[assets.letters]
kind = "board-font"
role = "graphics"
[assets.letters.from]
source = "cab"
asset = "letters"

[assets.stage]
kind = "board-screen"
role = "layout"
[assets.stage.from]
source = "cab"
asset = "stage"

[assets.colors]
kind = "board-palette"
role = "palettes"
[assets.colors.from]
source = "cab"
asset = "colors"

[assets.codes]
kind = "rows"
role = "graphics"
[assets.codes.from]
source = "cab"
asset = "codes"

[counters.score]
[counters.lives]

[entities.player]
role = "player"
size = [16, 16]

[entities.player.hitbox]
x = 0
y = 0
w = 16
h = 16

[levels.main]
description = "A screen"
empty = true
size = [32, 30]
tile_size = 16

[levels.main.present.board]
palette = "colors"
tiles = "tiles"
screen = "stage"

[[levels.main.present.board.counters]]
counter = "score"
at = [2, 1]
digits = 4
tile = { slot = "codes" }
high = { slot = "codes", column = 1 }

[[levels.main.present.board.counters]]
counter = "lives"
at = [20, 1]
digits = 1
tile = { slot = "codes", column = 2 }
high = { slot = "codes", column = 1 }

[[levels.main.spawn]]
entity = "player"
x = 53
y = 191
"#;

    const MAP: &str = "schema = 1\nplatform = \"arcade\"\nsource = \"cab\"\nsets = [\"cab\"]\n\n\
        [[asset]]\nname = \"tiles\"\nformat = \"mcr-tiles\"\nregion = \"gfx1\"\nfirst = 0\ncount = 64\nper_row = 16\n\n\
        [[asset]]\nname = \"letters\"\nformat = \"mcr-font\"\nregion = \"gfx1\"\nfirst = 0x20\ncount = 16\nper_row = 16\n\
        charset = [[0, \"0123456789\"], [10, \" \"]]\n\n\
        [[asset]]\nname = \"stage\"\nformat = \"mcr-screen\"\nregion = \"maincpu\"\noffset = 0\n\n\
        [[asset]]\nname = \"colors\"\nformat = \"mcr-palette\"\nregion = \"maincpu\"\noffset = 0\n\n\
        [[asset]]\nname = \"codes\"\nformat = \"byte-rows\"\nregion = \"maincpu\"\noffset = 0\ncount = 1\nwidth = 3\n";

    /// The lifted folder: glyphs 0 to 9 inked but 5, a screen of tiles 0
    /// to 3, the score's digit 0 at code 0x20 (the font's) and the lives'
    /// at 0x21 (not the font's).
    fn lift(dir: &Path) {
        let mut px = vec![0u8; 128 * 8];
        for g in (0..10).filter(|g| *g != 5) {
            px[g * 8 + 3] = 1;
        }
        fs::write(
            dir.join("letters.png"),
            gp_assets::chr::write_png(128, 8, &px),
        )
        .unwrap();
        let rows: Vec<String> = (0..30)
            .map(|r| {
                let cells: Vec<String> = (0..32).map(|c| ((r + c) % 4).to_string()).collect();
                format!("[{}]", cells.join(", "))
            })
            .collect();
        fs::write(
            dir.join("stage.toml"),
            format!("rows = [{}]\n", rows.join(",\n")),
        )
        .unwrap();
        fs::write(dir.join("codes.toml"), "rows = [[0x20, 0x00, 0x21]]\n").unwrap();
    }

    /// The board font check finds the counters' digits, a glyph with no
    /// ink, a counter drawing a digit with another tile than the font's,
    /// a screen's text with no glyph, and the tiles nothing draws.
    #[test]
    fn a_board_font_is_checked_in_the_boards_terms() {
        let text = MODEL.replace(
            "[entities.player]",
            "[[present.screens]]\nstates = [\"title\"]\n\n[[present.screens.text]]\nat = [1, 1]\ntext = \"GO 1\"\n\n[entities.player]",
        );
        let m = Model::parse(&text).unwrap();
        let map = gp_assets::arcade::ArcadeMap::parse(MAP).unwrap();
        assert_eq!(board_font_slot(&m).as_deref(), Some("letters"));
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let r = check_board(&m, &map, Ok(dir.path()), "letters");
        let has = |s: &str| r.problems.iter().any(|p| p.contains(s));
        assert!(
            has("writes `G`, which the font (letters) has no glyph for"),
            "{:?}",
            r.problems
        );
        assert!(
            has("the glyph for `5` (tile 0x25) draws nothing"),
            "{:?}",
            r.problems
        );
        assert!(
            has("counters[1] (counter lives) draws `0` with tile 0x21, the font with 0x20"),
            "{:?}",
            r.problems
        );
        assert!(!has("counters[0]"), "{:?}", r.problems);
        assert_eq!(r.json["characters"], " 0123456789");
        // Tiles 0 to 3 are the screen's and 0x20 to 0x2A the glyphs' (the
        // lives' codes 0x21 to 0x2A among them): the rest of the 64 are free.
        let free: Vec<u32> = serde_json::from_value(r.json["free"].clone()).unwrap();
        assert_eq!(free.len(), 64 - 4 - 11);
        assert!(
            !free.contains(&3)
                && free.contains(&4)
                && !free.contains(&0x2A)
                && free.contains(&0x2B)
        );
        // Without the lifted files the check says what it skipped.
        let r = check_board(&m, &map, Err("no romset"), "letters");
        assert!(r.text.contains("skip") && r.text.contains("no romset"));
        assert!(!r.problems.iter().any(|p| p.contains("draws nothing")));
    }
}
