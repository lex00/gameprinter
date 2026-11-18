//! `gp theme look play`: a theme folder in the game, unpublished.
//!
//! It checks the folder (`gp theme look check`, which holds the ROM-data
//! check), builds a registry in a temp folder (the game and the themes the
//! real registry offers for it, copied by their exact ids, plus the folder
//! published as a version nobody else sees), and serves the built page
//! from it on 127.0.0.1 with the theme chosen. The person loads their own
//! ROM in the page. With `--shots` it drives headless Chrome through the
//! game's route instead and saves the game's canvas at named moments on
//! each ROM of the game that is here; `--against` does the same for a
//! second theme, or for the game in its own pictures (`--against
//! original`), and writes the pairs side by side.
//!
//! This replaces the hand recipe of 2026-11-13 and 14 (copy the registry,
//! publish into the copy, reindex, copy the page with its source set to
//! `.`, serve both, step the route in Chrome, take the pictures).
//!
//! Where the parts come from:
//! - The page: `--page`, else the newest bundle that passed the release
//!   check in `$GP_HOME/work/page-release` (`scripts/web/release-check.sh`),
//!   else one built with `scripts/web/build-page.sh` from the gameprinter
//!   checkout. A gp with no checkout and no bundle can't serve the page.
//! - The game and its themes: `--registry`, else the page's own source
//!   (page.json), read from a checkout of it beside the games checkout when
//!   there is one with the same name, else over HTTPS (blobs cached by hash
//!   in `$GP_HOME/cache/registry-blobs`).
//! - The moments: `--moments`, else the game's `theme-shots.toml`, else the
//!   page check's own route and pictures (the game's `page-check.toml`
//!   `[[route]]` and `page-pictures.txt`).
//! - The browser: Chrome Headless Shell, driven from gp itself
//!   (src/chrome.rs): `--browser`, else the one in
//!   `$GP_HOME/work/chrome-headless-shell`, else the pinned build gp
//!   downloads there the first time. No checkout and no Node.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use gp_evidence::Paths;
use gp_package::registry::{Folder, Publish, Reader, Signer, Target, init, publish, reindex};
use gp_package::{blob_key, package_key, signature_key};
use gp_theme::pix::Rgba;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Outcome;

/// Ports a theme's server never takes: gp serve's, Laya's and the lead's.
const TAKEN_PORTS: [u16; 3] = [7710, 7720, 7796];

/// The moments file a game may hold in its folder.
pub(crate) const MOMENTS_FILE: &str = "theme-shots.toml";

#[derive(clap::Args, Debug)]
pub struct PlayArgs {
    /// The theme's folder (holding package.toml and theme/), or its theme/
    /// folder. Example: ~/my-theme
    pub dir: PathBuf,
    /// The game: its folder, or its name in the games checkout. Example:
    /// --game rooftop
    #[arg(long)]
    pub game: PathBuf,
    /// The port on 127.0.0.1 to serve on; 0 takes any free port. 7710, 7720
    /// and 7796 are never taken. Example: --port 8400
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Open the link in your browser once the page answers. Example: --open
    #[arg(long)]
    pub open: bool,
    /// Go on although the check found problems (warnings never stop it).
    /// Example: --anyway
    #[arg(long)]
    pub anyway: bool,
    /// Instead of serving, play the game's route in headless Chrome on each
    /// ROM of the game that is here and save pictures at its moments in this
    /// folder. They show your theme over the ROM's game: never commit them.
    /// Example: --shots ~/theme-shots
    #[arg(long, value_name = "DIR")]
    pub shots: Option<PathBuf>,
    /// Your ROM, for the check and the pictures (default: each one under
    /// $GP_ROM_DIR the game takes). Example: --rom ~/roms/rooftop.nes
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// A second theme to set beside yours: a folder, a published
    /// NAME@VERSION, or `original`, the game in its own pictures (a folder
    /// named original is ./original). Its pictures are taken too, and each
    /// pair is written side by side (yours on the left). Example: --against
    /// friend/night@0.9.2, or --against original
    #[arg(long, value_name = "DIR_OR_NAME@VERSION_OR_original")]
    pub against: Option<String>,
    /// The moments to picture, a TOML file of [[run]] and [[shot]] tables
    /// (default: the game's theme-shots.toml, else the page check's route
    /// in its page-check.toml and page-pictures.txt). Example: --moments shots.toml
    #[arg(long)]
    pub moments: Option<PathBuf>,
    /// The built page to serve, a folder holding index.html and page.json
    /// (default: the newest that passed the release check in
    /// $GP_HOME/work/page-release, else one built from the gameprinter
    /// checkout). Example: --page target/page-live
    #[arg(long)]
    pub page: Option<PathBuf>,
    /// The registry the game and its published themes are read from: a
    /// folder or an https address (default: the page's own source, from a
    /// checkout of it beside the games checkout when there is one). Nothing
    /// is written to it. Example: --registry ~/gp-registry
    #[arg(long)]
    pub registry: Option<String>,
    /// Chrome Headless Shell for --shots (default: the one in
    /// $GP_HOME/work/chrome-headless-shell, downloaded there the first
    /// time, about 100 MB). Example:
    /// --browser /opt/chrome-headless-shell
    #[arg(long)]
    pub browser: Option<PathBuf>,
    /// Serve from a process of its own and return at once with the link and
    /// its PID; it stops by itself after --minutes (default 60, 480 with
    /// --dev). The way to
    /// serve from an agent's tool. Example: --background
    #[arg(long)]
    pub background: bool,
    /// Stop serving after this many minutes (default: at ctrl-C, or 60 with
    /// --background, 480 with --background --dev). Example: --minutes 30
    #[arg(long)]
    pub minutes: Option<u64>,
    /// Play your working copies as they are, for a dev loop: the page and
    /// the game built now, from the gameprinter checkout this gp runs from
    /// and from the game's folder, never a released page or a published
    /// version of the game (and so not held to the publish's checks).
    /// Example: --dev
    #[arg(long)]
    pub dev: bool,
    /// Build all a server serves and write its serve.json into this
    /// folder, without serving: a --dev server rebuilding itself.
    #[arg(long, hide = true)]
    pub prepare_into: Option<PathBuf>,
    /// The folder a --background server serves (gp passes it to itself).
    #[arg(long, hide = true)]
    pub serve_prepared: Option<PathBuf>,
}

// ---------------------------------------------------------------------
// The theme and the game.

/// A theme folder's package.toml.
#[derive(Debug, Clone)]
struct ThemePkg {
    root: PathBuf,
    /// The name it is served under: package.toml's, made one a registry
    /// takes (`renamed` keeps package.toml's when it differs).
    name: String,
    renamed: Option<String>,
    version: String,
    title: String,
    /// look.txt's `id`: the look the page draws.
    look: Option<String>,
}

/// A package name as a registry takes it (NAMESPACE/NAME in lowercase
/// letters, digits, - and .): `gp theme look new` also takes _ and upper
/// case, which become - and lower case; a name with no namespace is put in
/// `local/`.
pub(crate) fn registry_name(name: &str) -> String {
    let fix = |s: &str| -> String {
        s.chars()
            .map(|c| match c.to_ascii_lowercase() {
                c @ ('a'..='z' | '0'..='9' | '-' | '.') => c,
                _ => '-',
            })
            .collect()
    };
    match name.split_once('/') {
        Some((ns, n)) => format!("{}/{}", fix(ns), fix(&n.replace('/', "-"))),
        None => format!("local/{}", fix(name)),
    }
}

fn theme_root(dir: &Path) -> PathBuf {
    if !dir.join("package.toml").is_file()
        && let Some(p) = dir.parent()
        && p.join("package.toml").is_file()
    {
        return p.to_owned();
    }
    dir.to_owned()
}

fn theme_pkg(dir: &Path) -> Result<ThemePkg> {
    let root = theme_root(dir);
    let file = root.join("package.toml");
    let t: toml::Table = fs::read_to_string(&file)
        .with_context(|| format!("reading {}", file.display()))?
        .parse()
        .with_context(|| format!("parsing {}", file.display()))?;
    let field = |k: &str| {
        t.get("package")
            .and_then(|p| p.get(k))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("{} has no [package] {k}", file.display()))
    };
    let declared = field("name")?;
    let name = registry_name(&declared);
    let look = crate::journeys::look_dir(&root)
        .and_then(|d| fs::read_to_string(d.join(gp_present::looks::LOOK_FILE)).ok())
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("id ").map(|x| x.trim().to_owned()))
        })
        .filter(|i| !i.is_empty());
    Ok(ThemePkg {
        renamed: (name != declared).then_some(declared),
        name,
        version: field("version")?,
        title: field("title")?,
        look,
        root,
    })
}

/// One side of a comparison: the folder, a theme already published, or
/// the game in its own pictures (`--against original`).
#[derive(Debug, Clone)]
enum Side {
    Folder(ThemePkg),
    Published { name: String, version: String },
    Original,
}

impl Side {
    fn name(&self) -> &str {
        match self {
            Side::Folder(p) => &p.name,
            Side::Published { name, .. } => name,
            Side::Original => gp_present::looks::ORIGINAL,
        }
    }
    fn version(&self) -> &str {
        match self {
            Side::Folder(p) => &p.version,
            Side::Published { version, .. } => version,
            Side::Original => "",
        }
    }
    /// The page's id for the theme: the package name's last part.
    fn look_id(&self) -> &str {
        match self {
            Side::Folder(ThemePkg { look: Some(l), .. }) => l,
            Side::Original => gp_present::looks::ORIGINAL,
            _ => self.name().rsplit('/').next().unwrap_or(""),
        }
    }
    fn tag(&self) -> String {
        match self {
            Side::Original => gp_present::looks::ORIGINAL.to_owned(),
            _ => format!("{}-{}", self.look_id(), self.version()),
        }
    }
    /// The page link's theme part: none for the game's own pictures.
    fn content_param(&self) -> String {
        match self {
            Side::Original => String::new(),
            _ => format!("&c={}", self.name()),
        }
    }
    fn words(&self) -> String {
        match self {
            Side::Folder(p) => format!(
                "{} {} from {} (not published)",
                p.title,
                p.version,
                p.root.display()
            ),
            Side::Published { name, version } => format!("{name}@{version} as published"),
            Side::Original => "the game in its own pictures (the original)".to_owned(),
        }
    }
}

