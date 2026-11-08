//! `NesRuntime::run_until_frames`, which `gp run-until` uses, keeps no
//! events, so its memory stays flat however long it runs. The protocol's
//! `run_until` returns every event and grows with the run; the test checks
//! that it does, so the measurement can tell the two apart.

use gp_evidence::manifest::PowerOn;
use gp_nes::protocol::NesRuntime;
use gp_nes::testrom;
use gp_runtime::{InputFrame, Runtime};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<std::alloc::System> = &INSTRUMENTED_SYSTEM;

/// Bytes still allocated after `f` returns its result, which is kept alive
/// for the measurement.
fn retained<T>(f: impl FnOnce() -> T) -> (T, isize) {
    let region = Region::new(GLOBAL);
    let out = f();
    let change = region.change();
    let net = change.bytes_allocated as isize - change.bytes_deallocated as isize
        + change.bytes_reallocated;
    (out, net)
}

#[test]
fn run_until_frames_keeps_no_events() {
    let frames = 3_000;
    // A condition that never holds, so every frame runs.
    let never = format!("ram[{}] == 0x1ff", testrom::addr::FRAMES);
    let inputs = [InputFrame::pads([0x80, 0, 0, 0])];

    let mut counting = NesRuntime::load(&testrom::build(), PowerOn::default()).unwrap();
    let (met, flat) = retained(|| counting.run_until_frames(&never, &inputs, frames).unwrap());
    assert!(!met);
    assert_eq!(counting.frame(), u64::from(frames));

    let mut rt = NesRuntime::load(&testrom::build(), PowerOn::default()).unwrap();
    let (until, kept) = retained(|| rt.run_until(&never, &inputs, frames).unwrap());
    assert!(!until.met);
    let events = until.events.len();
    assert!(events > frames as usize, "{events} events");

    // Under 64 KB for the whole run, where keeping events takes far more.
    assert!(flat < 64 * 1024, "run_until_frames kept {flat} bytes");
    assert!(
        kept > 16 * flat.max(1),
        "run_until kept {kept} bytes, flat {flat}"
    );
    eprintln!("run_until_frames kept {flat} bytes, run_until {kept} for {events} events");
    // Events come back on for the next step.
    assert!(counting.step(&inputs).unwrap().len() > 1);
}
