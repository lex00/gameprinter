//! What the theme commands need of a game's template.toml and its ROM.
//!
//! game-printer's theme commands read templates through its
//! `crate::template` module (2,800 lines, most of it `gp template` itself).
//! gameprinter keeps a game's template.toml in the games checkout
//! (`games/GAME/template.toml`) beside its package.toml, and has no
//! `gp template` module of that shape, so this file brings over only what
//! the theme commands call: the manifest's id, accepted hashes and asset
//! map, finding the game's ROM under `$GP_ROM_DIR`, and the check that a
//! folder holds none of a ROM's bytes, which in gameprinter is
//! `gp template served` ([`crate::served::served`]) on the game's package.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use gp_evidence::source::{file_looks_like_nes, identify_nes};
use serde::Deserialize;

use crate::Outcome;

/// The parts of template.toml the theme commands read.
#[derive(Debug, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) template: Header,
    pub(crate) source: SourceDecl,
    pub(crate) recipe: Recipe,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Header {
    pub(crate) id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SourceDecl {
    #[serde(default)]
    pub(crate) how: Option<String>,
    #[serde(default)]
    pub(crate) accepts: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Recipe {
    pub(crate) asset_map: String,
}

/// A template directory with its parsed manifest.
pub(crate) struct Template {
    pub(crate) dir: PathBuf,
    pub(crate) manifest: Manifest,
}

pub(crate) fn load(dir: &Path) -> Result<Template> {
    if !dir.is_dir() {
        bail!("{} isn't a directory", dir.display());
    }
    let path = dir.join("template.toml");
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "reading {} (is {} a game's folder?)",
            path.display(),
            dir.display()
        )
    })?;
    let manifest: Manifest =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Template {
        dir: dir.to_path_buf(),
        manifest,
    })
}

/// A source ROM: its path, file bytes and headerless SHA-256.
pub(crate) struct Rom {
    pub(crate) path: PathBuf,
    pub(crate) bytes: Vec<u8>,
    pub(crate) sha256: String,
}

impl Rom {
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let img = identify_nes(&bytes).with_context(|| path.display().to_string())?;
        Ok(Self {
            path: path.to_path_buf(),
            bytes,
            sha256: img.sha256,
        })
    }
}

/// The ROM given, or the first NES image in $GP_ROM_DIR, by path, whose
/// hash the template accepts. An authored original (`how = "authored"`)
/// has no ROM here: gameprinter builds those images with `gp template
/// build`.
pub(crate) fn find_rom(t: &Template, given: Option<&Path>, paths: &Paths) -> Result<Option<Rom>> {
    if let Some(p) = given {
        return Ok(Some(Rom::read(p)?));
    }
    if t.manifest.source.how.as_deref() == Some("authored") || !paths.roms.is_dir() {
        return Ok(None);
    }
    let mut files = Vec::new();
    crate::files::walk_lenient(&paths.roms, &mut files);
    files.sort();
    for path in files {
        if !file_looks_like_nes(&path) {
            continue;
        }
        let Ok(rom) = Rom::read(&path) else { continue };
        if t.manifest.source.accepts.contains(&rom.sha256) {
            return Ok(Some(rom));
        }
    }
    Ok(None)
}

/// Whether the folder `dir` holds any of the bytes of the ROMs of the
/// templates (game folders) given, game-printer's `template::served`. Each
/// game folder is checked with gameprinter's `gp template served` on its
/// package, the folder given as `--folder`. Its lines are put the way
/// game-printer's were: `<id>: N files, B bytes, against ROM (...)`, or
/// `<id>: no accepted ROM to compare with`.
pub(crate) fn served(
    dir: &Path,
    templates: &[PathBuf],
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let mut problems = Vec::new();
    let mut text = String::new();
    let mut exit = 0u8;
    for t in templates {
        let given: Vec<PathBuf> = rom.into_iter().map(Path::to_owned).collect();
        let o = crate::served::served(t, &[dir.to_owned()], &given, paths)?;
        problems.extend(o.json["problems"].as_array().cloned().unwrap_or_default());
        for l in o.text.lines() {
            if let Some(rest) = l.strip_prefix("NOT COMPARED with ") {
                let ids = rest.split(':').next().unwrap_or("");
                for id in ids.split(", ") {
                    text.push_str(&format!("{id}: no accepted ROM to compare with\n"));
                }
            } else if l.contains(" against ") {
                text.push_str(l);
                text.push('\n');
            }
        }
        exit = exit.max(o.exit);
    }
    Ok(Outcome {
        json: serde_json::json!({ "ok": problems.is_empty(), "problems": problems }),
        text,
        exit,
    })
}

/// The games checkout, which holds the games' folders where game-printer's
/// repo held `templates/`: `$GP_GAMES_DIR`, else a games folder at, above
/// or beside the working folder ([`crate::roms_cmd::games_dir`]).
pub(crate) fn games_root() -> Option<PathBuf> {
    crate::roms_cmd::games_dir(None)
}

/// The template folders in a folder (each holding template.toml).
fn templates_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("template.toml").is_file())
        .collect();
    out.sort();
    out
}

/// Every game folder: the games checkout's, then gameprinter's own
/// `templates/` (Moss Hop).
pub(crate) fn game_dirs() -> Vec<PathBuf> {
    let mut out = games_root().map(|r| templates_in(&r)).unwrap_or_default();
    out.extend(templates_in(&crate::serve::repo_root().join("templates")));
    out
}

/// A game folder named by a template id or a game: `GAME` in the games
/// checkout, else the game folder whose template.toml has that id
/// (`donkey-kong-oe` is `games/donkey-kong`). game-printer's was
/// `templates/ID` in its repo.
pub(crate) fn find_game(name: &str) -> Option<PathBuf> {
    let roots = [
        games_root(),
        Some(crate::serve::repo_root().join("templates")),
    ];
    for r in roots.into_iter().flatten() {
        let d = r.join(name);
        if d.join("template.toml").is_file() {
            return Some(d);
        }
    }
    game_dirs()
        .into_iter()
        .find(|d| load(d).is_ok_and(|t| t.manifest.template.id == name))
}

/// The games checkout for tests: `$GP_GAMES_DIR`, else `games` beside the
/// gameprinter checkout. None (and the test passes over it) without one.
#[cfg(test)]
pub(crate) fn test_games() -> Option<PathBuf> {
    let d = std::env::var_os("GP_GAMES_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games"));
    d.join("donkey-kong/template.toml").is_file().then_some(d)
}

/// The games checkout's game folders and gameprinter's own templates, for
/// tests.
#[cfg(test)]
pub(crate) fn test_game_dirs() -> Vec<PathBuf> {
    let mut out = test_games().map(|r| templates_in(&r)).unwrap_or_default();
    out.extend(templates_in(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates"),
    ));
    out
}

/// gameprinter's own templates only (Moss Hop), for the tests that hold a
/// kit to its model: the games checkout's kits are its own data, checked
/// there (its Donkey Kong kit is older than its model: `gp theme kit`
/// refuses it with game-printer's binary too).
#[cfg(test)]
pub(crate) fn test_own_templates() -> Vec<PathBuf> {
    templates_in(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates"))
}