fn against_side(arg: &str) -> Result<Side> {
    // The word, not a folder of that name (./original names one).
    if arg == gp_present::looks::ORIGINAL {
        return Ok(Side::Original);
    }
    let p = Path::new(arg);
    if p.is_dir() {
        return Ok(Side::Folder(theme_pkg(p)?));
    }
    match arg.split_once('@') {
        Some((n, v)) if n.contains('/') && !v.is_empty() => Ok(Side::Published {
            name: n.to_owned(),
            version: v.to_owned(),
        }),
        _ => bail!(
            "--against {arg}: not a folder, not a published NAME@VERSION, and not the word original"
        ),
    }
}

// ---------------------------------------------------------------------
// The page.

#[derive(Debug, Clone, Deserialize)]
struct Source {
    name: String,
    url: String,
}

fn page_json(page: &Path) -> Result<Value> {
    let f = page.join("page.json");
    serde_json::from_str(
        &fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?,
    )
    .with_context(|| format!("parsing {}", f.display()))
}

/// The newest page bundle that passed the release check, for this game.
fn passed_bundle(home: &Path, game: &str) -> Option<PathBuf> {
    let dir = home.join("work").join("page-release").join("passed");
    let mut best: Option<(String, PathBuf)> = None;
    for e in fs::read_dir(&dir).ok()?.flatten() {
        let Ok(text) = fs::read_to_string(e.path()) else {
            continue;
        };
        let get = |k: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix(' ')))
                .map(str::trim)
                .map(str::to_owned)
        };
        let (Some(bundle), Some(when)) = (get("bundle"), get("when")) else {
            continue;
        };
        let bundle = PathBuf::from(bundle);
        let ok = bundle.join("index.html").is_file()
            && page_json(&bundle).is_ok_and(|v| v["game"].as_str() == Some(game));
        if ok && best.as_ref().is_none_or(|(w, _)| when > *w) {
            best = Some((when, bundle));
        }
    }
    best.map(|(_, b)| b)
}

/// The gameprinter checkout, when gp can find one.
fn checkout() -> Option<PathBuf> {
    crate::repo::find_repo(|d| d.join("scripts/web/build-page.sh").is_file())
}

/// How the browser plays a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Road {
    /// Lifted from the person's own ROM in the page, the game and its
    /// themes installed from a registry: the page that plays any game by
    /// its name (`/p?g=NS/NAME`, docs/web.md). A ported game.
    Rom,
    /// From the game's own files, with no ROM to load: the interpreter
    /// preview `build_publish` builds (scripts/web/build-preview.sh). A
    /// template of the repo's and a copy of one.
    Own,
}

/// A game's template facts that say how it plays.
#[derive(Debug, Clone)]
struct GameFacts {
    road: Road,
    /// template.toml's `template.id`.
    id: String,
    title: String,
}

/// How a game plays in the browser, or why it can't yet, in one sentence.
fn game_facts(dir: &Path) -> Result<GameFacts> {
    let file = dir.join("template.toml");
    let t: toml::Table = fs::read_to_string(&file)
        .with_context(|| format!("reading {}", file.display()))?
        .parse()
        .with_context(|| format!("parsing {}", file.display()))?;
    let get = |table: &str, key: &str| {
        t.get(table)
            .and_then(|x| x.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let id = get("template", "id").unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let title = get("template", "title").unwrap_or_else(|| id.clone());
    // An arcade game plays on the any-game page from the player's romset
    // zip, its theme drawn over the board's pictures by gp_board.wasm
    // (docs/formats/board-art.md, "Themes"): the ROM road, as on the NES.
    let road = match get("source", "how").as_deref() {
        Some("rom") => Road::Rom,
        Some("build" | "authored") => Road::Own,
        _ => bail!(
            "{title} doesn't say where its pictures come from (from a ROM, or its own), so it can't be built for the browser"
        ),
    };
    Ok(GameFacts { road, id, title })
}

/// A game package's `[package] title`.
fn package_title(dir: &Path) -> Option<String> {
    let t: toml::Table = fs::read_to_string(dir.join("package.toml"))
        .ok()?
        .parse()
        .ok()?;
    Some(t.get("package")?.get("title")?.as_str()?.to_owned())
}

/// Whether a game plays in the browser from its own files, with no ROM
/// to load (a template of the repo's, or a copy of one).
pub(crate) fn plays_without_rom(dir: &Path) -> bool {
    game_facts(dir).is_ok_and(|f| f.road == Road::Own)
}

/// The built page to serve and, when it was built here, the game's play
/// files beside it (build-page.sh's OUT.play).
struct PageBuilt {
    page: PathBuf,
    play: Option<PathBuf>,
}

/// The built page to serve: given, a passed release bundle, or one built
/// now from the checkout.
fn find_page(
    given: Option<&Path>,
    game_dir: &Path,
    game: &str,
    paths: &Paths,
) -> Result<PageBuilt> {
    if let Some(p) = given {
        if !p.join("index.html").is_file() {
            bail!(
                "--page {}: no index.html there, so it isn't a built page",
                p.display()
            );
        }
        return Ok(PageBuilt {
            page: p.to_owned(),
            play: None,
        });
    }
    if let Some(b) = passed_bundle(&paths.home, game) {
        return Ok(PageBuilt {
            page: b,
            play: None,
        });
    }
    let Some(repo) = checkout() else {
        // A released gp carries the public page (src/bundle.rs).
        if let Some(p) = crate::bundle::page() {
            return Ok(PageBuilt {
                page: p,
                play: None,
            });
        }
        bail!(
            "no built page for {game}: this gp has no gameprinter checkout to build one from, and $GP_HOME/work/page-release holds no bundle that passed the release check. Give one with --page DIR (a folder holding the page's index.html and page.json)"
        );
    };
    build_page(&repo, game_dir, game, paths)
}

/// The page and the game's play files, built now with
/// scripts/web/build-page.sh into the home's work folder, a folder for
/// each game.
fn build_page(repo: &Path, game_dir: &Path, game: &str, paths: &Paths) -> Result<PageBuilt> {
    let out = paths
        .home
        .join("work")
        .join("theme-play")
        .join(game.replace('/', "-"))
        .join("page");
    eprintln!(
        "building the page from {} into {} (scripts/web/build-page.sh; a few minutes)",
        repo.display(),
        out.display()
    );
    if let Some(p) = out.parent() {
        fs::create_dir_all(p)?;
    }
    let exe = std::env::current_exe().context("finding gp")?;
    let o = Command::new("bash")
        .arg(repo.join("scripts/web/build-page.sh"))
        .arg(game_dir)
        .arg(&out)
        .env("GP", exe)
        .current_dir(repo)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("running build-page.sh")?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        bail!(
            "scripts/web/build-page.sh exited {}: no page to serve ({})",
            o.status.code().unwrap_or(-1),
            err.lines()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }
    let mut play = out.clone().into_os_string();
    play.push(".play");
    Ok(PageBuilt {
        page: out,
        play: Some(PathBuf::from(play)),
    })
}

// ---------------------------------------------------------------------
// The scratch registry.

/// A registry over HTTPS whose blobs are kept by hash in a folder, so a
/// second run doesn't fetch them again. Every blob is checked against its
/// hash when it is read into a package, so a bad one fails loudly.
struct Cached {
    inner: crate::registry::Http,
    dir: PathBuf,
}

impl Target for Cached {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let Some(hash) = key.strip_prefix("blobs/") else {
            return self.inner.read(key);
        };
        let keep = self.dir.join(hash);
        if let Ok(b) = fs::read(&keep) {
            return Ok(Some(b));
        }
        let got = self.inner.read(key)?;
        if let Some(b) = &got {
            let _ = fs::create_dir_all(&self.dir);
            let _ = fs::write(&keep, b);
        }
        Ok(got)
    }
    fn write_once(&self, key: &str, _: &[u8]) -> Result<bool, String> {
        Err(format!("{key}: the source registry is only read"))
    }
    fn replace(&self, key: &str, _: &[u8]) -> Result<(), String> {
        Err(format!("{key}: the source registry is only read"))
    }
}

/// Where the game and its published themes are read from, and how it was
/// found.
fn source_registry(
    given: Option<&str>,
    page_source: &Source,
    game_dir: &Path,
    paths: &Paths,
) -> Result<(Box<dyn Target>, String)> {
    let cache = paths.home.join("cache").join("registry-blobs");
    let open = |spec: &str| -> Box<dyn Target> {
        if spec.starts_with("https://") || spec.starts_with("http://") {
            Box::new(Cached {
                inner: crate::registry::Http(spec.to_owned()),
                dir: cache.clone(),
            })
        } else {
            Box::new(Folder(PathBuf::from(spec)))
        }
    };
    if let Some(s) = given {
        return Ok((open(s), s.to_owned()));
    }
    // A checkout of the page's source beside the games checkout.
    let near = game_dir
        .parent()
        .and_then(Path::parent)
        .map(|p| p.join(&page_source.name));
    if let Some(n) = near
        && let Ok(t) = fs::read_to_string(n.join("registry.toml"))
        && t.parse::<toml::Table>().ok().and_then(|v| {
            v.get("name")
                .and_then(|x| x.as_str())
                .map(|x| x == page_source.name)
        }) == Some(true)
    {
        let s = n.display().to_string();
        return Ok((open(&s), s));
    }
    if page_source.url == "." {
        bail!(
            "the page's source is its own address, so there is no registry to read the game from: give one with --registry"
        );
    }
    Ok((open(&page_source.url), page_source.url.clone()))
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let nums = |s: &str| -> Vec<u64> { s.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    nums(a).cmp(&nums(b)).then_with(|| a.cmp(b))
}

fn version_key(name: &str, version: &str) -> String {
    format!("index/{name}/{version}.toml")
}

/// Copies one published version, by its exact id, every hash checked.
fn copy_version(reader: &Reader, to: &dyn Target, name: &str, version: &str) -> Result<String> {
    let v = reader.version(name, version).map_err(|e| anyhow!(e))?;
    let (m, files) = reader
        .fetch(name, version, &v.id)
        .map_err(|e| anyhow!("{name}@{version}: {e}"))?;
    for f in &m.files {
        to.write_once(&blob_key(&f.hash), &files[&f.path])
            .map_err(|e| anyhow!(e))?;
    }
    for key in [
        package_key(&v.id),
        signature_key(&v.id),
        version_key(name, version),
        format!("{}.sig", version_key(name, version)),
    ] {
        if let Some(b) = reader.target.read(&key).map_err(|e| anyhow!(e))? {
            to.write_once(&key, &b).map_err(|e| anyhow!(e))?;
        }
    }
    Ok(v.id)
}

/// Every file of a theme folder as its package would hold it: hidden
/// files and folders (.git, .gitattributes) left out.
fn theme_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let p = e?.path();
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
                files.insert(rel, fs::read(&p)?);
            }
        }
    }
    Ok(files)
}

