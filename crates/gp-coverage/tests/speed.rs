//! How long coverage takes, against plain replay, on a stored recording:
//! `GP_STORE=... GP_COVERAGE_ROM=rom.nes GP_COVERAGE_ID=<prefix> cargo test
//! --release -p gp-coverage --test speed -- --ignored --nocapture`. Only
//! reads the store. The numbers are in docs/coverage.md.

use std::time::Instant;

use gp_events::{Footprint as Observer, Layout};
use gp_evidence::Store;
use gp_nes::seek;

#[test]
#[ignore = "needs a stored recording and its ROM; see the file's comment"]
fn compare_replay_and_coverage() {
    let (Ok(store), Ok(rom), Ok(id)) = (
        std::env::var("GP_STORE"),
        std::env::var("GP_COVERAGE_ROM"),
        std::env::var("GP_COVERAGE_ID"),
    ) else {
        eprintln!("set GP_STORE, GP_COVERAGE_ROM and GP_COVERAGE_ID");
        return;
    };
    let store = Store::open_existing(store).unwrap();
    let rom = gp_evidence::source::identify_nes(&std::fs::read(rom).unwrap())
        .map(|_| ())
        .ok();
    assert!(rom.is_some());
    let rom = std::fs::read(std::env::var("GP_COVERAGE_ROM").unwrap()).unwrap();
    let entry = gp_evidence::recordings::resolve(&store, &id).unwrap();
    let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id).unwrap();
    let frames = manifest.frames;
    let fps = |t: Instant| frames as f64 / t.elapsed().as_secs_f64();

    // Plain replay from power-on.
    let t = Instant::now();
    let mut cursor = seek::open(&store, &manifest, &rom, 0).unwrap();
    cursor.advance_to(frames).unwrap();
    assert!(cursor.matches_recording().unwrap());
    println!("replay:                 {:>8.0} frames/s", fps(t));

    // The coverage hook, one thread, from power-on.
    let t = Instant::now();
    let mut cursor = seek::open(&store, &manifest, &rom, 0).unwrap();
    let mut obs = Observer::new(Layout::of(cursor.nes().bus()).unwrap());
    while cursor.frame() < frames {
        let input = cursor.input(cursor.frame()).unwrap();
        cursor
            .nes_mut()
            .step_with(input, |bus| obs.before(bus))
            .unwrap();
    }
    assert!(cursor.matches_recording().unwrap());
    println!(
        "observer, one thread:   {:>8.0} frames/s ({} executed bytes)",
        fps(t),
        obs.executed.count()
    );

    // tetanes-core's own code map, recorded inside clock_instr: the cost a
    // fork hook on the same path would have.
    let t = Instant::now();
    let mut cursor = seek::open(&store, &manifest, &rom, 0).unwrap();
    cursor
        .nes_mut()
        .bus_mut_for_observers()
        .attach_code_map(None);
    cursor.advance_to(frames).unwrap();
    println!("tetanes code map:       {:>8.0} frames/s", fps(t));

    // The whole measure, segments in parallel.
    for threads in [1, 4, 8] {
        let t = Instant::now();
        let cov = gp_coverage::measure::measure(&store, entry.id, &manifest, &rom, None, threads)
            .unwrap();
        println!(
            "measure, {threads} threads:     {:>8.0} frames/s ({} executed bytes, {} RAM bytes written)",
            fps(t),
            cov.executed.count(),
            cov.ram_written.count()
        );
    }
}
