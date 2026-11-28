//! Moss Hop's level as authored data (row moss-hop-level-as-data): the
//! level of templates/mosshop is a file in the template
//! (level-data/level.toml, which scripts/mosshop/build.sh assembles the
//! ROM's level from), read with no ROM, no lift and no cc65, and `gp level
//! edit` on a copy changes that file. crates/gp-cli/tests/gate_mosshop.rs
//! holds the built ROM's lift to the same files.
//!
//! A fresh home with no lifted assets, no ROM folder and a PATH with
//! neither ca65 nor ld65 on it: the template is copied into the home's
//! work area (where an edit may write), its level is read with `gp level
//! show --data`, a dry run of an edit writes nothing, and the real edit
//! writes the copy's level file and nothing in the home's lifts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The PATH the test runs gp with: one folder that isn't there, so no
/// program at all is found on it, cc65's included. The system's own
/// folders won't do: a Linux machine with cc65 installed has ca65 in
/// /usr/bin (GitHub's check failed on that on 2026-11-27).
const PATH: &str = "/gp-level-data-test/no-such-folder";

fn gp(home: &Path, person: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gp"))
        .env("HOME", person)
        .env("GP_HOME", home)
        .env("PATH", PATH)
        .env_remove("GP_ROM_DIR")
        .env_remove("GP_STORE")
        .env_remove("GP_GAMES_DIR")
        .env_remove("GP_TOOL_SRC")
        .env_remove("GP_STEP")
        .env_remove("GP_WORKER")
        .current_dir(person)
        .args(args)
        .output()
        .unwrap()
}

fn said(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn json(o: &Output, what: &str) -> serde_json::Value {
    assert!(o.status.success(), "{what}: {}", said(o));
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{what}: {e}: {}", said(o)))
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).unwrap();
        }
    }
}

/// Every file under `dir` with its bytes, by its path from `dir`.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![dir.to_owned()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                todo.push(p);
            } else {
                out.insert(
                    p.strip_prefix(dir).unwrap().to_owned(),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }
    out
}

fn level_rows(v: &serde_json::Value) -> Vec<Vec<i64>> {
    serde_json::from_value(v["data"]["slots"]["level"].clone())
        .unwrap_or_else(|e| panic!("{e}: gp level show --data gave no level rows: {v:#}"))
}

/// The data folder's asset map is the repo's (assets/mosshop.toml, which
/// template.toml pins by its hash): one map, kept in two places so a copy
/// of the template carries it.
#[test]
fn the_level_datas_map_is_moss_hops_asset_map() {
    let map = std::fs::read(repo().join("assets/mosshop.toml")).unwrap();
    let data = std::fs::read(repo().join("templates/mosshop/level-data/assets.toml")).unwrap();
    assert!(
        map == data,
        "templates/mosshop/level-data/assets.toml differs from assets/mosshop.toml: copy the map there again"
    );
}

#[test]
fn moss_hops_level_is_data_read_and_edited_with_no_rom_or_cc65() {
    for dir in PATH.split(':') {
        for tool in ["ca65", "ld65"] {
            assert!(
                !Path::new(dir).join(tool).exists(),
                "{dir}/{tool} is on this test's PATH, so it can't show the level needs no cc65"
            );
        }
    }
    let d = tempfile::tempdir().unwrap();
    let person = d.path().join("person");
    let home = person.join(".gameprinter");
    std::fs::create_dir_all(&home).unwrap();
    let copy = home.join("work/mcp/hop");
    copy_dir(&repo().join("templates/mosshop"), &copy);
    let c = copy.to_str().unwrap();

    // Read with nothing lifted.
    let shown = json(
        &gp(&home, &person, &["--json", "level", "show", c, "--data"]),
        "gp level show",
    );
    let rows = level_rows(&shown);
    assert_eq!(rows.len(), 15, "{shown:#}");
    assert!(rows.iter().all(|r| r.len() == 32), "{rows:?}");

    // A coin (metatile 8) put in the sky over the start.
    let mut row = rows[3].clone();
    assert_eq!(row[6], 0, "row 3's cell 6 isn't sky: {row:?}");
    row[6] = 8;
    let op = format!(
        "set row level 3 {}",
        row.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
    );
    let before = files(&copy);

    let dry = json(
        &gp(
            &home,
            &person,
            &[
                "--json",
                "level",
                "edit",
                c,
                "--op",
                &op,
                "--no-measures",
                "--dry-run",
            ],
        ),
        "gp level edit --dry-run",
    );
    assert_eq!(dry["dry_run"], true, "{dry:#}");
    assert_eq!(files(&copy), before, "a dry run changed the copy");

    json(
        &gp(
            &home,
            &person,
            &["--json", "level", "edit", c, "--op", &op, "--no-measures"],
        ),
        "gp level edit",
    );
    let after = files(&copy);
    let changed: Vec<&PathBuf> = after
        .iter()
        .filter(|(p, b)| before.get(*p) != Some(*b))
        .map(|(p, _)| p)
        .filter(|p| p.as_os_str() != "level-edits.jsonl")
        .collect();
    assert_eq!(
        changed.len(),
        1,
        "the edit should change the copy's level file alone: {changed:?}"
    );
    let text = String::from_utf8(after[changed[0]].clone()).unwrap();
    let was = String::from_utf8(before.get(changed[0]).cloned().unwrap_or_default()).unwrap();
    assert!(
        text.contains("rows = ["),
        "{} isn't the level's grid:\n{text}",
        changed[0].display()
    );
    assert_ne!(text, was);
    assert!(
        !home.join("work/assets").exists(),
        "the level was read or written through a lift in the home"
    );

    let again = json(
        &gp(&home, &person, &["--json", "level", "show", c, "--data"]),
        "gp level show after the edit",
    );
    assert_eq!(level_rows(&again)[3], row, "the edit isn't what's read");
    assert_eq!(level_rows(&again)[4], rows[4]);
}
