//! `gp-web-lift ROM TEMPLATE OUT`: a visitor's ROM checked against the
//! template and lifted into what the preview loads (crate docs). Writes
//! `OUT/result.json` either way.
//!
//! Exit codes: 0 lifted, 3 the ROM isn't one the template accepts, 2
//! couldn't run.

use std::path::PathBuf;

use gp_web_lift::{lift_rom, result_json, write};

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [rom, template, out] = args.as_slice() else {
        eprintln!("usage: gp-web-lift ROM TEMPLATE OUT");
        std::process::exit(2);
    };
    let result = std::fs::read(rom)
        .map_err(|e| format!("{}: {e}", rom.display()))
        .and_then(|bytes| lift_rom(&bytes, template, out));
    let (json, code) = result_json(result);
    if let Some(e) = json.get("error").and_then(|e| e.as_str()) {
        eprintln!("gp-web-lift: {e}");
    }
    if let Err(e) = write(&out.join("result.json"), json.to_string().as_bytes()) {
        eprintln!("gp-web-lift: {e}");
        std::process::exit(2);
    }
    std::process::exit(code);
}
