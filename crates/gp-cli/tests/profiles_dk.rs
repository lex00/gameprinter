//! Donkey Kong's `normal` profile against `retro` (ROM tier).
//!
//! game-printer's test stepped Donkey Kong's model in gp-sim with its
//! hooks linked into gp, on Rivet Rodeo's assets, and checked two player
//! reports frame by frame: the hammer swings in normal as in retro (report
//! 2026-11-04), and a jump moves Mario across only by his speed (report
//! 2026-11-05). In gameprinter the hooks run only in the game's module and
//! Rivet Rodeo is retired, so this port runs the model through `gp interp
//! run --module` on a corpus clip's input from power-on (`gp clip log`),
//! and keeps the checks a run's hashes can make: the profile's run replays
//! to the same state hashes, and none of them is retro's (the profile is in
//! the hash). The hammer's and the jumps' frame-by-frame checks wait for a
//! way to read a module run's counters on every frame.
//!
//! `just check-roms` runs it; it skips with a notice without the ROM, the
//! games checkout or the module ($GP_DK_MODULE).

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn games() -> PathBuf {
    std::env::var_os("GP_GAMES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("../games"))
}

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
#[ignore = "ROM tier: needs Donkey Kong: Original Edition in $GP_ROM_DIR, the games checkout and DK's module"]
fn the_normal_profile_replays_and_is_in_the_hash() {
    let Some(roms) = std::env::var_os("GP_ROM_DIR").map(PathBuf::from) else {
        eprintln!("profiles_dk: skipped, GP_ROM_DIR isn't set");
        return;
    };
    let game = games().join("donkey-kong");
    if !game.join("profiles.toml").is_file() {
        eprintln!(
            "profiles_dk: skipped, no games checkout at {}",
            games().display()
        );
        return;
    }
    let Some(module) = std::env::var_os("GP_DK_MODULE").map(PathBuf::from) else {
        eprintln!("profiles_dk: skipped, GP_DK_MODULE doesn't name Donkey Kong's module");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join("store")).unwrap();
    let scan: serde_json::Value =
        serde_json::from_str(&gp(home, &roms, &["--json", "roms", "scan"])).unwrap();
    let rom = scan["roms"].as_array().and_then(|r| {
        r.iter()
            .find(|r| r["known"]["id"] == "donkey-kong-oe")
            .map(|r| {
                Path::new(scan["roms_dir"].as_str().unwrap()).join(r["path"].as_str().unwrap())
            })
    });
    let Some(rom) = rom else {
        eprintln!(
            "profiles_dk: skipped, no donkey-kong-oe ROM in {}",
            roms.display()
        );
        return;
    };
    let (game, module) = (game.to_str().unwrap(), module.to_str().unwrap());
    // The hammer clip: Mario takes the hammer and swings it while he walks.
    let log = home.join("hammer.log");
    gp(
        home,
        &roms,
        &[
            "clip",
            "log",
            game,
            "dk.hammer.005",
            "--out",
            log.to_str().unwrap(),
        ],
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
    let normal = run("normal", 1);
    let again = run("normal", 2);
    assert!(normal.len() > 9000, "{} frames", normal.len());
    assert_eq!(retro.len(), normal.len());
    assert_eq!(normal, again, "normal doesn't replay to the same hashes");
    let same = retro.iter().zip(&normal).filter(|(a, b)| a == b).count();
    assert_eq!(same, 0, "{same} of normal's hashes are retro's");
    eprintln!(
        "profiles_dk: {} frames; normal replays to its own hashes and none is retro's",
        normal.len()
    );
}
