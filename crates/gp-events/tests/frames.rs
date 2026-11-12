//! L2 on the small program of tests/common: each frame lists the console's
//! events, the code and edges first seen, the RAM changed and the writes
//! to a watched address with their writer.

mod common;

use std::collections::BTreeSet;

use common::rom;
use gp_events::run_frames;
use gp_evidence::events::EventKind;
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;

#[test]
fn frames_hold_events_novelty_ram_and_watches() {
    let inputs = vec![InputFrame::default(); 12];
    let watch = BTreeSet::from([0x0010u16]);
    let (run, frames) = run_frames(&rom(), PowerOn::default(), &inputs, &watch).unwrap();
    assert_eq!(frames.len(), 12);
    assert_eq!(frames[0].frame, 1);

    // Novelty comes first and stops: every executed byte is new exactly
    // once, and every edge appears as new exactly once.
    let new_bytes: u64 = frames
        .iter()
        .flat_map(|f| &f.new_code)
        .map(|(a, b)| u64::from(b - a))
        .sum();
    assert_eq!(new_bytes, run.collector.footprint.executed.count());
    let new_edges: Vec<_> = frames.iter().flat_map(|f| f.new_edges.clone()).collect();
    let unique: BTreeSet<_> = new_edges.iter().copied().collect();
    assert_eq!(new_edges.len(), unique.len());
    assert_eq!(
        unique.len(),
        run.collector.edges.as_ref().unwrap().table.len()
    );
    assert!(frames.last().unwrap().new_code.is_empty());

    // The NMI handler runs each frame once NMIs are on.
    assert!(frames[5].rom.iter().any(|e| e.kind == EventKind::Nmi));

    // $10 is written with $30 by the STA at $C00E, whose PRG offset is $0E.
    let w: Vec<_> = frames.iter().flat_map(|f| f.watch.clone()).collect();
    assert!(!w.is_empty());
    assert!(
        w.iter()
            .all(|h| h.addr == 0x10 && h.value == 0x30 && h.offset == 0x0E && h.cpu == 0xC00E)
    );
    // It changes value once, in the frame that first writes it.
    let changed: Vec<_> = frames
        .iter()
        .filter(|f| f.ram.iter().any(|r| r.0 == 0x10))
        .collect();
    assert_eq!(changed.len(), 1);

    let (_, again) = run_frames(&rom(), PowerOn::default(), &inputs, &watch).unwrap();
    assert_eq!(again, frames);
}
