//! The search on a hand-made program whose routine at $C100 runs only
//! while A is held: from power-on with nothing pressed it never runs; the
//! search, aimed at it through the harvest's call graph, finds presses
//! that run it, the same ones for the same seed.

use std::collections::BTreeSet;

use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_explore::{CallGraph, NewCode, Opts, Progress, Start, Toward, search, search_watched};
use gp_nes::codemap::Board;

#[rustfmt::skip]
const MAIN: &[u8] = &[
    0xA9, 0x80,       // C000 LDA #$80
    0x8D, 0x00, 0x20, // C002 STA $2000      NMI on
    0x4C, 0x05, 0xC0, // C005 JMP $C005      wait
];

#[rustfmt::skip]
const NMI: &[u8] = &[
    0xA9, 0x01,       // C010 LDA #1
    0x8D, 0x16, 0x40, // C012 STA $4016      strobe
    0xA9, 0x00,       // C015 LDA #0
    0x8D, 0x16, 0x40, // C017 STA $4016
    0xAD, 0x16, 0x40, // C01A LDA $4016      A is the first bit
    0x29, 0x01,       // C01D AND #1
    0xF0, 0x03,       // C01F BEQ $C024
    0x20, 0x00, 0xC1, // C021 JSR $C100      only with A held
    0x40,             // C024 RTI
];

fn rom() -> Vec<u8> {
    let mut prg = vec![0xEAu8; 0x4000];
    prg[..MAIN.len()].copy_from_slice(MAIN);
    prg[0x10..0x10 + NMI.len()].copy_from_slice(NMI);
    prg[0x100..0x103].copy_from_slice(&[0xE6, 0x40, 0x60]); // C100 INC $40; RTS
    prg[0x3FFA..0x3FFC].copy_from_slice(&0xC010u16.to_le_bytes());
    prg[0x3FFC..0x3FFE].copy_from_slice(&0xC000u16.to_le_bytes());
    prg[0x3FFE..0x4000].copy_from_slice(&0xC024u16.to_le_bytes());
    let mut rom = b"NES\x1a\x01\x00\x00\x00".to_vec();
    rom.resize(16, 0);
    rom.extend(prg);
    rom
}

#[test]
fn finds_the_press_that_runs_a_routine_and_repeats_it() {
    let rom = rom();
    let idle = vec![InputFrame::default(); 30];
    let known = gp_events::Levels::of(
        &gp_events::run(&rom, PowerOn::default(), &idle, true)
            .unwrap()
            .collector,
        30,
    );
    assert!(
        !known.footprint.starts.contains(0x100),
        "idle never runs $C100"
    );

    let (prg, mapper) = gp_nes::harvest::ines_prg(&rom).unwrap();
    let board = Board::of(prg, mapper);
    let h = gp_nes::harvest::harvest_board(&board, &gp_nes::harvest::Input::default());
    let graph = CallGraph::of(&board, &h);
    let target = graph.index("sub_C100").expect("the code map has $C100");
    assert_eq!(
        graph.distances_to(&BTreeSet::from([target]))[target],
        Some(0)
    );

    let start = Start::of(&rom, PowerOn::default(), &idle).unwrap();
    let opts = Opts {
        seed: 3,
        budget: 200,
        min_frames: 10,
        max_frames: 40,
    };
    let go = || {
        let mut s = Toward::new(graph.clone(), BTreeSet::from([target]), &known);
        let found = search(&rom, std::slice::from_ref(&start), &mut s, &opts).unwrap();
        (
            s.reached.clone(),
            found.iter().map(|f| f.pads.clone()).collect::<Vec<_>>(),
        )
    };
    let (reached, pads) = go();
    assert_eq!(reached, vec![target]);
    assert!(
        pads[0]
            .iter()
            .any(|p| p & gp_evidence::input_log::button::A != 0)
    );
    assert_eq!(
        go(),
        (reached.clone(), pads.clone()),
        "the same seed gives the same search"
    );
    // A watcher hears every candidate and every kept one, and changes
    // nothing.
    let (mut ran, mut kept) = (0, Vec::new());
    let mut s = Toward::new(graph.clone(), BTreeSet::from([target]), &known);
    let watched = search_watched(
        &rom,
        std::slice::from_ref(&start),
        &mut s,
        &opts,
        &mut |p| match p {
            Progress::Ran { ran: n, .. } => ran = n,
            Progress::Kept { found, .. } => kept.push(found.pads.clone()),
        },
    )
    .unwrap();
    let watched: Vec<Vec<u8>> = watched.iter().map(|f| f.pads.clone()).collect();
    assert_eq!(watched, pads);
    assert!(ran > 0);
    assert_eq!(kept.len(), pads.len());
    assert!(kept.iter().all(|k| pads.contains(k)));

    // With no target, new code alone finds it too.
    let mut s = NewCode::new(&known);
    let found = search(&rom, std::slice::from_ref(&start), &mut s, &opts).unwrap();
    assert!(
        found
            .iter()
            .any(|f| f.levels.footprint.starts.contains(0x100))
    );
}

/// A clip the search keeps is a recipe: the prefix's input from power-on
/// and then its presses run what the search saw from the snapshot.
#[test]
fn a_kept_candidate_replays_from_power_on() {
    let rom = rom();
    let idle = vec![InputFrame::default(); 30];
    let known = gp_events::Levels::of(
        &gp_events::run(&rom, PowerOn::default(), &idle, true)
            .unwrap()
            .collector,
        30,
    );
    let start = Start::of(&rom, PowerOn::default(), &idle).unwrap();
    let opts = Opts {
        seed: 5,
        budget: 200,
        min_frames: 10,
        max_frames: 40,
    };
    let mut s = NewCode::new(&known);
    let found = search(&rom, std::slice::from_ref(&start), &mut s, &opts).unwrap();
    assert!(!found.is_empty());
    for f in &found {
        let mut all = idle.clone();
        all.extend(gp_explore::presses::log_frames(&f.pads));
        let whole = gp_events::run(&rom, PowerOn::default(), &all, true).unwrap();
        // Everything the candidate ran from the snapshot runs in the replay.
        for (w, (a, b)) in f
            .levels
            .footprint
            .starts
            .words()
            .iter()
            .zip(whole.collector.footprint.starts.words())
            .enumerate()
        {
            assert_eq!(
                a & !b,
                0,
                "word {w}: ran from the snapshot but not from power-on"
            );
        }
        // And the edges it took are among the replay's, with no return
        // misread as a jump.
        let replay = whole.collector.edges.unwrap().table;
        for r in &f.levels.edges {
            assert!(
                replay.contains_key(&(r.kind, r.from, r.to)),
                "{:?}",
                (r.kind, r.from, r.to)
            );
        }
    }
}
