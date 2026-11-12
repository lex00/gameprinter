//! `gp element inputs` on Moss Hop: a game made by `gp game new` from the
//! built ROM, one played clip in its corpus, and the routines that write two bytes whose code we wrote
//! (games/mosshop/mosshop.s). `read_pad` writes PAD_NEW from PAD_PREV's
//! byte and the bits it reads from the controller into PAD; `move_enemies`
//! steps ENEMY_X+x every other frame, testing FRAMES, ENEMY_STATE+x and
//! ENEMY_DIR+x. Routine names come from ld65's label file, written into a
//! labels file here, since Moss Hop's committed one names RAM only. Needs
//! cc65 to build Moss Hop, like `just check`. Brought from game-printer,
//! whose test made a capture run; gameprinter's `--run` is a game's folder.

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

/// Moss Hop's RAM labels with its routines from ld65's VICE label file
/// (`al 00C115 .read_pad`): global labels at $8000 and up, each sized to
/// the next.
fn labels_with_routines(built: &Path, out: &Path) {
    let mut text = std::fs::read_to_string(repo().join("games/mosshop/labels.toml")).unwrap();
    let vice = std::fs::read_to_string(built.join("mosshop.labels")).unwrap();
    let mut rows: Vec<(u16, String)> = vice
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let (_, addr, name) = (w.next()?, w.next()?, w.next()?);
            let name = name.trim_start_matches('.');
            let addr = u16::from_str_radix(addr.trim_start_matches("00"), 16).ok()?;
            (addr >= 0x8000 && !name.starts_with('@')).then(|| (addr, name.to_owned()))
        })
        .collect();
    rows.sort();
    rows.dedup_by(|a, b| a.1 == b.1);
    for (i, (addr, name)) in rows.iter().enumerate() {
        let size = rows.get(i + 1).map_or(16, |n| n.0 - addr).max(1);
        text.push_str(&format!(
            "\n[[routine]]\nname = \"{name}\"\nbank = 0\naddr = {addr:#06x}\nprg = {:#06x}\nsize = {size}\n",
            addr - 0xC000
        ));
    }
    std::fs::write(out, text).unwrap();
}

fn inputs(home: &Path, labels: &Path, frame: &str, byte: &str) -> Value {
    let out = gp(home)
        .args(["--json", "element", "inputs", "--run"])
        .arg(home.join("games/hop"))
        .args([
            "--clip",
            "played.hop",
            "--frame",
            frame,
            "--byte",
            byte,
            "--labels",
        ])
        .arg(labels)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn names(reads: &Value, how: &str) -> Vec<String> {
    let mut v: Vec<String> = reads
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["how"] == how)
        .map(|r| r["name"].as_str().unwrap().to_owned())
        .collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn names_the_routine_and_its_inputs_on_moss_hop() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let build = StdCommand::new(repo().join("scripts/mosshop/build.sh"))
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "Moss Hop didn't build; this test needs cc65 like just check:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    std::fs::create_dir_all(home.join("store")).unwrap();
    std::fs::create_dir_all(home.join("roms/nes")).unwrap();
    std::fs::create_dir_all(home.join("games")).unwrap();
    let rom = home.join("roms/nes/mosshop.nes");
    std::fs::copy(dir.path().join("mosshop.nes"), &rom).unwrap();
    let made = gp(&home)
        .args(["game", "new", "hop", "--genre", "platformer"])
        .args(["--namespace", "test", "--no-draws", "--rom"])
        .arg(&rom)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    // One played clip: 120 frames of walking right from power-on, then
    // walk right and hop.
    let game = home.join("games/hop");
    std::fs::create_dir_all(game.join("inputs")).unwrap();
    std::fs::write(
        game.join("inputs/played.hop.start.presses"),
        "gp-presses 0\nframes 120\nright@60-119\n",
    )
    .unwrap();
    let mut corpus = std::fs::read_to_string(game.join("corpus.toml")).unwrap();
    corpus.push_str(
        "\n[[clip]]\nid = \"played.hop\"\nname = \"Walk right and hop\"\nsource = \"played\"\nstart = \"inputs/played.hop.start.presses\"\npresses = [\"right@0-59\", \"a@20-30\"]\nframes = 60\n",
    );
    std::fs::write(game.join("corpus.toml"), corpus).unwrap();
    let labels = dir.path().join("labels.toml");
    labels_with_routines(dir.path(), &labels);

    // read_pad: PAD_NEW from PAD (read twice) and the controller; PAD
    // itself was set on the frame before, from the controller.
    let v = inputs(&home, &labels, "25", "PAD_NEW");
    assert_eq!(v["schema"], "gp element inputs 1");
    let b = &v["bytes"][0];
    assert_eq!(b["writer"]["routine"], "read_pad", "{b}");
    assert_eq!(names(&b["reads"], "value"), ["JOYPAD1", "PAD"], "{b}");
    let joy = b["reads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "JOYPAD1")
        .unwrap();
    assert_eq!(
        (&joy["role"], &joy["times"]),
        (&"controller".into(), &8.into())
    );
    // The controller is an input, not a gap.
    assert_eq!(v["gaps"], serde_json::json!([]), "{v}");
    let pad = &b["earlier"][0];
    assert_eq!(
        (&pad["through"], &pad["frame_written"]),
        (&"PAD".into(), &24.into())
    );
    assert_eq!(pad["writer"]["routine"], "read_pad");
    assert_eq!(
        b["sentence"],
        "read_pad wrote PAD_NEW from PAD and JOYPAD1. Before the frame: PAD was set on frame 24 by read_pad from JOYPAD1 and PAD."
    );

    // move_enemies: the second enemy's place, stepped from itself on even
    // frames, after tests of the frame count and its own flags.
    let v = inputs(&home, &labels, "25", "ENEMY_X+1");
    let b = &v["bytes"][0];
    assert_eq!(b["writer"]["routine"], "move_enemies", "{b}");
    assert_eq!(b["name"], "ENEMY_X+x");
    assert_eq!(names(&b["reads"], "value"), ["ENEMY_X+x"]);
    let tests = names(&b["reads"], "branch");
    for want in ["FRAMES", "ENEMY_STATE+x", "ENEMY_DIR+x"] {
        assert!(tests.iter().any(|t| t == want), "{want} in {tests:?}");
    }
    let frames = b["reads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "FRAMES")
        .unwrap();
    assert_eq!(frames["role"], "frame counter");
    // It moves every other frame: frame 25 didn't write it.
    let wrote = b["frame_written"].as_u64().unwrap();
    assert!(wrote == 24 || wrote == 25, "{b}");

    // The same trace gives the same output.
    let again = inputs(&home, &labels, "25", "ENEMY_X+1");
    let strip = |mut v: Value| {
        v.as_object_mut().unwrap().remove("run_manifest");
        v
    };
    assert_eq!(strip(v), strip(again));
}
