//! Junk around a dump end to end: `gp roms scan`, `gp roms clean` and
//! `gp roms add --clean` on copies of our own test program (never a game),
//! built here at run time, with the program registered in a scratch games
//! checkout's `gp-testrom/roms.toml` (game-printer's registered it in a
//! scratch sources/nes.toml; the games checkout holds the sources now).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

struct Env {
    home: tempfile::TempDir,
}

/// The test program registered as a source, with board facts that match its
/// header (mapper 0, 16KB PRG, 8KB CHR RAM, horizontal mirroring).
fn registry(battery: bool) -> String {
    let sha256 = gp_evidence::source::identify_nes(&rom()).unwrap().sha256;
    format!(
        r#"[[source]]
id = "gp-testrom"
title = "gp-testrom"
platform = "nes"
region = "ntsc"
sha256 = "{sha256}"
mapper = 0
submapper = 0
mirroring = "horizontal"
battery = {battery}
prg_rom_bytes = 16384
chr_rom_bytes = 0
prg_ram_bytes = 0
chr_ram_bytes = 8192
status = "verified"
"#
    )
}

fn rom() -> Vec<u8> {
    gp_nes::testrom::build()
}

fn junk(name: &str) -> Vec<u8> {
    let rom = rom();
    let (header, data) = rom.split_at(16);
    let mut h = header.to_vec();
    let mut out = Vec::new();
    match name {
        "diskdude" => h[7..16].copy_from_slice(b"DiskDude!"),
        "ni0330" => h[10..16].copy_from_slice(b"Ni0330"),
        "nes20" => {
            h[7] |= 0x08;
            h[11] = 0x07;
        }
        "trainer" => {
            h[6] |= 0x04;
            out.extend(std::iter::repeat_n(0xEE, 512));
        }
        "trailing" | "patched" => {}
        other => panic!("{other}"),
    }
    let mut v = h;
    v.extend(out);
    v.extend_from_slice(data);
    match name {
        "trailing" => v.extend(std::iter::repeat_n(0x55, 435_069)),
        "patched" => v[16 + 0x100] ^= 0xFF,
        _ => {}
    }
    v
}

