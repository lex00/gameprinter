//! Tapper against the MAME oracle, when the romset and a capture are on
//! this machine; otherwise each test says so and passes.
//!
//! - The romset: $GP_ARCADE_RBTAPPER, else $GP_ROM_DIR/arcade/rbtapper.zip,
//!   else ~/Downloads/rbtapper.zip (gp_arcade::romset::local_romset).
//! - The captures: tapper-first-a and tapper-deaths-a under
//!   $GP_HOME/work/oracle ($GP_HOME defaults to ~/.gameprinter), played
//!   with scripts/oracle/inputs/tapper-first.input and tapper-deaths.input.
//!   Each one that is missing is skipped. So are tapper-watchdog-b
//!   (tapper-watchdog.input, with its sound.wav), tapper-diagonal-a and
//!   tapper-diagonal-4way-a.
//! - tapper-first-sound: the same log recorded with the AY writes
//!   (`ay.tsv`) and the speakers (`sound.wav`): every frame's AY writes
//!   must be MAME's, and the speakers' samples all but equal.

use std::path::PathBuf;

use gp_arcade::oracle::{Capture, InputLog, compare};
use gp_arcade::{Romset, Tapper};

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn romset() -> Option<Romset> {
    let Some(path) = gp_arcade::romset::local_romset("rbtapper") else {
        eprintln!("skipped: no rbtapper romset");
        return None;
    };
    let set = Romset::load_zip(&path).expect("the rbtapper romset loads");
    assert_eq!(set.name, "rbtapper");
    Some(set)
}

fn capture(name: &str, log: &str) -> Option<(Capture, InputLog)> {
    let gp_home =
        std::env::var_os("GP_HOME").map_or_else(|| home().join(".gameprinter"), PathBuf::from);
    let dir = gp_home.join("work/oracle").join(name);
    if !dir.join("frames.tsv").exists() {
        eprintln!("skipped: no oracle capture at {}", dir.display());
        return None;
    }
    let log = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/oracle/inputs")
        .join(log);
    let log =
        InputLog::parse(&std::fs::read_to_string(log).expect("the input log")).expect("it parses");
    Some((Capture::load(&dir).expect("the capture loads"), log))
}

fn matches_every_frame(rom: &Romset, cap: &Capture, log: &InputLog) {
    let mut t = Tapper::power_on(rom);
    for n in 0..cap.rows.len() {
        if n > 0 {
            t.step(log.controls(n));
        }
        let d = compare(&t, cap, n);
        assert!(
            d.is_empty(),
            "frame {n} differs from MAME:\n{}",
            d.describe(16)
        );
    }
}

#[test]
fn every_frame_of_tapper_first_matches_the_mame_oracle() {
    let (Some(rom), Some((cap, log))) = (romset(), capture("tapper-first-a", "tapper-first.input"))
    else {
        return;
    };
    matches_every_frame(&rom, &cap, &log);
}

#[test]
fn every_frame_of_tapper_deaths_matches_the_mame_oracle() {
    let (Some(rom), Some((cap, log))) =
        (romset(), capture("tapper-deaths-a", "tapper-deaths.input"))
    else {
        return;
    };
    matches_every_frame(&rom, &cap, &log);
}

#[test]
fn snapshots_restore_exactly_and_runs_repeat() {
    let Some(rom) = romset() else {
        return;
    };
    let coin_at = |n: u64| gp_arcade::Controls {
        coin: (300..306).contains(&n),
        ..Default::default()
    };
    let mut a = Tapper::power_on(&rom);
    for _ in 0..320 {
        let n = a.frame() + 1;
        a.step(coin_at(n));
    }
    let snap = a.snapshot();
    let mut b = a.clone();
    for _ in 0..90 {
        let n = a.frame() + 1;
        a.step(coin_at(n));
    }
    let end = a.state_hash();
    let picture = a.picture_hash();

    // The snapshot taken mid-run, put back into a machine that ran on.
    for _ in 0..40 {
        let n = b.frame() + 1;
        b.step(coin_at(n));
    }
    b.restore(&snap).expect("restores");
    for _ in 0..90 {
        let n = b.frame() + 1;
        b.step(coin_at(n));
    }
    assert_eq!(b.state_hash(), end);
    assert_eq!(b.picture_hash(), picture);
    assert_eq!(b.snapshot(), a.snapshot());

    // A fresh machine, run the same way, ends the same.
    let mut c = Tapper::power_on(&rom);
    for _ in 0..410 {
        let n = c.frame() + 1;
        c.step(coin_at(n));
    }
    assert_eq!(c.state_hash(), end);
}