/// What a scratch registry offers.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Built {
    game: String,
    game_version: String,
    /// NAME@VERSION of each theme listed, the side's first.
    themes: Vec<String>,
}

/// A game that isn't published, built here: its folder and its play files
/// (build-page.sh's OUT.play), published into the scratch registry as
/// scripts/web/publish-game.sh publishes a game.
struct LocalGame {
    dir: PathBuf,
    play: PathBuf,
    version: String,
    title: String,
}

impl LocalGame {
    /// The package's files: the game's folder (its build output, hidden
    /// files and .DS_Store left out) and the play files under play/.
    fn files(&self) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut files: BTreeMap<String, Vec<u8>> = theme_files(&self.dir)?
            .into_iter()
            .filter(|(k, _)| !k.starts_with("target/") && !k.starts_with("play/"))
            .collect();
        for (k, v) in theme_files(&self.play)? {
            files.insert(format!("play/{k}"), v);
        }
        Ok(files)
    }
}

/// Whether the source registry lists the game.
fn source_has(source: &dyn Target, game: &str) -> Result<bool> {
    let reader = Reader::unsigned(source).map_err(|e| anyhow!("reading the registry: {e}"))?;
    let now = crate::registry::utc(0)?;
    let catalog = reader.catalog(&now).map_err(|e| anyhow!(e))?;
    Ok(catalog.package.iter().any(|p| p.name == game))
}

/// Builds the registry one side plays from in `out`: the game's latest
/// version (or, for a game built here, that build), the newest version of
/// each theme the source offers for it (the side's own package name left
/// out), and the side's theme.
fn build_registry(
    source: &dyn Target,
    out: &Path,
    game: &str,
    side: &Side,
    local: Option<&LocalGame>,
) -> Result<Built> {
    let now = crate::registry::utc(0)?;
    // A game built here needs nothing from the source but its themes, so
    // a source that can't be read leaves it with none.
    let reader = match Reader::unsigned(source) {
        Ok(r) => Some(r),
        Err(_) if local.is_some() => None,
        Err(e) => bail!("reading the registry: {e}"),
    };
    let latest = match (&reader, local) {
        (_, Some(l)) => l.version.clone(),
        (Some(r), None) => {
            let catalog = r.catalog(&now).map_err(|e| anyhow!(e))?;
            catalog
                .package
                .iter()
                .find(|p| p.name == game)
                .map(|e| e.latest.clone())
                .ok_or_else(|| {
                    anyhow!("the registry has no {game}: nothing to play the theme in")
                })?
        }
        (None, None) => unreachable!("a source that can't be read was refused above"),
    };
    let mut newest: BTreeMap<String, String> = BTreeMap::new();
    if let Some(by) = reader.as_ref().and_then(|r| r.by_game(game, &now).ok()) {
        for l in by.line {
            if l.package == side.name() {
                continue;
            }
            let keep = newest
                .get(&l.package)
                .is_none_or(|v| version_cmp(&l.version, v).is_gt());
            if keep {
                newest.insert(l.package, l.version);
            }
        }
    }
    let ns = |n: &str| n.split('/').next().unwrap_or("").to_owned();
    let mut namespaces: BTreeSet<String> = BTreeSet::new();
    namespaces.insert(ns(game));
    if !matches!(side, Side::Original) {
        namespaces.insert(ns(side.name()));
    }
    namespaces.extend(newest.keys().map(|n| ns(n)));
    fs::create_dir_all(out)?;
    let to = Folder(out.to_owned());
    let grants: Vec<(String, Vec<String>)> =
        namespaces.into_iter().map(|n| (n, Vec::new())).collect();
    let registry_name = reader
        .as_ref()
        .map_or_else(|| "scratch".to_owned(), |r| r.file.name.clone());
    init(&to, &registry_name, &Signer::none(), &[], &grants, &now).map_err(|e| anyhow!(e))?;
    match (local, &reader) {
        (Some(l), _) => {
            let p = Publish {
                name: game,
                version: &l.version,
                kind: "game",
                title: &l.title,
                for_games: &[],
                published: &now,
            };
            publish(&to, &p, &l.files()?, &Signer::none())
                .map_err(|e| anyhow!("publishing {game} into the scratch registry: {e}"))?;
        }
        (None, Some(r)) => {
            copy_version(r, &to, game, &latest)?;
        }
        (None, None) => unreachable!("a source that can't be read was refused above"),
    }
    let mut listed = vec![(game.to_owned(), latest.clone())];
    let mut themes = vec![match side {
        Side::Original => "the game's own look".to_owned(),
        _ => format!("{}@{}", side.name(), side.version()),
    }];
    match side {
        Side::Folder(p) => {
            let files = theme_files(&p.root)?;
            let p2 = Publish {
                name: &p.name,
                version: &p.version,
                kind: "content",
                title: &p.title,
                for_games: &[game.to_owned()],
                published: &now,
            };
            publish(&to, &p2, &files, &Signer::none())
                .map_err(|e| anyhow!("publishing {} into the scratch registry: {e}", p.name))?;
        }
        Side::Published { name, version } => {
            let r = reader.as_ref().ok_or_else(|| {
                anyhow!("{name}@{version} is published, and the registry can't be read to fetch it")
            })?;
            copy_version(r, &to, name, version)?;
        }
        Side::Original => {}
    }
    if !matches!(side, Side::Original) {
        listed.push((side.name().to_owned(), side.version().to_owned()));
    }
    for (n, v) in &newest {
        // Listed only by a source that could be read.
        let Some(r) = &reader else { break };
        copy_version(r, &to, n, v)?;
        listed.push((n.clone(), v.clone()));
        themes.push(format!("{n}@{v}"));
    }
    let expires = crate::registry::utc(7)?;
    reindex(&to, &listed, &Signer::none(), &now, &expires).map_err(|e| anyhow!(e))?;
    Ok(Built {
        game: game.to_owned(),
        game_version: latest,
        themes,
    })
}

// ---------------------------------------------------------------------
// The moments.

/// A route: holds from power-on, on one ROM of the game.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct RunDef {
    pub name: String,
    /// The game's ROM entry id it plays on (package.toml's [[rom.entry]]).
    pub rom: String,
    pub frames: usize,
    pub holds: Vec<String>,
}

/// A moment to picture: a frame of a run.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct ShotDef {
    pub name: String,
    pub run: String,
    pub frame: u32,
    #[serde(default)]
    pub shows: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub(crate) struct Moments {
    #[serde(default)]
    pub run: Vec<RunDef>,
    #[serde(default)]
    pub shot: Vec<ShotDef>,
}

impl Moments {
    fn check(&self) -> Result<()> {
        for s in &self.shot {
            let Some(r) = self.run.iter().find(|r| r.name == s.run) else {
                bail!("shot {} names run {}, which isn't there", s.name, s.run);
            };
            if s.frame as usize >= r.frames {
                bail!(
                    "shot {} is at frame {}, past run {}'s {} frames",
                    s.name,
                    s.frame,
                    r.name,
                    r.frames
                );
            }
        }
        Ok(())
    }
}