impl Env {
    fn new(battery: bool) -> Self {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("games/gp-testrom")).unwrap();
        std::fs::write(
            home.path().join("games/gp-testrom/roms.toml"),
            registry(battery),
        )
        .unwrap();
        Self { home }
    }

    fn path(&self, p: &str) -> PathBuf {
        self.home.path().join(p)
    }

    fn gp(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_gp"));
        cmd.env("GP_HOME", self.home.path())
            .env("GP_GAMES_DIR", self.path("games"))
            .env_remove("GP_SOURCES")
            .env("GP_ROM_DIR", self.path("roms"))
            .env_remove("GP_STORE")
            .env_remove("GP_LOG")
            .env_remove("GP_RUN_GROUP");
        cmd
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.path(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// `gp --json roms clean`: exit code and JSON.
    fn clean(&self, input: &Path, out: &Path) -> (i32, Value) {
        let o = self
            .gp()
            .args(["--json", "roms", "clean"])
            .arg(input)
            .arg("-o")
            .arg(out)
            .output()
            .unwrap();
        let json = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
        (o.status.code().unwrap(), json)
    }
}

#[test]
fn five_kinds_of_junk_clean_to_the_exact_original() {
    let env = Env::new(false);
    for kind in ["diskdude", "ni0330", "nes20", "trainer", "trailing"] {
        let dirty = junk(kind);
        let input = env.write(&format!("in/{kind}.nes"), &dirty);
        let out = env.path(&format!("out/{kind}.nes"));
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        let (code, json) = env.clean(&input, &out);
        assert_eq!(code, 0, "{kind}: {json}");
        assert_eq!(std::fs::read(&out).unwrap(), rom(), "{kind}");
        assert_eq!(
            std::fs::read(&input).unwrap(),
            dirty,
            "{kind}: input changed"
        );
        assert_eq!(json["source"], "gp-testrom");
        assert_eq!(json["header_rule"], "registry");
        let sidecar =
            std::fs::read_to_string(env.path(&format!("out/{kind}.nes.clean.toml"))).unwrap();
        let sidecar: toml::Table = toml::from_str(&sidecar).unwrap();
        assert_eq!(sidecar["source"].as_str(), Some("gp-testrom"));
        let removed = sidecar["removed"].as_array().unwrap();
        let what: Vec<_> = removed
            .iter()
            .map(|r| r["what"].as_str().unwrap())
            .collect();
        let want: &[&str] = match kind {
            "trainer" => &["header", "trainer"],
            "trailing" => &["trailing"],
            _ => &["header"],
        };
        assert_eq!(what, want, "{kind}");
        if kind == "trailing" {
            assert_eq!(removed[0]["len"].as_integer(), Some(435_069));
            let blake3 = blake3_hex(&dirty[dirty.len() - 435_069..]);
            assert_eq!(removed[0]["blake3"].as_str(), Some(&blake3[..]));
        }
    }
}

fn blake3_hex(bytes: &[u8]) -> String {
    gp_evidence::Hash::of(bytes).to_hex()
}

#[test]
fn a_patched_prg_byte_matches_no_known_release() {
    let env = Env::new(false);
    let input = env.write("Test Game (U) [h1].nes", &junk("patched"));
    let out = env.path("patched.nes");
    let (code, json) = env.clean(&input, &out);
    assert_eq!(code, 1, "{json}");
    assert!(json["source"].is_null());
    assert_eq!(json["header_rule"], "file");
    let release = &json["report"]["release"];
    assert_eq!(release["known"], false);
    assert_eq!(release["same_sizes_as"][0], "gp-testrom");
    assert_eq!(json["report"]["name_tags"][0]["tag"], "[h1]");

    let text = env
        .gp()
        .args(["roms", "clean"])
        .arg(&input)
        .arg("-o")
        .arg(&out)
        .arg("--force")
        .output()
        .unwrap();
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.contains("matches no known release; same mapper and PRG and CHR sizes as gp-testrom, so it may be a modified copy"),
        "{text}"
    );
    assert!(text.contains("[h1] a hack"), "{text}");
}

