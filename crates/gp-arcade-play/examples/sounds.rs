//! Replays an arcade input log on a romset natively with the sound on, as
//! the browser build plays it, and prints each frame's sound from frame 1:
//! how many samples (left and right interleaved) and the FNV-1a hash of
//! their little-endian f32 bytes, in hex, one frame a line. `just
//! play-web-arcade` compares them with what gp_arcade_play.wasm made for
//! the same log (scripts/web/check-play-arcade.mjs's fifth argument).
//!   cargo run -p gp-arcade-play --example sounds ZIP LOG

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [zip, log] = args.as_slice() else {
        return Err("usage: sounds ZIP LOG".into());
    };
    let zip = std::fs::read(zip).map_err(|e| format!("{zip}: {e}"))?;
    let log = std::fs::read_to_string(log).map_err(|e| format!("{log}: {e}"))?;
    let mut p = gp_arcade_play::Player::replay(&zip, &log)?;
    p.set_audio(true);
    let mut out = String::new();
    while p.advance()? {
        let bytes: Vec<u8> = p.audio().iter().flat_map(|s| s.to_le_bytes()).collect();
        out.push_str(&format!(
            "{} {:08x}\n",
            p.audio().len(),
            gp_arcade_play::fnv1a(&bytes)
        ));
    }
    print!("{out}");
    Ok(())
}
