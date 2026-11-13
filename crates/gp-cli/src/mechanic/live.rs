//! The rebuild played live on the mechanic page: the interpreter preview of
//! the game's model (docs/web.md; scripts/web/build-preview.sh), in control
//! mode in a frame the page drives (`files=here`, so it reads the build
//! beside it rather than the browser's store). The page plays the original
//! in gp_play.wasm and hands the model each frame the original read the
//! pad, so both take the same presses (PLAY THE REBUILD), or plays the model
//! alone from its own start (FREE PLAY). The model starts from the
//! original's state where the person asks, through the state mapping, as
//! `gp interp translate` starts it.
//!
//! Ported from game-printer's mechanic/live.rs. The model is the game's in
//! the games checkout (games/GAME), not a template in the tool's repo, and a
//! game with hooks plays its module (`--module`, or
//! `$GP_HOME/work/modules/GAME.wasm`), copied into the build as
//! module.wasm, which the preview loads in place of its own interpreter.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use serde_json::{Value, json};

use super::http::Response;
use super::review::Rebuild;
use gp_evidence::Paths;
use gp_interp::mapping::Mapping;

/// Where the preview is built: the model's, shared, so nothing private.
/// Two slots, `a` and `b`: the page plays the one `current` names while a
/// newer build goes into the other, which then becomes current.
fn out_dir(paths: &Paths, game: &str) -> PathBuf {
    paths.home.join("work").join("mechanic-preview").join(game)
}

fn current(paths: &Paths, game: &str) -> PathBuf {
    let out = out_dir(paths, game);
    match std::fs::read_to_string(out.join("current"))
        .ok()
        .as_deref()
        .map(str::trim)
    {
        Some(slot @ ("a" | "b")) => out.join(slot),
        _ => out.join("a"),
    }
}

fn spare(paths: &Paths, game: &str) -> (PathBuf, &'static str) {
    let out = out_dir(paths, game);
    let slot = if current(paths, game).ends_with("a") && built(&out.join("a")) {
        "b"
    } else {
        "a"
    };
    (out.join(slot), slot)
}

/// Whether the game's preview has been built once, so its files can be served.
pub(crate) fn is_built(paths: &Paths, game: &str) -> bool {
    built(&current(paths, game))
}

fn built(dir: &Path) -> bool {
    dir.join("gp_web.wasm").is_file() && dir.join("bundle.json").is_file()
}

/// The crates the preview's module is built from (`crates/gp-web` and the
/// workspace crates it depends on), so a change to how a picture is picked
/// or drawn makes the build stale as a model change does.
const PREVIEW_CRATES: &[&str] = &[
    "gp-web",
    "gp-exts",
    "gp-interp",
    "gp-model",
    "gp-present",
    "gp-runtime",
    "gp-sim",
    "gp-sound",
];

/// The page's own scripts, served from the repo rather than the build, so
/// a build made before a fix to them still runs it. build-preview.sh
/// copies the same list.
pub(crate) const PAGE_SCRIPTS: &[&str] = &[
    "index.html",
    "preview.js",
    "level.js",
    "shell-keys.js",
    "storage.js",
    "storage-wit.js",
    "module-host.js",
    "module-worker.js",
];

/// What a build is made from, by content: the model, its mapping and
/// profiles, its ext crate, its module, the lifted assets, the preview's
/// crates and the build script. Nothing about where or when the files were
/// written, so a checkout brought to a new tip with the same content keeps
/// its build.
fn content_key(paths: &Paths, repo: &Path, rb: &Rebuild) -> String {
    let dir = &rb.game;
    let mut h = blake3::Hasher::new();
    for c in PREVIEW_CRATES {
        h.update(c.as_bytes());
        add_dir(&mut h, &repo.join("crates").join(c), &["target"]);
    }
    for f in ["Cargo.lock", "scripts/web/build-preview.sh"] {
        h.update(f.as_bytes());
        h.update(&std::fs::read(repo.join(f)).unwrap_or_default());
    }
    for f in ["model.toml", "mapping.toml", "profiles.toml"] {
        h.update(f.as_bytes());
        h.update(&std::fs::read(dir.join(f)).unwrap_or_default());
    }
    add_dir(&mut h, &dir.join("ext"), &["target"]);
    if let Some(m) = &rb.module {
        h.update(b"module");
        h.update(&std::fs::read(m).unwrap_or_default());
    }
    if let Some(a) = &rb.assets {
        h.update(b"set assets");
        add_dir(&mut h, a, &[]);
    } else if let Some(src) = std::fs::read_to_string(dir.join("model.toml"))
        .ok()
        .and_then(|t| gp_model::Model::parse(&t).ok())
        .and_then(|m| m.model.source.clone())
    {
        add_dir(
            &mut h,
            &paths.home.join("work").join("assets").join(src),
            &[],
        );
    }
    h.finalize().to_hex().to_string()
}

