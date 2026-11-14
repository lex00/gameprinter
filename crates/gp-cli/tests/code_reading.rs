//! The code-reading tools (docs/code-reading.md) on made-up ROMs. Every
//! program and table here is made up in the test. Brought from
//! game-printer; its `roms diff` test comes with wave 1's `roms diff` and
//! its `template cites` test with `template cites` (wave 9).

use std::fs;
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

/// Runs gp with `--json` and returns its data, failing on an error.
fn json(home: &Path, args: &[&str]) -> Value {
    let out = gp(home).arg("--json").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "gp {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v.get("data").cloned().unwrap_or(v)
}

fn text(home: &Path, args: &[&str]) -> String {
    let out = gp(home).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "gp {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// An iNES file from PRG banks of 16 KB, a mapper and 8 KB of CHR.
fn ines(prg: &[u8], mapper: u8) -> Vec<u8> {
    let mut rom = b"NES\x1A".to_vec();
    rom.push((prg.len() / 0x4000) as u8);
    rom.push(1);
    rom.push(mapper << 4);
    rom.push(mapper & 0xF0);
    rom.resize(16, 0);
    rom.extend_from_slice(prg);
    rom.extend(vec![0u8; 0x2000]);
    rom
}

/// A made-up NROM-128 program that reads the controller every loop: when
/// A is held it counts $10 up (the BEQ falls through), else it skips.
///
/// C000: lda #1; sta $4016; lda #0; sta $4016; lda $4016; and #1;
/// beq $C015; inc $10; C015: jmp $C000. NMI and IRQ: rti at $C020.
fn pad_rom() -> Vec<u8> {
    let mut prg = vec![0xEAu8; 0x4000];
    let code = [
        0xA9, 0x01, 0x8D, 0x16, 0x40, 0xA9, 0x00, 0x8D, 0x16, 0x40, 0xAD, 0x16, 0x40, 0x29, 0x01,
        0xF0, 0x02, 0xE6, 0x10, 0x4C, 0x00, 0xC0,
    ];
    prg[..code.len()].copy_from_slice(&code);
    prg[0x20] = 0x40;
    prg[0x3FFA..].copy_from_slice(&[0x20, 0xC0, 0x00, 0xC0, 0x20, 0xC0]);
    ines(&prg, 0)
}

/// A clip set of two clips from power-on, 20 frames each: `held` presses
/// A from frame 6, `idle` presses nothing.
fn clips(dir: &Path) -> PathBuf {
    fs::write(dir.join("start.presses"), "gp-presses 0\nframes 0\n").unwrap();
    let set = r#"[clipset]
format = "0.5.0"
game = "made-up"

[[clips]]
id = "held"
name = "A held"
presses = ["a@5-"]
frames = 20

[clips.start]
presses_from_power_on = "start.presses"
frame = 0

[[clips]]
id = "idle"
name = "nothing pressed"
presses = []
frames = 20

[clips.start]
presses_from_power_on = "start.presses"
frame = 0
"#;
    let p = dir.join("clips.toml");
    fs::write(&p, set).unwrap();
    p
}

#[test]
fn a_listing_counts_each_instruction_and_each_branch_way() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let rom = tmp.path().join("pad.nes");
    fs::write(&rom, pad_rom()).unwrap();
    let set = clips(tmp.path());
    let (rom, set) = (rom.to_str().unwrap(), set.to_str().unwrap());

    // The code alone: every routine, no counts.
    let v = json(&home, &["code", "listing", rom]);
    assert!(v["routines_listed"].as_u64().unwrap() >= 1, "{v:#}");
    assert!(v["clips"].is_null());

    // The held clip: the BEQ at C00F falls on the frames A is held and is
    // taken on the others, and the INC runs as often as it falls.
    let v = json(
        &home,
        &["code", "listing", rom, "--clips", set, "--only", "held"],
    );
    let lines: Vec<&Value> = v["listing"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r["lines"].as_array().unwrap())
        .collect();
    let at = |pc: &str| lines.iter().find(|l| l["pc"] == pc).copied().unwrap();
    let beq = at("C00F");
    let (taken, fell) = (
        beq["taken"].as_u64().unwrap(),
        beq["fell"].as_u64().unwrap(),
    );
    assert!(taken > 0 && fell > 0, "{beq}");
    assert_eq!(at("C011")["count"], beq["fell"]);
    assert_eq!(beq["count"].as_u64().unwrap(), taken + fell);

    // A window before A is pressed takes only one way, and the listing
    // says so in words.
    let t = text(
        &home,
        &[
            "code", "listing", rom, "--clips", set, "--only", "held", "--frames", "1..=4",
        ],
    );
    assert!(t.contains("(taken only)"), "{t}");
    assert!(t.contains("C011  inc $10") && t.contains(" -"), "{t}");

    // The same ROM and clips give the same listing, from the store or not.
    let again = text(
        &home,
        &[
            "code",
            "listing",
            rom,
            "--clips",
            set,
            "--only",
            "held",
            "--frames",
            "1..=4",
            "--no-store",
        ],
    );
    let body = |s: &str| s.lines().skip(1).collect::<Vec<_>>().join("\n");
    assert_eq!(body(&t), body(&again));

    // --writes names every instruction that wrote a RAM byte: the INC at
    // C011 writes $10 on each frame A is held, through any mirror.
    let w = json(
        &home,
        &[
            "code", "listing", rom, "--clips", set, "--only", "held", "--writes", "0810",
        ],
    );
    let row = &w["writes"][0];
    assert_eq!(row["addr"], "0010");
    assert_eq!(row["writers"][0]["pc"], "C011");
    assert_eq!(row["writers"][0]["insn"], "inc $10");
    assert_eq!(row["writes"], at("C011")["count"]);
    let t = text(
        &home,
        &[
            "code", "listing", rom, "--clips", set, "--only", "idle", "--writes", "$10",
        ],
    );
    assert!(t.contains("; $0010: no run wrote it"), "{t}");

    // A listing is never written inside a git work tree.
    let out = gp(&home)
        .args(["code", "listing", rom, "--write"])
        .arg(repo().join("listing.txt"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("git work tree"));
}

#[test]
fn reach_compares_two_windows_or_clips_by_routine() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let rom = tmp.path().join("pad.nes");
    fs::write(&rom, pad_rom()).unwrap();
    let set = clips(tmp.path());
    let (rom, set) = (rom.to_str().unwrap(), set.to_str().unwrap());

    let v = json(
        &home,
        &[
            "reach", rom, "--clip", set, "--only", "held", "--window", "8..",
        ],
    );
    assert_eq!(v["a"]["routines_run"], 1, "{v:#}");
    assert_eq!(v["routines"][0]["both"], 0, "{v:#}");
    assert_eq!(v["routines"][0]["one_way"][0], "C00F fell", "{v:#}");

    // Held against idle: held ran the INC and the BEQ's fall; idle ran
    // nothing held didn't.
    let v = json(
        &home,
        &[
            "reach",
            rom,
            "--clip",
            set,
            "--only",
            "held",
            "--diff-only",
            "idle",
        ],
    );
    assert_eq!(v["a_not_b"]["new_instructions"], 1, "{v:#}");
    assert_eq!(v["a_not_b"]["routines_with_new_paths"][0]["at"], "C011");
    assert_eq!(v["a_not_b"]["branch_ways"][0], "C00F fell (sub_C000)");
    assert_eq!(v["b_not_a"]["new_instructions"], 0, "{v:#}");
    assert_eq!(v["b_not_a"]["branch_ways"].as_array().unwrap().len(), 0);

    // Two windows of one clip: before and after A is pressed.
    let v = json(
        &home,
        &[
            "reach",
            rom,
            "--clip",
            set,
            "--only",
            "held",
            "--window",
            "1..=4",
            "--diff-window",
            "8..",
        ],
    );
    assert_eq!(v["b_not_a"]["new_instructions"], 1, "{v:#}");
    assert_eq!(v["a_not_b"]["new_instructions"], 0, "{v:#}");
    assert_eq!(
        v["a_not_b"]["branch_ways"][0], "C00F taken (sub_C000)",
        "{v:#}"
    );

    // Against a reach output of the idle clip: the same answer without
    // counts.
    let dir = tmp.path().join("reach-idle");
    json(
        &home,
        &[
            "reach",
            rom,
            "--clip",
            set,
            "--only",
            "idle",
            "--no-starts",
            "--write",
            dir.to_str().unwrap(),
        ],
    );
    let v = json(
        &home,
        &[
            "reach",
            rom,
            "--clip",
            set,
            "--only",
            "held",
            "--diff-reach",
            dir.join("reach.json").to_str().unwrap(),
        ],
    );
    assert_eq!(v["b"]["counted"], false);
    assert_eq!(v["a_not_b"]["new_instructions"], 1, "{v:#}");
    assert_eq!(v["a_not_b"]["branch_ways"][0], "C00F fell (sub_C000)");
}

