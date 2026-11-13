//! Runs the open-tier baseline, baselines/gp-testrom/buttons.toml, on our
//! test ROM and checks its per-frame hashes against the committed digest. The
//! baseline is compiled in and the program is `gp_nes::testrom`, so the wasm
//! module needs no files and no preopened directories. `just check-wasm` runs
//! it natively and under wasmtime; both must match the committed digest, so
//! every platform that passes agrees with every other.
//!
//! `gp-wasm-check replay-all STORE ROMS` instead checks every recording in an
//! evidence store the way `gp replay --all` does, finding each ROM by hash
//! under ROMS. Under wasmtime, both directories are preopened
//! (`just check-wasm-roms`), so stored recordings replay on WASM too. It
//! opens the store without creating anything and writes nothing. Exit 0 when
//! everything that ran matched (stale and skipped recordings don't fail), 1
//! on a divergence or when every recording was skipped, 2 on an error (a
//! missing store or ROM folder included).
//!
//! An arcade recording (a gp-arcade run of a romset) is replayed from its
//! romset, found by its identity among the zips under ROMS (src/arcade.rs),
//! so Tapper's recordings replay on WASM too.
//!
//! Model recordings (`gp record --model`) need no ROM: replay-all replays
//! them in the interpreter from the model bundle in the store, so the
//! interpreter's hashes are checked on WASM against the native ones.
//!
//! `gp-wasm-check baseline ROM BASELINE...` checks baselines for a game built
//! from source, such as the reference game (baselines/mosshop/), against
//! that ROM file the same way: each baseline's SHA-256 must be the ROM's,
//! and its digest, lag count and RAM assertions must match. Under wasmtime the ROM's and the
//! baselines' directories are preopened (`just check-wasm`). It prints one
//! `ok` or `FAILED` line per baseline and exits 0 when all pass, 1 when any
//! doesn't match, 2 when one can't be read or run.
//!
//! Output, one `key value` pair per line: `frames`, `digest` (the BLAKE3 hash
//! of the frame hash object, encoded as a recording stores it), `expected`,
//! `lag`, and `fps`. Exit 0 when the digest and lag count match, 1 when they
//! don't, 2 when the run fails.

mod arcade;

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use gp_evidence::Hash;
use gp_evidence::baseline::Baseline;
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::input_log::InputLog;
use gp_evidence::manifest::{PowerOn, SourceRef};
use gp_evidence::{Store, recordings};
use gp_nes::{Nes, recording, testrom};

const BASELINE_PATH: &str = "baselines/gp-testrom/buttons.toml";
const BASELINE: &str = include_str!("../../../baselines/gp-testrom/buttons.toml");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => check_baseline(),
        [cmd, store, roms] if cmd == "replay-all" => replay_all(Path::new(store), Path::new(roms)),
        [cmd, rom, files @ ..] if cmd == "baseline" && !files.is_empty() => {
            check_baselines(Path::new(rom), files)
        }
        _ => {
            eprintln!("usage: gp-wasm-check [replay-all STORE ROMS | baseline ROM BASELINE...]");
            ExitCode::from(2)
        }
    }
}

