//! A gate card's look: the model as the card's patch would leave it, so its
//! LOOK AT IT plays the change the person is asked to land, not main's
//! model. The ask (`$GP_HOME/work/asks/ID.json`, a `gp model gate` refused
//! for want of the person's yes) names its patch (`--patch FILE`); the
//! game's folder in the games checkout is copied, the patch applied to the
//! copy, the copy's module built from its hook code, its pictures lifted by
//! its asset map, and the preview built from it as [`super::live`] builds
//! main's. The screen loads it (`rebuild-ask/ID/index.html`) and starts it
//! at the clip (`POST /api/rebuild/start` with `ask`) as it does main's.
//!
//! | Path | What |
//! |---|---|
//! | `$GP_HOME/work/mechanic-preview/GAME/asks/ID/ask.patch` | The card's patch, copied when first built, so the look outlives the worker's tree |
//! | `.../asks/ID/tree/GAME/` | The game's folder with the patch applied |
//! | `.../asks/ID/assets/` | Its pictures, lifted by the patched asset map |
//! | `.../asks/ID/build/` | The preview, with `module.wasm` and `.key` (the patch's hash) |
//!
//! A theme's pictures are in Git LFS, so a patch shows a changed picture as
//! its pointer (`version`, `oid`, `size`) while the copy holds the picture:
//! each such picture is written as its pointer for the patch to apply, and
//! the new pointer put back as the picture it names, from the games
//! checkout's LFS store, the worker's tree beside the patch, or `git lfs
//! smudge`.
//!
//! A look is built once for its ask, on the patch applied to main as it
//! stood then. gp serve builds the looks of the game's open gate cards with
//! a clip in the background (every half minute, one at a time), so LOOK AT
//! IT seldom waits; a look of an ask no longer open is removed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::http::Response;
use super::review::Rebuild;
use gp_evidence::Paths;

/// Whether `id` can be an ask's id (gp-mcp's asks: letters, digits, `-`).
fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn asks_dir(paths: &Paths, game: &str) -> PathBuf {
    super::live::out_dir(paths, game).join("asks")
}

fn dir(paths: &Paths, game: &str, id: &str) -> PathBuf {
    asks_dir(paths, game).join(id)
}

/// The asks' folder, `$GP_HOME/work/asks`.
fn asks_home(paths: &Paths) -> PathBuf {
    paths.home.join("work").join("asks")
}

fn ask(paths: &Paths, id: &str) -> Option<Value> {
    if !id_ok(id) {
        return None;
    }
    let f = asks_home(paths).join(format!("{id}.json"));
    serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()
}

/// The value after `flag` in the ask's arguments.
fn arg_of(v: &Value, flag: &str) -> Option<String> {
    let argv = v["argv"].as_array()?;
    let i = argv.iter().position(|a| a == flag)?;
    argv.get(i + 1)?.as_str().map(str::to_owned)
}

/// The patch file a model gate's ask for `game` names, or why it has none.
fn patch_of(v: &Value, game: &str) -> std::result::Result<PathBuf, String> {
    if v["command"] != json!("model gate") {
        return Err("this card isn't a model gate's: it lands no change to look at".into());
    }
    if v["game"].as_str() != Some(game) {
        let other = v["game"].as_str().unwrap_or("another game");
        return Err(format!("this card is for {other}, not {game}"));
    }
    match arg_of(v, "--patch") {
        Some(p) => Ok(PathBuf::from(p)),
        None => Err("this gate's card names no --patch".into()),
    }
}

/// Whether ask `v` waits for the person or holds their yes, so its look is
/// worth keeping.
fn live_ask(v: &Value) -> bool {
    matches!(v["state"].as_str(), Some("open" | "yes" | "running"))
}

/// The files a patch changes, by its `diff --git a/X b/Y` lines (both).
fn patch_files(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for l in text.lines() {
        if let Some(rest) = l.strip_prefix("diff --git a/")
            && let Some((a, b)) = rest.split_once(" b/")
        {
            out.push(a.to_owned());
            out.push(b.to_owned());
        }
    }
    out
}

