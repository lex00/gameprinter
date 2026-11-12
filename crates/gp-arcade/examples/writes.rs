//! Which instructions write which RAM: plays an oracle input log and
//! prints every main-CPU write to the addresses asked for, in frames FROM
//! to TO, as `frame pc addr value` in hex, one a line. For reading the
//! game's code: what writes a byte, and when. It reads the board's access
//! log (Tapper::set_accesses, the main CPU's writes of it), which took the
//! place of the write log this example was first written for.
//!
//! ```text
//! cargo run --release -p gp-arcade --example writes -- \
//!     --rom ~/Documents/roms/arcade/tapper.zip \
//!     --log scripts/oracle/inputs/tapper-cycle.input \
//!     --from 1090 --to 1101 --addr e2e6 --addr e6f0-e6ff
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use gp_arcade::mcr::AccessKind;
use gp_arcade::oracle::InputLog;
use gp_arcade::{Romset, Tapper};

fn range(s: &str) -> Result<(u16, u16), String> {
    let hex = |t: &str| u16::from_str_radix(t, 16).map_err(|e| format!("{t}: {e}"));
    match s.split_once('-') {
        Some((a, b)) => Ok((hex(a)?, hex(b)?)),
        None => Ok((hex(s)?, hex(s)?)),
    }
}

fn run() -> Result<(), String> {
    let (mut rom, mut log, mut from, mut to) = (None, None, 0u64, 0u64);
    let mut addrs = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--rom" => rom = Some(PathBuf::from(value()?)),
            "--log" => log = Some(PathBuf::from(value()?)),
            "--from" => from = value()?.parse().map_err(|e| format!("--from: {e}"))?,
            "--to" => to = value()?.parse().map_err(|e| format!("--to: {e}"))?,
            "--addr" => addrs.push(range(&value()?)?),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    let (Some(rom), Some(log)) = (rom, log) else {
        return Err("--rom and --log are needed".into());
    };
    let rom = Romset::load_zip(&rom).map_err(|e| e.to_string())?;
    let log = InputLog::parse(
        &std::fs::read_to_string(&log).map_err(|e| format!("{}: {e}", log.display()))?,
    )?;
    let mut t = Tapper::power_on(&rom);
    for n in 1..=to.min(log.frames.len() as u64) {
        if n == from {
            t.set_accesses(true);
        }
        t.step(log.controls(n as usize));
        for x in t
            .take_accesses()
            .into_iter()
            .filter(|x| x.cpu == 0 && x.kind == AccessKind::Write)
        {
            if addrs.is_empty() || addrs.iter().any(|&(a, b)| (a..=b).contains(&x.addr)) {
                println!("{n} {:04x} {:04x} {:02x}", x.pc, x.addr, x.value);
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("writes: {e}");
            ExitCode::from(2)
        }
    }
}
