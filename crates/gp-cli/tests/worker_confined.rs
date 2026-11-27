//! `gp worker confined` run for real (issue 48): python3 started through
//! it writes in the worktree it is started in, is refused a write beside
//! that worktree, and reaches no network. A loop worker has no python3 or
//! cp of its own, so this is the wall around the two.
//!
//! Needs sandbox-exec (macOS) or bwrap (Linux); where neither is there
//! the test says so and checks only that gp refuses to run unconfined.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn gp(home: &Path, dir: &Path, argv: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gp"))
        .args(["worker", "confined", "--"])
        .args(argv)
        .current_dir(dir)
        .env("GP_HOME", home)
        .env("GP_NO_HINTS", "1")
        .env_remove("GP_STEP")
        .env_remove("GP_LOOP_WORKTREE")
        .output()
        .unwrap()
}

fn confines_here() -> bool {
    if cfg!(target_os = "macos") {
        return Path::new("/usr/bin/sandbox-exec").is_file();
    }
    cfg!(target_os = "linux")
        && std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("bwrap").is_file()))
}

const WRITE: &str = "import sys; open(sys.argv[1], 'w').write('x')";
const CONNECT: &str =
    "import socket, sys; socket.create_connection(('127.0.0.1', int(sys.argv[1])), 2)";

#[test]
fn a_confined_python_writes_only_in_its_worktree_and_has_no_network() {
    // Under the build's own folder, not the temp folder: the wrapper lets
    // a program write in the temp folder too.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("worker-confined-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    // gp's home stays outside any repo, as gp asks.
    let home_dir = tempfile::tempdir().unwrap();
    let (repo, home) = (root.join("repo"), home_dir.path().join("home"));
    fs::create_dir_all(&repo).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(init.success());
    let (inside, outside) = (repo.join("drawn.txt"), root.join("outside.txt"));
    let s = |p: &Path| p.to_string_lossy().into_owned();

    // Any other program is refused, on every machine.
    let o = gp(&home, &repo, &["sh", "-c", "true"]);
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("python3 or cp only"),
        "{o:?}"
    );

    if !confines_here() {
        eprintln!("no sandbox-exec or bwrap here: the confined run itself is not checked");
        let o = gp(&home, &repo, &["python3", "-c", WRITE, &s(&inside)]);
        assert!(!o.status.success() && !inside.exists(), "{o:?}");
        let _ = fs::remove_dir_all(&root);
        return;
    }

    let o = gp(&home, &repo, &["python3", "-c", WRITE, &s(&inside)]);
    assert!(o.status.success(), "a write in the worktree: {o:?}");
    assert_eq!(fs::read_to_string(&inside).unwrap(), "x");

    let o = gp(&home, &repo, &["python3", "-c", WRITE, &s(&outside)]);
    assert!(!o.status.success(), "a write beside the worktree: {o:?}");
    assert!(!outside.exists(), "nothing was written outside");

    // cp into the worktree goes; out of it doesn't.
    let o = gp(
        &home,
        &repo,
        &["cp", &s(&inside), &s(&repo.join("copy.txt"))],
    );
    assert!(
        o.status.success() && repo.join("copy.txt").is_file(),
        "{o:?}"
    );
    let o = gp(&home, &repo, &["cp", &s(&inside), &s(&outside)]);
    assert!(!o.status.success() && !outside.exists(), "{o:?}");

    // A listener on this machine, which a program outside the wall reaches.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let free = Command::new("python3")
        .args(["-c", CONNECT, &port])
        .output()
        .unwrap();
    assert!(free.status.success(), "the listener answers: {free:?}");
    let o = gp(&home, &repo, &["python3", "-c", CONNECT, &port]);
    assert!(!o.status.success(), "no network inside: {o:?}");

    let _ = fs::remove_dir_all(&root);
}
