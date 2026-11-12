//! `gp author` on Moss Hop (templates/mosshop, our own game): the
//! scripted proposer's edit passes every check that runs, the row it logs
//! is the same on two runs, and a difficulty target the edits can't meet
//! makes the loop try again with the failure as feedback
//! (docs/author.md). Brought from game-printer's tests/author.rs;
//! gameprinter's `gp assets lift` takes the asset map with --map.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Builds Moss Hop and lifts its assets into `dir/lifted`.
fn build_and_lift(dir: &Path) {
    let out = std::process::Command::new(repo().join("scripts/mosshop/build.sh"))
        .arg(dir)
        .output()
        .expect("running scripts/mosshop/build.sh");
    assert!(
        out.status.success(),
        "Moss Hop didn't build; this test needs cc65 (ca65 and ld65), like just check:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lifted = std::process::Command::new(env!("CARGO_BIN_EXE_gp"))
        .env("GP_HOME", dir.join("home"))
        .env_remove("GP_STORE")
        .args(["assets", "lift"])
        .arg(dir.join("mosshop.nes"))
        .arg("-o")
        .arg(dir.join("lifted"))
        .args(["--map", "assets/mosshop.toml"])
        .current_dir(repo())
        .output()
        .unwrap();
    assert!(lifted.status.success(), "{lifted:?}");
}

fn author(dir: &Path, log: &str, extra: &[&str]) -> (i32, Value) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_gp"))
        .env("GP_HOME", dir.join("home"))
        .env("GP_BUILD", "test")
        .env_remove("GP_STORE")
        .args([
            "--json",
            "author",
            "--base",
            "templates/mosshop",
            "--dry-run",
        ])
        .arg("--assets")
        .arg(dir.join("lifted"))
        .arg("--log")
        .arg(dir.join(log))
        .args(extra)
        .current_dir(repo())
        .output()
        .unwrap();
    let text = std::fs::read_to_string(dir.join(log)).unwrap_or_default();
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 1, "one row a request: {out:?}");
    (
        out.status.code().unwrap(),
        serde_json::from_str(rows[0]).unwrap(),
    )
}

#[test]
fn a_floatier_jump_is_verified_kept_and_logged_the_same_twice() {
    let tmp = tempfile::tempdir().unwrap();
    build_and_lift(tmp.path());
    let prompt = ["--prompt", "make the jump floatier"];
    let (code, row) = author(tmp.path(), "a.jsonl", &prompt);
    assert_eq!(code, 0, "{row:#}");
    assert_eq!(row["schema"], "gp author log 1");
    assert_eq!(row["kept"], 1);
    let a = &row["attempts"][0];
    assert_eq!(
        a["proposal"]["set"]["entities.player.jump.gravity"],
        "0.1875"
    );
    let status = |name: &str| {
        a["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["check"] == name)
            .unwrap()["status"]
            .clone()
    };
    for c in [
        "schema",
        "mechanics",
        "public",
        "reach",
        "playthrough",
        "difficulty",
        "feel",
    ] {
        assert_eq!(status(c), "pass", "{c}: {a:#}");
    }
    assert_eq!(status("conformance"), "skipped");
    // The lower gravity keeps the player in the air longer than the base.
    assert!(
        a["measures"]["airtime"].as_u64() > row["base_measures"]["airtime"].as_u64(),
        "{row:#}"
    );
    // Every checker is deterministic, so the same request logs the same row.
    let (_, again) = author(tmp.path(), "b.jsonl", &prompt);
    assert_eq!(row, again);

    // A target no edit meets: each attempt fails on difficulty, and the
    // failure is the feedback the next proposal gets.
    let (code, row) = author(
        tmp.path(),
        "c.jsonl",
        &["--prompt", "floatier", "--target-bits", "40"],
    );
    assert_eq!(code, 1);
    assert!(row["kept"].is_null());
    let attempts = row["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 2);
    for a in attempts {
        let fb = a["feedback"].as_str().unwrap();
        assert!(fb.starts_with("difficulty: too easy"), "{fb}");
    }
}
