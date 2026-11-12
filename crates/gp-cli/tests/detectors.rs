//! The detectors (`gp track`, `gp coverage`, `gp gaps`, `gp sprites
//! capture`, `gp sound`) end to end on gp-nes's own test ROM (built in
//! code, never a game's data), each test in its own GP_HOME.

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

/// The test ROM in GP_HOME/roms, recorded through 240 frames of presses;
/// the recording's ID.
fn recorded(home: &Path) -> String {
    std::fs::create_dir_all(home.join("roms")).unwrap();
    std::fs::create_dir_all(home.join("games")).unwrap();
    let rom = home.join("roms/test.nes");
    std::fs::write(&rom, gp_nes::testrom::build()).unwrap();
    let log = gp_evidence::input_log::generate(240, &["a@10-30", "right@40-"]).unwrap();
    let input = home.join("walk.input");
    std::fs::write(&input, log.to_text()).unwrap();
    let rec = json(
        home,
        &[
            "record",
            "--rom",
            rom.to_str().unwrap(),
            "--input",
            input.to_str().unwrap(),
            "--name",
            "walk",
        ],
    );
    rec["id"].as_str().unwrap().to_owned()
}

#[test]
fn track_reads_a_recordings_trace() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let id = recorded(home);
    let r = json(home, &["track", &id[..8]]);
    assert_eq!(r["id"], id.as_str());
    assert_eq!(r["frames"], serde_json::json!([1, 240]));
    assert!(r["report"]["tracks"].is_array());
    let r = json(
        home,
        &["track", &id[..8], "--frames", "10..50", "--propose-slots"],
    );
    assert_eq!(r["frames"], serde_json::json!([10, 49]));
    assert!(r["slots"].is_string());
    // Frame 0 is power-on, with no input behind it.
    let out = gp(home, &["track", &id[..8], "--frames", "0"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("frame 0 is the power-on state"));
}

#[test]
fn coverage_measures_then_reads_back_what_it_stored() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let id = recorded(home);
    let first = json(home, &["coverage", &id[..8], "--no-save"]);
    assert_eq!(first["recordings"][0]["result"], "measured");
    let group = &first["groups"][0];
    assert_eq!(group["summary"]["frames"], 240);
    assert!(group["summary"]["executed_bytes"].as_u64().unwrap() > 0);
    // --no-save left nothing, so this measures again and keeps it.
    let saved = json(home, &["coverage", &id[..8]]);
    assert_eq!(saved["recordings"][0]["result"], "measured");
    let again = json(home, &["coverage", "--store"]);
    assert_eq!(again["recordings"][0]["result"], "stored");
    assert_eq!(again["groups"][0]["summary"], group["summary"]);
    // With no definition there are no thresholds, so a check can't pass.
    let out = gp(home, &["coverage", "check", &id[..8]]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn coverage_refuses_an_arcade_romset_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let id = recorded(home);
    let zip = home.join("set.zip");
    std::fs::write(&zip, b"PK\x03\x04 not a real romset").unwrap();
    let out = gp(
        home,
        &["coverage", &id[..8], "--rom", zip.to_str().unwrap()],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no arcade counterpart"), "{err}");
}

/// A made-up package whose ROM entry is an arcade board's.
fn arcade_package(home: &Path) -> std::path::PathBuf {
    let dir = home.join("games/made-up-arcade");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("model.toml"),
        "# not read: the platform is refused first\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("package.toml"),
        "[package]\nname = \"test/made-up-arcade\"\n\n[[rom.entry]]\nid = \"made-up\"\nrole = \"scored\"\nplatform = \"arcade\"\n",
    )
    .unwrap();
    dir
}

#[test]
fn gaps_refuses_an_arcade_game_with_an_nes_recording_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let id = recorded(home);
    let pkg = arcade_package(home);
    let out = gp(
        home,
        &[
            "gaps",
            "all",
            pkg.to_str().unwrap(),
            "--recording",
            &id[..8],
        ],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("is an arcade game, and"), "{err}");
}

