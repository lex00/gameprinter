//! A game's MAME captures against the board, when the game, its romset
//! and the captures are on this machine; otherwise each test says why and
//! passes.
//!
//! - The game: $GP_GAME_DIR, an arcade game's folder in the games
//!   checkout. Its roms.toml lists the sets; its oracle/mame-captures.toml
//!   lists, as `[[replay]]`, the captures to replay and the input log each
//!   was recorded with.
//! - The romset: the set an input log's `system` line names, from
//!   $GP_ARCADE_<SET> or $GP_ROM_DIR/arcade/<set>.zip
//!   (gp_arcade::romset::local_romset).
//! - The captures: $GP_HOME/work/oracle/NAME ($GP_HOME defaults to
//!   ~/.gameprinter). Each one that is missing is skipped. Every frame
//!   must match MAME's; with ay.tsv every frame's AY writes, and with
//!   sound.wav the speakers' samples all but equal. A log whose watchdog
//!   runs out must reset the board, snapshots around the reset must
//!   restore exactly, and the game must come back and read its inputs.

use std::path::PathBuf;

use gp_arcade::oracle::{Capture, InputLog, compare};
use gp_arcade::{Mcr, Romset};

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// The game's captures that are on this machine, each with its input log
/// and its set, loaded. Empty, with a line saying why, when there are none.
fn captures() -> Vec<(String, Romset, Capture, InputLog)> {
    let replays = match gp_arcade::romset::test_replays() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipped: {e}");
            return Vec::new();
        }
    };
    let gp_home =
        std::env::var_os("GP_HOME").map_or_else(|| home().join(".gameprinter"), PathBuf::from);
    let mut out = Vec::new();
    for (name, input) in replays {
        let dir = gp_home.join("work/oracle").join(&name);
        if !dir.join("frames.tsv").exists() {
            eprintln!("skipped: no oracle capture at {}", dir.display());
            continue;
        }
        let log = InputLog::parse(&std::fs::read_to_string(&input).expect("the input log"))
            .expect("it parses");
        let rom = match gp_arcade::romset::test_set(&log.system) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipped {name}: {e}");
                continue;
            }
        };
        out.push((
            name,
            rom,
            Capture::load(&dir).expect("the capture loads"),
            log,
        ));
    }
    out
}

fn romset() -> Option<Romset> {
    match gp_arcade::romset::test_romset() {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("skipped: {e}");
            None
        }
    }
}

fn matches_every_frame(rom: &Romset, cap: &Capture, log: &InputLog) {
    let mut t = Mcr::power_on(rom);
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
fn every_capture_of_the_game_matches_the_mame_oracle() {
    for (name, rom, cap, log) in captures() {
        eprintln!("{name}: {} frames", cap.rows.len());
        if log.four_way {
            assert_ne!(
                log.played, log.frames,
                "{name}: the 4-way filter changes no frame"
            );
        }
        matches_every_frame(&rom, &cap, &log);
        if cap.ay.is_some() {
            sounds_as_mame_does(&rom, &cap, &log);
        }
        watchdog_comes_back(&rom, &log);
    }
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
    let mut a = Mcr::power_on(&rom);
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
    let mut c = Mcr::power_on(&rom);
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
    let mut t = Mcr::power_on(rom);
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

/// When the log's watchdog runs out (a tilt hangs the game), the board
/// resets: snapshots around the reset restore exactly, and the game tests
/// its RAM and reads its inputs again. A log whose watchdog never runs out
/// passes.
fn watchdog_comes_back(rom: &Romset, log: &InputLog) {
    let mut t = Mcr::power_on(rom);
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
        if fired_at.is_some_and(|f| n < f + 20) {
            // Snapshots around the reset restore exactly.
            let mut u = t.clone();
            u.restore(&t.snapshot()).expect("it restores");
            assert_eq!(u.snapshot(), t.snapshot(), "frame {n}");
        }
    }
    if let Some(at) = fired_at {
        eprintln!("the watchdog ran out at frame {at}; {polled_after} frames polled after");
        assert!(polled_after > 300, "the game came back and read its inputs");
    }
}
