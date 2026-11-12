//! Hashes the sources the original's side of a clip depends on (gp-nes
//! and gp-arcade, and what they build on), so a tape
//! (`src/tape.rs`) is reused only for the same emulator. Sets
//! `GP_ORIGINAL_SOURCES`.

use std::fs;
use std::path::{Path, PathBuf};

const SOURCES: &[&str] = &[
    "../gp-nes",
    "../gp-arcade",
    "../gp-runtime",
    "../gp-evidence",
    "../../third_party/tetanes-core",
    "../../Cargo.lock",
    "src/tape.rs",
    "src/screen.rs",
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
