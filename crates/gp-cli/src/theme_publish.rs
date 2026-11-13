//! `gp theme publish`, `gp theme fetch` and `gp theme list`
//! (docs/theme-publishing.md): a theme version as a package
//! (gp_package, docs/theme-packages.md), checked, signed, kept in the local
//! store, copied to the mirrors and listed in its game's themes index,
//! `themes/index/<game>.toml`, through the gate. A game is a template's
//! `template.game` (Donkey Kong JU and Original Edition are both
//! `donkey-kong`), else its id.
//!
//! Two kinds of package: a look package (`look/`, drawn by gp) for a game
//! with no ROM art, and a source package (the theme's own files and
//! checks.json, nothing drawn) for a game lifted from the player's ROM,
//! whose runtime draws the theme over the player's own lifted art.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::theme::RunRecorder;
use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use gp_package::index::{self, Index, Theme as Entry};
use gp_package::{Manifest, Store, blob_key, package_key, signature_key, themes_index_key};
use serde_json::{Value, json};

use crate::Outcome;
use crate::theme::ThemeArgs;
use crate::theme_mirror::{self as mirror, Mirror};

/// The SSH signature namespace for packages (docs/research/package-hosting.md).
pub const NAMESPACE: &str = "game-printer-package";

/// The repo's list of who may sign packages, in `ssh-keygen`'s
/// allowed_signers format; `$GP_HOME/allowed_signers` is read first.
pub const ALLOWED_SIGNERS: &str = "themes/allowed_signers";

/// The largest file a package holds, as the pre-commit hook's limit.
const MAX_FILE: u64 = 1 << 20;

/// What a theme package may hold, by extension: text, and the pictures
/// and sound a look is drawn into.
const KINDS: &[&str] = &["toml", "pix", "tune", "txt", "md", "json", "png", "wav"];

#[derive(clap::Args, Debug)]
pub struct PublishArgs {
    /// The theme's folder (holding theme.toml).
    pub theme: PathBuf,
    /// The game: a game of the games checkout, or a template's id.
    #[arg(long)]
    pub game: String,
    /// The game's assets (default: $GP_HOME/work/assets/<source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
    /// The given template's ROM, for the originality and ROM-bytes checks
    /// (default: the first file under $GP_ROM_DIR the template accepts; an
    /// original's own image is built). The other models in the theme's
    /// `for` are checked with the ROM their templates accept in
    /// $GP_ROM_DIR, and recorded as not checked when it isn't there.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// The SSH private key to sign with (default: $GP_SIGNING_KEY, else
    /// git's user.signingkey).
    #[arg(long, value_name = "FILE")]
    pub key: Option<PathBuf>,
    /// Who signs, as allowed_signers names them (default: $GP_SIGNER,
    /// else git's user.email).
    #[arg(long)]
    pub signer: Option<String>,
    /// The index file to add the theme to (default: the repo's
    /// themes/index/<game>.toml).
    #[arg(long, value_name = "FILE")]
    pub index: Option<PathBuf>,
    /// The template whose gate lands the index change.
    #[arg(long, default_value = "donkey-kong")]
    pub gate: PathBuf,
    /// Write the index file in place instead of landing it through the
    /// gate (for a fork's own index, or a test's).
    #[arg(long)]
    pub no_gate: bool,
    /// Keep the package in the local store only: copy nothing to the
    /// mirrors and leave the index as it is.
    #[arg(long)]
    pub local: bool,
    /// Run every check and the package scan, then print the files the
    /// package would hold, the check results and the index line it would
    /// write, and stop: nothing goes to the store, the mirrors or the
    /// index, and no key is needed.
    #[arg(long, conflicts_with = "local")]
    pub dry_run: bool,
}

#[derive(clap::Args, Debug)]
pub struct RecheckArgs {
    /// The game: a template's id, or a game (template.game).
    pub game: String,
    /// The theme's id in the game's index.
    pub theme: String,
    /// The model to check, one in the theme's `for` (default: the model
    /// whose template accepts --rom, else each model the index doesn't
    /// record as passed whose ROM is in $GP_ROM_DIR).
    #[arg(long)]
    pub model: Option<String>,
    /// The ROM to check with; its template must accept it (default: the
    /// first file in $GP_ROM_DIR the model's template accepts).
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// The model's assets, lifted from the ROM (default: lifted from it
    /// here into a temporary folder). Needs --model or --rom.
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
    /// The index file (default: the repo's themes/index/<game>.toml).
    #[arg(long, value_name = "FILE")]
    pub index: Option<PathBuf>,
    /// The template whose gate lands the index change.
    #[arg(long, default_value = "donkey-kong")]
    pub gate: PathBuf,
    /// Write the index file in place instead of landing it through the
    /// gate (for a fork's own index, or a test's).
    #[arg(long)]
    pub no_gate: bool,
    /// Run the checks and print the index line it would write, and stop:
    /// nothing goes to the index or the mirrors.
    #[arg(long)]
    pub dry_run: bool,
}

/// Where the themes index lives: the games checkout, which holds the
/// games' folders (game-printer's was its own repo, with `templates/`).
fn repo_root() -> Result<PathBuf> {
    crate::theme_template::games_root().context(
        "no games checkout: run gp theme publish, fetch and list inside it, or set GP_GAMES_DIR",
    )
}

fn template_dir(_repo: &Path, game: &str) -> Result<PathBuf> {
    crate::theme_template::find_game(game)
        .with_context(|| format!("no game `{game}` in the games checkout (GAME/template.toml)"))
}

/// The game whose themes index lists a template's themes: its
/// `template.game`, else its id. A name that is no template's is a game
/// already.
pub(crate) fn family(_repo: &Path, name: &str) -> String {
    crate::theme_template::find_game(name)
        .and_then(|d| crate::theme_slots::game_of(&d))
        .unwrap_or_else(|| name.to_owned())
}

fn index_path(repo: &Path, game: &str, given: Option<&Path>) -> PathBuf {
    given
        .map(Path::to_owned)
        .unwrap_or_else(|| repo.join(index::REPO_DIR).join(format!("{game}.toml")))
}

fn read_index(path: &Path, game: &str) -> Result<Index> {
    match fs::read_to_string(path) {
        Ok(t) => Index::read(&t).map_err(|e| anyhow::anyhow!("{}: {e}", path.display())),
        Err(_) => Ok(Index::new(game)),
    }
}

fn git_config(key: &str) -> Option<String> {
    let o = Command::new("git")
        .args(["config", "--get", key])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_owned();
    (o.status.success() && !s.is_empty()).then_some(s)
}

/// Every file under a folder, by its path relative to it, `/` between
/// parts, sorted; dot files left out.
fn files_under(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| format!("reading {}", d.display()))? {
            let p = e?.path();
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p
                    .strip_prefix(dir)?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((rel, p));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The package scan: each file's kind, size and contents (docs/theme-packages.md,
/// "The scan"). The ROM-bytes checks run after it, on the staged folder.
pub(crate) fn scan(files: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, b) in files {
        let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let Some(ext) = ext.filter(|e| KINDS.contains(&e.as_str())) else {
            problems.push(format!(
                "{path}: a package holds only {} files",
                KINDS.join(", ")
            ));
            continue;
        };
        if b.len() as u64 > MAX_FILE {
            problems.push(format!(
                "{path}: {} bytes, over the {MAX_FILE} a file may have",
                b.len()
            ));
        }
        if b.starts_with(b"NES\x1a") {
            problems.push(format!("{path}: starts with an iNES header: a game image"));
        }
        let ok = match ext.as_str() {
            "png" => b.starts_with(b"\x89PNG\r\n\x1a\n"),
            "wav" => b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WAVE"),
            _ => std::str::from_utf8(b).is_ok(),
        };
        if !ok {
            problems.push(format!("{path}: isn't the {ext} file its name says"));
        }
    }
    problems
}