/// Why a patch can't be applied to a copy of `game`'s folder alone: it
/// changes nothing, or a file outside `GAME/`.
fn outside(text: &str, game: &str) -> Option<String> {
    let files = patch_files(text);
    if files.is_empty() {
        return Some("the card's patch changes no file".into());
    }
    let prefix = format!("{game}/");
    for f in files {
        if !f.starts_with(&prefix) || f.contains("..") {
            return Some(format!(
                "the card's patch changes {f}, outside {game}/: its look can't be built from the game's folder"
            ));
        }
    }
    None
}

/// The game's folder copied into `to`, but its build outputs (`target`).
fn copy_game(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let t = e.file_type()?;
        if t.is_dir() {
            copy_game(&e.path(), &to.join(&name))?;
        } else if t.is_file() {
            std::fs::copy(e.path(), to.join(&name))?;
        } else if t.is_symlink() {
            // A link is copied as what it points at, or left out when
            // that is gone.
            if let Ok(b) = std::fs::read(e.path()) {
                std::fs::write(to.join(&name), b)?;
            }
        }
    }
    Ok(())
}

/// The first line of a Git LFS pointer, as `git diff` shows an LFS file.
const LFS_VERSION: &str = "version https://git-lfs.github.com/spec/v1";

/// The files a patch changes as Git LFS pointer text (a theme's pictures,
/// diffed as their pointers): each section's `b/` path, and its `a/` path
/// where that differs.
fn lfs_files(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut lfs = false;
    let mut end = |cur: &mut Vec<String>, lfs: bool| {
        if lfs {
            for f in cur.drain(..) {
                if !out.contains(&f) {
                    out.push(f);
                }
            }
        }
        cur.clear();
    };
    for l in text.lines() {
        if let Some(rest) = l.strip_prefix("diff --git a/") {
            end(&mut cur, lfs);
            lfs = false;
            if let Some((a, b)) = rest.split_once(" b/") {
                cur.push(b.to_owned());
                if a != b {
                    cur.push(a.to_owned());
                }
            }
        } else if l.strip_prefix(['+', '-', ' ']) == Some(LFS_VERSION) {
            lfs = true;
        }
    }
    end(&mut cur, lfs);
    out
}

/// The pointer Git LFS stores for `bytes`.
fn lfs_pointer(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let oid = hex::encode(sha2::Sha256::digest(bytes));
    format!("{LFS_VERSION}\noid sha256:{oid}\nsize {}\n", bytes.len())
}

/// The oid of an LFS pointer's text, when `bytes` is one.
fn lfs_oid(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    if bytes.len() > 400 || !text.starts_with(LFS_VERSION) {
        return None;
    }
    let oid = text.lines().find_map(|l| l.strip_prefix("oid sha256:"))?;
    (oid.len() == 64 && oid.bytes().all(|b| b.is_ascii_hexdigit())).then(|| oid.to_owned())
}

/// The picture an LFS pointer names: from the LFS store of the git
/// repository `main_game` is in, else the file at `rel` beside the card's
/// patch (the worker's tree) when it is that picture, else `git lfs smudge`.
fn lfs_object(main_game: &Path, near: Option<&Path>, rel: &str, pointer: &[u8]) -> Option<Vec<u8>> {
    use sha2::Digest as _;
    let oid = lfs_oid(pointer)?;
    let is = |b: &[u8]| hex::encode(sha2::Sha256::digest(b)) == oid;
    let common = crate::gate::git(main_game, &["rev-parse", "--git-common-dir"]).ok();
    if let Some(c) = common {
        let c = main_game.join(c.trim());
        let f = c
            .join("lfs/objects")
            .join(&oid[..2])
            .join(&oid[2..4])
            .join(&oid);
        if let Ok(b) = std::fs::read(f)
            && is(&b)
        {
            return Some(b);
        }
    }
    if let Some(n) = near
        && let Ok(b) = std::fs::read(n.join(rel))
        && is(&b)
    {
        return Some(b);
    }
    use std::io::Write as _;
    let mut c = std::process::Command::new("git")
        .args(["lfs", "smudge", rel])
        .current_dir(main_game)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    c.stdin.take()?.write_all(pointer).ok()?;
    let o = c.wait_with_output().ok()?;
    (o.status.success() && is(&o.stdout)).then_some(o.stdout)
}