/// The page check's moments: each ROM version's route in the game's
/// page-check.toml (`[[route]]` by its `rom`, with `holds` and `frames` of
/// its own; a route that names a theme-shots.toml run instead is passed
/// over, as a game with that file reads it) with each stage's start, its
/// shot and its clear, the ending and the next loop, and the moments of the
/// game's page-pictures.txt on that ROM. `roms` is each ROM entry's id and
/// hash.
pub(crate) fn page_check_moments(
    page_check: &str,
    pictures: &str,
    roms: &[(String, String)],
) -> Result<Moments> {
    let file: toml::Value = toml::from_str(page_check).context("reading page-check.toml")?;
    let file = serde_json::to_value(file).context("reading page-check.toml")?;
    let mut routes = serde_json::Map::new();
    for r in file["route"].as_array().into_iter().flatten() {
        if let Some(rom) = r["rom"].as_str()
            && r["holds"].is_array()
        {
            let mut r = r.clone();
            r["stages"] = r["stage"].take();
            routes.insert(rom.to_owned(), r);
        }
    }
    let routes = Value::Object(routes);
    let mut m = Moments::default();
    let strs = |v: &Value| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let pics = pictures_moments(pictures);
    for (id, sha) in roms {
        let Some(r) = routes.get(id) else { continue };
        let run = format!("{id}-route");
        let holds = strs(&r["holds"]);
        let frames = r["frames"].as_u64().unwrap_or(0) as usize;
        m.run.push(RunDef {
            name: run.clone(),
            rom: id.clone(),
            frames,
            holds: holds.clone(),
        });
        let mut shot = |name: String, frame: Option<u64>, shows: String| {
            if let Some(f) = frame {
                m.shot.push(ShotDef {
                    name,
                    run: run.clone(),
                    frame: f as u32,
                    shows,
                });
            }
        };
        for st in r["stages"].as_array().into_iter().flatten() {
            let n = st["name"].as_str().unwrap_or("stage");
            shot(
                format!("{n}-start"),
                st["start"].as_u64(),
                format!("{n} at its start, the score line"),
            );
            shot(n.to_owned(), st["shot"].as_u64(), format!("{n} in play"));
            shot(
                format!("{n}-clear"),
                st["clear"].as_u64(),
                format!("{n} won, the hero at the top"),
            );
        }
        shot(
            "ending".into(),
            r["ending"]["frame"].as_u64(),
            "the ending".into(),
        );
        shot(
            "again".into(),
            r["again"]["frame"].as_u64(),
            "the next loop's first stage".into(),
        );
        for p in &pics {
            let Some(short) = p.section.strip_prefix("pictures-rom-") else {
                continue;
            };
            if short.is_empty() || !sha.starts_with(short) || p.frame as usize >= frames {
                continue;
            }
            // A moment the route's own list has already: once is enough.
            if p.press.is_empty() && m.shot.iter().any(|s| s.run == run && s.frame == p.frame) {
                continue;
            }
            // Its look is the page's theme; here it is played in the
            // theme being looked at, so the name leaves the look out.
            let shot_name = match &p.look {
                Some(l) => {
                    p.id.strip_suffix(&format!("-{l}"))
                        .unwrap_or(&p.id)
                        .to_owned()
                }
                None => p.id.clone(),
            };
            let run_of = if p.press.is_empty() {
                run.clone()
            } else {
                let name = format!("{id}-{}", p.id);
                let mut h = holds.clone();
                h.extend(p.press.iter().cloned());
                m.run.push(RunDef {
                    name: name.clone(),
                    rom: id.clone(),
                    frames,
                    holds: h,
                });
                name
            };
            m.shot.push(ShotDef {
                name: shot_name,
                run: run_of,
                frame: p.frame,
                shows: p.shows.clone(),
            });
        }
    }
    Ok(m)
}

struct Picture {
    id: String,
    section: String,
    look: Option<String>,
    frame: u32,
    press: Vec<String>,
    shows: String,
}

/// pictures.txt's moments (`moment ID` and its indented lines).
fn pictures_moments(text: &str) -> Vec<Picture> {
    let mut out: Vec<Picture> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        if let Some(id) = line.strip_prefix("moment ") {
            out.push(Picture {
                id: id.trim().to_owned(),
                section: String::new(),
                look: None,
                frame: 0,
                press: Vec::new(),
                shows: String::new(),
            });
            continue;
        }
        let (Some(p), Some((k, v))) = (out.last_mut(), line.trim().split_once(char::is_whitespace))
        else {
            continue;
        };
        let v = v.trim();
        match k {
            "section" => p.section = v.to_owned(),
            "look" => p.look = Some(v.to_owned()),
            "frame" => p.frame = v.parse().unwrap_or(0),
            "press" => p.press = v.split_whitespace().map(str::to_owned).collect(),
            "shows" => p.shows = v.to_owned(),
            _ => {}
        }
    }
    out
}

/// The moments to picture and where they came from.
fn moments_for(
    given: Option<&Path>,
    game_dir: &Path,
    roms: &[(String, String)],
) -> Result<(Moments, String)> {
    let from_file = |f: &Path| -> Result<(Moments, String)> {
        let m: Moments = toml::from_str(
            &fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?,
        )
        .with_context(|| format!("parsing {}", f.display()))?;
        m.check().with_context(|| f.display().to_string())?;
        Ok((m, f.display().to_string()))
    };
    if let Some(f) = given {
        return from_file(f);
    }
    let own = game_dir.join(MOMENTS_FILE);
    if own.is_file() {
        return from_file(&own);
    }
    // The page check's route, where the game's page-check.toml has one for
    // a ROM of this game.
    let pc_file = game_dir.join("page-check.toml");
    if pc_file.is_file() {
        let pc = fs::read_to_string(&pc_file)
            .with_context(|| format!("reading {}", pc_file.display()))?;
        let pics = fs::read_to_string(game_dir.join("page-pictures.txt")).unwrap_or_default();
        let m =
            page_check_moments(&pc, &pics, roms).with_context(|| pc_file.display().to_string())?;
        if !m.shot.is_empty() {
            m.check()?;
            return Ok((
                m,
                format!(
                    "the page check's route ({}) and page-pictures.txt",
                    pc_file.display()
                ),
            ));
        }
    }
    if let Some(columns) = arcade_columns(game_dir) {
        let m = arcade_fallback_moments(roms, &columns);
        m.check()?;
        return Ok((
            m,
            format!(
                "the game has no {MOMENTS_FILE} of its own, so the attract mode and a few seconds into the first level (a coin at frame {ARCADE_COIN}, the start at {ARCADE_START}); write one in the game's folder for more"
            ),
        ));
    }
    let m = fallback_moments(roms);
    m.check()?;
    Ok((
        m,
        format!(
            "the game has no {MOMENTS_FILE} of its own, so the title and a few seconds into the first stage (Start pressed at frame {FALLBACK_START}); write one in the game's folder for more"
        ),
    ))
}

/// An arcade game's button columns (`CSRLDUT`: a letter each from bit 0,
/// docs/formats/input-log.md), from the `arcade SYSTEM COLUMNS` line of
/// its recorded plays (inputs/*.presses): None for an NES game.
pub(crate) fn arcade_columns(game_dir: &Path) -> Option<String> {
    let mut files: Vec<PathBuf> = fs::read_dir(game_dir.join("inputs"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "presses"))
        .collect();
    files.sort();
    files.iter().find_map(|f| {
        let text = fs::read_to_string(f).ok()?;
        let line = text.lines().nth(2)?.strip_prefix("arcade ")?;
        line.split_whitespace().nth(1).map(str::to_owned)
    })
}

/// The frames the arcade fallback drops a coin and presses the start at.
const ARCADE_COIN: usize = 124;
const ARCADE_START: usize = 195;

/// [`fallback_moments`] for an arcade game whose button columns are
/// `columns`: a coin (`C`) and the start (`S`), held as the board's
/// switches are, then the attract mode and the first level. A frames file
/// holds the columns' bits from bit 0, which [`frames_of`] names by the
/// NES buttons of those bits.
pub(crate) fn arcade_fallback_moments(roms: &[(String, String)], columns: &str) -> Moments {
    const KEYS: [&str; 8] = ["a", "b", "select", "start", "up", "down", "left", "right"];
    let key = |c: char| {
        columns
            .chars()
            .position(|x| x == c)
            .and_then(|i| KEYS.get(i))
    };
    let mut m = Moments::default();
    let (Some(coin), Some(start)) = (key('C'), key('S')) else {
        return m;
    };
    for (id, _) in roms {
        let run = format!("{id}-start");
        m.run.push(RunDef {
            name: run.clone(),
            rom: id.clone(),
            frames: ARCADE_START + 600,
            holds: vec![
                format!("{coin}@{ARCADE_COIN}-{}", ARCADE_COIN + 10),
                format!("{start}@{ARCADE_START}-{}", ARCADE_START + 6),
            ],
        });
        for (name, frame, shows) in [
            ("attract", 100, "the attract mode"),
            (
                "level-1",
                ARCADE_START + 240,
                "the first level, 8 s after the start",
            ),
            (
                "level-1-later",
                ARCADE_START + 540,
                "the first level, 18 s after the start",
            ),
        ] {
            m.shot.push(ShotDef {
                name: name.to_owned(),
                run: run.clone(),
                frame: frame as u32,
                shows: shows.to_owned(),
            });
        }
    }
    m
}

/// The frame the fallback route presses Start at.
const FALLBACK_START: usize = 150;

/// The moments of a game that names none: on each ROM (each id of
/// `roms`), the title screen, then Start and the first stage a few
/// seconds in, twice.
pub(crate) fn fallback_moments(roms: &[(String, String)]) -> Moments {
    let mut m = Moments::default();
    let start = FALLBACK_START;
    for (id, _) in roms {
        let run = format!("{id}-start");
        m.run.push(RunDef {
            name: run.clone(),
            rom: id.clone(),
            frames: start + 600,
            holds: vec![format!("start@{start}-{}", start + 4)],
        });
        for (name, frame, shows) in [
            ("title", 120, "the title screen"),
            ("stage-1", start + 240, "the first stage, 4 s after Start"),
            (
                "stage-1-later",
                start + 540,
                "the first stage, 9 s after Start",
            ),
        ] {
            m.shot.push(ShotDef {
                name: name.to_owned(),
                run: run.clone(),
                frame: frame as u32,
                shows: shows.to_owned(),
            });
        }
    }
    m
}