/// The game reads its inputs on most frames once it runs, and the runtime
/// protocol's `frame` event says so.
#[test]
fn the_frame_event_says_when_the_game_read_its_inputs() {
    use gp_runtime::Runtime;
    let Some(rom) = romset() else { return };
    let mut rt = gp_arcade::ArcadeRuntime::load(&rom);
    let ev = rt
        .step(&vec![gp_runtime::InputFrame::default(); 300])
        .unwrap();
    let polled = ev
        .iter()
        .filter(|e| e.data[0].value == gp_runtime::Value::Flag(true))
        .count();
    eprintln!("{polled} of 300 frames polled");
    assert!(polled > 150, "{polled} of 300 frames polled");
}

#[test]
fn tapper_first_sounds_as_mame_does() {
    let (Some(rom), Some((cap, log))) = (
        romset(),
        capture("tapper-first-sound", "tapper-first.input"),
    ) else {
        return;
    };
    assert!(cap.ay.is_some(), "tapper-first-sound has no ay.tsv");
    matches_every_frame(&rom, &cap, &log);
    sounds_as_mame_does(&rom, &cap, &log);
}

/// The speakers' samples against the capture's sound.wav, when it has one.
fn sounds_as_mame_does(rom: &Romset, cap: &Capture, log: &InputLog) {
    let Ok(wav) = std::fs::read(cap.dir.join("sound.wav")) else {
        eprintln!("skipped the samples: no sound.wav");
        return;
    };
    // The data chunk of MAME's 16-bit stereo WAV.
    let at = wav
        .windows(4)
        .position(|w| w == b"data")
        .expect("a data chunk")
        + 8;
    let theirs: Vec<i16> = wav[at..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect();
    let mut t = Tapper::power_on(rom);
    t.set_audio(true);
    let mut ours = Vec::new();
    for n in 1..cap.rows.len() {
        t.step(log.controls(n));
        for s in t.take_audio() {
            ours.push(gp_arcade::sound::to_i16(s[0]));
            ours.push(gp_arcade::sound::to_i16(s[1]));
        }
    }
    assert_eq!(ours.len(), theirs.len());
    let worst = ours
        .iter()
        .zip(&theirs)
        .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
        .max()
        .unwrap_or(0);
    let differ = ours.iter().zip(&theirs).filter(|(a, b)| a != b).count();
    // Float rounding in the mixing leaves a few samples one step apart.
    assert!(worst <= 1, "a sample differs by {worst}");
    assert!(differ * 100_000 < ours.len(), "{differ} samples differ");
}

/// A tilt hangs the game and the watchdog resets the board: run alone,
/// the reset must come, the game must test its RAM and read its inputs
/// again, and a second game must start. With the capture
/// tapper-watchdog-b, every frame must match MAME's, the reset included,
/// and the speakers too.
#[test]
fn the_watchdog_resets_the_board_after_a_tilt() {
    let Some(rom) = romset() else {
        return;
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/oracle/inputs/tapper-watchdog.input");
    let log =
        InputLog::parse(&std::fs::read_to_string(path).expect("the input log")).expect("it parses");
    let mut t = Tapper::power_on(&rom);
    let mut fired_at = None;
    let mut polled_after = 0;
    for n in 1..=log.frames.len() {
        t.step(log.controls(n));
        if fired_at.is_none() && t.watchdog_fired() {
            fired_at = Some(n);
        }
        if fired_at.is_some() && t.polled() {
            polled_after += 1;
        }
        if (680..700).contains(&n) {
            // Snapshots around the reset restore exactly.
            let mut u = t.clone();
            u.restore(&t.snapshot()).expect("it restores");
            assert_eq!(u.snapshot(), t.snapshot(), "frame {n}");
        }
    }
    let fired_at = fired_at.expect("the watchdog ran out");
    eprintln!("the watchdog ran out at frame {fired_at}; {polled_after} frames polled after");
    assert!(polled_after > 300, "the game came back and read its inputs");
    if let Some((cap, log)) = capture("tapper-watchdog-b", "tapper-watchdog.input") {
        matches_every_frame(&rom, &cap, &log);
        sounds_as_mame_does(&rom, &cap, &log);
    }
}

/// Diagonals and opposing directions: as the oracle sets them they reach
/// the board as held (tapper-diagonal-a); through MAME's 4-way filter, as
/// a player's would, one direction is kept (tapper-diagonal-4way-a, which
/// gp_arcade::FourWay must match).
#[test]
fn diagonals_match_the_mame_oracle_with_and_without_the_4way_filter() {
    let Some(rom) = romset() else {
        return;
    };
    if let Some((cap, log)) = capture("tapper-diagonal-a", "tapper-diagonal.input") {
        assert!(!log.four_way);
        matches_every_frame(&rom, &cap, &log);
    }
    if let Some((cap, log)) = capture("tapper-diagonal-4way-a", "tapper-diagonal-4way.input") {
        assert!(log.four_way);
        assert_ne!(log.played, log.frames, "the filter changes some frames");
        matches_every_frame(&rom, &cap, &log);
    }
}
