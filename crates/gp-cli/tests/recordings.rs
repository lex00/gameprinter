//! `gp record` and what reads a recording, end to end on gp-nes's own test
//! ROM (built in code, never a game's data), each test in its own GP_HOME.
//! Brought from game-printer's tests/cli.rs, cut to what gameprinter has.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn gp(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gp"))
        .env("GP_HOME", home)
        .env("GP_ROM_DIR", home.join("roms"))
        .env("GP_GAMES_DIR", home.join("games"))
        .env_remove("GP_STORE")
        .env_remove("GP_SOURCES")
        .arg("--json")
        .args(args)
        .output()
        .unwrap()
}

fn json(home: &Path, args: &[&str]) -> Value {
    let out = gp(home, args);
    assert!(
        out.status.success(),
        "gp {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// The test ROM in GP_HOME/roms and an input log of 120 frames.
fn setup(home: &Path) -> (String, String) {
    std::fs::create_dir_all(home.join("roms")).unwrap();
    std::fs::create_dir_all(home.join("games")).unwrap();
    let rom = home.join("roms/test.nes");
    std::fs::write(&rom, gp_nes::testrom::build()).unwrap();
    let log = gp_evidence::input_log::generate(120, &["a@10-30", "right@40-"]).unwrap();
    let input = home.join("walk.input");
    std::fs::write(&input, log.to_text()).unwrap();
    (
        rom.to_str().unwrap().to_owned(),
        input.to_str().unwrap().to_owned(),
    )
}

#[test]
fn record_replay_query_and_trace_with_the_test_rom() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let (rom, input) = setup(home);
    let rec = json(
        home,
        &[
            "record",
            "--rom",
            &rom,
            "--input",
            &input,
            "--checkpoint-interval",
            "30",
            "--name",
            "walk",
        ],
    );
    assert_eq!(rec["frames"], 120);
    let id = rec["id"].as_str().unwrap().to_owned();
    let short = &id[..8];

    let listed = json(home, &["recordings", "list"]);
    assert_eq!(listed["recordings"][0]["id"], id.as_str());
    assert_eq!(listed["recordings"][0]["name"], "walk");
    let shown = json(home, &["recordings", "show", short]);
    assert_eq!(shown["recording"]["frames"], 120);
    assert_eq!(shown["missing"], 0);
    assert!(shown["recording"]["has_trace"].as_bool().unwrap());

    // Replay finds the ROM in $GP_ROM_DIR by hash, or from a checkpoint.
    let full = json(home, &["replay", short]);
    assert_eq!(full["frames_checked"], 120);
    assert!(full["divergence"].is_null());
    let tail = json(home, &["replay", short, "--from-checkpoint", "60"]);
    assert_eq!(tail["frames_checked"], 60);
    let all = json(home, &["replay", "--all"]);
    assert_eq!(all["matched"], 1);

    // The trace and a replay give the same answer.
    let from_trace = json(home, &["query", short, "--frame", "50", "ram[0..16]"]);
    let replayed = json(
        home,
        &["query", short, "--frame", "50", "ram[0..16]", "--replay"],
    );
    assert_eq!(from_trace["read_from"], "trace");
    assert_eq!(replayed["read_from"], "replay");
    assert_eq!(from_trace["entries"], replayed["entries"]);
    let until = json(home, &["run-until", short, "frame >= 10"]);
    assert_eq!(until["met"], true);
    assert_eq!(until["frame"], 10);

    let frame = json(home, &["trace", "frame", short, "60", "--field", "cpu"]);
    assert!(frame["fields"]["cpu.pc"].is_string());
    let fields = json(home, &["trace", "fields", short]);
    assert!(!fields["fields"].as_array().unwrap().is_empty());
    assert!(json(home, &["trace", "verify", short])["frame"].is_null());
    let instr = json(home, &["trace", "instructions", short, "--frames", "3"]);
    assert!(instr["instructions"].as_u64().unwrap() > 0);
    let same = json(home, &["trace", "diff", short, short]);
    assert!(same["frame"].is_null());
    let events = json(home, &["events", short, "--kind", "nmi"]);
    assert_eq!(events["from"], "event stream");

    // A short ID is refused, exit 2; a --rom with another hash too.
    assert_eq!(gp(home, &["replay", "ab"]).status.code(), Some(2));
    let mut other = gp_nes::testrom::build();
    let last = other.len() - 7;
    other[last] ^= 0xff;
    let other_path = home.join("other.nes");
    std::fs::write(&other_path, other).unwrap();
    let out = gp(
        home,
        &["replay", short, "--rom", other_path.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("isn't the ROM this recording was made from")
    );
}

#[test]
fn a_recording_from_another_input_differs_at_its_first_changed_frame() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let (rom, input) = setup(home);
    let a = json(home, &["record", "--rom", &rom, "--input", &input]);
    let log = gp_evidence::input_log::generate(120, &["a@10-30", "right@50-"]).unwrap();
    let later = home.join("later.input");
    std::fs::write(&later, log.to_text()).unwrap();
    let b = json(
        home,
        &["record", "--rom", &rom, "--input", later.to_str().unwrap()],
    );
    let (a, b) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());
    let out = gp(home, &["trace", "diff", &a[..8], &b[..8]]);
    assert_eq!(out.status.code(), Some(1));
    let d: Value = serde_json::from_slice(&out.stdout).unwrap();
    // Input log entry 40 runs frame 41, the first that can differ.
    assert!(d["frame"].as_u64().unwrap() >= 41, "{d}");
}