/// A made-up NROM-128 program reading tables by the byte $53, as
/// `gp_cli::code`'s unit test lays it out: pointers at $C100 by `2*$53 -
/// 2`, bytes at $C200 by `$53`, 4-byte rows at $C300 by `4*$53` through a
/// call. It compares $53 with 4 and stores 1 in it.
fn stage_rom() -> Vec<u8> {
    let mut prg = vec![0xEAu8; 0x4000];
    let code: &[u8] = &[
        0xA9, 0x01, 0x85, 0x53, 0xA5, 0x53, 0x38, 0xE9, 0x01, 0x0A, 0xAA, 0xBD, 0x00, 0xC1, 0x85,
        0x00, 0xBD, 0x01, 0xC1, 0x85, 0x01, 0xA6, 0x53, 0xBD, 0x00, 0xC2, 0xA5, 0x53, 0xC9, 0x04,
        0xF0, 0x03, 0x20, 0x30, 0xC0, 0x4C, 0x04, 0xC0,
    ];
    prg[..code.len()].copy_from_slice(code);
    prg[0x30..0x39].copy_from_slice(&[0xA5, 0x53, 0x0A, 0x0A, 0xA8, 0xB9, 0x00, 0xC3, 0x60]);
    prg[0x40] = 0x40;
    for (i, t) in [0xC400u16, 0xC410, 0xC430, 0xC460].iter().enumerate() {
        prg[0x100 + 2 * i..0x102 + 2 * i].copy_from_slice(&t.to_le_bytes());
    }
    prg[0x3FFA..].copy_from_slice(&[0x40, 0xC0, 0x00, 0xC0, 0x40, 0xC0]);
    ines(&prg, 0)
}

