//! `gp store` end to end, on a scratch store in a temporary GP_HOME, with
//! gp-nes's own test ROM (built in code, never a game's data). Brought from
//! game-printer's tests/gc.rs. gameprinter has no `gp recordings retire` or
//! `gp input-log generate` yet, so the tests write the retire mark into the
//! log and make input logs with gp-evidence directly.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

struct Env {
    home: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self) -> &Path {
        self.home.path()
    }

    fn gp(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_gp"))
            .env("GP_HOME", self.path())
            .env("GP_ROM_DIR", self.path().join("roms"))
            .env_remove("GP_STORE")
            .env_remove("GP_SOURCES")
            .args(args)
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self.gp(&all);
        assert!(
            out.status.success(),
            "gp {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    /// Runs gp with --json, expecting exit 2, and returns its JSON.
    fn json_exit2(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self.gp(&all);
        assert_eq!(out.status.code(), Some(2), "gp {args:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn put(&self, bytes: &[u8]) -> String {
        let file = self.path().join("loose");
        std::fs::write(&file, bytes).unwrap();
        self.json(&["store", "put", file.to_str().unwrap()])["hash"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn record(&self, name: &str, hold: &str) -> String {
        let roms = self.path().join("roms");
        std::fs::create_dir_all(&roms).unwrap();
        let rom = roms.join("test.nes");
        std::fs::write(&rom, gp_nes::testrom::build()).unwrap();
        let input = self.path().join(format!("{name}.input"));
        let log = gp_evidence::input_log::generate(60, &[hold]).unwrap();
        std::fs::write(&input, log.to_text()).unwrap();
        self.json(&[
            "record",
            "--rom",
            rom.to_str().unwrap(),
            "--input",
            input.to_str().unwrap(),
            "--name",
            name,
        ])["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn retire(&self, id: &str) {
        let mut log = std::fs::OpenOptions::new()
            .append(true)
            .open(self.path().join("store/index/recordings.jsonl"))
            .unwrap();
        writeln!(
            log,
            "{{\"mark\":\"retired\",\"id\":\"{id}\",\"at\":\"2026-10-28T00:00:00Z\"}}"
        )
        .unwrap();
    }

    fn has(&self, hash: &str) -> bool {
        self.path()
            .join("store/objects")
            .join(&hash[..2])
            .join(&hash[2..])
            .is_file()
    }
}

fn hashes(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|o| o["hash"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn put_get_stats_verify_and_reindex() {
    let env = Env::new();
    let hash = env.put(b"some bytes");
    let out = env.path().join("back");
    let got = env.json(&["store", "get", &hash, "--out", out.to_str().unwrap()]);
    assert_eq!(got["bytes"], 10);
    assert_eq!(std::fs::read(&out).unwrap(), b"some bytes");
    let raw = env.gp(&["store", "get", &hash]);
    // The object's bytes, exactly: a command that printed its own output
    // returns no text, so nothing follows them.
    assert_eq!(raw.stdout, b"some bytes");
    let stats = env.json(&["store", "stats"]);
    assert_eq!(
        (stats["objects"].as_u64(), stats["bytes"].as_u64()),
        (Some(1), Some(10))
    );
    assert_eq!(env.json(&["store", "verify"])["checked"], 1);

    env.record("run", "a@5-20");
    let re = env.json(&["store", "reindex"]);
    assert_eq!(re["synced"]["entries"], 1);
    assert_eq!(re["synced"]["recordings"], 1);
    assert_eq!(re["index"], Value::Null);

    // A changed object fails verify, exit 1.
    let path = env
        .path()
        .join("store/objects")
        .join(&hash[..2])
        .join(&hash[2..]);
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&path, perms).unwrap();
    std::fs::write(&path, b"other bytes").unwrap();
    let bad = env.gp(&["--json", "store", "verify"]);
    assert_eq!(bad.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&bad.stdout).unwrap();
    assert_eq!(v["corrupt"], serde_json::json!([hash]));
}

#[test]
fn gc_deletes_only_what_stayed_unused_across_two_runs() {
    let env = Env::new();
    let old = env.record("old", "a@5-20");
    let new = env.record("new", "a@5-30");
    let loose = env.put(b"used by nothing");

    // The first run only lists it; a dry run after it says what would go.
    let first = env.json(&["store", "gc", "--min-age", "0", "--yes"]);
    assert_eq!(first["deleted_objects"], 0);
    assert_eq!(first["pending"], 1);
    assert!(env.has(&loose));
    let dry = env.json(&["store", "gc", "--min-age", "0"]);
    assert_eq!(hashes(&dry["would_delete"]), [loose.as_str()]);
    assert!(env.has(&loose), "a dry run deletes nothing");

    // An object that turns up between runs waits for the next one.
    let later = env.put(b"stored after the first run");
    let second = env.json(&["store", "gc", "--min-age", "0", "--yes"]);
    assert_eq!(hashes(&second["deleted"]), [loose.as_str()]);
    assert!(!env.has(&loose) && env.has(&later));

    // Retired: its objects are listed by one run and deleted by the next,
    // all together, and its recording leaves the log's listings.
    env.retire(&old);
    let without = env.json(&["store", "gc", "--min-age", "0"]);
    assert!(
        hashes(&without["would_delete"]) == [later.as_str()],
        "{without}"
    );
    let third = env.json(&["store", "gc", "--min-age", "0", "--retired", "--yes"]);
    assert_eq!(hashes(&third["deleted"]), [later.as_str()]);
    assert!(third["dropped"].as_array().unwrap().is_empty());
    assert!(env.has(&old), "the manifest waits for the next run");
    // Without --retired, they're used again and nothing would go.
    let dry = env.json(&["store", "gc", "--min-age", "0"]);
    assert!(dry["would_delete"].as_array().unwrap().is_empty(), "{dry}");
    let fourth = env.json(&["store", "gc", "--min-age", "0", "--retired", "--yes"]);
    assert_eq!(fourth["dropped"], serde_json::json!([old.clone()]));
    assert!(fourth["deleted_objects"].as_u64().unwrap() >= 3, "{fourth}");
    assert!(!env.has(&old));
    assert!(env.has(&new));

    let log = std::fs::read_to_string(env.path().join("store/index/recordings.jsonl")).unwrap();
    assert!(log.contains(&format!("{{\"mark\":\"deleted\",\"id\":\"{old}\"")));
    let listed = env.json(&["recordings", "list", "--retired"]);
    let names: Vec<_> = listed["recordings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["new"]);
    assert_eq!(
        env.gp(&["recordings", "show", &old[..8]]).status.code(),
        Some(2)
    );
    // What's left is whole and replays.
    assert_eq!(
        env.json(&["store", "verify"])["corrupt"],
        serde_json::json!([])
    );
    assert_eq!(env.json(&["replay", "--all"])["matched"], 1);
    assert!(
        env.json(&["store", "unreferenced"])["objects"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Recording the same run again brings the ID back.
    assert_eq!(env.record("old", "a@5-20"), old);
    assert_eq!(env.json(&["replay", "--all"])["matched"], 2);
}

#[test]
fn gc_refuses_while_a_log_line_cant_be_read() {
    let env = Env::new();
    env.record("run", "a@5-20");
    let loose = env.put(b"used by nothing");
    env.json(&["store", "gc", "--min-age", "0", "--yes"]);
    let mut log = std::fs::OpenOptions::new()
        .append(true)
        .open(env.path().join("store/index/recordings.jsonl"))
        .unwrap();
    log.write_all(b"{\"id\": \"trunc\n").unwrap();
    drop(log);
    let v = env.json_exit2(&["store", "gc", "--min-age", "0", "--yes"]);
    assert_eq!(v["complete"], false);
    assert!(env.has(&loose), "nothing deleted");
}

#[test]
fn gc_keeps_objects_the_coverage_index_names() {
    let env = Env::new();
    let id = env.record("run", "a@5-20");
    let stored = env.put(b"{\"a coverage result\":1}");
    let loose = env.put(b"used by nothing");
    let index = env.path().join("store/index/coverage.jsonl");
    std::fs::write(
        &index,
        format!(
            "{{\"recording\":\"{id}\",\"coverage_version\":1,\"definition\":null,\"object\":\"{stored}\",\"at\":\"2026-10-27T00:00:00Z\"}}\n"
        ),
    )
    .unwrap();
    env.json(&["store", "gc", "--min-age", "0", "--yes"]);
    let second = env.json(&["store", "gc", "--min-age", "0", "--yes"]);
    assert_eq!(hashes(&second["deleted"]), [loose.as_str()]);
    assert!(env.has(&stored), "coverage results are kept");

    // An unreadable coverage line stops gc before it deletes anything.
    let other = env.put(b"also used by nothing");
    env.json(&["store", "gc", "--min-age", "0", "--yes"]);
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .unwrap();
    f.write_all(b"{\"recording\": \"trunc\n").unwrap();
    drop(f);
    let v = env.json_exit2(&["store", "gc", "--min-age", "0", "--yes"]);
    assert_eq!(v["bad_coverage_lines"], 1);
    assert!(env.has(&other) && env.has(&stored));
}

#[test]
fn gc_keeps_objects_the_validation_index_names() {
    let env = Env::new();
    env.record("run", "a@5-20");
    let stored = env.put(b"{\"a validation result\":1}");
    let loose = env.put(b"used by nothing");
    let index = env.path().join("store/index/validation.jsonl");
    let key = "0".repeat(64);
    std::fs::write(
        &index,
        format!(
            "{{\"key\":\"{key}\",\"validate_version\":1,\"recording\":\"baselines/x.toml\",\"object\":\"{stored}\",\"at\":\"2026-10-28T00:00:00Z\"}}\n"
        ),
    )
    .unwrap();
    env.json(&["store", "gc", "--min-age", "0", "--yes"]);
    let second = env.json(&["store", "gc", "--min-age", "0", "--yes"]);
    assert_eq!(hashes(&second["deleted"]), [loose.as_str()]);
    assert!(env.has(&stored), "validation results are kept");

    // An unreadable line stops gc before it deletes anything.
    let other = env.put(b"also used by nothing");
    env.json(&["store", "gc", "--min-age", "0", "--yes"]);
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .unwrap();
    f.write_all(b"{\"key\": \"trunc\n").unwrap();
    drop(f);
    let v = env.json_exit2(&["store", "gc", "--min-age", "0", "--yes"]);
    assert_eq!(v["bad_validation_lines"], 1);
    assert!(env.has(&other) && env.has(&stored));
}

#[test]
fn gc_waits_for_the_list_to_age() {
    let env = Env::new();
    env.record("run", "a@5-20");
    let loose = env.put(b"used by nothing");
    env.json(&["store", "gc", "--yes"]);
    let list = std::fs::read_to_string(env.path().join("store/gc/pending.json")).unwrap();
    // Straight after, with the default --min-age of 10 minutes: nothing goes,
    // and the list (with its time) is kept.
    let again = env.json(&["store", "gc", "--yes"]);
    assert_eq!(again["waiting"], true);
    assert_eq!(again["deleted_objects"], 0);
    assert!(env.has(&loose));
    assert_eq!(
        std::fs::read_to_string(env.path().join("store/gc/pending.json")).unwrap(),
        list
    );
    let dry = env.json(&["store", "gc"]);
    assert!(dry["would_delete"].as_array().unwrap().is_empty());
}
