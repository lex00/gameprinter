//! What the ROM-tier tests (profiles.rs, clip_agree.rs) share: the game
//! they run on, found in the games checkout by its checks.toml, and its ROM.
//! The tool knows no game by name, so a test reads the game's facts from
//! the game's own folder.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn games() -> PathBuf {
    std::env::var_os("GP_GAMES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("../games"))
}

pub fn read_toml(path: &Path) -> Option<toml::Table> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}

/// The game and its checks.toml table `[TABLE]`: $GP_GAME_DIR, else the
/// first game of the games checkout that has the table.
pub fn game_with(table: &str) -> Result<(PathBuf, toml::Table), String> {
    let has = |dir: &Path| {
        read_toml(&dir.join("checks.toml"))
            .and_then(|t| t.get(table).and_then(|v| v.as_table()).cloned())
    };
    if let Some(dir) = std::env::var_os("GP_GAME_DIR").map(PathBuf::from) {
        return has(&dir)
            .map(|t| (dir.clone(), t))
            .ok_or_else(|| format!("{} has no checks.toml with [{table}]", dir.display()));
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(games())
        .map_err(|_| format!("no games checkout at {}", games().display()))?
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .find_map(|d| has(&d).map(|t| (d, t)))
        .ok_or_else(|| {
            format!(
                "no game in the games checkout at {} has a checks.toml with [{table}]",
                games().display()
            )
        })
}

/// The ROM the game's corpus.toml names, from `gp roms scan`.
pub fn corpus_rom(game: &Path, scan: &serde_json::Value) -> Option<PathBuf> {
    let sha = read_toml(&game.join("corpus.toml"))?
        .get("target")?
        .get("rom")?
        .as_str()?
        .to_owned();
    let r = scan["roms"]
        .as_array()?
        .iter()
        .find(|r| r["sha256"] == sha.as_str())?;
    Some(Path::new(scan["roms_dir"].as_str()?).join(r["path"].as_str()?))
}
