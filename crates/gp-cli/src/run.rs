//! Runs of the original: baselines. `gp interp run` and `compare` are in
//! interp.rs.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_evidence::baseline::Baseline;
use gp_evidence::source::identify_nes;
use gp_evidence::{InputLog, Store};
use gp_runtime::InputFrame;
use serde_json::json;

use crate::Outcome;

pub(crate) fn frames(input: &InputLog) -> Vec<InputFrame> {
    input
        .frames
        .iter()
        .map(|f| InputFrame {
            buttons: f.buttons_word(),
            axes: Vec::new(),
            reset: f.reset,
        })
        .collect()
}

/// `gp baseline check`: each baseline recorded into a temporary store and
/// its frame hashes, lag frames and RAM checks held to what it expects. A
/// baseline's ROM is `--rom`, else the built-in test ROM for its own
/// baselines, else found in $GP_ROM_DIR by its hash; one whose ROM isn't
/// there is skipped. With `video`, each runs with the picture drawn too and
/// fails if its hashes change. Ported from game-printer's main.rs.
pub fn baseline_check(
    files: &[PathBuf],
    video: bool,
    rom_file: Option<&Path>,
    roms_dir: &Path,
) -> Result<Outcome> {
    use gp_evidence::baseline;
    let files = if files.is_empty() {
        let cwd = std::env::current_dir()?;
        let dir = baseline::find_dir(&cwd).with_context(|| {
            format!(
                "no baselines/ directory at or above {}; run inside the gameprinter repo or name the files",
                cwd.display()
            )
        })?;
        baseline::list(&dir)?
    } else {
        // A directory stands for every baseline under it.
        let mut out = Vec::new();
        for f in files {
            if f.is_dir() {
                out.extend(baseline::list(f)?);
            } else {
                out.push(f.clone());
            }
        }
        out
    };
    if files.is_empty() {
        bail!("no baselines to check; a baseline is a .toml file under baselines/");
    }
    let testrom_sha256 = identify_nes(&gp_nes::testrom::build())?.sha256;
    // A ROM given by path: read once, and used for every baseline.
    let given = match rom_file {
        Some(path) => {
            let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let sha256 = identify_nes(&bytes)
                .with_context(|| format!("identifying {}", path.display()))?
                .sha256;
            Some((path, bytes, sha256))
        }
        None => None,
    };
    // The ROM folder is read only when a baseline needs it, once.
    let mut found: Option<std::collections::BTreeMap<String, crate::roms::Rom>> = None;
    let wanted: Vec<String> = files
        .iter()
        .filter_map(|f| Baseline::load(f).ok().map(|b| b.sha256))
        .collect();
    let cwd = std::env::current_dir()?;
    let (mut lines, mut results) = (Vec::new(), Vec::new());
    let (mut passed, mut failed, mut skipped, mut errors) = (0u64, 0u64, 0u64, 0u64);
    for file in &files {
        let name = file
            .file_stem()
            .map_or_else(|| "baseline".into(), |s| s.to_string_lossy().into_owned());
        let shown = file
            .strip_prefix(&cwd)
            .unwrap_or(file)
            .display()
            .to_string();
        // One baseline that can't be checked doesn't stop the others.
        let checked = (|| -> Result<Option<(Baseline, gp_nes::baseline::BaselineRun)>> {
            let baseline = Baseline::load(file)?;
            let rom = if let Some((path, bytes, sha256)) = &given {
                // A game built from source changes whenever its source does,
                // so a mismatch is an error, not a missing ROM to skip.
                if baseline.sha256 != *sha256 {
                    bail!(
                        "{} has SHA-256 {sha256}, but this baseline expects {}; if the game's source changed, record the baseline again",
                        path.display(),
                        baseline.sha256
                    );
                }
                bytes.clone()
            } else if baseline.source.as_deref() == Some(gp_nes::testrom::SOURCE_ID) {
                // Found by name, so a test program that changed fails here
                // instead of being skipped as a missing ROM.
                if baseline.sha256 != testrom_sha256 {
                    bail!(
                        "the built-in test ROM's SHA-256 is {testrom_sha256}, but this baseline expects {}; the test program changed, so record the baseline again",
                        baseline.sha256
                    );
                }
                gp_nes::testrom::build()
            } else {
                if found.is_none() {
                    found = Some(crate::roms::find(&wanted, &[], roms_dir)?);
                }
                match found.as_ref().and_then(|f| f.get(&baseline.sha256)) {
                    Some(r) => r.bytes.clone(),
                    None => return Ok(None),
                }
            };
            let tmp = tempfile::tempdir().context("creating a temporary store")?;
            let store = Store::open(tmp.path())?;
            let run = gp_nes::baseline::run(&store, &baseline, &name, &rom, video)?;
            Ok(Some((baseline, run)))
        })();
        match checked {
            Ok(None) => {
                skipped += 1;
                lines.push(format!(
                    "skip   {shown}  (no ROM with its hash in {})",
                    roms_dir.display()
                ));
                results.push(json!({ "file": file, "result": "skip" }));
            }
            Ok(Some((baseline, run))) => {
                let failures = run.failures(&baseline);
                let frames = baseline.frames;
                if failures.is_empty() {
                    passed += 1;
                    lines.push(format!(
                        "ok     {shown}  {frames} frames, {} lag, {} RAM checks{}, at {:.0} frames/s",
                        run.lag_frames,
                        run.asserts.len(),
                        if video {
                            ", same hashes and events with video"
                        } else {
                            ""
                        },
                        run.frames_per_second
                    ));
                } else {
                    failed += 1;
                    lines.push(format!("FAILED {shown}"));
                    lines.extend(failures.iter().map(|f| format!("         {f}")));
                }
                results.push(json!({
                    "file": file,
                    "result": if failures.is_empty() { "ok" } else { "failed" },
                    "frames": frames,
                    "frame_hashes": run.frame_hashes,
                    "video_frame_hashes": run.video_frame_hashes,
                    "lag_frames": run.lag_frames,
                    "fps": run.frames_per_second,
                    "failures": failures,
                }));
            }
            Err(e) => {
                errors += 1;
                lines.push(format!("ERROR  {shown}  {e:#}"));
                results.push(json!({ "file": file, "result": "error", "error": format!("{e:#}") }));
            }
        }
    }
    lines.push(format!(
        "{passed} baselines passed, {failed} failed, {errors} couldn't be checked, {skipped} skipped"
    ));
    let exit = if errors > 0 { 2 } else { u8::from(failed > 0) };
    Ok(Outcome {
        json: json!({ "results": results, "passed": passed, "failed": failed, "errors": errors, "skipped": skipped }),
        text: lines.join("\n"),
        exit,
    })
}
