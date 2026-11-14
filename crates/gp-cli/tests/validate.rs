//! `gp validate` on the hand-written Moss Hop model (templates/mosshop)
//! and its baselines: the contract compiled into checks, the original run
//! through the state mapping, results kept in the store, and broken copies
//! of the model failing the checks that cover what was broken. Ported from
//! game-printer; the Bevy and --allow-missing-ext cases went with those
//! options.

use std::path::{Path, PathBuf};

use std::process::Command;

use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn gp(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gp"));
    cmd.env("GP_HOME", home)
        .env("GP_GAMES_DIR", home.join("games"))
        .env_remove("GP_STORE")
        .env_remove("GP_ROM_DIR")
        .env_remove("GP_SOURCES")
        .env_remove("GP_LOG")
        .current_dir(repo());
    cmd
}

/// What assert_cmd's `assert` gave game-printer's tests: the exit code
/// checked, with the output on failure.
trait Assert {
    fn assert(&mut self) -> Ran;
}

struct Ran(std::process::Output);

impl Assert for Command {
    fn assert(&mut self) -> Ran {
        Ran(self.output().unwrap())
    }
}

impl Ran {
    fn code(self, want: i32) -> Self {
        assert_eq!(
            self.0.status.code(),
            Some(want),
            "{}\n{}",
            String::from_utf8_lossy(&self.0.stdout),
            String::from_utf8_lossy(&self.0.stderr)
        );
        self
    }

    fn success(self) -> Self {
        self.code(0)
    }
}

