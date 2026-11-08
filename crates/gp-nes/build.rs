//! Derives the runtime identity that `runtime_ref` (lib.rs) writes into every
//! recording, from the vendored `tetanes-core`: its upstream version and
//! commit, from the header of third_party/tetanes-core.upstream.sha256, and a
//! BLAKE3 hash of our changes to it, third_party/tetanes-core.patch.
//! `scripts/vendor/tetanes-core.sh --check` (part of `just check`) makes sure
//! those two files describe the vendored tree exactly.

use std::path::Path;

fn main() {
    let third_party = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party");
    let upstream = third_party.join("tetanes-core.upstream.sha256");
    let patch = third_party.join("tetanes-core.patch");
    for path in [&upstream, &patch] {
        println!("cargo::rerun-if-changed={}", path.display());
    }

    // "# tetanes-core 0.17.0, crate SHA-256 <hex>, commit <hex>"
    let text = std::fs::read_to_string(&upstream)
        .unwrap_or_else(|e| panic!("reading {}: {e}", upstream.display()));
    let header = text.lines().next().unwrap_or_default();
    let fields: Vec<&str> = header.split([' ', ',']).filter(|f| !f.is_empty()).collect();
    let (version, commit) = match fields.as_slice() {
        [
            "#",
            "tetanes-core",
            version,
            "crate",
            "SHA-256",
            _,
            "commit",
            commit,
        ] => (*version, *commit),
        _ => panic!(
            "{}: unexpected first line {header:?}; rerun scripts/vendor/tetanes-core.sh",
            upstream.display()
        ),
    };
    let patch_bytes =
        std::fs::read(&patch).unwrap_or_else(|e| panic!("reading {}: {e}", patch.display()));
    let patch_hash = blake3::hash(&patch_bytes).to_hex();

    println!("cargo::rustc-env=GP_NES_TETANES_VERSION={version}");
    println!("cargo::rustc-env=GP_NES_TETANES_COMMIT={commit}");
    println!("cargo::rustc-env=GP_NES_PATCH_BLAKE3={}", &patch_hash[..12]);
}