/// test-roms/apu/notes.s (a made-up phrase, no game's data) assembled with
/// cc65, as `just check` needs for Moss Hop, and recorded for 400 frames.
fn notes_recorded(home: &Path) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms/apu");
    let o = home.join("notes.o");
    let rom = home.join("roms/notes.nes");
    std::fs::create_dir_all(home.join("roms")).unwrap();
    std::fs::create_dir_all(home.join("games")).unwrap();
    let run = |tool: &str, args: &[&Path]| {
        let st = Command::new(tool)
            .args(args)
            .status()
            .expect("cc65 is installed");
        assert!(st.success(), "{tool} failed");
    };
    run("ca65", &[Path::new("-o"), &o, &src.join("notes.s")]);
    run(
        "ld65",
        &[
            Path::new("-C"),
            &src.join("notes.cfg"),
            Path::new("-o"),
            &rom,
            &o,
        ],
    );
    let log = gp_evidence::input_log::generate(400, &[]).unwrap();
    let input = home.join("notes.input");
    std::fs::write(&input, log.to_text()).unwrap();
    let r = json(
        home,
        &[
            "record",
            "--rom",
            rom.to_str().unwrap(),
            "--input",
            input.to_str().unwrap(),
            "--name",
            "notes",
        ],
    );
    r["id"].as_str().unwrap().to_owned()
}

/// From game-printer's tests/sound.rs, cut to the commands gameprinter
/// has: capture, fidelity and compare (export, import and render aren't
/// here).
#[test]
fn the_notes_test_rom_captures_and_holds_to_its_log() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let id = notes_recorded(home);
    let c = json(
        home,
        &[
            "sound",
            "capture",
            "--recording",
            &id[..12],
            "--name",
            "notes",
        ],
    );
    assert_eq!(c["songs"], 1, "{c}");
    assert_eq!(c["sfx"], 0);
    assert_eq!(c["fidelity"]["music_f1"], 1.0, "{c}");
    let out = std::path::PathBuf::from(c["dir"].as_str().unwrap());
    assert!(out.starts_with(home.join("work/sound")));
    let set =
        gp_sound::song::SongSet::parse(&std::fs::read_to_string(out.join("songs.json")).unwrap())
            .unwrap();
    let s = &set.songs[0];
    assert_eq!(s.loop_frame, Some(0));
    assert_eq!(s.frames, 96, "eight notes of 12 frames");
    let ag = &c["fidelity"]["songs"]["notes"];
    assert_eq!(ag["pitch_frames"], 1.0);
    assert_eq!(ag["volume_frames"], 1.0);

    // fidelity again from the files gives what the capture said.
    let f = json(home, &["sound", "fidelity", out.to_str().unwrap()]);
    assert_eq!(f, c["fidelity"]);

    // A log held to itself agrees on every frame; shifted by a note's
    // length (the phrase isn't periodic in 12) it doesn't.
    let log = out.join("apu.log");
    let same = json(
        home,
        &[
            "sound",
            "compare",
            log.to_str().unwrap(),
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(same["channels"], same["frames"]);
    let off = gp(
        home,
        &[
            "sound",
            "compare",
            log.to_str().unwrap(),
            log.to_str().unwrap(),
            "--offset",
            "12",
        ],
    );
    assert_eq!(off.status.code(), Some(1));
}

#[test]
fn sound_capture_refuses_an_arcade_template_for_an_nes_recording_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let id = recorded(home);
    let pkg = arcade_package(home);
    let out = gp(
        home,
        &[
            "sound",
            "capture",
            "--recording",
            &id[..8],
            "--name",
            "x",
            "--template",
            pkg.to_str().unwrap(),
        ],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("arcade"), "{err}");
}

/// Moss Hop built from source into `home` (scripts/mosshop/build.sh, cc65
/// as `just check` needs) and its assets lifted under $GP_HOME/work/assets;
/// the ROM's path.
fn mosshop_built(home: &Path) -> std::path::PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = home.join("mosshop");
    let st = Command::new(repo.join("scripts/mosshop/build.sh"))
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success(), "mosshop build failed");
    std::fs::create_dir_all(home.join("roms")).unwrap();
    std::fs::create_dir_all(home.join("games")).unwrap();
    let rom = home.join("roms/mosshop.nes");
    std::fs::copy(out.join("mosshop.nes"), &rom).unwrap();
    let assets = home.join("work/assets/mosshop");
    std::fs::create_dir_all(&assets).unwrap();
    json(
        home,
        &[
            "assets",
            "lift",
            rom.to_str().unwrap(),
            "--map",
            repo.join("assets/mosshop.toml").to_str().unwrap(),
            "-o",
            assets.to_str().unwrap(),
            "--force",
        ],
    );
    rom
}

