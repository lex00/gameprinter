//! The gp-kit binary as scripts/release/build.sh runs it: `gp-kit SRC OUT`
//! makes the module kit from this checkout, the same kit each time.

use std::path::Path;
use std::process::Command;

#[test]
fn the_binary_makes_the_kit_from_the_checkout_the_same_each_time() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let d = tempfile::tempdir().unwrap();
    let make = |out: &Path| {
        let o = Command::new(env!("CARGO_BIN_EXE_gp-kit"))
            .arg(&src)
            .arg(out)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let line = String::from_utf8(o.stdout).unwrap();
        line.trim().rsplit(' ').next().unwrap().to_owned()
    };
    let (a, b) = (d.path().join("a"), d.path().join("b"));
    let hash = make(&a);
    assert_eq!(hash, gp_kit::tree_hash(&a).unwrap());
    assert_eq!(make(&b), hash, "two kits from one checkout differ");
    for f in [
        "crates/gp-web/Cargo.toml",
        "rust-toolchain.toml",
        ".cargo/config.toml",
    ] {
        assert!(a.join(f).is_file(), "the kit has no {f}");
    }
    assert!(a.join("vendor").is_dir(), "the kit vendored nothing");
    assert!(!a.join("crates/gp-kit").exists() && !a.join("crates/gp-cli").exists());
    let lock = std::fs::read_to_string(a.join("Cargo.lock")).unwrap();
    assert!(
        !lock.contains("name = \"gp-cli\""),
        "the kit's lock keeps gp-cli"
    );
}
