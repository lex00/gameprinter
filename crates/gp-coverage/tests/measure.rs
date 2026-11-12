//! Measuring stored recordings of our test program: segments on several
//! threads give the same coverage as one replay from power-on, a recording
//! by another runtime replays from power-on, a replay that doesn't match is
//! refused, and stored coverage comes back as it went in.

use std::path::Path;

use gp_coverage::definition::Definition;
use gp_coverage::measure::{MeasureError, measure};
use gp_coverage::{Coverage, cache};
use gp_evidence::input_log::generate;
use gp_evidence::manifest::{PowerOn, Provenance, RecordingManifest, SourceRef};
use gp_evidence::{Hash, Store};
use gp_nes::recording::{RecordRequest, record};
use gp_nes::testrom;

/// The test program counts NMIs in $00 and keeps controller 1 in $01.
const DEFINITION: &str = r#"
schema = 1
source = "gp-testrom"
title = "gp-testrom"

[screens]
parts = [{ name = "high", read = "ram[0x00]" }]
screen = "count {high}"
level = "all"

[objects]
slots = [{ id = 1, active = "ram[0x01]" }]
names = { "1" = "pad" }

[thresholds]
levels = ["all"]
inputs = ["none", "a", "right"]
"#;

fn recorded(store: &Store, rom: &[u8], spans: &[&str]) -> (Hash, RecordingManifest) {
    let input = generate(300, spans).unwrap();
    let r = record(
        store,
        RecordRequest {
            rom,
            source: SourceRef::NesRom {
                sha256: gp_evidence::source::identify_nes(rom).unwrap().sha256,
                mapper: 0,
            },
            source_id: None,
            power_on: PowerOn::default(),
            input: &input,
            checkpoint_interval: 30,
            provenance: Provenance::Scripted,
            name: "coverage".into(),
            created_at: "2026-10-27T00:00:00Z".into(),
            notes: None,
        },
    )
    .unwrap();
    (r.id, r.manifest)
}

fn definition() -> Definition {
    Definition::parse(DEFINITION.as_bytes(), Path::new("gp-testrom.toml")).unwrap()
}

#[test]
fn segments_on_threads_match_one_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let rom = testrom::build();
    let (id, manifest) = recorded(&store, &rom, &["a@10-20", "right@50-"]);
    let def = definition();
    let one = measure(&store, id, &manifest, &rom, Some(&def), 1).unwrap();
    let many = measure(&store, id, &manifest, &rom, Some(&def), 4).unwrap();
    assert_eq!(one, many);
    assert_eq!(one.frames, 300);
    assert!(one.executed.count() > 0 && one.ram_written.count() > 0);
    assert!(one.ram_written.contains(0x00), "the NMI counter is written");
    assert_eq!(
        one.inputs[0].keys().collect::<Vec<_>>(),
        ["a", "none", "right"]
    );
    assert!(one.levels.contains_key("all"));
    assert!(one.objects.contains_key("pad"));
    let check = one.check(def.thresholds.as_ref().unwrap());
    assert!(check.passed, "{check:?}");

    // Another runtime's recording replays from power-on, with the same result.
    let mut stale = manifest.clone();
    stale.runtime.version = "0 (an older gp-nes)".into();
    assert_eq!(
        measure(&store, id, &stale, &rom, Some(&def), 4).unwrap(),
        one
    );
}

#[test]
fn a_replay_that_doesnt_match_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let rom = testrom::build();
    let (id, mut manifest) = recorded(&store, &rom, &["a@10-20"]);
    let (_, other) = recorded(&store, &rom, &["right@100-"]);
    // The first recording's hashes with the second's input.
    manifest.input_log = other.input_log;
    for threads in [1, 4] {
        let e = measure(&store, id, &manifest, &rom, None, threads).unwrap_err();
        assert!(matches!(e, MeasureError::Diverged { .. }), "{e}");
    }
}

#[test]
fn stored_coverage_comes_back_and_unions_per_rom() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let rom = testrom::build();
    let def = definition();
    let (a, ma) = recorded(&store, &rom, &["a@10-20"]);
    let (b, mb) = recorded(&store, &rom, &["right@100-"]);
    let ca = measure(&store, a, &ma, &rom, Some(&def), 2).unwrap();
    let cb = measure(&store, b, &mb, &rom, Some(&def), 2).unwrap();

    assert_eq!(cache::lookup(&store, a, Some(def.hash)).unwrap(), None);
    let object = cache::save(&store, &ca, "2026-10-27T00:00:00Z").unwrap();
    assert_eq!(
        cache::lookup(&store, a, Some(def.hash)).unwrap(),
        Some(ca.clone())
    );
    assert_eq!(
        cache::lookup(&store, a, None).unwrap(),
        None,
        "another definition"
    );
    assert_eq!(cache::lookup(&store, b, Some(def.hash)).unwrap(), None);
    assert_eq!(cache::lines(&store).unwrap()[0].object, object);

    let both = Coverage::union(&[ca.clone(), cb.clone(), ca.clone()]).unwrap();
    assert_eq!(both.recordings, vec![a, b]);
    assert_eq!(both.frames, 600);
    assert_eq!(
        both.inputs[0]["none"],
        ca.inputs[0]["none"] + cb.inputs[0]["none"]
    );
    assert_eq!(both.levels["all"].recordings, 2);
    assert_eq!(both.levels["all"].first, None);
    assert!(both.executed.count() >= ca.executed.count());

    let mut other_rom = cb.clone();
    other_rom.sha256 = "0".repeat(64);
    assert!(Coverage::union(&[ca.clone(), other_rom]).is_err());
    let mut no_def = cb.clone();
    no_def.definition = None;
    assert!(Coverage::union(&[ca.clone(), no_def]).is_err());
    // A union that shares only some recordings with another would count
    // them twice.
    assert!(
        Coverage::union(&[
            both.clone(),
            Coverage::union(&[ca.clone(), cb.clone()]).unwrap(),
            ca.clone()
        ])
        .is_ok()
    );
    let mut partial = both.clone();
    partial.recordings = vec![a, Hash::of(b"another")];
    assert!(Coverage::union(&[ca.clone(), partial]).is_err());

    // A newer line whose object is gone falls back to an older good one.
    let mut gone = ca.clone();
    gone.frames += 1;
    let object = cache::save(&store, &gone, "2026-10-28T00:00:00Z").unwrap();
    std::fs::rename(store.path_of(&object), dir.path().join("moved")).unwrap();
    assert_eq!(cache::lookup(&store, a, Some(def.hash)).unwrap(), Some(ca));
}