/// The size of a PNG from its header.
fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    let w = u32::from_be_bytes(b.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(b.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

/// Signs a file with an SSH key: the signature's text.
fn sign(key: &Path, file: &Path) -> Result<Vec<u8>> {
    let o = Command::new("ssh-keygen")
        .args(["-q", "-Y", "sign", "-n", NAMESPACE, "-f"])
        .arg(key)
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .context("running ssh-keygen")?;
    if !o.status.success() {
        bail!(
            "signing with {}: {}",
            key.display(),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    let sig = PathBuf::from(format!("{}.sig", file.display()));
    let b = fs::read(&sig).with_context(|| format!("reading {}", sig.display()))?;
    Ok(b)
}

/// The allowed signers files, the person's first.
fn allowed_signers(home: &Path, repo: &Path) -> Vec<PathBuf> {
    [home.join("allowed_signers"), repo.join(ALLOWED_SIGNERS)]
        .into_iter()
        .filter(|p| p.is_file())
        .collect()
}

/// Checks a signature of `data` by `signer` against the allowed signers.
fn verify(allowed: &[PathBuf], signer: &str, sig: &[u8], data: &[u8]) -> Result<()> {
    if allowed.is_empty() {
        bail!(
            "no allowed_signers file ($GP_HOME/allowed_signers or {ALLOWED_SIGNERS}) to check `{signer}` against"
        );
    }
    let tmp = tempfile::tempdir()?;
    let sig_file = tmp.path().join("sig");
    fs::write(&sig_file, sig)?;
    let mut last = String::new();
    for a in allowed {
        let mut child = Command::new("ssh-keygen")
            .args(["-Y", "verify", "-n", NAMESPACE, "-I", signer, "-f"])
            .arg(a)
            .arg("-s")
            .arg(&sig_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("running ssh-keygen")?;
        if let Some(mut s) = child.stdin.take() {
            std::io::Write::write_all(&mut s, data)?;
        }
        let o = child.wait_with_output()?;
        if o.status.success() {
            return Ok(());
        }
        last = String::from_utf8_lossy(&o.stderr).trim().to_owned();
    }
    bail!("the signature isn't `{signer}`'s by the allowed signers: {last}")
}

/// The themes a game ships with: each theme folder of the template and of
/// the other templates of its game, as (id, folder, the folder its files
/// are in), a `same_as.toml` pointer followed.
fn bundled(tdir: &Path) -> Vec<(String, PathBuf, PathBuf)> {
    let mut dirs = vec![tdir.to_owned()];
    dirs.extend(
        crate::theme_slots::same_game(tdir)
            .into_iter()
            .map(|(d, _)| d),
    );
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(d.join("themes")) else {
            continue;
        };
        let mut folders: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        folders.sort();
        for f in folders {
            if !gp_theme::same_as::is_theme_dir(&f) {
                continue;
            }
            let Ok(target) = gp_theme::same_as::target(&f) else {
                continue;
            };
            let id = fs::read_to_string(target.join(gp_theme::format::THEME_FILE))
                .ok()
                .and_then(|t| t.parse::<toml::Table>().ok())
                .and_then(|t| t.get("id")?.as_str().map(str::to_owned));
            if let Some(id) = id {
                out.push((id, f, target));
            }
        }
    }
    out
}

/// A refusal before anything is checked: the id is a bundled theme's,
/// and the theme isn't that one.
fn id_taken(tdir: &Path, repo: &Path, id: &str, theme_dir: &Path) -> Option<String> {
    let me = theme_dir.canonicalize().ok();
    let (_, folder, _) = bundled(tdir)
        .into_iter()
        .find(|(i, _, target)| i == id && target.canonicalize().ok() != me)?;
    let shown = folder.strip_prefix(repo).unwrap_or(&folder);
    Some(format!(
        "the id `{id}` is taken: {} ships with the game under that id, and a player's list would show only one of the two. Give this theme an id of its own in its theme.toml, such as `{id}-2`",
        shown.display()
    ))
}

/// The digest of a drawn look: the BLAKE3 hash of its files' hashes and
/// paths, a line each, sorted. Drawing is deterministic, so a checker with
/// the same ROM and gp draws the same digest.
fn look_digest(look_dir: &Path) -> Result<String> {
    let mut s = String::new();
    for (rel, p) in files_under(look_dir)? {
        let _ = writeln!(s, "{} {rel}", gp_package::hash(&fs::read(&p)?));
    }
    Ok(gp_package::hash(s.as_bytes()))
}

/// A line for the index from a check's outcome.
fn check_line(pass: bool, what: String) -> String {
    format!("{}: {what}", if pass { "pass" } else { "refused" })
}

/// One model's checks, run on this machine: the theme check with
/// originality, and, when they pass, the look drawn over the model's
/// lifted assets.
struct ModelRun {
    model: String,
    /// The template's id.
    template: String,
    tdir: PathBuf,
    /// The ROM the checks ran with, when one was given or found.
    rom: Option<PathBuf>,
    rom_hash: Option<String>,
    /// The model's slot fingerprint.
    slots: String,
    out: Outcome,
    /// None when the checks refused the theme.
    look: Option<Look>,
    /// The assets lifted here for this model, kept while it is used.
    lifted: Option<tempfile::TempDir>,
}

/// A look drawn for one model.
struct Look {
    /// The look's digest ([`look_digest`]).
    hash: String,
    /// The build's pictures, in order.
    pictures: Vec<String>,
    /// The look's files, by path in the look, and where they are.
    drawn: Vec<(String, PathBuf)>,
    coverage: Value,
    /// The coverage in words.
    line: String,
    _dir: tempfile::TempDir,
}

/// Runs the checks of the theme in `theme_dir` for the template `tdir`
/// (id `template`), with its lifted assets and ROM (each the template's
/// default when None), and draws its look when they pass.
fn check_model(
    theme_dir: &Path,
    tdir: &Path,
    template: &str,
    assets: Option<&Path>,
    rom: Option<&Path>,
    (work, paths): (&Path, &Paths),
    rec: &mut RunRecorder,
) -> Result<ModelRun> {
    let (model, _) = crate::theme::read_model(tdir)?;
    let kit = crate::theme_kit::kit_of(&model, template);
    let found = crate::theme_template::find_rom(&crate::theme_template::load(tdir)?, rom, paths)?;
    let targs = ThemeArgs {
        theme_dirs: vec![theme_dir.to_owned()],
        ..ThemeArgs::default()
    };
    let mut out = crate::theme::check(tdir, &targs, assets, paths, rec)?;
    if out.json.get("readability").is_some() {
        crate::theme::originality(tdir, &targs, assets, rom, paths, rec, &mut out)?;
    }
    let mut run = ModelRun {
        model: model.model.name.clone(),
        template: template.to_owned(),
        tdir: tdir.to_owned(),
        rom: found.as_ref().map(|r| r.path.clone()),
        rom_hash: found.map(|r| r.sha256),
        slots: kit.slots.clone(),
        out,
        look: None,
        lifted: None,
    };
    if run.out.exit != 0 {
        return Ok(run);
    }

    // The look: the theme drawn for the model's build, as the web preview
    // and the Bevy build find looks (gp_look::looks). A look package
    // holds it; a source package holds only its digest, since it is drawn
    // over the maker's own lifted art.
    let prep_tmp = tempfile::tempdir_in(work)?;
    let prep = prep_tmp.path().to_owned();
    let model_file = tdir.join(gp_model::MODEL_FILE);
    let adir = match assets {
        Some(d) => d.to_owned(),
        None => paths
            .home
            .join("work")
            .join("assets")
            .join(model.model.source.as_deref().unwrap_or(template)),
    };
    let places = gp_theme::stack::Places::for_model(tdir, Some(&paths.home));
    let own = gp_theme::stack::resolve(
        &places
            .default_stack(&run.model)
            .map_err(anyhow::Error::msg)?,
        &places,
        &run.model,
    )
    .map_err(anyhow::Error::msg)?;
    let spec = gp_look::looks::LookSpec::theme(theme_dir).map_err(anyhow::Error::msg)?;
    gp_web_lift::prep(
        &model_file,
        &adir,
        &prep,
        &own,
        Some(&paths.home),
        &[spec],
        false,
    )
    .map_err(|e| anyhow::anyhow!("drawing the look for {}: {e}", run.model))?;
    let preview: Value = serde_json::from_slice(&fs::read(prep.join("preview.json"))?)?;
    let pictures: Vec<String> = preview["pictures"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(str::to_owned))
        .collect();
    let id = gp_theme::stack::load(theme_dir, gp_theme::Scope::Personal)
        .map_err(anyhow::Error::msg)?
        .theme
        .id;
    let look_dir = prep.join(gp_present::looks::THEMES_DIR).join(&id);
    if !look_dir.join(gp_present::looks::LOOK_FILE).is_file() {
        bail!(
            "the look for {} wasn't drawn: {}",
            run.model,
            prep.display()
        );
    }
    let drawn = files_under(&look_dir)?;
    let hash = look_digest(&look_dir)?;
    // How much of the game the theme draws: the build's pictures its look
    // changes, and the sound roles it gives a sound of its own.
    let changed = drawn
        .iter()
        .filter(|(rel, _)| pictures.contains(rel))
        .count();
    let sounds = drawn
        .iter()
        .filter(|(rel, _)| rel.ends_with(".wav"))
        .count();
    let coverage = json!({
        "pictures": changed, "of_pictures": pictures.len(),
        "sounds": sounds, "of_sounds": kit.sound.len(),
    });
    let line = format!(
        "{changed} of {} pictures, {sounds} of {} sound roles",
        pictures.len(),
        kit.sound.len()
    );
    run.look = Some(Look {
        hash,
        pictures,
        drawn,
        coverage,
        line,
        _dir: prep_tmp,
    });
    Ok(run)
}

/// A template's assets lifted from a ROM into a new folder under `work`,
/// as `gp assets lift` writes them.
fn lift_into(
    t: &crate::theme_template::Template,
    rom: &[u8],
    work: &Path,
) -> Result<tempfile::TempDir> {
    let map_text = fs::read_to_string(t.dir.join(&t.manifest.recipe.asset_map))
        .context("reading its asset map")?;
    let map = gp_assets::AssetMap::parse(&map_text).map_err(anyhow::Error::msg)?;
    let (files, _) = gp_assets::lift(rom, &map, &map_text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let dir = tempfile::tempdir_in(work)?;
    for f in files {
        let p = dir.path().join(&f.name);
        fs::create_dir_all(p.parent().unwrap_or(dir.path()))?;
        fs::write(&p, &f.bytes)?;
    }
    Ok(dir)
}

/// Runs the checks of the theme in `theme_dir` for another template of
/// the game, `d`: with the ROM given, else the first in $GP_ROM_DIR the
/// template accepts, over `assets` or else assets lifted from that ROM
/// here. `Err(why)` in plain words when it can't be checked on this
/// machine.
fn check_other(
    theme_dir: &Path,
    d: &Path,
    rom: Option<&Path>,
    assets: Option<&Path>,
    (work, paths): (&Path, &Paths),
    rec: &mut RunRecorder,
) -> Result<std::result::Result<ModelRun, String>> {
    let t = crate::theme_template::load(d)?;
    let tid = t.manifest.template.id.clone();
    let found = match rom {
        Some(p) => {
            let r = crate::theme_template::Rom::read(p)?;
            if !t.manifest.source.accepts.contains(&r.sha256) {
                return Ok(Err(format!(
                    "{} isn't a ROM templates/{tid}/template.toml accepts (its SHA-256 is {})",
                    p.display(),
                    r.sha256
                )));
            }
            Some(r)
        }
        None => crate::theme_template::find_rom(&t, None, paths)?,
    };
    let Some(rom) = found else {
        return Ok(Err(format!(
            "no {tid} ROM on this machine: no file in {} has a hash templates/{tid}/template.toml accepts (gp roms add puts one there, docs/sources.md)",
            paths.roms.display()
        )));
    };
    if let Some(a) = assets {
        return Ok(Ok(check_model(
            theme_dir,
            d,
            &tid,
            Some(a),
            Some(&rom.path),
            (work, paths),
            rec,
        )?));
    }
    match lift_into(&t, &rom.bytes, work) {
        Err(e) => Ok(Err(format!(
            "the assets of {tid} couldn't be lifted from {}: {e:#}",
            rom.path.display()
        ))),
        Ok(lifted) => {
            let mut r = check_model(
                theme_dir,
                d,
                &tid,
                Some(lifted.path()),
                Some(&rom.path),
                (work, paths),
                rec,
            )?;
            r.lifted = Some(lifted);
            Ok(Ok(r))
        }
    }
}

/// What the ROM-bytes checks found on a package's files.
#[derive(Default)]
struct RomBytes {
    problems: Vec<String>,
    /// The models a ROM's bytes were found for.
    fails: BTreeMap<String, Vec<&'static str>>,
    /// `<template> against <ROM file>`, or that it had no ROM, with the
    /// models it was checked for.
    lines: Vec<(String, Vec<String>)>,
}

/// The ROM-bytes checks, on the files as staged: for each model checked,
/// against its template's ROM and the ROM of the template it is derived
/// from, each template once.
fn rom_bytes(
    runs: &[ModelRun],
    files: &BTreeMap<String, Vec<u8>>,
    work: &Path,
    paths: &Paths,
) -> Result<RomBytes> {
    let stage_tmp = tempfile::tempdir_in(work)?;
    let stage = stage_tmp.path().to_owned();
    for (rel, b) in files {
        let p = stage.join(rel);
        fs::create_dir_all(p.parent().unwrap_or(&stage))?;
        fs::write(&p, b)?;
    }
    // (template folder, its ROM if known, the models it is checked for)
    let mut templates: Vec<(PathBuf, Option<PathBuf>, Vec<String>)> = Vec::new();
    for r in runs {
        let mut ts = vec![(r.tdir.clone(), r.rom.clone())];
        if let Some(from) = derived_from(&r.tdir) {
            ts.push((r.tdir.join(from), None));
        }
        for (d, rom) in ts {
            let key = d.canonicalize().unwrap_or(d);
            match templates.iter_mut().find(|x| x.0 == key) {
                Some(x) => {
                    x.2.push(r.model.clone());
                    if x.1.is_none() {
                        x.1 = rom;
                    }
                }
                None => templates.push((key, rom, vec![r.model.clone()])),
            }
        }
    }
    let mut out = RomBytes::default();
    for (t, rom, models) in &templates {
        let s =
            crate::theme_template::served(&stage, std::slice::from_ref(t), rom.as_deref(), paths)?;
        for p in s.json["problems"].as_array().into_iter().flatten() {
            out.problems.push(format!(
                "ROM bytes check for {}: {}: {}",
                models.join(", "),
                p["file"].as_str().unwrap_or(""),
                p["message"].as_str().unwrap_or("")
            ));
            for m in models {
                out.fails.insert(m.clone(), vec!["ROM bytes"]);
            }
        }
        // `<template>: N files, B bytes, against ROM (...)`, or no ROM.
        out.lines.extend(s.text.lines().filter_map(|l| {
            if let Some((t, rest)) = l.split_once(": ")
                && let Some((_, rom)) = rest.split_once(" against ")
            {
                let name = Path::new(rom.rsplit_once(" (").map_or(rom, |(r, _)| r));
                return Some((
                    format!(
                        "{t} against {}",
                        name.file_name().map_or(rom.into(), |n| n.to_string_lossy())
                    ),
                    models.clone(),
                ));
            }
            l.contains("no accepted ROM").then(|| {
                (
                    format!("{} not checked (no ROM)", l.split(':').next().unwrap_or("")),
                    models.clone(),
                )
            })
        }));
    }
    Ok(out)
}

/// A source package holds no file gp drew, for any model checked: each
/// file with the same bytes as a file of a drawn look.
fn drawn_copies(runs: &[ModelRun], files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<String>> {
    let mut by_hash = BTreeMap::new();
    for r in runs {
        for (rel, p) in r.look.iter().flat_map(|l| &l.drawn) {
            by_hash.insert(
                gp_package::hash(&fs::read(p)?),
                format!("{rel} of {}", r.model),
            );
        }
    }
    Ok(files
        .iter()
        .filter_map(|(path, b)| {
            by_hash.get(&gp_package::hash(b)).map(|rel| {
                format!(
                    "{path}: the same bytes as {rel}, a file gp drew over the game's art; a ROM theme's package holds only the theme's own files"
                )
            })
        })
        .collect())
}

/// The readability check's line: `pass: ok`, or `pass with a warning:`
/// and what it warned of.
fn readability_line(r: &Value) -> String {
    let verdict = r["verdict"].as_str().unwrap_or("ok");
    if verdict != "warn" {
        return check_line(true, verdict.to_owned());
    }
    let warned = |v: &Value| v["verdict"] == "warn";
    let mut why: Vec<String> = r["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|g| warned(g))
        .map(|g| {
            format!(
                "{} ({})",
                g["group"].as_str().unwrap_or(""),
                g["why"].as_str().unwrap_or("hard to tell apart")
            )
        })
        .collect();
    why.extend(
        r["pairs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| warned(p))
            .map(|p| {
                format!(
                    "{} and {} close in color (delta E {:.1})",
                    p["a"].as_str().unwrap_or(""),
                    p["b"].as_str().unwrap_or(""),
                    p["delta_e"].as_f64().unwrap_or(0.0)
                )
            }),
    );
    format!("pass with a warning: {}", why.join("; "))
}

/// A model that passed its checks here: its record for checks.json, its
/// readability and originality lines, and its line of output.
fn passed(r: &ModelRun, rom_lines: &[(String, Vec<String>)]) -> Result<(Value, String, String)> {
    let look = r.look.as_ref().context("the look wasn't drawn")?;
    let readability = readability_line(&r.out.json["readability"]);
    let (art, shape, melody) = top_scores(&r.out.json["originality"]);
    let mine: Vec<&str> = rom_lines
        .iter()
        .filter(|(_, ms)| ms.contains(&r.model))
        .map(|(l, _)| l.as_str())
        .collect();
    let originality = check_line(
        true,
        format!(
            "art at most {art:.3}, shape {shape:.3}, melody {melody:.3}; no ROM bytes: {}",
            mine.join(", ")
        ),
    );
    let geo = self::geometry(&r.out.json);
    let record = json!({
        "result": "pass", "template": r.template, "slots": r.slots,
        "rom_sha256": r.rom_hash, "look": look.hash, "coverage": look.coverage,
        "readability": readability, "originality": originality, "geometry": geo,
    });
    Ok((record, readability, originality))
}

/// The output line of a model's checks.
fn checks_text(model: &str, record: &Value) -> String {
    format!(
        "checks on {model}: readability {}; originality {}; geometry {}",
        record["readability"].as_str().unwrap_or(""),
        record["originality"].as_str().unwrap_or(""),
        record["geometry"].as_str().unwrap_or("")
    )
}

/// Lands a game's new index text: in place with `no_gate`, else as a
/// patch through the gate of the template `gate`; then, once it has
/// landed, its copy on each writable mirror. Whether it landed, and a
/// line saying how.
#[allow(clippy::too_many_arguments)]
fn land_index(
    repo: &Path,
    ipath: &Path,
    new_index: &str,
    (no_gate, gate): (bool, &Path),
    msg: &str,
    work: &Path,
    tag: &str,
    (game, mirrors, home): (&str, &[&Mirror], &Path),
) -> Result<(bool, String)> {
    let (landed, line) = if no_gate {
        if let Some(d) = ipath.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(ipath, new_index)?;
        (true, format!("index {} written", ipath.display()))
    } else {
        let rel = ipath
            .strip_prefix(repo)
            .context("the index given with --index is outside the repo: pass --no-gate")?;
        let old = fs::read_to_string(ipath).unwrap_or_default();
        let patch = whole_file_patch(&rel.to_string_lossy(), &old, new_index);
        let pfile = work.join(format!("index-{tag}.patch"));
        fs::write(&pfile, patch)?;
        let exe = std::env::current_exe()?;
        let g = Command::new(exe)
            .current_dir(repo)
            .args(["model", "gate"])
            .arg(gate)
            .arg("--patch")
            .arg(&pfile)
            .arg("-m")
            .arg(msg)
            .output()
            .context("running the gate")?;
        let gout = String::from_utf8_lossy(&g.stdout);
        let last = gout
            .lines()
            .rev()
            .find(|l| l.contains("PASS") || l.contains("FAIL"))
            .unwrap_or("");
        (g.status.success(), format!("gate: {last}"))
    };
    if landed {
        for mi in mirrors {
            mi.put(home, &themes_index_key(game), new_index.as_bytes(), true)?;
        }
    }
    Ok((landed, line))
}

/// A template's `[derive] from`, the template it is derived from.
fn derived_from(tdir: &Path) -> Option<String> {
    let t: toml::Table = fs::read_to_string(tdir.join("template.toml"))
        .ok()?
        .parse()
        .ok()?;
    t.get("derive")?.get("from")?.as_str().map(str::to_owned)
}

/// The checks a theme check's outcome refused, by name.
fn failed_checks(j: &Value) -> Vec<&'static str> {
    if j.get("readability").is_none() {
        return vec!["slot names"];
    }
    let mut f = Vec::new();
    if j["readability"]["verdict"] == "refuse" {
        f.push("readability");
    }
    if j["style"]["failed"].as_u64().unwrap_or(0) > 0 {
        f.push("Style");
    }
    let any = |v: &Value, k: &str, want: &Value| {
        v.as_array()
            .is_some_and(|a| a.iter().any(|x| x[k] == *want))
    };
    if any(&j["geometry"]["flags"], "verdict", &json!("refuse")) {
        f.push("geometry");
    }
    if any(&j["originality"]["scores"], "refused", &json!(true)) {
        f.push("originality");
    }
    if j["originality"]["rom_files"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        f.push("ROM bytes");
    }
    if f.is_empty() {
        f.push("theme");
    }
    f
}

/// The highest judged originality scores: art, shape and melody.
fn top_scores(o: &Value) -> (f64, f64, f64) {
    let max_of = |f: &dyn Fn(&Value) -> Option<f64>| {
        o["scores"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["judged"].as_bool() == Some(true))
            .filter_map(f)
            .fold(0.0, f64::max)
    };
    let art = max_of(&|s| s["shape"].as_f64().and(s["value"].as_f64()));
    let shape = max_of(&|s| s["shape"].as_f64());
    let melody = max_of(&|s| {
        s["what"]
            .as_str()
            .is_some_and(|w| w.starts_with("sound"))
            .then(|| s["value"].as_f64())
            .flatten()
    });
    (art, shape, melody)
}

pub(crate) fn publish(a: &PublishArgs, paths: &Paths, rec: &mut RunRecorder) -> Result<Outcome> {
    let repo = repo_root()?;
    let tdir = template_dir(&repo, &a.game)?;
    let ttoml: toml::Table = fs::read_to_string(tdir.join("template.toml"))?.parse()?;
    let how = ttoml
        .get("source")
        .and_then(|s| s.get("how"))
        .and_then(|h| h.as_str())
        .unwrap_or("");
    // A game lifted from a ROM publishes the theme's own files: a look
    // drawn for it would hold the lifted art wherever the theme leaves a
    // picture alone.
    let source = how == "rom";
    let kind = if source { "source" } else { "look" };
    let game = family(&repo, &a.game);
    // A folder holding same_as.toml is published as the theme it points at.
    let theme_dir = gp_theme::same_as::target(&a.theme).map_err(anyhow::Error::msg)?;
    let layer =
        gp_theme::stack::load(&theme_dir, gp_theme::Scope::Personal).map_err(anyhow::Error::msg)?;
    let th = &layer.theme;
    let (model, _) = crate::theme::read_model(&tdir)?;
    let model_name = model.model.name.clone();
    if !th.for_models.is_empty() && !th.for_models.contains(&model_name) {
        bail!(
            "theme `{}` is for {}, not `{model_name}`",
            th.id,
            th.for_models.join(", ")
        );
    }
    if let Some(why) = id_taken(&tdir, &repo, &th.id, &theme_dir) {
        return Ok(Outcome {
            json: json!({ "published": false, "game": game, "theme": th.id, "problems": [why] }),
            text: format!("{why}\ntheme `{}` refused; nothing published", th.id),
            exit: 1,
        });
    }
    let mut text = String::new();
    let work = paths
        .home
        .join("work")
        .join("theme-publish")
        .join(&game)
        .join(&th.id);
    fs::create_dir_all(&work)?;

    // Each model the theme is for (its `for`, else the template's model),
    // checked on this machine when it can be: the given template's with
    // its assets and ROM, and each other template of the game whose ROM is
    // in $GP_ROM_DIR, over assets lifted from that ROM here. A model whose
    // ROM isn't here is recorded as not checked, with the reason.
    let mut for_models = th.for_models.clone();
    if for_models.is_empty() {
        for_models.push(model_name.clone());
    }
    let siblings = crate::theme_slots::same_game(&tdir);
    let mut runs: Vec<ModelRun> = Vec::new();
    let mut unchecked: BTreeMap<String, String> = BTreeMap::new();
    for n in &for_models {
        if *n == model_name {
            runs.push(check_model(
                &theme_dir,
                &tdir,
                &a.game,
                a.assets.as_deref(),
                a.rom.as_deref(),
                (&work, paths),
                rec,
            )?);
            continue;
        }
        let Some((d, _)) = siblings.iter().find(|(_, m)| m.model.name == *n) else {
            unchecked.insert(
                n.clone(),
                format!("no template of the game `{game}` in this repo has the model `{n}`"),
            );
            continue;
        };
        match check_other(&theme_dir, d, None, None, (&work, paths), rec)? {
            Ok(r) => runs.push(r),
            Err(why) => {
                unchecked.insert(n.clone(), why);
            }
        }
    }
    let results = |fails: &BTreeMap<String, Vec<&str>>| {
        let mut m = serde_json::Map::new();
        for n in &for_models {
            let v = match runs.iter().find(|r| r.model == *n) {
                Some(r) => match fails.get(n) {
                    Some(f) => json!({
                        "result": "fail", "template": r.template, "rom_sha256": r.rom_hash,
                        "failed": f,
                    }),
                    None => json!({
                        "result": "pass", "template": r.template, "rom_sha256": r.rom_hash,
                    }),
                },
                None => json!({ "result": "unchecked", "why": unchecked.get(n) }),
            };
            m.insert(n.clone(), v);
        }
        Value::Object(m)
    };

    // A refusal by any model's checks refuses the publish.
    let fails: BTreeMap<String, Vec<&str>> = runs
        .iter()
        .filter(|r| r.out.exit != 0)
        .map(|r| (r.model.clone(), failed_checks(&r.out.json)))
        .collect();
    if !fails.is_empty() {
        let mut t = String::new();
        let mut problems = Vec::new();
        for r in runs.iter().filter(|r| r.out.exit != 0) {
            let _ = writeln!(t, "{}: {}", r.model, r.out.text);
            problems.push(format!(
                "{} (template {}): refused by its {} check",
                r.model,
                r.template,
                fails[&r.model].join(", ")
            ));
        }
        let first = runs.iter().find(|r| r.out.exit != 0).map(|r| &r.out.json);
        return Ok(Outcome {
            json: json!({
                "published": false, "game": game, "theme": th.id, "checks": first,
                "models": results(&fails), "problems": problems,
            }),
            text: format!(
                "{t}{}\ntheme `{}` refused by its checks; nothing published",
                problems.join("\n"),
                th.id
            ),
            exit: 1,
        });
    }
    let given = runs
        .iter()
        .find(|r| r.model == model_name)
        .context("the template's own model wasn't checked")?;
    let out = &given.out;
    let geometry = geometry(&out.json);
    let given_look = given.look.as_ref().context("the look wasn't drawn")?;

    // The package's files.
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (rel, p) in files_under(&theme_dir)? {
        files.insert(format!("theme/{rel}"), fs::read(&p)?);
    }
    if !source {
        for (rel, p) in &given_look.drawn {
            files.insert(format!("look/{rel}"), fs::read(p)?);
        }
    }
    let mut problems = scan(&files);
    if source {
        problems.extend(drawn_copies(&runs, &files)?);
    }
    let RomBytes {
        problems: rom_problems,
        fails: rom_fails,
        lines: rom_lines,
    } = rom_bytes(&runs, &files, &work, paths)?;
    problems.extend(rom_problems);
    if !problems.is_empty() {
        return Ok(Outcome {
            json: json!({
                "published": false, "game": game, "theme": th.id,
                "models": results(&rom_fails), "problems": problems,
            }),
            text: format!(
                "{}\ntheme `{}` refused by the package scan; nothing published",
                problems.join("\n"),
                th.id
            ),
            exit: 1,
        });
    }
    let all_rom_lines: Vec<String> = rom_lines.iter().map(|(l, _)| l.clone()).collect();

    // Each model the theme is for, with its result and, for one checked
    // here, its lines.
    let mut per_model = serde_json::Map::new();
    let mut model_lines = Vec::new();
    let mut lines_of = BTreeMap::new();
    for r in &runs {
        let (record, readability, originality) = passed(r, &rom_lines)?;
        let _ = writeln!(text, "{}", checks_text(&r.model, &record));
        per_model.insert(r.model.clone(), record);
        lines_of.insert(r.model.clone(), (readability, originality));
    }
    for n in &for_models {
        if let Some(r) = runs.iter().find(|r| r.model == *n) {
            let line = &r.look.as_ref().map(|l| l.line.clone()).unwrap_or_default();
            model_lines.push(format!("{n} checked: pass, {line}"));
        } else {
            let why = unchecked.get(n).cloned().unwrap_or_default();
            model_lines.push(format!("{n} not checked: {why}"));
            per_model.insert(n.clone(), json!({ "result": "unchecked", "why": why }));
        }
    }
    let _ = writeln!(text, "package scan: {} files, no problem", files.len());
    let _ = writeln!(text, "models: {}", model_lines.join("; "));
    let (readability, originality) = lines_of[&model_name].clone();
    let checks = json!({
        "format": "0.1.0",
        "kind": kind,
        "readability": out.json["readability"],
        "style": out.json["style"],
        "originality": out.json["originality"],
        "geometry": geometry,
        "scan": { "files": files.len(), "rom": all_rom_lines },
        "models": per_model,
    });
    files.insert("checks.json".into(), serde_json::to_vec_pretty(&checks)?);

    let mut m = Manifest::new("theme", &th.id);
    for (p, b) in &files {
        m.add(p, b).map_err(anyhow::Error::msg)?;
    }
    let id = m.id();

    // The index entry. A source package keys no build by `fits` (empty, so
    // a runtime from before source packages offers it nowhere) and has no
    // preview picture, since anything gp drew holds the lifted art.
    let preview_hash = files
        .iter()
        .filter(|(p, _)| p.starts_with("look/") && p.ends_with(".png"))
        .filter_map(|(p, b)| png_size(b).map(|(w, h)| (u64::from(w) * u64::from(h), p)))
        .max()
        .and_then(|(_, p)| m.get(p))
        .map(|f| f.hash.clone())
        .unwrap_or_default();
    let kit_hash = fs::read(tdir.join("themes/kit/kit.toml"))
        .map(|b| gp_package::hash(&b))
        .unwrap_or_default();
    let signer = a
        .signer
        .clone()
        .or_else(|| std::env::var("GP_SIGNER").ok())
        .or_else(|| git_config("user.email"));
    let mut entry = Entry {
        id: th.id.clone(),
        title: th.title.clone(),
        author: th.author.clone().unwrap_or_default(),
        license: th.license.clone(),
        package: id.clone(),
        model: model_name.clone(),
        fits: if source {
            String::new()
        } else {
            index::pictures_digest(&given_look.pictures)
        },
        kit: kit_hash,
        kind: kind.into(),
        preview: preview_hash,
        signer: signer.clone().unwrap_or_default(),
        originality,
        readability,
        geometry: geometry.clone(),
        size: m.size(),
        ..Entry::default()
    };
    for r in &runs {
        entry.record_model(&r.model, &r.slots);
        if let Some(l) = &r.look {
            entry.record_look(&r.model, &l.hash);
        }
    }
    for n in &for_models {
        entry.record_result(n, per_model[n]["result"].as_str().unwrap_or("unchecked"));
    }
    let ipath = index_path(&repo, &game, a.index.as_deref());
    let mut ix = read_index(&ipath, &game)?;
    ix.put(entry.clone());
    let new_index = ix.write();

    if a.dry_run {
        let mut one = Index::new(&game);
        one.put(entry.clone());
        let line = one.write();
        let line = &line[line.find("[[theme]]").unwrap_or(0)..];
        let mut t = format!(
            "dry run: `{}` for {game} (template {}), a {kind} package{}\n",
            th.id,
            a.game,
            if source {
                ": the theme's own files and checks.json, no look drawn from the ROM's art"
            } else {
                ": the theme, the look gp drew and checks.json"
            }
        );
        let _ = writeln!(t, "files ({}, {} bytes):", m.files.len(), m.size());
        for f in &m.files {
            let _ = writeln!(t, "  {} {}", f.path, f.size);
        }
        t.push_str(&text);
        let _ = writeln!(
            t,
            "package {id} (not stored)\nindex line it would write in {}:",
            ipath.display()
        );
        for l in line.lines() {
            let _ = writeln!(t, "  {l}");
        }
        t.push_str("nothing written: no store, no signature, no mirror, no index (--dry-run)");
        return Ok(Outcome::ok(
            json!({
                "published": false, "dry_run": true, "game": game, "kind": kind,
                "package": id,
                "files": m.files.iter().map(|f| json!({"path": f.path, "size": f.size, "hash": f.hash})).collect::<Vec<_>>(),
                "checks": checks, "entry": entry_json(&entry), "index": ipath, "index_line": line,
            }),
            t,
        ));
    }

    let store = Store::at(&paths.store);
    store.put_package(&m, &files).map_err(anyhow::Error::msg)?;
    let _ = writeln!(
        text,
        "package {id}: {} files, {} bytes, in {}",
        m.files.len(),
        m.size(),
        paths.store.display()
    );

    // The signature, over the manifest's bytes, checked before it goes out.
    // Only maintainers sign: their keys are the allowed signers.
    let key = a
        .key
        .clone()
        .or_else(|| std::env::var_os("GP_SIGNING_KEY").map(PathBuf::from))
        .or_else(|| git_config("user.signingkey").map(PathBuf::from))
        .context("no key to sign with: only maintainers sign, with a key listed in themes/allowed_signers. Give --key FILE, or set GP_SIGNING_KEY or git's user.signingkey to an SSH key; --dry-run shows what would be published with no key")?;
    let signer = signer.context("no signer: give --signer NAME")?;
    let sig = sign(&key, &store.path(&package_key(&id)))?;
    let allowed = allowed_signers(&paths.home, &repo);
    verify(&allowed, &signer, &sig, m.write().as_bytes()).with_context(|| {
        format!("only maintainers sign: a maintainer adds `{signer} namespaces=\"{NAMESPACE}\" <public key>` to {ALLOWED_SIGNERS} through the gate")
    })?;
    store
        .put(&signature_key(&id), &sig)
        .map_err(anyhow::Error::msg)?;
    let _ = writeln!(text, "signed by {signer}");

    if a.local {
        let _ = write!(
            text,
            "kept in the local store only (--local): no mirror and no index change"
        );
        return Ok(Outcome::ok(
            json!({ "published": false, "package": id, "entry": entry_json(&entry) }),
            text,
        ));
    }

    // The mirrors: blobs each lacks, then the manifest and its signature
    // last, so a mirror never names a file it hasn't.
    let mirrors = mirror::configured(&paths.home, Some(&repo))?;
    let writable: Vec<&Mirror> = mirrors.iter().filter(|m| m.writable()).collect();
    let mut sent = Vec::new();
    for mi in &writable {
        let mut n = 0;
        for f in &m.files {
            n += usize::from(mi.put(&paths.home, &blob_key(&f.hash), &files[&f.path], false)?);
        }
        mi.put(&paths.home, &package_key(&id), m.write().as_bytes(), false)?;
        mi.put(&paths.home, &signature_key(&id), &sig, false)?;
        let _ = writeln!(
            text,
            "mirror {}: {n} new blobs, the manifest and its signature",
            mi.name
        );
        sent.push(json!({ "mirror": mi.name, "blobs": n }));
    }
    if writable.is_empty() {
        let _ = writeln!(
            text,
            "no mirror to write to ({} lists none with a write endpoint)",
            mirror::MIRRORS_FILE
        );
    }

    // The index, then its copy on each mirror once it has landed.
    let msg = format!(
        "Themes: {} publishes `{}` ({}) for {}, package {}",
        signer,
        th.id,
        th.title,
        game,
        &id[..12]
    );
    let (landed, line) = land_index(
        &repo,
        &ipath,
        &new_index,
        (a.no_gate, &a.gate),
        &msg,
        &work,
        &id[..12],
        (&game, &writable, &paths.home),
    )?;
    let _ = writeln!(text, "{line}");
    let _ = write!(
        text,
        "{} `{}` for {game}",
        if landed {
            "published"
        } else {
            "not listed (the gate didn't pass)"
        },
        th.id,
    );
    Ok(Outcome {
        json: json!({
            "published": landed,
            "game": game,
            "package": id,
            "entry": entry_json(&entry),
            "mirrors": sent,
            "index": ipath,
        }),
        text,
        exit: u8::from(!landed),
    })
}

/// The templates of a game in the repo, with their models: each folder of
/// templates/ whose game ([`family`]) is `game`.
fn game_templates(repo: &Path, game: &str) -> Vec<(PathBuf, gp_model::Model)> {
    // gameprinter: the games checkout's game folders.
    crate::theme_template::game_dirs()
        .into_iter()
        .filter(|d| d.join("template.toml").is_file())
        .filter(|d| {
            crate::theme_slots::game_of(d).unwrap_or_else(|| {
                d.file_name()
                    .map(|n| family(repo, &n.to_string_lossy()))
                    .unwrap_or_default()
            }) == game
        })
        .filter_map(|d| {
            let (m, _) = crate::theme::read_model(&d).ok()?;
            Some((d, m))
        })
        .collect()
}

/// `gp theme recheck`: a published source package's checks run on this
/// machine for a model of its `for`, with this machine's ROM, and the
/// model's result, slot fingerprint and look digest added to the theme's
/// index line. The package and its signature are left as they are.
pub(crate) fn recheck(a: &RecheckArgs, paths: &Paths, rec: &mut RunRecorder) -> Result<Outcome> {
    let repo = repo_root()?;
    let game = family(&repo, &a.game);
    let ipath = index_path(&repo, &game, a.index.as_deref());
    let ix = read_index(&ipath, &game)?;
    let e = ix
        .theme(&a.theme)
        .with_context(|| {
            format!(
                "the themes index of {game} ({}) has no theme `{}`",
                ipath.display(),
                a.theme
            )
        })?
        .clone();
    if e.kind != "source" {
        bail!(
            "`{}` is a look package, drawn by gp for one build of {game}; a recheck adds a model to a source package (a ROM theme). Publish the theme again to draw it for another build",
            e.id
        );
    }
    let store = Store::at(&paths.store);
    let mirrors = mirror::configured(&paths.home, Some(&repo))?;
    let pkg = read_package(&repo, &e, &store, &mirrors, paths)?;
    let work = paths
        .home
        .join("work")
        .join("theme-recheck")
        .join(&game)
        .join(&e.id);
    fs::create_dir_all(&work)?;
    // The package's files in a temporary folder: the theme is checked as
    // the package holds it.
    let unpacked = tempfile::tempdir_in(&work)?;
    for (p, b) in &pkg.files {
        let to = unpacked.path().join(p);
        fs::create_dir_all(to.parent().unwrap_or(unpacked.path()))?;
        fs::write(&to, b)?;
    }
    let theme_dir = unpacked.path().join("theme");
    let checks: Value = pkg
        .files
        .get("checks.json")
        .and_then(|b| serde_json::from_slice(b).ok())
        .unwrap_or(Value::Null);
    let th = gp_theme::stack::load(&theme_dir, gp_theme::Scope::Personal)
        .map_err(anyhow::Error::msg)?
        .theme;
    let mut for_models = th.for_models.clone();
    if for_models.is_empty() {
        for_models.push(e.model.clone());
    }
    let templates = game_templates(&repo, &game);
    let template_of = |m: &str| templates.iter().find(|(_, x)| x.model.name == m);

    // The models to check: the one named, else the one whose template
    // accepts the ROM given, else each one not recorded as passed.
    let named: Option<String> = match (&a.model, &a.rom) {
        (Some(m), _) => Some(m.clone()),
        (None, Some(p)) => {
            let r = crate::theme_template::Rom::read(p)?;
            let found = templates.iter().find(|(d, _)| {
                crate::theme_template::load(d)
                    .is_ok_and(|t| t.manifest.source.accepts.contains(&r.sha256))
            });
            match found {
                Some((_, m)) => Some(m.model.name.clone()),
                None => bail!(
                    "no template of {game} in this repo accepts {} (its SHA-256 is {})",
                    p.display(),
                    r.sha256
                ),
            }
        }
        (None, None) => None,
    };
    if a.assets.is_some() && named.is_none() {
        bail!("--assets are one model's: give --model or --rom with them");
    }
    let wanted: Vec<String> = match &named {
        Some(m) => {
            if !for_models.contains(m) {
                bail!(
                    "theme `{}` is for {}, not `{m}`",
                    e.id,
                    for_models.join(", ")
                );
            }
            vec![m.clone()]
        }
        None => for_models
            .iter()
            .filter(|m| e.result_for(m) != Some("pass"))
            .cloned()
            .collect(),
    };
    if wanted.is_empty() {
        bail!(
            "every model `{}` is for is recorded as passed ({}); give --model or --rom to check one again",
            e.id,
            e.results
        );
    }
    let mut runs: Vec<ModelRun> = Vec::new();
    let mut not_checked: Vec<(String, String)> = Vec::new();
    for m in &wanted {
        let Some((d, _)) = template_of(m) else {
            not_checked.push((
                m.clone(),
                format!("no template of the game `{game}` in this repo has the model `{m}`"),
            ));
            continue;
        };
        match check_other(
            &theme_dir,
            d,
            a.rom.as_deref(),
            a.assets.as_deref(),
            (&work, paths),
            rec,
        )? {
            Ok(r) => runs.push(r),
            Err(why) => not_checked.push((m.clone(), why)),
        }
    }

    let mut text = format!(
        "recheck of `{}` ({}) for {game}, package {} signed by {}; its files are left as they are\n",
        e.id,
        e.title,
        &e.package[..12.min(e.package.len())],
        if e.signer.is_empty() {
            "nobody"
        } else {
            &e.signer
        }
    );
    let mut problems: Vec<String> = Vec::new();
    for (m, why) in &not_checked {
        let _ = writeln!(text, "{m} not checked: {why}");
        if named.is_some() {
            problems.push(format!("{m} not checked: {why}"));
        }
    }
    if runs.is_empty() && problems.is_empty() {
        problems.push("no model could be checked on this machine".into());
    }
    let mut results = serde_json::Map::new();
    for r in runs.iter().filter(|r| r.out.exit != 0) {
        let f = failed_checks(&r.out.json);
        let _ = writeln!(text, "{}: {}", r.model, r.out.text);
        problems.push(format!(
            "{} (template {}): refused by its {} check",
            r.model,
            r.template,
            f.join(", ")
        ));
        results.insert(
            r.model.clone(),
            json!({ "result": "fail", "template": r.template, "rom_sha256": r.rom_hash, "failed": f }),
        );
    }
    let ok: Vec<ModelRun> = runs.into_iter().filter(|r| r.out.exit == 0).collect();
    problems.extend(drawn_copies(&ok, &pkg.files)?);
    let rb = rom_bytes(&ok, &pkg.files, &work, paths)?;
    problems.extend(rb.problems.iter().cloned());

    // Each look drawn here against the one recorded for the model: in the
    // index (the publisher's, or an earlier recheck's), else the
    // package's checks.json.
    let mut lines = Vec::new();
    for r in &ok {
        let look = r.look.as_ref().context("the look wasn't drawn")?;
        let made = &checks["models"][&r.model];
        let recorded = e.look_for(&r.model).map(str::to_owned).or_else(|| {
            (made["result"] == "pass")
                .then(|| made["look"].as_str().map(str::to_owned))
                .flatten()
        });
        let short = |h: &str| h[..12.min(h.len())].to_owned();
        let compared = match &recorded {
            None => "the first look recorded for this model".to_owned(),
            Some(h) if *h == look.hash => "the same look as the one recorded".to_owned(),
            Some(h) => {
                let roms = match made["rom_sha256"].as_str() {
                    Some(theirs) if Some(theirs) != r.rom_hash.as_deref() => format!(
                        "; the maker's ROM had SHA-256 {theirs}, this one {}",
                        r.rom_hash.as_deref().unwrap_or("unknown")
                    ),
                    _ => String::new(),
                };
                problems.push(format!(
                    "the look drawn here differs from the maker's for {}: the maker's ROM or gp differs (recorded {}, drawn here {}{roms}); not added",
                    r.model,
                    short(h),
                    short(&look.hash)
                ));
                results.insert(
                    r.model.clone(),
                    json!({
                        "result": "differs", "template": r.template, "rom_sha256": r.rom_hash,
                        "look": look.hash, "recorded": h,
                    }),
                );
                continue;
            }
        };
        let (record, _, _) = passed(r, &rb.lines)?;
        let _ = writeln!(text, "{}", checks_text(&r.model, &record));
        let rom_name = r
            .rom
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        lines.push(format!(
            "{} (template {}, ROM {rom_name}): pass, {}; look {}, {compared}",
            r.model,
            r.template,
            look.line,
            short(&look.hash)
        ));
        if !rb.fails.contains_key(&r.model) {
            let mut record = record;
            record["recorded"] = json!(recorded);
            results.insert(r.model.clone(), record);
        }
    }
    for l in &lines {
        let _ = writeln!(text, "{l}");
    }
    if !problems.is_empty() {
        return Ok(Outcome {
            json: json!({
                "rechecked": false, "game": game, "theme": e.id, "package": e.package,
                "models": results, "problems": problems,
            }),
            text: format!(
                "{text}{}\nnothing added to the index of {game}",
                problems.join("\n")
            ),
            exit: 1,
        });
    }

    // The index line with each model checked here added.
    let mut entry = e.clone();
    for r in &ok {
        entry.record_model(&r.model, &r.slots);
        entry.record_result(&r.model, "pass");
        if let Some(l) = &r.look {
            entry.record_look(&r.model, &l.hash);
        }
    }
    let mut ix = ix;
    ix.put(entry.clone());
    let new_index = ix.write();
    let mut one = Index::new(&game);
    one.put(entry.clone());
    let line = one.write();
    let line = line[line.find("[[theme]]").unwrap_or(0)..].to_owned();
    let added: Vec<&str> = ok.iter().map(|r| r.model.as_str()).collect();
    let mut j = json!({
        "rechecked": true, "dry_run": a.dry_run, "game": game, "theme": e.id,
        "package": e.package, "models": results, "entry": entry_json(&entry),
        "index": ipath, "index_line": line,
    });
    if a.dry_run {
        let _ = writeln!(text, "index line it would write in {}:", ipath.display());
        for l in line.lines() {
            let _ = writeln!(text, "  {l}");
        }
        text.push_str("nothing written: no index, no mirror (--dry-run)");
        return Ok(Outcome::ok(j, text));
    }
    let writable: Vec<&Mirror> = mirrors.iter().filter(|m| m.writable()).collect();
    let msg = format!(
        "Themes: a recheck of `{}` for {game} adds {}, package {}",
        e.id,
        added.join(", "),
        &e.package[..12.min(e.package.len())]
    );
    let tag = format!(
        "recheck-{}",
        gp_package::hash(new_index.as_bytes())
            .get(..12)
            .unwrap_or_default()
    );
    let (landed, l) = land_index(
        &repo,
        &ipath,
        &new_index,
        (a.no_gate, &a.gate),
        &msg,
        &work,
        &tag,
        (&game, &writable, &paths.home),
    )?;
    let _ = writeln!(text, "{l}");
    let _ = write!(
        text,
        "{} for `{}` in the index of {game}",
        if landed {
            format!("added {}", added.join(", "))
        } else {
            "nothing added (the gate didn't pass)".to_owned()
        },
        e.id
    );
    j["added"] = json!(landed);
    Ok(Outcome {
        json: j,
        text,
        exit: u8::from(!landed),
    })
}

/// The geometry check's line (docs/theme-kit.md, "Gameplay geometry"),
/// from a theme check's outcome. A refusal has stopped the publish
/// before this.
fn geometry(checks: &Value) -> String {
    let g = &checks["geometry"];
    match g["checked"].as_u64() {
        Some(n) if n > 0 => {
            let flags = g["flags"].as_array().map_or(0, Vec::len);
            check_line(
                true,
                format!("{n} slots and stages checked, {flags} flags, none refused"),
            )
        }
        _ => format!(
            "not checked: {}",
            g["why"].as_str().unwrap_or("this gp has no geometry check")
        ),
    }
}

fn entry_json(e: &Entry) -> Value {
    json!({
        "id": e.id, "title": e.title, "author": e.author, "license": e.license,
        "package": e.package, "model": e.model, "fits": e.fits, "kit": e.kit,
        "kind": e.kind, "models": e.models, "slots": e.slots, "results": e.results,
        "looks": e.looks,
        "preview": e.preview, "signer": e.signer, "originality": e.originality,
        "readability": e.readability, "geometry": e.geometry, "size": e.size,
    })
}

/// A patch that replaces a file's whole text, for `git apply`.
pub(crate) fn whole_file_patch(rel: &str, old: &str, new: &str) -> String {
    let lines = |s: &str| s.lines().map(str::to_owned).collect::<Vec<_>>();
    let (o, n) = (lines(old), lines(new));
    let mut p = format!("diff --git a/{rel} b/{rel}\n");
    if o.is_empty() {
        p.push_str("new file mode 100644\n--- /dev/null\n");
    } else {
        let _ = writeln!(p, "--- a/{rel}");
    }
    let _ = writeln!(p, "+++ b/{rel}");
    let range = |len: usize| {
        if len == 0 {
            "0,0".to_owned()
        } else {
            format!("1,{len}")
        }
    };
    let _ = writeln!(p, "@@ -{} +{} @@", range(o.len()), range(n.len()));
    for l in &o {
        let _ = writeln!(p, "-{l}");
    }
    for l in &n {
        let _ = writeln!(p, "+{l}");
    }
    p
}

/// Gets a key from the local store, else from the first mirror that has
/// it, storing it locally.
fn get_key(store: &Store, mirrors: &[Mirror], key: &str) -> Result<Option<Vec<u8>>> {
    if store.has(key) {
        return Ok(Some(store.get(key).map_err(anyhow::Error::msg)?));
    }
    for m in mirrors {
        if let Some(b) = m.get(key)? {
            return Ok(Some(b));
        }
    }
    Ok(None)
}

/// A package read by an index line, every hash and its signature checked.
struct Fetched {
    m: Manifest,
    mbytes: Vec<u8>,
    sig: Option<Vec<u8>>,
    /// Each file's bytes, by its path in the package.
    files: BTreeMap<String, Vec<u8>>,
}

/// Reads the package an index line names from the local store, else the
/// first mirror that has it: the manifest against the package id, its
/// signature when the line names a signer, each file by its size and
/// hash. Writes nothing.
fn read_package(
    repo: &Path,
    e: &Entry,
    store: &Store,
    mirrors: &[Mirror],
    paths: &Paths,
) -> Result<Fetched> {
    let id = &e.package;
    let mbytes = get_key(store, mirrors, &package_key(id))?
        .with_context(|| format!("package {id} is in no store or mirror"))?;
    let m = Manifest::read_checked(id, &mbytes).map_err(anyhow::Error::msg)?;
    let sig = if e.signer.is_empty() {
        None
    } else {
        let sig = get_key(store, mirrors, &signature_key(id))?.with_context(|| {
            format!(
                "package {id} has no signature, and the index names {}",
                e.signer
            )
        })?;
        verify(
            &allowed_signers(&paths.home, repo),
            &e.signer,
            &sig,
            &mbytes,
        )?;
        Some(sig)
    };
    let mut files = BTreeMap::new();
    for f in &m.files {
        let b = get_key(store, mirrors, &blob_key(&f.hash))?
            .with_context(|| format!("{}: blob {} is in no store or mirror", f.path, f.hash))?;
        let b = gp_package::checked_blob(f, b).map_err(anyhow::Error::msg)?;
        files.insert(f.path.clone(), b);
    }
    Ok(Fetched {
        m,
        mbytes,
        sig,
        files,
    })
}

pub(crate) fn fetch(
    game: &str,
    theme: &str,
    index_file: Option<&Path>,
    out: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let repo = repo_root()?;
    let family = family(&repo, game);
    let game = family.as_str();
    let ix = read_index(&index_path(&repo, game, index_file), game)?;
    let e = ix
        .theme(theme)
        .with_context(|| format!("the themes index of {game} has no theme `{theme}`"))?;
    let store = Store::at(&paths.store);
    let mirrors = mirror::configured(&paths.home, Some(&repo))?;
    let id = &e.package;
    let Fetched {
        m,
        mbytes,
        sig,
        files,
    } = read_package(&repo, e, &store, &mirrors, paths)?;
    if let Some(sig) = &sig {
        store
            .put(&signature_key(id), sig)
            .map_err(anyhow::Error::msg)?;
    }
    let mut fetched = 0;
    for f in &m.files {
        if store
            .put(&blob_key(&f.hash), &files[&f.path])
            .map_err(anyhow::Error::msg)?
        {
            fetched += 1;
        }
    }
    store
        .put(&package_key(id), &mbytes)
        .map_err(anyhow::Error::msg)?;
    let dir = out.map(Path::to_owned).unwrap_or_else(|| {
        paths
            .home
            .join("themes")
            .join("packages")
            .join(game)
            .join(theme)
    });
    for (p, b) in &files {
        let to = dir.join(p);
        fs::create_dir_all(to.parent().unwrap_or(&dir))?;
        fs::write(&to, b)?;
    }
    Ok(Outcome::ok(
        json!({ "game": game, "theme": theme, "package": id, "files": m.files.len(), "fetched": fetched, "dir": dir }),
        format!(
            "{theme} ({}) package {id}: {} files, {fetched} fetched, every hash checked{}; in {} ({})",
            e.title,
            m.files.len(),
            if e.signer.is_empty() {
                String::new()
            } else {
                format!(", signed by {}", e.signer)
            },
            dir.display(),
            if m.files.iter().any(|f| f.path.starts_with("look/")) {
                "the theme in theme/, the look in look/"
            } else {
                "the theme in theme/; a source package, drawn over your own ROM's art"
            }
        ),
    ))
}

pub(crate) fn list(
    game: &str,
    index_file: Option<&Path>,
    from_mirror: bool,
    paths: &Paths,
) -> Result<Outcome> {
    let repo = repo_root()?;
    let family = family(&repo, game);
    let game = family.as_str();
    let ix = if from_mirror {
        let mirrors = mirror::configured(&paths.home, Some(&repo))?;
        let mut found = None;
        for m in &mirrors {
            if let Some(b) = m.get(&themes_index_key(game))? {
                found =
                    Some(Index::read(&String::from_utf8_lossy(&b)).map_err(anyhow::Error::msg)?);
                break;
            }
        }
        found.with_context(|| format!("no mirror has the themes index of {game}"))?
    } else {
        read_index(&index_path(&repo, game, index_file), game)?
    };
    let store = Store::at(&paths.store);
    let mut text = format!("{game}: {} themes\n", ix.themes.len());
    let mut items = Vec::new();
    for t in &ix.themes {
        let local = store.has(&package_key(&t.package));
        let _ = writeln!(
            text,
            "{} \"{}\" by {} ({}), {} bytes, {} package {}{}\n  for {}\n  originality {}\n  readability {}\n  geometry {}",
            t.id,
            t.title,
            if t.author.is_empty() {
                "nobody named"
            } else {
                &t.author
            },
            t.license,
            t.size,
            if t.kind.is_empty() { "look" } else { &t.kind },
            &t.package[..12.min(t.package.len())],
            if local { ", in the local store" } else { "" },
            if t.results.is_empty() {
                &t.model
            } else {
                &t.results
            },
            t.originality,
            t.readability,
            t.geometry
        );
        let mut j = entry_json(t);
        j["local"] = json!(local);
        items.push(j);
    }
    Ok(Outcome::ok(
        json!({ "game": game, "themes": items }),
        text.trim_end().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scan_refuses_what_a_package_may_not_hold() {
        let mut f = BTreeMap::new();
        f.insert("theme/theme.toml".to_owned(), b"id = \"x\"\n".to_vec());
        f.insert("look/a.png".to_owned(), b"\x89PNG\r\n\x1a\n....".to_vec());
        assert!(scan(&f).is_empty());
        f.insert("theme/game.nes".to_owned(), b"NES\x1a".to_vec());
        f.insert("look/b.png".to_owned(), b"not a png".to_vec());
        f.insert(
            "theme/big.txt".to_owned(),
            vec![b'a'; (MAX_FILE + 1) as usize],
        );
        f.insert("theme/rom.txt".to_owned(), b"NES\x1a....".to_vec());
        let p = scan(&f);
        assert_eq!(p.len(), 4, "{p:?}");
    }

    #[test]
    fn a_whole_file_patch_applies() {
        let d = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            let o = Command::new("git")
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
        };
        g(&["init", "-q"]);
        let new = whole_file_patch("themes/index/x.toml", "", "a\nb\n");
        fs::write(d.path().join("p1"), &new).unwrap();
        g(&["apply", "p1"]);
        assert_eq!(
            fs::read_to_string(d.path().join("themes/index/x.toml")).unwrap(),
            "a\nb\n"
        );
        let changed = whole_file_patch("themes/index/x.toml", "a\nb\n", "a\nc\nd\n");
        fs::write(d.path().join("p2"), &changed).unwrap();
        g(&["apply", "p2"]);
        assert_eq!(
            fs::read_to_string(d.path().join("themes/index/x.toml")).unwrap(),
            "a\nc\nd\n"
        );
    }
}
