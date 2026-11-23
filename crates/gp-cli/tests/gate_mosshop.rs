//! A model gate from end to end, on Moss Hop (templates/mosshop, our own
//! game, built from scripts/mosshop), as a person with a fresh home runs
//! it: an empty HOME, GP_HOME and gate home, a games repo of its own with
//! the ROM-blocking hook, the game made by `gp game new` from the built
//! ROM, two clips saved by `gp clip presses` from Moss Hop's baselines, and
//! a one-line patch to its model.toml gated with `--dry-run`. Every part of
//! the gate runs: `gp game kit`, the hook, the lift, the tip's score and
//! the patch's. It must PASS and leave main where it was.
//!
//! The release's build job can't run this on the bare archive: its runner
//! has no ROM, so no game to gate (docs/release.md). Needs cc65, like
//! just check.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The folders of one person's machine, all under a temp folder.
struct Rig {
    person: PathBuf,
    home: PathBuf,
    roms: PathBuf,
    gate: PathBuf,
    games: PathBuf,
}

impl Rig {
    fn env(&self, c: &mut Command) {
        c.env("HOME", &self.person)
            .env("GP_HOME", &self.home)
            .env("GP_ROM_DIR", &self.roms)
            .env("GP_GATE_HOME", &self.gate)
            .env("GP_GAMES_DIR", &self.games)
            .env("GP", env!("CARGO_BIN_EXE_gp"))
            .env("GIT_AUTHOR_NAME", "A Person")
            .env("GIT_AUTHOR_EMAIL", "person@game-printer.invalid")
            .env("GIT_COMMITTER_NAME", "A Person")
            .env("GIT_COMMITTER_EMAIL", "person@game-printer.invalid")
            .env_remove("GP_STORE")
            .env_remove("GP_TOOL_SRC")
            .env_remove("GP_HIDE_BUILT_SOURCE")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .current_dir(&self.games);
    }

    fn gp(&self, args: &[&str]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_gp"));
        self.env(&mut c);
        c.args(args).output().unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        let mut c = Command::new("git");
        self.env(&mut c);
        let o = c.args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", said(&o));
        String::from_utf8_lossy(&o.stdout).trim().to_owned()
    }
}

