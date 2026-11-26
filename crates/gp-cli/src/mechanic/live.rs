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
pub(crate) fn out_dir(paths: &Paths, game: &str) -> PathBuf {
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

pub(crate) fn built(dir: &Path) -> bool {
    dir.join("gp_web.wasm").is_file() && dir.join("bundle.json").is_file()
}

/// The crates the preview's module is built from (`crates/gp-web` and the
/// workspace crates it depends on), so a change to how a picture is picked
/// or drawn makes the build stale as a model change does.
const PREVIEW_CRATES: &[&str] = &[
    "gp-web",
    "gp-web-board",
    "gp-controls",
    "gp-look",
    "gp-arcade",
    "gp-assets",
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
    "preview.css",
    "preview.js",
    "level.js",
    "shell-keys.js",
    "controls.js",
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
    if let Some(m) = &rb.module_now() {
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
    crate::files::walk_lenient(root, &mut files);
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

/// Whether the game has a model to play: its package holds a model.toml,
/// on either platform (an arcade game's model plays too, issue #3).
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
        // Which build is played: a page holding another reloads.
        "build": is_built.then(|| made.trim().to_owned()),
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
        rb.module_now(),
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
/// doesn't hold and this says so; the page tries again. Frame 0 is
/// power-on ([`at_power_on`]).
///
/// The model stands at the frame a lockstep from power-on would have it
/// at, one step for each frame the original read the pad (the start
/// cache's prefix): a model that counts its own frames (a boot that waits
/// so many frames from the model's first) then goes on with the original.
/// Until 2026-11-19 it stood at its first frame wherever it started, so
/// one started in the boot waited the whole boot again and trailed the
/// original on the screen page by as many frames as it joined late.
pub(crate) fn start(
    paths: &Paths,
    rb: &Rebuild,
    rom: &[u8],
    path: &str,
    frame: u64,
) -> Result<Value> {
    use gp_interp::lockstep::{Mode, Options};
    if !has_model(rb) {
        bail!("there is no model of this game to start");
    }
    let opened = opened(paths, rb, rom)?;
    let (source, mapping) = (&opened.0, &opened.1);
    if frame == 0 {
        return at_power_on(source, mapping);
    }
    let mut log = super::paths::read_path(&paths.home, path)?;
    let at = usize::try_from(frame)?;
    if at > log.frames.len() {
        bail!("the play has {} frames, not {frame}", log.frames.len());
    }
    log.frames.truncate(at);
    let inputs = crate::run::frames(&log);
    let spec = source.spec.clone();
    let opts = Options {
        mode: Mode::Free,
        from: Some(frame),
        until_start: true,
        ..Options::default()
    };
    let (mut original, p) = prefix_at(paths, rom, source, mapping, &log, &inputs, &opts, at - 1)?;
    let mut model = source.open()?;
    let ls = gp_interp::lockstep::lockstep_resumed(
        &mut original,
        &mut model,
        &spec,
        mapping,
        &inputs,
        &opts,
        None,
        p,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let s = ls.start.ok_or_else(|| {
        anyhow::anyhow!(
            "the original never reached a frame the model stands for at frame {frame} (the mapping's start doesn't hold, or the frame was one the game didn't read the pad on)"
        )
    })?;
    Ok(json!({
        "original_frame": s.original_frame,
        "frame": s.frame,
        "state": base64::engine::general_purpose::STANDARD.encode(&s.state),
        "missing": s.notes.iter().filter(|n| n.how.name() == "missing").count(),
        "notes": s.notes.len(),
        "draw_sites": draw_sites(mapping),
    }))
}

/// [`start`] at frame 0, power-on: no frame of the original stands before
/// it, so the model stands there as it opens, as a lockstep from power-on
/// has it, and takes the clip's presses from its first frame. Until
/// 2026-11-23 frame 0 said "the game hasn't started", so LOOK AT IT on a
/// clip of the whole power-on showed no model.
fn at_power_on(source: &crate::hosted::Source, mapping: &Mapping) -> Result<Value> {
    let model = source.open()?;
    let state = gp_runtime::Runtime::canonical_state(&model);
    Ok(json!({
        "original_frame": 0,
        "frame": 0,
        "state": base64::engine::general_purpose::STANDARD.encode(state),
        "missing": 0,
        "notes": 0,
        "draw_sites": draw_sites(mapping),
    }))
}

/// The original after the first `k` frames of `log` and where a lockstep
/// from power-on stands there ([`gp_interp::lockstep::Prefix`]): from the
/// start cache, under the clip scorer's tag, else played once from
/// power-on and kept.
#[allow(clippy::too_many_arguments)]
fn prefix_at(
    paths: &Paths,
    rom: &[u8],
    source: &crate::hosted::Source,
    mapping: &Mapping,
    log: &gp_evidence::input_log::InputLog,
    inputs: &[gp_runtime::InputFrame],
    opts: &gp_interp::lockstep::Options,
    k: usize,
) -> Result<(
    crate::original::OriginalRuntime,
    gp_interp::lockstep::Prefix,
)> {
    use crate::clip::cache;
    use crate::original::OriginalRuntime;
    use gp_evidence::manifest::PowerOn;
    let home = &paths.home;
    let key = cache::start_key(rom, &PowerOn::default(), &log.frames, k);
    let tag = crate::clip_score::prefix_tag(mapping);
    if cache::starts_on()
        && let Some(p) = cache::read_note(home, &key, &tag)
            .and_then(|b| serde_json::from_slice::<gp_interp::lockstep::Prefix>(&b).ok())
            .filter(|p| p.inputs == k)
        && let Some(saved) = cache::read_start(home, &key, rom, PowerOn::default())
    {
        return Ok((OriginalRuntime::from_saved(rom, &saved)?, p));
    }
    let mut original = OriginalRuntime::of_rom(rom)?;
    let mut interp = source.open()?;
    let p = gp_interp::lockstep::prefix(
        &mut original,
        &mut interp,
        &source.spec,
        mapping,
        inputs,
        opts,
        k,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    if cache::starts_on() {
        cache::write_start_of(home, &key, &original)?;
        cache::write_note(home, &key, &tag, &serde_json::to_vec(&p)?);
    }
    Ok((original, p))
}

/// The mapping's draw sites for the page's player to record the
/// original's random draws at, a site a line
/// ([`gp_arcade::draws::Site::line`]), when the mapping scores with them
/// (`score_draws`): the page hands each frame's draws to the model as a
/// clip's score does, so the two make the same random choices live.
/// Null when it doesn't.
pub(crate) fn draw_sites(mapping: &Mapping) -> Value {
    if !mapping.score_draws || mapping.draws.is_empty() {
        return Value::Null;
    }
    let lines: String = crate::lockstep::arcade_sites(mapping)
        .iter()
        .map(|s| s.line() + "\n")
        .collect();
    json!(lines)
}

/// [`draw_sites`] of the game's mapping as it is on disk, without opening
/// the model: null when there is none or it doesn't read.
pub(crate) fn draw_sites_of(rb: &Rebuild) -> Value {
    std::fs::read_to_string(rb.game.join(gp_interp::mapping::MAPPING_FILE))
        .ok()
        .and_then(|t| Mapping::parse(&t).ok())
        .map_or(Value::Null, |m| draw_sites(&m))
}

/// The model and its mapping, opened once and kept while their files and
/// the module are unchanged: each start of the rebuild needs them, and
/// opening them (the module above all) takes seconds. The last two are
/// kept, so the screen going between main's model and a gate card's
/// (BEFORE and AFTER, [`super::ask_look`]) opens neither again.
pub(crate) type Opened = std::sync::Arc<(crate::hosted::Source, Mapping)>;
static OPENED: Mutex<Vec<(blake3::Hash, Opened)>> = Mutex::new(Vec::new());
const OPENED_KEPT: usize = 2;

pub(crate) fn opened(paths: &Paths, rb: &Rebuild, rom: &[u8]) -> Result<Opened> {
    let mut h = blake3::Hasher::new();
    for f in [gp_model::MODEL_FILE, gp_interp::mapping::MAPPING_FILE] {
        h.update(&std::fs::read(rb.game.join(f)).unwrap_or_default());
    }
    let module = rb.module_now();
    if let Some(m) = &module {
        h.update(&std::fs::read(m).unwrap_or_default());
    }
    if let Some((_, sha)) = &rb.rom {
        h.update(sha.as_bytes());
    }
    h.update(rb.game.as_os_str().as_encoded_bytes());
    let key = h.finalize();
    {
        let kept = OPENED.lock().expect("opened");
        if let Some((_, o)) = kept.iter().find(|(k, _)| *k == key) {
            return Ok(o.clone());
        }
    }
    let mfile = rb.game.join(gp_interp::mapping::MAPPING_FILE);
    let mapping = Mapping::parse(
        &std::fs::read_to_string(&mfile).with_context(|| format!("reading {}", mfile.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", mfile.display()))?;
    let assets = rb.assets_for(paths, rom)?;
    let (source, _) =
        crate::hosted::Source::open_dir(&rb.game, Some(&assets), paths, module.as_deref())?;
    let o: Opened = std::sync::Arc::new((source, mapping));
    let mut kept = OPENED.lock().expect("opened");
    kept.retain(|(k, _)| *k != key);
    kept.push((key, o.clone()));
    if kept.len() > OPENED_KEPT {
        kept.remove(0);
    }
    Ok(o)
}

/// A file of the preview: the page's scripts from the repo (so a build
/// made before a fix still runs it), the rest from the build.
/// The themes the screen offers for `game`, each by the id the page picks
/// it by with the folder its look.txt is in, found again on every request
/// so a theme's edit shows on the next load ([`themes_of`] with the game's
/// folder).
fn own_themes(paths: &Paths, game: &str) -> Vec<(String, PathBuf)> {
    themes_of(paths, game, game_folder(game).as_deref())
}

/// The game's folder, `GAME` in the games checkout or in gameprinter's
/// own templates/, looked up without reading any template (the screen asks
/// every second).
fn game_folder(game: &str) -> Option<PathBuf> {
    if !super::name_ok(game) {
        return None;
    }
    [
        crate::theme_template::games_root(),
        Some(crate::serve::repo_root().join("templates")),
    ]
    .into_iter()
    .flatten()
    .map(|r| r.join(game))
    .find(|d| d.join("template.toml").is_file() || d.join("roms.toml").is_file())
}

/// The themes of [`own_themes`], the first of an id kept: each folder a
/// row of the quest log names for its clip's look (`gp mechanic queue
/// --look DIR`, kept as the clip's `look_dir`), the person's theme copies
/// made from `game`, then the game folder's own themes, published or not
/// (`GAME/themes/NAME`, the kit left out), with no server of its own
/// (docs/mechanic-page.md, the screen).
fn themes_of(paths: &Paths, game: &str, folder: Option<&Path>) -> Vec<(String, PathBuf)> {
    use crate::journeys::{Kind, list, look_dir};
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut add = |id: &str, dir: Option<PathBuf>| {
        if let Some(d) = dir
            && super::name_ok(id)
            && id != "game"
            && !out.iter().any(|(i, _)| i == id)
        {
            out.push((id.to_owned(), d));
        }
    };
    if let Ok(f) = super::session::queue_file(paths, game) {
        for r in super::session::read_queue(&f) {
            let c = &r["clip"];
            if let (Some(id), Some(d)) = (c["look"].as_str(), c["look_dir"].as_str()) {
                add(id, look_dir(Path::new(d)));
            }
        }
    }
    for c in list(&paths.home, None) {
        if c.kind == Kind::Theme && c.from.as_ref().is_some_and(|o| o.name == game) {
            add(&c.name, look_dir(&c.dir));
        }
    }
    if let Some(g) = folder {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(g.join("themes"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !p.ends_with("kit"))
            .collect();
        dirs.sort();
        for d in dirs {
            let id = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
            add(id, look_dir(&d));
        }
    }
    out
}

/// Each of the person's themes for `game` with the time, in ms since 1970,
/// its folder's newest file was written: a theme being made changes the
/// screen by itself.
pub(crate) fn theme_times(paths: &Paths, game: &str) -> Vec<(String, u64)> {
    own_themes(paths, game)
        .into_iter()
        .map(|(id, dir)| {
            let mut files = Vec::new();
            crate::files::walk_lenient(&dir, &mut files);
            let newest = files
                .iter()
                .filter_map(|f| std::fs::metadata(f).ok()?.modified().ok())
                .chain(std::fs::metadata(&dir).ok().and_then(|m| m.modified().ok()))
                .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .max()
                .unwrap_or(0);
            (id, newest)
        })
        .collect()
}

pub(crate) fn file(paths: &Paths, repo: &Path, game: &str, name: &str) -> Result<Response> {
    file_in(paths, repo, game, &current(paths, game), name)
}

/// A file of the preview built into `built` ([`file`] for main's, a gate
/// card's look for one built from its patch, [`super::ask_look`]).
pub(crate) fn file_in(
    paths: &Paths,
    repo: &Path,
    game: &str,
    built: &Path,
    name: &str,
) -> Result<Response> {
    if name.is_empty()
        || name.contains("..")
        || name.starts_with('/')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
    {
        return Ok(Response::error(404, "not found"));
    }
    // The person's own themes for this game (their theme copies, made
    // from it), offered as the preview's looks: the side by side shows the
    // rebuilt game in a theme being made beside the original.
    let own = own_themes(paths, game);
    if name == "preview.json" && !own.is_empty() {
        let Ok(b) = std::fs::read(built.join(name)) else {
            return Ok(Response::error(404, "not in the preview: build it first"));
        };
        let mut v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
        v["looks"] = json!(true);
        return Ok(Response::game_file(
            "application/json",
            serde_json::to_vec(&v).unwrap_or_default(),
        ));
    }
    if name == "themes/index.json" {
        let list: Vec<Value> = own.iter().map(|(id, _)| json!({ "id": id })).collect();
        return Ok(Response::game_file(
            "application/json",
            serde_json::to_vec(&json!({ "themes": list })).unwrap_or_default(),
        ));
    }
    if let Some(rest) = name.strip_prefix("themes/")
        && let Some((id, file)) = rest.split_once('/')
        && let Some((_, dir)) = own.iter().find(|(i, _)| i == id)
    {
        let f = dir.join(file);
        let kind = match f.extension().and_then(|x| x.to_str()) {
            Some("png") => "image/png",
            Some("wav") => "audio/wav",
            _ => "text/plain; charset=utf-8",
        };
        return Ok(match std::fs::read(&f) {
            Ok(b) => Response::game_file(kind, b),
            Err(_) => Response::error(404, "not in the theme"),
        });
    }
    let f = if PAGE_SCRIPTS.contains(&name) {
        repo.join("scripts/web/page").join(name)
    } else {
        built.join(name)
    };
    let kind = match f.extension().and_then(|x| x.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        _ => "text/plain; charset=utf-8",
    };
    match std::fs::read(&f) {
        Ok(b) => Ok(Response::game_file(kind, b)),
        Err(_) => Ok(Response::error(404, "not in the preview: build it first")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_paths as paths;

    #[test]
    fn the_slots_take_turns_and_only_a_built_one_is_current() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        assert!(current(&p, "mosshop").ends_with("a"));
        assert_eq!(spare(&p, "mosshop").1, "a");
        let a = out_dir(&p, "mosshop").join("a");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("gp_web.wasm"), b"").unwrap();
        std::fs::write(a.join("bundle.json"), b"{}").unwrap();
        std::fs::write(out_dir(&p, "mosshop").join("current"), "a").unwrap();
        assert_eq!(spare(&p, "mosshop").1, "b");
        std::fs::write(out_dir(&p, "mosshop").join("current"), "b").unwrap();
        assert!(current(&p, "mosshop").ends_with("b"));
    }

    /// The screen offers the game folder's own themes (the kit and a folder
    /// with no look left out) and the folder a row names for its look,
    /// which wins over a theme of the same id; each is read from disk as it
    /// is, so an edit shows on the next load.
    #[test]
    fn the_screen_offers_the_games_themes_and_a_rows_theme_folder() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let look = |dir: &Path, id: &str| {
            std::fs::create_dir_all(dir.join("theme")).unwrap();
            std::fs::write(dir.join("theme/look.txt"), format!("gp-look 1\nid {id}\n")).unwrap();
        };
        let game = d.path().join("games/rooftop");
        look(&game.join("themes/last-call"), "last-call");
        look(&game.join("themes/kit"), "kit");
        std::fs::create_dir_all(game.join("themes/half-made")).unwrap();
        let ids = |v: &[(String, PathBuf)]| v.iter().map(|(i, _)| i.clone()).collect::<Vec<_>>();
        let found = themes_of(&p, "rooftop", Some(&game));
        assert_eq!(ids(&found), ["last-call"]);
        assert_eq!(found[0].1, game.join("themes/last-call/theme"));

        let work = d.path().join("work/last-call");
        look(&work, "last-call");
        let other = d.path().join("elsewhere/neon");
        look(&other, "neon");
        let q = super::super::session::queue_file(&p, "rooftop").unwrap();
        std::fs::create_dir_all(q.parent().unwrap()).unwrap();
        std::fs::write(
            &q,
            serde_json::to_vec(&json!([
                {"mechanic": "a", "clip": {"n": 1, "look": "last-call", "look_dir": work}},
                {"mechanic": "b", "clip": {"n": 2, "look": "neon", "look_dir": other}},
                {"mechanic": "c", "clip": {"n": 3, "look": "game"}},
            ]))
            .unwrap(),
        )
        .unwrap();
        let found = themes_of(&p, "rooftop", Some(&game));
        assert_eq!(ids(&found), ["last-call", "neon"]);
        assert_eq!(found[0].1, work.join("theme"));
        assert_eq!(found[1].1, other.join("theme"));
    }

    #[test]
    fn a_file_is_one_of_the_builds_or_the_pages_scripts() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let repo = d.path().join("repo");
        std::fs::create_dir_all(repo.join("scripts/web/page")).unwrap();
        std::fs::write(repo.join("scripts/web/page/preview.js"), "js").unwrap();
        let a = out_dir(&p, "mosshop").join("a");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("bundle.json"), "{}").unwrap();
        assert_eq!(
            file(&p, &repo, "mosshop", "preview.js").unwrap().body,
            b"js"
        );
        assert_eq!(
            file(&p, &repo, "mosshop", "bundle.json").unwrap().body,
            b"{}"
        );
        for bad in ["../x", "/etc/passwd", "a b", ""] {
            assert_eq!(
                file(&p, &repo, "mosshop", bad).unwrap().status,
                404,
                "{bad}"
            );
        }
        assert_eq!(
            file(&p, &repo, "mosshop", "module.wasm").unwrap().status,
            404
        );
    }

    /// A clip that starts at power-on (frame 0) starts the model as it
    /// opens, with no original frame before it, and the state loads back
    /// into a new run of the model as the one it opens at.
    #[test]
    fn a_clip_from_power_on_starts_the_model_as_it_opens() {
        use base64::Engine as _;
        use gp_runtime::Runtime as _;
        let text = "[model]\nschema = \"0.4\"\nname = \"cab\"\ntitle = \"Cab\"\n\
                    description = \"A made-up game\"\nview = \"side\"\nframe_rate = \"30\"\n\
                    systems = [\"rules\", \"entity_states\", \"counters\", \"spawn\", \"platformer.player\", \"platformer.enemies\", \"platformer.contact\", \"exits\", \"camera\"]\n\
                    start_level = \"main\"\n\n[model.extensions]\nplatformer = \"0.4\"\n\n\
                    [entities.player]\nrole = \"player\"\nsize = [16, 16]\n\
                    hitbox = { x = 0, y = 0, w = 16, h = 16 }\n\n\
                    [levels.main]\nempty = true\nsize = [32, 30]\ntile_size = 16\n";
        let m = gp_model::Model::parse(text).unwrap();
        let mut lifted = gp_interp::Lifted::default();
        lifted.files.insert(
            "assets.toml".into(),
            "schema = 1\nplatform = \"arcade\"\nsource = \"cab\"\nsets = [\"cab\"]\n\n\
             [[asset]]\nname = \"tiles\"\nformat = \"mcr-tiles\"\nregion = \"gfx1\"\ncount = 1\nper_row = 1\n"
                .into(),
        );
        let bundle = gp_interp::Bundle::new(&m, lifted);
        let source = crate::hosted::Source::new(bundle, None).unwrap();
        let mapping = Mapping::parse(&format!(
            "schema = {}\nmodel = \"cab\"\nsource = \"cab\"\nalign = \"polled\"\n\
             [[field]]\npath = \"entities.player[0].x\"\nram = [0x40]\n",
            gp_interp::mapping::SCHEMA
        ))
        .unwrap();
        let r = at_power_on(&source, &mapping).unwrap();
        assert_eq!(r["original_frame"], json!(0));
        assert_eq!(r["frame"], json!(0));
        assert_eq!(r["missing"], json!(0));
        assert_eq!(r["draw_sites"], Value::Null);
        let state = base64::engine::general_purpose::STANDARD
            .decode(r["state"].as_str().unwrap())
            .unwrap();
        let opened = source.open().unwrap();
        assert_eq!(state, opened.canonical_state());
        let mut again = source.open().unwrap();
        again.load_state(&state).unwrap();
        assert_eq!(again.state_hash(), opened.state_hash());
    }
}
