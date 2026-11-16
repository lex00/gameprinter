//! A game's preset profile against `retro` (ROM tier).
//!
//! game-printer's test stepped a game's model in gp-sim with its hooks
//! linked into gp, and checked two player reports frame by frame: the
//! hammer swings in normal as in retro (report 2026-11-04), and a jump
//! moves the player across only by their speed (report 2026-11-05). In
//! gameprinter the hooks run only in the game's module, so this port runs
//! the model through `gp interp run --module` on a corpus clip's input from
//! power-on (`gp clip log`), and keeps the checks a run's hashes can make:
//! the profile's run replays to the same state hashes, and none of them is
//! retro's (the profile is in the hash). The frame-by-frame checks wait for
//! a way to read a module run's counters on every frame.
//!
//! The tool knows no game by name. The game is $GP_GAME_DIR (a game folder),
//! else the first game of the games checkout ($GP_GAMES_DIR, or `games`
//! beside the repo) whose checks.toml has a `[profiles]` table: the preset
//! `profile`, the corpus clip `clip` and its `min_frames`. Its module is
//! $GP_GAME_MODULE and its ROM is the one its corpus.toml names, found in
//! $GP_ROM_DIR by sha256. `just check-roms` runs it on every game that has
//! one; it skips in words without the ROM, the game or the module.

use std::path::{Path, PathBuf};
use std::process::Command;

mod rom_tier;
use rom_tier::{corpus_rom, game_with, games, repo};

fn gp(home: &Path, roms: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_gp"))
        .env("GP_HOME", home)
        .env("GP_STORE", home.join("store"))
        .env("GP_ROM_DIR", roms)
        .env("GP_GAMES_DIR", games())
        .env_remove("GP_WORKER")
        .env_remove("GP_LOG")
        .env_remove("GP_ASSETS")
        .current_dir(repo())
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "gp {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
#[ignore = "ROM tier: needs a game of the games checkout with checks.toml's [profiles], its ROM in $GP_ROM_DIR and its module"]
fn a_preset_replays_and_is_in_the_hash() {
    let Some(roms) = std::env::var_os("GP_ROM_DIR").map(PathBuf::from) else {
        eprintln!("profiles: skipped, GP_ROM_DIR isn't set");
        return;
    };
    let (game, checks) = match game_with("profiles") {
        Ok(g) => g,
        Err(why) => {
            eprintln!("profiles: skipped, {why}");
            return;
        }
    };
    let name = game.file_name().unwrap().to_string_lossy().into_owned();
    let (Some(profile), Some(clip), Some(min_frames)) = (
        checks.get("profile").and_then(|v| v.as_str()),
        checks.get("clip").and_then(|v| v.as_str()),
        checks.get("min_frames").and_then(|v| v.as_integer()),
    ) else {
        panic!("{name}'s checks.toml [profiles] needs profile, clip and min_frames: {checks:?}");
    };
    if !game.join("profiles.toml").is_file() {
        eprintln!("profiles: skipped, {name} has no profiles.toml");
        return;
    }
    let Some(module) = std::env::var_os("GP_GAME_MODULE").map(PathBuf::from) else {
        eprintln!("profiles: skipped, GP_GAME_MODULE doesn't name {name}'s module");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join("store")).unwrap();
    let scan: serde_json::Value =
        serde_json::from_str(&gp(home, &roms, &["--json", "roms", "scan"])).unwrap();
    let Some(rom) = corpus_rom(&game, &scan) else {
        eprintln!(
            "profiles: skipped, no ROM {name}'s corpus.toml names in {}",
            roms.display()
        );
        return;
    };
    let (game, module) = (game.to_str().unwrap(), module.to_str().unwrap());
    // The clip checks.toml names, from power-on.
    let log = home.join("clip.log");
    gp(
        home,
        &roms,
        &["clip", "log", game, clip, "--out", log.to_str().unwrap()],
    );
    // The model's assets, lifted into the scratch home.
    let assets = home.join("assets");
    gp(
        home,
        &roms,
        &[
            "assets",
            "lift",
            rom.to_str().unwrap(),
            "-o",
            assets.to_str().unwrap(),
        ],
    );
    let run = |profile: &str, n: u32| -> Vec<String> {
        let out = home.join(format!("{profile}-{n}.txt"));
        gp(
            home,
            &roms,
            &[
                "interp",
                "run",
                game,
                "--input",
                log.to_str().unwrap(),
                "--module",
                module,
                "--assets",
                assets.to_str().unwrap(),
                "--profile",
                profile,
                "--hashes",
                out.to_str().unwrap(),
            ],
        );
        std::fs::read_to_string(out)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    };
    let retro = run("retro", 1);
    let normal = run(profile, 1);
    let again = run(profile, 2);
    assert!(
        normal.len() as i64 > min_frames,
        "{} frames, not over {min_frames}",
        normal.len()
    );
    assert_eq!(retro.len(), normal.len());
    assert_eq!(normal, again, "{profile} doesn't replay to the same hashes");
    let same = retro.iter().zip(&normal).filter(|(a, b)| a == b).count();
    assert_eq!(same, 0, "{same} of {profile}'s hashes are retro's");
    eprintln!(
        "profiles: {name}: {} frames of {clip}; {profile} replays to its own hashes and none is retro's",
        normal.len()
    );
}
