//! Hashes the sources the original's side of a clip depends on (gp-nes
//! and gp-arcade, and what they build on), so a tape
//! (`src/tape.rs`) is reused only for the same emulator. Sets
//! `GP_ORIGINAL_SOURCES`. Tapes are shared by every run on the machine
//! (the gate's canary homes too), so the code that starts the original for
//! a clip (scenario.rs, original.rs, clip_score.rs, and clip.rs's start
//! cache) and records its random draws (lockstep.rs, gp-sim's draws.rs)
//! is in the hash as well: a patch to any of them is scored on the
//! emulator, not on tapes an older gp wrote.
//!
//! With the `bundle` feature it also writes the table of files a released
//! gp carries (src/bundle.rs): every file under `$GP_BUNDLE_DIR`, which
//! `just release` fills. Without it the table is empty and nothing more is
//! read, so a dev build costs what it did.

use std::fmt::Write as _;
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

/// `$OUT_DIR/bundle.rs`: `FILES`, each carried file's path under the
/// bundle and its bytes, and `ID`, a hash of them all.
fn bundle() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("bundle.rs");
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    if std::env::var_os("CARGO_FEATURE_BUNDLE").is_none() {
        println!("cargo:rustc-env=GP_VERSION_LINE={version}");
        fs::write(
            &out,
            "pub(crate) static FILES: &[(&str, &[u8])] = &[];\npub(crate) const ID: &str = \"\";\n",
        )
        .expect("writing bundle.rs");
        return;
    }
    println!("cargo:rerun-if-env-changed=GP_BUNDLE_DIR");
    let dir = std::env::var_os("GP_BUNDLE_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .expect("the bundle feature needs GP_BUNDLE_DIR, the folder `just release` fills");
    let dir = dir.canonicalize().expect("GP_BUNDLE_DIR doesn't exist");
    println!("cargo:rerun-if-changed={}", dir.display());
    // Every file, `target/` folders too (the players sit in target/play-web).
    let mut files = Vec::new();
    let mut todo = vec![dir.clone()];
    while let Some(d) = todo.pop() {
        for e in fs::read_dir(&d).expect("reading GP_BUNDLE_DIR").flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut h = blake3::Hasher::new();
    let mut table = String::from("pub(crate) static FILES: &[(&str, &[u8])] = &[\n");
    for f in &files {
        let rel = f
            .strip_prefix(&dir)
            .expect("under the bundle")
            .to_string_lossy()
            .replace('\\', "/");
        h.update(rel.as_bytes());
        h.update(&[0]);
        h.update(&fs::read(f).expect("reading a bundle file"));
        h.update(&[0]);
        writeln!(
            table,
            "    ({rel:?}, include_bytes!({:?})),",
            f.display().to_string()
        )
        .unwrap();
    }
    table.push_str("];\n");
    let id = h.finalize().to_hex()[..16].to_owned();
    writeln!(table, "pub(crate) const ID: &str = {id:?};").unwrap();
    fs::write(&out, table).expect("writing bundle.rs");
    // `gp --version` names the commit a released gp was built from.
    println!("cargo:rerun-if-env-changed=GP_RELEASE_COMMIT");
    let commit = std::env::var("GP_RELEASE_COMMIT").unwrap_or_else(|_| "unknown commit".into());
    println!("cargo:rustc-env=GP_VERSION_LINE={version} (commit {commit}, bundle {id})");
}

fn main() {
    bundle();
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
