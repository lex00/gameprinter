//! Replays an arcade input log on a romset natively and writes each
//! frame's latch writes, as they reached the sound board, in the records
//! gp_sound_frame takes (gp_arcade_play::LATCH_RECORD bytes each): for each
//! frame from frame 1, a u32 little-endian byte count, then the records.
//! `just play-web-arcade` feeds them to gp_arcade_play.wasm's sound board
//! alone and holds its sound to the whole machine's.
//!   cargo run -p gp-arcade-play --example latches ZIP LOG OUT

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [zip, log, out] = args.as_slice() else {
        return Err("usage: latches ZIP LOG OUT".into());
    };
    let zip = std::fs::read(zip).map_err(|e| format!("{zip}: {e}"))?;
    let log = std::fs::read_to_string(log).map_err(|e| format!("{log}: {e}"))?;
    let mut p = gp_arcade_play::Player::replay(&zip, &log)?;
    p.set_latch_log(true);
    let mut bytes = Vec::new();
    loop {
        let start = p.time();
        if !p.advance()? {
            break;
        }
        let r = p.take_latches(start);
        bytes.extend_from_slice(&(r.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&r);
    }
    std::fs::write(out, bytes).map_err(|e| format!("{out}: {e}"))
}