/// A route's holds (`right+a@100-120`, frames both included) as a frames
/// file (`gp-frames 1`, docs/generator.md), as check-page.mjs makes them.
pub(crate) fn frames_of(holds: &[String], n: usize) -> Result<String> {
    let mut pad = vec![0u8; n];
    for h in holds {
        let (keys, span) = h
            .split_once('@')
            .ok_or_else(|| anyhow!("hold {h:?} has no @"))?;
        let (a, b) = span.split_once('-').unwrap_or((span, span));
        let a: usize = a.parse().with_context(|| format!("hold {h:?}"))?;
        let b: usize = if b.is_empty() {
            n.saturating_sub(1)
        } else {
            b.parse().with_context(|| format!("hold {h:?}"))?
        };
        let mut bits = 0u8;
        for k in keys.split('+') {
            bits |= match k {
                "a" => 1,
                "b" => 2,
                "select" => 4,
                "start" => 8,
                "up" => 16,
                "down" => 32,
                "left" => 64,
                "right" => 128,
                _ => bail!("hold {h:?}: no key {k}"),
            };
        }
        for v in pad.iter_mut().take(b.saturating_add(1)).skip(a) {
            *v |= bits;
        }
    }
    let mut out = String::from("gp-frames 1\n");
    for v in pad {
        let _ = writeln!(out, "{v:02x}");
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// Serving.

/// One side served: its registry and the files laid over the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Served {
    label: String,
    registry: PathBuf,
    overlay: PathBuf,
    port: u16,
    link: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Prepared {
    page: PathBuf,
    minutes: u64,
    sides: Vec<Served>,
    /// --dev: what the server watches and rebuilds from.
    #[serde(default)]
    dev: Option<DevWatch>,
}

/// A --dev server's rebuild: the theme folder and game it was asked for,
/// the ROM given, and the folders whose changes make it build again.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DevWatch {
    dir: PathBuf,
    game: PathBuf,
    rom: Option<PathBuf>,
    watch: Vec<PathBuf>,
}

/// How long a --dev server started with --background serves: a working
/// session, eight hours.
const DEV_MINUTES: u64 = 480;

fn refuse_port(port: u16) -> Result<()> {
    if TAKEN_PORTS.contains(&port) {
        bail!(
            "port {port} is kept for another server (7710 gp serve, 7720 Laya, 7796); give another or 0"
        );
    }
    Ok(())
}

/// Binds 127.0.0.1:`port` (0: any free port but the kept ones).
fn bind(port: u16) -> Result<std::net::TcpListener> {
    refuse_port(port)?;
    loop {
        let l = std::net::TcpListener::bind(("127.0.0.1", port))
            .with_context(|| format!("listening on 127.0.0.1:{port}"))?;
        if !TAKEN_PORTS.contains(&l.local_addr()?.port()) {
            return Ok(l);
        }
    }
}

/// Serves the page, the overlay and the registry on `listener` from a
/// thread of its own, until gp ends.
fn serve_on(listener: std::net::TcpListener, page: &Path, side: &Served) -> Arc<RwLock<Roots>> {
    let now = Arc::new(RwLock::new(Roots::of(page, side)));
    let shared = now.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let r = shared
                .read()
                .map(|r| r.clone())
                .unwrap_or_else(|e| e.into_inner().clone());
            std::thread::spawn(move || crate::registry::answer(stream, &r.roots, &r.headers));
        }
    });
    now
}

/// What one side serves: the overlay, the page and the registry, and the
/// page's headers. A --dev server swaps in a new set when it rebuilds.
#[derive(Clone)]
struct Roots {
    roots: Vec<PathBuf>,
    headers: String,
}

impl Roots {
    fn of(page: &Path, side: &Served) -> Self {
        Self {
            roots: vec![side.overlay.clone(), page.to_owned(), side.registry.clone()],
            headers: crate::registry::page_headers(page),
        }
    }
}

/// A --dev page's overlay: page.json says to reload on a new build, and
/// `dev-rev.txt` holds the build's number, which the page reads every
/// two seconds (page.js).
fn dev_overlay(dir: &Path) -> Result<()> {
    let pj = dir.join("page.json");
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&pj)?)?;
    v["dev_reload"] = json!("dev-rev.txt");
    fs::write(&pj, serde_json::to_string_pretty(&v)? + "\n")?;
    fs::write(dir.join("dev-rev.txt"), "0\n")?;
    Ok(())
}

/// The newest change under `dirs`, by modification time, and the count of
/// files: build output, git and installed packages left out.
fn fingerprint(dirs: &[PathBuf]) -> (u128, u64) {
    let mut newest = 0u128;
    let mut count = 0u64;
    let mut stack: Vec<PathBuf> = dirs.to_vec();
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.')
                || ["target", "node_modules", "results"].contains(&name.as_ref())
            {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                count += 1;
                let t = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |t| t.as_millis());
                newest = newest.max(t);
            }
        }
    }
    (newest, count)
}

/// A --dev server's watch: when a watched file changes (and has stayed
/// unchanged two seconds), gp builds everything again into a new folder
/// (--prepare-into) while the old build is still served, then swaps the
/// sides to it with the next build number, which reloads the open pages.
fn dev_watch(dev: DevWatch, sides: Vec<Arc<RwLock<Roots>>>) {
    std::thread::spawn(move || {
        let mut seen = fingerprint(&dev.watch);
        let mut rev = 0u64;
        let mut last_build: Option<PathBuf> = None;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let now = fingerprint(&dev.watch);
            if now == seen {
                continue;
            }
            std::thread::sleep(Duration::from_secs(2));
            if fingerprint(&dev.watch) != now {
                continue;
            }
            seen = now;
            let Ok(into) = tempfile::Builder::new()
                .prefix("gp-theme-dev-")
                .tempdir()
                .map(|t| t.keep())
            else {
                continue;
            };
            eprintln!("a change: building again into {}", into.display());
            let Ok(exe) = std::env::current_exe() else {
                continue;
            };
            let mut cmd = Command::new(exe);
            cmd.args(["theme", "look", "play"])
                .arg(&dev.dir)
                .arg("--game")
                .arg(&dev.game)
                .args(["--dev", "--anyway", "--prepare-into"])
                .arg(&into)
                .stdin(Stdio::null());
            if let Some(r) = &dev.rom {
                cmd.arg("--rom").arg(r);
            }
            let ok = cmd.status().is_ok_and(|s| s.success());
            let prep: Option<Prepared> = fs::read_to_string(into.join("serve.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
            let Some(prep) = prep.filter(|_| ok) else {
                eprintln!("the build failed; the last one stays served");
                continue;
            };
            rev += 1;
            for (side, served) in sides.iter().zip(&prep.sides) {
                let _ = fs::write(served.overlay.join("dev-rev.txt"), format!("{rev}\n"));
                if let Ok(mut r) = side.write() {
                    *r = Roots::of(&prep.page, served);
                }
            }
            eprintln!("build {rev} served");
            // The build before goes, its pages reloaded onto this one.
            if let Some(old) = last_build.replace(into) {
                let _ = fs::remove_dir_all(old);
            }
        }
    });
}

/// The page's own files for one side: page.json pointed at this server and
/// at the side's theme, and the route files.
fn overlay(dir: &Path, page: &Path, side: &Side, runs: &BTreeMap<String, String>) -> Result<()> {
    let mut pj = page_json(page)?;
    pj["source"]["url"] = json!(".");
    // The name alone: the side's registry lists one version of it, and the
    // page takes a version only in the 1.2.3 shape.
    match side {
        // No theme chosen: the page plays the game's own look.
        Side::Original => {
            if let Some(o) = pj.as_object_mut() {
                o.remove("content");
            }
        }
        _ => pj["content"] = json!(side.name()),
    }
    fs::create_dir_all(dir.join("game").join("runs"))?;
    fs::write(
        dir.join("page.json"),
        serde_json::to_string_pretty(&pj)? + "\n",
    )?;
    for (name, text) in runs {
        fs::write(dir.join("game/runs").join(format!("{name}.frames")), text)?;
    }
    Ok(())
}

/// Puts the person's ROM `r` (the package's ROM `id`) in the overlay at
/// `dir` and names it in its page.json as `local_rom`, which the page
/// plays at once (page.js). Only a page served on this computer has one.
fn local_rom(dir: &Path, id: &str, r: &crate::roms::Rom) -> Result<()> {
    let ext = if r.system.is_some() { "zip" } else { "nes" };
    let name = format!("local/{id}.{ext}");
    fs::create_dir_all(dir.join("local"))?;
    fs::write(dir.join(&name), &r.bytes)?;
    let pj = dir.join("page.json");
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&pj)?)?;
    v["local_rom"] = json!(name);
    fs::write(&pj, serde_json::to_string_pretty(&v)? + "\n")?;
    Ok(())
}

fn answers(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    )
    .is_ok()
}

fn open_link(link: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        // start's first quoted argument is the window's title.
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "\"\""]);
        c
    } else {
        Command::new("xdg-open")
    };
    let _ = cmd
        .arg(link)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// `--serve-prepared`: the --background server.
fn serve_prepared(root: &Path) -> Result<Outcome> {
    let p: Prepared = serde_json::from_str(&fs::read_to_string(root.join("serve.json"))?)?;
    let mut handles = Vec::new();
    for s in &p.sides {
        handles.push(serve_on(bind(s.port)?, &p.page, s));
        println!("serving {} at {}", s.label, s.link);
    }
    if let Some(dev) = p.dev.clone() {
        println!(
            "watching {} folders, building again on a change",
            dev.watch.len()
        );
        dev_watch(dev, handles);
    }
    std::thread::sleep(Duration::from_secs(p.minutes * 60));
    // Its scratch folder goes with it.
    let _ = fs::remove_dir_all(root);
    Ok(Outcome::ok(json!({ "stopped": true }), "stopped".into()))
}

// ---------------------------------------------------------------------
// The check.

/// The look check's problems and warnings, as lines, and its counts.
fn check_lines(o: &Outcome) -> (Vec<String>, u64, u64) {
    let mut lines = Vec::new();
    for f in o.json["findings"].as_array().into_iter().flatten() {
        let level = f["level"].as_str().unwrap_or("");
        if level != "problem" && level != "warning" {
            continue;
        }
        let at = match (f["file"].as_str(), f["line"].as_u64()) {
            (Some(file), Some(l)) => format!("{file} line {l}: "),
            (Some(file), None) => format!("{file}: "),
            _ => String::new(),
        };
        lines.push(format!(
            "  {level}: {at}{}",
            f["message"].as_str().unwrap_or("")
        ));
    }
    (
        lines,
        o.json["problems"].as_u64().unwrap_or(0),
        o.json["warnings"].as_u64().unwrap_or(0),
    )
}

// ---------------------------------------------------------------------
// The pictures.

/// What the page says once a game frame has reached `frame`: the game's
/// `__gp.state()` from `win` (a JavaScript expression for its window),
/// or null.
fn state_js(win: &str, frame: u32) -> String {
    format!(
        "(() => {{ const w = {win}; const st = w && !w.__checkOld && w.__gp?.state?.(); return st && st.frame >= {frame} ? st : null; }})()"
    )
}

