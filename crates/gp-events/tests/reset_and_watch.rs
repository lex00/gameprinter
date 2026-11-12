//! A soft reset is one edge, with no bogus transfer and no stack write; a
//! watch on a register gives the value written; a board whose banks the
//! harvest doesn't model gets no banked name.

mod common;

use std::collections::BTreeSet;

use common::rom;
use gp_events::names::Namer;
use gp_events::{EdgeKind, run, run_frames};
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;

#[test]
fn a_soft_reset_is_one_edge() {
    let mut inputs = vec![InputFrame::default(); 12];
    inputs[6].reset = true;
    let r = run(&rom(), PowerOn::default(), &inputs, true).unwrap();
    let e = r.collector.edges.as_ref().unwrap();
    let resets: Vec<_> = e.table.keys().filter(|k| k.0 == EdgeKind::Reset).collect();
    assert_eq!(resets.len(), 1, "{resets:?}");
    assert_eq!(
        resets[0].2, 0,
        "to the reset handler at $C000, PRG offset 0"
    );
    // No transfer lands outside PRG ROM: the program never runs from RAM.
    assert!(
        e.table.keys().all(|k| k.2 != gp_events::NO_ROM),
        "{:?}",
        e.table.keys().collect::<Vec<_>>()
    );
    // Nothing writes the stack below where the program's own pushes reach.
    let plain = run(
        &rom(),
        PowerOn::default(),
        &[InputFrame::default(); 12],
        true,
    )
    .unwrap();
    let stack = |c: &gp_events::Collector| {
        (0x100..0x200)
            .filter(|&i| c.footprint.ram_written.contains(i))
            .collect::<Vec<_>>()
    };
    assert!(
        stack(&r.collector)
            .iter()
            .all(|i| stack(&plain.collector).contains(i))
    );
}

#[test]
fn a_watch_on_a_register_gives_the_value_written() {
    let inputs = vec![InputFrame::default(); 4];
    // The program writes #$80 to $2000; watch it through a mirror.
    let (_, frames) = run_frames(
        &rom(),
        PowerOn::default(),
        &inputs,
        &BTreeSet::from([0x2008u16]),
    )
    .unwrap();
    let hits: Vec<_> = frames.iter().flat_map(|f| f.watch.clone()).collect();
    assert!(!hits.is_empty());
    assert!(
        hits.iter()
            .all(|h| h.addr == 0x2000 && h.value == 0x80 && h.cpu == 0xC006)
    );
}

#[test]
fn a_bank_the_board_doesnt_model_gets_no_banked_name() {
    // 64 KB on AxROM (mapper 7): the harvest's board switches nothing on
    // it, so a byte of the first 32 KB run at $8009 isn't where the power-on
    // view puts $8009 (the last 16 KB start at $C000).
    let mut rom = b"NES\x1a\x04\x00\x70\x00".to_vec();
    rom.resize(16, 0);
    rom.extend(vec![0xEAu8; 0x10000]);
    let n = Namer::of_ines(&rom).unwrap();
    assert_eq!(n.va(0x8009, 0x8009), None);
    assert_eq!(n.name(0x8009, 0x8009), "prg:08009@8009");
    // The last 16 KB at $C000 is where the view puts it.
    assert_eq!(n.va(0xC000, 0xC000), Some(0xC000));
}
