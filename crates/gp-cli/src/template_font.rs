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
//! gp-look's `present::layout_cells`, its successor. An arcade map is
//! refused in words: it has no charset.

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
/// the ROM it accepts. None when there is no ROM.
fn lifted_dir(t: &Template, paths: &Paths) -> Result<Result<tempfile::TempDir, String>> {
    let Some((rom_path, _)) = crate::template_extract::find_rom(t, None, paths)? else {
        return Ok(Err(
            "no lifted files (no ROM the template accepts in $GP_ROM_DIR)".into(),
        ));
    };
    let rom = fs::read(&rom_path).with_context(|| format!("reading {}", rom_path.display()))?;
    let map_path = t.dir.join(t.str("recipe", "asset_map").unwrap_or_default());
    let map_text =
        fs::read_to_string(&map_path).with_context(|| format!("reading {}", map_path.display()))?;
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
    let Some(hud) = m.present.as_ref().and_then(|p| p.hud.as_ref()) else {
        return Ok(None);
    };
    let map_path = t.dir.join(t.str("recipe", "asset_map").unwrap_or_default());
    let map_text =
        fs::read_to_string(&map_path).with_context(|| format!("reading {}", map_path.display()))?;
    if gp_assets::arcade::is_arcade(&map_text) {
        anyhow::bail!(
            "{} is an arcade asset map: the font check reads an NES charset over a tile bank (gp_assets::charset), and an arcade map has no charset yet",
            map_path.display()
        );
    }
    let map = AssetMap::parse(&map_text).map_err(anyhow::Error::msg)?;
    let lifted = lifted_dir(t, paths)?;
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
            text: format!("{id}: no HUD font (present.hud.font), nothing to check"),
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