fn said(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn ok(o: Output, what: &str) -> Output {
    assert!(o.status.success(), "{what}: {}", said(&o));
    o
}

/// The games repo's ROM-blocking hook and what it reads, as the games
/// repo has them.
fn hooks(games: &Path) {
    for f in [
        ".githooks/pre-commit",
        ".githooks/binary-allowlist",
        ".githooks/guarded-roms.toml",
        ".githooks/rom-excerpt-allowlist",
        "scripts/romguard/rom_text.py",
    ] {
        let to = games.join(f);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(repo().join(f), &to).unwrap();
    }
}

#[test]
fn a_one_line_model_patch_to_moss_hop_passes_the_model_gate_from_a_fresh_home() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let r = Rig {
        person: root.join("person"),
        home: root.join("person/.gameprinter"),
        roms: root.join("roms"),
        gate: root.join("gate"),
        games: root.join("games"),
    };
    for dir in [&r.person, &r.home, &r.gate, &r.games] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let built = Command::new(repo().join("scripts/mosshop/build.sh"))
        .arg(root)
        .output()
        .expect("running scripts/mosshop/build.sh");
    assert!(
        built.status.success(),
        "Moss Hop didn't build; this test needs cc65 (ca65 and ld65), like just check:\n{}",
        said(&built)
    );
    std::fs::create_dir_all(r.roms.join("nes")).unwrap();
    let rom = r.roms.join("nes/mosshop.nes");
    std::fs::copy(root.join("mosshop.nes"), &rom).unwrap();

    // The games repo: its hook first, as every commit runs it.
    r.git(&["init", "-q", "-b", "main"]);
    r.git(&["config", "core.hooksPath", ".githooks"]);
    hooks(&r.games);
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "the ROM-blocking hook"]);

    // The game, and two clips from Moss Hop's baselines.
    ok(
        r.gp(&[
            "game",
            "new",
            "mosshop",
            "--rom",
            rom.to_str().unwrap(),
            "--genre",
            "platformer",
            "--no-draws",
        ]),
        "gp game new",
    );
    let game = r.games.join("mosshop");
    let g = game.to_str().unwrap();
    for (id, baseline, start, name) in [
        ("mosshop.jump.001", "jump-height", "40", "One standing jump"),
        (
            "mosshop.walk.001",
            "walk-and-turn",
            "8",
            "Walk, jump and turn",
        ),
    ] {
        let log = root.join(format!("{baseline}.log"));
        let b = repo().join(format!("baselines/mosshop/{baseline}.toml"));
        ok(
            r.gp(&[
                "input-log",
                "generate",
                "--baseline",
                b.to_str().unwrap(),
                "--out",
                log.to_str().unwrap(),
            ]),
            "gp input-log generate",
        );
        ok(
            r.gp(&[
                "clip",
                "presses",
                g,
                id,
                "--log",
                log.to_str().unwrap(),
                "--start",
                start,
                "--name",
                name,
                "--source",
                "generated",
            ]),
            "gp clip presses",
        );
    }
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "Moss Hop with two clips"]);
    let tip = r.git(&["rev-parse", "main"]);

    // One line of the model changed (the player's description, which the
    // gate scores like any model line), as a patch; the working copy as it
    // was. A comment wouldn't do: the gate holds the model to gp model fmt.
    let model = game.join("model.toml");
    let text = std::fs::read_to_string(&model).unwrap();
    let was = "description = \"The player, not measured yet\"";
    assert!(text.contains(was), "gp game new's player changed:\n{text}");
    let now = "description = \"The player, Moss Hop's hero, not measured yet\"";
    std::fs::write(&model, text.replace(was, now)).unwrap();
    let mut c = Command::new("git");
    r.env(&mut c);
    let diff = String::from_utf8(ok(c.arg("diff").output().unwrap(), "git diff").stdout).unwrap();
    assert_eq!(
        diff.lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .count(),
        1,
        "{diff}"
    );
    let patch = root.join("one-line.patch");
    std::fs::write(&patch, &diff).unwrap();
    r.git(&["checkout", "--", "mosshop/model.toml"]);

    let o = r.gp(&[
        "--json",
        "model",
        "gate",
        g,
        "--patch",
        patch.to_str().unwrap(),
        "--onto",
        "main",
        "-m",
        "mosshop: the player's description",
        "--dry-run",
    ]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: the gate gave no JSON: {}", said(&o)));
    let lines: Vec<&str> = v["lines"]
        .as_array()
        .map(|a| a.iter().filter_map(|l| l.as_str()).collect())
        .unwrap_or_default();
    assert_eq!(v["verdict"], "PASS", "{}\nthe patch:\n{diff}", said(&o));
    assert!(o.status.success(), "{}", said(&o));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(&format!("tip {}: 2 clips scored", &tip[..8]))),
        "the gate didn't score the tip's two clips: {lines:#?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("corpus no worse")),
        "the patch wasn't scored against the tip: {lines:#?}"
    );
    assert_eq!(r.git(&["rev-parse", "main"]), tip, "a dry run moved main");

    // The counts (the owner, 2026-11-22: the same slowness was fixed three
    // times in a day and came back, since nothing counted the work). Each
    // gate run says how many scoring passes it made, in its JSON and in the
    // gate home's steps.jsonl; these numbers are the budget.
    let scores = |v: &serde_json::Value| v["counts"]["scores"].as_u64();
    let dry = scores(&v).expect("a gate run counts its scoring passes");
    assert!(dry >= 1, "a model change is scored: {v:#}");

    // The YES after a dry run that passed on this tip: its commit lands,
    // and nothing is scored again.
    let gate = |patch: &Path, msg: &str| {
        let o = r.gp(&[
            "--json",
            "model",
            "gate",
            g,
            "--patch",
            patch.to_str().unwrap(),
            "--onto",
            "main",
            "-m",
            msg,
        ]);
        let v: serde_json::Value = serde_json::from_slice(&o.stdout)
            .unwrap_or_else(|e| panic!("{e}: the gate gave no JSON: {}", said(&o)));
        assert_eq!(v["verdict"], "PASS", "{}", said(&o));
        v
    };
    let landed = gate(&patch, "mosshop: the player's description");
    assert_eq!(
        scores(&landed),
        Some(0),
        "a landing after its dry run passed scored again: {landed:#}"
    );
    let after_one = r.git(&["rev-parse", "main"]);
    assert_ne!(after_one, tip, "the landing didn't move main");
    r.git(&["checkout", "-q", "main", "--", "mosshop"]);

    // A patch no score reads (the scoreboard, which says what the page
    // shows) scores no clip.
    let board_file = game.join("scoreboard.toml");
    let had = std::fs::read_to_string(&board_file).unwrap_or_default();
    std::fs::write(
        &board_file,
        format!("{had}\n[[line]]\nname = \"the jump\"\nclip = \"mosshop.jump.001\"\n"),
    )
    .unwrap();
    r.git(&["add", "-N", "mosshop/scoreboard.toml"]);
    let mut c = Command::new("git");
    r.env(&mut c);
    let board = String::from_utf8(ok(c.arg("diff").output().unwrap(), "git diff").stdout).unwrap();
    assert!(board.contains("the jump"), "{board}");
    let board_patch = root.join("scoreboard.patch");
    std::fs::write(&board_patch, &board).unwrap();
    if had.is_empty() {
        r.git(&["rm", "-q", "--cached", "-f", "mosshop/scoreboard.toml"]);
        std::fs::remove_file(&board_file).unwrap();
    } else {
        r.git(&["checkout", "-q", "--", "mosshop/scoreboard.toml"]);
    }
    assert_eq!(r.git(&["status", "--porcelain", "--", "mosshop"]), "");
    let free = gate(&board_patch, "mosshop: a scoreboard line");
    assert_eq!(
        scores(&free),
        Some(0),
        "a scoreboard line alone was scored: {free:#}"
    );
    let after_two = r.git(&["rev-parse", "main"]);
    assert_ne!(after_two, after_one, "it didn't land");

    // The next model patch, on the tip those two landings made: the tip's
    // score is the one the landings kept, read with no scoring, so only the
    // patch is scored (docs/gate.md, "The tip's score, kept": the first gate
    // after a landing scored every clip of the new tip).
    r.git(&["checkout", "-q", "main", "--", "mosshop"]);
    let text = std::fs::read_to_string(&model).unwrap();
    assert!(
        text.contains(now),
        "the landing's model isn't on main:\n{text}"
    );
    let next = "description = \"The player, Moss Hop's hero, still not measured\"";
    std::fs::write(&model, text.replace(now, next)).unwrap();
    let mut c = Command::new("git");
    r.env(&mut c);
    let diff2 = String::from_utf8(ok(c.arg("diff").output().unwrap(), "git diff").stdout).unwrap();
    let patch2 = root.join("next-line.patch");
    std::fs::write(&patch2, &diff2).unwrap();
    r.git(&["checkout", "--", "mosshop/model.toml"]);
    let o = r.gp(&[
        "--json",
        "model",
        "gate",
        g,
        "--patch",
        patch2.to_str().unwrap(),
        "--onto",
        "main",
        "-m",
        "mosshop: the player's description again",
        "--dry-run",
    ]);
    let again: serde_json::Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: the gate gave no JSON: {}", said(&o)));
    assert_eq!(again["verdict"], "PASS", "{}", said(&o));
    assert_eq!(
        scores(&again),
        Some(1),
        "the tip a landing scored was scored again: {again:#}"
    );
    let read = format!(
        "tip {}: 2 clips, the score an earlier gate kept",
        &after_two[..8]
    );
    assert!(
        again["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().is_some_and(|l| l.starts_with(&read))),
        "{again:#}"
    );
    assert_eq!(
        r.git(&["rev-parse", "main"]),
        after_two,
        "a dry run moved main"
    );

    // And the steps file has a line for each run with the same count, which
    // gp loop health and the gate's watch read.
    let steps = std::fs::read_to_string(r.gate.join("steps.jsonl")).unwrap();
    let lines: Vec<serde_json::Value> = steps
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .collect();
    let runs: Vec<&serde_json::Value> = lines.iter().filter(|l| l["step"] == "gate").collect();
    assert_eq!(runs.len(), 4, "one gate line per run: {steps}");
    assert_eq!(
        runs.iter()
            .map(|l| l["scores"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![dry, 0, 0, 1],
        "{steps}"
    );
    // The first run scored its tip; the last read the kept score.
    let tips: Vec<bool> = lines
        .iter()
        .filter(|l| l["step"] == "score tip")
        .map(|l| l["kept"] == true)
        .collect();
    assert_eq!(tips, [false, true], "{steps}");
}