/// Builds Moss Hop into `dir`, lifts its assets into `dir/lifted` for the
/// interpreter, and returns the ROM's path.
fn build_mosshop(dir: &Path) -> PathBuf {
    let out = std::process::Command::new(repo().join("scripts/mosshop/build.sh"))
        .arg(dir)
        .output()
        .expect("running scripts/mosshop/build.sh");
    assert!(
        out.status.success(),
        "Moss Hop didn't build; these tests need cc65 (ca65 and ld65), like just check:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let home = tempfile::tempdir().unwrap();
    gp(home.path())
        .args(["assets", "lift"])
        .arg(dir.join("mosshop.nes"))
        .arg("--map")
        .arg(repo().join("assets/mosshop.toml"))
        .arg("-o")
        .arg(dir.join("lifted"))
        .assert()
        .success();
    dir.join("mosshop.nes")
}

/// The assets lifted next to the ROM.
fn assets(rom: &Path) -> PathBuf {
    rom.with_file_name("lifted")
}

fn validate(home: &Path, model: &Path, rom: &Path, extra: &[&str]) -> (i32, Value) {
    let out = gp(home)
        .args(["--json", "validate"])
        .arg(model)
        .args(["--against", "baselines/mosshop", "--rom"])
        .arg(rom)
        .arg("--assets")
        .arg(assets(rom))
        .args(extra)
        .output()
        .unwrap();
    let json = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "no JSON: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code().unwrap(), json)
}

/// A copy of the template, with `from` replaced by `to` in `file`.
fn broken_copy(dir: &Path, file: &str, from: &str, to: &str) -> PathBuf {
    let out = dir.join("model");
    std::fs::create_dir_all(&out).unwrap();
    for f in ["model.toml", "fidelity.toml", "mapping.toml"] {
        let mut text = std::fs::read_to_string(repo().join("templates/mosshop").join(f)).unwrap();
        if f == file {
            assert_eq!(text.matches(from).count(), 1, "{from}");
            text = text.replace(from, to);
        }
        std::fs::write(out.join(f), text).unwrap();
    }
    out
}

/// Each check's status, as `part quantity (case) against: status`.
fn statuses(json: &Value) -> Vec<String> {
    json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let case = c["case"]
                .as_str()
                .map(|s| format!(" ({s})"))
                .unwrap_or_default();
            format!(
                "{} {}{case} {}: {}",
                c["part"].as_str().unwrap(),
                c["quantity"].as_str().unwrap(),
                c["against"].as_str().unwrap(),
                c["status"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn moss_hop_validates_at_its_contracts_levels() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path());
    let model = repo().join("templates/mosshop");
    let (code, json) = validate(home.path(), &model, &rom, &[]);
    assert_eq!(code, 0, "{json:#}");
    let mut want = Vec::new();
    for against in ["model", "original"] {
        for c in [
            "entities.player.jump height",
            "entities.player.jump airtime",
            "entities.player.run speed (walk)",
            "entities.player.run speed (run)",
            "entities.slug.patrol speed (patrol)",
            "cameras.main lead (scrolling)",
            "rules.game state",
            "counters.deaths value",
            "counters.lives value",
            "counters.stomps value",
            "counters.coins value",
            "entities.player.machine state",
            "entities.slug.machine state",
            "levels.main graph",
        ] {
            want.push(format!("{c} {against}: pass"));
        }
    }
    // The coins against the model's own events: block.coin and
    // coin.touched, fired by a head hit and a pickup, show in the original
    // as one more used block or sky cell of the mapping's [cells].
    // The level is playable by the player's moves, checked on the model.
    want.push("levels.main reachable model: pass".into());
    let mut got = statuses(&json);
    got.sort();
    want.sort();
    assert_eq!(got, want);
    // The levels they ran at are the contract's.
    let level = |part: &str| {
        json["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["part"] == part)
            .unwrap()["level"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(level("entities.player.jump"), "metric");
    assert_eq!(level("rules.game"), "exact");
    assert_eq!(level("levels.main"), "structural");
    // The facts no check measures are listed, art first among them.
    assert!(
        json["unchecked"]["style"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "entities.player.sprite.palette")
    );
    // Eight baselines and the model's reachability.
    assert_eq!(json["recordings"].as_array().unwrap().len(), 9);

    // Results are kept: the same run again reads them back.
    let out = gp(home.path())
        .args([
            "validate",
            "templates/mosshop",
            "--against",
            "baselines/mosshop",
            "--rom",
        ])
        .arg(&rom)
        .arg("--assets")
        .arg(assets(&rom))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert_eq!(text.matches("\n  stored    ").count(), 8, "{text}");
    let store = home.path().join("store");
    let index = std::fs::read_to_string(store.join("index/validation.jsonl")).unwrap();
    assert_eq!(index.lines().count(), 8);

    // With --no-save, nothing is read or written, and the checks run again.
    let home2 = tempfile::tempdir().unwrap();
    let (code, json2) = validate(home2.path(), &model, &rom, &["--no-save"]);
    assert_eq!(code, 0);
    assert_eq!(json2["checks"], json["checks"]);
    assert!(!home2.path().join("store").exists());
}

#[test]
fn a_model_that_differs_from_the_recordings_fails() {
    if gp_arcade::skip_slow_test() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path());
    for (i, (file, from, to, failing)) in [
        // Lower gravity: a higher, longer jump than the recordings show.
        (
            "model.toml",
            "gravity = \"0.25\"",
            "gravity = \"0.1875\"",
            &[
                "entities.player.jump height model: fail",
                "entities.player.jump airtime model: fail",
            ][..],
        ),
        // A shorter dying state.
        (
            "model.toml",
            "to = \"playing\"\nafter = 90",
            "to = \"playing\"\nafter = 80",
            &["rules.game state model: fail"][..],
        ),
        // A faster slug, past the 3% tolerance.
        (
            "model.toml",
            "[entities.slug.patrol]\nperiod = 2\nstep = 1",
            "[entities.slug.patrol]\nperiod = 2\nstep = \"1.0625\"",
            &["entities.slug.patrol speed (patrol) model: fail"][..],
        ),
        // A camera that keeps the player further back than the game's.
        (
            "model.toml",
            "lead = [120, 0]",
            "lead = [100, 0]",
            &["cameras.main lead (scrolling) model: fail"][..],
        ),
        // A counter on the wrong event.
        (
            "model.toml",
            "[[counters.stomps.change]]\non = \"enemy.stomped\"",
            "[[counters.stomps.change]]\non = \"player.died\"",
            &["counters.stomps value model: fail"][..],
        ),
        // A squashed slug that lies flat longer than the game's 30 frames.
        (
            "model.toml",
            "after = 30\ndo = [\"remove\"]",
            "after = 40\ndo = [\"remove\"]",
            &["entities.slug.machine state model: fail"][..],
        ),
        // A coin taken that the coins counter doesn't count.
        (
            "model.toml",
            "[[counters.coins.change]]\non = \"coin.touched\"\n",
            "",
            &["counters.coins value model: fail"][..],
        ),
        // Tolerances wide enough let the lower gravity through.
        (
            "fidelity.toml",
            "airtime = 2\nheight = \"5%\"",
            "airtime = 13\nheight = \"40%\"",
            &[][..],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = broken_copy(&tmp.path().join(i.to_string()), file, from, to);
        if file == "fidelity.toml" {
            let m = std::fs::read_to_string(dir.join("model.toml")).unwrap();
            std::fs::write(
                dir.join("model.toml"),
                m.replace("gravity = \"0.25\"", "gravity = \"0.1875\""),
            )
            .unwrap();
        }
        let (code, json) = validate(home.path(), &dir, &rom, &["--runtime", "none"]);
        let failed: Vec<String> = statuses(&json)
            .into_iter()
            .filter(|s| s.ends_with("fail"))
            .collect();
        assert_eq!(failed, failing, "{to}: {json:#}");
        assert_eq!(code, if failing.is_empty() { 0 } else { 1 }, "{to}");
        assert!(json["subject"].is_null());
    }
}

#[test]
fn the_interpreter_runs_what_the_model_says() {
    // With lower gravity the interpreter jumps higher than the game, so the
    // jump fails against the original as well as against the model's
    // numbers; everything else still agrees.
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path());
    let dir = broken_copy(
        tmp.path(),
        "model.toml",
        "gravity = \"0.25\"",
        "gravity = \"0.1875\"",
    );
    let (code, json) = validate(home.path(), &dir, &rom, &["--no-save"]);
    assert_eq!(code, 1);
    assert!(json["subject"].as_str().unwrap().starts_with("gp-interp"));
    let failed: Vec<String> = statuses(&json)
        .into_iter()
        .filter(|s| s.ends_with("fail"))
        .collect();
    assert!(
        failed.contains(&"entities.player.jump height original: fail".to_owned()),
        "{failed:?}"
    );
    assert!(
        failed.contains(&"entities.player.jump height model: fail".to_owned()),
        "{failed:?}"
    );
    // The slugs walk as the game's whatever the player does; which of
    // them the player stomps follows the jump, so their states may part.
    assert!(
        !failed.iter().any(|f| f.starts_with("entities.slug.patrol")),
        "{failed:?}"
    );
}

#[test]
fn conditions_choose_the_level_frame_by_frame() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path());
    let with = |i: usize, extra: &str| {
        let dir = broken_copy(
            &tmp.path().join(i.to_string()),
            "model.toml",
            "to = \"playing\"\nafter = 90",
            "to = \"playing\"\nafter = 80",
        );
        let mut c = std::fs::read_to_string(dir.join("fidelity.toml")).unwrap();
        c.push_str(extra);
        std::fs::write(dir.join("fidelity.toml"), c).unwrap();
        validate(home.path(), &dir, &rom, &["--runtime", "none", "--no-save"])
    };
    let rules_game = |json: &Value| {
        statuses(json)
            .into_iter()
            .find(|s| s.starts_with("rules.game state model"))
            .unwrap()
    };
    // The dying time is wrong, but no check runs while a death is counted.
    let (code, json) = with(
        0,
        "\n[[rule]]\npath = \"rules.game\"\nlevel = \"free\"\nwhen = \"counters.deaths >= 1\"\n",
    );
    assert_eq!(code, 0, "{json:#}");
    assert_eq!(rules_game(&json), "rules.game state model: pass");
    // Two conditions that hold together with different levels fail where
    // they meet.
    let (code, json) = with(
        1,
        "\n[[rule]]\npath = \"rules.game\"\nlevel = \"free\"\nwhen = \"counters.deaths >= 1\"\n\n[[rule]]\npath = \"rules.game\"\nlevel = \"exact\"\nwhen = \"counters.stomps >= 0\"\n",
    );
    assert_eq!(code, 1);
    let check = json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["part"] == "rules.game")
        .unwrap();
    assert!(
        check["example"]["detail"]
            .as_str()
            .unwrap()
            .contains("both hold"),
        "{check:#}"
    );
    // A condition on a name can't be read.
    let (code, json) = with(
        2,
        "\n[[rule]]\npath = \"rules.game\"\nlevel = \"free\"\nwhen = \"rules.game.state == 1\"\n",
    );
    assert_eq!(code, 1);
    assert!(json.to_string().contains("compares numbers"), "{json:#}");
}

#[test]
fn exits_2_when_nothing_can_run() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("roms")).unwrap();
    // No --rom and no Moss Hop in the ROM folder: every baseline is skipped.
    let out = gp(home.path())
        .args([
            "validate",
            "templates/mosshop",
            "--against",
            "baselines/mosshop",
            "--no-save",
            "--runtime",
            "none",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).contains("skip"));
    // A model without a contract can't be validated.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::copy(
        repo().join("templates/mosshop/model.toml"),
        tmp.path().join("model.toml"),
    )
    .unwrap();
    gp(home.path())
        .args(["validate"])
        .arg(tmp.path())
        .args(["--against", "baselines/mosshop"])
        .assert()
        .code(2);
}

/// A model naming an `ext` hook is refused without its game's module
/// (`--module`): gp runs hooks only there.
#[test]
fn a_model_naming_missing_hooks_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rom = build_mosshop(tmp.path());
    let dir = broken_copy(
        tmp.path(),
        "model.toml",
        "[entities.slug]\nrole = \"enemy\"",
        "[entities.slug]\next = \"nope\"\nrole = \"enemy\"",
    );
    let out = gp(home.path())
        .args(["validate"])
        .arg(&dir)
        .args(["--against", "baselines/mosshop", "--no-save", "--rom"])
        .arg(&rom)
        .arg("--assets")
        .arg(assets(&rom))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("run only in its game's module (--module): nope"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