/// Moss Hop's committed baseline `name` (baselines/mosshop/NAME.toml)
/// recorded from its frames and holds; the recording's ID.
fn mosshop_baseline(home: &Path, rom: &Path, name: &str) -> String {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text =
        std::fs::read_to_string(repo.join(format!("baselines/mosshop/{name}.toml"))).unwrap();
    let b: toml::Table = text.parse().unwrap();
    let frames = b["frames"].as_integer().unwrap() as usize;
    let holds: Vec<&str> = b["holds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap())
        .collect();
    let log = gp_evidence::input_log::generate(frames, &holds).unwrap();
    let input = home.join(format!("{name}.input"));
    std::fs::write(&input, log.to_text()).unwrap();
    let r = json(
        home,
        &[
            "record",
            "--rom",
            rom.to_str().unwrap(),
            "--input",
            input.to_str().unwrap(),
            "--name",
            name,
        ],
    );
    r["id"].as_str().unwrap().to_owned()
}

fn mosshop_recorded(home: &Path) -> String {
    let rom = mosshop_built(home);
    mosshop_baseline(home, &rom, "finish-level")
}

/// game-printer's `just track-mosshop` and `just coverage-mosshop`, which
/// its check suite held to floors in budgets.toml through gp-metrics:
/// the same floors, held here. gp track finds Moss Hop's player and slug
/// slots in two baselines and scores them against its mapping; every
/// baseline together reaches coverage/mosshop.toml's thresholds.
#[test]
fn moss_hops_baselines_track_and_cover_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rom = mosshop_built(home);
    let mut names: Vec<String> = std::fs::read_dir(repo.join("baselines/mosshop"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mapping = repo.join("templates/mosshop/mapping.toml");
    for name in &names {
        let id = mosshop_baseline(home, &rom, name);
        if name == "finish-level" || name == "stomp-then-pit" {
            let t = json(
                home,
                &["track", &id[..8], "--against", mapping.to_str().unwrap()],
            );
            let p = t["score"]["precision"].as_f64().unwrap();
            let r = t["score"]["recall"].as_f64().unwrap();
            assert!(p >= 0.95 && r >= 0.75, "{name}: precision {p}, recall {r}");
        }
    }
    let def = repo.join("coverage/mosshop.toml");
    let c = json(
        home,
        &[
            "coverage",
            "check",
            "--store",
            "--definition",
            def.to_str().unwrap(),
            "--rom",
            rom.to_str().unwrap(),
        ],
    );
    let g = &c["groups"][0];
    assert_eq!(g["check"]["passed"], true, "{}", g["check"]);
    let s = &g["summary"];
    for (what, floor) in [
        ("executed_bytes", 1150),
        ("ram_written_bytes", 2000),
        ("screens", 2),
        ("levels", 1),
        ("objects", 1),
        ("input_combinations", 5),
    ] {
        assert!(s[what].as_u64().unwrap() >= floor, "{what}: {s}");
    }
}

/// game-printer's `just sprites-mosshop`, its capture half: Moss Hop's
/// sprite tables and a run give every entity's frames, the set is written
/// to the lifted assets and the model proposed under $GP_HOME/work.
#[test]
fn sprites_capture_on_moss_hop() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let id = mosshop_recorded(home);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tpl = home.join("templates/mosshop");
    std::fs::create_dir_all(&tpl).unwrap();
    for f in ["model.toml", "mapping.toml", "sprites.toml"] {
        std::fs::copy(repo.join("templates/mosshop").join(f), tpl.join(f)).unwrap();
    }
    let r = json(
        home,
        &[
            "sprites",
            "capture",
            tpl.to_str().unwrap(),
            "--recording",
            &id[..8],
        ],
    );
    assert_eq!(r["template"], "mosshop");
    assert_eq!(r["applied"], false);
    assert!(r["frames"].as_u64().unwrap() > 0, "{r}");
    // The run stomps a slug and never dies: every state but dying.
    assert_eq!(
        r["entities"]["slug"]["covered"],
        serde_json::json!(["flat", "walking"])
    );
    assert_eq!(
        r["entities"]["player"]["covered"],
        serde_json::json!(["normal"])
    );
    assert!(home.join("work/assets/mosshop/sprites.toml").is_file());
    assert!(home.join("work/sprites/mosshop/model.toml").is_file());
}

#[test]
fn sprites_capture_refuses_an_arcade_game_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let pkg = arcade_package(home);
    let out = gp(home, &["sprites", "capture", pkg.to_str().unwrap()]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no arcade counterpart"), "{err}");
}