/// Every recording in the store, checked as `gp replay --all` checks them.
fn replay_all(store_dir: &Path, rom_dir: &Path) -> ExitCode {
    let setup = || -> Result<_, String> {
        let store = Store::open_existing(store_dir).map_err(|e| e.to_string())?;
        let listing = recordings::list_checked(&store).map_err(|e| e.to_string())?;
        for (line, message) in &listing.bad_lines {
            eprintln!("gp-wasm-check: skipping recordings index line {line}: {message}");
        }
        // Retired recordings are left out, as `gp replay --all` leaves them.
        let retired = recordings::retired(&listing.marks);
        let all = recordings::unique(listing.entries);
        let total = all.len();
        let entries: Vec<_> = all
            .into_iter()
            .filter(|e| !retired.contains(&e.id))
            .collect();
        let left_out = total - entries.len();
        // With nothing to replay, no ROM folder is needed.
        let roms = if entries.is_empty() {
            Default::default()
        } else {
            gp_evidence::source::index_nes_roms(rom_dir)
                .map_err(|e| format!("{}: {e}", rom_dir.display()))?
        };
        Ok((store, entries, roms, left_out))
    };
    let (store, entries, roms, left_out) = match setup() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gp-wasm-check: {e}");
            return ExitCode::from(2);
        }
    };
    let (mut ok, mut diverged, mut stale, mut skipped, mut failed) = (0, 0, 0, 0, 0);
    let mut other = 0;
    // The romsets under ROMS by identity, read at the first arcade recording.
    let mut romsets: Option<std::collections::HashMap<String, std::path::PathBuf>> = None;
    for entry in &entries {
        let short = &entry.id.to_hex()[..12];
        let checked = (|| -> Result<Option<Result<recording::Checked, gp_interp::recording::Checked>>, String> {
            let manifest =
                recordings::load_manifest(&store, &entry.id).map_err(|e| e.to_string())?;
            if let SourceRef::Romset { identity, .. } = &manifest.source {
                let sets = romsets.get_or_insert_with(|| arcade::index_romsets(rom_dir));
                let Some(path) = sets.get(identity) else {
                    return Ok(None);
                };
                let set = gp_arcade::Romset::load_zip(path).map_err(|e| e.to_string())?;
                return arcade::check(&store, &manifest, &set).map(|c| Some(Ok(c)));
            }
            let Some(sha256) = manifest.source.nes_sha256() else {
                // A model run replays with the hooks gp has for its model.
                let model = match &manifest.source {
                    SourceRef::Model { name, .. } => name.clone(),
                    SourceRef::NesRom { .. } | SourceRef::Romset { .. } => String::new(),
                };
                let (exts, label) = gp_exts::registry(&model);
                return gp_interp::recording::check_with_hooks(
                    &store,
                    &manifest,
                    std::sync::Arc::new(exts),
                    &label,
                )
                    .map(|c| Some(Err(c)))
                    .map_err(|e| e.to_string());
            };
            let Some(path) = roms.get(sha256) else {
                return Ok(None);
            };
            let rom = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            recording::check(&store, &manifest, &rom)
                .map(|c| Some(Ok(c)))
                .map_err(|e| e.to_string())
        })();
        match checked {
            Ok(None) => {
                skipped += 1;
                println!(
                    "skip     {short}  {}  (no ROM with its hash in {})",
                    entry.name,
                    rom_dir.display()
                );
            }
            Ok(Some(Err(m))) => {
                println!("{}", m.line(short, &entry.name));
                match m.result() {
                    "ok" => ok += 1,
                    "diverged" => diverged += 1,
                    _ => stale += 1,
                }
            }
            Ok(Some(Ok(c))) => {
                println!("{}", c.line(short, &entry.name));
                match c.result() {
                    "ok" => ok += 1,
                    "diverged" => diverged += 1,
                    "other" => other += 1,
                    _ => stale += 1,
                }
            }
            Err(e) => {
                failed += 1;
                println!("ERROR    {short}  {}  {e}", entry.name);
            }
        }
    }
    if entries.is_empty() && left_out > 0 {
        println!("Every recording in the store is retired, so nothing was checked.");
    } else if entries.is_empty() {
        println!("No recordings in the store, so nothing was checked.");
    } else {
        println!(
            "{ok} recordings replayed identically, {diverged} diverged, {stale} stale, {failed} couldn't be checked, {skipped} skipped"
        );
    }
    if other > 0 {
        println!(
            "{other} recordings were made by another emulator (such as Mesen), so they weren't replayed; their traces were checked"
        );
    }
    if left_out > 0 {
        println!("{left_out} retired recordings not replayed");
    }
    // A store whose recordings are all retired checked nothing either.
    let all_retired = entries.is_empty() && left_out > 0;
    let nothing_checked = all_retired || (!entries.is_empty() && skipped == entries.len());
    if nothing_checked && !all_retired {
        println!(
            "Every recording was skipped, so nothing was checked. Is {} the right ROM folder?",
            rom_dir.display()
        );
    }
    if failed > 0 {
        ExitCode::from(2)
    } else if diverged > 0 || nothing_checked {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Baseline files for one ROM file, each checked against its committed
/// digest and lag count.
fn check_baselines(rom_path: &Path, files: &[String]) -> ExitCode {
    let rom = match std::fs::read(rom_path) {
        Ok(rom) => rom,
        Err(e) => {
            eprintln!("gp-wasm-check: {}: {e}", rom_path.display());
            return ExitCode::from(2);
        }
    };
    let sha256 = match gp_evidence::source::identify_nes(&rom) {
        Ok(image) => image.sha256,
        Err(e) => {
            eprintln!("gp-wasm-check: {}: {e}", rom_path.display());
            return ExitCode::from(2);
        }
    };
    let (mut passed, mut failed, mut errors) = (0, 0, 0);
    for file in files {
        let path = Path::new(file);
        let checked = (|| -> Result<Option<String>, String> {
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let baseline = Baseline::parse(&text, path).map_err(|e| e.to_string())?;
            if baseline.sha256 != sha256 {
                return Err(format!(
                    "{} has SHA-256 {sha256}, but this baseline expects {}",
                    rom_path.display(),
                    baseline.sha256
                ));
            }
            let log = baseline.input().map_err(|e| e.to_string())?;
            let reads: Vec<(u64, u16)> =
                baseline.asserts.iter().map(|a| (a.frame, a.addr)).collect();
            let (hashes, lag, values) = run(&rom, &log, &reads)?;
            let digest = Hash::of(&frame_hashes::encode(&hashes));
            let expected = baseline
                .expected_frame_hashes()
                .expect("validated on parse");
            let mut why = Vec::new();
            if digest != expected || lag != baseline.lag_frames {
                why.push(format!(
                    "digest {digest} with {lag} lag frames, expected {expected} with {}",
                    baseline.lag_frames
                ));
            }
            for (a, actual) in baseline.asserts.iter().zip(values) {
                if actual != a.equals {
                    why.push(format!(
                        "{} (${:04X}) at frame {} is {actual}, expected {}",
                        a.label, a.addr, a.frame, a.equals
                    ));
                }
            }
            Ok((!why.is_empty()).then(|| why.join("; ")))
        })();
        match checked {
            Ok(None) => {
                passed += 1;
                println!("ok     {file}");
            }
            Ok(Some(why)) => {
                failed += 1;
                println!("FAILED {file}  {why}");
            }
            Err(e) => {
                errors += 1;
                println!("ERROR  {file}  {e}");
            }
        }
    }
    println!("{passed} baselines passed, {failed} failed, {errors} couldn't be checked");
    if errors > 0 {
        ExitCode::from(2)
    } else if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// The embedded open baseline, checked against its committed digest.
fn check_baseline() -> ExitCode {
    let baseline = match Baseline::parse(BASELINE, Path::new(BASELINE_PATH)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("gp-wasm-check: {e}");
            return ExitCode::from(2);
        }
    };
    let start = Instant::now();
    let log = match baseline.input() {
        Ok(log) => log,
        Err(e) => {
            eprintln!("gp-wasm-check: {e}");
            return ExitCode::from(2);
        }
    };
    match run(&testrom::build(), &log, &[]) {
        Ok((hashes, lag, _)) => {
            let secs = start.elapsed().as_secs_f64();
            let frames = log.frames.len();
            let digest = Hash::of(&frame_hashes::encode(&hashes));
            let expected = baseline
                .expected_frame_hashes()
                .expect("validated on parse");
            println!("frames {frames}");
            println!("digest {digest}");
            println!("expected {expected}");
            println!("lag {lag}");
            // Frames run: the whole log, then its second half again from the checkpoint.
            let run = frames + frames - frames / 2;
            println!("fps {:.0}", run as f64 / secs);
            if digest != expected || lag != baseline.lag_frames {
                eprintln!(
                    "gp-wasm-check: {BASELINE_PATH} expects digest {expected} with {} lag frames",
                    baseline.lag_frames
                );
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gp-wasm-check: {e}");
            ExitCode::from(2)
        }
    }
}

/// Runs the log on `rom` from power-on, then restores the checkpoint taken
/// halfway and runs the rest again, which must give the same hashes. Returns
/// the hashes for frame 0 (power-on) to the last frame, as a recording
/// stores them, the number of lag frames, and the RAM byte at each of
/// `reads` (frame, address), where frame 0 is power-on.
fn run(
    rom: &[u8],
    log: &InputLog,
    reads: &[(u64, u16)],
) -> Result<(Vec<FrameHash>, usize, Vec<u8>), String> {
    let frames = log.frames.len();
    let mut nes = Nes::power_on(rom, PowerOn::default()).map_err(|e| e.to_string())?;
    let hash = |nes: &Nes| -> Result<FrameHash, String> {
        Ok(FrameHash {
            state: nes.state_hash().map_err(|e| e.to_string())?,
            ram: nes.wram_hash(),
        })
    };
    let checkpoint = frames / 2;
    let mut snapshot = None;
    let mut hashes = vec![hash(&nes)?];
    let mut values = vec![0u8; reads.len()];
    let mut read = |frame: u64, nes: &Nes| {
        for (i, (f, addr)) in reads.iter().enumerate() {
            if *f == frame {
                values[i] = nes.wram()[usize::from(*addr)];
            }
        }
    };
    read(0, &nes);
    let mut lag = 0;
    for (f, frame) in log.frames.iter().enumerate() {
        if f == checkpoint {
            snapshot = Some(nes.snapshot().map_err(|e| e.to_string())?);
        }
        if !nes.step(*frame).map_err(|e| e.to_string())?.polled {
            lag += 1;
        }
        read(f as u64 + 1, &nes);
        hashes.push(hash(&nes)?);
    }

    let snapshot = snapshot.ok_or("no checkpoint was taken")?;
    let held = log.frames[checkpoint - 1].pads;
    nes.restore(&snapshot, checkpoint as u64, held)
        .map_err(|e| e.to_string())?;
    for (f, frame) in log.frames.iter().enumerate().skip(checkpoint) {
        nes.step(*frame).map_err(|e| e.to_string())?;
        if hash(&nes)? != hashes[f + 1] {
            return Err(format!(
                "frame {} differs after restoring the frame-{checkpoint} checkpoint",
                f + 1
            ));
        }
    }
    Ok((hashes, lag, values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_evidence::input_log;

    #[test]
    fn same_hashes_twice() {
        let log = input_log::generate(240, &["a@10-40", "right+b@50-"]).unwrap();
        let rom = testrom::build();
        let (a, _, values) = run(&rom, &log, &[(0, 0), (240, 0)]).unwrap();
        assert_eq!(a.len(), 241);
        assert_eq!(a, run(&rom, &log, &[]).unwrap().0);
        // FRAMES counts NMIs: 0 at power-on, more than 200 after 240 frames.
        assert_eq!(values[0], 0);
        assert!(values[1] > 200, "{values:?}");
    }

    #[test]
    fn the_embedded_baseline_parses() {
        let b = Baseline::parse(BASELINE, Path::new(BASELINE_PATH)).unwrap();
        assert_eq!(b.source.as_deref(), Some("gp-testrom"));
    }
}
