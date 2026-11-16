//! `gp mechanic autoplay`: runs the game's own auto-player. The tools hold
//! no game's play: a game that has one keeps it in its folder of the games
//! checkout, as its checks.toml's `[autoplay]` says, and this finds the
//! game by the romset `--rom` names (its identity in a game's roms.toml)
//! and runs it with the flags as given. `gp mechanic triage` runs this.
//!
//! ```toml
//! [autoplay]
//! manifest = "autoplay/Cargo.toml"   # a cargo package gp builds and runs
//! # or: bin = "autoplay/bin/player"  # a program, from the game's folder
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_json::json;

use crate::Outcome;

/// What a game's checks.toml says of its auto-player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Player {
    /// A cargo package in the game's folder, built and run with cargo.
    Manifest(PathBuf),
    /// A program, from the game's folder.
    Bin(PathBuf),
}

const HELP: &str = "gp mechanic autoplay --rom ZIP [FLAGS]: runs the auto-player of the game whose romset ZIP is (its identity in a game's roms.toml, in the games checkout). The game's checks.toml [autoplay] names it: `manifest = \"DIR/Cargo.toml\"`, a cargo package gp builds and runs, or `bin = \"PATH\"`, a program; both from the game's folder. The flags are the player's own (with --rom ZIP --help it lists them). A game without one has no auto-player, and this says so.";

/// The `[autoplay]` of the game folder `game`, or why there is none.
pub(crate) fn player_of(game: &Path) -> Result<Player> {
    let file = game.join("checks.toml");
    let text = fs::read_to_string(&file)
        .with_context(|| format!("{} has no auto-player: no checks.toml", game.display()))?;
    let t: toml::Table =
        toml::from_str(&text).with_context(|| format!("reading {}", file.display()))?;
    let Some(a) = t.get("autoplay").and_then(|a| a.as_table()) else {
        bail!(
            "{} has no auto-player: its checks.toml has no [autoplay]",
            game.display()
        );
    };
    let path = |k: &str| a.get(k).and_then(|v| v.as_str()).map(|p| game.join(p));
    match (path("manifest"), path("bin")) {
        (Some(m), None) => Ok(Player::Manifest(m)),
        (None, Some(b)) => Ok(Player::Bin(b)),
        _ => bail!(
            "{}: [autoplay] gives one of manifest (a cargo package) or bin (a program)",
            file.display()
        ),
    }
}

/// The game folder of `games` whose roms.toml gives the identity `sha256`.
pub(crate) fn game_of(games: &Path, sha256: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(games)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort();
    let line = format!("sha256 = \"{sha256}\"");
    dirs.into_iter().find(|d| {
        fs::read_to_string(d.join("roms.toml"))
            .is_ok_and(|t| t.lines().any(|l| l.trim().eq_ignore_ascii_case(&line)))
    })
}

pub(crate) fn run(args: &[String]) -> Result<Outcome> {
    let rom = args
        .windows(2)
        .find(|w| w[0] == "--rom")
        .map(|w| PathBuf::from(&w[1]));
    let Some(rom) = rom else {
        if args.iter().any(|a| a == "--help" || a == "-h") {
            return Ok(Outcome::ok(json!({"help": HELP}), HELP.to_owned()));
        }
        bail!("{HELP}");
    };
    let games = crate::roms_cmd::games_dir(None).context(
        "no games checkout: set GP_GAMES_DIR, whose games' folders hold their auto-players",
    )?;
    let set = gp_arcade::Romset::load_zip(&rom).map_err(|e| anyhow::anyhow!("{e}"))?;
    let game = game_of(&games, &set.identity).with_context(|| {
        format!(
            "no game in {} names the romset {} ({}) in its roms.toml",
            games.display(),
            set.name,
            rom.display()
        )
    })?;
    let mut cmd = match player_of(&game)? {
        Player::Manifest(m) => {
            let mut c = std::process::Command::new("cargo");
            c.args(["run", "-q", "--release", "--manifest-path"])
                .arg(&m)
                .arg("--");
            c
        }
        Player::Bin(b) => std::process::Command::new(b),
    };
    let status = cmd
        .args(args)
        .env("GP_GAMES_DIR", &games)
        .status()
        .with_context(|| format!("running {}'s auto-player", game.display()))?;
    let code = status.code().unwrap_or(2);
    let mut out = Outcome::ok(json!({"game": game, "exit": code}), String::new());
    out.exit = u8::try_from(code).unwrap_or(2);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The game is found by its romset's identity, and its checks.toml
    /// names its player or says it has none (made-up games).
    #[test]
    fn a_game_names_its_player_or_has_none() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("made-up-a");
        let b = d.path().join("made-up-b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let id = "ab".repeat(32);
        fs::write(
            a.join("roms.toml"),
            format!("[[source]]\nsha256 = \"{id}\"\n"),
        )
        .unwrap();
        fs::write(
            a.join("checks.toml"),
            "[autoplay]\nmanifest = \"autoplay/Cargo.toml\"\n",
        )
        .unwrap();
        fs::write(b.join("checks.toml"), "[mechanic_page]\n").unwrap();
        assert_eq!(game_of(d.path(), &id), Some(a.clone()));
        assert_eq!(game_of(d.path(), &"cd".repeat(32)), None);
        assert_eq!(
            player_of(&a).unwrap(),
            Player::Manifest(a.join("autoplay/Cargo.toml"))
        );
        let e = player_of(&b).unwrap_err().to_string();
        assert!(e.contains("no [autoplay]"), "{e}");
        let e = player_of(&d.path().join("none")).unwrap_err().to_string();
        assert!(e.contains("no checks.toml"), "{e}");
        fs::write(
            b.join("checks.toml"),
            "[autoplay]\nbin = \"p\"\nmanifest = \"m\"\n",
        )
        .unwrap();
        assert!(player_of(&b).is_err());
        assert!(run(&["--help".into()]).unwrap().text.contains("[autoplay]"));
    }
}