/// Before the patch is applied to the copy in `tree`: each picture it
/// changes as LFS pointer text is written as its pointer, so the pointer
/// lines apply.
fn lfs_to_pointers(tree: &Path, files: &[String]) -> std::io::Result<()> {
    for f in files {
        let p = tree.join(f);
        if let Ok(b) = std::fs::read(&p)
            && lfs_oid(&b).is_none()
        {
            std::fs::write(&p, lfs_pointer(&b))?;
        }
    }
    Ok(())
}

/// After the patch is applied: each pointer it left is put back as the
/// picture it names, or the look is refused naming the first it can't find.
fn lfs_from_pointers(
    tree: &Path,
    files: &[String],
    main_game: &Path,
    near: Option<&Path>,
) -> Result<()> {
    for f in files {
        let p = tree.join(f);
        let Ok(b) = std::fs::read(&p) else {
            continue;
        };
        if lfs_oid(&b).is_none() {
            continue;
        }
        let Some(pic) = lfs_object(main_game, near, f, &b) else {
            bail!(
                "the card's patch changes {f} as a Git LFS pointer, and the picture it names isn't in the games checkout's LFS store or beside the patch"
            );
        };
        std::fs::write(&p, pic)?;
    }
    Ok(())
}

/// One look's build, running or ended.
#[derive(Default)]
struct Build {
    started: Option<std::time::Instant>,
    error: Option<String>,
}

static BUILDS: Mutex<BTreeMap<String, Build>> = Mutex::new(BTreeMap::new());

fn key_of(patch: &[u8]) -> String {
    blake3::hash(patch).to_hex().to_string()
}

/// Whether the look in `d` is built for the patch it keeps.
fn is_built(d: &Path) -> bool {
    let build = d.join("build");
    if !super::live::built(&build) {
        return false;
    }
    let patch = std::fs::read(d.join("ask.patch"));
    let key = std::fs::read_to_string(build.join(".key"));
    matches!((patch, key), (Ok(p), Ok(k)) if k.trim() == key_of(&p))
}

/// `GET /api/rebuild/status?ask=ID`: as main's status
/// ([`super::live::status`]): built, building (seconds), error, and build
/// (the look's key), with `ask` and its `message`.
pub(crate) fn status(paths: &Paths, game: &str, id: &str) -> Value {
    let d = dir(paths, game, id);
    let built = is_built(&d);
    let key = std::fs::read_to_string(d.join("build/.key")).unwrap_or_default();
    let message = ask(paths, id).and_then(|v| arg_of(&v, "--message"));
    let all = BUILDS.lock().expect("builds");
    let b = all.get(id);
    let error = if built {
        None
    } else {
        b.and_then(|b| b.error.clone())
    };
    json!({
        "ask": id,
        "model": true,
        "built": built,
        "stale": false,
        "build": built.then(|| key.trim().to_owned()),
        "building": b.and_then(|b| b.started).map(|t| t.elapsed().as_secs_f64()),
        "error": error,
        "message": message,
    })
}

/// Whether a look is being built.
fn building() -> bool {
    BUILDS
        .lock()
        .expect("builds")
        .values()
        .any(|b| b.started.is_some())
}

