//! `gp self check`: what works on this machine, in plain words, before
//! anything fails later (issue #11). Each line is one thing gp reads or
//! runs, whether it is here, what needs it, and how to get it. The end
//! says, for each kind of work (a theme, a new game, a port), whether
//! this machine is ready for it.
//!
//! It only looks: it installs, downloads and changes nothing, except that
//! a released gp unpacks the files it carries into $GP_HOME the first time
//! (src/bundle.rs), as any command that needs them would.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;

/// The Rust release the module kit builds a game's hook code with.
const TOOLCHAIN_FILE: &str = include_str!("../../../rust-toolchain.toml");
/// The target a game's hook code is built for (module.rs).
const MODULE_TARGET: &str = "wasm32-unknown-unknown";

#[derive(Clone, Copy, PartialEq)]
enum Work {
    Theme,
    Game,
    Port,
}

impl Work {
    fn words(self) -> &'static str {
        match self {
            Work::Theme => "Making a theme (gp theme look new, check, play)",
            Work::Game => {
                "Making a game from a template (gp game new, the level commands, gp model check, gp serve)"
            }
            Work::Port => "Porting a game from a ROM (gp game build, clips, recordings)",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Work::Theme => "theme",
            Work::Game => "game",
            Work::Port => "port",
        }
    }
}

struct Item {
    what: &'static str,
    ok: bool,
    /// What was found, or what is missing.
    found: String,
    /// What needs it.
    for_what: &'static str,
    /// How to get it, when it is missing.
    fix: String,
    /// The kinds of work that can't go on without it.
    needed_by: Vec<Work>,
}

/// The file `name` on PATH, with `.exe` on Windows.
fn on_path(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// A program's first line of output for `args`, when it runs and succeeds.
fn first_line(program: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = if out.stdout.is_empty() {
        out.stderr
    } else {
        out.stdout
    };
    String::from_utf8_lossy(&text)
        .lines()
        .next()
        .map(|l| l.trim().to_owned())
}

fn channel() -> String {
    TOOLCHAIN_FILE
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| {
            t.get("toolchain")?
                .get("channel")?
                .as_str()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "stable".into())
}

fn writable(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let probe = dir.join(format!(".gp-self-check-{}", std::process::id()));
    std::fs::write(&probe, b"").map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// The command that fetches a game from the public registry into
/// games/GAME, lifting from the ROMs in `roms`: the newest version the
/// registry's index lists, so it needs no version or id. OWNER/GAME is
/// the game's name in the registry, which the person fills in.
fn fetch_hint(roms: &Path) -> String {
    format!(
        "gp game fetch OWNER/GAME --from https://raw.githubusercontent.com/lex00/gp-registry/main --into games/GAME --rom-dir {}",
        roms.display()
    )
}

/// ROM files in the ROM folder, by kind: NES images and arcade zips.
fn roms_in(dir: &Path) -> (usize, usize) {
    let (mut nes, mut zips) = (0, 0);
    let mut todo = vec![dir.to_owned()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
                continue;
            }
            match p
                .extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("nes") => nes += 1,
                Some("zip") => zips += 1,
                _ => {}
            }
        }
    }
    (nes, zips)
}

