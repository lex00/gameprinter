//! Hashes the sources the original's side of a clip depends on (gp-nes
//! and gp-arcade, and what they build on), so a tape
//! (`src/tape.rs`) is reused only for the same emulator. Sets
//! `GP_ORIGINAL_SOURCES`. Tapes are shared by every run on the machine
//! (the gate's canary homes too), so the code that starts the original for
//! a clip (scenario.rs, original.rs, clip_score.rs, and clip.rs's start
//! cache) and records its random draws (lockstep.rs, gp-sim's draws.rs)
//! is in the hash as well: a patch to any of them is scored on the
//! emulator, not on tapes an older gp wrote.

use std::fs;
use std::path::{Path, PathBuf};

const SOURCES: &[&str] = &[
    "../gp-nes",
    "../gp-arcade",
    "../gp-runtime",
    "../gp-evidence",
    "../../third_party/tetanes-core",
    "../../Cargo.lock",
    "../gp-sim/src/draws.rs",
    "src/tape.rs",
    "src/screen.rs",
    "src/scenario.rs",
    "src/original.rs",
    "src/lockstep.rs",
    "src/clip.rs",
    "src/clip_score.rs",
];

fn walk(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for e in entries {
            // Build output of a vendored crate isn't its source.
            if e.file_name().is_some_and(|n| n != "target") {
                walk(&e, out);
            }
        }
    } else {
        out.push(path.to_owned());
    }
}

fn main() {
    let mut files = Vec::new();
    for s in SOURCES {
        println!("cargo:rerun-if-changed={s}");
        walk(Path::new(s), &mut files);
    }
    let mut h = blake3::Hasher::new();
    for f in &files {
        h.update(f.to_string_lossy().as_bytes());
        h.update(&[0]);
        h.update(&fs::read(f).unwrap_or_default());
        h.update(&[0]);
    }
    println!(
        "cargo:rustc-env=GP_ORIGINAL_SOURCES={}",
        h.finalize().to_hex()
    );
}