/// Starts building the look of ask `id` for `main`'s game, unless it is
/// built or a look builds now (one at a time: each builds a module and a
/// preview). An ask that isn't a model gate's for this game, or whose
/// patch can't be read, is refused at once.
pub(crate) fn build(
    paths: &Paths,
    repo: &Path,
    game: &str,
    main: &Rebuild,
    rom: &std::sync::Arc<Vec<u8>>,
    id: &str,
) -> Result<()> {
    let Some(v) = ask(paths, id) else {
        bail!("there's no card {id}");
    };
    let src = patch_of(&v, game).map_err(anyhow::Error::msg)?;
    let d = dir(paths, game, id);
    if is_built(&d) {
        return Ok(());
    }
    // The card's patch as it was when its look was first asked for: the
    // worker's tree it was written in goes when the worker is done.
    let kept = d.join("ask.patch");
    let text = match std::fs::read(&src) {
        Ok(b) => b,
        Err(e) => match std::fs::read(&kept) {
            Ok(b) => b,
            Err(_) => bail!("reading the card's patch {}: {e}", src.display()),
        },
    };
    if let Some(why) = outside(&String::from_utf8_lossy(&text), game) {
        bail!("{why}");
    }
    {
        let mut all = BUILDS.lock().expect("builds");
        if all.values().any(|b| b.started.is_some()) {
            return Ok(());
        }
        let b = all.entry(id.to_owned()).or_default();
        b.started = Some(std::time::Instant::now());
        b.error = None;
    }
    let (paths, repo, game, id) = (
        paths.clone(),
        repo.to_path_buf(),
        game.to_owned(),
        id.to_owned(),
    );
    let (main_game, rom_id, rom) = (main.game.clone(), main.rom.clone(), rom.clone());
    let near = src.parent().map(Path::to_path_buf);
    std::thread::spawn(move || {
        let near = near.as_deref();
        let r = make(
            &paths, &repo, &game, &main_game, near, rom_id, &rom, &d, &text,
        );
        let mut all = BUILDS.lock().expect("builds");
        let b = all.entry(id).or_default();
        b.started = None;
        b.error = r.err().map(|e| format!("{e:#}"));
    });
    Ok(())
}

/// The card's patch (file `patch`) applied to the copy of `game` in `tree`.
/// A picture it changes as Git LFS pointer text is a pointer while it
/// applies and the picture it names after.
fn apply(
    tree: &Path,
    game: &str,
    main_game: &Path,
    near: Option<&Path>,
    patch: &Path,
) -> Result<()> {
    let lfs = lfs_files(&String::from_utf8_lossy(&std::fs::read(patch)?));
    lfs_to_pointers(tree, &lfs)?;
    // Outside any repository (GIT_CEILING_DIRECTORIES), so git applies the
    // patch to the copy by its paths, GAME/...
    let git = |more: &[&str]| {
        std::process::Command::new("git")
            .args(["apply", "--whitespace=nowarn"])
            .args(more)
            .arg(patch)
            .current_dir(tree)
            .env("GIT_CEILING_DIRECTORIES", tree.parent().unwrap_or(tree))
            .output()
            .context("running git apply")
    };
    let o = git(&[])?;
    if !o.status.success() {
        // The copy is of the checkout as it stands, local changes too. When
        // the patch comes off it cleanly, the copy already holds the whole
        // change, and is the model after it.
        if !git(&["--check", "--reverse"])?.status.success() {
            bail!(
                "the card's patch doesn't apply to {game} in your games checkout as it is now: {}",
                apply_trouble(&String::from_utf8_lossy(&o.stderr))
            );
        }
    }
    lfs_from_pointers(tree, &lfs, main_game, near)
}

/// git apply's complaint in a line: how many files the patch didn't fit and
/// the first two, where git writes two lines for each.
fn apply_trouble(stderr: &str) -> String {
    let files: Vec<&str> = stderr
        .lines()
        .filter_map(|l| {
            l.strip_prefix("error: ")?
                .strip_suffix(": patch does not apply")
        })
        .collect();
    match files.as_slice() {
        [] => stderr
            .lines()
            .next()
            .unwrap_or("git apply failed")
            .trim()
            .to_owned(),
        [one] => format!("{one} differs from what the patch expects"),
        [a, b] => format!("{a} and {b} differ from what the patch expects"),
        [a, b, rest @ ..] => format!(
            "{} files differ from what the patch expects ({a}, {b} and {} more)",
            files.len(),
            rest.len()
        ),
    }
}

