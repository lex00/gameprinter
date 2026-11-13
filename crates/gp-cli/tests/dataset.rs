//! `gp dataset export --hf` and `gp dataset catalogue` on the games
//! checkout's Donkey Kong and Tapper packages (docs/research/hf-datasets.md):
//! the configs a reader opens, a card, Croissant metadata and
//! gp-dataset.json, the clips from each corpus plan with their presses
//! files beside them, and the same bytes from the same package twice.
//!
//! game-printer's tests ran these on its Moss Hop and Super Mario Bros.
//! templates; gameprinter has neither, so these run on the packages it
//! has. They skip with a notice without the games checkout ($GP_GAMES_DIR,
//! else ../games beside the repo). No ROM is read: the ROM check is skipped
//! with $GP_ROM_DIR pointed at an empty folder.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn games() -> PathBuf {
    std::env::var_os("GP_GAMES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("../games"))
}

fn gp(home: &Path, args: &[&str]) -> Value {
    let roms = home.join("roms");
    std::fs::create_dir_all(&roms).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_gp"))
        .env("GP_HOME", home)
        .env("GP_STORE", home.join("store"))
        .env("GP_ROM_DIR", &roms)
        .env("GP_BUILD", "test")
        .env_remove("GP_WORKER")
        .env_remove("GP_LOG")
        .current_dir(repo())
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "gp {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn rows(v: &Value, path: &str) -> u64 {
    v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == path)
        .and_then(|f| f["rows"].as_u64())
        .unwrap_or_else(|| panic!("no {path} in {v}"))
}

#[test]
fn a_package_exports_as_configs_and_the_same_twice() {
    let dk = games().join("donkey-kong");
    let tapper = games().join("tapper");
    if !dk.join("model.toml").is_file() || !tapper.join("model.toml").is_file() {
        eprintln!(
            "dataset: skipped, no games checkout at {}",
            games().display()
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    for game in [&dk, &tapper] {
        let out = dir.path().join("a");
        let s = |p: &Path| p.display().to_string();
        let v = gp(
            &home,
            &[
                "dataset",
                "export",
                "--hf",
                &s(&out),
                "--template",
                &s(game),
            ],
        );
        assert!(rows(&v, "data/facts.parquet") > 0, "{v}");
        assert!(rows(&v, "data/mapping.parquet") > 0, "{v}");
        assert!(rows(&v, "data/clips.parquet") > 0, "{v}");
        for f in ["README.md", "croissant.json", "gp-dataset.json"] {
            assert!(out.join(f).is_file(), "{f}");
        }
        // Every clip's presses files are beside the tables.
        let presses = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["path"].as_str().unwrap().ends_with(".presses"))
            .count();
        assert!(presses > 0, "{v}");
        let again = dir.path().join("b");
        let w = gp(
            &home,
            &[
                "dataset",
                "export",
                "--hf",
                &s(&again),
                "--template",
                &s(game),
            ],
        );
        assert_eq!(v["files"], w["files"]);
        std::fs::rename(
            &out,
            dir.path()
                .join(format!("{}-a", v["game"].as_str().unwrap())),
        )
        .unwrap();
        std::fs::rename(
            &again,
            dir.path()
                .join(format!("{}-b", v["game"].as_str().unwrap())),
        )
        .unwrap();
    }
    let cat = dir.path().join("catalogue");
    let v = gp(
        &home,
        &[
            "dataset",
            "catalogue",
            "--out",
            &cat.display().to_string(),
            &dk.display().to_string(),
            &tapper.display().to_string(),
        ],
    );
    assert_eq!(v["games"].as_array().map(Vec::len), Some(2), "{v}");
    assert!(cat.join("data/taxonomy.parquet").is_file());
}
