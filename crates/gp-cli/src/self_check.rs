//! `gp self check`: what works on this machine, in plain words, before
//! anything fails later (issue #11). Each line is one thing gp reads or
//! runs, whether it is here, what needs it, and how to get it. The end
//! says, for each kind of work (a theme, a new game, a port), whether
//! this machine is ready for it.
//!
//! Then it uses what a released gp carries (src/bundle.rs), from the
//! bundle alone, with the checkout gp was built from hidden even when it
//! is there (crate::repo::hide_built): it opens the page's files and the
//! templates, makes the module kit as `gp game kit` does, builds a module
//! from it when Rust and the build sandbox are here, and starts the page's
//! server on a free port and asks it for its first page. Each is a line:
//! ready, failed with what it tried, or not checked and why. A failed one
//! exits 1, so the release's own check of its archive fails on a gp that
//! can't do these away from its source (issue 46).
//!
//! A gp built from source carries nothing: it reads the checkout it was
//! built from, so there its check uses that checkout in place of a bundle
//! (the first line says so) and exits 1 by what failed there, or when the
//! checkout is gone. Every gp the tool gate publishes on a developer's
//! machine is such a build. `--as-installed` checks it as a released gp,
//! from its bundle alone, as the release's own check does.
//!
//! It installs, downloads and changes nothing outside $GP_HOME: there it
//! unpacks the carried files the first time (as any command that needs
//! them would), and keeps the kit and the module it built under
//! work/self-check/ and work/module-build/self-check-kit/.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::bundle::Carried;

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
pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
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

