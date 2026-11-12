//! The generated Z80 instructions (src/z80/ops/) are what
//! tools/gen_z80.py makes from mame/z80.lst today.

use std::path::Path;
use std::process::Command;

#[test]
fn the_z80_instructions_are_up_to_date() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::env::temp_dir().join(format!("gp-arcade-gen-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let status = Command::new("python3")
        .arg("-I")
        .arg(root.join("tools/gen_z80.py"))
        .arg(root.join("mame/z80.lst"))
        .arg(&out)
        .output()
        .expect("python3 runs");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let committed = root.join("src/z80/ops");
    let mut names: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    let mut have: Vec<_> = std::fs::read_dir(&committed)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    have.sort();
    assert_eq!(names, have, "the generator makes a different set of files");
    for name in names {
        let a = std::fs::read(out.join(&name)).unwrap();
        let b = std::fs::read(committed.join(&name)).unwrap();
        assert!(
            a == b,
            "src/z80/ops/{} is stale: run python3 -I tools/gen_z80.py mame/z80.lst src/z80/ops",
            name.to_string_lossy()
        );
    }
    std::fs::remove_dir_all(&out).ok();
}