#[test]
fn code_tables_finds_the_tables_a_byte_indexes() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let rom = tmp.path().join("stage.nes");
    fs::write(&rom, stage_rom()).unwrap();
    let rom = rom.to_str().unwrap();
    let v = json(&home, &["code", "tables", rom, "--index", "$53"]);
    assert_eq!(v["values"], serde_json::json!([1, 2, 3, 4]), "{v:#}");
    let t = v["tables"].as_array().unwrap();
    let firsts: Vec<&str> = t.iter().map(|r| r["first"].as_str().unwrap()).collect();
    assert_eq!(firsts, ["C100", "C201", "C304"], "{v:#}");
    assert_eq!(t[0]["pointers"], true);
    let pts: Vec<&str> = t[0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["points_to"].as_str().unwrap())
        .collect();
    assert_eq!(pts, ["C400", "C410", "C430", "C460"]);
    assert_eq!(t[0]["rows"][0]["len"], 16);
    assert_eq!(t[0]["rows"][1]["len"], 32);
    assert_eq!(t[1]["row_size"], 1);
    assert_eq!(t[2]["row_size"], 4);
    let assets = v["assets"].as_str().unwrap();
    assert!(assets.contains("format = \"pointers\"") && assets.contains("offset = 0x0100"));
    assert!(assets.contains("format = \"byte-rows\"") && assets.contains("width = 4"));
    // Values given replace the code's.
    let v = json(
        &home,
        &["code", "tables", rom, "--index", "$53", "--values", "2..=3"],
    );
    assert_eq!(v["values"], serde_json::json!([2, 3]));
}