/// The game's state, its canvas as a PNG data address and its theme, all
/// from the window `win` once its frame has reached `frame`, or null.
fn snap_js(win: &str, frame: u32) -> String {
    format!(
        "(() => {{ const w = {win}; if (!w || w.__checkOld) return null; const st = w.__gp?.state?.(); if (!st || st.frame < {frame}) return null; const c = w.document.getElementById('game'); if (!c) return null; return {{ st, png: c.toDataURL('image/png'), theme: w.__gp?.theme ?? null }}; }})()"
    )
}

/// The game frame's window in the page that plays any game by name.
const GAME_WIN: &str = "document.getElementById('game')?.contentWindow";

/// One side's pictures on one ROM, in headless Chrome driven from here
/// (src/chrome.rs): a line for each picture (`shot`, `file`, `frame`,
/// `level`, `theme`), and a last one (`done`, `console_lines`,
/// `console_errors`). The console goes to PREFIX-console.txt. With no ROM
/// the game plays from its own files and `link` is the player itself (the
/// interpreter preview, runs/ beside it), opened at each moment with
/// `run`, `from` and `to`. These are the ways of the page the page check
/// (check-page.mjs, playTo) plays it by.
#[allow(clippy::too_many_arguments)]
fn shoot(
    work: &Path,
    origin: &str,
    link: &str,
    rom: Option<&Path>,
    browser: &Path,
    out: &Path,
    prefix: &str,
    shots: &[&ShotDef],
) -> Result<Vec<Value>> {
    let mut p = crate::chrome::Page::launch(browser, work, 1366, 1100)?;
    let mut lines = Vec::new();
    let r = shoot_in(&mut p, origin, link, rom, out, prefix, shots, &mut lines);
    let mut console = p.console.join("\n");
    console.push('\n');
    fs::write(out.join(format!("{prefix}-console.txt")), console)?;
    lines.push(json!({
        "done": true,
        "console_lines": p.console.len(),
        "console_errors": p.errors,
    }));
    p.close();
    if let Err(e) = r {
        lines.push(json!({ "failed": format!("{e:#}") }));
    }
    Ok(lines)
}

#[allow(clippy::too_many_arguments)]
fn shoot_in(
    p: &mut crate::chrome::Page,
    origin: &str,
    link: &str,
    rom: Option<&Path>,
    out: &Path,
    prefix: &str,
    shots: &[&ShotDef],
    lines: &mut Vec<Value>,
) -> Result<()> {
    // The game reached the frame, then, a moment later, its state, canvas
    // and theme read in one go from the same window: the game frame can be
    // replaced while it loads (a run's file fetch aborted and started
    // again), and a read across two windows gives a blank canvas.
    let save = |p: &mut crate::chrome::Page, s: &ShotDef, win: &str| -> Result<Value> {
        let t0 = Instant::now();
        let mut got: Option<Value> = None;
        while got.is_none() && t0.elapsed() < Duration::from_secs(120) {
            if p.wait_for(&state_js(win, s.frame), 120).is_none() {
                break;
            }
            p.pause(250);
            got = p.eval(&snap_js(win, s.frame)).ok().filter(|v| !v.is_null());
        }
        let st = got.as_ref().map(|v| v["st"].clone());
        let png = got
            .as_ref()
            .and_then(|v| v["png"].as_str().map(str::to_owned));
        let theme = got.as_ref().map_or(Value::Null, |v| v["theme"].clone());
        let mut file = Value::Null;
        if let (Some(png), Some(_)) = (&png, &st) {
            use base64::Engine as _;
            let b64 = png.split_once(',').map_or("", |x| x.1);
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
            let f = out.join(format!("{prefix}-{}.png", s.name));
            fs::write(&f, bytes)?;
            file = json!(f);
        }
        let frame = st.as_ref().map_or(Value::Null, |v| v["frame"].clone());
        let level = st.as_ref().map_or(Value::Null, |v| v["level"].clone());
        eprintln!(
            "  {prefix}-{}: frame {frame}, {}",
            s.name,
            if file.is_null() {
                "no picture"
            } else {
                "saved"
            }
        );
        Ok(json!({ "shot": s.name, "file": file, "frame": frame, "level": level, "theme": theme }))
    };
    let Some(rom) = rom else {
        for s in shots {
            p.goto(&format!(
                "{origin}{link}&run={}&from={}&to={}",
                s.run, s.frame, s.frame
            ))?;
            lines.push(save(p, s, "window")?);
        }
        return Ok(());
    };
    p.goto(&format!("{origin}{link}"))?;
    p.wait_for("window.__page !== undefined", 300);
    p.set_file("#rom", rom)?;
    let kind = p
        .wait_for(
            "(() => { const k = document.getElementById('message')?.dataset.kind; return ['ok', 'warn', 'error'].includes(k) ? k : null; })()",
            300,
        )
        .ok_or_else(|| anyhow!("the page said nothing of the ROM in five minutes"))?;
    let message = p
        .eval("document.getElementById('message').innerText")?
        .as_str()
        .unwrap_or("")
        .to_owned();
    if kind == "error" {
        bail!("the page refused the ROM: {message}");
    }
    p.wait_for(
        "document.getElementById('game').src.includes('files=idb:')",
        60,
    )
    .ok_or_else(|| anyhow!("the page didn't start the game from the ROM in a minute"))?;
    p.pause(2000);
    let src = p
        .eval("document.getElementById('game').getAttribute('src')")?
        .as_str()
        .unwrap_or("")
        .to_owned();
    lines.push(json!({ "message": message, "src": src }));
    for s in shots {
        let _ = p.eval(&format!("{GAME_WIN}.__checkOld = true"));
        let to = format!("{src}&run={}&from={}&to={}", s.run, s.frame, s.frame);
        p.eval(&format!(
            "document.getElementById('game').src = {}",
            json!(to)
        ))?;
        p.wait_for("document.getElementById('game').src.includes('run=')", 5);
        lines.push(save(p, s, GAME_WIN)?);
    }
    Ok(())
}

/// A picture made `k` times bigger, each pixel a k by k square.
fn bigger(img: &Rgba, k: u32) -> Rgba {
    let mut out = Rgba::new(img.width * k, img.height * k);
    for y in 0..out.height {
        for x in 0..out.width {
            out.set(x, y, img.get(x / k, y / k));
        }
    }
    out
}