/// Every file under `root` (but the top folders in `skip`), by its path
/// there and its bytes, in a fixed order.
fn add_dir(h: &mut blake3::Hasher, root: &Path, skip: &[&str]) {
    let mut files = Vec::new();
    walk(root, &mut files);
    files.retain(|f| {
        !f.strip_prefix(root)
            .ok()
            .and_then(|r| r.components().next())
            .and_then(|c| c.as_os_str().to_str())
            .is_some_and(|c| skip.contains(&c))
    });
    files.sort();
    for f in files {
        if let Ok(rel) = f.strip_prefix(root) {
            h.update(rel.to_string_lossy().as_bytes());
            h.update(&std::fs::read(&f).unwrap_or_default());
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// A build running, and the last one's end.
#[derive(Default)]
struct Build {
    started: Option<std::time::Instant>,
    error: Option<String>,
    seconds: Option<f64>,
}

static BUILD: Mutex<Option<Build>> = Mutex::new(None);

/// Whether a preview build is running.
pub(crate) fn building() -> bool {
    BUILD
        .lock()
        .expect("build")
        .as_ref()
        .is_some_and(|b| b.started.is_some())
}

/// Whether the game has a model to play: an arcade game has none yet.
fn has_model(rb: &Rebuild) -> bool {
    rb.game.join(gp_model::MODEL_FILE).is_file()
}

pub(crate) fn status(paths: &Paths, repo: &Path, game: &str, rb: &Rebuild) -> Value {
    let cur = current(paths, game);
    let is_built = built(&cur);
    let made = std::fs::read_to_string(cur.join(".key")).unwrap_or_default();
    let b = BUILD.lock().expect("build");
    let b = b.as_ref();
    json!({
        "model": has_model(rb),
        "built": is_built,
        // Older than the model: still played, while a newer one builds.
        "stale": is_built && made.trim() != content_key(paths, repo, rb),
        "building": b.and_then(|b| b.started).map(|t| t.elapsed().as_secs_f64()),
        "error": b.and_then(|b| b.error.clone()),
        "seconds": b.and_then(|b| b.seconds),
    })
}

/// Builds the preview again when the one played is older than the model
/// or the preview's code, so the next PLAY THE REBUILD or FREE PLAY plays
/// the model as it stands rather than the build before the change. Only a
/// preview built once is kept up to date: the first build waits until the
/// page asks for it. The server calls this every half minute.
pub(crate) fn refresh(paths: &Paths, repo: &Path, game: &str, rb: &Rebuild, rom: &[u8]) {
    let cur = current(paths, game);
    if building() || !built(&cur) {
        return;
    }
    let made = std::fs::read_to_string(cur.join(".key")).unwrap_or_default();
    if made.trim() != content_key(paths, repo, rb)
        && let Err(e) = build(paths, repo, game, rb, rom)
    {
        eprintln!("gp mechanic: couldn't build the rebuild's preview again: {e:#}");
    }
}

/// Starts building the preview into the spare slot, unless a build is
/// running; the slot becomes current when the build succeeds.
pub(crate) fn build(
    paths: &Paths,
    repo: &Path,
    game: &str,
    rb: &Rebuild,
    rom: &[u8],
) -> Result<()> {
    let mut b = BUILD.lock().expect("build");
    let state = b.get_or_insert_with(Build::default);
    if state.started.is_some() {
        return Ok(());
    }
    if !has_model(rb) {
        bail!(
            "{game} has no model to play: {} has no model.toml",
            rb.game.display()
        );
    }
    let assets = rb.assets_for(paths, rom)?;
    let (dir, slot) = spare(paths, game);
    let pointer = out_dir(paths, game).join("current");
    state.started = Some(std::time::Instant::now());
    state.error = None;
    let (repo, model, module, key) = (
        repo.to_path_buf(),
        rb.game.clone(),
        rb.module.clone(),
        content_key(paths, repo, rb),
    );
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let r = std::fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                std::process::Command::new("bash")
                    .arg(repo.join("scripts/web/build-preview.sh"))
                    .arg(&dir)
                    .arg(&model)
                    .arg(&assets)
                    .current_dir(&repo)
                    .output()
                    .map_err(|e| e.to_string())
            })
            .and_then(|o| {
                if o.status.success() {
                    // The game's module beside the build, or none: a slot
                    // built before for a module that has gone keeps no
                    // stale one.
                    let m = dir.join("module.wasm");
                    let moved = match &module {
                        Some(src) => std::fs::copy(src, &m).map(|_| ()),
                        None if m.exists() => std::fs::remove_file(&m),
                        None => Ok(()),
                    };
                    // The key first, then the switch: the page never plays
                    // a half-written slot.
                    moved
                        .and_then(|()| std::fs::write(dir.join(".key"), &key))
                        .and_then(|()| std::fs::write(&pointer, slot))
                        .map_err(|e| e.to_string())
                } else {
                    let err = String::from_utf8_lossy(&o.stderr);
                    let tail: Vec<&str> = err.lines().rev().take(6).collect();
                    Err(tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
                }
            });
        let mut b = BUILD.lock().expect("build");
        let state = b.get_or_insert_with(Build::default);
        state.started = None;
        state.seconds = Some(t.elapsed().as_secs_f64());
        state.error = r.err();
    });
    Ok(())
}

