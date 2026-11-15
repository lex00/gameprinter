//! Hosting your own registry on GitHub, end to end on this machine
//! (docs/host-your-own.md): a local bare repo stands in for the GitHub
//! repo, and made-up files for the theme. `gp registry host` says where the
//! road stands after each step, and `gp registry init`, `publish` and
//! `reindex` commit and push to the stand-in. The registry holds the theme
//! and no game: a page installs games from its own registry and takes only
//! themes from this one.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

struct World {
    d: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        let w = World {
            d: tempfile::tempdir().unwrap(),
        };
        for d in ["home", "roms"] {
            std::fs::create_dir_all(w.path(d)).unwrap();
        }
        w
    }

    fn path(&self, p: &str) -> PathBuf {
        self.d.path().join(p)
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {o:?}");
        String::from_utf8_lossy(&o.stdout).trim().to_owned()
    }

    /// `gp --json ARGS`: exit code, JSON and the text of stderr.
    fn gp(&self, args: &[&str]) -> (i32, Value, String) {
        let o = Command::new(env!("CARGO_BIN_EXE_gp"))
            .arg("--json")
            .args(args)
            .current_dir(self.d.path())
            .env("GP_HOME", self.path("home"))
            .env("GP_ROM_DIR", self.path("roms"))
            .env_remove("GP_STORE")
            .env_remove("GP_LOG")
            .env_remove("GP_RUN_GROUP")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        let json = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
        (
            o.status.code().unwrap(),
            json,
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    }

    fn ok(&self, args: &[&str]) -> Value {
        let (code, json, err) = self.gp(args);
        assert_eq!(code, 0, "gp {args:?}: {json} {err}");
        json
    }

    fn files(&self, dir: &str, files: &[(&str, &str)]) -> String {
        for (p, text) in files {
            let f = self.path(dir).join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        self.path(dir).display().to_string()
    }
}

#[test]
fn a_registry_of_your_own_from_an_empty_repo_to_the_link() {
    let w = World::new();
    let bare = w.path("harbor-registry.git");
    w.git(
        w.d.path(),
        &["init", "-q", "--bare", "-b", "main", "harbor-registry.git"],
    );
    w.git(
        w.d.path(),
        &["clone", "-q", bare.to_str().unwrap(), "acme-registry"],
    );
    let dir = w.path("acme-registry").display().to_string();
    let target = format!("git:{dir}");
    let host = |extra: &[&str]| {
        let mut a = vec![
            "registry",
            "host",
            dir.as_str(),
            "--theme",
            "acme/harbor-lights",
            "--game",
            "test/made-up",
        ];
        a.extend_from_slice(extra);
        w.ok(&a)
    };
    let gh = ["--github", "acme/harbor-registry"];

    // The stand-in's origin isn't a GitHub address: the first step says so.
    let h = host(&[]);
    assert_eq!(h["stage"]["n"], 1, "{h}");
    let says = h["next"]["says"].as_str().unwrap();
    assert!(says.contains("isn't on GitHub"), "{h}");
    let h = host(&gh);
    assert_eq!(h["stage"]["n"], 2, "{h}");
    assert_eq!(h["steps"].as_array().unwrap().len(), 7, "{h}");
    let raw = "https://raw.githubusercontent.com/acme/harbor-registry/main";
    assert_eq!(h["raw"], raw);

    let init = [
        "registry",
        "init",
        &target,
        "--name",
        "harbor-registry",
        "--namespace",
        "acme",
    ];
    w.ok(&init);
    let h = host(&gh);
    assert_eq!(
        h["stage"]["n"], 4,
        "init granted acme, so the theme is next: {h}"
    );
    assert_eq!(h["facts"]["pushed"], true, "{h}");
    // No step copies a game in: the page installs games from its own registry.
    let all = h["steps"].to_string();
    assert!(
        !all.contains("registry copy") && !all.contains("Bring the game"),
        "{h}"
    );

    let theme = w.files("theme-src", &[("theme/look.txt", "made up\n")]);
    w.ok(&[
        "registry",
        "publish",
        &target,
        &theme,
        "--name",
        "acme/harbor-lights",
        "--version",
        "0.1.0",
        "--kind",
        "content",
        "--title",
        "Harbor Lights",
        "--for",
        "test/made-up",
    ]);
    w.ok(&[
        "registry",
        "reindex",
        &target,
        "--keep",
        "acme/harbor-lights@0.1.0",
    ]);
    let h = host(&gh);
    assert_eq!(h["stage"]["n"], 5, "{h}");
    assert_eq!(h["facts"]["pushed"], true, "{h}");
    assert!(
        !w.path("acme-registry/index/test").exists(),
        "the registry holds no game"
    );
    assert_eq!(
        h["link"],
        format!(
            "https://gameprinter.intentius.workers.dev/p?g=test/made-up&c=acme/harbor-lights&r={raw}"
        )
    );
    assert_eq!(h["themes"]["url"], raw, "{h}");
    // The theme list for the game reads as the page reads it, with no game
    // in the registry.
    let check = w.ok(&[
        "registry",
        "check",
        &dir,
        "--for",
        "test/made-up",
        "--theme",
        "acme/harbor-lights",
    ]);
    assert_ne!(check, Value::Null);

    // A commit not pushed yet is said.
    std::fs::write(w.path("acme-registry/README.md"), "made up\n").unwrap();
    w.git(&w.path("acme-registry"), &["add", "README.md"]);
    let commit = [
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-q",
        "-m",
        "a note",
    ];
    w.git(&w.path("acme-registry"), &commit);
    let h = host(&gh);
    assert_eq!(h["facts"]["pushed"], false, "{h}");
    let says = h["steps"][4]["says"].as_str().unwrap();
    assert!(says.contains("push them"), "{h}");
}
