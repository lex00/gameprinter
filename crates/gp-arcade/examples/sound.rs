//! Plays a MAME oracle capture's input log in gp-arcade's Tapper with the
//! sound on and compares what reaches the speakers with the capture's
//! `sound.wav` (recorded with `just mame-record ... 1`, docs/oracle-mame.md).
//!
//!   cargo run --release -p gp-arcade --example sound -- \
//!       --rom ~/Documents/roms/arcade/rbtapper.zip \
//!       --capture ~/.gameprinter/work/oracle/tapper-first-sound \
//!       --log scripts/oracle/inputs/tapper-first.input [--wav OUT.wav]
//!
//! It prints, for each speaker, how many 16-bit samples are equal, how many
//! are within 1 and within 16, the largest difference, and the difference's
//! loudness against the sound's (both RMS), and the shift of ours against
//! MAME's that lines them up best. `--wav` writes ours beside it; the sound
//! is the game's, so OUT goes outside any repo.

use std::path::PathBuf;
use std::process::ExitCode;

use gp_arcade::oracle::InputLog;
use gp_arcade::sound::{OUT_RATE, to_i16};
use gp_arcade::{Romset, Tapper};

fn read_wav(path: &std::path::Path) -> Result<Vec<[i16; 2]>, String> {
    let d = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if d.len() < 12 || &d[..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return Err(format!("{}: not a WAV file", path.display()));
    }
    let mut at = 12;
    let mut fmt_ok = false;
    while at + 8 <= d.len() {
        let id = &d[at..at + 4];
        let len = u32::from_le_bytes(d[at + 4..at + 8].try_into().expect("4 bytes")) as usize;
        let body = &d[at + 8..(at + 8 + len).min(d.len())];
        if id == b"fmt " {
            let ch = u16::from_le_bytes([body[2], body[3]]);
            let rate = u32::from_le_bytes(body[4..8].try_into().expect("4 bytes"));
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if ch != 2 || rate != OUT_RATE || bits != 16 {
                return Err(format!(
                    "{}: {ch} channels at {rate} Hz, {bits} bits; expected 2 at {OUT_RATE}, 16",
                    path.display()
                ));
            }
            fmt_ok = true;
        } else if id == b"data" && fmt_ok {
            return Ok(body
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| {
                    [
                        i16::from_le_bytes([c[0], c[1]]),
                        i16::from_le_bytes([c[2], c[3]]),
                    ]
                })
                .collect());
        }
        at += 8 + len + (len & 1);
    }
    Err(format!("{}: no data chunk", path.display()))
}

fn write_wav(path: &std::path::Path, s: &[[i16; 2]]) -> Result<(), String> {
    let data = s.len() * 4;
    let mut out = Vec::with_capacity(44 + data);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&OUT_RATE.to_le_bytes());
    out.extend_from_slice(&(OUT_RATE * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    for p in s {
        out.extend_from_slice(&p[0].to_le_bytes());
        out.extend_from_slice(&p[1].to_le_bytes());
    }
    std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))
}

/// How one speaker's samples compare.
fn report(name: &str, ours: &[i16], theirs: &[i16]) {
    let n = ours.len().min(theirs.len());
    let (mut equal, mut one, mut sixteen, mut max) = (0usize, 0usize, 0usize, 0i32);
    let (mut sig, mut err) = (0f64, 0f64);
    for i in 0..n {
        let d = (i32::from(ours[i]) - i32::from(theirs[i])).abs();
        equal += usize::from(d == 0);
        one += usize::from(d <= 1);
        sixteen += usize::from(d <= 16);
        max = max.max(d);
        sig += f64::from(theirs[i]).powi(2);
        err += f64::from(d).powi(2);
    }
    let pct = |k: usize| 100.0 * k as f64 / n.max(1) as f64;
    let db = if err == 0.0 {
        "silent".to_string()
    } else if sig == 0.0 {
        "MAME's is silent".to_string()
    } else {
        format!("{:.1} dB under the sound", 10.0 * (sig / err).log10())
    };
    // The shift that lines them up best, within 2 ms.
    let mut best = (0i64, f64::MAX);
    for lag in -96i64..=96 {
        let mut e = 0f64;
        let mut k = 0usize;
        for i in (0..n).step_by(7) {
            let j = i as i64 + lag;
            if j < 0 || j as usize >= n {
                continue;
            }
            e += (f64::from(ours[j as usize]) - f64::from(theirs[i])).powi(2);
            k += 1;
        }
        let e = e / k.max(1) as f64;
        if e < best.1 {
            best = (lag, e);
        }
    }
    println!(
        "{name}: {n} samples; equal {:.2}%, within 1 {:.2}%, within 16 {:.2}%; largest difference {max}; difference {db}; best shift {} samples",
        pct(equal),
        pct(one),
        pct(sixteen),
        best.0
    );
}

fn run() -> Result<(), String> {
    let mut rom = None;
    let mut capture = None;
    let mut log = None;
    let mut wav = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--rom" => rom = Some(PathBuf::from(value()?)),
            "--capture" => capture = Some(PathBuf::from(value()?)),
            "--log" => log = Some(PathBuf::from(value()?)),
            "--wav" => wav = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    let (Some(rom), Some(capture), Some(log)) = (rom, capture, log) else {
        return Err("--rom, --capture and --log are needed".into());
    };
    let rom = Romset::load_zip(&rom).map_err(|e| e.to_string())?;
    let log = InputLog::parse(
        &std::fs::read_to_string(&log).map_err(|e| format!("{}: {e}", log.display()))?,
    )?;
    let theirs = read_wav(&capture.join("sound.wav"))?;
    let mut t = Tapper::power_on(&rom);
    t.set_audio(true);
    let mut ours: Vec<[i16; 2]> = Vec::new();
    for n in 1..=log.frames.len() {
        t.step(log.controls(n));
        ours.extend(t.take_audio().iter().map(|s| [to_i16(s[0]), to_i16(s[1])]));
    }
    println!(
        "ours {} samples, MAME's {} ({:.2} s)",
        ours.len(),
        theirs.len(),
        theirs.len() as f64 / f64::from(OUT_RATE)
    );
    let side = |v: &[[i16; 2]], k: usize| v.iter().map(|s| s[k]).collect::<Vec<_>>();
    report("left (AY 0)", &side(&ours, 0), &side(&theirs, 0));
    report("right (AY 1)", &side(&ours, 1), &side(&theirs, 1));
    if let Some(w) = wav {
        write_wav(&w, &ours)?;
        println!("wrote {}", w.display());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sound: {e}");
            ExitCode::from(2)
        }
    }
}