#[test]
fn clean_refuses_a_repo_an_existing_output_and_its_input() {
    let env = Env::new(false);
    let input = env.write("in.nes", &junk("diskdude"));
    std::fs::create_dir_all(env.path("repo/.git")).unwrap();
    let (code, _) = env.clean(&input, &env.path("repo/out.nes"));
    assert_eq!(code, 2);
    assert!(!env.path("repo/out.nes").exists());

    env.write("exists.nes", b"x");
    assert_eq!(env.clean(&input, &env.path("exists.nes")).0, 2);
    assert_eq!(std::fs::read(env.path("exists.nes")).unwrap(), b"x");

    let o = env
        .gp()
        .args(["roms", "clean"])
        .arg(&input)
        .arg("-o")
        .arg(&input)
        .arg("--force")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(std::fs::read(&input).unwrap(), junk("diskdude"));

    let short = env.write("short.nes", &rom()[..4000]);
    let o = env
        .gp()
        .args(["roms", "clean"])
        .arg(&short)
        .arg("-o")
        .arg(env.path("short-out.nes"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("underdump"));
}

#[test]
fn scan_reports_each_finding() {
    let env = Env::new(false);
    env.write("roms/nes/dude.nes", &junk("diskdude"));
    env.write("roms/nes/trainer.nes", &junk("trainer"));
    env.write("roms/nes/over [o1].nes", &junk("trailing"));
    env.write("roms/nes/short.nes", &rom()[..4000]);
    let o = env.gp().args(["roms", "scan"]).output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    let text = String::from_utf8(o.stdout).unwrap();
    for want in [
        "header: bytes 7-15 hold \"DiskDude!\" (the DiskDude! stamp), ignored; mapper 0 from byte 6",
        "trainer: 512 bytes after the header, stripped from the hash",
        "overdump: 435,069 trailing bytes, not hashed",
        "file name tags (hints only): [o1] an overdump",
        "underdump: the header states 16,384 bytes of PRG and CHR, the file has 3,984 after the header; not fixed",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    let o = env.gp().args(["--json", "roms", "scan"]).output().unwrap();
    let json: Value = serde_json::from_slice(&o.stdout).unwrap();
    let kinds: Vec<_> = json["roms"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r["report"]["findings"].as_array().unwrap().clone())
        .map(|f| f["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["ignored-header-bytes", "trailing", "trainer"]);
}

#[test]
fn a_header_that_contradicts_the_registry_is_reported_and_cleaned() {
    // The registry says the board has a battery; the file's header doesn't.
    let env = Env::new(true);
    env.write("roms/nes/test.nes", &rom());
    let o = env.gp().args(["roms", "scan"]).output().unwrap();
    let text = String::from_utf8(o.stdout).unwrap();
    assert!(
        text.contains(
            "header: contradicts the registry (battery: header says false, registry says true)"
        ),
        "{text}"
    );
    let (code, json) = env.clean(&env.path("roms/nes/test.nes"), &env.path("fixed.nes"));
    assert_eq!(code, 0, "{json}");
    let fixed = std::fs::read(env.path("fixed.nes")).unwrap();
    assert_eq!(fixed[6], 0x02);
    assert_eq!(fixed[16..], rom()[16..]);
}

#[test]
fn add_clean_puts_only_the_cleaned_copy_in_the_rom_folder() {
    let env = Env::new(false);
    let input = env.write("downloads/Test (U) [t1].nes", &junk("trainer"));
    let o = env
        .gp()
        .args(["roms", "add", "--clean"])
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let added = env.path("roms/nes/Test (U) [t1].nes");
    assert_eq!(std::fs::read(&added).unwrap(), rom());
    assert!(env.path("roms/nes/Test (U) [t1].nes.clean.toml").exists());
    assert_eq!(std::fs::read(&input).unwrap(), junk("trainer"));

    let patched = env.write("downloads/patched.nes", &junk("patched"));
    let o = env
        .gp()
        .args(["roms", "add", "--clean"])
        .arg(&patched)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));

    let o = env
        .gp()
        .args(["roms", "add", "--clean", "--move"])
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_bare_output_name_in_a_repo_subdirectory_is_refused() {
    let env = Env::new(false);
    let input = env.write("in.nes", &junk("diskdude"));
    std::fs::create_dir_all(env.path("repo/.git")).unwrap();
    std::fs::create_dir_all(env.path("repo/sub")).unwrap();
    let o = env
        .gp()
        .current_dir(env.path("repo/sub"))
        .args(["roms", "clean"])
        .arg(&input)
        .args(["-o", "out.nes"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(!env.path("repo/sub/out.nes").exists());
    assert!(!env.path("repo/sub/out.nes.clean.toml").exists());
}

#[test]
fn an_input_named_like_a_temporary_file_survives() {
    // The old temporary name was `.<out>.partial`; an input with that name
    // must never be written over.
    let env = Env::new(false);
    let input = env.write(".o.nes.partial", &junk("diskdude"));
    let (code, json) = env.clean(&input, &env.path("o.nes"));
    assert_eq!(code, 0, "{json}");
    assert_eq!(std::fs::read(&input).unwrap(), junk("diskdude"));
    assert_eq!(std::fs::read(env.path("o.nes")).unwrap(), rom());
    let leftovers: Vec<_> = std::fs::read_dir(env.home.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".gp-clean-"))
        .collect();
    assert!(leftovers.is_empty());
}