/// The Agent line: whether an agent connected to gp mcp in the last week
/// (gp_mcp::clients), which agents this machine has, and the line that
/// connects the first one found (crate::mcp_setup).
fn agent_item(paths: &Paths) -> Item {
    use crate::mcp_setup::{Harness, harnesses, line};
    let now = gp_mcp::asks::now_ms();
    let last = gp_mcp::clients::recent(&paths.home, now, gp_mcp::clients::WEEK_MS);
    let found = crate::mcp_setup::user_home()
        .map(|u| harnesses(&u, &|p: &str| on_path(p).is_some()))
        .unwrap_or_default();
    let here: Vec<String> = found
        .iter()
        .filter(|f| f.here)
        .map(|f| format!("{} ({})", f.harness.words(), f.found))
        .collect();
    let on_machine = if here.is_empty() {
        "no Claude Code, Codex, Cursor, Gemini CLI or VS Code found on this machine".to_owned()
    } else {
        format!("on this machine: {}", here.join("; "))
    };
    let said = match &last {
        Some(c) => format!(
            "{} {} connected {} (gp {}); {on_machine}",
            c["name"].as_str().unwrap_or(""),
            c["version"].as_str().unwrap_or(""),
            gp_mcp::clients::ago(c["at_ms"].as_u64().unwrap_or(0), now),
            c["gp"].as_str().unwrap_or("")
        ),
        None => format!("none connected to gp mcp in the last week; {on_machine}"),
    };
    let first = found.iter().find(|f| f.here);
    let first = first.map_or(Harness::Claude, |f| f.harness);
    Item {
        what: "an agent",
        ok: last.is_some(),
        found: said,
        for_what: "gp is used through your agent: it runs the steps, and gp's tools reach it over gp mcp",
        fix: format!(
            "{}; gp mcp setup prints the line for each agent",
            line(first, &crate::mcp_setup::gp_command())
        ),
        needed_by: vec![Work::Theme, Work::Game, Work::Port],
    }
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

    items.push(agent_item(paths));

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

    let (rust_ok, rust_found, rust_fix) = rust_here();
    items.push(Item {
        what: "Rust",
        ok: rust_ok,
        found: rust_found,
        for_what: "building a game's own code (its hooks) into its module: gp game build, gp game test, and every command that opens a game with hooks on your machine (clips, scoring, the game's page in gp serve). A theme needs none: the page takes a published game's module from the registry",
        fix: rust_fix,
        needed_by: vec![Port],
    });

    let (sandbox_ok, sandbox_found, sandbox_fix) = sandbox_here();
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

/// Why this machine can't build a module from the kit, so self check
/// skips 'a module built from the kit': no Rust with the module's target,
/// or no build sandbox. None when it can.
pub(crate) fn why_no_module_build() -> Option<String> {
    let (rust, rust_found, _) = rust_here();
    let (sandbox, sandbox_found, _) = sandbox_here();
    match (rust, sandbox) {
        (true, true) => None,
        (false, _) => Some(format!("no toolchain ({rust_found})")),
        (true, false) => Some(format!("no build sandbox ({sandbox_found})")),
    }
}

/// Whether Rust is here with the toolchain file's channel and the
/// module's target: whether, what was found, and how to get it.
fn rust_here() -> (bool, String, String) {
    let ch = channel();
    let Some(r) = on_path("rustup") else {
        return (
            false,
            "rustup not on PATH".to_owned(),
            format!(
                "install rustup from https://rustup.rs, then: rustup toolchain install {ch} --target {MODULE_TARGET}"
            ),
        );
    };
    let cargo = first_line(&r, &["which", "--toolchain", &ch, "cargo"]);
    let targets = Command::new(&r)
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

/// Whether gp game build's sandbox is here: whether, what was found, and
/// how to get it.
fn sandbox_here() -> (bool, String, String) {
    if cfg!(target_os = "macos") {
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
    }
}

/// What came of using one thing this gp carries.
#[derive(Clone, Copy, PartialEq)]
enum Did {
    Ready,
    Failed,
    /// Not tried, for want of something on the machine (Rust, the
    /// sandbox) or of an earlier use: never ready.
    NotChecked,
}

impl Did {
    fn word(self) -> &'static str {
        match self {
            Did::Ready => "ready",
            Did::Failed => "failed",
            Did::NotChecked => "skipped",
        }
    }
}

fn did(ok: bool) -> Did {
    if ok { Did::Ready } else { Did::Failed }
}

/// `p` and what went wrong with it.
fn at(p: &Path, e: impl std::fmt::Display) -> String {
    format!("{}: {e:#}", p.display())
}

/// What the uses read: the files a released gp carries, or the checkout
/// a gp built from source reads.
#[derive(Clone, Copy)]
enum Source<'a> {
    Bundle(Carried<'a>),
    Checkout(&'a Path),
}

/// One use of the carried files, from the bundle alone, or of the
/// checkout a gp built from source reads.
struct Use {
    what: &'static str,
    did: Did,
    /// What it made or opened, what failed and the path it tried, or why
    /// it wasn't tried.
    said: String,
    for_what: &'static str,
    needed_by: Vec<Work>,
}

/// The page's files gp reads at run time from a checkout, or from the
/// bundle unpacked: each must be there and open.
const PAGE_FILES: [&str; 6] = [
    "mechanic/screen.html",
    "mechanic/start.html",
    "editor/dist/index.html",
    "scripts/web/page/preview.js",
    "target/play-web/gp_play.wasm",
    "target/play-web/gp_arcade_play.wasm",
];

/// Why a gp built from source has nothing to use.
const NO_FILES: &str = "this gp carries no files (a gp built from source reads its checkout; a released gp carries them)";

/// What a module built from the kit alone is.
const BUILT: &str = "built from the kit with its empty game slot, as for Moss Hop";

/// The carried files unpacked under the home, or why they aren't.
fn unpacked(home: &Path, carried: Carried<'_>) -> Result<PathBuf, String> {
    if carried.files.is_empty() {
        return Err(NO_FILES.to_owned());
    }
    let dir = home.join("work/bundle").join(carried.id);
    carried.unpack(home).map_err(|e| at(&dir, e))
}

/// `dir` again when each of the page's files opens in it.
fn opened(dir: &Path) -> Result<PathBuf, String> {
    for rel in PAGE_FILES {
        let p = dir.join(rel);
        std::fs::read(&p).map_err(|e| at(&p, e))?;
    }
    Ok(dir.to_owned())
}

/// `rel` of the page's files in the checkout `dir`, where gp reads it:
/// mechanic/ and editor/dist/ from the page files published beside this
/// gp when it has them (crate::repo::page_files), the rest from `dir`.
fn checkout_file(dir: &Path, rel: &str) -> PathBuf {
    for top in ["mechanic", "editor/dist"] {
        if let Some(rest) = rel.strip_prefix(top).and_then(|r| r.strip_prefix('/')) {
            return crate::repo::page_files(dir, top).join(rest);
        }
    }
    dir.join(rel)
}

/// The checkout `dir` again when each of the page's files opens where gp
/// reads it.
fn opened_in_checkout(dir: &Path) -> Result<PathBuf, String> {
    for rel in PAGE_FILES {
        let p = checkout_file(dir, rel);
        std::fs::read(&p).map_err(|e| at(&p, e))?;
    }
    Ok(dir.to_owned())
}

/// The checkout `dir` when it is still there, or a line saying it is gone.
fn checkout_root(dir: &Path) -> Result<PathBuf, String> {
    if dir.join("crates/gp-cli/Cargo.toml").is_file() {
        return Ok(dir.to_owned());
    }
    Err(format!(
        "{}: the checkout this gp was built from is gone (a gp built from source reads its files there): build gp again from a checkout, or give --as-installed to check it as a released gp",
        dir.display()
    ))
}

/// What the page's files line says once they opened in `dir`.
fn opened_said(carried: Carried<'_>, dir: &Path) -> String {
    let bytes: usize = carried.files.iter().map(|(_, b)| b.len()).sum();
    let page = if dir.join("page/index.html").is_file() {
        ", the public page among them"
    } else {
        "; no public page, so gp theme look play needs --page"
    };
    format!(
        "opened from the bundle {} ({} files, {:.1} MB{page}), unpacked in {}",
        carried.id,
        carried.files.len(),
        bytes as f64 / 1e6,
        dir.display()
    )
}

/// The templates under `root`'s templates/, each with a template.toml
/// and a model.toml that parse: their names, or what failed. Moss Hop
/// must be among them.
fn templates(root: &Path) -> Result<Vec<String>, String> {
    let dir = root.join("templates");
    let mut names = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| at(&dir, e))?;
    for e in entries.flatten() {
        let t = e.path();
        if !t.is_dir() {
            continue;
        }
        for f in ["template.toml", "model.toml"] {
            let p = t.join(f);
            let text = std::fs::read_to_string(&p).map_err(|e| at(&p, e))?;
            text.parse::<toml::Table>().map_err(|e| at(&p, e))?;
        }
        names.push(e.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    if !names.iter().any(|n| n == "mosshop") {
        return Err(format!("{} has no mosshop/", dir.display()));
    }
    Ok(names)
}

/// A module built from the kit in `kit` as it is.
fn built_module(kit: &Path) -> (Did, String) {
    match crate::module::build_kit_alone(kit) {
        Ok((n, hash)) => (Did::Ready, format!("{BUILT}: {n} bytes, hash {hash}")),
        Err(e) => (Did::Failed, format!("{e:#}")),
    }
}

/// The page's server with the page's files in `dir`, asked for `/`.
fn ask_server(paths: &Paths, dir: &Path, games: &Path) -> Result<String, String> {
    std::fs::create_dir_all(games).map_err(|e| at(games, e))?;
    let first = crate::serve::first_page(paths, dir, games);
    let (port, status, body) = first.map_err(|e| format!("{e:#}"))?;
    let url = format!("http://127.0.0.1:{port}/");
    if status != 200 {
        return Err(format!("{url} answered {status}"));
    }
    if body.contains("isn't here") {
        return Err(format!("{url} answered without its page: {body}"));
    }
    let editor = crate::repo::page_files(dir, "editor/dist");
    let editor = editor.display();
    Ok(format!(
        "answered 200 at {url}, with the editor from {editor}"
    ))
}

/// Each use of what `src` holds, from it alone. `rust`: what is missing
/// of Rust, when something is; `sandbox`: whether the build sandbox is
/// here.
fn uses(paths: &Paths, src: Source<'_>, rust: Option<&str>, sandbox: bool) -> Vec<Use> {
    use Work::*;
    let mut out = Vec::new();
    let (root, from) = match src {
        Source::Bundle(c) => (unpacked(&paths.home, c), "the bundle"),
        Source::Checkout(d) => (checkout_root(d), "the checkout"),
    };
    let page = match src {
        Source::Bundle(_) => root.clone().and_then(|r| opened(&r)),
        Source::Checkout(_) => root.clone().and_then(|r| opened_in_checkout(&r)),
    };
    out.push(Use {
        what: "the page's files",
        did: did(page.is_ok()),
        said: match (&page, src) {
            (Ok(r), Source::Bundle(c)) => opened_said(c, r),
            (Ok(r), Source::Checkout(_)) => format!(
                "{} opened from the checkout {} (mechanic/ from {})",
                PAGE_FILES.len(),
                r.display(),
                crate::repo::page_files(r, "mechanic").display()
            ),
            (Err(e), _) => e.clone(),
        },
        for_what: "gp serve and the game's page, gp theme look play's page, the preview and the players",
        needed_by: vec![Theme, Game, Port],
    });

    let tpl = root.clone().and_then(|r| templates(&r));
    out.push(Use {
        what: "the templates",
        did: did(tpl.is_ok()),
        said: match &tpl {
            Ok(n) => format!("{} read from {from}: {}", n.len(), n.join(", ")),
            Err(e) => e.clone(),
        },
        for_what: "gp mcp's templates and gp game new's copies (Moss Hop and the games made here)",
        needed_by: vec![Game],
    });

    let kit_dir = paths.home.join("work/self-check/kit");
    let kit = match (src, &root) {
        (Source::Bundle(c), _) => crate::module::kit_with(&kit_dir, c),
        (Source::Checkout(_), Ok(r)) => crate::module::kit_from_source(r, &kit_dir),
        (Source::Checkout(_), Err(e)) => Err(anyhow::anyhow!("no checkout to make it from: {e}")),
    };
    out.push(Use {
        what: "the module kit",
        did: did(kit.is_ok()),
        said: match &kit {
            Ok(o) => o.text.clone(),
            Err(e) => format!("{e:#}"),
        },
        for_what: "gp game kit, which gp model gate runs first, and gp game build",
        needed_by: vec![Game, Port],
    });

    let skip = |why: &str| (Did::NotChecked, format!("not checked: {why}"));
    let (built, said) = match (&kit, rust, sandbox) {
        (Err(_), _, _) => skip("no module kit"),
        (_, Some(why), _) => skip(&format!("no toolchain ({why})")),
        (_, _, false) => skip("no build sandbox"),
        (Ok(_), None, true) => built_module(&kit_dir),
    };
    out.push(Use {
        what: "a module built from the kit",
        did: built,
        said,
        for_what: "gp game build: a game's module, its hooks with the interpreter, for the browser",
        needed_by: vec![Port],
    });

    let games = paths.home.join("work/self-check/games");
    let served = root.and_then(|r| ask_server(paths, &r, &games));
    out.push(Use {
        what: "the page's server",
        did: did(served.is_ok()),
        said: served.unwrap_or_else(|e| e),
        for_what: "gp serve, the page you work on",
        needed_by: vec![Theme, Game, Port],
    });
    out
}

/// The checkout a self check uses in place of a bundle: the one this gp
/// was built from, by its build-time path, for a gp that carries nothing,
/// unless `as_installed` (or GP_HIDE_BUILT_SOURCE) says to check it as a
/// released gp. None: the bundle.
fn checkout_to_check(as_installed: bool) -> Option<PathBuf> {
    if as_installed || crate::bundle::carried() {
        return None;
    }
    crate::repo::built()
}

/// `gp self check`: on a gp built from source, against the checkout it
/// was built from; else (or `as_installed`) from the bundle alone, with
/// that checkout hidden for the whole run (crate::repo::hide_built).
/// With `strict`, a skipped use fails it (the release job's check).
pub(crate) fn run(paths: &Paths, as_installed: bool, strict: bool) -> Result<Outcome> {
    let o = if let Some(dir) = checkout_to_check(as_installed) {
        report(paths, Source::Checkout(&dir))?
    } else {
        let _hidden = crate::repo::hide_built();
        report(paths, Source::Bundle(Carried::own()))?
    };
    Ok(if strict { strictly(o) } else { o })
}

/// `o` with each use it skipped counted as failed: a line saying so and
/// exit 1 when there is one.
fn strictly(mut o: Outcome) -> Outcome {
    let skipped: Vec<String> = o.json["skipped"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    o.json["strict"] = true.into();
    if !skipped.is_empty() {
        let _ = write!(
            o.text,
            "\n--strict: a use skipped is a use not checked, so this check fails: {} skipped",
            skipped.join(", ")
        );
        o.exit = 1;
    }
    o
}

fn item<'a>(items: &'a [Item], what: &str) -> Option<&'a Item> {
    items.iter().find(|i| i.what == what)
}

fn report(paths: &Paths, src: Source<'_>) -> Result<Outcome> {
    let items = checks(paths);
    let rust = item(&items, "Rust")
        .filter(|i| !i.ok)
        .map(|i| i.found.as_str());
    let sandbox = item(&items, "the build sandbox").is_some_and(|i| i.ok);
    let tried = uses(paths, src, rust, sandbox);
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mut text = match src {
        Source::Checkout(d) => format!(
            "built from source at {}: checked against the checkout, not a bundle\n",
            d.display()
        ),
        Source::Bundle(_) => String::new(),
    };
    let _ = write!(
        text,
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
    let heading = match src {
        Source::Bundle(_) => {
            "What this gp carries, used from its bundle alone (the checkout it was built from hidden):"
        }
        Source::Checkout(_) => {
            "What this gp reads, used from the checkout it was built from (it carries no bundle):"
        }
    };
    let _ = writeln!(text, "\n{heading}");
    for u in &tried {
        let _ = writeln!(text, "{:<7} {}: {}", u.did.word(), u.what, u.said);
        let _ = writeln!(text, "        why: {}", u.for_what);
    }
    text.push('\n');
    let mut ready = serde_json::Map::new();
    for w in [Work::Theme, Work::Game, Work::Port] {
        let mut missing: Vec<&str> = items
            .iter()
            .filter(|i| !i.ok && i.needed_by.contains(&w))
            .map(|i| i.what)
            .collect();
        for u in &tried {
            if u.did != Did::Ready && u.needed_by.contains(&w) {
                missing.push(u.what);
            }
        }
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
    let tried_list: Vec<Value> = tried
        .iter()
        .map(|u| {
            json!({
                "what": u.what, "did": u.did.word(), "said": u.said, "for": u.for_what,
                "needed_by": u.needed_by.iter().map(|w| w.key()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let home_ok = items.first().is_some_and(|i| i.ok);
    let failed: Vec<&str> = tried
        .iter()
        .filter(|u| u.did == Did::Failed)
        .map(|u| u.what)
        .collect();
    if !failed.is_empty() {
        let _ = write!(
            text,
            "\nThis gp can't do what it {} for: {} failed",
            match src {
                Source::Bundle(_) => "carries files",
                Source::Checkout(_) => "reads its checkout",
            },
            failed.join(", ")
        );
    }
    let skipped: Vec<&str> = tried
        .iter()
        .filter(|u| u.did == Did::NotChecked)
        .map(|u| u.what)
        .collect();
    if !skipped.is_empty() {
        let _ = write!(
            text,
            "\nNot checked on this machine: {} (skipped, see above)",
            skipped.join(", ")
        );
    }
    Ok(Outcome {
        json: json!({
            "version": env!("CARGO_PKG_VERSION"),
            "exe": exe,
            "carries": matches!(src, Source::Bundle(c) if !c.files.is_empty()),
            "bundle": match src {
                Source::Bundle(c) => Value::from(c.id),
                Source::Checkout(_) => Value::Null,
            },
            "checked_against": match src {
                Source::Bundle(_) => "bundle",
                Source::Checkout(_) => "checkout",
            },
            "checkout": match src {
                Source::Checkout(d) => Value::from(d.display().to_string()),
                Source::Bundle(_) => Value::Null,
            },
            "checks": list,
            "uses": tried_list,
            "failed": failed,
            "skipped": skipped,
            "ready": ready,
        }),
        text: text.trim_end().to_owned(),
        exit: if home_ok && failed.is_empty() { 0 } else { 1 },
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
        let o = run(&paths, true, false).unwrap();
        // Checked as installed, a gp built from source carries nothing, so
        // it fails every use of the carried files, though its checkout is
        // right here.
        if !crate::bundle::carried() {
            assert_eq!(o.exit, 1, "{}", o.text);
            let none = "failed  the page's files: this gp carries no files";
            assert!(o.text.contains(none), "{}", o.text);
            let kit = "failed  the module kit: no module kit";
            assert!(o.text.contains(kit), "{}", o.text);
        }
        for k in ["theme", "game", "port"] {
            assert!(o.json["ready"][k]["ready"].is_boolean(), "{k}");
        }
        // No agent connected to this home: it says how to connect one.
        let none = "missing an agent: none connected";
        assert!(o.text.contains(none), "{}", o.text);
        assert!(o.text.contains("gp mcp setup"), "{}", o.text);
        let start = json!({"clientInfo": {"name": "claude-code", "version": "2.1.0"}});
        let now = gp_mcp::asks::now_ms();
        gp_mcp::clients::record(&paths.home, &start, "0.1.2", now).unwrap();
        let o = run(&paths, true, false).unwrap();
        let one = "ok      an agent: claude-code 2.1.0 connected just now (gp 0.1.2)";
        assert!(o.text.contains(one), "{}", o.text);
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

    /// The files a released gp carries, small: the page's files, Moss Hop's
    /// template, and `kit` as the module kit's archive when given.
    fn fake_bundle(kit: Option<&[u8]>) -> Vec<(&'static str, Vec<u8>)> {
        let mut files: Vec<(&'static str, Vec<u8>)> = vec![
            ("templates/mosshop/template.toml", b"x = 1\n".to_vec()),
            ("templates/mosshop/model.toml", b"[model]\n".to_vec()),
        ];
        for rel in PAGE_FILES {
            files.push((rel, b"<!doctype html>\n".to_vec()));
        }
        if let Some(k) = kit {
            files.push((crate::bundle::KIT, k.to_vec()));
        }
        files
    }

    fn kit_zip() -> Vec<u8> {
        use std::io::Write as _;
        type O = zip::write::SimpleFileOptions;
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (rel, text) in [
            ("Cargo.toml", "[workspace]\n"),
            ("crates/gp-web/Cargo.toml", "[package]\n"),
            ("crates/gp-web/src/lib.rs", ""),
        ] {
            z.start_file(rel, O::default()).unwrap();
            z.write_all(text.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    /// Self check on the carried files `files`, the checkout hidden, in
    /// a home of its own.
    fn check_with(files: &[(&'static str, Vec<u8>)], id: &str) -> (Outcome, tempfile::TempDir) {
        let t = tempfile::tempdir().unwrap();
        let (home, roms) = (t.path().join("h"), t.path().join("roms"));
        let paths = Paths::resolve(Some(home), None, Some(roms)).unwrap();
        let list: Vec<(&str, &[u8])> = files.iter().map(|(r, b)| (*r, b.as_slice())).collect();
        let _hidden = crate::repo::hide_built();
        let o = report(&paths, Source::Bundle(Carried { files: &list, id })).unwrap();
        (o, t)
    }

    fn says(o: &Outcome, line: &str) {
        assert!(o.text.contains(line), "{}", o.text);
    }

    /// Issue 46: a released gp whose bundle has no module kit, checked
    /// with the checkout it was built from hidden, exits 1 and names the
    /// kit, though this test runs beside that checkout. The rest of the
    /// bundle is used: its page's files open, its templates read, and the
    /// page's server answers its first page.
    #[test]
    fn a_bundle_without_its_kit_fails_and_names_the_kit() {
        let (o, _t) = check_with(&fake_bundle(None), "test-no-kit");
        assert_eq!(o.exit, 1, "{}", o.text);
        for line in [
            "failed  the module kit: no module kit: this gp carries none (module-kit.zip",
            "the checkout it was built from is hidden",
            "skipped a module built from the kit: not checked: no module kit",
            "ready   the page's files: opened from the bundle test-no-kit",
            "ready   the templates: 1 read from the bundle: mosshop",
            "ready   the page's server: answered 200 at http://127.0.0.1:",
        ] {
            says(&o, line);
        }
        assert_eq!(o.json["failed"], json!(["the module kit"]));
        says(&o, "the module kit failed\n");
        assert_eq!(o.json["skipped"], json!(["a module built from the kit"]));
        assert!(
            o.text.ends_with(
                "Not checked on this machine: a module built from the kit (skipped, see above)"
            ),
            "{}",
            o.text
        );
    }

    /// docs/release.md: the release job's check passes
    /// --strict, so a use skipped there (a module built from the kit, on
    /// a runner without Rust or the sandbox) fails it in place of passing
    /// unchecked; without it a skip alone exits 0.
    #[test]
    fn strict_fails_a_check_that_skipped_a_use() {
        let skipping = || Outcome {
            json: json!({"failed": [], "skipped": ["a module built from the kit"]}),
            text: "Not checked on this machine: a module built from the kit".into(),
            exit: 0,
        };
        let o = strictly(skipping());
        assert_eq!(o.exit, 1, "{}", o.text);
        says(
            &o,
            "--strict: a use skipped is a use not checked, so this check fails: a module built from the kit skipped",
        );
        let clean = strictly(Outcome {
            json: json!({"failed": [], "skipped": []}),
            text: String::new(),
            exit: 0,
        });
        assert_eq!(clean.exit, 0);
        assert_eq!(clean.json["strict"], true);
    }

    /// With the kit in the bundle, `gp game kit`'s path takes it from
    /// there: the checkout is hidden, so nothing is made from source.
    #[test]
    fn a_bundle_with_its_kit_makes_the_kit_from_it() {
        let (o, t) = check_with(&fake_bundle(Some(&kit_zip())), "test-kit");
        says(&o, "ready   the module kit: module kit in ");
        says(&o, "from this gp's bundle test-kit: 1 crates");
        let kit = t.path().join("h/work/self-check/kit");
        assert!(kit.join("crates/gp-web/Cargo.toml").is_file());
    }

    /// A checkout as small as a self check reads: the page's files, Moss
    /// Hop's template, and a kit of one crate with no dependencies.
    fn fake_checkout(dir: &Path) {
        let web = "[package]\nname = \"gp-web\"\nversion = \"0.0.0\"\nedition = \"2024\"\n";
        let lock = "version = 4\n\n[[package]]\nname = \"gp-web\"\nversion = \"0.0.0\"\n";
        let mut files: Vec<(String, String)> = vec![
            ("crates/gp-cli/Cargo.toml".into(), "[package]\n".into()),
            ("templates/mosshop/template.toml".into(), "x = 1\n".into()),
            ("templates/mosshop/model.toml".into(), "[model]\n".into()),
            (
                "Cargo.toml".into(),
                "[workspace]\nresolver = \"2\"\n".into(),
            ),
            ("crates/gp-web/Cargo.toml".into(), web.into()),
            ("crates/gp-web/src/lib.rs".into(), String::new()),
            ("Cargo.lock".into(), lock.into()),
            (
                "rust-toolchain.toml".into(),
                format!("[toolchain]\nchannel = \"{}\"\n", channel()),
            ),
        ];
        for rel in PAGE_FILES {
            files.push((rel.into(), "<!doctype html>\n".into()));
        }
        for (rel, text) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
    }

    /// Self check against the checkout `dir`, in a home of its own.
    fn check_checkout(dir: &Path) -> (Outcome, tempfile::TempDir) {
        let t = tempfile::tempdir().unwrap();
        let (home, roms) = (t.path().join("h"), t.path().join("roms"));
        let paths = Paths::resolve(Some(home), None, Some(roms)).unwrap();
        (report(&paths, Source::Checkout(dir)).unwrap(), t)
    }

    /// A gp built from source is checked against its checkout: the first
    /// line says so, and each use reads the checkout, not a bundle.
    #[test]
    fn a_source_build_is_checked_against_its_checkout() {
        let c = tempfile::tempdir().unwrap();
        fake_checkout(c.path());
        let (o, _t) = check_checkout(c.path());
        let first = format!(
            "built from source at {}: checked against the checkout, not a bundle\n",
            c.path().display()
        );
        assert!(o.text.starts_with(&first), "{}", o.text);
        for line in [
            "What this gp reads, used from the checkout it was built from",
            "ready   the page's files: 6 opened from the checkout ",
            "ready   the templates: 1 read from the checkout: mosshop",
            "ready   the module kit: module kit in ",
            "ready   the page's server: answered 200 at http://127.0.0.1:",
        ] {
            says(&o, line);
        }
        says(&o, &format!("from {}: 1 crates", c.path().display()));
        assert_eq!(o.json["checked_against"], "checkout");
        assert_eq!(o.json["carries"], false);
        // It exits by what failed: only the module build may fail, on a
        // machine with Rust and the sandbox, as this kit is no game's.
        let failed = o.json["failed"].as_array().unwrap();
        let build = "a module built from the kit";
        assert!(failed.iter().all(|f| f == build), "{}", o.text);
        assert_eq!(o.exit, u8::from(!failed.is_empty()), "{}", o.text);
    }

    /// The checkout a source build was built from, gone: each use fails
    /// saying so, and it exits 1.
    #[test]
    fn a_source_build_whose_checkout_is_gone_fails_saying_so() {
        let t = tempfile::tempdir().unwrap();
        let gone = t.path().join("moved-away");
        let (o, _h) = check_checkout(&gone);
        assert_eq!(o.exit, 1, "{}", o.text);
        let is_gone = format!(
            "failed  the page's files: {}: the checkout this gp was built from is gone",
            gone.display()
        );
        says(&o, &is_gone);
        says(&o, "or give --as-installed to check it as a released gp");
        says(&o, "failed  the module kit: no checkout to make it from");
        says(
            &o,
            "skipped a module built from the kit: not checked: no module kit",
        );
        let all = [
            "the page's files",
            "the templates",
            "the module kit",
            "the page's server",
        ];
        assert_eq!(o.json["failed"], json!(all));
    }

    /// Without --as-installed a gp that carries nothing checks the
    /// checkout it was built from; with it, or for a gp that carries its
    /// files, the bundle.
    #[test]
    fn as_installed_checks_the_bundle_in_place_of_the_checkout() {
        assert_eq!(checkout_to_check(true), None);
        if crate::bundle::carried() {
            assert_eq!(checkout_to_check(false), None);
        } else {
            assert!(checkout_to_check(false).is_some());
            assert_eq!(checkout_to_check(false), crate::repo::built());
        }
    }
}