fn checks(paths: &Paths) -> Vec<Item> {
    use Work::*;
    let mut items = Vec::new();

    let home = writable(&paths.home);
    items.push(Item {
        what: "gp's home folder",
        ok: home.is_ok(),
        found: match &home {
            Ok(()) => format!("{} (gp can write there)", paths.home.display()),
            Err(e) => format!("{}: gp can't write there ({e})", paths.home.display()),
        },
        for_what: "every command: gp keeps its working files, the unpacked page and the module kit there",
        fix: "set GP_HOME to a folder you own, outside any git repo".into(),
        needed_by: vec![Theme, Game, Port],
    });

    let checkout = crate::repo::find_repo(|d| {
        d.join("mechanic/index.html").is_file() && d.join("Cargo.toml").is_file()
    });
    let (n, bytes) = crate::bundle::count("");
    let page_files = checkout.is_some() || crate::bundle::carried();
    items.push(Item {
        what: "the page's files",
        ok: page_files,
        found: match (&checkout, crate::bundle::carried()) {
            (Some(c), _) => format!("read from the gameprinter checkout {}", c.display()),
            (None, true) => format!(
                "carried in this gp ({n} files, {:.1} MB, bundle {}{}), unpacked under {}",
                bytes as f64 / 1e6,
                crate::bundle::id(),
                if crate::bundle::page().is_some() { ", the public page among them" } else { "" },
                crate::bundle::root()
                    .map(|r| r.display().to_string())
                    .unwrap_or_else(|| "(couldn't unpack)".into())
            ),
            (None, false) => "none: this gp was built without them and there is no gameprinter checkout here".into(),
        },
        for_what: "gp serve and the game's page, gp mcp's templates, gp theme look play's page, the module kit",
        fix: "use a released gp (it carries them), or run gp inside a gameprinter checkout".into(),
        needed_by: vec![Theme, Game, Port],
    });

    let (nes, zips) = roms_in(&paths.roms);
    let rom_ok = nes + zips > 0;
    items.push(Item {
        what: "your ROMs",
        ok: rom_ok,
        found: if paths.roms.is_dir() {
            format!(
                "{}: {nes} NES file(s), {zips} zip file(s)",
                paths.roms.display()
            )
        } else {
            format!("{} doesn't exist", paths.roms.display())
        },
        for_what: "gp theme look check names the game's pictures and checks your files hold nothing of the ROM; gp game new and a port read the ROM; the page plays from it",
        fix: format!(
            "put your own copy of the game in {} (or set GP_ROM_DIR to the folder that holds it); gp finds it by its bytes, whatever it is called",
            paths.roms.display()
        ),
        needed_by: vec![Theme, Game, Port],
    });

    let games = crate::roms_cmd::games_dir(None);
    items.push(Item {
        what: "a games folder",
        ok: games.is_some(),
        found: games.as_ref().map_or_else(
            || "none here (no GP_GAMES_DIR, and no folder of games at or above this one)".into(),
            |g| g.display().to_string(),
        ),
        for_what: "naming a game by its name (--game GAME) and gp serve; without one, give a game's folder, which gp game fetch makes",
        fix: format!(
            "{}, then set GP_GAMES_DIR to that games folder",
            fetch_hint(&paths.roms)
        ),
        needed_by: vec![Game],
    });

    let curl = on_path("curl");
    items.push(Item {
        what: "curl",
        ok: curl.is_some(),
        found: curl.as_ref().map_or_else(|| "not on PATH".into(), |p| p.display().to_string()),
        for_what: "reading the public registry over HTTPS: gp game fetch, gp theme look play, gp theme fetch",
        fix: "install curl (it comes with macOS and Windows 10 and later; apt install curl on Linux)".into(),
        needed_by: vec![Theme, Game, Port],
    });

    let git = on_path("git");
    items.push(Item {
        what: "git",
        ok: git.is_some(),
        found: git
            .as_ref()
            .and_then(|p| first_line(p, &["--version"]))
            .unwrap_or_else(|| "not on PATH".into()),
        for_what: "publishing a theme or a game (gp theme publish, gp registry publish) and recording which version of your files a run used",
        fix: "install git (xcode-select --install on a Mac, apt install git on Linux)".into(),
        needed_by: vec![],
    });

    let keygen = on_path("ssh-keygen");
    items.push(Item {
        what: "ssh-keygen",
        ok: keygen.is_some(),
        found: keygen
            .as_ref()
            .map_or_else(|| "not on PATH".into(), |p| p.display().to_string()),
        for_what: "signing what you publish to a registry",
        fix: "it comes with macOS and with OpenSSH on Linux and Windows".into(),
        needed_by: vec![],
    });

    let ch = channel();
    let rustup = on_path("rustup");
    let (rust_ok, rust_found, rust_fix) = match &rustup {
        None => (
            false,
            "rustup not on PATH".to_owned(),
            format!(
                "install rustup from https://rustup.rs, then: rustup toolchain install {ch} --target {MODULE_TARGET}"
            ),
        ),
        Some(r) => {
            let cargo = first_line(r, &["which", "--toolchain", &ch, "cargo"]);
            let targets = Command::new(r)
                .args(["target", "list", "--installed", "--toolchain", &ch])
                .stdin(Stdio::null())
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            let has_target = targets.lines().any(|l| l.trim() == MODULE_TARGET);
            match (cargo, has_target) {
                (Some(_), true) => (
                    true,
                    format!("Rust {ch} with {MODULE_TARGET}"),
                    String::new(),
                ),
                (Some(_), false) => (
                    false,
                    format!("Rust {ch} without the {MODULE_TARGET} target"),
                    format!("rustup target add {MODULE_TARGET} --toolchain {ch}"),
                ),
                (None, _) => (
                    false,
                    format!("rustup is here, Rust {ch} isn't"),
                    format!("rustup toolchain install {ch} --target {MODULE_TARGET}"),
                ),
            }
        }
    };
    items.push(Item {
        what: "Rust",
        ok: rust_ok,
        found: rust_found,
        for_what: "building a game's own code (its hooks) into its module: gp game build, gp game test, and every command that opens a game with hooks on your machine (clips, scoring, the game's page in gp serve). A theme needs none: the page takes a published game's module from the registry",
        fix: rust_fix,
        needed_by: vec![Port],
    });

    let (sandbox_ok, sandbox_found, sandbox_fix) = if cfg!(target_os = "macos") {
        let p = Path::new("/usr/bin/sandbox-exec");
        (
            p.is_file(),
            if p.is_file() {
                "sandbox-exec (part of macOS)".into()
            } else {
                "no /usr/bin/sandbox-exec".into()
            },
            "it is part of macOS; nothing to install".to_owned(),
        )
    } else if cfg!(target_os = "linux") {
        let b = on_path("bwrap");
        (
            b.is_some(),
            b.map_or_else(|| "bwrap not on PATH".into(), |p| p.display().to_string()),
            "apt install bubblewrap (or your system's package of it)".to_owned(),
        )
    } else {
        (
            false,
            "no build sandbox gp knows on this system".into(),
            "build game code on macOS or Linux; gp game build has no sandbox for Windows yet"
                .to_owned(),
        )
    };
    items.push(Item {
        what: "the build sandbox",
        ok: sandbox_ok,
        found: sandbox_found,
        for_what: "gp game build runs cargo in a sandbox with no network that reads only the game's files",
        fix: sandbox_fix,
        needed_by: vec![Port],
    });

    let node = on_path("node");
    items.push(Item {
        what: "Node",
        ok: node.is_some(),
        found: node
            .as_ref()
            .and_then(|p| first_line(p, &["--version"]))
            .map(|v| format!("node {v}"))
            .unwrap_or_else(|| "not on PATH".into()),
        for_what: "the mechanic demo's decision server and the page's own checks in a checkout. Playing your theme in a browser and gp theme look play --shots don't need it",
        fix: "install Node 22 or later from https://nodejs.org".into(),
        needed_by: vec![],
    });

    let chrome = paths.home.join("work").join("chrome-headless-shell");
    let browser = crate::chrome::installed(&paths.home);
    items.push(Item {
        what: "a headless browser",
        ok: browser.is_some(),
        found: match &browser {
            Some(b) => b.display().to_string(),
            None => format!("none in {}", chrome.display()),
        },
        for_what: "gp theme look play --shots, which drives it itself",
        fix: if crate::chrome::pin().is_some() {
            "nothing to do: gp theme look play --shots downloads Chrome Headless Shell the first time (about 100 MB), or give one with --browser".into()
        } else {
            "gp has no pinned download for this machine: install Chrome Headless Shell and give it to --shots with --browser".into()
        },
        needed_by: vec![],
    });

    let docker = on_path("docker");
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    let image = crate::oracle_mame::image(arch);
    let has_image = docker.as_ref().is_some_and(|d| {
        Command::new(d)
            .args(["image", "inspect", "--format", "{{.Id}}", &image])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    });
    items.push(Item {
        what: "MAME",
        ok: has_image,
        found: match (&docker, has_image) {
            (None, _) => "docker not on PATH".into(),
            (Some(_), false) => format!("docker is here, the image {image} isn't (or docker isn't running)"),
            (Some(_), true) => format!("the image {image}"),
        },
        for_what: "recording an arcade game in MAME to compare the model with (gp oracle record on an arcade recording). NES games and themes don't need it",
        fix: "install Docker, then in a gameprinter checkout: just mame-build (docs/oracle-mame.md)".into(),
        needed_by: vec![],
    });

    items
}

