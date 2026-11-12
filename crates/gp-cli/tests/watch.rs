//! `gp watch` and `gp explore` on gp-nes's test ROM (built in code, never
//! a game's data): a session's events are numbered 1, 2, 3 with no gaps,
//! begin with `watch.start` and a `snapshot` and end with `watch.end`, and
//! the same ROM and options give the same file. `gp watch events` reads
//! them back after a `seq` and by kind. Brought from game-printer's
//! tests/watch.rs, which ran on Moss Hop.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn gp(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gp"))
        .env("GP_HOME", home)
        .env("GP_ROM_DIR", home.join("roms"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn a_session_is_numbered_whole_and_the_same_each_time() {
    let tmp = tempfile::tempdir().unwrap();
    let rom = tmp.path().join("test.nes");
    std::fs::write(&rom, gp_nes::testrom::build()).unwrap();
    let rom = rom.to_str().unwrap();
    let home = tmp.path().join("home");
    for name in ["a", "b"] {
        let out = gp(
            &home,
            &[
                "watch",
                "start",
                name,
                rom,
                "--start-runs",
                "0",
                "--start-frames",
                "300",
                "--budget",
                "4",
                "--seed",
                "1",
            ],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let read = |n: &str| std::fs::read(home.join("watch").join(n).join("events.ndjson")).unwrap();
    let a = read("a");
    assert_eq!(a, read("b"), "the same ROM and options give the same file");
    let evs: Vec<Value> = String::from_utf8(a)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for (i, e) in evs.iter().enumerate() {
        assert_eq!(e["seq"], i as u64 + 1, "seq has no gaps");
        assert_eq!(e["v"], 1);
    }
    let kinds: Vec<&str> = evs.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds[..2], ["watch.start", "snapshot"]);
    assert_eq!(kinds.last(), Some(&"watch.end"));
    assert!(kinds.contains(&"new-code"));
    // Every event from a run names its recipe, which the session kept.
    for e in evs.iter().filter(|e| e["kind"] == "new-code") {
        let r = e["run"].as_str().unwrap();
        assert!(
            home.join("watch/a/recipes")
                .join(format!("{r}.presses"))
                .is_file()
        );
    }
    let after = gp(
        &home,
        &[
            "--json", "watch", "events", "a", "--after", "2", "--kind", "new-code",
        ],
    );
    let v: Value = serde_json::from_slice(&after.stdout).unwrap();
    let got = v["events"].as_array().unwrap();
    assert_eq!(
        got.len(),
        kinds[2..].iter().filter(|k| **k == "new-code").count()
    );
    let list: Value =
        serde_json::from_slice(&gp(&home, &["--json", "watch", "list"]).stdout).unwrap();
    assert_eq!(list["sessions"].as_array().unwrap().len(), 2);
}

#[test]
fn explore_writes_a_clip_set_from_power_on() {
    let tmp = tempfile::tempdir().unwrap();
    let rom = tmp.path().join("test.nes");
    std::fs::write(&rom, gp_nes::testrom::build()).unwrap();
    let home = tmp.path().join("home");
    let out_dir = tmp.path().join("clips");
    let out = gp(
        &home,
        &[
            "--json",
            "explore",
            rom.to_str().unwrap(),
            "--start-runs",
            "0",
            "--start-frames",
            "300",
            "--new-code",
            "--budget",
            "6",
            "--write",
            out_dir.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["seed"], 1);
    let set = std::fs::read_to_string(out_dir.join("clips.toml")).unwrap();
    assert!(set.contains("generator = \"gp explore\""), "{set}");
    assert!(out_dir.join("explore-start-0.presses").is_file());
}
