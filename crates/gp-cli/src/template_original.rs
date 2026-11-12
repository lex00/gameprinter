//! `gp template check` with game-printer's options back: the ROM found by
//! the template's accepted hash when `--rom` isn't given, and `--original`,
//! the layout originality check of a derived template's authored stages
//! against its base's, lifted in memory from the person's own ROM of the
//! base and never written anywhere.
//!
//! Ported from game-printer's stage_measure.rs (`check_original`) and
//! template.rs (`base_lifted`). Changed by gameprinter's layout: the base
//! (`[derive] from` in template.toml) is a game's package folder, whose
//! ROM is its scored entry found in $GP_ROM_DIR and whose asset map is that
//! entry's; the template's own built files are read from where `gp
//! template build` writes them, `$GP_HOME/work/assets/<source>`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use gp_evidence::Paths;
use gp_interp::Lifted;
use gp_interp::stage::{self, Cells, Layout, Similarity};
use gp_model::Model;
use gp_sim::Spec;
use serde_json::json;

use crate::Outcome;

/// The share of cells at which a level is refused as the base's.
pub(crate) const CELLS_REFUSE: f64 = 0.6;
/// The share of ladders at which a level is refused as the base's.
pub(crate) const LADDERS_REFUSE: f64 = 0.5;

fn manifest(dir: &Path) -> Result<toml::Table> {
    let f = dir.join("template.toml");
    fs::read_to_string(&f)
        .with_context(|| format!("reading {}", f.display()))?
        .parse()
        .with_context(|| f.display().to_string())
}

/// `gp template check DIR [--rom ROM] [--original]`.
pub(crate) fn check_cmd(
    dir: &Path,
    rom: Option<&Path>,
    original: bool,
    paths: &Paths,
) -> Result<Outcome> {
    let rom = match rom {
        Some(r) => r.to_path_buf(),
        None => find_rom(dir, paths)?,
    };
    let mut out = crate::template::check(dir, &rom)?;
    if original {
        check_original(dir, paths, &mut out)?;
    }
    Ok(out)
}