pub(crate) fn run(paths: &Paths) -> Result<Outcome> {
    let items = checks(paths);
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mut text = format!(
        "gp {} at {exe}, built for {}-{}\n\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    for i in &items {
        let _ = writeln!(
            text,
            "{:<7} {}: {}",
            if i.ok { "ok" } else { "missing" },
            i.what,
            i.found
        );
        let _ = writeln!(text, "        why: {}", i.for_what);
        if !i.ok {
            let _ = writeln!(text, "        to get it: {}", i.fix);
        }
    }
    text.push('\n');
    let mut ready = serde_json::Map::new();
    for w in [Work::Theme, Work::Game, Work::Port] {
        let missing: Vec<&str> = items
            .iter()
            .filter(|i| !i.ok && i.needed_by.contains(&w))
            .map(|i| i.what)
            .collect();
        let _ = if missing.is_empty() {
            writeln!(text, "{}: ready", w.words())
        } else {
            writeln!(text, "{}: needs {}", w.words(), missing.join(", "))
        };
        ready.insert(
            w.key().into(),
            json!({"ready": missing.is_empty(), "missing": missing}),
        );
    }
    let list: Vec<Value> = items
        .iter()
        .map(|i| {
            json!({
                "what": i.what, "ok": i.ok, "found": i.found, "for": i.for_what,
                "fix": if i.ok { Value::Null } else { Value::from(i.fix.clone()) },
                "needed_by": i.needed_by.iter().map(|w| w.key()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let home_ok = items.first().is_some_and(|i| i.ok);
    Ok(Outcome {
        json: json!({
            "version": env!("CARGO_PKG_VERSION"),
            "exe": exe,
            "carries": crate::bundle::carried(),
            "bundle": crate::bundle::id(),
            "checks": list,
            "ready": ready,
        }),
        text: text.trim_end().to_owned(),
        exit: if home_ok { 0 } else { 1 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fetch_hint_is_a_command_gp_takes() {
        use clap::Parser as _;
        let hint = fetch_hint(Path::new("/my/roms"));
        let args: Vec<&str> = hint.split(' ').collect();
        assert_eq!(args[..3], ["gp", "game", "fetch"]);
        assert!(hint.ends_with("--rom-dir /my/roms"), "{hint}");
        if let Err(e) = crate::Cli::try_parse_from(&args) {
            panic!("{hint}: {e}");
        }
    }

    #[test]
    fn the_toolchain_is_the_repos() {
        assert_ne!(channel(), "stable");
    }

    #[test]
    fn each_kind_of_work_is_said_ready_or_what_it_needs() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(
            Some(home.path().join("h")),
            None,
            Some(home.path().join("roms")),
        )
        .unwrap();
        let o = run(&paths).unwrap();
        assert_eq!(o.exit, 0, "{}", o.text);
        for k in ["theme", "game", "port"] {
            assert!(o.json["ready"][k]["ready"].is_boolean(), "{k}");
        }
        // No ROM folder: every kind of work says it needs your ROMs.
        assert!(o.text.contains("missing your ROMs"), "{}", o.text);
        assert!(
            o.json["ready"]["theme"]["missing"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m == "your ROMs")
        );
    }
}
