//! Tapper against the MAME oracle, when the romset and a capture are on
//! this machine; otherwise each test says so and passes.
//!
//! - The romset: $GP_ARCADE_RBTAPPER, else ~/Downloads/rbtapper.zip.
//! - The captures: tapper-first-a and tapper-deaths-a under
//!   $GP_HOME/work/oracle ($GP_HOME defaults to ~/.gameprinter), played
//!   with scripts/oracle/inputs/tapper-first.input and tapper-deaths.input.
//!   Each one that is missing is skipped.

use std::path::PathBuf;

use gp_arcade::oracle::{Capture, InputLog, compare};
use gp_arcade::{Romset, Tapper};

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn romset() -> Option<Romset> {
    let path = std::env::var_os("GP_ARCADE_RBTAPPER")
        .map_or_else(|| home().join("Downloads/rbtapper.zip"), PathBuf::from);
    if !path.exists() {
        eprintln!("skipped: no rbtapper romset at {}", path.display());
        return None;
    }
    Some(Romset::load_zip(&path).expect("the rbtapper romset loads"))
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