/// The first file in $GP_ROM_DIR whose hash the template accepts.
fn find_rom(dir: &Path, paths: &Paths) -> Result<PathBuf> {
    let m = manifest(dir)?;
    let accepts: Vec<String> = m
        .get("source")
        .and_then(|s| s.get("accepts"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let index = if paths.roms.is_dir() {
        gp_evidence::source::index_nes_roms(&paths.roms)
            .with_context(|| format!("reading {}", paths.roms.display()))?
    } else {
        Default::default()
    };
    accepts
        .iter()
        .find_map(|a| index.get(a).cloned())
        .with_context(|| {
            format!(
                "no ROM in {} has a hash {} accepts; give it with --rom",
                paths.roms.display(),
                dir.display()
            )
        })
}

/// The base a derived template names, its model and its lifted files from
/// the person's ROM of it, in memory; `None` when that ROM isn't there.
fn base_lifted(dir: &Path, paths: &Paths) -> Result<Option<(PathBuf, Model, Lifted)>> {
    let m = manifest(dir)?;
    let Some(from) = m
        .get("derive")
        .and_then(|d| d.get("from"))
        .and_then(toml::Value::as_str)
    else {
        bail!(
            "{} isn't derived from another template ([derive] in template.toml)",
            dir.display()
        );
    };
    let base_dir = dir.join(from);
    let pkg = crate::package::Package::load(&base_dir)?;
    let entry = pkg
        .entries
        .iter()
        .find(|e| e.role == "scored")
        .or(pkg.entries.first())
        .with_context(|| format!("{} names no ROM", base_dir.display()))?;
    let found = crate::roms::find(std::slice::from_ref(&entry.sha256), &[], &paths.roms)?;
    let Some(rom) = found.get(&entry.sha256) else {
        return Ok(None);
    };
    let map_path = base_dir.join(&entry.asset_map);
    let map_text =
        fs::read_to_string(&map_path).with_context(|| format!("reading {}", map_path.display()))?;
    let map = gp_assets::AssetMap::parse(&map_text).map_err(|e| anyhow!("{e}"))?;
    let (files, _) = gp_assets::lift(&rom.bytes, &map, &map_text)?;
    let mut lifted = Lifted::default();
    for f in files {
        if let Ok(text) = String::from_utf8(f.bytes) {
            lifted.files.insert(f.name, text);
        }
    }
    let file = base_dir.join(gp_model::MODEL_FILE);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let model = Model::parse(&text).map_err(|e| anyhow!("{e}"))?;
    Ok(Some((base_dir, model, lifted)))
}

fn level_of(spec: &Spec, name: &str) -> Option<(Cells, Layout)> {
    let i = spec.levels.iter().position(|l| l.name == name)?;
    Some((Cells::of(&spec.levels[i]), stage::layout(spec, i)))
}

/// `--original`: each level of a derived template that its base has too,
/// its layout against the base's, and the lines its built files share with
/// the base's lifted files.
fn check_original(dir: &Path, paths: &Paths, out: &mut Outcome) -> Result<()> {
    let Some((base_dir, base_model, lifted)) = base_lifted(dir, paths)? else {
        out.text
            .push_str("\nlayout originality: not checked (no ROM of the base in $GP_ROM_DIR)");
        out.json["layout_originality"] = json!({"checked": false});
        out.exit = out.exit.max(2);
        return Ok(());
    };
    let base = gp_interp::compile::compile(&base_model, &lifted).map_err(|e| anyhow!("{e}"))?;
    // Our lifted files (as `gp template build` writes them) against the
    // base's: no data line in common.
    let model = Model::parse(&fs::read_to_string(dir.join(gp_model::MODEL_FILE))?)
        .map_err(|e| anyhow!("{e}"))?;
    let ours_dir = paths
        .home
        .join("work")
        .join("assets")
        .join(model.model.source.clone().unwrap_or_default());
    let mut ours_files = BTreeMap::new();
    for e in fs::read_dir(&ours_dir).with_context(|| {
        format!(
            "reading {} (run gp template build first)",
            ours_dir.display()
        )
    })? {
        let p = e?.path();
        if let Ok(t) = fs::read_to_string(&p) {
            ours_files.insert(
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                t,
            );
        }
    }
    let shared = crate::template::shared_lines(&ours_files, &lifted.files);
    let _ = write!(
        out.text,
        "\nlines shared with the base's lifted files: {}",
        shared.len()
    );
    for (f, l, t) in shared.iter().take(20) {
        let _ = write!(out.text, "\n  {f}: {l} (as in {t}) REFUSED");
    }
    let ours_lifted = Lifted::read_dir(&ours_dir, &model).map_err(|e| anyhow!("{e}"))?;
    let ours = gp_interp::compile::compile(&model, &ours_lifted).map_err(|e| anyhow!("{e}"))?;
    let mut levels = Vec::new();
    let mut refused = false;
    let _ = write!(
        out.text,
        "\nlayout originality against {} (refused at cells {CELLS_REFUSE} or ladders {LADDERS_REFUSE}):",
        base_dir.display()
    );
    for l in &ours.levels {
        let (Some(a), Some(b)) = (level_of(&ours, &l.name), level_of(&base, &l.name)) else {
            continue;
        };
        let s: Similarity = stage::similarity((&a.0, &a.1), (&b.0, &b.1));
        let bad = s.cells >= CELLS_REFUSE || s.ladders >= LADDERS_REFUSE;
        refused |= bad;
        let _ = write!(
            out.text,
            "\n  {}: cells {:.3} at shift [{}, {}]{}, ladders {:.3} ({} of {}), things {:.3}{}",
            l.name,
            s.cells,
            s.shift[0],
            s.shift[1],
            if s.mirrored { " mirrored" } else { "" },
            s.ladders,
            s.ladders_matched,
            s.ladders_of,
            s.things,
            if bad { " REFUSED" } else { "" }
        );
        levels.push(json!({"level": l.name, "cells": s.cells, "shift": s.shift,
                           "mirrored": s.mirrored, "ladders": s.ladders,
                           "ladders_matched": s.ladders_matched, "ladders_of": s.ladders_of,
                           "things": s.things, "refused": bad}));
    }
    refused |= !shared.is_empty();
    out.json["layout_originality"] = json!({"checked": true, "cells_refuse": CELLS_REFUSE,
                                            "ladders_refuse": LADDERS_REFUSE, "levels": levels,
                                            "shared_lines": shared.iter().map(|(f, l, t)| json!({"file": f, "line": l, "base_file": t})).collect::<Vec<_>>()});
    if refused {
        out.exit = 1;
        if let Some(o) = out.json.get_mut("ok") {
            *o = json!(false);
        }
    }
    Ok(())
}