/// The switches and presets of the preview being played, for FREE PLAY's
/// settings: from the build's own profiles.toml (the one the preview
/// applies), else the game's, else none. Each group with its switches and
/// their descriptions, in the file's order, and each preset with its
/// switches; `base` is the model as written.
pub(crate) fn profiles(paths: &Paths, game: &str, rb: &Rebuild) -> Result<Value> {
    let built = current(paths, game);
    let dir = if built.join(gp_model::profile::PROFILES_FILE).is_file() {
        built
    } else {
        rb.game.clone()
    };
    let p = gp_model::profile::Profiles::read_dir(&dir).map_err(anyhow::Error::msg)?;
    let groups: Vec<Value> = p
        .groups
        .iter()
        .map(|g| {
            json!({
                "name": g.name,
                "title": g.heading(),
                "description": g.description,
                "switches": g
                    .switches
                    .iter()
                    .map(|s| json!({"name": s.name, "description": s.description}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let presets: Vec<Value> = p
        .presets
        .iter()
        .map(|(name, pr)| {
            json!({
                "name": name,
                "description": pr.description,
                "switches": pr.switches,
            })
        })
        .collect();
    Ok(json!({
        "base": gp_model::profile::BASE,
        "groups": groups,
        "presets": presets,
    }))
}

/// The model's state at frame `frame` of the stored play `path` (the
/// original's state there, translated), for the page to load into the
/// preview: the original stands one frame before and takes that frame's
/// presses, so the start is the frame itself when the game reads the pad
/// on it. Until the game is in play (a title screen) the mapping's start
/// doesn't hold and this says so; the page tries again.
pub(crate) fn start(
    paths: &Paths,
    rb: &Rebuild,
    rom: &[u8],
    path: &str,
    frame: u64,
) -> Result<Value> {
    if frame == 0 {
        bail!("the game hasn't started");
    }
    if !has_model(rb) {
        bail!("there is no model of this game to start");
    }
    let opened = opened(paths, rb, rom)?;
    let (source, mapping) = (&opened.0, &opened.1);
    let mut model = source.open()?;
    let log = super::paths::read_path(&paths.home, path)?;
    let at = usize::try_from(frame)?;
    if at > log.frames.len() {
        bail!("the play has {} frames, not {frame}", log.frames.len());
    }
    let original = super::paths::original_at(&paths.home, rom, path, at - 1)?;
    let mut one = log.clone();
    one.frames = vec![log.frames[at - 1]];
    let inputs = crate::run::frames(&one);
    let spec = source.spec.clone();
    let (_, s) =
        gp_interp::lockstep::start_at(&original, &mut model, &spec, mapping, &inputs, frame)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(json!({
        "original_frame": s.original_frame,
        "frame": s.frame,
        "state": base64::engine::general_purpose::STANDARD.encode(&s.state),
        "missing": s.notes.iter().filter(|n| n.how.name() == "missing").count(),
        "notes": s.notes.len(),
    }))
}

/// The model and its mapping, opened once and kept while their files and
/// the module are unchanged: each start of the rebuild needs them, and
/// opening them (the module above all) takes seconds.
pub(crate) type Opened = std::sync::Arc<(crate::hosted::Source, Mapping)>;
static OPENED: Mutex<Option<(blake3::Hash, Opened)>> = Mutex::new(None);

pub(crate) fn opened(paths: &Paths, rb: &Rebuild, rom: &[u8]) -> Result<Opened> {
    let mut h = blake3::Hasher::new();
    for f in [gp_model::MODEL_FILE, gp_interp::mapping::MAPPING_FILE] {
        h.update(&std::fs::read(rb.game.join(f)).unwrap_or_default());
    }
    if let Some(m) = &rb.module {
        h.update(&std::fs::read(m).unwrap_or_default());
    }
    if let Some((_, sha)) = &rb.rom {
        h.update(sha.as_bytes());
    }
    let key = h.finalize();
    let mut kept = OPENED.lock().expect("opened");
    if let Some((k, o)) = kept.as_ref()
        && *k == key
    {
        return Ok(o.clone());
    }
    let mfile = rb.game.join(gp_interp::mapping::MAPPING_FILE);
    let mapping = Mapping::parse(
        &std::fs::read_to_string(&mfile).with_context(|| format!("reading {}", mfile.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", mfile.display()))?;
    let assets = rb.assets_for(paths, rom)?;
    let (source, _) =
        crate::hosted::Source::open_dir(&rb.game, Some(&assets), paths, rb.module.as_deref())?;
    let o: Opened = std::sync::Arc::new((source, mapping));
    *kept = Some((key, o.clone()));
    Ok(o)
}

/// A file of the preview: the page's scripts from the repo (so a build
/// made before a fix still runs it), the rest from the build.
pub(crate) fn file(paths: &Paths, repo: &Path, game: &str, name: &str) -> Result<Response> {
    if name.is_empty()
        || name.contains("..")
        || name.starts_with('/')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
    {
        return Ok(Response::error(404, "not found"));
    }
    let f = if PAGE_SCRIPTS.contains(&name) {
        repo.join("scripts/web/page").join(name)
    } else {
        current(paths, game).join(name)
    };
    let kind = match f.extension().and_then(|x| x.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        _ => "text/plain; charset=utf-8",
    };
    match std::fs::read(&f) {
        Ok(b) => Ok(Response::bytes(kind, b)),
        Err(_) => Ok(Response::error(404, "not in the preview: build it first")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(home: &Path) -> Paths {
        Paths {
            home: home.to_path_buf(),
            store: home.join("store"),
            roms: home.join("roms"),
        }
    }

    #[test]
    fn the_slots_take_turns_and_only_a_built_one_is_current() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        assert!(current(&p, "dk").ends_with("a"));
        assert_eq!(spare(&p, "dk").1, "a");
        let a = out_dir(&p, "dk").join("a");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("gp_web.wasm"), b"").unwrap();
        std::fs::write(a.join("bundle.json"), b"{}").unwrap();
        std::fs::write(out_dir(&p, "dk").join("current"), "a").unwrap();
        assert_eq!(spare(&p, "dk").1, "b");
        std::fs::write(out_dir(&p, "dk").join("current"), "b").unwrap();
        assert!(current(&p, "dk").ends_with("b"));
    }

    #[test]
    fn a_file_is_one_of_the_builds_or_the_pages_scripts() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let repo = d.path().join("repo");
        std::fs::create_dir_all(repo.join("scripts/web/page")).unwrap();
        std::fs::write(repo.join("scripts/web/page/preview.js"), "js").unwrap();
        let a = out_dir(&p, "dk").join("a");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("bundle.json"), "{}").unwrap();
        assert_eq!(file(&p, &repo, "dk", "preview.js").unwrap().body, b"js");
        assert_eq!(file(&p, &repo, "dk", "bundle.json").unwrap().body, b"{}");
        for bad in ["../x", "/etc/passwd", "a b", ""] {
            assert_eq!(file(&p, &repo, "dk", bad).unwrap().status, 404, "{bad}");
        }
        assert_eq!(file(&p, &repo, "dk", "module.wasm").unwrap().status, 404);
    }
}
