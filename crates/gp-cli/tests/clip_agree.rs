//! `gp clip score` and `gp corpus score` agree on a corpus clip (ROM tier).
//! Both take the clip's presses from power-on to its start and its own
//! presses (or presses file), the original's draws over it when the
//! mapping sets `score_draws`, and the original's state at the start; on
//! the same clip they must give the same frames together, the same start
//! and the same first split. On 2026-11-04 a private log script that read
//! a clip's `presses` and not its `presses_file` gave `gp interp compare` a
//! clip in which the player stood still, where the corpus scorer split at the
//! clip's frame 1330. `gp clip log` and `gp clip score` take a corpus
//! clip's ID now, so no one writes that log by hand.
//!
//! Ported from game-printer, where it ran on one game. The tool knows no
//! game by name now: the game is $GP_GAME_DIR (a game folder), else the
//! first game of the games checkout ($GP_GAMES_DIR, or `games` beside the
//! repo) whose checks.toml has a `[clip_agree]` table, whose `clips` are
//! the corpus clips it checks. Its hooks run in its module
//! ($GP_GAME_MODULE, which `just check-roms` builds from this tool's kit),
//! its ROM is the one its corpus.toml names, and no capture run is started
//! first. `just check-roms` runs it on every game that has one; it skips in
//! words without the ROM, the game or the module.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

mod rom_tier;
use rom_tier::{corpus_rom, game_with, games, repo};

fn gp(home: &Path, roms: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gp"));
    cmd.env("GP_HOME", home)
        .env("GP_STORE", home.join("store"))
        .env("GP_ROM_DIR", roms)
        .env("GP_GAMES_DIR", games())
        .env_remove("GP_WORKER")
        .env_remove("GP_LOG")
        .env_remove("GP_ASSETS")
        .env_remove("GP_SOURCES")
        .current_dir(repo());
    cmd
}

fn json(home: &Path, roms: &Path, args: &[&str]) -> Value {
    let out = gp(home, roms).arg("--json").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "gp {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
#[ignore = "ROM tier: needs a game of the games checkout with checks.toml's [clip_agree], its ROM in $GP_ROM_DIR and its module"]
fn clip_score_and_corpus_score_agree_on_a_corpus_clip() {
    let Some(roms) = std::env::var_os("GP_ROM_DIR").map(PathBuf::from) else {
        eprintln!("clip_agree: skipped, GP_ROM_DIR isn't set");
        return;
    };
    let (game, checks) = match game_with("clip_agree") {
        Ok(g) => g,
        Err(why) => {
            eprintln!("clip_agree: skipped, {why}");
            return;
        }
    };
    let name = game.file_name().unwrap().to_string_lossy().into_owned();
    let clips: Vec<&str> = checks
        .get("clips")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|c| c.as_str()).collect())
        .unwrap_or_default();
    assert!(
        !clips.is_empty(),
        "{name}'s checks.toml [clip_agree] names no clips: {checks:?}"
    );
    if !game.join("corpus.toml").is_file() {
        eprintln!("clip_agree: skipped, {name} has no corpus.toml");
        return;
    }
    let Some(module) = std::env::var_os("GP_GAME_MODULE").map(PathBuf::from) else {
        eprintln!("clip_agree: skipped, GP_GAME_MODULE doesn't name {name}'s module");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join("store")).unwrap();
    let scan = json(home, &roms, &["roms", "scan"]);
    if corpus_rom(&game, &scan).is_none() {
        eprintln!(
            "clip_agree: skipped, no ROM {name}'s corpus.toml names in {}",
            roms.display()
        );
        return;
    }
    let (game, module) = (game.to_str().unwrap(), module.to_str().unwrap());
    let mut args = vec![
        "corpus", "score", game, "--run", "agree", "--module", module,
    ];
    for id in &clips {
        args.extend(["--only", id]);
    }
    let corpus = json(home, &roms, &args);
    let mut args = vec!["clip", "score", game, "--module", module];
    args.extend(clips.iter().copied());
    let clip = json(home, &roms, &args);
    let rows = corpus["record"]["predicate"]["clips"].as_array().unwrap();
    for id in &clips {
        let c = rows
            .iter()
            .find(|r| r["id"] == *id)
            .unwrap_or_else(|| panic!("{id}: not in {corpus}"));
        let s = clip["clips"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["clip"] == *id)
            .unwrap_or_else(|| panic!("{id}: not in {clip}"));
        assert_eq!(
            (&c["frames_together"], &c["frames"]),
            (&s["straight"]["together"], &s["straight"]["frames"]),
            "{id}: frames together, gp corpus score against gp clip score"
        );
        assert_eq!(
            c["first"]["original_frame"], s["first"]["original_frame"],
            "{id}: the first split ({c} / {s})"
        );
        assert_eq!(s["start"].as_u64(), c["start"].as_u64(), "{id}: the start");
        eprintln!(
            "clip_agree: {name}: {id}: {} of {} frames together both ways, start {}",
            s["straight"]["together"], s["straight"]["frames"], s["start"]
        );
    }
}