/// Two pictures side by side, `a` on the left, on a dark ground. The
/// shorter is made a whole number of times bigger, up to the taller's
/// height, so a large-art look (drawn at its `scale`) and the game in its
/// own pictures (at the console's size) show at about the same size.
pub(crate) fn pair(a: &Rgba, b: &Rgba) -> Rgba {
    let k = |s: &Rgba, t: &Rgba| (t.height / s.height.max(1)).max(1);
    let (a, b) = match (k(a, b), k(b, a)) {
        (ka, _) if ka > 1 => (bigger(a, ka), b.clone()),
        (_, kb) if kb > 1 => (a.clone(), bigger(b, kb)),
        _ => (a.clone(), b.clone()),
    };
    let (a, b) = (&a, &b);
    let gap = 16;
    let mut out = Rgba::new(a.width + gap + b.width, a.height.max(b.height));
    for y in 0..out.height {
        for x in 0..out.width {
            out.set(x, y, [24, 24, 24, 255]);
        }
    }
    for (img, dx) in [(a, 0), (b, a.width + gap)] {
        for y in 0..img.height {
            for x in 0..img.width {
                out.set(dx + x, y, img.get(x, y));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------
// The command.

pub(crate) fn run(a: &PlayArgs, paths: &Paths) -> Result<Outcome> {
    if let Some(root) = &a.serve_prepared {
        return serve_prepared(root);
    }
    refuse_port(a.port)?;
    if a.shots.is_some() && a.background {
        bail!("--shots runs to the end by itself: leave out --background");
    }
    let game_dir = crate::theme_kit::template_dir(&a.game)?;
    let game_dir = game_dir.canonicalize().unwrap_or(game_dir);
    let facts = game_facts(&game_dir)?;
    let theme = theme_pkg(&a.dir)?;
    let mut text = String::new();

    // 1. The check.
    eprintln!(
        "checking {} against {} ...",
        theme.root.display(),
        facts.title
    );
    let checked = crate::theme_look::check(&a.dir, &a.game, a.rom.as_deref(), paths)?;
    let (lines, problems, warnings) = check_lines(&checked);
    let _ = writeln!(
        text,
        "The check (gp theme look check): {problems} problem{}, {warnings} warning{}",
        if problems == 1 { "" } else { "s" },
        if warnings == 1 { "" } else { "s" }
    );
    for l in &lines {
        let _ = writeln!(text, "{l}");
    }
    if problems > 0 && !a.anyway {
        let _ = write!(
            text,
            "Stopped: fix the problems above (gp theme look check {} --game {} says more), or give --anyway to play it as it is.",
            a.dir.display(),
            a.game.display()
        );
        return Ok(Outcome {
            json: json!({ "ok": false, "check": checked.json, "stopped": "the check found problems" }),
            text,
            exit: 1,
        });
    }
    if let Some(was) = &theme.renamed {
        let _ = writeln!(
            text,
            "Your package name {was} isn't one a registry takes (lowercase letters, digits, - and .), so the game is given it as {}: rename it in package.toml before you ask for it to be published",
            theme.name
        );
    }
    let mut sides = vec![Side::Folder(theme.clone())];
    if let Some(other) = &a.against {
        let s = against_side(other)?;
        if s.name() == theme.name && s.version() == theme.version {
            bail!(
                "--against {other} has the same name and version as your folder ({}@{}): the page couldn't tell them apart. Change one's version in package.toml",
                theme.name,
                theme.version
            );
        }
        if let Side::Folder(p) = &s {
            let o = crate::theme_look::check(&p.root, &a.game, a.rom.as_deref(), paths)?;
            let (_, pr, wa) = check_lines(&o);
            let _ = writeln!(
                text,
                "The check on {}: {pr} problems, {wa} warnings (not shown)",
                p.root.display()
            );
        }
        sides.push(s);
    }

    let work = tempfile::Builder::new()
        .prefix("gp-theme-play-")
        .tempdir()
        .context("making a scratch folder")?;
    let tags: Vec<String> = {
        let mut t: Vec<String> = sides.iter().map(Side::tag).collect();
        if t.len() == 2 && t[0] == t[1] {
            t[1].push_str("-against");
        }
        t
    };
    if facts.road == Road::Own {
        return own::run(
            a,
            paths,
            &game_dir,
            &facts,
            &sides,
            &tags,
            work,
            text,
            (problems, warnings),
        );
    }
    let pkg = crate::package::Package::load(&game_dir)?;

    // 2. The page and the registries.
    let PageBuilt { page, play } = if a.dev && a.page.is_none() {
        let Some(repo) = checkout() else {
            bail!("--dev builds the page from the gameprinter checkout, which this gp can't find");
        };
        build_page(&repo, &game_dir, &pkg.name, paths)?
    } else {
        find_page(a.page.as_deref(), &game_dir, &pkg.name, paths)?
    };
    let source: Source = serde_json::from_value(page_json(&page)?["source"].clone())
        .context("the page's page.json has no source")?;
    let (src, src_words) = source_registry(a.registry.as_deref(), &source, &game_dir, paths)?;
    let _ = writeln!(text, "The page: {}", page.display());
    // A game the source doesn't have (one made here, not published yet)
    // is built here and published into the scratch registry alone.
    let local = match source_has(src.as_ref(), &pkg.name) {
        Ok(true) if !a.dev => None,
        has => {
            let play = match play {
                Some(p) => p,
                None => {
                    let Some(repo) = checkout() else {
                        bail!(
                            "{} isn't published in {src_words}, and building it for the browser needs the gameprinter checkout, which this gp can't find",
                            facts.title
                        );
                    };
                    build_page(&repo, &game_dir, &pkg.name, paths)?
                        .play
                        .expect("a page built here has its play files")
                }
            };
            let _ = writeln!(
                text,
                "The game: {}@{} built here from {}, since {}",
                pkg.name,
                pkg.version,
                game_dir.display(),
                if a.dev {
                    "--dev plays the game's folder as it is"
                } else if has.is_err() {
                    "the registry couldn't be read"
                } else {
                    "the registry has no version of it"
                }
            );
            let title = package_title(&game_dir).unwrap_or_else(|| facts.title.clone());
            Some(LocalGame {
                dir: game_dir.clone(),
                play,
                version: pkg.version.clone(),
                title,
            })
        }
    };
    let _ = writeln!(text, "The game and its themes from: {src_words}");
    let mut built = Vec::new();
    for (i, s) in sides.iter().enumerate() {
        eprintln!("building the registry for {} ...", s.words());
        let b = build_registry(
            src.as_ref(),
            &work.path().join(format!("side-{i}/registry")),
            &pkg.name,
            s,
            local.as_ref(),
        )?;
        built.push(b);
    }

    // 3a. Pictures.
    if let Some(out) = &a.shots {
        let r = shots(
            a,
            paths,
            &pkg,
            &game_dir,
            &page,
            &sides,
            &tags,
            work.path(),
            out,
            text,
        );
        // A theme copy's journey: pictured in the game.
        if let Ok(o) = &r
            && o.exit == 0
        {
            crate::journeys::note_play(&paths.home, &a.dir, json!({"shots": out}));
        }
        return r;
    }

    // 3b. Serving. The person's own ROM, when one the game takes is here
    // (--rom, else $GP_ROM_DIR), goes with the page on this computer so it
    // plays at once: page.json's `local_rom`, a file of the overlay, which
    // goes when the server stops.
    let wanted: Vec<String> = pkg.entries.iter().map(|e| e.sha256.clone()).collect();
    let given: Vec<PathBuf> = a.rom.iter().cloned().collect();
    let found = crate::roms::find(&wanted, &given, &paths.roms).unwrap_or_default();
    let mine = pkg
        .entries
        .iter()
        .find_map(|e| found.get(&e.sha256).map(|r| (e.id.clone(), r)));
    let mut served = Vec::new();
    let mut listeners = Vec::new();
    for (i, s) in sides.iter().enumerate() {
        let dir = work.path().join(format!("side-{i}"));
        overlay(&dir.join("overlay"), &page, s, &BTreeMap::new())?;
        if let Some((id, r)) = &mine {
            local_rom(&dir.join("overlay"), id, r)?;
        }
        if a.dev {
            dev_overlay(&dir.join("overlay"))?;
        }
        let l = bind(if i == 0 { a.port } else { 0 })?;
        let port = l.local_addr()?.port();
        let link = format!(
            "http://127.0.0.1:{port}/p?g={}{}",
            pkg.name,
            s.content_param()
        );
        served.push(Served {
            label: s.words(),
            registry: dir.join("registry"),
            overlay: dir.join("overlay"),
            port,
            link,
        });
        listeners.push(l);
    }
    // --dev: the theme, the game and the gameprinter checkout's sources.
    let mut watch = vec![a.dir.clone(), game_dir.clone()];
    if let Some(repo) = checkout() {
        watch.extend(["crates", "scripts/web"].map(|d| repo.join(d)));
    }
    let how = Serving {
        watch,
        page: &page,
        built: &built,
        checked: (problems, warnings),
        source: &src_words,
        hint: if mine.is_some() {
            "Your ROM from this computer plays at once; your theme is chosen (Settings, Theme lists the others). Nothing was published."
        } else {
            "Load your own ROM in the page; your theme is chosen (Settings, Theme lists the others). Nothing was published."
        },
    };
    serve_sides(a, paths, work, &how, served, listeners, text)
}

/// What a served run says about itself.
struct Serving<'a> {
    /// The folders a --dev server watches.
    watch: Vec<PathBuf>,
    page: &'a Path,
    built: &'a [Built],
    /// The look check's problems and warnings.
    checked: (u64, u64),
    /// Where the game and its themes came from.
    source: &'a str,
    /// The last line: what the person does in the page.
    hint: &'a str,
}

/// Serves each side on its listener, in the background (a process of its
/// own) or until the minutes are up, and says where.
fn serve_sides(
    a: &PlayArgs,
    paths: &Paths,
    work: tempfile::TempDir,
    how: &Serving,
    served: Vec<Served>,
    listeners: Vec<std::net::TcpListener>,
    mut text: String,
) -> Result<Outcome> {
    let (page, built, (problems, warnings), src_words) =
        (how.page, how.built, how.checked, how.source);
    // A --dev server is a working session's page: hours, not the hour a
    // look at a theme gets.
    let minutes = a
        .minutes
        .or(a.background.then_some(if a.dev { DEV_MINUTES } else { 60 }));
    let summary = |text: &mut String| {
        for (s, b) in served.iter().zip(built) {
            let _ = writeln!(
                text,
                "\n{} {} with {}:\n  {}\n  The page also offers: {}",
                b.game,
                b.game_version,
                s.label,
                s.link,
                if b.themes.len() > 1 {
                    b.themes[1..].join(", ")
                } else {
                    "nothing else".into()
                }
            );
        }
        let _ = writeln!(text, "\n{}", how.hint);
    };
    let json_out = |pid: Option<u32>| {
        json!({
            "ok": true,
            "check": { "problems": problems, "warnings": warnings },
            "page": page,
            "registry": src_words,
            "links": served.iter().map(|s| s.link.clone()).collect::<Vec<_>>(),
            "offers": built,
            "pid": pid,
            "minutes": minutes,
        })
    };
    let dev = a.dev.then(|| DevWatch {
        dir: a.dir.clone(),
        game: a.game.clone(),
        rom: a.rom.clone(),
        watch: how.watch.clone(),
    });
    if let Some(into) = &a.prepare_into {
        // Built for a --dev server to swap in: its folders stay.
        let _ = work.keep();
        drop(listeners);
        let prep = Prepared {
            page: page.to_owned(),
            minutes: minutes.unwrap_or(60),
            sides: served.clone(),
            dev,
        };
        fs::create_dir_all(into)?;
        fs::write(
            into.join("serve.json"),
            serde_json::to_string_pretty(&prep)?,
        )?;
        return Ok(Outcome::ok(json!({ "prepared": into }), "prepared".into()));
    }
    if a.background {
        // The scratch folder stays for the server, which removes it when it
        // stops, and binds the ports itself.
        let root = work.keep();
        drop(listeners);
        let prep = Prepared {
            page: page.to_owned(),
            minutes: minutes.unwrap_or(60),
            sides: served.clone(),
            dev,
        };
        fs::write(
            root.join("serve.json"),
            serde_json::to_string_pretty(&prep)?,
        )?;
        let log = fs::File::create(root.join("serve.log"))?;
        let mut cmd = Command::new(std::env::current_exe()?);
        cmd.args(["theme", "look", "play"])
            .arg(&a.dir)
            .arg("--game")
            .arg(&a.game)
            .arg("--serve-prepared")
            .arg(&root)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        gp_mcp::proc::own_group(&mut cmd);
        let child = cmd.spawn().context("starting the server")?;
        let t0 = Instant::now();
        while !served.iter().all(|s| answers(s.port)) {
            if t0.elapsed() > Duration::from_secs(20) {
                bail!(
                    "the server (PID {}) didn't answer in 20 s; its log is {}",
                    child.id(),
                    root.join("serve.log").display()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        summary(&mut text);
        let _ = write!(
            text,
            "Serving from PID {} for {} minutes, then it stops by itself; `kill {}` stops it sooner.",
            child.id(),
            prep.minutes,
            child.id()
        );
        if a.open {
            open_link(&served[0].link);
        }
        // A theme copy's journey: seen in the game.
        crate::journeys::note_play(&paths.home, &a.dir, json_out(Some(child.id())));
        return Ok(Outcome::ok(json_out(Some(child.id())), text));
    }
    let mut handles = Vec::new();
    for (s, l) in served.iter().zip(listeners) {
        handles.push(serve_on(l, page, s));
    }
    // A --dev server served here watches as one served in the background
    // does: it builds again on a change and reloads the open pages.
    if let Some(dev) = dev.clone() {
        dev_watch(dev, handles);
    }
    summary(&mut text);
    let _ = write!(
        text,
        "{}",
        match minutes {
            Some(m) => format!(
                "Serving for {m} minute{}; ctrl-C stops it sooner.",
                if m == 1 { "" } else { "s" }
            ),
            None => "Serving until ctrl-C.".into(),
        }
    );
    // The link goes out now: the command runs on.
    println!("{text}");
    if a.open {
        open_link(&served[0].link);
    }
    match minutes {
        Some(m) => std::thread::sleep(Duration::from_secs(m * 60)),
        None => loop {
            std::thread::sleep(Duration::from_secs(3600));
        },
    }
    drop(work);
    Ok(Outcome {
        json: json_out(None),
        text: "stopped".into(),
        exit: 0,
    })
}

#[allow(clippy::too_many_arguments)]
fn shots(
    a: &PlayArgs,
    paths: &Paths,
    pkg: &crate::package::Package,
    game_dir: &Path,
    page: &Path,
    sides: &[Side],
    tags: &[String],
    work: &Path,
    out: &Path,
    mut text: String,
) -> Result<Outcome> {
    // The ROMs: the one given, else each the game takes that is here.
    let wanted: Vec<String> = pkg.entries.iter().map(|e| e.sha256.clone()).collect();
    let given: Vec<PathBuf> = a.rom.iter().cloned().collect();
    let found = crate::roms::find(&wanted, &given, &paths.roms)?;
    let found: BTreeMap<String, crate::roms::Rom> = if a.rom.is_some() {
        let only = crate::roms::Rom::read(a.rom.as_deref().unwrap_or(Path::new("")))?.sha256;
        found.into_iter().filter(|(k, _)| *k == only).collect()
    } else {
        found
    };
    let roms: Vec<(String, String)> = pkg
        .entries
        .iter()
        .map(|e| (e.id.clone(), e.sha256.clone()))
        .collect();
    let here: Vec<&crate::package::RomEntry> = pkg
        .entries
        .iter()
        .filter(|e| found.contains_key(&e.sha256))
        .collect();
    if here.is_empty() {
        bail!(
            "--shots needs a ROM of {}: none of its ROMs is in $GP_ROM_DIR ({}); give one with --rom FILE",
            pkg.name,
            paths.roms.display()
        );
    }
    let (moments, from) = moments_for(a.moments.as_deref(), game_dir, &roms)?;
    let _ = writeln!(text, "The moments: {from}");
    let mut runs = BTreeMap::new();
    for r in &moments.run {
        runs.insert(r.name.clone(), frames_of(&r.holds, r.frames)?);
    }
    let mut origins = Vec::new();
    for (i, s) in sides.iter().enumerate() {
        let dir = work.join(format!("side-{i}"));
        overlay(&dir.join("overlay"), page, s, &runs)?;
        let l = bind(0)?;
        let port = l.local_addr()?.port();
        let sv = Served {
            label: s.words(),
            registry: dir.join("registry"),
            overlay: dir.join("overlay"),
            port,
            link: format!("/p?g={}{}", pkg.name, s.content_param()),
        };
        serve_on(l, page, &sv);
        origins.push(sv);
    }
    let rom_dir = work.join("roms");
    fs::create_dir_all(&rom_dir)?;
    let mut plays = Vec::new();
    for e in &here {
        let rom = &found[&e.sha256];
        let rom_file = rom_dir.join(format!(
            "{}.{}",
            e.id,
            if rom.platform == crate::roms::Platform::Arcade {
                "zip"
            } else {
                "nes"
            }
        ));
        fs::write(&rom_file, &rom.bytes)?;
        let list: Vec<&ShotDef> = moments
            .shot
            .iter()
            .filter(|s| moments.run.iter().any(|r| r.name == s.run && r.rom == e.id))
            .collect();
        if list.is_empty() {
            let _ = writeln!(text, "{}: no moments for this ROM in {from}", e.id);
            continue;
        }
        plays.push(Play {
            id: e.id.clone(),
            rom: Some(rom_file),
            shots: list,
        });
    }
    let pictured = Pictured {
        sides,
        tags,
        origins: &origins,
        work,
        moments: &from,
    };
    pictured.take(paths, a.browser.as_deref(), out, &plays, text)
}

/// One play of the game for pictures: on a ROM (its entry id), or, for a
/// game played from its own files, on none.
struct Play<'a> {
    id: String,
    rom: Option<PathBuf>,
    shots: Vec<&'a ShotDef>,
}

/// The sides being pictured and where each is served.
struct Pictured<'a> {
    sides: &'a [Side],
    tags: &'a [String],
    origins: &'a [Served],
    work: &'a Path,
    /// Where the moments came from.
    moments: &'a str,
}

impl Pictured<'_> {
    /// Each play's pictures for each side into `out`, and the pairs when
    /// there are two sides.
    fn take(
        &self,
        paths: &Paths,
        browser: Option<&Path>,
        out: &Path,
        plays: &[Play],
        mut text: String,
    ) -> Result<Outcome> {
        let (sides, tags, origins, work) = (self.sides, self.tags, self.origins, self.work);
        let browser = crate::chrome::browser(browser, &paths.home)?;
        fs::create_dir_all(out).with_context(|| format!("making {}", out.display()))?;
        let out = out.canonicalize()?;
        // Git leaves the folder alone even inside a checkout.
        fs::write(out.join(".gitignore"), "*\n")?;
        let mut report: Vec<Value> = Vec::new();
        let mut pairs = Vec::new();
        // Each pair with its moment, for a page that steps through them.
        let mut paired = Vec::new();
        let mut shot_count = 0;
        for play in plays {
            let list = &play.shots;
            let mut files: Vec<BTreeMap<String, PathBuf>> = Vec::new();
            for (i, s) in sides.iter().enumerate() {
                let prefix = format!("{}-{}", play.id, tags[i]);
                eprintln!(
                    "playing {} on {} ({} moments) ...",
                    s.words(),
                    play.id,
                    list.len()
                );
                let lines = shoot(
                    work,
                    &format!("http://127.0.0.1:{}", origins[i].port),
                    &origins[i].link,
                    play.rom.as_deref(),
                    &browser,
                    &out,
                    &prefix,
                    list,
                )?;
                let mut got = BTreeMap::new();
                let mut errors = 0;
                for v in &lines {
                    if let Some(f) = v["failed"].as_str() {
                        bail!("{prefix}: {f}");
                    }
                    if let (Some(n), Some(file)) = (v["shot"].as_str(), v["file"].as_str()) {
                        got.insert(n.to_owned(), PathBuf::from(file));
                        if !matches!(s, Side::Original) && v["theme"].as_str() != Some(s.look_id())
                        {
                            let _ = writeln!(
                                text,
                                "warning: {prefix}-{n} was drawn in look {}, not {}",
                                v["theme"],
                                s.look_id()
                            );
                        }
                    }
                    if v["done"].as_bool() == Some(true) {
                        errors = v["console_errors"].as_u64().unwrap_or(0);
                    }
                }
                shot_count += got.len();
                let missing: Vec<&str> = list
                    .iter()
                    .filter(|m| !got.contains_key(&m.name))
                    .map(|m| m.name.as_str())
                    .collect();
                let _ = writeln!(
                    text,
                    "{} on {}: {} of {} pictures{}; console errors: {errors} ({prefix}-console.txt)",
                    s.words(),
                    play.id,
                    got.len(),
                    list.len(),
                    if missing.is_empty() {
                        String::new()
                    } else {
                        format!(" (none at {})", missing.join(", "))
                    }
                );
                report.push(json!({
                    "rom": play.id,
                    "theme": match s {
                        Side::Original => gp_present::looks::ORIGINAL.to_owned(),
                        _ => format!("{}@{}", s.name(), s.version()),
                    },
                    "pictures": got,
                    "missing": missing,
                    "console_errors": errors,
                }));
                files.push(got);
            }
            if files.len() == 2 {
                for s in list {
                    let (Some(l), Some(r)) = (files[0].get(&s.name), files[1].get(&s.name)) else {
                        continue;
                    };
                    let (l, r) = (
                        Rgba::load(l).map_err(|e| anyhow!(e))?,
                        Rgba::load(r).map_err(|e| anyhow!(e))?,
                    );
                    let p = out.join(format!("pair-{}-{}.png", play.id, s.name));
                    fs::write(&p, pair(&l, &r).png())?;
                    paired.push(json!({
                        "rom": play.id, "moment": s.name, "run": s.run, "frame": s.frame,
                        "shows": s.shows, "file": p,
                    }));
                    pairs.push(p);
                }
            }
        }
        let _ = writeln!(text, "\n{shot_count} pictures in {}", out.display());
        for (i, s) in sides.iter().enumerate() {
            let _ = writeln!(text, "  ROM-{}-MOMENT.png: {}", tags[i], s.words());
        }
        if !pairs.is_empty() {
            let _ = writeln!(
                text,
                "  pair-ROM-MOMENT.png: {} pairs, {} on the left, {} on the right",
                pairs.len(),
                tags[0],
                tags[1]
            );
        }
        let _ = write!(
            text,
            "{}Nothing was published.",
            if plays.iter().any(|p| p.rom.is_some()) {
                "These pictures show a theme over your ROM's game: keep them to yourself and never commit them (the folder's .gitignore keeps git from adding them). "
            } else {
                ""
            }
        );
        Ok(Outcome::ok(
            json!({
                "ok": true,
                "out": out,
                "moments": self.moments,
                "runs": report,
                "pairs": pairs,
                "paired": paired,
                "sides": sides.iter().map(Side::words).collect::<Vec<_>>(),
                "pictures": shot_count,
            }),
            text,
        ))
    }
}

mod own;

#[cfg(test)]
mod tests;
