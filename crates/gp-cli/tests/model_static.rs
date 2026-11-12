//! `gp model static` (stage 0 of docs/plan-extraction-process.md) on Moss
//! Hop: two runs give byte-identical output, and the grade against its
//! answer key (games/mosshop/static-key.toml) holds: every element found,
//! no fact wrong, and at least the numbers and transitions it reaches
//! today. Needs cc65 to build Moss Hop, like `just check`. Brought from
//! game-printer.

use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn gp(home: &Path) -> StdCommand {
    let mut cmd = StdCommand::new(env!("CARGO_BIN_EXE_gp"));
    cmd.env("GP_HOME", home)
        .env("GP_STORE", home.join("store"))
        .env("GP_ROM_DIR", home.join("roms"))
        .env("GP_GAMES_DIR", home.join("games"))
        .env_remove("GP_SOURCES")
        .current_dir(repo());
    cmd
}

#[test]
fn moss_hop_harvests_the_same_bytes_and_grades_with_nothing_wrong() {
    let tmp = tempfile::tempdir().unwrap();
    let build = StdCommand::new(repo().join("scripts/mosshop/build.sh"))
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "Moss Hop didn't build; this test needs cc65 like just check:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let rom = tmp.path().join("mosshop.nes");
    let home = tmp.path().join("home");
    let mut outs = Vec::new();
    for i in 0..2 {
        let dir = tmp.path().join(format!("out{i}"));
        let out = gp(&home)
            .args(["--json", "model", "static"])
            .arg(&rom)
            .args([
                "--assets",
                "assets/mosshop.toml",
                "--grade",
                "games/mosshop/static-key.toml",
                "--runs",
                "1",
                "--write",
            ])
            .arg(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        outs.push((std::fs::read(dir.join("static.json")).unwrap(), v));
    }
    assert_eq!(outs[0].0, outs[1].0, "two runs wrote different bytes");
    let v = &outs[0].1;
    let g = v.get("data").unwrap_or(v)["grade"].clone();
    assert_eq!(g["wrong"], 0, "{g:#}");
    assert_eq!(g["shape_wrong"], 0, "{g:#}");
    assert_eq!(g["elements"]["found"], g["elements"]["source"], "{g:#}");
    assert!(g["right"].as_u64().unwrap() >= 36, "{g:#}");
    assert!(
        g["elements"]["routines_placed"].as_u64().unwrap() >= 36,
        "{g:#}"
    );
    assert_eq!(
        g["open_graded"]["named_right"], g["open_graded"]["with_a_routine"],
        "{g:#}"
    );
    for m in g["machines"].as_array().unwrap() {
        assert_eq!(m["found"], m["real"], "{m:#}");
        assert_eq!(m["extra"].as_array().unwrap().len(), 0, "{m:#}");
    }
}

/// The second test program (test-roms/static): UxROM, other RAM, ROR
/// controller reads, a CMP/BNE mode test and an RTS jump table on each
/// slot's kind. The facts its source says, and none of Moss Hop's that it
/// doesn't have.
#[test]
fn another_layout_gives_its_own_facts() {
    let tmp = tempfile::tempdir().unwrap();
    let src = repo().join("test-roms/static");
    let obj = tmp.path().join("static.o");
    let rom = tmp.path().join("static.nes");
    let ca65 = StdCommand::new("ca65")
        .arg(src.join("static.s"))
        .arg("-o")
        .arg(&obj)
        .output()
        .unwrap();
    assert!(
        ca65.status.success(),
        "ca65: {}",
        String::from_utf8_lossy(&ca65.stderr)
    );
    let ld65 = StdCommand::new("ld65")
        .arg("-C")
        .arg(src.join("static.cfg"))
        .arg(&obj)
        .arg("-o")
        .arg(&rom)
        .output()
        .unwrap();
    assert!(
        ld65.status.success(),
        "ld65: {}",
        String::from_utf8_lossy(&ld65.stderr)
    );
    let home = tmp.path().join("home");
    let out = gp(&home)
        .args(["--json", "model", "static"])
        .arg(&rom)
        .arg("--no-run")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let v = v.get("data").cloned().unwrap_or(v);
    let fact = |p: &str| {
        v["facts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == p)
            .map(|f| f["value"].clone())
            .unwrap_or(Value::Null)
    };
    let want: &[(&str, Value)] = &[
        ("hardware.frame_counter", "$40".into()),
        ("input.pad", "$42".into()),
        ("input.pad_new", "$44".into()),
        ("entities.player.bytes.x", "$50".into()),
        ("entities.player.bytes.y", "$51".into()),
        ("entities.player.jump.button", "a".into()),
        ("entities.player.jump.speed", (-3.5).into()),
        ("entities.player.jump.gravity", "0.125".into()),
        ("entities.player.jump.max_fall", 3.into()),
        ("entities.slots_400.count", 4.into()),
        ("entities.slots_400.bytes.x", "$404,x".into()),
        ("entities.slots_400_k0.patrol.period", 4.into()),
        ("entities.slots_400_k0.patrol.step", 1.into()),
        ("entities.slots_400.machine.initial", "s1".into()),
        (
            "entities.slots_400.machine.states.s2.transition[s1].after",
            45.into(),
        ),
        ("rules.state_45.initial", "s0".into()),
        ("rules.state_45.states.s2.transition[s0].after", 75.into()),
    ];
    for (p, w) in want {
        assert_eq!(&fact(p), w, "{p}");
    }
    for f in v["facts"].as_array().unwrap() {
        let p = f["path"].as_str().unwrap();
        assert!(
            !p.starts_with("counters.") && !p.contains("hitbox") && !p.starts_with("cameras."),
            "a fact it has no code for: {p}"
        );
    }
    let table = v["dispatch"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "jump table")
        .expect("the RTS jump table");
    assert_eq!(table["subject"], "$400,x");
    assert_eq!(table["cases"].as_object().unwrap().len(), 2);
    for k in ["slots_400_k0", "slots_400_k1"] {
        assert!(
            v["elements"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["name"] == k),
            "element {k}"
        );
    }
    // UxROM's banks are followed, and every routine is in the fixed bank:
    // no call goes into a bank the reader doesn't know.
    assert!(!v["open"].as_array().unwrap().iter().any(|o| {
        let w = o["what"].as_str().unwrap_or("");
        w == "code in switched banks" || w.starts_with("call into a switched bank")
    }));
}
