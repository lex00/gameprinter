//! The sound board run alone (gp_arcade::ssio) against the whole machine
//! on Tapper, when the rbtapper romset is on this machine (else skipped):
//! tapper-first's input played on the whole board, and the latch writes
//! that reached its sound board fed to a sound board of its own, must give
//! the same AY writes and the same speaker samples on every frame.

use std::path::PathBuf;

use gp_arcade::oracle::InputLog;
use gp_arcade::ssio::SoundBoard;
use gp_arcade::{Romset, Tapper};

#[test]
fn tappers_sound_board_alone_sounds_as_the_whole_machine() {
    let Some(path) = gp_arcade::romset::local_romset("rbtapper") else {
        eprintln!("skipped: no rbtapper romset");
        return;
    };
    let rom = Romset::load_zip(&path).expect("the rbtapper romset loads");
    let log = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/oracle/inputs/tapper-first.input");
    let log =
        InputLog::parse(&std::fs::read_to_string(log).expect("the input log")).expect("it parses");
    let mut whole = Tapper::power_on(&rom);
    whole.set_audio(true);
    whole.set_latch_log(true);
    let mut alone = SoundBoard::power_on(&rom);
    alone.set_audio(true);
    let (mut latches, mut ay, mut loud) = (0, 0, 0);
    let frames = log.frames.len().min(1500);
    for n in 1..frames {
        whole.step(log.controls(n));
        alone.clear_writes();
        for (at, latch, value) in whole.take_latch_log() {
            alone.latch(at, latch, value);
            latches += 1;
        }
        alone.run_to(whole.time());
        assert_eq!(
            alone.ay_writes(),
            whole.ay_writes(),
            "frame {n}'s AY writes"
        );
        ay += whole.ay_writes().len();
        let (a, w) = (alone.take_audio(), whole.take_audio());
        loud += w.iter().filter(|s| s[0].abs() + s[1].abs() > 0.01).count();
        assert!(a == w, "frame {n}'s speaker samples differ");
    }
    assert!(
        latches > 0 && ay > 0 && loud > 0,
        "{latches} latch writes, {ay} AY writes, {loud} loud samples"
    );
    eprintln!("{frames} frames: {latches} latch writes, {ay} AY writes and every sample the same");
}