/// The look made: the game copied, the patch applied, the module built,
/// the pictures lifted, the preview built.
#[allow(clippy::too_many_arguments)]
fn make(
    paths: &Paths,
    repo: &Path,
    game: &str,
    main_game: &Path,
    near: Option<&Path>,
    rom_id: Option<(std::sync::Arc<Vec<u8>>, String)>,
    rom: &[u8],
    d: &Path,
    patch: &[u8],
) -> Result<()> {
    std::fs::create_dir_all(d)?;
    std::fs::write(d.join("ask.patch"), patch)?;
    let tree = d.join("tree");
    if tree.exists() {
        std::fs::remove_dir_all(&tree)?;
    }
    let copy = tree.join(game);
    if let Err(e) = copy_game(main_game, &copy) {
        bail!("copying {}: {e}", main_game.display());
    }
    apply(&tree, game, main_game, near, &d.join("ask.patch"))?;
    let module = crate::module::built_for(&paths.home, &copy)
        .context("building the changed model's module")?
        .map(|m| m.0);
    let rb = Rebuild {
        game: copy.clone(),
        module: module.clone(),
        follows: false,
        rom: rom_id,
        assets: Some(d.join("assets")),
    };
    let assets = rb.assets_for(paths, rom)?;
    let build = d.join("build");
    std::fs::create_dir_all(&build)?;
    let o = std::process::Command::new("bash")
        .arg(repo.join("scripts/web/build-preview.sh"))
        .arg(&build)
        .arg(&copy)
        .arg(&assets)
        .current_dir(repo)
        .output()
        .context("running build-preview.sh")?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        let tail: Vec<&str> = err.lines().rev().take(6).collect();
        bail!(
            "the changed model's preview didn't build: {}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        );
    }
    let m = build.join("module.wasm");
    match &module {
        Some(src) => {
            std::fs::copy(src, &m)?;
        }
        None if m.exists() => std::fs::remove_file(&m)?,
        None => {}
    }
    std::fs::write(build.join(".key"), key_of(patch))?;
    Ok(())
}

/// What starting the look of ask `id` needs: its patched folder, its
/// module and its pictures, with main's ROM set. Refused until it is built.
pub(crate) fn rebuild(paths: &Paths, game: &str, main: &Rebuild, id: &str) -> Result<Rebuild> {
    if !id_ok(id) {
        bail!("{id:?} isn't a card's id");
    }
    let d = dir(paths, game, id);
    if !is_built(&d) {
        bail!("the look of card {id} isn't built yet");
    }
    let m = d.join("build/module.wasm");
    Ok(Rebuild {
        game: d.join("tree").join(game),
        module: m.is_file().then_some(m),
        follows: false,
        rom: main.rom.clone(),
        assets: Some(d.join("assets")),
    })
}

/// `GET /rebuild-ask/ID/NAME`: a file of the look's preview, as
/// [`super::live::file`] serves main's.
pub(crate) fn file(paths: &Paths, repo: &Path, game: &str, rest: &str) -> Result<Response> {
    let Some((id, name)) = rest.split_once('/').filter(|(id, _)| id_ok(id)) else {
        return Ok(Response::error(404, "not found"));
    };
    let built = dir(paths, game, id).join("build");
    super::live::file_in(paths, repo, game, &built, name)
}

