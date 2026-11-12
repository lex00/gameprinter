//! Replays an arcade input log on a romset natively, as the browser build
//! plays it, and prints every frame's picture hash, frame 0 first, one in
//! hex a line: `just play-web-arcade` compares them with what
//! gp_arcade_play.wasm got for the same log.
//!   cargo run -p gp-arcade-play --example pictures ZIP LOG

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [zip, log] = args.as_slice() else {
        return Err("usage: pictures ZIP LOG".into());
    };
    let zip = std::fs::read(zip).map_err(|e| format!("{zip}: {e}"))?;
    let log = std::fs::read_to_string(log).map_err(|e| format!("{log}: {e}"))?;
    let mut p = gp_arcade_play::Player::replay(&zip, &log)?;
    p.run_to(u64::MAX)?;
    let mut out = String::new();
    for h in p.pictures() {
        out.push_str(&format!("{h:016x}\n"));
    }
    print!("{out}");
    Ok(())
}
