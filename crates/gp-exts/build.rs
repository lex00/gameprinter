//! Hashes each hook crate's code, so a recording made with a hook names
//! exactly the code that ran: a change to a hook's code changes its hash,
//! the runtime version model recordings store, and so makes them stale
//! (docs/interpreter.md, "Hooks in gp"). What the hash covers is in
//! src/hashing.rs, shared with the tests.

use std::fs;
use std::path::PathBuf;

include!("src/hashing.rs");

fn main() {
    let here = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = here.join("../..");
    let workspace =
        fs::read_to_string(root.join("Cargo.toml")).expect("the workspace's Cargo.toml");
    // Registering a crate edits this crate's Cargo.toml, which reruns the
    // script; the hook crates' own files are watched one by one, so a
    // model.toml edit doesn't rebuild gp.
    println!("cargo:rerun-if-changed=Cargo.toml");
    // A members edit in the workspace, or a new template, can add a hook
    // crate: watch the root manifest and each template directory. Cargo
    // scans a watched directory's files, so editing a template's model.toml
    // reruns this script too; that's the cost of noticing a new ext crate.
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.toml").display()
    );
    println!("cargo:rerun-if-changed=src/hashing.rs");
    let mut rows = Vec::new();
    // The demo hook, in this crate.
    let demo = here.join("src/demo.rs");
    println!("cargo:rerun-if-changed={}", demo.display());
    rows.push(format!(
        "(\"demo\", \"gp-exts demo\", {:?}, {:?})",
        env!("CARGO_PKG_VERSION"),
        hook_hash(&here, std::slice::from_ref(&demo))
    ));
    println!("cargo:rerun-if-env-changed=GP_HOOK_MODELS");
    if std::env::var_os("GP_HOOK_MODELS").is_some() {
        let ext = root.join("crates/gp-ext-game");
        let files = hook_files(&ext);
        for f in &files {
            println!("cargo:rerun-if-changed={}", f.display());
        }
        let text = fs::read_to_string(ext.join("Cargo.toml")).expect("the hook crate's Cargo.toml");
        rows.push(format!(
            "(\"game\", \"game hooks\", {:?}, {:?})",
            package_version(&text, &workspace),
            hook_hash(&ext, &files)
        ));
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("hashes.rs");
    fs::write(
        out,
        format!(
            "/// Each hook crate: its key, where it is, its version and the hash\n/// of its code.\npub const CRATES: &[(&str, &str, &str, &str)] = &[{}];\n",
            rows.join(", ")
        ),
    )
    .unwrap();
}