/// Builds the look of one open gate card of `game` with a clip whose look
/// isn't built, when main's preview is built (the screen has been used)
/// and nothing else builds; removes the looks of asks no longer open. gp
/// serve calls it every half minute.
pub(crate) fn refresh(
    paths: &Paths,
    repo: &Path,
    game: &str,
    main: &Rebuild,
    rom: &std::sync::Arc<Vec<u8>>,
) {
    let looks = asks_dir(paths, game);
    for e in std::fs::read_dir(&looks).into_iter().flatten().flatten() {
        let id = e.file_name().to_string_lossy().into_owned();
        let keep = ask(paths, &id).is_some_and(|v| live_ask(&v));
        let mut all = BUILDS.lock().expect("builds");
        let running = all.get(&id).is_some_and(|b| b.started.is_some());
        if !keep && !running {
            let _ = std::fs::remove_dir_all(e.path());
            all.remove(&id);
        }
    }
    if building() || super::live::building() || !super::live::is_built(paths, game) {
        return;
    }
    let asks = asks_home(paths);
    let mut next = None;
    for e in std::fs::read_dir(&asks).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        let Some(v) = ask(paths, id) else {
            continue;
        };
        let failed = BUILDS
            .lock()
            .expect("builds")
            .get(id)
            .is_some_and(|b| b.error.is_some());
        if v["state"] == json!("open")
            && v["clip"].is_object()
            && patch_of(&v, game).is_ok()
            && !failed
            && !is_built(&dir(paths, game, id))
        {
            next = Some(id.to_owned());
            break;
        }
    }
    if let Some(id) = next
        && let Err(e) = build(paths, repo, game, main, rom, &id)
    {
        eprintln!("gp mechanic: the look of card {id} didn't build: {e:#}");
        let mut all = BUILDS.lock().expect("builds");
        all.entry(id).or_default().error = Some(format!("{e:#}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_paths as paths;

    #[test]
    fn a_card_looks_at_its_own_patch_on_its_own_game() {
        let mut v = json!({
            "command": "model gate", "game": "hilltop",
            "argv": ["--patch", "/t/x.patch", "--message", "hilltop: m", "hilltop"],
        });
        let p = patch_of(&v, "hilltop").unwrap();
        assert_eq!(p, PathBuf::from("/t/x.patch"));
        assert_eq!(arg_of(&v, "--message").as_deref(), Some("hilltop: m"));
        assert!(patch_of(&v, "rooftop").unwrap_err().contains("for hilltop"));
        v["command"] = json!("registry publish");
        assert!(patch_of(&v, "hilltop").is_err());
        v["command"] = json!("model gate");
        v["argv"] = json!(["hilltop"]);
        assert!(patch_of(&v, "hilltop").unwrap_err().contains("--patch"));
    }

    #[test]
    fn a_patch_outside_the_games_folder_has_no_look() {
        let ok = "diff --git a/hilltop/model.toml b/hilltop/model.toml\n";
        assert_eq!(outside(ok, "hilltop"), None);
        let other = "diff --git a/rooftop/model.toml b/rooftop/model.toml\n";
        let why = outside(other, "hilltop").unwrap();
        assert!(why.contains("rooftop/model.toml"));
        assert!(outside("", "hilltop").unwrap().contains("no file"));
        let up = "diff --git a/hilltop/../x b/hilltop/../x\n";
        assert!(outside(up, "hilltop").is_some());
    }

    #[test]
    fn a_cards_pictures_in_git_lfs_apply_to_the_real_pictures() {
        let d = tempfile::tempdir().unwrap();
        let games = d.path().join("games");
        let main_game = games.join("hilltop");
        std::fs::create_dir_all(main_game.join("art")).unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&games)
            .status()
            .unwrap();
        assert!(ok.success());
        let (old, new) = (b"old picture".as_slice(), b"new picture!".as_slice());
        std::fs::write(main_game.join("art/mug.png"), old).unwrap();
        std::fs::write(main_game.join("model.toml"), "a = 1\n").unwrap();
        // The new picture is in the games checkout's LFS store, as git
        // diff's clean filter left it.
        let oid = |b: &[u8]| lfs_oid(lfs_pointer(b).as_bytes()).unwrap();
        let o = oid(new);
        let store = games.join(".git/lfs/objects").join(&o[..2]).join(&o[2..4]);
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join(&o), new).unwrap();
        let (po, pn) = (lfs_pointer(old), lfs_pointer(new));
        let (lo, ln): (Vec<&str>, Vec<&str>) = (po.lines().collect(), pn.lines().collect());
        let patch = format!(
            "diff --git a/hilltop/art/mug.png b/hilltop/art/mug.png\nindex e412e7d..64eff61 100644\n--- a/hilltop/art/mug.png\n+++ b/hilltop/art/mug.png\n@@ -1,3 +1,3 @@\n {}\n-{}\n-{}\n+{}\n+{}\ndiff --git a/hilltop/model.toml b/hilltop/model.toml\nindex 1111111..2222222 100644\n--- a/hilltop/model.toml\n+++ b/hilltop/model.toml\n@@ -1 +1 @@\n-a = 1\n+a = 2\n",
            lo[0], lo[1], lo[2], ln[1], ln[2]
        );
        assert_eq!(lfs_files(&patch), vec!["hilltop/art/mug.png".to_owned()]);
        let look = d.path().join("look");
        let tree = look.join("tree");
        copy_game(&main_game, &tree.join("hilltop")).unwrap();
        std::fs::write(look.join("ask.patch"), &patch).unwrap();
        apply(&tree, "hilltop", &main_game, None, &look.join("ask.patch")).unwrap();
        let got = std::fs::read(tree.join("hilltop/art/mug.png")).unwrap();
        assert_eq!(got, new);
        let m = std::fs::read_to_string(tree.join("hilltop/model.toml")).unwrap();
        assert_eq!(m, "a = 2\n");
        // A picture the store hasn't got (here, bytes that aren't it) is
        // refused by its name.
        std::fs::write(store.join(&o), b"something else").unwrap();
        let tree = look.join("tree2");
        copy_game(&main_game, &tree.join("hilltop")).unwrap();
        let e = apply(&tree, "hilltop", &main_game, None, &look.join("ask.patch"));
        assert!(format!("{:#}", e.unwrap_err()).contains("hilltop/art/mug.png"));
    }

    /// A checkout that already holds a card's whole change (a turned-down
    /// import left in place) is the model after it; one that holds something
    /// else is refused in a line, however many files git names.
    #[test]
    fn a_copy_that_already_holds_the_change_is_the_look() {
        let d = tempfile::tempdir().unwrap();
        let tree = d.path().join("tree");
        std::fs::create_dir_all(tree.join("hilltop")).unwrap();
        let mut patch = String::new();
        for k in 0..4 {
            std::fs::write(tree.join(format!("hilltop/f{k}.txt")), "new\n").unwrap();
            patch += &format!(
                "diff --git a/hilltop/f{k}.txt b/hilltop/f{k}.txt\n--- a/hilltop/f{k}.txt\n+++ b/hilltop/f{k}.txt\n@@ -1 +1 @@\n-old\n+new\n"
            );
        }
        let p = d.path().join("ask.patch");
        std::fs::write(&p, &patch).unwrap();
        let main_game = d.path().join("games/hilltop");
        apply(&tree, "hilltop", &main_game, None, &p).unwrap();
        assert_eq!(
            std::fs::read_to_string(tree.join("hilltop/f0.txt")).unwrap(),
            "new\n"
        );
        for k in 0..4 {
            std::fs::write(tree.join(format!("hilltop/f{k}.txt")), "other\n").unwrap();
        }
        let e = format!(
            "{:#}",
            apply(&tree, "hilltop", &main_game, None, &p).unwrap_err()
        );
        assert!(e.contains("4 files differ"), "{e}");
        assert!(
            e.contains("hilltop/f0.txt, hilltop/f1.txt and 2 more"),
            "{e}"
        );
        assert_eq!(e.lines().count(), 1, "{e}");
    }

    #[test]
    fn a_look_is_built_for_the_patch_it_keeps() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let look = dir(&p, "hilltop", "1-abc");
        let build = look.join("build");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("gp_web.wasm"), b"").unwrap();
        std::fs::write(build.join("bundle.json"), b"{}").unwrap();
        std::fs::write(look.join("ask.patch"), b"one").unwrap();
        assert!(!is_built(&look));
        std::fs::write(build.join(".key"), key_of(b"one")).unwrap();
        assert!(is_built(&look));
        std::fs::write(look.join("ask.patch"), b"two").unwrap();
        assert!(!is_built(&look));
        assert_eq!(status(&p, "hilltop", "1-abc")["built"], json!(false));
    }

    #[test]
    fn a_looks_files_are_its_builds() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let repo = d.path().join("repo");
        std::fs::create_dir_all(repo.join("scripts/web/page")).unwrap();
        std::fs::write(repo.join("scripts/web/page/index.html"), "page").unwrap();
        let build = dir(&p, "hilltop", "1-abc").join("build");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("bundle.json"), "{\"x\":1}").unwrap();
        let got = |n: &str| file(&p, &repo, "hilltop", n).unwrap().body;
        assert_eq!(got("1-abc/bundle.json"), b"{\"x\":1}");
        assert_eq!(got("1-abc/index.html"), b"page");
        let bad = ["../x/a.json", "1-abc", "a b/c", "1-abc/../../x"];
        for b in bad {
            assert_eq!(file(&p, &repo, "hilltop", b).unwrap().status, 404, "{b}");
        }
    }
}
