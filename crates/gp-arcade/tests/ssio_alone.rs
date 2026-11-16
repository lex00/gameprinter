//! The sound board run alone (gp_arcade::ssio) against the whole machine,
//! on the game $GP_GAME_DIR names when its romset is on this machine
//! (else skipped): the first input log its oracle/mame-captures.toml
//! lists to replay, played on
//! the whole board, and the latch writes that reached its sound board fed
//! to a sound board of its own, must give the same AY writes and the same
//! speaker samples on every frame.

use gp_arcade::oracle::InputLog;
use gp_arcade::ssio::SoundBoard;
use gp_arcade::{Mcr, Romset};

#[test]
fn the_sound_board_alone_sounds_as_the_whole_machine() {
    let log = match gp_arcade::romset::test_input_log() {
        Ok(t) => InputLog::parse(&t).expect("it parses"),
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let rom: Romset = match gp_arcade::romset::test_set(&log.system) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let mut whole = Mcr::power_on(&rom);
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
