//! Replays an input log on a ROM natively, as the browser build plays it,
//! and prints the frame hash digest: `just play-web` compares it with the
//! digest gp_play.wasm got for the same presses.
//!   cargo run -p gp-play --example digest ROM LOG

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [rom, log] = args.as_slice() else {
        return Err("usage: digest ROM LOG".into());
    };
    let rom = std::fs::read(rom).map_err(|e| format!("{rom}: {e}"))?;
    let log = std::fs::read_to_string(log).map_err(|e| format!("{log}: {e}"))?;
    let mut p = gp_play::Player::replay(&rom, &log)?;
    p.run_to(u64::MAX)?;
    println!("{}", p.digest());
    Ok(())
}
